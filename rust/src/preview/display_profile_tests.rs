use lcms2::{CIExyY, CIExyYTRIPLE, Profile, ToneCurve};
use std::path::{Path, PathBuf};

use super::grok_fixture::{self, wait_until, BAND_ROWS, PICTURE_SIDE};
use super::player_controls::set_display_profile;
use super::{Backend, Player};

const PACKAGE_FRAMES: usize = 24;
const SURFACE_SIDE: usize = PICTURE_SIDE as usize;
const BYTES_PER_PIXEL: usize = 4;
const BAND_ROW: usize = BAND_ROWS as usize / 2;
const MIDDLE_COLUMN: usize = SURFACE_SIDE / 2;
// a linear sRGB monitor shows the light the built-in sRGB encodes, so its codes are that light undone
const LINEAR_GAMMA: f64 = 1.0;
const D65_WHITE: (f64, f64) = (0.3127, 0.3290);
const SRGB_PRIMARIES: [(f64, f64); 3] = [(0.640, 0.330), (0.300, 0.600), (0.150, 0.060)];
const MAXIMUM_CODE: f64 = 255.0;
const SRGB_LINEAR_LIMIT: f64 = 0.04045;
const SRGB_LINEAR_SLOPE: f64 = 12.92;
const SRGB_OFFSET: f64 = 0.055;
const SRGB_GAMMA: f64 = 2.4;
// the 8-bit built-in colour the expectation starts from rounds by half a code
const LINEAR_TOLERANCE: u8 = 2;

fn linear_srgb_profile(directory: &Path) -> PathBuf {
    let chromaticity = |(x, y): (f64, f64)| CIExyY { x, y, Y: 1.0 };
    let curve = ToneCurve::new(LINEAR_GAMMA);
    let profile = Profile::new_rgb(
        &chromaticity(D65_WHITE),
        &CIExyYTRIPLE {
            Red: chromaticity(SRGB_PRIMARIES[0]),
            Green: chromaticity(SRGB_PRIMARIES[1]),
            Blue: chromaticity(SRGB_PRIMARIES[2]),
        },
        &[&curve, &curve, &curve],
    )
    .unwrap();
    let path = directory.join("linear-srgb.icc");
    std::fs::write(&path, profile.icc().unwrap()).unwrap();
    path
}

fn srgb_to_linear(code: u8) -> u8 {
    let encoded = f64::from(code) / MAXIMUM_CODE;
    let linear = if encoded <= SRGB_LINEAR_LIMIT {
        encoded / SRGB_LINEAR_SLOPE
    } else {
        ((encoded + SRGB_OFFSET) / (1.0 + SRGB_OFFSET)).powf(SRGB_GAMMA)
    };
    (linear * MAXIMUM_CODE).round() as u8
}

fn band_shown(player: &Player) -> [u8; 3] {
    let mut pixels = vec![0u8; SURFACE_SIDE * SURFACE_SIDE * BYTES_PER_PIXEL];
    player
        .render_software(SURFACE_SIDE, SURFACE_SIDE, &mut pixels)
        .unwrap();
    let at = (BAND_ROW * SURFACE_SIDE + MIDDLE_COLUMN) * BYTES_PER_PIXEL;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

// the paused frame is composed again through the new profile
fn band_shown_through(player: &Player, profile: Option<&Path>) -> [u8; 3] {
    while player.wants_redraw() {}
    let outcome = set_display_profile(player, profile).unwrap();
    assert!(!outcome.mpv_file_unchanged);
    wait_until("the frame was composed again", || player.wants_redraw());
    band_shown(player)
}

fn assert_near(shown: [u8; 3], expected: [u8; 3], tolerance: u8, what: &str) {
    let off = (0..3)
        .map(|channel| shown[channel].abs_diff(expected[channel]))
        .max()
        .unwrap();
    assert!(
        off <= tolerance,
        "{what}: shown {shown:?}, expected {expected:?}"
    );
}

#[test]
fn a_monitor_profile_recolours_the_frame_and_a_refused_one_names_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let package = grok_fixture::write_package(directory.path(), PACKAGE_FRAMES);
    let linear = linear_srgb_profile(directory.path());
    let garbage = directory.path().join("garbage.icc");
    std::fs::write(&garbage, b"not an icc profile").unwrap();
    let built_in = grok_fixture::band_colour();

    let player = Player::new().unwrap();
    player.init_software().unwrap();
    player
        .load_package_dir(&package.to_string_lossy(), None)
        .unwrap();
    assert_eq!(player.active(), Backend::Grok);
    player.grok().set_paused(true);
    wait_until("the first frame was composed", || {
        player.grok().frame_size().is_some()
    });
    assert_eq!(band_shown(&player), built_in);

    assert_near(
        band_shown_through(&player, Some(&linear)),
        built_in.map(srgb_to_linear),
        LINEAR_TOLERANCE,
        "the band through a linear sRGB monitor",
    );

    let error = set_display_profile(&player, Some(&garbage)).unwrap_err();
    assert!(error.contains(&garbage.display().to_string()), "{error}");

    assert_eq!(band_shown_through(&player, None), built_in);
}
