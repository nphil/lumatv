//! One card per game.
//!
//! Every engine lists its own entries per target. The catalog folds the entries that are the same game into one [`Game`], and the
//! rule for "the same" is deliberately narrow, because a wrong merge hides a game behind another's card and starts the wrong one:
//!
//! - Two entries are one game when they share an [`ExternalId`] (the same Steam app, the same GeForce NOW title, the same webOS
//!   app), or when the owner linked them.
//! - A title never merges anything. Two entries with the same title and no shared id stay two cards; the app may offer to link
//!   them ([`same_title`]) and does nothing until the owner agrees.
//! - A launcher (a desktop, Big Picture) is never a game and never merges, whatever ids it carries.
//! - A record ([`Role::Record`]) is what an account knows about a game, not a game: it never makes a card or an id. It is put on the
//!   game that has one of its ids ([`Game::reports`]), and on no game when none has one or two do.
//! - An entry's own id means something only on its engine and target: `42` on one PC is not `42` on another.
//!
//! [`Identity`] is the saved half: it gives each game a [`GameId`] the first time it is seen and keeps it, so favourites, history
//! and remembered targets survive an engine being switched off, a PC being asleep and a library being re-read.

use crate::activity::Report;
use crate::ids::{EngineId, ExternalId, GameId, ProviderGameId, SourceKey, TargetId};
use crate::ipc::{LibraryEntry, Readiness, Role};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One entry of one target's library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub engine: EngineId,
    pub target: TargetId,
    pub entry: LibraryEntry,
}

impl Source {
    pub fn key(&self) -> SourceKey {
        SourceKey { engine: self.engine.clone(), target: self.target.clone(), id: self.entry.id.clone() }
    }
}

/// Two games turned out to be one: everything kept for `from` now belongs to `into`. `into` is always the older game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Merge {
    pub from: GameId,
    pub into: GameId,
}

/// Which keys name which game. Saved with the state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// The last id given out.
    #[serde(default)]
    last: u64,
    /// `x:<external id>` and `s:<source key>` to the game they name.
    #[serde(default)]
    keys: BTreeMap<String, GameId>,
    /// The owner's word on one source. It outranks every key, in both directions: "this is that game" and "this is its own game".
    #[serde(default)]
    overrides: BTreeMap<SourceKey, GameId>,
}

impl Identity {
    fn mint(&mut self) -> GameId {
        self.last += 1;
        GameId(self.last)
    }

    /// The game a source belongs to, and the games it showed to be the same as that one. A source that carries the ids of two games
    /// known so far joins them: the older keeps its id.
    fn assign(&mut self, source: &Source) -> (GameId, Vec<GameId>) {
        let own = source.key();
        if let Some(game) = self.overrides.get(&own) {
            return (*game, Vec::new());
        }
        let mut keys = vec![format!("s:{own}")];
        // A launcher's ids say what it opens, not what it is.
        if source.entry.role == Role::Game {
            keys.extend(source.entry.external.iter().map(|id| format!("x:{id}")));
        }
        let known: BTreeSet<GameId> = keys.iter().filter_map(|k| self.keys.get(k).copied()).collect();
        let mut known = known.into_iter();
        let game = known.next().unwrap_or_else(|| self.mint());
        let absorbed: Vec<GameId> = known.collect();
        if !absorbed.is_empty() {
            let moved = |g: &mut GameId| {
                if absorbed.contains(g) {
                    *g = game;
                }
            };
            self.keys.values_mut().for_each(moved);
            self.overrides.values_mut().for_each(moved);
        }
        for key in keys {
            self.keys.insert(key, game);
        }
        (game, absorbed)
    }

    /// The owner says `source` is `game`. When the source was the last of the game it leaves, the caller folds that game's data into
    /// `game` with a [`Merge`]; the catalog is rebuilt either way.
    pub fn link(&mut self, source: SourceKey, game: GameId) {
        self.overrides.insert(source, game);
    }

    /// The owner says `source` is a game of its own, whatever ids it shares. Returns the new game.
    pub fn unlink(&mut self, source: SourceKey) -> GameId {
        let game = self.mint();
        self.overrides.insert(source, game);
        game
    }

    /// The game a source belonged to when it was last listed.
    pub fn game_of(&self, source: &SourceKey) -> Option<GameId> {
        self.overrides.get(source).or_else(|| self.keys.get(&format!("s:{source}"))).copied()
    }
}

/// One card: a game and every way to play it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub id: GameId,
    pub role: Role,
    pub title: String,
    /// In [`SourceKey`] order.
    pub sources: Vec<Source>,
    /// What the services say about the game: its own sources', then the accounts' records that share one of its ids. By engine.
    pub reports: Vec<Report>,
}

impl Game {
    pub fn source(&self, key: &SourceKey) -> Option<&Source> {
        self.sources.iter().find(|s| &s.key() == key)
    }

    pub fn engines(&self) -> BTreeSet<&EngineId> {
        self.sources.iter().map(|s| &s.engine).collect()
    }

    /// Every external id any source gives the game.
    pub fn external(&self) -> BTreeSet<&ExternalId> {
        self.sources.iter().flat_map(|s| s.entry.external.iter()).collect()
    }

    pub fn external_in(&self, ns: &str) -> Option<&str> {
        self.external().into_iter().find(|id| id.ns() == ns).map(ExternalId::id)
    }

    /// Where to look for one piece of artwork, best first; the app shows the first that loads. Steam's own images lead when the game
    /// has a Steam id because they are the same size and style for every game, which is what makes a shelf look like one shelf;
    /// each source's own artwork follows. An older Steam app may lack one of these files, which is why this is a list.
    pub fn art(&self, kind: ArtKind) -> Vec<String> {
        let mut urls = Vec::new();
        if let Some(app) = self.external_in(ExternalId::STEAM).filter(|id| id.bytes().all(|b| b.is_ascii_digit())) {
            let file = match kind {
                ArtKind::Cover => Some("library_600x900_2x.jpg"),
                ArtKind::Hero => Some("library_hero.jpg"),
                ArtKind::Logo => Some("logo.png"),
                ArtKind::Icon => None,
            };
            urls.extend(file.map(|file| format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{app}/{file}")));
        }
        for source in &self.sources {
            let art = &source.entry.art;
            let own = match kind {
                ArtKind::Cover => &art.cover,
                ArtKind::Hero => &art.hero,
                ArtKind::Logo => &art.logo,
                ArtKind::Icon => &art.icon,
            };
            if let Some(url) = own.as_ref().filter(|url| !urls.contains(url)) {
                urls.push(url.clone());
            }
        }
        urls
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtKind {
    Cover,
    Hero,
    Logo,
    Icon,
}

/// Folds every listed source into games. Returns them in shelf order (by title, articles aside) with the merges this pass found,
/// which the caller applies to whatever it keeps by [`GameId`] before it uses the games.
pub fn build(identity: &mut Identity, mut sources: Vec<Source>) -> (Vec<Game>, Vec<Merge>) {
    // A game's id must not depend on which engine answered first.
    sources.sort_by_cached_key(Source::key);
    sources.dedup_by_key(|s| s.key());
    let (records, sources): (Vec<Source>, Vec<Source>) = sources.into_iter().partition(|s| s.entry.role == Role::Record);

    let mut merges: Vec<Merge> = Vec::new();
    let mut placed: Vec<(GameId, Source)> = Vec::with_capacity(sources.len());
    for source in sources {
        let (game, absorbed) = identity.assign(&source);
        merges.extend(absorbed.into_iter().map(|from| Merge { from, into: game }));
        placed.push((game, source));
    }

    // A later source may have folded an earlier one's game into an older game. A merge always goes to a lower id, so this ends.
    let settled = |mut game: GameId| {
        while let Some(merge) = merges.iter().find(|m| m.from == game) {
            game = merge.into;
        }
        game
    };
    let mut by_game: BTreeMap<GameId, Vec<Source>> = BTreeMap::new();
    for (game, source) in placed {
        by_game.entry(settled(game)).or_default().push(source);
    }

    let mut games: Vec<Game> = by_game
        .into_iter()
        .map(|(id, sources)| {
            // The source that identifies the game best names it: a store's entry carries the published title, where a host's own list
            // may carry whatever its owner typed.
            let named = sources.iter().rev().max_by_key(|s| s.entry.external.len()).expect("a game has a source");
            let reports = sources.iter().filter(|s| !s.entry.activity.is_empty()).map(|s| Report { engine: s.engine.clone(), activity: s.entry.activity.clone() }).collect();
            Game { id, role: named.entry.role, title: named.entry.title.clone(), sources, reports }
        })
        .collect();
    attach(&mut games, &records);
    games.sort_by_cached_key(|g| (sort_key(&g.title), g.id));
    (games, merges)
}

/// Puts each record's activity on the one game that has one of the record's ids. An id two games both have (the owner kept them
/// apart) names neither, and a record whose ids name two different games belongs to neither: nothing is put on a game it may not
/// be about. A record with no id, or none that a game has, is dropped. A launcher never takes a record, whatever ids it carries.
fn attach(games: &mut [Game], records: &[Source]) {
    let mut owner: BTreeMap<ExternalId, Option<usize>> = BTreeMap::new();
    for (index, game) in games.iter().enumerate().filter(|(_, g)| g.role == Role::Game) {
        for id in game.external() {
            let slot = owner.entry(id.clone()).or_insert(Some(index));
            if *slot != Some(index) {
                *slot = None;
            }
        }
    }
    let mut touched = BTreeSet::new();
    for record in records.iter().filter(|r| !r.entry.activity.is_empty()) {
        let mut named = record.entry.external.iter().filter_map(|id| owner.get(id).copied().flatten());
        let Some(game) = named.next() else { continue };
        if named.any(|other| other != game) {
            continue;
        }
        games[game].reports.push(Report { engine: record.engine.clone(), activity: record.entry.activity.clone() });
        touched.insert(game);
    }
    for game in touched {
        games[game].reports.sort_by(|a, b| a.engine.cmp(&b.engine));
    }
}

/// A title as it sorts and compares: lowercase letters and digits, single spaces, no leading article.
pub fn sort_key(title: &str) -> String {
    let mut key = String::with_capacity(title.len());
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            key.push(c);
        } else if !key.is_empty() && !key.ends_with(' ') {
            key.push(' ');
        }
    }
    let key = key.trim_end();
    ["the ", "a ", "an "].iter().find_map(|article| key.strip_prefix(article)).unwrap_or(key).to_owned()
}

/// Whether two games read as the same title. A reason to ask the owner, never a reason to merge.
pub fn same_title(a: &Game, b: &Game) -> bool {
    a.id != b.id && a.role == Role::Game && b.role == Role::Game && !sort_key(&a.title).is_empty() && sort_key(&a.title) == sort_key(&b.title)
}

/// What the app may say about saves when a game can be played two ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveCompat {
    /// The same store, the same account there, and the store keeps this game's saves online.
    Shared,
    /// Different stores or accounts, or a store that does not keep this game's saves online.
    Separate,
    Unknown,
}

/// `store_syncs` is whether the store keeps this game's saves online, from the game's metadata; `None` when that is not known.
/// "Shared" is claimed only when every part of it is known. A missing fact reads as [`SaveCompat::Unknown`], never as a guess.
pub fn save_compat(a: &LibraryEntry, b: &LibraryEntry, store_syncs: Option<bool>) -> SaveCompat {
    match (&a.store, &b.store) {
        (Some(x), Some(y)) if x != y => SaveCompat::Separate,
        (Some(_), Some(_)) => match (&a.store_account, &b.store_account, store_syncs) {
            (Some(x), Some(y), _) if x != y => SaveCompat::Separate,
            (_, _, Some(false)) => SaveCompat::Separate,
            (Some(_), Some(_), Some(true)) => SaveCompat::Shared,
            _ => SaveCompat::Unknown,
        },
        _ => SaveCompat::Unknown,
    }
}

/// The saved copy of what the engines last listed, so the shelves are drawn before any engine has answered and a PC that is asleep
/// keeps its games. It holds nothing that cannot be fetched again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    #[serde(default)]
    pub sources: Vec<Source>,
}

impl Listing {
    /// Replaces what one target listed. An empty library is a fact too; a failed refresh must not call this.
    pub fn replace(&mut self, engine: &EngineId, target: &TargetId, entries: Vec<LibraryEntry>) {
        self.sources.retain(|s| !(&s.engine == engine && &s.target == target));
        self.sources.extend(entries.into_iter().map(|mut entry| {
            entry.activity.clamp();
            Source { engine: engine.clone(), target: target.clone(), entry }
        }));
    }

    /// Drops a target that was unlinked.
    pub fn forget(&mut self, engine: &EngineId, target: &TargetId) {
        self.sources.retain(|s| !(&s.engine == engine && &s.target == target));
    }

    /// What the catalog is built from: the sources of the engines that are switched on. A switched-off engine's entries stay saved,
    /// so switching it back on brings its games back at once, but none of them is shown or launched meanwhile.
    pub fn enabled(&self, engines: &BTreeSet<EngineId>) -> Vec<Source> {
        self.sources.iter().filter(|s| engines.contains(&s.engine)).cloned().collect()
    }

    pub fn set_readiness(&mut self, key: &SourceKey, readiness: Readiness) {
        if let Some(source) = self.sources.iter_mut().find(|s| &s.key() == key) {
            source.entry.readiness = readiness;
        }
    }

    /// Marks every entry unchecked. For the listing read back from disk: what was ready when it was saved is not a checked fact
    /// now, and stays unknown until its engine lists it again.
    pub fn unchecked(&mut self) {
        for source in &mut self.sources {
            source.entry.readiness = Readiness::Unknown;
            source.entry.resumable = false;
        }
    }
}

/// The id an engine gives a game that its far side lists without one, from what is stable about it. Hex of FNV-1a 64: short, the
/// same on every run and every build (which a std hasher does not promise), and collisions only matter inside one target's library.
pub fn derived_id(parts: &[&str]) -> ProviderGameId {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.bytes().chain([0]) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    ProviderGameId::new(format!("d{hash:016x}")).expect("hex is a valid id")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::Activity;
    use crate::ipc::Art;

    fn source(engine: &str, target: &str, id: &str, title: &str, external: &[&str]) -> Source {
        Source {
            engine: EngineId::new(engine).unwrap(),
            target: TargetId::new(target).unwrap(),
            entry: LibraryEntry {
                id: ProviderGameId::new(id).unwrap(),
                title: title.into(),
                role: Role::Game,
                external: external.iter().map(|x| x.parse().unwrap()).collect(),
                store: None,
                store_account: None,
                art: Art::default(),
                readiness: Readiness::Unknown,
                resumable: false,
                activity: Activity::default(),
            },
        }
    }

    fn launcher(engine: &str, target: &str, id: &str, title: &str, external: &[&str]) -> Source {
        let mut s = source(engine, target, id, title, external);
        s.entry.role = Role::Launcher;
        s
    }

    fn titles(games: &[Game]) -> Vec<(&str, usize)> {
        games.iter().map(|g| (g.title.as_str(), g.sources.len())).collect()
    }

    #[test]
    fn entries_that_share_an_external_id_are_one_game_and_a_title_alone_merges_nothing() {
        let mut identity = Identity::default();
        let (games, merges) = build(
            &mut identity,
            vec![
                source("gfn", "acct", "100:steam", "ELDEN RING", &["gfn:100", "steam:1245620"]),
                source("gfn", "acct", "100:epic", "ELDEN RING", &["gfn:100"]),
                source("punktfunk", "pc1", "steam:1245620", "Elden Ring", &["steam:1245620"]),
                source("gamestream", "pc1", "881", "Elden Ring", &[]),
                source("gamestream", "pc2", "881", "Hades", &[]),
            ],
        );
        assert_eq!(titles(&games), [("Elden Ring", 1), ("ELDEN RING", 3), ("Hades", 1)]);
        assert!(merges.is_empty());
        assert!(same_title(&games[0], &games[1]), "the app may ask");
        assert!(!same_title(&games[0], &games[2]) && !same_title(&games[0], &games[0]));
        assert_eq!(games[1].engines().len(), 2);
        assert_ne!(games[0].id, games[2].id, "the same id on two PCs is two games");
    }

    #[test]
    fn a_launcher_never_merges_even_with_a_shared_id() {
        let mut identity = Identity::default();
        let (games, _) = build(
            &mut identity,
            vec![
                launcher("punktfunk", "pc1", "steam-gamepadui", "Steam Big Picture", &["steam:1245620"]),
                launcher("gamestream", "pc1", "1", "Desktop", &[]),
                launcher("punktfunk", "pc1", "desktop", "Desktop", &[]),
                source("gfn", "acct", "100", "Elden Ring", &["steam:1245620"]),
            ],
        );
        assert_eq!(titles(&games), [("Desktop", 1), ("Desktop", 1), ("Elden Ring", 1), ("Steam Big Picture", 1)]);
        assert!(!same_title(&games[0], &games[1]), "two desktops are not offered as one game either");
    }

    fn record(engine: &str, id: &str, external: &[&str], activity: Activity) -> Source {
        let mut s = source(engine, "acct", id, id, external);
        s.entry.role = Role::Record;
        s.entry.activity = activity;
        s
    }

    fn played(at: u64) -> Activity {
        Activity { last_played: Some(at), playtime_s: Some(60), ..Activity::default() }
    }

    fn reports(game: &Game) -> Vec<(&str, Option<u64>, bool)> {
        game.reports.iter().map(|r| (r.engine.as_str(), r.activity.last_played, r.activity.favourite)).collect()
    }

    #[test]
    fn a_record_is_put_on_the_game_that_shares_its_id_and_is_never_a_card() {
        let mut identity = Identity::default();
        let mut cloud = source("gfn", "acct", "100", "Elden Ring", &["gfn:100", "steam:1245620"]);
        cloud.entry.activity = Activity { favourite: true, ..Activity::default() };
        let (games, merges) = build(
            &mut identity,
            vec![
                record("steam", "620", &["steam:620"], played(5)),
                record("steam", "1245620", &["steam:1245620"], played(9)),
                record("steam", "no-id", &[], played(7)),
                record("steam", "unowned", &["steam:999"], played(7)),
                cloud,
                source("punktfunk", "pc1", "620", "Portal 2", &["steam:620"]),
                source("gamestream", "pc1", "881", "Hades", &[]),
            ],
        );
        assert_eq!(titles(&games), [("Elden Ring", 1), ("Hades", 1), ("Portal 2", 1)], "a record makes no card");
        assert!(merges.is_empty());
        assert_eq!(reports(&games[0]), [("gfn", None, true), ("steam", Some(9), false)], "its own source's, then the account's");
        assert!(games[1].reports.is_empty(), "a record with no id, or one no game has, goes nowhere");
        assert_eq!(reports(&games[2]), [("steam", Some(5), false)]);
        assert_eq!(identity.game_of(&record("steam", "620", &[], Activity::default()).key()), None, "a record is given no game id");
    }

    #[test]
    fn a_record_that_two_games_could_own_goes_on_neither_and_a_launcher_takes_none() {
        let mut identity = Identity::default();
        let apart = source("punktfunk", "pc1", "elden", "Elden Ring", &["steam:1245620"]);
        identity.unlink(apart.key());
        let (games, _) = build(
            &mut identity,
            vec![
                source("gfn", "acct", "100", "Elden Ring", &["steam:1245620"]),
                apart,
                launcher("punktfunk", "pc1", "bigpicture", "Steam Big Picture", &["steam:7"]),
                source("gfn", "acct", "200", "Hades", &["steam:1145360"]),
                source("gfn", "acct", "300", "Portal 2", &["steam:620"]),
                record("steam", "1245620", &["steam:1245620"], played(9)),
                record("steam", "7", &["steam:7"], played(9)),
                record("steam", "two", &["steam:1145360", "steam:620"], played(9)),
            ],
        );
        assert_eq!(games.len(), 5);
        assert!(games.iter().all(|g| g.reports.is_empty()), "the owner kept two games apart, a record names two, a launcher is no game");
    }

    #[test]
    fn a_game_keeps_its_id_while_its_sources_come_and_go() {
        let mut identity = Identity::default();
        let gfn = source("gfn", "acct", "100", "Elden Ring", &["steam:1245620"]);
        let pc = source("punktfunk", "pc1", "steam:1245620", "Elden Ring", &["steam:1245620"]);
        let other = source("gfn", "acct", "200", "Hades", &["steam:1145360"]);

        let (games, _) = build(&mut identity, vec![other.clone(), gfn.clone()]);
        let (elden, hades) = (games[0].id, games[1].id);
        // The PC is asleep and GeForce NOW is switched off: only the PC's saved entry is listed.
        let (games, _) = build(&mut identity, vec![pc.clone()]);
        assert_eq!(games[0].id, elden);
        let (games, merges) = build(&mut identity, vec![pc, gfn, other]);
        assert_eq!(games.iter().map(|g| g.id).collect::<Vec<_>>(), [elden, hades]);
        assert!(merges.is_empty());
        assert_eq!(serde_json::from_str::<Identity>(&serde_json::to_string(&identity).unwrap()).unwrap(), identity);
    }

    #[test]
    fn a_source_that_carries_two_games_ids_joins_them_into_the_older() {
        let mut identity = Identity::default();
        let pc = source("punktfunk", "pc1", "steam:1245620", "Elden Ring", &["steam:1245620"]);
        let cloud_then = source("gfn", "acct", "100", "ELDEN RING", &["gfn:100"]);
        let (games, _) = build(&mut identity, vec![pc.clone(), cloud_then]);
        assert_eq!(games.len(), 2, "nothing says they are one game yet");
        let (cloud_game, pc_game) = (games[0].id, games[1].id);
        assert!(cloud_game < pc_game);

        // GeForce NOW now also reports the Steam id.
        let cloud_now = source("gfn", "acct", "100", "ELDEN RING", &["gfn:100", "steam:1245620"]);
        let (games, merges) = build(&mut identity, vec![pc.clone(), cloud_now.clone()]);
        assert_eq!(titles(&games), [("ELDEN RING", 2)]);
        assert_eq!(merges, [Merge { from: pc_game, into: cloud_game }]);
        assert_eq!(identity.game_of(&pc.key()), Some(cloud_game));

        let (games, merges) = build(&mut identity, vec![pc, cloud_now]);
        assert_eq!((games[0].id, merges.len()), (cloud_game, 0), "a merge is reported once");
    }

    #[test]
    fn the_owners_word_outranks_the_ids_both_ways() {
        let mut identity = Identity::default();
        let cloud = source("gfn", "acct", "100", "Elden Ring", &["steam:1245620"]);
        let pc = source("punktfunk", "pc1", "steam:1245620", "Elden Ring (modded)", &["steam:1245620"]);
        let stream = source("gamestream", "pc1", "881", "Elden Ring", &[]);
        let all = || vec![cloud.clone(), pc.clone(), stream.clone()];

        let (games, _) = build(&mut identity, all());
        assert_eq!(titles(&games), [("Elden Ring", 1), ("Elden Ring", 2)]);
        let merged = games[1].id;

        identity.link(stream.key(), merged);
        let modded = identity.unlink(pc.key());
        let (games, merges) = build(&mut identity, all());
        assert_eq!(titles(&games), [("Elden Ring", 2), ("Elden Ring (modded)", 1)]);
        assert_eq!((games[0].id, games[1].id), (merged, modded));
        assert!(merges.is_empty());
        assert_eq!(identity.game_of(&stream.key()), Some(merged));
    }

    #[test]
    fn titles_sort_without_their_articles_and_punctuation() {
        assert_eq!(sort_key("The Witcher\u{ae} 3: Wild Hunt"), "witcher 3 wild hunt");
        assert_eq!(sort_key("  A Plague Tale -- Requiem "), "plague tale requiem");
        assert_eq!(sort_key("An"), "an");
        assert_eq!(sort_key("\u{c9}cole"), "\u{e9}cole");
        let mut identity = Identity::default();
        let (games, _) = build(
            &mut identity,
            vec![source("gfn", "a", "1", "The Zone", &[]), source("gfn", "a", "2", "Apex", &[]), source("gfn", "a", "3", "A Way Out", &[])],
        );
        assert_eq!(titles(&games), [("Apex", 1), ("A Way Out", 1), ("The Zone", 1)]);
    }

    #[test]
    fn artwork_prefers_the_steam_set_and_falls_back_to_each_sources_own() {
        let mut cloud = source("gfn", "acct", "100", "Elden Ring", &["gfn:100", "steam:1245620"]);
        cloud.entry.art.cover = Some("https://example.invalid/gfn-cover.jpg".into());
        cloud.entry.art.icon = Some("https://example.invalid/gfn-icon.png".into());
        let mut pc = source("punktfunk", "pc1", "steam:1245620", "Elden Ring", &["steam:1245620"]);
        pc.entry.art.cover = cloud.entry.art.cover.clone();
        let (games, _) = build(&mut Identity::default(), vec![cloud, pc]);
        assert_eq!(
            games[0].art(ArtKind::Cover),
            ["https://cdn.cloudflare.steamstatic.com/steam/apps/1245620/library_600x900_2x.jpg", "https://example.invalid/gfn-cover.jpg"]
        );
        assert_eq!(games[0].art(ArtKind::Icon), ["https://example.invalid/gfn-icon.png"]);
        assert_eq!(games[0].art(ArtKind::Hero).len(), 1);

        let (games, _) = build(&mut Identity::default(), vec![source("gamestream", "pc1", "881", "Elden Ring", &[])]);
        assert!(games[0].art(ArtKind::Cover).is_empty(), "the app draws its own card for a game with no artwork");
    }

    #[test]
    fn saves_are_called_shared_only_when_every_part_of_that_is_known() {
        let copy = |store: Option<&str>, account: Option<&str>| {
            let mut entry = source("gfn", "a", "1", "Hades", &[]).entry;
            entry.store = store.map(str::to_owned);
            entry.store_account = account.map(str::to_owned);
            entry
        };
        let steam_me = copy(Some("steam"), Some("me"));
        assert_eq!(save_compat(&steam_me, &copy(Some("steam"), Some("me")), Some(true)), SaveCompat::Shared);
        assert_eq!(save_compat(&steam_me, &copy(Some("steam"), Some("me")), None), SaveCompat::Unknown);
        assert_eq!(save_compat(&steam_me, &copy(Some("steam"), Some("me")), Some(false)), SaveCompat::Separate);
        assert_eq!(save_compat(&steam_me, &copy(Some("steam"), None), Some(true)), SaveCompat::Unknown);
        assert_eq!(save_compat(&steam_me, &copy(Some("steam"), Some("you")), Some(true)), SaveCompat::Separate);
        assert_eq!(save_compat(&steam_me, &copy(Some("epic"), Some("me")), Some(true)), SaveCompat::Separate);
        assert_eq!(save_compat(&steam_me, &copy(None, None), Some(true)), SaveCompat::Unknown);
    }

    #[test]
    fn the_saved_listing_is_replaced_per_target_and_hides_what_is_switched_off() {
        let mut listing = Listing::default();
        let (gfn, pf) = (EngineId::new("gfn").unwrap(), EngineId::new("punktfunk").unwrap());
        let (acct, pc1, pc2) = (TargetId::new("acct").unwrap(), TargetId::new("pc1").unwrap(), TargetId::new("pc2").unwrap());
        let entry = |id: &str| source("gfn", "x", id, id, &[]).entry;
        listing.replace(&gfn, &acct, vec![entry("1"), entry("2")]);
        listing.replace(&pf, &pc1, vec![entry("1")]);
        listing.replace(&pf, &pc2, vec![entry("1")]);
        listing.replace(&gfn, &acct, vec![entry("2"), entry("3")]);
        let ids = |sources: Vec<Source>| sources.iter().map(|s| s.key().to_string()).collect::<Vec<_>>();
        assert_eq!(ids(listing.enabled(&BTreeSet::from([gfn.clone()]))), ["gfn|acct|2", "gfn|acct|3"]);

        let key: SourceKey = "punktfunk|pc1|1".parse().unwrap();
        listing.set_readiness(&key, Readiness::Offline);
        listing.forget(&pf, &pc2);
        let on = listing.enabled(&BTreeSet::from([pf]));
        assert_eq!(ids(on.clone()), ["punktfunk|pc1|1"]);
        assert_eq!(on[0].entry.readiness, Readiness::Offline);
        let mut read_back: Listing = serde_json::from_str(&serde_json::to_string(&listing).unwrap()).unwrap();
        assert_eq!(read_back, listing);
        read_back.unchecked();
        assert!(read_back.sources.iter().all(|s| s.entry.readiness == Readiness::Unknown), "a saved state is not a checked one");
    }

    #[test]
    fn a_derived_id_is_the_same_every_time_and_differs_by_part() {
        assert_eq!(derived_id(&["Elden Ring", "C:\\Games\\elden.exe"]).as_str(), derived_id(&["Elden Ring", "C:\\Games\\elden.exe"]).as_str());
        assert_ne!(derived_id(&["ab", "c"]), derived_id(&["a", "bc"]));
        assert_eq!(derived_id(&[]).as_str(), "dcbf29ce484222325");
    }
}
