//! The keyboard path: LG's text model (`text_model`, one per text field of an app) joined to an input method (`input_method`, the
//! on-screen keyboard's server: maliit) and back, the way LG's compositor does it (webosose/luna-surfacemanager,
//! `modules/weboscompositor/input`: `WaylandTextModel`, `WaylandInputMethodContext`, `WaylandInputPanel`, `KeyboardView.qml`).
//!
//! An app activates its model on its own surface while that surface has the keyboard focus; an input method context is made for
//! it in the input method's client (`input_method.activate`), the app is told `enter`, and from there each side's requests are the
//! other's events: the field's content type, enter key, surrounding text and `show_input_panel` go to the keyboard, its
//! `commit_string`, `preedit_string`, `delete_surrounding_text` and `keysym` come out at the field. A keyboard that grabs the hardware
//! keyboard (`grab_keyboard`) is given every key the app would have got (`input.rs`). The keyboard's own window (`input_panel`, or
//! a window of type KEYBOARD) is the scene's to place, bottom centre above the app (`Scene::plan`); when it is on the screen the app
//! is told so, with its rectangle, to keep its field clear of it. Leaving is the app's `deactivate`, its surface losing the focus,
//! the Exit key (`close_keyboard`), the keyboard's server going, or the app going.
//!
//! No input method yet: LG asks the system for one (`initctl emit ime-activate`) and finishes the activation when it binds and names
//! its display (`input_method.set_display_id`); so does this.
//!
//! LumaTV's own keyboard (`lumatv keyboard luma`): while the flag file [`OWN_FLAG`] is in the data directory and the shell client says it can
//! show a `keyboard`, a field is served by the shell instead of an input method. There is no context and no grab: the field's content type,
//! enter key and `show_input_panel` go to the shell as the `ime` topic of the UI channel, the remote's navigation keys go to it as `imekey`
//! events while its keyboard is on the screen, and what it types comes back as `POST /v1/ime` and leaves as the field's `commit_string`
//! and `keysym` ([`own_op`]). LG's input method (maliit) serves every field when the flag is absent, the shell is not live or its keyboard
//! does not come up within [`OWN_SHOW_WITHIN`].

use std::time::{Duration, Instant};

use wayland_backend::server::ClientId;
use wayland_server::protocol::wl_keyboard::{self, KeyState, WlKeyboard};
use wayland_server::protocol::wl_surface::WlSurface;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};

use lumatv_wl_protocols::input_method::server::input_method::{self, InputMethod};
use lumatv_wl_protocols::input_method::server::input_method_context::{self, InputMethodContext};
use lumatv_wl_protocols::input_method::server::input_panel::{self, InputPanel};
use lumatv_wl_protocols::input_method::server::input_panel_surface::{self, InputPanelSurface};
use lumatv_wl_protocols::text::server::text_model::{self, TextModel};
use lumatv_wl_protocols::text::server::text_model_factory::{self, TextModelFactory};

use super::compositor::surface_id;
use super::seat::{live, send_keymap};
use crate::scene::keyboard_geometry;
use crate::surface::{Role, SurfaceId};
use crate::window::{WindowId, WindowType};
use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect, State, plain_global};

/// The display an input method serves: the set has one.
const DISPLAY: u32 = 0;
/// At most one request for an input method in this long: the system takes a moment to start one.
const ASK_EVERY: Duration = Duration::from_secs(3);
/// The flag `lumatv keyboard luma` leaves in the data directory: LumaTV's own keyboard serves the fields while the shell can show it.
pub const OWN_FLAG: &str = ".keyboard-luma";
/// The shell has this long to put its keyboard on the screen after a field asked for it; past it LG's keyboard serves the field.
const OWN_SHOW_WITHIN: Duration = Duration::from_secs(2);
/// After a keyboard that did not come up, LG's keyboard serves every field for this long.
const OWN_BROKEN_FOR: Duration = Duration::from_secs(60);

pub struct ModelData(u32);
pub struct MethodData(u32);
pub struct ContextData {
    model: u32,
}
pub struct GrabData;
pub struct PanelSurfaceData {
    surface: SurfaceId,
    window: WindowId,
}

struct Model {
    id: u32,
    res: TextModel,
    /// Set by `activate`; the model holds the activation from then until it leaves, whether or not its context exists yet.
    active: bool,
    surface: Option<WlSurface>,
    surface_id: Option<SurfaceId>,
    /// The input method's context for this model and the input method it was made through, once there is one.
    ctx: Option<InputMethodContext>,
    method: Option<(u32, InputMethod)>,
    /// `show_input_panel` came before the context existed: sent as soon as it does (LG drops it).
    show_wanted: bool,
    /// Where the app wants the keyboard (`set_input_panel_rect`).
    preferred: Option<Rect>,
    /// What the field last said about itself, kept whether or not the model is active (an app may describe a field before it activates
    /// it). Of the surrounding text only its length and the cursor's byte offset are kept: a password never rests here.
    hint: u32,
    purpose: u32,
    enter: u32,
    max: u32,
    surround: Option<(u32, u32)>,
    /// Served by the shell's keyboard ([`begin_own`]): no input method context exists for it.
    own: bool,
    /// The field wants the shell's keyboard on the screen.
    own_show: bool,
    /// The serial of the activation, which the shell's keyboard's events to the field carry (as the one maliit is activated with).
    serial: u32,
}

struct Method {
    id: u32,
    res: InputMethod,
    display: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub activations: u64,
    pub declined: u64,
    pub commits: u64,
    pub preedits: u64,
    pub deletes: u64,
    pub keysyms: u64,
    pub grabbed_keys: u64,
}

/// What the shell's keyboard did, for `ctl ime` and the tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct OwnCounts {
    pub activations: u64,
    pub shown: u64,
    pub texts: u64,
    pub keys: u64,
    pub clears: u64,
    pub hides: u64,
    pub nav: u64,
    pub failed: u64,
}

/// The shell's keyboard as the compositor knows it.
#[derive(Default)]
pub struct Own {
    /// Where the keyboard is on the screen, from the moment it starts to slide in until the field leaves, hides it or the shell says so.
    pub up: Option<Rect>,
    /// `show_input_panel` went to the shell at this time and its keyboard has not said it is up.
    pending: Option<Instant>,
    /// LG's keyboard serves every field until then: the shell's did not come up in time.
    broken_until: Option<Instant>,
    pub counts: OwnCounts,
}

pub struct ImeState {
    models: Vec<Model>,
    methods: Vec<Method>,
    next: u32,
    active: Option<u32>,
    grab: Option<WlKeyboard>,
    /// The keys the grab was given a press of and not yet a release.
    pub grab_down: Vec<u32>,
    /// The keyboard window's rectangle while one is on the screen.
    pub panel: Option<Rect>,
    last_ask: Option<Instant>,
    /// How many times an input method was asked for.
    pub asked: u32,
    /// Asks the system for an input method; the host tests replace it.
    pub ask_for_input_method: Box<dyn FnMut()>,
    pub counts: Counts,
    /// The shell's own keyboard.
    pub own: Own,
}

impl Default for ImeState {
    fn default() -> Self {
        ImeState {
            models: Vec::new(),
            methods: Vec::new(),
            next: 0,
            active: None,
            grab: None,
            grab_down: Vec::new(),
            panel: None,
            last_ask: None,
            asked: 0,
            ask_for_input_method: Box::new(ask_system),
            counts: Counts::default(),
            own: Own::default(),
        }
    }
}

/// What LG's compositor does when a text model activates and no input method is there: `initctl emit ime-activate`, which starts
/// the keyboard's server. Not waited for (the loop must not stand still for it); the child is reaped on a thread.
fn ask_system() {
    let initctl = "/sbin/initctl";
    if !std::path::Path::new(initctl).exists() {
        log::warn!("ime: {initctl} is not there; no input method can be asked for");
        return;
    }
    match std::process::Command::new(initctl)
        .args(["emit", "ime-activate"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::warn!("ime: {initctl}: {e}"),
    }
}

impl ImeState {
    fn model(&self, id: u32) -> Option<&Model> {
        self.models.iter().find(|m| m.id == id)
    }

    fn model_mut(&mut self, id: u32) -> Option<&mut Model> {
        self.models.iter_mut().find(|m| m.id == id)
    }

    /// The input method that serves the display, when there is a live one that has said which.
    fn ready_method(&self) -> Option<(u32, InputMethod)> {
        self.methods.iter().find(|m| m.display == Some(DISPLAY) && live(&m.res)).map(|m| (m.id, m.res.clone()))
    }

    /// The model that holds the activation, with its surface's id.
    pub fn active_model(&self) -> Option<(u32, Option<SurfaceId>)> {
        self.active.and_then(|id| self.model(id)).map(|m| (m.id, m.surface_id))
    }

    /// An input method holds the keyboard grab and its keyboard is on the screen, or the shell's keyboard is: the keys go to it. A keyboard that
    /// is off the screen (hidden on Back) is given none, so that the remote steers the app again at once; its grab is back in force when the
    /// keyboard's window is.
    pub fn grabbing(&self) -> bool {
        self.grabbing_external() || self.own_taking()
    }

    fn grabbing_external(&self) -> bool {
        self.panel.is_some() && self.grab.as_ref().is_some_and(live)
    }

    /// The shell's keyboard is on the screen for the active field: it is given the remote's navigation keys.
    pub fn own_taking(&self) -> bool {
        self.own.up.is_some() && self.active.and_then(|id| self.model(id)).is_some_and(|m| m.own && m.active)
    }

    /// Whether `code` goes to the keyboard that is up: every key to an input method's grab; to the shell's the navigation keys, OK, Back and
    /// the digits (a hardware keyboard's letters are the app's own).
    pub fn grabs(&self, code: u32) -> bool {
        self.grabbing_external() || (self.own_taking() && own_key_name(code).is_some())
    }

    pub fn methods_bound(&self) -> usize {
        self.methods.iter().filter(|m| m.res.is_alive()).count()
    }

    /// Whether the active model has its input method context.
    pub fn context_made(&self) -> bool {
        self.active.and_then(|id| self.model(id)).is_some_and(|m| m.ctx.as_ref().is_some_and(live))
    }
}

plain_global!(TextModelFactory);
plain_global!(InputPanel);

impl GlobalDispatch<InputMethod, ()> for State {
    fn bind(state: &mut Self, _handle: &DisplayHandle, client: &Client, resource: New<InputMethod>, _global: &(), data_init: &mut DataInit<'_, Self>) {
        state.note_bind(client, "input_method");
        state.ime.next += 1;
        let id = state.ime.next;
        let res = data_init.init(resource, MethodData(id));
        let version = res.version();
        // Version 2 names its display with a request of its own; a client of version 1 has no way to and serves the first.
        let display = (version < 2).then_some(DISPLAY);
        state.ime.methods.push(Method { id, res, display });
        let who = state.client_info(client);
        state.journal(format!("ime: an input method (version {version}) bound by {who}"));
        if display.is_some() {
            method_ready(state);
        }
    }
}

impl Dispatch<InputMethod, MethodData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &InputMethod,
        request: input_method::Request,
        data: &MethodData,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        if let input_method::Request::SetDisplayId { id } = request {
            state.note_request("input_method", 0, &format!("set_display_id({id})"));
            if let Some(m) = state.ime.methods.iter_mut().find(|m| m.id == data.0) {
                m.display = Some(id);
            }
            state.journal(format!("ime: the input method serves display {id}"));
            method_ready(state);
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, _resource: &InputMethod, data: &MethodData) {
        method_gone(state, data.0);
    }
}

impl Dispatch<TextModelFactory, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        _resource: &TextModelFactory,
        request: text_model_factory::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        if let text_model_factory::Request::CreateTextModel { id } = request {
            state.ime.next += 1;
            let mid = state.ime.next;
            let res = data_init.init(id, ModelData(mid));
            state.ime.models.push(Model {
                id: mid,
                res,
                active: false,
                surface: None,
                surface_id: None,
                ctx: None,
                method: None,
                show_wanted: false,
                preferred: None,
                hint: 0x7,
                purpose: 0,
                enter: 0,
                max: 0,
                surround: None,
                own: false,
                own_show: false,
                serial: 0,
            });
            state.note_request("text_model_factory", 0, "create_text_model");
        }
    }
}

impl Dispatch<TextModel, ModelData> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &TextModel,
        request: text_model::Request,
        data: &ModelData,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        let mid = data.0;
        state.note_request("text_model", resource.id().protocol_id(), &quiet(&request));
        // What the field says goes to the keyboard's context while the model is active (LG forwards it the same way and drops it
        // while there is no context). The shell's keyboard has no context: it is told through the model's own record (`own_publish`).
        let ctx = state.ime.model(mid).filter(|m| m.active).and_then(|m| m.ctx.clone()).filter(live);
        let own = state.ime.model(mid).is_some_and(|m| m.active && m.own);
        match request {
            text_model::Request::Activate { surface, .. } => activate(state, mid, &surface),
            text_model::Request::Deactivate { .. } => deactivate(state, mid, "the app deactivated it"),
            text_model::Request::SetSurroundingText { text, cursor, anchor } => {
                if let Some(m) = state.ime.model_mut(mid) {
                    m.surround = Some((text.len() as u32, cursor));
                }
                if own {
                    own_publish(state);
                }
                if let Some(c) = ctx {
                    c.surrounding_text(text, cursor, anchor);
                }
            }
            text_model::Request::Reset { serial } => {
                if let Some(c) = ctx {
                    c.reset(serial);
                }
            }
            text_model::Request::SetContentType { hint, purpose } => {
                if let Some(m) = state.ime.model_mut(mid) {
                    m.hint = hint;
                    m.purpose = purpose;
                }
                if own {
                    own_publish(state);
                }
                if let Some(c) = ctx {
                    c.content_type(hint, purpose);
                }
            }
            text_model::Request::SetEnterKeyType { enter_key_type } => {
                if let Some(m) = state.ime.model_mut(mid) {
                    m.enter = enter_key_type;
                }
                if own {
                    own_publish(state);
                }
                if let Some(c) = ctx {
                    c.enter_key_type(enter_key_type);
                }
            }
            text_model::Request::InvokeAction { button, index } => {
                if let Some(c) = ctx {
                    c.invoke_action(button, index);
                }
            }
            text_model::Request::Commit => {
                if let Some(c) = ctx {
                    c.commit();
                }
            }
            text_model::Request::SetMaxTextLength { length } => {
                if let Some(m) = state.ime.model_mut(mid) {
                    m.max = length;
                }
                if own {
                    own_publish(state);
                }
                if let Some(c) = ctx {
                    c.max_text_length(length);
                }
            }
            text_model::Request::SetPlatformData { text } => {
                if let Some(c) = ctx {
                    c.platform_data(text);
                }
            }
            text_model::Request::ShowInputPanel => {
                if own {
                    own_show(state, mid);
                    return;
                }
                let method = state.ime.model(mid).filter(|m| m.active).and_then(|m| m.method.as_ref().map(|(_, r)| r.clone()));
                match (method.filter(live), ctx) {
                    (Some(method), Some(c)) => method.show_input_panel(&c),
                    _ => {
                        if let Some(m) = state.ime.model_mut(mid).filter(|m| m.active) {
                            m.show_wanted = true;
                        }
                    }
                }
            }
            text_model::Request::HideInputPanel => {
                if own {
                    own_app_hides(state, mid);
                    return;
                }
                let method = state.ime.model(mid).filter(|m| m.active).and_then(|m| m.method.as_ref().map(|(_, r)| r.clone()));
                if let Some(m) = state.ime.model_mut(mid) {
                    m.show_wanted = false;
                }
                if let (Some(method), Some(c)) = (method.filter(live), ctx) {
                    method.hide_input_panel(&c);
                }
            }
            text_model::Request::SetInputPanelRect { x, y, width, height } => {
                let rect = Rect::new(x, y, width as i32, height as i32);
                if let Some(m) = state.ime.model_mut(mid) {
                    m.preferred = Some(rect);
                }
                state.journal(format!("ime: text model {mid} asked for the keyboard at ({x}, {y}) {width}x{height}"));
                if state.ime.active == Some(mid) {
                    state.scene.keyboard_pref = Some(rect);
                    refresh_panel(state);
                }
            }
            text_model::Request::ResetInputPanelRect => {
                if let Some(m) = state.ime.model_mut(mid) {
                    m.preferred = None;
                }
                if state.ime.active == Some(mid) {
                    state.scene.keyboard_pref = None;
                    refresh_panel(state);
                }
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, _resource: &TextModel, data: &ModelData) {
        model_gone(state, data.0);
    }
}

/// A request as the protocol log shows it, with the words of the field left out: a password or a PIN is never logged.
fn quiet(request: &text_model::Request) -> String {
    match request {
        text_model::Request::SetSurroundingText { text, cursor, anchor } => {
            format!("SetSurroundingText {{ {} bytes, cursor: {cursor}, anchor: {anchor} }}", text.len())
        }
        text_model::Request::SetPlatformData { text } => format!("SetPlatformData {{ {} bytes }}", text.len()),
        other => format!("{other:?}"),
    }
}

/// The same for what an input method sends: the text it types is not logged.
fn quiet_context(request: &input_method_context::Request) -> String {
    match request {
        input_method_context::Request::CommitString { serial, text } => format!("CommitString {{ serial: {serial}, {} bytes }}", text.len()),
        input_method_context::Request::PreeditString { serial, text, commit } => {
            format!("PreeditString {{ serial: {serial}, {} bytes, commit {} bytes }}", text.len(), commit.len())
        }
        other => format!("{other:?}"),
    }
}

impl Dispatch<InputMethodContext, ContextData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &InputMethodContext,
        request: input_method_context::Request,
        data: &ContextData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.note_request("input_method_context", resource.id().protocol_id(), &quiet_context(&request));
        // The keyboard's answers are the field's events. A context that is not the active model's any more (the keyboard's server is
        // slow to destroy it) is listened to for nothing.
        let model = state.ime.model(data.model).filter(|m| m.active && m.ctx.as_ref().is_some_and(|c| c.id() == resource.id())).map(|m| m.res.clone());
        let Some(model) = model.filter(live) else {
            if let input_method_context::Request::GrabKeyboard { keyboard } = request {
                // The new object has to be made whatever happens to the grab; it is never fed.
                data_init.init(keyboard, GrabData);
            }
            return;
        };
        match request {
            input_method_context::Request::CommitString { serial, text } => {
                state.ime.counts.commits += 1;
                model.commit_string(serial, text);
            }
            input_method_context::Request::PreeditString { serial, text, commit } => {
                state.ime.counts.preedits += 1;
                model.preedit_string(serial, text, commit);
            }
            input_method_context::Request::PreeditStyling { serial, index, length, style } => model.preedit_styling(serial, index, length, style),
            input_method_context::Request::PreeditCursor { serial, index } => model.preedit_cursor(serial, index),
            input_method_context::Request::DeleteSurroundingText { serial, index, length } => {
                state.ime.counts.deletes += 1;
                model.delete_surrounding_text(serial, index, length);
            }
            input_method_context::Request::CursorPosition { serial, index, anchor } => model.cursor_position(serial, index, anchor),
            input_method_context::Request::ModifiersMap { map } => model.modifiers_map(map),
            input_method_context::Request::Keysym { serial, time, sym, state: key_state, modifiers } => {
                state.ime.counts.keysyms += 1;
                model.keysym(serial, time, sym, key_state, modifiers);
            }
            input_method_context::Request::GrabKeyboard { keyboard } => {
                let kb = data_init.init(keyboard, GrabData);
                send_keymap(&kb);
                let serial = state.next_serial();
                kb.modifiers(serial, 0, 0, 0, 0);
                state.ime.grab = Some(kb);
                state.ime.grab_down.clear();
                state.journal("ime: the input method grabbed the keyboard".into());
            }
            // The keys and modifiers an input method gives back are LG's no-ops too.
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &InputMethodContext, data: &ContextData) {
        // The keyboard's server dropped the context while the field still held the activation: the field is told it left.
        let id = resource.id();
        if state.ime.model(data.model).is_some_and(|m| m.active && m.ctx.as_ref().is_some_and(|c| c.id() == id)) {
            if let Some(m) = state.ime.model_mut(data.model) {
                m.ctx = None;
                m.method = None;
            }
            deactivate(state, data.model, "the input method destroyed its context");
        }
    }
}

impl Dispatch<WlKeyboard, GrabData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &WlKeyboard,
        request: wl_keyboard::Request,
        _data: &GrabData,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        if let wl_keyboard::Request::Release = request
            && state.ime.grab.as_ref().is_some_and(|g| g.id() == resource.id())
        {
            state.ime.grab = None;
            state.ime.grab_down.clear();
        }
    }
}

impl Dispatch<InputPanel, ()> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &InputPanel,
        request: input_panel::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.touch_client(client);
        if let input_panel::Request::GetInputPanelSurface { id, surface } = request {
            let Some(sid) = surface_id(&surface) else {
                resource.post_error(0u32, "unknown surface");
                return;
            };
            let Some(s) = state.surfaces.get_mut(sid) else { return };
            match s.role {
                Role::None => s.role = Role::InputPanel,
                Role::InputPanel | Role::Toplevel => {}
                _ => {
                    resource.post_error(0u32, "surface already has another role");
                    return;
                }
            }
            // LG's compositor sets the type of an input panel's surface to KEYBOARD, which is how its QML view takes it, and keeps the
            // keyboard focus where it was.
            let wid = state.scene.create_window(sid);
            if let Some(w) = state.scene.window_mut(wid) {
                w.set_property("_WEBOS_WINDOW_TYPE", "_WEBOS_WINDOW_TYPE_KEYBOARD");
                w.props.insert("needFocus".into(), "false".into());
            }
            data_init.init(id, PanelSurfaceData { surface: sid, window: wid });
            state.note_request("input_panel", 0, "get_input_panel_surface");
            let who = state.client_info(client);
            state.journal(format!("ime: window {} is the keyboard's (surface {}) by {who}", wid.0, sid.0));
        }
    }
}

impl Dispatch<InputPanelSurface, PanelSurfaceData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &InputPanelSurface,
        request: input_panel_surface::Request,
        _data: &PanelSurfaceData,
        _dh: &DisplayHandle,
        _di: &mut DataInit<'_, Self>,
    ) {
        // `set_toplevel`'s position is not implemented in LG's compositor either: the keyboard is placed by its view.
        state.note_request("input_panel_surface", resource.id().protocol_id(), &format!("{request:?}"));
    }

    fn destroyed(state: &mut Self, _client: ClientId, _resource: &InputPanelSurface, data: &PanelSurfaceData) {
        if state.scene.window(data.window).is_some_and(|w| w.kind == WindowType::Keyboard && !w.has_wl_shell && !w.has_webos_shell) {
            state.scene.remove_window(data.window);
            if let Some(s) = state.surfaces.get_mut(data.surface)
                && s.role == Role::InputPanel
            {
                s.role = Role::None;
            }
            refresh_panel(state);
            super::seat::refocus(state);
        }
    }
}

/// `text_model.activate`: only for the surface that has the keyboard focus (LG declines any other and tells the model it left).
fn activate(state: &mut State, mid: u32, surface: &WlSurface) {
    let Some(sid) = surface_id(surface) else { return };
    let Some(m) = state.ime.model(mid) else { return };
    if m.active && m.surface_id == Some(sid) {
        return;
    }
    let on_another = m.active;
    if state.seat.focus != Some(sid) {
        state.ime.counts.declined += 1;
        let focus = state.seat.focus.map(|s| s.0);
        state.journal(format!("ime: text model {mid} not activated: surface {} does not have the keyboard focus ({focus:?} does)", sid.0));
        if let Some(m) = state.ime.model(mid)
            && live(&m.res)
            && live(surface)
        {
            m.res.leave();
        }
        return;
    }
    if on_another {
        deactivate_with(state, mid, "activated on another surface", false);
    }
    if let Some(prev) = state.ime.active.filter(|p| *p != mid) {
        deactivate_with(state, prev, "another text field activated", false);
    }
    if let Some(m) = state.ime.model_mut(mid) {
        m.active = true;
        m.surface = Some(surface.clone());
        m.surface_id = Some(sid);
    }
    state.ime.active = Some(mid);
    state.ime.counts.activations += 1;
    let app = state.scene.window_of(sid).and_then(|w| state.scene.window(w)).map(|w| w.app_id().to_string()).unwrap_or_default();
    state.journal(format!("ime: text model {mid} activated on surface {} ({app:?})", sid.0));
    begin(state, mid);
    // The shell's keyboard hears of a field it serves from `begin_own`; a field that goes to LG's is news to it too when a field it served left.
    own_publish(state);
}

/// Makes the model's context in the input method's client and tells the app it entered, or asks for an input method first.
fn begin(state: &mut State, mid: u32) {
    if own_ready(state) {
        begin_own(state, mid);
        return;
    }
    let Some((method_id, method)) = state.ime.ready_method() else {
        request_input_method(state);
        return;
    };
    let Some(client) = method.client() else { return };
    let ctx = match client.create_resource::<InputMethodContext, ContextData, State>(&state.dh, 1, ContextData { model: mid }) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("ime: the input method's context: {e}");
            return;
        }
    };
    let serial = state.next_serial();
    method.activate(&ctx, serial);
    let panel = state.ime.panel;
    let Some(m) = state.ime.model_mut(mid) else { return };
    m.ctx = Some(ctx.clone());
    m.method = Some((method_id, method.clone()));
    let (res, surface, show, preferred) = (m.res.clone(), m.surface.clone(), std::mem::take(&mut m.show_wanted), m.preferred);
    if show {
        method.show_input_panel(&ctx);
    }
    state.scene.keyboard_pref = preferred;
    if let Some(surface) = surface
        && live(&res)
        && live(&surface)
    {
        res.enter(&surface);
    }
    // A keyboard that is already up is news to the field.
    if let Some(r) = panel
        && live(&res)
    {
        res.input_panel_state(1);
        res.input_panel_rect(r.x, r.y, r.w as u32, r.h as u32);
    }
    state.journal(format!("ime: text model {mid} entered; the input method's context is made"));
}

/// A model stops being active: the keyboard is told, the field is told it left and the keyboard is no longer up for it.
pub fn deactivate(state: &mut State, mid: u32, why: &str) {
    deactivate_with(state, mid, why, true);
}

/// [`deactivate`]; `publish` says whether the shell hears of it at once (an activation that follows tells it of the field it is for).
fn deactivate_with(state: &mut State, mid: u32, why: &str, publish: bool) {
    let Some(m) = state.ime.model_mut(mid) else { return };
    if !m.active {
        return;
    }
    let was_own = m.own;
    m.active = false;
    m.show_wanted = false;
    m.own = false;
    m.own_show = false;
    m.surface = None;
    m.surface_id = None;
    let (res, ctx, method) = (m.res.clone(), m.ctx.take(), m.method.take());
    if state.ime.active == Some(mid) {
        state.ime.active = None;
        state.scene.keyboard_pref = None;
    }
    state.ime.grab = None;
    state.ime.grab_down.clear();
    if let (Some(ctx), Some((_, method))) = (&ctx, &method)
        && live(method)
        && live(ctx)
    {
        method.deactivate(ctx);
    }
    if live(&res) {
        res.input_panel_state(0);
        res.leave();
    }
    if was_own {
        own_off(state);
        if publish {
            own_publish(state);
        }
    }
    state.journal(format!("ime: text model {mid} left ({why})"));
}

fn model_gone(state: &mut State, mid: u32) {
    deactivate(state, mid, "the app went");
    state.ime.models.retain(|m| m.id != mid);
}

/// An input method came up (or named its display): an activation that was waiting for it goes on.
fn method_ready(state: &mut State) {
    if let Some(mid) = state.ime.active
        && state.ime.model(mid).is_some_and(|m| m.active && m.ctx.is_none())
        && state.ime.ready_method().is_some()
    {
        begin(state, mid);
    }
}

fn method_gone(state: &mut State, id: u32) {
    state.ime.methods.retain(|m| m.id != id);
    let holders: Vec<u32> = state.ime.models.iter().filter(|m| m.method.as_ref().is_some_and(|(mid, _)| *mid == id)).map(|m| m.id).collect();
    for mid in holders {
        if let Some(m) = state.ime.model_mut(mid) {
            m.ctx = None;
            m.method = None;
        }
        deactivate(state, mid, "the input method went");
    }
}

/// Keyboard focus moved: a text field whose surface lost it is deactivated (LG does the same on `activeFocusChanged`).
pub fn focus_changed(state: &mut State) {
    let Some(mid) = state.ime.active else { return };
    let focus = state.seat.focus;
    if state.ime.model(mid).is_some_and(|m| m.surface_id != focus) {
        deactivate(state, mid, "its surface lost the keyboard focus");
    }
}

/// A surface went: a field on it leaves, and a keyboard window on it is gone from the screen.
pub fn surface_gone(state: &mut State, sid: SurfaceId) {
    let holders: Vec<u32> = state.ime.models.iter().filter(|m| m.active && m.surface_id == Some(sid)).map(|m| m.id).collect();
    for mid in holders {
        deactivate(state, mid, "its surface was destroyed");
    }
    refresh_panel(state);
}

/// The keyboard's rectangle on the screen as LG's `KeyboardView` has it: the full width, anchored at the bottom, as high as the keyboard
/// (`keyboard_geometry`: the window, or the part of it the window marks as its input region); the rectangle the app asked for
/// (`set_input_panel_rect`) when it asked for one.
pub fn panel_rect(size: (i32, i32), input: Option<&[Rect]>, preferred: Option<Rect>) -> Rect {
    let g = keyboard_geometry(size, input);
    preferred.unwrap_or(Rect::new(0, OUTPUT_HEIGHT as i32 - g.h, OUTPUT_WIDTH as i32, g.h))
}

/// Compares the keyboard on the screen with what the active field was last told, and tells it what changed: it is shown or gone
/// (`input_panel_state`) and where (`input_panel_rect`).
pub fn refresh_panel(state: &mut State) {
    let shown = state
        .scene
        .windows()
        .filter(|w| w.kind == WindowType::Keyboard && w.map_serial > 0)
        .filter_map(|w| state.surfaces.get(w.surface))
        .find(|s| s.is_mapped() && s.size().1 > 0);
    let now = shown.map(|s| panel_rect(s.size(), s.current.input.as_deref(), state.scene.keyboard_pref));
    if now == state.ime.panel {
        return;
    }
    let line = match (shown, now) {
        (Some(s), Some(r)) => {
            // The window's own size and input region are what a keyboard that is not where LG puts it is read from.
            let (size, input) = (s.size(), s.current.input.as_deref());
            let g = keyboard_geometry(size, input);
            format!(
                "ime: the keyboard is on the screen: {}x{} at ({}, {}); its window {}x{}, input region {:?} (keyboard {}x{} at ({}, {}) in it), the field asked for {:?}",
                r.w, r.h, r.x, r.y, size.0, size.1, input, g.w, g.h, g.x, g.y, state.scene.keyboard_pref
            )
        }
        _ => "ime: the keyboard is off the screen".into(),
    };
    let was_up = state.ime.panel.is_some();
    state.ime.panel = now;
    state.journal(line);
    let Some(res) = state.ime.active.and_then(|id| state.ime.model(id)).map(|m| m.res.clone()).filter(live) else { return };
    match now {
        Some(r) => {
            if !was_up {
                res.input_panel_state(1);
            }
            res.input_panel_rect(r.x, r.y, r.w as u32, r.h as u32);
        }
        None => res.input_panel_state(0),
    }
}

fn request_input_method(state: &mut State) {
    let now = Instant::now();
    if state.ime.last_ask.is_some_and(|t| now.saturating_duration_since(t) < ASK_EVERY) {
        return;
    }
    state.ime.last_ask = Some(now);
    state.ime.asked += 1;
    state.journal("ime: no input method is bound; asking the system for one (initctl emit ime-activate)".into());
    (state.ime.ask_for_input_method)();
}

// ---- the shell's own keyboard

/// Whether a field is served by the shell's keyboard now: the owner's flag, the shell able to show one, and no failure to bring one up
/// in the last minute.
fn own_ready(state: &State) -> bool {
    state.shell_can("keyboard") && state.ime.own.broken_until.is_none_or(|t| Instant::now() >= t) && state.config.data_dir.join(OWN_FLAG).exists()
}

/// The model the shell's keyboard serves, if any.
fn own_active(state: &State) -> Option<u32> {
    let id = state.ime.active?;
    state.ime.model(id).filter(|m| m.active && m.own).map(|m| m.id)
}

/// The name the shell's keyboard knows a key of the remote by, for the keys it takes while it is up: the arrows, OK, Back and the digits.
pub fn own_key_name(code: u32) -> Option<&'static str> {
    use crate::input::code::*;
    const DIGITS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
    Some(match code {
        KEY_UP => "up",
        KEY_DOWN => "down",
        KEY_LEFT => "left",
        KEY_RIGHT => "right",
        KEY_ENTER | KEY_OK => "ok",
        KEY_BACK | KEY_PREVIOUS => "back",
        KEY_1..=KEY_9 => DIGITS[(code - KEY_1) as usize],
        KEY_0 => "0",
        _ => return None,
    })
}

/// The keysym a named key of the shell's keyboard is typed as (XKB's).
fn keysym_of(name: &str) -> Option<u32> {
    Some(match name {
        "backspace" => 0xff08,
        "tab" => 0xff09,
        "enter" => 0xff0d,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        _ => return None,
    })
}

/// A field the shell's keyboard serves: no context, the field is told it entered, and the shell is told what the field is.
fn begin_own(state: &mut State, mid: u32) {
    let serial = state.next_serial();
    let Some(m) = state.ime.model_mut(mid) else { return };
    m.own = true;
    m.serial = serial;
    m.ctx = None;
    m.method = None;
    m.own_show = std::mem::take(&mut m.show_wanted);
    let (res, surface, show) = (m.res.clone(), m.surface.clone(), m.own_show);
    state.ime.own.counts.activations += 1;
    if let Some(surface) = surface
        && live(&res)
        && live(&surface)
    {
        res.enter(&surface);
    }
    if show {
        state.ime.own.pending = Some(Instant::now());
    }
    state.journal(format!("ime: text model {mid} entered; the shell's keyboard serves it"));
    own_publish(state);
}

/// Tells the shell what the active field is and whether it wants the keyboard: the `ime` topic, pushed when it changed. A field the shell
/// does not serve, or none, is `{"active": false}`. The text of the field is never in it, only whether there is any.
pub fn own_publish(state: &mut State) {
    let doc = match state.ime.active.and_then(|id| state.ime.model(id)).filter(|m| m.active && m.own) {
        Some(m) => {
            let app =
                m.surface_id.and_then(|s| state.scene.window_of(s)).and_then(|w| state.scene.window(w)).map(|w| w.app_id().to_string()).unwrap_or_default();
            serde_json::json!({
                "active": true, "show": m.own_show, "model": m.id, "app": app,
                "hint": m.hint, "purpose": m.purpose, "enter": m.enter, "max": m.max,
                "text": m.surround.is_some_and(|(len, _)| len > 0),
            })
        }
        None => serde_json::json!({"active": false, "show": false}),
    };
    state.shell.push_if_changed("ime", doc);
}

/// The field asks for its keyboard: the shell is told, and has [`OWN_SHOW_WITHIN`] to say it is up.
fn own_show(state: &mut State, mid: u32) {
    if let Some(m) = state.ime.model_mut(mid) {
        m.own_show = true;
    }
    if state.ime.own.up.is_none() && state.ime.own.pending.is_none() {
        state.ime.own.pending = Some(Instant::now());
        state.shell_mark("Q:keyboard-show");
    }
    own_publish(state);
}

/// The field hides its keyboard (it stays active: the app asks again to have it back).
fn own_app_hides(state: &mut State, mid: u32) {
    if let Some(m) = state.ime.model_mut(mid) {
        m.own_show = false;
    }
    own_publish(state);
    own_down(state, mid);
}

/// The shell's keyboard is not on the screen (any longer, or it never got on): the pointer and the keys are the app's, the topics say so.
/// Returns whether it was up.
fn own_off(state: &mut State) -> bool {
    state.ime.own.pending = None;
    let was = state.ime.own.up.take().is_some();
    state.scene.shell_pointer = None;
    if was {
        state.journal("ime: the shell's keyboard is off the screen".into());
        state.shell_sync_topics();
    }
    was
}

/// [`own_off`] for a field that stays active: it is told its keyboard is gone.
fn own_down(state: &mut State, mid: u32) {
    if own_off(state)
        && let Some(res) = state.ime.model(mid).map(|m| m.res.clone()).filter(live)
    {
        res.input_panel_state(0);
    }
}

/// The shell says its keyboard is on the screen (it starts to slide in) at `rect`: the field is told where, and the remote's keys are the
/// keyboard's from here.
fn own_shown(state: &mut State, mid: u32, rect: Rect) {
    let Some(res) = state.ime.model(mid).filter(|m| m.own_show).map(|m| m.res.clone()) else { return };
    state.ime.own.pending = None;
    if state.ime.own.up == Some(rect) {
        return;
    }
    let first = state.ime.own.up.is_none();
    state.ime.own.up = Some(rect);
    state.scene.shell_pointer = Some(rect);
    if live(&res) {
        if first {
            res.input_panel_state(1);
        }
        res.input_panel_rect(rect.x, rect.y, rect.w as u32, rect.h as u32);
    }
    if first {
        state.ime.own.counts.shown += 1;
        state.shell_mark("Q:keyboard-shown");
        state.journal(format!("ime: the shell's keyboard is on the screen: {}x{} at ({}, {})", rect.w, rect.h, rect.x, rect.y));
    }
    state.shell_sync_topics();
}

/// The shell's keyboard hides itself (Back, or Done on a field that is finished): the field stays active.
fn own_hide(state: &mut State, mid: u32) {
    if let Some(m) = state.ime.model_mut(mid) {
        m.own_show = false;
    }
    state.ime.own.counts.hides += 1;
    own_publish(state);
    own_down(state, mid);
}

/// Text the shell's keyboard typed: the field's `commit_string`.
fn own_text(state: &mut State, mid: u32, text: &str) {
    let Some((res, serial)) = state.ime.model(mid).map(|m| (m.res.clone(), m.serial)) else { return };
    if live(&res) {
        res.commit_string(serial, text.to_string());
        state.ime.own.counts.texts += 1;
    }
}

/// A key the shell's keyboard pressed that is not text (Backspace, Enter, the cursor keys): the field's `keysym`, press and release.
fn own_key(state: &mut State, mid: u32, sym: u32) {
    let Some((res, serial)) = state.ime.model(mid).map(|m| (m.res.clone(), m.serial)) else { return };
    if live(&res) {
        let time = crate::output::monotonic_ms();
        res.keysym(serial, time, sym, 1, 0);
        res.keysym(serial, time, sym, 0, 0);
        state.ime.own.counts.keys += 1;
    }
}

/// Clear all: the field's text, when the field has said how long it is (`set_surrounding_text`), as one `delete_surrounding_text` from the start.
fn own_clear(state: &mut State, mid: u32) {
    let Some((res, serial, surround)) = state.ime.model(mid).map(|m| (m.res.clone(), m.serial, m.surround)) else { return };
    let Some((len, cursor)) = surround.filter(|(len, _)| *len > 0) else { return };
    if live(&res) {
        res.delete_surrounding_text(serial, -(cursor as i32), len);
        state.ime.own.counts.clears += 1;
        if let Some(m) = state.ime.model_mut(mid) {
            m.surround = Some((0, 0));
        }
        own_publish(state);
    }
}

/// One request of the shell's keyboard (`POST /v1/ime`), for the field it serves: `text`, `key`, `clear`, `shown` (with its rectangle) and
/// `hide`. A request that names another field than the active one (`model`) is stale and dropped. Nothing of the text is journalled.
pub fn own_op(state: &mut State, body: &serde_json::Value) {
    let Some(mid) = own_active(state) else { return };
    if body.get("model").and_then(serde_json::Value::as_u64).is_some_and(|m| m != u64::from(mid)) {
        return;
    }
    let num = |k: &str| body.get(k).and_then(serde_json::Value::as_i64).unwrap_or(0) as i32;
    match body.get("op").and_then(serde_json::Value::as_str).unwrap_or("") {
        "text" => {
            if let Some(t) = body.get("text").and_then(serde_json::Value::as_str).filter(|t| !t.is_empty() && t.len() <= 256) {
                own_text(state, mid, t);
            }
        }
        "key" => {
            if let Some(sym) = body.get("key").and_then(serde_json::Value::as_str).and_then(keysym_of) {
                own_key(state, mid, sym);
            }
        }
        "clear" => own_clear(state, mid),
        "shown" => {
            let rect = Rect::new(num("x"), num("y"), num("w"), num("h"));
            if rect.w > 0 && rect.h > 0 {
                own_shown(state, mid, rect);
            }
        }
        "hide" => own_hide(state, mid),
        _ => {}
    }
}

/// A key of the remote taken by the keyboard that is up: the shell hears it as an `imekey` event, with whether it is the remote's repeat.
fn own_nav(state: &mut State, code: u32, pressed: bool, repeat: bool) {
    let Some(name) = own_key_name(code) else { return };
    state.ime.own.counts.nav += 1;
    state.shell.push("imekey", serde_json::json!({"k": name, "d": pressed, "r": repeat}));
}

/// The shell's client went: a field it served leaves (the next one goes to LG's keyboard until the shell is back).
pub fn own_gone(state: &mut State) {
    if let Some(mid) = own_active(state) {
        deactivate(state, mid, "the shell's keyboard went");
    }
    own_off(state);
}

/// Called every tick: a keyboard the shell was asked for and did not put on the screen in [`OWN_SHOW_WITHIN`] is given up for a minute, and
/// the field goes to LG's keyboard at once (it is shown there, as the field asked).
pub fn own_tick(state: &mut State, now: Instant) {
    let Some(t) = state.ime.own.pending else { return };
    if now.saturating_duration_since(t) < OWN_SHOW_WITHIN {
        return;
    }
    state.ime.own.pending = None;
    state.ime.own.broken_until = Some(now + OWN_BROKEN_FOR);
    state.ime.own.counts.failed += 1;
    let Some(mid) = own_active(state) else { return };
    state.journal(format!(
        "ime: the shell's keyboard did not come up in {} s; LG's keyboard serves for {} s",
        OWN_SHOW_WITHIN.as_secs(),
        OWN_BROKEN_FOR.as_secs()
    ));
    if let Some(m) = state.ime.model_mut(mid) {
        m.own = false;
        m.own_show = false;
        m.show_wanted = true;
    }
    own_publish(state);
    begin(state, mid);
}

impl State {
    /// An input method holds the keyboard grab: the hardware keys go to it.
    pub fn ime_grabbing(&self) -> bool {
        self.ime.grabbing()
    }

    /// One key to the keyboard that is up: an input method's grabbed keyboard (evdev code, as `wl_keyboard.key` carries it), or the shell's
    /// keyboard as an `imekey` event.
    pub fn grab_key(&mut self, code: u32, pressed: bool) {
        let repeat = pressed && self.ime.grab_down.contains(&code);
        if pressed {
            if !repeat {
                self.ime.grab_down.push(code);
            }
        } else {
            self.ime.grab_down.retain(|c| *c != code);
        }
        if self.ime.own_taking() {
            own_nav(self, code, pressed, repeat);
            self.keys.delivered += 1;
            return;
        }
        let Some(kb) = self.ime.grab.clone().filter(live) else { return };
        let serial = self.next_serial();
        kb.key(serial, crate::output::monotonic_ms(), code, if pressed { KeyState::Pressed } else { KeyState::Released });
        self.ime.counts.grabbed_keys += 1;
        self.keys.delivered += 1;
    }

    /// The Exit key with the keyboard up: the keyboard goes first (LG closes it before anything else).
    pub fn close_keyboard(&mut self, why: &str) {
        match self.ime.active {
            Some(mid) => deactivate(self, mid, why),
            None => self.journal(format!("ime: {why}: a keyboard is up with no text field to leave")),
        }
    }

    /// What the control socket says about the keyboard path.
    pub fn ime_report(&self) -> serde_json::Value {
        let ime = &self.ime;
        let active = ime.active.and_then(|id| ime.model(id)).map(|m| {
            let app = m.surface_id.and_then(|s| self.scene.window_of(s)).and_then(|w| self.scene.window(w)).map(|w| w.app_id().to_string()).unwrap_or_default();
            serde_json::json!({"model": m.id, "surface": m.surface_id.map(|s| s.0), "app": app, "context": m.ctx.as_ref().is_some_and(live), "showWanted": m.show_wanted,
                "own": m.own, "ownShow": m.own_show, "hint": m.hint, "purpose": m.purpose, "enter": m.enter, "max": m.max,
                "surround": m.surround.map(|(len, cursor)| serde_json::json!({"len": len, "cursor": cursor}))})
        });
        let own = &ime.own;
        serde_json::json!({
            "ok": true,
            "methods": ime.methods.iter().filter(|m| m.res.is_alive()).map(|m| serde_json::json!({"id": m.id, "display": m.display})).collect::<Vec<_>>(),
            "models": ime.models.len(),
            "active": active,
            "panel": ime.panel.map(|r| serde_json::json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h})),
            "keyboardUp": self.keyboard_up(),
            "grab": ime.grab.as_ref().is_some_and(live),
            "grabbing": ime.grabbing(),
            "asked": ime.asked,
            "counts": {
                "activations": ime.counts.activations, "declined": ime.counts.declined, "commits": ime.counts.commits, "preedits": ime.counts.preedits,
                "deletes": ime.counts.deletes, "keysyms": ime.counts.keysyms, "grabbedKeys": ime.counts.grabbed_keys,
            },
            "own": {
                "flag": self.config.data_dir.join(OWN_FLAG).exists(),
                "canShow": self.shell_can("keyboard"),
                "serving": own_active(self).is_some(),
                "up": own.up.map(|r| serde_json::json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h})),
                "pending": own.pending.is_some(),
                "broken": own.broken_until.is_some_and(|t| Instant::now() < t),
                "counts": {
                    "activations": own.counts.activations, "shown": own.counts.shown, "texts": own.counts.texts, "keys": own.counts.keys,
                    "clears": own.counts.clears, "hides": own.counts.hides, "nav": own.counts.nav, "failed": own.counts.failed,
                },
            },
        })
    }
}
