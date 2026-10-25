//! `wl_webos_shell` / `wl_webos_shell_surface`: LG's window types, properties, states and key masks.

use wayland_backend::server::ClientId;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use lumatv_wl_protocols::webos_shell::server::wl_webos_shell::{self, WlWebosShell};
use lumatv_wl_protocols::webos_shell::server::wl_webos_shell_surface::{self, State as WlState, WlWebosShellSurface};

use super::compositor::surface_id;
use super::shell::take_toplevel_role;
use crate::surface::{Role, SurfaceId};
use crate::window::{ShellState, WindowId};
use crate::{State, plain_global};

pub struct WebosShellSurfaceData {
    pub surface: SurfaceId,
    pub window: WindowId,
}

plain_global!(WlWebosShell);

impl Dispatch<WlWebosShell, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &WlWebosShell,
        request: wl_webos_shell::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        match request {
            wl_webos_shell::Request::GetShellSurface { id, surface } => {
                let Some(sid) = surface_id(&surface) else {
                    resource.post_error(0u32, "unknown surface");
                    return;
                };
                if !take_toplevel_role(state, sid) {
                    resource.post_error(0u32, "surface already has another role");
                    return;
                }
                let wid = state.scene.create_window(sid);
                let ss = data_init.init(id, WebosShellSurfaceData { surface: sid, window: wid });
                state.webos_shells.insert(wid, ss.clone());
                if let Some(w) = state.scene.window_mut(wid) {
                    w.has_webos_shell = true;
                }
                state.note_request(
                    "wl_webos_shell",
                    0,
                    &format!("get_shell_surface -> wl_webos_shell_surface@{} (window {}, surface {})", ss.id().protocol_id(), wid.0, sid.0),
                );
                state.journal(format!("webOS shell surface on window {} (surface {}) by {}", wid.0, sid.0, state.client_info(client)));
            }
            wl_webos_shell::Request::GetSystemPip => state.note_request("wl_webos_shell", 0, "get_system_pip (ignored)"),
            _ => {}
        }
    }
}

impl Dispatch<WlWebosShellSurface, WebosShellSurfaceData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlWebosShellSurface,
        request: wl_webos_shell_surface::Request,
        data: &WebosShellSurfaceData,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        let pid = resource.id().protocol_id();
        match request {
            wl_webos_shell_surface::Request::SetLocationHint { hint } => {
                state.note_request("wl_webos_shell_surface", pid, &format!("set_location_hint({hint})"));
                if let Some(w) = state.scene.window_mut(data.window) {
                    w.location_hint = hint;
                }
            }
            wl_webos_shell_surface::Request::SetState { state: st } => {
                state.note_request("wl_webos_shell_surface", pid, &format!("set_state({st})"));
                let new = ShellState::from_u32(st);
                if let Some(w) = state.scene.window_mut(data.window) {
                    w.state = new;
                }
                // Answered at once, mapped or not: a client may hold its first frame until the state it asked for is
                // confirmed, and HDMI 2's card stayed fullscreen but unmapped on the 2026-10-25 gate while only
                // mapped windows were answered.
                send_state(resource, new);
                super::seat::refocus(state);
            }
            wl_webos_shell_surface::Request::SetProperty { name, value } => {
                state.note_request("wl_webos_shell_surface", pid, &format!("set_property({name:?}, {value:?})"));
                let mut line = None;
                if let Some(w) = state.scene.window_mut(data.window) {
                    let relayer = w.set_property(&name, &value);
                    if relayer || name == "appId" || name == "title" {
                        line = Some(format!("window {}: {name} = {value} ({:?})", data.window.0, w.kind));
                    }
                }
                if let Some(l) = line {
                    state.journal(l);
                }
            }
            wl_webos_shell_surface::Request::SetKeyMask { webos_key } => {
                let mask = webos_key;
                state.note_request("wl_webos_shell_surface", pid, &format!("set_key_mask(0x{mask:x})"));
                if let Some(w) = state.scene.window_mut(data.window) {
                    w.key_mask = mask;
                }
            }
            wl_webos_shell_surface::Request::SetSize { width, height } => {
                state.note_request("wl_webos_shell_surface", pid, &format!("set_size({width}, {height})"));
                // LG-only; recorded with the properties so a TV session shows which apps send it.
                if let Some(w) = state.scene.window_mut(data.window) {
                    w.props.insert("lg.setSize".into(), format!("{width}x{height}"));
                }
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, _resource: &WlWebosShellSurface, data: &WebosShellSurfaceData) {
        state.webos_shells.remove(&data.window);
        let keep = state.scene.window(data.window).map(|w| w.has_wl_shell).unwrap_or(false);
        if keep {
            if let Some(w) = state.scene.window_mut(data.window) {
                w.has_webos_shell = false;
            }
        } else {
            state.scene.remove_window(data.window);
            if let Some(s) = state.surfaces.get_mut(data.surface) {
                s.role = Role::None;
            }
            super::seat::refocus(state);
        }
    }
}

fn send_state(resource: &WlWebosShellSurface, st: ShellState) {
    let v = match st {
        ShellState::Default => WlState::Default,
        ShellState::Minimized => WlState::Minimized,
        ShellState::Maximized => WlState::Maximized,
        ShellState::Fullscreen => WlState::Fullscreen,
    };
    resource.state_about_to_change(v as u32);
    resource.state_changed(v as u32);
}

/// A window's surface got its first buffer: a card becomes fullscreen (what LG's fullscreen container does),
/// and the client is told its position.
pub fn on_mapped(state: &mut State, wid: WindowId) {
    let Some(w) = state.scene.window(wid) else { return };
    let (kind, position, has_webos) = (w.kind, w.position, w.has_webos_shell);
    if !has_webos {
        return;
    }
    let Some(res) = state.webos_shell_resource(wid) else { return };
    if kind.is_card() {
        if let Some(w) = state.scene.window_mut(wid) {
            w.state = ShellState::Fullscreen;
        }
        send_state(&res, ShellState::Fullscreen);
        res.position_changed(0, 0);
    } else {
        res.position_changed(position.0, position.1);
    }
}

impl State {
    /// The `wl_webos_shell_surface` of a window, if it has one.
    pub fn webos_shell_resource(&self, wid: WindowId) -> Option<WlWebosShellSurface> {
        self.webos_shells.get(&wid).cloned()
    }
}
