use super::*;
use crate::hub::Hub;
use lumatv_home_grid::grid;
use parking_lot::Mutex;
use std::sync::Arc;

struct Rig {
    profiles: Profiles,
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let theme = dir.path().join("theme");
        fs::create_dir(&theme).unwrap();
        fs::write(theme.join("theme.json"), r##"{"material":"light","accent":"#112233","password":"theme-secret","clock":{"style":"analog"}}"##).unwrap();
        let mut layout = grid::shipped();
        layout["settings"] = json!({"private": "layout-secret"});
        layout["widgets"][0]["settings"] = json!({"style": "digital", "token": "widget-secret"});
        fs::write(theme.join("layout.json"), serde_json::to_vec(&layout).unwrap()).unwrap();
        Self { profiles: Self::profiles(dir.path()), dir }
    }

    fn profiles(dir: &Path) -> Profiles {
        Profiles { store: Mutex::new(Store::open(dir)), hub: Arc::new(Hub::new(1)) }
    }

    fn call(&self, method: &str, body: &Value) -> (u16, Value) {
        let request = request(method, body);
        let response = self.profiles.handle(&request, &["dashboards"]);
        (response.status, serde_json::from_slice(&response.body).unwrap())
    }

    fn get(&self) -> Value {
        let (status, body) = self.call("GET", &Value::Null);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["ok"], true);
        body["data"].clone()
    }

    fn post(&self, snap: &Value, action: &str, id: &str, patch: Value) -> (u16, Value) {
        self.call("POST", &body(snap, action, id, patch))
    }

    fn working(&self) -> Value {
        serde_json::from_slice(&fs::read(self.work_file()).unwrap()).unwrap()
    }

    fn work_file(&self) -> PathBuf {
        self.dir.path().join("theme/layout.json")
    }

    fn native_edit(&self, edit: impl FnOnce(&mut Value)) {
        let mut layout = self.working();
        edit(&mut layout);
        fs::write(self.work_file(), serde_json::to_vec(&layout).unwrap()).unwrap();
    }
}

fn request(method: &str, body: &Value) -> Request {
    Request {
        method: method.into(),
        path: "/v1/profiles/dashboards".into(),
        query: Vec::new(),
        headers: vec![(CLIENT_HEADER.into(), "lumatv-dashboard-bridge".into())],
        body: serde_json::to_vec(body).unwrap(),
        peer: "127.0.0.1".parse().unwrap(),
    }
}

fn body(snap: &Value, action: &str, id: &str, patch: Value) -> Value {
    let mut body = json!({"profile": snap["profile"], "profileCreated": snap["profileCreated"],
        "revision": snap["revision"], "action": action, "id": id});
    body.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
    body
}

fn active(snap: &Value) -> &Value {
    snap["dashboards"].as_array().unwrap().iter().find(|d| d["id"] == snap["active"]).unwrap()
}

fn dock(snap: &Value) -> Value {
    grid::widgets(&active(snap)["layout"]).iter().find(|w| w["type"] == "dock").unwrap().clone()
}

fn refused(reply: (u16, Value), status: u16, code: &str) {
    assert_eq!(reply.0, status, "{}", reply.1);
    assert_eq!(reply.1["ok"], false);
    assert_eq!(reply.1["error"]["code"], code);
    assert!(reply.1.get("data").is_none());
}

#[test]
fn migration_previews_current_tiles_and_safe_display_data_without_writing() {
    let rig = Rig::new();
    let before = fs::read(rig.work_file()).unwrap();
    let files = rig.profiles.store.lock().dashboard_files();
    rig.profiles.hub.set(crate::homefeeds::WEATHER_TOPIC, json!({
        "doc": {"zip":"private-zip", "display":{"place":"Atlanta", "temp":"74°", "nowKind":"sun", "days":[]}}
    }));
    rig.profiles.hub.set(crate::homefeeds::NEWS_TOPIC, json!({
        "doc": {"items":[{"title":"A story", "link":"http://private.news/", "img":"http://private.image/"}]}
    }));
    let view = rig.get();
    assert_eq!(view["profile"], "nitin");
    let reply = rig.profiles.handle(&request("GET", &Value::Null), &[]);
    let profiles: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(profiles["profiles"]["profiles"][0]["profileCreated"], view["profileCreated"]);
    assert_eq!(view["active"], "home");
    assert_eq!(active(&view)["name"], "Home");
    assert_eq!(active(&view)["layout"]["grid"]["rows"], 16);
    assert_eq!(active(&view)["layout"]["widgets"][0]["rows"], 4);
    assert_eq!(active(&view)["layout"]["widgets"][0]["settings"], json!({"style":"digital"}));
    assert_eq!(view["theme"]["mode"], "light");
    assert_eq!(view["theme"]["accent"], "#112233");
    assert_eq!(view["widgets"]["weather"]["kind"], "sun");
    assert_eq!(view["widgets"]["news"]["items"][0], json!({"title":"A story"}));
    assert!(!view.to_string().contains("secret"));
    assert!(!view.to_string().contains("private-"));
    assert_eq!(rig.get()["revision"], view["revision"]);
    assert_eq!(fs::read(rig.work_file()).unwrap(), before);
    assert!(!files.bank.exists());
    assert!(!files.pending.exists());
}

#[test]
fn an_absent_layout_uses_the_shipped_home_but_a_missing_dock_is_not_invented() {
    let rig = Rig::new();
    fs::remove_file(rig.work_file()).unwrap();
    let view = rig.get();
    assert_eq!(dock(&view)["id"], "dock");
    assert!(!rig.work_file().exists());
    fs::write(rig.work_file(), br#"{"settings":{"untouched":true}}"#).unwrap();
    assert_eq!(dock(&rig.get())["id"], "dock");
    for native in [
        json!({"widgets":[]}),
        json!({"widgets":[{"id":"dock","type":"dock","hidden":true}]}),
        json!({"widgets":[{"id":"dock","type":"dock","span":"bad"}]}),
    ] {
        fs::write(rig.work_file(), serde_json::to_vec(&native).unwrap()).unwrap();
        refused(rig.call("GET", &Value::Null), 503, "dashboard_unavailable");
        assert_eq!(rig.working(), native);
    }
}

#[test]
fn saves_and_activates_named_layouts_without_replacing_native_settings() {
    let rig = Rig::new();
    let initial = rig.get();
    let original = rig.working();
    let clock = active(&initial)["layout"]["widgets"][0].clone();
    let layout = json!({"widgets": [dock(&initial), clock, {
        "id":"clock-2", "type":"clock", "col":4,"row":0,"span":4,"rows":4,
        "settings":{"style":"analog"}
    }]});
    let (status, response) = rig.post(&initial, "save", "", json!({"name":" Evening ", "layout": layout}));
    assert_eq!(status, 200, "{response}");
    let saved = &response["data"];
    let id = saved["dashboards"][1]["id"].as_str().unwrap();
    assert_eq!(saved["active"], "home");
    assert_eq!(saved["dashboards"][1]["name"], "Evening");
    assert_eq!(rig.working(), original);
    refused(rig.post(&initial, "save", "", json!({"name":"Replay"})), 409, "stale_revision");
    refused(rig.post(saved, "save", "", json!({"name":"evening"})), 409, "duplicate_name");
    let (status, response) = rig.post(saved, "activate", id, json!({}));
    assert_eq!(status, 200, "{response}");
    let activated = &response["data"];
    assert_eq!(activated["active"], id);
    assert_eq!(grid::widgets(&active(activated)["layout"]).len(), 3);
    let work = rig.working();
    assert_eq!(work["settings"], original["settings"]);
    let clock_at = grid::index_of(&work, "clock").unwrap();
    assert_eq!(grid::widgets(&work)[clock_at]["settings"], original["widgets"][0]["settings"]);
    let fresh_at = grid::index_of(&work, "clock-2").unwrap();
    assert_eq!(grid::widgets(&work)[fresh_at]["settings"], json!({"style":"analog"}));
    refused(rig.post(activated, "delete", id, json!({})), 409, "active_dashboard");
    let (status, response) = rig.post(activated, "save", id, json!({"name":"Night"}));
    assert_eq!(status, 200, "{response}");
    assert_eq!(active(&response["data"])["name"], "Night");
    let (status, response) = rig.post(&response["data"], "activate", "home", json!({}));
    assert_eq!(status, 200, "{response}");
    assert_eq!(rig.working(), original);
    let (status, response) = rig.post(&response["data"], "delete", id, json!({}));
    assert_eq!(status, 200, "{response}");
    assert_eq!(response["data"]["dashboards"].as_array().unwrap().len(), 1);
}

#[test]
fn a_dock_only_home_can_add_every_kind_of_native_widget() {
    let rig = Rig::new();
    let start = rig.get();
    let (status, blank) = rig.post(&start, "save", "", json!({
        "name":"From scratch", "layout":{"widgets":[dock(&start)]}, "activate":true,
    }));
    assert_eq!(status, 200, "{blank}");
    let blank = &blank["data"];
    let id = blank["active"].as_str().unwrap();
    let layout = json!({"widgets": [
        dock(blank),
        {"id":"clock","type":"clock","col":0,"row":0,"span":2,"rows":3},
        {"id":"home","type":"homeassistant","col":2,"row":0,"span":2,"rows":3},
        {"id":"weather","type":"weather","col":4,"row":0,"span":2,"rows":3},
        {"id":"news","type":"news","col":6,"row":0,"span":3,"rows":4},
        {"id":"cameras","type":"cameras","col":9,"row":0,"span":3,"rows":3},
        {"id":"onhome","type":"onhome","col":0,"row":4,"span":3,"rows":3},
    ]});
    let (status, saved) = rig.post(blank, "save", id, json!({"layout":layout}));
    assert_eq!(status, 200, "{saved}");
    let saved = active(&saved["data"])["layout"]["widgets"].as_array().unwrap();
    for kind in grid::ADDABLE {
        assert!(saved.iter().any(|w| w["type"] == *kind));
    }
    assert_eq!(saved[2]["title"], "At home");
    assert_eq!(saved[6]["title"], "Smart home");
}

#[test]
fn activation_checks_current_profile_and_switching_away_and_back_expires_old_revisions() {
    let rig = Rig::new();
    let initial = rig.get();
    let (status, body) = rig.post(&initial, "save", "", json!({"name":"Private evening"}));
    assert_eq!(status, 200, "{body}");
    let nitin = &body["data"];
    rig.profiles.store.lock().switch("isabel", None).unwrap();
    let isabel = rig.get();
    assert_eq!(isabel["profile"], "isabel");
    assert_eq!(isabel["dashboards"].as_array().unwrap().len(), 1);
    assert!(!isabel.to_string().contains("Private evening"));
    refused(rig.post(nitin, "delete", "dashboard-1", json!({})), 409, "profile_changed");
    assert_eq!(rig.get()["revision"], isabel["revision"]);
    rig.profiles.store.lock().switch("nitin", None).unwrap();
    refused(rig.post(nitin, "delete", "dashboard-1", json!({})), 409, "stale_revision");
    assert_eq!(rig.get()["dashboards"][1]["name"], "Private evening");
}

#[test]
fn native_layout_edits_change_the_revision_and_survive_activation() {
    let rig = Rig::new();
    let initial = rig.get();
    let (status, response) = rig.post(&initial, "save", "", json!({"name":"Blank", "layout":{"widgets":[dock(&initial)]}}));
    assert_eq!(status, 200, "{response}");
    let before = &response["data"];
    let id = before["dashboards"][1]["id"].as_str().unwrap();
    rig.native_edit(|layout| {
        layout["widgets"][0]["span"] = json!(3);
        layout["settings"]["external"] = json!(true);
    });
    refused(rig.post(before, "activate", id, json!({})), 409, "stale_revision");
    let edited = rig.get();
    assert_eq!(active(&edited)["layout"]["widgets"][0]["span"], 3);
    let (status, response) = rig.post(&edited, "activate", id, json!({}));
    assert_eq!(status, 200, "{response}");
    let (status, response) = rig.post(&response["data"], "activate", "home", json!({}));
    assert_eq!(status, 200, "{response}");
    assert_eq!(rig.working()["settings"]["external"], true);
    assert_eq!(rig.working()["widgets"][0]["span"], 3);
}

#[test]
fn invalid_layouts_cannot_change_the_working_file_or_dashboard_revision() {
    let rig = Rig::new();
    let view = rig.get();
    let before = rig.working();
    let dock = dock(&view);
    let clock = |id: &str, col: i64, row: i64| json!({"id":id, "type":"clock", "col":col, "row":row,"span":4,"rows":4});
    let mut hidden = dock.clone();
    hidden["hidden"] = json!(true);
    let mut secret = clock("other", 0, 0);
    secret["settings"] = json!({"action":"not permitted"});
    for draft in [
        json!({"widgets":[]}),
        json!({"widgets":[hidden]}),
        json!({"widgets":[dock, clock("one", 0, 0), clock("one", 4, 0)]}),
        json!({"widgets":[dock, clock("one", 1, 0), clock("two", 4, 0)]}),
        json!({"widgets":[dock, clock("one", i64::MAX, 0)]}),
        json!({"widgets":[dock, secret]}),
        json!({"widgets":[dock], "grid":{"cols":24,"rows":16,"gutter":20}}),
    ] {
        refused(rig.post(&view, "save", "home", json!({"layout":draft, "activate":true})), 400, "bad_request");
    }
    assert_eq!(rig.working(), before);
    assert_eq!(rig.get()["revision"], view["revision"]);
}

#[test]
fn a_failed_mirror_write_can_resume_without_overwriting_a_later_native_edit() {
    let rig = Rig::new();
    let initial = rig.get();
    let files = rig.profiles.store.lock().dashboard_files();
    fs::remove_file(&files.mirror).unwrap();
    fs::create_dir(&files.mirror).unwrap();
    refused(rig.post(&initial, "save", "", json!({"name":"New Home", "layout":{"widgets":[dock(&initial)]}, "activate":true})), 500, "io");
    refused(rig.call("GET", &Value::Null), 503, "dashboard_unavailable");
    assert_eq!(grid::widgets(&rig.working()).len(), 1);
    rig.native_edit(|l| l["widgets"][0]["tile"] = json!(128));
    fs::remove_dir(&files.mirror).unwrap();
    let reopened = Rig::profiles(rig.dir.path());
    let recovered = Rig { profiles: reopened, dir: rig.dir };
    let selected = recovered.get();
    assert_eq!(active(&selected)["name"], "New Home");
    assert_eq!(active(&selected)["layout"]["widgets"][0]["tile"], 128);
    assert_eq!(selected["dashboards"][0]["layout"], active(&initial)["layout"]);
    assert!(!files.pending.exists());
    recovered.profiles.store.lock().switch("isabel", None).unwrap();
    recovered.profiles.store.lock().switch("nitin", None).unwrap();
    assert_eq!(recovered.working()["widgets"][0]["tile"], 128);
}

#[test]
fn intent_recovery_does_not_copy_another_profiles_late_layout() {
    let rig = Rig::new();
    let initial = rig.get();
    let files = rig.profiles.store.lock().dashboard_files();
    fs::remove_file(&files.mirror).unwrap();
    fs::create_dir(&files.mirror).unwrap();
    refused(rig.post(&initial, "save", "", json!({"name":"New Home", "layout":{"widgets":[dock(&initial)]}, "activate":true})), 500, "io");
    assert!(!files.staged.exists());
    let theme_path = rig.dir.path().join("theme/theme.json");
    let mut late_theme = super::super::read_json(&theme_path).unwrap();
    late_theme["_profile"] = json!("isabel");
    fs::write(theme_path, serde_json::to_vec(&late_theme).unwrap()).unwrap();
    let mut late_layout = grid::shipped();
    late_layout["settings"] = json!({"late_isabel": true});
    fs::write(rig.work_file(), serde_json::to_vec(&late_layout).unwrap()).unwrap();
    fs::remove_dir(&files.mirror).unwrap();
    let reopened = Rig { profiles: Rig::profiles(rig.dir.path()), dir: rig.dir };
    assert!(reopened.profiles.store.lock().sync());
    let active_home = reopened.get();
    assert_eq!(active(&active_home)["name"], "New Home");
    assert_eq!(grid::widgets(&reopened.working()).len(), 1);
    assert!(!reopened.working().to_string().contains("late_isabel"));
    reopened.profiles.store.lock().switch("isabel", None).unwrap();
    assert_eq!(reopened.working(), late_layout);
}

#[test]
fn a_prepared_activation_yields_to_an_edit_before_its_rename() {
    let rig = Rig::new();
    let view = rig.get();
    let (status, response) = rig.post(&view, "save", "", json!({"name":"Blank", "layout":{"widgets":[dock(&view)]}}));
    assert_eq!(status, 200, "{response}");
    let mut store = rig.profiles.store.lock();
    let mut snap = store.dashboard_snapshot().unwrap();
    let files = store.dashboard_files();
    let chosen = snap.bank.activate(1, &snap.working).unwrap();
    let intent = Intent { bank: snap.bank, layout: chosen, before: snap.working_digest };
    write_private(&files.staged, &serde_json::to_vec(&intent.layout).unwrap(), 0o644).unwrap();
    write_private(&files.pending, &bank_bytes(&intent).unwrap(), 0o600).unwrap();
    rig.native_edit(|l| l["widgets"][0]["span"] = json!(3));
    assert_eq!(store.finish_dashboard(), Ok(false));
    assert_eq!(store.dashboard_snapshot().unwrap().bank.active, "home");
    assert_eq!(rig.working()["widgets"][0]["span"], 3);
    assert!(!files.pending.exists());
}

#[test]
fn dashboard_guide_is_only_for_new_profiles_and_can_only_be_cleared_while_active() {
    let rig = Rig::new();
    let mut store = rig.profiles.store.lock();
    assert!(!store.records["isabel"].dashboard_first_run);
    let new = store.create("Dana", None, None).unwrap();
    assert!(store.records[&new].dashboard_first_run);
    let done = json!({"dashboardFirstRun":false});
    assert!(store.update(&new, &done).is_err());
    store.switch(&new, None).unwrap();
    let layout = store.dashboard_snapshot().unwrap().working;
    assert_eq!(grid::widgets(&layout).len(), 1);
    assert_eq!(grid::widgets(&layout)[0]["type"], "dock");
    assert!(layout.get("settings").is_none());
    let created = store.records[&new].created;
    let wrong_stamp = json!({"dashboardFirstRun":false, "profileCreated": created - 1});
    assert_eq!(store.update(&new, &wrong_stamp), Err(super::Error::ProfileChanged));
    assert!(store.records[&new].dashboard_first_run);
    store.update(&new, &json!({"dashboardFirstRun":false, "profileCreated": created})).unwrap();
    assert!(store.records[&new].first_run);
    let saved = Store::open(rig.dir.path());
    assert!(!saved.records[&new].dashboard_first_run);
    assert!(saved.records[&new].first_run);
}

#[test]
fn dashboard_requests_keep_local_client_guards() {
    let rig = Rig::new();
    let mut nonlocal = request("GET", &Value::Null);
    nonlocal.peer = "192.168.1.25".parse().unwrap();
    assert_eq!(rig.profiles.handle(&nonlocal, &["dashboards"]).status, 403);
    let mut unmarked = request("POST", &body(&rig.get(), "save", "", json!({"name":"New"})));
    unmarked.headers.clear();
    assert_eq!(rig.profiles.handle(&unmarked, &["dashboards"]).status, 403);
}

#[test]
fn a_recreated_profiles_stamp_is_not_the_same_account() {
    let rig = Rig::new();
    let id = "isabel";
    {
        let mut store = rig.profiles.store.lock();
        // The clock may have gone backward since a legacy account was first created.
        store.records.get_mut(id).unwrap().created = now_ms() + 10_000;
        store.switch(id, None).unwrap();
    }
    let old = rig.get();
    {
        let mut store = rig.profiles.store.lock();
        store.switch("nitin", None).unwrap();
        store.remove(id).unwrap();
    }
    let reopened = Rig::profiles(rig.dir.path());
    let rig = Rig { profiles: reopened, dir: rig.dir };
    {
        let mut store = rig.profiles.store.lock();
        assert_eq!(store.create("Isabel", None, None).unwrap(), id);
        store.switch(id, None).unwrap();
    }
    let fresh = rig.get();
    assert_eq!(fresh["profile"], old["profile"]);
    assert!(fresh["profileCreated"].as_u64().unwrap() > old["profileCreated"].as_u64().unwrap());
    refused(rig.post(&old, "save", "", json!({"name":"Wrong account"})), 409, "profile_changed");
    let mut missing = body(&fresh, "save", "", json!({"name":"Missing stamp"}));
    missing.as_object_mut().unwrap().remove("profileCreated");
    refused(rig.call("POST", &missing), 409, "profile_changed");
    assert_eq!(rig.get()["revision"], fresh["revision"]);
    let mut native = request("POST", &missing);
    native.headers[0].1 = crate::http::UI_CLIENT.into();
    assert_eq!(rig.profiles.handle(&native, &["dashboards"]).status, 200);
}

#[test]
fn the_same_layout_lock_serializes_legacy_settings_and_dashboard_writes() {
    let rig = Rig::new();
    let view = rig.get();
    let native_lock = rig.profiles.store.lock().lock_layout().unwrap();
    refused(rig.post(&view, "save", "", json!({"name":"New"})), 409, "home_busy");
    assert!(matches!(rig.profiles.store.lock().switch("isabel", None), Err(super::Error::LayoutBusy)));
    rig.native_edit(|l| l["widgets"][0]["span"] = json!(3));
    drop(native_lock);
    refused(rig.post(&view, "save", "", json!({"name":"New"})), 409, "stale_revision");
    assert_eq!(active(&rig.get())["layout"]["widgets"][0]["span"], 3);
}
