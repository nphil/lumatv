//! The screen: is the panel lit?
//!
//! "Screen off = no work": the receiver is polled and the weather and news are fetched only while the screen is on, and a
//! standby TV costs nothing but this watcher. The answer is `tvpower`'s power state, read through `luna-send` (the helper is a
//! 64-bit static binary and cannot link the set's 32-bit luna library). `Active` and `Screen Saver` are on (the panel is lit);
//! `Active Standby`, `Screen Off`, `Suspend` and the rest are off.
//!
//! Two ways to read it, tried in this order:
//! 1. a subscription (`luna-send -i ... {"subscribe":true}`): the power service pushes every change, so nothing runs while the
//!    TV sleeps. It is used only once it has delivered a reading, and it is cross-checked by a one-shot read every minute; a
//!    subscription that disagrees with the check, or goes quiet before its first reading (a `luna-send` that buffers its
//!    output on a pipe would), is dropped for the poll below, and tried again after ten minutes.
//! 2. a poll: `luna-send -n 1 ... {}` every `screen.pollMs`, the call the mod's tool already makes.
//!
//! If neither produces a reading the screen is *assumed on* and the topic says so (`source: "assumed"`, with the error):
//! a broken detector must not switch off the receiver and the Home cards.
//!
//! The topic `screen`: `{"on": bool, "state": "Active", "since": <ms>, "source": "luna-subscribe" | "luna-poll" | "config" | "assumed", "error": null | "..."}`.

use crate::control::Stop;
use crate::ctx::Ctx;
use crate::hub::Hub;
use crate::timeutil::now_ms;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The hub topic.
pub const TOPIC: &str = "screen";
const POWER_STATE_URI: &str = "luna://com.webos.service.tvpower/power/getPowerState";
/// How long a fresh subscription may take to deliver its first reading.
const FIRST_READING: Duration = Duration::from_secs(4);
/// How often a working subscription is checked against a one-shot read.
const CROSS_CHECK: Duration = Duration::from_secs(60);
/// How long the poll is used after a subscription failed, before it is tried again.
const POLL_SPELL: Duration = Duration::from_secs(600);
/// Readings that fail in a row before the screen is assumed on.
const FAILS_BEFORE_ASSUMING: u32 = 3;
/// The longest a one-shot read may take.
const ONE_SHOT_DEADLINE: Duration = Duration::from_secs(4);

/// Whether the screen is on in what `tvpower` calls `state`.
pub fn state_is_on(state: &str) -> bool {
    matches!(state, "Active" | "Screen Saver")
}

/// The `state` of the first reply in `luna-send`'s output (one JSON object per reply, compact or pretty-printed).
pub fn parse_state(output: &str) -> Option<String> {
    serde_json::Deserializer::from_str(output)
        .into_iter::<Value>()
        .map_while(Result::ok)
        .find_map(|v| v.get("state").and_then(Value::as_str).map(str::to_owned))
}

/// Whether the hub says the screen is on. Unknown (nothing published yet) is off: the watcher publishes within a moment of
/// starting, and an `assumed` reading counts as on.
pub fn is_on(hub: &Hub) -> bool {
    hub.get(TOPIC)
        .and_then(|v| v.get("on").and_then(Value::as_bool))
        .unwrap_or(false)
}

/// Publishes readings, keeping `since` steady while the state does not change.
struct Publisher<'a> {
    hub: &'a Hub,
    notify: &'a (dyn Fn() + Send + Sync),
    last_state: Option<String>,
    since: u64,
}

impl Publisher<'_> {
    fn reading(&mut self, state: &str, source: &str) {
        if self.last_state.as_deref() != Some(state) {
            self.last_state = Some(state.to_owned());
            self.since = now_ms();
            crate::log_info!(
                "screen: {state} ({})",
                if state_is_on(state) { "on" } else { "off" }
            );
        }
        self.publish(state_is_on(state), state, source, None);
    }

    fn assumed(&mut self, error: &str) {
        if self.last_state.as_deref() != Some("assumed") {
            self.last_state = Some("assumed".to_owned());
            self.since = now_ms();
            crate::log_warn!(
                "screen: the power state cannot be read ({error}); assuming the screen is on"
            );
        }
        self.publish(true, "Active", "assumed", Some(error));
    }

    fn publish(&self, on: bool, state: &str, source: &str, error: Option<&str>) {
        let value = json!({"on": on, "state": state, "since": self.since, "source": source, "error": error});
        if self.hub.set(TOPIC, value) {
            (self.notify)();
        }
    }
}

/// Runs `luna-send -n 1` once and returns the power state it reports.
fn read_once(program: &str) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(["-n", "1", POWER_STATE_URI, "{}"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    let mut out = child.stdout.take().ok_or("no output pipe")?;
    // The reader thread ends when the child closes its output (exits or is killed below).
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("screen-read".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            let mut text = String::new();
            let _ = out.by_ref().take(64 * 1024).read_to_string(&mut text);
            let _ = tx.send(text);
        })
        .map_err(|e| format!("could not start a reader: {e}"))?;
    let text = match rx.recv_timeout(ONE_SHOT_DEADLINE) {
        Ok(text) => text,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("luna-send did not answer".to_owned());
        }
    };
    let _ = child.wait();
    parse_state(&text).ok_or_else(|| "luna-send gave no power state".to_owned())
}

/// The subscription child, which lives in a slot the stop hook can also reach, and the reader of its output.
struct Subscription {
    slot: Arc<Mutex<Option<Child>>>,
    rx: mpsc::Receiver<String>,
    reader: Option<JoinHandle<()>>,
}

impl Subscription {
    fn start(program: &str, slot: Arc<Mutex<Option<Child>>>) -> Result<Self, String> {
        let mut child = Command::new(program)
            .args(["-i", POWER_STATE_URI, r#"{"subscribe":true}"#])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not run {program}: {e}"))?;
        let Some(out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("no output pipe".to_owned());
        };
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("screen-sub".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                // Each reply is one line (compact output); a pretty-printed reply is joined until it parses.
                let mut pending = String::new();
                for line in BufReader::new(out.take(1024 * 1024))
                    .lines()
                    .map_while(Result::ok)
                {
                    pending.push_str(&line);
                    pending.push('\n');
                    if let Some(state) = parse_state(&pending) {
                        pending.clear();
                        if tx.send(state).is_err() {
                            return;
                        }
                    } else if pending.len() > 8192 {
                        pending.clear();
                    }
                }
            });
        let reader = match spawned {
            Ok(reader) => reader,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("could not start a reader: {e}"));
            }
        };
        *slot.lock() = Some(child);
        Ok(Self {
            slot,
            rx,
            reader: Some(reader),
        })
    }

    /// Ends the child (a reaped child is never signalled: the slot is emptied first).
    fn end(mut self) {
        end_child(&self.slot);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn end_child(slot: &Mutex<Option<Child>>) {
    if let Some(mut child) = slot.lock().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// How a spell of subscription ended.
enum Spell {
    Stopped,
    /// Failed or disagreed: poll for a while.
    Failed(String),
}

/// Starts the watcher. With `screen.source = "on"` the topic is set once and no thread runs. `notify` is called after every
/// change of the topic (the runners re-check the screen then).
pub fn spawn(
    ctx: &Ctx,
    notify: Arc<dyn Fn() + Send + Sync>,
) -> std::io::Result<Option<JoinHandle<()>>> {
    if ctx.config.screen.source == "on" {
        let value = json!({"on": true, "state": "Active", "since": now_ms(), "source": "config", "error": null});
        ctx.hub.set(TOPIC, value);
        notify();
        return Ok(None);
    }
    let ctx = ctx.clone();
    let handle = thread::Builder::new()
        .name("screen".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || watch(&ctx, notify.as_ref()))?;
    Ok(Some(handle))
}

fn watch(ctx: &Ctx, notify: &(dyn Fn() + Send + Sync)) {
    let program = crate::sys::which(&ctx.config.luna.send)
        .map_or_else(|| ctx.config.luna.send.clone(), |p| p.display().to_string());
    let poll = Duration::from_millis(ctx.config.screen.poll_ms);
    let mut publisher = Publisher {
        hub: &ctx.hub,
        notify,
        last_state: None,
        since: now_ms(),
    };
    let mut fails = 0u32;
    let mut subscribe_after = Instant::now();
    let stop = ctx.stop.clone();
    // The stop hook kills the subscription's child so its reader and the loop below end at once.
    let slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    stop.on_stop({
        let slot = slot.clone();
        move || end_child(&slot)
    });
    loop {
        if stop.is_set() {
            return;
        }
        if Instant::now() >= subscribe_after {
            match subscribe_spell(&program, &stop, &mut publisher, &slot) {
                Spell::Stopped => return,
                Spell::Failed(why) => {
                    crate::log_info!(
                        "screen: using the poll ({why}); the subscription is tried again in ten minutes"
                    );
                    subscribe_after = Instant::now() + POLL_SPELL;
                }
            }
            continue;
        }
        match read_once(&program) {
            Ok(state) => {
                fails = 0;
                publisher.reading(&state, "luna-poll");
            }
            Err(why) => {
                fails += 1;
                crate::log_debug!("screen: poll failed ({why})");
                if fails >= FAILS_BEFORE_ASSUMING {
                    publisher.assumed(&why);
                }
            }
        }
        if stop.sleep(poll) {
            return;
        }
    }
}

/// One spell of subscription. Returns when it must stop (`Stopped`) or when the poll should take over.
fn subscribe_spell(
    program: &str,
    stop: &Stop,
    publisher: &mut Publisher<'_>,
    slot: &Arc<Mutex<Option<Child>>>,
) -> Spell {
    let subscription = match Subscription::start(program, slot.clone()) {
        Ok(s) => s,
        Err(why) => return Spell::Failed(why),
    };
    // A stop that came while the child was being started has already run its hook against an empty slot.
    if stop.is_set() {
        subscription.end();
        return Spell::Stopped;
    }
    let result = (|| {
        let first = match subscription.rx.recv_timeout(FIRST_READING) {
            Ok(state) => state,
            Err(RecvTimeoutError::Timeout) => {
                return Spell::Failed("no reading from the subscription".to_owned());
            }
            Err(RecvTimeoutError::Disconnected) => {
                return if stop.is_set() {
                    Spell::Stopped
                } else {
                    Spell::Failed("the subscription ended".to_owned())
                };
            }
        };
        publisher.reading(&first, "luna-subscribe");
        let mut last = first;
        loop {
            match subscription.rx.recv_timeout(CROSS_CHECK) {
                Ok(state) => {
                    publisher.reading(&state, "luna-subscribe");
                    last = state;
                }
                Err(RecvTimeoutError::Timeout) => match read_once(program) {
                    Ok(state) if state == last => {}
                    Ok(state) => {
                        publisher.reading(&state, "luna-poll");
                        return Spell::Failed(format!(
                            "the subscription missed a change to {state}"
                        ));
                    }
                    Err(why) => crate::log_debug!("screen: cross-check failed ({why})"),
                },
                Err(RecvTimeoutError::Disconnected) => {
                    return if stop.is_set() {
                        Spell::Stopped
                    } else {
                        Spell::Failed("the subscription ended".to_owned())
                    };
                }
            }
            if stop.is_set() {
                return Spell::Stopped;
            }
        }
    })();
    subscription.end();
    if stop.is_set() {
        Spell::Stopped
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_lit_panel_is_on() {
        for on in ["Active", "Screen Saver"] {
            assert!(state_is_on(on), "{on}");
        }
        for off in [
            "Active Standby",
            "Screen Off",
            "Suspend",
            "Power Off",
            "Garbage",
            "",
        ] {
            assert!(!state_is_on(off), "{off}");
        }
    }

    #[test]
    fn the_state_is_found_in_compact_and_pretty_output() {
        assert_eq!(
            parse_state(r#"{"returnValue":true,"state":"Active","processing":"Active"}"#)
                .as_deref(),
            Some("Active")
        );
        assert_eq!(
            parse_state("{\n    \"returnValue\": true,\n    \"state\": \"Active Standby\"\n}\n")
                .as_deref(),
            Some("Active Standby")
        );
        // Two replies: the first one counts.
        assert_eq!(
            parse_state("{\"state\":\"Screen Saver\"}\n{\"state\":\"Active\"}\n").as_deref(),
            Some("Screen Saver")
        );
        assert_eq!(
            parse_state(r#"{"returnValue":false,"errorText":"Denied method call"}"#),
            None
        );
        assert_eq!(parse_state("Unable to connect to the bus"), None);
        assert_eq!(parse_state(""), None);
    }

    #[test]
    fn the_hub_decides_on_and_unknown_is_off() {
        let hub = Hub::new(1);
        assert!(!is_on(&hub));
        hub.set(TOPIC, json!({"on": true, "state": "Active"}));
        assert!(is_on(&hub));
        hub.set(TOPIC, json!({"on": false, "state": "Active Standby"}));
        assert!(!is_on(&hub));
    }
}
