//! What 16 kHz mono audio makes: the preroll, the wake word, and the turn that follows it. No I/O.
//!
//! **No microphone sample leaves before a wake word, and then only after the client has said it wants the turn** ([`Engine::accept`],
//! within [`EngineConfig::ack_secs`] of the [`Out::Wake`]). The turn ends when the client says so, at [`EngineConfig::turn_secs`],
//! at a mute, or when the client goes. The preroll (the second before the wake) is held in RAM and goes with the turn; it is never
//! written anywhere. Times are audio time (samples counted), so they hold however the reader is scheduled.

use crate::pcm::{RATE, peak_dbfs};
use voice_dsp::preroll::Preroll;
use voice_wire::EndReason;
use wake_word::Model;
use wake_word::detector::{Event as WakeEvent, Listener, Manifest};

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub preroll_secs: f32,
    /// How long the client has to take a wake.
    pub ack_secs: f32,
    /// The longest a turn is.
    pub turn_secs: f32,
    /// Samples in one `Audio` message (80 ms is 2560 bytes: inside the frame limit).
    pub chunk_samples: usize,
    /// How long a detection mutes the model that made it.
    pub refractory_ms: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            preroll_secs: 1.0,
            ack_secs: 3.0,
            turn_secs: 30.0,
            chunk_samples: RATE as usize * 80 / 1000,
            refractory_ms: 2000,
        }
    }
}

/// A wake word the engine listens for. `manifest.probability_cutoff` is the cutoff it decides on.
pub struct Word {
    pub id: String,
    pub model: Model,
    pub manifest: Manifest,
}

/// What the engine says happened.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Wake {
        word: String,
        score: f32,
        level: f32,
    },
    Audio(Vec<i16>),
    Ended(EndReason),
}

enum Turn {
    None,
    /// The wake was heard; nothing has left. `head` is the preroll as it was at the wake, `tail` what came after.
    Awaiting {
        head: Vec<i16>,
        tail: Vec<i16>,
        since_wake: u64,
    },
    Streaming {
        since_wake: u64,
    },
}

pub struct Engine {
    cfg: EngineConfig,
    preroll: Preroll,
    listener: Listener,
    ids: Vec<String>,
    cutoffs: Vec<f32>,
    /// What is asked of a detection beyond its cutoff (the TV is loud).
    extra: f32,
    /// The listener has not seen the last stretch of audio: it starts again at the next one it does.
    gap: bool,
    turn: Turn,
    peak: i16,
    score_peak: f32,
    refused: u64,
}

impl Engine {
    pub fn new(cfg: EngineConfig, words: Vec<Word>) -> Result<Self, String> {
        let mut listener = Listener::new();
        let mut ids = Vec::new();
        let mut cutoffs = Vec::new();
        for w in &words {
            listener
                .add(&w.model, &w.manifest, cfg.refractory_ms)
                .map_err(|e| format!("the model {}: {e}", w.id))?;
            ids.push(w.id.clone());
            cutoffs.push(w.manifest.probability_cutoff);
        }
        Ok(Self {
            preroll: Preroll::new((cfg.preroll_secs * RATE as f32) as usize),
            listener,
            ids,
            cutoffs,
            extra: 0.0,
            gap: false,
            turn: Turn::None,
            peak: 0,
            score_peak: 0.0,
            refused: 0,
            cfg,
        })
    }

    pub fn words(&self) -> &[String] {
        &self.ids
    }

    pub fn in_turn(&self) -> bool {
        !matches!(self.turn, Turn::None)
    }

    /// Asks more of every detection: a model's average must pass its cutoff plus `extra` (0 to 0.03 is the range that matters).
    pub fn set_strictness(&mut self, extra: f32) {
        self.extra = extra.clamp(0.0, 0.05);
    }

    /// The largest absolute sample since the last call (a level, never audio).
    pub fn take_peak(&mut self) -> i16 {
        std::mem::take(&mut self.peak)
    }

    /// The highest average the wake model's window reached since the last call (0 to 1): how close the room came to the word.
    pub fn take_score_peak(&mut self) -> f32 {
        std::mem::take(&mut self.score_peak)
    }

    /// Detections that passed their cutoff but not the strictness asked on top of it.
    pub fn refused(&self) -> u64 {
        self.refused
    }

    fn ack_samples(&self) -> u64 {
        (self.cfg.ack_secs * RATE as f32) as u64
    }

    fn turn_samples(&self) -> u64 {
        (self.cfg.turn_secs * RATE as f32) as u64
    }

    fn audio(&self, samples: &[i16], out: &mut Vec<Out>) {
        for chunk in samples.chunks(self.cfg.chunk_samples.max(1)) {
            out.push(Out::Audio(chunk.to_vec()));
        }
    }

    /// Audio the wake word is not to be listened for in (`deaf`: another reader of the microphone has it, as the voice search of
    /// the mic key does). An open turn goes on; what the model missed is a gap, which it starts again after.
    pub fn push(&mut self, samples: &[i16], deaf: bool, out: &mut Vec<Out>) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        self.peak = samples
            .iter()
            .fold(self.peak, |p, s| p.max(s.unsigned_abs().min(32_767) as i16));
        let mut heard: Option<(usize, f32)> = None;
        let was_open = self.in_turn();
        if deaf || was_open {
            // A turn is the person talking, not the wake word being said: the model is not fed, and it starts clean afterwards.
            self.gap = true;
        } else {
            if std::mem::take(&mut self.gap) {
                self.listener.reset();
                self.preroll.clear();
            }
            let mut score_peak = 0.0f32;
            let (cutoffs, extra) = (&self.cutoffs, self.extra);
            let mut refused = 0;
            self.listener
                .push(samples, |e| match e {
                    WakeEvent::Detected { slot, average, .. } => {
                        if average >= cutoffs.get(slot).copied().unwrap_or(1.0) + extra {
                            heard.get_or_insert((slot, average));
                        } else {
                            refused += 1;
                        }
                    }
                    WakeEvent::Score { average, .. } => score_peak = score_peak.max(average),
                })
                .map_err(|e| e.to_string())?;
            self.refused += refused;
            self.score_peak = self.score_peak.max(score_peak);
        }
        self.preroll.push(samples);
        let (ack, limit) = (self.ack_samples(), self.turn_samples());
        match &mut self.turn {
            Turn::None => {
                if let Some((slot, score)) = heard {
                    let head = self.preroll.snapshot();
                    let level = peak_dbfs(&head);
                    self.turn = Turn::Awaiting {
                        head,
                        tail: Vec::new(),
                        since_wake: 0,
                    };
                    out.push(Out::Wake {
                        word: self.ids.get(slot).cloned().unwrap_or_default(),
                        score,
                        level,
                    });
                }
            }
            Turn::Awaiting {
                tail, since_wake, ..
            } if was_open => {
                tail.extend_from_slice(samples);
                *since_wake += samples.len() as u64;
                if *since_wake >= ack {
                    self.turn = Turn::None;
                    out.push(Out::Ended(EndReason::NoAck));
                }
            }
            Turn::Streaming { since_wake } if was_open => {
                *since_wake += samples.len() as u64;
                let over = *since_wake >= limit;
                self.audio(samples, out);
                if over {
                    self.turn = Turn::None;
                    out.push(Out::Ended(EndReason::Limit));
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The client takes the turn: the second before the wake and what has come since go up, then the live audio.
    pub fn accept(&mut self, out: &mut Vec<Out>) {
        if let Turn::Awaiting {
            head,
            tail,
            since_wake,
        } = std::mem::replace(&mut self.turn, Turn::None)
        {
            self.audio(&head, out);
            self.audio(&tail, out);
            self.turn = Turn::Streaming { since_wake };
        }
    }

    /// Opens a turn with no wake word, for a conversation that goes on (the client has a question's answer to hear): the audio goes
    /// up from now, with no preroll (what came before was the reply being spoken).
    pub fn open(&mut self) {
        if matches!(self.turn, Turn::None) {
            self.turn = Turn::Streaming { since_wake: 0 };
        }
    }

    /// Ends the turn, if one is open.
    pub fn end(&mut self, why: EndReason, out: &mut Vec<Out>) {
        if self.in_turn() {
            self.turn = Turn::None;
            out.push(Out::Ended(why));
        }
    }
}
