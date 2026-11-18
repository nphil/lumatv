//! Which kind of input the person is using, so hints draw the controller's buttons for a pad and the remote's keys for a remote.
use super::model::Key;
use lumatv_gfn_pad::button;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Remote,
    Controller,
}

/// The last kind of input used and how many controllers are connected.
///
/// Hints follow the last press: the first one of the other kind switches them, and nothing else does, so a pad lying idle beside the
/// remote changes nothing. Before any press a connected controller is the default. With none connected the remote's glyphs show
/// whatever was pressed last, since a hint for a button that is not there helps nobody.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    last: Option<Source>,
    controllers: usize,
}
impl Sources {
    pub fn glyphs(&self) -> Source {
        match (self.last, self.controllers) {
            (_, 0) => Source::Remote,
            (Some(source), _) => source,
            (None, _) => Source::Controller,
        }
    }
    /// A press of `source`. True when the glyphs change.
    pub fn used(&mut self, source: Source) -> bool {
        let before = self.glyphs();
        self.last = Some(source);
        // Engines send a pad's keys while the shell has released it, before the shell has counted the pad.
        self.controllers = self.controllers.max(usize::from(source == Source::Controller));
        before != self.glyphs()
    }
    /// The number of connected controllers. True when the glyphs change.
    pub fn connected(&mut self, count: usize) -> bool {
        let before = self.glyphs();
        self.controllers = count;
        if count == 0 && self.last == Some(Source::Controller) {
            // The pad left: a controller that comes back is a fresh start, not a continuation.
            self.last = None;
        }
        before != self.glyphs()
    }
}

/// The face buttons the menus use. X is bound to nothing there; the bumpers, View and Menu buttons are too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    A,
    B,
    Y,
}
impl Face {
    pub const ALL: [Self; 3] = [Self::A, Self::B, Self::Y];
    pub fn letter(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::Y => "Y",
        }
    }
    /// The pad button that sends this face's key to the menus.
    pub fn button(self) -> u16 {
        match self {
            Self::A => button::A,
            Self::B => button::B,
            Self::Y => button::Y,
        }
    }
    /// The key the menus receive from it (the remote's OK, Back and Menu).
    pub fn key(self) -> Key {
        match self {
            Self::A => Key::Ok,
            Self::B => Key::Back,
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumatv_gfn_pad::{PadState, menu::MenuKeys};

    #[test]
    fn a_connected_controller_is_the_start_default_and_a_remote_key_takes_over() {
        let mut s = Sources::default();
        assert_eq!(s.glyphs(), Source::Remote);
        assert!(s.connected(1), "a pad present before any press shows its buttons");
        assert_eq!(s.glyphs(), Source::Controller);
        assert!(s.used(Source::Remote));
        assert_eq!(s.glyphs(), Source::Remote);
        assert!(s.used(Source::Controller));
        assert_eq!(s.glyphs(), Source::Controller);
    }

    #[test]
    fn only_the_first_input_of_the_other_kind_switches() {
        let mut s = Sources::default();
        s.connected(1);
        assert!(!s.used(Source::Controller), "the default already shows the pad");
        assert!(s.used(Source::Remote));
        for _ in 0..3 {
            assert!(!s.used(Source::Remote), "more of the same changes nothing");
        }
        assert!(!s.connected(2), "another pad joining does not take the remote's glyphs away");
        assert!(!s.connected(1));
        assert_eq!(s.glyphs(), Source::Remote);
    }

    #[test]
    fn without_a_controller_the_remote_shows_and_a_returning_pad_is_a_fresh_start() {
        let mut s = Sources::default();
        s.connected(1);
        assert!(s.connected(0), "the pad left");
        assert_eq!(s.glyphs(), Source::Remote);
        assert!(s.connected(1), "back, with no press since: the default again");
        assert_eq!(s.glyphs(), Source::Controller);
        s.used(Source::Remote);
        s.connected(0);
        assert!(!s.connected(1), "the remote was the last press, and stays");
        assert_eq!(s.glyphs(), Source::Remote);
    }

    #[test]
    fn an_engine_forwarding_a_pad_key_counts_as_a_pad() {
        let mut s = Sources::default();
        assert!(s.used(Source::Controller));
        assert_eq!(s.glyphs(), Source::Controller);
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
        for key in [Key::Up, Key::Down, Key::Left, Key::Right, Key::Guide] {
            assert_eq!(key.face(), None, "{key:?} has no face button");
        }
    }
}
