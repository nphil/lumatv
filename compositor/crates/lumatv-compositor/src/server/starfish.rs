//! `wl_starfish_pointer` and `wl_starfish_output` (WebAppMgr). The opcode tables in `protocol/private/
//! starfish.xml` are unverified, so these globals exist only behind `--starfish`.

use wayland_server::{Client, DataInit, Dispatch, DisplayHandle};

use lumatv_wl_protocols::starfish::server::wl_starfish_output::WlStarfishOutput;
use lumatv_wl_protocols::starfish::server::wl_starfish_pointer::{self, WlStarfishPointer};

use crate::{State, noop_dispatch, plain_global};

plain_global!(WlStarfishPointer);
plain_global!(WlStarfishOutput);

impl Dispatch<WlStarfishPointer, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlStarfishPointer,
        request: wl_starfish_pointer::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_starfish_pointer::Request::SetCursorPosition { x, y } => {
                state.note_request("wl_starfish_pointer", 0, &format!("set_cursor_position({x}, {y})"));
                state.seat.pointer_position = (x as i32, y as i32);
            }
            wl_starfish_pointer::Request::SetMrcuStandbyTimer { value } => {
                state.note_request("wl_starfish_pointer", 0, &format!("set_mrcu_standby_timer({value})"));
                state.seat.mrcu_standby_timer = value;
            }
            wl_starfish_pointer::Request::GetMrcuStandbyTimer => {
                resource.timer_value(state.seat.mrcu_standby_timer);
            }
            _ => {}
        }
    }
}

noop_dispatch!(WlStarfishOutput, ());
