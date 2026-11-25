//! Stand-ins for the network, the clock and the disk, so that every path of the adapter runs in a test in microseconds.

use crate::http::{Http, HttpError, Resp};
use crate::{Adapter, Clock, files};
use lumagame_core::ipc::{Display, Event, FromEngine, Hello, PROTOCOL, Request};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The account every test signs in as.
pub const STEAMID: &str = "76561198000000001";
/// 2027-01-15: a plausible now for a token to be alive in.
pub const UNIX0: u64 = 1_800_000_000;

/// A reply that Steam counts as a success.
pub fn ok(body: &str) -> Result<Resp, HttpError> {
    eresult(1, body)
}

/// A reply with Steam's own result code, whatever the HTTP status says.
pub fn eresult(code: i32, body: &str) -> Result<Resp, HttpError> {
    Ok(Resp { status: 200, eresult: Some(code), body: body.as_bytes().to_vec() })
}

/// An answer with no result code, as the older web API endpoints give.
pub fn reply(code: u16, body: &str) -> Result<Resp, HttpError> {
    Ok(Resp { status: code, eresult: None, body: body.as_bytes().to_vec() })
}

/// A reply with an HTTP status and nothing else.
pub fn status(code: u16) -> Result<Resp, HttpError> {
    Ok(Resp { status: code, eresult: None, body: Vec::new() })
}

pub fn down() -> Result<Resp, HttpError> {
    Err(HttpError::Unreachable)
}

/// Standard base64 is what Steam's `request_id` is; this is the URL-safe form a token's parts are in, without padding.
pub fn b64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// A token shaped like Steam's, made for the mobile app: three parts, the middle one the claims.
pub fn jwt(sub: &str, exp: Option<u64>) -> String {
    jwt_for(sub, exp, &["web", "mobile"])
}

/// The same for a token whose audience is `aud`.
pub fn jwt_for(sub: &str, exp: Option<u64>, aud: &[&str]) -> String {
    let aud = aud.iter().map(|name| format!("\"{name}\"")).collect::<Vec<_>>().join(",");
    let claims = match exp {
        Some(exp) => format!(r#"{{"sub":"{sub}","aud":[{aud}],"exp":{exp}}}"#),
        None => format!(r#"{{"sub":"{sub}","aud":[{aud}]}}"#),
    };
    format!("{}.{}.c2ln", b64url(br#"{"alg":"none"}"#), b64url(claims.as_bytes()))
}

/// What `GetOwnedGames` answers for `(appid, name, minutes played, last played)`; 0 leaves a field out.
pub fn owned(games: &[(u32, &str, u64, u64)]) -> String {
    let games: Vec<String> = games
        .iter()
        .map(|&(appid, name, minutes, last)| {
            let mut game = format!(r#"{{"appid":{appid}"#);
            if !name.is_empty() {
                game += &format!(r#","name":"{name}""#);
            }
            game += &format!(r#","playtime_forever":{minutes},"img_icon_url":"x""#);
            if last > 0 {
                game += &format!(r#","rtime_last_played":{last}"#);
            }
            game + "}"
        })
        .collect();
    format!(r#"{{"response":{{"game_count":{},"games":[{}]}}}}"#, games.len(), games.join(","))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub post: bool,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Default)]
struct Fake {
    routes: Vec<(String, VecDeque<Result<Resp, HttpError>>)>,
    sent: Vec<Sent>,
}

/// A network that answers from a script. Replies go to the requests whose address holds `needle`, in order, and the last one repeats.
#[derive(Clone, Default)]
pub struct FakeHttp(Rc<RefCell<Fake>>);

impl FakeHttp {
    /// Scripts `needle`, replacing an earlier script for the same word.
    pub fn on(&self, needle: &str, replies: impl IntoIterator<Item = Result<Resp, HttpError>>) {
        let mut fake = self.0.borrow_mut();
        fake.routes.retain(|(known, _)| known != needle);
        fake.routes.push((needle.to_owned(), replies.into_iter().collect()));
    }

    pub fn sent(&self) -> Vec<Sent> {
        self.0.borrow().sent.clone()
    }

    /// The requests whose address holds `needle`.
    pub fn to(&self, needle: &str) -> Vec<Sent> {
        self.sent().into_iter().filter(|sent| sent.url.contains(needle)).collect()
    }

    pub fn count(&self, needle: &str) -> usize {
        self.to(needle).len()
    }

    /// Starts the record of requests again.
    pub fn forget(&self) {
        self.0.borrow_mut().sent.clear();
    }

    fn reply(&self, post: bool, url: &str, headers: &[(&str, &str)], body: &str, limit: usize) -> Result<Resp, HttpError> {
        let mut fake = self.0.borrow_mut();
        fake.sent.push(Sent { post, url: url.to_owned(), headers: headers.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect(), body: body.to_owned() });
        // The profile page is best effort, so a test that does not script it sees Steam unreachable.
        let Some((_, replies)) = fake.routes.iter_mut().find(|(needle, _)| url.contains(needle.as_str())) else {
            return if url.contains("/profiles/") { Err(HttpError::Unreachable) } else { panic!("no scripted reply for {url}") };
        };
        let reply = if replies.len() > 1 { replies.pop_front() } else { replies.front().cloned() };
        match reply.expect("a script holds a reply") {
            Ok(resp) if resp.body.len() > limit => Err(HttpError::TooLarge),
            other => other,
        }
    }
}

impl Http for FakeHttp {
    fn get(&self, url: &str, limit: usize) -> Result<Resp, HttpError> {
        self.reply(false, url, &[], "", limit)
    }

    fn get_within(&self, url: &str, headers: &[(&str, &str)], limit: usize, _timeout: Duration) -> Result<Resp, HttpError> {
        self.reply(false, url, headers, "", limit)
    }

    fn post_form(&self, url: &str, headers: &[(&str, &str)], body: &str, limit: usize) -> Result<Resp, HttpError> {
        self.reply(true, url, headers, body, limit)
    }
}

/// A clock that moves only when told to. Wall-clock seconds follow the monotonic milliseconds from [`UNIX0`].
#[derive(Clone)]
pub struct FakeClock(Rc<Cell<u64>>);

impl FakeClock {
    pub fn new() -> Self {
        Self(Rc::new(Cell::new(1_000)))
    }

    pub fn advance_ms(&self, ms: u64) {
        self.0.set(self.0.get() + ms);
    }

    pub fn advance_s(&self, s: u64) {
        self.advance_ms(s * 1000);
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FakeClock {
    fn mono_ms(&self) -> u64 {
        self.0.get()
    }

    fn unix_s(&self) -> u64 {
        UNIX0 + self.0.get() / 1000
    }
}

#[derive(Clone, Default)]
pub struct Log(Rc<RefCell<Vec<String>>>);

impl Log {
    pub fn lines(&self) -> Vec<String> {
        self.0.borrow().clone()
    }

    pub fn sink(&self) -> impl FnMut(&str) + 'static {
        let lines = Rc::clone(&self.0);
        move |line| lines.borrow_mut().push(line.to_owned())
    }
}

/// A directory that is removed when it goes out of scope.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("lumagame-steam-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Default for TempDir {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// An adapter over empty directories, with its fakes in reach.
pub struct Rig {
    pub adapter: Adapter<FakeHttp, FakeClock>,
    pub http: FakeHttp,
    pub clock: FakeClock,
    pub log: Log,
    pub dir: TempDir,
    /// The seq of the last request sent.
    pub seq: u64,
}

impl Rig {
    /// An adapter nobody has spoken to yet. The directories do not exist: the engine makes them.
    pub fn new() -> Self {
        let (http, clock, log) = (FakeHttp::default(), FakeClock::new(), Log::default());
        let adapter = Adapter::new(http.clone(), clock.clone(), log.sink());
        Self { adapter, http, clock, log, dir: TempDir::new(), seq: 0 }
    }

    /// An adapter that has said hello and is ready, signed out.
    pub fn ready() -> Self {
        let mut rig = Self::new();
        rig.hello();
        rig
    }

    /// Ready, with a saved sign-in that is good for a month.
    pub fn signed_in() -> Self {
        Self::signed_in_until(UNIX0 + 30 * 86_400)
    }

    pub fn signed_in_until(exp: u64) -> Self {
        let mut rig = Self::new();
        rig.save_token(&jwt(STEAMID, Some(exp)), STEAMID, Some("Rig"));
        rig.hello();
        rig
    }

    pub fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    pub fn cache(&self) -> PathBuf {
        self.dir.path().join("cache")
    }

    /// Writes `token.json` as the engine does, before the engine reads it.
    pub fn save_token(&self, refresh_token: &str, steamid: &str, persona: Option<&str>) {
        let file = serde_json::json!({ "refresh_token": refresh_token, "steamid": steamid, "persona": persona });
        files::write(&self.data(), "token.json", file.to_string().as_bytes()).unwrap();
    }

    /// The hello the app would send for this rig's directories.
    pub fn hello_request(&self) -> Hello {
        Hello {
            protocol: PROTOCOL,
            app_version: "test".into(),
            data_dir: self.data().to_string_lossy().into_owned(),
            cache_dir: self.cache().to_string_lossy().into_owned(),
            feeder: None,
            locale: "en-US".into(),
            display: Display { width: 3840, height: 2160, refresh_hz: 120, hdr10: true, codecs: Vec::new() },
            settings: Default::default(),
        }
    }

    pub fn hello(&mut self) -> Vec<FromEngine> {
        let events = self.send(Request::Hello(self.hello_request()));
        assert!(matches!(events.as_slice(), [FromEngine { event: Event::Ready { .. }, .. }]), "{events:?}");
        events
    }

    pub fn send(&mut self, request: Request) -> Vec<FromEngine> {
        self.seq += 1;
        self.adapter.handle(self.seq, request)
    }

    pub fn tick(&mut self) -> Vec<FromEngine> {
        self.adapter.tick()
    }

    /// Moves the clock to where the adapter next wants to run, and runs it.
    pub fn wake(&mut self) -> Vec<FromEngine> {
        let wait = self.adapter.next_wake().expect("the adapter has something to wake for");
        self.clock.advance_ms(wait);
        self.tick()
    }

    /// A new adapter over the same directories and clock, as after the process restarts; not yet spoken to.
    pub fn restart(&mut self) {
        self.adapter = Adapter::new(self.http.clone(), self.clock.clone(), self.log.sink());
        self.seq = 0;
    }
}
