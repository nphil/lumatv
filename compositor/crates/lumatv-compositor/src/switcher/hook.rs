//! An app's hook (`taskmgr/hook.rs`) in the task manager's flows.
//!
//! Quit: where the app serves a hook and has not hung, it is told to end its session and has [`QUIT_GRACE`] to be gone; if it is still
//! there then (or the hook did not take the request) the ordinary close runs, and its own four seconds count from there.
//!
//! Picture: when an app leaves the front, and when the task manager opens over the front app, the app is asked for its last UI frame, which
//! becomes its card's kept picture; a black one drops the picture, and the card shows the icon. The compositor's capture tap is not used: a
//! video on the hardware layer is not in what the app draws, so a card for it stays an icon card.
//!
//! The socket is spoken to on a thread of its own per ask, with the answer ([`ANSWER`]) bounded; the frame polls the channel. An app with
//! no hook costs one `stat`.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

use serde_json::json;

use super::own_app;
use crate::State;
use crate::taskmgr::hook::{self, CARD, QUIT_GRACE, Snapshot};

pub(super) struct Hooks {
    /// The directory the apps' own directories are in.
    root: PathBuf,
    quits: Vec<Quit>,
    snaps: Vec<Snap>,
}

impl Default for Hooks {
    fn default() -> Self {
        Hooks { root: PathBuf::from("/tmp"), quits: Vec::new(), snaps: Vec::new() }
    }
}

/// An app told to quit: when its grace ends, and the hook's answer while it is awaited.
struct Quit {
    id: String,
    until: Instant,
    answer: Option<Receiver<bool>>,
}

/// A picture asked of an app.
struct Snap {
    id: String,
    answer: Receiver<Snapshot>,
}

impl State {
    /// Where the apps' hooks are looked for (`/tmp`), for the tests to point at a directory of their own.
    pub fn hook_root_set(&mut self, root: PathBuf) {
        self.switcher.hooks.root = root;
    }

    /// Tells the app to quit through its hook. `false` when it has none (or no thread could be made): the ordinary close is the caller's.
    pub(super) fn hook_quit(&mut self, id: &str, now: Instant) -> bool {
        let hooks = &mut self.switcher.hooks;
        if !hook::present(&hooks.root, id) {
            return false;
        }
        let (tx, rx) = mpsc::channel();
        let (root, app) = (hooks.root.clone(), id.to_string());
        if thread::Builder::new().name("hook-quit".into()).spawn(move || {
            let _ = tx.send(hook::quit(&root, &app));
        }).is_err() {
            return false;
        }
        hooks.quits.retain(|q| q.id != id);
        hooks.quits.push(Quit { id: id.to_string(), until: now + QUIT_GRACE, answer: Some(rx) });
        self.journal(format!("switcher: {id}: told to quit through its hook, {} s to go", QUIT_GRACE.as_secs()));
        true
    }

    /// The app is ended another way (it hung, or did not quit in time): its hook is not waited for.
    pub(super) fn hook_quit_cancel(&mut self, id: &str) {
        self.switcher.hooks.quits.retain(|q| q.id != id);
    }

    /// Asks the app for its last frame, if it has a hook and none is asked of it already.
    pub(super) fn hook_snapshot(&mut self, id: &str) {
        let hooks = &mut self.switcher.hooks;
        if own_app(id) || hooks.snaps.iter().any(|s| s.id == id) || !hook::present(&hooks.root, id) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let (root, app) = (hooks.root.clone(), id.to_string());
        if thread::Builder::new().name("hook-snapshot".into()).spawn(move || {
            let _ = tx.send(hook::snapshot(&root, &app));
        }).is_ok() {
            hooks.snaps.push(Snap { id: id.to_string(), answer: rx });
        }
    }

    /// Every frame, whether the task manager is open or not: takes the answers that came, and closes the apps whose grace ended.
    pub(super) fn hook_tick(&mut self, now: Instant) {
        if self.switcher.hooks.snaps.is_empty() && self.switcher.hooks.quits.is_empty() {
            return;
        }
        let mut arrived = Vec::new();
        self.switcher.hooks.snaps.retain(|s| match s.answer.try_recv() {
            Ok(snapshot) => {
                arrived.push((s.id.clone(), snapshot));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => false,
        });
        let mut changed = false;
        for (id, snapshot) in arrived {
            changed |= match snapshot {
                Snapshot::Pixels(rgba) => self.output.keep_thumb_rgba(&id, CARD.0, CARD.1, &rgba),
                Snapshot::Black => {
                    self.output.drop_thumb(&id);
                    true
                }
                Snapshot::Failed => false,
            };
        }
        if changed {
            self.switcher.list.version += 1;
            self.shell_sync_topics();
        }
        let mut close = Vec::new();
        self.switcher.hooks.quits.retain_mut(|q| {
            match q.answer.as_ref().map(Receiver::try_recv) {
                Some(Ok(true)) => q.answer = None,
                // the hook did not take it: nothing is gained by waiting
                Some(Ok(false) | Err(TryRecvError::Disconnected)) => {
                    close.push(q.id.clone());
                    return false;
                }
                Some(Err(TryRecvError::Empty)) | None => {}
            }
            if now < q.until {
                return true;
            }
            close.push(q.id.clone());
            false
        });
        if close.is_empty() {
            return;
        }
        let running = self.switcher_apps();
        for id in close.into_iter().filter(|id| running.iter().any(|a| a.id == *id)) {
            self.journal(format!("switcher: {id}: still there, closing it"));
            if let Some(q) = self.switcher.quitting.iter_mut().find(|(a, _)| *a == id) {
                q.1 = now;
            }
            self.close_through_app_manager(&id);
        }
    }

    /// The ordinary close: the window is asked to close and the app manager is told to close the app.
    pub(super) fn close_through_app_manager(&mut self, id: &str) {
        self.close_app(id);
        self.luna_call("luna://com.webos.applicationManager/closeByAppId".into(), json!({"id": id, "reason": "windowPolicy"}));
    }
}
