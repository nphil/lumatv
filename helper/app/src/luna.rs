//! `luna-send` as the helper's modules use it: one answer of a call, and subscriptions.

use crate::lsbridge::Sub;
use serde_json::Value;
use std::io::{self, BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// What a subscription's reader reads: the replies as `luna-send` prints them, one JSON text to a line (compact or spread over lines).
pub type FeedOut = Box<dyn Read + Send>;

/// One subscription: its replies and the means to end it. The helper's bus bridge (`lsbridge`) holds it when that is running and takes
/// it, and a `luna-send -n 1000000` of its own otherwise. The modules that read the replies themselves hold a `Feed`.
pub struct Feed {
    held: Held,
}

enum Held {
    Process(Child),
    Bridge(Sub),
}

impl Feed {
    /// Subscribes to `uri` with `payload`: through the bridge, or through `program` (a `luna-send`) when the bridge is not there for it.
    pub fn start(program: &str, uri: &str, payload: &str) -> io::Result<Self> {
        match crate::lsbridge::subscribe(program, uri, payload) {
            Some(sub) => Ok(Self { held: Held::Bridge(sub) }),
            None => Self::start_process(program, uri, payload, false),
        }
    }

    /// Starts `program` (a `luna-send`) on `uri` with `payload`, as a process of its own. `with_helper` ends the child with the helper: a
    /// child that waits for a reply that never comes would outlive a restart of it. It is the thread that starts the child, not the
    /// process, that the child follows, so only a caller that starts it from a thread that lives as long as the child may ask for it.
    pub fn start_process(program: &str, uri: &str, payload: &str, with_helper: bool) -> io::Result<Self> {
        let mut command = Command::new(program);
        command
            .args(["-n", "1000000", uri, payload])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if with_helper {
            // SAFETY: only `prctl` runs between fork and exec, and it is async-signal-safe.
            unsafe {
                command.pre_exec(|| {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        Ok(Self { held: Held::Process(command.spawn()?) })
    }

    /// The replies, once: a second call has nothing.
    pub fn output(&mut self) -> Option<FeedOut> {
        match &mut self.held {
            Held::Process(child) => child.stdout.take().map(|out| Box::new(out) as FeedOut),
            Held::Bridge(sub) => sub.output(),
        }
    }

    /// Whether the subscription ended on its own (the child exited, or the bridge ended it).
    pub fn has_ended(&mut self) -> bool {
        match &mut self.held {
            Held::Process(child) => child.try_wait().ok().flatten().is_some(),
            Held::Bridge(sub) => sub.has_ended(),
        }
    }

    /// Ends the subscription (its output ends with it) and waits for what ended it.
    pub fn end(&mut self) {
        match &mut self.held {
            Held::Process(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Held::Bridge(sub) => sub.end(),
        }
    }
}

/// One answer of a luna call (`luna-send -n 1`), or `None` when it fails or takes longer than 4 s.
pub fn once(luna: &str, uri: &str, payload: &str) -> Option<Value> {
    once_as(luna, None, uri, payload)
}

/// [`once`] as another caller (`luna-send -a`): the service a method answers to, as LG's own callers are known to it.
pub fn once_as(luna: &str, caller: Option<&str>, uri: &str, payload: &str) -> Option<Value> {
    let mut command = Command::new(luna);
    if let Some(caller) = caller {
        command.args(["-a", caller]);
    }
    let mut child = command
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
    feed: Feed,
    reader: Option<JoinHandle<()>>,
    alive: Arc<AtomicBool>,
}

pub type OnReply = Box<dyn Fn(&Value) + Send>;

impl Follow {
    /// `name` names the reader thread.
    pub fn start(name: &str, luna: &str, uri: &str, payload: &str, on_reply: OnReply) -> io::Result<Self> {
        let mut feed = Feed::start(luna, uri, payload)?;
        let Some(out) = feed.output() else {
            feed.end();
            return Err(io::Error::other("no subscription output"));
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
                feed,
                reader: Some(reader),
                alive,
            }),
            Err(e) => {
                feed.end();
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
        self.feed.end();
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
