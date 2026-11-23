//! Which kind of input the person is using, so hints draw the controller's buttons for a pad and the remote's keys for a remote.
use super::model::Key;
use lumatv_gfn_pad::{MAX_PADS, button};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Remote,
    Controller,
}

/// Whose markings a hint draws on a face button. Xbox is what every pad that is not recognised shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadKind {
    #[default]
    Xbox,
    PlayStation,
    GameSir,
    Leadjoy,
}
impl PadKind {
    /// The kind a pad's name says. The Cyclone 2 in DS4 mode is `054c:09cc` on the wire, the same id as a real DualShock 4 v2, but the
    /// kernel names an HID pad after its USB manufacturer string and the receiver's is GameSir's own ("Guangzhou Chicken Run Network
    /// Technology Co., Ltd."), so the name tells them apart where the id cannot. A Xeno Plus names itself Leadjoy or Xeno. Anything else
    /// keeps the Xbox glyphs.
    pub fn from_name(name: &str) -> Self {
        let name = name.to_ascii_lowercase();
        if name.contains("chicken run") || name.contains("gamesir") {
            Self::GameSir
        } else if name.contains("leadjoy") || name.contains("xeno") || name.contains("liteemf") {
            Self::Leadjoy
        } else {
            Self::Xbox
        }
    }
}

/// The Controllers setting "Button labels": follow the pad in use, or always draw one kind's markings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Labels {
    #[default]
    Automatic,
    Fixed(PadKind),
}
impl Labels {
    pub const ALL: [Self; 5] =
        [Self::Automatic, Self::Fixed(PadKind::Xbox), Self::Fixed(PadKind::PlayStation), Self::Fixed(PadKind::GameSir), Self::Fixed(PadKind::Leadjoy)];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|l| *l == self).unwrap_or(0)
    }
    pub fn name(self) -> &'static str {
        ["Automatic", "Xbox", "PlayStation", "GameSir Cyclone 2", "Leadjoy Xeno Plus"][self.index()]
    }
    pub fn key(self) -> &'static str {
        ["automatic", "xbox", "playstation", "gamesir", "leadjoy"][self.index()]
    }
    pub fn from_key(key: &str) -> Self {
        Self::ALL.into_iter().find(|l| l.key() == key).unwrap_or_default()
    }
    /// The kind to draw: the setting's own, or `detected` when it is automatic.
    pub fn kind(self, detected: Option<PadKind>) -> PadKind {
        match self {
            Self::Automatic => detected.unwrap_or_default(),
            Self::Fixed(kind) => kind,
        }
    }
}

/// The last kind of input used, which controllers are connected and which of them was pressed last.
///
/// Hints follow the last press: the first one of the other kind switches them, and nothing else does, so a pad lying idle beside the
/// remote changes nothing. Before any press a connected controller is the default. With none connected the remote's glyphs show
/// whatever was pressed last, since a hint for a button that is not there helps nobody. With several pads the markings are those of
/// the one pressed last, or of the first connected before any press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    last: Option<Source>,
    controllers: usize,
    kinds: [Option<PadKind>; MAX_PADS],
    slot: Option<usize>,
}
impl Sources {
    pub fn glyphs(&self) -> Source {
        match (self.last, self.controllers) {
            (_, 0) => Source::Remote,
            (Some(source), _) => source,
            (None, _) => Source::Controller,
        }
    }
    /// The kind of the pad whose markings hints draw, when its name is known.
    pub fn pad(&self) -> Option<PadKind> {
        self.slot.and_then(|slot| self.kinds[slot]).or_else(|| self.kinds.iter().flatten().next().copied())
    }
    /// What the hints show: the remote's icons, or a pad's markings.
    fn look(&self) -> (Source, Option<PadKind>) {
        let source = self.glyphs();
        (source, self.pad().filter(|_| source == Source::Controller))
    }
    /// A press of `source`. True when the glyphs change.
    pub fn used(&mut self, source: Source) -> bool {
        let before = self.look();
        self.last = Some(source);
        // Engines send a pad's keys while the shell has released it, before the shell has counted the pad.
        self.controllers = self.controllers.max(usize::from(source == Source::Controller));
        before != self.look()
    }
    /// A press on the pad in `slot`. True when the glyphs change.
    pub fn used_pad(&mut self, slot: usize) -> bool {
        let before = self.look();
        self.slot = Some(slot).filter(|slot| *slot < MAX_PADS);
        self.used(Source::Controller);
        before != self.look()
    }
    /// The connected controllers' names by slot. True when the glyphs change.
    pub fn connected(&mut self, names: &[Option<String>; MAX_PADS]) -> bool {
        let before = self.look();
        self.controllers = names.iter().flatten().count();
        self.kinds = std::array::from_fn(|i| names[i].as_deref().map(PadKind::from_name));
        if self.slot.is_some_and(|slot| names[slot].is_none()) {
            self.slot = None;
        }
        if self.controllers == 0 && self.last == Some(Source::Controller) {
            // The pad left: a controller that comes back is a fresh start, not a continuation.
            self.last = None;
        }
        before != self.look()
    }
}

/// A direction a pad holds, which repeats: the first move after 300 ms, then one every `interval`, which a held stick sets by how far it
/// is pushed (a nudge steps at 200 ms, a full push at 100) and which quickens by up to 30 % over the next 1.2 s of holding. Release
/// stops it; the springs the screens move on carry what is left. A call gives at most one move, so a slow frame takes a single step and
/// not a burst.
#[derive(Clone, Copy, Debug)]
pub struct Repeat {
    key: Key,
    since_ms: u64,
    next_ms: u64,
    strength: f32,
}
impl Repeat {
    const FIRST_MS: u64 = 300;
    const GENTLE_MS: f32 = 200.0;
    const FULL_MS: f32 = 100.0;
    const RAMP_MS: f32 = 1_200.0;
    const QUICKEST: f32 = 0.7;
    /// A d-pad direction, which is held at full strength.
    pub fn start(key: Key, now_ms: u64) -> Self {
        Self { key, since_ms: now_ms, next_ms: now_ms + Self::FIRST_MS, strength: 1.0 }
    }
    pub fn key(&self) -> Key {
        self.key
    }
    /// How hard the direction is held now, 0 to 255 as the pad reader sends it.
    pub fn push(&mut self, strength: u8) {
        self.strength = f32::from(strength) / 255.0;
    }
    /// Milliseconds between moves once `held_ms` of holding have gone.
    fn interval(&self, held_ms: u64) -> u64 {
        let base = Self::GENTLE_MS + (Self::FULL_MS - Self::GENTLE_MS) * self.strength;
        let warm = ((held_ms.saturating_sub(Self::FIRST_MS)) as f32 / Self::RAMP_MS).min(1.0);
        let warm = warm * warm * (3.0 - 2.0 * warm);
        (base * (1.0 - (1.0 - Self::QUICKEST) * warm)).round() as u64
    }
    /// The key to move with, once its time has come.
    pub fn due(&mut self, now_ms: u64) -> Option<Key> {
        if now_ms < self.next_ms {
            return None;
        }
        self.next_ms = now_ms + self.interval(now_ms - self.since_ms);
        Some(self.key)
    }
    /// Milliseconds until the next move is due.
    pub fn wake_in(&self, now_ms: u64) -> u64 {
        self.next_ms.saturating_sub(now_ms)
    }
}

/// The face buttons the menus use. The View and Menu buttons are bound to nothing there, and the bumpers send [`Key::Prev`] and
/// [`Key::Next`] without a release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    A,
    B,
    X,
    Y,
}
impl Face {
    pub const ALL: [Self; 4] = [Self::A, Self::B, Self::X, Self::Y];
    pub fn letter(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::X => "X",
            Self::Y => "Y",
        }
    }
    /// The pad button that sends this face's key to the menus.
    pub fn button(self) -> u16 {
        match self {
            Self::A => button::A,
            Self::B => button::B,
            Self::X => button::X,
            Self::Y => button::Y,
        }
    }
    /// The key the menus receive from it (the remote's OK, Back and Menu; X is the second action, Alt, which the remote has no key for).
    pub fn key(self) -> Key {
        match self {
            Self::A => Key::Ok,
            Self::B => Key::Back,
            Self::X => Key::Alt,
            Self::Y => Key::Menu,
        }
    }
}
impl Key {
    /// The face button that sends this key from a controller, if one does. The d-pad and left stick send the arrows, and the Xbox
    /// button sends Guide.
    pub fn face(self) -> Option<Face> {
        Face::ALL.into_iter().find(|face| face.key() == self)
    }
    pub fn from_menu(key: lumatv_gfn_pad::menu::MenuKey) -> Self {
        use lumatv_gfn_pad::menu::MenuKey;
        match key {
            MenuKey::Up => Self::Up,
            MenuKey::Down => Self::Down,
            MenuKey::Left => Self::Left,
            MenuKey::Right => Self::Right,
            MenuKey::Ok => Self::Ok,
            MenuKey::Back => Self::Back,
            MenuKey::Menu => Self::Menu,
            MenuKey::Alt => Self::Alt,
            MenuKey::PageLeft => Self::Prev,
            MenuKey::PageRight => Self::Next,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumatv_gfn_pad::{PadState, menu::MenuKeys};

    /// `names` in the first slots, as the shell reports them.
    pub(crate) fn pads(names: &[&str]) -> [Option<String>; MAX_PADS] {
        std::array::from_fn(|i| names.get(i).map(|n| (*n).to_owned()))
    }
    fn n(count: usize) -> [Option<String>; MAX_PADS] {
        pads(&["Pad"; MAX_PADS][..count])
    }

    #[test]
    fn a_connected_controller_is_the_start_default_and_a_remote_key_takes_over() {
        let mut s = Sources::default();
        assert_eq!(s.glyphs(), Source::Remote);
        assert!(s.connected(&n(1)), "a pad present before any press shows its buttons");
        assert_eq!(s.glyphs(), Source::Controller);
        assert!(s.used(Source::Remote));
        assert_eq!(s.glyphs(), Source::Remote);
        assert!(s.used(Source::Controller));
        assert_eq!(s.glyphs(), Source::Controller);
    }

    #[test]
    fn only_the_first_input_of_the_other_kind_switches() {
        let mut s = Sources::default();
        s.connected(&n(1));
        assert!(!s.used(Source::Controller), "the default already shows the pad");
        assert!(s.used(Source::Remote));
        for _ in 0..3 {
            assert!(!s.used(Source::Remote), "more of the same changes nothing");
        }
        assert!(!s.connected(&n(2)), "another pad joining does not take the remote's glyphs away");
        assert!(!s.connected(&n(1)));
        assert_eq!(s.glyphs(), Source::Remote);
    }

    #[test]
    fn without_a_controller_the_remote_shows_and_a_returning_pad_is_a_fresh_start() {
        let mut s = Sources::default();
        s.connected(&n(1));
        assert!(s.connected(&n(0)), "the pad left");
        assert_eq!(s.glyphs(), Source::Remote);
        assert!(s.connected(&n(1)), "back, with no press since: the default again");
        assert_eq!(s.glyphs(), Source::Controller);
        s.used(Source::Remote);
        s.connected(&n(0));
        assert!(!s.connected(&n(1)), "the remote was the last press, and stays");
        assert_eq!(s.glyphs(), Source::Remote);
    }

    #[test]
    fn an_engine_forwarding_a_pad_key_counts_as_a_pad() {
        let mut s = Sources::default();
        assert!(s.used(Source::Controller));
        assert_eq!(s.glyphs(), Source::Controller);
    }

    #[test]
    fn a_pad_is_told_by_its_name_and_the_last_pressed_one_decides() {
        let cyclone = "Guangzhou Chicken Run Network Technology Co., Ltd. Wireless Controller";
        assert_eq!(PadKind::from_name(cyclone), PadKind::GameSir);
        assert_eq!(PadKind::from_name("GameSir-G7 SE"), PadKind::GameSir);
        assert_eq!(PadKind::from_name("Leadjoy Xeno Plus"), PadKind::Leadjoy);
        assert_eq!(PadKind::from_name("Sony Interactive Entertainment Wireless Controller"), PadKind::Xbox);
        let mut s = Sources::default();
        s.connected(&pads(&["Microsoft X-Box pad", cyclone]));
        assert_eq!(s.pad(), Some(PadKind::Xbox), "before any press, the first pad");
        assert!(s.used_pad(1));
        assert_eq!(s.pad(), Some(PadKind::GameSir));
        assert!(!s.used_pad(1), "the same pad again changes nothing");
        s.connected(&pads(&["Microsoft X-Box pad"]));
        assert_eq!(s.pad(), Some(PadKind::Xbox), "the pad that was pressed left");
    }

    #[test]
    fn the_setting_overrides_the_detected_pad() {
        assert_eq!(Labels::Automatic.kind(Some(PadKind::GameSir)), PadKind::GameSir);
        assert_eq!(Labels::Automatic.kind(None), PadKind::Xbox);
        assert_eq!(Labels::Fixed(PadKind::Leadjoy).kind(Some(PadKind::GameSir)), PadKind::Leadjoy);
        for l in Labels::ALL {
            assert_eq!(Labels::from_key(l.key()), l);
        }
    }

    /// Every face button a hint can name sends the very key the hint stands for: feeding the button to the menu's pad reader yields it.
    #[test]
    fn each_face_a_hint_names_really_sends_its_key() {
        for face in Face::ALL {
            let mut keys = MenuKeys::default();
            let mut sent = Vec::new();
            keys.feed(&PadState { buttons: face.button(), ..PadState::default() }, |key| sent.push(Key::from_menu(key)));
            assert_eq!(sent, [face.key()], "{face:?}");
            assert_eq!(face.key().face(), Some(face));
        }
        for key in [Key::Up, Key::Down, Key::Left, Key::Right, Key::Guide, Key::Prev, Key::Next] {
            assert_eq!(key.face(), None, "{key:?} has no face button");
        }
    }

    #[test]
    fn the_bumpers_are_the_previous_and_next_key_from_either_pad_path() {
        let mut keys = MenuKeys::default();
        let mut sent = Vec::new();
        for buttons in [button::LEFT_SHOULDER, 0, button::RIGHT_SHOULDER] {
            keys.feed(&PadState { buttons, ..PadState::default() }, |key| sent.push(Key::from_menu(key)));
        }
        assert_eq!(sent, [Key::Prev, Key::Next], "the shell's own pad reader");
        let engine = [lumagame_core::ipc::PadKey::PageLeft, lumagame_core::ipc::PadKey::PageRight].map(Key::from_pad);
        assert_eq!(engine, [Some(Key::Prev), Some(Key::Next)], "a stream's engine forwarding its menu keys");
    }
    #[test]
    fn a_held_direction_repeats_after_300_ms_with_one_move_per_frame() {
        let mut held = Repeat::start(Key::Down, 1_000);
        assert_eq!(held.due(1_299), None);
        assert_eq!(held.wake_in(1_299), 1);
        assert_eq!(held.due(1_300), Some(Key::Down));
        assert_eq!(held.wake_in(1_300), 100, "the d-pad starts at the full push's 100 ms");
        // A slow frame gets one move, and the next waits its full interval.
        assert_eq!(held.due(1_900), Some(Key::Down));
        assert_eq!(held.due(1_950), None);
    }

    #[test]
    fn a_stick_pushed_further_steps_faster_and_holding_quickens_it() {
        let gap = |strength: u8, held_ms: u64| {
            let mut held = Repeat::start(Key::Right, 0);
            held.push(strength);
            let mut now = Repeat::FIRST_MS;
            while now < held_ms {
                held.due(now);
                now += held.wake_in(now);
            }
            held.due(now);
            held.wake_in(now)
        };
        assert_eq!(gap(0, 300), 200, "a nudge");
        assert_eq!(gap(255, 300), 100, "a full push");
        assert!(gap(128, 300) > 140 && gap(128, 300) < 160, "between");
        assert!(gap(255, 900) < gap(255, 300), "holding quickens it");
        assert_eq!(gap(255, 5_000), 70, "to 70 % and no further");
        assert_eq!(gap(0, 5_000), 140);
    }

    #[test]
    fn easing_the_stick_off_slows_the_next_step_after_the_one_due() {
        let mut held = Repeat::start(Key::Left, 0);
        assert_eq!(held.due(300), Some(Key::Left));
        held.push(0);
        assert_eq!(held.due(400), Some(Key::Left), "the step already due was timed at the full push");
        assert!(held.wake_in(400) > 190, "a nudge's 200 ms, barely quickened");
    }
}
