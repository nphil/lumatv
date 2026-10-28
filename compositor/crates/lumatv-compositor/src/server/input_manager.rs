//! `wl_webos_input_manager`, `wl_webos_seat` and the sensor objects.

use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use lumatv_wl_protocols::webos_input_manager::server::wl_webos_accelerometer::WlWebosAccelerometer;
use lumatv_wl_protocols::webos_input_manager::server::wl_webos_gyroscope::WlWebosGyroscope;
use lumatv_wl_protocols::webos_input_manager::server::wl_webos_input_manager::{self, WlWebosInputManager};
use lumatv_wl_protocols::webos_input_manager::server::wl_webos_seat::{self, Capability, WlWebosSeat};

use crate::{State, noop_dispatch};

impl wayland_server::GlobalDispatch<WlWebosInputManager, ()> for State {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        client: &Client,
        resource: wayland_server::New<WlWebosInputManager>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.note_bind(client, "wl_webos_input_manager");
        let r = data_init.init(resource, ());
        // A client bound late learns the current state at once (LG's apps pick pointer or 5-way focus from it).
        r.cursor_visibility(state.seat.cursor_visible as u32, None);
        state.seat.input_managers.push(r);
    }
}

impl Dispatch<WlWebosInputManager, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &WlWebosInputManager,
        request: wl_webos_input_manager::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        match request {
            wl_webos_input_manager::Request::GetWebosSeat { id, seat } => {
                let ws = data_init.init(id, ());
                let sid = seat.data::<crate::server::seat::SeatId>().copied().unwrap_or_default();
                // As LG's compositor: the seat's index as its id, the input device's name, designator 0 (the primary), no
                // sensors. LG's SDL maps "LGE RCU" and "LGE M-RCU" in the name to its remote numbers.
                let name = state.seat.device_name(sid).to_string();
                ws.info(sid.0, name.clone(), 0, Capability::Default as u32);
                state.note_request("wl_webos_input_manager", 0, &format!("get_webos_seat({}: {name})", sid.name()));
            }
            wl_webos_input_manager::Request::SetCursorVisibility { visibility } => {
                // An app may hide the cursor (video playback does); showing it is the pointer's own business, so a
                // 1 here does not put a sprite on screen before the remote moves.
                state.note_request("wl_webos_input_manager", 0, &format!("set_cursor_visibility({visibility})"));
                if visibility == 0 {
                    crate::server::seat::hide_cursor(state);
                } else {
                    resource.cursor_visibility(state.seat.cursor_visible as u32, None);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlWebosSeat, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &WlWebosSeat,
        request: wl_webos_seat::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
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
