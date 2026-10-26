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
use crate::surface::{Role, SurfaceId};
use crate::window::{WindowId, WindowType};
use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect, State, plain_global};

/// The display an input method serves: the set has one.
const DISPLAY: u32 = 0;
/// At most one request for an input method in this long: the system takes a moment to start one.
const ASK_EVERY: Duration = Duration::from_secs(3);

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
    match std::process::Command::new(initctl).args(["emit", "ime-activate"]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
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

    pub fn grabbing(&self) -> bool {
        self.grab.as_ref().is_some_and(live)
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
        // Version 2 names its display with a request of its own; a client of version 1 has no way to and serves the first.
        let display = (res.version() < 2).then_some(DISPLAY);
        state.ime.methods.push(Method { id, res, display });
        let who = state.client_info(client);
        state.journal(format!("ime: an input method bound by {who}"));
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
            state.ime.models.push(Model { id: mid, res, active: false, surface: None, surface_id: None, ctx: None, method: None, show_wanted: false, preferred: None });
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
        state.note_request("text_model", resource.id().protocol_id(), &format!("{request:?}"));
        // What the field says goes to the keyboard's context while the model is active (LG forwards it the same way and drops it
        // while there is no context).
        let ctx = state.ime.model(mid).filter(|m| m.active).and_then(|m| m.ctx.clone()).filter(live);
        match request {
            text_model::Request::Activate { surface, .. } => activate(state, mid, &surface),
            text_model::Request::Deactivate { .. } => deactivate(state, mid, "the app deactivated it"),
            text_model::Request::SetSurroundingText { text, cursor, anchor } => {
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
                if let Some(c) = ctx {
                    c.content_type(hint, purpose);
                }
            }
            text_model::Request::SetEnterKeyType { enter_key_type } => {
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
        state.note_request("input_method_context", resource.id().protocol_id(), &format!("{request:?}"));
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
    fn request(state: &mut Self, _client: &Client, resource: &WlKeyboard, request: wl_keyboard::Request, _data: &GrabData, _dh: &DisplayHandle, _di: &mut DataInit<'_, Self>) {
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
        deactivate(state, mid, "activated on another surface");
    }
    if let Some(prev) = state.ime.active.filter(|p| *p != mid) {
        deactivate(state, prev, "another text field activated");
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
}

/// Makes the model's context in the input method's client and tells the app it entered, or asks for an input method first.
fn begin(state: &mut State, mid: u32) {
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
    let Some(m) = state.ime.model_mut(mid) else { return };
    if !m.active {
        return;
    }
    m.active = false;
    m.show_wanted = false;
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

/// The keyboard window's rectangle as LG's `KeyboardView` has it: the full width, anchored at the bottom, as high as the window; the
/// rectangle the app asked for (`set_input_panel_rect`) when it asked for one.
pub fn panel_rect(size: (i32, i32), preferred: Option<Rect>) -> Rect {
    preferred.unwrap_or(Rect::new(0, OUTPUT_HEIGHT as i32 - size.1, OUTPUT_WIDTH as i32, size.1))
}

/// Compares the keyboard on the screen with what the active field was last told, and tells it what changed: it is shown or gone
/// (`input_panel_state`) and where (`input_panel_rect`).
pub fn refresh_panel(state: &mut State) {
    let now = state
        .scene
        .windows()
        .filter(|w| w.kind == WindowType::Keyboard && w.map_serial > 0)
        .filter_map(|w| state.surfaces.get(w.surface))
        .find(|s| s.is_mapped() && s.size().1 > 0)
        .map(|s| panel_rect(s.size(), state.scene.keyboard_pref));
    if now == state.ime.panel {
        return;
    }
    let was_up = state.ime.panel.is_some();
    state.ime.panel = now;
    match now {
        Some(r) => state.journal(format!("ime: the keyboard is on the screen: {}x{} at ({}, {})", r.w, r.h, r.x, r.y)),
        None => state.journal("ime: the keyboard is off the screen".into()),
    }
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

impl State {
    /// An input method holds the keyboard grab: the hardware keys go to it.
    pub fn ime_grabbing(&self) -> bool {
        self.ime.grabbing()
    }

    /// One key to the input method's grabbed keyboard (evdev code, as `wl_keyboard.key` carries it).
    pub fn grab_key(&mut self, code: u32, pressed: bool) {
        if pressed {
            if !self.ime.grab_down.contains(&code) {
                self.ime.grab_down.push(code);
            }
        } else {
            self.ime.grab_down.retain(|c| *c != code);
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
            serde_json::json!({"model": m.id, "surface": m.surface_id.map(|s| s.0), "app": app, "context": m.ctx.as_ref().is_some_and(live), "showWanted": m.show_wanted})
        });
        serde_json::json!({
            "ok": true,
            "methods": ime.methods.iter().filter(|m| m.res.is_alive()).map(|m| serde_json::json!({"id": m.id, "display": m.display})).collect::<Vec<_>>(),
            "models": ime.models.len(),
            "active": active,
            "panel": ime.panel.map(|r| serde_json::json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h})),
            "keyboardUp": self.keyboard_up(),
            "grab": ime.grabbing(),
            "asked": ime.asked,
            "counts": {
                "activations": ime.counts.activations, "declined": ime.counts.declined, "commits": ime.counts.commits, "preedits": ime.counts.preedits,
                "deletes": ime.counts.deletes, "keysyms": ime.counts.keysyms, "grabbedKeys": ime.counts.grabbed_keys,
            },
        })
    }
}
