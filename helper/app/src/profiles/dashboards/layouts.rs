use super::{MAX_LAYOUT_BYTES, Problem};
use crate::profiles::merge;
use lumatv_home_grid::grid;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::sync::LazyLock;

const MAX_WIDGETS: usize = 64;
const PLACEMENT: [&str; 4] = ["col", "row", "span", "rows"];
static SHIPPED: LazyLock<Value> = LazyLock::new(grid::shipped);

fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub(super) fn display_text(value: &Value, limit: usize) -> Option<Value> {
    let s = value.as_str()?;
    if s.contains("://") || s.starts_with('/') || s.chars().any(char::is_control) {
        return None;
    }
    Some(json!(s.chars().take(limit).collect::<String>()))
}

fn number(v: &Value, low: f64, high: f64) -> bool {
    v.as_f64().is_some_and(|n| (low..=high).contains(&n))
}

fn native(raw: &Value) -> Result<Value, Problem> {
    let layout = merge(&SHIPPED, raw);
    // Bound the inputs before native rounding, pixel-row walks and grid arithmetic.
    for (path, min, max) in [
        ("/grid/cols", 1., 32.),
        ("/grid/gutter", 0., 512.),
        ("/safe/x", 0., 959.),
        ("/safe/y", 0., 539.),
        ("/statusBar/height", 0., 1080.),
        ("/statusBar/gap", 0., 1080.),
    ] {
        if !layout.pointer(path).is_some_and(|v| number(v, min, max)) {
            return Err(Problem::Unavailable);
        }
    }
    let rows = &layout["grid"]["rows"];
    if !rows.as_array().map_or_else(
        || number(rows, 1., 64.),
        |r| !r.is_empty() && r.len() <= 64 && r.iter().all(|v| number(v, 1., 1080.)),
    ) || !layout["widgets"].as_array().is_some_and(|w| w.len() <= MAX_WIDGETS)
        || !has_dock(&layout)
    {
        return Err(Problem::Unavailable);
    }
    let (width, height) = grid::cell_size(&layout);
    if width < 1.0 || height < 1.0 {
        return Err(Problem::Unavailable);
    }
    Ok(grid::tidy_layout(&layout))
}

fn normalized(raw: &Value) -> Result<Value, Problem> {
    let layout = native(raw)?;
    let layout = if layout["grid"]["rows"].is_array() {
        let uniform = grid::to_uniform(&layout);
        if grid::cell_size(&uniform).1 < 1.0 {
            return Err(Problem::Unavailable);
        }
        uniform
    } else {
        layout
    };
    if grid::widgets(&layout).iter().any(|w| {
        ["id", "type"].iter().any(|k| !w[k].as_str().is_some_and(identifier))
    }) {
        return Err(Problem::Unavailable);
    }
    Ok(layout)
}

fn public_widget(w: &Value) -> Value {
    let mut out = json!({
        "id": w["id"], "type": w["type"], "col": w["col"], "row": w["row"],
        "span": w["span"], "rows": w["rows"],
    });
    if w["hidden"] == true {
        out["hidden"] = json!(true);
    }
    if let Some(title) = display_text(&w["title"], 96) {
        out["title"] = title;
    }
    if w["type"] == "clock"
        && let Some(style @ ("analog" | "digital")) = w.pointer("/settings/style").and_then(Value::as_str)
    {
        out["settings"] = json!({"style": style});
    }
    if w["type"] == "dock" {
        for k in ["tile", "tileMax", "gap", "padY", "padX", "fade"] {
            if number(&w[k], 0., 1920.) {
                out[k] = w[k].clone();
            }
        }
    }
    out
}

fn public_layout(layout: &Value) -> Value {
    let bar = &layout["statusBar"];
    let items: Vec<Value> = bar["items"]
        .as_array()
        .into_iter()
        .flatten()
        .take(32)
        .filter_map(|item| {
            if let Some(kind) = item.as_str().filter(|s| identifier(s)) {
                return Some(json!(kind));
            }
            let kind = item["type"].as_str().filter(|s| identifier(s))?;
            let mut out = json!({"type": kind});
            if let Some(icon) = item["icon"].as_str().filter(|s| identifier(s)) {
                out["icon"] = json!(icon);
            }
            Some(out)
        })
        .collect();
    let widgets: Vec<Value> = grid::widgets(layout).iter().map(public_widget).collect();
    let (cols, gutter) = grid::grid(layout);
    let area = grid::area(layout);
    let sy = layout["safe"]["y"].as_f64().unwrap_or(54.).round() as i32;
    let mut out = json!({
        "safe": {"x": area.x, "y": sy},
        "grid": {"cols": cols, "gutter": gutter, "rows": grid::row_count(layout)},
        "statusBar": {"height": bar["height"].as_f64().unwrap_or(60.).round() as i32,
            "gap": bar["gap"].as_f64().unwrap_or(20.).round() as i32, "items": items},
        "widgets": widgets,
    });
    if layout["version"].is_u64() {
        out["version"] = layout["version"].clone();
    }
    out
}

pub(super) fn preview(raw: &Value) -> Result<Value, Problem> {
    normalized(raw).map(|layout| public_layout(&layout))
}

fn geometry(layout: &Value, widgets: &[Value]) -> Result<(), Problem> {
    let (cols, _) = grid::grid(layout);
    let max_rows = grid::row_count(layout);
    let mut used = [0u64; 64];
    for w in widgets {
        let [Some(col), Some(row), Some(span), Some(rows)] = PLACEMENT.map(|k| w[k].as_i64()) else {
            return Err(Problem::Invalid("placement uses whole cells"));
        };
        if col < 0 || row < 0 || !(1..=cols).contains(&span) || !(1..=max_rows).contains(&rows)
            || col > cols - span || row > max_rows - rows
        {
            return Err(Problem::Invalid("a widget is outside the grid"));
        }
        if w["hidden"] == true {
            continue;
        }
        let mask = ((1u64 << span) - 1) << col;
        for line in &mut used[row as usize..(row + rows) as usize] {
            if *line & mask != 0 {
                return Err(Problem::Invalid("widgets overlap"));
            }
            *line |= mask;
        }
    }
    Ok(())
}

fn has_dock(layout: &Value) -> bool {
    let mut docks = grid::widgets(layout).iter().filter(|w| w["type"] == "dock");
    docks.next().is_some_and(|dock| {
        dock.get("hidden").is_none_or(|v| v.as_bool() == Some(false))
            && dock.get("id").is_none_or(|v| v.as_str().is_some_and(identifier))
            && PLACEMENT.iter().all(|k| dock.get(*k).is_none_or(Value::is_number))
    }) && docks.next().is_none()
}

pub(super) fn validate_activation(raw: &Value) -> Result<(), Problem> {
    let native = native(raw)?;
    geometry(&native, grid::widgets(&native))
}

pub(super) fn edit(raw: &Value, draft: &Map<String, Value>) -> Result<Value, Problem> {
    let base = normalized(raw)?;
    let shown = public_layout(&base);
    if draft.iter().any(|(k, v)| k != "widgets" && shown.get(k) != Some(v)) {
        return Err(Problem::Invalid("only widget placement can be edited"));
    }
    let requested = draft.get("widgets").and_then(Value::as_array)
        .filter(|w| w.len() <= MAX_WIDGETS)
        .ok_or(Problem::Invalid("layout needs a bounded widgets list"))?;
    // Tidy assigns unique IDs in order. Use that order to map the editor's IDs back to the raw cards.
    let originals = raw.get("widgets").map_or_else(|| grid::widgets(&SHIPPED), |_| grid::widgets(raw));
    let originals: Vec<&Value> = originals.iter().filter(|w| w["type"].is_string()).collect();
    let base_widgets = grid::widgets(&base);
    let mut unchanged = requested.len() == base_widgets.len();
    let mut ids = BTreeSet::new();
    let mut edited = Vec::with_capacity(requested.len());
    for (row, change) in requested.iter().enumerate() {
        let fields = change.as_object().ok_or(Problem::Invalid("a widget must be an object"))?;
        let id = change["id"].as_str().filter(|s| identifier(s))
            .ok_or(Problem::Invalid("a widget needs a plain id"))?;
        if !ids.insert(id) {
            return Err(Problem::Invalid("widget ids must be unique"));
        }
        let kind = change["type"].as_str().ok_or(Problem::Invalid("a widget needs a type"))?;
        let index = grid::index_of(&base, id);
        let old = index.map(|i| &base_widgets[i]);
        let kind_spec = grid::type_of(kind);
        let mut widget = if let Some(old) = old {
            if old["type"] != kind {
                return Err(Problem::Invalid("an existing widget's type cannot change"));
            }
            originals[index.expect("matched native widget")].clone()
        } else {
            let spec = kind_spec.filter(|t| !t.fixed)
                .ok_or(Problem::Invalid("only native addable widgets may be added"))?;
            let mut fresh = json!({"id": id, "type": kind});
            if let Some(title) = spec.default_title {
                fresh["title"] = json!(title);
            }
            if kind == "clock"
                && let Some(settings) = change.get("settings").and_then(Value::as_object)
                && settings.len() == 1
                && settings.get("style").and_then(Value::as_str).is_some_and(|s| matches!(s, "analog" | "digital"))
            {
                fresh["settings"] = change["settings"].clone();
            }
            fresh
        };
        let visible = public_widget(old.unwrap_or(&widget));
        for (k, v) in fields {
            if !matches!(k.as_str(), "hidden" | "id" | "type") && !PLACEMENT.contains(&k.as_str())
                && visible.get(k) != Some(v)
            {
                return Err(Problem::Invalid("widget settings cannot be changed here"));
            }
        }
        // A missing or repeated legacy ID may have been repaired by the native editor.
        if widget["id"] != id {
            widget["id"] = json!(id);
        }
        for k in PLACEMENT {
            widget[k] = change[k].clone();
        }
        let hidden = match fields.get("hidden") {
            None | Some(Value::Bool(false)) => false,
            Some(Value::Bool(true)) => true,
            _ => return Err(Problem::Invalid("hidden must be true or false")),
        };
        unchanged &= index == Some(row) && old.is_some_and(|old| {
            (old["hidden"] == true) == hidden && PLACEMENT.iter().all(|k| old[*k] == change[*k])
        });
        if kind == "dock" && hidden {
            return Err(Problem::Invalid("the dock cannot be hidden"));
        }
        if hidden {
            widget["hidden"] = json!(true);
        } else {
            widget.as_object_mut().expect("native widget").remove("hidden");
        }
        let min = kind_spec.map_or((1, 1), |t| t.min);
        for (side, min) in [("span", min.0), ("rows", min.1)] {
            // Native permits a legacy small card to stay that size, but not to shrink further.
            let min = old.and_then(|v| v[side].as_i64()).map_or(min, |was| was.min(min));
            if widget[side].as_i64().is_none_or(|n| n < min) {
                return Err(Problem::Invalid("a widget is smaller than its native minimum"));
            }
        }
        edited.push(widget);
    }
    if grid::widgets(&base).iter().any(|w| w["type"] == "dock" && !w["id"].as_str().is_some_and(|id| ids.contains(id))) {
        return Err(Problem::Invalid("the dock cannot be removed"));
    }
    if edited.iter().filter(|w| w["type"] == "dock").count() > 1 {
        return Err(Problem::Invalid("the layout can hold only one dock"));
    }
    geometry(&base, &edited)?;
    if unchanged {
        return Ok(raw.clone());
    }
    // The editor does not see entries the native grid ignores. Keep them and the file's other keys.
    edited.extend(grid::widgets(raw).iter().filter(|w| w["type"].as_str().is_none()).cloned());
    if edited.len() > MAX_WIDGETS {
        return Err(Problem::Invalid("too many widgets"));
    }
    let mut out = raw.clone();
    out["widgets"] = Value::Array(edited);
    out["grid"]["rows"] = base["grid"]["rows"].clone();
    if serde_json::to_vec(&out).map_err(|_| Problem::Storage)?.len() > MAX_LAYOUT_BYTES {
        return Err(Problem::Invalid("the layout is too large"));
    }
    Ok(out)
}
