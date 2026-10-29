//! The Rust shell's HTTP client (`rustui/ui/src/host/http.rs`, compiled in here as it is) against the helper's real server and its
//! `Host` check and write guard: a client that leaves the port off `Host` is refused 403 `host_not_allowed`, and a write without `X-Luma-Client` 403 too; Home came up empty that way.

#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../../../rustui/ui/src/host/http.rs"]
mod ui_http;

use lumatv_helper::http::{Handler, Options, Request, Response, Server};
use serde_json::json;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

struct Ok200;

impl Handler for Ok200 {
    fn handle(&self, _: &Request) -> Response {
        Response::json(200, &json!({"ok": true}))
    }
}

#[test]
fn the_shells_client_is_not_refused_by_the_helpers_host_check() {
    let server = Server::start("127.0.0.1:0".parse().unwrap(), Arc::new(Ok200), Options::default()).unwrap();
    for (method, path) in [("GET", "/v1/state"), ("POST", "/v1/anything")] {
        let mut stream = TcpStream::connect(server.local_addr()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (status, body, _) = ui_http::exchange(&mut stream, method, path, Some("token"), (method == "POST").then_some(b"{}".as_slice())).unwrap();
        assert_eq!(status, 200, "{method} {path}: {}", String::from_utf8_lossy(&body));
    }
    server.stop(Duration::from_secs(2));
}
