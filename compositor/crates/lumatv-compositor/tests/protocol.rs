//! Protocol tests: a real `wayland-client` against the compositor over a socketpair, software renderer.

mod harness;

use std::time::Duration;

use harness::{Client, Server};
use lumatv_compositor::window::{ShellState, WindowType};
use lumatv_wl_protocols::mali_buffer_sharing::client::mali_buffer_sharing::MaliBufferSharing;
use wayland_client::Proxy;

const WAIT: Duration = Duration::from_secs(3);

#[test]
fn advertises_the_globals_lg_clients_bind() {
    let mut server = Server::start();
    let client = Client::connect(&mut server);
    let names = client.global_names();
    for needed in [
        "wl_compositor",
        "wl_subcompositor",
        "wl_shm",
        "wl_shell",
        "wl_seat",
        "wl_output",
        "wl_data_device_manager",
        "wl_webos_shell",
        "wl_webos_surface_group_compositor",
        "wl_webos_foreign",
        "wl_webos_input_manager",
        "wl_webos_xinput_extension",
        "text_model_factory",
        "input_method",
        "input_panel",
        "wl_webos_system_ux",
        "zwp_linux_dmabuf_v1",
        "mali_buffer_sharing",
    ] {
        assert!(names.iter().any(|n| n == needed), "missing global {needed}; have {names:?}");
    }
    assert!(!names.iter().any(|n| n == "wl_starfish_pointer"), "starfish must stay off until its tables are verified");
}

#[test]
fn shm_card_window_reaches_the_output_pixel_exact() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "First");
    client.roundtrip();
    assert_eq!(client.state.configures, vec![(1920, 1080)], "set_toplevel configures a card to the output size");

    let buffer = client.shm_buffer(1920, 1080, 0xff80_00ff); // opaque: R 0x80, G 0x00, B 0xff
    client.present(&win.surface, &buffer);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "a frame callback follows the first presented frame");
    assert!(client.wait_until(WAIT, |s| s.shell_states.contains(&(ShellState::Fullscreen as u32))), "a card is told it is fullscreen on map");
    assert_eq!(client.state.positions, vec![(0, 0)]);

    let (mapped, kind, title, app) = server.eval(|s| {
        let w = s.scene.windows().next().expect("a window");
        (s.surfaces.get(w.surface).unwrap().is_mapped(), w.kind, w.title.clone(), w.app_id().to_string())
    });
    assert!(mapped);
    assert_eq!(kind, WindowType::Card);
    assert_eq!(title, "First");
    assert_eq!(app, "org.test.first");

    let image = server.eval(|s| s.output.capture().unwrap());
    assert_eq!(image.pixel(0, 0), [0x80, 0x00, 0xff, 0xff]);
    assert_eq!(image.pixel(960, 540), [0x80, 0x00, 0xff, 0xff]);
    assert_eq!(image.pixel(1919, 1079), [0x80, 0x00, 0xff, 0xff]);
    let summary = server.summary();
    assert!(summary.contains("drawn 1 ") || summary.contains("drawn 2 "), "one frame drawn, not one per tick: {summary}");
}

#[test]
fn smaller_card_draws_at_the_origin_over_black() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Small");
    let buffer = client.shm_buffer(256, 256, 0xffff_0000);
    client.present(&win.surface, &buffer);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    let image = server.eval(|s| s.output.capture().unwrap());
    assert_eq!(image.pixel(10, 10), [0xff, 0, 0, 0xff]);
    assert_eq!(image.pixel(300, 300), [0, 0, 0, 0xff], "outside the window the output is opaque black");
}

#[test]
fn window_properties_type_and_key_mask_are_kept() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_OVERLAY", "Overlay");
    win.webos.set_property("_WEBOS_ACCESS_POLICY_KEYS_BACK".into(), "true".into());
    win.webos.set_property("displayAffinity".into(), "0".into());
    win.webos.set_property("someFutureProperty".into(), "x".into());
    win.webos.set_key_mask(1 | 2 | 4);
    win.webos.set_location_hint(16);
    win.webos.set_state(1);
    client.roundtrip();
    let (kind, back, unknown, mask, hint, state) = server.eval(|s| {
        let w = s.scene.windows().next().unwrap();
        (w.kind, w.access_policy("_WEBOS_ACCESS_POLICY_KEYS_BACK"), w.props.get("someFutureProperty").cloned(), w.key_mask, w.location_hint, w.state)
    });
    assert_eq!(kind, WindowType::Overlay);
    assert!(back);
    assert_eq!(unknown.as_deref(), Some("x"));
    assert_eq!(mask, 7);
    assert_eq!(hint, 16);
    assert_eq!(state, ShellState::Minimized);
}

#[test]
fn newest_card_is_the_foreground_and_the_older_one_returns_on_close() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let first = client.card("_WEBOS_WINDOW_TYPE_CARD", "A");
    let red = client.shm_buffer(1920, 1080, 0xffff_0000);
    client.present(&first.surface, &red);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));

    let second = client.card("_WEBOS_WINDOW_TYPE_CARD", "B");
    let green = client.shm_buffer(1920, 1080, 0xff00_ff00);
    client.present(&second.surface, &green);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 2));
    let (fg, image) = server.eval(|s| (s.scene.foreground(&s.surfaces).map(|w| w.title.clone()), s.output.capture().unwrap()));
    assert_eq!(fg.as_deref(), Some("B"));
    assert_eq!(image.pixel(5, 5), [0, 0xff, 0, 0xff]);

    // Shell surfaces have no destroy request; the window goes with its surface.
    second.surface.destroy();
    client.roundtrip();
    std::thread::sleep(Duration::from_millis(30));
    let (fg, image) = server.eval(|s| (s.scene.foreground(&s.surfaces).map(|w| w.title.clone()), s.output.capture().unwrap()));
    assert_eq!(fg.as_deref(), Some("A"));
    assert_eq!(image.pixel(5, 5), [0xff, 0, 0, 0xff], "the older card is drawn again once the newer one is gone");
}

#[test]
fn overlay_stacks_above_the_card_at_its_position() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Card");
    let blue = client.shm_buffer(1920, 1080, 0xff00_00ff);
    client.present(&card.surface, &blue);
    let overlay = client.card("_WEBOS_WINDOW_TYPE_OVERLAY", "Toast");
    overlay.webos.set_property("windowPosX".into(), "100".into());
    overlay.webos.set_property("windowPosY".into(), "200".into());
    let white = client.shm_buffer(300, 100, 0xffff_ffff);
    client.present(&overlay.surface, &white);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 2));
    let image = server.eval(|s| s.output.capture().unwrap());
    assert_eq!(image.pixel(50, 50), [0, 0, 0xff, 0xff]);
    assert_eq!(image.pixel(150, 250), [0xff, 0xff, 0xff, 0xff]);
    assert_eq!(image.pixel(100 + 299, 200 + 99), [0xff, 0xff, 0xff, 0xff]);
    assert_eq!(image.pixel(100 + 300, 200 + 100), [0, 0, 0xff, 0xff]);
    assert!(client.state.positions.contains(&(100, 200)));
}

#[test]
fn buffers_are_released_after_the_frame_that_replaced_them() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Swap");
    let a = client.shm_buffer(64, 64, 0xff11_1111);
    let b = client.shm_buffer(64, 64, 0xff22_2222);
    client.present(&win.surface, &a);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    assert!(client.state.releases.is_empty(), "the only buffer stays held while it is on screen");
    client.present(&win.surface, &b);
    assert!(client.wait_until(WAIT, |s| s.releases.contains(&a.id().protocol_id())), "a is released once b is presented");
    assert!(!client.state.releases.contains(&b.id().protocol_id()));
}

#[test]
fn damage_without_a_new_buffer_redraws_only_that_rectangle() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Damage");
    let buf = client.shm_buffer(1920, 1080, 0xff00_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    // Second commit: same buffer, a 4 px damage. The plan's damage rect must be that small.
    win.surface.damage(10, 10, 4, 4);
    win.surface.commit();
    client.conn.flush().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    let plan_damage = server.eval(|s| {
        let State { scene, surfaces, output, .. } = s;
        // Re-plan from the current state without presenting: layout unchanged, so the item's damage is used.
        win_damage(scene, surfaces, output)
    });
    assert!(plan_damage.1 <= 4 * 4 || plan_damage.0, "damage after a 4 px commit is 4 px (or already consumed): {plan_damage:?}");
}

use lumatv_compositor::State;
fn win_damage(scene: &mut lumatv_compositor::scene::Scene, surfaces: &lumatv_compositor::surface::Surfaces, output: &mut Box<dyn lumatv_compositor::output::Output>) -> (bool, i32) {
    let plan = scene.plan(surfaces, |b| output.query_buffer(b).map(|(_, i)| i).unwrap_or(true));
    (plan.damage.is_empty(), plan.damage.w * plan.damage.h)
}

#[test]
fn surface_group_member_draws_above_its_owner() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let owner = client.card("_WEBOS_WINDOW_TYPE_CARD", "Owner");
    let base = client.shm_buffer(1920, 1080, 0xff00_0000);
    client.present(&owner.surface, &base);
    let group = client.groups.create_surface_group(&owner.surface, "test.group".into(), &client.qh, ());
    let layer = group.create_layer("top".into(), 10, &client.qh, ());
    let member_surface = client.compositor.create_surface(&client.qh, ());
    group.attach(&member_surface, "top".into());
    let yellow = client.shm_buffer(200, 200, 0xffff_ff00);
    client.present(&member_surface, &yellow);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 2));
    let (image, items) = server.eval(|s| {
        let State { scene, surfaces, output, .. } = s;
        let plan = scene.plan(surfaces, |_| true);
        (output.capture().unwrap(), plan.items.len())
    });
    assert_eq!(items, 2, "owner and member");
    assert_eq!(image.pixel(100, 100), [0xff, 0xff, 0, 0xff]);
    assert_eq!(image.pixel(500, 500), [0, 0, 0, 0xff]);
    drop(layer);
}

#[test]
fn exported_video_region_becomes_a_hole_and_a_placement_call() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let app = client.card("_WEBOS_WINDOW_TYPE_CARD", "Player");
    let ui = client.shm_buffer(1920, 1080, 0xff20_2020);
    client.present(&app.surface, &ui);
    let exported = client.foreign.export_element(&app.surface, 0, &client.qh, ());
    client.roundtrip();
    assert_eq!(client.state.window_ids.len(), 1);
    let (window_id, t) = client.state.window_ids[0].clone();
    assert!(window_id.starts_with("_Window_Id_"));
    assert_eq!(t, 0);
    let src = client.region(0, 0, 1920, 1080);
    let dst = client.region(160, 90, 960, 540);
    exported.set_exported_window(&src, &dst);
    // The pipeline's side: import by id and attach a punch-through with its context.
    let imported = client.foreign.import_element(window_id.clone(), 0, &client.qh, ());
    imported.attach_punchthrough_with_context("_ctx_1".into());
    client.roundtrip();
    assert!(client.wait_until(WAIT, |s| s.punchthrough_attached == vec!["_ctx_1".to_string()]));
    assert!(client.state.destination_changed.contains(&(960, 540)));
    std::thread::sleep(Duration::from_millis(20));
    let (image, calls, video) = server.eval(|s| {
        let calls: Vec<String> = s.luna_sent().iter().map(|(u, p)| format!("{u} {p}")).collect();
        (s.output.capture().unwrap(), calls, s.foreign.video_windows())
    });
    assert_eq!(image.pixel(10, 10), [0x20, 0x20, 0x20, 0xff], "UI stays");
    assert_eq!(image.pixel(640, 360), [0, 0, 0, 0], "the exported region is a transparent hole");
    assert!(calls.iter().any(|c| c.contains("display/setDisplayWindow") && c.contains("\"width\":960") && c.contains("_ctx_1")), "{calls:?}");
    assert!(calls.iter().any(|c| c.contains("setDisplayWindowProperty") && c.contains("fastMode")), "{calls:?}");
    assert_eq!(video.len(), 1);
    assert_eq!(video[0]["windowId"], window_id);
}

#[test]
fn own_mali_buffer_sharing_takes_a_create_buffer() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    client.roundtrip();
    assert_eq!(client.state.alloc_devices, vec!["/dev/dma_buf_te".to_string()]);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Mali");
    // Stand-in for the dma-buf: any fd works for the protocol; the software renderer cannot read it and skips it.
    let raw = unsafe { libc::memfd_create(c"fake-dmabuf".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(raw >= 0);
    let fd = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(raw) };
    let buffer = mali.create_buffer(1920, 1080, 7680, 0x3432_4241, 0x0800_0000, 0x0000_0151, std::os::fd::AsFd::as_fd(&fd), &client.qh, ());
    mali.auth(42);
    client.present(&win.surface, &buffer);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    let (size, kind, skipped, journal) = server.eval(|s| {
        let w = s.scene.windows().next().unwrap();
        (s.surfaces.get(w.surface).unwrap().size(), format!("{:?}", w.kind), s.stats.skipped_items, s.journal.clone())
    });
    assert_eq!(size, (1920, 1080), "size comes from create_buffer, not from import");
    assert_eq!(kind, "Card");
    assert!(skipped >= 1, "software renderer skips the dma-buf");
    assert!(journal.iter().any(|l| l.contains("modifier 0x0800000000000151")), "{journal:?}");
}

#[test]
fn synchronized_subsurface_waits_for_the_parent_commit() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let parent = client.card("_WEBOS_WINDOW_TYPE_CARD", "Parent");
    let bg = client.shm_buffer(1920, 1080, 0xff00_0000);
    client.present(&parent.surface, &bg);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    let child = client.compositor.create_surface(&client.qh, ());
    let sub = client.subcompositor.get_subsurface(&child, &parent.surface, &client.qh, ());
    sub.set_position(300, 300);
    let cyan = client.shm_buffer(100, 100, 0xff00_ffff);
    child.attach(Some(&cyan), 0, 0);
    child.damage(0, 0, 100, 100);
    child.commit();
    client.roundtrip();
    std::thread::sleep(Duration::from_millis(20));
    let before = server.eval(|s| s.output.capture().unwrap());
    assert_eq!(before.pixel(350, 350), [0, 0, 0, 0xff], "cached until the parent commits");
    parent.surface.commit();
    client.roundtrip();
    std::thread::sleep(Duration::from_millis(30));
    let after = server.eval(|s| s.output.capture().unwrap());
    assert_eq!(after.pixel(350, 350), [0, 0xff, 0xff, 0xff]);
    assert_eq!(after.pixel(299, 299), [0, 0, 0, 0xff]);
}

#[test]
fn keyboard_focus_enters_the_mapped_card_and_keys_reach_it() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let _kb = client.seat.get_keyboard(&client.qh, ());
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Focus");
    let buf = client.shm_buffer(64, 64, 0xff00_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.keyboard_enters >= 1), "enter follows the map");
    let delivered = server.eval(|s| lumatv_compositor::server::seat::send_key(s, 412, true) && lumatv_compositor::server::seat::send_key(s, 412, false));
    assert!(delivered);
    assert!(client.wait_until(WAIT, |s| s.keys.len() >= 2));
    assert_eq!(client.state.keys[0], (412, 1));
    assert_eq!(client.state.keys[1], (412, 0));
}
