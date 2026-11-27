//! Offline check of Prime's player against the ad filter: `cargo run -p lumatv-privacy --example check_prime_player -- player.js [more.js ...]`.
//! Reads each local file and runs the same `transform` the helper serves with. Nothing is sent anywhere and the transformed player is
//! neither written nor printed. Exit 0 when every ad patch fits every file, 1 when one does not, 2 on a file that cannot be read.
//! Reports that the player has no place for are listed, but do not fail the check: the helper serves the player without them.
use lumatv_privacy::noads::transform;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut files = std::env::args().skip(1).peekable();
    if files.peek().is_none() {
        eprintln!("usage: check_prime_player <player.js>...");
        return ExitCode::from(2);
    }
    let mut code = 0;
    for file in files {
        let source = match std::fs::read_to_string(&file) {
            Ok(source) => source,
            Err(e) => {
                eprintln!("{file}: cannot be read: {e}");
                return ExitCode::from(2);
            }
        };
        match transform(&source) {
            Ok(done) => {
                println!("{file}: compatible ({} bytes)", source.len());
                for name in done.missed {
                    println!("  Unmatched report rule: {name}");
                }
            }
            Err(reason) => {
                println!("{file}: INCOMPATIBLE, {reason}");
                code = 1;
            }
        }
    }
    ExitCode::from(code)
}
