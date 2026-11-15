//! Which target Play uses.
//!
//! The order is fixed: the target pinned for this game, then the engine preferred for every game, then the target that last played
//! it, then the only target checked ready, then the chooser. Two rules cut across it:
//!
//! - Play never switches silently. When the target the order arrives at is known to be blocked (the PC is offline, the game is not
//!   installed there, its engine was switched off), the answer is the chooser with that target in focus and its state on it, not
//!   the next target down the order.
//! - Only a checked fact counts as ready. "Unknown" may be tried when the owner already chose that target; it never makes a target
//!   the automatic pick over others.
//!
//! Nothing here remembers anything. A target is remembered when its session reaches Running ([`crate::session::Effect::Started`]),
//! so a failed or cancelled launch never becomes the next default.

use crate::catalog::Source;
use crate::ids::{EngineId, SourceKey};
use crate::ipc::{Link, Reach, Readiness, Target};

/// One way to play the game, with what is known of it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub key: SourceKey,
    pub readiness: Readiness,
}

impl Candidate {
    /// What is known of a source now: its entry's readiness, capped by the state of its target. `target` is `None` while the engine
    /// has not reported its targets (it is still starting, or the target is gone), and then nothing is known.
    pub fn of(source: &Source, target: Option<&Target>) -> Candidate {
        let entry = source.entry.readiness;
        let readiness = match target.map(|t| (t.link, t.reach)) {
            None => Readiness::Unknown,
            Some((Link::Expired | Link::Found, _)) => Readiness::SignIn,
            Some((_, Reach::Offline)) => Readiness::Offline,
            // Waking the host does not install the game.
            Some((_, Reach::Asleep)) if entry != Readiness::Install => Readiness::Asleep,
            Some(_) => entry,
        };
        Candidate { key: source.key(), readiness }
    }
}

/// What the owner's earlier choices say about this game.
#[derive(Debug, Clone, Copy, Default)]
pub struct Remembered<'a> {
    /// "Remember for this game" in the chooser.
    pub pinned: Option<&'a SourceKey>,
    /// The engine preferred for every game, from Settings.
    pub preferred_engine: Option<&'a EngineId>,
    /// The target the game last ran on.
    pub last_ok: Option<&'a SourceKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Launch {
        key: SourceKey,
        why: Why,
    },
    /// Ask. `focus` is the row the chooser opens on.
    Choose {
        why: ChooseWhy,
        focus: Option<SourceKey>,
    },
    /// The game has no way to play it now.
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Pinned,
    PreferredEngine,
    LastPlayed,
    OnlyReady,
    /// The only way to play it, and not known to be blocked.
    Only,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChooseWhy {
    /// More than one target could be meant.
    Several,
    /// The remembered target cannot be used now. It is in focus, with its state.
    Unavailable,
    /// Every target is known to be blocked.
    Blocked,
}

pub fn resolve(candidates: &[Candidate], remembered: Remembered<'_>) -> Resolution {
    if candidates.is_empty() {
        return Resolution::Nothing;
    }
    // The owner's choice: start it when it can be tried, show it when it cannot. A remembered target that is no longer listed at
    // all (its engine is off, its PC was forgotten) is shown the same way, by its key, so the chooser can say what happened to it.
    let chosen = |key: &SourceKey, why: Why| match candidates.iter().find(|c| &c.key == key) {
        Some(c) if c.readiness.can_attempt() => Resolution::Launch { key: key.clone(), why },
        _ => Resolution::Choose { why: ChooseWhy::Unavailable, focus: Some(key.clone()) },
    };

    if let Some(pinned) = remembered.pinned {
        return chosen(pinned, Why::Pinned);
    }

    let mut pool: Vec<&Candidate> = candidates.iter().collect();
    let mut narrowed_to: Option<&EngineId> = None;
    if let Some(engine) = remembered.preferred_engine {
        let on_engine: Vec<&Candidate> = candidates.iter().filter(|c| &c.key.engine == engine).collect();
        match on_engine.as_slice() {
            // The preferred engine does not have this game: the preference says nothing about it.
            [] => {}
            [only] => return chosen(&only.key, Why::PreferredEngine),
            // Two PCs, or two stores on one account: the preference narrows the choice and the rest of the order settles it.
            _ => {
                pool = on_engine;
                narrowed_to = Some(engine);
            }
        }
    }

    // The preference outranks the last target, so a last target on another engine is no longer what Play would be leaving.
    if let Some(last) = remembered.last_ok.filter(|last| narrowed_to.is_none_or(|engine| &last.engine == engine)) {
        return chosen(last, Why::LastPlayed);
    }

    let mut ready = pool.iter().filter(|c| c.readiness.is_ready());
    if let (Some(only), None) = (ready.next(), ready.next()) {
        return Resolution::Launch { key: only.key.clone(), why: Why::OnlyReady };
    }
    if let Some(only) = pool.first().filter(|c| pool.len() == 1 && c.readiness.can_attempt()) {
        return Resolution::Launch { key: only.key.clone(), why: Why::Only };
    }
    let why = if pool.iter().any(|c| c.readiness.can_attempt()) { ChooseWhy::Several } else { ChooseWhy::Blocked };
    Resolution::Choose { why, focus: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{Art, LibraryEntry, Role};
    use Readiness::{Asleep, Install, Offline, Ready, SignIn, Unknown};

    fn key(s: &str) -> SourceKey {
        s.parse().unwrap()
    }

    fn candidates(list: &[(&str, Readiness)]) -> Vec<Candidate> {
        list.iter().map(|(k, readiness)| Candidate { key: key(k), readiness: *readiness }).collect()
    }

    fn launch(k: &str, why: Why) -> Resolution {
        Resolution::Launch { key: key(k), why }
    }

    fn choose(why: ChooseWhy, focus: Option<&str>) -> Resolution {
        Resolution::Choose { why, focus: focus.map(key) }
    }

    const CLOUD: &str = "gfn|acct|100";
    const PC1: &str = "punktfunk|pc1|steam:1";
    const PC2: &str = "punktfunk|pc2|steam:1";

    #[test]
    fn a_source_is_no_readier_than_its_target() {
        let source = |readiness| Source {
            engine: EngineId::new("punktfunk").unwrap(),
            target: "pc1".to_owned().try_into().unwrap(),
            entry: LibraryEntry {
                id: "steam:1".to_owned().try_into().unwrap(),
                title: "Hades".into(),
                role: Role::Game,
                external: vec![],
                store: None,
                store_account: None,
                art: Art::default(),
                readiness,
                resumable: false,
            },
        };
        let target = |link, reach| Target { id: "pc1".to_owned().try_into().unwrap(), name: "Gaming PC".into(), detail: None, link, reach };
        let of = |entry, link, reach| Candidate::of(&source(entry), Some(&target(link, reach))).readiness;

        assert_eq!(of(Ready, Link::Linked, Reach::Online), Ready);
        assert_eq!(of(Install, Link::Linked, Reach::Online), Install);
        assert_eq!(of(Ready, Link::Linked, Reach::Unknown), Ready, "the engine checked the game itself");
        assert_eq!(of(Ready, Link::Linked, Reach::Offline), Offline);
        assert_eq!(of(Ready, Link::Linked, Reach::Asleep), Asleep);
        assert_eq!(of(Install, Link::Linked, Reach::Asleep), Install);
        assert_eq!(of(Ready, Link::Expired, Reach::Online), SignIn);
        assert_eq!(of(Ready, Link::Found, Reach::Offline), SignIn);
        assert_eq!(Candidate::of(&source(Ready), None), Candidate { key: key(PC1), readiness: Unknown }, "its engine has not reported yet");
    }

    #[test]
    fn the_order_is_pinned_then_preferred_engine_then_last_played_then_only_ready() {
        let all = candidates(&[(CLOUD, Ready), (PC1, Ready), (PC2, Ready)]);
        let (cloud, pc1, pc2) = (key(CLOUD), key(PC1), key(PC2));
        let (gfn, punktfunk) = (EngineId::new("gfn").unwrap(), EngineId::new("punktfunk").unwrap());

        let everything = Remembered { pinned: Some(&pc2), preferred_engine: Some(&gfn), last_ok: Some(&pc1) };
        assert_eq!(resolve(&all, everything), launch(PC2, Why::Pinned));
        assert_eq!(resolve(&all, Remembered { pinned: None, ..everything }), launch(CLOUD, Why::PreferredEngine));
        assert_eq!(resolve(&all, Remembered { last_ok: Some(&pc1), ..Remembered::default() }), launch(PC1, Why::LastPlayed));
        assert_eq!(resolve(&all, Remembered::default()), choose(ChooseWhy::Several, None));
        assert_eq!(resolve(&candidates(&[(CLOUD, Unknown), (PC1, Ready), (PC2, Asleep)]), Remembered::default()), launch(PC1, Why::OnlyReady));

        // A preferred engine with two targets narrows the choice; the last played among them, or the only ready one, settles it.
        let prefer_pc = Remembered { preferred_engine: Some(&punktfunk), ..Remembered::default() };
        assert_eq!(resolve(&all, prefer_pc), choose(ChooseWhy::Several, None));
        assert_eq!(resolve(&all, Remembered { last_ok: Some(&pc2), ..prefer_pc }), launch(PC2, Why::LastPlayed));
        assert_eq!(resolve(&all, Remembered { last_ok: Some(&cloud), ..prefer_pc }), choose(ChooseWhy::Several, None), "last played is outside the preference");
        assert_eq!(resolve(&candidates(&[(CLOUD, Ready), (PC1, Offline), (PC2, Ready)]), prefer_pc), launch(PC2, Why::OnlyReady));

        // A preferred engine that does not have the game says nothing.
        let only_pcs = candidates(&[(PC1, Ready), (PC2, Ready)]);
        assert_eq!(resolve(&only_pcs, Remembered { preferred_engine: Some(&gfn), last_ok: Some(&pc1), ..Remembered::default() }), launch(PC1, Why::LastPlayed));
    }

    #[test]
    fn a_blocked_choice_opens_the_chooser_on_itself_and_never_starts_another_target() {
        let pc1 = key(PC1);
        let gfn = EngineId::new("gfn").unwrap();
        for blocked in [Offline, Install] {
            let list = candidates(&[(CLOUD, Ready), (PC1, blocked)]);
            assert_eq!(resolve(&list, Remembered { pinned: Some(&pc1), ..Remembered::default() }), choose(ChooseWhy::Unavailable, Some(PC1)));
            assert_eq!(resolve(&list, Remembered { last_ok: Some(&pc1), ..Remembered::default() }), choose(ChooseWhy::Unavailable, Some(PC1)));
        }
        let cloud_down = candidates(&[(CLOUD, Offline), (PC1, Ready)]);
        assert_eq!(resolve(&cloud_down, Remembered { preferred_engine: Some(&gfn), ..Remembered::default() }), choose(ChooseWhy::Unavailable, Some(CLOUD)));

        // The remembered target is not listed at all: its engine was switched off, or its PC no longer has the game.
        let without_pc = candidates(&[(CLOUD, Ready)]);
        assert_eq!(resolve(&without_pc, Remembered { pinned: Some(&pc1), ..Remembered::default() }), choose(ChooseWhy::Unavailable, Some(PC1)));
        assert_eq!(resolve(&without_pc, Remembered { last_ok: Some(&pc1), ..Remembered::default() }), choose(ChooseWhy::Unavailable, Some(PC1)));
    }

    #[test]
    fn a_chosen_target_is_tried_while_it_is_not_known_to_be_blocked() {
        let pc1 = key(PC1);
        for possible in [Ready, SignIn, Asleep, Unknown] {
            let list = candidates(&[(CLOUD, Ready), (PC1, possible)]);
            assert_eq!(resolve(&list, Remembered { pinned: Some(&pc1), ..Remembered::default() }), launch(PC1, Why::Pinned), "{possible}");
            assert_eq!(resolve(&list, Remembered { last_ok: Some(&pc1), ..Remembered::default() }), launch(PC1, Why::LastPlayed), "{possible}");
        }
    }

    #[test]
    fn with_nothing_remembered_only_a_checked_target_or_a_single_one_starts_by_itself() {
        let none = Remembered::default();
        assert_eq!(resolve(&[], none), Resolution::Nothing);
        assert_eq!(resolve(&candidates(&[(CLOUD, Unknown)]), none), launch(CLOUD, Why::Only));
        assert_eq!(resolve(&candidates(&[(CLOUD, SignIn)]), none), launch(CLOUD, Why::Only), "which leads into the sign-in");
        assert_eq!(resolve(&candidates(&[(PC1, Offline)]), none), choose(ChooseWhy::Blocked, None));
        assert_eq!(resolve(&candidates(&[(CLOUD, Unknown), (PC1, Unknown)]), none), choose(ChooseWhy::Several, None), "unknown is not ready");
        assert_eq!(
            resolve(&candidates(&[(CLOUD, Unknown), (PC1, Offline)]), none),
            choose(ChooseWhy::Several, None),
            "one unchecked target among others is not picked for the owner"
        );
        assert_eq!(resolve(&candidates(&[(CLOUD, Install), (PC1, Offline)]), none), choose(ChooseWhy::Blocked, None));
    }
}
