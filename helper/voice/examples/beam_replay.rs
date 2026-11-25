//! Replays a capture of the TV's two microphone channels and its reference stream through three chains and compares them:
//!
//! * `today`: the channels added, then the echo canceller (the engine before the beam);
//! * `beam`: the channels through the beam, then one canceller;
//! * `permic`: a canceller on each channel, then the beam.
//!
//! Each chain's decoder output also goes to a wake word of its own before the canceller, which is what the engine would hear with no
//! reference (a TrueHD bitstream leaves the reference silent). Capture the streams with the commands the engine opens them with
//! (`echo_replay` lists them), then `cargo run --release --example beam_replay -- mic.raw ref.raw [options]`.
//!
//! `--bench BLOCKS` times the chains on synthetic sound instead (no file; run it once per chain to read the memory it adds).

use lumavoice::beam::{Beam, BeamConfig, Mode};
use lumavoice::echo::{Canceller, EchoConfig, EchoState};
use lumavoice::engine::{Engine, EngineConfig, Out};
use lumavoice::models;
use lumavoice::pcm::{Format, Frames, Pick, Sample};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Instant;
use voice_wire::EndReason;

const RATE: usize = 16_000;
const STEP_MS: usize = 16;
const STEP_S: f32 = STEP_MS as f32 / 1_000.0;

const HELP: &str = "usage: beam_replay MIC.raw REF.raw [options] | beam_replay --bench BLOCKS [--chains today,beam,permic]
  --mic-format s24:2:16000  --ref-format s16:2:96000  --ref-pick 1  --gain-db 12  --skip-ms 250
  --word okay_nabu|hey_jarvis  --cutoff 0.97  --extra 0.0
  --chains today,beam,permic             the chains to run (today is the baseline and goes first)
  --marks FILE                           lines of `START END LABEL` in seconds of file time: phrases the TV says, bursts of noise
  --inject CLIP.wav:LEVEL_DBFS:EVERY_S:FIRST_S   adds the clip to both channels (couch speech) at its active level, every so often
  --seconds N                            only the first N seconds
  --settle-secs 3                        suppression counts from this long after the lock
  --inject-delay 1                       the second channel hears the injected clip this many samples late (a talker to one side)
  --spacing-mm 55  --look-delay 0  --noise-gain-db 4  --memory-secs 6  --cone 1.3   the beam";

struct Mark {
    start: f32,
    end: f32,
    label: String,
}

struct Inject {
    clip: Vec<f32>,
    level_db: f32,
    every: f32,
    first: f32,
}

struct Options {
    mic_format: Format,
    ref_format: Format,
    ref_pick: Pick,
    gain_db: f32,
    skip_ms: usize,
    word: String,
    cutoff: f32,
    extra: f32,
    chains: Vec<String>,
    marks: Vec<Mark>,
    inject: Option<Inject>,
    /// The second channel hears the injected clip this many samples late: a talker off to one side.
    inject_delay: usize,
    seconds: Option<f32>,
    settle_secs: f32,
    beam: BeamConfig,
    bench: Option<usize>,
}

fn parse_format(text: &str) -> Result<Format, String> {
    let bad = || format!("format {text:?} is not sample:channels:rate (s24:2:16000)");
    let mut parts = text.split(':');
    let sample = match parts.next().ok_or_else(bad)? {
        "s16" => Sample::S16,
        "s24" => Sample::S24,
        "s32" => Sample::S32,
        _ => return Err(bad()),
    };
    let channels = parts.next().and_then(|c| c.parse().ok()).ok_or_else(bad)?;
    let rate = parts.next().and_then(|c| c.parse().ok()).ok_or_else(bad)?;
    Ok(Format { rate, channels, sample })
}

fn number<T: std::str::FromStr>(flag: &str, text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("{flag} wants a number, not {text:?}"))
}

fn read_wav(path: &str) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        if &bytes[at..at + 4] == b"data" {
            let end = (at + 8 + size).min(bytes.len());
            return Ok(bytes[at + 8..end]
                .chunks_exact(2)
                .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32_768.0)
                .collect());
        }
        at += 8 + size + (size & 1);
    }
    Err(format!("{path}: no data chunk"))
}

fn read_marks(path: &str) -> Result<Vec<Mark>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let mut marks = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        let mut p = line.split_whitespace();
        let (Some(a), Some(b), Some(label)) = (p.next(), p.next(), p.next()) else {
            return Err(format!("{path}: {line:?} is not START END LABEL"));
        };
        marks.push(Mark { start: number("marks", a)?, end: number("marks", b)?, label: label.to_owned() });
    }
    Ok(marks)
}

fn parse_args(args: &[String]) -> Result<(Vec<String>, Options), String> {
    let mut o = Options {
        mic_format: Format { rate: 16_000, channels: 2, sample: Sample::S24 },
        ref_format: Format { rate: 96_000, channels: 2, sample: Sample::S16 },
        ref_pick: Pick::Channel(1),
        gain_db: 12.0,
        skip_ms: 250,
        word: "okay_nabu".into(),
        cutoff: models::Sensitivity::Normal.cutoff(),
        extra: 0.0,
        chains: vec!["today".into(), "beam".into(), "permic".into()],
        marks: Vec::new(),
        inject: None,
        inject_delay: 0,
        seconds: None,
        settle_secs: 3.0,
        beam: BeamConfig::default(),
        bench: None,
    };
    let (mut files, mut it) = (Vec::new(), args.iter());
    while let Some(arg) = it.next() {
        if !arg.starts_with("--") {
            files.push(arg.clone());
            continue;
        }
        let value = it.next().ok_or_else(|| format!("{arg} wants a value"))?;
        match arg.as_str() {
            "--mic-format" => o.mic_format = parse_format(value)?,
            "--ref-format" => o.ref_format = parse_format(value)?,
            "--ref-pick" => o.ref_pick = Pick::Channel(number(arg, value)?),
            "--gain-db" => o.gain_db = number(arg, value)?,
            "--skip-ms" => o.skip_ms = number(arg, value)?,
            "--word" => o.word = value.clone(),
            "--cutoff" => o.cutoff = number(arg, value)?,
            "--extra" => o.extra = number(arg, value)?,
            "--chains" => o.chains = value.split(',').filter(|c| *c != "none").map(str::to_owned).collect(),
            "--marks" => o.marks = read_marks(value)?,
            "--seconds" => o.seconds = Some(number(arg, value)?),
            "--settle-secs" => o.settle_secs = number(arg, value)?,
            "--spacing-mm" => o.beam.spacing_mm = number(arg, value)?,
            "--look-delay" => o.beam.look_delay = number(arg, value)?,
            "--noise-gain-db" => o.beam.max_noise_gain_db = number(arg, value)?,
            "--memory-secs" => o.beam.memory_secs = number(arg, value)?,
            "--cone" => o.beam.cone_samples = number(arg, value)?,
            "--bench" => o.bench = Some(number(arg, value)?),
            "--inject-delay" => o.inject_delay = number(arg, value)?,
            "--inject" => {
                let p: Vec<&str> = value.split(':').collect();
                let [clip, level, every, first] = p[..] else {
                    return Err("--inject is CLIP.wav:LEVEL_DBFS:EVERY_S:FIRST_S".into());
                };
                o.inject = Some(Inject {
                    clip: read_wav(clip)?,
                    level_db: number(arg, level)?,
                    every: number(arg, every)?,
                    first: number(arg, first)?,
                });
            }
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    Ok((files, o))
}

fn db(power: f64) -> f32 {
    (10.0 * power.max(1e-12).log10()) as f32
}

fn power(samples: &[i16]) -> f64 {
    samples.iter().map(|&s| (f64::from(s) / 32_768.0).powi(2)).sum::<f64>() / samples.len().max(1) as f64
}

fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return f32::NAN;
    }
    sorted[((sorted.len() - 1) as f32 * p).round() as usize]
}

fn to_i16(v: f32) -> i16 {
    (v * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16
}

fn s24(v: f32) -> [u8; 4] {
    let b = ((v * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32).to_le_bytes();
    [b[0], b[1], b[2], 0]
}

fn rss_kb() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

/// What a wake word model made of a stream: the highest score of every step, and the steps it woke at.
struct Listener {
    engine: Engine,
    step: usize,
    peaks: Vec<f32>,
    wakes: Vec<(usize, f32)>,
}

impl Listener {
    fn new(o: &Options) -> Result<Self, String> {
        let word = models::load(&o.word, o.cutoff)?;
        let mut engine = Engine::new(EngineConfig::default(), vec![word])?;
        engine.set_strictness(o.extra);
        Ok(Self { engine, step: 0, peaks: Vec::new(), wakes: Vec::new() })
    }

    fn feed(&mut self, samples: &[i16]) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        let mut out = Vec::new();
        self.engine.push(samples, false, &mut out)?;
        for o in &out {
            if let Out::Wake { score, .. } = o {
                self.wakes.push((self.step, *score));
                self.engine.end(EndReason::NoAck, &mut Vec::new());
            }
        }
        Ok(())
    }

    fn end_step(&mut self) {
        self.peaks.push(self.engine.take_score_peak());
        self.step += 1;
    }

    fn describe(&self, name: &str, marks: &[Mark], cutoff: f32, out: &mut String) {
        let best = self.peaks.iter().copied().fold(0.0f32, f32::max);
        let at = self.peaks.iter().position(|&p| p == best).unwrap_or(0);
        let _ = writeln!(
            out,
            "  {name}: {} wakes; best peak {best:.2} at {:.1} s (cutoff {cutoff})",
            self.wakes.len(),
            at as f32 * STEP_S
        );
        // Per label: events, the highest and lowest peak around them (from 0.3 s before to 1.5 s after), and the wakes inside.
        let mut labels: BTreeMap<&str, (usize, f32, f32, usize, usize)> = BTreeMap::new();
        let mut bits = String::new();
        for m in marks.iter().filter(|m| !m.label.starts_with("noise")) {
            let first = ((m.start - 0.3).max(0.0) / STEP_S) as usize;
            let last = (((m.end + 1.5) / STEP_S) as usize).min(self.peaks.len().saturating_sub(1));
            if first > last {
                continue;
            }
            let peak = self.peaks[first..=last].iter().copied().fold(0.0f32, f32::max);
            let woke = self.wakes.iter().filter(|(k, _)| (first..=last).contains(k)).count();
            let e = labels.entry(&m.label).or_insert((0, 0.0, f32::MAX, 0, 0));
            e.0 += 1;
            e.1 = e.1.max(peak);
            e.2 = e.2.min(peak);
            e.3 += woke;
            e.4 += usize::from(peak >= cutoff);
            if m.label == "inject" {
                bits.push(if woke > 0 { '1' } else { '0' });
            }
        }
        for (label, (n, hi, lo, woke, over)) in labels {
            let _ = writeln!(
                out,
                "      {label}: {n} events, peak max {hi:.2} min {lo:.2} (margin to the cutoff {:+.2}), {woke} wakes, {over} at or over the cutoff",
                cutoff - hi
            );
        }
        if !bits.is_empty() {
            let _ = writeln!(out, "      inject bits {bits}");
        }
    }
}

/// One way of taking the microphones to the wake word.
struct Chain {
    name: String,
    frames: Vec<Frames>,
    cancellers: Vec<Canceller>,
    beam: Option<Beam>,
    queue: [Vec<f32>; 2],
    skip: usize,
    opened: bool,
    /// What the decoder gave (one decoder only), and what the chain gave the wake word.
    raw: Vec<i16>,
    out: Vec<i16>,
    raw_ear: Option<Listener>,
    ear: Option<Listener>,
    decode_us: Vec<f32>,
    cancel_us: Vec<f32>,
    mix_us: Vec<f32>,
    lock_at: Option<f32>,
    lag_ms: Option<f32>,
    state: EchoState,
}

impl Chain {
    fn new(kind: &str, o: &Options, ears: bool) -> Result<Self, String> {
        let beamed = BeamConfig { enabled: true, ..o.beam };
        let decoder = |pick| Frames::new(o.mic_format, o.gain_db, pick).map_err(|e| e.to_string());
        let (frames, beam) = match kind {
            "today" => (vec![decoder(Pick::Auto)?], None),
            "beam" => (vec![decoder(Pick::Auto)?.with_beam(&beamed)], None),
            // The beam's own path (its 16 ms and its transform) with the weights held at the plain sum: tells a loss of the weights from a loss of the path.
            "frozen" => {
                let mut frames = decoder(Pick::Auto)?.with_beam(&beamed);
                frames.set_beam_adapt(false);
                (vec![frames], None)
            }
            "permic" => (vec![decoder(Pick::Channel(0))?, decoder(Pick::Channel(1))?], Some(Beam::new(&beamed))),
            other => return Err(format!("chain {other:?} is today, beam or permic")),
        };
        let echo = EchoConfig::default();
        echo.check()?;
        Ok(Self {
            name: kind.to_owned(),
            cancellers: frames.iter().map(|_| Canceller::new(&echo)).collect(),
            frames,
            beam,
            queue: [Vec::new(), Vec::new()],
            skip: o.skip_ms * RATE / 1_000,
            opened: false,
            raw: Vec::new(),
            out: Vec::new(),
            raw_ear: if ears && kind != "permic" { Some(Listener::new(o)?) } else { None },
            ear: if ears { Some(Listener::new(o)?) } else { None },
            decode_us: Vec::new(),
            cancel_us: Vec::new(),
            mix_us: Vec::new(),
            lock_at: None,
            lag_ms: None,
            state: EchoState::Off,
        })
    }

    fn step(&mut self, mic: &[u8], reference: Option<&[i16]>, now: f32) -> Result<(), String> {
        let n = self.frames.len();
        let mut outs: Vec<Vec<i16>> = vec![Vec::new(); n];
        let (mut decode, mut cancel, mut mix) = (0.0f32, 0.0f32, 0.0f32);
        if reference.is_some() && !self.opened {
            self.opened = true;
            for (c, o) in self.cancellers.iter_mut().zip(&mut outs) {
                c.open_reference(o);
            }
        }
        let t = Instant::now();
        let mut monos: Vec<Vec<i16>> = vec![Vec::new(); n];
        let mut dropped = 0;
        for (f, m) in self.frames.iter_mut().zip(&mut monos) {
            f.push(mic, m);
            dropped = self.skip.min(m.len());
            m.drain(..dropped);
        }
        self.skip -= dropped;
        decode += t.elapsed().as_secs_f32();
        if !monos[0].is_empty() {
            let t = Instant::now();
            for ((c, m), o) in self.cancellers.iter_mut().zip(&monos).zip(&mut outs) {
                c.push_mic(m, o);
            }
            cancel += t.elapsed().as_secs_f32();
            if n == 1 {
                self.raw.extend_from_slice(&monos[0]);
                if let Some(e) = self.raw_ear.as_mut() {
                    e.feed(&monos[0])?;
                }
            }
        }
        if let Some(r) = reference {
            let t = Instant::now();
            for (c, o) in self.cancellers.iter_mut().zip(&mut outs) {
                c.push_reference(r, o);
            }
            cancel += t.elapsed().as_secs_f32();
        }
        let clean = match self.beam.as_mut() {
            Some(beam) => {
                let t = Instant::now();
                for (q, o) in self.queue.iter_mut().zip(&outs) {
                    q.extend(o.iter().map(|&s| f32::from(s) / 32_768.0));
                }
                let k = self.queue[0].len().min(self.queue[1].len());
                let mut mixed = Vec::new();
                beam.process(&self.queue[0][..k], &self.queue[1][..k], Mode::Beam, &mut mixed);
                self.queue.iter_mut().for_each(|q| {
                    q.drain(..k);
                });
                let clean: Vec<i16> = mixed.iter().map(|&v| to_i16(v)).collect();
                mix += t.elapsed().as_secs_f32();
                clean
            }
            None => outs.swap_remove(0),
        };
        if self.opened {
            self.decode_us.push(decode * 1e6);
            self.cancel_us.push(cancel * 1e6);
            self.mix_us.push(mix * 1e6);
        }
        if let Some(ear) = self.ear.as_mut() {
            ear.feed(&clean)?;
            ear.end_step();
        }
        if let Some(ear) = self.raw_ear.as_mut() {
            ear.end_step();
        }
        self.out.extend_from_slice(&clean);
        let status = self.cancellers[0].status();
        if status.state == EchoState::Cancelling && self.lock_at.is_none() {
            self.lock_at = Some(now);
            self.lag_ms = status.lag_ms;
        }
        self.state = status.state;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        let mut tails = vec![Vec::new(); self.cancellers.len()];
        for (c, t) in self.cancellers.iter_mut().zip(&mut tails) {
            c.close_reference(t);
        }
        if self.beam.is_none() {
            self.out.extend_from_slice(&tails[0]);
            if let Some(e) = self.ear.as_mut() {
                e.feed(&tails[0])?;
            }
        }
        Ok(())
    }

    fn cost(&self, out: &mut String) {
        let line = |name: &str, v: &[f32]| {
            let mut s = v.to_vec();
            s.sort_by(f32::total_cmp);
            let mean = s.iter().sum::<f32>() / s.len().max(1) as f32;
            format!(
                "{name} mean {mean:.0} us (p50 {:.0}, p99 {:.0}, max {:.0})",
                percentile(&s, 0.5),
                percentile(&s, 0.99),
                s.last().copied().unwrap_or(f32::NAN)
            )
        };
        let total: Vec<f32> = (0..self.decode_us.len())
            .map(|i| self.decode_us[i] + self.cancel_us[i] + self.mix_us[i])
            .collect();
        let mean = total.iter().sum::<f32>() / total.len().max(1) as f32;
        let _ = writeln!(
            out,
            "  {}: {}; {}; {}; {}; {:.2} % of a core",
            self.name,
            line("decode", &self.decode_us),
            line("canceller", &self.cancel_us),
            line("beam after", &self.mix_us),
            line("total", &total),
            mean / (STEP_MS as f32 * 10.0)
        );
    }
}

/// The reduction of `out` against `base` over 1 s windows from `from` (a sample count) that stand 6 dB over `floor`: the total, and each window's.
fn reduction(base: &[i16], out: &[i16], from: usize, floor: f32) -> (Option<f32>, Vec<f32>) {
    let n = base.len().min(out.len());
    let (mut sum_in, mut sum_out, mut each) = (0.0f64, 0.0f64, Vec::new());
    let mut at = from;
    while at + RATE <= n {
        let (pin, pout) = (power(&base[at..at + RATE]), power(&out[at..at + RATE]));
        if db(pin) > floor + 6.0 {
            sum_in += pin;
            sum_out += pout;
            each.push(db(pin) - db(pout));
        }
        at += RATE;
    }
    each.sort_by(f32::total_cmp);
    ((sum_in > 0.0).then(|| db(sum_in) - db(sum_out)), each)
}

/// The reduction over the marked span (seconds of file time) as one figure.
fn span_reduction(base: &[i16], out: &[i16], m: &Mark, skip_ms: usize) -> Option<f32> {
    let skip = skip_ms * RATE / 1_000;
    let a = (m.start * RATE as f32) as usize;
    let b = (m.end * RATE as f32) as usize;
    let (a, b) = (a.checked_sub(skip)?, b.checked_sub(skip)?);
    if b > base.len().min(out.len()) || a >= b {
        return None;
    }
    Some(db(power(&base[a..b])) - db(power(&out[a..b])))
}

/// Adds the clip to both channels at its active level, every so often; the marks of the places it went to.
fn inject(mic: &mut [u8], inj: &Inject, o: &Options) -> Result<Vec<Mark>, String> {
    if o.mic_format.sample != Sample::S24 || o.mic_format.channels != 2 {
        return Err("--inject needs a 24-bit two-channel microphone file".into());
    }
    // The active level: the mean power of the 20 ms blocks within 25 dB of the loudest.
    let blocks: Vec<f64> = inj
        .clip
        .chunks(RATE / 50)
        .map(|c| c.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / c.len() as f64)
        .collect();
    let top = blocks.iter().copied().fold(0.0f64, f64::max);
    let active: Vec<f64> = blocks.iter().copied().filter(|&p| p > top * 10f64.powf(-2.5)).collect();
    let active_db = db(active.iter().sum::<f64>() / active.len().max(1) as f64);
    let gain = 10f32.powf((inj.level_db - active_db) / 20.0);
    let mut marks = Vec::new();
    let frames = mic.len() / 8;
    let mut t = inj.first;
    while ((t * RATE as f32) as usize + inj.clip.len() + o.inject_delay) < frames {
        let start = (t * RATE as f32) as usize;
        for (j, &v) in inj.clip.iter().enumerate() {
            for c in 0..2 {
                let at = (start + j + if c == 1 { o.inject_delay } else { 0 }) * 8 + c * 4;
                let raw = i32::from_le_bytes([mic[at], mic[at + 1], mic[at + 2], 0]) << 8 >> 8;
                let sum = raw as f32 / 8_388_608.0 + gain * v;
                mic[at..at + 4].copy_from_slice(&s24(sum));
            }
        }
        marks.push(Mark { start: t, end: t + inj.clip.len() as f32 / RATE as f32, label: "inject".into() });
        t += inj.every;
    }
    Ok(marks)
}

fn replay(mut mic: Vec<u8>, mut reference: Vec<u8>, o: &Options) -> Result<String, String> {
    let mut text = String::new();
    if let Some(s) = o.seconds {
        mic.truncate((s * RATE as f32) as usize * o.mic_format.frame_bytes());
        reference.truncate((s * o.ref_format.rate as f32) as usize * o.ref_format.frame_bytes());
    }
    let mut marks = Vec::new();
    marks.extend(o.marks.iter().map(|m| Mark { start: m.start, end: m.end, label: m.label.clone() }));
    if let Some(inj) = &o.inject {
        marks.extend(inject(&mut mic, inj, o)?);
    }
    let mut chains = o.chains.iter().map(|k| Chain::new(k, o, true)).collect::<Result<Vec<_>, _>>()?;
    if chains.first().is_none_or(|c| c.name != "today") {
        return Err("the first chain is today: it is the baseline".into());
    }
    let mut ref_frames = Frames::new(o.ref_format, 0.0, o.ref_pick).map_err(|e| e.to_string())?;
    let mic_step = o.mic_format.frame_bytes() * (o.mic_format.rate as usize * STEP_MS / 1_000);
    let ref_step = o.ref_format.frame_bytes() * (o.ref_format.rate as usize * STEP_MS / 1_000);
    let open_step = o.skip_ms.div_ceil(STEP_MS);
    let ref_blocks: Vec<&[u8]> = reference.chunks(ref_step).collect();
    let mut decoded = Vec::new();
    for (k, block) in mic.chunks(mic_step).enumerate() {
        let now = k as f32 * STEP_S;
        let wanted = k >= open_step && ref_blocks.get(k).is_some();
        let reference_pcm = if wanted {
            decoded.clear();
            ref_frames.push(ref_blocks[k], &mut decoded);
            Some(decoded.as_slice())
        } else {
            None
        };
        for chain in &mut chains {
            chain.step(block, reference_pcm, now)?;
        }
    }
    for chain in &mut chains {
        chain.finish()?;
    }

    let base = chains[0].raw.clone();
    let mut quiet: Vec<f32> = base.chunks(RATE / 10).map(|c| db(power(c))).collect();
    quiet.sort_by(f32::total_cmp);
    let floor = percentile(&quiet, 0.05);
    let _ = writeln!(
        text,
        "{:.0} s of microphone; marks {}; the baseline's floor {floor:.1} dBFS after gain; word {} cutoff {} + {}",
        mic.len() as f32 / o.mic_format.frame_bytes() as f32 / RATE as f32,
        marks.len(),
        o.word,
        o.cutoff,
        o.extra
    );
    for chain in &chains {
        let from = chain.lock_at.map_or(usize::MAX, |l| ((l + o.settle_secs) * RATE as f32) as usize);
        let from = from.saturating_sub(o.skip_ms * RATE / 1_000);
        let _ = writeln!(
            text,
            "{}: lag {} locked at {} s, final state {:?}",
            chain.name,
            chain.lag_ms.map_or("none".into(), |l| format!("{l:.1} ms")),
            chain.lock_at.map_or("never".into(), |l| format!("{l:.1}")),
            chain.state
        );
        let (total, each) = reduction(&base, &chain.out, from, floor);
        let _ = writeln!(
            text,
            "  suppression of the chain against the plain sum: {} over {} windows; per window p10 {:.1}, median {:.1}, p90 {:.1}",
            total.map_or("none".into(), |t| format!("{t:.1} dB")),
            each.len(),
            percentile(&each, 0.1),
            percentile(&each, 0.5),
            percentile(&each, 0.9)
        );
        if chain.name != "today" && !chain.raw.is_empty() {
            let (alone, each) = reduction(&base, &chain.raw, from, floor);
            let _ = writeln!(
                text,
                "  the beam alone against the plain sum (no canceller): {} ; per window p10 {:.1}, median {:.1}, p90 {:.1}",
                alone.map_or("none".into(), |t| format!("{t:.2} dB")),
                percentile(&each, 0.1),
                percentile(&each, 0.5),
                percentile(&each, 0.9)
            );
        }
        if !chain.raw.is_empty() {
            // The room's own noise is what a beam that adds noise gain raises: the quietest quarter of the baseline's 100 ms blocks.
            let n = base.len().min(chain.raw.len()) / (RATE / 10);
            let mut blocks: Vec<(f64, f64)> = (0..n)
                .map(|i| {
                    let r = i * (RATE / 10)..(i + 1) * (RATE / 10);
                    (power(&base[r.clone()]), power(&chain.raw[r]))
                })
                .collect();
            blocks.sort_by(|a, b| a.0.total_cmp(&b.0));
            let (a, b) = blocks[..n / 4].iter().fold((0.0, 0.0), |(a, b), x| (a + x.0, b + x.1));
            let _ = writeln!(text, "  the quietest quarter of the blocks against the plain sum: {:+.2} dB", db(b) - db(a));
        }
        let gain = chain.frames[0].beam().or(chain.beam.as_ref()).map(Beam::noise_gain_db);
        if let Some(g) = gain {
            let _ = writeln!(text, "  the beam's noise gain at the end: {g:.2} dB");
        }
        for m in marks.iter().filter(|m| m.label.starts_with("noise")) {
            let full = span_reduction(&base, &chain.out, m, o.skip_ms);
            let alone = span_reduction(&base, &chain.raw, m, o.skip_ms);
            let _ = writeln!(
                text,
                "  {} {:.1}-{:.1} s: chain {} dB, beam alone {} dB",
                m.label,
                m.start,
                m.end,
                full.map_or("n/a".into(), |v| format!("{v:.1}")),
                alone.map_or("n/a".into(), |v| format!("{v:.2}"))
            );
        }
        if let Some(e) = &chain.raw_ear {
            e.describe("no canceller", &marks, o.cutoff, &mut text);
        }
        if let Some(e) = &chain.ear {
            e.describe("with the canceller", &marks, o.cutoff, &mut text);
        }
    }
    let _ = writeln!(text, "cost on this machine (the TV's A53 is several times slower):");
    for chain in &chains {
        chain.cost(&mut text);
    }
    Ok(text)
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / 2_147_483_648.0 - 1.0
    }
}

/// Times the chains on sound made up for the purpose: white noise as the TV, 0.3 of it 77 ms late in both channels with a little noise of
/// their own as the room.
fn bench(blocks: usize, o: &Options) -> Result<String, String> {
    let mut text = String::new();
    let n = blocks * 256;
    let mut rng = Rng(0x1234_5678);
    let tv: Vec<f32> = (0..n).map(|_| 0.05 * rng.next()).collect();
    let mut mic = Vec::with_capacity(n * 8);
    for i in 0..n {
        let echo = if i >= 1_232 { 0.3 * tv[i - 1_232] } else { 0.0 };
        for _ in 0..2 {
            mic.extend_from_slice(&s24(echo + 0.002 * rng.next()));
        }
    }
    let mut reference = Vec::with_capacity(n * 6 * 4);
    for &v in &tv {
        for _ in 0..6 {
            reference.extend_from_slice(&0i16.to_le_bytes());
            reference.extend_from_slice(&to_i16(v).to_le_bytes());
        }
    }
    let _ = writeln!(text, "synthetic {blocks} blocks; resident before the chains {} kB", rss_kb());
    let mut ref_frames = Frames::new(o.ref_format, 0.0, o.ref_pick).map_err(|e| e.to_string())?;
    let before = rss_kb();
    let mut chains = o.chains.iter().map(|k| Chain::new(k, o, false)).collect::<Result<Vec<_>, _>>()?;
    let built = rss_kb();
    let mic_step = o.mic_format.frame_bytes() * 256;
    let ref_step = o.ref_format.frame_bytes() * 1_536;
    let mut pcm = Vec::new();
    for (k, (block, r)) in mic.chunks(mic_step).zip(reference.chunks(ref_step)).enumerate() {
        pcm.clear();
        ref_frames.push(r, &mut pcm);
        for chain in &mut chains {
            chain.step(block, Some(&pcm), k as f32 * STEP_S)?;
        }
    }
    let _ = writeln!(text, "resident {before} kB before the chains, {built} kB built, {} kB after {blocks} blocks", rss_kb());
    for chain in &mut chains {
        // The first seconds are the lock; the filter works the same blocks after it.
        let settle = 400.min(chain.decode_us.len() / 2);
        chain.decode_us.drain(..settle);
        chain.cancel_us.drain(..settle);
        chain.mix_us.drain(..settle);
        let _ = writeln!(text, "{}: locked at {} s, state {:?}", chain.name, chain.lock_at.map_or("never".into(), |l| format!("{l:.1}")), chain.state);
        chain.cost(&mut text);
    }
    Ok(text)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }
    let run = || -> Result<String, String> {
        let (files, opts) = parse_args(&args)?;
        if let Some(blocks) = opts.bench {
            return bench(blocks, &opts);
        }
        let [mic, reference] = &files[..] else {
            return Err(HELP.to_owned());
        };
        let read = |p: &String| std::fs::read(p).map_err(|e| format!("{p}: {e}"));
        replay(read(mic)?, read(reference)?, &opts)
    };
    match run() {
        Ok(text) => print!("{text}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}
