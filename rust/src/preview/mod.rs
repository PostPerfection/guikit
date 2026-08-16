//! Video preview drawn inside the app window, plus the tauri commands the page
//! drives it with. Each platform supplies its own host behind `attach`.

use postkit::mpv_render::MpvRenderPlayer;
use serde::Deserialize;
use tauri::Manager;

mod overlays;
use overlays::overlay_filter_chain;
pub use overlays::PreviewOverlays;

/// mpv's video filter chain, which only the overlays use.
const VIDEO_FILTER_PROPERTY: &str = "vf";
/// Options handed to the libavcodec decoder when it opens.
const DECODER_OPTIONS_PROPERTY: &str = "vd-lavc-o";
/// The file mpv currently has loaded, which a decode scale change reloads.
const PATH_PROPERTY: &str = "path";
const PAUSE_PROPERTY: &str = "pause";

/// The HUD fields added to the metadata poll, and the mpv property behind each.
const HUD_COUNTER_PROPERTIES: [(&str, &str); 5] = [
    ("dropped_frames", "frame-drop-count"),
    ("delayed_frames", "vo-delayed-frame-count"),
    ("cache_seconds", "demuxer-cache-duration"),
    ("decoder_fps", "estimated-vf-fps"),
    ("container_fps", "container-fps"),
];

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

/// Playback position and the HUD counters as one JSON object, polled by the
/// page every quarter second.
#[tauri::command]
pub fn preview_get_metadata(state: tauri::State<'_, PreviewPlayer>) -> Result<String, String> {
    let player = state.player()?;
    let counters: Vec<(&str, Option<f64>)> = HUD_COUNTER_PROPERTIES
        .iter()
        .map(|(field, property)| (*field, player.get_property_f64(property).ok()))
        .collect();
    with_hud_counters(&player.get_metadata()?, &counters)
}

#[tauri::command]
pub fn preview_load_dcp(
    dir_path: String,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    state.player()?.load_package_dir(&dir_path)
}

/// Draw the requested QC overlays over playback, or none of them.
#[tauri::command]
pub fn preview_set_overlays(
    overlays: PreviewOverlays,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    state
        .player()?
        .set_property(VIDEO_FILTER_PROPERTY, &overlay_filter_chain(&overlays))
}

/// How much of the picture the decoder reconstructs. JPEG 2000 drops DWT levels
/// to reach half and quarter, so each step costs a fraction of a full decode.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecodeScale {
    Full,
    Half,
    Quarter,
}

impl DecodeScale {
    /// The value for libavcodec's `lowres`, which halves each dimension per step.
    fn decoder_option_value(self) -> String {
        let level = match self {
            DecodeScale::Full => 0,
            DecodeScale::Half => 1,
            DecodeScale::Quarter => 2,
        };
        format!("lowres={level}")
    }
}

#[tauri::command]
pub fn preview_set_decode_scale(
    scale: DecodeScale,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    let player = state.player()?;
    player.set_property(DECODER_OPTIONS_PROPERTY, &scale.decoder_option_value())?;
    // lowres is read when the decoder opens, so the file has to be loaded again
    let Ok(path) = player.get_property_string(PATH_PROPERTY) else {
        return Ok(());
    };
    let paused = player.get_property_bool(PAUSE_PROPERTY).unwrap_or(true);
    let mut file_options = format!("pause={}", if paused { "yes" } else { "no" });
    if let Ok(position) = player.get_position() {
        file_options.push_str(&format!(",start={position}"));
    }
    player.command(&["loadfile", &path, "replace", "0", &file_options])
}

/// Append the HUD counters to the metadata postkit produced, which owns the
/// position and pause fields the transport bar reads.
fn with_hud_counters(
    metadata_json: &str,
    counters: &[(&str, Option<f64>)],
) -> Result<String, String> {
    let base = metadata_json
        .strip_suffix('}')
        .ok_or_else(|| format!("metadata is not a JSON object: {metadata_json}"))?;
    let counter_fields: String = counters
        .iter()
        .map(|(name, value)| format!(", \"{name}\": {}", json_number(*value)))
        .collect();
    Ok(format!("{base}{counter_fields}}}"))
}

fn json_number(value: Option<f64>) -> String {
    match value {
        Some(number) if number.is_finite() => number.to_string(),
        _ => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_scale_maps_to_lowres_levels() {
        assert_eq!(DecodeScale::Full.decoder_option_value(), "lowres=0");
        assert_eq!(DecodeScale::Half.decoder_option_value(), "lowres=1");
        assert_eq!(DecodeScale::Quarter.decoder_option_value(), "lowres=2");
    }

    #[test]
    fn counters_are_null_until_mpv_reports_them() {
        let counters: Vec<(&str, Option<f64>)> = HUD_COUNTER_PROPERTIES
            .iter()
            .map(|(field, _)| (*field, None))
            .collect();
        let metadata = with_hud_counters(
            r#"{"position": null, "duration": null, "paused": null, "filename": null}"#,
            &counters,
        )
        .unwrap();
        assert_eq!(
            metadata,
            r#"{"position": null, "duration": null, "paused": null, "filename": null, "dropped_frames": null, "delayed_frames": null, "cache_seconds": null, "decoder_fps": null, "container_fps": null}"#
        );
    }

    #[test]
    fn counters_carry_the_values_mpv_reports() {
        let metadata = with_hud_counters(
            r#"{"position": 1.5}"#,
            &[("dropped_frames", Some(3.0)), ("cache_seconds", Some(1.25))],
        )
        .unwrap();
        assert_eq!(
            metadata,
            r#"{"position": 1.5, "dropped_frames": 3, "cache_seconds": 1.25}"#
        );
    }

    #[test]
    fn metadata_that_is_not_an_object_fails() {
        assert!(with_hud_counters("not json", &[]).is_err());
    }
}
