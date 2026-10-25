//! The assistants' door over real TCP: the LAN-mode server with the MCP handler, as an AI client (and a hostile web page) meets it.

mod support;

use lumatv_helper::ai::mcp::{McpHandler, ServerInfo};
use lumatv_helper::ai::types::{
    AssistantView, AuthFailure, Authenticator, PairFailure, Paired, ToolDef, ToolHost, ToolOutcome,
};
use lumatv_helper::http::{Mode, Options, Server, default_lan_peers};
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use support::*;

struct Host;

impl ToolHost for Host {
    fn list_tools(&self, _who: &AssistantView) -> Vec<ToolDef> {
        vec![ToolDef {
            name: "get_volume",
            title: "Get the volume",
            description: "Reads the receiver's volume.".into(),
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            read_only: true,
        }]
    }

    fn call_tool(
        &self,
        who: &AssistantView,
        _peer: IpAddr,
        name: &str,
        _arguments: &Value,
    ) -> ToolOutcome {
        match name {
            "get_volume" => ToolOutcome::Ok {
                text: format!("The volume is 35 for {}.", who.name),
                data: json!({"volume": 35}),
            },
            _ => ToolOutcome::Err {
                code: "not_found",
                message: "no such tool".into(),
            },
        }
    }
}

struct Auth;

impl Authenticator for Auth {
    fn authenticate(&self, bearer: &str, _peer: IpAddr) -> Result<AssistantView, AuthFailure> {
        if bearer == "tok-1" {
            Ok(AssistantView {
                id: "a1".into(),
                name: "Cody".into(),
            })
        } else {
            Err(AuthFailure::Unknown)
        }
    }

    fn pair(&self, code: &str, _name: &str, _peer: IpAddr) -> Result<Paired, PairFailure> {
        if code == "424242" {
            Ok(Paired {
                assistant_id: "a1".into(),
                token: "tok-1".into(),
            })
        } else {
            Err(PairFailure::Refused)
        }
    }
}

fn start() -> Server {
    let handler = McpHandler::new(
        Arc::new(Host),
        Arc::new(Auth),
        ServerInfo {
            name: "lumatv".into(),
            title: "LumaTV".into(),
            version: "0.0.0-test".into(),
        },
    );
    Server::start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(handler),
        Options {
            max_connections: 8,
            mode: Mode::Lan {
                peers: default_lan_peers(),
            },
            ..Options::default()
        },
    )
    .unwrap()
}

/// One JSON request the way an MCP client sends it (no `X-Luma-Client`, no `Origin`).
fn send(
    addr: SocketAddr,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
    body: Option<&str>,
) -> Reply {
    let mut headers = vec![("Content-Type", "application/json")];
    headers.extend_from_slice(extra);
    http(addr, method, path, &headers, body)
}

fn rpc(addr: SocketAddr, token: &str, body: Value) -> Reply {
    let bearer = format!("Bearer {token}");
    send(
        addr,
        "POST",
        "/mcp",
        &[
            ("Authorization", bearer.as_str()),
            ("MCP-Protocol-Version", "2025-06-18"),
        ],
        Some(&body.to_string()),
    )
}

#[test]
fn an_assistant_pairs_and_uses_the_tools() {
    let server = start();
    let addr = server.local_addr();

    let paired = send(
        addr,
        "POST",
        "/pair",
        &[],
        Some(r#"{"code":"424242","name":"Cody"}"#),
    );
    assert_eq!(paired.status, 200);
    assert_eq!(
        paired.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    let paired = paired.json();
    assert_eq!(paired["ok"], true);
    assert_eq!(paired["mcpPath"], "/mcp");
    let token = paired["token"].as_str().unwrap().to_owned();

    let init = rpc(
        addr,
        &token,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "cody", "version": "1"}}}),
    );
    assert_eq!(init.status, 200);
    let init = init.json();
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "lumatv");

    let initialized = rpc(
        addr,
        &token,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    assert_eq!(initialized.status, 202);
    assert!(initialized.body.is_empty());
    assert_eq!(initialized.header("content-type"), None);
    assert_eq!(initialized.header("content-length"), Some("0"));

    let tools = rpc(
        addr,
        &token,
        json!({"jsonrpc": "2.0", "id": "t", "method": "tools/list"}),
    )
    .json();
    assert_eq!(tools["id"], "t");
    assert_eq!(tools["result"]["tools"][0]["name"], "get_volume");
    assert_eq!(
        tools["result"]["tools"][0]["annotations"]["readOnlyHint"],
        true
    );

    let called = rpc(
        addr,
        &token,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "get_volume"}}),
    )
    .json();
    assert_eq!(called["result"]["isError"], false);
    assert_eq!(
        called["result"]["content"][0]["text"],
        "The volume is 35 for Cody.\n{\"volume\":35}"
    );
    assert_eq!(called["result"]["structuredContent"], json!({"volume": 35}));

    let unknown = rpc(
        addr,
        &token,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "reboot"}}),
    )
    .json();
    assert_eq!(unknown["result"]["isError"], true);

    let refused = rpc(
        addr,
        "stolen",
        json!({"jsonrpc": "2.0", "id": 4, "method": "ping"}),
    );
    assert_eq!(refused.status, 401);
    assert_eq!(
        refused.header("www-authenticate"),
        Some("Bearer realm=\"lumatv\"")
    );
    assert_eq!(refused.json()["error"]["code"], "unauthorized");
    server.stop(Duration::from_millis(200));
}

#[test]
fn a_refused_pairing_is_a_403() {
    let server = start();
    let r = send(
        server.local_addr(),
        "POST",
        "/pair",
        &[],
        Some(r#"{"code":"000000","name":"Cody"}"#),
    );
    assert_eq!(r.status, 403);
    assert_eq!(r.json()["error"]["code"], "pairing_refused");
    server.stop(Duration::from_millis(200));
}

#[test]
fn a_browser_is_refused_even_with_a_valid_token() {
    let server = start();
    let addr = server.local_addr();
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let auth = ("Authorization", "Bearer tok-1");

    let page = send(
        addr,
        "POST",
        "/mcp",
        &[auth, ("Origin", "https://evil.example")],
        Some(&body),
    );
    assert_eq!(page.status, 403);
    assert_eq!(page.json()["error"]["code"], "origin_not_allowed");
    assert_eq!(page.header("access-control-allow-origin"), None);

    for site in ["cross-site", "same-origin", "none"] {
        let sub = send(
            addr,
            "POST",
            "/mcp",
            &[auth, ("Sec-Fetch-Site", site)],
            Some(&body),
        );
        assert_eq!(sub.status, 403, "{site}");
        assert_eq!(sub.json()["error"]["code"], "cross_site_not_allowed");
    }

    let preflight = http(
        addr,
        "OPTIONS",
        "/mcp",
        &[
            ("Origin", "https://evil.example"),
            ("Access-Control-Request-Method", "POST"),
        ],
        None,
    );
    assert_eq!(preflight.status, 403);

    let options = http(addr, "OPTIONS", "/mcp", &[], None);
    assert_eq!(options.status, 405);
    assert_eq!(options.header("allow"), Some("GET, POST"));
    server.stop(Duration::from_millis(200));
}

#[test]
fn methods_paths_and_media_types() {
    let server = start();
    let addr = server.local_addr();
    let auth = ("Authorization", "Bearer tok-1");

    for method in ["GET", "DELETE"] {
        let r = send(addr, method, "/mcp", &[auth], None);
        assert_eq!(r.status, 405, "{method}");
        if method == "GET" {
            assert_eq!(r.header("allow"), Some("POST"));
        } else {
            // DELETE never reaches the handler: the transport allows GET and POST only.
            assert_eq!(r.header("allow"), Some("GET, POST"));
        }
    }
    let missing = send(addr, "POST", "/nope", &[auth], Some("{}"));
    assert_eq!(missing.status, 404);
    assert_eq!(missing.json()["error"]["code"], "not_found");

    let text = http(
        addr,
        "POST",
        "/mcp",
        &[
            ("Authorization", "Bearer tok-1"),
            ("Content-Type", "text/plain"),
        ],
        Some(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#),
    );
    assert_eq!(text.status, 415);
    server.stop(Duration::from_millis(200));
}
