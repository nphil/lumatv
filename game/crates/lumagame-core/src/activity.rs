//! What a service knows about a game besides listing it, and how that joins what this TV recorded.
//!
//! An engine for an account (Steam) lists no games to play. It reports, per game, when the game was last played there, for how long,
//! whether it is a favourite and what kind of game it is. GeForce NOW reports the same about the games it streams. The catalog puts
//! each report on the game it belongs to ([`Game::reports`]) and the rules here decide what the shelves show:
//!
//! - **Last played** is the latest of this TV's history and every service, and says where it was. A service that reports a day and no
//!   time counts from the start of that day, so a play with a time on the same day outranks it.
//! - **Play time** is never added up. A service that streams a game records that play as its own, so a sum would count the same hours
//!   twice. Each place keeps its figure, and the longest of them orders "most played".
//! - **Favourites** are the owner's own set plus every game a service marks. The heart on a game edits only the owner's own set.
//! - **Genres** from the metadata provider and from every service are joined under one label each ([`genre_label`]).
//! - Equal values order by game id, the way history has always ordered games, so a shelf does not reshuffle from one run to the next.

use crate::catalog::Game;
use crate::history::History;
use crate::ids::{EngineId, GameId};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BTreeSet;

/// What is kept per game and service. An engine is another program, so what it sends is cut to what the app can show.
pub const MAX_GENRES: usize = 6;
pub const MAX_TAGS: usize = 8;
/// The longest genre or tag name kept, in characters.
pub const MAX_NAME: usize = 40;

fn is_false(value: &bool) -> bool {
    !*value
}

/// What one service says about one game.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    /// When it was last played there, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played: Option<u64>,
    /// `last_played` is the start of a day, because the service gives a date and no time.
    #[serde(default, skip_serializing_if = "is_false")]
    pub day_only: bool,
    /// What was played in the last two weeks, in seconds, for a service that says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_s: Option<u64>,
    /// The service's own total for the game, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playtime_s: Option<u64>,
    /// Marked a favourite there.
    #[serde(default, skip_serializing_if = "is_false")]
    pub favourite: bool,
    /// The kinds of game it is listed under (`Action`, `RPG`). The genre shelf and the genre filter group by these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    /// Finer descriptions, the most telling first (`Souls-like`). Shown on the game's page, never grouped by.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Activity {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Cuts the lists and the names to what is kept: [`MAX_GENRES`], [`MAX_TAGS`] and [`MAX_NAME`].
    pub fn clamp(&mut self) {
        for (names, most) in [(&mut self.genres, MAX_GENRES), (&mut self.tags, MAX_TAGS)] {
            names.truncate(most);
            for name in names {
                if let Some((end, _)) = name.char_indices().nth(MAX_NAME) {
                    name.truncate(end);
                }
            }
        }
    }
}

/// One service's [`Activity`] for one game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub engine: EngineId,
    pub activity: Activity,
}

/// Where a game was last played.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    ThisTv,
    Service(EngineId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastPlayed {
    /// Unix seconds; the start of the day when `day_only`.
    pub at: u64,
    pub day_only: bool,
    pub place: Place,
}

/// The latest play of the game anywhere, or `None` when nothing says it was ever played. On equal times this TV wins, then the
/// first service; and a time of day beats a bare date that falls on the same second.
pub fn last_played(game: &Game, history: &History) -> Option<LastPlayed> {
    let own = history.total(game.id).last;
    let mut best = (own > 0).then_some(LastPlayed { at: own, day_only: false, place: Place::ThisTv });
    for report in &game.reports {
        let Some(at) = report.activity.last_played.filter(|at| *at > 0) else { continue };
        let found = LastPlayed { at, day_only: report.activity.day_only, place: Place::Service(report.engine.clone()) };
        if best.as_ref().is_none_or(|b| found.at > b.at || (found.at == b.at && b.day_only && !found.day_only)) {
            best = Some(found);
        }
    }
    best
}

/// The longest play time any one place reports, in seconds, and which place it is: this TV on a tie, then the first service. Never the
/// sum: see the module's rules. `None` when no place has any.
pub fn longest_play(game: &Game, history: &History) -> Option<(u64, Place)> {
    let own = history.total(game.id).seconds;
    let mut best = (own > 0).then_some((own, Place::ThisTv));
    for report in &game.reports {
        let Some(seconds) = report.activity.playtime_s.filter(|s| *s > 0) else { continue };
        if best.as_ref().is_none_or(|(most, _)| seconds > *most) {
            best = Some((seconds, Place::Service(report.engine.clone())));
        }
    }
    best
}

/// [`longest_play`]'s figure, or 0.
pub fn play_seconds(game: &Game, history: &History) -> u64 {
    longest_play(game, history).map_or(0, |(seconds, _)| seconds)
}

/// What the game was played in the last two weeks, in seconds, by the service that says the most.
pub fn recent_play(game: &Game) -> Option<u64> {
    game.reports.iter().filter_map(|r| r.activity.recent_s).filter(|s| *s > 0).max()
}

/// Every game played anywhere, the latest first.
pub fn recently_played(games: &[Game], history: &History) -> Vec<GameId> {
    let mut played: Vec<(u64, GameId)> = games.iter().filter_map(|g| last_played(g, history).map(|l| (l.at, g.id))).collect();
    played.sort_by_key(|(at, id)| (Reverse(*at), *id));
    played.into_iter().map(|(_, id)| id).collect()
}

/// Every game with play time anywhere, the longest first.
pub fn most_played(games: &[Game], history: &History) -> Vec<GameId> {
    let mut played: Vec<(u64, GameId)> = games.iter().map(|g| (play_seconds(g, history), g.id)).filter(|(seconds, _)| *seconds > 0).collect();
    played.sort_by_key(|(seconds, id)| (Reverse(*seconds), *id));
    played.into_iter().map(|(_, id)| id).collect()
}

/// The services that mark the game a favourite, each once.
pub fn favourite_on(game: &Game) -> Vec<&EngineId> {
    let mut on: Vec<&EngineId> = game.reports.iter().filter(|r| r.activity.favourite).map(|r| &r.engine).collect();
    on.dedup();
    on
}

/// Whether the owner or any service marks the game a favourite. `own` is the owner's set.
pub fn is_favourite(game: &Game, own: &BTreeSet<GameId>) -> bool {
    own.contains(&game.id) || game.reports.iter().any(|r| r.activity.favourite)
}

/// The favourites shelf: the owner's own, in the order they are kept, then the games only a service marks, in shelf order.
/// `games` is what may be shown.
pub fn favourites(games: &[Game], own: &BTreeSet<GameId>) -> Vec<GameId> {
    let shown: BTreeSet<GameId> = games.iter().map(|g| g.id).collect();
    let mut shelf: Vec<GameId> = own.iter().copied().filter(|id| shown.contains(id)).collect();
    shelf.extend(games.iter().filter(|g| !own.contains(&g.id) && g.reports.iter().any(|r| r.activity.favourite)).map(|g| g.id));
    shelf
}

/// A genre or tag as the key two spellings of it share: lowercase letters and digits only.
fn key(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Spellings that are one genre, by [`key`], and the label they all show under. IGDB, Steam and GeForce NOW each name a few of the
/// same genres their own way.
const SAME_GENRE: [(&str, &str); 12] = [
    ("rpg", "RPG"),
    ("roleplaying", "RPG"),
    ("roleplayinggame", "RPG"),
    ("roleplayingrpg", "RPG"),
    ("simulator", "Simulation"),
    ("sport", "Sports"),
    ("platform", "Platformer"),
    ("mmo", "Massively Multiplayer"),
    ("rts", "Real-time strategy"),
    ("realtimestrategyrts", "Real-time strategy"),
    ("tbs", "Turn-based strategy"),
    ("turnbasedstrategytbs", "Turn-based strategy"),
];

/// The one label a genre goes by in the app, so `Role-playing (RPG)`, `RPG` and `ROLE_PLAYING` are one genre. A name written as an
/// enum (`SPORTS_GAME`, all capitals) is read as words; a short all-capitals name (`FPS`) is an acronym and stays. `None` for a
/// blank name.
pub fn genre_label(raw: &str) -> Option<String> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return None;
    }
    let shouting = !name.chars().any(char::is_lowercase) && (name.contains('_') || name.chars().count() > 4);
    let name = if shouting {
        name.replace('_', " ")
            .split(' ')
            .filter(|word| !word.is_empty())
            .map(|word| {
                let mut letters = word.chars();
                letters.next().into_iter().flat_map(char::to_uppercase).chain(letters.flat_map(char::to_lowercase)).collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        name
    };
    let key = key(&name);
    Some(SAME_GENRE.iter().find(|(spelling, _)| *spelling == key).map_or(name, |(_, label)| (*label).to_owned()))
}

/// A game's genres: the metadata provider's `own` first, then every service's, one label each, in the order first met.
pub fn genres(game: &Game, own: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    own.iter()
        .chain(game.reports.iter().flat_map(|r| &r.activity.genres))
        .filter_map(|name| genre_label(name))
        .filter(|label| seen.insert(key(label)))
        .collect()
}

/// A game's tags from every service, one each, without the ones that are already one of its genres.
pub fn tags(game: &Game) -> Vec<String> {
    let mut seen: BTreeSet<String> = game.reports.iter().flat_map(|r| &r.activity.genres).filter_map(|name| genre_label(name)).map(|label| key(&label)).collect();
    game.reports
        .iter()
        .flat_map(|r| &r.activity.tags)
        .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|name| !name.is_empty() && seen.insert(key(name)))
        .collect()
}

/// A date or a date with a time as services write them (`2026-11-21`, `2026-11-21T19:04:05Z`): Unix seconds, and whether the time of
/// day is missing. A time with a zone other than `Z` is read as its date alone, because guessing the zone would be wrong by hours.
pub fn parse_when(text: &str) -> Option<(u64, bool)> {
    let text = text.trim();
    let (date, rest) = (text.get(..10)?, text.get(10..)?);
    let mut parts = date.split('-');
    let mut number = || parts.next().and_then(|part| part.parse::<i64>().ok());
    let (year, month, day) = (number()?, number()?, number()?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => return None,
    };
    if !(1970..=9999).contains(&year) || !(1..=last).contains(&day) {
        return None;
    }
    // Days from 1970-01-01 (Hinnant's civil-date algorithm, March-based years).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let start = (era * 146_097 + day_of_era - 719_468) as u64 * 86_400;
    let time = rest.strip_prefix(['T', ' ']).and_then(|time| {
        fn field(part: Option<&str>) -> Option<u64> {
            part?.split('.').next()?.parse().ok()
        }
        let mut parts = time.strip_suffix('Z').unwrap_or(time).split(':');
        let (h, m) = (field(parts.next())?, field(parts.next())?);
        let s = match parts.next() {
            None => 0,
            Some(part) => field(Some(part))?,
        };
        (parts.next().is_none() && h < 24 && m < 60 && s < 61).then_some(h * 3600 + m * 60 + s)
    });
    Some(time.map_or((start, true), |seconds| (start + seconds, false)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Played;
    use crate::ipc::Role;

    fn engine(id: &str) -> EngineId {
        EngineId::new(id).unwrap()
    }

    fn report(engine_id: &str, activity: Activity) -> Report {
        Report { engine: engine(engine_id), activity }
    }

    fn played_at(at: u64) -> Activity {
        Activity { last_played: Some(at), ..Activity::default() }
    }

    fn game(id: u64, reports: Vec<Report>) -> Game {
        Game { id: GameId(id), role: Role::Game, title: format!("Game {id}"), sources: Vec::new(), reports }
    }

    fn history(sessions: &[(u64, u64, u32)]) -> History {
        let mut history = History::default();
        for (game, start, seconds) in sessions {
            history.record(Played { game: GameId(*game), source: "gfn|acct|100".parse().unwrap(), start: *start, seconds: *seconds });
        }
        history
    }

    #[test]
    fn the_latest_play_anywhere_wins_and_names_its_place() {
        let g = game(1, vec![report("gfn", Activity { day_only: true, ..played_at(1_500) }), report("steam", played_at(2_000))]);
        let place = |h: &History| last_played(&g, h).map(|l| (l.at, l.place));
        assert_eq!(place(&History::default()), Some((2_000, Place::Service(engine("steam")))));
        assert_eq!(place(&history(&[(1, 2_900, 100)])), Some((3_000, Place::ThisTv)), "a session counts from when it ended");
        assert_eq!(place(&history(&[(1, 100, 100)])), Some((2_000, Place::Service(engine("steam")))));
        assert_eq!(place(&history(&[(1, 1_900, 100)])), Some((2_000, Place::ThisTv)), "a tie goes to this TV");
        assert_eq!(last_played(&game(2, vec![]), &History::default()), None);
        assert_eq!(last_played(&game(2, vec![report("steam", Activity { playtime_s: Some(60), ..Activity::default() })]), &History::default()), None);
    }

    #[test]
    fn a_bare_date_counts_from_the_start_of_its_day_and_a_time_on_that_day_beats_it() {
        let day = parse_when("2026-11-21").unwrap().0;
        let g = game(1, vec![report("gfn", Activity { day_only: true, ..played_at(day) }), report("steam", played_at(day + 3_600))]);
        let last = last_played(&g, &History::default()).unwrap();
        assert_eq!((last.at, last.day_only, last.place), (day + 3_600, false, Place::Service(engine("steam"))));
        let g = game(1, vec![report("gfn", Activity { day_only: true, ..played_at(day + 86_400) }), report("steam", played_at(day + 3_600))]);
        let last = last_played(&g, &History::default()).unwrap();
        assert_eq!((last.at, last.day_only), (day + 86_400, true), "a later day beats an earlier time");
        let g = game(1, vec![report("gfn", Activity { day_only: true, ..played_at(day) }), report("steam", played_at(day))]);
        assert!(!last_played(&g, &History::default()).unwrap().day_only, "the same second: the exact one");
    }

    #[test]
    fn rows_order_by_the_merged_value_then_by_id_and_leave_out_games_nothing_says_were_played() {
        let games = [
            game(1, vec![]),
            game(2, vec![report("steam", played_at(500))]),
            game(3, vec![report("steam", played_at(500))]),
            game(4, vec![report("gfn", played_at(100))]),
            game(5, vec![]),
        ];
        let history = history(&[(4, 800, 100), (5, 10, 5)]);
        assert_eq!(recently_played(&games, &history), [GameId(4), GameId(2), GameId(3), GameId(5)]);
        assert_eq!(recently_played(&games, &History::default()), [GameId(2), GameId(3), GameId(4)]);
    }

    #[test]
    fn play_time_is_the_longest_place_never_the_sum() {
        let g = game(1, vec![report("steam", Activity { playtime_s: Some(7_200), ..Activity::default() })]);
        assert_eq!(play_seconds(&g, &history(&[(1, 0, 3_600)])), 7_200);
        assert_eq!(play_seconds(&g, &history(&[(1, 0, 9_000)])), 9_000);
        assert_eq!(play_seconds(&g, &History::default()), 7_200);
        assert_eq!(longest_play(&g, &history(&[(1, 0, 3_600)])), Some((7_200, Place::Service(engine("steam")))));
        assert_eq!(longest_play(&g, &history(&[(1, 0, 9_000)])), Some((9_000, Place::ThisTv)));
        assert_eq!(longest_play(&g, &history(&[(1, 0, 7_200)])), Some((7_200, Place::ThisTv)), "this TV wins a tie");
        assert_eq!(longest_play(&game(2, vec![]), &History::default()), None);
        let games = [g, game(2, vec![]), game(3, vec![report("steam", Activity { playtime_s: Some(7_200), ..Activity::default() })])];
        assert_eq!(most_played(&games, &history(&[(2, 0, 7_199)])), [GameId(1), GameId(3), GameId(2)]);
        assert_eq!(most_played(&[game(9, vec![])], &History::default()), []);
    }

    #[test]
    fn favourites_are_the_owners_then_the_ones_only_a_service_marks() {
        let on = |engine_id: &str| report(engine_id, Activity { favourite: true, ..Activity::default() });
        let games = [game(1, vec![on("gfn")]), game(2, vec![]), game(3, vec![on("gfn"), on("gfn")]), game(4, vec![report("steam", played_at(1))])];
        let own = BTreeSet::from([GameId(4), GameId(1), GameId(77)]);
        assert_eq!(favourites(&games, &own), [GameId(1), GameId(4), GameId(3)], "77 is not shown; 1 is in both sets and listed once");
        assert_eq!((is_favourite(&games[0], &own), is_favourite(&games[1], &own), is_favourite(&games[2], &own), is_favourite(&games[3], &own)), (true, false, true, true));
        assert_eq!(favourite_on(&games[2]), [&engine("gfn")]);
        assert!(favourite_on(&games[3]).is_empty());
    }

    #[test]
    fn spellings_of_one_genre_join_and_the_first_spelling_order_is_kept() {
        let names = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(genre_label("ACTION").as_deref(), Some("Action"));
        assert_eq!(genre_label("ROLE_PLAYING").as_deref(), Some("RPG"));
        assert_eq!(genre_label("Role-playing (RPG)").as_deref(), Some("RPG"));
        assert_eq!(genre_label("  Hack and\n slash  ").as_deref(), Some("Hack and slash"));
        assert_eq!(genre_label("FPS").as_deref(), Some("FPS"));
        assert_eq!(genre_label("Souls-like").as_deref(), Some("Souls-like"));
        assert_eq!(genre_label(" \n"), None);

        let g = game(
            1,
            vec![
                report("gfn", Activity { genres: names(&["ROLE_PLAYING", "ADVENTURE"]), ..Activity::default() }),
                report("steam", Activity { genres: names(&["Action", "RPG", "action"]), tags: names(&["Souls-like", "action", "Open World", "souls-like"]), ..Activity::default() }),
            ],
        );
        assert_eq!(genres(&g, &names(&["Role-playing (RPG)", "Indie"])), ["RPG", "Indie", "Adventure", "Action"]);
        assert_eq!(genres(&g, &[]), ["RPG", "Adventure", "Action"]);
        assert_eq!(tags(&g), ["Souls-like", "Open World"], "a tag that is also a genre is shown once, as the genre");
    }

    #[test]
    fn what_an_engine_sends_is_cut_to_what_is_kept() {
        let mut a = Activity { genres: (0..20).map(|i| format!("Genre {i}")).collect(), tags: vec!["é".repeat(100)], ..Activity::default() };
        a.clamp();
        assert_eq!((a.genres.len(), a.tags[0].chars().count()), (MAX_GENRES, MAX_NAME));
        assert!(Activity::default().is_empty() && !a.is_empty());
    }

    #[test]
    fn dates_and_times_read_as_unix_seconds_and_say_when_the_time_is_missing() {
        assert_eq!(parse_when("1970-01-01"), Some((0, true)));
        assert_eq!(parse_when("2000-03-01"), Some((951_868_800, true)));
        assert_eq!(parse_when("2024-02-29"), Some((1_709_164_800, true)));
        assert_eq!(parse_when(" 2026-11-21 "), Some((1_795_219_200, true)));
        assert_eq!(parse_when("2026-11-21T19:04:05Z"), Some((1_795_287_845, false)));
        assert_eq!(parse_when("2026-11-21 19:04:05.250"), Some((1_795_287_845, false)));
        assert_eq!(parse_when("2026-11-21T19:04:05+05:00"), Some((1_795_219_200, true)), "a zone is not guessed");
        for bad in ["", "2023-02-29", "2026-13-01", "2026-00-10", "1969-12-31", "yesterday", "2026/11/21", "20261121"] {
            assert_eq!(parse_when(bad), None, "{bad:?}");
        }
    }
}
