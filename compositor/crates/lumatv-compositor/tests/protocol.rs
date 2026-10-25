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
        // Re-plan from the current state without presenting: layout unchanged, so the item's damage is used.
        let plan = s.scene.plan(&s.surfaces);
        (plan.damage.is_empty(), plan.damage.w * plan.damage.h)
    });
    assert!(plan_damage.1 <= 4 * 4 || plan_damage.0, "damage after a 4 px commit is 4 px (or already consumed): {plan_damage:?}");
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
        let items = s.scene.plan(&s.surfaces).items.len();
        (s.output.capture().unwrap(), items)
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
    imported.attach_punchthrough("_ctx_1".into());
    // LG's opcode 7 (Netflix sends it right after the attach): it must reach the handler, not kill the connection.
    imported.set_z_index(3);
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
    assert!(calls.iter().any(|c| c.contains("setCompositing") && c.contains("_ctx_1")), "{calls:?}");
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

#[test]
fn home_held_through_the_remote_command_asks_for_the_fallback() {
    use lumatv_compositor::luna::ToBridge;
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let _kb = client.seat.get_keyboard(&client.qh, ());
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Held");
    let buf = client.shm_buffer(64, 64, 0xff00_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.keyboard_enters >= 1));
    // A tap goes to launchDefaultApp (Home is the compositor's), never to the client.
    let reply = server.eval(|s| lumatv_compositor::control::handle(s, "remote 172 tap"));
    assert!(reply.contains("\"ok\":true"), "{reply}");
    let launched = server.eval(|s| s.luna.sent().iter().any(|(uri, _)| uri.ends_with("/launchDefaultApp")));
    assert!(launched, "a tapped Home launches the home app");
    // Held past 650 ms, the tick's hold poll sends the fallback to the supervisor.
    server.eval(|s| lumatv_compositor::control::handle(s, "remote 172 press"));
    std::thread::sleep(std::time::Duration::from_millis(700));
    server.eval(|s| s.poll_holds());
    let fallback = server.eval(|s| s.luna.history().iter().any(|m| matches!(m, ToBridge::Fallback { .. })));
    assert!(fallback, "a held Home is the escape to LG's compositor");
    server.eval(|s| lumatv_compositor::control::handle(s, "remote 172 release"));
    let launches = server.eval(|s| s.luna.sent().iter().filter(|(uri, _)| uri.ends_with("/launchDefaultApp")).count());
    assert_eq!(launches, 1, "the release after an escape does not launch Home again");
    assert!(client.state.keys.is_empty(), "Home never reached the client");
}

#[test]
fn pointer_enters_the_window_under_it_and_ok_clicks_while_the_cursor_shows() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let _kb = client.seat.get_keyboard(&client.qh, ());
    let _ptr = client.seat.get_pointer(&client.qh, ());
    let input: lumatv_wl_protocols::webos_input_manager::client::wl_webos_input_manager::WlWebosInputManager =
        client.globals.bind(&client.qh, 1..=1, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Pointed");
    let buf = client.shm_buffer(64, 64, 0xffff_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.keyboard_enters >= 1));
    assert_eq!(client.state.cursor_visibility, vec![0], "a new input manager hears the current state");
    // An app asking for the cursor does not put a sprite up before the remote moves.
    input.set_cursor_visibility(1);
    client.roundtrip();
    assert!(!server.eval(|s| s.seat.cursor_visible));
    std::thread::sleep(Duration::from_millis(30));
    // Off the window: nothing enters, the cursor shows all the same and every client hears it.
    let reply = server.eval(|s| lumatv_compositor::control::handle(s, "pointer 500 500"));
    assert!(reply.contains("\"delivered\":false"), "{reply}");
    assert!(server.eval(|s| s.seat.cursor_visible && s.seat.last_input == "pointer"));
    assert!(client.wait_until(WAIT, |s| s.cursor_visibility.last() == Some(&1)));
    // Onto the 64x64 card at the origin: enter at the surface-local point, then the motion.
    let reply = server.eval(|s| lumatv_compositor::control::handle(s, "pointer 10 20"));
    assert!(reply.contains("\"delivered\":true"), "{reply}");
    assert!(client.wait_until(WAIT, |s| !s.pointer_enters.is_empty() && !s.motions.is_empty()));
    assert_eq!(client.state.pointer_enters[0], (10.0, 20.0));
    assert_eq!(client.state.motions[0], (10.0, 20.0));
    // OK while the cursor shows is BTN_LEFT, never a key.
    server.eval(|s| lumatv_compositor::control::handle(s, "click tap"));
    assert!(client.wait_until(WAIT, |s| s.buttons.len() >= 2));
    assert_eq!(client.state.buttons[0], (0x110, 1));
    assert_eq!(client.state.buttons[1], (0x110, 0));
    assert!(client.state.keys.is_empty(), "the click did not arrive as KEY_ENTER");
    // An arrow key hides the cursor (leave) and reaches the window as a key; OK is a key again.
    server.eval(|s| lumatv_compositor::control::handle(s, "remote 108 tap"));
    assert!(client.wait_until(WAIT, |s| s.pointer_leaves >= 1 && s.keys.len() >= 2));
    assert!(!server.eval(|s| s.seat.cursor_visible));
    assert!(client.wait_until(WAIT, |s| s.cursor_visibility.last() == Some(&0)), "the hide reaches every client");
    server.eval(|s| lumatv_compositor::control::handle(s, "click tap"));
    assert!(client.wait_until(WAIT, |s| s.keys.len() >= 4));
    assert_eq!(client.state.keys[2], (28, 1));
    assert_eq!(client.state.buttons.len(), 2, "no button without the cursor");
    // Moving again re-enters and the sprite is composed at the pointer: white at its centre, the card's red
    // untouched beside it. A cursor left alone past the timeout hides from the tick and the red comes back.
    server.eval(|s| lumatv_compositor::control::handle(s, "pointer 30 30"));
    assert!(client.wait_until(WAIT, |s| s.pointer_enters.len() >= 2));
    std::thread::sleep(Duration::from_millis(30));
    let shown = server.eval(|s| s.output.capture().unwrap());
    assert_eq!(shown.pixel(30, 30), [0xff, 0xff, 0xff, 0xff], "the sprite's centre");
    assert_eq!(shown.pixel(60, 60), [0xff, 0, 0, 0xff], "the card beside it");
    server.eval(|s| {
        s.seat.cursor_shown_at = Some(std::time::Instant::now() - lumatv_compositor::server::seat::CURSOR_TIMEOUT);
        s.poll_holds();
    });
    assert!(client.wait_until(WAIT, |s| s.pointer_leaves >= 2));
    assert!(!server.eval(|s| s.seat.cursor_visible));
    std::thread::sleep(Duration::from_millis(30));
    let hidden = server.eval(|s| s.output.capture().unwrap());
    assert_eq!(hidden.pixel(30, 30), [0xff, 0, 0, 0xff], "the card is whole again");
}

#[test]
fn luna_tier_one_methods_answer_from_the_scene() {
    use lumatv_compositor::luna::{Inbound, ToBridge};
    use serde_json::json;
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    // A subscription opened before any window exists, like SAM's at boot.
    server.eval(|s| {
        s.luna_inbound(Inbound {
            token: 11,
            service: "com.webos.surfacemanager".into(),
            method: "getForegroundAppInfo".into(),
            payload: json!({"subscribe": true}),
            subscribe: true,
            sender: "com.webos.applicationManager".into(),
            cancelled: false,
        });
        s.luna_inbound(Inbound {
            token: 12,
            service: "com.webos.coverUI".into(),
            method: "getStatus".into(),
            payload: json!({}),
            subscribe: false,
            sender: "com.webos.applicationManager".into(),
            cancelled: false,
        });
        s.luna_inbound(Inbound {
            token: 13,
            service: "com.webos.surfacemanager".into(),
            method: "noSuchMethod".into(),
            payload: json!({}),
            subscribe: false,
            sender: "x".into(),
            cancelled: false,
        });
    });
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Player");
    let buf = client.shm_buffer(1920, 1080, 0xff00_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    std::thread::sleep(Duration::from_millis(20));
    let history = server.eval(|s| s.luna.history());
    let replies: Vec<(u64, serde_json::Value)> = history
        .iter()
        .filter_map(|m| match m {
            ToBridge::Reply(r) => Some((r.token, r.payload.clone())),
            _ => None,
        })
        .collect();
    let first = &replies.iter().find(|(t, _)| *t == 11).unwrap().1;
    assert_eq!(first["returnValue"], true);
    assert_eq!(first["subscribed"], true);
    assert_eq!(first["foregroundAppInfo"].as_array().unwrap().len(), 0, "nothing in front at subscription time");
    let cover = &replies.iter().find(|(t, _)| *t == 12).unwrap().1;
    assert_eq!(cover["status"], "closed");
    let unknown = &replies.iter().find(|(t, _)| *t == 13).unwrap().1;
    assert_eq!(unknown["returnValue"], false);
    let pushed = replies.iter().rev().find(|(t, _)| *t == 11).unwrap();
    let info = &pushed.1["foregroundAppInfo"][0];
    assert_eq!(info["appId"], "org.test.player", "the mapped card was pushed to the subscriber");
    assert_eq!(info["windowType"], "_WEBOS_WINDOW_TYPE_CARD");
    assert_eq!(info["primary"], true);
    assert_eq!(pushed.1["mode"], "normal");
    assert!(pushed.1.get("subscribed").is_none(), "pushes carry no subscribed flag (LG's do not)");
    assert_eq!(info["processId"], std::process::id().to_string(), "the client's pid, as a string");

    let (focused, windows, keymask) = server.eval(|s| {
        (
            s.luna_method("com.webos.surfacemanager", "getFocusedWindowInfo", &json!({}), "eim").unwrap(),
            s.luna_method("com.webos.surfacemanager", "getCurrentState", &json!({}), "alwaysready").unwrap(),
            s.luna_method("com.webos.surfacemanager.input", "getForegroundAppKeymask", &json!({}), "qt").unwrap(),
        )
    });
    assert_eq!(focused["focusedWindowInfo"][0]["type"], "fullscreen");
    assert_eq!(windows["windows"][0]["appId"], "org.test.player");
    assert_eq!(keymask["appId"], "org.test.player");

    // An overlay app above the card comes second, in the short form (X4: Settings over Home).
    let overlay = client.card("_WEBOS_WINDOW_TYPE_OVERLAY", "Settings");
    let obuf = client.shm_buffer(640, 360, 0xff20_2020);
    client.present(&overlay.surface, &obuf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 2));
    std::thread::sleep(Duration::from_millis(20));
    let last = server.eval(|s| {
        s.luna.history().iter().rev().find_map(|m| match m {
            ToBridge::Reply(r) if r.token == 11 => Some(r.payload.clone()),
            _ => None,
        })
    });
    let apps = last.unwrap()["foregroundAppInfo"].as_array().unwrap().clone();
    assert_eq!(apps.len(), 2, "{apps:?}");
    assert_eq!(apps[0]["appId"], "org.test.player");
    assert_eq!(apps[1]["appId"], "org.test.settings");
    assert_eq!(apps[1]["windowType"], "_WEBOS_WINDOW_TYPE_OVERLAY");
    assert!(apps[1].get("primary").is_none(), "the short form");

    // closeByAppId reaches the window as wl_webos_shell_surface.close.
    let closed = server.eval(|s| s.luna_method("com.webos.surfacemanager", "closeByAppId", &json!({"appId": "org.test.player"}), "home").unwrap());
    assert_eq!(closed["closed"], 1);
    assert!(client.wait_until(WAIT, |s| s.closes >= 1));
}

#[test]
fn destroying_the_shown_buffer_keeps_the_last_frame() {
    // LG's libmali destroys its swapchain on eglTerminate while the last frame is still on screen; the X2
    // client's last frame went black for it on the set (first light 2026-10-25, own case).
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Last");
    let buf = client.shm_buffer(1920, 1080, 0xff7d_8082);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    buf.destroy();
    client.roundtrip();
    // Force a redraw of the whole output and look again.
    server.eval(|s| s.scene.layout_changed = true);
    std::thread::sleep(Duration::from_millis(30));
    let (image, mapped) = server.eval(|s| (s.output.capture().unwrap(), s.scene.foreground(&s.surfaces).is_some()));
    assert!(mapped, "the window stays mapped");
    assert_eq!(image.pixel(960, 540), [0x7d, 0x80, 0x82, 0xff], "the last frame stays on screen");
}

/// The GL renderer on Mesa's llvmpipe (surfaceless), the same code that runs on the 64-bit Mali blob: an shm
/// buffer uploaded as a texture must come back pixel-exact. On the set this path drew black with GL error 0x502
/// (first light 2026-10-25, shm case): a BGRA upload into an RGBA texture.
#[test]
fn gl_renderer_uploads_shm_pixel_exact() {
    const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
    if !std::path::Path::new(MESA).exists() {
        eprintln!("no Mesa libEGL on this host; skipping");
        return;
    }
    // SAFETY: set before the server thread (the only EGL user) starts.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let mut server = Server::start_with(Box::new(|| {
        use lumatv_compositor::render::egl::Platform;
        use lumatv_compositor::render::gl::{BufferPath, Gl};
        Box::new(Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).expect("Mesa GL renderer"))
    }));
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Gl");
    let buf = client.shm_buffer(1920, 1080, 0xff19_80e6); // X2 frame 1: R 0x19 G 0x80 B 0xe6
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    let overlay = client.card("_WEBOS_WINDOW_TYPE_OVERLAY", "Half");
    overlay.webos.set_property("windowPosX".into(), "100".into());
    overlay.webos.set_property("windowPosY".into(), "100".into());
    let half = client.shm_buffer(200, 200, 0x8080_8080); // premultiplied half-transparent white
    client.present(&overlay.surface, &half);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 2));
    std::thread::sleep(Duration::from_millis(30));
    let (image, name, skipped, gl_errors) = server.eval(|s| (s.output.capture().unwrap(), s.output.name(), s.stats.skipped_items, s.stats.gl_errors));
    assert_eq!(name, "gl");
    assert_eq!(skipped, 0);
    assert_eq!(gl_errors, 0);
    assert_eq!(image.pixel(10, 10), [0x19, 0x80, 0xe6, 0xff], "card pixel through the GL upload");
    assert_eq!(image.pixel(1919, 1079), [0x19, 0x80, 0xe6, 0xff]);
    let p = image.pixel(150, 150);
    // Half white over the card: each channel = 0x80 + card * 0.5 (within rounding).
    let expect = [0x80 + 0x19 / 2, 0x80 + 0x80 / 2, 0x80 + 0xe6 / 2, 0xff];
    for (i, (got, want)) in p.iter().zip(expect.iter()).enumerate() {
        assert!(got.abs_diff(*want) <= 2, "channel {i}: got {got:#x} want {want:#x} (pixel {p:?})");
    }
}

#[test]
fn a_surface_that_vanishes_with_an_undrawn_frame_is_drawn_once() {
    // The X2 client disconnects about a millisecond after its last swap, inside the tick (first light re-run
    // 2026-10-25: 6 commits, 5 drawn). The frame must still reach the output and its callback.
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Last");
    let a = client.shm_buffer(1920, 1080, 0xff10_2030);
    client.present(&win.surface, &a);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    // No more ticks: the next commit stays undrawn until something forces a frame.
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    let b = client.shm_buffer(1920, 1080, 0xff7d_8082);
    client.present(&win.surface, &b);
    client.roundtrip();
    let drawn_before = server.eval(|s| s.stats.frames_drawn);
    assert_eq!(drawn_before, 1, "the second commit is not drawn while the tick is paused");
    win.surface.destroy();
    client.roundtrip();
    std::thread::sleep(Duration::from_millis(20));
    let (drawn, image, surfaces) = server.eval(|s| (s.stats.frames_drawn, s.output.capture().unwrap(), s.surfaces.len()));
    assert_eq!(drawn, 2, "the undrawn frame was drawn before the surface went");
    assert_eq!(image.pixel(960, 540), [0x7d, 0x80, 0x82, 0xff]);
    assert_eq!(surfaces, 0);
}

#[test]
fn planning_a_live_scene_allocates_nothing_once_warm() {
    // LumaShow lesson 20: the frame path must not touch the heap once warm. A card with a group member and an
    // overlay, re-planned 200 times with fresh damage each time, on the server thread.
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Card");
    let base = client.shm_buffer(1920, 1080, 0xff00_0000);
    client.present(&card.surface, &base);
    let group = client.groups.create_surface_group(&card.surface, "alloc.group".into(), &client.qh, ());
    let _layer = group.create_layer("top".into(), 10, &client.qh, ());
    let member = client.compositor.create_surface(&client.qh, ());
    group.attach(&member, "top".into());
    let yellow = client.shm_buffer(200, 200, 0xffff_ff00);
    client.present(&member, &yellow);
    let overlay = client.card("_WEBOS_WINDOW_TYPE_OVERLAY", "Toast");
    let white = client.shm_buffer(300, 100, 0xffff_ffff);
    client.present(&overlay.surface, &white);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 3));
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(10));
    let allocations = server.eval(|s| {
        // Warm: one plan at this size with damage rectangles present, so the plan's storage has its capacity.
        for surf in s.surfaces.iter_mut() {
            surf.current.damage = Some(vec![lumatv_compositor::Rect::new(0, 40, 32, 32)]);
        }
        let _ = s.scene.plan(&s.surfaces);
        let mut total_items = 0;
        let n = harness::counted(|| {
            for k in 0..200 {
                // Fresh damage on every surface each frame, as clients drawing would leave (written in place).
                for surf in s.surfaces.iter_mut() {
                    if let Some(d) = surf.current.damage.as_mut() {
                        d[0] = lumatv_compositor::Rect::new((k * 3) % 1500, 40, 32, 32);
                    }
                }
                let plan = s.scene.plan(&s.surfaces);
                total_items += plan.items.len();
                assert!(!plan.damage.is_empty());
            }
        });
        assert_eq!(total_items, 200 * 3);
        n
    });
    assert_eq!(allocations, 0, "allocations while planning 200 frames");
}

/// Software output whose drawn frames are on their way to the screen until the test lets them land, like KMS.
struct FlipOutput {
    inner: lumatv_compositor::render::software::Software,
    landings: std::sync::Arc<std::sync::atomic::AtomicU32>,
    drew: bool,
}

impl lumatv_compositor::output::Output for FlipOutput {
    fn name(&self) -> &'static str {
        "flip"
    }
    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }
    fn query_buffer(&mut self, b: &wayland_server::protocol::wl_buffer::WlBuffer) -> Option<((i32, i32), bool)> {
        self.inner.query_buffer(b)
    }
    fn forget_buffer(&mut self, b: &wayland_server::protocol::wl_buffer::WlBuffer) {
        self.inner.forget_buffer(b)
    }
    fn render(&mut self, plan: &lumatv_compositor::scene::Plan, s: &lumatv_compositor::surface::Surfaces) -> std::io::Result<lumatv_compositor::output::RenderStats> {
        self.drew = true;
        self.inner.render(plan, s)
    }
    fn present(&mut self) -> std::io::Result<lumatv_compositor::output::Presented> {
        let mut p = self.inner.present()?;
        p.pending = std::mem::take(&mut self.drew);
        Ok(p)
    }
    fn capture(&mut self) -> std::io::Result<lumatv_compositor::output::Image> {
        self.inner.capture()
    }
    fn dispatch_events(&mut self) -> std::io::Result<u32> {
        Ok(self.landings.swap(0, std::sync::atomic::Ordering::SeqCst))
    }
}

#[test]
fn frame_callbacks_wait_for_the_frame_to_land() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    let landings = Arc::new(AtomicU32::new(0));
    let l2 = landings.clone();
    let mut server = Server::start_with(Box::new(move || {
        Box::new(FlipOutput { inner: lumatv_compositor::render::software::Software::new(1920, 1080), landings: l2, drew: false })
    }));
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Paced");
    let buf = client.shm_buffer(64, 64, 0xff00_ff00);
    client.present(&win.surface, &buf);
    std::thread::sleep(Duration::from_millis(60));
    client.roundtrip();
    assert_eq!(client.state.frames_done, 0, "drawn and on its way, not landed: the callback waits");
    landings.store(1, Ordering::SeqCst);
    server.eval(|s| {
        let n = s.output.dispatch_events().unwrap();
        s.frames_landed(n);
    });
    assert!(client.wait_until(WAIT, |s| s.frames_done == 1), "the landing fires it");
    // A flip that never reports does not hold clients for good.
    client.present(&win.surface, &buf);
    assert!(client.wait_until(Duration::from_secs(2), |s| s.frames_done == 2), "sent after 250 ms without a flip");
}
