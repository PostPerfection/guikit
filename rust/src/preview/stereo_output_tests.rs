use super::grok_fixture::{self, wait_until, PICTURE_SIDE};
use super::player_controls::{set_stereo_output, StereoMode};
use super::{Backend, Player};

const PICTURE_FRAMES: usize = 24;
const SURFACE_SIDE: usize = PICTURE_SIDE as usize;
const BYTES_PER_PIXEL: usize = 4;
const QUARTER: usize = SURFACE_SIDE / 4;
const THREE_QUARTERS: usize = 3 * SURFACE_SIDE / 4;
const STEREOSCOPIC_FIELD: &str = "stereoscopic";

fn shown(player: &Player) -> Vec<u8> {
    let mut pixels = vec![0u8; SURFACE_SIDE * SURFACE_SIDE * BYTES_PER_PIXEL];
    player
        .render_software(SURFACE_SIDE, SURFACE_SIDE, &mut pixels)
        .unwrap();
    pixels
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 3] {
    let at = (y * SURFACE_SIDE + x) * BYTES_PER_PIXEL;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

// the paused frame is composed again for the new output
fn shown_as(player: &Player, mode: StereoMode) -> Vec<u8> {
    while player.wants_redraw() {}
    let outcome = set_stereo_output(player, mode);
    assert!(!outcome.mpv_file_unchanged);
    wait_until("the frame was composed again", || player.wants_redraw());
    shown(player)
}

#[test]
fn each_stereo_output_shows_the_eyes_where_it_says() {
    let directory = tempfile::tempdir().unwrap();
    let stereo = grok_fixture::write_stereo_picture_file(directory.path(), PICTURE_FRAMES);
    let (left, right) = (
        grok_fixture::left_eye_colour(),
        grok_fixture::right_eye_colour(),
    );

    let player = Player::new().unwrap();
    player.init_software().unwrap();
    player.load_source(&stereo.to_string_lossy(), None).unwrap();
    assert_eq!(player.active(), Backend::Grok);
    player.grok().set_paused(true);
    wait_until("the first frame was composed", || {
        player.grok().frame_size().is_some()
    });
    let metadata: serde_json::Value = serde_json::from_str(&player.grok().metadata_json()).unwrap();
    assert_eq!(metadata[STEREOSCOPIC_FIELD], true);
    let pixels = shown(&player);
    assert_eq!(
        [
            pixel(&pixels, QUARTER, QUARTER),
            pixel(&pixels, THREE_QUARTERS, THREE_QUARTERS)
        ],
        [left; 2],
        "the left eye fills the frame by default"
    );

    let pixels = shown_as(&player, StereoMode::RightEye);
    assert_eq!(
        [
            pixel(&pixels, QUARTER, QUARTER),
            pixel(&pixels, THREE_QUARTERS, THREE_QUARTERS)
        ],
        [right; 2]
    );

    let pixels = shown_as(&player, StereoMode::SideBySide);
    assert_eq!(
        [
            pixel(&pixels, QUARTER, QUARTER),
            pixel(&pixels, THREE_QUARTERS, QUARTER)
        ],
        [left, right],
        "the left eye on the left"
    );

    let pixels = shown_as(&player, StereoMode::TopAndBottom);
    assert_eq!(
        [
            pixel(&pixels, QUARTER, QUARTER),
            pixel(&pixels, QUARTER, THREE_QUARTERS)
        ],
        [left, right],
        "the left eye on top"
    );

    let pixels = shown_as(&player, StereoMode::LeftEye);
    assert_eq!(pixel(&pixels, THREE_QUARTERS, THREE_QUARTERS), left);
}
