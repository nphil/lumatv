//! The way out of LG's saver and the idle timers' subscription, against a compositor with no bridge (what it sends is kept): a wake call
//! whose answer never comes, a saver that stays up under keys, and a timer subscription tvpower's neighbour refuses.

mod harness;

use std::time::{Duration, Instant};

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::input::code::{KEY_POWER, KEY_UP};
use lumatv_compositor::luna::{Event, ToBridge};
use lumatv_compositor::server::seat::SeatId;
use serde_json::json;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_keyboard::WlKeyboard;

const WAIT: Duration = Duration::from_secs(3);

struct Shown {
    client: Client,
    win: Window,
    _kb: WlKeyboard,
    _buf: WlBuffer,
}

fn shown(server: &mut Server, first: bool, kind: &str, title: &str, app_id: &str, argb: u32) -> Shown {
    let mut client = if first { Client::connect(server) } else { Client::connect_extra(server) };
    let kb = client.seat.get_keyboard(&client.qh, ());
    let win = client.card(kind, title);
    win.webos.set_property("appId".into(), app_id.into());
    let buf = client.shm_buffer(64, 64, argb);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "{title} presented");
    Shown { client, win, _kb: kb, _buf: buf }
}

fn outbound(s: &State, tail: &str) -> Vec<(String, u64)> {
    s.luna
        .history()
        .into_iter()
        .filter_map(|m| match m {
            ToBridge::Outbound(o) if o.uri.ends_with(tail) => Some((o.uri, o.token)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_saver_whose_wake_is_never_answered_is_asked_again_closed_and_then_given_up_on() {
    let mut server = Server::start();
    let mut app = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "Player", "org.test.player", 0xff00_ff00);
    let mut saver = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_SCREENSAVER", "Screen Saver", "com.webos.app.screensaver", 0xffff_0000);
    saver.win.shell_surface.set_fullscreen(wayland_client::protocol::wl_shell_surface::FullscreenMethod::Default, 0, None);
    saver.client.roundtrip();
    let t0 = Instant::now();
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_UP, true, false);
        s.key_event(SeatId(0), KEY_UP, false, false);
        assert_eq!(outbound(s, "/power2/turnOnScreen").len(), 1, "the first key asks tvpower to end the saver");
        assert!(s.lg_saver_engaged());
        s.tick();
        assert!(s.scene.care.saver, "the OLED care stands aside for LG's saver: the app under it is not dimmed or shifted");
    });
    // no answer comes: a key two seconds later asks again
    server.eval(move |s| {
        s.wake_lg_saver_at(t0 + Duration::from_millis(2200));
        assert_eq!(outbound(s, "/power2/turnOnScreen").len(), 2, "a wake with no answer is lost and made again");
    });
    // three seconds on, the saver's app is asked to close, through its window and through SAM
    server.eval(move |s| {
        s.wake_lg_saver_at(t0 + Duration::from_millis(3300));
        assert!(s.luna_sent().iter().any(|(u, p)| u.ends_with("/closeByAppId") && p["id"] == "com.webos.app.screensaver"));
        assert!(s.lg_saver_engaged(), "still the saver's keys");
    });
    saver.client.roundtrip();
    assert!(saver.client.state.keys.is_empty(), "no key reached the saver");
    // six seconds on, it is given up on: its window is hidden and the next key is the app's
    server.eval(move |s| {
        s.wake_lg_saver_at(t0 + Duration::from_millis(6300));
        assert!(!s.lg_saver_engaged());
        assert!(!s.lg_saver_up(), "its window is hidden");
        assert_eq!(s.luna_report()["lgSaver"]["guard"]["abandons"], 1);
    });
    assert_eq!(server.eval(harness::drawn).pixel(5, 5), [0, 0xff, 0, 0xff], "the app shows again");
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_UP, true, false);
        s.key_event(SeatId(0), KEY_UP, false, false);
    });
    app.client.roundtrip();
    assert_eq!(app.client.state.keys, [(KEY_UP, 1), (KEY_UP, 0)], "the keys reach the app under the hung saver");
}

#[test]
fn tvpower_saying_screen_saver_before_its_window_is_mapped_still_takes_the_key() {
    let mut server = Server::start();
    let mut app = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "Player", "org.test.player", 0xff00_ff00);
    server.eval(|s| {
        s.luna_register();
        let token = outbound(s, "/power2/getPowerState")[0].1;
        s.luna_event(Event { token, payload: json!({"returnValue": true, "state": "ACTIVE", "sub state": "screen saver"}) });
        assert!(s.lg_saver_engaged(), "the state says saver; no window yet");
        s.key_event(SeatId(0), KEY_UP, true, false);
        s.key_event(SeatId(0), KEY_UP, false, false);
        assert_eq!(outbound(s, "/power2/turnOnScreen").len(), 1);
        let token = outbound(s, "/power2/getPowerState")[0].1;
        s.luna_event(Event { token, payload: json!({"returnValue": true, "state": "ACTIVE", "sub state": "active"}) });
        assert!(!s.lg_saver_engaged());
    });
    app.client.roundtrip();
    assert!(app.client.state.keys.is_empty(), "the key that found the saver coming is not the app's");
}

#[test]
fn a_refused_timer_subscription_is_made_again_and_the_new_one_is_the_one_listened_to() {
    let server = Server::start();
    server.eval(|s| {
        s.luna_register();
        let first = outbound(s, "/getTimerInfo");
        assert_eq!(first.len(), 1);
        let t0 = Instant::now();
        s.luna_event(Event { token: first[0].1, payload: json!({"returnValue": false, "errorText": "Service does not exist"}) });
        s.nooperation_tick_at(t0);
        assert_eq!(outbound(s, "/getTimerInfo").len(), 1, "not at once");
        s.nooperation_tick_at(t0 + Duration::from_millis(1100));
        let again = outbound(s, "/getTimerInfo");
        assert_eq!(again.len(), 2, "after a second");
        assert!(s.luna.history().iter().any(|m| matches!(m, ToBridge::Cancel { token } if *token == first[0].1)), "the dead call is cancelled");
        // the old token's late answer is nobody's; the new one's is the timers
        let timers = json!({"returnValue": true, "timers": [{"type": "[TIMER]ScreenSaver:180", "interval": 180, "once": false, "repeat": true}]});
        s.luna_event(Event { token: first[0].1, payload: timers.clone() });
        assert_eq!(s.luna_report()["idleTimers"]["timers"].as_array().unwrap().len(), 0);
        s.luna_event(Event { token: again[1].1, payload: timers });
        assert_eq!(s.luna_report()["idleTimers"]["timers"][0]["intervalSec"], 180);
        assert_eq!(s.luna_report()["subscriptions"][4]["answered"], true);
    });
}

#[test]
fn a_tvpower_registration_that_never_answers_is_made_again() {
    let server = Server::start();
    server.eval(|s| {
        s.luna_register();
        let before = outbound(s, "/power2/registerPreparePowerOff");
        s.nooperation_tick_at(Instant::now() + Duration::from_secs(11));
        let after = outbound(s, "/power2/registerPreparePowerOff");
        assert_eq!((before.len(), after.len()), (1, 2));
        // the push on the new registration is the round the compositor answers
        s.luna_event(Event { token: after[1].1, payload: json!({"timestamp": 3, "next sub state": "active standby"}) });
        assert_eq!(outbound(s, "/power2/responsePreparePowerOff").len(), 1);
        assert_eq!(s.luna_report()["rendering"], false);
    });
}

#[test]
fn a_key_or_any_other_input_pushes_the_idle_deadline_out() {
    let server = Server::start();
    server.eval(|s| {
        s.luna_register();
        let token = outbound(s, "/getTimerInfo")[0].1;
        let timers = json!({"returnValue": true, "timers": [{"type": "[TIMER]ScreenSaver:180", "interval": 180, "once": false, "repeat": true}]});
        s.luna_event(Event { token, payload: timers });
        let first = s.nooperation_deadline().expect("the saver's timer is armed");
        s.key_event(SeatId(0), KEY_UP, true, false);
        let after_key = s.nooperation_deadline().unwrap();
        assert!(after_key > first, "a key restarts the count");
        // the pointer and the wheel reach the same call (`care_input`)
        s.care_input();
        assert!(s.nooperation_deadline().unwrap() > after_key, "so does the pointer's and the wheel's input");
    });
}

fn calls_to(s: &State, tail: &str) -> usize {
    s.luna_sent().iter().filter(|(u, _)| u.ends_with(tail)).count()
}

fn power_state(s: &mut State, state: &str, sub: &str) {
    let token = outbound(s, "/power2/getPowerState")[0].1;
    s.luna_event(Event { token, payload: json!({"returnValue": true, "state": state, "sub state": sub}) });
}

#[test]
fn power_pressed_with_lg_saver_up_goes_to_standby_at_once_and_never_wakes_it() {
    let mut server = Server::start();
    let mut app = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "Player", "org.test.player", 0xff00_ff00);
    let mut saver = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_SCREENSAVER", "Screen Saver", "com.webos.app.screensaver", 0xffff_0000);
    saver.win.shell_surface.set_fullscreen(wayland_client::protocol::wl_shell_surface::FullscreenMethod::Default, 0, None);
    saver.client.roundtrip();
    server.eval(|s| {
        s.luna_register();
        power_state(s, "ACTIVE", "screen saver");
        assert!(s.lg_saver_up());
        s.key_event(SeatId(0), KEY_POWER, true, false);
        s.key_event(SeatId(0), KEY_POWER, false, false);
        assert_eq!(calls_to(s, "/power/powerOff"), 1, "the first press asks for standby");
        assert_eq!(calls_to(s, "/power2/turnOnScreen"), 0, "and does not wake the saver it turns off");
    });
    saver.client.roundtrip();
    app.client.roundtrip();
    assert!(saver.client.state.keys.is_empty() && app.client.state.keys.is_empty(), "the key is nobody's: not the saver app's, which would wake the screen");
}

#[test]
fn power_pressed_while_tvpower_says_screen_saver_before_the_window_maps_goes_to_standby() {
    let server = Server::start();
    server.eval(|s| {
        s.luna_register();
        power_state(s, "ACTIVE", "screen saver");
        s.key_event(SeatId(0), KEY_POWER, true, false);
        assert_eq!(calls_to(s, "/power/powerOff"), 1);
        assert_eq!(calls_to(s, "/power2/turnOnScreen"), 0);
        // a state that names the saver itself, as the legacy service does, asks the same
        power_state(s, "Screen Saver", "");
        s.key_event(SeatId(0), KEY_POWER, false, false);
        s.key_event_at(SeatId(0), KEY_POWER, true, false, Instant::now() + Duration::from_secs(3));
        assert_eq!(calls_to(s, "/power/powerOff"), 2);
    });
}

#[test]
fn power_stays_a_wake_in_standby_and_a_no_op_in_a_transition_with_no_saver() {
    let server = Server::start();
    server.eval(|s| {
        s.luna_register();
        power_state(s, "ACTIVE", "active");
        s.key_event(SeatId(0), KEY_POWER, true, false);
        assert_eq!((calls_to(s, "/power/powerOff"), calls_to(s, "/power2/turnOnScreen")), (1, 0));
        power_state(s, "Active Standby", "");
        s.key_event(SeatId(0), KEY_POWER, false, false);
        s.key_event_at(SeatId(0), KEY_POWER, true, false, Instant::now() + Duration::from_secs(3));
        assert_eq!(calls_to(s, "/power/powerOff"), 1, "standby is left to tvpower's own wake");
        assert_eq!(calls_to(s, "/power/powerOn"), 0);
    });
}
