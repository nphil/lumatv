//! Durable owner data is replaced by rename; disposable catalog data is loaded as unchecked.
use lumagame_core::catalog::Listing;
use lumagame_core::state::{LoadError, OpenSession, State};
use serde::Serialize;
use lumagame_meta::secrets::Secrets;
use super::diagnostics::Config as DiagnosticsConfig;
use std::os::unix::fs::PermissionsExt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub const APP_ID: &str = "org.nphil.lumagame";

pub struct Snapshot {
    pub state: State,
    pub listing: Listing,
    pub secrets: Secrets,
    pub diagnostics: DiagnosticsConfig,
    pub notices: Vec<String>,
}

pub struct Store {
    root: PathBuf,
    writable: bool,
}

impl Store {
    pub fn open(root: PathBuf) -> io::Result<(Self, Snapshot)> {
        private_dir(&root)?;
        let mut store = Self { root, writable: true };
        let mut notices = Vec::new();
        let mut state = store.load_state(&mut notices)?;
        let mut listing: Listing = store.read_json("catalog.json").unwrap_or_default();
        listing.unchecked();
        let secrets = match Secrets::load(&store.root.join("secrets.json")) {
            Ok(secrets) => secrets,
            Err(e) => { notices.push(format!("Metadata keys could not be read: {e}")); Secrets::default() }
        };
        let diagnostics_path = store.root.join("diagnostics.json");
        let diagnostics = match fs::symlink_metadata(&diagnostics_path) {
            // webOS's installer resets the app's data files to 0777 on every install; a file this user can chmod
            // is its own, so it is made private again instead of being ignored (which silently disabled diagnostics).
            Ok(meta) if meta.is_file() && meta.len() <= 8192
                && (meta.permissions().mode() & 0o077 == 0 || fs::set_permissions(&diagnostics_path, fs::Permissions::from_mode(0o600)).is_ok()) => {
                store.read_json("diagnostics.json").unwrap_or_default()
            }
            _ => DiagnosticsConfig::default(),
        };
        if store.writable {
            match store.read_json::<OpenSession>("session.json") {
                Ok(open) => {
                    state.record(open);
                    // A crash between these two writes is safe: History::record rejects the same session twice.
                    match store.save_state(&state) {
                        Ok(()) => store.clear_checkpoint()?,
                        Err(e) => notices.push(format!("Playtime could not be saved: {e}")),
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(_) => {
                    store.preserve("session.json", "unreadable")?;
                    notices.push("The interrupted session's playtime could not be read.".into());
                }
            }
        }
        Ok((store, Snapshot { state, listing, secrets, diagnostics, notices }))
    }

    pub fn root(&self) -> &Path { &self.root }
    pub fn writable(&self) -> bool { self.writable }

    fn load_state(&mut self, notices: &mut Vec<String>) -> io::Result<State> {
        for name in ["state.json", "state.json.bak"] {
            let text = match fs::read_to_string(self.root.join(name)) {
                Ok(text) => text,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            match State::load(&text) {
                Ok(loaded) => {
                    if let Some(schema) = loaded.migrated_from {
                        self.preserve(name, &format!("schema-{schema}"))?;
                        self.save_state(&loaded.value)?;
                    } else if name.ends_with(".bak") {
                        self.save_state(&loaded.value)?;
                        notices.push("Saved choices were recovered from the backup.".into());
                    }
                    return Ok(loaded.value);
                }
                Err(e @ LoadError::Newer { .. }) => {
                    self.writable = false;
                    notices.push(format!("Saved choices need a newer LumaGame ({e}). This run will not save changes."));
                    return Ok(State::default());
                }
                Err(LoadError::Corrupt(_)) => {
                    self.preserve(name, "unreadable")?;
                    notices.push(format!("{name} could not be read; the original was kept aside."));
                }
            }
        }
        Ok(State::default())
    }

    fn preserve(&self, name: &str, suffix: &str) -> io::Result<()> {
        for n in 0u32.. {
            let aside = self.root.join(format!("{name}.{suffix}.{n}"));
            match fs::hard_link(self.root.join(name), aside) {
                Ok(()) => return fs::remove_file(self.root.join(name)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    fn read_json<T: serde::de::DeserializeOwned>(&self, name: &str) -> io::Result<T> {
        serde_json::from_reader(File::open(self.root.join(name))?).map_err(io::Error::other)
    }

    pub fn save_state(&self, state: &State) -> io::Result<()> { self.save("state.json", state, true) }
    pub fn save_catalog(&self, listing: &Listing) -> io::Result<()> { self.save("catalog.json", listing, false) }
    pub fn checkpoint(&self, open: &OpenSession) -> io::Result<()> { self.save("session.json", open, false) }
    pub fn save_secrets(&self, secrets: &Secrets) -> io::Result<()> {
        if !self.writable { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "saved choices belong to a newer LumaGame")); }
        secrets.save(&self.root.join("secrets.json")).map_err(io::Error::other)
    }
    pub fn save_diagnostics(&self, config: &DiagnosticsConfig) -> io::Result<()> { self.save("diagnostics.json", config, false) }

    fn save<T: Serialize>(&self, name: &str, value: &T, backup: bool) -> io::Result<()> {
        if !self.writable { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "saved choices belong to a newer LumaGame")); }
        let temp = self.root.join(format!(".{name}.{}", std::process::id()));
        let result = (|| {
            let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
            serde_json::to_writer(&mut file, value).map_err(io::Error::other)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            if backup {
                match fs::rename(self.root.join(name), self.root.join(format!("{name}.bak"))) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
            fs::rename(&temp, self.root.join(name))?;
            File::open(&self.root)?.sync_all()
        })();
        if result.is_err() { let _ = fs::remove_file(temp); }
        result
    }

    pub fn clear_checkpoint(&self) -> io::Result<()> {
        if !self.writable { return Ok(()); }
        match fs::remove_file(self.root.join("session.json")) {
            Ok(()) => File::open(&self.root)?.sync_all(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// A directory only this user can open. Missing parents are made open to pass through: a data base such as
/// `/media/developer/apps/usr/palm/data` can be missing in the app's jail and is shared with every other app's directory.
pub fn private_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::DirBuilder::new().recursive(true).mode(0o755).create(parent)?;
    }
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        made => made,
    }
}

/// The same app-data bases as the feeder, before a per-user host path and the explicitly temporary last resort.
pub fn data_dir(wanted: &Path) -> io::Result<(PathBuf, Option<String>)> {
    let mut candidates = vec![wanted.to_owned()];
    for base in ["/media/developer/apps/usr/palm/data", "/media/cryptofs/apps/usr/palm/data", "/media/internal", "/var/luna/preferences", "/var/palm/data", "/media/developer/temp"] {
        candidates.push(Path::new(base).join(APP_ID));
    }
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        candidates.push(PathBuf::from(home).join("lumagame"));
    } else if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/share/lumagame"));
    }
    let temporary = std::env::temp_dir().join(format!("{APP_ID}-data"));
    candidates.push(temporary.clone());
    for candidate in candidates {
        if private_dir(&candidate).is_err() { continue; }
        let probe = candidate.join(format!(".write-test-{}", std::process::id()));
        if OpenOptions::new().write(true).create_new(true).mode(0o600).open(&probe).is_ok() {
            fs::remove_file(probe)?;
            let warning = (candidate == temporary).then(|| "Choices and sign-ins are stored temporarily and will be lost when the TV restarts.".into());
            return Ok((candidate.canonicalize()?, warning));
        }
    }
    Err(io::Error::new(io::ErrorKind::PermissionDenied, "no writable LumaGame data directory"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use lumagame_core::{history::Played, ids::GameId};

    fn temp() -> tempfile::TempDir {
        let base = std::env::var_os("CODY_DATA_SCRATCH").or_else(|| std::env::var_os("TMPDIR")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        tempfile::tempdir_in(base).unwrap()
    }

    #[test]
    fn corrupt_primary_recovers_backup_without_losing_either_original() {
        let dir = temp();
        let (store, mut snapshot) = Store::open(dir.path().into()).unwrap();
        snapshot.state.profile_mut().name = "Player one".into();
        store.save_state(&snapshot.state).unwrap();
        snapshot.state.profile_mut().name = "Player two".into();
        store.save_state(&snapshot.state).unwrap();
        fs::write(dir.path().join("state.json"), b"broken").unwrap();
        let (_, snapshot) = Store::open(dir.path().into()).unwrap();
        assert_eq!(snapshot.state.profile().name, "Player one");
        assert_eq!(fs::read_to_string(dir.path().join("state.json.unreadable.0")).unwrap(), "broken");
        assert!(dir.path().join("state.json.bak").exists());
    }

    #[test]
    fn newer_schema_never_falls_back_or_writes_over_owner_data() {
        let dir = temp();
        let original = r#"{"schema":900,"future":"keep"}"#;
        fs::write(dir.path().join("state.json"), original).unwrap();
        let (store, snapshot) = Store::open(dir.path().into()).unwrap();
        assert!(!store.writable());
        assert!(store.save_state(&snapshot.state).is_err());
        assert_eq!(fs::read_to_string(dir.path().join("state.json")).unwrap(), original);
    }

    #[test]
    fn power_cut_checkpoint_is_counted_once_and_secrets_are_private() {
        let dir = temp();
        let (store, snapshot) = Store::open(dir.path().into()).unwrap();
        let open = OpenSession { profile: snapshot.state.active.clone(), played: Played { game: GameId(1), source: "native|tv|game".parse().unwrap(), start: 123, seconds: 60 } };
        store.save_state(&snapshot.state).unwrap();
        store.checkpoint(&open).unwrap();
        let (store, snapshot) = Store::open(dir.path().into()).unwrap();
        assert_eq!(snapshot.state.profile().history.total(GameId(1)).seconds, 60);
        store.checkpoint(&open).unwrap();
        let (store, snapshot) = Store::open(dir.path().into()).unwrap();
        assert_eq!(snapshot.state.profile().history.total(GameId(1)).sessions, 1);
        assert!(!dir.path().join("session.json").exists());
        store.save_secrets(&Secrets { steamgriddb_key: "private".into(), ..Secrets::default() }).unwrap();
        assert_eq!(fs::metadata(dir.path().join("secrets.json")).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn diagnostics_defaults_off_and_token_stays_in_private_file() {
        let dir = temp();
        let (store, snapshot) = Store::open(dir.path().into()).unwrap();
        assert!(!snapshot.diagnostics.enabled);
        assert!(snapshot.diagnostics.server.is_empty() && snapshot.diagnostics.topic.is_empty() && snapshot.diagnostics.token.is_empty());
        assert_eq!(snapshot.diagnostics.level, crate::game::diagnostics::Level::Info);
        let config = DiagnosticsConfig { enabled: true, level: crate::game::diagnostics::Level::Debug, server: "https://example.invalid".into(), topic: "test".into(), token: "private-test".into() };
        store.save_diagnostics(&config).unwrap();
        store.save_state(&snapshot.state).unwrap();
        assert_eq!(fs::metadata(dir.path().join("diagnostics.json")).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!fs::read_to_string(dir.path().join("state.json")).unwrap().contains("private-test"));
        assert_eq!(Store::open(dir.path().into()).unwrap().1.diagnostics, config);
    }

    #[test]
    fn diagnostics_left_open_by_an_install_are_made_private_and_kept() {
        let dir = temp();
        let (store, _) = Store::open(dir.path().into()).unwrap();
        let config = DiagnosticsConfig { enabled: true, server: "https://example.invalid".into(), topic: "test".into(), token: "private-test".into(), ..Default::default() };
        store.save_diagnostics(&config).unwrap();
        let path = dir.path().join("diagnostics.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(Store::open(dir.path().into()).unwrap().1.diagnostics, config);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_missing_data_base_is_made_open_and_only_the_apps_directory_private() {
        let dir = temp();
        let base = dir.path().join("usr/palm/data");
        let (data, temporary) = data_dir(&base.join(APP_ID)).unwrap();
        assert!(temporary.is_none());
        assert_eq!(fs::metadata(&data).unwrap().permissions().mode() & 0o777, 0o700);
        assert_ne!(fs::metadata(&base).unwrap().permissions().mode() & 0o777, 0o700, "other apps keep their own directories here");
        private_dir(&data).unwrap();
    }
}
