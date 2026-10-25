//! JavaScript number behaviour the QML widgets rely on and Rust does not share.

/// JavaScript's `Math.round`: halves round toward positive infinity (`-2.5` gives `-2`, `2.5` gives `3`), unlike
/// `f64::round`, which rounds halves away from zero. A negative half-step that rounds to zero prints as `0`, not `-0`.
pub(crate) fn js_round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// A number as JavaScript string concatenation prints it: an integral value has no fraction (`72`, not `72.0`).
/// Rust's `Display` for `f64` already does exactly that.
pub(crate) fn fmt_num(x: f64) -> String {
    format!("{x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_matches_math_round() {
        let cases = [
            (2.5, 3),
            (-2.5, -2),
            (-0.4, 0),
            (-0.5, 0),
            (-0.6, -1),
            (0.49, 0),
            (0.5, 1),
            (54.2, 54),
            (49.7, 50),
            (7.8, 8),
            (-1.5, -1),
            (-1.51, -2),
            (77.5, 78),
        ];
        for (x, want) in cases {
            assert_eq!(js_round(x), want, "js_round({x})");
        }
    }

    #[test]
    fn numbers_print_like_javascript() {
        assert_eq!(fmt_num(72.0), "72");
        assert_eq!(fmt_num(72.5), "72.5");
        assert_eq!(fmt_num(-84.4702), "-84.4702");
        assert_eq!(fmt_num(33.723), "33.723");
        assert_eq!(fmt_num(0.0), "0");
    }
}
