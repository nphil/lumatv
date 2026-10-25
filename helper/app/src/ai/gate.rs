//! The gate: the one place in the helper where "a person holding the physical remote agreed" is decided.
//!
//! A confirmation that an assistant (or anything reachable over a network or an API) can see, forge or answer is not a control:
//! a hijacked assistant fakes or skips it. So the helper does not ask anyone *whether* a person pressed OK; it watches the TV's
//! input devices itself while a prompt is open, and only a key from a *physical remote's* device counts. The remotes reach the
//! compositor as virtual devices made by LG's input daemon: the IR remote (`LGE RCU`) and the Magic Remote (`LGE M-RCU - ...`)
//! are physical; keys that come from the network (Home Assistant's `webostv.button`, SSAP, this module's own media keys) arrive
//! on `Smart Remote RCU Input` and `LGE Network Input`. A key from any other device, known or not, approves nothing.
//!
//! What the gate guarantees, in code:
//! - [`GateApproval`] is the proof that a physical key approved a prompt. It has a private field, no constructor outside this
//!   module and no `Clone`, so a function that takes `&GateApproval` can only be called after [`Pending::wait`] returned
//!   [`Decision::Approved`]. Every change that gives an assistant more (turning the listener on, opening pairing, widening a
//!   level, resuming a paused assistant, raising the volume limit) takes one; narrowing never does.
//! - A prompt is answered by the *release* of a key whose *press* came after the prompt had been on screen for
//!   [`GateConfig::min_display`]: a key that was already held, an auto-repeat, or a stray press as the prompt appeared cannot
//!   answer it.
//! - One prompt at a time. A second request while one is open is refused at once (`Busy`), so nothing can flood the screen.
//! - A prompt nobody can answer (no device could be opened, the screen is off, the owner is away) expires and means *no*.
//!
//! The prompt itself is published for the TV's UI as the hub topic `ai.prompt` (`null` when none): what it says is built by the
//! helper from validated data, never taken from an assistant. The UI shows it and swallows the remote's keys while it is up; it
//! has no way to answer it, and there is no route that does.

use super::types::{AssistantView, KeySource};
use crate::hub::Hub;
use crate::timeutil::now_ms;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The hub topic of the open prompt.
pub const PROMPT_TOPIC: &str = "ai.prompt";

/// How the gate reads the remote.
#[derive(Clone, Debug)]
pub struct GateConfig {
    /// How long a prompt waits for a key before it expires (30 s in the plan).
    pub prompt_ttl: Duration,
    /// How long a prompt is up before a key press can answer it.
    pub min_display: Duration,
    /// Evdev codes that approve (OK is 28; 352 is KEY_OK).
    pub approve_codes: Vec<u16>,
    /// Evdev codes that deny (Back arrives as 412; 158 is KEY_BACK, 174 KEY_EXIT).
    pub deny_codes: Vec<u16>,
    /// Device names that are physical remotes. An entry ending in `*` is a prefix.
    pub physical: Vec<String>,
    /// Device names known to carry network keys; only for the log, since a device that is not physical is ignored either way.
    pub network: Vec<String>,
    /// The longest one wait for a key lasts, so a cancel is seen within this.
    pub slice: Duration,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            prompt_ttl: Duration::from_secs(30),
            min_display: Duration::from_millis(300),
            approve_codes: vec![28, 352],
            deny_codes: vec![412, 158, 174],
            physical: vec!["LGE RCU".to_owned(), "LGE M-RCU*".to_owned()],
            network: vec![
                "Smart Remote RCU Input".to_owned(),
                "LGE Network Input".to_owned(),
            ],
            slice: Duration::from_millis(200),
        }
    }
}

fn pattern_matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pattern == name,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeviceClass {
    Physical,
    Network,
    Unknown,
}

impl GateConfig {
    fn classify(&self, device: &str) -> DeviceClass {
        // Physical is checked first, and a name must match a physical pattern to count: nothing else is ever trusted.
        if self.physical.iter().any(|p| pattern_matches(p, device)) {
            DeviceClass::Physical
        } else if self.network.iter().any(|p| pattern_matches(p, device)) {
            DeviceClass::Network
        } else {
            DeviceClass::Unknown
        }
    }

    fn is_decision_key(&self, code: u16) -> bool {
        self.approve_codes.contains(&code) || self.deny_codes.contains(&code)
    }
}

/// The proof that a physical remote approved a prompt (see the module docs). It cannot be built, copied or cloned outside this
/// module, and it is consumed by reference: holding one means [`Pending::wait`] returned [`Decision::Approved`].
#[derive(Debug)]
pub struct GateApproval {
    _proof: (),
}

/// How a prompt ended.
#[derive(Debug)]
pub enum Decision {
    /// A physical remote's OK. Carries the proof.
    Approved(GateApproval),
    /// A physical remote's Back.
    Denied,
    /// Nobody answered in time.
    Expired,
    /// The prompt was withdrawn (a lock-out, or the helper stopping).
    Cancelled,
}

/// A decision and what the gate saw besides it.
#[derive(Debug)]
pub struct Verdict {
    pub decision: Decision,
    /// How many presses of OK or Back came from devices that carry network keys, and were ignored.
    pub ignored_network_keys: u32,
    /// The same for devices that are neither physical nor known network devices.
    pub ignored_unknown_keys: u32,
}

/// What kind of change a prompt asks about (for the UI's icon and for the log).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// An assistant's tool call at the `ask` level.
    Call,
    EnableListener,
    StartPairing,
    WidenLevels,
    Resume,
    RaiseLimit,
}

impl PromptKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::EnableListener => "enable_listener",
            Self::StartPairing => "start_pairing",
            Self::WidenLevels => "widen_levels",
            Self::Resume => "resume",
            Self::RaiseLimit => "raise_limit",
        }
    }
}

/// What a prompt says. `title` and `detail` are built by the helper; `reason` is the assistant's own words, shown labelled as such.
#[derive(Clone, Debug)]
pub struct PromptSpec {
    pub kind: PromptKind,
    pub assistant: Option<AssistantView>,
    pub title: String,
    pub detail: String,
    pub reason: Option<String>,
}

/// Another prompt is open.
#[derive(Debug, PartialEq, Eq)]
pub struct Busy;

struct Current {
    id: String,
    cancelled: bool,
}

pub struct Gate {
    keys: Arc<dyn KeySource>,
    hub: Arc<Hub>,
    config: GateConfig,
    current: Mutex<Option<Current>>,
}

impl Gate {
    pub fn new(keys: Arc<dyn KeySource>, hub: Arc<Hub>, config: GateConfig) -> Arc<Self> {
        Arc::new(Self {
            keys,
            hub,
            config,
            current: Mutex::new(None),
        })
    }

    pub fn config(&self) -> &GateConfig {
        &self.config
    }

    /// Whether a prompt is open.
    pub fn is_busy(&self) -> bool {
        self.current.lock().is_some()
    }

    /// Withdraws the open prompt, if any; its waiter returns [`Decision::Cancelled`] within one slice.
    pub fn cancel(&self) -> bool {
        match self.current.lock().as_mut() {
            Some(current) => {
                current.cancelled = true;
                true
            }
            None => false,
        }
    }

    fn is_cancelled(&self, id: &str) -> bool {
        self.current
            .lock()
            .as_ref()
            .is_some_and(|c| c.id == id && c.cancelled)
    }

    /// Opens a prompt: the remote's devices are opened first, then the prompt is published, so a key pressed as it appears is
    /// seen. The caller runs [`Pending::wait`] on whatever thread should block (an assistant's request, or a thread made for a
    /// change the TV's own UI asked for). Refused with [`Busy`] while another prompt is open.
    pub fn begin(self: &Arc<Self>, spec: PromptSpec) -> Result<Pending, Busy> {
        let id = {
            let mut current = self.current.lock();
            if current.is_some() {
                return Err(Busy);
            }
            let id = prompt_id();
            *current = Some(Current {
                id: id.clone(),
                cancelled: false,
            });
            id
        };
        let session = self.keys.open();
        let shown_at = Instant::now();
        let expires_at = shown_at + self.config.prompt_ttl;
        let shown_ms = now_ms();
        let doc = json!({
            "id": id,
            "kind": spec.kind.id(),
            "assistant": spec.assistant.as_ref().map(|a| json!({"id": a.id, "name": a.name})),
            "title": spec.title,
            "detail": spec.detail,
            "reason": spec.reason,
            "shownAt": shown_ms,
            "expiresAt": shown_ms + u64::try_from(self.config.prompt_ttl.as_millis()).unwrap_or(u64::MAX),
            "allow": "OK",
            "deny": "Back",
        });
        self.hub.set(PROMPT_TOPIC, doc);
        crate::log_info!("ai: prompt {} ({})", id, spec.kind.id());
        Ok(Pending {
            gate: self.clone(),
            id,
            session: Some(session),
            shown_at,
            expires_at,
        })
    }
}

fn prompt_id() -> String {
    let mut bytes = [0u8; 6];
    if crate::sys::random_bytes(&mut bytes).is_err() {
        bytes = (now_ms() & 0xffff_ffff_ffff).to_le_bytes()[..6]
            .try_into()
            .unwrap_or([0; 6]);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// An open prompt. Dropping it (answered or not) closes the prompt and clears the topic.
pub struct Pending {
    gate: Arc<Gate>,
    id: String,
    session: Option<Box<dyn super::types::KeySession>>,
    shown_at: Instant,
    expires_at: Instant,
}

impl Pending {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Blocks until a physical key answers the prompt, it expires, or it is cancelled.
    pub fn wait(mut self) -> Verdict {
        let config = self.gate.config.clone();
        let armed_at = self.shown_at + config.min_display;
        let Some(mut session) = self.session.take() else {
            return Verdict {
                decision: Decision::Cancelled,
                ignored_network_keys: 0,
                ignored_unknown_keys: 0,
            };
        };
        // Presses seen after the prompt was armed, by (device, key): only the release of one of these answers.
        let mut pressed: HashSet<(String, u16)> = HashSet::new();
        let (mut ignored_network, mut ignored_unknown) = (0u32, 0u32);
        let decision = loop {
            if self.gate.is_cancelled(&self.id) {
                break Decision::Cancelled;
            }
            let now = Instant::now();
            if now >= self.expires_at {
                break Decision::Expired;
            }
            let slice = config.slice.min(self.expires_at - now);
            let Some(event) = session.next(slice) else {
                continue;
            };
            if !config.is_decision_key(event.code) {
                continue;
            }
            match config.classify(&event.device) {
                DeviceClass::Physical => match event.value {
                    1 if event.at >= armed_at => {
                        pressed.insert((event.device.clone(), event.code));
                    }
                    0 if pressed.remove(&(event.device.clone(), event.code)) => {
                        if config.approve_codes.contains(&event.code) {
                            break Decision::Approved(GateApproval { _proof: () });
                        }
                        break Decision::Denied;
                    }
                    _ => {}
                },
                DeviceClass::Network => {
                    if event.value == 1 {
                        ignored_network += 1;
                    }
                }
                DeviceClass::Unknown => {
                    if event.value == 1 {
                        ignored_unknown += 1;
                    }
                }
            }
        };
        if ignored_network > 0 || ignored_unknown > 0 {
            crate::log_warn!(
                "ai: prompt {}: ignored {ignored_network} approval key(s) from network devices and {ignored_unknown} from unknown devices",
                self.id
            );
        }
        let word = match &decision {
            Decision::Approved(_) => "approved",
            Decision::Denied => "denied",
            Decision::Expired => "expired",
            Decision::Cancelled => "cancelled",
        };
        crate::log_info!("ai: prompt {} {word}", self.id);
        Verdict {
            decision,
            ignored_network_keys: ignored_network,
            ignored_unknown_keys: ignored_unknown,
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        let mut current = self.gate.current.lock();
        if current.as_ref().is_some_and(|c| c.id == self.id) {
            *current = None;
            drop(current);
            self.gate.hub.set(PROMPT_TOPIC, Value::Null);
        }
    }
}

#[cfg(test)]
pub(crate) fn approval_for_tests() -> GateApproval {
    GateApproval { _proof: () }
}

/// A scripted key source for tests: `open()` makes a fresh channel and keeps its sender for the test to use.
#[cfg(test)]
pub(crate) mod fake {
    use super::super::types::{KeyEvent, KeySession, KeySource};
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    pub struct FakeKeys {
        sender: Mutex<Option<Sender<KeyEvent>>>,
        pub opens: AtomicUsize,
    }

    struct Session(Receiver<KeyEvent>);

    impl KeySession for Session {
        fn next(&mut self, timeout: Duration) -> Option<KeyEvent> {
            self.0.recv_timeout(timeout).ok()
        }
    }

    impl KeySource for FakeKeys {
        fn open(&self) -> Box<dyn KeySession> {
            let (tx, rx) = channel();
            *self.sender.lock() = Some(tx);
            self.opens.fetch_add(1, Ordering::SeqCst);
            Box::new(Session(rx))
        }
    }

    impl FakeKeys {
        pub fn send(&self, device: &str, code: u16, value: i32) {
            if let Some(tx) = self.sender.lock().as_ref() {
                let _ = tx.send(KeyEvent {
                    device: device.to_owned(),
                    code,
                    value,
                    at: Instant::now(),
                });
            }
        }

        /// A press and a release of `code` on `device`, as a person's tap.
        pub fn tap(&self, device: &str, code: u16) {
            self.send(device, code, 1);
            std::thread::sleep(Duration::from_millis(5));
            self.send(device, code, 0);
        }

        pub fn opens(&self) -> usize {
            self.opens.load(Ordering::SeqCst)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeKeys;
    use super::*;
    use std::thread;

    const IR: &str = "LGE RCU";
    const MAGIC: &str = "LGE M-RCU - Builtin [1]";
    const NETWORK: &str = "Smart Remote RCU Input";
    const OK: u16 = 28;
    const BACK: u16 = 412;

    fn gate(min_display_ms: u64, ttl_ms: u64) -> (Arc<Gate>, Arc<FakeKeys>, Arc<Hub>) {
        let keys = Arc::new(FakeKeys::default());
        let hub = Arc::new(Hub::new(1));
        let config = GateConfig {
            prompt_ttl: Duration::from_millis(ttl_ms),
            min_display: Duration::from_millis(min_display_ms),
            slice: Duration::from_millis(10),
            ..GateConfig::default()
        };
        (Gate::new(keys.clone(), hub.clone(), config), keys, hub)
    }

    fn spec(title: &str) -> PromptSpec {
        PromptSpec {
            kind: PromptKind::Call,
            assistant: Some(AssistantView {
                id: "a1".into(),
                name: "Cody".into(),
            }),
            title: title.into(),
            detail: "detail".into(),
            reason: Some("because".into()),
        }
    }

    /// Runs the prompt on a thread; returns its handle.
    fn run(gate: &Arc<Gate>) -> thread::JoinHandle<Verdict> {
        let pending = gate.begin(spec("Open Netflix")).unwrap();
        thread::spawn(move || pending.wait())
    }

    fn settle(ms: u64) {
        thread::sleep(Duration::from_millis(ms));
    }

    #[test]
    fn a_physical_ok_after_the_prompt_is_armed_approves() {
        let (gate, keys, _) = gate(40, 5_000);
        let waiter = run(&gate);
        settle(80);
        keys.tap(IR, OK);
        let verdict = waiter.join().unwrap();
        assert!(
            matches!(verdict.decision, Decision::Approved(_)),
            "{:?}",
            verdict.decision
        );
        assert_eq!(
            (verdict.ignored_network_keys, verdict.ignored_unknown_keys),
            (0, 0)
        );
    }

    #[test]
    fn the_magic_remote_counts_too_and_back_denies() {
        let (gate, keys, _) = gate(20, 5_000);
        let waiter = run(&gate);
        settle(50);
        keys.tap(MAGIC, OK);
        assert!(matches!(
            waiter.join().unwrap().decision,
            Decision::Approved(_)
        ));
        let waiter = run(&gate);
        settle(50);
        keys.tap(IR, BACK);
        assert!(matches!(waiter.join().unwrap().decision, Decision::Denied));
        // The other Back codes deny as well.
        for code in [158, 174] {
            let waiter = run(&gate);
            settle(50);
            keys.tap(MAGIC, code);
            assert!(
                matches!(waiter.join().unwrap().decision, Decision::Denied),
                "{code}"
            );
        }
    }

    #[test]
    fn a_key_pressed_before_the_prompt_is_armed_cannot_answer_it() {
        let (gate, keys, _) = gate(200, 1_500);
        let waiter = run(&gate);
        // A tap right as the prompt appears: before the minimum display time.
        keys.tap(IR, OK);
        settle(10);
        // And a press that began early with its release after arming.
        keys.send(IR, OK, 1);
        settle(260);
        keys.send(IR, OK, 0);
        let verdict = waiter.join().unwrap();
        assert!(
            matches!(verdict.decision, Decision::Expired),
            "{:?}",
            verdict.decision
        );
    }

    #[test]
    fn only_the_release_of_a_press_seen_while_armed_answers() {
        let (gate, keys, _) = gate(20, 600);
        let waiter = run(&gate);
        settle(50);
        keys.send(IR, OK, 1);
        // Held: auto-repeat events do nothing.
        for _ in 0..5 {
            keys.send(IR, OK, 2);
        }
        let verdict = waiter.join().unwrap();
        assert!(
            matches!(verdict.decision, Decision::Expired),
            "a press without a release approved"
        );
        // A lone release (its press was before the prompt) does nothing either.
        let waiter = run(&gate);
        settle(50);
        keys.send(IR, OK, 0);
        assert!(matches!(waiter.join().unwrap().decision, Decision::Expired));
    }

    #[test]
    fn keys_from_the_network_device_never_answer_and_are_counted() {
        let (gate, keys, _) = gate(20, 500);
        let waiter = run(&gate);
        settle(50);
        keys.tap(NETWORK, OK);
        keys.tap("LGE Network Input", OK);
        keys.tap(NETWORK, BACK);
        let verdict = waiter.join().unwrap();
        assert!(matches!(verdict.decision, Decision::Expired));
        assert_eq!(verdict.ignored_network_keys, 3);
    }

    #[test]
    fn keys_from_unknown_or_look_alike_devices_never_answer() {
        let (gate, keys, _) = gate(20, 500);
        let waiter = run(&gate);
        settle(50);
        for device in [
            "IoT keypad",
            "LGE W-RCU - Builtin [5]",
            "LGE RCU2",
            "LGE",
            "lge rcu",
            " LGE RCU",
            "LGE RCU\u{0}",
            "Smart Remote RCU Input 2",
            "uinput virtual keyboard",
        ] {
            keys.tap(device, OK);
        }
        let verdict = waiter.join().unwrap();
        assert!(
            matches!(verdict.decision, Decision::Expired),
            "{:?}",
            verdict.decision
        );
        assert!(
            verdict.ignored_unknown_keys >= 9,
            "{}",
            verdict.ignored_unknown_keys
        );
    }

    #[test]
    fn keys_that_are_not_decision_keys_are_ignored_on_any_device() {
        let (gate, keys, _) = gate(20, 400);
        let waiter = run(&gate);
        settle(50);
        for code in [103, 108, 105, 106, 773, 139, 1, 207] {
            keys.tap(IR, code);
        }
        assert!(matches!(waiter.join().unwrap().decision, Decision::Expired));
    }

    #[test]
    fn device_patterns_match_exactly_or_by_prefix() {
        let c = GateConfig::default();
        assert_eq!(c.classify("LGE RCU"), DeviceClass::Physical);
        assert_eq!(c.classify("LGE M-RCU - Builtin [0]"), DeviceClass::Physical);
        assert_eq!(c.classify("LGE M-RCU"), DeviceClass::Physical);
        assert_eq!(c.classify("LGE RCU2"), DeviceClass::Unknown);
        assert_eq!(c.classify("Smart Remote RCU Input"), DeviceClass::Network);
        assert_eq!(c.classify("LGE Network Input"), DeviceClass::Network);
        assert_eq!(c.classify("IoT keypad"), DeviceClass::Unknown);
        assert_eq!(c.classify(""), DeviceClass::Unknown);
        // A name on both lists is physical only if a physical pattern names it: a network name cannot be promoted by accident.
        let mut both = GateConfig::default();
        both.network.push("LGE RCU".to_owned());
        assert_eq!(both.classify("LGE RCU"), DeviceClass::Physical);
    }

    #[test]
    fn a_prompt_nobody_answers_expires() {
        let (gate, _keys, _) = gate(10, 120);
        let started = Instant::now();
        let verdict = run(&gate).join().unwrap();
        assert!(matches!(verdict.decision, Decision::Expired));
        assert!(
            started.elapsed() >= Duration::from_millis(100)
                && started.elapsed() < Duration::from_secs(3)
        );
    }

    #[test]
    fn only_one_prompt_is_open_at_a_time_and_the_next_can_open_after() {
        let (gate, keys, _) = gate(10, 5_000);
        let first = gate.begin(spec("one")).unwrap();
        assert!(gate.is_busy());
        assert_eq!(gate.begin(spec("two")).err(), Some(Busy));
        let waiter = thread::spawn(move || first.wait());
        settle(40);
        keys.tap(IR, OK);
        assert!(matches!(
            waiter.join().unwrap().decision,
            Decision::Approved(_)
        ));
        assert!(!gate.is_busy());
        assert!(gate.begin(spec("three")).is_ok());
    }

    #[test]
    fn the_prompt_is_on_the_hub_while_open_and_gone_after() {
        let (gate, keys, hub) = gate(10, 5_000);
        assert_eq!(hub.get(PROMPT_TOPIC), None);
        let pending = gate.begin(spec("Open Netflix")).unwrap();
        let shown = hub.get(PROMPT_TOPIC).unwrap();
        assert_eq!(shown["title"], "Open Netflix");
        assert_eq!(shown["kind"], "call");
        assert_eq!(shown["assistant"], json!({"id": "a1", "name": "Cody"}));
        assert_eq!(shown["reason"], "because");
        assert_eq!(shown["id"], pending.id());
        assert_eq!(
            (shown["allow"].clone(), shown["deny"].clone()),
            (json!("OK"), json!("Back"))
        );
        assert!(shown["expiresAt"].as_u64().unwrap() > shown["shownAt"].as_u64().unwrap());
        let waiter = thread::spawn(move || pending.wait());
        settle(40);
        keys.tap(IR, BACK);
        waiter.join().unwrap();
        assert_eq!(hub.get(PROMPT_TOPIC), Some(Value::Null));
    }

    #[test]
    fn dropping_a_prompt_unanswered_frees_the_gate() {
        let (gate, _keys, hub) = gate(10, 5_000);
        drop(gate.begin(spec("x")).unwrap());
        assert!(!gate.is_busy());
        assert_eq!(hub.get(PROMPT_TOPIC), Some(Value::Null));
    }

    #[test]
    fn a_cancel_ends_the_wait_quickly() {
        let (gate, _keys, _) = gate(10, 30_000);
        let waiter = run(&gate);
        settle(30);
        assert!(gate.cancel());
        let started = Instant::now();
        assert!(matches!(
            waiter.join().unwrap().decision,
            Decision::Cancelled
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!gate.cancel(), "nothing is left to cancel");
    }

    #[test]
    fn the_devices_are_opened_for_each_prompt_and_only_then() {
        let (gate, keys, _) = gate(10, 100);
        assert_eq!(keys.opens(), 0);
        run(&gate).join().unwrap();
        assert_eq!(keys.opens(), 1);
        run(&gate).join().unwrap();
        assert_eq!(keys.opens(), 2);
    }

    #[test]
    fn a_session_that_saw_nothing_cannot_approve() {
        // A source that opened no device at all: the prompt expires.
        struct Deaf;
        struct DeafSession;
        impl super::super::types::KeySession for DeafSession {
            fn next(&mut self, timeout: Duration) -> Option<super::super::types::KeyEvent> {
                thread::sleep(timeout.min(Duration::from_millis(5)));
                None
            }
        }
        impl KeySource for Deaf {
            fn open(&self) -> Box<dyn super::super::types::KeySession> {
                Box::new(DeafSession)
            }
        }
        let hub = Arc::new(Hub::new(1));
        let gate = Gate::new(
            Arc::new(Deaf),
            hub,
            GateConfig {
                prompt_ttl: Duration::from_millis(80),
                min_display: Duration::from_millis(1),
                slice: Duration::from_millis(10),
                ..GateConfig::default()
            },
        );
        let verdict = gate.begin(spec("x")).unwrap().wait();
        assert!(matches!(verdict.decision, Decision::Expired));
    }
}
