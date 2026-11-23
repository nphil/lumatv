//! Keep apps ready (the theme's `system.prewarm`, read by the helper's `prewarm`): the apps behind the remote's buttons wait in the
//! background so a press opens them faster. The order is the priority: the first is kept longest, the last is closed first when memory
//! runs short. Left or right on an app moves it earlier or later.

use crate::settings::ctx::Ctx;
use crate::settings::page::Page;
use crate::settings::row::{Arg, Op, Row, patch_of};
use serde_json::{Value, json};

/// The apps the page orders, in the order the helper starts with (`prewarm::policy::DEFAULT_ORDER`).
const APPS: [(&str, &str, &str); 5] = [
    ("com.beb.plxnative", "Marquee", "film"),
    ("netflix", "Netflix", "tv"),
    ("amazon", "Prime Video", "tv"),
    ("com.apple.appletv", "Apple TV", "tv"),
    ("com.webos.app.livetv", "Live TV", "antenna"),
];

pub fn page() -> Page {
    Page::new("prewarm", "Keep apps ready", "zap").parent("system").back("System").hidden_from_search().rows(rows).summary(summary)
}

/// On unless the theme says `false`.
pub fn is_on(c: &Ctx) -> bool {
    c.theme_at("system.prewarm.on").and_then(Value::as_bool).unwrap_or(true)
}

/// The theme's order: the listed apps of the five first (each once), then the ones it leaves out, in the default order.
fn order(c: &Ctx) -> Vec<&'static str> {
    let listed: Vec<&str> = c.theme_at("system.prewarm.apps").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut out: Vec<&'static str> = Vec::new();
    for id in listed.iter().copied().chain(APPS.iter().map(|a| a.0)) {
        if let Some((known, _, _)) = APPS.iter().find(|a| a.0 == id) {
            if !out.contains(known) {
                out.push(known);
            }
        }
    }
    out
}

pub fn page_summary(c: &Ctx) -> String {
    if is_on(c) { "On".into() } else { "Off".into() }
}

fn summary(c: &Ctx) -> String {
    page_summary(c)
}

/// What the helper says about one app: "Ready" while it waits in the background, "Starting", "Left out" when it showed itself or made
/// sound when preloaded, "Opens normally" for an app that cannot be preloaded (kept once opened), nothing while the helper has not said.
fn state_of(c: &Ctx, id: &str) -> &'static str {
    let apps = c.helper("prewarm").and_then(|t| t.get("apps")).and_then(Value::as_array);
    let Some(app) = apps.and_then(|a| a.iter().find(|e| e.get("id").and_then(Value::as_str) == Some(id))) else { return "" };
    match (app.get("state").and_then(Value::as_str), app.get("note").and_then(Value::as_str)) {
        (Some("ready"), _) => "Ready",
        (Some("launching"), _) => "Starting",
        (Some("skipped"), Some(note)) if note.starts_with("opens normally") => "Opens normally the first time",
        (Some("skipped"), _) => "Left out",
        (Some("cold"), _) => "Not running",
        _ => "",
    }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let on = is_on(c);
    let mut rows = vec![Row::toggle("on")
        .tab(0)
        .icon("zap")
        .label("Keep apps ready")
        .help("Marquee, Netflix, Prime Video, Apple TV and Live TV wait in the background, so a press of their button opens them faster. They use memory: the last in the list is closed first when it runs short.")
        .on(on)
        .act(|a| match a {
            Arg::Toggle(v) => vec![Op::Patch(patch_of("system.prewarm.on", json!(v)))],
            _ => Vec::new(),
        })];
    let now = order(c);
    rows.push(Row::head("Priority (left or right moves an app)", 0));
    for (i, id) in now.iter().enumerate() {
        let (_, name, icon) = APPS.iter().find(|a| a.0 == *id).copied().unwrap_or(("", id, "tv"));
        let list = now.clone();
        let mut row = Row::nudge(&format!("app:{id}")).tab(0).icon(icon).label(name).value(&format!("{}", i + 1)).enabled(on);
        row.detail = state_of(c, id).into();
        rows.push(row.act(move |a| match a {
            // Left is earlier (kept longer), right is later.
            Arg::Nudge(d) => {
                let to = if d < 0 { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < list.len()) };
                let Some(to) = to else { return Vec::new() };
                let mut moved = list.clone();
                moved.swap(i, to);
                vec![Op::Patch(patch_of("system.prewarm.apps", json!(moved)))]
            }
            _ => Vec::new(),
        }));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn the_page_is_on_with_the_five_apps_in_the_default_order_until_the_theme_says_otherwise() {
        let w = World::new(lg_with(&[], &[]));
        let r = rows(&w.ctx());
        assert!(by_id(&r, "on").on);
        let apps: Vec<&str> = r.iter().filter_map(|r| r.id.strip_prefix("app:")).collect();
        assert_eq!(apps, ["com.beb.plxnative", "netflix", "amazon", "com.apple.appletv", "com.webos.app.livetv"]);
        assert_eq!(by_id(&r, "app:netflix").value, "2");
    }

    #[test]
    fn each_app_says_what_the_helper_said_of_it() {
        let mut w = World::new(lg_with(&[], &[]));
        w.topics.helper_up = true;
        w.topics.set_helper(
            "prewarm",
            json!({"apps": [
                {"id": "netflix", "state": "ready", "note": null},
                {"id": "com.beb.plxnative", "state": "skipped", "note": "opens normally the first time"},
                {"id": "amazon", "state": "skipped", "note": "plays sound when preloaded"},
            ]}),
        );
        let r = rows(&w.ctx());
        let says = |id: &str| by_id(&r, id).detail.clone();
        assert_eq!((says("app:netflix").as_str(), says("app:com.beb.plxnative").as_str(), says("app:amazon").as_str()), ("Ready", "Opens normally the first time", "Left out"));
        assert_eq!(says("app:com.webos.app.livetv"), "");
    }

    #[test]
    fn nudging_swaps_with_the_neighbour_writes_the_order_and_stops_at_the_ends() {
        let mut w = World::new(lg_with(&[], &[]));
        w.theme = json!({"system": {"prewarm": {"on": false, "apps": ["amazon", "netflix"]}}});
        let r = rows(&w.ctx());
        assert!(!by_id(&r, "on").on && !by_id(&r, "app:amazon").enabled);
        // The five, the owner's two first and the rest after them.
        let apps: Vec<&str> = r.iter().filter_map(|r| r.id.strip_prefix("app:")).collect();
        assert_eq!(apps, ["amazon", "netflix", "com.beb.plxnative", "com.apple.appletv", "com.webos.app.livetv"]);
        let nudge = |id: &str, d: i32| (by_id(&r, id).act.as_ref().unwrap())(Arg::Nudge(d));
        assert!(nudge("app:amazon", -1).is_empty());
        assert!(nudge("app:com.webos.app.livetv", 1).is_empty());
        let ops = nudge("app:netflix", -1);
        assert!(matches!(&ops[..], [Op::Patch(p)] if p == &json!({"system": {"prewarm": {"apps": ["netflix", "amazon", "com.beb.plxnative", "com.apple.appletv", "com.webos.app.livetv"]}}})));
        let ops = (by_id(&r, "on").act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::Patch(p)] if p == &json!({"system": {"prewarm": {"on": true}}})));
    }
}
