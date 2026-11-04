//! What a row says in its cells: where a title goes and what of it is said, one rule for every cell of every row.
//!
//! A cell speaks in the part of itself the screen shows, kept `pad` from both ends of that part: never under the grid's edge, never past the
//! cell's own end, so never on the next cell's words or on the divider between them. A title that does not fit steps down to its beginning and
//! an ellipsis, and a cell with too little room says nothing: a cell of five minutes is a divider and a plate, not a word cut off. A cell that
//! begins left of the screen has its title pinned to the screen's edge (the lead), and so has one that begins in the clearance the view keeps
//! the focus off the edge by, so that every title of the first column begins in one place. Everything here is measured in the pixels of the
//! grid's content and knows nothing of fonts or of the scene, so a matrix of cells, views and widths is checked in microseconds.

/// What the screen shows of the grid's content: `left..right`, and how far in from `left` titles begin (the view's clearance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub left: f32,
    pub right: f32,
    pub lead: f32,
}

/// Where the title of a cell goes, in the content of the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    /// The left edge of the words and the room they have.
    pub x: f32,
    pub room: f32,
    /// The cell begins at or left of where titles begin: its title stays there, at the screen's edge.
    pub pinned: bool,
}

/// The slot of the title of a cell `start..end` on `screen`: the part of the cell the screen shows from where titles begin, less `pad` at
/// each end, or nothing when nothing is left of it. A cell the screen does not reach (it only goes by in a flight) has all of itself.
pub fn slot(start: f32, end: f32, screen: Screen, pad: f32) -> Option<Slot> {
    let edge = screen.left + screen.lead;
    let reached = start < screen.right && end > screen.left;
    let (a, b) = if reached { (start.max(edge), end.min(screen.right)) } else { (start, end) };
    let room = b - a - 2.0 * pad;
    (room > 0.0).then_some(Slot { x: a + pad, room, pinned: reached && start < edge })
}

/// The fewest characters a cut title may keep: a shorter beginning is no word, and the cell says nothing.
pub const MIN_CUT: usize = 5;

/// `text` as it is said in `room`: whole when it fits, else its beginning and an ellipsis (at least [`MIN_CUT`] characters of it, and never a
/// space before the ellipsis), else nothing. `width` measures a run of words.
pub fn fit(text: &str, room: f32, width: &mut dyn FnMut(&str) -> f32) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    if width(text) <= room {
        return Some(text.to_owned());
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut used, mut keep) = (width("\u{2026}"), 0);
    for (i, c) in chars.iter().enumerate() {
        used += width(&c.to_string());
        if used > room {
            break;
        }
        keep = i + 1;
    }
    while keep > 0 && chars[keep - 1] == ' ' {
        keep -= 1;
    }
    (keep >= MIN_CUT).then(|| {
        let mut cut: String = chars[..keep].iter().collect();
        cut.push('\u{2026}');
        cut
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twelve pixels a character, the ellipsis too: a ruler that needs no font.
    fn width(t: &str) -> f32 {
        12.0 * t.chars().count() as f32
    }

    fn on(left: f32, right: f32, lead: f32) -> Screen {
        Screen { left, right, lead }
    }

    #[test]
    fn a_cell_speaks_in_the_part_of_itself_the_screen_shows_kept_pad_from_both_ends() {
        let screen = on(0.0, 1000.0, 0.0);
        // whole on the screen
        assert_eq!(slot(100.0, 400.0, screen, 12.0), Some(Slot { x: 112.0, room: 276.0, pinned: false }));
        // begins left of the screen: the title is pinned to its edge and the room is what shows
        assert_eq!(slot(-300.0, 200.0, screen, 12.0), Some(Slot { x: 12.0, room: 176.0, pinned: true }));
        // ends right of the screen: the room stops at the edge
        assert_eq!(slot(900.0, 1500.0, screen, 12.0), Some(Slot { x: 912.0, room: 76.0, pinned: false }));
        // wider than the screen at both ends
        assert_eq!(slot(-50.0, 2000.0, screen, 12.0), Some(Slot { x: 12.0, room: 976.0, pinned: true }));
        // less of it shows than the air on both sides takes: no slot, not a speck of a letter at the edge
        assert_eq!(slot(990.0, 1500.0, screen, 12.0), None);
        assert_eq!(slot(-500.0, 20.0, screen, 12.0), None);
        // beyond the screen, as in a flight: all of the cell
        assert_eq!(slot(1200.0, 1500.0, screen, 12.0), Some(Slot { x: 1212.0, room: 276.0, pinned: false }));
    }

    #[test]
    fn a_cell_that_begins_in_the_clearance_has_its_title_where_a_cell_that_began_before_the_screen_does() {
        let screen = on(100.0, 1100.0, 14.0);
        let before = slot(-200.0, 500.0, screen, 12.0).unwrap();
        let inside = slot(105.0, 500.0, screen, 12.0).unwrap();
        let at_the_edge = slot(100.0, 500.0, screen, 12.0).unwrap();
        assert_eq!((before.x, inside.x, at_the_edge.x), (126.0, 126.0, 126.0), "114 and the air");
        assert!(before.pinned && inside.pinned && at_the_edge.pinned);
        // one that begins where titles do is its own: its title is the pad from its divider
        let own = slot(114.0, 500.0, screen, 12.0).unwrap();
        assert_eq!((own.x, own.pinned), (126.0, false));
        let after = slot(130.0, 500.0, screen, 12.0).unwrap();
        assert_eq!((after.x, after.pinned), (142.0, false));
        // one that ends inside the clearance has no room
        assert_eq!(slot(80.0, 110.0, screen, 12.0), None);
    }

    #[test]
    fn a_title_that_fits_is_said_whole_and_one_that_does_not_steps_down_to_its_beginning_or_to_nothing() {
        assert_eq!(fit("Weather", 100.0, &mut width).as_deref(), Some("Weather"));
        assert_eq!(fit("Evening News Tonight", 140.0, &mut width).as_deref(), Some("Evening Ne\u{2026}"));
        // a space is never left before the ellipsis
        assert_eq!(fit("Evening News Tonight", 108.0, &mut width).as_deref(), Some("Evening\u{2026}"));
        // five characters of it at least
        assert_eq!(fit("Evening News Tonight", 72.0, &mut width).as_deref(), Some("Eveni\u{2026}"));
        assert_eq!(fit("Evening News Tonight", 71.0, &mut width), None);
        assert_eq!(fit("A", 12.0, &mut width).as_deref(), Some("A"), "a short title is said whole in what room it needs");
        assert_eq!(fit("A", 11.0, &mut width), None);
        assert_eq!(fit("", 500.0, &mut width), None);
    }

    #[test]
    fn across_cells_views_and_widths_no_title_leaves_its_cell_or_the_screen_or_comes_within_two_pads_of_the_next() {
        let (pad, lead) = (12.0, 14.0);
        let titles = ["A", "News", "The Big Valley", "Law & Order: Special Victims Unit", "The Extraordinarily Long Documentary About The Rivers"];
        for ppm in [8.0_f32, 12.0, 16.0] {
            let screen = 176.0 * ppm;
            for minutes in [5.0_f32, 10.0, 15.0, 30.0, 45.0, 60.0, 120.0, 400.0] {
                for shift in (-200..=200).step_by(7) {
                    // a row of cells of `minutes` each, the first begun `shift` minutes before the screen's left edge
                    let mut words: Vec<(f32, f32, f32, f32)> = Vec::new();
                    for k in 0..40 {
                        let (start, end) = ((shift as f32 + k as f32 * minutes) * ppm, (shift as f32 + (k + 1) as f32 * minutes) * ppm);
                        let Some(s) = slot(start, end, on(0.0, screen, lead), pad) else { continue };
                        let reached = start < screen && end > 0.0;
                        let (lo, hi) = if reached { (start.max(lead), end.min(screen)) } else { (start, end) };
                        for title in titles {
                            let Some(shown) = fit(title, s.room, &mut width) else { continue };
                            let (a, b) = (s.x, s.x + width(&shown));
                            assert!(a >= lo + pad - 0.01 && b <= hi - pad + 0.01, "{shown:?} {a}..{b} is not inside the cell's {lo}..{hi} less {pad} ({minutes} min at {shift})");
                            if title == titles[3] {
                                words.push((start, end, a, b));
                            }
                        }
                    }
                    for pair in words.windows(2) {
                        let ((_, end, _, b), (start, _, a, _)) = (pair[0], pair[1]);
                        if (end - start).abs() < 0.01 {
                            assert!(a - b >= 2.0 * pad - 0.01, "the words of neighbours are {} apart ({minutes} min at {shift})", a - b);
                        }
                    }
                }
            }
        }
    }
}
