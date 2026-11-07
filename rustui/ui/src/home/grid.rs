//! The Home grid, as one model (the port of `LayoutGrid.js`, which stays the QML's until Home's kind flips): where each card is drawn,
//! what Settings' Home widgets page changes and what the layout editor moves. The stage is 1920x1080, the safe area and the status bar
//! above the grid, `cols` columns `gutter` apart and rows that are a count or a list of pixel heights; a card is whole cells and never
//! overlaps another; a change that would overlap is refused; a new card takes the first free place at the first size of a ladder that
//! fits; the TV's pixel rows become [`UNIFORM_ROWS`] equal rows when a layout is edited. A layout is the merged document Home holds (the
//! shipped `layout.json` under the owner's), kept as JSON so the owner's file round-trips key for key; nothing here changes one in place.

use serde_json::{Map, Value, json};

pub const STAGE_W: i32 = 1920;
pub const STAGE_H: i32 = 1080;
/// The editor's grid: 37 px rows 20 px apart on the TV's 892 px of card area.
pub const UNIFORM_ROWS: i64 = 16;

/// What a person can put on Home: the name and the glyph the add list shows, the smallest card worth drawing (columns, rows), the ladder
/// of sizes a new card tries (largest first) and the id a first card of the type gets.
pub struct Type {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub min: (i64, i64),
    pub want: &'static [(i64, i64)],
    pub first_id: &'static str,
    pub fixed: bool,
    pub default_title: Option<&'static str>,
}

pub const TYPES: &[Type] = &[
    Type { id: "clock", title: "Clock", icon: "clock", min: (2, 3), want: &[(4, 4), (3, 4), (3, 3), (2, 3)], first_id: "clock", fixed: false, default_title: None },
    Type {
        id: "homeassistant",
        title: "At home",
        icon: "house-wifi",
        min: (2, 3),
        want: &[(4, 5), (4, 4), (3, 4), (3, 3), (2, 3)],
        first_id: "home",
        fixed: false,
        default_title: Some("At home"),
    },
    Type { id: "weather", title: "Weather", icon: "sun", min: (2, 3), want: &[(4, 6), (3, 6), (3, 4), (2, 4), (2, 3)], first_id: "weather", fixed: false, default_title: None },
    Type { id: "news", title: "News", icon: "file-text", min: (3, 4), want: &[(5, 7), (4, 6), (4, 5), (3, 5), (3, 4)], first_id: "news", fixed: false, default_title: None },
    Type { id: "cameras", title: "Cameras", icon: "camera", min: (3, 3), want: &[(6, 5), (4, 5), (4, 4), (3, 4), (3, 3)], first_id: "cameras", fixed: false, default_title: None },
    Type { id: "onhome", title: "Smart home", icon: "house-wifi", min: (3, 3), want: &[(6, 9), (6, 7), (6, 5), (4, 9), (4, 5), (4, 4), (3, 4), (3, 3)], first_id: "onhome", fixed: false, default_title: Some("Smart home") },
    Type { id: "dock", title: "Apps", icon: "layout-grid", min: (3, 3), want: &[(12, 4), (6, 4), (6, 3), (4, 3), (3, 3)], first_id: "dock", fixed: true, default_title: None },
];
/// The types the add list offers, in its order. The dock is moved and resized, never added or removed.
pub const ADDABLE: &[&str] = &["clock", "homeassistant", "weather", "news", "cameras", "onhome"];
/// The widths Settings offers, in columns of the twelve.
pub const SIZES: &[(&str, &str, i64)] = &[("small", "Small", 4), ("half", "Half", 6), ("wide", "Wide", 8)];

/// The shipped layout (`payload/theme/layout.json`), the base every owner's file is merged over.
pub fn shipped() -> Value {
    serde_json::from_str(include_str!("../../../../payload/theme/layout.json")).expect("the shipped layout is JSON")
}

pub fn type_of(name: &str) -> Option<&'static Type> {
    TYPES.iter().find(|t| t.id == name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn centre(&self) -> (f32, f32) {
        (self.x as f32 + self.w as f32 / 2.0, self.y as f32 + self.h as f32 / 2.0)
    }
}

/// One card's numbers, read out of its JSON object.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub kind: String,
    pub col: i64,
    pub row: i64,
    pub span: i64,
    pub rows: i64,
    pub hidden: bool,
    pub title: Option<String>,
}

/// Why a change was turned down; `Edge` is a press that goes nowhere.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    Edge,
    Overlap(Entry),
    Small(Entry),
    Full,
    Missing,
    Unknown,
    Fixed,
}

pub type Changed = Result<(Value, Entry), Refusal>;

fn num(v: Option<&Value>, d: i64) -> i64 {
    v.and_then(Value::as_f64).filter(|f| f.is_finite()).map_or(d, |f| f.round() as i64)
}

fn clamp(v: i64, a: i64, b: i64) -> i64 {
    v.max(a).min(b)
}

pub fn entry_of(v: &Value) -> Option<Entry> {
    let o = v.as_object()?;
    let kind = o.get("type")?.as_str()?.to_owned();
    Some(Entry {
        id: o.get("id").and_then(Value::as_str).unwrap_or("").to_owned(),
        kind,
        col: num(o.get("col"), 0),
        row: num(o.get("row"), 0),
        span: num(o.get("span"), 1),
        rows: num(o.get("rows"), 1),
        hidden: o.get("hidden").and_then(Value::as_bool) == Some(true),
        title: o.get("title").and_then(Value::as_str).map(str::to_owned),
    })
}

impl Entry {
    pub fn kind_of(&self) -> Option<&'static Type> {
        type_of(&self.kind)
    }
    /// The name a card goes by: its own title (the At-home card's), else its type's.
    pub fn title(&self) -> String {
        self.title.clone().unwrap_or_else(|| self.kind_of().map_or(self.kind.clone(), |t| t.title.to_owned()))
    }
    pub fn is_dock(&self) -> bool {
        self.kind == "dock"
    }
    fn min(&self) -> (i64, i64) {
        self.kind_of().map_or((1, 1), |t| t.min)
    }
    fn with(&self, col: i64, row: i64, span: i64, rows: i64) -> Entry {
        Entry { col, row, span, rows, ..self.clone() }
    }
    fn overlaps(&self, o: &Entry) -> bool {
        self.col < o.col + o.span && o.col < self.col + self.span && self.row < o.row + o.rows && o.row < self.row + self.rows
    }
}

/// The cards as JSON objects.
pub fn widgets(layout: &Value) -> &[Value] {
    layout.get("widgets").and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// The cards as entries; objects with no type are not cards.
pub fn entries(layout: &Value) -> Vec<Entry> {
    widgets(layout).iter().filter_map(entry_of).collect()
}

pub fn index_of(layout: &Value, id: &str) -> Option<usize> {
    widgets(layout).iter().position(|w| w.get("id").and_then(Value::as_str) == Some(id))
}

pub fn find(layout: &Value, id: &str) -> Option<Entry> {
    index_of(layout, id).and_then(|i| entry_of(&widgets(layout)[i]))
}

/// The id of the dock, if the layout has one.
pub fn dock_id(layout: &Value) -> Option<String> {
    entries(layout).into_iter().find(Entry::is_dock).map(|e| e.id)
}

/// Whether a card of this type is there but hidden (adding one brings it back).
pub fn hidden_of(layout: &Value, kind: &str) -> bool {
    entries(layout).iter().any(|e| e.kind == kind && e.hidden)
}

pub fn grid(layout: &Value) -> (i64, i64) {
    let g = layout.get("grid");
    (num(g.and_then(|g| g.get("cols")), 12).max(1), num(g.and_then(|g| g.get("gutter")), 20))
}

fn safe(layout: &Value) -> (i64, i64) {
    let s = layout.get("safe");
    (num(s.and_then(|s| s.get("x")), 64), num(s.and_then(|s| s.get("y")), 54))
}

// ---- geometry
pub fn area(layout: &Value) -> Rect {
    let (sx, sy) = safe(layout);
    let sb = layout.get("statusBar");
    let y = sy + num(sb.and_then(|s| s.get("height")), 60) + num(sb.and_then(|s| s.get("gap")), 20);
    Rect { x: sx as i32, y: y as i32, w: (STAGE_W as i64 - 2 * sx) as i32, h: (STAGE_H as i64 - sy - y) as i32 }
}

pub fn pixel_rows(layout: &Value) -> Option<Vec<f64>> {
    layout.pointer("/grid/rows").and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())
}

pub fn row_count(layout: &Value) -> i64 {
    match layout.pointer("/grid/rows") {
        Some(Value::Array(a)) => a.len() as i64,
        other => num(other, 8).max(1),
    }
}

pub fn row_heights(layout: &Value) -> Vec<f64> {
    if let Some(p) = pixel_rows(layout) {
        return p;
    }
    let a = area(layout);
    let n = row_count(layout);
    let (_, gutter) = grid(layout);
    let h = (a.h as f64 - ((n - 1) * gutter) as f64) / n as f64;
    vec![h; n as usize]
}

/// One cell's size (the uniform cell: with pixel rows the height is the mean).
pub fn cell_size(layout: &Value) -> (f64, f64) {
    let a = area(layout);
    let (cols, gutter) = grid(layout);
    let n = row_count(layout);
    ((a.w as f64 - ((cols - 1) * gutter) as f64) / cols as f64, (a.h as f64 - ((n - 1) * gutter) as f64) / n as f64)
}

/// Where a card is drawn, whole pixels.
pub fn rect_of(layout: &Value, e: &Entry) -> Rect {
    let a = area(layout);
    let (cols, gutter) = grid(layout);
    let rows = row_heights(layout);
    let cw = (a.w as f64 - ((cols - 1) * gutter) as f64) / cols as f64;
    let mut y = a.y as f64;
    let mut h = 0.0;
    let first = (e.row.max(0) as usize).min(rows.len());
    let last = ((e.row + e.rows).max(0) as usize).min(rows.len());
    for row in &rows[..first] {
        y += row + gutter as f64;
    }
    for (k, row) in rows.iter().enumerate().take(last).skip(first) {
        h += row + if k as i64 > e.row { gutter as f64 } else { 0.0 };
    }
    Rect {
        x: (a.x as f64 + e.col as f64 * (cw + gutter as f64)).round() as i32,
        y: y.round() as i32,
        w: (e.span as f64 * cw + ((e.span - 1) * gutter) as f64).round() as i32,
        h: h.round() as i32,
    }
}

/// The layout as it is drawn under a status bar `h` tall (the text size grew it): the bar's height is `h`, and pixel rows shrink in
/// proportion so the grid still ends at the safe edge. The owner's file is never this.
pub fn with_status_height(layout: &Value, h: f32) -> Value {
    let mut out = layout.clone();
    let was = layout.pointer("/statusBar/height").and_then(Value::as_f64).unwrap_or(60.0);
    if (h as f64 - was).abs() < 0.5 {
        return out;
    }
    out["statusBar"]["height"] = json!(h);
    if let Some(rows) = pixel_rows(layout) {
        let sum: f64 = rows.iter().sum();
        let target = (sum - (h as f64 - was)).max(rows.len() as f64);
        let mut scaled: Vec<f64> = rows.iter().map(|r| (r * target / sum.max(1.0)).floor()).collect();
        let short = target - scaled.iter().sum::<f64>();
        if let Some(last) = scaled.last_mut() {
            *last += short;
        }
        out["grid"]["rows"] = Value::Array(scaled.iter().map(|r| json!(r)).collect());
    }
    out
}

// ---- entries
fn with_widgets(layout: &Value, list: Vec<Value>) -> Value {
    let mut o = layout.clone();
    o["widgets"] = Value::Array(list);
    o
}

fn to_json(e: &Entry, base: &Value) -> Value {
    let mut o = base.as_object().cloned().unwrap_or_default();
    o.insert("id".into(), json!(e.id));
    o.insert("type".into(), json!(e.kind));
    o.insert("col".into(), json!(e.col));
    o.insert("row".into(), json!(e.row));
    o.insert("span".into(), json!(e.span));
    o.insert("rows".into(), json!(e.rows));
    if e.hidden {
        o.insert("hidden".into(), json!(true));
    } else {
        o.remove("hidden");
    }
    match &e.title {
        Some(t) => {
            o.insert("title".into(), json!(t));
        }
        None => {
            o.remove("title");
        }
    }
    Value::Object(o)
}

fn replace_at(layout: &Value, i: usize, e: &Entry) -> Value {
    let mut w = widgets(layout).to_vec();
    w[i] = to_json(e, &w[i]);
    with_widgets(layout, w)
}

/// An entry the grid can hold: inside the grid, no smaller than a cell.
fn tidy(layout: &Value, e: &Entry) -> Entry {
    let (cols, _) = grid(layout);
    let n = row_count(layout);
    let span = clamp(e.span, 1, cols);
    let rows = clamp(e.rows, 1, n);
    Entry { span, rows, col: clamp(e.col, 0, cols - span), row: clamp(e.row, 0, n - rows), ..e.clone() }
}

fn fits(layout: &Value, e: &Entry) -> bool {
    let (cols, _) = grid(layout);
    e.span >= 1 && e.rows >= 1 && e.col >= 0 && e.row >= 0 && e.col + e.span <= cols && e.row + e.rows <= row_count(layout)
}

/// Another card (a hidden one takes no room) whose cells meet `e`'s; cards are told apart by id.
pub fn blocker(layout: &Value, e: &Entry) -> Option<Entry> {
    entries(layout).into_iter().find(|o| o.id != e.id && !o.hidden && e.overlaps(o))
}

pub fn overlaps(layout: &Value, e: &Entry) -> bool {
    blocker(layout, e).is_some()
}

/// A layout with every entry made to fit: objects with no type are dropped, ids made unique, each cut to the grid.
pub fn tidy_layout(layout: &Value) -> Value {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for w in widgets(layout) {
        let Some(e) = entry_of(w) else { continue };
        let mut o = tidy(layout, &e);
        let base = if o.id.is_empty() { o.kind.clone() } else { o.id.clone() };
        let mut id = base.clone();
        let mut n = 2;
        while seen.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        seen.push(id.clone());
        o.id = id;
        out.push(to_json(&o, w));
    }
    with_widgets(layout, out)
}

/// The TV's pixel rows to [`UNIFORM_ROWS`] equal ones, each card keeping the rows nearest the pixels it covered. A layout whose rows are a
/// count comes back as it was.
pub fn to_uniform(layout: &Value) -> Value {
    let Some(pix) = pixel_rows(layout) else { return layout.clone() };
    let (_, gutter) = grid(layout);
    let g = gutter as f64;
    let n = UNIFORM_ROWS;
    let c = (area(layout).h as f64 - ((n - 1) * gutter) as f64) / n as f64;
    let at = |r: usize| pix.get(r).copied().unwrap_or(0.0);
    let list = widgets(layout)
        .iter()
        .map(|w| {
            let Some(e) = entry_of(w) else { return w.clone() };
            let mut y0 = 0.0;
            for r in 0..e.row.max(0) as usize {
                y0 += at(r) + g;
            }
            let mut h = 0.0;
            for k in (e.row.max(0) as usize)..(e.row + e.rows).max(0) as usize {
                h += at(k) + if k as i64 > e.row { g } else { 0.0 };
            }
            let row = (y0 / (c + g)).round() as i64;
            let rows = (((h + g) / (c + g)).round() as i64).max(1);
            to_json(&Entry { row, rows, ..e }, w)
        })
        .collect();
    let mut out = with_widgets(layout, list);
    out["grid"]["rows"] = json!(n);
    tidy_layout(&out)
}

/// Whether a card lies above (`dy` < 0) or below one with columns in common.
pub fn has_beside(layout: &Value, id: &str, dy: i64) -> bool {
    let Some(e) = find(layout, id) else { return false };
    entries(layout).iter().any(|o| {
        o.id != id && !o.hidden && o.col < e.col + e.span && e.col < o.col + o.span && if dy < 0 { o.row + o.rows <= e.row } else { o.row >= e.row + e.rows }
    })
}

// ---- changing one card
/// A move or a resize to the numbers `next` gives (`None` keeps the card's own).
pub fn change(layout: &Value, id: &str, col: Option<i64>, row: Option<i64>, span: Option<i64>, rows: Option<i64>) -> Changed {
    let i = index_of(layout, id).ok_or(Refusal::Missing)?;
    let e = entry_of(&widgets(layout)[i]).ok_or(Refusal::Missing)?;
    let o = e.with(col.unwrap_or(e.col), row.unwrap_or(e.row), span.unwrap_or(e.span), rows.unwrap_or(e.rows));
    if o.col == e.col && o.row == e.row && o.span == e.span && o.rows == e.rows {
        return Err(Refusal::Edge);
    }
    let m = e.min();
    if (o.span < e.span && o.span < m.0) || (o.rows < e.rows && o.rows < m.1) {
        return Err(Refusal::Small(e));
    }
    if !fits(layout, &o) {
        return Err(Refusal::Edge);
    }
    if let Some(b) = blocker(layout, &o) {
        return Err(Refusal::Overlap(b));
    }
    Ok((replace_at(layout, i, &o), o))
}

/// One cell in a direction, never off the grid.
pub fn shift(layout: &Value, id: &str, dc: i64, dr: i64) -> Changed {
    let e = find(layout, id).ok_or(Refusal::Missing)?;
    let (cols, _) = grid(layout);
    change(layout, id, Some(clamp(e.col + dc, 0, cols - e.span)), Some(clamp(e.row + dr, 0, row_count(layout) - e.rows)), None, None)
}

/// One cell at the bottom right corner.
pub fn resize(layout: &Value, id: &str, dspan: i64, drows: i64) -> Changed {
    let e = find(layout, id).ok_or(Refusal::Missing)?;
    let (cols, _) = grid(layout);
    change(layout, id, None, None, Some(clamp(e.span + dspan, 1, cols - e.col)), Some(clamp(e.rows + drows, 1, row_count(layout) - e.row)))
}

// ---- finding room
fn first_free(layout: &Value, e: &Entry, sizes: &[(i64, i64)]) -> Result<Entry, Refusal> {
    let n = row_count(layout);
    let (cols, _) = grid(layout);
    for &(s, r) in sizes {
        let (span, rows) = (s.min(cols), r.min(n));
        for row in 0..=(n - rows).max(-1) {
            for col in 0..=(cols - span).max(-1) {
                let o = e.with(col, row, span, rows);
                if !overlaps(layout, &o) {
                    return Ok(o);
                }
            }
        }
    }
    Err(Refusal::Full)
}

fn fallbacks(e: &Entry) -> Vec<(i64, i64)> {
    let half = |v: i64| (v + 1) / 2;
    let mut s = vec![(e.span, e.rows)];
    if e.span > 2 {
        s.push((half(e.span), e.rows));
    }
    if e.rows > 2 {
        s.push((e.span, half(e.rows)));
        s.push((half(e.span), half(e.rows)));
    }
    s
}

fn at_least(sizes: &[(i64, i64)], min: (i64, i64)) -> Vec<(i64, i64)> {
    sizes.iter().copied().filter(|s| s.0 >= min.0 && s.1 >= min.1).collect()
}

/// An id for a new card of a type: the type's own while it is free, then `clock-2`, `clock-3`.
pub fn new_id(layout: &Value, kind: &str) -> String {
    let base = type_of(kind).map_or(kind, |t| t.first_id);
    let mut id = base.to_owned();
    let mut n = 2;
    while index_of(layout, &id).is_some() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Adds a card of `kind` in the first free place of its ladder; a hidden card of that type comes back instead. The new layout and the id.
pub fn add(layout: &Value, kind: &str) -> Result<(Value, String), Refusal> {
    let t = type_of(kind).ok_or(Refusal::Unknown)?;
    if t.fixed {
        return Err(Refusal::Fixed);
    }
    if let Some(h) = entries(layout).into_iter().find(|e| e.kind == kind && e.hidden) {
        return show(layout, &h.id);
    }
    let e = Entry { id: new_id(layout, kind), kind: kind.to_owned(), col: 0, row: 0, span: 1, rows: 1, hidden: false, title: t.default_title.map(str::to_owned) };
    let placed = first_free(layout, &e, &at_least(t.want, t.min))?;
    let mut w = widgets(layout).to_vec();
    w.push(to_json(&placed, &Value::Null));
    Ok((with_widgets(layout, w), placed.id))
}

pub fn remove(layout: &Value, id: &str) -> Result<Value, Refusal> {
    let e = find(layout, id).ok_or(Refusal::Missing)?;
    if e.is_dock() {
        return Err(Refusal::Fixed);
    }
    Ok(with_widgets(layout, widgets(layout).iter().filter(|w| w.get("id").and_then(Value::as_str) != Some(id)).cloned().collect()))
}

/// Hiding keeps the card where it was; the room it took is free.
pub fn hide(layout: &Value, id: &str) -> Result<Value, Refusal> {
    let i = index_of(layout, id).ok_or(Refusal::Missing)?;
    let e = entry_of(&widgets(layout)[i]).ok_or(Refusal::Missing)?;
    if e.is_dock() {
        return Err(Refusal::Fixed);
    }
    Ok(replace_at(layout, i, &Entry { hidden: true, ..e }))
}

/// Showing puts a hidden card back where it was, or, if something has taken the place, in the first free one at its size, then smaller.
pub fn show(layout: &Value, id: &str) -> Result<(Value, String), Refusal> {
    let i = index_of(layout, id).ok_or(Refusal::Missing)?;
    let e = entry_of(&widgets(layout)[i]).ok_or(Refusal::Missing)?;
    let o = tidy(layout, &Entry { hidden: false, ..e });
    if !overlaps(layout, &o) {
        return Ok((replace_at(layout, i, &o), id.to_owned()));
    }
    let placed = first_free(layout, &o, &at_least(&fallbacks(&o), o.min()))?;
    Ok((replace_at(layout, i, &placed), id.to_owned()))
}

// ---- sizes and order
/// Which of the three widths a card has, or `custom`.
pub fn size_of(e: &Entry) -> &'static str {
    SIZES.iter().find(|s| s.2 == e.span).map_or("custom", |s| s.0)
}

/// The card made `span` columns wide: it grows to the right; one at the right edge grows to the left.
pub fn set_span(layout: &Value, id: &str, span: i64) -> Changed {
    let e = find(layout, id).ok_or(Refusal::Missing)?;
    let (cols, _) = grid(layout);
    if span > cols {
        return Err(Refusal::Edge);
    }
    let col = if e.col + e.span >= cols { cols - span } else { e.col.min(cols - span) };
    if span == e.span && col == e.col {
        return Err(Refusal::Edge);
    }
    change(layout, id, Some(col.max(0)), None, Some(span), None)
}

/// The ids of the cards on Home in reading order: top to bottom, then left to right; neither the dock nor a hidden card.
pub fn order(layout: &Value) -> Vec<String> {
    let mut list: Vec<Entry> = entries(layout).into_iter().filter(|e| !e.is_dock() && !e.hidden).collect();
    list.sort_by_key(|e| (e.row, e.col));
    list.into_iter().map(|e| e.id).collect()
}

/// Two cards trade places; a card may not end up smaller than its type's smallest.
pub fn swap(layout: &Value, a_id: &str, b_id: &str) -> Result<Value, Refusal> {
    let (ia, ib) = (index_of(layout, a_id).ok_or(Refusal::Missing)?, index_of(layout, b_id).ok_or(Refusal::Missing)?);
    if ia == ib {
        return Err(Refusal::Missing);
    }
    let (a, b) = (entry_of(&widgets(layout)[ia]).ok_or(Refusal::Missing)?, entry_of(&widgets(layout)[ib]).ok_or(Refusal::Missing)?);
    let na = a.with(b.col, b.row, b.span, b.rows);
    let nb = b.with(a.col, a.row, a.span, a.rows);
    let (ma, mb) = (a.min(), b.min());
    if na.span < ma.0 || na.rows < ma.1 {
        return Err(Refusal::Small(a));
    }
    if nb.span < mb.0 || nb.rows < mb.1 {
        return Err(Refusal::Small(b));
    }
    Ok(replace_at(&replace_at(layout, ia, &na), ib, &nb))
}

/// A card one place earlier (-1) or later (+1) in reading order.
pub fn nudge(layout: &Value, id: &str, d: i64) -> Result<Value, Refusal> {
    let o = order(layout);
    let i = o.iter().position(|x| x == id).ok_or(Refusal::Edge)? as i64;
    if i + d < 0 || i + d >= o.len() as i64 {
        return Err(Refusal::Edge);
    }
    swap(layout, id, &o[(i + d) as usize])
}

// ---- the owner's file
const RETIRED: &[&str] = &["version"];

/// Only what differs from the shipped layout (objects key by key, lists and numbers whole); `{}` for the shipped layout.
fn minimize(user: &Map<String, Value>, shipped: Option<&Map<String, Value>>, top: bool) -> Option<Map<String, Value>> {
    let mut out = Map::new();
    for (k, v) in user {
        if top && RETIRED.contains(&k.as_str()) {
            continue;
        }
        let s = shipped.and_then(|s| s.get(k));
        match (v.as_object(), s.and_then(Value::as_object)) {
            (Some(vo), Some(so)) => {
                if let Some(sub) = minimize(vo, Some(so), false) {
                    out.insert(k.clone(), Value::Object(sub));
                }
            }
            _ => {
                if s != Some(v) {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
    }
    if out.is_empty() && !top { None } else { Some(out) }
}

/// What `layout.json` holds for a layout.
pub fn for_file(layout: &Value, shipped: &Value) -> Value {
    let mut o = layout.as_object().cloned().unwrap_or_default();
    o.insert(
        "widgets".into(),
        Value::Array(
            widgets(layout)
                .iter()
                .map(|w| {
                    let mut c = w.as_object().cloned().unwrap_or_default();
                    c.remove("_dock");
                    Value::Object(c)
                })
                .collect(),
        ),
    );
    Value::Object(minimize(&o, shipped.as_object(), true).unwrap_or_default())
}

/// A card's place in two words for a list: "left, top", "full width, bottom".
pub fn place_words(layout: &Value, e: &Entry) -> String {
    let third = |v: f64, n: f64| if v < n / 3.0 { 0 } else if v < 2.0 * n / 3.0 { 1 } else { 2 };
    let (cols, _) = grid(layout);
    let n = row_count(layout);
    let h = if e.span >= cols { "full width" } else { ["left", "centre", "right"][third(e.col as f64 + e.span as f64 / 2.0, cols as f64)] };
    let v = if e.rows >= n { "full height" } else { ["top", "middle", "bottom"][third(e.row as f64 + e.rows as f64 / 2.0, n as f64)] };
    format!("{h}, {v}")
}

// ---- the pointer
/// Where a card lands when dragged `dx`, `dy` pixels: whole cells, on the grid.
pub fn snap_move(layout: &Value, id: &str, dx: f32, dy: f32) -> Option<(i64, i64)> {
    let e = find(layout, id)?;
    let (cw, ch) = cell_size(layout);
    let (cols, gutter) = grid(layout);
    Some((
        clamp((e.col as f64 + dx as f64 / (cw + gutter as f64)).round() as i64, 0, cols - e.span),
        clamp((e.row as f64 + dy as f64 / (ch + gutter as f64)).round() as i64, 0, row_count(layout) - e.rows),
    ))
}

/// The size a card takes when its corner is dragged by `dw`, `dh`.
pub fn snap_resize(layout: &Value, id: &str, dw: f32, dh: f32) -> Option<(i64, i64)> {
    let e = find(layout, id)?;
    let (cw, ch) = cell_size(layout);
    let (cols, gutter) = grid(layout);
    let r = rect_of(layout, &e);
    let m = e.min();
    Some((
        clamp(((r.w as f64 + dw as f64 + gutter as f64) / (cw + gutter as f64)).round() as i64, m.0.min(e.span), cols - e.col),
        clamp(((r.h as f64 + dh as f64 + gutter as f64) / (ch + gutter as f64)).round() as i64, m.1.min(e.rows), row_count(layout) - e.row),
    ))
}

/// The nearest card in a direction from `from` (the dock counts): the centres' offset along the axis plus twice the offset across it.
pub fn neighbour(layout: &Value, from: &str, dx: i64, dy: i64) -> Option<String> {
    let f = find(layout, from)?;
    let (fcx, fcy) = rect_of(layout, &f).centre();
    let mut best: Option<(f32, String)> = None;
    for e in entries(layout) {
        if e.id == from || e.hidden {
            continue;
        }
        let (cx, cy) = rect_of(layout, &e).centre();
        let (ddx, ddy) = (cx - fcx, cy - fcy);
        if (dx != 0 && ddx * dx as f32 <= 0.0) || (dy != 0 && ddy * dy as f32 <= 0.0) {
            continue;
        }
        let (primary, secondary) = if dx != 0 { (ddx.abs(), ddy.abs()) } else { (ddy.abs(), ddx.abs()) };
        let score = primary + secondary * 2.0;
        if best.as_ref().is_none_or(|b| score < b.0) {
            best = Some((score, e.id));
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(l: &Value) -> Vec<(String, Rect)> {
        entries(l).iter().map(|e| (e.id.clone(), rect_of(l, e))).collect()
    }

    #[test]
    fn homes_own_cards_sit_where_lumahome_has_always_drawn_them() {
        let r = rects(&shipped());
        let at = |id: &str| r.iter().find(|x| x.0 == id).unwrap().1;
        assert_eq!(at("clock"), Rect { x: 64, y: 134, w: 584, h: 222 });
        assert_eq!(at("home"), Rect { x: 64, y: 376, w: 584, h: 238 });
        assert_eq!(at("news"), Rect { x: 64 + 4 * 151, y: 134, w: 735, h: 480 });
        assert_eq!(at("weather"), Rect { x: 1423, y: 134, w: 433, h: 480 });
        assert_eq!(at("dock"), Rect { x: 64, y: 634, w: 1792, h: 392 });
    }

    #[test]
    fn the_pixel_rows_become_sixteen_equal_rows() {
        let u = to_uniform(&shipped());
        assert_eq!(u["grid"]["rows"], json!(16));
        let got: Vec<(String, i64, i64)> = entries(&u).iter().map(|e| (e.id.clone(), e.row, e.rows)).collect();
        let want = [("clock", 0, 4), ("home", 4, 5), ("news", 0, 9), ("weather", 0, 9), ("dock", 9, 7)];
        assert_eq!(got, want.iter().map(|w| (w.0.to_owned(), w.1, w.2)).collect::<Vec<_>>());
        assert_eq!(to_uniform(&u), u);
    }

    #[test]
    fn a_card_moves_a_cell_and_is_refused_where_it_would_overlap_or_leave_the_grid() {
        let u = to_uniform(&shipped());
        let free = hide(&u, "news").unwrap();
        let (_, e) = shift(&free, "clock", 1, 0).unwrap();
        assert_eq!(e.col, 1);
        assert!(matches!(shift(&u, "clock", 1, 0), Err(Refusal::Overlap(b)) if b.id == "news"));
        assert_eq!(shift(&u, "clock", -1, 0), Err(Refusal::Edge));
        assert_eq!(shift(&u, "clock", 0, -1), Err(Refusal::Edge));
        assert_eq!(shift(&u, "nope", 1, 0), Err(Refusal::Missing));
        let (_, r) = resize(&free, "clock", 2, 0).unwrap();
        assert_eq!(r.span, 6);
        let (tiny, _) = change(&u, "clock", None, None, Some(2), Some(3)).unwrap();
        assert!(matches!(resize(&tiny, "clock", -1, 0), Err(Refusal::Small(_))));
        assert_eq!(resize(&tiny, "clock", 0, -1), Err(Refusal::Small(find(&tiny, "clock").unwrap())));
    }

    #[test]
    fn a_new_card_takes_the_first_free_place_and_a_hidden_one_comes_back() {
        let u = to_uniform(&shipped());
        assert_eq!(add(&u, "clock").unwrap_err(), Refusal::Full);
        let free = remove(&u, "news").unwrap();
        let (l, id) = add(&free, "news").unwrap();
        let e = find(&l, &id).unwrap();
        assert_eq!((id.as_str(), e.col, e.row, e.span, e.rows), ("news", 4, 0, 5, 7));
        let hidden = hide(&u, "news").unwrap();
        let (back, id2) = add(&hidden, "news").unwrap();
        assert_eq!(id2, "news");
        assert!(!find(&back, "news").unwrap().hidden);
        assert_eq!(remove(&u, "dock"), Err(Refusal::Fixed));
        assert_eq!(hide(&u, "dock"), Err(Refusal::Fixed));
        assert_eq!(add(&u, "dock"), Err(Refusal::Fixed));
        assert_eq!(new_id(&u, "clock"), "clock-2");
    }

    #[test]
    fn sizes_order_and_the_file() {
        let u = to_uniform(&shipped());
        assert_eq!(size_of(&find(&u, "clock").unwrap()), "small");
        let hidden = hide(&u, "news").unwrap();
        let (wide, e) = set_span(&hidden, "weather", 8).unwrap();
        assert_eq!((e.col, e.span), (4, 8));
        assert_eq!(order(&wide), vec!["clock", "weather", "home"]);
        assert_eq!(for_file(&shipped(), &shipped()), json!({}));
        let file = for_file(&wide, &shipped());
        assert_eq!(file.as_object().unwrap().keys().cloned().collect::<Vec<_>>(), vec!["grid", "widgets"]);
        assert_eq!(file["grid"], json!({"rows": 16}));
        let mut merged = shipped();
        lumatv_tokens::merge(&mut merged, &file);
        assert_eq!(rects(&merged), rects(&wide));
        assert_eq!(place_words(&u, &find(&u, "clock").unwrap()), "left, top");
        assert_eq!(place_words(&u, &find(&u, "dock").unwrap()), "full width, bottom");
    }

    #[test]
    fn up_leaves_the_grid_from_a_card_with_nothing_over_it() {
        let u = to_uniform(&shipped());
        assert!(!has_beside(&u, "clock", -1));
        assert!(has_beside(&u, "home", -1));
        assert!(has_beside(&u, "clock", 1));
        assert_eq!(dock_id(&u).as_deref(), Some("dock"));
        assert!(!hidden_of(&u, "news"));
        assert!(hidden_of(&hide(&u, "news").unwrap(), "news"));
        assert_eq!(neighbour(&u, "clock", 1, 0).as_deref(), Some("news"));
        assert_eq!(neighbour(&u, "clock", 0, 1).as_deref(), Some("home"));
        assert_eq!(neighbour(&u, "home", 0, 1).as_deref(), Some("dock"));
    }

    #[test]
    fn a_drag_lands_on_whole_cells() {
        let u = to_uniform(&shipped());
        let (cw, ch) = cell_size(&u);
        assert_eq!(snap_move(&u, "clock", (cw + 20.0) as f32 * 2.0, 0.0), Some((2, 0)));
        assert_eq!(snap_resize(&u, "clock", (cw + 20.0) as f32, (ch + 20.0) as f32), Some((5, 5)));
        assert_eq!(snap_resize(&u, "clock", -5000.0, -5000.0), Some((2, 3)));
    }

    #[test]
    fn a_hand_edited_layout_is_cut_to_the_grid_not_refused() {
        let mut l = shipped();
        l["widgets"] = json!([{"id": "a", "type": "clock", "col": 20, "row": 0, "span": 30, "rows": 1}, {"id": "a", "type": "news", "col": 0, "row": 0, "span": 1, "rows": 1}, {"nope": 1}]);
        let t = tidy_layout(&l);
        let e = entries(&t);
        assert_eq!(e.len(), 2);
        assert_eq!((e[0].col, e[0].span), (0, 12));
        assert_eq!(e[1].id, "a-2");
    }

    #[test]
    fn every_addable_type_has_a_ladder_no_smaller_than_its_smallest() {
        for name in ADDABLE {
            let t = type_of(name).unwrap();
            assert!(t.want.iter().all(|s| s.0 >= t.min.0 && s.1 >= t.min.1), "{name}");
            assert!(!t.title.is_empty() && !t.icon.is_empty() && !t.first_id.is_empty());
        }
    }
}
