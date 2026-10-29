//! AI access end to end, over real TCP, with a remote the test presses by hand and a TV and receiver that record what they are told.
//!
//! What these tests hold the module to (Principle 0): it is off until a physical remote turns it on; no API path of the helper
//! turns it on, answers a prompt, creates a token or widens a permission; every action needs the level that allows it; a lock-out
//! takes everything away at once; and nothing an assistant reaches over the LAN can touch the TV's own admin API.

mod support;

use lumatv_helper::ai::Deps;
use serde_json::{Value, json};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);

struct AiRig {
    rig: Rig,
    keys: Arc<ScriptedKeys>,
    tv: Arc<FakeTv>,
    receiver: Arc<FakeReceiver>,
}

/// A port nothing listens on, different for every caller in this process: asking the system for port 0 and handing it back
/// would give two tests running side by side the same number, and the system's own client sockets draw from the same range.
fn free_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(0);
    // Below the system's ephemeral range (32768 and up), and a different stretch for every run.
    let base = 20_000 + u16::try_from(std::process::id() % 8_000).unwrap_or(0);
    loop {
        let port = base + NEXT.fetch_add(1, Ordering::SeqCst);
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

/// Starts the service with AI access configured to bind the loopback (a private LAN address cannot be assumed on a test host).
fn start(port: u16) -> AiRig {
    let rig = Rig::new();
    receiver_off(&rig.paths);
    start_in(rig, port)
}

fn start_in(mut rig: Rig, port: u16) -> AiRig {
    let keys = Arc::new(ScriptedKeys::default());
    let tv = Arc::new(FakeTv::default());
    let receiver = Arc::new(FakeReceiver::default());
    let config =
        test_config(json!({"ai": {"bind": "127.0.0.1", "port": port, "promptSeconds": 5}}));
    rig.start_with(
        config,
        Script::new(),
        Some(Deps {
            keys: keys.clone(),
            tv: tv.clone(),
            receiver: receiver.clone(),
        }),
    );
    AiRig {
        rig,
        keys,
        tv,
        receiver,
    }
}

impl AiRig {
    /// Stops the helper completely and starts it again on the same data.
    fn restart(self, port: u16) -> AiRig {
        let mut rig = self.rig;
        rig.stop();
        start_in(rig, port)
    }

    fn doc(&self) -> Value {
        get(self.rig.addr, "/v1/ai").json()["ai"].clone()
    }

    fn wait_doc(&self, check: impl Fn(&Value) -> bool) -> Value {
        let mut last = Value::Null;
        assert!(
            wait_until(WAIT, || {
                last = self.doc();
                check(&last)
            }),
            "the ai state never became what was waited for: {last}"
        );
        last
    }

    fn prompt(&self) -> Value {
        get(self.rig.addr, "/v1/state/ai.prompt")
            .json()
            .get("value")
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn wait_prompt(&self) -> Value {
        let mut last = Value::Null;
        assert!(
            wait_until(WAIT, || {
                last = self.prompt();
                !last.is_null()
            }),
            "no prompt appeared"
        );
        last
    }

    fn wait_no_prompt(&self) {
        assert!(
            wait_until(WAIT, || self.prompt().is_null()),
            "the prompt never closed"
        );
    }

    /// Presses a key on a device after the prompt has been up long enough to count.
    fn press(&self, device: &str, code: u16) {
        thread::sleep(Duration::from_millis(450));
        self.keys.tap(device, code);
    }

    fn api(&self, method: &str, path: &str, body: &str) -> Reply {
        http(
            self.rig.addr,
            method,
            path,
            &[
                ("X-Luma-Client", "test"),
                ("Content-Type", "application/json"),
            ],
            if body.is_empty() && method != "POST" {
                None
            } else {
                Some(if body.is_empty() { "{}" } else { body })
            },
        )
    }

    /// The whole turn-it-on flow: ask, see the prompt, press OK on the IR remote, wait for the socket.
    fn turn_on(&self) -> SocketAddr {
        assert_eq!(
            self.api("POST", "/v1/ai/listener", r#"{"enabled":true}"#)
                .status,
            202
        );
        self.wait_prompt();
        self.press(IR_REMOTE, KEY_OK);
        let doc = self.wait_doc(|d| d["listener"]["bound"] == true);
        doc["listener"]["addr"].as_str().unwrap().parse().unwrap()
    }

    /// Pairs an assistant: ask, OK on the remote, read the code off the TV, present it on the LAN.
    fn pair(&self, lan: SocketAddr, name: &str) -> (String, String) {
        assert_eq!(self.api("POST", "/v1/ai/pairing", "{}").status, 202);
        self.wait_prompt();
        self.press(MAGIC_REMOTE, KEY_OK);
        let doc = self.wait_doc(|d| !d["pairing"].is_null());
        let code = doc["pairing"]["code"].as_str().unwrap().to_owned();
        let reply = lan_post(
            lan,
            "/pair",
            None,
            &json!({"code": code, "name": name}).to_string(),
        );
        assert_eq!(
            reply.status,
            200,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        let body = reply.json();
        (
            body["token"].as_str().unwrap().to_owned(),
            body["assistantId"].as_str().unwrap().to_owned(),
        )
    }
}

fn lan_post(addr: SocketAddr, path: &str, token: Option<&str>, body: &str) -> Reply {
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut headers = vec![("Content-Type", "application/json")];
    if let Some(auth) = &auth {
        headers.push(("Authorization", auth));
    }
    http(addr, "POST", path, &headers, Some(body))
}

/// One MCP request: returns the JSON-RPC response body.
fn mcp(addr: SocketAddr, token: &str, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    let reply = lan_post(addr, "/mcp", Some(token), &body);
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    reply.json()
}

fn call(addr: SocketAddr, token: &str, tool: &str, arguments: Value) -> Value {
    mcp(
        addr,
        token,
        "tools/call",
        json!({"name": tool, "arguments": arguments}),
    )["result"]
        .clone()
}

fn is_error(result: &Value) -> bool {
    result["isError"] == true
}

fn error_text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn port_open(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

// ------------------------------------------------------------------------------------------------ off by default

#[test]
fn off_by_default_nothing_is_bound_and_the_state_says_so() {
    let port = free_port();
    let ai = start(port);
    let doc = ai.doc();
    assert_eq!(doc["listener"]["enabled"], false);
    assert_eq!(doc["listener"]["bound"], false);
    assert_eq!(doc["listener"]["addr"], Value::Null);
    assert_eq!(doc["assistants"], json!([]));
    assert_eq!(doc["pairing"], Value::Null);
    assert_eq!(doc["limits"]["maxVolume"], 50.0);
    assert_eq!(ai.prompt(), Value::Null);
    assert!(
        !port_open(format!("127.0.0.1:{port}").parse().unwrap()),
        "something listens on the AI port with AI access off"
    );
    assert_eq!(
        ai.keys.opens(),
        0,
        "the remote's devices are not opened until a prompt is up"
    );
    // The state lists what exists to be given: four groups, thirteen tools.
    assert_eq!(doc["groups"].as_array().unwrap().len(), 4);
    assert_eq!(doc["tools"].as_array().unwrap().len(), 13);
    assert!(
        !ai.rig.paths.data_dir.join("ai/state.json").exists(),
        "nothing is written until something changes"
    );
}

#[test]
fn the_helper_binary_starts_with_ai_access_off_and_nothing_listening() {
    use std::process::Command;
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    std::fs::write(
        dir.path().join("helper.json"),
        json!({"listen": {"port": 0}, "screen": {"source": "on"}, "fetch": {"openssl": "/nonexistent/openssl"}, "nice": 0,
               "ai": {"bind": "127.0.0.1", "port": port}})
        .to_string(),
    )
    .unwrap();
    receiver_off(&lumatv_helper::config::Paths::new(dir.path()));
    let bin = env!("CARGO_BIN_EXE_lumatv-helper");
    let started = Command::new(bin)
        .args(["start", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    let started: Value = serde_json::from_slice(&started.stdout).unwrap();
    assert_eq!(started["started"], true, "{started}");
    let api: SocketAddr = format!("127.0.0.1:{}", started["port"]).parse().unwrap();
    let doc = get(api, "/v1/ai").json();
    assert_eq!(doc["ai"]["listener"]["enabled"], false);
    assert!(!port_open(format!("127.0.0.1:{port}").parse().unwrap()));
    let stopped = Command::new(bin)
        .args(["stop", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&stopped.stdout).contains("\"stopped\":true"));
}

// ------------------------------------------------------------------------------------------------ only the remote turns it on

#[test]
fn no_api_path_turns_it_on_and_the_remote_does() {
    let port = free_port();
    let ai = start(port);
    let lan: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    // Every loopback request that could conceivably be taken for "turn it on", in every method, with every body.
    let bodies = [
        "",
        "{}",
        r#"{"enabled":true}"#,
        r#"{"listener":{"enabled":true}}"#,
        r#"{"ai":{"listener":true}}"#,
        r#"{"on":true}"#,
        "true",
    ];
    let paths = [
        "/v1/ai",
        "/v1/ai/listener",
        "/v1/ai/enable",
        "/v1/ai/listener/enable",
        "/v1/ai/state",
        "/v1/state",
        "/v1/settings",
        "/v1/settings/ai",
        "/v1/settings/ai.listener",
        "/v1/settings/base.receiver",
        "/v1/refresh",
    ];
    for path in paths {
        for method in ["GET", "POST", "PUT", "DELETE", "PATCH"] {
            for body in bodies {
                // `/v1/ai/listener` with `enabled: true` is the one request that asks (and only opens a prompt).
                let asking = path == "/v1/ai/listener"
                    && method == "POST"
                    && body.contains("\"enabled\":true");
                if asking {
                    continue;
                }
                let r = ai.api(method, path, body);
                let _ = r.status;
                assert!(
                    !ai.doc()["listener"]["enabled"].as_bool().unwrap(),
                    "{method} {path} {body} turned it on"
                );
            }
        }
    }
    assert!(!port_open(lan));
    assert_eq!(ai.keys.opens(), 0, "no request opened a prompt by itself");

    // Asking opens a prompt; it does not turn anything on.
    let asked = ai.api("POST", "/v1/ai/listener", r#"{"enabled":true}"#);
    assert_eq!(asked.status, 202);
    assert_eq!(asked.json()["pending"], true);
    let prompt = ai.wait_prompt();
    assert_eq!(prompt["kind"], "enable_listener");
    assert_eq!(prompt["title"], "Turn on AI access?");
    assert_eq!(asked.json()["promptId"], prompt["id"]);
    thread::sleep(Duration::from_millis(500));
    assert!(!ai.doc()["listener"]["enabled"].as_bool().unwrap());

    // While the prompt is up: keys from the network device and from unknown devices do nothing.
    for device in [
        NETWORK_KEYS,
        "LGE Network Input",
        "IoT keypad",
        "LGE RCU2",
        "uinput",
    ] {
        ai.keys.tap(device, KEY_OK);
    }
    thread::sleep(Duration::from_millis(300));
    assert!(
        !ai.doc()["listener"]["enabled"].as_bool().unwrap(),
        "a key from a device that is not a physical remote turned it on"
    );
    assert!(!port_open(lan));

    // Nothing over the API answers the prompt either.
    for (method, path) in [
        (
            "POST",
            format!("/v1/ai/prompt/{}", prompt["id"].as_str().unwrap()),
        ),
        (
            "POST",
            format!("/v1/ai/prompt/{}/allow", prompt["id"].as_str().unwrap()),
        ),
        (
            "POST",
            format!("/v1/ai/prompt/{}/approve", prompt["id"].as_str().unwrap()),
        ),
        ("POST", "/v1/ai/approve".to_owned()),
        ("POST", "/v1/ai/allow".to_owned()),
        ("POST", "/v1/ai/decision".to_owned()),
        ("POST", "/v1/ai/answer".to_owned()),
        ("PUT", "/v1/ai/prompt".to_owned()),
        ("POST", "/v1/state/ai.prompt".to_owned()),
        ("PUT", "/v1/state/ai.prompt".to_owned()),
    ] {
        let r = ai.api(
            method,
            &path,
            r#"{"allow":true,"decision":"allow","approve":true}"#,
        );
        assert!(
            matches!(r.status, 404 | 405),
            "{method} {path} -> {}",
            r.status
        );
    }
    assert_eq!(
        ai.prompt()["id"],
        prompt["id"],
        "the prompt is still waiting for the remote"
    );
    assert!(!port_open(lan));

    // A physical remote's OK is what does it.
    ai.press(IR_REMOTE, KEY_OK);
    let doc = ai.wait_doc(|d| d["listener"]["bound"] == true);
    assert_eq!(doc["listener"]["enabled"], true);
    assert!(doc["listener"]["since"].is_u64());
    let bound: SocketAddr = doc["listener"]["addr"].as_str().unwrap().parse().unwrap();
    assert_eq!(bound, lan);
    assert!(port_open(lan));
    ai.wait_no_prompt();
}

#[test]
fn back_on_the_physical_remote_leaves_it_off() {
    let port = free_port();
    let ai = start(port);
    assert_eq!(
        ai.api("POST", "/v1/ai/listener", r#"{"enabled":true}"#)
            .status,
        202
    );
    ai.wait_prompt();
    ai.press(MAGIC_REMOTE, KEY_BACK);
    ai.wait_no_prompt();
    thread::sleep(Duration::from_millis(200));
    assert!(!ai.doc()["listener"]["enabled"].as_bool().unwrap());
    assert!(!port_open(format!("127.0.0.1:{port}").parse().unwrap()));
}

#[test]
fn turning_it_off_is_immediate_and_closes_the_socket() {
    let port = free_port();
    let ai = start(port);
    let lan = ai.turn_on();
    assert!(port_open(lan));
    let r = ai.api("POST", "/v1/ai/listener", r#"{"enabled":false}"#);
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["changed"], true);
    assert!(
        wait_until(WAIT, || !port_open(lan)),
        "the port stayed open after turning it off"
    );
    ai.wait_doc(|d| d["listener"]["bound"] == false);
    assert_eq!(
        ai.api("POST", "/v1/ai/listener", r#"{"enabled":false}"#)
            .json()["changed"],
        false
    );
}

// ------------------------------------------------------------------------------------------------ pairing and the tools over MCP

#[test]
fn pairing_needs_the_remote_and_then_the_code_gives_a_working_token() {
    let port = free_port();
    let ai = start(port);
    let lan = ai.turn_on();

    // Without the remote there is no code and nothing to pair with.
    let refused = lan_post(lan, "/pair", None, r#"{"code":"AAAA-AAAA","name":"Cody"}"#);
    assert_eq!(refused.status, 403);
    assert_eq!(ai.doc()["pairing"], Value::Null);
    assert_eq!(ai.api("POST", "/v1/ai/pairing", "{}").status, 202);
    ai.wait_prompt();
    thread::sleep(Duration::from_millis(500));
    assert_eq!(
        ai.doc()["pairing"],
        Value::Null,
        "the code is shown only after the press"
    );
    ai.press(NETWORK_KEYS, KEY_OK);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        ai.doc()["pairing"],
        Value::Null,
        "a network key did not show the code"
    );
    ai.press(IR_REMOTE, KEY_OK);
    let doc = ai.wait_doc(|d| !d["pairing"].is_null());
    let code = doc["pairing"]["code"].as_str().unwrap().to_owned();
    assert_eq!(code.len(), 9);
    assert_eq!(doc["pairing"]["attemptsLeft"], 5);

    // A wrong code, then the right one.
    assert_eq!(
        lan_post(lan, "/pair", None, r#"{"code":"2222-2222","name":"Cody"}"#).status,
        403
    );
    assert_eq!(ai.doc()["pairing"]["attemptsLeft"], 4);
    let paired = lan_post(
        lan,
        "/pair",
        None,
        &json!({"code": code, "name": "  Cody\tthe agent "}).to_string(),
    );
    assert_eq!(paired.status, 200);
    let body = paired.json();
    let token = body["token"].as_str().unwrap().to_owned();
    let id = body["assistantId"].as_str().unwrap().to_owned();
    assert!(token.starts_with("ltv_") && token.len() == 68);
    assert_eq!(body["mcpPath"], "/mcp");

    // The code is spent; the TV lists the assistant with the plan's default levels; the token is nowhere in the state.
    let doc = ai.wait_doc(|d| d["assistants"].as_array().is_some_and(|a| a.len() == 1));
    assert_eq!(doc["pairing"], Value::Null);
    assert_eq!(doc["assistants"][0]["name"], "Cody the agent");
    assert_eq!(doc["assistants"][0]["id"], id);
    assert_eq!(
        doc["assistants"][0]["levels"],
        json!({"state": "always", "playback": "always", "launch": "ask", "power": "ask"})
    );
    assert!(!doc.to_string().contains(&token));
    assert_eq!(
        lan_post(
            lan,
            "/pair",
            None,
            &json!({"code": code, "name": "Again"}).to_string()
        )
        .status,
        403
    );
    let on_disk = std::fs::read_to_string(ai.rig.paths.data_dir.join("ai/state.json")).unwrap();
    assert!(
        !on_disk.contains(&token),
        "the token is stored in the clear"
    );

    // The token works; others do not.
    let init = mcp(
        lan,
        &token,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "lumatv");
    // The same token with its last character changed (it would be the real one one time in sixteen if it were fixed).
    let last = if token.ends_with('0') { '1' } else { '0' };
    let wrong = format!("{}{last}", &token[..67]);
    let r = lan_post(
        lan,
        "/mcp",
        Some(&wrong),
        r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
    );
    assert_eq!(r.status, 401);
    assert_eq!(
        lan_post(
            lan,
            "/mcp",
            None,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#
        )
        .status,
        401
    );
}

#[test]
fn the_tools_follow_the_levels_through_mcp() {
    let ai = start(free_port());
    let lan = ai.turn_on();
    let (token, id) = ai.pair(lan, "Cody");

    // The default levels: all thirteen tools are listed; reads and playback run at once.
    let tools = mcp(lan, &token, "tools/list", json!({}))["result"]["tools"].clone();
    assert_eq!(tools.as_array().unwrap().len(), 13);
    let launch = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "tv_launch_app")
        .unwrap();
    assert!(
        launch["description"]
            .as_str()
            .unwrap()
            .contains("Asks for approval on the TV")
    );
    let power = call(lan, &token, "tv_get_power", json!({}));
    assert!(!is_error(&power), "{power}");
    assert_eq!(power["content"][0]["type"], "text");
    let pause = call(lan, &token, "tv_media_key", json!({"key": "pause"}));
    assert!(!is_error(&pause), "{pause}");
    assert_eq!(ai.tv.acted.lock().as_slice(), ["key pause"]);
    assert!(!is_error(&call(
        lan,
        &token,
        "receiver_set_mute",
        json!({"muted": true})
    )));
    assert_eq!(ai.receiver.acted.lock().as_slice(), ["mute true"]);
    assert_eq!(
        ai.keys.opens(),
        2,
        "reads and playback needed no prompt: only turning on and pairing did"
    );

    // Opening an app asks: the prompt names it in the TV's words and carries the assistant's reason apart.
    let lan2 = lan;
    let token2 = token.clone();
    let worker = thread::spawn(move || {
        call(
            lan2,
            &token2,
            "tv_launch_app",
            json!({"app_id": "netflix", "reason": "Nitin asked for a film"}),
        )
    });
    let prompt = ai.wait_prompt();
    assert_eq!(prompt["kind"], "call");
    assert_eq!(prompt["title"], "Open Netflix");
    assert_eq!(prompt["assistant"]["name"], "Cody");
    assert_eq!(prompt["reason"], "Nitin asked for a film");
    ai.press(IR_REMOTE, KEY_OK);
    let launched = worker.join().unwrap();
    assert!(!is_error(&launched), "{launched}");
    assert_eq!(ai.tv.acted.lock().last().unwrap(), "launch netflix");

    // Back refuses, pauses the assistant, and nothing runs.
    let (lan2, token2) = (lan, token.clone());
    let worker = thread::spawn(move || call(lan2, &token2, "tv_standby", json!({})));
    ai.wait_prompt();
    ai.press(MAGIC_REMOTE, KEY_BACK);
    let denied = worker.join().unwrap();
    assert!(is_error(&denied));
    assert!(error_text(&denied).starts_with("denied"), "{denied}");
    assert!(!ai.tv.acted.lock().iter().any(|a| a == "standby"));
    let doc = ai.wait_doc(|d| d["assistants"][0]["paused"] == true);
    assert_eq!(doc["assistants"][0]["paused"], true);
    for (tool, arguments) in [
        ("tv_get_power", json!({})),
        ("tv_media_key", json!({"key": "play"})),
        ("tv_launch_app", json!({"app_id": "netflix"})),
    ] {
        let r = call(lan, &token, tool, arguments);
        assert!(
            is_error(&r) && error_text(&r).starts_with("paused"),
            "{tool}: {r}"
        );
    }

    // Resuming is something only the remote can do: the API asks, the TV prompts, a network key changes nothing.
    assert_eq!(
        ai.api("POST", &format!("/v1/ai/assistants/{id}/resume"), "")
            .status,
        202
    );
    ai.wait_prompt();
    ai.press(NETWORK_KEYS, KEY_OK);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(ai.doc()["assistants"][0]["paused"], true);
    ai.press(IR_REMOTE, KEY_OK);
    ai.wait_doc(|d| d["assistants"][0]["paused"] == false);
    assert!(!is_error(&call(lan, &token, "tv_get_power", json!({}))));
}

#[test]
fn levels_go_down_at_once_and_up_only_with_the_remote() {
    let ai = start(free_port());
    let lan = ai.turn_on();
    let (token, id) = ai.pair(lan, "Cody");
    let levels_path = format!("/v1/ai/assistants/{id}/levels");

    // Down is immediate: reads become `ask`, playback `never`; the tool disappears from the list.
    let lower = r#"{"levels":{"state":"ask","playback":"never","launch":"never","power":"ask"}}"#;
    let r = ai.api("POST", &levels_path, lower);
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["changed"], true);
    assert_eq!(
        ai.doc()["assistants"][0]["levels"],
        json!({"state": "ask", "playback": "never", "launch": "never", "power": "ask"})
    );
    let names: Vec<String> = mcp(lan, &token, "tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        !names.contains(&"tv_media_key".to_owned()) && !names.contains(&"tv_launch_app".to_owned())
    );
    let never = call(lan, &token, "tv_media_key", json!({"key": "play"}));
    assert!(
        is_error(&never) && error_text(&never).starts_with("never"),
        "{never}"
    );

    // Up is a request: the TV prompts, the levels do not change until a physical remote says OK.
    let raise =
        r#"{"levels":{"state":"always","playback":"always","launch":"never","power":"ask"}}"#;
    let asked = ai.api("POST", &levels_path, raise);
    assert_eq!(asked.status, 202);
    let prompt = ai.wait_prompt();
    assert_eq!(prompt["kind"], "widen_levels");
    assert!(
        prompt["detail"]
            .as_str()
            .unwrap()
            .contains("See what is playing, power and input state: Ask on TV to Always"),
        "{prompt}"
    );
    ai.press(NETWORK_KEYS, KEY_OK);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        ai.doc()["assistants"][0]["levels"]["state"],
        "ask",
        "a network key widened a permission"
    );
    ai.press(IR_REMOTE, KEY_BACK);
    ai.wait_no_prompt();
    assert_eq!(ai.doc()["assistants"][0]["levels"]["state"], "ask");
    assert_eq!(ai.api("POST", &levels_path, raise).status, 202);
    ai.wait_prompt();
    ai.press(IR_REMOTE, KEY_OK);
    ai.wait_doc(|d| d["assistants"][0]["levels"]["playback"] == "always");
    assert!(!is_error(&call(
        lan,
        &token,
        "tv_media_key",
        json!({"key": "play"})
    )));

    // Mixed (one group up, others down) is a widening and needs the remote.
    let mixed =
        r#"{"levels":{"state":"never","playback":"always","launch":"ask","power":"never"}}"#;
    assert_eq!(ai.api("POST", &levels_path, mixed).status, 202);
    ai.wait_prompt();
    ai.press(IR_REMOTE, KEY_BACK);
    ai.wait_no_prompt();

    // The generic settings API cannot touch an assistant's permissions at all.
    let owner = format!("/v1/settings/ai.{id}");
    let change = r#"{"changes":[{"key":"launch","value":{"type":"choice","value":"always"}}]}"#;
    let r = ai.api("POST", &owner, change);
    assert_eq!(r.status, 403);
    assert_eq!(r.json()["error"]["code"], "protected_owner");
    assert_eq!(ai.doc()["assistants"][0]["levels"]["launch"], "never");
    // Reading it is fine, and it is a page the TV's Settings can draw.
    let page = ai.api("GET", &owner, "").json();
    assert_eq!(
        page["schema"]["sections"][0]["fields"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    // Bad levels are refused as bad requests.
    for bad in [
        r#"{"levels":{"state":"always"}}"#,
        r#"{"levels":{"state":"sometimes","playback":"ask","launch":"ask","power":"ask"}}"#,
        r#"{"levels":5}"#,
        "{}",
        r#"{"levels":{},"extra":1}"#,
    ] {
        assert_eq!(ai.api("POST", &levels_path, bad).status, 400, "{bad}");
    }
    assert_eq!(
        ai.api("POST", "/v1/ai/assistants/ffffffffffff/levels", lower)
            .status,
        404
    );
    assert_eq!(
        ai.api("POST", "/v1/ai/assistants/not-an-id/levels", lower)
            .status,
        404
    );
}

#[test]
fn the_volume_limit_goes_down_at_once_and_up_only_with_the_remote() {
    let ai = start(free_port());
    let lan = ai.turn_on();
    let (token, _) = ai.pair(lan, "Cody");
    let over = call(lan, &token, "receiver_set_volume", json!({"level": 60}));
    assert!(
        is_error(&over) && error_text(&over).starts_with("over_limit"),
        "{over}"
    );
    assert!(ai.receiver.acted.lock().is_empty());
    assert_eq!(
        ai.api("POST", "/v1/ai/limits", r#"{"maxVolume":40}"#)
            .status,
        200
    );
    assert_eq!(ai.doc()["limits"]["maxVolume"], 40.0);
    let over = call(lan, &token, "receiver_set_volume", json!({"level": 45}));
    assert!(is_error(&over));
    assert!(!is_error(&call(
        lan,
        &token,
        "receiver_set_volume",
        json!({"level": 38})
    )));
    assert_eq!(ai.receiver.acted.lock().as_slice(), ["volume 38"]);
    assert_eq!(
        ai.api("POST", "/v1/ai/limits", r#"{"maxVolume":70}"#)
            .status,
        202
    );
    ai.wait_prompt();
    ai.press(NETWORK_KEYS, KEY_OK);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(ai.doc()["limits"]["maxVolume"], 40.0);
    ai.press(IR_REMOTE, KEY_OK);
    ai.wait_doc(|d| d["limits"]["maxVolume"] == 70.0);
    for bad in [
        r#"{"maxVolume":99}"#,
        r#"{"maxVolume":-1}"#,
        r#"{"maxVolume":"loud"}"#,
        "{}",
    ] {
        assert_eq!(ai.api("POST", "/v1/ai/limits", bad).status, 400, "{bad}");
    }
}

// ------------------------------------------------------------------------------------------------ the LAN door is only the MCP door

#[test]
fn what_an_assistant_reaches_over_the_lan_is_mcp_and_pairing_and_nothing_else() {
    let ai = start(free_port());
    let lan = ai.turn_on();
    let (token, id) = ai.pair(lan, "Cody");
    let auth = format!("Bearer {token}");
    let headers = [
        ("Authorization", auth.as_str()),
        ("Content-Type", "application/json"),
    ];
    for (method, path) in [
        ("GET", "/v1/ai".to_owned()),
        ("GET", "/v1/state".to_owned()),
        ("GET", "/v1/settings".to_owned()),
        ("GET", "/v1/health".to_owned()),
        ("POST", "/v1/ai/listener".to_owned()),
        ("POST", "/v1/ai/pairing".to_owned()),
        ("POST", "/v1/ai/lockout".to_owned()),
        ("POST", format!("/v1/ai/assistants/{id}/levels")),
        ("POST", format!("/v1/ai/assistants/{id}/resume")),
        ("DELETE", format!("/v1/ai/assistants/{id}")),
        ("POST", "/v1/settings/base.receiver".to_owned()),
        ("POST", format!("/v1/settings/ai.{id}")),
        ("GET", "/".to_owned()),
        ("GET", "/mcp/../v1/ai".to_owned()),
        // the display lab is the TV's own shell's: not a path of the LAN door, whoever is paired
        ("GET", "/v1/displaylab".to_owned()),
        ("POST", "/v1/displaylab/begin".to_owned()),
        ("POST", "/v1/displaylab/set".to_owned()),
        ("POST", "/v1/displaylab/ping".to_owned()),
        ("POST", "/v1/displaylab/end".to_owned()),
        ("GET", "/v1/displaylab/results".to_owned()),
    ] {
        let r = http(
            lan,
            method,
            &path,
            &headers,
            if method == "POST" {
                Some(
                    r#"{"enabled":false,"levels":{"state":"always","playback":"always","launch":"always","power":"always"}}"#,
                )
            } else {
                None
            },
        );
        assert!(
            matches!(r.status, 404 | 405),
            "{method} {path} reached something: {} {}",
            r.status,
            String::from_utf8_lossy(&r.body)
        );
    }
    // And nothing changed.
    let doc = ai.doc();
    assert_eq!(doc["listener"]["enabled"], true);
    assert_eq!(doc["assistants"].as_array().unwrap().len(), 1);
    assert_eq!(doc["assistants"][0]["levels"]["launch"], "ask");

    // Tools that would widen anything do not exist, whatever they are called.
    for name in [
        "ai_enable_listener",
        "ai_set_levels",
        "ai_pair",
        "ai_resume",
        "tv_shell",
        "luna_call",
        "read_file",
        "set_setting",
        "press_key",
        "approve",
        "tv_approve_prompt",
        "displaylab_set",
        "display_lab",
        "tv_set_sharpness",
        "tv_dim_osd",
        "write_register",
        "devmem_write",
    ] {
        let r = call(lan, &token, name, json!({"enabled": true}));
        assert!(is_error(&r), "{name}: {r}");
    }
    let key = call(lan, &token, "tv_media_key", json!({"key": "ok"}));
    assert!(is_error(&key), "an assistant pressed OK: {key}");
    let home = call(lan, &token, "tv_media_key", json!({"key": "home"}));
    assert!(is_error(&home));
    assert!(ai.tv.acted.lock().is_empty());
    assert_eq!(
        ai.keys.opens(),
        2,
        "only the two prompts of turning on and pairing were opened"
    );
}

#[test]
fn a_browser_cannot_reach_the_lan_door() {
    let ai = start(free_port());
    let lan = ai.turn_on();
    let (token, _) = ai.pair(lan, "Cody");
    let auth = format!("Bearer {token}");
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let r = http(
        lan,
        "POST",
        "/mcp",
        &[
            ("Authorization", auth.as_str()),
            ("Content-Type", "application/json"),
            ("Origin", "http://evil.example"),
        ],
        Some(body),
    );
    assert_eq!(r.status, 403);
    let r = http(
        lan,
        "POST",
        "/mcp",
        &[
            ("Authorization", auth.as_str()),
            ("Content-Type", "application/json"),
            ("Sec-Fetch-Site", "cross-site"),
        ],
        Some(body),
    );
    assert_eq!(r.status, 403);
    let r = http(
        lan,
        "OPTIONS",
        "/mcp",
        &[
            ("Origin", "http://evil.example"),
            ("Access-Control-Request-Method", "POST"),
        ],
        None,
    );
    assert_eq!(r.status, 403);
    assert!(r.header("access-control-allow-origin").is_none());
    let r = http(lan, "OPTIONS", "/mcp", &[], None);
    assert_eq!(r.status, 405);
    assert!(r.header("access-control-allow-origin").is_none());
}

// ------------------------------------------------------------------------------------------------ lock out and restart

#[test]
fn a_lock_out_takes_everything_away_at_once() {
    let ai = start(free_port());
    let lan = ai.turn_on();
    let (token, id) = ai.pair(lan, "Cody");
    let (_, other_id) = ai.pair(lan, "Home Assistant");
    assert_eq!(ai.doc()["assistants"].as_array().unwrap().len(), 2);
    let r = ai.api("POST", "/v1/ai/lockout", "");
    assert_eq!(r.status, 200);
    let body = r.json();
    assert_eq!(body["revoked"], 2);
    assert_eq!(body["listenerWasOn"], true);
    assert_eq!(body["ai"]["assistants"], json!([]));
    assert_eq!(body["ai"]["listener"]["enabled"], false);
    assert!(
        wait_until(WAIT, || !port_open(lan)),
        "the AI port stayed open after a lock-out"
    );
    // The listener's own thread closes the port first and publishes the state a moment later.
    let doc = ai.wait_doc(|d| d["listener"]["bound"] == false);
    assert_eq!(doc["assistants"], json!([]));
    assert_eq!(doc["pairing"], Value::Null);
    // The permissions pages are gone from the settings layer, and from disk.
    assert_eq!(
        ai.api("GET", &format!("/v1/settings/ai.{id}"), "").status,
        404
    );
    assert_eq!(
        ai.api("GET", &format!("/v1/settings/ai.{other_id}"), "")
            .status,
        404
    );
    assert!(
        !ai.rig
            .paths
            .settings_dir()
            .join(format!("ai.{id}.json"))
            .exists()
    );
    // Turning it on again does not bring the old token back.
    let again = ai.turn_on();
    let r = lan_post(
        again,
        "/mcp",
        Some(&token),
        r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
    );
    assert_eq!(r.status, 401);
    // A lock-out with nothing to take away is fine.
    assert_eq!(ai.api("POST", "/v1/ai/lockout", "").status, 200);
    assert_eq!(ai.api("POST", "/v1/ai/lockout", "").json()["revoked"], 0);
}

#[test]
fn a_lock_out_withdraws_an_open_prompt() {
    let ai = start(free_port());
    assert_eq!(
        ai.api("POST", "/v1/ai/listener", r#"{"enabled":true}"#)
            .status,
        202
    );
    ai.wait_prompt();
    assert_eq!(ai.api("POST", "/v1/ai/lockout", "").status, 200);
    ai.wait_no_prompt();
    // Even a physical OK now, with no prompt, turns nothing on.
    ai.press(IR_REMOTE, KEY_OK);
    thread::sleep(Duration::from_millis(300));
    assert!(!ai.doc()["listener"]["enabled"].as_bool().unwrap());
}

#[test]
fn an_assistant_can_be_unpaired_alone_and_a_second_prompt_is_refused_while_one_is_open() {
    let ai = start(free_port());
    let lan = ai.turn_on();
    let (token, id) = ai.pair(lan, "Cody");
    let (token2, _) = ai.pair(lan, "Other");
    let r = ai.api("DELETE", &format!("/v1/ai/assistants/{id}"), "");
    assert_eq!(r.status, 200);
    assert_eq!(
        lan_post(
            lan,
            "/mcp",
            Some(&token),
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#
        )
        .status,
        401
    );
    assert_eq!(
        lan_post(
            lan,
            "/mcp",
            Some(&token2),
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#
        )
        .status,
        200
    );
    assert_eq!(
        ai.api("DELETE", &format!("/v1/ai/assistants/{id}"), "")
            .status,
        404
    );

    // One prompt at a time: a second ask while one is open is refused, and the first is unharmed.
    assert_eq!(ai.api("POST", "/v1/ai/pairing", "{}").status, 202);
    let prompt = ai.wait_prompt();
    assert_eq!(ai.api("POST", "/v1/ai/pairing", "{}").status, 409);
    assert_eq!(
        ai.api("POST", "/v1/ai/limits", r#"{"maxVolume":90}"#)
            .status,
        409
    );
    assert_eq!(ai.prompt()["id"], prompt["id"]);
    ai.press(IR_REMOTE, KEY_BACK);
    ai.wait_no_prompt();
}

#[test]
fn what_was_paired_and_turned_on_survives_a_restart() {
    let port = free_port();
    let ai = start(port);
    let lan = ai.turn_on();
    let (token, _) = ai.pair(lan, "Cody");
    assert_eq!(
        ai.api("POST", "/v1/ai/limits", r#"{"maxVolume":42}"#)
            .status,
        200
    );
    let ai = ai.restart(port);
    let doc = ai.wait_doc(|d| d["listener"]["bound"] == true);
    assert_eq!(doc["listener"]["enabled"], true);
    assert_eq!(doc["assistants"][0]["name"], "Cody");
    assert_eq!(doc["limits"]["maxVolume"], 42.0);
    let lan: SocketAddr = doc["listener"]["addr"].as_str().unwrap().parse().unwrap();
    assert_eq!(mcp(lan, &token, "ping", json!({}))["result"], json!({}));
}

#[test]
fn a_damaged_state_file_means_off_and_unpaired_after_a_restart() {
    let port = free_port();
    let ai = start(port);
    let lan = ai.turn_on();
    let (token, _) = ai.pair(lan, "Cody");
    let mut rig = ai.rig;
    rig.stop();
    assert!(
        !port_open(lan),
        "the port stayed open after the helper stopped"
    );
    std::fs::write(
        rig.paths.data_dir.join("ai/state.json"),
        "{ this is not json",
    )
    .unwrap();
    let again = start_in(rig, port);
    thread::sleep(Duration::from_millis(300));
    let doc = again.doc();
    assert_eq!(doc["listener"]["enabled"], false);
    assert_eq!(doc["assistants"], json!([]));
    assert!(!port_open(lan));
    // The old token opens nothing even when the switch is turned on again.
    let lan = again.turn_on();
    assert_eq!(
        lan_post(
            lan,
            "/mcp",
            Some(&token),
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#
        )
        .status,
        401
    );
}

#[test]
fn a_lock_out_while_a_tool_call_waits_for_the_remote_refuses_the_call() {
    let ai = start(free_port());
    let lan = ai.turn_on();
    let (token, _) = ai.pair(lan, "Cody");
    let (lan2, token2) = (lan, token.clone());
    let worker =
        thread::spawn(move || call(lan2, &token2, "tv_launch_app", json!({"app_id": "netflix"})));
    assert_eq!(ai.wait_prompt()["kind"], "call");
    assert_eq!(ai.api("POST", "/v1/ai/lockout", "").status, 200);
    let result = worker.join().unwrap();
    assert!(is_error(&result), "{result}");
    ai.wait_no_prompt();
    // A physical OK after the lock-out finds no prompt and nothing runs.
    ai.press(IR_REMOTE, KEY_OK);
    thread::sleep(Duration::from_millis(300));
    assert!(ai.tv.acted.lock().is_empty());
}

#[test]
fn an_unanswered_prompt_expires_and_nothing_runs() {
    let ai = start(free_port());
    let lan = ai.turn_on();
    let (token, _) = ai.pair(lan, "Cody");
    let started = std::time::Instant::now();
    let result = call(lan, &token, "tv_standby", json!({}));
    assert!(
        started.elapsed() >= Duration::from_secs(4),
        "it did not wait for the remote"
    );
    assert!(is_error(&result));
    assert!(error_text(&result).starts_with("expired"), "{result}");
    assert!(ai.tv.acted.lock().is_empty());
    // An expiry is not a refusal: the assistant is not paused, but it has to wait before asking again.
    assert_eq!(ai.doc()["assistants"][0]["paused"], false);
    let again = call(lan, &token, "tv_standby", json!({}));
    assert!(
        is_error(&again) && error_text(&again).starts_with("cooldown"),
        "{again}"
    );
    assert!(!is_error(&call(lan, &token, "tv_get_power", json!({}))));
}

// ------------------------------------------------------------------------------------------------ the audit trail and the pages

#[test]
fn every_call_is_audited_and_the_trail_is_on_the_api() {
    let ai = start(free_port());
    let lan = ai.turn_on();
    let (token, id) = ai.pair(lan, "Cody");
    call(lan, &token, "tv_get_power", json!({}));
    call(lan, &token, "tv_media_key", json!({"key": "stop"}));
    call(lan, &token, "tv_media_key", json!({"key": "home"}));
    call(lan, &token, "no_such_tool", json!({}));
    let entries = ai
        .api("GET", &format!("/v1/ai/audit?assistant={id}&limit=50"), "")
        .json()["entries"]
        .clone();
    let rows: Vec<(String, String)> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["tool"].as_str().unwrap().to_owned(),
                e["outcome"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("pair".to_owned(), "ok".to_owned()),
            ("tv_get_power".to_owned(), "ok".to_owned()),
            ("tv_media_key".to_owned(), "ok".to_owned()),
            ("tv_media_key".to_owned(), "invalid".to_owned()),
            ("no_such_tool".to_owned(), "invalid".to_owned()),
        ]
    );
    assert_eq!(entries[1]["peer"], "127.0.0.1");
    assert_eq!(entries[2]["args"], json!({"key": "stop"}));
    // The file holds the same, and no token.
    let file = std::fs::read_to_string(ai.rig.paths.data_dir.join("ai/audit.jsonl")).unwrap();
    assert!(file.lines().count() >= 5);
    assert!(!file.contains(&token));
    assert_eq!(ai.api("GET", "/v1/ai/audit?limit=abc", "").status, 400);
    assert_eq!(ai.api("GET", "/v1/ai/audit?limit=9999", "").status, 400);
    // A call raises an `ai`/`audit` event for the TV's page.
    let events = get(ai.rig.addr, "/v1/events?after=0&topics=ai&timeout=0.5").json();
    assert!(
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "audit" && e["data"]["tool"] == "tv_get_power")
    );
}

#[test]
fn the_api_documents_its_ai_routes_and_none_of_them_answers_a_prompt() {
    let ai = start(free_port());
    let root = get(ai.rig.addr, "/v1").json();
    let listed: Vec<String> = root["endpoints"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["path"].as_str().unwrap().starts_with("/v1/ai"))
        .map(|e| {
            format!(
                "{} {}",
                e["method"].as_str().unwrap(),
                e["path"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(listed.len(), lumatv_helper::ai::routes::ROUTES.len());
    for route in &listed {
        for word in [
            "prompt", "answer", "allow", "approve", "decision", "token", "confirm",
        ] {
            assert!(
                !route.contains(word),
                "{route} looks like a way to answer a prompt or make a token"
            );
        }
    }
    assert!(listed.contains(&"POST /v1/ai/lockout".to_owned()));
}
