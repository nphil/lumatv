//! The pages of Settings, in the rail's order and then the pages that hang under one of its sections. Each is a function from what the
//! shell knows to rows (`settings/Page*.qml` of the QML shell, row for row).

use super::page::Page;

pub mod access;
pub mod ambient;
pub mod bluetooth;
pub mod capture;
pub mod home;
pub mod language;
pub mod owner;
pub mod picture;
pub mod privacy;
pub mod profiles;
pub mod receiver;
pub mod saver;
pub mod sound;
pub mod support;
pub mod system;
pub mod util;

/// Every page: the rail's sections in the rail's order (Privacy and Profiles first, then LG's order), then the pages that hang under one.
pub fn all() -> Vec<Page> {
    vec![
        privacy::page(),
        profiles::page(),
        picture::page(),
        sound::page(),
        ambient::page(),
        home::page(),
        system::page(),
        privacy::held(),
        profiles::person(),
        profiles::avatars(),
        home::popups(),
        saver::page(),
        system::clock(),
        system::timer_on(),
        system::timer_off(),
        language::page(),
        access::safety(),
        access::audio_guide(),
        access::audio_options(),
        access::emergency(),
        access::captions(),
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
        bluetooth::audio(),
        bluetooth::hid(),
        bluetooth::audio_device(),
        bluetooth::hid_device(),
    ]
}
