use std::ffi::c_void;
use std::path::Path;
use std::sync::{Arc, Mutex};

use postkit::grok_player::{GetProcAddressFn, GrokPlayer};
use postkit::mpv_render::{MpvRenderPlayer, NativeDisplay};

use super::{FRAME_BACK_STEP, FRAME_STEP};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Mpv,
    Grok,
}

impl Backend {
    fn name(self) -> &'static str {
        match self {
            Backend::Mpv => "mpv",
            Backend::Grok => "grok",
        }
    }
}

pub struct Player {
    mpv: MpvRenderPlayer,
    grok: GrokPlayer,
    backend: Mutex<Backend>,
}

impl Player {
    pub fn new() -> Result<Self, String> {
        Ok(Player {
            mpv: MpvRenderPlayer::new()?,
            grok: GrokPlayer::new(),
            backend: Mutex::new(Backend::Mpv),
        })
    }

    pub fn mpv(&self) -> &MpvRenderPlayer {
        &self.mpv
    }

    pub fn grok(&self) -> &GrokPlayer {
        &self.grok
    }

    pub fn active(&self) -> Backend {
        *self.backend.lock().unwrap()
    }

    // ─── render backends ───────────────────────────────────────────────────

    pub fn is_initialized(&self) -> bool {
        self.mpv.is_initialized()
    }

    pub fn init_opengl(
        &self,
        get_proc_address: GetProcAddressFn,
        get_proc_address_ctx: *mut c_void,
        native_display: Option<NativeDisplay>,
    ) -> Result<(), String> {
        self.mpv
            .init_opengl(get_proc_address, get_proc_address_ctx, native_display)?;
        self.grok
            .init_opengl(get_proc_address, get_proc_address_ctx)
    }

    pub fn init_software(&self) -> Result<(), String> {
        self.mpv.init_software()?;
        self.grok.init_software()
    }

    pub fn set_update_callback<F: Fn() + Send + Sync + 'static>(&self, callback: F) {
        let callback = Arc::new(callback);
        let for_mpv = Arc::clone(&callback);
        self.mpv.set_update_callback(move || for_mpv());
        self.grok.set_update_callback(move || callback());
    }

    // mpv's half runs whichever backend is on screen, or its core blocks
    pub fn wants_redraw(&self) -> bool {
        let mpv = self.mpv.wants_redraw();
        self.grok.wants_redraw() || mpv
    }

    pub fn render_opengl(
        &self,
        framebuffer: i32,
        width: i32,
        height: i32,
        flip_y: bool,
    ) -> Result<(), String> {
        match self.active() {
            Backend::Mpv => self.mpv.render_opengl(framebuffer, width, height, flip_y),
            Backend::Grok => self.grok.render_opengl(framebuffer, width, height, flip_y),
        }
    }

    pub fn render_software(
        &self,
        width: usize,
        height: usize,
        target: &mut [u8],
    ) -> Result<(), String> {
        match self.active() {
            Backend::Mpv => self.mpv.render_software(width, height, target),
            Backend::Grok => self.grok.render_software(width, height, target),
        }
    }

    pub fn report_swap(&self) {
        self.mpv.report_swap();
    }

    // ─── loading ───────────────────────────────────────────────────────────

    pub fn load_source(&self, path: &str) -> Result<(), String> {
        if GrokPlayer::accepts(Path::new(path)) {
            return self.start_grok(path);
        }
        self.start_mpv(path);
        self.mpv.load_file(path)
    }

    pub fn load_package_dir(&self, path: &str) -> Result<(), String> {
        if GrokPlayer::accepts(Path::new(path)) {
            return self.start_grok(path);
        }
        self.start_mpv(path);
        self.mpv.load_package_dir(path)
    }

    // the page sends no play after a load, mpv's loadfile starts playing itself
    fn start_grok(&self, path: &str) -> Result<(), String> {
        self.take_over(Backend::Grok, path);
        self.mpv.stop()?;
        self.grok.load(Path::new(path))?;
        self.grok.set_paused(false);
        Ok(())
    }

    fn start_mpv(&self, path: &str) {
        self.take_over(Backend::Mpv, path);
        self.grok.stop();
    }

    fn take_over(&self, backend: Backend, path: &str) {
        *self.backend.lock().unwrap() = backend;
        eprintln!("[preview] backend: {} for {path}", backend.name());
    }

    pub fn stop(&self) -> Result<(), String> {
        self.grok.stop();
        self.mpv.stop()
    }

    // ─── transport ─────────────────────────────────────────────────────────

    pub fn play_pause(&self) -> Result<(), String> {
        match self.active() {
            Backend::Mpv => self.mpv.play_pause(),
            Backend::Grok => {
                self.grok.play_pause();
                Ok(())
            }
        }
    }

    pub fn seek(&self, seconds: f64) -> Result<(), String> {
        match self.active() {
            Backend::Mpv => self.mpv.seek(seconds),
            Backend::Grok => {
                self.grok.seek(seconds);
                Ok(())
            }
        }
    }

    pub fn seek_absolute(&self, seconds: f64) -> Result<(), String> {
        match self.active() {
            Backend::Mpv => self.mpv.seek_absolute(seconds),
            Backend::Grok => {
                self.grok.seek_absolute(seconds);
                Ok(())
            }
        }
    }

    pub fn frame_step(&self) -> Result<(), String> {
        match self.active() {
            Backend::Mpv => self.mpv.command(&[FRAME_STEP]),
            Backend::Grok => {
                self.grok.frame_step();
                Ok(())
            }
        }
    }

    pub fn frame_back_step(&self) -> Result<(), String> {
        match self.active() {
            Backend::Mpv => self.mpv.command(&[FRAME_BACK_STEP]),
            Backend::Grok => {
                self.grok.frame_back_step();
                Ok(())
            }
        }
    }

    pub fn position(&self) -> Result<f64, String> {
        match self.active() {
            Backend::Mpv => self.mpv.get_position(),
            Backend::Grok => self
                .grok
                .position()
                .ok_or_else(|| "nothing is loaded".to_string()),
        }
    }

    pub fn duration(&self) -> Result<f64, String> {
        match self.active() {
            Backend::Mpv => self.mpv.get_duration(),
            Backend::Grok => self
                .grok
                .duration()
                .ok_or_else(|| "nothing is loaded".to_string()),
        }
    }
}
