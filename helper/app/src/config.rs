//! The helper's own configuration (`<data dir>/helper.json`) and where its files live.
//!
//! Two kinds of setting exist and they are kept apart. What a person chooses (weather zip, news edition, the receiver's
//! address) is a *setting*: it has a schema, is validated, and lives in the settings store. This file holds how the process
//! itself runs: the listen port, the log, the paths of programs it calls, and the receiver polling rhythm. A file that does not
//! exist means the defaults; a file that does not parse stops the helper with the reason (a typo in `listen` must not be
//! silently ignored).

use serde::Deserialize;
use std::fmt;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// The default loopback port of the API.
pub const DEFAULT_PORT: u16 = 18731;

/// Where the helper's files are, all under the data dir.
#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// `helper.json`
    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("helper.json")
    }

    /// `log-helper`
    pub fn log_file(&self) -> PathBuf {
        self.data_dir.join("log-helper")
    }

    /// `helper.lock`: held (flock) by the running helper, which writes its pid into it.
    pub fn lock_file(&self) -> PathBuf {
        self.data_dir.join("helper.lock")
    }

    /// `cache/`: the saved-first weather and news documents.
    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache")
    }

    /// `settings/`: one file of stored values per settings owner.
    pub fn settings_dir(&self) -> PathBuf {
        self.data_dir.join("settings")
    }
}

/// A config problem, in words.
#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    pub listen: Listen,
    pub log: LogConfig,
    pub screen: ScreenConfig,
    pub receiver: ReceiverTuning,
    pub fetch: FetchConfig,
    pub luna: LunaConfig,
    /// How far the helper's scheduling priority is lowered (`nice`): it is background work and must never take a core from the
    /// compositor. 0 leaves it as started.
    pub nice: i32,
    /// Browser origins allowed to call the API (empty: none). Requests from the compositor's QML and from `curl` carry no
    /// `Origin` header and are always allowed; a web page does carry one, and a page on the TV's browser must not be able to
    /// drive the helper.
    pub allowed_origins: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: Listen::default(),
            log: LogConfig::default(),
            screen: ScreenConfig::default(),
            receiver: ReceiverTuning::default(),
            fetch: FetchConfig::default(),
            luna: LunaConfig::default(),
            nice: 10,
            allowed_origins: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Listen {
    /// A loopback address. Anything else is refused: opening the LAN is a decision of the owner, made on the TV.
    pub host: String,
    /// 0 asks the system for a free port (tests).
    pub port: u16,
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: DEFAULT_PORT,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct LogConfig {
    /// `error`, `warn`, `info` or `debug`.
    pub level: String,
    /// The size at which the log file becomes `log-helper.1` and a new one starts.
    pub max_bytes: u64,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
            max_bytes: 256 * 1024,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct ScreenConfig {
    /// `luna` reads the power state from `luna-send`; `on` treats the screen as always on (development on a host).
    pub source: String,
    /// The poll period when the power-state subscription is not available.
    pub poll_ms: u64,
}

impl Default for ScreenConfig {
    fn default() -> Self {
        Self {
            source: "luna".to_owned(),
            poll_ms: 5_000,
        }
    }
}

/// How the receiver is polled. The address and the on/off switch are settings (`base.receiver`).
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct ReceiverTuning {
    /// The period between reads while the screen is on and nothing is happening.
    pub poll_ms: u64,
    /// The period during a burst, which follows a nudge (a volume key was pressed) so the change is seen within a fraction of a second.
    pub fast_poll_ms: u64,
    /// How long a burst lasts.
    pub burst_ms: u64,
    /// The longest wait between reads while the receiver does not answer.
    pub max_backoff_ms: u64,
    /// A reading older than this is hidden rather than shown (the plan's 6 s).
    pub hide_after_ms: u64,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
}

impl Default for ReceiverTuning {
    fn default() -> Self {
        Self {
            poll_ms: 2_000,
            fast_poll_ms: 250,
            burst_ms: 3_000,
            max_backoff_ms: 30_000,
            hide_after_ms: 6_000,
            connect_timeout_ms: 1_500,
            request_timeout_ms: 3_000,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct FetchConfig {
    /// The `openssl` program LumaShow's `fetch` runs as `s_client`.
    pub openssl: String,
    /// The CA bundle; unset means the first of the usual places that exists.
    pub ca_bundle: Option<String>,
    /// The user the TLS child runs as when the helper is root, so the program that parses certificates holds no privilege.
    /// `null` runs it as the helper does.
    #[serde(default = "default_fetch_user")]
    pub user: Option<String>,
}

fn default_fetch_user() -> Option<String> {
    Some("nobody".to_owned())
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            openssl: fetch::IMAGE_OPENSSL.to_owned(),
            ca_bundle: None,
            user: default_fetch_user(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct LunaConfig {
    /// The `luna-send` program (a path, or a name looked up in `PATH`).
    pub send: String,
}

impl Default for LunaConfig {
    fn default() -> Self {
        Self {
            send: "luna-send".to_owned(),
        }
    }
}

/// The CA bundles a Linux image keeps, in the order they are tried when the config names none.
pub const CA_BUNDLE_CANDIDATES: [&str; 5] = [
    fetch::IMAGE_CA_BUNDLE,
    "/etc/ssl/certs/ca-bundle.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/cert.pem",
    "/etc/ssl/ca-bundle.pem",
];

impl Config {
    /// Parses a config document and checks what serde cannot.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let config: Self = serde_json::from_str(text).map_err(|e| ConfigError(format!("helper.json: {e}")))?;
        config.check()?;
        Ok(config)
    }

    /// Reads `path`; a missing file is the defaults, any other problem is an error.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                config.check()?;
                Ok(config)
            }
            Err(e) => Err(ConfigError(format!("{}: {e}", path.display()))),
        }
    }

    fn check(&self) -> Result<(), ConfigError> {
        let err = |m: &str| Err(ConfigError(format!("helper.json: {m}")));
        match self.listen.host.parse::<IpAddr>() {
            Ok(ip) if ip.is_loopback() => {}
            _ => return err("listen.host must be a loopback address (127.0.0.1 or ::1)"),
        }
        if crate::log::Level::parse(&self.log.level).is_none() {
            return err("log.level must be error, warn, info or debug");
        }
        if !matches!(self.screen.source.as_str(), "luna" | "on") {
            return err("screen.source must be luna or on");
        }
        for (name, value) in [
            ("screen.pollMs", self.screen.poll_ms),
            ("receiver.pollMs", self.receiver.poll_ms),
            ("receiver.fastPollMs", self.receiver.fast_poll_ms),
            ("receiver.maxBackoffMs", self.receiver.max_backoff_ms),
            ("receiver.hideAfterMs", self.receiver.hide_after_ms),
            ("receiver.connectTimeoutMs", self.receiver.connect_timeout_ms),
            ("receiver.requestTimeoutMs", self.receiver.request_timeout_ms),
        ] {
            if value == 0 {
                return err(&format!("{name} must be above 0"));
            }
        }
        if !(0..=19).contains(&self.nice) {
            return err("nice must be between 0 and 19");
        }
        if self.fetch.openssl.is_empty() || self.luna.send.is_empty() {
            return err("fetch.openssl and luna.send must not be empty");
        }
        Ok(())
    }
}

/// Reads the data dir from a `brand.json`: the same rule as `tools/brand.sh` (a directory directly under `/var/lib`, lowercase
/// letters, digits and dashes), because the helper must never be pointed outside the places the mod writes.
pub fn data_dir_from_brand(brand_json: &str) -> Result<PathBuf, ConfigError> {
    let value: serde_json::Value =
        serde_json::from_str(brand_json).map_err(|e| ConfigError(format!("brand.json: {e}")))?;
    let dir = value
        .get("dataDir")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ConfigError("brand.json: dataDir is missing".to_owned()))?;
    let name = dir
        .strip_prefix("/var/lib/")
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        .ok_or_else(|| {
            ConfigError("brand.json: dataDir must be /var/lib/<lowercase letters, digits, dashes>".to_owned())
        })?;
    Ok(PathBuf::from(format!("/var/lib/{name}")))
}

/// The first CA bundle that exists, as the config or the usual places give it.
pub fn find_ca_bundle(config: &FetchConfig) -> Option<PathBuf> {
    if let Some(path) = &config.ca_bundle {
        return Some(PathBuf::from(path));
    }
    CA_BUNDLE_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_file_means_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("helper.json")).unwrap();
        assert_eq!(config.listen.port, DEFAULT_PORT);
        assert_eq!(config.listen.host, "127.0.0.1");
        assert_eq!(config.receiver.hide_after_ms, 6_000);
        assert_eq!(config.fetch.user.as_deref(), Some("nobody"));
        assert!(config.allowed_origins.is_empty());
        assert_eq!(config.nice, 10);
    }

    #[test]
    fn a_partial_file_overrides_only_what_it_names() {
        let config = Config::parse(r#"{"listen":{"port":19000},"receiver":{"pollMs":5000}}"#).unwrap();
        assert_eq!(config.listen.port, 19000);
        assert_eq!(config.listen.host, "127.0.0.1");
        assert_eq!(config.receiver.poll_ms, 5000);
        assert_eq!(config.receiver.fast_poll_ms, 250);
    }

    #[test]
    fn a_typo_is_an_error_not_a_default() {
        let e = Config::parse(r#"{"listn":{"port":1}}"#).unwrap_err();
        assert!(e.0.contains("listn"), "{e}");
        let e = Config::parse(r#"{"receiver":{"pollms":1}}"#).unwrap_err();
        assert!(e.0.contains("pollms"), "{e}");
    }

    #[test]
    fn the_listener_cannot_be_moved_off_loopback() {
        for host in ["0.0.0.0", "192.168.1.5", "::", "tv.local", ""] {
            let doc = format!(r#"{{"listen":{{"host":"{host}"}}}}"#);
            let e = Config::parse(&doc).unwrap_err();
            assert!(e.0.contains("loopback"), "{host}: {e}");
        }
        assert!(Config::parse(r#"{"listen":{"host":"::1"}}"#).is_ok());
        assert!(Config::parse(r#"{"listen":{"host":"127.0.0.2"}}"#).is_ok());
    }

    #[test]
    fn bad_values_are_named() {
        assert!(Config::parse(r#"{"log":{"level":"loud"}}"#).unwrap_err().0.contains("log.level"));
        assert!(Config::parse(r#"{"screen":{"source":"magic"}}"#).unwrap_err().0.contains("screen.source"));
        assert!(Config::parse(r#"{"receiver":{"pollMs":0}}"#).unwrap_err().0.contains("receiver.pollMs"));
        assert!(Config::parse(r#"{"nice":20}"#).unwrap_err().0.contains("nice"));
        assert!(Config::parse(r#"{"nice":-1}"#).unwrap_err().0.contains("nice"));
    }

    #[test]
    fn the_fetch_user_can_be_switched_off() {
        let config = Config::parse(r#"{"fetch":{"user":null}}"#).unwrap();
        assert_eq!(config.fetch.user, None);
        let config = Config::parse(r#"{"fetch":{"user":"daemon"}}"#).unwrap();
        assert_eq!(config.fetch.user.as_deref(), Some("daemon"));
    }

    #[test]
    fn the_data_dir_comes_from_brand_json_by_the_tools_rule() {
        let brand = r#"{"displayName":"LumaTV","dataDir":"/var/lib/lumatv"}"#;
        assert_eq!(data_dir_from_brand(brand).unwrap(), PathBuf::from("/var/lib/lumatv"));
        for bad in [
            r#"{"dataDir":"/var/lib/"}"#,
            r#"{"dataDir":"/var/lib/Luma"}"#,
            r#"{"dataDir":"/var/lib/a/b"}"#,
            r#"{"dataDir":"/tmp/x"}"#,
            r#"{"dataDir":"/var/lib/../etc"}"#,
            r#"{"name":"x"}"#,
            "not json",
        ] {
            assert!(data_dir_from_brand(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn paths_are_under_the_data_dir() {
        let p = Paths::new("/var/lib/lumatv");
        assert_eq!(p.log_file(), PathBuf::from("/var/lib/lumatv/log-helper"));
        assert_eq!(p.config_file(), PathBuf::from("/var/lib/lumatv/helper.json"));
        assert_eq!(p.lock_file(), PathBuf::from("/var/lib/lumatv/helper.lock"));
        assert_eq!(p.cache_dir(), PathBuf::from("/var/lib/lumatv/cache"));
        assert_eq!(p.settings_dir(), PathBuf::from("/var/lib/lumatv/settings"));
    }
}
