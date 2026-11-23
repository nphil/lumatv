use super::*;
use crate::host::Stuck;
use crate::testing::{CONVERTER, FakeHost, PID, TAIL, graph_with};
use std::sync::Mutex;

const JOURNAL: &str = "/data/audio-passthrough/journal";

fn env() -> Env {
    Env {
        enabled: true,
        screen_on: true,
        stopping: false,
        sound_output: Some("external_arc".into()),
        digital: Some("passThrough".into()),
    }
}

struct Rig {
    host: Arc<FakeHost>,
    route: Route,
    env: Env,
}

impl Rig {
    fn new() -> Self {
        let host = Arc::new(FakeHost::c4());
        let route = Route::new(host.clone(), JOURNAL);
        Self {
            host,
            route,
            env: env(),
        }
    }

    fn arm(&mut self) -> Result<String, Refusal> {
        self.route.arm(PID, TAIL, &self.env)
    }

    /// Runs the monitor for `ms` of the fake clock.
    fn run(&mut self, ms: u64) {
        let end = self.host.now() + ms;
        while self.host.now() < end {
            let wait = self.route.tick(&self.env);
            self.host.sleep_ms(wait);
        }
    }

    /// A session through the whole connect sequence.
    fn audible() -> Self {
        let mut rig = Self::new();
        rig.arm().unwrap();
        rig.run(10_000);
        assert_eq!(rig.session().phase, Phase::Audible);
        rig
    }

    fn snap(&self) -> Snapshot {
        self.route.snapshot()
    }

    fn session(&self) -> SessionView {
        self.snap().session.expect("a session")
    }

    fn count(&self, entry: &str) -> usize {
        self.host.log().iter().filter(|e| *e == entry).count()
    }
}

fn first(log: &[String], entry: &str) -> usize {
    log.iter()
        .position(|e| e == entry)
        .unwrap_or_else(|| panic!("{entry} missing from {log:?}"))
}

fn last(log: &[String], entry: &str) -> usize {
    log.iter()
        .rposition(|e| e == entry)
        .unwrap_or_else(|| panic!("{entry} missing from {log:?}"))
}

/// ADEC1 muted again and disconnected, in that order, after the session's own unmute.
fn assert_adec_restored(rig: &Rig) {
    let log = rig.host.log();
    let (unmute, remute, disconnect) = (
        first(&log, "luna mute false"),
        last(&log, "luna mute true"),
        first(&log, "luna disconnect"),
    );
    assert!(unmute < remute && remute < disconnect, "{log:?}");
    rig.host.with(|sim| {
        assert!(!sim.adec_connected);
        assert!(sim.adec_mute, "ADEC1 stays muted, as LG leaves it");
    });
}

/// LG's mix words written back, and the journal dropped.
fn assert_words_restored(rig: &Rig) {
    let log = rig.host.log();
    assert_eq!(log.last().map(String::as_str), Some("remove"), "{log:?}");
    rig.host.with(|sim| {
        assert_eq!(sim.words.to_string(), "2,1,0,0,1");
        assert!(!sim.files.contains_key(JOURNAL));
    });
}

/// What every way of ending a connected session does: the stream's owner is told, ADEC1 is muted and disconnected, LG's words are
/// back, the journal is gone and the state says how it ended.
fn assert_restored(rig: &Rig, why: Why) {
    assert_adec_restored(rig);
    assert_words_restored(rig);
    let log = rig.host.log();
    assert!(
        first(&log, "luna disconnect") < last(&log, "words 2,1,0,0,1"),
        "the words wait for the disconnect: {log:?}"
    );
    let snap = rig.snap();
    assert_eq!(snap.session, None);
    assert_eq!(snap.end.as_ref().map(|end| end.why), Some(why));
    assert_eq!(snap.pending, None);
}

// ---- arming --------------------------------------------------------------------------------------------------------------

#[test]
fn arming_journals_first_then_configures_the_converter_and_unmixes() {
    let mut rig = Rig::new();
    assert_eq!(rig.arm(), Ok(CONVERTER.to_owned()));
    assert_eq!(rig.host.log(), ["save", "cmd 0427000d", "words 2,1,0,0,0"]);
    rig.host.with(|sim| {
        let journal = Journal::parse(&sim.files[JOURNAL]).unwrap();
        assert_eq!(
            (
                journal.pid,
                journal.before.to_string(),
                journal.applied.to_string()
            ),
            (PID, "2,1,0,0,1".into(), "2,1,0,0,0".into())
        );
        assert_eq!(journal.boot, "boot-1");
    });
    let snap = rig.snap();
    assert_eq!(snap.state, State::Armed);
    assert!(!snap.ready);
    let session = snap.session.unwrap();
    assert_eq!(
        (
            session.pid,
            session.tail.as_str(),
            session.converter.as_str()
        ),
        (PID, TAIL, CONVERTER)
    );
    assert!(
        session.bypassing
            && session.converter_present
            && !session.mixed
            && !session.connected
            && !session.audible
    );
}

#[test]
fn the_receiver_is_connected_and_unmuted_in_the_proven_order_and_time() {
    let mut rig = Rig::new();
    rig.arm().unwrap();
    let armed = rig.host.now();
    rig.run(10_000);
    let log = rig.host.log();
    assert_eq!(
        &log[3..],
        ["save", "luna connect", "luna mute true", "luna mute false"],
        "{log:?}"
    );
    let (connect, mute, unmute) = (
        rig.host.at("luna connect").unwrap(),
        rig.host.at("luna mute true").unwrap(),
        rig.host.at("luna mute false").unwrap(),
    );
    assert!(connect - armed >= CONNECT_DWELL_MS, "{connect} {armed}");
    assert!(mute - connect >= MUTE_AFTER_CONNECT_MS && unmute - mute >= UNMUTE_AFTER_MUTE_MS);
    let session = rig.session();
    assert!(session.connected && session.audible);
    assert_eq!(session.phase, Phase::Audible);
    rig.host.with(|sim| {
        assert!(sim.adec_connected && !sim.adec_mute);
        assert_eq!(
            Journal::parse(&sim.files[JOURNAL]).unwrap().adec.as_deref(),
            Some("HDMI_PORT2")
        );
    });
}

#[test]
fn nothing_is_connected_before_the_dwell_has_passed() {
    let mut rig = Rig::new();
    rig.arm().unwrap();
    rig.run(CONNECT_DWELL_MS - 500);
    assert_eq!(rig.count("luna connect"), 0);
    assert_eq!(rig.session().phase, Phase::Bypassing);
}

fn refused(mutate: impl FnOnce(&mut Rig)) -> (Rig, Refusal) {
    let mut rig = Rig::new();
    mutate(&mut rig);
    let refusal = rig.arm().unwrap_err();
    // A refusal changes nothing on the TV and leaves no session.
    let log = rig.host.log();
    assert!(
        !log.iter().any(|e| e.starts_with("cmd")
            || e.starts_with("words")
            || e == "save"
            || e.starts_with("luna")),
        "{log:?}"
    );
    assert_eq!(rig.snap().session, None);
    (rig, refusal)
}

#[test]
fn every_precondition_refuses_before_touching_the_tv() {
    let wedge = Stuck {
        comm: "kworker/0:1".into(),
        wchan: String::new(),
        stack: "[<0>] rwsem_down_write_slowpath+0x300/0x500\n".into(),
    };
    type Case = (&'static str, fn(&mut Rig), &'static str, bool);
    let cases: Vec<Case> = vec![
        (
            "plugin off",
            |r| r.env.enabled = false,
            "switched off",
            false,
        ),
        (
            "firmware",
            |r| {
                r.host.with(|s| {
                    s.hashes
                        .insert("/usr/lib/libpf-1.0.so.1".into(), "0".repeat(64));
                })
            },
            "libpf-1.0.so.1",
            false,
        ),
        ("standby", |r| r.env.screen_on = false, "standby", true),
        (
            "speakers",
            |r| r.env.sound_output = Some("tv_speaker".into()),
            "tv_speaker",
            true,
        ),
        (
            "output unknown",
            |r| r.env.sound_output = None,
            "has not been read",
            true,
        ),
        (
            "eARC off",
            |r| r.host.with(|s| s.earc_on = false),
            "eARC is off",
            true,
        ),
        (
            "unknown words",
            |r| r.host.with(|s| s.words.codec = 0),
            "not a known state",
            true,
        ),
        (
            "hdmi input has the audio",
            |r| r.host.with(|s| s.adec_connected = true),
            "HDMI input",
            true,
        ),
        (
            "no such pid",
            |r| {
                r.host.with(|s| {
                    s.procs.remove(&PID);
                })
            },
            "not running",
            false,
        ),
        (
            "no renderer fd",
            |r| r.host.with(|s| s.procs.get_mut(&PID).unwrap().1.clear()),
            "holds 0",
            false,
        ),
        (
            "two renderer fds",
            |r| {
                r.host.with(|s| {
                    s.procs
                        .get_mut(&PID)
                        .unwrap()
                        .1
                        .push("/dev/lg/aren0".into())
                })
            },
            "holds 2",
            false,
        ),
        (
            "instance missing",
            |r| r.host.with(|s| s.dirs.clear()),
            "no instance directory",
            true,
        ),
        (
            "converter missing",
            |r| {
                r.host.with(|s| {
                    s.chain_graph = graph_with(TAIL, CONVERTER).replace(
                        "rts_0426001b_00000000316e9257_out0",
                        "queue_041802c9_00000000316e9257_out0",
                    )
                })
            },
            "no PCM bypass converter",
            true,
        ),
        (
            "two converters",
            |r| {
                r.host.with(|s| s.chain_graph.push_str(&format!(" rts_0426001c_0000000000000001_out0 -> srcbin_04270999_0000000000000002_in0;\n srcbin_04270999_0000000000000001_out0 -> lipsync_{TAIL}_0000000000000002_in0;\n")))
            },
            "2 converters",
            true,
        ),
    ];
    for (name, mutate, expect, retry) in cases {
        let (_, refusal) = refused(mutate);
        assert!(
            refusal.reason.contains(expect),
            "{name}: {}",
            refusal.reason
        );
        assert_eq!(refusal.retry, retry, "{name}");
    }
    let (_, refusal) = refused(|r| r.host.with(|s| s.stuck.push(wedge)));
    assert!(refusal.reason.contains("stuck") && !refusal.retry);
    let (rig, refusal) = refused(|r| {
        r.host.with(|s| {
            s.stuck.push(Stuck {
                comm: "audiooutputd".into(),
                ..Stuck::default()
            })
        })
    });
    assert!(refusal.reason.contains("busy") && refusal.retry);
    assert!(
        rig.host.now() >= 100_000 + HEALTH_RETRY_MS,
        "a busy stack is looked at again after a moment"
    );
}

#[test]
fn a_tail_that_never_joins_the_graph_is_refused_after_the_wait() {
    let mut rig = Rig::new();
    let before = rig.host.now();
    let refusal = rig.route.arm(PID, "0424ffff", &rig.env).unwrap_err();
    assert!(
        refusal.retry && refusal.reason.contains("lipsync_0424ffff"),
        "{refusal:?}"
    );
    assert!(rig.host.now() - before >= 2_000);
    assert_eq!(
        rig.route.arm(PID, "bogus", &rig.env).unwrap_err(),
        Refusal::now("tail must be 8 lowercase hex digits")
    );
    assert!(
        !rig.host
            .log()
            .iter()
            .any(|e| e != "save" && !e.starts_with("luna"))
            || rig.host.log().is_empty()
    );
}

#[test]
fn a_graph_that_fills_in_late_is_waited_for() {
    let mut rig = Rig::new();
    rig.host.with(|s| s.graph_at = s.now + 700);
    assert_eq!(rig.arm(), Ok(CONVERTER.to_owned()));
}

#[test]
fn another_process_cannot_take_a_held_route_and_cannot_release_it() {
    let mut rig = Rig::new();
    rig.host.with(|s| {
        s.procs.insert(7, (5, vec![AREN.to_owned()]));
    });
    rig.arm().unwrap();
    assert!(
        rig.route
            .arm(7, TAIL, &rig.env)
            .unwrap_err()
            .reason
            .contains("holds the route")
    );
    assert!(
        rig.route
            .release(7)
            .unwrap_err()
            .contains("holds the route")
    );
    assert!(rig.snap().session.is_some());
    assert_eq!(rig.route.release(PID), Ok(true));
    assert_eq!(rig.route.release(PID), Ok(false));
}

#[test]
fn a_hung_sound_card_write_stops_all_further_changes_until_restart() {
    let mut rig = Rig::new();
    rig.host.with(|s| s.words_error = Some(WriteError::Hung));
    let refusal = rig.arm().unwrap_err();
    assert!(refusal.reason.contains("did not answer") && !refusal.retry);
    assert!(
        rig.snap().pending.is_some(),
        "the journal stays so that a restart can put the words back"
    );
    assert!(rig.host.with(|s| s.files.contains_key(JOURNAL)));
    rig.host.with(|s| s.words_error = None);
    rig.host.with(|s| s.log.clear());
    assert!(rig.arm().unwrap_err().reason.contains("did not answer"));
    rig.run(3_000);
    assert_eq!(rig.host.log(), Vec::<String>::new(), "no more writes");
    assert!(!rig.snap().ready);
}

#[test]
fn a_failed_words_write_leaves_nothing_behind() {
    let mut rig = Rig::new();
    rig.host
        .with(|s| s.words_error = Some(WriteError::Failed("amixer exited with 1".into())));
    assert!(rig.arm().unwrap_err().reason.contains("amixer exited"));
    assert!(!rig.host.with(|s| s.files.contains_key(JOURNAL)));
    assert_eq!(rig.snap().pending, None);
}

// ---- the ways a session ends ---------------------------------------------------------------------------------------------

#[test]
fn the_stream_is_told_to_stop_before_anything_is_undone() {
    let mut rig = Rig::audible();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (host, sink) = (rig.host.clone(), seen.clone());
    rig.route.observe_with(move |snap| {
        sink.lock().unwrap().push((
            snap.session.as_ref().map(|s| (s.phase, s.bypassing)),
            host.log().len(),
        ));
    });
    let before = rig.host.log().len();
    rig.route.release(PID).unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.first(),
        Some(&(Some((Phase::Ending, false)), before)),
        "{seen:?}"
    );
    assert_eq!(seen.last().unwrap().0, None);
}

#[test]
fn release_puts_everything_back() {
    let mut rig = Rig::audible();
    assert_eq!(rig.route.release(PID), Ok(true));
    assert_restored(&rig, Why::Released);
    rig.run(2_100);
    assert_eq!(rig.snap().state, State::Ready);
}

#[test]
fn a_dead_player_puts_everything_back() {
    let mut rig = Rig::audible();
    rig.host.with(|s| {
        s.procs.remove(&PID);
    });
    rig.run(300);
    assert_restored(&rig, Why::OwnerExited);
}

#[test]
fn a_recycled_pid_is_a_dead_player() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.procs.get_mut(&PID).unwrap().0 += 1);
    rig.run(300);
    assert_restored(&rig, Why::OwnerExited);
}

#[test]
fn switching_the_plugin_off_puts_everything_back() {
    let mut rig = Rig::audible();
    rig.env.enabled = false;
    rig.run(300);
    assert_restored(&rig, Why::Disabled);
    assert_eq!(rig.snap().state, State::Off);
}

#[test]
fn standby_puts_everything_back() {
    let mut rig = Rig::audible();
    rig.env.screen_on = false;
    rig.run(300);
    assert_restored(&rig, Why::Standby);
    let snap = rig.snap();
    assert_eq!(snap.state, State::NotReady);
    assert!(snap.reason.contains("standby"));
}

#[test]
fn the_helper_stopping_puts_everything_back() {
    let mut rig = Rig::audible();
    rig.env.stopping = true;
    rig.run(300);
    assert_restored(&rig, Why::HelperExit);
}

#[test]
fn a_failed_helper_thread_puts_everything_back() {
    let mut rig = Rig::audible();
    rig.route.end(Why::Fault, "the monitor failed");
    assert_restored(&rig, Why::Fault);
}

#[test]
fn a_lost_converter_ends_the_session_after_the_rebuild_grace() {
    let mut rig = Rig::audible();
    rig.host.with(|s| {
        s.dirs.clear();
        s.configured.clear();
    });
    rig.run(2_000);
    let session = rig.session();
    assert!(
        !session.converter_present && !session.bypassing,
        "the player is told at once"
    );
    assert_eq!(rig.snap().end, None, "a rebuild is waited out");
    rig.run(4_000);
    assert_restored(&rig, Why::RouteLost);
    let snap = rig.snap();
    assert_eq!(snap.state, State::Lost);
    assert!(snap.reason.contains("converter is gone"));
    assert!(snap.ready, "a new session can start straight away");
}

#[test]
fn lg_remixing_ends_the_session_but_lgs_own_words_are_left_alone() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.words.mix = 1);
    rig.run(500);
    let session = rig.session();
    assert!(
        session.mixed && !session.bypassing,
        "the player is told at once"
    );
    rig.run(1_000);
    assert_adec_restored(&rig);
    let snap = rig.snap();
    assert_eq!((snap.state, snap.session), (State::Lost, None));
    assert!(snap.reason.contains("mixing back on"), "{}", snap.reason);
    assert_eq!(
        rig.count("words 2,1,0,0,1"),
        0,
        "words LG changed are not ours to write"
    );
    assert_eq!(rig.host.with(|s| s.words.mix), 1);
    assert!(!rig.host.with(|s| s.files.contains_key(JOURNAL)));
}

#[test]
fn a_brief_remix_inside_the_grace_is_ridden_out() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.words.mix = 1);
    rig.run(400);
    rig.host.with(|s| s.words.mix = 0);
    rig.run(3_000);
    assert!(rig.session().bypassing);
    assert_eq!(rig.snap().end, None);
}

#[test]
fn lg_taking_the_audio_input_ends_the_session_without_touching_it() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.adec_connected = false);
    rig.run(300);
    let log = rig.host.log();
    assert_eq!(rig.count("luna disconnect"), 0, "{log:?}");
    assert_eq!(
        rig.count("luna mute true"),
        1,
        "no further mute: the input is not ours any more"
    );
    assert_words_restored(&rig);
    assert!(rig.snap().reason.contains("disconnected the audio input"));
}

#[test]
fn lg_moving_the_input_to_another_source_is_left_alone() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.adec_input = "HDMI_PORT1".into());
    rig.run(300);
    assert_eq!(rig.count("luna disconnect"), 0);
    assert!(
        rig.host.with(|s| s.adec_connected),
        "LG's own connection stays"
    );
    assert_words_restored(&rig);
    assert_eq!(rig.snap().state, State::Lost);
}

#[test]
fn lg_muting_the_input_ends_the_session() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.adec_mute = true);
    rig.run(300);
    assert!(rig.snap().reason.contains("muted the audio input"));
    assert_eq!(rig.snap().state, State::Lost);
}

#[test]
fn a_wedged_sound_card_is_never_written_to_and_the_restore_waits() {
    let mut rig = Rig::audible();
    let wedge = Stuck {
        comm: "kworker/1:2".into(),
        wchan: String::new(),
        stack: "[<0>] rwsem_down_read_slowpath+0x1d4/0x420\n".into(),
    };
    rig.host.with(|s| s.stuck.push(wedge));
    rig.host.with(|s| s.log.clear());
    rig.run(2_500);
    let snap = rig.snap();
    assert_eq!(snap.state, State::Lost);
    assert!(
        snap.pending.as_deref().unwrap().contains("stuck"),
        "{snap:?}"
    );
    assert_eq!(
        rig.host.log(),
        Vec::<String>::new(),
        "no luna call or write reaches a wedged card"
    );
    assert!(
        rig.host.with(|s| s.files.contains_key(JOURNAL)),
        "the journal stays"
    );
    assert!(!snap.ready);
    // The stack recovers: the restore completes from the monitor.
    rig.host.with(|s| s.stuck.clear());
    rig.run(10_000);
    assert_eq!(rig.snap().pending, None);
    assert_words_restored(&rig);
    assert!(!rig.host.with(|s| s.adec_connected));
}

#[test]
fn the_words_wait_while_adec1_will_not_disconnect() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.disconnect_ok = false);
    assert_eq!(rig.route.release(PID), Ok(true));
    let snap = rig.snap();
    assert!(
        snap.pending
            .as_deref()
            .unwrap()
            .contains("will not disconnect")
    );
    assert_eq!(
        rig.count("words 2,1,0,0,1"),
        0,
        "the words stay un-mixed while the stream could still be flowing"
    );
    assert!(rig.host.with(|s| s.files.contains_key(JOURNAL)));
    rig.host.with(|s| s.disconnect_ok = true);
    rig.run(2_100);
    assert_eq!(rig.snap().pending, None);
    assert_words_restored(&rig);
}

// ---- re-arming, readiness, recovery --------------------------------------------------------------------------------------

#[test]
fn a_rebuilt_renderer_is_re_armed_without_reconnecting_and_keeps_lgs_original_words() {
    let mut rig = Rig::audible();
    let (tail, converter) = ("0424000f", "04270010");
    rig.host.with(|s| {
        s.dirs.clear();
        s.dirs.insert(format!("{INSTANCES}/{converter}"));
        s.chain_graph = graph_with(tail, converter);
        s.words.mix = 1; // LG re-mixed while it rebuilt
    });
    assert_eq!(rig.route.arm(PID, tail, &rig.env), Ok(converter.to_owned()));
    let session = rig.session();
    assert_eq!(
        (session.converter.as_str(), session.phase),
        (converter, Phase::Audible)
    );
    assert_eq!(rig.count("luna connect"), 1);
    assert_eq!(rig.count("words 2,1,0,0,0"), 2);
    rig.host.with(|s| {
        s.dirs.insert(format!("{INSTANCES}/{CONVERTER}"));
    });
    assert_eq!(rig.route.release(PID), Ok(true));
    assert_restored(&rig, Why::Released);
}

#[test]
fn a_rebuild_inside_the_grace_keeps_the_session() {
    let mut rig = Rig::audible();
    rig.host.with(|s| {
        s.dirs.clear();
        s.configured.clear();
    });
    rig.run(2_000);
    rig.host.with(|s| {
        s.dirs.insert(format!("{INSTANCES}/0427000e"));
        s.chain_graph = graph_with("0424000f", "0427000e");
    });
    rig.route.arm(PID, "0424000f", &rig.env).unwrap();
    rig.run(1_500);
    assert_eq!(rig.snap().end, None);
    assert!(rig.session().bypassing);
}

#[test]
fn readiness_follows_what_the_tv_says() {
    let mut rig = Rig::new();
    rig.run(2_100);
    let snap = rig.snap();
    assert_eq!((snap.state, snap.ready), (State::Ready, true));
    assert_eq!(
        (snap.earc, snap.words.as_deref(), snap.output.as_deref()),
        (Some(true), Some("2,1,0,0,1"), Some("external_arc"))
    );
    rig.env.digital = None;
    rig.run(2_100);
    assert!(
        rig.snap().ready,
        "the digital output setting is reported, not required"
    );
    for (mutate, expect) in [
        (
            Box::new(|r: &mut Rig| r.env.sound_output = Some("tv_speaker".into()))
                as Box<dyn Fn(&mut Rig)>,
            "tv_speaker",
        ),
        (
            Box::new(|r: &mut Rig| r.env.sound_output = Some("external_arc".into())),
            "",
        ),
        (
            Box::new(|r: &mut Rig| r.host.with(|s| s.earc_on = false)),
            "eARC is off",
        ),
        (
            Box::new(|r: &mut Rig| r.host.with(|s| s.earc_on = true)),
            "",
        ),
        (Box::new(|r: &mut Rig| r.env.screen_on = false), "standby"),
        (Box::new(|r: &mut Rig| r.env.screen_on = true), ""),
    ] {
        mutate(&mut rig);
        rig.run(2_100);
        let snap = rig.snap();
        if expect.is_empty() {
            assert_eq!((snap.state, snap.ready), (State::Ready, true), "{snap:?}");
        } else {
            assert!(
                snap.state == State::NotReady && !snap.ready && snap.reason.contains(expect),
                "{snap:?}"
            );
        }
    }
}

#[test]
fn a_wedge_that_appears_while_idle_is_reported_within_the_health_interval() {
    let mut rig = Rig::new();
    rig.run(2_100);
    rig.host.with(|s| {
        s.stuck.push(Stuck {
            comm: "x".into(),
            wchan: "snd_ctl_elem_write".into(),
            stack: String::new(),
        })
    });
    rig.run(IDLE_HEALTH_MS + 2_100);
    let snap = rig.snap();
    assert!(
        !snap.ready && snap.reason.contains("not healthy"),
        "{snap:?}"
    );
}

#[test]
fn a_foreign_firmware_is_refused_and_says_why() {
    let mut rig = Rig::new();
    rig.host.with(|s| {
        s.files.insert(
            "/etc/starfish-release".into(),
            "Rockhopper release 10.3.0-6000 (x)\n".into(),
        );
    });
    rig.run(2_100);
    let snap = rig.snap();
    assert_eq!(snap.state, State::Refused);
    assert!(snap.reason.contains("10.3.0-6000"), "{}", snap.reason);
    assert!(!snap.ready);
}

#[test]
fn a_disabled_plugin_reads_nothing_from_the_sound_card() {
    let mut rig = Rig::new();
    rig.env.enabled = false;
    rig.run(10_000);
    assert_eq!(rig.host.with(|s| s.sndout_reads), 0);
    let snap = rig.snap();
    assert_eq!((snap.state, snap.ready), (State::Off, false));
}

#[test]
fn a_journal_from_this_boot_is_replayed_at_start() {
    let rig = Rig::audible();
    // The helper dies here; a new one starts on the same TV.
    let mut next = Route::new(rig.host.clone(), JOURNAL);
    let note = next.recover().unwrap();
    assert!(note.contains("put LG's audio settings back"), "{note}");
    assert_adec_restored(&rig);
    assert_words_restored(&rig);
    let snap = next.snapshot();
    assert_eq!(snap.end.map(|e| e.why), Some(Why::Recovered));
    assert_eq!(snap.pending, None);
}

#[test]
fn a_journal_from_another_boot_is_dropped_without_touching_the_tv() {
    let rig = Rig::audible();
    rig.host.with(|s| {
        s.files.insert(BOOT_ID.into(), "boot-2\n".into());
        s.log.clear();
    });
    let mut next = Route::new(rig.host.clone(), JOURNAL);
    assert_eq!(next.recover().as_deref(), Some("dropped a stale journal"));
    assert_eq!(rig.host.log(), ["remove"]);
}

#[test]
fn no_journal_means_nothing_to_recover() {
    let rig = Rig::new();
    let mut route = Route::new(rig.host.clone(), JOURNAL);
    assert_eq!(route.recover(), None);
    assert!(rig.host.log().is_empty());
}

#[test]
fn recovery_of_a_wedged_tv_keeps_the_journal() {
    let rig = Rig::audible();
    rig.host.with(|s| {
        s.stuck.push(Stuck {
            comm: "x".into(),
            wchan: "rwsem".into(),
            stack: String::new(),
        });
        s.log.clear();
    });
    let mut next = Route::new(rig.host.clone(), JOURNAL);
    assert!(next.recover().unwrap().contains("not back yet"));
    assert_eq!(rig.host.log(), Vec::<String>::new());
    assert!(next.snapshot().pending.is_some());
}
