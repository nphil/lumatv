//! Cooperative shutdown for every thread of the helper.

use parking_lot::{Condvar, Mutex};
use std::sync::Arc;
use std::time::Duration;

type Hook = Box<dyn Fn() + Send + Sync>;

struct Inner {
    stopped: Mutex<bool>,
    changed: Condvar,
    hooks: Mutex<Vec<Hook>>,
}

/// A stop request every thread can wait on. Sleeping through it ends the moment a stop is requested, and hooks (wake a
/// runner's channel, close the hub, unblock the accept loop, kill a child process) run once, on the requesting thread.
#[derive(Clone)]
pub struct Stop {
    inner: Arc<Inner>,
}

impl Default for Stop {
    fn default() -> Self {
        Self::new()
    }
}

impl Stop {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                stopped: Mutex::new(false),
                changed: Condvar::new(),
                hooks: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn is_set(&self) -> bool {
        *self.inner.stopped.lock()
    }

    /// Requests the stop (idempotent) and runs the hooks that have been registered.
    pub fn request(&self) {
        {
            let mut stopped = self.inner.stopped.lock();
            if *stopped {
                return;
            }
            *stopped = true;
        }
        self.inner.changed.notify_all();
        let hooks = std::mem::take(&mut *self.inner.hooks.lock());
        for hook in hooks {
            hook();
        }
    }

    /// Runs `hook` when the stop is requested, or at once if it already was.
    pub fn on_stop(&self, hook: impl Fn() + Send + Sync + 'static) {
        {
            let mut hooks = self.inner.hooks.lock();
            if !self.is_set() {
                hooks.push(Box::new(hook));
                return;
            }
        }
        hook();
    }

    /// Sleeps up to `duration`; returns `true` as soon as a stop is requested (and at once if it already was).
    pub fn sleep(&self, duration: Duration) -> bool {
        let mut stopped = self.inner.stopped.lock();
        if !*stopped {
            self.inner
                .changed
                .wait_while_for(&mut stopped, |s| !*s, duration);
        }
        *stopped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    #[test]
    fn a_sleep_ends_when_the_stop_is_requested() {
        let stop = Stop::new();
        let sleeper = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                let stopped = stop.sleep(Duration::from_secs(30));
                (stopped, started.elapsed())
            })
        };
        std::thread::sleep(Duration::from_millis(30));
        stop.request();
        let (stopped, elapsed) = sleeper.join().unwrap();
        assert!(stopped);
        assert!(elapsed < Duration::from_secs(5));
    }

    #[test]
    fn a_sleep_that_runs_out_reports_no_stop() {
        let stop = Stop::new();
        assert!(!stop.sleep(Duration::from_millis(10)));
        assert!(!stop.is_set());
    }

    #[test]
    fn after_the_stop_every_sleep_returns_at_once() {
        let stop = Stop::new();
        stop.request();
        let started = Instant::now();
        assert!(stop.sleep(Duration::from_secs(30)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn hooks_run_once_whether_registered_before_or_after() {
        let stop = Stop::new();
        let count = Arc::new(AtomicUsize::new(0));
        for _ in 0..2 {
            let count = count.clone();
            stop.on_stop(move || {
                count.fetch_add(1, Ordering::SeqCst);
            });
        }
        stop.request();
        stop.request();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        let late = count.clone();
        stop.on_stop(move || {
            late.fetch_add(10, Ordering::SeqCst);
        });
        assert_eq!(count.load(Ordering::SeqCst), 12);
    }
}
