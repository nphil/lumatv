//! The OLED care on the scene (`oledcare`, `scene.rs`): LumaTV's two shell windows orbit and dim, an app's card does not, nothing
//! happens while it is off, a beat on a still screen is a frame of its own that the loop may call quiet, and the software renderer
//! draws the dim.

mod harness;

use std::sync::atomic::Ordering;

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::oledcare::GRACE_NS;
use lumatv_compositor::output::monotonic_ns;
use lumatv_compositor::scene::Plan;
use lumatv_compositor::window::WindowType;
use wayland_client::protocol::wl_buffer::WlBuffer;

const SEC: u64 = 1_000_000_000;

struct Shown {
    _client: Client,
    _win: Window,
    _buf: WlBuffer,
}

fn shown(server: &mut Server, first: bool, kind: &str, title: &str, argb: u32) -> Shown {
    shown_at(server, first, kind, title, argb, (64, 64))
}

fn shown_at(server: &mut Server, first: bool, kind: &str, title: &str, argb: u32, size: (i32, i32)) -> Shown {
    let mut client = if first { Client::connect(server) } else { Client::connect_extra(server) };
    let win = client.card(kind, title);
    win.webos.set_property("appId".into(), "org.nphil.lumatv.shell".into());
    let buf = client.shm_buffer(size.0, size.1, argb);
    client.present(&win.surface, &buf);
    // The loop is not ticking here: the commit is taken in when the server answers, there is no frame to wait for.
    client.roundtrip();
    Shown { _client: client, _win: win, _buf: buf }
}

/// The scene's plan at `t`, as (window kind, x, y, dim) of each item.
fn items(s: &mut State, t: u64) -> Vec<(WindowType, i32, i32, f32)> {
    let State { scene, surfaces, .. } = s;
    let plan: &Plan = scene.plan_at(surfaces, t);
    let ids: Vec<_> = plan.items.iter().map(|i| (i.window, i.rect.x, i.rect.y, i.dim)).collect();
    // what a tick does after its frame: the surfaces' own damage is spent
    surfaces.clear_damage();
    ids.into_iter().map(|(w, x, y, d)| (scene.window(w).unwrap().kind, x, y, d)).collect()
}

fn quiet_server() -> Server {
    let server = Server::start();
    // The loop's own ticks plan with the real clock; these tests plan with theirs.
    server.ticking.store(false, Ordering::SeqCst);
    server
}

#[test]
fn off_by_default_nothing_moves_and_nothing_dims() {
    let mut server = quiet_server();
    let _home = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaHome", 0xff00_00ff);
    let _sys = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "System", 0xff00_ff00);
    server.eval(|s| {
        let base = monotonic_ns();
        for k in 0..100 {
            let got = items(s, base + k * 40 * SEC);
            assert_eq!(got.len(), 2, "Home and the system layer are drawn");
            assert!(got.iter().all(|&(_, x, y, d)| (x, y, d) == (0, 0, 1.0)), "{got:?}");
        }
        assert!(!s.scene.care_frame());
    });
}

#[test]
fn the_shell_windows_orbit_together_one_pixel_a_beat_and_an_app_does_not() {
    let mut server = quiet_server();
    let _home = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaHome", 0xff00_00ff);
    let _sys = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "System", 0xff00_ff00);
    server.eval(|s| {
        s.scene.care.settings.enabled = true;
        s.scene.care.settings.dim_floor = 1.0;
        let base = monotonic_ns();
        // Armed by the first look; nothing at rest happens before the beat is due and its grace is over.
        let first = items(s, base);
        assert!(first.iter().all(|&(_, x, y, _)| (x, y) == (0, 0)));
        let due = base + 30 * SEC;
        items(s, due);
        let held = items(s, due + SEC);
        assert!(held.iter().all(|&(_, x, y, _)| (x, y) == (0, 0)), "due, but the screen is at rest: held for its grace");
        assert!(s.scene.last_plan().damage.is_empty(), "no frame for it yet");
        // The beat: both shell windows one pixel to the left; a frame of its own, with no commit in it.
        let beat = items(s, due + GRACE_NS);
        assert_eq!(beat.len(), 2);
        assert!(beat.iter().all(|&(kind, x, y, _)| matches!(kind, WindowType::ShellHome | WindowType::ShellSystem) && (x, y) == (-1, 0)), "{beat:?}");
        assert!(!s.scene.last_plan().damage.is_empty(), "the step is drawn");
        assert!(s.scene.care_frame(), "and it is one a still screen may call quiet");
        let after = items(s, due + GRACE_NS + SEC);
        assert!(after.iter().all(|&(_, x, y, _)| (x, y) == (-1, 0)));
        assert!(s.scene.last_plan().damage.is_empty(), "then nothing is drawn again");
        assert!(!s.scene.care_frame());
        // A frame that is drawn anyway carries the next beat without waiting for the grace.
        let due2 = due + GRACE_NS + 30 * SEC;
        items(s, due2 - SEC);
        let surfaces_commit = s.scene.windows().map(|w| w.id).collect::<Vec<_>>();
        for id in surfaces_commit {
            s.scene.window_mut(id).unwrap().commits += 1;
        }
        let riding = items(s, due2);
        assert!(riding.iter().all(|&(_, x, y, _)| (x, y) == (-2, 0)), "{riding:?}");
        assert!(!s.scene.care_frame(), "a commit was in that frame: the screen is not at rest after it");
    });
}

#[test]
fn an_apps_card_is_never_moved_or_dimmed_and_the_system_layer_over_it_only_moves() {
    let mut server = quiet_server();
    let _home = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaHome", 0xff00_00ff);
    let _sys = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "System", 0xff00_ff00);
    let mut app = Client::connect_extra(&mut server);
    let card = app.card("_WEBOS_WINDOW_TYPE_CARD", "Video");
    card.webos.set_property("appId".into(), "com.example.video".into());
    let buf = app.shm_buffer(64, 64, 0xff11_2233);
    app.present(&card.surface, &buf);
    app.roundtrip();
    server.eval(|s| {
        s.scene.care.settings.enabled = true;
        s.scene.care.settings.dim_after_ns = SEC;
        s.scene.care.settings.dim_over_ns = SEC;
        s.scene.care.settings.dim_floor = 0.8;
        let mut t = monotonic_ns();
        items(s, t);
        let mut got = Vec::new();
        for _ in 0..80 {
            t += 30 * SEC + GRACE_NS;
            got = items(s, t);
        }
        let shell: Vec<_> = got.iter().filter(|i| matches!(i.0, WindowType::ShellHome | WindowType::ShellSystem)).collect();
        let others: Vec<_> = got.iter().filter(|i| !matches!(i.0, WindowType::ShellHome | WindowType::ShellSystem)).collect();
        // Home is under the card, so only the card and the system layer are in the stack.
        assert!(!others.is_empty(), "the card is drawn: {got:?}");
        assert!(others.iter().all(|&&(_, x, y, d)| (x, y, d) == (0, 0, 1.0)), "{others:?}");
        assert!(!shell.is_empty() && shell.iter().all(|&&(_, _, _, d)| d == 1.0), "the system layer is not dimmed: {shell:?}");
        assert!(shell.iter().all(|&&(_, x, y, _)| x.abs() <= 3 && y.abs() <= 3));
    });
}

#[test]
fn a_key_brings_the_dim_back_and_a_still_home_goes_to_the_plane_whole_only_while_it_is_bright_and_unshifted() {
    let mut server = quiet_server();
    let _home = shown_at(&mut server, true, "_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaHome", 0xff00_00ff, (1920, 1080));
    server.eval(|s| {
        s.scene.care.settings.enabled = true;
        s.scene.care.settings.radius = 0;
        s.scene.care.settings.dim_after_ns = SEC;
        s.scene.care.settings.dim_over_ns = SEC;
        s.scene.care.settings.dim_floor = 0.8;
        let mut t = monotonic_ns();
        items(s, t);
        assert!(s.scene.last_plan().sole_fullscreen().is_some(), "bright: direct scan-out");
        for _ in 0..4 {
            t += 30 * SEC + GRACE_NS;
            items(s, t);
        }
        let d = items(s, t + 1);
        assert_eq!(d[0].3, 0.8);
        assert!(s.scene.last_plan().sole_fullscreen().is_none(), "a dimmed Home is composed, not scanned out whole");
        s.scene.care.input(t + 2);
        let d = items(s, t + 3);
        assert_eq!(d[0].3, 1.0, "a key");
        assert!(!s.scene.last_plan().damage.is_empty(), "the brightness coming back is drawn");
        assert!(s.scene.last_plan().sole_fullscreen().is_some());
    });
}

#[test]
fn the_software_renderer_draws_the_shift_and_the_dim() {
    let mut server = quiet_server();
    let _home = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaHome", 0xffff_0000);
    server.eval(|s| {
        s.scene.care.settings.enabled = true;
        s.scene.care.settings.radius = 3;
        s.scene.care.settings.dim_after_ns = SEC;
        s.scene.care.settings.dim_over_ns = SEC;
        s.scene.care.settings.dim_floor = 0.8;
        let mut t = monotonic_ns();
        {
            let State { scene, surfaces, output, .. } = &mut *s;
            let plan = scene.plan_at(surfaces, t);
            output.render(plan, surfaces).unwrap();
        }
        for _ in 0..3 {
            t += 30 * SEC + GRACE_NS;
            let State { scene, surfaces, output, .. } = &mut *s;
            let plan = scene.plan_at(surfaces, t);
            output.render(plan, surfaces).unwrap();
        }
        let img = s.output.capture().unwrap();
        let px = |x: usize, y: usize| {
            let i = (y * img.width as usize + x) * 4;
            [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
        };
        // Three beats: three pixels left; the 64 px square now ends at x 61. Red at 0.8 is 204.
        assert_eq!(px(0, 0)[0], 204, "dimmed: {:?}", px(0, 0));
        assert_eq!(px(0, 0)[1..3], [0, 0]);
        assert_eq!(px(60, 0)[0], 204);
        assert_eq!(px(61, 0), [0, 0, 0, 255], "the shifted edge shows the black behind");
    });
}
