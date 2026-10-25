//! The receiver client: the Denon AVR's status as a hub topic, polled only while the screen is on.
//!
//! One thread. While the screen is off (or the receiver is switched off in the settings) it blocks on its command channel and
//! does nothing at all: no timer, no connection. While the screen is on it reads the Lite status every `receiver.pollMs`,
//! backing off when the receiver does not answer. A *nudge* (QML sends one when a volume key is pressed) starts a burst of fast
//! reads, so a change made by the receiver is on the topic within a fraction of a second instead of a poll period later.
//!
//! The topic `receiver`:
//! `{"enabled", "host", "polling", "reachable", "fresh", "updatedAt", "error", "power", "input", "volumeDb", "volumeAbs",
//! "volumeDisplay", "volumeText", "mute"}`. The six values are present only while `fresh`: a reading older than
//! `receiver.hideAfterMs` (6 s) is hidden, not shown stale, and so is everything when polling has stopped.
//! `reachable` is `true`/`false` for the last attempt and `null` before the first one or after polling stopped.

use crate::ctx::Ctx;
use crate::hub::Hub;
use crate::screen;
use crate::timeutil::now_ms;
use lumatv_denon::{Client, ClientError, Status, VolumeDisplay};
use serde_json::{Map, Value, json};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The hub topic.
pub const TOPIC: &str = "receiver";

/// What wakes the receiver thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// The screen state or the settings may have changed: look again.
    Wake,
    /// A volume key was pressed: read now and then fast for a short while.
    Nudge,
    Stop,
}

/// The level as the receiver's panel writes it: whole numbers without a decimal, otherwise one.
fn level_text(value: f64) -> String {
    if ((value * 10.0).round() as i64) % 10 == 0 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// `volumeText`: what the receiver's own display shows. Absolute mode shows 0 to 98, relative mode decibels.
fn volume_text(status: &Status) -> Option<String> {
    let db = status.volume_db?;
    Some(match status.volume_display {
        VolumeDisplay::Absolute => level_text(db + 80.0),
        VolumeDisplay::Relative | VolumeDisplay::Unknown => format!("{db:.1} dB"),
    })
}

struct Reading {
    status: Status,
    at: Instant,
    at_ms: u64,
}

struct Poller<'a> {
    ctx: &'a Ctx,
    last_ok: Option<Reading>,
    /// The last attempt: `None` before any, `Some(None)` it worked, `Some(Some(error))` it failed.
    last_attempt: Option<Option<ClientError>>,
    failures: u32,
}

impl Poller<'_> {
    /// The document for the topic, as of now.
    fn document(&self, enabled: bool, host: &str, polling: bool) -> Value {
        let hide = Duration::from_millis(self.ctx.config.receiver.hide_after_ms);
        let fresh = polling
            && self
                .last_ok
                .as_ref()
                .is_some_and(|r| r.at.elapsed() <= hide);
        let reachable = match (&self.last_attempt, polling) {
            (Some(attempt), true) => Value::Bool(attempt.is_none()),
            _ => Value::Null,
        };
        let error = match (&self.last_attempt, polling) {
            (Some(Some(e)), true) => json!({"code": e.code(), "message": e.to_string()}),
            _ => Value::Null,
        };
        let mut doc = Map::new();
        doc.insert("enabled".into(), json!(enabled));
        doc.insert("host".into(), json!(host));
        doc.insert("polling".into(), json!(polling));
        doc.insert("reachable".into(), reachable);
        doc.insert("fresh".into(), json!(fresh));
        doc.insert(
            "updatedAt".into(),
            json!(self.last_ok.as_ref().map(|r| r.at_ms)),
        );
        doc.insert("error".into(), error);
        let shown = self.last_ok.as_ref().filter(|_| fresh).map(|r| &r.status);
        doc.insert("power".into(), json!(shown.map(|s| s.power.word())));
        doc.insert("input".into(), json!(shown.and_then(|s| s.input.clone())));
        doc.insert("volumeDb".into(), json!(shown.and_then(|s| s.volume_db)));
        doc.insert(
            "volumeAbs".into(),
            json!(shown.and_then(Status::volume_absolute)),
        );
        doc.insert(
            "volumeDisplay".into(),
            json!(shown.map(|s| s.volume_display.word())),
        );
        doc.insert("volumeText".into(), json!(shown.and_then(volume_text)));
        doc.insert("mute".into(), json!(shown.and_then(|s| s.mute)));
        Value::Object(doc)
    }

    fn publish(&self, hub: &Hub, enabled: bool, host: &str, polling: bool) {
        // `updatedAt` moves on every read; only the other fields are news.
        hub.set_ignoring(TOPIC, self.document(enabled, host, polling), &["updatedAt"]);
    }

    /// One read. Logs only a change between working and not.
    fn poll(&mut self, client: &Client) {
        match client.status() {
            Ok(status) => {
                if self.failures > 0 || self.last_attempt.is_none() {
                    crate::log_info!("receiver: reading {}:{}", client.host(), client.port());
                }
                self.failures = 0;
                self.last_attempt = Some(None);
                self.last_ok = Some(Reading {
                    status,
                    at: Instant::now(),
                    at_ms: now_ms(),
                });
            }
            Err(e) => {
                if self.failures == 0 {
                    crate::log_warn!("receiver: {}:{}: {e}", client.host(), client.port());
                }
                self.failures = self.failures.saturating_add(1);
                self.last_attempt = Some(Some(e));
            }
        }
    }
}

/// Starts the receiver thread. Commands arrive on `rx`; the sender side is wired to the screen watcher, the API and the stop.
pub fn spawn(ctx: &Ctx, rx: Receiver<Cmd>) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("receiver".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || run(&ctx, &rx))
}

fn run(ctx: &Ctx, rx: &Receiver<Cmd>) {
    let tuning = ctx.config.receiver.clone();
    let mut poller = Poller {
        ctx,
        last_ok: None,
        last_attempt: None,
        failures: 0,
    };
    let mut burst_until: Option<Instant> = None;
    let mut next_poll = Instant::now();
    let mut client_for: Option<(String, u16)> = None;
    let mut client: Option<Client> = None;
    let mut was_polling = false;
    loop {
        if ctx.stop.is_set() {
            return;
        }
        let settings = ctx.settings.receiver();
        let polling = settings.enabled && screen::is_on(&ctx.hub);
        if !polling {
            if was_polling || poller.last_attempt.is_none() {
                // Stopped polling (or never started): the document says nothing is live.
                poller.publish(&ctx.hub, settings.enabled, &settings.host, false);
                if was_polling {
                    crate::log_debug!("receiver: polling stopped");
                }
            }
            was_polling = false;
            poller.last_attempt = None;
            poller.failures = 0;
            // Nothing to do until something changes: block, no timer.
            match rx.recv() {
                Ok(Cmd::Stop) | Err(_) => return,
                Ok(_) => continue,
            }
        }
        if !was_polling {
            was_polling = true;
            next_poll = Instant::now();
            crate::log_debug!("receiver: polling started");
        }
        let target = (settings.host.clone(), settings.port);
        if client_for.as_ref() != Some(&target) {
            client = Some(
                Client::new(settings.host.clone(), settings.port).with_timeouts(
                    Duration::from_millis(tuning.connect_timeout_ms),
                    Duration::from_millis(tuning.request_timeout_ms),
                ),
            );
            client_for = Some(target);
            poller.last_ok = None;
            poller.failures = 0;
            next_poll = Instant::now();
        }
        let now = Instant::now();
        if now >= next_poll
            && let Some(client) = &client
        {
            poller.poll(client);
            poller.publish(&ctx.hub, settings.enabled, &settings.host, true);
            let burst = burst_until.is_some_and(|b| Instant::now() < b);
            let base = Duration::from_millis(tuning.poll_ms);
            let wait = if burst {
                Duration::from_millis(tuning.fast_poll_ms)
            } else if poller.failures > 0 {
                let factor = 1u32 << poller.failures.min(10);
                (base * factor).min(Duration::from_millis(tuning.max_backoff_ms))
            } else {
                base
            };
            next_poll = Instant::now() + wait;
        }
        // Wake for the next poll, or earlier to hide a reading that has gone stale (a backed-off poll may be far away).
        let mut wake_at = next_poll;
        if let Some(reading) = &poller.last_ok {
            let hide_at =
                reading.at + Duration::from_millis(tuning.hide_after_ms) + Duration::from_millis(5);
            if hide_at > Instant::now() && hide_at < wake_at {
                wake_at = hide_at;
            }
        }
        match rx.recv_timeout(wake_at.saturating_duration_since(Instant::now())) {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Cmd::Nudge) => {
                burst_until = Some(Instant::now() + Duration::from_millis(tuning.burst_ms));
                next_poll = Instant::now();
            }
            Ok(Cmd::Wake) => {}
            Err(RecvTimeoutError::Timeout) => {
                // Publishing again lets a reading that passed its age turn `fresh: false` at once.
                poller.publish(&ctx.hub, settings.enabled, &settings.host, true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumatv_denon::Power;

    fn status(db: Option<f64>, display: VolumeDisplay) -> Status {
        Status {
            power: Power::On,
            input: Some("TV".into()),
            volume_display: display,
            volume_db: db,
            mute: Some(false),
        }
    }

    #[test]
    fn levels_read_like_the_panel() {
        assert_eq!(level_text(35.5), "35.5");
        assert_eq!(level_text(40.0), "40");
        assert_eq!(level_text(0.0), "0");
        assert_eq!(level_text(80.0), "80");
        assert_eq!(level_text(35.04), "35");
    }

    #[test]
    fn the_volume_text_follows_the_receivers_display_mode() {
        assert_eq!(
            volume_text(&status(Some(-44.5), VolumeDisplay::Absolute)).as_deref(),
            Some("35.5")
        );
        assert_eq!(
            volume_text(&status(Some(-44.5), VolumeDisplay::Relative)).as_deref(),
            Some("-44.5 dB")
        );
        assert_eq!(
            volume_text(&status(Some(-80.0), VolumeDisplay::Absolute)).as_deref(),
            Some("0")
        );
        assert_eq!(volume_text(&status(None, VolumeDisplay::Absolute)), None);
    }
}
