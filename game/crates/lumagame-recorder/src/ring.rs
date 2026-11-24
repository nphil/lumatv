//! The RAM ring: the stream's coded units from the last keyframes, in whole GOPs, under a byte cap. The oldest GOP goes first, so what is
//! left starts at a keyframe and a clip can begin on any GOP boundary, unless the one open GOP has itself outgrown the cap (see
//! [`Ring::push`]). Units are shared (`Arc`), so a reader holds no lock while it writes, and a unit it holds stays valid after the
//! ring drops it.
use std::collections::VecDeque;
use std::sync::Arc;

use crate::{Config, Track};

#[derive(Clone, Debug)]
pub struct Unit {
    /// Position in the stream as the ring accepted it; consecutive.
    pub seq: u64,
    pub track: Track,
    pub pts_ns: i64,
    pub key: bool,
    pub data: Arc<[u8]>,
}

#[derive(Debug)]
struct Gop {
    first_seq: u64,
    start_pts_ns: i64,
    units: VecDeque<Unit>,
    bytes: usize,
    config: Option<Arc<Config>>,
    /// Units were lost inside it, so it shows damage until its next keyframe; a clip prefers to start elsewhere.
    damaged: bool,
}

/// What reading position `seq` finds.
#[derive(Debug)]
pub enum Read {
    /// The unit, and the config of the GOP it belongs to.
    Unit(Unit, Option<Arc<Config>>),
    /// Not arrived yet.
    Wait,
    /// Dropped to make room before the reader got to it.
    Evicted,
}

#[derive(Debug, Default)]
pub struct Ring {
    cap: usize,
    bytes: usize,
    gops: VecDeque<Gop>,
    next_seq: u64,
    newest_video_pts: i64,
    /// Whole GOPs dropped for room.
    pub evicted_gops: u64,
    /// Units refused because no keyframe had arrived to start from.
    pub before_keyframe: u64,
    /// Times a single GOP outgrew the cap and lost its keyframe.
    pub oversize: u64,
}

impl Ring {
    pub fn new(cap: usize) -> Self {
        Self { cap, ..Self::default() }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn gops(&self) -> usize {
        self.gops.len()
    }

    /// The sequence number the next accepted unit gets; a clip that ends "now" ends here.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Adds a unit. A video keyframe opens a GOP and carries `config` (or inherits the previous GOP's when it has none); anything else
    /// joins the open GOP, and is refused while there is none. Returns whether the unit was kept.
    ///
    /// When the open GOP alone outgrows the cap (a stream with one keyframe, or far apart), its oldest units go one at a time: readers
    /// that keep up still find the stream, but the GOP has lost its keyframe and cannot start a clip, so the ring is not ready until the
    /// next keyframe.
    pub fn push(&mut self, track: Track, pts_ns: i64, key: bool, data: Arc<[u8]>, config: Option<Arc<Config>>) -> bool {
        let opens = track == Track::Video && key;
        if opens {
            let config = config.or_else(|| self.gops.back().and_then(|g| g.config.clone()));
            self.gops.push_back(Gop { first_seq: self.next_seq, start_pts_ns: pts_ns, units: VecDeque::new(), bytes: 0, config, damaged: false });
        } else if self.gops.is_empty() {
            self.before_keyframe += 1;
            return false;
        }
        if track == Track::Video {
            self.newest_video_pts = pts_ns;
        }
        let gop = self.gops.back_mut().expect("a GOP is open");
        gop.bytes += data.len();
        self.bytes += data.len();
        gop.units.push_back(Unit { seq: self.next_seq, track, pts_ns, key, data });
        self.next_seq += 1;
        while self.bytes > self.cap && self.gops.len() > 1 {
            let old = self.gops.pop_front().expect("more than one GOP");
            self.bytes -= old.bytes;
            self.evicted_gops += 1;
        }
        if self.bytes > self.cap {
            let gop = self.gops.back_mut().expect("a GOP is open");
            if gop.config.take().is_some() {
                self.oversize += 1;
            }
            while self.bytes > self.cap && gop.units.len() > 1 {
                let unit = gop.units.pop_front().expect("more than one unit");
                gop.bytes -= unit.data.len();
                gop.first_seq += 1;
                self.bytes -= unit.data.len();
            }
        }
        true
    }

    fn gop_of(&self, seq: u64) -> Option<&Gop> {
        let i = self.gops.partition_point(|g| g.first_seq <= seq).checked_sub(1)?;
        self.gops.get(i).filter(|g| seq < g.first_seq + g.units.len() as u64)
    }

    pub fn read(&self, seq: u64) -> Read {
        if seq >= self.next_seq {
            return Read::Wait;
        }
        match self.gop_of(seq) {
            Some(g) => Read::Unit(g.units[(seq - g.first_seq) as usize].clone(), g.config.clone()),
            None => Read::Evicted,
        }
    }

    /// Where a clip of "the last `seconds`" begins: the newest keyframe at or before that far back from the newest picture, so the clip is
    /// at least that long, or the oldest GOP when the ring holds less. Only GOPs that know their decoder config count.
    pub fn start_for_last(&self, seconds: u32) -> Option<u64> {
        let from = self.newest_video_pts - i64::from(seconds) * 1_000_000_000;
        let ready = self.starts();
        ready.iter().rev().find(|g| g.start_pts_ns <= from).or_else(|| ready.first()).map(|g| g.first_seq)
    }

    /// The GOPs a clip may start on: those that know their decoder config, undamaged ones if there are any.
    fn starts(&self) -> Vec<&Gop> {
        let known: Vec<&Gop> = self.gops.iter().filter(|g| g.config.is_some()).collect();
        let whole: Vec<&Gop> = known.iter().copied().filter(|g| !g.damaged).collect();
        if whole.is_empty() { known } else { whole }
    }

    /// Units were lost before the next one: the open GOP is damaged from the loss to its next keyframe.
    pub fn mark_damaged(&mut self) {
        if let Some(g) = self.gops.back_mut() {
            g.damaged = true;
        }
    }

    /// Where a recording that starts now begins: the open GOP's keyframe, so it has at most one GOP of lead-in.
    pub fn start_of_open_gop(&self) -> Option<u64> {
        self.starts().last().map(|g| g.first_seq)
    }

    /// Whether a clip can start now: some GOP opened with a keyframe whose decoder config is known.
    pub fn ready(&self) -> bool {
        self.gops.iter().any(|g| g.config.is_some())
    }

    /// The first keyframe that arrived at or after position `seq`: where a clip or recording that began then starts.
    pub fn first_key_from(&self, seq: u64) -> Option<u64> {
        self.gops.iter().find(|g| g.first_seq >= seq && g.config.is_some()).map(|g| g.first_seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::test_config;

    fn data(n: usize) -> Arc<[u8]> {
        vec![0u8; n].into()
    }
    fn gop(r: &mut Ring, key_pts: i64, frames: usize, size: usize) {
        r.push(Track::Video, key_pts, true, data(size), Some(test_config()));
        for i in 1..frames {
            r.push(Track::Video, key_pts + i as i64 * 16_000_000, false, data(size), None);
        }
    }

    #[test]
    fn the_cap_drops_whole_gops_oldest_first_and_the_ring_still_starts_on_a_keyframe() {
        let mut r = Ring::new(1000);
        for g in 0..6 {
            gop(&mut r, g * 1_000_000_000, 4, 100);
            assert!(r.bytes() <= 1000);
        }
        // 400 bytes a GOP, cap 1000: two whole GOPs fit.
        assert_eq!((r.gops(), r.bytes(), r.evicted_gops), (2, 800, 4));
        let Read::Unit(first, _) = r.read(r.start_of_open_gop().unwrap() - 4) else { panic!("the older GOP is kept") };
        assert!(first.key && first.track == Track::Video, "the oldest unit is a keyframe");
        assert!(matches!(r.read(0), Read::Evicted));
    }

    #[test]
    fn units_before_the_first_keyframe_are_refused_and_audio_joins_the_open_gop() {
        let mut r = Ring::new(1 << 20);
        assert!(!r.push(Track::Audio, 0, false, data(10), None));
        assert!(!r.push(Track::Video, 0, false, data(10), None));
        assert_eq!(r.before_keyframe, 2);
        gop(&mut r, 0, 2, 10);
        assert!(r.push(Track::Audio, 5, true, data(10), None), "an audio unit's key flag does not open a GOP");
        assert_eq!(r.gops(), 1);
    }

    #[test]
    fn a_gop_larger_than_the_cap_loses_its_oldest_units_and_its_keyframe_but_a_reader_that_keeps_up_still_reads_on() {
        let mut r = Ring::new(250);
        gop(&mut r, 0, 2, 100);
        gop(&mut r, 1_000_000_000, 4, 100);
        assert_eq!((r.gops(), r.bytes(), r.oversize, r.ready()), (1, 200, 1, false));
        assert!(r.push(Track::Video, 2_000_000_000, false, data(10), None), "the stream goes on");
        assert!(matches!(r.read(r.next_seq() - 1), Read::Unit(..)) && matches!(r.read(2), Read::Evicted));
        gop(&mut r, 3_000_000_000, 2, 100);
        assert_eq!((r.gops(), r.ready()), (1, true), "the next keyframe is a start again, and the headless GOP is dropped for room");
    }

    #[test]
    fn a_clip_of_the_last_seconds_starts_at_the_keyframe_at_or_before_that_far_back() {
        let mut r = Ring::new(1 << 20);
        for g in 0..10 {
            gop(&mut r, g * 1_000_000_000, 3, 10); // a keyframe a second, the newest picture at 9.032 s
        }
        let first = |seconds| match r.read(r.start_for_last(seconds).unwrap()) {
            Read::Unit(u, _) => u.pts_ns / 1_000_000_000,
            _ => panic!(),
        };
        assert_eq!(first(3), 6, "6 s is at or before 9.032 - 3");
        assert_eq!(first(0), 9, "no look-back: the open GOP");
        assert_eq!(first(100), 0, "the ring holds less: all of it");
        assert_eq!(r.start_of_open_gop(), r.start_for_last(0));
    }

    #[test]
    fn a_gop_without_a_decoder_config_never_starts_a_clip_but_inherits_the_last_one() {
        let mut r = Ring::new(1 << 20);
        r.push(Track::Video, 0, true, data(10), None);
        assert_eq!(r.start_of_open_gop(), None);
        r.push(Track::Video, 1_000_000_000, true, data(10), Some(test_config()));
        r.push(Track::Video, 2_000_000_000, true, data(10), None);
        let Read::Unit(_, config) = r.read(2) else { panic!() };
        assert!(config.is_some());
        assert_eq!(r.start_of_open_gop(), Some(2));
    }

    #[test]
    fn eviction_never_takes_the_newest_gop_and_a_look_back_past_what_is_held_starts_at_the_oldest_one() {
        let mut r = Ring::new(1000);
        for g in 0..40 {
            gop(&mut r, g * 5_000_000_000, 4, 100); // a keyframe every 5 s; 400 bytes a GOP, so two whole GOPs fit
            assert!(r.ready());
            assert!(matches!(r.read(r.next_seq() - 1), Read::Unit(..)), "the newest GOP is kept");
        }
        let oldest = r.gops.front().map(|g| g.first_seq);
        assert_eq!(r.start_for_last(15), oldest, "15 s back is past what is held");
        let Read::Unit(first, _) = r.read(oldest.unwrap()) else { panic!() };
        assert!(first.key, "a clip starts on a keyframe");
    }

    #[test]
    fn a_single_keyframe_stream_is_not_ready_once_it_outgrows_the_ring_until_the_next_cut_point() {
        let mut r = Ring::new(1000);
        r.push(Track::Video, 0, true, data(100), Some(test_config()));
        assert!(r.ready());
        for i in 1..12 {
            r.push(Track::Video, i * 16_000_000, false, data(100), None);
        }
        assert_eq!((r.ready(), r.oversize), (false, 1), "the only keyframe went with its oversized GOP");
        let from = r.next_seq();
        assert_eq!(r.first_key_from(from), None);
        r.push(Track::Video, 1_000_000_000, true, data(100), Some(test_config()));
        assert!(r.ready());
        assert_eq!((r.first_key_from(from), r.first_key_from(from + 1)), (Some(from), None));
    }

    #[test]
    fn a_clip_avoids_starting_on_a_gop_that_lost_units() {
        let mut r = Ring::new(1 << 20);
        gop(&mut r, 0, 3, 10);
        gop(&mut r, 1_000_000_000, 3, 10);
        r.mark_damaged();
        assert_eq!((r.start_of_open_gop(), r.start_for_last(0)), (Some(0), Some(0)), "the newer GOP lost units");
        let mut all = Ring::new(1 << 20);
        gop(&mut all, 0, 1, 10);
        all.mark_damaged();
        assert_eq!(all.start_of_open_gop(), Some(0), "a damaged GOP is still better than none");
    }

    #[test]
    fn reads_report_waiting_and_evicted_positions() {
        let mut r = Ring::new(1 << 20);
        gop(&mut r, 0, 2, 10);
        assert!(matches!(r.read(1), Read::Unit(..)));
        assert!(matches!(r.read(2), Read::Wait));
    }
}
