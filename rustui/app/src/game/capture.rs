//! The trigger for the diagnostics-only VRR/PTS timing capture (docs/research/decoded-vrr-4k.md).
//!
//! Saving Diagnostics that are on, valid and at Trace asks the two producers, the 32-bit feeder and the engine's streamer, for one 10 s capture
//! of the next stream: a small request file per producer in its own data directory (a producer reads only what it can already write). The
//! request names the output directory and this build. Each producer writes `<nonce>-<producer>.json` there once its window ends and does
//! nothing more for that nonce, so a capture happens once per save. Any other setting removes the requests.
use super::diagnostics::{Config, Level};
use super::store::private_dir;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub const ARM_FILE: &str = "vrr-capture.arm";
pub const OUTPUT_DIR: &str = "vrr-capture";

/// The feeder reads the first file, the streamer (through the engine's data directory) the second.
fn arm_paths(root: &Path) -> [PathBuf; 2] { [root.join(ARM_FILE), root.join("engines").join("gfn").join(ARM_FILE)] }

/// The installed package's `build-id` (game/build-ipk.sh), beside `bin64/` in the app's directory.
fn build_id() -> String {
    std::env::current_exe().ok()
        .and_then(|exe| exe.parent().and_then(Path::parent).map(|home| home.join("build-id")))
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|text| text.trim().to_owned())
        .filter(|id| !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "unknown".into())
}

fn write_private(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() { private_dir(dir)?; }
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() { let _ = fs::remove_file(&temp); }
    result
}

/// Writes fresh requests when `config` asks for a capture, removes them otherwise. `now_ms` makes the nonce.
fn apply(root: &Path, config: &Config, build: &str, now_ms: u128) -> io::Result<()> {
    let paths = arm_paths(root);
    if !(config.active() && config.level == Level::Trace) {
        for path in &paths {
            match fs::remove_file(path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        return Ok(());
    }
    let out = root.join(OUTPUT_DIR);
    private_dir(&out)?;
    let text = format!("vrr-capture 1\n{now_ms}\n{}\n{build}\n", out.display());
    for path in &paths { write_private(path, &text)?; }
    Ok(())
}

/// Called when Diagnostics are saved. A request that cannot be written only means no capture, so the save itself never fails on it.
pub fn sync(root: &Path, config: &Config) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(1);
    let _ = apply(root, config, &build_id(), now);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp() -> tempfile::TempDir {
        let base = std::env::var_os("CODY_DATA_SCRATCH").or_else(|| std::env::var_os("TMPDIR")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        tempfile::tempdir_in(base).unwrap()
    }
    fn config(level: Level) -> Config { Config { enabled: true, level, server: "https://example.invalid".into(), topic: "test".into(), token: String::new() } }
    fn mode(path: &Path) -> u32 { fs::metadata(path).unwrap().permissions().mode() & 0o777 }

    #[test]
    fn only_active_trace_diagnostics_ask_both_producers_for_a_capture_and_anything_else_withdraws_it() {
        let dir = temp();
        let root = dir.path();
        apply(root, &config(Level::Trace), "0123456789abcdef", 1700).unwrap();
        let [feeder, streamer] = arm_paths(root);
        for path in [&feeder, &streamer] {
            assert_eq!(fs::read_to_string(path).unwrap(), format!("vrr-capture 1\n1700\n{}/{OUTPUT_DIR}\n0123456789abcdef\n", root.display()));
            assert_eq!(mode(path), 0o600);
        }
        assert_eq!(mode(&root.join(OUTPUT_DIR)), 0o700);

        apply(root, &config(Level::Trace), "0123456789abcdef", 1701).unwrap();
        assert!(fs::read_to_string(&feeder).unwrap().starts_with("vrr-capture 1\n1701\n"), "a new save is a new request");

        for withdrawn in [config(Level::Debug), Config { enabled: false, ..config(Level::Trace) }, Config { server: "http://example.invalid".into(), ..config(Level::Trace) }] {
            apply(root, &config(Level::Trace), "0123456789abcdef", 1702).unwrap();
            apply(root, &withdrawn, "0123456789abcdef", 1703).unwrap();
            assert!(!feeder.exists() && !streamer.exists());
        }
        apply(root, &config(Level::Debug), "0123456789abcdef", 1704).unwrap();
    }

    #[test]
    fn an_unreadable_build_id_is_unknown_and_never_a_path_or_a_line_break() {
        assert_eq!(build_id(), "unknown", "the test binary has no build-id beside it");
    }
}
