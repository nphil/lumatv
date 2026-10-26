//! `home-assistant`: LumaShow's Home Assistant plugin as a program for the TV. It does nothing until the broker speaks, and stops
//! when it is told to. This is the Show's `plugins/home-assistant/src/main.rs` (its library is `plugin-home-assistant`).

use lumashow_sdk::{Config, shutdown_on_signals, signalled};

fn main() {
    shutdown_on_signals();
    let config = Config::default();
    if let Err(why) = plugin_home_assistant::run(&config, &signalled) {
        eprintln!("home-assistant: {why}");
        std::process::exit(1);
    }
}
