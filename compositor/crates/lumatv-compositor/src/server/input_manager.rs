//! `wl_webos_input_manager`, `wl_webos_seat` and the sensor objects.

use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use lumatv_wl_protocols::webos_input_manager::server::wl_webos_accelerometer::WlWebosAccelerometer;
use lumatv_wl_protocols::webos_input_manager::server::wl_webos_gyroscope::WlWebosGyroscope;
use lumatv_wl_protocols::webos_input_manager::server::wl_webos_input_manager::{self, WlWebosInputManager};
use lumatv_wl_protocols::webos_input_manager::server::wl_webos_seat::{self, Capability, WlWebosSeat};

use crate::{State, noop_dispatch, plain_global};

plain_global!(WlWebosInputManager);

impl Dispatch<WlWebosInputManager, ()> for State {
    fn request(state: &mut Self, client: &Client, resource: &WlWebosInputManager, request: wl_webos_input_manager::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        state.touch_client(client);
        match request {
            wl_webos_input_manager::Request::GetWebosSeat { id, seat } => {
                let ws = data_init.init(id, ());
                // id, name, designator and capabilities: the remote is one keyboard-and-pointer seat, no sensors.
                ws.info(1, "seat0".into(), 0, Capability::Default as u32);
                state.note_request("wl_webos_input_manager", 0, &format!("get_webos_seat(wl_seat@{})", seat.id().protocol_id()));
            }
            wl_webos_input_manager::Request::SetCursorVisibility { visibility } => {
                state.note_request("wl_webos_input_manager", 0, &format!("set_cursor_visibility({visibility})"));
                state.seat.cursor_visible = visibility != 0;
                resource.cursor_visibility(visibility, None);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlWebosSeat, ()> for State {
    fn request(state: &mut Self, _client: &Client, _resource: &WlWebosSeat, request: wl_webos_seat::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        match request {
            wl_webos_seat::Request::GetGyroscope { id } => {
                data_init.init(id, ());
                state.note_request("wl_webos_seat", 0, "get_gyroscope");
            }
            wl_webos_seat::Request::GetAccelerometer { id } => {
                data_init.init(id, ());
                state.note_request("wl_webos_seat", 0, "get_accelerometer");
            }
            _ => {}
        }
    }
}

noop_dispatch!(WlWebosGyroscope, ());
noop_dispatch!(WlWebosAccelerometer, ());
