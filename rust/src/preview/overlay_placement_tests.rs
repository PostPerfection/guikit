//! Where the overlays land, read off the pixels a real libmpv renders.
//!
//! mpv stretches an overlay's canvas over the whole surface, so a drawing meant
//! for the picture has to be placed on it. A clip is rendered into a square
//! surface through the software renderer, which leaves the picture between bars
//! the way the preview panel does, and the drawn pixels are looked for on the
//! picture.

use std::time::{Duration, Instant};

use postkit::mpv_render::{MpvRenderPlayer, OsdAssOverlay};

use super::overlays::{overlay_drawing, OsdRectangle, PreviewOverlays, SourceSize};
use super::{read_osd_rectangle, QC_OVERLAY_ID};

/// A square surface for a 16:9 clip, so mpv has bars to leave.
const SURFACE_SIDE: usize = 400;
const BYTES_PER_PIXEL: usize = 4;
const CLIP_WIDTH: u32 = 320;
const CLIP_HEIGHT: u32 = 180;

/// How light a pixel counts as drawn on, out of 255. The overlays are drawn over
/// a black clip, so anything lit at all is the drawing.
const DRAWN_ON: u8 = 60;
/// How far off the picture's edge the outermost drawn pixel may sit, in surface
/// pixels: the canvas is scaled by a fraction, and libass antialiases the edge it
/// lands between.
const EDGE_TOLERANCE: usize = 2;

const FRAME_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

fn write_black_clip(path: &std::path::Path) {
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=black:size={CLIP_WIDTH}x{CLIP_HEIGHT}:rate=24:duration=2"),
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

struct Surface {
    player: MpvRenderPlayer,
    pixels: Vec<u8>,
}

impl Surface {
    fn playing(clip: &std::path::Path) -> Self {
        let player = MpvRenderPlayer::new().unwrap();
        player.init_software().unwrap();
        player.load_file(&clip.to_string_lossy()).unwrap();
        Surface {
            player,
            pixels: vec![0u8; SURFACE_SIDE * SURFACE_SIDE * BYTES_PER_PIXEL],
        }
    }

    /// An overlay change reaches the surface a frame or two later, so this takes
    /// several rather than one.
    fn render_frames(&mut self, count: usize) {
        let deadline = Instant::now() + FRAME_TIMEOUT;
        let mut drawn = 0;
        while Instant::now() < deadline && drawn < count {
            if self.player.wants_redraw() {
                self.player
                    .render_software(SURFACE_SIDE, SURFACE_SIDE, &mut self.pixels)
                    .unwrap();
                drawn += 1;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        assert_eq!(drawn, count, "libmpv offered fewer frames than {count}");
    }

    /// Draw the overlays for a picture of the clip's size, placed on `osd`, and
    /// hand back the first and last row and column that came out lit.
    fn drawn_edges(
        &mut self,
        overlays: &PreviewOverlays,
        osd: Option<OsdRectangle>,
    ) -> ((usize, usize), (usize, usize)) {
        let source = SourceSize::new(f64::from(CLIP_WIDTH), f64::from(CLIP_HEIGHT));
        let drawing = overlay_drawing(overlays, source, osd).expect("something to draw");
        self.player
            .set_osd_overlay(
                QC_OVERLAY_ID,
                Some(OsdAssOverlay {
                    events: &drawing.events,
                    play_res_x: drawing.play_res_x,
                    play_res_y: drawing.play_res_y,
                }),
            )
            .unwrap();
        self.render_frames(3);

        let mut rows: Option<(usize, usize)> = None;
        let mut columns: Option<(usize, usize)> = None;
        for row in 0..SURFACE_SIDE {
            for column in 0..SURFACE_SIDE {
                let at = (row * SURFACE_SIDE + column) * BYTES_PER_PIXEL;
                if self.pixels[at..at + 3]
                    .iter()
                    .all(|value| *value <= DRAWN_ON)
                {
                    continue;
                }
                rows = Some(match rows {
                    None => (row, row),
                    Some((first, _)) => (first, row),
                });
                columns = Some(match columns {
                    None => (column, column),
                    Some((first, last)) => (first.min(column), last.max(column)),
                });
            }
        }
        (rows.expect("no row was drawn on"), columns.unwrap())
    }
}

/// The outline of the whole picture, which is the safe area at its widest, so the
/// drawn edges are the picture's edges.
fn picture_outline() -> PreviewOverlays {
    PreviewOverlays {
        safe_area_percent: Some(100),
        ..Default::default()
    }
}

fn close_enough(drawn: usize, expected: f64, what: &str) {
    let off = (drawn as f64 - expected).abs();
    assert!(
        off <= EDGE_TOLERANCE as f64,
        "{what}: drawn at {drawn}, the picture is at {expected}"
    );
}

#[test]
fn the_drawing_lands_on_the_picture_rather_than_the_whole_surface() {
    let directory = tempfile::tempdir().unwrap();
    let clip = directory.path().join("black.mp4");
    write_black_clip(&clip);
    let mut surface = Surface::playing(&clip);
    surface.render_frames(3);

    let osd = read_osd_rectangle(&surface.player).expect("mpv reports where it drew the picture");
    assert!(
        osd.margin_top > 0.0 || osd.margin_left > 0.0,
        "mpv left no bars around the picture, so this proves nothing"
    );

    let (rows, columns) = surface.drawn_edges(&picture_outline(), Some(osd));
    close_enough(rows.0, osd.margin_top, "the top of the outline");
    close_enough(rows.1, osd.height - osd.margin_bottom - 1.0, "the bottom");
    close_enough(columns.0, osd.margin_left, "the left");
    close_enough(columns.1, osd.width - osd.margin_right - 1.0, "the right");

    // and the same drawing with nothing said about where the picture is, which is
    // what the placement is there to stop: mpv stretches it over the bars too
    let (rows, columns) = surface.drawn_edges(&picture_outline(), None);
    assert_eq!(
        (rows, columns),
        ((0, SURFACE_SIDE - 1), (0, SURFACE_SIDE - 1)),
        "the unplaced drawing did not cover the whole surface"
    );
}
