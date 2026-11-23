//! Other voice assistants in the TV's room, as Home Assistant tells it.
//!
//! The TV does not hear them; Home Assistant does. Its assist satellite entities say when a device listens, thinks and answers
//! (`listening`, `processing`, `responding`), and its debug records of the assistant's pipeline runs carry the words of the turn. The
//! watcher keeps one WebSocket to Home Assistant on the link the owner saved, finds the satellites in the TV's room (other than the
//! TV's own), follows them, and tells the overlay (`turn.rs`) what they do. The records need an administrator's token; without it the
//! overlay shows that the assistant listens and answers, and not what it heard.
//!
//! The thread runs only while the plugin is on and the room is set, reads while an assistant is active, and is otherwise one idle
//! connection. Nothing here logs what was heard or said.

use super::Surface;
use super::turn::Other;
use crate::ha_store::{self, HaLink};
use crate::voice_ws::{Fail, Ha};
use fetch::TlsFetch;
use ha_link::endpoint::AddressPolicy;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long a conversation is kept before the satellites are looked for again.
const REFRESH: Duration = Duration::from_secs(600);
const CONNECT: Duration = Duration::from_secs(10);
const ANSWER: Duration = Duration::from_secs(8);
/// How often the records are read while an assistant is active.
const LOOK: Duration = Duration::from_millis(300);
const IDLE_WAIT: Duration = Duration::from_secs(1);
const LONGEST_PAUSE: Duration = Duration::from_secs(60);
/// A record that began this long before the entity said it listens is still the turn's (the run starts a moment before).
const BEFORE_LISTENING_MS: u64 = 15_000;
const MOST_SATELLITES: usize = 4;
/// The runs of each pipeline that are looked into for a turn's record.
const MOST_CANDIDATES: usize = 3;

/// What the thread was given.
pub struct Params {
    /// Where the Home Assistant link may be, in the order tried.
    pub links: Vec<PathBuf>,
    pub tls: Option<TlsFetch>,
    /// The Home Assistant area the TV is in.
    pub room: String,
    /// The TV's own address as the voice device reports it: its own satellite is not another assistant.
    pub own_mac: String,
}

/// The watcher: stopped by dropping it.
pub struct Follower {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    said: Arc<Mutex<String>>,
}

impl Follower {
    pub fn spawn(params: Params, surface: Arc<Surface>) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let said = Arc::new(Mutex::new("Starting".to_owned()));
        let thread = {
            let (stop, said) = (stop.clone(), said.clone());
            thread::Builder::new()
                .name("voice-others".to_owned())
                .stack_size(256 * 1024)
                .spawn(move || run(&params, &surface, &said, &stop))?
        };
        Ok(Self {
            stop,
            thread: Some(thread),
            said,
        })
    }

    /// What it is doing, in words, for the settings page.
    pub fn said(&self) -> String {
        self.said.lock().clone()
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Idle,
    Listening,
    Thinking,
    Replying,
}

fn stage_of(state: &str) -> Stage {
    match state {
        "listening" => Stage::Listening,
        "processing" => Stage::Thinking,
        "responding" => Stage::Replying,
        _ => Stage::Idle,
    }
}

fn said_of(stage: Stage) -> Other {
    match stage {
        Stage::Idle => Other::Idle,
        Stage::Listening => Other::Listening,
        Stage::Thinking => Other::Thinking,
        Stage::Replying => Other::Replying,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Satellite {
    id: String,
    label: String,
}

/// A device's name as the overlay shows it: Home Assistant's names for these end in a code in brackets ("Echo Dot Gen 2 (biscuit)").
fn label_of(name: &str) -> String {
    let name = name.trim();
    let name = match name.rfind(" (") {
        Some(at) if name.ends_with(')') && at > 0 => &name[..at],
        _ => name,
    };
    name.chars().filter(|c| !c.is_control()).take(40).collect()
}

/// The template that lists the assist satellites of `room` (an area's name) other than the device with address `own_mac`, as JSON:
/// `[{"id", "name"}]`. `None` for a room name a template cannot carry safely.
fn template(room: &str, own_mac: &str) -> Option<String> {
    let safe = |text: &str| {
        !text.is_empty()
            && text.len() <= 80
            && !text
                .chars()
                .any(|c| c.is_control() || matches!(c, '\'' | '"' | '\\' | '{' | '}' | '%' | '#'))
    };
    let mac = own_mac.to_ascii_lowercase();
    if !safe(room) || !mac.chars().all(|c| c.is_ascii_hexdigit() || c == ':') {
        return None;
    }
    Some(format!(
        "{{% set ns = namespace(out=[]) %}}\
         {{% for e in area_entities('{room}') %}}\
         {{% if e.startswith('assist_satellite.') %}}\
         {{% set d = device_id(e) %}}\
         {{% if not d or ('mac', '{mac}') not in (device_attr(d, 'connections') or []) %}}\
         {{% set ns.out = ns.out + [{{'id': e, 'name': (device_attr(d, 'name_by_user') or device_attr(d, 'name') or state_attr(e, 'friendly_name') or e)}}] %}}\
         {{% endif %}}{{% endif %}}{{% endfor %}}\
         {{{{ ns.out | tojson }}}}"
    ))
}

/// The satellites a rendered template lists (a list, or the text of one).
fn found(rendered: &Value) -> Vec<Satellite> {
    let parsed;
    let list = match rendered {
        Value::String(text) => {
            parsed = serde_json::from_str::<Value>(text).unwrap_or(Value::Null);
            &parsed
        }
        other => other,
    };
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item["id"].as_str().filter(|id| id.starts_with("assist_satellite."))?;
            Some(Satellite {
                id: id.to_owned(),
                label: label_of(item["name"].as_str().unwrap_or(id)),
            })
        })
        .take(MOST_SATELLITES)
        .collect()
}

/// One entity's new state out of a `subscribe_entities` event (the first event holds all of them under `a`, the next ones hold what
/// changed under `c`, and `r` names the ones that are gone).
#[derive(Debug, PartialEq)]
struct Change {
    id: String,
    state: String,
    /// When Home Assistant says it changed, in Unix milliseconds.
    at_ms: Option<u64>,
}

fn changes(event: &Value) -> Vec<Change> {
    let at = |v: &Value| v["lc"].as_f64().map(|s| (s * 1000.0) as u64);
    let mut out = Vec::new();
    for (id, v) in event["a"].as_object().into_iter().flatten() {
        if let Some(state) = v["s"].as_str() {
            out.push(Change { id: id.clone(), state: state.to_owned(), at_ms: at(v) });
        }
    }
    for (id, v) in event["c"].as_object().into_iter().flatten() {
        if let Some(state) = v["+"]["s"].as_str() {
            out.push(Change { id: id.clone(), state: state.to_owned(), at_ms: at(&v["+"]) });
        }
    }
    for id in event["r"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        out.push(Change { id: id.to_owned(), state: "unavailable".to_owned(), at_ms: None });
    }
    out
}

/// When a record's id (a ULID: ten characters of Crockford base32 that count milliseconds) was made, by Home Assistant's clock.
fn ulid_ms(id: &str) -> Option<u64> {
    const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    id.get(..10)?.chars().try_fold(0u64, |n, c| {
        let digit = ALPHABET.find(c.to_ascii_uppercase())?;
        Some(n.checked_mul(32)? + digit as u64)
    })
}

/// What a run's events say.
#[derive(Debug, Default, PartialEq, Eq)]
struct Words {
    satellite: Option<String>,
    heard: String,
    reply: String,
    failed: String,
}

fn words_of(events: &Value) -> Words {
    let mut words = Words::default();
    for event in events.as_array().into_iter().flatten() {
        let data = &event["data"];
        match event["type"].as_str() {
            Some("run-start") => words.satellite = data["satellite_id"].as_str().map(str::to_owned),
            Some("stt-end") => words.heard = text(&data["stt_output"]["text"]),
            Some("intent-end") => {
                words.reply = text(&data["intent_output"]["response"]["speech"]["plain"]["speech"]);
            }
            Some("error") => {
                words.failed = text(&data["message"]);
                if words.failed.is_empty() {
                    words.failed = text(&data["code"]);
                }
            }
            _ => {}
        }
    }
    words
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or("").trim().to_owned()
}

/// What is known of one satellite's turn.
struct Follow {
    sat: Satellite,
    stage: Stage,
    /// The pipeline and record of the turn in hand, once found.
    run: Option<(String, String)>,
    /// Records made before this (Unix milliseconds, Home Assistant's clock) are not the turn's.
    floor_ms: u64,
    /// The record of this satellite's last turn: never the next one's.
    last_run: String,
    said: Words,
}

/// What a change of state asks for.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Say(Other),
    /// Read the records now.
    Look,
}

impl Follow {
    fn new(sat: Satellite) -> Self {
        Self { sat, stage: Stage::Idle, run: None, floor_ms: 0, last_run: String::new(), said: Words::default() }
    }

    fn advance(&mut self, state: &str, at_ms: Option<u64>, now_ms: u64) -> Vec<Step> {
        let (before, stage) = (self.stage, stage_of(state));
        if stage == before {
            return Vec::new();
        }
        self.stage = stage;
        match (before, stage) {
            (Stage::Idle, _) => {
                self.run = None;
                self.said = Words::default();
                self.floor_ms = at_ms.unwrap_or(now_ms).saturating_sub(BEFORE_LISTENING_MS);
                vec![Step::Say(said_of(stage)), Step::Look]
            }
            (_, Stage::Idle) => vec![Step::Look, Step::Say(Other::Idle)],
            _ => vec![Step::Say(said_of(stage))],
        }
    }

    /// What `words` adds to what was said of the turn.
    fn hear(&mut self, words: &Words) -> Vec<Other> {
        let mut out = Vec::new();
        if !words.heard.is_empty() && words.heard != self.said.heard {
            out.push(Other::Heard(words.heard.clone()));
            self.said.heard.clone_from(&words.heard);
        }
        if !words.reply.is_empty() && words.reply != self.said.reply {
            out.push(Other::Reply(words.reply.clone()));
            self.said.reply.clone_from(&words.reply);
        }
        if !words.failed.is_empty() && words.failed != self.said.failed {
            out.push(Other::Failed(words.failed.clone()));
            self.said.failed.clone_from(&words.failed);
        }
        out
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A conversation with Home Assistant: what arrives while a command waits for its answer is kept for the loop.
struct Conn {
    ha: Ha,
    next: u64,
    queue: VecDeque<Value>,
}

/// Home Assistant's refusal of a command, in words.
struct Refused(String);

impl Conn {
    fn send(&mut self, mut message: Value) -> Result<u64, Fail> {
        let id = self.next;
        self.next += 1;
        message["id"] = json!(id);
        self.ha.send_json(&message, Instant::now() + ANSWER)?;
        Ok(id)
    }

    /// The command's `result`; events of other subscriptions that come first are kept.
    fn call(&mut self, message: Value) -> Result<Result<Value, Refused>, Fail> {
        let id = self.send(message)?;
        let deadline = Instant::now() + ANSWER;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.ha.recv(left)? {
                Some(m) if m["id"].as_u64() == Some(id) && m["type"] == "result" => {
                    if m["success"].as_bool() == Some(true) {
                        return Ok(Ok(m["result"].clone()));
                    }
                    let said = m["error"]["message"].as_str().unwrap_or("it refused");
                    return Ok(Err(Refused(said.chars().take(100).collect())));
                }
                Some(m) => self.queue.push_back(m),
                None => return Err(Fail::Lost("Home Assistant did not answer in time".into())),
            }
        }
    }

    /// The next message, kept ones first.
    fn recv(&mut self, wait: Duration) -> Result<Option<Value>, Fail> {
        match self.queue.pop_front() {
            Some(m) => Ok(Some(m)),
            None => self.ha.recv(wait),
        }
    }
}

fn pause(stop: &AtomicBool, mut left: Duration) {
    let slice = Duration::from_millis(250);
    while !left.is_zero() && !stop.load(Ordering::Relaxed) {
        let nap = left.min(slice);
        thread::sleep(nap);
        left -= nap;
    }
}

fn run(params: &Params, surface: &Surface, said: &Mutex<String>, stop: &AtomicBool) {
    let mut pause_for = Duration::from_secs(5);
    let mut last_failure = String::new();
    while !stop.load(Ordering::Relaxed) {
        match follow(params, surface, said, stop) {
            Ok(()) => pause_for = Duration::from_secs(5),
            Err(why) => {
                if last_failure != why {
                    crate::log_info!("voice assistant: not following other assistants: {why}");
                    last_failure.clone_from(&why);
                }
                *said.lock() = why;
                pause(stop, pause_for);
                pause_for = (pause_for * 2).min(LONGEST_PAUSE);
            }
        }
    }
}

fn link_of(files: &[PathBuf]) -> Result<HaLink, String> {
    for file in files {
        match ha_store::load(file) {
            Ok(Some(link)) => return Ok(link),
            Ok(None) => {}
            Err(_) => return Err("The Home Assistant link cannot be read".to_owned()),
        }
    }
    Err("Needs the Home Assistant link".to_owned())
}

/// One conversation: connected, satellites found, followed until the stop, a failure or the time to look again.
fn follow(
    params: &Params,
    surface: &Surface,
    said: &Mutex<String>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let link = link_of(&params.links)?;
    let hard = |e: Fail| format!("Cannot follow Home Assistant: {e}");
    let ha = Ha::connect(&link, AddressPolicy::HOME, params.tls.as_ref(), Instant::now() + CONNECT)
        .map_err(hard)?;
    let mut conn = Conn { ha, next: 1, queue: VecDeque::new() };
    let satellites = discover(&mut conn, params)?;
    if satellites.is_empty() {
        *said.lock() = format!("No other assistant in {}", params.room);
        let started = Instant::now();
        while !stop.load(Ordering::Relaxed) && started.elapsed() < REFRESH {
            // Keeps the connection answering pings; nothing is expected.
            conn.recv(IDLE_WAIT).map_err(hard)?;
        }
        return Ok(());
    }
    let mut follows: HashMap<String, Follow> = satellites
        .iter()
        .map(|s| (s.id.clone(), Follow::new(s.clone())))
        .collect();
    let ids: Vec<&str> = satellites.iter().map(|s| s.id.as_str()).collect();
    let sub = conn
        .send(json!({"type": "subscribe_entities", "entity_ids": ids}))
        .map_err(hard)?;
    let names: Vec<&str> = satellites.iter().map(|s| s.label.as_str()).collect();
    let following = format!("Following {} in {}", names.join(", "), params.room);
    *said.lock() = following.clone();
    // Whether the records can be read: they need an administrator's token.
    let mut records = true;
    let mut pipelines: Vec<String> = Vec::new();
    let started = Instant::now();
    let mut looked = Instant::now();
    while !stop.load(Ordering::Relaxed) && started.elapsed() < REFRESH {
        let active = follows.values().any(|f| f.stage != Stage::Idle);
        let wait = if active { LOOK } else { IDLE_WAIT };
        if let Some(m) = conn.recv(wait).map_err(hard)? {
            if m["id"].as_u64() == Some(sub) && m["type"] == "event" {
                for change in changes(&m["event"]) {
                    let Some(f) = follows.get_mut(&change.id) else { continue };
                    for step in f.advance(&change.state, change.at_ms, unix_ms()) {
                        match step {
                            Step::Say(event) => surface.other(&f.sat.label, &event),
                            Step::Look if records => {
                                look(&mut conn, &mut pipelines, f, surface, &mut records, said, &following)
                                    .map_err(hard)?;
                            }
                            Step::Look => {}
                        }
                    }
                }
                looked = Instant::now();
                continue;
            }
        }
        if records && looked.elapsed() >= LOOK {
            looked = Instant::now();
            for f in follows.values_mut().filter(|f| f.stage != Stage::Idle) {
                look(&mut conn, &mut pipelines, f, surface, &mut records, said, &following).map_err(hard)?;
            }
        }
    }
    conn.ha.close();
    Ok(())
}

fn discover(conn: &mut Conn, params: &Params) -> Result<Vec<Satellite>, String> {
    let hard = |e: Fail| format!("Cannot follow Home Assistant: {e}");
    let Some(template) = template(&params.room, &params.own_mac) else {
        return Err("The room's name cannot be used to look for assistants".to_owned());
    };
    let id = conn
        .send(json!({"type": "render_template", "template": template, "timeout": 5, "report_errors": true}))
        .map_err(hard)?;
    let deadline = Instant::now() + ANSWER;
    let rendered = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Some(m) = conn.ha.recv(left).map_err(hard)? else {
            return Err("Home Assistant did not render the room's assistants in time".to_owned());
        };
        if m["id"].as_u64() != Some(id) {
            continue;
        }
        if m["type"] == "result" && m["success"].as_bool() != Some(true) {
            let said = m["error"]["message"].as_str().unwrap_or("it refused");
            return Err(format!("Home Assistant would not list the room's assistants: {}", clip(said)));
        }
        if m["type"] == "event" {
            if let Some(error) = m["event"]["error"].as_str() {
                return Err(format!("Home Assistant could not list the room's assistants: {}", clip(error)));
            }
            break m["event"]["result"].clone();
        }
    };
    // The subscription renders again whenever something it read changes; one answer is all that is wanted.
    let _ = conn.send(json!({"type": "unsubscribe_events", "subscription": id}));
    Ok(found(&rendered))
}

fn clip(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(100).collect()
}

/// Reads the records of the pipelines for `f`'s turn and tells the overlay what is new.
fn look(
    conn: &mut Conn,
    pipelines: &mut Vec<String>,
    f: &mut Follow,
    surface: &Surface,
    records: &mut bool,
    said: &Mutex<String>,
    following: &str,
) -> Result<(), Fail> {
    match read_turn(conn, pipelines, f) {
        Err(fail) => return Err(fail),
        Ok(Err(Refused(why))) => {
            *records = false;
            *said.lock() = format!("{following}, without the words: Home Assistant refused its records ({why})");
        }
        Ok(Ok(words)) => {
            for event in f.hear(&words) {
                surface.other(&f.sat.label, &event);
            }
        }
    }
    Ok(())
}

fn read_turn(conn: &mut Conn, pipelines: &mut Vec<String>, f: &mut Follow) -> Result<Result<Words, Refused>, Fail> {
    if pipelines.is_empty() {
        match conn.call(json!({"type": "assist_pipeline/pipeline/list"}))? {
            Ok(list) => {
                *pipelines = list["pipelines"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p["id"].as_str().map(str::to_owned))
                    .collect();
            }
            Err(refused) => return Ok(Err(refused)),
        }
    }
    if f.run.is_none() {
        for pipeline in pipelines.iter() {
            let runs = match conn.call(json!({"type": "assist_pipeline/pipeline_debug/list", "pipeline_id": pipeline}))? {
                Ok(list) => list,
                Err(refused) => return Ok(Err(refused)),
            };
            let mut ids: Vec<&str> = runs["pipeline_runs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r["pipeline_run_id"].as_str())
                .filter(|id| ulid_ms(id).is_some_and(|ms| ms >= f.floor_ms) && *id > f.last_run.as_str())
                .collect();
            ids.sort_unstable_by(|a, b| b.cmp(a));
            for id in ids.into_iter().take(MOST_CANDIDATES) {
                let record = match conn.call(json!({"type": "assist_pipeline/pipeline_debug/get", "pipeline_id": pipeline, "pipeline_run_id": id}))? {
                    Ok(record) => record,
                    Err(refused) => return Ok(Err(refused)),
                };
                if words_of(&record["events"]).satellite.as_deref() == Some(f.sat.id.as_str()) {
                    f.run = Some((pipeline.clone(), id.to_owned()));
                    f.last_run = id.to_owned();
                    break;
                }
            }
            if f.run.is_some() {
                break;
            }
        }
    }
    let Some((pipeline, id)) = f.run.clone() else {
        return Ok(Ok(Words::default()));
    };
    match conn.call(json!({"type": "assist_pipeline/pipeline_debug/get", "pipeline_id": pipeline, "pipeline_run_id": id}))? {
        Ok(record) => Ok(Ok(words_of(&record["events"]))),
        Err(refused) => Ok(Err(refused)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_devices_name_loses_the_code_in_brackets() {
        assert_eq!(label_of("Echo Dot Gen 2 (biscuit)"), "Echo Dot Gen 2");
        assert_eq!(label_of("  Kitchen Dot "), "Kitchen Dot");
        assert_eq!(label_of("(odd)"), "(odd)");
        assert_eq!(label_of(&"x".repeat(100)).chars().count(), 40);
    }

    #[test]
    fn the_template_carries_the_room_and_leaves_out_the_tvs_own_device() {
        let t = template("Living Room", "AA:23:FE:01:02:02").unwrap();
        assert!(t.contains("area_entities('Living Room')"));
        assert!(t.contains("('mac', 'aa:23:fe:01:02:02') not in"));
        assert!(t.starts_with("{% set ns") && t.ends_with("{{ ns.out | tojson }}"));
        for room in ["", "Bob's room", "a\"b", "{{ x }}", "a%b", "a\\b", "line\nbreak", &"r".repeat(81)] {
            assert_eq!(template(room, "AA:23:FE:01:02:02"), None, "{room:?}");
        }
        assert_eq!(template("Living Room", "not a mac;"), None);
    }

    #[test]
    fn satellites_are_read_from_a_list_or_its_text_and_only_assist_satellites() {
        let list = json!([
            {"id": "assist_satellite.living_room_voice_satellite_assist_satellite", "name": "Echo Dot Gen 2 (biscuit)"},
            {"id": "light.lamp", "name": "Lamp"},
            {"id": "assist_satellite.kitchen"}
        ]);
        let want = vec![
            Satellite { id: "assist_satellite.living_room_voice_satellite_assist_satellite".into(), label: "Echo Dot Gen 2".into() },
            Satellite { id: "assist_satellite.kitchen".into(), label: "assist_satellite.kitchen".into() },
        ];
        assert_eq!(found(&list), want);
        assert_eq!(found(&Value::String(list.to_string())), want);
        assert!(found(&Value::Null).is_empty());
        assert!(found(&json!("not json")).is_empty());
    }

    #[test]
    fn the_compressed_events_give_the_states_that_changed() {
        let first = json!({"a": {"assist_satellite.dot": {"s": "idle", "a": {"friendly_name": "Dot"}, "c": "01M", "lc": 1795403607.3145201}}});
        assert_eq!(
            changes(&first),
            [Change { id: "assist_satellite.dot".into(), state: "idle".into(), at_ms: Some(1_795_403_607_314) }]
        );
        let next = json!({"c": {"assist_satellite.dot": {"+": {"s": "listening", "lc": 1795403610.5}, "-": {"a": ["x"]}}}});
        assert_eq!(
            changes(&next),
            [Change { id: "assist_satellite.dot".into(), state: "listening".into(), at_ms: Some(1_795_403_610_500) }]
        );
        let only_attributes = json!({"c": {"assist_satellite.dot": {"+": {"a": {"x": 1}}}}});
        assert!(changes(&only_attributes).is_empty());
        let gone = json!({"r": ["assist_satellite.dot"]});
        assert_eq!(changes(&gone)[0].state, "unavailable");
    }

    #[test]
    fn a_record_id_tells_when_home_assistant_made_it() {
        // A real run of the owner's: the list said 2026-11-23T03:13:21.941685+00:00.
        assert_eq!(ulid_ms("01M8369P0NMP2HGRCH6VCNG4V4"), Some(1_795_403_601_941));
        assert_eq!(ulid_ms("short"), None);
        assert_eq!(ulid_ms("01M8369P0U"), None, "U is not in the alphabet");
    }

    #[test]
    fn a_run_says_which_satellite_began_it_what_it_heard_and_what_it_answered() {
        let events = json!([
            {"type": "run-start", "data": {"pipeline": "p", "satellite_id": "assist_satellite.dot", "tts_output": {}}},
            {"type": "stt-start", "data": {}},
            {"type": "stt-end", "data": {"stt_output": {"text": " what time is it "}}},
            {"type": "intent-end", "data": {"processed_locally": true, "intent_output": {"response": {"speech": {"plain": {"speech": "It is 5 o'clock"}}}}}},
            {"type": "run-end", "data": null}
        ]);
        assert_eq!(
            words_of(&events),
            Words {
                satellite: Some("assist_satellite.dot".into()),
                heard: "what time is it".into(),
                reply: "It is 5 o'clock".into(),
                failed: String::new()
            }
        );
        let failed = json!([{"type": "error", "data": {"code": "stt-no-text-recognized", "message": "No text recognized"}}]);
        assert_eq!(words_of(&failed).failed, "No text recognized");
        assert_eq!(words_of(&json!(null)), Words::default());
    }

    fn follow_one() -> Follow {
        Follow::new(Satellite { id: "assist_satellite.dot".into(), label: "Echo Dot Gen 2".into() })
    }

    #[test]
    fn a_turn_goes_from_listening_through_thinking_to_idle_and_the_records_are_read_when_it_starts_and_ends() {
        let mut f = follow_one();
        assert!(f.advance("idle", None, 10_000_000).is_empty());
        assert_eq!(
            f.advance("listening", Some(2_000_000), 10_000_000),
            [Step::Say(Other::Listening), Step::Look]
        );
        assert_eq!(f.floor_ms, 2_000_000 - BEFORE_LISTENING_MS);
        assert!(f.advance("listening", None, 0).is_empty(), "no change, no step");
        assert_eq!(f.advance("processing", None, 0), [Step::Say(Other::Thinking)]);
        assert_eq!(f.advance("responding", None, 0), [Step::Say(Other::Replying)]);
        assert_eq!(f.advance("idle", None, 0), [Step::Look, Step::Say(Other::Idle)]);
        assert!(f.advance("unavailable", None, 0).is_empty(), "unavailable is idle");
    }

    #[test]
    fn a_new_turn_forgets_the_last_ones_record_and_words() {
        let mut f = follow_one();
        f.advance("listening", Some(5_000_000), 0);
        f.run = Some(("p".into(), "01A".into()));
        f.last_run = "01A".into();
        let heard = Words { heard: "one".into(), ..Words::default() };
        assert_eq!(f.hear(&heard), [Other::Heard("one".into())]);
        assert!(f.hear(&heard).is_empty(), "said once");
        f.advance("idle", None, 0);
        f.advance("listening", Some(6_000_000), 0);
        assert_eq!(f.run, None);
        assert_eq!(f.last_run, "01A", "the last record is not the next turn's");
        assert_eq!(f.hear(&heard), [Other::Heard("one".into())]);
    }

    #[test]
    fn heard_then_reply_then_failure_are_each_said_when_they_first_appear() {
        let mut f = follow_one();
        let mut words = Words::default();
        assert!(f.hear(&words).is_empty());
        words.heard = "turn on the light".into();
        words.reply = "Done".into();
        assert_eq!(f.hear(&words), [Other::Heard("turn on the light".into()), Other::Reply("Done".into())]);
        words.reply = "Done again".into();
        words.failed = "boom".into();
        assert_eq!(f.hear(&words), [Other::Reply("Done again".into()), Other::Failed("boom".into())]);
    }
}
