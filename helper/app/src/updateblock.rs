//! Whether LG's system updates are blocked: the `updateblock` topic, read only.
//!
//! The Homebrew Channel blocks LG's update servers with two things: a flag file, `/var/luna/preferences/webosbrew_block_updates`
//! (its own setting, `blockUpdates`, which Settings > System > Support > Update reads and sets through the channel's service), and
//! at every boot, while the flag exists, a bind-mounted `/etc/hosts` that maps `snu.lge.com`, `su.lge.com`, `su-ssl.lge.com` and
//! `su-dev.lge.com` to `127.0.0.1` and `::1` (its `startup.sh`). The two can disagree: the flag is read at boot, so a change of the
//! flag does nothing to the running `hosts` until the TV restarts, and another mod's bind-mounted `hosts` stacked on top of the
//! channel's once hid the block entirely (docs/HANDOFF.md, "The other mod on this set"). So the topic says what the files say now.
//!
//! Topic `updateblock`: `{flag, hosts: {<name>: bool}, live, blocking, state}`.
//! - `flag`: the channel's flag file exists (the setting is on);
//! - `hosts`: for each of the four names, whether `/etc/hosts` maps it to a loopback address (a commented line does not count);
//! - `live`: all four are mapped;
//! - `blocking`: the flag is on or any name is mapped: LG's update is, or will be after a restart, out of reach;
//! - `state`: `blocked` (flag and live), `armed` (flag on, hosts not mapping yet: applies at the next restart), `hosts-only` (flag off
//!   but a name is still mapped: until the next restart, or another tool's), `open` (neither).
//!
//! Nothing here writes: the setting is changed at the remote, in Settings, through the channel's own service. The files are read
//! every [`POLL`] and the topic is set only when it changes. `tools/lumatv` answers the same fields (`lumatv updateblock status`,
//! and `updateBlock` in `lumatv status`) with the same rules.

use crate::ctx::Ctx;
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const TOPIC: &str = "updateblock";
/// LG's update servers, as the Homebrew Channel's `startup.sh` maps them.
pub const NAMES: [&str; 4] = ["snu.lge.com", "su.lge.com", "su-ssl.lge.com", "su-dev.lge.com"];
const POLL: Duration = Duration::from_secs(10);
const MAX_HOSTS_BYTES: u64 = 256 * 1024;

/// The files read.
#[derive(Clone, Debug)]
pub struct Sources {
    pub flag: PathBuf,
    pub hosts: PathBuf,
}

impl Sources {
    /// The set's own.
    pub fn lg() -> Self {
        Self {
            flag: PathBuf::from("/var/luna/preferences/webosbrew_block_updates"),
            hosts: PathBuf::from("/etc/hosts"),
        }
    }
}

/// Which of [`NAMES`] a `hosts` file maps to a loopback address: `127.0.0.1` or `::1` first, the name among the rest of the line,
/// nothing after a `#`.
pub fn mapped(hosts: &str) -> [bool; 4] {
    let mut out = [false; 4];
    for line in hosts.lines() {
        let line = line.split('#').next().unwrap_or("");
        let mut words = line.split_whitespace();
        if !matches!(words.next(), Some("127.0.0.1" | "::1")) {
            continue;
        }
        for name in words {
            if let Some(i) = NAMES.iter().position(|n| n.eq_ignore_ascii_case(name)) {
                out[i] = true;
            }
        }
    }
    out
}

/// The topic's value for a flag and the names a `hosts` file maps.
pub fn value(flag: bool, hosts: [bool; 4]) -> Value {
    let count = hosts.iter().filter(|m| **m).count();
    let live = count == NAMES.len();
    let state = match (flag, live, count) {
        (true, true, _) => "blocked",
        (true, false, _) => "armed",
        (false, _, n) if n > 0 => "hosts-only",
        _ => "open",
    };
    let names: serde_json::Map<String, Value> = NAMES
        .iter()
        .zip(hosts)
        .map(|(n, m)| ((*n).to_owned(), Value::Bool(m)))
        .collect();
    json!({"flag": flag, "hosts": names, "live": live, "blocking": flag || count > 0, "state": state})
}

/// Reads the files now. An unreadable `hosts` counts as mapping nothing; a flag is only a file that exists.
pub fn read(sources: &Sources) -> Value {
    let mut text = String::new();
    if let Ok(f) = std::fs::File::open(&sources.hosts) {
        let _ = f.take(MAX_HOSTS_BYTES).read_to_string(&mut text);
    }
    value(sources.flag.exists(), mapped(&text))
}

/// Starts the watcher thread.
pub fn spawn(ctx: &Ctx) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("updateblock".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            let sources = Sources::lg();
            loop {
                ctx.hub.set(TOPIC, read(&sources));
                if ctx.stop.sleep(POLL) {
                    return;
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HBC: &str = "127.0.0.1 localhost\n\n# This file is dynamically regenerated on boot by webosbrew startup script\n\
                       127.0.0.1 snu.lge.com su-dev.lge.com su.lge.com su-ssl.lge.com\n::1 snu.lge.com su-dev.lge.com su.lge.com su-ssl.lge.com\n";

    #[test]
    fn the_channels_hosts_block_maps_all_four() {
        assert_eq!(mapped(HBC), [true; 4]);
    }

    #[test]
    fn a_comment_a_wrong_address_and_other_names_map_nothing() {
        let hosts = "# 127.0.0.1 su.lge.com\n192.168.1.5 su.lge.com\n127.0.0.1 notsu.lge.com snu.lge.com.example\n127.0.0.1 localhost # snu.lge.com\n";
        assert_eq!(mapped(hosts), [false; 4]);
        assert_eq!(mapped("::1 SU.LGE.COM\n"), [false, true, false, false]);
        assert_eq!(mapped(""), [false; 4]);
    }

    #[test]
    fn the_four_states() {
        assert_eq!(value(true, [true; 4])["state"], "blocked");
        assert_eq!(value(true, [false; 4])["state"], "armed");
        assert_eq!(value(true, [true, true, false, false])["state"], "armed");
        assert_eq!(value(false, [true; 4])["state"], "hosts-only");
        assert_eq!(value(false, [false, true, false, false])["state"], "hosts-only");
        assert_eq!(value(false, [false; 4])["state"], "open");
        assert_eq!(value(true, [true; 4])["live"], true);
        assert_eq!(value(true, [false; 4])["blocking"], true);
        assert_eq!(value(false, [false; 4])["blocking"], false);
        assert_eq!(value(false, [true; 4])["blocking"], true);
    }

    #[test]
    fn read_follows_the_files_and_a_missing_hosts_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let s = Sources { flag: dir.path().join("flag"), hosts: dir.path().join("hosts") };
        assert_eq!(read(&s)["state"], "open");
        std::fs::write(&s.flag, "").unwrap();
        assert_eq!(read(&s)["state"], "armed");
        std::fs::write(&s.hosts, HBC).unwrap();
        let v = read(&s);
        assert_eq!(v["state"], "blocked");
        assert_eq!(v["hosts"]["su-ssl.lge.com"], true);
        std::fs::remove_file(&s.flag).unwrap();
        assert_eq!(read(&s)["state"], "hosts-only");
    }
}
