//! The end-of-file flag the page's playlist advances on, read off a real player.
//!
//! A two-reel package written by postkit's ASSETMAP and CPL writers around
//! ffmpeg clips is played to its end through libmpv's software renderer, which
//! needs no display. Skips when ffmpeg is absent.

use std::path::Path;
use std::time::{Duration, Instant};

use postkit::mpv_render::MpvRenderPlayer;
use postkit::packaging::{ns, AssetMap, AssetMapAsset, DcpCpl, DcpCplReel};

use super::{
    apply_overlays, player_metadata, PreviewCrop, PreviewOverlays, PreviewPlayer, PreviewSurface,
};

/// Picture file name, asset uuid and clip length in seconds, per reel.
const REELS: [(&str, &str, u32); 2] = [
    ("reel1.mxf", "11111111-1111-1111-1111-111111111111", 1),
    ("reel2.mxf", "22222222-2222-2222-2222-222222222222", 1),
];
const FRAMES_PER_SECOND: u32 = 24;
const CLIP_WIDTH: usize = 320;
const CLIP_HEIGHT: usize = 180;
const BYTES_PER_PIXEL: usize = 4;
/// The metadata poll's flag, either way round.
const AT_THE_END: &str = r#""eof": true"#;
const NOT_AT_THE_END: &str = r#""eof": false"#;
/// How far off the duration the position may sit once playback stops, which
/// leaves room for a frame either way.
const POSITION_TOLERANCE_SECONDS: f64 = 0.2;
/// How far a resumed package has to get for playback to have restarted.
const RESUMED_POSITION_SECONDS: f64 = 0.1;
const PLAYBACK_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(super) fn have_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn write_clip(path: &Path, seconds: u32) {
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!(
                "testsrc=size={CLIP_WIDTH}x{CLIP_HEIGHT}:rate={FRAMES_PER_SECOND}:duration={seconds}"
            ),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .expect("ffmpeg");
    assert!(
        out.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A package with a real ASSETMAP and CPL, clips standing in for the picture
/// track files.
fn write_package(dir: &Path) {
    let mut assets = vec![AssetMapAsset {
        id: "cc10cc10-0000-0000-0000-000000000000".into(),
        path: "CPL_test.xml".into(),
        ..Default::default()
    }];
    let mut reels = Vec::new();
    for (index, (name, picture_id, seconds)) in REELS.iter().enumerate() {
        write_clip(&dir.join(name), *seconds);
        assets.push(AssetMapAsset {
            id: (*picture_id).into(),
            path: (*name).into(),
            ..Default::default()
        });
        reels.push(DcpCplReel {
            reel_id: format!("aaaaaaaa-0000-0000-0000-00000000000{index}"),
            picture_id: (*picture_id).into(),
            picture_edit_rate_num: FRAMES_PER_SECOND,
            picture_edit_rate_den: 1,
            picture_duration: u64::from(seconds * FRAMES_PER_SECOND),
            picture_width: 1998,
            picture_height: 1080,
            ..Default::default()
        });
    }

    let assetmap = AssetMap {
        uuid: "bbbbbbbb-0000-0000-0000-000000000000".into(),
        namespace: ns::AM_SMPTE.into(),
        assets,
        ..Default::default()
    };
    std::fs::write(dir.join("ASSETMAP.xml"), assetmap.to_xml()).unwrap();

    let cpl = DcpCpl {
        uuid: "cc10cc10-0000-0000-0000-000000000000".into(),
        namespace: ns::CPL_SMPTE.into(),
        title: "Playlist End Test".into(),
        reels,
        ..Default::default()
    };
    std::fs::write(dir.join("CPL_test.xml"), cpl.to_xml()).unwrap();
}

fn loaded_player(dir: &Path) -> MpvRenderPlayer {
    let player = MpvRenderPlayer::new().unwrap();
    player.init_software().unwrap();
    player.load_package_dir(&dir.to_string_lossy()).unwrap();
    player
}

/// Take the frames mpv offers, which is the only thing that makes playback
/// advance under the software renderer, until `ready` holds for the metadata the
/// page polls.
fn pump_until(
    player: &MpvRenderPlayer,
    mut ready: impl FnMut(&str) -> bool,
) -> Result<String, String> {
    let mut pixels = vec![0u8; CLIP_WIDTH * CLIP_HEIGHT * BYTES_PER_PIXEL];
    let deadline = Instant::now() + PLAYBACK_TIMEOUT;
    let mut last = String::new();
    while Instant::now() < deadline {
        if player.wants_redraw() {
            player
                .render_software(CLIP_WIDTH, CLIP_HEIGHT, &mut pixels)
                .unwrap();
        }
        last = player_metadata(player).unwrap();
        if ready(&last) {
            return Ok(last);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    Err(last)
}

/// Play the loaded package to its end and hand back the metadata read there.
fn play_to_the_end(player: &MpvRenderPlayer) -> String {
    let mut played_with_the_flag_clear = false;
    let metadata = pump_until(player, |metadata| {
        if metadata.contains(NOT_AT_THE_END) && player.get_position().unwrap_or(0.0) > 0.0 {
            played_with_the_flag_clear = true;
        }
        metadata.contains(AT_THE_END)
    });
    assert!(
        played_with_the_flag_clear,
        "the flag never read false while the package was playing"
    );
    metadata.unwrap_or_else(|last| panic!("the package never reported eof, last read {last}"))
}

#[test]
fn a_package_played_to_its_end_reports_eof_in_the_metadata() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write_package(dir.path());
    let player = loaded_player(dir.path());

    // keep-open is what leaves mpv sitting on the last frame with the flag set,
    // rather than closing the file and reporting nothing
    let metadata = play_to_the_end(&player);
    assert!(
        metadata.contains(r#""paused": true"#),
        "mpv did not pause at the end: {metadata}"
    );
    let position = player.get_position().unwrap();
    let duration = player.get_duration().unwrap();
    assert!(
        (position - duration).abs() < POSITION_TOLERANCE_SECONDS,
        "stopped at {position}s of a {duration}s package"
    );
}

/// Every overlay at once, which is the most a page can ask to have drawn.
fn every_overlay() -> PreviewOverlays {
    PreviewOverlays {
        safe_area_percent: Some(95),
        aspect_mask: Some(2.39),
        centre_cross: true,
        thirds_grid: true,
        crop: Some(PreviewCrop {
            left: 100,
            right: 100,
            top: 20,
            bottom: 20,
        }),
        crop_visible: true,
    }
}

/// The state the commands keep, with no surface behind it: the overlays are put
/// on the player handed to `apply_overlays`, which is the player the test drives.
fn overlay_state(overlays: PreviewOverlays) -> PreviewPlayer {
    let state = PreviewPlayer::new(PreviewSurface::Unavailable(
        "no surface in a test".to_string(),
    ));
    *state.overlays.lock().unwrap() = overlays;
    state
}

/// Overlays going on while a package plays and coming off at the end of it, which
/// is where the page's playlist is deciding whether to advance. A `vf` change here
/// cleared mpv's `eof-reached` for good, which is why the overlays are drawn on
/// the OSD instead; nothing about the drawing may reach the flag or stall
/// playback. The rule about the render thread is held in `render_thread_tests`.
#[test]
fn overlays_drawn_over_a_package_leave_the_end_of_file_flag_alone() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write_package(dir.path());
    let player = loaded_player(dir.path());
    let state = overlay_state(every_overlay());

    // playing, so a frame has been rendered and mpv knows where the picture is
    pump_until(&player, |_| player.get_position().unwrap_or(0.0) > 0.0)
        .unwrap_or_else(|last| panic!("the package never started playing, last read {last}"));
    apply_overlays(&player, &state).unwrap();
    assert!(
        state.drawn_overlay.lock().unwrap().is_some(),
        "no overlay was drawn over the playing package"
    );

    let metadata = play_to_the_end(&player);
    assert!(
        metadata.contains(AT_THE_END),
        "the package did not report eof with the overlays on: {metadata}"
    );

    // the page sends the overlays again on every poll, which must not disturb
    // anything either
    apply_overlays(&player, &state).unwrap();
    assert!(
        player_metadata(&player).unwrap().contains(AT_THE_END),
        "drawing the overlays again at the end cleared the flag"
    );

    *state.overlays.lock().unwrap() = PreviewOverlays::default();
    apply_overlays(&player, &state).unwrap();
    assert!(
        state.drawn_overlay.lock().unwrap().is_none(),
        "the overlay stayed on the player after every overlay went off"
    );
    let metadata = player_metadata(&player).unwrap();
    assert!(
        metadata.contains(AT_THE_END),
        "clearing the overlays at the end cleared the flag: {metadata}"
    );
}

/// What the page's playlist does at the end of a row: load the next package and
/// send one play/pause, because the pause mpv took at the end outlives the load.
#[test]
fn the_package_loaded_at_the_end_starts_on_one_play_pause() {
    if !have_ffmpeg() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write_package(dir.path());
    let player = loaded_player(dir.path());
    play_to_the_end(&player);

    player
        .load_package_dir(&dir.path().to_string_lossy())
        .unwrap();
    let metadata = pump_until(&player, |metadata| metadata.contains(NOT_AT_THE_END))
        .unwrap_or_else(|last| panic!("the flag never cleared after the load, last read {last}"));
    assert!(
        metadata.contains(r#""paused": true"#),
        "the load did not stay paused: {metadata}"
    );

    player.play_pause().unwrap();
    pump_until(&player, |_| {
        player.get_position().unwrap_or(0.0) > RESUMED_POSITION_SECONDS
    })
    .unwrap_or_else(|last| panic!("play/pause did not restart playback, last read {last}"));
}
