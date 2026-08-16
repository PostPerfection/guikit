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

const SAFE_AREA_THICKNESS_PIXELS: u32 = 2;
const CENTRE_CROSS_THICKNESS_PIXELS: u32 = 2;
const THIRDS_GRID_THICKNESS_PIXELS: u32 = 1;

/// Which overlays the page asked for. All off is the default, and produces an
/// empty chain.
#[derive(Default, Deserialize)]
pub struct PreviewOverlays {
    pub safe_area_percent: Option<u8>,
    pub aspect_mask: Option<f64>,
    pub centre_cross: bool,
    pub thirds_grid: bool,
}

/// The value for mpv's `vf` property, empty when no overlay is on.
pub fn overlay_filter_chain(overlays: &PreviewOverlays) -> String {
    let mut filters: Vec<String> = Vec::new();
    // the mask is a fill, so it draws first or it covers the lines
    if let Some(aspect) = overlays.aspect_mask {
        filters.extend(aspect_mask_filters(aspect));
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

    #[test]
    fn nothing_on_clears_the_chain() {
        assert_eq!(overlay_filter_chain(&PreviewOverlays::default()), "");
    }

    #[test]
    fn safe_area_is_a_centred_outline() {
        let chain = overlay_filter_chain(&PreviewOverlays {
            safe_area_percent: Some(95),
            ..Default::default()
        });
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=iw*0.025:y=ih*0.025:w=iw*0.95:h=ih*0.95:color=white@0.8:t=2]"
        );
    }

    #[test]
    fn safe_area_takes_the_percent_it_is_given() {
        let chain = overlay_filter_chain(&PreviewOverlays {
            safe_area_percent: Some(90),
            ..Default::default()
        });
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=iw*0.05:y=ih*0.05:w=iw*0.9:h=ih*0.9:color=white@0.8:t=2]"
        );
    }

    #[test]
    fn aspect_mask_covers_both_band_pairs() {
        let chain = overlay_filter_chain(&PreviewOverlays {
            aspect_mask: Some(2.39),
            ..Default::default()
        });
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
        let chain = overlay_filter_chain(&PreviewOverlays {
            centre_cross: true,
            ..Default::default()
        });
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=(iw-2)/2:y=0:w=2:h=ih:color=white@0.8:t=fill,\
             drawbox=x=0:y=(ih-2)/2:w=iw:h=2:color=white@0.8:t=fill]"
        );
    }

    #[test]
    fn thirds_grid_divides_the_frame() {
        let chain = overlay_filter_chain(&PreviewOverlays {
            thirds_grid: true,
            ..Default::default()
        });
        assert_eq!(chain, "lavfi=[drawgrid=w=iw/3:h=ih/3:color=white@0.4:t=1]");
    }

    #[test]
    fn every_overlay_at_once_masks_first() {
        let chain = overlay_filter_chain(&PreviewOverlays {
            safe_area_percent: Some(95),
            aspect_mask: Some(1.85),
            centre_cross: true,
            thirds_grid: true,
        });
        assert_eq!(
            chain,
            "lavfi=[drawbox=x=0:y=0:w=(iw-ih*1.85)/2:h=ih:color=black@0.6:t=fill:enable='gt(w/h,1.85)',\
             drawbox=x=(iw+ih*1.85)/2:y=0:w=(iw-ih*1.85)/2:h=ih:color=black@0.6:t=fill:enable='gt(w/h,1.85)',\
             drawbox=x=0:y=0:w=iw:h=(ih-iw/1.85)/2:color=black@0.6:t=fill:enable='lt(w/h,1.85)',\
             drawbox=x=0:y=(ih+iw/1.85)/2:w=iw:h=(ih-iw/1.85)/2:color=black@0.6:t=fill:enable='lt(w/h,1.85)',\
             drawbox=x=iw*0.025:y=ih*0.025:w=iw*0.95:h=ih*0.95:color=white@0.8:t=2,\
             drawbox=x=(iw-2)/2:y=0:w=2:h=ih:color=white@0.8:t=fill,\
             drawbox=x=0:y=(ih-2)/2:w=iw:h=2:color=white@0.8:t=fill,\
             drawgrid=w=iw/3:h=ih/3:color=white@0.4:t=1]"
        );
    }
}
