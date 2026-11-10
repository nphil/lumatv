//! Weather from a Home Assistant weather entity: the other source of Home's weather card, the weather view and the screensaver's weather
//! (`base.weather` `source` = `homeassistant`, `entity` = a `weather.*` id).
//!
//! One short WebSocket conversation per refresh, through the saved Home Assistant link (`ha/link.json`, else the voice link's; `ha_store`,
//! `voice_ws`): the entity's state and attributes (`subscribe_entities` for that one entity, never every state of the
//! house), where home is (`get_config`, which the radar centres on) and the forecasts the entity says it supports
//! (`weather.get_forecasts`, daily and hourly). What comes back is put in the snapshot shape of [`lumatv_feeds::weather_ha`], which
//! makes the card's and the view's documents; this module only reads, converts what the mapper does not (pressure, visibility and rain to
//! hPa, km and inches; the forecasts' times to the TV's own clock) and speaks.
//!
//! The list of the instance's weather entities for the Settings picker is `get_states` read once when the person opens it.
//!
//! The token never leaves `voice_ws` and goes only to the address it was saved with; a failure is a [`Why`] in words.

use crate::ha_store::{self, HaLink};
use crate::voice_ws::{Fail, Ha};
use fetch::{FetchError, TlsFetch};
use ha_link::endpoint::AddressPolicy;
use lumatv_feeds::weather::{WeatherConfig, WeatherDoc};
use lumatv_feeds::weather_ha;
use lumatv_feeds::weather_more::{DAYS, HOURS, MoreDoc};
use lumatv_feeds::FeedError;
use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long a whole read may take, dialling included.
const WITHIN: Duration = Duration::from_secs(15);
/// What the weather document's `zip` holds for this source, before the entity: it keeps the saved document of one source from being taken
/// for the other's.
pub const KEY_PREFIX: &str = "ha:";

/// `WeatherEntityFeature`: which forecasts an entity can give.
const FORECAST_DAILY: i64 = 1;
const FORECAST_HOURLY: i64 = 2;

/// Why a read gave nothing, in words for the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Why {
    pub code: &'static str,
    pub words: String,
}

fn why(code: &'static str, words: impl Into<String>) -> Why {
    Why { code, words: words.into() }
}

impl From<Fail> for Why {
    fn from(f: Fail) -> Self {
        match f {
            Fail::AuthInvalid => why("auth", "Home Assistant did not accept the link's token"),
            Fail::Cannot(words) | Fail::Lost(words) => why("unavailable", words),
        }
    }
}

impl From<Why> for FeedError {
    fn from(w: Why) -> Self {
        match w.code {
            "no_link" => FeedError::Config(w.words),
            "no_entity" | "unreadable" => FeedError::Parse(w.words),
            _ => FeedError::Fetch(FetchError::Failed(w.words)),
        }
    }
}

/// One weather entity of the instance, as the picker lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub id: String,
    pub name: String,
}

/// What a read gave: the snapshot the mapper takes and where home is.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub snapshot: Value,
    pub lat: f64,
    pub lon: f64,
}

/// The document key (`WeatherConfig::zip`) of an entity.
pub fn key_of(entity: &str) -> String {
    format!("{KEY_PREFIX}{entity}")
}

/// The entity a document key names, when it is this source's.
pub fn entity_of(key: &str) -> Option<&str> {
    key.strip_prefix(KEY_PREFIX)
}

/// The way into Home Assistant for weather: where the link may be, which addresses it may be dialled at, the TLS helper a secure address
/// needs, and the details document of the last read (the card's refresh makes both documents from one conversation).
pub struct HaWeather {
    links: Vec<PathBuf>,
    policy: AddressPolicy,
    tls: Option<TlsFetch>,
    parked: Mutex<Option<(WeatherConfig, MoreDoc)>>,
}

impl HaWeather {
    pub fn real(ctx: &crate::ctx::Ctx, policy: AddressPolicy) -> Self {
        Self::new(
            vec![ctx.paths.ha_link_file(), ctx.paths.data_dir.join("voice").join("ha.json")],
            policy,
            crate::voice::tls_for(&ctx.config.fetch),
        )
    }

    pub fn new(links: Vec<PathBuf>, policy: AddressPolicy, tls: Option<TlsFetch>) -> Self {
        Self { links, policy, tls, parked: Mutex::new(None) }
    }

    fn link(&self) -> Result<HaLink, Why> {
        for file in &self.links {
            match ha_store::load(file) {
                Ok(Some(link)) => return Ok(link),
                Ok(None) => {}
                Err(_) => return Err(why("no_link", "The Home Assistant link cannot be read")),
            }
        }
        Err(why("no_link", "Weather from Home Assistant needs the Home Assistant link"))
    }

    fn connect(&self, deadline: Instant) -> Result<Ha, Why> {
        let link = self.link()?;
        Ha::connect(&link, self.policy, self.tls.as_ref(), deadline).map_err(Why::from)
    }

    /// The instance's weather entities by friendly name.
    pub fn entities(&self) -> Result<Vec<Entity>, Why> {
        let deadline = Instant::now() + WITHIN;
        let mut ha = self.connect(deadline)?;
        let mut next = 1;
        let states = command(&mut ha, &mut next, json!({"type": "get_states"}), deadline);
        ha.close();
        Ok(entities_of(&states?))
    }

    /// The entity's reading and forecasts, as the snapshot of [`weather_ha`].
    pub fn read(&self, entity: &str, now_ms: u64) -> Result<Reading, Why> {
        let deadline = Instant::now() + WITHIN;
        let mut ha = self.connect(deadline)?;
        let read = converse(&mut ha, entity, now_ms, deadline);
        ha.close();
        read
    }

    /// The card's and the view's documents of the entity, `cfg` carrying its key ([`key_of`]) and the unit.
    pub fn documents(&self, entity: &str, cfg: &WeatherConfig, now_ms: u64) -> Result<(WeatherDoc, MoreDoc), FeedError> {
        let reading = self.read(entity, now_ms)?;
        if reading.snapshot["available"] != json!(true) {
            return Err(FeedError::Parse(format!("{entity} is not available in Home Assistant")));
        }
        let (mut doc, mut more) = weather_ha::documents(&reading.snapshot, cfg, now_ms)
            .ok_or_else(|| FeedError::Parse(format!("{entity} has no temperature")))?;
        doc.lat = reading.lat;
        doc.lon = reading.lon;
        more.lat = reading.lat;
        more.lon = reading.lon;
        doc.current.time = local_stamp(now_ms as i64 / 1000, &local_offset_s);
        Ok((doc, more))
    }

    /// Keeps the details document of a read for the details feed's refresh that follows the card's.
    pub fn park(&self, cfg: &WeatherConfig, more: MoreDoc) {
        *self.parked.lock() = Some((cfg.clone(), more));
    }

    /// The parked details for `cfg`, when they are newer than `than_ms` (what the details feed has).
    pub fn take_parked(&self, cfg: &WeatherConfig, than_ms: u64) -> Option<MoreDoc> {
        let mut parked = self.parked.lock();
        match parked.take() {
            Some((key, more)) if &key == cfg && more.fetched_at > than_ms => Some(more),
            other => {
                *parked = other;
                None
            }
        }
    }
}

/// One command and its `result`; events and other ids are skipped.
fn command(ha: &mut Ha, next: &mut u64, mut message: Value, deadline: Instant) -> Result<Value, Why> {
    let id = *next;
    *next += 1;
    message["id"] = json!(id);
    ha.send_json(&message, deadline).map_err(Why::from)?;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match ha.recv(left).map_err(Why::from)? {
            Some(m) if m["id"].as_u64() == Some(id) && m["type"] == "result" => {
                if m["success"].as_bool() == Some(true) {
                    return Ok(m["result"].clone());
                }
                let said = m["error"]["message"].as_str().unwrap_or("it refused");
                return Err(why("refused", format!("Home Assistant: {}", said.chars().take(120).collect::<String>())));
            }
            Some(_) => {}
            None => return Err(why("unavailable", "Home Assistant did not answer in time")),
        }
    }
}

/// The compressed states of `ids` as the first event of a `subscribe_entities` carries them.
fn watch(ha: &mut Ha, next: &mut u64, ids: &[&str], deadline: Instant) -> Result<Map<String, Value>, Why> {
    let id = *next;
    *next += 1;
    ha.send_json(&json!({"id": id, "type": "subscribe_entities", "entity_ids": ids}), deadline).map_err(Why::from)?;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match ha.recv(left).map_err(Why::from)? {
            Some(m) if m["id"].as_u64() == Some(id) => match m["type"].as_str() {
                Some("result") if m["success"].as_bool() != Some(true) => {
                    return Err(why("refused", "Home Assistant would not give the entity's state"));
                }
                Some("event") => {
                    return Ok(m["event"]["a"].as_object().cloned().unwrap_or_default());
                }
                _ => {}
            },
            Some(_) => {}
            None => return Err(why("unavailable", "Home Assistant did not answer in time")),
        }
    }
}

fn converse(ha: &mut Ha, entity: &str, now_ms: u64, deadline: Instant) -> Result<Reading, Why> {
    let mut next = 1;
    let states = watch(ha, &mut next, &[entity], deadline)?;
    let state = states.get(entity).ok_or_else(|| why("no_entity", format!("Home Assistant has no {entity}")))?;
    let features = state["a"]["supported_features"].as_i64().unwrap_or(0);
    let config = command(ha, &mut next, json!({"type": "get_config"}), deadline)?;
    let (Some(lat), Some(lon)) = (config["latitude"].as_f64(), config["longitude"].as_f64()) else {
        return Err(why("unreadable", "Home Assistant did not say where home is"));
    };
    let mut forecast = |kind: &str| -> Result<Vec<Value>, Why> {
        let result = command(
            ha,
            &mut next,
            json!({"type": "call_service", "domain": "weather", "service": "get_forecasts", "service_data": {"type": kind},
                   "target": {"entity_id": entity}, "return_response": true}),
            deadline,
        )?;
        Ok(result["response"][entity]["forecast"].as_array().cloned().unwrap_or_default())
    };
    let daily = if features & FORECAST_DAILY != 0 { forecast("daily")? } else { Vec::new() };
    let hourly = if features & FORECAST_HOURLY != 0 { forecast("hourly")? } else { Vec::new() };
    let snapshot = snapshot(entity, state, (lat, lon), &daily, &hourly, (now_ms / 1000) as i64, &local_offset_s);
    Ok(Reading { snapshot, lat, lon })
}

/// The instance's `weather.*` entities from `get_states`, by friendly name.
fn entities_of(states: &Value) -> Vec<Entity> {
    let mut out: Vec<Entity> = states
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s["entity_id"].as_str().filter(|id| id.starts_with("weather."))?;
            let name = s["attributes"]["friendly_name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or(id);
            Some(Entity { id: id.to_owned(), name: name.to_owned() })
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    out
}

// -- the snapshot ------------------------------------------------------------------------------------------------------------------

fn number(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

/// hPa in one `unit` of pressure (an entity that names none is in hPa).
fn hpa_per(unit: &str) -> f64 {
    match unit {
        "Pa" => 0.01,
        "kPa" => 10.0,
        "inHg" => 33.8639,
        "mmHg" => 1.33322,
        "psi" => 68.9476,
        _ => 1.0,
    }
}

/// Kilometres in one `unit` of distance (an entity that names none is in km).
fn km_per(unit: &str) -> f64 {
    match unit {
        "mi" => 1.609_344,
        "m" => 0.001,
        "ft" => 0.000_304_8,
        _ => 1.0,
    }
}

/// Inches in one `unit` of rain (an entity that names none is in mm, which Home Assistant's metric system has).
fn inches_per(unit: &str) -> f64 {
    match unit {
        "in" => 1.0,
        "cm" => 1.0 / 2.54,
        _ => 1.0 / 25.4,
    }
}

/// The entity's state and forecasts in the shape [`weather_ha::documents`] takes. Times are put on the clock `offset` (seconds east of
/// UTC at an instant) gives: the TV's own, which is the house's; an hour that has gone by is left out and no more than the view's
/// hours and days are kept.
pub fn snapshot(entity: &str, state: &Value, home: (f64, f64), daily: &[Value], hourly: &[Value], now_s: i64, offset: &dyn Fn(i64) -> i64) -> Value {
    let attrs = &state["a"];
    let condition = state["s"].as_str().unwrap_or("");
    let available = !matches!(condition, "" | "unavailable" | "unknown");
    let mut reading = Map::new();
    for key in ["temperature", "apparent_temperature", "humidity", "wind_speed", "wind_gust_speed", "wind_bearing", "uv_index", "cloud_coverage"] {
        if let Some(v) = attrs.get(key).filter(|v| v.is_number()) {
            reading.insert(key.to_owned(), v.clone());
        }
    }
    for key in ["temperature_unit", "wind_speed_unit"] {
        if let Some(v) = attrs.get(key).filter(|v| v.is_string()) {
            reading.insert(key.to_owned(), v.clone());
        }
    }
    if let Some(p) = number(attrs, "pressure") {
        reading.insert("pressure".into(), json!(p * hpa_per(attrs["pressure_unit"].as_str().unwrap_or(""))));
    }
    if let Some(v) = number(attrs, "visibility") {
        reading.insert("visibility".into(), json!(v * km_per(attrs["visibility_unit"].as_str().unwrap_or(""))));
        reading.insert("visibility_unit".into(), json!("km"));
    }
    let rain = inches_per(attrs["precipitation_unit"].as_str().unwrap_or(""));
    let days: Vec<Value> = daily
        .iter()
        .take(DAYS)
        .filter_map(|d| {
            let mut d = d.as_object()?.clone();
            let at = d.get("datetime")?.as_str()?.to_owned();
            let date = match parse_iso(&at) {
                Some(unix) => local_stamp(unix, offset).chars().take(10).collect::<String>(),
                None => at.chars().take(10).collect(),
            };
            // the entity does not say when the sun rises; the day's own, on the TV's clock
            if let Some(day) = day_number(&date) {
                let (rise, set) = sun_day(day, home, offset);
                for (key, at) in [("sunrise", rise), ("sunset", set)] {
                    if let Some(at) = at {
                        d.insert(key.into(), json!(local_stamp(at + 30, offset)));
                    }
                }
            }
            d.insert("datetime".into(), json!(date));
            scale(&mut d, "precipitation", rain);
            Some(Value::Object(d))
        })
        .collect();
    let hours: Vec<Value> = hourly
        .iter()
        .filter_map(|h| {
            let mut h = h.as_object()?.clone();
            let at = h.get("datetime")?.as_str()?.to_owned();
            let unix = parse_iso(&at);
            if unix.is_some_and(|u| u < now_s - 3_600) {
                return None;
            }
            h.insert("datetime".into(), json!(match unix {
                Some(u) => local_stamp(u, offset),
                None => at.chars().take(16).collect(),
            }));
            if !h.contains_key("is_daytime")
                && let Some(u) = unix
            {
                h.insert("is_daytime".into(), json!(elevation_deg(u, home) > HORIZON_DEG));
            }
            scale(&mut h, "precipitation", rain);
            Some(Value::Object(h))
        })
        .take(HOURS)
        .collect();
    json!({"entity": entity, "available": available, "name": attrs["friendly_name"].as_str().unwrap_or(entity), "condition": condition,
           "reading": reading, "hourly": hours, "daily": days})
}

fn scale(map: &mut Map<String, Value>, key: &str, by: f64) {
    if let Some(v) = map.get(key).and_then(Value::as_f64) {
        map.insert(key.to_owned(), json!(v * by));
    }
}

// -- the sun -----------------------------------------------------------------------------------------------------------------------

/// The sun's centre at sunrise and sunset, degrees: the disc's radius and the air's refraction put the edge on the horizon 0.833 degree
/// before the centre is there.
const HORIZON_DEG: f64 = -0.833;

/// How far apart a day's elevations are sampled to find where the sun crosses the horizon; a change inside a step is bisected to the
/// second. A day's first rise and first set are kept.
const STEP_S: i64 = 600;

/// The sun's elevation above the horizon at `home` (latitude, longitude, degrees) at an instant, from LumaShow's subsolar point
/// (`map::globe::sun_at`, the almanac's low-precision formulae).
fn elevation_deg(unix_s: i64, (lat, lon): (f64, f64)) -> f64 {
    let sun = map::globe::sun_at(unix_s);
    let (phi, dec, hour) = (lat.to_radians(), sun.lat.to_radians(), (lon - sun.lon).to_radians());
    (phi.sin() * dec.sin() + phi.cos() * dec.cos() * hour.cos()).clamp(-1.0, 1.0).asin().to_degrees()
}

/// Unix seconds of local midnight that starts day number `day` (days since 1970-01-01) on the clock `offset` gives.
fn local_midnight(day: i64, offset: &dyn Fn(i64) -> i64) -> i64 {
    let local = day * 86_400;
    let guess = local - offset(local);
    local - offset(guess)
}

/// Unix seconds when the sun rises and sets on local day `day`, `None` for an event the day does not have (the polar day and night).
/// The day is the TV's own, so a daylight-saving day of 23 or 25 hours is walked as it is.
fn sun_day(day: i64, home: (f64, f64), offset: &dyn Fn(i64) -> i64) -> (Option<i64>, Option<i64>) {
    let (start, end) = (local_midnight(day, offset), local_midnight(day + 1, offset));
    let up = |t: i64| elevation_deg(t, home) > HORIZON_DEG;
    let (mut rise, mut set) = (None, None);
    let (mut from, mut was_up) = (start, up(start));
    while from < end && (rise.is_none() || set.is_none()) {
        let to = (from + STEP_S).min(end);
        let now_up = up(to);
        if was_up != now_up {
            let (mut lo, mut hi) = (from, to);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if up(mid) == now_up { hi = mid } else { lo = mid }
            }
            let slot = if now_up { &mut rise } else { &mut set };
            slot.get_or_insert(hi);
        }
        (from, was_up) = (to, now_up);
    }
    (rise, set)
}

/// The day number of a `YYYY-MM-DD` date.
fn day_number(date: &str) -> Option<i64> {
    let n = |from: usize, to: usize| date.get(from..to)?.parse::<i64>().ok();
    Some(days_from_civil(n(0, 4)?, n(5, 7)?, n(8, 10)?))
}

// -- time --------------------------------------------------------------------------------------------------------------------------

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count (the inverse of [`days_from_civil`]).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Unix seconds of an ISO 8601 time with an offset (`2026-10-30T02:00:00+00:00`, `...Z`, `...-04:00`, fractions and a space for the `T`
/// accepted); `None` for a date alone or a time that says no offset.
pub fn parse_iso(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 19 || !(b[10] == b'T' || b[10] == b' ') || b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, s) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        rest = fraction.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let digits: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
            if digits.len() != 4 || rest.len() > 6 {
                return None;
            }
            sign * (digits[..2].parse::<i64>().ok()? * 3_600 + digits[2..].parse::<i64>().ok()? * 60)
        }
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3_600 + mi * 60 + s - offset)
}

/// `YYYY-MM-DDTHH:MM` of an instant on the clock `offset` gives.
fn local_stamp(unix_s: i64, offset: &dyn Fn(i64) -> i64) -> String {
    let local = unix_s + offset(unix_s);
    let (y, m, d) = civil_from_days(local.div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}", secs / 3_600, secs % 3_600 / 60)
}

/// Seconds east of UTC the system's time zone has at an instant (musl reads `TZ` or `/etc/localtime`).
pub fn local_offset_s(unix_s: i64) -> i64 {
    let t: libc::time_t = unix_s;
    // SAFETY: `tm` is ours to fill; localtime_r writes through the pointer it is given and keeps nothing.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return 0;
    }
    tm.tm_gmtoff
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumatv_feeds::weather::Units;

    /// New York in November: UTC-5.
    fn east_5(_: i64) -> i64 {
        -5 * 3_600
    }

    const NEW_YORK: (f64, f64) = (40.7128, -74.0060);

    /// New York's clock: UTC-4 from the second Sunday of March 2026, 2 a.m., until the first Sunday of November, 2 a.m.; else UTC-5.
    fn new_york(unix_s: i64) -> i64 {
        let (on, off) = (parse_iso("2026-03-08T07:00:00+00:00").unwrap(), parse_iso("2026-11-01T06:00:00+00:00").unwrap());
        if (on..off).contains(&unix_s) { -4 * 3_600 } else { -5 * 3_600 }
    }

    /// Minutes after local midnight of a `YYYY-MM-DDTHH:MM` stamp.
    fn minutes(stamp: &str) -> i64 {
        stamp[11..13].parse::<i64>().unwrap() * 60 + stamp[14..16].parse::<i64>().unwrap()
    }

    /// The local stamps of a day's sunrise and sunset (`YYYY-MM-DD`).
    fn sun_stamps(date: &str, home: (f64, f64), offset: &dyn Fn(i64) -> i64) -> (Option<String>, Option<String>) {
        let (rise, set) = sun_day(day_number(date).unwrap(), home, offset);
        (rise.map(|t| local_stamp(t + 30, offset)), set.map(|t| local_stamp(t + 30, offset)))
    }

    #[test]
    fn a_days_sun_is_found_on_the_tvs_clock_through_daylight_saving_and_the_polar_day_and_night() {
        // Reference: NOAA's solar calculator formulae, minutes after local midnight.
        let london = (51.5072, -0.1276);
        let tromso = (69.6492, 18.9553);
        let check = |date: &str, home: (f64, f64), offset: &dyn Fn(i64) -> i64, rise: i64, set: i64| {
            let (r, s) = sun_stamps(date, home, offset);
            let (r, s) = (r.unwrap_or_else(|| panic!("{date}: no sunrise")), s.unwrap_or_else(|| panic!("{date}: no sunset")));
            assert!(r.starts_with(date) && s.starts_with(date), "{date}: {r} {s}");
            assert!((minutes(&r) - rise).abs() <= 1 && (minutes(&s) - set).abs() <= 1, "{date}: {r} {s}");
        };
        check("2026-11-10", NEW_YORK, &new_york, 6 * 60 + 37, 16 * 60 + 42);
        check("2026-11-01", NEW_YORK, &new_york, 6 * 60 + 26, 16 * 60 + 52); // the 25-hour day: the clock goes back at 2 a.m.
        check("2026-03-08", NEW_YORK, &new_york, 7 * 60 + 19, 18 * 60 + 55); // the 23-hour day
        check("2026-07-01", london, &|_| 3_600, 4 * 60 + 48, 21 * 60 + 21);
        let plus_1 = |_: i64| 3_600;
        assert_eq!(sun_stamps("2026-12-21", tromso, &plus_1), (None, None), "the polar night");
        assert_eq!(sun_stamps("2026-06-21", tromso, &plus_1), (None, None), "the midnight sun");
    }

    #[test]
    fn iso_times_are_read_with_their_offsets_and_dates_alone_are_not_times() {
        let t = 1_788_228_000; // 2026-09-01T02:00:00Z
        assert_eq!(parse_iso("2026-09-01T02:00:00+00:00"), Some(t));
        assert_eq!(parse_iso("2026-09-01T02:00:00Z"), Some(t));
        assert_eq!(parse_iso("2026-08-31T22:00:00-04:00"), Some(t));
        assert_eq!(parse_iso("2026-09-01 04:30:00.250+02:30"), Some(t));
        assert_eq!(parse_iso("2026-09-01"), None);
        assert_eq!(parse_iso("2026-09-01T02:00:00"), None, "no offset said");
        assert_eq!(local_stamp(t, &east_5), "2026-08-31T21:00");
        assert_eq!(local_stamp(t, &|_| 0), "2026-09-01T02:00");
    }

    fn state() -> Value {
        json!({"s": "clear-night", "a": {"friendly_name": "Pirate Weather", "temperature": 12.0, "temperature_unit": "\u{b0}C", "humidity": 60, "pressure": 29.92, "pressure_unit": "inHg",
              "visibility": 6.0, "visibility_unit": "mi", "wind_speed": 10.0, "wind_speed_unit": "km/h", "precipitation_unit": "mm", "supported_features": 3, "attribution": "x"}})
    }

    #[test]
    fn a_state_and_its_forecasts_become_the_mappers_snapshot_in_its_units_and_the_tvs_clock() {
        let now = parse_iso("2026-11-03T05:30:00+00:00").unwrap();
        let hourly: Vec<Value> = ["04:00", "05:00", "12:00", "23:00"]
            .iter()
            .map(|t| json!({"datetime": format!("2026-11-03T{t}:00+00:00"), "condition": "cloudy", "temperature": 10.0, "precipitation": 2.54}))
            .collect();
        let daily = [json!({"datetime": "2026-11-03T05:00:00+00:00", "condition": "rainy", "temperature": 14.0, "templow": 8.0, "precipitation": 25.4}), json!({"datetime": "2026-11-04"})];
        let s = snapshot("weather.pirate_weather", &state(), NEW_YORK, &daily, &hourly, now, &east_5);
        assert_eq!((s["available"].clone(), s["name"].clone(), s["condition"].clone()), (json!(true), json!("Pirate Weather"), json!("clear-night")));
        assert!((s["reading"]["pressure"].as_f64().unwrap() - 1013.2).abs() < 0.5, "{}", s["reading"]["pressure"]);
        assert!((s["reading"]["visibility"].as_f64().unwrap() - 9.656).abs() < 0.01);
        assert_eq!(s["reading"]["visibility_unit"], "km");
        assert!(s["reading"].get("attribution").is_none());
        // 04:00Z is more than an hour before now; the rest are on the TV's clock, and the sun's day is told by the turn of the sun
        let h: Vec<(String, Option<bool>)> = s["hourly"].as_array().unwrap().iter().map(|h| (h["datetime"].as_str().unwrap().to_owned(), h["is_daytime"].as_bool())).collect();
        assert_eq!(h, vec![("2026-11-03T00:00".to_owned(), Some(false)), ("2026-11-03T07:00".to_owned(), Some(true)), ("2026-11-03T18:00".to_owned(), Some(false))]);
        assert!((s["hourly"][0]["precipitation"].as_f64().unwrap() - 0.1).abs() < 1e-9, "mm to inches");
        // a day is the TV's date of its start; a date alone stays
        assert_eq!(s["daily"][0]["datetime"], "2026-11-03");
        assert_eq!(s["daily"][1]["datetime"], "2026-11-04");
        assert!((s["daily"][0]["precipitation"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_days_sunrise_and_sunset_are_the_same_day_whatever_the_time_of_day_it_is_asked() {
        // A forecast's days start at local midnight: 2026-11-10 05:00Z is midnight in New York.
        let daily = [json!({"datetime": "2026-11-10T05:00:00+00:00", "temperature": 10.0}), json!({"datetime": "2026-11-11", "temperature": 9.0})];
        let sun_of = |now: &str| {
            let s = snapshot("weather.x", &state(), NEW_YORK, &daily, &[], parse_iso(now).unwrap(), &new_york);
            let at = |day: usize, key: &str| s["daily"][day][key].as_str().map(str::to_owned);
            (at(0, "sunrise"), at(0, "sunset"), at(1, "sunrise"), at(1, "sunset"))
        };
        let morning = sun_of("2026-11-10T09:00:00+00:00");
        // after sunrise and before sunset, when `sun.sun` can only say tomorrow's sunrise; and at night, when it says both of tomorrow
        assert_eq!(sun_of("2026-11-10T19:00:00+00:00"), morning);
        assert_eq!(sun_of("2026-11-11T03:00:00+00:00"), morning);
        let (rise, set, next_rise, next_set) = morning;
        let (rise, set, next_rise, next_set) = (rise.unwrap(), set.unwrap(), next_rise.unwrap(), next_set.unwrap());
        assert!(rise.starts_with("2026-11-10") && set.starts_with("2026-11-10") && next_rise.starts_with("2026-11-11") && next_set.starts_with("2026-11-11"));
        assert!(minutes(&rise) < minutes(&set) && minutes(&next_rise) > minutes(&rise), "{rise} {set} {next_rise}");
    }

    #[test]
    fn an_unavailable_entity_is_not_available_and_the_forecasts_are_capped_to_the_views() {
        let off = json!({"s": "unavailable", "a": {}});
        assert_eq!(snapshot("weather.x", &off, NEW_YORK, &[], &[], 0, &east_5)["available"], json!(false));
        let many: Vec<Value> = (0..60).map(|i| json!({"datetime": format!("2026-11-{:02}", 1 + i % 28), "temperature": 1.0})).collect();
        let hours: Vec<Value> = (0..60).map(|i| json!({"datetime": format!("2026-11-03T{:02}:00:00+00:00", i % 24), "temperature": 1.0})).collect();
        let s = snapshot("weather.x", &state(), NEW_YORK, &many, &hours, 0, &east_5);
        assert_eq!((s["daily"].as_array().unwrap().len(), s["hourly"].as_array().unwrap().len()), (DAYS, HOURS));
    }

    #[test]
    fn the_pickers_entities_are_the_weather_ones_by_friendly_name() {
        let states = json!([
            {"entity_id": "sensor.t", "state": "1", "attributes": {"friendly_name": "Temperature"}},
            {"entity_id": "weather.pirate_weather", "state": "rainy", "attributes": {"friendly_name": "Pirate Weather"}},
            {"entity_id": "weather.home", "state": "sunny", "attributes": {"friendly_name": "forecast Home"}},
            {"entity_id": "weather.bare", "state": "sunny", "attributes": {}}
        ]);
        let names: Vec<(String, String)> = entities_of(&states).into_iter().map(|e| (e.id, e.name)).collect();
        assert_eq!(names, vec![("weather.home".into(), "forecast Home".into()), ("weather.pirate_weather".into(), "Pirate Weather".into()), ("weather.bare".into(), "weather.bare".into())]);
    }

    #[test]
    fn the_parked_details_are_given_once_to_the_refresh_they_are_newer_than() {
        let ha = HaWeather::new(Vec::new(), AddressPolicy::HOME, None);
        let cfg = WeatherConfig { zip: key_of("weather.x"), units: Units::Celsius };
        let (_, more) = weather_ha::documents(&snapshot("weather.x", &state(), NEW_YORK, &[], &[], 0, &east_5), &cfg, 5_000).unwrap();
        ha.park(&cfg, more);
        assert!(ha.take_parked(&cfg, 5_000).is_none(), "not newer");
        let other = WeatherConfig { zip: key_of("weather.y"), units: Units::Celsius };
        assert!(ha.take_parked(&other, 0).is_none(), "another entity's");
        assert_eq!(ha.take_parked(&cfg, 4_999).map(|m| m.fetched_at), Some(5_000));
        assert!(ha.take_parked(&cfg, 0).is_none(), "once");
        assert_eq!(ha.read("weather.x", 0).unwrap_err().code, "no_link");
    }
}
