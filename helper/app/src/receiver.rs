//! The receiver client: the Denon AVR's status as a hub topic, read only while the screen is on.
//!
//! One thread. While the screen is off (or the Denon receiver plugin is switched off, `receiver_plugin`) it blocks on its command channel and
//! does nothing at all: no timer, no connection. While the screen is on it points the telnet link (`receiver_telnet.rs`) at the
//! receiver: that one held connection delivers every volume, mute, input and power change the moment the receiver makes it, and
//! its periodic questions keep the reading's stamp moving. Only while the link is down does the thread fall back to reading the
//! Lite status over HTTP every `receiver.pollMs`, backing off when the receiver does not answer; a *nudge* (QML sends one when a
//! volume key is pressed) starts a burst of fast reads during the fallback. The telnet link cannot say which scale the panel
//! shows, so one HTTP read learns that when the link comes up.
//!
//! The topic `receiver`:
//! `{"enabled", "host", "polling", "reachable", "fresh", "updatedAt", "error", "power", "input", "volumeDb", "volumeAbs",
//! "volumeDisplay", "volumeText", "mute", "soundMode", "signal", "volumeMax"}`. The values are present only while `fresh`: a reading older than
//! `receiver.hideAfterMs` (6 s) is hidden, not shown stale, and so is everything when polling has stopped. `volumeMax` is the
//! receiver's own volume limit on the 0 to 98 scale, once the telnet link has heard it.
//! `reachable` is `true`/`false` for the last attempt and `null` before the first one or after polling stopped.
//!
//! The level the volume display shows goes the other way: a `Cmd::Volume` makes `receiver_volume.rs` send it to the receiver.

use crate::ctx::Ctx;
use crate::hub::Hub;
use crate::plugins::session::Shared;
use crate::receiver_settings::SettingsState;
use crate::receiver_telnet::{Event, Link, QUERY_GAP, Target, Tuning};
use crate::receiver_volume::VolumeDriver;
use crate::screen;
use crate::timeutil::now_ms;
use lumatv_denon::{Client, ClientError, Power, Status, VolumeDisplay};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The hub topic.
pub const TOPIC: &str = "receiver";

/// What wakes the receiver thread.
#[derive(Debug)]
pub enum Cmd {
    /// The screen state or the settings may have changed: look again.
    Wake,
    /// A volume key was pressed: read now and then fast for a short while (only matters while the HTTP fallback is reading).
    Nudge,
    /// The telnet link queued an event.
    Link,
    /// The volume display's level, in the receiver's half steps (0 to 196): send it to the receiver. Dropped while the screen is off.
    Volume(u16),
    SettingsInterest,
    SettingsSet {
        setting: String,
        value: Value,
        session: String,
        reply: Sender<SettingsResponse>,
    },
    Stop,
}

#[derive(Debug)]
pub enum SettingsResponse {
    Accepted { request_id: String, session: String },
    Invalid(String),
    Busy,
    Unavailable(String),
    WorkerError,
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
    settings: SettingsState,
    last_ok: Option<Reading>,
    /// The last attempt: `None` before any, `Some(None)` it worked, `Some(Some(error))` it failed.
    last_attempt: Option<Option<ClientError>>,
    failures: u32,
    /// The scale the receiver's panel shows, as last read over HTTP.
    display: VolumeDisplay,
    /// The sound mode and the input signal's words as the telnet link last heard them, raw (`DOLBY AUDIO-DSUR`, `PCM ZERO`).
    mode: Option<String>,
    signal: Option<String>,
    /// The receiver's volume limit on the panel's 0 to 98 scale, as the telnet link last heard it.
    max_volume: Option<f64>,
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
        doc.insert(
            "soundMode".into(),
            json!(self.mode.as_deref().filter(|_| fresh)),
        );
        doc.insert(
            "signal".into(),
            json!(self.signal.as_deref().filter(|_| fresh)),
        );
        doc.insert("volumeMax".into(), json!(self.max_volume.filter(|_| fresh)));
        doc.insert("settings".into(), self.settings.document());
        doc.insert("settingsSession".into(), json!(self.settings.token()));
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

    /// The receiver's level on the panel's 0 to 98 scale, from the newest reading.
    fn level(&self) -> Option<f64> {
        self.last_ok
            .as_ref()
            .and_then(|r| r.status.volume_absolute())
    }

    /// Takes a reading from the telnet link in: it counts as a successful attempt and moves the stamp, whether or not the
    /// receiver said anything new.
    fn accept(
        &mut self,
        mut status: Status,
        at: Instant,
        at_ms: u64,
        mode: Option<String>,
        signal: Option<String>,
        max_volume: Option<f64>,
    ) {
        status.volume_display = self.display.clone();
        self.failures = 0;
        self.last_attempt = Some(None);
        self.last_ok = Some(Reading { status, at, at_ms });
        self.mode = mode;
        self.signal = signal;
        // heard with the volume answers; a reading without it keeps the last one
        self.max_volume = max_volume.or(self.max_volume);
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
// Only tvpower readings are evidence for power commands, not assumed/configured state.
fn actual_screen_value(v: &Value) -> Option<bool> {
    if !matches!(v.get("source")?.as_str()?, "luna-subscribe" | "luna-poll") {
        return None;
    }
    let on = match v.get("state")?.as_str()? {
        "Active" | "Screen Saver" => true,
        "Active Standby" => false,
        _ => return None,
    };
    (v.get("on")?.as_bool()? == on).then_some(on)
}

fn actual_screen(hub: &Hub) -> Option<(bool, u64)> {
    let (seq, value) = hub.topic_at(screen::TOPIC);
    Some((actual_screen_value(&value?)?, seq))
}

static POWER_TOPICS: std::sync::LazyLock<[String; 1]> =
    std::sync::LazyLock::new(|| [screen::TOPIC.to_owned()]);

// Consume each real power edge once, including a standby/wake during an HTTP wait.
fn new_transition(
    last: &mut Option<(bool, u64)>,
    current: Option<(bool, u64)>,
    hub: &Hub,
) -> Option<(bool, u64)> {
    let current = current?;
    let previous = last.map(|value| value.0);
    let changed = previous != Some(current.0)
        || (current.0
            && last.is_some_and(|prior| {
                if prior.1 == current.1 {
                    return false;
                }
                let changes = hub.events_after(Some(prior.1), None, Some(&*POWER_TOPICS));
                !changes.reset
                    && changes
                        .events
                        .iter()
                        .any(|event| actual_screen_value(&event.data) == Some(false))
            }));
    let act = changed && (current.0 || previous == Some(true));
    *last = Some(current);
    act.then_some(current)
}

fn same_transition(ctx: &Ctx, transition: (bool, u64), host: &str, port: u16) -> bool {
    let settings = ctx.settings.receiver();
    if ctx.stop.is_set() || !settings.enabled || settings.host != host || settings.port != port {
        return false;
    }
    let changes = ctx
        .hub
        .events_after(Some(transition.1), None, Some(&*POWER_TOPICS));
    // Active and Screen Saver share an ON epoch. History also catches a fast
    // standby/wake or unknown state that a blocking HTTP call could otherwise miss.
    !changes.reset
        && changes
            .events
            .iter()
            .all(|event| actual_screen_value(&event.data) == Some(transition.0))
        && actual_screen(&ctx.hub).is_some_and(|current| current.0 == transition.0)
}

fn restore_power_on(
    ctx: &Ctx,
    client: &Client,
    since: Instant,
    wake_on: &mut Option<(Client, Instant)>,
    authorized: &dyn Fn() -> bool,
) -> bool {
    if !authorized() {
        return false;
    }
    // Telnet volume/input messages refresh the whole cached reading, not its
    // power evidence. Only an actual HTTP ON response can authorize ARC.
    let fresh_for = Duration::from_millis(
        ctx.config
            .receiver
            .poll_ms
            .min(ctx.config.receiver.request_timeout_ms),
    );
    let witnessed = wake_on.take().is_some_and(|(witness, at)| {
        witness.host() == client.host()
            && witness.port() == client.port()
            && at >= since
            && at.elapsed() <= fresh_for
    });
    (witnessed
        || client
            .status()
            .is_ok_and(|status| status.power == Power::On))
        && authorized()
}

struct Fallback {
    desired: &'static str,
    generation: u64,
    since: Instant,
    host: String,
    port: u16,
    retry_at: Instant,
}

// CEC remains primary for standby. A still-on receiver gets one backup command.
fn sync_power(
    ctx: &Ctx,
    transition: (bool, u64),
    client: &Client,
    authorized: &dyn Fn() -> bool,
) -> Option<bool> {
    let current = || same_transition(ctx, transition, client.host(), client.port()) && authorized();
    if !current() {
        return None;
    }
    let status = client.status().ok().map(|s| s.power);
    let command = match (transition.0, status) {
        (true, Some(Power::On)) | (false, Some(Power::Standby)) => {
            return current().then_some(true);
        }
        (true, _) => "ZMON",
        (false, _) => "ZMOFF",
    };
    if !current() {
        return None;
    }
    let sent = client.send_command(command).is_ok();
    if !transition.0 {
        return current().then_some(sent);
    }
    // A lost command reply does not prove the AVR stayed off. Confirm before
    // replacing ARC, including when the HTTP command timed out or failed.
    let end = Instant::now() + Duration::from_millis(ctx.config.receiver.request_timeout_ms);
    let mut probe = client.clone();
    while current() {
        let remaining = end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        probe = probe.with_timeouts(
            Duration::from_millis(ctx.config.receiver.connect_timeout_ms).min(remaining),
            remaining,
        );
        if probe.status().is_ok_and(|status| status.power == Power::On) {
            return current().then_some(true);
        }
        let remaining = end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        thread::sleep(Duration::from_millis(150).min(remaining));
    }
    current().then_some(false)
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
/// `tx` is the same channel's sender, for the telnet link to wake the thread with. Guarded as the Denon receiver plugin's: after a panic
/// it starts again with a new client and volume driver.
pub fn spawn(
    ctx: &Ctx,
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
    rx: Receiver<Cmd>,
) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    crate::plugins::guard::spawn(
        "receiver",
        256 * 1024,
        shared,
        &[crate::receiver_plugin::ID],
        ctx.stop.clone(),
        move |_| run(&ctx, tx.clone(), &rx),
    )
}

#[allow(clippy::too_many_arguments)] // the loop's own state, lent to the one place that drains the link's events
fn drain_link_events(
    link: Option<&Link>,
    poller: &mut Poller<'_>,
    target: Option<&Target>,
    state: &mut LinkState,
    probe_at: &mut Option<Instant>,
    next_poll: &mut Instant,
    host: &str,
    port: u16,
) -> bool {
    let mut changed = false;
    for event in link.into_iter().flat_map(Link::drain) {
        changed = true;
        match event {
            Event::Reading {
                status,
                at,
                at_ms,
                mode,
                signal,
                max_volume,
            } => {
                if *state != LinkState::Up {
                    *state = LinkState::Up;
                    *probe_at = Some(Instant::now());
                    crate::log_info!("receiver: telnet {}:{}", host, port);
                }
                poller.accept(status, at, at_ms, mode, signal, max_volume);
            }
            Event::SettingsConnected {
                session,
                context_revision,
                target: event_target,
            } => {
                if target == Some(&event_target) {
                    poller.settings.connected(session);
                    poller.settings.observe_context(session, context_revision);
                }
            }
            Event::SettingsContextChanged {
                session,
                target: event_target,
                context_revision,
                field,
            } => {
                if target == Some(&event_target) {
                    let field = match field {
                        crate::receiver_telnet::ContextField::Power => "power",
                        crate::receiver_telnet::ContextField::Input => "input",
                        crate::receiver_telnet::ContextField::Mode => "soundMode",
                        crate::receiver_telnet::ContextField::Signal => "signal",
                    };
                    poller
                        .settings
                        .context_changed(session, context_revision, field);
                }
            }
            Event::SettingsReply {
                session,
                target: event_target,
                context_revision,
                reply,
                at,
                at_ms,
                ..
            } => {
                if target == Some(&event_target) {
                    poller.settings.observe_context(session, context_revision);
                    poller.settings.observe(session, reply, at, at_ms);
                }
            }
            Event::SettingsMode {
                session,
                target: event_target,
                context_revision,
                mode,
                at,
                at_ms,
            } => {
                if target == Some(&event_target) {
                    poller.settings.observe_context(session, context_revision);
                    poller.settings.mode(session, mode, at, at_ms);
                }
            }
            Event::SettingsSignal {
                session,
                target: event_target,
                context_revision,
                signal,
                at,
                ..
            } => {
                if target == Some(&event_target) {
                    poller.settings.observe_context(session, context_revision);
                    poller.settings.signal(session, signal, at);
                }
            }
            Event::SettingsResult {
                request_id,
                status,
                error,
                ..
            } => {
                let status = match status {
                    crate::receiver_telnet::SettingsStatus::Confirmed => "applied",
                    crate::receiver_telnet::SettingsStatus::Failed => "failed",
                };
                poller.settings.finish(&request_id, status, error);
            }
            Event::Down(e) => {
                poller.settings.invalidate("Receiver connection was lost");
                if *state != LinkState::Down {
                    crate::log_warn!("receiver: telnet {}:{}: {e}; reading over HTTP", host, port);
                }
                *state = LinkState::Down;
                *probe_at = None;
                *next_poll = Instant::now();
            }
        }
    }
    changed
}

fn run(ctx: &Ctx, tx: Sender<Cmd>, rx: &Receiver<Cmd>) {
    let tuning = ctx.config.receiver.clone();
    let link = start_link(ctx, tx.clone());
    let mut poller = Poller {
        ctx,
        settings: SettingsState::new(ctx.hub.boot()),
        last_ok: None,
        last_attempt: None,
        failures: 0,
        display: VolumeDisplay::Unknown,
        mode: None,
        signal: None,
        max_volume: None,
    };
    let driver = match VolumeDriver::spawn(ctx) {
        Ok(driver) => Some(driver),
        Err(e) => {
            crate::log_warn!("receiver: volume driver not started: {e}");
            None
        }
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
    let mut last_actual: Option<(bool, u64)> = None;
    let mut output_sub: Option<crate::receiver_sync::Subscription> = None;
    let mut next_subscription = Instant::now();
    let mut fallback: Option<Fallback> = None;
    let mut reconcile = true;
    let mut reconcile_since = Instant::now();
    let mut reconcile_at = Instant::now();
    let mut wake_on: Option<(Client, Instant)> = None;
    loop {
        if ctx.stop.is_set() {
            return;
        }
        let settings = ctx.settings.receiver();
        let actual = actual_screen(&ctx.hub);
        if output_sub.as_ref().is_some_and(|s| !s.state().valid) {
            output_sub = None;
            next_subscription = Instant::now();
        }
        if !settings.enabled || settings.host.is_empty() {
            output_sub = None;
            fallback = None;
            reconcile = false;
        } else if output_sub.is_none()
            && actual.is_some_and(|s| s.0)
            && Instant::now() >= next_subscription
        {
            output_sub =
                crate::receiver_sync::Subscription::start(&ctx.config.luna.send, tx.clone()).ok();
            let until = Instant::now() + Duration::from_secs(4);
            while output_sub.as_ref().is_some_and(|s| !s.state().valid)
                && Instant::now() < until
                && actual.is_some_and(|t| same_transition(ctx, t, &settings.host, settings.port))
            {
                if drain_link_events(
                    link.as_ref(),
                    &mut poller,
                    link_target.as_ref(),
                    &mut link_state,
                    &mut probe_at,
                    &mut next_poll,
                    &settings.host,
                    tuning.telnet_port,
                ) {
                    poller.publish(&ctx.hub, settings.enabled, &settings.host, true);
                }
                thread::sleep(Duration::from_millis(20));
            }
            if let Some(state) = output_sub.as_ref().map(|s| s.state()).filter(|s| s.valid) {
                // A fresh subscribed snapshot is authoritative after disconnect;
                // no consumed wake power attempt is replayed.
                if let Some(owned) = &mut fallback {
                    if state.preference == Some(owned.desired) {
                        owned.generation = state.generation;
                        owned.since = Instant::now();
                    } else {
                        fallback = None;
                    }
                }
                reconcile = true;
                reconcile_since = Instant::now();
                reconcile_at = Instant::now();
            } else {
                output_sub = None;
            }
            next_subscription = Instant::now() + Duration::from_millis(500);
        }
        if let Some(owned) = &fallback {
            let revoked = output_sub
                .as_ref()
                .map(|s| s.state())
                .filter(|s| s.valid)
                .is_some_and(|s| {
                    s.generation != owned.generation || s.preference != Some(owned.desired)
                });
            if revoked
                || !settings.enabled
                || settings.host != owned.host
                || settings.port != owned.port
            {
                fallback = None;
                reconcile = false;
            }
        }
        if let Some(transition) =
            new_transition(&mut last_actual, actual_screen(&ctx.hub), &ctx.hub)
        {
            wake_on = None;
            if transition.0 {
                reconcile = true;
                reconcile_since = Instant::now();
                reconcile_at = Instant::now();
                if let Some(owned) = &mut fallback {
                    owned.since = Instant::now();
                }
            } else {
                if let Some(link) = &link {
                    link.set_target(None);
                }
                link_target = None;
                link_state = LinkState::Connecting;
                probe_at = None;
            }
            if settings.enabled && !settings.host.is_empty() {
                let state = output_sub.as_ref().map(|s| s.state());
                if let Some(state) = state.filter(|s| s.valid && s.preference.is_some()) {
                    let generation = state.generation;
                    let desired = state.preference.unwrap();
                    let authorized = || {
                        output_sub.as_ref().is_some_and(|s| {
                            let state = s.state();
                            state.valid
                                && state.generation == generation
                                && state.preference == Some(desired)
                        })
                    };
                    let power_client = Client::new(settings.host.clone(), settings.port)
                        .with_timeouts(
                            Duration::from_millis(tuning.connect_timeout_ms),
                            Duration::from_millis(tuning.request_timeout_ms),
                        );
                    let outcome = sync_power(ctx, transition, &power_client, &authorized);
                    if transition.0 && outcome == Some(true) {
                        wake_on = Some((power_client, Instant::now()));
                    }
                    if transition.0 && outcome == Some(false) {
                        let current = || {
                            authorized()
                                && same_transition(ctx, transition, &settings.host, settings.port)
                        };
                        let result = crate::receiver_sync::set_temporary(
                            &ctx.config.luna.send,
                            "tv_speaker",
                            &current,
                        );
                        if result != crate::receiver_sync::RouteResult::NotAttempted {
                            fallback = Some(Fallback {
                                desired,
                                generation,
                                since: Instant::now(),
                                host: settings.host.clone(),
                                port: settings.port,
                                retry_at: Instant::now(),
                            });
                            reconcile = false;
                        }
                    }
                }
            }
            continue;
        }
        let state = output_sub.as_ref().map(|s| s.state()).filter(|s| s.valid);
        if let Some(transition) = actual_screen(&ctx.hub).filter(|s| s.0) {
            if let Some(state) = state.filter(|s| s.preference.is_some()) {
                let desired = state.preference.unwrap();
                let authorized = || {
                    same_transition(ctx, transition, &settings.host, settings.port)
                        && output_sub.as_ref().is_some_and(|s| {
                            let now = s.state();
                            now.valid
                                && now.generation == state.generation
                                && now.preference == state.preference
                        })
                };
                if fallback.is_none()
                    && reconcile
                    && Instant::now() >= reconcile_at
                    && poller
                        .last_ok
                        .as_ref()
                        .is_some_and(|r| r.at >= reconcile_since && r.status.power == Power::On)
                {
                    match crate::receiver_sync::tv_speaker_active(
                        &ctx.config.luna.send,
                        &authorized,
                    ) {
                        Some(true) => {
                            fallback = Some(Fallback {
                                desired,
                                generation: state.generation,
                                since: reconcile_since,
                                host: settings.host.clone(),
                                port: settings.port,
                                retry_at: Instant::now(),
                            });
                            reconcile = false;
                        }
                        Some(false) => reconcile = false,
                        None => reconcile_at = Instant::now() + Duration::from_millis(500),
                    }
                }
                if let Some(owned) = &mut fallback {
                    if Instant::now() >= owned.retry_at
                        && poller
                            .last_ok
                            .as_ref()
                            .is_some_and(|r| r.at >= owned.since && r.status.power == Power::On)
                    {
                        let confirmed = client.as_ref().is_some_and(|client| {
                            client.host() == settings.host
                                && client.port() == settings.port
                                && restore_power_on(
                                    ctx,
                                    client,
                                    owned.since,
                                    &mut wake_on,
                                    &authorized,
                                )
                        });
                        if confirmed
                            && crate::receiver_sync::set_temporary(
                                &ctx.config.luna.send,
                                owned.desired,
                                &authorized,
                            ) == crate::receiver_sync::RouteResult::Confirmed
                        {
                            fallback = None;
                            reconcile = false;
                        } else {
                            owned.since = Instant::now();
                            owned.retry_at =
                                owned.since + Duration::from_millis(tuning.poll_ms.max(500));
                        }
                    }
                }
            } else if state.is_some() {
                fallback = None;
                reconcile = false;
            }
        }

        let polling = settings.enabled && screen::is_on(&ctx.hub);
        let idle = if settings.enabled { "Screen is off" } else { crate::receiver_plugin::off_words() };
        if !polling {
            if was_polling {
                poller.settings.invalidate(idle);
            }
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
                Ok(Cmd::SettingsSet { reply, .. }) => {
                    let _ = reply.send(SettingsResponse::Unavailable(idle.into()));
                    continue;
                }
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
            if moved {
                poller.settings.invalidate("Receiver target changed");
                if let Some(link) = &link {
                    link.set_target(None);
                }
                link_target = None;
                link_state = LinkState::Connecting;
            }
            poller.last_ok = None;
            poller.max_volume = None;
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
        if poller.settings.identity_due() {
            let model = client
                .as_ref()
                .and_then(|client| client.model_name().ok().flatten());
            poller.settings.qualify(model.as_deref());
            poller.publish(&ctx.hub, settings.enabled, &settings.host, true);
        }
        if poller.settings.prerequisites_due() {
            let evidence = client
                .as_ref()
                .and_then(|client| client.settings_prerequisites().ok());
            poller.settings.set_prerequisites(evidence);
            poller.publish(&ctx.hub, settings.enabled, &settings.host, true);
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
            if let (Some(driver), Some(level)) = (&driver, poller.level()) {
                driver.heard(level);
            }
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
        if settings.enabled && output_sub.is_none() && actual_screen(&ctx.hub).is_some_and(|s| s.0)
        {
            wake_at = soonest(wake_at, next_subscription);
        }
        if reconcile && reconcile_at > Instant::now() {
            wake_at = soonest(wake_at, reconcile_at);
        }
        if let Some(owned) = &fallback {
            if owned.retry_at > Instant::now() {
                wake_at = soonest(wake_at, owned.retry_at);
            }
        }
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
            Ok(Cmd::SettingsInterest) => {
                poller.settings.interest_until = Some(Instant::now() + Duration::from_secs(5));
                if let Some(link) = &link {
                    link.refresh_settings_interest();
                }
            }
            Ok(Cmd::SettingsSet {
                setting,
                value,
                session,
                reply,
            }) => {
                let response = match poller.settings.prepare(&setting, &value, &session) {
                    Ok((request_id, generation, context_revision, command, valid_until)) => {
                        let store = ctx.settings.clone();
                        let hub = ctx.hub.clone();
                        let host = settings.host.clone();
                        let port = settings.port;
                        let version = store
                            .describe(crate::receiver_plugin::ID)
                            .and_then(|v| v.get("version").and_then(Value::as_u64));
                        let screen_since =
                            hub.get(screen::TOPIC).and_then(|v| v.get("since").cloned());
                        match link.as_ref().map(|l| {
                            l.submit_settings_command(
                                request_id.clone(),
                                generation,
                                context_revision,
                                command,
                                valid_until,
                                Box::new(move || {
                                    let current = store.receiver();
                                    current.enabled
                                        && current.host == host
                                        && current.port == port
                                        && store
                                            .describe(crate::receiver_plugin::ID)
                                            .and_then(|v| v.get("version").and_then(Value::as_u64))
                                            == version
                                        && screen::is_on(&hub)
                                        && hub
                                            .get(screen::TOPIC)
                                            .and_then(|v| v.get("since").cloned())
                                            == screen_since
                                }),
                            )
                        }) {
                            Some(Ok(())) => {
                                poller.settings.pending(&setting, request_id.clone());
                                poller.publish(&ctx.hub, settings.enabled, &settings.host, true);
                                SettingsResponse::Accepted {
                                    request_id,
                                    session,
                                }
                            }
                            Some(Err(crate::receiver_telnet::SettingsSubmitError::Busy)) => {
                                SettingsResponse::Busy
                            }
                            _ => {
                                SettingsResponse::Unavailable("Receiver link is unavailable".into())
                            }
                        }
                    }
                    Err("Another change is pending") => SettingsResponse::Busy,
                    Err(
                        "Unknown setting"
                        | "Invalid setting value"
                        | "Value is not currently advertised",
                    ) => SettingsResponse::Invalid("Unknown setting or invalid value".into()),
                    Err(reason) => SettingsResponse::Unavailable(reason.into()),
                };
                let _ = reply.send(response);
            }
            Ok(Cmd::Volume(halves)) => {
                if let Some(driver) = &driver {
                    driver.set(f64::from(halves) / 2.0);
                }
            }
            Ok(Cmd::Link) => {
                drain_link_events(
                    link.as_ref(),
                    &mut poller,
                    link_target.as_ref(),
                    &mut link_state,
                    &mut probe_at,
                    &mut next_poll,
                    &settings.host,
                    tuning.telnet_port,
                );
                poller.publish(&ctx.hub, settings.enabled, &settings.host, true);
                if let (Some(driver), Some(level)) = (&driver, poller.level()) {
                    driver.heard(level);
                }
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

    /// The receiver's thread, guarded under a plugin session of its own.
    fn spawn(ctx: &Ctx, tx: Sender<Cmd>, rx: Receiver<Cmd>) -> std::io::Result<JoinHandle<()>> {
        let shared = Shared::new(
            ctx.hub.clone(),
            ctx.settings.clone(),
            Duration::from_secs(1),
            Arc::new(now_ms),
        );
        super::spawn(ctx, shared, tx, rx)
    }

    struct RecoveryRig {
        dir: tempfile::TempDir,
        ctx: Ctx,
        tx: Sender<Cmd>,
        runner: Option<JoinHandle<()>>,
        server: Option<JoinHandle<()>>,
        telnet: Option<JoinHandle<()>>,
        telnet_volume: std::sync::Arc<std::sync::atomic::AtomicU16>,
        server_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        power: std::sync::Arc<std::sync::atomic::AtomicBool>,
        requests: std::sync::Arc<parking_lot::Mutex<Vec<String>>>,
    }

    impl RecoveryRig {
        fn new(
            saved: &str,
            route: &str,
            on: bool,
            change_screen_at: Option<usize>,
            faults: &[&str],
        ) -> Self {
            use std::io::{Read, Write};
            use std::net::TcpListener;
            use std::os::unix::fs::PermissionsExt;
            use std::sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
                mpsc,
            };
            let dir = tempfile::tempdir().unwrap();
            for fault in faults {
                std::fs::write(dir.path().join(fault), "").unwrap();
            }
            std::fs::write(dir.path().join("preference"), saved).unwrap();
            std::fs::write(dir.path().join("route"), route).unwrap();
            std::fs::write(dir.path().join("calls"), "").unwrap();
            let script = dir.path().join("luna-send");
            let boundary = r##"#!/bin/sh
cd '@ROOT@' || exit 1
case "$3" in
  */getSystemSettings)
    echo subscription >> calls
    printf '{"returnValue":true,"subscribed":true,"settings":{"soundOutput":"%s"}}\n' "$(cat preference)"
    while :; do
      if [ -e disconnect ]; then rm disconnect; echo malformed; exit 0; fi
      sleep 0.02
    done ;;
  */soundSettings/setSoundOut)
    case "$4" in *tv_speaker*) output=tv_speaker ;; *) output=external_arc ;; esac
    echo "set_$output" >> calls
    if [ "$output" = external_arc ] && [ -e fail-restore ]; then rm fail-restore; echo '{"returnValue":false}'; exit 0; fi
    if [ "$output" = external_arc ] && [ -e fail-restore-applied ]; then rm fail-restore-applied; printf '%s' "$output" > route; echo '{"returnValue":false}'; exit 0; fi
    printf '%s' "$output" > route
    echo '{"returnValue":true}' ;;
  */getSoundOutput)
    echo audio >> calls
    if [ -e fail-audio ]; then rm fail-audio; echo '{"returnValue":false}'; exit 0; fi
    printf '{"returnValue":true,"soundOutput":"%s"}\n' "$(cat route)" ;;
  */master/getVolume)
    echo volume >> calls
    printf '{"returnValue":true,"volumeStatus":{"soundOutput":"%s"}}\n' "$(cat route)" ;;
esac
"##.replace("@ROOT@", dir.path().to_str().unwrap());
            std::fs::write(&script, boundary).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let telnet_listener = faults.contains(&"stale-telnet").then(|| {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                listener.set_nonblocking(true).unwrap();
                listener
            });
            let hub = Arc::new(Hub::new(0));
            hub.set(screen::TOPIC, json!({"on":true,"state":if change_screen_at.is_some() {"Screen Saver"} else {"Active"},"source":"luna-subscribe","since":1}));
            let settings = Arc::new(crate::settings::SettingsStore::new(
                dir.path().join("settings"),
                hub.clone(),
            ));
            crate::receiver_plugin::register_page(&settings).unwrap();
            settings
                .apply(
                    crate::receiver_plugin::ID,
                    vec![
                        settings_schema::SettingChange {
                            key: "host".parse().unwrap(),
                            value: settings_schema::Value::Text("127.0.0.1".into()),
                        },
                        settings_schema::SettingChange {
                            key: "port".parse().unwrap(),
                            value: settings_schema::Value::Integer(port.into()),
                        },
                    ],
                    None,
                )
                .unwrap();
            let mut config = crate::config::Config::default();
            config.luna.send = script.display().to_string();
            config.receiver.telnet_port = telnet_listener
                .as_ref()
                .map_or(0, |listener| listener.local_addr().unwrap().port());
            config.receiver.poll_ms = 50;
            config.receiver.max_backoff_ms = 100;
            config.receiver.connect_timeout_ms = 100;
            config.receiver.request_timeout_ms = 150;
            let ctx = Ctx {
                config: Arc::new(config),
                paths: crate::config::Paths::new(dir.path()),
                hub: hub.clone(),
                settings,
                stop: crate::control::Stop::new(),
            };
            let power = Arc::new(AtomicBool::new(on));
            let server_stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(parking_lot::Mutex::new(Vec::new()));
            let telnet_volume = Arc::new(std::sync::atomic::AtomicU16::new(41));
            let telnet = telnet_listener.map(|listener| {
                let stopped = server_stop.clone();
                let volume = telnet_volume.clone();
                thread::spawn(move || {
                    while !stopped.load(Ordering::Relaxed) {
                        let (mut stream, _) = match listener.accept() {
                            Ok(stream) => stream,
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(2));
                                continue;
                            }
                            Err(e) => panic!("{e}"),
                        };
                        stream
                            .set_read_timeout(Some(Duration::from_millis(30)))
                            .unwrap();
                        let mut pending = Vec::new();
                        let mut sent_power = false;
                        while !stopped.load(Ordering::Relaxed) {
                            let mut bytes = [0; 256];
                            let size = match stream.read(&mut bytes) {
                                Ok(0) => break,
                                Ok(size) => size,
                                Err(e)
                                    if matches!(
                                        e.kind(),
                                        std::io::ErrorKind::WouldBlock
                                            | std::io::ErrorKind::TimedOut
                                    ) =>
                                {
                                    continue;
                                }
                                Err(_) => break,
                            };
                            pending.extend_from_slice(&bytes[..size]);
                            while let Some(end) = pending.iter().position(|byte| *byte == b'\r') {
                                let query = String::from_utf8_lossy(&pending[..end]);
                                let reply = match query.as_ref() {
                                    "PW?" if !sent_power => {
                                        sent_power = true;
                                        "PWON\r".to_owned()
                                    }
                                    "MV?" => format!("MV{}\r", volume.load(Ordering::Relaxed)),
                                    "SI?" => "SITV\r".to_owned(),
                                    "MS?" => "MSSTEREO\r".to_owned(),
                                    "SSINFAISSIG ?" => "SYSDA PCM ZERO\r".to_owned(),
                                    _ => "MUOFF\r".to_owned(),
                                };
                                let _ = stream.write_all(reply.as_bytes());
                                pending.drain(..=end);
                            }
                        }
                    }
                })
            });
            let powered = power.clone();
            let stopped = server_stop.clone();
            let received = requests.clone();
            let rapid_cycle = faults.contains(&"rapid-cycle");
            let server = thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(stream) => stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(e) => panic!("{e}"),
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut buf = [0; 2048];
                    let n = stream.read(&mut buf).unwrap();
                    let request = String::from_utf8_lossy(&buf[..n])
                        .lines()
                        .next()
                        .unwrap()
                        .to_owned();
                    let index = received.lock().len();
                    if change_screen_at == Some(index) {
                        if rapid_cycle {
                            hub.set(screen::TOPIC, json!({"on":false,"state":"Active Standby","source":"luna-subscribe","since":2}));
                        }
                        hub.set(
                            screen::TOPIC,
                            json!({"on":true,"state":"Active","source":"luna-subscribe","since":2}),
                        );
                    }
                    if request.contains("?ZMOFF") {
                        powered.store(false, Ordering::Relaxed);
                    }
                    let body = if request.contains("formiPhoneAppDirect") {
                        String::new()
                    } else {
                        format!(
                            "<item><Power><value>{}</value></Power><VolumeDisplay><value>Absolute</value></VolumeDisplay></item>",
                            if powered.load(Ordering::Relaxed) {
                                "ON"
                            } else {
                                "OFF"
                            }
                        )
                    };
                    received.lock().push(request);
                    write!(
                        stream,
                        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                }
            });
            let (tx, rx) = mpsc::channel();
            let runner = Some(spawn(&ctx, tx.clone(), rx).unwrap());
            Self {
                dir,
                ctx,
                tx,
                runner,
                server: Some(server),
                telnet,
                telnet_volume,
                server_stop,
                power,
                requests,
            }
        }
        fn count(&self, event: &str) -> usize {
            std::fs::read_to_string(self.dir.path().join("calls"))
                .unwrap()
                .lines()
                .filter(|line| *line == event)
                .count()
        }
        fn route(&self) -> String {
            std::fs::read_to_string(self.dir.path().join("route")).unwrap()
        }
        fn flag(&self, name: &str) {
            std::fs::write(self.dir.path().join(name), "").unwrap();
        }
        fn wait(&self, condition: impl Fn() -> bool) {
            let until = Instant::now() + Duration::from_secs(5);
            while !condition() {
                assert!(
                    Instant::now() < until,
                    "native calls: {:?}; HTTP: {:?}",
                    std::fs::read_to_string(self.dir.path().join("calls")),
                    self.requests.lock()
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
        fn restart(&mut self) {
            self.tx.send(Cmd::Stop).unwrap();
            self.runner.take().unwrap().join().unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            self.tx = tx;
            self.runner = Some(spawn(&self.ctx, self.tx.clone(), rx).unwrap());
        }
    }
    impl Drop for RecoveryRig {
        fn drop(&mut self) {
            let _ = self.tx.send(Cmd::Stop);
            if let Some(runner) = self.runner.take() {
                let _ = runner.join();
            }
            self.server_stop
                .store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(server) = self.server.take() {
                let _ = server.join();
            }
            if let Some(telnet) = self.telnet.take() {
                let _ = telnet.join();
            }
        }
    }

    #[test]
    fn fresh_nonpower_telnet_delta_cannot_restore_arc_using_cached_on_after_reconnect() {
        use std::sync::atomic::Ordering;
        let rig = RecoveryRig::new(
            "external_arc",
            "external_arc",
            true,
            None,
            &["stale-telnet"],
        );
        rig.wait(|| {
            rig.count("audio") >= 1
                && rig.ctx.hub.get(TOPIC).is_some_and(|value| {
                    value["volumeDisplay"] == "absolute" && value["volumeDb"] == -39.0
                })
        });
        rig.power.store(false, Ordering::Relaxed);
        std::fs::write(rig.dir.path().join("route"), "tv_speaker").unwrap();
        let reads = rig.requests.lock().len();
        rig.flag("disconnect");
        rig.telnet_volume.store(42, Ordering::Relaxed);
        rig.wait(|| {
            rig.count("subscription") >= 2
                && rig.requests.lock().len() >= reads + 2
                && rig
                    .ctx
                    .hub
                    .get(TOPIC)
                    .is_some_and(|value| value["power"] == "on" && value["volumeDb"] == -38.0)
        });
        assert_eq!(rig.route(), "tv_speaker");
        assert_eq!(rig.count("set_external_arc"), 0);
        rig.power.store(true, Ordering::Relaxed);
        rig.telnet_volume.store(43, Ordering::Relaxed);
        rig.wait(|| rig.route() == "external_arc" && rig.count("volume") >= 2);
        assert_eq!(rig.count("set_external_arc"), 1);
        assert!(
            !rig.requests
                .lock()
                .iter()
                .any(|request| request.contains("?ZMON"))
        );
    }

    #[test]
    fn restore_reuses_a_valid_direct_http_on_witness_but_not_one_before_the_phase() {
        use std::sync::atomic::Ordering;
        let mut rig = RecoveryRig::new("external_arc", "external_arc", true, None, &[]);
        rig.wait(|| rig.count("audio") >= 1);
        rig.tx.send(Cmd::Stop).unwrap();
        rig.runner.take().unwrap().join().unwrap();
        let settings = rig.ctx.settings.receiver();
        let client = Client::new(settings.host, settings.port);
        let since = Instant::now();
        assert_eq!(client.status().unwrap().power, Power::On);
        let mut witness = Some((client.clone(), Instant::now()));
        let reads = rig.requests.lock().len();
        assert!(restore_power_on(
            &rig.ctx,
            &client,
            since,
            &mut witness,
            &|| true
        ));
        assert_eq!(rig.requests.lock().len(), reads);
        rig.power.store(false, Ordering::Relaxed);
        witness = Some((client.clone(), since));
        assert!(!restore_power_on(
            &rig.ctx,
            &client,
            Instant::now(),
            &mut witness,
            &|| true
        ));
        assert_eq!(rig.requests.lock().len(), reads + 1);
    }

    #[test]
    fn subscription_reconnects_while_awake_without_replaying_on_and_preserves_standby_backup() {
        let rig = RecoveryRig::new("external_arc", "external_arc", true, None, &[]);
        rig.wait(|| rig.count("audio") >= 1);
        rig.flag("disconnect");
        rig.wait(|| rig.count("subscription") >= 2 && rig.count("audio") >= 2);
        assert!(!rig.requests.lock().iter().any(|r| r.contains("?ZMON")));
        assert!(!rig.requests.lock().iter().any(|r| r.contains("?SI")));
        assert_eq!(rig.count("set_tv_speaker"), 0);
        assert_eq!(rig.count("set_external_arc"), 0);
        rig.ctx.hub.set(
            screen::TOPIC,
            json!({"on":false,"state":"Active Standby","source":"luna-subscribe","since":3}),
        );
        rig.tx.send(Cmd::Wake).unwrap();
        rig.wait(|| rig.requests.lock().iter().any(|r| r.contains("?ZMOFF")));
    }

    #[test]
    fn unconfirmed_fallback_and_failed_restore_keep_restoration_responsibility() {
        use std::sync::atomic::Ordering;
        for fail_confirmation in [false, true] {
            let rig = RecoveryRig::new(
                "external_arc",
                "external_arc",
                false,
                None,
                if fail_confirmation {
                    &["fail-audio"]
                } else {
                    &[]
                },
            );
            rig.wait(|| rig.route() == "tv_speaker" && rig.count("audio") >= 1);
            rig.flag("fail-restore");
            rig.power.store(true, Ordering::Relaxed);
            rig.tx.send(Cmd::Nudge).unwrap();
            rig.wait(|| {
                rig.route() == "external_arc"
                    && rig.count("set_external_arc") >= 2
                    && rig.count("volume") >= if fail_confirmation { 1 } else { 2 }
            });
            assert_eq!(
                rig.requests
                    .lock()
                    .iter()
                    .filter(|r| r.contains("?ZMON"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn a_lost_restore_reply_confirms_already_active_arc_without_rerouting_it_again() {
        use std::sync::atomic::Ordering;
        let rig = RecoveryRig::new("external_arc", "external_arc", false, None, &[]);
        rig.wait(|| rig.route() == "tv_speaker" && rig.count("volume") >= 1);
        rig.flag("fail-restore-applied");
        rig.power.store(true, Ordering::Relaxed);
        rig.tx.send(Cmd::Nudge).unwrap();
        rig.wait(|| rig.route() == "external_arc" && rig.count("volume") >= 2);
        assert_eq!(rig.count("set_external_arc"), 1);
    }

    #[test]
    fn restart_reconciles_saved_arc_native_tv_and_confirmed_receiver_on() {
        use std::sync::atomic::Ordering;
        let mut rig = RecoveryRig::new("external_arc", "external_arc", false, None, &[]);
        rig.wait(|| rig.route() == "tv_speaker" && rig.count("volume") >= 1);
        rig.power.store(true, Ordering::Relaxed);
        rig.restart();
        rig.wait(|| rig.route() == "external_arc" && rig.count("set_external_arc") >= 1);
        assert_eq!(
            std::fs::read_to_string(rig.dir.path().join("preference")).unwrap(),
            "external_arc"
        );
    }

    #[test]
    fn saver_to_active_during_status_or_command_is_not_a_power_cancellation() {
        for request in [0, 1] {
            let rig = RecoveryRig::new("external_arc", "external_arc", false, Some(request), &[]);
            rig.wait(|| rig.route() == "tv_speaker" && rig.count("volume") >= 1);
            assert_eq!(
                rig.requests
                    .lock()
                    .iter()
                    .filter(|r| r.contains("?ZMON"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn fast_standby_wake_during_http_consumes_a_new_real_wake_not_the_canceled_old_one() {
        for (request, expected) in [(0, 1), (1, 2)] {
            let rig = RecoveryRig::new(
                "external_arc",
                "external_arc",
                false,
                Some(request),
                &["rapid-cycle"],
            );
            rig.wait(|| rig.route() == "tv_speaker" && rig.count("volume") >= 1);
            assert_eq!(
                rig.requests
                    .lock()
                    .iter()
                    .filter(|r| r.contains("?ZMON"))
                    .count(),
                expected
            );
        }
    }

    #[test]
    fn a_later_failed_wake_reapplies_fallback_if_native_cec_reset_current_routing() {
        let rig = RecoveryRig::new("external_arc", "external_arc", false, None, &[]);
        rig.wait(|| {
            rig.route() == "tv_speaker"
                && rig
                    .ctx
                    .hub
                    .get(TOPIC)
                    .is_some_and(|value| value["polling"] == true)
        });
        rig.ctx.hub.set(
            screen::TOPIC,
            json!({"on":false,"state":"Active Standby","source":"luna-subscribe","since":2}),
        );
        rig.tx.send(Cmd::Wake).unwrap();
        rig.wait(|| {
            rig.ctx
                .hub
                .get(TOPIC)
                .is_some_and(|value| value["polling"] == false)
        });
        std::fs::write(rig.dir.path().join("route"), "external_arc").unwrap();
        rig.ctx.hub.set(
            screen::TOPIC,
            json!({"on":true,"state":"Active","source":"luna-subscribe","since":3}),
        );
        rig.tx.send(Cmd::Wake).unwrap();
        rig.wait(|| {
            rig.route() == "tv_speaker"
                && rig.count("set_tv_speaker") == 2
                && rig.count("volume") >= 2
        });
        assert_eq!(
            rig.requests
                .lock()
                .iter()
                .filter(|r| r.contains("?ZMON"))
                .count(),
            2
        );
    }

    #[test]
    fn only_actual_tvpower_state_can_drive_power() {
        let hub = Hub::new(0);
        assert_eq!(actual_screen(&hub), None);
        for source in ["assumed", "config"] {
            hub.set(screen::TOPIC, json!({"on":true,"source":source,"since":1}));
            assert_eq!(actual_screen(&hub), None);
        }
        hub.set(
            screen::TOPIC,
            json!({"on":true,"state":"Active","source":"luna-subscribe","since":2}),
        );
        assert_eq!(
            actual_screen(&hub),
            Some((true, hub.topic_at(screen::TOPIC).0))
        );
        let mut last = actual_screen(&hub);
        for state in ["Screen Off", "Suspend", "Power Off", "unknown"] {
            hub.set(
                screen::TOPIC,
                json!({"on":false,"state":state,"source":"luna-subscribe","since":4}),
            );
            assert_eq!(new_transition(&mut last, actual_screen(&hub), &hub), None);
        }
        hub.set(
            screen::TOPIC,
            json!({"on":true,"state":"Screen Saver","source":"luna-subscribe","since":5}),
        );
        assert_eq!(new_transition(&mut last, actual_screen(&hub), &hub), None);
        hub.set(
            screen::TOPIC,
            json!({"on":false,"state":"Active Standby","source":"luna-poll","since":3}),
        );
        assert_eq!(
            actual_screen(&hub),
            Some((false, hub.topic_at(screen::TOPIC).0))
        );
    }
    #[test]
    fn each_real_wake_and_standby_has_one_attempt() {
        let hub = Hub::new(0);
        let mut last = None;
        assert_eq!(new_transition(&mut last, None, &hub), None);
        assert_eq!(new_transition(&mut last, Some((false, 1)), &hub), None);
        assert_eq!(
            new_transition(&mut last, Some((true, 2)), &hub),
            Some((true, 2))
        );
        assert_eq!(new_transition(&mut last, Some((true, 3)), &hub), None);
        assert_eq!(new_transition(&mut last, None, &hub), None);
        assert_eq!(new_transition(&mut last, Some((true, 4)), &hub), None);
        assert_eq!(
            new_transition(&mut last, Some((false, 5)), &hub),
            Some((false, 5))
        );
        assert_eq!(new_transition(&mut last, Some((false, 6)), &hub), None);
        assert_eq!(
            new_transition(&mut last, Some((true, 7)), &hub),
            Some((true, 7))
        );
    }
    #[test]
    fn temporary_fallback_survives_standby_and_respects_selection() {
        use settings_schema::{SettingChange, Value as SettingValue};
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        for explicit_selection in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("luna-send");
            let calls = dir.path().join("calls");
            let fifo = dir.path().join("events");
            let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
            let route = dir.path().join("route");
            std::fs::write(&route, "external_arc").unwrap();
            let body = format!(
                r##"#!/bin/sh
case "$3" in
  */getSystemSettings) echo '{{"returnValue":true,"subscribed":true,"settings":{{"soundOutput":"external_arc"}}}}'; exec cat '{events}' ;;
  */soundSettings/setSoundOut)
    case "$4" in *tv_speaker*) output=tv_speaker ;; *) output=external_arc ;; esac
    printf '%s' "$output" > '{route}'
    printf '%s\n' "$4" >> '{calls}'
    echo '{{"returnValue":true}}' ;;
  */getSoundOutput) output=$(cat '{route}'); printf '{{"returnValue":true,"soundOutput":"%s","settingsSoundOutputs":["%s"]}}\n' "$output" "$output" ;;
  */master/getVolume) output=$(cat '{route}'); printf '{{"returnValue":true,"volumeStatus":{{"soundOutput":"%s"}}}}\n' "$output" ;;
esac
"##,
                events = fifo.display(),
                route = route.display(),
                calls = calls.display()
            );
            std::fs::write(&script, body).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            let hub = Arc::new(Hub::new(0));
            hub.set(
                screen::TOPIC,
                json!({"on":true,"state":"Active","source":"luna-subscribe","since":1}),
            );
            let settings = Arc::new(crate::settings::SettingsStore::new(
                dir.path().join("settings"),
                hub.clone(),
            ));
            crate::receiver_plugin::register_page(&settings).unwrap();
            settings
                .apply(
                    crate::receiver_plugin::ID,
                    vec![
                        SettingChange {
                            key: "host".parse().unwrap(),
                            value: SettingValue::Text("127.0.0.1".into()),
                        },
                        SettingChange {
                            key: "port".parse().unwrap(),
                            value: SettingValue::Integer(port.into()),
                        },
                    ],
                    None,
                )
                .unwrap();
            let mut config = crate::config::Config::default();
            config.luna.send = script.display().to_string();
            config.receiver.telnet_port = 0;
            config.receiver.connect_timeout_ms = 100;
            config.receiver.request_timeout_ms = 200;
            config.receiver.poll_ms = 100;
            let ctx = Ctx {
                config: Arc::new(config),
                paths: crate::config::Paths::new(dir.path()),
                hub: hub.clone(),
                settings,
                stop: crate::control::Stop::new(),
            };
            let (tx, rx) = mpsc::channel();
            let runner = spawn(&ctx, tx.clone(), rx).unwrap();
            let wait_for = |f: &dyn Fn() -> bool| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !f() {
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(20));
                }
            };
            wait_for(&|| {
                std::fs::read_to_string(&calls).is_ok_and(|s| s.contains("tv_speaker"))
                    && hub.get(TOPIC).is_some_and(|v| v["polling"] == true)
            });
            hub.set(
                screen::TOPIC,
                json!({"on":false,"state":"Active Standby","source":"luna-subscribe","since":2}),
            );
            tx.send(Cmd::Wake).unwrap();
            wait_for(&|| hub.get(TOPIC).is_some_and(|v| v["polling"] == false));
            let mut events = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
            if explicit_selection {
                writeln!(
                    events,
                    "{{\"returnValue\":true,\"settings\":{{\"soundOutput\":\"tv_speaker\"}}}}"
                )
                .unwrap();
                thread::sleep(Duration::from_millis(100));
            }
            let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let server = thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    if let Ok((mut stream, _)) = listener.accept() {
                        let mut buf = [0; 2048];
                        let _ = stream.read(&mut buf);
                        let body = "<item><Power><value>ON</value></Power></item>";
                        let _ = write!(
                            stream,
                            "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                    } else {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            });
            hub.set(
                screen::TOPIC,
                json!({"on":true,"state":"Active","source":"luna-subscribe","since":3}),
            );
            tx.send(Cmd::Wake).unwrap();
            if explicit_selection {
                thread::sleep(Duration::from_millis(500));
            } else {
                wait_for(&|| {
                    std::fs::read_to_string(&calls).is_ok_and(|s| s.contains("external_arc"))
                });
            }
            let records = std::fs::read_to_string(&calls).unwrap();
            assert_eq!(
                records.lines().count(),
                if explicit_selection { 1 } else { 2 }
            );
            assert!(records.lines().all(|line| {
                let value = serde_json::from_str::<Value>(line).unwrap();
                value.get("store").is_none() && value.get("soundOut").is_some()
            }));
            tx.send(Cmd::Stop).unwrap();
            runner.join().unwrap();
            stop.store(true, Ordering::Relaxed);
            server.join().unwrap();
        }
    }
    #[test]
    fn state_host_or_preference_change_cancels_in_flight_power() {
        use settings_schema::{SettingChange, Value as SettingValue};
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        for (command_in_flight, cancellation) in [
            (false, "standby"),
            (true, "standby"),
            (false, "host"),
            (true, "host"),
            (false, "preference"),
            (true, "preference"),
            (false, "rapid-cycle"),
            (true, "rapid-cycle"),
            (false, "unknown"),
            (true, "unknown"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let hub = Arc::new(Hub::new(0));
            hub.set(
                screen::TOPIC,
                json!({"on":true,"state":"Active","source":"luna-subscribe","since":1}),
            );
            let settings = Arc::new(crate::settings::SettingsStore::new(
                dir.path().join("settings"),
                hub.clone(),
            ));
            crate::receiver_plugin::register_page(&settings).unwrap();
            settings
                .apply(
                    crate::receiver_plugin::ID,
                    vec![
                        SettingChange {
                            key: "host".parse().unwrap(),
                            value: SettingValue::Text("127.0.0.1".into()),
                        },
                        SettingChange {
                            key: "port".parse().unwrap(),
                            value: SettingValue::Integer(port.into()),
                        },
                    ],
                    None,
                )
                .unwrap();
            let ctx = Ctx {
                config: Arc::new(crate::config::Config::default()),
                paths: crate::config::Paths::new(dir.path()),
                hub: hub.clone(),
                settings: settings.clone(),
                stop: crate::control::Stop::new(),
            };
            let allowed = Arc::new(AtomicBool::new(true));
            let authorized = allowed.clone();
            let cancel = move || match cancellation {
                "standby" | "rapid-cycle" | "unknown" => {
                    hub.set(screen::TOPIC, json!({"on":false,"state": if cancellation == "unknown" {"unknown"} else {"Active Standby"},"source":"luna-subscribe","since":2}));
                    if cancellation != "standby" {
                        hub.set(
                            screen::TOPIC,
                            json!({"on":true,"state":"Active","source":"luna-subscribe","since":3}),
                        );
                    }
                }
                "host" => {
                    settings
                        .apply(
                            crate::receiver_plugin::ID,
                            vec![SettingChange {
                                key: "host".parse().unwrap(),
                                value: SettingValue::Text("127.0.0.2".into()),
                            }],
                            None,
                        )
                        .unwrap();
                }
                _ => allowed.store(false, Ordering::Relaxed),
            };
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 2048];
                assert!(stream.read(&mut request).unwrap() > 0);
                let body = "<item><Power><value>OFF</value></Power></item>";
                if command_in_flight {
                    write!(
                        stream,
                        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                    let (mut command, _) = listener.accept().unwrap();
                    let n = command.read(&mut request).unwrap();
                    assert!(String::from_utf8_lossy(&request[..n]).contains("?ZMON"));
                    cancel();
                    write!(command, "HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n").unwrap();
                } else {
                    cancel();
                    write!(
                        stream,
                        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                }
                listener.set_nonblocking(true).unwrap();
                thread::sleep(Duration::from_millis(100));
                assert_eq!(
                    listener.accept().unwrap_err().kind(),
                    std::io::ErrorKind::WouldBlock
                );
            });
            let client = Client::new("127.0.0.1", port)
                .with_timeouts(Duration::from_secs(1), Duration::from_secs(2));
            assert_eq!(
                sync_power(&ctx, (true, 1), &client, &|| authorized
                    .load(Ordering::Relaxed)),
                None
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn wake_attempt_confirms_even_when_status_or_command_fails() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        for (initial_fails, command_fails, powers_on) in [
            (true, false, true),
            (false, true, true),
            (true, true, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let hub = Arc::new(Hub::new(0));
            hub.set(
                screen::TOPIC,
                json!({"on":true,"state":"Active","source":"luna-subscribe","since":1}),
            );
            let settings = Arc::new(crate::settings::SettingsStore::new(
                dir.path().join("settings"),
                hub.clone(),
            ));
            crate::receiver_plugin::register_page(&settings).unwrap();
            settings
                .apply(
                    crate::receiver_plugin::ID,
                    vec![
                        settings_schema::SettingChange {
                            key: "host".parse().unwrap(),
                            value: settings_schema::Value::Text("127.0.0.1".into()),
                        },
                        settings_schema::SettingChange {
                            key: "port".parse().unwrap(),
                            value: settings_schema::Value::Integer(port.into()),
                        },
                    ],
                    None,
                )
                .unwrap();
            let mut config = crate::config::Config::default();
            config.receiver.request_timeout_ms = 250;
            let ctx = Ctx {
                config: Arc::new(config),
                paths: crate::config::Paths::new(dir.path()),
                hub,
                settings,
                stop: crate::control::Stop::new(),
            };
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let server = thread::spawn(move || {
                let mut requests = Vec::new();
                while !stopped.load(Ordering::Relaxed) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(stream) => stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(e) => panic!("{e}"),
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut buf = [0; 2048];
                    let n = stream.read(&mut buf).unwrap();
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let command = request.contains("?ZMON");
                    let failed = if requests.is_empty() {
                        initial_fails
                    } else {
                        command && command_fails
                    };
                    let power = if requests.len() >= 2 && powers_on {
                        "ON"
                    } else {
                        "OFF"
                    };
                    let body = format!("<item><Power><value>{power}</value></Power></item>");
                    let code = if failed { 500 } else { 200 };
                    write!(
                        stream,
                        "HTTP/1.0 {code} Test\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    requests.push(request.lines().next().unwrap().to_owned());
                }
                requests
            });
            let client = Client::new("127.0.0.1", port)
                .with_timeouts(Duration::from_millis(100), Duration::from_millis(250));
            let started = Instant::now();
            assert_eq!(
                sync_power(&ctx, (true, 1), &client, &|| true),
                Some(powers_on)
            );
            assert!(started.elapsed() < Duration::from_secs(2));
            stop.store(true, Ordering::Relaxed);
            let requests = server.join().unwrap();
            assert!(requests.len() >= 3, "{requests:?}");
            assert_eq!(requests.iter().filter(|r| r.contains("?ZMON")).count(), 1);
            assert!(
                requests[2..]
                    .iter()
                    .all(|r| r.contains("formMainZone_MainZoneXmlStatusLite.xml"))
            );
        }
    }

    #[test]
    fn denon_power_command_and_status_over_loopback() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let mut paths = Vec::new();
            for power in ["OFF", "", "ON"] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 2048];
                let n = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                paths.push(request.lines().next().unwrap().to_owned());
                let body = if power.is_empty() {
                    String::new()
                } else {
                    format!("<item><Power><value>{power}</value></Power></item>")
                };
                write!(
                    stream,
                    "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
            paths
        });
        let client = Client::new("127.0.0.1", port)
            .with_timeouts(Duration::from_secs(1), Duration::from_secs(2));
        assert_eq!(client.status().unwrap().power, Power::Standby);
        client.send_command("ZMON").unwrap();
        assert_eq!(client.status().unwrap().power, Power::On);
        let paths = server.join().unwrap();
        assert!(paths[0].contains("formMainZone_MainZoneXmlStatusLite.xml"));
        assert!(paths[1].contains("formiPhoneAppDirect.xml?ZMON"));
        assert!(paths[2].contains("formMainZone_MainZoneXmlStatusLite.xml"));
    }

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
