//! The guide: the owner's sources asked in turn and kept on disk, and the listings they make for the tuner's channels.
//!
//! **What is kept.** For each source, the antenna lineup of the zip code, its stations and a week of airings, one document a UTC day
//! (`<source>/<date>.json`, by the source's station keys so that a rescan of the tuner needs no new fetch), and `state.json` with when
//! each was fetched and what went wrong. The documents are written only when they change.
//!
//! **How often.** A source is asked at most every [`REFRESH_MS`]: then today and tomorrow are fetched again and the later days once a
//! day; a station list is fetched again daily. A failed request waits [`RETRY_MS`], credentials that were turned down wait for new ones;
//! an owner's "refresh now" still waits [`SOONEST_MS`] after the last try. What the host does between refreshes costs no request.
//!
//! **The listings.** The airings of every source are mapped to the tuner's channels ([`crate::map`]) and merged in the owner's order
//! ([`crate::merge`]): an immutable [`Listings`] the host shares with its readers and lays over the broadcast guide.

use crate::map::{self, ChannelMap};
use crate::merge;
use crate::model::{Channel, Programme};
use crate::net::Http;
use crate::provider::gracenote::Gracenote;
use crate::provider::plex::Plex;
use crate::provider::sd::SchedulesDirect;
use crate::provider::{Ctx, Error, Lineup, Provider, Source, Station};
use crate::store::Store;
use crate::time::{DAY_MS, HOUR_MS, MINUTE_MS, date, day_start};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;

/// Days of listings fetched, today (UTC) first.
pub const DAYS: i64 = 7;
/// The most often a source is asked.
pub const REFRESH_MS: i64 = 6 * HOUR_MS;
/// After an owner's "refresh now", a source is still not asked sooner than this after its last try.
pub const SOONEST_MS: i64 = HOUR_MS;
/// After a request that failed on the way, the source is asked again this much later.
pub const RETRY_MS: i64 = 30 * MINUTE_MS;
/// Credentials that were turned down are tried again after this long, or as soon as the owner types new ones.
const CREDENTIALS_RETRY_MS: i64 = 24 * HOUR_MS;
/// A lineup's stations are asked for again after this long.
const STATIONS_MS: i64 = 24 * HOUR_MS;
/// The days after the next two are fetched again after this long.
const LATER_DAYS_MS: i64 = 24 * HOUR_MS;
/// A day fetched this little short of its age is fetched again, so that a refresh on the dot finds it due.
const SLACK_MS: i64 = 5 * MINUTE_MS;
const STATE: &str = "state.json";

/// One source as the owner set it up.
#[derive(Clone, PartialEq, Eq)]
pub enum SourceSettings {
    /// A Plex server on the home network (`192.168.1.69:32400`) and its token.
    Plex {
        address: String,
        token: String,
    },
    SchedulesDirect {
        username: String,
        password: String,
    },
    Gracenote,
}

impl fmt::Debug for SourceSettings {
    /// Never says a token or a password.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceSettings::Plex { address, token } => f
                .debug_struct("Plex")
                .field("address", address)
                .field("token", &(!token.is_empty()))
                .finish(),
            SourceSettings::SchedulesDirect { username, password } => f
                .debug_struct("SchedulesDirect")
                .field("username", username)
                .field("password", &(!password.is_empty()))
                .finish(),
            SourceSettings::Gracenote => f.write_str("Gracenote"),
        }
    }
}

impl SourceSettings {
    pub fn source(&self) -> Source {
        match self {
            SourceSettings::Plex { .. } => Source::Plex,
            SourceSettings::SchedulesDirect { .. } => Source::SchedulesDirect,
            SourceSettings::Gracenote => Source::Gracenote,
        }
    }

    /// What is still to be typed before the source can be asked, in words; `None` when nothing is.
    pub fn missing(&self) -> Option<&'static str> {
        match self {
            SourceSettings::Plex { address, .. } if address.trim().is_empty() => {
                Some("the Plex server's address")
            }
            SourceSettings::Plex { token, .. } if token.trim().is_empty() => Some("the Plex token"),
            SourceSettings::SchedulesDirect { username, .. } if username.trim().is_empty() => {
                Some("the Schedules Direct user name")
            }
            SourceSettings::SchedulesDirect { password, .. } if password.is_empty() => {
                Some("the Schedules Direct password")
            }
            _ => None,
        }
    }

    /// What the source's kept state belongs to, with no secret in it: another zip code, server or account starts it over.
    fn made_for(&self, zip: &str) -> String {
        match self {
            SourceSettings::Plex { address, .. } => format!("{zip}|{}", address.trim()),
            SourceSettings::SchedulesDirect { username, .. } => {
                format!("{zip}|{}", username.trim())
            }
            SourceSettings::Gracenote => zip.to_owned(),
        }
    }

    fn provider(&self) -> Box<dyn Provider> {
        match self {
            SourceSettings::Plex { address, token } => Box::new(Plex::new(address, token)),
            SourceSettings::SchedulesDirect { username, password } => {
                Box::new(SchedulesDirect::new(username, password))
            }
            SourceSettings::Gracenote => Box::new(Gracenote),
        }
    }
}

/// The owner's choices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// A US zip code, five digits.
    pub zip: String,
    /// The sources in the owner's order: the first that has a programme for a time wins it.
    pub sources: Vec<SourceSettings>,
}

/// Whether `zip` is a US zip code.
pub fn zip_ok(zip: &str) -> bool {
    zip.len() == 5 && zip.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SourceState {
    made_for: String,
    #[serde(default)]
    lineup: Option<Lineup>,
    #[serde(default)]
    stations: Vec<Station>,
    #[serde(default)]
    stations_at: i64,
    /// The station keys the kept days hold, sorted.
    #[serde(default)]
    wanted: Vec<String>,
    /// When each kept day (`2026-11-05`) was fetched.
    #[serde(default)]
    days: BTreeMap<String, i64>,
    #[serde(default)]
    last_try: i64,
    #[serde(default)]
    last_ok: i64,
    #[serde(default)]
    error: Option<String>,
    /// Not asked before this.
    #[serde(default)]
    next: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    sources: BTreeMap<Source, SourceState>,
}

fn day_key(source: Source, day: &str) -> String {
    format!("{}/{day}.json", source.id())
}

/// The online programmes of the tuner's channels, merged across the sources: what readers share.
#[derive(Debug, Default)]
pub struct Listings {
    by_channel: HashMap<String, Vec<Programme>>,
}

impl Listings {
    /// Whether no channel has a programme.
    pub fn is_empty(&self) -> bool {
        self.by_channel.is_empty()
    }

    /// The channels that have programmes.
    pub fn channels(&self) -> usize {
        self.by_channel.len()
    }

    /// The online programmes of `channel` on the air at some time in `[from, to)`, by start.
    pub fn programmes(&self, channel: &str, from: i64, to: i64) -> &[Programme] {
        let Some(list) = self.by_channel.get(channel) else {
            return &[];
        };
        // the list is sorted and no two overlap, so the ends rise with the starts
        let a = list.partition_point(|p| p.end <= from);
        let b = list.partition_point(|p| p.start < to).max(a);
        &list[a..b]
    }

    /// `broadcast`, the broadcast guide of `channel` for `[from, to)`, with the online programmes over it.
    pub fn over(
        &self,
        channel: &str,
        broadcast: &[Programme],
        from: i64,
        to: i64,
    ) -> Vec<Programme> {
        merge::over(self.programmes(channel, from, to), broadcast)
    }
}

/// How one source stands, for the owner's page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourceStatus {
    pub source: Source,
    /// The lineup in use.
    pub lineup: Option<String>,
    /// The last refresh that worked, epoch milliseconds.
    pub refreshed: Option<i64>,
    /// The tuner's channels matched to a station, and the numbers of the others (`35.2`).
    pub matched: usize,
    pub unmatched: Vec<String>,
    /// Days of listings kept.
    pub days: usize,
    /// What went wrong last time, or what is still to be typed.
    pub problem: Option<String>,
    /// The source is not set up (`problem` says what to type): it is passed over, and the next one asked.
    pub skipped: bool,
}

pub struct Guide {
    store: Arc<dyn Store>,
    settings: Settings,
    /// The settings were set since the guide was made: a change of a secret after that is the owner's, and is tried at once.
    told: bool,
    providers: Vec<Box<dyn Provider>>,
    channels: Vec<Channel>,
    kept: Kept,
    listings: Arc<Listings>,
}

impl Guide {
    /// The guide kept in `store`, with no settings and no channels yet.
    pub fn new(store: Arc<dyn Store>) -> Self {
        let kept = store
            .load(STATE)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            store,
            settings: Settings::default(),
            told: false,
            providers: Vec::new(),
            channels: Vec::new(),
            kept,
            listings: Arc::default(),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The owner's settings. A source that is new, or whose zip code, server or account changed, starts over; one whose secret changed
    /// is asked at the next refresh; one that is gone is forgotten with its days. The first settings after a start keep the times kept.
    pub fn set_settings(&mut self, settings: Settings) {
        if self.told && settings == self.settings {
            return;
        }
        let old = std::mem::replace(&mut self.settings, settings);
        let told = std::mem::replace(&mut self.told, true);
        self.providers = self
            .settings
            .sources
            .iter()
            .map(SourceSettings::provider)
            .collect();
        for s in &self.settings.sources {
            let made_for = s.made_for(&self.settings.zip);
            let state = self.kept.sources.entry(s.source()).or_default();
            if state.made_for != made_for {
                forget_days(&*self.store, s.source(), state);
                *state = SourceState {
                    made_for,
                    ..SourceState::default()
                };
            } else if told && !old.sources.contains(s) {
                state.next = 0;
            }
        }
        let gone: Vec<Source> = self
            .kept
            .sources
            .keys()
            .filter(|k| !self.settings.sources.iter().any(|s| s.source() == **k))
            .copied()
            .collect();
        for source in gone {
            if let Some(mut state) = self.kept.sources.remove(&source) {
                forget_days(&*self.store, source, &mut state);
            }
        }
        self.save();
        self.rebuild();
    }

    /// The tuner's channels.
    pub fn set_channels(&mut self, channels: Vec<Channel>) {
        if channels != self.channels {
            self.channels = channels;
            self.rebuild();
        }
    }

    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    /// When the next source is due, epoch milliseconds; `None` while there is nothing to ask.
    pub fn due(&self) -> Option<i64> {
        if !zip_ok(&self.settings.zip) || self.channels.is_empty() {
            return None;
        }
        self.settings
            .sources
            .iter()
            .filter(|s| s.missing().is_none())
            .map(|s| self.kept.sources.get(&s.source()).map_or(0, |st| st.next))
            .min()
    }

    /// The owner asked for a refresh: every source is due now, or [`SOONEST_MS`] after its last try.
    pub fn refresh_soon(&mut self, now: i64) {
        for state in self.kept.sources.values_mut() {
            state.next = state.next.min(now.max(state.last_try + SOONEST_MS));
        }
    }

    /// Asks every source that is due at `now`; whether the listings changed.
    pub fn refresh(&mut self, net: &dyn Http, now: i64) -> bool {
        if self.due().is_none_or(|due| due > now) {
            return false;
        }
        let Guide {
            store,
            settings,
            providers,
            channels,
            kept,
            ..
        } = self;
        let cx = Ctx { net, now };
        let mut changed = false;
        for (s, provider) in settings.sources.iter().zip(providers.iter_mut()) {
            let state = kept.sources.entry(s.source()).or_default();
            if s.missing().is_some() || state.next > now {
                continue;
            }
            state.last_try = now;
            match refresh_source(
                &mut **provider,
                state,
                &settings.zip,
                channels,
                cx,
                &**store,
                &mut changed,
            ) {
                Ok(()) => {
                    state.error = None;
                    state.last_ok = now;
                    state.next = now + REFRESH_MS;
                }
                Err(e) => {
                    state.next = now
                        + match e {
                            Error::Net(_) => RETRY_MS,
                            Error::Credentials(_) => CREDENTIALS_RETRY_MS,
                            _ => REFRESH_MS,
                        };
                    state.error = Some(e.to_string());
                }
            }
        }
        self.save();
        if changed {
            self.rebuild();
        }
        changed
    }

    /// Everything kept goes: the plugin was switched off.
    pub fn clear(&mut self) {
        for (source, state) in &mut self.kept.sources {
            forget_days(&*self.store, *source, state);
        }
        self.kept = Kept::default();
        self.store.remove(STATE);
        self.settings = Settings::default();
        self.providers.clear();
        self.listings = Arc::default();
    }

    /// The listings as they stand: share them, they never change.
    pub fn listings(&self) -> Arc<Listings> {
        Arc::clone(&self.listings)
    }

    /// How each source stands, in the owner's order.
    pub fn status(&self) -> Vec<SourceStatus> {
        self.settings
            .sources
            .iter()
            .map(|s| {
                let state = self.kept.sources.get(&s.source());
                let map = state.map_or_else(ChannelMap::default, |st| {
                    map::match_channels(&self.channels, &st.stations)
                });
                let numbers = |ids: &[String]| -> Vec<String> {
                    let mut out: Vec<&Channel> = self
                        .channels
                        .iter()
                        .filter(|c| ids.contains(&c.id))
                        .collect();
                    out.sort_by_key(|c| (c.major, c.minor));
                    out.into_iter().map(Channel::number).collect()
                };
                let fetched = state.is_some_and(|st| !st.stations.is_empty());
                SourceStatus {
                    source: s.source(),
                    lineup: state
                        .and_then(|st| st.lineup.as_ref())
                        .map(|l| l.name.clone()),
                    refreshed: state.map(|st| st.last_ok).filter(|t| *t > 0),
                    matched: map.matched.len(),
                    unmatched: if fetched {
                        numbers(&map.unmatched)
                    } else {
                        Vec::new()
                    },
                    days: state.map_or(0, |st| st.days.len()),
                    problem: s
                        .missing()
                        .map(|m| format!("Type {m}"))
                        .or_else(|| state.and_then(|st| st.error.clone())),
                    skipped: s.missing().is_some(),
                }
            })
            .collect()
    }

    fn save(&self) {
        if let Ok(bytes) = serde_json::to_vec(&self.kept) {
            let _ = self.store.save(STATE, &bytes);
        }
    }

    /// The listings again from the days kept, the stations and the channels.
    fn rebuild(&mut self) {
        let mut layers: Vec<HashMap<String, Vec<Programme>>> = Vec::new();
        for s in &self.settings.sources {
            let source = s.source();
            let Some(state) = self.kept.sources.get(&source) else {
                continue;
            };
            let map = map::match_channels(&self.channels, &state.stations);
            let mut channels_of: HashMap<&str, Vec<&str>> = HashMap::new();
            for m in &map.matched {
                channels_of
                    .entry(m.station.as_str())
                    .or_default()
                    .push(m.channel.as_str());
            }
            let mut layer: HashMap<String, Vec<Programme>> = HashMap::new();
            for day in state.days.keys() {
                let Some(bytes) = self.store.load(&day_key(source, day)) else {
                    continue;
                };
                let Ok(list) = serde_json::from_slice::<Vec<Programme>>(&bytes) else {
                    continue;
                };
                for p in list {
                    let Some(channels) = channels_of.get(p.channel.as_str()) else {
                        continue;
                    };
                    for channel in channels {
                        layer
                            .entry((*channel).to_owned())
                            .or_default()
                            .push(Programme {
                                channel: (*channel).to_owned(),
                                ..p.clone()
                            });
                    }
                }
            }
            for list in layer.values_mut() {
                list.sort_by_key(|p| p.start);
            }
            layers.push(layer);
        }
        let mut by_channel = HashMap::new();
        for c in &self.channels {
            let lists: Vec<&[Programme]> = layers
                .iter()
                .filter_map(|l| l.get(&c.id).map(Vec::as_slice))
                .collect();
            if !lists.is_empty() {
                by_channel.insert(c.id.clone(), merge::merge(&lists));
            }
        }
        self.listings = Arc::new(Listings { by_channel });
    }
}

fn forget_days(store: &dyn Store, source: Source, state: &mut SourceState) {
    for day in std::mem::take(&mut state.days).keys() {
        store.remove(&day_key(source, day));
    }
}

/// One source asked: its lineup and stations when they are not kept or are old, then the days that are due. `changed` says whether a
/// kept day changed, even when a later request failed.
fn refresh_source(
    provider: &mut dyn Provider,
    state: &mut SourceState,
    zip: &str,
    channels: &[Channel],
    cx: Ctx<'_>,
    store: &dyn Store,
    changed: &mut bool,
) -> Result<(), Error> {
    let source = provider.source();
    let lineup = match &state.lineup {
        Some(lineup) => lineup.clone(),
        None => {
            let found = provider.lineups(cx, zip)?.into_iter().find(|l| l.antenna);
            let lineup = found.ok_or_else(|| {
                Error::Nothing(format!(
                    "{} has no antenna listings for {zip}",
                    source.label()
                ))
            })?;
            state.lineup = Some(lineup.clone());
            state.stations.clear();
            lineup
        }
    };
    if state.stations.is_empty() || cx.now - state.stations_at >= STATIONS_MS {
        state.stations = provider.stations(cx, &lineup)?;
        state.stations_at = cx.now;
    }
    let map = map::match_channels(channels, &state.stations);
    let wanted: Vec<String> = map.stations().into_iter().map(str::to_owned).collect();
    if wanted.is_empty() {
        return Err(Error::Nothing(format!(
            "none of the TV's channels is in {}",
            lineup.name
        )));
    }
    if wanted != state.wanted {
        forget_days(store, source, state);
        *changed = true;
        state.wanted = wanted;
    }
    let stations: Vec<&Station> = state
        .stations
        .iter()
        .filter(|s| state.wanted.binary_search(&s.key).is_ok())
        .collect();
    let today = day_start(cx.now);
    for d in 0..DAYS {
        let day = today + d * DAY_MS;
        let key = date(day);
        let age = if d < 2 { REFRESH_MS } else { LATER_DAYS_MS };
        if state
            .days
            .get(&key)
            .is_some_and(|at| cx.now - at < age - SLACK_MS)
        {
            continue;
        }
        let airings = provider.airings(cx, &lineup, &stations, day, day + DAY_MS)?;
        let bytes = serde_json::to_vec(&airings).map_err(|e| Error::Shape(e.to_string()))?;
        let doc = day_key(source, &key);
        if store.load(&doc).as_deref() != Some(bytes.as_slice()) {
            store
                .save(&doc, &bytes)
                .map_err(|e| Error::Refused(format!("the listings could not be kept ({e})")))?;
            *changed = true;
        }
        state.days.insert(key, cx.now);
    }
    // yesterday stays for what started then and runs into today; anything older, or not counted, goes
    let oldest = date(today - DAY_MS);
    state.days.retain(|day, _| *day >= oldest);
    for doc in store.keys(&format!("{}/", source.id())) {
        let day = doc
            .trim_start_matches(&format!("{}/", source.id()))
            .trim_end_matches(".json")
            .to_owned();
        if !state.days.contains_key(&day) {
            store.remove(&doc);
            *changed = true;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::fake::{NOW, Net};
    use crate::store::MemStore;
    use crate::time::parse_utc;

    const PLEX_DVRS: &str = include_str!("../fixtures/plex/dvrs.json");
    const PLEX_LINEUPS: &str = include_str!("../fixtures/plex/lineups.json");
    const PLEX_CHANNELS: &str = include_str!("../fixtures/plex/channels.json");
    const PLEX_GRID: &str = include_str!("../fixtures/plex/grid.json");
    const GN_PROVIDERS: &str = include_str!("../fixtures/gracenote/providers.json");
    const GN_GRID: &str = include_str!("../fixtures/gracenote/grid.json");

    fn net() -> Net {
        Net::new()
            .on(&["/lineups?postalCode=30311"], 200, PLEX_LINEUPS)
            .on(&["/livetv/dvrs"], 200, PLEX_DVRS)
            .on(&["/lineups/dvr/channels"], 200, PLEX_CHANNELS)
            .on(&["/tv.plex.providers.epg.cloud:5/grid?"], 200, PLEX_GRID)
            .on(&["/getPostalCodeProviders/USA/30311/"], 200, GN_PROVIDERS)
            .on(&["/api/grid?"], 200, GN_GRID)
    }

    fn channels() -> Vec<Channel> {
        let c = |id: &str, major, minor, name: &str| Channel {
            id: id.into(),
            major,
            minor,
            name: name.into(),
            rf: None,
        };
        vec![
            c("1_39_2_1_0_0_0", 2, 1, "WSB-HD"),
            c("1_39_2_2_0_0_0", 2, 2, "BOUNCE"),
            c("1_27_5_1_0_0_0", 5, 1, "WAGA-HD"),
            c("1_10_11_11_0_0_0", 11, 11, "WXIA-TV"),
            c("1_21_35_2_0_0_0", 35, 2, "ESPANOL"),
        ]
    }

    fn settings(sources: Vec<SourceSettings>) -> Settings {
        Settings {
            zip: "30311".into(),
            sources,
        }
    }

    fn plex() -> SourceSettings {
        SourceSettings::Plex {
            address: "192.168.1.69:32400".into(),
            token: "tk".into(),
        }
    }

    fn guide(store: &Arc<MemStore>, sources: Vec<SourceSettings>) -> Guide {
        let mut g = Guide::new(Arc::clone(store) as Arc<dyn Store>);
        g.set_settings(settings(sources));
        g.set_channels(channels());
        g
    }

    #[test]
    fn a_refresh_fetches_a_week_and_the_first_source_wins_where_both_have_a_programme() {
        let store = Arc::new(MemStore::new());
        let mut g = guide(&store, vec![plex(), SourceSettings::Gracenote]);
        let net = net();
        assert!(g.refresh(&net, NOW));
        assert_eq!(store.keys("plex/").len(), 7);
        assert_eq!(store.keys("gracenote/").len(), 7);

        let at = parse_utc("2026-11-05T18:00:00Z").unwrap();
        let l = g.listings();
        let wsb = l.programmes("1_39_2_1_0_0_0", at, at + 3 * HOUR_MS);
        assert_eq!(
            wsb.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(),
            [
                "GMA3",
                "General Hospital",
                "Channel 2 Action News at 3:00PM"
            ]
        );
        // Plex's (first) has no captions flag; Gracenote's (second) would have
        assert!(wsb[0].id.starts_with("plex://"));
        assert_eq!(wsb[0].channel, "1_39_2_1_0_0_0");
        // 11.11 is in Gracenote's fixture, not in Plex's grid: the second source fills it
        assert_eq!(
            l.programmes("1_10_11_11_0_0_0", at, at + HOUR_MS)[0].title,
            "NBC News Daily"
        );

        let status = g.status();
        assert_eq!(status[0].source, Source::Plex);
        assert_eq!(
            (status[0].matched, status[0].unmatched.as_slice()),
            (4, &["35.2".to_owned()][..])
        );
        assert_eq!(
            status[0].lineup.as_deref(),
            Some("Local Broadcast Listings (156 channels)")
        );
        assert_eq!(
            (
                status[0].refreshed,
                status[0].days,
                status[0].problem.as_deref()
            ),
            (Some(NOW), 7, None)
        );
    }

    #[test]
    fn nothing_is_asked_again_before_the_refresh_is_due_and_then_only_the_next_two_days() {
        let store = Arc::new(MemStore::new());
        let mut g = guide(&store, vec![SourceSettings::Gracenote]);
        let net = net();
        g.refresh(&net, NOW);
        let first = net.urls().len();
        assert_eq!(first, 1 + 1 + 7 * 4, "lineups, stations, four grids a day");
        assert!(!g.refresh(&net, NOW + REFRESH_MS - MINUTE_MS));
        assert_eq!(net.urls().len(), first);
        assert_eq!(g.due(), Some(NOW + REFRESH_MS));
        g.refresh(&net, NOW + REFRESH_MS);
        assert_eq!(net.urls().len(), first + 2 * 4);
    }

    #[test]
    fn the_listings_come_back_from_the_store_without_a_request() {
        let store = Arc::new(MemStore::new());
        guide(&store, vec![SourceSettings::Gracenote]).refresh(&net(), NOW);
        let again = guide(&store, vec![SourceSettings::Gracenote]);
        let at = parse_utc("2026-11-05T18:00:00Z").unwrap();
        assert_eq!(
            again
                .listings()
                .programmes("1_39_2_2_0_0_0", at, at + HOUR_MS)[0]
                .subtitle,
            "Citizen Trundel"
        );
        assert_eq!(again.due(), Some(NOW + REFRESH_MS));
    }

    #[test]
    fn a_source_not_set_up_is_passed_over_with_a_word_and_the_next_one_fetched() {
        let store = Arc::new(MemStore::new());
        let unset = SourceSettings::Plex {
            address: String::new(),
            token: String::new(),
        };
        let mut g = guide(&store, vec![unset, SourceSettings::Gracenote]);
        assert_eq!(g.due(), Some(0));
        assert!(g.refresh(&net(), NOW));
        assert_eq!(store.keys("gracenote/").len(), 7);
        let status = g.status();
        assert_eq!(
            (status[0].problem.as_deref(), status[0].skipped),
            (Some("Type the Plex server's address"), true)
        );
        assert_eq!((status[1].refreshed, status[1].skipped), (Some(NOW), false));
    }

    #[test]
    fn a_failure_keeps_what_was_kept_and_says_why_and_new_credentials_are_tried_at_once() {
        let store = Arc::new(MemStore::new());
        let mut g = guide(&store, vec![plex()]);
        g.refresh(&net(), NOW);
        let refused = Net::new().on(&["/"], 401, "");
        g.refresh(&refused, NOW + REFRESH_MS);
        let status = &g.status()[0];
        assert_eq!(
            status.problem.as_deref(),
            Some("the Plex server turned the token down")
        );
        assert_eq!(status.refreshed, Some(NOW));
        assert!(!g.listings().is_empty());
        assert_eq!(g.due(), Some(NOW + REFRESH_MS + CREDENTIALS_RETRY_MS));
        g.set_settings(settings(vec![SourceSettings::Plex {
            address: "192.168.1.69:32400".into(),
            token: "new".into(),
        }]));
        assert_eq!(g.due(), Some(0));
    }

    #[test]
    fn another_zip_code_starts_over_and_a_source_taken_off_is_forgotten() {
        let store = Arc::new(MemStore::new());
        let mut g = guide(&store, vec![SourceSettings::Gracenote, plex()]);
        g.refresh(&net(), NOW);
        g.set_settings(settings(vec![plex()]));
        assert!(store.keys("gracenote/").is_empty());
        assert_eq!(store.keys("plex/").len(), 7);
        g.set_settings(Settings {
            zip: "30303".into(),
            sources: vec![plex()],
        });
        assert!(store.keys("plex/").is_empty());
        assert!(g.listings().is_empty());
        assert_eq!(g.status()[0].problem, None);
        let missing = SourceSettings::SchedulesDirect {
            username: "nitin".into(),
            password: String::new(),
        };
        g.set_settings(settings(vec![missing]));
        assert_eq!(
            g.status()[0].problem.as_deref(),
            Some("Type the Schedules Direct password")
        );
        assert_eq!(g.due(), None);
    }

    #[test]
    fn the_settings_never_show_a_secret() {
        let shown = format!(
            "{:?}",
            settings(vec![
                plex(),
                SourceSettings::SchedulesDirect {
                    username: "nitin".into(),
                    password: "hunter2".into()
                }
            ])
        );
        assert!(
            !shown.contains("tk\"") && !shown.contains("hunter2"),
            "{shown}"
        );
    }

    #[test]
    fn the_online_programmes_lie_over_the_broadcast_guide() {
        let store = Arc::new(MemStore::new());
        let mut g = guide(&store, vec![SourceSettings::Gracenote]);
        g.refresh(&net(), NOW);
        let at = parse_utc("2026-11-05T18:00:00Z").unwrap();
        let broadcast = |title: &str, start: i64, end: i64| Programme {
            id: title.into(),
            channel: "1_27_5_1_0_0_0".into(),
            title: title.into(),
            subtitle: String::new(),
            text: String::new(),
            start,
            end,
            rating: String::new(),
            captions: false,
            described: false,
        };
        let tv = [
            broadcast("PORTIA", at, at + HOUR_MS),
            broadcast("LATE SHOW", at + 2 * HOUR_MS, at + 3 * HOUR_MS),
        ];
        let merged = g
            .listings()
            .over("1_27_5_1_0_0_0", &tv, at, at + 3 * HOUR_MS);
        assert_eq!(
            merged.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(),
            ["Portia", "Portia", "TMZ Live", "LATE SHOW"]
        );
    }
}
