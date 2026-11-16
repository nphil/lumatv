use anyhow::{Result, ensure};
use lumagame_core::{engine::{AuthMethod, Capabilities, Codec, Descriptor, EngineKind, Feature, Presentation, TargetKind, VideoCaps}, ids::EngineId, ipc::Display, settings::{SettingsSchema, Value, Values}};
use lumatv_gfn_pad::{Curve, PadConfig};
use punktfunk_core::{client::ConnectParams, config::Mode, quic};
use std::{collections::BTreeSet, time::Duration};

pub fn descriptor() -> Descriptor {
    Descriptor { id: EngineId::new("punktfunk").unwrap(), version: env!("CARGO_PKG_VERSION").into(), capabilities: Capabilities {
        kind: EngineKind::Host, targets: TargetKind::Host, auth: vec![AuthMethod::EnterPin], presentation: Presentation::Feeder,
        features: BTreeSet::from([Feature::Discovery, Feature::ManualHost, Feature::Wake, Feature::ManyTargets, Feature::Readiness, Feature::Resume, Feature::EndGame, Feature::Stats, Feature::RoundTrip, Feature::OverlayKeys, Feature::Rumble]),
        video: Some(VideoCaps { codecs: vec![Codec::H264,Codec::H265,Codec::Av1], width:3840,height:2160,fps:120,hdr10:true }),
    }}
}
/// The stream and stick fields use the keys of LumaGame's common settings, so the app's values apply to this engine.
pub fn schema() -> SettingsSchema {
    serde_json::from_str(r#"{"sections":[{"title":"Stream","fields":[
        {"key":"width","label":"Width","type":"number","min":1280,"max":3840,"step":640,"unit":"px","default":3840},
        {"key":"height","label":"Height","type":"number","min":720,"max":2160,"step":360,"unit":"px","default":2160},
        {"key":"fps","label":"Frame rate","type":"number","min":30,"max":120,"step":30,"unit":"fps","default":120},
        {"key":"codec","label":"Video codec","type":"choice","options":[{"id":"auto","label":"Automatic"},{"id":"h264","label":"H.264"},{"id":"h265","label":"HEVC"},{"id":"av1","label":"AV1 (up to 60 fps)"}],"default":"auto"},
        {"key":"hdr","label":"HDR10","type":"toggle","default":true,"help":"HEVC on an HDR-capable display."},
        {"key":"bitrate-mbps","label":"Bitrate","type":"number","min":5,"max":200,"step":5,"unit":"Mb/s","default":50},
        {"key":"pacing","label":"Presentation","type":"choice","options":[{"id":"latency","label":"Lowest latency"},{"id":"smooth","label":"Smoothness (two frames)"}],"default":"latency"},
        {"key":"rescan","label":"Find PCs again","type":"action"}
    ]},{"title":"Controller","fields":[
        {"key":"deadzone","label":"Stick dead zone","type":"number","min":0,"max":20,"step":1,"unit":"%","default":8},
        {"key":"anti-deadzone","label":"Stick anti-deadzone","type":"number","min":0,"max":30,"step":1,"unit":"%","default":0},
        {"key":"stick-curve","label":"Stick response","type":"choice","options":[{"id":"linear","label":"Linear"},{"id":"precise","label":"Precise"},{"id":"quick","label":"Quick"}],"default":"linear"}
    ]}]}"#).expect("static schema")
}
/// The pads shaped as the settings say: the pad crate applies the sticks' settings before a report goes to the host.
pub fn pad_config(values: &Values) -> PadConfig {
    let values = schema().resolve(values);
    let percent = |key| u8::try_from(number(&values,key)).expect("schema range");
    PadConfig { raw:true, deadzone_percent:percent("deadzone"), anti_deadzone_percent:percent("anti-deadzone"),
        curve:Curve::from_name(text(&values,"stick-curve")).expect("schema choice"), ..PadConfig::default() }
}
pub fn text<'a>(v: &'a Values, key: &str) -> &'a str {
    match &v[key] { Value::Text(s) => s, _ => unreachable!("resolved schema") }
}
fn number(v: &Values, key: &str) -> u32 {
    match v[key] { Value::Int(n) => u32::try_from(n).expect("schema range"), _ => unreachable!("resolved schema") }
}
pub fn params(display: &Display, values: &Values, host: &str, port: u16) -> Result<ConnectParams> {
    let values = schema().resolve(values);
    let width = number(&values,"width").min(display.width);
    let height = number(&values,"height").min(display.height);
    let fps = number(&values,"fps").min(display.refresh_hz);
    ensure!(width >= 640 && height >= 360 && fps > 0, "The display has no usable stream mode.");
    let bit = |c| match c { Codec::H264=>quic::CODEC_H264, Codec::H265=>quic::CODEC_HEVC, Codec::Av1=>quic::CODEC_AV1, _=>0 };
    // The TV's AV1 decoder is limited to 60 fps.
    let offered = display.codecs.iter().copied().fold(0, |b,c| b | bit(c)) & if fps > 60 { !quic::CODEC_AV1 } else { !0 };
    let preferred = match text(&values,"codec") { "h264"=>quic::CODEC_H264,"h265"=>quic::CODEC_HEVC,"av1"=>quic::CODEC_AV1,_=>0 };
    ensure!(!(preferred == quic::CODEC_AV1 && fps > 60), "AV1 is limited to 60 fps on this TV; choose HEVC for higher frame rates.");
    ensure!(offered != 0 && (preferred == 0 || offered & preferred != 0), "The display cannot decode the selected codec.");
    let hdr = display.hdr10 && values["hdr"] == Value::Bool(true) && offered & quic::CODEC_HEVC != 0 && (preferred == 0 || preferred == quic::CODEC_HEVC);
    let mut p = ConnectParams::new(host,port,Mode {width,height,refresh_hz:fps},Duration::from_secs(20));
    p.video_codecs = if hdr { quic::CODEC_HEVC } else { offered };
    p.preferred_codec = preferred;
    p.video_caps = quic::VIDEO_CAP_CHACHA20 | if hdr { quic::VIDEO_CAP_10BIT | quic::VIDEO_CAP_HDR } else {0};
    p.bitrate_kbps = number(&values,"bitrate-mbps") * 1000;
    p.audio_channels = 2;
    p.audio_layout = punktfunk_core::audio::AudioLayout::Standard;
    p.studio_range = true;
    p.studio_range_hdr = true;
    p.name = Some("LumaGame TV".into());
    p.rebuilds_decoder = true;
    Ok(p)
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn preferences_never_exceed_the_display() {
        schema().validate().unwrap();
        let display = Display { width:1920,height:1080,refresh_hz:60,hdr10:false,codecs:vec![Codec::H264] };
        let p = params(&display,&Values::new(),"pc",9777).unwrap();
        assert_eq!((p.mode.width,p.mode.height,p.mode.refresh_hz,p.video_codecs,p.audio_channels),(1920,1080,60,quic::CODEC_H264,2));
        assert_eq!(p.video_caps & quic::VIDEO_CAP_HDR, 0);
        assert!(params(&display,&Values::from([("codec".into(),Value::Text("av1".into()))]),"pc",9777).is_err());
    }
    #[test] fn the_apps_common_stream_values_apply_and_av1_needs_sixty_fps() {
        let common = Values::from([
            ("width".into(),Value::Int(2560)),("height".into(),Value::Int(1440)),("fps".into(),Value::Int(120)),
            ("hdr".into(),Value::Bool(false)),("bitrate-mbps".into(),Value::Int(150)),("codec".into(),Value::Text("auto".into())),
        ]);
        for (key,value) in &common { assert_eq!(schema().check(key,value),Ok(()),"{key}"); }
        let display = Display { width:3840,height:2160,refresh_hz:120,hdr10:true,codecs:vec![Codec::H264,Codec::H265,Codec::Av1] };
        let p = params(&display,&common,"pc",9777).unwrap();
        assert_eq!((p.mode.width,p.mode.height,p.mode.refresh_hz,p.bitrate_kbps),(2560,1440,120,150_000));
        assert_eq!(p.video_codecs & quic::CODEC_AV1, 0);
        let mut av1 = common.clone();
        av1.insert("codec".into(),Value::Text("av1".into()));
        assert!(params(&display,&av1,"pc",9777).is_err());
        av1.insert("fps".into(),Value::Int(60));
        assert_eq!(params(&display,&av1,"pc",9777).unwrap().preferred_codec, quic::CODEC_AV1);
    }
    #[test] fn the_apps_stick_values_shape_the_pads() {
        let sticks = Values::from([("deadzone".into(),Value::Int(3)),("anti-deadzone".into(),Value::Int(10)),("stick-curve".into(),Value::Text("precise".into()))]);
        for (key,value) in &sticks { assert_eq!(schema().check(key,value),Ok(()),"{key}"); }
        let c = pad_config(&sticks);
        assert_eq!((c.deadzone_percent,c.anti_deadzone_percent,c.curve),(3,10,Curve::Precise));
        let d = pad_config(&Values::new());
        assert_eq!((d.deadzone_percent,d.anti_deadzone_percent,d.curve),(8,0,Curve::Linear),"the pad crate's own defaults");
    }
}
