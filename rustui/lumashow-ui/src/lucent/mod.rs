//! Lucent, the design language: its device profiles by name, its palette roles and the material contract of the shell.
//!
//! Lucent v2 defines one set of structural tokens per device profile (layout, type scale, targets and which effects
//! are allowed), a palette of twenty roles in a dark and a light variant, and a material (glass, sheen, shadow and
//! depth). The token values themselves arrive with the token and palette imports; this module is the vocabulary a device
//! profile and a shell use to say which profile is meant, the one place a straight palette colour becomes the
//! compositor's premultiplied one, and the frozen shape of the shell's materials ([`material`]).

pub mod data;
pub mod material;
pub mod palette;
pub mod resolve;
pub mod smart;
pub mod tokens;

pub use smart::smart_material;

/// The device profiles of Lucent v2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Profile {
    /// A television, ten feet away, steered with a remote.
    Tv,
    /// A smart display: touch and voice, a short landscape screen.
    Smart,
    Phone,
    Tablet,
    /// A desktop with a pointer and a keyboard.
    Desktop,
    /// A Home Assistant wall panel.
    Ha,
}

impl Profile {
    pub const ALL: [Profile; 6] = [
        Self::Tv,
        Self::Smart,
        Self::Phone,
        Self::Tablet,
        Self::Desktop,
        Self::Ha,
    ];

    /// The name a device profile file and Lucent's tokens give it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Tv => "tv",
            Self::Smart => "smart",
            Self::Phone => "phone",
            Self::Tablet => "tablet",
            Self::Desktop => "desktop",
            Self::Ha => "ha",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_profile_is_found_by_its_own_name_and_no_other() {
        for profile in Profile::ALL {
            assert_eq!(Profile::from_name(profile.name()), Some(profile));
        }
        for unknown in ["", "Smart", "smart-display", "watch", " smart"] {
            assert_eq!(Profile::from_name(unknown), None, "{unknown:?}");
        }
    }
}
