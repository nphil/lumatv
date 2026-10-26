//! Where voice search keeps its way into Home Assistant: the address and the access token, in one private file.
//!
//! `<data dir>/voice/ha.json`, mode 0600, `{"url": "...", "token": "..."}`. The two are saved together on purpose: the token is only
//! ever sent to the address it was saved with, and neither the settings API nor any other route can change either, so nothing that
//! can reach the loopback API can point the microphone's audio and the token at another machine. They are written by the
//! command line (`lumatv-helper voice-link`, which reads the token from standard input) and cleared by `voice-unlink`; the owner
//! of the TV runs those, nothing in the helper does.
//!
//! The token is never logged, never put in a topic or an API answer, and its `Debug` says nothing of it.

pub use crate::ha_store::{HaLink, StoreError};
use std::path::{Path, PathBuf};

/// `<data dir>/voice/ha.json`.
pub fn file(data_dir: &Path) -> PathBuf {
    data_dir.join("voice").join("ha.json")
}

/// Checks and saves the link (see [`crate::ha_store::save`]).
pub fn save(data_dir: &Path, url: &str, token: &str) -> Result<HaLink, StoreError> {
    crate::ha_store::save(&file(data_dir), url, token)
}

/// The saved link: `Ok(None)` when there is none, an error (without the token) when the file is there and cannot be used.
pub fn load(data_dir: &Path) -> Result<Option<HaLink>, StoreError> {
    crate::ha_store::load(&file(data_dir))
}

/// Removes the saved link. True when there was one.
pub fn clear(data_dir: &Path) -> bool {
    crate::ha_store::clear(&file(data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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
