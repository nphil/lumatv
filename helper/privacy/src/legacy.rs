//! What an earlier AmazOff install leaves behind, retired by Amazoff's own switch so the owner never has to do it by hand:
//! its background programs (an nginx and the monitor that restarts it hold port 443), its start-up script, and the TLS-check
//! override it wrote into Prime's configuration.
use crate::{Paths, atomic, read_json};
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

pub const INIT_DIR: &str = "/var/lib/webosbrew/init.d";
pub const PRIME_CONFIG: &str =
    "/media/cryptofs/apps/usr/palm/applications/amazon/default_config.json";

/// What names the old app on a command line, a path or a script.
pub const NAME: &str = "amazoff";

fn mentions(bytes: &[u8], name: &str) -> bool {
    String::from_utf8_lossy(bytes).to_ascii_lowercase().contains(name)
}

/// The programs of the old app under `proc` (a `/proc`): those whose command line, program or working folder names it.
pub fn running(proc: &Path, own: u32, name: &str) -> Vec<(u32, String)> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(proc) else {
        return found;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        if pid <= 1 || pid == own {
            continue;
        }
        let dir = entry.path();
        let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
        let link = |name: &str| {
            std::fs::read_link(dir.join(name))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        if mentions(&cmdline, name) || mentions(link("exe").as_bytes(), name) || mentions(link("cwd").as_bytes(), name) {
            let argv0 = cmdline.split(|b| *b == 0).next().unwrap_or_default();
            let program = String::from_utf8_lossy(argv0);
            let program = program.rsplit('/').next().unwrap_or_default();
            found.push((pid, if program.is_empty() { format!("pid {pid}") } else { program.to_owned() }));
        }
    }
    found
}

fn gone(proc: &Path, pid: u32) -> bool {
    match std::fs::read_to_string(proc.join(pid.to_string()).join("stat")) {
        Err(_) => true,
        // a zombie holds no port; its parent has not collected it yet
        Ok(stat) => stat.rsplit(')').next().is_some_and(|rest| rest.trim_start().starts_with('Z')),
    }
}

fn signal(pid: u32, sig: i32) {
    // SAFETY: kill(2) with a pid found a moment ago; at worst the pid is gone.
    unsafe { libc::kill(pid as i32, sig) };
}

/// Stops every program of the old app and returns their names. The monitor goes with the rest and the folder is read again, so a
/// program it started in between is stopped as well; an error only when something survives SIGKILL.
pub fn stop(proc: &Path, own: u32, name: &str, grace: Duration) -> Result<Vec<String>, String> {
    let mut stopped: Vec<String> = Vec::new();
    for _ in 0..4 {
        let list = running(proc, own, name);
        let live: Vec<_> = list.into_iter().filter(|(pid, _)| !gone(proc, *pid)).collect();
        if live.is_empty() {
            return Ok(stopped);
        }
        for (pid, program) in &live {
            signal(*pid, libc::SIGTERM);
            if !stopped.contains(program) {
                stopped.push(program.clone());
            }
        }
        let until = Instant::now() + grace;
        while Instant::now() < until && live.iter().any(|(pid, _)| !gone(proc, *pid)) {
            std::thread::sleep(Duration::from_millis(20));
        }
        for (pid, _) in live.iter().filter(|(pid, _)| !gone(proc, *pid)) {
            signal(*pid, libc::SIGKILL);
        }
        let until = Instant::now() + grace;
        while Instant::now() < until && live.iter().any(|(pid, _)| !gone(proc, *pid)) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    if running(proc, own, name).iter().any(|(pid, _)| !gone(proc, *pid)) {
        return Err("the old AmazOff app would not stop".into());
    }
    Ok(stopped)
}

/// Moves the old app's start-up scripts out of the Homebrew Channel's start-up folder into LumaTV's own state (nothing is deleted),
/// so it does not come back at the next boot. Returns the names moved.
pub fn retire_autostart(paths: &Paths) -> Result<Vec<String>, String> {
    let dir = paths.system(INIT_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut moved = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !path.is_file() {
            continue;
        }
        let small = entry.metadata().is_ok_and(|m| m.len() <= 65536);
        let hit = mentions(name.as_bytes(), NAME) || (small && std::fs::read(&path).is_ok_and(|b| mentions(&b, NAME)));
        if !hit {
            continue;
        }
        let keep = paths.state("retired");
        std::fs::create_dir_all(&keep).map_err(|e| e.to_string())?;
        std::fs::rename(&path, keep.join(&name)).or_else(|_| {
            std::fs::copy(&path, keep.join(&name)).and_then(|_| std::fs::remove_file(&path))
        })
        .map_err(|e| e.to_string())?;
        moved.push(name);
    }
    Ok(moved)
}

/// Removes the one key the old app added to Prime's configuration, which switched certificate checks off; every other key stays.
/// Prime is closed when this runs. Returns whether the key was there.
pub fn clear_tls_override(paths: &Paths) -> Result<bool, String> {
    let file = paths.system(PRIME_CONFIG);
    if !file.is_file() {
        return Ok(false);
    }
    let mut config = read_json(&file)?;
    let on = |v: &Value| *v == "1" || *v == true || *v == 1;
    let Some(map) = config.as_object_mut().filter(|m| m.get("disable-ssl-cert").is_some_and(on)) else {
        return Ok(false);
    };
    map.remove("disable-ssl-cert");
    let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    atomic(&file, &bytes, false)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    #[test]
    fn the_old_apps_programs_are_stopped_the_monitor_included_and_nothing_else() {
        let tag = format!("amazoff-test-{}", std::process::id());
        let spawn = |arg0: &str| Command::new("sleep").arg0(arg0).arg("60").spawn().unwrap();
        let mut monitor = spawn(&format!("{tag}-monitor"));
        let mut nginx = spawn(&format!("/opt/{tag}/nginx"));
        let mut other = spawn("some-other-sleep");
        // the children are reaped by this test, so a killed one shows as a zombie until then: `gone` counts that
        let stopped = stop(Path::new("/proc"), std::process::id(), &tag, Duration::from_secs(2)).unwrap();
        assert!(stopped.contains(&format!("{tag}-monitor")) && stopped.contains(&"nginx".to_string()), "{stopped:?}");
        assert!(monitor.wait().unwrap().code().is_none(), "ended by a signal");
        assert!(nginx.wait().unwrap().code().is_none());
        assert!(other.try_wait().unwrap().is_none(), "an unrelated program keeps running");
        other.kill().unwrap();
        other.wait().unwrap();
        assert!(stop(Path::new("/proc"), std::process::id(), &tag, Duration::from_secs(1)).unwrap().is_empty());
    }
}
