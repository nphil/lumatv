//! The owner's "Refresh libraries": GeForce NOW re-checks the stores linked to the account (`account.connections.sync`), the engine
//! waits for it, and then says its part is over so that the app fetches the library again. The core accepts a sync only when the
//! store's service answers 202, and calls it `refreshing_library` once the catalog it holds is out of date; the library the app fetches
//! next is the new one, and the engine tells the core so when the last page is out (`libraryRefreshed`).
//!
//! Nothing here is a guess about the outcome: a store that never confirms is reported as unconfirmed, with the app's library fetch
//! still done, so the owner sees what GeForce NOW has now.

use super::{Adapter, Purpose};
use lumagame_core::ipc::{ErrorCode, Event};
use serde_json::{Value, json};

/// The core gives up on one sync after 120 s; the engine waits a little longer for its answer, then reports what it has.
const WAIT_MS: u64 = 135_000;
const POLL_MIN_MS: u64 = 1_000;
const POLL_MAX_MS: u64 = 10_000;
const POLL_DEFAULT_MS: u64 = 2_000;

pub(super) struct Refresh {
    seq: Option<u64>,
    deadline: u64,
    /// The list of linked stores is still to come.
    listing: bool,
    watches: Vec<Watch>,
    notes: Vec<String>,
}

struct Watch {
    provider: String,
    label: String,
    /// The core's id for the sync, once it has started one.
    operation: Option<String>,
    /// When the next status check is due, by the engine's clock; 0 while none is wanted.
    due: u64,
    polling: bool,
    done: bool,
}

/// What a core error code means to the owner.
pub(super) fn classify(code: &str) -> Option<(ErrorCode, &'static str)> {
    match code {
        "network_error" => Some((ErrorCode::Network, "GeForce NOW can't be reached. Check the network.")),
        "stale_account" | "http_unauthorized" | "authentication_required" => {
            Some((ErrorCode::SignIn, "GeForce NOW needs you to sign in again."))
        }
        _ => None,
    }
}

impl Adapter {
    pub(super) fn begin_refresh(&mut self, seq: u64) {
        if self.target.is_none() {
            return self.error(Some(seq), ErrorCode::SignIn, "GeForce NOW needs you to sign in again.");
        }
        // A second press while one runs is the same refresh: the app is already waiting for its end.
        if self.refresh.is_some() {
            return;
        }
        self.refresh = Some(Refresh { seq: Some(seq), deadline: self.now + WAIT_MS, listing: true, watches: Vec::new(), notes: Vec::new() });
        self.step("Checking your linked stores");
        self.enqueue("account.connections.list", json!({}), Purpose::SyncList, Some(seq));
    }

    fn step(&mut self, text: &str) {
        let seq = self.refresh.as_ref().and_then(|r| r.seq);
        self.emit(seq, Event::Refresh { step: Some(text.to_owned()), note: None });
    }

    /// The linked stores: each one GeForce NOW can sync and that is in a state to be synced gets a sync.
    pub(super) fn refresh_listed(&mut self, r: &Value) {
        let Some(refresh) = &mut self.refresh else { return };
        refresh.listing = false;
        let mut starts = Vec::new();
        for account in r["accounts"].as_array().map(Vec::as_slice).unwrap_or_default() {
            if account["isConnected"] != true || account["supportsSync"] != true {
                continue;
            }
            let Some(provider) = account["provider"].as_str().filter(|p| !p.is_empty()) else { continue };
            let label = account["label"].as_str().filter(|l| !l.is_empty()).unwrap_or(provider).to_owned();
            if account["status"].as_str() == Some("expired") {
                refresh.notes.push(format!("{label} needs to be linked again in GeForce NOW."));
                continue;
            }
            starts.push((provider.to_owned(), label));
        }
        if starts.is_empty() {
            return self.refresh_check();
        }
        let labels = starts.iter().map(|(_, label)| label.as_str()).collect::<Vec<_>>().join(", ");
        for (provider, label) in starts {
            refresh.watches.push(Watch { provider, label, operation: None, due: 0, polling: false, done: false });
        }
        let providers: Vec<String> = refresh.watches.iter().map(|w| w.provider.clone()).collect();
        self.step(&format!("GeForce NOW is checking {labels}"));
        for provider in providers {
            self.enqueue("account.connections.sync", json!({"provider":provider}), Purpose::SyncStart { provider }, None);
        }
    }

    /// The answer to a sync or a status check: where the sync is, and what to do next.
    pub(super) fn refresh_progress(&mut self, provider: &str, r: &Value) {
        let now = self.now;
        let Some(refresh) = &mut self.refresh else { return };
        let Some(index) = refresh.watches.iter().position(|w| w.provider == provider) else { return };
        let watch = &mut refresh.watches[index];
        watch.polling = false;
        if let Some(id) = r["operationId"].as_str().filter(|id| !id.is_empty()) {
            watch.operation = Some(id.to_owned());
        }
        let label = watch.label.clone();
        match r["phase"].as_str() {
            Some("refreshing_library") => {
                watch.done = true;
                if let Some(id) = &watch.operation {
                    self.acks.push(id.clone());
                }
            }
            Some("complete" | "cancelled_observation") => watch.done = true,
            Some("failed") => {
                watch.done = true;
                refresh.notes.push(format!("GeForce NOW reports a sync problem with {label}. Check its link in GeForce NOW."));
            }
            Some("timed_out") => {
                watch.done = true;
                refresh.notes.push(format!("GeForce NOW has not confirmed {label} yet. Check again in a minute."));
            }
            // Waiting, or a phase this engine does not know: ask again after the pause the core names.
            _ => {
                let pause = r["retryAfterMs"].as_u64().unwrap_or(POLL_DEFAULT_MS).clamp(POLL_MIN_MS, POLL_MAX_MS);
                watch.due = now + pause;
            }
        }
        self.refresh_check();
    }

    /// A core refusal of a step of the refresh. `None` when the job was not one.
    pub(super) fn refresh_rejected(&mut self, purpose: &Purpose, code: &str) {
        if let Some((kind, message)) = classify(code) {
            self.refresh = None;
            return self.error(None, kind, message);
        }
        let Some(refresh) = &mut self.refresh else { return };
        match purpose {
            Purpose::SyncList => {
                refresh.listing = false;
                refresh.notes.push("Your linked stores could not be checked.".into());
            }
            Purpose::SyncStart { provider } | Purpose::SyncPoll { provider } => {
                if let Some(watch) = refresh.watches.iter_mut().find(|w| &w.provider == provider) {
                    watch.done = true;
                    watch.polling = false;
                    refresh.notes.push(format!("GeForce NOW did not accept the refresh of {} ({code}).", watch.label));
                }
            }
            _ => {}
        }
        self.refresh_check();
    }

    /// Ends the refresh when nothing is left to wait for.
    fn refresh_check(&mut self) {
        if self.refresh.as_ref().is_some_and(|r| !r.listing && r.watches.iter().all(|w| w.done)) {
            self.refresh_end();
        }
    }

    fn refresh_end(&mut self) {
        let Some(refresh) = self.refresh.take() else { return };
        let note = (!refresh.notes.is_empty()).then(|| refresh.notes.join(" "));
        self.emit(refresh.seq, Event::Refresh { step: None, note });
    }

    /// Status checks that have come due, and the end of the wait.
    pub(super) fn refresh_tick(&mut self) {
        let now = self.now;
        let Some(refresh) = &mut self.refresh else { return };
        if now >= refresh.deadline {
            let waiting: Vec<String> = refresh.watches.iter().filter(|w| !w.done).map(|w| w.label.clone()).collect();
            refresh.notes.extend(waiting.iter().map(|label| format!("GeForce NOW was still checking {label} when the wait ended.")));
            return self.refresh_end();
        }
        let mut due = Vec::new();
        for watch in refresh.watches.iter_mut().filter(|w| !w.done && !w.polling && w.due > 0 && now >= w.due) {
            if let Some(id) = &watch.operation {
                watch.polling = true;
                watch.due = 0;
                due.push((watch.provider.clone(), id.clone()));
            }
        }
        for (provider, id) in due {
            self.enqueue("account.connections.sync.status", json!({"operationId":id}), Purpose::SyncPoll { provider }, None);
        }
    }

    /// The new library is out: tells the core, so that it can close the operations that were waiting for it.
    pub(super) fn acknowledge_refresh(&mut self) {
        for id in std::mem::take(&mut self.acks) {
            self.enqueue("account.connections.sync.status", json!({"operationId":id,"libraryRefreshed":true}), Purpose::SyncAck, None);
        }
    }
}
