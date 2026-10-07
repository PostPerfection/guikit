use postkit::grok_player::{
    sound_output_device_names, GrokPlayer, PictureMasks, PictureScaling, PresentationSettings,
    SoundOutputLayout, StereoOutput, SubtitlePresentation,
};
use postkit::preview::resolve_picture;
use postkit::subtitle_formats::Rgba;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{Backend, Player, PreviewPlayer};

const PERCENT: f32 = 100.0;
// the grok player's metadata fields the loaded picture is read from
const SOURCE_FIELD: &str = "source";
const STEREOSCOPIC_FIELD: &str = "stereoscopic";
// unavailable while mpv has nothing loaded
const MPV_PATH_PROPERTY: &str = "path";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PictureControls {
    // a gain on the displayed rgb, 1 leaves the picture alone
    pub brightness: f32,
    pub masks_percent: MaskPercents,
    pub scaling: Scaling,
}

impl Default for PictureControls {
    fn default() -> Self {
        PictureControls {
            brightness: PresentationSettings::default().brightness,
            masks_percent: MaskPercents::default(),
            scaling: Scaling::default(),
        }
    }
}

// how much of the picture each edge hides, in percent of its height or width
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskPercents {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scaling {
    #[default]
    Fit,
    Fill,
    Native,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SoundLayout {
    #[default]
    Stereo,
    FivePointOne,
    SevenPointOne,
    Automatic,
}

// what a 3D source shows, a mono source ignores it
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StereoMode {
    #[default]
    LeftEye,
    RightEye,
    SideBySide,
    TopAndBottom,
}

impl StereoMode {
    fn output(self) -> StereoOutput {
        match self {
            StereoMode::LeftEye => StereoOutput::LeftEye,
            StereoMode::RightEye => StereoOutput::RightEye,
            StereoMode::SideBySide => StereoOutput::SideBySide,
            StereoMode::TopAndBottom => StereoOutput::TopAndBottom,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SoundControls {
    // None plays on the system default device
    pub device: Option<String>,
    pub layout: SoundLayout,
    // positive delays the sound behind the picture
    pub delay_milliseconds: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleControls {
    // percent of the frame height, positive moves the text up
    pub offset_percent: f32,
    // RRGGBB or RRGGBBAA replacing the text colour, None keeps the colour of each cue
    pub colour: Option<String>,
}

// the controls only reach the grok player, so a file on mpv looks and sounds the same
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerControlOutcome {
    pub mpv_file_unchanged: bool,
}

// what is loaded, as far as which player controls reach it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LoadedPicture {
    Nothing,
    MpvFile,
    // an AS-DCP package or track file, or bare codestreams
    Dcp { stereoscopic: bool },
    // AS-02, shown in the colour its track signals rather than through the monitor profile
    Imf,
}

impl PictureControls {
    fn presentation(&self) -> PresentationSettings {
        let fraction = |percent: f32| percent / PERCENT;
        PresentationSettings {
            brightness: self.brightness,
            masks: PictureMasks {
                top: fraction(self.masks_percent.top),
                bottom: fraction(self.masks_percent.bottom),
                left: fraction(self.masks_percent.left),
                right: fraction(self.masks_percent.right),
            },
            scaling: match self.scaling {
                Scaling::Fit => PictureScaling::Fit,
                Scaling::Fill => PictureScaling::Fill,
                Scaling::Native => PictureScaling::Native,
            },
        }
    }
}

impl SoundLayout {
    fn output_layout(self) -> SoundOutputLayout {
        match self {
            SoundLayout::Stereo => SoundOutputLayout::Stereo,
            SoundLayout::FivePointOne => SoundOutputLayout::FivePointOne,
            SoundLayout::SevenPointOne => SoundOutputLayout::SevenPointOne,
            SoundLayout::Automatic => SoundOutputLayout::Automatic,
        }
    }
}

impl SubtitleControls {
    fn presentation(&self) -> Result<SubtitlePresentation, String> {
        Ok(SubtitlePresentation {
            vertical_offset_percent: self.offset_percent,
            colour: self.colour.as_deref().map(Rgba::parse_hex).transpose()?,
        })
    }
}

fn outcome(player: &Player) -> PlayerControlOutcome {
    PlayerControlOutcome {
        mpv_file_unchanged: player.active() == Backend::Mpv,
    }
}

// the grok player keeps every control through loads and stops
fn apply_to_grok(
    state: &PreviewPlayer,
    apply: impl FnOnce(&GrokPlayer),
) -> Result<PlayerControlOutcome, String> {
    let player = state.player()?;
    apply(player.grok());
    Ok(outcome(player))
}

// a refused profile leaves the one in use, and the error names the file and why
pub(super) fn set_display_profile(
    player: &Player,
    profile: Option<&Path>,
) -> Result<PlayerControlOutcome, String> {
    player.grok().set_display_profile(profile)?;
    Ok(outcome(player))
}

#[tauri::command(async)]
pub fn preview_set_picture(
    picture: PictureControls,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<PlayerControlOutcome, String> {
    apply_to_grok(&state, |grok| grok.set_presentation(picture.presentation()))
}

#[tauri::command(async)]
pub fn preview_set_sound_device(
    device: Option<String>,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<PlayerControlOutcome, String> {
    apply_to_grok(&state, |grok| grok.set_sound_output_device(device))
}

#[tauri::command(async)]
pub fn preview_set_sound_layout(
    layout: SoundLayout,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<PlayerControlOutcome, String> {
    apply_to_grok(&state, |grok| {
        grok.set_sound_output_layout(layout.output_layout())
    })
}

#[tauri::command(async)]
pub fn preview_set_sound_delay(
    milliseconds: i64,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<PlayerControlOutcome, String> {
    apply_to_grok(&state, |grok| {
        grok.set_sound_delay_milliseconds(milliseconds)
    })
}

#[tauri::command(async)]
pub fn preview_set_subtitle_presentation(
    subtitles: SubtitleControls,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<PlayerControlOutcome, String> {
    let presentation = subtitles.presentation()?;
    apply_to_grok(&state, |grok| grok.set_subtitle_presentation(presentation))
}

pub(super) fn set_stereo_output(player: &Player, mode: StereoMode) -> PlayerControlOutcome {
    player.grok().set_stereo_output(mode.output());
    outcome(player)
}

// side by side and top and bottom decode both eyes, twice the work of one
#[tauri::command(async)]
pub fn preview_set_stereo_output(
    output: StereoMode,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<PlayerControlOutcome, String> {
    let outcome = set_stereo_output(state.player()?, output);
    *state.stereo_output.lock().unwrap() = output;
    Ok(outcome)
}

#[tauri::command]
pub fn preview_stereo_output(state: tauri::State<'_, PreviewPlayer>) -> StereoMode {
    *state.stereo_output.lock().unwrap()
}

// None goes back to the built-in sRGB
#[tauri::command(async)]
pub fn preview_set_display_profile(
    profile: Option<PathBuf>,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<PlayerControlOutcome, String> {
    set_display_profile(state.player()?, profile.as_deref())
}

pub(super) fn loaded_picture(player: &Player) -> Result<LoadedPicture, String> {
    if player.active() == Backend::Mpv {
        let loaded = player.mpv().get_property_string(MPV_PATH_PROPERTY).is_ok();
        return Ok(if loaded {
            LoadedPicture::MpvFile
        } else {
            LoadedPicture::Nothing
        });
    }
    let metadata: serde_json::Value =
        serde_json::from_str(&player.grok().metadata_json()).map_err(|error| error.to_string())?;
    let Some(source) = metadata[SOURCE_FIELD].as_str() else {
        return Ok(LoadedPicture::Nothing);
    };
    if resolve_picture(Path::new(source)).is_ok_and(|picture| picture.as02) {
        return Ok(LoadedPicture::Imf);
    }
    Ok(LoadedPicture::Dcp {
        stereoscopic: metadata[STEREOSCOPIC_FIELD] == true,
    })
}

#[tauri::command(async)]
pub fn preview_loaded_picture(
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<LoadedPicture, String> {
    loaded_picture(state.player()?)
}

#[tauri::command(async)]
pub fn preview_sound_devices() -> Result<Vec<String>, String> {
    sound_output_device_names()
}

// set on both players, so whichever plays the next load is metered
pub(super) fn set_level_meter(player: &Player, enabled: bool) -> Result<(), String> {
    player.grok().set_level_meter(enabled);
    player.mpv().set_level_meter(enabled)
}

#[tauri::command(async)]
pub fn preview_set_level_meter(
    enabled: bool,
    state: tauri::State<'_, PreviewPlayer>,
) -> Result<(), String> {
    set_level_meter(state.player()?, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_percents_become_fractions_of_the_picture() {
        let picture = PictureControls {
            brightness: 1.5,
            masks_percent: MaskPercents {
                top: 10.0,
                bottom: 25.0,
                left: 0.0,
                right: 100.0,
            },
            scaling: Scaling::Fill,
        };

        assert_eq!(
            picture.presentation(),
            PresentationSettings {
                brightness: 1.5,
                masks: PictureMasks {
                    top: 0.1,
                    bottom: 0.25,
                    left: 0.0,
                    right: 1.0,
                },
                scaling: PictureScaling::Fill,
            }
        );
    }

    #[test]
    fn the_default_picture_is_the_players_default_presentation() {
        assert_eq!(
            PictureControls::default().presentation(),
            PresentationSettings::default()
        );
    }

    #[test]
    fn the_page_names_read_as_the_controls() {
        let picture: PictureControls = serde_json::from_str(
            r#"{"brightness": 2, "masksPercent": {"top": 5}, "scaling": "native"}"#,
        )
        .unwrap();
        let sound: SoundControls = serde_json::from_str(
            r#"{"device": "HDMI", "layout": "sevenPointOne", "delayMilliseconds": -40}"#,
        )
        .unwrap();

        assert_eq!(picture.brightness, 2.0);
        assert_eq!(picture.masks_percent.top, 5.0);
        assert_eq!(picture.masks_percent.bottom, 0.0);
        assert_eq!(picture.scaling, Scaling::Native);
        assert_eq!(sound.device.as_deref(), Some("HDMI"));
        assert_eq!(
            sound.layout.output_layout(),
            SoundOutputLayout::SevenPointOne
        );
        assert_eq!(sound.delay_milliseconds, -40);
    }

    #[test]
    fn a_subtitle_colour_reads_as_hex_and_none_keeps_the_cue_colours() {
        let coloured = SubtitleControls {
            offset_percent: 8.0,
            colour: Some("#ffcc00".to_string()),
        };

        assert_eq!(
            coloured.presentation().unwrap(),
            SubtitlePresentation {
                vertical_offset_percent: 8.0,
                colour: Some(Rgba {
                    r: 0xff,
                    g: 0xcc,
                    b: 0x00,
                    a: 0xff,
                }),
            }
        );
        assert_eq!(
            SubtitleControls::default().presentation().unwrap(),
            SubtitlePresentation::default()
        );
    }

    #[test]
    fn a_subtitle_colour_that_is_not_hex_is_refused() {
        let error = SubtitleControls {
            offset_percent: 0.0,
            colour: Some("yellow".to_string()),
        }
        .presentation()
        .unwrap_err();

        assert!(error.contains("yellow is not a colour"), "{error}");
    }
}
