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

/// The mode; the tests of the windows leave textures off (a claim the windows refuse is then denied, as they read), and the test of the textures turns them on.
fn set_mode(server: &Server, mode: Mode) {
    server.eval(move |s| {
        s.vwin_set_mode(mode);
        s.vwin_ctl(&["texture", "off"]);
    });
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
    // the video is asked for in the UI's own pixels, which videooutputd scales to the panel (TV 2026-10-30: LG sends 12,273 944x531 and it applies 24,546 1888x1062;
    // a request in the panel's pixels was applied twice as large)
    let p = last_placement(&server, "_ctx_cam");
    assert_eq!(p["displayOutput"], json!({"x": 1300, "y": 60, "width": 560, "height": 315}), "{p}");
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

/// Home in front, as the retime's rules want it (`retime.rs`: Home in front, nothing else on the main window).
fn home(server: &mut Server) -> Client {
    let mut client = Client::connect_extra(server);
    let win = client.card_as(CARD, "Home", "com.webos.app.home");
    let buf = client.shm_buffer(1920, 1080, 0xffff_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "Home presented");
    std::mem::forget((win, buf));
    client
}

#[test]
fn the_retime_clips_video_is_placed_under_the_ui_with_no_hole_and_hidden_again_when_the_clip_ends() {
    let mut server = Server::start();
    let _home = home(&mut server);
    // the player the compositor starts is a script that waits; the clip is written to a directory of the test's
    let dir = std::env::temp_dir().join(format!("lumatv-retime-window-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("feeder");
    std::fs::write(&program, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&program, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let (runtime, scaler) = (dir.clone(), dir.join("no-scaler"));
    server.eval(move |s| {
        s.config.runtime_dir = runtime;
        s.retime.enabled = true;
        s.retime.program = program;
        s.retime.scaler = scaler;
    });
    // the window the compositor's own player opens: the app id it is placed by, with its element and the pipeline's punch-through
    let mut client = Client::connect_extra(&mut server);
    let win = client.card_as(VIDEO, "LumaTV video", lumatv_compositor::retime::APP_ID);
    let buf = client.shm_pattern(1920, 1080, |x, y| if x < 16 && y < 16 { 0xff00_ff00 } else { 0 });
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "the clip's window presented");
    let el = export(&mut client, &win.surface, "_ctx_retime");
    std::mem::forget(buf);
    // no clip yet: the window is hidden, nothing is placed
    let image = server.eval(drawn);
    assert_eq!(image.pixel(5, 5), RED, "Home, whole: the window of no clip is not drawn");
    assert!(server.eval(placements).is_empty());

    let reply = server.eval(|s| s.retime_ctl(&["now"]));
    assert_eq!(reply["ok"], true, "{reply}");
    // placed at the clip's rectangle, in the UI's own pixels, and nothing of the UI is cut away for it or drawn over it
    let p = last_placement(&server, "_ctx_retime");
    assert_eq!(p["displayOutput"], json!({"x": 16, "y": 16, "width": 128, "height": 72}), "{p}");
    assert_eq!(p["fullScreen"], false);
    let image = server.eval(drawn);
    assert_eq!(image.pixel(20, 20), RED, "no hole: Home covers the video");
    assert_eq!(image.pixel(5, 5), RED, "and the window is not in the stack");
    assert_eq!(server.eval(|s| s.scene.holes.len()), 0);
    let report = server.eval(|s| s.vwin_report());
    assert_eq!((report["windows"][0]["quiet"].as_bool(), report["windows"][0]["live"].as_bool()), (Some(true), Some(false)), "{report}");

    // shown (the escape hatch for a decoder that wants a hole): a live window's hole at the same place
    server.eval(|s| s.retime_ctl(&["show"]));
    let image = server.eval(drawn);
    assert_eq!(image.pixel(20, 20), HOLE, "the hole");
    assert_eq!(image.pixel(5, 5), GREEN, "the window is in the stack");
    assert_eq!(image.pixel(150, 100), RED, "outside the hole, Home");
    server.eval(|s| s.retime_ctl(&["hide"]));
    assert_eq!(server.eval(drawn).pixel(20, 20), RED);

    // the clip ends: the video is hidden (a placement of nothing) and the window is left as it was before
    server.eval(|s| s.retime_ctl(&["stop"]));
    assert_eq!(last_placement(&server, "_ctx_retime")["displayOutput"]["width"], 0, "hidden");
    assert_eq!(server.eval(drawn).pixel(20, 20), RED);
    assert_eq!(server.eval(|s| s.scene.holes.len()), 0);
    drop((win, el));
    let _ = std::fs::remove_dir_all(&dir);
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
    assert_eq!(last_placement(&server, "_ctx_cam")["displayOutput"], json!({"x": 100, "y": 500, "width": 480, "height": 270}));
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
    // The grace (three seconds on the monotonic clock) starts when the loop first sees the unclaimed window; its start is put back past it.
    let mut aged = false;
    for _ in 0..400 {
        aged = server.eval(|s| {
            let ids: Vec<_> = s.scene.windows().filter(|w| w.vwin_seen_ns != 0).map(|w| w.id).collect();
            for id in &ids {
                s.scene.window_mut(*id).unwrap().vwin_seen_ns -= 4_000_000_000;
            }
            !ids.is_empty()
        });
        if aged {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(aged, "the loop saw the window");
    assert!(feeder.client.wait_until(WAIT, |s| s.closes >= 1), "the window was closed once the grace was over");
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

/// What the feeder does for a texture: a video window that starts 16 pixels square and empty, then paints a buffer of its own size (here a solid colour).
fn texture_feeder(server: &mut Server, app: &str) -> (Client, harness::Window) {
    let mut client = Client::connect_extra(server);
    let win = client.card_as(VIDEO, "Cameras", app);
    let empty = client.shm_pattern(16, 16, |_, _| 0);
    client.present(&win.surface, &empty);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "the video window presented");
    std::mem::forget(empty);
    (client, win)
}

#[test]
fn a_texture_claim_beside_an_app_that_plays_is_the_feeders_buffer_stretched_over_the_claims_rectangle() {
    let mut server = Server::start();
    let _bg = app(&mut server, "Bg", 0xffff_0000);
    set_mode(&server, Mode::Idle);
    // an app plays video: the windows are not for LumaTV's
    let mut player = Client::connect_extra(&mut server);
    let pwin = player.card(CARD, "Player");
    let pbuf = player.shm_buffer(1920, 1080, 0xff20_2020);
    player.present(&pwin.surface, &pbuf);
    assert!(player.wait_until(WAIT, |s| s.frames_done >= 1));
    let _el = export(&mut player, &pwin.surface, "_ctx_app");
    server.eval(|s| s.vwin_ctl(&["texture", "on"]));
    let placed_before = server.eval(placements).len();

    // the pop-up asks: no window, a texture; the full view is not given one
    let reply = ask(&server, claim("pop", "camera-popup", "org.test.cameras", POPUP));
    assert_eq!(state_of(&reply), ("granted".to_string(), String::new()), "{reply}");
    assert_eq!((reply["claim"]["slot"].as_str(), reply["claim"]["texture"].as_bool()), (Some("texture"), Some(true)), "{reply}");
    assert_eq!(reply["claim"]["limits"], json!({"w": 1280, "h": 720, "fps": 30}));
    assert_eq!(state_of(&ask(&server, claim("full", "camera-full", "org.test.cameras", [0, 0, 1920, 1080]))), ("denied".to_string(), "app-video".to_string()));

    // the feeder's window comes (16 x 16, empty): bound to the claim, drawn as nothing, not yet placed
    let (mut feeder, fwin) = texture_feeder(&mut server, "org.test.cameras");
    server.eval(|s| s.vwin_sync());
    let r = server.eval(|s| s.vwin_report());
    assert_eq!(r["claims"][0]["placed"], false, "nothing is painted yet: {r}");
    assert_ne!(server.eval(drawn).pixel(1310, 70), GREEN);

    // it paints 280 x 158 in green: that is the claim's rectangle, stretched; the rest of the screen is the app's
    let green = feeder.shm_buffer(280, 158, 0xff00_ff00);
    feeder.present(&fwin.surface, &green);
    assert!(feeder.wait_until(WAIT, |s| s.frames_done >= 2));
    server.eval(|s| s.vwin_sync());
    let image = server.eval(drawn);
    let [x, y, w, h] = POPUP.map(|v| v as u32);
    for (px, py) in [(x, y), (x + w - 1, y), (x, y + h - 1), (x + w - 1, y + h - 1), (x + w / 2, y + h / 2)] {
        assert_eq!(image.pixel(px, py), GREEN, "the picture is over the claim's rectangle at {px},{py}");
    }
    for (px, py) in [(x - 1, y + 5), (x + 5, y - 1), (x + w, y + 5), (x + 5, y + h)] {
        assert_ne!(image.pixel(px, py), GREEN, "and not beyond it at {px},{py}");
    }
    let r = server.eval(|s| s.vwin_report());
    assert_eq!((r["claims"][0]["placed"].as_bool(), r["claims"][0]["sink"].as_str()), (Some(true), Some("texture")), "{r}");
    assert_eq!(server.eval(placements).len(), placed_before, "no video was placed: a texture is not on the video plane");

    // the claim moves: the same buffer goes with it
    let r = ask(&server, json!({"op": "move", "id": "pop", "rect": {"x": 100, "y": 200, "w": 400, "h": 225}}));
    assert_eq!(r["ok"], true, "{r}");
    let image = server.eval(drawn);
    assert_eq!(image.pixel(100, 200), GREEN);
    assert_eq!(image.pixel(499, 424), GREEN);
    assert_ne!(image.pixel(x + 5, y + 5), GREEN);

    // `ctl vwin texture off` ends it: the window closes and nothing of it is drawn
    let r = server.eval(|s| s.vwin_ctl(&["texture", "off"]));
    assert_eq!(r["ok"], true, "{r}");
    assert!(feeder.wait_until(WAIT, |s| s.closes >= 1), "the window was told to close");
    assert_ne!(server.eval(drawn).pixel(150, 250), GREEN);
    let r = server.eval(|s| s.vwin_report());
    assert_eq!((r["claims"][0]["state"].as_str(), r["claims"][0]["reason"].as_str()), (Some("preempted"), Some("off")), "{r}");
    std::mem::forget((pwin, pbuf, green, fwin));
}

fn foreground(server: &Server) -> Vec<Value> {
    let info = server.eval(|s| s.luna_method("com.webos.surfacemanager", "getForegroundAppInfo", &json!({}), "com.webos.applicationManager").unwrap());
    info["foregroundAppInfo"].as_array().unwrap().clone()
}

#[test]
fn the_foreground_list_stays_the_front_cards_while_a_claim_holds_the_sub_window() {
    let mut server = Server::start();
    set_mode(&server, Mode::Apps);
    let mut player = Client::connect_extra(&mut server);
    let win = player.card(CARD, "Player");
    let buf = player.shm_buffer(1920, 1080, 0xff20_2020);
    player.present(&win.surface, &buf);
    assert!(player.wait_until(WAIT, |s| s.frames_done >= 1));
    let _el = export(&mut player, &win.surface, "_ctx_app");
    let before = foreground(&server);
    assert_eq!((before.len(), before[0]["multiview"].as_bool(), before[0]["viewType"].as_str()), (1, Some(false), Some("normal")), "{before:?}");

    // the app's video holds MAIN and a claim is granted the sub window: its pipeline is opened for the app in front, which SAM lists already, so
    // nothing is added to the list (a second entry would make SAM, the Multi View controller and the memory manager count it as an app in front)
    let status = json!({"video": [
        {"sink": "MAIN", "connected": true, "context": "_ctx_app", "appId": "netflix", "zOrder": 0, "width": 1920, "height": 1080, "frameRate": 30, "videoInfo": {}},
        {"sink": "SUB0", "connected": false, "context": "", "zOrder": 1, "videoInfo": {}}]});
    server.eval(move |s| s.vwin_status(&status));
    let reply = ask(&server, claim("cam", "camera-popup", "org.test.cameras", POPUP));
    assert_eq!(reply["claim"]["slot"], "sub", "{reply}");
    assert_eq!(foreground(&server), before);
    let r = server.eval(|s| s.vwin_ctl(&["fg", "org.test.sub"]));
    assert_eq!(r["ok"], false, "the switch that listed a second app is gone: {r}");
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
fn ctl_page_layout_is_refused_where_there_are_no_pages_or_the_holes_would_lose_their_alpha() {
    let server = Server::start();
    let ctl = |server: &Server, line: &'static str| -> Value { serde_json::from_str(&server.eval(move |s| control::handle(s, line).text())).unwrap() };
    let r = ctl(&server, "afbc on");
    assert_eq!(r["ok"], false, "the host output has no scan-out pages to convert: {r}");
    assert!(r["error"].as_str().unwrap().contains("no scan-out pages"), "{r}");
    assert_eq!(ctl(&server, "afbc dance")["ok"], false);
    set_mode(&server, Mode::Idle);
    let r = ctl(&server, "pagealpha off");
    assert_eq!((r["ok"].as_bool(), r["error"].as_str().is_some_and(|e| e.contains("need alpha"))), (Some(false), true), "{r}");
}

#[test]
fn the_shell_asks_through_its_channel_and_reads_the_answer_in_the_topic() {
    let mut server = Server::start();
    server.eval(|s| { s.shell_rust_open(0, "test".into()).unwrap(); s.shell_rust_handle(Msg::Connected); });
    let _app = app(&mut server, "App", 0xffff_0000);
    set_mode(&server, Mode::Apps);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    server.eval(|s| s.shell_rust_handle(Msg::Vwin(claim("cam", "camera-popup", "org.test.cameras", POPUP))));
    let topic = server.eval(|s| s.shell_rust.topic("vwin").cloned()).expect("the topic is published");
    assert_eq!(topic["mode"], "apps");
    assert_eq!(topic["claims"][0]["id"], "cam");
    assert_eq!(topic["claims"][0]["state"], "granted", "{topic}");
    assert_eq!(topic["claims"][0]["placed"], true);
    assert_eq!(topic["free"], 1);
    server.eval(|s| s.shell_rust_handle(Msg::Vwin(json!({"op": "release", "id": "cam"}))));
    let topic = server.eval(|s| s.shell_rust.topic("vwin").cloned()).unwrap();
    assert_eq!(topic["claims"].as_array().map(Vec::len), Some(0), "{topic}");
    assert_eq!(topic["free"], 2);
}

/// An element nobody plays on: exported, with no pipeline's punch-through attached (what LG's Home and an app that waits to play keep).
fn export_idle(client: &mut Client, surface: &WlSurface) -> WlWebosExported {
    let exported = client.foreign.export_element(surface, 0, &client.qh, ());
    client.roundtrip();
    exported
}

/// A card of the app `app`, in one colour, full screen: it is the one in front when it comes.
fn card_of(server: &mut Server, title: &str, app: &str) -> (Client, harness::Window, wayland_client::protocol::wl_buffer::WlBuffer) {
    let mut client = Client::connect_extra(server);
    let win = client.card_as(CARD, title, app);
    let buf = client.shm_buffer(1920, 1080, 0xff10_1010);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "{title} presented");
    (client, win, buf)
}

#[test]
fn an_element_nobody_plays_on_denies_idle_mode_only_in_the_window_in_front_and_not_in_home() {
    let mut server = Server::start();
    set_mode(&server, Mode::Idle);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    // LG's Home keeps a video element that holds no decoder: TV 2026-10-29 every claim over Home was denied "app-video" for it
    let (mut home, hw, hb) = card_of(&mut server, "Home", "com.webos.app.home");
    let _home_el = export_idle(&mut home, &hw.surface);
    let r = server.eval(|s| s.vwin_report());
    assert_eq!((r["world"]["appElements"].as_i64(), r["world"]["appPlaying"].as_i64()), (Some(0), Some(0)), "{r}");
    assert_eq!(r["elements"][0]["counted"], false, "{r}");
    let reply = ask(&server, claim("home", "camera-popup", "", POPUP));
    assert_eq!(state_of(&reply), ("granted".to_string(), String::new()), "{reply}");
    // an app comes to the front with an element nothing plays on yet: the video it is about to play owns the plane
    let (mut player, pw, pb) = card_of(&mut server, "Player", "org.test.player");
    let _player_el = export_idle(&mut player, &pw.surface);
    let r = server.eval(|s| s.vwin_report());
    assert_eq!((r["world"]["appElements"].as_i64(), r["world"]["appPlaying"].as_i64()), (Some(1), Some(0)), "{r}");
    let counted: Vec<_> = r["elements"].as_array().unwrap().iter().filter(|e| e["counted"] == true).map(|e| e["app"].clone()).collect();
    assert_eq!(counted, [json!("org.test.player")], "{r}");
    assert_eq!(r["claims"][0]["reason"], "app-video", "the granted claim ended: {r}");
    assert_eq!(state_of(&ask(&server, claim("over", "camera-popup", "", POPUP))), ("denied".to_string(), "app-video".to_string()));
    // another card comes to the front: the player's idle element is behind it and holds no decoder
    let (_other, _ow, _ob) = card_of(&mut server, "Other", "org.test.other");
    let r = server.eval(|s| s.vwin_report());
    assert_eq!(r["world"]["appElements"], 0, "{r}");
    assert_eq!(state_of(&ask(&server, claim("after", "camera-popup", "", POPUP))).0, "granted");
    std::mem::forget((hw, hb, pw, pb, _home_el, _player_el));
}

#[test]
fn a_claim_takes_the_wallpapers_window_though_another_is_free() {
    // LumaTV's own sessions of the decoder hold both scalers: a second does not start beside the wallpaper's
    let mut server = Server::start();
    set_mode(&server, Mode::Idle);
    let mut wall = Client::connect_extra(&mut server);
    let win = wall.card(WALLPAPER, "Wall");
    let buf = wall.shm_buffer(1920, 1080, 0xff00_00ff);
    wall.present(&win.surface, &buf);
    assert!(wall.wait_until(WAIT, |s| s.frames_done >= 1));
    let _wall_el = export(&mut wall, &win.surface, "_ctx_wall");
    let r = server.eval(|s| s.vwin_report());
    assert_eq!((r["wall"].as_bool(), r["free"].as_i64()), (Some(true), Some(1)), "the wallpaper holds one of two windows: {r}");
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    let reply = ask(&server, claim("cam", "camera-popup", "", POPUP));
    assert_eq!((state_of(&reply).0.as_str(), reply["claim"]["slot"].as_str()), ("granted", Some("main")), "the wallpaper's window: {reply}");
    assert!(wall.wait_until(WAIT, |s| s.closes >= 1), "the wallpaper's window was closed");
    std::mem::forget((win, buf, _wall_el));
}

/// What videooutputd's status says of one sink: the video of `ctx`, connected, where it is shown (`dest`: x, y, width, height in the panel's pixels) and whether it
/// was placed full screen.
fn sink_with(ctx: &str, dest: [i64; 4], full_screen: bool) -> Value {
    json!({"video": [
        {"sink": "MAIN", "connected": true, "context": ctx, "appId": "com.webos.app.home", "zOrder": 0, "width": 1280, "height": 720, "frameRate": 30, "fullScreen": full_screen,
         "displayOutput": {"x": dest[0], "y": dest[1], "width": dest[2], "height": dest[3]}, "videoInfo": {"subScalerOccupied": true}},
        {"sink": "SUB0", "connected": false, "context": "", "zOrder": 1, "videoInfo": {}}]})
}

fn sink_at(ctx: &str, dest: [i64; 4]) -> Value {
    sink_with(ctx, dest, false)
}

fn placed_again(server: &Server) -> Vec<String> {
    server.eval(|s| s.journal.clone()).into_iter().filter(|l| l.contains("placed again (")).collect()
}

/// How long the compositor waits after a placement before it believes the sink's answer to it: a test cannot wait for it on a loaded machine, and the loop
/// looks at every frame, so a test shuts the door (the wait is long) and opens it for one answer at a time.
const DOOR_SHUT_NS: u64 = 600_000_000_000;

fn shut(server: &Server) {
    server.eval(|s| s.vwin.settle_ns = DOOR_SHUT_NS);
}

/// The sink's answer, believed once and now: judged inside one step of the compositor, so that no frame of its own loop can judge it twice.
fn believed(server: &Server, status: Value) {
    server.eval(move |s| {
        s.vwin.settle_ns = 0;
        s.vwin_status(&status);
        s.vwin.settle_ns = DOOR_SHUT_NS;
    });
}

#[test]
fn a_video_that_videooutputd_holds_elsewhere_is_placed_again_where_its_claim_says() {
    let mut server = Server::start();
    set_mode(&server, Mode::Idle);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    let reply = ask(&server, claim("cam", "camera-popup", "", POPUP));
    assert_eq!(state_of(&reply).0, "granted", "{reply}");
    shut(&server);
    let placed = server.eval(placements).len();
    // connected, and where it was asked for (the panel is twice the UI: 1300,60 560x315 is 2600,120 1120x630 there): nothing to do
    believed(&server, sink_at("_ctx_cam", [2600, 120, 1120, 630]));
    assert_eq!(server.eval(placements).len(), placed, "where it was asked for: left alone");
    assert!(placed_again(&server).is_empty());
    // connected, and somewhere else (the old reading of the bug: parked at the panel's corner): placed again at once, in the UI's pixels again
    believed(&server, sink_at("_ctx_cam", [3840, 2160, 0, 0]));
    let after = server.eval(placements);
    assert_eq!(after.len(), placed + 1, "one more placement: {after:?}");
    assert_eq!(after.last().unwrap()["displayOutput"], json!({"x": 1300, "y": 60, "width": 560, "height": 315}));
    // asking again before the answer to the placement can be believed is not answered with another
    server.eval(|s| s.vwin_status(&sink_at("_ctx_cam", [3840, 2160, 1, 0])));
    assert_eq!(server.eval(placements).len(), placed + 1, "not twice in a breath");
    // where the claim says (a pixel or two off is rounding): let go
    believed(&server, sink_at("_ctx_cam", [2601, 120, 1120, 629]));
    // (the count starts over: the claim whose video was where it was told is let go of)
    believed(&server, sink_at("_ctx_cam", [3840, 2160, 0, 0]));
    let again = placed_again(&server);
    assert_eq!(again.len(), 2, "{again:?}");
    assert!(again.iter().all(|l| l.contains("(1)")), "the count starts over once the video has been where it was told: {again:?}");
    let r = server.eval(|s| s.vwin_report());
    assert_eq!(r["sinks"][0]["displayOutput"], json!([3840, 2160, 0, 0]), "{r}");
    assert_eq!(r["sinks"][0]["fullScreen"], false, "{r}");
    assert_eq!(r["replaced"], json!({"cam": 1}), "{r}");
}

#[test]
fn a_video_that_stays_where_it_is_after_two_placements_is_left_and_the_journal_says_so_once() {
    // TV 2026-10-30: the full view's video on the sink the camera pop-up had was "held" at the pop-up's rectangle and placed again twelve times
    let mut server = Server::start();
    set_mode(&server, Mode::Idle);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    assert_eq!(state_of(&ask(&server, claim("cam", "camera-popup", "", POPUP))).0, "granted");
    shut(&server);
    let placed = server.eval(placements).len();
    for i in 0..8 {
        believed(&server, sink_at("_ctx_cam", [3840, 988, 0, 1168 + i]));
    }
    assert_eq!(server.eval(placements).len(), placed + 2, "placed again twice, never more");
    let journal = server.eval(|s| s.journal.clone());
    let left: Vec<_> = journal.iter().filter(|l| l.contains("left as it is")).collect();
    assert_eq!(left.len(), 1, "said once: {journal:?}");
    assert_eq!(placed_again(&server).len(), 2);
    // the claim moves: it is a new question, with its two placements (the sink's answer at the move is the old one, and is not believed)
    let reply = ask(&server, json!({"op": "move", "id": "cam", "rect": {"x": 100, "y": 500, "w": 480, "h": 270}}));
    assert_eq!(reply["ok"], true, "{reply}");
    let placed = server.eval(placements).len();
    for i in 0..4 {
        believed(&server, sink_at("_ctx_cam", [3840, 988, 0, 1200 + i]));
    }
    assert_eq!(server.eval(placements).len(), placed + 2, "two more for the new rectangle");
    assert_eq!(placed_again(&server).len(), 4);
    assert_eq!(server.eval(|s| s.vwin_report())["replaced"], json!({"cam": 2}), "the report says how often");
    // and a claim that is let go of leaves no count behind
    ask(&server, json!({"op": "release", "id": "cam"}));
    server.eval(|s| s.vwin_status(&sink_at("_ctx_cam", [3840, 988, 0, 1999])));
    assert_eq!(server.eval(|s| s.vwin_report())["replaced"], json!({}));
}

#[test]
fn a_full_screen_claim_is_where_it_says_when_the_sink_says_full_screen_whatever_rectangle_it_held_before() {
    // the camera pop-up's window on the sink, then the full view's: the sink's `displayOutput` may stay the pop-up's
    let mut server = Server::start();
    set_mode(&server, Mode::Idle);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    assert_eq!(state_of(&ask(&server, claim("cam", "camera-popup", "", POPUP))).0, "granted");
    shut(&server);
    ask(&server, json!({"op": "release", "id": "cam"}));
    let reply = ask(&server, claim("full", "camera-full", "", [0, 0, 1920, 1080]));
    assert_eq!(state_of(&reply).0, "granted", "{reply}");
    let p = last_placement(&server, "_ctx_cam");
    assert_eq!(
        (p["fullScreen"].as_bool(), &p["displayOutput"]),
        (Some(true), &json!({"x": 0, "y": 0, "width": 3840, "height": 2160})),
        "the panel's whole size: {p}"
    );
    let placed = server.eval(placements).len();
    believed(&server, sink_with("_ctx_cam", [2600, 120, 1120, 630], true));
    believed(&server, sink_with("_ctx_cam", [2600, 120, 1120, 631], true));
    assert_eq!(server.eval(placements).len(), placed, "full screen by its own flag: not placed again");
    assert!(placed_again(&server).is_empty());
    // not full screen yet (the sink says so): placed again, as for any other claim
    believed(&server, sink_with("_ctx_cam", [2600, 120, 1120, 630], false));
    assert_eq!(server.eval(placements).len(), placed + 1);
    // and at the panel's whole size it is where it says, flag or not
    believed(&server, sink_with("_ctx_cam", [0, 0, 3840, 2160], false));
    assert_eq!(server.eval(placements).len(), placed + 1);
}

#[test]
fn a_sink_is_not_believed_in_the_first_moments_after_a_placement_and_a_video_that_connects_where_it_was_asked_for_is_never_placed_again() {
    // the real wait (0.7 s) is kept: the claim is placed, the sink shows an older answer for a while, and the answer that comes after is the one that counts
    let mut server = Server::start();
    set_mode(&server, Mode::Idle);
    let _feeder = feeder(&mut server, "Cameras", "_ctx_cam");
    assert_eq!(state_of(&ask(&server, claim("cam", "camera-popup", "", POPUP))).0, "granted");
    assert_eq!(server.eval(|s| s.vwin.settle_ns), 700_000_000, "the shipped wait");
    let placed = server.eval(placements).len();
    for i in 0..5 {
        server.eval(move |s| s.vwin_status(&sink_at("_ctx_cam", [3840, 988, 0, 1168 + i])));
    }
    assert_eq!(server.eval(placements).len(), placed, "a sink that was seen a moment ago is not judged yet");
    server.eval(|s| s.vwin_status(&sink_at("_ctx_cam", [2600, 120, 1120, 630])));
    assert!(placed_again(&server).is_empty());
    assert_eq!(server.eval(|s| s.vwin_report())["replaced"], json!({}));
}

#[test]
fn the_mode_that_ctl_vwin_sets_is_kept_for_the_next_start_and_idle_is_the_default() {
    let dir = std::env::temp_dir().join(format!("lumatv-vwin-mode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(lumatv_compositor::vwin::read_mode(&dir), None);
    assert_eq!(lumatv_compositor::vwin::default_mode(&dir), Mode::Idle);
    lumatv_compositor::vwin::write_mode(&dir, Mode::Apps).unwrap();
    assert_eq!(lumatv_compositor::vwin::default_mode(&dir), Mode::Apps);
    std::fs::write(dir.join(lumatv_compositor::vwin::MODE_FILE), "banana\n").unwrap();
    assert_eq!(lumatv_compositor::vwin::default_mode(&dir), Mode::Idle, "a file that says nothing known is no word");
    // and `ctl vwin apps` writes it, in the data directory the server runs with
    let server = Server::start();
    let ctl = |server: &Server, line: &'static str| -> Value { serde_json::from_str(&server.eval(move |s| control::handle(s, line).text())).unwrap() };
    ctl(&server, "vwin apps");
    let kept = server.eval(|s| lumatv_compositor::vwin::read_mode(&s.config.data_dir));
    assert_eq!(kept, Some(Mode::Apps));
    let _ = std::fs::remove_dir_all(&dir);
}
