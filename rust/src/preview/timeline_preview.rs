use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use postkit::encode::{FrameRate, InputType};
use postkit::grok_player::{
    FrameRange, GrokPlayer, SoundFile, SourceOptions, SourceSubtitles, SubtitleFile,
};
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
    // the timeline's rendered sound, which every clip plays from its place on the timeline
    sound: Option<PathBuf>,
    timeline_subtitle: Option<PathBuf>,
    current_clip: usize,
    queued_clip: Option<usize>,
}

impl GrokTimelineRun {
    pub fn start(
        timeline: Timeline,
        sound: Option<PathBuf>,
        player: &impl TimelinePlayer,
    ) -> Result<Self, String> {
        let mut run = GrokTimelineRun {
            frames: TimelineFrames::of(&timeline),
            clips: timeline.clips,
            sound,
            timeline_subtitle: timeline.subtitle,
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
        let start = self.frames.clip_starts[index];
        SourceOptions {
            range: Some(FrameRange {
                in_frame: clip.in_frame,
                out_frame: Some(clip.out_frame),
            }),
            sound: self.sound.clone().map(|path| SoundFile {
                path,
                fps: self.frames.fps,
                first_frame: start,
            }),
            subtitles: self.clip_subtitles(index),
            codestream_fps: Some(self.frames.fps),
            ..SourceOptions::default()
        }
    }

    // a clip's own file is timed against its source, the timeline's file against the timeline
    fn clip_subtitles(&self, index: usize) -> SourceSubtitles {
        let clip = &self.clips[index];
        let (path, source_frame_at_time_zero) = match (&clip.subtitle, &self.timeline_subtitle) {
            (Some(own), _) => (own, 0),
            (None, Some(timeline)) => (
                timeline,
                clip.in_frame as i64 - self.frames.clip_starts[index] as i64,
            ),
            (None, None) => return SourceSubtitles::Empty,
        };
        SourceSubtitles::File(SubtitleFile {
            path: path.clone(),
            source_frame_at_time_zero,
        })
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
    // the sound and subtitles rendered for mpv, removed with the timeline
    work_files: Vec<PathBuf>,
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
        let Some(cleared) = self.timeline.lock().unwrap().take() else {
            return;
        };
        for file in cleared.work_files {
            match std::fs::remove_file(&file) {
                Err(error) if error.kind() != ErrorKind::NotFound => {
                    eprintln!("[timeline] cannot remove {}: {error}", file.display())
                }
                _ => {}
            }
        }
    }

    pub(super) fn install_timeline(
        &self,
        playback: TimelinePlayback,
        work_files: Vec<PathBuf>,
    ) -> u64 {
        let generation = self.timeline_generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self.timeline.lock().unwrap() = Some(LoadedTimeline {
            generation,
            playback,
            work_files,
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
        GrokTimelineRun::start(three_clips(), None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, &player).unwrap();
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

    const TIMELINE_SOUND: &str = "/cache/timeline-sound.wav";

    #[test]
    fn every_clip_plays_the_timeline_sound_from_its_place_on_the_timeline() {
        let player = FakePlayer::default();
        let mut run =
            GrokTimelineRun::start(three_clips(), Some(PathBuf::from(TIMELINE_SOUND)), &player)
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
        let mut run = GrokTimelineRun::start(timeline, None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(timeline, None, &player).unwrap();
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

    #[test]
    fn clearing_a_grok_timeline_removes_the_sound_rendered_for_it() {
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
        let mut timeline = three_clips();
        timeline.clips[0].sound = ClipSound::File(clip_sound.clone());
        let work_dir = directory.path().join("work");
        let sound_file = work_dir.join(super::super::TIMELINE_SOUND_FILE);
        let state = overlay_state(PreviewOverlays::default());
        let player = FakePlayer::default();

        load_grok_timeline(&state, timeline, &work_dir, &player).unwrap();
        assert!(sound_file.exists());
        assert!(
            player.clip_media()[0].contains(&format!("{} from 0", sound_file.display())),
            "the first clip does not play the rendered sound"
        );

        state.clear_timeline();
        assert!(
            !sound_file.exists(),
            "{} is left behind",
            sound_file.display()
        );
        assert!(clip_sound.exists());
        assert_eq!(state.timeline_duration(), None);
    }

    #[test]
    fn the_last_clip_queues_nothing_after_it() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(three_clips(), None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, &player).unwrap();
        player.calls();
        run.seek(1.5, &player).unwrap();
        assert_eq!(player.calls(), vec!["seek 1.5"]);
    }

    #[test]
    fn a_seek_into_another_clip_loads_it_and_queues_the_one_after() {
        let player = FakePlayer::default();
        let mut run = GrokTimelineRun::start(three_clips(), None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, &player).unwrap();
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
        let mut run = GrokTimelineRun::start(three_clips(), None, &player).unwrap();
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
