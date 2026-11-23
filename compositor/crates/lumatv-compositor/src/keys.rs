//! The remote's key filters, moved out of LG's compositor (`KeyFilters/*.js`; what each does is read in
//! `docs/research/surfaces/KeyEntryPoints.json`) into the routing, without drawing anything: which key is which, what a tap, a
//! hold ask for. State only, time is an argument: `input.rs` feeds it the remote's events and the tick, and
//! `system_ui.rs` carries out what it asks for, so every decision here is a host test.
//!
//! LG's chain swallows the gear, Input and the app keys and decides at the release (a tap) or at 1 s (a hold, converted to
//! another key); the digits reach the app at once and a 1 s hold acts besides; Mute is a toggle at its release unless
//! it was held 2 s. The same shapes here, on every device (LG converts holds for its own remotes only, which is all this set has).
//! Back is LumaTV's own: it is held back until its release (a tap then reaches the app whole, press and release together) or until
//! [`BACK_HOLD`], when it opens the task manager and the app never hears of it. No repeat of a held Back reaches the app.
//! One exception: with LumaTV's own picker able to answer (`Ctx::picker_ready`) the Input key's tap acts on its press, as LumaTV's
//! key filter did under LG's compositor (`payload/keyfilter/lumatv.js`: "the Input key acts on the press, as LG's does"). Waiting
//! for the release put the picker's first frame behind the key by the length of the tap (50 ms for the gate's injected one).

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::input::code::*;

/// How long the gear, Input or a digit is held for LG's long-press conversion (`handleLongPressKeys`, 1000 ms).
pub const HOLD: Duration = Duration::from_millis(1000);
/// Back held for the task manager (`switcher.rs`): longer than a tap and shorter than LG's own long-press conversion (1000 ms). Chosen,
/// not measured; the tick polls it within one panel frame while keys are being pressed (`ticker.rs`).
pub const BACK_HOLD: Duration = Duration::from_millis(600);
/// Mute held for the accessibility panel (`handleAccessibility`, 2000 ms).
pub const HOLD_MUTE: Duration = Duration::from_millis(2000);

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

/// The keys the Rust shell's own screens know by name and the routing has no use for: the microphone key (push to talk), the on-screen remote's and
/// Search's, and the channel rocker, Info, Guide, List and the dash, which the channel guide pages, opens its sheet with, and is opened by (Live TV
/// launches LG's own guide on Guide and List unless the shell takes them), and which LumaTV's channel number entry types a sub-channel with. Their
/// codes are not known until the set has been read for the first three (no TV access when this was written): `keys.conf` lines `voice N`,
/// `screenremote N` and `search N` teach them, and the client is sent these names only while it lists them in its `takes` (`shell_rust.rs`), so a
/// mic key that the helper cannot answer stays LG's.
pub const SHELL_KEYS: [&str; 9] = ["voice", "screenremote", "search", "chup", "chdown", "info", "guide", "list", "dash"];

/// The keys of [`SHELL_KEYS`] whose codes are LG's, so nothing has to be taught: the channel rocker, Info, Guide, List and the dash.
const SHELL_BUILT_IN: [(u32, &str); 6] =
    [(KEY_CHANNELUP, "chup"), (KEY_CHANNELDOWN, "chdown"), (KEY_INFO, "info"), (KEY_GUIDE, "guide"), (KEY_LIST, "list"), (KEY_CHANNELDASH, "dash")];

/// Which evdev code is which key. The built-in codes are LG's (`tools`: network-input-service's key table; the gear and Input were
/// also read off this set); a `keys.conf` line `NAME CODE` adds a code for a key (gear, input, netflix, amazon, disneyplus,
/// lgchannels, or one of [`SHELL_KEYS`]), which is how a button this table does not know yet is taught without a rebuild.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyTable {
    extra: Vec<(u32, Logical)>,
    shell: Vec<(u32, &'static str)>,
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

    /// The name the Rust shell knows this code by: the one `keys.conf` taught it, or a built-in of [`SHELL_KEYS`].
    pub fn shell_name(&self, code: u32) -> Option<&'static str> {
        let taught = self.shell.iter().find(|(c, _)| *c == code).map(|(_, n)| *n);
        taught.or_else(|| SHELL_BUILT_IN.iter().find(|(c, _)| *c == code).map(|(_, n)| *n))
    }

    /// The code of an app button: the built-in one, or the one `keys.conf` taught.
    pub fn code_of(&self, key: AppKey) -> Option<u32> {
        if let Some((c, _)) = self.extra.iter().find(|(_, l)| *l == Logical::App(key)) {
            return Some(*c);
        }
        match key {
            AppKey::Netflix => Some(KEY_NETFLIX),
            AppKey::Amazon => Some(KEY_AMAZON),
            AppKey::DisneyPlus | AppKey::LgChannels => None,
        }
    }

    /// Whether the table names this code, built in or taught: what the routing treats as a key of its own. A code it does not
    /// name goes to the app as before.
    pub fn knows(&self, code: u32) -> bool {
        self.classify(code) != Logical::Other || self.shell_name(code).is_some()
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
            let shell = SHELL_KEYS.iter().copied().find(|k| *k == name);
            match (logical, shell, code, words.next()) {
                (Some(l), _, Some(c), None) => table.extra.push((c, l)),
                (None, Some(k), Some(c), None) => table.shell.push((c, k)),
                _ => bad.push(format!(
                    "line {}: {line:?} is not NAME CODE with a name of gear, input, netflix, amazon, disneyplus, lgchannels, voice, screenremote, search, chup, chdown, info, guide, list or dash",
                    n + 1
                )),
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
        out.extend(self.shell.iter().map(|(c, n)| ((*n).to_string(), *c)));
        out.extend(SHELL_BUILT_IN.iter().map(|(c, n)| ((*n).to_string(), *c)));
        out
    }
}

/// What the gear does when it is held (Settings > Remote & pointer > Hold gear: the theme's `remote.gearHold`). Any other word, a missing
/// key or the old on/off switch this replaced means the default, LumaTV's Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HoldAction {
    #[default]
    Settings,
    Notifications,
    Nothing,
}

impl HoldAction {
    pub fn from_theme(theme: &Value) -> HoldAction {
        match theme.pointer("/remote/gearHold").and_then(Value::as_str) {
            Some("notifications") => HoldAction::Notifications,
            Some("nothing") => HoldAction::Nothing,
            _ => HoldAction::Settings,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            HoldAction::Settings => "settings",
            HoldAction::Notifications => "notifications",
            HoldAction::Nothing => "nothing",
        }
    }
}

/// What a key asks of the system layer. LG's key chain opens these as windows of its own compositor (`systemUIManager/launch`);
/// they are asked of `State::system_request`, which is where LumaTV's own UI takes them once it exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemRequest {
    /// The gear's tap: Quick Settings (`com.webos.app.quicksettings`).
    QuickSettings,
    /// The Input key's tap: the input picker (`com.webos.app.quickinputpicker`). At its press when LumaTV's picker can answer, else at
    /// its release.
    InputPicker,
    /// The Input key held: the input picker again, for a press that did not already show it (LG's Home Hub is never opened).
    InputHold,
    /// The gear held: what the owner chose for it (`GearHold`). Never LG's Settings app, and never LG's hidden AV-reset menu, which three
    /// taps used to open (`tvhotkey`, `mute-hidden-action`): its items are in LumaTV's Settings.
    GearHold,
    /// Mute held: the accessibility quick panel.
    Accessibility,
    /// A digit held: Quick Access slot 1 to 9; 0 is the editor.
    QuickAccess(u8),
    /// An app button: launch this app (LG's own target for it, or the one the owner's `remote.buttons` names).
    LaunchApp(String),
    /// A button the owner set to open LumaTV's Settings.
    OpenSettings,
    /// A button the owner set to open the notification drawer.
    OpenNotifications,
    /// A button the owner set to turn the picture off and keep the sound.
    PictureOff,
    /// Back held for [`BACK_HOLD`]: the task manager, from anywhere (over a hung app, a keyboard, Home).
    TaskManager,
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
    /// A Back that was held back ended before its hold: the caller gives the app its press and this release now (the release itself is
    /// never delivered as it is).
    pub replay: bool,
    pub actions: Vec<Action>,
}

/// What the routing needs to know about the screen when a key arrives.
pub struct Ctx<'a> {
    /// The foreground card's app id, empty when none.
    pub foreground: &'a str,
    /// A keyboard window is mapped.
    pub keyboard_up: bool,
    /// The shell client can show the input picker now: the Input key's tap is acted on at its press.
    pub picker_ready: bool,
    /// The task manager is up: Back goes to it at once and is not held back to see whether it turns into a hold.
    pub taskmgr: bool,
}

struct Held {
    code: u32,
    logical: Logical,
    since: Instant,
    /// The press was taken here, so its repeats and release are too.
    swallowed: bool,
    /// The hold's action ran: the release is swallowed.
    fired: bool,
    /// The tap's action ran at the press (the Input key with the picker ready): the release asks for nothing.
    early: bool,
}

#[derive(Default)]
pub struct Router {
    pub table: KeyTable,
    down: Vec<Held>,
}

impl Router {
    pub fn new(table: KeyTable) -> Router {
        Router { table, ..Router::default() }
    }

    /// One press, repeat or release.
    pub fn event(&mut self, code: u32, pressed: bool, now: Instant, ctx: &Ctx) -> Outcome {
        if pressed {
            if let Some(h) = self.down.iter().find(|h| h.code == code) {
                return Outcome { swallow: h.swallowed || h.fired, ..Outcome::default() };
            }
            let logical = self.table.classify(code);
            let mut out = Outcome::default();
            let mut early = false;
            match logical {
                Logical::Gear => out.swallow = true,
                Logical::Input => {
                    out.swallow = true;
                    if ctx.picker_ready {
                        early = true;
                        out.actions.push(Action::Request(SystemRequest::InputPicker));
                    }
                }
                Logical::App(k) if ctx.foreground == k.lg_app() => {}
                Logical::App(k) => {
                    out.swallow = true;
                    out.actions.push(Action::Request(SystemRequest::LaunchApp(k.lg_app().to_string())));
                }
                // Held back until its release or its hold: whichever comes first decides what the app hears of it.
                Logical::Back if !ctx.taskmgr => out.swallow = true,
                Logical::Exit if ctx.keyboard_up => {
                    out.swallow = true;
                    out.actions.push(Action::Request(SystemRequest::CloseKeyboard));
                }
                _ => {}
            }
            self.down.push(Held { code, logical, since: now, swallowed: out.swallow, fired: false, early });
            return out;
        }
        let Some(i) = self.down.iter().position(|h| h.code == code) else { return Outcome::default() };
        let h = self.down.swap_remove(i);
        let mut out = Outcome { swallow: h.swallowed || h.fired, ..Outcome::default() };
        if !h.fired {
            match h.logical {
                Logical::Gear => out.actions.push(Action::Request(SystemRequest::QuickSettings)),
                Logical::Input if !h.early => out.actions.push(Action::Request(SystemRequest::InputPicker)),
                Logical::Back if h.swallowed => out.replay = true,
                _ => {}
            }
        }
        out
    }

    /// Whether a key with a hold is down and has not fired it: the tick has something to measure. A Back that was not held back has no hold.
    pub fn waiting(&self) -> bool {
        self.down.iter().any(|h| {
            !h.fired
                && match h.logical {
                    Logical::Gear | Logical::Input | Logical::Digit(_) | Logical::Mute => true,
                    Logical::Back => h.swallowed,
                    _ => false,
                }
        })
    }

    /// Whether `code` is down (a press seen, no release yet): its further presses are the remote's repeats.
    pub fn is_down(&self, code: u32) -> bool {
        self.down.iter().any(|h| h.code == code)
    }

    /// The holds that came due since the last call, each once. Called every tick.
    pub fn poll(&mut self, now: Instant, ctx: &Ctx) -> Vec<Action> {
        let mut out = Vec::new();
        for h in self.down.iter_mut().filter(|h| !h.fired) {
            let due = match h.logical {
                Logical::Gear | Logical::Input | Logical::Digit(_) => HOLD,
                Logical::Back if h.swallowed => BACK_HOLD,
                Logical::Mute => HOLD_MUTE,
                _ => continue,
            };
            if now.saturating_duration_since(h.since) < due {
                continue;
            }
            match h.logical {
                Logical::Gear => out.push(Action::Request(SystemRequest::GearHold)),
                // The press already showed the picker: showing it again would step it to the next input.
                Logical::Input if h.early => {}
                Logical::Input => out.push(Action::Request(SystemRequest::InputHold)),
                // Quick Access is not asked of a text field's keyboard: its digits are the field's (LG skips it while the keyboard shows).
                Logical::Digit(_) if ctx.keyboard_up => continue,
                Logical::Digit(n) => {
                    out.push(Action::Request(SystemRequest::QuickAccess(n)));
                    out.push(Action::ClientRelease(h.code));
                }
                // The app never had this press: there is no release to give it, and none of the repeats came. The task manager may have
                // come up by another door meanwhile (a double Home): the hold has nothing to ask then, and still ends the gesture.
                Logical::Back if ctx.taskmgr => {}
                Logical::Back => out.push(Action::Request(SystemRequest::TaskManager)),
                Logical::Mute => out.push(Action::Request(SystemRequest::Accessibility)),
                _ => continue,
            }
            h.fired = true;
        }
        out
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
        picker_ready: bool,
        taskmgr: bool,
    }

    impl Rig {
        fn new() -> Rig {
            Rig {
                router: Router::new(KeyTable::default()),
                now: Instant::now(),
                foreground: "com.webos.app.home".into(),
                keyboard_up: false,
                picker_ready: false,
                taskmgr: false,
            }
        }

        fn at(&mut self, ms: u64) {
            self.now += Duration::from_millis(ms);
        }

        fn key(&mut self, code: u32, pressed: bool) -> Outcome {
            let ctx = Ctx { foreground: &self.foreground, keyboard_up: self.keyboard_up, picker_ready: self.picker_ready, taskmgr: self.taskmgr };
            self.router.event(code, pressed, self.now, &ctx)
        }

        fn poll(&mut self) -> Vec<Action> {
            let ctx = Ctx { foreground: &self.foreground, keyboard_up: self.keyboard_up, picker_ready: self.picker_ready, taskmgr: self.taskmgr };
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
        assert_eq!(
            (t.classify(1041), t.classify(1042), t.classify(140), t.classify(139)),
            (Logical::App(AppKey::DisneyPlus), Logical::App(AppKey::LgChannels), Logical::Gear, Logical::Gear)
        );
        let (_, bad) = KeyTable::parse("volume 5\ngear\ngear 1 2\ngear x\nnetflix 1037\n");
        assert_eq!(bad.len(), 4, "{bad:?}");
    }

    #[test]
    fn the_shell_keys_are_taught_by_name_and_are_known_but_are_no_logical_key_of_the_routing() {
        let (t, bad) = KeyTable::parse("voice 1050\nscreenremote 1051 # the on-screen remote\nsearch 217\n");
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!((t.shell_name(1050), t.shell_name(1051), t.shell_name(217), t.shell_name(1)), (Some("voice"), Some("screenremote"), Some("search"), None));
        assert!(t.knows(1050) && t.knows(217) && !t.knows(1052), "taught: not reported as a code with no name");
        assert_eq!(t.classify(1050), Logical::Other, "the routing has nothing to do for them: the app keeps them unless the shell takes them");
        let named = t.named();
        assert!(named.contains(&("voice".to_string(), 1050)) && named.contains(&("search".to_string(), 217)), "{named:?}");
        let (_, bad) = KeyTable::parse("mic 5\nvoice\nvoice 1 2\n");
        assert_eq!(bad.len(), 3, "{bad:?}");
    }

    #[test]
    fn the_channel_rocker_info_guide_list_and_dash_are_named_for_the_shell_without_being_taught() {
        let t = KeyTable::default();
        let names = [KEY_CHANNELUP, KEY_CHANNELDOWN, KEY_INFO, KEY_GUIDE, KEY_LIST, KEY_CHANNELDASH].map(|c| t.shell_name(c));
        assert_eq!(names, [Some("chup"), Some("chdown"), Some("info"), Some("guide"), Some("list"), Some("dash")]);
        assert!(t.knows(KEY_INFO), "named, so not reported as a code with no name");
        assert_eq!(t.classify(KEY_CHANNELUP), Logical::Other, "the routing has nothing to do for them: the app keeps them unless the shell takes them");
        let (t, bad) = KeyTable::parse("info 1111\n");
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!((t.shell_name(1111), t.shell_name(KEY_INFO)), (Some("info"), Some("info")), "a taught code is one more, not a replacement");
        assert!(t.named().contains(&("chdown".to_string(), KEY_CHANNELDOWN)));
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
    fn gear_taps_are_quick_settings_however_many_and_however_close() {
        // Three taps within three seconds used to open LG's hidden AV-reset menu (`tvhotkey`); every tap is Quick Settings now.
        let mut r = Rig::new();
        let mut asked = Vec::new();
        for _ in 0..7 {
            r.key(KEY_MENU, true);
            r.at(80);
            asked.extend(r.key(KEY_MENU, false).actions);
            r.at(300);
        }
        assert_eq!(asked, vec![req(QuickSettings); 7]);
    }

    #[test]
    fn a_gear_hold_asks_for_the_hold_action_once_and_is_no_tap() {
        let mut r = Rig::new();
        r.key(KEY_MENU, true);
        r.at(999);
        assert!(r.poll().is_empty(), "not yet");
        r.at(2);
        assert_eq!(r.poll(), vec![req(GearHold)]);
        r.at(500);
        assert!(r.poll().is_empty(), "once");
        assert!(r.key(KEY_MENU, true).swallow, "a repeat after the hold");
        let release = r.key(KEY_MENU, false);
        assert!(release.swallow && release.actions.is_empty(), "a hold is no tap");
        for _ in 0..2 {
            r.key(KEY_MENU, true);
            r.at(60);
            assert_eq!(r.key(KEY_MENU, false).actions, vec![req(QuickSettings)]);
        }
    }

    #[test]
    fn the_hold_action_is_the_themes_word_and_settings_for_anything_else() {
        let of = |t: serde_json::Value| HoldAction::from_theme(&t);
        assert_eq!(of(serde_json::json!({})), HoldAction::Settings);
        assert_eq!(of(serde_json::json!({"remote": {"gearHold": "notifications"}})), HoldAction::Notifications);
        assert_eq!(of(serde_json::json!({"remote": {"gearHold": "nothing"}})), HoldAction::Nothing);
        assert_eq!(of(serde_json::json!({"remote": {"gearHold": "settings"}})), HoldAction::Settings);
        assert_eq!(of(serde_json::json!({"remote": {"gearHold": false}})), HoldAction::Settings, "the old switch's off handed the hold to LG's Settings, which is never opened now");
        assert_eq!(of(serde_json::json!({"remote": {"gearHold": "lg"}})), HoldAction::Settings);
    }

    #[test]
    fn input_is_a_picker_on_a_tap_and_a_hold() {
        let mut r = Rig::new();
        let (press, polled, release) = r.hold(KEY_VIDEO_NEXT, 120);
        assert!(press.swallow && polled.is_empty());
        assert_eq!(release.actions, vec![req(InputPicker)], "without a picker of LumaTV's to answer the tap is decided at the release");
        let (_, polled, release) = r.hold(KEY_VIDEO_NEXT, 1100);
        assert_eq!(polled, vec![req(InputHold)]);
        assert!(release.swallow && release.actions.is_empty());
    }

    #[test]
    fn with_the_picker_ready_input_acts_on_its_press_and_its_release_asks_for_nothing() {
        let mut r = Rig::new();
        r.picker_ready = true;
        let press = r.key(KEY_VIDEO_NEXT, true);
        assert!(press.swallow);
        assert_eq!(press.actions, vec![req(InputPicker)], "the first frame is not held behind the tap");
        r.at(40);
        let repeat = r.key(KEY_VIDEO_NEXT, true);
        assert!(repeat.swallow && repeat.actions.is_empty(), "a repeat asks for nothing");
        r.at(60);
        assert!(r.poll().is_empty());
        let release = r.key(KEY_VIDEO_NEXT, false);
        assert!(release.swallow && release.actions.is_empty(), "the tap was acted on");
        // The next tap is another one: the key again goes on to the next input.
        r.at(500);
        assert_eq!(r.key(KEY_VIDEO_NEXT, true).actions, vec![req(InputPicker)]);
        r.key(KEY_VIDEO_NEXT, false);
    }

    #[test]
    fn a_press_that_found_no_picker_leaves_the_tap_to_the_release_whatever_the_picker_is_by_then() {
        let mut r = Rig::new();
        assert!(r.key(KEY_VIDEO_NEXT, true).actions.is_empty());
        r.picker_ready = true;
        r.at(80);
        assert_eq!(r.key(KEY_VIDEO_NEXT, false).actions, vec![req(InputPicker)]);
        // And the other way: a press the picker answered is not asked again at the release when the shell is gone by then.
        assert_eq!(r.key(KEY_VIDEO_NEXT, true).actions, vec![req(InputPicker)]);
        r.picker_ready = false;
        r.at(80);
        assert!(r.key(KEY_VIDEO_NEXT, false).actions.is_empty());
    }

    #[test]
    fn with_the_picker_ready_a_held_input_asks_for_nothing_more() {
        let mut r = Rig::new();
        r.picker_ready = true;
        let (press, polled, release) = r.hold(KEY_VIDEO_NEXT, 1100);
        assert_eq!(press.actions, vec![req(InputPicker)], "the press asked for the picker, as LG's chain acts on Input's press");
        assert!(polled.is_empty(), "the hold does not show it a second time, which would step it on");
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
    fn a_back_tap_is_held_back_and_given_to_the_app_whole_at_its_release() {
        for code in [KEY_PREVIOUS, KEY_BACK] {
            let mut r = Rig::new();
            let (press, polled, release) = r.hold(code, 150);
            assert!(press.swallow && press.actions.is_empty() && !press.replay, "the press waits to see whether it is a hold");
            assert!(polled.is_empty());
            assert!(release.replay && release.swallow && release.actions.is_empty(), "press and release are the caller's to deliver: {release:?}");
        }
    }

    #[test]
    fn a_back_held_for_the_task_manager_asks_for_it_once_and_the_app_hears_nothing_of_it() {
        let mut r = Rig::new();
        assert!(r.key(KEY_PREVIOUS, true).swallow);
        assert!(r.router.waiting(), "the tick has a hold to measure");
        // the remote's auto-repeat while it is held: none of it reaches the app
        r.at(300);
        assert!(r.key(KEY_PREVIOUS, true).swallow);
        r.at(299);
        assert!(r.poll().is_empty(), "599 ms is still a tap");
        r.at(1);
        assert_eq!(r.poll(), vec![req(TaskManager)], "and the app never had the press, so it has no release to be given");
        assert!(r.poll().is_empty(), "once");
        assert!(!r.router.waiting());
        r.at(200);
        assert!(r.key(KEY_PREVIOUS, true).swallow, "repeats after the hold are the router's too");
        let release = r.key(KEY_PREVIOUS, false);
        assert!(release.swallow && !release.replay && release.actions.is_empty());
    }

    #[test]
    fn the_back_hold_opens_the_task_manager_over_the_keyboard_and_over_home() {
        let mut r = Rig::new();
        r.keyboard_up = true;
        let (press, polled, release) = r.hold(KEY_PREVIOUS, 700);
        assert!(press.swallow);
        assert_eq!(polled, vec![req(TaskManager)], "not the keyboard's close: Exit does that");
        assert!(release.swallow && !release.replay);
        r.foreground = "org.test.player".into();
        r.keyboard_up = false;
        assert_eq!(r.hold(KEY_BACK, 700).1, vec![req(TaskManager)]);
    }

    #[test]
    fn with_the_task_manager_up_back_goes_to_it_at_once_and_no_hold_is_measured() {
        let mut r = Rig::new();
        r.taskmgr = true;
        let press = r.key(KEY_PREVIOUS, true);
        assert!(!press.swallow && !r.router.waiting());
        r.at(300);
        assert!(!r.key(KEY_PREVIOUS, true).swallow, "its repeats too");
        r.at(2000);
        assert!(r.poll().is_empty());
        let release = r.key(KEY_PREVIOUS, false);
        assert!(!release.swallow && !release.replay);
    }

    #[test]
    fn a_back_hold_that_finds_the_task_manager_already_up_asks_for_nothing_and_ends_the_gesture() {
        let mut r = Rig::new();
        assert!(r.key(KEY_PREVIOUS, true).swallow);
        r.taskmgr = true;
        r.at(700);
        assert!(r.poll().is_empty());
        let release = r.key(KEY_PREVIOUS, false);
        assert!(release.swallow && !release.replay, "the app hears nothing of it");
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
