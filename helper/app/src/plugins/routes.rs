//! The loopback API the TV's own UI uses for plugins: `/v1/plugins...`.
//!
//! What plugins publish is read from the hub (`plugins`, `plugin.<id>.cards`, ...) and their settings pages through
//! `/v1/settings/<id>`. These routes only *ask a connected plugin to run an action it declared* (a tile was tapped, a button of its
//! page pressed: the UI asks the person first for an action that declared a confirmation) and ask the helper to read
//! `plugins/enabled.json` again. There is no route that switches a plugin on or off, installs one or changes its permissions.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/plugins` | the `plugins` topic |
//! | `POST /v1/plugins/rescan` | the roster is read again now |
//! | `POST /v1/plugins/{id}/actions/{action}` `{"target"?: "..."}` | `202 {"op": n}`; the plugin's answer is an `action_result` event on `plugin.<id>` |
//! | `GET /v1/plugins/{id}/choices/{provider}?offset=&limit=` | a page of the choices one of the plugin's settings fields offers (its room, its lights): `{"state", "rows": [{id, label, detail?}], "more"}`; the helper asks the plugin and waits for its answer |
//! | `POST /v1/plugins/lease/ha/refresh` | reads the saved Home Assistant link again (`lumatv-helper ha-link` says so after it saves one) |

use super::Plugins;
use super::session::{ChoicesError, InvokeError};
use crate::API_VERSION;
use crate::http::{Request, Response, error_response};
use crate::timeutil::now_ms;
use serde_json::{Map, Value, json};

pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/plugins", "the plugins and where each is"),
    (
        "POST",
        "/v1/plugins/rescan",
        "reads plugins/enabled.json again now",
    ),
    (
        "POST",
        "/v1/plugins/{id}/actions/{action}",
        "asks a connected plugin to run one of its declared actions: {\"target\"?: text}",
    ),
    (
        "GET",
        "/v1/plugins/{id}/choices/{provider}",
        "a page of the choices a field of the plugin's settings page offers: ?offset=&limit=",
    ),
    (
        "POST",
        "/v1/plugins/lease/ha/refresh",
        "reads the saved Home Assistant link again",
    ),
];

fn ok(status: u16, mut body: Map<String, Value>) -> Response {
    body.insert("ok".into(), Value::Bool(true));
    body.insert("api".into(), json!(API_VERSION));
    body.insert("now".into(), json!(now_ms()));
    Response::json(status, &Value::Object(body))
}

fn plain_key(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

/// Handles `/v1/plugins/...`; `rest` is the path after `plugins`.
pub fn handle(plugins: &Plugins, request: &Request, rest: &[&str]) -> Response {
    let (get, post) = (request.method == "GET", request.method == "POST");
    match rest {
        [] if get => {
            let view = plugins
                .shared()
                .hub
                .get("plugins")
                .unwrap_or_else(|| json!({"plugins": [], "firstParty": [], "problem": null}));
            let mut body = Map::new();
            body.insert("plugins".into(), view["plugins"].clone());
            body.insert("firstParty".into(), view["firstParty"].clone());
            body.insert("problem".into(), view["problem"].clone());
            ok(200, body)
        }
        ["rescan"] if post => {
            plugins.rescan();
            ok(202, Map::new())
        }
        [id, "actions", action] if post => {
            if plugin_api::check_id(id).is_err() || !plain_key(action) {
                return error_response(404, "no_such_action", "no such plugin action");
            }
            let target = if request.body.iter().all(u8::is_ascii_whitespace) {
                None
            } else {
                match serde_json::from_slice::<Value>(&request.body) {
                    Ok(Value::Object(map)) if map.keys().all(|k| k == "target") => {
                        match map.get("target") {
                            None | Some(Value::Null) => None,
                            Some(Value::String(t)) if t.len() <= 128 => Some(t.clone()),
                            _ => {
                                return error_response(
                                    400,
                                    "bad_request",
                                    "target is a text of at most 128 bytes",
                                );
                            }
                        }
                    }
                    _ => {
                        return error_response(
                            400,
                            "bad_request",
                            "the body is empty or {\"target\": text}",
                        );
                    }
                }
            };
            match plugins.shared().invoke_action(id, action, target) {
                Ok(op) => {
                    let mut body = Map::new();
                    body.insert("op".into(), json!(op));
                    ok(202, body)
                }
                Err(e @ (InvokeError::NoSuchPlugin | InvokeError::NoSuchAction)) => {
                    error_response(404, "no_such_action", e.words())
                }
                Err(e) => error_response(409, "unavailable", e.words()),
            }
        }
        ["lease", "ha", "refresh"] if post => {
            plugins.lease().refresh();
            ok(202, Map::new())
        }
        [id, "choices", provider] if get => {
            if plugin_api::check_id(id).is_err() || !plain_key(provider) {
                return error_response(404, "no_such_provider", "no such provider");
            }
            let number = |name: &str, fallback: u32, most: u32| {
                request
                    .param(name)
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(fallback)
                    .min(most)
            };
            let (offset, limit) = (number("offset", 0, 1_000_000), number("limit", 24, 64));
            match plugins.shared().choices(id, provider, offset, limit as u16) {
                Ok(page) => {
                    let mut body = Map::new();
                    body.insert("plugin".into(), json!(id));
                    body.insert("provider".into(), json!(provider));
                    body.insert(
                        "page".into(),
                        serde_json::to_value(&page).unwrap_or(Value::Null),
                    );
                    ok(200, body)
                }
                Err(e @ (ChoicesError::NoSuchPlugin | ChoicesError::NoSuchProvider)) => {
                    error_response(404, "no_such_provider", e.words())
                }
                Err(e @ ChoicesError::Timeout) => error_response(504, "timeout", e.words()),
                Err(e) => error_response(409, "unavailable", e.words()),
            }
        }
        _ if matches!(
            rest,
            [] | ["rescan"] | [_, "actions", _] | ["lease", "ha", "refresh"]
        ) =>
        {
            error_response(405, "method_not_allowed", "that method is not served here")
                .with_header("Allow", if rest.is_empty() { "GET" } else { "POST" })
        }
        _ if matches!(rest, [_, "choices", _]) => {
            error_response(405, "method_not_allowed", "that method is not served here")
                .with_header("Allow", "GET")
        }
        _ => error_response(404, "not_found", "no such endpoint"),
    }
}
