use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{fs::{self, OpenOptions}, io::{Read, Write}, path::Path};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// Deliberately not Debug: credentials never belong in logs or catalog/state files.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Secrets {
    pub steamgriddb_key: String,
    pub twitch_client_id: String,
    pub twitch_client_secret: String,
}
impl Secrets {
    pub fn load(path: &Path) -> Result<Self> {
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        if !meta.is_file() || meta.len() > 16_384 { return Err(Error::Invalid("unsafe secrets file")); }
        #[cfg(unix)]
        if meta.permissions().mode() & 0o077 != 0 { return Err(Error::Invalid("secrets file must be private (0600)")); }
        let mut bytes = Vec::new();
        fs::File::open(path)?.take(16_385).read_to_end(&mut bytes)?;
        if bytes.len() > 16_384 { return Err(Error::Invalid("secrets file too large")); }
        let secrets: Self = serde_json::from_slice(&bytes)?;
        secrets.validate()?;
        Ok(secrets)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path.parent().ok_or(Error::Invalid("secrets path needs a parent"))?;
        fs::create_dir_all(parent)?;
        let temporary = path.with_extension("json.new");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        let result = (|| {
            file.write_all(&serde_json::to_vec(self)?)?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() { let _ = fs::remove_file(&temporary); }
        result
    }

    fn validate(&self) -> Result<()> {
        for value in [&self.steamgriddb_key, &self.twitch_client_id, &self.twitch_client_secret] {
            if value.len() > 4096 || value.chars().any(char::is_control) {
                return Err(Error::Invalid("invalid provider credentials"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_credentials_round_trip_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        assert!(Secrets::load(&path).unwrap().steamgriddb_key.is_empty());
        let secrets = Secrets { steamgriddb_key: "fixture-key".into(), twitch_client_id: "fixture-client".into(), twitch_client_secret: "fixture-secret".into() };
        secrets.save(&path).unwrap();
        assert_eq!(Secrets::load(&path).unwrap().steamgriddb_key, "fixture-key");
        #[cfg(unix)]
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        Secrets::default().save(&path).unwrap();
        assert!(Secrets::load(&path).unwrap().twitch_client_secret.is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn public_files_and_symlinks_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        fs::write(&path, b"{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Secrets::load(&path).is_err());
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(Secrets::load(&link).is_err());
    }
}
