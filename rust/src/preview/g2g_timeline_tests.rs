use std::time::Instant;

use postkit::timeline::SoundTrack;

use super::end_of_file_tests::overlay_state;
use super::grok_fixture::wait_until;
use super::mpv_timeline_tests::{
    fitted_to_the_clips, gapped_clips, held_still_then_clip, reports_paused, tone, two_clips,
    FRAME_SECONDS, GAPPED_TIMELINE_SECONDS, HELD_STILL_TIMELINE_SECONDS,
    INTO_THE_CLIP_AFTER_SECONDS, INTO_THE_GAP_SECONDS, INTO_THE_SECOND_CLIP_SECONDS,
    INTO_THE_STILL_SECONDS, PAUSED_WATCH, TIMELINE_SECONDS,
};
use super::{
    load_timeline, poll_metadata, Backend, PlayState, Player, PreviewOverlays, PreviewPlayer,
    TIMELINE_BLACK_FILE,
};

fn metadata(player: &Player, state: &PreviewPlayer) -> serde_json::Value {
    serde_json::from_str(&poll_metadata(player, state).unwrap()).unwrap()
}

fn duration_once_read(player: &Player) -> f64 {
    wait_until("g2g reading the timeline's duration", || {
        player.duration().is_ok()
    });
    player.duration().unwrap()
}

fn play_past(player: &Player, seconds: f64) {
    wait_until(&format!("the timeline playing past {seconds}s"), || {
        player.position().unwrap_or(0.0) > seconds
    });
}

#[test]
fn a_g2g_timeline_lasts_as_long_as_its_clips_and_reports_the_clip_it_plays() {
    let directory = tempfile::tempdir().unwrap();
    let timeline = two_clips(
        directory.path(),
        SoundTrack::Override(tone(directory.path())),
    );
    let player = Player::new_on_g2g().unwrap();
    let state = overlay_state(PreviewOverlays::default());

    load_timeline(
        &player,
        &state,
        timeline,
        None,
        PlayState::Playing,
        directory.path(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(player.active(), Backend::G2g);
    let duration = duration_once_read(&player);
    assert!(
        (duration - TIMELINE_SECONDS).abs() < FRAME_SECONDS,
        "g2g plays the timeline for {duration}s"
    );
    assert_eq!(state.timeline_duration(), Some(TIMELINE_SECONDS));

    play_past(&player, INTO_THE_SECOND_CLIP_SECONDS);
    let metadata = metadata(&player, &state);
    assert_eq!(metadata["timeline_clip_index"], 1, "{metadata}");
}

#[test]
fn a_g2g_timeline_plays_black_through_a_gap_and_reports_no_clip_there() {
    let directory = tempfile::tempdir().unwrap();
    let timeline = gapped_clips(directory.path());
    let work_dir = directory.path().join("work");
    let black_file = work_dir.join(TIMELINE_BLACK_FILE);
    let player = Player::new_on_g2g().unwrap();
    let state = overlay_state(PreviewOverlays::default());

    load_timeline(
        &player,
        &state,
        timeline,
        None,
        PlayState::Playing,
        &work_dir,
        &mut |_| {},
    )
    .unwrap();
    let duration = duration_once_read(&player);
    assert!(
        (duration - GAPPED_TIMELINE_SECONDS).abs() < FRAME_SECONDS,
        "g2g plays the timeline for {duration}s"
    );

    play_past(&player, INTO_THE_GAP_SECONDS);
    let metadata = metadata(&player, &state);
    assert!(metadata["timeline_clip_index"].is_null(), "{metadata}");

    state.clear_timeline();
    assert!(
        !black_file.exists(),
        "{} is left behind",
        black_file.display()
    );
}

#[test]
fn a_timeline_loaded_paused_on_g2g_stays_on_its_first_frame() {
    let directory = tempfile::tempdir().unwrap();
    let timeline = two_clips(directory.path(), SoundTrack::PerClip);
    let player = Player::new_on_g2g().unwrap();
    let state = overlay_state(PreviewOverlays::default());

    load_timeline(
        &player,
        &state,
        timeline,
        None,
        PlayState::Paused,
        &directory.path().join("work"),
        &mut |_| {},
    )
    .unwrap();
    duration_once_read(&player);
    let watched = Instant::now();
    let mut furthest: f64 = 0.0;
    wait_until("the paused watch ending", || {
        furthest = furthest.max(player.position().unwrap_or(0.0));
        watched.elapsed() > PAUSED_WATCH
    });
    assert_eq!(furthest, 0.0);
    assert!(reports_paused(&player, &state));
}

#[test]
fn a_held_still_on_layer_0_renders_and_plays_on_g2g_before_the_clip_after_it() {
    let directory = tempfile::tempdir().unwrap();
    let timeline = held_still_then_clip(directory.path());
    let player = Player::new_on_g2g().unwrap();
    let state = overlay_state(PreviewOverlays::default());

    let loaded = load_timeline(
        &player,
        &state,
        timeline,
        Some(&fitted_to_the_clips()),
        PlayState::Playing,
        &directory.path().join("work"),
        &mut |_| {},
    );
    assert_eq!(loaded, Ok(None));
    let duration = duration_once_read(&player);
    assert!(
        (duration - HELD_STILL_TIMELINE_SECONDS).abs() < FRAME_SECONDS,
        "g2g plays the timeline for {duration}s"
    );
    for (seconds, clip_index) in [
        (INTO_THE_STILL_SECONDS, 0),
        (INTO_THE_CLIP_AFTER_SECONDS, 1),
    ] {
        play_past(&player, seconds);
        let metadata = metadata(&player, &state);
        assert_eq!(metadata["timeline_clip_index"], clip_index, "{metadata}");
    }
}
