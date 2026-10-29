//! TV-level privacy switches and the menu model consumed by the Rust shell.
use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use lumatv_privacy::{Paths, Privacy, system::System};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/privacy",
        "privacy status per item, including unavailable reasons and observed Prime route hashes",
    ),
    (
        "GET",
        "/v1/privacy/menu",
        "toolkit-neutral, owner-only Privacy menu and links",
    ),
    (
        "POST",
        "/v1/privacy/prepare",
        "preview an exact switch change: {id, enabled}; returns a single-use 60-second confirmation",
    ),
    (
        "POST",
        "/v1/privacy/apply",
        "apply the preview after owner confirmation: {id, enabled, confirmation}",
    ),
];
pub fn new(ctx: &Ctx) -> Arc<Privacy> {
    let app = ctx
        .config
        .brand_app_id
        .as_deref()
        .unwrap_or("org.nphil.lumatv");
    Arc::new(Privacy::new(
        Paths {
            root: PathBuf::from("/"),
            data: ctx.paths.data_dir.clone(),
        },
        Arc::new(System {
            luna_send: ctx.config.luna.send.clone(),
            declutter: PathBuf::from(format!(
                "/media/developer/apps/usr/palm/applications/{app}/tools/declutter.sh"
            )),
        }),
    ))
}
pub fn spawn(ctx: &Ctx, privacy: Arc<Privacy>) -> std::io::Result<JoinHandle<()>> {
    let stop = ctx.stop.clone();
    thread::Builder::new()
        .name("privacy".into())
        .spawn(move || {
            privacy.resume();
            while !stop.sleep(Duration::from_secs(5)) {
                privacy.maintain();
            }
            privacy.stop();
        })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    id: String,
    enabled: bool,
    confirmation: Option<String>,
}
pub fn handle(privacy: &Privacy, request: &Request, rest: &[&str], can_admin: bool) -> Response {
    match (request.method.as_str(), rest) {
        ("GET", [] | ["menu"]) => ok(json!({"privacy":privacy.status(can_admin)})),
        ("POST", [action @ ("prepare" | "apply")]) => {
            if !can_admin {
                return error_response(
                    403,
                    "owner_only",
                    "Only the TV's owner can change privacy settings",
                );
            }
            let change: Change = match serde_json::from_slice(&request.body) {
                Ok(c) => c,
                Err(_) => {
                    return error_response(
                        400,
                        "bad_request",
                        "expected {id, enabled, confirmation?}",
                    );
                }
            };
            let result = if *action == "prepare" {
                privacy.prepare(&change.id, change.enabled)
            } else {
                let Some(token) = change.confirmation else {
                    return error_response(
                        409,
                        "confirmation_required",
                        "show the exact privacy change for confirmation first",
                    );
                };
                privacy.apply(&change.id, change.enabled, &token)
            };
            match result {
                Ok(v) => ok(v),
                Err(e) => error_response(409, "privacy_change_refused", &e),
            }
        }
        (_, [] | ["menu"] | ["prepare"] | ["apply"]) => {
            error_response(405, "method_not_allowed", "that method is not served here").with_header(
                "Allow",
                if rest.is_empty() || rest == ["menu"] {
                    "GET"
                } else {
                    "POST"
                },
            )
        }
        _ => error_response(404, "not_found", "no such privacy endpoint"),
    }
}
fn ok(mut v: Value) -> Response {
    if let Some(m) = v.as_object_mut() {
        m.insert("ok".into(), json!(true));
        m.insert("api".into(), json!(crate::API_VERSION));
        m.insert("now".into(), json!(crate::timeutil::now_ms()));
    }
    Response::json(200, &v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumatv_privacy::Platform;
    use std::path::Path;
    struct NoPlatformAccess;
    impl Platform for NoPlatformAccess {
        fn luna(&self, _: &str, _: &Value) -> Result<Value, String> {
            panic!("validation must precede platform access")
        }
        fn declutter(&self, _: &[&str]) -> Result<Value, String> {
            panic!("validation must precede platform access")
        }
        fn bind(&self, _: &Path, _: &Path) -> Result<(), String> {
            panic!("validation must precede platform access")
        }
        fn unmount(&self, _: &Path) -> Result<(), String> {
            panic!("validation must precede platform access")
        }
        fn close_app(&self, _: &str) -> Result<(), String> {
            panic!("validation must precede platform access")
        }
        fn stop_old_amazoff(&self) -> Result<Vec<String>, String> {
            panic!("validation must precede platform access")
        }
        fn cert_hash(&self, _: &Path) -> Result<String, String> {
            panic!("validation must precede platform access")
        }
    }
    fn request(body: &str) -> Request {
        Request {
            method: "POST".into(),
            path: "/v1/privacy/apply".into(),
            query: vec![],
            headers: vec![],
            body: body.as_bytes().to_vec(),
            peer: "127.0.0.1".parse().unwrap(),
        }
    }
    #[test]
    fn owner_and_confirmation_checks_precede_all_platform_access() {
        let dir = tempfile::tempdir().unwrap();
        let privacy = Privacy::new(
            Paths {
                root: dir.path().to_owned(),
                data: dir.path().join("data"),
            },
            Arc::new(NoPlatformAccess),
        );
        let req = request(r#"{"id":"ads","enabled":true}"#);
        assert_eq!(handle(&privacy, &req, &["prepare"], false).status, 403);
        assert_eq!(handle(&privacy, &req, &["apply"], false).status, 403);
        assert_eq!(handle(&privacy, &req, &["apply"], true).status, 409);
        let prepared = handle(&privacy, &req, &["prepare"], true);
        assert_eq!(prepared.status, 200);
        let value: Value = serde_json::from_slice(&prepared.body).unwrap();
        assert_eq!(value["expiresInSeconds"], 60);
        assert!(value["confirmation"].as_str().is_some());
        assert_eq!(
            handle(
                &privacy,
                &request(r#"{"id":"ads","enabled":true,"confirmation":"wrong"}"#),
                &["apply"],
                true
            )
            .status,
            409
        );
    }
    #[test]
    fn invalid_http_input_does_not_reach_the_platform() {
        let dir = tempfile::tempdir().unwrap();
        let privacy = Privacy::new(
            Paths {
                root: dir.path().to_owned(),
                data: dir.path().join("data"),
            },
            Arc::new(NoPlatformAccess),
        );
        assert_eq!(
            handle(&privacy, &request("not-json"), &["prepare"], true).status,
            400
        );
        assert_eq!(
            handle(
                &privacy,
                &request(r#"{"id":"ads","enabled":true,"extra":1}"#),
                &["prepare"],
                true
            )
            .status,
            400
        );
        assert_eq!(
            handle(&privacy, &request(""), &["missing"], true).status,
            404
        );
        assert_eq!(handle(&privacy, &request(""), &["menu"], true).status, 405);
    }
}
