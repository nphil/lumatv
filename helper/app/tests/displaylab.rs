//! The display lab behind the helper's real server: who is answered, and what a machine that is not the TV does with it. The lab's own rules
//! (the list, the journal, the dead-man, every way out) are held by its unit tests against a set in memory (`src/displaylab/`); here it is the
//! door: the loopback API's guards and the lab's own, in front of the real `Service`, on a host that has no LG nodes at all.

mod support;

use serde_json::{Value, json};
use support::*;

const SHELL: &[(&str, &str)] = &[
    ("X-Luma-Client", "lumatv-ui"),
    ("Content-Type", "application/json"),
];

fn start() -> Rig {
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    rig.start(test_config(json!({})), Script::new());
    rig
}

fn code(r: &Reply) -> String {
    r.json()["error"]["code"].as_str().unwrap_or("").to_owned()
}

#[test]
fn the_lab_answers_the_tvs_own_shell_only_and_a_machine_that_is_not_the_tv_says_so() {
    let rig = start();
    let addr = rig.addr;
    // any other client name, or none, is refused by the lab (the API's write guard has let the first through, or refused the second)
    let r = post(addr, "/v1/displaylab/begin", "");
    assert_eq!(
        (r.status, code(&r)),
        (403, "not_the_ui".to_owned()),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let r = post(
        addr,
        "/v1/displaylab/set",
        r#"{"control":"sharpness","option":"strong"}"#,
    );
    assert_eq!((r.status, code(&r)), (403, "not_the_ui".to_owned()));
    let r = get(addr, "/v1/displaylab");
    assert_eq!(
        (r.status, code(&r)),
        (403, "not_the_ui".to_owned()),
        "a read needs the shell's name too"
    );
    let r = http(
        addr,
        "POST",
        "/v1/displaylab/set",
        &[
            ("X-Luma-Client", "home"),
            ("Content-Type", "application/json"),
        ],
        Some("{}"),
    );
    assert_eq!((r.status, code(&r)), (403, "not_the_ui".to_owned()));
    // a web page (an Origin the API does not allow) never gets as far
    let r = http(
        addr,
        "POST",
        "/v1/displaylab/begin",
        &[
            ("X-Luma-Client", "lumatv-ui"),
            ("Content-Type", "application/json"),
            ("Origin", "http://evil.example"),
        ],
        Some(""),
    );
    assert_eq!(r.status, 403);

    // the shell: this host has no LG nodes, so the lab is not available and nothing is read or written
    let r = http(addr, "GET", "/v1/displaylab", SHELL, None);
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let doc = r.json();
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["displaylab"]["available"], false);
    assert!(
        doc["displaylab"]["reason"]
            .as_str()
            .unwrap()
            .contains("not an LG set"),
        "{doc}"
    );
    assert_eq!(
        doc["displaylab"]["controls"]["sharpness"],
        json!(["normal", "strong", "soft", "off"])
    );
    let r = http(addr, "POST", "/v1/displaylab/begin", SHELL, Some(""));
    assert_eq!((r.status, code(&r)), (503, "unavailable".to_owned()));
    let r = http(
        addr,
        "POST",
        "/v1/displaylab/set",
        SHELL,
        Some(r#"{"control":"sharpness","option":"strong"}"#),
    );
    assert_eq!((r.status, code(&r)), (409, "no_session".to_owned()));
    // a request that names an address is a bad request before it is anything else
    let r = http(
        addr,
        "POST",
        "/v1/displaylab/set",
        SHELL,
        Some(r#"{"address":"0xCC480360","value":"0x0"}"#),
    );
    assert_eq!((r.status, code(&r)), (400, "bad_request".to_owned()));
    let r = http(addr, "POST", "/v1/displaylab/regs", SHELL, Some("{}"));
    assert_eq!(r.status, 404);
}

#[test]
fn the_discovery_document_lists_the_labs_routes() {
    let rig = start();
    let doc = get(rig.addr, "/v1").json();
    let listed: Vec<String> = doc["endpoints"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["path"].as_str().map(str::to_owned))
        .collect();
    for path in [
        "/v1/displaylab",
        "/v1/displaylab/begin",
        "/v1/displaylab/set",
        "/v1/displaylab/ping",
        "/v1/displaylab/end",
        "/v1/displaylab/results",
    ] {
        assert!(listed.iter().any(|p| p == path), "{path} is not listed");
    }
}

#[test]
fn the_results_of_a_run_are_kept_for_the_next_round() {
    let rig = start();
    let run = json!({"run": "r1", "steps": [{"id": "sharpness-strong", "answer": "b"}]});
    let r = http(
        rig.addr,
        "POST",
        "/v1/displaylab/results",
        SHELL,
        Some(&run.to_string()),
    );
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let back: Value = http(rig.addr, "GET", "/v1/displaylab/results", SHELL, None).json();
    assert_eq!(back["results"]["run"], "r1");
    let on_disk: Value = serde_json::from_slice(
        &std::fs::read(rig.paths.data_dir.join("displaylab/results.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(on_disk["steps"][0]["id"], "sharpness-strong");
}

#[test]
fn a_journal_waits_for_a_start_that_can_reach_the_hardware() {
    // a run that died with a register changed, found by a helper on a machine that cannot reach one: nothing is written, and the journal stays
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let dir = rig.paths.data_dir.join("displaylab");
    std::fs::create_dir_all(&dir).unwrap();
    let journal = json!({"v": 1, "session": 3, "started_ms": 1, "pid": 1, "targets": {"osd0_vsp": {"orig": 0, "written": 0x0100_0001}}});
    std::fs::write(dir.join("journal.json"), journal.to_string()).unwrap();
    rig.start(test_config(json!({})), Script::new());
    let doc = http(rig.addr, "GET", "/v1/displaylab", SHELL, None).json();
    assert_eq!(doc["displaylab"]["available"], false);
    rig.stop();
    assert!(
        dir.join("journal.json").exists(),
        "the journal is for a start that can put it back"
    );
}

#[test]
fn a_lab_that_is_switched_off_in_the_config_refuses_every_route() {
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    rig.start(
        test_config(json!({"displayLab": {"enabled": false}})),
        Script::new(),
    );
    for (method, path) in [
        ("GET", "/v1/displaylab"),
        ("POST", "/v1/displaylab/begin"),
        ("POST", "/v1/displaylab/set"),
    ] {
        let r = http(
            rig.addr,
            method,
            path,
            SHELL,
            (method == "POST").then_some("{}"),
        );
        assert_eq!(
            (r.status, code(&r)),
            (403, "disabled".to_owned()),
            "{method} {path}"
        );
    }
}
