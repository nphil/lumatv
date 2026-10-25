mod common;

use common::*;
use fetch::FetchError;
use lumatv_feeds::weather::*;
use lumatv_feeds::{EXPIRE_MS, FeedError, REFRESH_MS, SCHEMA};

const NOW: u64 = 1_792_900_000_000;

const FORECAST_F: &str = "https://api.open-meteo.com/v1/forecast?latitude=33.723&longitude=-84.4702\
&current=temperature_2m,apparent_temperature,weather_code,is_day,relative_humidity_2m,wind_speed_10m\
&daily=weather_code,temperature_2m_max,temperature_2m_min&temperature_unit=fahrenheit\
&wind_speed_unit=mph&timezone=auto&forecast_days=7";

fn atlanta_geo() -> Geo {
    Geo {
        place: "Atlanta".to_owned(),
        lat: 33.723,
        lon: -84.4702,
    }
}

#[test]
fn geo_from_zippopotam_fixture() {
    let geo = parse_geo(ZIPPOPOTAM.as_bytes()).unwrap();
    assert_eq!(geo, atlanta_geo());
}

#[test]
fn geo_rejects_bad_replies() {
    for body in [
        &b"not json"[..],
        b"{}",
        br#"{"places":[]}"#,
        br#"{"places":[{"place name":"X","latitude":33.7,"longitude":"-84"}]}"#,
        br#"{"places":[{"place name":"X","latitude":"abc","longitude":"-84"}]}"#,
        br#"{"places":[{"place name":"","latitude":"1","longitude":"2"}]}"#,
        br#"{"places":[{"latitude":"1","longitude":"2"}]}"#,
    ] {
        assert!(matches!(parse_geo(body), Err(FeedError::Parse(_))), "{}", String::from_utf8_lossy(body));
    }
}

#[test]
fn urls_are_exact() {
    assert_eq!(geocode_url("30311"), "https://api.zippopotam.us/us/30311");
    assert_eq!(forecast_url(&atlanta_geo(), Units::Fahrenheit), FORECAST_F);
    assert_eq!(
        forecast_url(&atlanta_geo(), Units::Celsius),
        FORECAST_F.replace("temperature_unit=fahrenheit", "temperature_unit=celsius")
    );
}

#[test]
fn units_names() {
    assert_eq!(Units::Fahrenheit.as_str(), "fahrenheit");
    assert_eq!(Units::Celsius.as_str(), "celsius");
    assert_eq!(Units::parse("fahrenheit"), Some(Units::Fahrenheit));
    assert_eq!(Units::parse("celsius"), Some(Units::Celsius));
    assert_eq!(Units::parse("Celsius"), None);
    assert_eq!(Units::parse(""), None);
}

#[test]
fn zip_validation() {
    assert!(validate_zip("30311").is_ok());
    assert!(validate_zip("00000").is_ok());
    for bad in ["", "3031", "303111", "3031a", " 30311", "30311 ", "３０３１１", "-0311"] {
        assert!(matches!(validate_zip(bad), Err(FeedError::Config(_))), "{bad:?}");
    }
}

#[test]
fn forecast_fixture_to_document() {
    let doc = weather_doc(NOW);
    assert_eq!(doc.schema, SCHEMA);
    assert_eq!(doc.fetched_at, NOW);
    assert_eq!(doc.stale_at, NOW + REFRESH_MS);
    assert_eq!(doc.expires_at, NOW + EXPIRE_MS);
    assert_eq!((doc.zip.as_str(), doc.units), ("30311", Units::Fahrenheit));
    assert_eq!((doc.place.as_str(), doc.lat, doc.lon), ("Atlanta", 33.723, -84.4702));

    let c = &doc.current;
    assert_eq!(c.time, "2026-10-25T00:15");
    assert_eq!((c.temp, c.feels_like, c.humidity, c.wind_mph), (54.2, 49.7, 72.0, 7.8));
    assert_eq!(c.code, 0);
    assert!(!c.is_day);

    let d = &doc.display;
    assert_eq!(d.place, "Atlanta");
    assert_eq!(d.temp, "54\u{b0}");
    assert_eq!(d.cond, "Clear");
    assert_eq!(d.stats, "Feels like 50\u{b0} \u{b7} 72% humidity \u{b7} 8 mph");
    assert_eq!(d.now_kind, "moon");
}

#[test]
fn forecast_fixture_daily_rows() {
    let doc = weather_doc(NOW);
    assert_eq!(doc.daily.len(), 7);
    assert_eq!(doc.display.days.len(), 7);
    // dates, codes, highs and lows straight from the fixture; labels: 2026-10-25 is a Sunday.
    let want = [
        ("2026-10-25", 0, 69.8, 49.4, "Today", "sun", "70", "49"),
        ("2026-10-26", 3, 72.1, 49.9, "Mon", "cloud", "72", "50"),
        ("2026-10-27", 3, 76.0, 55.8, "Tue", "cloud", "76", "56"),
        ("2026-10-28", 3, 72.6, 56.0, "Wed", "cloud", "73", "56"),
        ("2026-10-29", 3, 75.7, 57.9, "Thu", "cloud", "76", "58"),
        ("2026-10-30", 3, 77.5, 57.3, "Fri", "cloud", "78", "57"),
        ("2026-10-31", 3, 75.4, 57.5, "Sat", "cloud", "75", "58"),
    ];
    for (i, (date, code, hi, lo, day, kind, rhi, rlo)) in want.into_iter().enumerate() {
        let d = &doc.daily[i];
        assert_eq!((d.date.as_str(), d.code, d.hi, d.lo), (date, code, hi, lo), "daily {i}");
        let r = &doc.display.days[i];
        assert_eq!(r.day, day, "row {i}");
        assert_eq!(r.kind, kind, "row {i}");
        assert_eq!(r.hi, format!("{rhi}\u{b0}"), "row {i}");
        assert_eq!(r.lo, format!("{rlo}\u{b0}"), "row {i}");
    }
}

#[test]
fn forecast_skips_null_rows_and_rejects_missing_fields() {
    let geo = atlanta_geo();
    let cfg = atlanta();
    let body = br#"{"current":{"time":"2026-10-25T12:00","temperature_2m":-2.5,"apparent_temperature":-7.5,
        "weather_code":71,"is_day":1,"relative_humidity_2m":72.5,"wind_speed_10m":0.4},
        "daily":{"time":["2026-10-25","2026-10-26","2026-10-27"],"weather_code":[71,null,3],
        "temperature_2m_max":[1.5,2.0,3.0],"temperature_2m_min":[-3.5,-2.0,null]}}"#;
    let doc = parse_forecast(body, &geo, &cfg, NOW).unwrap();
    assert_eq!(doc.daily.len(), 1);
    assert_eq!(doc.display.temp, "-2\u{b0}");
    assert_eq!(doc.display.stats, "Feels like -7\u{b0} \u{b7} 72.5% humidity \u{b7} 0 mph");
    assert_eq!(doc.display.now_kind, "snow");
    assert_eq!(doc.display.cond, "Snow");
    assert_eq!(doc.display.days[0].hi, "2\u{b0}");
    assert_eq!(doc.display.days[0].lo, "-3\u{b0}");

    for body in [
        &b"nope"[..],
        b"{}",
        br#"{"current":{},"daily":{}}"#,
        br#"{"current":{"time":"t","temperature_2m":1,"apparent_temperature":1,"weather_code":0,"is_day":0,
            "relative_humidity_2m":1,"wind_speed_10m":1}}"#,
        br#"{"current":{"time":"t","temperature_2m":"1","apparent_temperature":1,"weather_code":0,"is_day":0,
            "relative_humidity_2m":1,"wind_speed_10m":1},"daily":{"time":[],"weather_code":[],
            "temperature_2m_max":[],"temperature_2m_min":[]}}"#,
    ] {
        assert!(matches!(parse_forecast(body, &geo, &cfg, NOW), Err(FeedError::Parse(_))));
    }
}

/// The QML's icon rules restated as ranges, independent of the implementation's if-chain.
fn expected_kind(code: i64, day: bool) -> &'static str {
    match code {
        0 if day => "sun",
        0 => "moon",
        i64::MIN..=2 if day => "partly",
        i64::MIN..=2 => "partly-night",
        3 => "cloud",
        45 | 48 => "fog",
        95.. => "storm",
        71..=77 | 85 | 86 => "snow",
        61..=94 => "rain",
        51..=60 => "drizzle",
        _ => "cloud",
    }
}

fn expected_text(code: i64) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        95.. => "Thunderstorm",
        71..=77 | 85 | 86 => "Snow",
        80..=94 => "Showers",
        61..=79 => "Rain",
        51..=60 => "Drizzle",
        _ => "Cloudy",
    }
}

#[test]
fn kind_and_text_over_every_code() {
    for code in -1..=120 {
        for day in [true, false] {
            assert_eq!(kind_for(code, day), expected_kind(code, day), "kind_for({code}, {day})");
        }
        assert_eq!(text_for(code), expected_text(code), "text_for({code})");
    }
}

#[test]
fn kind_and_text_spot_checks() {
    assert_eq!(kind_for(0, true), "sun");
    assert_eq!(kind_for(0, false), "moon");
    assert_eq!(kind_for(1, true), "partly");
    assert_eq!(kind_for(2, false), "partly-night");
    assert_eq!(kind_for(3, true), "cloud");
    assert_eq!(kind_for(45, true), "fog");
    assert_eq!(kind_for(53, true), "drizzle");
    assert_eq!(kind_for(63, true), "rain");
    assert_eq!(kind_for(73, true), "snow");
    assert_eq!(kind_for(80, true), "rain");
    assert_eq!(kind_for(85, true), "snow");
    assert_eq!(kind_for(95, true), "storm");
    assert_eq!(kind_for(-1, true), "partly");
    assert_eq!(kind_for(10, true), "cloud");
    assert_eq!(text_for(53), "Drizzle");
    assert_eq!(text_for(63), "Rain");
    assert_eq!(text_for(80), "Showers");
    assert_eq!(text_for(86), "Snow");
    assert_eq!(text_for(99), "Thunderstorm");
    assert_eq!(text_for(10), "Cloudy");
}

#[test]
fn weekday_names() {
    let week = [
        ("2026-10-25", "Sun"),
        ("2026-10-26", "Mon"),
        ("2026-10-27", "Tue"),
        ("2026-10-28", "Wed"),
        ("2026-10-29", "Thu"),
        ("2026-10-30", "Fri"),
        ("2026-10-31", "Sat"),
        ("1970-01-01", "Thu"),
        ("2000-01-01", "Sat"),
        ("2028-02-29", "Tue"),
        ("2000-02-29", "Tue"),
        ("2028-03-01", "Wed"),
    ];
    for (date, name) in week {
        assert_eq!(day_name(date), Some(name), "{date}");
    }
}

#[test]
fn invalid_dates_have_no_name() {
    for bad in [
        "", "2026-02-29", "2100-02-29", "2026-13-01", "2026-00-10", "2026-10-00", "2026-10-32", "2026-04-31", "2026-1-05",
        "26-10-25", "2026/10/25", "2026-10-25T00:00", "abcd-ef-gh", "2026-10-2a", "+026-10-25", "２０２６-10-25",
    ] {
        assert_eq!(day_name(bad), None, "{bad:?}");
    }
}

#[test]
fn document_json_shape() {
    let doc = weather_doc(NOW);
    let v = serde_json::to_value(&doc).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut want = vec![
        "schema", "fetchedAt", "staleAt", "expiresAt", "zip", "units", "place", "lat", "lon", "current", "daily", "display",
    ];
    want.sort_unstable();
    assert_eq!(keys, want);
    assert_eq!(v["units"], "fahrenheit");
    assert_eq!(v["schema"], 1);
    assert_eq!(v["fetchedAt"], NOW);

    let mut cur: Vec<&str> = v["current"].as_object().unwrap().keys().map(String::as_str).collect();
    cur.sort_unstable();
    assert_eq!(cur, ["code", "feelsLike", "humidity", "isDay", "temp", "time", "windMph"]);
    let mut disp: Vec<&str> = v["display"].as_object().unwrap().keys().map(String::as_str).collect();
    disp.sort_unstable();
    assert_eq!(disp, ["cond", "days", "nowKind", "place", "stats", "temp"]);
    let mut row: Vec<&str> = v["display"]["days"][0].as_object().unwrap().keys().map(String::as_str).collect();
    row.sort_unstable();
    assert_eq!(row, ["day", "hi", "kind", "lo"]);
    let mut day: Vec<&str> = v["daily"][0].as_object().unwrap().keys().map(String::as_str).collect();
    day.sort_unstable();
    assert_eq!(day, ["code", "date", "hi", "lo"]);

    let back: WeatherDoc = serde_json::from_value(v).unwrap();
    assert_eq!(back, doc);
}

fn geocode_ok() -> (&'static str, Result<Vec<u8>, FetchError>) {
    ("https://api.zippopotam.us/us/30311", ok(ZIPPOPOTAM))
}

#[test]
fn refresh_without_previous_asks_twice() {
    let script = Script::new(vec![geocode_ok(), (FORECAST_F, ok(OPEN_METEO))]);
    let doc = refresh(&script, &atlanta(), None, NOW).unwrap();
    assert_eq!(script.requests(), ["https://api.zippopotam.us/us/30311", FORECAST_F]);
    assert_eq!(doc, weather_doc(NOW));
}

#[test]
fn refresh_reuses_the_place_of_a_previous_document_of_the_same_zip() {
    let mut previous = weather_doc(NOW - 1000);
    previous.lat = 1.5;
    previous.lon = 2.5;
    let url = forecast_url(
        &Geo {
            place: "Atlanta".to_owned(),
            lat: 1.5,
            lon: 2.5,
        },
        Units::Fahrenheit,
    );
    let script = Script::new(vec![(url.as_str(), ok(OPEN_METEO))]);
    let doc = refresh(&script, &atlanta(), Some(&previous), NOW).unwrap();
    assert_eq!(script.requests(), [url]);
    assert_eq!((doc.place.as_str(), doc.lat, doc.lon, doc.fetched_at), ("Atlanta", 1.5, 2.5, NOW));
}

#[test]
fn refresh_geocodes_again_for_another_zip() {
    let mut previous = weather_doc(NOW - 1000);
    previous.zip = "30312".to_owned();
    let script = Script::new(vec![geocode_ok(), (FORECAST_F, ok(OPEN_METEO))]);
    refresh(&script, &atlanta(), Some(&previous), NOW).unwrap();
    assert_eq!(script.requests(), ["https://api.zippopotam.us/us/30311", FORECAST_F]);
}

#[test]
fn refresh_geocodes_again_when_the_saved_place_is_empty() {
    let mut previous = weather_doc(NOW - 1000);
    previous.place.clear();
    let script = Script::new(vec![geocode_ok(), (FORECAST_F, ok(OPEN_METEO))]);
    refresh(&script, &atlanta(), Some(&previous), NOW).unwrap();
    assert_eq!(script.requests().len(), 2);
}

#[test]
fn refresh_uses_the_configured_units() {
    let cfg = WeatherConfig {
        zip: "30311".to_owned(),
        units: Units::Celsius,
    };
    let url = FORECAST_F.replace("temperature_unit=fahrenheit", "temperature_unit=celsius");
    let script = Script::new(vec![geocode_ok(), (url.as_str(), ok(OPEN_METEO))]);
    let doc = refresh(&script, &cfg, None, NOW).unwrap();
    assert_eq!(doc.units, Units::Celsius);
}

#[test]
fn refresh_geocode_failure_is_an_error_and_no_forecast_is_asked() {
    let script = Script::new(vec![
        ("https://api.zippopotam.us/us/30311", Err(FetchError::Offline)),
        (FORECAST_F, ok(OPEN_METEO)),
    ]);
    let err = refresh(&script, &atlanta(), None, NOW).unwrap_err();
    assert_eq!(err, FeedError::Fetch(FetchError::Offline));
    assert_eq!(script.requests(), ["https://api.zippopotam.us/us/30311"]);
}

#[test]
fn refresh_geocode_garbage_is_a_parse_error() {
    let script = Script::new(vec![("https://api.zippopotam.us/us/30311", ok("<html>"))]);
    assert!(matches!(refresh(&script, &atlanta(), None, NOW), Err(FeedError::Parse(_))));
    assert_eq!(script.requests().len(), 1);
}

#[test]
fn refresh_forecast_http_error() {
    let script = Script::new(vec![geocode_ok(), (FORECAST_F, Err(FetchError::Http(400)))]);
    let err = refresh(&script, &atlanta(), None, NOW).unwrap_err();
    assert_eq!(err, FeedError::Fetch(FetchError::Http(400)));
    assert_eq!(err.code(), "http");
}

#[test]
fn refresh_forecast_garbage_is_a_parse_error() {
    let script = Script::new(vec![geocode_ok(), (FORECAST_F, ok("garbage"))]);
    let err = refresh(&script, &atlanta(), None, NOW).unwrap_err();
    assert!(matches!(err, FeedError::Parse(_)));
    assert_eq!(err.code(), "parse");
}

#[test]
fn refresh_rejects_a_bad_zip_before_any_request() {
    let script = Script::new(vec![]);
    let cfg = WeatherConfig {
        zip: "3031".to_owned(),
        units: Units::Fahrenheit,
    };
    assert!(matches!(refresh(&script, &cfg, None, NOW), Err(FeedError::Config(_))));
    assert!(script.requests().is_empty());
}
