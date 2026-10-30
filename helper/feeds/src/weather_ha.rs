//! The weather card's and the weather view's documents from a Home Assistant weather entity: the `weather` module of the LumaShow integration
//! (`snapshot`: the entity's reading and its hourly and daily forecasts, in the units Home Assistant has the entity in).
//!
//! Pure: a snapshot in, [`WeatherDoc`] and [`MoreDoc`] out, so that the card and the view are drawn from them exactly as they are from Open-Meteo's. The
//! place is the entity's name (a station has no zip code); the document carries the zip and units it is asked for, so that the saved-first cache
//! rules hold. Wind is converted to mph and the temperature to the unit the card is set to (the entity's `temperature_unit` is read); a number the entity
//! does not give is left out of the view rather than guessed.

use serde_json::Value;

use crate::weather::{Current, Day, Display, DisplayDay, Units, WeatherConfig, WeatherDoc, day_name, kind_for, text_for};
use crate::weather_more::{DayMore, Hour, MoreDoc, Now};
use crate::{EXPIRE_MS, REFRESH_MS, SCHEMA};

/// Home Assistant's condition words as the WMO codes the card's code reads (`kind_for`, `text_for`).
pub fn wmo_of(condition: &str) -> i64 {
    match condition {
        "sunny" | "clear-night" => 0,
        "partlycloudy" => 2,
        "cloudy" => 3,
        "fog" => 45,
        "hail" => 77,
        "lightning" | "lightning-rainy" => 95,
        "pouring" => 65,
        "rainy" => 63,
        "snowy" => 73,
        "snowy-rainy" => 67,
        "windy" | "windy-variant" => 2,
        "exceptional" => 3,
        _ => 3,
    }
}

fn number(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn to_fahrenheit(x: f64, unit: &str) -> f64 {
    if unit.contains('C') { x * 9.0 / 5.0 + 32.0 } else { x }
}

fn to_celsius(x: f64, unit: &str) -> f64 {
    if unit.contains('F') { (x - 32.0) * 5.0 / 9.0 } else { x }
}

fn to_mph(x: f64, unit: &str) -> f64 {
    match unit {
        "km/h" => x * 0.621_371,
        "m/s" => x * 2.236_94,
        "kn" => x * 1.150_78,
        _ => x,
    }
}

/// A snapshot as the documents, or `None` while the entity is not available.
pub fn documents(snap: &Value, cfg: &WeatherConfig, now_ms: u64) -> Option<(WeatherDoc, MoreDoc)> {
    if snap.get("available").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let reading = snap.get("reading")?;
    let unit = reading.get("temperature_unit").and_then(Value::as_str).unwrap_or("\u{b0}C").to_owned();
    let wind_unit = reading.get("wind_speed_unit").and_then(Value::as_str).unwrap_or("km/h").to_owned();
    let want = |x: f64| if cfg.units == Units::Celsius { to_celsius(x, &unit) } else { to_fahrenheit(x, &unit) };
    let place = snap.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
    let condition = snap.get("condition").and_then(Value::as_str).unwrap_or("");
    let code = wmo_of(condition);
    let day_now = condition != "clear-night" && snap.get("hourly").and_then(Value::as_array).and_then(|h| h.first()).and_then(|e| e.get("is_daytime")).and_then(Value::as_bool).unwrap_or(true);
    let temp = number(reading, "temperature")?;
    let current = Current {
        time: String::new(),
        temp: want(temp),
        feels_like: number(reading, "apparent_temperature").map_or_else(|| want(temp), want),
        code,
        is_day: day_now,
        humidity: number(reading, "humidity").unwrap_or(0.0),
        wind_mph: number(reading, "wind_speed").map_or(0.0, |w| to_mph(w, &wind_unit)),
    };
    let daily_in: Vec<&Value> = snap.get("daily").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let daily: Vec<Day> = daily_in
        .iter()
        .filter_map(|d| {
            Some(Day {
                date: d.get("datetime")?.as_str()?.get(..10)?.to_owned(),
                code: wmo_of(d.get("condition").and_then(Value::as_str).unwrap_or("")),
                hi: want(number(d, "temperature")?),
                lo: want(number(d, "templow").or_else(|| number(d, "temperature"))?),
            })
        })
        .collect();
    let degrees = |x: f64| format!("{}\u{b0}", crate::jsnum::js_round(x));
    let display = Display {
        place: place.clone(),
        temp: degrees(current.temp),
        cond: text_for(code).to_owned(),
        stats: format!("Feels like {} \u{b7} {}% humidity \u{b7} {} mph", degrees(current.feels_like), crate::jsnum::fmt_num(current.humidity), crate::jsnum::js_round(current.wind_mph)),
        now_kind: kind_for(code, day_now).to_owned(),
        days: daily
            .iter()
            .enumerate()
            .map(|(i, d)| DisplayDay {
                day: if i == 0 { "Today".to_owned() } else { day_name(&d.date).unwrap_or("").to_owned() },
                kind: kind_for(d.code, true).to_owned(),
                hi: degrees(d.hi),
                lo: degrees(d.lo),
            })
            .collect(),
    };
    let doc = WeatherDoc {
        schema: SCHEMA,
        fetched_at: now_ms,
        stale_at: now_ms.saturating_add(REFRESH_MS),
        expires_at: now_ms.saturating_add(EXPIRE_MS),
        zip: cfg.zip.clone(),
        units: cfg.units,
        place: place.clone(),
        lat: 0.0,
        lon: 0.0,
        current,
        daily,
        display,
    };
    let hourly: Vec<Hour> = snap
        .get("hourly")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|h| {
                    Some(Hour {
                        time: h.get("datetime")?.as_str()?.chars().take(16).collect(),
                        temp: want(number(h, "temperature")?),
                        rain_pct: number(h, "precipitation_probability"),
                        code: wmo_of(h.get("condition").and_then(Value::as_str).unwrap_or("")),
                        is_day: h.get("is_daytime").and_then(Value::as_bool).unwrap_or(true),
                        wind_mph: number(h, "wind_speed").map(|w| to_mph(w, &wind_unit)),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let more_daily: Vec<DayMore> = daily_in
        .iter()
        .filter_map(|d| {
            Some(DayMore {
                date: d.get("datetime")?.as_str()?.get(..10)?.to_owned(),
                sunrise: None,
                sunset: None,
                uv_max: number(d, "uv_index"),
                rain_pct: number(d, "precipitation_probability"),
                rain_in: number(d, "precipitation"),
            })
        })
        .collect();
    let more = MoreDoc {
        schema: SCHEMA,
        fetched_at: now_ms,
        stale_at: now_ms.saturating_add(REFRESH_MS),
        expires_at: now_ms.saturating_add(EXPIRE_MS),
        zip: cfg.zip.clone(),
        units: cfg.units,
        place,
        lat: 0.0,
        lon: 0.0,
        now: Now {
            uv_index: number(reading, "uv_index"),
            gust_mph: number(reading, "wind_gust_speed").map(|w| to_mph(w, &wind_unit)),
            wind_deg: number(reading, "wind_bearing"),
            pressure_hpa: number(reading, "pressure"),
            cloud_pct: number(reading, "cloud_coverage"),
            visibility_m: number(reading, "visibility").map(|v| if reading.get("visibility_unit").and_then(Value::as_str) == Some("mi") { v * 1609.34 } else { v * 1000.0 }),
            rain_in: None,
        },
        hourly,
        daily: more_daily,
        alerts: Vec::new(),
    };
    Some((doc, more))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(units: Units) -> WeatherConfig {
        WeatherConfig { zip: "30311".into(), units }
    }

    fn snapshot() -> Value {
        json!({"entity": "weather.pirate_weather", "available": true, "name": "Pirate Weather", "condition": "rainy",
               "reading": {"temperature": 16.0, "apparent_temperature": 14.0, "humidity": 80.0, "wind_speed": 18.0, "wind_gust_speed": 36.0, "wind_bearing": 225.0, "pressure": 1012.0, "uv_index": 3.0, "visibility": 10.0, "temperature_unit": "\u{b0}C", "wind_speed_unit": "km/h"},
               "hourly": [{"datetime": "2026-10-30T02:00:00+00:00", "condition": "rainy", "temperature": 15.0, "precipitation_probability": 70.0, "is_daytime": false, "wind_speed": 20.0}],
               "daily": [{"datetime": "2026-10-30", "condition": "rainy", "temperature": 18.0, "templow": 11.0, "precipitation_probability": 80.0, "uv_index": 4.0},
                         {"datetime": "2026-10-31", "condition": "sunny", "temperature": 20.0, "templow": 9.0}]})
    }

    #[test]
    fn a_snapshot_becomes_the_cards_document_in_the_units_it_is_set_to() {
        let (doc, more) = documents(&snapshot(), &cfg(Units::Fahrenheit), 1_000).expect("documents");
        assert_eq!(doc.display.place, "Pirate Weather");
        assert_eq!(doc.display.temp, "61\u{b0}");
        assert_eq!(doc.display.cond, "Rain");
        assert_eq!(doc.display.now_kind, "rain");
        assert_eq!(doc.display.stats, "Feels like 57\u{b0} \u{b7} 80% humidity \u{b7} 11 mph");
        assert_eq!(doc.display.days.len(), 2);
        assert_eq!((doc.display.days[0].day.as_str(), doc.display.days[0].hi.as_str(), doc.display.days[0].lo.as_str()), ("Today", "64\u{b0}", "52\u{b0}"));
        assert_eq!(doc.display.days[1].kind, "sun");
        let (celsius, _) = documents(&snapshot(), &cfg(Units::Celsius), 1_000).expect("documents");
        assert_eq!(celsius.display.temp, "16\u{b0}");
        assert_eq!(doc.stale_at, 1_000 + REFRESH_MS);
        // the view's details
        assert!((more.now.gust_mph.unwrap() - 22.4).abs() < 0.1);
        assert_eq!(more.now.wind_deg, Some(225.0));
        assert_eq!(more.now.visibility_m, Some(10_000.0));
        assert_eq!(more.hourly[0].time, "2026-10-30T02:00");
        assert!(!more.hourly[0].is_day && more.hourly[0].rain_pct == Some(70.0));
        assert_eq!(more.daily[0].rain_pct, Some(80.0));
        assert!(more.daily[0].sunrise.is_none(), "an entity does not say when the sun rises");
        assert!(more.alerts.is_empty());
    }

    #[test]
    fn an_unavailable_entity_or_a_reading_without_a_temperature_makes_no_document() {
        assert!(documents(&json!({"available": false}), &cfg(Units::Fahrenheit), 1).is_none());
        assert!(documents(&json!({"available": true, "reading": {}, "condition": "sunny"}), &cfg(Units::Fahrenheit), 1).is_none());
    }

    #[test]
    fn the_conditions_map_to_the_codes_the_cards_icons_read() {
        for (word, kind) in [("sunny", "sun"), ("clear-night", "sun"), ("partlycloudy", "partly"), ("cloudy", "cloud"), ("fog", "fog"), ("pouring", "rain"), ("lightning-rainy", "storm"), ("snowy", "snow"), ("hail", "snow")] {
            assert_eq!(kind_for(wmo_of(word), true), kind, "{word}");
        }
    }
}
