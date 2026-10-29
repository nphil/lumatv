//! The panels as one part of the shell, the way `system::layer::Layer` is: the quick panel, the input picker and the notification drawer
//! (the kinds `quick`, `picker`, `drawer`), their models fed from the channel, the helper, the keys and the clock, their surfaces drawn
//! with the kit in the panels' band, and what the compositor is told of them (who has the keys, what can be shown now). Nothing here does
//! I/O: effects go to the shell, which hands them to the host.
//!
//! One panel is up at a time, as in the QML overlay (`LumaOverlay.qml`): a `show` of a panel that is up closes it (the picker goes on to
//! the next input instead), a `show` of another gives way to the one that was up, and any panel closes after 45 s with no key and no
//! pointer, when an app comes to the front and when the screen goes off.

use super::drawer::Drawer;
use super::drawer_surface::{DrawerSurface, Target as DrawerTarget};
use super::env::Env;
use super::hold::Released;
use super::life::{Change, Life};
use super::picker::Picker;
use super::picker_surface::{PickerSurface, Target as PickerTarget};
use super::quick::Quick;
use super::quick_surface::QuickSurface;
use super::{Ask, K, Out};
use crate::kit::lint::Lint;
use crate::kit::pointer::Sample;
use crate::kit::screen::{Effect, Input};
use crate::kit::{Engine, Kit};
use crate::shell::Kinds;
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
}

impl Which {
    pub const ALL: [Which; 3] = [Which::Quick, Which::Picker, Which::Drawer];

    pub fn name(self) -> &'static str {
        match self {
            Which::Quick => "quick",
            Which::Picker => "picker",
            Which::Drawer => "drawer",
        }
    }

    pub fn parse(kind: &str) -> Option<Which> {
        Self::ALL.into_iter().find(|w| w.name() == kind)
    }

    pub fn on(self, kinds: &Kinds) -> bool {
        match self {
            Which::Quick => kinds.quick,
            Which::Picker => kinds.picker,
            Which::Drawer => kinds.drawer,
        }
    }
}

pub struct Panels {
    env: Env,
    quick: Quick,
    quick_s: QuickSurface,
    picker: Picker,
    picker_s: PickerSurface,
    drawer: Drawer,
    drawer_s: DrawerSurface,
    redraw: bool,
    idle_due: Option<u64>,
    /// The number of the last `show` taken (the compositor counts them; a replayed one is not a new press).
    last_show: u64,
    /// What a panel tells the rest of the shell (`settings.open`), taken by the shell and fed back as topics.
    events: Vec<(String, Value)>,
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
            redraw: false,
            idle_due: None,
            last_show: 0,
            events: Vec::new(),
        }
    }

    fn life(&self, w: Which) -> &Life {
        match w {
            Which::Quick => &self.quick.life,
            Which::Picker => &self.picker.life,
            Which::Drawer => &self.drawer.life,
        }
    }

    fn life_mut(&mut self, w: Which) -> &mut Life {
        match w {
            Which::Quick => &mut self.quick.life,
            Which::Picker => &mut self.picker.life,
            Which::Drawer => &mut self.drawer.life,
        }
    }

    /// The panel that is up (wanted on the screen: the keys are its).
    pub fn up(&self, kinds: &Kinds) -> Option<Which> {
        Which::ALL.into_iter().find(|w| w.on(kinds) && self.life(*w).presented())
    }

    /// The subscriptions the panels ask for at the start.
    pub fn start(&mut self, kinds: &Kinds) -> Vec<Effect> {
        let mut out = Vec::new();
        if kinds.quick || kinds.picker || kinds.drawer {
            out.extend(self.env.start());
        }
        if kinds.drawer {
            out.extend(self.drawer.log.subscribe());
        }
        out
    }

    /// An input of the host to every panel: the compositor's `show`, the topics, the helper's, the services' answers.
    pub fn update(&mut self, kinds: &Kinds, input: &Input, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Input::Topic { topic, data } = input
            && topic == "show"
        {
            self.on_show(kinds, data, now, &mut out);
            return out;
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

    fn on_show(&mut self, kinds: &Kinds, data: &Value, now: u64, out: &mut Vec<Effect>) {
        let n = data.get("n").and_then(Value::as_u64).unwrap_or(0);
        if n != 0 && n <= self.last_show {
            return;
        }
        self.last_show = self.last_show.max(n);
        let Some(w) = data.get("kind").and_then(Value::as_str).and_then(Which::parse).filter(|w| w.on(kinds)) else { return };
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
        for o in Which::ALL.into_iter().filter(|o| *o != w) {
            if self.life(o).presented() {
                self.hide(o, now, kinds, out);
            } else {
                self.life_mut(o).release();
            }
        }
        self.life_mut(w).show(now);
        let opened = match w {
            Which::Quick => self.quick.open(&mut self.env, now),
            Which::Picker => self.picker.open(&mut self.env, now),
            Which::Drawer => self.drawer.open(&self.env, now),
        };
        self.run(w, opened, kinds, now, out);
        self.idle_due = Some(now + IDLE_MS);
        self.redraw = true;
    }

    fn hide(&mut self, w: Which, now: u64, kinds: &Kinds, out: &mut Vec<Effect>) {
        if self.life_mut(w).hide(now).is_none() {
            return;
        }
        let closed = match w {
            Which::Quick => self.quick.close(&mut self.env, now),
            Which::Picker => self.picker.close(&mut self.env, now),
            Which::Drawer => Out::default(),
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
        };
        self.run(w, o, kinds, now, &mut out);
        out
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.quick_s.on_key(kit, engine);
        self.picker_s.on_key(engine);
        self.drawer_s.on_key(kit, engine);
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
        Ok(live)
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
    }

    /// The look changed: the surfaces are made again at the next draw.
    pub fn restyle(&mut self, engine: &mut Engine) {
        self.quick_s.restyle(engine);
        self.picker_s.restyle(engine);
        self.drawer_s.restyle(engine);
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
}
