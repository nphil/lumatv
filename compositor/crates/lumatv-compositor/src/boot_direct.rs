//! Boot direct: LumaTV's Home from the first frame, with LG's Flutter Home out of the way (`docs/research/boot-direct.md`).
//!
//! `lumatv boot-direct on` writes `<data dir>/boot-direct`, a file whose first line is `on`. While it is there the tool keeps the Home stub
//! over LG's Home directory, so what SAM, bootd and the preload manager open as "Home" is a page that draws nothing and the native Home is
//! what is on the screen. The compositor's part is one thing: the Rust shell's client starts with the compositor instead of waiting for SAM's
//! Home to map (the stub's launch waits for WebAppMgr, which comes up with the compositor).
//!
//! There is no way to LG's Home from in here: the Home key is LumaTV's Home key, held or not. The way out is the supervisor's escape (Home held
//! 10 s, `compositor/sm/lumatv-sm.c`), which reads the remotes by itself and ends the compositor, the UI and everything else LumaTV started. A UI
//! client that exits or stops answering ends the compositor too (`shell_rust_failure`), and the supervisor's fallback follows.

use std::path::Path;

use serde_json::{Value, json};
use wayland_server::Resource;

use crate::State;
use crate::scene::HOME_APP;

/// The flag file's name in the data directory (`BD_FLAG` in `tools/lumatv`).
pub const FLAG_FILE: &str = "boot-direct";
/// What LG's Home is as a client: LG's Flutter runtime started for the Home app (`HOME_PKILL_PATTERN` in `tools/lumatv`; the command line
/// on the set is `/usr/bin/flutter-client -i com.webos.app.home {...}`). The stub is a page in WebAppMgr's process and has no such name, and
/// the screensaver is a Flutter client too, for another app.
const LG_HOME_CLIENT: &str = "flutter-client -i com.webos.app.home";

/// Whether the flag file in `data_dir` says `on` on its first line, which is all `bd_enabled` in `tools/lumatv` reads of it as well.
pub fn flag_on(data_dir: &Path) -> bool {
    std::fs::read_to_string(data_dir.join(FLAG_FILE)).is_ok_and(|text| text.lines().next().is_some_and(|line| line.trim() == "on"))
}

/// Whether the client with this command line is LG's Home rather than the stub.
pub fn is_lg_home_client(cmd: &str) -> bool {
    cmd.contains(LG_HOME_CLIENT)
}

impl State {
    /// `ctl shell`'s `bootDirect` part: the flag and what the front card is. The checks of a boot read it to see that LumaTV's Home is what
    /// is in front and that LG's is not.
    pub(crate) fn boot_direct_report(&self) -> Value {
        let front = self.scene.foreground(&self.surfaces).filter(|w| w.app_id() == HOME_APP);
        let lg_home = front.is_some_and(|w| {
            let client = self.seat.surfaces.get(&w.surface).and_then(|s| s.client());
            client.and_then(|c| self.clients.get(&c.id())).is_some_and(|c| is_lg_home_client(&c.cmd))
        });
        json!({
            "on": flag_on(&self.config.data_dir),
            "homeFront": front.is_some(),
            "lgHomeFront": lg_home,
            "uiGone": self.shell_rust.gave_up || !self.shell_rust.enabled(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp directory of its own per test.
    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lumatv-boot-direct-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_flag_is_on_only_for_on_on_the_first_line() {
        let dir = temp("flag");
        for (text, on) in [("on\n", true), ("on", true), (" on \n", true), ("on\nanything after\n", true), ("", false), ("off\n", false),
                           ("\non\n", false), ("tool /x\non\n", false), ("onn\n", false), ("yes\n", false)] {
            std::fs::write(dir.join(FLAG_FILE), text).unwrap();
            assert_eq!(flag_on(&dir), on, "{text:?}");
        }
        std::fs::remove_file(dir.join(FLAG_FILE)).unwrap();
        assert!(!flag_on(&dir), "no file");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_lgs_home_is_a_flutter_client() {
        assert!(is_lg_home_client("flutter-client -i com.webos.app.home"));
        assert!(is_lg_home_client(r#"/usr/bin/flutter-client -i com.webos.app.home {"@system_native_app":true,"event":"launch"}"#));
        assert!(!is_lg_home_client(r#"/usr/bin/flutter-client -i com.webos.app.screensaver {"parameters":{}}"#), "another Flutter app");
        assert!(!is_lg_home_client("/usr/sbin/webapp-mgr --webos-wayland"));
        assert!(!is_lg_home_client(""));
    }
}
