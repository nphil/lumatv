//! The loopback API the TV's own UI uses for plugins: `/v1/plugins...`.
//!
//! What plugins publish is read from the hub (`plugins`, `plugin.<id>.cards`, ...) and their settings pages through
//! `/v1/settings/<id>`. These routes *ask a connected plugin to run an action it declared* (a tile was tapped, a button of its
//! page pressed: the UI asks the person first for an action that declared a confirmation), ask the helper to read
//! `plugins/enabled.json` again, and switch one of the helper's own plugins that ask first, for the TV's own Settings only. No route
//! switches a plugin on the socket on or off, installs one or changes its permissions.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/plugins` | the `plugins` topic |
//! | `POST /v1/plugins/rescan` | the roster is read again now |
//! | `POST /v1/plugins/{id}/actions/{action}` `{"target"?: "..."}` | `202 {"op": n}`; the plugin's answer is an `action_result` event on `plugin.<id>` |
//! | `GET /v1/plugins/{id}/choices/{provider}?offset=&limit=` | a page of the choices one of the plugin's settings fields offers (its room, its lights): `{"state", "rows": [{id, label, detail?}], "more"}`; the helper asks the plugin and waits for its answer |
//! | `POST /v1/plugins/lease/ha/refresh` | reads the saved Home Assistant link again (`lumatv-helper ha-link` says so after it saves one) |
//! | `POST /v1/plugins/{id}/switch` `{"on": true, "approve": code}` or `{"on": false}` | one of the helper's own plugins that ask first, switched by the owner in Settings (`X-Luma-Client: lumatv-ui` only): on takes the code of the words its `plugins` entry shows; 404 for no such plugin, 409 when its words changed |

use super::session::{ChoicesError, InvokeError};
use super::{Plugins, SwitchError};
use crate::API_VERSION;
use crate::http::{CLIENT_HEADER, Request, Response, UI_CLIENT, error_response};
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
    (
        "POST",
        "/v1/plugins/{id}/switch",
        "switches one of the helper's own plugins that ask first, for the TV's Settings: {\"on\": bool, \"approve\"?: code}",
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
        [id, "switch"] if post => {
            if request.header(CLIENT_HEADER) != Some(UI_CLIENT) {
                return error_response(
                    403,
                    "not_the_ui",
                    "only the TV's own Settings switches a plugin",
                );
            }
            let body = serde_json::from_slice::<Value>(&request.body).unwrap_or(Value::Null);
            let (Some(on), approve) = (body["on"].as_bool(), body["approve"].as_str()) else {
                return error_response(
                    400,
                    "bad_request",
                    "the body is {\"on\": true, \"approve\": code} or {\"on\": false}",
                );
            };
            match plugins.switch(id, on, approve) {
                Ok(()) => {
                    let mut body = Map::new();
                    body.insert("id".into(), json!(id));
                    body.insert("on".into(), json!(on));
                    ok(200, body)
                }
                Err(SwitchError::NotOffered) => error_response(
                    404,
                    "no_such_plugin",
                    "no plugin of the TV's own asks to be switched by that name",
                ),
                Err(SwitchError::Consent) => error_response(
                    409,
                    "consent_changed",
                    "the code is not the one of the words this plugin asks with now",
                ),
                Err(SwitchError::File(why)) => error_response(409, "unavailable", &why),
            }
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
            [] | ["rescan"] | [_, "actions", _] | ["lease", "ha", "refresh"] | [_, "switch"]
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
