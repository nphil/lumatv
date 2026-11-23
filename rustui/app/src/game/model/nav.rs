//! The navigation contract the slices code against. F0 fixes the types; F1 owns the logic behind them (layer stack, region memory,
//! the Back ladder, hints).
use super::{Command, Focus, Key, Layer, Model, Page, accounts, game, home, launch, library, settings, text};
use lumagame_core::ids::GameId;

/// One screen's rules. Each slice implements it once, and [`Owner`] dispatches to it on the open layer, else the page.
pub trait Slice {
    /// The focus grid, one `Vec` per row: the page's rows when `layer` is `None`, else the layer's.
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>>;
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>);
    /// Left or Right on a control that takes a value. False when `f` takes none, so the key moves focus instead.
    fn adjust(m: &mut Model, f: Focus, forward: bool, out: &mut Vec<Command>) -> bool;
    /// The first focus of a region with no memory.
    fn default_focus(m: &Model, region: Region) -> Option<Focus> {
        match region {
            Region::Top => Some(Focus::Nav(0)),
            Region::Page => Self::rows(m, None).into_iter().flatten().next(),
            Region::Layer => Self::rows(m, m.top_layer().copied()).into_iter().flatten().next(),
        }
    }
    /// Where Back goes before it leaves the screen.
    fn anchor(_m: &Model) -> Option<Focus> {
        None
    }
    /// The highlighted game, for the quick menu.
    fn subject(_m: &Model) -> Option<GameId> {
        None
    }
    /// At most three: a key and one word each.
    fn hints(_m: &Model) -> Hints {
        Hints::default()
    }
}
/// Where focus can rest: the top bar, the page under it, or the open layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Top,
    Page,
    Layer,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hints(pub Vec<(Key, &'static str)>);
/// What the owner pressed last, which hints draw the glyphs of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Device {
    #[default]
    Remote,
    Pad,
}
/// One timer for anything that leaves after `ms` without a key: the in-game strip, the viewer's caption, the end-session confirm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoHide {
    ms: u64,
    until: u64,
}
impl AutoHide {
    pub const fn new(ms: u64) -> Self {
        Self { ms, until: 0 }
    }
    pub fn show(&mut self, now_ms: u64) {
        self.until = now_ms + self.ms;
    }
    pub fn visible(&self, now_ms: u64) -> bool {
        now_ms < self.until
    }
    /// A key while hidden only shows it again: true when the key is used up that way.
    pub fn swallow(&mut self, now_ms: u64) -> bool {
        let hidden = !self.visible(now_ms);
        self.show(now_ms);
        hidden
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Neighbours {
    pub id: Focus,
    pub up: Option<Focus>,
    pub down: Option<Focus>,
    pub left: Option<Focus>,
    pub right: Option<Focus>,
}
/// The slice whose rules apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Home,
    Library,
    Game,
    Media,
    Settings,
    Accounts,
    Launch,
    Text,
}
impl Owner {
    pub fn of_page(page: Page) -> Self {
        match page {
            Page::Home => Self::Home,
            Page::Library | Page::Search => Self::Library,
            Page::Hub => Self::Game,
            Page::Media => Self::Media,
            Page::Settings | Page::Lab | Page::GameSir | Page::Leadjoy => Self::Settings,
            Page::Accounts => Self::Accounts,
            Page::Launch | Page::Running | Page::Overlay => Self::Launch,
        }
    }
    pub fn of_layer(layer: Layer) -> Self {
        match layer {
            Layer::Chooser => Self::Game,
            Layer::Sort | Layer::Genre => Self::Library,
            Layer::Theme | Layer::Delete => Self::Settings,
            Layer::Auth => Self::Accounts,
            Layer::EndSession => Self::Launch,
            Layer::AddHost | Layer::EditText => Self::Text,
        }
    }
    /// The open layer's slice, else the page's.
    pub fn of(m: &Model) -> Self {
        m.top_layer().map_or(Self::of_page(m.view.page), |layer| Self::of_layer(*layer))
    }
    pub fn rows(self, m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        match self {
            Self::Home => home::Rules::rows(m, layer),
            Self::Library => library::Rules::rows(m, layer),
            Self::Game => game::Rules::rows(m, layer),
            Self::Media => game::media::Rules::rows(m, layer),
            Self::Settings => settings::Rules::rows(m, layer),
            Self::Accounts => accounts::Rules::rows(m, layer),
            Self::Launch => launch::Rules::rows(m, layer),
            Self::Text => text::Rules::rows(m, layer),
        }
    }
    pub fn activate(self, m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        match self {
            Self::Home => home::Rules::activate(m, f, out),
            Self::Library => library::Rules::activate(m, f, out),
            Self::Game => game::Rules::activate(m, f, out),
            Self::Media => game::media::Rules::activate(m, f, out),
            Self::Settings => settings::Rules::activate(m, f, out),
            Self::Accounts => accounts::Rules::activate(m, f, out),
            Self::Launch => launch::Rules::activate(m, f, out),
            Self::Text => text::Rules::activate(m, f, out),
        }
    }
    pub fn adjust(self, m: &mut Model, f: Focus, forward: bool, out: &mut Vec<Command>) -> bool {
        match self {
            Self::Home => home::Rules::adjust(m, f, forward, out),
            Self::Library => library::Rules::adjust(m, f, forward, out),
            Self::Game => game::Rules::adjust(m, f, forward, out),
            Self::Media => game::media::Rules::adjust(m, f, forward, out),
            Self::Settings => settings::Rules::adjust(m, f, forward, out),
            Self::Accounts => accounts::Rules::adjust(m, f, forward, out),
            Self::Launch => launch::Rules::adjust(m, f, forward, out),
            Self::Text => text::Rules::adjust(m, f, forward, out),
        }
    }
}
