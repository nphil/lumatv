//! Named sizes describe the content the Home renderer has room to show.

use super::{
    bar_date,
    dock::Dock,
    grid::{self, Entry, Rect, Refusal},
    onhome::layout::{self as controls, Metrics},
    weather_card, widgets,
};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fit {
    Here,
    Elsewhere { col: i64, row: i64 },
    NoRoom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Preset {
    pub label: &'static str,
    pub size: (i64, i64),
    pub fit: Fit,
    pub current: bool,
}

impl Preset {
    fn rank(self) -> (u8, i64, i64) {
        let fit = match self.fit {
            Fit::Here => 0,
            Fit::Elsewhere { .. } => 1,
            Fit::NoRoom => 2,
        };
        (fit, self.size.0 * self.size.1, self.size.1)
    }
}

/// Evaluate once when the chooser opens, not while drawing or tracking the pointer.
pub(super) fn choices(layout: &Value, kind: &str, current: Option<&Entry>, theme: &Value, area: Rect, scale: f32, metrics: &Metrics) -> Vec<Preset> {
    let Some(spec) = grid::type_of(kind) else { return Vec::new() };
    if spec.fixed && current.is_none() {
        return Vec::new();
    }
    let objects = grid::widgets(layout);
    let object = if let Some(card) = current {
        grid::index_of(layout, &card.id).map(|i| &objects[i])
    } else {
        objects.iter().find(|w| w["type"] == kind && w["hidden"] == true)
    }.unwrap_or(&Value::Null);
    let scale = scale.max(1.0);
    let analog = object.pointer("/settings/style").or_else(|| theme.pointer("/clock/style")).and_then(Value::as_str) == Some("analog");
    let greeting = theme.pointer("/clock/greeting").and_then(Value::as_bool) != Some(false);
    let twelve = theme.pointer("/clock/twelveHour").and_then(Value::as_bool) != Some(false);
    let clock_type = theme.pointer("/type/clock").and_then(Value::as_f64).unwrap_or(92.0) as f32;
    let date = bar_date::Place::of(theme.pointer("/layout/statusBar/date").and_then(Value::as_str)) == bar_date::Place::Card;
    let mut dock = Dock::default();
    if kind == "dock" {
        dock.configure(object, 1, 1);
    }
    let mut describe = |rect: Rect| -> Option<(usize, &'static str)> {
        let w = (rect.w as f32 - 2.0 * widgets::PAD_SIDE).max(10.0);
        let h = (rect.h as f32 - 2.0 * widgets::PAD_TOP).max(10.0);
        Some(match kind {
            "clock" => {
                let (w, h) = (w / scale, h / scale);
                let p = widgets::plan(w, h, analog, greeting, clock_type, twelve, date);
                let (face_w, face_h) = if analog {
                    (p.dia, p.dia)
                } else {
                    (p.ts * 2.63 + if twelve { 10.0 + p.ts * 0.6 } else { 0.0 }, p.ts * 1.1)
                };
                if face_w > w || face_h > h {
                    return None;
                }
                match p.calendar() {
                    2 => (2, "Clock + month"),
                    1 => (1, "Clock + week"),
                    _ => (0, if analog { "Clock face" } else { "Digital time" }),
                }
            },
            "weather" => match weather_card::forecast_days(w, h, scale) {
                days @ 3..=6 => (1, ["Up to 3 forecast days", "Up to 4 forecast days", "Up to 5 forecast days", "Up to 6 forecast days"][days - 3]),
                7 => (2, "Up to 7 forecast days"),
                _ => (0, "Weather at a glance"),
            },
            "onhome" => {
                let g = controls::grid(w, h, false, metrics);
                if g.cell.0 < metrics.min_w || g.cell.1 < metrics.mini_h() {
                    return None;
                }
                if g.rows > 1 && g.cols > 1 {
                    (2, "Grid of controls")
                } else if g.rows > 1 || g.cols > 1 {
                    (1, "More controls")
                } else {
                    (0, "Quick controls")
                }
            }
            "homeassistant" => {
                let room = h - metrics.label - 12.0;
                let rows = (room / (metrics.label + metrics.caption + 12.0).max(76.0)).floor() as usize;
                match rows {
                    0 => return None,
                    1 => (0, "One home update"),
                    2..=3 => (1, "A few home updates"),
                    _ => (2, "More home updates"),
                }
            }
            "cameras" => {
                let room = h - metrics.label - 12.0;
                let details = metrics.label + metrics.caption + 4.0;
                let minimum = if room >= details { details } else { metrics.label + 4.0 };
                let rows = (room / (64.0 * scale).min(room).max(minimum)).floor() as usize;
                match rows {
                    0 => return None,
                    1 => (0, "One camera preview"),
                    2..=3 => (1, "A few camera previews"),
                    _ => (2, "More camera previews"),
                }
            }
            "news" => {
                let title_rows = if h >= 400.0 { 3.0 } else if h >= 200.0 { 2.0 } else { 1.0 };
                let summary = h - 2.0 * metrics.caption - 22.0 - title_rows * metrics.body;
                if summary >= 4.0 * metrics.caption {
                    (2, "Longer news summary")
                } else if summary >= metrics.caption {
                    (1, "Headline + summary")
                } else {
                    (0, "News headline")
                }
            }
            "dock" => {
                dock.resize(dock.width, rect.h);
                if rect.h < dock.tile { return None; }
                match dock.visible_rows() {
                    1 => (0, "One row of apps"),
                    2 => (1, "Two rows of apps"),
                    _ => (2, "More rows of apps"),
                }
            }
            _ => return None,
        })
    };

    let (cols, _) = grid::grid(layout);
    let rows = grid::row_count(layout);
    // Legacy local layouts predate the helper's bounds. Never size a scratch grid from an unbounded file value.
    if !(1..=32).contains(&cols) || !(1..=64).contains(&rows) {
        return Vec::new();
    }
    let stride = (cols + 1) as usize;
    let cell = |c: i64, r: i64| r as usize * stride + c as usize;
    // A running count of occupied cells lets every candidate test a location with four lookups.
    let mut covered = vec![0i32; stride * (rows + 1) as usize];
    for card in objects.iter().filter_map(grid::entry_of) {
        if card.hidden || current.is_some_and(|e| e.id == card.id) || !grid::fits(layout, &card) {
            continue;
        }
        for r in card.row..card.row + card.rows {
            for c in card.col..card.col + card.span {
                covered[cell(c + 1, r + 1)] = 1;
            }
        }
    }
    for r in 1..=rows {
        for c in 1..=cols {
            covered[cell(c, r)] += covered[cell(c - 1, r)] + covered[cell(c, r - 1)] - covered[cell(c - 1, r - 1)];
        }
    }
    let free = |c, r, s, n| {
        c >= 0 && r >= 0 && c + s <= cols && r + n <= rows
            && covered[cell(c + s, r + n)] + covered[cell(c, r)] == covered[cell(c, r + n)] + covered[cell(c + s, r)]
    };
    let fit = |s, n| {
        if current.is_some_and(|e| free(e.col, e.row, s, n)) {
            return Fit::Here;
        }
        (0..=rows - n)
            .find_map(|row| (0..=cols - s).find(|&col| free(col, row, s, n)).map(|col| Fit::Elsewhere { col, row }))
            .unwrap_or(Fit::NoRoom)
    };
    let mut shape = Entry { id: String::new(), kind: String::new(), col: 0, row: 0, span: 0, rows: 0, hidden: false, title: None };
    let mut picks: [Option<Preset>; 4] = [None; 4];
    if let Some(card) = current {
        let size = (card.span, card.rows);
        let (i, label) = describe(grid::uniform_rect(layout, area, card)).unwrap_or((3, "Current size"));
        picks[i] = Some(Preset { size, label, fit: fit(size.0, size.1), current: true });
    }
    // Size the dock by rows without changing the number of tiles across it.
    let (lo, hi) = if let Some(dock) = current.filter(|_| spec.fixed) { (dock.span, dock.span) } else { (spec.min.0, cols) };
    for n in spec.min.1..=rows {
        shape.rows = n;
        for s in lo..=hi {
            shape.span = s;
            let Some((i, label)) = describe(grid::uniform_rect(layout, area, &shape)) else { continue };
            if picks[i].is_some_and(|p| p.current) {
                continue;
            }
            let pick = Preset { label, size: (s, n), fit: fit(s, n), current: false };
            if picks[i].is_none_or(|p| pick.rank() < p.rank()) {
                picks[i] = Some(pick);
            }
        }
    }
    picks.into_iter().flatten().collect()
}

/// The grid creates or restores the widget and owns its ID and settings.
pub(super) fn add(layout: &Value, kind: &str, preset: &Preset) -> Result<(Value, String), Refusal> {
    let spec = grid::type_of(kind).ok_or(Refusal::Unknown)?;
    if spec.fixed {
        return Err(Refusal::Fixed);
    }
    let Fit::Elsewhere { col, row } = preset.fit else { return Err(Refusal::Full) };
    let size = preset.size;
    let seed = if spec.want.contains(&size) { size } else { spec.min };
    let (next, id) = grid::add_sized(layout, kind, seed)?;
    let entry = grid::find(&next, &id).ok_or(Refusal::Missing)?;
    if (entry.col, entry.row, entry.span, entry.rows) == (col, row, size.0, size.1) {
        return Ok((next, id));
    }
    let (next, _) = grid::change(&next, &id, Some(col), Some(row), Some(size.0), Some(size.1))?;
    Ok((next, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn offered(layout: &Value, kind: &str, current: Option<&Entry>, theme: &Value, scale: f32) -> Vec<Preset> {
        choices(layout, kind, current, theme, grid::area(layout), scale, &Metrics::nominal(scale))
    }

    #[test]
    fn clock_presets_use_widget_style_and_apply_a_real_month_size() {
        let layout = json!({"grid": {"rows": 16}, "widgets": [
            {"id": "sleeping", "type": "clock", "hidden": true, "settings": {"style": "analog"}}
        ]});
        let theme = json!({"clock": {"style": "digital"}});
        let presets = offered(&layout, "clock", None, &theme, 1.0);
        assert!(presets.iter().any(|p| p.label == "Clock face"));
        assert!(presets.iter().any(|p| p.label == "Clock + week"));
        let month = presets.iter().find(|p| p.label == "Clock + month").unwrap();
        let (next, id) = add(&layout, "clock", month).unwrap();
        let e = grid::find(&next, &id).unwrap();
        assert_eq!((id.as_str(), e.span, e.rows), ("sleeping", month.size.0, month.size.1));
        assert!(!e.hidden);
        assert!(offered(&next, "clock", Some(&e), &theme, 1.0).iter().any(|p| p.current && p.label == "Clock + month"));
    }

    #[test]
    fn resize_reports_current_room_elsewhere_and_no_room_without_moving_cards() {
        let layout = json!({"grid": {"rows": 16}, "widgets": [
            {"id": "apps", "type": "dock", "span": 4, "rows": 3},
            {"id": "top", "type": "weather", "col": 0, "row": 3, "span": 4, "rows": 13},
            {"id": "bottom", "type": "clock", "col": 4, "row": 7, "span": 8, "rows": 9}
        ]});
        let dock = grid::find(&layout, "apps").unwrap();
        let presets = offered(&layout, "dock", Some(&dock), &Value::Null, 1.0);
        assert_eq!(presets[0].fit, Fit::Here);
        assert!(presets[0].current);
        assert_eq!(presets[1].fit, Fit::Elsewhere { col: 4, row: 0 });
        assert_eq!(presets[2].fit, Fit::NoRoom);
        assert!(offered(&layout, "dock", None, &Value::Null, 1.0).is_empty());
    }

    #[test]
    fn a_full_weather_week_is_not_offered_when_the_large_text_preview_cannot_draw_it() {
        let layout = json!({"grid": {"rows": 16}});
        let area = grid::area(&layout);
        let normal = choices(&layout, "weather", None, &Value::Null, area, 1.0, &Metrics::nominal(1.0));
        assert!(normal.iter().any(|p| p.label == "Up to 7 forecast days"));

        let compressed = Rect { h: 724, ..area };
        let large = choices(&layout, "weather", None, &Value::Null, compressed, 2.0, &Metrics::nominal(2.0));
        assert!(large.iter().any(|p| p.label == "Weather at a glance"));
        assert!(!large.iter().any(|p| p.label == "Up to 7 forecast days"));
    }
}
