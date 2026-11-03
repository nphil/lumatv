//! Sound (`settings/PageSound.qml`, `PageEqualizer.qml`, `PageHdmiAudio.qml`, `PageUsbSound.qml`, `SoundOut.js`): LG's Sound menu written to the
//! TV's own settings service. The sound goes to the Denon over eARC and webOS's own volume is a counter that drifts, so the volume row is
//! the receiver's own level, read from the helper's `receiver` topic, and the TV's sound modes are dimmed while the receiver plays. The
//! receiver's own controls are one page deeper (`receiver.rs`). Keys and values are the ones LG's own Settings app uses: its Cinema is
//! `movie` and its Clear Voice Pro `news`. LG leaves a row out, or dims it, by the sound out in force (its own `i6`: the outputs that
//! include the TV's speakers or the eARC receiver); a dimmed row says what it needs.

use super::util::{go, waits};
use crate::host::sio::SERVICE;
use crate::settings::ctx::{Ctx, Spec, ask};
use crate::settings::names::{self, Table};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

pub fn page() -> Page {
    Page::new("sound", "Sound", "volume-2").rail().tabs(&["Output", "Mode", "Advanced", "Devices"]).reads(reads).rows(rows).summary(summary)
}

fn child(id: &'static str, title: &'static str, icon: &'static str) -> Page {
    Page::new(id, title, icon).parent("sound").back("Sound")
}

pub fn equalizer() -> Page {
    child("equalizer", "Equalizer", "sliders-vertical").reads(eq_reads).rows(eq_rows)
}

pub fn hdmi_audio() -> Page {
    child("hdmiaudio", "HDMI input audio", "cable").reads(hdmi_reads).rows(hdmi_rows)
}

pub fn usb() -> Page {
    child("usbsound", "USB Device", "usb").reads(usb_reads).rows(usb_rows)
}

// ------------------------------------------------------------------------------------------------------------------ the sound out

/// The ways out as the Sound page names them (the chooser's own table is LG's).
const OUTPUT_NAMES: Table = &[
    ("external_arc", "Receiver (eARC)"),
    ("external_optical", "Optical out"),
    ("tv_speaker", "TV speakers"),
    ("headphone", "Headphones"),
    ("bt_soundbar", "Bluetooth speaker"),
    ("tv_external_speaker", "TV and external speakers"),
    ("tv_speaker_external_arc", "TV and receiver"),
    ("lineout", "Line out"),
    ("usb_speaker", "USB speaker"),
    ("wow_cast", "Wireless speaker"),
    ("wisa_speaker", "WiSA speakers"),
];

/// The ways out the chooser offers (the TV's list still decides which exist); Bluetooth is paired on its own page and a USB or wireless
/// speaker needs the device list LG's own screens read.
const OUTPUT_VALUES: [&str; 8] =
    ["tv_speaker", "external_arc", "external_optical", "lineout", "headphone", "tv_speaker_external_arc", "tv_external_speaker", "tv_speaker_headphone"];

/// The outputs that play on the TV's own speakers (`SoundOut.js`: LG's `QH`).
const WITH_TV: [&str; 7] = [
    "tv_speaker",
    "tv_external_speaker",
    "tv_speaker_external_arc",
    "tv_speaker_headphone",
    "tv_speaker_bluetooth",
    "tv_speaker_wow_cast",
    "tv_speaker_optical_arc",
];

/// The TV's speakers play (balance, installation type, AI acoustic tuning).
pub fn tv_speakers(out: &str) -> bool {
    WITH_TV.contains(&out)
}

/// The TV's speakers or the eARC receiver (Dolby Atmos, automatic volume).
pub fn tv_or_arc(out: &str) -> bool {
    tv_speakers(out) || out == "external_arc"
}

/// The Equalizer: the TV's speakers, but not with a Bluetooth device.
pub fn equalizer_out(out: &str) -> bool {
    tv_speakers(out) && out != "tv_speaker_bluetooth"
}

/// The sound goes to the receiver over HDMI(ARC), alone or with the TV's speakers.
pub fn to_receiver(out: &str) -> bool {
    out == "external_arc" || out == "tv_speaker_external_arc"
}

fn output(c: &Ctx) -> String {
    c.lg.opt_text("sound.soundOutput").unwrap_or_default()
}

fn mode(c: &Ctx) -> String {
    c.lg.opt_text("sound.soundMode").unwrap_or_default()
}

const MODE_NAMES: Table = &[
    ("aiSoundPlus", "AI Sound Pro"),
    ("aiSound", "AI Sound"),
    ("standard", "Standard"),
    ("movie", "Cinema"),
    ("news", "Clear voice"),
    ("sports", "Sports"),
    ("music", "Music"),
    ("game", "Game"),
];

/// LG's delay slider is one key per way out of the TV (its `fre` row in main.js).
fn delay_ds(out: &str) -> String {
    let key = match out {
        "external_arc" | "external_optical" | "wow_cast" => "avSyncSpdif",
        "wisa_speaker" => "avSyncWisa",
        "tv_speaker_bluetooth" => "avSyncBtSurround",
        _ => "avSyncSpeaker",
    };
    format!("sound.{key}")
}

/// LG's Reset: the keys of the sound mode in force.
fn reset_keys(mode: &str) -> Vec<&'static str> {
    match mode {
        "game" => vec!["audioBalance", "gameTVEqualizer", "gameTVEqualizer10band", "soundModeModified"],
        "standard" => vec!["audioBalance", "userEqualizerValues", "soundModeModified"],
        _ => vec!["audioBalance", "soundModeModified"],
    }
}

/// Whether the TV has the sound wizard (its mode list has Personalized): then Reset is for every mode but that one.
fn wizard(c: &Ctx) -> bool {
    c.lg.list("sound.soundMode").is_some_and(|l| l.iter().any(|e| e.visible && e.value == "personalized"))
}

fn can_reset(c: &Ctx) -> bool {
    c.lg.has("sound.soundMode") && c.lg.usable("sound.soundMode") && if wizard(c) { mode(c) != "personalized" } else { mode(c) == "standard" }
}

/// USB Device is LG's row only where the TV's config `profile.list` lists "usbSoundOutput".
fn usb_listed(c: &Ctx) -> bool {
    let d = c.docs.get("profile");
    d.get("returnValue").and_then(Value::as_bool) != Some(false)
        && d.pointer("/configs/profile.list").and_then(Value::as_array).is_some_and(|l| l.iter().any(|v| v == "usbSoundOutput"))
}

/// The receiver's reading, when the helper says it answers.
fn receiver<'a>(c: &'a Ctx<'_>) -> Option<&'a Value> {
    c.helper("receiver").filter(|r| r.get("reachable").and_then(Value::as_bool) == Some(true) && r.get("fresh").and_then(Value::as_bool) == Some(true))
}

fn summary(c: &Ctx) -> String {
    if !c.lg.has("sound.soundOutput") {
        String::new()
    } else if to_receiver(&output(c)) {
        "Receiver".into()
    } else {
        names::name_or(OUTPUT_NAMES, &output(c), "")
    }
}

fn reads(_: &Ctx) -> Vec<Read> {
    // the keys that do not depend on the sound out first, then the ones that do: the last answer's dimension (the sound out and the
    // input the mode belongs to) is what a write carries, so the mode and the output go last
    vec![
        Read::lg(
            "sound",
            &[
                "avSyncSpdif",
                "avSyncSpeaker",
                "avSyncWisa",
                "avSyncBtSurround",
                "avSyncBypassInput",
                "autoVolume",
                "soundOutputDigital",
                "supportATMOS",
                "userEqualizer",
                "userEqualizerValues",
                "tvSetupConfiguration",
                "magicSpaceSound",
                "applyToAllInputSSM",
            ],
        ),
        Read::lg(
            "sound",
            &[
                "inputAudioFormatHDMI1",
                "inputAudioFormatHDMI2",
                "inputAudioFormatHDMI3",
                "inputAudioFormatHDMI4",
                "inputAudioFormatDP1",
                "inputAudioFormatDP2",
                "inputAudioFormatUSBC1",
                "inputAudioFormatUSBC2",
                "lgSoundSync",
                "soundAlive",
                "eArcSupport",
                "digitalAudioPriority",
            ],
        ),
        Read::lg("general", &["tvOnSound"]),
        Read::lg("sound", &["audioBalance", "soundOutput", "soundMode"]),
        Read::call("profile", "luna://com.webos.service.config/getConfigs", json!({"configNames": ["profile.list"]})),
    ]
}

fn rows(c: &Ctx) -> Vec<Row> {
    let out = output(c);
    let md = mode(c);
    let to_rx = to_receiver(&out);
    let tv = tv_speakers(&out);
    let tv_arc = tv_or_arc(&out);
    let rc = receiver(c);
    let mut rows = Vec::new();

    // ---- Output
    rows.push(
        c.choice(
            Spec::new("sound.soundOutput")
                .tab(0)
                .icon("speaker")
                .label("Sound out")
                .names(names::SOUND_OUTPUT)
                .values(&OUTPUT_VALUES)
                .help("Where the TV's sound goes. Bluetooth speakers are paired on their own page."),
        ),
    );
    rows.push(
        Row::link("receiver")
            .tab(0)
            .icon("boom-box")
            .label("Receiver")
            .help("The receiver's own volume, input and sound modes.")
            .visible(to_rx)
            .does(go("receiver")),
    );
    let level = rc.and_then(|r| r.get("volumeAbs")).and_then(Value::as_f64);
    let mut volume = Row::slider("rcvolume", 0.0, 98.0, 0.5).tab(0).icon("volume-2").label("Receiver volume");
    volume.info = true;
    volume.detail = if rc.is_some() { String::new() } else { "Receiver not answering".into() };
    volume.num = level.unwrap_or(0.0);
    volume.value_text = Some(level.map_or("\u{2013}".into(), |v| format!("{v}")));
    rows.push(volume);
    let mut shown_mode = Row::info("soundmode").tab(0).icon("audio-waveform").label("Sound mode").value(&names::name_or(MODE_NAMES, &md, ""));
    shown_mode.detail = if to_rx { "Not used with the receiver".into() } else { String::new() };
    shown_mode.enabled = !to_rx;
    rows.push(shown_mode);
    rows.push(delay_row(c, &out));
    rows.push(
        c.toggle(
            Spec::new("sound.avSyncBypassInput")
                .tab(0)
                .icon("timer")
                .label("Skip the sound delay")
                .help("Skips the delay above, for a source that keeps picture and sound in step itself.")
                .lg_shown(),
        ),
    );
    rows.push(
        c.seg(
            Spec::new("sound.soundOutputDigital")
                .tab(0)
                .icon("cable")
                .label("Digital sound output")
                .help("Pass through hands the original soundtrack to the receiver.")
                .options(&["Auto", "PCM", "Pass through"])
                .values(&["auto", "pcm", "passThrough"]),
        ),
    );
    let mut too = Row::toggle("tvtoo")
        .tab(0)
        .icon("tv")
        .label("Also play on TV speakers")
        .help("Also plays through the TV's own speakers while sound goes to the receiver.")
        .on(out == "tv_speaker_external_arc")
        .admin();
    too.enabled = c.lg.usable("sound.soundOutput") && to_rx;
    too.detail = if to_rx { String::new() } else { "Needs the receiver".into() };
    rows.push(too.act(|a| match a {
        Arg::Toggle(v) => vec![Op::Lg { ds: "sound.soundOutput".into(), value: json!(if v { "tv_speaker_external_arc" } else { "external_arc" }), app: None }],
        _ => Vec::new(),
    }));
    rows.push(Row::link("btaudio").tab(0).icon("headphones").label("Bluetooth Speakers").does(go("btaudio")));
    rows.push(
        Row::link("usbsound")
            .tab(0)
            .icon("usb")
            .label("USB Device")
            .value(if out == "usb_speaker" { "In use" } else { "" })
            .visible(usb_listed(c))
            .does(go("usbsound")),
    );

    // ---- Mode
    let mut cards =
        c.cards(Spec::new("sound.soundMode").tab(1).cols(3).values(&["aiSoundPlus", "standard", "movie", "news", "sports", "music", "game"]).cards(&[
            ("AI Sound Pro", "sparkles"),
            ("Standard", "tv"),
            ("Cinema", "film"),
            ("Clear voice", "mic"),
            ("Sports", "activity"),
            ("Music", "music"),
            ("Game", "gamepad-2"),
        ]));
    cards.enabled = c.lg.usable("sound.soundMode") && !to_rx;
    rows.push(cards);
    rows.push(Row::note("Sound is on the receiver.", 1).visible(to_rx));

    // ---- Advanced
    rows.push(waits(
        c.toggle(
            Spec::new("sound.supportATMOS")
                .tab(2)
                .icon("sparkles")
                .label("Dolby Atmos")
                .help("Dolby Atmos soundtracks play with height and surround effects.")
                .lg_shown(),
        ),
        tv_arc,
        "Needs the TV or receiver",
    ));
    rows.push(waits(
        c.slider(
            Spec::new("sound.audioBalance")
                .tab(2)
                .range(-10.0, 10.0, 1.0)
                .icon("sliders-horizontal")
                .label("Balance")
                .help("Left and right, for the TV's own speakers."),
        ),
        tv,
        TV_ONLY,
    ));
    let mut eq = Row::link("equalizer")
        .tab(2)
        .icon("sliders-vertical")
        .label("Equalizer")
        .help("Five bands for the TV's own speakers, in Standard mode.")
        .admin()
        .does(go("equalizer"));
    eq.enabled = c.lg.usable("sound.userEqualizer");
    rows.push(waits(eq, equalizer_out(&out) && md == "standard", "Standard mode and TV speakers"));
    rows.push(waits(
        c.toggle(
            Spec::new("sound.autoVolume")
                .tab(2)
                .icon("volume-1")
                .label("Automatic volume")
                .help("Evens out loud adverts and quiet dialogue on the TV's speakers."),
        ),
        tv_arc,
        "Needs the TV or receiver",
    ));
    rows.push(waits(
        c.seg_tv(
            Spec::new("sound.tvSetupConfiguration")
                .tab(2)
                .icon("monitor-speaker")
                .label("Installation Type")
                .names(names::TV_SETUP)
                .help("Tunes the TV's speakers for a stand or a wall mount."),
        ),
        tv && out != "external_arc",
        TV_ONLY,
    ));
    rows.push(waits(
        c.choice(
            Spec::new("sound.magicSpaceSound")
                .tab(2)
                .icon("waves")
                .label("AI Acoustic Tuning Options")
                .names(names::MAGIC_SOUND)
                .help("Shapes the TV's sound to the room, after a tuning with the Magic Remote."),
        ),
        tv,
        TV_ONLY,
    ));
    rows.push(
        c.choice(
            Spec::new("general.tvOnSound")
                .tab(2)
                .icon("bell")
                .label("TV Power Sound Volume")
                .names(names::LOW_TO_HIGH)
                .help("The sound the TV makes when it turns on or off."),
        ),
    );
    let mut apply =
        Row::action("applyall", "Apply").tab(2).icon("list-checks").label("Apply to All Inputs").help("Gives every input this sound mode's settings.").admin();
    apply.enabled = c.lg.usable("sound.applyToAllInputSSM");
    rows.push(apply.does(ask(
        "list-checks",
        "Apply to all inputs?",
        "Every input gets this sound mode's settings.",
        "Apply",
        "Cancel",
        false,
        vec![Op::Lg { ds: "sound.applyToAllInputSSM".into(), value: json!("sound"), app: None }],
    )));
    let can = can_reset(c);
    let mut reset =
        Row::action("reset", "Reset").tab(2).icon("rotate-ccw").label("Reset").help("Puts this sound mode's settings back to LG's.").danger().admin();
    reset.enabled = can;
    reset.detail = if can || !c.lg.has("sound.soundMode") {
        String::new()
    } else if wizard(c) {
        "Not for Personalized".into()
    } else {
        "Standard mode only".into()
    };
    rows.push(reset.does(ask(
        "rotate-ccw",
        "Reset this sound mode?",
        "Its settings go back to LG's.",
        "Reset",
        "Cancel",
        true,
        vec![Op::Act { uri: format!("{SERVICE}/resetSystemSettings"), payload: json!({"category": "sound", "keys": reset_keys(&md)}) }, Op::Refresh],
    )));

    // ---- Devices
    let hdmi_listed = (1..=4).any(|i| c.lg.shown(&format!("sound.inputAudioFormatHDMI{i}")));
    rows.push(
        Row::link("hdmiaudio")
            .tab(3)
            .icon("cable")
            .label("Select HDMI Input Audio Format")
            .help("Bitstream or PCM, for each HDMI input.")
            .visible(hdmi_listed)
            .does(go("hdmiaudio")),
    );
    rows.push(waits(
        c.toggle(
            Spec::new("sound.lgSoundSync")
                .tab(3)
                .icon("cable")
                .label("LG Sound Sync")
                .help("Lets the remote set the volume of a sound bar on the optical out.")
                .lg_shown(),
        ),
        out == "external_optical",
        "Needs the optical out",
    ));
    rows.push(waits(
        c.toggle(
            Spec::new("sound.soundAlive")
                .tab(3)
                .icon("boom-box")
                .label("TV Sound Mode Share")
                .help("Gives a LG sound bar on HDMI the sound mode of the TV's speakers.")
                .lg_shown(),
        ),
        to_rx,
        "Needs HDMI(ARC)",
    ));
    rows.push(
        c.toggle(
            Spec::new("sound.eArcSupport")
                .tab(3)
                .icon("cable")
                .label("eARC Support")
                .help("For a receiver that supports eARC: a richer sound from the HDMI(ARC) connection.")
                .lg_shown(),
        ),
    );
    rows.push(
        c.choice(
            Spec::new("sound.digitalAudioPriority")
                .tab(3)
                .icon("audio-lines")
                .label("DTV Audio Setting")
                .names(names::DTV_AUDIO)
                .help("The digital audio format a broadcast is played in, when it offers more than one."),
        ),
    );
    rows
}

const TV_ONLY: &str = "Needs the TV speakers";

/// LG's "Match screen and sound": a slider whose limits are in the key's description; `sound.avSync` itself is a switch.
fn delay_row(c: &Ctx, out: &str) -> Row {
    let ds = delay_ds(out);
    let range = c.lg.range(&ds);
    let bypass = c.lg.is("sound.avSyncBypassInput", "on");
    let (min, max) = range.map_or((0.0, 1.0), |r| (r.min, r.max));
    let mut r = Row::slider("avsync", min, max, range.and_then(|r| r.interval).unwrap_or(1.0))
        .tab(0)
        .icon("timer")
        .label("Match screen and sound")
        .help("Delays the sound if lips move before the words.")
        .admin();
    r.visible = !c.lg.missing(&ds);
    r.enabled = range.is_some() && c.lg.usable(&ds) && !bypass;
    r.detail = if range.is_none() || !c.lg.has(&ds) {
        "TV not answering".into()
    } else if !c.lg.usable(&ds) {
        "Not available now".into()
    } else if bypass {
        "Delay bypass is on".into()
    } else {
        String::new()
    };
    r.num = c.lg.num(&ds, 0.0);
    r.value_text = Some(if c.lg.has(&ds) { format!("{} ms", r.num.round() as i64) } else { "\u{2013}".into() });
    r.act(move |a| match a {
        Arg::Value(v) => vec![Op::Lg { ds: ds.clone(), value: json!(v), app: None }],
        _ => Vec::new(),
    })
}

// ------------------------------------------------------------------------------------------------------------------- Equalizer

const BANDS: [&str; 5] = ["100 Hz", "300 Hz", "1 kHz", "3 kHz", "10 kHz"];

fn eq_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::lg("sound", &["userEqualizer", "userEqualizerValues", "soundOutput", "soundMode"])]
}

/// The five levels LG keeps in one key; a band the TV did not say reads 0.
fn levels(c: &Ctx) -> [f64; 5] {
    let mut out = [0.0; 5];
    if let Some(list) = c.lg.get("sound.userEqualizerValues").and_then(Value::as_array) {
        for (i, v) in list.iter().take(5).enumerate() {
            out[i] = crate::host::sio::js::float(v).filter(|n| n.is_finite()).unwrap_or(0.0);
        }
    }
    out
}

/// A number as LG's list writes it: 3, not 3.0.
fn whole(v: f64) -> Value {
    if v.fract() == 0.0 { json!(v as i64) } else { json!(v) }
}

fn signed(n: i64) -> String {
    if n > 0 { format!("+{n}") } else { n.to_string() }
}

fn eq_rows(c: &Ctx) -> Vec<Row> {
    let allowed = mode(c) == "standard" && equalizer_out(&output(c)) && c.lg.usable("sound.userEqualizer") && c.lg.has("sound.userEqualizerValues");
    let lim = c.lg.range("sound.userEqualizerValues");
    let (min, max, step) = lim.map_or((-10.0, 10.0, 1.0), |l| (l.min, l.max, l.interval.unwrap_or(1.0)));
    let now = levels(c);
    (0..5)
        .map(|i| {
            let mut r = Row::slider(&format!("band{i}"), min, max, step).tab(0).icon("sliders-vertical").label(BANDS[i]).admin();
            if i == 0 {
                r.help = "Lower bands are the bass, higher ones the treble. 0 leaves a band as it is.".into();
            }
            r.enabled = allowed;
            r.detail = if !c.lg.has("sound.userEqualizerValues") {
                "TV not answering".into()
            } else if allowed {
                String::new()
            } else {
                "Standard mode and TV speakers".into()
            };
            r.num = now[i];
            r.value_text = Some(if c.lg.has("sound.userEqualizerValues") { signed(now[i].round() as i64) } else { "\u{2013}".into() });
            r.act(move |a| match a {
                Arg::Value(v) => {
                    let mut all = now;
                    all[i] = v;
                    let mut m = serde_json::Map::new();
                    m.insert("userEqualizerValues".into(), Value::Array(all.iter().map(|b| whole(*b)).collect()));
                    m.insert("soundModeModified".into(), json!("true"));
                    vec![Op::LgMany { category: "sound".into(), settings: m, app: None }]
                }
                _ => Vec::new(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------- HDMI input audio

const PORTS: [(&str, &str, &str); 8] = [
    ("HDMI1", "HDMI 1", "cable"),
    ("HDMI2", "HDMI 2", "cable"),
    ("HDMI3", "HDMI 3", "cable"),
    ("HDMI4", "HDMI 4", "cable"),
    ("DP1", "DisplayPort 1", "monitor"),
    ("DP2", "DisplayPort 2", "monitor"),
    ("USBC1", "USB-C 1", "usb"),
    ("USBC2", "USB-C 2", "usb"),
];

fn hdmi_reads(_: &Ctx) -> Vec<Read> {
    let keys: Vec<&'static str> = vec![
        "inputAudioFormatHDMI1",
        "inputAudioFormatHDMI2",
        "inputAudioFormatHDMI3",
        "inputAudioFormatHDMI4",
        "inputAudioFormatDP1",
        "inputAudioFormatDP2",
        "inputAudioFormatUSBC1",
        "inputAudioFormatUSBC2",
    ];
    vec![Read::lg("sound", &keys)]
}

/// For each input, the format the TV asks the device to send: Bitstream (the original soundtrack, for a receiver) or PCM. The TV's
/// description says which inputs exist and which may be changed now. A change makes the device renegotiate.
fn hdmi_rows(c: &Ctx) -> Vec<Row> {
    PORTS
        .iter()
        .enumerate()
        .map(|(i, (key, label, icon))| {
            let mut spec = Spec::new(&format!("sound.inputAudioFormat{key}")).icon(icon).label(label).names(names::HDMI_AUDIO);
            if i == 0 {
                spec = spec.help("Bitstream sends the original soundtrack to a receiver. Changing it may blank the picture or sound for a moment.");
            }
            c.seg_tv(spec)
        })
        .collect()
}

// ------------------------------------------------------------------------------------------------------------------- USB Device

const CARDS: &str = "luna://com.webos.service.audio/tv/getSoundCardsState";

fn usb_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::call("cards", CARDS, json!({}))]
}

/// The sound cards the TV sees on its USB ports: (number, name, playing).
fn cards(c: &Ctx) -> Vec<(i64, String, bool)> {
    let d = c.docs.get("cards");
    if d.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return Vec::new();
    }
    d.get("soundCards")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .map(|e| {
                    (
                        e.get("number").and_then(Value::as_i64).unwrap_or(0),
                        e.get("name").map(crate::host::sio::js::string).unwrap_or_default(),
                        e.get("playbackState").and_then(Value::as_str) == Some("connected"),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn usb_rows(c: &Ctx) -> Vec<Row> {
    let mut rows = vec![Row::head("Device List", 0)];
    let list = cards(c);
    if list.is_empty() {
        rows.push(Row::note("No USB sound device is connected.", 0));
    }
    for (number, name, playing) in list {
        rows.push(Row::link(&format!("card:{number}")).icon("usb").label(&name).value(if playing { "In use" } else { "" }).chevron(false).admin().does(vec![
            Op::Act { uri: "luna://com.webos.service.audio/setSoundOutput".into(), payload: json!({"soundOutput": "usb_speaker", "cardNumber": number}) },
            Op::RefreshIn(600),
        ]));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

    fn tv(out: &str, mode: &str) -> World {
        World::new(lg_with(
            &[(
                "sound",
                json!({"settings": {"soundOutput": out, "soundMode": mode, "audioBalance": 2, "autoVolume": "on", "supportATMOS": "on", "avSyncSpeaker": 40, "avSyncSpdif": 90, "avSyncBypassInput": "off", "userEqualizer": "on", "userEqualizerValues": [1, -2, 0, 3, 0], "tvSetupConfiguration": "stand", "soundOutputDigital": "pcm"}}),
            )],
            &[(
                "sound",
                json!({"results": [
                {"category": "sound", "key": "avSyncSpeaker", "values": {"range": {"min": 0, "max": 100, "interval": 5}}},
                {"category": "sound", "key": "avSyncSpdif", "values": {"range": {"min": 0, "max": 250, "interval": 10}}},
                {"category": "sound", "key": "userEqualizerValues", "values": {"range": {"min": -10, "max": 10}}},
                {"category": "sound", "key": "soundMode", "values": {"arrayExt": [{"value": "standard"}, {"value": "movie"}, {"value": "game"}]}}]}),
            )],
        ))
    }

    #[test]
    fn the_sound_out_decides_which_rows_wait_and_what_they_say() {
        let w = tv("tv_speaker", "standard");
        let r = rows(&w.ctx());
        assert_eq!(summary(&w.ctx()), "TV speakers");
        assert!(by_id(&r, "sound.audioBalance").enabled && by_id(&r, "equalizer").enabled);
        assert!(!by_id(&r, "receiver").visible && by_id(&r, "tvtoo").detail == "Needs the receiver" && !by_id(&r, "tvtoo").enabled);
        assert!(by_id(&r, "sound.soundMode").enabled && !by_id(&r, "note:Sound is on the receiver.").visible);
        let w = tv("external_arc", "standard");
        let r = rows(&w.ctx());
        assert_eq!(summary(&w.ctx()), "Receiver");
        assert!(by_id(&r, "receiver").visible && by_id(&r, "tvtoo").enabled && !by_id(&r, "tvtoo").on);
        let balance = by_id(&r, "sound.audioBalance");
        assert!(!balance.enabled && balance.detail == "Needs the TV speakers");
        assert!(by_id(&r, "sound.autoVolume").enabled, "the receiver counts for automatic volume");
        assert!(!by_id(&r, "sound.soundMode").enabled && by_id(&r, "note:Sound is on the receiver.").visible);
        assert_eq!(by_id(&r, "soundmode").detail, "Not used with the receiver");
        let ops = (by_id(&r, "tvtoo").act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::Lg { ds, value, .. }] if ds == "sound.soundOutput" && value == "tv_speaker_external_arc"));
    }

    #[test]
    fn the_delay_slider_takes_the_key_and_the_limits_of_the_way_out() {
        let w = tv("tv_speaker", "standard");
        let d = delay_row(&w.ctx(), "tv_speaker");
        assert_eq!((d.min, d.max, d.step, d.num), (0.0, 100.0, 5.0, 40.0));
        assert_eq!(d.value_text.as_deref(), Some("40 ms"));
        let d = delay_row(&w.ctx(), "external_optical");
        assert_eq!((d.max, d.num), (250.0, 90.0));
        assert!(matches!(&(d.act.as_ref().unwrap())(Arg::Value(50.0))[..], [Op::Lg { ds, .. }] if ds == "sound.avSyncSpdif"));
        let w2 = World::new(lg_with(
            &[("sound", json!({"settings": {"avSyncBypassInput": "on", "avSyncSpeaker": 10}}))],
            &[("sound", json!({"results": [{"category": "sound", "key": "avSyncSpeaker", "values": {"range": {"min": 0, "max": 100}}}]}))],
        ));
        let d = delay_row(&w2.ctx(), "tv_speaker");
        assert!(!d.enabled && d.detail == "Delay bypass is on");
        let none = World::new(lg_with(&[("sound", json!({"settings": {"avSyncSpeaker": 10}}))], &[]));
        assert_eq!(delay_row(&none.ctx(), "tv_speaker").detail, "TV not answering", "no limits, no slider");
    }

    #[test]
    fn reset_is_standard_modes_or_any_but_personalized_and_names_the_keys_of_the_mode() {
        let w = tv("tv_speaker", "standard");
        let r = rows(&w.ctx());
        assert!(by_id(&r, "reset").enabled);
        let ops = (by_id(&r, "reset").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!("{ops:?}") };
        assert!(
            matches!(&a.answers[1].then[..], [Op::Act { payload, .. }, Op::Refresh] if payload["keys"] == json!(["audioBalance", "userEqualizerValues", "soundModeModified"]))
        );
        let w = tv("tv_speaker", "movie");
        let r = rows(&w.ctx());
        assert!(!by_id(&r, "reset").enabled && by_id(&r, "reset").detail == "Standard mode only");
        assert_eq!(reset_keys("game")[1], "gameTVEqualizer");
        let mut w = tv("tv_speaker", "movie");
        w.lg.learn_desc(
            "sound",
            &json!({"results": [{"category": "sound", "key": "soundMode", "values": {"arrayExt": [{"value": "standard"}, {"value": "personalized"}]}}]}),
        );
        assert!(by_id(&rows(&w.ctx()), "reset").enabled, "with the wizard, any mode but Personalized");
        w.lg.learn("sound", &json!({"settings": {"soundMode": "personalized"}}));
        assert_eq!(by_id(&rows(&w.ctx()), "reset").detail, "Not for Personalized");
    }

    #[test]
    fn the_receiver_volume_is_the_receivers_own_and_only_while_it_answers() {
        let mut w = tv("external_arc", "standard");
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "rcvolume").detail, "Receiver not answering");
        assert_eq!(by_id(&r, "rcvolume").value_text.as_deref(), Some("\u{2013}"));
        w.topics.set_helper("receiver", json!({"reachable": true, "fresh": true, "volumeAbs": 41.5}));
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "rcvolume").value_text.as_deref(), Some("41.5"));
        assert!(by_id(&r, "rcvolume").info && by_id(&r, "rcvolume").detail.is_empty());
        w.topics.set_helper("receiver", json!({"reachable": true, "fresh": true, "volumeAbs": 42.0}));
        assert_eq!(by_id(&rows(&w.ctx()), "rcvolume").value_text.as_deref(), Some("42"));
        w.topics.set_helper("receiver", json!({"reachable": true, "fresh": false, "volumeAbs": 41.5}));
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "rcvolume").value_text.as_deref(), Some("\u{2013}"));
        assert_eq!(by_id(&r, "rcvolume").detail, "Receiver not answering");
    }

    #[test]
    fn usb_device_is_listed_only_where_the_config_says_so_and_in_use_when_it_plays() {
        let mut w = tv("usb_speaker", "standard");
        assert!(!by_id(&rows(&w.ctx()), "usbsound").visible);
        w.docs.set("profile", json!({"returnValue": true, "configs": {"profile.list": ["a", "usbSoundOutput"]}}));
        let r = rows(&w.ctx());
        assert!(by_id(&r, "usbsound").visible && by_id(&r, "usbsound").value == "In use");
    }

    #[test]
    fn the_equalizer_writes_the_five_levels_in_one_key_and_marks_the_mode_modified() {
        let w = tv("tv_speaker", "standard");
        let r = eq_rows(&w.ctx());
        assert_eq!(r.len(), 5);
        assert_eq!((by_id(&r, "band1").num, by_id(&r, "band1").value_text.as_deref()), (-2.0, Some("-2")));
        assert_eq!(by_id(&r, "band3").value_text.as_deref(), Some("+3"));
        let ops = (by_id(&r, "band2").act.as_ref().unwrap())(Arg::Value(4.0));
        let [Op::LgMany { category, settings, .. }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(category, "sound");
        assert_eq!(settings["userEqualizerValues"], json!([1, -2, 4, 3, 0]));
        assert_eq!(settings["soundModeModified"], json!("true"));
        let w = tv("tv_speaker", "movie");
        assert!(!eq_rows(&w.ctx())[0].enabled && eq_rows(&w.ctx())[0].detail == "Standard mode and TV speakers");
        let w = tv("tv_speaker_bluetooth", "standard");
        assert!(!eq_rows(&w.ctx())[0].enabled, "not with a Bluetooth device");
    }

    #[test]
    fn hdmi_input_audio_lists_the_ports_the_tv_answers_in_lgs_names() {
        let list = json!({"arrayExt": [{"value": "bitstream"}, {"value": "pcm"}]});
        let w = World::new(lg_with(
            &[("sound", json!({"settings": {"inputAudioFormatHDMI1": "bitstream", "inputAudioFormatHDMI2": "pcm"}}))],
            &[(
                "sound",
                json!({"results": [{"category": "sound", "key": "inputAudioFormatHDMI1", "values": list}, {"category": "sound", "key": "inputAudioFormatHDMI2", "values": list}]}),
            )],
        ));
        let r = hdmi_rows(&w.ctx());
        assert_eq!(r.len(), 8);
        assert_eq!((by_id(&r, "sound.inputAudioFormatHDMI1").index, by_id(&r, "sound.inputAudioFormatHDMI2").index), (Some(0), Some(1)));
        assert_eq!(by_id(&r, "sound.inputAudioFormatHDMI1").options, vec!["Bitstream", "PCM"]);
        assert!(by_id(&r, "sound.inputAudioFormatHDMI1").detail.is_empty() && by_id(&r, "sound.inputAudioFormatHDMI3").detail == "TV not answering");
    }

    #[test]
    fn a_usb_sound_card_is_used_with_ok_and_the_list_is_read_again() {
        let mut w = tv("tv_speaker", "standard");
        assert!(matches!(&usb_rows(&w.ctx())[..], [Row { .. }, n] if n.kind == crate::settings::row::Kind::Note));
        w.docs.set("cards", json!({"returnValue": true, "soundCards": [{"number": 2, "name": "USB Audio", "playbackState": "connected"}, {"number": 3, "name": "Dongle", "playbackState": "idle"}]}));
        let r = usb_rows(&w.ctx());
        assert_eq!((by_id(&r, "card:2").value.as_str(), by_id(&r, "card:3").value.as_str()), ("In use", ""));
        let ops = (by_id(&r, "card:3").act.as_ref().unwrap())(Arg::Press);
        assert!(
            matches!(&ops[..], [Op::Act { uri, payload }, Op::RefreshIn(600)] if uri.ends_with("/setSoundOutput") && payload == &json!({"soundOutput": "usb_speaker", "cardNumber": 3}))
        );
    }
}
