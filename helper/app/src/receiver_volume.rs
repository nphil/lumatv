//! The receiver's volume as the display's number drives it: the target the TV shows is sent to the Denon as an absolute level.
//!
//! The volume bar counts a held key by itself (`panels/VolumeOsd.qml`) and posts the level it shows. This thread sends that level
//! to the receiver (`MVxx` or `MVxx5` over its web port), at most one command per `receiver.volumeGapMs` with the newest level
//! only: a hold's intermediate levels are skipped, never queued. The receiver then follows the display instead of the other way round.
//!
//! LG's audio service passes every key repeat on over HDMI-CEC at the same time, and the Denon takes those steps slower than the
//! keys come (and some arrive after the key is released). An absolute level overrides them, but one still in flight can land after
//! the last command: when the level has been quiet for `receiver.volumeSettleMs` the receiver is read once, and a level that
//! differs is sent again (at most `receiver.volumeRetries` times).
//!
//! The level is held to the receiver's own limit (`volumeMax` in the `receiver` topic, from its `MVMAX`) when it is known.

use crate::ctx::Ctx;
use crate::receiver::TOPIC;
use lumatv_denon::{Client, volume_command};
use parking_lot::{Condvar, Mutex};
use std::io;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The panel's scale ends here; the receiver steps in halves.
pub const MAX_LEVEL: f64 = 98.0;

/// A level on the receiver's scale, held to `0..=98` and rounded to its half steps the way `volume_command` does.
pub fn quantize(level: f64) -> Option<f64> {
    level
        .is_finite()
        .then(|| ((level.clamp(0.0, MAX_LEVEL) * 2.0 + 0.5).floor() / 2.0).min(MAX_LEVEL))
}

#[derive(Default)]
struct State {
    /// The newest level asked for and not yet sent.
    pending: Option<f64>,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// The handle. Dropping it ends the thread.
pub struct VolumeDriver {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl VolumeDriver {
    pub fn spawn(ctx: &Ctx) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let thread = {
            let (shared, ctx) = (shared.clone(), ctx.clone());
            thread::Builder::new()
                .name("receiver-volume".to_owned())
                .stack_size(256 * 1024)
                .spawn(move || run(&ctx, &shared))?
        };
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Asks for `level` (0 to 98): the newest ask replaces one that has not been sent yet.
    pub fn set(&self, level: f64) {
        self.shared.state.lock().pending = Some(level);
        self.shared.changed.notify_all();
    }
}

impl Drop for VolumeDriver {
    fn drop(&mut self) {
        self.shared.state.lock().stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The last level sent and what is left to do about it.
struct Sent {
    level: f64,
    at: Instant,
    /// Corrections already made for this level.
    fixes: u32,
}

enum Work {
    Send(f64, u32),
    Verify(f64, u32),
}

fn run(ctx: &Ctx, shared: &Shared) {
    let tuning = &ctx.config.receiver;
    let gap = Duration::from_millis(tuning.volume_gap_ms);
    let settle = Duration::from_millis(tuning.volume_settle_ms);
    let mut sent: Option<Sent> = None;
    let mut client: Option<((String, u16), Client)> = None;
    let mut failing = false;
    loop {
        let work = {
            let mut state = shared.state.lock();
            loop {
                if state.stop {
                    return;
                }
                let now = Instant::now();
                if let Some(level) = state.pending {
                    let ready = sent.as_ref().map_or(now, |s| s.at + gap);
                    if now >= ready {
                        state.pending = None;
                        break Work::Send(level, 0);
                    }
                    shared.changed.wait_until(&mut state, ready);
                } else if let Some(s) = &sent {
                    let due = s.at + settle;
                    if now >= due {
                        break Work::Verify(s.level, s.fixes);
                    }
                    shared.changed.wait_until(&mut state, due);
                } else {
                    shared.changed.wait(&mut state);
                }
            }
        };
        let settings = ctx.settings.receiver();
        let target = (settings.host.clone(), settings.port);
        if client.as_ref().map(|(t, _)| t) != Some(&target) {
            let c = Client::new(settings.host.clone(), settings.port).with_timeouts(
                Duration::from_millis(tuning.connect_timeout_ms),
                Duration::from_millis(tuning.request_timeout_ms),
            );
            client = Some((target, c));
        }
        let Some((_, client)) = &client else { continue };
        if !settings.enabled {
            sent = None;
            continue;
        }
        let (level, fixes) = match work {
            Work::Send(level, fixes) => (limited(ctx, level), fixes),
            Work::Verify(level, fixes) => {
                // One read after the level has been quiet: a different level means a CEC step landed after the command.
                let off = match client.status() {
                    Ok(status) => status
                        .volume_absolute()
                        .is_some_and(|v| (v - level).abs() >= 0.25),
                    Err(_) => false,
                };
                if !off || fixes >= tuning.volume_retries {
                    sent = None;
                    continue;
                }
                (level, fixes + 1)
            }
        };
        let Some(command) = volume_command(level) else {
            sent = None;
            continue;
        };
        match client.send_command(&command) {
            Ok(()) => {
                if failing {
                    crate::log_info!("receiver: volume commands reach {}", client.host());
                }
                failing = false;
                sent = Some(Sent {
                    level,
                    at: Instant::now(),
                    fixes,
                });
            }
            Err(e) => {
                if !failing {
                    crate::log_warn!("receiver: volume command to {}: {e}", client.host());
                }
                failing = true;
                sent = None;
            }
        }
    }
}

/// `level` held to the receiver's own limit, if the topic says one, and to its half steps.
fn limited(ctx: &Ctx, level: f64) -> f64 {
    let limit = ctx
        .hub
        .get(TOPIC)
        .and_then(|doc| doc.get("volumeMax").and_then(serde_json::Value::as_f64))
        .filter(|m| *m > 0.0);
    let level = limit.map_or(level, |m| level.min(m));
    quantize(level).unwrap_or(level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_held_to_the_scale_and_its_half_steps() {
        assert_eq!(quantize(46.5), Some(46.5));
        assert_eq!(quantize(46.74), Some(46.5));
        assert_eq!(quantize(46.75), Some(47.0));
        assert_eq!(quantize(-3.0), Some(0.0));
        assert_eq!(quantize(120.0), Some(98.0));
        assert_eq!(quantize(f64::NAN), None);
    }
}
