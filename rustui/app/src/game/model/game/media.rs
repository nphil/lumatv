//! A game's pictures and videos: the Media section of its page (a filter over what Steam has, and a grid of stills) and the
//! full-screen viewer of a picture. A trailer or video never becomes a texture: OK asks the shell for its own video layer
//! (`Command::PlayVideo`), and until then it shows its still with a play mark.
use crate::game::model::facts::{Fact, Reason, Section, why};
use crate::game::model::nav::{AutoHide, Hints, Slice};
use crate::game::model::{Command, Focus, Key, Layer, Model};
use lumagame_core::community::HubItem;
use lumagame_core::ids::GameId;

/// The viewer's caption and hints leave after this long without a key.
const CHROME_MS: u64 = 3_000;
/// Thumbnails are fetched for the rows on screen: the focused row and this many on either side.
const NEAR_ROWS: usize = 2;

/// What the filter shows. A group Steam has nothing in is left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Screenshots,
    Trailers,
    Community,
    Artwork,
    Videos,
}
impl Group {
    pub const ALL: [Group; 5] = [Group::Screenshots, Group::Trailers, Group::Community, Group::Artwork, Group::Videos];
    pub fn name(self) -> &'static str {
        match self {
            Group::Screenshots => "Screenshots",
            Group::Trailers => "Trailers",
            Group::Community => "Community",
            Group::Artwork => "Artwork",
            Group::Videos => "Videos",
        }
    }
    /// One item of the group, as the viewer's caption names it.
    pub fn one(self) -> &'static str {
        match self {
            Group::Screenshots => "screenshot",
            Group::Trailers => "trailer",
            Group::Community => "community screenshot",
            Group::Artwork => "artwork",
            Group::Videos => "video",
        }
    }
}

/// Which player a video needs: a store trailer's stream, or a video a player linked from YouTube.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoKind {
    /// `source` is the trailer's HLS playlist, else its DASH manifest.
    Trailer,
    /// `source` is the YouTube id.
    YouTube,
}
/// Where a video plays, in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl VideoRect {
    pub const FULL: Self = Self { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };
}

/// One still of a group: what the grid and the viewer show of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item<'a> {
    pub thumb: &'a str,
    pub full: &'a str,
    /// Empty when Steam gives none.
    pub title: &'a str,
    pub author: &'a str,
    pub video: Option<(VideoKind, &'a str)>,
}

/// The Media section's filter and the viewer: which group, which picture, and its caption's timer.
#[derive(Clone, Debug, PartialEq)]
pub struct Viewer {
    pub group: Group,
    pub at: usize,
    pub chrome: AutoHide,
}
impl Default for Viewer {
    fn default() -> Self {
        Self { group: Group::Screenshots, at: 0, chrome: AutoHide::new(CHROME_MS) }
    }
}

/// The viewer's rules. The Media section itself is a layer of the game page, so `game::Rules` hands it to the functions below.
pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, _layer: Option<Layer>) -> Vec<Vec<Focus>> {
        vec![vec![Focus::Media(m.view.media.at)]]
    }
    fn activate(_m: &mut Model, _f: Focus, _out: &mut Vec<Command>) {}
    /// Left and Right step through the group's pictures and stop at either end.
    fn adjust(m: &mut Model, _f: Focus, forward: bool, _out: &mut Vec<Command>) -> bool {
        m.step_viewer(forward);
        true
    }
    fn step(m: &mut Model, forward: bool, _out: &mut Vec<Command>) -> bool {
        m.step_viewer(forward);
        true
    }
    fn hints(m: &Model) -> Hints {
        let count = m.view.selected.map_or(0, |id| m.media_items(id, m.view.media.group).len());
        let at = m.view.media.at;
        let mut hints = Vec::new();
        if at + 1 < count {
            hints.push((Key::Right, "Next"));
        }
        if at > 0 {
            hints.push((Key::Left, "Previous"));
        }
        hints.push((Key::Back, "Back"));
        Hints(hints)
    }
}

impl Model {
    /// The groups the game has something in, in the filter's order.
    pub fn media_groups(&self, id: GameId) -> Vec<Group> {
        Group::ALL.into_iter().filter(|g| !self.media_items(id, *g).is_empty()).collect()
    }
    /// The group the filter has chosen: the one picked last when the game has it, else the first it has.
    pub fn media_group(&self, id: GameId) -> Option<Group> {
        let groups = self.media_groups(id);
        groups.iter().copied().find(|g| *g == self.view.media.group).or(groups.first().copied())
    }
    pub fn media_items(&self, id: GameId, group: Group) -> Vec<Item<'_>> {
        let media = self.facts(id).media;
        fn hub(fact: Fact<&[HubItem]>) -> Vec<Item<'_>> {
            let Fact::Present(items) = fact else { return Vec::new() };
            items
                .iter()
                .map(|h| Item { thumb: &h.image, full: &h.image, title: &h.title, author: &h.author, video: h.youtube.as_deref().map(|id| (VideoKind::YouTube, id)) })
                .collect()
        }
        match group {
            Group::Screenshots => match media.store_screenshots {
                Fact::Present(shots) => shots.iter().map(|s| Item { thumb: &s.thumb, full: &s.full, title: "", author: "", video: None }).collect(),
                _ => Vec::new(),
            },
            Group::Trailers => match media.trailers {
                Fact::Present(trailers) => trailers
                    .iter()
                    .filter_map(|t| {
                        let source = t.hls.as_deref().or(t.dash.as_deref())?;
                        Some(Item { thumb: &t.thumb, full: &t.thumb, title: &t.title, author: "", video: Some((VideoKind::Trailer, source)) })
                    })
                    .collect(),
                _ => Vec::new(),
            },
            Group::Community => hub(media.screenshots),
            Group::Artwork => hub(media.artwork),
            Group::Videos => hub(media.videos),
        }
    }
    /// Why the section has nothing to show, while it has nothing: the store screenshots' state, which every Steam game has.
    pub fn media_absent(&self, id: GameId) -> &'static str {
        match self.facts(id).media.store_screenshots {
            Fact::Loading => "Loading",
            Fact::Absent(reason) => why(reason),
            Fact::Present(_) => why(Reason::Empty),
            Fact::SignIn { .. } => why(Reason::Other),
        }
    }
    /// Columns of the grid: three, two when the text is larger.
    pub fn media_columns(&self) -> usize {
        if self.prefs.text_percent > 100 { 2 } else { 3 }
    }
    /// The section's focus grid: the filter (when there is a choice) over the stills, row by row.
    pub(in crate::game::model) fn media_grid(&self) -> Vec<Vec<Focus>> {
        let Some(id) = self.view.selected else { return Vec::new() };
        let groups = self.media_groups(id);
        let Some(group) = self.media_group(id) else { return Vec::new() };
        let mut rows = Vec::new();
        if groups.len() > 1 {
            rows.push(vec![Focus::MediaFilter(groups.iter().position(|g| *g == group).unwrap_or(0))]);
        }
        let count = self.media_items(id, group).len();
        rows.extend((0..count).collect::<Vec<_>>().chunks(self.media_columns()).map(|row| row.iter().map(|&i| Focus::Media(i)).collect()));
        rows
    }
    /// Every focus of the section, for the page's memory of where it rested: the stills first, so a section opened afresh lands on the
    /// first of them, then the filter.
    pub(in crate::game::model) fn media_focuses(&self) -> Vec<Focus> {
        let mut grid = self.media_grid();
        if grid.first().is_some_and(|row| matches!(row.first(), Some(Focus::MediaFilter(_)))) {
            grid.rotate_left(1);
        }
        grid.into_iter().flatten().collect()
    }
    /// Left or Right in the section. On the filter it picks the next group; on a still it moves along the row. `None` at either end of
    /// the filter or a row, where the key switches sections; `Some(false)` lets navigation move focus.
    pub(in crate::game::model) fn media_sideways(&mut self, f: Focus, forward: bool) -> Option<bool> {
        let id = self.view.selected?;
        match f {
            Focus::MediaFilter(i) => {
                let groups = self.media_groups(id);
                let next = if forward { i + 1 } else { i.checked_sub(1)? };
                self.view.media.group = *groups.get(next)?;
                self.set_focus(Focus::MediaFilter(next));
                Some(true)
            }
            Focus::Media(i) => {
                let column = i % self.media_columns();
                let count = self.media_items(id, self.media_group(id)?).len();
                let moves = if forward { column + 1 < self.media_columns() && i + 1 < count } else { column > 0 };
                moves.then_some(false)
            }
            _ => None,
        }
    }
    /// OK's word on a still: a video plays, a picture opens.
    pub(in crate::game::model) fn media_ok(&self) -> Option<&'static str> {
        let Focus::Media(i) = self.view.focus else { return None };
        let id = self.view.selected?;
        let item = *self.media_items(id, self.media_group(id)?).get(i)?;
        Some(if item.video.is_some() { "Play" } else { "Open" })
    }
    /// OK on still `i`: a video plays on its own layer, full screen; a picture opens the viewer.
    pub(in crate::game::model) fn open_media(&mut self, i: usize, out: &mut Vec<Command>) {
        let Some(id) = self.view.selected else { return };
        let Some(group) = self.media_group(id) else { return };
        let Some(item) = self.media_items(id, group).get(i).copied() else { return };
        if let Some((kind, source)) = item.video {
            out.push(Command::PlayVideo { kind, source: source.to_owned(), rect: VideoRect::FULL });
            return;
        }
        self.view.media.group = group;
        self.view.media.at = i;
        self.view.media.chrome.show(self.now_ms);
        self.push_layer(Layer::Viewer, Focus::Media(i));
    }
    /// The next or previous picture; none past either end. Back then returns to the still last shown.
    fn step_viewer(&mut self, forward: bool) {
        let Some(id) = self.view.selected else { return };
        let count = self.media_items(id, self.view.media.group).len();
        let at = self.view.media.at;
        let next = if forward { at + 1 } else { at.wrapping_sub(1) };
        if next < count {
            self.view.media.at = next;
            self.view.focus = Focus::Media(next);
            if let Some(beneath) = self.view.beneath.last_mut() {
                *beneath = Focus::Media(next);
            }
        }
    }
    /// A key while the viewer's caption is away only brings it back; any other key there keeps it up.
    pub(in crate::game::model) fn media_swallow(&mut self) -> bool {
        self.view.top() == Some(Layer::Viewer) && self.view.media.chrome.swallow(self.now_ms)
    }
    /// The viewer's caption left between `before_ms` and now.
    pub(in crate::game::model) fn media_tick(&self, before_ms: u64) -> bool {
        self.view.top() == Some(Layer::Viewer) && self.view.media.chrome.visible(before_ms) && !self.view.media.chrome.visible(self.now_ms)
    }
    pub(in crate::game::model) fn media_wake(&self, now_ms: u64) -> Option<u64> {
        (self.view.top() == Some(Layer::Viewer)).then(|| self.view.media.chrome.left(now_ms)).flatten()
    }
    /// The pictures the open section or viewer shows: the stills of the rows near focus, or the picture in the viewer and the two
    /// beside it, so a step finds the next one on disk.
    pub(in crate::game::model) fn media_art(&self, id: GameId) -> Vec<String> {
        match self.view.top() {
            Some(Layer::Section(Section::Media)) => {
                let Some(group) = self.media_group(id) else { return Vec::new() };
                let columns = self.media_columns();
                let row = if let Focus::Media(i) = self.view.focus { i / columns } else { 0 };
                let first = row.saturating_sub(NEAR_ROWS) * columns;
                let items = self.media_items(id, group);
                items.iter().skip(first).take((2 * NEAR_ROWS + 1) * columns).map(|item| item.thumb.to_owned()).collect()
            }
            Some(Layer::Viewer) => {
                let items = self.media_items(id, self.view.media.group);
                let at = self.view.media.at;
                [Some(at), at.checked_add(1), at.checked_sub(1)].into_iter().flatten().filter_map(|i| items.get(i)).map(|item| item.full.to_owned()).collect()
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::hades_with;
    use super::*;
    use crate::game::model::tests::*;
    use crate::game::model::Input;
    use lumagame_core::community::{Community, HubItem, Section as Part, Shot, Trailer};

    /// Hades II with seven store screenshots, one trailer and one community video, its Media section open.
    fn media() -> Model {
        let shot = |n: usize| Shot { thumb: format!("https://x/t{n}.jpg"), full: format!("https://x/f{n}.jpg") };
        let trailer = Trailer { title: "Launch".into(), thumb: "https://x/trailer.jpg".into(), hls: Some("https://x/trailer.m3u8".into()), dash: None };
        let video = HubItem {
            id: "1".into(),
            title: "Speedrun".into(),
            author: "zag".into(),
            image: "https://x/video.jpg".into(),
            youtube: Some("dQw4w9WgXcQ".into()),
            likes: 0,
            comments: 0,
        };
        let community = Community {
            store_screenshots: Part::list((0..7).map(shot).collect()),
            trailers: Part::list(vec![trailer]),
            hub_videos: Part::list(vec![video]),
            ..Community::default()
        };
        let (mut m, _) = hades_with(true, community);
        m.set_focus(Focus::Card(2));
        key(&mut m, Key::Ok);
        m
    }

    #[test]
    fn the_filter_offers_only_the_groups_steam_has_and_right_switches_to_the_next() {
        let mut m = media();
        let id = m.view.selected.unwrap();
        assert_eq!(m.media_groups(id), [Group::Screenshots, Group::Trailers, Group::Videos]);
        assert_eq!(m.view.focus, Focus::Media(0), "the section opens on the first still");
        key(&mut m, Key::Up);
        assert_eq!(m.view.focus, Focus::MediaFilter(0));
        key(&mut m, Key::Right);
        assert_eq!((m.view.focus, m.media_group(id)), (Focus::MediaFilter(1), Some(Group::Trailers)));
        key(&mut m, Key::Down);
        assert_eq!(m.view.focus, Focus::Media(0));
    }

    #[test]
    fn left_and_right_move_along_a_row_and_switch_sections_past_its_end() {
        let mut m = media();
        key(&mut m, Key::Right);
        key(&mut m, Key::Right);
        assert_eq!((m.view.top(), m.view.focus), (Some(Layer::Section(Section::Media)), Focus::Media(2)));
        key(&mut m, Key::Right);
        assert_eq!(m.view.top(), Some(Layer::Section(Section::About)));
    }

    #[test]
    fn the_viewer_opens_at_the_still_steps_to_both_ends_and_back_returns_to_the_last_shown() {
        let mut m = media();
        key(&mut m, Key::Down);
        key(&mut m, Key::Right);
        key(&mut m, Key::Ok);
        assert_eq!((m.view.top(), m.view.media.at), (Some(Layer::Viewer), 4));
        for _ in 0..3 {
            key(&mut m, Key::Right);
        }
        assert_eq!(m.view.media.at, 6, "the last picture holds");
        for _ in 0..8 {
            key(&mut m, Key::Left);
        }
        assert_eq!(m.view.media.at, 0, "the first picture holds");
        key(&mut m, Key::Back);
        assert_eq!((m.view.top(), m.view.focus), (Some(Layer::Section(Section::Media)), Focus::Media(0)));
        key(&mut m, Key::Back);
        assert_eq!(m.view.top(), None);
    }

    #[test]
    fn the_caption_leaves_after_three_seconds_and_the_first_key_only_brings_it_back() {
        let mut m = media();
        key(&mut m, Key::Ok);
        let opened = m.now_ms();
        assert_eq!(m.media_wake(opened), Some(CHROME_MS));
        m.step(Input::Tick { now_ms: opened + CHROME_MS, unix_s: 0 });
        assert!(!m.view.media.chrome.visible(m.now_ms()));
        key(&mut m, Key::Right);
        assert_eq!(m.view.media.at, 0, "the key brought the caption back and did not step");
        assert!(m.view.media.chrome.visible(m.now_ms()));
        key(&mut m, Key::Right);
        assert_eq!(m.view.media.at, 1);
    }

    #[test]
    fn a_trailer_and_a_video_ask_for_their_own_video_layer() {
        let mut m = media();
        key(&mut m, Key::Up);
        key(&mut m, Key::Right);
        key(&mut m, Key::Down);
        let out = key(&mut m, Key::Ok);
        assert!(out.contains(&Command::PlayVideo { kind: VideoKind::Trailer, source: "https://x/trailer.m3u8".into(), rect: VideoRect::FULL }));
        assert_eq!(m.view.top(), Some(Layer::Section(Section::Media)), "no viewer for a video");
        key(&mut m, Key::Up);
        key(&mut m, Key::Right);
        key(&mut m, Key::Down);
        let out = key(&mut m, Key::Ok);
        assert!(out.contains(&Command::PlayVideo { kind: VideoKind::YouTube, source: "dQw4w9WgXcQ".into(), rect: VideoRect::FULL }));
    }

    #[test]
    fn with_nothing_to_show_the_section_says_why() {
        let (m, id) = hades_with(true, Community { store_screenshots: Part::absent(lumagame_core::community::Absence::Unreachable), ..Community::default() });
        assert!(m.media_groups(id).is_empty());
        assert_eq!(m.media_absent(id), "Steam can't be reached");
    }
}
