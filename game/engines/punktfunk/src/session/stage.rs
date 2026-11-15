// Adapted from client-webos session/timeline.rs and session/stage (MIT OR Apache-2.0).
use punktfunk_core::phase::{CadenceClock, CadenceTuning, pace_slot};
mod poc_bridge;
use poc_bridge::PocBridge;

pub struct Pacing {
    clock: CadenceClock,
    interval: i64,
    smooth: bool,
    arrival: bool,
    last: u64,
    source: Option<u64>,
    anchor: Option<i64>,
    slot: i64,
    reanchors: u64,
}
impl Pacing {
    pub fn new(hz: u32, smooth: bool, av1: bool) -> Self {
        Self { clock: CadenceClock::new(CadenceTuning::snapping()), interval: 1_000_000_000 / i64::from(hz.max(1)), smooth,
            arrival: av1 && !smooth, last:0, source:None, anchor:None, slot:0, reanchors:0 }
    }
    pub fn reset(&mut self) { self.clock.reset(); self.source = None; self.anchor = None; }
    pub fn rate(&mut self, hz: u32) {
        let interval = 1_000_000_000 / i64::from(hz.max(1));
        if self.interval != interval { self.interval = interval; self.reset(); }
    }
    pub fn map(&mut self, host: u64, now: u64) -> u64 {
        let mut due = if self.source == Some(host) { self.clock.note_off_cadence(now as i64, self.interval) }
            else { self.clock.due_ns(host, now as i64, self.interval) };
        if self.smooth {
            let h = host.min(i64::MAX as u64) as i64;
            let reanchors = self.clock.health().reanchors;
            match self.anchor {
                Some(anchor) if h >= anchor && self.reanchors == reanchors => {
                    let nominal = (h - anchor + self.interval / 2) / self.interval;
                    let slot = pace_slot(nominal, Some(self.slot), 2);
                    self.slot = self.slot.max(slot);
                    due = due.saturating_add(anchor + slot * self.interval - h);
                }
                _ => { self.anchor = Some(h); self.slot = 0; self.reanchors = reanchors; }
            }
            due = due.saturating_add(2 * self.interval);
        }
        self.source = Some(host);
        let stamp = if self.arrival { (now / 1_000_000 * 1_000_000).max(self.last.saturating_add(1_000_000)) }
            else { (due.max(0) as u64).max(self.last).div_ceil(1_000_000) * 1_000_000 };
        self.last = stamp;
        stamp
    }
    pub fn jitter_ms(&self) -> f32 { self.clock.health().jitter_ns as f32 / 1_000_000.0 }
}

pub struct Stage {
    pub pacing: Pacing,
    holding: bool,
    idr_only: bool,
    last_request: Option<u64>,
    bridge: Option<PocBridge>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Gate { Feed, Resume, Hold, Keyframe }
impl Stage {
    pub fn new(hz: u32, smooth: bool, codec: u8) -> Self {
        Self { pacing: Pacing::new(hz,smooth,codec == punktfunk_core::quic::CODEC_AV1), holding:false,idr_only:false,last_request:None,
            bridge:(codec == punktfunk_core::quic::CODEC_HEVC).then(PocBridge::default) }
    }
    pub fn hold(&mut self, now: u64, backstop: bool) {
        if !self.holding && backstop { self.last_request = Some(now); }
        self.holding = true;
    }
    pub fn decoder_loss(&mut self, now:u64) {
        // A socket write succeeded, but NDL refused the AU. Its reference state no longer
        // matches the bridge's mirror: a fresh IDR is the only safe restart.
        self.hold(now,false);
        self.idr_only=true;
        if self.bridge.is_some() {self.bridge=Some(PocBridge::default());}
    }
    pub fn anchor(&self, idr:bool, recovery:bool)->bool {idr || (recovery && !self.idr_only)}
    pub fn keyframe_due(&mut self, now: u64) -> bool {
        if self.last_request.is_none_or(|t| now.saturating_sub(t) >= 100_000_000) { self.last_request = Some(now); true } else { false }
    }
    pub fn gate(&mut self, anchor: bool, loss: bool, now: u64) -> Gate {
        if loss { self.hold(now,true); }
        if !self.holding { return Gate::Feed; }
        if anchor { self.holding = false; self.idr_only=false; self.pacing.reset(); return Gate::Resume; }
        if self.keyframe_due(now) { Gate::Keyframe } else { Gate::Hold }
    }
    pub fn bridges(&mut self, data: &[u8], index: u64) -> Vec<Vec<u8>> {
        self.bridge.as_mut().map_or_else(Vec::new, |b| b.bridges(data,index))
    }
    pub fn fed(&mut self, data: &[u8], index: u64) { if let Some(b) = &mut self.bridge { b.note_fed(data,index); } }
}

#[derive(Default)]
pub struct Loss {
    newest: Option<u32>,
    extended: u64,
    dropped: u64,
    credit: u64,
    expiry: u64,
}
impl Loss {
    pub fn new(dropped: u64) -> Self { Self { dropped, ..Default::default() } }
    pub fn late(&self, index: u32) -> bool { self.newest.is_some_and(|n| index != n && (index.wrapping_sub(n) as i32) < 0) }
    pub fn observe(&mut self, index: u32, gap: u32, dropped: u64, now: u64) -> (bool,u64) {
        self.extended += self.newest.map_or(u64::from(index), |n| u64::from(index.wrapping_sub(n)));
        self.newest = Some(index);
        if now >= self.expiry { self.credit = 0; }
        if gap > 0 {
            self.credit = self.credit.saturating_add(u64::from(gap));
            self.expiry = now.saturating_add(punktfunk_core::reanchor::DROP_CREDIT_WINDOW.as_nanos() as u64);
        }
        let delta = dropped.saturating_sub(self.dropped);
        self.dropped = dropped;
        let covered = delta.min(self.credit);
        self.credit -= covered;
        (gap > 0 || delta > covered,self.extended)
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn loss_freezes_until_anchor_and_throttles_only_the_request() {
        let mut s = Stage::new(120,false,2);
        assert_eq!(s.gate(false,true,0),Gate::Hold);
        assert_eq!(s.gate(false,false,99_000_000),Gate::Hold);
        assert_eq!(s.gate(false,false,100_000_000),Gate::Keyframe);
        assert_eq!(s.gate(true,false,101_000_000),Gate::Resume);
        s.hold(102_000_000,false);
        assert_eq!(s.gate(false,false,103_000_000),Gate::Hold);
        assert_eq!(s.gate(true,false,104_000_000),Gate::Resume);
    }
    #[test] fn recovery_keeps_timestamp_floor_and_millisecond_ceiling() {
        let mut p = Pacing::new(120,true,false);
        let before = p.map(1_000_000_000,4_000_000_123);
        p.reset();
        let after = p.map(2_000_000_000,3_000_000_123);
        assert!(after >= before);
        assert_eq!(after % 1_000_000,0);
        let mut av1 = Pacing::new(60,false,true);
        assert_eq!(av1.map(0,50_123_456),50_000_000);
    }
    #[test] fn delayed_loss_accounting_does_not_freeze_a_recovered_picture_twice() {
        let mut l = Loss::new(0);
        assert_eq!(l.observe(u32::MAX-1,0,0,0),(false,u64::from(u32::MAX-1)));
        assert!(l.observe(1,2,0,10).0);
        assert!(!l.observe(2,0,2,20).0);
        assert!(l.late(1));
        assert!(!l.late(3));
    }
    #[test] fn decoder_rejection_requires_idr_without_moving_the_clock_backwards() {
        let mut s=Stage::new(120,false,2);
        let stamp=s.pacing.map(100,1_000_000_000);
        s.decoder_loss(1_000_000_001);
        assert!(!s.anchor(false,true));
        assert!(s.anchor(true,false));
        assert_eq!(s.gate(s.anchor(true,false),false,1_001_000_000),Gate::Resume);
        assert!(s.pacing.map(200,999_999_999)>=stamp);
        assert!(s.anchor(false,true));
    }
}
