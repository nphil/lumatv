//! A pad as a remote for menus: the d-pad or the left stick moves, A chooses, B goes back, Y is the menu's own action (Menu) and X its
//! second (Alt), and the shoulder buttons turn pages (PageLeft, PageRight). The edge of a press is one key, and a stick pushed past
//! [`STICK`] is one key until it comes back under [`STICK_RELEASE`], so a held stick is one step; how far it is pushed
//! ([`strength`]) is how fast a shell that repeats a held direction repeats it.
use crate::{PadState, button};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Left,
    Right,
    Ok,
    Back,
    Menu,
    Alt,
    PageLeft,
    PageRight,
}

/// How far a stick goes before it counts as a direction: about 43 % of its travel. A thumb rests and drifts inside the dead zone below it.
pub const STICK: i16 = 14_000;
/// How far back a stick that counts as a direction must come before it stops: about 27 %, so a thumb easing off at the threshold does
/// not chatter between two keys.
pub const STICK_RELEASE: i16 = 9_000;
/// The push that is all of the repeat's speed: about 92 % of the travel.
const FULL: i16 = 30_000;

/// How hard `key`'s direction is held, 0 to 255: the d-pad is all of it, a stick runs from nothing at [`STICK`] to all at [`FULL`].
pub fn strength(s: &PadState, key: MenuKey) -> u8 {
    let (dpad, axis) = match key {
        MenuKey::Up => (button::DPAD_UP, s.left_y),
        MenuKey::Down => (button::DPAD_DOWN, s.left_y.saturating_neg()),
        MenuKey::Left => (button::DPAD_LEFT, s.left_x.saturating_neg()),
        MenuKey::Right => (button::DPAD_RIGHT, s.left_x),
        _ => return 0,
    };
    if s.buttons & dpad != 0 {
        return 255;
    }
    let travelled = f32::from(axis.saturating_sub(STICK).max(0)) / f32::from(FULL - STICK);
    (travelled.min(1.0) * 255.0).round() as u8
}

/// One pad's reports turned into menu keys; keep one per pad.
#[derive(Debug, Clone, Default)]
pub struct MenuKeys {
    buttons: u16,
    dir: Option<MenuKey>,
}

impl MenuKeys {
    /// The keys one report makes, chooser first, to `key`.
    pub fn feed(&mut self, s: &PadState, mut key: impl FnMut(MenuKey)) {
        let pressed = s.buttons & !self.buttons;
        self.buttons = s.buttons;
        if pressed & button::A != 0 {
            key(MenuKey::Ok);
        }
        if pressed & button::B != 0 {
            key(MenuKey::Back);
        }
        if pressed & button::Y != 0 {
            key(MenuKey::Menu);
        }
        if pressed & button::X != 0 {
            key(MenuKey::Alt);
        }
        if pressed & button::LEFT_SHOULDER != 0 {
            key(MenuKey::PageLeft);
        }
        if pressed & button::RIGHT_SHOULDER != 0 {
            key(MenuKey::PageRight);
        }
        // A direction already held stays held until the stick is back under the lower threshold.
        let past = |key: MenuKey, axis: i16| axis > if self.dir == Some(key) { STICK_RELEASE } else { STICK };
        let dir = if pressed & button::DPAD_UP != 0 || past(MenuKey::Up, s.left_y) {
            Some(MenuKey::Up)
        } else if pressed & button::DPAD_DOWN != 0 || past(MenuKey::Down, s.left_y.saturating_neg()) {
            Some(MenuKey::Down)
        } else if pressed & button::DPAD_LEFT != 0 || past(MenuKey::Left, s.left_x.saturating_neg()) {
            Some(MenuKey::Left)
        } else if pressed & button::DPAD_RIGHT != 0 || past(MenuKey::Right, s.left_x) {
            Some(MenuKey::Right)
        } else {
            None
        };
        if dir != self.dir {
            if let Some(d) = dir {
                key(d);
            }
            self.dir = dir;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(m: &mut MenuKeys, s: PadState) -> Vec<MenuKey> {
        let mut out = Vec::new();
        m.feed(&s, |k| out.push(k));
        out
    }

    #[test]
    fn a_press_is_one_key_and_a_held_stick_is_one_step() {
        let mut m = MenuKeys::default();
        let down = |buttons| PadState { buttons, ..Default::default() };
        assert_eq!(keys(&mut m, down(button::A | button::DPAD_DOWN)), [MenuKey::Ok, MenuKey::Down]);
        assert!(keys(&mut m, down(button::A | button::DPAD_DOWN)).is_empty(), "held buttons say nothing more");
        assert!(keys(&mut m, down(0)).is_empty());
        assert_eq!(keys(&mut m, down(button::B)), [MenuKey::Back]);
        assert_eq!(keys(&mut m, down(button::Y)), [MenuKey::Menu]);
        assert!(keys(&mut m, down(button::Y)).is_empty());
        assert_eq!(keys(&mut m, down(button::X)), [MenuKey::Alt]);
        assert_eq!(keys(&mut m, down(button::LEFT_SHOULDER)), [MenuKey::PageLeft]);
        assert_eq!(keys(&mut m, down(button::LEFT_SHOULDER | button::RIGHT_SHOULDER)), [MenuKey::PageRight], "one key per press, however the other shoulder is held");
        assert!(keys(&mut m, down(button::LEFT_SHOULDER | button::RIGHT_SHOULDER)).is_empty());
        let stick = |left_x, left_y| PadState { left_x, left_y, ..Default::default() };
        assert_eq!(keys(&mut m, stick(0, STICK + 1)), [MenuKey::Up], "up is positive");
        assert!(keys(&mut m, stick(0, 32_767)).is_empty());
        assert_eq!(keys(&mut m, stick(-STICK - 1, 0)), [MenuKey::Left], "a new direction without passing the centre");
        assert!(keys(&mut m, stick(STICK, 0)).is_empty(), "the threshold itself is the centre");
        assert_eq!(keys(&mut m, stick(STICK + 1, 0)), [MenuKey::Right]);
    }

    #[test]
    fn a_stick_easing_off_keeps_its_direction_until_the_lower_threshold() {
        let mut m = MenuKeys::default();
        let stick = |left_x| PadState { left_x, ..Default::default() };
        assert_eq!(keys(&mut m, stick(STICK + 1)), [MenuKey::Right]);
        assert!(keys(&mut m, stick(STICK - 1_000)).is_empty(), "under the threshold, still the same step");
        assert!(keys(&mut m, stick(STICK_RELEASE + 1)).is_empty());
        assert!(keys(&mut m, stick(STICK_RELEASE)).is_empty(), "released");
        assert!(keys(&mut m, stick(STICK - 1)).is_empty(), "and it takes the full threshold to press again");
        assert_eq!(keys(&mut m, stick(STICK + 1)), [MenuKey::Right]);
    }

    #[test]
    fn strength_is_all_of_the_dpad_and_a_stick_from_the_threshold_to_nearly_full_travel() {
        let stick = |left_x, left_y| PadState { left_x, left_y, ..Default::default() };
        assert_eq!(strength(&PadState { buttons: button::DPAD_LEFT, ..Default::default() }, MenuKey::Left), 255);
        assert_eq!(strength(&stick(STICK, 0), MenuKey::Right), 0);
        assert_eq!(strength(&stick(FULL, 0), MenuKey::Right), 255);
        assert_eq!(strength(&stick(i16::MAX, 0), MenuKey::Right), 255);
        let mid = strength(&stick(STICK + (FULL - STICK) / 2, 0), MenuKey::Right);
        assert!((126..=129).contains(&mid), "{mid}");
        assert_eq!(strength(&stick(0, -FULL), MenuKey::Down), 255, "down is negative");
        assert_eq!(strength(&stick(FULL, 0), MenuKey::Left), 0, "the other way");
        assert_eq!(strength(&stick(FULL, 0), MenuKey::Ok), 0);
    }
}
