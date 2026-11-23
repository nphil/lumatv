//! The on-TV transcoder: the Live TV Tuner's `?transcode=` streams made H.264 by the TV's own video decoder and encoder.
//!
//! The scheduler is the `lumaxcode` engine (one encoder job at a time, supervised child, a gate that closes within a second); this module is
//! the helper's side of it:
//!
//! | Part | What it does |
//! |---|---|
//! | [`child`] | writes the child program (`lumatv-venc xcode`, built into the helper) to the set's RAM disk while the transcoder is on |
//! | [`gate`] | closes the engine while an app other than Home and LumaTV's shell is in front, starting, or holds a video decoder |
//! | [`codec`] | the tuner's `Codec`: a stream's pictures as one `Live` job of the engine; faults in the tuner's words |
//!
//! It exists only while the Live TV Tuner plugin is on and its "Make H.264 for players that ask" setting is on (`tuner.rs`): nothing is
//! written, watched or started otherwise. A stream the transcoder cannot make (the setting off, no child program, an app has the hardware,
//! a picture shape it does not make, a failure) is served as the station sends it, byte for byte.

mod child;
mod codec;
mod gate;

use crate::ctx::Ctx;
use gate::Gate;
use lumatuner::Codec;
use lumaxcode::{Config, Engine, EngineState, Event, JobKind, Limits};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::thread::{self, JoinHandle};

/// The key the gate closes the engine by.
const GATE: &str = "app";
/// Home, which may be in front while the transcoder runs (LumaTV's own shell is the other).
const HOME: &str = "com.webos.app.home";

/// A running transcoder.
pub struct Xcode {
    engine: Engine,
    gate: Option<Gate>,
    events: Option<JoinHandle<()>>,
    /// The first digits of the hash of the child program in use.
    child: String,
}

impl Xcode {
    /// Writes the child program and starts the engine and the gate. `wake` is called when something the page says changed.
    pub fn start(ctx: &Ctx, wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        let path = child::install(Path::new(child::DIR), child::IMAGE)?;
        let engine = Engine::new(Config { child: path, limits: Limits::default() })?;
        let events = {
            let events = engine.subscribe();
            thread::Builder::new().name("xcode-events".to_owned()).stack_size(128 * 1024).spawn(move || follow(&events, &wake))?
        };
        let ours = vec![HOME.to_owned(), ctx.config.brand_app_id.clone().unwrap_or_else(|| "org.nphil.lumatv".to_owned())];
        let (holder, watcher) = (engine.clone(), engine.clone());
        let gate = Gate::start(
            &ctx.config.luna.send,
            ours,
            Box::new(move |why| holder.hold(GATE, why.as_deref())),
            Box::new(move || watcher.snapshot().jobs.iter().any(|job| job.state != JobKind::Ended)),
        );
        let gate = match gate {
            Ok(gate) => gate,
            Err(e) => {
                engine.shutdown();
                child::remove(Path::new(child::DIR));
                return Err(e);
            }
        };
        let id = child::id(child::IMAGE);
        crate::log_info!("xcode: started, child program {id}");
        Ok(Self { engine, gate: Some(gate), events: Some(events), child: id })
    }

    /// The tuner's converter over this engine.
    pub fn codec(&self) -> Option<Arc<dyn Codec>> {
        let look = self.gate.as_ref()?.looker();
        Some(Arc::new(codec::Converter::new(self.engine.clone(), look)))
    }

    /// Where it stands, in a line for the plugin's page.
    pub fn status(&self) -> String {
        let snapshot = self.engine.snapshot();
        let live = snapshot.jobs.iter().filter(|job| job.state != JobKind::Ended).count();
        match (snapshot.state, live) {
            (EngineState::Wedged, _) => "Stopped until the helper restarts: the video hardware did not answer".to_owned(),
            (EngineState::Cooling, _) => "Resting: the conversion failed repeatedly".to_owned(),
            (EngineState::Held, _) => format!("Paused: {}", snapshot.holds.first().map_or("the video hardware is in use", |h| h.reason.as_str())),
            (_, 0) => format!("Ready (program {})", self.child),
            (_, n) => format!("Converting {n} {}", if n == 1 { "stream" } else { "streams" }),
        }
    }
}

impl Drop for Xcode {
    fn drop(&mut self) {
        // the watch first, so that it cannot hold or release an engine that is being stopped; then the hardware, within a second
        self.gate = None;
        self.engine.shutdown();
        if let Some(events) = self.events.take() {
            let _ = events.join();
        }
        child::remove(Path::new(child::DIR));
        crate::log_info!("xcode: stopped");
    }
}

/// Logs what the engine does, and wakes the page when it matters to it.
fn follow(events: &Receiver<Event>, wake: &impl Fn()) {
    for event in events {
        match &event {
            Event::Started { job, restarts, setup_ms } => crate::log_info!("xcode: job {job} running after {setup_ms} ms ({restarts} restarts)"),
            Event::Paused { job, reason, stopped_ms } => crate::log_info!("xcode: job {job} paused: {reason}; the hardware was free {stopped_ms} ms after the request"),
            Event::Resumed { job } => crate::log_info!("xcode: job {job} resumed"),
            Event::Dropped { job, units } => crate::log_warn!("xcode: job {job} dropped {units} units: the reader is too slow"),
            Event::Ended { job, reason } => crate::log_info!("xcode: job {job} ended: {reason}"),
            Event::Wedged => crate::log_warn!("xcode: a call into the video hardware did not return; no conversion starts until the helper restarts"),
            Event::Bytes { .. } | Event::Hold { .. } => continue,
        }
        wake();
    }
}
