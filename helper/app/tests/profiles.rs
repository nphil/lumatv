//! Local profiles over the real service and real TCP: the first start makes Nitin and Isabel from `theme/`, a switch over HTTP puts the other person's
//! part in the working copy and is told on the topic, the helper's thread carries an edit back without a request, and the rules answer with statuses.

mod support;

use serde_json::{Value, json};
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);

/// A TV that has been used: a look of his own, a household setting, a layout.
fn rig_with_a_home() -> Rig {
    let mut rig = Rig::new();
    let theme = rig.paths.data_dir.join("theme");
    std::fs::create_dir_all(&theme).unwrap();
    std::fs::write(
        theme.join("theme.json"),
        r##"{"accent": "#EB6F92", "oledcare": {"enabled": true}}"##,
    )
    .unwrap();
    std::fs::write(theme.join("layout.json"), r#"{"widgets": []}"#).unwrap();
    rig.start(test_config(json!({})), Script::internet());
    rig
}

fn working_theme(rig: &Rig) -> Value {
    let text = std::fs::read_to_string(rig.paths.data_dir.join("theme/theme.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn the_first_start_makes_nitin_and_isabel_and_the_topic_says_who_is_active() {
    let rig = rig_with_a_home();
    let view = get(rig.addr, "/v1/profiles").json();
    assert_eq!(view["ok"], true);
    let p = &view["profiles"];
    assert_eq!(
        (
            p["ready"].as_bool(),
            p["active"].as_str(),
            p["owner"].as_str(),
            p["canAdmin"].as_bool()
        ),
        (Some(true), Some("nitin"), Some("nitin"), Some(true))
    );
    let names: Vec<&str> = p["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Nitin", "Isabel"]);
    let topic = rig.topic("profiles").expect("the topic is set at start");
    assert_eq!(topic["active"], "nitin");
    // what he had is still what Home reads: his key, the household's key, and the mark that says whose
    let t = working_theme(&rig);
    assert_eq!(
        (
            t["accent"].as_str(),
            t["oledcare"]["enabled"].as_bool(),
            t["_profile"].as_str()
        ),
        (Some("#EB6F92"), Some(true), Some("nitin"))
    );
}

#[test]
fn a_switch_over_http_puts_the_other_persons_part_in_the_working_copy_and_the_topic_follows() {
    let rig = rig_with_a_home();
    let r = post(rig.addr, "/v1/profiles/switch", r#"{"id": "isabel"}"#);
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert!(r.json()["ms"].as_f64().unwrap() < 500.0);
    let t = working_theme(&rig);
    assert_eq!(t["_profile"], "isabel");
    assert_eq!(t["oledcare"]["enabled"], true, "the household's key stays");
    assert!(t.get("accent").is_none(), "his accent is his");
    let topic = rig
        .wait_topic("profiles", WAIT, |v| v["active"] == "isabel")
        .expect("the topic follows");
    assert_eq!(topic["canAdmin"], false);
    assert!(post(rig.addr, "/v1/profiles/switch", r#"{"id": "ghost"}"#).status == 404);
    // she is not the owner: she cannot add a profile or change the options
    let r = post(rig.addr, "/v1/profiles", r#"{"name": "Mia"}"#);
    assert_eq!(
        (
            r.status,
            r.json()["error"]["code"].as_str().map(str::to_owned)
        ),
        (403, Some("owner_only".to_owned()))
    );
    assert_eq!(
        post(
            rig.addr,
            "/v1/profiles/options",
            r#"{"pickerOnWake": true}"#
        )
        .status,
        403
    );
    // and back
    assert_eq!(
        post(rig.addr, "/v1/profiles/switch", r#"{"id": "nitin"}"#).status,
        200
    );
    assert_eq!(working_theme(&rig)["accent"], "#EB6F92");
}

#[test]
fn a_pin_is_asked_over_http_and_the_wrong_one_is_counted() {
    let rig = rig_with_a_home();
    assert_eq!(
        post(rig.addr, "/v1/profiles/nitin/pin", r#"{"pin": "2468"}"#).status,
        200
    );
    assert_eq!(
        post(rig.addr, "/v1/profiles/switch", r#"{"id": "isabel"}"#).status,
        200
    );
    let r = post(rig.addr, "/v1/profiles/switch", r#"{"id": "nitin"}"#);
    assert_eq!(
        (
            r.status,
            r.json()["error"]["code"].as_str().map(str::to_owned)
        ),
        (403, Some("pin_required".to_owned()))
    );
    let r = post(
        rig.addr,
        "/v1/profiles/switch",
        r#"{"id": "nitin", "pin": "0000"}"#,
    );
    assert_eq!(
        (r.status, r.json()["error"]["triesLeft"].as_u64()),
        (403, Some(4))
    );
    assert_eq!(
        post(
            rig.addr,
            "/v1/profiles/switch",
            r#"{"id": "nitin", "pin": "2468"}"#
        )
        .status,
        200
    );
    let topic = rig.topic("profiles").unwrap().to_string();
    assert!(!topic.contains("2468") && !topic.contains("hash") && !topic.contains("salt"));
}

#[test]
fn what_home_writes_reaches_the_profile_without_anyone_asking() {
    let rig = rig_with_a_home();
    let mut t = working_theme(&rig);
    t["accent"] = json!("#00AA00");
    std::fs::write(
        rig.paths.data_dir.join("theme/theme.json"),
        serde_json::to_string_pretty(&t).unwrap(),
    )
    .unwrap();
    let own = rig.paths.data_dir.join("profiles/nitin/theme.json");
    assert!(
        wait_until(WAIT, || std::fs::read_to_string(&own)
            .is_ok_and(|s| s.contains("#00AA00"))),
        "the helper's thread did not carry the edit to Nitin's file"
    );
    let text = std::fs::read_to_string(&own).unwrap();
    assert!(
        !text.contains("oledcare") && !text.contains("_profile"),
        "only his own keys: {text}"
    );
}

#[test]
fn the_profiles_are_in_the_discovery_list_and_unknown_paths_are_not_found() {
    let rig = rig_with_a_home();
    let list = get(rig.addr, "/v1").json();
    let paths: Vec<&str> = list["endpoints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"/v1/profiles") && paths.contains(&"/v1/profiles/switch"));
    assert_eq!(get(rig.addr, "/v1/profiles/gallery").status, 200);
    assert_eq!(get(rig.addr, "/v1/profiles/a/b/c").status, 404);
    assert_eq!(get(rig.addr, "/v1/profiles/switch").status, 405);
    // a write without the client header is refused before the profiles see it, as everywhere
    let r = http(
        rig.addr,
        "POST",
        "/v1/profiles/switch",
        &[("Content-Type", "application/json")],
        Some(r#"{"id": "isabel"}"#),
    );
    assert_ne!(r.status, 200);
    assert_eq!(
        get(rig.addr, "/v1/profiles").json()["profiles"]["active"],
        "nitin"
    );
}
