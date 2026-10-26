//! OpenNOW's standalone streamer entry (native/opennow-streamer/crates/opennow-streamer/src/main.rs at the pinned revision) with
//! one addition: a `controller-rumble` event is applied to the pad here, in the process that holds the pad, before it goes up the
//! pipe. The binary keeps upstream's name because opennow-core looks for `opennow-streamer` beside itself.
use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::thread;

use opennow_streamer_core::Engine;
use opennow_streamer_platform::{MediaRuntime, controller_rumble, create_runtime};
use opennow_streamer_protocol::{Command, error};
use serde_json::Value;

fn write_message(message: &Value) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, message)?;
    writeln!(stdout)?;
    stdout.flush()
}

fn apply_rumble(message: &Value) {
    if message["type"] != "controller-rumble" {
        return;
    }
    let slot = message["controllerId"].as_u64().unwrap_or(0) as u8;
    let low = message["lowFrequency"].as_u64().unwrap_or(0) as u16;
    let high = message["highFrequency"].as_u64().unwrap_or(0) as u16;
    let duration = message["durationMs"].as_u64().unwrap_or(0) as u32;
    if let Err(e) = controller_rumble(slot, low, high, duration) {
        eprintln!("rumble slot {slot}: {e}");
    }
}

fn run_protocol(media_runtime: MediaRuntime) -> io::Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<Value>();
    let event_writer = thread::spawn(move || {
        while let Ok(message) = event_rx.recv() {
            apply_rumble(&message);
            if write_message(&message).is_err() {
                break;
            }
        }
    });
    let mut engine = Engine::with_media_runtime(event_tx, media_runtime);

    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let command: Command = match serde_json::from_str(&line) {
            Ok(command) => command,
            Err(parse_error) => {
                write_message(&error(None, "invalid-command", parse_error.to_string()))?;
                continue;
            }
        };
        let (responses, keep_running) = engine.handle(command);
        for response in responses {
            write_message(&response)?;
        }
        if !keep_running {
            break;
        }
    }

    drop(engine);
    let _ = event_writer.join();
    Ok(())
}

fn main() -> io::Result<()> {
    if let Some(path) = std::env::var_os("OPENNOW_STREAMER_LOG_FILE") {
        let _ = opennow_streamer_protocol::log::set_log_file(&path.to_string_lossy());
    }
    let (host, media_runtime) = create_runtime().map_err(io::Error::other)?;
    let shutdown_runtime = media_runtime.clone();
    let protocol = thread::Builder::new()
        .name("opennow-protocol".to_owned())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_protocol(media_runtime)));
            shutdown_runtime.shutdown();
            result
        })?;
    host.run();
    match protocol.join() {
        Ok(Ok(result)) => result,
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
