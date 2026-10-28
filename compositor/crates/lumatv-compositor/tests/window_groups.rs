//! Window groups as LG's apps use them: Live TV owns the group `com.webos.app.livetv` and Live Dmost, a WAM web app, attaches
//! its own window (it has a `wl_webos_shell_surface` of its own, type card) to the group's `dmostLayer`. Under LG's compositor
//! such a window is a child of the group, never a card (`StarfishFullscreenController.__allowedFullscreen`), so it neither takes
//! the front nor the keys from its owner. On the set the group member was taken for a card: it came to the front, minimized
//! Live TV (which hides its video then), SAM was told `["com.webos.app.livedmost"]` and brought Home back (2026-10-28).

mod harness;

use std::time::Duration;

use harness::{Client, ClientState, Server, Window};
use lumatv_compositor::State;
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell_surface::State as WlState;
use serde_json::json;
use wayland_client::Proxy;

const WAIT: Duration = Duration::from_secs(3);
const LIVETV: &str = "com.webos.app.livetv";
const DMOST: &str = "com.webos.app.livedmost";
const FULLSCREEN: u32 = WlState::Fullscreen as u32;
const MINIMIZED: u32 = WlState::Minimized as u32;

/// The states a shell surface was told, in order.
fn states(s: &ClientState, id: u32) -> Vec<u32> {
    s.states_of.iter().filter(|(w, _)| *w == id).map(|(_, st)| *st).collect()
}

fn id_of(w: &Window) -> u32 {
    w.webos.id().protocol_id()
}

/// What SAM's `getForegroundAppInfo` subscription is sent: per item the app, the group flags and the owner's id.
fn foreground(s: &mut State) -> Vec<(String, bool, bool, String)> {
    let info = s.luna_method("com.webos.surfacemanager", "getForegroundAppInfo", &json!({}), "com.webos.applicationManager").unwrap();
    info["foregroundAppInfo"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["appId"].as_str().unwrap().to_string(),
                a["windowGroup"].as_bool().unwrap(),
                a["windowGroupOwner"].as_bool().unwrap_or(false),
                a["windowGroupOwnerId"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

fn app(s: &State, title: &str) -> (Option<lumatv_compositor::window::WindowId>, Option<lumatv_compositor::surface::SurfaceId>) {
    let w = s.scene.windows().find(|w| w.title == title);
    (w.map(|w| w.id), w.map(|w| w.surface))
}

fn named(client: &mut Client, title: &str, app_id: &str) -> Window {
    let w = client.card("_WEBOS_WINDOW_TYPE_CARD", title);
    w.webos.set_property("appId".into(), app_id.into());
    w
}

/// Live TV's window as inputcommon makes it: a card that owns the group `com.webos.app.livetv` with LG's layers (appinfo.json).
fn live_tv(client: &mut Client) -> (Window, lumatv_wl_protocols::webos_surface_group::client::wl_webos_surface_group::WlWebosSurfaceGroup) {
    let tv = named(client, "Live TV", LIVETV);
    let group = client.groups.create_surface_group(&tv.surface, LIVETV.into(), &client.qh, ());
    for (name, z) in [("layer1", 1), ("jcapLayer", -2), ("dataLayer0", -5), ("acrLayer", -9), ("dmostLayer", -10), ("subtitleLayer", -20)] {
        group.create_layer(name.into(), z, &client.qh, ());
    }
    (tv, group)
}

/// Live TV's picture: transparent, with a red block at the top left (a banner).
fn banner_picture(client: &mut Client) -> wayland_client::protocol::wl_buffer::WlBuffer {
    client.shm_pattern(1920, 1080, |x, y| if x < 100 && y < 100 { 0xffff_0000 } else { 0 })
}

/// Live Dmost, the WAM window of the LG Channels overlay: a card by its own properties, attached to Live TV's `dmostLayer`.
fn live_dmost(client: &mut Client) -> Window {
    let dmost = named(client, "Live Dmost", DMOST);
    let group = client.groups.get_surface_group(LIVETV.into(), &client.qh, ());
    group.attach(&dmost.surface, "dmostLayer".into());
    dmost
}

#[test]
fn live_dmost_attached_to_live_tvs_group_does_not_take_the_front_from_live_tv() {
    let mut server = Server::start();
    let mut tv_client = Client::connect(&mut server);
    let mut wam = Client::connect_extra(&mut server);
    let _tv_keys = tv_client.seat.get_keyboard(&tv_client.qh, ());
    let _wam_keys = wam.seat.get_keyboard(&wam.qh, ());

    // Home is in front; SAM launches Live TV, which maps with its group.
    let home = named(&mut tv_client, "Home", "com.webos.app.home");
    let home_buffer = tv_client.shm_buffer(1920, 1080, 0xff00_00ff);
    tv_client.present(&home.surface, &home_buffer);
    assert!(tv_client.wait_until(WAIT, |s| states(s, id_of(&home)) == [FULLSCREEN]));
    let (tv, _group) = live_tv(&mut tv_client);
    let picture = banner_picture(&mut tv_client);
    tv_client.present(&tv.surface, &picture);
    assert!(tv_client.wait_until(WAIT, |s| states(s, id_of(&tv)) == [FULLSCREEN] && states(s, id_of(&home)) == [FULLSCREEN, MINIMIZED]));
    assert_eq!(server.eval(foreground), [(LIVETV.to_string(), true, true, String::new())]);

    // The LG Channels overlay's window comes up, attached to the dmost layer, a second after: green, all of the screen.
    let dmost = live_dmost(&mut wam);
    let green = wam.shm_buffer(1920, 1080, 0xff00_ff00);
    wam.present(&dmost.surface, &green);
    wam.roundtrip();
    tv_client.roundtrip();
    std::thread::sleep(Duration::from_millis(40));

    let (fg, front, focus, tv_surface, image) = server.eval(|s| {
        let (tv_window, tv_surface) = app(s, "Live TV");
        let front = s.scene.foreground(&s.surfaces).map(|w| w.id);
        let image = harness::drawn(s);
        (foreground(s), front == tv_window, s.seat.focus, tv_surface, image)
    });
    // What SAM is told: the group as LG's compositor lists it, the member below the owner first (lower children, owner, upper
    // children), never the member alone.
    assert_eq!(
        fg,
        [(DMOST.to_string(), true, false, LIVETV.to_string()), (LIVETV.to_string(), true, true, String::new())],
        "SAM is told the group, with Live TV in it"
    );
    assert!(front, "Live TV is still the foreground card");
    assert_eq!(focus, tv_surface, "the keys stay with Live TV");
    assert_eq!(states(&tv_client.state, id_of(&tv)), [FULLSCREEN], "Live TV is never minimized: it would hide its video");
    assert_eq!(states(&tv_client.state, id_of(&home)), [FULLSCREEN, MINIMIZED], "Home stays under it");
    assert_eq!(image.pixel(5, 5), [0xff, 0, 0, 0xff], "the banner is Live TV's, over everything");
    assert_eq!(image.pixel(500, 500), [0, 0xff, 0, 0xff], "the member shows through Live TV's transparent window, over the video");
    assert!(wam.wait_until(WAIT, |s| states(s, id_of(&dmost)) == [FULLSCREEN]), "the member is told it is fullscreen with its owner");

    // A key goes to Live TV, not to Live Dmost.
    let delivered = server.eval(|s| {
        use lumatv_compositor::server::seat::{SeatId, send_key};
        send_key(s, SeatId(0), 412, true) && send_key(s, SeatId(0), 412, false)
    });
    assert!(delivered);
    assert!(tv_client.wait_until(WAIT, |s| s.keys.len() >= 2));
    wam.roundtrip();
    assert!(wam.state.keys.is_empty(), "Live Dmost got no key");
}

#[test]
fn a_window_that_attaches_after_it_mapped_gives_the_front_back_to_its_owner() {
    let mut server = Server::start();
    let mut tv_client = Client::connect(&mut server);
    let mut wam = Client::connect_extra(&mut server);

    let (tv, _group) = live_tv(&mut tv_client);
    let picture = banner_picture(&mut tv_client);
    tv_client.present(&tv.surface, &picture);
    assert!(tv_client.wait_until(WAIT, |s| states(s, id_of(&tv)) == [FULLSCREEN]));

    // WAM maps the window as a card first; only then it attaches (the order is the client's).
    let dmost = named(&mut wam, "Live Dmost", DMOST);
    let green = wam.shm_buffer(1920, 1080, 0xff00_ff00);
    wam.present(&dmost.surface, &green);
    wam.roundtrip();
    assert!(tv_client.wait_until(WAIT, |s| states(s, id_of(&tv)) == [FULLSCREEN, MINIMIZED]), "a window that is a card is in front of Live TV");
    let group = wam.groups.get_surface_group(LIVETV.into(), &wam.qh, ());
    group.attach(&dmost.surface, "dmostLayer".into());
    wam.roundtrip();

    assert!(tv_client.wait_until(WAIT, |s| states(s, id_of(&tv)) == [FULLSCREEN, MINIMIZED, FULLSCREEN]), "Live TV is restored once the window is its member");
    let fg = server.eval(foreground);
    assert_eq!(fg.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), [DMOST, LIVETV]);
}

#[test]
fn a_member_whose_owner_is_gone_never_becomes_a_card() {
    let mut server = Server::start();
    let mut tv_client = Client::connect(&mut server);
    let mut wam = Client::connect_extra(&mut server);

    let home = named(&mut tv_client, "Home", "com.webos.app.home");
    let home_buffer = tv_client.shm_buffer(1920, 1080, 0xff00_00ff);
    tv_client.present(&home.surface, &home_buffer);
    let (tv, _group) = live_tv(&mut tv_client);
    let picture = banner_picture(&mut tv_client);
    tv_client.present(&tv.surface, &picture);
    let dmost = live_dmost(&mut wam);
    let green = wam.shm_buffer(1920, 1080, 0xff00_ff00);
    wam.present(&dmost.surface, &green);
    wam.roundtrip();
    assert!(tv_client.wait_until(WAIT, |s| states(s, id_of(&home)) == [FULLSCREEN, MINIMIZED]));

    // SAM closes Live TV; Live Dmost's window lingers a moment, attached to a group that is gone.
    tv.surface.destroy();
    tv_client.conn.flush().unwrap();
    assert!(tv_client.wait_until(WAIT, |s| states(s, id_of(&home)) == [FULLSCREEN, MINIMIZED, FULLSCREEN]), "Home is back");
    wam.roundtrip();
    std::thread::sleep(Duration::from_millis(40));
    let (fg, image) = server.eval(|s| (foreground(s), harness::drawn(s)));
    assert_eq!(fg.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["com.webos.app.home"], "the orphan is not in front");
    assert_eq!(image.pixel(500, 500), [0, 0, 0xff, 0xff], "Home's picture, not the orphan's");
    assert_eq!(states(&wam.state, id_of(&dmost)), [FULLSCREEN], "no card logic touches it");
}

#[test]
fn home_over_live_tv_minimizes_the_member_with_its_owner_and_live_tv_returns_with_it() {
    let mut server = Server::start();
    let mut tv_client = Client::connect(&mut server);
    let mut wam = Client::connect_extra(&mut server);

    let home = named(&mut tv_client, "Home", "com.webos.app.home");
    let home_buffer = tv_client.shm_buffer(1920, 1080, 0xff00_00ff);
    tv_client.present(&home.surface, &home_buffer);
    let (tv, _group) = live_tv(&mut tv_client);
    let picture = banner_picture(&mut tv_client);
    tv_client.present(&tv.surface, &picture);
    let dmost = live_dmost(&mut wam);
    let green = wam.shm_buffer(1920, 1080, 0xff00_ff00);
    wam.present(&dmost.surface, &green);
    wam.roundtrip();
    assert!(wam.wait_until(WAIT, |s| states(s, id_of(&dmost)) == [FULLSCREEN]));

    // Home relaunched over Live TV: both of Live TV's windows are minimized, SAM is told Home alone.
    home.webos.set_state(FULLSCREEN);
    tv_client.conn.flush().unwrap();
    assert!(wam.wait_until(WAIT, |s| states(s, id_of(&dmost)) == [FULLSCREEN, MINIMIZED]));
    assert!(tv_client.wait_until(WAIT, |s| states(s, id_of(&tv)) == [FULLSCREEN, MINIMIZED]));
    assert_eq!(server.eval(foreground), [("com.webos.app.home".to_string(), false, false, String::new())]);

    // Live TV asks for the front again: the member comes back with it.
    tv.webos.set_state(FULLSCREEN);
    tv_client.conn.flush().unwrap();
    assert!(wam.wait_until(WAIT, |s| states(s, id_of(&dmost)) == [FULLSCREEN, MINIMIZED, FULLSCREEN]));
    let fg = server.eval(foreground);
    assert_eq!(fg.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), [DMOST, LIVETV]);
}

/// LG's group model gives the keys to the topmost member of the group when it stacks above the owner (`layer1` is at z 1: the
/// in-front apps), never to one below it (the dmost, ACR and subtitle layers).
#[test]
fn the_keys_go_to_a_member_above_its_owner_and_come_back_when_it_leaves() {
    let mut server = Server::start();
    let mut tv_client = Client::connect(&mut server);
    let mut wam = Client::connect_extra(&mut server);
    let _tv_keys = tv_client.seat.get_keyboard(&tv_client.qh, ());
    let _wam_keys = wam.seat.get_keyboard(&wam.qh, ());

    let (tv, _group) = live_tv(&mut tv_client);
    let picture = banner_picture(&mut tv_client);
    tv_client.present(&tv.surface, &picture);
    assert!(tv_client.wait_until(WAIT, |s| s.keyboard_enters >= 1));
    let surface_of = |title: &'static str| move |s: &mut State| app(s, title).1;

    // The in-front app's window (a card by its properties) is attached to layer1, above Live TV.
    let crb = named(&mut wam, "Crb", "com.webos.app.crb");
    let group = wam.groups.get_surface_group(LIVETV.into(), &wam.qh, ());
    group.attach(&crb.surface, "layer1".into());
    let blue = wam.shm_buffer(400, 300, 0xff00_00ff);
    wam.present(&crb.surface, &blue);
    wam.roundtrip();
    assert!(wam.wait_until(WAIT, |s| s.keyboard_enters >= 1), "the member above the owner takes the keyboard");
    assert_eq!(server.eval(|s| s.seat.focus), server.eval(surface_of("Crb")));
    let (fg, front) = server.eval(|s| (foreground(s), s.scene.foreground(&s.surfaces).map(|w| w.title.clone())));
    assert_eq!(front.as_deref(), Some("Live TV"), "the member is not a card");
    assert_eq!(fg.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), [LIVETV, "com.webos.app.crb"], "listed above its owner");
    assert!(server.eval(|s| {
        use lumatv_compositor::server::seat::{SeatId, send_key};
        send_key(s, SeatId(0), 412, true)
    }));
    assert!(wam.wait_until(WAIT, |s| s.keys.len() == 1));
    tv_client.roundtrip();
    assert!(tv_client.state.keys.is_empty());

    // It leaves its group: the keys are Live TV's again, and the window, still mapped, is no card until it asks to be one.
    group.detach(&crb.surface);
    wam.roundtrip();
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(server.eval(|s| s.seat.focus), server.eval(surface_of("Live TV")));
    let (fg, image) = server.eval(|s| (foreground(s), harness::drawn(s)));
    assert_eq!(fg.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), [LIVETV]);
    assert_eq!(image.pixel(5, 5), [0xff, 0, 0, 0xff], "Live TV's picture, not the detached window's");
    crb.webos.set_state(FULLSCREEN);
    wam.conn.flush().unwrap();
    assert!(tv_client.wait_until(WAIT, |s| states(s, id_of(&tv)) == [FULLSCREEN, MINIMIZED]), "asking for fullscreen makes it a card");
    assert_eq!(server.eval(foreground).iter().map(|f| f.0.clone()).collect::<Vec<_>>(), ["com.webos.app.crb"]);
}

/// What the control socket's `windows` says of a group: its members by layer, and per window whether it is one.
#[test]
fn ctl_windows_names_the_members_of_a_group() {
    let mut server = Server::start();
    let mut tv_client = Client::connect(&mut server);
    let mut wam = Client::connect_extra(&mut server);
    let (tv, _group) = live_tv(&mut tv_client);
    let picture = banner_picture(&mut tv_client);
    tv_client.present(&tv.surface, &picture);
    let dmost = live_dmost(&mut wam);
    let green = wam.shm_buffer(1920, 1080, 0xff00_ff00);
    wam.present(&dmost.surface, &green);
    wam.roundtrip();
    tv_client.roundtrip();

    let report: serde_json::Value = serde_json::from_str(&server.eval(|s| lumatv_compositor::control::handle(s, "windows").text())).unwrap();
    let groups = report["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["name"], LIVETV);
    assert_eq!(groups[0]["members"], json!([{"layer": "dmostLayer", "z": -10, "surface": groups[0]["members"][0]["surface"], "appId": DMOST}]));
    let in_group: Vec<(&str, bool)> =
        report["windows"].as_array().unwrap().iter().map(|w| (w["appId"].as_str().unwrap(), w["inGroup"].as_bool().unwrap())).collect();
    assert!(in_group.contains(&(DMOST, true)) && in_group.contains(&(LIVETV, false)), "{in_group:?}");
}

/// An input app (HDMI 2 is inputcommon like Live TV) owns its group and nothing attaches to it: it is a card as any other, listed as the
/// owner of its group alone, and keeps the front and the keys.
#[test]
fn an_input_app_with_a_group_and_no_members_is_a_plain_card() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let _keys = client.seat.get_keyboard(&client.qh, ());
    let home = named(&mut client, "Home", "com.webos.app.home");
    let home_buffer = client.shm_buffer(1920, 1080, 0xff00_00ff);
    client.present(&home.surface, &home_buffer);
    let hdmi = named(&mut client, "HDMI 2", "com.webos.app.hdmi2");
    let group = client.groups.create_surface_group(&hdmi.surface, "com.webos.app.hdmi2".into(), &client.qh, ());
    group.create_layer("acrLayer".into(), -9, &client.qh, ());
    let picture = banner_picture(&mut client);
    client.present(&hdmi.surface, &picture);
    assert!(client.wait_until(WAIT, |s| states(s, id_of(&hdmi)) == [FULLSCREEN] && states(s, id_of(&home)) == [FULLSCREEN, MINIMIZED]));
    let (fg, focus, hdmi_surface) = server.eval(|s| (foreground(s), s.seat.focus, app(s, "HDMI 2").1));
    assert_eq!(fg, [("com.webos.app.hdmi2".to_string(), true, true, String::new())]);
    assert_eq!(focus, hdmi_surface);
    assert_eq!(states(&client.state, id_of(&hdmi)), [FULLSCREEN]);
}
