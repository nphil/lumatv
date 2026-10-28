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
                // A window attached to a surface group is no card (LG: `__allowedFullscreen`): it never comes to the front, whatever it asks for.
                let card = state.scene.window(data.window).is_some_and(|w| w.kind.is_card() && !w.in_group);
                let was_top = state.scene.foreground(&state.surfaces).is_some_and(|w| w.id == data.window);
                if let Some(w) = state.scene.window_mut(data.window) {
                    w.state = new;
                }
                // Answered at once, mapped or not: a client may hold its first frame until the state it asked for is
                // confirmed, and HDMI 2's card stayed fullscreen but unmapped on the 2026-10-25 gate while only
                // mapped windows were answered.
                send_state(resource, new);
                // A card that asks for fullscreen comes to the front, with or without a buffer yet (LG's fullscreen
                // container; a relaunch restores a covered card, and a preloaded app relaunched draws only once SAM
                // has it foreground, which SAM learns here: third gate, 2026-10-25), and the card it covers is
                // minimized; a foreground card that minimizes itself gives the one below back. Either is a new
                // foreground for the subscribers.
                if card && new == ShellState::Fullscreen && !was_top {
                    state.scene.raise(data.window);
                    state.journal(format!("window {}: raised to the front (set_state fullscreen)", data.window.0));
                    settle_cards(state, None);
                    state.luna_notify();
                } else if card && new == ShellState::Minimized && was_top {
                    settle_cards(state, Some(data.window));
                    state.luna_notify();
                }
                super::seat::refocus(state);
            }
            wl_webos_shell_surface::Request::SetProperty { name, value } => {
                state.note_request("wl_webos_shell_surface", pid, &format!("set_property({name:?}, {value:?})"));
                let mut line = None;
                let mut shell_shown = false;
                if let Some(w) = state.scene.window_mut(data.window) {
                    let relayer = w.set_property(&name, &value);
                    if relayer || name == "appId" || name == "title" {
                        line = Some(format!("window {}: {name} = {value} ({:?})", data.window.0, w.kind));
                    }
                    shell_shown = name == "title" && w.kind.is_shell();
                }
                if let Some(l) = line {
                    state.journal(l);
                }
                // The shell client sends its windows' properties at every showing. LG's Qt platform plugin makes Qt's expose of the
                // compositor's `exposed`, and Qt's own expose is missing at the first showing of a window whose platform window was
                // made earlier (the system layer, made with the client and shown later: never drew, gate runs 2026-10-27 21:05 and
                // 21:11); the rectangle is the output's, as for a card that has no buffer yet.
                if shell_shown {
                    let (ow, oh) = state.output.size();
                    send_exposed(resource, Some((ow as i32, oh as i32)));
                    if let Some(w) = state.scene.window_mut(data.window) {
                        w.exposed = true;
                    }
                    state.journal(format!("window {}: exposed {ow}x{oh} (a shell window shown)", data.window.0));
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
            let was_card = state.scene.window(data.window).is_some_and(|w| w.kind.is_card());
            state.scene.remove_window(data.window);
            if let Some(s) = state.surfaces.get_mut(data.surface) {
                s.role = Role::None;
            }
            if was_card {
                on_card_gone(state);
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

/// A window's surface got its first buffer: a card becomes fullscreen and the cards it covers are minimized (what LG's
/// fullscreen container does), and the client is told its position. A group member takes its owner's state instead, and SAM is told
/// that the group has one more.
pub fn on_mapped(state: &mut State, wid: WindowId) {
    let Some(w) = state.scene.window(wid) else { return };
    let (kind, position, has_webos, member) = (w.kind, w.position, w.has_webos_shell, w.in_group);
    if member {
        sync_group_members(state);
        state.luna_notify();
    } else if kind.is_card() {
        settle_cards(state, None);
    } else if kind.is_shell() {
        sync_exposed(state);
        state.shell_window_mapped(kind);
    }
    if !has_webos {
        return;
    }
    let Some(res) = state.webos_shell_resource(wid) else { return };
    if kind.is_card() {
        res.position_changed(0, 0);
    } else {
        res.position_changed(position.0, position.1);
    }
}

/// A surface group's members changed (an attach, a detach, a layer moved): SAM gets the list with the group as it is, the keys go where
/// LG's group model puts them, the members take their owners' states, and, when a window that was at the front as a card is a member now
/// (`card_left`), the card it covered comes back.
pub fn group_changed(state: &mut State, card_left: bool) {
    if card_left {
        settle_cards(state, None);
    } else {
        sync_group_members(state);
    }
    state.luna_notify();
    super::seat::refocus(state);
}

/// A card's surface lost its last buffer, or its window went: the card below comes back.
pub fn on_card_gone(state: &mut State) {
    settle_cards(state, None);
}

/// The cards' states as LG's fullscreen container keeps them: the card at the front is fullscreen and every other card
/// that was at the front is minimized, and each is told when its state changes. A client that restores fullscreen on
/// a relaunch (WAM, from a minimized window) then has a state to change; without this Home's relaunch over HDMI 2 sent
/// nothing and HDMI 2 stayed in front (second gate, 2026-10-25). `except` is a card that asked to be minimized itself:
/// it keeps that, and the one below it comes back. A card that never came to the front (a preloaded app's window
/// before its relaunch, bufferless in state Default) is left as it is. The card that ends up in front is then exposed
/// and every other card unexposed.
pub fn settle_cards(state: &mut State, except: Option<WindowId>) {
    if let Some(top) = state.scene.front_card(&state.surfaces, except) {
        let changes: Vec<(WindowId, ShellState)> = state
            .scene
            .windows()
            .filter(|w| w.kind.is_card() && w.front > 0 && Some(w.id) != except)
            .filter(|w| w.map_serial == 0 || state.surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false))
            .filter_map(|w| {
                let wanted = if w.id == top { ShellState::Fullscreen } else { ShellState::Minimized };
                (w.state != wanted).then_some((w.id, wanted))
            })
            .collect();
        for (wid, wanted) in changes {
            if let Some(w) = state.scene.window_mut(wid) {
                w.state = wanted;
            }
            if let Some(res) = state.webos_shell_resource(wid) {
                send_state(&res, wanted);
            }
            state.journal(format!(
                "window {}: {}",
                wid.0,
                if wanted == ShellState::Fullscreen { "fullscreen (the front card)" } else { "minimized (covered by a newer card)" }
            ));
        }
    }
    sync_exposed(state);
    sync_group_members(state);
}

/// Tells each card what `wl_webos_shell_surface.exposed` it has: the foreground card its whole rectangle, every other
/// card none. OSE's compositor sends the rectangle (0, 0, surface size) when a card becomes fullscreen and nothing
/// else, and LG's Qt platform plugin turns the event into Qt's expose, without which a Qt window does not render (the
/// preloaded HDMI 2 app never drew on the fourth gate, 2026-10-25). A card raised before its first buffer has no
/// surface size yet; it gets the output's, the rectangle a card fills [INFERENCE: OSE would send an empty list
/// there, which Qt 5 takes as an expose all the same]. The empty list for a covered or minimized card is this
/// compositor's reading of the protocol ("areas that are visible in this surface"); OSE never unexposes a card.
/// The shell client's windows (Qt as well) are exposed whole for as long as they are mapped.
pub fn sync_exposed(state: &mut State) {
    let fg = state.scene.foreground(&state.surfaces).map(|w| w.id);
    let changes: Vec<(WindowId, bool)> = state
        .scene
        .windows()
        .filter(|w| (w.kind.is_card() || w.kind.is_shell()) && w.has_webos_shell && !w.in_group)
        .filter_map(|w| {
            let wanted = if w.kind.is_shell() { w.map_serial > 0 } else { Some(w.id) == fg };
            (w.exposed != wanted).then_some((w.id, wanted))
        })
        .collect();
    for (wid, wanted) in changes {
        let Some(w) = state.scene.window_mut(wid) else { continue };
        w.exposed = wanted;
        let surface = w.surface;
        let rect = wanted.then(|| exposure_rect(state, surface));
        if let Some(res) = state.webos_shell_resource(wid) {
            send_exposed(&res, rect);
        }
        state.journal(match rect {
            Some((w, h)) => format!("window {}: exposed {w}x{h}", wid.0),
            None => format!("window {}: exposed nothing (covered)", wid.0),
        });
    }
}

/// The rectangle a window is exposed: its surface's size once it has a buffer, the output's before (a card raised before its first
/// buffer, the rectangle a card fills).
fn exposure_rect(state: &State, surface: SurfaceId) -> (i32, i32) {
    let (sw, sh) = state.surfaces.get(surface).filter(|s| s.is_mapped()).map(|s| s.size()).unwrap_or((0, 0));
    if sw > 0 && sh > 0 {
        (sw, sh)
    } else {
        let (ow, oh) = state.output.size();
        (ow as i32, oh as i32)
    }
}

/// The state and the exposure of each group member follow its owner's. LG's group model joins a child fullscreen and copies its owner's
/// state to it whenever that changes (`StarfishGroupModel.__handleChildItem`, `updateGroupState`): a member of an owner that is covered is
/// minimized (a web app hides then), one of an owner in front is fullscreen. [INFERENCE] `exposed` follows the owner the same way: no
/// source of LG's compositor sets it for a child. A member that is not mapped yet, or whose owner is gone or not mapped, is left as it
/// is; each change is sent once.
pub fn sync_group_members(state: &mut State) {
    let changes: Vec<(WindowId, ShellState, bool)> = state
        .scene
        .windows()
        .filter(|w| w.in_group && w.has_webos_shell && w.map_serial > 0)
        .filter_map(|w| {
            let owner = state.scene.group_of_member(w.surface).and_then(|g| state.scene.window_of(g.owner)).and_then(|o| state.scene.window(o))?;
            let wanted = match owner.state {
                ShellState::Minimized => ShellState::Minimized,
                _ if owner.map_serial > 0 => ShellState::Fullscreen,
                _ => return None,
            };
            let exposed = owner.exposed && wanted == ShellState::Fullscreen;
            (w.state != wanted || w.exposed != exposed).then_some((w.id, wanted, exposed))
        })
        .collect();
    for (wid, wanted, exposed) in changes {
        let Some(w) = state.scene.window_mut(wid) else { continue };
        let (state_changed, exposure_changed) = (w.state != wanted, w.exposed != exposed);
        w.state = wanted;
        w.exposed = exposed;
        let surface = w.surface;
        let res = state.webos_shell_resource(wid);
        if state_changed {
            if let Some(res) = &res {
                send_state(res, wanted);
            }
            let what = if wanted == ShellState::Fullscreen { "fullscreen" } else { "minimized" };
            state.journal(format!("window {}: {what} (a group member follows its owner)", wid.0));
        }
        if exposure_changed {
            let rect = exposed.then(|| exposure_rect(state, surface));
            if let Some(res) = &res {
                send_exposed(res, rect);
            }
            state.journal(match rect {
                Some((w, h)) => format!("window {}: exposed {w}x{h} (a group member follows its owner)", wid.0),
                None => format!("window {}: exposed nothing (a group member follows its owner)", wid.0),
            });
        }
    }
}

/// `exposed` with one rectangle at the origin, or none: ints x,y,w,h per rectangle and a closing -1, in the
/// client's byte order (a `wl_array` is memory, not marshalled ints).
fn send_exposed(resource: &WlWebosShellSurface, rect: Option<(i32, i32)>) {
    let mut ints = Vec::with_capacity(5);
    if let Some((w, h)) = rect {
        ints.extend([0, 0, w, h]);
    }
    ints.push(-1);
    resource.exposed(ints.iter().flat_map(|v| v.to_ne_bytes()).collect());
}

impl State {
    /// The `wl_webos_shell_surface` of a window, if it has one.
    pub fn webos_shell_resource(&self, wid: WindowId) -> Option<WlWebosShellSurface> {
        self.webos_shells.get(&wid).cloned()
    }
}
