//! The guide as one part of the shell: the model fed from the channel, the luna bridge, the keys and the clock, and the surface drawn in the
//! guide's band of the engine's z. Nothing here does I/O: effects go to the shell, which hands them to the host. With a fixture (host runs
//! only) the guide's calls of LG's services are answered here, a moment later, as the bridge would.

use super::fixture::Fixture;
use super::model::{Guide, Target};
use super::surface::GuideSurface;
use crate::host::channels as ch;
use crate::kit::lint::Lint;
use crate::kit::pointer::Sample;
use crate::kit::screen::{Effect, Input};
use crate::kit::{Engine, Kit};
use serde_json::{Value, json};

/// How long LG's guide service takes to answer, as the fixture has it (the TV's own is 160 ms for eight channels and six hours).
const LATENCY_MS: u64 = 90;
/// How soon the guide is looked at again while rows are still to be made (they come a few a frame) and while a scroll is moving: what waits
/// for the scroll to stop (the titles pinned to the edge) is shown once it has, and the plates the focus has left go. Nothing at rest.
const BUILD_MS: u64 = 8;
const SETTLE_MS: u64 = 50;

pub struct GuideHost {
    pub model: Guide,
    surface: GuideSurface,
    redraw: bool,
    /// The app in front, from the `foreground` topic, and the one the guide was opened over: a different app coming to the front ends it.
    front: String,
    over: String,
    fixture: Option<Fixture>,
    replies: Vec<(u64, u64, Value)>,
    /// The time of the last input, and when the guide next wants to be looked at for what is moving.
    now: u64,
    follow: Option<u64>,
}

impl GuideHost {
    /// `z` is the guide's band base (`system_z + kit::z::GUIDE`).
    pub fn new(z: i32) -> Self {
        Self { model: Guide::new(), surface: GuideSurface::new(z), redraw: false, front: String::new(), over: String::new(), fixture: None, replies: Vec::new(), now: 0, follow: None }
    }

    /// The host harness answers the guide's calls itself, with a make-believe set of channels and listings (logos from `logos` when given).
    pub fn with_fixture(&mut self, logos: Option<&std::path::Path>) {
        self.fixture = Some(Fixture::new(logos));
    }

    pub fn is_open(&self) -> bool {
        self.model.is_open()
    }

    /// The guide has the keys while it is open.
    pub fn wants(&self) -> bool {
        self.model.wants()
    }

    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    /// What the shell tells the compositor it takes. Live TV in front: the Guide key, which Live TV answers by launching LG's guide (the
    /// compositor gives the key to the shell only while the shell lists it, so LG's is what opens whenever the shell is not running). Open:
    /// also the channel rocker, which pages the guide, Info, which opens its sheet, and a turn of the Magic Remote's wheel, which moves a row.
    pub fn takes(&self) -> Vec<&'static str> {
        if self.model.wants() {
            vec!["chup", "chdown", "info", "wheelup", "wheeldown", "guide"]
        } else if self.front == ch::LIVE_TV_APP {
            vec!["guide"]
        } else {
            Vec::new()
        }
    }

    /// The Guide key opens the guide when it is claimed (Live TV in front, the guide not open). The guide's own key is ignored while it is open.
    pub fn guide_key(&mut self, now: u64) -> Vec<Effect> {
        if self.model.wants() || self.front != ch::LIVE_TV_APP {
            return Vec::new();
        }
        self.open(now)
    }

    /// The calls the fixture answers go to it; the rest (the tune) go on to the bridge.
    fn route(&mut self, fx: Vec<Effect>, now: u64) -> Vec<Effect> {
        self.now = self.now.max(now);
        if !fx.is_empty() {
            self.redraw |= fx.iter().any(|e| matches!(e, Effect::Redraw));
        }
        let Some(f) = &self.fixture else { return fx };
        let mut out = Vec::new();
        for e in fx {
            match e {
                Effect::Luna { tag, uri, payload, .. } if Fixture::stands_in_for(&uri) => {
                    if let Some(reply) = f.answer(&uri, &payload, now) {
                        self.replies.push((now + LATENCY_MS, tag, reply));
                    }
                }
                other => out.push(other),
            }
        }
        out
    }

    pub fn open(&mut self, now: u64) -> Vec<Effect> {
        self.over = self.front.clone();
        let fx = self.model.open(now);
        self.redraw = true;
        self.route(fx, now)
    }

    pub fn close(&mut self, why: &str, now: u64) -> Vec<Effect> {
        let fx = self.model.close(why, now);
        self.follow = None;
        self.redraw = true;
        self.route(fx, now)
    }

    /// A topic, a luna answer or the clock for the model. The app in front is kept; another app coming to the front ends the guide.
    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Input::Topic { topic, data } = input {
            match topic.as_str() {
                "foreground" => {
                    let app = data.get("app").and_then(Value::as_str).unwrap_or("").to_string();
                    if self.model.is_open() && app != self.over && !app.is_empty() {
                        out.extend(self.close("another app came to the front", now));
                    }
                    self.front = app;
                }
                "screen" if data.get("on").and_then(Value::as_bool) == Some(false) && self.model.is_open() => out.extend(self.close("the screen went off", now)),
                _ => {}
            }
        }
        if matches!(input, Input::Tick) {
            let due: Vec<(u64, u64, Value)> = self.replies.iter().filter(|r| r.0 <= now).cloned().collect();
            self.replies.retain(|r| r.0 > now);
            for (_, tag, payload) in due {
                let fx = self.model.update(&Input::Luna { tag, payload }, now);
                out.extend(self.route(fx, now));
            }
        }
        let fx = self.model.update(input, now);
        out.extend(self.route(fx, now));
        out
    }

    /// A key of the remote: the pointer's cue goes first.
    pub fn key(&mut self, name: &str, down: bool, repeat: bool, engine: &mut Engine, now: u64) -> Vec<Effect> {
        if down && self.model.is_open() {
            self.surface.on_key(engine);
        }
        let fx = self.model.key(name, down, repeat, now);
        self.redraw = true;
        self.route(fx, now)
    }

    /// A sample of the pointer: the cue follows it and a click is what OK would be.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample, now: u64) -> Vec<Effect> {
        self.now = self.now.max(now);
        if !self.model.is_open() {
            return Vec::new();
        }
        if s.visible {
            self.model.touch(now);
        }
        match self.surface.pointer(kit, engine, now_ns, s, &self.model) {
            Some(t) => self.click(t, now),
            None => Vec::new(),
        }
    }

    pub fn click(&mut self, t: Target, now: u64) -> Vec<Effect> {
        let fx = self.model.click(t, now);
        self.redraw = true;
        self.route(fx, now)
    }

    /// Draws what changed. Returns whether any of the guide is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, now_ns: u64) -> std::io::Result<bool> {
        let page = self.surface.page(kit);
        if self.model.set_page(page) {
            self.redraw = true;
        }
        let on = self.surface.apply(kit, engine, &self.model, now_ns)?;
        // nothing wakes the shell by itself when a spring stops or the rows are all made: the guide asks for the moment, while it needs one
        self.follow = self.surface.busy(engine, now_ns).then(|| self.now + if self.surface.pending() { BUILD_MS } else { SETTLE_MS });
        if self.follow.is_some() {
            self.redraw = true;
        }
        Ok(on)
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.model.due_at(), self.replies.iter().map(|r| r.0).min(), self.follow].into_iter().flatten().min()
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.surface.restyle(engine);
    }

    pub fn state(&self) -> Value {
        json!({"guide": self.model.state()})
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        json!({"guide": self.surface.geometry(engine)})
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        self.surface.feed_lint(kit, lint, &self.model);
    }

    /// How many layers the guide holds.
    pub fn layers(&self) -> usize {
        self.surface.layers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-04 04:20 UTC.
    const NOW: u64 = 1_793_766_000_000;

    fn front(app: &str) -> Input {
        Input::Topic { topic: "foreground".into(), data: json!({"app": app, "home": false}) }
    }

    #[test]
    fn the_guide_key_is_claimed_only_while_live_tv_is_in_front_and_opens_the_guide_over_it_and_another_app_ends_it() {
        let mut h = GuideHost::new(1060);
        assert!(h.takes().is_empty(), "nothing is in front yet: the key is LG's");
        h.update(&front("com.webos.app.home"), NOW);
        assert!(h.takes().is_empty() && h.guide_key(NOW + 10).is_empty() && !h.is_open(), "Home in front: the key is not claimed and opens nothing");
        h.update(&front(ch::LIVE_TV_APP), NOW + 500);
        assert_eq!(h.takes(), ["guide"]);
        let fx = h.guide_key(NOW + 1_000);
        assert!(h.is_open() && !fx.is_empty());
        assert!(h.takes().contains(&"guide") && h.takes().contains(&"chup"), "open: Guide stays claimed, so Live TV never launches LG's, and the rocker pages");
        assert!(h.guide_key(NOW + 1_100).is_empty() && h.is_open(), "the key while it is open does nothing");
        h.update(&front(ch::LIVE_TV_APP), NOW + 1_500);
        assert!(h.is_open(), "the app it came over is in front again: it stays");
        h.update(&front("com.beb.plxnative"), NOW + 2_000);
        assert!(!h.is_open() && h.takes().is_empty(), "another app ends it and the key is LG's again");
    }

    #[test]
    fn the_screen_going_off_ends_it_and_it_names_the_keys_it_wants_besides_the_arrows() {
        let mut h = GuideHost::new(1060);
        h.open(NOW);
        assert!(h.wants());
        assert_eq!(h.takes(), ["chup", "chdown", "info", "wheelup", "wheeldown", "guide"]);
        h.update(&Input::Topic { topic: "screen".into(), data: json!({"on": false}) }, NOW + 100);
        assert!(!h.is_open() && !h.wants());
    }

    #[test]
    fn with_a_fixture_the_calls_are_answered_a_moment_later_as_the_bridge_would() {
        let mut h = GuideHost::new(1060);
        h.with_fixture(None);
        let fx = h.open(NOW);
        assert!(!fx.iter().any(|e| matches!(e, Effect::Luna { uri, .. } if Fixture::stands_in_for(uri))), "the fixture took the calls of the services it stands in for");
        assert_eq!(h.model.rows(), 0);
        assert_eq!(h.due_at(), Some(NOW + LATENCY_MS));
        h.update(&Input::Tick, NOW + LATENCY_MS);
        assert!(h.model.rows() > 0, "the list came");
    }

    #[test]
    fn a_moving_grid_is_looked_at_again_until_it_has_stopped_and_a_resting_one_is_not() {
        use std::{cell::Cell, rc::Rc};
        const BASE: u64 = 5_000_000_000;
        let (mut h, mut kit) = crate::kit::testkit::kit_in(ui::lucent::palette::Mode::Dark, 1.0);
        h.settle_atlas();
        let clock = Rc::new(Cell::new(BASE));
        let read = Rc::clone(&clock);
        kit.ui.scene.set_clock(move || read.get(), 0);
        let mut engine = Engine::new(&kit).expect("an engine");
        let mut host = GuideHost::new(1060);
        host.with_fixture(None);
        host.open(NOW);
        // the host's loop: the clock moves on, the guide is ticked when it is due and drawn when it asked for it
        let (mut t, mut last_drawn) = (NOW, 0);
        for frame in 1..=500u64 {
            t += 8;
            clock.set(BASE + (t - NOW) * 1_000_000);
            if host.due_at().is_some_and(|due| due <= t) {
                host.update(&Input::Tick, t);
            }
            if host.take_redraw() {
                host.apply(&mut kit, &mut engine, clock.get()).expect("the guide is drawn");
                last_drawn = frame;
            }
        }
        let g = host.geometry(&engine);
        assert!(g["guide"]["rows"].as_array().unwrap().len() >= 7, "the rows on the screen are all made, not the first few: {g}");
        assert_eq!(g["guide"]["leadsWaiting"], 0, "what waited for the scroll to stop is in");
        assert!(last_drawn < 300, "it stopped asking for frames (the last at frame {last_drawn} of 500)");
        assert!(host.follow.is_none() && host.due_at().is_none_or(|due| due > t), "and has no more to look at");
    }
}
