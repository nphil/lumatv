//! `luna-send` as the helper's modules use it: one answer of a call, and a subscription whose replies go to a callback.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// One answer of a luna call (`luna-send -n 1`), or `None` when it fails or takes longer than 4 s.
pub fn once(luna: &str, uri: &str, payload: &str) -> Option<Value> {
    let mut child = Command::new(luna)
        .args(["-n", "1", uri, payload])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = out.by_ref().take(1024 * 1024).read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    let text = reader.join().ok()?;
    serde_json::Deserializer::from_str(&text)
        .into_iter::<Value>()
        .next()?
        .ok()
}

/// A luna subscription (`luna-send -n 1000000 ... {"subscribe":true}`): every reply goes to the callback, from a reader thread, until
/// the subscription ends or is dropped.
pub struct Follow {
    child: Child,
    reader: Option<JoinHandle<()>>,
    alive: Arc<AtomicBool>,
}

pub type OnReply = Box<dyn Fn(&Value) + Send>;

impl Follow {
    /// `name` names the reader thread.
    pub fn start(name: &str, luna: &str, uri: &str, payload: &str, on_reply: OnReply) -> std::io::Result<Self> {
        let mut child = Command::new(luna)
            .args(["-n", "1000000", uri, payload])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let Some(out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("no subscription output"));
        };
        let alive = Arc::new(AtomicBool::new(true));
        let flag = alive.clone();
        let reader = thread::Builder::new()
            .name(name.to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                read_replies(BufReader::new(out), &on_reply);
                flag.store(false, Ordering::Relaxed);
            });
        match reader {
            Ok(reader) => Ok(Self {
                child,
                reader: Some(reader),
                alive,
            }),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        }
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

impl Drop for Follow {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// The replies of a subscription, one JSON object each (compact, or spread over lines), until the output ends or stops being JSON.
fn read_replies(out: impl BufRead, on_reply: &dyn Fn(&Value)) {
    let mut pending = String::new();
    for line in out.lines() {
        let Ok(line) = line else { break };
        pending.push_str(&line);
        pending.push('\n');
        match serde_json::from_str::<Value>(&pending) {
            Ok(reply) => {
                pending.clear();
                if reply.get("returnValue").and_then(Value::as_bool) == Some(false) {
                    break;
                }
                on_reply(&reply);
            }
            Err(e) if e.is_eof() && pending.len() <= 64 * 1024 => {}
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    #[test]
    fn subscription_replies_reach_the_callback_compact_or_spread_until_one_fails() {
        let out = "{\"subscribed\":true,\"returnValue\":true,\"appId\":\"netflix\"}\n{\n  \"returnValue\": true,\n  \"appId\": \"com.webos.app.home\"\n}\n{\"returnValue\":false}\n{\"appId\":\"never\"}\n";
        let got = Mutex::new(Vec::new());
        read_replies(out.as_bytes(), &|reply: &Value| {
            got.lock()
                .push(reply["appId"].as_str().unwrap_or_default().to_owned())
        });
        assert_eq!(*got.lock(), vec!["netflix", "com.webos.app.home"]);
    }
}
