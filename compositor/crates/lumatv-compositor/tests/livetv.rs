//! Live TV in the compositor's frame: a card with a window group whose layers lie under the video (Live Dmost at `dmostLayer`, z -10). The plane's picture
//! hides them, so they are not drawn under the owner's hole, and their commits draw no frame; a launch and a leave of such a card run every refresh with
//! the member there, and a card that expects video whose hole never comes still ends its transition.

mod harness;

use harness::{Client, Server, Window, keysym_of};
use lumatv_compositor::Rect;
use lumatv_compositor::scene::Hole;
use lumatv_compositor::server::seat::SeatId;
use serde_json::json;
use wayland_client::protocol::wl_buffer::WlBuffer;

const MS: u64 = 1_000_000;
const REFRESH: u64 = 16_666_667;
const FULL: Rect = Rect::new(0, 0, 1920, 1080);
const RED: u32 = 0xffff_0000;
const BANNER: u32 = 0xff30_3030;

struct Rig {
    client: Client,
    stub: Window,
    _live: Window,
    group: lumatv_wl_protocols::webos_surface_group::client::wl_webos_surface_group::WlWebosSurfaceGroup,
    sid: lumatv_compositor::surface::SurfaceId,
    _bufs: Vec<WlBuffer>,
}

/// Native Home and Live TV: a card with a banner at the bottom and nothing over the video, a group with its layers.
fn rig(server: &mut Server, transitions: bool) -> Rig {
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut client = Client::connect(server);
    let stub = client.card("_WEBOS_WINDOW_TYPE_CARD", "Stub");
    stub.webos.set_property("appId".into(), "com.webos.app.home".into());
    let clear = client.shm_buffer(64, 64, 0);
    client.present(&stub.surface, &clear);
    client.roundtrip();
    server.eval(|s| { s.scene.set_home_rust(true); });
    if transitions {
        server.eval(|s| {
            s.motion.forced = Some(true);
            s.apply_motion();
            s.set_launch_origin(&json!({"appId": "com.webos.app.livetv", "tile": {"x": 287, "y": 699, "width": 120, "height": 120, "radius": 22}}));
            s.tick_at(1_000 * MS);
        });
    } else {
        server.eval(|s| {
            s.tick_at(1_000 * MS);
        });
    }
    let live = client.card("_WEBOS_WINDOW_TYPE_CARD", "Live TV");
    live.webos.set_property("appId".into(), "com.webos.app.livetv".into());
    client.roundtrip();
    let sid = server.eval(|s| s.scene.windows().find(|w| w.app_id() == "com.webos.app.livetv").map(|w| w.surface).unwrap());
    server.eval(move |s| s.scene.set_video_expected(sid, true));
    let group = client.groups.create_surface_group(&live.surface, "com.webos.app.livetv".into(), &client.qh, ());
    let _l1 = group.create_layer("layer1".into(), 1, &client.qh, ());
    let _l2 = group.create_layer("dmostLayer".into(), -10, &client.qh, ());
    let ui = client.shm_pattern(1920, 1080, |x, y| if y > 900 && (100..1800).contains(&x) { BANNER } else { 0 });
    client.present(&live.surface, &ui);
    client.roundtrip();
    Rig { client, stub, _live: live, group, sid, _bufs: vec![clear, ui] }
}

/// Live Dmost: its own card, attached to the group's `dmostLayer`, with a picture of one colour that covers `rect`.
fn dmost(rig: &mut Rig, rect: Rect, argb: u32) -> (Window, WlBuffer) {
    let d = rig.client.card("_WEBOS_WINDOW_TYPE_CARD", "Live Dmost");
    d.webos.set_property("appId".into(), "com.webos.app.livedmost".into());
    rig.client.roundtrip();
    let buf = rig.client.shm_buffer(rect.w, rect.h, argb);
    rig.group.attach(&d.surface, "dmostLayer".into());
    rig.client.present(&d.surface, &buf);
    rig.client.roundtrip();
    (d, buf)
}

/// One tick at `ns`: (items in the plan, the plan's damaged pixels, the picture).
fn tick(server: &Server, ns: u64) -> (usize, i64, lumatv_compositor::output::Image) {
    server.eval(move |s| {
        s.tick_at(ns);
        let plan = s.scene.last_plan();
        let (items, damage) = (plan.items.len(), plan.damage.w as i64 * plan.damage.h as i64);
        (items, damage, s.output.capture().unwrap())
    })
}

fn set_hole(server: &Server, sid: lumatv_compositor::surface::SurfaceId, rect: Option<Rect>) {
    server.eval(move |s| {
        s.scene.holes.retain(|h| h.surface != sid);
        if let Some(rect) = rect {
            s.scene.holes.push(Hole { surface: sid, rect });
        }
    });
}

#[test]
fn what_lies_under_the_videos_hole_is_not_drawn_and_its_commits_draw_nothing() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, false);
    let (d, dbuf) = dmost(&mut rig, FULL, RED);
    // no video yet: Live Dmost shows through the card's transparent picture
    let (items, _, image) = tick(&server, 2_000 * MS);
    assert_eq!(items, 2, "owner and member");
    assert_eq!(image.pixel(960, 300), [0xff, 0, 0, 0xff], "the member under the card's transparent pixels");
    // the video is there, full screen: the plane's picture is over the member
    set_hole(&server, rig.sid, Some(FULL));
    let (items, _, image) = tick(&server, 2_100 * MS);
    assert_eq!(items, 1, "the member is not drawn under a hole that covers it");
    assert_eq!(image.pixel(960, 300), [0, 0, 0, 0], "the hole, for the display engine's video");
    assert_eq!(image.pixel(960, 1000), [0x30, 0x30, 0x30, 0xff], "the card's own banner stays over the video");
    // the member commits again and again (an HbbTV layer repainting what nobody sees): no frame comes of it
    let again = rig.client.shm_buffer(1920, 1080, 0xff00_ff00);
    rig.client.present(&d.surface, &again);
    rig.client.roundtrip();
    let (items, damage, _) = tick(&server, 2_200 * MS);
    assert_eq!((items, damage), (1, 0), "nothing to draw");
    // a member that sticks out of the hole is drawn (the video is a window on the screen then, and the layer shows round it)
    set_hole(&server, rig.sid, Some(Rect::new(100, 100, 800, 450)));
    let (items, _, image) = tick(&server, 2_300 * MS);
    assert_eq!(items, 2);
    assert_eq!(image.pixel(1500, 300), [0, 0xff, 0, 0xff], "outside the video window, the member");
    assert_eq!(image.pixel(500, 300), [0, 0, 0, 0], "inside it, the plane");
    // and the video gone, the member is back at once
    set_hole(&server, rig.sid, None);
    let (items, _, image) = tick(&server, 2_400 * MS);
    assert_eq!((items, image.pixel(500, 300)), (2, [0, 0xff, 0, 0xff]));
    drop(dbuf);
}

#[test]
fn a_member_that_the_hole_covers_does_not_lose_its_callbacks_or_its_releases() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, false);
    let (d, _dbuf) = dmost(&mut rig, FULL, RED);
    set_hole(&server, rig.sid, Some(FULL));
    tick(&server, 2_000 * MS);
    tick(&server, 2_020 * MS);
    rig.client.roundtrip();
    let base = rig.client.state.frames_done;
    // it asks for a frame callback with each commit and is answered, drawn or not: an app that waits for it would stop
    for i in 0..3u64 {
        let next = rig.client.shm_buffer(1920, 1080, 0xff00_0000 | (i as u32 * 40));
        let _cb = rig.client.present(&d.surface, &next);
        rig.client.roundtrip();
        tick(&server, 2_100 * MS + i * REFRESH);
        tick(&server, 2_100 * MS + i * REFRESH + 8 * MS);
        assert!(rig.client.wait_until(std::time::Duration::from_secs(2), |s| s.frames_done >= base + 1 + i as u32), "callback {i} answered");
    }
}

/// With the UI engine (native Home is drawn by it: the leave's dissolve begins when Home has been drawn, not at a cap).
#[cfg(feature = "ui")]
#[test]
fn a_launch_and_a_leave_of_live_tv_run_every_refresh_with_its_dmost_window_there() {
    use harness::home_engine;
    if !home_engine::available() {
        return;
    }
    let (mut server, _engine) = home_engine::start([32, 48, 96]);
    let mut rig = rig(&mut server, true);
    let sid = rig.sid;
    let t0 = 2_000 * MS;
    // the move, a refresh apart
    for i in 0..16u64 {
        tick(&server, t0 + i * REFRESH);
    }
    let (late, max, frames) = server.eval(|s| (s.scene.transitions.stats.late_gaps, s.scene.transitions.stats.max_gap_ms, s.scene.transitions.stats.frames));
    assert_eq!((late, max, frames), (0, 16, 16), "16 frames, none late");
    // the card has arrived, nothing of the video yet: the plans are the app's commits and a slow tick, a hundred ms apart; no stutter
    tick(&server, t0 + 400 * MS);
    tick(&server, t0 + 500 * MS);
    // the hole comes (the compositor's placement), and Live Dmost starts under it a little later
    set_hole(&server, sid, Some(FULL));
    let (items, _, held) = tick(&server, t0 + 600 * MS);
    assert_eq!(held.pixel(960, 300), [0x19, 0x17, 0x24, 0xff], "the veil, not the plane's black");
    assert_eq!(items, 1);
    let (_d, _dbuf) = dmost(&mut rig, FULL, RED);
    let (items, _, held) = tick(&server, t0 + 700 * MS);
    assert_eq!(items, 1, "Live Dmost is under the veil and under the hole: not drawn");
    assert_eq!(held.pixel(960, 300), [0x19, 0x17, 0x24, 0xff]);
    // the video has had its 150 ms from 600 ms: the fade runs a refresh apart and ends
    let mut now = t0 + 760 * MS;
    let mut ticks = 0;
    while server.eval(|s| s.scene.transitions.active()) && ticks < 40 {
        tick(&server, now);
        now += REFRESH;
        ticks += 1;
    }
    assert!((9..=14).contains(&ticks), "the fade is 180 ms: {ticks} frames");
    let (late, max, caps) = server.eval(|s| (s.scene.transitions.stats.late_gaps, s.scene.transitions.stats.max_gap_ms, s.scene.transitions.stats.veil_caps));
    assert_eq!((late, caps), (0, 0), "the whole launch, held and faded, had no late frame (max gap {max} ms)");
    let (items, _, plane) = tick(&server, now + 100 * MS);
    assert_eq!((items, plane.pixel(960, 300)), (1, [0, 0, 0, 0]), "at rest: the card, its hole over the plane, Live Dmost not drawn");
    // Home: the card stays and Home dissolves in over it, 180 ms
    let leave = now + 2_000 * MS;
    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    let mut seen = Vec::new();
    let mut t = leave;
    loop {
        let (items, _, _) = tick(&server, t);
        if seen.is_empty() {
            assert!(server.eval(|s| s.scene.transitions.tracks().iter().any(|track| track.window == lumatv_compositor::scene::HOME_RUST)), "native Home participates in the leave");
        }
        seen.push(items);
        t += REFRESH;
        if !server.eval(|s| s.scene.transitions.active()) || seen.len() > 20 {
            break;
        }
    }
    // Native Home is rendered by the UI engine, not as a Wayland plan item.
    let (last, moving) = seen.split_last().unwrap();
    assert_eq!(*last, 0, "no Wayland windows over Home at the end");
    assert!(moving.iter().all(|&n| n == 1), "the card (a ghost), with Live Dmost not drawn under its hole: {seen:?}");
    assert!((11..=13).contains(&seen.len()), "180 ms: {} frames", seen.len());
    assert_eq!(server.eval(|s| s.scene.transitions.stats.late_gaps), 0);
}

#[test]
fn a_video_card_whose_hole_never_comes_ends_its_transition_at_the_veils_cap() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    let t0 = 2_000 * MS;
    for i in 0..16u64 {
        tick(&server, t0 + i * REFRESH);
    }
    assert!(server.eval(|s| s.scene.transitions.active()), "waiting for a hole");
    // no signal and no hole: a tick a hundred ms apart, as a calm loop slows to, until the cap (1.5 s after the arrival) is near; then the fade, a
    // refresh apart
    let mut now = t0 + 300 * MS;
    while now < t0 + 1_700 * MS {
        tick(&server, now);
        now += 100 * MS;
    }
    for _ in 0..20 {
        tick(&server, now);
        now += REFRESH;
    }
    let (active, caps, late) = server.eval(|s| (s.scene.transitions.active(), s.scene.transitions.stats.veil_caps, s.scene.transitions.stats.late_gaps));
    assert!(!active, "the card is at rest with no veil, no ghost and no track");
    assert_eq!((caps, late), (1, 0));
    let _ = &mut rig;
}

const KEY_CHANNELUP: u32 = 402;
const KEY_CHANNELDOWN: u32 = 403;

/// CH+ and CH- on Live TV: the card that owns the group has the keyboard (Live Dmost lies under it), is given both keys whole, and the keymap it was served
/// gives them the symbols its app changes channel on, `Key_webOS_ChannelUp` and `Key_webOS_ChannelDown` (0x01200021 and 0x01200022). On the set Live TV logged
/// Qt key 0 for every press (2026-11-03, `/var/log/inputcommon`: `[cpp][pressed] key: 0`) while the keymap named the keys and gave them no symbol.
#[test]
fn channel_up_and_down_reach_live_tv_as_the_keys_it_changes_channel_on() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, false);
    let (_d, _dbuf) = dmost(&mut rig, FULL, RED);
    tick(&server, 2_000 * MS);
    assert_eq!(server.eval(|s| s.seat.focus), Some(rig.sid), "Live TV's own card has the keys, not Live Dmost under it");
    let _kb = rig.client.seat.get_keyboard(&rig.client.qh, ());
    assert!(rig.client.wait_until(std::time::Duration::from_secs(3), |s| s.keyboard_enters >= 1), "Live TV's keyboard has the focus");
    server.eval(|s| {
        for code in [KEY_CHANNELUP, KEY_CHANNELDOWN] {
            s.key_event(SeatId(0), code, true, false);
            s.key_event(SeatId(0), code, false, false);
        }
    });
    rig.client.roundtrip();
    assert_eq!(rig.client.state.keys, vec![(KEY_CHANNELUP, 1), (KEY_CHANNELUP, 0), (KEY_CHANNELDOWN, 1), (KEY_CHANNELDOWN, 0)], "both keys reach Live TV");
    assert_eq!(server.eval(|s| (s.keys.delivered, s.keys.dropped)), (4, 0));
    let keymap = rig.client.state.keymaps.first().expect("the keyboard was served a keymap");
    assert_eq!(keysym_of(keymap, KEY_CHANNELUP).as_deref(), Some("0x01200021"), "Key_webOS_ChannelUp");
    assert_eq!(keysym_of(keymap, KEY_CHANNELDOWN).as_deref(), Some("0x01200022"), "Key_webOS_ChannelDown");
}
