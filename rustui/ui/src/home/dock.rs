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

    /// The person's arrangement of LG's list.
    fn arranged(list: &[Value], theme: &Value) -> Vec<Value> {
        let apps = theme.get("apps");
        let hide: Vec<&str> = apps.and_then(|a| a.get("dockHide")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let order: Vec<&str> = apps.and_then(|a| a.get("dockOrder")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let mut list: Vec<(usize, &Value)> = list
            .iter()
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

    fn icon_of(p: &Value) -> String {
        ["extraLargeIcon", "largeIcon", "icon", "mediumLargeIcon"].iter().find_map(|k| p.get(k).and_then(Value::as_str)).unwrap_or("").to_owned()
    }

    /// LG's launch points (the `listLaunchPoints` answer's list) as the person's dock, with the theme's names, icons and plates.
    pub fn set_points(&mut self, list: &[Value], theme: &Value) {
        self.raw = list.to_vec();
        self.refresh(theme);
    }

    pub fn refresh(&mut self, theme: &Value) {
        self.points = Self::points_of(&Self::arranged(&self.raw, theme), theme);
        self.clamp();
    }

    /// Every launch point LG lists (the library's list), with the theme's names, icons and plates, before the person's dock hiding.
    pub fn all_points(&self, theme: &Value) -> Vec<Point> {
        let list: Vec<Value> = self.raw.iter().filter(|p| p.get("hidden").and_then(Value::as_bool) != Some(true)).cloned().collect();
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

    #[test]
    fn the_dock_is_the_persons_own() {
        let d = dock(json!({"apps": {"dockHide": ["hidden"], "dockOrder": ["plex", "netflix"], "names": {"plex": "Films"}}}));
        assert_eq!(d.ids(), vec![APPS_TILE, "plex", "netflix", "youtube"]);
        assert_eq!(d.points[1].title, "Films");
        assert_eq!(d.accent_for(d.points.get(2), &json!({}), "#9a8cf0"), "#101010");
        assert_eq!(d.accent_for(d.points.get(2), &json!({"accent": "wallpaper"}), "#9a8cf0"), "#9a8cf0");
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
}
