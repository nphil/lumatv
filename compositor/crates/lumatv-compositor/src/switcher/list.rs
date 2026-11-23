//! The apps the task manager lists, and the processes behind them.
//!
//! The compositor's windows are not the apps that run: a client releases its buffer when it leaves the front, and an app with nothing
//! mapped is running all the same. So the list is the windows with a picture (always) plus what the app manager says runs (`running`,
//! asked when the task manager opens and again, [`ASK_AGAIN`] after a life event, while it is open) that is an app which had a window
//! while the compositor was up (`seen`): the services the app manager runs for itself never had one and stay out. With no answer (the
//! bridge is down, or it has not come yet) the windows alone are listed.
//!
//! The processes come from the same answer, each with its start time, so that ending one ([`State::kill_app`]) never ends a process that
//! took the number later. An app with a stopped process (`SIGSTOP`, a debugger) is *hung*: it cannot act on a request to quit, so Quit
//! ends it.
//!
//! An app's last frame is kept when it leaves the front ([`State::keep_app_thumb`]): it releases its buffer then.

use std::cmp::Reverse;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{App, Picture, input_app, own_app};
use crate::luna::{Outbound, ToBridge};
use crate::State;
use crate::surface::SurfaceId;
use crate::taskmgr::procs::Proc;
use crate::taskmgr::running::{self, RunningApp};
use crate::window::{Window, WindowId, WindowType};

/// A life event is answered by one query this long after it, so that the burst of events of one launch is one query.
const ASK_AGAIN: Duration = Duration::from_millis(250);
/// A query with no answer for this long is asked again.
const ASK_LOST: Duration = Duration::from_secs(3);
/// The first topics of an opening wait this long for the app manager.
pub(super) const ASK_WAIT: Duration = Duration::from_millis(120);
/// Apps remembered as having had a window.
const SEEN_KEPT: usize = 64;

/// An app that had a window.
struct Seen {
    id: String,
    title: String,
    /// Its picture is on the video plane.
    video: bool,
}

#[derive(Default)]
pub(super) struct Roster {
    /// Newest first.
    seen: Vec<Seen>,
    /// What the app manager said runs, each with its processes as they were then.
    listed: Vec<(RunningApp, Vec<Proc>)>,
    /// It has answered once: `listed` says something.
    answered: bool,
    /// The query in flight, and when it was sent.
    token: Option<u64>,
    asked: Option<Instant>,
    /// Ask again then.
    again: Option<Instant>,
    /// The first topics of this opening wait until then, or for the answer.
    waiting_until: Option<Instant>,
    /// Apps with a stopped process.
    pub(super) hung: Vec<String>,
    /// Changes whenever `listed` or `hung` does: what the topics' signature takes in.
    pub(super) version: u64,
}

impl Roster {
    pub(super) fn begin(&mut self, now: Instant) {
        self.waiting_until = Some(now + ASK_WAIT);
        self.again = None;
    }

    pub(super) fn waiting(&self) -> bool {
        self.waiting_until.is_some()
    }

    pub(super) fn is_hung(&self, id: &str) -> bool {
        self.hung.iter().any(|h| h == id)
    }

    fn runs(&self, id: &str) -> bool {
        self.answered && self.listed.iter().any(|(a, _)| a.id == id)
    }

    /// Looks at the processes again. Whether the set of hung apps changed.
    pub(super) fn refresh_hung(&mut self) -> bool {
        let hung: Vec<String> = self.listed.iter().filter(|(_, ps)| ps.iter().any(Proc::stopped)).map(|(a, _)| a.id.clone()).collect();
        if hung == self.hung {
            return false;
        }
        self.hung = hung;
        self.version += 1;
        true
    }
}

/// Whether `w` can be an app's card: not Home or LumaTV's own, not a member of a surface group, and in front at some time.
fn listable(w: &Window) -> bool {
    let id = w.app_id();
    w.kind == WindowType::Card && !w.in_group && w.map_serial != 0 && w.front != 0 && !id.is_empty() && !own_app(id)
}

impl State {
    /// Whether an app's picture is on the video plane (a hole in its window, an exported context, or an input), which cannot be sampled.
    fn on_video_plane(&self, id: &str, surface: SurfaceId) -> bool {
        input_app(id)
            || self.scene.holes.iter().any(|h| h.surface == surface)
            || self.foreign.exported.values().any(|e| e.surface == surface && !e.contexts.is_empty())
    }

    /// The running apps, newest first (see the module).
    pub fn switcher_apps(&self) -> Vec<App> {
        let list = &self.switcher.list;
        let mut found: Vec<(u64, App)> = Vec::new();
        for w in self.scene.windows().filter(|w| listable(w)) {
            let id = w.app_id();
            let mapped = self.surfaces.get(w.surface).filter(|s| s.is_mapped());
            // a window with nothing mapped is an app that runs only if the app manager says so
            if mapped.is_none() && !list.runs(id) {
                continue;
            }
            let size = mapped.map_or((0, 0), |s| s.size());
            // an app on the video plane has a kept picture only if its hook drew one (`hook.rs`)
            let picture = if self.on_video_plane(id, w.surface) {
                if self.output.has_thumb(id) { Picture::Thumb } else { Picture::Video }
            } else if mapped.is_some() && size.0 > 0 && size.1 > 0 {
                Picture::Live
            } else if self.output.has_thumb(id) {
                Picture::Thumb
            } else {
                Picture::None
            };
            let title = if w.title.is_empty() { id.to_string() } else { w.title.clone() };
            let app = App { id: id.to_string(), title, window: Some(w.id), surface: Some(w.surface), size, picture };
            match found.iter_mut().find(|(_, a)| a.id == app.id) {
                Some(old) if old.0 >= w.front => {}
                Some(old) => *old = (w.front, app),
                None => found.push((w.front, app)),
            }
        }
        if list.answered {
            // what the app manager runs that has no window left here: it was in front once, and is the oldest of the list
            for (r, _) in &list.listed {
                if own_app(&r.id) || found.iter().any(|(_, a)| a.id == r.id) {
                    continue;
                }
                let Some(s) = list.seen.iter().find(|s| s.id == r.id) else { continue };
                let picture = if self.output.has_thumb(&r.id) {
                    Picture::Thumb
                } else if s.video {
                    Picture::Video
                } else {
                    Picture::None
                };
                found.push((0, App { id: r.id.clone(), title: s.title.clone(), window: None, surface: None, size: (0, 0), picture }));
            }
        }
        found.sort_by_key(|a| Reverse(a.0));
        found.into_iter().map(|(_, a)| a).collect()
    }

    /// Remembers the apps that have a window now, newest first.
    pub(super) fn switcher_note_seen(&mut self) {
        let mut now: Vec<(u64, Seen)> = self
            .scene
            .windows()
            .filter(|w| listable(w))
            .map(|w| {
                let id = w.app_id();
                let title = if w.title.is_empty() { id.to_string() } else { w.title.clone() };
                (w.front, Seen { id: id.to_string(), title, video: self.on_video_plane(id, w.surface) })
            })
            .collect();
        now.sort_by_key(|(front, _)| *front);
        let seen = &mut self.switcher.list.seen;
        for (_, s) in now {
            seen.retain(|x| x.id != s.id);
            seen.insert(0, s);
        }
        seen.truncate(SEEN_KEPT);
    }

    /// Asks the app manager what runs. One query is in flight at a time; a life event that comes meanwhile asks again after the answer.
    pub(super) fn switcher_ask(&mut self, now: Instant) {
        let list = &mut self.switcher.list;
        if list.token.is_some() && list.asked.is_some_and(|t| now.saturating_duration_since(t) < ASK_LOST) {
            list.again = Some(now + ASK_AGAIN);
            return;
        }
        list.again = None;
        let token = self.luna_token();
        self.switcher.list.token = Some(token);
        self.switcher.list.asked = Some(now);
        self.luna.send(ToBridge::Outbound(Outbound { token, uri: running::URI.to_string(), payload: json!({}), subscribe: false }));
    }

    /// The bridge's answer to a query. Returns whether it was ours.
    pub(crate) fn switcher_sam_answer(&mut self, token: u64, payload: &Value) -> bool {
        if self.switcher.list.token != Some(token) {
            return false;
        }
        self.switcher.list.token = None;
        let Some(apps) = running::parse(payload) else {
            self.journal(format!("switcher: the app manager's list could not be read: {payload}"));
            return true;
        };
        let list = &mut self.switcher.list;
        list.listed = apps
            .into_iter()
            .map(|a| {
                let procs = a.pids.iter().filter_map(|p| Proc::identify(*p)).collect();
                (a, procs)
            })
            .collect();
        list.answered = true;
        list.waiting_until = None;
        list.refresh_hung();
        list.version += 1;
        self.switcher_note_seen();
        // the pictures kept of apps that are gone are dropped now
        let ids: Vec<String> = self.switcher_apps().into_iter().map(|a| a.id).collect();
        self.output.retain_thumbs(&ids);
        self.shell_sync_topics();
        true
    }

    /// An app launched or ended (the app manager's life events): while the task manager is open, the list is asked for again.
    pub(crate) fn switcher_life_event(&mut self, payload: &Value) {
        if self.switcher.active() && running::changes_the_list(payload) {
            self.switcher.list.again = Some(Instant::now() + ASK_AGAIN);
        }
    }

    /// What each frame owes the list: the query a life event asked for, the end of the first topics' wait. Whether the topics must be redone.
    pub(super) fn switcher_list_tick(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.switcher.list.waiting_until.is_some_and(|t| now >= t) {
            self.switcher.list.waiting_until = None;
            changed = true;
        }
        if self.switcher.list.again.is_some_and(|t| now >= t) {
            self.switcher_ask(now);
        }
        changed
    }

    /// Ends the processes of an app (`SIGKILL`, each one only if it is still the process it was when it was listed). Never LumaTV's own, the
    /// compositor itself, pid 1, nor a process another listed app shares. Whether any was ended.
    pub(super) fn kill_app(&mut self, id: &str) -> bool {
        let list = &self.switcher.list;
        let me = std::process::id() as i32;
        let Some((_, procs)) = list.listed.iter().find(|(a, _)| a.id == id) else { return false };
        let shared = |p: &Proc| list.listed.iter().any(|(a, ps)| a.id != id && ps.iter().any(|q| q.pid == p.pid));
        let victims: Vec<Proc> = procs.iter().filter(|p| p.pid > 1 && p.pid != me && !shared(p)).copied().collect();
        let ended = victims.iter().filter(|p| p.kill()).count();
        self.journal(format!("switcher: {id}: ended {ended} of {} process(es)", victims.len()));
        ended > 0
    }

    /// An app's window is about to lose its picture (`freeze_app`: it left the front, or it is going): its last frame is kept for the
    /// task manager. Nothing of an app on the video plane, which has no frame to keep here (a picture kept before it came there goes). An
    /// app with a hook is asked for its own picture (`hook.rs`), which takes the place of this one when it comes.
    pub(crate) fn keep_app_thumb(&mut self, window: WindowId) {
        let Some(w) = self.scene.window(window).filter(|w| listable(w)) else { return };
        let (id, surface) = (w.app_id().to_string(), w.surface);
        self.switcher_note_seen();
        self.hook_snapshot(&id);
        if self.on_video_plane(&id, surface) {
            self.output.drop_thumb(&id);
            return;
        }
        let Some(buffer) = self.surfaces.get(surface).and_then(|s| s.current.buffer.clone()) else { return };
        self.output.keep_thumb(&id, &buffer);
    }
}
