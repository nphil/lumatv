//! The pages of Settings, in the rail's order and then the pages that hang under one of its sections. Each is a function from what the
//! shell knows to rows (`settings/Page*.qml` of the QML shell, row for row).

use super::page::Page;

pub mod about;
pub mod access;
pub mod ai;
pub mod ambient;
pub mod bluetooth;
pub mod cameras;
pub mod capture;
pub mod home;
pub mod homeassistant;
pub mod inputs;
pub mod language;
pub mod network;
pub mod owner;
pub mod picture;
pub mod plugin_rows;
pub mod privacy;
pub mod profiles;
pub mod receiver;
pub mod remote;
pub mod saver;
pub mod sound;
pub mod support;
pub mod system;
pub mod util;
pub mod widgets;

/// Every page: the rail's sections in the rail's order (Privacy and Profiles first, then LG's order), then the pages that hang under one.
pub fn all() -> Vec<Page> {
    vec![
        privacy::page(),
        profiles::page(),
        picture::page(),
        sound::page(),
        inputs::page(),
        network::page(),
        ambient::page(),
        home::page(),
        remote::page(),
        ai::page(),
        system::page(),
        privacy::held(),
        profiles::person(),
        profiles::avatars(),
        home::popups(),
        widgets::page(),
        homeassistant::page(),
        cameras::page(),
        cameras::notify(),
        saver::page(),
        remote::app_key(),
        system::clock(),
        system::timer_on(),
        system::timer_off(),
        language::page(),
        access::safety(),
        access::audio_guide(),
        access::audio_options(),
        access::emergency(),
        access::captions(),
        about::page(),
        support::page(),
        support::delete_data(),
        support::permissions(),
        capture::page(),
        picture::fine_tune(),
        picture::white_balance(),
        picture::signalling(),
        sound::equalizer(),
        sound::hdmi_audio(),
        sound::usb(),
        receiver::page(),
        inputs::channels(),
        bluetooth::audio(),
        bluetooth::hid(),
        bluetooth::audio_device(),
        bluetooth::hid_device(),
        network::wifi(),
        network::ip(),
    ]
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Every Lucide glyph a Settings row names is one the module ships (a name that is not there draws nothing).
    #[test]
    fn every_icon_a_settings_page_names_is_in_the_icon_set() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let have = root.join("../../payload/compositor/lumatv/icons/lucide");
        assert!(have.is_dir(), "the icon set is at {have:?}");
        let mut missing = Vec::new();
        let mut files = vec![root.join("src/settings")];
        while let Some(p) = files.pop() {
            if p.is_dir() {
                files.extend(std::fs::read_dir(&p).unwrap().map(|e| e.unwrap().path()));
                continue;
            }
            if p.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            for marker in [".icon(\"", ".value_icon(\"", "icon: \"", "icon(\""] {
                for (at, _) in text.match_indices(marker) {
                    let name: String = text[at + marker.len()..].chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-').collect();
                    if name.is_empty() || !text[at + marker.len() + name.len()..].starts_with('"') {
                        continue;
                    }
                    if !have.join(format!("{name}.png")).is_file() {
                        missing.push(format!("{} in {}", name, p.file_name().unwrap().to_string_lossy()));
                    }
                }
            }
        }
        // the glyphs the page of one camera takes from the Cameras app's words
        for kind in crate::cameras::words::KINDS {
            let name = crate::cameras::words::kind_icon(kind);
            if !have.join(format!("{name}.png")).is_file() {
                missing.push(format!("{name} for {kind}"));
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "icons not in the set: {missing:?}");
    }
}
