//! What the full weather view shows beyond the card: the next 36 hours, the details of now (UV, gusts, wind direction, pressure, cloud,
//! visibility, rain so far), each day's sunrise, sunset, UV and chance of rain, and the National Weather Service's active alerts for
//! the place (the United States only; nothing for a place it does not cover).
//!
//! It is a document of its own (`cache/weather-more.json`, topic `weather.more`) so that `weather.json`, which the card and the QML read
//! byte for byte, does not change. The Open-Meteo request and the alerts request are made side by side, and the alerts are no part of
//! whether the document is good: a reading without them is a reading without alerts.

use fetch::{Fetch, FetchLimits};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::weather::{self, Geo, Units, WeatherConfig};
use crate::{EXPIRE_MS, FeedError, REFRESH_MS, SCHEMA};

/// The hours the document holds.
pub const HOURS: usize = 36;
/// The most alerts kept.
pub const MAX_ALERTS: usize = 6;

/// The details of now.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Now {
    #[serde(rename = "uvIndex", default)]
    pub uv_index: Option<f64>,
    #[serde(rename = "gustMph", default)]
    pub gust_mph: Option<f64>,
    /// Degrees the wind comes from.
    #[serde(rename = "windDeg", default)]
    pub wind_deg: Option<f64>,
    #[serde(rename = "pressureHpa", default)]
    pub pressure_hpa: Option<f64>,
    #[serde(rename = "cloudPct", default)]
    pub cloud_pct: Option<f64>,
    #[serde(rename = "visibilityM", default)]
    pub visibility_m: Option<f64>,
    #[serde(rename = "rainIn", default)]
    pub rain_in: Option<f64>,
}

/// One hour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hour {
    /// Local time, `YYYY-MM-DDTHH:MM`.
    pub time: String,
    pub temp: f64,
    /// Chance of rain, percent.
    #[serde(rename = "rainPct", default)]
    pub rain_pct: Option<f64>,
    pub code: i64,
    #[serde(rename = "isDay")]
    pub is_day: bool,
    #[serde(rename = "windMph", default)]
    pub wind_mph: Option<f64>,
}

/// What one day adds to the card's row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DayMore {
    pub date: String,
    /// Local `YYYY-MM-DDTHH:MM`.
    pub sunrise: Option<String>,
    pub sunset: Option<String>,
    #[serde(rename = "uvMax", default)]
    pub uv_max: Option<f64>,
    #[serde(rename = "rainPct", default)]
    pub rain_pct: Option<f64>,
    #[serde(rename = "rainIn", default)]
    pub rain_in: Option<f64>,
}

/// An active alert.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    pub event: String,
    pub severity: String,
    pub headline: String,
    /// When it ends (ISO 8601), if the service says.
    pub ends: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoreDoc {
    pub schema: u32,
    #[serde(rename = "fetchedAt")]
    pub fetched_at: u64,
    #[serde(rename = "staleAt")]
    pub stale_at: u64,
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
    pub zip: String,
    pub units: Units,
    pub place: String,
    pub lat: f64,
    pub lon: f64,
    pub now: Now,
    pub hourly: Vec<Hour>,
    pub daily: Vec<DayMore>,
    pub alerts: Vec<Alert>,
}

impl MoreDoc {
    /// An empty document for the place of `w`: what the details' first refresh starts from, so that it does not look the zip up again.
    pub fn seed(w: &weather::WeatherDoc) -> MoreDoc {
        MoreDoc {
            schema: SCHEMA,
            fetched_at: 0,
            stale_at: 0,
            expires_at: 0,
            zip: w.zip.clone(),
            units: w.units,
            place: w.place.clone(),
            lat: w.lat,
            lon: w.lon,
            now: Now::default(),
            hourly: Vec::new(),
            daily: Vec::new(),
            alerts: Vec::new(),
        }
    }
}

/// The Open-Meteo request for the details.
pub fn url(geo: &Geo, units: Units) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}\
         &current=uv_index,precipitation,cloud_cover,surface_pressure,wind_gusts_10m,wind_direction_10m,visibility\
         &hourly=temperature_2m,precipitation_probability,weather_code,is_day,wind_speed_10m\
         &daily=sunrise,sunset,uv_index_max,precipitation_probability_max,precipitation_sum\
         &temperature_unit={}&wind_speed_unit=mph&precipitation_unit=inch&timezone=auto&forecast_days=7&forecast_hours={HOURS}",
        crate::jsnum::fmt_num(geo.lat),
        crate::jsnum::fmt_num(geo.lon),
        units.as_str()
    )
}

/// The National Weather Service's active alerts at a point.
pub fn alerts_url(geo: &Geo) -> String {
    format!("https://api.weather.gov/alerts/active?point={:.4},{:.4}", geo.lat, geo.lon)
}

fn opt(v: &Value, field: &str) -> Option<f64> {
    v.get(field).and_then(Value::as_f64)
}

fn list<'a>(v: &'a Value, field: &str) -> Result<&'a Vec<Value>, FeedError> {
    v.get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| FeedError::Parse(format!("missing list field {field}")))
}

/// Turns the Open-Meteo reply (and the alerts' body, when there is one) into the document.
pub fn parse(body: &[u8], alerts: Option<&[u8]>, geo: &Geo, cfg: &WeatherConfig, now_ms: u64) -> Result<MoreDoc, FeedError> {
    let v: Value = serde_json::from_slice(body).map_err(|_| FeedError::Parse("forecast is not JSON".into()))?;
    let cur = v.get("current").cloned().unwrap_or(Value::Null);
    let now = Now {
        uv_index: opt(&cur, "uv_index"),
        gust_mph: opt(&cur, "wind_gusts_10m"),
        wind_deg: opt(&cur, "wind_direction_10m"),
        pressure_hpa: opt(&cur, "surface_pressure"),
        cloud_pct: opt(&cur, "cloud_cover"),
        visibility_m: opt(&cur, "visibility"),
        rain_in: opt(&cur, "precipitation"),
    };
    let h = v.get("hourly").ok_or_else(|| FeedError::Parse("forecast has no hourly".into()))?;
    let (times, temps, codes, days) = (list(h, "time")?, list(h, "temperature_2m")?, list(h, "weather_code")?, list(h, "is_day")?);
    let (rain, wind) = (h.get("precipitation_probability").and_then(Value::as_array), h.get("wind_speed_10m").and_then(Value::as_array));
    let hourly: Vec<Hour> = times
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            Some(Hour {
                time: t.as_str()?.to_owned(),
                temp: temps.get(i)?.as_f64()?,
                rain_pct: rain.and_then(|r| r.get(i)).and_then(Value::as_f64),
                code: codes.get(i)?.as_f64()? as i64,
                is_day: days.get(i)?.as_f64()? == 1.0,
                wind_mph: wind.and_then(|w| w.get(i)).and_then(Value::as_f64),
            })
        })
        .take(HOURS)
        .collect();
    let d = v.get("daily").ok_or_else(|| FeedError::Parse("forecast has no daily".into()))?;
    let dates = list(d, "time")?;
    let text = |f: &str, i: usize| d.get(f).and_then(Value::as_array).and_then(|a| a.get(i)).and_then(Value::as_str).map(str::to_owned);
    let num = |f: &str, i: usize| d.get(f).and_then(Value::as_array).and_then(|a| a.get(i)).and_then(Value::as_f64);
    let daily: Vec<DayMore> = dates
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            Some(DayMore {
                date: t.as_str()?.to_owned(),
                sunrise: text("sunrise", i),
                sunset: text("sunset", i),
                uv_max: num("uv_index_max", i),
                rain_pct: num("precipitation_probability_max", i),
                rain_in: num("precipitation_sum", i),
            })
        })
        .collect();
    Ok(MoreDoc {
        schema: SCHEMA,
        fetched_at: now_ms,
        stale_at: now_ms.saturating_add(REFRESH_MS),
        expires_at: now_ms.saturating_add(EXPIRE_MS),
        zip: cfg.zip.clone(),
        units: cfg.units,
        place: geo.place.clone(),
        lat: geo.lat,
        lon: geo.lon,
        now,
        hourly,
        daily,
        alerts: alerts.map(parse_alerts).unwrap_or_default(),
    })
}

/// The alerts of the National Weather Service's GeoJSON, the most severe first.
pub fn parse_alerts(body: &[u8]) -> Vec<Alert> {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    let rank = |s: &str| match s {
        "Extreme" => 0,
        "Severe" => 1,
        "Moderate" => 2,
        "Minor" => 3,
        _ => 4,
    };
    let mut out: Vec<Alert> = v
        .get("features")
        .and_then(Value::as_array)
        .map(|f| {
            f.iter()
                .filter_map(|f| {
                    let p = f.get("properties")?;
                    let s = |k: &str| p.get(k).and_then(Value::as_str);
                    Some(Alert {
                        event: s("event")?.to_owned(),
                        severity: s("severity").unwrap_or("Unknown").to_owned(),
                        headline: s("headline").unwrap_or_default().to_owned(),
                        ends: s("ends").or_else(|| s("expires")).map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|a| rank(&a.severity));
    out.truncate(MAX_ALERTS);
    out
}

/// Fetches a fresh document: the place from `previous` when it is for this zip (else the zip's lookup), then the forecast and the alerts side
/// by side. Alerts that fail leave the document without them.
pub fn refresh(fetch: &dyn Fetch, cfg: &WeatherConfig, previous: Option<&MoreDoc>, now_ms: u64) -> Result<MoreDoc, FeedError> {
    weather::validate_zip(&cfg.zip)?;
    let limits = FetchLimits::default();
    let geo = match previous {
        Some(p) if p.zip == cfg.zip && !p.place.is_empty() => Geo { place: p.place.clone(), lat: p.lat, lon: p.lon },
        _ => weather::parse_geo(&fetch.get(&weather::geocode_url(&cfg.zip), limits)?)?,
    };
    let alert_limits = FetchLimits { max_bytes: 512 * 1024, ..limits };
    let (body, alerts) = std::thread::scope(|s| {
        let a = s.spawn(|| fetch.get(&alerts_url(&geo), alert_limits).ok());
        let body = fetch.get(&url(&geo, cfg.units), FetchLimits { max_bytes: 512 * 1024, ..limits });
        (body, a.join().ok().flatten())
    });
    parse(&body?, alerts.as_deref(), &geo, cfg, now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn geo() -> Geo {
        Geo { place: "Atlanta".into(), lat: 33.723, lon: -84.4702 }
    }

    fn cfg() -> WeatherConfig {
        WeatherConfig { zip: "30315".into(), units: Units::Fahrenheit }
    }

    #[test]
    fn the_request_asks_for_the_details_in_one_call() {
        let u = url(&geo(), Units::Fahrenheit);
        assert!(u.starts_with("https://api.open-meteo.com/v1/forecast?latitude=33.723&longitude=-84.4702&current=uv_index,"));
        assert!(u.contains("&hourly=temperature_2m,precipitation_probability,weather_code,is_day,wind_speed_10m"));
        assert!(u.contains("&daily=sunrise,sunset,uv_index_max,precipitation_probability_max,precipitation_sum"));
        assert!(u.ends_with("&temperature_unit=fahrenheit&wind_speed_unit=mph&precipitation_unit=inch&timezone=auto&forecast_days=7&forecast_hours=36"));
        assert_eq!(alerts_url(&geo()), "https://api.weather.gov/alerts/active?point=33.7230,-84.4702");
    }

    #[test]
    fn a_reply_becomes_hours_days_and_the_details_of_now() {
        let body = json!({
            "current": {"uv_index": 3.5, "wind_gusts_10m": 18.2, "wind_direction_10m": 225.0, "surface_pressure": 1015.2, "cloud_cover": 40.0, "visibility": 24140.0, "precipitation": 0.0},
            "hourly": {"time": ["2026-10-29T22:00", "2026-10-29T23:00"], "temperature_2m": [61.0, 59.5], "precipitation_probability": [10.0, 40.0], "weather_code": [0.0, 61.0], "is_day": [0.0, 0.0], "wind_speed_10m": [6.0, 7.5]},
            "daily": {"time": ["2026-10-29", "2026-10-30"], "sunrise": ["2026-10-29T07:41", "2026-10-30T07:42"], "sunset": ["2026-10-29T18:25", "2026-10-30T18:24"], "uv_index_max": [4.1, 3.0], "precipitation_probability_max": [20.0, 70.0], "precipitation_sum": [0.0, 0.3]}
        });
        let alerts = json!({"features": [
            {"properties": {"event": "Wind Advisory", "severity": "Minor", "headline": "Wind Advisory until 6 PM", "ends": "2026-10-30T22:00:00-04:00"}},
            {"properties": {"event": "Flood Warning", "severity": "Severe", "headline": "Flood Warning", "expires": "2026-10-31T01:00:00-04:00"}}]});
        let doc = parse(body.to_string().as_bytes(), Some(alerts.to_string().as_bytes()), &geo(), &cfg(), 1_000).expect("a document");
        assert_eq!(doc.now.uv_index, Some(3.5));
        assert_eq!(doc.now.wind_deg, Some(225.0));
        assert_eq!(doc.hourly.len(), 2);
        assert_eq!((doc.hourly[1].code, doc.hourly[1].rain_pct, doc.hourly[1].is_day), (61, Some(40.0), false));
        assert_eq!(doc.daily[1].sunset.as_deref(), Some("2026-10-30T18:24"));
        assert_eq!(doc.daily[1].rain_pct, Some(70.0));
        assert_eq!(doc.alerts.iter().map(|a| a.event.as_str()).collect::<Vec<_>>(), ["Flood Warning", "Wind Advisory"], "the most severe first");
        assert_eq!(doc.alerts[0].ends.as_deref(), Some("2026-10-31T01:00:00-04:00"), "`expires` when there is no `ends`");
        assert_eq!(doc.stale_at, 1_000 + REFRESH_MS);
    }

    #[test]
    fn a_reply_without_the_optional_fields_still_makes_a_document_and_without_alerts_has_none() {
        let body = json!({"hourly": {"time": ["2026-10-29T22:00"], "temperature_2m": [61.0], "weather_code": [0.0], "is_day": [1.0]}, "daily": {"time": ["2026-10-29"]}});
        let doc = parse(body.to_string().as_bytes(), None, &geo(), &cfg(), 5).expect("a document");
        assert_eq!(doc.now, Now::default());
        assert_eq!(doc.hourly[0].rain_pct, None);
        assert_eq!(doc.daily[0].sunrise, None);
        assert!(doc.alerts.is_empty());
        assert!(parse(b"nope", None, &geo(), &cfg(), 5).is_err());
        assert_eq!(parse_alerts(b"nope"), Vec::new());
    }
}
