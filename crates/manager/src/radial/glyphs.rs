//! The pictures of the actions drawn around a petal: a closed list of line
//! drawings in a 24 by 24 box, stroke 2, rounded corners, the same style as
//! the icons of the interface. The menu draws them with the rest of its SVG;
//! a module only names the one it wants.

use hubergamof_module_api::{RadialGlyph, RadialItemState};

/// Side of the box the drawings are made in.
pub const BOX: f64 = 24.0;

/// Width of the stroke, in the units of the box.
const STROKE: f64 = 2.0;

/// The stroke that crosses a glyph out when what it controls is off.
const CROSSED_OUT: &str = r#"<path d="M3 3l18 18"/>"#;

/// What a glyph that this version does not know is drawn as: a plain ring,
/// never a hole in the arc and never a panic.
const NEUTRAL: &str = r#"<circle cx="12" cy="12" r="7"/>"#;

/// The shapes of `glyph`, in the box, to draw with a stroke and no fill.
fn shapes(glyph: RadialGlyph) -> &'static str {
    match glyph {
        RadialGlyph::Microphone => {
            r#"<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/>"#
        }
        RadialGlyph::Camera => {
            r#"<rect x="3" y="7" width="12" height="10" rx="2"/><path d="M15 11l6-3v8l-6-3z"/>"#
        }
        RadialGlyph::HangUp => {
            r#"<path d="M3 14c5-4 13-4 18 0l-2 3-4-1v-2c-2-1-4-1-6 0v2l-4 1z"/>"#
        }
        RadialGlyph::Chat => r#"<path d="M4 5h16v11H9l-5 4z"/>"#,
        RadialGlyph::Bell => r#"<path d="M6 16v-5a6 6 0 0 1 12 0v5l2 2H4zM10 21h4"/>"#,
        RadialGlyph::People => {
            r#"<circle cx="9" cy="8" r="3"/><path d="M3 20v-2a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v2M16 5a3 3 0 0 1 0 6M18 14a4 4 0 0 1 3 4v2"/>"#
        }
        RadialGlyph::ReturnToCall => r#"<path d="M4 12h14M12 6l6 6-6 6"/>"#,
        RadialGlyph::Speaker => {
            r#"<path d="M4 9h4l5-4v14l-5-4H4z"/><path d="M16 9a4 4 0 0 1 0 6"/>"#
        }
        RadialGlyph::Play => r#"<path d="M8 5l11 7-11 7z"/>"#,
        RadialGlyph::Pause => r#"<path d="M8 5v14M16 5v14"/>"#,
        RadialGlyph::Skip => r#"<path d="M5 5l9 7-9 7zM18 5v14"/>"#,
        RadialGlyph::Reset => r#"<path d="M4 12a8 8 0 1 0 3-6M4 4v5h5"/>"#,
        _ => NEUTRAL,
    }
}

/// Whether `state` crosses the glyph out.
fn is_crossed_out(state: RadialItemState) -> bool {
    matches!(state, RadialItemState::Off)
}

/// The SVG of `glyph` in `state`, `size` pixels wide, centred on (`x`, `y`),
/// drawn with the stroke `color` (an SVG colour). A state this version does
/// not know is drawn as the usual one.
#[must_use]
pub fn svg(
    glyph: RadialGlyph,
    state: RadialItemState,
    centre: (f64, f64),
    size: f64,
    color: &str,
) -> String {
    let scale = size / BOX;
    let (left, top) = (centre.0 - size / 2.0, centre.1 - size / 2.0);
    let mut drawing = format!(
        r#"<g transform="translate({left} {top}) scale({scale})" fill="none" stroke="{color}" stroke-width="{STROKE}" stroke-linecap="round" stroke-linejoin="round">{}"#,
        shapes(glyph)
    );
    if is_crossed_out(state) {
        drawing.push_str(CROSSED_OUT);
    }
    drawing.push_str("</g>");
    drawing
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [RadialGlyph; 12] = [
        RadialGlyph::Microphone,
        RadialGlyph::Camera,
        RadialGlyph::HangUp,
        RadialGlyph::Chat,
        RadialGlyph::Bell,
        RadialGlyph::People,
        RadialGlyph::ReturnToCall,
        RadialGlyph::Speaker,
        RadialGlyph::Play,
        RadialGlyph::Pause,
        RadialGlyph::Skip,
        RadialGlyph::Reset,
    ];

    #[test]
    fn every_glyph_has_its_own_drawing() {
        let mut seen = std::collections::HashSet::new();
        for glyph in ALL {
            let drawing = shapes(glyph);
            assert!(drawing.starts_with('<'), "{glyph:?}");
            assert_ne!(drawing, NEUTRAL, "{glyph:?} falls back to the neutral ring");
            assert!(seen.insert(drawing), "{glyph:?} draws like another");
        }
    }

    #[test]
    fn a_glyph_is_centred_and_scaled_from_its_box() {
        let drawing = svg(
            RadialGlyph::Play,
            RadialItemState::Normal,
            (100.0, 50.0),
            12.0,
            "#fff",
        );
        // 12 px in a 24 box is half; the top left is 6 px from the centre.
        assert!(
            drawing.contains(r"translate(94 44) scale(0.5)"),
            "{drawing}"
        );
        assert!(drawing.contains(r##"stroke="#fff""##));
        assert!(drawing.contains(r#"fill="none""#));
    }

    #[test]
    fn only_an_item_that_is_off_is_crossed_out() {
        let draw = |state| svg(RadialGlyph::Microphone, state, (0.0, 0.0), 20.0, "#fff");
        assert!(!draw(RadialItemState::Normal).contains("M3 3l18 18"));
        assert!(draw(RadialItemState::Off).contains("M3 3l18 18"));
    }

    #[test]
    fn the_drawings_are_well_formed_svg() {
        for glyph in ALL {
            for state in [RadialItemState::Normal, RadialItemState::Off] {
                let body = svg(glyph, state, (12.0, 12.0), 24.0, "#000");
                let document = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{body}</svg>"#);
                assert!(
                    resvg::usvg::Tree::from_str(&document, &resvg::usvg::Options::default())
                        .is_ok(),
                    "{glyph:?} {state:?}"
                );
            }
        }
    }
}
