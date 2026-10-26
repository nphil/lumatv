//! The command line: `run`, `start`, `stop`, `status`, `check-config`, `voice-link`, `voice-unlink`, `version`.
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
//! lumatv-helper voice-link --brand ... --url <Home Assistant>   the access token is read from standard input
//! lumatv-helper voice-unlink --brand ...
//! lumatv-helper hue-link --brand ... --host <bridge or Bifrost> (--pair | keys on standard input) [--area <id>] [--fps <n>]
//! lumatv-helper hue-area --brand ... [--area <id>]         list the bridge's entertainment areas, or choose one
//! lumatv-helper hue-unlink --brand ...
//! lumatv-helper z2m-link --brand ... --host <MQTT broker> [--port <n>] [--user <name>] [--base-topic <t>] [--fps <n>] --light <name>=<place> ...
//!                                                            the password on standard input when --user is given
//! lumatv-helper z2m-unlink --brand ...
//! lumatv-helper ble-link --brand ... [--port <n>] [--name <node>] [--scan-seconds <5-30>] [--rest-seconds <window-600>] [--standby] [--rotate-key]
//!                                                            the Bluetooth proxy for Home Assistant: prints the key it asks for
//! lumatv-helper ble-unlink --brand ...
//! ```
//!
//! `voice-link` is how voice search is pointed at Home Assistant: the address is an argument and the token comes on standard input,
//! so it is in no process list and no shell history that the owner does not choose, and neither is ever printed. It is the only
//! way to set either (see `voice_store`); a helper that is running is told to read the link again. `hue-link` does the same for
//! the Hue bridge (see `hue_store`): the keys never appear in an argument or in any output. `ha-link` does the same for the Home
//! Assistant the plugins' lease dials (see `ha_store`, `plugins::ha`), and `plugin-consent`, `plugin-enable`, `plugin-disable` and
//! `plugin-status` are how the owner switches a plugin on and off (see `plugins::install`). `z2m-link` does the same for the MQTT
//! broker Zigbee2MQTT uses (see `ambient_z2m_store`): the password comes on standard input and is never printed.
//!
//! `--data-dir <dir>` replaces `--brand` for tests and development. The data dir from `brand.json` is checked by the tool's own
//! rule (a directory directly under `/var/lib`).

use crate::ambient_z2m;
use crate::ambient_z2m_store::{self, Z2mLight, Z2mLink};
use crate::ble_cli;
use crate::config::{Config, Paths, data_dir_from_brand, find_ca_bundle};
use crate::hue_rest::{self, Area, Bridge, HueError, Tls};
use crate::hue_store::{self, HueLink};
use crate::log::{self, Level, Logger};
use crate::plugins::install;
use crate::service::{Service, StartOptions};
use crate::sys::{self, Forked};
use serde_json::{Value, json};
use std::fs::File;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
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
    url: Option<String>,
    stderr: bool,
    host: Option<String>,
    port: Option<u16>,
    stream_port: Option<u16>,
    area: Option<String>,
    pair: bool,
    fps: Option<u8>,
    user: Option<String>,
    base_topic: Option<String>,
    lights: Vec<String>,
    ble: ble_cli::BleArgs,
    id: Option<String>,
    approve: Option<String>,
    from: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--brand" => out.brand = Some(it.next().ok_or("--brand needs a file")?.into()),
            "--data-dir" => {
                out.data_dir = Some(it.next().ok_or("--data-dir needs a directory")?.into())
            }
            "--url" => out.url = Some(it.next().ok_or("--url needs an address")?.clone()),
            "--host" => out.host = Some(it.next().ok_or("--host needs an address")?.clone()),
            "--area" => out.area = Some(it.next().ok_or("--area needs an id")?.clone()),
            "--port" => out.port = Some(port_arg(it.next(), "--port")?),
            "--stream-port" => out.stream_port = Some(port_arg(it.next(), "--stream-port")?),
            "--fps" => {
                out.fps = Some(
                    it.next()
                        .and_then(|v| v.parse::<u8>().ok())
                        .filter(|v| (1..=hue_store::MAX_FPS).contains(v))
                        .ok_or("--fps needs a rate from 1 to 60")?,
                );
            }
            "--pair" => out.pair = true,
            "--user" => out.user = Some(it.next().ok_or("--user needs a name")?.clone()),
            "--base-topic" => {
                out.base_topic = Some(it.next().ok_or("--base-topic needs a topic")?.clone())
            }
            "--light" => out
                .lights
                .push(it.next().ok_or("--light needs <name>=<place>")?.clone()),
            "--name" | "--friendly-name" | "--scan-seconds" | "--rest-seconds" | "--standby"
            | "--no-standby" | "--always" | "--no-always" | "--rotate-key" => {
                ble_cli::flag(&mut out.ble, arg, &mut it)?
            }
            "--id" => out.id = Some(it.next().ok_or("--id needs a plugin id")?.clone()),
            "--approve" => {
                out.approve = Some(
                    it.next()
                        .ok_or("--approve needs the approval code")?
                        .clone(),
                )
            }
            "--from" => out.from = Some(it.next().ok_or("--from needs a directory")?.into()),
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

fn port_arg(value: Option<&String>, flag: &str) -> Result<u16, String> {
    value
        .ok_or_else(|| format!("{flag} needs a port"))?
        .parse::<u16>()
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| format!("{flag} needs a port from 1 to 65535"))
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
        "usage": "lumatv-helper <run|start|stop|status|check-config|voice-link|voice-unlink|ha-link|ha-unlink|plugin-consent|plugin-enable|plugin-disable|plugin-status|hue-link|hue-area|hue-unlink|z2m-link|z2m-unlink|ble-link|ble-unlink|version> (--brand <brand.json> | --data-dir <dir>) [--stderr] [--url <Home Assistant address>] [--id <plugin> [--approve <code>] [--from <package dir>]] [--host <Hue bridge, Bifrost or MQTT broker> [--port <n>] [--stream-port <n>] [--fps <n>] [--pair] [--area <id>] [--user <name>] [--base-topic <topic>] [--light <name>=<place>]...] [ble-link: [--port <n>] [--name <node>] [--friendly-name <text>] [--scan-seconds <5-30>] [--rest-seconds <window-600>] [--standby|--no-standby] [--always|--no-always] [--rotate-key]]",
    })
}

/// The data dir and the config for this invocation.
fn resolve(args: &Args) -> Result<(Paths, Config), String> {
    let mut brand_app_id = None;
    let mut brand_qml_module = None;
    let data_dir = match (&args.data_dir, &args.brand) {
        (Some(dir), _) => dir.clone(),
        (None, Some(brand)) => {
            let text =
                std::fs::read_to_string(brand).map_err(|e| format!("{}: {e}", brand.display()))?;
            brand_app_id = crate::config::app_id_from_brand(&text);
            brand_qml_module = crate::config::qml_module_from_brand(&text);
            data_dir_from_brand(&text).map_err(|e| e.to_string())?
        }
        (None, None) => return Err("give --brand <brand.json> or --data-dir <dir>".to_owned()),
    };
    let paths = Paths::new(data_dir);
    let mut config = Config::load(&paths.config_file()).map_err(|e| e.to_string())?;
    config.brand_app_id = brand_app_id;
    config.brand_qml_module = brand_qml_module;
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
            emit(
                &json!({"ok": true, "name": "lumatv-helper", "version": crate::VERSION, "api": crate::API_VERSION}),
            );
            0
        }
        "check-config" => with_config(&args, check_config),
        "run" => with_config(&args, |paths, config| {
            serve(paths, config, None, args.stderr)
        }),
        "start" => with_config(&args, start),
        "stop" => with_config(&args, |paths, _| stop(&paths)),
        "status" => with_config(&args, status),
        "voice-link" => with_config(&args, |paths, config| {
            voice_link(&paths, &config, args.url.as_deref())
        }),
        "voice-unlink" => with_config(&args, voice_unlink),
        "ha-link" => with_config(&args, |paths, config| {
            let mut token = String::new();
            // A token is about 180 characters: a longer input is not one, and is never read whole.
            if let Err(e) = std::io::stdin().take(4096).read_to_string(&mut token) {
                return fail(
                    1,
                    &format!("could not read the token from standard input: {e}"),
                );
            }
            finish(install::run_ha_link(
                &paths,
                &config,
                args.url.as_deref(),
                &token,
            ))
        }),
        "ha-unlink" => with_config(&args, |paths, config| {
            finish(install::run_ha_unlink(&paths, &config))
        }),
        "plugin-consent" => with_config(&args, |_, _| match package_dir(&args) {
            Ok(from) => finish(install::run_consent(&from)),
            Err(why) => fail(2, &why),
        }),
        "plugin-enable" => with_config(&args, |paths, config| {
            match (package_dir(&args), args.approve.as_deref()) {
                (Ok(from), Some(approve)) => {
                    finish(install::run_enable(&paths, &config, &from, approve))
                }
                (Ok(_), None) => fail(
                    2,
                    "plugin-enable needs --approve <code>: run plugin-consent first and read what the plugin may do",
                ),
                (Err(why), _) => fail(2, &why),
            }
        }),
        "plugin-disable" => with_config(&args, |paths, config| match args.id.as_deref() {
            Some(id) => finish(install::run_disable(&paths, &config, id)),
            None => fail(2, "plugin-disable needs --id <plugin id>"),
        }),
        "plugin-status" => with_config(&args, |paths, config| {
            finish(install::run_status(&paths, &config))
        }),
        "hue-link" => with_config(&args, |paths, config| hue_link(&paths, &config, &args)),
        "hue-area" => with_config(&args, |paths, config| hue_area(&paths, &config, &args)),
        "hue-unlink" => with_config(&args, hue_unlink),
        "z2m-link" => with_config(&args, |paths, config| z2m_link(&paths, &config, &args)),
        "z2m-unlink" => with_config(&args, z2m_unlink),
        "ble-link" => with_config(&args, |paths, _| {
            ble_cli::link(&paths, &args.ble, args.port)
        }),
        "ble-unlink" => with_config(&args, |paths, config| ble_cli::unlink(&paths, &config)),
        other => fail(2, &format!("unknown command {other}")),
    }
}

/// Prints a command's one JSON object and gives its exit code.
fn finish((code, doc): install::Outcome) -> i32 {
    emit(&doc);
    code
}

/// The plugin package a command is about: `--from`, else the one the ipk carries (`<app dir>/payload/plugins/<id>`).
fn package_dir(args: &Args) -> Result<PathBuf, String> {
    if let Some(from) = &args.from {
        return Ok(from.clone());
    }
    let id = args
        .id
        .as_deref()
        .ok_or("give --id <plugin id> (or --from <package directory>)")?;
    let brand = args.brand.as_ref().ok_or(
        "give --brand <brand.json> (the package is in the app) or --from <package directory>",
    )?;
    Ok(install::packaged(brand, id))
}

fn with_config(args: &Args, run: impl FnOnce(Paths, Config) -> i32) -> i32 {
    match resolve(args) {
        Ok((paths, config)) => run(paths, config),
        Err(why) => fail(2, &why),
    }
}

/// Whether a program exists (see [`sys::which`]).
fn program_found(program: &str) -> bool {
    sys::which(program).is_some()
}

fn check_config(paths: Paths, config: Config) -> i32 {
    let bundle = find_ca_bundle(&config.fetch);
    let chosen = crate::fetcher::choose(&config.fetch);
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
        "curl": {"program": config.fetch.curl, "found": program_found(&config.fetch.curl)},
        "fetchBackend": chosen.backend,
        "fetchBackendWhy": chosen.detail,
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
        Outcome::Listening { pid, port } => {
            json!({"ok": true, "started": true, "pid": pid, "port": port})
        }
        Outcome::Already { pid } => {
            json!({"ok": true, "started": false, "already": true, "pid": pid})
        }
        Outcome::Failed(why) => json!({"ok": false, "error": why}),
    }
}

/// The helper itself: takes the lock, starts the service, reports, waits for a termination signal, stops. Runs in the foreground
/// process for `run` and in the detached child for `start`.
fn serve(paths: Paths, config: Config, mut ready: Option<File>, stderr_log: bool) -> i32 {
    // The brand's data dir is directly under /var/lib (checked at load); the tool creates it, and a fresh one is made here.
    if let Err(e) = std::fs::create_dir_all(&paths.data_dir) {
        report(
            &mut ready,
            &Outcome::Failed(format!("cannot create {}: {e}", paths.data_dir.display())),
        );
        return 1;
    }
    // Before any thread exists, so every thread inherits the mask and `wait_termination` alone receives the signals.
    sys::block_termination_signals();
    let level = Level::parse(&config.log.level).unwrap_or(Level::Info);
    match Logger::open(&paths.log_file(), level, config.log.max_bytes, stderr_log) {
        Ok(logger) => log::init(logger),
        Err(e) => {
            report(
                &mut ready,
                &Outcome::Failed(format!(
                    "cannot open the log {}: {e}",
                    paths.log_file().display()
                )),
            );
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
            report(
                &mut ready,
                &Outcome::Failed(format!(
                    "cannot take the lock {}: {e}",
                    paths.lock_file().display()
                )),
            );
            return 1;
        }
    };
    let pid = sys::pid();
    let _ = sys::write_info(&mut lock, pid, None);
    if config.nice > 0 {
        sys::lower_priority(config.nice);
    }
    crate::log_info!(
        "starting lumatv-helper {} (pid {pid}, data dir {})",
        crate::VERSION,
        paths.data_dir.display()
    );
    let service = match Service::start(StartOptions {
        paths: paths.clone(),
        config,
        fetcher: None,
        ai: None,
        voice_policy: None,
    }) {
        Ok(service) => service,
        Err(e) => {
            crate::log_error!("could not start: {e}");
            let _ = sys::write_info(&mut lock, 0, None);
            report(
                &mut ready,
                &Outcome::Failed(format!("could not start: {e}")),
            );
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
    if let Err(e) = std::fs::create_dir_all(&paths.data_dir) {
        return fail(
            1,
            &format!("cannot create {}: {e}", paths.data_dir.display()),
        );
    }
    // Said before the fork, so the common case needs no child.
    match sys::try_lock(&paths.lock_file()) {
        Ok(Some(free)) => drop(free),
        Ok(None) => {
            let pid = sys::read_info(&paths.lock_file()).map_or(0, |i| i.pid);
            emit(&outcome_json(&Outcome::Already { pid }));
            return 0;
        }
        Err(e) => {
            return fail(
                1,
                &format!("cannot take the lock {}: {e}", paths.lock_file().display()),
            );
        }
    }
    match sys::fork_detached() {
        Err(e) => fail(1, &format!("cannot fork: {e}")),
        Ok(Forked::Child { ready }) => {
            let code = serve(paths, config, Some(ready), false);
            std::process::exit(code);
        }
        Ok(Forked::Parent { child, mut ready }) => {
            let line = sys::read_line_timeout(&mut ready, READY_WAIT);
            let outcome = match line
                .as_deref()
                .map(|l| l.splitn(3, ' ').collect::<Vec<_>>())
            {
                Some(parts) if parts.first() == Some(&"ok") => {
                    let pid = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(child);
                    let port = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
                    Outcome::Listening { pid, port }
                }
                Some(parts) if parts.first() == Some(&"already") => Outcome::Already {
                    pid: parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0),
                },
                Some(parts) if parts.first() == Some(&"err") => {
                    Outcome::Failed(parts[1..].join(" "))
                }
                _ => Outcome::Failed(format!(
                    "the helper (pid {child}) did not report ready within {READY_WAIT:?}"
                )),
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
        Err(e) => {
            return fail(
                1,
                &format!("cannot check the lock {}: {e}", lock_path.display()),
            );
        }
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

/// Saves the link voice search speaks to Home Assistant through: the address from `--url`, the token from standard input.
fn voice_link(paths: &Paths, config: &Config, url: Option<&str>) -> i32 {
    let Some(url) = url else {
        return fail(2, "voice-link needs --url <the address of Home Assistant>");
    };
    // A token is about 180 characters: a longer input is not one, and is never read whole.
    let mut token = String::new();
    if let Err(e) = std::io::stdin().take(4096).read_to_string(&mut token) {
        return fail(
            1,
            &format!("could not read the token from standard input: {e}"),
        );
    }
    match crate::voice_store::save(&paths.data_dir, url, &token) {
        Ok(link) => {
            let told = tell_helper(paths, config, "/v1/voice/refresh");
            emit(&json!({
                "ok": true,
                "linked": true,
                "url": link.endpoint.display(),
                "helperTold": told,
                "next": "voice search also needs the base.voice setting switched on and voice.capture in helper.json",
            }));
            0
        }
        Err(e) => fail(2, &e.to_string()),
    }
}

/// Removes the saved link.
fn voice_unlink(paths: Paths, config: Config) -> i32 {
    let removed = crate::voice_store::clear(&paths.data_dir);
    let told = tell_helper(&paths, &config, "/v1/voice/refresh");
    emit(&json!({"ok": true, "linked": false, "removed": removed, "helperTold": told}));
    0
}

/// Asks a running helper to read a link again (`POST <path>`). False when none runs or it did not answer.
fn tell_helper(paths: &Paths, config: &Config, path: &str) -> bool {
    // The lock is held only by a running helper.
    if !matches!(sys::try_lock(&paths.lock_file()), Ok(None)) {
        return false;
    }
    let port = sys::read_info(&paths.lock_file())
        .and_then(|i| i.port)
        .unwrap_or(config.listen.port);
    let Ok(ip) = config.listen.host.parse::<IpAddr>() else {
        return false;
    };
    let Ok(mut stream) =
        TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_secs(1))
    else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let request = format!(
        "POST {path} HTTP/1.0\r\nX-Luma-Client: cli\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut reply = Vec::new();
    let _ = stream.take(8192).read_to_end(&mut reply);
    String::from_utf8_lossy(&reply).starts_with("HTTP/1.1 200")
}

// ------------------------------------------------------------------------------------------------- the Hue bridge

/// How long `hue-link --pair` waits for the bridge's link button.
const PAIR_WAIT: Duration = Duration::from_secs(30);

fn area_json(area: &Area) -> Value {
    json!({
        "id": area.id,
        "name": area.name,
        "type": area.kind,
        "channels": area.channels.len(),
        "streaming": area.active,
    })
}

fn area_list(areas: &[Area]) -> String {
    if areas.is_empty() {
        return "the bridge has no entertainment area: make one in the Hue app".to_owned();
    }
    areas
        .iter()
        .map(|a| format!("{} ({})", a.id, a.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The two keys from the bridge's own answer to the registration, pasted on standard input (the whole answer, or just its
/// `success` part). Never echoed, whatever is wrong with them.
fn read_hue_keys() -> Result<(String, String), String> {
    let mut text = String::new();
    std::io::stdin()
        .take(4096)
        .read_to_string(&mut text)
        .map_err(|e| format!("could not read the keys from standard input: {e}"))?;
    let value: Value = serde_json::from_str(&text).map_err(|_| {
        "standard input is not the bridge's answer to the registration (give --pair to make the keys here)".to_owned()
    })?;
    let node = value.get(0).unwrap_or(&value);
    let node = node.get("success").unwrap_or(node);
    let take = |key: &str| node.get(key).and_then(Value::as_str).map(str::to_owned);
    match (take("username"), take("clientkey")) {
        (Some(app), Some(client)) => Ok((app, client)),
        _ => Err("the answer on standard input has no username and clientkey".to_owned()),
    }
}

/// Presses nothing: waits for a person to press the link button, up to [`PAIR_WAIT`], and takes the keys the bridge makes.
fn hue_pair(bridge: &Bridge) -> Result<(String, String), (i32, String)> {
    let until = Instant::now() + PAIR_WAIT;
    loop {
        match bridge.register() {
            Ok((app, client)) => return Ok((app.expose().to_owned(), client.expose().to_owned())),
            Err(HueError::LinkButton) if Instant::now() < until => {
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(HueError::LinkButton) => {
                return Err((
                    1,
                    "the link button was not pressed within 30 seconds: press it, then run this again"
                        .to_owned(),
                ));
            }
            Err(e) => return Err((1, e.to_string())),
        }
    }
}

/// The link the arguments and the bridge make, and every area the bridge has.
fn hue_make_link(
    bridge: &Bridge,
    args: &Args,
    ip: Ipv4Addr,
    port: u16,
    stream_port: u16,
) -> Result<(HueLink, Vec<Area>), (i32, String)> {
    let (app, client) = if args.pair {
        hue_pair(bridge)?
    } else {
        read_hue_keys().map_err(|e| (2, e))?
    };
    let link = HueLink::new(&ip.to_string(), port, stream_port, None, &app, &client)
        .and_then(|link| link.with_fps(args.fps.unwrap_or(hue_store::DEFAULT_FPS)))
        .map_err(|e| (2, e.to_string()))?;
    // The key is proved by asking something with it before it is kept.
    bridge
        .application_id(&link.app_key)
        .map_err(|e| (1, e.to_string()))?;
    let areas = bridge
        .areas(&link.app_key)
        .map_err(|e| (1, e.to_string()))?;
    let chosen = match args.area.as_deref() {
        Some(wanted) => {
            let wanted = wanted.trim().to_ascii_lowercase();
            if !areas.iter().any(|a| a.id == wanted) {
                return Err((
                    2,
                    format!("the bridge has no such area; it has {}", area_list(&areas)),
                ));
            }
            Some(wanted)
        }
        None if areas.len() == 1 => Some(areas[0].id.clone()),
        None => None,
    };
    let link = match chosen {
        Some(id) => link.with_area(&id).map_err(|e| (2, e.to_string()))?,
        None => link,
    };
    Ok((link, areas))
}

/// Links the Hue bridge: finds out which certificate it presents and pins it, gets the two keys (`--pair`: made here after the link
/// button; otherwise the bridge's answer to the registration on standard input), checks the key by using it, and saves it all with
/// the entertainment area (`--area`, or the only one there is).
fn hue_link(paths: &Paths, config: &Config, args: &Args) -> i32 {
    let Some(host) = args.host.as_deref() else {
        return fail(2, "hue-link needs --host <the bridge's address>");
    };
    let Ok(ip) = host.trim().parse::<Ipv4Addr>() else {
        return fail(2, "the bridge's address must be an IPv4 address");
    };
    if !sys::is_lan_address(ip) {
        return fail(
            2,
            "the bridge's address must be a private (or loopback) IPv4 address",
        );
    }
    let port = args.port.unwrap_or(hue_store::DEFAULT_PORT);
    let stream_port = args.stream_port.unwrap_or(hue_store::DEFAULT_STREAM_PORT);
    let addr = SocketAddr::from((ip, port));
    let tls = Tls::from_config(&config.fetch);
    // The certificate the bridge shows now is the one every later connection must show: this is the moment to be at the bridge.
    let pem = match hue_rest::scan_certificate(&tls, addr) {
        Ok(pem) => pem,
        Err(e) => return fail(1, &e.to_string()),
    };
    let staged = match hue_store::stage_certificate(&paths.data_dir, &pem) {
        Ok(path) => path,
        Err(e) => return fail(1, &e.to_string()),
    };
    let bridge = Bridge::new(tls, addr, staged);
    let made = hue_make_link(&bridge, args, ip, port, stream_port);
    let saved = made.and_then(|(link, areas)| {
        hue_store::settle_certificate(&paths.data_dir, true).map_err(|e| (1, e.to_string()))?;
        hue_store::save(&paths.data_dir, &link).map_err(|e| (1, e.to_string()))?;
        Ok((link, areas))
    });
    match saved {
        Ok((link, areas)) => {
            let told = tell_helper(paths, config, "/v1/ambient/refresh");
            let chosen = link
                .area
                .as_deref()
                .and_then(|id| areas.iter().find(|a| a.id == id));
            emit(&json!({
                "ok": true,
                "linked": true,
                "bridge": format!("{}:{}", link.host, link.port),
                "area": chosen.map(area_json),
                "areas": areas.iter().map(area_json).collect::<Vec<_>>(),
                "helperTold": told,
                "next": if chosen.is_some() {
                    "switch ambient light on in Settings, Ambient light"
                } else {
                    "choose the entertainment area: lumatv-helper hue-area --area <id>"
                },
            }));
            0
        }
        Err((code, why)) => {
            let _ = hue_store::settle_certificate(&paths.data_dir, false);
            fail(code, &why)
        }
    }
}

/// Lists the bridge's entertainment areas, and with `--area` chooses the one ambient light streams to.
fn hue_area(paths: &Paths, config: &Config, args: &Args) -> i32 {
    let link = match hue_store::load(&paths.data_dir) {
        Ok(Some(link)) => link,
        Ok(None) => return fail(2, "no bridge is linked: run lumatv-helper hue-link"),
        Err(e) => return fail(1, &e.to_string()),
    };
    let bridge = Bridge::new(
        Tls::from_config(&config.fetch),
        SocketAddr::from((link.host, link.port)),
        hue_store::certificate_file(&paths.data_dir),
    );
    let areas = match bridge.areas(&link.app_key) {
        Ok(areas) => areas,
        Err(e) => return fail(1, &e.to_string()),
    };
    let Some(wanted) = args.area.as_deref() else {
        emit(&json!({
            "ok": true,
            "areas": areas.iter().map(area_json).collect::<Vec<_>>(),
            "chosen": link.area,
        }));
        return 0;
    };
    let wanted = wanted.trim().to_ascii_lowercase();
    let Some(area) = areas.iter().find(|a| a.id == wanted) else {
        return fail(
            2,
            &format!("the bridge has no such area; it has {}", area_list(&areas)),
        );
    };
    let saved = link
        .with_area(&area.id)
        .and_then(|link| hue_store::save(&paths.data_dir, &link));
    match saved {
        Ok(()) => {
            let told = tell_helper(paths, config, "/v1/ambient/refresh");
            emit(&json!({"ok": true, "area": area_json(area), "helperTold": told}));
            0
        }
        Err(e) => fail(1, &e.to_string()),
    }
}

/// Removes the saved bridge, its keys and its certificate.
fn hue_unlink(paths: Paths, config: Config) -> i32 {
    let removed = hue_store::clear(&paths.data_dir);
    let told = tell_helper(&paths, &config, "/v1/ambient/refresh");
    emit(&json!({"ok": true, "linked": false, "removed": removed, "helperTold": told}));
    0
}

// ------------------------------------------------------------------------------------------------- Zigbee2MQTT

fn found_json(f: &ambient_z2m::Found, light: Option<&Z2mLight>) -> Value {
    json!({
        "name": f.name,
        "networkAddress": format!("0x{:04x}", f.network_address),
        "model": f.model,
        "segments": f.segments,
        "x": light.map(|l| l.x),
        "y": light.map(|l| l.y),
    })
}

/// Links Zigbee2MQTT's MQTT broker: the lights named by `--light` are looked up in Zigbee2MQTT's device list (nothing is sent to
/// any light), then the link is saved with the password that came on standard input.
fn z2m_link(paths: &Paths, config: &Config, args: &Args) -> i32 {
    let Some(host) = args.host.as_deref() else {
        return fail(2, "z2m-link needs --host <the MQTT broker's address>");
    };
    if args.lights.is_empty() {
        return fail(
            2,
            &format!(
                "z2m-link needs at least one --light <Zigbee2MQTT name>=<place>, the place one of {} or x,y (a strip: <place>:<segments>)",
                ambient_z2m_store::PLACES.map(|p| p.0).join(", ")
            ),
        );
    }
    let mut lights = Vec::new();
    for spec in &args.lights {
        match Z2mLight::parse(spec) {
            Ok(l) => lights.push(l),
            Err(e) => return fail(2, &e.to_string()),
        }
    }
    // The password: on standard input, only when there is a user; a line's end is not part of it.
    let mut password = String::new();
    if args.user.is_some()
        && let Err(e) = std::io::stdin().take(4096).read_to_string(&mut password)
    {
        return fail(
            1,
            &format!("could not read the password from standard input: {e}"),
        );
    }
    let password = password.trim_end_matches(['\r', '\n']);
    let link = match Z2mLink::new(
        host,
        args.port.unwrap_or(ambient_z2m_store::DEFAULT_PORT),
        args.user.as_deref(),
        (!password.is_empty()).then_some(password),
        args.base_topic
            .as_deref()
            .unwrap_or(ambient_z2m_store::DEFAULT_BASE_TOPIC),
        args.fps.unwrap_or(ambient_z2m_store::DEFAULT_FPS),
        lights,
    ) {
        Ok(link) => link,
        Err(e) => return fail(2, &e.to_string()),
    };
    let found = match ambient_z2m::probe(&link) {
        Ok(found) => found,
        Err(why) => return fail(1, &why),
    };
    if let Err(e) = ambient_z2m_store::save(&paths.data_dir, &link) {
        return fail(1, &e.to_string());
    }
    let told = tell_helper(paths, config, "/v1/ambient/refresh");
    emit(&json!({
        "ok": true,
        "linked": true,
        "broker": format!("{}:{}", link.host, link.port),
        "user": link.username,
        "baseTopic": link.base_topic,
        "fps": link.fps,
        "lights": found.iter().map(|f| found_json(f, link.lights.iter().find(|l| l.name == f.name))).collect::<Vec<_>>(),
        "framesTo": found.first().map(|f| f.name.clone()),
        "helperTold": told,
        "next": "switch ambient light on in Settings, Ambient light",
    }));
    0
}

/// Removes the saved broker, its password and the lights.
fn z2m_unlink(paths: Paths, config: Config) -> i32 {
    let removed = ambient_z2m_store::clear(&paths.data_dir);
    let told = tell_helper(&paths, &config, "/v1/ambient/refresh");
    emit(&json!({"ok": true, "linked": false, "removed": removed, "helperTold": told}));
    0
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
            let port = info
                .and_then(|i| i.port)
                .or((config.listen.port != 0).then_some(config.listen.port));
            let ip = config.listen.host.parse::<IpAddr>().ok();
            let healthy = port.zip(ip).map(|(port, ip)| healthy(ip, port));
            emit(
                &json!({"ok": true, "running": true, "pid": info.map(|i| i.pid), "port": port, "healthy": healthy}),
            );
            0
        }
        Err(e) => fail(
            1,
            &format!("cannot check the lock {}: {e}", paths.lock_file().display()),
        ),
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
    fn the_plugin_commands_take_an_id_a_code_and_a_package() {
        let a = parse_args(&args(&[
            "plugin-enable",
            "--brand",
            "/var/palm/apps/x/brand.json",
            "--id",
            "home-assistant",
            "--approve",
            "abc123",
        ]))
        .unwrap();
        assert_eq!(
            (a.id.as_deref(), a.approve.as_deref()),
            (Some("home-assistant"), Some("abc123"))
        );
        assert_eq!(
            package_dir(&a).unwrap(),
            PathBuf::from("/var/palm/apps/x/payload/plugins/home-assistant"),
            "the package the ipk carries"
        );
        let a = parse_args(&args(&["plugin-consent", "--from", "/tmp/pkg"])).unwrap();
        assert_eq!(package_dir(&a).unwrap(), PathBuf::from("/tmp/pkg"));
        let a = parse_args(&args(&["plugin-consent", "--id", "x"])).unwrap();
        assert!(package_dir(&a).unwrap_err().contains("--brand"));
        assert!(parse_args(&args(&["plugin-enable", "--approve"])).is_err());
        // The token is never an argument: the owner's command has no flag for it.
        assert!(parse_args(&args(&["ha-link", "--token", "x"])).is_err());
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
        let a = parse_args(&args(&[
            "status",
            "--data-dir",
            dir.path().to_str().unwrap(),
        ]))
        .unwrap();
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
            outcome_json(&Outcome::Listening {
                pid: 5,
                port: 18731
            }),
            json!({"ok": true, "started": true, "pid": 5, "port": 18731})
        );
        assert_eq!(
            outcome_json(&Outcome::Already { pid: 5 }),
            json!({"ok": true, "started": false, "already": true, "pid": 5})
        );
        assert_eq!(
            outcome_json(&Outcome::Failed("x".into())),
            json!({"ok": false, "error": "x"})
        );
    }
}
