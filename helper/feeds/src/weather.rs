//! Weather: zip code to place (zippopotam.us), place to forecast (Open-Meteo), and the strings Home's weather card draws.
//!
//! A port of `fetchWeather`, `kindFor`, `textFor` and `dayName` in `WeatherWidget.qml`, JavaScript rounding and number
//! printing included. One deliberate difference: the QML falls back to Atlanta's coordinates when the zip lookup fails;
//! here a failed lookup is an error, so the card never shows another place's weather.

use fetch::{Fetch, FetchLimits};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::calendar;
use crate::jsnum::{fmt_num, js_round};
use crate::{EXPIRE_MS, FeedError, REFRESH_MS, SCHEMA};

/// Degree sign, as the QML writes it (`\u00b0`).
const DEGREE: &str = "\u{b0}";
/// Middle dot between the statistics, as the QML writes it (`\u00b7`).
const DOT: &str = "\u{b7}";

/// Temperature unit of the card and of the Open-Meteo request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    Fahrenheit,
    Celsius,
}

impl Units {
    /// The lower-case name used in settings, in the stored document and in the Open-Meteo `temperature_unit` parameter.
    pub fn as_str(self) -> &'static str {
        match self {
            Units::Fahrenheit => "fahrenheit",
            Units::Celsius => "celsius",
        }
    }

    /// Parses `"fahrenheit"` or `"celsius"`, exactly as written in lower case.
    pub fn parse(s: &str) -> Option<Units> {
        match s {
            "fahrenheit" => Some(Units::Fahrenheit),
            "celsius" => Some(Units::Celsius),
            _ => None,
        }
    }
}

/// What the weather card is configured with: the place (a US zip code) and the unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeatherConfig {
    pub zip: String,
    pub units: Units,
}

/// A resolved place.
#[derive(Clone, Debug, PartialEq)]
pub struct Geo {
    pub place: String,
    pub lat: f64,
    pub lon: f64,
}

/// The current reading from Open-Meteo's `current` object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Current {
    /// Local time of the reading, e.g. `2026-10-25T00:15`.
    pub time: String,
    pub temp: f64,
    #[serde(rename = "feelsLike")]
    pub feels_like: f64,
    /// WMO weather code.
    pub code: i64,
    #[serde(rename = "isDay")]
    pub is_day: bool,
    pub humidity: f64,
    #[serde(rename = "windMph")]
    pub wind_mph: f64,
}

/// One day of the forecast.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Day {
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    pub code: i64,
    pub hi: f64,
    pub lo: f64,
}

/// What the Home weather card draws: exactly the strings `WeatherWidget.qml` builds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Display {
    pub place: String,
    pub temp: String,
    pub cond: String,
    pub stats: String,
    #[serde(rename = "nowKind")]
    pub now_kind: String,
    pub days: Vec<DisplayDay>,
}

/// One row of the card's forecast list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayDay {
    pub day: String,
    pub kind: String,
    pub hi: String,
    pub lo: String,
}

/// The stored and served weather document: the on-disk `weather.json`, which QML reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeatherDoc {
    /// [`SCHEMA`].
    pub schema: u32,
    /// `now_ms` of the successful fetch.
    #[serde(rename = "fetchedAt")]
    pub fetched_at: u64,
    /// `fetched_at` + [`REFRESH_MS`].
    #[serde(rename = "staleAt")]
    pub stale_at: u64,
    /// `fetched_at` + [`EXPIRE_MS`].
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
    pub zip: String,
    pub units: Units,
    pub place: String,
    pub lat: f64,
    pub lon: f64,
    pub current: Current,
    pub daily: Vec<Day>,
    pub display: Display,
}

/// Accepts exactly 5 ASCII digits.
pub fn validate_zip(zip: &str) -> Result<(), FeedError> {
    if zip.len() == 5 && zip.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(FeedError::Config("zip must be 5 digits".to_owned()))
    }
}

/// The zippopotam.us lookup for a (validated) zip code.
pub fn geocode_url(zip: &str) -> String {
    format!("https://api.zippopotam.us/us/{zip}")
}

/// Reads `places[0]` of a zippopotam.us reply; latitude and longitude arrive as strings.
pub fn parse_geo(body: &[u8]) -> Result<Geo, FeedError> {
    let v = parse_json(body, "geocode reply")?;
    let place = v
        .get("places")
        .and_then(|p| p.get(0))
        .ok_or_else(|| parse_err("geocode reply has no places"))?;
    let name = string(place, "place name")?;
    if name.is_empty() {
        return Err(parse_err("geocode place name is empty"));
    }
    Ok(Geo {
        place: name,
        lat: coordinate(place, "latitude")?,
        lon: coordinate(place, "longitude")?,
    })
}

/// The Open-Meteo request, byte for byte the QML's; `lat` and `lon` print as JavaScript string concatenation prints them.
pub fn forecast_url(geo: &Geo, units: Units) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}\
         &current=temperature_2m,apparent_temperature,weather_code,is_day,relative_humidity_2m,wind_speed_10m\
         &daily=weather_code,temperature_2m_max,temperature_2m_min&temperature_unit={}\
         &wind_speed_unit=mph&timezone=auto&forecast_days=7",
        fmt_num(geo.lat),
        fmt_num(geo.lon),
        units.as_str()
    )
}

/// Turns an Open-Meteo reply into the stored document, with the card's strings precomputed.
///
/// A daily row whose date, code or temperatures are null or missing is skipped, as the QML would draw nothing for it.
pub fn parse_forecast(
    body: &[u8],
    geo: &Geo,
    cfg: &WeatherConfig,
    now_ms: u64,
) -> Result<WeatherDoc, FeedError> {
    let v = parse_json(body, "forecast")?;
    let cur = v.get("current").ok_or_else(|| parse_err("forecast has no current"))?;
    let current = Current {
        time: string(cur, "time")?,
        temp: number(cur, "temperature_2m")?,
        feels_like: number(cur, "apparent_temperature")?,
        code: integer(cur, "weather_code")?,
        is_day: number(cur, "is_day")? == 1.0,
        humidity: number(cur, "relative_humidity_2m")?,
        wind_mph: number(cur, "wind_speed_10m")?,
    };
    let daily = daily_rows(&v)?;
    let display = build_display(&geo.place, &current, &daily);
    Ok(WeatherDoc {
        schema: SCHEMA,
        fetched_at: now_ms,
        stale_at: now_ms.saturating_add(REFRESH_MS),
        expires_at: now_ms.saturating_add(EXPIRE_MS),
        zip: cfg.zip.clone(),
        units: cfg.units,
        place: geo.place.clone(),
        lat: geo.lat,
        lon: geo.lon,
        current,
        daily,
        display,
    })
}

/// Fetches a fresh weather document.
///
/// With a `previous` document for the same zip the saved place is reused and only the forecast is requested; otherwise
/// the zip is looked up first. A failed lookup is an error: another place's weather is never shown.
pub fn refresh(
    fetch: &dyn Fetch,
    cfg: &WeatherConfig,
    previous: Option<&WeatherDoc>,
    now_ms: u64,
) -> Result<WeatherDoc, FeedError> {
    validate_zip(&cfg.zip)?;
    let limits = FetchLimits::default();
    let geo = match previous {
        Some(p) if p.zip == cfg.zip && !p.place.is_empty() => Geo {
            place: p.place.clone(),
            lat: p.lat,
            lon: p.lon,
        },
        _ => parse_geo(&fetch.get(&geocode_url(&cfg.zip), limits)?)?,
    };
    let body = fetch.get(&forecast_url(&geo, cfg.units), limits)?;
    parse_forecast(&body, &geo, cfg, now_ms)
}

/// The icon name the card draws for a WMO code. Ported with the QML's check order, quirks included: `code <= 2` is tested
/// before anything else, so codes below 0 give the partly-cloudy icons, and `>= 61` is rain all the way to 94 (except snow).
pub fn kind_for(code: i64, is_day: bool) -> &'static str {
    if code == 0 {
        return if is_day { "sun" } else { "moon" };
    }
    if code <= 2 {
        return if is_day { "partly" } else { "partly-night" };
    }
    if code == 3 {
        return "cloud";
    }
    if code == 45 || code == 48 {
        return "fog";
    }
    if code >= 95 {
        return "storm";
    }
    if is_snow(code) {
        return "snow";
    }
    if code >= 61 {
        return "rain";
    }
    if code >= 51 {
        return "drizzle";
    }
    "cloud"
}

/// The condition text the card shows for a WMO code, in the QML's check order.
pub fn text_for(code: i64) -> &'static str {
    match code {
        0 => return "Clear",
        1 => return "Mostly clear",
        2 => return "Partly cloudy",
        3 => return "Overcast",
        45 | 48 => return "Fog",
        _ => {}
    }
    if code >= 95 {
        "Thunderstorm"
    } else if is_snow(code) {
        "Snow"
    } else if code >= 80 {
        "Showers"
    } else if code >= 61 {
        "Rain"
    } else if code >= 51 {
        "Drizzle"
    } else {
        "Cloudy"
    }
}

/// The weekday abbreviation (`Sun`..`Sat`) of a `YYYY-MM-DD` calendar date, or `None` if the text is not a real date.
/// The date is already local to the place, so no time zone arithmetic applies.
pub fn day_name(iso_date: &str) -> Option<&'static str> {
    const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let b = iso_date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u32> {
        let part = &b[r];
        if part.iter().all(u8::is_ascii_digit) {
            part.iter().try_fold(0u32, |n, d| Some(n * 10 + u32::from(d - b'0')))
        } else {
            None
        }
    };
    let year = i64::from(digits(0..4)?);
    let month = digits(5..7)?;
    let day = digits(8..10)?;
    if !(1..=12).contains(&month) || day == 0 || day > calendar::days_in_month(year, month) {
        return None;
    }
    Some(NAMES[calendar::weekday(calendar::days_from_civil(year, month, day))])
}

fn is_snow(code: i64) -> bool {
    (71..=77).contains(&code) || code == 85 || code == 86
}

fn build_display(place: &str, cur: &Current, daily: &[Day]) -> Display {
    let days = daily
        .iter()
        .enumerate()
        .map(|(i, d)| DisplayDay {
            day: if i == 0 {
                "Today".to_owned()
            } else {
                day_name(&d.date).unwrap_or("").to_owned()
            },
            kind: kind_for(d.code, true).to_owned(),
            hi: degrees(d.hi),
            lo: degrees(d.lo),
        })
        .collect();
    Display {
        place: place.to_owned(),
        temp: degrees(cur.temp),
        cond: text_for(cur.code).to_owned(),
        stats: format!(
            "Feels like {} {DOT} {}% humidity {DOT} {} mph",
            degrees(cur.feels_like),
            fmt_num(cur.humidity),
            js_round(cur.wind_mph)
        ),
        now_kind: kind_for(cur.code, cur.is_day).to_owned(),
        days,
    }
}

/// `Math.round(x) + "\u00b0"`.
fn degrees(x: f64) -> String {
    format!("{}{DEGREE}", js_round(x))
}

fn daily_rows(v: &Value) -> Result<Vec<Day>, FeedError> {
    let daily = v.get("daily").ok_or_else(|| parse_err("forecast has no daily"))?;
    let times = array(daily, "time")?;
    let codes = array(daily, "weather_code")?;
    let his = array(daily, "temperature_2m_max")?;
    let los = array(daily, "temperature_2m_min")?;
    Ok(times
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            Some(Day {
                date: t.as_str()?.to_owned(),
                code: as_integer(codes.get(i)?)?,
                hi: his.get(i)?.as_f64()?,
                lo: los.get(i)?.as_f64()?,
            })
        })
        .collect())
}

fn parse_err(msg: &str) -> FeedError {
    FeedError::Parse(msg.to_owned())
}

fn parse_json(body: &[u8], what: &str) -> Result<Value, FeedError> {
    serde_json::from_slice(body).map_err(|_| FeedError::Parse(format!("{what} is not JSON")))
}

fn string(v: &Value, field: &str) -> Result<String, FeedError> {
    v.get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| FeedError::Parse(format!("missing text field {field}")))
}

fn number(v: &Value, field: &str) -> Result<f64, FeedError> {
    v.get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| FeedError::Parse(format!("missing number field {field}")))
}

fn integer(v: &Value, field: &str) -> Result<i64, FeedError> {
    v.get(field)
        .and_then(as_integer)
        .ok_or_else(|| FeedError::Parse(format!("missing integer field {field}")))
}

fn as_integer(v: &Value) -> Option<i64> {
    let n = v.as_f64()?;
    (n.fract() == 0.0 && n.abs() < 1e15).then_some(n as i64)
}

fn array<'a>(v: &'a Value, field: &str) -> Result<&'a Vec<Value>, FeedError> {
    v.get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| FeedError::Parse(format!("missing list field {field}")))
}

/// A zippopotam.us coordinate: a decimal number inside a JSON string.
fn coordinate(v: &Value, field: &str) -> Result<f64, FeedError> {
    v.get(field)
        .and_then(Value::as_str)
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|x| x.is_finite())
        .ok_or_else(|| FeedError::Parse(format!("missing coordinate {field}")))
}
