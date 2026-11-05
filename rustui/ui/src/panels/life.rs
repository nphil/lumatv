//! How a panel arrives and leaves (`panels/PanelMotion.qml` of the QML shell): a pure account of where it is in its life, on the models'
//! millisecond clock, that its surface follows. A panel that is not wanted has no layers (the scene's budget: a modal kind builds its
//! layers when it opens and drops them when it closes). The gear's press asks for the panel warm: its layers are made hidden, so the
//! slide of the opening starts in a scene that has it. It slides in over `enter_ms`, is open until it is told to go, slides out over
//! `exit_ms` and is then parked: hidden, its layers kept for `linger_ms` so that a panel opened again soon costs nothing, then gone.
//! Another surface that opens takes the parked layers away at once (`release`): nothing keeps hidden layers while another kind is up.
//!
//! Durations: Lucent's layer motion is 220 ms in and 180 ms out, but the gate wants the burst of frames a key starts to be over 180 ms
//! after the key, so a slide takes the shorter of the layer token and 110 ms both ways. Reduced motion is one short fade.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// No layers.
    Gone,
    /// Built and hidden: the gear is down, or the panel has been closed a moment ago.
    Parked,
    /// Sliding in.
    Opening,
    /// At rest, shown.
    Open,
    /// Sliding out.
    Leaving,
}

/// What a step of the life asks of the surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Make the layers (hidden).
    Build,
    /// Show them and start the slide in.
    Show,
    /// The slide in is over.
    Opened,
    /// Start the slide out.
    Hide,
    /// The slide out is over: hide the layers, keep them.
    Parked,
    /// Drop the layers.
    Drop,
}

#[derive(Clone, Copy, Debug)]
pub struct Life {
    phase: Phase,
    until: Option<u64>,
    pub enter_ms: u32,
    pub exit_ms: u32,
    pub linger_ms: u32,
}

/// How long a panel slides in and out (the shorter of the layer token and 110 ms, so the burst of frames a key starts is over by 180 ms), and
/// how long a closed panel's layers are kept hidden for the next opening.
pub const ENTER_MS: u32 = 110;
pub const EXIT_MS: u32 = 110;
pub const LINGER_MS: u32 = 4000;
/// How long a panel that moves on the shared spring (the drawer's container transform) is given to come in and to go: the spring's period
/// is 260 ms and critically damped it lands within 1.05 periods (`kit/flow.rs`), so 280 ms.
pub const SPRING_MS: u32 = 280;

impl Default for Life {
    fn default() -> Self {
        Self { phase: Phase::Gone, until: None, enter_ms: ENTER_MS, exit_ms: EXIT_MS, linger_ms: LINGER_MS }
    }
}

impl Life {
    pub fn new(enter_ms: u32, exit_ms: u32, linger_ms: u32) -> Self {
        Self { enter_ms, exit_ms, linger_ms, ..Self::default() }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Wanted on the screen: the keys are its.
    pub fn presented(&self) -> bool {
        matches!(self.phase, Phase::Opening | Phase::Open)
    }

    /// Drawn: presented, or still sliding out.
    pub fn visible(&self) -> bool {
        matches!(self.phase, Phase::Opening | Phase::Open | Phase::Leaving)
    }

    /// Holds layers.
    pub fn built(&self) -> bool {
        self.phase != Phase::Gone
    }

    /// The gear is down: build now, hidden. Nothing when the panel is already built or up.
    pub fn warm(&mut self, now: u64) -> Option<Change> {
        if self.phase != Phase::Gone {
            return None;
        }
        self.phase = Phase::Parked;
        self.until = Some(now + u64::from(self.linger_ms));
        Some(Change::Build)
    }

    /// The panel is wanted: the changes to make, in order.
    pub fn show(&mut self, now: u64) -> Vec<Change> {
        let mut out = Vec::new();
        match self.phase {
            Phase::Opening | Phase::Open => return out,
            Phase::Gone => out.push(Change::Build),
            Phase::Parked | Phase::Leaving => {}
        }
        out.push(Change::Show);
        self.phase = Phase::Opening;
        self.until = Some(now + u64::from(self.enter_ms));
        out
    }

    /// The panel is no longer wanted.
    pub fn hide(&mut self, now: u64) -> Option<Change> {
        if !self.presented() {
            return None;
        }
        self.phase = Phase::Leaving;
        self.until = Some(now + u64::from(self.exit_ms));
        Some(Change::Hide)
    }

    /// Takes the parked layers away at once (another surface is opening); a panel that is shown or leaving is left alone.
    pub fn release(&mut self) -> Option<Change> {
        if self.phase == Phase::Parked {
            self.phase = Phase::Gone;
            self.until = None;
            Some(Change::Drop)
        } else {
            None
        }
    }

    /// A panel that is sliding out goes now: a sheet over the whole screen came up over it at once (Settings with no fade, from 100 Hz), and the
    /// slide went on under the sheet's 96 % glass and in its margins. Opened from the quick panel, Settings drew that slide's 110 ms, the panel's
    /// 37 layers in every frame (66-109 layers on the set while Settings filled in), and the dim behind the panel went at the slide's end in a
    /// frame of the whole screen of its own (2.07 Mpx, 7.5 ms of GPU on the set, a refresh late).
    pub fn cut(&mut self) -> Option<Change> {
        if self.phase != Phase::Leaving {
            return None;
        }
        self.phase = Phase::Gone;
        self.until = None;
        Some(Change::Drop)
    }

    /// Everything goes (the screen went off, the shell restarted its look).
    pub fn reset(&mut self) -> Option<Change> {
        let had = self.phase != Phase::Gone;
        self.phase = Phase::Gone;
        self.until = None;
        had.then_some(Change::Drop)
    }

    pub fn tick(&mut self, now: u64) -> Option<Change> {
        let due = self.until.filter(|d| now >= *d)?;
        let _ = due;
        match self.phase {
            Phase::Opening => {
                self.phase = Phase::Open;
                self.until = None;
                Some(Change::Opened)
            }
            Phase::Leaving => {
                self.phase = Phase::Parked;
                self.until = Some(now + u64::from(self.linger_ms));
                Some(Change::Parked)
            }
            Phase::Parked => {
                self.phase = Phase::Gone;
                self.until = None;
                Some(Change::Drop)
            }
            _ => {
                self.until = None;
                None
            }
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        self.until
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_builds_when_asked_slides_in_rests_slides_out_is_parked_and_goes() {
        let mut l = Life::default();
        assert!(!l.built());
        assert_eq!(l.show(1000), vec![Change::Build, Change::Show]);
        assert!(l.presented() && l.visible());
        assert_eq!(l.tick(1109), None);
        assert_eq!(l.tick(1110), Some(Change::Opened));
        assert_eq!(l.phase(), Phase::Open);
        assert_eq!(l.hide(5000), Some(Change::Hide));
        assert!(!l.presented() && l.visible(), "drawn while it slides out");
        assert_eq!(l.tick(5110), Some(Change::Parked));
        assert!(!l.visible() && l.built(), "parked: hidden, built");
        assert_eq!(l.due_at(), Some(5110 + 4000));
        assert_eq!(l.tick(9110), Some(Change::Drop));
        assert!(!l.built());
    }

    #[test]
    fn the_gears_press_builds_it_hidden_and_the_tap_shows_it_with_nothing_to_build() {
        let mut l = Life::default();
        assert_eq!(l.warm(0), Some(Change::Build));
        assert!(l.built() && !l.visible());
        assert_eq!(l.warm(10), None, "warm twice builds once");
        assert_eq!(l.show(80), vec![Change::Show], "the slide starts in a scene that has the panel");
        // a warm that is not followed by a tap goes with the linger
        let mut l = Life::default();
        l.warm(0);
        assert_eq!(l.tick(4000), Some(Change::Drop));
    }

    #[test]
    fn opening_again_while_parked_or_leaving_builds_nothing_and_a_release_takes_only_the_parked() {
        let mut l = Life::default();
        l.show(0);
        l.tick(110);
        l.hide(200);
        assert_eq!(l.show(250), vec![Change::Show], "a panel that is still leaving slides back in");
        assert_eq!(l.release(), None, "an open panel is not released");
        l.hide(400);
        assert_eq!(l.release(), None, "nor one that is leaving");
        l.tick(510);
        assert_eq!(l.release(), Some(Change::Drop));
        assert!(!l.built());
        assert_eq!(l.release(), None);
    }

    #[test]
    fn hiding_what_is_not_up_does_nothing_and_reset_drops_whatever_there_is() {
        let mut l = Life::default();
        assert_eq!(l.hide(0), None);
        l.show(0);
        assert_eq!(l.reset(), Some(Change::Drop));
        assert_eq!(l.reset(), None);
        assert_eq!(l.due_at(), None);
    }
}
