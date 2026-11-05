//! Live TV Tuner: the TV's antenna tuner shared on the home network as an HDHomeRun, with an M3U playlist and the TV's own guide as
//! XMLTV (the `lumatuner` engine), as a plugin of the helper's own that asks first ([`OPT_IN`], `plugins/local.rs`).
//!
//! **A plugin of the helper's own, not a program of its own.** Tuning, the demux tap and the PVR download's certificate need root, LG's
//! luna services and LG's HAL, which only the trusted helper has; a plugin on the socket runs as a user of its own with none of them. So the
//! engine runs in the helper, and the plugin is the helper speaking the plugin API: listed with the other plugins, its settings page drawn
//! like theirs, switched like theirs by an approval of its words that `enabled.json` keeps.
//!
//! **Off until the owner switches it on** in Settings: the `plugins` topic lists it, off, with its consent, and Settings asks the owner in
//! those words (`POST /v1/plugins/live-tv-tuner/switch`). While it is off nothing listens and the tuner is left alone. On, it serves the
//! tuner on the TV's private address (`helper.json` `hdhr`: port, bind, allowedNets and the tuner's nodes) and says where it stands as
//! facts of its page: the status (idle, streaming a channel, busy while the TV's own Live TV has the tuner), the three addresses a player
//! takes and the networks it answers. The page's one setting is the name players show; its one action looks at the TV now.
//!
//! **`base.livetv`**, the Settings switch this replaced, is carried over at the start ([`migrate`]): a TV that shared its tuner comes up
//! with the plugin on (the owner approved sharing the tuner when they turned it on) and the same name.

use crate::ctx::Ctx;
use crate::plugins::firstparty::clip;
use crate::plugins::local::{Handler, Local, OptIn, Spec};
use crate::plugins::roster;
use crate::plugins::session::{MAX_FACT_LINE, Shared};
use crate::settings::SettingsStore;
use lumatuner::{Cidr, Config as Offer, Status, Tuner};
use plugin_api::wire::ToPlugin;
use serde_json::{Map, Value as Json, json};
use settings_schema::{Value as Setting, Values};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// The plugin's id, which is also its settings owner.
pub const ID: &str = "live-tv-tuner";
pub const OPT_IN: OptIn = OptIn {
    spec: Spec {
        id: ID,
        name: "Live TV Tuner",
        version: "1",
        permissions: &["settings"],
    },
    consent: "Shares this TV's antenna tuner on the local network as an HDHomeRun, with a channel playlist and the TV's own guide: devices on the \
              home network can list its channels, watch them and record them. One stream at a time, and the TV's own Live TV always comes first.",
};
/// The Settings switch the plugin replaced.
const OLD_OWNER: &str = "base.livetv";
/// What players call the tuner until the owner names it.
const DEFAULT_NAME: &str = "LumaTV";
/// The page's action: look at the TV now (Settings runs it when the page opens).
const CHECK: &str = "check";
/// How often a listener that should be up but is not tries again, and how often an up one checks that the TV still has its address and,
/// while the TV's own use has the tuner, whether it still has.
const RETRY: Duration = Duration::from_secs(15);
const RECHECK: Duration = Duration::from_secs(60);

/// Carries `base.livetv` over to the plugin, once: shared means switched on, with the plugin's words approved (the owner approved sharing
/// the tuner on the home network when they turned the switch on), and the name it had is the plugin's. The old file goes once that is done;
/// when `enabled.json` cannot be written it stays, to be carried over at the next start.
pub fn migrate(settings: &SettingsStore, plugins_dir: &Path) {
    let Some(old) = settings.stored(OLD_OWNER) else {
        return;
    };
    let shared = matches!(old.get("enabled"), Some(Setting::Bool(true)));
    if shared {
        let approved = roster::read_file(plugins_dir).and_then(|mut file| {
            file.first_party.retain(|a| a.id != ID);
            file.first_party.push(roster::Approved {
                id: ID.to_owned(),
                approve: OPT_IN.approval(),
            });
            roster::write_file(plugins_dir, &file)
        });
        if let Err(why) = approved {
            crate::log_warn!("tuner: {OLD_OWNER} was on and could not be carried over: {why}");
            return;
        }
    }
    match settings.carry_over(OLD_OWNER, ID, &["name"]) {
        Ok(()) => crate::log_info!(
            "tuner: {OLD_OWNER} carried over to the {} plugin, {}",
            OPT_IN.spec.name,
            if shared { "on" } else { "off" }
        ),
        Err(e) => crate::log_warn!("tuner: {OLD_OWNER} could not be carried over: {e}"),
    }
}

/// What the plugin's thread is told.
enum Event {
    /// The owner's switch may have changed: [`Shared::switched_on`] says how it stands.
    Switch,
    /// The name players show.
    Name(String),
    /// The engine's status changed.
    Status,
    /// Look at the TV now.
    Check,
    Stop,
}

/// Offers the plugin (off until the owner switches it on) and starts the thread that serves `tuner` while it is on. The thread ends when
/// the stop is requested, closing the listener.
pub fn spawn(ctx: &Ctx, shared: Arc<Shared>, tuner: Arc<Tuner>) -> std::io::Result<JoinHandle<()>> {
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        ctx.stop.on_stop(move || {
            let _ = tx.send(Event::Stop);
        });
    }
    {
        let tx = tx.clone();
        tuner.on_change(move || {
            let _ = tx.send(Event::Status);
        });
    }
    let mut plugin = Plugin::new(ctx, shared.clone(), tuner, tx.clone());
    shared.offer(
        &OPT_IN,
        Arc::new(move || {
            let _ = tx.send(Event::Switch);
        }),
    );
    thread::Builder::new()
        .name("tuner".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || plugin.run(&rx))
}

/// The plugin's state, held by its thread.
struct Plugin {
    ctx: Ctx,
    shared: Arc<Shared>,
    tuner: Arc<Tuner>,
    tx: Sender<Event>,
    /// Its conversation with the broker's session while it is on: its page, its facts, its action.
    page: Option<Arc<Local>>,
    on: bool,
    name: String,
    device_id: String,
    allowed: Vec<Cidr>,
    /// Why it is not listening though it is on.
    problem: Option<String>,
    /// The facts last put: an unchanged state is not put again.
    said: Json,
}

impl Plugin {
    fn new(ctx: &Ctx, shared: Arc<Shared>, tuner: Arc<Tuner>, tx: Sender<Event>) -> Self {
        let cfg = &ctx.config.hdhr;
        let mut allowed = Vec::new();
        for net in &cfg.allowed_nets {
            match Cidr::parse(net) {
                Some(cidr) => allowed.push(cidr),
                None => {
                    crate::log_warn!("tuner: allowedNets: {net:?} is not a network and is ignored")
                }
            }
        }
        let mac = std::fs::read_to_string(&cfg.mac_file).unwrap_or_default();
        Self {
            ctx: ctx.clone(),
            shared,
            tuner,
            tx,
            page: None,
            on: false,
            name: DEFAULT_NAME.to_owned(),
            device_id: lumatuner::docs::device_id(&mac),
            allowed,
            problem: None,
            said: Json::Null,
        }
    }

    fn run(&mut self, rx: &Receiver<Event>) {
        loop {
            let wait = if self.problem.is_some() {
                RETRY
            } else {
                RECHECK
            };
            match rx.recv_timeout(wait) {
                Ok(Event::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Event::Switch) => self.switch(),
                Ok(Event::Name(name)) => {
                    self.name = name;
                    self.reconcile();
                }
                Ok(Event::Status) => {}
                Ok(Event::Check) => {
                    self.tuner.check();
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.reconcile();
                    if matches!(self.tuner.status(), Status::Busy(_)) {
                        self.tuner.check();
                    }
                }
            }
            if self.ctx.stop.is_set() {
                break;
            }
            self.publish();
        }
        self.tuner.stop();
        self.page = None;
    }

    /// Follows the owner's switch: on registers the page and serves the tuner, off closes the listener and takes the page away.
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
                handler(self.tx.clone()),
            ) {
                Ok(page) => self.page = Some(page),
                Err(e) => crate::log_warn!("tuner: the plugin's page was not registered: {e}"),
            }
            self.name = self
                .ctx
                .settings
                .effective(ID)
                .map_or_else(|| DEFAULT_NAME.to_owned(), |v| name_of(&v));
        } else {
            self.page = None;
            self.said = Json::Null;
        }
        self.reconcile();
    }

    /// Makes the listener match the switch: up exactly while the plugin is on, under its name, on the address the TV has now.
    fn reconcile(&mut self) {
        if !self.on {
            self.tuner.stop();
            self.problem = None;
            return;
        }
        let cfg = &self.ctx.config.hdhr;
        let Some(ip) = crate::sys::lan_ipv4(cfg.bind.as_deref().and_then(|b| b.parse().ok()))
        else {
            self.tuner.stop();
            self.problem = Some("the TV has no private network address to listen on".to_owned());
            return;
        };
        let offer = Offer {
            name: self.name.clone(),
            device_id: self.device_id.clone(),
            addr: SocketAddr::new(ip.into(), cfg.port),
            allowed: self.allowed.clone(),
        };
        match self.tuner.serve(&offer) {
            Ok(_) => self.problem = None,
            Err(e) => {
                let why = e.to_string();
                if self.problem.as_deref() != Some(why.as_str()) {
                    crate::log_warn!("tuner: cannot listen: {why}");
                }
                self.problem = Some(why);
            }
        }
    }

    /// Puts what the page says when it changed: the status, the addresses while it listens, the networks it answers.
    fn publish(&mut self) {
        let Some(page) = &self.page else {
            return;
        };
        let line = |text: String| clip(&text.replace(char::is_control, " "), MAX_FACT_LINE);
        let mut facts = Map::new();
        let status = match &self.problem {
            Some(why) => format!("Not listening: {why}"),
            None => self.tuner.status().to_string(),
        };
        facts.insert("status".to_owned(), json!(line(status)));
        if let Some(addr) = self.tuner.addr() {
            for (key, path) in [
                ("discover", "discover.json"),
                ("playlist", "lineup.m3u"),
                ("guide", "xmltv.xml"),
            ] {
                facts.insert(key.to_owned(), json!(format!("http://{addr}/{path}")));
            }
        }
        let networks = self
            .allowed
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        facts.insert("networks".to_owned(), json!(line(networks)));
        let said = json!({"t": "facts.put", "facts": facts});
        if said != self.said {
            if let Err(e) = page.say(said.clone()) {
                crate::log_warn!("tuner: the page's facts were not put: {e}");
            }
            self.said = said;
        }
    }
}

/// The name in the page's values: trimmed, and the default when there is none.
fn name_of(values: &Values) -> String {
    match values.get("name") {
        Some(Setting::Text(name)) if !name.trim().is_empty() => name.trim().to_owned(),
        _ => DEFAULT_NAME.to_owned(),
    }
}

/// The page, as the schema format writes it: the name players show, and the facts it says.
fn schema_json() -> Json {
    let fact = |key: &str, label: &str| {
        json!({"key": key, "label": label, "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.{key}")}},
               "apply": "on-commit"})
    };
    json!({"format": 1, "owner": ID, "revision": 1, "title": OPT_IN.spec.name, "sections": [
        {"id": "tuner", "title": "Tuner", "fields": [
            fact("status", "Status"),
            {"key": "name", "label": "Name shown in players", "kind": {"type": "text", "max_chars": 40, "secret": false},
             "default": {"type": "text", "value": DEFAULT_NAME}, "apply": "on-commit"},
            fact("networks", "Answers")]},
        {"id": "addresses", "title": "Addresses", "fields": [
            fact("discover", "HDHomeRun (Plex, Jellyfin, Emby, Channels DVR)"),
            fact("playlist", "Playlist (VLC, TiviMate, Kodi)"),
            fact("guide", "Guide (XMLTV)")]}]})
}

fn register() -> Json {
    json!({"title": OPT_IN.spec.name, "settings": schema_json(), "actions": [{"key": CHECK, "label": "Check the tuner"}]})
}

/// Takes the page's answers to the thread: a new name, and the action that looks at the TV.
fn handler(tx: Sender<Event>) -> Handler {
    Arc::new(move |local: &Local, message: ToPlugin| match message {
        ToPlugin::ActionInvoke { op, action, .. } => {
            let known = action == CHECK;
            if known {
                let _ = tx.send(Event::Check);
            }
            let _ = local.answer(op, if known { Ok(()) } else { Err("no such action") });
        }
        ToPlugin::SettingsApply { txn, values, .. } => {
            let _ = tx.send(Event::Name(name_of(&values)));
            let _ = local.say(json!({"t": "settings.result", "txn": txn, "outcome": {"type": "applied", "values": values}}));
        }
        _ => {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, HdhrConfig, Paths};
    use crate::control::Stop;
    use crate::hub::Hub;
    use crate::plugins::{Plugins, SwitchError};
    use lumatuner::{Failure, Hardware, Tap};
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Instant;

    /// A TV with no channels and no tuner: what serving the documents needs.
    struct NoTv;

    impl Hardware for NoTv {
        fn channel_list(&self) -> Result<Json, Failure> {
            Ok(json!({"channelList": []}))
        }

        fn programmes(&self, _: &str) -> Result<Json, Failure> {
            Ok(json!({"programList": []}))
        }

        fn in_use(&self) -> Option<String> {
            None
        }

        fn tune(&self, _: &str) -> Result<Box<dyn Tap>, Failure> {
            Err(Failure::Hardware("no tuner in a test".into()))
        }
    }

    fn ctx(dir: &Path) -> Ctx {
        let hub = Arc::new(Hub::new(1));
        let paths = Paths::new(dir);
        let settings = Arc::new(SettingsStore::new(paths.settings_dir(), hub.clone()));
        settings.register_builtin().unwrap();
        let hdhr = HdhrConfig {
            port: 0,
            bind: Some("127.0.0.1".to_owned()),
            allowed_nets: vec!["127.0.0.0/8".to_owned()],
            mac_file: dir.join("mac").display().to_string(),
            ..HdhrConfig::default()
        };
        Ctx {
            config: Arc::new(Config {
                hdhr,
                ..Config::default()
            }),
            paths,
            hub,
            settings,
            stop: Stop::new(),
        }
    }

    /// The plugin manager and the plugin's thread, as the helper starts them.
    fn start(ctx: &Ctx) -> (Arc<Plugins>, Arc<Tuner>, Vec<JoinHandle<()>>) {
        let plugins = Plugins::new(ctx);
        let tuner = Arc::new(Tuner::new(Arc::new(NoTv), lumatuner::real_clock()));
        let threads = vec![
            plugins.spawn(ctx).unwrap(),
            spawn(ctx, plugins.shared().clone(), tuner.clone()).unwrap(),
        ];
        (plugins, tuner, threads)
    }

    fn stop(ctx: &Ctx, threads: Vec<JoinHandle<()>>) {
        ctx.stop.request();
        for t in threads {
            t.join().unwrap();
        }
    }

    /// Waits (a moment at most: two threads hand it over) until `done` holds.
    fn soon(what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < end, "{what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn entry(ctx: &Ctx) -> Json {
        let topic = ctx.hub.get("plugins").unwrap_or_default();
        topic["firstParty"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["id"] == ID)
            .cloned()
            .unwrap_or_default()
    }

    fn name_served(addr: SocketAddr) -> String {
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(b"GET /discover.json HTTP/1.1\r\n\r\n").unwrap();
        let mut answer = String::new();
        c.read_to_string(&mut answer).unwrap();
        let body = answer.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str::<Json>(body).unwrap()["FriendlyName"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn the_plugin_is_off_until_switched_on_and_the_switch_starts_and_stops_the_tuner() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let (plugins, tuner, threads) = start(&ctx);
        soon("listed, off, with its words", || {
            entry(&ctx)["state"] == "off"
        });
        assert_eq!(entry(&ctx)["consent"], OPT_IN.consent);
        assert_eq!(tuner.addr(), None, "off: nothing listens");
        assert_eq!(
            plugins.switch(ID, true, Some("not the code")),
            Err(SwitchError::Consent)
        );
        assert_eq!(
            plugins.switch("weather", true, None),
            Err(SwitchError::NotOffered)
        );

        plugins
            .switch(ID, true, Some(entry(&ctx)["approve"].as_str().unwrap()))
            .unwrap();
        soon("on: the tuner listens", || tuner.addr().is_some());
        assert_eq!(name_served(tuner.addr().unwrap()), DEFAULT_NAME);
        soon("its page says where it stands", || {
            ctx.hub
                .get(&format!("plugin.{ID}.facts"))
                .is_some_and(|f| f["facts"]["status"] == "Idle")
        });
        let facts = ctx.hub.get(&format!("plugin.{ID}.facts")).unwrap();
        assert_eq!(
            facts["facts"]["playlist"],
            format!("http://{}/lineup.m3u", tuner.addr().unwrap())
        );
        assert_eq!(facts["facts"]["networks"], "127.0.0.0/8");
        assert_eq!(entry(&ctx)["state"], "connected");

        plugins.switch(ID, false, None).unwrap();
        soon("off again: the listener is closed", || {
            tuner.addr().is_none()
        });
        soon("and the plugin is listed off", || {
            entry(&ctx)["state"] == "off"
        });
        assert!(
            ctx.settings.effective(ID).is_none(),
            "its page is gone with it"
        );
        stop(&ctx, threads);
    }

    #[test]
    fn a_tv_that_shared_its_tuner_with_base_livetv_comes_up_with_the_plugin_on_under_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let old = ctx.paths.settings_dir().join("base.livetv.json");
        std::fs::create_dir_all(ctx.paths.settings_dir()).unwrap();
        std::fs::write(
            &old,
            r#"{"owner": "base.livetv", "schemaRevision": 1, "version": 2,
                "values": {"enabled": {"type": "bool", "value": true}, "name": {"type": "text", "value": "Den"}}}"#,
        )
        .unwrap();
        migrate(&ctx.settings, &ctx.paths.plugins_dir());
        assert!(!old.exists(), "carried over once");
        let (_plugins, tuner, threads) = start(&ctx);
        soon("on from the start", || tuner.addr().is_some());
        soon("under the name it had", || {
            name_served(tuner.addr().unwrap()) == "Den"
        });
        assert_eq!(entry(&ctx)["state"], "connected");
        stop(&ctx, threads);
        assert_eq!(
            tuner.addr(),
            None,
            "the helper going down closes the listener"
        );

        // a switch that was off is carried over as off, and its file goes too
        let dir = tempfile::tempdir().unwrap();
        let ctx = self::ctx(dir.path());
        std::fs::create_dir_all(ctx.paths.settings_dir()).unwrap();
        let old = ctx.paths.settings_dir().join("base.livetv.json");
        std::fs::write(
            &old,
            r#"{"owner": "base.livetv", "schemaRevision": 1, "version": 1, "values": {"enabled": {"type": "bool", "value": false}}}"#,
        )
        .unwrap();
        migrate(&ctx.settings, &ctx.paths.plugins_dir());
        assert!(!old.exists());
        assert!(
            roster::read_file(&ctx.paths.plugins_dir())
                .unwrap()
                .first_party
                .is_empty()
        );
    }
}
