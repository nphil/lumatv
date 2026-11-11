//! Inputs & devices (`settings/PageInputs.qml`, mocks `30-settings-inputs`) and its Channels page (`PageChannels.qml`). Each input shows the
//! name its device gave itself over HDMI-CEC; the owner decides whether it is on Home's Inputs row and in the picker: Auto (only while
//! something is connected), Always, or Hidden. The choice is kept in theme.json (`inputs.<port>`), which the picker and Home read. The HDMI
//! tab holds the TV's own HDMI switches. Channels is the tuner's keys of the settings service: Channel Scrolling Options, System Color
//! (analog TV only: LG shows it while Live TV is in front) and the two Interactive Service switches (LG's own switch writes `onByUser` and
//! `offByUser`, and reads `on` and `onByUser` as on); tuning, the channel manager and the locks are scans and lists the tuner's service
//! answers, so they stay in LG's channel screen, the last row.

use super::util::{atsc3, atsc3_read, go, inputs, inputs_read};
use crate::settings::ctx::{Ctx, Spec, ask, lg_launch, toast};
use crate::settings::names::Table;
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row, patch_of};
use serde_json::{Value, json};

pub fn page() -> Page {
    Page::new("inputs", "Inputs & devices", "cable").rail().tabs(&["Inputs", "HDMI", "Devices"]).reads(reads).rows(rows).summary(summary).opened(opened)
}

pub fn channels() -> Page {
    Page::new("channels", "Channels", "antenna").parent("inputs").back("Inputs & devices").reads(channel_reads).rows(channel_rows).opened(channel_opened).answered(channel_answered)
}

const PORTS: [&str; 4] = ["hdmi1", "hdmi2", "hdmi3", "hdmi4"];
const CHOICES: [&str; 3] = ["auto", "always", "hidden"];
const CHOICE_HELP: &str = "Auto shows it on Home and in the picker while something is connected. Always keeps it there. Hidden removes it.";

fn summary(c: &Ctx) -> String {
    inputs(c).current().map(|e| e.port.clone()).unwrap_or_default()
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![
        inputs_read(),
        Read::lg(
            "other",
            &["simplinkEnable", "simplinkAutoPowerOn", "qmsVrr", "uhdDeepColor", "444BypassHDMI1", "444BypassHDMI2", "444BypassHDMI3", "444BypassHDMI4", "enableDolbyVisionPC"],
        ),
        Read::lg("general", &["noSignalScreenSaver"]),
        Read::call("bt", "luna://com.webos.service.config/getConfigs", json!({"configNames": ["system.supportBluetoothFeatures"]})),
    ]
}

fn opened(_: &Ctx) -> Vec<Op> {
    vec![Op::Set("inputs.historyDone".into(), json!(false))]
}

/// The owner's choice for an input: auto, always or hidden.
fn choice(c: &Ctx, key: &str) -> Option<usize> {
    let v = c.theme_str(&format!("inputs.{key}")).unwrap_or("auto");
    CHOICES.iter().position(|x| *x == v)
}

/// A row of three: Auto, Always, Hidden for one input.
fn visibility(c: &Ctx, key: &str, icon: &str, label: &str, help: &str) -> Row {
    let key = key.to_string();
    Row::seg(&key, &["Auto", "Always", "Hidden"]).tab(0).icon(icon).label(label).help(help).index(choice(c, &key)).act(move |a| match a {
        Arg::Index(i) => CHOICES.get(i).map(|v| vec![Op::Patch(patch_of(&format!("inputs.{key}"), json!(v)))]).unwrap_or_default(),
        _ => Vec::new(),
    })
}

/// What the TV's config says it can do over Bluetooth: the remote app's own link ("remoteapp") and the TV as a speaker ("btsoundsink").
fn bt_feature(c: &Ctx, name: &str) -> bool {
    c.docs.get("bt").pointer("/configs/system.supportBluetoothFeatures").and_then(Value::as_array).is_some_and(|l| l.iter().any(|v| v == name))
}

/// Delete Connection History (LG: External Devices): the services that keep a phone's, a cast's or a remote app's pairing are told to forget
/// them, as LG's row does after its question. Two of the five calls are LG's only where the TV's config lists the feature.
fn forget(c: &Ctx) -> Vec<Op> {
    let act = |uri: &str| Op::Act { uri: uri.into(), payload: json!({}) };
    let mut ops = Vec::new();
    if bt_feature(c, "remoteapp") {
        ops.push(act("luna://com.webos.service.asc/resetWoBLE"));
    }
    ops.push(act("luna://com.webos.service.miracast/deletePairedDeviceList"));
    if bt_feature(c, "btsoundsink") {
        ops.push(act("luna://com.webos.service.btaudiosnk/removePairedDevices"));
    }
    ops.push(act("luna://com.webos.service.ics/blaster/deleteIPConnectionHistory"));
    ops.push(act("luna://com.webos.service.ftms/removeBlockedDeviceList"));
    ops.push(Op::Set("inputs.historyDone".into(), json!(true)));
    ops
}

fn rows(c: &Ctx) -> Vec<Row> {
    let list = inputs(c);
    let mut rows = vec![visibility(c, "livetv", "antenna", "Live TV", CHOICE_HELP)];
    for (n, port) in PORTS.iter().enumerate() {
        let entry = list.list().iter().find(|e| e.app_id == format!("com.webos.app.{port}"));
        let device = match entry {
            Some(e) if e.connected => {
                if e.label.is_empty() {
                    "Connected"
                } else {
                    e.label.as_str()
                }
            }
            _ => "Nothing connected",
        };
        rows.push(visibility(c, port, entry.map_or("unplug", |e| e.icon.as_str()), &format!("HDMI {} \u{b7} {device}", n + 1), ""));
    }
    rows.push(
        Row::toggle("banner")
            .tab(0)
            .icon("tv-minimal-play")
            .label("Input banner")
            .help("LumaTV draws its own banner over an input or Live TV, besides LG's. For when LG's is hidden.")
            .on(c.theme_flag("banners.own", false))
            .act(|a| match a {
                Arg::Toggle(v) => vec![Op::Patch(patch_of("banners.own", json!(v)))],
                _ => Vec::new(),
            }),
    );
    rows.push(
        Row::toggle("previews")
            .tab(0)
            .icon("image")
            .label("Show input previews")
            .help("The picker shows what is on each input: live for the one in front, the last picture for the others. Pictures stay on this TV, and turning this off deletes them.")
            .on(c.theme_flag("inputs.previews", true))
            .act(|a| match a {
                Arg::Toggle(v) => vec![Op::Patch(patch_of("inputs.previews", json!(v)))],
                _ => Vec::new(),
            }),
    );

    rows.push(
        c.toggle(
            Spec::new("other.simplinkEnable")
                .id("cec2")
                .tab(1)
                .icon("cable")
                .label("HDMI-CEC (SIMPLINK)")
                .help("Devices turn on, switch and name themselves over the cable."),
        ),
    );
    // LG's own CEC switch and what hangs off it: the TV and the devices turning on together
    let mut sync = c.toggle(
        Spec::new("other.simplinkAutoPowerOn")
            .tab(1)
            .icon("power")
            .label("Auto power sync")
            .help("The TV turns on with a device on HDMI-CEC, and the device turns on with the TV."),
    );
    if !c.lg.is("other.simplinkEnable", "on") && c.lg.usable("other.simplinkAutoPowerOn") {
        sync.enabled = false;
        sync.detail = "Turn HDMI-CEC on".into();
    }
    rows.push(sync);
    rows.push(c.toggle(
        Spec::new("general.noSignalScreenSaver")
            .tab(1)
            .icon("image")
            .label("No signal image")
            .help("A picture from LG instead of a plain screen while an input has no signal."),
    ));
    rows.push(
        c.toggle(
            Spec::new("other.qmsVrr")
                .tab(1)
                .icon("fast-forward")
                .label("Quick Media Switching")
                .help("Switches between a game and a video without a black screen. Only for the input in front."),
        ),
    );
    rows.push(
        c.seg(
            Spec::new("other.uhdDeepColor")
                .tab(1)
                .icon("palette")
                .label("HDMI Deep Color")
                .options(&["Off", "4K"])
                .values(&["off", "4k"])
                .help("Richer picture from a source that sends it, such as a PC or console. Only for the input in front."),
        ),
    );
    for n in 1..=4 {
        let mut spec = Spec::new(&format!("other.444BypassHDMI{n}")).tab(1).icon("monitor").label(&format!("4:4:4 Pass Through (HDMI {n})"));
        if n == 1 {
            spec = spec.help("Shows a PC or console picture with no processing, so text stays sharp. Only for the input in front.");
        }
        rows.push(c.toggle(spec));
    }
    rows.push(
        c.toggle(
            Spec::new("other.enableDolbyVisionPC")
                .tab(1)
                .icon("monitor-cog")
                .label("Dolby Vision PC")
                .help("Lets a PC send Dolby Vision. Turn HDR on in the PC's display settings."),
        ),
    );

    rows.push(
        Row::link("channels")
            .tab(2)
            .icon("antenna")
            .label("Channels")
            .help("Channel scrolling, system colour and the interactive service.")
            .does(go("channels")),
    );
    rows.push(
        Row::link("tuner")
            .tab(2)
            .icon("antenna")
            .label("Live TV Tuner")
            .help("Shares the antenna tuner with Plex, Jellyfin, VLC and other players on the home network.")
            .value(&super::tuner::summary(c))
            .does(go("tuner")),
    );
    rows.push(
        Row::link("guidedata")
            .tab(2)
            .icon("tv-minimal")
            .label("Guide data")
            .help("Fills the Live TV guide with online listings for a zip code, from Plex, Schedules Direct or free listings.")
            .value(&super::guide_data::summary(c))
            .does(go("guidedata")),
    );
    rows.push(
        Row::link("plex")
            .tab(2)
            .icon("tv-minimal-play")
            .label("Plex")
            .help("The TV's one sign-in to Plex, by a code scanned with a phone: Guide data's Plex listings use it.")
            .value(&super::plex::summary(c))
            .does(go("plex")),
    );
    rows.push(
        Row::link("hadevice")
            .tab(2)
            .icon("radio")
            .label("Home Assistant device")
            .help("Lets Home Assistant use the TV as a Bluetooth proxy, and see its power, app, input, picture mode and tuner as sensors.")
            .value(&super::ha_device::summary(c))
            .does(go("hadevice")),
    );
    rows.push(Row::link("bt").tab(2).icon("gamepad-2").label("Bluetooth Devices").does(go("bthid")));
    let done = c.local.flag("inputs.historyDone");
    let mut history = Row::action("history", if done { "Done" } else { "Delete" })
        .tab(2)
        .icon("history")
        .label("Delete Connection History")
        .help("Makes the TV forget the phones, casts and remote apps that connected to it wirelessly.")
        .danger();
    history = history.does(ask(
        "history",
        "Delete the connection history?",
        "All history of wireless connection of this TV is deleted.",
        "Delete",
        "Cancel",
        true,
        forget(c),
    ));
    rows.push(history);
    rows.push(lg_launch("game", "Game Optimizer", "", "com.webos.app.gameoptimizer").tab(2).help("VRR, ALLM and latency per input, in LG's own screen."));
    rows
}

// ------------------------------------------------------------------------------------------------------------------- Channels

fn channel_reads(_: &Ctx) -> Vec<Read> {
    vec![
        Read::lg("channel", &["autoscanChannelGroup", "analogSystemColor"]),
        Read::lg("option", &["interactive-service", "interactive-service-hdmi"]),
        Read::Get { name: "lgchannels", path: "/v1/lgchannels" },
        atsc3_read(),
        inputs_read(),
    ]
}

const SCROLLING: Table = &[("primary_channel", "Primary Channels"), ("all_channel", "All Channels")];
const SYSTEM_COLOR: Table = &[("multi", "Multi"), ("pal-m", "PAL-M"), ("pal-n", "PAL-N"), ("ntsc", "NTSC")];

fn channel_opened(_: &Ctx) -> Vec<Op> {
    vec![Op::Set("lgchannels.pending".into(), json!("refresh"))]
}

fn channel_answered(c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    match name {
        "lgchannels.apply" => {
            let mut ops = vec![Op::Set("lgchannels.pending".into(), json!("refresh")), Op::Refresh];
            if doc.get("ok").and_then(Value::as_bool) != Some(true) {
                let reason = doc.pointer("/error/message").and_then(Value::as_str).unwrap_or("the TV did not answer");
                ops.push(toast(&format!("Not changed: {reason}")));
            }
            ops
        }
        "lgchannels" if c.local.text("lgchannels.pending") == "refresh" => vec![Op::Set("lgchannels.pending".into(), Value::Null)],
        _ => Vec::new(),
    }
}

fn lg_channels(c: &Ctx) -> Row {
    let doc = c.docs.get("lgchannels");
    let enabled = doc.get("enabled").and_then(Value::as_bool);
    let state = doc.get("state").and_then(Value::as_str).unwrap_or("unavailable");
    let pending = !c.local.text("lgchannels.pending").is_empty();
    let detail = if pending {
        if c.local.text("lgchannels.pending") == "apply" { "Changing…" } else { "Checking…" }
    } else if matches!(state, "partial" | "unavailable") {
        doc.get("reason").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("Not available")
    } else {
        ""
    };
    Row::toggle("lgchannels")
        .tab(0)
        .icon("tv")
        .label("LG Channels")
        .help("Turns LG Channels and its background preloading on or off. Antenna TV is unchanged.")
        .detail(detail)
        .on(enabled == Some(true))
        .enabled(!pending && enabled.is_some() && state != "unavailable" && doc.get("ok").and_then(Value::as_bool) == Some(true))
        .admin()
        .act(|a| match a {
            Arg::Toggle(v) => vec![
                Op::Set("lgchannels.pending".into(), json!("apply")),
                Op::Send { name: Some("lgchannels.apply".into()), method: "POST", path: "/v1/lgchannels".into(), body: Some(json!({"enabled": v})) },
            ],
            _ => Vec::new(),
        })
}


/// LG shows the two Interactive Service switches only on a TV with an ATSC 3.0 tuner.
fn interactive(c: &Ctx, ds: &str, label: &str, help: &str) -> Row {
    let mut r = c.toggle(Spec::new(ds).tab(0).icon("globe").label(label).help(help).on_off("onByUser", "offByUser"));
    r.on = matches!(c.lg.opt_text(ds).as_deref(), Some("on" | "onByUser"));
    r.visible = atsc3(c) && c.lg.shown(ds);
    r
}

fn channel_rows(c: &Ctx) -> Vec<Row> {
    let live = c.topics.channel("foreground").and_then(|f| f.get("app")).and_then(Value::as_str) == Some("com.webos.app.livetv");
    let mut colour = c.choice(
        Spec::new("channel.analogSystemColor").tab(0).icon("palette").label("System Color").names(SYSTEM_COLOR).help("The colour system of an analog channel."),
    );
    colour.enabled = live && c.lg.usable("channel.analogSystemColor");
    colour.detail = if live { String::new() } else { "Live TV only".into() };
    vec![
        lg_channels(c),
        c.choice(
            Spec::new("channel.autoscanChannelGroup")
                .tab(0)
                .icon("list-checks")
                .label("Channel Scrolling Options")
                .names(SCROLLING)
                .help("Which channels the channel keys step through."),
        ),
        colour,
        interactive(c, "option.interactive-service", "Interactive Service", "Content suggestions and ads that follow what Live TV is showing."),
        interactive(c, "option.interactive-service-hdmi", "Interactive Service (HDMI)", "The same for a broadcast signal on an HDMI input."),
        lg_launch("autotuning", "Auto Tuning", "", "com.webos.app.channelsetting").tab(0).help("Searches for channels, in LG's channel setup. The channel list is replaced."),
        lg_launch("channeltuning", "Channel Tuning & Settings", "channel", "")
            .tab(0)
            .help("Auto and manual tuning, the channel list and locks, in LG's channel screen."),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

    fn world() -> World {
        let mut w = World::new(lg_with(
            &[("other", json!({"settings": {"simplinkEnable": "on", "qmsVrr": "off", "uhdDeepColor": "4k", "444BypassHDMI1": "on", "444BypassHDMI2": "off"}}))],
            &[],
        ));
        w.docs.set(
            "inputs",
            json!({"devices": [
                {"appId": "com.webos.app.hdmi1", "label": "Apple TV 4K", "connected": true},
                {"appId": "com.webos.app.hdmi2", "label": "Denon AVR", "connected": true},
                {"appId": "com.webos.app.hdmi3", "label": "", "connected": false}]}),
        );
        w.topics.set_channel("foreground", json!({"app": "com.webos.app.hdmi2"}));
        w
    }

    #[test]
    fn each_input_is_named_by_its_device_and_the_owners_choice_is_kept_in_the_theme() {
        let mut w = world();
        w.theme = json!({"inputs": {"hdmi2": "always", "hdmi3": "hidden"}});
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "hdmi1").label, "HDMI 1 \u{b7} Apple TV 4K");
        assert_eq!(
            (by_id(&r, "hdmi1").icon.as_str(), by_id(&r, "hdmi3").icon.as_str(), by_id(&r, "hdmi4").icon.as_str()),
            ("tv-minimal-play", "unplug", "unplug")
        );
        assert_eq!(by_id(&r, "hdmi3").label, "HDMI 3 \u{b7} Nothing connected");
        assert_eq!(by_id(&r, "hdmi4").label, "HDMI 4 \u{b7} Nothing connected", "a port the TV does not list");
        assert_eq!(
            (by_id(&r, "livetv").index, by_id(&r, "hdmi1").index, by_id(&r, "hdmi2").index, by_id(&r, "hdmi3").index),
            (Some(0), Some(0), Some(1), Some(2))
        );
        let ops = (by_id(&r, "hdmi1").act.as_ref().unwrap())(Arg::Index(2));
        assert!(matches!(&ops[..], [Op::Patch(p)] if p == &json!({"inputs": {"hdmi1": "hidden"}})));
        assert_eq!(summary(&w.ctx()), "HDMI 2", "the input in front");
        w.topics.set_channel("foreground", json!({"app": "com.webos.app.livetv"}));
        assert_eq!(summary(&w.ctx()), "");
    }

    #[test]
    fn the_banner_switch_patches_the_theme_and_the_hdmi_tab_is_lgs_own_switches() {
        let w = world();
        let r = rows(&w.ctx());
        assert!(!by_id(&r, "banner").on);
        let ops = (by_id(&r, "banner").act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::Patch(p)] if p == &json!({"banners": {"own": true}})));
        assert!(by_id(&r, "cec2").on && !by_id(&r, "other.qmsVrr").on);
        assert_eq!(by_id(&r, "other.uhdDeepColor").index, Some(1));
        assert_eq!(by_id(&r, "other.444BypassHDMI3").detail, "TV not answering");
        assert_eq!(r.iter().filter(|x| x.tab == 1).count(), 10, "LG's eight and the two that were in its hidden menu");
        let ops = (by_id(&r, "other.qmsVrr").act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::Lg { ds, value, .. }] if ds == "other.qmsVrr" && value == "on"));
    }

    #[test]
    fn the_previews_switch_is_on_until_the_owner_turns_it_off_and_says_what_on_means() {
        let mut w = world();
        let r = rows(&w.ctx());
        let row = by_id(&r, "previews");
        assert!(row.on, "on by default");
        assert_eq!((row.tab, row.label.as_str()), (0, "Show input previews"));
        assert!(row.help.contains("stay on this TV") && row.help.contains("deletes"), "{}", row.help);
        let off = (row.act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&off[..], [Op::Patch(p)] if p == &json!({"inputs": {"previews": false}})));
        w.theme = json!({"inputs": {"previews": false}});
        assert!(!by_id(&rows(&w.ctx()), "previews").on);
        w.theme = json!({"inputs": {"hdmi1": "always"}});
        assert!(by_id(&rows(&w.ctx()), "previews").on, "a theme that says nothing of it is on");
    }

    #[test]
    fn auto_power_sync_and_the_no_signal_image_are_lgs_own_keys_and_the_sync_waits_for_cec() {
        let mut w = World::new(lg_with(&[("other", json!({"settings": {"simplinkEnable": "off", "simplinkAutoPowerOn": "on"}})), ("general", json!({"settings": {"noSignalScreenSaver": "off"}}))], &[]));
        let r = rows(&w.ctx());
        let sync = by_id(&r, "other.simplinkAutoPowerOn");
        assert_eq!((sync.tab, sync.label.as_str(), sync.on, sync.enabled, sync.detail.as_str()), (1, "Auto power sync", true, false, "Turn HDMI-CEC on"));
        let image = by_id(&r, "general.noSignalScreenSaver");
        assert_eq!((image.tab, image.label.as_str(), image.on, image.enabled), (1, "No signal image", false, true));
        assert!(matches!(&(image.act.as_ref().unwrap())(Arg::Toggle(true))[..], [Op::Lg { ds, value, .. }] if ds == "general.noSignalScreenSaver" && value == "on"));
        // CEC on: the sync is the owner's to change, and writes LG's key
        w.lg = lg_with(&[("other", json!({"settings": {"simplinkEnable": "on", "simplinkAutoPowerOn": "on"}}))], &[]);
        let r = rows(&w.ctx());
        let sync = by_id(&r, "other.simplinkAutoPowerOn");
        assert!(sync.enabled && sync.detail.is_empty());
        assert!(matches!(&(sync.act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Lg { ds, value, .. }] if ds == "other.simplinkAutoPowerOn" && value == "off"));
        // reads: the page asks for both keys
        let reads = reads(&w.ctx());
        assert!(reads.iter().any(|x| matches!(x, Read::Lg { category: "other", keys, .. } if keys.contains(&"simplinkAutoPowerOn"))));
        assert!(reads.iter().any(|x| matches!(x, Read::Lg { category: "general", keys, .. } if keys.contains(&"noSignalScreenSaver"))));
    }

    #[test]
    fn forgetting_the_history_asks_and_tells_the_services_the_tv_has() {
        let mut w = world();
        let r = rows(&w.ctx());
        let ops = (by_id(&r, "history").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!("{ops:?}") };
        assert!(a.answers[1].danger);
        let uris = |ops: &[Op]| {
            ops.iter().filter_map(|o| if let Op::Act { uri, .. } = o { Some(uri.rsplit('/').next().unwrap().to_string()) } else { None }).collect::<Vec<_>>()
        };
        assert_eq!(uris(&a.answers[1].then), ["deletePairedDeviceList", "deleteIPConnectionHistory", "removeBlockedDeviceList"]);
        w.docs.set("bt", json!({"configs": {"system.supportBluetoothFeatures": ["remoteapp", "btsoundsink"]}}));
        let r = rows(&w.ctx());
        let ops = (by_id(&r, "history").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert_eq!(
            uris(&a.answers[1].then),
            ["resetWoBLE", "deletePairedDeviceList", "removePairedDevices", "deleteIPConnectionHistory", "removeBlockedDeviceList"]
        );
        assert!(matches!(a.answers[1].then.last(), Some(Op::Set(k, v)) if k == "inputs.historyDone" && v == &json!(true)));
        w.local.set("inputs.historyDone", json!(true));
        assert_eq!(by_id(&rows(&w.ctx()), "history").button, "Done");
        let launch = (by_id(&r, "game").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&launch[..], [Op::Launch { app, .. }] if app == "com.webos.app.gameoptimizer"));
        let to_bt = (by_id(&r, "bt").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&to_bt[..], [Op::Open { page, .. }] if page == "bthid"));
    }

    #[test]
    fn lg_channels_waits_for_confirmed_status_and_refreshes_after_a_change() {
        let mut w = world();
        w.docs.set("lgchannels", json!({"ok": true, "enabled": true, "state": "enabled", "reason": ""}));
        let row = lg_channels(&w.ctx());
        assert!(row.enabled && row.on);
        let ops = (row.act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&ops[..], [Op::Set(key, value), Op::Send { name: Some(name), method: "POST", path, body: Some(body) }] if key == "lgchannels.pending" && value == "apply" && name == "lgchannels.apply" && path == "/v1/lgchannels" && body == &json!({"enabled": false})));
        w.local.set("lgchannels.pending", json!("apply"));
        let row = lg_channels(&w.ctx());
        assert!(!row.enabled && row.on && row.detail == "Changing…");
        let ops = channel_answered(&w.ctx(), "lgchannels.apply", &json!({"ok": false, "error": {"message": "failed"}}));
        assert!(matches!(&ops[..], [Op::Set(key, value), Op::Refresh, _] if key == "lgchannels.pending" && value == "refresh"));
        assert!(matches!(&channel_opened(&w.ctx())[..], [Op::Set(key, value)] if key == "lgchannels.pending" && value == "refresh"));
        w.local.set("lgchannels.pending", json!("refresh"));
        assert!(!lg_channels(&w.ctx()).enabled);
        assert!(matches!(&channel_answered(&w.ctx(), "lgchannels", &json!({}))[..], [Op::Set(key, value)] if key == "lgchannels.pending" && value.is_null()));
        w.local.set("lgchannels.pending", Value::Null);
        w.docs.set("lgchannels", json!({"ok": true, "enabled": false, "state": "partial", "reason": "Background service still running"}));
        let row = lg_channels(&w.ctx());
        assert!(!row.on && row.detail == "Background service still running");
    }


    #[test]
    fn channels_wait_for_live_tv_and_the_tuner() {
        let mut w = World::new(lg_with(
            &[
                ("channel", json!({"settings": {"autoscanChannelGroup": "all_channel", "analogSystemColor": "ntsc"}})),
                ("option", json!({"settings": {"interactive-service": "onByUser", "interactive-service-hdmi": "offByUser"}})),
            ],
            &[],
        ));
        w.topics.set_channel("foreground", json!({"app": "com.webos.app.hdmi1"}));
        let r = channel_rows(&w.ctx());
        assert_eq!(by_id(&r, "channel.autoscanChannelGroup").value, "All Channels");
        assert!(!by_id(&r, "channel.analogSystemColor").enabled && by_id(&r, "channel.analogSystemColor").detail == "Live TV only");
        assert_eq!(by_id(&r, "channel.analogSystemColor").value, "NTSC");
        assert!(!by_id(&r, "option.interactive-service").visible, "no ATSC 3.0 tuner");
        w.docs.set("atsc3", json!({"configs": {"system.supportATSC3tuner": true}}));
        w.topics.set_channel("foreground", json!({"app": "com.webos.app.livetv"}));
        let r = channel_rows(&w.ctx());
        assert!(by_id(&r, "channel.analogSystemColor").enabled);
        assert!(by_id(&r, "option.interactive-service").visible && by_id(&r, "option.interactive-service").on, "onByUser reads as on");
        assert!(!by_id(&r, "option.interactive-service-hdmi").on);
        let ops = (by_id(&r, "option.interactive-service-hdmi").act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::Lg { value, .. }] if value == "onByUser"));
        let ops = (by_id(&r, "option.interactive-service").act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&ops[..], [Op::Lg { value, .. }] if value == "offByUser"));
        let auto = (by_id(&r, "autotuning").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&auto[..], [Op::Launch { app, params }] if app == "com.webos.app.channelsetting" && params == &json!({})), "LG's own channel setup, as its menu opens it");
        assert_eq!(by_id(&r, "autotuning").label, "Auto Tuning");
        let tune = (by_id(&r, "channeltuning").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&tune[..], [Op::Launch { params, .. }] if params["target"] == "channel"));
    }
}
