use postkit::mpv_render::MpvRenderPlayer;

const NOT_IMPLEMENTED: &str = "the embedded preview is not implemented on windows yet";

/// No host exists yet, so the type has no values and every method on it is
/// unreachable. Filling in `attach` turns this into a real struct.
pub enum EmbeddedPreview {}

impl EmbeddedPreview {
    pub fn player(&self) -> &MpvRenderPlayer {
        match *self {}
    }

    pub fn set_surface(&self, x: i32, y: i32, width: i32, height: i32, visible: bool) {
        let _ = (x, y, width, height, visible);
        match *self {}
    }
}

pub fn attach(window: &tauri::Window) -> Result<EmbeddedPreview, String> {
    let _ = window;
    Err(NOT_IMPLEMENTED.to_string())
}
