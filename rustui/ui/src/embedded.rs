//! The module's icons as compiled in (`build.rs`): what the shell draws its status glyphs and weather pictures from when the module
//! directory is not there (a failed apply removes it, and the shell must not go blank with it).

include!(concat!(env!("OUT_DIR"), "/embedded_icons.rs"));

/// The PNG at `<module>/icons/<rel>` (`lucide/bell.png`, `wx-sun_d.png`), if the program carries it.
pub fn icon(rel: &str) -> Option<&'static [u8]> {
    ICONS.binary_search_by(|(name, _)| (*name).cmp(rel)).ok().map(|i| ICONS[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_glyphs_and_weather_pictures_are_carried() {
        assert!(icon("lucide/bell-ring.png").is_some());
        assert!(icon("wx-sun_d.png").is_some());
        assert!(icon("lucide/no-such-icon.png").is_none());
        assert!(icon("../theme/theme.png").is_none());
    }
}
