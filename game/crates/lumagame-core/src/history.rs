//! What was played, when and for how long.
//!
//! Only a session that reached Running is ever recorded: a launch that was cancelled, left in a queue or failed leaves nothing, so
//! "recently played" and playtime never name a game that was not on screen. The time is the time LumaGame's stream ran on this TV.
//! It is not the store's total, and the app labels it as its own.

use crate::catalog::Merge;
use crate::ids::{GameId, SourceKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// How many single sessions are kept. The totals are kept for every game for good; this bounds only the session list.
pub const RECENT_MAX: usize = 200;

/// One session that ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Played {
    pub game: GameId,
    pub source: SourceKey,
    /// When it began running, in Unix seconds.
    pub start: u64,
    pub seconds: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Total {
    pub seconds: u64,
    pub sessions: u32,
    /// When the game was last on screen, in Unix seconds.
    pub last: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    totals: BTreeMap<GameId, Total>,
    /// Oldest first.
    #[serde(default)]
    recent: VecDeque<Played>,
}

impl History {
    /// Adds a session that ran. Returns false, and changes nothing, for one already recorded: a session interrupted by a crash is
    /// recorded again from its checkpoint file at the next start, and that must not count it twice.
    pub fn record(&mut self, played: Played) -> bool {
        if self.recent.iter().any(|p| p.game == played.game && p.start == played.start) {
            return false;
        }
        let total = self.totals.entry(played.game).or_default();
        total.seconds += u64::from(played.seconds);
        total.sessions += 1;
        total.last = total.last.max(played.start + u64::from(played.seconds));
        self.recent.push_back(played);
        while self.recent.len() > RECENT_MAX {
            self.recent.pop_front();
        }
        true
    }

    pub fn total(&self, game: GameId) -> Total {
        self.totals.get(&game).copied().unwrap_or_default()
    }

    pub fn sessions(&self) -> impl DoubleEndedIterator<Item = &Played> {
        self.recent.iter()
    }

    /// Two games turned out to be one.
    pub fn merge(&mut self, merge: Merge) {
        if let Some(from) = self.totals.remove(&merge.from) {
            let into = self.totals.entry(merge.into).or_default();
            into.seconds += from.seconds;
            into.sessions += from.sessions;
            into.last = into.last.max(from.last);
        }
        for played in self.recent.iter_mut().filter(|p| p.game == merge.from) {
            played.game = merge.into;
        }
    }

    /// Removes a game's history at the owner's request.
    pub fn forget(&mut self, game: GameId) {
        self.totals.remove(&game);
        self.recent.retain(|p| p.game != game);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn played(game: u64, start: u64, seconds: u32) -> Played {
        Played { game: GameId(game), source: "gfn|acct|100".parse().unwrap(), start, seconds }
    }

    #[test]
    fn sessions_add_up_per_game() {
        let mut history = History::default();
        assert!(history.record(played(1, 1_000, 600)));
        assert!(history.record(played(2, 2_000, 7_200)));
        assert!(history.record(played(1, 20_000, 60)));
        assert!(history.record(played(3, 10_000, 30)));
        assert_eq!(history.total(GameId(1)), Total { seconds: 660, sessions: 2, last: 20_060 });
        assert_eq!(history.total(GameId(9)), Total::default());
        assert_eq!(serde_json::from_str::<History>(&serde_json::to_string(&history).unwrap()).unwrap(), history);
    }

    #[test]
    fn a_session_recovered_twice_counts_once() {
        let mut history = History::default();
        assert!(history.record(played(1, 1_000, 600)));
        assert!(!history.record(played(1, 1_000, 660)), "the same session, read again from its checkpoint");
        assert_eq!(history.total(GameId(1)), Total { seconds: 600, sessions: 1, last: 1_600 });
    }

    #[test]
    fn the_session_list_is_bounded_and_the_totals_are_not() {
        let mut history = History::default();
        for i in 0..RECENT_MAX as u64 + 5 {
            history.record(played(1, i * 100, 10));
        }
        assert_eq!(history.sessions().count(), RECENT_MAX);
        assert_eq!(history.sessions().next().unwrap().start, 500, "the oldest five are gone");
        assert_eq!(history.total(GameId(1)).sessions, RECENT_MAX as u32 + 5);
    }

    #[test]
    fn merged_games_share_one_history_and_a_forgotten_game_has_none() {
        let mut history = History::default();
        history.record(played(1, 1_000, 600));
        history.record(played(2, 5_000, 300));
        history.merge(Merge { from: GameId(2), into: GameId(1) });
        assert_eq!(history.total(GameId(1)), Total { seconds: 900, sessions: 2, last: 5_300 });
        assert_eq!(history.total(GameId(2)), Total::default());
        assert!(history.sessions().all(|p| p.game == GameId(1)));
        history.merge(Merge { from: GameId(7), into: GameId(1) });
        assert_eq!(history.total(GameId(1)).sessions, 2, "a game with no history merges as nothing");

        history.forget(GameId(1));
        assert_eq!(history, History::default());
    }
}
