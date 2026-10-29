use crate::Platform;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub struct System {
    pub luna_send: String,
    pub declutter: PathBuf,
}
impl System {
    fn json(&self, program: &Path, args: &[&str], seconds: u64) -> Result<Value, String> {
        let bytes = run(program, args, seconds)?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "the TV did not return JSON".to_owned())?;
        if value["returnValue"] == false || value["ok"] == false {
            return Err(value["errorText"]
                .as_str()
                .or_else(|| value["error"].as_str())
                .unwrap_or("the TV refused this change")
                .into());
        }
        Ok(value)
    }
}
impl Platform for System {
    fn luna(&self, method: &str, body: &Value) -> Result<Value, String> {
        let uri = match method {
            "getSystemSettings" => "luna://com.webos.settingsservice/getSystemSettings",
            "setSystemSettings" => "luna://com.webos.settingsservice/setSystemSettings",
            _ => return Err("privacy does not allow that LG method".into()),
        };
        self.json(
            Path::new(&self.luna_send),
            &["-n", "1", uri, &body.to_string()],
            4,
        )
    }
    fn declutter(&self, args: &[&str]) -> Result<Value, String> {
        if !matches!(
            args,
            ["status"]
                | ["persist", "on"]
                | [
                    "on" | "off",
                    "--group",
                    "ads" | "settings" | "uploaders" | "push"
                ]
        ) {
            return Err("not a privacy declutter operation".into());
        }
        self.json(
            Path::new("/bin/sh"),
            &std::iter::once(self.declutter.to_str().ok_or("invalid declutter path")?)
                .chain(args.iter().copied())
                .collect::<Vec<_>>(),
            90,
        )
    }
    fn bind(&self, source: &Path, target: &Path) -> Result<(), String> {
        run(
            Path::new("/bin/mount"),
            &[
                "--bind",
                source.to_str().ok_or("invalid source")?,
                target.to_str().ok_or("invalid target")?,
            ],
            4,
        )
        .map(drop)
    }
    fn unmount(&self, target: &Path) -> Result<(), String> {
        run(
            Path::new("/bin/umount"),
            &[target.to_str().ok_or("invalid target")?],
            4,
        )
        .map(drop)
    }
    fn close_app(&self, id: &str) -> Result<(), String> {
        // closeByAppId answers returnValue true for an app that is not running as well
        self.json(
            Path::new(&self.luna_send),
            &[
                "-n",
                "1",
                "luna://com.webos.applicationManager/closeByAppId",
                &serde_json::json!({ "id": id }).to_string(),
            ],
            4,
        )
        .map(drop)
    }
    fn stop_old_amazoff(&self) -> Result<Vec<String>, String> {
        crate::legacy::stop(Path::new("/proc"), std::process::id(), crate::legacy::NAME, Duration::from_secs(3))
    }
    fn cert_hash(&self, pem: &Path) -> Result<String, String> {
        let hash = run(
            Path::new("/usr/bin/openssl"),
            &[
                "x509",
                "-in",
                pem.to_str().ok_or("invalid CA path")?,
                "-hash",
                "-noout",
            ],
            4,
        )?;
        Ok(String::from_utf8(hash)
            .map_err(|_| "invalid certificate hash")?
            .trim()
            .into())
    }
}
fn run(program: &Path, args: &[&str], seconds: u64) -> Result<Vec<u8>, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    let stdout = child.stdout.take().ok_or("no command output")?;
    let reader = thread::spawn(move || {
        let mut out = Vec::new();
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut out)
            .map(|_| out)
    });
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Ok(s),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("privacy operation timed out".to_owned());
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e.to_string());
            }
        }
    }?;
    let bytes = reader
        .join()
        .map_err(|_| "privacy command output failed")?
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("{} failed ({status})", program.display()));
    }
    if bytes.len() > 1024 * 1024 {
        return Err("privacy command output exceeded 1 MB".into());
    }
    Ok(bytes)
}
