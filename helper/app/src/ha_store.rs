//! The saved way into Home Assistant: an address and an access token, in one private file.
//!
//! `{"url": "...", "token": "..."}`, mode 0600, in a directory of mode 0700. The two are saved together on purpose: the token is only
//! ever sent to the address it was saved with, and neither the settings API nor any other route can change either, so nothing that
//! can reach the loopback API can point a token at another machine. They are written by the command line (`voice-link`, `ha-link`),
//! which reads the token from standard input, and cleared by its `-unlink` or, for the plugin's link, by the person's "forget" in
//! Settings (which narrows what the TV may do); the owner of the TV runs those, nothing else in the helper saves a token.
//!
//! The token is never logged, never put in a topic or an API answer, and its `Debug` says nothing of it.
//!
//! Two links use this: voice search's (`voice_store`, `<data dir>/voice/ha.json`) and the Home Assistant plugin's lease
//! (`plugins::ha`, `<data dir>/ha/link.json`). They are two files, so one can be given a different token, or none, without the other.

use ha_link::endpoint::Endpoint;
use ha_link::link::Token;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

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

/// Checks and saves the link at `path`; the file is replaced whole. The address is kept as `display()` shows it (the scheme and
/// port made explicit), so what is read back dials the same place.
pub fn save(path: &Path, url: &str, token: &str) -> Result<HaLink, StoreError> {
    let link = parse(url, token)?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
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
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(StoreError(format!("{}: {e}", path.display())));
    }
    Ok(link)
}

/// The saved link: `Ok(None)` when there is none, an error (without the token) when the file is there and cannot be used.
pub fn load(path: &Path) -> Result<Option<HaLink>, StoreError> {
    let text = match std::fs::read_to_string(path) {
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
pub fn clear(path: &Path) -> bool {
    std::fs::remove_file(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.payload.signature-part";

    #[test]
    fn a_saved_link_is_private_reads_back_and_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ha").join("link.json");
        assert!(load(&path).unwrap().is_none());
        let saved = save(&path, "192.168.1.146", TOKEN).unwrap();
        assert_eq!(saved.endpoint.display(), "http://192.168.1.146:8123");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        let back = load(&path).unwrap().unwrap();
        assert_eq!(back.endpoint, saved.endpoint);
        assert_eq!(back.token.expose(), TOKEN);
        assert!(clear(&path));
        assert!(load(&path).unwrap().is_none());
        assert!(!clear(&path));
    }

    #[test]
    fn a_bad_address_or_token_saves_nothing_and_says_nothing_of_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link.json");
        for (url, token) in [
            ("", TOKEN),
            ("ftp://x", TOKEN),
            ("http://192.168.1.146", ""),
            ("http://192.168.1.146", "two words"),
            ("http://user:pw@192.168.1.146", TOKEN),
        ] {
            let e = save(&path, url, token).unwrap_err();
            assert!(!e.0.contains(TOKEN) && !e.0.contains("two words"), "{e}");
        }
        assert!(!path.exists());
    }

    #[test]
    fn a_damaged_file_is_an_error_that_does_not_quote_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link.json");
        save(&path, "http://192.168.1.146:8123", TOKEN).unwrap();
        std::fs::write(
            &path,
            format!("{{\"url\":\"ftp://nope\",\"token\":\"{TOKEN}\"}}"),
        )
        .unwrap();
        let e = load(&path).unwrap_err();
        assert!(!e.0.contains(TOKEN), "{e}");
        std::fs::write(&path, "not json").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn the_debug_form_hides_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let link = save(
            &dir.path().join("link.json"),
            "http://192.168.1.146:8123",
            TOKEN,
        )
        .unwrap();
        let shown = format!("{link:?}");
        assert!(
            !shown.contains("eyJ") && !shown.contains("signature"),
            "{shown}"
        );
    }
}
