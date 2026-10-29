//! Whether what a declutter group stops is in place before LG starts it. Read from the group's items
//! (payload/declutter/items.txt) and the `early` object of `declutter.sh status` (tools/declutter.sh, `verb_early`),
//! which reports the early step: autostart.sh's worker, started by the kdump shim about 13 to 14 s into a boot.
use serde_json::{Value, json};

const ITEMS: &str = include_str!("../../../payload/declutter/items.txt");

/// What a group stops, split by when it can be beaten: units and stubbed programs by the early step, programs
/// that can only be killed once they run (`kill` items) not at all. Settings, apps and list edits persist
/// across a boot and are in neither list.
struct Parts {
    early: Vec<(&'static str, &'static str)>,
    late: Vec<&'static str>,
}

fn parts(group: &str) -> Parts {
    let mut p = Parts {
        early: Vec::new(),
        late: Vec::new(),
    };
    for line in ITEMS.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&'static str> = line.split('|').collect();
        if f.len() < 4 || f[1] != group {
            continue;
        }
        match (f[0], f[3]) {
            ("unit", _) => p.early.push((f[2], "unit")),
            ("luna", "stub") => p.early.push((f[2], "program")),
            ("luna", _) => p.late.push(f[2]),
            _ => {}
        }
    }
    p
}

/// The `boot` object of one group's row. `blocked` is yes only when the group is on, everything it stops can be
/// put in place by the early step (which is installed), a pass has run since this boot started and did not skip
/// it, and it found none of the group's items already running. A record from an earlier boot proves nothing.
pub fn group(id: &str, on: Option<bool>, early: Option<&Value>) -> Value {
    let p = parts(id);
    let hook = early.is_some_and(|e| e["hook"] == true);
    let record = early.map(|e| &e["last"]).filter(|l| l.is_object());
    let stale = record.is_some() && !early.is_some_and(|e| e["thisBoot"] == true);
    let last = record.filter(|_| !stale);
    let ran = last.is_some_and(|l| l["ran"] == true);
    let skipped = !p.early.is_empty() && last.is_some() && !ran;
    let up = last.and_then(|l| l["uptimeSec"].as_f64());
    let running: Vec<&str> = last
        .and_then(|l| l["running"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let before: Vec<&str> = p
        .early
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| running.contains(n))
        .collect();
    let on_now = on == Some(true);
    let in_place = on_now && (p.early.is_empty() || hook);
    let stale_here = stale && !p.early.is_empty();
    let blocked = in_place && p.late.is_empty() && before.is_empty() && !skipped && !stale_here;
    let reason = if on.is_none() {
        "The TV's privacy tool did not answer.".to_owned()
    } else if !on_now {
        "Off.".to_owned()
    } else if !p.late.is_empty() {
        format!(
            "{} can only be stopped after LG has started it, by the boot pass.",
            p.late.join(", ")
        )
    } else if p.early.is_empty() {
        "LG keeps these values across a reboot; the boot pass sets them again.".to_owned()
    } else if !hook {
        "The early boot step is not installed: LG starts these first and only the boot pass stops them.".to_owned()
    } else if stale_here {
        "The early step has not run since this boot started; the last pass belongs to an earlier boot.".to_owned()
    } else if !before.is_empty() {
        let at = up
            .map(|s| format!(" ({s:.1} s into the boot)"))
            .unwrap_or_default();
        format!(
            "LG had already started {} when the early step came{at}.",
            before.join(", ")
        )
    } else if skipped {
        let why = last.and_then(|l| l["why"].as_str()).unwrap_or("");
        format!("The early step skipped the last boot: {why}.")
    } else if ran {
        let stubs = last
            .and_then(|l| l["stubbedSec"].as_f64())
            .map(|s| format!(" (stubs at {s:.1} s)"))
            .unwrap_or_default();
        format!(
            "Put in place {:.1} s into this boot{stubs}; none of it was running by then.",
            up.unwrap_or(0.0)
        )
    } else {
        "Put in place about 14 s into the next boot, before LG's units start.".to_owned()
    };
    let item = |name: &str, kind: &str, early_capable: bool| {
        let started = before.contains(&name);
        json!({"name": name, "kind": kind, "blocked": in_place && early_capable && !started && !skipped && !stale_here, "startedBefore": started})
    };
    let items: Vec<Value> = p
        .early
        .iter()
        .map(|(n, k)| item(n, k, true))
        .chain(p.late.iter().map(|n| item(n, "program", false)))
        .collect();
    json!({
        "blocked": blocked,
        "reason": reason,
        "verified": (on_now && !p.early.is_empty() && ran).then_some(before.is_empty()),
        "lastPass": record.map(pass),
        "items": items,
    })
}

/// The fields of a pass that a menu shows. The boot's clock values are seconds to the hundredth.
fn pass(l: &Value) -> Value {
    json!({"ran": l["ran"], "why": l["why"], "startSec": l["startSec"], "stubbedSec": l["stubbedSec"], "maskedSec": l["maskedSec"], "uptimeSec": l["uptimeSec"]})
}

/// A switch the helper applies itself, once it starts: after the early step, so not blocked from boot.
pub fn after_boot(on: bool) -> Value {
    json!({
        "blocked": false,
        "reason": if on { "Applied when LumaTV's helper starts, a little after the early boot step." } else { "Off." },
    })
}

/// The early step as a whole, for the menu's header.
pub fn root(early: Option<&Value>) -> Value {
    json!({
        "earlyStep": early.is_some_and(|e| e["hook"] == true),
        "pending": early.is_some_and(|e| e["pending"] == true),
        "thisBoot": early.is_some_and(|e| e["thisBoot"] == true),
        "lastPass": early.map(|e| &e["last"]).filter(|l| l.is_object()).map(pass),
    })
}
