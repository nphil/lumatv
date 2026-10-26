//! Globals LG's clients bind at start-up and that need objects, not behaviour, for a first app: xinput, system UX and the data
//! device manager. The text model, input method and input panel (the keyboard) are `ime.rs`.

use wayland_server::protocol::wl_data_device::WlDataDevice;
use wayland_server::protocol::wl_data_device_manager::{self, WlDataDeviceManager};
use wayland_server::protocol::wl_data_source::WlDataSource;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle};

use lumatv_wl_protocols::webos_extension::server::wl_webos_xinput::WlWebosXinput;
use lumatv_wl_protocols::webos_extension::server::wl_webos_xinput_extension::{self, WlWebosXinputExtension};
use lumatv_wl_protocols::webos_system_ux::server::wl_webos_system_ux::WlWebosSystemUx;

use crate::{State, noop_dispatch, plain_global};

plain_global!(WlWebosXinputExtension);
plain_global!(WlWebosSystemUx);
plain_global!(WlDataDeviceManager);

impl Dispatch<WlWebosXinputExtension, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        _resource: &WlWebosXinputExtension,
        request: wl_webos_xinput_extension::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        if let wl_webos_xinput_extension::Request::RegisterInput { id } = request {
            data_init.init(id, ());
            state.note_request("wl_webos_xinput_extension", 0, "register_input");
        }
    }
}

noop_dispatch!(WlWebosXinput, ());
noop_dispatch!(WlWebosSystemUx, ());

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        _resource: &WlDataDeviceManager,
        request: wl_data_device_manager::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        match request {
            wl_data_device_manager::Request::CreateDataSource { id } => {
                data_init.init(id, ());
            }
            wl_data_device_manager::Request::GetDataDevice { id, .. } => {
                data_init.init(id, ());
            }
            _ => {}
        }
    }
}

noop_dispatch!(WlDataDevice, ());
noop_dispatch!(WlDataSource, ());
