//! The panels as one part of the shell, the way `system::layer::Layer` is: the quick panel, the input picker and the notification drawer
//! (the kinds `quick`, `picker`, `drawer`), the context menu and the confirm dialog (`menu`, `confirm`, opened by whoever owns the thing they
//! are about) and the app switcher's chrome (`switcher`), their models fed from the channel, the helper, the keys and the clock, their
//! surfaces drawn with the kit in the panels' band, and what the compositor is told of them (who has the keys, what can be shown now).
//! Nothing here does I/O: effects go to the shell, which hands them to the host.
//!
//! One panel is up at a time, as in the QML overlay (`LumaOverlay.qml`): a `show` of a panel that is up closes it (the picker goes on to
//! the next input instead), a `show` of another gives way to the one that was up, and any panel closes after 45 s with no key and no
//! pointer, when an app comes to the front and when the screen goes off. A confirm dialog that closes that way answers with its safe
//! answer, as Back does, so its owner is never left waiting. The switcher is the compositor's to open and close (its `switcher` topic is
//! what says so); what is up under it gives way, and it is the one thing the idle rule and the front app do not close from here.
//!
//! The owner of a menu or a dialog opens it with the topic `menu.open` or `confirm.ask` (the spec of `menu.rs`, `confirm.rs`) and is told
//! what was picked or answered as `menu.picked` or `confirm.answered`.

use super::confirm::{Confirm, Spec as ConfirmSpec};
use super::confirm_surface::{ConfirmSurface, Target as ConfirmTarget};
use super::drawer::Drawer;
use super::drawer_surface::{DrawerSurface, Target as DrawerTarget};
use super::env::Env;
use super::hold::Released;
use super::life::{Change, Life};
use super::menu::{Menu, Spec as MenuSpec};
use super::menu_surface::{MenuSurface, Target as MenuTarget};
use super::picker::Picker;
use super::picker_surface::{PickerSurface, Target as PickerTarget};
use super::quick::Quick;
use super::quick_surface::QuickSurface;
use super::switcher::Switcher;
use super::switcher_surface::{SwitcherSurface, Target as SwitcherTarget};
use super::{Ask, K, Out};
use crate::kit::lint::Lint;
use crate::kit::pointer::Sample;
use crate::kit::screen::{Effect, Input};
use crate::kit::{Engine, Kit};
use crate::shell::Kinds;
use compositor_protocol::Rgba;
use serde_json::{Value, json};

/// How long a panel stays up with no key and no pointer.
pub const IDLE_MS: u64 = 45_000;
/// How long OK is held on a tile of the quick panel before it is picked up for editing.
pub const HOLD_MS: u64 = 700;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Quick,
    Picker,
    Drawer,
    Menu,
    Confirm,
    Switcher,
}

impl Which {
    pub const ALL: [Which; 6] = [Which::Quick, Which::Picker, Which::Drawer, Which::Menu, Which::Confirm, Which::Switcher];
    /// The kinds the compositor's `show` asks for (the gear, Input, the long press of the gear): the others open by their owners' topics.
    pub const SHOWN: [Which; 3] = [Which::Quick, Which::Picker, Which::Drawer];

    pub fn name(self) -> &'static str {
        match self {
            Which::Quick => "quick",
            Which::Picker => "picker",
            Which::Drawer => "drawer",
            Which::Menu => "menu",
            Which::Confirm => "confirm",
            Which::Switcher => "switcher",
        }
    }

    pub fn parse(kind: &str) -> Option<Which> {
        Self::SHOWN.into_iter().find(|w| w.name() == kind)
    }

    pub fn on(self, kinds: &Kinds) -> bool {
        match self {
            Which::Quick => kinds.quick,
            Which::Picker => kinds.picker,
            Which::Drawer => kinds.drawer,
            Which::Menu => kinds.menu,
            Which::Confirm => kinds.confirm,
            Which::Switcher => kinds.switcher,
        }
    }
}

/// The switcher's chrome fades with the compositor's own motion for the cards (`OPEN_MS`, `EXIT_MS` of its `switcher.rs`) and keeps no layers
/// once it is gone: the cards' pictures are the compositor's, and a parked copy of the chrome has nothing to wait for.
const SWITCHER_LIFE: (u32, u32, u32) = (200, 160, 0);

/// A colour of the palette (premultiplied) as the compositor takes the dim behind the cards: `{r, g, b, a}` straight, each 0 to 1.
fn straight(c: Rgba) -> Value {
    let a = f32::from(c.3);
    let un = |v: u8| if c.3 == 0 { 0.0 } else { f32::from(v) / a };
    json!({"r": un(c.0), "g": un(c.1), "b": un(c.2), "a": a / 255.0})
}

pub struct Panels {
    env: Env,
    quick: Quick,
    quick_s: QuickSurface,
    picker: Picker,
    picker_s: PickerSurface,
    drawer: Drawer,
    drawer_s: DrawerSurface,
    menu: Menu,
    menu_s: MenuSurface,
    confirm: Confirm,
    confirm_s: ConfirmSurface,
    switcher: Switcher,
    /// How the switcher's chrome comes and goes: it follows the compositor's `switcher` topic.
    switcher_life: Life,
    switcher_s: SwitcherSurface,
    redraw: bool,
    idle_due: Option<u64>,
    /// The number of the last `show` taken (the compositor counts them; a replayed one is not a new press).
    last_show: u64,
    /// What a panel tells the rest of the shell (`settings.open`), taken by the shell and fed back as topics.
    events: Vec<(String, Value)>,
    /// What the drawing asks of the compositor (the places of the switcher's cards), taken by the shell after a draw.
    drawn: Vec<Effect>,
}

impl Panels {
    /// `base` is the first z of the panels' band; `theme` the merged theme document.
    pub fn new(base: i32, theme: &Value) -> Self {
        Self {
            env: Env::new(theme.clone()),
            quick: Quick::new(),
            quick_s: QuickSurface::new(base),
            picker: Picker::new(),
            picker_s: PickerSurface::new(base),
            drawer: Drawer::new(),
            drawer_s: DrawerSurface::new(base),
            menu: Menu::new(),
            menu_s: MenuSurface::new(base),
            confirm: Confirm::new(),
            confirm_s: ConfirmSurface::new(base),
            switcher: Switcher::new(),
            switcher_life: Life::new(SWITCHER_LIFE.0, SWITCHER_LIFE.1, SWITCHER_LIFE.2),
            // over the panels: the compositor's pictures are under every layer of the shell, the chrome is the top of the band
            switcher_s: SwitcherSurface::new(base + 40),
            redraw: false,
            idle_due: None,
            last_show: 0,
            events: Vec::new(),
            drawn: Vec::new(),
        }
    }

    fn life(&self, w: Which) -> &Life {
        match w {
            Which::Quick => &self.quick.life,
            Which::Picker => &self.picker.life,
            Which::Drawer => &self.drawer.life,
            Which::Menu => &self.menu.life,
            Which::Confirm => &self.confirm.life,
            Which::Switcher => &self.switcher_life,
        }
    }

    fn life_mut(&mut self, w: Which) -> &mut Life {
        match w {
            Which::Quick => &mut self.quick.life,
            Which::Picker => &mut self.picker.life,
            Which::Drawer => &mut self.drawer.life,
            Which::Menu => &mut self.menu.life,
            Which::Confirm => &mut self.confirm.life,
            Which::Switcher => &mut self.switcher_life,
        }
    }

    /// The panel that is up (wanted on the screen: the keys are its).
    pub fn up(&self, kinds: &Kinds) -> Option<Which> {
        Which::ALL.into_iter().find(|w| w.on(kinds) && self.life(*w).presented())
    }

    /// The subscriptions the panels ask for at the start.
    pub fn start(&mut self, kinds: &Kinds) -> Vec<Effect> {
        let mut out = Vec::new();
        if kinds.quick || kinds.picker || kinds.drawer || kinds.menu || kinds.confirm || kinds.switcher {
            out.extend(self.env.start());
        }
        if kinds.drawer {
            out.extend(self.drawer.log.subscribe());
        }
        if kinds.switcher {
            out.extend(self.switcher.subscribe());
        }
        out
    }

    /// An input of the host to every panel: the compositor's `show`, the owners' `menu.open` and `confirm.ask`, the topics, the helper's,
    /// the services' answers.
    pub fn update(&mut self, kinds: &Kinds, input: &Input, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Input::Topic { topic, data } = input {
            match topic.as_str() {
                "show" => {
                    self.on_show(kinds, data, now, &mut out);
                    return out;
                }
                "menu.open" if kinds.menu => {
                    self.open_menu(MenuSpec::from_json(data), kinds, now, &mut out);
                    return out;
                }
                "confirm.ask" if kinds.confirm => {
                    self.ask(ConfirmSpec::from_json(data), kinds, now, &mut out);
                    return out;
                }
                _ => {}
            }
        }
        let was_front = (self.env.fg.known, self.env.fg.app.clone());
        let (effects, changed) = self.env.update(input, now);
        out.extend(effects);
        if changed {
            self.redraw = true;
            self.quick.sync(&self.env);
            self.picker.sync(&self.env);
        }
        // an app came to the front or the screen went off: what is up goes
        let moved = (was_front.0 && self.env.fg.app != was_front.1) || !self.env.screen_on;
        if moved {
            for w in Which::ALL {
                self.hide(w, now, kinds, &mut out);
            }
        }
        if kinds.switcher {
            let (effects, changed) = self.switcher.update(input, now);
            out.extend(effects);
            self.redraw |= changed;
            // the compositor closes it when another app comes to the front; with the screen off nothing is left to look at it
            if !self.env.screen_on && self.switcher.is_open() {
                out.push(self.switcher.close_effect());
            }
            self.follow_switcher(kinds, now, &mut out);
        }
        if let Input::Fetched { tag, status, data } = input
            && *tag == super::env::tags::AUDIT
            && kinds.drawer
            && self.drawer.answer(*status, data)
        {
            self.redraw = true;
        }
        if kinds.drawer {
            let notes = Drawer::notes(&self.env);
            if self.drawer.log.update(input, &notes, now) {
                self.redraw = true;
            }
        }
        if kinds.quick && matches!(input, Input::Fetched { .. } | Input::Luna { .. }) {
            let o = self.quick.answer(input, &mut self.env, now);
            self.run(Which::Quick, o, kinds, now, &mut out);
        }
        out
    }

    /// The switcher's chrome is up while the compositor says its switcher is open: what was up gives way when it opens.
    fn follow_switcher(&mut self, kinds: &Kinds, now: u64, out: &mut Vec<Effect>) {
        let open = self.switcher.is_open();
        if open && !self.switcher_life.presented() {
            self.make_room(Which::Switcher, kinds, now, out);
            self.switcher_life.show(now);
            self.redraw = true;
        } else if !open && self.switcher_life.presented() {
            self.switcher_life.hide(now);
            self.redraw = true;
        }
    }

    /// What is up gives way to `w`; what is only kept built is dropped.
    fn make_room(&mut self, w: Which, kinds: &Kinds, now: u64, out: &mut Vec<Effect>) {
        for o in Which::ALL.into_iter().filter(|o| *o != w) {
            if self.life(o).presented() {
                self.hide(o, now, kinds, out);
            } else {
                self.life_mut(o).release();
            }
        }
    }

    fn on_show(&mut self, kinds: &Kinds, data: &Value, now: u64, out: &mut Vec<Effect>) {
        let n = data.get("n").and_then(Value::as_u64).unwrap_or(0);
        if n != 0 && n <= self.last_show {
            return;
        }
        self.last_show = self.last_show.max(n);
        let Some(w) = data.get("kind").and_then(Value::as_str).and_then(Which::parse).filter(|w| w.on(kinds)) else { return };
        // the switcher is over everything: the gear's press goes to its keys, not to a panel behind it
        if self.switcher_life.presented() {
            return;
        }
        self.show(w, data.get("warm").and_then(Value::as_bool) == Some(true), kinds, now, out);
    }

    /// The panel is wanted (a press of its key). `warm` builds it hidden, so the slide of the opening that follows starts in a scene that
    /// has it.
    pub fn show(&mut self, w: Which, warm: bool, kinds: &Kinds, now: u64, out: &mut Vec<Effect>) {
        if warm {
            for o in Which::ALL.into_iter().filter(|o| *o != w) {
                self.life_mut(o).release();
            }
            if self.life_mut(w).warm(now).is_some() {
                self.redraw = true;
            }
            return;
        }
        if self.life(w).presented() {
            // the picker goes on to the next input; the others close
            if w == Which::Picker {
                self.picker.next(&self.env);
            } else {
                self.hide(w, now, kinds, out);
            }
            self.idle_due = Some(now + IDLE_MS);
            self.redraw = true;
            return;
        }
        self.make_room(w, kinds, now, out);
        self.life_mut(w).show(now);
        let opened = match w {
            Which::Quick => self.quick.open(&mut self.env, now),
            Which::Picker => self.picker.open(&mut self.env, now),
            Which::Drawer => self.drawer.open(&self.env, now),
            Which::Menu | Which::Confirm | Which::Switcher => Out::default(),
        };
        self.run(w, opened, kinds, now, out);
        self.idle_due = Some(now + IDLE_MS);
        self.redraw = true;
    }

    /// A menu its owner asks for: it opens on the first row that can be picked, over whatever panel was up (that gives way). Nothing opens
    /// over the switcher.
    pub fn open_menu(&mut self, spec: MenuSpec, kinds: &Kinds, now: u64, out: &mut Vec<Effect>) {
        if !kinds.menu || self.switcher_life.presented() {
            return;
        }
        self.make_room(Which::Menu, kinds, now, out);
        self.menu.open(spec);
        self.menu.life.show(now);
        self.idle_due = Some(now + IDLE_MS);
        self.redraw = true;
    }

    /// A question its owner asks: the safe answer is focused first, and any way out of the dialog (Back, the idle close, an app coming to
    /// the front) is that answer.
    pub fn ask(&mut self, spec: ConfirmSpec, kinds: &Kinds, now: u64, out: &mut Vec<Effect>) {
        if !kinds.confirm || self.switcher_life.presented() {
            return;
        }
        self.make_room(Which::Confirm, kinds, now, out);
        self.confirm.ask(spec);
        self.confirm.life.show(now);
        self.idle_due = Some(now + IDLE_MS);
        self.redraw = true;
    }

    fn hide(&mut self, w: Which, now: u64, kinds: &Kinds, out: &mut Vec<Effect>) {
        // the compositor opens and closes the switcher: this does not take its chrome from under its pictures
        if w == Which::Switcher || self.life_mut(w).hide(now).is_none() {
            return;
        }
        let closed = match w {
            Which::Quick => self.quick.close(&mut self.env, now),
            Which::Picker => self.picker.close(&mut self.env, now),
            Which::Drawer | Which::Switcher => Out::default(),
            Which::Menu => {
                self.menu.close();
                Out::default()
            }
            Which::Confirm => {
                // not yet answered: the way out is the safe answer, as Back is
                let answer = if self.confirm.is_up() { self.confirm.key(K::Back) } else { Out::default() };
                self.confirm.close();
                answer
            }
        };
        self.run(w, closed, kinds, now, out);
        if self.up(kinds).is_none() {
            self.idle_due = None;
        }
        self.redraw = true;
    }

    /// What a key, a click or a timer of panel `w` did: its effects, what it asks of the shell, what it tells the owner, and the close.
    fn run(&mut self, w: Which, o: Out, kinds: &Kinds, now: u64, out: &mut Vec<Effect>) {
        out.extend(o.effects);
        let mut opened_settings = false;
        for a in o.asks {
            let Ask::Settings(page) = a;
            opened_settings = true;
            self.events.push(("settings.open".into(), json!({"page": page})));
            if !kinds.settings {
                // LumaTV's Settings is not this process's to open (the QML Home has it): LG's Settings is what opens
                out.push(Effect::Luna { tag: super::env::tags::SETTINGS, uri: "luna://com.webos.applicationManager/launch".into(), payload: json!({"id": "com.palm.app.settings"}), subscribe: false });
            }
        }
        self.events.extend(o.events);
        // a dialog that has been answered is closed: leaving it must not answer it again
        if o.close {
            match w {
                Which::Confirm => self.confirm.close(),
                Which::Menu => self.menu.close(),
                _ => {}
            }
        }
        // the panel closes as Settings opens over it, and when it says so
        if o.close || opened_settings {
            self.hide(w, now, kinds, out);
        }
        self.redraw = true;
    }

    /// What the panels tell the rest of the shell, to be fed back as topics (`settings.open`).
    pub fn take_events(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.events)
    }

    /// A key of the remote. Keys are the panel's while one is up: the arrows act on every press and repeat, OK and Back once; OK on a tile
    /// of the quick panel that can be edited waits for its release (a tap) or its hold.
    pub fn key(&mut self, kinds: &Kinds, name: &str, down: bool, repeat: bool, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        let Some(w) = self.up(kinds) else { return out };
        let Some(k) = K::parse(name) else { return out };
        if down {
            self.idle_due = Some(now + IDLE_MS);
        }
        self.redraw = true;
        let o = match (w, k) {
            (Which::Quick, K::Ok) if self.quick.hold_ready(&self.env) => {
                if down && !repeat {
                    self.quick.hold.press(now, HOLD_MS);
                } else if !down && self.quick.hold.release() == Released::Tap {
                    let o = self.quick.key(K::Ok, &mut self.env, now);
                    self.run(w, o, kinds, now, &mut out);
                }
                return out;
            }
            _ if !down => return out,
            (_, K::Ok | K::Back) if repeat => return out,
            (Which::Quick, k) => self.quick.key(k, &mut self.env, now),
            (Which::Picker, k) => self.picker.key(k, &self.env),
            (Which::Drawer, k) => self.drawer.key(k, &self.env),
            (Which::Menu, k) => self.menu.key(k),
            (Which::Confirm, k) => self.confirm.key(k),
            (Which::Switcher, k) => self.switcher.key(k, now),
        };
        self.run(w, o, kinds, now, &mut out);
        out
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.quick_s.on_key(kit, engine);
        self.picker_s.on_key(engine);
        self.drawer_s.on_key(kit, engine);
        self.menu_s.on_key(engine);
        self.confirm_s.on_key(engine);
        self.switcher_s.on_key(engine);
    }

    /// A sample of the pointer: the cue follows it and a click acts. Any sample keeps the panel up.
    pub fn pointer(&mut self, kinds: &Kinds, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        let Some(w) = self.up(kinds) else { return out };
        if s.visible {
            self.idle_due = Some(now + IDLE_MS);
        }
        let o = match w {
            Which::Quick => match self.quick_s.pointer(kit, engine, now_ns, s) {
                Some((t, side)) => self.quick.click(t, side, &mut self.env, now),
                None => return out,
            },
            Which::Picker => match self.picker_s.pointer(kit, engine, now_ns, s) {
                Some(PickerTarget::Tile(i)) => self.picker.click(i, &self.env),
                Some(PickerTarget::Backdrop) => Out::close(),
                None => return out,
            },
            Which::Drawer => match self.drawer_s.pointer(kit, engine, now_ns, s) {
                Some(DrawerTarget::Clear) => {
                    self.drawer.click_clear(&self.env);
                    Out::default()
                }
                Some(DrawerTarget::Item(i)) => {
                    self.drawer.click_item(i, &self.env);
                    Out::default()
                }
                Some(DrawerTarget::Button(i, b)) => self.drawer.click_button(i, b, &self.env),
                Some(DrawerTarget::Backdrop) => Out::close(),
                None => return out,
            },
            Which::Menu => match self.menu_s.pointer(kit, engine, now_ns, s) {
                Some((MenuTarget::Row(i), seg)) => self.menu.click(i, seg),
                Some((MenuTarget::Backdrop, _)) => self.menu.click_outside(),
                None => return out,
            },
            Which::Confirm => match self.confirm_s.pointer(kit, engine, now_ns, s) {
                Some(ConfirmTarget::Answer(i)) => self.confirm.click(i),
                None => return out,
            },
            Which::Switcher => match self.switcher_s.pointer(kit, engine, now_ns, s) {
                Some(SwitcherTarget::Card(i)) => self.switcher.click_card(i, false, now),
                Some(SwitcherTarget::Quit(i)) => self.switcher.click_card(i, true, now),
                Some(SwitcherTarget::All) => self.switcher.click_all(now),
                Some(SwitcherTarget::Cancel) => self.switcher.click_ask(0, now),
                Some(SwitcherTarget::QuitAll) => self.switcher.click_ask(1, now),
                Some(SwitcherTarget::Backdrop) => self.switcher.click_outside(now),
                None => return out,
            },
        };
        self.run(w, o, kinds, now, &mut out);
        out
    }

    /// The panels' timers and the surfaces' own. Returns whatever the models ask for; `redraw` says whether a surface is to be drawn.
    pub fn tick(&mut self, kinds: &Kinds, kit: &Kit, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        for w in Which::ALL.into_iter().filter(|w| w.on(kinds)) {
            if matches!(self.life_mut(w).tick(now), Some(Change::Opened | Change::Parked | Change::Drop)) {
                self.redraw = true;
            }
        }
        if kinds.quick {
            let o = self.quick.tick(&mut self.env, now);
            self.run(Which::Quick, o, kinds, now, &mut out);
            if self.quick_s.tick(kit, now) {
                self.redraw = true;
            }
        }
        if kinds.picker {
            let o = self.picker.tick(&mut self.env, now);
            self.run(Which::Picker, o, kinds, now, &mut out);
        }
        if kinds.switcher {
            let o = self.switcher.tick(now);
            self.run(Which::Switcher, o, kinds, now, &mut out);
        }
        let (effects, _) = self.env.update(&Input::Tick, now);
        out.extend(effects);
        if self.idle_due.is_some_and(|d| now >= d) {
            self.idle_due = None;
            for w in Which::ALL {
                self.hide(w, now, kinds, &mut out);
            }
        }
        out
    }

    pub fn due_at(&self, kinds: &Kinds) -> Option<u64> {
        let mut due = [self.idle_due, self.env.due_at()].into_iter().flatten().min();
        for w in Which::ALL.into_iter().filter(|w| w.on(kinds)) {
            due = [due, self.life(w).due_at()].into_iter().flatten().min();
        }
        if kinds.quick {
            due = [due, self.quick.due_at()].into_iter().flatten().min();
        }
        if kinds.picker {
            due = [due, self.picker.due_at()].into_iter().flatten().min();
        }
        if kinds.quick {
            due = [due, self.quick_s.due_at()].into_iter().flatten().min();
        }
        if kinds.switcher {
            due = [due, self.switcher.due_at()].into_iter().flatten().min();
        }
        due
    }

    /// Takes the flag that a surface is to be drawn again.
    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    /// Draws what changed. Returns whether anything of a panel is on the screen.
    pub fn apply(&mut self, kinds: &Kinds, kit: &mut Kit, engine: &mut Engine, now_ms: u64, now_ns: u64) -> Result<bool, String> {
        let mut live = false;
        if kinds.quick {
            let view = self.quick.view(&self.env, now_ms);
            match self.quick_s.apply(kit, engine, &view, self.quick.life.phase(), now_ms, now_ns) {
                Ok(on) => live |= on && self.quick.life.visible(),
                Err(e) => return Err(format!("quick panel: {e}")),
            }
        }
        if kinds.picker {
            let view = self.picker.view(&self.env);
            match self.picker_s.apply(kit, engine, &view, self.picker.life.phase(), now_ns) {
                Ok(on) => live |= on && self.picker.life.visible(),
                Err(e) => return Err(format!("input picker: {e}")),
            }
        }
        if kinds.drawer {
            let view = self.drawer.view(&self.env, now_ms);
            match self.drawer_s.apply(kit, engine, &view, self.drawer.life.phase(), now_ns) {
                Ok(on) => live |= on && self.drawer.life.visible(),
                Err(e) => return Err(format!("notification drawer: {e}")),
            }
        }
        let over_video = !self.env.fg.home;
        if kinds.menu
            && let Some(view) = self.menu.view(over_video)
        {
            match self.menu_s.apply(kit, engine, &view, self.menu.life.phase(), now_ns) {
                Ok(on) => live |= on && self.menu.life.visible(),
                Err(e) => return Err(format!("context menu: {e}")),
            }
        }
        if kinds.confirm
            && let Some(view) = self.confirm.view(over_video)
        {
            match self.confirm_s.apply(kit, engine, &view, self.confirm.life.phase(), now_ns) {
                Ok(on) => live |= on && self.confirm.life.visible(),
                Err(e) => return Err(format!("confirm dialog: {e}")),
            }
        }
        if kinds.switcher {
            let view = self.switcher.view(now_ms);
            match self.switcher_s.apply(kit, engine, &view, self.switcher_life.phase(), now_ns) {
                Ok(on) => live |= on && self.switcher_life.visible(),
                Err(e) => return Err(format!("app switcher: {e}")),
            }
            // the cards' places go to the compositor, which moves the pictures there on its own clock
            self.drawn.extend(self.switcher.layout(straight(kit.tokens.extra(kit.mode).scrim_hide), kit.ui.look.reduced_motion));
        }
        Ok(live)
    }

    /// What the last draw asks of the compositor.
    pub fn take_drawn(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.drawn)
    }

    /// Opens the app switcher over what is on the screen (a press of Home with nothing open on Home; Back from the top of Home when the owner
    /// wants it), when it is on, the owner's switch is not off and there is an app to show.
    pub fn open_switcher(&self, kinds: &Kinds) -> Option<Effect> {
        (kinds.switcher && self.env.screen_on && !self.env.ai_prompt_up() && self.switcher_wanted() && self.switcher.count() > 0 && !self.switcher.is_open()).then(|| self.switcher.open_effect())
    }

    /// The owner's switches for the switcher (`remote.appSwitcher`: the second press of Home; `remote.recents`: Back from the top of Home). Both
    /// are off until the TV check.
    fn switcher_wanted(&self) -> bool {
        self.env.switch_set("appSwitcher") || self.env.switch_set("recents")
    }

    /// Whether Back from the top of Home opens the switcher.
    pub fn recents_on(&self) -> bool {
        self.env.switch_set("recents")
    }

    /// Whether a panel wants every key: one is up.
    pub fn wants(&self, kinds: &Kinds) -> bool {
        self.up(kinds).is_some()
    }

    /// The panels this process can show now: the owner's switch for it is not off and the screen is on.
    pub fn can(&self, kinds: &Kinds) -> Vec<&'static str> {
        let mut can = Vec::new();
        if !self.env.screen_on {
            return can;
        }
        if kinds.quick && self.env.switch_on("quickPanel") {
            can.push("quick");
        }
        if kinds.picker && self.env.switch_on("inputPicker") {
            can.push("picker");
        }
        if kinds.drawer {
            can.push("drawer");
        }
        if kinds.switcher && self.switcher_wanted() {
            can.push("switcher");
        }
        can
    }

    /// The panels' own words for the compositor: the phase of each (what the gate reads of `ctl shell`).
    pub fn state(&self, kinds: &Kinds) -> Value {
        let mut s = json!({});
        for w in Which::ALL.into_iter().filter(|w| w.on(kinds)) {
            s[w.name()] = json!(format!("{:?}", self.life(w).phase()).to_lowercase());
        }
        s
    }

    /// Where the surfaces are (the harness reads them).
    pub fn geometry(&self, kinds: &Kinds, engine: &Engine) -> Value {
        let mut g = json!({});
        if kinds.quick {
            g["quick"] = self.quick_s.geometry(engine);
        }
        if kinds.picker {
            g["picker"] = self.picker_s.geometry(engine);
        }
        if kinds.drawer {
            g["drawer"] = self.drawer_s.geometry(engine);
        }
        if kinds.menu {
            g["menu"] = self.menu_s.geometry(engine);
        }
        if kinds.confirm {
            g["confirm"] = self.confirm_s.geometry(engine);
        }
        if kinds.switcher {
            g["switcher"] = self.switcher_s.geometry(engine);
        }
        g
    }

    /// Describes what is on the screen to a lint.
    pub fn feed_lint(&self, kinds: &Kinds, kit: &mut Kit, lint: &mut Lint) {
        if kinds.quick {
            self.quick_s.feed_lint(kit, lint);
        }
        if kinds.picker {
            self.picker_s.feed_lint(kit, lint);
        }
        if kinds.drawer {
            self.drawer_s.feed_lint(kit, lint);
        }
        if kinds.menu {
            self.menu_s.feed_lint(kit, lint);
        }
        if kinds.confirm {
            self.confirm_s.feed_lint(kit, lint);
        }
        if kinds.switcher {
            self.switcher_s.feed_lint(kit, lint);
        }
    }

    /// The look changed: the surfaces are made again at the next draw.
    pub fn restyle(&mut self, engine: &mut Engine) {
        self.quick_s.restyle(engine);
        self.picker_s.restyle(engine);
        self.drawer_s.restyle(engine);
        self.menu_s.restyle(engine);
        self.confirm_s.restyle(engine);
        self.switcher_s.restyle(engine);
        self.redraw = true;
    }

    /// Unread notes: the bell's badge.
    pub fn unread(&self) -> usize {
        self.drawer.log.unread(&Drawer::notes(&self.env))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;

    struct Rig {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        p: Panels,
        kinds: Kinds,
        now: u64,
        n: u64,
    }

    impl Rig {
        fn new(theme: Value) -> Self {
            let (h, kit) = testkit::kit(1.0);
            let engine = Engine::new(&kit).expect("an engine");
            Self { _h: h, kit, engine, p: Panels::new(crate::kit::z::PANELS, &theme), kinds: Kinds::parse("quick,picker,drawer"), now: 10_000, n: 0 }
        }

        fn show(&mut self, kind: &str, warm: bool) {
            self.n += 1;
            let input = Input::Topic { topic: "show".into(), data: json!({"kind": kind, "warm": warm, "n": self.n}) };
            self.p.update(&self.kinds, &input, self.now);
        }

        fn apply(&mut self) -> bool {
            self.p.apply(&self.kinds, &mut self.kit, &mut self.engine, self.now, self.now * 1_000_000).expect("drawn")
        }

        fn key(&mut self, name: &str) {
            self.now += 40;
            self.p.key(&self.kinds, name, true, false, self.now);
            self.p.key(&self.kinds, name, false, false, self.now + 10);
        }

        fn tick(&mut self, ms: u64) {
            self.now += ms;
            self.p.tick(&self.kinds, &self.kit, self.now);
        }

        fn phase(&self, w: Which) -> String {
            format!("{:?}", self.p.life(w).phase())
        }
    }

    #[test]
    fn a_show_opens_the_panel_it_takes_the_keys_and_back_closes_it_and_the_layers_go_at_last() {
        let mut r = Rig::new(json!({}));
        assert!(r.p.can(&r.kinds).contains(&"quick") && !r.p.wants(&r.kinds));
        r.show("quick", false);
        assert_eq!(r.p.up(&r.kinds), Some(Which::Quick));
        assert!(r.p.wants(&r.kinds));
        assert!(r.apply());
        assert_eq!(r.p.geometry(&r.kinds, &r.engine)["quick"]["sheet"]["x"], 1320.0);
        r.key("back");
        assert!(!r.p.wants(&r.kinds) && r.phase(Which::Quick) == "Leaving");
        r.tick(200);
        assert_eq!(r.phase(Which::Quick), "Parked");
        r.tick(4100);
        assert_eq!(r.phase(Which::Quick), "Gone");
        assert!(!r.apply(), "nothing of it is on the screen");
    }

    #[test]
    fn one_panel_is_up_at_a_time_a_second_press_closes_it_and_the_input_key_goes_on_to_the_next_input() {
        let mut r = Rig::new(json!({}));
        r.show("quick", false);
        r.show("drawer", false);
        assert_eq!(r.p.up(&r.kinds), Some(Which::Drawer));
        assert_eq!(r.phase(Which::Quick), "Leaving", "the one that was up gives way");
        r.show("drawer", false);
        assert_eq!(r.p.up(&r.kinds), None, "a second press closes it");
        r.tick(300);
        r.p.env.inputs.parse(&json!({"devices": [{"appId": "com.webos.app.hdmi1", "label": "Apple TV", "connected": true}, {"appId": "com.webos.app.hdmi2", "label": "Denon", "connected": true}]}));
        r.show("picker", false);
        let first = r.p.picker.index();
        r.show("picker", false);
        assert_eq!(r.p.up(&r.kinds), Some(Which::Picker), "the picker stays up");
        assert_ne!(r.p.picker.index(), first, "and goes on");
    }

    #[test]
    fn a_replayed_show_is_not_a_press_and_a_warm_one_only_builds() {
        let mut r = Rig::new(json!({}));
        r.n = 4;
        let input = Input::Topic { topic: "show".into(), data: json!({"kind": "quick", "warm": false, "n": 5}) };
        r.p.update(&r.kinds, &input, r.now);
        r.key("back");
        r.tick(200);
        r.p.update(&r.kinds, &input, r.now);
        assert_eq!(r.p.up(&r.kinds), None, "the same number again is the replay of the state");
        r.tick(5000);
        r.n = 9;
        r.show("quick", true);
        assert_eq!(r.phase(Which::Quick), "Parked");
        assert!(!r.apply() && r.p.quick_s.is_up(), "built, and not on the screen");
        assert!(!r.p.wants(&r.kinds));
    }

    #[test]
    fn a_panel_closes_after_45_seconds_without_a_key_and_a_key_keeps_it_up() {
        let mut r = Rig::new(json!({}));
        r.show("drawer", false);
        r.tick(30_000);
        assert!(r.p.wants(&r.kinds));
        r.key("down");
        r.tick(30_000);
        assert!(r.p.wants(&r.kinds), "the key began the 45 seconds again");
        r.tick(16_000);
        assert!(!r.p.wants(&r.kinds));
    }

    #[test]
    fn an_app_coming_to_the_front_or_the_screen_going_off_closes_what_is_up() {
        let mut r = Rig::new(json!({}));
        let fg = |app: &str| Input::Topic { topic: "foreground".into(), data: json!({"app": app, "home": false}) };
        r.p.update(&r.kinds, &fg("com.a"), r.now);
        r.show("quick", false);
        r.p.update(&r.kinds, &fg("com.a"), r.now);
        assert!(r.p.wants(&r.kinds), "the same app is no change");
        r.p.update(&r.kinds, &fg("com.b"), r.now);
        assert!(!r.p.wants(&r.kinds));
        r.tick(5000);
        r.show("picker", false);
        r.p.update(&r.kinds, &Input::Topic { topic: "screen".into(), data: json!({"on": false}) }, r.now);
        assert!(!r.p.wants(&r.kinds) && r.p.can(&r.kinds).is_empty());
    }

    #[test]
    fn the_owners_switch_decides_what_can_be_shown() {
        let r = Rig::new(json!({"remote": {"quickPanel": false}}));
        assert_eq!(r.p.can(&r.kinds), vec!["picker", "drawer"]);
        assert_eq!(r.p.state(&r.kinds)["quick"], "gone");
    }

    fn rig_with(kinds: &str, theme: Value) -> Rig {
        let mut r = Rig::new(theme);
        r.kinds = Kinds::parse(kinds);
        r
    }

    impl Rig {
        fn topic(&mut self, topic: &str, data: Value) -> Vec<Effect> {
            self.p.update(&self.kinds, &Input::Topic { topic: topic.into(), data }, self.now)
        }

        fn press(&mut self, name: &str) -> Vec<Effect> {
            self.now += 40;
            let mut out = self.p.key(&self.kinds, name, true, false, self.now);
            out.extend(self.p.key(&self.kinds, name, false, false, self.now + 10));
            out
        }

        fn posts(effects: &[Effect]) -> Vec<Value> {
            effects.iter().filter_map(|e| if let Effect::Channel { path, body } = e { (path == "/v1/switcher").then(|| body.clone()) } else { None }).collect()
        }
    }

    fn dock_menu() -> Value {
        json!({"title": "Crunchyroll", "icon": "play", "items": [{"id": "open", "icon": "play", "label": "Open"}, {"id": "uninstall", "icon": "trash-2", "label": "Uninstall", "danger": true}]})
    }

    fn uninstall() -> Value {
        json!({"icon": "trash-2", "title": "Uninstall Crunchyroll?", "body": "The app is removed.", "answers": [{"id": "keep", "label": "Keep it"}, {"id": "uninstall", "label": "Uninstall", "danger": true}], "focus": 0})
    }

    #[test]
    fn a_menu_its_owner_opens_takes_the_keys_tells_what_was_picked_and_closes() {
        let mut r = rig_with("menu,confirm,quick", json!({}));
        r.topic("menu.open", dock_menu());
        assert_eq!(r.p.up(&r.kinds), Some(Which::Menu));
        assert!(r.apply());
        assert_eq!(r.p.geometry(&r.kinds, &r.engine)["menu"]["rows"].as_array().unwrap().len(), 2);
        r.press("down");
        r.press("ok");
        assert_eq!(r.p.take_events(), vec![("menu.picked".to_string(), json!({"id": "uninstall", "index": 0}))]);
        assert!(!r.p.wants(&r.kinds) && r.phase(Which::Menu) == "Leaving");
        r.topic("menu.open", dock_menu());
        r.press("back");
        assert!(r.p.take_events().is_empty(), "Back picks nothing");
    }

    #[test]
    fn a_dialog_answers_with_its_safe_answer_when_back_is_pressed_when_idle_and_when_an_app_comes_to_the_front() {
        let mut r = rig_with("menu,confirm,quick", json!({}));
        r.topic("confirm.ask", uninstall());
        assert_eq!(r.p.up(&r.kinds), Some(Which::Confirm));
        r.press("right");
        r.press("ok");
        assert_eq!(r.p.take_events(), vec![("confirm.answered".to_string(), json!({"id": "uninstall"}))]);
        r.tick(5000);
        r.topic("confirm.ask", uninstall());
        r.press("back");
        assert_eq!(r.p.take_events(), vec![("confirm.answered".to_string(), json!({"id": "keep"}))], "Back is the safe answer");
        r.tick(5000);
        r.topic("confirm.ask", uninstall());
        r.tick(46_000);
        assert_eq!(r.p.take_events(), vec![("confirm.answered".to_string(), json!({"id": "keep"}))], "left alone it answers the safe one");
        r.tick(5000);
        r.topic("foreground", json!({"app": "com.a", "home": false}));
        r.topic("confirm.ask", uninstall());
        r.topic("foreground", json!({"app": "com.b", "home": false}));
        assert_eq!(r.p.take_events(), vec![("confirm.answered".to_string(), json!({"id": "keep"}))], "an app that comes to the front is no answer to a question");
    }

    #[test]
    fn a_dialog_opens_over_a_panel_that_gives_way_and_nothing_opens_without_its_kind() {
        let mut r = rig_with("menu,confirm,quick", json!({}));
        r.show("quick", false);
        r.topic("confirm.ask", uninstall());
        assert_eq!(r.p.up(&r.kinds), Some(Which::Confirm));
        assert_eq!(r.phase(Which::Quick), "Leaving");
        let mut q = rig_with("quick", json!({}));
        q.topic("confirm.ask", uninstall());
        q.topic("menu.open", dock_menu());
        assert!(!q.p.wants(&q.kinds), "a kind that is not on opens nothing");
    }

    fn apps() -> Value {
        json!([
            {"id": "plex", "title": "Plex", "picture": "live", "w": 1920, "h": 1080, "state": ""},
            {"id": "netflix", "title": "Netflix", "picture": "live", "w": 1920, "h": 1080, "state": ""}
        ])
    }

    #[test]
    fn the_switcher_follows_the_compositors_topic_places_the_cards_and_asks_for_the_app_chosen() {
        let mut r = rig_with("switcher,quick", json!({"remote": {"appSwitcher": true}}));
        assert!(r.p.can(&r.kinds).contains(&"switcher"));
        r.topic("apps", apps());
        assert!(!r.p.wants(&r.kinds));
        r.topic("switcher", json!({"open": true, "from": "", "n": 1}));
        assert_eq!(r.p.up(&r.kinds), Some(Which::Switcher), "the compositor opened it: the keys are its");
        assert!(r.apply());
        let layout = Rig::posts(&r.p.take_drawn());
        assert_eq!(layout.len(), 1);
        assert_eq!(layout[0]["op"], "layout");
        assert_eq!(layout[0]["cards"].as_array().unwrap().len(), 2, "both pictures are live: both are placed");
        assert_eq!(layout[0]["cards"][0]["id"], "plex");
        assert_eq!(layout[0]["scrim"]["a"].as_f64().map(|a| a > 0.9), Some(true), "the dim the compositor draws behind the cards");
        assert!(Rig::posts(&{ r.apply(); r.p.take_drawn() }).is_empty(), "the same places are not sent twice");
        r.press("right");
        let posts = Rig::posts(&r.press("ok"));
        assert_eq!(posts, vec![json!({"op": "activate", "id": "netflix"})]);
        r.topic("switcher", json!({"open": false, "from": "", "n": 1}));
        assert!(!r.p.wants(&r.kinds));
        r.tick(300);
        r.tick(10);
        assert_eq!(r.phase(Which::Switcher), "Gone", "the chrome keeps no layers once it has left");
    }

    #[test]
    fn the_switcher_takes_over_from_a_panel_and_back_asks_the_compositor_to_close_it() {
        let mut r = rig_with("switcher,quick", json!({"remote": {"appSwitcher": true}}));
        r.show("quick", false);
        r.topic("apps", apps());
        r.topic("switcher", json!({"open": true, "from": "", "n": 1}));
        assert_eq!(r.phase(Which::Quick), "Leaving");
        r.show("quick", false);
        assert_eq!(r.p.up(&r.kinds), Some(Which::Switcher), "the gear does not open a panel behind the switcher");
        assert_eq!(Rig::posts(&r.press("back")), vec![json!({"op": "close"})]);
        // the compositor's word is what closes it: the keys stay with it until it says so
        assert!(r.p.wants(&r.kinds));
    }

    #[test]
    fn the_switcher_is_offered_only_when_the_owner_switched_it_on_and_something_is_open() {
        let off = rig_with("switcher", json!({}));
        assert!(!off.p.can(&off.kinds).contains(&"switcher") && off.p.open_switcher(&off.kinds).is_none());
        let mut on = rig_with("switcher", json!({"remote": {"recents": true}}));
        assert!(on.p.open_switcher(&on.kinds).is_none(), "nothing is open");
        on.topic("apps", apps());
        assert_eq!(on.p.open_switcher(&on.kinds), Some(Effect::Channel { path: "/v1/switcher".into(), body: json!({"op": "open"}) }));
        assert!(on.p.recents_on());
        on.topic("screen", json!({"on": false}));
        assert!(on.p.open_switcher(&on.kinds).is_none(), "not with the screen off");
    }
}
