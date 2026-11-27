//! Named layouts live in the active profile's store. The active layout itself is the native
//! working file, so on-TV edits are visible straight away and an initial read writes nothing.

mod layouts;
mod preview;

use super::{Error, FILE_MAX, MARK, Profiles, Store, sha_hex, write_atomic_inner};
use crate::http::{CLIENT_HEADER, Request, Response, error_response};
use crate::timeutil::now_ms;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_DASHBOARDS: usize = 8;
const MAX_NAME: usize = 32;
const MAX_LAYOUT_BYTES: usize = 48 * 1024;
const MAX_BANK_BYTES: usize = MAX_DASHBOARDS * MAX_LAYOUT_BYTES + 8 * 1024;
const BRIDGE_CLIENT: &str = "lumatv-dashboard-bridge";

pub(super) fn starter_layout() -> Value {
    lumatv_home_grid::shipped_dock_home()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Problem {
    ProfileChanged,
    Stale,
    Busy,
    DuplicateName,
    Full,
    Active,
    Missing,
    Invalid(&'static str),
    Unavailable,
    Storage,
}

impl Problem {
    fn response(self) -> Response {
        let (status, code, message) = match self {
            Self::ProfileChanged => (409, "profile_changed", "the active profile changed"),
            Self::Stale => (409, "stale_revision", "Home has changed; read it again before editing"),
            Self::Busy => (409, "home_busy", "another Home edit is finishing"),
            Self::DuplicateName => (409, "duplicate_name", "a dashboard already has that name"),
            Self::Full => (409, "full", "no room for another dashboard"),
            Self::Active => (409, "active_dashboard", "activate another dashboard before deleting this one"),
            Self::Missing => (404, "no_dashboard", "no such dashboard"),
            Self::Invalid(why) => (400, "bad_request", why),
            Self::Unavailable => (503, "dashboard_unavailable", "Home is not available for editing just now"),
            Self::Storage => (500, "io", "the dashboard change could not be written"),
        };
        error_response(status, code, message)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Action {
    Save,
    Activate,
    Delete,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct Edit {
    profile: String,
    profile_created: Option<u64>,
    revision: String,
    action: Action,
    id: String,
    name: Option<String>,
    layout: Option<Map<String, Value>>,
    #[serde(default)]
    activate: bool,
}

#[derive(Serialize, Deserialize)]
struct Dashboard {
    id: String,
    name: String,
    /// Only inactive dashboards keep a copy. The active dashboard is edited by the TV too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layout: Option<Value>,
}

#[derive(Serialize, Deserialize)]
struct Bank {
    generation: u64,
    active: String,
    dashboards: Vec<Dashboard>,
}

impl Default for Bank {
    fn default() -> Self {
        Self {
            generation: 0,
            active: "home".to_owned(),
            dashboards: vec![Dashboard { id: "home".to_owned(), name: "Home".to_owned(), layout: None }],
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Intent<B, L, H> {
    bank: B,
    layout: L,
    before: H,
}

impl Bank {
    fn validate(&self) -> Result<(), Problem> {
        if self.dashboards.is_empty() || self.dashboards.len() > MAX_DASHBOARDS
            || !self.dashboards.iter().any(|d| d.id == self.active)
        {
            return Err(Problem::Unavailable);
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for d in &self.dashboards {
            if !ids.insert(&d.id) || (d.id != "home" && !d.id.strip_prefix("dashboard-")
                .and_then(|n| n.parse::<u64>().ok()).is_some_and(|n| n > 0 && n <= self.generation))
                || dashboard_name(&d.name).ok().as_ref() != Some(&d.name) || !names.insert(d.name.to_lowercase())
                || (d.id == self.active) != d.layout.is_none()
                || d.layout.as_ref().is_some_and(|l| !l.is_object())
            {
                return Err(Problem::Unavailable);
            }
        }
        Ok(())
    }

    fn position(&self, id: &str) -> Result<usize, Problem> {
        self.dashboards.iter().position(|d| d.id == id).ok_or(Problem::Missing)
    }

    fn check_name(&self, id: &str, name: &str) -> Result<(), Problem> {
        let folded = name.to_lowercase();
        if self.dashboards.iter().any(|d| d.id != id && d.name.to_lowercase() == folded) {
            return Err(Problem::DuplicateName);
        }
        Ok(())
    }

    fn activate(&mut self, pos: usize, working: &Value) -> Option<Value> {
        let chosen = self.dashboards[pos].layout.take()?;
        let active = self.dashboards[pos].id.clone();
        let was = self.dashboards.iter_mut().find(|d| d.id == self.active).expect("validated bank");
        was.layout = Some(working.clone());
        self.active = active;
        Some(chosen)
    }
}

fn dashboard_name(name: &str) -> Result<String, Problem> {
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_control) || name.chars().count() > MAX_NAME {
        return Err(Problem::Invalid("a dashboard name is 1 to 32 characters"));
    }
    Ok(name.to_owned())
}

struct Files {
    bank: PathBuf,
    pending: PathBuf,
    staged: PathBuf,
    mirror: PathBuf,
}

impl Files {
    fn new(dir: &Path) -> Self {
        Self {
            bank: dir.join("dashboards.json"),
            pending: dir.join("dashboards.next.json"),
            staged: dir.join("dashboard-layout.next.json"),
            mirror: dir.join("layout.json"),
        }
    }
}

fn read_file(path: &Path, cap: usize) -> Result<Option<Vec<u8>>, Problem> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Problem::Unavailable),
    };
    let meta = file.metadata().map_err(|_| Problem::Unavailable)?;
    if !meta.is_file() || meta.len() > cap as u64 {
        return Err(Problem::Unavailable);
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(cap as u64 + 1).read_to_end(&mut bytes).map_err(|_| Problem::Unavailable)?;
    if bytes.len() > cap {
        return Err(Problem::Unavailable);
    }
    Ok(Some(bytes))
}

fn read_bank(path: &Path) -> Result<Option<Bank>, Problem> {
    read_file(path, MAX_BANK_BYTES)?.map(|b| {
        let bank: Bank = serde_json::from_slice(&b).map_err(|_| Problem::Unavailable)?;
        bank.validate()?;
        Ok(bank)
    }).transpose()
}

fn object(bytes: Option<&[u8]>) -> Result<Value, Problem> {
    match bytes {
        Some(b) => serde_json::from_slice::<Value>(b).ok().filter(Value::is_object).ok_or(Problem::Unavailable),
        None => Ok(json!({})),
    }
}

fn digest(bytes: Option<&[u8]>) -> String {
    bytes.map(sha_hex).unwrap_or_default()
}

pub(super) fn rename_durable(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)?;
    sync_parent(to)?;
    if from.parent() != to.parent() {
        sync_parent(from)?;
    }
    Ok(())
}

fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().expect("store file has a directory"))?.sync_all()
}

fn remove_intent(path: &Path) -> Result<(), Problem> {
    fs::remove_file(path).and_then(|()| sync_parent(path)).map_err(|_| Problem::Storage)
}

fn write_private(path: &Path, bytes: &[u8], mode: u32) -> Result<(), Problem> {
    write_atomic_inner(path, bytes, mode, true).map_err(|_| Problem::Storage)
}

struct Snapshot {
    bank: Bank,
    working: Value,
    working_digest: String,
    theme: Value,
}

impl Snapshot {
    fn layouts(&self, working: &Value) -> Result<Vec<Value>, Problem> {
        let mut dashboards = Vec::with_capacity(self.bank.dashboards.len());
        for d in &self.bank.dashboards {
            dashboards.push(json!({
                "id": d.id, "name": d.name, "layout": layouts::preview(d.layout.as_ref().unwrap_or(working))?,
            }));
        }
        Ok(dashboards)
    }

    fn body(&self, store: &Store, hub: &crate::hub::Hub, dashboards: Vec<Value>) -> Value {
        let tint = read_file(&store.tint_file(), 16 * 1024).ok().flatten();
        let tint = object(tint.as_deref()).unwrap_or(Value::Null);
        json!({
            "profile": store.index.active, "profileCreated": store.records[store.active_id()].created,
            "active": self.bank.active,
            "revision": self.revision(store), "dashboards": dashboards,
            "theme": preview::theme(&self.theme, &tint), "widgets": preview::widgets(hub),
        })
    }

    fn revision(&self, store: &Store) -> String {
        sha_hex(format!("{}:{}:{}:{}:{}:{}", store.dashboard_epoch, store.switched_at,
            store.index.active, store.records[store.active_id()].created, self.bank.generation, self.working_digest).as_bytes())
    }
}

impl Store {
    fn dashboard_theme(&self) -> Result<Option<Value>, Problem> {
        if !self.ready || self.index.switching.is_some() || self.index.work.id != self.index.active {
            return Err(Problem::Unavailable);
        }
        let bytes = read_file(&self.theme_file(), FILE_MAX as usize)?;
        let theme = object(bytes.as_deref())?;
        if theme.get(MARK).is_some_and(|mark| mark.as_str() != Some(self.active_id())) {
            // A late native edit still belongs to the profile that wrote it. Store::sync sorts it out.
            return Ok(None);
        }
        Ok(Some(theme))
    }

    fn dashboard_files(&self) -> Files {
        Files::new(&self.dir_of(self.active_id()))
    }

    fn dashboard_snapshot(&self) -> Result<Snapshot, Problem> {
        let theme = self.dashboard_theme()?.ok_or(Problem::Unavailable)?;
        let files = self.dashboard_files();
        if files.pending.try_exists().map_err(|_| Problem::Unavailable)? {
            return Err(Problem::Unavailable);
        }
        let bank = read_bank(&files.bank)?.unwrap_or_default();
        let bytes = read_file(&self.layout_file(), MAX_LAYOUT_BYTES)?;
        // A native theme write can happen outside this mutex. Never pair a late profile's layout with our theme.
        if self.dashboard_theme()?.as_ref() != Some(&theme) {
            return Err(Problem::Unavailable);
        }
        Ok(Snapshot { bank, working: object(bytes.as_deref())?, working_digest: digest(bytes.as_deref()), theme })
    }

    fn commit_dashboard(&mut self, snap: &mut Snapshot, apply: Option<Value>) -> Result<(), Problem> {
        let files = self.dashboard_files();
        self.dashboard_theme()?.ok_or(Problem::Stale)?;
        let before = read_file(&self.layout_file(), MAX_LAYOUT_BYTES)?;
        if digest(before.as_deref()) != snap.working_digest {
            return Err(Problem::Stale);
        }
        if let Some(layout) = apply.filter(|next| next != &snap.working) {
            layouts::validate_activation(&layout)?;
            let bytes = serde_json::to_vec(&layout).map_err(|_| Problem::Storage)?;
            if bytes.len() > MAX_LAYOUT_BYTES {
                return Err(Problem::Invalid("the layout is too large"));
            }
            let intent = Intent { bank: &snap.bank, layout: &layout, before: snap.working_digest.as_str() };
            let prepared = bank_bytes(&intent)?;
            write_private(&files.staged, &bytes, 0o644)?;
            write_private(&files.pending, &prepared, 0o600)?;
            if !self.finish_dashboard()? {
                return Err(Problem::Stale);
            }
            snap.working = layout;
            snap.working_digest = digest(Some(&bytes));
        } else {
            write_private(&files.bank, &bank_bytes(&snap.bank)?, 0o600)?;
        }
        Ok(())
    }

    /// A staged file and a durable intent precede the working-file rename. If the stage still
    /// exists, no layout has been installed; if it is gone, a later native edit is the new active
    /// layout and must not be overwritten on recovery. The intent becomes the bank only after the
    /// active profile's native copy is safe (a later profile switch reads that copy).
    pub(super) fn finish_dashboard(&mut self) -> Result<bool, Problem> {
        let dir = self.dir_of(self.active_id());
        let Some(bytes) = read_file(&dir.join("dashboards.next.json"), MAX_BANK_BYTES)? else {
            return Ok(true);
        };
        let intent: Intent<Bank, Value, String> = serde_json::from_slice(&bytes).map_err(|_| Problem::Unavailable)?;
        intent.bank.validate()?;
        let files = Files::new(&dir);
        let work = self.layout_file();
        let owns_work = self.dashboard_theme()?.is_some();
        if files.staged.try_exists().map_err(|_| Problem::Unavailable)? {
            let bytes = read_file(&work, MAX_LAYOUT_BYTES)?;
            if !owns_work || digest(bytes.as_deref()) != intent.before {
                remove_intent(&files.pending)?;
                // Keep the stage until the cancellation is durable; its absence means the rename ran.
                let _ = fs::remove_file(&files.staged);
                return Ok(false);
            }
            rename_durable(&files.staged, &work).map_err(|_| Problem::Storage)?;
        }
        let bytes = if self.dashboard_theme()?.is_some() {
            let bytes = read_file(&work, MAX_LAYOUT_BYTES)?;
            object(bytes.as_deref())?;
            if self.dashboard_theme()?.is_some() {
                bytes
            } else {
                Some(serde_json::to_vec(&intent.layout).map_err(|_| Problem::Storage)?)
            }
        } else {
            // A late edit from another profile replaced the working file after our rename.
            // Keep the intended layout in this profile; the regular profile sync handles that edit.
            Some(serde_json::to_vec(&intent.layout).map_err(|_| Problem::Storage)?)
        };
        write_private(&files.mirror, bytes.as_deref().unwrap_or(b"{}"), 0o644)?;
        write_private(&files.bank, &bank_bytes(&intent.bank)?, 0o600)?;
        remove_intent(&files.pending)?;
        self.index.work.layout = digest(bytes.as_deref());
        Ok(true)
    }

    fn edit_dashboard(&mut self, edit: Edit, stamped: bool) -> Result<(Snapshot, Vec<Value>), Problem> {
        if !self.ready {
            return Err(Problem::Unavailable);
        }
        if edit.profile != self.index.active || self.index.switching.is_some() {
            return Err(Problem::ProfileChanged);
        }
        let created = self.records[self.active_id()].created;
        if (stamped && (created == 0 || edit.profile_created.is_none()))
            || edit.profile_created.is_some_and(|got| got != created)
        {
            return Err(Problem::ProfileChanged);
        }
        let mut snap = self.dashboard_snapshot()?;
        if edit.revision != snap.revision(self) {
            return Err(Problem::Stale);
        }
        if !matches!(edit.action, Action::Save) && (edit.name.is_some() || edit.layout.is_some() || edit.activate) {
            return Err(Problem::Invalid("name, layout and activate are only for save"));
        }
        snap.bank.generation = snap.bank.generation.checked_add(1).ok_or(Problem::Unavailable)?;
        let bank = &mut snap.bank;
        let apply = match edit.action {
            Action::Save => {
                let creating = edit.id.is_empty();
                let name = edit.name.as_deref().map(dashboard_name).transpose()?;
                if let Some(name) = &name {
                    bank.check_name(&edit.id, name)?;
                }
                let pos = if creating {
                    if bank.dashboards.len() >= MAX_DASHBOARDS {
                        return Err(Problem::Full);
                    }
                    let name = name.ok_or(Problem::Invalid("a new dashboard needs a name"))?;
                    let layout = match &edit.layout {
                        Some(draft) => layouts::edit(&snap.working, draft)?,
                        None => snap.working.clone(),
                    };
                    layouts::validate_activation(&layout)?;
                    let pos = bank.dashboards.len();
                    bank.dashboards.push(Dashboard { id: format!("dashboard-{}", bank.generation), name, layout: Some(layout) });
                    pos
                } else {
                    let pos = bank.position(&edit.id)?;
                    if let Some(name) = name {
                        bank.dashboards[pos].name = name;
                    }
                    pos
                };
                let target = &mut bank.dashboards[pos];
                let layout = if creating { None } else { edit.layout.as_ref() };
                if let Some(draft) = layout {
                    if let Some(base) = &target.layout {
                        target.layout = Some(layouts::edit(base, draft)?);
                        if edit.activate { bank.activate(pos, &snap.working) } else { None }
                    } else {
                        Some(layouts::edit(&snap.working, draft)?)
                    }
                } else if edit.activate {
                    bank.activate(pos, &snap.working)
                } else {
                    None
                }
            }
            Action::Activate => {
                let pos = bank.position(&edit.id)?;
                bank.activate(pos, &snap.working)
            }
            Action::Delete => {
                let pos = bank.position(&edit.id)?;
                if edit.id == bank.active {
                    return Err(Problem::Active);
                }
                bank.dashboards.remove(pos);
                None
            }
        };
        let layouts = snap.layouts(apply.as_ref().unwrap_or(&snap.working))?;
        self.commit_dashboard(&mut snap, apply)?;
        Ok((snap, layouts))
    }
}

fn bank_bytes(bank: &impl Serialize) -> Result<Vec<u8>, Problem> {
    let bytes = serde_json::to_vec(bank).map_err(|_| Problem::Storage)?;
    if bytes.len() > MAX_BANK_BYTES {
        return Err(Problem::Invalid("the dashboards use too much storage"));
    }
    Ok(bytes)
}

pub(super) fn handle(profiles: &Profiles, request: &Request) -> Response {
    if !request.peer.to_canonical().is_loopback() {
        return error_response(403, "local_only", "dashboard editing is local only");
    }
    let edit = if request.method == "POST" {
        if request.header(CLIENT_HEADER).is_none_or(str::is_empty) {
            return error_response(403, "client_required", "dashboard writes need the local client header");
        }
        match serde_json::from_slice::<Edit>(&request.body) {
            Ok(body) => Some(body),
            Err(_) => return Problem::Invalid("expected profile, revision, action and id; save also accepts name, layout and activate").response(),
        }
    } else {
        None
    };
    let result = (|| {
        let mut store = profiles.store.lock();
        if !store.ready {
            return Err(Problem::Unavailable);
        }
        let _layout_lock = store.lock_layout().map_err(|e| match e {
            Error::LayoutBusy => Problem::Busy,
            _ => Problem::Unavailable,
        })?;
        let (snap, layouts) = match edit {
            Some(edit) => store.edit_dashboard(edit, request.header(CLIENT_HEADER) == Some(BRIDGE_CLIENT))?,
            None => {
                let snap = store.dashboard_snapshot()?;
                let layouts = snap.layouts(&snap.working)?;
                (snap, layouts)
            }
        };
        Ok(snap.body(&store, &profiles.hub, layouts))
    })();
    match result {
        Ok(data) => Response::json(200, &json!({"ok": true, "api": crate::API_VERSION, "now": now_ms(), "data": data})),
        Err(e) => e.response(),
    }
}

#[cfg(test)]
mod tests;
