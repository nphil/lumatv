//! The receiver client: the Denon AVR's status as a hub topic, read only while the screen is on.
//!
//! One thread. While the screen is off (or the receiver is switched off in the settings) it blocks on its command channel and
//! does nothing at all: no timer, no connection. While the screen is on it points the telnet link (`receiver_telnet.rs`) at the
//! receiver: that one held connection delivers every volume, mute, input and power change the moment the receiver makes it, and
//! its periodic questions keep the reading's stamp moving. Only while the link is down does the thread fall back to reading the
//! Lite status over HTTP every `receiver.pollMs`, backing off when the receiver does not answer; a *nudge* (QML sends one when a
//! volume key is pressed) starts a burst of fast reads during the fallback. The telnet link cannot say which scale the panel
//! shows, so one HTTP read learns that when the link comes up.
//!
//! The topic `receiver`:
//! `{"enabled", "host", "polling", "reachable", "fresh", "updatedAt", "error", "power", "input", "volumeDb", "volumeAbs",
//! "volumeDisplay", "volumeText", "mute"}`. The six values are present only while `fresh`: a reading older than
//! `receiver.hideAfterMs` (6 s) is hidden, not shown stale, and so is everything when polling has stopped.
//! `reachable` is `true`/`false` for the last attempt and `null` before the first one or after polling stopped.

use crate::ctx::Ctx;
use crate::hub::Hub;
use crate::receiver_telnet::{Event, Link, QUERY_GAP, Target, Tuning};
use crate::screen;
use crate::timeutil::now_ms;
use lumatv_denon::{Client, ClientError, Status, VolumeDisplay};
use serde_json::{Map, Value, json};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The hub topic.
pub const TOPIC: &str = "receiver";

/// What wakes the receiver thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// The screen state or the settings may have changed: look again.
    Wake,
    /// A volume key was pressed: read now and then fast for a short while (only matters while the HTTP fallback is reading).
    Nudge,
    /// The telnet link queued an event.
    Link,
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
    /// The scale the receiver's panel shows, as last read over HTTP.
    display: VolumeDisplay,
    /// The sound mode and the input signal's words as the telnet link last heard them, raw (`DOLBY AUDIO-DSUR`, `PCM ZERO`).
    mode: Option<String>,
    signal: Option<String>,
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
        // what the receiver says it plays, raw; the screen turns them into words (kit/AudioFormat.js). Like the rest, only while the reading is fresh.
        doc.insert("soundMode".into(), json!(self.mode.as_deref().filter(|_| fresh)));
        doc.insert("signal".into(), json!(self.signal.as_deref().filter(|_| fresh)));
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
                self.learn_display(&status);
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

    /// Remembers which scale the panel shows, the one thing the telnet link cannot hear.
    fn learn_display(&mut self, status: &Status) {
        if status.volume_display != VolumeDisplay::Unknown {
            self.display = status.volume_display.clone();
        }
    }

    /// Takes a reading from the telnet link in: it counts as a successful attempt and moves the stamp, whether or not the
    /// receiver said anything new.
    fn accept(&mut self, mut status: Status, at: Instant, at_ms: u64, mode: Option<String>, signal: Option<String>) {
        status.volume_display = self.display.clone();
        self.failures = 0;
        self.last_attempt = Some(None);
        self.last_ok = Some(Reading { status, at, at_ms });
        self.mode = mode;
        self.signal = signal;
    }
}

/// Where the telnet link stands, as far as this thread has heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkState {
    /// Pointed at the receiver, nothing heard yet: the HTTP fallback waits, the link reports a failure within its timeouts.
    Connecting,
    /// A complete reading has arrived: the receiver pushes, nothing is polled.
    Up,
    /// The last attempt failed: HTTP reads cover until the link is back.
    Down,
}

/// The sooner of an optional deadline and another.
fn soonest(a: Option<Instant>, b: Instant) -> Option<Instant> {
    Some(a.map_or(b, |a| a.min(b)))
}

/// Starts the telnet link, or nothing when `receiver.telnetPort` is 0 (or the thread cannot start: HTTP then does all of it).
fn start_link(ctx: &Ctx, tx: Sender<Cmd>) -> Option<Link> {
    let tuning = &ctx.config.receiver;
    if tuning.telnet_port == 0 {
        return None;
    }
    let ms = Duration::from_millis;
    let link_tuning = Tuning {
        connect_timeout: ms(tuning.connect_timeout_ms),
        heartbeat: ms(tuning.poll_ms),
        silence: ms(tuning.poll_ms + tuning.request_timeout_ms),
        ready_within: ms(tuning.request_timeout_ms) + QUERY_GAP * 4,
        backoff_base: ms(tuning.poll_ms),
        backoff_max: ms(tuning.max_backoff_ms),
    };
    match Link::spawn(link_tuning, move || {
        let _ = tx.send(Cmd::Link);
    }) {
        Ok(link) => Some(link),
        Err(e) => {
            crate::log_warn!("receiver: telnet link not started: {e}");
            None
        }
    }
}

/// Starts the receiver thread. Commands arrive on `rx`; the sender side is wired to the screen watcher, the API and the stop.
/// `tx` is the same channel's sender, for the telnet link to wake the thread with.
pub fn spawn(ctx: &Ctx, tx: Sender<Cmd>, rx: Receiver<Cmd>) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("receiver".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || run(&ctx, tx, &rx))
}

fn run(ctx: &Ctx, tx: Sender<Cmd>, rx: &Receiver<Cmd>) {
    let tuning = ctx.config.receiver.clone();
    let link = start_link(ctx, tx);
    let mut poller = Poller {
        ctx,
        last_ok: None,
        last_attempt: None,
        failures: 0,
        display: VolumeDisplay::Unknown,
        mode: None,
        signal: None,
    };
    let mut burst_until: Option<Instant> = None;
    let mut next_poll = Instant::now();
    let mut client_for: Option<(String, u16)> = None;
    let mut client: Option<Client> = None;
    let mut was_polling = false;
    let mut link_target: Option<Target> = None;
    let mut link_state = LinkState::Connecting;
    // When one HTTP read should (again) learn the panel's scale: when the link comes up, and until it is known.
    let mut probe_at: Option<Instant> = None;
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
            // No connection while the screen is off.
            if let Some(link) = &link {
                link.set_target(None);
            }
            link_target = None;
            link_state = LinkState::Connecting;
            probe_at = None;
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
            let moved = client_for.replace(target).is_some();
            poller.last_ok = None;
            poller.last_attempt = None;
            poller.failures = 0;
            poller.display = VolumeDisplay::Unknown;
            next_poll = Instant::now();
            if link_state == LinkState::Up {
                // The telnet link may be unaffected (only the web port changed): learn the scale from the new address.
                probe_at = Some(Instant::now());
            }
            if moved {
                // Whatever was read from the old address no longer counts.
                poller.publish(&ctx.hub, settings.enabled, &settings.host, true);
            }
        }
        if let Some(link) = &link {
            let want = Target {
                host: settings.host.clone(),
                port: tuning.telnet_port,
            };
            if link_target.as_ref() != Some(&want) {
                link.set_target(Some(want.clone()));
                link_target = Some(want);
                link_state = LinkState::Connecting;
                probe_at = None;
            }
        }
        // HTTP reads are the fallback: only without a link, or while it is down.
        let http_active = link.is_none() || link_state == LinkState::Down;
        let now = Instant::now();
        if http_active
            && now >= next_poll
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
        if link_state == LinkState::Up
            && probe_at.is_some_and(|due| Instant::now() >= due)
            && let Some(client) = &client
        {
            // Failing to learn the scale costs nothing but its name in the text; try again later while it is unknown.
            if let Ok(status) = client.status() {
                poller.learn_display(&status);
            }
            let display = poller.display.clone();
            if let Some(reading) = &mut poller.last_ok {
                reading.status.volume_display = display;
            }
            probe_at = (poller.display == VolumeDisplay::Unknown)
                .then(|| Instant::now() + Duration::from_millis(tuning.max_backoff_ms));
            poller.publish(&ctx.hub, settings.enabled, &settings.host, true);
        }
        // Wake for the next HTTP read or probe, or earlier to hide a reading that has gone stale (a backed-off read may be far
        // away, and with the link up there is none).
        let mut wake_at = None;
        if http_active {
            wake_at = soonest(wake_at, next_poll);
        }
        if link_state == LinkState::Up
            && let Some(due) = probe_at
        {
            wake_at = soonest(wake_at, due);
        }
        if let Some(reading) = &poller.last_ok {
            let hide_at =
                reading.at + Duration::from_millis(tuning.hide_after_ms) + Duration::from_millis(5);
            if hide_at > Instant::now() {
                wake_at = soonest(wake_at, hide_at);
            }
        }
        let message = match wake_at {
            Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match message {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Cmd::Nudge) => {
                burst_until = Some(Instant::now() + Duration::from_millis(tuning.burst_ms));
                next_poll = Instant::now();
            }
            Ok(Cmd::Wake) => {}
            Ok(Cmd::Link) => {
                for event in link.iter().flat_map(Link::drain) {
                    match event {
                        Event::Reading { status, at, at_ms, mode, signal } => {
                            if link_state != LinkState::Up {
                                link_state = LinkState::Up;
                                probe_at = Some(Instant::now());
                                crate::log_info!(
                                    "receiver: telnet {}:{}",
                                    settings.host,
                                    tuning.telnet_port
                                );
                            }
                            poller.accept(status, at, at_ms, mode, signal);
                        }
                        Event::Down(e) => {
                            if link_state != LinkState::Down {
                                crate::log_warn!(
                                    "receiver: telnet {}:{}: {e}; reading over HTTP",
                                    settings.host,
                                    tuning.telnet_port
                                );
                            }
                            link_state = LinkState::Down;
                            probe_at = None;
                            next_poll = Instant::now();
                        }
                    }
                }
                poller.publish(&ctx.hub, settings.enabled, &settings.host, true);
            }
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
