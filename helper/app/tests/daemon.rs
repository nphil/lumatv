//! The real binary: start detached, status, a second start, stop, and the foreground run, as `tools/lumatv` will use them.

mod support;

use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use support::*;

const BIN: &str = env!("CARGO_BIN_EXE_lumatv-helper");

struct Run {
    code: i32,
    json: Value,
}

fn helper(data_dir: &Path, args: &[&str]) -> Run {
    let started = Instant::now();
    // `output` waits for the end of both pipes: a daemon that kept them open would hang this call, which is the point.
    let output = Command::new(BIN)
        .args(args)
        .arg("--data-dir")
        .arg(data_dir)
        .output()
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{args:?} took {:?}",
        started.elapsed()
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let json = serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("not one JSON object ({e}): {text:?}"));
    Run {
        code: output.status.code().unwrap_or(-1),
        json,
    }
}

/// A data dir with a config that touches nothing outside the loopback.
fn data_dir(extra: Value) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mut config = json!({
        "listen": {"port": 0},
        "screen": {"source": "on"},
        "fetch": {"openssl": "/nonexistent/openssl"},
        "nice": 0
    });
    if let Value::Object(extra) = extra {
        for (k, v) in extra {
            config[k] = v;
        }
    }
    std::fs::write(dir.path().join("helper.json"), config.to_string()).unwrap();
    receiver_off(&lumatv_helper::config::Paths::new(dir.path()));
    dir
}

fn health(port: u64) -> bool {
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let reply = raw(
        addr,
        format!("GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes(),
    );
    reply.status == 200
}

fn port_open(port: u64) -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(300),
    )
    .is_ok()
}

#[test]
fn start_status_start_again_stop() {
    let dir = data_dir(json!({}));
    let path = dir.path();

    let status = helper(path, &["status"]);
    assert_eq!(status.code, 3);
    assert_eq!(status.json, json!({"ok": true, "running": false}));

    let started = helper(path, &["start"]);
    assert_eq!(started.code, 0, "{}", started.json);
    assert_eq!(started.json["ok"], true);
    assert_eq!(started.json["started"], true);
    let pid = started.json["pid"].as_u64().unwrap();
    let port = started.json["port"].as_u64().unwrap();
    assert!(pid > 1 && port > 0);
    assert!(health(port), "the API does not answer on {port}");

    // The log is in the data dir under the name the plan gives it.
    let log = std::fs::read_to_string(path.join("log-helper")).unwrap();
    assert!(log.contains("listening on 127.0.0.1"), "{log}");

    let status = helper(path, &["status"]);
    assert_eq!(status.code, 0);
    assert_eq!(status.json["running"], true);
    assert_eq!(status.json["pid"], pid);
    assert_eq!(status.json["port"], port);
    assert_eq!(status.json["healthy"], true);

    // A second start finds the lock held and changes nothing.
    let again = helper(path, &["start"]);
    assert_eq!(again.code, 0);
    assert_eq!(again.json["already"], true);
    assert_eq!(again.json["pid"], pid);
    assert!(health(port));

    // The helper is a session of its own, detached from the starter (no controlling terminal).
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let after_name = stat.rsplit(')').next().unwrap();
    let fields: Vec<&str> = after_name.split_whitespace().collect();
    // fields: state, ppid, pgrp, session, tty_nr
    assert_eq!(fields[3], pid.to_string(), "not a session leader: {stat}");
    assert_eq!(fields[4], "0", "has a controlling terminal: {stat}");

    let stopped = helper(path, &["stop"]);
    assert_eq!(stopped.code, 0, "{}", stopped.json);
    assert_eq!(stopped.json["stopped"], true);
    assert_eq!(stopped.json["pid"], pid);
    assert!(!port_open(port), "the port is still open after stop");
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    let log = std::fs::read_to_string(path.join("log-helper")).unwrap();
    assert!(
        log.contains("signal 15: stopping") && log.trim_end().ends_with("stopped"),
        "{log}"
    );

    assert_eq!(helper(path, &["status"]).code, 3);
    let nothing = helper(path, &["stop"]);
    assert_eq!(nothing.code, 0);
    assert_eq!(nothing.json["running"], false);

    // It can be started again.
    let restarted = helper(path, &["start"]);
    assert_eq!(restarted.json["started"], true);
    assert_eq!(helper(path, &["stop"]).json["stopped"], true);
}

#[test]
fn start_reports_what_stopped_it() {
    // The port is taken: the starter says so and nothing is left running or locked.
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let dir = data_dir(json!({"listen": {"port": port}}));
    let started = helper(dir.path(), &["start"]);
    assert_eq!(started.code, 1);
    assert_eq!(started.json["ok"], false);
    let why = started.json["error"].as_str().unwrap();
    assert!(why.contains("could not start"), "{why}");
    assert_eq!(helper(dir.path(), &["status"]).code, 3);
    drop(taken);
    assert_eq!(helper(dir.path(), &["start"]).json["started"], true);
    assert_eq!(helper(dir.path(), &["stop"]).json["stopped"], true);
}

#[test]
fn a_bad_config_is_refused_before_anything_starts() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("helper.json"),
        r#"{"listen":{"host":"0.0.0.0"}}"#,
    )
    .unwrap();
    let r = helper(dir.path(), &["start"]);
    assert_eq!(r.code, 2);
    assert!(
        r.json["error"].as_str().unwrap().contains("loopback"),
        "{}",
        r.json
    );
    assert_eq!(helper(dir.path(), &["check-config"]).code, 2);
    assert!(
        !dir.path().join("helper.lock").exists(),
        "the lock was taken for a config that was refused"
    );

    let none = Command::new(BIN).arg("start").output().unwrap();
    assert_eq!(none.status.code(), Some(2));
    let text = String::from_utf8_lossy(&none.stdout);
    assert!(text.contains("--brand"), "{text}");
}

#[test]
fn a_brand_file_names_the_data_dir_and_one_outside_var_lib_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let brand = dir.path().join("brand.json");
    std::fs::write(&brand, r#"{"displayName":"LumaTV","dataDir":"/etc"}"#).unwrap();
    let output = Command::new(BIN)
        .args(["check-config", "--brand"])
        .arg(&brand)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).contains("dataDir"));
}

#[test]
fn check_config_reports_what_the_set_must_have() {
    let dir = data_dir(json!({}));
    let r = helper(dir.path(), &["check-config"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.json["ok"], true);
    assert_eq!(r.json["openssl"]["found"], false);
    assert_eq!(r.json["lunaSend"]["program"], "luna-send");
    assert!(r.json["listen"].as_str().unwrap().starts_with("127.0.0.1:"));
    assert!(r.json.get("caBundle").is_some());
}

#[test]
fn the_foreground_run_reports_ready_and_ends_on_sigterm() {
    let dir = data_dir(json!({}));
    let mut child = Command::new(BIN)
        .args(["run", "--data-dir"])
        .arg(dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let ready: Value =
        serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("{e}: {line:?}"));
    assert_eq!(ready["started"], true);
    let port = ready["port"].as_u64().unwrap();
    assert!(health(port));
    // A second foreground run finds the lock held and says so with the exit code the tool tests for.
    let second = helper(dir.path(), &["run"]);
    assert_eq!(second.code, 3);
    assert_eq!(second.json["already"], true);

    // A hangup (an SSH session closing) does not stop it; SIGTERM does.
    unsafe {
        libc::kill(i32::try_from(child.id()).unwrap(), libc::SIGHUP);
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(health(port), "a hangup stopped the helper");
    unsafe {
        libc::kill(i32::try_from(child.id()).unwrap(), libc::SIGTERM);
    }
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
    assert!(!port_open(port));
}

/// The service's pid and the supervisor's, from `status`, once the API answers again.
fn serving(path: &Path, not: Option<u64>) -> (u64, u64) {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let status = helper(path, &["status"]);
        if let (Some(pid), Some(worker), Some(port)) = (
            status.json["pid"].as_u64(),
            status.json["worker"].as_u64(),
            status.json["port"].as_u64(),
        ) && Some(worker) != not
            && status.json["healthy"] == true
            && health(port)
        {
            return (pid, worker);
        }
        assert!(
            Instant::now() < until,
            "the helper never served again: {status:?}",
            status = status.json
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn kill(pid: u64, signal: i32) {
    let status = Command::new("kill")
        .args([format!("-{signal}"), pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn a_service_that_ends_for_any_reason_is_started_again_and_only_stop_ends_it() {
    let dir = data_dir(json!({}));
    let path = dir.path();
    let started = helper(path, &["start"]);
    assert_eq!(started.json["started"], true, "{}", started.json);
    let (supervisor, first) = serving(path, None);
    assert_eq!(started.json["pid"].as_u64(), Some(supervisor));
    assert_ne!(
        supervisor, first,
        "the lock names the supervisor, the status the service"
    );

    // The kernel's out-of-memory killer.
    kill(first, 9);
    let (same, second) = serving(path, Some(first));
    assert_eq!(same, supervisor, "the supervisor stays");

    // A stray SIGTERM to the service alone (not a stop asked of the supervisor).
    kill(second, 15);
    let (same, third) = serving(path, Some(second));
    assert_eq!(same, supervisor);

    let log = std::fs::read_to_string(path.join("log-helper")).unwrap();
    assert!(
        log.contains("supervisor: the helper ended with signal 9")
            && log.contains("starting it again"),
        "{log}"
    );

    let stopped = helper(path, &["stop"]);
    assert_eq!(stopped.json["stopped"], true, "{}", stopped.json);
    assert!(!Path::new(&format!("/proc/{supervisor}")).exists());
    assert!(
        !Path::new(&format!("/proc/{third}")).exists(),
        "the service outlived the stop"
    );
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(
        helper(path, &["status"]).code,
        3,
        "a stopped helper stays stopped"
    );
}
