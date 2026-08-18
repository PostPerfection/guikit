//! libmpv's rule about its render thread, held against a real render context.
//!
//! The preview surface renders on the app's main thread, and libmpv forbids a
//! call that waits on its core from that thread: the core hands the decoder's
//! buffer allocation back to the render thread, so the two wait on each other
//! and neither ever runs again. These probes hold the rule both ways round and
//! show which mpv option the trap hangs off.
//!
//! Each one needs an OpenGL driver, gets its context from EGL with no surface,
//! and ends a hang as a signal rather than as a failure, because a blocked
//! thread cannot report anything. Run one at a time:
//! `cargo test -p guikit --lib render_thread -- --ignored --exact <name>`

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use postkit::mpv_render::MpvRenderPlayer;

use super::end_of_file_tests::have_ffmpeg;
use super::{player_metadata, EOF_PROPERTY};

/// A frame size worth a real decoder allocation, so the window the deadlock
/// needs is as wide as a picture track makes it rather than as narrow as a
/// thumbnail. The clip is written straight here rather than taken from the
/// end-of-file harness, which writes one sized for speed.
const CLIP_WIDTH: u32 = 1920;
const CLIP_HEIGHT: u32 = 1080;
const CLIP_SECONDS: u32 = 2;
const FRAMES_PER_SECOND: u32 = 24;

const TARGET_WIDTH: i32 = 320;
const TARGET_HEIGHT: i32 = 180;
const FLIP_Y: bool = true;

/// mpv's direct rendering, which is what hands the decoder's buffer allocation
/// to the render thread. On by default.
const DIRECT_RENDERING_OPTION: &str = "vd-lavc-dr";

const PLAYBACK_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// How long a thread may go without a turn of its loop before it counts as
/// blocked for good.
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(20);

// ─── EGL, enough of it for a context with no surface ────────────────────────

const EGL_LIBRARY: &CStr = c"libEGL.so.1";
const EGL_OPENGL_API: u32 = 0x30A2;
const EGL_NONE: i32 = 0x3038;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_PBUFFER_BIT: i32 = 0x0001;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_BIT: i32 = 0x0008;

const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;

type EglGetDisplay = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type EglInitialize = unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32) -> u32;
type EglBindApi = unsafe extern "C" fn(u32) -> u32;
type EglChooseConfig =
    unsafe extern "C" fn(*mut c_void, *const i32, *mut *mut c_void, i32, *mut i32) -> u32;
type EglCreateContext =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *const i32) -> *mut c_void;
type EglMakeCurrent =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> u32;
type EglGetProcAddress = unsafe extern "C" fn(*const c_char) -> *mut c_void;

type GlGenIds = unsafe extern "C" fn(i32, *mut u32);
type GlBind = unsafe extern "C" fn(u32, u32);
type GlTexImage2D = unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
type GlFramebufferTexture2D = unsafe extern "C" fn(u32, u32, u32, u32, i32);
type GlCheckFramebufferStatus = unsafe extern "C" fn(u32) -> u32;

fn egl_symbol(name: &str) -> *mut c_void {
    let library = unsafe { libc::dlopen(EGL_LIBRARY.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    assert!(!library.is_null(), "libEGL is not loadable");
    let symbol = CString::new(name).unwrap();
    let address = unsafe { libc::dlsym(library, symbol.as_ptr()) };
    assert!(!address.is_null(), "libEGL has no {name}");
    address
}

unsafe extern "C" fn resolve_gl_symbol(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    let get_proc_address: EglGetProcAddress =
        unsafe { std::mem::transmute(egl_symbol("eglGetProcAddress")) };
    unsafe { get_proc_address(name) }
}

fn gl_symbol(name: &str) -> *mut c_void {
    let name = CString::new(name).unwrap();
    unsafe { resolve_gl_symbol(std::ptr::null_mut(), name.as_ptr()) }
}

/// An OpenGL context current on this thread, drawing into a texture. Hands back
/// the framebuffer mpv is given.
fn current_gl_context() -> i32 {
    unsafe {
        let get_display: EglGetDisplay = std::mem::transmute(egl_symbol("eglGetDisplay"));
        let initialize: EglInitialize = std::mem::transmute(egl_symbol("eglInitialize"));
        let bind_api: EglBindApi = std::mem::transmute(egl_symbol("eglBindAPI"));
        let choose_config: EglChooseConfig = std::mem::transmute(egl_symbol("eglChooseConfig"));
        let create_context: EglCreateContext = std::mem::transmute(egl_symbol("eglCreateContext"));
        let make_current: EglMakeCurrent = std::mem::transmute(egl_symbol("eglMakeCurrent"));

        let display = get_display(std::ptr::null_mut());
        assert!(!display.is_null(), "no EGL display");
        let (mut major, mut minor) = (0, 0);
        assert!(
            initialize(display, &mut major, &mut minor) != 0,
            "eglInitialize failed"
        );
        assert!(bind_api(EGL_OPENGL_API) != 0, "eglBindAPI failed");

        let config_attributes = [
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE,
            EGL_OPENGL_BIT,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
            EGL_NONE,
        ];
        let mut config: *mut c_void = std::ptr::null_mut();
        let mut count = 0;
        assert!(
            choose_config(
                display,
                config_attributes.as_ptr(),
                &mut config,
                1,
                &mut count
            ) != 0
                && count == 1,
            "no EGL config"
        );
        let context = create_context(display, config, std::ptr::null_mut(), [EGL_NONE].as_ptr());
        assert!(!context.is_null(), "eglCreateContext failed");
        assert!(
            make_current(display, std::ptr::null_mut(), std::ptr::null_mut(), context) != 0,
            "eglMakeCurrent with no surface failed"
        );

        let gen_textures: GlGenIds = std::mem::transmute(gl_symbol("glGenTextures"));
        let bind_texture: GlBind = std::mem::transmute(gl_symbol("glBindTexture"));
        let tex_image: GlTexImage2D = std::mem::transmute(gl_symbol("glTexImage2D"));
        let gen_framebuffers: GlGenIds = std::mem::transmute(gl_symbol("glGenFramebuffers"));
        let bind_framebuffer: GlBind = std::mem::transmute(gl_symbol("glBindFramebuffer"));
        let framebuffer_texture: GlFramebufferTexture2D =
            std::mem::transmute(gl_symbol("glFramebufferTexture2D"));
        let check_status: GlCheckFramebufferStatus =
            std::mem::transmute(gl_symbol("glCheckFramebufferStatus"));

        let mut texture = 0;
        gen_textures(1, &mut texture);
        bind_texture(GL_TEXTURE_2D, texture);
        tex_image(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as i32,
            TARGET_WIDTH,
            TARGET_HEIGHT,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            std::ptr::null(),
        );
        let mut framebuffer = 0;
        gen_framebuffers(1, &mut framebuffer);
        bind_framebuffer(GL_FRAMEBUFFER, framebuffer);
        framebuffer_texture(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT0,
            GL_TEXTURE_2D,
            texture,
            0,
        );
        assert_eq!(
            check_status(GL_FRAMEBUFFER),
            GL_FRAMEBUFFER_COMPLETE,
            "framebuffer incomplete"
        );
        framebuffer as i32
    }
}

// ─── The probes ────────────────────────────────────────────────────────────

struct RenderThread {
    player: Arc<MpvRenderPlayer>,
    framebuffer: i32,
}

impl RenderThread {
    /// Binds the render context to the calling thread, which is the render thread
    /// from here on.
    fn bound_to_this_thread() -> Self {
        let framebuffer = current_gl_context();
        let player = Arc::new(MpvRenderPlayer::new().unwrap());
        player
            .init_opengl(resolve_gl_symbol, std::ptr::null_mut(), None)
            .unwrap();
        RenderThread {
            player,
            framebuffer,
        }
    }

    /// Take the frame mpv offers and tell it the frame reached the screen, which
    /// is all the app's main thread is allowed to do with the player.
    fn pump_once(&self) {
        if self.player.wants_redraw() {
            self.player
                .render_opengl(self.framebuffer, TARGET_WIDTH, TARGET_HEIGHT, FLIP_Y)
                .unwrap();
            self.player.report_swap();
        }
    }
}

fn write_clip(path: &std::path::Path) {
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!(
                "testsrc=size={CLIP_WIDTH}x{CLIP_HEIGHT}:rate={FRAMES_PER_SECOND}:duration={CLIP_SECONDS}"
            ),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .expect("ffmpeg");
    assert!(
        out.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The metadata poll the page runs, which is the shortest thing that waits on
/// mpv's core. Reads until the file plays out.
fn poll_until_the_end(player: &MpvRenderPlayer, beat: impl Fn()) {
    let deadline = Instant::now() + PLAYBACK_TIMEOUT;
    let mut polls = 0u64;
    while Instant::now() < deadline {
        beat();
        player_metadata(player).unwrap();
        polls += 1;
        if player.get_property_bool(EOF_PROPERTY).unwrap_or(false) {
            eprintln!("[probe] played out after {polls} polls");
            return;
        }
    }
    panic!("the clip never played out");
}

/// A property read on the render thread, which is what a plain tauri command
/// does, because its body runs inline on the thread dispatching the webview's
/// IPC and that is the thread the surface renders on.
#[test]
#[ignore = "hangs on purpose and needs a GL driver, run on its own"]
fn a_property_read_on_the_render_thread_deadlocks() {
    let Some(dir) = clip_directory() else { return };
    let render = RenderThread::bound_to_this_thread();
    abort_when_the_beating_stops();
    render.player.load_file(&clip_path(&dir)).unwrap();

    poll_until_the_end(&render.player, || {
        beat();
        render.pump_once();
    });
    panic!("the read on the render thread came back, which it is not expected to");
}

/// The same playback with the reads on a thread of their own, which is what the
/// preview commands being `(async)` buys, and the render thread doing nothing
/// but render.
#[test]
#[ignore = "needs a GL driver, run on its own"]
fn a_property_read_off_the_render_thread_plays_out() {
    let Some(dir) = clip_directory() else { return };
    let render = RenderThread::bound_to_this_thread();
    abort_when_the_beating_stops();
    render.player.load_file(&clip_path(&dir)).unwrap();

    let reader = Arc::clone(&render.player);
    let polling = std::thread::spawn(move || poll_until_the_end(&reader, || {}));
    while !polling.is_finished() {
        beat();
        render.pump_once();
        std::thread::sleep(POLL_INTERVAL);
    }
    polling.join().unwrap();
}

/// The render thread reading properties again, with mpv's direct rendering off.
/// That is the option the trap hangs off: with it gone the core allocates the
/// decoder's buffers itself and never waits on the render thread for them.
#[test]
#[ignore = "needs a GL driver, run on its own"]
fn a_property_read_on_the_render_thread_survives_without_direct_rendering() {
    let Some(dir) = clip_directory() else { return };
    let render = RenderThread::bound_to_this_thread();
    render
        .player
        .set_property(DIRECT_RENDERING_OPTION, "no")
        .unwrap();
    abort_when_the_beating_stops();
    render.player.load_file(&clip_path(&dir)).unwrap();

    poll_until_the_end(&render.player, || {
        beat();
        render.pump_once();
    });
}

/// The page's real quarter-second poll: `apply_overlays` then the metadata
/// read, off the render thread, with every overlay drawn. The transport bar is
/// dead unless the position advances and the pause state reads false while the
/// clip is actually playing, not only once it stops.
#[test]
#[ignore = "needs a GL driver, run on its own"]
fn the_metadata_poll_reads_live_values_while_the_clip_plays() {
    let Some(dir) = clip_directory() else { return };
    let render = RenderThread::bound_to_this_thread();
    abort_when_the_beating_stops();
    render.player.load_file(&clip_path(&dir)).unwrap();

    let reader = Arc::clone(&render.player);
    let polling = std::thread::spawn(move || {
        let state =
            super::end_of_file_tests::overlay_state(super::end_of_file_tests::every_overlay());
        let deadline = Instant::now() + PLAYBACK_TIMEOUT;
        let mut playing_read = false;
        while Instant::now() < deadline {
            beat();
            super::apply_overlays(&reader, &state).unwrap();
            let metadata = player_metadata(&reader).unwrap();
            let position = reader.get_position().unwrap_or(0.0);
            if metadata.contains("\"paused\": false") && position > 0.0 {
                playing_read = true;
            }
            if reader.get_property_bool(EOF_PROPERTY).unwrap_or(false) {
                assert!(
                    playing_read,
                    "no poll saw the clip playing before it ended, last read {metadata}"
                );
                return;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        panic!("the clip never played out");
    });
    while !polling.is_finished() {
        beat();
        render.pump_once();
        std::thread::sleep(POLL_INTERVAL);
    }
    polling.join().unwrap();
}

fn clip_directory() -> Option<tempfile::TempDir> {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    write_clip(&dir.path().join("clip.mxf"));
    Some(dir)
}

fn clip_path(dir: &tempfile::TempDir) -> String {
    dir.path().join("clip.mxf").to_string_lossy().into_owned()
}

static HEARTBEAT: AtomicU64 = AtomicU64::new(0);

fn beat() {
    HEARTBEAT.fetch_add(1, Ordering::Release);
}

/// A blocked thread cannot fail a test, because nothing on it runs again, so the
/// hang is turned into a signal from a thread of its own.
fn abort_when_the_beating_stops() {
    std::thread::spawn(|| loop {
        let seen = HEARTBEAT.load(Ordering::Acquire);
        std::thread::sleep(HEARTBEAT_TIMEOUT);
        if HEARTBEAT.load(Ordering::Acquire) == seen {
            eprintln!("[probe] nothing has run for {HEARTBEAT_TIMEOUT:?}, aborting");
            std::process::abort();
        }
    });
}
