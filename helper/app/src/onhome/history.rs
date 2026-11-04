//! The history of one shared sensor, for the sheet Home opens on it: what Home Assistant's own recorder holds, asked through the saved
//! Home Assistant link (`ha/link.json`, else the voice link's), over one WebSocket for the length of one request.
//!
//! **What may be asked.** Only the history of a sensor or binary sensor that is shared with the TV and shown on Home (the caller checks),
//! for one of five ranges, as read-only commands (`history/history_during_period`, `recorder/statistics_during_period`). The token never
//! leaves this module and goes only to the address it was saved with (`ha_store`, `voice_ws`).
//!
//! **What comes back** is words and numbers for the three kinds of sensor, never states or attributes as Home Assistant has them:
//!
//! - a number: at most [`POINTS`] points (the mean of each slice of the range, so a noisy sensor does not draw noise) and the
//!   minimum, maximum and average over the range, with when the extremes were. Up to a day it is the raw states; beyond a day the hourly
//!   long-term statistics (mean, min, max) where Home Assistant keeps them, else the raw states;
//! - on or off: bands covering the range and when it last changed;
//! - words: the recent changes with their times.
//!
//! Pure over what Home Assistant answered: the parsing, the slicing and the statistics are tested without a socket.

use crate::ha_store::{self, HaLink};
use crate::voice_ws::{Fail, Ha};
use fetch::TlsFetch;
use ha_link::endpoint::AddressPolicy;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The most points a number's curve is given.
pub const POINTS: usize = 240;
/// The most changes a sensor of words lists.
pub const CHANGES: usize = 12;
/// How long a whole request may take, dialling included.
const WITHIN: Duration = Duration::from_secs(12);
/// A range beyond this many seconds is asked of the long-term statistics first.
const STATISTICS_FROM: f64 = 24.0 * 3600.0 + 1.0;

/// The ranges the sheet offers: their ids and lengths in seconds.
pub const RANGES: [(&str, f64); 5] = [("1h", 3_600.0), ("6h", 21_600.0), ("24h", 86_400.0), ("7d", 604_800.0), ("30d", 2_592_000.0)];

pub fn range_seconds(id: &str) -> Option<f64> {
    RANGES.iter().find(|(r, _)| *r == id).map(|(_, s)| *s)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Number,
    Binary,
    Text,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Number => "number",
            Kind::Binary => "binary",
            Kind::Text => "text",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "number" => Kind::Number,
            "binary" => Kind::Binary,
            "text" => Kind::Text,
            _ => return None,
        })
    }
}

/// Why a history is not given, in words for the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Why {
    pub code: &'static str,
    pub words: String,
}

fn why(code: &'static str, words: &str) -> Why {
    Why { code, words: words.to_owned() }
}

/// What a request dials with: the files the Home Assistant link may be in (the first that holds one), the addresses it may go to and the
/// TLS helper a secure address needs.
pub struct Source {
    pub links: Vec<PathBuf>,
    pub policy: AddressPolicy,
    pub tls: Option<TlsFetch>,
}

impl Source {
    pub fn real(ctx: &crate::ctx::Ctx) -> Self {
        Self {
            links: vec![ctx.paths.ha_link_file(), ctx.paths.data_dir.join("voice").join("ha.json")],
            policy: AddressPolicy::HOME,
            tls: crate::voice::tls_for(&ctx.config.fetch),
        }
    }

    fn link(&self) -> Result<HaLink, Why> {
        for file in &self.links {
            match ha_store::load(file) {
                Ok(Some(link)) => return Ok(link),
                Ok(None) => {}
                Err(_) => return Err(why("no_link", "The Home Assistant link cannot be read")),
            }
        }
        Err(why("no_link", "History needs the Home Assistant link"))
    }

    /// The history of `id`, a sensor of `kind`, over `range` ending at `now_ms`: the document the route answers with.
    pub fn ask(&self, id: &str, kind: Kind, range: &str, unit: &str, class: &str, now_ms: u64) -> Result<Value, Why> {
        let seconds = range_seconds(range).ok_or_else(|| why("invalid", "That range is not one the sheet has"))?;
        let link = self.link()?;
        let deadline = Instant::now() + WITHIN;
        let mut ha = Ha::connect(&link, self.policy, self.tls.as_ref(), deadline).map_err(|f| match f {
            Fail::AuthInvalid => why("unavailable", "Home Assistant did not accept the link's token"),
            Fail::Cannot(_) | Fail::Lost(_) => why("unavailable", "Home Assistant cannot be reached"),
        })?;
        let to = now_ms as f64 / 1000.0;
        let from = to - seconds;
        let mut next = 1u64;
        let answer: Result<Value, Why> = (|| -> Result<Value, Why> {
            if kind == Kind::Number && seconds >= STATISTICS_FROM {
                let stats = command(&mut ha, &mut next, json!({"type": "recorder/statistics_during_period", "start_time": iso(from), "end_time": iso(to), "statistic_ids": [id],
                                                                 "period": "hour", "types": ["mean", "min", "max"]}), deadline)?;
                let rows = stat_rows(&stats, id);
                if !rows.is_empty() {
                    return Ok(number_from_stats(&rows, from, to, "statistics"));
                }
            }
            let result = command(&mut ha, &mut next, json!({"type": "history/history_during_period", "start_time": iso(from), "end_time": iso(to), "entity_ids": [id],
                                                           "include_start_time_state": true, "significant_changes_only": kind != Kind::Number || seconds >= STATISTICS_FROM,
                                                           "minimal_response": true, "no_attributes": true}), deadline)?;
            let samples = samples_of(&result, id);
            Ok(match kind {
                Kind::Number => number_from_states(&samples, from, to),
                Kind::Binary => binary_doc(&samples, from, to),
                Kind::Text => text_doc(&samples, from, to),
            })
        })();
        ha.close();
        let mut doc: Value = answer?;
        if let Some(m) = doc.as_object_mut() {
            m.insert("id".into(), json!(id));
            m.insert("kind".into(), json!(kind.id()));
            m.insert("range".into(), json!(range));
            m.insert("from".into(), json!((from * 1000.0).round()));
            m.insert("to".into(), json!((to * 1000.0).round()));
            m.insert("unit".into(), json!(unit));
            m.insert("class".into(), json!(class));
        }
        Ok(doc)
    }
}

/// One command and its answer: the `result` of its own id. Events and other ids are skipped.
fn command(ha: &mut Ha, next: &mut u64, mut message: Value, deadline: Instant) -> Result<Value, Why> {
    let id = *next;
    *next += 1;
    message["id"] = json!(id);
    ha.send_json(&message, deadline).map_err(|_| why("unavailable", "Home Assistant cannot be reached"))?;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(why("unavailable", "Home Assistant did not answer in time"));
        }
        match ha.recv(left) {
            Ok(Some(m)) if m["id"].as_u64() == Some(id) && m["type"] == "result" => {
                if m["success"].as_bool() == Some(true) {
                    return Ok(m["result"].clone());
                }
                return Err(why("no_history", "Home Assistant has no history for that"));
            }
            Ok(_) => {}
            Err(_) => return Err(why("unavailable", "Home Assistant cannot be reached")),
        }
    }
}

// -- time --------------------------------------------------------------------------------------------------------------------------

/// `2026-11-03T04:22:00.000Z` for seconds since 1970 (UTC), as Home Assistant's commands take it.
pub fn iso(seconds: f64) -> String {
    let ms = (seconds * 1000.0).round().max(0.0) as u64;
    let stamp = crate::timeutil::utc_stamp(ms);
    format!("{}.{:03}Z", &stamp[..stamp.len() - 1], ms % 1000)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds since 1970 of `2026-11-03T04:22:00.123456+00:00` or the same with `Z`; `None` for anything else.
pub fn parse_iso(text: &str) -> Option<f64> {
    let (date, rest) = text.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    let (clock, offset) = match rest.find(['Z', '+', '-']) {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "Z"),
    };
    let mut c = clock.split(':');
    let (h, mi): (f64, f64) = (c.next()?.parse().ok()?, c.next()?.parse().ok()?);
    let s: f64 = c.next().unwrap_or("0").parse().ok()?;
    let shift = match offset {
        "Z" => 0.0,
        o => {
            let sign = if o.starts_with('-') { -1.0 } else { 1.0 };
            let (oh, om) = o[1..].split_once(':').unwrap_or((&o[1..], "0"));
            sign * (oh.parse::<f64>().ok()? * 3600.0 + om.parse::<f64>().ok()? * 60.0)
        }
    };
    Some(days_from_civil(y, m, day) as f64 * 86_400.0 + h * 3600.0 + mi * 60.0 + s - shift)
}

/// A time as Home Assistant writes it: seconds (or milliseconds, which are over 1e11) as a number, or an ISO text.
fn time_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().map(|t| if t > 1e11 { t / 1000.0 } else { t }),
        Value::String(s) => parse_iso(s),
        _ => None,
    }
}

// -- what Home Assistant answers -------------------------------------------------------------------------------------------------

/// One state and when it began.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub at: f64,
    pub state: String,
}

/// The states of `id` in a `history/history_during_period` answer, oldest first. A state is `{"s", "lu"}` in the compressed form
/// (`lc` when it differs) or `{"state", "last_updated"}` in the whole one.
pub fn samples_of(result: &Value, id: &str) -> Vec<Sample> {
    let Some(list) = result.get(id).and_then(Value::as_array) else { return Vec::new() };
    let mut out: Vec<Sample> = list
        .iter()
        .filter_map(|s| {
            let state = s.get("s").or_else(|| s.get("state")).and_then(Value::as_str)?.to_owned();
            let at = s.get("lc").or_else(|| s.get("last_changed")).or_else(|| s.get("lu")).or_else(|| s.get("last_updated")).and_then(time_of)?;
            Some(Sample { at, state })
        })
        .collect();
    out.sort_by(|a, b| a.at.total_cmp(&b.at));
    out
}

/// One hour of long-term statistics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stat {
    pub start: f64,
    pub end: f64,
    pub mean: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

pub fn stat_rows(result: &Value, id: &str) -> Vec<Stat> {
    let Some(list) = result.get(id).and_then(Value::as_array) else { return Vec::new() };
    let mut out: Vec<Stat> = list
        .iter()
        .filter_map(|r| {
            let f = |k: &str| r.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
            let start = r.get("start").and_then(time_of)?;
            let end = r.get("end").and_then(time_of).unwrap_or(start + 3600.0);
            let row = Stat { start, end, mean: f("mean"), min: f("min"), max: f("max") };
            (row.mean.is_some() || row.min.is_some() || row.max.is_some()).then_some(row)
        })
        .collect();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

// -- a number ----------------------------------------------------------------------------------------------------------------------

/// A value as a document point: milliseconds and the number, to three decimals.
fn point(t: f64, v: f64) -> Value {
    json!([(t * 1000.0).round(), (v * 1000.0).round() / 1000.0])
}

/// The mean of the step function `steps` (a value from its time until the next) over each of `n` equal slices of `from..to`.
fn slice_means(steps: &[(f64, f64)], from: f64, to: f64, n: usize) -> Vec<(f64, f64)> {
    let width = (to - from) / n as f64;
    let mut out = Vec::with_capacity(n);
    let mut i = 0usize;
    for k in 0..n {
        let (a, b) = (from + k as f64 * width, from + (k + 1) as f64 * width);
        // the step in force at `a`
        while i + 1 < steps.len() && steps[i + 1].0 <= a {
            i += 1;
        }
        let (mut at, mut total, mut j) = (a, 0.0, i);
        loop {
            let value = steps[j].1;
            let until = steps.get(j + 1).map_or(b, |s| s.0.min(b));
            if until > at {
                total += value * (until - at);
                at = until;
            }
            if at >= b || j + 1 >= steps.len() || steps[j + 1].0 >= b {
                break;
            }
            j += 1;
        }
        if at < b {
            total += steps[j].1 * (b - at);
        }
        out.push((a + width / 2.0, total / width));
    }
    out
}

/// A number sensor from its raw states: the numbers among them (a state that is not one, "unavailable", is a gap the line crosses).
pub fn number_from_states(samples: &[Sample], from: f64, to: f64) -> Value {
    let numbers: Vec<(f64, f64)> = samples.iter().filter_map(|s| s.state.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| (s.at.max(from), v))).collect();
    if numbers.is_empty() || to <= from {
        return json!({"source": "history", "points": [], "stats": null});
    }
    // the first value holds from the start of the range
    let mut steps = numbers.clone();
    steps[0].0 = from;
    // Time-weighted over the range; the extremes are the raw ones.
    let mut area = 0.0;
    for (k, s) in steps.iter().enumerate() {
        let until = steps.get(k + 1).map_or(to, |n| n.0);
        area += s.1 * (until - s.0).max(0.0);
    }
    let avg = area / (to - from);
    let (mut lo, mut hi) = (numbers[0], numbers[0]);
    for s in &numbers {
        if s.1 < lo.1 {
            lo = *s;
        }
        if s.1 > hi.1 {
            hi = *s;
        }
    }
    let last = *numbers.last().expect("not empty");
    let n = POINTS.min(((to - from) / 30.0).max(2.0) as usize);
    let points: Vec<Value> = slice_means(&steps, from, to, n).into_iter().map(|(t, v)| point(t, v)).collect();
    json!({"source": "history", "points": points, "stats": {"min": lo.1, "max": hi.1, "avg": (avg * 1000.0).round() / 1000.0, "min_at": (lo.0 * 1000.0).round(), "max_at": (hi.0 * 1000.0).round(), "now": last.1, "now_at": (last.0 * 1000.0).round()}})
}

/// A number sensor from its long-term statistics: the mean of each hour as a point (slices of hours when there are more than [`POINTS`]),
/// the extremes the hours' own.
pub fn number_from_stats(rows: &[Stat], from: f64, to: f64, source: &str) -> Value {
    let mid = |r: &Stat| (r.start + r.end) / 2.0;
    let value = |r: &Stat| r.mean.or_else(|| r.min.zip(r.max).map(|(a, b)| (a + b) / 2.0)).or(r.min).or(r.max);
    let pts: Vec<(f64, f64)> = rows.iter().filter(|r| r.end >= from && r.start <= to).filter_map(|r| value(r).map(|v| (mid(r), v))).collect();
    if pts.is_empty() {
        return json!({"source": source, "points": [], "stats": null});
    }
    let (mut lo, mut hi) = ((0.0, f64::INFINITY), (0.0, f64::NEG_INFINITY));
    for r in rows.iter().filter(|r| r.end >= from && r.start <= to) {
        if let Some(m) = r.min.or(r.mean)
            && m < lo.1
        {
            lo = (mid(r), m);
        }
        if let Some(m) = r.max.or(r.mean)
            && m > hi.1
        {
            hi = (mid(r), m);
        }
    }
    let avg = pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64;
    let last = *pts.last().expect("not empty");
    let shown: Vec<(f64, f64)> = if pts.len() <= POINTS {
        pts.clone()
    } else {
        let per = pts.len().div_ceil(POINTS);
        pts.chunks(per).map(|c| (c.iter().map(|p| p.0).sum::<f64>() / c.len() as f64, c.iter().map(|p| p.1).sum::<f64>() / c.len() as f64)).collect()
    };
    let points: Vec<Value> = shown.into_iter().map(|(t, v)| point(t, v)).collect();
    json!({"source": source, "points": points, "stats": {"min": lo.1, "max": hi.1, "avg": (avg * 1000.0).round() / 1000.0, "min_at": (lo.0 * 1000.0).round(), "max_at": (hi.0 * 1000.0).round(), "now": last.1, "now_at": (last.0 * 1000.0).round()}})
}

// -- on or off, and words ----------------------------------------------------------------------------------------------------------

fn band_of(state: &str) -> &'static str {
    match state {
        "on" => "on",
        "off" => "off",
        _ => "unknown",
    }
}

/// Bands of on, off and unknown covering `from..to`, equal neighbours joined, and when the sensor last changed.
pub fn binary_doc(samples: &[Sample], from: f64, to: f64) -> Value {
    let mut bands: Vec<(f64, f64, &'static str)> = Vec::new();
    for (k, s) in samples.iter().enumerate() {
        let (a, b) = (s.at.max(from), samples.get(k + 1).map_or(to, |n| n.at).min(to));
        if b <= a {
            continue;
        }
        let state = band_of(&s.state);
        match bands.last_mut() {
            Some(last) if last.2 == state => last.1 = b,
            _ => bands.push((a, b, state)),
        }
    }
    // a range that starts before the first state is known says so
    if let Some(first) = bands.first_mut() {
        first.0 = from;
    }
    let mut changes = 0usize;
    let mut last_change = None;
    for pair in samples.windows(2) {
        if band_of(&pair[0].state) != band_of(&pair[1].state) {
            changes += 1;
            last_change = Some(pair[1].at);
        }
    }
    let now = samples.last().map_or("unknown", |s| band_of(&s.state));
    let rows: Vec<Value> = bands.iter().map(|(a, b, s)| json!([(a * 1000.0).round(), (b * 1000.0).round(), s])).collect();
    json!({"source": "history", "bands": rows, "changes": changes, "now": now, "last_change": last_change.map(|t| (t * 1000.0).round())})
}

/// The recent changes of a sensor of words, newest first.
pub fn text_doc(samples: &[Sample], from: f64, to: f64) -> Value {
    let mut changes: Vec<(f64, String)> = Vec::new();
    for s in samples {
        if changes.last().is_none_or(|(_, last)| *last != s.state) {
            changes.push((s.at.clamp(from, to), s.state.clone()));
        }
    }
    let last_change = (changes.len() > 1).then(|| changes.last().map(|c| c.0)).flatten();
    let list: Vec<Value> = changes.iter().rev().take(CHANGES).map(|(t, s)| json!([(t * 1000.0).round(), short(s)])).collect();
    json!({"source": "history", "changes": list, "last_change": last_change.map(|t| (t * 1000.0).round())})
}

/// A state as a short sentence: words not underscores, at most 40 characters.
fn short(state: &str) -> String {
    let text = state.replace('_', " ");
    let mut c = text.chars();
    let word: String = match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    };
    if word.chars().count() <= 40 { word } else { word.chars().take(39).collect::<String>() + "\u{2026}" }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: f64 = 1_800_000_000.0;

    fn s(at: f64, state: &str) -> Sample {
        Sample { at, state: state.into() }
    }

    #[test]
    fn times_are_read_in_every_form_home_assistant_writes_them() {
        assert_eq!(parse_iso("1970-01-02T00:00:00+00:00"), Some(86_400.0));
        assert_eq!(parse_iso("2026-11-03T04:22:00.5Z"), Some(parse_iso("2026-11-03T04:22:00Z").unwrap() + 0.5));
        assert_eq!(parse_iso("2026-11-03T04:22:00-05:00"), Some(parse_iso("2026-11-03T09:22:00Z").unwrap()));
        assert_eq!(parse_iso("yesterday"), None);
        assert_eq!(time_of(&json!(1_700_000_000_500.0)), Some(1_700_000_000.5), "milliseconds");
        assert_eq!(time_of(&json!(1_700_000_000.5)), Some(1_700_000_000.5), "seconds");
        assert_eq!(iso(86_400.5), "1970-01-02T00:00:00.500Z");
        assert_eq!(parse_iso(&iso(T0 + 0.25)), Some(T0 + 0.25));
    }

    #[test]
    fn a_compressed_answer_and_a_whole_one_give_the_same_states_oldest_first() {
        let compressed = json!({"sensor.t": [{"s": "21.5", "lu": T0 + 60.0}, {"s": "21.0", "lu": T0}, {"s": "21.7", "lu": T0 + 90.0, "lc": T0 + 80.0}]});
        let whole = json!({"sensor.t": [{"state": "21.0", "last_changed": iso(T0)}, {"state": "21.5", "last_changed": iso(T0 + 60.0)}]});
        let a = samples_of(&compressed, "sensor.t");
        assert_eq!(a.iter().map(|x| x.state.as_str()).collect::<Vec<_>>(), ["21.0", "21.5", "21.7"]);
        assert_eq!(a[2].at, T0 + 80.0, "last changed wins over last updated");
        let b = samples_of(&whole, "sensor.t");
        assert_eq!((b[0].at, b[1].at), (T0, T0 + 60.0));
        assert!(samples_of(&json!({}), "sensor.t").is_empty());
    }

    #[test]
    fn a_number_is_averaged_over_time_and_its_extremes_are_the_raw_ones() {
        // 10 for the first half of an hour, 20 for the second quarter, 40 for the last quarter
        let samples = [s(T0, "10"), s(T0 + 1800.0, "20"), s(T0 + 2700.0, "40")];
        let doc = number_from_states(&samples, T0, T0 + 3600.0);
        let stats = &doc["stats"];
        assert_eq!((stats["min"].as_f64(), stats["max"].as_f64(), stats["avg"].as_f64()), (Some(10.0), Some(40.0), Some(20.0)));
        assert_eq!((stats["min_at"].as_f64(), stats["max_at"].as_f64(), stats["now"].as_f64()), (Some(T0 * 1000.0), Some((T0 + 2700.0) * 1000.0), Some(40.0)));
        let points = doc["points"].as_array().unwrap();
        assert_eq!(points.len(), 120, "one slice a half minute in an hour");
        assert_eq!(points[0][1], json!(10.0));
        assert_eq!(points[119][1], json!(40.0));
        // a slice that holds two steps is their time-weighted mean
        let means = slice_means(&[(0.0, 10.0), (30.0, 20.0)], 0.0, 60.0, 1);
        assert_eq!(means, vec![(30.0, 15.0)]);
        let uneven = slice_means(&[(0.0, 10.0), (45.0, 20.0)], 0.0, 60.0, 1);
        assert!((uneven[0].1 - 12.5).abs() < 1e-9);
    }

    #[test]
    fn a_state_that_is_not_a_number_is_a_gap_and_nothing_at_all_is_an_empty_chart() {
        let samples = [s(T0, "5"), s(T0 + 600.0, "unavailable"), s(T0 + 1200.0, "7")];
        let doc = number_from_states(&samples, T0, T0 + 1800.0);
        assert_eq!(doc["stats"]["min"], json!(5.0));
        assert_eq!(doc["stats"]["max"], json!(7.0));
        let none = number_from_states(&[s(T0, "unknown")], T0, T0 + 60.0);
        assert_eq!((none["points"].clone(), none["stats"].clone()), (json!([]), Value::Null));
        assert_eq!(number_from_states(&[], T0, T0 + 60.0)["points"], json!([]));
    }

    #[test]
    fn a_state_from_before_the_range_holds_from_its_start_and_a_long_range_keeps_240_points_at_most() {
        let doc = number_from_states(&[s(T0 - 5000.0, "3"), s(T0 + 1000.0, "9")], T0, T0 + 86_400.0);
        assert_eq!(doc["points"].as_array().unwrap().len(), POINTS);
        assert_eq!(doc["points"][0][1], json!(3.0));
        assert_eq!(doc["stats"]["min_at"], json!(T0 * 1000.0), "the first value holds from the start of the range");
    }

    #[test]
    fn hourly_statistics_make_points_and_extremes_and_thin_to_the_budget() {
        let rows: Vec<Stat> = (0..720).map(|h| Stat { start: T0 + h as f64 * 3600.0, end: T0 + (h + 1) as f64 * 3600.0, mean: Some(10.0 + (h % 24) as f64), min: Some(5.0 + (h % 24) as f64), max: Some(15.0 + (h % 24) as f64) }).collect();
        let doc = number_from_stats(&rows, T0, T0 + 720.0 * 3600.0, "statistics");
        assert_eq!(doc["points"].as_array().unwrap().len(), POINTS);
        assert_eq!((doc["stats"]["min"].clone(), doc["stats"]["max"].clone()), (json!(5.0), json!(38.0)));
        assert_eq!(doc["source"], "statistics");
        let day: Vec<Stat> = rows[..24].to_vec();
        let d = number_from_stats(&day, T0, T0 + 24.0 * 3600.0, "statistics");
        assert_eq!(d["points"].as_array().unwrap().len(), 24, "an hour a point while that is under the budget");
        assert_eq!(d["stats"]["avg"], json!(21.5));
        let parsed = stat_rows(&json!({"sensor.t": [{"start": (T0 * 1000.0), "end": (T0 + 3600.0) * 1000.0, "mean": 20.5, "min": 20.0, "max": null}, {"start": T0 + 7200.0, "mean": null, "min": null, "max": null}]}), "sensor.t");
        assert_eq!(parsed, vec![Stat { start: T0, end: T0 + 3600.0, mean: Some(20.5), min: Some(20.0), max: None }], "a row with nothing in it is dropped");
    }

    #[test]
    fn an_on_off_sensor_is_bands_that_cover_the_range_and_a_time_it_last_changed() {
        let samples = [s(T0 - 100.0, "off"), s(T0 + 600.0, "on"), s(T0 + 900.0, "on"), s(T0 + 1200.0, "off"), s(T0 + 2400.0, "unavailable")];
        let doc = binary_doc(&samples, T0, T0 + 3000.0);
        let bands: Vec<(f64, f64, &str)> = doc["bands"].as_array().unwrap().iter().map(|b| (b[0].as_f64().unwrap() / 1000.0, b[1].as_f64().unwrap() / 1000.0, b[2].as_str().unwrap())).collect();
        assert_eq!(bands, vec![(T0, T0 + 600.0, "off"), (T0 + 600.0, T0 + 1200.0, "on"), (T0 + 1200.0, T0 + 2400.0, "off"), (T0 + 2400.0, T0 + 3000.0, "unknown")]);
        assert_eq!((doc["changes"].clone(), doc["now"].clone()), (json!(3), json!("unknown")));
        assert_eq!(doc["last_change"], json!((T0 + 2400.0) * 1000.0));
        let quiet = binary_doc(&[s(T0 - 50.0, "on")], T0, T0 + 100.0);
        assert_eq!((quiet["changes"].clone(), quiet["last_change"].clone(), quiet["bands"].clone()), (json!(0), Value::Null, json!([[T0 * 1000.0, (T0 + 100.0) * 1000.0, "on"]])));
    }

    #[test]
    fn a_sensor_of_words_lists_its_changes_newest_first_in_short_words() {
        let samples = [s(T0, "away_mode"), s(T0 + 10.0, "away_mode"), s(T0 + 20.0, "home"), s(T0 + 30.0, "a_really_long_name_that_goes_on_and_on_and_on_and_on")];
        let doc = text_doc(&samples, T0, T0 + 100.0);
        let rows = doc["changes"].as_array().unwrap();
        assert_eq!(rows.len(), 3, "a state that did not change is not a change");
        assert_eq!(rows[1], json!([(T0 + 20.0) * 1000.0, "Home"]));
        assert!(rows[0][1].as_str().unwrap().chars().count() <= 40);
        assert_eq!(rows[2][1], json!("Away mode"));
        assert_eq!(doc["last_change"], json!((T0 + 30.0) * 1000.0));
        let many: Vec<Sample> = (0..40).map(|i| s(T0 + i as f64, &format!("s{i}"))).collect();
        assert_eq!(text_doc(&many, T0, T0 + 100.0)["changes"].as_array().unwrap().len(), CHANGES);
    }

    #[test]
    fn the_ranges_are_the_five_the_sheet_offers() {
        assert_eq!(RANGES.iter().map(|r| r.0).collect::<Vec<_>>(), ["1h", "6h", "24h", "7d", "30d"]);
        assert_eq!((range_seconds("24h"), range_seconds("90d")), (Some(86_400.0), None));
    }
}
