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

/// One `wl_seat` per input device, as LG's compositor has them (webOS OSE's `WebOSInputDevice`: a seat per device
/// id, keys delivered on the seat of the device that sent them). Seat 0 exists in every build and carries the
/// control socket's and the tests' keys; the evdev devices take 1, 2, … in the order they were opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SeatId(pub u32);

impl SeatId {
    pub fn name(self) -> String {
        format!("seat{}", self.0)
    }
}

/// What `wl_webos_seat.info` says a seat is: seat 0 (the control socket, the tests) is LG's own first seat's name, the
/// others their evdev device's ("LGE RCU", "LGE M-RCU - Builtin [0]", …). LG's SDL numbers the remotes by these names.
pub const SEAT0_DEVICE: &str = "Default USB Mouse input, not initialized yet.";

#[derive(Default)]
pub struct SeatState {
    /// Every bound `wl_keyboard`; its seat is its user data (`SeatId`).
    pub keyboards: Vec<WlKeyboard>,
    /// How many `wl_seat` globals exist (`State::add_seat`).
    pub seats: u32,
    /// The evdev name of each seat's device from seat 1 on (`State::add_seat`), for `wl_webos_seat.info`.
    pub device_names: Vec<String>,
    /// Keys currently down, per seat: what a keyboard made mid-press is told in `enter`.
    pub pressed: Vec<(SeatId, u32)>,
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
    /// A press of the pointer's button was delivered to a client and its release is owed: a release with no press seen is nothing.
    pub button_down: bool,
}

impl SeatState {
    /// The device name `wl_webos_seat.info` reports for a seat.
    pub fn device_name(&self, id: SeatId) -> &str {
        match id.0 {
            0 => SEAT0_DEVICE,
            n => self.device_names.get(n as usize - 1).map(String::as_str).unwrap_or(SEAT0_DEVICE),
        }
    }
}

/// What shows the Magic Remote's pointer (the theme's `remote.pointerShow`, Settings > Remote & pointer > Show pointer). Either way the pointer
/// hides on an arrow key, on the remote's cursor-off report (1199, sent as it is laid down) and when it has been idle for [`CURSOR_TIMEOUT`]; and
/// the remote's own motion only moves a pointer that shows, it never shows one (a resting remote reports one-pixel jitter).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerShow {
    /// A turn of the wheel shows it, where the remote points. The default.
    #[default]
    Wheel,
    /// LG's behaviour: the pointer shows when the remote's input service says it is moved on purpose (its cursor-on report, 1198, which it repeats
    /// every half second while the remote is in the hand), and a wheel turn alone does not show it.
    Movement,
}

impl PointerShow {
    /// The theme's choice: `"movement"` is LG's, anything else (absent, `"wheel"`, a word it does not know) the default.
    pub fn from_theme(theme: &serde_json::Value) -> PointerShow {
        match theme.pointer("/remote/pointerShow").and_then(serde_json::Value::as_str) {
            Some("movement") => PointerShow::Movement,
            _ => PointerShow::Wheel,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PointerShow::Wheel => "wheel",
            PointerShow::Movement => "movement",
        }
    }
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

impl GlobalDispatch<WlSeat, SeatId> for State {
    fn bind(state: &mut Self, _handle: &DisplayHandle, client: &Client, resource: New<WlSeat>, seat: &SeatId, data_init: &mut DataInit<'_, Self>) {
        state.note_bind(client, "wl_seat");
        let res = data_init.init(resource, *seat);
        // Every seat of LG's compositor says keyboard, pointer and touch (probed 2026-10-28), and no `name`, whatever the
        // bound version. Both matter: Cobalt's `DevInputWebOS` never initialises its touch proxy and "destroys" it when a
        // seat has no touch (SIGSEGV 32 ms after YouTube connects), and LG's libwayland-client calls a NULL listener entry
        // rather than aborting, so an event a client has no handler for (`name` in Cobalt's seat listener) is a crash.
        res.capabilities(Capability::Keyboard | Capability::Pointer | Capability::Touch);
    }
}

impl Dispatch<WlSeat, SeatId> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        _resource: &WlSeat,
        request: wl_seat::Request,
        seat: &SeatId,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        match request {
            wl_seat::Request::GetKeyboard { id } => {
                let kb = data_init.init(id, *seat);
                send_keymap(&kb);
                if kb.version() >= 4 {
                    kb.repeat_info(0, 0);
                }
                state.note_request("wl_seat", 0, "get_keyboard");
                enter_if_focused(state, &kb);
                state.seat.keyboards.push(kb);
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

pub(crate) fn send_keymap(kb: &WlKeyboard) {
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

impl Dispatch<WlKeyboard, SeatId> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlKeyboard,
        request: wl_keyboard::Request,
        _data: &SeatId,
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
        for kb in of_client(&state.seat.keyboards, &res) {
            kb.leave(serial, &res);
        }
    }
    if let Some(n) = new
        && let Some(res) = state.seat.surfaces.get(&n).cloned()
    {
        for kb in of_client(&state.seat.keyboards, &res) {
            enter(&state.seat, kb, serial, &res);
        }
    }
    state.seat.focus = new;
    if let Some(n) = new {
        state.journal(format!("focus -> surface {}", n.0));
    }
    // A text field on the surface that lost the focus leaves.
    super::ime::focus_changed(state);
}

/// A keyboard made after focus settled on its client's surface gets `enter` now, as Qt's compositor sends it at
/// the bind (`checkFocusResource`): WAM and LG's Qt apps create their keyboards after their window mapped, and a
/// client that saw no `enter` has no focus to deliver keys into.
fn enter_if_focused(state: &mut State, kb: &WlKeyboard) {
    let Some(focus) = state.seat.focus else { return };
    let Some(res) = state.seat.surfaces.get(&focus).cloned() else { return };
    if !(live(&res) && live(kb) && kb.id().same_client_as(&res.id())) {
        return;
    }
    let serial = state.next_serial();
    enter(&state.seat, kb, serial, &res);
}

/// `enter` with the keys held on the keyboard's seat (little-endian codes, as the protocol's array).
fn enter(seat: &SeatState, kb: &WlKeyboard, serial: u32, surface: &WlSurface) {
    let sid = seat_of(kb);
    let keys: Vec<u8> = seat.pressed.iter().filter(|(s, _)| *s == sid).flat_map(|(_, code)| code.to_le_bytes()).collect();
    kb.enter(serial, surface, keys);
    kb.modifiers(serial, 0, 0, 0, 0);
}

/// Whether an event may still be sent on `r` or carry it: the object is alive and its client still has its
/// data. A client in the middle of disconnecting has objects that are alive but whose client is gone, and the
/// backend's same-client check on an event with an object argument unwraps that lookup: the seventh gate's
/// session panicked (`InvalidId`) in `refocus` while WebAppMgr's first connection closed 1.2 s in.
pub(crate) fn live<R: Resource>(r: &R) -> bool {
    r.is_alive() && r.client().is_some()
}

/// The objects of `list` that belong to `surface`'s client and can still take an event that carries `surface`.
fn of_client<'a, R: Resource>(list: &'a [R], surface: &'a WlSurface) -> impl Iterator<Item = &'a R> + 'a {
    let ok = live(surface);
    list.iter().filter(move |r| ok && r.id().same_client_as(&surface.id()) && live(*r))
}

fn seat_of(kb: &WlKeyboard) -> SeatId {
    kb.data::<SeatId>().copied().unwrap_or(SeatId(0))
}

/// Delivers a key to the focused surface's client on the keyboards of `seat`, the seat of the device that sent
/// it (C4 feeds this from evdev; the control socket and the tests use seat 0).
pub fn send_key(state: &mut State, seat: SeatId, code: u32, pressed: bool) -> bool {
    if pressed {
        if !state.seat.pressed.contains(&(seat, code)) {
            state.seat.pressed.push((seat, code));
        }
    } else {
        state.seat.pressed.retain(|k| *k != (seat, code));
    }
    let Some(focus) = state.seat.focus else { return false };
    let Some(res) = state.seat.surfaces.get(&focus).cloned() else { return false };
    let serial = state.next_serial();
    let time = crate::output::monotonic_ms();
    let mut sent = false;
    for kb in of_client(&state.seat.keyboards, &res).filter(|k| seat_of(k) == seat) {
        kb.key(serial, time, code, if pressed { KeyState::Pressed } else { KeyState::Released });
        sent = true;
    }
    state.seat.last_input = "key";
    sent
}

/// The transport keys the Cameras full view sends to the app it sits over (Linux `KEY_PAUSE` and `KEY_PLAY`, both in [`KEYMAP`]; not yet read
/// on the set from the remote's own Play and Pause).
pub const KEY_PAUSE: u32 = 119;
pub const KEY_PLAY: u32 = 207;

/// Presses and releases `code` for the app `app` when it has the keyboard focus: the Cameras full view pauses the video of the app it is shown
/// over and plays it again on Back, which the app's own key handling takes as the remote's. The keys the layer takes by name never move the
/// focus from the app, so it is the app's while the view is up; any other focus gets nothing, and the answer says so.
pub fn tap_app_key(state: &mut State, app: &str, code: u32) -> bool {
    let focused = state.seat.focus.and_then(|s| state.scene.window_of(s)).and_then(|w| state.scene.window(w));
    if !focused.is_some_and(|w| !w.kind.is_shell() && w.app_id() == app) {
        return false;
    }
    let seat = SeatId(0);
    send_key(state, seat, code, true) && send_key(state, seat, code, false)
}

/// The Magic Remote's pointer axes reported `(x, y)`: [`pointer_motion`] where the pointer shows; otherwise only where the remote points is kept,
/// so that a turn of the wheel ([`PointerShow::Wheel`]) or the remote's cursor-on report ([`PointerShow::Movement`]) shows the pointer there. A hidden
/// pointer is nobody's: no client hears of the motion, and it is no input to the screen's care or to `getLastInputType`.
pub fn remote_motion(state: &mut State, x: i32, y: i32) -> bool {
    if state.seat.cursor_visible {
        return pointer_motion(state, x, y);
    }
    state.seat.pointer_position = (x.clamp(0, crate::OUTPUT_WIDTH as i32 - 1), y.clamp(0, crate::OUTPUT_HEIGHT as i32 - 1));
    false
}

/// The remote's cursor-on report (key 1198, down and up; every half second while its pointer is active): under [`PointerShow::Movement`] it shows
/// the pointer where the remote points and, while the pointer shows, keeps it up. It is no key and no input of the person's otherwise.
/// Returns whether a client heard the pointer appear.
pub fn cursor_on(state: &mut State) -> bool {
    if state.keys.pointer_show != PointerShow::Movement {
        return false;
    }
    if state.seat.cursor_visible {
        state.seat.cursor_shown_at = Some(std::time::Instant::now());
        return false;
    }
    let (x, y) = state.seat.pointer_position;
    pointer_motion(state, x, y)
}

/// A notch of the Magic Remote's wheel: under [`PointerShow::Wheel`] it shows the pointer where the remote points, and while the pointer
/// shows it keeps it up. A wheel press is OK, a key, and never gets here. Returns whether a client heard the pointer appear.
pub fn wheel_turn(state: &mut State) -> bool {
    // the Rust client's display lab, while it is open, takes the turn as the key `wheel` (its two looks switch with it)
    state.shell_rust_wheel();
    if state.seat.cursor_visible {
        state.seat.cursor_shown_at = Some(std::time::Instant::now());
        state.care_input();
        return false;
    }
    if state.keys.pointer_show == PointerShow::Wheel {
        let (x, y) = state.seat.pointer_position;
        return pointer_motion(state, x, y);
    }
    false
}

/// The pointer moved to `(x, y)` in output pixels: the cursor shows, the surface under it is entered (the
/// previous one left) and gets the motion in its own coordinates. Returns whether a client got it.
pub fn pointer_motion(state: &mut State, x: i32, y: i32) -> bool {
    let (x, y) = (x.clamp(0, crate::OUTPUT_WIDTH as i32 - 1), y.clamp(0, crate::OUTPUT_HEIGHT as i32 - 1));
    state.seat.pointer_position = (x, y);
    state.seat.last_input = "pointer";
    state.care_input();
    set_cursor_visible(state, true);
    state.seat.cursor_shown_at = Some(std::time::Instant::now());
    state.seat.motions += 1;
    state.pointer_sync();
    // the Rust client's layers have the pointer where they are (the sample goes over its key socket) and the app under them hears nothing
    let rust = state.shell_rust_pointer_sample(x, y, true);
    let hit = if rust { None } else { state.scene.surface_at(x, y) };
    let target = hit.map(|(s, _)| s);
    let time = crate::output::monotonic_ms();
    if target != state.seat.pointer_focus {
        let serial = state.next_serial();
        if let Some(old) = state.seat.pointer_focus.take()
            && let Some(res) = state.seat.surfaces.get(&old).cloned()
        {
            for p in of_client(&state.seat.pointers, &res) {
                p.leave(serial, &res);
            }
        }
        if let Some((n, r)) = hit
            && let Some(res) = state.seat.surfaces.get(&n).cloned()
        {
            for p in of_client(&state.seat.pointers, &res) {
                p.enter(serial, &res, (x - r.x) as f64, (y - r.y) as f64);
            }
            state.seat.pointer_focus = Some(n);
            state.seat.pointer_focus_rect = r;
        }
    } else if let Some((_, r)) = hit {
        state.seat.pointer_focus_rect = r;
    }
    let Some(n) = state.seat.pointer_focus else { return rust };
    let Some(res) = state.seat.surfaces.get(&n).cloned() else { return rust };
    let r = state.seat.pointer_focus_rect;
    let mut sent = false;
    for p in of_client(&state.seat.pointers, &res) {
        p.motion(time, (x - r.x) as f64, (y - r.y) as f64);
        sent = true;
    }
    sent
}

/// OK while the cursor shows: `BTN_LEFT` to the surface under the pointer.
pub fn pointer_button(state: &mut State, pressed: bool) -> bool {
    // A release is delivered only for a press that was: the Magic Remote ends its pointer session (SLEEP) with nothing the person pressed,
    // and a release with no press is nothing to anyone, not a click.
    if !pressed && !state.seat.button_down && !state.shell_rust_button_down() {
        return true;
    }
    if state.shell_rust_pointer_button(pressed) {
        state.seat.last_input = "pointer";
        state.seat.cursor_shown_at = Some(std::time::Instant::now());
        return true;
    }
    let Some(n) = state.seat.pointer_focus else { return false };
    let Some(res) = state.seat.surfaces.get(&n).cloned() else { return false };
    let serial = state.next_serial();
    let time = crate::output::monotonic_ms();
    let mut sent = false;
    for p in of_client(&state.seat.pointers, &res) {
        p.button(serial, time, BTN_LEFT, if pressed { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released });
        sent = true;
    }
    state.seat.button_down = pressed && sent;
    state.seat.last_input = "pointer";
    state.seat.cursor_shown_at = Some(std::time::Instant::now());
    sent
}

/// Hides the cursor (a navigation key, the timeout, a client's `set_cursor_visibility(0)`): the surface under it
/// is left, so a hover state does not linger.
pub fn hide_cursor(state: &mut State) {
    set_cursor_visible(state, false);
    let (x, y) = state.seat.pointer_position;
    state.seat.button_down = false;
    state.shell_rust_pointer_sample(x, y, false);
    if let Some(old) = state.seat.pointer_focus.take()
        && let Some(res) = state.seat.surfaces.get(&old).cloned()
    {
        let serial = state.next_serial();
        for p in of_client(&state.seat.pointers, &res) {
            p.leave(serial, &res);
        }
    }
}

/// Every client hears when the cursor appears or goes (`wl_webos_input_manager.cursor_visibility`): LG's web and
/// native apps switch between pointer hover and 5-way focus on it. So does lginput2, over LG's Input Manager queue (`im_queue`): it sends
/// the wheel's press as the mouse button while the pointer shows, and asks for the pointer (1198) while it is hidden.
fn set_cursor_visible(state: &mut State, visible: bool) {
    if state.seat.cursor_visible == visible {
        return;
    }
    state.seat.cursor_visible = visible;
    state.seat.input_managers.retain(|r| r.is_alive());
    for r in &state.seat.input_managers {
        r.cursor_visibility(visible as u32, None);
    }
    state.im_cursor_visible(visible);
    state.pointer_sync();
    state.shell_sync_topics();
}

impl State {
    /// Puts the pointer where it belongs: on the cursor plane when `--pointer plane` is on and the output carries it, else in the frame (`State::tick_at`
    /// draws the sprite there). The output says which: it has no plane, or the plane failed, or a capture or a recording is armed (every frame is composed
    /// and the picture must have the sprite in it, so the plane is hidden for as long). The screen being off hides the pointer. Called wherever the pointer's
    /// place or visibility changes and at each tick (a mode switched); it costs a comparison when nothing changed.
    pub fn pointer_sync(&mut self) {
        let wanted = self.config.pointer == crate::pointer::Mode::Plane;
        let at = (wanted && self.seat.cursor_visible && self.luna_state.power.rendering).then_some(self.seat.pointer_position);
        let carried = self.output.set_pointer(at) && wanted;
        if carried != self.pointer_carried {
            self.pointer_carried = carried;
            if wanted {
                let line = if carried {
                    "pointer: the cursor plane carries it".to_string()
                } else {
                    let why = self.output.pointer_report().and_then(|r| r["lastError"].as_str().filter(|e| !e.is_empty()).map(str::to_string));
                    format!("pointer: drawn into the frame ({})", why.unwrap_or_else(|| "a capture is armed or the plane is not up".to_string()))
                };
                self.journal(line);
            }
        }
    }

    /// The pointer's state for `ctl pointer` and `ctl status`: the switch, whether the plane carries it and the plane's counters.
    pub fn pointer_report(&self) -> serde_json::Value {
        serde_json::json!({
            "mode": self.config.pointer.name(),
            "carried": self.pointer_carried,
            "visible": self.seat.cursor_visible,
            "show": self.keys.pointer_show.name(),
            "at": [self.seat.pointer_position.0, self.seat.pointer_position.1],
            "motions": self.seat.motions,
            "plane": self.output.pointer_report(),
        })
    }
}
