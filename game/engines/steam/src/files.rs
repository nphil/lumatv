//! The small files the engine keeps. Each is written whole and replaced in one step, is readable by the engine's user only, and is read
//! with a cap: the cache directory may be emptied at any time, and another program may have left anything in either directory.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

/// Creates the directory and its missing parents for the engine's user only. A directory that exists is left as it is.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

/// Writes `name` in `dir` so that a reader sees the old file or the whole new one, never half of one.
pub fn write(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    private_dir(dir)?;
    let temp = dir.join(format!(".{name}.tmp"));
    // A leftover would keep whatever mode it was made with; the new file is created 0600.
    let _ = fs::remove_file(&temp);
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, dir.join(name))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// The file's content, or `None` when it is missing, unreadable or longer than `max` bytes.
pub fn read(dir: &Path, name: &str, max: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(dir.join(name)).ok()?.take(max + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

/// Deletes `name` in `dir`. A file that is not there is already what was asked for.
pub fn remove(dir: &Path, name: &str) -> io::Result<()> {
    match fs::remove_file(dir.join(name)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::TempDir;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_written_file_is_private_whole_and_leaves_nothing_behind() {
        let temp = TempDir::new();
        let dir = temp.path().join("a/b");
        write(&dir, "token.json", b"one").unwrap();
        write(&dir, "token.json", b"two").unwrap();
        assert_eq!(read(&dir, "token.json", 16).as_deref(), Some(&b"two"[..]));
        assert_eq!(mode(&dir.join("token.json")), 0o600);
        assert_eq!((mode(&dir), mode(dir.parent().unwrap())), (0o700, 0o700));
        let names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["token.json"]);
    }

    #[test]
    fn an_existing_directory_keeps_its_mode_and_a_stale_temp_file_is_replaced() {
        let temp = TempDir::new();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o750)).unwrap();
        fs::write(temp.path().join(".x.json.tmp"), b"stale").unwrap();
        fs::set_permissions(temp.path().join(".x.json.tmp"), fs::Permissions::from_mode(0o666)).unwrap();
        write(temp.path(), "x.json", b"new").unwrap();
        assert_eq!((mode(temp.path()), mode(&temp.path().join("x.json"))), (0o750, 0o600));
        assert!(!temp.path().join(".x.json.tmp").exists());
    }

    #[test]
    fn reading_is_capped_and_a_missing_file_is_none() {
        let temp = TempDir::new();
        write(temp.path(), "f", b"12345").unwrap();
        assert!(read(temp.path(), "f", 5).is_some());
        assert!(read(temp.path(), "f", 4).is_none());
        assert!(read(temp.path(), "nothing", 4).is_none());
    }

    #[test]
    fn removing_a_missing_file_is_fine() {
        let temp = TempDir::new();
        write(temp.path(), "f", b"1").unwrap();
        remove(temp.path(), "f").unwrap();
        remove(temp.path(), "f").unwrap();
        assert!(read(temp.path(), "f", 4).is_none());
    }
}
