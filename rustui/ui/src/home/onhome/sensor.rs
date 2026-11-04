//! The sheet of a sensor's history: what the helper's `GET /v1/onhome/history` answers, read into what the sheet draws, and the keys that change
//! the range it shows.
//!
//! A number sensor is a chart, with its extremes and its average; an on-off sensor is bands of on and off with when it last changed; a sensor of
//! words is its recent changes. The helper makes all three from Home Assistant's recorder (hourly statistics beyond a day, down to the points
//! the chart has pixels for); this module only reads the answer and says what to ask next.

use serde_json::Value;

/// The ranges the sheet has, as the helper takes them, and how they are written on their chips.
pub const RANGES: [(&str, &str); 5] = [("1h", "1h"), ("6h", "6h"), ("24h", "24h"), ("7d", "7d"), ("30d", "30d")];
/// The range the sheet opens on.
pub const DEFAULT_RANGE: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub min_at: f64,
    pub max_at: f64,
}

/// What came back, by the kind of sensor.
#[derive(Clone, Debug, PartialEq)]
pub enum Doc {
    Number { from: f64, to: f64, unit: String, class: String, points: Vec<(f64, f64)>, stats: Option<Stats>, now: Option<f64> },
    Binary { from: f64, to: f64, class: String, bands: Vec<(f64, f64, String)>, changes: usize, now: String, last_change: Option<f64> },
    Text { changes: Vec<(f64, String)>, last_change: Option<f64> },
}

/// Where the sheet is in getting what it shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Load {
    Asking,
    Ready,
    /// Home Assistant's own words for why not, as the helper gives them.
    Failed(String),
}

/// What a key on the sheet did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    Nothing,
    Close,
    /// The range changed: ask for this one.
    Ask(&'static str),
}

#[derive(Clone, Debug)]
pub struct SensorSheet {
    pub id: String,
    pub range: usize,
    pub load: Load,
    pub doc: Option<Doc>,
    /// Changes with everything the sheet shows, so that the surface draws what changed and no more.
    pub rev: u64,
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

/// The document of the helper's answer as the sheet reads it, or `None` when it is not one.
pub fn parse(doc: &Value) -> Option<Doc> {
    let text = |k: &str| doc.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    match doc.get("kind").and_then(Value::as_str)? {
        "number" => {
            let points = doc.get("points")?.as_array()?.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect();
            let stats = doc.get("stats").filter(|s| !s.is_null()).and_then(|s| Some(Stats { min: num(s, "min")?, max: num(s, "max")?, avg: num(s, "avg")?, min_at: num(s, "min_at")?, max_at: num(s, "max_at")? }));
            let now = doc.get("stats").and_then(|s| num(s, "now"));
            Some(Doc::Number { from: num(doc, "from")?, to: num(doc, "to")?, unit: text("unit"), class: text("class"), points, stats, now })
        }
        "binary" => {
            let bands = doc.get("bands")?.as_array()?.iter().filter_map(|b| Some((b.get(0)?.as_f64()?, b.get(1)?.as_f64()?, b.get(2)?.as_str()?.to_owned()))).collect();
            Some(Doc::Binary { from: num(doc, "from")?, to: num(doc, "to")?, class: text("class"), bands, changes: doc.get("changes").and_then(Value::as_u64).unwrap_or(0) as usize, now: text("now"), last_change: num(doc, "last_change") })
        }
        "text" => {
            let changes = doc.get("changes")?.as_array()?.iter().filter_map(|c| Some((c.get(0)?.as_f64()?, c.get(1)?.as_str()?.to_owned()))).collect();
            Some(Doc::Text { changes, last_change: num(doc, "last_change") })
        }
        _ => None,
    }
}

impl SensorSheet {
    pub fn new(id: &str) -> Self {
        Self { id: id.to_owned(), range: DEFAULT_RANGE, load: Load::Asking, doc: None, rev: 0 }
    }

    /// The range's id for the helper.
    pub fn range_id(&self) -> &'static str {
        RANGES[self.range.min(RANGES.len() - 1)].0
    }

    /// The path the history is asked at.
    pub fn path(&self) -> String {
        format!("/v1/onhome/history?id={}&range={}", self.id, self.range_id())
    }

    /// An answer of the helper: it is for this sheet's sensor and range, or it is not (a range that was left before it came). Whether it was taken.
    pub fn arrived(&mut self, status: u16, data: &Value) -> bool {
        if status != 200 {
            let words = data.pointer("/error/message").and_then(Value::as_str).filter(|w| !w.is_empty()).unwrap_or("The history could not be read").to_owned();
            self.load = Load::Failed(words);
            self.doc = None;
            self.rev += 1;
            return true;
        }
        let Some(doc) = data.get("history") else { return false };
        if doc.get("id").and_then(Value::as_str) != Some(self.id.as_str()) || doc.get("range").and_then(Value::as_str) != Some(self.range_id()) {
            return false;
        }
        match parse(doc) {
            Some(d) => {
                self.doc = Some(d);
                self.load = Load::Ready;
            }
            None => self.load = Load::Failed("The history could not be read".into()),
        }
        self.rev += 1;
        true
    }

    /// A key: Left and Right walk the ranges (each is asked for), Back closes. OK does nothing: a sheet that only shows has nothing to press.
    pub fn key(&mut self, name: &str) -> Done {
        let step: i64 = match name {
            "back" => return Done::Close,
            "left" => -1,
            "right" => 1,
            _ => return Done::Nothing,
        };
        let to = (self.range as i64 + step).clamp(0, RANGES.len() as i64 - 1) as usize;
        if to == self.range {
            return Done::Nothing;
        }
        self.range = to;
        self.load = Load::Asking;
        self.doc = None;
        self.rev += 1;
        Done::Ask(self.range_id())
    }

    /// A range chosen directly (the pointer's click on its chip).
    pub fn pick(&mut self, range: usize) -> Done {
        if range >= RANGES.len() || range == self.range {
            return Done::Nothing;
        }
        self.range = range;
        self.load = Load::Asking;
        self.doc = None;
        self.rev += 1;
        Done::Ask(self.range_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn number() -> Value {
        json!({"id": "sensor.t", "kind": "number", "range": "24h", "from": 1000.0, "to": 87_400_000.0, "unit": "\u{b0}C", "class": "temperature", "source": "history",
               "points": [[1000.0, 20.0], [2000.0, 22.0]], "stats": {"min": 20.0, "max": 22.0, "avg": 21.0, "min_at": 1000.0, "max_at": 2000.0, "now": 22.0, "now_at": 2000.0}})
    }

    #[test]
    fn the_three_kinds_of_answer_are_read_into_what_the_sheet_draws() {
        let Some(Doc::Number { points, stats, unit, now, .. }) = parse(&number()) else { panic!("a number") };
        assert_eq!((points.len(), unit.as_str(), now), (2, "\u{b0}C", Some(22.0)));
        assert_eq!(stats.map(|s| (s.min, s.max, s.avg)), Some((20.0, 22.0, 21.0)));
        let door = json!({"id": "binary_sensor.d", "kind": "binary", "range": "24h", "from": 0.0, "to": 100.0, "class": "door", "bands": [[0.0, 50.0, "off"], [50.0, 100.0, "on"]], "changes": 1, "now": "on", "last_change": 50.0});
        let Some(Doc::Binary { bands, changes, now, last_change, .. }) = parse(&door) else { panic!("bands") };
        assert_eq!((bands.len(), changes, now.as_str(), last_change), (2, 1, "on", Some(50.0)));
        let words = json!({"id": "sensor.m", "kind": "text", "range": "24h", "changes": [[200.0, "Away"], [100.0, "Home"]], "last_change": 200.0});
        let Some(Doc::Text { changes, .. }) = parse(&words) else { panic!("words") };
        assert_eq!(changes[0], (200.0, "Away".to_owned()));
        // an empty number is a doc with nothing to draw, not a failure
        let mut empty = number();
        empty["points"] = json!([]);
        empty["stats"] = Value::Null;
        assert!(matches!(parse(&empty), Some(Doc::Number { stats: None, .. })));
        assert_eq!(parse(&json!({"kind": "thermal"})), None);
    }

    #[test]
    fn the_sheet_opens_on_a_day_and_the_arrows_walk_the_ranges_each_asked_for_once() {
        let mut s = SensorSheet::new("sensor.t");
        assert_eq!((s.range_id(), s.load.clone()), ("24h", Load::Asking));
        assert_eq!(s.path(), "/v1/onhome/history?id=sensor.t&range=24h");
        assert!(s.arrived(200, &json!({"history": number()})));
        assert_eq!(s.load, Load::Ready);
        assert_eq!(s.key("right"), Done::Ask("7d"));
        assert_eq!((s.load.clone(), s.doc.is_none()), (Load::Asking, true), "the old range is not shown under the new one's name");
        assert_eq!(s.key("right"), Done::Ask("30d"));
        assert_eq!(s.key("right"), Done::Nothing, "the end holds");
        assert_eq!(s.key("left"), Done::Ask("7d"));
        assert_eq!(s.pick(0), Done::Ask("1h"));
        assert_eq!(s.pick(0), Done::Nothing);
        assert_eq!((s.key("ok"), s.key("up"), s.key("down")), (Done::Nothing, Done::Nothing, Done::Nothing));
        assert_eq!(s.key("back"), Done::Close);
    }

    #[test]
    fn an_answer_for_a_range_that_was_left_is_not_taken_and_a_refusal_comes_in_words() {
        let mut s = SensorSheet::new("sensor.t");
        s.key("right");
        // the 24 h answer arrives after the person went to 7 d
        assert!(!s.arrived(200, &json!({"history": number()})), "stale");
        assert_eq!(s.load, Load::Asking);
        assert!(!s.arrived(200, &json!({"nothing": 1})));
        assert!(s.arrived(409, &json!({"error": {"code": "no_link", "message": "Home Assistant is not linked to the TV"}})));
        assert_eq!(s.load, Load::Failed("Home Assistant is not linked to the TV".into()));
        // another sensor's answer is not this one's
        let mut other = number();
        other["id"] = json!("sensor.other");
        let mut s = SensorSheet::new("sensor.t");
        assert!(!s.arrived(200, &json!({"history": other})));
    }
}
