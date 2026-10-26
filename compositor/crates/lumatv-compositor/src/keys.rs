//! The remote's key filters, moved out of LG's compositor (`KeyFilters/*.js`; what each does is read in
//! `docs/research/surfaces/KeyEntryPoints.json`) into the routing, without drawing anything: which key is which, what a tap, a
//! hold and three taps ask for. State only, time is an argument: `input.rs` feeds it the remote's events and the tick, and
//! `system_ui.rs` carries out what it asks for, so every decision here is a host test.
//!
//! LG's chain swallows the gear, Input and the app keys and decides at the release (a tap) or at 1 s (a hold, converted to
//! another key); the digits and Back reach the app at once and a 1 s hold acts besides; Mute is a toggle at its release unless
//! it was held 2 s. The same shapes here, on every device (LG converts holds for its own remotes only, which is all this set has).

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::input::code::*;

/// How long the gear, Input, a digit or Back is held for LG's long-press conversion (`handleLongPressKeys`, 1000 ms).
pub const HOLD: Duration = Duration::from_millis(1000);
/// Mute held for the accessibility panel (`handleAccessibility`, 2000 ms).
pub const HOLD_MUTE: Duration = Duration::from_millis(2000);
/// Three gear taps within this are the hidden AV-reset menu (`handleAvReset`).
pub const AV_RESET_WINDOW: Duration = Duration::from_secs(3);
const AV_RESET_TAPS: usize = 3;

/// The remote's four app buttons. The id is what Settings > Remote stores them under (`settings/AppKeys.js`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppKey {
    Netflix,
    Amazon,
    DisneyPlus,
    LgChannels,
}

impl AppKey {
    pub const ALL: [AppKey; 4] = [AppKey::Netflix, AppKey::Amazon, AppKey::DisneyPlus, AppKey::LgChannels];

    pub fn id(self) -> &'static str {
        match self {
            AppKey::Netflix => "netflix",
            AppKey::Amazon => "amazon",
            AppKey::DisneyPlus => "disneyplus",
            AppKey::LgChannels => "lgchannels",
        }
    }

    /// The app LG's own chain launches for the button (`handleCPHotkeys`).
    pub fn lg_app(self) -> &'static str {
        match self {
            AppKey::Netflix => "netflix",
            AppKey::Amazon => "amazon",
            AppKey::DisneyPlus => "com.disney.disneyplus-prod",
            AppKey::LgChannels => "com.webos.app.lgchannels",
        }
    }
}

/// What a key is to the routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Logical {
    Gear,
    Input,
    App(AppKey),
    /// 0 to 9.
    Digit(u8),
    Mute,
    Back,
    Exit,
    Other,
}

/// Which evdev code is which key. The built-in codes are LG's (`tools`: network-input-service's key table; the gear and Input were
/// also read off this set); a `keys.conf` line `NAME CODE` adds a code for a key (gear, input, netflix, amazon, disneyplus,
/// lgchannels), which is how a button this table does not know yet is taught without a rebuild.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyTable {
    extra: Vec<(u32, Logical)>,
}

impl KeyTable {
    pub fn classify(&self, code: u32) -> Logical {
        if let Some((_, l)) = self.extra.iter().find(|(c, _)| *c == code) {
            return *l;
        }
        match code {
            KEY_MENU => Logical::Gear,
            KEY_VIDEO_NEXT => Logical::Input,
            KEY_NETFLIX => Logical::App(AppKey::Netflix),
            KEY_AMAZON => Logical::App(AppKey::Amazon),
            KEY_MUTE => Logical::Mute,
            KEY_BACK | KEY_PREVIOUS => Logical::Back,
            KEY_EXIT => Logical::Exit,
            KEY_1..=KEY_0 => Logical::Digit(if code == KEY_0 { 0 } else { (code - KEY_1 + 1) as u8 }),
            _ => Logical::Other,
        }
    }

    /// Whether the table names this code, built in or taught: what the routing treats as a key of its own. A code it does not
    /// name goes to the app as before.
    pub fn knows(&self, code: u32) -> bool {
        self.classify(code) != Logical::Other
    }

    /// Parses `keys.conf`: `NAME CODE` per line, `#` comments. Every line that is not one is returned with the reason.
    pub fn parse(text: &str) -> (KeyTable, Vec<String>) {
        let mut table = KeyTable::default();
        let mut bad = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut words = line.split_whitespace();
            let (name, code) = (words.next().unwrap_or(""), words.next().and_then(|c| c.parse::<u32>().ok()));
            let logical = match name {
                "gear" => Some(Logical::Gear),
                "input" => Some(Logical::Input),
                "netflix" => Some(Logical::App(AppKey::Netflix)),
                "amazon" => Some(Logical::App(AppKey::Amazon)),
                "disneyplus" => Some(Logical::App(AppKey::DisneyPlus)),
                "lgchannels" => Some(Logical::App(AppKey::LgChannels)),
                _ => None,
            };
            match (logical, code, words.next()) {
                (Some(l), Some(c), None) => table.extra.push((c, l)),
                _ => bad.push(format!("line {}: {line:?} is not NAME CODE with a name of gear, input, netflix, amazon, disneyplus or lgchannels", n + 1)),
            }
        }
        (table, bad)
    }

    /// The table as `NAME CODE` pairs, for the control socket.
    pub fn named(&self) -> Vec<(String, u32)> {
        let name = |l: Logical| match l {
            Logical::Gear => "gear",
            Logical::Input => "input",
            Logical::App(k) => k.id(),
            _ => "",
        };
        let mut out: Vec<(String, u32)> = [Logical::Gear, Logical::Input, Logical::App(AppKey::Netflix), Logical::App(AppKey::Amazon)]
            .into_iter()
            .zip([KEY_MENU, KEY_VIDEO_NEXT, KEY_NETFLIX, KEY_AMAZON])
            .map(|(l, c)| (name(l).to_string(), c))
            .collect();
        out.extend(self.extra.iter().map(|(c, l)| (name(*l).to_string(), *c)));
        out
    }
}

/// Settings > Remote's choice for the app buttons: the theme's `remote.appKeys`, `{netflix: "<app id>", amazon: "", ...}`. A button
/// the owner never set is absent and means LG's own target; `""` means nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppKeyConfig {
    chosen: Vec<(AppKey, String)>,
}

/// What an app button does now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppKeyDecision {
    Launch(String),
    /// Taken and nothing done: the owner chose nothing, or the app it opens is already in front.
    Nothing,
    /// LG's own button on its own app, which is in front: the app gets the key.
    Pass,
}

impl AppKeyConfig {
    /// What the owner chose, as (button id, app id), for the control socket.
    pub fn chosen(&self) -> Vec<(&'static str, &str)> {
        self.chosen.iter().map(|(k, app)| (k.id(), app.as_str())).collect()
    }

    pub fn from_theme(theme: &Value) -> AppKeyConfig {
        let chosen = AppKey::ALL
            .into_iter()
            .filter_map(|k| theme.pointer("/remote/appKeys").and_then(|a| a.get(k.id())).and_then(Value::as_str).map(|s| (k, s.to_string())))
            .collect();
        AppKeyConfig { chosen }
    }

    /// The app LumaHome's `appKey()` would open for the button over `foreground` (QML: `LumaHome.qml`, "the remote's app keys"): an app
    /// that is not installed is the app manager's to refuse, and the plain launch never starts LG's auto-install.
    pub fn decide(&self, key: AppKey, foreground: &str) -> AppKeyDecision {
        match self.chosen.iter().find(|(k, _)| *k == key) {
            None if foreground == key.lg_app() => AppKeyDecision::Pass,
            None => AppKeyDecision::Launch(key.lg_app().to_string()),
            Some((_, app)) if app.is_empty() || app == foreground => AppKeyDecision::Nothing,
            Some((_, app)) => AppKeyDecision::Launch(app.clone()),
        }
    }
}

/// What a key asks of the system layer. LG's key chain opens these as windows of its own compositor (`systemUIManager/launch`);
/// they are asked of `State::system_request`, which is where LumaTV's own UI takes them once it exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemRequest {
    /// The gear's tap: Quick Settings (`com.webos.app.quicksettings`).
    QuickSettings,
    /// The Input key's tap: the input picker (`com.webos.app.quickinputpicker`).
    InputPicker,
    /// The Input key held: the Home Hub (`com.webos.app.homeconnect`).
    InputHub,
    /// The gear held: LG's Settings app, closed when it is already in front (LG's `Key_Option`).
    Settings,
    /// Three gear taps: the hidden AV-reset menu (`tvhotkey`, `mute-hidden-action`).
    AvReset,
    /// Mute held: the accessibility quick panel.
    Accessibility,
    /// A digit held: Quick Access slot 1 to 9; 0 is the editor.
    QuickAccess(u8),
    /// An app button: launch this app.
    LaunchApp(String),
    /// Exit, or Back held: the foreground app closes (the Home app stays).
    Exit,
    /// Exit while the keyboard is up: the keyboard closes first (LG's order).
    CloseKeyboard,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Request(SystemRequest),
    /// Give the app the release of a key it got the press of: its hold fired and the real release will be swallowed.
    ClientRelease(u32),
}

/// The routing's answer to one event.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The event goes no further: no app gets it and the mask and policy routing is skipped.
    pub swallow: bool,
    pub actions: Vec<Action>,
}

/// What the routing needs to know about the screen when a key arrives.
pub struct Ctx<'a> {
    /// The foreground card's app id, empty when none.
    pub foreground: &'a str,
    /// A keyboard window is mapped.
    pub keyboard_up: bool,
    pub app_keys: &'a AppKeyConfig,
}

struct Held {
    code: u32,
    logical: Logical,
    since: Instant,
    /// The press was taken here, so its repeats and release are too.
    swallowed: bool,
    /// The hold's action ran: the release is swallowed.
    fired: bool,
}

#[derive(Default)]
pub struct Router {
    pub table: KeyTable,
    down: Vec<Held>,
    gear_taps: Vec<Instant>,
}

impl Router {
    pub fn new(table: KeyTable) -> Router {
        Router { table, ..Router::default() }
    }

    /// One press, repeat or release.
    pub fn event(&mut self, code: u32, pressed: bool, now: Instant, ctx: &Ctx) -> Outcome {
        if pressed {
            if let Some(h) = self.down.iter().find(|h| h.code == code) {
                return Outcome { swallow: h.swallowed || h.fired, actions: Vec::new() };
            }
            let logical = self.table.classify(code);
            let mut out = Outcome::default();
            match logical {
                Logical::Gear | Logical::Input => out.swallow = true,
                Logical::App(k) => match ctx.app_keys.decide(k, ctx.foreground) {
                    AppKeyDecision::Launch(app) => {
                        out.swallow = true;
                        out.actions.push(Action::Request(SystemRequest::LaunchApp(app)));
                    }
                    AppKeyDecision::Nothing => out.swallow = true,
                    AppKeyDecision::Pass => {}
                },
                Logical::Exit if ctx.keyboard_up => {
                    out.swallow = true;
                    out.actions.push(Action::Request(SystemRequest::CloseKeyboard));
                }
                _ => {}
            }
            self.down.push(Held { code, logical, since: now, swallowed: out.swallow, fired: false });
            return out;
        }
        let Some(i) = self.down.iter().position(|h| h.code == code) else { return Outcome::default() };
        let h = self.down.swap_remove(i);
        let mut out = Outcome { swallow: h.swallowed || h.fired, actions: Vec::new() };
        if !h.fired {
            match h.logical {
                Logical::Gear => out.actions.push(Action::Request(self.gear_tap(now))),
                Logical::Input => out.actions.push(Action::Request(SystemRequest::InputPicker)),
                _ => {}
            }
        }
        out
    }

    /// Whether a key with a hold is down and has not fired it: the tick has something to measure.
    pub fn waiting(&self) -> bool {
        self.down.iter().any(|h| !h.fired && matches!(h.logical, Logical::Gear | Logical::Input | Logical::Digit(_) | Logical::Back | Logical::Mute))
    }

    /// The holds that came due since the last call, each once. Called every tick.
    pub fn poll(&mut self, now: Instant, ctx: &Ctx) -> Vec<Action> {
        let mut out = Vec::new();
        for h in self.down.iter_mut().filter(|h| !h.fired) {
            let due = match h.logical {
                Logical::Gear | Logical::Input | Logical::Digit(_) | Logical::Back => HOLD,
                Logical::Mute => HOLD_MUTE,
                _ => continue,
            };
            if now.saturating_duration_since(h.since) < due {
                continue;
            }
            match h.logical {
                Logical::Gear => out.push(Action::Request(SystemRequest::Settings)),
                Logical::Input => out.push(Action::Request(SystemRequest::InputHub)),
                // Quick Access is not asked of a text field's keyboard: its digits are the field's (LG skips it while the keyboard shows).
                Logical::Digit(_) if ctx.keyboard_up => continue,
                Logical::Digit(n) => {
                    out.push(Action::Request(SystemRequest::QuickAccess(n)));
                    out.push(Action::ClientRelease(h.code));
                }
                Logical::Back => {
                    out.push(Action::Request(if ctx.keyboard_up { SystemRequest::CloseKeyboard } else { SystemRequest::Exit }));
                    out.push(Action::ClientRelease(h.code));
                }
                Logical::Mute => out.push(Action::Request(SystemRequest::Accessibility)),
                _ => continue,
            }
            h.fired = true;
        }
        out
    }

    fn gear_tap(&mut self, now: Instant) -> SystemRequest {
        self.gear_taps.retain(|t| now.saturating_duration_since(*t) <= AV_RESET_WINDOW);
        self.gear_taps.push(now);
        if self.gear_taps.len() >= AV_RESET_TAPS {
            self.gear_taps.clear();
            SystemRequest::AvReset
        } else {
            SystemRequest::QuickSettings
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SystemRequest::*;

    fn req(r: SystemRequest) -> Action {
        Action::Request(r)
    }

    struct Rig {
        router: Router,
        now: Instant,
        foreground: String,
        keyboard_up: bool,
        app_keys: AppKeyConfig,
    }

    impl Rig {
        fn new() -> Rig {
            Rig { router: Router::new(KeyTable::default()), now: Instant::now(), foreground: "com.webos.app.home".into(), keyboard_up: false, app_keys: AppKeyConfig::default() }
        }

        fn at(&mut self, ms: u64) {
            self.now += Duration::from_millis(ms);
        }

        fn key(&mut self, code: u32, pressed: bool) -> Outcome {
            let ctx = Ctx { foreground: &self.foreground, keyboard_up: self.keyboard_up, app_keys: &self.app_keys };
            self.router.event(code, pressed, self.now, &ctx)
        }

        fn poll(&mut self) -> Vec<Action> {
            let ctx = Ctx { foreground: &self.foreground, keyboard_up: self.keyboard_up, app_keys: &self.app_keys };
            self.router.poll(self.now, &ctx)
        }

        /// Press, hold for `ms` (polling at the end), release: every outcome in order.
        fn hold(&mut self, code: u32, ms: u64) -> (Outcome, Vec<Action>, Outcome) {
            let press = self.key(code, true);
            self.at(ms);
            let polled = self.poll();
            let release = self.key(code, false);
            (press, polled, release)
        }
    }

    #[test]
    fn the_codes_are_named_and_a_taught_one_is_added() {
        let t = KeyTable::default();
        assert_eq!(t.classify(139), Logical::Gear);
        assert_eq!(t.classify(241), Logical::Input);
        assert_eq!(t.classify(1037), Logical::App(AppKey::Netflix));
        assert_eq!(t.classify(1038), Logical::App(AppKey::Amazon));
        assert_eq!(t.classify(1041), Logical::Other, "Disney+ and LG Channels have no code until one is taught");
        assert_eq!((t.classify(2), t.classify(10), t.classify(11)), (Logical::Digit(1), Logical::Digit(9), Logical::Digit(0)));
        assert_eq!((t.classify(113), t.classify(412), t.classify(158), t.classify(174)), (Logical::Mute, Logical::Back, Logical::Back, Logical::Exit));
        let (t, bad) = KeyTable::parse("# the remote's buttons\ndisneyplus 1041\nlgchannels 1042 # LG Channels\n\ngear 140\n");
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!((t.classify(1041), t.classify(1042), t.classify(140), t.classify(139)), (Logical::App(AppKey::DisneyPlus), Logical::App(AppKey::LgChannels), Logical::Gear, Logical::Gear));
        let (_, bad) = KeyTable::parse("volume 5\ngear\ngear 1 2\ngear x\nnetflix 1037\n");
        assert_eq!(bad.len(), 4, "{bad:?}");
    }

    #[test]
    fn a_gear_tap_is_taken_whole_and_asks_for_quick_settings_at_its_release() {
        let mut r = Rig::new();
        let press = r.key(KEY_MENU, true);
        assert!(press.swallow && press.actions.is_empty(), "the press is LG's chain's to swallow and decide at the release");
        r.at(40);
        assert!(r.key(KEY_MENU, true).swallow, "a repeat is swallowed too");
        r.at(100);
        assert!(r.poll().is_empty());
        let release = r.key(KEY_MENU, false);
        assert!(release.swallow);
        assert_eq!(release.actions, vec![req(QuickSettings)]);
    }

    #[test]
    fn three_gear_taps_in_three_seconds_are_the_av_reset_and_the_first_two_are_not() {
        let mut r = Rig::new();
        let mut asked = Vec::new();
        for _ in 0..3 {
            r.key(KEY_MENU, true);
            r.at(80);
            asked.extend(r.key(KEY_MENU, false).actions);
            r.at(900);
        }
        assert_eq!(asked, vec![req(QuickSettings), req(QuickSettings), req(AvReset)]);
        // The count starts again: the next tap is a first one.
        r.key(KEY_MENU, true);
        r.at(80);
        assert_eq!(r.key(KEY_MENU, false).actions, vec![req(QuickSettings)]);
    }

    #[test]
    fn gear_taps_further_apart_than_three_seconds_never_reach_the_av_reset() {
        let mut r = Rig::new();
        let mut asked = Vec::new();
        for _ in 0..5 {
            r.key(KEY_MENU, true);
            r.at(50);
            asked.extend(r.key(KEY_MENU, false).actions);
            r.at(1600);
        }
        assert!(asked.iter().all(|a| *a == req(QuickSettings)), "{asked:?}");
    }

    #[test]
    fn a_gear_hold_opens_settings_once_and_is_no_tap() {
        let mut r = Rig::new();
        r.key(KEY_MENU, true);
        r.at(999);
        assert!(r.poll().is_empty(), "not yet");
        r.at(2);
        assert_eq!(r.poll(), vec![req(Settings)]);
        r.at(500);
        assert!(r.poll().is_empty(), "once");
        assert!(r.key(KEY_MENU, true).swallow, "a repeat after the hold");
        let release = r.key(KEY_MENU, false);
        assert!(release.swallow && release.actions.is_empty(), "a hold is no tap, and never counts toward the AV reset");
        for _ in 0..2 {
            r.key(KEY_MENU, true);
            r.at(60);
            assert_eq!(r.key(KEY_MENU, false).actions, vec![req(QuickSettings)]);
        }
    }

    #[test]
    fn input_is_a_picker_on_a_tap_and_the_hub_on_a_hold() {
        let mut r = Rig::new();
        let (press, polled, release) = r.hold(KEY_VIDEO_NEXT, 120);
        assert!(press.swallow && polled.is_empty());
        assert_eq!(release.actions, vec![req(InputPicker)]);
        let (_, polled, release) = r.hold(KEY_VIDEO_NEXT, 1100);
        assert_eq!(polled, vec![req(InputHub)]);
        assert!(release.swallow && release.actions.is_empty());
    }

    #[test]
    fn an_app_button_launches_its_app_and_ends_there() {
        let mut r = Rig::new();
        let press = r.key(KEY_NETFLIX, true);
        assert!(press.swallow);
        assert_eq!(press.actions, vec![req(LaunchApp("netflix".into()))], "LG's own target, on the press");
        assert!(r.key(KEY_NETFLIX, true).actions.is_empty(), "a repeat launches nothing");
        let release = r.key(KEY_NETFLIX, false);
        assert!(release.swallow && release.actions.is_empty());
    }

    #[test]
    fn the_buttons_follow_settings_remote() {
        let mut r = Rig::new();
        r.app_keys = AppKeyConfig::from_theme(&serde_json::json!({"remote": {"appKeys": {"netflix": "com.webos.app.hdmi2", "amazon": "", "disneyplus": 5}}}));
        assert_eq!(r.key(KEY_NETFLIX, true).actions, vec![req(LaunchApp("com.webos.app.hdmi2".into()))], "the owner's app");
        r.key(KEY_NETFLIX, false);
        let amazon = r.key(KEY_AMAZON, true);
        assert!(amazon.swallow && amazon.actions.is_empty(), "nothing, and the app does not get the key either");
        r.key(KEY_AMAZON, false);
        r.foreground = "com.webos.app.hdmi2".into();
        let again = r.key(KEY_NETFLIX, true);
        assert!(again.swallow && again.actions.is_empty(), "the app it opens is in front: nothing");
        r.key(KEY_NETFLIX, false);
        // A button the owner never set is LG's own, and LG hands it to its own app when that is in front.
        let c = AppKeyConfig::default();
        assert_eq!(c.decide(AppKey::DisneyPlus, "x"), AppKeyDecision::Launch("com.disney.disneyplus-prod".into()));
        assert_eq!(c.decide(AppKey::LgChannels, "com.webos.app.lgchannels"), AppKeyDecision::Pass);
        let c = AppKeyConfig::from_theme(&serde_json::json!({"remote": {"appKeys": {"disneyplus": 5}, "other": 1}}));
        assert_eq!(c, AppKeyConfig::default(), "a value that is no string is no choice");
        assert_eq!(AppKeyConfig::from_theme(&serde_json::json!({})), AppKeyConfig::default());
    }

    #[test]
    fn a_button_in_front_of_its_own_app_goes_to_the_app() {
        let mut r = Rig::new();
        r.foreground = "netflix".into();
        let press = r.key(KEY_NETFLIX, true);
        assert!(!press.swallow && press.actions.is_empty());
        assert!(!r.key(KEY_NETFLIX, false).swallow, "its release goes with it");
    }

    #[test]
    fn a_digit_reaches_the_app_and_a_second_of_it_asks_for_quick_access() {
        let mut r = Rig::new();
        let (press, polled, release) = r.hold(KEY_1 + 4, 300);
        assert!(!press.swallow && polled.is_empty() && !release.swallow, "a tap is the app's");
        let (press, polled, release) = r.hold(KEY_1 + 4, 1000);
        assert!(!press.swallow, "LG delivers the press");
        assert_eq!(polled, vec![req(QuickAccess(5)), Action::ClientRelease(KEY_1 + 4)], "the app is given its release now");
        assert!(release.swallow, "the real one is swallowed");
        let (_, polled, _) = r.hold(KEY_0, 1500);
        assert_eq!(polled[0], req(QuickAccess(0)), "0 is the editor");
    }

    #[test]
    fn a_keyboard_on_screen_keeps_the_digits_for_the_field() {
        let mut r = Rig::new();
        r.keyboard_up = true;
        let (_, polled, release) = r.hold(KEY_1, 1500);
        assert!(polled.is_empty() && !release.swallow);
    }

    #[test]
    fn mute_held_two_seconds_asks_for_the_accessibility_panel_and_is_no_toggle() {
        let mut r = Rig::new();
        let (press, polled, release) = r.hold(KEY_MUTE, 1500);
        assert!(!press.swallow && polled.is_empty() && !release.swallow, "a short press is the mute toggle, at its release");
        let (_, polled, release) = r.hold(KEY_MUTE, 2000);
        assert_eq!(polled, vec![req(Accessibility)]);
        assert!(release.swallow, "no toggle after the hold");
    }

    #[test]
    fn back_held_is_exit_and_the_app_is_given_its_release() {
        let mut r = Rig::new();
        let (_, polled, release) = r.hold(KEY_PREVIOUS, 400);
        assert!(polled.is_empty() && !release.swallow);
        let (press, polled, release) = r.hold(KEY_PREVIOUS, 1000);
        assert!(!press.swallow);
        assert_eq!(polled, vec![req(Exit), Action::ClientRelease(KEY_PREVIOUS)]);
        assert!(release.swallow);
        r.keyboard_up = true;
        let (_, polled, _) = r.hold(KEY_PREVIOUS, 1000);
        assert_eq!(polled[0], req(CloseKeyboard), "with the keyboard up the hold closes it first");
    }

    #[test]
    fn exit_with_the_keyboard_up_closes_the_keyboard_and_else_is_the_routings() {
        let mut r = Rig::new();
        let plain = r.key(KEY_EXIT, true);
        assert!(!plain.swallow && plain.actions.is_empty(), "mask and policy decide, as before");
        r.key(KEY_EXIT, false);
        r.keyboard_up = true;
        let up = r.key(KEY_EXIT, true);
        assert!(up.swallow);
        assert_eq!(up.actions, vec![req(CloseKeyboard)]);
        assert!(r.key(KEY_EXIT, false).swallow);
    }

    #[test]
    fn a_release_nobody_saw_the_press_of_is_the_routings_and_other_keys_are_untouched() {
        let mut r = Rig::new();
        assert_eq!(r.key(KEY_MENU, false), Outcome::default());
        let (press, polled, release) = r.hold(KEY_LEFT, 5000);
        assert!(!press.swallow && polled.is_empty() && !release.swallow);
    }
}
