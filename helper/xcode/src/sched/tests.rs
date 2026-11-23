use super::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn spec(priority: Priority, wait: Wait, units: bool) -> Spec {
    Spec { owner: "test".to_owned(), priority, wait, units }
}

/// A viewer: answers at once, fed by its owner.
fn live() -> Spec {
    spec(Priority::Live, Wait::Fail, true)
}

fn replay() -> Spec {
    spec(Priority::Replay, Wait::Queue, true)
}

/// A core on a clock the test sets, which also checks the rule everything rests on: one child at a time.
struct Rig {
    core: Core,
    alive: Vec<u64>,
    ticket: u64,
}

impl Rig {
    fn new() -> Self {
        Self::with(Limits::default())
    }

    fn with(lim: Limits) -> Self {
        Self { core: Core::new(lim), alive: Vec::new(), ticket: 0 }
    }

    fn at(&mut self, t: u64, input: Input) -> Vec<Action> {
        if let Input::Exited { epoch, .. } | Input::SpawnFailed { epoch, .. } = &input {
            self.alive.retain(|e| e != epoch);
        }
        let actions = self.core.step(ms(t), input);
        for action in &actions {
            match action {
                Action::Spawn { epoch, .. } => {
                    assert!(self.alive.is_empty(), "child {epoch} spawned while {:?} is alive", self.alive);
                    self.alive.push(*epoch);
                }
                Action::Park { epoch } => self.alive.retain(|e| e != epoch),
                _ => {}
            }
        }
        actions
    }

    fn start(&mut self, t: u64, spec: Spec) -> Vec<Action> {
        self.ticket += 1;
        self.at(t, Input::Start { ticket: self.ticket, spec })
    }

    fn ready(&mut self, t: u64, epoch: u64) -> Vec<Action> {
        self.at(t, Input::Ready { epoch })
    }

    fn exited(&mut self, t: u64, epoch: u64, code: i32) -> Vec<Action> {
        self.at(t, Input::Exited { epoch, exit: Exit::Code(code), last_line: None })
    }

    fn tick(&mut self, t: u64) -> Vec<Action> {
        self.at(t, Input::Tick)
    }
}

fn spawned(actions: &[Action]) -> Option<(u64, JobId, bool)> {
    actions.iter().find_map(|a| match a {
        Action::Spawn { epoch, job, resume } => Some((*epoch, *job, *resume)),
        _ => None,
    })
}

fn ended_with(actions: &[Action], job: JobId) -> Option<Reason> {
    actions.iter().find_map(|a| match a {
        Action::Emit(Event::Ended { job: j, reason }) if *j == job => Some(reason.clone()),
        _ => None,
    })
}

fn holds(rig: &mut Rig, t: u64, why: &str) -> Vec<Action> {
    rig.at(t, Input::Hold { key: "app".to_owned(), reason: Some(why.to_owned()) })
}

#[test]
fn a_start_spawns_a_child_and_the_job_runs_when_it_reports_ready() {
    let mut rig = Rig::new();
    assert_eq!(rig.start(0, live()), [Action::Accepted { ticket: 1, job: 1 }, Action::Spawn { epoch: 1, job: 1, resume: false }]);
    assert_eq!(
        rig.ready(40, 1),
        [Action::State { job: 1, state: JobState::Running }, Action::Emit(Event::Started { job: 1, restarts: 0, setup_ms: 40 })]
    );
    assert_eq!(rig.core.snapshot().state, EngineState::Running);
}

#[test]
fn a_viewer_replaces_a_viewer_and_the_new_child_waits_for_the_old_one_to_be_gone() {
    let mut rig = Rig::new();
    rig.start(0, live());
    rig.ready(40, 1);
    let a = rig.start(1000, live());
    assert_eq!(ended_with(&a, 1), Some(Reason::Replaced));
    assert!(a.contains(&Action::Term { epoch: 1 }) && a.contains(&Action::Accepted { ticket: 2, job: 2 }));
    assert_eq!(spawned(&a), None, "the hardware is still the old child's");
    let a = rig.exited(1030, 1, 0);
    assert_eq!(spawned(&a), Some((2, 2, false)));
    assert_eq!(rig.core.snapshot().last_stop_ms, Some(30));

    // ended jobs are kept for the status page, a few of them
    for n in 0..6 {
        let t = 2000 + n * 100;
        rig.ready(t - 50, 2 + n);
        rig.start(t, live());
        rig.exited(t + 20, 2 + n, 0);
    }
    assert_eq!(rig.core.snapshot().jobs.len(), 1 + KEEP_ENDED);
}

#[test]
fn a_higher_priority_job_takes_the_hardware_and_the_lower_one_resumes_after_it() {
    let mut rig = Rig::new();
    rig.start(0, replay());
    rig.ready(50, 1);
    let a = rig.start(1000, live());
    assert!(a.contains(&Action::Term { epoch: 1 }));
    assert!(a.contains(&Action::State { job: 1, state: JobState::Paused(Reason::Preempted { by: 2 }) }));
    assert!(!a.iter().any(|x| matches!(x, Action::Emit(Event::Paused { .. }))), "the pause is reported once the hardware is free");
    let a = rig.exited(1020, 1, 0);
    assert!(a.contains(&Action::Emit(Event::Paused { job: 1, reason: Reason::Preempted { by: 2 }, stopped_ms: 20 })));
    assert_eq!(spawned(&a), Some((2, 2, false)));
    rig.ready(1100, 2);

    rig.at(5000, Input::Stop { job: 2 });
    let a = rig.exited(5010, 2, 0);
    assert_eq!(spawned(&a), Some((3, 1, true)), "the replay job runs again, and says it has run before");
    let a = rig.ready(5060, 3);
    assert!(a.contains(&Action::Emit(Event::Resumed { job: 1 })));
}

#[test]
fn a_start_that_cannot_wait_is_refused_while_the_gate_is_closed_and_one_that_can_waits_for_it() {
    let mut rig = Rig::new();
    holds(&mut rig, 0, "netflix is in front");
    assert_eq!(rig.start(1, live()), [Action::Refused { ticket: 1, error: StartError::Held("app: netflix is in front".to_owned()) }]);
    let a = rig.start(2, replay());
    assert!(a.contains(&Action::Accepted { ticket: 2, job: 1 }));
    assert!(a.contains(&Action::State { job: 1, state: JobState::Paused(Reason::Held("app: netflix is in front".to_owned())) }));
    assert_eq!(spawned(&a), None);
    assert_eq!(rig.core.snapshot().state, EngineState::Held);

    let a = rig.at(3000, Input::Hold { key: "app".to_owned(), reason: None });
    assert!(a.contains(&Action::Emit(Event::Hold { key: "app".to_owned(), reason: None })));
    assert_eq!(spawned(&a), Some((1, 1, false)));
}

#[test]
fn closing_the_gate_ends_a_viewer_and_pauses_a_job_that_can_wait_and_frees_the_hardware_once() {
    let mut rig = Rig::new();
    rig.start(0, live());
    rig.ready(40, 1);
    let a = holds(&mut rig, 1000, "netflix is in front");
    assert_eq!(ended_with(&a, 1), Some(Reason::Held("app: netflix is in front".to_owned())));
    assert!(a.contains(&Action::Detach { epoch: 1, job: 1 }) && a.contains(&Action::Term { epoch: 1 }));
    rig.exited(1035, 1, 0);
    assert_eq!(rig.core.snapshot().last_stop_ms, Some(35));

    let mut rig = Rig::new();
    rig.start(0, replay());
    rig.ready(40, 1);
    let a = holds(&mut rig, 1000, "netflix is in front");
    assert_eq!(ended_with(&a, 1), None);
    assert!(a.contains(&Action::Term { epoch: 1 }));
    let a = rig.exited(1040, 1, 0);
    let held = Reason::Held("app: netflix is in front".to_owned());
    assert!(a.contains(&Action::Emit(Event::Paused { job: 1, reason: held, stopped_ms: 40 })));
}

#[test]
fn a_child_that_ignores_the_request_is_killed_and_one_that_ignores_that_wedges_the_hardware() {
    let mut rig = Rig::new();
    rig.start(0, replay());
    rig.ready(40, 1);
    holds(&mut rig, 1000, "x");
    assert_eq!(rig.core.next_deadline(), Some(ms(1300)));
    assert!(rig.tick(1299).is_empty());
    assert_eq!(rig.tick(1300), [Action::Kill { epoch: 1 }]);
    assert_eq!(rig.core.next_deadline(), Some(ms(2000)));
    assert!(rig.tick(1999).is_empty());
    let a = rig.tick(2000);
    assert!(a.contains(&Action::Park { epoch: 1 }) && a.contains(&Action::Emit(Event::Wedged)));
    assert_eq!(ended_with(&a, 1), Some(Reason::Fault(Fault::Wedged("the converter did not exit after SIGKILL".to_owned()))));
    assert_eq!(rig.core.snapshot().state, EngineState::Wedged);
    assert!(matches!(rig.start(3000, live()).as_slice(), [Action::Refused { error: StartError::Wedged(_), .. }]));
}

#[test]
fn a_child_that_reports_a_hung_call_wedges_the_hardware_whoever_asked_it_to_stop() {
    let mut rig = Rig::new();
    rig.start(0, replay());
    rig.ready(40, 1);
    holds(&mut rig, 1000, "x");
    let a = rig.exited(1100, 1, proto::EXIT_WEDGED);
    assert!(a.contains(&Action::Emit(Event::Wedged)));
}

#[test]
fn a_failed_child_ends_a_job_its_owner_feeds_and_the_exit_code_says_why() {
    let mut rig = Rig::new();
    rig.start(0, live());
    rig.ready(40, 1);
    rig.at(500, Input::ChildError { epoch: 1, status: 4, text: "no decoder".to_owned() });
    let a = rig.exited(520, 1, proto::EXIT_DECODER);
    assert_eq!(ended_with(&a, 1), Some(Reason::Fault(Fault::Failed("the video decoder failed (no decoder)".to_owned()))));
    assert!(a.contains(&Action::Detach { epoch: 1, job: 1 }));

    // the shape of a stream is not the converter's fault
    rig.start(1000, live());
    rig.ready(1040, 2);
    let a = rig.exited(1100, 2, proto::EXIT_SHAPE);
    assert!(matches!(ended_with(&a, 2), Some(Reason::Fault(Fault::Unsupported(_)))));
}

#[test]
fn a_job_the_engine_owns_restarts_with_a_delay_that_doubles_and_starts_over() {
    let mut rig = Rig::new();
    rig.start(0, spec(Priority::Recording, Wait::Queue, false));
    rig.ready(50, 1);
    let a = rig.exited(1000, 1, proto::EXIT_ENCODER);
    assert!(a.contains(&Action::State { job: 1, state: JobState::Paused(Reason::Restarting { fault: "the video encoder failed".to_owned() }) }));
    assert_eq!(rig.core.next_deadline(), Some(ms(1250)));
    assert_eq!(spawned(&rig.tick(1250)), Some((2, 1, true)));
    // it never became ready: the next delay is twice the last
    rig.exited(1300, 2, proto::EXIT_ENCODER);
    assert_eq!(rig.core.next_deadline(), Some(ms(1800)));
    assert_eq!(spawned(&rig.tick(1800)), Some((3, 1, true)));
    // it ran for longer than the longest delay: the delay starts over
    rig.ready(1850, 3);
    rig.exited(20_000, 3, proto::EXIT_ENCODER);
    assert_eq!(rig.core.next_deadline(), Some(ms(20_250)));
    assert_eq!(rig.core.snapshot().jobs[0].restarts, 3);
}

#[test]
fn four_failures_in_a_minute_rest_the_converter_for_half_a_minute_but_a_stream_it_cannot_take_does_not_count() {
    let mut rig = Rig::new();
    let mut t = 0;
    for epoch in 1..=4 {
        rig.start(t, live());
        rig.ready(t + 40, epoch);
        let a = rig.exited(t + 500, epoch, proto::EXIT_ENCODER);
        t += 5000;
        if epoch == 4 {
            let Some(Reason::Fault(fault)) = ended_with(&a, epoch) else { panic!("job {epoch} did not end with a fault") };
            assert!(fault.message().ends_with("(failed 4 times in 60 s)"), "{fault}");
        }
    }
    assert_eq!(rig.core.snapshot().state, EngineState::Cooling);
    let StartError::Cooling(left) = (match rig.start(t, live()).as_slice() {
        [Action::Refused { error, .. }] => error.clone(),
        other => panic!("{other:?}"),
    }) else {
        panic!("not cooling");
    };
    assert!(left > Duration::from_secs(10) && left <= Duration::from_secs(30), "{left:?}");
    // a viewer that waits for nothing is not cut short by it: only live starts are refused, and only until it is over
    let a = rig.start(t + 30_000, live());
    assert_eq!(spawned(&a), Some((5, 5, false)));

    let mut rig = Rig::new();
    for epoch in 1..=6 {
        let t = epoch * 1000;
        rig.start(t, live());
        rig.ready(t + 10, epoch);
        rig.exited(t + 20, epoch, proto::EXIT_SHAPE);
    }
    assert_eq!(rig.core.snapshot().state, EngineState::Idle);
}

#[test]
fn a_child_that_is_not_ready_in_time_or_goes_silent_is_failed() {
    let mut rig = Rig::new();
    rig.start(0, live());
    assert!(rig.tick(1499).is_empty());
    let a = rig.tick(1500);
    assert_eq!(ended_with(&a, 1), Some(Reason::Fault(Fault::Failed("the converter was not ready after 1500 ms".to_owned()))));
    assert!(a.contains(&Action::Term { epoch: 1 }));
    rig.exited(1510, 1, 0);

    rig.start(2000, live());
    rig.ready(2040, 2);
    // what the child sends moves the deadline
    rig.at(2000 + 3000, Input::Heard { epoch: 2, at: ms(5000) });
    assert!(rig.tick(8999).is_empty());
    let a = rig.tick(9000);
    assert_eq!(ended_with(&a, 2), Some(Reason::Fault(Fault::Stalled("the converter sent nothing for 4000 ms".to_owned()))));
}

#[test]
fn a_child_whose_output_closed_gets_the_grace_to_say_why_it_ended() {
    let mut rig = Rig::new();
    rig.start(0, live());
    rig.ready(40, 1);
    rig.at(2000, Input::OutClosed { epoch: 1 });
    assert_eq!(rig.core.next_deadline(), Some(ms(2300)));
    let a = rig.exited(2100, 1, proto::EXIT_ENCODER);
    assert_eq!(ended_with(&a, 1), Some(Reason::Fault(Fault::Failed("the video encoder failed".to_owned()))));

    rig.start(3000, live());
    rig.ready(3040, 2);
    rig.at(4000, Input::OutClosed { epoch: 2 });
    assert!(rig.tick(4299).is_empty());
    let a = rig.tick(4300);
    assert_eq!(ended_with(&a, 2), Some(Reason::Fault(Fault::Failed("the converter closed its output".to_owned()))));
}

#[test]
fn a_protocol_error_and_a_child_that_cannot_start_end_the_job() {
    let mut rig = Rig::new();
    rig.start(0, live());
    rig.ready(40, 1);
    let a = rig.at(100, Input::Protocol { epoch: 1, why: "a second ready record".to_owned() });
    assert_eq!(ended_with(&a, 1), Some(Reason::Fault(Fault::Failed("protocol error: a second ready record".to_owned()))));
    assert!(a.contains(&Action::Term { epoch: 1 }));
    rig.exited(120, 1, 0);

    rig.start(1000, live());
    let a = rig.at(1001, Input::SpawnFailed { epoch: 2, why: "No such file".to_owned() });
    assert_eq!(ended_with(&a, 2), Some(Reason::Fault(Fault::Failed("the converter did not start: No such file".to_owned()))));
}

#[test]
fn a_recording_waits_behind_a_viewer_and_two_recordings_do_not_run_together() {
    let mut rig = Rig::new();
    rig.start(0, live());
    rig.ready(40, 1);
    let recording = |wait| spec(Priority::Recording, wait, true);
    assert!(matches!(rig.start(100, recording(Wait::Fail)).as_slice(), [Action::Refused { error: StartError::Busy(_), .. }]));
    let a = rig.start(200, recording(Wait::Queue));
    assert!(a.contains(&Action::State { job: 2, state: JobState::Paused(Reason::Preempted { by: 1 }) }));
    rig.at(300, Input::Stop { job: 1 });
    rig.exited(320, 1, 0);
    assert!(matches!(rig.start(400, recording(Wait::Fail)).as_slice(), [Action::Refused { error: StartError::Busy(_), .. }]));
}

#[test]
fn the_number_of_jobs_is_bounded_and_a_replacement_does_not_count_against_it() {
    let mut rig = Rig::with(Limits { max_jobs: 2, ..Limits::default() });
    rig.start(0, replay());
    rig.start(1, spec(Priority::Recording, Wait::Queue, true));
    assert!(matches!(rig.start(2, spec(Priority::Recording, Wait::Queue, true)).as_slice(), [Action::Refused { error: StartError::TooMany, .. }]));
    let a = rig.start(3, replay());
    assert_eq!(ended_with(&a, 1), Some(Reason::Replaced));
}

#[test]
fn shutting_down_ends_every_job_and_is_finished_when_the_child_is_gone() {
    let mut rig = Rig::new();
    rig.start(0, live());
    rig.ready(40, 1);
    let a = rig.at(100, Input::Shutdown);
    assert_eq!(ended_with(&a, 1), Some(Reason::Shutdown));
    assert!(a.contains(&Action::Term { epoch: 1 }));
    assert!(!rig.core.finished());
    rig.exited(130, 1, 0);
    assert!(rig.core.finished());
    assert!(matches!(rig.start(200, live()).as_slice(), [Action::Refused { error: StartError::Setup(_), .. }]));
}

#[test]
fn late_reports_of_a_child_that_is_gone_are_ignored() {
    let mut rig = Rig::new();
    rig.start(0, live());
    rig.ready(40, 1);
    rig.at(100, Input::Stop { job: 1 });
    rig.exited(120, 1, 0);
    for input in [Input::Ready { epoch: 1 }, Input::OutClosed { epoch: 1 }, Input::Heard { epoch: 1, at: ms(150) }, Input::Exited { epoch: 1, exit: Exit::Code(6), last_line: None }] {
        assert!(rig.at(200, input).is_empty());
    }
}

#[test]
fn a_unit_is_taken_only_while_the_job_runs_and_the_first_after_a_start_is_a_key_unit() {
    let running = JobState::Running;
    assert_eq!(admit(&running, true, false), Err(PushError::WantKey));
    assert_eq!(admit(&running, true, true), Ok(()));
    assert_eq!(admit(&running, false, false), Ok(()));
    assert_eq!(admit(&JobState::Queued, false, true), Err(PushError::Paused(Reason::Queued)));
    assert_eq!(admit(&JobState::Paused(Reason::Replaced), false, true), Err(PushError::Paused(Reason::Replaced)));
    assert_eq!(admit(&JobState::Ended(Reason::Stopped), false, true), Err(PushError::Ended(Reason::Stopped)));
}

#[test]
fn a_child_exit_is_a_sentence_and_the_childs_own_words_follow_it() {
    assert!(matches!(fault_for(Exit::Code(proto::EXIT_CONFIG), None), Fault::Unsupported(_)));
    assert!(matches!(fault_for(Exit::Code(proto::EXIT_STALLED), None), Fault::Stalled(_)));
    assert!(matches!(fault_for(Exit::Code(proto::EXIT_WEDGED), None), Fault::Wedged(_)));
    assert_eq!(fault_for(Exit::Code(9), Some("boom")), Fault::Failed("the converter exited with status 9 (boom)".to_owned()));
    assert_eq!(fault_for(Exit::Signal(11), Some("")), Fault::Failed("the converter exited with signal 11".to_owned()));
    assert_eq!(fault_for(Exit::Code(0), None), Fault::Failed("the converter ended by itself".to_owned()));
}
