use std::path::Path;
use std::time::{Duration, Instant};

use super::end_of_file_tests::overlay_state;
use super::grok_fixture::{self, PICTURE_SIDE};
use super::{
    install_overlays, poll_metadata, Backend, Player, PreviewCrop, PreviewOverlays, PreviewPlayer,
};

// wide enough for a square picture to leave bars either side of it
const SURFACE_WIDTH: usize = 96;
const SURFACE_HEIGHT: usize = 64;
const BYTES_PER_PIXEL: usize = 4;
// the picture lands here at one surface pixel per source pixel
const PICTURE_LEFT: usize = (SURFACE_WIDTH - PICTURE_SIDE as usize) / 2;
const PICTURE_MIDDLE_ROW: usize = PICTURE_SIDE as usize / 2;

const PACKAGE_FRAMES: usize = 24;
const AT_THE_END: &str = r#""eof": true"#;
const PAUSED: &str = r#""paused": true"#;
const PLAYBACK_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

// pixels off each edge of the source picture
const CROP: PreviewCrop = PreviewCrop {
    left: 8,
    right: 8,
    top: 8,
    bottom: 8,
};
// how far the red channel has to clear the other two for a pixel to read as red
const RED_MARGIN: u8 = 40;
// what the nearest-neighbour scale may cost a flat colour, per channel
const COLOUR_TOLERANCE: u8 = 2;

fn playing_package(package: &Path) -> (Player, PreviewPlayer, Vec<u8>) {
    let player = Player::new().unwrap();
    player.init_software().unwrap();
    player.load_package_dir(&package.to_string_lossy()).unwrap();
    assert_eq!(
        player.active(),
        Backend::Grok,
        "a JPEG 2000 package is grok's"
    );
    let state = overlay_state(PreviewOverlays::default());
    let pixels = vec![0u8; SURFACE_WIDTH * SURFACE_HEIGHT * BYTES_PER_PIXEL];
    (player, state, pixels)
}

// pump frames the way the surface does until the polled metadata satisfies ready
fn pump_until(
    player: &Player,
    state: &PreviewPlayer,
    pixels: &mut [u8],
    mut ready: impl FnMut(&str) -> bool,
) -> Result<String, String> {
    let deadline = Instant::now() + PLAYBACK_TIMEOUT;
    let mut last = String::new();
    while Instant::now() < deadline {
        if player.wants_redraw() {
            player
                .render_software(SURFACE_WIDTH, SURFACE_HEIGHT, pixels)
                .unwrap();
        }
        last = poll_metadata(player, state).unwrap();
        if ready(&last) {
            return Ok(last);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    Err(last)
}

fn play_to_the_end(player: &Player, state: &PreviewPlayer, pixels: &mut [u8]) -> String {
    pump_until(player, state, pixels, |metadata| {
        metadata.contains(AT_THE_END)
    })
    .unwrap_or_else(|last| panic!("the package never reported eof, last read {last}"))
}

fn forget_composed_frames(player: &Player) {
    while player.wants_redraw() {}
}

fn render_the_next_frame(player: &Player, pixels: &mut [u8]) {
    let deadline = Instant::now() + PLAYBACK_TIMEOUT;
    while Instant::now() < deadline {
        if player.wants_redraw() {
            player
                .render_software(SURFACE_WIDTH, SURFACE_HEIGHT, pixels)
                .unwrap();
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!("no frame was composed within {PLAYBACK_TIMEOUT:?}");
}

fn frame_position(frame: usize) -> f64 {
    frame as f64 / f64::from(grok_fixture::FRAMES_PER_SECOND)
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 3] {
    let at = (y * SURFACE_WIDTH + x) * BYTES_PER_PIXEL;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

fn close_to(drawn: [u8; 3], expected: [u8; 3]) -> bool {
    drawn
        .iter()
        .zip(expected)
        .all(|(drawn, expected)| drawn.abs_diff(expected) <= COLOUR_TOLERANCE)
}

fn reads_as_red(drawn: [u8; 3]) -> bool {
    drawn[0] > drawn[1].saturating_add(RED_MARGIN) && drawn[0] > drawn[2].saturating_add(RED_MARGIN)
}

#[test]
fn a_j2k_package_plays_to_its_end_and_steps_back_a_frame() {
    let directory = tempfile::tempdir().unwrap();
    let package = grok_fixture::write_package(directory.path(), PACKAGE_FRAMES);
    let (player, state, mut pixels) = playing_package(&package);

    let metadata = play_to_the_end(&player, &state, &mut pixels);
    assert!(
        metadata.contains(PAUSED),
        "the end of the package did not pause it: {metadata}"
    );

    let ended_at = player.position().unwrap();
    assert_eq!(ended_at, frame_position(PACKAGE_FRAMES - 1));

    player.frame_back_step().unwrap();
    let metadata = pump_until(&player, &state, &mut pixels, |_| {
        player.position().unwrap_or(ended_at) < ended_at
    })
    .unwrap_or_else(|last| panic!("the step back never moved, last read {last}"));
    assert!(
        metadata.contains(PAUSED),
        "the step back left the package playing: {metadata}"
    );
    assert_eq!(
        player.position().unwrap(),
        frame_position(PACKAGE_FRAMES - 2),
        "the step back moved by something other than one frame"
    );
}

#[test]
fn a_crop_overlay_is_drawn_into_the_picture_the_grok_backend_composes() {
    let directory = tempfile::tempdir().unwrap();
    let package = grok_fixture::write_package(directory.path(), PACKAGE_FRAMES);
    let (player, state, mut pixels) = playing_package(&package);
    play_to_the_end(&player, &state, &mut pixels);

    let kept_centre = PICTURE_LEFT + PICTURE_SIDE as usize / 2;
    assert!(
        close_to(
            pixel(&pixels, kept_centre, PICTURE_MIDDLE_ROW),
            grok_fixture::body_colour()
        ),
        "the picture is not the fixture's colour before the crop goes on"
    );

    forget_composed_frames(&player);
    install_overlays(
        PreviewOverlays {
            crop: Some(CROP),
            crop_visible: true,
            ..Default::default()
        },
        &player,
        &state,
    )
    .unwrap();
    assert!(
        state.sent_rectangles.lock().unwrap().is_some(),
        "no rectangles reached the grok player"
    );
    render_the_next_frame(&player, &mut pixels);

    let in_the_left_band = PICTURE_LEFT + CROP.left as usize / 2;
    let band = pixel(&pixels, in_the_left_band, PICTURE_MIDDLE_ROW);
    assert!(reads_as_red(band), "the crop band is not red: {band:?}");
    let kept = pixel(&pixels, kept_centre, PICTURE_MIDDLE_ROW);
    assert!(
        close_to(kept, grok_fixture::body_colour()),
        "the crop reddened what it keeps: {kept:?}"
    );
    assert_eq!(
        pixel(&pixels, PICTURE_LEFT / 2, PICTURE_MIDDLE_ROW),
        [0, 0, 0],
        "the bar beside the picture is not black"
    );
}
