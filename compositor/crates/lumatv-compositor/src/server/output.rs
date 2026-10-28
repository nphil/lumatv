//! `wl_output`: one 1920x1080 output at 60 Hz on the 65-inch panel.

use wayland_server::protocol::wl_output::{self, Mode, Subpixel, Transform, WlOutput};
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};

use crate::{OUTPUT_MM, REFRESH_MHZ, State};

impl GlobalDispatch<WlOutput, ()> for State {
    fn bind(state: &mut Self, _handle: &DisplayHandle, client: &Client, resource: New<WlOutput>, _global_data: &(), data_init: &mut DataInit<'_, Self>) {
        state.note_bind(client, "wl_output");
        let out = data_init.init(resource, ());
        let (w, h) = state.output.size();
        out.geometry(0, 0, OUTPUT_MM.0, OUTPUT_MM.1, Subpixel::Unknown, "LG".into(), "OLED65C4PUA".into(), Transform::Normal);
        out.mode(Mode::Current | Mode::Preferred, w as i32, h as i32, REFRESH_MHZ);
        // LG's compositor sends geometry, mode and done, never scale (probed 2026-10-28); LG's libwayland-client calls a
        // listener entry a client left NULL, so an event LG never sends is one no LG client has to survive.
        if out.version() >= 2 {
            out.done();
        }
    }
}

impl Dispatch<WlOutput, ()> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlOutput,
        _request: wl_output::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
    }
}
