//! The panels of the Rust shell (plan section 4.3, order 2): the quick panel (its tiles, the value cards, the tile editor, the profiles'
//! strip and PIN pad), the input picker, the notification drawer, the context menu, the confirm dialog and the app switcher's chrome.
//! Each is a pure model with host tests (`update`, `view`, `due_at`) and a surface that draws its view with the kit; [`Panels`]
//! joins them to the shell: the compositor's `show` events and keys, the services' answers, the report (`can`, `wants`).
//!
//! The words of a panel are its data's and its state's; a hint is the glyphs of the remote's keys, never a sentence (Lucent).

pub mod arrange;
pub mod clock;
pub mod confirm;
pub mod drawer;
pub mod drawer_surface;
pub mod env;
pub mod hold;
pub mod hub;
pub mod life;
pub mod menu;
pub mod notelog;
pub mod picker;
pub mod picker_surface;
pub mod pin;
pub mod quick;
pub mod quick_layout;
pub mod quick_surface;
pub mod stage;
pub mod switcher;
pub mod ui;

use crate::kit::screen::Effect;
use serde_json::Value;

/// A key of the remote as the panels take it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum K {
    Up,
    Down,
    Left,
    Right,
    Ok,
    Back,
    Digit(u8),
}

impl K {
    /// The compositor's name for a key (`own_key_name`): the arrows, OK, Back and the digits; nothing for the rest.
    pub fn parse(name: &str) -> Option<K> {
        Some(match name {
            "up" => K::Up,
            "down" => K::Down,
            "left" => K::Left,
            "right" => K::Right,
            "ok" => K::Ok,
            "back" => K::Back,
            d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => K::Digit(d.as_bytes()[0] - b'0'),
            _ => return None,
        })
    }
}

/// What a panel asks of the shell beyond effects.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// LumaTV's Settings, at a page (`""` its first).
    Settings(String),
}

/// What a key or a click did: the effects it asks for, what it asks of the shell, what it tells the owner of the surface (an event
/// `topic` and its data, fed to every model as a topic: `menu.picked`, `confirm.answered`, `notelog.action`), and whether the panel is
/// to close.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Out {
    pub effects: Vec<Effect>,
    pub asks: Vec<Ask>,
    pub events: Vec<(String, Value)>,
    pub close: bool,
}

impl Out {
    pub fn effects(effects: Vec<Effect>) -> Self {
        Self { effects, ..Self::default() }
    }

    pub fn close() -> Self {
        Self { close: true, ..Self::default() }
    }

    pub fn merge(&mut self, other: Out) {
        self.effects.extend(other.effects);
        self.asks.extend(other.asks);
        self.events.extend(other.events);
        self.close |= other.close;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_are_the_compositors_names() {
        assert_eq!(K::parse("up"), Some(K::Up));
        assert_eq!(K::parse("ok"), Some(K::Ok));
        assert_eq!(K::parse("back"), Some(K::Back));
        assert_eq!(K::parse("7"), Some(K::Digit(7)));
        assert_eq!(K::parse("volumeup"), None);
        assert_eq!(K::parse("12"), None);
    }
}
