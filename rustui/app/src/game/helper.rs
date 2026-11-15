//! The optional gaming lease is renewed only when the UI loop ticks, never by an independent heartbeat.
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc::{self, Receiver, SyncSender}};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const RENEW_MS: u64 = 2_000;
const TIMEOUT: Duration = Duration::from_millis(500);

pub struct Lease {
    wanted: Arc<AtomicBool>,
    wake: Option<SyncSender<()>>,
    notices: Receiver<String>,
    worker: Option<JoinHandle<()>>,
    due: u64,
}

impl Lease {
    pub fn new(address: SocketAddr) -> io::Result<Self> {
        let wanted = Arc::new(AtomicBool::new(false));
        let active = wanted.clone();
        let (wake, requests) = mpsc::sync_channel(1);
        let (report, notices) = mpsc::sync_channel(1);
        let worker = thread::Builder::new().name("gaming-lease".into()).spawn(move || {
            let mut lease = LeaseState::default();
            let mut last_error = None;
            let mut http = |method: &str, path: &str, body: &Value| request(address, method, path, body);
            while requests.recv().is_ok() {
                let result = lease.sync(active.load(Ordering::Acquire), &mut http);
                if let Err(why) = result {
                    if last_error.as_ref() != Some(&why) { let _ = report.try_send(why.clone()); }
                    last_error = Some(why);
                } else { last_error = None; }
            }
            let _ = lease.sync(false, &mut http);
        })?;
        Ok(Self { wanted, wake: Some(wake), notices, worker: Some(worker), due: 0 })
    }

    pub fn set(&mut self, wanted: bool, now_ms: u64) {
        if self.wanted.swap(wanted, Ordering::AcqRel) != wanted {
            self.due = now_ms;
            self.notify();
            self.due = now_ms.saturating_add(RENEW_MS);
        }
    }

    pub fn tick(&mut self, now_ms: u64) {
        if self.wanted.load(Ordering::Acquire) && now_ms >= self.due {
            self.notify();
            self.due = now_ms.saturating_add(RENEW_MS);
        }
    }

    fn notify(&self) { if let Some(wake) = &self.wake { let _ = wake.try_send(()); } }
    pub fn notice(&self) -> Option<String> { self.notices.try_recv().ok() }
    pub fn wait_ms(&self, now_ms: u64) -> u64 { if self.wanted.load(Ordering::Acquire) { self.due.saturating_sub(now_ms) } else { u64::MAX } }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.wanted.store(false, Ordering::Release);
        self.wake = None;
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

#[derive(Default)]
struct LeaseState { id: Option<Value> }

impl LeaseState {
    fn sync(&mut self, wanted: bool, http: &mut impl FnMut(&str, &str, &Value) -> Result<Value, String>) -> Result<(), String> {
        if !wanted {
            if let Some(id) = self.id.take() { http("DELETE", "/v1/gaming/lease", &json!({"lease":id}))?; }
            return Ok(());
        }
        if let Some(id) = &self.id {
            if let Err(why) = http("POST", "/v1/gaming/renew", &json!({"lease":id})) {
                let _ = http("DELETE", "/v1/gaming/lease", &json!({"lease":id}));
                self.id = None;
                return Err(why);
            }
        } else {
            let result = http("POST", "/v1/gaming/lease", &json!({"pid":std::process::id()}))?;
            let id = result.get("lease").and_then(|lease| lease.get("id")).filter(|id| id.is_string() || id.is_u64()).ok_or("Gaming helper returned no lease; play continues without it.")?;
            self.id = Some(id.clone());
        }
        Ok(())
    }
}

fn request(address: SocketAddr, method: &str, path: &str, body: &Value) -> Result<Value, String> {
    http(address, method, path, body).map_err(|e| format!("Gaming helper is unavailable; play continues without it ({e})."))
}

fn http(address: SocketAddr, method: &str, path: &str, body: &Value) -> io::Result<Value> {
    if !address.ip().is_loopback() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "the gaming helper must be on loopback")); }
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let body = body.to_string();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {address}\r\nX-Luma-Client: lumagame\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
    let mut raw = Vec::new();
    stream.take(64 * 1024 + 1).read_to_end(&mut raw)?;
    if raw.len() > 64 * 1024 { return Err(io::Error::new(io::ErrorKind::InvalidData, "helper reply is too large")); }
    parse_reply(&raw)
}

fn parse_reply(raw: &[u8]) -> io::Result<Value> {
    let text = std::str::from_utf8(raw).map_err(io::Error::other)?;
    let (headers, body) = text.split_once("\r\n\r\n").ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "helper sent no HTTP headers"))?;
    let status = headers.lines().next().and_then(|line| line.split_whitespace().nth(1)).and_then(|code| code.parse::<u16>().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "helper sent no status"))?;
    let reply: Value = serde_json::from_str(body).map_err(io::Error::other)?;
    if status == 403 { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "enable the Gaming plugin in LumaTV to use its controls")); }
    if status != 200 || reply.get("ok") != Some(&Value::Bool(true)) {
        let message = reply.pointer("/error/message").and_then(Value::as_str).unwrap_or("request was not accepted");
        return Err(io::Error::other(format!("{status}: {message}")));
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_renew_release_and_failed_renew_are_best_effort() {
        let mut state = LeaseState::default();
        let mut seen = Vec::new();
        let mut http = |method: &str, path: &str, body: &Value| {
            seen.push((method.to_owned(), path.to_owned(), body.clone()));
            Ok(json!({"ok":true,"lease":{"id":"one"}}))
        };
        state.sync(false, &mut http).unwrap();
        state.sync(true, &mut http).unwrap();
        state.sync(true, &mut http).unwrap();
        state.sync(false, &mut http).unwrap();
        assert_eq!(seen.iter().map(|(m,p,_)| (m.as_str(),p.as_str())).collect::<Vec<_>>(), [("POST","/v1/gaming/lease"),("POST","/v1/gaming/renew"),("DELETE","/v1/gaming/lease")]);
        assert_eq!(seen[1].2, json!({"lease":"one"}));
        state.id = Some(json!("one"));
        assert!(state.sync(true, &mut |_,_,_| Err("helper restarted".into())).is_err());
        assert!(state.id.is_none());
    }

    #[test]
    fn renewal_needs_a_ui_tick_every_two_seconds() {
        let (wake, requests) = mpsc::sync_channel(1);
        let (_report, notices) = mpsc::sync_channel(1);
        let mut lease = Lease { wanted: Arc::new(AtomicBool::new(false)), wake: Some(wake), notices, worker: None, due: 0 };
        lease.set(true, 10);
        assert!(requests.try_recv().is_ok());
        lease.tick(2009);
        assert!(requests.try_recv().is_err());
        lease.tick(2010);
        assert!(requests.try_recv().is_ok());
        lease.set(false, 2011);
        assert!(requests.try_recv().is_ok());
        lease.tick(10_000);
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn helper_off_is_not_a_game_failure() {
        let reply = b"HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\n\r\n{\"ok\":false,\"error\":{\"code\":\"plugin_off\"}}";
        assert_eq!(parse_reply(reply).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(parse_reply(b"HTTP/1.1 200 OK\r\n\r\n{\"ok\":true}").is_ok());
    }
}
