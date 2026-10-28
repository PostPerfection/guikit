use std::io::ErrorKind;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use postkit::composited_preview::PreviewRenderProgress;
use postkit::encode::FrameRate;
use postkit::grok_player::{
    FrameRange, GrokPlayer, SoundFile, SourceOptions, SourceSubtitles, SubtitleFile,
};
use postkit::pipeline::{run_encode_with_options, EncodeRunOptions};
use postkit::timeline::{
    clip_name, played_stretches, preview_renders_any_stretch, Clip, PlayedPicture, RenderedStretch,
    Timeline, BLACK_GAP_SIZE,
};
use serde::Serialize;
use tauri::Manager;

use super::screening_runner::{handed_off, PlayerStatus, PreviewRunnerPlayer, RunnerPlayer};
use super::{json_number, Backend, PlayState, PreviewPlayer};

const TIMELINE_TICK_INTERVAL: Duration = Duration::from_millis(100);
const TIMELINE_POSITION_FIELD: &str = "timeline_position";
const TIMELINE_CLIP_INDEX_FIELD: &str = "timeline_clip_index";

// a grok timeline seeks within a clip
pub trait TimelinePlayer: RunnerPlayer {
    fn seek_absolute(&self, seconds: f64) -> Result<(), String>;
}

impl TimelinePlayer for PreviewRunnerPlayer<'_> {
    fn seek_absolute(&self, seconds: f64) -> Result<(), String> {
        self.0.player()?.grok().seek_absolute(seconds);
        Ok(())
    }
}

pub(super) struct TimelineFrames {
    fps: FrameRate,
    // each layer 0 clip's index on the timeline and the frames it covers
    base_clips: Vec<(usize, Range<u64>)>,
    total_frames: u64,
}

impl TimelineFrames {
    pub(super) fn of(timeline: &Timeline) -> TimelineFrames {
        TimelineFrames {
            fps: timeline.fps,
            base_clips: timeline
                .base_clips()
                .into_iter()
                .map(|(index, clip)| (index, clip.start..clip.end()))
                .collect(),
            total_frames: timeline.total_frames(),
        }
    }

    pub(super) fn duration_seconds(&self) -> f64 {
        self.seconds_at(self.total_frames)
    }

    fn seconds_at(&self, frame: u64) -> f64 {
        frame as f64 * f64::from(self.fps.denominator) / f64::from(self.fps.numerator)
    }

    // rounded, so a position the player computed as frame / fps comes back as that frame
    fn frame_at(&self, seconds: f64) -> u64 {
        let frame = (seconds.max(0.0) * self.fps.as_f64()).round() as u64;
        frame.min(self.total_frames.saturating_sub(1))
    }

    // None in a gap of layer 0
    fn clip_at(&self, frame: u64) -> Option<usize> {
        self.base_clips
            .iter()
            .find(|(_, frames)| frames.contains(&frame))
            .map(|(index, _)| *index)
    }
}

// a piece of a layer 0 clip, black over a gap in layer 0, or a rendered stretch
struct PlayedSegment {
    picture: PathBuf,
    start: u64,
    in_frame: u64,
    out_frame: u64,
    // None for a gap
    clip_index: Option<usize>,
    subtitle: Option<PathBuf>,
    // the frame of `picture` that the clip subtitle's time zero falls on
    subtitle_time_zero_frame: i64,
}

// in start order, cut where mpv cuts the timeline, each gap playing the first frames of black
fn played_segments(
    timeline: &Timeline,
    black: Option<&Path>,
    rendered: &[RenderedStretch],
) -> Result<Vec<PlayedSegment>, String> {
    let frames = TimelineFrames::of(timeline);
    played_stretches(timeline, rendered)
        .into_iter()
        .map(|played| {
            let start = played.frames.start;
            let length = played.frames.end - start;
            Ok(match played.picture {
                PlayedPicture::Clip { index, first_frame } => PlayedSegment {
                    picture: timeline.clips[index].picture.clone(),
                    start,
                    in_frame: first_frame,
                    out_frame: first_frame + length,
                    clip_index: Some(index),
                    subtitle: timeline.clips[index].subtitle.clone(),
                    subtitle_time_zero_frame: 0,
                },
                PlayedPicture::Rendered(path) => {
                    let clip_index = frames.clip_at(start);
                    let base = clip_index.map(|index| &timeline.clips[index]);
                    // the render's frame 0 is the base clip's source frame at the stretch start
                    let base_source_frame =
                        base.map_or(0, |clip| clip.in_frame + start - clip.start);
                    PlayedSegment {
                        picture: path,
                        start,
                        in_frame: 0,
                        out_frame: length,
                        clip_index,
                        subtitle: base.and_then(|clip| clip.subtitle.clone()),
                        subtitle_time_zero_frame: -(base_source_frame as i64),
                    }
                }
                PlayedPicture::Black => {
                    let Some(black) = black else {
                        return Err(format!(
                            "layer 0 has a gap of {length} frames at frame {start}, and no black \
                             codestreams were made for it"
                        ));
                    };
                    PlayedSegment {
                        picture: black.to_path_buf(),
                        start,
                        in_frame: 0,
                        out_frame: length,
                        clip_index: None,
                        subtitle: None,
                        subtitle_time_zero_frame: 0,
                    }
                }
            })
        })
        .collect()
}

// one black still encoded once and hard linked once per frame of the longest gap in layer 0, None without gaps
pub(super) fn render_black_codestreams(
    timeline: &Timeline,
    black_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(longest_gap) = timeline
        .base_gaps()
        .iter()
        .map(|gap| gap.end - gap.start)
        .max()
    else {
        return Ok(None);
    };
    // a directory an earlier run left behind refuses the links
    remove_work_path(black_dir)?;
    let still_dir = black_dir.join("still");
    create_directory(&still_dir)?;
    // postkit reads a tiff itself, so the encode loads no ffmpeg library into the app
    let still = still_dir.join("black.tif");
    let finished = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"])
        .arg(format!("color=c=black:s={BLACK_GAP_SIZE}"))
        .args(["-frames:v", "1", "-pix_fmt", "rgb48le"])
        .arg(&still)
        .output()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    if !finished.status.success() {
        return Err(format!(
            "ffmpeg cannot write the black still {}: {}",
            still.display(),
            String::from_utf8_lossy(&finished.stderr).trim()
        ));
    }
    let options = EncodeRunOptions {
        fps: timeline.fps,
        ..EncodeRunOptions::default()
    };
    let never = Arc::new(AtomicBool::new(false));
    let encoded = run_encode_with_options(
        &still,
        &black_dir.join("encode"),
        &options,
        &never,
        &never,
        |_| {},
        |_| {},
    )?;
    let codestream = only_file_in(&encoded.j2k_dir)?;
    let frames_dir = black_dir.join("frames");
    create_directory(&frames_dir)?;
    for frame in 0..longest_gap {
        let link = frames_dir.join(format!("frame_{frame:08}.j2c"));
        std::fs::hard_link(&codestream, &link)
            .map_err(|e| format!("cannot link {}: {e}", link.display()))?;
    }
    Ok(Some(frames_dir))
}

fn only_file_in(directory: &Path) -> Result<PathBuf, String> {
    let entries: Vec<PathBuf> = std::fs::read_dir(directory)
        .map_err(|e| format!("cannot list {}: {e}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("cannot list {}: {e}", directory.display()))?;
    match entries.as_slice() {
        [only] => Ok(only.clone()),
        _ => Err(format!(
            "{} holds {} files where the encode writes one",
            directory.display(),
            entries.len()
        )),
    }
}

fn create_directory(directory: &Path) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|e| format!("cannot create {}: {e}", directory.display()))
}

// a timeline's rendered sound and subtitles are files, its black codestreams a directory
pub(super) fn remove_work_path(path: &Path) -> Result<(), String> {
    let removed = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match removed {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            Err(format!("cannot remove {}: {error}", path.display()))
        }
        _ => Ok(()),
    }
}

// a segment the grok player plays loads with its range, and the segment after it is queued behind it
pub struct GrokTimelineRun {
    frames: TimelineFrames,
    segments: Vec<PlayedSegment>,
    // the timeline's rendered sound, which every segment plays from its place on the timeline
    sound: Option<PathBuf>,
    timeline_subtitle: Option<PathBuf>,
    current_segment: usize,
    queued_segment: Option<usize>,
}

impl GrokTimelineRun {
    pub fn start(
        timeline: Timeline,
        sound: Option<PathBuf>,
        black: Option<&Path>,
        rendered: &[RenderedStretch],
        play_state: PlayState,
        player: &impl TimelinePlayer,
    ) -> Result<Self, String> {
        let mut run = GrokTimelineRun {
            frames: TimelineFrames::of(&timeline),
            segments: played_segments(&timeline, black, rendered)?,
            sound,
            timeline_subtitle: timeline.subtitle,
            current_segment: 0,
            queued_segment: None,
        };
        run.load_segment(0, play_state, player)?;
        Ok(run)
    }

    fn segment_path(&self, index: usize) -> String {
        self.segments[index].picture.display().to_string()
    }

    fn segment_options(&self, index: usize) -> SourceOptions {
        let segment = &self.segments[index];
        SourceOptions {
            range: Some(FrameRange {
                in_frame: segment.in_frame,
                out_frame: Some(segment.out_frame),
            }),
            sound: self.sound.clone().map(|path| SoundFile {
                path,
                fps: self.frames.fps,
                first_frame: segment.start,
            }),
            subtitles: self.segment_subtitles(index),
            codestream_fps: Some(self.frames.fps),
            ..SourceOptions::default()
        }
    }

    // a clip's own file is timed against its source, the timeline's file against the timeline
    fn segment_subtitles(&self, index: usize) -> SourceSubtitles {
        let segment = &self.segments[index];
        let (path, source_frame_at_time_zero) = match (&segment.subtitle, &self.timeline_subtitle) {
            (Some(own), _) => (own, segment.subtitle_time_zero_frame),
            (None, Some(timeline)) => (timeline, segment.in_frame as i64 - segment.start as i64),
            (None, None) => return SourceSubtitles::Empty,
        };
        SourceSubtitles::File(SubtitleFile {
            path: path.clone(),
            source_frame_at_time_zero,
        })
    }

    fn load_segment(
        &mut self,
        index: usize,
        play_state: PlayState,
        player: &impl TimelinePlayer,
    ) -> Result<(), String> {
        player.load(
            &self.segments[index].picture,
            self.segment_options(index),
            play_state,
        )?;
        self.current_segment = index;
        self.queued_segment = None;
        self.queue_segment(index + 1, player)
    }

    fn queue_segment(&mut self, index: usize, player: &impl TimelinePlayer) -> Result<(), String> {
        if index >= self.segments.len() {
            return Ok(());
        }
        player.queue_next(&self.segments[index].picture, self.segment_options(index))?;
        self.queued_segment = Some(index);
        Ok(())
    }

    // the segment on screen, the queued one already once the player has handed off to it
    fn playing_segment(&self, status: &PlayerStatus) -> usize {
        match self.queued_segment {
            Some(queued) if handed_off(status, &self.segment_path(queued)) => queued,
            _ => self.current_segment,
        }
    }

    // the timeline index of the clip on screen, None while a gap plays
    pub fn playing_clip(&self, status: &PlayerStatus) -> Option<usize> {
        self.segments[self.playing_segment(status)].clip_index
    }

    fn follow_hand_off(&mut self, status: &PlayerStatus) {
        let playing = self.playing_segment(status);
        if playing != self.current_segment {
            self.current_segment = playing;
            self.queued_segment = None;
        }
    }

    // false once the player shows a source this timeline did not load
    pub fn tick(&mut self, player: &impl TimelinePlayer) -> Result<bool, String> {
        let status = player.status()?;
        self.follow_hand_off(&status);
        if status.source.as_deref() != Some(self.segment_path(self.current_segment).as_str()) {
            return Ok(false);
        }
        let next = self.current_segment + 1;
        if self.queued_segment.is_some() || next == self.segments.len() {
            return Ok(true);
        }
        // a segment shorter than a tick ends before its follower could be queued
        if status.eof {
            self.load_segment(next, PlayState::Playing, player)?;
        } else {
            self.queue_segment(next, player)?;
        }
        Ok(true)
    }

    pub fn position(&self, status: &PlayerStatus) -> Option<f64> {
        let frames_into_segment = (status.position? * status.container_fps?).round() as u64;
        let start = self.segments[self.playing_segment(status)].start;
        Some(self.frames.seconds_at(start + frames_into_segment))
    }

    pub fn seek(&mut self, seconds: f64, player: &impl TimelinePlayer) -> Result<(), String> {
        let status = player.status()?;
        self.follow_hand_off(&status);
        let target = self.frames.frame_at(seconds);
        let segment = self
            .segments
            .partition_point(|segment| segment.start <= target)
            .saturating_sub(1);
        if segment != self.current_segment {
            let play_state = if status.paused {
                PlayState::Paused
            } else {
                PlayState::Playing
            };
            self.load_segment(segment, play_state, player)?;
        }
        let segment_fps = player.status()?.container_fps.ok_or_else(|| {
            format!(
                "the player reports no frame rate for {}",
                self.segment_path(segment)
            )
        })?;
        let frames_into_segment = target - self.segments[segment].start;
        player.seek_absolute(frames_into_segment as f64 / segment_fps)
    }

    pub fn seek_by(&mut self, seconds: f64, player: &impl TimelinePlayer) -> Result<(), String> {
        let status = player.status()?;
        let position = self
            .position(&status)
            .ok_or_else(|| "nothing is loaded".to_string())?;
        self.seek(position + seconds, player)
    }
}

pub(super) enum TimelinePlayback {
    Mpv(TimelineFrames),
    Grok(GrokTimelineRun),
}

pub(super) struct TimelinePosition {
    pub(super) seconds: Option<f64>,
    pub(super) clip_index: Option<usize>,
}

pub(super) struct LoadedTimeline {
    // tells the ticker of an earlier grok timeline that it was replaced
    generation: u64,
    playback: TimelinePlayback,
    // the sound, subtitles and black rendered for the timeline, removed with it
    work_paths: Vec<PathBuf>,
}

// every layer 0 clip on one player: grok takes JPEG 2000, mpv the rest
pub(super) fn timeline_backend(timeline: &Timeline) -> Result<Backend, String> {
    let base = timeline.base_clips();
    let on_grok = |(_, clip): &&(usize, &Clip)| GrokPlayer::accepts(&clip.picture);
    let first_grok = base.iter().find(on_grok);
    let first_mpv = base.iter().find(|placed| !on_grok(placed));
    match (first_grok, first_mpv) {
        (Some((grok, grok_clip)), Some((mpv, mpv_clip))) => Err(format!(
            "{} plays on the JPEG 2000 player and {} on mpv: a timeline preview plays every clip \
             on one of them",
            clip_name(*grok, grok_clip),
            clip_name(*mpv, mpv_clip)
        )),
        (Some(_), None) => Ok(Backend::Grok),
        _ => Ok(Backend::Mpv),
    }
}

pub(super) const TIMELINE_LOAD_CANCELLED: &str = "the timeline preview load was cancelled";

// a render the flag stopped can fail any way, and the flag says why
fn unless_cancelled<T>(cancel: &AtomicBool, result: Result<T, String>) -> Result<T, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err(TIMELINE_LOAD_CANCELLED.to_string());
    }
    result
}

pub(super) fn refuse_misplaced_options(
    timeline: &Timeline,
    options: Option<&EncodeRunOptions>,
) -> Result<(), String> {
    match (preview_renders_any_stretch(timeline), options) {
        (true, None) => Err(
            "the timeline has a clip above layer 0 or a still or image sequence on layer 0, and \
             load_timeline_preview was given no build options to render it with"
                .to_string(),
        ),
        (false, Some(_)) => Err(
            "the timeline has no clip above layer 0 and no still or image sequence on layer 0, \
             and load_timeline_preview takes build options only to render one"
                .to_string(),
        ),
        _ => Ok(()),
    }
}

pub(super) const TIMELINE_RENDER_PROGRESS_EVENT: &str = "timeline-render-progress";

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(super) struct TimelineRenderProgress {
    pub(super) frames_rendered: u64,
    pub(super) total_frames: u64,
    // None until a frame is rendered
    pub(super) seconds_left: Option<f64>,
}

impl TimelineRenderProgress {
    fn after(progress: PreviewRenderProgress, elapsed: Duration) -> TimelineRenderProgress {
        let frames_left = progress.total_frames - progress.frames_rendered;
        TimelineRenderProgress {
            frames_rendered: progress.frames_rendered,
            total_frames: progress.total_frames,
            seconds_left: (progress.frames_rendered > 0).then(|| {
                elapsed.as_secs_f64() / progress.frames_rendered as f64 * frames_left as f64
            }),
        }
    }
}

pub(super) struct StretchRender<'a> {
    pub(super) options: &'a EncodeRunOptions,
    pub(super) on_progress: &'a mut dyn FnMut(TimelineRenderProgress),
}

pub(super) fn render_stretches(
    render: Option<StretchRender>,
    cancel: &AtomicBool,
    render_with: impl FnOnce(
        &EncodeRunOptions,
        &mut dyn FnMut(PreviewRenderProgress),
    ) -> Result<Vec<RenderedStretch>, String>,
) -> Result<Vec<RenderedStretch>, String> {
    let Some(render) = render else {
        return Ok(Vec::new());
    };
    let started = Instant::now();
    let on_progress = render.on_progress;
    let rendered = render_with(render.options, &mut |progress| {
        on_progress(TimelineRenderProgress::after(progress, started.elapsed()))
    });
    unless_cancelled(cancel, rendered)
}

impl PreviewPlayer {
    pub(super) fn cancel_timeline_render(&self) {
        self.timeline_render_cancel
            .lock()
            .unwrap()
            .store(true, Ordering::Relaxed);
    }

    // cancels the render in flight and hands back the flag of the load starting
    pub(super) fn start_timeline_render(&self) -> Arc<AtomicBool> {
        let mut in_flight = self.timeline_render_cancel.lock().unwrap();
        in_flight.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        *in_flight = cancel.clone();
        cancel
    }

    // a stop or a newer load sets the flag before this or waits until it returns
    pub(super) fn load_unless_cancelled<R>(
        &self,
        cancel: &AtomicBool,
        load: impl FnOnce() -> Result<R, String>,
    ) -> Result<R, String> {
        let _in_flight = self.timeline_render_cancel.lock().unwrap();
        unless_cancelled(cancel, Ok(()))?;
        load()
    }

    pub(super) fn clear_timeline(&self) {
        let Some(cleared) = self.timeline.lock().unwrap().take() else {
            return;
        };
        for path in cleared.work_paths {
            if let Err(error) = remove_work_path(&path) {
                eprintln!("[timeline] {error}");
            }
        }
    }

    pub(super) fn install_timeline(
        &self,
        playback: TimelinePlayback,
        work_paths: Vec<PathBuf>,
    ) -> u64 {
        let generation = self.timeline_generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self.timeline.lock().unwrap() = Some(LoadedTimeline {
            generation,
            playback,
            work_paths,
        });
        generation
    }

    // false once the run is gone, replaced, or has stopped following the player
    fn tick_timeline(&self, generation: u64) -> bool {
        let mut loaded = self.timeline.lock().unwrap();
        let Some(LoadedTimeline {
            generation: current,
            playback: TimelinePlayback::Grok(run),
            ..
        }) = loaded.as_mut()
        else {
            return false;
        };
        if *current != generation {
            return false;
        }
        match run.tick(&PreviewRunnerPlayer(self)) {
            Ok(true) => return true,
            Ok(false) => eprintln!("[timeline] the player left the timeline"),
            Err(error) => eprintln!("[timeline] {error}"),
        }
        *loaded = None;
        false
    }

    // None while no grok timeline is loaded
    pub(super) fn with_grok_timeline<R>(
        &self,
        action: impl FnOnce(&mut GrokTimelineRun, &PreviewRunnerPlayer) -> Result<R, String>,
    ) -> Option<Result<R, String>> {
        let mut loaded = self.timeline.lock().unwrap();
        let Some(LoadedTimeline {
            playback: TimelinePlayback::Grok(run),
            ..
        }) = loaded.as_mut()
        else {
            return None;
        };
        Some(action(run, &PreviewRunnerPlayer(self)))
    }

    // None while no timeline is loaded
    pub(super) fn timeline_position(
        &self,
        player: &super::Player,
    ) -> Result<Option<TimelinePosition>, String> {
        let loaded = self.timeline.lock().unwrap();
        let Some(loaded) = loaded.as_ref() else {
            return Ok(None);
        };
        Ok(Some(match &loaded.playback {
            TimelinePlayback::Mpv(frames) => {
                let seconds = player.mpv().get_position().ok();
                TimelinePosition {
                    seconds,
                    clip_index: seconds
                        .and_then(|seconds| frames.clip_at(frames.frame_at(seconds))),
                }
            }
            TimelinePlayback::Grok(run) => {
                let status = PreviewRunnerPlayer(self).status()?;
                TimelinePosition {
                    seconds: run.position(&status),
                    clip_index: run.playing_clip(&status),
                }
            }
        }))
    }

    pub(super) fn timeline_fields(
        &self,
        player: &super::Player,
    ) -> Result<Vec<(&'static str, String)>, String> {
        let Some(position) = self.timeline_position(player)? else {
            return Ok(Vec::new());
        };
        Ok(vec![
            (TIMELINE_POSITION_FIELD, json_number(position.seconds)),
            (
                TIMELINE_CLIP_INDEX_FIELD,
                json_number(position.clip_index.map(|index| index as f64)),
            ),
        ])
    }

    pub(super) fn timeline_duration(&self) -> Option<f64> {
        let loaded = self.timeline.lock().unwrap();
        Some(match &loaded.as_ref()?.playback {
            TimelinePlayback::Mpv(frames) => frames.duration_seconds(),
            TimelinePlayback::Grok(run) => run.frames.duration_seconds(),
        })
    }
}

// the hand-off happens on the player's own thread, this only queues the clip after it
pub(super) fn follow_grok_timeline(app: tauri::AppHandle, generation: u64) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TIMELINE_TICK_INTERVAL);
        if !app.state::<PreviewPlayer>().tick_timeline(generation) {
            return;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::end_of_file_tests::overlay_state;
    use super::super::grok_fixture;
    use super::super::{
        load_grok_timeline, PreviewOverlays, TIMELINE_BLACK_DIRECTORY,
        TIMELINE_GROK_STRETCHES_DIRECTORY, TIMELINE_SOUND_FILE,
    };
    use super::*;
    use postkit::picture_processing::{Fit, PictureProcessing, Placement};
    use postkit::timeline::{ClipSound, SoundTrack, TIMELINE_FORMAT_VERSION};
    use postkit::wav_io::{SampleFormat, WavSpec, WavWriter};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    const TIMELINE_FPS: u32 = 24;
    // what the grok player gives a directory of bare codestreams
    const CODESTREAM_DIRECTORY_FPS: f64 = 24.0;
    const FIRST: &str = "/media/first.mxf";
    const SECOND: &str = "/media/second";
    const THIRD: &str = "/media/third.mxf";

    #[derive(Default)]
    struct FakePlayer {
        status: RefCell<PlayerStatus>,
        calls: RefCell<Vec<String>>,
        // the sound and subtitles of each load and queue
        clip_media: RefCell<Vec<String>>,
    }

    impl FakePlayer {
        fn calls(&self) -> Vec<String> {
            self.calls.take()
        }

        fn clip_media(&self) -> Vec<String> {
            self.clip_media.take()
        }

        fn record_media(&self, source: &Path, options: &SourceOptions) {
            let sound = match &options.sound {
                Some(sound) => format!(
                    "{} from {} at {}/{}",
                    sound.path.display(),
                    sound.first_frame,
                    sound.fps.numerator,
                    sound.fps.denominator
                ),
                None => "no sound".to_string(),
            };
            let subtitles = match &options.subtitles {
                SourceSubtitles::Composition => "its own subtitles".to_string(),
                SourceSubtitles::Empty => "no subtitles".to_string(),
                SourceSubtitles::File(file) => format!(
                    "{} from source frame {}",
                    file.path.display(),
                    file.source_frame_at_time_zero
                ),
            };
            self.clip_media
                .borrow_mut()
                .push(format!("{}: {sound}, {subtitles}", source.display()));
        }

        fn play_to(&self, seconds: f64) {
            self.status.borrow_mut().position = Some(seconds);
        }

        fn reach_the_end(&self) {
            let mut status = self.status.borrow_mut();
            match status.queued_source.take() {
                Some(queued) => {
                    status.source = Some(queued);
                    status.position = Some(0.0);
                }
                None => status.eof = true,
            }
        }
    }

    fn described(source: &Path, options: &SourceOptions) -> String {
        let range = options.range.expect("every timeline clip has a range");
        format!(
            "{} {}..{}",
            source.display(),
            range.in_frame,
            range
                .out_frame
                .expect("every timeline clip has an out point")
        )
    }

    impl RunnerPlayer for FakePlayer {
        fn load(
            &self,
            source: &Path,
            options: SourceOptions,
            play_state: PlayState,
        ) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("load {}", described(source, &options)));
            self.record_media(source, &options);
            *self.status.borrow_mut() = PlayerStatus {
                source: Some(source.display().to_string()),
                position: Some(0.0),
                container_fps: Some(CODESTREAM_DIRECTORY_FPS),
                paused: play_state.is_paused(),
                ..PlayerStatus::default()
            };
            Ok(())
        }

        fn queue_next(&self, source: &Path, options: SourceOptions) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("queue {}", described(source, &options)));
            self.record_media(source, &options);
            self.status.borrow_mut().queued_source = Some(source.display().to_string());
            Ok(())
        }

        fn stop(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("stop".to_string());
            *self.status.borrow_mut() = PlayerStatus::default();
            Ok(())
        }

        fn status(&self) -> Result<PlayerStatus, String> {
            Ok(self.status.borrow().clone())
        }
    }

    impl TimelinePlayer for FakePlayer {
        fn seek_absolute(&self, seconds: f64) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("seek {seconds}"));
            self.status.borrow_mut().position = Some(seconds);
            Ok(())
        }
    }

    fn clip(picture: &str, start: u64, in_frame: u64, out_frame: u64) -> Clip {
        Clip {
            picture: PathBuf::from(picture),
            start,
            layer: 0,
            opacity: 1.0,
            in_frame,
            out_frame,
            sound: ClipSound::None,
            subtitle: None,
            source_colour: Default::default(),
            picture_processing: Default::default(),
        }
    }

    // 48, 24 and 12 frames, starting at timeline frames 0, 48 and 72
    fn three_clips() -> Timeline {
        Timeline {
            version: TIMELINE_FORMAT_VERSION,
            fps: FrameRate::whole(TIMELINE_FPS),
            clips: vec![
                clip(FIRST, 0, 24, 72),
                clip(SECOND, 48, 0, 24),
                clip(THIRD, 72, 100, 112),
            ],
            sound: SoundTrack::PerClip,
            subtitle: None,
            reel_starts: Vec::new(),
        }
    }

    #[test]
    fn the_first_clip_loads_with_its_range_and_the_second_queues_behind_it() {
        let player = FakePlayer::default();
        GrokTimelineRun::start(three_clips(), None, None, &[], PlayState::Playing, &player)
            .unwrap();
        assert_eq!(
            player.calls(),
            vec![
                format!("load {FIRST} 24..72"),
                format!("queue {SECOND} 0..24")
            ]
        );
    }

    #[test]
    fn the_position_adds_the_clip_start_and_follows_the_hand_off() {
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(three_clips(), None, None, &[], PlayState::Playing, &player)
                .unwrap();
        player.play_to(1.0);
        let status = player.status().unwrap();
        assert_eq!(run.position(&status), Some(1.0));
        assert_eq!(run.playing_clip(&status), Some(0));

        player.reach_the_end();
        player.play_to(0.5);
        // read before the tick catches up with the hand-off
        let status = player.status().unwrap();
        assert_eq!(run.position(&status), Some(2.5));
        assert_eq!(run.playing_clip(&status), Some(1));

        player.calls();
        assert!(run.tick(&player).unwrap());
        assert_eq!(player.calls(), vec![format!("queue {THIRD} 100..112")]);
        assert_eq!(run.playing_clip(&player.status().unwrap()), Some(1));
    }

    const TIMELINE_SOUND: &str = "/cache/timeline-sound.wav";

    #[test]
    fn every_clip_plays_the_timeline_sound_from_its_place_on_the_timeline() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(
            three_clips(),
            Some(PathBuf::from(TIMELINE_SOUND)),
            None,
            &[],
            PlayState::Playing,
            &player,
        )
        .unwrap();
        player.reach_the_end();
        run.tick(&player).unwrap();
        let sound_from = |start: u64| format!("{TIMELINE_SOUND} from {start} at 24/1");
        assert_eq!(
            player.clip_media(),
            vec![
                format!("{FIRST}: {}, no subtitles", sound_from(0)),
                format!("{SECOND}: {}, no subtitles", sound_from(48)),
                format!("{THIRD}: {}, no subtitles", sound_from(72)),
            ]
        );
    }

    #[test]
    fn a_clip_subtitle_is_timed_against_its_source_and_the_timeline_subtitle_against_the_timeline()
    {
        let mut timeline = three_clips();
        timeline.clips[1].subtitle = Some(PathBuf::from("/media/second.srt"));
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(timeline, None, None, &[], PlayState::Playing, &player).unwrap();
        player.reach_the_end();
        run.tick(&player).unwrap();
        assert_eq!(
            player.clip_media(),
            vec![
                format!("{FIRST}: no sound, no subtitles"),
                format!("{SECOND}: no sound, /media/second.srt from source frame 0"),
                format!("{THIRD}: no sound, no subtitles"),
            ]
        );

        let mut timeline = three_clips();
        timeline.subtitle = Some(PathBuf::from("/media/timeline.srt"));
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(timeline, None, None, &[], PlayState::Playing, &player).unwrap();
        // timeline frame 54 is frame 6 of the second clip
        run.seek(2.25, &player).unwrap();
        // the third clip starts at timeline frame 72 on its source frame 100
        assert_eq!(
            player.clip_media()[2..],
            [
                format!("{SECOND}: no sound, /media/timeline.srt from source frame -48"),
                format!("{THIRD}: no sound, /media/timeline.srt from source frame 28"),
            ]
        );
    }

    const BLACK: &str = "/cache/timeline-black/frames";

    // FIRST covers frames 0 to 48 and THIRD 60 to 72, with SECOND above the gap between them
    fn gapped_clips() -> Timeline {
        let mut timeline = three_clips();
        timeline.clips[1].layer = 1;
        timeline.clips[1].start = 40;
        timeline.clips[2].start = 60;
        timeline
    }

    #[test]
    fn a_gap_in_layer_0_plays_black_with_the_sound_from_its_place_and_no_clip_index() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(
            gapped_clips(),
            Some(PathBuf::from(TIMELINE_SOUND)),
            Some(Path::new(BLACK)),
            &[],
            PlayState::Playing,
            &player,
        )
        .unwrap();
        assert_eq!(
            player.calls(),
            vec![
                format!("load {FIRST} 24..72"),
                format!("queue {BLACK} 0..12")
            ]
        );

        player.reach_the_end();
        player.play_to(0.25);
        let status = player.status().unwrap();
        assert_eq!(run.playing_clip(&status), None);
        assert_eq!(run.position(&status), Some(2.25));

        run.tick(&player).unwrap();
        player.reach_the_end();
        run.tick(&player).unwrap();
        assert_eq!(player.calls(), vec![format!("queue {THIRD} 100..112")]);
        assert_eq!(run.playing_clip(&player.status().unwrap()), Some(2));
        let sound_from = |start: u64| format!("{TIMELINE_SOUND} from {start} at 24/1");
        assert_eq!(
            player.clip_media(),
            vec![
                format!("{FIRST}: {}, no subtitles", sound_from(0)),
                format!("{BLACK}: {}, no subtitles", sound_from(48)),
                format!("{THIRD}: {}, no subtitles", sound_from(60)),
            ]
        );
    }

    #[test]
    fn a_seek_into_a_gap_loads_the_black_and_seeks_within_it() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(
            gapped_clips(),
            None,
            Some(Path::new(BLACK)),
            &[],
            PlayState::Playing,
            &player,
        )
        .unwrap();
        player.calls();
        // timeline frame 54 is frame 6 of the gap
        run.seek(2.25, &player).unwrap();
        assert_eq!(
            player.calls(),
            vec![
                format!("load {BLACK} 0..12"),
                format!("queue {THIRD} 100..112"),
                "seek 0.25".to_string()
            ]
        );
    }

    #[test]
    fn a_gap_without_black_is_refused() {
        let player = FakePlayer::default();
        let refused =
            GrokTimelineRun::start(gapped_clips(), None, None, &[], PlayState::Playing, &player);
        let error = refused.err().expect("a gap with no black plays");
        assert!(error.contains("gap of 12 frames at frame 48"), "{error}");
    }

    #[test]
    fn the_black_for_the_gaps_is_one_codestream_linked_once_per_frame_of_the_longest_gap() {
        let directory = tempfile::tempdir().unwrap();
        let black_dir = directory.path().join("black");
        let mut timeline = gapped_clips();
        // a trailing gap from 72 to 75 under a clip on layer 1
        timeline.clips[1].start = 51;

        assert_eq!(
            render_black_codestreams(&three_clips(), &black_dir).unwrap(),
            None
        );
        let frames_dir = render_black_codestreams(&timeline, &black_dir)
            .unwrap()
            .expect("the timeline has gaps");

        let mut frames: Vec<String> = std::fs::read_dir(&frames_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        frames.sort();
        let expected: Vec<String> = (0..12)
            .map(|frame| format!("frame_{frame:08}.j2c"))
            .collect();
        assert_eq!(frames, expected);
        let first = frames_dir.join(&frames[0]);
        let codestream = std::fs::read(&first).unwrap();
        assert_eq!(codestream[..2], [0xFF, 0x4F], "no SOC marker");
        assert_eq!(
            std::fs::read(frames_dir.join(&frames[11])).unwrap(),
            codestream
        );
        assert_eq!(decoded_luma_peak(&first), 0, "the gap frame is not black");
    }

    // ffmpeg's own JPEG 2000 decoder, so the check does not lean on the encoder that wrote it
    fn decoded_luma_peak(codestream: &Path) -> u8 {
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(codestream)
            .args(["-f", "rawvideo", "-pix_fmt", "gray", "-"])
            .output()
            .expect("ffmpeg");
        assert!(
            decoded.status.success() && !decoded.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        decoded.stdout.into_iter().max().unwrap()
    }

    fn write_silence(path: &Path, seconds: u32) {
        const SAMPLE_RATE: u32 = 48_000;
        const CHANNELS: u16 = 2;
        const BYTES_PER_SAMPLE: u16 = 3;
        const BITS_PER_BYTE: u16 = 8;
        let spec = WavSpec {
            channels: CHANNELS,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: BYTES_PER_SAMPLE * BITS_PER_BYTE,
            sample_format: SampleFormat::Int,
        };
        let samples = u64::from(SAMPLE_RATE * seconds);
        let mut writer = WavWriter::create(path, spec, samples).unwrap();
        let bytes = samples as usize * usize::from(CHANNELS) * usize::from(BYTES_PER_SAMPLE);
        writer.write_bytes(&vec![0u8; bytes]).unwrap();
        writer.finalize().unwrap();
    }

    fn not_cancelled() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn clearing_a_grok_timeline_removes_the_sound_and_black_rendered_for_it() {
        let directory = tempfile::tempdir().unwrap();
        let clip_sound = directory.path().join("clip.wav");
        write_silence(&clip_sound, 1);
        let mut timeline = gapped_clips();
        timeline.clips[0].sound = ClipSound::File(clip_sound.clone());
        let work_dir = directory.path().join("work");
        let sound_file = work_dir.join(TIMELINE_SOUND_FILE);
        let black_dir = work_dir.join(TIMELINE_BLACK_DIRECTORY);
        let state = overlay_state(PreviewOverlays::default());
        let player = FakePlayer::default();

        load_grok_timeline(
            &state,
            timeline,
            &work_dir,
            None,
            &not_cancelled(),
            PlayState::Playing,
            &player,
        )
        .unwrap();
        assert!(sound_file.exists() && black_dir.exists());
        assert!(
            player.clip_media()[0].contains(&format!("{} from 0", sound_file.display())),
            "the first clip does not play the rendered sound"
        );

        state.clear_timeline();
        for rendered in [&sound_file, &black_dir] {
            assert!(!rendered.exists(), "{} is left behind", rendered.display());
        }
        assert!(clip_sound.exists());
        assert_eq!(state.timeline_duration(), None);
    }

    #[test]
    fn the_last_clip_queues_nothing_after_it() {
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(three_clips(), None, None, &[], PlayState::Playing, &player)
                .unwrap();
        player.reach_the_end();
        run.tick(&player).unwrap();
        player.reach_the_end();
        player.calls();
        assert!(run.tick(&player).unwrap());
        assert_eq!(run.playing_clip(&player.status().unwrap()), Some(2));
        assert!(player.calls().is_empty());
    }

    #[test]
    fn a_clip_that_ends_between_ticks_loads_the_clip_after_it() {
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(three_clips(), None, None, &[], PlayState::Playing, &player)
                .unwrap();
        player.reach_the_end();
        player.reach_the_end();
        player.calls();
        assert!(run.tick(&player).unwrap());
        assert_eq!(player.calls(), vec![format!("load {THIRD} 100..112")]);
        assert_eq!(run.playing_clip(&player.status().unwrap()), Some(2));
    }

    #[test]
    fn a_seek_inside_the_playing_clip_seeks_within_its_range() {
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(three_clips(), None, None, &[], PlayState::Playing, &player)
                .unwrap();
        player.calls();
        run.seek(1.5, &player).unwrap();
        assert_eq!(player.calls(), vec!["seek 1.5"]);
    }

    #[test]
    fn a_seek_into_another_clip_loads_it_and_queues_the_one_after() {
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(three_clips(), None, None, &[], PlayState::Playing, &player)
                .unwrap();
        player.calls();
        // timeline frame 54 is frame 6 of the second clip
        run.seek(2.25, &player).unwrap();
        assert_eq!(
            player.calls(),
            vec![
                format!("load {SECOND} 0..24"),
                format!("queue {THIRD} 100..112"),
                "seek 0.25".to_string()
            ]
        );
        assert_eq!(run.position(&player.status().unwrap()), Some(2.25));
    }

    #[test]
    fn a_seek_into_another_clip_while_paused_stays_paused() {
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(three_clips(), None, None, &[], PlayState::Playing, &player)
                .unwrap();
        player.status.borrow_mut().paused = true;
        player.calls();
        run.seek(3.25, &player).unwrap();
        assert_eq!(
            player.calls(),
            vec![format!("load {THIRD} 100..112"), "seek 0.25".to_string()]
        );
        assert!(player.status().unwrap().paused);
    }

    #[test]
    fn a_timeline_started_paused_loads_its_first_clip_paused() {
        let player = FakePlayer::default();
        GrokTimelineRun::start(three_clips(), None, None, &[], PlayState::Paused, &player).unwrap();
        assert!(player.status().unwrap().paused);
    }

    #[test]
    fn a_relative_seek_moves_from_the_timeline_position() {
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(three_clips(), None, None, &[], PlayState::Playing, &player)
                .unwrap();
        player.play_to(1.5);
        player.calls();
        run.seek_by(1.0, &player).unwrap();
        assert_eq!(
            player.calls(),
            vec![
                format!("load {SECOND} 0..24"),
                format!("queue {THIRD} 100..112"),
                "seek 0.5".to_string()
            ]
        );
    }

    #[test]
    fn another_source_on_the_player_ends_the_run() {
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(three_clips(), None, None, &[], PlayState::Playing, &player)
                .unwrap();
        player
            .load(
                Path::new("/elsewhere.mxf"),
                SourceOptions {
                    range: Some(FrameRange {
                        in_frame: 0,
                        out_frame: Some(1),
                    }),
                    ..SourceOptions::default()
                },
                PlayState::Playing,
            )
            .unwrap();
        assert!(!run.tick(&player).unwrap());
    }

    fn codestream_directory(directory: &Path) -> String {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join("frame_0000.j2c"), [0u8]).unwrap();
        directory.display().to_string()
    }

    #[test]
    fn a_timeline_mixing_jpeg_2000_and_video_names_a_clip_of_each() {
        let directory = tempfile::tempdir().unwrap();
        let codestreams = codestream_directory(&directory.path().join("codestreams"));
        let mut timeline = three_clips();
        timeline.clips = vec![clip("/media/a.mov", 0, 0, 1), clip(&codestreams, 1, 0, 1)];
        let error = timeline_backend(&timeline).unwrap_err();
        assert!(
            error.contains(&format!(
                "clip 2 ({codestreams}) plays on the JPEG 2000 player"
            )) && error.contains("clip 1 (/media/a.mov) on mpv"),
            "{error}"
        );

        // a clip above layer 0 is not played
        timeline.clips[0].layer = 1;
        assert_eq!(timeline_backend(&timeline), Ok(Backend::Grok));
    }

    #[test]
    fn a_timeline_of_stills_plays_on_mpv() {
        let directory = tempfile::tempdir().unwrap();
        let stills = directory.path().join("stills");
        std::fs::create_dir_all(&stills).unwrap();
        std::fs::write(stills.join("shot_0000.png"), b"png").unwrap();
        let mut timeline = three_clips();
        timeline.clips = vec![
            clip("/media/a.mov", 0, 0, 1),
            clip(&stills.display().to_string(), 1, 0, 1),
        ];
        assert_eq!(timeline_backend(&timeline), Ok(Backend::Mpv));
    }

    fn on_layer(picture: &str, start: u64, in_frame: u64, frames: u64, layer: u32) -> Clip {
        Clip {
            layer,
            ..clip(picture, start, in_frame, in_frame + frames)
        }
    }

    fn timeline_of(clips: Vec<Clip>) -> Timeline {
        Timeline {
            clips,
            ..three_clips()
        }
    }

    const STRETCHES: &str = "/cache/stretches";

    fn rendered(frames: Range<u64>) -> RenderedStretch {
        RenderedStretch {
            path: PathBuf::from(format!("{STRETCHES}/{}-{}", frames.start, frames.end)),
            frames,
        }
    }

    // each segment as what it plays, its range, where it starts and its clip index
    fn played(clips: Vec<Clip>, rendered: &[RenderedStretch]) -> Vec<String> {
        let timeline = timeline_of(clips);
        let player = FakePlayer::default();
        let run = GrokTimelineRun::start(
            timeline.clone(),
            Some(PathBuf::from(TIMELINE_SOUND)),
            Some(Path::new(BLACK)),
            rendered,
            PlayState::Playing,
            &player,
        )
        .unwrap();
        let mut next_start = 0;
        for (index, segment) in run.segments.iter().enumerate() {
            assert_eq!(segment.start, next_start, "segment {index} leaves a hole");
            next_start += segment.out_frame - segment.in_frame;
            let sound = run.segment_options(index).sound.unwrap();
            assert_eq!(sound.first_frame, segment.start, "segment {index}'s sound");
        }
        assert_eq!(next_start, timeline.total_frames());
        run.segments
            .iter()
            .map(|segment| {
                format!(
                    "{} {}..{} at {} clip {:?}",
                    segment.picture.display(),
                    segment.in_frame,
                    segment.out_frame,
                    segment.start,
                    segment.clip_index
                )
            })
            .collect()
    }

    struct Shape {
        name: &'static str,
        clips: Vec<Clip>,
        rendered: Vec<RenderedStretch>,
        played: Vec<&'static str>,
    }

    const OVERLAY: &str = "/media/overlay";
    const CARD: &str = "/media/card.png";

    #[test]
    fn the_segments_follow_every_cut_the_rendered_stretches_make() {
        let shapes = vec![
            Shape {
                name: "an overlay inside one base clip",
                clips: vec![on_layer(FIRST, 0, 5, 48, 0), on_layer(OVERLAY, 12, 0, 8, 1)],
                rendered: vec![rendered(12..20)],
                played: vec![
                    "/media/first.mxf 5..17 at 0 clip Some(0)",
                    "/cache/stretches/12-20 0..8 at 12 clip Some(0)",
                    "/media/first.mxf 25..53 at 20 clip Some(0)",
                ],
            },
            Shape {
                name: "an overlay across two base clips",
                clips: vec![
                    on_layer(FIRST, 0, 0, 24, 0),
                    on_layer(THIRD, 24, 10, 24, 0),
                    on_layer(OVERLAY, 16, 0, 16, 1),
                ],
                rendered: vec![rendered(16..24), rendered(24..32)],
                played: vec![
                    "/media/first.mxf 0..16 at 0 clip Some(0)",
                    "/cache/stretches/16-24 0..8 at 16 clip Some(0)",
                    "/cache/stretches/24-32 0..8 at 24 clip Some(1)",
                    "/media/third.mxf 18..34 at 32 clip Some(1)",
                ],
            },
            Shape {
                name: "an overlay over a gap",
                clips: vec![
                    on_layer(FIRST, 0, 0, 12, 0),
                    on_layer(THIRD, 24, 30, 12, 0),
                    on_layer(OVERLAY, 14, 0, 6, 1),
                ],
                rendered: vec![rendered(14..20)],
                played: vec![
                    "/media/first.mxf 0..12 at 0 clip Some(0)",
                    "/cache/timeline-black/frames 0..2 at 12 clip None",
                    "/cache/stretches/14-20 0..6 at 14 clip None",
                    "/cache/timeline-black/frames 0..4 at 20 clip None",
                    "/media/third.mxf 30..42 at 24 clip Some(1)",
                ],
            },
            Shape {
                name: "an overlay at frame 0",
                clips: vec![on_layer(FIRST, 0, 3, 24, 0), on_layer(OVERLAY, 0, 0, 8, 1)],
                rendered: vec![rendered(0..8)],
                played: vec![
                    "/cache/stretches/0-8 0..8 at 0 clip Some(0)",
                    "/media/first.mxf 11..27 at 8 clip Some(0)",
                ],
            },
            Shape {
                name: "an overlay running past the end of layer 0",
                clips: vec![
                    on_layer(FIRST, 0, 0, 24, 0),
                    on_layer(OVERLAY, 16, 0, 14, 1),
                ],
                rendered: vec![rendered(16..24), rendered(24..30)],
                played: vec![
                    "/media/first.mxf 0..16 at 0 clip Some(0)",
                    "/cache/stretches/16-24 0..8 at 16 clip Some(0)",
                    "/cache/stretches/24-30 0..6 at 24 clip None",
                ],
            },
            Shape {
                name: "staggered overlays on layers 1 and 2",
                clips: vec![
                    on_layer(FIRST, 0, 0, 32, 0),
                    on_layer(OVERLAY, 8, 0, 12, 1),
                    on_layer(OVERLAY, 12, 0, 12, 2),
                ],
                rendered: vec![rendered(8..12), rendered(12..20), rendered(20..24)],
                played: vec![
                    "/media/first.mxf 0..8 at 0 clip Some(0)",
                    "/cache/stretches/8-12 0..4 at 8 clip Some(0)",
                    "/cache/stretches/12-20 0..8 at 12 clip Some(0)",
                    "/cache/stretches/20-24 0..4 at 20 clip Some(0)",
                    "/media/first.mxf 24..32 at 24 clip Some(0)",
                ],
            },
            Shape {
                name: "two adjacent overlays on one layer",
                clips: vec![
                    on_layer(FIRST, 0, 0, 24, 0),
                    on_layer(OVERLAY, 8, 0, 4, 1),
                    on_layer(CARD, 12, 0, 4, 1),
                ],
                rendered: vec![rendered(8..12), rendered(12..16)],
                played: vec![
                    "/media/first.mxf 0..8 at 0 clip Some(0)",
                    "/cache/stretches/8-12 0..4 at 8 clip Some(0)",
                    "/cache/stretches/12-16 0..4 at 12 clip Some(0)",
                    "/media/first.mxf 16..24 at 16 clip Some(0)",
                ],
            },
            Shape {
                name: "a held still overlay",
                clips: vec![on_layer(FIRST, 0, 0, 24, 0), on_layer(CARD, 4, 0, 8, 1)],
                rendered: vec![rendered(4..12)],
                played: vec![
                    "/media/first.mxf 0..4 at 0 clip Some(0)",
                    "/cache/stretches/4-12 0..8 at 4 clip Some(0)",
                    "/media/first.mxf 12..24 at 12 clip Some(0)",
                ],
            },
            Shape {
                name: "a held still on layer 0",
                clips: vec![on_layer(CARD, 0, 0, 10, 0), on_layer(THIRD, 10, 0, 10, 0)],
                rendered: vec![rendered(0..10)],
                played: vec![
                    "/cache/stretches/0-10 0..10 at 0 clip Some(0)",
                    "/media/third.mxf 0..10 at 10 clip Some(1)",
                ],
            },
            Shape {
                name: "an image sequence on layer 0",
                clips: vec![
                    on_layer(FIRST, 0, 0, 6, 0),
                    on_layer(SECOND, 6, 0, 8, 0),
                    on_layer(FIRST, 14, 20, 6, 0),
                ],
                rendered: vec![rendered(6..14)],
                played: vec![
                    "/media/first.mxf 0..6 at 0 clip Some(0)",
                    "/cache/stretches/6-14 0..8 at 6 clip Some(1)",
                    "/media/first.mxf 20..26 at 14 clip Some(2)",
                ],
            },
        ];
        for shape in shapes {
            assert_eq!(
                played(shape.clips, &shape.rendered),
                shape.played,
                "{}",
                shape.name
            );
        }
    }

    // FIRST from its source frame 5, with an overlay over timeline frames 12 to 20
    fn overlaid_clip() -> (Vec<Clip>, Vec<RenderedStretch>) {
        (
            vec![on_layer(FIRST, 0, 5, 48, 0), on_layer(OVERLAY, 12, 0, 8, 1)],
            vec![rendered(12..20)],
        )
    }

    fn media_of_every_segment(timeline: Timeline, rendered: &[RenderedStretch]) -> Vec<String> {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(
            timeline,
            None,
            Some(Path::new(BLACK)),
            rendered,
            PlayState::Playing,
            &player,
        )
        .unwrap();
        player.reach_the_end();
        run.tick(&player).unwrap();
        player.clip_media()
    }

    #[test]
    fn a_rendered_stretch_over_a_clip_times_the_clip_subtitle_from_the_source_frame_at_its_start() {
        let (mut clips, stretches) = overlaid_clip();
        clips[0].subtitle = Some(PathBuf::from("/media/first.srt"));
        // the stretch starts on source frame 17
        assert_eq!(
            media_of_every_segment(timeline_of(clips), &stretches),
            vec![
                format!("{FIRST}: no sound, /media/first.srt from source frame 0"),
                "/cache/stretches/12-20: no sound, /media/first.srt from source frame -17"
                    .to_string(),
                format!("{FIRST}: no sound, /media/first.srt from source frame 0"),
            ]
        );

        let (clips, stretches) = overlaid_clip();
        let timeline = Timeline {
            subtitle: Some(PathBuf::from("/media/timeline.srt")),
            ..timeline_of(clips)
        };
        assert_eq!(
            media_of_every_segment(timeline, &stretches),
            vec![
                format!("{FIRST}: no sound, /media/timeline.srt from source frame 5"),
                "/cache/stretches/12-20: no sound, /media/timeline.srt from source frame -12"
                    .to_string(),
                format!("{FIRST}: no sound, /media/timeline.srt from source frame 5"),
            ]
        );
    }

    #[test]
    fn a_seek_across_either_edge_of_a_rendered_stretch_lands_in_the_segment_there() {
        let clips = vec![
            on_layer(FIRST, 0, 24, 48, 0),
            on_layer(OVERLAY, 12, 0, 12, 1),
        ];
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(
            timeline_of(clips),
            None,
            None,
            &[rendered(12..24)],
            PlayState::Playing,
            &player,
        )
        .unwrap();
        assert_eq!(
            player.calls(),
            vec![
                format!("load {FIRST} 24..36"),
                format!("queue {STRETCHES}/12-24 0..12")
            ]
        );
        let frame_seconds = |frame: u32| f64::from(frame) / f64::from(TIMELINE_FPS);

        run.seek(frame_seconds(11), &player).unwrap();
        assert_eq!(player.calls(), vec![format!("seek {}", frame_seconds(11))]);

        run.seek(frame_seconds(12), &player).unwrap();
        assert_eq!(
            player.calls(),
            vec![
                format!("load {STRETCHES}/12-24 0..12"),
                format!("queue {FIRST} 48..72"),
                "seek 0".to_string()
            ]
        );
        run.seek(frame_seconds(18), &player).unwrap();
        assert_eq!(player.calls(), vec![format!("seek {}", frame_seconds(6))]);
        let status = player.status().unwrap();
        assert_eq!(run.position(&status), Some(frame_seconds(18)));
        assert_eq!(run.playing_clip(&status), Some(0));

        run.seek(frame_seconds(24), &player).unwrap();
        assert_eq!(
            player.calls(),
            vec![format!("load {FIRST} 48..72"), "seek 0".to_string()]
        );
        run.seek(frame_seconds(23), &player).unwrap();
        assert_eq!(
            player.calls(),
            vec![
                format!("load {STRETCHES}/12-24 0..12"),
                format!("queue {FIRST} 48..72"),
                format!("seek {}", frame_seconds(11))
            ]
        );
        assert_eq!(
            run.position(&player.status().unwrap()),
            Some(frame_seconds(23))
        );
    }

    #[test]
    fn the_clip_at_a_frame_inside_a_rendered_stretch_is_the_layer_0_clip_under_it() {
        let frames = TimelineFrames::of(&timeline_of(vec![
            on_layer(OVERLAY, 12, 0, 12, 1),
            on_layer(FIRST, 0, 0, 24, 0),
            on_layer(OVERLAY, 30, 0, 6, 1),
            on_layer(THIRD, 36, 0, 12, 0),
        ]));
        assert_eq!(frames.clip_at(12), Some(1));
        assert_eq!(frames.clip_at(23), Some(1));
        assert_eq!(frames.clip_at(32), None);
        assert_eq!(frames.clip_at(36), Some(3));
    }

    #[test]
    fn the_time_left_follows_the_rate_so_far() {
        let elapsed = Duration::from_secs(3);
        let progress = |frames_rendered| PreviewRenderProgress {
            frames_rendered,
            total_frames: 24,
        };
        assert_eq!(
            TimelineRenderProgress::after(progress(6), elapsed),
            TimelineRenderProgress {
                frames_rendered: 6,
                total_frames: 24,
                seconds_left: Some(9.0),
            }
        );
        assert_eq!(
            TimelineRenderProgress::after(progress(24), elapsed).seconds_left,
            Some(0.0)
        );
        assert_eq!(
            TimelineRenderProgress::after(progress(0), elapsed).seconds_left,
            None
        );
    }

    fn write_card(path: &Path) {
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"])
            .arg(format!(
                "color=c=red:s={side}x{side}",
                side = grok_fixture::PICTURE_SIDE
            ))
            .args(["-frames:v", "1"])
            .arg(path)
            .output()
            .expect("ffmpeg");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
    }

    const PICTURE_FRAMES: usize = 24;
    const OVER_THE_GAP: Range<u64> = 12..20;

    #[test]
    fn a_rendered_codestream_stretch_plays_between_two_clips_with_the_sound_from_its_place() {
        let directory = tempfile::tempdir().unwrap();
        let picture = grok_fixture::write_picture_file(directory.path(), PICTURE_FRAMES);
        let card = directory.path().join("card.png");
        write_card(&card);
        let sound = directory.path().join("sound.wav");
        write_silence(&sound, 2);
        let picture_name = picture.display().to_string();
        let gap_frames = OVER_THE_GAP.end - OVER_THE_GAP.start;
        let timeline = Timeline {
            sound: SoundTrack::Override(sound.clone()),
            ..timeline_of(vec![
                on_layer(&picture_name, 0, 0, OVER_THE_GAP.start, 0),
                Clip {
                    subtitle: Some(PathBuf::from("/media/after.srt")),
                    ..on_layer(&picture_name, OVER_THE_GAP.end, 4, 12, 0)
                },
                on_layer(
                    &card.display().to_string(),
                    OVER_THE_GAP.start,
                    0,
                    gap_frames,
                    1,
                ),
            ])
        };
        let side = grok_fixture::PICTURE_SIDE;
        let options = EncodeRunOptions {
            fps: timeline.fps,
            picture: PictureProcessing {
                fit: Some(Fit {
                    box_width: side,
                    box_height: side,
                    raster_width: side,
                    raster_height: side,
                    placement: Placement::default(),
                }),
                ..PictureProcessing::default()
            },
            ..EncodeRunOptions::default()
        };
        let work_dir = directory.path().join("work");
        let state = overlay_state(PreviewOverlays::default());
        let player = FakePlayer::default();
        let mut progress = Vec::new();
        let render = StretchRender {
            options: &options,
            on_progress: &mut |reported| progress.push(reported),
        };

        load_grok_timeline(
            &state,
            timeline,
            &work_dir,
            Some(render),
            &not_cancelled(),
            PlayState::Playing,
            &player,
        )
        .unwrap();
        let stretches = work_dir.join(TIMELINE_GROK_STRETCHES_DIRECTORY);
        let entries: Vec<PathBuf> = std::fs::read_dir(&stretches)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        let [rendered_stretch] = entries.as_slice() else {
            panic!("{entries:?} is not one rendered stretch");
        };
        assert_eq!(
            std::fs::read_dir(rendered_stretch).unwrap().count() as u64,
            gap_frames
        );
        let last = progress.last().expect("the stretch rendered");
        assert_eq!(
            (last.frames_rendered, last.total_frames),
            (gap_frames, gap_frames)
        );
        let codestreams = rendered_stretch.display();
        assert_eq!(
            player.calls(),
            vec![
                format!("load {picture_name} 0..12"),
                format!("queue {codestreams} 0..8")
            ]
        );

        assert!(
            state.with_grok_timeline(|run, _| {
                player.reach_the_end();
                run.tick(&player)
            }) == Some(Ok(true))
        );
        assert_eq!(player.calls(), vec![format!("queue {picture_name} 4..16")]);
        // an override sound plays as it is
        let sound_from = |start: u64| format!("{} from {start} at 24/1", sound.display());
        assert_eq!(
            player.clip_media(),
            vec![
                format!("{picture_name}: {}, no subtitles", sound_from(0)),
                format!("{codestreams}: {}, no subtitles", sound_from(12)),
                format!(
                    "{picture_name}: {}, /media/after.srt from source frame 0",
                    sound_from(20)
                ),
            ]
        );

        state.clear_timeline();
        assert!(
            rendered_stretch.exists(),
            "clearing the timeline removed its rendered stretch"
        );
    }
}
