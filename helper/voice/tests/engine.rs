//! The engine on recorded audio: the wake word in a clip wakes it once, speech without it does not, and a turn goes up only after the
//! client takes it, with the second before the wake in front.

use lumavoice::engine::{Engine, EngineConfig, Out};
use lumavoice::models::{self, Sensitivity};
use voice_wire::EndReason;
use wake_word::wav;

const OKAY_NABU: &[u8] = include_bytes!("data/okay_nabu_hazel.wav");
const HEY_JARVIS: &[u8] = include_bytes!("data/hey_jarvis_hazel.wav");
const OKAY_NABU_TIME: &[u8] = include_bytes!("data/okay_nabu_time.wav");
const SPEECH: &[u8] = include_bytes!("data/speech_no_wake.wav");

/// A 20 ms read, what the microphone hands over at a time.
const READ: usize = 320;

fn clip(bytes: &[u8]) -> Vec<i16> {
    let wav = wav::read(bytes).unwrap();
    assert_eq!(wav.rate, 16_000);
    wav.samples
}

fn silence(seconds: f32) -> Vec<i16> {
    vec![0; (seconds * 16_000.0) as usize]
}

fn engine(word: &str, cfg: EngineConfig) -> Engine {
    Engine::new(
        cfg,
        vec![models::load(word, Sensitivity::Normal.cutoff()).unwrap()],
    )
    .unwrap()
}

fn feed(engine: &mut Engine, samples: &[i16], deaf: bool) -> Vec<Out> {
    let mut out = Vec::new();
    for read in samples.chunks(READ) {
        engine.push(read, deaf, &mut out).unwrap();
    }
    out
}

/// Reads until the engine wakes: what it said and how many samples it had been given.
fn until_wake(engine: &mut Engine, samples: &[i16]) -> (Vec<Out>, usize) {
    let (mut out, mut given) = (Vec::new(), 0);
    for read in samples.chunks(READ) {
        engine.push(read, false, &mut out).unwrap();
        given += read.len();
        if !out.is_empty() {
            break;
        }
    }
    (out, given)
}

fn wakes(out: &[Out]) -> Vec<&Out> {
    out.iter()
        .filter(|o| matches!(o, Out::Wake { .. }))
        .collect()
}

fn audio_len(out: &[Out]) -> usize {
    out.iter()
        .map(|o| match o {
            Out::Audio(a) => a.len(),
            _ => 0,
        })
        .sum()
}

#[test]
fn each_word_wakes_the_engine_once_on_its_own_recording() {
    for (word, bytes) in [
        ("okay_nabu", OKAY_NABU),
        ("hey_jarvis", HEY_JARVIS),
        ("okay_nabu", OKAY_NABU_TIME),
    ] {
        let mut e = engine(word, EngineConfig::default());
        let out = feed(&mut e, &clip(bytes), false);
        let heard = wakes(&out);
        assert_eq!(heard.len(), 1, "{word}: {out:?}");
        let Out::Wake {
            word: said,
            score,
            level,
        } = heard[0]
        else {
            unreachable!()
        };
        assert_eq!(said, word);
        assert!(*score >= 0.97, "{word} woke at {score}");
        assert!(*level < 0.0, "a level is dBFS: {level}");
        assert_eq!(
            audio_len(&out),
            0,
            "nothing goes up before the client takes the turn"
        );
        assert!(e.in_turn());
    }
}

#[test]
fn speech_without_the_wake_word_wakes_nothing() {
    for word in ["okay_nabu", "hey_jarvis"] {
        let mut e = engine(word, EngineConfig::default());
        let out = feed(&mut e, &clip(SPEECH), false);
        assert!(out.is_empty(), "{word}: {out:?}");
        assert!(e.take_peak() > 1_000, "the clip is speech");
    }
}

#[test]
fn the_other_words_clip_does_not_wake_a_word() {
    let mut e = engine("hey_jarvis", EngineConfig::default());
    assert!(feed(&mut e, &clip(OKAY_NABU), false).is_empty());
    let mut e = engine("okay_nabu", EngineConfig::default());
    assert!(feed(&mut e, &clip(HEY_JARVIS), false).is_empty());
}

#[test]
fn a_turn_goes_up_after_accept_with_the_preroll_in_front_and_the_live_audio_after() {
    let mut e = engine("okay_nabu", EngineConfig::default());
    let (out, given) = until_wake(&mut e, &clip(OKAY_NABU));
    assert_eq!(wakes(&out).len(), 1);
    let mut out = Vec::new();
    // What the person says after the word, before the client has taken the turn: it is kept.
    let after = silence(0.2);
    out.extend(feed(&mut e, &after, false));
    assert_eq!(audio_len(&out), 0);
    e.accept(&mut out);
    let preroll = audio_len(&out) - after.len();
    assert_eq!(
        preroll,
        given.min(16_000),
        "the second before the wake, or all there was"
    );
    for o in &out {
        if let Out::Audio(a) = o {
            assert!(a.len() <= 1_280, "a message fits the frame: {}", a.len());
        }
    }
    out.clear();
    let live = feed(&mut e, &silence(0.5), false);
    assert_eq!(audio_len(&live), 8_000, "live audio goes on as it is heard");
    e.end(EndReason::Client, &mut out);
    assert_eq!(out, [Out::Ended(EndReason::Client)]);
    assert!(!e.in_turn());
    out.clear();
    e.end(EndReason::Client, &mut out);
    assert!(out.is_empty(), "an ended turn ends once");
}

#[test]
fn a_wake_nobody_takes_ends_without_a_sample_leaving() {
    let mut e = engine("okay_nabu", EngineConfig::default());
    let (out, _) = until_wake(&mut e, &clip(OKAY_NABU));
    assert_eq!(wakes(&out).len(), 1);
    let mut out = feed(&mut e, &silence(3.5), false);
    assert_eq!(out, [Out::Ended(EndReason::NoAck)]);
    assert!(!e.in_turn());
    out.clear();
    e.accept(&mut out);
    assert_eq!(audio_len(&out), 0, "a late accept has no turn to take");
}

#[test]
fn a_turn_is_cut_at_its_limit() {
    let cfg = EngineConfig {
        turn_secs: 2.0,
        ..EngineConfig::default()
    };
    let mut e = engine("okay_nabu", cfg);
    until_wake(&mut e, &clip(OKAY_NABU));
    let mut out = Vec::new();
    e.accept(&mut out);
    let out = feed(&mut e, &silence(3.0), false);
    assert_eq!(out.last(), Some(&Out::Ended(EndReason::Limit)));
    assert_eq!(
        audio_len(&out),
        32_000,
        "the turn is two seconds from the wake"
    );
    assert!(!e.in_turn());
}

#[test]
fn audio_that_is_not_for_the_wake_word_is_not_listened_to() {
    let mut e = engine("okay_nabu", EngineConfig::default());
    let samples = clip(OKAY_NABU);
    assert!(
        feed(&mut e, &samples, true).is_empty(),
        "another reader has the microphone"
    );
    assert!(!e.in_turn());
    // The model started clean: the word heard whole afterwards wakes it.
    assert_eq!(wakes(&feed(&mut e, &samples, false)).len(), 1);
}

#[test]
fn an_open_turn_is_not_cut_by_the_word_being_said_in_it() {
    let mut e = engine("okay_nabu", EngineConfig::default());
    let mut out = Vec::new();
    e.open();
    out.extend(feed(&mut e, &clip(OKAY_NABU), false));
    assert!(
        wakes(&out).is_empty(),
        "the word said inside a turn is the person talking: {out:?}"
    );
    assert_eq!(
        audio_len(&out),
        clip(OKAY_NABU).len(),
        "no preroll in an opened turn, all of it live"
    );
}

#[test]
fn more_strictness_than_the_scores_can_give_refuses_and_counts() {
    let mut e = engine("okay_nabu", EngineConfig::default());
    e.set_strictness(0.05);
    let out = feed(&mut e, &clip(OKAY_NABU), false);
    assert!(out.is_empty(), "{out:?}");
    assert_eq!(e.refused(), 1);
    assert!(e.take_score_peak() >= 0.97, "the model did hear it");
    assert_eq!(e.take_score_peak(), 0.0, "taking resets");
}

#[test]
fn the_level_meter_gives_a_peak_and_resets() {
    let mut e = engine("okay_nabu", EngineConfig::default());
    feed(&mut e, &clip(SPEECH)[..16_000], false);
    let peak = e.take_peak();
    assert!(peak > 500, "{peak}");
    assert_eq!(e.take_peak(), 0);
}
