//! Which plane shows what this frame. Pure, so the decisions are tested without a device.
//!
//! The planes stack in hardware order, primary under overlay, with no zpos and no plane alpha (`kms/mod.rs`). So a
//! single opaque full-screen client buffer whose layout the primary plane takes is scanned out there directly (no GPU
//! pass), and when the system UI shows something over it, the UI alone is drawn into a page with alpha on the overlay
//! plane (the split: a volume bar over Netflix costs no composition of the video). Anything else is composed into a page
//! of the ring on the primary plane. Whether the overlay's per-pixel alpha blends over the primary is for the set to
//! answer [INFERENCE: the OSD planes take AR24, which is only worth listing for that].

use crate::Rect;

/// What a plane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Page `i` of the compositor's own ring (opaque, the primary plane).
    Page(usize),
    /// Page `i` of the UI ring (with alpha, the overlay plane).
    UiPage(usize),
    /// A client buffer's framebuffer id.
    Direct(u32),
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub primary: Source,
    /// The overlay plane's source and output rectangle, if used.
    pub overlay: Option<(Source, Rect)>,
    /// Where the primary plane puts a direct frame: the OLED care's orbit (`scanout::Policy::offset`), (0, 0) at the origin and for every
    /// composed page.
    pub at: (i32, i32),
}

/// The facts the decision needs, gathered from the plan and the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    /// The plan is one opaque item covering the output with no holes.
    pub sole_fullscreen: bool,
    /// That item's buffer is a dma-buf whose (format, modifier) the primary plane lists.
    pub layout_accepted: bool,
    /// Its framebuffer id once imported, and where the plane is to put it.
    pub direct_fb: Option<(u32, (i32, i32))>,
    /// The ring page that would be composed into.
    pub next_page: usize,
    /// The system UI shows something over the windows this frame.
    pub ui_over: bool,
    /// The device has an overlay plane that takes the UI ring's format, and the ring exists; with the page it would be
    /// drawn into.
    pub ui_page: Option<usize>,
    /// The output's size, the overlay's rectangle in the split.
    pub output: Rect,
}

pub fn decide(f: &Facts) -> Assignment {
    let direct = if f.sole_fullscreen && f.layout_accepted { f.direct_fb } else { None };
    match (direct, f.ui_over, f.ui_page) {
        (Some((fb, at)), false, _) => Assignment { primary: Source::Direct(fb), overlay: None, at },
        (Some((fb, at)), true, Some(page)) => Assignment { primary: Source::Direct(fb), overlay: Some((Source::UiPage(page), f.output)), at },
        _ => Assignment { primary: Source::Page(f.next_page), overlay: None, at: (0, 0) },
    }
}

/// Whether the plan changed anything a commit must carry: a direct frame always does (a new buffer), a composed
/// frame only when something was drawn.
pub fn needs_commit(a: &Assignment, drawn: bool) -> bool {
    matches!(a.primary, Source::Direct(_)) || drawn
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            sole_fullscreen: true,
            layout_accepted: true,
            direct_fb: Some((77, (0, 0))),
            next_page: 1,
            ui_over: false,
            ui_page: Some(2),
            output: Rect::new(0, 0, 1920, 1080),
        }
    }

    #[test]
    fn a_full_screen_client_buffer_the_plane_takes_is_scanned_out_directly() {
        let f = facts();
        assert_eq!(decide(&f), Assignment { primary: Source::Direct(77), overlay: None, at: (0, 0) });
        assert!(needs_commit(&decide(&f), false));
    }

    #[test]
    fn the_ui_over_a_direct_window_goes_to_the_overlay_plane() {
        let f = Facts { ui_over: true, ..facts() };
        assert_eq!(decide(&f), Assignment { primary: Source::Direct(77), overlay: Some((Source::UiPage(2), Rect::new(0, 0, 1920, 1080))), at: (0, 0) });
    }

    #[test]
    fn without_an_overlay_the_ui_over_a_window_means_composition() {
        let f = Facts { ui_over: true, ui_page: None, ..facts() };
        assert_eq!(decide(&f), Assignment { primary: Source::Page(1), overlay: None, at: (0, 0) });
    }

    #[test]
    fn a_direct_frame_keeps_the_place_the_care_gave_it_and_a_composed_one_is_at_the_origin() {
        let f = Facts { direct_fb: Some((77, (2, 3))), ..facts() };
        assert_eq!(decide(&f), Assignment { primary: Source::Direct(77), overlay: None, at: (2, 3) });
        let f = Facts { direct_fb: Some((77, (2, 3))), ui_over: true, ..facts() };
        assert_eq!(decide(&f).at, (2, 3), "the split keeps the window where it is; the UI's overlay covers the output");
        let f = Facts { direct_fb: Some((77, (2, 3))), sole_fullscreen: false, ..facts() };
        assert_eq!(decide(&f), Assignment { primary: Source::Page(1), overlay: None, at: (0, 0) });
    }

    #[test]
    fn anything_else_is_composed_into_the_next_page() {
        let f = Facts { layout_accepted: false, direct_fb: None, next_page: 2, ..facts() };
        assert_eq!(decide(&f).primary, Source::Page(2));
        let f = Facts { sole_fullscreen: false, direct_fb: Some((5, (0, 0))), next_page: 0, ui_over: true, ..facts() };
        assert_eq!(decide(&f), Assignment { primary: Source::Page(0), overlay: None, at: (0, 0) }, "a windowed scene composes the UI with it");
        assert!(!needs_commit(&decide(&f), false));
        assert!(needs_commit(&decide(&f), true));
    }

    #[test]
    fn in_formats_blob_parses_formats_per_modifier() {
        // struct drm_format_modifier_blob { version, flags, count_formats, formats_offset, count_modifiers,
        // modifiers_offset }, then formats, then { formats: u64, offset: u32, pad: u32, modifier: u64 } entries.
        let mut b = Vec::new();
        let push32 = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
        let push64 = |b: &mut Vec<u8>, v: u64| b.extend_from_slice(&v.to_le_bytes());
        push32(&mut b, 1);
        push32(&mut b, 0);
        push32(&mut b, 2); // count_formats
        push32(&mut b, 24); // formats_offset
        push32(&mut b, 2); // count_modifiers
        push32(&mut b, 32); // modifiers_offset
        push32(&mut b, 0x3432_5258); // XR24
        push32(&mut b, 0x3432_4258); // XB24
        // linear: both formats
        push64(&mut b, 0b11);
        push32(&mut b, 0);
        push32(&mut b, 0);
        push64(&mut b, 0);
        // AFBC 32x8 split sparse: XB24 only
        push64(&mut b, 0b10);
        push32(&mut b, 0);
        push32(&mut b, 0);
        push64(&mut b, 0x0800_0000_0000_0062);
        let parsed = super::super::parse_in_formats(&b);
        assert_eq!(parsed, vec![(0x3432_5258, 0), (0x3432_4258, 0), (0x3432_4258, 0x0800_0000_0000_0062)]);
    }
}
