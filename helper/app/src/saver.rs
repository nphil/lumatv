//! The screensaver's slots for anything that is not a plugin card: a bridge from ntfy, a script, an app posts a *complication* (a glyph, a
//! short label and value, a priority and an expiry; the contract and its validation are `lumatv-saver`'s, `docs/research/saver-complications.md`)
//! and the helper holds it on a board and publishes the topic `saver` that the TV's saver draws. A plugin on the plugin API does not post: its cards
//! are complications by the crate's rules, read by the saver from the `plugin.<id>.cards` topics, so the board here holds only what the routes put.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/saver/complications` | `{items: [...]}`: what is held, in the order it shows |
//! | `POST /v1/saver/complications` | `{id, source?, glyph?, text?, value?, priority?, expiresAt? or ttlS?, tone?}`: puts (or replaces) one; `source` is the caller's name (default `api`) |
//! | `DELETE /v1/saver/complications/{source}/{id}` | takes one away; `DELETE /v1/saver/complications/{source}` takes away everything the source said |
//!
//! Nothing is saved: the board is the helper's memory, and a restarted helper starts empty, which is right for things that are true now.

use crate::API_VERSION;
use crate::hub::Hub;
use crate::http::{Request, Response, error_response};
use crate::timeutil::now_ms;
use lumatv_saver::{Board, parse};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::LazyLock;

pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/saver/complications", "what the screensaver's slots hold, in the order they show"),
    (
        "POST",
        "/v1/saver/complications",
        "puts one complication on the screensaver: {id, source?, glyph?, text?, value?, priority?, ttlS? or expiresAt?, tone?}",
    ),
    (
        "DELETE",
        "/v1/saver/complications/{source}/{id}",
        "takes one complication away (without the id: everything the source said)",
    ),
];

#[derive(Default)]
pub struct Saver {
    board: Mutex<Board>,
}

static SAVER: LazyLock<Saver> = LazyLock::new(Saver::default);

fn ok(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}

impl Saver {
    /// The topic `saver`, as the board stands at `now`.
    fn publish(&self, hub: &Hub, board: &Board, now: u64) {
        hub.set("saver", board.document(now));
    }

    pub fn handle_at(&self, hub: &Hub, request: &Request, rest: &[&str], now: u64) -> Response {
        let mut board = self.board.lock();
        board.expire(now);
        match (request.method.as_str(), rest) {
            ("GET", ["complications"]) => ok(board.document(now)),
            ("POST", ["complications"]) => {
                let body: Value = match serde_json::from_slice(&request.body) {
                    Ok(v) => v,
                    Err(e) => return error_response(400, "bad_request", &format!("the body is not JSON: {e}")),
                };
                let source = body.get("source").and_then(Value::as_str).unwrap_or("api").to_string();
                match parse(&body, &source, now) {
                    Ok(c) => {
                        let id = c.id.clone();
                        let lost = board.put(c);
                        self.publish(hub, &board, now);
                        ok(json!({"id": id, "source": source, "dropped": lost}))
                    }
                    Err(why) => error_response(400, "bad_complication", &why),
                }
            }
            ("DELETE", ["complications", source, id]) => {
                let gone = board.remove(source, id);
                self.publish(hub, &board, now);
                ok(json!({"removed": usize::from(gone)}))
            }
            ("DELETE", ["complications", source]) => {
                let gone = board.remove_source(source);
                self.publish(hub, &board, now);
                ok(json!({"removed": gone}))
            }
            (_, ["complications"] | ["complications", _] | ["complications", _, _]) => {
                error_response(405, "method_not_allowed", "that method is not served here").with_header("Allow", "GET, POST, DELETE")
            }
            _ => error_response(404, "not_found", "no such saver route"),
        }
    }
}

/// Serves `/v1/saver/...` on the helper's one board.
pub fn handle(hub: &Hub, request: &Request, rest: &[&str]) -> Response {
    SAVER.handle_at(hub, request, rest, now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, path: &str, body: &str) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            peer: std::net::IpAddr::from([127, 0, 0, 1]),
        }
    }

    fn rest(path: &str) -> Vec<String> {
        path.trim_start_matches("/v1/saver/").split('/').map(str::to_string).collect()
    }

    fn call(s: &Saver, hub: &Hub, method: &str, path: &str, body: &str, now: u64) -> (u16, Value) {
        let segs = rest(path);
        let refs: Vec<&str> = segs.iter().map(String::as_str).collect();
        let r = s.handle_at(hub, &request(method, path, body), &refs, now);
        (r.status, serde_json::from_slice(&r.body).unwrap_or(Value::Null))
    }

    const NOW: u64 = 1_800_000_000_000;

    #[test]
    fn a_post_is_held_published_on_the_topic_and_taken_away_again() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        let (status, a) = call(&s, &hub, "POST", "/v1/saver/complications", r#"{"id": "door", "source": "ntfy", "glyph": "bell-ring", "text": "Front door", "value": "Open", "priority": 1, "ttlS": 600}"#, NOW);
        assert_eq!(status, 200, "{a}");
        assert_eq!((a["source"].as_str(), a["dropped"].as_u64()), (Some("ntfy"), Some(0)));
        let topic = hub.topic_at("saver").1.expect("the topic is published");
        assert_eq!(topic["items"][0]["text"], "Front door");
        assert_eq!(topic["items"][0]["expiresAt"], NOW + 600_000);
        let (_, got) = call(&s, &hub, "GET", "/v1/saver/complications", "", NOW + 1);
        assert_eq!(got["items"].as_array().unwrap().len(), 1);
        let (_, gone) = call(&s, &hub, "DELETE", "/v1/saver/complications/ntfy/door", "", NOW + 2);
        assert_eq!(gone["removed"], 1);
        assert_eq!(hub.topic_at("saver").1.unwrap()["items"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn a_source_with_no_name_is_api_a_bad_one_is_refused_with_its_words_and_a_source_can_be_cleared_at_once() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        let (_, a) = call(&s, &hub, "POST", "/v1/saver/complications", r#"{"id": "x", "text": "Hi"}"#, NOW);
        assert_eq!(a["source"], "api");
        let (status, bad) = call(&s, &hub, "POST", "/v1/saver/complications", r#"{"id": "y"}"#, NOW);
        assert_eq!(status, 400);
        assert_eq!(bad["error"]["code"], "bad_complication");
        assert!(bad["error"]["message"].as_str().unwrap().contains("glyph, a text or a value"));
        assert_eq!(call(&s, &hub, "POST", "/v1/saver/complications", "not json", NOW).0, 400);
        for i in 0..3 {
            call(&s, &hub, "POST", "/v1/saver/complications", &format!(r#"{{"id": "i{i}", "source": "bridge", "text": "n{i}"}}"#), NOW);
        }
        let (_, cleared) = call(&s, &hub, "DELETE", "/v1/saver/complications/bridge", "", NOW);
        assert_eq!(cleared["removed"], 3);
        assert_eq!(call(&s, &hub, "PUT", "/v1/saver/complications", "", NOW).0, 405);
        assert_eq!(call(&s, &hub, "GET", "/v1/saver/other", "", NOW).0, 404);
    }

    #[test]
    fn what_has_expired_is_not_served() {
        let (s, hub) = (Saver::default(), Hub::new(1));
        call(&s, &hub, "POST", "/v1/saver/complications", r#"{"id": "short", "text": "Short", "ttlS": 5}"#, NOW);
        let (_, got) = call(&s, &hub, "GET", "/v1/saver/complications", "", NOW + 6000);
        assert_eq!(got["items"].as_array().unwrap().len(), 0);
    }
}
