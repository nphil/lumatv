use super::*;
use crate::host::Stuck;
use crate::testing::{ADEC, CONVERTER, FakeHost, PID, TAIL, graph_with};
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
        self.route.arm(PID, TAIL, ADEC, &self.env)
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

/// What every way of ending a connected session does: the stream's owner is told, the input is muted and disconnected, LG's words
/// are back, the journal is gone, the state says how it ended, the TV reads as it did before and its HDMI service was asked.
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
    assert_eq!(
        snap.after, None,
        "the TV reads as it did before the session"
    );
    assert!(
        rig.host.with(|sim| sim.hdmi_calls) >= 1,
        "the HDMI input service is asked after the restore"
    );
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
        assert_eq!(
            (journal.adec, journal.kernel.as_str(), journal.connected),
            (
                ADEC,
                "Open=1 Connect=HDMI_PORT2 Start=0 Gain=0x800000 Mute=1",
                false
            )
        );
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
        assert!(Journal::parse(&sim.files[JOURNAL]).unwrap().connected);
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
            "audio routed to another input",
            |r| r.host.with(|s| s.others = vec![0]),
            "ADEC0",
            true,
        ),
        (
            "the granted decoder is running",
            |r| r.host.with(|s| s.adec_start = true),
            "decoding for LG",
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
    let refusal = rig.route.arm(PID, "0424ffff", ADEC, &rig.env).unwrap_err();
    assert!(
        refusal.retry && refusal.reason.contains("lipsync_0424ffff"),
        "{refusal:?}"
    );
    assert!(rig.host.now() - before >= 2_000);
    assert_eq!(
        rig.route.arm(PID, "bogus", ADEC, &rig.env).unwrap_err(),
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
            .arm(7, TAIL, ADEC, &rig.env)
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
    assert_eq!(
        rig.route.arm(PID, tail, ADEC, &rig.env),
        Ok(converter.to_owned())
    );
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
    rig.route.arm(PID, "0424000f", ADEC, &rig.env).unwrap();
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

#[test]
fn a_journal_nobody_replayed_blocks_arming_so_lgs_words_are_not_lost() {
    let rig = Rig::audible();
    // A restarted helper that has not replayed the journal yet, with ADEC1 already free.
    rig.host.with(|s| {
        s.adec_connected = false;
        s.log.clear();
    });
    let mut next = Route::new(rig.host.clone(), JOURNAL);
    let refusal = next.arm(PID, TAIL, ADEC, &env()).unwrap_err();
    assert!(
        refusal.reason.contains("not been replayed") && !refusal.retry,
        "{refusal:?}"
    );
    assert_eq!(rig.host.log(), Vec::<String>::new());
    let journal = rig.host.with(|s| s.files[JOURNAL].clone());
    assert_eq!(
        Journal::parse(&journal).unwrap().before.to_string(),
        "2,1,0,0,1"
    );
}

// ---- the granted decoder, the exact restore and the HDMI service ---------------------------------------------------------

#[test]
fn the_decoder_the_player_was_granted_is_the_one_used_whatever_its_number() {
    let mut rig = Rig::new();
    rig.host.with(|s| s.adec = 3);
    assert_eq!(
        rig.route.arm(PID, TAIL, 3, &rig.env),
        Ok(CONVERTER.to_owned())
    );
    rig.run(10_000);
    // The fake refuses every call that names another decoder, so a session that got this far used ADEC3 and nothing else.
    let session = rig.session();
    assert_eq!((session.adec, session.phase), (3, Phase::Audible));
    assert_eq!(
        rig.host
            .with(|s| Journal::parse(&s.files[JOURNAL]).unwrap().adec),
        3
    );
    assert_eq!(rig.route.release(PID), Ok(true));
    assert_restored(&rig, Why::Released);
}

#[test]
fn a_decoder_that_is_not_there_is_refused_before_anything_changes() {
    let mut rig = Rig::new();
    let beyond = rig
        .route
        .arm(PID, TAIL, MAX_ADEC + 1, &rig.env)
        .unwrap_err();
    assert!(beyond.reason.contains("not a decoder input") && !beyond.retry);
    let unreadable = rig.route.arm(PID, TAIL, 4, &rig.env).unwrap_err();
    assert!(unreadable.reason.contains("ADEC4 cannot be read") && !unreadable.retry);
    assert_eq!(rig.host.log(), Vec::<String>::new());
    assert_eq!(rig.snap().session, None);
}

#[test]
fn an_input_found_unmuted_is_not_toggled_and_goes_back_unmuted_once_the_player_lets_go() {
    let mut rig = Rig::new();
    rig.host.with(|s| s.adec_mute = false);
    rig.arm().unwrap();
    rig.run(10_000);
    assert_eq!(rig.session().phase, Phase::Audible);
    assert_eq!(
        rig.count("luna mute true") + rig.count("luna mute false"),
        0,
        "the connect alone made it audible"
    );
    rig.host.with(|s| s.procs.get_mut(&PID).unwrap().1.clear());
    assert_eq!(rig.route.release(PID), Ok(true));
    let log = rig.host.log();
    let (silence, unmute, disconnect) = (
        first(&log, "luna mute true"),
        first(&log, "luna mute false"),
        first(&log, "luna disconnect"),
    );
    assert!(silence < unmute && unmute < disconnect, "{log:?}");
    rig.host
        .with(|s| assert!(!s.adec_mute && !s.adec_connected));
    assert_eq!(rig.snap().after, None);
    assert_words_restored(&rig);
}

#[test]
fn an_input_found_unmuted_stays_muted_while_the_player_still_streams_and_the_page_says_so() {
    let mut rig = Rig::new();
    rig.host.with(|s| s.adec_mute = false);
    rig.arm().unwrap();
    rig.run(10_000);
    let before = rig.host.now();
    assert_eq!(rig.route.release(PID), Ok(true));
    assert!(
        rig.host.now() - before
            >= u64::from(RENDERER_CLOSE_WAIT.polls) * RENDERER_CLOSE_WAIT.every_ms,
        "the player is given time to close its renderer"
    );
    assert_eq!(
        rig.count("luna mute false"),
        0,
        "no unmute while the stream may still flow"
    );
    rig.host.with(|s| assert!(s.adec_mute && !s.adec_connected));
    let after = rig.snap().after.expect("the difference is reported");
    assert!(
        after.contains("Mute=1") && after.contains("Mute=0"),
        "{after}"
    );
    assert_words_restored(&rig);
    rig.run(2_100);
    assert!(
        rig.snap().ready,
        "a difference LG clears on its own does not block the next session"
    );
}

#[test]
fn a_silent_hdmi_service_after_a_restore_holds_the_next_session_off_until_it_answers() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.hdmi_ok = false);
    let asked = rig.host.with(|s| s.hdmi_calls);
    assert_eq!(rig.route.release(PID), Ok(true));
    assert_eq!(
        rig.host.with(|s| s.hdmi_calls),
        asked + 1,
        "asked once, after the restore"
    );
    assert_words_restored(&rig);
    let after = rig.snap().after.expect("reported");
    assert!(after.contains("HDMI input service"), "{after}");
    rig.run(2_100);
    assert!(!rig.snap().ready);
    let refusal = rig.arm().unwrap_err();
    assert!(
        refusal.reason.contains("HDMI input service") && !refusal.retry,
        "{refusal:?}"
    );
    rig.host.with(|s| s.hdmi_ok = true);
    rig.run(IDLE_HEALTH_MS + 2_100);
    let snap = rig.snap();
    assert!(snap.ready && snap.after.is_none(), "{snap:?}");
    assert_eq!(rig.arm(), Ok(CONVERTER.to_owned()));
}

#[test]
fn an_input_that_takes_the_audio_mid_session_ends_it_and_is_left_alone() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.others = vec![0]);
    rig.run(300);
    let snap = rig.snap();
    assert_eq!(snap.state, State::Lost);
    assert!(snap.reason.contains("ADEC0"), "{}", snap.reason);
    assert!(snap.after.as_deref().unwrap().contains("ADEC0"), "{snap:?}");
    assert_eq!(
        rig.count("luna disconnect"),
        1,
        "only the session's own decoder is disconnected"
    );
    rig.host
        .with(|s| assert_eq!(s.others, vec![0], "LG's own connection is untouched"));
    assert_words_restored(&rig);
}

#[test]
fn a_journal_written_before_the_connect_never_touches_a_decoder_lg_connected_itself() {
    let mut rig = Rig::new();
    rig.arm().unwrap();
    let journal = rig.host.with(|s| s.files[JOURNAL].clone());
    assert!(!Journal::parse(&journal).unwrap().connected);
    // The helper dies; LG then starts an input that connects the same decoder from the same source.
    rig.host.with(|s| {
        s.adec_connected = true;
        s.belief_muted = false;
        s.log.clear();
    });
    let mut next = Route::new(rig.host.clone(), JOURNAL);
    assert!(
        next.recover()
            .unwrap()
            .contains("put LG's audio settings back")
    );
    assert_eq!(
        rig.count("luna disconnect") + rig.count("luna mute true"),
        0
    );
    rig.host
        .with(|s| assert!(s.adec_connected, "LG's own connection stays"));
    assert_words_restored(&rig);
}
