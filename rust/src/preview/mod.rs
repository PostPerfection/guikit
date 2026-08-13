//! Video preview drawn inside the app window, plus the tauri commands the page
//! drives it with. Each platform supplies its own host behind `attach`.

use postkit::mpv_render::MpvRenderPlayer;
use tauri::Manager;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{attach, EmbeddedPreview};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{attach, EmbeddedPreview};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{attach, EmbeddedPreview};

/// The preview surface, or the reason there is none. Playback commands hand
/// that reason back to the page rather than failing silently.
pub enum PreviewPlayer {
    Embedded(EmbeddedPreview),
    Unavailable(String),
}

impl PreviewPlayer {
    fn player(&self) -> Result<&MpvRenderPlayer, String> {
        match self {
            PreviewPlayer::Embedded(preview) => Ok(preview.player()),
            PreviewPlayer::Unavailable(reason) => Err(reason.clone()),
        }
    }
}

/// Put the video surface on the app's window. Failure leaves the app running
/// with playback disabled, so it never stops the app from starting.
pub fn create_player(app: &tauri::App, window_label: &str) -> PreviewPlayer {
    let Some(window) = app.get_window(window_label) else {
        return PreviewPlayer::Unavailable(format!("no window labelled {window_label}"));
    };
    match attach(&window) {
        Ok(preview) => PreviewPlayer::Embedded(preview),
        Err(error) => {
            eprintln!("[preview] embedded playback unavailable: {error}");
            PreviewPlayer::Unavailable(error)
        }
    }
}

/// Report where the page's video placeholder sits, in CSS pixels from the
/// top-left of the webview, so the embedded surface can be moved to match.
#[tauri::command]
pub fn preview_set_surface(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    visible: bool,
    state: tauri::State<'_, PreviewPlayer>,
) {
    if let PreviewPlayer::Embedded(preview) = &*state {
        preview.set_surface(x, y, width, height, visible);
    }
}

/// True when the video surface came up, which is what tells the page to report
/// its placeholder position and to show the preview panel at all.
#[tauri::command]
pub fn preview_is_embedded(state: tauri::State<'_, PreviewPlayer>) -> bool {
    matches!(&*state, PreviewPlayer::Embedded(_))
}

#[tauri::command]
pub fn preview_load(
    file_path: String,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    state.player()?.load_file(&file_path)
}

#[tauri::command]
pub fn preview_play_pause(state: tauri::State<'_, PreviewPlayer>) -> Result<(), String> {
    state.player()?.play_pause()
}

#[tauri::command]
pub fn preview_seek(seconds: f64, state: tauri::State<'_, PreviewPlayer>) -> Result<(), String> {
    state.player()?.seek(seconds)
}

#[tauri::command]
pub fn preview_seek_absolute(
    seconds: f64,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    state.player()?.seek_absolute(seconds)
}

#[tauri::command]
pub fn preview_stop(state: tauri::State<'_, PreviewPlayer>) -> Result<(), String> {
    state.player()?.stop()
}

#[tauri::command]
pub fn preview_get_position(state: tauri::State<'_, PreviewPlayer>) -> Result<f64, String> {
    state.player()?.get_position()
}

#[tauri::command]
pub fn preview_get_duration(state: tauri::State<'_, PreviewPlayer>) -> Result<f64, String> {
    state.player()?.get_duration()
}

#[tauri::command]
pub fn preview_get_metadata(state: tauri::State<'_, PreviewPlayer>) -> Result<String, String> {
    state.player()?.get_metadata()
}

#[tauri::command]
pub fn preview_load_dcp(
    dir_path: String,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    state.player()?.load_package_dir(&dir_path)
}
