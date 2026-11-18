//! The audio an app decodes for itself, for the format badge (`docs/media-audio.md`): LG's own services see only what the app hands the
//! TV, so an app that decodes (Marquee, a native Plex client) says what it plays and what it delivers.
//!
//! | Request | Answer |
//! |---|---|
//! | `POST /v1/media/audio` | `{app, context?, codec, profile?, atmos?, channels?, layout?, delivered}`: the app's latest entry, replacing its last; 204 |
//! | `DELETE /v1/media/audio?app=<id>` | clears that app's entry; 204 |
//!
//! `codec` is one of `ac3 eac3 truehd dts aac pcm opus flac mp3 other` (lowercase), `delivered` is `source`, `server_converted` or
//! `app_stereo_mix`, `context` is the videooutputd context of the picture the sound belongs to (null: whatever the app has in front).
//! An entry lives 75 s after its last POST (senders repeat every 30 s) and a repeat of the same entry is no news. At most [`MAX_APPS`]
//! apps are kept; a new one past that replaces the entry that was posted longest ago.
//!
//! The topic `media.audio`: `{"entries": [{app, context, codec, profile, atmos, channels, layout, delivered}]}`, published on every change
//! and every expiry; no time is in it, so a keep-alive wakes nobody. One thread sleeps until the next expiry; nothing is polled.

use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::timeutil::now_ms;
use parking_lot::{Condvar, Mutex};
use serde_json::{Map, Value, json};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "POST",
        "/v1/media/audio",
        "the audio an app decodes, for the format badge: {app, context?, codec, profile?, atmos?, channels?, layout?, delivered}; repeat within 75 s",
    ),
    ("DELETE", "/v1/media/audio", "clears an app's audio: ?app=<id>"),
];

pub const TOPIC: &str = "media.audio";
/// An entry ends this long after its last POST; senders repeat every 30 s.
pub const TTL_MS: u64 = 75_000;
/// How many apps have an entry at once.
pub const MAX_APPS: usize = 4;
const CODECS: [&str; 10] = ["ac3", "eac3", "truehd", "dts", "aac", "pcm", "opus", "flac", "mp3", "other"];
const DELIVERED: [&str; 3] = ["source", "server_converted", "app_stereo_mix"];
const TEXT_MAX: usize = 64;
const CHANNELS_MAX: u64 = 32;

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    /// The entry as the topic carries it.
    json: Value,
    app: String,
    expires: u64,
}

#[derive(Default)]
struct MediaAudio {
    entries: Mutex<Vec<Entry>>,
    /// Wakes the timer when the next expiry moves.
    changed: Condvar,
}

static AUDIO: LazyLock<MediaAudio> = LazyLock::new(MediaAudio::default);

fn bad(message: &str) -> Response {
    error_response(400, "bad_request", message)
}

fn no_content() -> Response {
    Response { status: 204, headers: Vec::new(), body: Vec::new() }
}

/// A short text field: absent or null is none; anything but a non-empty string of printable characters is the reason it is refused.
fn text_field(body: &Map<String, Value>, name: &str, max: usize) -> Result<Option<String>, String> {
    match body.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{name} must be null or a text of 1 to {max} characters")),
    }
}

/// Checks the body of a POST and makes the entry the topic carries.
fn parse(body: &Map<String, Value>) -> Result<(String, Value), String> {
    let app = text_field(body, "app", TEXT_MAX)?.ok_or("app is required: the id of the app that plays")?;
    let context = text_field(body, "context", TEXT_MAX)?;
    let profile = text_field(body, "profile", TEXT_MAX)?;
    let layout = text_field(body, "layout", TEXT_MAX)?;
    let one_of = |name: &str, allowed: &[&str]| match body.get(name).and_then(Value::as_str) {
        Some(v) if allowed.contains(&v) => Ok(v.to_owned()),
        _ => Err(format!("{name} must be one of {}", allowed.join(", "))),
    };
    let codec = one_of("codec", &CODECS)?;
    let delivered = one_of("delivered", &DELIVERED)?;
    let atmos = match body.get("atmos") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("atmos must be true or false".into()),
    };
    let channels = match body.get("channels") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().filter(|n| (1..=CHANNELS_MAX).contains(n)).ok_or(format!("channels must be null or a whole number from 1 to {CHANNELS_MAX}"))?),
    };
    let entry = json!({"app": app, "context": context, "codec": codec, "profile": profile, "atmos": atmos, "channels": channels, "layout": layout, "delivered": delivered});
    Ok((app, entry))
}

impl MediaAudio {
    fn expire(entries: &mut Vec<Entry>, now: u64) -> bool {
        let before = entries.len();
        entries.retain(|e| e.expires > now);
        entries.len() != before
    }

    /// Publishes the entries and tells the timer; called with them locked. A topic set to what it holds raises no event.
    fn commit(&self, hub: &Hub, entries: &[Entry]) {
        hub.set(TOPIC, json!({"entries": entries.iter().map(|e| &e.json).collect::<Vec<_>>()}));
        self.changed.notify_all();
    }

    fn handle_at(&self, hub: &Hub, request: &Request, now: u64) -> Response {
        if !request.peer.is_loopback() {
            return error_response(403, "local_only", "an app's audio is reported from the TV itself");
        }
        let mut entries = self.entries.lock();
        let expired = Self::expire(&mut entries, now);
        let applied = Self::apply(&mut entries, request, now);
        // an entry that ran out is published even when this request is refused: the timer would find nothing left to publish
        if applied.is_ok() || expired {
            self.commit(hub, &entries);
        }
        applied.map_or_else(|refusal| refusal, |()| no_content())
    }

    fn apply(entries: &mut Vec<Entry>, request: &Request, now: u64) -> Result<(), Response> {
        match request.method.as_str() {
            "POST" => {
                let body = match serde_json::from_slice::<Value>(&request.body) {
                    Ok(Value::Object(map)) => map,
                    Ok(_) => return Err(bad("the body must be a JSON object")),
                    Err(e) => return Err(bad(&format!("the body is not JSON: {e}"))),
                };
                let (app, json) = parse(&body).map_err(|why| bad(&why))?;
                let entry = Entry { json, app, expires: now.saturating_add(TTL_MS) };
                if let Some(slot) = entries.iter_mut().find(|e| e.app == entry.app) {
                    *slot = entry;
                } else {
                    if entries.len() >= MAX_APPS
                        && let Some(oldest) = entries.iter().enumerate().min_by_key(|(_, e)| e.expires).map(|(i, _)| i)
                    {
                        entries.remove(oldest);
                    }
                    entries.push(entry);
                }
            }
            "DELETE" => {
                let app = request.param("app").filter(|a| !a.is_empty()).ok_or_else(|| bad("app is required: ?app=<id>"))?;
                entries.retain(|e| e.app != app);
            }
            _ => return Err(error_response(405, "method_not_allowed", "that method is not served here").with_header("Allow", "POST, DELETE")),
        }
        Ok(())
    }

    /// Sleeps until the next expiry (or a change) and publishes what ran out.
    fn run(&self, hub: &Hub) {
        let mut entries = self.entries.lock();
        loop {
            let now = now_ms();
            if Self::expire(&mut entries, now) {
                self.commit(hub, &entries);
            }
            match entries.iter().map(|e| e.expires).min() {
                Some(at) => {
                    self.changed.wait_for(&mut entries, Duration::from_millis(at.saturating_sub(now).max(1)));
                }
                None => self.changed.wait(&mut entries),
            }
        }
    }
}

/// Serves `/v1/media/audio`.
pub fn handle(hub: &Hub, request: &Request) -> Response {
    AUDIO.handle_at(hub, request, now_ms())
}

/// Publishes the empty topic (a client's first snapshot has it) and starts the timer. Called once, with the helper's context.
pub fn open(ctx: &Ctx) {
    AUDIO.commit(&ctx.hub, &[]);
    let hub: Arc<Hub> = ctx.hub.clone();
    if let Err(e) = std::thread::Builder::new().name("media-audio-timer".to_owned()).stack_size(64 * 1024).spawn(move || AUDIO.run(&hub)) {
        crate::log_info!("media audio: the timer did not start ({e}); an entry's end is applied at the next request only");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn request(method: &str, query: &[(&str, &str)], body: &str) -> Request {
        Request {
            method: method.into(),
            path: "/v1/media/audio".into(),
            query: query.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            peer: std::net::IpAddr::from([127, 0, 0, 1]),
        }
    }

    fn entries(hub: &Hub) -> Vec<Value> {
        hub.topic_at(TOPIC).1.unwrap()["entries"].as_array().unwrap().clone()
    }

    const ATMOS: &str = r#"{"app":"marquee","context":"ctx1","codec":"truehd","profile":null,"atmos":true,"channels":8,"layout":"7.1","delivered":"source"}"#;

    #[test]
    fn a_post_is_checked_stored_per_app_and_a_delete_clears_it() {
        let (a, hub) = (MediaAudio::default(), Hub::new(1));
        let r = a.handle_at(&hub, &request("POST", &[], ATMOS), NOW);
        assert_eq!((r.status, r.body.len()), (204, 0));
        assert_eq!(entries(&hub), [json!({"app": "marquee", "context": "ctx1", "codec": "truehd", "profile": null, "atmos": true, "channels": 8, "layout": "7.1", "delivered": "source"})]);
        // the same app replaces its entry; the optional fields may be left out
        a.handle_at(&hub, &request("POST", &[], r#"{"app":"marquee","codec":"dts","profile":"dts-hd ma","delivered":"server_converted"}"#), NOW + 1);
        assert_eq!(entries(&hub), [json!({"app": "marquee", "context": null, "codec": "dts", "profile": "dts-hd ma", "atmos": false, "channels": null, "layout": null, "delivered": "server_converted"})]);
        // another app has its own
        a.handle_at(&hub, &request("POST", &[], r#"{"app":"other","codec":"aac","delivered":"app_stereo_mix"}"#), NOW + 2);
        assert_eq!(entries(&hub).len(), 2);
        assert_eq!(a.handle_at(&hub, &request("DELETE", &[("app", "marquee")], ""), NOW + 3).status, 204);
        assert_eq!(entries(&hub).iter().map(|e| e["app"].as_str().unwrap()).collect::<Vec<_>>(), ["other"]);
        assert_eq!(a.handle_at(&hub, &request("DELETE", &[("app", "unknown")], ""), NOW + 3).status, 204, "clearing nothing is not an error");
        assert_eq!(a.handle_at(&hub, &request("DELETE", &[], ""), NOW + 3).status, 400);
        assert_eq!(a.handle_at(&hub, &request("GET", &[], ""), NOW + 3).status, 405);
    }

    #[test]
    fn a_bad_or_missing_field_is_refused_with_its_name_and_changes_nothing() {
        let (a, hub) = (MediaAudio::default(), Hub::new(1));
        for (body, field) in [
            (r#"{"codec":"ac3","delivered":"source"}"#, "app"),
            (r#"{"app":"","codec":"ac3","delivered":"source"}"#, "app"),
            (r#"{"app":"m","context":7,"codec":"ac3","delivered":"source"}"#, "context"),
            (r#"{"app":"m","delivered":"source"}"#, "codec"),
            (r#"{"app":"m","codec":"AC3","delivered":"source"}"#, "codec"),
            (r#"{"app":"m","codec":"wma","delivered":"source"}"#, "codec"),
            (r#"{"app":"m","codec":"ac3"}"#, "delivered"),
            (r#"{"app":"m","codec":"ac3","delivered":"stereo"}"#, "delivered"),
            (r#"{"app":"m","codec":"ac3","atmos":"yes","delivered":"source"}"#, "atmos"),
            (r#"{"app":"m","codec":"ac3","channels":0,"delivered":"source"}"#, "channels"),
            (r#"{"app":"m","codec":"ac3","channels":"6","delivered":"source"}"#, "channels"),
        ] {
            let r = a.handle_at(&hub, &request("POST", &[], body), NOW);
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            assert_eq!(r.status, 400, "{body}");
            assert!(b["error"]["message"].as_str().unwrap().starts_with(field), "{body}: {b}");
        }
        for body in ["", "[]", "{"] {
            assert_eq!(a.handle_at(&hub, &request("POST", &[], body), NOW).status, 400, "{body:?}");
        }
        assert!(hub.topic_at(TOPIC).1.is_none(), "nothing was stored or published");
    }

    #[test]
    fn an_entry_ends_75_seconds_after_its_last_post_and_a_repeat_is_no_event() {
        let (a, hub) = (MediaAudio::default(), Hub::new(1));
        a.handle_at(&hub, &request("POST", &[], ATMOS), NOW);
        let seq = hub.snapshot().seq;
        // a keep-alive moves the end but not the topic
        a.handle_at(&hub, &request("POST", &[], ATMOS), NOW + 30_000);
        assert_eq!(hub.snapshot().seq, seq);
        assert_eq!(a.entries.lock()[0].expires, NOW + 30_000 + TTL_MS);
        let mut held = a.entries.lock();
        assert!(!MediaAudio::expire(&mut held, NOW + 30_000 + TTL_MS - 1));
        assert!(MediaAudio::expire(&mut held, NOW + 30_000 + TTL_MS));
        a.commit(&hub, &held);
        drop(held);
        assert_eq!(entries(&hub), Vec::<Value>::new());
        // a request after the end drops the entry before it adds its own, so a stale one never stays
        a.handle_at(&hub, &request("POST", &[], ATMOS), NOW);
        a.handle_at(&hub, &request("POST", &[], r#"{"app":"b","codec":"ac3","delivered":"source"}"#), NOW + TTL_MS);
        assert_eq!(entries(&hub).iter().map(|e| e["app"].as_str().unwrap()).collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn only_a_few_apps_are_kept_and_a_new_one_replaces_the_one_posted_longest_ago() {
        let (a, hub) = (MediaAudio::default(), Hub::new(1));
        for i in 0..MAX_APPS as u64 + 1 {
            let body = format!(r#"{{"app":"app{i}","codec":"ac3","delivered":"source"}}"#);
            a.handle_at(&hub, &request("POST", &[], &body), NOW + i);
        }
        let apps: Vec<String> = entries(&hub).iter().map(|e| e["app"].as_str().unwrap().to_owned()).collect();
        assert_eq!(apps, ["app1", "app2", "app3", "app4"]);
    }

    #[test]
    fn only_the_tv_itself_may_report() {
        let (a, hub) = (MediaAudio::default(), Hub::new(1));
        let mut r = request("POST", &[], ATMOS);
        r.peer = std::net::IpAddr::from([192, 168, 1, 9]);
        assert_eq!(a.handle_at(&hub, &r, NOW).status, 403);
        assert!(a.entries.lock().is_empty());
    }
}
