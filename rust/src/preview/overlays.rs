//! The QC overlays drawn over playback, as ASS drawings on mpv's OSD.
//!
//! The requested overlays become filled rectangles in the source picture's own
//! pixels. A renderer that composites frames itself takes those rectangles, and
//! mpv takes them as one ASS dialogue event per overlay. mpv stretches an
//! overlay's canvas over the whole rendered surface, black bars included, so the
//! canvas is sized and the drawing shifted to land on the picture instead, which
//! is what `PicturePlacement` works out.

use serde::Deserialize;

/// Fill colour and alpha as ASS writes them: the colour is BBGGRR, and the alpha
/// counts down from FF for invisible to 00 for solid.
#[derive(Clone, Copy)]
struct OverlayInk {
    colour: &'static str,
    alpha: &'static str,
}

const WHITE: &str = "FFFFFF";
const BLACK: &str = "000000";
const RED: &str = "0000FF";

const SAFE_AREA_INK: OverlayInk = OverlayInk {
    colour: WHITE,
    alpha: "33",
};
const ASPECT_MASK_INK: OverlayInk = OverlayInk {
    colour: BLACK,
    alpha: "66",
};
const CENTRE_CROSS_INK: OverlayInk = OverlayInk {
    colour: WHITE,
    alpha: "33",
};
const THIRDS_GRID_INK: OverlayInk = OverlayInk {
    colour: WHITE,
    alpha: "99",
};
const CROP_BAND_INK: OverlayInk = OverlayInk {
    colour: RED,
    alpha: "A6",
};
const CROP_OUTLINE_INK: OverlayInk = OverlayInk {
    colour: RED,
    alpha: "1A",
};

// pixels of the source picture at full resolution, not of the rendered surface
pub use postkit::grok_player::OverlayRectangle;

impl OverlayInk {
    // ass writes the colour BBGGRR and the alpha as transparency
    fn filled(self, x: i64, y: i64, width: i64, height: i64) -> OverlayRectangle {
        OverlayRectangle {
            x,
            y,
            width,
            height,
            colour: [
                hexadecimal_pair(self.colour, RED_DIGITS_AT),
                hexadecimal_pair(self.colour, GREEN_DIGITS_AT),
                hexadecimal_pair(self.colour, BLUE_DIGITS_AT),
            ],
            alpha: u8::MAX - hexadecimal_pair(self.alpha, 0),
        }
    }
}

const BLUE_DIGITS_AT: usize = 0;
const GREEN_DIGITS_AT: usize = 2;
const RED_DIGITS_AT: usize = 4;

fn hexadecimal_pair(text: &str, at: usize) -> u8 {
    let digits = text.as_bytes();
    hexadecimal_digit(digits[at]) * 16 + hexadecimal_digit(digits[at + 1])
}

fn hexadecimal_digit(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'A'..=b'F' => digit - b'A' + 10,
        _ => panic!("overlay ink is written as uppercase hexadecimal"),
    }
}

/// Line widths in source pixels, so a line is as thick against the picture as it
/// was when the same overlays were drawn by a video filter.
const SAFE_AREA_THICKNESS: i64 = 2;
const CENTRE_CROSS_THICKNESS: i64 = 2;
const THIRDS_GRID_THICKNESS: i64 = 1;
const CROP_OUTLINE_THICKNESS: i64 = 2;

/// A mask band thinner than this is the target aspect meeting the picture's own,
/// which wants no band at all.
const SMALLEST_MASK_BAND_PIXELS: f64 = 1.0;

/// The picture size the container declares, which every drawing is measured
/// against. Built only by reading it off the player, so it is never zero.
#[derive(Clone, Copy)]
pub struct SourceSize {
    width: f64,
    height: f64,
}

impl SourceSize {
    pub fn new(width: f64, height: f64) -> Option<Self> {
        if width > 0.0 && height > 0.0 {
            Some(SourceSize { width, height })
        } else {
            None
        }
    }

    pub fn scaled_by(self, factor: f64) -> Option<Self> {
        SourceSize::new(self.width * factor, self.height * factor)
    }
}

/// Where mpv put the picture on the surface it last rendered: the size of that
/// surface and the bars left around the picture, mpv's `osd-dimensions`.
#[derive(Clone, Copy)]
pub struct OsdRectangle {
    pub width: f64,
    pub height: f64,
    pub margin_left: f64,
    pub margin_top: f64,
    pub margin_right: f64,
    pub margin_bottom: f64,
}

/// What the page asked to see. All off is the default, and draws nothing.
#[derive(Default, Deserialize)]
pub struct PreviewOverlays {
    pub safe_area_percent: Option<u8>,
    pub aspect_mask: Option<f64>,
    pub centre_cross: bool,
    pub thirds_grid: bool,
    pub crop: Option<PreviewCrop>,
    pub crop_visible: bool,
}

impl PreviewOverlays {
    /// Whether anything at all is switched on, which is what saves reading the
    /// picture's size and place on every metadata poll.
    pub fn any(&self) -> bool {
        self.safe_area_percent.is_some()
            || self.aspect_mask.is_some()
            || self.centre_cross
            || self.thirds_grid
            || (self.crop.is_some() && self.crop_visible)
    }
}

/// What the job's crop takes off each edge, in source pixels.
#[derive(Clone, Copy, Deserialize)]
pub struct PreviewCrop {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

/// An ASS overlay ready for mpv: the events to draw and the canvas their
/// coordinates are in.
#[derive(Clone, PartialEq)]
pub struct OverlayDrawing {
    pub events: String,
    pub play_res_x: u32,
    pub play_res_y: u32,
}

/// The overlay for what the page asked for, or none when nothing is on, nothing
/// reports a picture size, or the crop is the only thing on and there is none.
pub fn overlay_drawing(
    overlays: &PreviewOverlays,
    source: Option<SourceSize>,
    osd: Option<OsdRectangle>,
) -> Option<OverlayDrawing> {
    let source = source?;
    let events = overlay_events(overlays, source);
    if events.is_empty() {
        return None;
    }
    let placement = picture_placement(source, osd);
    let drawn: Vec<String> = events
        .iter()
        .map(|event| drawing_event(event, &placement))
        .collect();
    Some(OverlayDrawing {
        events: drawn.join("\n"),
        play_res_x: placement.play_res_x,
        play_res_y: placement.play_res_y,
    })
}

// in the order the ass events draw them
pub fn overlay_rectangles(overlays: &PreviewOverlays, source: SourceSize) -> Vec<OverlayRectangle> {
    overlay_events(overlays, source)
        .into_iter()
        .flat_map(|event| event.rectangles)
        .collect()
}

struct OverlayEvent {
    ink: OverlayInk,
    rectangles: Vec<OverlayRectangle>,
}

fn overlay_events(overlays: &PreviewOverlays, source: SourceSize) -> Vec<OverlayEvent> {
    let mut events: Vec<OverlayEvent> = Vec::new();
    // the mask is a fill, so it draws first or it covers the lines
    if let Some(aspect) = overlays.aspect_mask {
        events.extend(aspect_mask_event(aspect, source));
    }
    if let Some(crop) = overlays.crop.filter(|_| overlays.crop_visible) {
        events.extend(crop_events(crop, source));
    }
    if let Some(percent) = overlays.safe_area_percent {
        events.push(safe_area_event(percent, source));
    }
    if overlays.centre_cross {
        events.push(centre_cross_event(source));
    }
    if overlays.thirds_grid {
        events.push(thirds_grid_event(source));
    }
    events
}

/// The canvas a drawing in source pixels needs, and where the picture's top left
/// corner sits on it. mpv stretches the canvas over the whole surface, so the
/// canvas is the surface's shape written in source pixels, and the drawing is
/// shifted by the bars mpv left around the picture.
struct PicturePlacement {
    play_res_x: u32,
    play_res_y: u32,
    offset_x: i64,
    offset_y: i64,
}

fn picture_placement(source: SourceSize, osd: Option<OsdRectangle>) -> PicturePlacement {
    let whole_surface = PicturePlacement {
        play_res_x: rounded(source.width) as u32,
        play_res_y: rounded(source.height) as u32,
        offset_x: 0,
        offset_y: 0,
    };
    // mpv reports no dimensions until it has rendered a frame, and none of this
    // holds for a surface or a picture with no size
    let Some(osd) = osd else {
        return whole_surface;
    };
    let picture_width = osd.width - osd.margin_left - osd.margin_right;
    let picture_height = osd.height - osd.margin_top - osd.margin_bottom;
    if picture_width <= 0.0 || picture_height <= 0.0 {
        return whole_surface;
    }
    let horizontal = source.width / picture_width;
    let vertical = source.height / picture_height;
    PicturePlacement {
        play_res_x: rounded(osd.width * horizontal).max(1) as u32,
        play_res_y: rounded(osd.height * vertical).max(1) as u32,
        offset_x: rounded(osd.margin_left * horizontal),
        offset_y: rounded(osd.margin_top * vertical),
    }
}

/// One ASS dialogue event: the drawing's origin at the picture's top left corner,
/// no border or shadow, and the event's rectangles filled in the ink asked for.
fn drawing_event(event: &OverlayEvent, placement: &PicturePlacement) -> String {
    let path: Vec<String> = event
        .rectangles
        .iter()
        .map(|rectangle| rectangle_path(*rectangle))
        .collect();
    let path = path.join(" ");
    format!(
        "{{\\an7\\pos({},{})\\bord0\\shad0\\1c&H{}&\\1a&H{}&\\p1}}{path}{{\\p0}}",
        placement.offset_x, placement.offset_y, event.ink.colour, event.ink.alpha
    )
}

fn rectangle_path(rectangle: OverlayRectangle) -> String {
    let OverlayRectangle {
        x,
        y,
        width,
        height,
        ..
    } = rectangle;
    let right = x + width;
    let bottom = y + height;
    format!("m {x} {y} l {right} {y} l {right} {bottom} l {x} {bottom}")
}

/// A rectangle's edges as four filled bars, because a path with a hole in it
/// depends on which way round libass winds the two.
fn outline_rectangles(
    ink: OverlayInk,
    x: i64,
    y: i64,
    width: i64,
    height: i64,
    thickness: i64,
) -> Vec<OverlayRectangle> {
    let sides_height = height - 2 * thickness;
    vec![
        ink.filled(x, y, width, thickness),
        ink.filled(x, y + height - thickness, width, thickness),
        ink.filled(x, y + thickness, thickness, sides_height),
        ink.filled(
            x + width - thickness,
            y + thickness,
            thickness,
            sides_height,
        ),
    ]
}

fn safe_area_event(percent: u8, source: SourceSize) -> OverlayEvent {
    let size = f64::from(percent) / 100.0;
    let ink = SAFE_AREA_INK;
    OverlayEvent {
        ink,
        rectangles: outline_rectangles(
            ink,
            rounded((source.width - source.width * size) / 2.0),
            rounded((source.height - source.height * size) / 2.0),
            rounded(source.width * size),
            rounded(source.height * size),
            SAFE_AREA_THICKNESS,
        ),
    }
}

/// The bands a target aspect leaves on the picture, as fills over it. Which pair
/// applies is arithmetic here, the picture's own size being known.
fn aspect_mask_event(aspect: f64, source: SourceSize) -> Option<OverlayEvent> {
    let width = source.width;
    let height = source.height;
    let band_width = (width - height * aspect) / 2.0;
    let band_height = (height - width / aspect) / 2.0;
    let ink = ASPECT_MASK_INK;
    let rectangles = if band_width >= SMALLEST_MASK_BAND_PIXELS {
        let band = rounded(band_width);
        vec![
            ink.filled(0, 0, band, rounded(height)),
            ink.filled(rounded(width) - band, 0, band, rounded(height)),
        ]
    } else if band_height >= SMALLEST_MASK_BAND_PIXELS {
        let band = rounded(band_height);
        vec![
            ink.filled(0, 0, rounded(width), band),
            ink.filled(0, rounded(height) - band, rounded(width), band),
        ]
    } else {
        // the target is the picture's own aspect, so there is nothing to mask
        return None;
    };
    Some(OverlayEvent { ink, rectangles })
}

/// The bands the crop discards, as fills, plus an outline around what it keeps.
fn crop_events(crop: PreviewCrop, source: SourceSize) -> Vec<OverlayEvent> {
    let width = rounded(source.width);
    let height = rounded(source.height);
    let left = i64::from(crop.left);
    let right = i64::from(crop.right);
    let top = i64::from(crop.top);
    let bottom = i64::from(crop.bottom);
    let mut events = Vec::new();
    let bands: Vec<OverlayRectangle> = [
        (left > 0, (0, 0, left, height)),
        (right > 0, (width - right, 0, right, height)),
        (top > 0, (0, 0, width, top)),
        (bottom > 0, (0, height - bottom, width, bottom)),
    ]
    .into_iter()
    .filter(|(cropped, _)| *cropped)
    .map(|(_, (x, y, band_width, band_height))| CROP_BAND_INK.filled(x, y, band_width, band_height))
    .collect();
    if !bands.is_empty() {
        events.push(OverlayEvent {
            ink: CROP_BAND_INK,
            rectangles: bands,
        });
    }
    events.push(OverlayEvent {
        ink: CROP_OUTLINE_INK,
        rectangles: outline_rectangles(
            CROP_OUTLINE_INK,
            left,
            top,
            width - left - right,
            height - top - bottom,
            CROP_OUTLINE_THICKNESS,
        ),
    });
    events
}

fn centre_cross_event(source: SourceSize) -> OverlayEvent {
    let width = rounded(source.width);
    let height = rounded(source.height);
    let ink = CENTRE_CROSS_INK;
    OverlayEvent {
        ink,
        rectangles: vec![
            ink.filled(
                (width - CENTRE_CROSS_THICKNESS) / 2,
                0,
                CENTRE_CROSS_THICKNESS,
                height,
            ),
            ink.filled(
                0,
                (height - CENTRE_CROSS_THICKNESS) / 2,
                width,
                CENTRE_CROSS_THICKNESS,
            ),
        ],
    }
}

/// The four lines that divide the picture in thirds, the picture's own edges left
/// alone.
fn thirds_grid_event(source: SourceSize) -> OverlayEvent {
    let width = rounded(source.width);
    let height = rounded(source.height);
    let ink = THIRDS_GRID_INK;
    OverlayEvent {
        ink,
        rectangles: vec![
            ink.filled(
                rounded(source.width / 3.0),
                0,
                THIRDS_GRID_THICKNESS,
                height,
            ),
            ink.filled(
                rounded(source.width * 2.0 / 3.0),
                0,
                THIRDS_GRID_THICKNESS,
                height,
            ),
            ink.filled(
                0,
                rounded(source.height / 3.0),
                width,
                THIRDS_GRID_THICKNESS,
            ),
            ink.filled(
                0,
                rounded(source.height * 2.0 / 3.0),
                width,
                THIRDS_GRID_THICKNESS,
            ),
        ],
    }
}

/// ASS drawing coordinates are whole numbers, and sub-pixel placement is below
/// what a scaled preview can show anyway.
fn rounded(value: f64) -> i64 {
    value.round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT_CROP: PreviewCrop = PreviewCrop {
        left: 138,
        right: 0,
        top: 0,
        bottom: 0,
    };

    /// The source most of these measure against.
    fn hd_source() -> Option<SourceSize> {
        SourceSize::new(1920.0, 1080.0)
    }

    /// The overlay drawn on a surface the picture fills, so the drawing is in the
    /// source's own pixels and nothing is shifted.
    fn drawn(overlays: &PreviewOverlays, source: Option<SourceSize>) -> Option<OverlayDrawing> {
        overlay_drawing(overlays, source, None)
    }

    fn events(overlays: &PreviewOverlays) -> String {
        drawn(overlays, hd_source())
            .map(|d| d.events)
            .unwrap_or_default()
    }

    fn rectangles(overlays: &PreviewOverlays) -> Vec<OverlayRectangle> {
        overlay_rectangles(overlays, hd_source().unwrap())
    }

    fn every_overlay() -> PreviewOverlays {
        PreviewOverlays {
            safe_area_percent: Some(95),
            aspect_mask: Some(2.39),
            centre_cross: true,
            thirds_grid: true,
            crop: Some(LEFT_CROP),
            crop_visible: true,
        }
    }

    #[test]
    fn nothing_on_draws_no_overlay() {
        assert!(drawn(&PreviewOverlays::default(), hd_source()).is_none());
    }

    #[test]
    fn the_canvas_is_the_picture_when_it_fills_the_surface() {
        let drawing = drawn(
            &PreviewOverlays {
                centre_cross: true,
                ..Default::default()
            },
            hd_source(),
        )
        .unwrap();
        assert_eq!((drawing.play_res_x, drawing.play_res_y), (1920, 1080));
    }

    #[test]
    fn safe_area_is_a_centred_outline() {
        assert_eq!(
            events(&PreviewOverlays {
                safe_area_percent: Some(95),
                ..Default::default()
            }),
            "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&HFFFFFF&\\1a&H33&\\p1}\
             m 48 27 l 1872 27 l 1872 29 l 48 29 \
             m 48 1051 l 1872 1051 l 1872 1053 l 48 1053 \
             m 48 29 l 50 29 l 50 1051 l 48 1051 \
             m 1870 29 l 1872 29 l 1872 1051 l 1870 1051{\\p0}"
        );
    }

    #[test]
    fn safe_area_takes_the_percent_it_is_given() {
        assert_eq!(
            events(&PreviewOverlays {
                safe_area_percent: Some(90),
                ..Default::default()
            }),
            "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&HFFFFFF&\\1a&H33&\\p1}\
             m 96 54 l 1824 54 l 1824 56 l 96 56 \
             m 96 1024 l 1824 1024 l 1824 1026 l 96 1026 \
             m 96 56 l 98 56 l 98 1024 l 96 1024 \
             m 1822 56 l 1824 56 l 1824 1024 l 1822 1024{\\p0}"
        );
    }

    #[test]
    fn a_wider_target_aspect_bands_the_top_and_bottom() {
        // 2.39 on a 16:9 picture leaves (1080 - 1920/2.39) / 2 off each of them
        assert_eq!(
            events(&PreviewOverlays {
                aspect_mask: Some(2.39),
                ..Default::default()
            }),
            "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H000000&\\1a&H66&\\p1}\
             m 0 0 l 1920 0 l 1920 138 l 0 138 \
             m 0 942 l 1920 942 l 1920 1080 l 0 1080{\\p0}"
        );
    }

    #[test]
    fn a_narrower_target_aspect_bands_the_sides() {
        // 1.85 on a 2048x858 picture leaves (2048 - 858*1.85) / 2 off each side
        let drawing = overlay_drawing(
            &PreviewOverlays {
                aspect_mask: Some(1.85),
                ..Default::default()
            },
            SourceSize::new(2048.0, 858.0),
            None,
        )
        .unwrap();
        assert_eq!(
            drawing.events,
            "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H000000&\\1a&H66&\\p1}\
             m 0 0 l 230 0 l 230 858 l 0 858 \
             m 1818 0 l 2048 0 l 2048 858 l 1818 858{\\p0}"
        );
    }

    #[test]
    fn a_target_aspect_the_picture_already_has_draws_nothing() {
        let scope = SourceSize::new(1998.0, 1080.0);
        assert!(overlay_drawing(
            &PreviewOverlays {
                aspect_mask: Some(1.85),
                ..Default::default()
            },
            scope,
            None,
        )
        .is_none());
    }

    #[test]
    fn centre_cross_is_two_lines_through_the_middle() {
        assert_eq!(
            events(&PreviewOverlays {
                centre_cross: true,
                ..Default::default()
            }),
            "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&HFFFFFF&\\1a&H33&\\p1}\
             m 959 0 l 961 0 l 961 1080 l 959 1080 \
             m 0 539 l 1920 539 l 1920 541 l 0 541{\\p0}"
        );
    }

    #[test]
    fn thirds_grid_is_four_lines_at_the_thirds() {
        assert_eq!(
            events(&PreviewOverlays {
                thirds_grid: true,
                ..Default::default()
            }),
            "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&HFFFFFF&\\1a&H99&\\p1}\
             m 640 0 l 641 0 l 641 1080 l 640 1080 \
             m 1280 0 l 1281 0 l 1281 1080 l 1280 1080 \
             m 0 360 l 1920 360 l 1920 361 l 0 361 \
             m 0 720 l 1920 720 l 1920 721 l 0 721{\\p0}"
        );
    }

    #[test]
    fn crop_shades_the_cropped_edge_and_outlines_what_is_kept() {
        assert_eq!(
            events(&PreviewOverlays {
                crop: Some(LEFT_CROP),
                crop_visible: true,
                ..Default::default()
            }),
            "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H0000FF&\\1a&HA6&\\p1}\
             m 0 0 l 138 0 l 138 1080 l 0 1080{\\p0}\n\
             {\\an7\\pos(0,0)\\bord0\\shad0\\1c&H0000FF&\\1a&H1A&\\p1}\
             m 138 0 l 1920 0 l 1920 2 l 138 2 \
             m 138 1078 l 1920 1078 l 1920 1080 l 138 1080 \
             m 138 2 l 140 2 l 140 1078 l 138 1078 \
             m 1918 2 l 1920 2 l 1920 1078 l 1918 1078{\\p0}"
        );
    }

    #[test]
    fn crop_shades_every_edge_it_takes_pixels_off() {
        let drawing = drawn(
            &PreviewOverlays {
                crop: Some(PreviewCrop {
                    left: 10,
                    right: 20,
                    top: 30,
                    bottom: 40,
                }),
                crop_visible: true,
                ..Default::default()
            },
            hd_source(),
        )
        .unwrap();
        let bands = drawing.events.lines().next().unwrap();
        assert_eq!(
            bands,
            "{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H0000FF&\\1a&HA6&\\p1}\
             m 0 0 l 10 0 l 10 1080 l 0 1080 \
             m 1900 0 l 1920 0 l 1920 1080 l 1900 1080 \
             m 0 0 l 1920 0 l 1920 30 l 0 30 \
             m 0 1040 l 1920 1040 l 1920 1080 l 0 1080{\\p0}"
        );
    }

    #[test]
    fn a_crop_in_pixels_is_the_same_share_of_a_wider_source() {
        let hd = drawn(
            &PreviewOverlays {
                crop: Some(LEFT_CROP),
                crop_visible: true,
                ..Default::default()
            },
            hd_source(),
        )
        .unwrap();
        let scope = drawn(
            &PreviewOverlays {
                crop: Some(LEFT_CROP),
                crop_visible: true,
                ..Default::default()
            },
            SourceSize::new(1998.0, 1080.0),
        )
        .unwrap();
        // the band is the same 138 pixels on both, and the canvas is what makes
        // it a narrower share of the wider picture
        assert!(hd.events.contains("m 0 0 l 138 0 l 138 1080 l 0 1080"));
        assert!(scope.events.contains("m 0 0 l 138 0 l 138 1080 l 0 1080"));
        assert_eq!(hd.play_res_x, 1920);
        assert_eq!(scope.play_res_x, 1998);
    }

    #[test]
    fn a_crop_nobody_asked_to_see_draws_nothing() {
        assert!(drawn(
            &PreviewOverlays {
                crop: Some(LEFT_CROP),
                crop_visible: false,
                ..Default::default()
            },
            hd_source()
        )
        .is_none());
    }

    #[test]
    fn the_crop_toggle_draws_nothing_without_a_crop() {
        assert!(drawn(
            &PreviewOverlays {
                crop: None,
                crop_visible: true,
                ..Default::default()
            },
            hd_source()
        )
        .is_none());
    }

    #[test]
    fn nothing_is_drawn_while_nothing_reports_a_picture_size() {
        assert!(drawn(
            &PreviewOverlays {
                thirds_grid: true,
                ..Default::default()
            },
            None
        )
        .is_none());
    }

    #[test]
    fn a_source_with_no_size_at_all_is_refused() {
        assert!(SourceSize::new(0.0, 1080.0).is_none());
        assert!(SourceSize::new(1920.0, 0.0).is_none());
    }

    #[test]
    fn every_overlay_at_once_is_one_event_each_with_the_mask_first() {
        let drawing = drawn(
            &PreviewOverlays {
                safe_area_percent: Some(95),
                aspect_mask: Some(2.39),
                centre_cross: true,
                thirds_grid: true,
                crop: Some(LEFT_CROP),
                crop_visible: true,
            },
            hd_source(),
        )
        .unwrap();
        let events: Vec<&str> = drawing.events.lines().collect();
        // mask, crop bands, crop outline, safe area, cross, thirds
        assert_eq!(events.len(), 6);
        assert!(events[0].contains(&format!("&H{BLACK}&")));
        assert_eq!(
            events
                .iter()
                .filter(|event| event.contains(&format!("&H{RED}&")))
                .count(),
            2
        );
    }

    #[test]
    fn every_overlay_at_once_fills_rectangles_with_the_mask_bands_first() {
        let filled = rectangles(&every_overlay());
        assert!(!filled.is_empty());
        assert_eq!(filled[0].colour, [0, 0, 0]);
        let drawing = drawn(&every_overlay(), hd_source()).unwrap();
        assert_eq!(drawing.events.lines().count(), 6);
    }

    #[test]
    fn nothing_on_fills_no_rectangles() {
        assert!(rectangles(&PreviewOverlays::default()).is_empty());
    }

    #[test]
    fn the_crop_band_rectangles_are_the_edges_it_takes_off() {
        let filled = rectangles(&PreviewOverlays {
            crop: Some(LEFT_CROP),
            crop_visible: true,
            ..Default::default()
        });
        assert!(filled.contains(&OverlayRectangle {
            x: 0,
            y: 0,
            width: 138,
            height: 1080,
            colour: [255, 0, 0],
            alpha: 89,
        }));
    }

    #[test]
    fn ass_ink_reads_back_to_front_and_counts_transparency_down() {
        let band = OverlayInk {
            colour: RED,
            alpha: "A6",
        }
        .filled(0, 0, 1, 1);
        assert_eq!(band.colour, [255, 0, 0]);
        assert_eq!(band.alpha, 89);
        let safe_area = OverlayInk {
            colour: WHITE,
            alpha: "33",
        }
        .filled(0, 0, 1, 1);
        assert_eq!(safe_area.colour, [255, 255, 255]);
        assert_eq!(safe_area.alpha, 204);
    }

    #[test]
    fn the_overlays_switched_on_are_what_asks_for_a_drawing_at_all() {
        assert!(!PreviewOverlays::default().any());
        assert!(PreviewOverlays {
            thirds_grid: true,
            ..Default::default()
        }
        .any());
        // the crop needs both halves, a crop to draw and the toggle on
        assert!(!PreviewOverlays {
            crop: Some(LEFT_CROP),
            ..Default::default()
        }
        .any());
        assert!(!PreviewOverlays {
            crop_visible: true,
            ..Default::default()
        }
        .any());
    }

    /// The surface a wizard's preview panel gives mpv is a wide box of a fixed
    /// height, so a 1.85 picture sits between bars on the sides.
    #[test]
    fn a_picture_between_bars_is_drawn_onto_the_picture() {
        let letterboxed = OsdRectangle {
            width: 1200.0,
            height: 360.0,
            margin_left: 267.0,
            margin_top: 0.0,
            margin_right: 267.0,
            margin_bottom: 0.0,
        };
        let drawing = overlay_drawing(
            &PreviewOverlays {
                centre_cross: true,
                ..Default::default()
            },
            SourceSize::new(1998.0, 1080.0),
            Some(letterboxed),
        )
        .unwrap();
        // the canvas is the whole surface written in source pixels, and the
        // drawing starts where the picture does
        assert_eq!((drawing.play_res_x, drawing.play_res_y), (3600, 1080));
        assert!(
            drawing.events.starts_with("{\\an7\\pos(801,0)"),
            "{}",
            drawing.events
        );
    }

    #[test]
    fn a_surface_the_picture_fills_shifts_nothing() {
        let filled = OsdRectangle {
            width: 666.0,
            height: 360.0,
            margin_left: 0.0,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
        };
        let drawing = overlay_drawing(
            &PreviewOverlays {
                centre_cross: true,
                ..Default::default()
            },
            SourceSize::new(1998.0, 1080.0),
            Some(filled),
        )
        .unwrap();
        assert_eq!((drawing.play_res_x, drawing.play_res_y), (1998, 1080));
        assert!(drawing.events.starts_with("{\\an7\\pos(0,0)"));
    }

    /// mpv reports zeroes until it has rendered a frame, and a surface with no
    /// picture on it leaves nothing to measure against.
    #[test]
    fn a_surface_with_no_picture_on_it_is_drawn_as_if_the_picture_filled_it() {
        let empty = OsdRectangle {
            width: 0.0,
            height: 0.0,
            margin_left: 0.0,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
        };
        let drawing = overlay_drawing(
            &PreviewOverlays {
                centre_cross: true,
                ..Default::default()
            },
            hd_source(),
            Some(empty),
        )
        .unwrap();
        assert_eq!((drawing.play_res_x, drawing.play_res_y), (1920, 1080));
        assert!(drawing.events.starts_with("{\\an7\\pos(0,0)"));
    }
}
