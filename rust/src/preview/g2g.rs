use std::ffi::{c_void, CString};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use g2g_player::{
    EndFile, Event, GlRenderContext, GlTarget, LoadOptions, Lowres, Media,
    OverlayRectangle as G2gOverlayRectangle, Player, Property, PropertyChange, VideoSize,
};
use postkit::audio_levels::{
    at_least_the_floor, numbered_channel_label, ChannelLevel, RMS_WINDOW_SECONDS,
};
use postkit::grok_player::GetProcAddressFn;

use super::{
    audio_levels_json, json_bool, json_number, overlay_rectangles, with_extra_fields,
    OverlayRectangle, PreviewPlayer, SourceSize, SubtitleTrackSlot, SubtitleTracks,
    AUDIO_LEVELS_FIELD, CACHE_SECONDS_FIELD, CONTAINER_FPS_FIELD, DECODER_FPS_FIELD,
    DELAYED_FRAMES_FIELD, DROPPED_FRAMES_FIELD, EOF_FIELD, SHOWN_FRAME_SIZE_PROPERTIES,
    SUBTITLE_TRACK_SLOTS,
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

struct LoadedSource {
    // a path or an edl:// uri
    source: String,
    // mpv shows a forced composition title in place of the filename
    title: Option<String>,
}

pub struct G2gPlayer {
    player: Player,
    render: Mutex<Option<RenderOnGlThread>>,
    loaded: Mutex<Option<LoadedSource>>,
    sent_overlay: Mutex<Vec<OverlayRectangle>>,
    picture_filters_set: Mutex<bool>,
    // g2g-player drops its subtitle files on every load, as mpv does
    subtitle_files: Mutex<SubtitleTracks>,
}

fn source_uri(source: &str) -> Result<String, String> {
    if source.starts_with(EDL_SCHEME) {
        return Ok(source.to_string());
    }
    let path = std::path::absolute(source).map_err(|error| error.to_string())?;
    Ok(format!("file://{}", path.display()))
}

// a picture measured as `measured` and shown at `shown` pixels
fn shown_rectangle(
    rectangle: &OverlayRectangle,
    measured: VideoSize,
    shown: VideoSize,
) -> OverlayRectangle {
    let horizontal = f64::from(shown.width) / f64::from(measured.width);
    let vertical = f64::from(shown.height) / f64::from(measured.height);
    let scaled = |value: i64, scale: f64| (value as f64 * scale).round() as i64;
    let x = scaled(rectangle.x, horizontal);
    let y = scaled(rectangle.y, vertical);
    OverlayRectangle {
        x,
        y,
        width: scaled(rectangle.x + rectangle.width, horizontal) - x,
        height: scaled(rectangle.y + rectangle.height, vertical) - y,
        ..*rectangle
    }
}

impl G2gPlayer {
    pub fn new() -> Self {
        G2gPlayer {
            player: Player::new(),
            render: Mutex::new(None),
            loaded: Mutex::new(None),
            sent_overlay: Mutex::new(Vec::new()),
            picture_filters_set: Mutex::new(false),
            subtitle_files: Mutex::new(SubtitleTracks::default()),
        }
    }

    #[cfg(test)]
    pub(super) fn discard_sound(&self) {
        self.player
            .set_sound_output(g2g_player::SoundOutput::Discard);
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

    pub fn load(&self, source: &str, title: Option<&str>, paused: bool) -> Result<(), String> {
        let uri = source_uri(source)?;
        // guikit takes mpv's filters off before every load
        self.set_picture_filters(None)?;
        let options = LoadOptions {
            start: None,
            pause: Some(paused),
        };
        self.player
            .load(Media::Uri(&uri), options)
            .map_err(|error| error.to_string())?;
        *self.subtitle_files.lock().unwrap() = SubtitleTracks::default();
        *self.loaded.lock().unwrap() = Some(LoadedSource {
            source: source.to_string(),
            title: title.map(str::to_string),
        });
        Ok(())
    }

    // the file loads again at its position, as mpv's lowres is read when the decoder opens
    pub fn set_decode_scale(&self, lowres: Lowres) -> Result<(), String> {
        self.player.set_lowres(lowres);
        let loaded = self.loaded.lock().unwrap();
        let Some(loaded) = loaded.as_ref() else {
            return Ok(());
        };
        // g2g-player decodes a timeline's clips inside edlsrc, which lowres does not reach
        if loaded.source.starts_with(EDL_SCHEME) {
            return Ok(());
        }
        let options = LoadOptions {
            start: self.position().map(Duration::from_secs_f64),
            pause: Some(self.paused()),
        };
        self.player
            .load(Media::Uri(&source_uri(&loaded.source)?), options)
            .map_err(|error| error.to_string())?;
        let subtitle_files = std::mem::take(&mut *self.subtitle_files.lock().unwrap());
        for track in SUBTITLE_TRACK_SLOTS {
            if let Some(path) = &track.state_in(&subtitle_files).file_path {
                self.set_subtitle_file(track, Some(Path::new(path)))?;
            }
        }
        Ok(())
    }

    pub fn stop(&self) {
        self.player.stop();
        *self.subtitle_files.lock().unwrap() = SubtitleTracks::default();
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

    // the HUD fields of the metadata poll, from the g2g-player properties named as mpv's
    fn hud_counters(&self) -> [(&'static str, Option<f64>); 5] {
        let counter = |property| match self.player.property(property) {
            PropertyChange::FrameDropCount(count) | PropertyChange::DelayedFrameCount(count) => {
                Some(count as f64)
            }
            PropertyChange::DemuxerCacheDuration(cached) => {
                cached.map(|cached| cached.as_secs_f64())
            }
            PropertyChange::EstimatedVfFps(fps) | PropertyChange::ContainerFps(fps) => fps,
            other => unreachable!("g2g-player answers {property:?} with {other:?}"),
        };
        [
            (DROPPED_FRAMES_FIELD, counter(Property::FrameDropCount)),
            (DELAYED_FRAMES_FIELD, counter(Property::DelayedFrameCount)),
            (CACHE_SECONDS_FIELD, counter(Property::DemuxerCacheDuration)),
            (DECODER_FPS_FIELD, counter(Property::EstimatedVfFps)),
            (CONTAINER_FPS_FIELD, counter(Property::ContainerFps)),
        ]
    }

    fn demux_size(&self) -> Option<VideoSize> {
        let PropertyChange::DemuxSize(size) = self.player.property(Property::DemuxSize) else {
            unreachable!("g2g-player answers DemuxSize with DemuxSize");
        };
        size
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
            })?;
        track
            .state_in_mut(&mut self.subtitle_files.lock().unwrap())
            .file_path = path.map(|path| path.display().to_string());
        Ok(())
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
        let Some(shown) = self.shown_picture_size() else {
            return;
        };
        // overlays name source pixels, which a decode scale shrinks only in the unfiltered frame
        let measured = match *self.picture_filters_set.lock().unwrap() {
            true => shown,
            false => self.demux_size().unwrap_or(shown),
        };
        let Some(source) = SourceSize::new(f64::from(measured.width), f64::from(measured.height))
        else {
            return;
        };
        let wanted: Vec<OverlayRectangle> = overlay_rectangles(&overlays, source)
            .iter()
            .map(|rectangle| shown_rectangle(rectangle, measured, shown))
            .collect();
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
        let filename = self.loaded.lock().unwrap().as_ref().and_then(|loaded| {
            let file_name = || {
                Some(&loaded.source)
                    .filter(|source| !source.starts_with(EDL_SCHEME))
                    .and_then(|path| Path::new(path).file_name())
                    .map(|name| name.to_string_lossy().into_owned())
            };
            loaded.title.clone().or_else(file_name)
        });
        let base = serde_json::json!({
            "position": self.position(),
            "duration": self.duration(),
            "paused": self.paused(),
            "filename": filename,
        })
        .to_string();
        let mut fields: Vec<(&str, String)> = self
            .hud_counters()
            .into_iter()
            .map(|(field, value)| (field, json_number(value)))
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

#[cfg(test)]
mod tests {
    use super::super::end_of_file_tests::overlay_state;
    use super::super::grok_fixture::wait_until;
    use super::super::{ContentRectangle, DecodeScale, PreviewOverlays};
    use super::*;

    const CLIP_WIDTH: u32 = 320;
    const CLIP_HEIGHT: u32 = 180;
    const CLIP_FRAMES_PER_SECOND: u32 = 24;

    // libavcodec's MPEG-2 decoder implements lowres
    fn write_mpeg2_clip(path: &Path) {
        let out = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!(
                "color=c=black:size={CLIP_WIDTH}x{CLIP_HEIGHT}:rate={CLIP_FRAMES_PER_SECOND}:duration=2"
            ))
            .args(["-c:v", "mpeg2video"])
            .arg(path)
            .output()
            .expect("ffmpeg");
        assert!(
            out.status.success(),
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn a_half_decode_scale_keeps_a_source_pixel_outline_on_the_shown_picture() {
        let directory = tempfile::tempdir().unwrap();
        let clip = directory.path().join("black.mpg");
        write_mpeg2_clip(&clip);
        let player = G2gPlayer::new();
        player.discard_sound();
        player.load(&clip.to_string_lossy(), None, true).unwrap();
        player
            .set_decode_scale(DecodeScale::Half.g2g_lowres())
            .unwrap();
        let half = VideoSize {
            width: CLIP_WIDTH / 2,
            height: CLIP_HEIGHT / 2,
        };
        wait_until("the picture shown at half size", || {
            player.shown_picture_size() == Some(half)
        });

        // the whole picture, named in source pixels
        let state = overlay_state(PreviewOverlays {
            content_outline: Some(ContentRectangle {
                x: 0,
                y: 0,
                width: CLIP_WIDTH,
                height: CLIP_HEIGHT,
            }),
            ..Default::default()
        });
        player.apply_overlays(&state);
        let sent = player.sent_overlay.lock().unwrap();
        let right = sent.iter().map(|r| r.x + r.width).max().unwrap();
        let bottom = sent.iter().map(|r| r.y + r.height).max().unwrap();
        assert_eq!(
            (right, bottom),
            (i64::from(half.width), i64::from(half.height))
        );
    }

    #[test]
    fn the_hud_reports_the_frame_rate_the_filters_put_out() {
        let directory = tempfile::tempdir().unwrap();
        let clip = directory.path().join("black.mpg");
        write_mpeg2_clip(&clip);
        let player = G2gPlayer::new();
        player.discard_sound();
        player.load(&clip.to_string_lossy(), None, false).unwrap();
        let decoder_fps = || {
            let metadata: serde_json::Value =
                serde_json::from_str(&player.metadata_json().unwrap()).unwrap();
            metadata[DECODER_FPS_FIELD].as_f64()
        };
        wait_until("the HUD reporting a frame rate", || decoder_fps().is_some());
        // whole-nanosecond timestamps put 1/24 s a fraction of a nanosecond off
        let fps = decoder_fps().unwrap();
        assert!(
            (fps - f64::from(CLIP_FRAMES_PER_SECOND)).abs() < 1e-6,
            "the HUD reports {fps} fps"
        );
    }
}
