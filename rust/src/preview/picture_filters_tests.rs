use std::time::{Duration, Instant};

use postkit::mpv_render::MpvRenderPlayer;

use super::end_of_file_tests::{overlay_state, write_clip};
use super::{
    apply_overlays, player_metadata, set_picture_filters, PreviewCrop, PreviewOverlays, SourceSize,
    PAUSE_PROPERTY,
};

const CLIP_SECONDS: u32 = 10;
const CLIP_SIZE: (f64, f64) = (320.0, 180.0);
const SURFACE_WIDTH: usize = 320;
const SURFACE_HEIGHT: usize = 180;
const BYTES_PER_PIXEL: usize = 4;
// a build that crops the clip, shrinks it and pads it onto a larger raster
const RASTER_SIZE: (f64, f64) = (640.0, 360.0);
const BUILD_FILTERS: [&str; 3] = [
    "crop=w=300:h=180:x=10:y=0",
    "scale=w=200:h=120:flags=lanczos",
    "pad=w=640:h=360:x=220:y=120:color=black",
];
const VIDEO_FILTERS_PROPERTY: &str = "vf";
const PLAYBACK_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

fn crop_only() -> PreviewOverlays {
    PreviewOverlays {
        crop: Some(PreviewCrop {
            left: 10,
            right: 10,
            top: 0,
            bottom: 0,
        }),
        crop_visible: true,
        ..Default::default()
    }
}

fn shown_frame_size(player: &MpvRenderPlayer) -> (Option<f64>, Option<f64>) {
    let metadata: serde_json::Value =
        serde_json::from_str(&player_metadata(player).unwrap()).unwrap();
    (
        metadata["shown_frame_width"].as_f64(),
        metadata["shown_frame_height"].as_f64(),
    )
}

// the software renderer only advances when its frames are taken
fn render_until(player: &MpvRenderPlayer, what: &str, mut ready: impl FnMut() -> bool) {
    let mut pixels = vec![0u8; SURFACE_WIDTH * SURFACE_HEIGHT * BYTES_PER_PIXEL];
    let deadline = Instant::now() + PLAYBACK_TIMEOUT;
    while Instant::now() < deadline {
        if player.wants_redraw() {
            player
                .render_software(SURFACE_WIDTH, SURFACE_HEIGHT, &mut pixels)
                .unwrap();
        }
        if ready() {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!(
        "{what} within {PLAYBACK_TIMEOUT:?}, last shown {:?}",
        shown_frame_size(player)
    );
}

fn wait_for_shown_size(player: &MpvRenderPlayer, (width, height): (f64, f64)) {
    render_until(
        player,
        &format!("mpv never showed a {width}x{height} frame"),
        || shown_frame_size(player) == (Some(width), Some(height)),
    );
}

fn build_filters() -> Vec<String> {
    BUILD_FILTERS
        .iter()
        .map(|filter| filter.to_string())
        .collect()
}

#[test]
fn the_build_filters_set_the_frame_the_overlays_measure_and_a_load_drops_them() {
    let directory = tempfile::tempdir().unwrap();
    let clip = directory.path().join("clip.mp4");
    write_clip(&clip, CLIP_SECONDS);
    let player = MpvRenderPlayer::new().unwrap();
    player.init_software().unwrap();
    player.load_file(&clip.to_string_lossy()).unwrap();
    let state = overlay_state(crop_only());

    wait_for_shown_size(&player, CLIP_SIZE);
    player.set_property(PAUSE_PROPERTY, "yes").unwrap();
    assert_eq!(
        state.source_size(&player),
        SourceSize::new(CLIP_SIZE.0, CLIP_SIZE.1)
    );
    apply_overlays(&player, &state).unwrap();
    assert!(
        state.drawn_overlay.lock().unwrap().is_some(),
        "the crop was not drawn"
    );

    set_picture_filters(&player, &state, Some(build_filters())).unwrap();
    wait_for_shown_size(&player, RASTER_SIZE);
    assert_eq!(
        state.source_size(&player),
        SourceSize::new(RASTER_SIZE.0, RASTER_SIZE.1),
        "the overlays are not measured against the filtered frame"
    );
    apply_overlays(&player, &state).unwrap();
    assert!(
        state.drawn_overlay.lock().unwrap().is_none(),
        "the source crop was drawn over a frame that has it taken off"
    );

    set_picture_filters(&player, &state, None).unwrap();
    assert_eq!(
        player.get_property_string(VIDEO_FILTERS_PROPERTY).unwrap(),
        ""
    );
    wait_for_shown_size(&player, CLIP_SIZE);
    assert_eq!(
        state.source_size(&player),
        SourceSize::new(CLIP_SIZE.0, CLIP_SIZE.1)
    );
    apply_overlays(&player, &state).unwrap();
    assert!(
        state.drawn_overlay.lock().unwrap().is_some(),
        "the crop did not come back"
    );

    set_picture_filters(&player, &state, Some(build_filters())).unwrap();
    assert_ne!(
        player.get_property_string(VIDEO_FILTERS_PROPERTY).unwrap(),
        ""
    );
    state.forget_loaded_file(&player).unwrap();
    assert_eq!(
        player.get_property_string(VIDEO_FILTERS_PROPERTY).unwrap(),
        "",
        "the filters outlived the source they were set for"
    );
    assert!(state.picture_filters.lock().unwrap().is_none());
}
