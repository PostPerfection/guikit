//! Hosts libmpv's OpenGL output in an `NSOpenGLView` layered over the webview.
//!
//! The GL view is a sibling of tauri's WKWebView inside the window's content
//! view, added last so it sits above it, and positioned from the page: the
//! frontend reports where its placeholder element is and the view is moved to
//! match. AppKit and mpv's renderer both demand the main thread, so every call
//! that touches either goes through tauri's main-thread dispatcher.
#![allow(deprecated)]

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{
    NSOpenGLContext, NSOpenGLPFAAccelerated, NSOpenGLPFAAlphaSize, NSOpenGLPFAColorSize,
    NSOpenGLPFADoubleBuffer, NSOpenGLPFAOpenGLProfile, NSOpenGLPixelFormat,
    NSOpenGLPixelFormatAttribute, NSOpenGLProfileVersion3_2Core, NSOpenGLView, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use postkit::mpv_render::MpvRenderPlayer;
use tauri::Manager;

const GL_RENDERER: u32 = 0x1F01;
const GL_VERSION: u32 = 0x1F02;

/// An `NSOpenGLView` draws into the default framebuffer, so mpv's target is the
/// one GL always has.
const DEFAULT_FRAMEBUFFER: i32 = 0;

/// The AppKit GL default framebuffer has its origin at the bottom left, which is
/// the orientation mpv already draws in.
const FLIP_Y: bool = false;

const COLOR_BITS: NSOpenGLPixelFormatAttribute = 24;
const ALPHA_BITS: NSOpenGLPixelFormatAttribute = 8;
const ATTRIBUTES_END: NSOpenGLPixelFormatAttribute = 0;

const PIXEL_FORMAT_ATTRIBUTES: [NSOpenGLPixelFormatAttribute; 9] = [
    NSOpenGLPFAOpenGLProfile,
    NSOpenGLProfileVersion3_2Core,
    NSOpenGLPFAAccelerated,
    NSOpenGLPFADoubleBuffer,
    NSOpenGLPFAColorSize,
    COLOR_BITS,
    NSOpenGLPFAAlphaSize,
    ALPHA_BITS,
    ATTRIBUTES_END,
];

/// Where the view sits with no preview on screen. It stays in the window rather
/// than being torn down: mpv's render context lives on this GL context, and
/// advanced control needs the render loop still answering.
const HIDDEN_SURFACE: NSRect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));

const OPENGL_FRAMEWORK: &CStr = c"/System/Library/Frameworks/OpenGL.framework/OpenGL";

#[derive(Clone, Copy, Default)]
struct SurfaceRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    visible: bool,
}

/// The AppKit objects behind the preview. They are held as pointers, and
/// retained for the life of the process, so that the preview can be `Send` and
/// `Sync`, which is sound because they are only ever dereferenced by the
/// closures below, all of which run on the main thread.
struct PreviewSurface {
    container: *mut NSView,
    gl_view: *mut NSOpenGLView,
    context: *mut NSOpenGLContext,
}

unsafe impl Send for PreviewSurface {}
unsafe impl Sync for PreviewSurface {}

pub struct EmbeddedPreview {
    player: Arc<MpvRenderPlayer>,
    surface: Arc<PreviewSurface>,
    rect: Arc<Mutex<SurfaceRect>>,
    app_handle: tauri::AppHandle,
}

impl EmbeddedPreview {
    pub fn player(&self) -> &MpvRenderPlayer {
        &self.player
    }

    /// Move the video surface to where the page says its placeholder is, in CSS
    /// pixels from the top-left of the webview.
    pub fn set_surface(&self, x: i32, y: i32, width: i32, height: i32, visible: bool) {
        *self.rect.lock().unwrap() = SurfaceRect {
            x,
            y,
            width,
            height,
            visible,
        };
        let surface = Arc::clone(&self.surface);
        let rect = Arc::clone(&self.rect);
        let _ = self.app_handle.run_on_main_thread(move || {
            let current = *rect.lock().unwrap();
            apply_rect(&surface, current);
        });
    }
}

/// Put a GL view over the window's webview and hand back the player driving it.
/// Everything here touches AppKit, so it must run on the main thread.
pub fn attach(window: &tauri::Window) -> Result<EmbeddedPreview, String> {
    let mtm = MainThreadMarker::new().ok_or("the preview must be attached on the main thread")?;
    let content_view = window.ns_view().map_err(|e| e.to_string())?;
    if content_view.is_null() {
        return Err("the window has no content view".to_string());
    }
    let container: &NSView = unsafe { &*content_view.cast::<NSView>() };

    let mut attributes = PIXEL_FORMAT_ATTRIBUTES;
    let pixel_format = unsafe {
        NSOpenGLPixelFormat::initWithAttributes(
            NSOpenGLPixelFormat::alloc(),
            NonNull::from(&mut attributes).cast(),
        )
    }
    .ok_or("no OpenGL pixel format matches what the preview asks for")?;

    let gl_view = NSOpenGLView::initWithFrame_pixelFormat(
        NSOpenGLView::alloc(mtm),
        HIDDEN_SURFACE,
        Some(&*pixel_format),
    )
    .ok_or("the OpenGL view could not be created")?;
    gl_view.setWantsBestResolutionOpenGLSurface(true);
    // Last subview is topmost, which is what puts the video over the webview.
    container.addSubview(&gl_view);

    // The context only gets a drawable once its view is in a window, so this
    // has to come after the view is a subview.
    let context = gl_view
        .openGLContext()
        .ok_or("the OpenGL view has no context")?;
    context.makeCurrentContext();

    let player = Arc::new(MpvRenderPlayer::new()?);
    // mpv has no display handle to take on macOS, hardware decode is negotiated
    // by its own hwdec option instead.
    player.init_opengl(resolve_gl_symbol, ptr::null_mut(), None)?;
    eprintln!(
        "[preview] GL renderer: {} ({})",
        gl_string(GL_RENDERER),
        gl_string(GL_VERSION)
    );

    let surface = Arc::new(PreviewSurface {
        container: Retained::into_raw(container.retain()),
        gl_view: Retained::into_raw(gl_view),
        context: Retained::into_raw(context),
    });
    let rect = Arc::new(Mutex::new(SurfaceRect::default()));
    let app_handle = window.app_handle().clone();

    // A strong handle here would keep the player alive through its own update
    // callback, and mpv would never be shut down.
    let waiting_player = Arc::downgrade(&player);
    player.set_update_callback({
        let surface = Arc::clone(&surface);
        let app_handle = app_handle.clone();
        move || {
            let Some(player) = waiting_player.upgrade() else {
                return;
            };
            let surface = Arc::clone(&surface);
            let _ = app_handle.run_on_main_thread(move || draw(&surface, &player));
        }
    });

    Ok(EmbeddedPreview {
        player,
        surface,
        rect,
        app_handle,
    })
}

/// The main-thread half of the render loop. Advanced control makes calling
/// `wants_redraw` after every update callback mandatory, and it has to happen on
/// the render thread with the context current, never inside the callback.
fn draw(surface: &PreviewSurface, player: &MpvRenderPlayer) {
    let context = unsafe { &*surface.context };
    let gl_view = unsafe { &*surface.gl_view };
    context.makeCurrentContext();
    if !player.wants_redraw() {
        return;
    }
    let pixels = gl_view.convertRectToBacking(gl_view.bounds());
    if let Err(error) = player.render_opengl(
        DEFAULT_FRAMEBUFFER,
        pixels.size.width as i32,
        pixels.size.height as i32,
        FLIP_Y,
    ) {
        eprintln!("[preview] render failed: {error}");
    }
    context.flushBuffer();
    player.report_swap();
}

fn apply_rect(surface: &PreviewSurface, rect: SurfaceRect) {
    let container = unsafe { &*surface.container };
    let gl_view = unsafe { &*surface.gl_view };
    let active = rect.visible && rect.width > 0 && rect.height > 0;
    let frame = if active {
        placed_frame(container, rect)
    } else {
        HIDDEN_SURFACE
    };
    gl_view.setFrame(frame);
    gl_view.update();
}

/// The page measures its placeholder down from the top of the webview, so the y
/// coordinate has to be flipped unless the container counts down from the top
/// as well.
fn placed_frame(container: &NSView, rect: SurfaceRect) -> NSRect {
    let top = f64::from(rect.y);
    let height = f64::from(rect.height);
    let y = if container.isFlipped() {
        top
    } else {
        container.bounds().size.height - top - height
    };
    NSRect::new(
        NSPoint::new(f64::from(rect.x), y),
        NSSize::new(f64::from(rect.width), height),
    )
}

type GlGetString = unsafe extern "C" fn(name: u32) -> *const c_char;

fn gl_string(name: u32) -> String {
    let address = gl_symbol("glGetString") as usize;
    if address == 0 {
        return "unknown".to_string();
    }
    let get_string = unsafe { std::mem::transmute::<usize, GlGetString>(address) };
    let value = unsafe { get_string(name) };
    if value.is_null() {
        return "unknown".to_string();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

unsafe extern "C" fn resolve_gl_symbol(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    let Ok(name) = (unsafe { CStr::from_ptr(name) }).to_str() else {
        return ptr::null_mut();
    };
    gl_symbol(name)
}

/// Resolve a GL entry point out of the process, which already has the OpenGL
/// framework loaded once AppKit hands out an `NSOpenGLContext`, and open the
/// framework by path if that search comes up empty.
fn gl_symbol(name: &str) -> *mut c_void {
    static FRAMEWORK: OnceLock<usize> = OnceLock::new();
    let Ok(symbol) = CString::new(name) else {
        return ptr::null_mut();
    };
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, symbol.as_ptr()) };
    if !address.is_null() {
        return address;
    }
    let framework = *FRAMEWORK.get_or_init(|| unsafe {
        libc::dlopen(
            OPENGL_FRAMEWORK.as_ptr(),
            libc::RTLD_NOW | libc::RTLD_GLOBAL,
        ) as usize
    });
    if framework == 0 {
        return ptr::null_mut();
    }
    unsafe { libc::dlsym(framework as *mut c_void, symbol.as_ptr()) }
}
