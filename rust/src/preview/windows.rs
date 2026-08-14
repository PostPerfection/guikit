//! Hosts libmpv's OpenGL output in a child window layered over the webview.
//!
//! WebView2 owns a child window covering tauri's whole client area, so the
//! preview is a sibling child window raised above it and positioned from the
//! page: the frontend reports where its placeholder element sits and the window
//! is moved to match. A window and its GL context belong to the thread that
//! created them, so everything that touches either runs on the main thread.

use std::ffi::{c_char, c_void, CStr};
use std::mem::size_of;
use std::ptr;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use postkit::mpv_render::MpvRenderPlayer;
use tauri::Manager;
use windows::core::{w, Error, PCSTR, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, GetDC, HDC, PAINTSTRUCT};
use windows::Win32::Graphics::OpenGL::{
    glGetString, wglCreateContext, wglDeleteContext, wglGetProcAddress, wglMakeCurrent,
    ChoosePixelFormat, SetPixelFormat, SwapBuffers, GL_RENDERER, GL_VERSION, HGLRC,
    PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW, LoadCursorW,
    RegisterClassExW, SetWindowLongPtrW, SetWindowPos, CS_OWNDC, GWLP_USERDATA, HWND_TOP,
    IDC_ARROW, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_SHOWWINDOW, WINDOW_EX_STYLE, WM_ERASEBKGND,
    WM_NCDESTROY, WM_PAINT, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS,
};

const PREVIEW_WINDOW_CLASS: PCWSTR = w!("PostPerfectionEmbeddedPreview");
const OPENGL_LIBRARY: PCWSTR = w!("opengl32.dll");

/// A WGL window context draws into the default framebuffer rather than into one
/// the toolkit owns, so mpv's render target is always zero.
const DEFAULT_FRAMEBUFFER: i32 = 0;

/// The WGL default framebuffer has its origin at the bottom left, which is the
/// orientation mpv already draws for.
const FLIP_Y: bool = false;

/// The only value `PIXELFORMATDESCRIPTOR` has ever been versioned with.
const PIXEL_FORMAT_VERSION: u16 = 1;
const COLOR_BITS: u8 = 32;

/// An inactive preview shrinks to this rather than being torn down: mpv's
/// render context lives on the GL context, and advanced control needs the
/// render loop still answering.
const HIDDEN_SURFACE_SIZE: i32 = 1;

/// `WM_PAINT` is answered with zero, and `WM_ERASEBKGND` with non-zero to claim
/// the background is already erased, which it is: the video covers every pixel.
const MESSAGE_HANDLED: LRESULT = LRESULT(0);
const BACKGROUND_ERASED: LRESULT = LRESULT(1);

/// The window, its device context and its GL context, all created together and
/// destroyed together.
struct GlSurface {
    window: HWND,
    device_context: HDC,
    gl_context: HGLRC,
}

// Sound because the handles are only ever touched from the main thread, which
// is the thread that created them.
unsafe impl Send for GlSurface {}
unsafe impl Sync for GlSurface {}

impl Drop for GlSurface {
    fn drop(&mut self) {
        unsafe {
            let _ = wglMakeCurrent(self.device_context, HGLRC::default());
            let _ = wglDeleteContext(self.gl_context);
            let _ = DestroyWindow(self.window);
        }
    }
}

/// What a repaint draws, reached from the window procedure through the window's
/// own `GWLP_USERDATA` slot. Both references are weak so that the window, which
/// owns this, cannot keep alive the surface whose teardown destroys it.
struct PaintTarget {
    surface: Weak<GlSurface>,
    player: Weak<MpvRenderPlayer>,
}

/// Hand the window everything a repaint needs. The window owns it from here
/// until `WM_NCDESTROY`, the last message it can ever be sent.
fn install_paint_target(surface: &Arc<GlSurface>, player: &Arc<MpvRenderPlayer>) {
    let target = Box::new(PaintTarget {
        surface: Arc::downgrade(surface),
        player: Arc::downgrade(player),
    });
    unsafe {
        SetWindowLongPtrW(
            surface.window,
            GWLP_USERDATA,
            Box::into_raw(target) as isize,
        );
    }
}

fn paint_target(window: HWND) -> Option<(Arc<GlSurface>, Arc<MpvRenderPlayer>)> {
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) };
    if pointer == 0 {
        return None;
    }
    let target = unsafe { &*(pointer as *const PaintTarget) };
    Some((target.surface.upgrade()?, target.player.upgrade()?))
}

fn release_paint_target(window: HWND) {
    let pointer = unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, 0) };
    if pointer != 0 {
        drop(unsafe { Box::from_raw(pointer as *mut PaintTarget) });
    }
}

#[derive(Clone, Copy, Default)]
struct SurfaceRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    visible: bool,
}

pub struct EmbeddedPreview {
    // The player is declared before the surface so that mpv's render context is
    // freed while the GL context it was created with is still alive.
    player: Arc<MpvRenderPlayer>,
    surface: Arc<GlSurface>,
    rect: Arc<Mutex<SurfaceRect>>,
    app: tauri::AppHandle,
    scale_factor: f64,
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
        let player = Arc::clone(&self.player);
        let rect = Arc::clone(&self.rect);
        let scale_factor = self.scale_factor;
        let _ = self.app.run_on_main_thread(move || {
            let rect = *rect.lock().unwrap();
            apply_rect(&surface, rect, scale_factor);
            make_current_and_draw(&surface, &player);
        });
    }
}

/// Put a GL child window over the window's webview and hand back the player
/// driving it. The window and its context stay on the calling thread, which is
/// the main thread the app is set up on.
pub fn attach(window: &tauri::Window) -> Result<EmbeddedPreview, String> {
    let parent = window.hwnd().map_err(|error| error.to_string())?;
    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let surface = Arc::new(create_gl_surface(parent)?);

    let player = Arc::new(MpvRenderPlayer::new()?);
    // `create_gl_surface` left the GL context current on this thread. Windows
    // has no native display handle to pass, so mpv negotiates hardware decode.
    player.init_opengl(resolve_gl_symbol, ptr::null_mut(), None)?;
    eprintln!(
        "[preview] GL renderer: {} ({})",
        gl_string(GL_RENDERER),
        gl_string(GL_VERSION)
    );

    install_paint_target(&surface, &player);

    let app = window.app_handle().clone();
    // A weak player keeps the update callback, which the player itself owns,
    // from holding the player alive forever.
    let weak_player = Arc::downgrade(&player);
    let redraw_surface = Arc::clone(&surface);
    let redraw_app = app.clone();
    player.set_update_callback(move || {
        let surface = Arc::clone(&redraw_surface);
        let weak_player = weak_player.clone();
        let _ = redraw_app.run_on_main_thread(move || {
            let Some(player) = weak_player.upgrade() else {
                return;
            };
            if !make_current(&surface) {
                return;
            }
            if player.wants_redraw() {
                draw(&surface, &player);
            }
        });
    });

    Ok(EmbeddedPreview {
        player,
        surface,
        rect: Arc::new(Mutex::new(SurfaceRect::default())),
        app,
        scale_factor,
    })
}

fn create_gl_surface(parent: HWND) -> Result<GlSurface, String> {
    register_window_class()?;
    let instance = module_handle()?;
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PREVIEW_WINDOW_CLASS,
            PCWSTR::null(),
            // Clipping siblings is what stops WebView2, which covers the same
            // rectangle, from painting over the video.
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            HIDDEN_SURFACE_SIZE,
            HIDDEN_SURFACE_SIZE,
            Some(parent),
            None,
            Some(HINSTANCE::from(instance)),
            None,
        )
    }
    .map_err(|error| format!("creating the preview window failed: {error}"))?;

    match create_gl_context(window) {
        Ok((device_context, gl_context)) => Ok(GlSurface {
            window,
            device_context,
            gl_context,
        }),
        Err(error) => {
            let _ = unsafe { DestroyWindow(window) };
            Err(error)
        }
    }
}

/// Give the window a double-buffered RGBA pixel format and a GL context, and
/// leave that context current on the calling thread.
fn create_gl_context(window: HWND) -> Result<(HDC, HGLRC), String> {
    // The class is registered with `CS_OWNDC`, so this device context belongs
    // to the window and stays valid until the window is destroyed.
    let device_context = unsafe { GetDC(Some(window)) };
    if device_context.is_invalid() {
        return Err("the preview window has no device context".to_string());
    }

    let descriptor = PIXELFORMATDESCRIPTOR {
        nSize: size_of::<PIXELFORMATDESCRIPTOR>() as u16,
        nVersion: PIXEL_FORMAT_VERSION,
        dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
        iPixelType: PFD_TYPE_RGBA,
        cColorBits: COLOR_BITS,
        ..Default::default()
    };
    let format = unsafe { ChoosePixelFormat(device_context, &descriptor) };
    if format == 0 {
        return Err(format!("no usable pixel format: {}", Error::from_win32()));
    }
    unsafe { SetPixelFormat(device_context, format, &descriptor) }
        .map_err(|error| format!("SetPixelFormat failed: {error}"))?;

    let gl_context = unsafe { wglCreateContext(device_context) }
        .map_err(|error| format!("creating the GL context failed: {error}"))?;
    if let Err(error) = unsafe { wglMakeCurrent(device_context, gl_context) } {
        let _ = unsafe { wglDeleteContext(gl_context) };
        return Err(format!("wglMakeCurrent failed: {error}"));
    }
    Ok((device_context, gl_context))
}

/// `RegisterClassExW` rejects a duplicate class name, and `attach` can be
/// reached more than once in a process, so the outcome is decided once.
fn register_window_class() -> Result<(), String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| {
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_OWNDC,
                lpfnWndProc: Some(preview_window_procedure),
                hInstance: HINSTANCE::from(module_handle()?),
                hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
                lpszClassName: PREVIEW_WINDOW_CLASS,
                ..Default::default()
            };
            if unsafe { RegisterClassExW(&class) } == 0 {
                return Err(format!(
                    "registering the preview window class failed: {}",
                    Error::from_win32()
                ));
            }
            Ok(())
        })
        .clone()
}

fn module_handle() -> Result<HMODULE, String> {
    unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map_err(|error| format!("GetModuleHandleW failed: {error}"))
}

/// The window takes no input, so painting and the teardown of what painting
/// reads are the only messages it answers itself.
unsafe extern "system" fn preview_window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_PAINT => {
            repaint(window);
            MESSAGE_HANDLED
        }
        WM_ERASEBKGND => BACKGROUND_ERASED,
        WM_NCDESTROY => {
            release_paint_target(window);
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// Redraw the frame mpv already has, for the window being uncovered or grown.
/// `WM_PAINT` is delivered on the thread that owns the window, which is the
/// main thread, so the render loop's threading rule already holds here.
fn repaint(window: HWND) {
    let mut paint = PAINTSTRUCT::default();
    // The pair is what clears the update region. Skipping it leaves the region
    // dirty and Windows sends WM_PAINT again forever.
    let _ = unsafe { BeginPaint(window, &mut paint) };
    if let Some((surface, player)) = paint_target(window) {
        make_current_and_draw(&surface, &player);
    }
    let _ = unsafe { EndPaint(window, &paint) };
}

/// Move the surface to the page's placeholder, converting the CSS pixels the
/// page reports into the physical pixels Win32 wants, and raise it above the
/// WebView2 child that shares the rectangle.
fn apply_rect(surface: &GlSurface, rect: SurfaceRect, scale_factor: f64) {
    let active = rect.visible && rect.width > 0 && rect.height > 0;
    let physical = |value: i32| (f64::from(value) * scale_factor).round() as i32;
    let (x, y, width, height, visibility) = if active {
        (
            physical(rect.x),
            physical(rect.y),
            physical(rect.width),
            physical(rect.height),
            SWP_SHOWWINDOW,
        )
    } else {
        (
            0,
            0,
            HIDDEN_SURFACE_SIZE,
            HIDDEN_SURFACE_SIZE,
            SWP_HIDEWINDOW,
        )
    };
    let placed = unsafe {
        SetWindowPos(
            surface.window,
            Some(HWND_TOP),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | visibility,
        )
    };
    if let Err(error) = placed {
        eprintln!("[preview] moving the video surface failed: {error}");
    }
}

fn make_current(surface: &GlSurface) -> bool {
    match unsafe { wglMakeCurrent(surface.device_context, surface.gl_context) } {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[preview] wglMakeCurrent failed: {error}");
            false
        }
    }
}

fn make_current_and_draw(surface: &GlSurface, player: &MpvRenderPlayer) {
    if make_current(surface) {
        draw(surface, player);
    }
}

fn draw(surface: &GlSurface, player: &MpvRenderPlayer) {
    let (width, height) = client_size(surface.window);
    if let Err(error) = player.render_opengl(DEFAULT_FRAMEBUFFER, width, height, FLIP_Y) {
        eprintln!("[preview] render failed: {error}");
        return;
    }
    if let Err(error) = unsafe { SwapBuffers(surface.device_context) } {
        eprintln!("[preview] SwapBuffers failed: {error}");
    }
    player.report_swap();
}

fn client_size(window: HWND) -> (i32, i32) {
    let mut client = RECT::default();
    if let Err(error) = unsafe { GetClientRect(window, &mut client) } {
        eprintln!("[preview] GetClientRect failed: {error}");
        return (HIDDEN_SURFACE_SIZE, HIDDEN_SURFACE_SIZE);
    }
    (client.right - client.left, client.bottom - client.top)
}

fn gl_string(name: u32) -> String {
    let value = unsafe { glGetString(name) };
    if value.is_null() {
        return "unknown".to_string();
    }
    unsafe { CStr::from_ptr(value as *const c_char) }
        .to_string_lossy()
        .into_owned()
}

unsafe extern "C" fn resolve_gl_symbol(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    let name = PCSTR(name as *const u8);
    if let Some(entry_point) = unsafe { wglGetProcAddress(name) } {
        return entry_point as usize as *mut c_void;
    }
    // wglGetProcAddress reports nothing for the OpenGL 1.1 entry points, which
    // opengl32.dll exports directly instead.
    let Some(library) = opengl_library() else {
        return ptr::null_mut();
    };
    match unsafe { GetProcAddress(library, name) } {
        Some(entry_point) => entry_point as usize as *mut c_void,
        None => ptr::null_mut(),
    }
}

/// opengl32.dll is already loaded, so this only bumps a refcount.
fn opengl_library() -> Option<HMODULE> {
    static LIBRARY: OnceLock<usize> = OnceLock::new();
    let address = *LIBRARY.get_or_init(|| match unsafe { LoadLibraryW(OPENGL_LIBRARY) } {
        Ok(library) => library.0 as usize,
        Err(error) => {
            eprintln!("[preview] loading opengl32.dll failed: {error}");
            0
        }
    });
    (address != 0).then(|| HMODULE(address as *mut c_void))
}
