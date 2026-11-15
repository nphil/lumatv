use anyhow::{Result, ensure};
use lumagame_core::{engine::{AuthMethod, Capabilities, Codec, Descriptor, EngineKind, Feature, Presentation, TargetKind, VideoCaps}, ids::EngineId, ipc::Display, settings::{SettingsSchema, Value, Values}};
use punktfunk_core::{client::ConnectParams, config::Mode, quic};
use std::{collections::BTreeSet, time::Duration};

pub fn descriptor() -> Descriptor {
    Descriptor { id: EngineId::new("punktfunk").unwrap(), version: env!("CARGO_PKG_VERSION").into(), capabilities: Capabilities {
        kind: EngineKind::Host, targets: TargetKind::Host, auth: vec![AuthMethod::EnterPin], presentation: Presentation::Feeder,
        features: BTreeSet::from([Feature::Discovery, Feature::ManualHost, Feature::Wake, Feature::ManyTargets, Feature::Readiness, Feature::Resume, Feature::EndGame, Feature::Stats, Feature::OverlayKeys, Feature::Rumble]),
        video: Some(VideoCaps { codecs: vec![Codec::H264,Codec::H265,Codec::Av1], width:3840,height:2160,fps:120,hdr10:true }),
    }}
}
pub fn schema() -> SettingsSchema {
    serde_json::from_str(r#"{"sections":[{"title":"Stream","fields":[
        {"key":"height","label":"Resolution","type":"choice","options":[{"id":"720","label":"720p"},{"id":"1080","label":"1080p"},{"id":"1440","label":"1440p"},{"id":"2160","label":"4K"}],"default":"2160"},
        {"key":"fps","label":"Frame rate","type":"choice","options":[{"id":"30","label":"30 fps"},{"id":"60","label":"60 fps"},{"id":"120","label":"120 fps"}],"default":"120"},
        {"key":"codec","label":"Video codec","type":"choice","options":[{"id":"auto","label":"Automatic"},{"id":"h264","label":"H.264"},{"id":"h265","label":"HEVC"},{"id":"av1","label":"AV1"}],"default":"auto"},
        {"key":"hdr","label":"HDR10","type":"toggle","default":true,"help":"HEVC on an HDR-capable display."},
        {"key":"bitrate","label":"Bitrate","type":"number","min":5,"max":200,"step":5,"unit":"Mb/s","default":50},
        {"key":"pacing","label":"Presentation","type":"choice","options":[{"id":"latency","label":"Lowest latency"},{"id":"smooth","label":"Smoothness (two frames)"}],"default":"latency"},
        {"key":"rescan","label":"Find PCs again","type":"action"}
    ]}]}"#).expect("static schema")
}
pub fn text<'a>(v: &'a Values, key: &str) -> &'a str {
    match &v[key] { Value::Text(s) => s, _ => unreachable!("resolved schema") }
}
pub fn params(display: &Display, values: &Values, host: &str, port: u16) -> Result<ConnectParams> {
    let values = schema().resolve(values);
    let height = text(&values,"height").parse::<u32>()?.min(display.height);
    let width = (height * 16 / 9).min(display.width);
    let fps = text(&values,"fps").parse::<u32>()?.min(display.refresh_hz);
    ensure!(width >= 640 && height >= 360 && fps > 0, "The display has no usable stream mode.");
    let bit = |c| match c { Codec::H264=>quic::CODEC_H264, Codec::H265=>quic::CODEC_HEVC, Codec::Av1=>quic::CODEC_AV1, _=>0 };
    let offered = display.codecs.iter().copied().fold(0, |b,c| b | bit(c));
    let preferred = match text(&values,"codec") { "h264"=>quic::CODEC_H264,"h265"=>quic::CODEC_HEVC,"av1"=>quic::CODEC_AV1,_=>0 };
    ensure!(offered != 0 && (preferred == 0 || offered & preferred != 0), "The display cannot decode the selected codec.");
    let hdr = display.hdr10 && values["hdr"] == Value::Bool(true) && offered & quic::CODEC_HEVC != 0 && (preferred == 0 || preferred == quic::CODEC_HEVC);
    let mut p = ConnectParams::new(host,port,Mode {width,height,refresh_hz:fps},Duration::from_secs(20));
    p.video_codecs = if hdr { quic::CODEC_HEVC } else { offered };
    p.preferred_codec = preferred;
    p.video_caps = quic::VIDEO_CAP_CHACHA20 | if hdr { quic::VIDEO_CAP_10BIT | quic::VIDEO_CAP_HDR } else {0};
    p.bitrate_kbps = match values["bitrate"] { Value::Int(n) => n as u32 * 1000, _=>unreachable!() };
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
}
