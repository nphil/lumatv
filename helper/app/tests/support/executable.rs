//! Programs for a test to run: a script, or a copy of a binary, put on disk so that it runs at once. Included by the integration
//! tests (`support`) and, under `cfg(test)`, by the library's own tests (`lib.rs`), so every test writes programs the same way.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

/// Writes `body` to `path` as an executable (a script, so it starts with `#!`) that can be run at once, replacing what is there.
pub fn write_executable(path: &Path, body: &str) {
    install(path, r#"printf '%s' "$1" > "$0""#, body.as_ref());
}

/// Copies the program at `from` to `to` as an executable that can be run at once, replacing what is there.
pub fn copy_executable(from: &Path, to: &Path) {
    install(to, r#"cat -- "$1" > "$0""#, from.as_os_str());
}

/// A file cannot be run while a descriptor open for writing refers to it ("text file busy"). Writing it from a test thread leaves such a
/// descriptor in this process, and a test thread that starts a program meanwhile forks a child that holds a copy of every descriptor
/// this process has open until its exec: the copy outlives the close here, and the run fails at random, more often the more the tests
/// start programs. So the file is written by a short-lived `sh`, whose descriptor no fork of this process can hold, and moved onto
/// `path` by `rename`, which also works while the file it replaces is running and gives a new file, as a deploy does.
fn install(path: &Path, script: &str, arg: &OsStr) {
    let mut name = path.file_name().expect("a file name").to_owned();
    name.push(".staged");
    let staged = path.with_file_name(name);
    // `sh -c script $0 $1`: the staged path and the text or the source path
    let wrote = Command::new("/bin/sh")
        .args(["-c", script])
        .arg(&staged)
        .arg(arg)
        .stdin(Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("could not run /bin/sh to write {}: {e}", path.display()));
    assert!(wrote.success(), "writing {} failed: {wrote}", path.display());
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, path).unwrap();
}
