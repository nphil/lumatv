//! What Home Assistant's LumaShow integration has given the TV, kept whole: the shared entities with the attributes their tiles are made
//! from, the services the TV may call on each, and the areas they stand in. No I/O.
//!
//! This is `ha-bridge`'s own mirror with one difference: that one keeps six attributes (what the Show's lease needs) and drops a
//! thermostat's temperatures, a cover's position, a fan's speed and a player's title, which a Home tile says. Everything else is the
//! same contract (`lumashow-ha` `docs/protocol.md` section 4): the mirror is exactly the last snapshot brought up to date by the updates,
//! a snapshot may come in parts and replaces the set when the last arrives, and the whole mirror is dropped when the session ends.
//!
//! **Only what Home can show is kept.** An entity of a domain the TV does not carry (a camera, an alarm panel, a siren, a valve, a
//! button) is dropped on arrival; the TV has no tile for it and no way to ask anything of it. A service the integration offers only with
//! a confirmation (`ctl_pin`) is dropped too, but for a lock's `unlock`, which only the TV's own screen asks for, after a question.

use super::tile::{self, Ent};
use ha_bridge::wire::{AreaRec, EntityRec, MAX_ENTITIES, Snapshot, StateRec, Update};
use serde_json::Value;
#[cfg(test)]
use serde_json::Map;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

const MAX_NAME_CHARS: usize = 128;

/// One shared entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    /// An id of the mirror's areas.
    pub area: Option<String>,
    pub ent: Ent,
    /// The services the integration lets the TV call now.
    pub ctl: Vec<String>,
    /// The services it lets a person at the TV do after a confirmation (`ctl_pin`), kept for a lock's `unlock` and nothing else: the only
    /// sensitive verb the TV ever uses, and only from its own screen (`OnHome::unlock`).
    pub ctl_pin: Vec<String>,
    /// Hidden, configuration or diagnostic: a picker leaves it out.
    pub minor: bool,
    pub changed: Option<f64>,
}

/// What a part of a snapshot did.
#[derive(Debug, PartialEq, Eq)]
pub enum Snap {
    Waiting,
    Applied,
    /// Not taken, and why: the caller asks for the set again.
    Refused(&'static str),
}

struct Partial {
    since: Instant,
    next_part: u32,
    rev: u64,
    paused: bool,
    areas: BTreeMap<String, String>,
    items: BTreeMap<String, Item>,
}

#[derive(Default)]
pub struct Mirror {
    items: BTreeMap<String, Item>,
    areas: BTreeMap<String, String>,
    paused: bool,
    rev: u64,
    partial: Option<Partial>,
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn item_of(rec: EntityRec) -> Item {
    let lock = tile::domain_of(&rec.id) == Some("lock");
    Item {
        name: cut(&rec.name, MAX_NAME_CHARS),
        area: rec.area.filter(|a| !a.is_empty()),
        ent: Ent { state: cut(&rec.state, MAX_NAME_CHARS), attrs: tile::kept(&rec.attrs) },
        // a lock's `unlock` and `open` are not ordinary services whatever the integration lists them under: the TV keeps `lock` only
        ctl: rec.ctl.into_iter().filter(|v| v.len() <= 48 && (!lock || v == "lock")).take(32).collect(),
        ctl_pin: if lock { rec.ctl_pin.into_iter().filter(|v| v == "unlock").collect() } else { Vec::new() },
        minor: rec.minor,
        changed: rec.changed.filter(|c| c.is_finite()),
    }
}

impl Mirror {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// Control is paused in Home Assistant: every `ctl` is empty and the TV only looks.
    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.get(id)
    }

    pub fn items(&self) -> impl Iterator<Item = (&String, &Item)> {
        self.items.iter()
    }

    pub fn area_name(&self, id: &str) -> Option<&str> {
        self.areas.get(id).map(String::as_str)
    }

    /// The areas of the shared entities, by name.
    pub fn areas(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.areas.iter().map(|(i, n)| (i.clone(), n.clone())).collect();
        out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// How many shared entities the TV may call a service on.
    pub fn controllable(&self) -> usize {
        self.items.values().filter(|i| !i.ctl.is_empty()).count()
    }

    /// The unit of the house's temperatures: the one most of the shared temperature sensors say, or nothing (a thermostat's own numbers
    /// then decide, `tile`).
    pub fn temp_unit(&self) -> &'static str {
        let (mut c, mut f) = (0, 0);
        for (id, item) in &self.items {
            if tile::domain_of(id) == Some("sensor") && item.ent.attrs.get("device_class").and_then(Value::as_str) == Some("temperature") {
                match item.ent.attrs.get("unit_of_measurement").and_then(Value::as_str) {
                    Some("\u{b0}C") => c += 1,
                    Some("\u{b0}F") => f += 1,
                    _ => {}
                }
            }
        }
        match (c, f) {
            (0, 0) => "",
            (c, f) if f > c => "\u{b0}F",
            _ => "\u{b0}C",
        }
    }

    /// One part of a snapshot. The set is replaced when the part with `more: false` is taken.
    pub fn snapshot(&mut self, part: Snapshot, now: Instant) -> Snap {
        if part.part == 0 {
            self.partial = Some(Partial { since: now, next_part: 0, rev: part.rev, paused: part.policy.paused, areas: BTreeMap::new(), items: BTreeMap::new() });
        }
        let Some(partial) = self.partial.as_mut() else {
            return Snap::Refused("a part of a snapshot with no beginning");
        };
        if part.part != partial.next_part {
            self.partial = None;
            return Snap::Refused("the parts of a snapshot are out of order");
        }
        partial.next_part += 1;
        for AreaRec { id, name } in part.areas {
            partial.areas.insert(id, cut(&name, MAX_NAME_CHARS));
        }
        for rec in part.entities {
            // an id the TV has no tile for is not kept, however it was shared
            if tile::domain_of(&rec.id).is_none() {
                continue;
            }
            let id = rec.id.clone();
            partial.items.insert(id, item_of(rec));
            if partial.items.len() > MAX_ENTITIES {
                self.partial = None;
                return Snap::Refused("more entities than the TV takes");
            }
        }
        if part.more {
            return Snap::Waiting;
        }
        let Some(done) = self.partial.take() else {
            return Snap::Refused("a snapshot that vanished");
        };
        self.rev = done.rev;
        self.paused = done.paused;
        let used: BTreeSet<&String> = done.items.values().filter_map(|i| i.area.as_ref()).collect();
        self.areas = done.areas.into_iter().filter(|(id, _)| used.contains(id)).collect();
        self.items = done.items;
        Snap::Applied
    }

    /// Whether a snapshot has waited for its next part longer than `within`: it is dropped, and the caller asks again.
    pub fn expire_partial(&mut self, now: Instant, within: Duration) -> bool {
        match &self.partial {
            Some(p) if now.duration_since(p.since) > within => {
                self.partial = None;
                true
            }
            _ => false,
        }
    }

    pub fn partial_since(&self) -> Option<Instant> {
        self.partial.as_ref().map(|p| p.since)
    }

    /// A change of states: whether it changed anything (a rewrite of the same value, an entity that is not shared, changes nothing).
    pub fn update(&mut self, update: Update) -> bool {
        let mut changed = false;
        for StateRec { id, state, attrs, changed: at } in update.entities {
            let Some(item) = self.items.get_mut(&id) else { continue };
            let ent = Ent { state: cut(&state, MAX_NAME_CHARS), attrs: tile::kept(&attrs) };
            let at = at.filter(|c| c.is_finite());
            if item.ent == ent && item.changed == at {
                continue;
            }
            item.ent = ent;
            item.changed = at;
            changed = true;
        }
        for id in update.removed {
            changed |= self.items.remove(&id).is_some();
        }
        self.rev = self.rev.max(update.rev);
        changed
    }

    /// The session is over: nothing it gave is kept. Whether anything was.
    pub fn clear(&mut self) -> bool {
        let had = !self.items.is_empty() || !self.areas.is_empty() || self.partial.is_some();
        *self = Self::default();
        had
    }
}

/// The attributes of an entity as a map, for the tests.
#[cfg(test)]
pub fn attrs(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_bridge::wire::Policy;
    use serde_json::json;

    fn rec(id: &str, state: &str, attrs: Value, ctl: &[&str]) -> EntityRec {
        EntityRec { id: id.into(), name: id.into(), area: Some("kitchen".into()), state: state.into(), attrs: attrs.as_object().cloned().unwrap_or_default(), changed: Some(1.0), minor: false, ctl: ctl.iter().map(|s| (*s).into()).collect(), ctl_pin: vec!["unlock".into()] }
    }

    fn part(n: u32, more: bool, entities: Vec<EntityRec>) -> Snapshot {
        Snapshot { rev: 3, part: n, more, policy: Policy::default(), areas: vec![AreaRec { id: "kitchen".into(), name: "Kitchen".into() }, AreaRec { id: "attic".into(), name: "Attic".into() }], entities }
    }

    #[test]
    fn a_snapshot_in_parts_replaces_the_set_when_the_last_arrives_and_keeps_only_what_the_tv_can_show() {
        let mut m = Mirror::default();
        let t = Instant::now();
        assert_eq!(m.snapshot(part(0, true, vec![rec("light.a", "on", json!({"brightness": 100}), &["turn_on"])]), t), Snap::Waiting);
        assert!(m.is_empty(), "nothing is replaced until the last part");
        let rest = part(1, false, vec![rec("camera.door", "idle", json!({}), &["turn_off"]), rec("lock.front", "locked", json!({}), &["lock"]), rec("siren.s", "off", json!({}), &[]), rec("climate.c", "heat", json!({"temperature": 70.0, "current_temperature": 68.0}), &["set_temperature"])]);
        assert_eq!(m.snapshot(rest, t), Snap::Applied);
        assert_eq!(m.items().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), ["climate.c", "light.a", "lock.front"], "a camera and a siren are not kept");
        assert_eq!(m.get("climate.c").unwrap().ent.attrs["temperature"], json!(70.0), "a thermostat keeps its temperatures");
        assert_eq!(m.areas(), vec![("kitchen".to_owned(), "Kitchen".to_owned())], "an area nothing stands in is not kept");
        assert_eq!(m.controllable(), 3, "the light, the thermostat and the lock (which the TV only looks at)");
    }

    #[test]
    fn parts_out_of_order_and_half_a_snapshot_that_waits_too_long_are_refused() {
        let mut m = Mirror::default();
        let t = Instant::now();
        assert!(matches!(m.snapshot(part(1, false, vec![]), t), Snap::Refused(_)));
        assert_eq!(m.snapshot(part(0, true, vec![]), t), Snap::Waiting);
        assert!(matches!(m.snapshot(part(2, false, vec![]), t), Snap::Refused(_)));
        assert_eq!(m.snapshot(part(0, true, vec![]), t), Snap::Waiting);
        assert!(!m.expire_partial(t + Duration::from_secs(5), Duration::from_secs(10)));
        assert!(m.expire_partial(t + Duration::from_secs(11), Duration::from_secs(10)));
    }

    #[test]
    fn an_update_changes_only_what_is_shared_and_a_rewrite_changes_nothing() {
        let mut m = Mirror::default();
        m.snapshot(part(0, false, vec![rec("light.a", "on", json!({"brightness": 100}), &["turn_on"])]), Instant::now());
        let same = Update { rev: 4, entities: vec![StateRec { id: "light.a".into(), state: "on".into(), attrs: attrs(json!({"brightness": 100})), changed: Some(1.0) }], removed: vec![] };
        assert!(!m.update(same));
        let stranger = Update { rev: 5, entities: vec![StateRec { id: "light.b".into(), state: "on".into(), attrs: Map::new(), changed: None }], removed: vec![] };
        assert!(!m.update(stranger));
        let dim = Update { rev: 6, entities: vec![StateRec { id: "light.a".into(), state: "on".into(), attrs: attrs(json!({"brightness": 20, "entity_picture": "/x"})), changed: Some(2.0) }], removed: vec![] };
        assert!(m.update(dim));
        assert_eq!(m.get("light.a").unwrap().ent.attrs.get("brightness"), Some(&json!(20)));
        assert!(m.get("light.a").unwrap().ent.attrs.get("entity_picture").is_none());
        assert_eq!(m.rev(), 6);
        assert!(m.update(Update { rev: 7, entities: vec![], removed: vec!["light.a".into()] }));
        assert!(m.is_empty());
    }

    #[test]
    fn the_end_of_a_session_drops_everything_and_the_house_has_one_unit_of_temperature() {
        let mut m = Mirror::default();
        let sensor = |id: &str, unit: &str| rec(id, "20", json!({"device_class": "temperature", "unit_of_measurement": unit}), &[]);
        m.snapshot(part(0, false, vec![sensor("sensor.a", "\u{b0}F"), sensor("sensor.b", "\u{b0}F"), sensor("sensor.c", "\u{b0}C")]), Instant::now());
        assert_eq!(m.temp_unit(), "\u{b0}F");
        assert!(m.clear());
        assert!(m.is_empty() && m.areas().is_empty() && !m.clear());
        assert_eq!(m.temp_unit(), "");
    }

    #[test]
    fn a_locks_unlock_and_open_are_never_among_its_ordinary_services_and_only_unlock_is_kept_apart() {
        let mut m = Mirror::default();
        let mut front = rec("lock.front", "locked", json!({}), &["lock", "unlock", "open"]);
        front.ctl_pin = vec!["unlock".into(), "open".into()];
        let mut light = rec("light.a", "on", json!({}), &["turn_on"]);
        light.ctl_pin = vec!["turn_off".into()];
        m.snapshot(part(0, false, vec![front, light]), Instant::now());
        let lock = m.get("lock.front").unwrap();
        assert_eq!((lock.ctl.clone(), lock.ctl_pin.clone()), (vec!["lock".to_owned()], vec!["unlock".to_owned()]), "unlock is kept apart, open is not kept at all");
        assert!(m.get("light.a").unwrap().ctl_pin.is_empty(), "no other domain has a sensitive service here");
    }
}
