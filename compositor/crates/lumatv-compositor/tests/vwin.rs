//! LumaTV's own video windows (`vwin`, docs/research/hw-planes-result.md) under the real `State`: a claim from the shell or `ctl` is bound to the video
//! window the feeder opened, the window is drawn with a hole and its video placed at the claim's rectangle, and the window is not there when there is no
//! claim, when an app plays video (`idle`), or when the switch is off. The sinks `videooutputd` reports are fed in as its status pushes.

mod harness;

use std::time::Duration;

use harness::{Client, Server, assert_placements_valid, drawn, placements};
use lumatv_compositor::control;
use lumatv_compositor::shell::Msg;
use lumatv_compositor::vwin::Mode;
use serde_json::{Value, json};
use wayland_client::protocol::wl_surface::WlSurface;
use lumatv_wl_protocols::webos_foreign::client::wl_webos_exported::WlWebosExported;
use lumatv_wl_protocols::webos_foreign::client::wl_webos_imported::WlWebosImported;

const WAIT: Duration = Duration::from_secs(5);
const VIDEO: &str = "_WEBOS_WINDOW_TYPE_LUMATV_VIDEO";
const CARD: &str = "_WEBOS_WINDOW_TYPE_CARD";
const WALLPAPER: &str = "_WEBOS_WINDOW_TYPE_LUMATV_WALLPAPER";

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const HOLE: [u8; 4] = [0, 0, 0, 0];

/// A video element of a window: exported by it, with the pipeline's punch-through attached.
struct Element {
    exported: WlWebosExported,
    _imported: WlWebosImported,
}

fn export(client: &mut Client, surface: &WlSurface, ctx: &str) -> Element {
    let n = client.state.window_ids.len();
    let exported = client.foreign.export_element(surface, 0, &client.qh, ());
    client.roundtrip();
    let id = client.state.window_ids[n].0.clone();
    let src = client.region(0, 0, 1920, 1080);
    let dst = client.region(0, 0, 1920, 1080);
    exported.set_exported_window(&src, &dst);
    let imported = client.foreign.import_element(id, 0, &client.qh, ());
    imported.attach_punchthrough(ctx.to_string());
    client.roundtrip();
    assert!(client.wait_until(WAIT, |s| s.punchthrough_attached.iter().any(|c| c == ctx)), "{ctx} attached");
    Element { exported, _imported: imported }
}

/// An app's card in one colour, full screen.
fn app(server: &mut Server, title: &str, argb: u32) -> Client {
    let mut client = Client::connect_extra(server);
    let win = client.card(CARD, title);
    let buf = client.shm_buffer(1920, 1080, argb);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "{title} presented");
    std::mem::forget((win, buf));
    client
}

/// What the feeder opens: a video window that is transparent everywhere (the video is the plane's, under the hole) but for a green marker at its corner,
/// so that a test can tell whether the window is in the stack; with its element and the pipeline's punch-through.
struct Feeder {
    client: Client,
    win: harness::Window,
    el: Element,
}

fn feeder(server: &mut Server, title: &str, ctx: &str) -> Feeder {
    let mut client = Client::connect_extra(server);
    let win = client.card(VIDEO, title);
    let buf = client.shm_pattern(1920, 1080, |x, y| if x < 16 && y < 16 { 0xff00_ff00 } else { 0 });
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "{title} presented");
    let el = export(&mut client, &win.surface, ctx);
    std::mem::forget(buf);
    Feeder { client, win, el }
}

fn claim(id: &str, kind: &str, app: &str, rect: [i32; 4]) -> Value {
    json!({"op": "claim", "id": id, "kind": kind, "app": app, "rect": {"x": rect[0], "y": rect[1], "w": rect[2], "h": rect[3]},
           "need": {"codec": "h264", "w": 1920, "h": 1080, "fps": 30}})
}

const POPUP: [i32; 4] = [1300, 60, 560, 315];

fn ask(server: &Server, body: Value) -> Value {
    server.eval(move |s| s.vwin_command(&body))
}

fn state_of(reply: &Value) -> (String, String) {
    (reply["claim"]["state"].as_str().unwrap_or("?").to_string(), reply["claim"]["reason"].as_str().unwrap_or("?").to_string())
}

fn set_mode(server: &Server, mode: Mode) {
    server.eval(move |s| s.vwin_set_mode(mode));
}

fn last_placement(server: &Server, ctx: &'static str) -> Value {
    let placed = server.eval(placements);
    assert_placements_valid(&placed);
    placed.iter().rev().find(|p| p["context"] == ctx).cloned().unwrap_or_else(|| panic!("no placement for {ctx} in {placed:?}"))
}

#[test]
fn a_granted_claim_shows_its_window_with_a_hole_and_places_the_video_at_the_claims_rectangle() {
    let mut server = Server::start();
    let _app = app(&mut server, "App", 0xffff_0000);
    set_mode(&server, Mode::Apps);
    let feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    // no claim: the window is not in the stack, nothing is placed
    let image = server.eval(drawn);
    assert_eq!(image.pixel(5, 5), RED, "the window of no claim is not drawn");
    assert!(server.eval(placements).is_empty());

    let reply = ask(&server, claim("cam", "camera-popup", "org.test.cameras", POPUP));
    assert_eq!(state_of(&reply), ("granted".to_string(), String::new()), "{reply}");
    assert_eq!(reply["claim"]["slot"], "main");
    let image = server.eval(drawn);
    assert_eq!(image.pixel(5, 5), GREEN, "the window is in the stack from the grant");
    assert_eq!(image.pixel(1300, 60), HOLE, "the hole's first pixel");
    assert_eq!(image.pixel(1859, 374), HOLE, "and its last");
    assert_eq!(image.pixel(1299, 60), RED, "outside it, the app under the transparent window");
    assert_eq!(image.pixel(1860, 200), RED);
    // the video is placed in the panel's pixels (the panel is twice the output)
    let p = last_placement(&server, "_ctx_cam");
    assert_eq!(p["displayOutput"], json!({"x": 2600, "y": 120, "width": 1120, "height": 630}), "{p}");
    assert_eq!(p["fullScreen"], false);
    let r = server.eval(|s| s.vwin_report());
    assert_eq!(r["windows"][0]["live"], true, "{r}");
    assert_eq!(r["claims"][0]["placed"], true);
    assert_eq!(r["free"], 1, "one of the two windows is left");

    // released: the window is not drawn and the video is hidden; the window itself stays for the next claim
    let reply = ask(&server, json!({"op": "release", "id": "cam"}));
    assert_eq!(reply["released"], true);
    let image = server.eval(drawn);
    assert_eq!(image.pixel(5, 5), RED);
    assert_eq!(image.pixel(1500, 200), RED, "no hole");
    let p = last_placement(&server, "_ctx_cam");
    assert_eq!(p["displayOutput"]["width"], 0, "hidden: {p}");
    let mut c = feeder.client;
    c.roundtrip();
    assert_eq!(c.state.closes, 0, "a released window is left for the next claim, not closed");
    drop((feeder.win, feeder.el));
}

#[test]
fn a_moved_claim_moves_the_hole_and_the_placement() {
    let mut server = Server::start();
    let _app = app(&mut server, "App", 0xffff_0000);
    set_mode(&server, Mode::Apps);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    ask(&server, claim("cam", "camera-popup", "org.test.cameras", POPUP));
    let reply = ask(&server, json!({"op": "move", "id": "cam", "rect": {"x": 100, "y": 500, "w": 480, "h": 270}}));
    assert_eq!(reply["ok"], true, "{reply}");
    let image = server.eval(drawn);
    assert_eq!(image.pixel(1500, 200), RED, "the old place is the app's again");
    assert_eq!(image.pixel(300, 600), HOLE, "the new one is the hole");
    assert_eq!(last_placement(&server, "_ctx_cam")["displayOutput"], json!({"x": 200, "y": 1000, "width": 960, "height": 540}));
    // a rectangle that leaves the output, or a claim that was never made, is answered and changes nothing
    let reply = ask(&server, json!({"op": "move", "id": "cam", "rect": {"x": 1800, "y": 0, "w": 560, "h": 315}}));
    assert_eq!(reply["ok"], false);
    assert_eq!(ask(&server, json!({"op": "move", "id": "nobody", "rect": {"x": 0, "y": 0, "w": 200, "h": 100}}))["ok"], false);
    assert_eq!(server.eval(drawn).pixel(300, 600), HOLE, "the hole stayed where it was put");
}

#[test]
fn a_claim_is_denied_while_the_switch_is_off_and_nothing_is_drawn() {
    let mut server = Server::start();
    let _app = app(&mut server, "App", 0xffff_0000);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    let reply = ask(&server, claim("cam", "camera-popup", "org.test.cameras", POPUP));
    assert_eq!(state_of(&reply), ("denied".to_string(), "off".to_string()), "{reply}");
    let image = server.eval(drawn);
    assert_eq!(image.pixel(5, 5), RED);
    assert_eq!(image.pixel(1500, 200), RED);
    assert!(server.eval(placements).is_empty(), "no placement for a window that was not granted");
    // a bad request is answered, not guessed at
    assert_eq!(ask(&server, json!({"op": "claim", "id": "x", "kind": "banner", "rect": {"x": 0, "y": 0, "w": 200, "h": 100}}))["ok"], false);
    assert_eq!(ask(&server, json!({"op": "claim", "kind": "pip", "rect": {"x": 0, "y": 0, "w": 200, "h": 100}}))["ok"], false);
    assert_eq!(ask(&server, json!({"op": "wiggle"}))["ok"], false);
}

#[test]
fn a_video_window_nobody_claimed_is_closed_so_that_its_decoder_is_free() {
    let mut server = Server::start();
    let _app = app(&mut server, "App", 0xffff_0000);
    let mut feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    assert!(feeder.client.wait_until(Duration::from_secs(8), |s| s.closes >= 1), "the window was closed after a few seconds without a claim");
    let journal = server.eval(|s| s.journal.clone());
    assert!(journal.iter().any(|l| l.contains("vwin: close sent") && l.contains("no claim was granted")), "{journal:?}");
}

#[test]
fn an_app_that_plays_video_takes_the_window_back_in_idle_mode() {
    let mut server = Server::start();
    let _bg = app(&mut server, "Bg", 0xffff_0000);
    set_mode(&server, Mode::Idle);
    let mut feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    let reply = ask(&server, claim("cam", "camera-full", "org.test.cameras", [0, 0, 1920, 1080]));
    assert_eq!(state_of(&reply).0, "granted", "{reply}");
    assert_eq!(server.eval(drawn).pixel(300, 300), HOLE);
    // an app opens a player: its element is on the screen, and the video plane is its
    let mut player = Client::connect_extra(&mut server);
    let win = player.card(CARD, "Player");
    let buf = player.shm_buffer(1920, 1080, 0xff20_2020);
    player.present(&win.surface, &buf);
    assert!(player.wait_until(WAIT, |s| s.frames_done >= 1));
    let _el = export(&mut player, &win.surface, "_ctx_app");
    assert!(feeder.client.wait_until(WAIT, |s| s.closes >= 1), "the claim's window was closed for the app's video");
    let r = server.eval(|s| s.vwin_report());
    assert_eq!((r["claims"][0]["state"].as_str(), r["claims"][0]["reason"].as_str()), (Some("preempted"), Some("app-video")), "{r}");
    let image = server.eval(drawn);
    assert_ne!(image.pixel(5, 5), GREEN, "the video window is gone from the picture");
    assert_eq!(last_placement(&server, "_ctx_cam")["displayOutput"]["width"], 0, "its video is hidden");
    // and a new claim is refused while the app plays
    let reply = ask(&server, claim("cam2", "camera-popup", "org.test.cameras", POPUP));
    assert_eq!(state_of(&reply), ("denied".to_string(), "app-video".to_string()), "{reply}");
    std::mem::forget((win, buf, feeder.win, feeder.el));
}

#[test]
fn apps_mode_gives_the_second_window_beside_an_app_unless_its_video_uses_both_scalers() {
    let mut server = Server::start();
    set_mode(&server, Mode::Apps);
    let mut player = Client::connect_extra(&mut server);
    let win = player.card(CARD, "Player");
    let buf = player.shm_buffer(1920, 1080, 0xff20_2020);
    player.present(&win.surface, &buf);
    assert!(player.wait_until(WAIT, |s| s.frames_done >= 1));
    let _el = export(&mut player, &win.surface, "_ctx_app");
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    // `videooutputd`'s status: the app's video on MAIN using both scalers
    let sinks = |both: bool| {
        json!({"video": [
            {"sink": "MAIN", "connected": true, "context": "_ctx_app", "appId": "netflix", "zOrder": 0, "width": 3840, "height": 2160, "frameRate": 120,
             "videoInfo": {"subScalerOccupied": both}},
            {"sink": "SUB0", "connected": false, "context": "", "zOrder": 1, "videoInfo": {}}]})
    };
    server.eval(move |s| s.vwin_status(&sinks(true)));
    let reply = ask(&server, claim("cam", "camera-popup", "org.test.cameras", POPUP));
    assert_eq!(state_of(&reply), ("denied".to_string(), "both-scalers".to_string()), "{reply}");
    server.eval(move |s| s.vwin_status(&sinks(false)));
    let reply = ask(&server, claim("cam", "camera-popup", "org.test.cameras", POPUP));
    assert_eq!(state_of(&reply), ("granted".to_string(), String::new()), "{reply}");
    assert_eq!(reply["claim"]["slot"], "sub");
    assert_eq!(reply["claim"]["limits"], json!({"w": 1920, "h": 1088, "fps": 30}), "the sub window's limits");
    // a stream the sub window cannot take is not granted that window
    let mut big = claim("big", "pip", "", [0, 0, 640, 360]);
    big["need"] = json!({"codec": "h265", "w": 3840, "h": 2160, "fps": 60});
    let reply = ask(&server, big);
    assert_ne!(state_of(&reply).0, "granted", "{reply}");
    // the app's video turns to using both scalers while the claim plays: the claim ends
    server.eval(move |s| s.vwin_status(&sinks(true)));
    let r = server.eval(|s| s.vwin_report());
    assert_eq!(r["claims"][0]["state"], "preempted", "{r}");
    assert_eq!(r["claims"][0]["reason"], "both-scalers");
    assert_eq!(r["sinks"][0]["occupiesBoth"], true);
    std::mem::forget((win, buf));
}

#[test]
fn a_claim_takes_the_wallpapers_window_and_the_wallpaper_is_closed() {
    let mut server = Server::start();
    set_mode(&server, Mode::Apps);
    server.eval(|s| s.vwin_ctl(&["slots", "1"]));
    let mut wall = Client::connect_extra(&mut server);
    let win = wall.card(WALLPAPER, "Wall");
    let buf = wall.shm_buffer(1920, 1080, 0xff00_00ff);
    wall.present(&win.surface, &buf);
    assert!(wall.wait_until(WAIT, |s| s.frames_done >= 1));
    let wall_el = export(&mut wall, &win.surface, "_ctx_wall");
    let r = server.eval(|s| s.vwin_report());
    assert_eq!((r["wall"].as_bool(), r["free"].as_i64()), (Some(true), Some(0)), "the wallpaper holds the one window: {r}");
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    let reply = ask(&server, claim("cam", "camera-popup", "org.test.cameras", POPUP));
    assert_eq!(state_of(&reply).0, "granted", "{reply}");
    assert!(wall.wait_until(WAIT, |s| s.closes >= 1), "the wallpaper's window was closed (as when Home leaves the screen)");
    // a claim of a lower priority than the wallpaper does not exist, but one of the same kind is not served by closing the first
    let reply = ask(&server, claim("cam2", "pip", "", [0, 0, 480, 270]));
    assert_eq!(state_of(&reply).0, "denied", "{reply}");
    wall_el.exported.destroy();
    wall.roundtrip();
    std::mem::forget((win, buf));
}

#[test]
fn a_video_window_is_not_where_the_pointer_lands() {
    let mut server = Server::start();
    let _app = app(&mut server, "App", 0xffff_0000);
    set_mode(&server, Mode::Apps);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    ask(&server, claim("cam", "camera-full", "org.test.cameras", [0, 0, 1920, 1080]));
    let (hit, focus) = server.eval(|s| {
        s.tick();
        let hit = s.scene.surface_at(960, 540).and_then(|(id, _)| s.scene.window_of(id)).and_then(|w| s.scene.window(w)).map(|w| w.title.clone());
        let focus = s.seat.focus.and_then(|f| s.scene.window_of(f)).and_then(|w| s.scene.window(w)).map(|w| w.title.clone());
        (hit, focus)
    });
    assert_eq!(hit.as_deref(), Some("App"), "the pointer goes through to the app under the video");
    assert_eq!(focus.as_deref(), Some("App"), "and the keys stay with the app");
}

#[test]
fn ctl_vwin_reads_switches_claims_and_releases() {
    let mut server = Server::start();
    let _app = app(&mut server, "App", 0xffff_0000);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    let ctl = |server: &Server, line: &'static str| -> Value { serde_json::from_str(&server.eval(move |s| control::handle(s, line).text())).unwrap() };
    let r = ctl(&server, "vwin");
    assert_eq!((r["ok"].as_bool(), r["vwin"]["mode"].as_str(), r["vwin"]["slots"].as_i64()), (Some(true), Some("off"), Some(2)), "{r}");
    assert_eq!(ctl(&server, "status")["vwin"]["mode"], "off");
    let r = ctl(&server, "vwin claim cam camera-popup 1300 60 560 315 org.test.cameras h264 1920x1080@30");
    assert_eq!(state_of(&r), ("denied".to_string(), "off".to_string()), "{r}");
    let r = ctl(&server, "vwin apps");
    assert_eq!(r["vwin"]["mode"], "apps");
    let r = ctl(&server, "vwin claim cam camera-popup 1300 60 560 315 org.test.cameras h264 1920x1080@30");
    assert_eq!(state_of(&r), ("granted".to_string(), String::new()), "{r}");
    assert_eq!(r["claim"]["need"], json!({"codec": "h264", "w": 1920, "h": 1080, "fps": 30}));
    assert_eq!(ctl(&server, "vwin move cam 100 100 480 270")["claim"]["rect"], json!({"x": 100, "y": 100, "w": 480, "h": 270}));
    assert_eq!(ctl(&server, "vwin release cam")["released"], true);
    assert_eq!(ctl(&server, "vwin release all")["released"], 0);
    assert_eq!(ctl(&server, "vwin slots 1")["vwin"]["slots"], 1);
    assert_eq!(ctl(&server, "vwin slots 9")["ok"], false);
    assert_eq!(ctl(&server, "vwin claim cam camera-popup 1 2")["ok"], false);
    assert_eq!(ctl(&server, "vwin dance")["ok"], false);
    let r = ctl(&server, "vwin off");
    assert_eq!(r["vwin"]["mode"], "off");
}

#[test]
fn the_shell_asks_through_its_channel_and_reads_the_answer_in_the_topic() {
    let mut server = Server::start();
    server.eval(|s| s.shell_handle(Msg::Connected));
    let _app = app(&mut server, "App", 0xffff_0000);
    set_mode(&server, Mode::Apps);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    server.eval(|s| s.shell_handle(Msg::Vwin(claim("cam", "camera-popup", "org.test.cameras", POPUP))));
    let topic = server.eval(|s| s.shell.topic("vwin").cloned()).expect("the topic is published");
    assert_eq!(topic["mode"], "apps");
    assert_eq!(topic["claims"][0]["id"], "cam");
    assert_eq!(topic["claims"][0]["state"], "granted", "{topic}");
    assert_eq!(topic["claims"][0]["placed"], true);
    assert_eq!(topic["free"], 1);
    server.eval(|s| s.shell_handle(Msg::Vwin(json!({"op": "release", "id": "cam"}))));
    let topic = server.eval(|s| s.shell.topic("vwin").cloned()).unwrap();
    assert_eq!(topic["claims"].as_array().map(Vec::len), Some(0), "{topic}");
    assert_eq!(topic["free"], 2);
}
