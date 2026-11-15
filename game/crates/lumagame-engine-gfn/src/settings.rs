use lumagame_core::engine::Codec;
use lumagame_core::ipc::{Display, StreamInfo};
use lumagame_core::settings::{SettingsSchema, Values};
use serde_json::{Value, json};

const TIERS: [(&str, &str, u32, u32, u32); 5] = [
    ("uhd120", "4K 120", 3840, 2160, 120),
    ("qhd120", "1440p 120", 2560, 1440, 120),
    ("uhd60", "4K 60", 3840, 2160, 60),
    ("uhd60av1", "4K 60 AV1", 3840, 2160, 60),
    ("fhd120", "1080p 120", 1920, 1080, 120),
];

pub fn schema(prefs: &Value) -> SettingsSchema {
    let tier = prefs["tier"].as_str().filter(|s| TIERS.iter().any(|t| t.0 == *s)).unwrap_or("uhd120");
    let nearest = |key: &str, ladder: &[i64], default: i64| {
        let n = prefs[key].as_i64().unwrap_or(default);
        *ladder.iter().min_by_key(|v| i128::from(**v).abs_diff(i128::from(n))).unwrap()
    };
    serde_json::from_value(json!({"sections": [{"title": "Stream", "fields": [
        {"key":"tier","label":"Quality","type":"choice","default":tier,
         "options":TIERS.map(|t| json!({"id":t.0,"label":t.1}))},
        {"key":"hdr","label":"HDR","type":"toggle","default":prefs["hdr"].as_bool().unwrap_or(true),
         "help":"Needs an HDR display, an eligible plan and a game that supports HDR."},
        {"key":"bitrate-mbps","label":"Bitrate cap","type":"number","min":1,"max":100,"step":1,"unit":"Mbit/s",
         "default":nearest("bitrateMbps", &[20,35,50,65,80,100], 80)},
        {"key":"deadzone","label":"Stick dead zone","type":"number","min":0,"max":20,"step":1,"unit":"%",
         "default":nearest("deadzone", &[0,2,4,6,8,10,12,15,20], 8)},
        {"key":"show-stats","label":"Show stats","type":"toggle","default":prefs["showStats"].as_bool().unwrap_or(false)},
        {"key":"sign-out","label":"Sign out","type":"action"}
    ]}]})).expect("the GFN schema is valid data")
}

pub fn profile(values: &Values, display: &Display) -> StreamInfo {
    let text = |key| match values.get(key) {
        Some(lumagame_core::settings::Value::Text(s)) => s.as_str(),
        _ => "",
    };
    let tier = TIERS.iter().find(|t| t.0 == text("tier")).unwrap_or(&TIERS[0]);
    let fps = tier.4.min(display.refresh_hz).min(120);
    let codec = if tier.0 == "uhd60av1" && fps <= 60 && display.codecs.contains(&Codec::Av1) {
        Codec::Av1
    } else if display.codecs.contains(&Codec::H265) {
        Codec::H265
    } else {
        Codec::H264
    };
    StreamInfo {
        codec,
        width: tier.2.min(display.width),
        height: tier.3.min(display.height),
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
        assert_eq!(integer(&v, "deadzone", 0), 12);
        assert!(boolean(&v, "show-stats"));
        let d = Display { width:3840,height:2160,refresh_hz:120,hdr10:true,codecs:vec![Codec::Av1,Codec::H265] };
        let p = profile(&v, &d);
        assert_eq!((p.codec,p.fps,p.hdr), (Codec::Av1,60,false));
        assert_eq!(profile(&schema(&Value::Null).defaults(), &d).codec, Codec::H265);
        assert_eq!(profile(&v, &Display { codecs:vec![Codec::H265], ..d }).codec, Codec::H265);
    }
}
