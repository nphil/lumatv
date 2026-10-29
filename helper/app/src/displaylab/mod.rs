//! The display lab's door: the shell's guided look at what the display chip can do beyond LG's own settings (the OSD sharpener's gain and
//! switch, OSD0's dimming and greying, a window cut out of the page onto the picture behind it), driven by the person with the remote
//! (`rustui/ui/src/lab/`, `docs/research/display-lab-result.md`). The shell shows one thing at a time and asks this module for it.
//!
//! This is the only code that writes a register for the lab, and it can write only what [`allow`] lists: no request names an address or a
//! value, only a control and one of its options. Every change is journaled before it is made, read back after, and put back by whichever
//! way the lab ends ([`lab`]): the shell's own end, the dead-man (six seconds without a word from the shell), the helper stopping, and the
//! next start of the helper from the journal. The routes answer only the TV's own shell: a loopback peer, the header `X-Luma-Client:
//! lumatv-ui`, and no browser mark. They are on the loopback API alone: the AI-access listener is another server with its own handler and no
//! route to this one, and no tool of its catalogue is for the display (`ai/tools.rs` pins that).
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/displaylab` | `{displaylab}`: whether the set can be reached (`available`, `reason`), the controls and their options, the session if one is open (`shown`, `held`, `heldBy` per control, `leftMs`, which controls are `usable`), how the last one ended (`last`) and what is still to put back (`pending`) |
//! | `POST /v1/displaylab/begin` | reads every target, opens the session and starts the dead-man's lease; the same answer as the status. An open session is found, not replaced |
//! | `POST /v1/displaylab/set` | `{control, option}`: shows the option (`sharpness`: `normal`, `medium`, `strong`, `soft`, `off`; `window`: `off`, `on`; `dimming`: `normal`, `dimmed`, `grey`, which the lab no longer offers: OSD0's VSP is the firmware's, which puts `0x02000001` back at once, and grey showed nothing on LumaTV's plane), renews the lease; 400 `not_allowed` for anything else |
//! | `POST /v1/displaylab/ping` | renews the lease; the answer says whether what was written still holds (`held`, `drifted`) |
//! | `POST /v1/displaylab/end` | puts everything back and closes the session; always answers, `last` says how it went |
//! | `POST /v1/displaylab/results`, `GET /v1/displaylab/results` | a run's answers, saved for the next round (`<data dir>/displaylab/results.json`), and the last run read |
//!
//! Failures: 400 `not_allowed`, 403 `not_the_ui` or `disabled` (`displayLab.enabled` in `helper.json`), 409 `no_session` or `not_original`, 500
//! `journal` or `hardware`, 502 `verify_failed`, 503 `unavailable` (not the TV the lab is for, or no access) or `stuck` (a restore is still failing).

pub mod allow;
pub mod hw;
pub mod lab;
pub mod results;

use crate::API_VERSION;
use crate::config::Paths;
use crate::control::Stop;
use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::timeutil::now_ms;
use lab::{Clock, DEADMAN_MS, Journal, Lab, LabError};
use results::Results;
use serde::Deserialize;
use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/displaylab",
        "the display lab: whether the set can be reached, the controls, the session and how the last one ended (the shell only)",
    ),
    (
        "POST",
        "/v1/displaylab/begin",
        "opens the lab's session (reads only) and starts the dead-man's lease",
    ),
    (
        "POST",
        "/v1/displaylab/set",
        "shows one option of one control: {control, option}; the journal first, a read-back after",
    ),
    (
        "POST",
        "/v1/displaylab/ping",
        "renews the lease; says whether what was written still holds",
    ),
    (
        "POST",
        "/v1/displaylab/end",
        "puts everything back and closes the session",
    ),
    (
        "POST",
        "/v1/displaylab/results",
        "saves a run's answers for the next round",
    ),
    ("GET", "/v1/displaylab/results", "the last run's answers"),
];

/// The one client of the lab: the shell's logic process (`rustui/ui/src/host/http.rs` sends it with every request).
pub const CLIENT: &str = "lumatv-ui";
/// How often the dead-man looks while a session is open or a restore is pending, and while nothing is.
const WATCH: Duration = Duration::from_millis(250);
const IDLE: Duration = Duration::from_secs(2);

pub struct DisplayLab {
    lab: Arc<Lab>,
    results: Results,
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetBody {
    control: String,
    option: String,
}

fn ok(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}

fn failed(e: &LabError) -> Response {
    let status = match e {
        LabError::NotAllowed(_) => 400,
        LabError::NoSession | LabError::NotOriginal(_) => 409,
        LabError::Journal(_) | LabError::Hardware(_) => 500,
        LabError::Verify(_) => 502,
        LabError::Unavailable(_) | LabError::Stuck(_) => 503,
    };
    error_response(status, e.code(), &e.message())
}

/// The requests the lab answers are the TV's own shell's: a loopback peer, its header, and nothing a web page can add.
fn from_the_shell(request: &Request) -> Result<(), Response> {
    if !request.peer.to_canonical().is_loopback() {
        return Err(error_response(
            403,
            "not_the_ui",
            "the display lab answers the TV's own shell on this machine only",
        ));
    }
    if request.header(crate::http::CLIENT_HEADER) != Some(CLIENT) {
        return Err(error_response(
            403,
            "not_the_ui",
            "the display lab answers the TV's own shell only",
        ));
    }
    if request.header("origin").is_some() || request.header("sec-fetch-site").is_some() {
        return Err(error_response(
            403,
            "not_the_ui",
            "a web page is not the TV's shell",
        ));
    }
    Ok(())
}

fn object(request: &Request) -> Result<Value, Response> {
    serde_json::from_slice::<Value>(&request.body)
        .map_err(|e| error_response(400, "bad_request", &format!("the body is not JSON: {e}")))
}

impl DisplayLab {
    /// The lab on the set's own hardware, with the journal of an earlier run put back before anything can be asked of it.
    pub fn start(ctx: &Ctx) -> Arc<Self> {
        Self::on(
            ctx.config.display_lab.enabled,
            Arc::new(hw::LgRegs::set()),
            Arc::new(now_ms),
            &ctx.paths,
            DEADMAN_MS,
        )
    }

    /// The lab on `regs`, `clock` and the data directory of `paths`; a test's way in.
    pub fn on(
        enabled: bool,
        regs: Arc<dyn hw::Regs>,
        clock: Clock,
        paths: &Paths,
        deadman_ms: u64,
    ) -> Arc<Self> {
        let dir = paths.data_dir.join("displaylab");
        let lab = Arc::new(Lab::new(
            regs,
            clock,
            Journal::new(dir.join("journal.json")),
            deadman_ms,
        ));
        // a journal an earlier run left is put back at once, whether or not the lab is switched on: a switch that is turned off after a crash
        // must not leave a register changed
        lab.recover();
        Arc::new(Self {
            lab,
            results: Results::new(dir),
            enabled,
        })
    }

    pub fn lab(&self) -> &Arc<Lab> {
        &self.lab
    }

    /// The dead-man's thread: it looks every 250 ms while something is open, and puts everything back when the helper stops.
    pub fn spawn(self: &Arc<Self>, stop: Stop) -> std::io::Result<JoinHandle<()>> {
        let me = self.clone();
        std::thread::Builder::new()
            .name("displaylab".into())
            .spawn(move || {
                loop {
                    let wait = if me.lab.busy() { WATCH } else { IDLE };
                    if stop.sleep(wait) {
                        break;
                    }
                    let lab = me.lab.clone();
                    if catch_unwind(AssertUnwindSafe(|| lab.tick())).is_err() {
                        crate::log_error!("displaylab: the dead-man's check panicked");
                    }
                }
                me.lab.shutdown();
            })
    }

    pub fn handle(&self, request: &Request, rest: &[&str]) -> Response {
        if let Err(r) = from_the_shell(request) {
            return r;
        }
        if !self.enabled {
            return error_response(
                403,
                "disabled",
                "the display lab is switched off (displayLab.enabled in helper.json)",
            );
        }
        let (get, post) = (request.method == "GET", request.method == "POST");
        match rest {
            [] if get => ok(json!({"displaylab": self.lab.status()})),
            ["begin"] if post => self.answer(self.lab.begin()),
            ["ping"] if post => self.answer(self.lab.ping()),
            ["end"] if post => self.answer(self.lab.end()),
            ["set"] if post => match object(request).and_then(|v| {
                serde_json::from_value::<SetBody>(v).map_err(|e| {
                    error_response(
                        400,
                        "bad_request",
                        &format!("the body must be {{control, option}}: {e}"),
                    )
                })
            }) {
                Ok(b) => self.answer(self.lab.set(&b.control, &b.option)),
                Err(r) => r,
            },
            ["results"] if post => match object(request) {
                Ok(doc) => match Results::check(&doc) {
                    Ok(()) => match self.results.save(&doc, now_ms()) {
                        Ok(name) => {
                            crate::log_info!("displaylab: a run's answers saved as {name}");
                            ok(json!({"saved": name}))
                        }
                        Err(e) => error_response(
                            500,
                            "io",
                            &format!("the results could not be saved: {e}"),
                        ),
                    },
                    Err(why) => error_response(400, "bad_request", &why),
                },
                Err(r) => r,
            },
            ["results"] if get => ok(json!({"results": self.results.latest()})),
            [] | ["begin" | "ping" | "end" | "set" | "results"] => {
                error_response(405, "method_not_allowed", "that method is not served here")
            }
            _ => error_response(404, "not_found", "no such endpoint"),
        }
    }

    fn answer(&self, r: Result<Value, LabError>) -> Response {
        match r {
            Ok(status) => ok(json!({"displaylab": status})),
            Err(e) => failed(&e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::displaylab::allow::Target;
    use crate::displaylab::hw::fake::FakeRegs;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Rig {
        lab: Arc<DisplayLab>,
        regs: Arc<FakeRegs>,
        time: Arc<AtomicU64>,
        dir: tempfile::TempDir,
    }

    fn rig_with(enabled: bool, regs: FakeRegs) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let regs = Arc::new(regs);
        let time = Arc::new(AtomicU64::new(5_000_000));
        let clock: Clock = {
            let t = time.clone();
            Arc::new(move || t.load(Ordering::SeqCst))
        };
        let lab = DisplayLab::on(
            enabled,
            regs.clone(),
            clock,
            &Paths::new(dir.path()),
            DEADMAN_MS,
        );
        Rig {
            lab,
            regs,
            time,
            dir,
        }
    }

    fn rig() -> Rig {
        rig_with(true, FakeRegs::the_set())
    }

    fn req(method: &str, body: &str, headers: &[(&str, &str)]) -> Request {
        Request {
            method: method.into(),
            path: String::new(),
            query: vec![],
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            body: body.as_bytes().to_vec(),
            peer: std::net::Ipv4Addr::LOCALHOST.into(),
        }
    }

    /// A request from the shell.
    fn shell(method: &str, body: &str) -> Request {
        req(method, body, &[("x-luma-client", CLIENT)])
    }

    fn call(r: &Rig, method: &str, rest: &[&str], body: &str) -> (u16, Value) {
        let resp = r.lab.handle(&shell(method, body), rest);
        (resp.status, serde_json::from_slice(&resp.body).unwrap())
    }

    #[test]
    fn the_shell_runs_a_session_through_the_routes() {
        let r = rig();
        let (s, v) = call(&r, "GET", &[], "");
        assert_eq!(
            (
                s,
                v["ok"].as_bool(),
                v["displaylab"]["available"].as_bool(),
                v["displaylab"]["session"].is_null()
            ),
            (200, Some(true), Some(true), true),
            "{v}"
        );
        let (s, v) = call(&r, "POST", &["begin"], "");
        assert_eq!(s, 200, "{v}");
        assert_eq!(v["displaylab"]["session"]["id"], 1);
        let (s, v) = call(
            &r,
            "POST",
            &["set"],
            r#"{"control":"sharpness","option":"strong"}"#,
        );
        assert_eq!(s, 200, "{v}");
        assert_eq!(v["displaylab"]["session"]["shown"]["sharpness"], "strong");
        assert_eq!(v["displaylab"]["session"]["held"], true);
        assert_eq!(r.regs.get(Target::Gain), 0x80);
        let (s, v) = call(&r, "POST", &["ping"], "");
        assert_eq!(
            (s, v["displaylab"]["session"]["held"].as_bool()),
            (200, Some(true))
        );
        let (s, v) = call(&r, "POST", &["end"], "");
        assert_eq!(s, 200, "{v}");
        assert!(v["displaylab"]["session"].is_null());
        assert_eq!(v["displaylab"]["last"]["why"], "end");
        assert_eq!(r.regs.get(Target::Gain), 0x10);
    }

    #[test]
    fn only_the_tvs_own_shell_is_answered() {
        let r = rig();
        let mut remote = shell("POST", "");
        remote.peer = "192.168.1.9".parse().unwrap();
        let cases: Vec<(&str, Request)> = vec![
            ("a peer that is not this machine", remote),
            ("no client header", req("POST", "", &[])),
            (
                "the QML shell's header",
                req("POST", "", &[("x-luma-client", "home")]),
            ),
            (
                "an assistant's name",
                req("POST", "", &[("x-luma-client", "Cody")]),
            ),
            (
                "the right name in another case",
                req("POST", "", &[("x-luma-client", "LumaTV-UI")]),
            ),
            (
                "a web page",
                req(
                    "POST",
                    "",
                    &[("x-luma-client", CLIENT), ("origin", "http://evil.example")],
                ),
            ),
            (
                "a browser's fetch mark",
                req(
                    "POST",
                    "",
                    &[("x-luma-client", CLIENT), ("sec-fetch-site", "cross-site")],
                ),
            ),
        ];
        for (what, request) in cases {
            for rest in [
                vec!["begin"],
                vec!["set"],
                vec!["end"],
                vec!["ping"],
                vec![],
                vec!["results"],
            ] {
                let resp = r.lab.handle(&request, &rest);
                let body: Value = serde_json::from_slice(&resp.body).unwrap();
                assert_eq!(
                    (resp.status, body["error"]["code"].as_str()),
                    (403, Some("not_the_ui")),
                    "{what} {rest:?}"
                );
            }
        }
        assert!(
            r.regs.writes().is_empty() && r.regs.reads() == 0,
            "nothing was read or written"
        );
        // a loopback peer in the IPv6 spelling of the same machine
        let mut v6 = shell("GET", "");
        v6.peer = "::ffff:127.0.0.1".parse().unwrap();
        assert_eq!(r.lab.handle(&v6, &[]).status, 200);
        let mut v6 = shell("GET", "");
        v6.peer = "::1".parse().unwrap();
        assert_eq!(r.lab.handle(&v6, &[]).status, 200);
    }

    #[test]
    fn a_request_that_names_an_address_or_a_value_is_refused_and_nothing_is_written() {
        let r = rig();
        call(&r, "POST", &["begin"], "");
        for body in [
            r#"{"control":"sharpness","option":"strong","value":255}"#,
            r#"{"control":"dimming","option":"dimmed","address":"0xCC480360"}"#,
            r#"{"addr":"0xCC480360","value":"0x0"}"#,
            r#"{"control":"0xCC480360","option":"0xFFFFFFFF"}"#,
            r#"{"control":"vsp","option":"grey"}"#,
            r#"{"control":"dimming","option":"0x02000003"}"#,
            r#"{"control":"sharpness"}"#,
            r#"{"control":["dimming"],"option":"grey"}"#,
            r#"[]"#,
            r#"null"#,
            r#"not json"#,
            r#""#,
        ] {
            let (s, v) = call(&r, "POST", &["set"], body);
            assert!(s == 400, "{body}: {s} {v}");
        }
        let (s, v) = call(
            &r,
            "POST",
            &["set"],
            r#"{"control":"dimming","option":"0x02000003"}"#,
        );
        assert_eq!(
            (s, v["error"]["code"].as_str()),
            (400, Some("not_allowed")),
            "{v}"
        );
        assert!(r.regs.writes().is_empty(), "{:?}", r.regs.writes());
        assert!(!r.dir.path().join("displaylab/journal.json").exists());
    }

    #[test]
    fn nothing_else_is_served_under_the_prefix() {
        let r = rig();
        for (method, rest) in [
            ("DELETE", vec!["begin"]),
            ("PUT", vec!["set"]),
            ("GET", vec!["set"]),
            ("GET", vec!["begin"]),
            ("POST", vec![]),
            ("POST", vec!["regs"]),
            ("POST", vec!["write"]),
            ("GET", vec!["journal"]),
            ("POST", vec!["set", "x"]),
            ("POST", vec!["..", "begin"]),
            ("GET", vec!["results", "latest"]),
        ] {
            let (s, v) = call(&r, method, &rest, "{}");
            assert!(s == 404 || s == 405, "{method} {rest:?}: {s} {v}");
        }
        assert!(r.regs.writes().is_empty() && r.regs.reads() == 0);
    }

    #[test]
    fn a_lab_that_is_switched_off_answers_nothing_and_still_puts_back_what_an_earlier_run_left() {
        let r = rig_with(false, FakeRegs::the_set());
        let (s, v) = call(&r, "POST", &["begin"], "");
        assert_eq!(
            (s, v["error"]["code"].as_str()),
            (403, Some("disabled")),
            "{v}"
        );
        assert!(r.regs.reads() == 0 && r.regs.writes().is_empty());
        // a crashed run's journal, and the switch turned off afterwards: the start still puts it back
        let first = rig();
        call(&first, "POST", &["begin"], "");
        call(
            &first,
            "POST",
            &["set"],
            r#"{"control":"dimming","option":"grey"}"#,
        );
        assert_eq!(first.regs.get(Target::Vsp), 0x0200_0003);
        let clock: Clock = {
            let t = first.time.clone();
            Arc::new(move || t.load(Ordering::SeqCst))
        };
        let _off = DisplayLab::on(
            false,
            first.regs.clone(),
            clock,
            &Paths::new(first.dir.path()),
            DEADMAN_MS,
        );
        assert_eq!(
            first.regs.get(Target::Vsp),
            0,
            "put back at the start although the lab is off"
        );
    }

    #[test]
    fn a_machine_that_is_not_the_set_says_so_and_touches_nothing() {
        let r = rig_with(true, FakeRegs::not_the_set());
        let (s, v) = call(&r, "GET", &[], "");
        assert_eq!(
            (s, v["displaylab"]["available"].as_bool()),
            (200, Some(false))
        );
        assert!(
            v["displaylab"]["reason"]
                .as_str()
                .unwrap()
                .contains("not an LG set")
        );
        let (s, v) = call(&r, "POST", &["begin"], "");
        assert_eq!(
            (s, v["error"]["code"].as_str()),
            (503, Some("unavailable")),
            "{v}"
        );
        let (s, v) = call(
            &r,
            "POST",
            &["set"],
            r#"{"control":"sharpness","option":"strong"}"#,
        );
        assert_eq!(
            (s, v["error"]["code"].as_str()),
            (409, Some("no_session")),
            "{v}"
        );
        assert!(r.regs.writes().is_empty() && r.regs.reads() == 0);
    }

    #[test]
    fn the_dead_mans_thread_puts_back_a_lab_that_went_quiet_and_everything_when_the_helper_stops() {
        let dir = tempfile::tempdir().unwrap();
        let regs = Arc::new(FakeRegs::the_set());
        // a real clock and a short lease: the thread's own timing is what is tried
        let lab = DisplayLab::on(
            true,
            regs.clone(),
            Arc::new(now_ms),
            &Paths::new(dir.path()),
            400,
        );
        let stop = Stop::new();
        let handle = lab.spawn(stop.clone()).unwrap();
        lab.lab().begin().unwrap();
        lab.lab().set("sharpness", "strong").unwrap();
        assert_eq!(regs.get(Target::Gain), 0x80);
        // the thread notices within a couple of seconds: it looks every two seconds while idle and every 250 ms once a session is open
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        while regs.get(Target::Gain) != 0x10 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(regs.get(Target::Gain), 0x10, "the dead-man fired");
        assert!(!lab.lab().open());
        // a lab that is open when the helper stops is put back by the thread's last act
        lab.lab().begin().unwrap();
        lab.lab().set("dimming", "grey").unwrap();
        stop.request();
        handle.join().unwrap();
        assert_eq!(regs.get(Target::Vsp), 0);
        assert!(!dir.path().join("displaylab/journal.json").exists());
    }

    #[test]
    fn results_are_saved_for_the_next_round_and_read_back() {
        let r = rig();
        let (s, v) = call(&r, "GET", &["results"], "");
        assert_eq!((s, v["results"].is_null()), (200, true));
        let run = r#"{"run":"r1","steps":[{"id":"sharpness-strong","answer":"b"},{"id":"dimming","answer":"yes"}]}"#;
        let (s, v) = call(&r, "POST", &["results"], run);
        assert_eq!(s, 200, "{v}");
        assert!(v["saved"].as_str().unwrap().starts_with("results-"));
        let (s, v) = call(&r, "GET", &["results"], "");
        assert_eq!((s, v["results"]["run"].as_str()), (200, Some("r1")));
        assert!(v["results"]["savedAtMs"].as_u64().unwrap() > 1_700_000_000_000);
        assert!(r.dir.path().join("displaylab/results.json").exists());
        for bad in [r#"{"steps":[{"id":"../x"}]}"#, r#"{"run":1}"#, "[1]", "x"] {
            assert_eq!(call(&r, "POST", &["results"], bad).0, 400, "{bad}");
        }
        assert!(
            r.regs.writes().is_empty() && r.regs.reads() == 0,
            "the results touch no register"
        );
        let _ = r.time.load(Ordering::SeqCst);
    }

    #[test]
    fn the_routes_are_listed_and_the_ai_catalogue_has_no_tool_for_the_display() {
        let listed: Vec<&str> = ROUTES.iter().map(|r| r.1).collect();
        for path in [
            "/v1/displaylab",
            "/v1/displaylab/begin",
            "/v1/displaylab/set",
            "/v1/displaylab/ping",
            "/v1/displaylab/end",
            "/v1/displaylab/results",
        ] {
            assert!(listed.contains(&path), "{path}");
        }
        assert!(ROUTES.iter().all(|r| r.1.starts_with("/v1/displaylab")));
        for def in crate::ai::tools::catalogue() {
            for word in [
                "display", "lab", "register", "mem", "sharp", "dim", "vsp", "hole", "window",
                "picture",
            ] {
                assert!(!def.name.contains(word), "{} mentions {word}", def.name);
                assert!(
                    !def.description.to_lowercase().contains("register"),
                    "{}",
                    def.name
                );
            }
        }
    }
}
