//! What the TV says about what is on it (`panels/Banner.qml` of the QML shell, `docs/mocks/20-channel-banner`, `25-input-banner`, `25-input-pill`,
//! `18-no-signal`), for the three moments LG's `inputcommon` draws:
//!
//! - `input`: the pill that says which input is in front and what it carries (4K 120 Hz HDR10, eARC), shown for four seconds when an input comes
//!   to the front and the owner has switched LumaTV's own banners on (theme `banners.own`, off by default: LG's own are restyled through the overlay
//!   mount and draw the same moment); grown into a bar with Details, Inputs and Picture buttons when a spec asks for them;
//! - `nosignal`: the black screen of an input with nothing on it: a dim icon, which port, what to do and two buttons (Inputs, Home). It is black on
//!   purpose, in both materials, and its block hops to another spot every minute: a still frame on an OLED is what the screen must never hold;
//! - `channel`: the channel banner of live TV (the logo, number and name, the programme with its time and how much is left; INFO adds what it is about,
//!   what is next and the signal facts). Live TV is rarely used (Nitin, 2026-10-23), so this is the banner only, no guide.
//!
//! A spec comes from the inputs themselves (`foreground`, the pill), or from the owner of the moment as the topic `banner.show` (the spec's JSON,
//! `banner.hide` takes it away), which is what the QML's development door and `showBanner` were. Only a banner with buttons takes the keys (left and
//! right move, OK picks, Back ends it); the pill and the channel banner take none and go by themselves.

use super::channel_feed::{self, Feed, Raise};
use super::lg_mask::{self, LgMask};
use crate::host::channels;
use crate::host::inputs::{self, Inputs};
use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

pub const SHOW_TOPIC: &str = "banner.show";
pub const HIDE_TOPIC: &str = "banner.hide";
/// The tag of the one-shot ask of the input list (the pill's name is the device's own).
pub const INPUTS_TAG: u64 = 0x4A00_0001;
/// The tag of the call that goes Home.
pub const HOME_TAG: u64 = 0x4A00_0002;
/// How long an input pill waits for the list before it says the port alone.
const LIST_WAIT_MS: u64 = 1500;
/// How long a banner is held: the pill four seconds, no signal until something is picked, one with buttons eight, and the channel banner as LG
/// holds its own (`BannerInterfaces.qml`: five seconds for a change or a launch), eight for INFO (`channel_feed`).
const HOLD_PILL_MS: u64 = 4000;
const HOLD_BUTTONS_MS: u64 = 8000;
pub const HOLD_CHANNEL_MS: u64 = 5000;
pub const HOLD_INFO_MS: u64 = 8000;
/// The no-signal block moves to its next spot this often.
pub const SPOT_MS: u64 = 60_000;
/// The spots of the no-signal block, as offsets from the middle of the screen (the mock's first is 180 right and 120 up: `[0, 0]` here is
/// the middle, and the others hop about it).
pub const SPOTS: [(f32, f32); 5] = [(0.0, 0.0), (180.0, -120.0), (-200.0, 100.0), (140.0, 150.0), (-170.0, -110.0)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Channel,
    Input,
    NoSignal,
}

/// What a button of a banner does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Details,
    Inputs,
    Picture,
    Home,
}

impl Pick {
    pub fn parse(id: &str) -> Option<Pick> {
        Some(match id {
            "details" => Pick::Details,
            "inputs" => Pick::Inputs,
            "picture" => Pick::Picture,
            "home" => Pick::Home,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Pick::Details => "details",
            Pick::Inputs => "inputs",
            Pick::Picture => "picture",
            Pick::Home => "home",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Pick::Details => "info",
            Pick::Inputs => "cable",
            Pick::Picture => "image",
            Pick::Home => "house",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Pick::Details => "Details",
            Pick::Inputs => "Inputs",
            Pick::Picture => "Picture",
            Pick::Home => "Home",
        }
    }
}

/// One banner, as its owner words it.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub kind: Kind,
    // channel
    pub number: String,
    pub call: String,
    pub sub: String,
    pub progress: f32,
    pub next: String,
    pub chips: Vec<String>,
    /// The channel's logo: a picture file on the set ("" for a channel with none).
    pub logo: String,
    /// What the programme is about, under the title (the INFO banner only).
    pub detail: String,
    /// The banner INFO raised: INFO and Back end it.
    pub info: bool,
    // input
    pub icon: String,
    pub name: String,
    pub facts: String,
    // nosignal
    pub port: String,
    pub body: String,
    /// The headline of a channel banner (the programme), of a no-signal screen (what is wrong).
    pub title: String,
    pub buttons: Vec<Pick>,
    pub hold_ms: u64,
}

impl Spec {
    fn blank(kind: Kind) -> Spec {
        Spec {
            kind,
            number: String::new(),
            call: String::new(),
            sub: String::new(),
            progress: 0.0,
            next: String::new(),
            chips: Vec::new(),
            logo: String::new(),
            detail: String::new(),
            info: false,
            icon: String::new(),
            name: String::new(),
            facts: String::new(),
            port: String::new(),
            body: String::new(),
            title: String::new(),
            buttons: Vec::new(),
            hold_ms: HOLD_PILL_MS,
        }
    }

    /// A channel banner with nothing said yet, held as LG holds its own.
    pub fn channel() -> Spec {
        Spec { hold_ms: HOLD_CHANNEL_MS, ..Spec::blank(Kind::Channel) }
    }

    /// The spec of a JSON document (`{"kind": "input", "icon", "name", "facts", "buttons": bool, "actions": [ids]}`, and the others'); none for a kind
    /// that is not one of the three.
    pub fn parse(v: &Value) -> Option<Spec> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let kind = match v.get("kind").and_then(Value::as_str)? {
            "channel" => Kind::Channel,
            "input" => Kind::Input,
            "nosignal" => Kind::NoSignal,
            _ => return None,
        };
        let mut s = Spec::blank(kind);
        s.title = text("title");
        s.hold_ms = v.get("holdMs").and_then(Value::as_u64).unwrap_or(0);
        match kind {
            Kind::Channel => {
                s.number = text("number");
                s.call = text("call");
                s.sub = text("sub");
                s.next = text("next");
                s.progress = v.get("progress").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 1.0) as f32;
                s.chips = v.get("chips").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).take(6).collect()).unwrap_or_default();
                s.logo = text("logo");
                s.detail = text("detail");
                s.info = v.get("info").and_then(Value::as_bool) == Some(true);
                s.hold_ms = if s.hold_ms > 0 { s.hold_ms } else if s.info { HOLD_INFO_MS } else { HOLD_CHANNEL_MS };
            }
            Kind::Input => {
                s.icon = if text("icon").is_empty() { "cable".to_string() } else { text("icon") };
                s.name = text("name");
                s.facts = text("facts");
                if v.get("buttons").and_then(Value::as_bool) == Some(true) {
                    // the buttons the surface that shows it can answer; a spec without `actions` has all three
                    let ids: Vec<String> = match v.get("actions").and_then(Value::as_array) {
                        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                        None => vec!["details".into(), "inputs".into(), "picture".into()],
                    };
                    s.buttons = ids.iter().filter_map(|i| Pick::parse(i)).collect();
                }
                s.hold_ms = if s.hold_ms > 0 { s.hold_ms } else if s.buttons.is_empty() { HOLD_PILL_MS } else { HOLD_BUTTONS_MS };
            }
            Kind::NoSignal => {
                s.icon = if text("icon").is_empty() { "unplug".to_string() } else { text("icon") };
                s.port = text("port");
                s.body = text("body");
                s.buttons = vec![Pick::Inputs, Pick::Home];
                // black until something is picked: no time of its own unless the spec gives one
            }
        }
        Some(s)
    }

    /// The pill of the input in front: `port · what the device calls itself`, and eARC where the device is a receiver.
    pub fn for_input(app_id: &str, list: &Inputs) -> Spec {
        let mut s = Spec::blank(Kind::Input);
        if app_id == "com.webos.app.livetv" {
            s.icon = "antenna".into();
            s.name = "Live TV".into();
            s.facts = "Antenna".into();
            return s;
        }
        let port = inputs::port_name(app_id);
        let cur = list.list().iter().find(|e| e.app_id == app_id);
        let label = cur.filter(|e| e.connected).map_or("", |e| e.label.as_str());
        s.icon = cur.map_or("cable", |e| if e.icon.is_empty() { "cable" } else { e.icon.as_str() }).to_string();
        s.name = if !label.is_empty() && label != port { format!("{port} \u{b7} {label}") } else { port };
        if !label.is_empty() && inputs::is_receiver(label) {
            s.facts = "eARC".into();
        }
        s
    }

    /// Whether it has buttons, and so takes the keys.
    pub fn interactive(&self) -> bool {
        !self.buttons.is_empty()
    }
}

/// An input app: what comes to the front when an input is picked.
pub fn is_input_app(app: &str) -> bool {
    let rest = app.strip_prefix("com.webos.app.");
    rest.is_some_and(|r| {
        r == "livetv"
            || ["hdmi", "dp", "usbc"].iter().any(|p| r.strip_prefix(p).is_some_and(|n| n.len() == 1 && n.chars().all(|c| c.is_ascii_digit())))
    })
}

/// What the surface draws.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub spec: Spec,
    /// The button that has the focus.
    pub index: usize,
    /// The spot of the no-signal block.
    pub spot: usize,
}

/// What a banner asks of the shell besides effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Inputs: the input picker.
    OpenPicker,
    /// Picture: the quick panel.
    OpenQuick,
}

#[derive(Debug, Default)]
pub struct Banner {
    /// The owner's own banners (theme `banners.own`).
    own: bool,
    inputs: Inputs,
    /// What is in front: an app, not Home (`up`), and with the screen on (`front`: what the banners are for).
    app: String,
    up: bool,
    front: bool,
    screen_on: bool,
    spec: Option<Spec>,
    index: usize,
    spot: usize,
    /// The banner ends at this time (none: it stays until a button or Back).
    until: Option<u64>,
    spot_at: Option<u64>,
    /// An input came to the front and its pill waits for the list: the app and when it was asked for.
    waiting: Option<(String, u64)>,
    /// What live TV's channel banner says and when (`channel_feed`).
    feed: Feed,
    /// LG's own banner on Live TV, which the compositor leaves undrawn while this one stands in for it (`lg_mask`).
    lg: LgMask,
    intents: Vec<Intent>,
}

impl Banner {
    pub fn new(theme: &Value) -> Self {
        let mut b = Self { screen_on: true, ..Self::default() };
        let _ = b.set_theme(theme, 0);
        b
    }

    /// The banners a TV check asks for by name (`ctl shell show banner pill|bar|nosignal`: what the QML's development door did), made of the input in
    /// front, or HDMI 1 when none is.
    pub fn show_sample(&mut self, page: &str, now: u64) -> Vec<Effect> {
        let app = if is_input_app(&self.app) { self.app.clone() } else { "com.webos.app.hdmi1".to_string() };
        let spec = match page {
            "pill" => Spec::for_input(&app, &self.inputs),
            "bar" => {
                let mut s = Spec::for_input(&app, &self.inputs);
                s.buttons = vec![Pick::Details, Pick::Inputs, Pick::Picture];
                s.hold_ms = HOLD_BUTTONS_MS;
                s
            }
            "nosignal" => {
                let port = inputs::port_name(&app);
                let mut s = Spec::blank(Kind::NoSignal);
                s.icon = "unplug".into();
                s.title = "Nothing is connected here".into();
                s.body = format!("Plug a device into {port}, or pick another input. The screen stays dark to protect the panel.");
                s.port = port;
                s.buttons = vec![Pick::Inputs, Pick::Home];
                s.hold_ms = 0;
                s
            }
            _ => return Vec::new(),
        };
        self.show(spec, now)
    }

    /// The owner's switch (`banners.own`): off, nothing of LumaTV's own shows on its own; a banner that was asked for by its owner always does.
    pub fn set_theme(&mut self, theme: &Value, now: u64) -> Vec<Effect> {
        self.own = theme.pointer("/banners/own").and_then(Value::as_bool) == Some(true);
        if !self.own {
            self.waiting = None;
        }
        let mut out = self.sync_live(now);
        if !self.own && self.spec.as_ref().is_some_and(|s| s.kind == Kind::Channel) && !self.feed.live() {
            out.extend(self.hide());
        }
        out
    }

    /// Live TV in front with the owner's banners on is what the channel banner follows the tuner for, and what LG's own banner is blanked for.
    fn sync_live(&mut self, now: u64) -> Vec<Effect> {
        let live = self.front && self.own && self.app == channels::LIVE_TV_APP;
        let was = self.feed.live();
        let mut out = self.feed.set_live(live, now);
        if live != was {
            out.extend(if live { self.lg.engage() } else { self.lg.release() });
        }
        out
    }

    pub fn up(&self) -> bool {
        self.spec.is_some()
    }

    /// A banner with buttons is up: it has the keys.
    pub fn wants(&self) -> bool {
        self.spec.as_ref().is_some_and(Spec::interactive)
    }

    pub fn view(&self) -> Option<View> {
        self.spec.clone().map(|spec| View { spec, index: self.index.min(self.spec.as_ref().map_or(0, |s| s.buttons.len().saturating_sub(1))), spot: self.spot })
    }

    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    fn show(&mut self, spec: Spec, now: u64) -> Vec<Effect> {
        self.until = (spec.hold_ms > 0).then(|| now + spec.hold_ms);
        self.spot_at = (spec.kind == Kind::NoSignal).then(|| now + SPOT_MS);
        self.index = 0;
        self.spot = 0;
        self.spec = Some(spec);
        vec![Effect::Redraw]
    }

    fn hide(&mut self) -> Vec<Effect> {
        self.waiting = None;
        self.feed.gone();
        if self.spec.take().is_some() {
            self.until = None;
            self.spot_at = None;
            return vec![Effect::Redraw];
        }
        Vec::new()
    }

    /// What the feed asked of the banner: its effects, the banner it raises or refreshes, the plain pill when the tuner named no channel.
    fn feed_out(&mut self, out: channel_feed::Out, now: u64) -> Vec<Effect> {
        let mut fx = out.effects;
        if out.changed {
            fx.extend(self.lg.tuned());
        }
        match out.raise {
            Some(Raise::Show(spec)) => fx.extend(self.show(spec, now)),
            Some(Raise::Refresh(spec)) if self.spec.as_ref().is_some_and(|s| s.kind == Kind::Channel) && self.spec.as_ref() != Some(&spec) => {
                self.spec = Some(spec);
                fx.push(Effect::Redraw);
            }
            Some(Raise::Refresh(_)) | None => {}
        }
        if out.pill && self.spec.is_none() {
            let spec = Spec::for_input(channels::LIVE_TV_APP, &self.inputs);
            fx.extend(self.show(spec, now));
        }
        fx
    }

    /// The keys the layer takes by name while an app has the focus: INFO while Live TV is in front with the banners on (so LG's own INFO card
    /// does not open under this one), and Back while the INFO banner is up (it ends it, as it ends LG's).
    pub fn takes(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        if self.feed.live() {
            keys.push("info");
            if self.feed.info_up() {
                keys.push("back");
            }
        }
        keys
    }

    /// An input came to the front (a switch, with the owner's banners on): the pill is asked for with the list's word on the device.
    fn input_switched(&mut self, app: &str, now: u64) -> Vec<Effect> {
        self.waiting = Some((app.to_string(), now));
        vec![Effect::Luna { tag: INPUTS_TAG, uri: inputs::EIM.into(), payload: json!({}), subscribe: false }]
    }

    fn pill_ready(&mut self, now: u64) -> Vec<Effect> {
        let Some((app, _)) = self.waiting.take() else { return Vec::new() };
        if !self.front || self.app != app || !self.own {
            return Vec::new();
        }
        let spec = Spec::for_input(&app, &self.inputs);
        self.show(spec, now)
    }

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        match input {
            Input::Topic { topic, data } => match topic.as_str() {
                "foreground" => {
                    let app = data.get("app").and_then(Value::as_str).unwrap_or("").to_string();
                    let home = data.get("home").and_then(Value::as_bool).unwrap_or(false);
                    let up = !home && !app.is_empty() && app != "com.webos.app.home";
                    let front = up && self.screen_on;
                    let was = (self.front, self.app.clone());
                    self.up = up;
                    self.front = front;
                    self.app = app.clone();
                    if was == (front, app.clone()) {
                        return Vec::new();
                    }
                    // what is in front changed: a banner the inputs made goes with the input, and an input that came wants its pill (Live TV's
                    // is the channel banner, which the feed raises once the tuner names its channel)
                    let mut out = self.sync_live(now);
                    let channel_up = self.spec.as_ref().is_some_and(|s| s.kind == Kind::Channel);
                    if self.spec.as_ref().is_some_and(|s| s.kind == Kind::Input) || self.waiting.is_some() || !front || (channel_up && !self.feed.live()) {
                        out.extend(self.hide());
                    }
                    if front && self.own && is_input_app(&app) && !self.feed.live() {
                        out.extend(self.input_switched(&app, now));
                    }
                    return out;
                }
                "screen" => {
                    self.screen_on = data.get("on").and_then(Value::as_bool) != Some(false);
                    // with the screen off nothing is in front; Live TV is followed again, and LG's banner blanked, from the moment it is on
                    self.front = self.up && self.screen_on;
                    let mut out = self.sync_live(now);
                    if !self.screen_on {
                        out.extend(self.hide());
                    }
                    return out;
                }
                SHOW_TOPIC => {
                    return match Spec::parse(data) {
                        Some(spec) => {
                            self.feed.gone();
                            self.show(spec, now)
                        }
                        None => Vec::new(),
                    };
                }
                HIDE_TOPIC => return self.hide(),
                _ => {}
            },
            Input::Luna { tag, payload } if *tag == INPUTS_TAG => {
                self.inputs.parse(payload);
                return self.pill_ready(now);
            }
            Input::Luna { tag, payload } if *tag == lg_mask::POSITION_TAG => return self.lg.position(payload),
            Input::Luna { tag, payload } if [channel_feed::LAST_TAG, channel_feed::INFO_TAG, channel_feed::NOW_TAG].contains(tag) => {
                let mut out = self.lg.tuner_answer(*tag, payload);
                let fed = self.feed.update(input, now);
                out.extend(self.feed_out(fed, now));
                return out;
            }
            Input::Fetched { tag, .. } if *tag == channel_feed::ONLINE_TAG => {
                let fed = self.feed.update(input, now);
                return self.feed_out(fed, now);
            }
            Input::Key { name, down: true, repeat } if self.wants() => return self.key(name, *repeat, now),
            Input::Key { name, down: true, repeat: false } if name == "info" && self.feed.live() => {
                if self.feed.info_up() {
                    return self.hide();
                }
                let out = self.feed.info_pressed(now);
                return self.feed_out(out, now);
            }
            Input::Key { name, down: true, repeat: false } if name == "back" && self.feed.info_up() => return self.hide(),
            Input::Tick => {
                let mut out = Vec::new();
                let fed = self.feed.update(input, now);
                out.extend(self.feed_out(fed, now));
                if self.waiting.as_ref().is_some_and(|(_, at)| now >= at + LIST_WAIT_MS) {
                    out.extend(self.pill_ready(now));
                }
                if self.until.is_some_and(|t| now >= t) {
                    out.extend(self.hide());
                }
                if self.spot_at.is_some_and(|t| now >= t) {
                    self.spot = (self.spot + 1) % SPOTS.len();
                    self.spot_at = Some(now + SPOT_MS);
                    out.push(Effect::Redraw);
                }
                return out;
            }
            _ => {}
        }
        Vec::new()
    }

    /// A press while a banner with buttons is up: left and right move, OK picks and ends it, Back ends it. Any key keeps the banner a while longer.
    fn key(&mut self, name: &str, repeat: bool, now: u64) -> Vec<Effect> {
        let Some(spec) = self.spec.clone() else { return Vec::new() };
        let last = spec.buttons.len().saturating_sub(1);
        let mut out = vec![Effect::Redraw];
        match name {
            "left" => self.index = self.index.min(last).saturating_sub(1),
            "right" => self.index = (self.index + 1).min(last),
            "ok" if !repeat => {
                let pick = spec.buttons[self.index.min(last)];
                out.extend(self.pick(pick));
                out.extend(self.hide());
                return out;
            }
            "back" if !repeat => return self.hide(),
            _ => return Vec::new(),
        }
        if spec.hold_ms > 0 {
            self.until = Some(now + spec.hold_ms);
        }
        out
    }

    /// A button was chosen with the pointer.
    pub fn picked(&mut self, i: usize) -> Vec<Effect> {
        let Some(pick) = self.spec.as_ref().and_then(|s| s.buttons.get(i).copied()) else { return Vec::new() };
        self.index = i;
        let mut out = self.pick(pick);
        out.extend(self.hide());
        out
    }

    /// Moves the focus to button `i` (the pointer's hover or a click begun on it).
    pub fn focus(&mut self, i: usize) {
        if self.spec.as_ref().is_some_and(|s| i < s.buttons.len()) {
            self.index = i;
        }
    }

    fn pick(&mut self, pick: Pick) -> Vec<Effect> {
        match pick {
            Pick::Inputs => {
                self.intents.push(Intent::OpenPicker);
                Vec::new()
            }
            Pick::Picture => {
                self.intents.push(Intent::OpenQuick);
                Vec::new()
            }
            Pick::Home => vec![Effect::Luna {
                tag: HOME_TAG,
                uri: "luna://com.webos.applicationManager/launchDefaultApp".into(),
                payload: json!({"category": "home"}),
                subscribe: false,
            }],
            Pick::Details => Vec::new(),
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.until, self.spot_at, self.waiting.as_ref().map(|(_, at)| at + LIST_WAIT_MS), self.feed.due_at()].into_iter().flatten().min()
    }

    pub fn state(&self) -> Value {
        match &self.spec {
            Some(s) => json!({"up": true, "kind": format!("{:?}", s.kind).to_lowercase(), "keys": s.interactive(), "buttons": s.buttons.iter().map(|b| b.id()).collect::<Vec<_>>(), "index": self.index, "spot": self.spot, "live": self.feed.live(), "info": s.info}),
            None => json!({"up": false, "live": self.feed.live()}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000_000;

    fn on() -> Banner {
        Banner::new(&json!({"banners": {"own": true}}))
    }

    fn fg(app: &str, home: bool) -> Input {
        Input::Topic { topic: "foreground".into(), data: json!({"app": app, "home": home, "covered": true}) }
    }

    fn list() -> Value {
        json!({"returnValue": true, "devices": [
            {"appId": "com.webos.app.hdmi1", "label": "Apple TV 4K", "connected": true},
            {"appId": "com.webos.app.hdmi2", "label": "AVR-S760H", "connected": true},
            {"appId": "com.webos.app.hdmi3", "label": "", "connected": false}]})
    }

    fn spec_of(b: &Banner) -> Spec {
        b.view().expect("a banner is up").spec
    }

    #[test]
    fn an_input_that_comes_to_the_front_shows_its_pill_with_the_devices_own_name_for_four_seconds_and_only_with_the_owners_banners_on() {
        let mut b = on();
        assert!(b.update(&fg("com.webos.app.home", true), T0).is_empty() || !b.up());
        let asked = b.update(&fg("com.webos.app.hdmi1", false), T0 + 100);
        assert!(matches!(&asked[..], [Effect::Luna { tag: INPUTS_TAG, subscribe: false, .. }]), "the list is asked for once: {asked:?}");
        assert!(!b.up(), "the pill waits for the list");
        assert_eq!(b.due_at(), Some(T0 + 100 + LIST_WAIT_MS));
        assert_eq!(b.update(&Input::Luna { tag: INPUTS_TAG, payload: list() }, T0 + 150), vec![Effect::Redraw]);
        let s = spec_of(&b);
        assert_eq!((s.kind, s.name.as_str(), s.facts.as_str(), s.icon.as_str()), (Kind::Input, "HDMI 1 \u{b7} Apple TV 4K", "", "tv-minimal-play"));
        assert!(!b.wants(), "a pill takes no key: the keys are the app's");
        assert_eq!(b.due_at(), Some(T0 + 150 + 4000));
        assert!(b.update(&Input::Tick, T0 + 4149).is_empty());
        assert_eq!(b.update(&Input::Tick, T0 + 4150), vec![Effect::Redraw]);
        assert!(!b.up());
        // a receiver says eARC; an input with nothing on it is the port alone
        b.update(&fg("com.webos.app.hdmi2", false), T0 + 5000);
        b.update(&Input::Luna { tag: INPUTS_TAG, payload: list() }, T0 + 5050);
        assert_eq!((spec_of(&b).name.as_str(), spec_of(&b).facts.as_str()), ("HDMI 2 \u{b7} AVR-S760H", "eARC"));
        b.update(&fg("com.webos.app.hdmi3", false), T0 + 6000);
        b.update(&Input::Luna { tag: INPUTS_TAG, payload: list() }, T0 + 6050);
        assert_eq!((spec_of(&b).name.as_str(), spec_of(&b).icon.as_str()), ("HDMI 3", "unplug"), "the port alone, with the list's glyph for a port with nothing on it");
        // Live TV is the channel banner's: the plain pill (one word and the antenna) only when the tuner names no channel
        let fx = b.update(&fg("com.webos.app.livetv", false), T0 + 7000);
        assert!(fx.iter().any(|e| matches!(e, Effect::Luna { tag: channel_feed::LAST_TAG, subscribe: true, .. })), "the tuner is followed: {fx:?}");
        assert!(!fx.iter().any(|e| matches!(e, Effect::Luna { tag: INPUTS_TAG, .. })), "no input list is asked for: {fx:?}");
        assert!(!b.up());
        b.update(&Input::Tick, T0 + 7000 + channel_feed::NAME_WAIT_MS);
        assert_eq!((spec_of(&b).name.as_str(), spec_of(&b).facts.as_str(), spec_of(&b).icon.as_str()), ("Live TV", "Antenna", "antenna"));
        // the owner's switch off: LG's own banners draw the moment and this draws nothing of itself
        let mut off = Banner::new(&json!({}));
        assert!(off.update(&fg("com.webos.app.hdmi1", false), T0).is_empty() && !off.up());
    }

    #[test]
    fn an_app_that_is_not_an_input_a_list_that_never_comes_and_the_input_leaving_are_all_handled() {
        let mut b = on();
        assert!(b.update(&fg("netflix", false), T0).is_empty(), "not an input");
        b.update(&fg("com.webos.app.hdmi2", false), T0 + 100);
        // the list does not answer: the pill says the port
        assert_eq!(b.update(&Input::Tick, T0 + 100 + LIST_WAIT_MS), vec![Effect::Redraw]);
        assert_eq!(spec_of(&b).name, "HDMI 2");
        // the input leaves: its pill goes with it
        assert_eq!(b.update(&fg("com.webos.app.home", true), T0 + 2000), vec![Effect::Redraw]);
        assert!(!b.up());
        // an answer that comes after the app went finds nobody waiting
        b.update(&fg("com.webos.app.hdmi1", false), T0 + 3000);
        b.update(&fg("netflix", false), T0 + 3100);
        assert!(b.update(&Input::Luna { tag: INPUTS_TAG, payload: list() }, T0 + 3200).is_empty() && !b.up());
        // the screen going off ends what is up
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: json!({"kind": "input", "name": "HDMI 3"}) }, T0 + 4000);
        assert!(b.up());
        b.update(&Input::Topic { topic: "screen".into(), data: json!({"on": false}) }, T0 + 4100);
        assert!(!b.up());
    }

    #[test]
    fn a_banner_with_buttons_takes_the_keys_left_and_right_move_ok_picks_and_ends_it_and_back_ends_it() {
        let mut b = on();
        let spec = json!({"kind": "input", "icon": "gamepad-2", "name": "HDMI 3", "facts": "4K 120 Hz", "buttons": true});
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: spec }, T0);
        assert!(b.wants());
        assert_eq!(spec_of(&b).buttons, vec![Pick::Details, Pick::Inputs, Pick::Picture], "all three when the spec names none");
        assert_eq!(b.due_at(), Some(T0 + 8000), "eight seconds with buttons");
        let key = |b: &mut Banner, n: &str, at: u64| b.update(&Input::Key { name: n.into(), down: true, repeat: false }, at);
        key(&mut b, "left", T0 + 100);
        assert_eq!(b.view().unwrap().index, 0, "no wrap");
        key(&mut b, "right", T0 + 200);
        assert_eq!(b.view().unwrap().index, 1);
        assert_eq!(b.due_at(), Some(T0 + 200 + 8000), "a key keeps it a while longer");
        key(&mut b, "right", T0 + 300);
        key(&mut b, "right", T0 + 400);
        assert_eq!(b.view().unwrap().index, 2);
        let fx = key(&mut b, "ok", T0 + 500);
        assert!(!b.up(), "OK on Picture picks it and ends the banner");
        assert!(fx.contains(&Effect::Redraw));
        assert_eq!(b.take_intents(), vec![Intent::OpenQuick]);
        // Inputs is the input picker, Home goes Home, Details is only read
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: json!({"kind": "input", "name": "HDMI 3", "buttons": true, "actions": ["inputs", "home", "bogus"]}) }, T0 + 1000);
        assert_eq!(spec_of(&b).buttons, vec![Pick::Inputs, Pick::Home], "the buttons the surface can answer, a word it does not know left out");
        key(&mut b, "ok", T0 + 1100);
        assert_eq!(b.take_intents(), vec![Intent::OpenPicker]);
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: json!({"kind": "input", "name": "HDMI 3", "buttons": true, "actions": ["inputs", "home"]}) }, T0 + 2000);
        key(&mut b, "right", T0 + 2100);
        let fx = key(&mut b, "ok", T0 + 2200);
        assert!(fx.iter().any(|e| matches!(e, Effect::Luna { uri, payload, .. } if uri.ends_with("/launchDefaultApp") && payload["category"] == "home")));
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: json!({"kind": "input", "name": "HDMI 3", "buttons": true}) }, T0 + 3000);
        key(&mut b, "back", T0 + 3100);
        assert!(!b.up() && b.take_intents().is_empty(), "Back ends it and picks nothing");
        // a key that is not the pad's is not the banner's
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: json!({"kind": "input", "name": "HDMI 3", "buttons": true}) }, T0 + 4000);
        assert!(key(&mut b, "up", T0 + 4100).is_empty() && b.up());
        // a pill with no buttons takes no key at all
        b.update(&Input::Topic { topic: HIDE_TOPIC.into(), data: Value::Null }, T0 + 4200);
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: json!({"kind": "input", "name": "HDMI 3"}) }, T0 + 4300);
        assert!(!b.wants() && key(&mut b, "ok", T0 + 4400).is_empty() && b.up());
    }

    #[test]
    fn no_signal_is_black_with_inputs_and_home_stays_until_picked_and_hops_to_another_spot_every_minute() {
        let mut b = on();
        let spec = json!({"kind": "nosignal", "icon": "unplug", "port": "HDMI 4", "title": "Nothing is connected here", "body": "Plug a device into HDMI 4."});
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: spec }, T0);
        let s = spec_of(&b);
        assert_eq!((s.kind, s.buttons.clone(), s.port.as_str()), (Kind::NoSignal, vec![Pick::Inputs, Pick::Home], "HDMI 4"));
        assert!(b.wants());
        assert_eq!(b.due_at(), Some(T0 + SPOT_MS), "no time of its own: only the spot's turn");
        assert!(b.update(&Input::Tick, T0 + SPOT_MS - 1).is_empty());
        assert_eq!(b.update(&Input::Tick, T0 + SPOT_MS), vec![Effect::Redraw]);
        assert_eq!(b.view().unwrap().spot, 1);
        for i in 2..=6 {
            b.update(&Input::Tick, T0 + SPOT_MS * i);
        }
        assert_eq!(b.view().unwrap().spot, 6 % SPOTS.len(), "round the five spots");
        assert!(b.up(), "it never goes by itself");
        // the pointer picks a button
        let fx = b.picked(1);
        assert!(fx.iter().any(|e| matches!(e, Effect::Luna { tag: HOME_TAG, .. })) && !b.up());
    }

    #[test]
    fn the_channel_banner_reads_its_fields_and_a_spec_that_is_none_of_the_three_is_nothing() {
        let mut b = on();
        let spec = json!({"kind": "channel", "number": "7", "call": "KABC", "title": "Evening News", "sub": "6:00 PM - 6:30 PM", "progress": 1.4, "next": "Jeopardy!", "chips": ["1080i", "Stereo", "CC"]});
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: spec }, T0);
        let s = spec_of(&b);
        assert_eq!((s.number.as_str(), s.call.as_str(), s.title.as_str(), s.next.as_str(), s.progress), ("7", "KABC", "Evening News", "Jeopardy!", 1.0), "the bar is a fraction");
        assert_eq!(s.chips, vec!["1080i", "Stereo", "CC"]);
        assert!(!b.wants());
        assert_eq!(b.due_at(), Some(T0 + HOLD_CHANNEL_MS), "LG's five seconds");
        b.update(&Input::Topic { topic: HIDE_TOPIC.into(), data: Value::Null }, T0 + 10);
        assert!(b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: json!({"kind": "guide"}) }, T0 + 20).is_empty() && !b.up());
        assert!(is_input_app("com.webos.app.hdmi4") && is_input_app("com.webos.app.livetv") && is_input_app("com.webos.app.dp1") && is_input_app("com.webos.app.usbc2"));
        assert!(!is_input_app("com.webos.app.hdmi10") && !is_input_app("com.webos.app.home") && !is_input_app("netflix"));
    }

    const ID: &str = "1_32_2_1_0_0_0";
    const OTHER: &str = "1_32_9_1_0_0_0";

    fn tuner(id: &str) -> Input {
        Input::Luna { tag: channel_feed::LAST_TAG, payload: json!({"returnValue": true, "channelId": id}) }
    }

    /// What LG says of the channel and of what is on it, the programme on the air at `at` for twenty minutes more.
    fn answers(id: &str, at: u64) -> [Input; 2] {
        let stamp = |min: i64| channels::stamp(at as i64 + min * 60_000, 0);
        let programme = |name: &str, from: i64, to: i64| json!({"channelId": id, "programId": format!("{id}{from}"), "programName": name, "description": "About the programme.", "startTime": stamp(from), "endTime": stamp(to)});
        [
            Input::Luna { tag: channel_feed::INFO_TAG, payload: json!({"returnValue": true, "channelList": [{"channelId": id, "channelNumber": channel_feed::number_of(id), "channelName": "WSB-HD", "imgUrl": "", "channelTypeId": 1}]}) },
            Input::Luna { tag: channel_feed::NOW_TAG, payload: json!({"returnValue": true, "programList": [programme("Evening News", -10, 20), programme("Jeopardy!", 20, 50)]}) },
        ]
    }

    fn tune(b: &mut Banner, id: &str, at: u64) {
        b.update(&tuner(id), at);
        for (i, a) in answers(id, at).iter().enumerate() {
            b.update(a, at + 10 * (i as u64 + 1));
        }
    }

    fn key(b: &mut Banner, name: &str, at: u64) -> Vec<Effect> {
        b.update(&Input::Key { name: name.into(), down: true, repeat: false }, at)
    }

    #[test]
    fn live_tv_in_front_shows_the_channel_banner_at_a_launch_and_at_every_change_for_five_seconds_and_nothing_with_the_owners_banners_off() {
        let mut b = on();
        b.update(&fg("com.webos.app.livetv", false), T0);
        assert_eq!(b.takes(), vec!["info"], "INFO is the banner's while Live TV is in front");
        tune(&mut b, ID, T0 + 100);
        let s = spec_of(&b);
        assert_eq!((s.kind, s.number.as_str(), s.call.as_str(), s.title.as_str(), s.next.as_str()), (Kind::Channel, "2-1", "WSB-HD", "Evening News", "Jeopardy!"));
        assert!(!b.wants(), "no key but INFO and Back is the banner's");
        assert_eq!(b.due_at(), Some(T0 + 120 + HOLD_CHANNEL_MS));
        assert!(b.update(&Input::Tick, T0 + 120 + HOLD_CHANNEL_MS - 1).is_empty() && b.up());
        assert_eq!(b.update(&Input::Tick, T0 + 120 + HOLD_CHANNEL_MS), vec![Effect::Redraw]);
        assert!(!b.up(), "it goes on LG's timing");
        // the channel goes up: the banner shows again
        tune(&mut b, OTHER, T0 + 20_000);
        assert!(b.up() && spec_of(&b).number == "9-1", "the banner of the channel the tuner named");
        assert!(b.update(&Input::Tick, T0 + 20_000 + HOLD_CHANNEL_MS + 100).contains(&Effect::Redraw) && !b.up());
        // Live TV leaves: the tuner is let go of, and a banner that is up goes with it
        tune(&mut b, ID, T0 + 30_000);
        let fx = b.update(&fg("com.webos.app.home", true), T0 + 31_000);
        assert!(fx.contains(&Effect::Cancel { tag: channel_feed::LAST_TAG }) && fx.contains(&Effect::Redraw) && !b.up(), "{fx:?}");
        assert!(b.takes().is_empty());
        // the owner's banners off: LG's own draws the moment, and nothing of this follows the tuner
        let mut off = Banner::new(&json!({}));
        assert!(off.update(&fg("com.webos.app.livetv", false), T0).is_empty() && off.takes().is_empty());
        // turned on while Live TV is in front: it follows from then; turned off again, it lets go and takes the banner away
        let fx = off.set_theme(&json!({"banners": {"own": true}}), T0 + 10);
        assert!(fx.iter().any(|e| matches!(e, Effect::Luna { tag: channel_feed::LAST_TAG, subscribe: true, .. })), "{fx:?}");
        off.update(&tuner(ID), T0 + 20);
        let fx = off.set_theme(&json!({}), T0 + 30);
        assert!(fx.contains(&Effect::Cancel { tag: channel_feed::LAST_TAG }) && off.takes().is_empty());
    }

    fn mask_ops(fx: &[Effect]) -> Vec<String> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Channel { path, body } if path == "/v1/mask" => body["op"].as_str().map(str::to_string),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn lgs_own_banner_is_claimed_while_live_tv_is_in_front_and_given_back_when_it_is_not_or_the_tuner_fails() {
        let mut b = on();
        let fx = b.update(&fg("com.webos.app.livetv", false), T0);
        assert_eq!(mask_ops(&fx), ["claim", "engage"], "Live TV in front: LG's banner for a launch is not drawn, nor the ones for a channel key: {fx:?}");
        let fx = b.update(&tuner(ID), T0 + 10);
        assert_eq!(mask_ops(&fx), ["engage"], "the tuner named a channel: LG draws its banner for it now, so the blanking runs on from here: {fx:?}");
        for (i, a) in answers(ID, T0 + 10).iter().enumerate() {
            assert!(mask_ops(&b.update(a, T0 + 20 + 10 * i as u64)).is_empty(), "the answers of the record and the programmes change nothing of it");
        }
        assert!(b.up() && mask_ops(&b.update(&tuner(ID), T0 + 40)).is_empty(), "the same channel named again is no change");
        let at = Input::Luna { tag: lg_mask::POSITION_TAG, payload: json!({"returnValue": true, "settings": {"bannerPosition": "righttop"}}) };
        let fx = b.update(&at, T0 + 50);
        assert!(mask_ops(&fx) == ["claim"] && matches!(&fx[..], [Effect::Channel { body, .. }] if body["rects"][0]["x"] == 414), "the banner is where the owner put it: {fx:?}");
        let fx = b.update(&Input::Luna { tag: channel_feed::LAST_TAG, payload: json!({"returnValue": false, "errorCode": -106}) }, T0 + 100);
        assert_eq!(mask_ops(&fx), ["release"], "a tuner that cannot be followed leaves LG's banner alone: {fx:?}");
        assert!(mask_ops(&b.update(&fg("com.webos.app.home", true), T0 + 200)).is_empty(), "nothing is left to give back");
        assert_eq!(mask_ops(&b.update(&fg("com.webos.app.livetv", false), T0 + 300)), ["claim", "engage"], "the next visit claims again");
        assert_eq!(mask_ops(&b.update(&fg("com.webos.app.hdmi1", false), T0 + 400)), ["release"], "another app in front: LG's banner is its own");
        b.update(&fg("com.webos.app.livetv", false), T0 + 500);
        assert_eq!(mask_ops(&b.set_theme(&json!({}), T0 + 600)), ["release"], "the owner's banners off: LG's own draws the moment");
        assert!(mask_ops(&Banner::new(&json!({})).update(&fg("com.webos.app.livetv", false), T0)).is_empty(), "never claimed with the switch off");
    }

    #[test]
    fn live_tv_is_let_go_of_with_the_screen_off_and_followed_and_blanked_again_from_the_moment_it_is_on() {
        let screen = |on: bool| Input::Topic { topic: "screen".into(), data: json!({"on": on}) };
        let mut b = on();
        b.update(&fg("com.webos.app.livetv", false), T0);
        tune(&mut b, ID, T0 + 100);
        let fx = b.update(&screen(false), T0 + 1_000);
        assert!(fx.contains(&Effect::Cancel { tag: channel_feed::LAST_TAG }) && mask_ops(&fx) == ["release"] && !b.up() && b.takes().is_empty(), "{fx:?}");
        let fx = b.update(&screen(true), T0 + 60_000);
        assert_eq!(mask_ops(&fx), ["claim", "engage"], "LG's banner of the wake is blanked: {fx:?}");
        assert!(fx.iter().any(|e| matches!(e, Effect::Luna { tag: channel_feed::LAST_TAG, subscribe: true, .. })) && b.takes() == vec!["info"], "the tuner is followed again: {fx:?}");
        tune(&mut b, ID, T0 + 60_100);
        assert!(b.up(), "the channel on the wake has its banner, as at a launch");
        // the screen coming on with Home in front, or with no foreground yet, follows nothing
        let mut home = on();
        home.update(&fg("com.webos.app.home", true), T0);
        assert!(home.update(&screen(false), T0 + 10).is_empty() && home.update(&screen(true), T0 + 20).is_empty());
    }

    #[test]
    fn info_shows_the_detail_for_eight_seconds_and_info_or_back_ends_it_and_a_banner_the_owner_raised_is_left_alone() {
        let mut b = on();
        b.update(&fg("com.webos.app.livetv", false), T0);
        assert!(key(&mut b, "info", T0 + 50).is_empty(), "nothing is known of a channel before the tuner names one");
        tune(&mut b, ID, T0 + 100);
        let fx = key(&mut b, "info", T0 + 2000);
        assert!(fx.contains(&Effect::Redraw));
        let s = spec_of(&b);
        assert!(s.info && s.detail == "About the programme." && s.hold_ms == HOLD_INFO_MS, "{s:?}");
        assert_eq!(b.due_at(), Some(T0 + 2000 + HOLD_INFO_MS));
        assert_eq!(b.takes(), vec!["info", "back"], "Back ends this banner as it ends LG's INFO card");
        assert_eq!(key(&mut b, "back", T0 + 3000), vec![Effect::Redraw]);
        assert!(!b.up() && b.takes() == vec!["info"]);
        key(&mut b, "info", T0 + 4000);
        assert!(b.up());
        assert_eq!(key(&mut b, "info", T0 + 4500), vec![Effect::Redraw], "INFO again ends it");
        assert!(!b.up());
        // Back with no INFO banner up is LG's: previous channel
        assert!(key(&mut b, "back", T0 + 5000).is_empty());
        // a banner the owner of the moment raised is not touched by a late answer of the feed
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: json!({"kind": "channel", "number": "9", "title": "Raised"}) }, T0 + 6000);
        let late = answers(ID, T0)[0].clone();
        assert!(b.update(&late, T0 + 6100).is_empty() && spec_of(&b).title == "Raised");
    }
}
