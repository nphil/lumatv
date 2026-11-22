//! Names the part of a stream that is responsible when it feels wrong, from the readings the HUD already has.
//!
//! Every reading is optional: a leg nobody measured is absent, never guessed, and a cause is scored only from what is present.
//! A score is a reading divided by the threshold at which it starts to matter, so 1.0 is "begins to hurt" and scores of different
//! causes compare with each other.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause { Network, Host, Tv, Controller }
impl Cause {
    pub const ALL: [Self; 4] = [Self::Network, Self::Host, Self::Tv, Self::Controller];
    pub fn name(self) -> &'static str { ["Network", "Host", "TV", "Controller"][self as usize] }
}

/// One tone per verdict; the screen maps it to a palette role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone { Good, Warn, Bad, Quiet }

/// A plain sentence and how worried to be about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Line { pub tag: String, pub text: String, pub tone: Tone }
impl Line {
    fn new(tone: Tone, tag: impl Into<String>, text: impl Into<String>) -> Self { Self { tag: tag.into(), text: text.into(), tone } }
}

/// What the last second or so measured. Frame and gap times are in microseconds as the frame history keeps them, the rest in
/// milliseconds, percent or megabits as named.
#[derive(Clone, Debug, Default)]
pub struct Evidence {
    pub target_us: f32,
    pub fps: Option<f32>,
    pub rtt_ms: Option<f32>,
    pub jitter_ms: Option<f32>,
    pub loss_pct: Option<f32>,
    pub bitrate_mbps: Option<f32>,
    /// The bitrate the stream was asked for after the wired-link cap.
    pub cap_mbps: Option<f32>,
    pub host_ms: Option<f32>,
    pub client_ms: Option<f32>,
    pub pacing_ms: Option<f32>,
    pub host_gap_p95_ms: Option<f32>,
    pub arrival_gap_p95_ms: Option<f32>,
    /// Spacing of frames at the decoder feed: p99 and max.
    pub gap_p99_us: Option<u64>,
    pub gap_max_us: Option<u64>,
    pub feed_p99_us: Option<u64>,
    /// Frames lost to the TV (full, not ready, refused, player error) and to packet loss, over the last ten seconds.
    pub tv_drops: u64,
    pub loss_drops: u64,
    pub socket_drops: u64,
    /// Refreshes in the worst 100 ms against the number the panel runs at.
    pub refreshes: Option<(u64, u64)>,
    pub panel_delay_ms: Option<f32>,
    pub pad_rate_hz: Option<f32>,
    pub pad_gap_p99_ms: Option<f32>,
    pub pad_bursts: u32,
    /// Why HDR was asked for and SDR delivered, when the engine says.
    pub sdr_reason: Option<String>,
}
impl Evidence {
    /// The HUD's display reading: the refresh the panel runs at and how many of its refreshes the worst 100 ms kept.
    pub fn refreshes_of(display: &Value) -> Option<(u64, u64)> {
        let refresh = &display["refresh"];
        let fewest = refresh["fewest"].as_u64()?;
        let expected = (refresh["hz"].as_f64()? / 10.0).round() as u64;
        Some((fewest, expected))
    }
    fn target_ms(&self) -> f32 { (self.target_us / 1000.0).max(1.0) }
}

// Thresholds. Each is the reading at which a person starts to feel it, not the point of failure.
/// Packet loss: the HUD's long-standing "Losing packets" line; a lost packet costs a frame or a visible artefact at this rate.
const LOSS_PCT: f32 = 0.5;
/// Loss at this rate is a frame in every few seconds at 4K: the line moves from a nuisance to a fault.
const HEAVY_LOSS_PCT: f32 = 2.0;
/// Jitter: about one frame at 120 fps; the pacer cannot hide arrivals that wander more than a frame.
const JITTER_MS: f32 = 8.0;
/// Round trip: above this cloud play stops feeling local; it adds to every press.
const RTT_MS: f32 = 60.0;
/// Spacing p95 against the frame time: a frame arriving half a frame late or early reads as judder.
const UNEVEN: f32 = 1.5;
/// Encode: more than three quarters of a frame period means the host has no headroom left.
const ENCODE_OF_FRAME: f32 = 0.75;
/// Frame rate below nine tenths of what was asked is a lost frame in ten.
const FPS_FLOOR: f32 = 0.9;
/// Handover plus pacing hold: one 60 Hz frame spent on the TV before the decoder has the picture.
const TV_HOLD_MS: f32 = 16.0;
/// Handing a frame to the decoder taking half a frame period at p99 leaves the decoder starved.
const FEED_OF_FRAME: f32 = 0.5;
/// Reported panel delay: game mode sits near 10 to 16 ms, a processing mode at 30 ms or more.
const PANEL_MS: f32 = 30.0;
/// Controller report gap p99: one 60 Hz frame; longer and a press waits for a report.
const PAD_GAP_MS: f32 = 16.0;
/// Below this share of what was asked for, the picture is being held back by the link, not by the setting.
const BITRATE_SHARE: f32 = 0.6;

/// How far each cause is over its threshold. 0 is clean and 1 is where it begins to matter.
pub fn scores(e: &Evidence) -> [f32; 4] {
    let target = e.target_ms();
    let ratio = |v: Option<f32>, t: f32| v.map_or(0.0, |v| v / t);
    let uneven = |g: Option<f32>| g.is_some_and(|g| g > target * UNEVEN);
    let host_even = e.host_gap_p95_ms.is_some() && !uneven(e.host_gap_p95_ms);
    let present = |v: bool, s: f32| if v { s } else { 0.0 };
    let network = ratio(e.loss_pct, LOSS_PCT)
        .max(ratio(e.jitter_ms, JITTER_MS))
        .max(ratio(e.rtt_ms, RTT_MS))
        .max(present(e.loss_drops > 0, 1.0))
        // Frames the host sent evenly and the network bunched.
        .max(present(host_even && uneven(e.arrival_gap_p95_ms), 1.0));
    let target_fps = 1e6 / e.target_us.max(1.0);
    let shortfall = e.fps.map_or(0.0, |f| ((1.0 - f / target_fps) / (1.0 - FPS_FLOOR)).max(0.0));
    let host = ratio(e.host_ms, target * ENCODE_OF_FRAME)
        .max(present(uneven(e.host_gap_p95_ms), ratio(e.host_gap_p95_ms, target * UNEVEN)))
        // A short frame rate with nothing lost or late anywhere else is the host not producing frames; it yields to a cause with
        // evidence of its own.
        .max(shortfall * 0.9);
    let arrival_even = e.arrival_gap_p95_ms.is_some() && !uneven(e.arrival_gap_p95_ms);
    let feed_gap = e.gap_p99_us.is_some_and(|g| g as f32 > target * 1000.0 * UNEVEN);
    let tv = present(e.tv_drops > 0, 1.0)
        .max(present(e.socket_drops > 0, 1.0))
        .max(ratio(e.client_ms.or(e.pacing_ms).map(|_| e.client_ms.unwrap_or(0.0) + e.pacing_ms.unwrap_or(0.0)), TV_HOLD_MS))
        .max(ratio(e.feed_p99_us.map(|f| f as f32 / 1000.0), target * FEED_OF_FRAME))
        .max(ratio(e.panel_delay_ms, PANEL_MS))
        .max(present(e.refreshes.is_some_and(|(fewest, expected)| fewest + 1 < expected), 1.0))
        // Uneven at the feed while the host and the arrival were even: the unevenness began on the TV.
        .max(present(feed_gap && host_even && arrival_even, 1.0));
    let controller = ratio(e.pad_gap_p99_ms, PAD_GAP_MS).max(present(e.pad_bursts > 0, 1.0));
    [network, host, tv, controller]
}

/// Enter after two consecutive readings (half a second at the HUD's four a second); leave after six (a second and a half). A rival
/// takes over only when it is over its own threshold and a quarter above the one held.
const ENTER_TICKS: u8 = 2;
const LEAVE_TICKS: u8 = 6;
/// The held cause stays until its score falls under this, well under the 1.0 that raised it, so a reading that sits on the
/// threshold does not flicker.
const KEEP: f32 = 0.7;
const OVERTAKE: f32 = 1.25;

#[derive(Clone, Debug, Default)]
pub struct Diagnoser {
    held: Option<Cause>,
    rival: Option<(Option<Cause>, u8)>,
}
impl Diagnoser {
    pub fn held(&self) -> Option<Cause> { self.held }
    pub fn update(&mut self, score: [f32; 4]) -> Option<Cause> {
        let (best, top) = Cause::ALL.iter().map(|c| (*c, score[*c as usize])).fold((Cause::Network, 0.0), |a, b| if b.1 > a.1 { b } else { a });
        let wanted = match self.held {
            Some(held) if score[held as usize] >= KEEP => {
                (best != held && top >= 1.0 && top >= score[held as usize] * OVERTAKE).then_some((Some(best), ENTER_TICKS))
            }
            Some(_) => Some(((top >= 1.0).then_some(best), LEAVE_TICKS)),
            None => (top >= 1.0).then_some((Some(best), ENTER_TICKS)),
        };
        let Some((next, ticks)) = wanted else {
            self.rival = None;
            return self.held;
        };
        let seen = match self.rival { Some((c, n)) if c == next => n + 1, _ => 1 };
        if seen >= ticks { self.held = next; self.rival = None; } else { self.rival = Some((next, seen)); }
        self.held
    }
}

/// The legs of the round trip that were measured, in the order the press travels, in milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Legs { pub parts: [Option<f32>; 5] }
impl Legs {
    pub const NAMES: [&'static str; 5] = ["Controller", "Network", "Host", "TV", "Panel"];
    /// A press waits on average for half a report interval and at worst one; the interval is the figure that can be compared with
    /// the others. The TV leg is what the TV does before the decoder has the frame (handover and pacing hold); decode and scan-out
    /// themselves are not measured, and the panel leg is the delay the panel's firmware reports.
    pub fn of(e: &Evidence) -> Self {
        let sum = |a: Option<f32>, b: Option<f32>| match (a, b) { (None, None) => None, (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)) };
        Self { parts: [e.pad_rate_hz.filter(|r| *r >= 1.0).map(|r| 1000.0 / r), e.rtt_ms, e.host_ms, sum(e.client_ms, e.pacing_ms), e.panel_delay_ms] }
    }
    pub fn total(&self) -> Option<f32> {
        self.parts.iter().flatten().copied().reduce(|a, b| a + b)
    }
    fn biggest(&self) -> Option<(usize, f32)> {
        self.parts.iter().enumerate().filter_map(|(i, p)| p.map(|p| (i, p))).max_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// The headline and the one-line verdict of each section of the full overlay. `tag` is the short word beside a section's name.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub cause: Option<Cause>,
    pub headline: Line,
    pub latency: Line,
    pub smooth: Line,
    pub network: Line,
    pub picture: Line,
    pub tv: Line,
    pub controller: Line,
}
impl Default for Report {
    fn default() -> Self { report(&Evidence::default(), None) }
}

fn one(v: f32) -> String { format!("{v:.1}") }

pub fn report(e: &Evidence, cause: Option<Cause>) -> Report {
    let target = e.target_ms();
    let legs = Legs::of(e);
    let latency = match (legs.total(), legs.biggest()) {
        (Some(total), Some((i, part))) if legs.parts.iter().flatten().count() > 1 => {
            let tag = format!("~{total:.0} ms");
            if part / total >= 0.5 { Line::new(Tone::Quiet, tag, format!("{} is {part:.0} of about {total:.0} ms", Legs::NAMES[i])) }
            else { Line::new(Tone::Quiet, tag, format!("About {total:.0} ms, spread evenly")) }
        }
        (Some(total), _) => Line::new(Tone::Quiet, format!("~{total:.0} ms"), format!("About {total:.0} ms; only one leg is measured")),
        _ => Line::new(Tone::Quiet, "No data", "No latency leg is measured yet"),
    };
    let smooth = if let Some(g) = e.gap_max_us.filter(|g| *g >= 100_000) {
        Line::new(Tone::Bad, "Stalling", format!("Stalled for {:.0} ms", g as f32 / 1000.0))
    } else if let Some(g) = e.host_gap_p95_ms.filter(|g| *g > target * UNEVEN) {
        Line::new(Tone::Warn, "Uneven", format!("The host sends frames unevenly, {} ms apart at p95", one(g)))
    } else if let Some(g) = e.arrival_gap_p95_ms.filter(|g| *g > target * UNEVEN) {
        Line::new(Tone::Warn, "Bunched", format!("The network bunches frames, {} ms apart at p95", one(g)))
    } else if e.tv_drops > 0 {
        Line::new(Tone::Warn, "Dropping", format!("The TV dropped {} frames in the last 10 s", e.tv_drops))
    } else if e.loss_drops > 0 {
        Line::new(Tone::Warn, "Dropping", format!("{} frames lost to packet loss in the last 10 s", e.loss_drops))
    } else if e.gap_p99_us.is_none() {
        Line::new(Tone::Quiet, "No data", "No frame timing yet")
    } else {
        Line::new(Tone::Good, "Even", "Even frames, none dropped")
    };
    let network = if e.loss_pct.is_none() && e.jitter_ms.is_none() && e.rtt_ms.is_none() {
        Line::new(Tone::Quiet, "No data", "No network reading yet")
    } else if let Some(l) = e.loss_pct.filter(|l| *l >= LOSS_PCT) {
        let heavy = l >= HEAVY_LOSS_PCT;
        Line::new(if heavy { Tone::Bad } else { Tone::Warn }, "Loss", format!("{} packet loss, {l:.1}%", if heavy { "Heavy" } else { "Occasional" }))
    } else if let Some(j) = e.jitter_ms.filter(|j| *j >= JITTER_MS) {
        Line::new(Tone::Warn, "Jitter", format!("Packets arrive unevenly, {} ms of jitter", one(j)))
    } else if let Some(r) = e.rtt_ms.filter(|r| *r >= RTT_MS) {
        Line::new(Tone::Warn, "Far", format!("The host is far away, {r:.0} ms round trip"))
    } else {
        Line::new(Tone::Good, "Good", format!("Steady{}", e.rtt_ms.map_or(String::new(), |r| format!(", {r:.0} ms round trip"))))
    };
    let picture = if let Some(reason) = &e.sdr_reason {
        Line::new(Tone::Warn, "SDR", format!("HDR was asked for and SDR delivered: {reason}"))
    } else {
        match (e.bitrate_mbps, e.cap_mbps) {
            (Some(b), Some(cap)) if cap > 0.0 && b < cap * BITRATE_SHARE && e.loss_pct.is_some_and(|l| l >= LOSS_PCT) =>
                Line::new(Tone::Warn, "Softer", format!("Softer picture: {b:.0} of {cap:.0} Mb/s after packet loss")),
            (Some(b), Some(cap)) if cap > 0.0 && b < cap * BITRATE_SHARE =>
                Line::new(Tone::Warn, "Softer", format!("Softer picture: the stream holds {b:.0} of {cap:.0} Mb/s")),
            (Some(b), Some(cap)) => Line::new(Tone::Good, "Full", format!("Full quality, {b:.0} of {cap:.0} Mb/s")),
            (Some(b), None) => Line::new(Tone::Quiet, "Full", format!("{b:.0} Mb/s")),
            _ => Line::new(Tone::Quiet, "No data", "No bitrate reading yet"),
        }
    };
    let tv = if e.tv_drops > 0 {
        Line::new(Tone::Warn, "Dropping", format!("The TV dropped {} frames in the last 10 s", e.tv_drops))
    } else if e.socket_drops > 0 {
        Line::new(Tone::Warn, "Dropping", format!("The TV's network buffer discarded {} packets", e.socket_drops))
    } else if let Some((fewest, expected)) = e.refreshes.filter(|(f, x)| f + 1 < *x) {
        Line::new(Tone::Warn, "Uneven", format!("The panel kept {fewest} of {expected} refreshes in its worst 100 ms"))
    } else if let Some(d) = e.panel_delay_ms.filter(|d| *d >= PANEL_MS) {
        Line::new(Tone::Warn, "Slow", format!("The panel adds {d:.0} ms; a game picture mode is faster"))
    } else if let Some(h) = e.client_ms.or(e.pacing_ms).map(|_| e.client_ms.unwrap_or(0.0) + e.pacing_ms.unwrap_or(0.0)).filter(|h| *h >= TV_HOLD_MS) {
        Line::new(Tone::Warn, "Slow", format!("The TV holds each frame {} ms before decode", one(h)))
    } else if e.gap_p99_us.is_some_and(|g| g as f32 > target * 1000.0 * UNEVEN)
        && e.host_gap_p95_ms.is_some_and(|g| g <= target * UNEVEN) && e.arrival_gap_p95_ms.is_some_and(|g| g <= target * UNEVEN) {
        Line::new(Tone::Warn, "Uneven", format!("Frames arrive evenly, then leave the TV unevenly, {} ms at p99", one(e.gap_p99_us.unwrap_or(0) as f32 / 1000.0)))
    } else if e.gap_p99_us.is_none() && e.refreshes.is_none() {
        Line::new(Tone::Quiet, "No data", "No TV reading yet")
    } else {
        Line::new(Tone::Good, "Good", "No frames dropped, the panel kept every refresh")
    };
    let controller = match (e.pad_rate_hz, e.pad_gap_p99_ms) {
        (None, _) => Line::new(Tone::Quiet, "No data", "No controller reports yet"),
        _ if e.pad_bursts > 0 => Line::new(Tone::Warn, "Bursty", format!("Reports arrive in bursts, {} in the last seconds", e.pad_bursts)),
        (_, Some(g)) if g >= PAD_GAP_MS => Line::new(Tone::Warn, "Slow", format!("Reports are {} ms apart at p99", one(g))),
        (Some(r), _) => Line::new(Tone::Good, "Even", format!("Steady, {r:.0} reports a second")),
    };
    // A cause that is still held after its sentence has gone quiet is not "good": the word stays amber until it is let go.
    let held = |line: Line, tag: &str| Line { tag: tag.into(), tone: if line.tone == Tone::Good { Tone::Warn } else { line.tone }, ..line };
    let headline = match cause {
        None if e.fps.is_none() && e.rtt_ms.is_none() => Line::new(Tone::Quiet, "No data", "Waiting for the stream's readings"),
        None => Line::new(Tone::Good, "Good", "Nothing is holding the stream back"),
        Some(Cause::Network) => held(network.clone(), "Network"),
        Some(Cause::Host) => host_line(e, target),
        Some(Cause::Tv) => held(tv.clone(), "TV"),
        Some(Cause::Controller) => held(controller.clone(), "Controller"),
    };
    Report { cause, headline, latency, smooth, network, picture, tv, controller }
}

fn host_line(e: &Evidence, target: f32) -> Line {
    let tag = "Host";
    if let Some(g) = e.host_gap_p95_ms.filter(|g| *g > target * UNEVEN) { return Line::new(Tone::Warn, tag, format!("The host sends frames unevenly, {} ms apart at p95", one(g))); }
    if let Some(h) = e.host_ms.filter(|h| *h >= target * ENCODE_OF_FRAME) { return Line::new(Tone::Warn, tag, format!("The host takes {} ms to encode a {} ms frame", one(h), one(target))); }
    Line::new(Tone::Warn, tag, format!("The host delivers {:.0} fps, short of the {:.0} asked for", e.fps.unwrap_or(0.0), 1_000_000.0 / e.target_us.max(1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream at 120 fps with every leg healthy; each scenario spoils one thing.
    fn healthy() -> Evidence {
        Evidence { target_us: 8333.0, fps: Some(119.9), rtt_ms: Some(11.0), jitter_ms: Some(0.8), loss_pct: Some(0.0), bitrate_mbps: Some(74.0),
            cap_mbps: Some(80.0), host_ms: Some(5.0), client_ms: Some(1.0), pacing_ms: Some(2.0), host_gap_p95_ms: Some(9.1),
            arrival_gap_p95_ms: Some(9.5), gap_p99_us: Some(8600), gap_max_us: Some(9000), feed_p99_us: Some(600), refreshes: Some((12, 12)),
            panel_delay_ms: Some(16.0), pad_rate_hz: Some(250.0), pad_gap_p99_ms: Some(4.9), ..Default::default() }
    }
    fn winner(e: &Evidence) -> Option<Cause> {
        let s = scores(e);
        let mut d = Diagnoser::default();
        let mut cause = None;
        for _ in 0..ENTER_TICKS { cause = d.update(s); }
        cause
    }

    #[test]
    fn a_healthy_stream_names_nobody() {
        assert_eq!(scores(&healthy()).map(|s| s < 1.0), [true; 4]);
        assert_eq!(winner(&healthy()), None);
    }
    #[test]
    fn each_cause_wins_in_its_own_scenario() {
        let mut network = healthy();
        network.loss_pct = Some(1.4);
        network.jitter_ms = Some(12.0);
        network.loss_drops = 3;
        let mut host = healthy();
        host.host_gap_p95_ms = Some(16.9);
        host.host_ms = Some(7.5);
        host.fps = Some(100.0);
        let mut tv = healthy();
        tv.tv_drops = 6;
        tv.socket_drops = 12;
        tv.refreshes = Some((9, 12));
        let mut pad = healthy();
        pad.pad_bursts = 6;
        pad.pad_gap_p99_ms = Some(22.0);
        assert_eq!(winner(&network), Some(Cause::Network));
        assert_eq!(winner(&host), Some(Cause::Host));
        assert_eq!(winner(&tv), Some(Cause::Tv));
        assert_eq!(winner(&pad), Some(Cause::Controller));
    }
    #[test]
    fn uneven_frames_are_blamed_where_they_began() {
        let mut e = healthy();
        e.host_gap_p95_ms = Some(8.4);
        e.arrival_gap_p95_ms = Some(16.4);
        assert_eq!(winner(&e), Some(Cause::Network), "even at the host, bunched on the way");
        e.arrival_gap_p95_ms = Some(8.6);
        e.gap_p99_us = Some(20_000);
        assert_eq!(winner(&e), Some(Cause::Tv), "even at the host and on arrival, uneven at the feed");
        e.host_gap_p95_ms = Some(17.0);
        assert_eq!(winner(&e), Some(Cause::Host), "uneven from the host");
    }
    #[test]
    fn a_short_frame_rate_is_the_hosts_only_when_nothing_else_has_evidence() {
        let mut e = healthy();
        e.fps = Some(100.0);
        assert_eq!(winner(&e), Some(Cause::Host));
        e.loss_pct = Some(1.0);
        assert_eq!(winner(&e), Some(Cause::Network));
    }
    #[test]
    fn a_missing_reading_is_not_a_clean_one_or_a_bad_one() {
        let e = Evidence { target_us: 8333.0, ..Default::default() };
        assert_eq!(scores(&e), [0.0; 4]);
    }
    #[test]
    fn a_reading_on_the_threshold_does_not_flicker_the_verdict() {
        let mut d = Diagnoser::default();
        let mut shown = Vec::new();
        // Network score 1.1 and 0.9 alternating, as loss reading 0.55 % and 0.45 % would.
        for i in 0..40 { shown.push(d.update([if i % 2 == 0 { 1.1 } else { 0.9 }, 0.0, 0.0, 0.0])); }
        assert_eq!(shown[0], None, "one reading over the line is not enough");
        assert_eq!(shown[1], None, "the readings are not consecutive");
        let flips = shown.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(flips <= 1, "{flips} flips: {shown:?}");
    }
    #[test]
    fn a_held_cause_stays_through_a_dip_and_leaves_only_after_a_steady_calm() {
        let mut d = Diagnoser::default();
        for _ in 0..ENTER_TICKS { d.update([1.5, 0.0, 0.0, 0.0]); }
        assert_eq!(d.held(), Some(Cause::Network));
        for _ in 0..40 { d.update([0.8, 0.0, 0.0, 0.0]); }
        assert_eq!(d.held(), Some(Cause::Network), "0.8 is under the threshold but over the keep line");
        for _ in 0..LEAVE_TICKS - 1 { d.update([0.2, 0.0, 0.0, 0.0]); }
        assert_eq!(d.held(), Some(Cause::Network));
        d.update([0.2, 0.0, 0.0, 0.0]);
        assert_eq!(d.held(), None);
    }
    #[test]
    fn a_rival_must_be_clearly_worse_and_stay_worse_to_take_over() {
        let mut d = Diagnoser::default();
        for _ in 0..ENTER_TICKS { d.update([1.4, 0.0, 0.0, 0.0]); }
        for i in 0..40 { d.update(if i % 2 == 0 { [1.4, 1.5, 0.0, 0.0] } else { [1.4, 1.2, 0.0, 0.0] }); }
        assert_eq!(d.held(), Some(Cause::Network), "1.5 against 1.4 is noise");
        for _ in 0..ENTER_TICKS { d.update([1.4, 2.5, 0.0, 0.0]); }
        assert_eq!(d.held(), Some(Cause::Host));
    }
    #[test]
    fn the_latency_legs_add_only_what_was_measured() {
        let mut e = healthy();
        let all = Legs::of(&e);
        assert_eq!(all.parts[0], Some(4.0));
        assert_eq!(all.parts[3], Some(3.0));
        assert_eq!(all.total(), Some(4.0 + 11.0 + 5.0 + 3.0 + 16.0));
        e.host_ms = None;
        e.panel_delay_ms = None;
        assert_eq!(Legs::of(&e).parts[2], None);
        assert_eq!(Legs::of(&e).total(), Some(4.0 + 11.0 + 3.0));
        assert_eq!(Legs::of(&Evidence::default()).total(), None);
    }
    #[test]
    fn verdict_lines_say_which_part_and_what_it_did() {
        let mut e = healthy();
        e.loss_pct = Some(0.8);
        let r = report(&e, Some(Cause::Network));
        assert_eq!(r.network.text, "Occasional packet loss, 0.8%");
        assert_eq!(r.headline.text, r.network.text);
        e.bitrate_mbps = Some(30.0);
        assert!(report(&e, None).picture.text.starts_with("Softer picture"));
        assert_eq!(report(&healthy(), None).latency.text, "About 39 ms, spread evenly");
        let mut far = healthy();
        far.rtt_ms = Some(70.0);
        assert_eq!(report(&far, None).latency.text, "Network is 70 of about 98 ms");
        assert_eq!(report(&Evidence::default(), None).headline.tag, "No data");
    }
}
