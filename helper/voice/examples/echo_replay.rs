//! Replays a capture of the TV's microphone and its reference stream through the real decoder, canceller and wake word, and prints
//! what the canceller made of it: the lag, the lock time, the reduction (ERLE), the wake scores with and without it, and its cost.
//!
//! Capture the two streams together, while known sound plays, with the commands the engine opens them with:
//!
//! ```text
//! arecord -q -D hw:1,0 -t raw -f S24_LE -c 2 -r 16000 --period-size=256 --buffer-size=2048 -d 30 mic.raw &
//! arecord -q -D hw:1,2 -t raw -f S16_LE -c 2 -r 48000 -d 30 ref.raw &
//! ```
//!
//! then `cargo run --release --example echo_replay -- mic.raw ref.raw [options]`. The reference stream is opened at the moment the
//! server would open it (`--ref-open-ms`), and the microphone's first `--skip-ms` is dropped as the server drops it. Every option
//! that changes the canceller is one of `EchoConfig`'s or the filter's (`--help` lists them), so a run is a tuning point.

use lumavoice::delay::Search;
use lumavoice::echo::{Canceller, EchoConfig, EchoState};
use lumavoice::engine::{Engine, EngineConfig, Out};
use lumavoice::models;
use lumavoice::pcm::{Format, Frames, Pick, Sample};
use std::fmt::Write as _;
use std::time::Instant;
use voice_wire::EndReason;

const RATE: usize = 16_000;
/// One read of the engine's streams.
const STEP_MS: usize = 16;
/// A block of the level tables.
const BLOCK_MS: usize = 20;
/// The reference is sound from this level up (dBFS of a 20 ms block).
const SOUND_DB: f32 = -70.0;

struct Options {
    mic_format: Format,
    ref_format: Format,
    gain_db: f32,
    pick: Pick,
    ref_pick: Pick,
    skip_ms: usize,
    ref_open_ms: Option<usize>,
    echo: EchoConfig,
    word: String,
    cutoff: f32,
    extra: f32,
    rms_blocks: usize,
    settle_secs: f32,
    floor_db: Option<f32>,
    window_secs: f32,
    write_clean: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mic_format: Format {
                rate: 16_000,
                channels: 2,
                sample: Sample::S24,
            },
            ref_format: Format {
                rate: 48_000,
                channels: 2,
                sample: Sample::S16,
            },
            gain_db: 12.0,
            pick: Pick::Auto,
            ref_pick: Pick::Auto,
            skip_ms: 250,
            ref_open_ms: None,
            echo: EchoConfig::default(),
            word: "okay_nabu".into(),
            cutoff: models::Sensitivity::Normal.cutoff(),
            extra: 0.0,
            rms_blocks: 0,
            settle_secs: 3.0,
            floor_db: None,
            window_secs: 10.0,
            write_clean: None,
        }
    }
}

const HELP: &str = "usage: echo_replay MIC.raw REF.raw [options]
  --mic-format s24:2:16000      sample (s16|s24|s32), channels, rate of the microphone file
  --ref-format s16:2:48000      the same for the reference file
  --gain-db 12  --pick auto|sum|N        the microphone's gain and channel choice (the server's Config)
  --ref-pick auto|sum|N                  which reference channel carries the sound (Config.reference_pick)
  --skip-ms 250                          dropped from the microphone's start (the open pop)
  --ref-open-ms N                        when the reference stream opens, in file time (default: as soon as the skip is over)
  --mu F  --mu-start F  --start-secs F  --copy-ratio F  --partitions N   the filter (EchoConfig.aec)
  --margin-ms N  --search WINDOW:BEFORE:AFTER                            EchoConfig.margin_ms and .search (milliseconds)
  --no-echo                              canceller off: the microphone passes through
  --word okay_nabu|hey_jarvis  --cutoff 0.97  --extra 0.0                wake model, its cutoff, strictness on top
  --rms N                                print the first N 20 ms levels of every channel of both files
  --settle-secs 3  --floor-dbfs F        ERLE counts from this long after the lock, over windows this far above the floor
  --window-secs 10                       wake score window
  --write-clean PATH                     the canceller's output as 16 kHz mono S16_LE";

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
    if parts.next().is_some() {
        return Err(bad());
    }
    Ok(Format {
        rate,
        channels,
        sample,
    })
}

fn parse_pick(text: &str) -> Result<Pick, String> {
    match text {
        "auto" => Ok(Pick::Auto),
        "sum" => Ok(Pick::Sum),
        n => n
            .parse()
            .map(Pick::Channel)
            .map_err(|_| format!("pick {text:?} is auto, sum or a channel number")),
    }
}

fn number<T: std::str::FromStr>(flag: &str, text: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("{flag} wants a number, not {text:?}"))
}

fn parse_args(args: &[String]) -> Result<(String, String, Options), String> {
    let mut opts = Options::default();
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if !arg.starts_with("--") {
            files.push(arg.clone());
            continue;
        }
        if arg == "--no-echo" {
            opts.echo.enabled = false;
            continue;
        }
        let value = it
            .next()
            .ok_or_else(|| format!("{arg} wants a value"))?
            .as_str();
        match arg.as_str() {
            "--mic-format" => opts.mic_format = parse_format(value)?,
            "--ref-format" => opts.ref_format = parse_format(value)?,
            "--gain-db" => opts.gain_db = number(arg, value)?,
            "--pick" => opts.pick = parse_pick(value)?,
            "--ref-pick" => opts.ref_pick = parse_pick(value)?,
            "--skip-ms" => opts.skip_ms = number(arg, value)?,
            "--ref-open-ms" => opts.ref_open_ms = Some(number(arg, value)?),
            "--mu" => opts.echo.aec.mu = number(arg, value)?,
            "--mu-start" => opts.echo.aec.mu_start = number(arg, value)?,
            "--start-secs" => opts.echo.aec.start_secs = number(arg, value)?,
            "--copy-ratio" => opts.echo.aec.copy_ratio = number(arg, value)?,
            "--partitions" => opts.echo.aec.partitions = number(arg, value)?,
            "--margin-ms" => opts.echo.margin_ms = number(arg, value)?,
            "--search" => {
                let ms: Vec<u32> = value.split(':').filter_map(|v| v.parse().ok()).collect();
                let [window_ms, before_ms, after_ms] = ms[..] else {
                    return Err("--search is WINDOW:BEFORE:AFTER in milliseconds".into());
                };
                opts.echo.search = Search {
                    window_ms,
                    before_ms,
                    after_ms,
                };
            }
            "--word" => opts.word = value.to_owned(),
            "--cutoff" => opts.cutoff = number(arg, value)?,
            "--extra" => opts.extra = number(arg, value)?,
            "--rms" => opts.rms_blocks = number(arg, value)?,
            "--settle-secs" => opts.settle_secs = number(arg, value)?,
            "--floor-dbfs" => opts.floor_db = Some(number(arg, value)?),
            "--window-secs" => opts.window_secs = number(arg, value)?,
            "--write-clean" => opts.write_clean = Some(value.to_owned()),
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    let [mic, reference] = &files[..] else {
        return Err("two files: the microphone's and the reference's".into());
    };
    Ok((mic.clone(), reference.clone(), opts))
}

fn db(power: f64) -> f32 {
    (10.0 * power.max(1e-12).log10()) as f32
}

fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return f32::NAN;
    }
    sorted[((sorted.len() - 1) as f32 * p).round() as usize]
}

/// What one capture file is made of, channel by channel.
struct Stream {
    format: Format,
    seconds: f32,
    /// dBFS of every 20 ms block, per channel.
    blocks: Vec<Vec<f32>>,
    /// Frames in which every channel holds the same sample, as a fraction.
    identical: f32,
    /// Correlation of the first two channels over the file.
    correlation: f32,
}

impl Stream {
    fn read(bytes: &[u8], format: Format) -> Self {
        let width = format.sample.bytes();
        let frame = format.frame_bytes();
        let block = format.rate as usize * BLOCK_MS / 1_000;
        let mut blocks = vec![Vec::new(); format.channels];
        let mut sum_sq = vec![0.0f64; format.channels];
        let (mut same, mut frames, mut xy, mut xx, mut yy) =
            (0usize, 0usize, 0.0f64, 0.0f64, 0.0f64);
        for (i, f) in bytes.chunks_exact(frame).enumerate() {
            let v: Vec<f32> = (0..format.channels)
                .map(|c| format.sample.value(&f[c * width..(c + 1) * width]))
                .collect();
            for (s, x) in sum_sq.iter_mut().zip(&v) {
                *s += f64::from(*x) * f64::from(*x);
            }
            if v.iter().all(|&x| x == v[0]) {
                same += 1;
            }
            if let [a, b, ..] = v[..] {
                xy += f64::from(a) * f64::from(b);
                xx += f64::from(a) * f64::from(a);
                yy += f64::from(b) * f64::from(b);
            }
            frames += 1;
            if (i + 1) % block == 0 {
                for (c, s) in sum_sq.iter_mut().enumerate() {
                    blocks[c].push(db(*s / block as f64));
                    *s = 0.0;
                }
            }
        }
        Self {
            format,
            seconds: frames as f32 / format.rate as f32,
            blocks,
            identical: same as f32 / frames.max(1) as f32,
            correlation: (xy / (xx * yy).sqrt().max(1e-12)) as f32,
        }
    }

    /// The median level of the channel's blocks: what it holds most of the time.
    fn median(&self, c: usize) -> f32 {
        let mut v = self.blocks[c].clone();
        v.sort_by(f32::total_cmp);
        percentile(&v, 0.5)
    }

    /// The first block of any channel at `SOUND_DB` or above, as seconds.
    fn first_sound(&self) -> Option<f32> {
        (0..self.blocks.first()?.len())
            .find(|&b| self.blocks.iter().any(|c| c[b] >= SOUND_DB))
            .map(|b| (b * BLOCK_MS) as f32 / 1_000.0)
    }

    fn describe(&self, name: &str, out: &mut String) {
        let f = self.format;
        let _ = writeln!(
            out,
            "{name}: {} ch {} Hz {}, {:.1} s; frames with all channels equal {:.1} %, L/R correlation {:.3}",
            f.channels,
            f.rate,
            f.sample.alsa(),
            self.seconds,
            self.identical * 100.0,
            self.correlation
        );
        for (c, blocks) in self.blocks.iter().enumerate() {
            let mut v = blocks.clone();
            v.sort_by(f32::total_cmp);
            let _ = writeln!(
                out,
                "  ch{c}: median {:.1} dBFS, 5th {:.1}, 95th {:.1}, loudest {:.1}",
                percentile(&v, 0.5),
                percentile(&v, 0.05),
                percentile(&v, 0.95),
                v.last().copied().unwrap_or(f32::NAN)
            );
        }
    }

    fn table(&self, name: &str, n: usize, out: &mut String) {
        let _ = writeln!(
            out,
            "{name}, the first {n} blocks of {BLOCK_MS} ms (dBFS per channel):"
        );
        for b in 0..n.min(self.blocks[0].len()) {
            let row: Vec<String> = self
                .blocks
                .iter()
                .map(|c| format!("{:7.1}", c[b]))
                .collect();
            let _ = writeln!(out, "  {:5} ms {}", b * BLOCK_MS, row.join(" "));
        }
    }

    /// The microphone's open pop: how long into the file a channel stays 12 dB over its usual level, against what is dropped.
    fn pop(&self, skip_ms: usize, out: &mut String) {
        let first_second = 1_000 / BLOCK_MS;
        for (c, blocks) in self.blocks.iter().enumerate() {
            let usual = self.median(c);
            let last = blocks
                .iter()
                .take(first_second)
                .rposition(|&l| l > usual + 12.0);
            let _ = match last {
                Some(b) => writeln!(
                    out,
                    "  ch{c} open pop: last block over median+12 dB ends at {} ms ({:.1} dBFS, median {:.1}); skip {} ms {}",
                    (b + 1) * BLOCK_MS,
                    blocks[b],
                    usual,
                    skip_ms,
                    if (b + 1) * BLOCK_MS <= skip_ms {
                        "covers it"
                    } else {
                        "DOES NOT cover it"
                    }
                ),
                None => writeln!(
                    out,
                    "  ch{c} open pop: none over median+12 dB in the first second"
                ),
            };
        }
    }
}

/// What a wake word model made of a stream.
#[derive(Default)]
struct Wake {
    fed: usize,
    /// Highest score in each window.
    windows: Vec<f32>,
    /// Seconds of each detection that passed its cutoff and the strictness.
    wakes: Vec<(f32, f32)>,
    refused: u64,
}

struct Listener {
    engine: Engine,
    wake: Wake,
    window: usize,
}

impl Listener {
    fn new(opts: &Options) -> Result<Self, String> {
        let word = models::load(&opts.word, opts.cutoff)?;
        let mut engine = Engine::new(EngineConfig::default(), vec![word])?;
        engine.set_strictness(opts.extra);
        Ok(Self {
            engine,
            wake: Wake::default(),
            window: (opts.window_secs * RATE as f32) as usize,
        })
    }

    fn feed(&mut self, samples: &[i16]) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        let mut out = Vec::new();
        self.engine.push(samples, false, &mut out)?;
        self.wake.fed += samples.len();
        for o in &out {
            if let Out::Wake { score, .. } = o {
                self.wake
                    .wakes
                    .push((self.wake.fed as f32 / RATE as f32, *score));
                // Every detection counts: the turn that follows is not waited for.
                self.engine.end(EndReason::NoAck, &mut Vec::new());
            }
        }
        if self.wake.fed / self.window > self.wake.windows.len() {
            self.wake.windows.push(self.engine.take_score_peak());
        }
        Ok(())
    }

    fn finish(mut self) -> Wake {
        self.wake.windows.push(self.engine.take_score_peak());
        self.wake.refused = self.engine.refused();
        self.wake
    }

    fn describe(wake: &Wake, name: &str, out: &mut String) {
        let windows: Vec<String> = wake.windows.iter().map(|s| format!("{s:.2}")).collect();
        let times: Vec<String> = wake
            .wakes
            .iter()
            .map(|(t, s)| format!("{t:.1}s({s:.3})"))
            .collect();
        let _ = writeln!(
            out,
            "  {name}: {} wakes {}, {} refused by the strictness; best score per window: {}",
            wake.wakes.len(),
            times.join(" "),
            wake.refused,
            windows.join(" ")
        );
    }
}

/// Sample-by-sample figures of one block's cost.
#[derive(Default)]
struct Cost {
    canceller_us: Vec<f32>,
    decode_us: Vec<f32>,
}

impl Cost {
    fn describe(&self, name: &str, v: &[f32], out: &mut String) {
        let mut s = v.to_vec();
        s.sort_by(f32::total_cmp);
        let mean = s.iter().sum::<f32>() / s.len().max(1) as f32;
        let _ = writeln!(
            out,
            "  {name}: mean {mean:.0} us per {STEP_MS} ms block ({:.2} % of a core), p50 {:.0}, p99 {:.0}, max {:.0}",
            mean / (STEP_MS as f32 * 10.0),
            percentile(&s, 0.5),
            percentile(&s, 0.99),
            s.last().copied().unwrap_or(f32::NAN)
        );
    }
}

struct Report {
    text: String,
    lag_ms: Option<f32>,
    lock_secs: Option<f32>,
    erle_db: Option<f32>,
}

fn power(samples: &[i16]) -> f64 {
    samples
        .iter()
        .map(|&s| (f64::from(s) / 32_768.0).powi(2))
        .sum::<f64>()
        / samples.len().max(1) as f64
}

fn replay(mic: &[u8], reference: &[u8], opts: &Options) -> Result<Report, String> {
    let mut text = String::new();
    let mic_stream = Stream::read(mic, opts.mic_format);
    let ref_stream = Stream::read(reference, opts.ref_format);
    mic_stream.describe("microphone", &mut text);
    mic_stream.pop(opts.skip_ms, &mut text);
    ref_stream.describe("reference", &mut text);
    let ref_sound = ref_stream.first_sound();
    let _ = writeln!(
        text,
        "  reference sound (>= {SOUND_DB} dBFS) from {}",
        ref_sound.map_or("never".into(), |t| format!("{t:.2} s"))
    );
    if opts.rms_blocks > 0 {
        mic_stream.table("microphone", opts.rms_blocks, &mut text);
        ref_stream.table("reference", opts.rms_blocks, &mut text);
    }

    opts.echo.check()?;
    let mut mic_frames =
        Frames::new(opts.mic_format, opts.gain_db, opts.pick).map_err(|e| e.to_string())?;
    let mut ref_frames =
        Frames::new(opts.ref_format, 0.0, opts.ref_pick).map_err(|e| e.to_string())?;
    let mut canceller = Canceller::new(&opts.echo);
    let mut raw_ear = Listener::new(opts)?;
    let mut clean_ear = Listener::new(opts)?;

    let mic_step =
        opts.mic_format.frame_bytes() * (opts.mic_format.rate as usize * STEP_MS / 1_000);
    let ref_step =
        opts.ref_format.frame_bytes() * (opts.ref_format.rate as usize * STEP_MS / 1_000);
    let open_step = opts
        .ref_open_ms
        .unwrap_or(0)
        .max(opts.skip_ms)
        .div_ceil(STEP_MS);
    let ref_blocks: Vec<&[u8]> = reference.chunks(ref_step).collect();

    let (mut heard, mut clean, mut ref_mono) = (Vec::new(), Vec::new(), Vec::new());
    let mut skip = opts.skip_ms * RATE / 1_000;
    let mut cost = Cost::default();
    let mut changes: Vec<(f32, EchoState, Option<f32>)> = Vec::new();
    let mut last_state = EchoState::Off;
    let mut lock_at: Option<f32> = None;
    let mut lag_ms = None;
    let mut max_held = 0.0f32;
    let mut ref_at = 0usize;
    let mut open = false;

    for (k, mic_block) in mic.chunks(mic_step).enumerate() {
        let mut out = Vec::new();
        let (mut decode, mut cancel) = (0.0f32, 0.0f32);
        if k == open_step {
            canceller.open_reference(&mut out);
            open = true;
            ref_at = heard.len();
        }
        let t = Instant::now();
        let mut mono = Vec::new();
        mic_frames.push(mic_block, &mut mono);
        let drop = skip.min(mono.len());
        mono.drain(..drop);
        skip -= drop;
        decode += t.elapsed().as_secs_f32();
        if !mono.is_empty() {
            let t = Instant::now();
            canceller.push_mic(&mono, &mut out);
            cancel += t.elapsed().as_secs_f32();
            heard.extend_from_slice(&mono);
            raw_ear.feed(&mono)?;
        }
        if open && let Some(block) = ref_blocks.get(k) {
            let t = Instant::now();
            let mut r = Vec::new();
            ref_frames.push(block, &mut r);
            decode += t.elapsed().as_secs_f32();
            let t = Instant::now();
            canceller.push_reference(&r, &mut out);
            cancel += t.elapsed().as_secs_f32();
            ref_mono.extend_from_slice(&r);
        }
        if open {
            cost.canceller_us.push(cancel * 1e6);
            cost.decode_us.push(decode * 1e6);
        }
        clean.extend_from_slice(&out);
        clean_ear.feed(&out)?;

        let status = canceller.status();
        max_held = max_held.max(status.held_ms);
        let now = heard.len() as f32 / RATE as f32 + opts.skip_ms as f32 / 1_000.0;
        if status.state != last_state {
            changes.push((now, status.state, status.lag_ms));
            last_state = status.state;
        }
        if status.state == EchoState::Cancelling && lock_at.is_none() {
            lock_at = Some(now);
            lag_ms = status.lag_ms;
        }
    }
    // The figures are read before the reference is closed: closing it ends the lock.
    let status = canceller.status();
    let credit = canceller.credit_db();
    let mut tail = Vec::new();
    canceller.close_reference(&mut tail);
    clean.extend_from_slice(&tail);
    clean_ear.feed(&tail)?;

    // The lag, the lock and the filter's own figures.
    let _ = writeln!(text, "canceller (enabled {}):", opts.echo.enabled);
    let _ = writeln!(
        text,
        "  reference opened at {:.2} s of file time; reference sound from {}",
        (open_step * STEP_MS) as f32 / 1_000.0,
        ref_sound.map_or("never".into(), |t| format!("{t:.2} s"))
    );
    for (t, state, lag) in changes.iter().take(24) {
        let _ = writeln!(
            text,
            "  {t:6.2} s  {state:?}{}",
            lag.map_or(String::new(), |l| format!("  lag {l:.2} ms"))
        );
    }
    if changes.len() > 24 {
        let _ = writeln!(text, "  ... {} more changes", changes.len() - 24);
    }
    let lock_secs = lock_at.zip(ref_sound).map(|(l, s)| l - s.max(0.0));
    let _ = writeln!(
        text,
        "  lag {}  locks {}  confidence {:.1}  lock {} after the reference's first sound  longest hold {max_held:.0} ms  final state {:?}",
        lag_ms.map_or("none".into(), |l| format!("{l:.2} ms")),
        status.locks,
        status.confidence,
        lock_secs.map_or("never".into(), |s| format!("{s:.1} s")),
        status.state
    );
    let _ = writeln!(
        text,
        "  the canceller's own ERLE {} dB, credit {:.1} dB",
        status.erle_db.map_or("none".into(), |e| format!("{e:.1}")),
        credit
    );

    // ERLE from the two signals themselves, over the seconds the filter has had to settle.
    let n = heard.len().min(clean.len());
    let mut quiet: Vec<f32> = heard[..n].chunks(RATE / 10).map(|c| db(power(c))).collect();
    quiet.sort_by(f32::total_cmp);
    let floor = opts.floor_db.unwrap_or_else(|| percentile(&quiet, 0.05));
    let from = lock_at.map_or(f32::MAX, |l| l + opts.settle_secs);
    let lag_samples = lag_ms.map_or(0, |l| (l * RATE as f32 / 1_000.0).round() as i64);
    let (mut sum_in, mut sum_out, mut series, mut per_window) =
        (0.0f64, 0.0f64, Vec::new(), Vec::new());
    let mut start = 0;
    while start + RATE <= n {
        let t = (start + opts.skip_ms * RATE / 1_000) as f32 / RATE as f32;
        let (pin, pout) = (
            power(&heard[start..start + RATE]),
            power(&clean[start..start + RATE]),
        );
        let far = (start as i64 - ref_at as i64 - lag_samples)
            .try_into()
            .ok()
            .and_then(|r: usize| ref_mono.get(r..r + RATE))
            .map(power);
        if t >= from && db(pin) > floor + 6.0 && far.is_some_and(|f| db(f) > -60.0) {
            sum_in += pin;
            sum_out += pout;
            let e = db(pin) - db(pout);
            per_window.push(e);
            series.push(format!("{t:.0}s:{e:.1}({:.0})", db(pin)));
        }
        start += RATE;
    }
    per_window.sort_by(f32::total_cmp);
    let erle = (sum_in > 0.0).then(|| db(sum_in) - db(sum_out));
    let _ = writeln!(
        text,
        "ERLE (mic floor {floor:.1} dBFS after gain; windows of 1 s from lock+{:.0} s that are 6 dB over it with the reference above -60 dBFS): {} over {} windows; per window p10 {:.1}, median {:.1}, p90 {:.1}",
        opts.settle_secs,
        erle.map_or("no qualifying window".into(), |e| format!("{e:.1} dB")),
        per_window.len(),
        percentile(&per_window, 0.1),
        percentile(&per_window, 0.5),
        percentile(&per_window, 0.9)
    );
    if !series.is_empty() {
        let _ = writeln!(text, "  {}", series.join("  "));
    }

    let (raw, cleaned) = (raw_ear.finish(), clean_ear.finish());
    let _ = writeln!(
        text,
        "wake word {} at cutoff {} + {} ({} s windows):",
        opts.word, opts.cutoff, opts.extra, opts.window_secs
    );
    Listener::describe(&raw, "microphone as it is", &mut text);
    Listener::describe(&cleaned, "after the canceller", &mut text);
    let _ = writeln!(
        text,
        "cost on this machine (the TV's A53 is several times slower):"
    );
    cost.describe("canceller", &cost.canceller_us, &mut text);
    cost.describe("decode", &cost.decode_us, &mut text);

    if let Some(path) = &opts.write_clean {
        let bytes: Vec<u8> = clean.iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(path, bytes).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(Report {
        text,
        lag_ms,
        lock_secs,
        erle_db: erle,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }
    let run = || -> Result<Report, String> {
        let (mic, reference, opts) = parse_args(&args)?;
        let mic = std::fs::read(&mic).map_err(|e| format!("{mic}: {e}"))?;
        let reference = std::fs::read(&reference).map_err(|e| format!("{reference}: {e}"))?;
        replay(&mic, &reference, &opts)
    };
    match run() {
        Ok(report) => {
            print!("{}", report.text);
            let figure = |v: Option<f32>| v.map_or("none".into(), |v| format!("{v:.1}"));
            println!(
                "summary lag_ms={} lock_s={} erle_db={}",
                figure(report.lag_ms),
                figure(report.lock_secs),
                figure(report.erle_db)
            );
        }
        Err(e) => {
            eprintln!("echo_replay: {e}\n\n{HELP}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wake_word::wav;

    const SPEECH: &[u8] = include_bytes!("../tests/data/speech_no_wake.wav");
    const JARVIS: &[u8] = include_bytes!("../tests/data/hey_jarvis_hazel.wav");
    /// The echo trails the reference by this much.
    const DELAY: usize = 2_880;

    /// A programme, and the two capture files of the TV playing it: the microphone hears it late, through three reflections, over a
    /// little noise; the reference is stereo at 48 kHz with the left channel dead, as the TV's mixer was seen to give it.
    fn capture() -> (Vec<u8>, Vec<u8>) {
        let mut sound = Vec::new();
        for clip in [SPEECH, JARVIS] {
            sound.extend(wav::read(clip).unwrap().samples);
        }
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ((seed >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 40.0
        };
        let mut mic = vec![0.0f32; 16_000 + sound.len() + DELAY];
        mic.iter_mut().for_each(|s| *s = noise());
        for (at, gain) in [(0, 0.3f32), (90, 0.12), (700, -0.08)] {
            for (slot, &s) in mic[16_000 + DELAY + at..].iter_mut().zip(&sound) {
                *slot += gain * f32::from(s);
            }
        }
        let mic_bytes = mic
            .iter()
            .flat_map(|&v| {
                let s = (v.round().clamp(-32_768.0, 32_767.0) as i32) << 8;
                [s, s]
            })
            .flat_map(i32::to_le_bytes)
            .collect();
        // A second of silence, then the programme: the reference stream carries the programme from its first sample.
        let mut played = vec![0i16; 16_000];
        played.extend(&sound);
        let ref_bytes = played
            .iter()
            .flat_map(|&s| [0i16, s, 0, s, 0, s])
            .flat_map(i16::to_le_bytes)
            .collect();
        (mic_bytes, ref_bytes)
    }

    #[test]
    fn a_capture_of_a_delayed_echo_is_locked_and_taken_out() {
        let (mic, reference) = capture();
        let opts = Options {
            gain_db: 0.0,
            settle_secs: 1.0,
            // The auto pick would take the reference as the sum of both channels for its first seconds and switch at about 7 s: the
            // filter sees the level double. A mapping that is known is pinned.
            ref_pick: Pick::Channel(1),
            ..Options::default()
        };
        let report = replay(&mic, &reference, &opts).unwrap();
        let lag = report.lag_ms.expect("a lag is locked");
        // The 3 ms of difference is the up and down conversion of the reference and the strongest reflection's own offset.
        assert!(
            (lag - DELAY as f32 / 16.0).abs() < 3.0,
            "lag {lag} ms\n{}",
            report.text
        );
        assert!(report.lock_secs.unwrap() < 5.0, "{}", report.text);
        let erle = report.erle_db.expect("a window is measured");
        assert!(erle > 15.0, "ERLE {erle} dB\n{}", report.text);
    }

    #[test]
    fn a_file_pair_that_is_not_two_files_is_refused() {
        assert!(parse_args(&["a.raw".into()]).is_err());
        assert!(parse_args(&["a".into(), "b".into(), "--mu".into(), "x".into()]).is_err());
        assert!(
            parse_args(&[
                "a".into(),
                "b".into(),
                "--ref-format".into(),
                "s16:2:48000".into()
            ])
            .is_ok()
        );
    }
}
