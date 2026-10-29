//! Rows for a plugin's own settings page, from the schema the helper serves for it (`settings/PluginRows.js`): a plugin never draws, it names
//! fields, and each device draws them in its own sizes and for its own input. A switch is a toggle, a number a slider, a choice of up to
//! three answers segments, a longer one or a field whose choices the plugin lists (`{"provider": "rooms"}`) a list of rows with a check
//! (the order picked shown as a number), and what the plugin only says (`read-only`, a line it published) a row that shows it. A kind this
//! TV has no row for is left out, not drawn wrong, and an address or a secret is never typed here.
//!
//! The lists a plugin makes are asked of the helper (`GET /v1/plugins/<id>/choices/<provider>`) by the page, which keeps each answer under
//! `choices:<id>:<provider>`; the rows read it from there.

use super::owner::{Field, seg, slider, toggle};
use crate::settings::ctx::Ctx;
use crate::settings::row::{Op, Row};
use serde_json::{Value, json};

/// A plugin whose settings page the helper serves: its id (also its settings owner) and the sections its tabs are, in order.
#[derive(Clone, Copy)]
pub struct Plugin {
    pub id: &'static str,
    pub sections: &'static [&'static str],
}

/// Every field of the schema with the tab its section is; a section the plugin does not declare among `sections` is left out.
pub fn fields(plugin: Plugin, schema: &Value) -> Vec<(usize, &Value)> {
    let mut out = Vec::new();
    for s in schema.get("sections").and_then(Value::as_array).into_iter().flatten() {
        let Some(tab) = s.get("id").and_then(Value::as_str).and_then(|id| plugin.sections.iter().position(|x| *x == id)) else { continue };
        out.extend(s.get("fields").and_then(Value::as_array).into_iter().flatten().map(|f| (tab, f)));
    }
    out
}

/// The providers a schema names (what to ask the helper for), in the order they are met.
pub fn providers(plugin: Plugin, schema: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, f) in fields(plugin, schema) {
        let kind = &f["kind"];
        if matches!(kind["type"].as_str(), Some("choice" | "ordered-set"))
            && let Some(p) = kind.pointer("/options/provider").and_then(Value::as_str)
            && !out.iter().any(|o| o == p)
        {
            out.push(p.to_string());
        }
    }
    out
}

/// Asks the plugin for every list its page needs.
pub fn ask_all(plugin: Plugin, schema: &Value) -> Vec<Op> {
    providers(plugin, schema)
        .into_iter()
        .map(|p| Op::Send { name: Some(format!("choices:{}:{p}", plugin.id)), method: "GET", path: format!("/v1/plugins/{}/choices/{p}?limit=64", plugin.id), body: None })
        .collect()
}

/// The line the plugin published for a read-only field (`plugin.<id>.<key>` in its facts), when there is one.
pub fn fact(c: &Ctx, plugin: Plugin, capability: &str) -> Option<String> {
    let key = capability.strip_prefix(&format!("plugin.{}.", plugin.id)).unwrap_or(capability);
    let v = c.helper(&format!("plugin.{}.facts", plugin.id))?.pointer(&format!("/facts/{key}"))?;
    Some(crate::host::sio::js::string(v))
}

pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A heading with an explanation behind its mark is a row the D-pad can reach; one without is a plain heading.
pub fn head(label: &str, help: &str, tab: usize) -> Row {
    if help.is_empty() { Row::head(label, tab) } else { Row::info(&format!("help:{label}")).tab(tab).label(label).help(help) }
}

/// The rows of one field on `tab`; the page's own kinds (an action, a field it draws its own way) are not here.
pub fn field_rows(c: &Ctx, plugin: Plugin, tab: usize, f: &Value, icon: &str) -> Vec<Row> {
    let (key, label, help) = (text(f, "key"), text(f, "label"), text(f, "help"));
    let kind = &f["kind"];
    let ours = Field::new(plugin.id, key, tab, icon, label).help(help);
    match text(kind, "type") {
        "read-only" => {
            let cap = kind.pointer("/value/capability").and_then(Value::as_str);
            let mut r = Row::info(key).tab(tab).label(label).value(&cap.and_then(|k| fact(c, plugin, k)).unwrap_or_default());
            if f.pointer("/visible_when/type").and_then(Value::as_str) == Some("capability-present") {
                r = r.visible(f.pointer("/visible_when/capability").and_then(Value::as_str).is_some_and(|k| fact(c, plugin, k).is_some()));
            }
            vec![r]
        }
        "bool" => vec![toggle(c, ours)],
        "integer" => {
            let unit = kind.get("unit").and_then(Value::as_str).map(|u| format!(" {u}")).unwrap_or_default();
            let num = |k: &str, d: f64| kind.get(k).and_then(Value::as_f64).unwrap_or(d);
            vec![slider(c, ours, num("min", 0.0), num("max", 100.0), num("step", 1.0), &unit)]
        }
        "choice" | "ordered-set" => match kind.pointer("/options/inline").and_then(Value::as_array).filter(|l| text(kind, "type") == "choice" && l.len() <= 3) {
            Some(inline) => {
                let names: Vec<&str> = inline.iter().map(|o| text(o, "label")).collect();
                let ids: Vec<&str> = inline.iter().map(|o| text(o, "id")).collect();
                vec![seg(c, ours, &names, &ids)]
            }
            None => picker(c, plugin, tab, f, ours),
        },
        // Shown, not typed: an address is the owner's to give from a shell.
        "text" if kind.get("secret").and_then(Value::as_bool) != Some(true) => {
            let now = c.owner(plugin.id).and_then(|o| o.text(key)).filter(|t| !t.is_empty());
            vec![Row::info(key).tab(tab).label(label).value(now.as_deref().unwrap_or("Not set"))]
        }
        _ => Vec::new(),
    }
}

/// One choice, or an ordered set, among what the plugin listed.
fn picker(c: &Ctx, plugin: Plugin, tab: usize, f: &Value, field: Field) -> Vec<Row> {
    let (key, kind) = (field.key, &f["kind"]);
    let many = text(kind, "type") == "ordered-set";
    let max = kind.get("max_items").and_then(Value::as_u64).unwrap_or(64) as usize;
    let mut rows = vec![head(field.label, field.help, tab)];
    let options = kind.get("options").cloned().unwrap_or(Value::Null);
    let (state, list, more) = match options.get("inline").and_then(Value::as_array) {
        Some(inline) => ("ready".to_string(), inline.clone(), false),
        None => {
            let page = c.docs.get(&format!("choices:{}:{}", plugin.id, text(&options, "provider"))).get("page");
            match page.filter(|p| !p.is_null()) {
                Some(p) => (text(p, "state").to_string(), p.get("rows").and_then(Value::as_array).cloned().unwrap_or_default(), p.get("more").and_then(Value::as_bool) == Some(true)),
                None => ("loading".to_string(), Vec::new(), false),
            }
        }
    };
    if state != "ready" {
        rows.push(Row::note(
            match state.as_str() {
                "loading" => "Loading\u{2026}",
                "unconfigured" => "Not linked",
                "offline" => "Offline",
                _ => "Not loaded",
            },
            tab,
        ));
        return rows;
    }
    if list.is_empty() {
        rows.push(Row::note("Nothing to pick", tab));
    }
    let now: Vec<String> = match c.owner(plugin.id).and_then(|o| o.get(key)) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        Some(v) => vec![crate::host::sio::js::string(v)],
        None => Vec::new(),
    };
    for r in &list {
        let id = text(r, "id").to_string();
        let at = now.iter().position(|n| *n == id);
        let mut row = Row::link(&format!("{key}:{id}")).tab(tab).label(text(r, "label")).chevron(false).selected(at.is_some()).admin();
        if many {
            row = row.value(&at.map(|i| (i + 1).to_string()).unwrap_or_default());
        }
        row.detail = text(r, "detail").to_string();
        let value = if many {
            let mut next = now.clone();
            match at {
                Some(i) => {
                    next.remove(i);
                }
                None if next.len() < max => next.push(id.clone()),
                None => {}
            }
            json!(next)
        } else {
            json!(id)
        };
        rows.push(row.does(vec![Op::Owner { owner: plugin.id.into(), key: key.to_string(), value }]));
    }
    if more {
        rows.push(Row::note(&format!("First {} shown", list.len()), tab));
    }
    rows
}
