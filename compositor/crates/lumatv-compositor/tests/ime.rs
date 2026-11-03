//! Vendor input-method and input-panel protocol binding and window lifecycle.

mod harness;

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
fn vendor_input_panels_are_registered_but_not_drawn() {
    let bottom = (0, 642, 1920, 438);
    for (name, w, h, region, _window_at) in [
        ("the keyboard's own size", 1920, 438, None, (0, 642, 1920, 438)),
        ("the whole screen, the keyboard in its top rows", 1920, 1080, Some((0, 0, 1920, 438)), (0, 642, 1920, 1080)),
        // Round 7 of the TV: the full-screen window with no input region (kisf-2), the keyboard in its top 438 rows.
        ("the whole screen and no region at all", 1920, 1080, None, (0, 642, 1920, 1080)),
        ("a region that is not on the window, on the whole screen", 1920, 1080, Some((0, 5000, 10, 10)), (0, 642, 1920, 1080)),
        ("the whole screen, the keyboard in its bottom rows", 1920, 1080, Some((0, 642, 1920, 438)), (0, 0, 1920, 1080)),
        ("the keyboard's own size with the region the whole window", 1920, 438, Some((0, 0, 1920, 438)), (0, 642, 1920, 438)),
        ("a region that is not on the window at all", 1920, 438, Some((0, 5000, 10, 10)), (0, 642, 1920, 438)),
    ] {
        let mut server = Server::start();
        let _a = app(&mut server, "Search");
        let mut k = keyboard(&mut server);
        let (_surface, _ps, _buf) = maliit_window(&mut k, w, h, region);
        let (_, items) = server.eval(surfaces);
        assert_eq!(items.len(), 1, "{name}: vendor panel must not be drawn");
        let panel = server.eval(|s| s.ime.panel);
        assert_eq!(panel.map(|r| (r.x, r.y, r.w, r.h)), Some(bottom), "{name}: the control socket's panel");
    }
}

#[test]
fn a_keyboard_window_is_hidden_and_kept_off_focus() {
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
    assert_eq!(items.len(), 1, "vendor keyboard must not be drawn");
    drop(a);
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

