//! `wl_shell` / `wl_shell_surface`: every LG client creates its window through this (then adds the webOS shell
//! surface for the LG-specific state).

use wayland_backend::server::ClientId;
use wayland_server::protocol::wl_shell::{self, WlShell};
use wayland_server::protocol::wl_shell_surface::{self, WlShellSurface};
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use super::compositor::surface_id;
use crate::surface::{Role, SurfaceId};
use crate::window::WindowId;
use crate::{State, plain_global};

pub struct ShellSurfaceData {
    pub surface: SurfaceId,
    pub window: WindowId,
}

plain_global!(WlShell);

impl Dispatch<WlShell, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &WlShell,
        request: wl_shell::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        if let wl_shell::Request::GetShellSurface { id, surface } = request {
            let Some(sid) = surface_id(&surface) else {
                resource.post_error(wl_shell::Error::Role, "unknown surface");
                return;
            };
            if !take_toplevel_role(state, sid) {
                resource.post_error(wl_shell::Error::Role, "surface already has another role");
                return;
            }
            let wid = state.scene.create_window(sid);
            let ss = data_init.init(id, ShellSurfaceData { surface: sid, window: wid });
            if let Some(w) = state.scene.window_mut(wid) {
                w.has_wl_shell = true;
            }
            state.note_request(
                "wl_shell",
                0,
                &format!("get_shell_surface -> wl_shell_surface@{} (window {}, surface {})", ss.id().protocol_id(), wid.0, sid.0),
            );
            state.journal(format!("window {} created on surface {} by {}", wid.0, sid.0, state.client_info(client)));
        }
    }
}

/// Gives `sid` the toplevel role; a second shell object on the same surface (wl_shell plus webOS shell) is fine.
pub fn take_toplevel_role(state: &mut State, sid: SurfaceId) -> bool {
    let Some(s) = state.surfaces.get_mut(sid) else { return false };
    match s.role {
        Role::None => {
            s.role = Role::Toplevel;
            true
        }
        Role::Toplevel => true,
        _ => false,
    }
}

impl Dispatch<WlShellSurface, ShellSurfaceData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlShellSurface,
        request: wl_shell_surface::Request,
        data: &ShellSurfaceData,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        let pid = resource.id().protocol_id();
        match request {
            wl_shell_surface::Request::Pong { serial } => state.note_request("wl_shell_surface", pid, &format!("pong({serial})")),
            wl_shell_surface::Request::SetToplevel => {
                state.note_request("wl_shell_surface", pid, "set_toplevel");
                // A card takes the whole output; LG's compositor configures it to 1080p on the way in.
                let (w, h) = state.output.size();
                resource.configure(wl_shell_surface::Resize::None, w as i32, h as i32);
            }
            wl_shell_surface::Request::SetTitle { title } => {
                state.note_request("wl_shell_surface", pid, &format!("set_title({title:?})"));
                // The same as the webos shell's `title` property: the shell client's windows are known by theirs.
                let line = state
                    .scene
                    .window_mut(data.window)
                    .and_then(|w| w.set_property("title", &title).then(|| format!("window {}: title = {title} ({:?})", data.window.0, w.kind)));
                if let Some(l) = line {
                    state.journal(l);
                    state.refresh_policy_sync();
                }
            }
            wl_shell_surface::Request::SetClass { class_ } => {
                state.note_request("wl_shell_surface", pid, &format!("set_class({class_:?})"));
                if let Some(w) = state.scene.window_mut(data.window) {
                    w.class = class_;
                }
            }
            wl_shell_surface::Request::SetFullscreen { .. } | wl_shell_surface::Request::SetMaximized { .. } => {
                state.note_request("wl_shell_surface", pid, "set_fullscreen/maximized");
                let (w, h) = state.output.size();
                resource.configure(wl_shell_surface::Resize::None, w as i32, h as i32);
            }
            // A window with a transient parent (the shell client's system layer, a QML `Window` nested in another) is configured as a
            // toplevel is: Qt's Wayland client exposes a wl_shell window at its first configure, and one it never gets never draws (the
            // shell's system layer was created at its first showing and mapped only at a later one, gate run 2026-10-27 21:05).
            wl_shell_surface::Request::SetTransient { .. } | wl_shell_surface::Request::SetPopup { .. } => {
                state.note_request("wl_shell_surface", pid, "set_transient/popup");
                let (w, h) = state.output.size();
                resource.configure(wl_shell_surface::Resize::None, w as i32, h as i32);
            }
            other => state.note_request("wl_shell_surface", pid, &format!("{other:?} (ignored)")),
        }
    }

    fn destroyed(state: &mut Self, client: ClientId, _resource: &WlShellSurface, data: &ShellSurfaceData) {
        state.trace_client(Some(client), "wl-shell-surface-destroyed", 0, true);
        let keep = state.scene.window(data.window).map(|w| w.has_webos_shell).unwrap_or(false);
        if keep {
            if let Some(w) = state.scene.window_mut(data.window) {
                w.has_wl_shell = false;
            }
        } else {
            let window = state.scene.window(data.window).map(|w| w.kind.is_card());
            state.scene.remove_window(data.window);
            if let Some(s) = state.surfaces.get_mut(data.surface) {
                s.role = Role::None;
            }
            match window {
                Some(was_card) => super::webos_shell::window_gone(state, was_card),
                None => super::seat::refocus(state),
            }
        }
    }
}
