//! The display lab as a model (`docs/research/display-lab-result.md`): pure, with the screen contract of the kit (`update(Input, now) ->
//! Vec<Effect>`, `view()`, `due_at()`), so the whole flow is tested on the host with nothing but a clock.
//!
//! What a person goes through: an intro that asks the helper for its session (and says plainly when the TV cannot do this), then one step
//! after another (`plan.rs`): the left and right keys, the wheel and a click on A or B switch what the screen shows between the two options,
//! and the person answers, skips or leaves. A run ends on a screen of what was answered, which is saved on the TV and sent to the helper.
//!
//! The registers are the helper's (`helper/app/src/displaylab/`): this model only asks, one request at a time and always in the same words
//! (a control and one of its options), tells the helper it is there every second, and puts everything back by asking for the end on every way
//! out (Back, Home, the screen going off, another app coming up, the end of the run). A word that does not come (the shell died, the link
//! broke) is the helper's to answer: it puts everything back after six seconds of silence.

use super::plan::{Answer, Ask, STEPS, StepDef, step_index};
use crate::kit::screen::{Effect, Input};
use crate::panels::life::{Change, ENTER_MS, EXIT_MS, Life, Phase};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

pub const TAG_BEGIN: u64 = 0x4C41_0001;
pub const TAG_SET: u64 = 0x4C41_0002;
pub const TAG_PING: u64 = 0x4C41_0003;
pub const TAG_END: u64 = 0x4C41_0004;
pub const TAG_RESULTS: u64 = 0x4C41_0005;

/// The run's answers under the data directory (the shell's file effects name it relative to it).
pub const RESULTS_FILE: &str = "displaylab/results.json";
/// How often the lab tells the helper it is there; the helper puts everything back after six seconds without a word.
pub const PING_MS: u64 = 1000;
/// A lab nobody has touched for this long ends by itself: whoever walked away is not left with a look the set was changed to (or with the saver
/// over a lab that is still pinging).
pub const IDLE_MS: u64 = 180_000;
/// How long an answer of the helper is waited for.
const ANSWER_MS: u64 = 5000;
/// LG's Home, which is what is in front when nothing plays.
const HOME_APP: &str = "com.webos.app.home";

/// Where a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Intro,
    Step,
    Results,
}

/// The state of the helper's session as far as the lab knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// Asked for; no answer yet.
    Beginning,
    Ready,
    /// The helper said no, and why in its own words: this TV cannot do the lab (or the helper is not there).
    Refused(String),
    /// The session is gone (the helper put everything back, a write was refused): the next step starts a new one.
    Lost(String),
    /// The run is over and the helper has been told to put everything back.
    Ended,
}

/// What the helper is asked, one at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Cmd {
    Begin,
    Set(&'static str, &'static str),
    Ping,
    End,
}

/// What a person can press on a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The one on the screen is better.
    Pick,
    Yes,
    No,
    Same,
    Skip,
    /// The look the person asked for never showed: the one way on, in place of every answer about a look that was not on the screen.
    Failed,
}

impl Reply {
    pub fn label(self) -> &'static str {
        match self {
            Reply::Pick => "This one is better",
            Reply::Yes => "Yes",
            Reply::No => "No",
            Reply::Same => "Can't tell",
            Reply::Skip => "Skip",
            Reply::Failed => "Next",
        }
    }
}

/// The answers a step takes, in the order they are offered: the ones that say something first.
pub fn replies(ask: Ask) -> &'static [Reply] {
    match ask {
        Ask::Compare => &[Reply::Pick, Reply::Same, Reply::Skip],
        Ask::Question(_) => &[Reply::Yes, Reply::No, Reply::Same, Reply::Skip],
    }
}

/// What a click can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Start,
    /// The switch to option 0 (A) or 1 (B).
    Chip(usize),
    Answer(usize),
    Done,
    Again,
    /// The intro's way out when the lab cannot run.
    Leave,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptView {
    pub letter: &'static str,
    pub name: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepView {
    pub id: &'static str,
    pub group: &'static str,
    pub counter: String,
    pub card: super::plan::Card,
    pub look: &'static str,
    pub hint: &'static str,
    pub options: [OptView; 2],
    /// The option on the screen (0: A, 1: B).
    pub shown: usize,
    pub question: &'static str,
    pub replies: Vec<Reply>,
    pub focus: usize,
    /// What is not as it should be, in words: the helper's own, or what the lab found.
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntroView {
    /// The helper has given its session: Start can be pressed.
    pub ready: bool,
    /// The helper said no, or did not answer: only leaving is left.
    pub refused: bool,
    /// Why the lab cannot run, when it cannot.
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultsView {
    /// What was compared, and what the person said of it.
    pub rows: Vec<(String, String)>,
    /// Whether the run is kept: on the TV, and with the helper.
    pub saved: String,
    pub focus: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Intro(IntroView),
    Step(StepView),
    Results(ResultsView),
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub phase: Phase,
    pub body: Body,
}

/// How the run was kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Kept {
    /// The file was written (the shell's write has no answer: it is as honest as that).
    file: bool,
    /// The helper's answer to the run: `None` while nothing has been sent or nothing has come back.
    sent: Option<bool>,
}

pub struct Lab {
    life: Life,
    open: bool,
    stage: Stage,
    link: Link,
    at: usize,
    shown: usize,
    focus: usize,
    answers: Vec<Option<Answer>>,
    /// Whether what the helper wrote still held when the step was answered.
    held_at: Vec<Option<bool>>,
    /// The controls the helper cannot use, and why.
    unusable: BTreeMap<String, String>,
    /// The option each control shows now, as the helper said.
    confirmed: BTreeMap<&'static str, &'static str>,
    held: bool,
    note: Option<String>,
    queue: VecDeque<Cmd>,
    in_flight: Option<(Cmd, u64)>,
    next_ping: Option<u64>,
    fg: String,
    over: String,
    run: u64,
    started: u64,
    step_since: u64,
    times: Vec<Option<u64>>,
    /// How often the person switched between the two looks in each step, and whether B was on the screen at all: an answer given without looking at
    /// both is not much of an answer, and the results say so.
    switches: Vec<u32>,
    saw_b: Vec<bool>,
    /// Whether B was ever on the screen in the step, as the helper said: it wrote B and what it wrote read back as written. A step whose B never
    /// was is not compared: its answers are replaced by the one that goes on.
    applied_b: Vec<bool>,
    /// The last key or pointer the lab felt.
    last_active: u64,
    kept: Kept,
}

/// The first option of a control: the set's own state, which is what a control is put back to when the lab moves on.
fn first_option(control: &str) -> Option<&'static str> {
    STEPS.iter().find(|s| s.control == control).map(|s| s.a.id)
}

fn request(cmd: &Cmd) -> Effect {
    let (tag, path, body) = match cmd {
        Cmd::Begin => (TAG_BEGIN, "/v1/displaylab/begin", json!({})),
        Cmd::Set(control, option) => (TAG_SET, "/v1/displaylab/set", json!({"control": control, "option": option})),
        Cmd::Ping => (TAG_PING, "/v1/displaylab/ping", json!({})),
        Cmd::End => (TAG_END, "/v1/displaylab/end", json!({})),
    };
    Effect::Request { tag, method: "POST".into(), path: path.into(), body: Some(body) }
}

/// The helper's own words for what went wrong, or ours when it said none (it did not answer at all).
fn why(status: u16, data: &Value) -> String {
    let said = data.pointer("/error/message").and_then(Value::as_str).filter(|m| !m.is_empty());
    match (said, status) {
        (Some(m), _) => m.to_owned(),
        (None, 0) => "The TV's helper did not answer.".to_owned(),
        (None, s) => format!("The TV's helper said {s}."),
    }
}

impl Default for Lab {
    fn default() -> Self {
        Self::new()
    }
}

impl Lab {
    pub fn new() -> Self {
        Self {
            life: Life::new(ENTER_MS, EXIT_MS, 0),
            open: false,
            stage: Stage::Intro,
            link: Link::Beginning,
            at: 0,
            shown: 0,
            focus: 0,
            answers: vec![None; STEPS.len()],
            held_at: vec![None; STEPS.len()],
            unusable: BTreeMap::new(),
            confirmed: BTreeMap::new(),
            held: true,
            note: None,
            queue: VecDeque::new(),
            in_flight: None,
            next_ping: None,
            fg: String::new(),
            over: String::new(),
            run: 0,
            started: 0,
            step_since: 0,
            times: vec![None; STEPS.len()],
            switches: vec![0; STEPS.len()],
            saw_b: vec![false; STEPS.len()],
            applied_b: vec![false; STEPS.len()],
            last_active: 0,
            kept: Kept::default(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The lab has the keys while it is open (and while it fades out: it takes none then, the screen under it does).
    pub fn wants(&self) -> bool {
        self.open
    }

    pub fn phase(&self) -> Phase {
        self.life.phase()
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn link(&self) -> &Link {
        &self.link
    }

    /// The step on the screen, when a step is.
    pub fn step(&self) -> Option<&'static StepDef> {
        (self.stage == Stage::Step).then(|| &STEPS[self.at])
    }

    pub fn shown(&self) -> usize {
        self.shown
    }

    pub fn answers(&self) -> &[Option<Answer>] {
        &self.answers
    }

    /// Whether anything is waiting for the helper (a request is out or queued).
    pub fn busy(&self) -> bool {
        self.in_flight.is_some() || !self.queue.is_empty()
    }

    // ---- opening and closing

    /// Everything of a run is as it was before the first step: the answers, the helper's session, the queue. What goes across runs is the
    /// window the lab is in (`life`, `open`), what is in front, and how the last run was kept.
    fn reset_run(&mut self, now: u64) {
        let (life, open, fg, kept) = (std::mem::replace(&mut self.life, Life::new(ENTER_MS, EXIT_MS, 0)), self.open, self.fg.clone(), self.kept);
        *self = Self { life, open, over: fg.clone(), fg, kept, run: now, started: now, ..Self::new() };
    }

    /// Opens the lab on its intro and asks the helper for its session.
    pub fn open(&mut self, now: u64) -> Vec<Effect> {
        if self.open {
            return Vec::new();
        }
        self.reset_run(now);
        self.kept = Kept::default();
        self.open = true;
        self.last_active = now;
        let _ = self.life.show(now);
        let mut out = vec![Effect::Log("displaylab: opened".into()), Effect::Redraw];
        self.push(Cmd::Begin);
        self.pump(now, &mut out);
        out
    }

    /// Leaves: everything is put back (the helper is asked to), what was answered is kept, and the lab fades out.
    pub fn close(&mut self, why: &str, now: u64) -> Vec<Effect> {
        if !self.open {
            return Vec::new();
        }
        self.open = false;
        let mut out = vec![Effect::Log(format!("displaylab: closed ({why})")), Effect::Redraw];
        if self.answers.iter().any(Option::is_some) && self.stage != Stage::Results {
            out.extend(self.keep(now, false));
        }
        // anything still queued is moot; the end goes after what is already out
        self.queue.clear();
        if !matches!(self.link, Link::Refused(_) | Link::Ended) {
            self.queue.push_back(Cmd::End);
        }
        self.link = Link::Ended;
        self.next_ping = None;
        let _ = self.life.hide(now);
        self.pump(now, &mut out);
        out
    }

    // ---- the queue of requests

    fn push(&mut self, cmd: Cmd) {
        if let Cmd::Set(control, _) = &cmd {
            // a switch that has not gone out is replaced by the newer one for the same control
            self.queue.retain(|c| !matches!(c, Cmd::Set(other, _) if other == control));
            // a session that is gone is started again before anything is written
            if matches!(self.link, Link::Lost(_)) && !self.queue.contains(&Cmd::Begin) && !matches!(&self.in_flight, Some((Cmd::Begin, _))) {
                self.queue.push_back(Cmd::Begin);
                self.link = Link::Beginning;
            }
        }
        self.queue.push_back(cmd);
    }

    fn pump(&mut self, now: u64, out: &mut Vec<Effect>) {
        if self.in_flight.is_some() {
            return;
        }
        if let Some(cmd) = self.queue.pop_front() {
            out.push(request(&cmd));
            self.in_flight = Some((cmd, now));
        }
    }

    // ---- the run

    /// What the screen shows for the step: the A the set starts from, and the control put back when the lab moves to another one.
    fn enter_step(&mut self, i: usize, now: u64, out: &mut Vec<Effect>) {
        let s = &STEPS[i];
        if let Some(prev) = self.step().filter(|p| p.control != s.control)
            && let Some(first) = first_option(prev.control)
            && self.confirmed.get(prev.control).is_some_and(|c| *c != first)
        {
            self.push(Cmd::Set(prev.control, first));
        }
        self.stage = Stage::Step;
        self.at = i;
        self.shown = 0;
        self.focus = 0;
        self.note = self.note_for(s);
        self.step_since = now;
        self.push(Cmd::Set(s.control, s.a.id));
        self.pump(now, out);
        out.push(Effect::Redraw);
    }

    /// What the lab can say about the step before the person has looked at it.
    fn note_for(&self, s: &StepDef) -> Option<String> {
        if let Some(why) = self.unusable.get(s.control) {
            return Some(format!("The TV cannot do this one: {why}"));
        }
        // the window shows what the video plane holds: Home's own wallpaper may move, so only what is certain is said
        if s.control == "window" && self.fg == HOME_APP {
            return Some("Home is behind this. A playing video shows the window best.".to_owned());
        }
        None
    }

    fn next_step(&self, after: usize) -> Option<usize> {
        (after + 1..STEPS.len()).find(|i| match STEPS[*i].skip_after.and_then(step_index) {
            Some(j) => self.answers[j] != Some(Answer::Yes),
            None => true,
        })
    }

    fn first_step(&self) -> usize {
        0
    }

    fn answer(&mut self, a: Answer, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if self.stage != Stage::Step {
            return out;
        }
        let i = self.at;
        self.answers[i] = Some(a);
        self.held_at[i] = Some(self.held);
        self.times[i] = Some(now.saturating_sub(self.step_since));
        out.extend(self.keep(now, false));
        match self.next_step(i) {
            Some(n) => self.enter_step(n, now, &mut out),
            None => self.finish(now, &mut out),
        }
        out
    }

    /// The last step is answered: everything is put back while the results are read, and the run is kept and sent.
    fn finish(&mut self, now: u64, out: &mut Vec<Effect>) {
        self.stage = Stage::Results;
        self.focus = 0;
        self.note = None;
        self.queue.clear();
        if !matches!(self.link, Link::Refused(_)) {
            self.queue.push_back(Cmd::End);
        }
        self.link = Link::Ended;
        self.next_ping = None;
        out.extend(self.keep(now, true));
        self.pump(now, out);
        out.push(Effect::Redraw);
    }

    /// The run as the results file says it.
    fn doc(&self, now: u64, complete: bool) -> Value {
        let behind = if self.over.is_empty() || self.over == HOME_APP { "home".to_owned() } else { format!("app:{}", self.over) };
        let steps: Vec<Value> = STEPS
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let a = self.answers[i]?;
                Some(json!({
                    "id": s.id, "group": s.group, "control": s.control, "a": s.a.id, "b": s.b.id,
                    "answer": a.word(), "verdict": s.verdict(a), "held": self.held_at[i], "ms": self.times[i], "switches": self.switches[i], "sawBoth": self.saw_b[i], "bApplied": self.applied_b[i],
                }))
            })
            .collect();
        json!({"run": self.run.to_string(), "startedMs": self.started, "finishedMs": now, "complete": complete, "behind": behind, "steps": steps})
    }

    /// Writes the run to the data directory, and sends it to the helper when the run is over.
    fn keep(&mut self, now: u64, send: bool) -> Vec<Effect> {
        let doc = self.doc(now, self.answers.iter().enumerate().all(|(i, a)| a.is_some() || self.skipped(i)));
        let mut out = vec![Effect::Write { path: RESULTS_FILE.into(), text: doc.to_string() }];
        self.kept.file = true;
        if send {
            self.kept.sent = None;
            out.push(Effect::Request { tag: TAG_RESULTS, method: "POST".into(), path: "/v1/displaylab/results".into(), body: Some(doc) });
        }
        out
    }

    /// A step that is not answered because an earlier answer made it not needed.
    fn skipped(&self, i: usize) -> bool {
        STEPS[i].skip_after.and_then(step_index).is_some_and(|j| self.answers[j] == Some(Answer::Yes))
    }

    // ---- what comes in

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match input {
            Input::Topic { topic, data } => self.topic(topic, data, now, &mut out),
            Input::Fetched { tag, status, data } => self.answered(*tag, *status, data, now, &mut out),
            Input::Key { name, down, repeat } => {
                if self.open && *down {
                    self.last_active = now;
                }
                if self.open && *down && !*repeat {
                    out.extend(self.key(name, now));
                }
            }
            Input::Tick => self.tick(now, &mut out),
            Input::Helper { topic, data } if topic == "up" && self.open && data == &json!(false) && !matches!(self.link, Link::Refused(_) | Link::Ended) => {
                self.link = Link::Lost("The TV's helper is not answering.".into());
                self.in_flight = None;
                self.queue.clear();
                self.next_ping = None;
                self.note = self.step().map(|_| "The TV's helper is not answering.".to_owned());
                out.push(Effect::Redraw);
            }
            _ => {}
        }
        out
    }

    fn topic(&mut self, topic: &str, data: &Value, now: u64, out: &mut Vec<Effect>) {
        match topic {
            "foreground" => {
                let app = data.get("app").and_then(Value::as_str).unwrap_or("").to_owned();
                if self.open && app != self.fg && !self.over.is_empty() && app != self.over {
                    out.extend(self.close("another app came up", now));
                }
                self.fg = app;
            }
            // the Home key, a power-off: whatever is on the screen is not the lab's any more
            "homekey" => out.extend(self.close("the Home key", now)),
            "screen" if data.get("on").and_then(Value::as_bool) == Some(false) => out.extend(self.close("the screen went off", now)),
            _ => {}
        }
    }

    fn tick(&mut self, now: u64, out: &mut Vec<Effect>) {
        if let Some(Change::Opened | Change::Parked | Change::Drop | Change::Hide | Change::Show | Change::Build) = self.life.tick(now) {
            out.push(Effect::Redraw);
        }
        if !self.open {
            // the end is out or answered; a lab that is closed asks for nothing more
            if let Some((_, sent)) = &self.in_flight
                && now.saturating_sub(*sent) > ANSWER_MS
            {
                self.in_flight = None;
            }
            self.pump(now, out);
            return;
        }
        if now.saturating_sub(self.last_active) >= IDLE_MS {
            out.extend(self.close("nobody touched it", now));
            return;
        }
        if let Some((cmd, sent)) = self.in_flight.clone()
            && now.saturating_sub(sent) > ANSWER_MS
        {
            // the helper said nothing: it puts everything back by itself
            self.in_flight = None;
            self.queue.clear();
            self.link = if cmd == Cmd::Begin && self.stage == Stage::Intro {
                Link::Refused("The TV's helper did not answer.".into())
            } else {
                Link::Lost("The TV's helper did not answer.".into())
            };
            self.note = self.step().map(|_| "The TV's helper did not answer.".to_owned());
            out.push(Effect::Redraw);
        }
        if self.link == Link::Ready && self.next_ping.is_some_and(|t| now >= t) && !self.busy() {
            self.queue.push_back(Cmd::Ping);
            self.next_ping = Some(now + PING_MS);
        }
        self.pump(now, out);
    }

    /// An answer of the helper.
    fn answered(&mut self, tag: u64, status: u16, data: &Value, now: u64, out: &mut Vec<Effect>) {
        let ok = status == 200;
        if matches!(tag, TAG_BEGIN | TAG_SET | TAG_PING | TAG_END) && !self.open {
            // the lab was left while a request was out: what is queued behind it is the end, which goes now
            self.in_flight = None;
            if tag == TAG_END && !ok {
                out.push(Effect::Log(format!("displaylab: end answered {status}: {}", why(status, data))));
            }
            self.pump(now, out);
            return;
        }
        match tag {
            TAG_BEGIN => {
                self.in_flight = None;
                if ok {
                    self.link = Link::Ready;
                    self.next_ping = Some(now + PING_MS);
                    self.held = true;
                    self.unusable.clear();
                    if let Some(usable) = data.pointer("/displaylab/session/usable").and_then(Value::as_object) {
                        for (control, up) in usable {
                            if up.as_bool() == Some(false) {
                                let because = data
                                    .pointer(&format!("/displaylab/session/why/{control}"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("it cannot be read")
                                    .to_owned();
                                self.unusable.insert(control.clone(), because);
                            }
                        }
                    }
                    if let Some(s) = self.step() {
                        self.note = self.note_for(s);
                    }
                } else {
                    let reason = why(status, data);
                    self.link = if self.stage == Stage::Intro { Link::Refused(reason.clone()) } else { Link::Lost(reason.clone()) };
                    self.queue.clear();
                    if self.stage == Stage::Step {
                        self.note = Some(reason);
                    }
                    out.push(Effect::Log(format!("displaylab: begin refused ({status})")));
                }
                out.push(Effect::Redraw);
            }
            TAG_SET => {
                let sent = self.in_flight.take();
                if ok {
                    let mut asked_b = false;
                    if let Some((Cmd::Set(control, option), _)) = sent {
                        self.confirmed.insert(control, option);
                        asked_b = self.step().is_some_and(|s| s.control == control && s.b.id == option);
                    }
                    self.held = self.holds(data);
                    if self.held {
                        self.applied_b[self.at] |= asked_b;
                        self.note = self.step().and_then(|s| self.note_for(s));
                    } else {
                        // the write went in and read back, and the firmware took it back before the answer: the screen shows what it showed before
                        self.note = Some(self.not_held_note());
                    }
                    self.focus = self.focus.min(self.replies().len() - 1);
                    self.next_ping = Some(now + PING_MS);
                } else {
                    // a refused or failed write ends the helper's session, with everything put back: the step is told, the next one starts again
                    let reason = why(status, data);
                    self.link = Link::Lost(reason.clone());
                    self.confirmed.clear();
                    self.queue.clear();
                    self.note = Some(reason);
                    self.focus = self.focus.min(self.replies().len() - 1);
                    self.next_ping = None;
                    out.push(Effect::Log(format!("displaylab: set refused ({status})")));
                }
                out.push(Effect::Redraw);
            }
            TAG_PING => {
                self.in_flight = None;
                if ok {
                    let held = self.holds(data);
                    if held != self.held {
                        self.held = held;
                        self.note = if held { self.step().and_then(|s| self.note_for(s)) } else { Some(self.not_held_note()) };
                        out.push(Effect::Redraw);
                    }
                } else {
                    let reason = if status == 409 { "The TV put everything back: it had not heard from the lab.".to_owned() } else { why(status, data) };
                    self.link = Link::Lost(reason.clone());
                    self.confirmed.clear();
                    self.queue.clear();
                    self.next_ping = None;
                    if self.stage == Stage::Step {
                        self.note = Some(reason);
                    }
                    out.push(Effect::Redraw);
                }
            }
            TAG_END => {
                self.in_flight = None;
                if !ok {
                    out.push(Effect::Log(format!("displaylab: end answered {status}: {}", why(status, data))));
                }
            }
            TAG_RESULTS => {
                self.kept.sent = Some(ok);
                out.push(Effect::Redraw);
            }
            _ => return,
        }
        self.pump(now, out);
    }

    // ---- keys and clicks

    fn key(&mut self, name: &str, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match (self.stage, name) {
            (_, "back") => return self.close("Back", now),
            (Stage::Intro, "ok") => return self.activate(Target::Start, now),
            (Stage::Intro, _) => {}
            (Stage::Step, "left" | "right" | "wheel") => self.toggle(None, now, &mut out),
            (Stage::Step, "up") => self.focus = self.focus.saturating_sub(1),
            (Stage::Step, "down") => self.focus = (self.focus + 1).min(self.replies().len() - 1),
            (Stage::Step, "ok") => return self.activate(Target::Answer(self.focus.min(self.replies().len() - 1)), now),
            (Stage::Results, "up") => self.focus = 0,
            (Stage::Results, "down") => self.focus = 1,
            (Stage::Results, "left" | "right") => self.focus = 1 - self.focus.min(1),
            (Stage::Results, "ok") => return self.activate(if self.focus == 0 { Target::Done } else { Target::Again }, now),
            _ => {}
        }
        out.push(Effect::Redraw);
        out
    }

    /// The person asked for B and it never showed: there is nothing to compare, and nothing to say about a look that was not on the screen.
    fn unapplied(&self) -> bool {
        self.stage == Stage::Step && self.saw_b[self.at] && !self.applied_b[self.at]
    }

    fn replies(&self) -> &'static [Reply] {
        if self.unapplied() { &[Reply::Failed] } else { replies(STEPS[self.at].ask) }
    }

    /// Whether what the helper wrote for the step's control still holds, from an answer of its: the control's own entry when it gives one (a
    /// register the firmware took back for another control does not fail this one), else the session's.
    fn holds(&self, data: &Value) -> bool {
        self.step()
            .and_then(|s| data.pointer(&format!("/displaylab/session/heldBy/{}", s.control)).and_then(Value::as_bool))
            .or_else(|| data.pointer("/displaylab/session/held").and_then(Value::as_bool))
            .unwrap_or(true)
    }

    /// What the screen says when the look on it is not the one asked for.
    fn not_held_note(&self) -> String {
        let name = self.step().map_or("this look", |s| if self.shown == 0 { s.a.name } else { s.b.name });
        if self.shown == 1 && !self.applied_b[self.at] {
            format!("Couldn't apply {name}. The TV puts it back.")
        } else {
            format!("The TV put {name} back by itself.")
        }
    }

    /// Switches what the screen shows to the other option, or to `to`.
    fn toggle(&mut self, to: Option<usize>, now: u64, out: &mut Vec<Effect>) {
        let s = &STEPS[self.at];
        let want = to.unwrap_or(1 - self.shown);
        if want == self.shown && self.confirmed.get(s.control) == Some(&if want == 0 { s.a.id } else { s.b.id }) {
            return;
        }
        if want != self.shown {
            self.switches[self.at] += 1;
        }
        self.saw_b[self.at] |= want == 1;
        self.shown = want;
        self.push(Cmd::Set(s.control, if want == 0 { s.a.id } else { s.b.id }));
        self.pump(now, out);
    }

    /// A press on a control: the key's OK and the pointer's click are this.
    fn activate(&mut self, t: Target, now: u64) -> Vec<Effect> {
        let mut out = vec![Effect::Redraw];
        match (self.stage, t) {
            (Stage::Intro, Target::Start) => match self.link {
                Link::Ready => {
                    let first = self.first_step();
                    self.enter_step(first, now, &mut out);
                }
                // the helper has not answered yet: OK waits for it
                Link::Beginning => {}
                // a lab that cannot run has one thing left to do
                _ => return self.close("Leave", now),
            },
            (Stage::Intro, Target::Leave) => return self.close("Leave", now),
            (Stage::Results, Target::Done) => return self.close("Done", now),
            (Stage::Results, Target::Again) => {
                self.reset_run(now);
                self.push(Cmd::Begin);
                self.pump(now, &mut out);
            }
            (Stage::Step, Target::Chip(i)) => self.toggle(Some(i.min(1)), now, &mut out),
            (Stage::Step, Target::Answer(i)) => {
                let Some(r) = self.replies().get(i).copied() else { return out };
                self.focus = i;
                let a = match r {
                    Reply::Pick => Answer::Better(self.shown),
                    Reply::Yes => Answer::Yes,
                    Reply::No => Answer::No,
                    Reply::Same => Answer::Same,
                    Reply::Skip => Answer::Skipped,
                    Reply::Failed => Answer::Failed,
                };
                return self.answer(a, now);
            }
            _ => {}
        }
        out
    }

    /// A click on a control: the pointer's press is OK on it.
    pub fn click(&mut self, t: Target, now: u64) -> Vec<Effect> {
        if !self.open {
            return Vec::new();
        }
        self.last_active = now;
        if let Target::Answer(i) = t {
            self.focus = i;
        }
        self.activate(t, now)
    }

    /// The pointer is moving over the lab: somebody is there.
    pub fn touch(&mut self, now: u64) {
        if self.open {
            self.last_active = now;
        }
    }

    // ---- what is read

    pub fn due_at(&self) -> Option<u64> {
        let flight = self.in_flight.as_ref().map(|(_, sent)| sent + ANSWER_MS + 1);
        let ping = self.open.then_some(self.next_ping).flatten();
        let idle = self.open.then_some(self.last_active + IDLE_MS);
        [self.life.due_at(), flight, ping, idle].into_iter().flatten().min()
    }

    pub fn view(&self) -> Option<View> {
        if !self.life.built() {
            return None;
        }
        let body = match self.stage {
            Stage::Intro => Body::Intro(IntroView {
                ready: self.link == Link::Ready,
                refused: matches!(self.link, Link::Refused(_) | Link::Lost(_)),
                note: match &self.link {
                    Link::Refused(why) | Link::Lost(why) => Some(why.clone()),
                    Link::Beginning => Some("Getting ready\u{2026}".to_owned()),
                    _ => None,
                },
            }),
            Stage::Step => {
                let s = &STEPS[self.at];
                Body::Step(StepView {
                    id: s.id,
                    group: s.group,
                    counter: s.counter(),
                    card: s.card,
                    look: s.look,
                    hint: s.hint,
                    options: [OptView { letter: "A", name: s.a.name }, OptView { letter: "B", name: s.b.name }],
                    shown: self.shown,
                    question: match s.ask {
                        Ask::Compare => "Which looks better?",
                        Ask::Question(q) => q,
                    },
                    replies: self.replies().to_vec(),
                    focus: self.focus.min(self.replies().len() - 1),
                    note: self.note.clone(),
                })
            }
            Stage::Results => Body::Results(ResultsView {
                rows: STEPS.iter().enumerate().filter_map(|(i, s)| self.answers[i].map(|a| (s.row(), s.verdict(a)))).collect(),
                saved: match (self.kept.file, self.kept.sent) {
                    (_, Some(true)) => "Saved on the TV and sent".to_owned(),
                    (true, Some(false)) => "Saved on the TV, not sent".to_owned(),
                    (true, None) => "Saved on the TV".to_owned(),
                    (false, _) => "Not saved".to_owned(),
                },
                focus: self.focus,
            }),
        };
        Some(View { phase: self.life.phase(), body })
    }

    /// What the gate and the harness read.
    pub fn state(&self) -> Value {
        json!({
            "open": self.open, "phase": format!("{:?}", self.life.phase()).to_lowercase(),
            "stage": match self.stage { Stage::Intro => "intro", Stage::Step => "step", Stage::Results => "results" },
            "link": match &self.link { Link::Beginning => "beginning", Link::Ready => "ready", Link::Refused(_) => "refused", Link::Lost(_) => "lost", Link::Ended => "ended" },
            "step": self.step().map(|s| s.id), "shown": self.shown, "focus": self.focus, "held": self.held, "note": self.note,
            "answers": STEPS.iter().enumerate().filter_map(|(i, s)| self.answers[i].map(|a| (s.id.to_owned(), json!(a.word())))).collect::<serde_json::Map<_, _>>(),
            "busy": self.busy(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> Input {
        Input::Key { name: name.into(), down: true, repeat: false }
    }

    fn requests(fx: &[Effect]) -> Vec<(u64, String, Value)> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Request { tag, path, body, .. } => Some((*tag, path.clone(), body.clone().unwrap_or(Value::Null))),
                _ => None,
            })
            .collect()
    }

    fn set_of(r: &(u64, String, Value)) -> (String, String) {
        (r.2["control"].as_str().unwrap_or("").to_owned(), r.2["option"].as_str().unwrap_or("").to_owned())
    }

    fn fetched(tag: u64, status: u16, data: Value) -> Input {
        Input::Fetched { tag, status, data }
    }

    fn session(held: bool) -> Value {
        json!({"ok": true, "displaylab": {"available": true, "session": {"id": 1, "held": held, "usable": {"sharpness": true, "window": true}, "why": {}}}})
    }

    /// A lab, opened and begun, over Home.
    fn begun() -> (Lab, u64) {
        let mut lab = Lab::new();
        let mut now = 10_000;
        lab.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": HOME_APP, "home": true}) }, now);
        let fx = lab.open(now);
        assert_eq!(requests(&fx).iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["/v1/displaylab/begin"]);
        now += 20;
        lab.update(&fetched(TAG_BEGIN, 200, session(true)), now);
        assert_eq!(*lab.link(), Link::Ready);
        (lab, now)
    }

    /// Answers whatever the lab asked of the helper last, as a helper that works would.
    fn settle(lab: &mut Lab, fx: &[Effect], now: u64) -> Vec<Effect> {
        let mut all = Vec::new();
        let mut pending = requests(fx);
        while let Some(r) = pending.pop() {
            let data = if r.0 == TAG_SET { session(true) } else { json!({"ok": true}) };
            let more = lab.update(&fetched(r.0, 200, data), now);
            pending.extend(requests(&more));
            all.extend(more);
        }
        all
    }

    #[test]
    fn the_lab_opens_on_its_intro_asks_for_the_session_and_starts_on_ok() {
        let mut lab = Lab::new();
        assert!(!lab.is_open() && lab.view().is_none() && !lab.wants());
        let fx = lab.open(1000);
        let r = requests(&fx);
        assert_eq!((r.len(), r[0].0, r[0].1.as_str()), (1, TAG_BEGIN, "/v1/displaylab/begin"));
        let Some(View { body: Body::Intro(i), phase }) = lab.view() else { panic!("an intro") };
        assert_eq!(phase, Phase::Opening);
        assert!(!i.ready, "not before the helper has answered");
        assert_eq!(i.note.as_deref(), Some("Getting ready\u{2026}"));
        // OK before the helper answers does nothing
        assert!(requests(&lab.update(&key("ok"), 1010)).is_empty());
        assert_eq!(lab.stage(), Stage::Intro);
        lab.update(&fetched(TAG_BEGIN, 200, session(true)), 1030);
        let Some(View { body: Body::Intro(i), .. }) = lab.view() else { panic!() };
        assert!(i.ready && i.note.is_none());
        // OK starts: the first step shows A, which is the set's own state
        let fx = lab.update(&key("ok"), 1100);
        assert_eq!(lab.stage(), Stage::Step);
        let r = requests(&fx);
        assert_eq!((r.len(), set_of(&r[0])), (1, ("sharpness".into(), "normal".into())));
        let Some(View { body: Body::Step(s), .. }) = lab.view() else { panic!("a step") };
        assert_eq!((s.id, s.group, s.counter.as_str(), s.shown, s.focus), ("sharpness-strong", "Sharpness", "1 of 5", 0, 0));
        assert_eq!((s.options[0].letter, s.options[0].name, s.options[1].letter, s.options[1].name), ("A", "Normal", "B", "Strong"));
        assert_eq!(s.question, "Which looks better?");
        assert_eq!(s.replies, vec![Reply::Pick, Reply::Same, Reply::Skip]);
    }

    #[test]
    fn left_right_and_the_wheel_switch_the_option_and_the_helper_is_asked_for_each() {
        let (mut lab, mut now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        for (name, shown, option) in [("right", 1, "strong"), ("left", 0, "normal"), ("wheel", 1, "strong"), ("wheel", 0, "normal"), ("left", 1, "strong")] {
            now += 100;
            let fx = lab.update(&key(name), now);
            let r = requests(&fx);
            assert_eq!((r.len(), set_of(&r[0])), (1, ("sharpness".to_owned(), option.to_owned())), "{name}");
            assert_eq!(lab.shown(), shown, "{name}");
            settle(&mut lab, &fx, now);
        }
        // a held key is one switch: its repeats do nothing
        now += 100;
        let shown = lab.shown();
        let fx = lab.update(&Input::Key { name: "right".into(), down: true, repeat: true }, now);
        assert!(fx.is_empty() && lab.shown() == shown);
        // the release does nothing either
        assert!(lab.update(&Input::Key { name: "right".into(), down: false, repeat: false }, now).is_empty());
        // a click on a chip is a switch to that option, never an answer
        now += 100;
        let fx = lab.click(Target::Chip(0), now);
        assert_eq!(set_of(&requests(&fx)[0]), ("sharpness".into(), "normal".into()));
        assert_eq!(lab.answers().iter().filter(|a| a.is_some()).count(), 0);
    }

    #[test]
    fn switches_that_come_faster_than_the_helper_answers_are_one_request_at_a_time_and_the_last_wins() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        // the first set is out and unanswered: three more switches queue behind it, and only the last of them goes
        let mut sent = requests(&fx).len();
        for _ in 0..3 {
            sent += requests(&lab.update(&key("right"), now + 10)).len();
        }
        assert_eq!(sent, 1, "one request at a time");
        let after = lab.update(&fetched(TAG_SET, 200, session(true)), now + 20);
        let r = requests(&after);
        assert_eq!(r.len(), 1, "the newest switch alone goes next");
        assert_eq!(set_of(&r[0]).1, if lab.shown() == 0 { "normal" } else { "strong" });
        assert_eq!(lab.shown(), 1, "three switches from A end on B");
    }

    #[test]
    fn a_comparison_is_answered_with_the_option_on_the_screen_and_moves_to_the_next_step() {
        let (mut lab, mut now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        let fx = lab.update(&key("right"), now + 50);
        settle(&mut lab, &fx, now + 50);
        now += 3000;
        // OK on the first answer: B, which is on the screen, is better
        let fx = lab.update(&key("ok"), now);
        assert_eq!(lab.answers()[0], Some(Answer::Better(1)));
        assert_eq!(lab.step().unwrap().id, "sharpness-medium");
        assert_eq!(lab.shown(), 0, "the next step starts on A");
        let r = requests(&fx);
        assert_eq!(set_of(&r[0]), ("sharpness".into(), "normal".into()), "the same control: A again, nothing to put back");
        let write = fx
            .iter()
            .find_map(|e| if let Effect::Write { path, text } = e { Some((path.clone(), text.clone())) } else { None })
            .expect("the answer is kept at once");
        assert_eq!(write.0, RESULTS_FILE);
        let doc: Value = serde_json::from_str(&write.1).unwrap();
        assert_eq!(doc["steps"][0]["id"], "sharpness-strong");
        assert_eq!(doc["steps"][0]["answer"], "b");
        assert_eq!(doc["steps"][0]["verdict"], "Strong looks better");
        assert_eq!(doc["steps"][0]["ms"], 3000);
        assert_eq!(doc["steps"][0]["held"], true);
        assert_eq!((doc["steps"][0]["switches"].clone(), doc["steps"][0]["sawBoth"].clone()), (json!(1), json!(true)), "the person looked at both");
        assert_eq!(doc["behind"], "home");
        assert_eq!(doc["complete"], false);
    }

    #[test]
    fn a_lab_nobody_touches_ends_by_itself_and_a_key_or_the_pointer_keeps_it() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        let mut t = now;
        // the pings go on, answered; a key two minutes in is somebody
        while t < now + 120_000 {
            t += 1000;
            let fx = lab.update(&Input::Tick, t);
            settle(&mut lab, &fx, t);
        }
        let fx = lab.update(&key("right"), t);
        settle(&mut lab, &fx, t);
        let keyed = t;
        // two more minutes of nothing but the pointer's movement every minute: still open
        while t < keyed + 170_000 {
            t += 1000;
            if t % 60_000 == 0 {
                lab.touch(t);
            }
            let fx = lab.update(&Input::Tick, t);
            settle(&mut lab, &fx, t);
        }
        assert!(lab.is_open(), "a minute of nothing after the pointer, a hundred and fifty seconds after a key");
        assert!(lab.due_at().is_some_and(|d| d <= t + PING_MS));
        // and then nobody: three minutes after the last touch it ends and asks the helper to put everything back
        let mut ended = Vec::new();
        while lab.is_open() && t < keyed + 600_000 {
            t += 1000;
            let fx = lab.update(&Input::Tick, t);
            ended.extend(requests(&fx));
            settle(&mut lab, &fx, t);
        }
        assert!(!lab.is_open());
        assert!(t >= keyed + IDLE_MS && ended.iter().any(|r| r.1.ends_with("/end")), "{ended:?}");
        assert!(lab.answers().iter().all(Option::is_none));
    }

    #[test]
    fn an_answer_given_without_looking_at_both_looks_says_so_in_the_results() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        // OK at once, on A: nothing was compared
        let fx = lab.update(&key("ok"), now + 500);
        let doc: Value = fx.iter().find_map(|e| if let Effect::Write { text, .. } = e { serde_json::from_str(text).ok() } else { None }).expect("kept");
        assert_eq!((doc["steps"][0]["switches"].clone(), doc["steps"][0]["sawBoth"].clone()), (json!(0), json!(false)));
        // the next step: B and back to A, then OK: both were seen, twice switched
        let fx = lab.update(&key("right"), now + 600);
        settle(&mut lab, &fx, now + 600);
        let fx = lab.update(&key("left"), now + 700);
        settle(&mut lab, &fx, now + 700);
        let fx = lab.update(&key("ok"), now + 800);
        let doc: Value = fx.iter().find_map(|e| if let Effect::Write { text, .. } = e { serde_json::from_str(text).ok() } else { None }).expect("kept");
        assert_eq!((doc["steps"][1]["switches"].clone(), doc["steps"][1]["sawBoth"].clone()), (json!(2), json!(true)));
    }

    #[test]
    fn can_t_tell_and_skip_are_answers_and_up_and_down_move_between_the_answers() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        lab.update(&key("down"), now);
        assert_eq!(lab.view().and_then(|v| if let Body::Step(s) = v.body { Some(s.focus) } else { None }), Some(1));
        lab.update(&key("down"), now);
        lab.update(&key("down"), now);
        assert!(matches!(lab.view().map(|v| v.body), Some(Body::Step(s)) if s.focus == 2), "no wrap past the last");
        lab.update(&key("up"), now);
        lab.update(&key("up"), now);
        lab.update(&key("up"), now);
        assert!(matches!(lab.view().map(|v| v.body), Some(Body::Step(s)) if s.focus == 0), "nor past the first");
        lab.update(&key("down"), now);
        lab.update(&key("ok"), now);
        assert_eq!(lab.answers()[0], Some(Answer::Same));
        lab.update(&key("down"), now);
        lab.update(&key("down"), now);
        lab.update(&key("ok"), now);
        assert_eq!(lab.answers()[1], Some(Answer::Skipped));
        assert_eq!(lab.step().unwrap().id, "sharpness-medium-strong");
        // a click on an answer is that answer
        lab.click(Target::Answer(1), now);
        assert_eq!(lab.answers()[2], Some(Answer::Same));
    }

    #[test]
    fn a_question_step_takes_yes_no_can_t_tell_and_skip_and_moving_on_puts_the_old_control_back() {
        let (mut lab, mut now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        // through the five sharpness steps, leaving the last with B on the screen
        for _ in 0..4 {
            lab.update(&key("down"), now);
            let fx = lab.update(&key("ok"), now);
            settle(&mut lab, &fx, now);
        }
        let fx = lab.update(&key("right"), now);
        settle(&mut lab, &fx, now);
        assert_eq!(lab.step().unwrap().id, "sharpness-off");
        now += 500;
        lab.update(&key("down"), now);
        let fx = lab.update(&key("ok"), now);
        let mut r = requests(&fx);
        r.extend(requests(&settle(&mut lab, &fx, now)));
        assert_eq!(
            r.iter().map(set_of).collect::<Vec<_>>(),
            [("sharpness".to_owned(), "normal".to_owned()), ("window".to_owned(), "off".to_owned())],
            "sharpness is put back, then the window's A"
        );
        let Some(View { body: Body::Step(s), .. }) = lab.view() else { panic!() };
        assert_eq!((s.id, s.question, s.counter.as_str()), ("window-a", "Does the window show the picture cleanly?", "1 of 2"));
        assert_eq!(s.replies, vec![Reply::Yes, Reply::No, Reply::Same, Reply::Skip]);
        // No on the first setting asks for the second; Yes on that one ends the run
        lab.update(&key("down"), now);
        lab.update(&key("ok"), now);
        assert_eq!(lab.answers()[5], Some(Answer::No));
        assert_eq!(lab.step().map(|s| s.id), Some("window-b"));
        lab.update(&key("ok"), now);
        assert_eq!(lab.answers()[6], Some(Answer::Yes));
        assert_eq!(lab.stage(), Stage::Results);
    }

    #[test]
    fn the_second_window_setting_is_left_out_when_the_first_worked_and_asked_when_it_did_not() {
        for (first, second_asked) in [(Reply::Yes, false), (Reply::No, true), (Reply::Same, true), (Reply::Skip, true)] {
            let (mut lab, now) = begun();
            lab.update(&fetched(TAG_SET, 200, session(true)), now);
            let fx = lab.update(&key("ok"), now);
            settle(&mut lab, &fx, now);
            // straight to the window step
            lab.at = step_index("window-a").unwrap();
            let i = replies(STEPS[lab.at].ask).iter().position(|r| *r == first).unwrap();
            let before = lab.at;
            lab.click(Target::Answer(i), now);
            assert_eq!(lab.step().map(|s| s.id), if second_asked { Some("window-b") } else { None }, "{first:?}");
            if second_asked {
                assert_eq!(lab.at, before + 1);
            } else {
                assert_eq!(lab.stage(), Stage::Results, "the last step that is asked ends the run");
            }
        }
    }

    #[test]
    fn the_last_answer_ends_the_run_puts_everything_back_and_keeps_and_sends_the_answers() {
        let (mut lab, mut now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        let mut last = Vec::new();
        for _ in 0..STEPS.len() {
            now += 400;
            if lab.stage() != Stage::Step {
                break;
            }
            // Skip is the last answer of every step
            let skip = lab.replies().len() - 1;
            last = lab.click(Target::Answer(skip), now);
            settle(&mut lab, &last, now);
        }
        assert_eq!(lab.stage(), Stage::Results);
        let paths: Vec<String> = requests(&last).iter().map(|r| r.1.clone()).collect();
        assert!(paths.contains(&"/v1/displaylab/end".to_owned()) && paths.contains(&"/v1/displaylab/results".to_owned()), "{paths:?}");
        let sent = requests(&last).into_iter().find(|r| r.0 == TAG_RESULTS).unwrap();
        assert_eq!(sent.2["steps"].as_array().unwrap().len(), STEPS.len(), "a skipped first window setting does not make the second one needless");
        assert_eq!(*lab.link(), Link::Ended);
        let Some(View { body: Body::Results(r), .. }) = lab.view() else { panic!("results") };
        assert_eq!(r.rows.len(), STEPS.len());
        assert!(r.rows.iter().all(|row| row.1 == "Skipped"));
        assert_eq!(r.saved, "Saved on the TV and sent", "the helper answered the run");
        // the session is over: nothing is pinged any more
        assert!(lab.update(&Input::Tick, now + 10_000).iter().all(|e| !matches!(e, Effect::Request { path, .. } if path.ends_with("/ping"))));
    }

    #[test]
    fn every_way_out_asks_the_helper_to_put_everything_back() {
        type Way = Box<dyn Fn(&mut Lab, u64) -> Vec<Effect>>;
        let ways: Vec<(&str, Way)> = vec![
            ("Back", Box::new(|l, n| l.update(&key("back"), n))),
            ("Home", Box::new(|l, n| l.update(&Input::Topic { topic: "homekey".into(), data: json!({"n": 1, "switcher": false}) }, n))),
            ("screen off", Box::new(|l, n| l.update(&Input::Topic { topic: "screen".into(), data: json!({"on": false}) }, n))),
            ("another app", Box::new(|l, n| l.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "netflix", "home": false}) }, n))),
            ("closed by the shell", Box::new(|l, n| l.close("shell", n))),
        ];
        for (name, way) in ways {
            let (mut lab, now) = begun();
            let fx = lab.update(&key("ok"), now);
            settle(&mut lab, &fx, now);
            let fx = lab.update(&key("right"), now + 10);
            settle(&mut lab, &fx, now + 10);
            let out = way(&mut lab, now + 100);
            assert!(!lab.is_open() && !lab.wants(), "{name}");
            let paths: Vec<String> = requests(&out).iter().map(|r| r.1.clone()).collect();
            assert!(paths.contains(&"/v1/displaylab/end".to_owned()), "{name}: {paths:?}");
            assert_eq!(*lab.link(), Link::Ended, "{name}");
            // and a lab that is closed is a lab that asks for nothing more
            assert!(lab.update(&Input::Tick, now + 10_000).iter().all(|e| !matches!(e, Effect::Request { path, .. } if path.ends_with("/ping"))), "{name}");
        }
    }

    #[test]
    fn leaving_with_answers_keeps_them_and_the_end_goes_after_a_request_that_is_out() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        let fx = lab.update(&key("right"), now + 10);
        assert_eq!(requests(&fx).len(), 1, "a set is out");
        lab.update(&key("down"), now + 20);
        lab.update(&key("ok"), now + 20);
        assert!(lab.answers()[0].is_some());
        let out = lab.update(&key("back"), now + 30);
        assert!(out.iter().any(|e| matches!(e, Effect::Write { path, .. } if path == RESULTS_FILE)), "what was answered is kept");
        assert!(requests(&out).is_empty(), "the end waits for the set that is out");
        let after = lab.update(&fetched(TAG_SET, 200, session(true)), now + 40);
        assert_eq!(requests(&after).iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["/v1/displaylab/end"]);
    }

    #[test]
    fn the_lab_tells_the_helper_it_is_there_every_second_and_not_while_a_request_is_out() {
        let (mut lab, now) = begun();
        assert!(lab.due_at().is_some());
        assert!(requests(&lab.update(&Input::Tick, now + 500)).is_empty(), "not yet");
        let fx = lab.update(&Input::Tick, now + PING_MS);
        assert_eq!(requests(&fx).iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["/v1/displaylab/ping"]);
        // while it is unanswered no second one goes
        assert!(requests(&lab.update(&Input::Tick, now + PING_MS + 5)).is_empty());
        lab.update(&fetched(TAG_PING, 200, session(true)), now + PING_MS + 10);
        let fx = lab.update(&Input::Tick, now + 2 * PING_MS + 5);
        assert_eq!(requests(&fx).len(), 1, "the next second, the next ping");
        // a switch is a word too: it moves the next ping a second on
        lab.update(&fetched(TAG_PING, 200, session(true)), now + 2 * PING_MS + 10);
        lab.update(&key("ok"), now + 2 * PING_MS + 20);
        let before = lab.next_ping;
        lab.update(&fetched(TAG_SET, 200, session(true)), now + 2 * PING_MS + 40);
        assert!(lab.next_ping > before);
    }

    #[test]
    fn a_helper_that_says_no_is_told_to_the_person_in_its_own_words_and_only_leaving_is_left() {
        let mut lab = Lab::new();
        lab.open(1000);
        let why = json!({"ok": false, "error": {"code": "unavailable", "message": "not an LG set: /proc/lg/fbdev/chip/osd_sr_gain is missing"}});
        lab.update(&fetched(TAG_BEGIN, 503, why), 1020);
        let Some(View { body: Body::Intro(i), .. }) = lab.view() else { panic!() };
        assert!(!i.ready);
        assert_eq!(i.note.as_deref(), Some("not an LG set: /proc/lg/fbdev/chip/osd_sr_gain is missing"));
        assert!(matches!(lab.link(), Link::Refused(_)));
        assert!(requests(&lab.update(&key("ok"), 1030)).iter().all(|r| r.1.ends_with("/end")) || !lab.is_open(), "OK on a lab that cannot run leaves it");
        assert!(!lab.is_open());
        // a helper that is not there at all
        let mut lab = Lab::new();
        lab.open(1000);
        lab.update(&fetched(TAG_BEGIN, 0, Value::Null), 1020);
        let Some(View { body: Body::Intro(i), .. }) = lab.view() else { panic!() };
        assert_eq!(i.note.as_deref(), Some("The TV's helper did not answer."));
        // Back leaves, and nothing is asked: there is no session to end
        let out = lab.update(&key("back"), 1030);
        assert!(requests(&out).is_empty() && !lab.is_open());
    }

    #[test]
    fn a_helper_that_never_answers_is_given_five_seconds_and_the_lab_says_so() {
        let mut lab = Lab::new();
        lab.open(1000);
        lab.update(&Input::Tick, 5000);
        assert!(matches!(lab.link(), Link::Beginning), "not yet");
        lab.update(&Input::Tick, 6100);
        assert!(matches!(lab.link(), Link::Refused(w) if w.contains("did not answer")));
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        assert_eq!(requests(&fx).len(), 1);
        lab.update(&Input::Tick, now + 5100);
        assert!(matches!(lab.link(), Link::Lost(_)));
        let Some(View { body: Body::Step(s), .. }) = lab.view() else { panic!() };
        assert_eq!(s.note.as_deref(), Some("The TV's helper did not answer."));
    }

    #[test]
    fn a_session_the_helper_has_put_back_is_started_again_for_the_next_switch() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        // the helper says the session is gone (it did not hear from the lab)
        lab.update(&Input::Tick, now + PING_MS);
        let gone = json!({"ok": false, "error": {"code": "no_session", "message": "no lab session is open"}});
        lab.update(&fetched(TAG_PING, 409, gone), now + PING_MS + 10);
        assert!(matches!(lab.link(), Link::Lost(_)));
        let Some(View { body: Body::Step(s), .. }) = lab.view() else { panic!() };
        assert_eq!(s.note.as_deref(), Some("The TV put everything back: it had not heard from the lab."));
        // the next switch begins a session first, then writes
        let fx = lab.update(&key("right"), now + 2000);
        assert_eq!(requests(&fx).iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["/v1/displaylab/begin"]);
        let fx = lab.update(&fetched(TAG_BEGIN, 200, session(true)), now + 2020);
        assert_eq!(requests(&fx).iter().map(set_of).collect::<Vec<_>>(), [("sharpness".to_owned(), "strong".to_owned())]);
        lab.update(&fetched(TAG_SET, 200, session(true)), now + 2040);
        assert!(matches!(lab.link(), Link::Ready));
        assert_eq!(lab.view().and_then(|v| if let Body::Step(s) = v.body { s.note } else { None }), None, "the note went with the trouble");
    }

    #[test]
    fn a_write_the_tv_refused_is_said_and_no_comparison_is_offered_only_the_way_on() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        lab.update(&key("right"), now + 100);
        let bad = json!({"ok": false, "error": {"code": "verify_failed", "message": "osd0_vsp reads 0x0 after 0x1000001 was written"}});
        lab.update(&fetched(TAG_SET, 502, bad), now + 120);
        let Some(View { body: Body::Step(s), .. }) = lab.view() else { panic!() };
        assert_eq!(s.note.as_deref(), Some("osd0_vsp reads 0x0 after 0x1000001 was written"));
        // B never showed: there is nothing to pick between, and the one answer is that
        assert_eq!((s.replies, s.focus), (vec![Reply::Failed], 0));
        lab.update(&key("down"), now + 200);
        lab.update(&key("ok"), now + 200);
        assert_eq!(lab.answers()[0], Some(Answer::Failed), "the lab's own finding, not the person's opinion");
        assert_eq!(lab.step().unwrap().id, "sharpness-medium", "and it goes on");
    }

    /// A helper's answer for the sharpness control: the session as a whole and that control either hold or not.
    fn session_sharpness(holds: bool) -> Value {
        json!({"ok": true, "displaylab": {"available": true, "session": {"id": 1, "held": holds, "heldBy": {"sharpness": holds}, "drifted": if holds { json!([]) } else { json!(["osd_sr_gain"]) },
            "usable": {"sharpness": true, "window": true}, "why": {}}}})
    }

    fn step_view(lab: &Lab) -> StepView {
        let Some(View { body: Body::Step(s), .. }) = lab.view() else { panic!("a step") };
        s
    }

    #[test]
    fn a_look_the_firmware_takes_back_at_once_is_said_on_the_screen_and_no_comparison_is_offered() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        // B is asked for; the helper wrote it and read it back, and by its answer the firmware had put the old value back: two identical pictures
        lab.update(&key("right"), now + 100);
        lab.update(&fetched(TAG_SET, 200, session_sharpness(false)), now + 120);
        let s = step_view(&lab);
        assert_eq!(s.note.as_deref(), Some("Couldn't apply Strong. The TV puts it back."));
        assert_eq!((s.replies.clone(), s.focus), (vec![Reply::Failed], 0), "nothing to pick between: only the way on");
        // down and OK stay on the one answer, which is not an opinion
        lab.update(&key("down"), now + 200);
        let fx = lab.update(&key("ok"), now + 210);
        assert_eq!(lab.answers()[0], Some(Answer::Failed));
        let doc: Value = fx.iter().find_map(|e| if let Effect::Write { text, .. } = e { serde_json::from_str(text).ok() } else { None }).expect("kept");
        assert_eq!(
            (doc["steps"][0]["answer"].clone(), doc["steps"][0]["verdict"].clone(), doc["steps"][0]["bApplied"].clone(), doc["steps"][0]["held"].clone()),
            (json!("failed"), json!("Couldn't apply"), json!(false), json!(false))
        );
        assert_eq!(lab.step().unwrap().id, "sharpness-medium", "and it goes on");
    }

    #[test]
    fn a_look_that_could_not_be_applied_can_be_tried_again_and_is_compared_when_it_shows() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        lab.update(&key("right"), now + 100);
        lab.update(&fetched(TAG_SET, 200, session_sharpness(false)), now + 120);
        assert_eq!(step_view(&lab).replies, vec![Reply::Failed]);
        // back to A, which holds, and to B again, which now does
        let fx = lab.update(&key("left"), now + 200);
        settle(&mut lab, &fx, now + 200);
        let fx = lab.update(&key("right"), now + 300);
        assert_eq!(set_of(&requests(&fx)[0]).1, "strong");
        lab.update(&fetched(TAG_SET, 200, session_sharpness(true)), now + 320);
        let s = step_view(&lab);
        assert_eq!((s.replies, s.note), (vec![Reply::Pick, Reply::Same, Reply::Skip], None));
    }

    #[test]
    fn a_register_taken_back_for_another_control_does_not_fail_the_look_on_the_screen() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        lab.update(&key("right"), now + 100);
        // the session as a whole does not hold (the window's registers did not), the sharpness does
        let doc = json!({"ok": true, "displaylab": {"session": {"held": false, "heldBy": {"sharpness": true, "window": false}, "drifted": ["hole_sel"], "usable": {"sharpness": true, "window": true}, "why": {}}}});
        lab.update(&fetched(TAG_SET, 200, doc), now + 120);
        let s = step_view(&lab);
        assert_eq!((s.replies, s.note), (vec![Reply::Pick, Reply::Same, Reply::Skip], None));
    }

    #[test]
    fn a_look_that_showed_and_is_taken_back_later_keeps_its_answers_and_says_so() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        lab.update(&key("right"), now + 100);
        lab.update(&fetched(TAG_SET, 200, session_sharpness(true)), now + 120);
        lab.update(&Input::Tick, now + 120 + PING_MS);
        lab.update(&fetched(TAG_PING, 200, session_sharpness(false)), now + 130 + PING_MS);
        let s = step_view(&lab);
        assert_eq!(s.note.as_deref(), Some("The TV put Strong back by itself."));
        assert_eq!(s.replies, vec![Reply::Pick, Reply::Same, Reply::Skip], "it was seen: the person's answer stands");
    }

    #[test]
    fn what_the_helper_says_has_not_held_is_told_and_goes_when_it_holds_again() {
        let (mut lab, now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        lab.update(&Input::Tick, now + PING_MS);
        lab.update(&fetched(TAG_PING, 200, session(false)), now + PING_MS + 5);
        let note = |l: &Lab| l.view().and_then(|v| if let Body::Step(s) = v.body { s.note } else { None });
        assert_eq!(note(&lab).as_deref(), Some("The TV put Normal back by itself."));
        // the answer records that it did not hold
        lab.update(&key("down"), now + PING_MS + 10);
        lab.update(&key("ok"), now + PING_MS + 10);
        assert_eq!(lab.held_at[0], Some(false));
        lab.update(&Input::Tick, now + 2 * PING_MS + 10);
        lab.update(&fetched(TAG_PING, 200, session(true)), now + 2 * PING_MS + 20);
        assert_eq!(note(&lab), None);
    }

    #[test]
    fn a_control_the_helper_cannot_use_is_said_on_its_steps() {
        let mut lab = Lab::new();
        lab.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.hdmi1"}) }, 0);
        lab.open(1000);
        let doc = json!({"ok": true, "displaylab": {"session": {"held": true, "usable": {"sharpness": true, "window": false}, "why": {"window": "hole_sel reads 0x3, which is not the set's own state"}}}});
        lab.update(&fetched(TAG_BEGIN, 200, doc), 1010);
        lab.at = step_index("window-a").unwrap();
        lab.stage = Stage::Step;
        assert_eq!(lab.note_for(&STEPS[lab.at]).as_deref(), Some("The TV cannot do this one: hole_sel reads 0x3, which is not the set's own state"));
        // a control that can be used has nothing to say
        assert_eq!(lab.note_for(&STEPS[0]), None);
    }

    #[test]
    fn the_window_says_so_when_only_home_is_behind_it() {
        let (lab, _) = begun();
        assert_eq!(lab.note_for(&STEPS[step_index("window-a").unwrap()]).as_deref(), Some("Home is behind this. A playing video shows the window best."));
        assert_eq!(lab.note_for(&STEPS[0]), None);
    }

    #[test]
    fn again_starts_a_new_run_and_done_leaves() {
        let (mut lab, mut now) = begun();
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        while lab.stage() == Stage::Step {
            now += 100;
            let skip = lab.replies().len() - 1;
            let fx = lab.click(Target::Answer(skip), now);
            settle(&mut lab, &fx, now);
        }
        assert_eq!(lab.stage(), Stage::Results);
        lab.update(&key("down"), now);
        let fx = lab.update(&key("ok"), now + 10);
        assert_eq!(requests(&fx).iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["/v1/displaylab/begin"], "Again begins a session again");
        assert_eq!((lab.stage(), lab.answers().iter().flatten().count()), (Stage::Intro, 0));
        assert!(lab.is_open());
        lab.update(&fetched(TAG_BEGIN, 200, session(true)), now + 30);
        let fx = lab.update(&key("ok"), now + 40);
        assert_eq!(lab.stage(), Stage::Step);
        settle(&mut lab, &fx, now + 40);
        // and Done on a results screen leaves (a second run, to its end)
        while lab.stage() == Stage::Step {
            let skip = lab.replies().len() - 1;
            let fx = lab.click(Target::Answer(skip), now + 50);
            settle(&mut lab, &fx, now + 50);
        }
        lab.update(&key("ok"), now + 60);
        assert!(!lab.is_open());
    }

    #[test]
    fn the_state_says_what_a_gate_wants_to_know() {
        let (mut lab, now) = begun();
        let s = lab.state();
        assert_eq!((s["open"].clone(), s["stage"].clone(), s["link"].clone()), (json!(true), json!("intro"), json!("ready")));
        let fx = lab.update(&key("ok"), now);
        settle(&mut lab, &fx, now);
        let s = lab.state();
        assert_eq!((s["stage"].as_str(), s["step"].as_str(), s["shown"].as_u64()), (Some("step"), Some("sharpness-strong"), Some(0)));
    }

    #[test]
    fn the_foreground_that_does_not_change_is_not_a_reason_to_leave() {
        let (mut lab, now) = begun();
        lab.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": HOME_APP, "home": true}) }, now + 10);
        assert!(lab.is_open());
        lab.update(&Input::Topic { topic: "screen".into(), data: json!({"on": true}) }, now + 20);
        assert!(lab.is_open());
        lab.update(&Input::Topic { topic: "audio".into(), data: json!({"volume": 10}) }, now + 30);
        assert!(lab.is_open());
    }

    #[test]
    fn a_lab_closed_fades_out_and_is_gone() {
        let (mut lab, now) = begun();
        lab.update(&key("back"), now);
        assert_eq!(lab.phase(), Phase::Leaving);
        assert!(lab.view().is_some(), "drawn while it fades");
        lab.update(&Input::Tick, now + 200);
        lab.update(&Input::Tick, now + 400);
        assert!(lab.view().is_none(), "nothing is kept");
    }
}
