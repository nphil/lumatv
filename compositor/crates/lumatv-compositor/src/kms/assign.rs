//! Which plane shows what this frame. Pure, so the decisions are tested without a device.
//!
//! One rule for now: a single opaque full-screen client buffer whose layout the primary plane takes is scanned
//! out directly (no GPU pass); anything else is composed into a page of the ring on the primary plane. The
//! overlay plane is left for C3's split mode (UI above a direct-scanned app), which needs X6's answer on whether
//! osd1 scales like osd0.

use crate::Rect;

/// What a plane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Page `i` of the compositor's own ring.
    Page(usize),
    /// A client buffer's framebuffer id.
    Direct(u32),
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub primary: Source,
    /// The overlay plane's source and output rectangle, if used.
    pub overlay: Option<(Source, Rect)>,
}

/// The facts the decision needs, gathered from the plan and the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    /// The plan is one opaque item covering the output with no holes.
    pub sole_fullscreen: bool,
    /// That item's buffer is a dma-buf whose (format, modifier) the primary plane lists.
    pub layout_accepted: bool,
    /// Its framebuffer id once imported.
    pub direct_fb: Option<u32>,
    /// The ring page that would be composed into.
    pub next_page: usize,
}

pub fn decide(f: &Facts) -> Assignment {
    match (f.sole_fullscreen && f.layout_accepted, f.direct_fb) {
        (true, Some(fb)) => Assignment { primary: Source::Direct(fb), overlay: None },
        _ => Assignment { primary: Source::Page(f.next_page), overlay: None },
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

    #[test]
    fn a_full_screen_client_buffer_the_plane_takes_is_scanned_out_directly() {
        let f = Facts { sole_fullscreen: true, layout_accepted: true, direct_fb: Some(77), next_page: 1 };
        assert_eq!(decide(&f), Assignment { primary: Source::Direct(77), overlay: None });
        assert!(needs_commit(&decide(&f), false));
    }

    #[test]
    fn anything_else_is_composed_into_the_next_page() {
        let f = Facts { sole_fullscreen: true, layout_accepted: false, direct_fb: None, next_page: 2 };
        assert_eq!(decide(&f).primary, Source::Page(2));
        let f = Facts { sole_fullscreen: false, layout_accepted: true, direct_fb: Some(5), next_page: 0 };
        assert_eq!(decide(&f).primary, Source::Page(0));
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
