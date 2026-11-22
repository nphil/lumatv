//! The pieces as the app joins them: two engines list the same game, Play asks once, the session runs and is recorded, and the next
//! Play goes back to where the game last ran without ever moving to the other target by itself.

use lumagame_core::catalog::{self, Listing};
use lumagame_core::engine::{Engine, Presentation};
use lumagame_core::history::Played;
use lumagame_core::ids::{EngineId, SessionId, SourceKey, TargetId};
use lumagame_core::ipc::{self, EndReason, Event, FromEngine, LibraryEntry, Link, Reach, Request, SessionState, Target, ToEngine};
use lumagame_core::launch::{self, Candidate, ChooseWhy, Resolution, Why};
use lumagame_core::session::{Effect, Input, Outcome, Session};
use lumagame_core::state::{OpenSession, State};
use std::collections::{BTreeMap, VecDeque};

/// An engine with a script for a far side. Every message crosses as its line, as it would to an engine in a process of its own.
struct Scripted {
    target: Target,
    library: Vec<LibraryEntry>,
    lines_in: Vec<String>,
    lines_out: VecDeque<String>,
}

impl Scripted {
    fn new(target: &str, name: &str, library: &str) -> Scripted {
        let target = Target { id: TargetId::new(target).unwrap(), name: name.into(), detail: None, link: Link::Linked, reach: Reach::Online };
        Scripted { target, library: serde_json::from_str(library).unwrap(), lines_in: Vec::new(), lines_out: VecDeque::new() }
    }

    fn told(&mut self, seq: u64, event: Event) {
        self.lines_out.push_back(ipc::encode(&FromEngine::answer(seq, event)));
    }
}

impl Engine for Scripted {
    fn send(&mut self, message: ToEngine) {
        let line = ipc::encode(&message);
        let ToEngine { seq, request } = ipc::decode(&line).unwrap();
        self.lines_in.push(line);
        match request {
            Request::Targets => self.told(seq, Event::Targets { targets: vec![self.target.clone()] }),
            Request::Library { target, .. } => self.told(seq, Event::Library { target, entries: self.library.clone(), next: None }),
            Request::Launch { session, .. } => {
                self.told(seq, Event::Session { session, state: SessionState::Starting });
                self.told(seq, Event::Session { session, state: SessionState::Running { stream: None } });
            }
            Request::Stop { session, .. } => self.told(seq, Event::Session { session, state: SessionState::Ended { reason: EndReason::Stopped } }),
            _ => {}
        }
    }

    fn poll(&mut self) -> Option<FromEngine> {
        self.lines_out.pop_front().map(|line| ipc::decode(&line).unwrap())
    }
}

type Targets = BTreeMap<(EngineId, TargetId), Target>;

fn candidates(game: &catalog::Game, targets: &Targets) -> Vec<Candidate> {
    game.sources.iter().map(|s| Candidate::of(s, targets.get(&(s.engine.clone(), s.target.clone())))).collect()
}

fn build(state: &mut State, listing: &Listing) -> Vec<catalog::Game> {
    let sources = listing.enabled(&state.enabled_engines());
    let (games, merges) = catalog::build(&mut state.identity, sources);
    state.apply_merges(&merges);
    games
}

#[test]
fn play_asks_once_then_returns_to_the_last_target_and_never_switches_by_itself() {
    let (gfn, punktfunk) = (EngineId::new("gfn").unwrap(), EngineId::new("punktfunk").unwrap());
    let cloud_library = r#"[{"id":"100","title":"ELDEN RING","external":["gfn:100","steam:1245620"],"store":"steam","readiness":"ready"}]"#;
    let pc_library = r#"[{"id":"steam:1245620","title":"Elden Ring","external":["steam:1245620"],"store":"steam","readiness":"ready"}]"#;
    let mut engines = vec![(gfn.clone(), Scripted::new("acct", "Nitin", cloud_library)), (punktfunk.clone(), Scripted::new("pc1", "Gaming PC", pc_library))];

    let mut state = State::default();
    for id in [&gfn, &punktfunk] {
        state.engines.entry(id.clone()).or_default().enabled = true;
    }

    // Each engine names its target and lists its library; the two entries share a Steam id and become one card.
    let mut listing = Listing::default();
    let mut targets = Targets::new();
    for (id, engine) in &mut engines {
        engine.send(ToEngine { seq: 1, request: Request::Targets });
        engine.send(ToEngine { seq: 2, request: Request::Library { target: engine.target.id.clone(), cursor: None } });
        while let Some(FromEngine { event, .. }) = engine.poll() {
            match event {
                Event::Targets { targets: told } => targets.extend(told.into_iter().map(|t| ((id.clone(), t.id.clone()), t))),
                Event::Library { target, entries, .. } => listing.replace(id, &target, entries),
                _ => {}
            }
        }
    }
    let games = build(&mut state, &listing);
    assert_eq!(games.len(), 1);
    let game = games[0].clone();
    assert_eq!((game.title.as_str(), game.sources.len()), ("ELDEN RING", 2));

    // Nothing is remembered and both are ready: Play asks.
    let asked = launch::resolve(&candidates(&game, &targets), state.profile().user.remembered(game.id));
    assert_eq!(asked, Resolution::Choose { why: ChooseWhy::Several, focus: None });

    // The owner picks the PC. The session's effects are carried out against the engine; its events come back as inputs.
    let pc: SourceKey = "punktfunk|pc1|steam:1245620".parse().unwrap();
    let engine = &mut engines[1].1;
    let (mut session, mut effects) = Session::start(SessionId(1), game.id, pc.clone(), Presentation::Feeder, false, 0);
    let mut low_latency = Vec::new();
    let mut open: Option<OpenSession> = None;
    let mut finished = None;
    let mut now_ms = 0;
    let mut owner_leaves_at = Some(599_000);
    while finished.is_none() {
        for effect in std::mem::take(&mut effects) {
            match effect {
                Effect::Launch => {
                    engine.send(ToEngine { seq: 10, request: Request::Launch { session: session.id, target: pc.target.clone(), game: pc.id.clone() } })
                }
                Effect::Stop { end_game } => engine.send(ToEngine { seq: 11, request: Request::Stop { session: session.id, end_game } }),
                Effect::LowLatency(on) => low_latency.push(on),
                Effect::Started { pin } => {
                    state.profile_mut().user.started(session.game, &session.source, pin);
                    let played = Played { game: session.game, source: session.source.clone(), start: 1_700_000_000, seconds: 0 };
                    open = Some(OpenSession { profile: state.active.clone(), played });
                }
                Effect::Checkpoint { seconds } => open.as_mut().unwrap().played.seconds = seconds,
                Effect::EndEngine => panic!("the engine answered"),
                Effect::Finished { played, outcome } => finished = Some((played, outcome)),
            }
        }
        if let Some(FromEngine { event: Event::Session { state, .. }, .. }) = engine.poll() {
            effects = session.step(now_ms, Input::Engine(state));
        } else if owner_leaves_at.is_some_and(|at| now_ms >= at) {
            owner_leaves_at = None;
            effects = session.step(now_ms, Input::Stop { end_game: false });
        } else {
            now_ms += 1_000;
            effects = session.step(now_ms, Input::Tick);
        }
    }
    assert_eq!(finished, Some((Some(599), Outcome::Ended(EndReason::Suspended))));
    assert_eq!(low_latency, [true, false]);
    assert_eq!(engine.lines_in[2], "{\"seq\":10,\"type\":\"launch\",\"session\":1,\"target\":\"pc1\",\"game\":\"steam:1245620\"}\n");

    // The checkpoint file had the last whole minute; the finished session replaces it in the history.
    let mut recorded = open.unwrap();
    assert_eq!(recorded.played.seconds, 540);
    recorded.played.seconds = 599;
    assert!(state.record(recorded));
    assert_eq!((state.profile().history.total(game.id).sessions, state.profile().history.total(game.id).seconds), (1, 599));

    // Play now goes straight back to the PC.
    let again = launch::resolve(&candidates(&game, &targets), state.profile().user.remembered(game.id));
    assert_eq!(again, Resolution::Launch { key: pc.clone(), why: Why::LastPlayed });

    // After a restart the PC is offline. GeForce NOW is ready, and Play still does not move there by itself.
    let mut state = State::load(&state.save()).unwrap().value;
    targets.get_mut(&(punktfunk, pc.target.clone())).unwrap().reach = Reach::Offline;
    let games = build(&mut state, &listing);
    assert_eq!(games[0].id, game.id, "the game is the same game after a restart");
    let offline = launch::resolve(&candidates(&games[0], &targets), state.profile().user.remembered(game.id));
    assert_eq!(offline, Resolution::Choose { why: ChooseWhy::Unavailable, focus: Some(pc) });
}
