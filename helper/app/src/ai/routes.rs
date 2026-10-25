//! The loopback API the TV's own UI uses for AI access: `/v1/ai/...`.
//!
//! These routes let the UI *ask*. What gives more access (the listener on, a pairing code, a wider level, a resumed assistant, a
//! higher volume limit) answers `202` and opens a prompt on the TV; it happens only if a physical remote approves it. What
//! gives less is done at once. There is **no route that answers a prompt** and none that creates a token: the list below is the
//! whole surface, and a test walks it.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/ai` | the module's state (the `ai` topic) |
//! | `GET /v1/ai/audit?limit=&assistant=` | recent calls |
//! | `POST /v1/ai/listener {"enabled": bool}` | off: done; on: `202`, prompt on the TV |
//! | `POST /v1/ai/pairing` | `202`, prompt on the TV; the code shows in the state after OK |
//! | `DELETE /v1/ai/pairing` | closes the window |
//! | `POST /v1/ai/assistants/{id}/levels {"levels": {state, playback, launch, power}}` | lower: done; any higher: `202` |
//! | `POST /v1/ai/assistants/{id}/pause` | done |
//! | `POST /v1/ai/assistants/{id}/resume` | `202`, prompt on the TV |
//! | `DELETE /v1/ai/assistants/{id}` | unpairs it, done |
//! | `POST /v1/ai/limits {"maxVolume": n}` | lower: done; higher: `202` |
//! | `POST /v1/ai/lockout` | every assistant unpaired, the listener off: done |

use super::levels::Levels;
use super::tools::round_half;
use super::{Action, Ai, Refusal};
use crate::API_VERSION;
use crate::http::{Request, Response, error_response};
use crate::timeutil::now_ms;
use serde_json::{Map, Value, json};

/// Method, path and what it does: the discovery document and the route test read this.
pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/ai", "the AI access state"),
    ("GET", "/v1/ai/audit", "recent calls: ?limit=&assistant="),
    (
        "POST",
        "/v1/ai/listener",
        "{\"enabled\": bool}: off is done at once; on opens a prompt on the TV",
    ),
    (
        "POST",
        "/v1/ai/pairing",
        "opens a prompt on the TV; the code shows after OK",
    ),
    ("DELETE", "/v1/ai/pairing", "closes the pairing window"),
    (
        "POST",
        "/v1/ai/assistants/{id}/levels",
        "{\"levels\": {...}}: lowering is done at once; raising opens a prompt on the TV",
    ),
    (
        "POST",
        "/v1/ai/assistants/{id}/pause",
        "pauses an assistant at once",
    ),
    (
        "POST",
        "/v1/ai/assistants/{id}/resume",
        "opens a prompt on the TV to resume it",
    ),
    (
        "DELETE",
        "/v1/ai/assistants/{id}",
        "unpairs an assistant at once",
    ),
    (
        "POST",
        "/v1/ai/limits",
        "{\"maxVolume\": n}: lowering is done at once; raising opens a prompt on the TV",
    ),
    (
        "POST",
        "/v1/ai/lockout",
        "unpairs every assistant and turns the listener off",
    ),
];

fn ok(status: u16, mut body: Map<String, Value>) -> Response {
    body.insert("ok".into(), Value::Bool(true));
    body.insert("api".into(), json!(API_VERSION));
    body.insert("now".into(), json!(now_ms()));
    Response::json(status, &Value::Object(body))
}

fn object(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

fn action(result: Result<Action, Refusal>, ai: &Ai) -> Response {
    match result {
        Ok(Action::Done { changed }) => ok(
            200,
            object(&[
                ("changed", json!(changed)),
                ("pending", json!(false)),
                ("ai", ai.doc()),
            ]),
        ),
        Ok(Action::Pending { prompt_id }) => ok(
            202,
            object(&[
                ("pending", json!(true)),
                ("promptId", json!(prompt_id)),
                (
                    "note",
                    json!(
                        "a prompt is open on the TV; it completes only if the physical remote approves it"
                    ),
                ),
            ]),
        ),
        Err(Refusal::Busy) => error_response(409, "busy", "another prompt is open on the TV"),
        Err(Refusal::NotFound) => error_response(404, "no_such_assistant", "no such assistant"),
        Err(Refusal::Invalid(why)) => error_response(400, "bad_request", &why),
        Err(Refusal::Io(why)) => error_response(500, "io", &why),
    }
}

/// A JSON object body (an empty body is an empty object) with only the allowed keys.
fn body_of(request: &Request, allowed: &[&str]) -> Result<Map<String, Value>, Response> {
    let map = if request.body.iter().all(u8::is_ascii_whitespace) {
        Map::new()
    } else {
        match serde_json::from_slice::<Value>(&request.body) {
            Ok(Value::Object(map)) => map,
            Ok(_) => {
                return Err(error_response(
                    400,
                    "bad_request",
                    "the body must be a JSON object",
                ));
            }
            Err(e) => {
                return Err(error_response(
                    400,
                    "bad_request",
                    &format!("the body is not JSON: {e}"),
                ));
            }
        }
    };
    if let Some(extra) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(error_response(
            400,
            "bad_request",
            &format!("unexpected field '{}'", super::tools::clean_text(extra, 40)),
        ));
    }
    Ok(map)
}

fn well_formed_id(id: &str) -> bool {
    id.len() == 12
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Handles `/v1/ai/...`; `rest` is the path after `ai`.
pub fn handle(ai: &Ai, request: &Request, rest: &[&str]) -> Response {
    let (get, post, delete) = (
        request.method == "GET",
        request.method == "POST",
        request.method == "DELETE",
    );
    match rest {
        [] if get => ok(200, object(&[("ai", ai.doc())])),
        ["audit"] if get => {
            let limit = match request.param("limit") {
                None | Some("") => 50,
                Some(text) => match text.parse::<usize>() {
                    Ok(n) if n <= 500 => n,
                    _ => {
                        return error_response(
                            400,
                            "bad_request",
                            "limit must be a whole number up to 500",
                        );
                    }
                },
            };
            let assistant = request.param("assistant").filter(|a| well_formed_id(a));
            ok(
                200,
                object(&[("entries", Value::Array(ai.recent_audit(limit, assistant)))]),
            )
        }
        ["listener"] if post => {
            let body = match body_of(request, &["enabled"]) {
                Ok(b) => b,
                Err(r) => return r,
            };
            match body.get("enabled").and_then(Value::as_bool) {
                Some(enabled) => action(ai.set_listener(enabled), ai),
                None => error_response(400, "bad_request", "enabled must be true or false"),
            }
        }
        ["pairing"] if post => match body_of(request, &[]) {
            Ok(_) => action(ai.start_pairing(), ai),
            Err(r) => r,
        },
        ["pairing"] if delete => ok(
            200,
            object(&[
                ("changed", json!(ai.cancel_pairing())),
                ("pending", json!(false)),
                ("ai", ai.doc()),
            ]),
        ),
        ["assistants", id, "levels"] if post && well_formed_id(id) => {
            let body = match body_of(request, &["levels"]) {
                Ok(b) => b,
                Err(r) => return r,
            };
            let Some(levels) = body.get("levels") else {
                return error_response(400, "bad_request", "levels is required");
            };
            match Levels::from_json(levels) {
                Ok(levels) => action(ai.set_levels(id, levels), ai),
                Err(why) => error_response(400, "bad_request", &why),
            }
        }
        ["assistants", id, "pause"] if post && well_formed_id(id) => match body_of(request, &[]) {
            Ok(_) => action(ai.pause(id), ai),
            Err(r) => r,
        },
        ["assistants", id, "resume"] if post && well_formed_id(id) => match body_of(request, &[]) {
            Ok(_) => action(ai.resume(id), ai),
            Err(r) => r,
        },
        ["assistants", id] if delete && well_formed_id(id) => action(ai.revoke(id), ai),
        ["limits"] if post => {
            let body = match body_of(request, &["maxVolume"]) {
                Ok(b) => b,
                Err(r) => return r,
            };
            match body.get("maxVolume").and_then(Value::as_f64) {
                Some(v) if v.is_finite() && (0.0..=98.0).contains(&v) => {
                    action(ai.set_max_volume(round_half(v)), ai)
                }
                _ => error_response(
                    400,
                    "bad_request",
                    "maxVolume must be a number from 0 to 98",
                ),
            }
        }
        ["lockout"] if post => match body_of(request, &[]) {
            Ok(_) => {
                let result = ai.lock_out();
                ok(
                    200,
                    object(&[
                        (
                            "changed",
                            json!(
                                result.revoked > 0
                                    || result.listener_was_on
                                    || result.pairing_was_open
                            ),
                        ),
                        ("revoked", json!(result.revoked)),
                        ("listenerWasOn", json!(result.listener_was_on)),
                        ("pending", json!(false)),
                        ("ai", ai.doc()),
                    ]),
                )
            }
            Err(r) => r,
        },
        ["assistants", id, ..] | ["assistants", id] if !well_formed_id(id) => {
            error_response(404, "no_such_assistant", "no such assistant")
        }
        _ if is_known(rest) => {
            error_response(405, "method_not_allowed", "that method is not served here")
                .with_header("Allow", allowed(rest))
        }
        _ => error_response(404, "not_found", "no such endpoint"),
    }
}

fn is_known(rest: &[&str]) -> bool {
    matches!(
        rest,
        [] | ["audit" | "listener" | "pairing" | "limits" | "lockout"]
            | ["assistants", _]
            | ["assistants", _, "levels" | "pause" | "resume"]
    )
}

fn allowed(rest: &[&str]) -> &'static str {
    match rest {
        [] | ["audit"] => "GET",
        ["pairing"] => "POST, DELETE",
        ["assistants", _] => "DELETE",
        _ => "POST",
    }
}
