//! What the System menu can do. Each is confirmed first (by the UI; by the basic screen when the UI is not there), and each is carried out by
//! `lumatv-sm`, the one process that knows how to end and start LumaTV (`switcher/system.rs`).

/// One of the System menu's three things.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// End LumaTV and start it again; LG's Home shows meanwhile.
    RestartLumaTv,
    /// Hand the screen back to LG's own Home; LumaTV stays off until it is started again.
    LgHome,
    /// Restart the TV; it comes back on by itself.
    RestartTv,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::RestartLumaTv, Action::LgHome, Action::RestartTv];

    /// The word of `{"op": "system", "action": ...}`.
    pub fn word(self) -> &'static str {
        match self {
            Action::RestartLumaTv => "restart-lumatv",
            Action::LgHome => "lg-home",
            Action::RestartTv => "reboot",
        }
    }

    pub fn parse(word: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.word() == word)
    }

    /// What the menu calls it.
    pub fn label(self) -> &'static str {
        match self {
            Action::RestartLumaTv => "Restart LumaTV",
            Action::LgHome => "LG Home",
            Action::RestartTv => "Restart TV",
        }
    }

    /// The question of its confirm.
    pub fn ask(self) -> &'static str {
        match self {
            Action::RestartLumaTv => "Restart LumaTV?",
            Action::LgHome => "Switch to LG's Home?",
            Action::RestartTv => "Restart the TV?",
        }
    }

    /// What the confirm says will happen.
    pub fn detail(self) -> &'static str {
        match self {
            Action::RestartLumaTv => "Open apps are closed first. LumaTV starts again by itself.",
            Action::LgHome => "Open apps are closed first. LG's own Home takes over, and LumaTV stays off until it is started again.",
            Action::RestartTv => "Open apps are closed first. The TV restarts and comes back on by itself.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_found_by_the_word_it_is_sent_as() {
        for a in Action::ALL {
            assert_eq!(Action::parse(a.word()), Some(a));
        }
        assert_eq!(Action::parse("shutdown"), None);
        assert_eq!(Action::parse(""), None);
    }
}
