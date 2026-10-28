//! The remote's own keys through the compositor's routing (`keys.rs`, `input.rs`, `system_ui.rs`): what a tap, a hold and three taps
//! of the gear, Input, an app button, a digit, Mute, Back and Exit launch or close, and that no app sees a key that was taken.
//! Time is moved by hand (`key_event_at`, `poll_holds_at`), so nothing here sleeps.

mod harness;

use std::time::{Duration, Instant};

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::config::Config;
use lumatv_compositor::input::Keys;
use lumatv_compositor::input::code::*;
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
use serde_json::{Value, json};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_keyboard::WlKeyboard;

const WAIT: Duration = Duration::from_secs(3);

/// An app with a card, the keyboard focus and a mapped buffer.
struct App {
    client: Client,
    _win: Window,
    _kb: WlKeyboard,
    _buf: WlBuffer,
}

fn app(server: &mut Server, title: &str, app_id: &str) -> App {
    let mut client = Client::connect(server);
    let kb = client.seat.get_keyboard(&client.qh, ());
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", title);
    win.webos.set_property("appId".into(), app_id.into());
    let buf = client.shm_buffer(64, 64, 0xff00_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.keyboard_enters >= 1), "the card has the focus");
    App { client, _win: win, _kb: kb, _buf: buf }
}

/// The calls the compositor made to the app manager (`launch`, `closeByAppId`) as (method, payload), oldest first.
fn app_manager_calls(s: &mut State) -> Vec<(String, Value)> {
    s.luna
        .sent()
        .into_iter()
        .filter_map(|(uri, payload)| uri.strip_prefix("luna://com.webos.applicationManager/").map(|m| (m.to_string(), payload)))
        .filter(|(m, _)| m == "launch" || m == "closeByAppId")
        .collect()
}

fn launches(s: &mut State) -> Vec<String> {
    app_manager_calls(s).into_iter().filter(|(m, _)| m == "launch").map(|(_, p)| p["id"].as_str().unwrap_or("").to_string()).collect()
}

fn journal_has(s: &State, needle: &str) -> bool {
    s.journal.iter().any(|l| l.contains(needle))
}

/// Press at `t`, release `ms` later; returns the release time.
fn tap(s: &mut State, code: u32, t: Instant, ms: u64) -> Instant {
    let seat = SeatId(0);
    s.key_event_at(seat, code, true, false, t);
    let end = t + Duration::from_millis(ms);
    s.key_event_at(seat, code, false, false, end);
    end
}

#[test]
fn a_gear_tap_asks_for_quick_settings_and_no_app_sees_the_key() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Player", "org.test.player");
    let t = Instant::now();
    let (launched, said) = server.eval(move |s| {
        tap(s, KEY_MENU, t, 90);
        (launches(s), journal_has(s, "input: gear tap: Quick Settings has no window of its own here yet, launching com.palm.app.settings"))
    });
    assert_eq!(launched, vec!["com.palm.app.settings"]);
    assert!(said);
    a.client.roundtrip();
    assert!(a.client.state.keys.is_empty(), "the gear is the compositor's: {:?}", a.client.state.keys);
}

#[test]
fn three_gear_taps_in_three_seconds_open_the_av_reset_menu() {
    let mut server = Server::start();
    let _a = app(&mut server, "Player", "org.test.player");
    let t = Instant::now();
    let (launched, params) = server.eval(move |s| {
        let mut at = t;
        for _ in 0..3 {
            tap(s, KEY_MENU, at, 70);
            at += Duration::from_millis(800);
        }
        let params = app_manager_calls(s).last().map(|(_, p)| p["params"].clone());
        (launches(s), params)
    });
    assert_eq!(launched, vec!["com.palm.app.settings", "com.palm.app.settings", "com.webos.app.tvhotkey"]);
    assert_eq!(params, Some(json!({"activateType": "mute-hidden-action"})));
}

#[test]
fn a_gear_held_a_second_opens_settings_and_closes_it_when_it_is_in_front() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Player", "org.test.player");
    let t = Instant::now();
    let calls = server.eval(move |s| {
        s.key_event_at(SeatId(0), KEY_MENU, true, false, t);
        s.poll_holds_at(t + Duration::from_millis(900));
        assert!(launches(s).is_empty(), "not yet");
        s.poll_holds_at(t + Duration::from_millis(1050));
        s.key_event_at(SeatId(0), KEY_MENU, false, false, t + Duration::from_millis(1400));
        app_manager_calls(s)
    });
    assert_eq!(calls, vec![("launch".to_string(), json!({"id": "com.palm.app.settings"}))], "a hold launches Settings once and is no tap");
    // Settings in front now: the same hold closes it.
    let settings = {
        let mut c = Client::connect_extra(&mut server);
        let kb = c.seat.get_keyboard(&c.qh, ());
        let w = c.card("_WEBOS_WINDOW_TYPE_CARD", "Settings");
        w.webos.set_property("appId".into(), "com.palm.app.settings".into());
        let buf = c.shm_buffer(64, 64, 0xff00_0000);
        c.present(&w.surface, &buf);
        assert!(c.wait_until(WAIT, |s| s.keyboard_enters >= 1));
        (c, w, kb, buf)
    };
    a.client.roundtrip();
    let t2 = t + Duration::from_secs(5);
    let calls = server.eval(move |s| {
        s.key_event_at(SeatId(0), KEY_MENU, true, false, t2);
        s.poll_holds_at(t2 + Duration::from_millis(1100));
        s.key_event_at(SeatId(0), KEY_MENU, false, false, t2 + Duration::from_millis(1200));
        app_manager_calls(s)
    });
    assert_eq!(calls.last(), Some(&("closeByAppId".to_string(), json!({"id": "com.palm.app.settings"}))));
    drop(settings);
}

#[test]
fn input_is_a_picker_on_a_tap_and_the_hub_on_a_hold() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Player", "org.test.player");
    let t = Instant::now();
    let (launched, said_tap, said_hold) = server.eval(move |s| {
        let end = tap(s, KEY_VIDEO_NEXT, t, 120);
        let said_tap = journal_has(s, "input: Input tap: the input picker has no window of its own here yet");
        let t2 = end + Duration::from_secs(1);
        s.key_event_at(SeatId(0), KEY_VIDEO_NEXT, true, false, t2);
        s.poll_holds_at(t2 + Duration::from_millis(1100));
        s.key_event_at(SeatId(0), KEY_VIDEO_NEXT, false, false, t2 + Duration::from_millis(1300));
        (launches(s), said_tap, journal_has(s, "input: Input held: the Home Hub, launching com.webos.app.homeconnect"))
    });
    assert_eq!(launched, vec!["com.webos.app.homeconnect", "com.webos.app.homeconnect"], "one launch for the tap, one for the hold");
    assert!(said_tap && said_hold);
    a.client.roundtrip();
    assert!(a.client.state.keys.is_empty());
}

#[test]
fn with_the_shells_picker_input_acts_on_its_press_and_a_hold_still_opens_the_hub() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Player", "org.test.player");
    let t = Instant::now();
    let (at_press, at_release, hub_after_hold) = server.eval(move |s| {
        s.shell_handle(Msg::Connected);
        s.shell_handle(Msg::Ui { can: Some(vec!["picker".into()]), wants: None, live: None, state: None });
        let shows = |s: &State| s.shell.recorded.iter().filter(|e| e["topic"] == "show").count();
        let seat = SeatId(0);
        s.key_event_at(seat, KEY_VIDEO_NEXT, true, false, t);
        let at_press = (shows(s), journal_has(s, "input: Input tap: the input picker -> LumaTV's input picker"), launches(s));
        s.key_event_at(seat, KEY_VIDEO_NEXT, false, false, t + Duration::from_millis(120));
        let at_release = (shows(s), launches(s));
        // Held a second: the picker came at the press, the Hub follows at the hold; the release asks for nothing more.
        let t2 = t + Duration::from_secs(2);
        s.key_event_at(seat, KEY_VIDEO_NEXT, true, false, t2);
        s.poll_holds_at(t2 + Duration::from_millis(1100));
        s.key_event_at(seat, KEY_VIDEO_NEXT, false, false, t2 + Duration::from_millis(1300));
        (at_press, at_release, (shows(s), launches(s)))
    });
    assert_eq!(at_press, (1, true, vec![]), "the shell was asked for the picker at the press, and nothing was launched");
    assert_eq!(at_release, (1, vec![]), "the release asked for nothing");
    assert_eq!(hub_after_hold, (2, vec!["com.webos.app.homeconnect".to_string()]), "the second press showed the picker again; the hold opened the Hub once");
    a.client.roundtrip();
    assert!(a.client.state.keys.is_empty(), "the app never saw the key");
}

#[test]
fn the_app_buttons_launch_what_settings_remote_says_and_otherwise_lgs_own() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Player", "org.test.player");
    let dir = std::env::temp_dir().join(format!("lumatv-keys-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("theme")).unwrap();
    let theme = dir.join("theme/theme.json");
    std::fs::write(&theme, r#"{"remote": {"appKeys": {"netflix": "com.webos.app.hdmi2", "amazon": ""}}}"#).unwrap();
    let t = Instant::now();
    let (dir2, launched) = (dir.clone(), {
        let dir = dir.clone();
        server.eval(move |s| {
            s.keys = Keys::new(&Config { data_dir: dir, ..Config::default() });
            let mut at = t;
            for code in [KEY_NETFLIX, KEY_AMAZON] {
                at = tap(s, code, at, 60) + Duration::from_millis(500);
            }
            launches(s)
        })
    });
    assert_eq!(launched, vec!["com.webos.app.hdmi2"], "Netflix is the owner's app, Prime Video is nothing");
    // The owner changes his mind: the file is read again at the next press.
    std::fs::write(&theme, r#"{"remote": {"appKeys": {"amazon": "com.webos.app.hdmi3"}}}"#).unwrap();
    let launched = server.eval(move |s| {
        let mut at = t + Duration::from_secs(5);
        for code in [KEY_NETFLIX, KEY_AMAZON] {
            at = tap(s, code, at, 60) + Duration::from_millis(500);
        }
        launches(s)
    });
    assert_eq!(&launched[1..], ["netflix", "com.webos.app.hdmi3"], "{launched:?}");
    a.client.roundtrip();
    assert!(a.client.state.keys.is_empty(), "no app sees an app button that launched something");
    let _ = std::fs::remove_dir_all(dir2);
}

#[test]
fn a_button_over_its_own_app_goes_to_the_app() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Netflix", "netflix");
    let t = Instant::now();
    let launched = server.eval(move |s| {
        tap(s, KEY_NETFLIX, t, 60);
        launches(s)
    });
    assert!(launched.is_empty(), "{launched:?}");
    a.client.roundtrip();
    assert_eq!(a.client.state.keys, vec![(KEY_NETFLIX, 1), (KEY_NETFLIX, 0)], "LG hands the key to its own app");
}

#[test]
fn a_taught_code_is_a_button() {
    let mut server = Server::start();
    let _a = app(&mut server, "Player", "org.test.player");
    let dir = std::env::temp_dir().join(format!("lumatv-teach-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("compositor")).unwrap();
    std::fs::write(dir.join("compositor/keys.conf"), "# this remote\ndisneyplus 1041\n").unwrap();
    let t = Instant::now();
    let dir2 = dir.clone();
    let (launched, noted_before, noted_after) = server.eval(move |s| {
        // Before it is taught, the code is a key the table lacks, said once.
        tap(s, 1099, t, 60);
        tap(s, 1099, t + Duration::from_secs(1), 60);
        let noted_before = s.journal.iter().filter(|l| l.contains("key 1099 has no name")).count();
        s.keys = Keys::new(&Config { data_dir: dir2, ..Config::default() });
        tap(s, 1041, t + Duration::from_secs(2), 60);
        (launches(s), noted_before, s.journal.iter().filter(|l| l.contains("key 1041 has no name")).count())
    });
    assert_eq!(launched, vec!["com.disney.disneyplus-prod"]);
    assert_eq!((noted_before, noted_after), (1, 0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_digit_is_the_apps_and_held_a_second_it_asks_for_quick_access() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Player", "org.test.player");
    let t = Instant::now();
    server.eval(move |s| {
        // The card must take digits: its mask says so.
        let w = s.scene.windows().next().unwrap().id;
        s.scene.window_mut(w).unwrap().key_mask |= lumatv_compositor::window::key_mask::NUMERIC;
        tap(s, KEY_1 + 4, t, 100);
    });
    a.client.roundtrip();
    assert_eq!(a.client.state.keys, vec![(KEY_1 + 4, 1), (KEY_1 + 4, 0)], "a tap of 5 is the app's");
    a.client.state.keys.clear();
    let (launched, said) = server.eval(move |s| {
        let t2 = t + Duration::from_secs(2);
        s.key_event_at(SeatId(0), KEY_1 + 4, true, false, t2);
        s.poll_holds_at(t2 + Duration::from_millis(1100));
        s.key_event_at(SeatId(0), KEY_1 + 4, false, false, t2 + Duration::from_millis(1500));
        (launches(s), journal_has(s, "input: 5 held: Quick Access slot 5 has no window of its own here yet, launching com.webos.app.magicnum"))
    });
    assert_eq!(launched, vec!["com.webos.app.magicnum"]);
    assert!(said);
    a.client.roundtrip();
    assert_eq!(
        a.client.state.keys,
        vec![(KEY_1 + 4, 1), (KEY_1 + 4, 0)],
        "the app got the press and, at the hold, its release: no key stays down in its state"
    );
    let held = server.eval(|s| s.seat.pressed.clone());
    assert!(held.is_empty(), "{held:?}");
}

#[test]
fn mute_held_two_seconds_opens_the_accessibility_panel_and_does_not_toggle_the_mute() {
    let mut server = Server::start();
    let _a = app(&mut server, "Player", "org.test.player");
    let t = Instant::now();
    let (calls, mutes) = server.eval(move |s| {
        s.luna_register();
        s.key_event_at(SeatId(0), KEY_MUTE, true, false, t);
        s.poll_holds_at(t + Duration::from_millis(1900));
        s.poll_holds_at(t + Duration::from_millis(2050));
        s.key_event_at(SeatId(0), KEY_MUTE, false, false, t + Duration::from_millis(2300));
        let mutes = s.luna.sent().iter().filter(|(u, _)| u.ends_with("/master/muteVolume")).count();
        (app_manager_calls(s), mutes)
    });
    assert_eq!(calls, vec![("launch".to_string(), json!({"id": "com.webos.app.accessibility-overlay", "params": {"quickMode": true}}))]);
    assert_eq!(mutes, 0, "the release after the hold is swallowed");
    // A short press is still the toggle.
    let mutes = server.eval(move |s| {
        tap(s, KEY_MUTE, t + Duration::from_secs(5), 150);
        s.luna.sent().iter().filter(|(u, _)| u.ends_with("/master/muteVolume")).count()
    });
    assert_eq!(mutes, 1);
}

#[test]
fn exit_closes_the_app_that_does_not_take_it_and_never_the_home_app() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Player", "org.test.player");
    let t = Instant::now();
    let calls = server.eval(move |s| {
        tap(s, KEY_EXIT, t, 80);
        app_manager_calls(s)
    });
    assert_eq!(calls, vec![("closeByAppId".to_string(), json!({"id": "org.test.player", "reason": "windowPolicy"}))]);
    a.client.roundtrip();
    assert!(a.client.state.keys.is_empty());
    assert!(a.client.state.closes >= 1, "the window is asked to close");
    let mut server = Server::start();
    let _home = app(&mut server, "Home", "com.webos.app.home");
    let (calls, said) = server.eval(move |s| {
        tap(s, KEY_EXIT, t, 80);
        tap(s, KEY_PREVIOUS, t + Duration::from_secs(1), 80);
        (app_manager_calls(s), journal_has(s, "input: Back or Exit on Home: nothing to close"))
    });
    assert!(calls.is_empty() && said, "{calls:?}");
}

#[test]
fn back_held_a_second_is_exit_and_a_short_back_is_unchanged() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Player", "org.test.player");
    // The app takes Back (a KEYS_BACK app such as Netflix): a tap reaches it, a hold closes it.
    let t = Instant::now();
    let calls = server.eval(move |s| {
        let w = s.scene.windows().next().unwrap().id;
        s.scene.window_mut(w).unwrap().props.insert("_WEBOS_ACCESS_POLICY_KEYS_BACK".into(), "true".into());
        tap(s, KEY_PREVIOUS, t, 150);
        app_manager_calls(s)
    });
    assert!(calls.is_empty(), "{calls:?}");
    a.client.roundtrip();
    assert_eq!(a.client.state.keys, vec![(KEY_PREVIOUS, 1), (KEY_PREVIOUS, 0)]);
    a.client.state.keys.clear();
    let (calls, held) = server.eval(move |s| {
        let t2 = t + Duration::from_secs(3);
        s.key_event_at(SeatId(0), KEY_PREVIOUS, true, false, t2);
        s.poll_holds_at(t2 + Duration::from_millis(1100));
        s.key_event_at(SeatId(0), KEY_PREVIOUS, false, false, t2 + Duration::from_millis(1300));
        (app_manager_calls(s), s.seat.pressed.clone())
    });
    assert_eq!(calls, vec![("closeByAppId".to_string(), json!({"id": "org.test.player", "reason": "windowPolicy"}))]);
    assert!(held.is_empty(), "the app was given the release of its Back: {held:?}");
    a.client.roundtrip();
    assert_eq!(a.client.state.keys, vec![(KEY_PREVIOUS, 1), (KEY_PREVIOUS, 0)]);
}

#[test]
fn the_remote_command_and_a_tick_drive_the_same_holds() {
    // What the gate does on the set: `ctl remote 139 press`, a pause, `release`; the holds are read at the loop's tick.
    let mut server = Server::start();
    let _a = app(&mut server, "Player", "org.test.player");
    server.eval(|s| lumatv_compositor::control::handle(s, "remote 139 press").text());
    std::thread::sleep(Duration::from_millis(1100));
    server.eval(|s| s.poll_holds());
    server.eval(|s| lumatv_compositor::control::handle(s, "remote 139 release").text());
    let launched = server.eval(launches);
    assert_eq!(launched, vec!["com.palm.app.settings"]);
    let keys = server.eval(|s| lumatv_compositor::control::handle(s, "keys").text());
    assert!(keys.contains("{\"code\":139,\"name\":\"gear\"}") && keys.contains("\"foreground\":\"org.test.player\""), "{keys}");
}

#[test]
fn a_note_is_a_journal_line_the_gate_reads_what_followed_from() {
    let server = Server::start();
    let reply = server.eval(|s| lumatv_compositor::control::handle(s, "note keys-gear").text());
    assert!(reply.contains("\"ok\":true"), "{reply}");
    let journal = server.eval(|s| lumatv_compositor::control::handle(s, "journal").text());
    assert!(journal.contains("\"note: keys-gear\""), "{journal}");
}

#[test]
fn lgs_rf_home_773_is_home_not_an_unnamed_key_and_the_network_device_sends_the_network_home() {
    let server = Server::start();
    let journal = server.eval(|s| {
        s.key_event(SeatId(0), KEY_RF_HOME, true, true);
        s.key_event(SeatId(0), KEY_RF_HOME, false, true);
        s.key_event(SeatId(0), KEY_RF_HOME, true, false);
        s.key_event(SeatId(0), KEY_RF_HOME, false, false);
        s.journal.clone()
    });
    let homes: Vec<&String> = journal.iter().filter(|l| l.contains("launchDefaultApp")).collect();
    assert_eq!(homes, ["input: Home (network key): launchDefaultApp", "input: Home: launchDefaultApp"], "{journal:?}");
    assert!(!journal.iter().any(|l| l.contains("has no name")), "Home is a key the routing knows: {journal:?}");
}
