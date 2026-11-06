//! The preload policy is told to the preload manager at start with the screen on, and again at every switch-on, from `preload.json`.

mod support;

use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);

fn calls(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_listed_apps_are_told_at_start_and_at_every_switch_on_and_a_removed_file_means_nothing() {
    let mut rig = Rig::new();
    let dir = rig.dir.path().to_path_buf();
    let (state, log) = (dir.join("state"), dir.join("calls"));
    std::fs::write(&state, "Active").unwrap();
    let program = dir.join("luna-send");
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\ncase \"$3\" in\n  *getPowerState) printf '{{\"returnValue\":true,\"state\":\"%s\"}}\\n' \"$(cat {})\" ;;\n  *setPreloadPolicy) echo \"$4\" >> {} ;;\n  *) exit 1 ;;\nesac\n",
            state.display(),
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    settle(&program);
    std::fs::write(
        rig.paths.preload_file(),
        r#"{"off":["youtube.leanback.v4","com.webos.app.lgchannels","com.webos.app.browser"]}"#,
    )
    .unwrap();
    let config =
        test_config(json!({"screen": {"source": "luna", "pollMs": 40}, "luna": {"send": program}}));
    rig.start(config, Script::new());

    assert!(
        wait_until(WAIT, || calls(&log).len() == 3),
        "{:?}",
        calls(&log)
    );
    let first = calls(&log);
    assert_eq!(
        first[0],
        r#"{"application":{"id":"youtube.leanback.v4","isEnabled":false}}"#
    );
    assert_eq!(
        first[2],
        r#"{"application":{"id":"com.webos.app.browser","isEnabled":false}}"#
    );

    // Off and on again: told again.
    std::fs::write(&state, "Active Standby").unwrap();
    assert!(
        rig.wait_topic("screen", WAIT, |v| v["on"] == false)
            .is_some()
    );
    std::fs::write(&state, "Active").unwrap();
    assert!(
        wait_until(WAIT, || calls(&log).len() == 6),
        "{:?}",
        calls(&log)
    );

    // `lumatv preload off` deletes the file: the next switch-on tells nobody anything.
    std::fs::remove_file(rig.paths.preload_file()).unwrap();
    std::fs::write(&state, "Active Standby").unwrap();
    assert!(
        rig.wait_topic("screen", WAIT, |v| v["on"] == false)
            .is_some()
    );
    std::fs::write(&state, "Active").unwrap();
    assert!(
        rig.wait_topic("screen", WAIT, |v| v["on"] == true)
            .is_some()
    );
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(calls(&log).len(), 6);
}
