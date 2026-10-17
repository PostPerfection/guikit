use std::sync::atomic::Ordering;
use std::time::Duration;

use postkit::encode::{FrameRate, InputType};
use postkit::grok_player::{FrameRange, GrokPlayer, SourceOptions};
use postkit::timeline::{clip_name, Clip, Timeline};
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
    clip_starts: Vec<u64>,
    total_frames: u64,
}

impl TimelineFrames {
    pub(super) fn of(timeline: &Timeline) -> TimelineFrames {
        TimelineFrames {
            fps: timeline.fps,
            clip_starts: timeline.clip_starts(),
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

    fn clip_at(&self, frame: u64) -> usize {
        self.clip_starts
            .partition_point(|&start| start <= frame)
            .saturating_sub(1)
    }
}

// a clip the grok player plays loads with its range, and the clip after it is queued behind it
pub struct GrokTimelineRun {
    frames: TimelineFrames,
    clips: Vec<Clip>,
    current_clip: usize,
    queued_clip: Option<usize>,
}

impl GrokTimelineRun {
    pub fn start(timeline: Timeline, player: &impl TimelinePlayer) -> Result<Self, String> {
        let mut run = GrokTimelineRun {
            frames: TimelineFrames::of(&timeline),
            clips: timeline.clips,
            current_clip: 0,
            queued_clip: None,
        };
        run.load_clip(0, player)?;
        Ok(run)
    }

    fn clip_path(&self, index: usize) -> String {
        self.clips[index].picture.display().to_string()
    }

    fn clip_options(&self, index: usize) -> SourceOptions {
        let clip = &self.clips[index];
        SourceOptions {
            range: Some(FrameRange {
                in_frame: clip.in_frame,
                out_frame: Some(clip.out_frame),
            }),
            ..SourceOptions::default()
        }
    }

    fn load_clip(&mut self, index: usize, player: &impl TimelinePlayer) -> Result<(), String> {
        player.load(&self.clips[index].picture, self.clip_options(index))?;
        self.current_clip = index;
        self.queued_clip = None;
        self.queue_clip(index + 1, player)
    }

    fn queue_clip(&mut self, index: usize, player: &impl TimelinePlayer) -> Result<(), String> {
        if index >= self.clips.len() {
            return Ok(());
        }
        player.queue_next(&self.clips[index].picture, self.clip_options(index))?;
        self.queued_clip = Some(index);
        Ok(())
    }

    // the clip on screen, the queued one already once the player has handed off to it
    pub fn playing_clip(&self, status: &PlayerStatus) -> usize {
        match self.queued_clip {
            Some(queued) if handed_off(status, &self.clip_path(queued)) => queued,
            _ => self.current_clip,
        }
    }

    fn follow_hand_off(&mut self, status: &PlayerStatus) {
        let playing = self.playing_clip(status);
        if playing != self.current_clip {
            self.current_clip = playing;
            self.queued_clip = None;
        }
    }

    // false once the player shows a source this timeline did not load
    pub fn tick(&mut self, player: &impl TimelinePlayer) -> Result<bool, String> {
        let status = player.status()?;
        self.follow_hand_off(&status);
        if status.source.as_deref() != Some(self.clip_path(self.current_clip).as_str()) {
            return Ok(false);
        }
        let next = self.current_clip + 1;
        if self.queued_clip.is_some() || next == self.clips.len() {
            return Ok(true);
        }
        // a clip shorter than a tick ends before its follower could be queued
        if status.eof {
            self.load_clip(next, player)?;
        } else {
            self.queue_clip(next, player)?;
        }
        Ok(true)
    }

    pub fn position(&self, status: &PlayerStatus) -> Option<f64> {
        // the player counts in its own frame rate, which for a codestream directory is not the timeline's
        let frames_into_clip = (status.position? * status.container_fps?).round() as u64;
        let start = self.frames.clip_starts[self.playing_clip(status)];
        Some(self.frames.seconds_at(start + frames_into_clip))
    }

    pub fn seek(&mut self, seconds: f64, player: &impl TimelinePlayer) -> Result<(), String> {
        let status = player.status()?;
        self.follow_hand_off(&status);
        let target = self.frames.frame_at(seconds);
        let clip = self.frames.clip_at(target);
        if clip != self.current_clip {
            self.load_clip(clip, player)?;
            if status.paused {
                player.set_paused(true)?;
            }
        }
        let clip_fps = player.status()?.container_fps.ok_or_else(|| {
            format!(
                "the player reports no frame rate for {}",
                self.clip_path(clip)
            )
        })?;
        let frames_into_clip = target - self.frames.clip_starts[clip];
        player.seek_absolute(frames_into_clip as f64 / clip_fps)
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
}

// every clip on one player: grok takes JPEG 2000, mpv the rest
pub(super) fn timeline_backend(timeline: &Timeline) -> Result<Backend, String> {
    let on_grok = |clip: &Clip| GrokPlayer::accepts(&clip.picture);
    let first_grok = timeline.clips.iter().position(on_grok);
    let first_mpv = timeline.clips.iter().position(|clip| !on_grok(clip));
    match (first_grok, first_mpv) {
        (Some(grok), Some(mpv)) => Err(format!(
            "{} plays on the JPEG 2000 player and {} on mpv: a timeline preview plays every clip \
             on one of them",
            clip_name(grok, &timeline.clips[grok]),
            clip_name(mpv, &timeline.clips[mpv])
        )),
        (Some(_), None) => Ok(Backend::Grok),
        _ => {
            refuse_stills_on_mpv(timeline)?;
            Ok(Backend::Mpv)
        }
    }
}

fn refuse_stills_on_mpv(timeline: &Timeline) -> Result<(), String> {
    let stills = timeline.clips.iter().enumerate().find(|(_, clip)| {
        postkit::encode::detect_input_type(&clip.picture) == InputType::ImageSequence
    });
    match stills {
        Some((index, clip)) => Err(format!(
            "{} is a sequence of stills, which mpv cannot play inside a timeline",
            clip_name(index, clip)
        )),
        None => Ok(()),
    }
}

impl PreviewPlayer {
    pub(super) fn clear_timeline(&self) {
        *self.timeline.lock().unwrap() = None;
    }

    pub(super) fn install_timeline(&self, playback: TimelinePlayback) -> u64 {
        let generation = self.timeline_generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self.timeline.lock().unwrap() = Some(LoadedTimeline {
            generation,
            playback,
        });
        generation
    }

    // false once the run is gone, replaced, or has stopped following the player
    fn tick_timeline(&self, generation: u64) -> bool {
        let mut loaded = self.timeline.lock().unwrap();
        let Some(LoadedTimeline {
            generation: current,
            playback: TimelinePlayback::Grok(run),
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
                    clip_index: seconds.map(|seconds| frames.clip_at(frames.frame_at(seconds))),
                }
            }
            TimelinePlayback::Grok(run) => {
                let status = PreviewRunnerPlayer(self).status()?;
                TimelinePosition {
                    seconds: run.position(&status),
                    clip_index: Some(run.playing_clip(&status)),
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
    use super::*;
    use postkit::timeline::{ClipSound, SoundTrack, TIMELINE_FORMAT_VERSION};
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
    }

    impl FakePlayer {
        fn calls(&self) -> Vec<String> {
            self.calls.take()
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

    fn clip(picture: &str, in_frame: u64, out_frame: u64) -> Clip {
        Clip {
            picture: PathBuf::from(picture),
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
                clip(FIRST, 24, 72),
                clip(SECOND, 0, 24),
                clip(THIRD, 100, 112),
            ],
            sound: SoundTrack::PerClip,
            subtitle: None,
            reel_starts: Vec::new(),
        }
    }

    #[test]
    fn the_first_clip_loads_with_its_range_and_the_second_queues_behind_it() {
        let player = FakePlayer::default();
        GrokTimelineRun::start(three_clips(), &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), &player).unwrap();
        player.play_to(1.0);
        let status = player.status().unwrap();
        assert_eq!(run.position(&status), Some(1.0));
        assert_eq!(run.playing_clip(&status), 0);

        player.reach_the_end();
        player.play_to(0.5);
        // read before the tick catches up with the hand-off
        let status = player.status().unwrap();
        assert_eq!(run.position(&status), Some(2.5));
        assert_eq!(run.playing_clip(&status), 1);

        player.calls();
        assert!(run.tick(&player).unwrap());
        assert_eq!(player.calls(), vec![format!("queue {THIRD} 100..112")]);
        assert_eq!(run.playing_clip(&player.status().unwrap()), 1);
    }

    #[test]
    fn the_last_clip_queues_nothing_after_it() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(three_clips(), &player).unwrap();
        player.reach_the_end();
        run.tick(&player).unwrap();
        player.reach_the_end();
        player.calls();
        assert!(run.tick(&player).unwrap());
        assert_eq!(run.playing_clip(&player.status().unwrap()), 2);
        assert!(player.calls().is_empty());
    }

    #[test]
    fn a_clip_that_ends_between_ticks_loads_the_clip_after_it() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(three_clips(), &player).unwrap();
        player.reach_the_end();
        player.reach_the_end();
        player.calls();
        assert!(run.tick(&player).unwrap());
        assert_eq!(player.calls(), vec![format!("load {THIRD} 100..112")]);
        assert_eq!(run.playing_clip(&player.status().unwrap()), 2);
    }

    #[test]
    fn a_seek_inside_the_playing_clip_seeks_within_its_range() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(three_clips(), &player).unwrap();
        player.calls();
        run.seek(1.5, &player).unwrap();
        assert_eq!(player.calls(), vec!["seek 1.5"]);
    }

    #[test]
    fn a_seek_into_another_clip_loads_it_and_queues_the_one_after() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(three_clips(), &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), &player).unwrap();
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
        timeline.clips = vec![clip("/media/a.mov", 0, 1), clip(&codestreams, 0, 1)];
        let error = timeline_backend(&timeline).unwrap_err();
        assert!(
            error.contains(&format!(
                "clip 2 ({codestreams}) plays on the JPEG 2000 player"
            )) && error.contains("clip 1 (/media/a.mov) on mpv"),
            "{error}"
        );

        timeline.clips.remove(0);
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
            clip("/media/a.mov", 0, 1),
            clip(&stills.display().to_string(), 0, 1),
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
