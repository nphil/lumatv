//! The app's end of OpenNOW's core protocol (OpenNOW `docs/core-protocol.md`): newline-delimited JSON on the core process's stdin and
//! stdout. Requests carry an id and get one response; the core also sends events (`session.changed`, `streamer.changed`,
//! `auth.session.changed`, ...). A reader thread turns the core's lines into messages on a channel; [`CoreClient::poll`] drains it.
//! Nothing here interprets a message: that is `app.rs`'s job, so it can be tested without a core.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::io::RawFd;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: u64 = 5;
pub const STREAMER_PROTOCOL_VERSION: u64 = 7;

/// What the core sent, as the app sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreMsg {
    /// The answer to the request with this id and method.
    Response { id: String, method: String, ok: bool, result: Value, error_code: String, error_message: String },
    Event { name: String, payload: Value },
    /// The core's stdout ended (it exited or crashed).
    Gone,
}

pub struct CoreClient {
    stdin: Box<dyn Write + Send>,
    rx: Receiver<Value>,
    reader: Option<JoinHandle<()>>,
    child: Option<Child>,
    next_id: u64,
    pending: HashMap<String, (String, Instant)>,
    gone: bool,
}

impl CoreClient {
    /// Starts `opennow-core` through `runner` (the 64-bit loader script on the set, the binary itself on the host) with `HOME` and
    /// `XDG_CONFIG_HOME` under `data_dir`, where it keeps its account files, and `nativeStreamerExecutablePath` the engine's path.
    pub fn spawn(runner: &Path, args: &[String], data_dir: &Path, env: &[(&str, String)]) -> io::Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let mut cmd = Command::new(runner);
        cmd.args(args)
            .env("HOME", data_dir)
            .env("XDG_CONFIG_HOME", data_dir.join("config"))
            .env("XDG_DATA_HOME", data_dir.join("data"))
            .env("XDG_CACHE_HOME", data_dir.join("cache"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| io::Error::other("the core's stdin is not a pipe"))?;
        let stdout = child.stdout.take().ok_or_else(|| io::Error::other("the core's stdout is not a pipe"))?;
        let mut client = Self::over(Box::new(stdin), Box::new(stdout))?;
        client.child = Some(child);
        Ok(client)
    }

    /// The client over any pair of streams (the tests hand it a socket pair with a fake core on the other end).
    pub fn over(stdin: Box<dyn Write + Send>, stdout: Box<dyn io::Read + Send>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Value>();
        let reader = thread::Builder::new().name("gfn-core-reader".to_owned()).spawn(move || read_lines(stdout, &tx))?;
        Ok(Self { stdin, rx, reader: Some(reader), child: None, next_id: 1, pending: HashMap::new(), gone: false })
    }

    /// Sends a request and returns its id.
    pub fn request(&mut self, method: &str, params: Value) -> io::Result<String> {
        let id = self.next_id.to_string();
        self.next_id += 1;
        let line = json!({"type": "request", "id": id, "method": method, "params": params});
        self.write(&line)?;
        self.pending.insert(id.clone(), (method.to_owned(), Instant::now()));
        Ok(id)
    }

    /// The acknowledgement a successful `session.create` response requires before the shell acts on it.
    pub fn ack(&mut self, id: &str) -> io::Result<()> {
        self.write(&json!({"type": "ack", "id": id}))
    }

    pub fn cancel(&mut self, id: &str) -> io::Result<()> {
        self.pending.remove(id);
        self.write(&json!({"type": "cancel", "id": id}))
    }

    fn write(&mut self, line: &Value) -> io::Result<()> {
        let mut s = serde_json::to_string(line).map_err(io::Error::other)?;
        s.push('\n');
        self.stdin.write_all(s.as_bytes())?;
        self.stdin.flush()
    }

    /// Everything the core has said since the last call.
    pub fn poll(&mut self) -> Vec<CoreMsg> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(v) => {
                    if let Some(m) = self.decode(v) {
                        out.push(m);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.gone {
                        self.gone = true;
                        out.push(CoreMsg::Gone);
                    }
                    break;
                }
            }
        }
        out
    }

    /// Waits for the next message, at most `timeout`.
    pub fn wait(&mut self, timeout: Duration) -> Option<CoreMsg> {
        match self.rx.recv_timeout(timeout) {
            Ok(v) => self.decode(v),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if self.gone {
                    None
                } else {
                    self.gone = true;
                    Some(CoreMsg::Gone)
                }
            }
        }
    }

    fn decode(&mut self, v: Value) -> Option<CoreMsg> {
        match v["type"].as_str() {
            Some("response") => {
                let id = v["id"].as_str().unwrap_or("").to_owned();
                let method = self.pending.remove(&id).map(|(m, _)| m).unwrap_or_default();
                Some(CoreMsg::Response {
                    id,
                    method,
                    ok: v["ok"].as_bool().unwrap_or(false),
                    result: v["result"].clone(),
                    error_code: v["error"]["code"].as_str().unwrap_or("").to_owned(),
                    error_message: v["error"]["message"].as_str().unwrap_or("").to_owned(),
                })
            }
            Some("event") => Some(CoreMsg::Event { name: v["name"].as_str().unwrap_or("").to_owned(), payload: v["payload"].clone() }),
            _ => None,
        }
    }

    /// Requests still waiting for an answer, oldest first.
    pub fn pending(&self) -> Vec<(String, String, Duration)> {
        let mut v: Vec<_> = self.pending.iter().map(|(id, (m, t))| (id.clone(), m.clone(), t.elapsed())).collect();
        v.sort_by_key(|(_, _, age)| std::cmp::Reverse(*age));
        v
    }

    pub fn is_gone(&self) -> bool {
        self.gone
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// Ends the core: stdin closes, the process is given a moment, then killed.
    pub fn shutdown(mut self) {
        let _ = self.write(&json!({"type": "request", "id": "bye", "method": "app.shutdown", "params": {}}));
        drop(self.stdin);
        if let Some(mut child) = self.child.take() {
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

fn read_lines(stdout: Box<dyn io::Read + Send>, tx: &Sender<Value>) {
    let reader = BufReader::with_capacity(1 << 20, stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(v) => {
                if tx.send(v).is_err() {
                    break;
                }
            }
            Err(e) => eprintln!("gfn core: a line that is not JSON ({e}): {}", &line[..line.len().min(200)]),
        }
    }
}

/// A descriptor to wait on beside the window's (the reader thread owns the pipe; the channel has none), so the loop wakes on a timer
/// instead: `None`.
pub fn wake_fd() -> Option<RawFd> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn requests_get_ids_and_responses_find_their_method() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut c = CoreClient::over(Box::new(a.try_clone().unwrap()), Box::new(a)).unwrap();
        let id = c.request("core.hello", json!({"protocolVersion": 5})).unwrap();
        assert_eq!(id, "1");
        let mut core = BufReader::new(b.try_clone().unwrap());
        let mut line = String::new();
        core.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["method"], "core.hello");
        assert_eq!(v["params"]["protocolVersion"], 5);
        let mut w = b.try_clone().unwrap();
        w.write_all(b"{\"type\":\"response\",\"id\":\"1\",\"ok\":true,\"result\":{\"capabilities\":[]}}\n{\"type\":\"event\",\"name\":\"session.changed\",\"payload\":{\"x\":1}}\n").unwrap();
        let m = c.wait(Duration::from_secs(2)).unwrap();
        assert_eq!(m, CoreMsg::Response { id: "1".into(), method: "core.hello".into(), ok: true, result: json!({"capabilities": []}), error_code: "".into(), error_message: "".into() });
        let m = c.wait(Duration::from_secs(2)).unwrap();
        assert_eq!(m, CoreMsg::Event { name: "session.changed".into(), payload: json!({"x": 1}) });
        assert!(c.pending().is_empty());
        drop(w);
        drop(b);
        drop(core);
        assert_eq!(c.wait(Duration::from_secs(2)), Some(CoreMsg::Gone));
        assert!(c.poll().is_empty());
    }
}
