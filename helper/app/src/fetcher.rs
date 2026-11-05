//! The HTTPS transport for weather and news, and the choice between the two that exist.
//!
//! LumaShow's `fetch` runs `openssl s_client` and checks the certificate chain and host name itself; it is the first choice.
//! It passes `-no-CAstore`, an option that only OpenSSL 3 knows, and the set's userland is old (glibc 2.28): its `openssl`
//! may be 1.1.1, on which every fetch would fail with "unknown option". So the helper asks `openssl version` once at start
//! (`fetch.backend: "auto"`) and, when it is not OpenSSL 3 or later, uses the set's own `curl` instead ([`CurlFetch`]: HTTPS
//! only, TLS 1.2 or later, certificate and host name verified by curl, no redirects, a hard deadline and size cap, the child
//! run as an unprivileged user like `fetch`'s). `fetch.backend: "openssl"` or `"curl"` forces one. `/v1/state` and the log say
//! which is in use.

use crate::config::{FetchConfig, find_ca_bundle};
use crate::sys;
use fetch::{Fetch, FetchError, FetchLimits, TlsFetch};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// The chosen transport and why, for the log and the diagnostics.
pub struct Chosen {
    pub fetcher: Arc<dyn Fetch>,
    /// `openssl`, `curl` or `none`.
    pub backend: &'static str,
    pub detail: String,
}

/// Runs the TLS child as an unprivileged user and, if the system refuses that (the user cannot run the program, read the CA
/// bundle or open a socket: `FetchError::Denied`), as the helper's own user from then on, saying so once in the log. A set that
/// restricts the network or the bundle to some users must not leave Home without weather and news over a defence in depth.
struct Adaptive {
    unprivileged: Arc<dyn Fetch>,
    as_helper: Arc<dyn Fetch>,
    user: String,
    refused: AtomicBool,
}

impl Fetch for Adaptive {
    fn get(&self, url: &str, limits: FetchLimits) -> Result<Vec<u8>, FetchError> {
        if self.refused.load(Ordering::Relaxed) {
            return self.as_helper.get(url, limits);
        }
        match self.unprivileged.get(url, limits) {
            Err(FetchError::Denied(why)) => {
                self.refused.store(true, Ordering::Relaxed);
                crate::log_warn!(
                    "fetch: the system refused the TLS child as {} ({why}); it runs as the helper's user from now on",
                    self.user
                );
                self.as_helper.get(url, limits)
            }
            other => other,
        }
    }
}

/// A transport that is not available: every request says why.
struct Unavailable(String);

impl Fetch for Unavailable {
    fn get(&self, _url: &str, _limits: FetchLimits) -> Result<Vec<u8>, FetchError> {
        Err(FetchError::Failed(self.0.clone()))
    }
}

/// The major version in `openssl version`'s output (`OpenSSL 3.0.13 30 Jan 2024`); `None` for anything else, LibreSSL included.
pub fn openssl_major(version_output: &str) -> Option<u32> {
    let rest = version_output.trim().strip_prefix("OpenSSL ")?;
    rest.split('.').next()?.parse().ok()
}

/// What the program at `path` prints for `version`, within two seconds.
fn program_version(program: &str) -> Option<String> {
    let mut child = Command::new(sys::which(program)?)
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let (tx, rx) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("fetch-probe".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            let mut text = String::new();
            let _ = out.by_ref().take(4096).read_to_string(&mut text);
            let _ = tx.send(text);
        });
    if spawned.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(text) => {
            let _ = child.wait();
            Some(text)
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// The user and group the TLS child runs as: only when the helper is root and the configured user exists.
fn drop_target(config: &FetchConfig) -> Option<(u32, u32)> {
    if !sys::is_root() {
        return None;
    }
    let user = config.user.as_ref()?;
    match sys::lookup_user(user) {
        Some(ids) => Some(ids),
        None => {
            crate::log_warn!("fetch: user {user} does not exist; the TLS child runs as root");
            None
        }
    }
}

/// Wraps a transport built with and without the privilege drop (`build(None)` runs the child as the helper's user).
fn with_drop<T: Fetch + 'static>(
    config: &FetchConfig,
    build: impl Fn(Option<(u32, u32)>) -> T,
) -> Arc<dyn Fetch> {
    match drop_target(config) {
        Some(ids) => Arc::new(Adaptive {
            unprivileged: Arc::new(build(Some(ids))),
            as_helper: Arc::new(build(None)),
            user: config.user.clone().unwrap_or_default(),
            refused: AtomicBool::new(false),
        }),
        None => Arc::new(build(None)),
    }
}

fn via_openssl(config: &FetchConfig, bundle: PathBuf, why: String) -> Chosen {
    let program = sys::which(&config.openssl).unwrap_or_else(|| PathBuf::from(&config.openssl));
    Chosen {
        fetcher: with_drop(config, |ids| {
            let fetcher = TlsFetch::new(program.clone(), bundle.clone());
            match ids {
                Some((uid, gid)) => fetcher.unprivileged(uid, gid),
                None => fetcher,
            }
        }),
        backend: "openssl",
        detail: why,
    }
}

fn via_curl(config: &FetchConfig, bundle: Option<PathBuf>, why: String) -> Chosen {
    let program =
        sys::which(&config.curl).map_or_else(|| config.curl.clone(), |p| p.display().to_string());
    Chosen {
        fetcher: with_drop(config, |ids| CurlFetch {
            program: program.clone(),
            bundle: bundle.clone(),
            drop_to: ids,
        }),
        backend: "curl",
        detail: why,
    }
}

/// Picks the transport (see the module docs).
pub fn choose(config: &FetchConfig) -> Chosen {
    let bundle = find_ca_bundle(config);
    let none = |why: &str| Chosen {
        fetcher: Arc::new(Unavailable(why.to_owned())),
        backend: "none",
        detail: why.to_owned(),
    };
    match config.backend.as_str() {
        "openssl" => match bundle {
            Some(bundle) => via_openssl(config, bundle, "forced by fetch.backend".to_owned()),
            None => none("no CA certificate bundle on this system"),
        },
        "curl" => {
            if sys::which(&config.curl).is_some() {
                via_curl(config, bundle, "forced by fetch.backend".to_owned())
            } else {
                none("fetch.backend is curl but curl was not found")
            }
        }
        _ => {
            let version = program_version(&config.openssl);
            let major = version.as_deref().and_then(openssl_major);
            match (major, &bundle) {
                (Some(major), Some(_)) if major >= 3 => {
                    let bundle = bundle.clone().unwrap_or_default();
                    via_openssl(
                        config,
                        bundle,
                        format!("openssl {}", version.unwrap_or_default().trim()),
                    )
                }
                _ if sys::which(&config.curl).is_some() => {
                    let why = match &version {
                        Some(v) => format!("{} is not OpenSSL 3 or later", v.trim()),
                        None => format!("{} could not be run", config.openssl),
                    };
                    via_curl(config, bundle, why)
                }
                _ => match bundle {
                    Some(bundle) => via_openssl(
                        config,
                        bundle,
                        "no curl to fall back on; openssl may be too old for s_client's options"
                            .to_owned(),
                    ),
                    None => none("no CA certificate bundle and no curl on this system"),
                },
            }
        }
    }
}

/// HTTPS through `curl`, for a set whose `openssl` is too old for LumaShow's `fetch`.
#[derive(Clone, Debug)]
pub struct CurlFetch {
    program: String,
    bundle: Option<PathBuf>,
    drop_to: Option<(u32, u32)>,
}

impl CurlFetch {
    pub fn new(program: impl Into<String>, bundle: Option<PathBuf>) -> Self {
        Self {
            program: program.into(),
            bundle,
            drop_to: None,
        }
    }

    /// The set's curl as `config` names it, with its CA bundle, run as the unprivileged user when `unprivileged` (and the helper is root).
    pub fn of(config: &FetchConfig, unprivileged: bool) -> Self {
        Self {
            program: sys::which(&config.curl)
                .map_or_else(|| config.curl.clone(), |p| p.display().to_string()),
            bundle: find_ca_bundle(config),
            drop_to: if unprivileged {
                drop_target(config)
            } else {
                None
            },
        }
    }

    /// One request of any method, with headers and a body, and its answer whatever the status (a service may say in its body why it
    /// said no). The address, the headers and the body reach curl on its standard input, never on its command line, so a token is not
    /// in the process list. No redirect is followed; `http://` is for an address the caller has checked.
    pub fn exchange(
        &self,
        x: &Exchange<'_>,
        limits: FetchLimits,
    ) -> Result<(u16, Vec<u8>), FetchError> {
        let https = x.url.starts_with("https://");
        if !https && !x.url.starts_with("http://") {
            return Err(FetchError::Failed("that is not a web address".to_owned()));
        }
        let quoted = |text: &str| {
            if text.chars().any(char::is_control) {
                return Err(FetchError::Failed(
                    "the request holds a character that cannot be sent".to_owned(),
                ));
            }
            Ok(text.replace('\\', "\\\\").replace('"', "\\\""))
        };
        let mut config = format!(
            "url = \"{}\"\nrequest = \"{}\"\nuser-agent = \"LumaTV\"\n",
            quoted(x.url)?,
            quoted(x.method)?
        );
        for (name, value) in x.headers {
            config.push_str(&format!(
                "header = \"{}: {}\"\n",
                quoted(name)?,
                quoted(value)?
            ));
        }
        if let Some(body) = x.body {
            let text = std::str::from_utf8(body)
                .map_err(|_| FetchError::Failed("the request's body is not text".to_owned()))?;
            config.push_str(&format!("data-binary = \"{}\"\n", quoted(text)?));
        }
        let mut command = Command::new(&self.program);
        command
            .args(["--config", "-", "--silent", "--show-error", "--proto"])
            .arg(if https { "=https" } else { "=http" })
            .args(["--tlsv1.2", "--connect-timeout", "5", "--max-time"])
            .arg(limits.total.as_secs().max(1).to_string())
            .arg("--max-filesize")
            .arg(limits.max_bytes.to_string())
            .args(["--write-out", "\n%{http_code}"]);
        if let Some(bundle) = &self.bundle {
            command.arg("--cacert").arg(bundle);
        }
        self.run(command, Some(config.into_bytes()), limits)
    }

    /// Runs curl as `command` says, with `input` on its standard input, and reads the body and the status `--write-out` appends.
    fn run(
        &self,
        mut command: Command,
        input: Option<Vec<u8>>,
        limits: FetchLimits,
    ) -> Result<(u16, Vec<u8>), FetchError> {
        let started = Instant::now();
        let deadline = started + limits.total;
        command
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some((uid, gid)) = self.drop_to {
            use std::os::unix::process::CommandExt;
            command.uid(uid).gid(gid);
        }
        let mut child = command.spawn().map_err(|e| {
            let why = format!("could not start curl: {e}");
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                FetchError::Denied(why)
            } else {
                FetchError::Failed(why)
            }
        })?;
        let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
            stop(&mut child);
            return Err(FetchError::Failed(
                "curl's output could not be read".to_owned(),
            ));
        };
        // One byte past the cap tells a body that fits from one that does not.
        let cap = limits.max_bytes.saturating_add(64);
        let out = read_capped(out, cap);
        let err = read_capped(err, 4096);
        if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
            // curl reads its whole configuration before it connects; a write that fails is a curl that stopped, and its exit says why
            let _ = std::io::Write::write_all(&mut stdin, &input);
        }
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline + Duration::from_secs(2) => {
                    stop(&mut child);
                    return Err(FetchError::Timeout);
                }
                // A short nap while a quick answer is likely, a longer one after: no spin for the length of a slow request.
                Ok(None) => thread::sleep(Duration::from_millis(
                    if started.elapsed() < Duration::from_millis(300) {
                        5
                    } else {
                        50
                    },
                )),
                Err(e) => {
                    stop(&mut child);
                    return Err(FetchError::Failed(format!("could not wait for curl: {e}")));
                }
            }
        };
        let body = out.join().unwrap_or_default();
        let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
        // The reader stops at the cap and closes the pipe, which ends a curl that has more to say (by a signal or a write error):
        // that is a body that is too large, whatever the exit status says.
        if body.len() >= cap {
            return Err(FetchError::TooLarge);
        }
        if !status.success() {
            return Err(classify(status.code(), &stderr));
        }
        // The body, then "\n" and the three digits of the status that `--write-out` appended.
        let Some(split) = body.len().checked_sub(4) else {
            return Err(FetchError::Failed("curl gave no status".to_owned()));
        };
        let (content, tail) = body.split_at(split);
        let code = (tail[0] == b'\n' && tail[1..].iter().all(u8::is_ascii_digit))
            .then(|| {
                std::str::from_utf8(&tail[1..])
                    .ok()
                    .and_then(|t| t.parse::<u16>().ok())
            })
            .flatten()
            .ok_or_else(|| FetchError::Failed("curl gave no status".to_owned()))?;
        if content.len() > limits.max_bytes {
            return Err(FetchError::TooLarge);
        }
        Ok((code, content.to_vec()))
    }
}

/// What [`CurlFetch::exchange`] sends.
pub struct Exchange<'a> {
    /// `GET`, `POST`, `PUT`.
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    /// A JSON body.
    pub body: Option<&'a [u8]>,
}

/// Only `https`, no user information, no control or space characters: nothing that could be taken for an option or break the
/// request.
fn check_url(url: &str) -> Result<(), FetchError> {
    let bad = || FetchError::Failed("that is not a secure web address".to_owned());
    let rest = url.strip_prefix("https://").ok_or_else(bad)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if rest.is_empty()
        || authority.is_empty()
        || authority.contains('@')
        || authority.starts_with('-')
    {
        return Err(bad());
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(bad());
    }
    Ok(())
}

/// The error a failed curl stands for, from its exit code and what it said.
fn classify(code: Option<i32>, stderr: &str) -> FetchError {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect::<String>();
    match code {
        Some(6 | 7) => FetchError::Offline,
        Some(28) => FetchError::Timeout,
        Some(63) => FetchError::TooLarge,
        // The CA file cannot be used (missing, or not readable by this user): no certificate was judged.
        Some(77) => FetchError::Denied(line),
        // Failed SSL handshake, bad certificate, unknown CA, bad CRL, pinned key: a certificate that is refused.
        Some(35 | 51 | 58 | 59 | 60 | 82 | 83 | 90 | 91) => FetchError::Untrusted(line),
        Some(126 | 127) => FetchError::Failed(format!("could not run curl: {line}")),
        Some(n) => FetchError::Failed(if line.is_empty() {
            format!("curl stopped with status {n}")
        } else {
            line
        }),
        None => FetchError::Failed("curl was stopped by a signal".to_owned()),
    }
}

fn read_capped(mut pipe: impl Read + Send + 'static, cap: usize) -> thread::JoinHandle<Vec<u8>> {
    thread::Builder::new()
        .name("fetch-curl-read".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            let mut data = Vec::new();
            let _ = pipe.by_ref().take(cap as u64).read_to_end(&mut data);
            data
        })
        .unwrap_or_else(|_| thread::spawn(Vec::new))
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

impl Fetch for CurlFetch {
    fn get(&self, url: &str, limits: FetchLimits) -> Result<Vec<u8>, FetchError> {
        check_url(url)?;
        let mut command = Command::new(&self.program);
        command
            .args([
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--tlsv1.2",
            ])
            .args(["--connect-timeout", "5", "--max-time"])
            .arg(limits.total.as_secs().max(1).to_string())
            .arg("--max-filesize")
            .arg(limits.max_bytes.to_string())
            .args([
                "--user-agent",
                "LumaTV",
                "--header",
                "Accept: */*",
                "--write-out",
                "\n%{http_code}",
            ]);
        if let Some(bundle) = &self.bundle {
            command.arg("--cacert").arg(bundle);
        }
        command.arg(url);
        let (code, content) = self.run(command, None, limits)?;
        if !(200..300).contains(&code) {
            return Err(FetchError::Http(code));
        }
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    #[test]
    fn the_openssl_major_version_is_read() {
        assert_eq!(
            openssl_major("OpenSSL 3.0.13 30 Jan 2024 (Library: OpenSSL 3.0.13 30 Jan 2024)\n"),
            Some(3)
        );
        assert_eq!(openssl_major("OpenSSL 1.1.1w  11 Sep 2023"), Some(1));
        assert_eq!(openssl_major("OpenSSL 3.5.0"), Some(3));
        assert_eq!(openssl_major("LibreSSL 3.8.2"), None);
        assert_eq!(openssl_major(""), None);
        assert_eq!(openssl_major("OpenSSL x.y"), None);
    }

    #[test]
    fn only_plain_https_addresses_pass() {
        for ok in [
            "https://api.open-meteo.com/v1/forecast?latitude=1&longitude=2",
            "https://feeds.bbci.co.uk/news/world/rss.xml",
            "https://127.0.0.1:8443/",
        ] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://example.com/",
            "ftp://example.com/",
            "https://",
            "https:///path",
            "https://user@example.com/",
            "https://example.com/a b",
            "https://example.com/\n",
            "https://-oSomething/",
            "file:///etc/passwd",
            "-o/etc/passwd",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn curls_exit_codes_become_the_errors_the_feeds_report() {
        assert_eq!(
            classify(Some(6), "curl: (6) Could not resolve host: x"),
            FetchError::Offline
        );
        assert_eq!(classify(Some(7), ""), FetchError::Offline);
        assert!(matches!(
            classify(Some(77), "curl: (77) error setting certificate file: /etc/ssl/ca.pem"),
            FetchError::Denied(why) if why.contains("certificate file")
        ));
        assert_eq!(
            classify(Some(28), "curl: (28) Operation timed out"),
            FetchError::Timeout
        );
        assert_eq!(classify(Some(63), ""), FetchError::TooLarge);
        assert!(matches!(
            classify(Some(60), "curl: (60) SSL certificate problem: certificate has expired"),
            FetchError::Untrusted(why) if why.contains("expired")
        ));
        assert!(
            matches!(classify(Some(127), "sh: curl: not found"), FetchError::Failed(why) if why.contains("could not run"))
        );
        assert!(matches!(classify(Some(56), ""), FetchError::Failed(why) if why.contains("56")));
        assert!(matches!(classify(None, ""), FetchError::Failed(_)));
    }

    /// A fake `curl`: records its arguments next to itself and answers by the end of the URL.
    fn fake_curl(dir: &Path) -> PathBuf {
        let args = dir.join("curl-args");
        let program = dir.join("curl");
        let script = format!(
            r#"#!/bin/sh
echo "$@" > '{args}'
for last; do :; done
case "$last" in
  */ok) printf 'hello body\n200' ;;
  */json) printf '{{"a":1}}\n200' ;;
  */missing) printf 'nope\n404' ;;
  */offline) echo 'curl: (6) Could not resolve host: x' >&2; exit 6 ;;
  */cert) echo 'curl: (60) SSL certificate problem: self-signed certificate' >&2; exit 60 ;;
  */slow) sleep 10 ;;
  */big) head -c 5000 /dev/zero | tr '\0' 'x'; printf '\n200' ;;
  */nostatus) printf 'just a body' ;;
  *) exit 1 ;;
esac
"#,
            args = args.display()
        );
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        settle(&program);
        program
    }

    /// Runs a script that was just written until the system lets it run. Another test thread's fork can hold the script's write
    /// descriptor for a moment, and exec of a file that is open for writing fails with "text file busy"; once one exec has
    /// worked, no later one can fail that way.
    fn settle(program: &Path) {
        for _ in 0..300 {
            let result = Command::new(program)
                .arg("probe")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            match result {
                Err(e) if e.raw_os_error() == Some(26) => thread::sleep(Duration::from_millis(10)),
                _ => return,
            }
        }
    }

    fn limits(secs: u64, max: usize) -> FetchLimits {
        FetchLimits {
            total: Duration::from_secs(secs),
            max_bytes: max,
        }
    }

    #[test]
    fn a_reply_is_returned_with_its_status_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let curl = CurlFetch::new(
            fake_curl(dir.path()).to_str().unwrap(),
            Some(PathBuf::from("/etc/ssl/ca.pem")),
        );
        assert_eq!(
            curl.get("https://example.test/ok", limits(5, 1024))
                .unwrap(),
            b"hello body"
        );
        assert_eq!(
            curl.get("https://example.test/json", limits(5, 1024))
                .unwrap(),
            br#"{"a":1}"#
        );
        // What was asked of curl: https only, no redirects followed, a time and size cap, the bundle, the URL last.
        let args = std::fs::read_to_string(dir.path().join("curl-args")).unwrap();
        for wanted in [
            "--proto =https",
            "--proto-redir =https",
            "--tlsv1.2",
            "--max-time 5",
            "--max-filesize 1024",
            "--cacert /etc/ssl/ca.pem",
        ] {
            assert!(args.contains(wanted), "{wanted} missing from {args}");
        }
        assert!(
            !args.contains(" -L")
                && !args.contains("--location")
                && !args.contains("--insecure")
                && !args.contains(" -k"),
            "{args}"
        );
        assert!(args.trim_end().ends_with("https://example.test/json"));
    }

    #[test]
    fn failures_are_classified() {
        let dir = tempfile::tempdir().unwrap();
        let curl = CurlFetch::new(fake_curl(dir.path()).to_str().unwrap(), None);
        assert_eq!(
            curl.get("https://example.test/missing", limits(5, 1024)),
            Err(FetchError::Http(404))
        );
        assert_eq!(
            curl.get("https://example.test/offline", limits(5, 1024)),
            Err(FetchError::Offline)
        );
        assert!(matches!(
            curl.get("https://example.test/cert", limits(5, 1024)),
            Err(FetchError::Untrusted(_))
        ));
        assert_eq!(
            curl.get("https://example.test/big", limits(5, 1000)),
            Err(FetchError::TooLarge)
        );
        assert!(matches!(
            curl.get("https://example.test/nostatus", limits(5, 1024)),
            Err(FetchError::Failed(_))
        ));
        assert!(matches!(
            curl.get("http://example.test/ok", limits(5, 1024)),
            Err(FetchError::Failed(_))
        ));
        let args = std::fs::read_to_string(dir.path().join("curl-args")).unwrap();
        assert!(
            !args.contains("--cacert"),
            "no bundle was configured: {args}"
        );
    }

    #[test]
    fn a_stuck_curl_is_killed_at_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let curl = CurlFetch::new(fake_curl(dir.path()).to_str().unwrap(), None);
        let started = Instant::now();
        assert_eq!(
            curl.get("https://example.test/slow", limits(1, 1024)),
            Err(FetchError::Timeout)
        );
        assert!(
            started.elapsed() < Duration::from_secs(6),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_missing_curl_is_reported_not_panicked() {
        let curl = CurlFetch::new("/nonexistent/curl", None);
        assert!(
            matches!(curl.get("https://example.test/ok", limits(2, 1024)), Err(FetchError::Failed(why)) if why.contains("curl"))
        );
    }

    #[test]
    fn an_exchange_sends_address_headers_and_body_on_standard_input_and_returns_any_status() {
        let dir = tempfile::tempdir().unwrap();
        let (args, input) = (dir.path().join("args"), dir.path().join("stdin"));
        let program = dir.path().join("curl");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\necho \"$@\" > '{}'\ncat > '{}'\nprintf '{{\"code\":4003}}\\n400'\n",
                args.display(),
                input.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        settle(&program);
        let curl = CurlFetch::new(program.to_str().unwrap(), None);
        let x = Exchange {
            method: "POST",
            url: "https://json.schedulesdirect.org/20141201/token",
            headers: &[("token", "tk-secret"), ("Content-Type", "application/json")],
            body: Some(br#"{"username":"n","password":"a\"b"}"#),
        };
        let (status, body) = curl.exchange(&x, limits(5, 1024)).unwrap();
        assert_eq!((status, body.as_slice()), (400, &br#"{"code":4003}"#[..]));
        let argv = std::fs::read_to_string(&args).unwrap();
        assert!(
            !argv.contains("schedulesdirect") && !argv.contains("tk-secret"),
            "{argv}"
        );
        assert!(
            argv.contains("--config -") && argv.contains("--proto =https"),
            "{argv}"
        );
        assert_eq!(
            std::fs::read_to_string(&input).unwrap(),
            "url = \"https://json.schedulesdirect.org/20141201/token\"\nrequest = \"POST\"\nuser-agent = \"LumaTV\"\n\
             header = \"token: tk-secret\"\nheader = \"Content-Type: application/json\"\n\
             data-binary = \"{\\\"username\\\":\\\"n\\\",\\\"password\\\":\\\"a\\\\\\\"b\\\"}\"\n"
        );
        let bad = Exchange {
            url: "https://x/\n",
            ..x
        };
        assert!(curl.exchange(&bad, limits(5, 1024)).is_err());
    }

    fn config(
        dir: &Path,
        openssl_says: Option<&str>,
        with_curl: bool,
        backend: &str,
    ) -> FetchConfig {
        let openssl = dir.join("openssl");
        match openssl_says {
            Some(text) => {
                std::fs::write(&openssl, format!("#!/bin/sh\necho '{text}'\n")).unwrap();
                std::fs::set_permissions(&openssl, std::fs::Permissions::from_mode(0o755)).unwrap();
                settle(&openssl);
            }
            None => {
                let _ = std::fs::remove_file(&openssl);
            }
        }
        let curl = if with_curl {
            fake_curl(dir)
        } else {
            dir.join("no-curl")
        };
        let bundle = dir.join("ca.pem");
        std::fs::write(&bundle, "pem").unwrap();
        let json = serde_json::json!({
            "backend": backend,
            "openssl": openssl.display().to_string(),
            "curl": curl.display().to_string(),
            "caBundle": bundle.display().to_string(),
            "user": null,
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn openssl_3_is_used_when_there_and_curl_when_it_is_older() {
        let dir = tempfile::tempdir().unwrap();
        let chosen = choose(&config(
            dir.path(),
            Some("OpenSSL 3.0.13 30 Jan 2024"),
            true,
            "auto",
        ));
        assert_eq!(chosen.backend, "openssl", "{}", chosen.detail);
        let chosen = choose(&config(
            dir.path(),
            Some("OpenSSL 1.1.1w  11 Sep 2023"),
            true,
            "auto",
        ));
        assert_eq!(chosen.backend, "curl");
        assert!(chosen.detail.contains("not OpenSSL 3"), "{}", chosen.detail);
        let chosen = choose(&config(dir.path(), Some("LibreSSL 3.8.2"), true, "auto"));
        assert_eq!(chosen.backend, "curl");
        let chosen = choose(&config(dir.path(), None, true, "auto"));
        assert_eq!(chosen.backend, "curl");
        assert!(
            chosen.detail.contains("could not be run"),
            "{}",
            chosen.detail
        );
    }

    #[test]
    fn with_no_curl_an_old_openssl_is_still_tried_and_nothing_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let chosen = choose(&config(dir.path(), Some("OpenSSL 1.1.1w"), false, "auto"));
        assert_eq!(chosen.backend, "openssl");
        assert!(chosen.detail.contains("too old"), "{}", chosen.detail);
        let mut cfg = config(dir.path(), None, false, "auto");
        cfg.ca_bundle = Some("/nonexistent/ca.pem".to_owned());
        // A configured bundle that is missing is still a bundle (curl or openssl will say so); with neither program the
        // openssl route is the one that reports it.
        assert_eq!(choose(&cfg).backend, "openssl");
    }

    /// Answers by a fixed result and counts its calls.
    struct Counted {
        answer: Result<Vec<u8>, FetchError>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl Counted {
        fn new(answer: Result<Vec<u8>, FetchError>) -> Arc<Self> {
            Arc::new(Self {
                answer,
                calls: std::sync::atomic::AtomicUsize::new(0),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Fetch for Counted {
        fn get(&self, _url: &str, _limits: FetchLimits) -> Result<Vec<u8>, FetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.answer.clone()
        }
    }

    #[test]
    fn a_refused_unprivileged_child_falls_back_once_and_stays_there() {
        let unprivileged = Counted::new(Err(FetchError::Denied("permission denied".into())));
        let as_helper = Counted::new(Ok(b"fine".to_vec()));
        let adaptive = Adaptive {
            unprivileged: unprivileged.clone(),
            as_helper: as_helper.clone(),
            user: "nobody".into(),
            refused: AtomicBool::new(false),
        };
        assert_eq!(
            adaptive.get("https://x.test/", limits(1, 10)).unwrap(),
            b"fine"
        );
        assert_eq!(
            adaptive.get("https://x.test/", limits(1, 10)).unwrap(),
            b"fine"
        );
        assert_eq!(
            (unprivileged.calls(), as_helper.calls()),
            (1, 2),
            "the refused user is not tried again"
        );
    }

    #[test]
    fn other_failures_do_not_drop_the_privilege_drop() {
        for error in [
            FetchError::Offline,
            FetchError::Timeout,
            FetchError::Http(500),
            FetchError::Untrusted("x".into()),
        ] {
            let unprivileged = Counted::new(Err(error.clone()));
            let as_helper = Counted::new(Ok(b"fine".to_vec()));
            let adaptive = Adaptive {
                unprivileged: unprivileged.clone(),
                as_helper: as_helper.clone(),
                user: "nobody".into(),
                refused: AtomicBool::new(false),
            };
            assert_eq!(adaptive.get("https://x.test/", limits(1, 10)), Err(error));
            assert_eq!(as_helper.calls(), 0);
        }
    }

    #[test]
    fn as_root_a_user_that_cannot_reach_the_program_falls_back_to_running_as_the_helper() {
        if !sys::is_root() || sys::lookup_user("nobody").is_none() {
            return;
        }
        // The temporary directory is private to the helper's user, so `nobody` cannot even reach the fake curl in it.
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(dir.path(), Some("OpenSSL 1.1.1w"), true, "curl");
        config.user = Some("nobody".to_owned());
        let chosen = choose(&config);
        assert_eq!(chosen.backend, "curl");
        assert_eq!(
            chosen
                .fetcher
                .get("https://example.test/ok", limits(5, 1024))
                .unwrap(),
            b"hello body"
        );
        assert_eq!(
            chosen
                .fetcher
                .get("https://example.test/json", limits(5, 1024))
                .unwrap(),
            br#"{"a":1}"#
        );
    }

    #[test]
    fn the_backend_can_be_forced() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            choose(&config(dir.path(), Some("OpenSSL 3.0.13"), true, "curl")).backend,
            "curl"
        );
        assert_eq!(
            choose(&config(dir.path(), Some("OpenSSL 1.1.1w"), true, "openssl")).backend,
            "openssl"
        );
        let none = choose(&config(dir.path(), Some("OpenSSL 3.0.13"), false, "curl"));
        assert_eq!(none.backend, "none");
        let err = none
            .fetcher
            .get("https://example.test/", limits(1, 10))
            .unwrap_err();
        assert!(matches!(err, FetchError::Failed(why) if why.contains("curl")));
    }
}
