//! Video preview drawn inside the app window, plus the tauri commands the page
//! drives it with. Each platform supplies its own host behind `attach`.

use std::sync::Mutex;

use postkit::mpv_render::MpvRenderPlayer;
use serde::Deserialize;
use tauri::Manager;

mod overlays;
use overlays::{overlay_filter_chain, SourceSize};
pub use overlays::{PreviewCrop, PreviewOverlays};

/// mpv's video filter chain, which only the overlays use.
const VIDEO_FILTER_PROPERTY: &str = "vf";
/// Options handed to the libavcodec decoder when it opens.
const DECODER_OPTIONS_PROPERTY: &str = "vd-lavc-o";
/// The file mpv currently has loaded, which a decode scale change reloads.
const PATH_PROPERTY: &str = "path";
const PAUSE_PROPERTY: &str = "pause";
/// The picture size the demuxer reports for the current video track, which is
/// what the container declares rather than what came out of the decoder.
const SOURCE_WIDTH_PROPERTY: &str = "current-tracks/video/demux-w";
const SOURCE_HEIGHT_PROPERTY: &str = "current-tracks/video/demux-h";
/// The size of the frame the decoder emitted, which the decode scale shrinks
/// only where the decoder implements lowres.
const DECODED_WIDTH_PROPERTY: &str = "video-params/w";
const DECODED_HEIGHT_PROPERTY: &str = "video-params/h";
/// How many tracks the loaded file has, all types counted together.
const TRACK_COUNT_PROPERTY: &str = "track-list/count";
/// The external subtitle files a reload has to load again, as a per-file option.
const SUBTITLE_FILES_OPTION: &str = "sub-files";
/// mpv's list separator, which the option value above is joined with.
#[cfg(target_os = "windows")]
const SUBTITLE_FILE_SEPARATOR: &str = ";";
#[cfg(not(target_os = "windows"))]
const SUBTITLE_FILE_SEPARATOR: &str = ":";
/// `sub-add` flag that loads a file without selecting it, so each track lands in
/// the slot chosen for it rather than in whichever one mpv prefers.
const ADD_TRACK_UNSELECTED: &str = "auto";

/// The HUD fields added to the metadata poll, and the mpv property behind each.
const HUD_COUNTER_PROPERTIES: [(&str, &str); 5] = [
    ("dropped_frames", "frame-drop-count"),
    ("delayed_frames", "vo-delayed-frame-count"),
    ("cache_seconds", "demuxer-cache-duration"),
    ("decoder_fps", "estimated-vf-fps"),
    ("container_fps", "container-fps"),
];

/// The metadata field the page's playlist advances on, and the mpv property
/// behind it. mpv only holds this true, paused on the last frame, because
/// postkit's player starts mpv with `keep-open` on.
const EOF_FIELD: &str = "eof";
const EOF_PROPERTY: &str = "eof-reached";

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
enum PreviewSurface {
    Embedded(EmbeddedPreview),
    Unavailable(String),
}

/// The player and what the page has set on it that mpv itself does not keep for
/// us: the decode scale, the subtitle files a reload has to load again, and the
/// size of the picture the crop overlay is measured against.
pub struct PreviewPlayer {
    surface: PreviewSurface,
    decode_scale: Mutex<DecodeScale>,
    subtitle_tracks: Mutex<SubtitleTracks>,
    source_size: Mutex<Option<SourceSize>>,
}

impl PreviewPlayer {
    fn new(surface: PreviewSurface) -> Self {
        PreviewPlayer {
            surface,
            decode_scale: Mutex::new(DecodeScale::default()),
            subtitle_tracks: Mutex::new(SubtitleTracks::default()),
            source_size: Mutex::new(None),
        }
    }

    /// The size the crop overlay measures against, read again every time so a
    /// reloaded track is measured as itself. mpv reports no size at all for the
    /// moment a reload takes, and the page sends the overlays inside it, so the
    /// last size read stands until another one arrives.
    fn source_size(&self, player: &MpvRenderPlayer) -> Option<SourceSize> {
        let decode_scale = *self.decode_scale.lock().unwrap();
        let mut remembered = self.source_size.lock().unwrap();
        if let Some(size) = read_source_size(player, decode_scale) {
            *remembered = Some(size);
        }
        *remembered
    }

    fn player(&self) -> Result<&MpvRenderPlayer, String> {
        match &self.surface {
            PreviewSurface::Embedded(preview) => Ok(preview.player()),
            PreviewSurface::Unavailable(reason) => Err(reason.clone()),
        }
    }

    /// Loading or stopping takes mpv's external subtitle tracks with it, so the
    /// track ids held here would name tracks that no longer exist, and the size
    /// held here would be another file's.
    fn forget_loaded_file(&self) {
        *self.subtitle_tracks.lock().unwrap() = SubtitleTracks::default();
        *self.source_size.lock().unwrap() = None;
    }
}

/// Put the video surface on the app's window. Failure leaves the app running
/// with playback disabled, so it never stops the app from starting.
pub fn create_player(app: &tauri::App, window_label: &str) -> PreviewPlayer {
    let Some(window) = app.get_window(window_label) else {
        return PreviewPlayer::new(PreviewSurface::Unavailable(format!(
            "no window labelled {window_label}"
        )));
    };
    match attach(&window) {
        Ok(preview) => PreviewPlayer::new(PreviewSurface::Embedded(preview)),
        Err(error) => {
            eprintln!("[preview] embedded playback unavailable: {error}");
            PreviewPlayer::new(PreviewSurface::Unavailable(error))
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
    if let PreviewSurface::Embedded(preview) = &state.surface {
        preview.set_surface(x, y, width, height, visible);
    }
}

/// True when the video surface came up, which is what tells the page to report
/// its placeholder position and to show the preview panel at all.
#[tauri::command]
pub fn preview_is_embedded(state: tauri::State<'_, PreviewPlayer>) -> bool {
    matches!(&state.surface, PreviewSurface::Embedded(_))
}

#[tauri::command]
pub fn preview_load(
    file_path: String,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    let player = state.player()?;
    state.forget_loaded_file();
    player.load_file(&file_path)
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
    let player = state.player()?;
    state.forget_loaded_file();
    player.stop()
}

#[tauri::command]
pub fn preview_get_position(state: tauri::State<'_, PreviewPlayer>) -> Result<f64, String> {
    state.player()?.get_position()
}

#[tauri::command]
pub fn preview_get_duration(state: tauri::State<'_, PreviewPlayer>) -> Result<f64, String> {
    state.player()?.get_duration()
}

/// Playback position, the HUD counters and the end-of-file flag as one JSON
/// object, polled by the page every quarter second.
#[tauri::command]
pub fn preview_get_metadata(state: tauri::State<'_, PreviewPlayer>) -> Result<String, String> {
    player_metadata(state.player()?)
}

fn player_metadata(player: &MpvRenderPlayer) -> Result<String, String> {
    let mut fields: Vec<(&str, String)> = HUD_COUNTER_PROPERTIES
        .iter()
        .map(|(field, property)| (*field, json_number(player.get_property_f64(property).ok())))
        .collect();
    fields.push((
        EOF_FIELD,
        json_bool(player.get_property_bool(EOF_PROPERTY).ok()),
    ));
    with_extra_fields(&player.get_metadata()?, &fields)
}

#[tauri::command]
pub fn preview_load_dcp(
    dir_path: String,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    let player = state.player()?;
    state.forget_loaded_file();
    player.load_package_dir(&dir_path)
}

/// Draw the requested QC overlays over playback, or none of them.
#[tauri::command]
pub fn preview_set_overlays(
    overlays: PreviewOverlays,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    let player = state.player()?;
    let source_size = state.source_size(player);
    player.set_property(
        VIDEO_FILTER_PROPERTY,
        &overlay_filter_chain(&overlays, source_size),
    )
}

fn read_source_size(player: &MpvRenderPlayer, decode_scale: DecodeScale) -> Option<SourceSize> {
    resolve_source_size(
        read_size(player, SOURCE_WIDTH_PROPERTY, SOURCE_HEIGHT_PROPERTY),
        read_size(player, DECODED_WIDTH_PROPERTY, DECODED_HEIGHT_PROPERTY),
        decode_scale,
    )
}

/// The size the crop is measured against: what the container declares, which no
/// decode scale changes, or the decoded frame where it declares nothing.
fn resolve_source_size(
    demuxed: Option<SourceSize>,
    decoded: Option<SourceSize>,
    decode_scale: DecodeScale,
) -> Option<SourceSize> {
    if demuxed.is_some() {
        return demuxed;
    }
    // nothing declared for this source, so take the decoded frame as having
    // been shrunk by the decode scale, which only holds for a decoder that
    // implements lowres
    decoded?.scaled_by(decode_scale.frame_divisor())
}

fn read_size(
    player: &MpvRenderPlayer,
    width_property: &str,
    height_property: &str,
) -> Option<SourceSize> {
    let width = player.get_property_f64(width_property).ok()?;
    let height = player.get_property_f64(height_property).ok()?;
    SourceSize::new(width, height)
}

/// How much of the picture the decoder reconstructs. JPEG 2000 drops DWT levels
/// to reach half and quarter, so each step costs a fraction of a full decode.
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecodeScale {
    #[default]
    Full,
    Half,
    Quarter,
}

impl DecodeScale {
    /// libavcodec's `lowres` level, which halves each dimension per step.
    fn lowres_level(self) -> u32 {
        match self {
            DecodeScale::Full => 0,
            DecodeScale::Half => 1,
            DecodeScale::Quarter => 2,
        }
    }

    fn decoder_option_value(self) -> String {
        format!("lowres={}", self.lowres_level())
    }

    /// What a decoder that implements lowres makes the frame smaller by.
    fn frame_divisor(self) -> f64 {
        f64::from(1u32 << self.lowres_level())
    }
}

#[tauri::command]
pub fn preview_set_decode_scale(
    scale: DecodeScale,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    let player = state.player()?;
    *state.decode_scale.lock().unwrap() = scale;
    player.set_property(DECODER_OPTIONS_PROPERTY, &scale.decoder_option_value())?;
    // lowres is read when the decoder opens, so the file has to be loaded again
    let Ok(path) = player.get_property_string(PATH_PROPERTY) else {
        return Ok(());
    };
    let paused = player.get_property_bool(PAUSE_PROPERTY).unwrap_or(true);
    let tracks = state.subtitle_tracks.lock().unwrap();
    let file_options = reload_file_options(paused, player.get_position().ok(), &tracks);
    player.command(&["loadfile", &path, "replace", "0", &file_options])
}

/// The per-file options the reload carries. The subtitle files ride along with
/// it because a `sub-add` sent straight after a `loadfile` is refused, the file
/// not being loaded yet, and mpv hands the same ids back to the same files as
/// long as the list keeps its order.
fn reload_file_options(paused: bool, position: Option<f64>, tracks: &SubtitleTracks) -> String {
    let mut options = format!("{PAUSE_PROPERTY}={}", yes_or_no(paused));
    if let Some(position) = position {
        options.push_str(&format!(",start={position}"));
    }
    let files: Vec<&str> = SUBTITLE_TRACK_SLOTS
        .iter()
        .filter_map(|slot| slot.state_in(tracks).file_path.as_deref())
        .collect();
    if !files.is_empty() {
        options.push_str(&format!(
            ",{SUBTITLE_FILES_OPTION}={}",
            files.join(SUBTITLE_FILE_SEPARATOR)
        ));
    }
    for slot in SUBTITLE_TRACK_SLOTS {
        if let Some(track_id) = slot.state_in(tracks).track_id {
            options.push_str(&format!(",{}={track_id}", slot.selection_property()));
        }
    }
    options
}

fn yes_or_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

/// mpv's two subtitle slots. The secondary one renders at the top of the frame,
/// which is where a caption track belongs while a subtitle track holds the
/// bottom.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleTrackSlot {
    Subtitle,
    Caption,
}

/// Both slots, in the order their files are handed to mpv, which is the order it
/// numbers the tracks in.
const SUBTITLE_TRACK_SLOTS: [SubtitleTrackSlot; 2] =
    [SubtitleTrackSlot::Subtitle, SubtitleTrackSlot::Caption];

impl SubtitleTrackSlot {
    fn selection_property(self) -> &'static str {
        match self {
            SubtitleTrackSlot::Subtitle => "sid",
            SubtitleTrackSlot::Caption => "secondary-sid",
        }
    }

    fn visibility_property(self) -> &'static str {
        match self {
            SubtitleTrackSlot::Subtitle => "sub-visibility",
            SubtitleTrackSlot::Caption => "secondary-sub-visibility",
        }
    }

    fn state_in(self, tracks: &SubtitleTracks) -> &SubtitleTrackState {
        match self {
            SubtitleTrackSlot::Subtitle => &tracks.subtitle,
            SubtitleTrackSlot::Caption => &tracks.caption,
        }
    }

    fn state_in_mut(self, tracks: &mut SubtitleTracks) -> &mut SubtitleTrackState {
        match self {
            SubtitleTrackSlot::Subtitle => &mut tracks.subtitle,
            SubtitleTrackSlot::Caption => &mut tracks.caption,
        }
    }
}

#[derive(Default)]
struct SubtitleTracks {
    subtitle: SubtitleTrackState,
    caption: SubtitleTrackState,
}

#[derive(Default)]
struct SubtitleTrackState {
    file_path: Option<String>,
    /// mpv's id for the loaded file, which `sub-remove` and the slot's selection
    /// property both name it by.
    track_id: Option<i64>,
}

/// Load a subtitle file into one of mpv's subtitle slots, or drop what is in it
/// by passing no path. Only the formats libass reads natively work: SRT, ASS or
/// SSA and WebVTT, so a wizard converts its subtitle XML to SRT first. A file
/// has to be playing, since the track is added to it.
#[tauri::command]
pub fn preview_set_subtitle_file(
    track: SubtitleTrackSlot,
    file_path: Option<String>,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    let player = state.player()?;
    let mut tracks = state.subtitle_tracks.lock().unwrap();
    track.state_in_mut(&mut tracks).file_path = file_path.clone();
    reload_subtitle_tracks(player, &mut tracks)?;
    if file_path.is_none() {
        return Ok(());
    }
    // the slot may have been toggled off earlier, and a file loaded into a
    // hidden slot shows nothing
    set_track_visibility(player, track, true)
}

/// Render or hide one of the subtitle slots, which leaves the track loaded.
#[tauri::command]
pub fn preview_set_subtitle_visibility(
    track: SubtitleTrackSlot,
    visible: bool,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    set_track_visibility(state.player()?, track, visible)
}

fn set_track_visibility(
    player: &MpvRenderPlayer,
    track: SubtitleTrackSlot,
    visible: bool,
) -> Result<(), String> {
    player.set_property(track.visibility_property(), yes_or_no(visible))
}

/// Take every loaded track off mpv and add the wanted ones back in slot order,
/// so the ids mpv hands out follow that order however the page got here.
fn reload_subtitle_tracks(
    player: &MpvRenderPlayer,
    tracks: &mut SubtitleTracks,
) -> Result<(), String> {
    let mut loaded: Vec<i64> = SUBTITLE_TRACK_SLOTS
        .iter()
        .filter_map(|slot| slot.state_in(tracks).track_id)
        .collect();
    // highest first, so a removal cannot renumber a track still to be removed
    loaded.sort_unstable_by(|left, right| right.cmp(left));
    for track_id in loaded {
        player.command(&["sub-remove", &track_id.to_string()])?;
    }
    for slot in SUBTITLE_TRACK_SLOTS {
        slot.state_in_mut(tracks).track_id = None;
    }
    for slot in SUBTITLE_TRACK_SLOTS {
        let Some(file_path) = slot.state_in(tracks).file_path.clone() else {
            continue;
        };
        player.command(&["sub-add", &file_path, ADD_TRACK_UNSELECTED])?;
        let track_id = added_track_id(player)?;
        player.set_property(slot.selection_property(), &track_id.to_string())?;
        slot.state_in_mut(tracks).track_id = Some(track_id);
    }
    Ok(())
}

/// The id of the track just added, which `sub-add` appends to the track list.
fn added_track_id(player: &MpvRenderPlayer) -> Result<i64, String> {
    let count = read_track_number(player, TRACK_COUNT_PROPERTY)?;
    read_track_number(player, &format!("track-list/{}/id", count - 1))
}

fn read_track_number(player: &MpvRenderPlayer, property: &str) -> Result<i64, String> {
    let value = player.get_property_string(property)?;
    value
        .parse()
        .map_err(|_| format!("property {property} is not a number: {value}"))
}

/// Append fields, already rendered as JSON values, to the metadata postkit
/// produced, which owns the position and pause fields the transport bar reads.
fn with_extra_fields(metadata_json: &str, fields: &[(&str, String)]) -> Result<String, String> {
    let base = metadata_json
        .strip_suffix('}')
        .ok_or_else(|| format!("metadata is not a JSON object: {metadata_json}"))?;
    let appended: String = fields
        .iter()
        .map(|(name, value)| format!(", \"{name}\": {value}"))
        .collect();
    Ok(format!("{base}{appended}}}"))
}

fn json_number(value: Option<f64>) -> String {
    match value {
        Some(number) if number.is_finite() => number.to_string(),
        _ => "null".to_string(),
    }
}

fn json_bool(value: Option<bool>) -> String {
    match value {
        Some(flag) => flag.to_string(),
        None => "null".to_string(),
    }
}

#[cfg(test)]
mod end_of_file_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded_track(file_path: &str, track_id: i64) -> SubtitleTrackState {
        SubtitleTrackState {
            file_path: Some(file_path.to_string()),
            track_id: Some(track_id),
        }
    }

    #[test]
    fn decode_scale_maps_to_lowres_levels() {
        assert_eq!(DecodeScale::Full.decoder_option_value(), "lowres=0");
        assert_eq!(DecodeScale::Half.decoder_option_value(), "lowres=1");
        assert_eq!(DecodeScale::Quarter.decoder_option_value(), "lowres=2");
    }

    #[test]
    fn decode_scale_halves_the_frame_per_level() {
        assert_eq!(DecodeScale::Full.frame_divisor(), 1.0);
        assert_eq!(DecodeScale::Half.frame_divisor(), 2.0);
        assert_eq!(DecodeScale::Quarter.frame_divisor(), 4.0);
    }

    /// The crop the job applies, in pixels off the edges of a 1920x1080 source.
    const JOB_CROP: PreviewCrop = PreviewCrop {
        left: 200,
        right: 0,
        top: 100,
        bottom: 60,
    };

    const JOB_CROP_ON_HD: &str = "lavfi=[drawbox=x=0:y=0:w=iw*0.1042:h=ih:color=red@0.35:t=fill,\
        drawbox=x=0:y=0:w=iw:h=ih*0.0926:color=red@0.35:t=fill,\
        drawbox=x=0:y=ih*0.9444:w=iw:h=ih*0.0556:color=red@0.35:t=fill,\
        drawbox=x=iw*0.1042:y=ih*0.0926:w=iw*0.8958:h=ih*0.8519:color=red@0.9:t=2]";

    fn crop_chain(source_size: Option<SourceSize>) -> String {
        overlay_filter_chain(
            &PreviewOverlays {
                crop: Some(JOB_CROP),
                crop_visible: true,
                ..Default::default()
            },
            source_size,
        )
    }

    #[test]
    fn the_crop_draws_the_same_bands_at_every_decode_scale() {
        let hd = SourceSize::new(1920.0, 1080.0);
        // a decoder that implements lowres hands back a smaller frame at each
        // step, and one that does not hands back the same frame every time
        for (decode_scale, decoded_width, decoded_height) in [
            (DecodeScale::Full, 1920.0, 1080.0),
            (DecodeScale::Half, 960.0, 540.0),
            (DecodeScale::Half, 1920.0, 1080.0),
            (DecodeScale::Quarter, 480.0, 270.0),
            (DecodeScale::Quarter, 1920.0, 1080.0),
        ] {
            let source_size = resolve_source_size(
                hd,
                SourceSize::new(decoded_width, decoded_height),
                decode_scale,
            );
            assert_eq!(crop_chain(source_size), JOB_CROP_ON_HD);
        }
    }

    #[test]
    fn a_source_that_declares_no_size_falls_back_to_the_decoded_frame() {
        let source_size =
            resolve_source_size(None, SourceSize::new(960.0, 540.0), DecodeScale::Half);
        assert_eq!(crop_chain(source_size), JOB_CROP_ON_HD);
    }

    #[test]
    fn a_crop_is_left_out_while_nothing_reports_a_size() {
        assert!(resolve_source_size(None, None, DecodeScale::Full).is_none());
        assert_eq!(crop_chain(None), "");
    }

    #[test]
    fn a_reload_with_no_subtitles_keeps_the_position_and_pause() {
        let options = reload_file_options(true, Some(1.5), &SubtitleTracks::default());
        assert_eq!(options, "pause=yes,start=1.5");
    }

    #[test]
    fn a_reload_loads_both_subtitle_files_back_into_their_slots() {
        let tracks = SubtitleTracks {
            subtitle: loaded_track("/subtitles.srt", 1),
            caption: loaded_track("/captions.srt", 2),
        };
        let options = reload_file_options(false, Some(4.0), &tracks);
        assert_eq!(
            options,
            format!(
                "pause=no,start=4,sub-files=/subtitles.srt{SUBTITLE_FILE_SEPARATOR}/captions.srt,sid=1,secondary-sid=2"
            )
        );
    }

    #[test]
    fn a_reload_carries_a_caption_track_on_its_own() {
        let tracks = SubtitleTracks {
            caption: loaded_track("/captions.srt", 1),
            ..Default::default()
        };
        let options = reload_file_options(true, None, &tracks);
        assert_eq!(options, "pause=yes,sub-files=/captions.srt,secondary-sid=1");
    }

    #[test]
    fn a_subtitle_file_nothing_has_loaded_yet_is_left_out_of_the_reload() {
        let tracks = SubtitleTracks {
            subtitle: SubtitleTrackState {
                file_path: Some("/subtitles.srt".to_string()),
                track_id: None,
            },
            ..Default::default()
        };
        let options = reload_file_options(true, None, &tracks);
        assert_eq!(options, "pause=yes,sub-files=/subtitles.srt");
    }

    #[test]
    fn counters_are_null_until_mpv_reports_them() {
        let fields: Vec<(&str, String)> = HUD_COUNTER_PROPERTIES
            .iter()
            .map(|(field, _)| (*field, json_number(None)))
            .chain(std::iter::once((EOF_FIELD, json_bool(None))))
            .collect();
        let metadata = with_extra_fields(
            r#"{"position": null, "duration": null, "paused": null, "filename": null}"#,
            &fields,
        )
        .unwrap();
        assert_eq!(
            metadata,
            r#"{"position": null, "duration": null, "paused": null, "filename": null, "dropped_frames": null, "delayed_frames": null, "cache_seconds": null, "decoder_fps": null, "container_fps": null, "eof": null}"#
        );
    }

    #[test]
    fn counters_carry_the_values_mpv_reports() {
        let metadata = with_extra_fields(
            r#"{"position": 1.5}"#,
            &[
                ("dropped_frames", json_number(Some(3.0))),
                ("cache_seconds", json_number(Some(1.25))),
            ],
        )
        .unwrap();
        assert_eq!(
            metadata,
            r#"{"position": 1.5, "dropped_frames": 3, "cache_seconds": 1.25}"#
        );
    }

    #[test]
    fn the_end_of_file_flag_is_a_json_boolean() {
        let at_the_end =
            with_extra_fields(r#"{"position": 2}"#, &[(EOF_FIELD, json_bool(Some(true)))]).unwrap();
        assert_eq!(at_the_end, r#"{"position": 2, "eof": true}"#);
        let playing =
            with_extra_fields(r#"{"position": 1}"#, &[(EOF_FIELD, json_bool(Some(false)))])
                .unwrap();
        assert_eq!(playing, r#"{"position": 1, "eof": false}"#);
    }

    #[test]
    fn metadata_that_is_not_an_object_fails() {
        assert!(with_extra_fields("not json", &[]).is_err());
    }
}
