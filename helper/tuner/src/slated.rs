//! A stream with slates (the owner's switch, on by default): answered at once, it shows a slate while the channel is tuned, the
//! programme from its first whole picture on, a slate again whenever the picture stops or breaks up, and a last slate before it ends.
//! A player shows why it waits instead of failing (Plex's Error 16) or freezing, and a weak channel's stream outlasts a fade.
//!
//! | When | The slate | Then |
//! |---|---|---|
//! | the channel is being tuned and its first whole picture waited for | `Tuning 16.9 WXYZ…`, a bar | the programme at that picture |
//! | no picture after `weak_after`, or the picture stopped for `dropout`, or a second lost `loss_weak` of its packets | `Weak signal on 16.9`, the signal and the wait | the programme at its next whole picture, once a second has lost at most `loss_clear` |
//! | no picture for `give_up_tuning` (it never came) or `give_up` (it stopped); no lock; not in the multiplex | `No signal on 16.9` for `last_word` | the end |
//! | the TV's own Live TV takes the tuner | `The TV is using its tuner` for `last_word` | the end; the TV has the tuner at once |
//!
//! What cannot be taken at all is still a status, as an HDHomeRun answers and as a player needs to give its tuner up: another client's
//! stream and the TV in use (503), no such channel (404).

use crate::Status;
use crate::docs::Entry;
use crate::health::{Health, Seen, Verdict, lost_before};
use crate::mpeg2::Format;
use crate::psi::{self, PACKET};
use crate::slate::{Look, Says, Slate};
use crate::splice::{Layout, Program, Routes};
use crate::stream::{End, Facade, Failure, Lease, Over, Signal, Tap, Unfound, program};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// A request that has the tuner, to be answered with slates.
pub(crate) struct Slated {
    pub(crate) entry: Entry,
    pub(crate) lease: Lease,
    pub(crate) look: Look,
}

/// What goes out: the program, the slate that stands in for its picture, and when that slate's frames are due.
struct Show {
    program: Program,
    slate: Slate,
    /// When the slate's first frame was due, and how many have gone out since.
    anchor: Duration,
    frames: u64,
    tables: Option<Duration>,
    out: Vec<u8>,
}

impl Show {
    /// The tables when they are due, the slate's frames due by `now` while it stands in, then out with all of it.
    fn tick(&mut self, now: Duration, every: Duration, w: &mut dyn Write) -> io::Result<()> {
        if self.tables.is_none_or(|t| now.saturating_sub(t) >= every) {
            self.program.tables(&mut self.out);
            self.tables = Some(now);
        }
        if !self.program.is_live() {
            while self.due(self.frames) <= now {
                self.frame();
            }
        }
        self.flush(w)
    }

    fn frame(&mut self) {
        let picture = self.slate.next_picture();
        self.program.slate_frame(&picture, &mut self.out);
        self.frames += 1;
    }

    fn flush(&mut self, w: &mut dyn Write) -> io::Result<()> {
        if !self.out.is_empty() {
            w.write_all(&self.out)?;
            self.out.clear();
        }
        Ok(())
    }

    fn due(&self, frame: u64) -> Duration {
        self.anchor + Duration::from_micros(self.slate.format().ticks(frame) * 100 / 9)
    }

    /// The slate stands in again from `since`, when the picture stopped: its frames since then go out at once.
    fn stand_in(&mut self, slate: Slate, since: Duration) {
        self.program.splice_out(slate.format(), &mut self.out);
        self.slate = slate;
        self.anchor = since;
        self.frames = 0;
    }

    /// `long` worth of the slate at once, the stream's last: a player plays it out after the stream has closed.
    fn last_word(&mut self, long: Duration, w: &mut dyn Write) -> io::Result<()> {
        let frames = (long.as_secs_f64() * self.slate.format().fps()).round() as u64;
        self.program.tables(&mut self.out);
        for _ in 0..frames {
            self.frame();
        }
        self.flush(w)
    }
}

/// The station's picture as it arrives: when its last packet came, its packets lost by continuity count (in the second going on, the last
/// whole second and the stream), and while a slate stands in, the whole picture the programme can start at, gathered with what comes
/// along with it.
struct Watch {
    pid: u16,
    cc: Option<u8>,
    seen: Duration,
    second: (Duration, u32, u32),
    /// The last whole second's share lost and count lost.
    last: Option<(f32, u32)>,
    got: u64,
    lost: u64,
    gathered: Option<(Vec<u8>, bool)>,
}

impl Watch {
    fn new(pid: u16, now: Duration) -> Self {
        Self {
            pid,
            cc: None,
            seen: now,
            second: (now, 0, 0),
            last: None,
            got: 0,
            lost: 0,
            gathered: None,
        }
    }

    /// The packets lost before picture packet `p`.
    fn lost_before(&self, p: &[u8]) -> u32 {
        if p[3] & 0x10 == 0 {
            return 0;
        }
        self.cc.map_or(0, |last| lost_before(last, p[3] & 0x0F))
    }

    /// Counts picture packet `p`; whether packets were lost before it.
    fn count(&mut self, p: &[u8], now: Duration) -> bool {
        self.seen = now;
        if p[3] & 0x10 == 0 {
            return false;
        }
        let lost = self.lost_before(p);
        self.cc = Some(p[3] & 0x0F);
        self.second.1 += 1;
        self.second.2 += lost;
        self.got += 1;
        self.lost += u64::from(lost);
        lost > 0
    }

    fn roll(&mut self, now: Duration) {
        if now.saturating_sub(self.second.0) >= Duration::from_secs(1) {
            let (_, got, lost) = self.second;
            self.last = (got + lost > 0).then(|| (lost as f32 / (got + lost) as f32, lost));
            self.second = (now, 0, 0);
        }
    }

    /// The programme's packets while it plays.
    fn live(&mut self, buf: &[u8], now: Duration) {
        for p in buf.chunks_exact(PACKET) {
            if psi::pid_of_packet(p) == self.pid {
                self.count(p, now);
            }
        }
        self.roll(now);
    }

    /// While a slate stands in: gathers from the first packet of a whole picture (an MPEG-2 sequence header) to the first of the next. When
    /// none of the picture's own packets was lost (and, when `clear` says, the last whole second lost at most that share), gives the
    /// gathered packets and where in `buf` the programme goes on: at the next picture's first packet, not yet counted.
    fn gate(&mut self, buf: &[u8], now: Duration, clear: Option<f32>) -> Option<(Vec<u8>, usize)> {
        for (i, p) in buf.chunks_exact(PACKET).enumerate() {
            let mine = psi::pid_of_packet(p) == self.pid;
            if mine && p[1] & 0x40 != 0 {
                let clean = clear.is_none_or(|most| self.last.is_some_and(|(share, _)| share <= most));
                if let Some((gathered, whole)) = self.gathered.take()
                    && whole
                    && clean
                    && self.lost_before(p) == 0
                {
                    return Some((gathered, i * PACKET));
                }
                self.count(p, now);
                if psi::access_point(p, 0x02) {
                    self.gathered = Some((p.to_vec(), true));
                }
                continue;
            }
            if mine && self.count(p, now) {
                if let Some((_, whole)) = self.gathered.as_mut() {
                    *whole = false;
                }
            }
            if let Some((gathered, _)) = self.gathered.as_mut() {
                gathered.extend_from_slice(p);
            }
        }
        self.roll(now);
        None
    }

    /// A slate stands in from here: what was gathered and the seconds counted are let go.
    fn restart(&mut self, now: Duration) {
        self.gathered = None;
        self.second = (now, 0, 0);
        self.last = None;
    }
}

/// How far the tuning bar is: each step of a tune moves it toward the next step's start, as fast as that step took on the set on
/// 2026-11-05 (the lock in 0.35 s, the tables in 0.3 s, the first whole picture in 0.1 to 2.2 s).
fn progress(step: usize, within: Duration) -> f32 {
    const STEPS: [(f32, f32, f32); 3] = [(0.0, 0.35, 0.4), (0.35, 0.2, 0.3), (0.55, 0.4, 1.0)];
    let (from, span, tau) = STEPS[step.min(2)];
    from + span * (1.0 - (-within.as_secs_f32() / tau).exp())
}

/// One stream with slates, from its request to its end. Fields drop in order: the tap lets go of the TV, the tuner says it is free, then
/// the lease lets the next request have it.
struct Play<'a> {
    tap: Option<Box<dyn Tap>>,
    facade: &'a Facade,
    entry: Entry,
    look: Look,
    asked: Duration,
    show: Show,
    signal: Option<Signal>,
    /// The weakest signal read, for the channel's health.
    weakest: Option<Signal>,
    /// When the first whole picture came, how many times it stopped, and its packets that came and were lost.
    pictured: Option<Duration>,
    dropouts: u32,
    lost: (u64, u64),
    over: Over,
    lease: Lease,
}

impl Facade {
    /// Answers `slated` on `out`: the slate at once, the programme when its picture comes, until the client goes, `stop` is set, another
    /// request takes the tuner, the TV takes it or the channel is given up. However it ends, the stream is over from then on.
    pub(crate) fn play(&self, slated: Slated, out: &mut dyn Write, stop: &AtomicBool) -> io::Result<End> {
        let Slated { entry, lease, look } = slated;
        let seen = self.tuning.seen(&entry.guide);
        let format = seen.as_ref().map_or(Format::FALLBACK, |s| s.format);
        let (layout, tsid) = seen.map_or_else(|| (Layout::fallback(1), 1), |s| (s.layout, s.tsid));
        let asked = (self.clock)();
        self.tuning.streaming(&entry);
        let mut play = Play {
            tap: None,
            facade: self,
            show: Show {
                program: Program::new(tsid, layout, &format),
                slate: Slate::new(look.clone(), &entry.guide, &entry.name, format),
                anchor: asked,
                frames: 0,
                tables: None,
                out: Vec::with_capacity(PACKET * 512),
            },
            entry,
            look,
            asked,
            signal: None,
            weakest: None,
            pictured: None,
            dropouts: 0,
            lost: (0, 0),
            over: Over {
                tuning: self.tuning.clone(),
                then: Status::Idle,
            },
            lease,
        };
        let end = play.run(out, stop);
        play.lease.over();
        play.record();
        end
    }
}

impl Play<'_> {
    fn now(&self) -> Duration {
        (self.facade.clock)()
    }

    fn tick(&mut self, out: &mut dyn Write) -> io::Result<()> {
        let now = self.now();
        self.show.tick(now, self.facade.limits.pat_every, out)
    }

    fn given_up(&self, stop: &AtomicBool) -> Option<End> {
        if stop.load(Ordering::Relaxed) {
            return Some(End::Stopped);
        }
        self.lease.replaced.load(Ordering::Acquire).then_some(End::Replaced)
    }

    /// The tuning slate's bar at `step`, begun at `since`.
    fn tuning(&mut self, step: usize, since: Duration) {
        let now = self.now();
        self.show.slate.say(Says::Tuning(progress(step, now.saturating_sub(since))));
    }

    fn read_signal(&mut self) {
        self.signal = self.facade.hw.signal();
        if let Some(s) = self.signal
            && self.weakest.is_none_or(|w| s.strength() < w.strength())
        {
            self.weakest = Some(s);
        }
    }

    fn run(&mut self, out: &mut dyn Write, stop: &AtomicBool) -> io::Result<End> {
        let f = self.facade;
        let lim = &f.limits;
        self.tuning(0, self.asked);
        self.tick(out)?;
        let mut tap = match f.hw.tune(&self.entry.channel_id) {
            Ok(tap) => tap,
            Err(why @ Failure::NoSignal(_)) => {
                self.read_signal();
                return self.last(Says::NoSignal, End::Failed(why), out);
            }
            Err(why) => return self.last(Says::Failed, End::Failed(why), out),
        };
        let tuned = self.now();
        self.tuning(1, tuned);
        self.tick(out)?;
        // The tables, as a stream without slates reads them, a second at a time so the slate goes on between tries. A table read whole that
        // does not list the channel is a multiplex without it (read again for `section_wait`, as the last multiplex's could still be on the
        // demux); one that does not come whole is a weak signal, tried again until the channel is given up, the slate saying so.
        let try_for = lim.section_wait.min(Duration::from_secs(1));
        let mut last_signal = tuned;
        let mut sections = Vec::new();
        let (channel, pat, pmt) = loop {
            if let Some(end) = self.given_up(stop) {
                return Ok(end);
            }
            let now = match program(tap.as_mut(), &self.entry, &mut sections, try_for) {
                Ok((channel, pat, _, _, pmt)) => break (channel, pat, pmt),
                Err(why) => {
                    let now = self.now();
                    let absent = matches!(why, Unfound::Absent(_));
                    if (absent && now.saturating_sub(tuned) >= lim.section_wait) || now.saturating_sub(self.asked) >= lim.give_up_tuning {
                        // the tuner stays where it is while the demodulator is read for the reason
                        self.tap = Some(tap);
                        let failure = f.unfound(&self.entry, why);
                        return self.last(Says::NoSignal, End::Failed(failure), out);
                    }
                    now
                }
            };
            if now.saturating_sub(last_signal) >= Duration::from_secs(1) {
                last_signal = now;
                self.read_signal();
            }
            let waited = now.saturating_sub(self.asked);
            if waited < lim.weak_after {
                self.tuning(1, tuned);
            } else {
                self.show.slate.say(Says::Weak { signal: self.signal, waited });
            }
            self.tick(out)?;
        };
        if psi::protected(&pmt, Some(&channel)) {
            return self.last(Says::Protected, End::Failed(Failure::Protected), out);
        }
        let Some((layout, routes)) = Layout::of(&pmt).filter(|(l, _)| l.video_type == 0x02) else {
            // a slate is MPEG-2: a channel whose picture is not (or which has none) is served without slates from its next request on
            let layout = Layout::of(&pmt).map_or_else(|| Layout { video_type: 0, ..Layout::fallback(pmt.program) }, |(l, _)| l);
            f.tuning.saw(&self.entry.guide, Seen { format: *self.show.slate.format(), layout, tsid: pat.tsid });
            return self.last(Says::Failed, End::Failed(Failure::Hardware("the picture is not MPEG-2".to_owned())), out);
        };
        if let Err(why) = tap.start(&routes.pids()) {
            return self.last(Says::Failed, End::Failed(why), out);
        }
        self.tap = Some(tap);
        self.stream(layout, routes, pat.tsid, out, stop)
    }

    /// The program is routed: the programme in turns with the slate until the end.
    fn stream(&mut self, layout: Layout, routes: Routes, tsid: u16, out: &mut dyn Write, stop: &AtomicBool) -> io::Result<End> {
        let f = self.facade;
        let lim = &f.limits;
        let routed = self.now();
        let mut watch = Watch::new(routes.video, routed);
        // when the slate began standing in (the request, or when the picture stopped), and the format the programme showed
        let mut waiting: Option<Duration> = Some(self.asked);
        let mut format = *self.show.slate.format();
        let mut last_owner = routed;
        let mut last_signal: Option<Duration> = None;
        let mut buf = Vec::with_capacity(PACKET * 512);
        // whether any of the program's packets came at all: a multiplex that brings the others but none of the picture does not carry it
        let mut heard = false;
        let frame = Duration::from_micros(format.ticks(1) * 100 / 9);
        loop {
            if let Some(end) = self.given_up(stop) {
                return Ok(end);
            }
            let now = self.now();
            if now.saturating_sub(last_owner) >= lim.owner_every {
                last_owner = now;
                let taken = f.hw.in_use().or_else(|| self.tap.as_mut().and_then(|t| t.taken()));
                if let Some(why) = taken {
                    self.over.then = Status::Busy(why.clone());
                    return self.last(Says::TvInUse, End::Preempted(why), out);
                }
            }
            // the signal is read every second while the slate shows it, and with the owner check while the programme plays
            let every = if waiting.is_some() { Duration::from_secs(1) } else { lim.owner_every };
            if last_signal.is_none_or(|t| now.saturating_sub(t) >= every) {
                last_signal = Some(now);
                self.read_signal();
            }
            buf.clear();
            let wait = if waiting.is_some() { frame.min(lim.poll) } else { lim.poll };
            if let Err(why) = self.tap.as_mut().expect("routed").read(&mut buf, wait) {
                return self.last(Says::Failed, End::Failed(why), out);
            }
            let now = self.now();
            heard |= !buf.is_empty();
            match waiting {
                None => {
                    watch.live(&buf, now);
                    self.show.program.live(&buf, &mut self.show.out);
                    let stopped = now.saturating_sub(watch.seen) >= lim.dropout;
                    let broken = watch.last.is_some_and(|(share, lost)| share >= lim.loss_weak && lost >= 10);
                    if stopped || broken {
                        let since = if stopped { watch.seen } else { now };
                        self.dropouts += 1;
                        log::info!(
                            "tuner: {} {}; a slate stands in",
                            self.entry.guide,
                            if stopped { "lost its picture" } else { "is breaking up" }
                        );
                        let mut slate = Slate::new(self.look.clone(), &self.entry.guide, &self.entry.name, format);
                        slate.say(Says::Weak { signal: self.signal, waited: Duration::ZERO });
                        self.show.stand_in(slate, since);
                        watch.restart(now);
                        waiting = Some(since);
                    }
                }
                Some(since) => {
                    let clear = self.pictured.is_some().then_some(lim.loss_clear);
                    if let Some((gathered, rest)) = watch.gate(&buf, now, clear) {
                        let first = &gathered[..PACKET];
                        format = Format::of_sequence(&first[4..]).unwrap_or(format);
                        self.show.program.splice_in(layout.clone(), routes.clone(), first, &format);
                        self.show.program.live(&gathered, &mut self.show.out);
                        self.show.program.live(&buf[rest..], &mut self.show.out);
                        watch.live(&buf[rest..], now);
                        if self.pictured.is_none() {
                            self.pictured = Some(now.saturating_sub(self.asked));
                            log::info!(
                                "tuner: {}'s picture came {:.1} s after it was asked for",
                                self.entry.guide,
                                now.saturating_sub(self.asked).as_secs_f64()
                            );
                        } else {
                            log::info!("tuner: {}'s picture is back after {:.1} s", self.entry.guide, now.saturating_sub(since).as_secs_f64());
                        }
                        f.tuning.saw(&self.entry.guide, Seen { format, layout: layout.clone(), tsid });
                        waiting = None;
                    } else {
                        let waited = now.saturating_sub(since);
                        let limit = if self.pictured.is_some() { lim.give_up } else { lim.give_up_tuning };
                        if waited >= limit {
                            let failure = if self.pictured.is_some() {
                                Failure::WeakSignal(format!("the picture stopped ({})", self.entry.guide))
                            } else {
                                f.unfound(
                                    &self.entry,
                                    match (watch.got > 0, heard) {
                                        (true, _) => Unfound::Missing("no whole picture"),
                                        (false, true) => Unfound::Absent("no picture"),
                                        (false, false) => Unfound::Missing("no packets"),
                                    },
                                )
                            };
                            return self.last(Says::NoSignal, End::Failed(failure), out);
                        }
                        if self.pictured.is_none() && waited < lim.weak_after {
                            self.tuning(2, routed);
                        } else {
                            self.show.slate.say(Says::Weak { signal: self.signal, waited });
                        }
                    }
                }
            }
            self.lost = (watch.got, watch.lost);
            self.tick(out)?;
        }
    }

    /// The stream's last slate: the TV is let go first, then `says` stands for `last_word` and the stream ends as `end`.
    fn last(&mut self, says: Says, end: End, out: &mut dyn Write) -> io::Result<End> {
        self.tap = None;
        self.lease.over();
        log::info!("tuner: {} ends with {says:?}", self.entry.guide);
        if self.show.program.is_live() {
            let format = *self.show.slate.format();
            let slate = Slate::new(self.look.clone(), &self.entry.guide, &self.entry.name, format);
            let now = self.now();
            self.show.stand_in(slate, now);
        }
        self.show.slate.say(says);
        self.show.last_word(self.facade.limits.last_word, out)?;
        Ok(end)
    }

    /// What the stream found of the channel, once it was tuned.
    fn record(&self) {
        let (got, lost) = self.lost;
        let share = if got + lost == 0 { 0.0 } else { lost as f32 / (got + lost) as f32 };
        if self.weakest.is_none() && self.pictured.is_none() && got == 0 {
            return;
        }
        self.facade.tuning.record(
            &self.entry.guide,
            Health {
                at: SystemTime::now(),
                verdict: Verdict::of(self.pictured.is_some(), share, self.dropouts),
                signal: self.weakest,
                lost: share,
                picture: self.pictured,
                dropouts: self.dropouts,
                checked: false,
            },
        );
    }
}


#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::docs::Identity;
    use crate::slate::tests::look;
    use crate::splice::{AUDIO, PMT, VIDEO, head, write_stamp};
    use crate::stream::tests::{Multiplex, PLEX, Time, rf32, served};
    use crate::stream::{Admitted, Hardware, Limits};
    use parking_lot::Mutex;
    use serde_json::{Value, json};
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    /// A TV whose tap gives each chunk of its script once the clock has come to the chunk's time, and whose reads wait by moving the clock.
    /// Its tables come from `tables_from` on (before, a read waits and gives nothing: a weak signal), and its own Live TV takes the tuner
    /// at `taken_at`.
    pub(crate) struct Script {
        pub(crate) time: Time,
        mux: Multiplex,
        chunks: Mutex<VecDeque<(Duration, Vec<u8>)>>,
        tables_from: Duration,
        taken_at: Option<Duration>,
        pub(crate) released: AtomicU64,
        pub(crate) started: Mutex<Vec<Vec<u16>>>,
    }

    impl Script {
        pub(crate) fn new(time: &Time, chunks: Vec<(u64, Vec<u8>)>) -> Arc<Self> {
            Arc::new(Self {
                time: time.clone(),
                mux: rf32(&[]),
                chunks: Mutex::new(chunks.into_iter().map(|(ms, c)| (Duration::from_millis(ms), c)).collect()),
                tables_from: Duration::ZERO,
                taken_at: None,
                released: AtomicU64::new(0),
                started: Mutex::default(),
            })
        }

        fn now(&self) -> Duration {
            (self.time.clock())()
        }
    }

    pub(crate) struct Tv(pub(crate) Arc<Script>);
    struct ScriptTap(Arc<Script>);

    impl Hardware for Tv {
        fn channel_list(&self) -> Result<Value, Failure> {
            Ok(json!({"channelList": [
                {"channelId": "1_32_2_1_0_0_0", "channelName": "WSB-HD"},
                {"channelId": "1_32_2_4_0_0_0", "channelName": "WEST"},
            ]}))
        }

        fn programmes(&self, _: &str) -> Result<Value, Failure> {
            Ok(Value::Null)
        }

        fn in_use(&self) -> Option<String> {
            None
        }

        fn tune(&self, _: &str) -> Result<Box<dyn Tap>, Failure> {
            self.0.time.advance(Duration::from_millis(300));
            Ok(Box::new(ScriptTap(self.0.clone())))
        }

        fn signal(&self) -> Option<Signal> {
            Some(Signal { locked: true, snr: Some(18.0), errors: Some(3) })
        }
    }

    impl Tap for ScriptTap {
        fn section(&mut self, pid: u16, table_id: u8, number: u8, wait: Duration) -> Option<Vec<u8>> {
            let s = &self.0;
            if s.now() < s.tables_from {
                s.time.advance(wait);
                return None;
            }
            s.time.advance(Duration::from_millis(50));
            match (pid, table_id) {
                (psi::PID_PSIP, psi::TID_TVCT) => s.mux.tvct.get(usize::from(number)).cloned(),
                (psi::PID_PAT, psi::TID_PAT) => Some(s.mux.pat.clone()),
                (pid, psi::TID_PMT) => s.mux.pmts.get(&pid).cloned(),
                _ => None,
            }
        }

        fn start(&mut self, pids: &[u16]) -> Result<(), Failure> {
            self.0.started.lock().push(pids.to_vec());
            Ok(())
        }

        fn read(&mut self, out: &mut Vec<u8>, wait: Duration) -> Result<(), Failure> {
            let now = self.0.now();
            let mut chunks = self.0.chunks.lock();
            match chunks.front() {
                Some((due, _)) if *due <= now => out.extend(chunks.pop_front().map(|c| c.1).unwrap_or_default()),
                Some((due, _)) => self.0.time.advance(wait.min(*due - now).max(Duration::from_millis(1))),
                None => self.0.time.advance(wait),
            }
            Ok(())
        }

        fn taken(&mut self) -> Option<String> {
            self.0.taken_at.filter(|t| self.0.now() >= *t).map(|_| "Live TV is in front".to_owned())
        }
    }

    impl Drop for ScriptTap {
        fn drop(&mut self) {
            self.0.released.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The packets of one PES of `pid`: its head stamped `pts` (and `dts`), then `body`, the first packet starting it, the counter going on
    /// from `cc`.
    pub(crate) fn pes(pid: u16, cc: &mut u8, pts: u64, dts: Option<u64>, body: &[u8]) -> Vec<u8> {
        let mut data = vec![0, 0, 1, if pid == 0x61 { 0xE0 } else { 0xBD }, 0, 0, 0x80, 0x80, 5, 0x21, 0, 1, 0, 1];
        write_stamp(&mut data, 9, pts);
        if let Some(dts) = dts {
            data[7] = 0xC0;
            data[8] = 10;
            data[9] = 0x31;
            data.extend([0x11, 0, 1, 0, 1]);
            write_stamp(&mut data, 14, dts);
        }
        data.extend_from_slice(body);
        let mut out = Vec::new();
        for (i, chunk) in data.chunks(PACKET - 4).enumerate() {
            let mut p = vec![0x47, if i == 0 { 0x40 } else { 0 } | (pid >> 8) as u8, pid as u8, 0x10 | *cc];
            *cc = (*cc + 1) & 0x0F;
            p.extend_from_slice(chunk);
            p.resize(PACKET, 0xFF);
            out.extend(p);
        }
        out
    }

    /// 2.4's picture at decoding time `dts`, a frame each 3003: a whole one (a sequence header of 720x480, a closed GOP, an I picture)
    /// or a P picture, with `mark` in it to be found again.
    pub(crate) fn picture(cc: &mut u8, dts: u64, whole: bool, mark: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        if whole {
            body.extend([0, 0, 1, 0xB3, 0x2D, 0x01, 0xE0, 0x34, 0xFF, 0xFF, 0xE0, 0x18]);
            body.extend([0, 0, 1, 0xB8, 0, 8, 0, 0x40]);
        }
        body.extend([0, 0, 1, 0, 0, if whole { 1 << 3 } else { 2 << 3 }, 0xFF, 0xF8]);
        body.extend_from_slice(mark);
        body.resize(300, 0x5A);
        pes(0x61, cc, dts + 3003, Some(dts), &body)
    }

    fn sound(cc: &mut u8, pts: u64) -> Vec<u8> {
        pes(0x64, cc, pts, None, &[0x0b, 0x77, 0, 0])
    }

    fn facade(script: &Arc<Script>, limits: Limits) -> Facade {
        let f = Facade::new(
            Arc::new(Tv(script.clone())),
            Identity { name: "LumaTV".into(), device_id: "0A1B2C3D".into() },
            limits,
            script.time.clock(),
            served(),
            None,
        );
        f.tuning.options.lock().look = Some(Arc::new(|| Some(look())));
        f
    }

    /// Limits a test runs through in a few seconds of its clock.
    fn quick() -> Limits {
        Limits { give_up: Duration::from_secs(2), give_up_tuning: Duration::from_secs(3), weak_after: Duration::from_secs(1), last_word: Duration::from_millis(500), ..Limits::default() }
    }

    fn play(f: &Facade) -> (Vec<u8>, End) {
        let Ok(Admitted::Slated(s)) = f.admit("2.4", PLEX) else {
            panic!("not slated");
        };
        let mut out = Vec::new();
        let end = f.play(s, &mut out, &AtomicBool::new(false)).unwrap();
        (out, end)
    }

    /// Each picture written on the program's picture PID: whether it is the slate's (an MPEG-2 picture made here, its first frame with a
    /// sequence header of the slate's own) or the station's (found by its mark), and its decoding time.
    fn pictures(out: &[u8]) -> Vec<(bool, u64, Vec<u8>)> {
        let mut got: Vec<(bool, u64, Vec<u8>)> = Vec::new();
        for p in out.chunks_exact(PACKET) {
            if psi::pid_of_packet(p) != VIDEO {
                continue;
            }
            if let Some(h) = head(p) {
                let dts = h.dts.or(h.pts).map(|x| x.1).unwrap();
                let body = p[h.payload..].to_vec();
                let station = body.windows(4).any(|w| w == b"MARK");
                got.push((station, dts, body));
            }
        }
        got
    }

    fn mark(body: &[u8]) -> String {
        let at = body.windows(4).position(|w| w == b"MARK").unwrap();
        String::from_utf8_lossy(&body[at..at + 6]).into_owned()
    }

    #[test]
    fn a_stream_with_slates_tunes_under_its_slate_then_plays_from_the_first_whole_picture_and_the_slate_stands_in_when_it_stops() {
        let time = Time::default();
        let (mut v, mut a) = (0u8, 0u8);
        let mut chunks = vec![(600, [picture(&mut v, 900_000, false, b"MARK00"), sound(&mut a, 899_000)].concat())];
        // the first whole picture at 0.9 s, then a picture each 33 ms to 2 s, then nothing
        let mut dts = 1_000_000;
        chunks.push((900, [picture(&mut v, dts, true, b"MARK01"), sound(&mut a, dts + 3003)].concat()));
        for n in 2..36u64 {
            dts += 3003;
            chunks.push((900 + 33 * (n - 1), picture(&mut v, dts, false, format!("MARK{n:02}").as_bytes())));
        }
        let script = Script::new(&time, chunks);
        let f = facade(&script, quick());
        let (out, end) = play(&f);
        assert_eq!(end, End::Failed(Failure::WeakSignal("the picture stopped (2.4)".to_owned())));
        let first: Vec<u16> = out.chunks_exact(PACKET).take(2).map(psi::pid_of_packet).collect();
        assert_eq!(first, [0, PMT], "the tables lead");
        let pmt = out.chunks_exact(PACKET).nth(1).and_then(|p| psi::parse_pmt(&p[5..])).unwrap();
        assert_eq!(pmt.streams.iter().map(|s| (s.stream_type, s.pid)).collect::<Vec<_>>(), [(0x02, VIDEO), (0x81, AUDIO)]);
        let pics = pictures(&out);
        let station_at = pics.iter().position(|p| p.0).unwrap();
        // the slate went on while the channel was tuned: 0.9 s of frames at 29.97 a second
        assert!((20..=30).contains(&station_at), "{station_at} slate frames before the programme");
        assert!(Format::of_sequence(&pics[0].2).is_some_and(|f| f == Format::FALLBACK), "the slate is the shape of a channel not seen yet");
        assert_eq!(mark(&pics[station_at].2), "MARK01", "the programme starts at its first whole picture, not before");
        assert_eq!(pics[station_at].1, pics[station_at - 1].1 + 3003, "and is decoded a frame after the slate's last");
        let back = station_at + pics[station_at..].iter().position(|p| !p.0).unwrap();
        assert_eq!(mark(&pics[back - 1].2), "MARK34", "every picture of the programme went out but the one it broke off in");
        assert!(pics[back].1 > pics[back - 1].1 + 3003, "the slate after it starts after the last picture shown");
        assert!(Format::of_sequence(&pics[back].2).is_some_and(|f| (f.width, f.height) == (720, 480)), "now the channel's own shape");
        let health = f.tuning.known()["2.4"].health.clone().unwrap();
        assert_eq!((health.verdict, health.dropouts), (Verdict::Weak, 1));
        assert!(health.picture.is_some_and(|d| d >= Duration::from_millis(900)));
        assert_eq!(script.released.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_channel_whose_picture_never_comes_says_weak_signal_then_no_signal() {
        let time = Time::default();
        let script = Script::new(&time, vec![]);
        let f = facade(&script, quick());
        let (out, end) = play(&f);
        assert_eq!(end, End::Failed(Failure::WeakSignal("no packets (2.4, 3 packet errors)".to_owned())));
        let pics = pictures(&out);
        assert!(pics.iter().all(|p| !p.0));
        // three seconds of waiting and half a second of last word, at 29.97 a second
        assert!((95..=115).contains(&pics.len()), "{}", pics.len());
        assert_eq!(f.tuning.known()["2.4"].health.as_ref().map(|h| h.verdict), Some(Verdict::NoSignal));
    }

    #[test]
    fn the_tv_taking_the_tuner_lets_it_go_first_then_says_so() {
        let time = Time::default();
        let (mut v, mut a) = (0u8, 0u8);
        let chunks = vec![(500, [picture(&mut v, 1_000_000, true, b"MARK01"), sound(&mut a, 1_003_003), picture(&mut v, 1_003_003, false, b"MARK02")].concat())];
        let mut script = Script::new(&time, chunks);
        Arc::get_mut(&mut script).unwrap().taken_at = Some(Duration::from_millis(1_200));
        let f = facade(&script, Limits { owner_every: Duration::from_secs(1), ..quick() });
        let (_, end) = play(&f);
        assert_eq!(end, End::Preempted("Live TV is in front".to_owned()));
        assert_eq!(script.released.load(Ordering::Relaxed), 1);
        assert_eq!(f.tuning.status(), Status::Busy("Live TV is in front".to_owned()));
    }

    #[test]
    fn tables_that_come_late_are_waited_for_under_the_slate() {
        let time = Time::default();
        let (mut v, mut a) = (0u8, 0u8);
        let chunks = vec![(2_600, [picture(&mut v, 1_000_000, true, b"MARK01"), sound(&mut a, 1_003_003), picture(&mut v, 1_003_003, false, b"MARK02")].concat())];
        let mut script = Script::new(&time, chunks);
        Arc::get_mut(&mut script).unwrap().tables_from = Duration::from_millis(2_000);
        let f = facade(&script, quick());
        let (out, _) = play(&f);
        assert!(pictures(&out).iter().any(|p| p.0), "the programme came after its tables did");
        assert_eq!(*script.started.lock(), [vec![0x61, 0x64]]);
    }

    #[test]
    fn a_picture_that_breaks_up_brings_the_slate_and_a_clean_second_brings_it_back() {
        let time = Time::default();
        let (mut v, mut a) = (0u8, 0u8);
        let mut chunks = vec![];
        let mut dts = 1_000_000;
        let mut at = 500;
        for n in 0..180u64 {
            let mut p = picture(&mut v, dts, n % 15 == 0, format!("MARK{n:02}").as_bytes());
            // from 1.5 s to 2.5 s every picture loses its second packet: half the picture's packets
            if (30..60).contains(&n) {
                p.drain(PACKET..2 * PACKET);
            }
            chunks.push((at, [p, sound(&mut a, dts)].concat()));
            dts += 3003;
            at += 33;
        }
        let script = Script::new(&time, chunks);
        let f = facade(&script, quick());
        let (out, _) = play(&f);
        let pics = pictures(&out);
        let runs: Vec<bool> = pics.iter().map(|p| p.0).collect::<Vec<_>>().dedup_by_key_into();
        assert_eq!(runs, [false, true, false, true, false], "slate, programme, slate, programme, the last slate");
        let back = pics.iter().enumerate().filter(|(i, p)| p.0 && *i > 0 && !pics[i - 1].0).nth(1).map(|(i, _)| i).unwrap();
        let n: u64 = mark(&pics[back].2)[4..].parse().unwrap();
        assert!(n >= 60 && n % 15 == 0, "back at a whole picture after the clean second: {n}");
    }

    trait Runs {
        fn dedup_by_key_into(self) -> Vec<bool>;
    }

    impl Runs for Vec<bool> {
        fn dedup_by_key_into(mut self) -> Vec<bool> {
            self.dedup();
            self
        }
    }

    #[test]
    fn hidden_channels_leave_the_lineup_and_without_a_look_there_are_no_slates() {
        let time = Time::default();
        let script = Script::new(&time, vec![]);
        let f = facade(&script, quick());
        f.tuning.options.lock().hidden = ["2.1".to_owned()].into();
        assert_eq!(f.lineup().unwrap().iter().map(|e| e.guide.as_str()).collect::<Vec<_>>(), ["2.4"]);
        f.tuning.options.lock().look = Some(Arc::new(|| None));
        assert!(!matches!(f.admit("2.4", PLEX), Ok(Admitted::Slated(_))));
    }
}
