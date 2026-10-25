//! Globals LG's clients bind at start-up and that need objects, not behaviour, for a first app: xinput, the text
//! model and input method (the keyboard, phase 6), system UX, and the data device manager.

use wayland_server::protocol::wl_data_device::WlDataDevice;
use wayland_server::protocol::wl_data_device_manager::{self, WlDataDeviceManager};
use wayland_server::protocol::wl_data_source::WlDataSource;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use lumatv_wl_protocols::input_method::server::input_method::InputMethod;
use lumatv_wl_protocols::input_method::server::input_method_context::InputMethodContext;
use lumatv_wl_protocols::input_method::server::input_panel::{self, InputPanel};
use lumatv_wl_protocols::input_method::server::input_panel_surface::InputPanelSurface;
use lumatv_wl_protocols::text::server::text_model::{self, TextModel};
use lumatv_wl_protocols::text::server::text_model_factory::{self, TextModelFactory};
use lumatv_wl_protocols::webos_extension::server::wl_webos_xinput::WlWebosXinput;
use lumatv_wl_protocols::webos_extension::server::wl_webos_xinput_extension::{self, WlWebosXinputExtension};
use lumatv_wl_protocols::webos_system_ux::server::wl_webos_system_ux::WlWebosSystemUx;

use super::compositor::surface_id;
use crate::surface::Role;
use crate::{State, noop_dispatch, plain_global};

plain_global!(WlWebosXinputExtension);
plain_global!(TextModelFactory);
plain_global!(InputMethod);
plain_global!(InputPanel);
plain_global!(WlWebosSystemUx);
plain_global!(WlDataDeviceManager);

impl Dispatch<WlWebosXinputExtension, ()> for State {
    fn request(state: &mut Self, client: &Client, _resource: &WlWebosXinputExtension, request: wl_webos_xinput_extension::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        state.touch_client(client);
        if let wl_webos_xinput_extension::Request::RegisterInput { id } = request {
            data_init.init(id, ());
            state.note_request("wl_webos_xinput_extension", 0, "register_input");
        }
    }
}

noop_dispatch!(WlWebosXinput, ());

impl Dispatch<TextModelFactory, ()> for State {
    fn request(state: &mut Self, client: &Client, _resource: &TextModelFactory, request: text_model_factory::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        state.touch_client(client);
        if let text_model_factory::Request::CreateTextModel { id } = request {
            data_init.init(id, ());
            state.note_request("text_model_factory", 0, "create_text_model");
        }
    }
}

impl Dispatch<TextModel, ()> for State {
    fn request(state: &mut Self, _client: &Client, resource: &TextModel, request: text_model::Request, _data: &(), _dh: &DisplayHandle, _di: &mut DataInit<'_, Self>) {
        let pid = resource.id().protocol_id();
        match &request {
            text_model::Request::Activate { surface, .. } => {
                state.note_request("text_model", pid, &format!("activate(surface {:?})", surface_id(surface).map(|s| s.0)));
                // No IME yet: the model enters and leaves at once so a client's focus logic does not wait.
                resource.enter(surface);
            }
            text_model::Request::Deactivate { .. } => {
                state.note_request("text_model", pid, "deactivate");
                resource.leave();
            }
            other => state.note_request("text_model", pid, &format!("{other:?}")),
        }
    }
}

noop_dispatch!(InputMethod, ());
noop_dispatch!(InputMethodContext, ());

impl Dispatch<InputPanel, ()> for State {
    fn request(state: &mut Self, client: &Client, _resource: &InputPanel, request: input_panel::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        state.touch_client(client);
        if let input_panel::Request::GetInputPanelSurface { id, surface } = request {
            data_init.init(id, ());
            if let Some(sid) = surface_id(&surface)
                && let Some(s) = state.surfaces.get_mut(sid)
                && s.role == Role::None
            {
                s.role = Role::InputPanel;
            }
            state.note_request("input_panel", 0, "get_input_panel_surface");
        }
    }
}

noop_dispatch!(InputPanelSurface, ());
noop_dispatch!(WlWebosSystemUx, ());

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn request(state: &mut Self, client: &Client, _resource: &WlDataDeviceManager, request: wl_data_device_manager::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
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
