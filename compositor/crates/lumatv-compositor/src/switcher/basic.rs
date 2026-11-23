//! The basic screen: what the task manager is when the shell cannot draw it, with the numbers and the watch on the shell that go with it.
//!
//! The shell draws the task manager. If it is not running, does not place the cards in time ([`LAYOUT_WAIT`](super)), stops answering the
//! numbers it is sent ([`BEAT_WAIT`]) or goes away, the compositor draws a plain one itself (`taskmgr/fallback.rs`): the same cards, the
//! numbers and the same actions, in the palette the shell last sent. The CPU paints a few bands of it over a flat dim and the renderer draws
//! the cards' pictures in their slots. It takes the keys itself, so that a hung shell cannot hold them. The shell is told (`mode` in the
//! `switcher` topic) and stops drawing; one that is up and silent is restarted once, because its last frame would stay over this screen.
//!
//! The numbers (memory, CPU per core, GPU, temperature) are read only while the task manager is open, once a second (`taskmgr/stats.rs`).
//! The same beat is the watch on the shell: it answers each `stats` topic with a `beat`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::{App, Curve, Phase, Picture, Pose, QUIT_ALPHA, SCRIM_MS};
use crate::input::code;
use crate::keys::Logical;
use crate::luna::ToBridge;
use crate::scene::{Overlay, PreviewSource, PreviewSpec};
use crate::taskmgr::Palette;
use crate::taskmgr::fallback::{CARD_RADIUS, Event, Item, Key, Mark, Screen, View};
use crate::taskmgr::stats::Sampler;
use crate::{Rect, State};

/// A `stats` topic the shell has not answered with a `beat` in this long: it is not drawing.
const BEAT_WAIT: Duration = Duration::from_millis(2500);
/// With no key for this long the basic screen closes itself: nobody is there, and a screen that stays wears the panel.
const IDLE: Duration = Duration::from_secs(45);
/// The dim behind the basic screen, over Home or an app: close to opaque, so that what is under it does not compete with the text.
const SCRIM_ALPHA: f32 = 0.96;
/// Presses of keys the basic screen took that have not been let go (bounded: a release that never comes does not grow it).
const DOWN_KEPT: usize = 16;

/// Which screen draws the task manager.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    /// The shell's.
    #[default]
    Ui,
    /// The compositor's own, this opening.
    Fallback,
}

impl Mode {
    pub(super) fn word(self) -> &'static str {
        match self {
            Mode::Ui => "ui",
            Mode::Fallback => "fallback",
        }
    }
}

#[derive(Default)]
pub(super) struct Basic {
    pub(super) mode: Mode,
    /// Why the compositor is drawing it, as a sentence.
    why: &'static str,
    /// Built when the basic screen is first needed and dropped with the opening: its typeface is not kept while nothing is open.
    screen: Option<Box<Screen>>,
    /// The shell's colours (`layout`): Rosé Pine until it says.
    pub(super) palette: Palette,
    /// The shell sent its palette: it draws, and it is expected to answer the beat.
    pub(super) armed: bool,
    overlays: Vec<Overlay>,
    rev: u64,
    last_key: Option<Instant>,
    /// Keys whose press the basic screen took: their release is its too, whatever has happened since.
    down: Vec<u32>,
    /// A line about what just failed, on the screen.
    pub(super) notice: Option<String>,
    ui_restarted: bool,
    pub(super) sampler: Sampler,
    stats_n: u64,
    /// The first `stats` topic the shell has not answered, and when it was sent.
    pending_beat: Option<(u64, Instant)>,
}

impl Basic {
    pub(super) fn begin(&mut self, now: Instant) {
        self.mode = Mode::Ui;
        self.why = "";
        self.palette = Palette::default();
        self.armed = false;
        self.overlays.clear();
        self.notice = None;
        self.ui_restarted = false;
        self.last_key = Some(now);
        self.stats_n = 0;
        self.pending_beat = None;
        self.sampler.start(now);
    }

    /// It stops being open: nothing is read from here on.
    pub(super) fn end(&mut self) {
        self.sampler.stop();
        self.pending_beat = None;
    }

    /// Everything is over.
    pub(super) fn finish(&mut self) {
        self.mode = Mode::Ui;
        self.screen = None;
        self.overlays.clear();
        self.notice = None;
    }
}

impl State {
    /// The compositor draws the task manager itself from here on (see the module). `restart`: a shell that is up is restarted, as its last
    /// frame would stay over this screen.
    pub(super) fn basic_enter(&mut self, why: &'static str, now: Instant, restart: bool) {
        if !self.switcher.accepting() || self.switcher.basic.mode == Mode::Fallback {
            return;
        }
        self.journal(format!("switcher: the basic screen takes over: {why}"));
        let sw = &mut self.switcher;
        let b = &mut sw.basic;
        b.mode = Mode::Fallback;
        b.why = why;
        b.overlays.clear();
        b.notice = None;
        b.pending_beat = None;
        b.last_key = Some(now);
        b.screen.get_or_insert_with(|| Box::new(Screen::new())).reset();
        let [r, g, bl] = b.palette.panel;
        // the shell's cards are of no use now, the basic screen has its own places; the dim moves to the palette's panel colour
        sw.scrim_rgb = [r as f32 / 255.0, g as f32 / 255.0, bl as f32 / 255.0];
        sw.cards.clear();
        sw.phase = Phase::Open;
        sw.until = None;
        if let Some(t) = &mut sw.scrim {
            t.retarget(now, Pose { alpha: SCRIM_ALPHA, ..Pose::new(Rect::default(), 0.0, 0.0) }, SCRIM_MS, Curve::Settle, true);
        }
        self.shell_sync_topics();
        if restart && self.shell_rust.connected && !self.switcher.basic.ui_restarted {
            self.switcher.basic.ui_restarted = true;
            self.shell_rust_restart();
        }
    }

    /// `ctl switcher basic`: the basic screen draws this opening and the shell is left as it is (the TV check of a screen that is meant for
    /// when the shell is not there).
    pub(crate) fn switcher_basic(&mut self, now: Instant) {
        self.basic_enter("Asked for over the control socket.", now, false);
    }

    /// A key event while the task manager is open or one of its presses is still down. `true` when the basic screen took it: while it is
    /// up it takes every key but Home, Volume and Mute. Every Back while it is open is told to the helper (`lumatv-sm`), whose rescue
    /// otherwise takes a Back held for 3 s with no answer for the compositor being gone.
    pub(crate) fn switcher_key(&mut self, code: u32, pressed: bool, first_press: bool, now: Instant) -> bool {
        if !pressed {
            let down = &mut self.switcher.basic.down;
            return down.iter().position(|c| *c == code).map(|i| down.swap_remove(i)).is_some();
        }
        if !self.switcher.accepting() {
            return false;
        }
        self.switcher.basic.last_key = Some(now);
        let logical = self.keys.router.table.classify(code);
        if first_press && logical == Logical::Back {
            self.luna.send(ToBridge::Taskmgr { open: true });
        }
        if self.switcher.basic.mode != Mode::Fallback
            || matches!(code, code::KEY_HOME | code::KEY_HOMEPAGE | code::KEY_RF_HOME | code::KEY_VOLUMEUP | code::KEY_VOLUMEDOWN | code::KEY_MUTE)
        {
            return false;
        }
        if first_press {
            let down = &mut self.switcher.basic.down;
            if down.len() >= DOWN_KEPT {
                down.remove(0);
            }
            down.push(code);
        }
        let key = match code {
            code::KEY_LEFT => Key::Left,
            code::KEY_RIGHT => Key::Right,
            code::KEY_UP => Key::Up,
            code::KEY_DOWN => Key::Down,
            code::KEY_ENTER | code::KEY_OK => Key::Ok,
            _ if matches!(logical, Logical::Back | Logical::Exit) => Key::Back,
            _ => return true,
        };
        // Only the arrows repeat; OK and Back act once per press.
        if !first_press && matches!(key, Key::Ok | Key::Back) {
            return true;
        }
        let items = self.basic_items(&self.switcher_apps());
        let refresher = self.switcher.basic.sampler.stats.refresher;
        let Some(screen) = self.switcher.basic.screen.as_mut() else { return true };
        let event = screen.key(key, &items, refresher);
        self.basic_event(event, now);
        if self.output.draws_on_damage(now) {
            self.tick();
        }
        true
    }

    fn basic_event(&mut self, event: Event, now: Instant) {
        match event {
            Event::Nothing | Event::Redraw => {}
            Event::Close => self.switcher_close("Back", now),
            Event::Activate(id) => {
                self.journal(format!("switcher: {id} chosen, launching it"));
                self.luna_call("luna://com.webos.applicationManager/launch".into(), json!({"id": id}));
                self.switcher_close("chosen", now);
            }
            Event::Quit { id, force } => self.switcher_quit(&id, force, now),
            Event::QuitAll => self.switcher_quit_all(now),
            Event::System(action) => {
                if let Err(why) = self.system_action(action, now) {
                    self.switcher.basic.notice = Some(
                        match why {
                            "refresher" => "Restart TV waits for the panel's pixel refresher to finish.",
                            "busy" => "Another action is already running.",
                            _ => "That did not start.",
                        }
                        .to_string(),
                    );
                }
            }
        }
    }

    pub(super) fn basic_items(&self, apps: &[App]) -> Vec<Item> {
        apps.iter()
            .map(|a| {
                let mark = if self.switcher.quitting.iter().any(|(q, _)| *q == a.id) {
                    Mark::Quitting
                } else if self.switcher.stuck.contains(&a.id) {
                    Mark::Stuck
                } else {
                    Mark::Nothing
                };
                Item { id: a.id.clone(), title: a.title.clone(), picture: a.picture, mark, hung: self.switcher.list.is_hung(&a.id) }
            })
            .collect()
    }

    /// This frame of the basic screen: the pictures for the card slots, and the bands that changed since the last (a band is drawn again from
    /// the scene's copy of it while it stays).
    pub(super) fn basic_frame(&mut self, apps: &[App]) -> (Vec<PreviewSpec>, Vec<Overlay>) {
        let items = self.basic_items(apps);
        let working = self.switcher.system.as_ref().map(|g| g.action);
        let b = &mut self.switcher.basic;
        let screen = b.screen.get_or_insert_with(|| Box::new(Screen::new()));
        let stats = b.sampler.ready().then_some(&b.sampler.stats);
        let view = View { items: &items, stats, palette: &b.palette, why: b.why, notice: b.notice.as_deref(), refresher: b.sampler.stats.refresher, working };
        for band in screen.render(&view) {
            b.overlays.retain(|o| o.key != band.key);
            if !band.rect.is_empty() {
                b.rev += 1;
                b.overlays.push(Overlay { key: band.key, rev: b.rev, rect: band.rect, rgba: Arc::new(band.rgba) });
            }
        }
        let previews = screen
            .slots(&items)
            .into_iter()
            .filter_map(|(i, rect)| {
                let (app, item) = (apps.get(i)?, items.get(i)?);
                let source = match (app.picture, app.surface) {
                    (Picture::Live, Some(surface)) => PreviewSource::Window(surface),
                    (Picture::Thumb, _) => PreviewSource::Thumb(app.id.clone()),
                    _ => return None,
                };
                Some(PreviewSpec { source, rect, alpha: if item.mark == Mark::Quitting { QUIT_ALPHA } else { 1.0 }, radius: CARD_RADIUS })
            })
            .collect();
        (previews, b.overlays.clone())
    }

    /// While it is open, once a second: reads the numbers and hands them to the shell, watches that the shell answers, and closes the basic
    /// screen when nobody is there. Whether the topics changed.
    pub(super) fn switcher_numbers(&mut self, now: Instant) -> bool {
        if !self.switcher.accepting() {
            return false;
        }
        let mut changed = false;
        if self.switcher.basic.sampler.due(now) {
            self.switcher.basic.sampler.sample(now);
            // a process that stops, or ends, is noticed on the same beat
            changed |= self.switcher.list.refresh_hung();
            let b = &mut self.switcher.basic;
            if b.sampler.ready() && b.mode == Mode::Ui {
                b.stats_n += 1;
                let n = b.stats_n;
                let stats = b.sampler.stats.json(n);
                if b.armed && b.pending_beat.is_none() {
                    b.pending_beat = Some((n, now));
                }
                self.shell_rust.push_if_changed("stats", stats);
            }
        }
        let b = &self.switcher.basic;
        if b.mode == Mode::Ui && b.pending_beat.is_some_and(|(_, at)| now.saturating_duration_since(at) >= BEAT_WAIT) {
            self.basic_enter("LumaTV's interface stopped answering.", now, true);
        } else if b.mode == Mode::Fallback && b.last_key.is_some_and(|t| now.saturating_duration_since(t) >= IDLE) {
            self.switcher_close("idle", now);
        }
        changed
    }

    /// The shell answers a `stats` topic: it is drawing.
    pub(super) fn switcher_beat(&mut self, n: u64) {
        let b = &mut self.switcher.basic;
        if b.pending_beat.is_some_and(|(sent, _)| n >= sent) {
            b.pending_beat = None;
        }
    }
}
