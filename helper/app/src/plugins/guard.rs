//! A plugin of the helper's own that panics fails alone. Each runs on a thread of its own whose body runs under `catch_unwind`: the
//! helper unwinds on a panic (no `panic = "abort"`) and its locks are `parking_lot`'s, which a panic does not poison, so the panic ends
//! that body and nothing else. The plugin's switch then says what stopped it, and the body runs again after a pause that doubles with
//! each panic in a row, or at once when the owner turns the switch. What follows a switch on the manager's thread is guarded the same
//! way (`Shared::set_roster`), so the broker, the Home Assistant link and every other plugin's switch go on.

use super::session::Shared;
use crate::control::Stop;
use std::any::Any;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The longest part of a panic's message a switch shows.
const LONGEST_WORDS: usize = 200;

/// How long a body that panicked waits before it runs again.
#[derive(Clone, Copy, Debug)]
pub struct Pauses {
    pub first: Duration,
    pub longest: Duration,
    /// A run that lasted this long before it panicked starts the doubling over.
    pub steady: Duration,
}

impl Default for Pauses {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(30),
            longest: Duration::from_secs(30 * 60),
            steady: Duration::from_secs(10 * 60),
        }
    }
}

/// Starts thread `name` for the plugins `ids` (the ESPHome device's thread serves two), running `body` until it returns, which it does
/// when the helper stops. `body` is told whether it runs again after a panic, with what it holds as the panic left it.
pub fn spawn(
    name: &str,
    stack_size: usize,
    shared: Arc<Shared>,
    ids: &'static [&'static str],
    stop: Stop,
    body: impl FnMut(bool) + Send + 'static,
) -> io::Result<JoinHandle<()>> {
    {
        let shared = shared.clone();
        stop.on_stop(move || shared.wake_guards());
    }
    let label = name.to_owned();
    thread::Builder::new()
        .name(name.to_owned())
        .stack_size(stack_size)
        .spawn(move || run(&label, &shared, ids, &stop, Pauses::default(), body))
}

/// Runs `body(false)` until it returns, and `body(true)` after each panic.
pub fn run(
    name: &str,
    shared: &Shared,
    ids: &[&str],
    stop: &Stop,
    pauses: Pauses,
    mut body: impl FnMut(bool),
) {
    let (mut again, mut in_a_row) = (false, 0u32);
    loop {
        let started = Instant::now();
        let Err(panic) = panic::catch_unwind(AssertUnwindSafe(|| body(again))) else {
            return;
        };
        again = true;
        in_a_row = if started.elapsed() >= pauses.steady {
            1
        } else {
            in_a_row + 1
        };
        let pause = pauses
            .first
            .saturating_mul(1 << (in_a_row - 1).min(16))
            .min(pauses.longest);
        let what = message(&*panic);
        crate::log_error!(
            "{name}: panicked ({what}); {} starts again in {} s, or when its switch is turned",
            ids.join(" and "),
            pause.as_secs()
        );
        shared.fail(ids, &words(&what));
        if shared.wait_retry(ids, stop, pause) {
            return;
        }
        shared.recover(ids);
    }
}

/// What a panic said.
pub fn message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_owned())
}

/// What the switch of a plugin that panicked says.
pub fn words(message: &str) -> String {
    let mut short: String = message.chars().take(LONGEST_WORDS).collect();
    if short.len() < message.len() {
        short.push('…');
    }
    format!("Stopped by an internal error: {short}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;
    use crate::plugins::local::{OptIn, Spec};
    use crate::plugins::roster::{Approved, Roster};
    use crate::settings::SettingsStore;
    use serde_json::Value;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;

    const A: OptIn = OptIn {
        spec: Spec {
            id: "a",
            name: "A",
            version: "1",
            permissions: &[],
        },
        consent: "Does A.",
    };
    const B: OptIn = OptIn {
        spec: Spec {
            id: "b",
            name: "B",
            version: "1",
            permissions: &[],
        },
        consent: "Does B.",
    };

    fn shared(dir: &tempfile::TempDir) -> (Arc<Hub>, Arc<Shared>) {
        let hub = Arc::new(Hub::new(1));
        let shared = Shared::new(
            hub.clone(),
            Arc::new(SettingsStore::new(dir.path(), hub.clone())),
            Duration::from_secs(10),
            Arc::new(|| 1_800_000_000_000),
        );
        (hub, shared)
    }

    fn switch_on(shared: &Shared, plugins: &[&OptIn]) {
        let first_party = plugins
            .iter()
            .map(|p| Approved {
                id: p.spec.id.to_owned(),
                approve: p.approval(),
            })
            .collect();
        shared.set_roster(
            &Roster {
                members: Vec::new(),
                inert: Vec::new(),
                first_party,
            },
            None,
        );
    }

    fn entry(hub: &Hub, id: &str) -> Value {
        hub.get("plugins").unwrap()["firstParty"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()
            .clone()
    }

    #[test]
    fn a_plugin_that_panics_fails_alone_and_its_switch_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let (hub, shared) = shared(&dir);
        let woken_b = Arc::new(AtomicU32::new(0));
        shared.offer(&A, Arc::new(|| panic!("A cannot follow its switch")));
        {
            let woken_b = woken_b.clone();
            shared.offer(
                &B,
                Arc::new(move || {
                    woken_b.fetch_add(1, Ordering::SeqCst);
                }),
            );
        }
        // B's thread answers until the helper stops.
        let (stop_b, (ask, asked), (tell, told)) =
            (Stop::new(), mpsc::channel::<u32>(), mpsc::channel());
        let b = spawn(
            "b",
            256 * 1024,
            shared.clone(),
            &["b"],
            stop_b.clone(),
            move |_| {
                while let Ok(n) = asked.recv() {
                    tell.send(n + 1).unwrap();
                }
            },
        )
        .unwrap();

        // The manager follows both switches: A's panic does not reach it, and B is still told.
        switch_on(&shared, &[&A, &B]);
        assert_eq!(woken_b.load(Ordering::SeqCst), 2);
        assert_eq!(entry(&hub, "a")["state"], "failed");
        assert_eq!(
            entry(&hub, "a")["reason"],
            "Stopped by an internal error: A cannot follow its switch"
        );
        assert_eq!(entry(&hub, "b")["state"], "on");

        // A's thread panics; with the helper stopping it is not started again.
        let stop_a = Stop::new();
        stop_a.request();
        let runs = AtomicU32::new(0);
        run("a", &shared, &["a"], &stop_a, Pauses::default(), |_| {
            runs.fetch_add(1, Ordering::SeqCst);
            panic!("A's thread broke");
        });
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(
            entry(&hub, "a")["reason"],
            "Stopped by an internal error: A's thread broke"
        );
        assert_eq!(entry(&hub, "b")["state"], "on");
        ask.send(41).unwrap();
        assert_eq!(told.recv().unwrap(), 42, "B's thread still answers");

        drop(ask);
        stop_b.request();
        b.join().unwrap();
    }

    #[test]
    fn a_body_that_panicked_runs_again_after_the_pause_or_once_its_switch_is_turned() {
        let dir = tempfile::tempdir().unwrap();
        let (hub, shared) = shared(&dir);
        shared.offer(&A, Arc::new(|| {}));
        switch_on(&shared, &[&A]);
        let instant = Pauses {
            first: Duration::ZERO,
            ..Pauses::default()
        };
        let mut told = Vec::new();
        run("a", &shared, &["a"], &Stop::new(), instant, |again| {
            told.push(again);
            if told.len() == 1 {
                panic!("once");
            }
        });
        assert_eq!(told, [false, true], "the second run is told it runs again");
        assert_eq!(
            entry(&hub, "a")["state"],
            "on",
            "the failure went with the run"
        );

        // A long pause: the owner turning the switch starts it at once.
        let (started, starts) = mpsc::channel();
        let guard = {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let mut runs = 0;
                run(
                    "a",
                    &shared,
                    &["a"],
                    &Stop::new(),
                    Pauses::default(),
                    |_| {
                        runs += 1;
                        started.send(runs).unwrap();
                        if runs == 1 {
                            panic!("first run");
                        }
                    },
                )
            })
        };
        assert_eq!(starts.recv().unwrap(), 1);
        let topics = ["plugins".to_owned()];
        let mut after = 0;
        while entry(&hub, "a")["state"] != "failed" {
            after = hub
                .wait_events(Some(after), None, Some(&topics), Duration::from_secs(5))
                .seq;
        }
        switch_on(&shared, &[]);
        assert_eq!(starts.recv().unwrap(), 2);
        guard.join().unwrap();
        assert_eq!(entry(&hub, "a")["state"], "off");
    }
}
