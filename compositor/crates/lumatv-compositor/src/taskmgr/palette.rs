//! The colours of the basic screen: the theme's tokens, as the UI last sent them.
//!
//! The compositor draws the basic screen only when the UI cannot, so it cannot ask the UI for a colour then. The UI sends these five with
//! its layout (`{"op": "layout", "palette": {...}}`) every time the task manager opens, and the compositor keeps the last of them; the
//! Rose Pine dark tokens stand until one arrives.

use serde_json::Value;

/// Five colours, `[r, g, b]`, 0 to 255.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// What the screen is made of: the dim behind everything, and the sheets.
    pub panel: [u8; 3],
    /// Titles and values.
    pub text: [u8; 3],
    /// Hints, labels, the tracks of the bars.
    pub muted: [u8; 3],
    /// The focus and what is chosen.
    pub accent: [u8; 3],
    /// What is wrong (an app that does not answer) and what cannot be undone (a restart).
    pub danger: [u8; 3],
}

impl Default for Palette {
    /// Rose Pine, dark: base, text, subtle, iris, love.
    fn default() -> Palette {
        Palette { panel: [0x19, 0x17, 0x24], text: [0xe0, 0xde, 0xf4], muted: [0x90, 0x8c, 0xaa], accent: [0xc4, 0xa7, 0xe7], danger: [0xeb, 0x6f, 0x92] }
    }
}

impl Palette {
    /// The UI's `palette` object; `None` unless all five colours are there as three numbers each (a half palette would be a screen of two
    /// themes).
    pub fn parse(v: &Value) -> Option<Palette> {
        let rgb = |k: &str| -> Option<[u8; 3]> {
            let a = v.get(k)?.as_array()?;
            if a.len() != 3 {
                return None;
            }
            let c = |i: usize| a[i].as_f64().map(|n| n.round().clamp(0.0, 255.0) as u8);
            Some([c(0)?, c(1)?, c(2)?])
        };
        Some(Palette { panel: rgb("panel")?, text: rgb("text")?, muted: rgb("muted")?, accent: rgb("accent")?, danger: rgb("danger")? })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_whole_palette_parses_and_clamps() {
        let v = json!({"panel": [1, 2, 3], "text": [255, 255, 255], "muted": [10.4, 20.6, 300], "accent": [0, 0, 0], "danger": [9, 9, 9]});
        let p = Palette::parse(&v).unwrap();
        assert_eq!(p.panel, [1, 2, 3]);
        assert_eq!(p.muted, [10, 21, 255]);
    }

    #[test]
    fn a_palette_with_a_colour_missing_or_malformed_is_refused() {
        assert_eq!(Palette::parse(&json!({"panel": [1, 2, 3]})), None);
        let bad = json!({"panel": [1, 2], "text": [1, 2, 3], "muted": [1, 2, 3], "accent": [1, 2, 3], "danger": [1, 2, 3]});
        assert_eq!(Palette::parse(&bad), None);
        let word = json!({"panel": "#191724", "text": [1, 2, 3], "muted": [1, 2, 3], "accent": [1, 2, 3], "danger": [1, 2, 3]});
        assert_eq!(Palette::parse(&word), None);
    }
}
