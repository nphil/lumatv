//! `wl_starfish_pointer` and `wl_starfish_output`: LG's private cursor and 3D globals, which every LG set offers (its
//! `libstarfishcompositorplugin.so`). WebAppMgr binds both; LG's SDL (PlxNative, Apple TV) binds the pointer and sends
//! `set_mrcu_standby_timer` at the bind, at each `state_changed(fullscreen)` and at each pointer enter, so the global
//! must exist for those apps to live. The tables in `protocol/private/starfish.xml` are LG's (read on the set).

use wayland_server::{Client, DataInit, Dispatch, DisplayHandle};

use lumatv_wl_protocols::starfish::server::wl_starfish_output::WlStarfishOutput;
use lumatv_wl_protocols::starfish::server::wl_starfish_pointer::{self, WlStarfishPointer};

use crate::{State, noop_dispatch, plain_global};

/// What `get_mrcu_standby_timer` answers a client that set none: LG's compositor says 5000 (probed 2026-10-28).
pub const MRCU_STANDBY_DEFAULT_MS: u32 = 5000;

plain_global!(WlStarfishPointer);
plain_global!(WlStarfishOutput);

impl Dispatch<WlStarfishPointer, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &WlStarfishPointer,
        request: wl_starfish_pointer::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        match request {
            wl_starfish_pointer::Request::SetCursorPosition { x, y } => {
                state.note_request("wl_starfish_pointer", 0, &format!("set_cursor_position({x}, {y})"));
                state.seat.pointer_position = (x as i32, y as i32);
            }
            wl_starfish_pointer::Request::SetMrcuStandbyTimer { value } => {
                state.note_request("wl_starfish_pointer", 0, &format!("set_mrcu_standby_timer({value})"));
                if let Some(c) = state.clients.get_mut(&client.id()) {
                    c.mrcu_standby_ms = value;
                }
            }
            wl_starfish_pointer::Request::GetMrcuStandbyTimer => {
                resource.timer_value(state.clients.get(&client.id()).map_or(MRCU_STANDBY_DEFAULT_MS, |c| c.mrcu_standby_ms));
            }
            _ => {}
        }
    }
}

noop_dispatch!(WlStarfishOutput, ());
