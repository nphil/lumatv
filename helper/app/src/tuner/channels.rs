//! The page's list of channels and what is kept of them. Each antenna channel is a row under its RF channel (the channels that share a
//! signal come in alike): how it came in the last time it was streamed or checked, and a switch that keeps it in the lineup players see.
//! `tuner/channels.json` keeps the channels left out and each channel's last reception and picture, given back to the engine at the
//! start so that a channel's first slate is already its shape.

use lumatuner::mpeg2::Format;
use lumatuner::{Health, Reception, Signal, Tuner, Verdict};
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The provider of the page's list.
pub const PROVIDER: &str = "channels";
/// The action of a row's switch: the channel in the lineup or out of it.
pub const LINEUP: &str = "lineup";

/// What `tuner/channels.json` keeps: the channels left out, and the rest of the file as it was read (each channel's last reception).
pub struct Kept {
    pub hidden: BTreeSet<String>,
    channels: Map<String, Json>,
}

impl Kept {
    /// The file as it is, or nothing kept when there is none (or it is not the file's shape).
    pub fn read(path: &Path) -> Self {
        let doc: Json = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self {
            hidden: doc["hidden"].as_array().into_iter().flatten().filter_map(Json::as_str).map(str::to_owned).collect(),
            channels: doc["channels"].as_object().cloned().unwrap_or_default(),
        }
    }

    /// Gives the engine back what was kept.
    pub fn restore(&self, tuner: &Tuner) {
        tuner.hide(self.hidden.iter().cloned());
        for (guide, kept) in &self.channels {
            let sounds = kept["sounds"].as_u64().map_or(1, |n| n as usize);
            tuner.recall(guide, health_of(&kept["health"]), format_of(&kept["format"]), sounds);
        }
    }

    /// Takes in what the engine knows of its channels now: a channel it lists is kept as it says; one it does not list is forgotten,
    /// unless it lists none (it is not serving). `true` when that changed what is kept.
    pub fn update(&mut self, reception: &[Reception]) -> bool {
        if reception.is_empty() {
            return false;
        }
        let channels: Map<String, Json> = reception
            .iter()
            .filter(|r| r.health.is_some() || r.format.is_some())
            .map(|r| {
                let mut kept = Map::new();
                if let Some(h) = &r.health {
                    kept.insert("health".to_owned(), health_json(h));
                }
                if let Some(f) = &r.format {
                    kept.insert("format".to_owned(), json!({"width": f.width, "height": f.height, "rate": f.rate, "aspect": f.aspect, "progressive": f.progressive}));
                    kept.insert("sounds".to_owned(), json!(r.sounds));
                }
                (r.guide.clone(), Json::Object(kept))
            })
            .collect();
        let changed = channels != self.channels;
        self.channels = channels;
        changed
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let doc = json!({"v": 1, "hidden": self.hidden, "channels": self.channels});
        crate::hue_store::write_private(path, doc.to_string().as_bytes(), 0o600)
    }
}

fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Good => "good",
        Verdict::Glitchy => "glitchy",
        Verdict::Weak => "weak",
        Verdict::NoSignal => "no-signal",
    }
}

fn health_json(h: &Health) -> Json {
    let at = h.at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let mut doc = json!({"at": at, "verdict": verdict_word(h.verdict), "lost": h.lost, "dropouts": h.dropouts, "checked": h.checked});
    if let Some(s) = &h.signal {
        doc["signal"] = json!({"locked": s.locked, "snr": s.snr, "errors": s.errors});
    }
    if let Some(p) = h.picture {
        doc["pictureMs"] = json!(p.as_millis() as u64);
    }
    doc
}

fn health_of(doc: &Json) -> Option<Health> {
    let verdict = match doc["verdict"].as_str()? {
        "good" => Verdict::Good,
        "glitchy" => Verdict::Glitchy,
        "weak" => Verdict::Weak,
        "no-signal" => Verdict::NoSignal,
        _ => return None,
    };
    let signal = doc.get("signal").filter(|s| s.is_object()).map(|s| Signal {
        locked: s["locked"].as_bool().unwrap_or(false),
        snr: s["snr"].as_f64().map(|v| v as f32),
        errors: s["errors"].as_u64().map(|v| v as u32),
    });
    Some(Health {
        at: UNIX_EPOCH + Duration::from_millis(doc["at"].as_u64()?),
        verdict,
        signal,
        lost: doc["lost"].as_f64().unwrap_or(0.0) as f32,
        picture: doc["pictureMs"].as_u64().map(Duration::from_millis),
        dropouts: doc["dropouts"].as_u64().unwrap_or(0) as u32,
        checked: doc["checked"].as_bool().unwrap_or(false),
    })
}

fn format_of(doc: &Json) -> Option<Format> {
    Some(Format {
        width: u16::try_from(doc["width"].as_u64()?).ok()?,
        height: u16::try_from(doc["height"].as_u64()?).ok()?,
        rate: u8::try_from(doc["rate"].as_u64()?).ok()?,
        aspect: u8::try_from(doc["aspect"].as_u64()?).ok()?,
        progressive: doc["progressive"].as_bool()?,
    })
}

/// How long ago, in words.
fn ago(then: SystemTime, now: SystemTime) -> String {
    let s = now.duration_since(then).unwrap_or_default().as_secs();
    match s {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        _ if s < 2 * 86_400 => "yesterday".to_owned(),
        _ => format!("{} days ago", s / 86_400),
    }
}

/// A row's status line: how the channel came in, in words, and the tone of its dot.
fn status(health: Option<&Health>, now: SystemTime) -> Json {
    let Some(h) = health else {
        return json!({"text": "Not checked yet", "tone": "muted", "ring": true});
    };
    let (word, tone) = match h.verdict {
        Verdict::Good => ("Good", "good"),
        Verdict::Glitchy => ("Some glitches", "warn"),
        Verdict::Weak => ("Weak", "warn"),
        Verdict::NoSignal => ("No signal", "bad"),
    };
    let mut words = vec![word.to_owned()];
    if let Some(snr) = h.signal.and_then(|s| s.snr) {
        words.push(format!("{snr:.0} dB"));
    }
    if h.lost >= 0.001 {
        words.push(if h.lost < 0.01 { format!("{:.1}% lost", h.lost * 100.0) } else { format!("{:.0}% lost", h.lost * 100.0) });
    }
    if h.dropouts > 0 {
        words.push(if h.dropouts == 1 { "1 dropout".to_owned() } else { format!("{} dropouts", h.dropouts) });
    }
    words.push(format!("{} {}", if h.checked { "checked" } else { "watched" }, ago(h.at, now)));
    json!({"text": words.join(" · "), "tone": tone})
}

/// The list's rows, under their RF channels in order: a channel's number and name, how it came in, and its switch.
pub fn rows(reception: &[Reception], now: SystemTime) -> Vec<Json> {
    let mut sorted: Vec<&Reception> = reception.iter().collect();
    let number = |g: &str| -> (u32, u32) {
        let (major, minor) = g.split_once('.').unwrap_or((g, "0"));
        (major.parse().unwrap_or(u32::MAX), minor.parse().unwrap_or(0))
    };
    sorted.sort_by_key(|r| (r.rf, number(&r.guide)));
    sorted
        .into_iter()
        .map(|r| {
            json!({
                "id": r.guide,
                "label": format!("{} {}", r.guide, r.name).trim(),
                "group": format!("RF {}", r.rf),
                "item": {
                    "tile": "base.icons/antenna",
                    "status": status(r.health.as_ref(), now),
                    "switch": {"on": !r.hidden, "action": LINEUP},
                    "muted": r.hidden,
                }
            })
        })
        .collect()
}

/// One page of the rows.
pub fn page(reception: &[Reception], offset: u32, limit: u16, now: SystemTime) -> Json {
    let rows = rows(reception, now);
    let from = (offset as usize).min(rows.len());
    let to = (from + usize::from(limit.max(1))).min(rows.len());
    json!({"offset": offset, "rows": rows[from..to], "more": to < rows.len(), "state": "ready"})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reception(guide: &str, rf: u8, health: Option<Health>) -> Reception {
        Reception {
            guide: guide.to_owned(),
            name: format!("W{rf}"),
            rf,
            hidden: false,
            health,
            format: None,
            sounds: 0,
        }
    }

    #[test]
    fn rows_say_how_each_channel_came_in_under_its_rf_channel_with_its_lineup_switch() {
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let weak = Health {
            at: now - Duration::from_secs(12 * 60),
            verdict: Verdict::Weak,
            signal: Some(Signal { locked: true, snr: Some(18.4), errors: Some(3) }),
            lost: 0.04,
            picture: Some(Duration::from_millis(2100)),
            dropouts: 2,
            checked: false,
        };
        let mut hidden = reception("30.1", 29, Some(Health { verdict: Verdict::NoSignal, signal: None, lost: 0.0, dropouts: 0, checked: true, ..weak }));
        hidden.hidden = true;
        let all = [reception("16.9", 29, Some(weak)), reception("2.1", 32, None), hidden];
        let rows = rows(&all, now);
        let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["16.9", "30.1", "2.1"]);
        assert_eq!(rows[0]["label"], "16.9 W29");
        assert_eq!(rows[0]["group"], "RF 29");
        assert_eq!(rows[0]["item"]["status"], json!({"text": "Weak · 18 dB · 4% lost · 2 dropouts · watched 12 min ago", "tone": "warn"}));
        assert_eq!(rows[1]["item"]["status"]["text"], "No signal · checked 12 min ago");
        assert_eq!((rows[1]["item"]["switch"]["on"].clone(), rows[1]["item"]["muted"].clone()), (json!(false), json!(true)));
        assert_eq!(rows[2]["item"]["status"], json!({"text": "Not checked yet", "tone": "muted", "ring": true}));
        assert_eq!(rows[2]["item"]["switch"], json!({"on": true, "action": LINEUP}));
        let first = page(&all, 0, 2, now);
        assert_eq!((first["rows"].as_array().unwrap().len(), first["more"].clone()), (2, json!(true)));
    }

    #[test]
    fn what_is_kept_comes_back_as_it_was_and_a_channel_the_tv_no_longer_lists_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tuner").join("channels.json");
        let at = UNIX_EPOCH + Duration::from_millis(1_800_000_000_123);
        let health = Health {
            at,
            verdict: Verdict::Glitchy,
            signal: Some(Signal { locked: true, snr: Some(24.5), errors: None }),
            lost: 0.005,
            picture: Some(Duration::from_millis(900)),
            dropouts: 0,
            checked: true,
        };
        let format = Format { width: 1280, height: 720, rate: 7, aspect: 3, progressive: true };
        let mut kept = Kept::read(&path);
        assert!(kept.hidden.is_empty());
        let mut seen = reception("2.1", 32, Some(health.clone()));
        seen.format = Some(format);
        seen.sounds = 2;
        assert!(kept.update(&[seen.clone(), reception("2.2", 32, None)]));
        assert!(!kept.update(&[]), "a tuner that is not serving lists nothing, and that forgets nothing");
        kept.hidden.insert("2.2".to_owned());
        kept.write(&path).unwrap();
        let back = Kept::read(&path);
        assert_eq!(back.hidden, BTreeSet::from(["2.2".to_owned()]));
        let doc = &back.channels["2.1"];
        assert_eq!(health_of(&doc["health"]), Some(health));
        assert_eq!((format_of(&doc["format"]), doc["sounds"].as_u64()), (Some(format), Some(2)));
        let mut back = back;
        assert!(back.update(&[reception("2.2", 32, None)]));
        assert!(back.channels.is_empty());
    }
}
