//! The echo canceller on a synthetic room: the TV's speech comes back to the microphone after a long delay and through a reverberant
//! tail, and the lag it finds, what it takes out and the wake word over it are measured on that.

use lumavoice::echo::{Canceller, EchoConfig, EchoState, EchoStatus};
use lumavoice::engine::{Engine, EngineConfig, Out};
use lumavoice::models::{self, Sensitivity};
use std::sync::LazyLock;
use wake_word::wav;

const OKAY_NABU: &[u8] = include_bytes!("data/okay_nabu_hazel.wav");
const OKAY_NABU_TIME: &[u8] = include_bytes!("data/okay_nabu_time.wav");
const HEY_JARVIS: &[u8] = include_bytes!("data/hey_jarvis_hazel.wav");
const SPEECH: &[u8] = include_bytes!("data/speech_no_wake.wav");

/// A 20 ms read of both streams.
const PERIOD: usize = 320;
const RATE: usize = 16_000;
/// Where the microphone hears the sound after the reference carries it: a receiver's and an HDMI link's buffers, and the air.
const DELAY: usize = 2_880;
/// Seconds into the programme that the TV says the wake word, after a pause.
const TV_SAYS_IT: f32 = 16.0;

fn clip(bytes: &[u8]) -> Vec<i16> {
    wav::read(bytes).unwrap().samples
}

fn seconds(s: f32) -> usize {
    (s * RATE as f32) as usize
}

struct Rng(u64);

impl Rng {
    /// Uniform in [-1, 1).
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

/// The room between the receiver and the microphone: the direct sound and 80 reflections that fade over 110 ms, as sparse taps, at
/// -6 dB in all.
fn room() -> Vec<(usize, f32)> {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut taps = vec![(0, 1.0)];
    for _ in 0..80 {
        let at = 20 + ((rng.next() + 1.0) * 870.0) as usize;
        taps.push((at, rng.next() * 0.35 * (-(at as f32) / 600.0).exp()));
    }
    let energy: f32 = taps.iter().map(|t| t.1 * t.1).sum();
    let scale = (0.25 / energy).sqrt();
    taps.iter().map(|&(at, g)| (at, g * scale)).collect()
}

/// What `sound` becomes in the room, `delay` samples late, with the room's tail after it.
fn through_room(sound: &[i16], delay: usize) -> Vec<f32> {
    let x: Vec<f32> = sound.iter().map(|&s| f32::from(s)).collect();
    let mut heard = vec![0.0; sound.len() + delay + 1_800];
    for (at, gain) in room() {
        for (slot, s) in heard[at + delay..].iter_mut().zip(&x) {
            *slot += gain * s;
        }
    }
    heard
}

fn to_i16(heard: &[f32]) -> Vec<i16> {
    heard
        .iter()
        .map(|v| v.round().clamp(-32_768.0, 32_767.0) as i16)
        .collect()
}

fn add(mic: &mut [i16], at: usize, clip: &[i16]) {
    for (slot, &s) in mic[at..].iter_mut().zip(clip) {
        *slot = slot.saturating_add(s);
    }
}

fn scaled(clip: &[i16], gain: f32) -> Vec<i16> {
    clip.iter().map(|&s| (f32::from(s) * gain) as i16).collect()
}

/// Steady noise at -35 dBFS: sound that is nothing the room played.
fn noise(len: usize) -> Vec<i16> {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    (0..len).map(|_| (rng.next() * 1_000.0) as i16).collect()
}

/// The TV's programme and what the microphone hears of it: 15 s of speech that does not say the wake word, a pause, and then the
/// wake word, through the room over a noise floor at -60 dBFS.
struct Scene {
    sound: Vec<i16>,
    mic: Vec<i16>,
}

static SCENE: LazyLock<Scene> = LazyLock::new(|| {
    let mut sound = clip(SPEECH);
    sound.extend(clip(HEY_JARVIS));
    sound.extend(clip(SPEECH));
    sound.resize(seconds(TV_SAYS_IT), 0);
    sound.extend(clip(OKAY_NABU_TIME));
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let heard: Vec<f32> = through_room(&sound, DELAY)
        .iter()
        .map(|v| v + rng.next() * 57.0)
        .collect();
    Scene {
        mic: to_i16(&heard[..sound.len()]),
        sound,
    }
});

fn run(c: &mut Canceller, mic: &[i16], reference: &[i16]) -> Vec<i16> {
    let mut out = Vec::new();
    for (m, r) in mic.chunks(PERIOD).zip(reference.chunks(PERIOD)) {
        c.push_mic(m, &mut out);
        c.push_reference(r, &mut out);
    }
    out
}

fn opened() -> Canceller {
    let mut c = Canceller::new(&EchoConfig::default());
    c.open_reference(&mut Vec::new());
    c
}

/// The programme through a canceller that is given the reference as it plays: what comes out, how it ended, and the credit it claims.
static CLEANED: LazyLock<(Vec<i16>, EchoStatus, f32)> = LazyLock::new(|| {
    let mut c = opened();
    let out = run(&mut c, &SCENE.mic, &SCENE.sound);
    (out, c.status(), c.credit_db())
});

/// How much weaker `out` is than `mic` over the same stretch, in dB. The canceller gives the microphone back in order, so the two
/// line up by position.
fn erle_db(mic: &[i16], out: &[i16], from: f32, to: f32) -> f32 {
    let power = |x: &[i16]| {
        x[seconds(from)..seconds(to)]
            .iter()
            .map(|&s| (f64::from(s) / 32_768.0).powi(2))
            .sum::<f64>()
    };
    (10.0 * (power(mic) / power(out)).log10()) as f32
}

/// The wake words an engine hears in `samples`.
fn wakes(samples: &[i16], cutoff: f32) -> usize {
    let word = models::load("okay_nabu", cutoff).unwrap();
    let mut engine = Engine::new(EngineConfig::default(), vec![word]).unwrap();
    let mut out = Vec::new();
    for read in samples.chunks(PERIOD) {
        engine.push(read, false, &mut out).unwrap();
    }
    out.iter().filter(|o| matches!(o, Out::Wake { .. })).count()
}

#[test]
fn the_tvs_echo_is_taken_out_by_at_least_20_db() {
    let (out, status, credit) = &*CLEANED;
    assert_eq!(status.state, EchoState::Cancelling);
    assert!((status.lag_ms.unwrap() - 180.0).abs() < 1.0, "{status:?}");
    let erle = erle_db(&SCENE.mic, out, 10.0, 14.5);
    assert!(
        erle >= 20.0,
        "the last 4.5 s of the echo are {erle:.1} dB down"
    );
    assert!(
        status.erle_db.is_some_and(|db| db >= 15.0),
        "the meter tells it as it measures it: {status:?}"
    );
    assert!(
        *credit >= 15.0,
        "and the wake word is asked for less: {credit}"
    );
}

#[test]
fn a_reference_that_comes_after_its_echo_is_waited_for_and_let_go_with_the_stream() {
    let s = &*SCENE;
    // The reference stream is 480 ms behind the sound where the microphone is 180 ms behind it: it trails the echo by 300 ms.
    let mut late = vec![0i16; seconds(0.48)];
    late.extend_from_slice(&s.sound);
    let mut c = opened();
    let mut out = run(&mut c, &s.mic, &late);
    let status = c.status();
    assert_eq!(status.state, EchoState::Cancelling);
    assert!((status.lag_ms.unwrap() + 300.0).abs() < 1.0, "{status:?}");
    assert!(
        (300.0..330.0).contains(&status.held_ms),
        "the microphone waits the lag and the margin: {status:?}"
    );
    assert!(erle_db(&s.mic, &out, 10.0, 14.5) >= 20.0);
    c.close_reference(&mut out);
    assert!(
        s.mic.len() - out.len() < 256,
        "what was held comes out with the stream's end: {} of {}",
        out.len(),
        s.mic.len()
    );
}

fn unchanged(mic: &[i16], out: &[i16]) {
    assert!(
        mic.len() - out.len() < 256,
        "less than a block behind: {} of {}",
        out.len(),
        mic.len()
    );
    assert_eq!(out, &mic[..out.len()]);
}

#[test]
fn without_a_usable_reference_the_microphone_passes_unchanged() {
    let s = &*SCENE;
    let mut out = Vec::new();
    let mut none = Canceller::new(&EchoConfig::default());
    for m in s.mic.chunks(PERIOD) {
        none.push_mic(m, &mut out);
    }
    assert_eq!(none.status().state, EchoState::Off);
    unchanged(&s.mic, &out);

    // A stream that is not what the room hears is not trusted either.
    let mut c = opened();
    let out = run(&mut c, &s.mic, &noise(s.mic.len()));
    assert_ne!(c.status().state, EchoState::Cancelling, "{:?}", c.status());
    assert_eq!(c.credit_db(), 0.0);
    unchanged(&s.mic, &out);

    let mut off = Canceller::new(&EchoConfig {
        enabled: false,
        ..EchoConfig::default()
    });
    off.open_reference(&mut Vec::new());
    let mut out = Vec::new();
    off.push_mic(&s.mic[..1_000], &mut out);
    assert_eq!(
        out,
        &s.mic[..1_000],
        "a canceller that is off adds no delay"
    );
    assert_eq!(off.status().state, EchoState::Off);
}

#[test]
fn a_reference_that_stops_arriving_is_given_up_and_the_microphone_goes_on() {
    let s = &*SCENE;
    let cut = seconds(12.0);
    let mut c = opened();
    let mut out = run(&mut c, &s.mic[..cut], &s.sound[..cut]);
    assert_eq!(c.status().state, EchoState::Cancelling);
    for m in s.mic[cut..cut + seconds(3.0)].chunks(PERIOD) {
        c.push_mic(m, &mut out);
    }
    assert_eq!(c.status().state, EchoState::Lost);
    assert_eq!(c.credit_db(), 0.0);
    assert!(
        cut + seconds(3.0) - out.len() < 256,
        "the microphone was not held for a stream that is gone"
    );
}

#[test]
fn a_lock_that_takes_nothing_off_any_more_is_given_up() {
    let s = &*SCENE;
    // The reference follows the room for 6 s, then carries something else (the TV's route has changed).
    let cut = seconds(6.0);
    let mut reference = s.sound[..cut].to_vec();
    reference.extend(noise(s.sound.len() - cut));
    let mut c = opened();
    let _ = run(&mut c, &s.mic, &reference);
    let status = c.status();
    assert_ne!(status.state, EchoState::Cancelling, "{status:?}");
    assert_eq!(c.credit_db(), 0.0);
}

#[test]
fn a_person_over_the_echo_is_heard_where_the_raw_microphone_does_not_wake() {
    let s = &*SCENE;
    // Half the programme's level at the microphone: 0 dB over the echo, which the wake word does not get through.
    let mut mic = s.mic[..seconds(14.5)].to_vec();
    add(&mut mic, seconds(10.0), &scaled(&clip(OKAY_NABU), 0.5));
    let out = run(&mut opened(), &mic, &s.sound);
    let cutoff = Sensitivity::Normal.cutoff();
    assert_eq!(wakes(&out[seconds(9.0)..], cutoff), 1);
    assert_eq!(wakes(&mic[seconds(9.0)..], cutoff), 0);
}

#[test]
fn the_tvs_own_wake_word_is_taken_out_even_at_the_most_lenient_cutoff() {
    let (out, ..) = &*CLEANED;
    let from = seconds(TV_SAYS_IT - 1.0);
    let cutoff = Sensitivity::Lenient.cutoff();
    assert_eq!(
        wakes(&SCENE.mic[from..], cutoff),
        1,
        "the room gives the TV's word back as it is said"
    );
    assert_eq!(wakes(&out[from..], cutoff), 0);
}
