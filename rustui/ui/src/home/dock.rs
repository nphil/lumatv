//! The dock's model (`Dock.qml`'s logic without its drawing): the launch points as the person has them (LG's list is what is installed;
//! theme `apps.dockHide` and `apps.dockOrder` are the person's own, and a tile they never placed keeps its place after the ones they did;
//! LG's own first tile stays first), the columns that fit the card, the row that is scrolled to, a tile picked up and moved along the
//! order (`orderChanged` is Home's to keep as `apps.dockOrder`), and the accent a tile lends the glow behind it.

use serde_json::Value;

/// One tile.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub id: String,
    pub launch_point_id: String,
    pub title: String,
    pub params: Value,
    /// The icon's path on the TV (a file), or empty.
    pub icon: String,
    /// The plate colour under a floating icon (`#rrggbb`), from LG's `iconColor`/`bgColor` or the theme's `tiles.plates`.
    pub plate: String,
    pub forced_plate: bool,
    pub neutral_plate: bool,
    pub unmovable: bool,
    /// LG says the app can be removed and it is not a system app nor one LumaTV protects ([`removable_app`]): the tile menu offers Uninstall only then.
    pub removable: bool,
}

/// Whether a launch point may be uninstalled from Home (`kit/Apps.qml`): LG says `removable`, it is no system app, and its id is not LG's own, LumaTV's
/// or the Homebrew Channel's.
pub fn removable_app(p: &Value) -> bool {
    let id = p.get("id").and_then(Value::as_str).unwrap_or("");
    let protected = id.starts_with("com.webos.") || id.starts_with("com.palm.") || id.starts_with("com.lge.") || id.starts_with("org.nphil.luma") || id == "org.webosbrew.hbchannel";
    p.get("removable").and_then(Value::as_bool) == Some(true) && p.get("systemApp").and_then(Value::as_bool) != Some(true) && !protected
}

/// The launch points with one per HDMI port. A port that has a receiver with a device behind it (eARC on HDMI 2 and an Apple TV on the receiver) is
/// listed once per CEC device, every one with the id `com.webos.app.hdmi2` and its own `launchPointId` and `PhysicalAddress`; they open the same
/// input, so the first stays. Other repeated ids (a browser's bookmarks) are different pages and all stay.
fn one_per_port(list: &[Value]) -> Vec<&Value> {
    let mut ports: Vec<&str> = Vec::new();
    list.iter()
        .filter(|p| {
            let id = p.get("id").and_then(Value::as_str).unwrap_or("");
            let port = p.get("lptype").and_then(Value::as_str) == Some("bookmark") && id.strip_prefix("com.webos.app.").is_some_and(|r| r.starts_with("hdmi"));
            !port || {
                let new = !ports.contains(&id);
                ports.push(id);
                new
            }
        })
        .collect()
}

pub const APPS_TILE: &str = "com.webos.app.discovery";
const NEUTRAL: &[&str] = &["#ffffff", "#fff", "#000000", "#000", "#060606", "#1e1e1e"];

#[derive(Clone, Debug, Default)]
pub struct Dock {
    pub points: Vec<Point>,
    /// LG's launch points as last answered, before the person's arrangement.
    raw: Vec<Value>,
    pub tile: i32,
    pub gap: i32,
    pub pad_y: i32,
    pub pad_x_min: i32,
    pub fade: i32,
    pub width: i32,
    pub height: i32,
    pub index: usize,
    pub scroll_row: usize,
    pub moving: bool,
    move_origin: Option<usize>,
    /// Counts the changes of the tiles' order or of the id list (a tile moved, the list refreshed); the surface remakes its tiles when it moves.
    pub rev: u64,
}

impl Dock {
    /// The geometry from the dock's layout entry (`tile`, `gap`, `padY`, `padX`, `fade`) and the card's size.
    pub fn configure(&mut self, entry: &Value, width: i32, height: i32) {
        let n = |k: &str, d: i32| entry.get(k).and_then(Value::as_f64).map_or(d, |v| v.round() as i32);
        self.tile = n("tile", 120).max(24);
        self.gap = n("gap", 16).max(0);
        self.pad_y = n("padY", 40);
        self.pad_x_min = n("padX", 40);
        self.fade = n("fade", 110);
        self.width = width;
        self.height = height;
        self.clamp();
    }

    pub fn cols(&self) -> usize {
        (((self.width - 2 * self.pad_x_min + self.gap) / (self.tile + self.gap)).max(1)) as usize
    }

    /// The grid is centred: whatever width is left over splits evenly.
    pub fn pad_x(&self) -> i32 {
        let cols = self.cols() as i32;
        ((self.width - (cols * self.tile + (cols - 1) * self.gap)) as f32 / 2.0).round() as i32
    }

    pub fn visible_rows(&self) -> usize {
        (((self.height - self.pad_y + self.gap) / (self.tile + self.gap)).max(1)) as usize
    }

    pub fn total_rows(&self) -> usize {
        self.points.len().div_ceil(self.cols())
    }

    pub fn scrollable(&self) -> bool {
        self.total_rows() > self.visible_rows()
    }

    /// All rows fit: they are centred; otherwise the mock's top inset with the fade over the overflow.
    pub fn grid_top(&self) -> i32 {
        if self.scrollable() {
            self.pad_y.max(64)
        } else {
            let rows = self.total_rows() as i32;
            ((self.height - (rows * self.tile + (rows - 1) * self.gap)) as f32 / 2.0).round() as i32
        }
    }

    pub fn pitch(&self) -> i32 {
        self.tile + self.gap
    }

    /// Where tile `i` sits in the card, with the grid at its rest (unscrolled) place.
    pub fn slot(&self, i: usize) -> (i32, i32) {
        let cols = self.cols();
        (self.pad_x() + (i % cols) as i32 * self.pitch(), self.grid_top() + (i / cols) as i32 * self.pitch())
    }

    pub fn row_of(&self, i: usize) -> usize {
        i / self.cols()
    }

    /// The scroll in pixels for the current row.
    pub fn scroll_px(&self) -> f32 {
        (self.scroll_row as i32 * self.pitch()) as f32
    }

    /// The ids of the dock's tiles in order for LG's list and the theme: what a number key stands for, without a dock to lay out.
    pub fn tile_ids(list: &[Value], theme: &Value) -> Vec<String> {
        Self::arranged(list, theme).iter().map(|p| p.get("id").and_then(Value::as_str).unwrap_or("").to_owned()).collect()
    }

    /// The person's arrangement of LG's list.
    fn arranged(list: &[Value], theme: &Value) -> Vec<Value> {
        let apps = theme.get("apps");
        let hide: Vec<&str> = apps.and_then(|a| a.get("dockHide")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let order: Vec<&str> = apps.and_then(|a| a.get("dockOrder")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let mut list: Vec<(usize, &Value)> = one_per_port(list)
            .into_iter()
            .filter(|p| p.get("hidden").and_then(Value::as_bool) != Some(true))
            .filter(|p| !hide.contains(&p.get("id").and_then(Value::as_str).unwrap_or("")))
            .enumerate()
            .collect();
        if !order.is_empty() {
            let rank = |i: usize, p: &Value| {
                let id = p.get("id").and_then(Value::as_str).unwrap_or("");
                order.iter().position(|o| *o == id).unwrap_or(order.len() + i)
            };
            list.sort_by_key(|(i, p)| (!p.get("unmovable").and_then(Value::as_bool).unwrap_or(false), rank(*i, p)));
        }
        list.into_iter().map(|(_, p)| p.clone()).collect()
    }

    /// The largest icon the launch point has: a field the TV leaves as "" is not one (`Dock.iconFor`).
    fn icon_of(p: &Value) -> String {
        crate::host::apps::icon_of(p)
    }

    /// LG's launch points (the `listLaunchPoints` answer's list) as the person's dock, with the theme's names, icons and plates.
    pub fn set_points(&mut self, list: &[Value], theme: &Value) {
        self.raw = list.to_vec();
        self.refresh(theme);
    }

    pub fn refresh(&mut self, theme: &Value) {
        let mut list = Self::arranged(&self.raw, theme);
        // the tiles are focus ids `DOCK_ID..DOCK_ID + DOCK_MAX`; the next id is the dock card's lift, so a longer list cannot be shown
        list.truncate(super::model::DOCK_MAX as usize);
        let points = Self::points_of(&list, theme);
        // a title or an icon that changed is carried by the tile's own redraw, not by making the tiles again
        if points.len() != self.points.len() || points.iter().zip(&self.points).any(|(a, b)| a.id != b.id) {
            self.rev += 1;
        }
        self.points = points;
        self.clamp();
    }

    /// Every launch point LG lists (the library's list), with the theme's names, icons and plates, before the person's dock hiding.
    pub fn all_points(&self, theme: &Value) -> Vec<Point> {
        let list: Vec<Value> = one_per_port(&self.raw).into_iter().filter(|p| p.get("hidden").and_then(Value::as_bool) != Some(true)).cloned().collect();
        Self::points_of(&list, theme)
    }

    fn points_of(list: &[Value], theme: &Value) -> Vec<Point> {
        let overrides = theme.pointer("/tiles/icons").cloned().unwrap_or(Value::Null);
        let plates = theme.pointer("/tiles/plates").cloned().unwrap_or(Value::Null);
        let renamed = theme.pointer("/apps/names").cloned().unwrap_or(Value::Null);
        list.iter()
            .map(|p| {
                let id = p.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
                let raw = p.get("iconColor").or_else(|| p.get("bgColor")).and_then(Value::as_str).unwrap_or("#2a2833").to_ascii_lowercase();
                let forced = plates.get(&id).and_then(Value::as_str).map(str::to_ascii_lowercase).unwrap_or_default();
                let icon = overrides.get(&id).and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| Self::icon_of(p));
                let title = renamed.get(&id).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
                Point {
                    launch_point_id: p.get("launchPointId").and_then(Value::as_str).unwrap_or("").to_owned(),
                    title: title.or_else(|| p.get("title").and_then(Value::as_str).map(str::to_owned)).unwrap_or_else(|| id.clone()),
                    params: p.get("params").cloned().unwrap_or_else(|| Value::Object(Default::default())),
                    icon,
                    neutral_plate: forced.is_empty() && NEUTRAL.contains(&raw.as_str()),
                    forced_plate: !forced.is_empty(),
                    plate: if forced.is_empty() { raw } else { forced },
                    unmovable: p.get("unmovable").and_then(Value::as_bool).unwrap_or(false),
                    removable: removable_app(p),
                    id,
                }
            })
            .collect()
    }

    fn clamp(&mut self) {
        if self.index >= self.points.len() {
            self.index = 0;
            self.scroll_row = 0;
        }
        self.ensure_visible();
    }

    pub fn focused(&self) -> Option<&Point> {
        self.points.get(self.index)
    }

    pub fn ids(&self) -> Vec<String> {
        self.points.iter().map(|p| p.id.clone()).collect()
    }

    /// The accent a tile lends: theme `accent` is `tile` (its own plate; a neutral plate takes the wallpaper's, Netflix its red),
    /// `wallpaper`, or a fixed colour.
    pub fn accent_for(&self, p: Option<&Point>, theme: &Value, wall_accent: &str) -> String {
        let t = theme.get("accent").and_then(Value::as_str).unwrap_or("tile");
        match (p, t) {
            (_, "wallpaper") => wall_accent.to_owned(),
            (_, fixed) if fixed.starts_with('#') => fixed.to_owned(),
            (None, _) => wall_accent.to_owned(),
            (Some(p), _) if p.neutral_plate => {
                if p.title.to_ascii_lowercase().contains("netflix") || p.id.to_ascii_lowercase().contains("netflix") {
                    "#e50914".to_owned()
                } else {
                    wall_accent.to_owned()
                }
            }
            (Some(p), _) => p.plate.clone(),
        }
    }

    /// Whether tile `i`'s name stands above it: the dock's top visible row puts it above so the name never covers the row beneath
    /// (`Dock.qml` `labelAbove`, a row against `scrollRow`, whether or not the dock scrolls); every other row puts it below.
    pub fn label_above(&self, i: usize) -> bool {
        self.row_of(i) == self.scroll_row
    }

    /// The top of tile `i`'s name pill (`pill_h` high, whole pixels, in the card), the tile lifted by `lift`: on the side `label_above` names, moved
    /// into the card when it would cross the card's edge, as long as it still clears the lifted tile; when it cannot (a large text size makes the
    /// pill tall and the card holds two rows) on the other side, over the neighbouring row's near edge; where neither fits, pinned inside the card
    /// on its own side. `edge` is the margin kept to the card's top and bottom.
    pub fn label_top(&self, i: usize, pill_h: f32, lift: f32, edge: f32) -> f32 {
        let top = (self.slot(i).1 - self.scroll_px() as i32) as f32;
        let pin = |y: f32| y.clamp(edge, (self.height as f32 - edge - pill_h).max(edge));
        // A dock 7 of 16 grid rows high on a 1080 screen is 379 px: two rows leave 62 above the first, and the pill at its 14 px gap would start at -2.
        let above = pin(top - lift - 14.0 - pill_h);
        let below = pin(top + self.tile as f32 + lift + 6.0);
        let clears_above = above + pill_h <= top - lift;
        let clears_below = below >= top + self.tile as f32 + lift;
        let (first, other) = if self.label_above(i) { ((above, clears_above), (below, clears_below)) } else { ((below, clears_below), (above, clears_above)) };
        let y = if first.1 || !other.1 { first.0 } else { other.0 };
        y.round()
    }

    // ---- navigation
    pub fn ensure_visible(&mut self) {
        let row = self.row_of(self.index);
        let vis = self.visible_rows();
        if row < self.scroll_row {
            self.scroll_row = row;
        } else if row + 1 > self.scroll_row + vis {
            self.scroll_row = row + 1 - vis;
        }
        self.scroll_row = self.scroll_row.min(self.total_rows().saturating_sub(vis));
    }

    pub fn focus(&mut self, i: usize) -> bool {
        if i >= self.points.len() {
            return false;
        }
        self.index = i;
        self.ensure_visible();
        true
    }

    /// The tile Left (`dx` -1) or Right (1) rolls over to: past a row's last tile the next row's first, before its first the row above's
    /// last, so the arrows read the dock like text. `None` away from a row's edge and at the two ends of the dock, where the focus holds.
    pub fn rolled(&self, dx: i32) -> Option<usize> {
        let cols = self.cols();
        match dx {
            1 if self.index % cols == cols - 1 && self.index + 1 < self.points.len() => Some(self.index + 1),
            -1 if self.index % cols == 0 && self.index > 0 => Some(self.index - 1),
            _ => None,
        }
    }

    /// The tile nearest an x on the dock's top visible row (entering from a card above): the column under `x`.
    pub fn nearest_in_row(&self, x_in_card: f32) -> usize {
        let cols = self.cols();
        let col = (((x_in_card - self.pad_x() as f32 - self.tile as f32 / 2.0) / self.pitch() as f32).round().max(0.0) as usize).min(cols.saturating_sub(1));
        let row = self.scroll_row;
        (row * cols + col).min(self.points.len().saturating_sub(1))
    }

    /// The x of the focused tile's centre in the card.
    pub fn focus_centre_x(&self) -> f32 {
        self.slot(self.index).0 as f32 + self.tile as f32 / 2.0
    }

    // ---- rearranging
    pub fn begin_move(&mut self) -> bool {
        match self.points.get(self.index) {
            Some(p) if !p.unmovable => {
                self.moving = true;
                self.move_origin = Some(self.index);
                true
            }
            _ => false,
        }
    }

    /// Puts the tile down; the new order when it changed.
    pub fn end_move(&mut self) -> Option<Vec<String>> {
        if !self.moving {
            return None;
        }
        self.moving = false;
        let origin = self.move_origin.take();
        if origin != Some(self.index) { Some(self.ids()) } else { None }
    }

    pub fn cancel_move(&mut self) {
        if !self.moving {
            return;
        }
        self.moving = false;
        if let Some(o) = self.move_origin.take()
            && o != self.index
            && o < self.points.len()
        {
            let p = self.points.remove(self.index);
            self.points.insert(o, p);
            self.rev += 1;
            self.index = o;
            self.ensure_visible();
        }
    }

    /// The picked-up tile to slot `j`: the neighbours slide across.
    pub fn swap_to(&mut self, j: usize) -> bool {
        if j >= self.points.len() || j == self.index || self.points[j].unmovable {
            return false;
        }
        let p = self.points.remove(self.index);
        self.points.insert(j, p);
        self.rev += 1;
        self.index = j;
        self.ensure_visible();
        true
    }

    /// A direction while a tile is picked up: it swaps with the neighbour that way.
    pub fn move_held(&mut self, dx: i32, dy: i32) -> bool {
        let cols = self.cols() as i32;
        let j = self.index as i32 + dx + dy * cols;
        if j < 0 {
            return false;
        }
        self.swap_to((j as usize).min(self.points.len().saturating_sub(1)))
    }

    /// The tile at the pointer's (`x`, `y`) in the card, with the grid scrolled as it is.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let y = y + self.scroll_px();
        for i in 0..self.points.len() {
            let (sx, sy) = self.slot(i);
            if x >= sx as f32 && x < (sx + self.tile) as f32 && y >= sy as f32 && y < (sy + self.tile) as f32 {
                return Some(i);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lp(id: &str, title: &str) -> Value {
        json!({"id": id, "launchPointId": format!("{id}_default"), "title": title, "icon": format!("/usr/palm/{id}/icon.png"), "iconColor": "#101010"})
    }

    fn dock(theme: Value) -> Dock {
        let mut d = Dock::default();
        d.configure(&json!({"tile": 120, "gap": 16, "padY": 40, "fade": 110}), 1792, 392);
        let mut apps = json!(lp(APPS_TILE, "Apps"));
        apps["unmovable"] = json!(true);
        d.set_points(&[apps, lp("netflix", "Netflix"), lp("youtube", "YouTube"), lp("plex", "Plex"), lp("hidden", "Hidden")], &theme);
        d
    }

    #[test]
    fn the_columns_fit_the_card_and_the_grid_is_centred() {
        let d = dock(json!({}));
        assert_eq!(d.cols(), 12);
        assert_eq!(d.pad_x(), 88);
        assert_eq!(d.visible_rows(), 2);
        assert!(!d.scrollable());
        assert_eq!(d.grid_top(), 136);
        assert_eq!(d.slot(1), (88 + 136, 136));
    }

    /// The TV's own answer (`listLaunchPoints`, 33 points): most developer apps give an `extraLargeIcon` of "", which is not an icon.
    #[test]
    fn every_launch_point_that_has_any_icon_gets_the_largest_one_it_has() {
        let all: Value = serde_json::from_str(include_str!("../../../../docs/research/cardapp/tv-20261025-1939/launchpoints.json")).expect("the TV's list");
        let list = all["launchPoints"].as_array().expect("a list").clone();
        let mut d = Dock::default();
        d.configure(&json!({"tile": 120, "gap": 16, "padY": 40, "fade": 110}), 1792, 392);
        d.set_points(&list, &json!({}));
        let icon = |id: &str| d.points.iter().find(|p| p.id == id).map(|p| p.icon.clone()).expect(id);
        assert_eq!(icon("com.beb.plxnative"), "/media/developer/apps/usr/palm/applications/com.beb.plxnative/largeIcon.png");
        assert_eq!(icon("com.webos.app.discovery"), "/mnt/otncabi/usr/palm/applications/com.webos.app.discovery/asset/hd1080/lgstore_130x130.png");
        assert_eq!(icon("netflix"), "/media/cryptofs/apps/usr/palm/applications/netflix/125627457772030877_21790820472215287large_app_icon_192x192.png");
        assert_eq!(icon("org.webosbrew.bridge-64to32"), "/media/developer/apps/usr/palm/applications/org.webosbrew.bridge-64to32/icon.png", "only `icon` is set");
        let blank: Vec<&str> = d.points.iter().filter(|p| p.icon.is_empty()).map(|p| p.id.as_str()).collect();
        assert!(blank.is_empty(), "drawn with no picture: {blank:?}");
    }

    #[test]
    fn the_dock_is_the_persons_own() {
        let d = dock(json!({"apps": {"dockHide": ["hidden"], "dockOrder": ["plex", "netflix"], "names": {"plex": "Films"}}}));
        assert_eq!(d.ids(), vec![APPS_TILE, "plex", "netflix", "youtube"]);
        assert_eq!(d.points[1].title, "Films");
        assert_eq!(d.accent_for(d.points.get(2), &json!({}), "#9a8cf0"), "#101010");
        assert_eq!(d.accent_for(d.points.get(2), &json!({"accent": "wallpaper"}), "#9a8cf0"), "#9a8cf0");
    }

    #[test]
    fn an_hdmi_port_listed_once_per_cec_device_is_one_tile_and_repeated_bookmarks_of_other_apps_stay() {
        let port = |lp: &str, title: &str| json!({"id": "com.webos.app.hdmi2", "launchPointId": lp, "title": title, "lptype": "bookmark"});
        let page = |lp: &str| json!({"id": "com.webos.app.browser", "launchPointId": lp, "title": "Page", "lptype": "bookmark"});
        let mut d = Dock::default();
        d.configure(&json!({"tile": 120, "gap": 16, "padY": 40, "fade": 110}), 1792, 392);
        d.set_points(&[lp("netflix", "Netflix"), port("824972", "AVR-S760H"), port("547811", "Apple TV"), page("1"), page("2")], &json!({}));
        let shown: Vec<(&str, &str)> = d.points.iter().map(|p| (p.id.as_str(), p.launch_point_id.as_str())).collect();
        assert_eq!(shown, vec![("netflix", "netflix_default"), ("com.webos.app.hdmi2", "824972"), ("com.webos.app.browser", "1"), ("com.webos.app.browser", "2")]);
        assert_eq!(d.all_points(&json!({})).len(), 4);
    }

    #[test]
    fn a_list_longer_than_the_focus_ids_is_cut_to_them() {
        let mut d = Dock::default();
        d.configure(&json!({"tile": 120, "gap": 16, "padY": 40, "fade": 110}), 1792, 392);
        let list: Vec<Value> = (0..super::super::model::DOCK_MAX + 5).map(|i| lp(&format!("app{i}"), "App")).collect();
        d.set_points(&list, &json!({}));
        assert_eq!(d.points.len(), super::super::model::DOCK_MAX as usize);
        assert_eq!(d.points.last().map(|p| p.id.as_str()), Some("app59"));
    }

    #[test]
    fn a_picked_up_tile_moves_along_the_order_and_back_puts_it_back() {
        let mut d = dock(json!({}));
        d.focus(2);
        assert!(d.begin_move());
        assert!(d.move_held(1, 0));
        assert_eq!(d.ids()[2..4], ["plex".to_owned(), "youtube".to_owned()]);
        assert!(!d.move_held(-10, 0));
        assert_eq!(d.end_move().unwrap()[3], "youtube");
        d.focus(3);
        d.begin_move();
        d.move_held(-1, 0);
        d.cancel_move();
        assert_eq!(d.ids()[3], "youtube");
        assert_eq!(d.index, 3);
        d.focus(0);
        assert!(!d.begin_move(), "LG's Apps tile stays first");
    }

    #[test]
    fn a_name_goes_where_the_card_has_room_for_it() {
        let mut d = Dock::default();
        let list: Vec<Value> = (0..20).map(|i| lp(&format!("app{i}"), "App")).collect();
        // 392 high: two rows of 120 leave 68 above and below; a 43 pill stands above the top row and below the second
        d.configure(&json!({"tile": 120, "gap": 16, "padY": 40, "fade": 110}), 1792, 392);
        d.set_points(&list, &json!({}));
        assert_eq!((d.total_rows(), d.scrollable(), d.grid_top()), (2, false, 68));
        assert_eq!((d.label_top(0, 43.0, 7.0, 2.0), d.label_top(12, 43.0, 7.0, 2.0)), (4.0, 68.0 + 136.0 + 120.0 + 13.0));
        // 360 high and a 70 pill (200 % text): no room above row 1 or below row 2, so each name stands on the row's other side
        d.configure(&json!({"tile": 120, "gap": 16, "padY": 40, "fade": 110}), 1792, 360);
        for (i, ph) in [(0, 70.0), (12, 70.0), (0, 43.0), (12, 43.0)] {
            let y = d.label_top(i, ph, 7.0, 2.0);
            assert!(y >= 2.0 && y + ph <= 358.0, "tile {i}, pill {ph}: {y}");
        }
        let top = d.grid_top() as f32;
        assert!(d.label_top(0, 70.0, 7.0, 2.0) > top + 120.0, "row 1's name falls below its tile");
        assert!(d.label_top(12, 70.0, 7.0, 2.0) + 70.0 < top + 136.0 + 120.0, "row 2's name rises above its tile");
    }

    #[test]
    fn the_top_rows_name_stands_above_it_and_the_last_rows_below_it_on_a_card_with_no_room_to_spare() {
        let mut d = Dock::default();
        // the owner's dock: 7 of 16 grid rows, 379 high, so the top row's pill at its usual gap would start 2 px over the card's edge
        d.configure(&json!({"tile": 120, "gap": 16, "padY": 40, "fade": 110}), 1792, 379);
        d.set_points(&(0..20).map(|i| lp(&format!("app{i}"), "App")).collect::<Vec<_>>(), &json!({}));
        assert_eq!((d.total_rows(), d.grid_top()), (2, 62));
        let lift = 7.2;
        let (first, last) = (d.label_top(0, 43.0, lift, 2.0), d.label_top(19, 43.0, lift, 2.0));
        assert_eq!(first, 2.0, "pinned to the card's edge");
        assert!(first + 43.0 <= 62.0 - lift, "row 1's name is above its lifted tile: {first}");
        assert!(last >= 62.0 + 136.0 + 120.0 + lift && last + 43.0 <= 377.0, "row 2's name is below its lifted tile: {last}");
    }

    #[test]
    fn left_and_right_roll_over_between_rows_and_hold_at_the_two_ends_of_the_dock() {
        let mut d = Dock::default();
        d.configure(&json!({"tile": 120, "gap": 16, "padY": 40, "fade": 110}), 1792, 392);
        let list: Vec<Value> = (0..30).map(|i| lp(&format!("app{i}"), "App")).collect();
        d.set_points(&list, &json!({}));
        assert_eq!((d.cols(), d.visible_rows(), d.total_rows()), (12, 2, 3));
        for (at, dx, to) in [(11, 1, Some(12)), (12, -1, Some(11)), (23, 1, Some(24)), (24, -1, Some(23)), (5, 1, None), (5, -1, None), (29, 1, None), (0, -1, None)] {
            d.focus(at);
            assert_eq!(d.rolled(dx), to, "{dx} from {at}");
        }
        // a row that ends the list has nothing to roll to
        d.set_points(&list[..12], &json!({}));
        d.focus(11);
        assert_eq!(d.rolled(1), None);
        // the target is in view once the focus lands on it: the dock's top row follows the focus up and down
        d.set_points(&list, &json!({}));
        d.focus(23);
        d.focus(d.rolled(1).unwrap());
        assert_eq!((d.row_of(d.index), d.scroll_row), (2, 1));
        d.focus(12);
        d.focus(d.rolled(-1).unwrap());
        assert_eq!((d.row_of(d.index), d.scroll_row), (0, 0));
    }

    #[test]
    fn the_revision_moves_when_the_tiles_do_and_not_for_a_new_title() {
        let mut d = dock(json!({}));
        let rev = d.rev;
        d.refresh(&json!({}));
        d.refresh(&json!({"apps": {"names": {"netflix": "Films"}}}));
        assert_eq!(d.rev, rev, "the same tiles in the same order");
        d.index = 1;
        assert!(d.swap_to(2));
        assert_eq!(d.rev, rev + 1);
        d.refresh(&json!({"apps": {"dockHide": ["plex"]}}));
        assert_eq!(d.rev, rev + 2);
    }
}
