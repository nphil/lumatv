//! The date in Home's top bar: its words, the owner's choice of where along the bar it stands (`layout.statusBar.date` of the theme, a profile's
//! own key), and the place that choice gives it among what else the bar holds.

use super::clock::Civil;

/// Where the date stands: along the bar's row, in the clock card as it was before the bar had it, or nowhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Left,
    Centre,
    Right,
    Hidden,
    Card,
}

impl Place {
    pub const KEY: &'static str = "date";
    pub const CHOICES: [Place; 5] = [Place::Centre, Place::Left, Place::Right, Place::Hidden, Place::Card];
    pub const WORDS: [&'static str; 5] = ["Centre", "Left", "Right", "Hidden", "In the clock card"];

    /// The middle of the bar until chosen.
    pub fn of(key: Option<&str>) -> Self {
        match key {
            Some("left") => Self::Left,
            Some("right") => Self::Right,
            Some("hidden") => Self::Hidden,
            Some("clock") => Self::Card,
            _ => Self::Centre,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Centre => "centre",
            Self::Right => "right",
            Self::Hidden => "hidden",
            Self::Card => "clock",
        }
    }
}

/// "Monday, November 23".
pub fn words(t: &Civil) -> String {
    format!("{}, {}", t.weekday_name(), t.date_line())
}

/// The left edge of a date `width` wide in the row `(x, w)`: at the end `place` names, or where it is clear of the spans `taken` (`(from, to)` along the row,
/// each kept `gap` away) as near to that end as it can be; `None` with the date hidden, kept in the card, or with no clear place in the row.
pub fn slot(place: Place, row: (f32, f32), width: f32, taken: &[(f32, f32)], gap: f32) -> Option<f32> {
    let (x, w) = row;
    let want = match place {
        Place::Left => x,
        Place::Centre => x + ((w - width) / 2.0).round(),
        Place::Right => x + w - width,
        Place::Hidden | Place::Card => return None,
    };
    let clear = |at: f32| at >= x && at + width <= x + w && taken.iter().all(|&(a, b)| at + width + gap <= a || at >= b + gap);
    if clear(want) {
        return Some(want);
    }
    taken
        .iter()
        .flat_map(|&(a, b)| [a - gap - width, b + gap])
        .filter(|&at| clear(at))
        .min_by(|p, q| (p - want).abs().total_cmp(&(q - want).abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: (f32, f32) = (64.0, 1792.0);

    #[test]
    fn the_date_stands_where_the_owner_puts_it_along_the_bar() {
        assert_eq!(slot(Place::Centre, ROW, 400.0, &[], 24.0), Some(760.0));
        assert_eq!(slot(Place::Left, ROW, 400.0, &[], 24.0), Some(64.0));
        assert_eq!(slot(Place::Right, ROW, 400.0, &[], 24.0), Some(1456.0));
        assert_eq!(slot(Place::Hidden, ROW, 400.0, &[], 24.0), None);
        assert_eq!(slot(Place::Card, ROW, 400.0, &[], 24.0), None);
        assert_eq!([None, Some("centre"), Some("left"), Some("right"), Some("hidden"), Some("clock")].map(Place::of), [Place::Centre, Place::Centre, Place::Left, Place::Right, Place::Hidden, Place::Card]);
        assert!(Place::CHOICES.iter().all(|p| Place::of(Some(p.key())) == *p));
    }

    #[test]
    fn the_date_keeps_clear_of_the_bars_buttons_and_glyphs_and_goes_when_there_is_no_room() {
        // the chip at the left end and the buttons at the right: the middle is free
        assert_eq!(slot(Place::Centre, ROW, 400.0, &[(64.0, 220.0), (1500.0, 1856.0)], 24.0), Some(760.0));
        // the buttons in the middle (the bar's default place): the date slides to the nearer side of them, whichever end it asked for
        let middle = [(800.0, 1120.0)];
        let at = slot(Place::Centre, ROW, 400.0, &middle, 24.0).expect("a side of the buttons");
        assert!(at + 400.0 + 24.0 <= 800.0 || at >= 1120.0 + 24.0);
        assert_eq!(slot(Place::Right, ROW, 400.0, &middle, 24.0), Some(1456.0), "already clear at its end");
        // buttons and a strip across the whole row leave nowhere
        assert_eq!(slot(Place::Centre, ROW, 400.0, &[(64.0, 1856.0)], 24.0), None);
        assert_eq!(slot(Place::Centre, ROW, 2000.0, &[], 24.0), None, "wider than the row");
    }
}
