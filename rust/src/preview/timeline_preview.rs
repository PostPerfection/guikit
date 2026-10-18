use std::io::ErrorKind;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use postkit::encode::{FrameRate, InputType};
use postkit::grok_player::{
    FrameRange, GrokPlayer, SoundFile, SourceOptions, SourceSubtitles, SubtitleFile,
};
use postkit::pipeline::{run_encode_with_options, EncodeRunOptions};
use postkit::timeline::{clip_name, Clip, Timeline, BLACK_GAP_SIZE};
use tauri::Manager;

use super::screening_runner::{handed_off, PlayerStatus, PreviewRunnerPlayer, RunnerPlayer};
use super::{json_number, Backend, PreviewPlayer};

const TIMELINE_TICK_INTERVAL: Duration = Duration::from_millis(100);
const TIMELINE_POSITION_FIELD: &str = "timeline_position";
const TIMELINE_CLIP_INDEX_FIELD: &str = "timeline_clip_index";

// a grok timeline seeks within a clip and keeps the pause across the load into another clip
pub trait TimelinePlayer: RunnerPlayer {
    fn seek_absolute(&self, seconds: f64) -> Result<(), String>;
    fn set_paused(&self, paused: bool) -> Result<(), String>;
}

impl TimelinePlayer for PreviewRunnerPlayer<'_> {
    fn seek_absolute(&self, seconds: f64) -> Result<(), String> {
        self.0.player()?.grok().seek_absolute(seconds);
        Ok(())
    }

    fn set_paused(&self, paused: bool) -> Result<(), String> {
        self.0.player()?.grok().set_paused(paused);
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

// a layer 0 clip, or black over a gap in layer 0
struct PlayedSegment {
    picture: PathBuf,
    start: u64,
    in_frame: u64,
    out_frame: u64,
    // None for a gap
    clip_index: Option<usize>,
    subtitle: Option<PathBuf>,
}

// the layer 0 clips and the gaps between them in start order, each gap playing the first frames of black
fn played_segments(
    timeline: &Timeline,
    black: Option<&Path>,
) -> Result<Vec<PlayedSegment>, String> {
    let mut segments: Vec<PlayedSegment> = timeline
        .base_clips()
        .into_iter()
        .map(|(index, clip)| PlayedSegment {
            picture: clip.picture.clone(),
            start: clip.start,
            in_frame: clip.in_frame,
            out_frame: clip.out_frame,
            clip_index: Some(index),
            subtitle: clip.subtitle.clone(),
        })
        .collect();
    for gap in timeline.base_gaps() {
        let frames = gap.end - gap.start;
        let Some(black) = black else {
            return Err(format!(
                "layer 0 has a gap of {frames} frames at frame {}, and no black codestreams were \
                 made for it",
                gap.start
            ));
        };
        segments.push(PlayedSegment {
            picture: black.to_path_buf(),
            start: gap.start,
            in_frame: 0,
            out_frame: frames,
            clip_index: None,
            subtitle: None,
        });
    }
    segments.sort_by_key(|segment| segment.start);
    Ok(segments)
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
fn remove_work_path(path: &Path) -> Result<(), String> {
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
        player: &impl TimelinePlayer,
    ) -> Result<Self, String> {
        let mut run = GrokTimelineRun {
            frames: TimelineFrames::of(&timeline),
            segments: played_segments(&timeline, black)?,
            sound,
            timeline_subtitle: timeline.subtitle,
            current_segment: 0,
            queued_segment: None,
        };
        run.load_segment(0, player)?;
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
            (Some(own), _) => (own, 0),
            (None, Some(timeline)) => (timeline, segment.in_frame as i64 - segment.start as i64),
            (None, None) => return SourceSubtitles::Empty,
        };
        SourceSubtitles::File(SubtitleFile {
            path: path.clone(),
            source_frame_at_time_zero,
        })
    }

    fn load_segment(&mut self, index: usize, player: &impl TimelinePlayer) -> Result<(), String> {
        player.load(&self.segments[index].picture, self.segment_options(index))?;
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
            self.load_segment(next, player)?;
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
            self.load_segment(segment, player)?;
            if status.paused {
                player.set_paused(true)?;
            }
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
        _ => {
            refuse_stills_on_mpv(&base)?;
            Ok(Backend::Mpv)
        }
    }
}

fn refuse_stills_on_mpv(base: &[(usize, &Clip)]) -> Result<(), String> {
    let stills = base.iter().find(|(_, clip)| {
        postkit::encode::detect_input_type(&clip.picture) == InputType::ImageSequence
    });
    match stills {
        Some((index, clip)) => Err(format!(
            "{} is a sequence of stills, which mpv cannot play inside a timeline",
            clip_name(*index, clip)
        )),
        None => Ok(()),
    }
}

impl PreviewPlayer {
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
    use super::super::{load_grok_timeline, PreviewOverlays};
    use super::*;
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
        // the grok player starts a load playing
        fn load(&self, source: &Path, options: SourceOptions) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("load {}", described(source, &options)));
            self.record_media(source, &options);
            *self.status.borrow_mut() = PlayerStatus {
                source: Some(source.display().to_string()),
                position: Some(0.0),
                container_fps: Some(CODESTREAM_DIRECTORY_FPS),
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

        fn set_paused(&self, paused: bool) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("paused {paused}"));
            self.status.borrow_mut().paused = paused;
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
        GrokTimelineRun::start(three_clips(), None, None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(timeline, None, None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(timeline, None, None, &player).unwrap();
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
        let mut run =
            GrokTimelineRun::start(gapped_clips(), None, Some(Path::new(BLACK)), &player).unwrap();
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
        let refused = GrokTimelineRun::start(gapped_clips(), None, None, &player);
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

    #[test]
    fn clearing_a_grok_timeline_removes_the_sound_and_black_rendered_for_it() {
        const SAMPLE_RATE: u32 = 48_000;
        const CHANNELS: u16 = 2;
        const BYTES_PER_SAMPLE: u16 = 3;
        const BITS_PER_BYTE: u16 = 8;
        let directory = tempfile::tempdir().unwrap();
        let clip_sound = directory.path().join("clip.wav");
        let spec = WavSpec {
            channels: CHANNELS,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: BYTES_PER_SAMPLE * BITS_PER_BYTE,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&clip_sound, spec, u64::from(SAMPLE_RATE)).unwrap();
        let second_of_silence =
            SAMPLE_RATE as usize * usize::from(CHANNELS) * usize::from(BYTES_PER_SAMPLE);
        writer.write_bytes(&vec![0u8; second_of_silence]).unwrap();
        writer.finalize().unwrap();
        let mut timeline = gapped_clips();
        timeline.clips[0].sound = ClipSound::File(clip_sound.clone());
        let work_dir = directory.path().join("work");
        let sound_file = work_dir.join(super::super::TIMELINE_SOUND_FILE);
        let black_dir = work_dir.join(super::super::TIMELINE_BLACK_DIRECTORY);
        let state = overlay_state(PreviewOverlays::default());
        let player = FakePlayer::default();

        load_grok_timeline(&state, timeline, &work_dir, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, None, &player).unwrap();
        player.calls();
        run.seek(1.5, &player).unwrap();
        assert_eq!(player.calls(), vec!["seek 1.5"]);
    }

    #[test]
    fn a_seek_into_another_clip_loads_it_and_queues_the_one_after() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(three_clips(), None, None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, None, &player).unwrap();
        player.set_paused(true).unwrap();
        player.calls();
        run.seek(3.25, &player).unwrap();
        assert_eq!(
            player.calls(),
            vec![
                format!("load {THIRD} 100..112"),
                "paused true".to_string(),
                "seek 0.25".to_string()
            ]
        );
    }

    #[test]
    fn a_relative_seek_moves_from_the_timeline_position() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(three_clips(), None, None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, None, &player).unwrap();
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
    fn a_timeline_of_stills_is_refused_on_mpv() {
        let directory = tempfile::tempdir().unwrap();
        let stills = directory.path().join("stills");
        std::fs::create_dir_all(&stills).unwrap();
        std::fs::write(stills.join("shot_0000.png"), b"png").unwrap();
        let mut timeline = three_clips();
        timeline.clips = vec![
            clip("/media/a.mov", 0, 0, 1),
            clip(&stills.display().to_string(), 1, 0, 1),
        ];
        let error = timeline_backend(&timeline).unwrap_err();
        assert!(
            error.contains("clip 2") && error.contains("stills"),
            "{error}"
        );

        timeline.clips.remove(1);
        assert_eq!(timeline_backend(&timeline), Ok(Backend::Mpv));
    }
}
