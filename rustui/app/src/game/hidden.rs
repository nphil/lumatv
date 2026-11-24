//! What LumaGame does while another app is in front: let go of what only the screen needs at once, and leave after a long dwell.
//!
//! A hidden app held about 208 MB of resident memory and 74 MB on the GPU, which the TV's contiguous memory pool pays for. On hide
//! with no stream the page, its art and the renderer's caches are released (the task manager's card is kept as a small picture); after
//! [`EXIT_AFTER_MS`] with nothing going on the app leaves by its normal close, and the next open is a cold start.
//! This holds no clock: [`Hidden::step`] is given the time.

/// How long the app stays hidden, with nothing going on, before it closes. Long enough that going to LG Home and back for a
/// minute or two never costs a cold start (about 1-2 s), short enough that an app forgotten behind others gives its 280 MB back.
pub const EXIT_AFTER_MS: u64 = 10 * 60 * 1000;

/// What is going on that keeps the app from closing. Every field holds the exit and restarts the dwell; only `streaming` keeps the
/// page from being released.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Busy {
    /// A stream is up or coming up.
    pub streaming: bool,
    /// A session of any kind has not ended, a TV game's launch among them.
    pub session: bool,
    /// An engine process still runs: a recording or a replay feed, or one that was told to stop and has not.
    pub engine: bool,
    /// A launch or a message for the TV's application service is still on its way out.
    pub launching: bool,
    /// The pads are leased or read by the controller lab.
    pub pads: bool,
    /// A sign-in or pairing flow is open.
    pub sign_in: bool,
    /// The frame app's player is up over the app.
    pub player: bool,
}

impl Busy {
    fn any(self) -> bool {
        let Busy { streaming, session, engine, launching, pads, sign_in, player } = self;
        streaming || session || engine || launching || pads || sign_in || player
    }
}

/// What the app is to do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Nothing,
    /// Let go of the page, the art and the renderer's caches; keep the task manager's card.
    Release,
    /// Back in front after a release: the page is drawn again from the model.
    Restore,
    /// Close, as the task manager's quit does.
    Exit,
}

#[derive(Debug)]
pub struct Hidden {
    dwell_ms: u64,
    /// Since when the app has been hidden with nothing going on; none while it is in front.
    quiet_since: Option<u64>,
    released: bool,
    exiting: bool,
}

impl Hidden {
    pub fn new(dwell_ms: u64) -> Self {
        Self { dwell_ms, quiet_since: None, released: false, exiting: false }
    }

    /// One pass of the loop. `foreground`: the app is in front.
    pub fn step(&mut self, now_ms: u64, foreground: bool, busy: Busy) -> Act {
        if foreground {
            self.quiet_since = None;
            self.exiting = false;
            return if std::mem::take(&mut self.released) { Act::Restore } else { Act::Nothing };
        }
        if !self.released && !busy.streaming {
            self.released = true;
            self.quiet_since = Some(now_ms);
            return Act::Release;
        }
        if busy.any() {
            self.quiet_since = Some(now_ms);
            return Act::Nothing;
        }
        let since = *self.quiet_since.get_or_insert(now_ms);
        if now_ms.saturating_sub(since) >= self.dwell_ms && !std::mem::replace(&mut self.exiting, true) {
            return Act::Exit;
        }
        Act::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    const NONE: Busy = Busy { streaming: false, session: false, engine: false, launching: false, pads: false, sign_in: false, player: false };

    fn hidden() -> Hidden {
        Hidden::new(EXIT_AFTER_MS)
    }

    #[test]
    fn hiding_releases_once_and_showing_restores_once() {
        let mut h = hidden();
        assert_eq!(h.step(0, true, NONE), Act::Nothing);
        assert_eq!(h.step(1_000, false, NONE), Act::Release);
        assert_eq!(h.step(2_000, false, NONE), Act::Nothing);
        assert_eq!(h.step(3_000, true, NONE), Act::Restore);
        assert_eq!(h.step(4_000, true, NONE), Act::Nothing);
        assert_eq!(h.step(5_000, false, NONE), Act::Release, "each hide releases again");
    }

    #[test]
    fn a_stream_keeps_the_page_until_it_ends() {
        let mut h = hidden();
        let stream = Busy { streaming: true, session: true, ..NONE };
        assert_eq!(h.step(0, false, stream), Act::Nothing);
        assert_eq!(h.step(MIN, false, NONE), Act::Release);
    }

    #[test]
    fn ten_minutes_hidden_and_quiet_exits_once() {
        let mut h = hidden();
        assert_eq!(h.step(0, false, NONE), Act::Release);
        assert_eq!(h.step(EXIT_AFTER_MS - 1, false, NONE), Act::Nothing);
        assert_eq!(h.step(EXIT_AFTER_MS, false, NONE), Act::Exit);
        assert_eq!(h.step(EXIT_AFTER_MS + 1_000, false, NONE), Act::Nothing, "the close is asked for once");
    }

    #[test]
    fn each_busy_condition_holds_the_exit_and_the_dwell_starts_again_when_it_clears() {
        let conditions = [
            Busy { session: true, ..NONE },
            Busy { engine: true, ..NONE },
            Busy { launching: true, ..NONE },
            Busy { pads: true, ..NONE },
            Busy { sign_in: true, ..NONE },
            Busy { player: true, ..NONE },
        ];
        for busy in conditions {
            let mut h = hidden();
            assert_eq!(h.step(0, false, NONE), Act::Release);
            assert_eq!(h.step(EXIT_AFTER_MS + MIN, false, busy), Act::Nothing, "{busy:?}");
            let cleared = EXIT_AFTER_MS + 2 * MIN;
            assert_eq!(h.step(cleared, false, NONE), Act::Nothing, "{busy:?} dwell restarts");
            assert_eq!(h.step(cleared + EXIT_AFTER_MS, false, NONE), Act::Exit, "{busy:?}");
        }
    }

    #[test]
    fn returning_before_the_dwell_ends_cancels_the_exit_and_the_next_hide_counts_afresh() {
        let mut h = hidden();
        assert_eq!(h.step(0, false, NONE), Act::Release);
        assert_eq!(h.step(EXIT_AFTER_MS - MIN, true, NONE), Act::Restore);
        assert_eq!(h.step(EXIT_AFTER_MS, true, NONE), Act::Nothing);
        assert_eq!(h.step(EXIT_AFTER_MS + MIN, false, NONE), Act::Release);
        assert_eq!(h.step(EXIT_AFTER_MS + MIN + EXIT_AFTER_MS - 1, false, NONE), Act::Nothing);
        assert_eq!(h.step(EXIT_AFTER_MS + MIN + EXIT_AFTER_MS, false, NONE), Act::Exit);
    }
}
