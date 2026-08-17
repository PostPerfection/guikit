//! The QC overlays drawn over playback, as one mpv filter chain.
//!
//! Everything here is pure text: the chain is built from the requested overlays
//! and handed to mpv's `vf` property. Sizes are ffmpeg expressions over `iw` and
//! `ih` so one chain suits any frame size, reduced decode resolutions included.

use serde::Deserialize;

const SAFE_AREA_COLOUR: &str = "white@0.8";
const ASPECT_MASK_COLOUR: &str = "black@0.6";
const CENTRE_CROSS_COLOUR: &str = "white@0.8";
const THIRDS_GRID_COLOUR: &str = "white@0.4";
const CROP_BAND_COLOUR: &str = "red@0.35";
const CROP_OUTLINE_COLOUR: &str = "red@0.9";

const SAFE_AREA_THICKNESS_PIXELS: u32 = 2;
const CENTRE_CROSS_THICKNESS_PIXELS: u32 = 2;
const THIRDS_GRID_THICKNESS_PIXELS: u32 = 1;
const CROP_OUTLINE_THICKNESS_PIXELS: u32 = 2;
/// How far the crop fractions are written out, which is under a pixel on any
/// frame the preview plays.
const CROP_FRACTION_DECIMALS: usize = 4;

/// The picture size the container declares, which is what the crop is measured
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

/// What the job's crop takes off each edge, in source pixels.
#[derive(Clone, Copy, Deserialize)]
pub struct PreviewCrop {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

/// Which overlays the page asked for. All off is the default, and produces an
/// empty chain.
#[derive(Default, Deserialize)]
pub struct PreviewOverlays {
    pub safe_area_percent: Option<u8>,
    pub aspect_mask: Option<f64>,
    pub centre_cross: bool,
    pub thirds_grid: bool,
    pub crop: Option<PreviewCrop>,
    pub crop_visible: bool,
}

/// The value for mpv's `vf` property, empty when no overlay is on. The crop is
/// the one overlay given in pixels, so it needs the source size to turn them
/// into fractions of the frame, and draws nothing without one.
pub fn overlay_filter_chain(overlays: &PreviewOverlays, source_size: Option<SourceSize>) -> String {
    let mut filters: Vec<String> = Vec::new();
    // the mask is a fill, so it draws first or it covers the lines
    if let Some(aspect) = overlays.aspect_mask {
        filters.extend(aspect_mask_filters(aspect));
    }
    if let Some(crop) = overlays.crop.filter(|_| overlays.crop_visible) {
        if let Some(source_size) = source_size {
            filters.extend(crop_filters(crop, source_size));
        }
    }
    if let Some(percent) = overlays.safe_area_percent {
        filters.push(safe_area_filter(percent));
    }
    if overlays.centre_cross {
        filters.extend(centre_cross_filters());
    }
    if overlays.thirds_grid {
        filters.push(thirds_grid_filter());
    }
    if filters.is_empty() {
        return String::new();
    }
    // the bracket form keeps mpv's option parser out of the graph text, which
    // holds the separators it would otherwise split on
    format!("lavfi=[{}]", filters.join(","))
}

fn safe_area_filter(percent: u8) -> String {
    let size = f64::from(percent) / 100.0;
    // from the percent rather than from `size`, so the text stays free of the
    // digits a subtraction of two fractions leaves behind
    let inset = f64::from(100 - i32::from(percent)) / 200.0;
    format!(
        "drawbox=x=iw*{inset}:y=ih*{inset}:w=iw*{size}:h=ih*{size}:color={SAFE_AREA_COLOUR}:t={SAFE_AREA_THICKNESS_PIXELS}"
    )
}

/// The bands a target aspect leaves on the frame, as fills over the picture.
fn aspect_mask_filters(aspect: f64) -> [String; 4] {
    let band_width = format!("(iw-ih*{aspect})/2");
    let band_height = format!("(ih-iw/{aspect})/2");
    // a drawbox sized zero covers the whole frame, so the pair that does not
    // apply to this frame is switched off rather than sized away
    let wider_than_target = format!("gt(w/h,{aspect})");
    let taller_than_target = format!("lt(w/h,{aspect})");
    [
        format!(
            "drawbox=x=0:y=0:w={band_width}:h=ih:color={ASPECT_MASK_COLOUR}:t=fill:enable='{wider_than_target}'"
        ),
        format!(
            "drawbox=x=(iw+ih*{aspect})/2:y=0:w={band_width}:h=ih:color={ASPECT_MASK_COLOUR}:t=fill:enable='{wider_than_target}'"
        ),
        format!(
            "drawbox=x=0:y=0:w=iw:h={band_height}:color={ASPECT_MASK_COLOUR}:t=fill:enable='{taller_than_target}'"
        ),
        format!(
            "drawbox=x=0:y=(ih+iw/{aspect})/2:w=iw:h={band_height}:color={ASPECT_MASK_COLOUR}:t=fill:enable='{taller_than_target}'"
        ),
    ]
}

/// The bands the crop discards, as fills, plus an outline around what it keeps.
/// Each edge is written as a fraction of the frame, because the decoded frame is
/// only smaller than the source where the decoder honours the decode scale.
fn crop_filters(crop: PreviewCrop, source: SourceSize) -> Vec<String> {
    let left = f64::from(crop.left) / source.width;
    let right = f64::from(crop.right) / source.width;
    let top = f64::from(crop.top) / source.height;
    let bottom = f64::from(crop.bottom) / source.height;
    let mut filters = Vec::new();
    // a drawbox sized zero covers the whole frame, so an edge with no crop on
    // it gets no band at all
    if left > 0.0 {
        filters.push(format!(
            "drawbox=x=0:y=0:w={}:h=ih:color={CROP_BAND_COLOUR}:t=fill",
            frame_fraction("iw", left)
        ));
    }
    if right > 0.0 {
        filters.push(format!(
            "drawbox=x={}:y=0:w={}:h=ih:color={CROP_BAND_COLOUR}:t=fill",
            frame_fraction("iw", 1.0 - right),
            frame_fraction("iw", right)
        ));
    }
    if top > 0.0 {
        filters.push(format!(
            "drawbox=x=0:y=0:w=iw:h={}:color={CROP_BAND_COLOUR}:t=fill",
            frame_fraction("ih", top)
        ));
    }
    if bottom > 0.0 {
        filters.push(format!(
            "drawbox=x=0:y={}:w=iw:h={}:color={CROP_BAND_COLOUR}:t=fill",
            frame_fraction("ih", 1.0 - bottom),
            frame_fraction("ih", bottom)
        ));
    }
    filters.push(format!(
        "drawbox=x={}:y={}:w={}:h={}:color={CROP_OUTLINE_COLOUR}:t={CROP_OUTLINE_THICKNESS_PIXELS}",
        frame_fraction("iw", left),
        frame_fraction("ih", top),
        frame_fraction("iw", 1.0 - left - right),
        frame_fraction("ih", 1.0 - top - bottom),
    ));
    filters
}

/// A fraction of `iw` or `ih` as drawbox takes it. A whole frame and an empty
/// one are written plainly rather than as a multiplication.
fn frame_fraction(dimension: &str, fraction: f64) -> String {
    if fraction <= 0.0 {
        return "0".to_string();
    }
    if fraction >= 1.0 {
        return dimension.to_string();
    }
    format!(
        "{dimension}*{fraction:.decimals$}",
        decimals = CROP_FRACTION_DECIMALS
    )
}

fn centre_cross_filters() -> [String; 2] {
    [
        format!(
            "drawbox=x=(iw-{CENTRE_CROSS_THICKNESS_PIXELS})/2:y=0:w={CENTRE_CROSS_THICKNESS_PIXELS}:h=ih:color={CENTRE_CROSS_COLOUR}:t=fill"
        ),
        format!(
            "drawbox=x=0:y=(ih-{CENTRE_CROSS_THICKNESS_PIXELS})/2:w=iw:h={CENTRE_CROSS_THICKNESS_PIXELS}:color={CENTRE_CROSS_COLOUR}:t=fill"
        ),
    ]
}

fn thirds_grid_filter() -> String {
    format!("drawgrid=w=iw/3:h=ih/3:color={THIRDS_GRID_COLOUR}:t={THIRDS_GRID_THICKNESS_PIXELS}")
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

    /// The source most of these measure against, so a crop in pixels has a
    /// frame size to become a fraction of.
    fn hd_source() -> Option<SourceSize> {
        SourceSize::new(1920.0, 1080.0)
    }

    #[test]
    fn nothing_on_clears_the_chain() {
        assert_eq!(
            overlay_filter_chain(&PreviewOverlays::default(), hd_source()),
            ""
        );
    }

    #[test]
    fn safe_area_is_a_centred_outline() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                safe_area_percent: Some(95),
                ..Default::default()
            },
            hd_source(),
        );
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=iw*0.025:y=ih*0.025:w=iw*0.95:h=ih*0.95:color=white@0.8:t=2]"
        );
    }

    #[test]
    fn safe_area_takes_the_percent_it_is_given() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                safe_area_percent: Some(90),
                ..Default::default()
            },
            hd_source(),
        );
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=iw*0.05:y=ih*0.05:w=iw*0.9:h=ih*0.9:color=white@0.8:t=2]"
        );
    }

    #[test]
    fn aspect_mask_covers_both_band_pairs() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                aspect_mask: Some(2.39),
                ..Default::default()
            },
            hd_source(),
        );
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=0:y=0:w=(iw-ih*2.39)/2:h=ih:color=black@0.6:t=fill:enable='gt(w/h,2.39)',\
             drawbox=x=(iw+ih*2.39)/2:y=0:w=(iw-ih*2.39)/2:h=ih:color=black@0.6:t=fill:enable='gt(w/h,2.39)',\
             drawbox=x=0:y=0:w=iw:h=(ih-iw/2.39)/2:color=black@0.6:t=fill:enable='lt(w/h,2.39)',\
             drawbox=x=0:y=(ih+iw/2.39)/2:w=iw:h=(ih-iw/2.39)/2:color=black@0.6:t=fill:enable='lt(w/h,2.39)']"
        );
    }

    #[test]
    fn centre_cross_is_two_lines_through_the_middle() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                centre_cross: true,
                ..Default::default()
            },
            hd_source(),
        );
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=(iw-2)/2:y=0:w=2:h=ih:color=white@0.8:t=fill,\
             drawbox=x=0:y=(ih-2)/2:w=iw:h=2:color=white@0.8:t=fill]"
        );
    }

    #[test]
    fn thirds_grid_divides_the_frame() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                thirds_grid: true,
                ..Default::default()
            },
            hd_source(),
        );
        assert_eq!(chain, "lavfi=[drawgrid=w=iw/3:h=ih/3:color=white@0.4:t=1]");
    }

    #[test]
    fn crop_shades_the_cropped_edge_and_outlines_what_is_kept() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                crop: Some(LEFT_CROP),
                crop_visible: true,
                ..Default::default()
            },
            hd_source(),
        );
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=0:y=0:w=iw*0.0719:h=ih:color=red@0.35:t=fill,\
             drawbox=x=iw*0.0719:y=0:w=iw*0.9281:h=ih:color=red@0.9:t=2]"
        );
    }

    #[test]
    fn the_same_crop_is_a_wider_band_on_a_narrower_source() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                crop: Some(LEFT_CROP),
                crop_visible: true,
                ..Default::default()
            },
            SourceSize::new(1998.0, 1080.0),
        );
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=0:y=0:w=iw*0.0691:h=ih:color=red@0.35:t=fill,\
             drawbox=x=iw*0.0691:y=0:w=iw*0.9309:h=ih:color=red@0.9:t=2]"
        );
    }

    #[test]
    fn crop_shades_every_edge_it_takes_pixels_off() {
        let chain = overlay_filter_chain(
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
        );
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=0:y=0:w=iw*0.0052:h=ih:color=red@0.35:t=fill,\
             drawbox=x=iw*0.9896:y=0:w=iw*0.0104:h=ih:color=red@0.35:t=fill,\
             drawbox=x=0:y=0:w=iw:h=ih*0.0278:color=red@0.35:t=fill,\
             drawbox=x=0:y=ih*0.9630:w=iw:h=ih*0.0370:color=red@0.35:t=fill,\
             drawbox=x=iw*0.0052:y=ih*0.0278:w=iw*0.9844:h=ih*0.9352:color=red@0.9:t=2]"
        );
    }

    #[test]
    fn a_crop_with_no_source_size_to_measure_it_draws_nothing() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                crop: Some(LEFT_CROP),
                crop_visible: true,
                thirds_grid: true,
                ..Default::default()
            },
            None,
        );
        assert_eq!(chain, "lavfi=[drawgrid=w=iw/3:h=ih/3:color=white@0.4:t=1]");
    }

    #[test]
    fn a_source_with_no_size_at_all_is_refused() {
        assert!(SourceSize::new(0.0, 1080.0).is_none());
        assert!(SourceSize::new(1920.0, 0.0).is_none());
    }

    #[test]
    fn a_crop_nobody_asked_to_see_draws_nothing() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                crop: Some(LEFT_CROP),
                crop_visible: false,
                ..Default::default()
            },
            hd_source(),
        );
        assert_eq!(chain, "");
    }

    #[test]
    fn the_crop_toggle_draws_nothing_without_a_crop() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                crop: None,
                crop_visible: true,
                ..Default::default()
            },
            hd_source(),
        );
        assert_eq!(chain, "");
    }

    #[test]
    fn every_overlay_at_once_masks_first() {
        let chain = overlay_filter_chain(
            &PreviewOverlays {
                safe_area_percent: Some(95),
                aspect_mask: Some(1.85),
                centre_cross: true,
                thirds_grid: true,
                crop: Some(LEFT_CROP),
                crop_visible: true,
            },
            hd_source(),
        );
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=0:y=0:w=(iw-ih*1.85)/2:h=ih:color=black@0.6:t=fill:enable='gt(w/h,1.85)',\
             drawbox=x=(iw+ih*1.85)/2:y=0:w=(iw-ih*1.85)/2:h=ih:color=black@0.6:t=fill:enable='gt(w/h,1.85)',\
             drawbox=x=0:y=0:w=iw:h=(ih-iw/1.85)/2:color=black@0.6:t=fill:enable='lt(w/h,1.85)',\
             drawbox=x=0:y=(ih+iw/1.85)/2:w=iw:h=(ih-iw/1.85)/2:color=black@0.6:t=fill:enable='lt(w/h,1.85)',\
             drawbox=x=0:y=0:w=iw*0.0719:h=ih:color=red@0.35:t=fill,\
             drawbox=x=iw*0.0719:y=0:w=iw*0.9281:h=ih:color=red@0.9:t=2,\
             drawbox=x=iw*0.025:y=ih*0.025:w=iw*0.95:h=ih*0.95:color=white@0.8:t=2,\
             drawbox=x=(iw-2)/2:y=0:w=2:h=ih:color=white@0.8:t=fill,\
             drawbox=x=0:y=(ih-2)/2:w=iw:h=2:color=white@0.8:t=fill,\
             drawgrid=w=iw/3:h=ih/3:color=white@0.4:t=1]"
        );
    }
}
