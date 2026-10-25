//! `wl_output`: one 1920x1080 output at 60 Hz on the 65-inch panel.

use wayland_server::protocol::wl_output::{self, Mode, Subpixel, Transform, WlOutput};
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};

use crate::{OUTPUT_MM, REFRESH_MHZ, State};

impl GlobalDispatch<WlOutput, ()> for State {
    fn bind(state: &mut Self, _handle: &DisplayHandle, _client: &Client, resource: New<WlOutput>, _global_data: &(), data_init: &mut DataInit<'_, Self>) {
        let out = data_init.init(resource, ());
        let (w, h) = state.output.size();
        out.geometry(0, 0, OUTPUT_MM.0, OUTPUT_MM.1, Subpixel::Unknown, "LG".into(), "OLED65C4PUA".into(), Transform::Normal);
        out.mode(Mode::Current | Mode::Preferred, w as i32, h as i32, REFRESH_MHZ);
        if out.version() >= 2 {
            out.scale(1);
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
