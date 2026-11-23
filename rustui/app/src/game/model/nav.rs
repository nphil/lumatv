//! The navigation contract the slices code against: the one trait a screen implements, the [`Owner`] that dispatches to it, and what
//! navigation keeps between keys (where focus last rested in each region, which device the hints show, the auto-hide timer).
use super::{Command, Focus, Key, Layer, Model, Page, accounts, game, home, launch, library, quick, settings, text};
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
            Region::Top => Some(Focus::Nav(m.tab())),
            Region::Page => Self::rows(m, None).into_iter().flatten().next(),
            Region::Layer => Self::rows(m, m.view.top()).into_iter().flatten().next(),
        }
    }
    /// Where Back goes before it leaves the screen.
    fn anchor(_m: &Model) -> Option<Focus> {
        None
    }
    /// The bumpers over the screen's own peers: its sections or its library views, `forward` for the next. False when it has none, so
    /// the top bar's places step instead.
    fn step(_m: &mut Model, _forward: bool, _out: &mut Vec<Command>) -> bool {
        false
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
impl Focus {
    /// The region the focus rests in. Under an open layer (`layered`) only the layer's own rows exist, so every focus is the layer's:
    /// the same `Keyboard` row is the page's on the search page and the layer's in the text sheets.
    pub fn region(self, layered: bool) -> Region {
        match self {
            Self::Nav(_) => Region::Top,
            _ if layered => Region::Layer,
            _ => Region::Page,
        }
    }
}
/// Where focus last rested in the top bar and in the page, so that going back into one resumes it. A layer needs none: the layer
/// stack keeps the focus each one covered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    top: Option<Focus>,
    page: Option<Focus>,
}
impl Memory {
    pub fn recall(&self, region: Region) -> Option<Focus> {
        match region {
            Region::Top => self.top,
            Region::Page => self.page,
            Region::Layer => None,
        }
    }
    pub fn keep(&mut self, region: Region, focus: Focus) {
        match region {
            Region::Top => self.top = Some(focus),
            Region::Page => self.page = Some(focus),
            Region::Layer => {}
        }
    }
    pub fn forget(&mut self) {
        *self = Self::default();
    }
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
    /// Ends the showing now, unless it ends sooner of itself.
    pub fn hide(&mut self, now_ms: u64) {
        self.until = self.until.min(now_ms);
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
    /// Milliseconds until it hides, while it is shown.
    pub fn left(&self, now_ms: u64) -> Option<u64> {
        self.until.checked_sub(now_ms).filter(|left| *left > 0)
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
    Quick,
}
/// Calls the slice behind an [`Owner`].
macro_rules! slice {
    ($owner:expr, $call:ident($($arg:expr),*)) => {
        match $owner {
            Owner::Home => home::Rules::$call($($arg),*),
            Owner::Library => library::Rules::$call($($arg),*),
            Owner::Game => game::Rules::$call($($arg),*),
            Owner::Media => game::media::Rules::$call($($arg),*),
            Owner::Settings => settings::Rules::$call($($arg),*),
            Owner::Accounts => accounts::Rules::$call($($arg),*),
            Owner::Launch => launch::Rules::$call($($arg),*),
            Owner::Text => text::Rules::$call($($arg),*),
            Owner::Quick => quick::Rules::$call($($arg),*),
        }
    };
}
impl Owner {
    pub fn of_page(page: Page) -> Self {
        match page {
            Page::Home => Self::Home,
            Page::Library => Self::Library,
            Page::Game => Self::Game,
            Page::Settings => Self::Settings,
            Page::Launch | Page::Running => Self::Launch,
        }
    }
    pub fn of_layer(layer: Layer) -> Self {
        match layer {
            Layer::Chooser | Layer::Section(_) | Layer::Reader => Self::Game,
            Layer::Viewer => Self::Media,
            Layer::Sort | Layer::Genre | Layer::PlaysOn => Self::Library,
            Layer::Delete => Self::Settings,
            Layer::Auth | Layer::SignOut => Self::Accounts,
            Layer::Picture | Layer::Achievements | Layer::EndConfirm => Self::Launch,
            Layer::AddHost | Layer::EditText => Self::Text,
            Layer::Quick | Layer::QuickSettings => Self::Quick,
        }
    }
    /// The open layer's slice, else the page's.
    pub fn of(m: &Model) -> Self {
        m.view.top().map_or(Self::of_page(m.view.page), Self::of_layer)
    }
    pub fn rows(self, m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        slice!(self, rows(m, layer))
    }
    pub fn activate(self, m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        slice!(self, activate(m, f, out))
    }
    pub fn adjust(self, m: &mut Model, f: Focus, forward: bool, out: &mut Vec<Command>) -> bool {
        slice!(self, adjust(m, f, forward, out))
    }
    pub fn default_focus(self, m: &Model, region: Region) -> Option<Focus> {
        slice!(self, default_focus(m, region))
    }
    pub fn anchor(self, m: &Model) -> Option<Focus> {
        slice!(self, anchor(m))
    }
    pub fn step(self, m: &mut Model, forward: bool, out: &mut Vec<Command>) -> bool {
        slice!(self, step(m, forward, out))
    }
    pub fn subject(self, m: &Model) -> Option<GameId> {
        slice!(self, subject(m))
    }
    pub fn hints(self, m: &Model) -> Hints {
        slice!(self, hints(m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::{key, launching, report};
    use crate::game::model::{Input, LEAVE_MS};
    use lumagame_core::ipc::SessionState;

    /// Home with one game, nothing running.
    fn home() -> Model {
        let mut m = launching("Game", false);
        m.session = None;
        m.switch_tab(0, Focus::Hero(0));
        m.rebuild_navigation();
        m
    }
    fn tick(m: &mut Model, now_ms: u64) -> Vec<Command> {
        m.step(Input::Tick { now_ms, unix_s: 0 })
    }

    #[test]
    fn a_key_while_hidden_only_shows_the_strip() {
        let mut strip = AutoHide::new(5_000);
        assert!(strip.swallow(0), "hidden: the first key is used up showing it");
        assert!(strip.visible(4_999) && strip.left(4_999) == Some(1));
        assert!(!strip.swallow(4_999), "shown: the key goes through");
        assert!(strip.visible(9_998), "and the quiet time starts again");
        assert!(!strip.visible(9_999) && strip.left(9_999).is_none(), "five quiet seconds hide it");
        assert!(strip.swallow(9_999), "hidden again: swallowed again");
    }

    #[test]
    fn back_closes_the_layer_then_the_anchor_then_the_page_and_twice_at_home_leaves() {
        let mut m = home();
        m.open_game(GameId(1), None);
        m.rebuild_navigation();
        let other = m.navigation.iter().map(|n| n.id).find(|f| *f != Focus::Play).expect("the game page has more than Play");
        m.set_focus(other);
        m.push_layer(Layer::Chooser, Focus::SheetRow(0));
        key(&mut m, Key::Back);
        assert_eq!((m.view.top(), m.view.focus), (None, other), "the layer closes onto what it covered");
        key(&mut m, Key::Back);
        assert_eq!((m.view.page, m.view.focus), (Page::Game, Focus::Play), "then the anchor");
        key(&mut m, Key::Back);
        assert_eq!((m.view.page, m.view.focus), (Page::Home, Focus::Hero(0)), "then the page before");

        assert!(key(&mut m, Key::Back).is_empty() && m.leave_armed(), "at Home's anchor the first Back only asks");
        assert_eq!(m.hints(), Hints(vec![(Key::Back, "Back again to leave")]));
        tick(&mut m, LEAVE_MS - 1);
        assert!(key(&mut m, Key::Back).contains(&Command::Exit), "a second Back inside the window leaves");

        let mut m = home();
        key(&mut m, Key::Back);
        tick(&mut m, LEAVE_MS);
        assert!(!m.leave_armed(), "the ask runs out");
        assert!(!key(&mut m, Key::Back).contains(&Command::Exit), "and the next Back asks again");
        key(&mut m, Key::Right);
        assert!(!m.leave_armed(), "any other key calls it off");
    }

    #[test]
    fn the_menu_key_is_allowed_on_pages_and_never_over_a_layer_or_a_stream() {
        let mut m = home();
        for page in [Page::Home, Page::Library, Page::Game, Page::Settings] {
            m.view.page = page;
            assert!(m.menu_allowed(), "{page:?}");
        }
        for page in [Page::Launch, Page::Running] {
            m.view.page = page;
            assert!(!m.menu_allowed(), "{page:?}");
        }
        m.view.page = Page::Home;
        use Layer::*;
        for layer in [Chooser, Reader, Viewer, Sort, Genre, PlaysOn, Delete, Auth, SignOut, Picture, Achievements, EndConfirm, AddHost, EditText, Quick, QuickSettings] {
            m.view.push(layer, Focus::None);
            assert!(!m.menu_allowed(), "{layer:?}");
            m.view.pop();
        }
    }

    #[test]
    fn leaving_a_region_and_coming_back_resumes_where_focus_rested() {
        let mut m = home();
        m.set_focus(Focus::Hero(2));
        key(&mut m, Key::Up);
        assert_eq!(m.view.focus, Focus::Nav(0), "a region with no memory starts at its default");
        key(&mut m, Key::Right);
        key(&mut m, Key::Down);
        assert_eq!(m.view.focus, Focus::Hero(2), "the page resumes");
        key(&mut m, Key::Up);
        assert_eq!(m.view.focus, Focus::Nav(1), "and so does the top bar");
    }

    #[test]
    fn the_bumpers_step_home_library_and_search_without_wrapping() {
        let mut m = home();
        m.set_focus(Focus::Nav(0));
        let place = |m: &Model| (m.view.page, m.view.search);
        key(&mut m, Key::Prev);
        assert_eq!(place(&m), (Page::Home, false));
        let mut seen = Vec::new();
        for _ in 0..3 {
            key(&mut m, Key::Next);
            seen.push(place(&m));
        }
        assert_eq!(seen, [(Page::Library, false), (Page::Library, true), (Page::Library, true)]);
        key(&mut m, Key::Prev);
        assert_eq!(place(&m), (Page::Library, false));
        m.switch_tab(3, Focus::Nav(3));
        key(&mut m, Key::Next);
        assert_eq!(m.view.page, Page::Settings, "other pages have no peers to step");
    }

    #[test]
    fn on_a_bare_stream_a_key_only_brings_the_strip_back_and_back_resumes_it() {
        let mut m = launching("Game", false);
        report(&mut m, SessionState::Running { stream: None });
        assert!(m.strip_visible(), "the strip shows as the stream starts");
        tick(&mut m, 6_000);
        assert!(!m.strip_visible());
        key(&mut m, Key::Back);
        assert!(m.strip_visible() && m.view.focus == Focus::Strip(0), "the first key does nothing but bring the strip back");
        key(&mut m, Key::Back);
        assert!(!m.strip_visible(), "Back with the strip up resumes");
    }
}
