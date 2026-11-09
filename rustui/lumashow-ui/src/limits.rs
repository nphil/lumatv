//! The retained-UI ceilings of the design's resource ledger (`docs/g2-shell-design.md`, section 11.2).
//!
//! These are allocation ceilings to implement and measure, **not measured memory**. They bound what the shell and the toolkit may hold
//! inside the G2 total of 90 MB (app, compositor and idle codec, PSS); they do not add to it. A preflight resource planner refuses a surface
//! expansion before it would pass one of them, with readable pagination or a summary and never a dropped essential control; raising a
//! number needs a new ledger and a PSS proof. A test reads the design and holds the byte rows and the depth to the text, so the code and
//! the document cannot drift apart unseen. The node ceilings are LumaTV's, raised above the design's first ones on 2026-11-09.

/// Drawable nodes (layers that draw something) retained at once: the compositor's `Limits::layers`. The design's first ceiling was 256;
/// the TV's shell held 178 at most (2026-11-09), and a layer costs only its record and what it draws.
pub const MAX_DRAWABLE_NODES: usize = 512;
/// Group nodes retained at once: those of the shell on show and of the one that replaces it, built before the first goes. Groups
/// themselves allocate no textures. The design's first ceiling was 96; the TV's shell held 68 at most (2026-11-09).
pub const MAX_GROUP_NODES: usize = 256;
/// How deep groups nest: the day line under the first screen's parts is nine levels from the home's root.
pub const MAX_GROUP_DEPTH: usize = 9;
/// Motion tracks held at once: those of the shell on show and of the one that replaces it, built before the first goes. The design's
/// first ceiling was 32; the TV's shell held 13 at most (2026-11-09).
pub const MAX_MOTION_TRACKS: usize = 64;

const MIB: usize = 1024 * 1024;

/// The active still wallpaper and one prepared replacement, decoded at 960x480x4 each (3.52 MiB).
pub const WALLPAPER_BYTES: usize = 2 * 960 * 480 * 4;
/// The prepared blur bundle: two generations and scratch, quarter-resolution plates with margins.
pub const BLUR_BUNDLE_BYTES: usize = 2 * MIB;
/// The font CPU cache and the GPU atlases together.
pub const FONT_BYTES: usize = 8 * MIB;
/// The visible surfaces and their neighbours: timeline, cards, settings and notifications, with a reservation each.
pub const SURFACE_BYTES: usize = 12 * MIB;
/// Transition and scratch staging: one route transition and one pending theme generation.
pub const STAGING_BYTES: usize = 4 * MIB;
/// Settings, timeline, card and notification models and the command rings.
pub const MODEL_BYTES: usize = 4 * MIB;
/// Audio samples and rings, none duplicated per card or alarm.
pub const AUDIO_BYTES: usize = 2 * MIB;
/// What the app, the compositor and the idle codec may hold together after ten minutes idle (proportional set size).
pub const TOTAL_PSS_BYTES: usize = 90 * MIB;

#[cfg(test)]
mod tests {
    use super::*;

    /// The design with its line breaks and runs of spaces folded to single spaces, so that a phrase can be looked for across a wrap.
    fn design() -> String {
        include_str!("../../../docs/g2-shell-design.md")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn the_group_depth_is_the_one_the_design_states() {
        let doc = design();
        let depth = format!("Maximum depth {MAX_GROUP_DEPTH};");
        assert!(doc.contains(&depth), "the design no longer says: {depth}");
    }

    #[test]
    fn the_byte_ceilings_are_the_ledger_rows_of_the_design() {
        let doc = design();
        let mib = |bytes: usize| bytes as f64 / MIB as f64;
        assert_eq!((mib(WALLPAPER_BYTES) * 100.0).round() / 100.0, 3.52);
        for (bytes, phrase) in [
            (BLUR_BUNDLE_BYTES, "<=2 MiB total;"),
            (FONT_BYTES, "<=8 MiB combined;"),
            (SURFACE_BYTES, "<=12 MiB total,"),
            (STAGING_BYTES, "<=4 MiB; at most one route transition"),
            (MODEL_BYTES, "<=4 MiB; caps above enforced"),
            (AUDIO_BYTES, "<=2 MiB, none duplicated"),
        ] {
            assert!(
                doc.contains(phrase),
                "the ledger no longer says {phrase:?} ({} MiB)",
                mib(bytes)
            );
        }
        assert!(doc.contains("= 3.52 MiB decoded"));
        assert!(doc.contains(&format!("PSS <={} MB for G2", TOTAL_PSS_BYTES / MIB)));
    }

    #[test]
    fn the_ledger_rows_fit_inside_the_total_with_room_for_the_compositor_and_the_codec() {
        let rows = WALLPAPER_BYTES
            + BLUR_BUNDLE_BYTES
            + FONT_BYTES
            + SURFACE_BYTES
            + STAGING_BYTES
            + MODEL_BYTES
            + AUDIO_BYTES;
        assert!(
            rows < TOTAL_PSS_BYTES / 2,
            "{} MiB of ceilings is over half the total",
            rows / MIB
        );
    }
}
