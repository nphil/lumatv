//! The pop-up policy: whether an event from Scrypted becomes a card on the TV. A pure function of the owner's choices, the event and the
//! time (design 3.5), so the Show can adopt the same rules later and every rule is tested alone.
//!
//! An event pops up when the pop-ups are on, its kind is in its camera's *notify* list, its phase is `new`, the same camera and kind have not
//! popped within the cooldown (a doorbell ring is never held back) and the camera is one the TV shows. An `update` changes the words of a card
//! that is already up and is never a new card; an `end` is nothing. Whether the shell is free to show the card (a PIN, an alert) and whether an
//! app is playing are the shell's to judge: the card waits for the first and is told the second (`overApps`).

use std::collections::HashMap;

/// The kinds an owner may pick for a camera, in the order a list shows them. `other` never notifies.
pub const KINDS: [&str; 6] = [
    "person", "vehicle", "animal", "package", "doorbell", "motion",
];
/// What every camera notifies for until the owner picks otherwise: Nitin's cameras see birds and cats all day, so `animal` is not here.
pub const DEFAULT_KINDS: [&str; 2] = ["person", "doorbell"];
/// How long a card stays and the least cooldown between two cards of one camera and kind, from the owner's page (seconds).
pub const SECONDS: std::ops::RangeInclusive<u32> = 5..=60;
pub const COOLDOWN: std::ops::RangeInclusive<u32> = 0..=600;
/// An event older than this when it arrives is history (the bridge replays what a reconnect missed): it is kept in the list and never a card.
pub const STALE_MS: i64 = 30_000;

/// Where an event stands in the life of the object the detector tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The first time the object is seen.
    New,
    /// The same object gained a label or a better score.
    Update,
    /// It left.
    End,
}

impl Phase {
    pub fn of(word: &str) -> Self {
        match word {
            "update" => Self::Update,
            "end" => Self::End,
            _ => Self::New,
        }
    }
}

/// The owner's page, as the policy needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    pub on: bool,
    pub seconds: u32,
    pub cooldown: u32,
    pub over_apps: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            on: true,
            seconds: 12,
            cooldown: 30,
            over_apps: true,
        }
    }
}

/// Why an event made no card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// The owner turned the pop-ups off.
    Off,
    /// Its kind is not one the owner wants for this camera.
    Kind,
    /// An object that left.
    End,
    /// The same camera and kind popped up a moment ago.
    Cooldown,
    /// It happened too long ago to be news.
    Old,
    /// Do Not Disturb or a snooze held it (`dnd.rs`).
    Quiet,
}

/// What an event comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// A new card, staying this many milliseconds.
    Pop {
        lasts_ms: u64,
    },
    /// The words of a card that is up change (if one of this camera is).
    Update,
    Held(Held),
}

/// The times of the last card of each camera and kind.
#[derive(Debug, Default)]
pub struct Cooldowns {
    last: HashMap<(String, String), i64>,
}

impl Cooldowns {
    /// Forgets what is older than the longest cooldown (the map never grows past the cameras and kinds there are, but a camera that is
    /// gone should not stay).
    pub fn forget_old(&mut self, now_ms: i64) {
        self.last
            .retain(|_, at| now_ms - *at < i64::from(*COOLDOWN.end()) * 1000);
    }

    pub fn clear(&mut self) {
        self.last.clear();
    }
}

/// An event as the policy sees it.
#[derive(Clone, Copy, Debug)]
pub struct Seen<'a> {
    pub camera: &'a str,
    pub kind: &'a str,
    pub phase: Phase,
    /// How long ago it happened, in milliseconds, by the bridge's clock.
    pub age_ms: i64,
}

/// Decides one event. `notify` is the kinds the owner wants for the event's camera; `now_ms` is the helper's clock.
pub fn decide(
    rules: &Rules,
    notify: &[String],
    seen: Seen<'_>,
    now_ms: i64,
    cooldowns: &mut Cooldowns,
) -> Decision {
    let Seen {
        camera,
        kind,
        phase,
        age_ms,
    } = seen;
    if !rules.on {
        return Decision::Held(Held::Off);
    }
    if !KINDS.contains(&kind) || !notify.iter().any(|k| k == kind) {
        return Decision::Held(Held::Kind);
    }
    match phase {
        Phase::End => return Decision::Held(Held::End),
        Phase::Update => return Decision::Update,
        Phase::New => {}
    }
    if age_ms > STALE_MS {
        return Decision::Held(Held::Old);
    }
    let seconds = u64::from(rules.seconds.clamp(*SECONDS.start(), *SECONDS.end()));
    // somebody is waiting at the door: a ring is never held back and stays half as long again
    if kind == "doorbell" {
        return Decision::Pop {
            lasts_ms: seconds * 1500,
        };
    }
    let key = (camera.to_owned(), kind.to_owned());
    let cooldown_ms = i64::from(rules.cooldown.min(*COOLDOWN.end())) * 1000;
    if let Some(last) = cooldowns.last.get(&key)
        && now_ms - *last < cooldown_ms
    {
        return Decision::Held(Held::Cooldown);
    }
    cooldowns.last.insert(key, now_ms);
    Decision::Pop {
        lasts_ms: seconds * 1000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(list: &[&str]) -> Vec<String> {
        list.iter().map(|k| (*k).to_owned()).collect()
    }

    fn run(
        rules: &Rules,
        notify: &[&str],
        camera: &str,
        kind: &str,
        phase: Phase,
        now: i64,
        c: &mut Cooldowns,
    ) -> Decision {
        decide(
            rules,
            &kinds(notify),
            Seen {
                camera,
                kind,
                phase,
                age_ms: 0,
            },
            now,
            c,
        )
    }

    #[test]
    fn only_a_kind_the_owner_picked_for_the_camera_pops_up_and_a_doorbell_stays_longer() {
        let rules = Rules::default();
        let mut c = Cooldowns::default();
        let picked = ["person", "doorbell"];
        assert_eq!(
            run(&rules, &picked, "1", "vehicle", Phase::New, 0, &mut c),
            Decision::Held(Held::Kind)
        );
        assert_eq!(
            run(&rules, &picked, "1", "other", Phase::New, 0, &mut c),
            Decision::Held(Held::Kind)
        );
        assert_eq!(
            run(&rules, &picked, "1", "person", Phase::New, 0, &mut c),
            Decision::Pop { lasts_ms: 12_000 }
        );
        assert_eq!(
            run(&rules, &picked, "1", "doorbell", Phase::New, 0, &mut c),
            Decision::Pop { lasts_ms: 18_000 }
        );
        // the defaults are the person and the doorbell, and nothing else
        assert_eq!(DEFAULT_KINDS, ["person", "doorbell"]);
        // a kind that is not one the bridge has is never a card, whatever was picked
        assert_eq!(
            run(&rules, &["ghost"], "1", "ghost", Phase::New, 0, &mut c),
            Decision::Held(Held::Kind)
        );
    }

    #[test]
    fn the_same_camera_and_kind_are_held_for_the_cooldown_but_another_camera_kind_or_a_doorbell_is_not()
     {
        let rules = Rules {
            cooldown: 30,
            ..Rules::default()
        };
        let mut c = Cooldowns::default();
        let picked = ["person", "vehicle", "doorbell"];
        assert!(matches!(
            run(&rules, &picked, "a", "person", Phase::New, 0, &mut c),
            Decision::Pop { .. }
        ));
        assert_eq!(
            run(&rules, &picked, "a", "person", Phase::New, 5_000, &mut c),
            Decision::Held(Held::Cooldown)
        );
        assert!(matches!(
            run(&rules, &picked, "b", "person", Phase::New, 5_000, &mut c),
            Decision::Pop { .. }
        ));
        assert!(matches!(
            run(&rules, &picked, "a", "vehicle", Phase::New, 5_000, &mut c),
            Decision::Pop { .. }
        ));
        assert!(matches!(
            run(&rules, &picked, "a", "person", Phase::New, 30_000, &mut c),
            Decision::Pop { .. }
        ));
        // a held event does not push the cooldown back
        assert_eq!(
            run(&rules, &picked, "a", "person", Phase::New, 59_999, &mut c),
            Decision::Held(Held::Cooldown)
        );
        assert!(matches!(
            run(&rules, &picked, "a", "person", Phase::New, 60_000, &mut c),
            Decision::Pop { .. }
        ));
        // two rings in a row are two cards
        assert!(matches!(
            run(&rules, &picked, "a", "doorbell", Phase::New, 0, &mut c),
            Decision::Pop { .. }
        ));
        assert!(matches!(
            run(&rules, &picked, "a", "doorbell", Phase::New, 1, &mut c),
            Decision::Pop { .. }
        ));
        // a cooldown of nothing holds nothing back
        let none = Rules {
            cooldown: 0,
            ..rules
        };
        assert!(matches!(
            run(&none, &picked, "z", "person", Phase::New, 0, &mut c),
            Decision::Pop { .. }
        ));
        assert!(matches!(
            run(&none, &picked, "z", "person", Phase::New, 0, &mut c),
            Decision::Pop { .. }
        ));
    }

    #[test]
    fn an_update_changes_a_card_and_is_never_a_new_one_and_an_end_is_nothing() {
        let rules = Rules::default();
        let mut c = Cooldowns::default();
        let picked = ["person"];
        assert!(matches!(
            run(&rules, &picked, "a", "person", Phase::New, 0, &mut c),
            Decision::Pop { .. }
        ));
        assert_eq!(
            run(&rules, &picked, "a", "person", Phase::Update, 1_000, &mut c),
            Decision::Update
        );
        assert_eq!(
            run(&rules, &picked, "a", "person", Phase::End, 2_000, &mut c),
            Decision::Held(Held::End)
        );
        // an update of a kind the owner does not want changes no card either
        assert_eq!(
            run(
                &rules,
                &picked,
                "a",
                "vehicle",
                Phase::Update,
                1_000,
                &mut c
            ),
            Decision::Held(Held::Kind)
        );
        // an update does not start a cooldown
        assert_eq!(
            run(&rules, &picked, "b", "person", Phase::Update, 0, &mut c),
            Decision::Update
        );
        assert!(matches!(
            run(&rules, &picked, "b", "person", Phase::New, 1, &mut c),
            Decision::Pop { .. }
        ));
    }

    #[test]
    fn the_pop_ups_switch_the_stay_and_the_limits_come_from_the_page() {
        let mut c = Cooldowns::default();
        let off = Rules {
            on: false,
            ..Rules::default()
        };
        assert_eq!(
            run(&off, &["person"], "a", "person", Phase::New, 0, &mut c),
            Decision::Held(Held::Off)
        );
        assert_eq!(
            run(&off, &["doorbell"], "a", "doorbell", Phase::New, 0, &mut c),
            Decision::Held(Held::Off)
        );
        let long = Rules {
            seconds: 40,
            ..Rules::default()
        };
        assert_eq!(
            run(&long, &["person"], "a", "person", Phase::New, 0, &mut c),
            Decision::Pop { lasts_ms: 40_000 }
        );
        // a stay outside the page's range is brought inside it
        let odd = Rules {
            seconds: 1,
            ..Rules::default()
        };
        assert_eq!(
            run(&odd, &["person"], "q", "person", Phase::New, 0, &mut c),
            Decision::Pop { lasts_ms: 5_000 }
        );
        let huge = Rules {
            seconds: 9999,
            ..Rules::default()
        };
        assert_eq!(
            run(&huge, &["person"], "w", "person", Phase::New, 0, &mut c),
            Decision::Pop { lasts_ms: 60_000 }
        );
        c.clear();
        c.forget_old(0);
    }

    #[test]
    fn an_event_the_bridge_replays_after_a_reconnect_is_history_and_never_a_card() {
        let rules = Rules::default();
        let mut c = Cooldowns::default();
        let notify = kinds(&["person", "doorbell"]);
        let seen = |kind, age_ms| Seen {
            camera: "a",
            kind,
            phase: Phase::New,
            age_ms,
        };
        assert!(matches!(
            decide(&rules, &notify, seen("person", STALE_MS), 0, &mut c),
            Decision::Pop { .. }
        ));
        assert_eq!(
            decide(
                &rules,
                &notify,
                seen("person", STALE_MS + 1),
                100_000,
                &mut c
            ),
            Decision::Held(Held::Old)
        );
        assert_eq!(
            decide(&rules, &notify, seen("doorbell", 10 * 60_000), 0, &mut c),
            Decision::Held(Held::Old)
        );
        // history does not start a cooldown
        assert!(matches!(
            decide(&rules, &notify, seen("person", 1), 100_000, &mut c),
            Decision::Pop { .. }
        ));
    }

    #[test]
    fn old_cooldowns_are_forgotten_and_a_word_of_phase_is_read() {
        let mut c = Cooldowns::default();
        let rules = Rules::default();
        assert!(matches!(
            run(&rules, &["person"], "gone", "person", Phase::New, 0, &mut c),
            Decision::Pop { .. }
        ));
        assert_eq!(c.last.len(), 1);
        c.forget_old(601_000);
        assert!(c.last.is_empty());
        assert_eq!(
            (
                Phase::of("new"),
                Phase::of("update"),
                Phase::of("end"),
                Phase::of("")
            ),
            (Phase::New, Phase::Update, Phase::End, Phase::New)
        );
    }
}
