//! A held OK (`motion.holdMs`, 700 ms): on a surface where holding means something (the quick panel's tile is picked up for editing)
//! OK is decided on its release so that a hold can be told from a tap. The press starts the timer; a release before it is up is an
//! ordinary OK; the timer is the hold, and the release that follows it is nothing.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Released {
    /// OK was not down here.
    #[default]
    Nothing,
    /// Let go before the timer: an ordinary OK.
    Tap,
    /// Let go after the hold fired: nothing more.
    Held,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Hold {
    down: bool,
    fired: bool,
    due: Option<u64>,
}

impl Hold {
    pub fn is_down(&self) -> bool {
        self.down
    }

    /// OK went down (a repeat is not a new press).
    pub fn press(&mut self, now: u64, hold_ms: u64) {
        if self.down {
            return;
        }
        self.down = true;
        self.fired = false;
        self.due = Some(now + hold_ms);
    }

    pub fn release(&mut self) -> Released {
        if !self.down {
            return Released::Nothing;
        }
        let was = if self.fired { Released::Held } else { Released::Tap };
        *self = Self::default();
        was
    }

    /// The timer: true once, when the hold fires.
    pub fn tick(&mut self, now: u64) -> bool {
        match self.due {
            Some(d) if now >= d && self.down && !self.fired => {
                self.fired = true;
                self.due = None;
                true
            }
            _ => false,
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        self.due
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_before_the_timer_is_a_tap_and_the_timer_is_the_hold() {
        let mut h = Hold::default();
        h.press(1000, 700);
        assert_eq!(h.due_at(), Some(1700));
        assert!(!h.tick(1699));
        assert_eq!(h.release(), Released::Tap);
        assert_eq!(h.due_at(), None);
        h.press(2000, 700);
        h.press(2100, 700);
        assert_eq!(h.due_at(), Some(2700), "a repeat is not a new press");
        assert!(h.tick(2700));
        assert!(!h.tick(2800), "the hold fires once");
        assert_eq!(h.release(), Released::Held);
        assert_eq!(h.release(), Released::Nothing);
    }
}
