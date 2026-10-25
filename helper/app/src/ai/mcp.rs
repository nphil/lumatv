//! The MCP transport: Streamable HTTP with JSON-only answers, stateless (no session ids), plus the one-shot pairing endpoint.
//!
//! Routes: `POST /mcp` (JSON-RPC 2.0, bearer token required), `POST /pair` (a one-time code for a token), everything else
//! `404`; `GET`/`DELETE` on `/mcp` are `405` because the server never streams. This file decides nothing about permissions:
//! it parses, authenticates through [`Authenticator`] and hands tool calls to [`ToolHost`].
//!
//! Failures are counted per peer address (bad bearer, refused or malformed pairing): after 8 within 60 s that address is
//! answered `429` until its window has passed, success does not reset the count, so a host on the LAN cannot guess tokens or
//! pairing codes at speed. The table is in memory and bounded; when it is full the address with the oldest failure is dropped.
//! Tokens, codes, names, arguments and bodies are never logged.

use crate::API_VERSION;
use crate::ai::types::{Authenticator, PairFailure, ToolHost, ToolOutcome};
use crate::http::{Handler, Request, Response, error_response};
use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Protocol revisions this server speaks, newest first. The tool surface is identical in all three.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// The first revision with `structuredContent` in tool results.
const STRUCTURED_SINCE: &str = "2025-06-18";
const MAX_FAILURES: usize = 8;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);
const MAX_TRACKED_PEERS: usize = 256;
const MAX_CODE_CHARS: usize = 32;
const MAX_NAME_CHARS: usize = 100;
const MAX_TOKEN_BYTES: usize = 512;

const INSTRUCTIONS: &str = "These tools read and control a LumaTV television and its receiver: what is playing, the volume and power, \
launching apps and pressing transport keys. Some actions ask the owner for permission on the TV itself and wait up to 30 seconds for \
an answer from the remote control. If the owner denies a request, this assistant is paused and further calls are refused until the \
owner resumes it on the TV.";

/// What the server calls itself in `initialize`.
#[derive(Clone, Debug)]
pub struct ServerInfo {
    pub name: String,
    pub title: String,
    pub version: String,
}

type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// A JSON-RPC failure: code and a short plain message.
type RpcError = (i32, &'static str);

/// Recent failures per peer address.
#[derive(Default)]
struct Limiter {
    entries: Vec<(IpAddr, Vec<Instant>)>,
}

impl Limiter {
    fn prune(times: &mut Vec<Instant>, now: Instant) {
        times.retain(|t| now.saturating_duration_since(*t) < FAILURE_WINDOW);
    }

    fn blocked(&mut self, peer: IpAddr, now: Instant) -> bool {
        let Some(at) = self.entries.iter().position(|(ip, _)| *ip == peer) else {
            return false;
        };
        Self::prune(&mut self.entries[at].1, now);
        if self.entries[at].1.is_empty() {
            self.entries.swap_remove(at);
            return false;
        }
        self.entries[at].1.len() >= MAX_FAILURES
    }

    fn record(&mut self, peer: IpAddr, now: Instant) {
        let at = match self.entries.iter().position(|(ip, _)| *ip == peer) {
            Some(at) => at,
            None => {
                if self.entries.len() >= MAX_TRACKED_PEERS {
                    let oldest = self
                        .entries
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, (_, times))| times.last().copied())
                        .map(|(i, _)| i);
                    if let Some(i) = oldest {
                        self.entries.swap_remove(i);
                    }
                }
                self.entries.push((peer, Vec::new()));
                self.entries.len() - 1
            }
        };
        let times = &mut self.entries[at].1;
        Self::prune(times, now);
        times.push(now);
        if times.len() > MAX_FAILURES {
            times.remove(0);
        }
    }
}

/// The HTTP handler of the MCP endpoint.
pub struct McpHandler {
    host: Arc<dyn ToolHost>,
    auth: Arc<dyn Authenticator>,
    info: ServerInfo,
    clock: Clock,
    limiter: Mutex<Limiter>,
    /// Failure kinds already logged, as (address, kind): only the first of each is written.
    logged: Mutex<HashSet<(IpAddr, &'static str)>>,
}

impl McpHandler {
    pub fn new(host: Arc<dyn ToolHost>, auth: Arc<dyn Authenticator>, info: ServerInfo) -> Self {
        Self {
            host,
            auth,
            info,
            clock: Arc::new(Instant::now),
            limiter: Mutex::new(Limiter::default()),
            logged: Mutex::new(HashSet::new()),
        }
    }

    /// Replaces the clock the failure limiter reads (a test moves time by hand).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Counts one failure against `peer` and logs the first of its kind.
    fn fail(&self, peer: IpAddr, kind: &'static str) {
        self.limiter.lock().record(peer, (self.clock)());
        self.note(peer, kind);
    }

    fn note(&self, peer: IpAddr, kind: &'static str) {
        let mut logged = self.logged.lock();
        if logged.len() >= MAX_TRACKED_PEERS {
            logged.clear();
        }
        if logged.insert((peer, kind)) {
            crate::log_warn!("mcp: refused ({kind}) from {peer}");
        }
    }

    fn mcp(&self, request: &Request, peer: IpAddr) -> Response {
        let who =
            bearer(request).and_then(|token| self.auth.authenticate(token, request.peer).ok());
        let Some(who) = who else {
            self.fail(peer, "bad_bearer");
            return error_response(401, "unauthorized", "a valid bearer token is required")
                .with_header("WWW-Authenticate", "Bearer realm=\"lumatv\"");
        };
        let value: Value = match serde_json::from_slice(&request.body) {
            Ok(v) => v,
            Err(_) => return rpc_error(&Value::Null, -32700, "parse error"),
        };
        let object = match value {
            Value::Object(o) => o,
            Value::Array(_) => {
                return rpc_error(&Value::Null, -32600, "batches are not supported");
            }
            _ => {
                return rpc_error(&Value::Null, -32600, "the request must be a JSON object");
            }
        };
        let id = match object.get("id") {
            None => None,
            Some(v) if v.is_string() || v.is_i64() || v.is_u64() || v.is_null() => Some(v.clone()),
            Some(_) => {
                return rpc_error(
                    &Value::Null,
                    -32600,
                    "the id must be a string, an integer or null",
                );
            }
        };
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return rpc_error(
                id.as_ref().unwrap_or(&Value::Null),
                -32600,
                "jsonrpc must be \"2.0\"",
            );
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return rpc_error(
                id.as_ref().unwrap_or(&Value::Null),
                -32600,
                "method must be a string",
            );
        };
        // A message without an id is a notification: it is acknowledged and never executed, whatever its method.
        let Some(id) = id else {
            return Response {
                status: 202,
                headers: Vec::new(),
                body: Vec::new(),
            };
        };
        let params = object.get("params");
        let outcome = match method {
            "initialize" => self.initialize(params),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.tools_list(&who)),
            "tools/call" => self.tools_call(&who, request, params),
            _ => Err((-32601, "method not found")),
        };
        match outcome {
            Ok(result) => {
                Response::json(200, &json!({"jsonrpc": "2.0", "id": id, "result": result}))
            }
            Err((code, message)) => rpc_error(&id, code, message),
        }
    }

    fn initialize(&self, params: Option<&Value>) -> Result<Value, RpcError> {
        let asked = params
            .and_then(Value::as_object)
            .and_then(|p| p.get("protocolVersion"))
            .and_then(Value::as_str)
            .ok_or((-32602, "protocolVersion must be a string"))?;
        let version = PROTOCOL_VERSIONS
            .iter()
            .find(|v| **v == asked)
            .unwrap_or(&PROTOCOL_VERSIONS[0]);
        Ok(json!({
            "protocolVersion": version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {
                "name": self.info.name,
                "title": self.info.title,
                "version": self.info.version,
            },
            "instructions": INSTRUCTIONS,
        }))
    }

    fn tools_list(&self, who: &crate::ai::types::AssistantView) -> Value {
        let tools: Vec<Value> = self
            .host
            .list_tools(who)
            .into_iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "title": t.title,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                    "annotations": {
                        "readOnlyHint": t.read_only,
                        "destructiveHint": false,
                        "idempotentHint": t.read_only,
                        "openWorldHint": false,
                    },
                })
            })
            .collect();
        json!({"tools": tools})
    }

    fn tools_call(
        &self,
        who: &crate::ai::types::AssistantView,
        request: &Request,
        params: Option<&Value>,
    ) -> Result<Value, RpcError> {
        let params = params
            .and_then(Value::as_object)
            .ok_or((-32602, "params must be an object"))?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .ok_or((-32602, "name must be a non-empty string"))?;
        let none = Value::Object(Map::new());
        let arguments = match params.get("arguments") {
            None => &none,
            Some(v) if v.is_object() => v,
            Some(_) => return Err((-32602, "arguments must be an object")),
        };
        let structured = request
            .header("mcp-protocol-version")
            .is_some_and(at_least_structured);
        Ok(
            match self.host.call_tool(who, request.peer, name, arguments) {
                ToolOutcome::Ok { text, data } => {
                    let has_data = data.as_object().is_some_and(|o| !o.is_empty());
                    let text = if has_data {
                        format!(
                            "{text}\n{}",
                            serde_json::to_string(&data).unwrap_or_default()
                        )
                    } else {
                        text
                    };
                    let mut result =
                        json!({"content": [{"type": "text", "text": text}], "isError": false});
                    if structured && data.is_object() {
                        result["structuredContent"] = data;
                    }
                    result
                }
                ToolOutcome::Err { code, message } => json!({
                    "content": [{"type": "text", "text": format!("{code}: {message}")}],
                    "isError": true,
                }),
            },
        )
    }

    fn pair(&self, request: &Request, peer: IpAddr) -> Response {
        let Some((code, name)) = pair_body(&request.body) else {
            self.fail(peer, "pairing_malformed");
            return error_response(
                400,
                "bad_request",
                "send {\"code\": ..., \"name\": ...} as JSON",
            );
        };
        match self.auth.pair(&code, &name, request.peer) {
            Ok(paired) => Response::json(
                200,
                &json!({
                    "ok": true,
                    "api": API_VERSION,
                    "assistantId": paired.assistant_id,
                    "token": paired.token,
                    "mcpPath": "/mcp",
                }),
            ),
            Err(PairFailure::Refused) => {
                self.fail(peer, "pairing_refused");
                error_response(403, "pairing_refused", "pairing was refused")
            }
            Err(PairFailure::TooManyAttempts) => {
                self.fail(peer, "pairing_limited");
                too_many().with_header("Retry-After", "60")
            }
            Err(PairFailure::BadName) => {
                self.fail(peer, "pairing_bad_name");
                error_response(400, "bad_name", "the name must not be empty")
            }
        }
    }
}

impl Handler for McpHandler {
    fn handle(&self, request: &Request) -> Response {
        let peer = request.peer.to_canonical();
        if self.limiter.lock().blocked(peer, (self.clock)()) {
            self.note(peer, "rate_limited");
            return too_many().with_header("Retry-After", "60");
        }
        let route = match request.path.as_str() {
            "/mcp" => Self::mcp,
            "/pair" => Self::pair,
            _ => return error_response(404, "not_found", "no such path"),
        };
        if request.method != "POST" {
            return error_response(405, "method_not_allowed", "use POST")
                .with_header("Allow", "POST");
        }
        route(self, request, peer)
    }
}

fn too_many() -> Response {
    error_response(
        429,
        "too_many_attempts",
        "too many failed attempts; try again later",
    )
}

/// The token of `Authorization: Bearer <token>`; `None` for anything else.
fn bearer(request: &Request) -> Option<&str> {
    let (scheme, token) = request.header("authorization")?.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer")
        && !token.is_empty()
        && token.len() <= MAX_TOKEN_BYTES
        && !token.chars().any(char::is_whitespace))
    .then_some(token)
}

/// Whether an `MCP-Protocol-Version` value names a revision that has `structuredContent`. Dates compare as text.
fn at_least_structured(version: &str) -> bool {
    version.len() == 10
        && version.bytes().all(|b| b.is_ascii_digit() || b == b'-')
        && version >= STRUCTURED_SINCE
}

/// `{"code": string, "name": string}` and nothing else, within the length limits.
fn pair_body(body: &[u8]) -> Option<(String, String)> {
    let Value::Object(object) = serde_json::from_slice::<Value>(body).ok()? else {
        return None;
    };
    if object.len() != 2 {
        return None;
    }
    let code = object.get("code")?.as_str()?;
    let name = object.get("name")?.as_str()?;
    (code.chars().count() <= MAX_CODE_CHARS && name.chars().count() <= MAX_NAME_CHARS)
        .then(|| (code.to_owned(), name.to_owned()))
}

fn rpc_error(id: &Value, code: i32, message: &str) -> Response {
    Response::json(
        200,
        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::{AssistantView, AuthFailure, Paired, ToolDef};

    const A: &str = "192.168.1.20";
    const B: &str = "192.168.1.21";

    #[derive(Default)]
    struct FakeHost {
        calls: Mutex<Vec<(String, IpAddr, String, Value)>>,
    }

    impl ToolHost for FakeHost {
        fn list_tools(&self, _who: &AssistantView) -> Vec<ToolDef> {
            vec![
                ToolDef {
                    name: "get_thing",
                    title: "Get a thing",
                    description: "Reads a thing.".into(),
                    input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                    read_only: true,
                },
                ToolDef {
                    name: "do_thing",
                    title: "Do a thing",
                    description: "Does a thing.".into(),
                    input_schema: json!({"type": "object", "properties": {"n": {"type": "integer"}}, "additionalProperties": false}),
                    read_only: false,
                },
            ]
        }

        fn call_tool(
            &self,
            who: &AssistantView,
            peer: IpAddr,
            name: &str,
            arguments: &Value,
        ) -> ToolOutcome {
            self.calls
                .lock()
                .push((who.id.clone(), peer, name.to_owned(), arguments.clone()));
            match name {
                "get_thing" => ToolOutcome::Ok {
                    text: "done".into(),
                    data: json!({"x": 1}),
                },
                "quiet" => ToolOutcome::Ok {
                    text: "done".into(),
                    data: json!({}),
                },
                "denied" => ToolOutcome::Err {
                    code: "denied",
                    message: "the owner said no".into(),
                },
                _ => ToolOutcome::Err {
                    code: "not_found",
                    message: "no such tool".into(),
                },
            }
        }
    }

    #[derive(Default)]
    struct FakeAuth {
        authenticated: Mutex<Vec<(String, IpAddr)>>,
        paired: Mutex<Vec<(String, String, IpAddr)>>,
    }

    impl Authenticator for FakeAuth {
        fn authenticate(&self, bearer: &str, peer: IpAddr) -> Result<AssistantView, AuthFailure> {
            self.authenticated.lock().push((bearer.to_owned(), peer));
            if bearer == "good" {
                Ok(AssistantView {
                    id: "a1".into(),
                    name: "Cody".into(),
                })
            } else {
                Err(AuthFailure::Unknown)
            }
        }

        fn pair(&self, code: &str, name: &str, peer: IpAddr) -> Result<Paired, PairFailure> {
            self.paired
                .lock()
                .push((code.to_owned(), name.to_owned(), peer));
            match code {
                "123456" => Ok(Paired {
                    assistant_id: "a2".into(),
                    token: "fresh-token".into(),
                }),
                "locked" => Err(PairFailure::TooManyAttempts),
                "noname" => Err(PairFailure::BadName),
                _ => Err(PairFailure::Refused),
            }
        }
    }

    struct Rig {
        handler: McpHandler,
        host: Arc<FakeHost>,
        auth: Arc<FakeAuth>,
        offset: Arc<Mutex<Duration>>,
    }

    fn rig() -> Rig {
        let host = Arc::new(FakeHost::default());
        let auth = Arc::new(FakeAuth::default());
        let offset = Arc::new(Mutex::new(Duration::ZERO));
        let base = Instant::now();
        let clock_offset = offset.clone();
        let handler = McpHandler::new(
            host.clone(),
            auth.clone(),
            ServerInfo {
                name: "lumatv".into(),
                title: "LumaTV".into(),
                version: "9.9.9".into(),
            },
        )
        .with_clock(Arc::new(move || base + *clock_offset.lock()));
        Rig {
            handler,
            host,
            auth,
            offset,
        }
    }

    fn request(
        method: &str,
        path: &str,
        peer: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: vec![],
            headers: headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
            body: body.as_bytes().to_vec(),
            peer: peer.parse().unwrap(),
        }
    }

    fn json_body(r: &Response) -> Value {
        serde_json::from_slice(&r.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&r.body)))
    }

    fn header<'a>(r: &'a Response, name: &str) -> Option<&'a str> {
        r.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// An authenticated `POST /mcp` from `peer`.
    fn rpc_from(rig: &Rig, peer: &str, extra: &[(&str, &str)], body: &str) -> Response {
        let mut headers = vec![("authorization", "Bearer good")];
        headers.extend_from_slice(extra);
        rig.handler
            .handle(&request("POST", "/mcp", peer, &headers, body))
    }

    fn rpc(rig: &Rig, body: &str) -> Response {
        rpc_from(rig, A, &[], body)
    }

    /// The `result` of a successful JSON-RPC answer.
    fn result(r: &Response) -> Value {
        assert_eq!(r.status, 200);
        let body = json_body(r);
        assert_eq!(body["jsonrpc"], "2.0");
        assert!(body.get("error").is_none(), "{body}");
        body["result"].clone()
    }

    /// The `error` of a JSON-RPC answer: (code, message).
    fn rpc_failure(r: &Response) -> (i64, String, Value) {
        assert_eq!(r.status, 200);
        let body = json_body(r);
        assert_eq!(body["jsonrpc"], "2.0");
        (
            body["error"]["code"].as_i64().unwrap(),
            body["error"]["message"].as_str().unwrap().to_owned(),
            body["id"].clone(),
        )
    }

    fn call(name: &str, arguments: Option<Value>) -> String {
        let mut params = json!({"name": name});
        if let Some(a) = arguments {
            params["arguments"] = a;
        }
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params}).to_string()
    }

    #[test]
    fn initialize_negotiates_the_protocol_version() {
        let rig = rig();
        for (asked, answered) in [
            ("2025-06-18", "2025-06-18"),
            ("2025-03-26", "2025-03-26"),
            ("2024-11-05", "2024-11-05"),
            ("2099-01-01", "2025-06-18"),
            ("2024-01-01", "2025-06-18"),
            ("nonsense", "2025-06-18"),
            ("", "2025-06-18"),
        ] {
            let body = json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": asked, "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}},
            })
            .to_string();
            let r = result(&rpc(&rig, &body));
            assert_eq!(r["protocolVersion"], answered, "{asked:?}");
            assert_eq!(r["capabilities"], json!({"tools": {"listChanged": false}}));
            assert_eq!(
                r["serverInfo"],
                json!({"name": "lumatv", "title": "LumaTV", "version": "9.9.9"})
            );
            let instructions = r["instructions"].as_str().unwrap();
            assert!(instructions.contains("30 seconds") && !instructions.contains('\n'));
        }
        for bad in [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":5}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":[]}"#,
        ] {
            assert_eq!(rpc_failure(&rpc(&rig, bad)).0, -32602, "{bad}");
        }
    }

    #[test]
    fn notifications_are_accepted_without_a_body_and_never_run() {
        let rig = rig();
        for body in [
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
            r#"{"jsonrpc":"2.0","method":"no/such/method"}"#,
            r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"get_thing"}}"#,
            r#"{"jsonrpc":"2.0","method":"ping"}"#,
        ] {
            let r = rpc(&rig, body);
            assert_eq!(r.status, 202, "{body}");
            assert!(r.body.is_empty(), "{body}");
        }
        assert!(rig.host.calls.lock().is_empty());
    }

    #[test]
    fn ping_answers_an_empty_object() {
        let rig = rig();
        let r = result(&rpc(&rig, r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#));
        assert_eq!(r, json!({}));
    }

    #[test]
    fn tools_list_carries_schemas_and_annotations() {
        let rig = rig();
        let r = result(&rpc(
            &rig,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        ));
        let tools = r["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(
            tools[0],
            json!({
                "name": "get_thing",
                "title": "Get a thing",
                "description": "Reads a thing.",
                "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
                "annotations": {
                    "readOnlyHint": true,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": false,
                },
            })
        );
        assert_eq!(tools[1]["annotations"]["readOnlyHint"], false);
        assert_eq!(tools[1]["annotations"]["idempotentHint"], false);
        assert_eq!(tools[1]["annotations"]["destructiveHint"], false);
        assert_eq!(
            tools[1]["inputSchema"]["properties"]["n"]["type"],
            "integer"
        );
    }

    #[test]
    fn tools_call_returns_text_and_data() {
        let rig = rig();
        let r = result(&rpc(&rig, &call("get_thing", Some(json!({"k": "v"})))));
        assert_eq!(r["isError"], false);
        assert_eq!(r["content"][0]["type"], "text");
        assert_eq!(r["content"][0]["text"], "done\n{\"x\":1}");
        assert!(r.get("structuredContent").is_none());
        let calls = rig.host.calls.lock();
        assert_eq!(
            calls[0],
            (
                "a1".to_owned(),
                A.parse::<IpAddr>().unwrap(),
                "get_thing".to_owned(),
                json!({"k": "v"})
            )
        );
        drop(calls);
        // An empty data object adds nothing to the text.
        let r = result(&rpc(&rig, &call("quiet", None)));
        assert_eq!(r["content"][0]["text"], "done");
    }

    #[test]
    fn missing_arguments_default_to_an_empty_object() {
        let rig = rig();
        result(&rpc(&rig, &call("get_thing", None)));
        assert_eq!(rig.host.calls.lock()[0].3, json!({}));
    }

    #[test]
    fn structured_content_needs_the_protocol_header() {
        let rig = rig();
        let with = |version: &str| {
            result(&rpc_from(
                &rig,
                A,
                &[("mcp-protocol-version", version)],
                &call("get_thing", None),
            ))
        };
        assert_eq!(with("2025-06-18")["structuredContent"], json!({"x": 1}));
        assert_eq!(with("2026-01-01")["structuredContent"], json!({"x": 1}));
        assert!(with("2025-03-26").get("structuredContent").is_none());
        assert!(with("2024-11-05").get("structuredContent").is_none());
        assert!(with("garbage!!!").get("structuredContent").is_none());
        // A refusal never carries structured content.
        let r = result(&rpc_from(
            &rig,
            A,
            &[("mcp-protocol-version", "2025-06-18")],
            &call("denied", None),
        ));
        assert!(r.get("structuredContent").is_none());
    }

    #[test]
    fn a_refused_tool_is_an_error_result_not_a_protocol_error() {
        let rig = rig();
        let r = result(&rpc(&rig, &call("denied", None)));
        assert_eq!(r["isError"], true);
        assert_eq!(r["content"][0]["text"], "denied: the owner said no");
        let r = result(&rpc(&rig, &call("nonexistent", None)));
        assert_eq!(r["isError"], true);
        assert_eq!(r["content"][0]["text"], "not_found: no such tool");
    }

    #[test]
    fn invalid_tool_call_params_are_32602() {
        let rig = rig();
        for body in [
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#.to_owned(),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":[]}"#.to_owned(),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#.to_owned(),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":5}}"#.to_owned(),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":""}}"#.to_owned(),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"x","arguments":[1]}}"#.to_owned(),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"x","arguments":null}}"#.to_owned(),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"x","arguments":"s"}}"#.to_owned(),
        ] {
            let (code, message, _) = rpc_failure(&rpc(&rig, &body));
            assert_eq!(code, -32602, "{body}");
            assert!(message.len() < 80 && !message.contains('{'), "{message}");
        }
        assert!(rig.host.calls.lock().is_empty());
    }

    #[test]
    fn malformed_json_rpc_is_answered_in_json_rpc() {
        let rig = rig();
        let (code, _, id) = rpc_failure(&rpc(&rig, "{not json"));
        assert_eq!((code, id), (-32700, Value::Null));
        let (code, _, id) = rpc_failure(&rpc(&rig, ""));
        assert_eq!((code, id), (-32700, Value::Null));
        let (code, message, id) =
            rpc_failure(&rpc(&rig, r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#));
        assert_eq!(
            (code, message.as_str(), id),
            (-32600, "batches are not supported", Value::Null)
        );
        for body in ["5", "\"x\"", "null", "true"] {
            assert_eq!(rpc_failure(&rpc(&rig, body)).0, -32600, "{body}");
        }
        for body in [
            r#"{"id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
            r#"{"jsonrpc":2,"id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":7}"#,
        ] {
            let (code, _, id) = rpc_failure(&rpc(&rig, body));
            assert_eq!((code, id), (-32600, json!(1)), "{body}");
        }
        for body in [
            r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":[1],"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#,
        ] {
            let (code, _, id) = rpc_failure(&rpc(&rig, body));
            assert_eq!((code, id), (-32600, Value::Null), "{body}");
        }
        let (code, message, id) = rpc_failure(&rpc(
            &rig,
            r#"{"jsonrpc":"2.0","id":4,"method":"resources/list"}"#,
        ));
        assert_eq!(
            (code, message.as_str(), id),
            (-32601, "method not found", json!(4))
        );
    }

    #[test]
    fn ids_are_echoed_as_given() {
        let rig = rig();
        for id in [
            json!("abc-1"),
            json!(0),
            json!(42),
            json!(-3),
            json!(u64::MAX),
            Value::Null,
        ] {
            let body = json!({"jsonrpc": "2.0", "id": id, "method": "ping"}).to_string();
            let answer = json_body(&rpc(&rig, &body));
            assert_eq!(answer["id"], id, "{body}");
        }
        let (_, _, id) = rpc_failure(&rpc(&rig, r#"{"jsonrpc":"2.0","id":"x","method":"nope"}"#));
        assert_eq!(id, json!("x"));
    }

    #[test]
    fn a_missing_or_wrong_bearer_is_one_401() {
        let rig = rig();
        let cases: Vec<Vec<(&str, &str)>> = vec![
            vec![],
            vec![("authorization", "Basic Zm9vOmJhcg==")],
            vec![("authorization", "Bearer")],
            vec![("authorization", "good")],
            vec![("authorization", "Bearer nope")],
            vec![("authorization", "Bearer good extra")],
            vec![("authorization", "Bearer  good")],
        ];
        for headers in cases {
            let r = rig.handler.handle(&request(
                "POST",
                "/mcp",
                A,
                &headers,
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ));
            assert_eq!(r.status, 401, "{headers:?}");
            assert_eq!(
                header(&r, "www-authenticate"),
                Some("Bearer realm=\"lumatv\""),
                "{headers:?}"
            );
            let body = json_body(&r);
            assert_eq!(body["error"]["code"], "unauthorized");
            assert_eq!(body["ok"], false);
        }
        // The token reaches the authenticator with the peer; a case-insensitive scheme is accepted.
        let r = rig.handler.handle(&request(
            "POST",
            "/mcp",
            A,
            &[("authorization", "bearer good")],
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
        ));
        assert_eq!(r.status, 200);
        let seen = rig.auth.authenticated.lock();
        assert_eq!(
            seen.last().unwrap(),
            &("good".to_owned(), A.parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn eight_failures_lock_one_address_for_a_minute() {
        let rig = rig();
        let bad = |peer: &str| {
            rig.handler.handle(&request(
                "POST",
                "/mcp",
                peer,
                &[("authorization", "Bearer nope")],
                "{}",
            ))
        };
        for _ in 0..8 {
            assert_eq!(bad(A).status, 401);
        }
        // Even a good token is refused now, with a plain 429.
        let r = rpc_from(&rig, A, &[], r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
        assert_eq!(r.status, 429);
        assert_eq!(header(&r, "retry-after"), Some("60"));
        assert_eq!(json_body(&r)["error"]["code"], "too_many_attempts");
        assert_eq!(bad(A).status, 429);
        // Another address is unaffected.
        assert_eq!(
            rpc_from(&rig, B, &[], r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).status,
            200
        );
        // The other paths are locked too.
        let r = rig.handler.handle(&request(
            "POST",
            "/pair",
            A,
            &[],
            r#"{"code":"123456","name":"x"}"#,
        ));
        assert_eq!(r.status, 429);
        assert!(rig.auth.paired.lock().is_empty());
        // Requests refused with 429 do not extend the window.
        *rig.offset.lock() = Duration::from_secs(59);
        assert_eq!(bad(A).status, 429);
        *rig.offset.lock() = Duration::from_secs(61);
        assert_eq!(
            rpc_from(&rig, A, &[], r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).status,
            200
        );
    }

    #[test]
    fn success_does_not_reset_the_failure_count() {
        let rig = rig();
        let ping = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let bad = || {
            rig.handler.handle(&request(
                "POST",
                "/mcp",
                A,
                &[("authorization", "Bearer nope")],
                "{}",
            ))
        };
        for _ in 0..4 {
            assert_eq!(bad().status, 401);
        }
        assert_eq!(rpc(&rig, ping).status, 200);
        for _ in 0..4 {
            assert_eq!(bad().status, 401);
        }
        assert_eq!(rpc(&rig, ping).status, 429);
    }

    #[test]
    fn failures_age_out_one_by_one() {
        let rig = rig();
        let ping = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let bad = || {
            rig.handler.handle(&request(
                "POST",
                "/mcp",
                A,
                &[("authorization", "Bearer nope")],
                "{}",
            ))
        };
        for step in 0..8u64 {
            *rig.offset.lock() = Duration::from_secs(step * 5);
            assert_eq!(bad().status, 401);
        }
        // The oldest failure (at 0 s) leaves the window at 60 s, so seven remain and the address may try again.
        *rig.offset.lock() = Duration::from_secs(59);
        assert_eq!(rpc(&rig, ping).status, 429);
        *rig.offset.lock() = Duration::from_secs(60);
        assert_eq!(rpc(&rig, ping).status, 200);
    }

    #[test]
    fn the_failure_table_is_bounded() {
        let rig = rig();
        for i in 0..(MAX_TRACKED_PEERS as u32 + 50) {
            *rig.offset.lock() = Duration::from_millis(u64::from(i));
            let peer = format!("10.1.{}.{}", i / 250, i % 250 + 1);
            rig.handler.handle(&request(
                "POST",
                "/mcp",
                &peer,
                &[("authorization", "Bearer nope")],
                "{}",
            ));
        }
        assert!(rig.handler.limiter.lock().entries.len() <= MAX_TRACKED_PEERS);
    }

    #[test]
    fn pairing_hands_out_a_token() {
        let rig = rig();
        let r = rig.handler.handle(&request(
            "POST",
            "/pair",
            A,
            &[],
            r#"{"code":"123456","name":"Home Assistant"}"#,
        ));
        assert_eq!(r.status, 200);
        assert_eq!(
            json_body(&r),
            json!({"ok": true, "api": 1, "assistantId": "a2", "token": "fresh-token", "mcpPath": "/mcp"})
        );
        assert_eq!(
            rig.auth.paired.lock()[0],
            (
                "123456".to_owned(),
                "Home Assistant".to_owned(),
                A.parse::<IpAddr>().unwrap()
            )
        );
    }

    #[test]
    fn pairing_failures_map_to_their_statuses() {
        let rig = rig();
        let pair = |body: &str| rig.handler.handle(&request("POST", "/pair", B, &[], body));
        let r = pair(r#"{"code":"000000","name":"x"}"#);
        assert_eq!(
            (r.status, json_body(&r)["error"]["code"].clone()),
            (403, json!("pairing_refused"))
        );
        let r = pair(r#"{"code":"locked","name":"x"}"#);
        assert_eq!(
            (r.status, json_body(&r)["error"]["code"].clone()),
            (429, json!("too_many_attempts"))
        );
        assert_eq!(header(&r, "retry-after"), Some("60"));
        let r = pair(r#"{"code":"noname","name":"x"}"#);
        assert_eq!(
            (r.status, json_body(&r)["error"]["code"].clone()),
            (400, json!("bad_name"))
        );
    }

    #[test]
    fn pairing_bodies_are_checked_before_the_module_sees_them() {
        let rig = rig();
        let long_code = format!(r#"{{"code":"{}","name":"x"}}"#, "1".repeat(33));
        let long_name = format!(r#"{{"code":"1","name":"{}"}}"#, "n".repeat(101));
        let edge = format!(
            r#"{{"code":"{}","name":"{}"}}"#,
            "1".repeat(32),
            "é".repeat(100)
        );
        for (i, bad) in [
            "",
            "{",
            "[]",
            "null",
            r#"{"code":"1"}"#,
            r#"{"name":"x"}"#,
            r#"{"code":1,"name":"x"}"#,
            r#"{"code":"1","name":null}"#,
            r#"{"code":"1","name":"x","extra":true}"#,
            r#"{"code":"1","name":"x","token":"t"}"#,
            long_code.as_str(),
            long_name.as_str(),
        ]
        .into_iter()
        .enumerate()
        {
            // One address per case: the failure limiter would otherwise answer 429 after the eighth.
            let peer = format!("10.2.0.{}", i + 1);
            let r = rig
                .handler
                .handle(&request("POST", "/pair", &peer, &[], bad));
            assert_eq!(r.status, 400, "{bad:?}");
            assert_eq!(json_body(&r)["error"]["code"], "bad_request");
        }
        assert!(rig.auth.paired.lock().is_empty());
        // The limits are inclusive, and a character is not a byte.
        let r = rig.handler.handle(&request("POST", "/pair", B, &[], &edge));
        assert_eq!(r.status, 403);
        assert_eq!(rig.auth.paired.lock().len(), 1);
    }

    #[test]
    fn every_failed_pairing_counts_against_the_address() {
        let rig = rig();
        let pair = |peer: &str, body: &str| {
            rig.handler
                .handle(&request("POST", "/pair", peer, &[], body))
                .status
        };
        for _ in 0..4 {
            assert_eq!(pair(A, r#"{"code":"000000","name":"x"}"#), 403);
        }
        for _ in 0..4 {
            assert_eq!(pair(A, "garbage"), 400);
        }
        assert_eq!(pair(A, r#"{"code":"123456","name":"x"}"#), 429);
        assert_eq!(pair(B, r#"{"code":"123456","name":"x"}"#), 200);
        assert_eq!(rig.auth.paired.lock().len(), 5);
        // A success never counts.
        let fresh = super::tests::rig();
        for _ in 0..20 {
            assert_eq!(
                fresh
                    .handler
                    .handle(&request(
                        "POST",
                        "/pair",
                        A,
                        &[],
                        r#"{"code":"123456","name":"x"}"#
                    ))
                    .status,
                200
            );
        }
    }

    #[test]
    fn routes_and_methods() {
        let rig = rig();
        for method in ["GET", "DELETE", "PUT", "PATCH"] {
            for path in ["/mcp", "/pair"] {
                let r = rig.handler.handle(&request(
                    method,
                    path,
                    A,
                    &[("authorization", "Bearer good")],
                    "",
                ));
                assert_eq!(r.status, 405, "{method} {path}");
                assert_eq!(header(&r, "allow"), Some("POST"));
                assert_eq!(json_body(&r)["error"]["code"], "method_not_allowed");
            }
        }
        for path in [
            "/",
            "/mcp/",
            "/v1/health",
            "/MCP",
            "/pair/x",
            "/.well-known/oauth-authorization-server",
        ] {
            let r = rig.handler.handle(&request(
                "POST",
                path,
                A,
                &[("authorization", "Bearer good")],
                "{}",
            ));
            assert_eq!(r.status, 404, "{path}");
            assert_eq!(json_body(&r)["error"]["code"], "not_found");
        }
        // Wrong methods and unknown paths are not failures.
        assert!(rig.handler.limiter.lock().entries.is_empty());
        assert!(rig.auth.authenticated.lock().is_empty());
    }

    #[test]
    fn answers_do_not_leak_internals() {
        let rig = rig();
        for body in ["{nope", "[1]", r#"{"jsonrpc":"2.0","id":1,"method":"x"}"#] {
            let text = String::from_utf8(rpc(&rig, body).body).unwrap();
            for needle in ["Error", "panicked", "src/", "serde", "line ", "column"] {
                assert!(!text.contains(needle), "{text}");
            }
        }
    }
}
