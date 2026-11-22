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
        {"key":"frame-pacing","label":"Frame pacing","type":"choice","default":"smooth","options":[
            {"id":"smooth","label":"Smooth"},{"id":"latency","label":"Lowest latency"}],
         "help":"Smooth evens out frames that arrive in bursts, holding a frame for at most 1.5 frame times."},
        {"key":"cloud-gsync","label":"Cloud G-SYNC (experimental)","type":"toggle","default":false,
         "help":"Asks the server for Cloud G-SYNC, a stream that follows the game's frame rate, and stops padding it to a fixed 120 fps. NVIDIA offers it only in its Windows and Mac apps, on an Ultimate plan; the TV's own refresh rate does not change. May judder."},
        {"key":"client-identity","label":"Client identity","type":"choice","default":"linux","options":[
            {"id":"linux","label":"Linux PC (current)"},{"id":"lg-webos","label":"LG webOS TV (official LG app)"},{"id":"windows","label":"Windows PC (official GFN app)"}],
         "help":"Experimental. Who the session request says it is, to see whether the server then grants HDR. LG webOS TV sends the official LG app's client type, device and user agent; Windows PC sends the Windows app's user agent, operating system and platform name. The server may also change the resolution or frame rate it grants, or refuse the session. Applies from the next launch."},
        {"key":"bitrate-mbps","label":"Bitrate cap","type":"number","min":5,"max":150,"step":5,"unit":"Mbit/s","default":bitrate},
        {"key":"deadzone","label":"Stick dead zone","type":"number","min":0,"max":20,"step":1,"unit":"%","default":deadzone},
        {"key":"anti-deadzone","label":"Stick anti-deadzone","type":"number","min":0,"max":30,"step":1,"unit":"%","default":0},
        {"key":"stick-curve","label":"Stick response","type":"choice","default":"linear","options":[
            {"id":"linear","label":"Linear"},{"id":"precise","label":"Precise"},{"id":"quick","label":"Quick"}]},
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
        vui: None,
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
        // SDR HEVC is 10-bit as well, as the official client sends it whenever the decoder takes Main10 (every
        // webOS HEVC decoder does: HDR uses it); it keeps dark gradients from banding.
        ("colorQuality", json!(if p.hdr || p.codec == Codec::H265 { "10bit_420" } else { "8bit_420" })),
        // Depth precedes codec: the core checks a codec against its currently saved depth.
        ("codec", json!(p.codec.as_str())),
        ("maxBitrateMbps", json!(integer(values, "bitrate-mbps", 80))),
        ("controllerLeftStickDeadzone", json!(integer(values, "deadzone", 8))),
        ("controllerRightStickDeadzone", json!(integer(values, "deadzone", 8))),
        // `forced` asks for Cloud G-SYNC without the PC client's VRR-display check, which a TV cannot pass; `disabled` stops a
        // value left in the core's own settings from asking for it. The core's session request and the streamer's ANNOUNCE read this one key.
        ("nativeCloudGsyncMode", json!(if boolean(values, "cloud-gsync") { "forced" } else { "disabled" })),
        // Which client the session request says it is; the core reads it for every CloudMatch request. Anything but the LG or Windows app is the desktop identity it sent before.
        ("clientIdentity", json!(match values.get("client-identity") {
            Some(lumagame_core::settings::Value::Text(id)) if id == "lg-webos" => "lgWebos",
            Some(lumagame_core::settings::Value::Text(id)) if id == "windows" => "windows",
            _ => "desktop",
        })),
    ]
}

/// What the streamer reads when a stream starts (`LUMATV_GFN_PREFS`): the stick settings, shaped by the pad crate on the
/// TV, and the frame pacing mode.
pub fn runtime_prefs(values: &Values) -> Value {
    let text = |key, default| match values.get(key) {
        Some(lumagame_core::settings::Value::Text(c)) => c.clone(),
        _ => String::from(default),
    };
    json!({"deadzone": integer(values, "deadzone", 8), "antiDeadzone": integer(values, "anti-deadzone", 0),
        "curve": text("stick-curve", "linear"), "framePacing": text("frame-pacing", "smooth")})
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
        let mut sdr = v.clone();
        sdr.insert("hdr".into(), V::Bool(false));
        assert!(core_settings(&sdr, &d).contains(&("colorQuality", json!("10bit_420"))));
        assert!(core_settings(&sixty, &d).contains(&("colorQuality", json!("8bit_420"))));
    }

    #[test]
    fn the_apps_stick_and_pacing_values_reach_the_streamer() {
        use lumagame_core::settings::Value as V;
        let s = schema(&Value::Null);
        let picked = Values::from([("deadzone".into(), V::Int(4)), ("anti-deadzone".into(), V::Int(12)),
            ("stick-curve".into(), V::Text("quick".into())), ("frame-pacing".into(), V::Text("latency".into()))]);
        for (key, value) in &picked {
            assert_eq!(s.check(key, value), Ok(()), "{key}");
        }
        assert!(s.check("stick-curve", &V::Text("wobbly".into())).is_err());
        assert_eq!(runtime_prefs(&s.resolve(&picked)), json!({"deadzone":4,"antiDeadzone":12,"curve":"quick","framePacing":"latency"}));
        assert_eq!(runtime_prefs(&s.defaults()), json!({"deadzone":8,"antiDeadzone":0,"curve":"linear","framePacing":"smooth"}));
    }

    #[test]
    fn cloud_gsync_is_requested_only_when_the_viewer_turns_it_on() {
        use lumagame_core::settings::Value as V;
        let s = schema(&Value::Null);
        s.validate().unwrap();
        let d = Display { width:3840,height:2160,refresh_hz:120,hdr10:true,codecs:vec![Codec::H265] };
        let mode = |v: &Values| core_settings(v, &d).into_iter().find(|(key, _)| *key == "nativeCloudGsyncMode").map(|(_, value)| value);
        assert_eq!(s.check("cloud-gsync", &V::Bool(true)), Ok(()));
        assert!(!boolean(&s.defaults(), "cloud-gsync"));
        assert_eq!(mode(&s.defaults()), Some(json!("disabled")), "off pushes `disabled`, so a stale core setting cannot ask for it");
        let on = s.resolve(&Values::from([("cloud-gsync".into(), V::Bool(true))]));
        assert_eq!(mode(&on), Some(json!("forced")));
    }

    #[test]
    fn the_client_identity_is_the_desktop_client_unless_the_viewer_picks_an_official_app() {
        use lumagame_core::settings::Value as V;
        let s = schema(&Value::Null);
        s.validate().unwrap();
        let d = Display { width:3840,height:2160,refresh_hz:120,hdr10:true,codecs:vec![Codec::H265] };
        let identity = |v: &Values| core_settings(v, &d).into_iter().find(|(key, _)| *key == "clientIdentity").map(|(_, value)| value);
        assert!(s.check("client-identity", &V::Text("toaster".into())).is_err());
        assert_eq!(identity(&s.defaults()), Some(json!("desktop")), "the default is the identity the core sent before");
        for (id, core) in [("lg-webos", "lgWebos"), ("windows", "windows")] {
            assert_eq!(s.check("client-identity", &V::Text(id.into())), Ok(()));
            let picked = s.resolve(&Values::from([("client-identity".into(), V::Text(id.into()))]));
            assert_eq!(identity(&picked), Some(json!(core)));
        }
    }
}
