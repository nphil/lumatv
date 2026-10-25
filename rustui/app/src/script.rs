//! The remote, scripted: the D-pad sweeps of `docs/research/perf/gate.py` (`home-dpad`, `home-sweep`) as timed key presses.

use crate::home::Key;

/// A key press `at_s` seconds into the phase.
#[derive(Clone, Copy, Debug)]
pub struct Press {
    pub at_s: f64,
    pub key: Key,
}

/// `RIGHTx8@0.35 LEFTx8@0.35 UP s0.5 DOWN`: sixteen steps along the dock at the pace of a thumb, then up into the cards and back.
pub fn dpad(offset: f64) -> (Vec<Press>, f64) {
    let mut out = Vec::new();
    let mut t = offset;
    for _ in 0..8 {
        out.push(Press { at_s: t, key: Key::Right });
        t += 0.35;
    }
    for _ in 0..8 {
        out.push(Press { at_s: t, key: Key::Left });
        t += 0.35;
    }
    out.push(Press { at_s: t, key: Key::Up });
    t += 0.5;
    out.push(Press { at_s: t, key: Key::Down });
    (out, t + 0.5 - offset)
}

/// `RIGHT:1500 s0.4 LEFT:1500 s0.4 UP s0.6 DOWN`: a held key repeats as the remote's own does (first repeat after 300 ms, then every
/// 100 ms).
pub fn sweep(offset: f64) -> (Vec<Press>, f64) {
    let mut out = Vec::new();
    let mut t = offset;
    for key in [Key::Right, Key::Left] {
        out.push(Press { at_s: t, key });
        let mut r = 0.3;
        while r <= 1.5 + 1e-9 {
            out.push(Press { at_s: t + r, key });
            r += 0.1;
        }
        t += 1.5 + 0.4;
    }
    out.push(Press { at_s: t, key: Key::Up });
    t += 0.6;
    out.push(Press { at_s: t, key: Key::Down });
    (out, t + 0.5 - offset)
}

/// A named script, repeated to fill `seconds`.
pub fn build(name: &str, seconds: f64) -> Vec<Press> {
    let mut out = Vec::new();
    let mut at = 0.0;
    let mut turn = 0;
    while at < seconds {
        let (mut presses, length) = match (name, turn % 2) {
            ("dpad", _) | ("mix", 0) => dpad(at),
            ("sweep", _) | ("mix", _) => sweep(at),
            _ => return out,
        };
        presses.retain(|p| p.at_s < seconds);
        out.extend(presses);
        at += length;
        turn += 1;
    }
    out
}
