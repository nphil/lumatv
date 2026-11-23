//! The transcoder's child program, `lumatv-venc xcode`: built into the helper (`build.rs`), and written where the engine starts it from.
//!
//! The helper carries the program so that it and the engine always come from one commit and an install of the helper is an install of
//! both. It is written to the set's RAM disk, in a directory only this user can enter, when the transcoder is switched on: nothing of it
//! stays on the set's storage, and a restart of the set loses nothing but a copy the next switch-on writes again.

use sha2::{Digest, Sha256};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The program's image; empty in a build for another machine or made without `venc/build.sh`.
pub static IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lumatv-venc"));

/// Where the program is kept while the transcoder is on.
pub const DIR: &str = "/tmp/lumatv-xcode";
const NAME: &str = "lumatv-venc";

/// The first eight hex digits of the image's SHA-256, which the log and the status name it by.
pub fn id(image: &[u8]) -> String {
    Sha256::digest(image).iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// Writes `image` as `dir/lumatv-venc`, executable by this user alone, unless the file there is already exactly it, and answers its path.
/// `dir` is made if it is not there, and refused when it is not this user's own, closed directory (a link, or another user's, would let
/// another user's file be run in its place).
pub fn install(dir: &Path, image: &[u8]) -> io::Result<PathBuf> {
    if image.is_empty() {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "this build of the helper has no transcoder program in it"));
    }
    match DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let own = fs::symlink_metadata(dir)?;
    // SAFETY: geteuid has no arguments and cannot fail.
    if !own.is_dir() || own.uid() != unsafe { libc::geteuid() } || own.mode() & 0o077 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is not this user's own closed directory", dir.display())));
    }
    let path = dir.join(NAME);
    let current = fs::symlink_metadata(&path).ok().is_some_and(|m| m.is_file() && m.permissions().mode() & 0o700 == 0o700);
    if current && fs::read(&path).is_ok_and(|have| have == image) {
        return Ok(path);
    }
    // written beside it and renamed over it, so that a copy a running child executes is never opened for writing
    let fresh = dir.join(format!("{NAME}.new"));
    let _ = fs::remove_file(&fresh);
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o700).open(&fresh)?;
    file.write_all(image)?;
    drop(file);
    fs::rename(&fresh, &path)?;
    Ok(path)
}

/// Removes the program and its directory (the transcoder is switched off). A directory that is not there is not an error.
pub fn remove(dir: &Path) {
    let _ = fs::remove_file(dir.join(NAME));
    let _ = fs::remove_file(dir.join(format!("{NAME}.new")));
    let _ = fs::remove_dir(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_is_written_once_closed_to_other_users_and_written_again_when_it_differs() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("xcode");
        let path = install(&dir, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let at = fs::metadata(&path).unwrap().ino();
        assert_eq!(install(&dir, b"first").unwrap(), path);
        assert_eq!(fs::metadata(&path).unwrap().ino(), at, "the same file is kept");
        install(&dir, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        remove(&dir);
        assert!(!dir.exists());
    }

    #[test]
    fn an_empty_image_a_linked_directory_and_an_open_one_are_refused() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("xcode");
        assert_eq!(install(&dir, b"").unwrap_err().kind(), io::ErrorKind::Unsupported);
        assert!(!dir.exists(), "nothing is made for a build with no program");
        let real = base.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&real, &dir).unwrap();
        assert_eq!(install(&dir, b"x").unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        let open = base.path().join("open");
        fs::create_dir(&open).unwrap();
        fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(install(&open, b"x").unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn the_image_is_named_by_the_start_of_its_hash() {
        assert_eq!(id(b"abc"), "ba7816bf");
    }
}
