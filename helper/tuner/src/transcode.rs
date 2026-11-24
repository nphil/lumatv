//! H.264 for the clients that ask for it: `GET /auto/v<channel>?transcode=<profile>`, the HDHomeRun EXTEND's request (SiliconDust's HTTP
//! API). The station's MPEG-2 picture goes through the TV's own video hardware, its decoder and then its H.264 encoder, and what comes
//! out is a transport stream again: the station's PIDs, the picture as H.264 (stream type 0x1B), the sound as the station sent it. A
//! phone then decodes the picture in hardware instead of MPEG-2 in software (`docs/research/hw-transcode.md`).
//!
//! The hardware is behind two traits, [`Codec`] and [`Convert`], so all of this runs in tests on a fake. A client never loses its
//! stream to the converter: one that is not installed, cannot take the picture or does not open leaves the request as the station sends
//! it (MPEG-2, byte for byte as without any of this); one that fails while it streams ends the stream and rests ([`Breaker`]), so the
//! client's next request is MPEG-2 again; one that hangs a call into the TV's video hardware is not used again until the helper starts.

use crate::mpeg2::Format;
use crate::psi::{self, PACKET, Pmt};
use crate::splice::{self, WRAP};
use crate::stream::Limits;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

/// The profiles of `?transcode=` (SiliconDust's EXTEND): what picture the client is sent. Every one is progressive: the TV's decoder deinterlaces in
/// hardware, so the encoder is given whole frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// H.264 at the station's own size, whatever it is (1080i is 1920x1080, 720p is 1280x720, 480i keeps its 704x480 or 720x480 and says what shape
    /// its pixels are), and at its own rate: 59.94 stays 59.94, 1080i and 480i come out at 29.97.
    Heavy,
    /// At most 1280x720 and 30 frames a second.
    Mobile,
    /// At most 30 frames a second, at a lower bit rate, within 960x540 (4:3: 720x540), 848x480 (640x480), 640x360 (480x360) or 432x240 (320x240).
    Internet540,
    Internet480,
    Internet360,
    Internet240,
}

/// What the converter is asked to make: the largest picture the profile allows for the station's, never larger than the station's, always
/// progressive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    /// Square pixels, a multiple of 16 wide, for a profile with a box: a coded size H.264 needs no padding for, and what the SiliconDust profiles
    /// name. `Heavy` is the station's own coded size, whose pixels may not be square.
    pub width: u16,
    pub height: u16,
    /// Frames a second as a numerator and a denominator (30000 and 1001 for 29.97).
    pub rate: (u32, u32),
    /// The video bit rate to aim for in kbit/s: a starting point to tune on the set, SiliconDust does not publish its own.
    pub kbps: u32,
}

/// The box a profile fits a station's picture in, for a 16:9 station and for a 4:3 one, and the bit rate it aims for in it.
struct Fit {
    wide: (u32, u32),
    narrow: (u32, u32),
    kbps: u32,
}

/// What `Heavy` aims for at 1080 lines, in kbit/s: a station's MPEG-2 is at most ATSC's 19.4 Mbit/s, and H.264 holds a picture in about half.
const HEAVY_KBPS: u32 = 9000;

impl Profile {
    /// Every profile, the best picture first: the order `/status.json` lists the ones a converter makes in.
    pub const ALL: [Self; 6] = [Self::Heavy, Self::Mobile, Self::Internet540, Self::Internet480, Self::Internet360, Self::Internet240];

    /// The profile `name` names, as `?transcode=` spells it.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name().eq_ignore_ascii_case(name))
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Heavy => "heavy",
            Self::Mobile => "mobile",
            Self::Internet540 => "internet540",
            Self::Internet480 => "internet480",
            Self::Internet360 => "internet360",
            Self::Internet240 => "internet240",
        }
    }

    /// The box of a profile that has one: every profile but `Heavy`.
    fn fit(self) -> Option<Fit> {
        let (wide, narrow, kbps) = match self {
            Self::Heavy => return None,
            Self::Mobile => ((1280, 720), (960, 720), 3500),
            Self::Internet540 => ((960, 540), (720, 540), 2000),
            Self::Internet480 => ((848, 480), (640, 480), 1200),
            Self::Internet360 => ((640, 360), (480, 360), 800),
            Self::Internet240 => ((432, 240), (320, 240), 400),
        };
        Some(Fit { wide, narrow, kbps })
    }

    /// What `Heavy` aims for at `height` lines and the rate: `HEAVY_KBPS` above 720 lines, 7.5 Mbit/s for 720p60 and 5 for 720p30, 2.5 for standard
    /// definition.
    fn heavy_kbps(height: u16, rate: (u32, u32)) -> u32 {
        match height {
            721.. => HEAVY_KBPS,
            481..=720 if rate.0 > 30 * rate.1 => 7500,
            481..=720 => 5000,
            _ => 2500,
        }
    }

    /// The profile as `/status.json` lists it: `maxHeight` is `null` for the one that keeps the station's size, and `kbps` is what the profile aims for
    /// at its largest picture (a smaller station picture gets less).
    pub fn describe(self) -> serde_json::Value {
        let (max_height, kbps) = match self.fit() {
            Some(fit) => (Some(fit.wide.1), fit.kbps),
            None => (None, HEAVY_KBPS),
        };
        serde_json::json!({ "name": self.name(), "codec": "h264", "maxHeight": max_height, "progressive": true, "kbps": kbps })
    }

    /// What the profile asks for a station's picture. A box profile halves a rate above 30 frames a second (59.94 becomes 29.97, 50 becomes 25),
    /// which is every other picture: the decoder still decodes them all, they are references. `Heavy` keeps the rate.
    pub fn want(self, source: &Format) -> Want {
        let (num, den) = source.frame_rate();
        let (num, den) = (num as u32, den as u32);
        let height = source.height.min(1080);
        let Some(fit) = self.fit() else {
            return Want { width: source.width, height, rate: (num, den), kbps: Self::heavy_kbps(height, (num, den)) };
        };
        let wide = source.display_width() / f32::from(source.height) > 1.5;
        let (box_width, box_height) = if wide { fit.wide } else { fit.narrow };
        // the box has the picture's own shape, so its height sets the width; a station's own height is a ceiling, nothing is enlarged
        let height = box_height.min(u32::from(height));
        let width = (height * box_width / box_height) & !15;
        // above 30 frames a second every other picture goes (60000/1001 becomes 30000/1001, which players read as 29.97)
        let rate = match (num > 30 * den, num % 2 == 0) {
            (false, _) => (num, den),
            (true, true) => (num / 2, den),
            (true, false) => (num, 2 * den),
        };
        Want { width: width as u16, height: height as u16, rate, kbps: fit.kbps }
    }
}

/// Why a converter stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Not for this picture (a size or rate the hardware does not take, a picture that is not MPEG-2): nothing is wrong with the
    /// converter, and the stream goes out as the station sends it.
    Unsupported(String),
    /// The converter failed: the stream goes out as the station sends it, and the converter rests.
    Failed(String),
    /// Pictures went in and nothing came out in time: as `Failed`, kept apart for the log.
    Stalled(String),
    /// A call into the TV's video hardware did not return. The hardware may stay stuck until the TV restarts, so no request tries a
    /// converter again until the helper has been restarted.
    Wedged(String),
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Self::Unsupported(why) | Self::Failed(why) | Self::Stalled(why) | Self::Wedged(why)) = self;
        f.write_str(why)
    }
}

/// One H.264 access unit as Annex B (start codes), with its SPS and PPS when it starts a sequence, and the PTS of the MPEG-2 picture it
/// was made from.
pub struct Unit {
    pub pts: u64,
    pub data: Vec<u8>,
}

/// One picture stream being converted. Dropping it gives the hardware back; that must not wait on a hung hardware call.
pub trait Convert: Send {
    /// Takes the next MPEG-2 picture, as the station sent it (its sequence header when it has one), in the order it was sent, and its
    /// PTS. Never waits on the hardware: a picture that cannot be taken now is a fault.
    fn push(&mut self, picture: &[u8], pts: u64) -> Result<(), Fault>;
    /// Adds the units that are ready to `out`, in the order they are shown, each with the PTS of its picture. Never waits. A picture
    /// that is dropped to lower the rate has no unit.
    fn pull(&mut self, out: &mut Vec<Unit>) -> Result<(), Fault>;
}

/// The TV's video hardware, or whatever else makes H.264.
pub trait Codec: Send + Sync {
    /// Opens a converter for pictures of the `source` format that makes `want`, and returns within a second or so, `Fault::Failed` or
    /// `Fault::Wedged` if it cannot.
    fn open(&self, source: &Format, want: &Want) -> Result<Box<dyn Convert>, Fault>;

    /// The profiles this converter makes, the best picture first: what `/status.json` tells a client it may ask for. A request for another is still
    /// answered, as well as the converter can.
    fn profiles(&self) -> &'static [Profile] {
        &Profile::ALL
    }
}

/// When a converter may be tried: the one that failed rests, the one that hung is not tried again.
#[derive(Default)]
pub(crate) struct Breaker(Mutex<Rest>);

#[derive(Clone, Copy, Default)]
enum Rest {
    #[default]
    Ready,
    Until(Duration),
    Always,
}

impl Breaker {
    pub(crate) fn allows(&self, now: Duration) -> bool {
        match *self.0.lock() {
            Rest::Ready => true,
            Rest::Until(until) => now >= until,
            Rest::Always => false,
        }
    }

    /// A converter that failed at `now`: not tried for `rest`, or ever if it hung. A picture the converter cannot take is no failure.
    pub(crate) fn failed(&self, fault: &Fault, now: Duration, rest: Duration) {
        let mut state = self.0.lock();
        *state = match fault {
            Fault::Unsupported(_) => return,
            Fault::Wedged(_) => Rest::Always,
            _ if matches!(*state, Rest::Always) => Rest::Always,
            _ => Rest::Until(now + rest),
        };
    }
}

/// How far the 33-bit clock may jump between two PCRs before it is taken for a reset of the station's encoder: half a second.
const JUMP: i64 = 45_000;
/// The longest the PCR of a stream may go unsent while pictures are (an MPEG-2 PCR is due every 100 ms).
const PCR_EVERY: Duration = Duration::from_millis(40);
/// How far behind its picture the sound is held while the first unit has not come yet: what the hardware is expected to take
/// (100 to 170 ms was estimated for the set); the first unit replaces it with what it did take.
const FIRST_LAG: Duration = Duration::from_millis(100);
/// Packets of the other PIDs kept while waiting for the first unit (8 s of 384 kbit/s AC-3 is about 2000).
const HELD_MAX: usize = 2048;
/// Pictures pushed that were not answered, remembered to measure how long one takes.
const PENDING_MAX: usize = 256;

/// The 90 kHz ticks of `d`, on the 33-bit clock.
fn ticks(d: Duration) -> u64 {
    (d.as_micros() * 9 / 100) as u64 & (WRAP - 1)
}

/// The station's clock, followed from its PCRs.
#[derive(Default)]
struct Station {
    /// The station's clock minus this stream's own, in ticks on the 33-bit clock. Each PCR pulls it a sixteenth of the way: the PCR
    /// sent then does not carry the jitter of when a read happened to return, and still follows the two clocks' drift.
    skew: Option<u64>,
    /// The clock jumped: the next PCR sent says so.
    jumped: bool,
    /// The last PCR sent, which a later one does not go back from.
    last: Option<u64>,
}

impl Station {
    fn seen(&mut self, pcr: u64, at: Duration) {
        let local = ticks(at);
        let reset = splice::add(pcr, WRAP - local);
        let Some(skew) = self.skew else {
            self.skew = Some(reset);
            return;
        };
        let err = splice::diff(pcr, splice::add(local, skew));
        if err.abs() > JUMP {
            self.skew = Some(reset);
            self.jumped = true;
            self.last = None;
        } else {
            self.skew = Some((skew as i64 + err / 16).rem_euclid(WRAP as i64) as u64);
        }
    }

    /// The PCR for `now` of a stream that runs `lag` behind its input, and whether the clock jumped since the last one: the whole
    /// stream is the station's, delayed, so a picture keeps its margin of PTS over PCR.
    fn pcr(&mut self, now: Duration, lag: Duration) -> Option<(u64, bool)> {
        let at = splice::add(ticks(now), self.skew?);
        let mut pcr = splice::add(at, WRAP - ticks(lag));
        if self.last.is_some_and(|last| splice::diff(pcr, last) < 0) {
            pcr = self.last?;
        }
        self.last = Some(pcr);
        Some((pcr, std::mem::take(&mut self.jumped)))
    }
}

/// A picture's PES as it comes in: its payload, and the PTS in its head.
#[derive(Default)]
struct Gather {
    es: Vec<u8>,
    pts: Option<u64>,
    open: bool,
}

struct Picture {
    es: Vec<u8>,
    pts: Option<u64>,
}

impl Gather {
    /// Takes a packet of the picture's PID; the picture that the packet's start ends, if one was open.
    fn push(&mut self, p: &[u8]) -> Option<Picture> {
        if (p[3] >> 4) & 1 == 0 {
            return None;
        }
        if p[1] & 0x40 != 0 {
            let done = self.take();
            if let Some(head) = splice::head(p) {
                self.open = true;
                self.pts = head.pts.map(|(_, pts)| pts);
                self.es.extend_from_slice(p.get(head.payload..).unwrap_or_default());
            }
            return done;
        }
        if self.open
            && let Some(at) = splice::payload_at(p)
        {
            self.es.extend_from_slice(&p[at..]);
        }
        None
    }

    fn take(&mut self) -> Option<Picture> {
        let open = std::mem::take(&mut self.open);
        let es = std::mem::take(&mut self.es);
        (open && !es.is_empty()).then(|| Picture { es, pts: self.pts.take() })
    }
}

/// What an H.264 access unit holds.
struct Nals {
    /// It starts with an access unit delimiter.
    aud: bool,
    sps: bool,
    idr: bool,
}

impl Nals {
    fn of(es: &[u8]) -> Self {
        let mut n = Self { aud: false, sps: false, idr: false };
        let (mut at, mut first) = (0, true);
        while let Some(i) = es[at..].windows(3).position(|w| w == [0, 0, 1]) {
            let Some(&head) = es.get(at + i + 3) else { break };
            match head & 0x1F {
                9 => n.aud |= first,
                7 => n.sps = true,
                5 => n.idr = true,
                _ => {}
            }
            first = false;
            at += i + 3;
        }
        n
    }

    /// A player can start here: the parameter sets and an IDR slice.
    fn key(&self) -> bool {
        self.sps && self.idr
    }
}

/// `pes` as packets of `pid`, the first starting it. `clock` is a PCR for the first packet and whether it follows a jump; `key` marks
/// the first a random access point.
fn packetize(pid: u16, cc: &mut u8, pes: &[u8], clock: Option<(u64, bool)>, key: bool, out: &mut Vec<u8>) {
    let mut at = 0;
    while at < pes.len() {
        let first = at == 0;
        let flags = if first {
            u8::from(key) << 6 | clock.map_or(0, |(_, jump)| 0x10 | u8::from(jump) << 7)
        } else {
            0
        };
        // the adaptation field's bytes after its length: the flags and the PCR; whatever is left of the packet it fills with stuffing
        let fixed = if flags != 0 { 1 + 6 * usize::from(clock.is_some()) } else { 0 };
        let take = (pes.len() - at).min(if fixed > 0 { 183 - fixed } else { 184 });
        let stuffed = fixed > 0 || take < 184;
        let mut p = [0xFF; PACKET];
        p[0] = 0x47;
        p[1] = u8::from(first) << 6 | (pid >> 8) as u8;
        p[2] = pid as u8;
        p[3] = if stuffed { 0x30 } else { 0x10 } | (*cc & 15);
        *cc = (*cc + 1) & 15;
        let start = if stuffed {
            let field = 183 - take;
            p[4] = field as u8;
            if field > 0 {
                p[5] = flags;
            }
            if let (true, Some((pcr, _))) = (first, clock) {
                p[6..12].fill(0);
                splice::write_pcr(&mut p, pcr);
            }
            5 + field
        } else {
            4
        };
        p[start..start + take].copy_from_slice(&pes[at..at + take]);
        out.extend_from_slice(&p);
        at += take;
    }
}

/// A packet that carries a PCR and nothing else, for a stream whose pictures come slower than the PCR is due. It repeats the last
/// continuity count, as a packet without payload does.
fn pcr_only(pid: u16, cc: u8, (pcr, jump): (u64, bool), out: &mut Vec<u8>) {
    let mut p = [0xFF; PACKET];
    p[0] = 0x47;
    p[1] = (pid >> 8) as u8;
    p[2] = pid as u8;
    p[3] = 0x20 | (cc & 15);
    p[4] = 183;
    p[5] = 0x10 | u8::from(jump) << 7;
    p[6..12].fill(0);
    splice::write_pcr(&mut p, pcr);
    out.extend_from_slice(&p);
}

/// A converted program: the picture's PES goes to the converter, its units come back as a PES of H.264, and everything else the
/// station sent goes on as it was, a little later, so the sound arrives with its picture.
pub(crate) struct Stage {
    convert: Box<dyn Convert>,
    video: u16,
    pmt_pid: u16,
    /// The station's PMT with the picture's stream type 0x1B and no descriptors on it (they describe MPEG-2: the caption service rides
    /// in its pictures' user data, which the converter does not carry).
    pmt: Vec<u8>,
    /// The picture's clock rides on its PID: the station's PCRs there are replaced.
    carries_clock: bool,
    gather: Gather,
    /// The pictures pushed and when, to find how long one took, and when the oldest unanswered one was pushed.
    pushed: VecDeque<(u64, Duration)>,
    waiting_since: Option<Duration>,
    last_push: Duration,
    /// A picture of the station's own PTS has none: it is the last one's and a frame.
    frame: u64,
    last_pts: Option<u64>,
    units: Vec<Unit>,
    scratch: Vec<u8>,
    cc: u8,
    station: Station,
    last_pcr: Duration,
    /// How far the stream runs behind the station: what the first unit took. Not known until it has gone out, which is when the
    /// stream starts.
    lag: Option<Duration>,
    /// The packets of the other PIDs and when they came, until their picture is out.
    held: VecDeque<(Duration, [u8; PACKET])>,
    stall: Duration,
    lag_max: Duration,
}

impl Stage {
    pub(crate) fn new(convert: Box<dyn Convert>, pmt: &Pmt, pmt_pid: u16, video: u16, format: &Format, limits: &Limits) -> Self {
        let streams: Vec<(u8, u16, &[u8])> = pmt
            .streams
            .iter()
            .map(|e| match e.pid == video {
                true => (0x1B, e.pid, &[][..]),
                false => (e.stream_type, e.pid, &e.descriptors[..]),
            })
            .collect();
        Self {
            convert,
            video,
            pmt_pid,
            pmt: psi::build_pmt(pmt.program, 0, pmt.pcr_pid, &pmt.descriptors, &streams),
            carries_clock: pmt.pcr_pid == video,
            gather: Gather::default(),
            pushed: VecDeque::new(),
            waiting_since: None,
            last_push: Duration::ZERO,
            frame: format.ticks(1),
            last_pts: None,
            units: Vec::new(),
            scratch: Vec::new(),
            cc: 0,
            station: Station::default(),
            last_pcr: Duration::ZERO,
            lag: None,
            held: VecDeque::new(),
            stall: limits.convert_stall,
            lag_max: limits.convert_lag,
        }
    }

    /// The PMT the stream sends in the station's place.
    pub(crate) fn pmt(&self) -> &[u8] {
        &self.pmt
    }

    /// Takes the packets that were read at `now` (none, to let what the converter has made and what is held come out) and adds what
    /// goes out to `out`. The station's PAT and PMT are dropped: the stream sends its own.
    pub(crate) fn feed(&mut self, packets: &[u8], now: Duration, out: &mut Vec<u8>) -> Result<(), Fault> {
        for p in packets.chunks_exact(PACKET) {
            let pid = psi::pid_of_packet(p);
            if pid == self.video {
                if p[1] & 0x80 != 0 {
                    continue;
                }
                if self.carries_clock
                    && let Some(pcr) = splice::pcr(p)
                {
                    self.station.seen(pcr, now);
                }
                if let Some(picture) = self.gather.push(p) {
                    self.push(picture, now)?;
                }
            } else if pid != self.pmt_pid && pid != psi::PID_PAT {
                self.held.push_back((now, p.try_into().unwrap_or([0; PACKET])));
            }
        }
        self.collect(now, out)
    }

    fn push(&mut self, picture: Picture, now: Duration) -> Result<(), Fault> {
        let pts = picture
            .pts
            .or_else(|| self.last_pts.map(|last| splice::add(last, self.frame)))
            .unwrap_or_default();
        self.last_pts = Some(pts);
        self.waiting_since.get_or_insert(now);
        self.last_push = now;
        if self.pushed.len() == PENDING_MAX {
            self.pushed.pop_front();
        }
        self.pushed.push_back((pts, now));
        self.convert.push(&picture.es, pts)
    }

    fn collect(&mut self, now: Duration, out: &mut Vec<u8>) -> Result<(), Fault> {
        self.units.clear();
        self.convert.pull(&mut self.units)?;
        for unit in std::mem::take(&mut self.units) {
            self.emit(&unit, now, out)?;
        }
        // Pictures are still coming and none has come back for `stall`: the converter is stuck. A station that has gone quiet is the
        // stream's to end, not a fault of the converter.
        if let Some(since) = self.waiting_since
            && now.saturating_sub(since) >= self.stall
            && now.saturating_sub(self.last_push) < self.stall
        {
            return Err(Fault::Stalled(format!("no picture came back for {} ms", self.stall.as_millis())));
        }
        if let Some(lag) = self.lag {
            while let Some((at, _)) = self.held.front() {
                if now < *at + lag {
                    break;
                }
                if let Some((_, packet)) = self.held.pop_front() {
                    out.extend_from_slice(&packet);
                }
            }
            self.keep_clock(lag, now, out);
        } else {
            while self.held.len() > HELD_MAX {
                self.held.pop_front();
            }
        }
        Ok(())
    }

    /// How long the picture `pts` was in the converter, if it is remembered.
    fn took(&mut self, pts: u64, now: Duration) -> Option<Duration> {
        let at = self.pushed.iter().position(|&(p, _)| p == pts)?;
        let (_, since) = self.pushed.remove(at)?;
        // what is older than a stall was dropped to lower the rate: it is not coming back
        while self.pushed.front().is_some_and(|&(_, pushed)| now.saturating_sub(pushed) > self.stall * 2) {
            self.pushed.pop_front();
        }
        Some(now.saturating_sub(since))
    }

    fn emit(&mut self, unit: &Unit, now: Duration, out: &mut Vec<u8>) -> Result<(), Fault> {
        let took = self.took(unit.pts, now);
        self.waiting_since = None;
        if let Some(took) = took
            && took > self.lag_max
        {
            return Err(Fault::Stalled(format!("a picture took {} ms", took.as_millis())));
        }
        let nals = Nals::of(&unit.data);
        let lag = match self.lag {
            Some(lag) => lag,
            // a stream starts where a player can start: units before the first key one are dropped
            None if !nals.key() => return Ok(()),
            None => *self.lag.insert(took.unwrap_or(FIRST_LAG)),
        };
        let clock = self.carries_clock.then(|| self.station.pcr(now, lag)).flatten();
        self.scratch.clear();
        // 6 bytes of header, then a PES head with a PTS and no DTS: the pictures are shown in the order they are decoded
        self.scratch.extend_from_slice(&[0, 0, 1, 0xE0, 0, 0, 0x84, 0x80, 5, 0x20, 0, 0, 0, 0]);
        splice::write_stamp(&mut self.scratch, 9, unit.pts);
        if !nals.aud {
            self.scratch.extend_from_slice(&[0, 0, 0, 1, 0x09, 0xF0]);
        }
        self.scratch.extend_from_slice(&unit.data);
        packetize(self.video, &mut self.cc, &self.scratch, clock, nals.key(), out);
        if clock.is_some() {
            self.last_pcr = now;
        }
        Ok(())
    }

    /// A PCR on its own when the pictures have not carried one for `PCR_EVERY`.
    fn keep_clock(&mut self, lag: Duration, now: Duration, out: &mut Vec<u8>) {
        if self.carries_clock
            && now.saturating_sub(self.last_pcr) >= PCR_EVERY
            && let Some(clock) = self.station.pcr(now, lag)
        {
            pcr_only(self.video, self.cc.wrapping_sub(1), clock, out);
            self.last_pcr = now;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::psi::Es;
    use std::sync::Arc;

    pub(crate) const VIDEO: u16 = 0x61;
    pub(crate) const AUDIO: u16 = 0x64;
    pub(crate) const PMT: u16 = 0x60;

    /// 1080i at 29.97, the shape of most main channels.
    pub(crate) fn hd() -> Format {
        Format { width: 1920, height: 1080, rate: 4, aspect: 3, progressive: false }
    }

    /// What the fake converter does and what it was given.
    #[derive(Default)]
    pub(crate) struct Plan {
        /// Does not open, for this.
        pub(crate) open: Option<Fault>,
        /// A picture is answered once this many more have come.
        pub(crate) delay: usize,
        /// The unit that is the first key one, and the gap between the key ones after it.
        pub(crate) first_key: usize,
        pub(crate) gop: usize,
        /// Fails the nth picture (from 1), or takes pictures and gives nothing back.
        pub(crate) fail_at: Option<usize>,
        pub(crate) mute: bool,
        /// How often it was asked to open, and what the opens that did not fail were for.
        pub(crate) opens: usize,
        pub(crate) opened: Vec<(Format, Want)>,
        pub(crate) pictures: Vec<(u64, Vec<u8>)>,
        pub(crate) closed: usize,
    }

    #[derive(Clone, Default)]
    pub(crate) struct Fake(pub(crate) Arc<Mutex<Plan>>);

    impl Codec for Fake {
        fn open(&self, source: &Format, want: &Want) -> Result<Box<dyn Convert>, Fault> {
            let mut plan = self.0.lock();
            plan.opens += 1;
            if let Some(fault) = plan.open.clone() {
                return Err(fault);
            }
            plan.opened.push((*source, *want));
            Ok(Box::new(Echo { plan: self.0.clone(), pending: VecDeque::new(), made: 0, ready: Vec::new() }))
        }
    }

    /// A unit that a player can start from, or one that follows; the byte after the slice header says which picture.
    pub(crate) fn unit(pts: u64, key: bool) -> Unit {
        let mut data = Vec::new();
        if key {
            data.extend_from_slice(&[0, 0, 0, 1, 0x67, 0x42, 0xC0, 0x1F, 0, 0, 0, 1, 0x68, 0xCE, 0x38, 0x80]);
            data.extend_from_slice(&[0, 0, 1, 0x65, 0x88]);
        } else {
            data.extend_from_slice(&[0, 0, 1, 0x41, 0x9A]);
        }
        data.push(pts as u8);
        Unit { pts, data }
    }

    struct Echo {
        plan: Arc<Mutex<Plan>>,
        pending: VecDeque<u64>,
        made: usize,
        ready: Vec<Unit>,
    }

    impl Convert for Echo {
        fn push(&mut self, picture: &[u8], pts: u64) -> Result<(), Fault> {
            let mut plan = self.plan.lock();
            plan.pictures.push((pts, picture.to_vec()));
            if plan.fail_at == Some(plan.pictures.len()) {
                return Err(Fault::Failed("the encoder said no".to_owned()));
            }
            if plan.mute {
                return Ok(());
            }
            self.pending.push_back(pts);
            while self.pending.len() > plan.delay {
                let pts = self.pending.pop_front().unwrap_or_default();
                let key = self.made >= plan.first_key && (self.made - plan.first_key) % plan.gop.max(1) == 0;
                self.ready.push(unit(pts, key));
                self.made += 1;
            }
            Ok(())
        }

        fn pull(&mut self, out: &mut Vec<Unit>) -> Result<(), Fault> {
            out.append(&mut self.ready);
            Ok(())
        }
    }

    impl Drop for Echo {
        fn drop(&mut self) {
            self.plan.lock().closed += 1;
        }
    }

    /// An MPEG-2 sequence header and its extension, the first bytes of a picture a decoder can start at.
    pub(crate) fn sequence(f: &Format) -> Vec<u8> {
        let mut es = vec![0, 0, 1, 0xB3, (f.width >> 4) as u8, ((f.width & 15) << 4) as u8 | (f.height >> 8) as u8, f.height as u8];
        es.extend_from_slice(&[f.aspect << 4 | f.rate, 0, 0, 0, 0]);
        es.extend_from_slice(&[0, 0, 1, 0xB5, 0x14, if f.progressive { 0x8A } else { 0x82 }]);
        es
    }

    /// One MPEG-2 picture as a PES with `pts` in a packet of its own: its sequence header, a picture start code and `tag`. `pcr` rides it.
    pub(crate) fn picture(cc: &mut u8, pts: u64, tag: u8, pcr: Option<u64>) -> Vec<u8> {
        let mut pes = vec![0, 0, 1, 0xE0, 0, 0, 0x80, 0x80, 5, 0x20, 0, 0, 0, 0];
        splice::write_stamp(&mut pes, 9, pts);
        pes.extend_from_slice(&sequence(&hd()));
        pes.extend_from_slice(&[0, 0, 1, 0, tag]);
        let mut out = Vec::new();
        packetize(VIDEO, cc, &pes, pcr.map(|p| (p, false)), false, &mut out);
        out
    }

    /// An AC-3 packet that says which one it is by its first payload byte.
    pub(crate) fn sound(cc: u8, tag: u8) -> Vec<u8> {
        let mut p = vec![0xFF; PACKET];
        p[..4].copy_from_slice(&[0x47, (AUDIO >> 8) as u8, AUDIO as u8, 0x10 | cc]);
        p[4] = tag;
        p
    }

    pub(crate) fn station_pmt() -> Pmt {
        Pmt {
            program: 4,
            pcr_pid: VIDEO,
            tags: Vec::new(),
            descriptors: Vec::new(),
            streams: vec![
                Es { stream_type: 0x02, pid: VIDEO, tags: vec![0x86], descriptors: vec![0x86, 1, 0] },
                Es { stream_type: 0x81, pid: AUDIO, tags: vec![0x81], descriptors: vec![0x81, 1, 0x38] },
            ],
        }
    }

    fn stage(fake: &Fake, limits: &Limits) -> Stage {
        let f = hd();
        let convert = fake.open(&f, &Profile::Mobile.want(&f)).unwrap();
        Stage::new(convert, &station_pmt(), PMT, VIDEO, &f, limits)
    }

    /// A packet as the tests read it back, with nothing of the writer's in it but the bytes.
    #[derive(Debug)]
    pub(crate) struct Read {
        pub(crate) pid: u16,
        pub(crate) start: bool,
        pub(crate) cc: u8,
        pub(crate) pcr: Option<u64>,
        pub(crate) jump: bool,
        pub(crate) key: bool,
        pub(crate) payload: Vec<u8>,
    }

    pub(crate) fn read(ts: &[u8]) -> Vec<Read> {
        assert_eq!(ts.len() % PACKET, 0);
        ts.chunks_exact(PACKET)
            .map(|p| {
                assert_eq!(p[0], 0x47);
                let field = if p[3] & 0x20 != 0 { 1 + usize::from(p[4]) } else { 0 };
                Read {
                    pid: psi::pid_of_packet(p),
                    start: p[1] & 0x40 != 0,
                    cc: p[3] & 15,
                    pcr: splice::pcr(p),
                    jump: field > 1 && p[5] & 0x80 != 0,
                    key: field > 1 && p[5] & 0x40 != 0,
                    payload: if p[3] & 0x10 != 0 { p[4 + field..].to_vec() } else { Vec::new() },
                }
            })
            .collect()
    }

    /// The video PESs of a stream: their PTS, the ES after the head, whether they were marked random access, and the PCR of the first packet.
    pub(crate) fn pictures(packets: &[Read]) -> Vec<(u64, Vec<u8>, bool, Option<u64>)> {
        let mut all: Vec<(u64, Vec<u8>, bool, Option<u64>)> = Vec::new();
        for p in packets.iter().filter(|p| p.pid == VIDEO && !p.payload.is_empty()) {
            if p.start {
                let h = &p.payload;
                assert_eq!(h[..4], [0, 0, 1, 0xE0]);
                assert_eq!(h[6..9], [0x84, 0x80, 5], "data alignment, a PTS and no DTS");
                let b = &h[9..14];
                let pts = (u64::from(b[0] >> 1 & 7) << 30) | (u64::from(b[1]) << 22) | (u64::from(b[2] >> 1) << 15) | (u64::from(b[3]) << 7) | u64::from(b[4] >> 1);
                all.push((pts, h[14..].to_vec(), p.key, p.pcr));
            } else if let Some(last) = all.last_mut() {
                last.1.extend_from_slice(&p.payload);
            }
        }
        all
    }

    #[test]
    fn profiles_ask_what_silicondust_names() {
        let hd1080 = hd();
        let hd720 = Format { width: 1280, height: 720, rate: 7, aspect: 3, progressive: true };
        let sd43 = Format { width: 720, height: 480, rate: 4, aspect: 2, progressive: false };
        let sd169 = Format { aspect: 3, ..sd43 };
        let size = |p: Profile, f: &Format| {
            let w = p.want(f);
            (w.width, w.height)
        };
        assert_eq!(Profile::Heavy.want(&hd1080), Want { width: 1920, height: 1080, rate: (30000, 1001), kbps: 9000 });
        assert_eq!(Profile::Heavy.want(&hd720), Want { width: 1280, height: 720, rate: (60000, 1001), kbps: 7500 }, "heavy keeps the size and the rate");
        assert_eq!(Profile::Heavy.want(&Format { rate: 4, ..hd720 }).kbps, 5000, "720p30 is half the pictures of 720p60");
        assert_eq!(Profile::Heavy.want(&sd169), Want { width: 720, height: 480, rate: (30000, 1001), kbps: 2500 });
        assert_eq!(Profile::Mobile.want(&hd1080), Want { width: 1280, height: 720, rate: (30000, 1001), kbps: 3500 });
        assert_eq!(Profile::Mobile.want(&hd720).rate, (30000, 1001), "59.94 is halved");
        assert_eq!(Profile::Mobile.want(&Format { rate: 6, ..hd720 }).rate, (25, 1), "and 50 is halved");
        assert_eq!(Profile::Mobile.want(&Format { rate: 5, ..hd720 }).rate, (30, 1), "30 is kept");
        assert_eq!(
            [Profile::Internet540, Profile::Internet480, Profile::Internet360, Profile::Internet240].map(|p| size(p, &hd1080)),
            [(960, 540), (848, 480), (640, 360), (432, 240)]
        );
        assert_eq!(
            [Profile::Mobile, Profile::Internet480, Profile::Internet360, Profile::Internet240].map(|p| size(p, &sd43)),
            [(640, 480), (640, 480), (480, 360), (320, 240)],
            "4:3 standard definition: square pixels, and nothing is enlarged"
        );
        assert_eq!(size(Profile::Mobile, &sd169), (848, 480));
        assert_eq!(Profile::parse("Mobile"), Some(Profile::Mobile));
        assert_eq!(Profile::parse("internet240"), Some(Profile::Internet240));
        assert_eq!(Profile::parse("fast"), None);
    }

    #[test]
    fn the_status_page_describes_each_profile_from_the_box_and_the_rate_want_uses() {
        let json = |name: &str, max_height: Option<u32>, kbps: u32| {
            serde_json::json!({ "name": name, "codec": "h264", "maxHeight": max_height, "progressive": true, "kbps": kbps })
        };
        assert_eq!(
            Profile::ALL.map(Profile::describe),
            [
                json("heavy", None, 9000),
                json("mobile", Some(720), 3500),
                json("internet540", Some(540), 2000),
                json("internet480", Some(480), 1200),
                json("internet360", Some(360), 800),
                json("internet240", Some(240), 400),
            ]
        );
        for profile in Profile::ALL {
            let (described, asked) = (profile.describe(), profile.want(&hd()));
            assert_eq!(described["kbps"], asked.kbps, "{profile:?}: the top rate is what a 1080 station gets");
            if profile != Profile::Heavy {
                assert_eq!(described["maxHeight"], asked.height, "{profile:?}: the height of the biggest picture it makes");
            }
        }
        assert_eq!(Profile::ALL.map(Profile::name), ["heavy", "mobile", "internet540", "internet480", "internet360", "internet240"]);
    }

    #[test]
    fn a_failed_converter_rests_and_a_hung_one_does_not_come_back() {
        let b = Breaker::default();
        let (now, rest) = (Duration::from_secs(100), Duration::from_secs(60));
        assert!(b.allows(now));
        b.failed(&Fault::Unsupported("H.264 already".to_owned()), now, rest);
        assert!(b.allows(now), "a picture it cannot take is no failure");
        b.failed(&Fault::Failed("x".to_owned()), now, rest);
        assert!(!b.allows(now + Duration::from_secs(59)));
        assert!(b.allows(now + rest));
        b.failed(&Fault::Wedged("x".to_owned()), now, rest);
        assert!(!b.allows(now + Duration::from_secs(86_400)));
        b.failed(&Fault::Failed("x".to_owned()), now, rest);
        assert!(!b.allows(now + Duration::from_secs(86_400)), "a later failure does not lift it");
    }

    #[test]
    fn nals_are_found_behind_either_start_code() {
        let n = Nals::of(&[0, 0, 0, 1, 9, 0xF0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x65, 3]);
        assert!(n.aud && n.sps && n.idr && n.key());
        let n = Nals::of(&[0, 0, 1, 0x67, 1, 0, 0, 1, 9, 0xF0]);
        assert!(!n.aud && !n.idr && !n.key(), "a delimiter that is not first does not count, and an SPS alone is not a key unit");
        assert!(!Nals::of(&[0, 0, 1]).key() && !Nals::of(&[]).key());
    }

    #[test]
    fn a_pes_is_cut_into_whole_packets_that_put_it_back() {
        for len in [1, 100, 176, 177, 182, 183, 184, 185, 366, 367, 368, 369, 1000] {
            let pes: Vec<u8> = (0..len).map(|i| i as u8).collect();
            for clock in [None, Some((0x1_2345_6789 & (WRAP - 1), true))] {
                let (mut cc, mut out) = (14, Vec::new());
                packetize(VIDEO, &mut cc, &pes, clock, true, &mut out);
                let read = read(&out);
                assert!(read[0].start && read[1..].iter().all(|p| !p.start), "{len}");
                assert_eq!(read[0].pcr, clock.map(|c| c.0), "{len}");
                assert_eq!((read[0].jump, read[0].key), (clock.is_some(), true));
                let ccs: Vec<u8> = read.iter().map(|p| p.cc).collect();
                assert_eq!(ccs[0], 14);
                assert!(ccs.windows(2).all(|w| w[1] == (w[0] + 1) & 15), "the counter runs on and wraps: {len}");
                assert_eq!(cc, (14 + read.len() as u8) & 15);
                let back: Vec<u8> = read.iter().flat_map(|p| p.payload.clone()).collect();
                assert_eq!(back, pes, "{len}");
            }
        }
    }

    #[test]
    fn the_picture_goes_out_as_h264_and_the_sound_goes_with_it_unchanged() {
        let fake = Fake::default();
        fake.0.lock().delay = 2;
        fake.0.lock().first_key = 0;
        fake.0.lock().gop = 3;
        let mut s = stage(&fake, &Limits::default());
        assert_eq!(
            psi::parse_pmt(s.pmt()).map(|p| p.streams.iter().map(|e| (e.stream_type, e.pid, e.descriptors.clone())).collect::<Vec<_>>()),
            Some(vec![(0x1B, VIDEO, vec![]), (0x81, AUDIO, vec![0x81, 1, 0x38])]),
            "the picture is H.264 with no MPEG-2 descriptors, the sound is as it was"
        );
        let (mut out, mut vcc) = (Vec::new(), 0);
        let mut sent = Vec::new();
        for i in 0..8u8 {
            let now = Duration::from_millis(u64::from(i) * 33);
            let mut buf = picture(&mut vcc, 90_000 + u64::from(i) * 3003, i, Some(1_000_000 + u64::from(i) * 2970));
            buf.extend(sound(i, 0x10 + i));
            sent.push(0x10 + i);
            buf.extend(vec![0x47, 0, 0x60, 0x10, 1].into_iter().chain([0xFF; PACKET - 5])); // the station's PMT, to be dropped
            s.feed(&buf, now, &mut out).unwrap();
        }
        s.feed(&[], Duration::from_millis(1000), &mut out).unwrap();
        let ts = read(&out);
        assert!(ts.iter().all(|p| p.pid == VIDEO || p.pid == AUDIO), "the station's own tables are not passed on");
        let h264 = pictures(&ts);
        assert!(h264.len() >= 5, "{}", h264.len());
        let sent_pts: Vec<u64> = (0..8).map(|i| 90_000 + i * 3003).collect();
        for (i, (pts, es, key, pcr)) in h264.iter().enumerate() {
            assert_eq!(*pts, sent_pts[i], "the picture keeps the PTS of its source");
            assert!(es.starts_with(&[0, 0, 0, 1, 9, 0xF0]), "an access unit delimiter leads");
            assert_eq!(*key, i % 3 == 0, "the random access flag is on the key units");
            assert!(pcr.is_some(), "every picture carries the clock");
        }
        let pcrs: Vec<u64> = h264.iter().filter_map(|p| p.3).collect();
        assert!(pcrs.windows(2).all(|w| w[1] > w[0]), "{pcrs:?}");
        let video: Vec<u8> = ts.iter().filter(|p| p.pid == VIDEO && !p.payload.is_empty()).map(|p| p.cc).collect();
        assert!(video.windows(2).all(|w| w[1] == (w[0] + 1) & 15), "the counter runs on: {video:?}");
        let heard: Vec<u8> = ts.iter().filter(|p| p.pid == AUDIO).map(|p| p.payload[0]).collect();
        assert_eq!(heard, sent[..heard.len()], "the sound is in order and unchanged");
        assert_eq!(heard.len(), sent.len(), "all of it came out once the lag had passed");
        assert_eq!(fake.0.lock().pictures.len(), 7, "a picture goes in when the next one starts: the eighth is still being read");
        assert!(fake.0.lock().pictures[0].1.starts_with(&[0, 0, 1, 0xB3]), "the converter is given the picture's ES, from its sequence header");
        drop(s);
        assert_eq!(fake.0.lock().closed, 1, "the converter is given back with the stage");
    }

    #[test]
    fn nothing_goes_out_before_a_unit_a_player_can_start_from_and_the_sound_waits_for_it() {
        let fake = Fake::default();
        fake.0.lock().first_key = 2;
        fake.0.lock().gop = 30;
        let mut s = stage(&fake, &Limits::default());
        let (mut out, mut vcc) = (Vec::new(), 0);
        for i in 0..2u8 {
            let mut buf = picture(&mut vcc, 90_000 + u64::from(i) * 3003, i, None);
            buf.extend(sound(i, 0x20 + i));
            s.feed(&buf, Duration::from_millis(u64::from(i) * 100), &mut out).unwrap();
            s.feed(&[], Duration::from_millis(u64::from(i) * 100 + 50), &mut out).unwrap();
        }
        assert!(out.is_empty(), "two non-key units and the sound that came with them");
        let mut buf = picture(&mut vcc, 90_000 + 2 * 3003, 2, None);
        buf.extend(picture(&mut vcc, 90_000 + 3 * 3003, 3, None));
        s.feed(&buf, Duration::from_millis(200), &mut out).unwrap();
        let ts = read(&out);
        assert_eq!(pictures(&ts).first().map(|p| (p.0, p.2)), Some((90_000 + 2 * 3003, true)), "the first picture out is the key one");
        assert_eq!(ts.iter().filter(|p| p.pid == AUDIO).count(), 2, "the sound follows");
    }

    #[test]
    fn the_sound_is_held_for_as_long_as_the_picture_took() {
        let fake = Fake::default();
        fake.0.lock().delay = 1;
        let mut s = stage(&fake, &Limits::default());
        let (mut out, mut vcc) = (Vec::new(), 0);
        let ms = Duration::from_millis;
        let heard = |out: &[u8]| read(out).iter().filter(|p| p.pid == AUDIO).count();
        s.feed(&picture(&mut vcc, 90_000, 0, None), ms(0), &mut out).unwrap();
        // a picture goes in when the next one starts: picture 0 at 40, and its unit comes out when picture 2 starts, at 80
        s.feed(&picture(&mut vcc, 93_003, 1, None), ms(40), &mut out).unwrap();
        s.feed(&sound(0, 1), ms(50), &mut out).unwrap();
        s.feed(&picture(&mut vcc, 96_006, 2, None), ms(80), &mut out).unwrap();
        assert_eq!((pictures(&read(&out)).len(), heard(&out)), (1, 0), "the picture took 40 ms: the sound that came at 50 is due at 90");
        s.feed(&[], ms(89), &mut out).unwrap();
        assert_eq!(heard(&out), 0);
        s.feed(&[], ms(90), &mut out).unwrap();
        assert_eq!(heard(&out), 1);
    }

    #[test]
    fn a_converter_that_fails_or_stops_answering_is_a_fault() {
        let limits = Limits { convert_stall: Duration::from_secs(2), convert_lag: Duration::from_secs(1), ..Limits::default() };
        // fails
        let fake = Fake::default();
        fake.0.lock().fail_at = Some(2);
        let (mut s, mut out, mut vcc) = (stage(&fake, &limits), Vec::new(), 0);
        s.feed(&picture(&mut vcc, 90_000, 0, None), Duration::ZERO, &mut out).unwrap();
        s.feed(&picture(&mut vcc, 93_003, 1, None), Duration::from_millis(33), &mut out).unwrap();
        assert_eq!(
            s.feed(&picture(&mut vcc, 96_006, 2, None), Duration::from_millis(66), &mut out),
            Err(Fault::Failed("the encoder said no".to_owned()))
        );
        // takes pictures and gives nothing back while the station keeps sending: a stall once the oldest has waited 2 s (the first
        // picture goes in when the second starts, at 500 ms)
        let fake = Fake::default();
        fake.0.lock().mute = true;
        let (mut s, mut out, mut vcc) = (stage(&fake, &limits), Vec::new(), 0);
        for i in 0..7u8 {
            let now = Duration::from_millis(u64::from(i) * 500);
            let fed = s.feed(&picture(&mut vcc, 90_000 + u64::from(i) * 3003, i, None), now, &mut out);
            assert_eq!(fed.is_err(), now >= Duration::from_millis(2500), "{now:?}");
        }
        // the station going quiet is not the converter's fault
        let fake = Fake::default();
        fake.0.lock().mute = true;
        let (mut s, mut out, mut vcc) = (stage(&fake, &limits), Vec::new(), 0);
        s.feed(&picture(&mut vcc, 90_000, 0, None), Duration::ZERO, &mut out).unwrap();
        s.feed(&picture(&mut vcc, 93_003, 1, None), Duration::from_millis(100), &mut out).unwrap();
        s.feed(&[], Duration::from_secs(10), &mut out).unwrap();
        // answers, but later and later: picture 0 goes in at 400 and its unit comes out at 1600
        let fake = Fake::default();
        fake.0.lock().delay = 3;
        let (mut s, mut out, mut vcc) = (stage(&fake, &limits), Vec::new(), 0);
        let mut fed = Ok(());
        for i in 0..5u8 {
            fed = s.feed(&picture(&mut vcc, 90_000 + u64::from(i) * 3003, i, None), Duration::from_millis(u64::from(i) * 400), &mut out);
        }
        assert!(matches!(fed, Err(Fault::Stalled(_))), "a picture that took 1.2 s against a limit of 1 s: {fed:?}");
    }

    #[test]
    fn the_pcr_follows_the_stations_clock_without_its_jitter_and_flags_a_jump() {
        let fake = Fake::default();
        let (mut s, mut out, mut vcc) = (stage(&fake, &Limits::default()), Vec::new(), 0);
        let jitter = [8i64, -6, 3, 0, -8, 7, -2, 5, -4, 1];
        let mut at_ms = 0i64;
        let mut station = 1_000_000u64;
        for i in 0..40usize {
            at_ms = i as i64 * 33;
            station += 2970;
            let read_at = Duration::from_millis((at_ms + jitter[i % jitter.len()]).max(0) as u64);
            s.feed(&picture(&mut vcc, 90_000 + station - 1_000_000, i as u8, Some(station)), read_at, &mut out).unwrap();
        }
        let pcrs: Vec<u64> = pictures(&read(&out)).iter().filter_map(|p| p.3).collect();
        assert!(pcrs.windows(2).all(|w| w[1] >= w[0]), "never back: {pcrs:?}");
        // the lag was zero: the stream is the station's clock as it was read, within the jitter's average
        let last = *pcrs.last().unwrap() as i64;
        assert!((last - station as i64).abs() < 450, "{last} against {station}");
        // the station's encoder restarts: its clock is ten seconds on, and the next PCR says so
        let before = read(&out).len();
        station += 900_000;
        s.feed(&picture(&mut vcc, 90_000, 99, Some(station)), Duration::from_millis(at_ms as u64 + 33), &mut out).unwrap();
        let after = read(&out);
        let next = after[before..].iter().find(|p| p.pcr.is_some()).expect("a PCR");
        assert!(next.jump, "the discontinuity indicator is set");
        assert!((next.pcr.unwrap() as i64 - station as i64).abs() < 10, "and the clock is the new one at once");
    }

    #[test]
    fn a_pcr_goes_out_on_its_own_while_the_pictures_are_slow() {
        let fake = Fake::default();
        let (mut s, mut out, mut vcc) = (stage(&fake, &Limits::default()), Vec::new(), 0);
        let ms = Duration::from_millis;
        s.feed(&picture(&mut vcc, 90_000, 0, Some(5_000_000)), ms(0), &mut out).unwrap();
        s.feed(&picture(&mut vcc, 93_003, 1, Some(5_000_900)), ms(10), &mut out).unwrap();
        let before = read(&out).len();
        assert_eq!(before, 1, "picture 0 is out, with a PCR");
        s.feed(&[], ms(49), &mut out).unwrap();
        assert_eq!(read(&out).len(), before, "not due until 40 ms after the last");
        s.feed(&[], ms(50), &mut out).unwrap();
        let ts = read(&out);
        let alone = ts.last().unwrap();
        assert_eq!((alone.pid, alone.pcr.is_some(), alone.payload.len()), (VIDEO, true, 0));
        assert_eq!(alone.cc, ts[ts.len() - 2].cc, "it repeats the last count: it has no payload");
    }
}
