//! Reads a receiver's Lite status once and prints it: `cargo run -p lumatv-denon --example status -- 192.168.1.93 [port]`.

use lumatv_denon::{Client, DEFAULT_PORT};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(host) = args.next() else {
        eprintln!("usage: status HOST [PORT]");
        std::process::exit(2);
    };
    let port = args
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    match Client::new(host, port).status() {
        Ok(status) => println!("{status:#?}\nvolume on the panel (absolute): {:?}", status.volume_absolute()),
        Err(e) => {
            eprintln!("{} ({e})", e.code());
            std::process::exit(1);
        }
    }
}
