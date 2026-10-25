//! `wl_seat`, `wl_keyboard`, `wl_pointer`, `wl_touch`: the objects, keyboard focus and the pointer. Key and
//! pointer events come in C4 from the `lginput2` evdev devices (`input.rs`); the data model (focus follows the
//! scene's top window, a keymap is served, the pointer enters the window under it) is here so a client that waits
//! for `wl_keyboard.enter` before drawing gets it.

use std::io::Write;
use std::os::fd::AsFd;

use wayland_server::protocol::wl_keyboard::{self, KeyState, KeymapFormat, WlKeyboard};
use wayland_server::protocol::wl_pointer::{self, WlPointer};
use wayland_server::protocol::wl_seat::{self, Capability, WlSeat};
use wayland_server::protocol::wl_surface::WlSurface;
use wayland_server::protocol::wl_touch::{self, WlTouch};
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};

use crate::State;
use crate::surface::SurfaceId;

#[derive(Default)]
pub struct SeatState {
    pub keyboards: Vec<WlKeyboard>,
    /// Every bound `wl_webos_input_manager`, for the cursor visibility broadcast.
    pub input_managers: Vec<lumatv_wl_protocols::webos_input_manager::server::wl_webos_input_manager::WlWebosInputManager>,
    pub pointers: Vec<WlPointer>,
    pub touches: Vec<WlTouch>,
    pub focus: Option<SurfaceId>,
    /// `wl_surface` resources by surface id, for enter/leave.
    pub surfaces: std::collections::HashMap<SurfaceId, WlSurface>,
    /// The last input kind seen (`getLastInputType`): "key" or "pointer".
    pub last_input: &'static str,
    pub cursor_visible: bool,
    pub pointer_position: (i32, i32),
    /// The surface under the pointer that was last entered, and the output rectangle it was entered at.
    pub pointer_focus: Option<SurfaceId>,
    pub pointer_focus_rect: crate::Rect,
    /// When the cursor was last shown or moved, for the timeout.
    pub cursor_shown_at: Option<std::time::Instant>,
    pub motions: u64,
    pub mrcu_standby_timer: u32,
}

/// Linux `BTN_LEFT`: what the Magic Remote's OK is to a client while the cursor shows.
pub const BTN_LEFT: u32 = 0x110;
/// A cursor that has not moved for this long hides (LG's default cursor timeout is in this range).
pub const CURSOR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A minimal xkb keymap: evdev codes pass through as keycodes (xkb code = evdev + 8). Covers the keys the remote
/// sends, including KEY_PREVIOUS (412), which is the remote's Back on this set, and the colour keys. Clients
/// that want symbols map these themselves (LG's platform plugins do); clients that only want codes ignore it.
pub const KEYMAP: &str = "xkb_keymap {
xkb_keycodes \"evdev\" { minimum = 8; maximum = 708;
 <ESC> = 9; <LEFT> = 113; <RIGHT> = 114; <UP> = 111; <DOWN> = 116; <RTRN> = 36; <BKSP> = 22; <SPCE> = 65;
 <AE01> = 10; <AE02> = 11; <AE03> = 12; <AE04> = 13; <AE05> = 14; <AE06> = 15; <AE07> = 16; <AE08> = 17; <AE09> = 18; <AE10> = 19;
 <I166> = 166; <I172> = 172; <I164> = 164; <I420> = 420; <I398> = 398; <I399> = 399; <I400> = 400; <I401> = 401;
 <I402> = 402; <I403> = 403; <I115> = 123; <I114> = 122; <I113> = 121; <I207> = 215; <I119> = 127; <I128> = 136; <I208> = 216; <I168> = 176;
};
xkb_types \"basic\" { type \"ONE_LEVEL\" { modifiers = none; level_name[Level1] = \"Any\"; }; };
xkb_compatibility \"basic\" { };
xkb_symbols \"basic\" {
 key <ESC> { [ Escape ] }; key <LEFT> { [ Left ] }; key <RIGHT> { [ Right ] }; key <UP> { [ Up ] }; key <DOWN> { [ Down ] };
 key <RTRN> { [ Return ] }; key <BKSP> { [ BackSpace ] }; key <SPCE> { [ space ] };
 key <AE01> { [ 1 ] }; key <AE02> { [ 2 ] }; key <AE03> { [ 3 ] }; key <AE04> { [ 4 ] }; key <AE05> { [ 5 ] };
 key <AE06> { [ 6 ] }; key <AE07> { [ 7 ] }; key <AE08> { [ 8 ] }; key <AE09> { [ 9 ] }; key <AE10> { [ 0 ] };
};
};
";

impl GlobalDispatch<WlSeat, ()> for State {
    fn bind(state: &mut Self, _handle: &DisplayHandle, client: &Client, resource: New<WlSeat>, _global_data: &(), data_init: &mut DataInit<'_, Self>) {
        state.note_bind(client, "wl_seat");
        let seat = data_init.init(resource, ());
        seat.capabilities(Capability::Keyboard | Capability::Pointer);
        if seat.version() >= 2 {
            seat.name("seat0".into());
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        _resource: &WlSeat,
        request: wl_seat::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        match request {
            wl_seat::Request::GetKeyboard { id } => {
                let kb = data_init.init(id, ());
                send_keymap(&kb);
                if kb.version() >= 4 {
                    kb.repeat_info(0, 0);
                }
                state.seat.keyboards.push(kb);
                state.note_request("wl_seat", 0, "get_keyboard");
                refocus(state);
            }
            wl_seat::Request::GetPointer { id } => {
                let p = data_init.init(id, ());
                state.seat.pointers.push(p);
                state.note_request("wl_seat", 0, "get_pointer");
            }
            wl_seat::Request::GetTouch { id } => {
                let t = data_init.init(id, ());
                state.seat.touches.push(t);
                state.note_request("wl_seat", 0, "get_touch");
            }
            wl_seat::Request::Release => {}
            _ => {}
        }
    }
}

fn send_keymap(kb: &WlKeyboard) {
    match memfd_with(KEYMAP.as_bytes()) {
        Ok(fd) => kb.keymap(KeymapFormat::XkbV1, fd.as_fd(), KEYMAP.len() as u32 + 1),
        Err(e) => {
            log::warn!("keymap memfd: {e}");
            // A keyboard with no keymap still delivers codes.
            if let Ok(fd) = memfd_with(b"") {
                kb.keymap(KeymapFormat::NoKeymap, fd.as_fd(), 0);
            }
        }
    }
}

fn memfd_with(bytes: &[u8]) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    // SAFETY: memfd_create with a static name; the fd is owned from here.
    let fd = unsafe { libc::memfd_create(c"lumatv-keymap".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    let mut f = std::fs::File::from(owned.try_clone()?);
    f.write_all(bytes)?;
    f.write_all(&[0])?;
    Ok(owned)
}

impl Dispatch<WlKeyboard, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlKeyboard,
        request: wl_keyboard::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        if let wl_keyboard::Request::Release = request {
            state.seat.keyboards.retain(|k| k.id() != resource.id());
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlPointer,
        request: wl_pointer::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_pointer::Request::SetCursor { surface, .. } => {
                if let Some(s) = surface
                    && let Some(id) = super::compositor::surface_id(&s)
                    && let Some(sf) = state.surfaces.get_mut(id)
                    && sf.role == crate::surface::Role::None
                {
                    sf.role = crate::surface::Role::Cursor;
                }
            }
            wl_pointer::Request::Release => state.seat.pointers.retain(|p| p.id() != resource.id()),
            _ => {}
        }
    }
}

impl Dispatch<WlTouch, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlTouch,
        request: wl_touch::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        if let wl_touch::Request::Release = request {
            state.seat.touches.retain(|t| t.id() != resource.id());
        }
    }
}

/// Moves keyboard focus to the scene's top window (enter/leave on the keyboards of the owning clients).
pub fn refocus(state: &mut State) {
    let new = state.scene.focus(&state.surfaces).and_then(|wid| state.scene.window(wid)).filter(|w| w.needs_focus()).map(|w| w.surface);
    if new == state.seat.focus {
        return;
    }
    let serial = state.next_serial();
    if let Some(old) = state.seat.focus.take()
        && let Some(res) = state.seat.surfaces.get(&old).cloned()
    {
        for kb in state.seat.keyboards.iter().filter(|k| k.id().same_client_as(&res.id())) {
            kb.leave(serial, &res);
        }
    }
    if let Some(n) = new
        && let Some(res) = state.seat.surfaces.get(&n).cloned()
    {
        for kb in state.seat.keyboards.iter().filter(|k| k.id().same_client_as(&res.id())) {
            kb.enter(serial, &res, Vec::new());
            kb.modifiers(serial, 0, 0, 0, 0);
        }
    }
    state.seat.focus = new;
    if let Some(n) = new {
        state.journal(format!("focus -> surface {}", n.0));
    }
}

/// Delivers a key to the focused surface's client (C4 feeds this from evdev; tests and the control socket now).
pub fn send_key(state: &mut State, code: u32, pressed: bool) -> bool {
    let Some(focus) = state.seat.focus else { return false };
    let Some(res) = state.seat.surfaces.get(&focus).cloned() else { return false };
    let serial = state.next_serial();
    let time = crate::output::monotonic_ms();
    let mut sent = false;
    for kb in state.seat.keyboards.iter().filter(|k| k.id().same_client_as(&res.id())) {
        kb.key(serial, time, code, if pressed { KeyState::Pressed } else { KeyState::Released });
        sent = true;
    }
    state.seat.last_input = "key";
    sent
}

/// The pointer moved to `(x, y)` in output pixels: the cursor shows, the surface under it is entered (the
/// previous one left) and gets the motion in its own coordinates. Returns whether a client got it.
pub fn pointer_motion(state: &mut State, x: i32, y: i32) -> bool {
    let (x, y) = (x.clamp(0, crate::OUTPUT_WIDTH as i32 - 1), y.clamp(0, crate::OUTPUT_HEIGHT as i32 - 1));
    state.seat.pointer_position = (x, y);
    state.seat.last_input = "pointer";
    set_cursor_visible(state, true);
    state.seat.cursor_shown_at = Some(std::time::Instant::now());
    state.seat.motions += 1;
    let hit = state.scene.surface_at(x, y);
    let target = hit.map(|(s, _)| s);
    let time = crate::output::monotonic_ms();
    if target != state.seat.pointer_focus {
        let serial = state.next_serial();
        if let Some(old) = state.seat.pointer_focus.take()
            && let Some(res) = state.seat.surfaces.get(&old).cloned()
            && res.is_alive()
        {
            for p in state.seat.pointers.iter().filter(|p| p.id().same_client_as(&res.id())) {
                p.leave(serial, &res);
            }
        }
        if let Some((n, r)) = hit
            && let Some(res) = state.seat.surfaces.get(&n).cloned()
        {
            for p in state.seat.pointers.iter().filter(|p| p.id().same_client_as(&res.id())) {
                p.enter(serial, &res, (x - r.x) as f64, (y - r.y) as f64);
            }
            state.seat.pointer_focus = Some(n);
            state.seat.pointer_focus_rect = r;
        }
    } else if let Some((_, r)) = hit {
        state.seat.pointer_focus_rect = r;
    }
    let Some(n) = state.seat.pointer_focus else { return false };
    let Some(res) = state.seat.surfaces.get(&n).cloned() else { return false };
    let r = state.seat.pointer_focus_rect;
    let mut sent = false;
    for p in state.seat.pointers.iter().filter(|p| p.id().same_client_as(&res.id())) {
        p.motion(time, (x - r.x) as f64, (y - r.y) as f64);
        sent = true;
    }
    sent
}

/// OK while the cursor shows: `BTN_LEFT` to the surface under the pointer.
pub fn pointer_button(state: &mut State, pressed: bool) -> bool {
    let Some(n) = state.seat.pointer_focus else { return false };
    let Some(res) = state.seat.surfaces.get(&n).cloned() else { return false };
    let serial = state.next_serial();
    let time = crate::output::monotonic_ms();
    let mut sent = false;
    for p in state.seat.pointers.iter().filter(|p| p.id().same_client_as(&res.id())) {
        p.button(serial, time, BTN_LEFT, if pressed { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released });
        sent = true;
    }
    state.seat.last_input = "pointer";
    state.seat.cursor_shown_at = Some(std::time::Instant::now());
    sent
}

/// Hides the cursor (a navigation key, the timeout, a client's `set_cursor_visibility(0)`): the surface under it
/// is left, so a hover state does not linger.
pub fn hide_cursor(state: &mut State) {
    set_cursor_visible(state, false);
    if let Some(old) = state.seat.pointer_focus.take()
        && let Some(res) = state.seat.surfaces.get(&old).cloned()
        && res.is_alive()
    {
        let serial = state.next_serial();
        for p in state.seat.pointers.iter().filter(|p| p.id().same_client_as(&res.id())) {
            p.leave(serial, &res);
        }
    }
}

/// Every client hears when the cursor appears or goes (`wl_webos_input_manager.cursor_visibility`): LG's web and
/// native apps switch between pointer hover and 5-way focus on it.
fn set_cursor_visible(state: &mut State, visible: bool) {
    if state.seat.cursor_visible == visible {
        return;
    }
    state.seat.cursor_visible = visible;
    state.seat.input_managers.retain(|r| r.is_alive());
    for r in &state.seat.input_managers {
        r.cursor_visibility(visible as u32, None);
    }
}
