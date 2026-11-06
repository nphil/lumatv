//! A row of a Settings page as data (`settings/PageList.qml`'s descriptors, evaluated): its kind, its words, its state, and the
//! operations its owner's key sets off. A page is a function from what the shell knows ([`super::ctx::Ctx`]) to rows, run again
//! whenever that changes, so a row never holds a function that reads the world: it holds what was true when the page was asked.
//! A row's `act` holds what to do, not what is on screen.

use serde_json::{Map, Value, json};
use std::rc::Rc;

/// What a key or a click gives an action.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Press,
    Toggle(bool),
    Index(usize),
    Value(f64),
    /// Left (-1) or right (1) on a row whose value is read back.
    Nudge(i32),
}

pub type Act = Rc<dyn Fn(Arg) -> Vec<Op>>;
/// How a pick of a list is written (the operations the chosen value makes).
pub type Writer = dyn Fn(&str) -> Vec<Op>;

/// The kinds of row (`kind` of a descriptor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Toggle,
    Seg,
    Slider,
    Value,
    Action,
    Cards,
    Stepper,
    Nudge,
    Head,
    Figure,
    /// A code to scan with a phone (a QR code) and its words.
    Code,
    Empty,
    Note,
}

/// One card of a row of cards (the mode choosers).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Card {
    pub title: String,
    pub icon: String,
    pub sub: String,
    pub face: Option<Face>,
}

/// What the owner asked, answered with what happens (a question with the safe answer first, as the confirm dialog asks it).
#[derive(Clone)]
pub struct Answer {
    pub id: String,
    pub label: String,
    pub danger: bool,
    pub then: Vec<Op>,
}

#[derive(Clone)]
pub struct Ask {
    pub icon: String,
    pub title: String,
    pub body: String,
    pub answers: Vec<Answer>,
    pub focus: usize,
}

impl Ask {
    /// The usual question: the safe answer first (Back means it), then the one that does `then`.
    pub fn yes_no(icon: &str, title: &str, body: &str, yes: &str, no: &str, danger: bool, then: Vec<Op>) -> Self {
        Ask {
            icon: icon.into(),
            title: title.into(),
            body: body.into(),
            answers: vec![Answer { id: "no".into(), label: no.into(), danger: false, then: Vec::new() }, Answer { id: "yes".into(), label: yes.into(), danger, then }],
            focus: 0,
        }
    }
}

/// A page's argument: what it was opened for (`host.openPage(id, arg)`).
#[derive(Clone, Default)]
pub enum PageArg {
    #[default]
    None,
    Json(Value),
}

impl PageArg {
    /// A string of what the page was opened for.
    pub fn str(&self, key: &str) -> Option<&str> {
        match self {
            PageArg::Json(v) => v.get(key).and_then(Value::as_str),
            PageArg::None => None,
        }
    }
}

/// The small picker behind a choice row: the TV's own values for the key (or the options a page offers), each in its name, the one in force
/// marked, and how a pick is written.
pub struct ChoiceSpec {
    pub title: String,
    pub options: Vec<Pick>,
    pub current: String,
    pub write: Box<Writer>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Pick {
    pub value: String,
    pub label: String,
    pub enabled: bool,
    /// A Lucide icon before the label, and a colour swatch (`#RRGGBB`) in place of one.
    pub icon: String,
    pub swatch: String,
}

impl Pick {
    pub fn new(value: &str, label: &str) -> Self {
        Pick { value: value.into(), label: label.into(), enabled: true, icon: String::new(), swatch: String::new() }
    }
}

/// What a row would put on the page's picture: its glyph, and how (the id of the answer it holds).
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub icon: String,
    pub choice: String,
}

/// A person's picture on a tile or a card: the bundled picture or the photo when there is a file, else the letter on the colour.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Face {
    pub file: Option<String>,
    pub letter: String,
    /// `#RRGGBB`.
    pub colour: String,
}

/// What a row does when it is used. The model carries them out in order.
#[derive(Clone)]
pub enum Op {
    /// `Sio.write`: one key of LG's settings, shown at once.
    Lg { ds: String, value: Value, app: Option<String> },
    /// `Sio.writeMany`: several keys of one category in one call.
    LgMany { category: String, settings: Map<String, Value>, app: Option<String> },
    /// A luna call with no answer wanted (`luna.act`).
    Act { uri: String, payload: Value },
    /// A luna call whose answer lands in the page's documents under `name` (`luna.call`, `luna.subscribe`).
    Call { name: String, uri: String, payload: Value, subscribe: bool },
    /// Lets go of a subscription made with [`Op::Call`].
    Close(String),
    /// A request to the helper, the answer kept under `name` (`Helper.request`, `Helper.post`).
    Send { name: Option<String>, method: &'static str, path: String, body: Option<Value> },
    /// A change to one key of a helper owner.
    Owner { owner: String, key: String, value: Value },
    /// A patch of the theme: applied live and written to the owner's theme.json.
    Patch(Value),
    /// Confirm a gallery theme only after its write succeeds.
    ThemeChoice(Value),
    /// A key of the page's own state.
    Set(String, Value),
    /// Another page (`host.openPage`).
    Open { page: String, arg: PageArg },
    /// Back to the page this one was opened from (`host.up`).
    Up,
    /// A question; its answers carry what happens.
    Ask(Ask),
    /// The small picker: a list of options by the row that asked, a pick writes and closes.
    Pick(Rc<ChoiceSpec>),
    /// Text from the person (the keyboard); what is typed becomes the operations `then` makes.
    Entry { prompt: String, text: String, secret: bool, purpose: u32, then: Rc<dyn Fn(String) -> Vec<Op>> },
    /// LG's Settings app (or another), at a screen of its own; this Settings closes.
    Launch { app: String, params: Value },
    /// The shell is asked for something outside Settings (the layout editor).
    Shell(ShellRequest),
    /// `then` is done after `ms` of quiet: another `Later` with the same key replaces it (a key held down sends one step, not each).
    Later { key: String, ms: u64, then: Vec<Op> },
    /// Sets the sleep timer to the choice `i` of LG's list.
    Sleep(usize),
    /// The focus goes to the row of this id (the answer in force, in a chooser), when the page has it and it takes the focus.
    Focus(String),
    /// Reads what the page reads when it opens, now.
    Refresh,
    /// The same, after this many milliseconds (the page gives the service a moment to apply a change).
    RefreshIn(u64),
    /// Leaves Settings.
    Leave,
}

/// What Settings asks the shell for.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellRequest {
    /// Closes the Settings kind.
    Close,
    /// Launches an app through the application manager.
    Launch { app: String, params: Value },
    /// Opens the layout editor of the pop-ups (kind `layout`), at an item when one is named.
    Layout { item: Option<String> },
    /// Home's layout is now this one (a card shown, sized or moved): Home writes the owner's `layout.json` and draws it at once.
    HomeLayout(Value),
    /// Closes Settings and opens Home's layout editor.
    EditHome,
    /// Opens Home's wallpaper editor.
    Wallpaper,
    /// Opens the display lab (kind `displaylab`) over Settings, which stays where it was for the way back.
    DisplayLab,
    /// Asks the keyboard for text (a Wi-Fi password, the TV's name); the answer comes back as `entered` or `entry_cancelled`.
    Text { prompt: String, text: String, secret: bool, purpose: u32 },
}

impl std::fmt::Debug for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Op::Lg { ds, value, app } => write!(f, "Lg({ds}={value} app={app:?})"),
            Op::LgMany { category, settings, .. } => write!(f, "LgMany({category} {settings:?})"),
            Op::Act { uri, payload } => write!(f, "Act({uri} {payload})"),
            Op::Call { name, uri, payload, subscribe } => {
                // a Wi-Fi password must not reach a log through a debug print of the ops
                let mut shown = payload.clone();
                if let Some(key) = shown.pointer_mut("/security/simpleSecurity/passKey") {
                    *key = json!("***");
                }
                write!(f, "Call({name} {uri} {shown} sub={subscribe})")
            }
            Op::Close(n) => write!(f, "Close({n})"),
            Op::Send { name, method, path, body } => write!(f, "Send({name:?} {method} {path} {body:?})"),
            Op::Owner { owner, key, value } => write!(f, "Owner({owner}.{key}={value})"),
            Op::Patch(p) => write!(f, "Patch({p})"),
            Op::ThemeChoice(p) => write!(f, "ThemeChoice({p})"),
            Op::Set(k, v) => write!(f, "Set({k}={v})"),
            Op::Open { page, .. } => write!(f, "Open({page})"),
            Op::Up => write!(f, "Up"),
            Op::Ask(a) => write!(f, "Ask({})", a.title),
            Op::Sleep(i) => write!(f, "Sleep({i})"),
            Op::Later { key, ms, then } => write!(f, "Later({key} {ms} {then:?})"),
            Op::Pick(c) => write!(f, "Pick({})", c.title),
            Op::Entry { prompt, .. } => write!(f, "Entry({prompt})"),
            Op::Launch { app, params } => write!(f, "Launch({app} {params})"),
            Op::Shell(r) => write!(f, "Shell({r:?})"),
            Op::Focus(id) => write!(f, "Focus({id})"),
            Op::Refresh => write!(f, "Refresh"),
            Op::RefreshIn(ms) => write!(f, "RefreshIn({ms})"),
            Op::Leave => write!(f, "Leave"),
        }
    }
}

/// A row, as the page's function evaluated it.
#[derive(Clone)]
pub struct Row {
    pub id: String,
    pub kind: Kind,
    pub tab: usize,
    pub icon: String,
    /// A person's picture in place of the icon's tile.
    pub face: Option<Face>,
    pub label: String,
    pub detail: String,
    /// The sentence behind the row's info mark; empty draws no mark.
    pub help: String,
    pub value: String,
    /// A small glyph before the value (the answer a row holds, as a mark: `eye-off`).
    pub value_icon: String,
    /// The widest word a chip of this kind of row holds, so every chip of a list is as wide as the widest.
    pub value_min: String,
    /// What the row would put on the page's figure while it is focused or its picker is open.
    pub preview: Option<Preview>,
    /// The words of a head, a note, an empty tab and a figure.
    pub text: String,
    pub enabled: bool,
    pub visible: bool,
    /// A row that only shows (full strength, never focused unless it has help).
    pub info: bool,
    pub danger: bool,
    pub selected: bool,
    /// A lighter wash that stays without focus (the rail's current section): the chosen look of a row without the check of a choice.
    pub wash: bool,
    pub chevron: bool,
    /// The tile in the accent.
    pub accent: bool,
    /// Owner-only: locked for a profile that may not change the household's settings.
    pub admin: bool,
    pub on: bool,
    pub options: Vec<String>,
    pub index: Option<usize>,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub num: f64,
    pub value_text: Option<String>,
    pub unit: String,
    pub cards: Vec<Card>,
    pub cols: usize,
    pub strip: bool,
    pub in_view: usize,
    pub button: String,
    /// A figure's lit edges: top, bottom, left, right.
    pub zones: [bool; 4],
    pub act: Option<Act>,
}

impl std::fmt::Debug for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Row({:?} {} {:?} on={} idx={:?} enabled={} visible={})", self.kind, self.id, self.label, self.on, self.index, self.enabled, self.visible)
    }
}

impl Row {
    pub fn new(kind: Kind, id: &str) -> Self {
        Row {
            id: id.to_string(),
            kind,
            tab: 0,
            icon: String::new(),
            face: None,
            label: String::new(),
            detail: String::new(),
            help: String::new(),
            value: String::new(),
            value_icon: String::new(),
            value_min: String::new(),
            preview: None,
            text: String::new(),
            enabled: true,
            visible: true,
            info: false,
            danger: false,
            selected: false,
            wash: false,
            chevron: true,
            accent: false,
            admin: false,
            on: false,
            options: Vec::new(),
            index: None,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            num: 0.0,
            value_text: None,
            unit: String::new(),
            cards: Vec::new(),
            cols: 4,
            strip: false,
            in_view: 4,
            button: "Run".into(),
            zones: [false; 4],
            act: None,
        }
    }

    pub fn toggle(id: &str) -> Self {
        Self::new(Kind::Toggle, id)
    }

    pub fn seg(id: &str, options: &[&str]) -> Self {
        let mut r = Self::new(Kind::Seg, id);
        r.options = options.iter().map(|s| s.to_string()).collect();
        r
    }

    pub fn slider(id: &str, min: f64, max: f64, step: f64) -> Self {
        let mut r = Self::new(Kind::Slider, id);
        (r.min, r.max, r.step) = (min, max, step);
        r
    }

    /// A row that shows a value and a chevron and does something on OK.
    pub fn link(id: &str) -> Self {
        Self::new(Kind::Value, id)
    }

    /// A row that only shows: LG's name and a value.
    pub fn info(id: &str) -> Self {
        let mut r = Self::new(Kind::Value, id);
        r.info = true;
        r.chevron = false;
        r
    }

    pub fn action(id: &str, button: &str) -> Self {
        let mut r = Self::new(Kind::Action, id);
        r.button = button.into();
        r
    }

    pub fn stepper(id: &str, options: &[String]) -> Self {
        let mut r = Self::new(Kind::Stepper, id);
        r.options = options.to_vec();
        r
    }

    pub fn nudge(id: &str) -> Self {
        Self::new(Kind::Nudge, id)
    }

    pub fn head(text: &str, tab: usize) -> Self {
        let mut r = Self::new(Kind::Head, &format!("head:{text}"));
        r.text = text.into();
        r.tab = tab;
        r
    }

    pub fn note(text: &str, tab: usize) -> Self {
        let mut r = Self::new(Kind::Note, &format!("note:{text}"));
        r.text = text.into();
        r.tab = tab;
        r
    }

    /// A small TV with its edges lit (top, bottom, left, right) and a caption beside it; shown, never focused.
    pub fn figure(id: &str, text: &str, zones: [bool; 4], tab: usize) -> Self {
        let mut r = Self::new(Kind::Figure, id);
        r.text = text.into();
        r.zones = zones;
        r.tab = tab;
        r
    }

    /// A QR code of `scan` (an address) with `code` large beside it and `detail` under it (where to type the code instead); shown, never
    /// focused.
    pub fn code(id: &str, scan: &str, code: &str, detail: &str, tab: usize) -> Self {
        let mut r = Self::new(Kind::Code, id);
        r.text = scan.into();
        r.label = code.into();
        r.detail = detail.into();
        r.tab = tab;
        r
    }

    /// What a tab has nothing to list: a glyph tile, a title and a line under it.
    pub fn empty(title: &str, text: &str, icon: &str, tab: usize) -> Self {
        let mut r = Self::new(Kind::Empty, &format!("empty:{title}"));
        r.label = title.into();
        r.text = text.into();
        r.icon = icon.into();
        r.tab = tab;
        r
    }

    pub fn cards(id: &str, cards: Vec<Card>) -> Self {
        let mut r = Self::new(Kind::Cards, id);
        r.cards = cards;
        r
    }

    pub fn tab(mut self, t: usize) -> Self {
        self.tab = t;
        self
    }

    pub fn icon(mut self, icon: &str) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn face(mut self, face: Face) -> Self {
        self.face = Some(face);
        self
    }

    pub fn label(mut self, label: &str) -> Self {
        self.label = label.into();
        self
    }

    pub fn detail(mut self, detail: &str) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn help(mut self, help: &str) -> Self {
        self.help = help.into();
        self
    }

    pub fn value(mut self, value: &str) -> Self {
        self.value = value.into();
        self
    }

    pub fn value_icon(mut self, icon: &str) -> Self {
        self.value_icon = icon.into();
        self
    }

    pub fn value_min(mut self, widest: &str) -> Self {
        self.value_min = widest.into();
        self
    }

    pub fn preview(mut self, icon: &str, choice: &str) -> Self {
        self.preview = Some(Preview { icon: icon.into(), choice: choice.into() });
        self
    }

    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    pub fn index(mut self, index: Option<usize>) -> Self {
        self.index = index;
        self
    }

    pub fn num(mut self, num: f64) -> Self {
        self.num = num;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn chevron(mut self, chevron: bool) -> Self {
        self.chevron = chevron;
        self
    }

    pub fn admin(mut self) -> Self {
        self.admin = true;
        self
    }

    pub fn unit(mut self, unit: &str) -> Self {
        self.unit = unit.into();
        self
    }

    pub fn value_text(mut self, text: &str) -> Self {
        self.value_text = Some(text.into());
        self
    }

    pub fn act(mut self, act: impl Fn(Arg) -> Vec<Op> + 'static) -> Self {
        self.act = Some(Rc::new(act));
        self
    }

    /// Does `ops` on OK, whatever the argument (a link, an action).
    pub fn does(self, ops: Vec<Op>) -> Self {
        self.act(move |_| ops.clone())
    }

    /// The row as PageList's rules take it for the D-pad: shown, not a head, a note, an empty tab, a figure or a code, not a row that only
    /// shows (unless it has help: the focus can reach its info mark), and enabled.
    pub fn focusable(&self) -> bool {
        self.visible && !matches!(self.kind, Kind::Head | Kind::Note | Kind::Empty | Kind::Figure | Kind::Code) && (!self.info || !self.help.is_empty()) && self.enabled
    }

    /// Whether Left and Right mean something on the row (a segmented row, a stepper, cards, a slider, a nudge).
    pub fn sideways(&self) -> bool {
        matches!(self.kind, Kind::Seg | Kind::Cards | Kind::Stepper | Kind::Slider | Kind::Nudge)
    }

    /// How many options the row holds for the inner focus.
    pub fn count(&self) -> usize {
        match self.kind {
            Kind::Cards => self.cards.len(),
            _ => self.options.len(),
        }
    }

    /// The text of a value row as the page shows it: the value, or a dash for nothing.
    pub fn shown_value(&self) -> &str {
        &self.value
    }
}

/// `{"k": v}` as a settings map.
pub fn settings(pairs: &[(&str, Value)]) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    m
}

/// The theme patch `{"a": {"b": v}}` for the path `a.b`.
pub fn patch_of(path: &str, v: Value) -> Value {
    path.rsplit('.').fold(v, |acc, key| json!({ key: acc }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_takes_the_focus_as_the_pages_rules_say() {
        let t = Row::toggle("a").label("A");
        assert!(t.focusable());
        assert!(!t.clone().enabled(false).focusable(), "a dimmed row takes no focus");
        assert!(!t.clone().visible(false).focusable());
        assert!(!Row::head("Vision", 0).focusable() && !Row::note("n", 0).focusable());
        let info = Row::info("i").label("Model");
        assert!(!info.focusable(), "a row that only shows is skipped");
        assert!(info.clone().help("Why").focusable(), "unless the focus can reach its info mark");
    }

    #[test]
    fn a_theme_patch_nests_by_the_dotted_path() {
        assert_eq!(patch_of("clock.twelveHour", json!(true)), json!({"clock": {"twelveHour": true}}));
        assert_eq!(patch_of("accent", json!("tile")), json!({"accent": "tile"}));
    }
}
