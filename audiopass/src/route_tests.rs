use super::*;
use crate::host::Stuck;
use crate::sample::{NOT_READ, READING_FRESH_MS, Sample};
use crate::testing::{
    ADEC, CONVERTER, FakeHost, Fault, PID, PIPELINE, PORT, Pipe, TAIL, graph_with,
};
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

    /// Arms as the player does: the arm, then its word that the stream flows once its sink has turned `pcm-multi-ch` back on.
    fn arm(&mut self) -> Result<String, Refusal> {
        let converter = self.route.arm(PID, TAIL, ADEC, &self.env)?;
        self.route.flowing(PID).map_err(Refusal::now)?;
        Ok(converter)
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
    // A stopping helper keeps its few seconds for the restore itself and skips the check afterwards.
    if why != Why::HelperExit {
        assert!(
            rig.host.with(|sim| sim.hdmi_calls) >= 1,
            "the HDMI input service is asked after the restore"
        );
    }
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
        wchan: "snd_ctl_elem_info".into(),
        stack: String::new(),
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
        (
            "LG's mixer for another app's stream",
            |r| {
                r.host.lg_mixer();
                r.host
                    .with(|s| s.pipelines[0].app = "youtube.leanback.v4".into());
            },
            "youtube.leanback.v4 holds PCMMC4",
            true,
        ),
        (
            "another app playing beside the player's stream",
            |r| {
                r.host.lg_mixer();
                r.host.with(|s| {
                    s.pipelines.push(Pipe {
                        id: "_avc".into(),
                        app: "com.webos.app.hdmi1".into(),
                        resources: vec![("MAIN_SOUND".into(), 0)],
                    })
                });
            },
            "com.webos.app.hdmi1 holds MAIN_SOUND0",
            true,
        ),
        (
            "the player's process is not the app SAM runs",
            |r| {
                r.host.lg_mixer();
                r.host.with(|s| s.apps[0].1 = PID + 1);
            },
            "SAM lists no app as process 4242",
            true,
        ),
        (
            "the player's stream on another mix channel",
            |r| {
                r.host.lg_mixer();
                r.host.with(|s| s.pipelines[0].resources[1].1 = 3);
            },
            "com.beb.plxnative holds no PCMMC4",
            true,
        ),
        (
            "LG's mixer beside an HDMI input",
            |r| {
                r.host.lg_mixer();
                r.host.with(|s| s.others = vec![2]);
            },
            "ADEC2",
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

#[test]
fn a_read_that_times_out_right_after_the_words_were_written_keeps_the_journal() {
    let mut rig = Rig::new();
    // The driver stops answering the moment the mix words are written: they are changed, but not read back.
    rig.host
        .with(|s| s.trip = Some(("words 2,1,0,0,0".to_owned(), Fault::Blocked)));
    let refusal = rig.arm().unwrap_err();
    assert!(refusal.reason.contains("did not take") && !refusal.retry);
    assert!(
        rig.snap().pending.is_some(),
        "the journal is what puts the words back"
    );
    rig.host.with(|s| {
        assert!(s.files.contains_key(JOURNAL));
        assert_eq!(s.words.to_string(), "2,1,0,0,0");
    });
    // Nothing new starts over an unrestored journal.
    let again = rig.arm().unwrap_err();
    assert!(
        again.reason.contains("settings are not back yet"),
        "{}",
        again.reason
    );
    // Once the driver answers, the restore puts the words back and drops the journal.
    rig.host.with(|s| s.fault = None);
    rig.run(2_100);
    assert_eq!(rig.snap().pending, None);
    assert_words_restored(&rig);
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
fn lg_re_mixing_once_is_undone_and_a_second_time_ends_the_session_leaving_lgs_words() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.words.mix = 1);
    rig.run(300);
    assert_eq!(rig.count("words 2,1,0,0,0"), 2, "un-mixed again");
    assert_eq!(rig.snap().end, None);
    assert!(rig.session().bypassing);
    rig.host.with(|s| s.words.mix = 1);
    rig.run(300);
    assert_adec_restored(&rig);
    let snap = rig.snap();
    assert_eq!((snap.state, snap.session), (State::Lost, None));
    assert!(snap.reason.contains("a second time"), "{}", snap.reason);
    assert_eq!(rig.count("words 2,1,0,0,0"), 2);
    assert_eq!(
        rig.count("words 2,1,0,0,1"),
        0,
        "words LG changed are not ours to write"
    );
    assert!(!rig.host.with(|s| s.files.contains_key(JOURNAL)));
}

#[test]
fn lg_re_mixing_while_another_decoder_decodes_ends_the_session_at_once() {
    let mut rig = Rig::audible();
    rig.host.with(|s| {
        s.files.insert(
            sndout::adec_path(0),
            "Open=1 Connect=HDMI_PORT1 Start=1 Gain=0x800000 Mute=0\n".into(),
        );
        s.words.mix = 1;
    });
    rig.run(300);
    let snap = rig.snap();
    assert_eq!(snap.state, State::Lost);
    assert!(snap.reason.contains("ADEC0 is decoding"), "{}", snap.reason);
    assert_eq!(rig.count("words 2,1,0,0,0"), 1, "not un-mixed again");
}

#[test]
fn the_words_wait_for_the_players_word_that_its_stream_flows() {
    let mut rig = Rig::new();
    rig.route.arm(PID, TAIL, ADEC, &rig.env).unwrap();
    rig.run(UNMIX_FALLBACK_MS - 200);
    assert_eq!(rig.host.log(), ["save", "cmd 0427000d"]);
    let session = rig.session();
    assert!(session.mixed && !session.bypassing && rig.snap().end.is_none());
    assert_eq!(rig.route.flowing(PID), Ok(()));
    assert_eq!(rig.host.log(), ["save", "cmd 0427000d", "words 2,1,0,0,0"]);
    assert!(rig.session().bypassing);
    rig.run(10_000);
    assert_eq!(rig.count("words 2,1,0,0,0"), 1);
    assert_eq!(rig.session().phase, Phase::Audible);
}

#[test]
fn without_the_players_word_the_words_are_unmixed_after_the_fallback() {
    let mut rig = Rig::new();
    rig.route.arm(PID, TAIL, ADEC, &rig.env).unwrap();
    rig.run(UNMIX_FALLBACK_MS - 200);
    assert_eq!(rig.count("words 2,1,0,0,0"), 0);
    rig.run(400);
    assert_eq!(rig.count("words 2,1,0,0,0"), 1);
    assert!(rig.session().bypassing);
    assert_eq!(
        rig.route.flowing(PID),
        Ok(()),
        "a late word changes nothing"
    );
    assert_eq!(rig.count("words 2,1,0,0,0"), 1);
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
        wchan: "snd_ctl_elem_info".into(),
        stack: String::new(),
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
            wchan: "snd_ctl_elem_info".into(),
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
fn a_busy_audio_driver_waits_and_is_not_taken_for_a_foreign_firmware() {
    let mut rig = Rig::new();
    rig.run(2_100);
    // A read of the driver's files is stuck, so its files fail at once. The firmware's own files do not.
    rig.host.with(|s| s.fault = Some(Fault::Blocked));
    rig.run(2_100);
    let snap = rig.snap();
    assert_eq!(
        (snap.state, snap.ready),
        (State::NotReady, false),
        "{snap:?}"
    );
    assert!(
        snap.reason.contains("audio driver is busy"),
        "{}",
        snap.reason
    );
    assert_eq!(snap.firmware.as_ref().map(|fw| &fw.verdict), Some(&Ok(())));
    let refusal = rig.arm().unwrap_err();
    assert!(
        refusal.retry && refusal.reason.contains("audio driver is busy"),
        "{}",
        refusal.reason
    );
    // The next read of the driver's files finds it answering again.
    rig.host.with(|s| s.fault = None);
    rig.run(2_100);
    assert_eq!(rig.snap().state, State::Ready);
}

#[test]
fn an_identity_that_cannot_be_read_waits_and_is_read_again_instead_of_refusing() {
    let mut rig = Rig::new();
    let release = rig
        .host
        .with(|s| s.files.remove("/etc/starfish-release"))
        .unwrap();
    rig.run(2_100);
    let snap = rig.snap();
    assert_eq!(
        (snap.state, snap.ready),
        (State::NotReady, false),
        "{snap:?}"
    );
    assert!(
        snap.reason.contains("cannot read /etc/starfish-release"),
        "{}",
        snap.reason
    );
    assert!(snap.firmware.as_ref().is_some_and(|fw| fw.unread));
    assert!(
        rig.arm().unwrap_err().retry,
        "a player is told to ask again"
    );
    rig.host
        .with(|s| s.files.insert("/etc/starfish-release".into(), release));
    rig.run(2_100);
    assert_eq!(rig.snap().state, State::Ready);
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
            wchan: "snd_ctl_elem_info".into(),
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
fn a_wedged_task_after_a_restore_holds_the_next_session_off_and_skips_the_ping() {
    let mut rig = Rig::audible();
    rig.host.with(|s| s.wedge_on_words = true);
    let asked = rig.host.with(|s| s.hdmi_calls);
    assert_eq!(rig.route.release(PID), Ok(true));
    assert_eq!(
        rig.host.with(|s| s.hdmi_calls),
        asked,
        "a wedged stack is found in /proc and not asked anything"
    );
    let after = rig.snap().after.expect("reported");
    assert!(after.contains("extinput-integration"), "{after}");
    rig.run(2_100);
    assert!(!rig.snap().ready);
    let refusal = rig.arm().unwrap_err();
    assert!(
        refusal.reason.contains("extinput-integration") && !refusal.retry,
        "{refusal:?}"
    );
    rig.host.with(|s| {
        s.wedge_on_words = false;
        s.stuck.clear();
    });
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

// ---- LG's own input on the decoder, how long the route is held, and what a cut-off restore leaves --------------------------

/// LG's HDMI input starts decoding on the session's ADEC. The kernel line still reads `Connect=HDMI_PORT2`; only `Start=` and the
/// mute change.
fn lg_starts_decoding(rig: &Rig) {
    rig.host.with(|s| {
        s.adec_start = true;
        s.adec_mute = false;
        s.belief_muted = false;
        s.log.clear();
    });
}

/// Nothing of LG's live input was muted or disconnected, and the words are back.
fn assert_lgs_input_untouched(rig: &Rig) {
    assert_eq!(
        rig.count("luna mute true") + rig.count("luna mute false") + rig.count("luna disconnect"),
        0,
        "{:?}",
        rig.host.log()
    );
    rig.host.with(|s| {
        assert!(
            s.adec_connected && s.adec_start && !s.adec_mute,
            "LG's live audio stays as LG made it"
        );
    });
    assert_words_restored(rig);
}

#[test]
fn an_hdmi_input_decoding_on_the_same_adec_ends_the_session_and_is_left_alone() {
    let mut rig = Rig::audible();
    lg_starts_decoding(&rig);
    rig.run(300);
    let snap = rig.snap();
    assert_eq!(snap.state, State::Lost);
    assert!(snap.reason.contains("LG is using ADEC1"), "{}", snap.reason);
    assert_lgs_input_untouched(&rig);
    let after = snap.after.expect("reported");
    assert!(after.contains("left as LG has it"), "{after}");
}

#[test]
fn an_hdmi_switch_just_before_the_release_is_left_alone_too() {
    let mut rig = Rig::audible();
    lg_starts_decoding(&rig);
    assert_eq!(rig.route.release(PID), Ok(true));
    assert_lgs_input_untouched(&rig);
    assert_eq!(rig.snap().end.map(|e| e.why), Some(Why::Released));
}

#[test]
fn a_journal_replayed_after_lg_took_the_decoder_leaves_lgs_connection() {
    let rig = Rig::audible();
    // The helper dies; the viewer switches to HDMI2 before the next one starts: same decoder, same source, now decoding.
    lg_starts_decoding(&rig);
    let mut next = Route::new(rig.host.clone(), JOURNAL);
    let note = next.recover().unwrap();
    assert!(note.contains("put LG's audio settings back"), "{note}");
    assert_lgs_input_untouched(&rig);
    let after = next.snapshot().after.expect("reported");
    assert!(after.contains("left as LG has it"), "{after}");
}

#[test]
fn watching_a_session_asks_luna_nothing_and_never_holds_the_route_for_a_luna_call() {
    let mut rig = Rig::audible();
    rig.host.with(|s| {
        s.luna_ms = 300;
        s.scan_ms = 40;
        s.luna_calls = 0;
    });
    let end = rig.host.now() + 10_000;
    while rig.host.now() < end {
        let started = rig.host.now();
        let wait = rig.route.tick(&rig.env);
        let held = rig.host.now() - started;
        assert!(
            held <= ARMED_TICK_MS,
            "one step held the route for {held} ms"
        );
        rig.host.sleep_ms(wait);
    }
    assert_eq!(rig.host.with(|s| s.luna_calls), 0);
    assert!(rig.session().audible);
}

#[test]
fn a_sample_says_bypass_only_from_a_fresh_reading_of_the_sessions_converter() {
    let rig = Rig::audible();
    let snap = rig.snap();
    let read_at = snap.session.as_ref().unwrap().at;
    let (newer, _) = sample::read(&*rig.host, CONVERTER);
    assert!(newer.at > read_at);
    let mut published = Sample::default();
    let mut look = |reading: Option<&Reading>, now: u64| {
        published.refresh(&snap, reading, now);
        (published.bypassing, published.reason.clone())
    };
    let passing = (true, "Passthrough is audible".to_owned());
    let stale = (false, NOT_READ.to_owned());
    // The route's own reading, fresh and then too old.
    assert_eq!(look(None, read_at + READING_FRESH_MS), passing);
    assert_eq!(look(None, read_at + READING_FRESH_MS + 1), stale);
    // A newer reading of the session's converter counts from when it was read; one of another converter does not count.
    assert_eq!(look(Some(&newer), read_at + READING_FRESH_MS + 1), passing);
    let other = Reading {
        converter: "04270010".into(),
        ..newer
    };
    assert_eq!(look(Some(&other), read_at + READING_FRESH_MS + 1), stale);
}

#[test]
fn a_re_arm_publishes_once_it_is_done_not_while_the_old_converter_is_gone() {
    let mut rig = Rig::audible();
    let (tail, converter) = ("0424000f", "04270010");
    rig.host.with(|s| {
        s.dirs.clear();
        s.dirs.insert(format!("{INSTANCES}/{converter}"));
        s.chain_graph = graph_with(tail, converter);
        s.luna_ms = 400;
    });
    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = published.clone();
    rig.route.observe_with(move |snap| {
        sink.lock()
            .unwrap()
            .push(snap.session.as_ref().map(|s| s.bypassing));
    });
    rig.route.arm(PID, tail, ADEC, &rig.env).unwrap();
    // The old converter reads as gone while LG's graph is rebuilt: a reading taken mid-way would say the route is not bypassing.
    assert_eq!(*published.lock().unwrap(), vec![Some(true)]);
}

#[test]
fn a_task_in_the_control_lock_for_a_moment_does_not_end_the_session() {
    let mut rig = Rig::audible();
    rig.host.with(|s| {
        s.stuck.push(Stuck {
            comm: "extinput-integration".into(),
            wchan: "snd_ctl_elem_info".into(),
            stack: String::new(),
        });
        s.stuck_scans = Some(1);
    });
    rig.run(2_500);
    assert_eq!(rig.snap().end, None);
    assert!(rig.session().audible);
}

#[test]
fn a_helper_that_is_stopping_restores_in_about_a_second_while_the_player_still_streams() {
    let mut rig = Rig::new();
    rig.host.with(|s| s.adec_mute = false); // the input was found unmuted
    rig.arm().unwrap();
    rig.run(10_000);
    assert_eq!(rig.session().phase, Phase::Audible);
    rig.host.with(|s| {
        s.luna_ms = 200;
        s.log.clear();
    });
    rig.env.stopping = true;
    let started = rig.host.now();
    rig.route.tick(&rig.env);
    let took = rig.host.now() - started;
    assert!(
        took <= 1_200,
        "the restore took {took} ms of the helper's 4 s"
    );
    assert_eq!(rig.snap().end.map(|e| e.why), Some(Why::HelperExit));
    assert_eq!(rig.snap().pending, None);
    assert_words_restored(&rig);
    rig.host.with(|s| {
        assert!(!s.adec_connected);
        assert!(
            s.adec_mute,
            "an input found unmuted stays muted: the player has not closed its renderer"
        );
    });
}

#[test]
fn a_restore_cut_off_at_any_step_is_finished_by_the_next_helper() {
    for cut in ["luna mute true", "luna disconnect", "words 2,1,0,0,1"] {
        let mut rig = Rig::audible();
        rig.host.with(|s| {
            s.log.clear();
            s.trip = Some((cut.to_owned(), Fault::Dead));
        });
        // The helper is stopped and dies right after `cut`.
        rig.env.stopping = true;
        rig.route.tick(&rig.env);
        assert!(
            rig.host.with(|s| s.files.contains_key(JOURNAL)),
            "{cut}: the journal is what the next helper works from"
        );
        // The next helper starts on the same TV.
        rig.host.with(|s| s.fault = None);
        let mut next = Route::new(rig.host.clone(), JOURNAL);
        let note = next.recover().unwrap();
        assert!(
            note.contains("put LG's audio settings back"),
            "{cut}: {note}"
        );
        let log = rig.host.log();
        assert_eq!(rig.count("luna disconnect"), 1, "{cut}: {log:?}");
        assert_eq!(rig.count("words 2,1,0,0,1"), 1, "{cut}: {log:?}");
        assert_words_restored(&rig);
        rig.host
            .with(|s| assert!(!s.adec_connected && s.adec_mute, "{cut}"));
    }
}

#[test]
fn a_blocked_proc_read_mid_restore_leaves_the_journal_and_a_later_tick_finishes() {
    let mut rig = Rig::audible();
    rig.host.with(|s| {
        s.log.clear();
        s.trip = Some(("luna mute true".to_owned(), Fault::Blocked));
    });
    assert_eq!(rig.route.release(PID), Ok(true));
    assert!(rig.snap().pending.is_some());
    assert!(rig.host.with(|s| s.files.contains_key(JOURNAL)));
    assert_eq!(
        rig.count("words 2,1,0,0,1"),
        0,
        "the words are not written blind"
    );
    rig.host.with(|s| s.fault = None);
    rig.run(2_100);
    assert_eq!(rig.snap().pending, None);
    assert_eq!(rig.count("luna disconnect"), 1);
    assert_words_restored(&rig);
}

#[test]
fn audiooutputd_going_silent_mid_restore_holds_the_words_until_it_answers() {
    let mut rig = Rig::audible();
    rig.host.with(|s| {
        s.log.clear();
        s.trip = Some(("luna mute true".to_owned(), Fault::Silent));
    });
    assert_eq!(rig.route.release(PID), Ok(true));
    assert!(rig.snap().pending.is_some());
    assert_eq!(
        rig.count("luna disconnect"),
        0,
        "it never reached audiooutputd"
    );
    assert_eq!(rig.count("words 2,1,0,0,1"), 0);
    rig.host.with(|s| s.fault = None);
    rig.run(2_100);
    assert_eq!(rig.snap().pending, None);
    assert_words_restored(&rig);
    assert!(!rig.host.with(|s| s.adec_connected));
}

// ---- LG's mixer input for the player's stream ----------------------------------------------------------------------------

/// A TV on which LG routed the player's Load through its mixer input, as the C4 does before the player arms.
fn lg_mixing() -> Rig {
    let rig = Rig::new();
    rig.host.lg_mixer();
    rig
}

fn lg_mixer_connected(rig: &Rig) -> bool {
    rig.host.with(|sim| sim.mixer == Some(PORT))
}

#[test]
fn lgs_mixer_for_the_players_own_stream_is_taken_over_at_the_arm_and_given_back_while_it_plays() {
    let mut rig = lg_mixing();
    assert_eq!(rig.arm(), Ok(CONVERTER.to_owned()));
    assert_eq!(
        rig.host.log(),
        [
            "save",
            "luna disconnect AMIXER4",
            "cmd 0427000d",
            "words 2,1,0,0,0"
        ]
    );
    let taken = Taken {
        port: PORT,
        pipeline: PIPELINE.into(),
    };
    rig.host.with(|sim| {
        assert_eq!(
            Journal::parse(&sim.files[JOURNAL]).unwrap().mixer,
            Some(taken)
        );
    });
    assert!(!lg_mixer_connected(&rig));
    // LG's mixer input stays the main input after its disconnect, so Pass-Through needs the ADEC: it connects straight after the
    // un-mix, and the mute pair waits for Pass-Through.
    assert!(!rig.session().bypassing);
    rig.run(200);
    assert_eq!(rig.session().phase, Phase::Connected);
    assert_eq!(rig.host.log()[4..], ["save", "luna connect"]);
    assert!(rig.session().bypassing);
    rig.run(10_000);
    assert_eq!(rig.session().phase, Phase::Audible);
    assert_eq!(rig.session().mixer, Some(PORT));
    assert_eq!(rig.route.release(PID), Ok(true));
    assert_restored(&rig, Why::Released);
    let log = rig.host.log();
    assert!(
        last(&log, "words 2,1,0,0,1") < first(&log, "luna connect AMIXER4"),
        "{log:?}"
    );
    assert!(lg_mixer_connected(&rig));
}

#[test]
fn lg_connecting_its_mixer_mid_session_is_taken_over_again_and_its_words_are_the_ones_put_back() {
    let mut rig = Rig::new();
    rig.host.with(|sim| sim.words.codec = 2);
    rig.arm().unwrap();
    rig.run(10_000);
    let mark = rig.host.log().len();
    rig.host.lg_mixer();
    // audiooutputd lists the input a moment after the kernel shows it.
    rig.host.with(|sim| {
        sim.words.codec = 1;
        sim.listed_at = sim.now + 300;
    });
    rig.run(500);
    assert_eq!(
        rig.host.log()[mark..],
        ["save", "luna disconnect AMIXER4", "save", "words 2,1,0,0,0"]
    );
    assert_eq!(rig.session().phase, Phase::Audible);
    assert_eq!(rig.session().mixer, Some(PORT));
    assert_eq!(rig.route.release(PID), Ok(true));
    assert_restored(&rig, Why::Released);
    assert!(lg_mixer_connected(&rig));
}

#[test]
fn lg_fighting_over_its_mixer_ends_the_session_and_keeps_it() {
    let mut rig = lg_mixing();
    rig.arm().unwrap();
    for _ in 1..FIGHTS {
        rig.host.lg_mixer();
        rig.run(500);
        assert!(rig.snap().session.is_some());
    }
    rig.host.lg_mixer();
    rig.run(500);
    let end = rig.snap().end.unwrap();
    assert_eq!(end.why, Why::RouteLost);
    assert!(
        end.detail.contains("LG keeps connecting AMIXER4"),
        "{}",
        end.detail
    );
    assert_eq!(rig.count("luna disconnect AMIXER4"), FIGHTS);
    assert!(lg_mixer_connected(&rig));
}

#[test]
fn a_stream_that_has_ended_leaves_lgs_mixer_disconnected_as_lgs_own_stop_does() {
    type Ending = (fn(&mut Rig), Why);
    let endings: [Ending; 2] = [
        // The player stops its stream, then lets the route go.
        (
            |rig| {
                rig.host.with(|sim| sim.pipelines.clear());
                assert_eq!(rig.route.release(PID), Ok(true));
            },
            Why::Released,
        ),
        // The player dies; uMS may still list its pipeline.
        (
            |rig| {
                rig.host.with(|sim| sim.procs.remove(&PID));
                rig.run(300);
            },
            Why::OwnerExited,
        ),
    ];
    for (end, why) in endings {
        let mut rig = lg_mixing();
        rig.arm().unwrap();
        rig.run(10_000);
        end(&mut rig);
        assert_restored(&rig, why);
        assert_eq!(rig.count("luna connect AMIXER4"), 0, "{why:?}");
        assert!(!lg_mixer_connected(&rig));
    }
}

#[test]
fn switching_the_plugin_off_gives_lgs_mixer_back() {
    let mut rig = lg_mixing();
    rig.arm().unwrap();
    rig.run(10_000);
    rig.env.enabled = false;
    rig.run(300);
    assert_restored(&rig, Why::Disabled);
    assert!(lg_mixer_connected(&rig));
}

#[test]
fn the_next_helper_gives_lgs_mixer_back_from_the_journal() {
    let mut rig = lg_mixing();
    rig.arm().unwrap();
    rig.run(10_000);
    // The helper dies here; a new one starts on the same TV.
    let mut next = Route::new(rig.host.clone(), JOURNAL);
    next.recover().unwrap();
    assert_adec_restored(&rig);
    assert_words_restored(&rig);
    assert!(lg_mixer_connected(&rig));
}

#[test]
fn an_arm_that_fails_after_the_takeover_gives_lgs_mixer_back() {
    let mut rig = lg_mixing();
    rig.host.with(|sim| sim.cmd_ok = false);
    let refusal = rig.arm().unwrap_err();
    assert!(
        refusal.reason.contains("cannot configure converter"),
        "{}",
        refusal.reason
    );
    assert_eq!(
        rig.host.log(),
        [
            "save",
            "luna disconnect AMIXER4",
            "luna connect AMIXER4",
            "remove"
        ]
    );
    assert!(lg_mixer_connected(&rig));
    assert_eq!(rig.snap().session, None);
}

#[test]
fn a_session_leaves_the_decoders_gain_and_mute_as_it_found_them_and_names_a_gain_lg_changed() {
    // The route never writes a gain: a decoder found at half gain, through a takeover of LG's mixer input, reads back as the
    // journal found it, Gain and Mute included.
    let mut rig = lg_mixing();
    rig.host.with(|sim| sim.adec_gain = "0x400000".into());
    rig.arm().unwrap();
    let found = rig
        .host
        .with(|sim| Journal::parse(&sim.files[JOURNAL]).unwrap().kernel);
    assert!(found.contains("Gain=0x400000"), "{found}");
    rig.run(10_000);
    assert_eq!(rig.session().phase, Phase::Audible);
    assert_eq!(rig.route.release(PID), Ok(true));
    assert_restored(&rig, Why::Released);
    rig.host.with(|sim| assert_eq!(sim.adec_gain, "0x400000"));

    // A gain LG changed while the session held the decoder is named in the comparison after the restore.
    let mut rig = Rig::audible();
    rig.host.with(|sim| sim.adec_gain = "0x200000".into());
    assert_eq!(rig.route.release(PID), Ok(true));
    let after = rig.snap().after.expect("the difference is reported");
    assert!(
        after.contains("Gain=0x200000") && after.contains("Gain=0x800000"),
        "{after}"
    );
}
