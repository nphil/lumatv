//! The keyboard path (`server/ime.rs`): an app's text model on one connection, the on-screen keyboard's input method and input panel on
//! another (as WebAppMgr's apps and maliit's server are), a real `wayland-client` on each against the compositor.

mod harness;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::input::code::*;
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::window::WindowType;
use lumatv_wl_protocols::input_method::client::input_method::InputMethod;
use lumatv_wl_protocols::input_method::client::input_panel::InputPanel;
use lumatv_wl_protocols::input_method::client::input_panel_surface::InputPanelSurface;
use lumatv_wl_protocols::text::client::text_model::TextModel;
use wayland_client::Proxy;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_surface::WlSurface;

const WAIT: Duration = Duration::from_secs(3);

/// An app with a card that has the keyboard focus and a text model.
struct App {
    client: Client,
    win: Window,
    model: TextModel,
    _kb: WlKeyboard,
    _buf: WlBuffer,
}

fn app(server: &mut Server, title: &str) -> App {
    let mut client = if server.client_stream.is_some() { Client::connect(server) } else { Client::connect_extra(server) };
    let kb = client.seat.get_keyboard(&client.qh, ());
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", title);
    let buf = client.shm_buffer(64, 64, 0xff00_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.keyboard_enters >= 1), "the card has the focus");
    let model = client.text_model_factory().create_text_model(&client.qh, ());
    client.roundtrip();
    App { client, win, model, _kb: kb, _buf: buf }
}

impl App {
    fn activate(&mut self) {
        self.model.activate(1, &self.client.seat, &self.win.surface);
        self.client.conn.flush().unwrap();
    }
}

/// The on-screen keyboard's side: its input method (serving display 0) and the input panel.
struct Keyboard {
    client: Client,
    _method: InputMethod,
    panel: InputPanel,
}

fn keyboard(server: &mut Server) -> Keyboard {
    let mut client = Client::connect_extra(server);
    let method = client.input_method();
    method.set_display_id(0);
    let panel = client.input_panel();
    client.roundtrip();
    Keyboard { client, _method: method, panel }
}

impl Keyboard {
    /// The keyboard's window: an input panel surface with a buffer of this size.
    fn show(&mut self, w: i32, h: i32) -> (WlSurface, InputPanelSurface, WlBuffer) {
        let surface = self.client.compositor.create_surface(&self.client.qh, ());
        let ps = self.panel.get_input_panel_surface(&surface, &self.client.qh, ());
        let buf = self.client.shm_buffer(w, h, 0xff20_2020);
        self.client.present(&surface, &buf);
        (surface, ps, buf)
    }
}

/// A drawn surface and its rectangle (x, y, w, h).
type Placed = (u32, (i32, i32, i32, i32));

fn surfaces(s: &mut State) -> (Option<u32>, Vec<Placed>) {
    s.tick();
    let items = s.scene.last_plan().items.iter().map(|i| (i.surface.0, (i.rect.x, i.rect.y, i.rect.w, i.rect.h))).collect();
    (s.seat.focus.map(|f| f.0), items)
}

#[test]
fn a_field_and_the_on_screen_keyboard_talk_through_the_compositor() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Search");
    let mut k = keyboard(&mut server);
    a.activate();
    assert!(k.client.wait_until(WAIT, |s| s.ime.activations == 1), "the input method is given a context");
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_enters == 1), "the field is told it entered");

    // The field's description goes to the keyboard's context, in the order it was sent.
    a.model.set_content_type(0x7, 5);
    a.model.set_enter_key_type(5);
    a.model.set_surrounding_text("abc".into(), 3, 3);
    a.model.set_max_text_length(20);
    a.model.set_platform_data("p".into());
    a.model.commit();
    a.model.show_input_panel();
    a.client.conn.flush().unwrap();
    assert!(k.client.wait_until(WAIT, |s| s.ime.context_events.len() >= 6 && s.ime.shows == 1));
    assert_eq!(
        k.client.state.ime.context_events,
        ["content_type 7 5", "enter_key_type 5", "surrounding_text \"abc\" 3 3", "max_text_length 20", "platform_data \"p\"", "commit"]
    );

    // What the keyboard types is the field's.
    let ctx = k.client.state.ime.contexts[0].clone();
    ctx.commit_string(1, "hi".into());
    ctx.preedit_string(2, "wor".into(), "world".into());
    ctx.delete_surrounding_text(3, -1, 1);
    ctx.keysym(4, 0, 0xff0d, 1, 0);
    k.client.conn.flush().unwrap();
    assert!(a.client.wait_until(WAIT, |s| !s.ime.keysyms.is_empty()));
    let log = &a.client.state.ime;
    assert_eq!(
        (log.commits.clone(), log.preedits.clone(), log.deletes.clone(), log.keysyms.clone()),
        (vec!["hi".to_string()], vec!["wor".to_string()], vec![(-1, 1)], vec![(0xff0d, 1)])
    );

    // The keyboard's window is placed at the bottom, above the app, and the focus stays with the app.
    let (_surface, _ps, _buf) = k.show(1920, 360);
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1]), "{:?}", a.client.state.ime.panel_states);
    assert_eq!(a.client.state.ime.panel_rects.last(), Some(&(0, 720, 1920, 360)));
    let card = server.eval(|s| s.scene.windows().find(|w| w.kind == WindowType::Card).map(|w| w.surface.0)).unwrap();
    let (focus, items) = server.eval(surfaces);
    assert_eq!(focus, Some(card), "the keyboard does not take the focus");
    assert_eq!(items.first(), Some(&(card, (0, 0, 64, 64))));
    let keyboard_item = items.last().unwrap();
    assert_eq!(keyboard_item.1, (0, 720, 1920, 360), "full width, anchored at the bottom: {items:?}");
    assert!(server.eval(|s| s.keyboard_up()));
    let windows = server.eval(|s| lumatv_compositor::control::handle(s, "windows").text());
    assert!(windows.contains("\"type\":\"Keyboard\"") && windows.contains("\"mapped\":true"), "{windows}");

    // The keyboard grabs the hardware keyboard: the arrows are its, Home stays the compositor's, the app sees none.
    let grabbed = ctx.grab_keyboard(&k.client.qh, ());
    k.client.conn.flush().unwrap();
    assert!(k.client.wait_until(WAIT, |_| server.eval(|s| s.ime_grabbing())));
    server.eval(|s| {
        for code in [KEY_LEFT, KEY_HOMEPAGE] {
            s.key_event(SeatId(0), code, true, false);
            s.key_event(SeatId(0), code, false, false);
        }
    });
    let grab_id = grabbed.id().protocol_id();
    assert!(k.client.wait_until(WAIT, |s| s.keys_by_keyboard.len() >= 2));
    assert_eq!(k.client.state.keys_by_keyboard, vec![(grab_id, KEY_LEFT), (grab_id, KEY_LEFT)]);
    a.client.roundtrip();
    assert!(a.client.state.keys.is_empty(), "{:?}", a.client.state.keys);
    assert!(server.eval(|s| s.luna.sent().iter().any(|(u, _)| u.ends_with("/launchDefaultApp"))), "Home is not the keyboard's");

    // The field leaves: the keyboard is told, the field is told, the grab ends.
    a.model.deactivate(&a.client.seat);
    a.client.conn.flush().unwrap();
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_leaves == 1 && s.ime.panel_states.last() == Some(&0)));
    assert!(k.client.wait_until(WAIT, |s| s.ime.deactivated.len() == 1));
    assert!(!server.eval(|s| s.ime_grabbing()));
}

#[test]
fn an_activation_waits_for_an_input_method_and_asks_the_system_for_one() {
    let mut server = Server::start();
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = asked.clone();
    server.eval(move |s| {
        s.ime.ask_for_input_method = Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
    });
    let mut a = app(&mut server, "Search");
    a.activate();
    a.model.show_input_panel();
    a.client.conn.flush().unwrap();
    a.client.roundtrip();
    assert_eq!(a.client.state.ime.model_enters, 0, "no input method yet");
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    // The keyboard's server comes up and names its display: the activation goes on, with the show that came before it.
    let mut k = keyboard(&mut server);
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_enters == 1));
    assert!(k.client.wait_until(WAIT, |s| s.ime.activations == 1 && s.ime.shows == 1), "show_input_panel was remembered");
    assert_eq!(asked.load(Ordering::SeqCst), 1, "it was asked for once");
}

#[test]
fn a_text_field_on_a_surface_without_the_keyboard_focus_is_declined() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Search");
    let _b = app(&mut server, "Other");
    let mut k = keyboard(&mut server);
    a.activate();
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_leaves == 1), "told it left, as LG does");
    k.client.roundtrip();
    assert_eq!(k.client.state.ime.activations, 0);
    assert_eq!(server.eval(|s| s.ime.counts.declined), 1);
}

#[test]
fn the_activation_ends_when_the_field_loses_the_focus_the_keyboard_goes_or_the_app_goes() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Search");
    let mut k = keyboard(&mut server);
    a.activate();
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_enters == 1));
    // Another card comes to the front: the focus moves and the field leaves.
    let _b = app(&mut server, "Other");
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_leaves == 1), "focus lost");
    assert!(k.client.wait_until(WAIT, |s| s.ime.deactivated.len() == 1));
    // The same field on a keyboard that goes away.
    let mut c = app(&mut server, "Third");
    c.activate();
    assert!(c.client.wait_until(WAIT, |s| s.ime.model_enters == 1));
    drop(k);
    assert!(c.client.wait_until(WAIT, |s| s.ime.model_leaves == 1), "the input method went");
    assert!(server.eval(|s| s.ime.active_model().is_none()));
    // And an app that goes while its field holds the activation does not leave the keyboard with a context it never ends.
    let mut k2 = keyboard(&mut server);
    c.activate();
    assert!(k2.client.wait_until(WAIT, |s| s.ime.activations == 1));
    drop(c);
    assert!(k2.client.wait_until(WAIT, |s| s.ime.deactivated.len() == 1), "the keyboard was told");
    assert!(server.eval(|s| s.ime.active_model().is_none()));
}

#[test]
fn exit_with_the_keyboard_up_closes_the_keyboard_and_nothing_else() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Search");
    let mut k = keyboard(&mut server);
    a.activate();
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_enters == 1));
    let (_surface, _ps, _buf) = k.show(1920, 360);
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1]));
    let calls = server.eval(|s| {
        s.key_event(SeatId(0), KEY_EXIT, true, false);
        s.key_event(SeatId(0), KEY_EXIT, false, false);
        s.luna.sent().iter().filter(|(u, _)| u.ends_with("/closeByAppId")).count()
    });
    assert_eq!(calls, 0, "the app is not closed while the keyboard is up");
    assert!(k.client.wait_until(WAIT, |s| s.ime.deactivated.len() == 1), "the keyboard is told to go");
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_leaves == 1));
    a.client.roundtrip();
    assert!(a.client.state.keys.is_empty());
}

#[test]
fn the_keyboard_going_off_the_screen_is_told_and_the_app_can_ask_where_it_goes() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Search");
    let mut k = keyboard(&mut server);
    a.model.set_input_panel_rect(100, 200, 800, 300);
    a.activate();
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_enters == 1));
    let (surface, _ps, buf) = k.show(800, 300);
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1]));
    assert_eq!(a.client.state.ime.panel_rects.last(), Some(&(100, 200, 800, 300)));
    let (_, items) = server.eval(surfaces);
    assert_eq!(items.last().unwrap().1, (100, 200, 800, 300), "where the field asked for it");
    // The keyboard takes its window off the screen: the field hears it.
    surface.attach(None, 0, 0);
    surface.commit();
    k.client.conn.flush().unwrap();
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1, 0]), "{:?}", a.client.state.ime.panel_states);
    assert!(!server.eval(|s| s.keyboard_up()));
    drop(buf);
}

/// maliit's window: `w` x `h` with `region` (x, y, w, h) as its input region when it has one, as an input panel surface.
fn maliit_window(k: &mut Keyboard, w: i32, h: i32, region: Option<(i32, i32, i32, i32)>) -> (WlSurface, InputPanelSurface, WlBuffer) {
    let surface = k.client.compositor.create_surface(&k.client.qh, ());
    let ps = k.panel.get_input_panel_surface(&surface, &k.client.qh, ());
    if let Some((x, y, rw, rh)) = region {
        let r = k.client.region(x, y, rw, rh);
        surface.set_input_region(Some(&r));
    }
    let buf = k.client.shm_buffer(w, h, 0xff20_2020);
    k.client.present(&surface, &buf);
    (surface, ps, buf)
}

#[test]
fn the_keyboard_is_at_the_bottom_whatever_size_maliit_makes_its_window() {
    // On the TV maliit's window was 1920x438 in two runs of six and 1920x1080 in the other four, the keyboard drawn in the top 438 rows
    // and the rest transparent, and the keyboard was at the top of the screen whenever the window was the full size. Its input region
    // is the keyboard's area; the keyboard is placed, and told to the field, by that.
    let bottom = (0, 642, 1920, 438);
    let bottom_u = (0, 642, 1920u32, 438u32);
    for (name, w, h, region, window_at) in [
        ("the keyboard's own size", 1920, 438, None, (0, 642, 1920, 438)),
        ("the whole screen, the keyboard in its top rows", 1920, 1080, Some((0, 0, 1920, 438)), (0, 642, 1920, 1080)),
        ("the whole screen, the keyboard in its bottom rows", 1920, 1080, Some((0, 642, 1920, 438)), (0, 0, 1920, 1080)),
        ("the keyboard's own size with the region the whole window", 1920, 438, Some((0, 0, 1920, 438)), (0, 642, 1920, 438)),
        ("a region that is not on the window at all", 1920, 438, Some((0, 5000, 10, 10)), (0, 642, 1920, 438)),
    ] {
        let mut server = Server::start();
        let mut a = app(&mut server, "Search");
        let mut k = keyboard(&mut server);
        a.activate();
        assert!(a.client.wait_until(WAIT, |s| s.ime.model_enters == 1), "{name}");
        let (_surface, _ps, _buf) = maliit_window(&mut k, w, h, region);
        assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1]), "{name}");
        assert_eq!(a.client.state.ime.panel_rects.last(), Some(&bottom_u), "{name}: the field is told the keyboard's rectangle");
        let (_, items) = server.eval(surfaces);
        assert_eq!(items.last().unwrap().1, window_at, "{name}: the window");
        let panel = server.eval(|s| s.ime.panel);
        assert_eq!(panel.map(|r| (r.x, r.y, r.w, r.h)), Some(bottom), "{name}: the control socket's panel");
    }
}

#[test]
fn a_window_of_type_keyboard_is_placed_and_kept_off_the_focus_whichever_way_it_was_made() {
    // maliit's window may come through the webOS shell with the type set, not through input_panel.
    let mut server = Server::start();
    let a = app(&mut server, "Search");
    let mut k = Client::connect_extra(&mut server);
    let w = k.card("_WEBOS_WINDOW_TYPE_KEYBOARD", "keyboard");
    let buf = k.shm_buffer(1920, 300, 0xff30_3030);
    k.present(&w.surface, &buf);
    k.roundtrip();
    let card = server.eval(|s| s.scene.windows().find(|w| w.kind == WindowType::Card).map(|w| w.surface.0)).unwrap();
    let (focus, items) = server.eval(surfaces);
    assert_eq!(focus, Some(card));
    assert_eq!(items.last().unwrap().1, (0, 780, 1920, 300));
    drop(a);
}

#[test]
fn the_release_of_the_key_that_opened_the_keyboard_goes_back_to_the_app() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Search");
    let mut k = keyboard(&mut server);
    // OK is pressed on the field, which opens the keyboard; the keyboard grabs, and then OK is let go.
    server.eval(|s| s.key_event(SeatId(0), KEY_ENTER, true, false));
    a.activate();
    assert!(k.client.wait_until(WAIT, |s| s.ime.activations == 1));
    let ctx = k.client.state.ime.contexts[0].clone();
    let grabbed = ctx.grab_keyboard(&k.client.qh, ());
    let (_surface, _ps, _buf) = k.show(1920, 360);
    assert!(k.client.wait_until(WAIT, |_| server.eval(|s| s.ime_grabbing())), "the grab is in force once the keyboard is on the screen");
    server.eval(|s| s.key_event(SeatId(0), KEY_ENTER, false, false));
    a.client.roundtrip();
    assert_eq!(a.client.state.keys, vec![(KEY_ENTER, 1), (KEY_ENTER, 0)], "the app gets the release of the press it got");
    // The next press is the keyboard's.
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_ENTER, true, false);
        s.key_event(SeatId(0), KEY_ENTER, false, false);
    });
    assert!(k.client.wait_until(WAIT, |s| s.keys_by_keyboard.len() >= 2));
    assert_eq!(k.client.state.keys_by_keyboard, vec![(grabbed.id().protocol_id(), KEY_ENTER); 2]);
    a.client.roundtrip();
    assert_eq!(a.client.state.keys.len(), 2);
}

#[test]
fn a_keyboard_window_nobody_asked_for_does_not_stop_exit_from_closing_the_app() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Search");
    let mut k = keyboard(&mut server);
    let (_surface, _ps, _buf) = k.show(1920, 360);
    k.client.roundtrip();
    let calls = server.eval(|s| {
        s.key_event(SeatId(0), KEY_EXIT, true, false);
        s.key_event(SeatId(0), KEY_EXIT, false, false);
        s.luna.sent().iter().filter(|(u, _)| u.ends_with("/closeByAppId")).count()
    });
    assert_eq!(calls, 1);
    a.client.roundtrip();
}

#[test]
fn a_keyboard_that_is_off_the_screen_gives_the_keys_back_to_the_app() {
    let mut server = Server::start();
    let mut a = app(&mut server, "Search");
    let mut k = keyboard(&mut server);
    a.activate();
    assert!(k.client.wait_until(WAIT, |s| s.ime.activations == 1));
    let ctx = k.client.state.ime.contexts[0].clone();
    let _grabbed = ctx.grab_keyboard(&k.client.qh, ());
    k.client.conn.flush().unwrap();
    // Grabbed, but not on the screen yet: the arrows are the app's.
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_RIGHT, true, false);
        s.key_event(SeatId(0), KEY_RIGHT, false, false);
    });
    a.client.roundtrip();
    assert_eq!(a.client.state.keys, vec![(KEY_RIGHT, 1), (KEY_RIGHT, 0)]);
    a.client.state.keys.clear();
    // On the screen: the keyboard's.
    let (surface, _ps, _buf) = k.show(1920, 360);
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1]));
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_RIGHT, true, false);
        s.key_event(SeatId(0), KEY_RIGHT, false, false);
    });
    assert!(k.client.wait_until(WAIT, |s| s.keys_by_keyboard.len() >= 2));
    a.client.roundtrip();
    assert!(a.client.state.keys.is_empty());
    // The keyboard hides itself (Back): the app steers again at once, with the activation still the field's.
    surface.attach(None, 0, 0);
    surface.commit();
    k.client.conn.flush().unwrap();
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1, 0]));
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_RIGHT, true, false);
        s.key_event(SeatId(0), KEY_RIGHT, false, false);
    });
    a.client.roundtrip();
    assert_eq!(a.client.state.keys, vec![(KEY_RIGHT, 1), (KEY_RIGHT, 0)], "{:?}", a.client.state.keys);
    assert!(server.eval(|s| s.ime.active_model().is_some()), "the field is still active: the keyboard can come back");
}
