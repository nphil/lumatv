//! The steps of the display lab, in the order the person goes through them, with the few words each one says. Every step is one small
//! question about one thing the display chip can do: two options of a control the helper owns (`helper/app/src/displaylab/allow.rs` is the
//! list; the options named here are held to it by a test), a test card with the zone to look at marked, a line on what the difference looks like,
//! and what is asked: which of the two looks better, or whether one look is acceptable.
//!
//! Skipped on purpose (`docs/research/display-lab-result.md`): the smaller composed page (the driver refused it) and the orbit (measured, not
//! judged by eye).

use ui::geometry::Frame;

/// An option of a control: the id the helper knows it by and the name the person reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opt {
    pub id: &'static str,
    pub name: &'static str,
}

/// What a step asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Which of the two looks better: A, B or neither.
    Compare,
    /// Whether the second look is acceptable, in these words.
    Question(&'static str),
}

/// The test card a step shows (`card.rs` paints it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Card {
    /// Small letters, black on white and white on black.
    Text,
    /// Lines one pixel wide, and checkerboards.
    Lines,
    /// Letters and colour patches on a ramp of greys.
    Colour,
    /// What the screensaver shows: a clock, a date and a picture.
    Saver,
    /// Saturated colours and a sweep of hues.
    Hues,
    /// Bold stripes with the place a window is expected marked.
    Window,
}

impl Card {
    /// The zone the step asks the person to look at, in the 1920 by 1080 page: left of where the panel stands at any text size, and clear
    /// of its arrow.
    pub fn zone(self) -> Frame {
        match self {
            Card::Text => Frame::new(72.0, 150.0, 760.0, 600.0),
            Card::Lines => Frame::new(72.0, 150.0, 760.0, 600.0),
            Card::Colour => Frame::new(72.0, 150.0, 760.0, 600.0),
            Card::Saver => Frame::new(72.0, 150.0, 760.0, 600.0),
            Card::Hues => Frame::new(72.0, 150.0, 760.0, 600.0),
            // where the hole of `window` is expected: x 1440, width 960, y 810 and height 540 of the mixer's 3840 by 2160, halved
            Card::Window => Frame::new(720.0, 405.0, 480.0, 270.0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StepDef {
    /// The id the results name it by.
    pub id: &'static str,
    /// The title the panel shows, and this step's place among the steps of that title.
    pub group: &'static str,
    pub nth: u8,
    pub of: u8,
    /// The helper's control, and its two options: A is the set's own state.
    pub control: &'static str,
    pub a: Opt,
    pub b: Opt,
    pub card: Card,
    /// Where to look, in a few words: what the tag on the frame says.
    pub look: &'static str,
    /// What the difference looks like.
    pub hint: &'static str,
    pub ask: Ask,
}

const NORMAL: Opt = Opt { id: "normal", name: "Normal" };
const WINDOW_OFF: Opt = Opt { id: "off", name: "Window off" };

pub const STEPS: [StepDef; 6] = [
    StepDef {
        id: "sharpness-strong",
        group: "Sharpness",
        nth: 1,
        of: 5,
        control: "sharpness",
        a: NORMAL,
        b: Opt { id: "strong", name: "Strong" },
        card: Card::Text,
        look: "Small letters",
        hint: "Sharper edges, or bright halos.",
        ask: Ask::Compare,
    },
    StepDef {
        id: "sharpness-medium",
        group: "Sharpness",
        nth: 2,
        of: 5,
        control: "sharpness",
        a: NORMAL,
        b: Opt { id: "medium", name: "Medium" },
        card: Card::Text,
        look: "Small letters",
        hint: "Crisper edges. Jagged up close?",
        ask: Ask::Compare,
    },
    StepDef {
        id: "sharpness-medium-strong",
        group: "Sharpness",
        nth: 3,
        of: 5,
        control: "sharpness",
        a: Opt { id: "medium", name: "Medium" },
        b: Opt { id: "strong", name: "Strong" },
        card: Card::Text,
        look: "Small letters",
        hint: "Crisp, without jagged edges?",
        ask: Ask::Compare,
    },
    StepDef {
        id: "sharpness-soft",
        group: "Sharpness",
        nth: 4,
        of: 5,
        control: "sharpness",
        a: NORMAL,
        b: Opt { id: "soft", name: "Soft" },
        card: Card::Lines,
        look: "Thin lines",
        hint: "Lines turn fainter and blurrier.",
        ask: Ask::Compare,
    },
    StepDef {
        id: "sharpness-off",
        group: "Sharpness",
        nth: 5,
        of: 5,
        control: "sharpness",
        a: NORMAL,
        b: Opt { id: "off", name: "Sharpener off" },
        card: Card::Colour,
        look: "Letters and colours",
        hint: "Plainer letters, colours may shift.",
        ask: Ask::Compare,
    },
    StepDef {
        id: "window",
        group: "Window",
        nth: 1,
        of: 1,
        control: "window",
        a: WINDOW_OFF,
        b: Opt { id: "on", name: "Window on" },
        card: Card::Window,
        look: "The window",
        hint: "The picture behind shows through.",
        ask: Ask::Question("Does the window show the picture cleanly?"),
    },
];

pub fn step_index(id: &str) -> Option<usize> {
    STEPS.iter().position(|s| s.id == id)
}

/// What a person can answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The option of this index (0: A, 1: B) looks better.
    Better(usize),
    Yes,
    No,
    /// Could not tell the two apart.
    Same,
    Skipped,
    /// The look could not be put on the screen (the helper refused it, or the firmware took it back at once): nothing was compared.
    Failed,
}

impl Answer {
    /// The word the results file keeps.
    pub fn word(self) -> &'static str {
        match self {
            Answer::Better(0) => "a",
            Answer::Better(_) => "b",
            Answer::Yes => "yes",
            Answer::No => "no",
            Answer::Same => "same",
            Answer::Skipped => "skipped",
            Answer::Failed => "failed",
        }
    }

    pub fn from_word(word: &str) -> Option<Answer> {
        Some(match word {
            "a" => Answer::Better(0),
            "b" => Answer::Better(1),
            "yes" => Answer::Yes,
            "no" => Answer::No,
            "same" => Answer::Same,
            "skipped" => Answer::Skipped,
            "failed" => Answer::Failed,
            _ => return None,
        })
    }
}

impl StepDef {
    /// The step's place in the panel's counter: "1 of 3"; a group of one steps has nothing to count.
    pub fn counter(&self) -> String {
        if self.of <= 1 { String::new() } else { format!("{} of {}", self.nth, self.of) }
    }

    /// What an answer to this step says, as the results screen reads it.
    pub fn verdict(&self, a: Answer) -> String {
        match (self.ask, a) {
            (_, Answer::Skipped) => "Skipped".to_owned(),
            (_, Answer::Failed) => "Couldn't apply".to_owned(),
            (_, Answer::Same) => "Can't tell".to_owned(),
            (Ask::Compare, Answer::Better(i)) => format!("{} looks better", if i == 0 { self.a.name } else { self.b.name }),
            (Ask::Question(_), Answer::Yes) => "OK".to_owned(),
            (Ask::Question(_), Answer::No) => "Not OK".to_owned(),
            // an answer the step does not ask for is not one
            _ => "Skipped".to_owned(),
        }
    }

    /// The row of the results: what was compared, in a few words.
    pub fn row(&self) -> String {
        format!("{} \u{b7} {}", self.group, self.b.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_are_the_assignments_in_its_order_and_each_is_whole() {
        let ids: Vec<&str> = STEPS.iter().map(|s| s.id).collect();
        assert_eq!(ids, ["sharpness-strong", "sharpness-medium", "sharpness-medium-strong", "sharpness-soft", "sharpness-off", "window"]);
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
        for s in &STEPS {
            assert!(!s.look.is_empty() && !s.hint.is_empty() && !s.a.name.is_empty() && !s.b.name.is_empty(), "{}", s.id);
            assert_ne!(s.a.id, s.b.id, "{}", s.id);
            // a few words: the tag is one short phrase and the hint one line at the sheet's width
            assert!(s.look.len() <= 24 && s.hint.len() <= 40, "{}: {} / {}", s.id, s.look.len(), s.hint.len());
            if let Ask::Question(q) = s.ask {
                assert!(q.ends_with('?') && q.len() <= 56, "{}", s.id);
            }
            // a counter counts the steps of its title, from one
            assert!(s.nth >= 1 && s.nth <= s.of, "{}", s.id);
            assert_eq!(STEPS.iter().filter(|o| o.group == s.group).count(), usize::from(s.of), "{}", s.id);
        }
        // the sharpness steps are comparisons, the window asks whether a look is acceptable
        assert!(STEPS[..5].iter().all(|s| s.ask == Ask::Compare && s.group == "Sharpness"));
        assert!(STEPS[5..].iter().all(|s| matches!(s.ask, Ask::Question(_)) && s.group == "Window"));
        // one window step: a group of one is not counted
        assert_eq!(STEPS[5].counter(), "");
        // OSD0's dimming is the firmware's (it puts its own value back at once, and grey showed nothing on LumaTV's plane): no step offers it
        assert!(STEPS.iter().all(|s| s.control != "dimming"));
        assert_eq!(step_index("nothing"), None);
    }

    #[test]
    fn the_words_say_what_the_assignment_asks() {
        let by = |id: &str| &STEPS[step_index(id).unwrap()];
        assert_eq!((by("sharpness-strong").b.id, by("sharpness-soft").b.id, by("sharpness-off").b.id), ("strong", "soft", "off"));
        // Normal, Medium and Strong can all be set against each other: each against Normal, and the two of them together
        assert_eq!((by("sharpness-medium").a.id, by("sharpness-medium").b.id), ("normal", "medium"));
        assert_eq!((by("sharpness-medium-strong").a.id, by("sharpness-medium-strong").b.id), ("medium", "strong"));
        assert_eq!(by("window").ask, Ask::Question("Does the window show the picture cleanly?"));
        assert!(by("sharpness-strong").hint.contains("Sharper edges") && by("sharpness-strong").hint.contains("halos"));
    }

    #[test]
    fn a_zone_is_clear_of_the_panel_and_inside_the_page() {
        for card in [Card::Text, Card::Lines, Card::Colour, Card::Saver, Card::Hues, Card::Window] {
            let z = card.zone();
            assert!(z.x >= 64.0 && z.y >= 54.0 && z.x + z.w <= 1264.0 - 24.0 && z.y + z.h <= 1026.0, "{card:?}: {z:?}");
        }
        // the window's zone is the middle of the page
        let w = Card::Window.zone();
        assert_eq!((w.x + w.w / 2.0, w.y + w.h / 2.0), (960.0, 540.0));
    }

    #[test]
    fn an_answer_has_a_word_in_the_results_and_a_verdict_on_the_results_screen() {
        for a in [Answer::Better(0), Answer::Better(1), Answer::Yes, Answer::No, Answer::Same, Answer::Skipped, Answer::Failed] {
            assert_eq!(Answer::from_word(a.word()), Some(a));
        }
        assert_eq!(Answer::from_word("maybe"), None);
        let strong = &STEPS[0];
        assert_eq!(strong.verdict(Answer::Better(0)), "Normal looks better");
        assert_eq!(strong.verdict(Answer::Better(1)), "Strong looks better");
        assert_eq!(strong.verdict(Answer::Same), "Can't tell");
        assert_eq!(strong.verdict(Answer::Skipped), "Skipped");
        assert_eq!(strong.verdict(Answer::Yes), "Skipped", "a compare step does not ask yes or no");
        assert_eq!(strong.verdict(Answer::Failed), "Couldn't apply", "a look that never showed is not an answer about it");
        let window = &STEPS[5];
        assert_eq!((window.verdict(Answer::Yes).as_str(), window.verdict(Answer::No).as_str()), ("OK", "Not OK"));
        assert_eq!(window.verdict(Answer::Better(1)), "Skipped");
        assert_eq!(strong.row(), "Sharpness \u{b7} Strong");
        assert_eq!(strong.counter(), "1 of 5");
        assert_eq!(STEPS[2].verdict(Answer::Better(0)), "Medium looks better");
    }

    /// The options the shell names are options of the helper's list (`helper/app/src/displaylab/allow.rs`), and the first of each control is
    /// the A of every step that uses it.
    #[test]
    fn every_option_is_one_the_helpers_list_has() {
        let allow = include_str!("../../../../helper/app/src/displaylab/allow.rs");
        let block = |name: &str| -> Vec<String> {
            let start = allow.find(&format!("const {name}: &[Opt] = &[")).unwrap_or_else(|| panic!("no {name} in the list"));
            let rest = &allow[start..];
            let end = rest.find("];").unwrap();
            rest[..end].split("id: \"").skip(1).map(|p| p.split('"').next().unwrap().to_owned()).collect()
        };
        for (control, list) in [("sharpness", "SHARPNESS"), ("window", "WINDOW")] {
            let options = block(list);
            assert!(!options.is_empty(), "{control}");
            for s in STEPS.iter().filter(|s| s.control == control) {
                assert!(options.iter().any(|o| o == s.a.id) && options.iter().any(|o| o == s.b.id), "{}: {options:?}", s.id);
                // A is the set's own state, except where two looks that are not it are set against each other
                assert!(s.a.id == options[0] || s.id == "sharpness-medium-strong", "{}: A is the set's own state", s.id);
            }
        }
        assert!(STEPS.iter().all(|s| ["sharpness", "window"].contains(&s.control)));
    }
}
