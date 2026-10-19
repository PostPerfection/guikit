use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;

use postkit::encode::{FrameRate, SourceColour};
use postkit::picture_processing::{Fit, PictureProcessing, Placement};
use postkit::pipeline::EncodeRunOptions;
use postkit::timeline::{Clip, ClipSound, SoundTrack, Timeline, TIMELINE_FORMAT_VERSION};

use super::end_of_file_tests::{overlay_state, pump_until, write_clip};
use super::grok_fixture::wait_until;
use super::timeline_preview::{TimelineRenderProgress, TIMELINE_LOAD_CANCELLED};
use super::{
    load_timeline, poll_metadata, stop_preview, Backend, Player, PreviewOverlays, PreviewPlayer,
    TIMELINE_BLACK_FILE, TIMELINE_MPV_STRETCHES_DIRECTORY, TIMELINE_SOUND_FILE,
    TIMELINE_SUBTITLE_FILE,
};

const CLIP_FPS: u32 = 24;
// half a second of the first clip, then all of the second
const FIRST_CLIP_FRAMES: (u64, u64) = (6, 18);
const SECOND_CLIP_FRAMES: (u64, u64) = (0, 24);
const TIMELINE_SECONDS: f64 = 1.5;
const FRAME_SECONDS: f64 = 1.0 / CLIP_FPS as f64;
const INTO_THE_SECOND_CLIP_SECONDS: f64 = 0.6;
const PICTURE_AND_SOUND_TRACKS: i64 = 2;
const CLIP_SUBTITLES: &str = "1\n00:00:00,000 --> 00:00:01,000\nhello\n";
// a quarter second of the first clip, two seconds of black, then all of the second
const GAPPED_FIRST_CLIP_FRAMES: (u64, u64) = (0, 6);
const GAPPED_SECOND_CLIP_START: u64 = 54;
const GAPPED_TIMELINE_SECONDS: f64 = 3.25;
const INTO_THE_GAP_SECONDS: f64 = 0.5;

fn clip(picture: PathBuf, start: u64, (in_frame, out_frame): (u64, u64)) -> Clip {
    Clip {
        picture,
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

fn tone(directory: &Path) -> PathBuf {
    let sound = directory.join("tone.wav");
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("sine=frequency=1000:sample_rate=48000:duration=2")
        .arg(&sound)
        .output()
        .expect("ffmpeg");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    sound
}

#[test]
fn an_mpv_timeline_lasts_as_long_as_its_clips_and_reports_the_clip_it_plays() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.mov");
    let second = directory.path().join("second.mov");
    write_clip(&first, 1);
    write_clip(&second, 1);
    let timeline = Timeline {
        version: TIMELINE_FORMAT_VERSION,
        fps: FrameRate::whole(CLIP_FPS),
        clips: vec![
            clip(first, 0, FIRST_CLIP_FRAMES),
            clip(
                second,
                FIRST_CLIP_FRAMES.1 - FIRST_CLIP_FRAMES.0,
                SECOND_CLIP_FRAMES,
            ),
        ],
        sound: SoundTrack::Override(tone(directory.path())),
        subtitle: None,
        reel_starts: Vec::new(),
    };
    let player = Player::new().unwrap();
    // never the real sound device
    player.mpv().set_property("ao", "null").unwrap();
    player.init_software().unwrap();
    let state = overlay_state(PreviewOverlays::default());

    load_timeline(
        &player,
        &state,
        timeline,
        None,
        directory.path(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(player.active(), Backend::Mpv);
    pump_until(player.mpv(), |_| player.mpv().get_duration().is_ok())
        .unwrap_or_else(|last| panic!("mpv never read the timeline, last read {last}"));
    let duration = player.mpv().get_duration().unwrap();
    assert!(
        (duration - TIMELINE_SECONDS).abs() < FRAME_SECONDS,
        "mpv plays the timeline for {duration}s"
    );
    assert_eq!(state.timeline_duration(), Some(TIMELINE_SECONDS));
    let tracks = player
        .mpv()
        .get_property_string("track-list/count")
        .unwrap();
    assert_eq!(tracks, PICTURE_AND_SOUND_TRACKS.to_string());

    pump_until(player.mpv(), |_| {
        player.mpv().get_position().unwrap_or(0.0) > INTO_THE_SECOND_CLIP_SECONDS
    })
    .unwrap_or_else(|last| panic!("the timeline never reached its second clip, last read {last}"));
    let metadata: serde_json::Value =
        serde_json::from_str(&poll_metadata(&player, &state).unwrap()).unwrap();
    assert_eq!(metadata["timeline_clip_index"], 1, "{metadata}");
    assert!(
        metadata["timeline_position"].as_f64().unwrap() > INTO_THE_SECOND_CLIP_SECONDS,
        "{metadata}"
    );
}

#[test]
fn clearing_an_mpv_timeline_removes_the_sound_and_subtitles_rendered_for_it() {
    let directory = tempfile::tempdir().unwrap();
    let picture = directory.path().join("clip.mov");
    write_clip(&picture, 1);
    let clip_sound = tone(directory.path());
    let clip_subtitles = directory.path().join("clip.srt");
    std::fs::write(&clip_subtitles, CLIP_SUBTITLES).unwrap();
    let timeline = Timeline {
        version: TIMELINE_FORMAT_VERSION,
        fps: FrameRate::whole(CLIP_FPS),
        clips: vec![Clip {
            sound: ClipSound::File(clip_sound.clone()),
            subtitle: Some(clip_subtitles.clone()),
            ..clip(picture, 0, SECOND_CLIP_FRAMES)
        }],
        sound: SoundTrack::PerClip,
        subtitle: None,
        reel_starts: Vec::new(),
    };
    let work_dir = directory.path().join("work");
    let sound_file = work_dir.join(TIMELINE_SOUND_FILE);
    let subtitle_file = work_dir.join(TIMELINE_SUBTITLE_FILE);
    let player = Player::new().unwrap();
    // never the real sound device
    player.mpv().set_property("ao", "null").unwrap();
    player.init_software().unwrap();
    let state = overlay_state(PreviewOverlays::default());

    load_timeline(&player, &state, timeline, None, &work_dir, &mut |_| {}).unwrap();
    assert!(sound_file.exists() && subtitle_file.exists());

    state.clear_timeline();
    assert!(
        !sound_file.exists(),
        "{} is left behind",
        sound_file.display()
    );
    assert!(
        !subtitle_file.exists(),
        "{} is left behind",
        subtitle_file.display()
    );
    assert!(clip_sound.exists() && clip_subtitles.exists());
    assert_eq!(state.timeline_duration(), None);
}

#[test]
fn an_mpv_timeline_plays_black_through_a_gap_and_reports_no_clip_there() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.mov");
    let second = directory.path().join("second.mov");
    write_clip(&first, 1);
    write_clip(&second, 1);
    let timeline = Timeline {
        version: TIMELINE_FORMAT_VERSION,
        fps: FrameRate::whole(CLIP_FPS),
        clips: vec![
            clip(first, 0, GAPPED_FIRST_CLIP_FRAMES),
            clip(second, GAPPED_SECOND_CLIP_START, SECOND_CLIP_FRAMES),
        ],
        sound: SoundTrack::PerClip,
        subtitle: None,
        reel_starts: Vec::new(),
    };
    let work_dir = directory.path().join("work");
    let black_file = work_dir.join(TIMELINE_BLACK_FILE);
    let player = Player::new().unwrap();
    // never the real sound device
    player.mpv().set_property("ao", "null").unwrap();
    player.init_software().unwrap();
    let state = overlay_state(PreviewOverlays::default());

    load_timeline(&player, &state, timeline, None, &work_dir, &mut |_| {}).unwrap();
    assert!(black_file.exists());
    pump_until(player.mpv(), |_| player.mpv().get_duration().is_ok())
        .unwrap_or_else(|last| panic!("mpv never read the timeline, last read {last}"));
    let duration = player.mpv().get_duration().unwrap();
    assert!(
        (duration - GAPPED_TIMELINE_SECONDS).abs() < FRAME_SECONDS,
        "mpv plays the timeline for {duration}s"
    );

    pump_until(player.mpv(), |_| {
        player.mpv().get_position().unwrap_or(0.0) > INTO_THE_GAP_SECONDS
    })
    .unwrap_or_else(|last| panic!("the timeline never reached its gap, last read {last}"));
    let metadata: serde_json::Value =
        serde_json::from_str(&poll_metadata(&player, &state).unwrap()).unwrap();
    assert!(metadata["timeline_clip_index"].is_null(), "{metadata}");

    state.clear_timeline();
    assert!(
        !black_file.exists(),
        "{} is left behind",
        black_file.display()
    );
}

// the clips write_clip writes
const CLIP_WIDTH: u32 = 320;
const CLIP_HEIGHT: u32 = 180;

fn write_card(directory: &Path) -> PathBuf {
    let card = directory.join("card.png");
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("color=c=lime:s={CLIP_WIDTH}x{CLIP_HEIGHT}"))
        .args(["-frames:v", "1"])
        .arg(&card)
        .output()
        .expect("ffmpeg");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    card
}

fn fitted_to_the_clips() -> EncodeRunOptions {
    EncodeRunOptions {
        fps: FrameRate::whole(CLIP_FPS),
        source_colour: SourceColour::KeepRgb,
        picture: PictureProcessing {
            fit: Some(Fit {
                box_width: CLIP_WIDTH,
                box_height: CLIP_HEIGHT,
                raster_width: CLIP_WIDTH,
                raster_height: CLIP_HEIGHT,
                placement: Placement::default(),
            }),
            ..PictureProcessing::default()
        },
        ..EncodeRunOptions::default()
    }
}

fn timeline_of(clips: Vec<Clip>) -> Timeline {
    Timeline {
        version: TIMELINE_FORMAT_VERSION,
        fps: FrameRate::whole(CLIP_FPS),
        clips,
        sound: SoundTrack::PerClip,
        subtitle: None,
        reel_starts: Vec::new(),
    }
}

fn software_player() -> Player {
    let player = Player::new().unwrap();
    // never the real sound device
    player.mpv().set_property("ao", "null").unwrap();
    player.init_software().unwrap();
    player
}

const HELD_FRAMES: u64 = 24;
const INTO_THE_STILL_SECONDS: f64 = 0.1;
const INTO_THE_CLIP_AFTER_SECONDS: f64 = 1.1;
const HELD_STILL_TIMELINE_SECONDS: f64 = 2.0;

#[test]
fn a_held_still_on_layer_0_renders_and_plays_on_mpv_before_the_clip_after_it() {
    let directory = tempfile::tempdir().unwrap();
    let clip_after = directory.path().join("after.mov");
    write_clip(&clip_after, 1);
    let card = write_card(directory.path());
    let timeline = timeline_of(vec![
        clip(card, 0, (0, HELD_FRAMES)),
        clip(clip_after, HELD_FRAMES, SECOND_CLIP_FRAMES),
    ]);
    let work_dir = directory.path().join("work");
    let player = software_player();
    let state = overlay_state(PreviewOverlays::default());
    let mut progress = Vec::new();

    let loaded = load_timeline(
        &player,
        &state,
        timeline,
        Some(&fitted_to_the_clips()),
        &work_dir,
        &mut |reported| progress.push(reported),
    );
    assert_eq!(loaded, Ok(None));
    assert!(
        progress
            .windows(2)
            .all(|pair| pair[0].frames_rendered < pair[1].frames_rendered),
        "{progress:?}"
    );
    assert!(
        progress
            .iter()
            .all(|reported| reported.total_frames == HELD_FRAMES),
        "{progress:?}"
    );
    assert_eq!(
        progress.last().map(|last| last.frames_rendered),
        Some(HELD_FRAMES)
    );
    let stretches: Vec<PathBuf> =
        std::fs::read_dir(work_dir.join(TIMELINE_MPV_STRETCHES_DIRECTORY))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
    assert_eq!(stretches.len(), 1, "{stretches:?}");

    pump_until(player.mpv(), |_| player.mpv().get_duration().is_ok())
        .unwrap_or_else(|last| panic!("mpv never read the timeline, last read {last}"));
    let duration = player.mpv().get_duration().unwrap();
    assert!(
        (duration - HELD_STILL_TIMELINE_SECONDS).abs() < FRAME_SECONDS,
        "mpv plays the timeline for {duration}s"
    );
    for (seconds, clip_index) in [
        (INTO_THE_STILL_SECONDS, 0),
        (INTO_THE_CLIP_AFTER_SECONDS, 1),
    ] {
        pump_until(player.mpv(), |_| {
            player.mpv().get_position().unwrap_or(0.0) > seconds
        })
        .unwrap_or_else(|last| panic!("the timeline never reached {seconds}s, last read {last}"));
        let metadata: serde_json::Value =
            serde_json::from_str(&poll_metadata(&player, &state).unwrap()).unwrap();
        assert_eq!(metadata["timeline_clip_index"], clip_index, "{metadata}");
    }

    state.clear_timeline();
    assert!(
        stretches[0].exists(),
        "clearing the timeline removed its rendered stretch"
    );
}

#[test]
fn the_loader_refuses_build_options_that_do_not_match_the_timeline() {
    let directory = tempfile::tempdir().unwrap();
    let picture = directory.path().join("clip.mov");
    write_clip(&picture, 1);
    let card = write_card(directory.path());
    let plain = timeline_of(vec![clip(picture.clone(), 0, SECOND_CLIP_FRAMES)]);
    let overlaid = timeline_of(vec![
        clip(picture, 0, SECOND_CLIP_FRAMES),
        Clip {
            layer: 1,
            ..clip(card, 0, (0, HELD_FRAMES))
        },
    ]);
    let player = software_player();
    let state = overlay_state(PreviewOverlays::default());
    let work_dir = directory.path().join("work");

    let given_options = load_timeline(
        &player,
        &state,
        plain,
        Some(&fitted_to_the_clips()),
        &work_dir,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        given_options.contains("load_timeline_preview takes build options only"),
        "{given_options}"
    );
    let not_given =
        load_timeline(&player, &state, overlaid, None, &work_dir, &mut |_| {}).unwrap_err();
    assert!(
        not_given.contains("load_timeline_preview was given no build options"),
        "{not_given}"
    );
    assert_eq!(state.timeline_duration(), None);
}

const CANCELLED_HOLD_FRAMES: u64 = 48;

// holds the render on its first frame until `cancel` has run, then lets it see the cancel
fn assert_cancelled_during_its_render(
    player: &Player,
    state: &PreviewPlayer,
    directory: &Path,
    cancel: impl FnOnce(),
) {
    let card = write_card(directory);
    let timeline = timeline_of(vec![clip(card, 0, (0, CANCELLED_HOLD_FRAMES))]);
    let options = fitted_to_the_clips();
    let work_dir = directory.join("work");
    let (first_frame_rendered, first_frame) = mpsc::channel();
    let (cancel_done, cancel_ran) = mpsc::channel::<()>();
    std::thread::scope(|scope| {
        let options = &options;
        let work_dir = &work_dir;
        let loading = scope.spawn(move || {
            let mut progress: Vec<TimelineRenderProgress> = Vec::new();
            let loaded = load_timeline(
                player,
                state,
                timeline,
                Some(options),
                work_dir,
                &mut |reported| {
                    if progress.is_empty() {
                        first_frame_rendered.send(()).unwrap();
                        cancel_ran.recv().unwrap();
                    }
                    progress.push(reported);
                },
            );
            (loaded, progress)
        });
        if first_frame.recv().is_err() {
            panic!("no frame rendered: {:?}", loading.join().unwrap());
        }
        cancel();
        cancel_done.send(()).unwrap();
        let (loaded, progress) = loading.join().unwrap();
        assert_eq!(loaded, Err(TIMELINE_LOAD_CANCELLED.to_string()));
        let last = progress.last().unwrap();
        assert!(last.frames_rendered < last.total_frames, "{progress:?}");
    });
}

fn nothing_is_loaded(player: &Player, state: &PreviewPlayer) -> bool {
    state.timeline_duration().is_none() && player.mpv().get_property_string("path").is_err()
}

#[test]
fn stopping_the_preview_cancels_a_timeline_render() {
    let directory = tempfile::tempdir().unwrap();
    let player = software_player();
    let state = overlay_state(PreviewOverlays::default());
    assert_cancelled_during_its_render(&player, &state, directory.path(), || {
        // the test state has no surface, and the stop cancels before it asks for one
        assert_eq!(
            stop_preview(&state),
            Err("no surface in a test".to_string())
        );
    });
    assert!(nothing_is_loaded(&player, &state));
}

#[test]
fn the_cancel_command_cancels_a_timeline_render() {
    let directory = tempfile::tempdir().unwrap();
    let player = software_player();
    let state = overlay_state(PreviewOverlays::default());
    assert_cancelled_during_its_render(&player, &state, directory.path(), || {
        state.cancel_timeline_render()
    });
    assert!(nothing_is_loaded(&player, &state));
}

#[test]
fn a_newer_load_cancels_a_timeline_render_and_plays_once_it_has_stopped() {
    let directory = tempfile::tempdir().unwrap();
    let picture = directory.path().join("newer.mov");
    write_clip(&picture, 1);
    let newer = timeline_of(vec![clip(picture, 0, SECOND_CLIP_FRAMES)]);
    let work_dir = directory.path().join("work");
    let player = software_player();
    let state = overlay_state(PreviewOverlays::default());
    std::thread::scope(|scope| {
        let mut newer_load = None;
        assert_cancelled_during_its_render(&player, &state, directory.path(), || {
            let render_in_flight = state.timeline_render_cancel.lock().unwrap().clone();
            let (player, state, work_dir) = (&player, &state, &work_dir);
            newer_load =
                Some(scope.spawn(move || {
                    load_timeline(player, state, newer, None, work_dir, &mut |_| {})
                }));
            wait_until("the newer load's cancel", || {
                render_in_flight.load(Ordering::Relaxed)
            });
        });
        assert_eq!(newer_load.unwrap().join().unwrap(), Ok(None));
    });
    assert_eq!(state.timeline_duration(), Some(1.0));
}
