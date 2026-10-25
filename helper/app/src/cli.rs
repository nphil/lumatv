//! The command line: `run`, `start`, `stop`, `status`, `check-config`, `version`.
//!
//! `tools/lumatv` starts and stops the helper through this and nothing else. Every command prints exactly one JSON object on
//! stdout and exits 0 on success, 1 on failure, 2 on a usage or configuration error, and 3 from `status` when nothing runs:
//!
//! ```text
//! lumatv-helper start  --brand <app dir>/brand.json     detach, wait until the API listens
//! lumatv-helper stop   --brand <app dir>/brand.json     SIGTERM, wait for the lock, SIGKILL after 8 s
//! lumatv-helper status --brand <app dir>/brand.json
//! lumatv-helper run    --brand ... [--stderr]           foreground (development)
//! lumatv-helper check-config --brand ...
//! ```
//!
//! `--data-dir <dir>` replaces `--brand` for tests and development. The data dir from `brand.json` is checked by the tool's own
//! rule (a directory directly under `/var/lib`).

use crate::config::{Config, Paths, data_dir_from_brand, find_ca_bundle};
use crate::log::{self, Level, Logger};
use crate::service::{Service, StartOptions};
use crate::sys::{self, Forked};
use serde_json::{Value, json};
use std::fs::File;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long `start` waits for the helper to report that it listens.
const READY_WAIT: Duration = Duration::from_secs(15);
/// How long `stop` waits after SIGTERM before SIGKILL.
const TERM_WAIT: Duration = Duration::from_secs(8);

#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    command: String,
    brand: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    stderr: bool,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--brand" => out.brand = Some(it.next().ok_or("--brand needs a file")?.into()),
            "--data-dir" => out.data_dir = Some(it.next().ok_or("--data-dir needs a directory")?.into()),
            "--stderr" => out.stderr = true,
            "-h" | "--help" | "help" if out.command.is_empty() => out.command = "help".to_owned(),
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            word if out.command.is_empty() => out.command = word.to_owned(),
            word => return Err(format!("unexpected argument {word}")),
        }
    }
    if out.command.is_empty() {
        return Err("no command".to_owned());
    }
    Ok(out)
}

fn emit(value: &Value) {
    println!("{value}");
}

fn fail(code: i32, message: &str) -> i32 {
    emit(&json!({"ok": false, "error": message}));
    code
}

fn usage() -> Value {
    json!({
        "ok": true,
        "usage": "lumatv-helper <run|start|stop|status|check-config|version> (--brand <brand.json> | --data-dir <dir>) [--stderr]",
    })
}

/// The data dir and the config for this invocation.
fn resolve(args: &Args) -> Result<(Paths, Config), String> {
    let data_dir = match (&args.data_dir, &args.brand) {
        (Some(dir), _) => dir.clone(),
        (None, Some(brand)) => {
            let text = std::fs::read_to_string(brand).map_err(|e| format!("{}: {e}", brand.display()))?;
            data_dir_from_brand(&text).map_err(|e| e.to_string())?
        }
        (None, None) => return Err("give --brand <brand.json> or --data-dir <dir>".to_owned()),
    };
    let paths = Paths::new(data_dir);
    let config = Config::load(&paths.config_file()).map_err(|e| e.to_string())?;
    Ok((paths, config))
}

/// Runs the command line and returns the exit code.
pub fn main(args: Vec<String>) -> i32 {
    let args = match parse_args(&args) {
        Ok(args) => args,
        Err(why) => {
            emit(&json!({"ok": false, "error": why, "usage": usage()["usage"]}));
            return 2;
        }
    };
    match args.command.as_str() {
        "help" => {
            emit(&usage());
            0
        }
        "version" => {
            emit(&json!({"ok": true, "name": "lumatv-helper", "version": crate::VERSION, "api": crate::API_VERSION}));
            0
        }
        "check-config" => with_config(&args, check_config),
        "run" => with_config(&args, |paths, config| serve(paths, config, None, args.stderr)),
        "start" => with_config(&args, start),
        "stop" => with_config(&args, |paths, _| stop(&paths)),
        "status" => with_config(&args, status),
        other => fail(2, &format!("unknown command {other}")),
    }
}

fn with_config(args: &Args, run: impl FnOnce(Paths, Config) -> i32) -> i32 {
    match resolve(args) {
        Ok((paths, config)) => run(paths, config),
        Err(why) => fail(2, &why),
    }
}

/// Whether a program exists: a path that is a file, or a name found in `PATH`.
fn program_found(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

fn check_config(paths: Paths, config: Config) -> i32 {
    let bundle = find_ca_bundle(&config.fetch);
    emit(&json!({
        "ok": true,
        "dataDir": paths.data_dir.display().to_string(),
        "configFile": paths.config_file().display().to_string(),
        "configFileExists": paths.config_file().is_file(),
        "listen": format!("{}:{}", config.listen.host, config.listen.port),
        "screenSource": config.screen.source,
        "lunaSend": {"program": config.luna.send, "found": program_found(&config.luna.send)},
        "openssl": {"program": config.fetch.openssl, "found": program_found(&config.fetch.openssl)},
        "caBundle": bundle.as_ref().map(|p| p.display().to_string()),
        "fetchUser": config.fetch.user,
        "fetchUserFound": config.fetch.user.as_deref().map(|u| sys::lookup_user(u).is_some()),
        "root": sys::is_root(),
    }));
    0
}

/// The one line a detached child reports to its starter: `ok <pid> <port>`, `already <pid>` or `err <message>`.
enum Outcome {
    Listening { pid: u32, port: u16 },
    Already { pid: u32 },
    Failed(String),
}

fn report(ready: &mut Option<File>, outcome: &Outcome) {
    match ready {
        Some(pipe) => {
            let line = match outcome {
                Outcome::Listening { pid, port } => format!("ok {pid} {port}\n"),
                Outcome::Already { pid } => format!("already {pid}\n"),
                Outcome::Failed(why) => format!("err {}\n", why.replace('\n', " ")),
            };
            let _ = pipe.write_all(line.as_bytes());
        }
        None => emit(&outcome_json(outcome)),
    }
}

fn outcome_json(outcome: &Outcome) -> Value {
    match outcome {
        Outcome::Listening { pid, port } => json!({"ok": true, "started": true, "pid": pid, "port": port}),
        Outcome::Already { pid } => json!({"ok": true, "started": false, "already": true, "pid": pid}),
        Outcome::Failed(why) => json!({"ok": false, "error": why}),
    }
}

/// The helper itself: takes the lock, starts the service, reports, waits for a termination signal, stops. Runs in the foreground
/// process for `run` and in the detached child for `start`.
fn serve(paths: Paths, config: Config, mut ready: Option<File>, stderr_log: bool) -> i32 {
    // Before any thread exists, so every thread inherits the mask and `wait_termination` alone receives the signals.
    sys::block_termination_signals();
    let level = Level::parse(&config.log.level).unwrap_or(Level::Info);
    match Logger::open(&paths.log_file(), level, config.log.max_bytes, stderr_log) {
        Ok(logger) => log::init(logger),
        Err(e) => {
            report(&mut ready, &Outcome::Failed(format!("cannot open the log {}: {e}", paths.log_file().display())));
            return 1;
        }
    }
    let mut lock = match sys::try_lock(&paths.lock_file()) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            let pid = sys::read_info(&paths.lock_file()).map_or(0, |i| i.pid);
            report(&mut ready, &Outcome::Already { pid });
            return if ready.is_some() { 0 } else { 3 };
        }
        Err(e) => {
            report(&mut ready, &Outcome::Failed(format!("cannot take the lock {}: {e}", paths.lock_file().display())));
            return 1;
        }
    };
    let pid = sys::pid();
    let _ = sys::write_info(&mut lock, pid, None);
    if config.nice > 0 {
        sys::lower_priority(config.nice);
    }
    crate::log_info!("starting lumatv-helper {} (pid {pid}, data dir {})", crate::VERSION, paths.data_dir.display());
    let service = match Service::start(StartOptions {
        paths: paths.clone(),
        config,
        fetcher: None,
    }) {
        Ok(service) => service,
        Err(e) => {
            crate::log_error!("could not start: {e}");
            let _ = sys::write_info(&mut lock, 0, None);
            report(&mut ready, &Outcome::Failed(format!("could not start: {e}")));
            return 1;
        }
    };
    let port = service.addr().port();
    let _ = sys::write_info(&mut lock, pid, Some(port));
    report(&mut ready, &Outcome::Listening { pid, port });
    drop(ready);
    let signal = sys::wait_termination();
    crate::log_info!("signal {signal}: stopping");
    service.stop();
    // A pid file that outlives the process would only mislead a reader that does not check the lock.
    let _ = lock.set_len(0);
    crate::log_info!("stopped");
    0
}

fn start(paths: Paths, config: Config) -> i32 {
    // Said before the fork, so the common case needs no child.
    match sys::try_lock(&paths.lock_file()) {
        Ok(Some(free)) => drop(free),
        Ok(None) => {
            let pid = sys::read_info(&paths.lock_file()).map_or(0, |i| i.pid);
            emit(&outcome_json(&Outcome::Already { pid }));
            return 0;
        }
        Err(e) => return fail(1, &format!("cannot take the lock {}: {e}", paths.lock_file().display())),
    }
    match sys::fork_detached() {
        Err(e) => fail(1, &format!("cannot fork: {e}")),
        Ok(Forked::Child { ready }) => {
            let code = serve(paths, config, Some(ready), false);
            std::process::exit(code);
        }
        Ok(Forked::Parent { child, mut ready }) => {
            let line = sys::read_line_timeout(&mut ready, READY_WAIT);
            let outcome = match line.as_deref().map(|l| l.splitn(3, ' ').collect::<Vec<_>>()) {
                Some(parts) if parts.first() == Some(&"ok") => {
                    let pid = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(child);
                    let port = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
                    Outcome::Listening { pid, port }
                }
                Some(parts) if parts.first() == Some(&"already") => Outcome::Already {
                    pid: parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0),
                },
                Some(parts) if parts.first() == Some(&"err") => Outcome::Failed(parts[1..].join(" ")),
                _ => Outcome::Failed(format!("the helper (pid {child}) did not report ready within {READY_WAIT:?}")),
            };
            emit(&outcome_json(&outcome));
            i32::from(matches!(outcome, Outcome::Failed(_)))
        }
    }
}

fn stop(paths: &Paths) -> i32 {
    let lock_path = paths.lock_file();
    match sys::try_lock(&lock_path) {
        Ok(Some(_free)) => {
            emit(&json!({"ok": true, "running": false, "stopped": false}));
            return 0;
        }
        Ok(None) => {}
        Err(e) => return fail(1, &format!("cannot check the lock {}: {e}", lock_path.display())),
    }
    // The holder writes its pid right after taking the lock; give a helper that is just starting a moment to do so.
    let deadline = Instant::now() + Duration::from_secs(2);
    let pid = loop {
        if let Some(info) = sys::read_info(&lock_path) {
            break info.pid;
        }
        if Instant::now() >= deadline {
            return fail(1, "the lock is held but names no process");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if let Err(e) = sys::kill(pid, libc::SIGTERM) {
        return fail(1, &format!("cannot signal pid {pid}: {e}"));
    }
    let released = |wait: Duration| -> bool {
        let until = Instant::now() + wait;
        loop {
            if matches!(sys::try_lock(&lock_path), Ok(Some(_))) {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    if released(TERM_WAIT) {
        emit(&json!({"ok": true, "running": false, "stopped": true, "pid": pid}));
        return 0;
    }
    let _ = sys::kill(pid, libc::SIGKILL);
    if released(Duration::from_secs(2)) {
        emit(&json!({"ok": true, "running": false, "stopped": true, "killed": true, "pid": pid}));
        0
    } else {
        fail(1, &format!("pid {pid} did not exit"))
    }
}

/// A one-shot `GET /v1/health`: whether the API answers.
fn healthy(ip: IpAddr, port: u16) -> bool {
    let addr = SocketAddr::new(ip, port);
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let host = match ip {
        IpAddr::V4(v4) => format!("{v4}:{port}"),
        IpAddr::V6(v6) => format!("[{v6}]:{port}"),
    };
    if stream
        .write_all(format!("GET /v1/health HTTP/1.0\r\nHost: {host}\r\n\r\n").as_bytes())
        .is_err()
    {
        return false;
    }
    let mut reply = Vec::new();
    let _ = stream.take(8192).read_to_end(&mut reply);
    let text = String::from_utf8_lossy(&reply);
    text.starts_with("HTTP/1.1 200") && text.contains("\"ok\":true")
}

fn status(paths: Paths, config: Config) -> i32 {
    match sys::try_lock(&paths.lock_file()) {
        Ok(Some(_free)) => {
            emit(&json!({"ok": true, "running": false}));
            3
        }
        Ok(None) => {
            let info = sys::read_info(&paths.lock_file());
            let port = info.and_then(|i| i.port).or((config.listen.port != 0).then_some(config.listen.port));
            let ip = config.listen.host.parse::<IpAddr>().ok();
            let healthy = port.zip(ip).map(|(port, ip)| healthy(ip, port));
            emit(&json!({"ok": true, "running": true, "pid": info.map(|i| i.pid), "port": port, "healthy": healthy}));
            0
        }
        Err(e) => fail(1, &format!("cannot check the lock {}: {e}", paths.lock_file().display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_command_and_options_are_parsed() {
        let a = parse_args(&args(&["start", "--brand", "/app/brand.json"])).unwrap();
        assert_eq!(a.command, "start");
        assert_eq!(a.brand, Some(PathBuf::from("/app/brand.json")));
        let a = parse_args(&args(&["--data-dir", "/tmp/x", "run", "--stderr"])).unwrap();
        assert_eq!((a.command.as_str(), a.stderr), ("run", true));
        assert_eq!(a.data_dir, Some(PathBuf::from("/tmp/x")));
        assert_eq!(parse_args(&args(&["--help"])).unwrap().command, "help");
    }

    #[test]
    fn bad_command_lines_are_refused() {
        assert!(parse_args(&args(&[])).is_err());
        assert!(parse_args(&args(&["start", "--brand"])).is_err());
        assert!(parse_args(&args(&["start", "--fast"])).is_err());
        assert!(parse_args(&args(&["start", "stop"])).is_err());
    }

    #[test]
    fn a_data_dir_or_a_brand_is_required_to_resolve() {
        let a = parse_args(&args(&["status"])).unwrap();
        assert!(resolve(&a).unwrap_err().contains("--brand"));
        let dir = tempfile::tempdir().unwrap();
        let a = parse_args(&args(&["status", "--data-dir", dir.path().to_str().unwrap()])).unwrap();
        let (paths, config) = resolve(&a).unwrap();
        assert_eq!(paths.data_dir, dir.path());
        assert_eq!(config.listen.port, crate::config::DEFAULT_PORT);
    }

    #[test]
    fn a_brand_file_names_the_data_dir_by_the_tools_rule() {
        let dir = tempfile::tempdir().unwrap();
        let brand = dir.path().join("brand.json");
        std::fs::write(&brand, r#"{"dataDir":"/var/lib/lumatv"}"#).unwrap();
        let a = parse_args(&args(&["status", "--brand", brand.to_str().unwrap()])).unwrap();
        // `/var/lib/lumatv/helper.json` may or may not exist where the tests run; the data dir is what matters.
        if let Ok((paths, _)) = resolve(&a) {
            assert_eq!(paths.data_dir, PathBuf::from("/var/lib/lumatv"));
        }
        std::fs::write(&brand, r#"{"dataDir":"/etc"}"#).unwrap();
        assert!(resolve(&a).is_err());
    }

    #[test]
    fn programs_are_found_by_path_or_by_name() {
        assert!(program_found("/bin/sh"));
        assert!(program_found("sh"));
        assert!(!program_found("/nonexistent/program"));
        assert!(!program_found("no-such-program-anywhere"));
    }

    #[test]
    fn outcomes_have_the_documented_json() {
        assert_eq!(
            outcome_json(&Outcome::Listening { pid: 5, port: 18731 }),
            json!({"ok": true, "started": true, "pid": 5, "port": 18731})
        );
        assert_eq!(
            outcome_json(&Outcome::Already { pid: 5 }),
            json!({"ok": true, "started": false, "already": true, "pid": 5})
        );
        assert_eq!(outcome_json(&Outcome::Failed("x".into())), json!({"ok": false, "error": "x"}));
    }
}
