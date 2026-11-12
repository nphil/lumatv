//! Beszel: the TV as a system of a Beszel hub (beszel.dev), as a plugin of the helper's own that asks first (`plugins/local.rs`'s
//! `OptIn`).
//!
//! **The hub's own agent.** While the plugin is on, the helper runs `beszel-agent` (Beszel's release for linux arm64, which `lumatv`
//! stages from the payload into `bin/beszel`) as a user of its own, confined as a plugin's program is (`plugins/runner.rs`). The agent
//! dials the hub at the page's address with the TV's token and answers only to the page's key; it listens on no port. Off, the agent
//! is stopped and nothing is read for it.
//!
//! **What the agent cannot read as itself** the helper lays out under `/run/lumatv-beszel` every 10 s:
//! - the SoC temperature (`/proc/lg/pm/temperature`, whole degrees) as the hwmon sensor `soc`, the only sensor the agent is shown
//!   (`SYS_SENSORS`);
//! - the Mali GPU's load since the last look (busy and idle time, in debugfs, which only root reads) and the memory its contexts hold,
//!   as one sample of `nvtop -lP`, which the payload's `nvtop` hands the agent (`GPU_COLLECTOR=nvtop`). The GPU has no memory of its
//!   own, so its total is the TV's.
//!
//! **The token** is a secret field: kept in `beszel/token` (mode 0600) and given only to the agent, in its environment.

use crate::config::Paths;
use crate::ctx::Ctx;
use crate::plugins::firstparty::clip;
use crate::plugins::local::{Handler, Local, OptIn, Spec};
use crate::plugins::runner::{confine, end, sweep};
use crate::plugins::session::{MAX_FACT_LINE, Shared};
use parking_lot::Mutex;
use plugin_api::wire::ToPlugin;
use serde_json::{Map, Value as Json, json};
use settings_schema::{Value as Setting, Values};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The plugin's id, which is also its settings owner.
pub const ID: &str = "beszel";
pub const OPT_IN: OptIn = OptIn {
    spec: Spec {
        id: ID,
        name: "Beszel",
        version: "1",
        permissions: &["settings"],
    },
    consent: "Lets a Beszel hub watch this TV: while this is on, Beszel's agent runs on the TV and sends the hub at the address on its \
              page the TV's processor, memory, storage, network, temperature and graphics load, with the token the hub gave for this \
              TV, which is kept on the TV.",
};

const HUB_URL: &str = "hub.url";
const HUB_KEY: &str = "hub.key";
const TOKEN: &str = "hub.token";
/// The agent's user and group: none of the TV's, and below the plugins' users, which count up from 20000.
const AGENT_ID: u32 = 19_999;
/// How often the helper lays out what the agent cannot read.
const TICK: Duration = Duration::from_secs(10);
/// How long the agent has to finish when it is stopped.
const GRACE: Duration = Duration::from_secs(2);
/// The pause before an agent that stopped is started again: doubling from the first to the last, and back to the first once it
/// connects.
const FIRST_PAUSE: Duration = Duration::from_secs(2);
const LAST_PAUSE: Duration = Duration::from_secs(60);
/// The longest the thread sleeps while the plugin is off.
const LONGEST_SLEEP: Duration = Duration::from_secs(3600);
const NEEDS: &str = "Needs the hub's address, key and token (Add System in Beszel)";
/// The agent's words for a connection that failed or ended (`connection_manager.go`, `client.go`).
const LOST: [&str; 4] = [
    "Disconnected from hub",
    "WebSocket connection failed",
    "Connection closed",
    "Error creating WebSocket client",
];

/// Where the plugin finds and puts things: the TV's ([`Files::tv`]) or a test's.
struct Files {
    /// `beszel-agent` and the `nvtop` that answers it.
    bin: PathBuf,
    /// The token and the agent's data directory.
    dir: PathBuf,
    /// What the helper lays out for the agent: a tmpfs, so none of it outlives the TV's power.
    run: PathBuf,
    /// The SoC temperature in whole degrees.
    temperature: PathBuf,
    /// The Mali driver's debugfs: `dvfs_utilization` and `gpu_memory`.
    mali: PathBuf,
    /// The GPU's model and cores (`Mali-G52 3 cores r1p0 0x07040002`).
    gpuinfo: PathBuf,
    meminfo: PathBuf,
    /// Whom the agent runs as (user, group): its own when the helper is root.
    user: Option<(u32, u32)>,
}

impl Files {
    fn tv(paths: &Paths) -> Self {
        // SAFETY: `geteuid` has no preconditions.
        let root = unsafe { libc::geteuid() } == 0;
        Self {
            bin: paths.data_dir.join("bin/beszel"),
            dir: paths.data_dir.join("beszel"),
            run: PathBuf::from("/run/lumatv-beszel"),
            temperature: PathBuf::from("/proc/lg/pm/temperature"),
            mali: PathBuf::from("/sys/kernel/debug/mali0"),
            gpuinfo: PathBuf::from("/sys/class/misc/mali0/device/gpuinfo"),
            meminfo: PathBuf::from("/proc/meminfo"),
            user: root.then_some((AGENT_ID, AGENT_ID)),
        }
    }

    fn hwmon(&self) -> PathBuf {
        self.run.join("sys/class/hwmon/hwmon0")
    }
}

/// The token, in a file of its own (mode 0600, replaced whole); never in a log, a fact or a setting.
struct Token {
    path: PathBuf,
    kept: Mutex<String>,
}

impl Token {
    fn open(path: PathBuf) -> Self {
        let kept = std::fs::read_to_string(&path)
            .map(|t| t.trim().to_owned())
            .unwrap_or_default();
        Self {
            path,
            kept: Mutex::new(kept),
        }
    }

    fn get(&self) -> String {
        self.kept.lock().clone()
    }

    /// Keeps what was typed; nothing typed forgets it.
    fn put(&self, typed: &str) -> Result<(), String> {
        let typed = typed.trim();
        if typed.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err("that token has a character it cannot have".into());
        }
        let mut kept = self.kept.lock();
        let done = if typed.is_empty() {
            std::fs::remove_file(&self.path).or_else(|e| match e.kind() {
                std::io::ErrorKind::NotFound => Ok(()),
                _ => Err(e),
            })
        } else {
            crate::guidedata::write_private(&self.path, typed.as_bytes())
        };
        done.map_err(|e| format!("it could not be kept ({e})"))?;
        *kept = typed.to_owned();
        Ok(())
    }
}

fn text_of(values: &Values, key: &str) -> String {
    match values.get(key) {
        Some(Setting::Text(t)) => t.trim().to_owned(),
        _ => String::new(),
    }
}

/// The hub's host, from its address: `beszel.example.net` from `https://beszel.example.net/`.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

/// The page, as the schema format writes it.
fn schema_json() -> Json {
    let text = |key: &str, label: &str, help: &str| {
        json!({"key": key, "label": label, "help": help, "kind": {"type": "text", "max_chars": 200, "secret": false},
               "default": {"type": "text", "value": ""}, "apply": "on-commit"})
    };
    json!({"format": 1, "owner": ID, "revision": 1, "title": OPT_IN.spec.name, "sections": [
        {"id": "hub", "title": "Hub", "fields": [
            {"key": "status", "label": "Status", "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.status")}},
             "apply": "on-commit"},
            text(HUB_URL, "Hub address", "The hub's web address, as Beszel's Add System shows it."),
            text(HUB_KEY, "Hub key", "The hub's public key from Add System (ssh-ed25519 ...)."),
            {"key": TOKEN, "label": "Token", "help": "The token Add System shows for this TV. It stays on this TV.",
             "kind": {"type": "text", "max_chars": 200, "secret": true}, "apply": "on-commit"}]}]})
}

fn register() -> Json {
    json!({"title": OPT_IN.spec.name, "settings": schema_json()})
}

/// What a line of the agent's log (Go's `slog` text: `2026/11/11 21:16:03 WARN msg key=value ...`) says.
#[derive(Debug, PartialEq)]
enum Heard {
    Connected,
    /// The connection failed or ended, and why.
    Lost(String),
    /// Another warning or error, for the helper's log.
    Problem(String),
    Nothing,
}

fn heard(line: &str) -> Heard {
    let mut parts = line.splitn(4, ' ');
    let (Some(_date), Some(_time), Some(level), Some(said)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Heard::Nothing;
    };
    let words = || said.replacen(" err=", ": ", 1).replace('"', "");
    match level {
        "INFO" if said.starts_with("WebSocket connected") => Heard::Connected,
        "WARN" | "ERROR" if LOST.iter().any(|l| said.starts_with(l)) => Heard::Lost(words()),
        "WARN" | "ERROR" => Heard::Problem(words()),
        _ => Heard::Nothing,
    }
}

/// The Mali GPU as nvtop would name it, the memory it shares with the TV, and its busy and idle time at the last look.
struct Gpu {
    name: String,
    total: u64,
    last: Option<(u32, u32)>,
}

impl Gpu {
    fn read(files: &Files) -> Self {
        let info = std::fs::read_to_string(&files.gpuinfo).unwrap_or_default();
        let meminfo = std::fs::read_to_string(&files.meminfo).unwrap_or_default();
        let total = meminfo
            .lines()
            .find_map(|l| l.strip_prefix("MemTotal:"))
            .and_then(|l| l.split_whitespace().next()?.parse::<u64>().ok())
            .unwrap_or(0);
        Self {
            name: gpu_name(&info),
            total: total * 1024,
            last: None,
        }
    }

    /// A sample once there is a look before this one.
    fn sample(&mut self, mali: &Path) -> Option<String> {
        let counters = std::fs::read_to_string(mali.join("dvfs_utilization")).ok()?;
        let mut numbers = counters
            .split_whitespace()
            .filter_map(|w| w.parse::<u64>().ok());
        // the driver's counters are 32 bits and wrap
        let now = (numbers.next()? as u32, numbers.next()? as u32);
        let last = self.last.replace(now)?;
        let memory = std::fs::read_to_string(mali.join("gpu_memory")).ok()?;
        let pages = memory
            .lines()
            .next()?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()?;
        Some(gpu_sample(&self.name, last, now, pages, self.total))
    }
}

/// `Mali-G52 MC3` from the driver's `Mali-G52 3 cores r1p0 0x07040002`.
fn gpu_name(info: &str) -> String {
    match info.split_whitespace().collect::<Vec<_>>()[..] {
        [model, cores, "cores", ..] => format!("{model} MC{cores}"),
        [model, ..] => model.to_owned(),
        [] => "Mali".to_owned(),
    }
}

/// One sample as `nvtop -lP` prints it: the load between the two looks, the memory the GPU's contexts hold (in 4 KiB pages), and
/// `total`.
fn gpu_sample(name: &str, last: (u32, u32), now: (u32, u32), pages: u64, total: u64) -> String {
    let busy = u64::from(now.0.wrapping_sub(last.0));
    let idle = u64::from(now.1.wrapping_sub(last.1));
    let load = (busy * 100).checked_div(busy + idle).unwrap_or(0);
    let sample = json!([{"device_name": name, "gpu_util": format!("{load}%"), "mem_used": (pages * 4096).to_string(),
                         "mem_total": total.to_string()}]);
    format!("{sample}\n")
}

/// `bytes` as the file at `path`, for the agent to read: written beside it, then renamed, so it never reads half of one.
fn put(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("new");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    // whatever the helper's umask is
    file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
    std::fs::rename(&tmp, path)
}

/// What the plugin's thread is told.
enum Event {
    /// The owner's switch may have changed: [`Shared::switched_on`] says how it stands.
    Switch,
    Settings(Values),
    /// The token was kept or forgotten.
    Token,
    /// A line of the agent's log.
    Said(String),
    Stop,
}

/// Offers the plugin (off until the owner switches it on) and starts its thread.
pub fn spawn(ctx: &Ctx, shared: Arc<Shared>) -> std::io::Result<JoinHandle<()>> {
    spawn_with(ctx, shared, Files::tv(&ctx.paths))
}

fn spawn_with(ctx: &Ctx, shared: Arc<Shared>, files: Files) -> std::io::Result<JoinHandle<()>> {
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        ctx.stop.on_stop(move || {
            let _ = tx.send(Event::Stop);
        });
    }
    let now = Instant::now();
    let mut plugin = Plugin {
        ctx: ctx.clone(),
        shared: shared.clone(),
        tx: tx.clone(),
        token: Arc::new(Token::open(files.dir.join("token"))),
        gpu: Gpu::read(&files),
        files,
        page: None,
        on: false,
        values: Values::new(),
        agent: None,
        due: now,
        failures: 0,
        connected: false,
        problem: None,
        logged: String::new(),
        next_look: now,
        said: Json::Null,
    };
    shared.offer(
        &OPT_IN,
        Arc::new(move || {
            let _ = tx.send(Event::Switch);
        }),
    );
    crate::plugins::guard::spawn("beszel", 256 * 1024, shared, &[ID], ctx.stop.clone(), move |again| {
        crate::sys::bulk_thread();
        if again {
            plugin.follow_afresh();
        }
        plugin.run(&rx)
    })
}

struct Plugin {
    ctx: Ctx,
    shared: Arc<Shared>,
    tx: Sender<Event>,
    files: Files,
    token: Arc<Token>,
    gpu: Gpu,
    /// Its conversation with the broker's session while it is on: its page and its facts.
    page: Option<Arc<Local>>,
    on: bool,
    values: Values,
    agent: Option<Child>,
    /// When the agent is started (again), while it is not running.
    due: Instant,
    /// How many times in a row the agent stopped or could not be started.
    failures: u32,
    connected: bool,
    /// Why the agent is not connected: its last word on it, or why it stopped or could not be started.
    problem: Option<String>,
    /// What the helper's log was told last: the agent says the same every few seconds while the hub is away.
    logged: String,
    /// When the helper lays out what the agent cannot read next.
    next_look: Instant,
    /// The facts last put: unchanged ones are not put again.
    said: Json,
}

impl Plugin {
    /// After a panic: follows the switch from scratch, the agent being as the panic left it.
    fn follow_afresh(&mut self) {
        self.on = !self.shared.switched_on(ID);
        self.switch();
    }

    fn run(&mut self, rx: &Receiver<Event>) {
        loop {
            let wait = if self.on {
                self.wake_at().saturating_duration_since(Instant::now())
            } else {
                LONGEST_SLEEP
            };
            match rx.recv_timeout(wait) {
                Ok(Event::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Event::Switch) => self.switch(),
                Ok(Event::Settings(values)) => {
                    self.values = values;
                    self.restart();
                }
                Ok(Event::Token) => self.restart(),
                Ok(Event::Said(line)) => self.hear(&line),
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.ctx.stop.is_set() {
                break;
            }
            if self.on {
                self.work();
            }
            self.publish();
        }
        self.halt();
        self.page = None;
    }

    fn configured(&self) -> bool {
        !text_of(&self.values, HUB_URL).is_empty()
            && !text_of(&self.values, HUB_KEY).is_empty()
            && !self.token.get().is_empty()
    }

    /// The next look, or the agent's start when that is sooner.
    fn wake_at(&self) -> Instant {
        if self.agent.is_none() && self.configured() {
            self.next_look.min(self.due)
        } else {
            self.next_look
        }
    }

    /// Follows the owner's switch: on registers the page and starts the agent once it has a hub; off stops it and takes the page and
    /// what was laid out for it away.
    fn switch(&mut self) {
        let on = self.shared.switched_on(ID);
        if on == self.on {
            return;
        }
        self.on = on;
        if on {
            match Local::start(
                &self.shared,
                &OPT_IN.spec,
                register(),
                handler(self.tx.clone(), self.token.clone()),
            ) {
                Ok(page) => self.page = Some(page),
                Err(e) => crate::log_warn!("beszel: the plugin's page was not registered: {e}"),
            }
            self.values = self.ctx.settings.effective(ID).unwrap_or_default();
            if let Err(e) = self.prepare() {
                crate::log_warn!("beszel: {}: {e}", self.files.run.display());
            }
            let now = Instant::now();
            self.next_look = now;
            self.due = now;
            self.failures = 0;
        } else {
            self.halt();
            self.gpu.last = None;
            let _ = std::fs::remove_dir_all(&self.files.run);
            self.page = None;
            self.said = Json::Null;
        }
    }

    /// The run directory down to the sensor's, readable by the agent whatever the helper's umask is.
    fn prepare(&self) -> std::io::Result<()> {
        let hwmon = self.files.hwmon();
        std::fs::create_dir_all(&hwmon)?;
        for dir in hwmon.ancestors().take_while(|d| d.starts_with(&self.files.run)) {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    /// The agent again with what changed: stopped now, started at once.
    fn restart(&mut self) {
        self.halt();
        self.due = Instant::now();
        self.failures = 0;
    }

    fn halt(&mut self) {
        if let Some(agent) = self.agent.take() {
            end(agent, GRACE);
        }
        self.connected = false;
        self.problem = None;
    }

    /// The pause before the next start, longer each time in a row.
    fn pause(&mut self) -> Duration {
        let pause = FIRST_PAUSE
            .saturating_mul(1 << self.failures.min(5))
            .min(LAST_PAUSE);
        self.failures += 1;
        pause
    }

    /// The look when it is due, then the agent: noticed when it stopped, started when it is due.
    fn work(&mut self) {
        let now = Instant::now();
        if now >= self.next_look {
            self.look();
            self.next_look = now + TICK;
        }
        if let Some(status) = self
            .agent
            .as_mut()
            .and_then(|a| a.try_wait().ok().flatten())
        {
            let agent = self.agent.take().map(|a| a.id());
            // what it started (the `nvtop` that answers it)
            sweep(agent);
            let pause = self.pause();
            crate::log_warn!(
                "beszel: the agent stopped ({status}); starting it again in {} s",
                pause.as_secs()
            );
            self.connected = false;
            self.problem
                .get_or_insert_with(|| format!("the agent stopped ({status})"));
            self.due = now + pause;
        }
        if self.agent.is_none() && now >= self.due && self.configured() {
            match self.start() {
                Ok(agent) => self.agent = Some(agent),
                Err(e) => {
                    self.note(&format!("the agent was not started: {e}"));
                    self.problem = Some(e);
                    self.due = now + self.pause();
                }
            }
        }
    }

    /// Lays out what the agent cannot read as itself: the SoC temperature as a hwmon sensor, the GPU's sample as nvtop's.
    fn look(&mut self) {
        let degrees = std::fs::read_to_string(&self.files.temperature)
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok());
        if let Some(degrees) = degrees {
            let hwmon = self.files.hwmon();
            let _ = put(&hwmon.join("name"), b"soc\n").and_then(|()| {
                put(&hwmon.join("temp1_input"), format!("{}\n", degrees * 1000).as_bytes())
            });
        }
        if let Some(sample) = self.gpu.sample(&self.files.mali) {
            let _ = put(&self.files.run.join("gpu"), sample.as_bytes());
        }
    }

    fn start(&self) -> Result<Child, String> {
        let program = self.files.bin.join("beszel-agent");
        if !program.is_file() {
            return Err(format!("{} is not installed", program.display()));
        }
        let data = self.files.dir.join("data");
        let prepare = || -> std::io::Result<()> {
            std::fs::create_dir_all(&data)?;
            // passed through to the data, not read: the token beside it is root's
            std::fs::set_permissions(&self.files.dir, std::fs::Permissions::from_mode(0o711))?;
            std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700))?;
            if let Some((uid, gid)) = self.files.user {
                std::os::unix::fs::chown(&data, Some(uid), Some(gid))?;
            }
            Ok(())
        };
        prepare().map_err(|e| format!("its data directory: {e}"))?;
        let mut command = Command::new(&program);
        command
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", self.files.bin.display()))
            .env("HOME", &data)
            .env("DATA_DIR", &data)
            .env("HUB_URL", text_of(&self.values, HUB_URL))
            .env("KEY", text_of(&self.values, HUB_KEY))
            .env("TOKEN", self.token.get())
            // the agent dials the hub; nothing on the network may dial it
            .env("DISABLE_SSH", "true")
            .env("SKIP_SYSTEMD", "true")
            .env("LOG_LEVEL", "info")
            .env("SYS_SENSORS", self.files.run.join("sys"))
            .env("GPU_COLLECTOR", "nvtop")
            .env("LUMATV_GPU_SAMPLE", self.files.run.join("gpu"))
            .env("NICS", "eth0,wlan0")
            // `/` is the read-only system image: the TV's storage is `/media` (the apps) and `/var` (its data)
            .env("FILESYSTEM", "/media__Apps")
            .env("EXTRA_FILESYSTEMS", "/var__Data")
            .current_dir(&data)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        // no ceiling on its address space: Go reserves far more than it touches (1.27 GB seen on the TV for 12 MB resident)
        confine(&mut command, self.files.user, None);
        let mut child = command.spawn().map_err(|e| format!("{e}"))?;
        let log = child.stderr.take();
        let tx = self.tx.clone();
        let reader = std::thread::Builder::new()
            .name("beszel-log".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let Some(log) = log else { return };
                for line in BufReader::new(log).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(Event::Said(line)).is_err() {
                        break;
                    }
                }
            });
        if let Err(e) = reader {
            end(child, Duration::ZERO);
            return Err(format!("its log: {e}"));
        }
        Ok(child)
    }

    fn hear(&mut self, line: &str) {
        match heard(line) {
            Heard::Connected => {
                if !self.connected {
                    crate::log_info!(
                        "beszel: connected to {}",
                        host(&text_of(&self.values, HUB_URL))
                    );
                }
                self.connected = true;
                self.problem = None;
                self.failures = 0;
                self.logged.clear();
            }
            Heard::Lost(why) => {
                self.connected = false;
                self.note(&why);
                self.problem = Some(why);
            }
            Heard::Problem(why) => self.note(&why),
            Heard::Nothing => {}
        }
    }

    /// Tells the helper's log, once while the same is said again.
    fn note(&mut self, words: &str) {
        if self.logged != words {
            crate::log_warn!("beszel: {words}");
            self.logged = words.to_owned();
        }
    }

    /// Puts the page's facts when they changed.
    fn publish(&mut self) {
        let Some(page) = &self.page else { return };
        let said = json!({"t": "facts.put", "facts": self.facts()});
        if said != self.said {
            if let Err(e) = page.say(said.clone()) {
                crate::log_warn!("beszel: the page's facts were not put: {e}");
            }
            self.said = said;
        }
    }

    fn facts(&self) -> Map<String, Json> {
        let url = text_of(&self.values, HUB_URL);
        let status = if !self.configured() {
            NEEDS.to_owned()
        } else if self.connected {
            format!("Connected to {}", host(&url))
        } else if let Some(problem) = &self.problem {
            format!("Not connected: {problem}")
        } else {
            format!("Connecting to {}", host(&url))
        };
        let mut facts = Map::new();
        facts.insert(
            "status".into(),
            json!(clip(&status.replace(char::is_control, " "), MAX_FACT_LINE)),
        );
        // a secret field's row reads the fact of its name, dots as dashes (`plugin_rows` in the TV's Settings)
        let kept = if self.token.get().is_empty() { "Not set" } else { "Kept on this TV" };
        facts.insert(TOKEN.replace('.', "-"), json!(kept));
        facts
    }
}

/// Takes what the base says to the thread: the page's values, and the token (kept here, at once, and answered).
fn handler(tx: Sender<Event>, token: Arc<Token>) -> Handler {
    Arc::new(move |local: &Local, message: ToPlugin| match message {
        ToPlugin::SettingsApply { txn, values, .. } => {
            let applied = json!({"t": "settings.result", "txn": txn, "outcome": {"type": "applied", "values": values}});
            let _ = tx.send(Event::Settings(values));
            let _ = local.say(applied);
        }
        ToPlugin::SecretSubmit(s) => {
            let kept = if s.field == TOKEN {
                token.put(&s.secret)
            } else {
                Err("this page has no such field".into())
            };
            let outcome = match kept {
                Ok(()) => {
                    let _ = tx.send(Event::Token);
                    json!({"type": "accepted"})
                }
                Err(reason) => json!({"type": "rejected", "reason": reason}),
            };
            let _ = local.say(json!({"t": "secret.result", "op": s.op, "outcome": outcome}));
        }
        _ => {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::control::Stop;
    use crate::hub::Hub;
    use crate::plugins::Plugins;
    use crate::settings::SettingsStore;

    fn ctx(dir: &Path) -> Ctx {
        let hub = Arc::new(Hub::new(1));
        let paths = Paths::new(dir);
        let settings = Arc::new(SettingsStore::new(paths.settings_dir(), hub.clone()));
        settings.register_builtin().unwrap();
        Ctx {
            config: Arc::new(Config::default()),
            paths,
            hub,
            settings,
            stop: Stop::new(),
        }
    }

    fn soon(what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < end, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn facts(ctx: &Ctx) -> Json {
        ctx.hub
            .get(&format!("plugin.{ID}.facts"))
            .unwrap_or_default()["facts"]
            .clone()
    }

    /// A TV's sources, and an agent that notes what it was started with and says it connected.
    fn files(dir: &Path) -> Files {
        let tv = dir.join("tv");
        std::fs::create_dir_all(tv.join("mali")).unwrap();
        std::fs::write(tv.join("temperature"), "44\n").unwrap();
        std::fs::write(tv.join("mali/dvfs_utilization"), "busy_time: 100 idle_time: 300\n").unwrap();
        std::fs::write(tv.join("mali/gpu_memory"), "mali0       10\n  kctx-0x1   10\n").unwrap();
        std::fs::write(tv.join("gpuinfo"), "Mali-G52 3 cores r1p0 0x07040002\n").unwrap();
        std::fs::write(tv.join("meminfo"), "MemTotal:        1000 kB\n").unwrap();
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let agent = bin.join("beszel-agent");
        std::fs::write(
            &agent,
            "#!/bin/sh\nenv > \"$DATA_DIR/env\"\necho $$ > \"$DATA_DIR/pid\"\n\
             echo '2026/11/11 21:16:03 INFO WebSocket connected host=hub.test' >&2\nexec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();
        Files {
            bin,
            dir: dir.join("beszel"),
            run: dir.join("run"),
            temperature: tv.join("temperature"),
            mali: tv.join("mali"),
            gpuinfo: tv.join("gpuinfo"),
            meminfo: tv.join("meminfo"),
            user: None,
        }
    }

    #[test]
    fn switched_on_with_a_hub_it_runs_the_agent_with_the_sensors_laid_out_and_off_ends_it() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let plugins = Plugins::new(&ctx);
        let threads = [
            plugins.spawn(&ctx).unwrap(),
            spawn_with(&ctx, plugins.shared().clone(), files(dir.path())).unwrap(),
        ];
        let approve = || plugins.shared().approval_of(ID);
        soon("offered, off", || approve().is_some());
        plugins.switch(ID, true, approve().as_deref()).unwrap();
        soon("its page", || facts(&ctx)["status"] == NEEDS);
        let hwmon = dir.path().join("run/sys/class/hwmon/hwmon0");
        assert_eq!(std::fs::read_to_string(hwmon.join("name")).unwrap(), "soc\n");
        assert_eq!(std::fs::read_to_string(hwmon.join("temp1_input")).unwrap(), "44000\n");

        let changes = serde_json::from_value(json!([
            {"key": HUB_URL, "value": {"type": "text", "value": "https://hub.test/"}},
            {"key": HUB_KEY, "value": {"type": "text", "value": "ssh-ed25519 AAAA"}}]))
        .unwrap();
        ctx.settings.apply(ID, changes, None).unwrap();
        plugins.shared().submit_secret(ID, TOKEN, "tk-1").unwrap();
        soon("connected", || facts(&ctx)["status"] == "Connected to hub.test");
        assert_eq!(facts(&ctx)["hub-token"], "Kept on this TV");
        let data = dir.path().join("beszel/data");
        let env = std::fs::read_to_string(data.join("env")).unwrap();
        let sensors = format!("SYS_SENSORS={}", dir.path().join("run/sys").display());
        for want in ["HUB_URL=https://hub.test/", "KEY=ssh-ed25519 AAAA", "TOKEN=tk-1", "DISABLE_SSH=true", "GPU_COLLECTOR=nvtop", &sensors] {
            assert!(env.lines().any(|l| l == want), "{want}: {env}");
        }

        let pid: libc::pid_t = std::fs::read_to_string(data.join("pid")).unwrap().trim().parse().unwrap();
        plugins.switch(ID, false, None).unwrap();
        // SAFETY: signal 0 only asks whether the process exists.
        soon("the agent ends", || unsafe { libc::kill(pid, 0) } != 0);
        soon("nothing laid out", || !dir.path().join("run").exists());
        ctx.stop.request();
        for t in threads {
            t.join().unwrap();
        }
    }

    #[test]
    fn the_gpu_sample_is_the_mali_load_between_two_looks() {
        assert_eq!(gpu_name("Mali-G52 3 cores r1p0 0x07040002"), "Mali-G52 MC3");
        // busy 40 across the 32-bit wrap, idle 60
        let sample: Json =
            serde_json::from_str(&gpu_sample("Mali-G52 MC3", (u32::MAX - 9, 300), (30, 360), 10, 1000)).unwrap();
        assert_eq!(
            sample,
            json!([{"device_name": "Mali-G52 MC3", "gpu_util": "40%", "mem_used": "40960", "mem_total": "1000"}])
        );
    }

    #[test]
    fn the_status_follows_what_the_agent_says_of_its_connection() {
        let at = "2026/11/11 21:16:03";
        assert_eq!(heard(&format!("{at} INFO WebSocket connected host=hub.test")), Heard::Connected);
        assert_eq!(
            heard(&format!("{at} WARN WebSocket connection failed err=\"dial tcp: i/o timeout\"")),
            Heard::Lost("WebSocket connection failed: dial tcp: i/o timeout".into())
        );
        assert_eq!(
            heard(&format!("{at} WARN Error collecting GPU data via nvtop err=EOF")),
            Heard::Problem("Error collecting GPU data via nvtop: EOF".into())
        );
        assert_eq!(heard(&format!("{at} INFO Detected disk name=Apps")), Heard::Nothing);
    }
}
