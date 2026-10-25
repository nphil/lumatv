//! Where voice search keeps its way into Home Assistant: the address and the access token, in one private file.
//!
//! `<data dir>/voice/ha.json`, mode 0600, `{"url": "...", "token": "..."}`. The two are saved together on purpose: the token is only
//! ever sent to the address it was saved with, and neither the settings API nor any other route can change either, so nothing that
//! can reach the loopback API can point the microphone's audio and the token at another machine. They are written by the
//! command line (`lumatv-helper voice-link`, which reads the token from standard input) and cleared by `voice-unlink`; the owner
//! of the TV runs those, nothing in the helper does.
//!
//! The token is never logged, never put in a topic or an API answer, and its `Debug` says nothing of it.

use ha_link::endpoint::Endpoint;
use ha_link::link::Token;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// `<data dir>/voice/ha.json`.
pub fn file(data_dir: &Path) -> PathBuf {
    data_dir.join("voice").join("ha.json")
}

/// Where Home Assistant is and the token to speak to it with.
#[derive(Clone)]
pub struct HaLink {
    pub endpoint: Endpoint,
    pub token: Token,
}

impl fmt::Debug for HaLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HaLink({}, token <redacted>)", self.endpoint.display())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    url: String,
    token: String,
}

/// Why the link could not be saved or read, in words that never carry the token.
#[derive(Debug, PartialEq, Eq)]
pub struct StoreError(pub String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

fn parse(url: &str, token: &str) -> Result<HaLink, StoreError> {
    let endpoint = Endpoint::parse(url).map_err(|e| StoreError(e.to_string()))?;
    let token = Token::new(token).map_err(|e| StoreError(e.to_string()))?;
    Ok(HaLink { endpoint, token })
}

/// Checks and saves the link; the file is replaced whole. The address is kept as `display()` shows it (the scheme and port made
/// explicit), so what is read back dials the same place.
pub fn save(data_dir: &Path, url: &str, token: &str) -> Result<HaLink, StoreError> {
    let link = parse(url, token)?;
    let path = file(data_dir);
    let dir = path.parent().unwrap_or(data_dir);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| StoreError(format!("{}: {e}", dir.display())))?;
    // A directory made earlier by something else may be wider than this one wants.
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    let stored = Stored {
        url: link.endpoint.display(),
        token: link.token.expose().to_owned(),
    };
    let bytes = serde_json::to_vec(&stored).map_err(|e| StoreError(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &path)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(StoreError(format!("{}: {e}", path.display())));
    }
    Ok(link)
}

/// The saved link: `Ok(None)` when there is none, an error (without the token) when the file is there and cannot be used.
pub fn load(data_dir: &Path) -> Result<Option<HaLink>, StoreError> {
    let path = file(data_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StoreError(format!("{}: {e}", path.display()))),
    };
    let stored: Stored = serde_json::from_str(&text)
        .map_err(|_| StoreError(format!("{} is not a saved link", path.display())))?;
    parse(&stored.url, &stored.token)
        .map(Some)
        .map_err(|e| StoreError(format!("{}: {e}", path.display())))
}

/// Removes the saved link. True when there was one.
pub fn clear(data_dir: &Path) -> bool {
    std::fs::remove_file(file(data_dir)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.payload.signature-part";

    #[test]
    fn a_saved_link_is_private_reads_back_and_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).unwrap().is_none());
        let saved = save(dir.path(), "192.168.1.146", TOKEN).unwrap();
        assert_eq!(saved.endpoint.display(), "http://192.168.1.146:8123");
        let path = file(dir.path());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let back = load(dir.path()).unwrap().unwrap();
        assert_eq!(back.endpoint, saved.endpoint);
        assert_eq!(back.token.expose(), TOKEN);
        assert!(clear(dir.path()));
        assert!(load(dir.path()).unwrap().is_none());
        assert!(!clear(dir.path()));
    }

    #[test]
    fn a_bad_address_or_token_saves_nothing_and_says_nothing_of_the_token() {
        let dir = tempfile::tempdir().unwrap();
        for (url, token) in [
            ("", TOKEN),
            ("ftp://x", TOKEN),
            ("http://192.168.1.146", ""),
            ("http://192.168.1.146", "two words"),
            ("http://user:pw@192.168.1.146", TOKEN),
        ] {
            let e = save(dir.path(), url, token).unwrap_err();
            assert!(!e.0.contains(TOKEN) && !e.0.contains("two words"), "{e}");
        }
        assert!(!file(dir.path()).exists());
    }

    #[test]
    fn a_damaged_file_is_an_error_that_does_not_quote_it() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), "http://192.168.1.146:8123", TOKEN).unwrap();
        std::fs::write(
            file(dir.path()),
            format!("{{\"url\":\"ftp://nope\",\"token\":\"{TOKEN}\"}}"),
        )
        .unwrap();
        let e = load(dir.path()).unwrap_err();
        assert!(!e.0.contains(TOKEN), "{e}");
        std::fs::write(file(dir.path()), "not json").unwrap();
        assert!(load(dir.path()).is_err());
    }

    #[test]
    fn the_debug_form_hides_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let link = save(dir.path(), "http://192.168.1.146:8123", TOKEN).unwrap();
        let shown = format!("{link:?}");
        assert!(
            !shown.contains("eyJ") && !shown.contains("signature"),
            "{shown}"
        );
    }
}
