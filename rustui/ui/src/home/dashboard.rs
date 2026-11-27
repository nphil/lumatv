//! Dashboard snapshots are scoped to the person at the TV. A draft uses the
//! revision it started at, never the revision of an unrelated later read.

use serde_json::{Value, json};

pub(crate) const PATH: &str = "/v1/profiles/dashboards";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Edit,
    Create,
    FirstRun,
    Replay,
}

impl Start {
    pub(super) fn is_new(self) -> bool {
        self != Self::Edit
    }

    pub(super) fn guided(self) -> bool {
        matches!(self, Self::FirstRun | Self::Replay)
    }
}

pub(crate) fn profile(topic: &Value) -> Option<&str> {
    let active = topic.get("active")?.as_str()?;
    (topic.get("ready")?.as_bool()? && !active.is_empty()).then_some(active)
}

/// An id can be reused after deletion; the creation stamp distinguishes the people.
pub(crate) fn identity(topic: &Value) -> Option<(&str, Option<u64>)> {
    let id = profile(topic)?;
    let created = topic.get("profiles").and_then(Value::as_array)
        .and_then(|profiles| profiles.iter().find(|p| text(p, "id") == id))
        .and_then(|p| p.get("profileCreated").and_then(Value::as_u64));
    Some((id, created))
}

pub(crate) fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub(crate) fn document<'a>(reply: &'a Value, profile: &str, created: Option<u64>) -> Option<&'a Value> {
    let doc = reply.get("data")?;
    if reply.get("ok").and_then(Value::as_bool) != Some(true)
        || profile.is_empty() || text(doc, "profile") != profile || text(doc, "revision").is_empty()
        || doc.get("profileCreated").and_then(Value::as_u64) != created
        || doc.get("active").and_then(Value::as_str).is_none() || !doc.get("dashboards").is_some_and(Value::is_array)
    {
        return None;
    }
    Some(doc)
}

pub(crate) fn list(doc: &Value) -> &[Value] {
    doc.get("dashboards").and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

pub(crate) fn find<'a>(doc: &'a Value, id: &str) -> Option<&'a Value> {
    list(doc).iter().find(|d| text(d, "id") == id)
}

#[derive(Clone)]
pub(crate) struct Guard {
    profile: String,
    created: Option<u64>,
    revision: String,
}

impl Guard {
    pub(crate) fn of(doc: &Value) -> Self {
        Self { profile: text(doc, "profile").into(), created: doc.get("profileCreated").and_then(Value::as_u64), revision: text(doc, "revision").into() }
    }

    pub(super) fn identity(&self) -> (&str, Option<u64>) {
        (&self.profile, self.created)
    }

    pub(crate) fn body(&self, action: &str, id: &str) -> Value {
        let mut body = json!({"profile": self.profile, "revision": self.revision, "action": action, "id": id});
        if let Some(created) = self.created { body["profileCreated"] = json!(created); }
        body
    }
}

pub(crate) fn name_ok(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 32 && !name.chars().any(char::is_control)
}

pub(super) enum Load {
    Pending,
    Fetching(u64),
    Ready { guard: Guard, active: String, layout: Value },
}

pub(super) struct Open {
    pub profile: String,
    pub created: Option<u64>,
    pub start: Start,
    pub selected: String,
    pub load: Load,
}

pub(super) struct Edit {
    pub guard: Guard,
    pub id: String,
    pub name: String,
    pub naming: bool,
    pub tag: Option<u64>,
}

/// Submit geometry and membership only. The helper keeps the actual widget
/// settings from this profile's guarded working layout.
pub(super) fn changed_widgets(layout: &Value) -> Value {
    let widgets: Vec<_> = super::grid::entries(layout).into_iter().map(|e| {
        let mut w = json!({"id": e.id, "type": e.kind, "col": e.col, "row": e.row, "span": e.span, "rows": e.rows});
        if e.hidden {
            w["hidden"] = json!(true);
        }
        w
    }).collect();
    json!({"widgets": widgets})
}

pub(crate) fn error(status: u16, reply: &Value) -> &'static str {
    match reply.pointer("/error/code").and_then(Value::as_str) {
        Some("profile_changed") => "The signed-in profile changed.",
        Some("stale_revision") => "Dashboards changed elsewhere.",
        Some("duplicate_name") => "A dashboard already has that name.",
        Some("full") => "There’s no room for another dashboard.",
        Some("active_dashboard") => "Switch dashboards before removing this one.",
        _ if status == 0 => "The TV’s helper isn’t answering.",
        _ => "The dashboard could not be saved.",
    }
}
