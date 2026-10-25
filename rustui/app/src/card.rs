//! The card app's rules, with no window and no GPU in them: what Back, OK, the compositor's state changes and SAM's `relaunch` and
//! `close` do. The window and the toolkit's scene (`card_ui`, `bin/card.rs`) carry the decisions out, so the rules are tested here
//! without either.
//!
//! What an LG card app does (docs/research/cardapp-result.md, "What the set does"):
//!
//! - Home is never the app's: the compositor sends `launchDefaultApp`, Home comes to the front and the app is told it is minimized.
//!   A minimized app draws nothing: LG's compositor need not answer a frame of a card it hides.
//! - Back is the app's, because the window sets `_WEBOS_ACCESS_POLICY_KEYS_BACK`: it goes up a page, and at the root it ends the app,
//!   which is what LG's own close does (the card goes, the one below comes back).
//! - SAM's `relaunch` of a running app brings it back to the front by asking for fullscreen, as WAM does for a web app; a launch
//!   that is a preload (`reason: preload`, `parameters.snapshot`) shows no window until then.
//! - `close`, from SAM or from the compositor, ends the app.

use crate::home::Key;
use crate::lifecycle::FromApp;
use std::collections::VecDeque;

pub const DEFAULT: u32 = 0;
pub const MINIMIZED: u32 = 1;
pub const MAXIMIZED: u32 = 2;
pub const FULLSCREEN: u32 = 3;

/// How many keys the Keys page remembers.
pub const KEY_LINES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Status,
    Keys,
}

impl Page {
    pub fn name(self) -> &'static str {
        match self {
            Page::Status => "status",
            Page::Keys => "keys",
        }
    }
}

/// The two buttons of the status page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Keys,
    Close,
}

/// Whether the app shows anything: not yet (a preload), now, or not while another card covers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Hidden,
    Visible,
    Covered,
}

/// Why the app ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    /// Back on the first page.
    Back,
    /// OK on the Close button.
    Button,
    /// SAM's `close`, with its reason.
    Sam(String),
    /// `wl_webos_shell_surface.close`: LG's compositor, or LumaTV's when a window does not take Back.
    Compositor,
    /// The launcher is gone.
    Launcher,
}

impl Exit {
    pub fn why(&self) -> String {
        match self {
            Exit::Back => "Back at the first page".into(),
            Exit::Button => "the Close button".into(),
            Exit::Sam(reason) => format!("SAM closed the app ({reason})"),
            Exit::Compositor => "the compositor closed the window".into(),
            Exit::Launcher => "the launcher is gone".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Key {
        code: u32,
        pressed: bool,
    },
    /// `wl_webos_shell_surface.state_changed`.
    State(u32),
    CompositorClose,
    Relaunch {
        reason: String,
        parameters: String,
    },
    SamClose {
        reason: String,
    },
    Sam {
        registered: bool,
    },
    LauncherGone,
}

/// What the loop has to do after an input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Out {
    /// Something the page shows changed.
    pub redraw: bool,
    /// Ask the compositor for this window state (`set_state`).
    pub request_state: Option<u32>,
    pub exit: Option<Exit>,
    /// For the launcher's log.
    pub report: Vec<FromApp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyLine {
    pub code: u32,
    pub pressed: bool,
}

pub struct Card {
    pub page: Page,
    pub focus: Button,
    pub phase: Phase,
    /// The compositor's last word on the window; `None` before it has said any.
    pub state: Option<u32>,
    /// The launch and every relaunch since.
    pub launches: u32,
    /// What the latest launch said, for the page: its reason.
    pub launch_reason: String,
    /// Whether SAM took the launcher's registration; `None` when the app has no launcher.
    pub sam: Option<bool>,
    /// Newest first.
    pub keys: VecDeque<KeyLine>,
}

/// The remote's keys as the card reads them: the arrows, OK and Back, in the codes the compositor sends (Linux input codes). The
/// remote's Back is KEY_PREVIOUS (412) on this set; KEY_BACK, Escape and Backspace are a keyboard's.
pub fn key_of(code: u32) -> Option<Key> {
    Some(match code {
        103 => Key::Up,
        108 => Key::Down,
        105 => Key::Left,
        106 => Key::Right,
        28 | 352 | 353 => Key::Ok,
        412 | 158 | 1 | 14 => Key::Back,
        _ => return None,
    })
}

pub fn key_name(code: u32) -> &'static str {
    match key_of(code) {
        Some(Key::Up) => "Up",
        Some(Key::Down) => "Down",
        Some(Key::Left) => "Left",
        Some(Key::Right) => "Right",
        Some(Key::Ok) => "OK",
        Some(Key::Back) => "Back",
        None => "Key",
    }
}

impl Card {
    /// `hidden`: SAM started the app for a preload; it shows nothing until it is relaunched.
    pub fn new(hidden: bool, reason: &str, has_launcher: bool) -> Card {
        Card {
            page: Page::Status,
            focus: Button::Keys,
            phase: if hidden { Phase::Hidden } else { Phase::Visible },
            state: None,
            launches: 1,
            launch_reason: reason.to_owned(),
            sam: has_launcher.then_some(false),
            keys: VecDeque::new(),
        }
    }

    /// Frames are drawn and presented only while visible.
    pub fn wants_frames(&self) -> bool {
        self.phase == Phase::Visible
    }

    pub fn handle(&mut self, input: Input) -> Out {
        let mut out = Out::default();
        match input {
            Input::Key { code, pressed } => {
                self.keys.push_front(KeyLine { code, pressed });
                self.keys.truncate(KEY_LINES);
                out.redraw = true;
                if pressed {
                    self.press(code, &mut out);
                }
            }
            Input::State(state) => {
                self.state = Some(state);
                out.report.push(FromApp::State(state));
                out.redraw = true;
                match (state, self.phase) {
                    (MINIMIZED, _) => self.phase = Phase::Covered,
                    (FULLSCREEN | MAXIMIZED, Phase::Hidden | Phase::Covered) => self.phase = Phase::Visible,
                    (DEFAULT, Phase::Covered) => self.phase = Phase::Visible,
                    _ => {}
                }
            }
            Input::Relaunch { reason, parameters: _ } => {
                self.launches += 1;
                self.launch_reason = reason;
                out.redraw = true;
                if self.phase != Phase::Visible {
                    // A relaunch is the user asking for the app. Asking for fullscreen is what brings a covered card back (and
                    // is how SAM learns it is in front); the card is shown from here on, and the compositor's word corrects it.
                    self.phase = Phase::Visible;
                    out.request_state = Some(FULLSCREEN);
                }
            }
            Input::SamClose { reason } => out.exit = Some(Exit::Sam(reason)),
            Input::CompositorClose => out.exit = Some(Exit::Compositor),
            Input::Sam { registered } => {
                self.sam = Some(registered);
                out.redraw = true;
            }
            Input::LauncherGone => out.exit = Some(Exit::Launcher),
        }
        out
    }

    fn press(&mut self, code: u32, out: &mut Out) {
        let Some(key) = key_of(code) else { return };
        match (self.page, key) {
            (Page::Keys, Key::Back | Key::Ok) => self.go(Page::Status, out),
            (Page::Status, Key::Back) => out.exit = Some(Exit::Back),
            (Page::Status, Key::Left | Key::Right) => self.focus = if self.focus == Button::Keys { Button::Close } else { Button::Keys },
            (Page::Status, Key::Ok) => match self.focus {
                Button::Keys => self.go(Page::Keys, out),
                Button::Close => out.exit = Some(Exit::Button),
            },
            _ => {}
        }
    }

    fn go(&mut self, page: Page, out: &mut Out) {
        self.page = page;
        out.report.push(FromApp::Page(page.name().into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BACK: u32 = 412;
    const OK: u32 = 28;
    const RIGHT: u32 = 106;

    fn tap(card: &mut Card, code: u32) -> Out {
        let down = card.handle(Input::Key { code, pressed: true });
        let up = card.handle(Input::Key { code, pressed: false });
        assert!(up.exit.is_none() && up.request_state.is_none(), "a release does nothing");
        down
    }

    #[test]
    fn back_goes_up_a_page_and_ends_the_app_at_the_first() {
        let mut card = Card::new(false, "undefined", true);
        assert_eq!(tap(&mut card, OK).report, vec![FromApp::Page("keys".into())]);
        assert_eq!(card.page, Page::Keys);
        let up = tap(&mut card, BACK);
        assert_eq!((card.page, up.exit), (Page::Status, None), "Back on the Keys page returns to the first");
        assert_eq!(up.report, vec![FromApp::Page("status".into())]);
        assert_eq!(tap(&mut card, BACK).exit, Some(Exit::Back), "Back on the first page ends the app");
    }

    #[test]
    fn the_remotes_back_is_412_and_a_keyboards_is_158_escape_or_backspace() {
        for code in [412, 158, 1, 14] {
            assert_eq!(key_of(code), Some(Key::Back), "{code}");
            assert_eq!(tap(&mut Card::new(false, "undefined", false), code).exit, Some(Exit::Back), "{code}");
        }
        assert_eq!(key_of(102), None, "Home is the compositor's: the card does not read it");
        assert_eq!(tap(&mut Card::new(false, "undefined", false), 102).exit, None);
    }

    #[test]
    fn focus_moves_between_the_buttons_and_ok_presses_the_focused_one() {
        let mut card = Card::new(false, "undefined", false);
        assert_eq!(card.focus, Button::Keys);
        tap(&mut card, RIGHT);
        assert_eq!(card.focus, Button::Close);
        tap(&mut card, 105);
        assert_eq!(card.focus, Button::Keys);
        tap(&mut card, 106);
        assert_eq!(tap(&mut card, OK).exit, Some(Exit::Button));
    }

    #[test]
    fn a_minimized_card_draws_nothing_and_a_fullscreen_one_draws_again() {
        let mut card = Card::new(false, "undefined", true);
        assert!(card.wants_frames());
        assert!(card.handle(Input::State(FULLSCREEN)).redraw);
        let out = card.handle(Input::State(MINIMIZED));
        assert_eq!((card.phase, card.wants_frames(), out.report), (Phase::Covered, false, vec![FromApp::State(MINIMIZED)]));
        assert!(card.handle(Input::State(FULLSCREEN)).redraw);
        assert_eq!((card.phase, card.wants_frames()), (Phase::Visible, true));
    }

    #[test]
    fn a_relaunch_of_a_covered_card_asks_for_fullscreen_and_one_of_a_visible_card_asks_for_nothing() {
        let mut card = Card::new(false, "undefined", true);
        card.handle(Input::State(MINIMIZED));
        let out = card.handle(Input::Relaunch { reason: "undefined".into(), parameters: r#"{"launchType":"homeKey"}"#.into() });
        assert_eq!((out.request_state, card.phase, card.launches), (Some(FULLSCREEN), Phase::Visible, 2));
        let out = card.handle(Input::Relaunch { reason: "undefined".into(), parameters: "{}".into() });
        assert_eq!((out.request_state, card.launches), (None, 3), "already in front: nothing to ask");
    }

    #[test]
    fn a_preloaded_app_shows_nothing_until_it_is_relaunched() {
        let mut card = Card::new(true, "preload", true);
        assert!(!card.wants_frames());
        // The compositor's `default` for a window that was never raised changes nothing.
        card.handle(Input::State(DEFAULT));
        assert_eq!(card.phase, Phase::Hidden);
        let out = card.handle(Input::Relaunch { reason: "undefined".into(), parameters: "{}".into() });
        assert_eq!((out.request_state, card.phase, card.wants_frames()), (Some(FULLSCREEN), Phase::Visible, true));
    }

    #[test]
    fn close_from_sam_the_compositor_or_a_lost_launcher_ends_the_app() {
        let mut card = Card::new(false, "undefined", true);
        assert_eq!(card.handle(Input::SamClose { reason: "memoryReclaim".into() }).exit, Some(Exit::Sam("memoryReclaim".into())));
        assert_eq!(card.handle(Input::CompositorClose).exit, Some(Exit::Compositor));
        assert_eq!(card.handle(Input::LauncherGone).exit, Some(Exit::Launcher));
        assert_eq!(Exit::Sam("powerOff".into()).why(), "SAM closed the app (powerOff)");
    }

    #[test]
    fn the_key_list_keeps_the_newest_eight_and_the_registration_shows() {
        let mut card = Card::new(false, "undefined", true);
        assert_eq!(card.sam, Some(false));
        card.handle(Input::Sam { registered: true });
        assert_eq!(card.sam, Some(true));
        for i in 0..12 {
            card.handle(Input::Key { code: 200 + i, pressed: true });
        }
        assert_eq!(card.keys.len(), KEY_LINES);
        assert_eq!(card.keys.front().map(|k| k.code), Some(211));
        assert_eq!(Card::new(false, "undefined", false).sam, None);
    }
}
