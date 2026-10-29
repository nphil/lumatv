//! Toolkit-neutral privacy controls. No changes occur on a status read.
pub mod assets;
pub mod boot;
pub mod consent;
pub mod hosts;
pub mod legacy;
pub mod noads;
pub mod prime;
pub mod system;
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub trait Platform: Send + Sync {
    fn luna(&self, method: &str, body: &Value) -> Result<Value, String>;
    fn declutter(&self, args: &[&str]) -> Result<Value, String>;
    fn bind(&self, source: &Path, target: &Path) -> Result<(), String>;
    fn unmount(&self, target: &Path) -> Result<(), String>;
    fn cert_hash(&self, pem: &Path) -> Result<String, String>;
    /// Closes an app through the application manager. Closing one that is not running is not an error.
    fn close_app(&self, id: &str) -> Result<(), String>;
    /// Stops the programs of an earlier AmazOff install and returns their names.
    fn stop_old_amazoff(&self) -> Result<Vec<String>, String>;
}
#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub data: PathBuf,
}
impl Paths {
    pub fn system(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }
    pub fn state(&self, name: &str) -> PathBuf {
        self.data.join("privacy").join(name)
    }
}
pub fn atomic(path: &Path, bytes: &[u8], private: bool) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(path.parent().ok_or("no parent directory")?)
        .map_err(|e| e.to_string())?;
    let tmp = path.with_extension("new");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(if private { 0o600 } else { 0o644 })
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    f.set_permissions(std::fs::Permissions::from_mode(if private {
        0o600
    } else {
        0o644
    }))
    .map_err(|e| e.to_string())?;
    use std::io::Write;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}
fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}
fn remove(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
struct Pending {
    token: String,
    key: String,
    on: bool,
    plan: Value,
    expires: Instant,
}
struct State {
    pending: Option<Pending>,
    prime: Option<prime::Front>,
    error: Option<String>,
}
/// The file the compositor looks for before it lets LG's Remote Service (`com.webos.app.remoteservice`) open. Absent is blocked, which is
/// the default; only an owner's confirmed change here writes it (`apply`), and no assistant has a route to it.
pub const REMOTE_SERVICE: &str = "remoteservice.allowed";
pub struct Privacy {
    paths: Paths,
    platform: Arc<dyn Platform>,
    state: Mutex<State>,
}
const GROUPS: &[(&str, &str, &str)] = &[
    (
        "ads",
        "Block content recognition (ACR) and ads",
        "Stops content recognition, content mining, ad overlays, sports promotions and banners.",
    ),
    (
        "settings",
        "Keep LG's viewing and ad settings off",
        "Switches LG's 16 viewing-data and ad settings off and keeps them off. Turning this off puts back the values they had.",
    ),
    (
        "uploaders",
        "Block telemetry uploads",
        "Stops LG's log, crash, usage and diagnostic uploads. Logs stay on the TV.",
    ),
    (
        "push",
        "Block LG push promotions",
        "Stops push notices, nudges and LG's smart-home client. The Content Store sign-in keeps working.",
    ),
];
impl Privacy {
    pub fn new(paths: Paths, platform: Arc<dyn Platform>) -> Self {
        Self {
            paths,
            platform,
            state: Mutex::new(State {
                pending: None,
                prime: None,
                error: None,
            }),
        }
    }
    fn consent_data(&self) -> Result<(Value, Value, consent::Groups), String> {
        let response = self.platform.luna(
            "getSystemSettings",
            &json!({"keys":["eulaStatus","eulaInfoNetwork"]}),
        )?;
        let flags = response
            .pointer("/settings/eulaStatus")
            .cloned()
            .ok_or("LG did not return consent flags")?;
        let docs = response
            .pointer("/settings/eulaInfoNetwork")
            .cloned()
            .ok_or("LG did not return agreement documents")?;
        let mapping = read_json(&self.paths.system("/var/palm/license/eulaInfoNetwork.json"))?;
        Ok((flags, docs, consent::groups(&mapping)))
    }
    pub fn status(&self, can_admin: bool) -> Value {
        let dc = self.platform.declutter(&["status"]);
        let early = dc.as_ref().ok().and_then(|v| v.get("early"));
        let mut rows = Vec::new();
        for (id, label, detail) in GROUPS {
            let group = dc
                .as_ref()
                .ok()
                .and_then(|v| v.pointer(&format!("/groups/{id}")));
            let on = group.and_then(|v| v["on"].as_bool());
            rows.push(json!({"id":id,"title":label,"detail":detail,"kind":"switch","on":on,"status":group,"available":group.is_some(),"reason":dc.as_ref().err(),"boot":boot::group(id,on,early),"admin":true}));
        }
        // Read the settings only, never the ACR/ad service: probing it would start it again.
        let mut observed = serde_json::Map::new();
        for category in ["option", "general", "other"] {
            let keys: Vec<_> = include_str!("../../../payload/declutter/items.txt")
                .lines()
                .filter(|l| l.starts_with("setting|settings|"))
                .filter_map(|l| l.split('|').nth(2)?.split_once('.'))
                .filter(|(c, _)| *c == category)
                .map(|(_, k)| k)
                .collect();
            if let Ok(reply) = self.platform.luna(
                "getSystemSettings",
                &json!({"category":category,"keys":keys}),
            ) {
                if let Some(settings) = reply["settings"].as_object() {
                    for (k, v) in settings {
                        observed.insert(format!("{category}.{k}"), v.clone());
                    }
                }
            }
        }
        let settings: Vec<Value> = include_str!("../../../payload/declutter/items.txt").lines().filter(|l|l.starts_with("setting|settings|")).filter_map(|l|{
            let f:Vec<_>=l.split('|').collect(); let key=*f.get(2)?; let actual=observed.get(key); Some(json!({"key":key,"target":serde_json::from_str::<Value>(f.get(3)?).ok()?,"detail":f.get(4)?,"value":actual,"available":actual.is_some()}))
        }).collect();
        rows[1]["settings"] = json!(settings);
        let host_on = self.paths.state("hosts.enabled").is_file();
        let effective = text(&self.paths.system("/etc/hosts"));
        rows.push(json!({"id":"hosts","title":"Block ad & tracker hosts","detail":"Blocks advertising and tracker names, including LG's and Alphonso's regional ones, for every app. The Content Store, ThinQ and update blocks stay as they are.","kind":"switch","on":host_on,"available":effective.is_ok(),"effective":effective.as_ref().is_ok_and(|s|s.lines().any(|l|l==hosts::BEGIN)),"count":hosts::names(&self.country()).len(),"boot":boot::after_boot(host_on),"admin":true}));
        let state = self.state.lock();
        let routes = state.prime.as_ref().map(|f| f.routes());
        let protection = match routes.as_ref() {
            None => {
                if self.paths.state("prime.enabled").is_file() {
                    "unavailable"
                } else {
                    "off"
                }
            }
            Some(r) if r.is_empty() => "waiting",
            Some(r) if r.values().all(|v| v["state"] == "patched") => "active",
            Some(r) if r.values().any(|v| v["state"] == "patched") => "partial",
            Some(_) => "unmodified",
        };
        let missing = if !self.paths.system(prime::CERT_DIR).is_dir() {
            Some("Prime Video is not installed on this TV")
        } else if !self.paths.system(prime::JAIL_HOSTS).is_file() {
            Some("Prime Video has not run on this TV yet")
        } else {
            None
        };
        let reason = missing.or(state.error.as_deref()).or_else(|| {
            routes
                .as_ref()
                .and_then(|r| r.values().find_map(|v| v["reason"].as_str()))
        });
        rows.push(json!({"id":"amazoff","title":"Block Prime Video ads","detail":"Takes the ads out of Prime Video. Prime Video restarts when this is turned on or off.","kind":"switch","on":self.paths.state("prime.enabled").is_file(),"running":state.prime.is_some(),"protection":protection,"available":missing.is_none(),"routes":routes.as_deref(),"reason":reason,"admin":true,"confirm":true}));
        drop(routes);
        drop(state);
        rows.push(json!({"id":"remoteservice","title":"Allow LG Remote Service","detail":"LG's remote support: with a code read out to LG, its staff can see and control this TV. Blocked unless the owner allows it here; it opens only while this is on.","kind":"switch","on":self.paths.state(REMOTE_SERVICE).is_file(),"available":true,"admin":true,"confirm":true}));
        match self.consent_data() {
            Ok((flags, docs, groups)) => {
                if let Some(flags) = flags.as_object() {
                    for (key, value) in flags.iter().filter(|(k, v)| {
                        k.ends_with("Allowed") && *k != "chpAllowed" && v.is_boolean()
                    }) {
                        let documents = groups.get(key);
                        let mut row = json!({"id":format!("consent.{key}"),"title":consent::label(key,&groups),"kind":"switch","on":value,"available":true,"settable":consent::settable(key,&groups)&&documents.is_some_and(|v|!v.is_empty()),"documents":documents,"admin":true,"confirm":true});
                        row["agreements"] = json!(
                            docs["eulaList"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|d| documents.is_some_and(|ids| d["id"]
                                    .as_str()
                                    .is_some_and(|id| ids.iter().any(|s| s == id))))
                                .cloned()
                                .collect::<Vec<_>>()
                        );
                        rows.push(row);
                    }
                }
            }
            Err(e) => rows.push(
                json!({"id":"consents","title":"LG consent switches","available":false,"reason":e}),
            ),
        }
        rows.push(json!({"id":"permissions","title":"App Permissions","kind":"link","destination":"permissions","admin":true}));
        rows.push(json!({"id":"deletedata","title":"Delete My Personal Information","kind":"link","destination":"deletedata","admin":true}));
        json!({"id":"privacy","title":"Privacy","icon":"shield-check","admin":true,"canAdmin":can_admin,"boot":boot::root(early),"items":rows})
    }
    fn country(&self) -> String {
        text(&self.paths.system("/var/lib/tvweb/country"))
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
    }
    fn plan(&self, key: &str, on: bool) -> Result<Value, String> {
        if GROUPS.iter().any(|(id, _, _)| *id == key) {
            return Ok(json!({"key":key,"on":on,"mechanism":"declutter","group":key}));
        }
        match key {
            "hosts" => Ok(
                json!({"key":key,"on":on,"hosts":hosts::names(&self.country()),"mechanism":"writable hosts table bind-mounted at /etc/hosts; original file never written"}),
            ),
            "remoteservice" => Ok(
                json!({"key":key,"on":on,"changes":[REMOTE_SERVICE],"notice":if on {"LG's remote support may open on this TV. With a code read out to LG, its staff can see the screen and control the TV. LumaTV has no menu for it; LG's own screens open it."} else {"LG's remote support stays closed: LumaTV drops any launch of it."}}),
            ),
            "amazoff" => Ok(
                json!({"key":key,"on":on,"changes":[prime::CERT_DIR,prime::JAIL_HOSTS],"notice":if on {"Prime Video restarts, and the old AmazOff app, if it is installed, is switched off."} else {"Prime Video restarts and shows its ads again."}}),
            ),
            _ if key.starts_with("consent.") => {
                let (flags, docs, groups) = self.consent_data()?;
                let changes = consent::plan(&key[8..], on, &flags, &docs, &groups)?;
                Ok(
                    json!({"key":key,"on":on,"changes":changes,"notice":"LG is told about this change. Consents that share its agreement change too; voice and LG's platform terms never do."}),
                )
            }
            _ => Err("unknown privacy item".into()),
        }
    }
    pub fn prepare(&self, key: &str, on: bool) -> Result<Value, String> {
        let plan = self.plan(key, on)?;
        let mut random = [0; 24];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|e| e.to_string())?;
        let token = hex(&random);
        self.state.lock().pending = Some(Pending {
            token: token.clone(),
            key: key.into(),
            on,
            plan: plan.clone(),
            expires: Instant::now() + Duration::from_secs(60),
        });
        Ok(json!({"confirmation":token,"expiresInSeconds":60,"plan":plan}))
    }
    pub fn apply(&self, key: &str, on: bool, token: &str) -> Result<Value, String> {
        let mut state = self.state.lock();
        let pending = state.pending.take().ok_or("show the confirmation first")?;
        if pending.token != token
            || pending.key != key
            || pending.on != on
            || pending.expires < Instant::now()
        {
            return Err("confirmation expired or does not match this change".into());
        }
        let plan = self.plan(key, on)?;
        if plan != pending.plan {
            return Err("LG's agreements changed; show a new confirmation".into());
        }
        if GROUPS.iter().any(|(id, _, _)| *id == key) {
            let result =
                self.platform
                    .declutter(&[if on { "on" } else { "off" }, "--group", key])?;
            if result["ok"] != true {
                return Err("declutter refused the group change".into());
            }
            if on {
                let result = self.platform.declutter(&["persist", "on"])?;
                if result["ok"] != true {
                    return Err("the group changed but boot persistence failed".into());
                }
            }
        } else {
            match key {
                "hosts" => self.set_hosts(on)?,
                "remoteservice" => {
                    if on {
                        atomic(&self.paths.state(REMOTE_SERVICE), b"on\n", false)?;
                    } else {
                        remove(&self.paths.state(REMOTE_SERVICE))?;
                    }
                }
                "amazoff" => {
                    if on && state.prime.is_none() {
                        prime::retire_old(&self.paths, self.platform.as_ref())?;
                        let front = prime::enable(&self.paths, self.platform.as_ref())?;
                        state.prime = Some(front);
                        state.error = None;
                    } else if !on {
                        let _ = self.platform.close_app(prime::APP);
                        prime::disable(&self.paths)?;
                        state.prime.take();
                        state.error = None;
                    }
                }
                _ => {
                    let changes = &plan["changes"];
                    self.platform.luna("setSystemSettings",&json!({"settings":{"eulaStatus":changes["eulaStatus"],"eulaInfoNetwork":changes["eulaInfoNetwork"]}}))?;
                    let verified = self.platform.luna(
                        "getSystemSettings",
                        &json!({"keys":["eulaStatus","eulaInfoNetwork"]}),
                    )?;
                    if verified.pointer("/settings/eulaStatus") != Some(&changes["eulaStatus"])
                        || verified.pointer("/settings/eulaInfoNetwork")
                            != Some(&changes["eulaInfoNetwork"])
                    {
                        return Err(
                            "LG accepted the consent change without applying it completely".into(),
                        );
                    }
                }
            }
        }
        Ok(json!({"changed":key,"on":on}))
    }
    fn set_hosts(&self, on: bool) -> Result<(), String> {
        let target = self.paths.system("/etc/hosts");
        let file = self.paths.state("hosts");
        // A covered foreign table may have changed. Read it after removing our bind, not from the stale overlay.
        use std::os::unix::fs::MetadataExt;
        let bound = std::fs::metadata(&file)
            .ok()
            .zip(std::fs::metadata(&target).ok())
            .is_some_and(|(a, b)| a.dev() == b.dev() && a.ino() == b.ino());
        if bound {
            self.platform.unmount(&target)?;
        }
        let current = text(&target)?;
        let next = hosts::table(&current, on, &self.country())?;
        if !on && current == next {
            remove(&self.paths.state("hosts.enabled"))?;
            return Ok(());
        }
        atomic(&file, next.as_bytes(), false)?;
        self.platform.bind(&file, &target)?;
        if on {
            atomic(&self.paths.state("hosts.enabled"), b"on\n", false)?;
        } else {
            remove(&self.paths.state("hosts.enabled"))?;
        }
        Ok(())
    }
    /// Boot/shutdown integration. A helper never touches the set on a host where no saved switch exists.
    pub fn resume(&self) {
        let mut errors = Vec::new();
        if self.paths.state("hosts.enabled").is_file() {
            if let Err(e) = self.set_hosts(true) {
                errors.push(format!("Hosts: {e}"));
            }
        }
        if self.paths.state("prime.enabled").is_file() {
            match prime::enable(&self.paths, self.platform.as_ref()) {
                Ok(front) => self.state.lock().prime = Some(front),
                Err(e) => {
                    let _ = prime::remove_redirect(&self.paths);
                    errors.push(format!("Amazoff: {e}"));
                }
            }
        }
        self.state.lock().error = (!errors.is_empty()).then(|| errors.join("; "));
    }
    /// Reapply only an explicitly enabled hosts block or a Prime jail file regenerated by LG at app launch.
    pub fn maintain(&self) {
        let mut state = self.state.lock();
        let result = (|| {
            if self.paths.state("hosts.enabled").is_file()
                && !text(&self.paths.system("/etc/hosts"))?
                    .lines()
                    .any(|l| l == hosts::BEGIN)
            {
                self.set_hosts(true)?;
            }
            if state.prime.is_some() {
                let path = self.paths.system(prime::JAIL_HOSTS);
                if path.is_file() {
                    let current = text(&path)?;
                    let next = hosts::prime_hosts(&current, true);
                    if current != next {
                        std::fs::write(path, next).map_err(|e| e.to_string())?;
                    }
                }
            }
            Ok::<(), String>(())
        })();
        if let Err(e) = result {
            state.error = Some(e);
        }
    }
    pub fn stop(&self) {
        // Remove the jail redirect before dropping TLS, without changing the saved user choice.
        if self.state.lock().prime.is_some() {
            let _ = prime::remove_redirect(&self.paths);
        }
        self.state.lock().prime.take();
    }
}
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}
pub fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
