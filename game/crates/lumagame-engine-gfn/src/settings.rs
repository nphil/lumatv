use lumagame_core::engine::Codec;
use lumagame_core::ipc::{Display, StreamInfo};
use lumagame_core::settings::{SettingsSchema, Values};
use serde_json::{Value, json};

/// The old app's quality tiers, read only to migrate its `prefs.json`: (id, width, height, fps, codec).
const TIERS: [(&str, u32, u32, u32, &str); 5] = [
    ("uhd120", 3840, 2160, 120, "auto"),
    ("qhd120", 2560, 1440, 120, "auto"),
    ("uhd60", 3840, 2160, 60, "auto"),
    ("uhd60av1", 3840, 2160, 60, "av1"),
    ("fhd120", 1920, 1080, 120, "auto"),
];

/// The stream fields use the keys and ranges of LumaGame's Streaming settings, so the app's common values apply to this engine.
pub fn schema(prefs: &Value) -> SettingsSchema {
    let tier = prefs["tier"].as_str().and_then(|s| TIERS.iter().find(|t| t.0 == s)).unwrap_or(&TIERS[0]);
    let bitrate = prefs["bitrateMbps"].as_i64().map_or(80, |n| (n.saturating_add(2) / 5 * 5).clamp(5, 150));
    let deadzone = prefs["deadzone"].as_i64().map_or(8, |n| n.clamp(0, 20));
    serde_json::from_value(json!({"sections": [{"title": "Stream", "fields": [
        {"key":"width","label":"Width","type":"number","min":1280,"max":3840,"step":640,"unit":"px","default":tier.1},
        {"key":"height","label":"Height","type":"number","min":720,"max":2160,"step":360,"unit":"px","default":tier.2},
        {"key":"fps","label":"Frame rate","type":"number","min":60,"max":120,"step":60,"unit":"fps","default":tier.3},
        {"key":"codec","label":"Video codec","type":"choice","default":tier.4,"options":[
            {"id":"auto","label":"Automatic"},{"id":"h264","label":"H.264"},{"id":"h265","label":"HEVC"},{"id":"av1","label":"AV1 (up to 60 fps)"}]},
        {"key":"hdr","label":"HDR","type":"toggle","default":prefs["hdr"].as_bool().unwrap_or(true),
         "help":"Needs an HDR display, an eligible plan and a game that supports HDR."},
        {"key":"bitrate-mbps","label":"Bitrate cap","type":"number","min":5,"max":150,"step":5,"unit":"Mbit/s","default":bitrate},
        {"key":"deadzone","label":"Stick dead zone","type":"number","min":0,"max":20,"step":1,"unit":"%","default":deadzone},
        {"key":"show-stats","label":"Show stats","type":"toggle","default":prefs["showStats"].as_bool().unwrap_or(false)},
        {"key":"sign-out","label":"Sign out","type":"action"}
    ]}]})).expect("the GFN schema is valid data")
}

pub fn profile(values: &Values, display: &Display) -> StreamInfo {
    let number = |key, default: u32| u32::try_from(integer(values, key, i64::from(default))).unwrap_or(default);
    let fps = number("fps", 120).min(display.refresh_hz).min(120);
    let choice = match values.get("codec") {
        Some(lumagame_core::settings::Value::Text(c)) => c.as_str(),
        _ => "auto",
    };
    let codec = if choice == "av1" && fps <= 60 && display.codecs.contains(&Codec::Av1) {
        Codec::Av1
    } else if choice != "h264" && display.codecs.contains(&Codec::H265) {
        Codec::H265
    } else {
        Codec::H264
    };
    StreamInfo {
        codec,
        width: number("width", 3840).min(display.width),
        height: number("height", 2160).min(display.height),
        fps,
        hdr: boolean(values, "hdr") && display.hdr10 && codec != Codec::H264,
        audio_channels: 2,
        sdr_reason: None,
    }
}

pub fn boolean(values: &Values, key: &str) -> bool {
    matches!(values.get(key), Some(lumagame_core::settings::Value::Bool(true)))
}

pub fn integer(values: &Values, key: &str, default: i64) -> i64 {
    match values.get(key) {
        Some(lumagame_core::settings::Value::Int(n)) => *n,
        _ => default,
    }
}

pub fn core_settings(values: &Values, display: &Display) -> Vec<(&'static str, Value)> {
    let p = profile(values, display);
    vec![
        ("resolution", json!(format!("{}x{}", p.width, p.height))),
        ("fps", json!(p.fps)),
        ("enableHdr", json!(p.hdr)),
        ("colorQuality", json!(if p.hdr { "10bit_420" } else { "8bit_420" })),
        // Depth precedes codec: the core checks a codec against its currently saved depth.
        ("codec", json!(p.codec.as_str())),
        ("maxBitrateMbps", json!(integer(values, "bitrate-mbps", 80))),
        ("controllerLeftStickDeadzone", json!(integer(values, "deadzone", 8))),
        ("controllerRightStickDeadzone", json!(integer(values, "deadzone", 8))),
    ]
}

pub fn pad_prefs(values: &Values) -> Value {
    json!({"deadzone": integer(values, "deadzone", 8)})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrated_defaults_keep_all_preferences_and_limit_av1() {
        let s = schema(&json!({"tier":"uhd60av1","hdr":false,"bitrateMbps":64,"deadzone":13,"showStats":true}));
        s.validate().unwrap();
        let v = s.defaults();
        assert_eq!(integer(&v, "bitrate-mbps", 0), 65);
        assert_eq!(integer(&v, "deadzone", 0), 13);
        assert!(boolean(&v, "show-stats"));
        let d = Display { width:3840,height:2160,refresh_hz:120,hdr10:true,codecs:vec![Codec::Av1,Codec::H265] };
        let p = profile(&v, &d);
        assert_eq!((p.codec,p.width,p.height,p.fps,p.hdr), (Codec::Av1,3840,2160,60,false));
        assert_eq!(profile(&schema(&Value::Null).defaults(), &d).codec, Codec::H265);
        assert_eq!(profile(&v, &Display { codecs:vec![Codec::H265], ..d }).codec, Codec::H265);
        let fhd = profile(&schema(&json!({"tier":"fhd120","bitrateMbps":500})).defaults(), &d);
        assert_eq!((fhd.width,fhd.height,fhd.fps), (1920,1080,120));
        assert_eq!(integer(&schema(&json!({"bitrateMbps":500})).defaults(), "bitrate-mbps", 0), 150);
    }

    #[test]
    fn the_apps_common_stream_values_apply_and_av1_needs_sixty_fps() {
        use lumagame_core::settings::Value as V;
        let s = schema(&Value::Null);
        let common = Values::from([
            ("width".into(), V::Int(2560)), ("height".into(), V::Int(1440)), ("fps".into(), V::Int(120)),
            ("hdr".into(), V::Bool(true)), ("bitrate-mbps".into(), V::Int(5)), ("codec".into(), V::Text("av1".into())),
        ]);
        for (key, value) in &common {
            assert_eq!(s.check(key, value), Ok(()), "{key}");
        }
        assert!(s.check("bitrate-mbps", &V::Int(155)).is_err());
        let v = s.resolve(&common);
        let d = Display { width:3840,height:2160,refresh_hz:120,hdr10:true,codecs:vec![Codec::H264,Codec::H265,Codec::Av1] };
        let p = profile(&v, &d);
        assert_eq!((p.codec,p.width,p.height,p.fps,p.hdr), (Codec::H265,2560,1440,120,true));
        let mut sixty = v.clone();
        sixty.insert("fps".into(), V::Int(60));
        assert_eq!(profile(&sixty, &d).codec, Codec::Av1);
        sixty.insert("codec".into(), V::Text("h264".into()));
        assert_eq!((profile(&sixty, &d).codec, profile(&sixty, &d).hdr), (Codec::H264, false));
        assert!(core_settings(&v, &d).contains(&("maxBitrateMbps", json!(5))));
    }
}
