//! A pad as a remote for menus: the d-pad or the left stick moves, A chooses, B goes back. The edge of a press is one key, and a
//! stick pushed past [`STICK`] is one key until it comes back, so a held stick is one step.
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
}

/// How far a stick goes before it counts as a direction: about 60 % of its travel.
pub const STICK: i16 = 20_000;

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
        let dir = if pressed & button::DPAD_UP != 0 || s.left_y > STICK {
            Some(MenuKey::Up)
        } else if pressed & button::DPAD_DOWN != 0 || s.left_y < -STICK {
            Some(MenuKey::Down)
        } else if pressed & button::DPAD_LEFT != 0 || s.left_x < -STICK {
            Some(MenuKey::Left)
        } else if pressed & button::DPAD_RIGHT != 0 || s.left_x > STICK {
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
        let stick = |left_x, left_y| PadState { left_x, left_y, ..Default::default() };
        assert_eq!(keys(&mut m, stick(0, STICK + 1)), [MenuKey::Up], "up is positive");
        assert!(keys(&mut m, stick(0, 32_767)).is_empty());
        assert_eq!(keys(&mut m, stick(-STICK - 1, 0)), [MenuKey::Left], "a new direction without passing the centre");
        assert!(keys(&mut m, stick(STICK, 0)).is_empty(), "the threshold itself is the centre");
        assert_eq!(keys(&mut m, stick(STICK + 1, 0)), [MenuKey::Right]);
    }
}
