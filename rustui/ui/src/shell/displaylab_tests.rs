//! The display lab in the shell: the kind's keys, the wheel, the pointer, the ways out and what it asks of the helper, with Settings and Home under it.

use super::*;
use crate::kit::testkit;

/// A Home shell on `kinds` with the launch points of the dock and Home in front: what a key that was not the lab's would launch.
fn home_shell(kinds: &str) -> (ui::testing::Harness, Shell) {
    let (h, kit) = testkit::kit(1.0);
    let theme = lumatv_tokens::Tokens::shipped().json().clone();
    let mut shell = Shell::new(kit, Kinds::parse(kinds), 2000, &testkit::module_dir().join("icons/audio"), &theme, &json!({})).expect("a shell");
    shell.start();
    shell.channel(&Event::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 1_000);
    let mut lps = vec![json!({"id": "com.webos.app.discovery", "title": "Apps", "unmovable": true, "icon": "/a.png"})];
    for (id, t) in [("netflix", "Netflix"), ("youtube", "YouTube")] {
        lps.push(json!({"id": id, "title": t, "icon": format!("/{id}.png"), "iconColor": "#101010"}));
    }
    shell.channel(&Event::Topic { topic: "luna".into(), data: json!({"tag": crate::home::model::LP_TAG, "payload": {"launchPoints": lps}}) }, 1_010);
    (h, shell)
}

fn key(shell: &mut Shell, name: &str, at: u64) -> Vec<Act> {
    let mut acts = shell.key(&KeyEvent::Key { name: name.into(), down: true, repeat: false, seat: 0 }, at);
    acts.extend(shell.key(&KeyEvent::Key { name: name.into(), down: false, repeat: false, seat: 0 }, at + 30));
    acts
}

fn luna_calls(acts: &[Act], method: &str) -> Vec<Value> {
    acts.iter()
        .filter_map(|a| match a {
            Act::Channel(o) if o.path == "/v1/luna" => o.body.clone(),
            _ => None,
        })
        .filter(|b| b["uri"].as_str().is_some_and(|u| u.ends_with(method)))
        .collect()
}

fn lab_session() -> Value {
    json!({"ok": true, "displaylab": {"available": true, "session": {"id": 1, "held": true, "usable": {"sharpness": true, "dimming": true, "window": true}, "why": {}}}})
}

/// What the lab asked of the helper, as (path, body).
fn lab_calls(acts: &[Act]) -> Vec<(String, Value)> {
    acts.iter()
        .filter_map(|a| match a {
            Act::Helper(o) if o.path.starts_with("/v1/displaylab/") => Some((o.path.clone(), o.body.clone().unwrap_or(Value::Null))),
            _ => None,
        })
        .collect()
}

/// The tag a request of the helper's API was sent with, which its answer carries back.
fn tag_of(acts: &[Act], path: &str) -> u64 {
    acts.iter()
        .find_map(|a| match a {
            Act::Helper(o) if o.path == path => o.tag,
            _ => None,
        })
        .unwrap_or_else(|| panic!("no request {path} in {acts:?}"))
}

/// A Home shell with the lab, opened by `ctl shell show displaylab` and begun: the helper has said yes. Returns the clock after it.
fn open_lab(kinds: &str) -> (ui::testing::Harness, Shell, u64) {
    let (h, mut shell) = home_shell(kinds);
    shell.helper(&Event::Up(true), 1_020);
    let acts = shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "displaylab"}) }, 2_000);
    let tag = tag_of(&acts, "/v1/displaylab/begin");
    shell.helper(&Event::Answer { tag, status: 200, body: lab_session() }, 2_050);
    (h, shell, 2_100)
}

fn lab_state(shell: &Shell) -> Value {
    shell.state()["displaylab"].clone()
}

#[test]
fn the_display_lab_has_every_key_and_the_wheel_and_asks_the_helper_for_each_look_and_nothing_under_it_hears_a_key() {
    let (_h, mut shell) = home_shell("home,settings,displaylab");
    shell.helper(&Event::Up(true), 1_020);
    let report = shell.report();
    assert!(report["can"].as_array().unwrap().iter().any(|c| c == "displaylab"));
    assert!(report["wants"] == true, "Home in front wants the keys");
    assert!(!report["takes"].as_array().unwrap().iter().any(|t| t == "wheel"), "the wheel is LG's until the lab is open");
    let acts = shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "displaylab"}) }, 2_000);
    assert_eq!(lab_calls(&acts).iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["/v1/displaylab/begin"]);
    assert_eq!((lab_state(&shell)["open"].clone(), lab_state(&shell)["stage"].clone()), (json!(true), json!("intro")));
    assert!(shell.report()["takes"].as_array().unwrap().iter().any(|t| t == "wheel"), "the turns of the wheel switch between the two looks");
    assert!(shell.is_live(), "and it is drawn");
    let tag = tag_of(&acts, "/v1/displaylab/begin");
    shell.helper(&Event::Answer { tag, status: 200, body: lab_session() }, 2_050);
    assert_eq!(lab_state(&shell)["link"], "ready");
    // OK starts the first step: A is the set's own state; Home under it launches nothing
    let mut all = key(&mut shell, "ok", 2_100);
    let calls = lab_calls(&all);
    assert_eq!(calls.iter().map(|c| (c.0.as_str(), c.1["control"].as_str().unwrap(), c.1["option"].as_str().unwrap())).collect::<Vec<_>>(), [("/v1/displaylab/set", "sharpness", "normal")]);
    assert_eq!(lab_state(&shell)["step"], "sharpness-strong");
    shell.helper(&Event::Answer { tag: tag_of(&all, "/v1/displaylab/set"), status: 200, body: lab_session() }, 2_150);
    // Right and the wheel's turn each switch the look
    let acts = key(&mut shell, "right", 2_200);
    assert_eq!(lab_calls(&acts)[0].1["option"], "strong");
    shell.helper(&Event::Answer { tag: tag_of(&acts, "/v1/displaylab/set"), status: 200, body: lab_session() }, 2_250);
    let acts = key(&mut shell, "wheel", 2_300);
    assert_eq!(lab_calls(&acts)[0].1["option"], "normal", "the wheel switches back");
    all.extend(acts);
    assert!(luna_calls(&all, "/launch").is_empty(), "the keys were the lab's: Home launched nothing");
    // Back leaves the lab and the helper is asked to put everything back
    shell.helper(&Event::Answer { tag: tag_of(&all, "/v1/displaylab/set"), status: 200, body: lab_session() }, 2_350);
    let acts = key(&mut shell, "back", 2_400);
    assert_eq!(lab_state(&shell)["open"], false);
    assert!(lab_calls(&acts).iter().any(|c| c.0 == "/v1/displaylab/end"), "{acts:?}");
    assert!(!shell.report()["takes"].as_array().unwrap().iter().any(|t| t == "wheel"));
}

#[test]
fn the_press_that_closes_the_lab_does_not_reach_settings_with_its_release_and_settings_has_the_keys_again_after() {
    let (_h, mut shell) = home_shell("home,settings,displaylab");
    shell.helper(&Event::Up(true), 1_020);
    shell.open_settings(None, 1_500);
    assert_eq!(shell.state()["settings"]["zone"], "rail");
    // Settings' row asks; the lab opens over it, and the helper cannot run it: the one thing left is to leave
    shell.settings.raise(ShellRequest::DisplayLab);
    let acts = shell.tick(2_000);
    let tag = tag_of(&acts, "/v1/displaylab/begin");
    shell.helper(&Event::Answer { tag, status: 503, body: json!({"error": {"code": "unavailable", "message": "not an LG set"}}) }, 2_050);
    assert_eq!((lab_state(&shell)["open"].clone(), shell.state()["settings"]["open"].clone()), (json!(true), json!(true)), "Settings stays behind it");
    // OK on Leave closes the lab on the press. Its release is the lab's too, and OK on Settings' rail, which would enter the page, is not pressed
    shell.key(&KeyEvent::Key { name: "ok".into(), down: true, repeat: false, seat: 0 }, 2_100);
    assert_eq!(lab_state(&shell)["open"], false);
    shell.key(&KeyEvent::Key { name: "ok".into(), down: false, repeat: false, seat: 0 }, 2_130);
    assert_eq!(shell.state()["settings"]["zone"], "rail", "the release was the lab's");
    assert!(shell.lab_keys.is_empty(), "and it is forgotten");
    // the next OK is Settings': it enters the page
    key(&mut shell, "ok", 2_300);
    assert_eq!(shell.state()["settings"]["zone"], "page", "Settings has the keys again");
}

#[test]
fn leaving_by_the_home_key_another_app_or_the_screen_going_off_asks_the_helper_to_put_everything_back() {
    for (topic, data) in [("homekey", json!({})), ("foreground", json!({"app": "netflix", "home": false, "covered": true})), ("screen", json!({"on": false}))] {
        let (_h, mut shell, now) = open_lab("home,displaylab");
        let acts = key(&mut shell, "ok", now);
        shell.helper(&Event::Answer { tag: tag_of(&acts, "/v1/displaylab/set"), status: 200, body: lab_session() }, now + 50);
        let acts = shell.channel(&Event::Topic { topic: topic.into(), data }, now + 100);
        assert_eq!(lab_state(&shell)["open"], false, "{topic}");
        assert!(lab_calls(&acts).iter().any(|c| c.0 == "/v1/displaylab/end"), "{topic}: {acts:?}");
    }
}

#[test]
fn the_pointer_is_the_labs_while_it_is_open_a_click_on_an_answer_answers_it_and_nothing_under_it_hears_it() {
    let (_h, mut shell, now) = open_lab("home,displaylab");
    let acts = key(&mut shell, "ok", now);
    shell.helper(&Event::Answer { tag: tag_of(&acts, "/v1/displaylab/set"), status: 200, body: lab_session() }, now + 40);
    let g = shell.geometry()["displaylab"]["answers"][1]["frame"].clone();
    let (x, y) = ((g["x"].as_f64().unwrap() + 20.0) as f32, (g["y"].as_f64().unwrap() + 20.0) as f32);
    let mut all = Vec::new();
    for (i, buttons) in [0, 0, 1, 0].into_iter().enumerate() {
        all.extend(shell.key(&KeyEvent::Pointer { x, y, buttons, visible: true }, now + 100 + i as u64 * 50));
        shell.tick(now + 100 + i as u64 * 50);
    }
    assert_eq!(lab_state(&shell)["step"], "sharpness-soft", "the second answer, Can't tell, moved on");
    assert_eq!(lab_state(&shell)["answers"]["sharpness-strong"], "same");
    assert!(luna_calls(&all, "/launch").is_empty(), "Home under the lab heard nothing: {all:?}");
}

#[test]
fn without_the_kind_the_lab_is_neither_offered_nor_opened_and_the_keys_stay_with_home() {
    let (_h, mut shell) = home_shell("home,settings");
    assert!(!shell.report()["can"].as_array().unwrap().iter().any(|c| c == "displaylab"));
    let acts = shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "displaylab"}) }, 2_000);
    assert!(lab_calls(&acts).is_empty() && shell.state().get("displaylab").is_none());
    shell.settings.raise(ShellRequest::DisplayLab);
    let acts = shell.tick(2_100);
    assert!(lab_calls(&acts).is_empty());
    assert!(acts.iter().any(|a| matches!(a, Act::Log(l) if l.contains("display lab"))), "it says so: {acts:?}");
}

#[test]
fn the_lint_reads_the_lab_alone_and_finds_nothing_at_100_and_200_percent() {
    for scale in [1.0, 2.0] {
        let (h, kit) = testkit::kit(scale);
        let mut shell = Shell::new(kit, Kinds::parse("home,settings,displaylab"), 2000, &testkit::module_dir().join("icons/audio"), &json!({}), &json!({})).expect("a shell");
        shell.start();
        shell.helper(&Event::Up(true), 1_020);
        let acts = shell.channel(&Event::Topic { topic: "show".into(), data: json!({"kind": "displaylab"}) }, 2_000);
        shell.helper(&Event::Answer { tag: tag_of(&acts, "/v1/displaylab/begin"), status: 200, body: lab_session() }, 2_050);
        assert_eq!(shell.lint(), json!([]), "the intro at {scale}");
        key(&mut shell, "ok", 2_100);
        shell.tick(2_700);
        assert_eq!(shell.lint(), json!([]), "the first step at {scale}");
        drop(h);
    }
}
