//! Turns what the shell already sees into diagnostics events: session phases, the negotiated format, how a session ended, frame pacing,
//! network trouble, controllers and the panel's state. Plain data in, events out, so it is tested without a TV.
use super::diagnostics::{Event, Level};
use super::helper::Sample;
use super::stats::percentiles;
use crate::lifecycle::Frames;
use lumagame_core::{ipc::{self, StreamInfo}, session::{Outcome, Phase}};
use serde_json::Value;

/// Frame samples are checked for trouble this often.
const WINDOW_MS: u64 = 1000;
/// Debug's pacing line covers this long. Once a second, with the lines Info already sends, would fill nearly all of a batch.
const DETAIL_MS: u64 = 2000;
/// Histograms cover this long, so the finest detail costs one line every few seconds.
const HIST_MS: u64 = 6000;
/// A warning of one kind goes out at most this often and carries what built up since the last.
const WARN_MS: u64 = 5000;
/// Loss that earns a warning; the HUD already marks 0.5 %.
const LOSS_PCT: f32 = 1.0;
/// Dropped frames in one window that make a burst: a tenth of the frames at 60 fps.
const DROP_BURST: u64 = 6;
/// An arrival gap the HUD calls stalling.
const STALL_US: u64 = 100_000;
/// The panel's first answer can be left over from the last run, so the start state is read this long after Running begins.
const SETTLE_MS: u64 = 2000;
/// The panel is watched this long after a session stops running, so what a quit leaves on screen is on record.
const TAIL_MS: u64 = 3000;
/// A change in the panel is reported at most this often.
const CHANGE_MS: u64 = 2000;
/// A reading older than this is not the panel's state any more.
const FRESH_MS: u64 = 3000;
/// A stream whose panel shows no picture this long after it ran is reported.
const PICTURE_MS: u64 = 5000;
/// Frame intervals binned in tenths of the interval: under half, under 0.9, under 1.1, under 1.5, under 2, under 3, and beyond.
const GAP_EDGES: [u64; 6] = [5, 9, 11, 15, 20, 30];
/// Arrival-to-feed times binned in microseconds: under 0.25 ms, 0.5, 1, 2, 4, 8, 16, and beyond.
const FEED_EDGES: [u64; 7] = [250, 500, 1000, 2000, 4000, 8000, 16000];
/// Samples kept per window, so a flood of them cannot grow memory.
const CAP: usize = 1024;

/// The engines this app ships; any other id reports as `other`, never by its name.
pub fn engine_word(id: &str) -> &'static str { ["gfn", "punktfunk", "gamestream", "native"].into_iter().find(|known| *known == id).unwrap_or("other") }

pub fn phase_word(phase: Option<&Phase>) -> &'static str {
    match phase {
        None => "none",
        Some(Phase::Checking) => "checking",
        Some(Phase::SignIn) => "sign-in",
        Some(Phase::Queue { .. }) => "queue",
        Some(Phase::Starting) => "starting",
        Some(Phase::Running) => "running",
        Some(Phase::Stopping) => "stopping",
        Some(Phase::Done(_)) => "done",
    }
}

/// H.273 transfer characteristics that carry HDR: SMPTE ST 2084 (PQ) and ARIB STD-B67 (HLG).
const TRANSFER_PQ: u8 = 16;
const TRANSFER_HLG: u8 = 18;

/// Why a stream is not HDR, from the stream and what was asked for. The engines' own wording never leaves the TV.
/// A stream the engine declared HDR still reads `service_sent_sdr` when its own colour tags are neither PQ nor HLG: the tags are what the service really sent.
pub fn sdr_kind(stream: &StreamInfo, wanted: bool) -> &'static str {
    if stream.hdr {
        let sdr_tags = stream.vui.is_some_and(|vui| !matches!(vui.transfer, TRANSFER_PQ | TRANSFER_HLG));
        return if wanted && sdr_tags { "service_sent_sdr" } else { "none" };
    }
    if !wanted { return "not_requested"; }
    match stream.sdr_reason.as_deref() {
        Some(reason) if reason.contains("no HDR for this game") => "game_unsupported",
        Some(reason) if reason.contains("sent no HDR") => "service_sent_sdr",
        Some(_) => "other",
        None => "unreported",
    }
}

/// What was asked for in Settings, to set beside what the stream turned out to be.
#[derive(Clone, Copy, Debug)]
pub struct Want { pub hdr: bool, pub fps: u32, pub height: u32 }

/// The session as the model stands after a step.
pub struct Seen<'a> {
    pub id: u64,
    pub engine: &'static str,
    pub phase: &'a Phase,
    pub stream: Option<&'a StreamInfo>,
    pub ended_by: Option<&'static str>,
    pub played_s: Option<u32>,
    pub want: Want,
}

/// One answer from the helper's display endpoint. Only the fields worth a line are kept.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reading { width: u32, height: u32, hdr_type: u8, muted: Option<bool>, hz: Option<f32>, min_hz: Option<f32>, fewest: Option<u32>, src_hz: Option<f32>, vrr: Option<bool>, low_delay: Option<bool> }
impl Reading {
    fn of(value: &Value) -> Option<Self> {
        let window = value.get("window").filter(|window| window.is_object())?;
        let refresh = value.get("refresh");
        let float = |v: Option<&Value>| v.and_then(Value::as_f64).map(|n| n as f32);
        let whole = |v: Option<&Value>| v.and_then(Value::as_u64).map(|n| u32::try_from(n).unwrap_or(u32::MAX));
        Some(Self {
            width: whole(window.get("width")).unwrap_or(0),
            height: whole(window.get("height")).unwrap_or(0),
            hdr_type: window.get("hdrType").and_then(Value::as_u64).map_or(0, |n| n.min(255) as u8),
            muted: window.get("muted").and_then(Value::as_bool),
            hz: float(refresh.and_then(|r| r.get("hz"))),
            min_hz: float(refresh.and_then(|r| r.get("minHz"))),
            fewest: whole(refresh.and_then(|r| r.get("fewest"))),
            src_hz: float(window.get("sourceHz")),
            vrr: value.pointer("/firmware/sysVeri10/vrr").and_then(Value::as_bool),
            low_delay: window.get("lowDelay").and_then(Value::as_bool),
        })
    }
    /// A different picture, not a refresh counter wobbling around its rate.
    fn differs(&self, other: &Self) -> bool {
        (self.width, self.height, self.hdr_type, self.muted, self.vrr, self.low_delay) != (other.width, other.height, other.hdr_type, other.muted, other.vrr, other.low_delay)
            || self.hz.zip(other.hz).is_some_and(|(a, b)| (a - b).abs() >= 2.0)
    }
    /// The video window has a size and is not muted. A mute the TV does not report counts as none.
    fn shows_picture(&self) -> bool { self.width > 0 && self.muted != Some(true) }
    fn event(&self, ms: u64, at: &'static str) -> Event {
        Event::Display { ms, at, width: self.width, height: self.height, hdr_type: self.hdr_type, muted: self.muted, hz: self.hz, src_hz: self.src_hz, vrr: self.vrr, low_delay: self.low_delay }
    }
}

#[derive(Default)]
struct Tracked {
    id: u64,
    engine: &'static str,
    phase: &'static str,
    queue: Option<u32>,
    began: u64,
    format: Option<StreamInfo>,
    /// The `hdr_missing` reasons already sent, so a stream that is re-announced (its colour tags arrive after the first line) never repeats one.
    missing: Vec<&'static str>,
    running: Option<u64>,
    left: Option<u64>,
    over: bool,
    start_read: bool,
    after_read: bool,
    shown: Option<Reading>,
    changed: u64,
    /// When the feeder first took a frame of this stream.
    first_frame: Option<u64>,
    /// Still waiting for the first picture. A stream already running when first seen is not timed: its start is not known.
    looking: bool,
    warned: bool,
}

#[derive(Default)]
struct Latest {
    fps: Option<f32>, client_ms: Option<f32>, host_ms: Option<f32>, pad_bursts: Option<u32>, pad_gap_us: Option<u32>,
    pacing_ms: Option<f32>, host_gap_ms: Option<[f32; 2]>, arrival_gap_ms: Option<[f32; 2]>,
}

/// Per-frame samples since the last pacing line.
#[derive(Default)]
struct Window { from: Option<u64>, n: u32, gaps: Vec<u64>, feeds: Vec<u64>, queue: u32, late: u32, stalls: u32, max_gap: u64, drops: [u64; 5] }
impl Window {
    fn add(&mut self, frames: &Frames, interval_us: u64, now: u64) {
        self.from.get_or_insert(now);
        let late = if interval_us > 0 { interval_us * 3 / 2 } else { u64::MAX };
        for s in &frames.samples {
            self.n = self.n.saturating_add(1);
            if let Ok(gap) = u64::try_from(s[0]) {
                if self.gaps.len() < CAP { self.gaps.push(gap); }
                self.late += u32::from(gap > late);
                self.stalls += u32::from(gap >= STALL_US);
                self.max_gap = self.max_gap.max(gap);
            }
            match s[3] {
                0 => if let Ok(feed) = u64::try_from(s[1]) { if self.feeds.len() < CAP { self.feeds.push(feed); } },
                reason @ 1..=5 => self.drops[reason as usize - 1] += 1,
                _ => {}
            }
            self.queue = self.queue.max(u32::try_from(s[2]).unwrap_or(0));
        }
    }
}

#[derive(Default)]
struct Hist { from: Option<u64>, n: u32, gap: [u32; 7], feed: [u32; 8], queue: [u32; 5] }
impl Hist {
    fn add(&mut self, frames: &Frames, interval_us: u64, now: u64) {
        self.from.get_or_insert(now);
        let interval = if interval_us > 0 { interval_us } else { 16_667 };
        for s in &frames.samples {
            self.n = self.n.saturating_add(1);
            if let Ok(gap) = u64::try_from(s[0]) { self.gap[GAP_EDGES.iter().position(|edge| gap * 10 < interval * edge).unwrap_or(6)] += 1; }
            if s[3] == 0 { if let Ok(feed) = u64::try_from(s[1]) { self.feed[FEED_EDGES.iter().position(|edge| feed < *edge).unwrap_or(7)] += 1; } }
            self.queue[usize::try_from(s[2]).unwrap_or(0).min(4)] += 1;
        }
    }
}

/// Trouble seen but not yet sent, because the last warning of its kind was recent.
#[derive(Default)]
struct Carry { dropped: [u64; 5], stalls: u32, max_gap: u64, socket_drops: u32, pending: bool }

#[derive(Default)]
pub struct Observer {
    level: Option<Level>,
    session: Option<Tracked>,
    /// The negotiated frame interval, 0 until the stream's format is known.
    interval_us: u64,
    latest: Latest,
    window: Window,
    detail: Window,
    hist: Hist,
    carry: Carry,
    panel: Option<(u64, Reading)>,
    pads: Option<u32>,
    stream_at: Option<u64>,
    frames_at: Option<u64>,
    loss_at: Option<u64>,
    trouble_at: Option<u64>,
    /// The engine's count of packets the TV dropped, as last seen. It runs from the session's start, so what rose is what is new.
    socket_seen: u64,
}

fn due(last: &mut Option<u64>, now: u64) -> bool {
    if last.is_some_and(|at| now.saturating_sub(at) < WARN_MS) { return false; }
    *last = Some(now);
    true
}
fn tenth(value: f32) -> f32 { (value * 10.0).round() / 10.0 }

impl Observer {
    /// A new level starts a clean slate and is announced; the session in progress is announced again, since the queue that held its start was discarded.
    pub fn configure(&mut self, level: Option<Level>, now: u64, out: &mut Vec<Event>) {
        if self.level == level { return; }
        *self = Self { level, ..Self::default() };
        if let Some(level) = level { out.push(Event::Setting { ms: now, level }); }
    }
    fn at_least(&self, level: Level) -> bool { self.level.is_some_and(|on| on >= level) }

    /// Called after every model step with the session as it now stands.
    pub fn session(&mut self, now: u64, seen: Option<Seen>, out: &mut Vec<Event>) {
        let Some(seen) = seen else { self.session = None; return };
        if self.session.as_ref().is_none_or(|t| t.id != seen.id) {
            let witnessed = !matches!(seen.phase, Phase::Running | Phase::Stopping | Phase::Done(_));
            self.session = Some(Tracked { id: seen.id, engine: seen.engine, began: now, over: matches!(seen.phase, Phase::Done(_)), looking: witnessed, ..Tracked::default() });
            self.window = Window::default();
            self.detail = Window::default();
            self.hist = Hist::default();
            self.latest = Latest::default();
            self.interval_us = 0;
            self.socket_seen = 0;
        }
        let Self { session, panel, interval_us, .. } = self;
        let t = session.as_mut().expect("set above");
        if t.over { return; }
        let word = phase_word(Some(seen.phase));
        let (queue, eta_s) = match seen.phase { Phase::Queue { position, eta_s } => (*position, *eta_s), _ => (None, None) };
        if word != "done" && (word != t.phase || queue != t.queue) {
            t.phase = word;
            t.queue = queue;
            out.push(Event::Session { ms: now, session: t.id, engine: t.engine, phase: word, after_ms: now.saturating_sub(t.began), queue, eta_s });
        }
        match seen.phase {
            Phase::Running => {
                t.running.get_or_insert(now);
                if let Some(stream) = seen.stream.filter(|stream| t.format.as_ref() != Some(*stream)) {
                    t.format = Some(stream.clone());
                    *interval_us = if stream.fps > 0 { 1_000_000 / u64::from(stream.fps) } else { 0 };
                    out.push(Event::StreamFormat {
                        ms: now, session: t.id, codec: stream.codec.as_str(), width: stream.width, height: stream.height, fps: stream.fps,
                        // The engines do not report depth: HDR streams are 10-bit and SDR ones 8-bit.
                        bit_depth: if stream.hdr { 10 } else { 8 }, hdr: stream.hdr, want_hdr: seen.want.hdr, sdr: sdr_kind(stream, seen.want.hdr),
                        audio: stream.audio_channels, want_fps: seen.want.fps, want_height: seen.want.height, vui: stream.vui,
                    });
                    let sdr = sdr_kind(stream, seen.want.hdr);
                    if seen.want.hdr && sdr != "none" && !t.missing.contains(&sdr) {
                        t.missing.push(sdr);
                        out.push(Event::HdrMissing { ms: now, session: t.id, engine: t.engine, why: sdr });
                    }
                }
            }
            Phase::Stopping | Phase::Done(_) if t.running.is_some() && t.left.is_none() => {
                t.left = Some(now);
                if let Some((_, reading)) = panel.filter(|(at, _)| now.saturating_sub(*at) <= FRESH_MS) { out.push(reading.event(now, "end")); }
            }
            _ => {}
        }
        if let Phase::Done(outcome) = seen.phase {
            t.over = true;
            let by = seen.ended_by.unwrap_or("unknown");
            let unexpected = matches!(by, "engine" | "engine_exit" | "timeout");
            let code = match outcome { Outcome::Failed { message, .. } => ipc::failure_code(message).map(str::to_owned), _ => None };
            let (outcome, reason, level) = match outcome {
                // An idle end is the far side's policy working as designed, even though the engine is what reported it.
                Outcome::Ended(reason) => ("ended", reason.as_str(), if unexpected && !matches!(reason, ipc::EndReason::Idle) { Level::Warn } else { Level::Info }),
                Outcome::Failed { reason, .. } => ("failed", reason.as_str(), Level::Error),
                Outcome::Cancelled => ("cancelled", "none", Level::Info),
            };
            out.push(Event::SessionEnd { ms: now, session: t.id, engine: t.engine, outcome, reason, by, ran_s: seen.played_s, code, level });
        }
    }

    /// Engine events as they arrive, before the model sees them.
    pub fn engine_event(&mut self, now: u64, engine: &str, event: &ipc::Event, out: &mut Vec<Event>) {
        match event {
            ipc::Event::Stats { stats, .. } => self.stats(now, stats, out),
            ipc::Event::Error { code, .. } => out.push(Event::EngineError { ms: now, engine: engine_word(engine), code: code.as_str() }),
            ipc::Event::RoundTrip { trip, .. } if self.at_least(Level::Trace) => out.push(Event::Trip {
                ms: now, presses: trip.presses, press_p50_ms: trip.press_p50_ms.map(tenth), press_p99_ms: trip.press_p99_ms.map(tenth), network_ms: trip.network_ms.map(tenth),
                host_ms: trip.host_ms.map(tenth), arrival_p50_ms: trip.arrival_p50_ms.map(tenth), arrival_p95_ms: trip.arrival_p95_ms.map(tenth), client_ms: trip.client_ms.map(tenth),
            }),
            _ => {}
        }
    }
    fn stats(&mut self, now: u64, stats: &ipc::Stats, out: &mut Vec<Event>) {
        let pad = stats.pad.as_ref();
        self.latest = Latest { fps: stats.fps.map(tenth), client_ms: stats.client_ms, host_ms: stats.host_ms, pad_bursts: pad.map(|p| p.bursts), pad_gap_us: pad.map(|p| p.burst_gap_us),
            pacing_ms: stats.pacing_ms, host_gap_ms: stats.host_gap_ms, arrival_gap_ms: stats.arrival_gap_ms };
        if self.stream_at.is_none_or(|at| now.saturating_sub(at) >= 1000) {
            self.stream_at = Some(now);
            out.push(Event::Stream {
                ms: now, fps: stats.fps.map(tenth), bitrate_kbps: stats.bitrate_kbps, rtt_ms: stats.rtt_ms.map(tenth), jitter_ms: stats.jitter_ms.map(tenth), loss_pct: stats.loss_pct.map(tenth), dropped: stats.dropped,
                socket_drops: stats.socket_drops, pad_reports: pad.map(|p| p.reports), pad_rate_hz: pad.map(|p| tenth(p.rate_hz)), pad_p99_us: pad.map(|p| p.p99_us),
            });
        }
        if let Some(loss) = stats.loss_pct.filter(|loss| *loss >= LOSS_PCT) {
            if due(&mut self.loss_at, now) { out.push(Event::NetLoss { ms: now, loss_pct: tenth(loss), rtt_ms: stats.rtt_ms.map(tenth), jitter_ms: stats.jitter_ms.map(tenth) }); }
        }
        if let Some(total) = stats.socket_drops {
            let rose = total.saturating_sub(self.socket_seen);
            self.socket_seen = total;
            if rose > 0 {
                self.carry.socket_drops = self.carry.socket_drops.saturating_add(u32::try_from(rose).unwrap_or(u32::MAX));
                self.carry.pending = true;
            }
        }
        self.trouble(now, out);
    }

    /// The feeder's per-frame samples. They are checked for trouble every second, then folded into a pacing line at Debug and a histogram at Trace.
    pub fn frames(&mut self, now: u64, frames: &Frames, out: &mut Vec<Event>) {
        if let Some(t) = self.session.as_mut().filter(|t| t.first_frame.is_none()) {
            if frames.samples.iter().any(|sample| sample[3] == 0) { t.first_frame = Some(now); }
        }
        if self.frames_at.is_none_or(|at| now.saturating_sub(at) >= 1000) {
            self.frames_at = Some(now);
            out.push(Event::Frames { ms: now, fed: frames.fed, accepted: frames.accepted, dropped: frames.dropped, omitted: frames.omitted });
        }
        self.window.add(frames, self.interval_us, now);
        if self.at_least(Level::Debug) { self.detail.add(frames, self.interval_us, now); }
        if self.at_least(Level::Trace) { self.hist.add(frames, self.interval_us, now); }
        if self.window.from.is_some_and(|from| now.saturating_sub(from) >= WINDOW_MS) { self.flush_window(now, out); }
        if let Some(from) = self.detail.from.filter(|from| now.saturating_sub(*from) >= DETAIL_MS) { self.flush_detail(from, now, out); }
        if let Some(from) = self.hist.from.filter(|from| now.saturating_sub(*from) >= HIST_MS) {
            let h = std::mem::replace(&mut self.hist, Hist { from: Some(now), ..Hist::default() });
            if h.n > 0 { out.push(Event::Hist { ms: now, span_ms: u32::try_from(now.saturating_sub(from)).unwrap_or(u32::MAX), gap: h.gap, feed: h.feed, queue: h.queue }); }
        }
    }
    fn flush_window(&mut self, now: u64, out: &mut Vec<Event>) {
        let w = std::mem::replace(&mut self.window, Window { from: Some(now), ..Window::default() });
        if w.n == 0 { return; }
        if w.drops.iter().sum::<u64>() >= DROP_BURST || w.stalls > 0 {
            for (carried, dropped) in self.carry.dropped.iter_mut().zip(w.drops) { *carried += dropped; }
            self.carry.stalls += w.stalls;
            self.carry.max_gap = self.carry.max_gap.max(w.max_gap);
            self.carry.pending = true;
        }
        self.trouble(now, out);
    }
    /// Sends the trouble carried, unless the last report went out within `WARN_MS`: what is held back goes with the next one.
    fn trouble(&mut self, now: u64, out: &mut Vec<Event>) {
        if self.carry.pending && due(&mut self.trouble_at, now) {
            let c = std::mem::take(&mut self.carry);
            out.push(Event::FrameTrouble { ms: now, dropped: c.dropped, stalls: c.stalls, max_gap_us: c.max_gap, socket_drops: c.socket_drops });
        }
    }
    fn flush_detail(&mut self, from: u64, now: u64, out: &mut Vec<Event>) {
        let w = std::mem::replace(&mut self.detail, Window { from: Some(now), ..Window::default() });
        if w.n == 0 { return; }
        let panel = self.panel.filter(|(at, _)| now.saturating_sub(*at) <= FRESH_MS).map(|(_, reading)| reading);
        let bursts = self.latest.pad_bursts.filter(|n| *n > 0);
        out.push(Event::Detail {
            ms: now, fps: tenth(w.n as f32 * 1000.0 / now.saturating_sub(from) as f32), gap_us: percentiles(w.gaps).unwrap_or_default(), feed_us: percentiles(w.feeds).unwrap_or_default(),
            queue: w.queue, late: w.late, stalls: w.stalls, client_ms: self.latest.client_ms.map(tenth), host_ms: self.latest.host_ms.map(tenth),
            pacing_ms: self.latest.pacing_ms.map(tenth), host_gap_ms: self.latest.host_gap_ms.map(|g| g.map(tenth)),
            arrival_gap_ms: self.latest.arrival_gap_ms.map(|g| g.map(tenth)), pad_bursts: bursts,
            pad_gap_us: bursts.and(self.latest.pad_gap_us), hz: panel.and_then(|r| r.hz), min_hz: panel.and_then(|r| r.min_hz), fewest: panel.and_then(|r| r.fewest),
        });
    }

    /// The panel's answer: the first picture, the start state once the stream has settled, then any change in it.
    pub fn display(&mut self, now: u64, value: &Value, out: &mut Vec<Event>) {
        let Some(reading) = Reading::of(value) else { return };
        self.panel = Some((now, reading));
        let Some(t) = self.session.as_mut() else { return };
        let Some(running) = t.running.filter(|_| t.left.is_none()) else { return };
        // Before the feeder has taken a frame, a window that looks fine can only be what the last stream left behind.
        if let Some(frame) = t.first_frame.filter(|_| t.looking && reading.shows_picture()) {
            t.looking = false;
            out.push(Event::FirstPicture { ms: now, session: t.id, after_running_ms: now.saturating_sub(running), after_frame_ms: now.saturating_sub(frame) });
        }
        if !t.start_read {
            if now >= running + SETTLE_MS {
                t.start_read = true;
                t.shown = Some(reading);
                t.changed = now;
                out.push(reading.event(now, "start"));
            }
        } else if t.shown.is_some_and(|shown| reading.differs(&shown)) && now >= t.changed + CHANGE_MS {
            t.shown = Some(reading);
            t.changed = now;
            out.push(reading.event(now, "change"));
        }
    }

    /// Reports a stream whose panel shows no picture, and closes the watch on the panel after a session: the first reading taken after it stopped running.
    pub fn tick(&mut self, now: u64, out: &mut Vec<Event>) {
        let Self { session, panel, latest, .. } = self;
        let Some(t) = session.as_mut() else { return };
        // Judged on a reading from this stream that is still current; with none, the TV's state is unknown and nothing is claimed.
        if let Some(running) = t.running.filter(|running| t.looking && !t.warned && t.left.is_none() && now >= running + PICTURE_MS) {
            if let Some((_, window)) = panel.filter(|(at, _)| *at >= running && now.saturating_sub(*at) <= FRESH_MS) {
                t.warned = true;
                out.push(Event::NoPicture { ms: now, session: t.id, muted: window.muted, width: window.width, height: window.height, src_hz: window.src_hz, fps: latest.fps });
            }
        }
        let Some(left) = t.left.filter(|left| !t.after_read && now >= left + TAIL_MS) else { return };
        t.after_read = true;
        if let Some((_, reading)) = panel.filter(|(at, _)| *at > left && now.saturating_sub(*at) <= FRESH_MS) { out.push(reading.event(now, "after_end")); }
    }

    /// What the shell should ask the helper's display endpoint for: once a second while a stream runs and for the tail after it. At Warn only the
    /// video window is read, and only until a picture shows or is reported missing.
    pub fn sample(&self, now: u64) -> Option<Sample> {
        let t = self.session.as_ref()?;
        let running = t.running.is_some() && t.left.is_none();
        if self.at_least(Level::Info) { return (running || t.left.is_some_and(|left| now < left + TAIL_MS + 1000)).then_some(Sample { every_ms: 1000, full: true }); }
        (self.at_least(Level::Warn) && running && t.looking && !t.warned).then_some(Sample { every_ms: 1000, full: false })
    }

    /// Controllers as a bit per slot. A change is reported; the first look announces those already connected.
    pub fn pads(&mut self, now: u64, mask: u32, out: &mut Vec<Event>) {
        if self.pads == Some(mask) { return; }
        let before = self.pads.replace(mask).unwrap_or(0);
        for slot in 0..32u8 {
            let bit = 1u32 << slot;
            if (mask ^ before) & bit != 0 { out.push(Event::Controller { ms: now, slot, connected: mask & bit != 0 }); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::diagnostics::{BATCH_LIMIT, Diagnostics, assert_allowlisted};
    use lumagame_core::{engine::Codec, ids::SessionId, ipc::{EndReason, ErrorCode, FailReason, PadStats, RoundTrip}};
    use serde_json::json;
    use std::collections::BTreeSet;

    const WANT: Want = Want { hdr: true, fps: 60, height: 2160 };

    fn negotiated(hdr: bool, sdr_reason: Option<&str>) -> StreamInfo {
        StreamInfo { codec: Codec::H265, width: 3840, height: 2160, fps: 60, hdr, audio_channels: 6, sdr_reason: sdr_reason.map(str::to_owned), vui: None }
    }
    fn seen<'a>(phase: &'a Phase, stream: Option<&'a StreamInfo>, ended_by: Option<&'static str>) -> Seen<'a> {
        Seen { id: 7, engine: "gfn", phase, stream, ended_by, played_s: Some(95), want: WANT }
    }
    fn observer(level: Level) -> Observer {
        let mut o = Observer::default();
        o.configure(Some(level), 0, &mut Vec::new());
        o
    }
    /// An observer whose session has been running at 60 fps since 0.
    fn running(level: Level) -> Observer {
        let mut o = observer(level);
        o.session(0, Some(seen(&Phase::Running, Some(&negotiated(true, None)), None)), &mut Vec::new());
        o
    }
    fn json_of(out: &[Event]) -> Vec<Value> { out.iter().map(|event| serde_json::to_value(event).unwrap()).collect() }
    fn names(out: &[Event]) -> Vec<String> { json_of(out).iter().map(|v| format!("{}@{}", v["event"].as_str().unwrap(), v["ms"])).collect() }
    fn only(out: &[Event], name: &str) -> Vec<Value> { json_of(out).into_iter().filter(|v| v["event"] == name).collect() }
    fn ms_of(event: &Event) -> u64 { serde_json::to_value(event).unwrap()["ms"].as_u64().unwrap() }
    fn frames(fed: u64, samples: Vec<[i64; 4]>) -> Frames { Frames { fed, accepted: fed, samples, ..Frames::default() } }
    fn calm(n: usize) -> Vec<[i64; 4]> { vec![[16_666, 300, 1, 0]; n] }
    fn panel(hdr_type: u64, hz: f64, vrr: Option<bool>) -> Value {
        let mut value = json!({"window": {"width": 3840, "height": 2160, "hdrType": hdr_type, "muted": false, "lowDelay": true, "sourceHz": 60.0}, "refresh": {"hz": hz, "minHz": hz - 0.5, "fewest": 3}});
        if let Some(vrr) = vrr { value["firmware"] = json!({"sysVeri10": {"vrr": vrr}}); }
        value
    }
    /// The video window as the helper reports it, with the given mute state and width.
    fn video(muted: bool, width: u32) -> Value {
        let mut value = panel(0, 60.0, None);
        value["window"]["muted"] = json!(muted);
        value["window"]["width"] = json!(width);
        value
    }
    /// An observer that saw the stream start (at 500) and run (at 1000), as one watching from the first phase does.
    fn started(level: Level) -> Observer {
        let mut o = observer(level);
        o.session(500, Some(seen(&Phase::Starting, None, None)), &mut Vec::new());
        o.session(1000, Some(seen(&Phase::Running, Some(&negotiated(true, None)), None)), &mut Vec::new());
        o
    }

    /// A stream as the shell reports it, with hostile text in every field an engine or the TV fills, so a leak shows in the output.
    fn busy_stream(level: Level, secs: u64) -> Vec<Event> {
        let mut o = Observer::default();
        let mut out = Vec::new();
        o.configure(Some(level), 0, &mut out);
        let sdr = negotiated(false, Some("SDR: token=abc https://secret.example/"));
        o.session(0, Some(seen(&Phase::Starting, None, None)), &mut out);
        o.session(0, Some(seen(&Phase::Running, Some(&sdr), None)), &mut out);
        o.pads(0, 0b11, &mut out);
        let stats = ipc::Stats {
            rtt_ms: Some(12.345_678), jitter_ms: Some(1.234_567), loss_pct: Some(2.5), bitrate_kbps: Some(40_000), fps: Some(59.94), dropped: Some(0), socket_drops: Some(0), client_ms: Some(2.345_678), host_ms: Some(6.5),
            pad: Some(PadStats { name: "Alice's Pad".into(), reports: 120, rate_hz: 125.123_45, p99_us: 9_000, bursts: 2, burst_gap_us: 30_000, age_ms: 5 }),
            pacing_ms: None, host_gap_ms: None, arrival_gap_ms: None,
        };
        let trip = RoundTrip { presses: Some(12), press_p50_ms: Some(1.234_567), press_p99_ms: Some(4.5), network_ms: Some(9.0), host_ms: Some(6.5), arrival_p50_ms: Some(14.0), arrival_p95_ms: Some(20.0), client_ms: Some(2.5) };
        for ms in (0..secs * 1000).step_by(100) {
            let n = ms / 100;
            if n % 2 == 0 { o.engine_event(ms, "Alice's Engine", &ipc::Event::Stats { session: SessionId(7), stats: stats.clone() }, &mut out); }
            let mut samples = vec![[16_666 + (n % 5) as i64 * 100, 300 + (n % 7) as i64 * 40, 1 + (n % 3) as i64, 0]; 6];
            if n % 50 == 25 { samples[2] = [150_000, 900, 3, 0]; samples[3] = [16_666, 0, 2, 1]; }
            o.frames(ms, &frames(n * 6, samples), &mut out);
            if ms % 1000 == 0 {
                out.push(Event::Render { ms, frames: 60, max_cpu_us: 9000, max_draw_us: 4000 });
                o.display(ms, &panel(1, 60.0, Some(false)), &mut out);
            }
            if ms == 1000 {
                o.engine_event(ms, "Alice's Engine", &ipc::Event::Error { code: ErrorCode::Internal, message: "Alice's secret".into() }, &mut out);
                o.engine_event(ms, "gfn", &ipc::Event::RoundTrip { session: SessionId(7), trip: trip.clone() }, &mut out);
            }
            if ms == 1500 { o.pads(ms, 0b01, &mut out); }
        }
        let end = Phase::Done(Outcome::Failed { reason: FailReason::Video, message: "https://secret.example/?token=abc".into() });
        o.session(secs * 1000, Some(seen(&end, Some(&sdr), Some("engine"))), &mut out);
        out
    }

    #[test]
    fn a_session_is_reported_by_phase_with_its_format_and_the_way_it_ended() {
        let mut o = observer(Level::Info);
        let mut out = Vec::new();
        let sdr = negotiated(false, Some("SDR: no HDR for this game"));
        let steps = [
            (1000, Phase::Checking, None),
            (1100, Phase::Queue { position: Some(3), eta_s: Some(40) }, None),
            (1200, Phase::Queue { position: Some(3), eta_s: Some(35) }, None),
            (1300, Phase::Queue { position: Some(2), eta_s: None }, None),
            (1400, Phase::Starting, None),
            (5000, Phase::Running, Some(&sdr)),
            (5100, Phase::Running, Some(&sdr)),
            (9000, Phase::Stopping, Some(&sdr)),
        ];
        for (now, phase, stream) in &steps { o.session(*now, Some(seen(phase, *stream, None)), &mut out); }
        for now in [9500, 9600] { o.session(now, Some(seen(&Phase::Done(Outcome::Ended(EndReason::Stopped)), Some(&sdr), Some("owner"))), &mut out); }
        let phase = |ms: u64, phase: &str, after_ms: u64| json!({"event": "session", "ms": ms, "session": 7, "engine": "gfn", "phase": phase, "after_ms": after_ms});
        let mut queued = phase(1100, "queue", 100);
        queued["queue"] = json!(3);
        queued["eta_s"] = json!(40);
        let mut moved_up = phase(1300, "queue", 300);
        moved_up["queue"] = json!(2);
        assert_eq!(json_of(&out), vec![
            phase(1000, "checking", 0),
            queued,
            moved_up,
            phase(1400, "starting", 400),
            phase(5000, "running", 4000),
            json!({"event": "stream_format", "ms": 5000, "session": 7, "codec": "h265", "width": 3840, "height": 2160, "fps": 60, "bit_depth": 8, "hdr": false, "want_hdr": true, "sdr": "game_unsupported", "audio": 6, "want_fps": 60, "want_height": 2160}),
            json!({"event": "hdr_missing", "ms": 5000, "session": 7, "engine": "gfn", "why": "game_unsupported"}),
            phase(9000, "stopping", 8000),
            json!({"event": "session_end", "ms": 9500, "session": 7, "engine": "gfn", "outcome": "ended", "reason": "stopped", "by": "owner", "ran_s": 95}),
        ]);
    }

    #[test]
    fn how_a_session_ended_sets_the_level_it_is_reported_at() {
        let cases = [
            (Outcome::Failed { reason: FailReason::Network, message: "https://secret.example/?token=abc".into() }, "engine", Level::Error, "failed", "network"),
            (Outcome::Failed { reason: FailReason::Video, message: String::new() }, "owner", Level::Error, "failed", "video"),
            (Outcome::Ended(EndReason::Stopped), "owner", Level::Info, "ended", "stopped"),
            (Outcome::Ended(EndReason::Suspended), "background", Level::Info, "ended", "suspended"),
            (Outcome::Ended(EndReason::Remote), "app_close", Level::Info, "ended", "remote"),
            (Outcome::Ended(EndReason::Remote), "engine", Level::Warn, "ended", "remote"),
            (Outcome::Ended(EndReason::Remote), "engine_exit", Level::Warn, "ended", "remote"),
            (Outcome::Ended(EndReason::Remote), "timeout", Level::Warn, "ended", "remote"),
            (Outcome::Ended(EndReason::Idle), "engine", Level::Info, "ended", "idle"),
            (Outcome::Cancelled, "owner", Level::Info, "cancelled", "none"),
        ];
        for (outcome, by, level, word, reason) in cases {
            let mut o = running(Level::Trace);
            let mut out = Vec::new();
            o.session(10, Some(seen(&Phase::Done(outcome), None, Some(by))), &mut out);
            let [end] = out.as_slice() else { panic!("{by} {reason}: {} events", out.len()) };
            let line = serde_json::to_string(end).unwrap();
            let value: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(end.level(), level, "{line}");
            assert_eq!((value["outcome"].as_str(), value["reason"].as_str(), value["by"].as_str()), (Some(word), Some(reason), Some(by)), "{line}");
            assert!(!line.contains("secret"), "{line}");
        }
    }

    #[test]
    fn a_failed_session_carries_the_engines_error_code_and_nothing_else_of_its_message() {
        let mut o = running(Level::Error);
        let mut out = Vec::new();
        let message = ipc::failure_text("Alice's secret https://example/?token=abc", "session_update_busy");
        o.session(10, Some(seen(&Phase::Done(Outcome::Failed { reason: FailReason::Engine, message }), None, Some("engine"))), &mut out);
        let line = serde_json::to_string(&out[0]).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["code"], "session_update_busy");
        assert_allowlisted(&line);
        for leak in ["secret", "Alice", "example", "token", "http"] { assert!(!line.contains(leak), "{leak} in {line}"); }
    }

    #[test]
    fn a_finished_session_is_not_announced_again_and_the_next_one_is() {
        let mut o = observer(Level::Info);
        let mut out = Vec::new();
        o.session(10, Some(seen(&Phase::Done(Outcome::Ended(EndReason::Stopped)), None, Some("owner"))), &mut out);
        assert!(out.is_empty());
        o.session(20, Some(Seen { id: 8, ..seen(&Phase::Checking, None, None) }), &mut out);
        assert_eq!(names(&out), ["session@20"]);
    }

    #[test]
    fn a_new_level_announces_itself_and_the_session_in_progress() {
        let mut o = Observer::default();
        let mut out = Vec::new();
        let hdr = negotiated(true, None);
        o.configure(Some(Level::Info), 5, &mut out);
        o.session(100, Some(seen(&Phase::Running, Some(&hdr), None)), &mut out);
        assert_eq!(names(&out), ["level@5", "session@100", "stream_format@100"]);
        out.clear();
        o.configure(Some(Level::Info), 200, &mut out);
        assert!(out.is_empty());
        o.configure(Some(Level::Debug), 300, &mut out);
        o.session(300, Some(seen(&Phase::Running, Some(&hdr), None)), &mut out);
        assert_eq!(names(&out), ["level@300", "session@300", "stream_format@300"]);
        out.clear();
        o.configure(None, 400, &mut out);
        assert!(out.is_empty() && o.sample(400).is_none());
    }

    #[test]
    fn the_engines_wording_is_reduced_to_fixed_kinds() {
        assert_eq!(sdr_kind(&negotiated(true, None), true), "none");
        assert_eq!(sdr_kind(&negotiated(false, None), false), "not_requested");
        assert_eq!(sdr_kind(&negotiated(false, Some("SDR: no HDR for this game")), true), "game_unsupported");
        assert_eq!(sdr_kind(&negotiated(false, Some("SDR: GeForce NOW sent no HDR")), true), "service_sent_sdr");
        assert_eq!(sdr_kind(&negotiated(false, Some("SDR: the TV refused")), true), "other");
        assert_eq!(sdr_kind(&negotiated(false, None), true), "unreported");
        assert_eq!([engine_word("gfn"), engine_word("punktfunk"), engine_word("Alice's Engine")], ["gfn", "punktfunk", "other"]);
        assert_eq!([phase_word(None), phase_word(Some(&Phase::SignIn)), phase_word(Some(&Phase::Queue { position: None, eta_s: None }))], ["none", "sign-in", "queue"]);
    }

    fn tagged(hdr: bool, sdr_reason: Option<&str>, [primaries, transfer, matrix]: [u8; 3]) -> StreamInfo {
        StreamInfo { vui: Some(ipc::Vui { primaries, transfer, matrix, full_range: false }), ..negotiated(hdr, sdr_reason) }
    }
    /// What a session logs when it is shown these streams in turn, while HDR is or is not wanted.
    fn shown(want_hdr: bool, streams: &[StreamInfo]) -> Vec<Event> {
        let mut o = observer(Level::Info);
        let mut out = Vec::new();
        for (n, stream) in streams.iter().enumerate() {
            o.session(1000 + n as u64, Some(Seen { want: Want { hdr: want_hdr, ..WANT }, ..seen(&Phase::Running, Some(stream), None) }), &mut out);
        }
        out
    }
    fn why(out: &[Event]) -> Vec<String> { only(out, "hdr_missing").iter().map(|v| v["why"].as_str().unwrap().to_owned()).collect() }

    #[test]
    fn the_streams_own_colour_tags_decide_whether_declared_hdr_arrived() {
        let cases = [
            ("PQ", true, tagged(true, None, [9, 16, 9]), "none", None),
            ("HLG", true, tagged(true, None, [9, 18, 9]), "none", None),
            ("BT.709", true, tagged(true, None, [1, 1, 1]), "service_sent_sdr", Some("service_sent_sdr")),
            ("unspecified", true, tagged(true, None, [2, 2, 2]), "service_sent_sdr", Some("service_sent_sdr")),
            ("not wanted", false, tagged(true, None, [1, 1, 1]), "none", None),
            ("no tags", true, negotiated(true, None), "none", None),
        ];
        for (name, want, stream, sdr, missing) in cases {
            let out = shown(want, std::slice::from_ref(&stream));
            let [format] = only(&out, "stream_format").try_into().unwrap_or_else(|_| panic!("{name}: one stream_format"));
            assert_eq!((format["sdr"].as_str(), format["hdr"].as_bool(), format["vui"] != Value::Null), (Some(sdr), Some(true), stream.vui.is_some()), "{name}");
            assert_eq!(why(&out), missing.into_iter().collect::<Vec<_>>(), "{name}");
        }
    }

    #[test]
    fn a_stream_format_with_tags_carries_them_and_the_warning_is_the_same_line_as_before() {
        let out = shown(true, &[tagged(true, None, [9, 16, 9])]);
        assert_eq!(json_of(&out)[1], json!({"event": "stream_format", "ms": 1000, "session": 7, "codec": "h265", "width": 3840, "height": 2160, "fps": 60, "bit_depth": 10, "hdr": true, "want_hdr": true, "sdr": "none", "audio": 6, "want_fps": 60, "want_height": 2160,
            "vui": {"primaries": 9, "transfer": 16, "matrix": 9, "full_range": false}}));
        let out = shown(true, &[tagged(true, None, [1, 1, 1])]);
        assert_eq!(json_of(&out)[2], json!({"event": "hdr_missing", "ms": 1000, "session": 7, "engine": "gfn", "why": "service_sent_sdr"}));
        assert_allowlisted(&serde_json::to_string(&out[1]).unwrap());
    }

    #[test]
    fn colour_tags_that_arrive_after_the_first_line_never_repeat_a_warning() {
        // Declared SDR for a service that sent none: the tags change nothing, the reason was already given.
        let sdr = negotiated(false, Some("SDR: GeForce NOW sent no HDR"));
        let out = shown(true, &[sdr.clone(), StreamInfo { vui: Some(ipc::Vui { primaries: 1, transfer: 1, matrix: 1, full_range: false }), ..sdr }]);
        assert_eq!((only(&out, "stream_format").len(), why(&out)), (2, vec!["service_sent_sdr".to_owned()]));
        // Declared HDR that the tags then contradict: warned once, when they arrive.
        let out = shown(true, &[negotiated(true, None), tagged(true, None, [1, 1, 1]), tagged(true, None, [1, 1, 1])]);
        assert_eq!((only(&out, "stream_format").len(), why(&out)), (2, vec!["service_sent_sdr".to_owned()]));
        assert_eq!(names(&out).iter().position(|n| n.starts_with("hdr_missing")), Some(names(&out).len() - 1));
    }

    #[test]
    fn engine_stats_make_one_stream_line_a_second_and_loss_warns_at_most_every_five_seconds() {
        let mut o = observer(Level::Info);
        let mut out = Vec::new();
        for (now, loss) in [(0, 0.2), (400, 0.2), (1000, 2.5), (2000, 3.0), (5999, 4.0), (6000, 1.0), (7000, 0.9)] {
            let stats = ipc::Stats { loss_pct: Some(loss), rtt_ms: Some(18.04), jitter_ms: Some(3.0), ..ipc::Stats::default() };
            o.engine_event(now, "gfn", &ipc::Event::Stats { session: SessionId(7), stats }, &mut out);
        }
        assert_eq!(names(&out), ["stream@0", "stream@1000", "net_loss@1000", "stream@2000", "stream@5999", "net_loss@6000", "stream@7000"]);
        assert_eq!(json_of(&out)[2], json!({"event": "net_loss", "ms": 1000, "loss_pct": 2.5, "rtt_ms": 18.0, "jitter_ms": 3.0}));
    }

    #[test]
    fn packets_the_tv_dropped_ride_the_stream_line_and_make_frame_trouble_at_most_every_five_seconds() {
        let mut o = observer(Level::Info);
        let mut out = Vec::new();
        // The engine counts from the session's start, so each reading is a total and what rose since the last one is what is new.
        for (now, total) in [(0, Some(0)), (1000, Some(4)), (2000, Some(9)), (3000, Some(9)), (6000, Some(9)), (7000, None)] {
            let stats = ipc::Stats { socket_drops: total, ..ipc::Stats::default() };
            o.engine_event(now, "punktfunk", &ipc::Event::Stats { session: SessionId(7), stats }, &mut out);
        }
        let totals: Vec<_> = only(&out, "stream").iter().map(|line| line["socket_drops"].as_u64()).collect();
        assert_eq!(totals, [Some(0), Some(4), Some(9), Some(9), Some(9), None]);
        // The five that rose at 2000 wait out the five-second gap and go with the next report.
        assert_eq!(only(&out, "frame_trouble"), vec![
            json!({"event": "frame_trouble", "ms": 1000, "dropped": [0, 0, 0, 0, 0], "stalls": 0, "max_gap_us": 0, "socket_drops": 4}),
            json!({"event": "frame_trouble", "ms": 6000, "dropped": [0, 0, 0, 0, 0], "stalls": 0, "max_gap_us": 0, "socket_drops": 5}),
        ]);
    }

    #[test]
    fn engine_errors_and_round_trips_carry_only_fixed_words_and_numbers() {
        let mut o = observer(Level::Trace);
        let mut out = Vec::new();
        o.engine_event(10, "Alice's Engine", &ipc::Event::Error { code: ErrorCode::BadRequest, message: "/home/alice/secret.json".into() }, &mut out);
        let trip = RoundTrip { presses: Some(12), press_p50_ms: Some(1.04), network_ms: Some(9.0), ..RoundTrip::default() };
        o.engine_event(20, "gfn", &ipc::Event::RoundTrip { session: SessionId(7), trip }, &mut out);
        assert_eq!(json_of(&out), vec![
            json!({"event": "engine_error", "ms": 10, "engine": "other", "code": "bad-request"}),
            json!({"event": "trip", "ms": 20, "presses": 12, "press_p50_ms": 1.0, "network_ms": 9.0}),
        ]);
    }

    #[test]
    fn frames_are_checked_for_trouble_each_second_and_folded_into_one_pacing_line_every_two() {
        let mut o = running(Level::Debug);
        let mut out = Vec::new();
        let mut first = calm(29);
        first.insert(0, [-1, 300, 1, 0]);
        let mut rough = calm(29);
        rough.extend([[40_000, 900, 3, 0], [16_666, 0, 2, 1], [130_000, 500, 2, 0]]);
        o.frames(1000, &frames(30, first), &mut out);
        o.frames(1500, &frames(62, rough), &mut out);
        assert_eq!(names(&out), ["frames@1000"]);
        o.frames(2000, &frames(72, calm(10)), &mut out);
        assert_eq!(names(&out), ["frames@1000", "frames@2000", "frame_trouble@2000"]);
        o.frames(3000, &frames(82, calm(10)), &mut out);
        assert_eq!(names(&out), ["frames@1000", "frames@2000", "frame_trouble@2000", "frames@3000", "detail@3000"]);
        let shown = json_of(&out);
        assert_eq!(shown[2], json!({"event": "frame_trouble", "ms": 2000, "dropped": [1, 0, 0, 0, 0], "stalls": 1, "max_gap_us": 130_000}));
        assert_eq!(shown[4], json!({
            "event": "detail", "ms": 3000, "fps": 41.0, "gap_us": [16_666, 16_666, 130_000, 130_000], "feed_us": [300, 300, 900, 900], "queue": 3, "late": 2, "stalls": 1,
        }));
    }

    #[test]
    fn the_pacing_line_carries_the_latest_engine_and_panel_readings() {
        let mut o = running(Level::Debug);
        let mut out = Vec::new();
        o.display(900, &panel(1, 60.0, None), &mut out);
        let stats = ipc::Stats { client_ms: Some(2.5), host_ms: Some(6.5), pad: Some(PadStats { bursts: 2, burst_gap_us: 30_000, ..PadStats::default() }),
            pacing_ms: Some(2.5), host_gap_ms: Some([8.5, 9.0]), arrival_gap_ms: Some([4.5, 16.5]), ..ipc::Stats::default() };
        o.engine_event(950, "gfn", &ipc::Event::Stats { session: SessionId(7), stats }, &mut out);
        out.clear();
        o.frames(1000, &frames(10, calm(10)), &mut out);
        o.frames(3000, &frames(20, calm(10)), &mut out);
        assert_eq!(only(&out, "detail"), vec![json!({
            "event": "detail", "ms": 3000, "fps": 10.0, "gap_us": [16_666, 16_666, 16_666, 16_666], "feed_us": [300, 300, 300, 300], "queue": 1,
            "client_ms": 2.5, "host_ms": 6.5, "pacing_ms": 2.5, "host_gap_ms": [8.5, 9.0], "arrival_gap_ms": [4.5, 16.5], "pad_bursts": 2, "pad_gap_us": 30_000,
            "hz": 60.0, "min_hz": 59.5, "fewest": 3,
        })]);
    }

    #[test]
    fn five_dropped_frames_in_a_window_are_noise_and_six_are_a_burst() {
        for (drops, bursts) in [(5, 0), (6, 1)] {
            let mut o = running(Level::Warn);
            let mut out = Vec::new();
            let mut dropping = calm(10);
            for sample in dropping.iter_mut().take(drops) { sample[3] = 3; }
            o.frames(1000, &frames(10, calm(10)), &mut out);
            o.frames(2000, &frames(20, dropping), &mut out);
            let trouble = only(&out, "frame_trouble");
            assert_eq!(trouble.len(), bursts, "{drops} drops");
            if bursts == 1 { assert_eq!(trouble[0]["dropped"], json!([0, 0, 6, 0, 0])); }
        }
    }

    #[test]
    fn frame_trouble_is_sent_at_most_every_five_seconds_and_carries_what_was_held_back() {
        let mut o = running(Level::Warn);
        let mut out = Vec::new();
        // Each pair of messages makes one window, and the second message of a pair holds that window's stalls.
        for (start, stalls, gap) in [(0, 1, 150_000), (1500, 2, 200_000), (3000, 1, 120_000), (5000, 0, 0)] {
            o.frames(start, &frames(1, calm(5)), &mut out);
            let mut samples = calm(5);
            for sample in samples.iter_mut().take(stalls) { sample[0] = gap; }
            o.frames(start + 1000, &frames(2, samples), &mut out);
        }
        assert_eq!(only(&out, "frame_trouble"), vec![
            json!({"event": "frame_trouble", "ms": 1000, "dropped": [0, 0, 0, 0, 0], "stalls": 1, "max_gap_us": 150_000}),
            json!({"event": "frame_trouble", "ms": 6000, "dropped": [0, 0, 0, 0, 0], "stalls": 3, "max_gap_us": 200_000}),
        ]);
    }

    #[test]
    fn trace_folds_frame_timing_into_a_histogram_every_six_seconds() {
        let samples = vec![
            [-1, 100, 0, 0], [8_000, 300, 1, 0], [14_000, 700, 1, 0], [16_666, 1500, 2, 0], [16_666, 3000, 4, 0],
            [16_666, 6000, 7, 0], [20_000, 12_000, 0, 0], [30_000, 20_000, 1, 0], [40_000, 0, 1, 2], [60_000, 400, 1, 0],
        ];
        let mut trace = running(Level::Trace);
        let mut out = Vec::new();
        trace.frames(0, &frames(10, samples.clone()), &mut out);
        trace.frames(6000, &frames(10, Vec::new()), &mut out);
        trace.frames(12_000, &frames(10, Vec::new()), &mut out);
        assert_eq!(only(&out, "hist"), vec![json!({"event": "hist", "ms": 6000, "span_ms": 6000, "gap": [1, 1, 3, 1, 1, 1, 1], "feed": [1, 2, 1, 1, 1, 1, 1, 1], "queue": [2, 5, 1, 0, 2]})]);
        let mut debug = running(Level::Debug);
        let mut out = Vec::new();
        debug.frames(0, &frames(10, samples), &mut out);
        debug.frames(6000, &frames(10, Vec::new()), &mut out);
        assert!(only(&out, "hist").is_empty());
    }

    #[test]
    fn a_flood_of_samples_cannot_grow_the_window() {
        let mut o = running(Level::Debug);
        for _ in 0..5 { o.frames(100, &frames(256, calm(256)), &mut Vec::new()); }
        assert_eq!((o.window.gaps.len(), o.window.feeds.len(), o.window.n), (CAP, CAP, 1280));
        assert_eq!((o.detail.gaps.len(), o.detail.feeds.len(), o.detail.n), (CAP, CAP, 1280));
    }

    #[test]
    fn the_panel_is_reported_at_the_start_on_change_and_after_the_stream_ends() {
        let mut o = observer(Level::Info);
        let mut out = Vec::new();
        let hdr = negotiated(true, None);
        o.session(1000, Some(seen(&Phase::Running, Some(&hdr), None)), &mut out);
        out.clear();
        for odd in [Value::Null, json!({"window": null}), json!({"error": "forbidden"})] { o.display(1500, &odd, &mut out); }
        assert!(out.is_empty() && o.panel.is_none());
        // Too early to be the stream's state, then steady, a refresh counter wobble, an HDR drop, a rate change inside the hold-off, and one after it.
        let readings = [(1500, panel(1, 60.0, Some(false))), (3000, panel(1, 60.0, Some(false))), (4000, panel(1, 60.0, Some(false))), (5000, panel(1, 59.0, Some(false))), (6000, panel(0, 60.0, Some(false))), (6500, panel(0, 120.0, Some(false))), (8000, panel(0, 120.0, Some(false)))];
        for (now, reading) in &readings { o.display(*now, reading, &mut out); }
        o.session(9000, Some(seen(&Phase::Stopping, Some(&hdr), None)), &mut out);
        o.display(10_000, &panel(0, 60.0, Some(true)), &mut out);
        for now in [11_900, 12_000, 12_500] { o.tick(now, &mut out); }
        let shown = json_of(&out);
        let at: Vec<(&str, u64)> = shown.iter().filter(|v| v["event"] == "display").map(|v| (v["at"].as_str().unwrap(), v["ms"].as_u64().unwrap())).collect();
        assert_eq!(at, [("start", 3000), ("change", 6000), ("change", 8000), ("end", 9000), ("after_end", 12_000)]);
        assert_eq!(shown[0], json!({"event": "display", "ms": 3000, "at": "start", "width": 3840, "height": 2160, "hdr_type": 1, "muted": false, "hz": 60.0, "src_hz": 60.0, "vrr": false, "low_delay": true}));
        assert_eq!(shown[4], json!({"event": "display", "ms": 9000, "at": "end", "width": 3840, "height": 2160, "hdr_type": 0, "muted": false, "hz": 120.0, "src_hz": 60.0, "vrr": false, "low_delay": true}));
        assert_eq!(shown[5], json!({"event": "display", "ms": 12_000, "at": "after_end", "width": 3840, "height": 2160, "hdr_type": 0, "muted": false, "hz": 60.0, "src_hz": 60.0, "vrr": true, "low_delay": true}));
    }

    #[test]
    fn the_first_picture_is_the_first_sized_unmuted_window_after_the_feeder_took_a_frame() {
        let mut o = started(Level::Info);
        let mut out = Vec::new();
        // Before a frame the window is only what the last stream left; after it, a muted or an empty one is no picture either.
        o.display(1200, &video(false, 3840), &mut out);
        o.frames(1500, &frames(3, calm(3)), &mut out);
        for (now, reading) in [(2000, video(true, 3840)), (2500, video(false, 0)), (3000, video(false, 3840)), (4000, video(false, 3840)), (6000, video(true, 3840))] { o.display(now, &reading, &mut out); }
        assert_eq!(only(&out, "first_picture"), vec![json!({"event": "first_picture", "ms": 3000, "session": 7, "after_running_ms": 2000, "after_frame_ms": 1500})]);
        // Every reading says whether the window is muted, and a mute coming on is a change of the panel.
        let shown: Vec<(String, Option<bool>)> = only(&out, "display").iter().map(|v| (v["at"].as_str().unwrap().to_owned(), v["muted"].as_bool())).collect();
        assert_eq!(shown, [("start".to_owned(), Some(false)), ("change".to_owned(), Some(true))]);
        o.tick(60_000, &mut out);
        assert!(only(&out, "no_picture").is_empty());
    }

    #[test]
    fn no_picture_five_seconds_after_running_is_warned_once_with_the_window_as_it_stood() {
        let mut o = started(Level::Info);
        let mut out = Vec::new();
        let stats = ipc::Stats { fps: Some(59.52), ..ipc::Stats::default() };
        o.engine_event(1100, "gfn", &ipc::Event::Stats { session: SessionId(7), stats }, &mut out);
        o.frames(1500, &frames(3, calm(3)), &mut out);
        o.display(5900, &video(true, 3840), &mut out);
        o.tick(5999, &mut out);
        assert!(only(&out, "no_picture").is_empty());
        for now in [6000, 6100] { o.tick(now, &mut out); }
        let warned = only(&out, "no_picture");
        assert_eq!(warned, vec![json!({"event": "no_picture", "ms": 6000, "session": 7, "muted": true, "width": 3840, "height": 2160, "src_hz": 60.0, "fps": 59.5})]);
        assert_allowlisted(&warned[0].to_string());
        // A picture that does come later is still timed.
        o.display(8000, &video(false, 3840), &mut out);
        assert_eq!(only(&out, "first_picture"), vec![json!({"event": "first_picture", "ms": 8000, "session": 7, "after_running_ms": 7000, "after_frame_ms": 6500})]);
    }

    #[test]
    fn a_picture_is_judged_only_for_a_stream_seen_from_its_start_and_on_a_current_reading() {
        let mut out = Vec::new();
        // Already running when first seen, as after a change of level: the time since it ran is not known.
        let mut o = running(Level::Info);
        o.frames(100, &frames(3, calm(3)), &mut out);
        o.display(2000, &video(false, 3840), &mut out);
        o.tick(60_000, &mut out);
        assert!(only(&out, "first_picture").is_empty() && only(&out, "no_picture").is_empty());
        // With no reading, or only an old one, the TV's state is not known and nothing is claimed.
        let mut o = started(Level::Warn);
        o.tick(9000, &mut out);
        o.display(2000, &video(true, 3840), &mut out);
        o.tick(6000, &mut out);
        assert!(only(&out, "no_picture").is_empty());
        // At Warn the window alone is read, until a picture shows.
        assert_eq!(o.sample(6100), Some(Sample { every_ms: 1000, full: false }));
        o.frames(6200, &frames(3, calm(3)), &mut out);
        o.display(6300, &video(false, 3840), &mut out);
        assert_eq!(o.sample(6400), None);
    }

    #[test]
    fn the_panel_is_asked_for_only_while_a_stream_runs_and_a_little_after() {
        let mut o = observer(Level::Info);
        let mut out = Vec::new();
        let hdr = negotiated(true, None);
        assert_eq!(o.sample(0), None);
        o.session(0, Some(seen(&Phase::Starting, None, None)), &mut out);
        assert_eq!(o.sample(1), None);
        o.session(1000, Some(seen(&Phase::Running, Some(&hdr), None)), &mut out);
        assert_eq!(o.sample(1001), Some(Sample { every_ms: 1000, full: true }));
        o.session(9000, Some(seen(&Phase::Stopping, Some(&hdr), None)), &mut out);
        assert!(o.sample(12_999).is_some() && o.sample(13_000).is_none());
        for level in [Level::Error, Level::Warn] { assert_eq!(running(level).sample(1), None, "{level:?}"); }
    }

    #[test]
    fn controllers_are_reported_as_they_connect_and_disconnect() {
        let mut o = observer(Level::Info);
        let mut out = Vec::new();
        for (now, mask) in [(10, 0b0), (20, 0b101), (30, 0b101), (40, 0b100)] { o.pads(now, mask, &mut out); }
        assert_eq!(json_of(&out), vec![
            json!({"event": "controller", "ms": 20, "slot": 0, "connected": true}),
            json!({"event": "controller", "ms": 20, "slot": 2, "connected": true}),
            json!({"event": "controller", "ms": 40, "slot": 0, "connected": false}),
        ]);
        assert_eq!((out[0].level(), out[2].level()), (Level::Info, Level::Warn));
    }

    #[test]
    fn each_level_adds_its_documented_events_and_those_below() {
        let events = busy_stream(Level::Trace, 8);
        let kinds = |level: Level| -> BTreeSet<String> {
            let sent: Vec<Event> = events.iter().filter(|event| event.level() <= level).cloned().collect();
            json_of(&sent).iter().map(|v| v["event"].as_str().unwrap().to_owned()).collect()
        };
        let words = |list: &[&str]| -> BTreeSet<String> { list.iter().map(|word| (*word).to_owned()).collect() };
        assert_eq!(kinds(Level::Error), words(&["level", "session_end"]));
        let mut sent = vec!["level", "session_end", "hdr_missing", "frame_trouble", "net_loss", "engine_error", "controller"];
        assert_eq!(kinds(Level::Warn), words(&sent));
        sent.extend(["session", "stream_format", "stream", "frames", "render", "display", "first_picture"]);
        assert_eq!(kinds(Level::Info), words(&sent));
        sent.push("detail");
        assert_eq!(kinds(Level::Debug), words(&sent));
        sent.extend(["hist", "trip"]);
        assert_eq!(kinds(Level::Trace), words(&sent));
        // Below Debug the observer does not even build the finer lines.
        for level in [Level::Info, Level::Warn, Level::Error] { assert!(busy_stream(level, 4).iter().all(|event| event.level() <= Level::Info), "{level:?}"); }
    }

    #[test]
    fn nothing_an_engine_or_the_tv_says_in_words_reaches_an_event() {
        for level in Level::ALL {
            for event in busy_stream(level, 12) {
                let line = serde_json::to_string(&event).unwrap();
                assert_allowlisted(&line);
                for leak in ["secret", "Alice", "example", "token", "http"] { assert!(!line.contains(leak), "{leak} in {line}"); }
            }
        }
    }

    #[test]
    fn a_busy_stream_fits_every_batch_and_only_the_finest_lines_yield() {
        for level in [Level::Info, Level::Debug, Level::Trace] {
            let events = busy_stream(level, 30);
            for window in 0..6 {
                let d = Diagnostics::open_for_test(level);
                let batch: Vec<&Event> = events.iter().filter(|event| ms_of(event) / 5000 == window).collect();
                for event in &batch { d.emit((*event).clone()); }
                let (lines, dropped) = d.queued();
                let bytes: usize = lines.iter().map(|line| line.len() + 1).sum();
                // The first window also holds the start burst, where only Debug and Trace lines may yield; later windows lose nothing below Trace.
                let kept = if window == 0 { Level::Info } else { level.min(Level::Debug) };
                for event in batch.iter().filter(|event| event.level() <= kept) { assert!(lines.contains(&serde_json::to_string(event).unwrap()), "{level:?} window {window} shed {}", serde_json::to_string(event).unwrap()); }
                if window > 0 { assert!(bytes <= BATCH_LIMIT, "{level:?} window {window}: {bytes} bytes"); }
                if window > 0 && level < Level::Trace { assert_eq!(dropped, 0, "{level:?} window {window}"); }
            }
        }
    }
}
