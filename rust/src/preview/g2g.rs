use std::ffi::{c_void, CString};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use g2g_player::{
    EndFile, Event, GlRenderContext, GlTarget, LoadOptions, Media,
    OverlayRectangle as G2gOverlayRectangle, Player, Property, PropertyChange, VideoSize,
};
use postkit::audio_levels::{
    at_least_the_floor, numbered_channel_label, ChannelLevel, RMS_WINDOW_SECONDS,
};
use postkit::grok_player::GetProcAddressFn;

use super::{
    audio_levels_json, json_bool, json_number, overlay_rectangles, with_extra_fields,
    OverlayRectangle, PreviewPlayer, SourceSize, SubtitleTrackSlot, AUDIO_LEVELS_FIELD,
    CONTAINER_FPS_FIELD, EOF_FIELD, HUD_COUNTER_PROPERTIES, SHOWN_FRAME_SIZE_PROPERTIES,
};

const PLAYER_CHOICE_VARIABLE: &str = "GUIKIT_PLAYER";
const G2G_CHOICE: &str = "g2g";
const EDL_SCHEME: &str = "edl://";

pub(super) fn g2g_chosen() -> bool {
    std::env::var(PLAYER_CHOICE_VARIABLE).is_ok_and(|choice| choice == G2G_CHOICE)
}

struct RenderOnGlThread(GlRenderContext);

// SAFETY: the hosts create the render context and render it on their GL thread, and no other thread touches it
unsafe impl Send for RenderOnGlThread {}

pub struct G2gPlayer {
    player: Player,
    render: Mutex<Option<RenderOnGlThread>>,
    // the path or edl:// uri playing, None after a stop
    loaded: Mutex<Option<String>>,
    sent_overlay: Mutex<Vec<OverlayRectangle>>,
    picture_filters_set: Mutex<bool>,
}

impl G2gPlayer {
    pub fn new() -> Self {
        G2gPlayer {
            player: Player::new(),
            render: Mutex::new(None),
            loaded: Mutex::new(None),
            sent_overlay: Mutex::new(Vec::new()),
            picture_filters_set: Mutex::new(false),
        }
    }

    pub fn init_opengl(&self, get_proc_address: GetProcAddressFn, context: *mut c_void) {
        let resolve = move |name: &str| {
            let name = CString::new(name).expect("GL entry point names hold no NUL");
            // SAFETY: the host's resolver takes its own context pointer and a NUL-terminated name
            unsafe { get_proc_address(context, name.as_ptr()) as *const c_void }
        };
        *self.render.lock().unwrap() = Some(RenderOnGlThread(GlRenderContext::new(
            &self.player,
            resolve,
        )));
    }

    pub fn set_update_callback(&self, callback: impl Fn() + Send + Sync + 'static) {
        self.player.set_update_callback(callback);
    }

    pub fn wants_redraw(&self) -> bool {
        self.player.wants_redraw()
    }

    pub fn render_opengl(
        &self,
        framebuffer: i32,
        width: i32,
        height: i32,
        flip_y: bool,
    ) -> Result<(), String> {
        let mut render = self.render.lock().unwrap();
        let render = render
            .as_mut()
            .ok_or("the g2g player has no GL context yet")?;
        let unsigned = |value: i32| u32::try_from(value).map_err(|error| error.to_string());
        let target = GlTarget {
            framebuffer: unsigned(framebuffer)?,
            width: unsigned(width)?,
            height: unsigned(height)?,
            flip_y,
        };
        // SAFETY: the hosts render with their GL context current and the framebuffer belonging to it
        unsafe { render.0.render(target) }.map_err(|error| format!("{error:?}"))
    }

    pub fn load(&self, source: &str, paused: bool) -> Result<(), String> {
        let uri = match source.starts_with(EDL_SCHEME) {
            true => source.to_string(),
            false => {
                let path = std::path::absolute(source).map_err(|error| error.to_string())?;
                format!("file://{}", path.display())
            }
        };
        // guikit takes mpv's filters off before every load
        self.set_picture_filters(None)?;
        let options = LoadOptions {
            start: None,
            pause: Some(paused),
        };
        self.player
            .load(Media::Uri(&uri), options)
            .map_err(|error| error.to_string())?;
        *self.loaded.lock().unwrap() = Some(source.to_string());
        Ok(())
    }

    pub fn stop(&self) {
        self.player.stop();
        *self.loaded.lock().unwrap() = None;
        self.set_picture_filters(None)
            .expect("an empty filter chain is always accepted");
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded.lock().unwrap().is_some()
    }

    fn paused(&self) -> bool {
        let PropertyChange::Pause(paused) = self.player.property(Property::Pause) else {
            unreachable!("g2g-player answers Pause with Pause");
        };
        paused
    }

    pub fn play_pause(&self) {
        match self.paused() {
            true => self.player.play(),
            false => self.player.pause(),
        }
    }

    pub fn seek(&self, seconds: f64) -> Result<(), String> {
        let offset =
            Duration::try_from_secs_f64(seconds.abs()).map_err(|error| error.to_string())?;
        let sought = match seconds < 0.0 {
            true => self.player.seek_backward(offset),
            false => self.player.seek_forward(offset),
        };
        sought.map_err(|error| error.to_string())
    }

    pub fn seek_absolute(&self, seconds: f64) -> Result<(), String> {
        let position = Duration::try_from_secs_f64(seconds).map_err(|error| error.to_string())?;
        self.player
            .seek(position)
            .map_err(|error| error.to_string())
    }

    pub fn frame_step(&self) -> Result<(), String> {
        self.player.frame_step().map_err(|error| error.to_string())
    }

    pub fn frame_back_step(&self) -> Result<(), String> {
        self.player
            .frame_back_step()
            .map_err(|error| error.to_string())
    }

    pub fn position(&self) -> Option<f64> {
        let PropertyChange::TimePos(position) = self.player.property(Property::TimePos) else {
            unreachable!("g2g-player answers TimePos with TimePos");
        };
        position.map(|position| position.as_secs_f64())
    }

    pub fn duration(&self) -> Option<f64> {
        let PropertyChange::Duration(duration) = self.player.property(Property::Duration) else {
            unreachable!("g2g-player answers Duration with Duration");
        };
        duration.map(|duration| duration.as_secs_f64())
    }

    fn end_of_file(&self) -> bool {
        let PropertyChange::EofReached(reached) = self.player.property(Property::EofReached) else {
            unreachable!("g2g-player answers EofReached with EofReached");
        };
        reached
    }

    fn container_fps(&self) -> Option<f64> {
        let PropertyChange::ContainerFps(fps) = self.player.property(Property::ContainerFps) else {
            unreachable!("g2g-player answers ContainerFps with ContainerFps");
        };
        fps
    }

    fn shown_picture_size(&self) -> Option<VideoSize> {
        let PropertyChange::VideoOutParams(size) = self.player.property(Property::VideoOutParams)
        else {
            unreachable!("g2g-player answers VideoOutParams with VideoOutParams");
        };
        size
    }

    pub fn set_picture_filters(&self, filters: Option<&[String]>) -> Result<(), String> {
        let chain: Vec<&str> = filters
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .collect();
        self.player
            .set_video_filters(&chain)
            .map_err(|error| error.to_string())?;
        *self.picture_filters_set.lock().unwrap() = !chain.is_empty();
        Ok(())
    }

    pub fn set_subtitle_file(
        &self,
        track: SubtitleTrackSlot,
        path: Option<&Path>,
    ) -> Result<(), String> {
        self.player
            .set_subtitle_file(track.g2g_slot(), path)
            .map_err(|error| {
                let path = path.expect("only reading a subtitle file fails");
                format!("cannot read {}: {error}", path.display())
            })
    }

    pub fn set_subtitle_visibility(&self, track: SubtitleTrackSlot, visible: bool) {
        self.player
            .set_subtitle_visibility(track.g2g_slot(), visible);
    }

    pub fn set_level_meter(&self, enabled: bool) {
        self.player
            .set_level_meter(enabled.then(|| Duration::from_secs_f64(RMS_WINDOW_SECONDS)));
    }

    fn audio_levels(&self) -> Option<Vec<ChannelLevel>> {
        let levels = self.player.audio_levels()?;
        let labels = self.channel_labels(levels.peak_db.len());
        let channels = levels.peak_db.iter().zip(&levels.rms_db).zip(labels);
        Some(
            channels
                .map(|((&peak, &rms), label)| ChannelLevel {
                    label,
                    peak_dbfs: at_least_the_floor(peak),
                    rms_dbfs: at_least_the_floor(rms),
                })
                .collect(),
        )
    }

    // the speaker names mpv's audio-params/channels gives, numbered when the stream names none
    fn channel_labels(&self, channel_count: usize) -> Vec<String> {
        let PropertyChange::AudioParams(params) = self.player.property(Property::AudioParams)
        else {
            unreachable!("the player answers a property with its own variant");
        };
        let speakers: Vec<_> = params
            .map(|params| params.channel_layout.positions().collect())
            .unwrap_or_default();
        if speakers.len() != channel_count {
            return (0..channel_count).map(numbered_channel_label).collect();
        }
        speakers
            .into_iter()
            .map(|speaker| speaker.ffmpeg_name().to_string())
            .collect()
    }

    pub(super) fn apply_overlays(&self, state: &PreviewPlayer) {
        let mut overlays = *state.overlays.lock().unwrap();
        // the filtered frame has had the crop taken off already
        if *self.picture_filters_set.lock().unwrap() {
            overlays.crop_visible = false;
        }
        let Some(size) = self.shown_picture_size() else {
            return;
        };
        let Some(source) = SourceSize::new(f64::from(size.width), f64::from(size.height)) else {
            return;
        };
        let wanted = overlay_rectangles(&overlays, source);
        let mut sent = self.sent_overlay.lock().unwrap();
        if *sent == wanted {
            return;
        }
        self.player.set_overlay(
            wanted
                .iter()
                .map(|rectangle| G2gOverlayRectangle {
                    x: rectangle.x,
                    y: rectangle.y,
                    width: rectangle.width,
                    height: rectangle.height,
                    colour: rectangle.colour,
                    alpha: rectangle.alpha,
                })
                .collect(),
        );
        *sent = wanted;
    }

    fn print_failures(&self) {
        while let Some(event) = self.player.wait_event(Duration::ZERO) {
            if let Event::EndFile(EndFile::Error(message)) = event {
                eprintln!("[preview] g2g failed: {message}");
            }
        }
    }

    pub(super) fn metadata_json(&self) -> Result<String, String> {
        self.print_failures();
        let filename = self
            .loaded
            .lock()
            .unwrap()
            .as_deref()
            .filter(|source| !source.starts_with(EDL_SCHEME))
            .and_then(|path| Path::new(path).file_name())
            .map(|name| name.to_string_lossy().into_owned());
        let base = serde_json::json!({
            "position": self.position(),
            "duration": self.duration(),
            "paused": self.paused(),
            "filename": filename,
        })
        .to_string();
        // g2g-player reports the frame rate and none of mpv's other HUD counters
        let container_fps = self.container_fps();
        let mut fields: Vec<(&str, String)> = HUD_COUNTER_PROPERTIES
            .iter()
            .map(|(field, _)| {
                let value = container_fps.filter(|_| *field == CONTAINER_FPS_FIELD);
                (*field, json_number(value))
            })
            .collect();
        let shown = self.shown_picture_size();
        let [(width_field, _), (height_field, _)] = SHOWN_FRAME_SIZE_PROPERTIES;
        fields.push((
            width_field,
            json_number(shown.map(|size| f64::from(size.width))),
        ));
        fields.push((
            height_field,
            json_number(shown.map(|size| f64::from(size.height))),
        ));
        fields.push((EOF_FIELD, json_bool(Some(self.end_of_file()))));
        fields.push((AUDIO_LEVELS_FIELD, audio_levels_json(self.audio_levels())?));
        with_extra_fields(&base, &fields)
    }
}
