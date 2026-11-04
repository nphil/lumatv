//! Input devices made after the compositor started, on the real `lumatv-compositor` binary (software renderer). lginput2 makes its
//! virtual devices (`Smart Remote RCU Input`, the network keys of Home Assistant, the phone apps and the perf gate's injector) anew at
//! every wake from standby, and the compositor read only what it found at its start: from the first wake on, no network key arrived, the
//! idle timer ran out, and the saver came up under a gate that thought it was pressing keys.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn ctl(dir: &Path, word: &str) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor")).args(["ctl", "--runtime-dir", dir.to_str().unwrap(), word]).output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("ctl {word}: {e}: {}", String::from_utf8_lossy(&out.stdout)))
}

/// Keys the compositor has routed, to a client or to nowhere.
fn routed(dir: &Path) -> u64 {
    let keys = ctl(dir, "keys");
    keys["delivered"].as_u64().unwrap() + keys["dropped"].as_u64().unwrap()
}

/// A FIFO standing in for the device node, with its writer end held (read and write, which never blocks): what is written waits in the
/// pipe for whoever opens the node, and a node that is read from after the compositor found it behaves as the device does.
fn make_node(path: &Path) -> std::fs::File {
    let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    // SAFETY: a path the test owns.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    std::fs::OpenOptions::new().read(true).write(true).open(path).unwrap()
}

fn press_enter(device: &mut std::fs::File) {
    let mut event = [0u8; 24];
    event[16..18].copy_from_slice(&1u16.to_ne_bytes()); // EV_KEY
    event[18..20].copy_from_slice(&28u16.to_ne_bytes()); // KEY_ENTER
    event[20..24].copy_from_slice(&1i32.to_ne_bytes()); // pressed
    device.write_all(&event).unwrap();
}

struct Compositor {
    child: Child,
    dir: std::path::PathBuf,
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Runs until `done`, asking at the pace `ctl` itself takes; false after five seconds.
fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
    }
    false
}

#[test]
fn keys_from_a_device_made_after_the_start_arrive_and_again_once_it_is_made_anew() {
    let dir = std::env::temp_dir().join(format!("lumatv-input-hotplug-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let input = dir.join("input");
    std::fs::create_dir_all(&input).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
        .args([
            "--runtime-dir",
            dir.to_str().unwrap(),
            "--renderer",
            "software",
            "--seconds",
            "60",
            "--input",
            "--no-im-queue",
            "--input-dir",
            input.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let c = Compositor { child, dir };
    assert!(wait_until(|| c.dir.join("lumatv-control").exists() && c.dir.join("wayland-lumatv").exists()), "the compositor is up");
    assert_eq!(routed(&c.dir), 0);

    // The device is made with the compositor already running, and a key waits on it.
    let node = input.join("event10");
    let mut device = make_node(&node);
    press_enter(&mut device);
    assert!(wait_until(|| routed(&c.dir) == 1), "the key of a device made after the start is read");

    // The wake: the device goes, and lginput2 makes it again at the same path.
    drop(device);
    std::fs::remove_file(&node).unwrap();
    let mut device = make_node(&node);
    press_enter(&mut device);
    assert!(wait_until(|| routed(&c.dir) == 2), "the key of a device made anew is read");

    let journal = ctl(&c.dir, "journal");
    let lines: Vec<&str> = journal["lines"].as_array().unwrap().iter().filter_map(|l| l.as_str()).collect();
    let node = node.to_str().unwrap();
    assert_eq!(lines.iter().filter(|l| l.starts_with(&format!("input: {node} "))).count(), 2, "each node found is in the journal: {lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("input: ") && l.contains("is gone")), "so is the device that went: {lines:?}");
}
