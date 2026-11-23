//! The quick menu: a drawer on the Menu key (and the top bar's avatar) over any page. Its menu holds Refresh libraries, Accounts and PCs,
//! Settings and, for the game the page had highlighted, Game page, Pin to Home and Hide. Settings opens Quick settings over it: the five
//! settings changed most often ([`QuickSetting`]), which change in place.
use super::nav::{Hints, Slice};
use super::settings::Place;
use super::*;
use ui::components::Icon;

/// What the quick menu keeps while it is open.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// The game the page had highlighted when the menu opened, which its last group acts on.
    pub subject: Option<GameId>,
    /// The stats mode the switch in Quick settings turns back on: the last one used.
    resume: Option<StatsMode>,
}

/// A row of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Refresh,
    Accounts,
    Settings,
    GamePage,
    Pin,
    Hide,
}

/// The settings Quick settings changes in place. Launch's Picture panel shows the same rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickSetting {
    Stream,
    Hdr,
    Stats,
    Theme,
    TextSize,
}

/// The stream's shapes the Stream stepper walks, in order: each height at 60 and at 120 fps.
const HEIGHTS: [u32; 4] = [720, 1080, 1440, 2160];
const WIDTHS: [u32; 4] = [1280, 1920, 2560, 3840];
const TEXT_PERCENT: [u16; 3] = [100, 150, 200];

/// The next of `len` choices after `index`, round the end either way.
fn wrap(index: usize, len: usize, forward: bool) -> usize {
    if forward { (index + 1) % len } else { (index + len - 1) % len }
}

impl QuickSetting {
    pub const ALL: [Self; 5] = [Self::Stream, Self::Hdr, Self::Stats, Self::Theme, Self::TextSize];
    pub fn label(self) -> &'static str {
        match self {
            Self::Stream => "Stream",
            Self::Hdr => "HDR",
            Self::Stats => "Stats while playing",
            Self::Theme => "Theme",
            Self::TextSize => "Text size",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::Stream => Icon::Video,
            Self::Hdr => Icon::Sun,
            Self::Stats => Icon::Gauge,
            Self::Theme => Icon::Palette,
            Self::TextSize => Icon::Expand,
        }
    }
    /// A switch, which shows [`on`](Self::on); the rest are steppers, which show [`value`](Self::value).
    pub fn is_switch(self) -> bool {
        matches!(self, Self::Hdr | Self::Stats)
    }
    pub fn on(self, m: &Model) -> bool {
        match self {
            Self::Hdr => m.prefs.hdr,
            Self::Stats => m.prefs.stats.mode.on(),
            _ => false,
        }
    }
    /// What a stepper shows; empty for a switch.
    pub fn value(self, m: &Model) -> String {
        match self {
            Self::Stream if m.prefs.height == 2160 => format!("4K, {} fps", m.prefs.fps),
            Self::Stream => format!("{}p, {} fps", m.prefs.height, m.prefs.fps),
            Self::Theme => {
                let options = lumatv_tokens::themes::game_options();
                options.iter().find(|o| o.id == m.prefs.family).unwrap_or(&options[0]).name.to_owned()
            }
            Self::TextSize => format!("{}%", m.prefs.text_percent),
            Self::Hdr | Self::Stats => String::new(),
        }
    }
    /// Left (`forward` false) or Right: a stepper moves to its previous or next choice, round the end; a switch flips either way.
    pub fn adjust(self, m: &mut Model, forward: bool, out: &mut Vec<Command>) {
        match self {
            Self::Stream => {
                let height = HEIGHTS.iter().position(|h| *h == m.prefs.height).unwrap_or(HEIGHTS.len() - 1);
                let preset = wrap(height * 2 + usize::from(m.prefs.fps > 60), 2 * HEIGHTS.len(), forward);
                (m.prefs.width, m.prefs.height) = (WIDTHS[preset / 2], HEIGHTS[preset / 2]);
                m.prefs.fps = if preset % 2 == 1 { 120 } else { 60 };
                // AV1 is offered only up to 60 fps.
                if m.prefs.fps > 60 && m.prefs.codec == Some(Codec::Av1) {
                    m.prefs.codec = None;
                }
                m.save_prefs(out);
                m.push_common(out);
            }
            Self::Hdr => {
                m.prefs.hdr = !m.prefs.hdr;
                m.save_prefs(out);
                m.push_common(out);
            }
            Self::Stats => {
                let mode = m.prefs.stats.mode;
                if mode.on() {
                    m.quick.resume = Some(mode);
                    m.set_stats(StatsMode::Off, out);
                } else {
                    m.set_stats(m.quick.resume.unwrap_or(StatsMode::Compact), out);
                }
            }
            Self::Theme => {
                let options = lumatv_tokens::themes::game_options();
                let at = options.iter().position(|o| o.id == m.prefs.family).unwrap_or(0);
                m.prefs.family = options[wrap(at, options.len(), forward)].id.into();
                m.save_prefs(out);
            }
            Self::TextSize => {
                let at = TEXT_PERCENT.iter().position(|t| *t == m.prefs.text_percent).unwrap_or(0);
                m.prefs.text_percent = TEXT_PERCENT[wrap(at, TEXT_PERCENT.len(), forward)];
                m.save_prefs(out);
            }
        }
    }
}

pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        match layer {
            Some(Layer::Quick) => (0..m.quick_items().len()).map(|i| vec![Focus::SheetRow(i)]).collect(),
            Some(Layer::QuickSettings) => {
                let mut rows = vec![vec![Focus::SheetAction(0)]];
                rows.extend((0..QuickSetting::ALL.len()).map(|i| vec![Focus::SheetRow(i)]));
                rows.push(vec![Focus::SheetAction(1)]);
                rows
            }
            _ => Vec::new(),
        }
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        m.quick_activate(f, out);
    }
    fn adjust(m: &mut Model, f: Focus, forward: bool, out: &mut Vec<Command>) -> bool {
        match (m.view.top(), f) {
            (Some(Layer::QuickSettings), Focus::SheetRow(i)) if i < QuickSetting::ALL.len() => {
                QuickSetting::ALL[i].adjust(m, forward, out);
                true
            }
            _ => false,
        }
    }
    fn subject(m: &Model) -> Option<GameId> {
        m.quick.subject
    }
    fn hints(m: &Model) -> Hints {
        Hints(vec![(Key::Back, if m.view.top() == Some(Layer::QuickSettings) { "Back" } else { "Close" })])
    }
}

impl Model {
    /// The game the menu's last group acts on, while it is still in the library.
    pub fn quick_subject(&self) -> Option<GameId> {
        self.quick.subject.filter(|id| self.game(*id).is_some() && !self.state.profile().user.hidden.contains(id))
    }
    /// The menu's rows, in order.
    pub fn quick_items(&self) -> Vec<Item> {
        let mut items = vec![Item::Refresh, Item::Accounts, Item::Settings];
        if self.quick_subject().is_some() {
            items.extend([Item::GamePage, Item::Pin, Item::Hide]);
        }
        items
    }
    /// The owner pinned `id` to Home.
    pub fn pinned(&self, id: GameId) -> bool {
        self.state.profile().user.favourites.contains(&id)
    }
    /// The Menu key: closes the quick menu when it is open, else opens it where `menu_allowed` lets it.
    pub(super) fn toggle_quick(&mut self) {
        if matches!(self.view.top(), Some(Layer::Quick | Layer::QuickSettings)) {
            self.close_quick();
        } else if self.menu_allowed() {
            self.open_quick();
        }
    }
    /// Opens the menu over the page. The highlighted game is read first: under the layer, focus is the layer's.
    pub(super) fn open_quick(&mut self) {
        self.quick.subject = Owner::of(self).subject(self);
        self.push_layer(Layer::Quick, Focus::SheetRow(0));
    }
    fn close_quick(&mut self) {
        while matches!(self.view.top(), Some(Layer::Quick | Layer::QuickSettings)) {
            self.pop_layer();
        }
    }
    fn quick_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match (self.view.top(), f) {
            (Some(Layer::Quick), Focus::SheetRow(i)) => {
                let Some(item) = self.quick_items().get(i).copied() else { return };
                let subject = self.quick_subject();
                match item {
                    Item::Refresh => self.refresh_libraries(None, out),
                    // The page opens after the menu closes, so the way back returns to the page's own focus.
                    Item::Accounts => {
                        self.close_quick();
                        self.open_settings(Place::Accounts);
                    }
                    Item::Settings => self.push_layer(Layer::QuickSettings, Focus::SheetRow(0)),
                    Item::GamePage => {
                        self.close_quick();
                        if let Some(id) = subject {
                            self.open_game(id, None);
                        }
                    }
                    Item::Pin => {
                        if let Some(id) = subject {
                            let pins = &mut self.state.profile_mut().user.favourites;
                            if !pins.remove(&id) {
                                pins.insert(id);
                            }
                            self.rebuild_rows();
                            self.rebuild_filter();
                            out.push(Command::SaveState);
                        }
                    }
                    Item::Hide => {
                        if let Some(id) = subject {
                            self.set_hidden(id, true, out);
                        }
                    }
                }
            }
            (Some(Layer::QuickSettings), Focus::SheetAction(0)) => self.pop_layer(),
            (Some(Layer::QuickSettings), Focus::SheetAction(_)) => {
                self.close_quick();
                self.open_settings(Place::ALL[0]);
            }
            (Some(Layer::QuickSettings), Focus::SheetRow(i)) if i < QuickSetting::ALL.len() => QuickSetting::ALL[i].adjust(self, true, out),
            _ => {}
        }
    }
    /// Takes `id` out of every list (`hidden`) or puts it back. Hiding closes the quick menu and leaves the game's own page; focus stays
    /// on what is left of the list it was in.
    pub fn set_hidden(&mut self, id: GameId, hidden: bool, out: &mut Vec<Command>) {
        let set = &mut self.state.profile_mut().user.hidden;
        let changed = if hidden { set.insert(id) } else { set.remove(&id) };
        if !changed {
            return;
        }
        self.refeature();
        self.rebuild_rows();
        self.rebuild_filter();
        out.push(Command::SaveState);
        if !hidden {
            return;
        }
        self.close_quick();
        if self.view.page == Page::Game && self.view.selected == Some(id) && self.view.layers.is_empty() {
            match self.back.pop() {
                Some(way) => {
                    self.view.restore(way);
                    self.rebuild_filter();
                }
                None => self.switch_tab(0, Focus::Hero(0)),
            }
        }
        self.view.focus = match self.view.focus {
            Focus::Grid(i) if i >= self.filtered.len() => self.filtered.len().checked_sub(1).map_or(Focus::Filter(0), Focus::Grid),
            Focus::Home(r, i) => match self.home_rows.get(r) {
                Some(row) if !row.items.is_empty() => Focus::Home(r, i.min(row.items.len() - 1)),
                _ => Focus::Hero(0),
            },
            focus => focus,
        };
    }
    /// What the Refresh row says under its name: what a refresh is doing or found, else how long ago the libraries last loaded.
    pub fn refresh_note(&self) -> Option<String> {
        match self.refresh_line() {
            Some(RefreshLine::Working { text, seconds }) => Some(format!("{text}…{}", seconds.map_or(String::new(), |s| format!("  {s} s")))),
            Some(RefreshLine::Done { text, .. }) => Some(text),
            None => self.libraries_age_ms().map(age),
        }
    }
    /// The clock moved the menu's "N min ago" on.
    pub(super) fn quick_tick(&self, before_ms: u64) -> bool {
        if self.view.top() != Some(Layer::Quick) {
            return false;
        }
        let Some(now) = self.libraries_age_ms() else { return false };
        let then = now.saturating_sub(self.now_ms.saturating_sub(before_ms));
        age(now) != age(then)
    }
}

/// "Updated just now", "Updated 5 min ago", "Updated 3 h ago" or "Updated 2 d ago".
fn age(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => "Updated just now".to_owned(),
        1..60 => format!("Updated {minutes} min ago"),
        60..1440 => format!("Updated {} h ago", minutes / 60),
        _ => format!("Updated {} d ago", minutes / 1440),
    }
}
