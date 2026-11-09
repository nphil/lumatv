//! What several pages ask the TV the same way: whether it has an ATSC 3.0 tuner, the panel's hours, a row that waits for another; and
//! the rows of the idle timers.

use crate::host::inputs::Inputs;
use crate::host::net::Net;
use crate::settings::ctx::Ctx;
use crate::settings::page::Read;
use crate::settings::row::{ChoiceSpec, Op, PageArg, Pick, Row, patch_of};
use serde_json::{Value, json};
use std::rc::Rc;

/// Opens another page (`host.openPage(id)`).
pub fn go(page: &str) -> Vec<Op> {
    vec![Op::Open { page: page.into(), arg: PageArg::None }]
}

/// The TV's inputs, subscribed once (`Inputs.refresh()`): the page keeps the answer as the document `inputs`.
pub fn inputs_read() -> Read {
    Read::Watch { name: "inputs", uri: crate::host::inputs::EIM, payload: json!({}) }
}

/// The inputs as the external input manager lists them, with the app in front (`Inputs.list`, `Inputs.current`).
pub fn inputs(c: &Ctx) -> Inputs {
    let mut i = Inputs::new();
    i.parse(c.docs.get("inputs"));
    i.foreground = c.topics.channel("foreground").and_then(|f| f.get("app")).and_then(Value::as_str).unwrap_or("").to_string();
    i
}

/// The network as the connection manager says it (`kit/Net.qml`); the model keeps that subscription while Settings is open.
pub fn net(c: &Ctx) -> Net {
    let mut n = Net::new();
    n.parse(c.docs.get("net"));
    n
}

/// The config `system.supportATSC3tuner` (`settings/Atsc3.qml`): LG's NEXTGEN TV rows are on show only where it is true.
pub fn atsc3_read() -> Read {
    Read::call("atsc3", "luna://com.webos.service.config/getConfigs", json!({"configNames": ["system.supportATSC3tuner"]}))
}

/// A config answer's flag as LG reads it: absent, null, 0, false and "false" are no.
fn config_flag(doc: &Value, name: &str) -> Option<bool> {
    let v = doc.get("configs")?.get(name)?;
    Some(!(v.is_null() || v == &json!(false) || v == &json!("false") || v == &json!(0)))
}

pub fn atsc3(c: &Ctx) -> bool {
    config_flag(c.docs.get("atsc3"), "system.supportATSC3tuner").unwrap_or(false)
}

/// The panel's hours as LG shows them (`kit/Power.qml`): its counter ticks every ten minutes. The row is shown unless the TV's config says
/// the counter is not supported.
pub fn panel_reads() -> Vec<Read> {
    vec![
        Read::call("panelcfg", "luna://com.webos.service.config/getConfigs", json!({"configNames": ["tv.conti.supportUsedTime"]})),
        Read::Watch { name: "panel", uri: "luna://com.webos.service.panelcontroller/getPanelUsageTime", payload: json!({}) },
    ]
}

pub fn panel_shown(c: &Ctx) -> bool {
    config_flag(c.docs.get("panelcfg"), "tv.conti.supportUsedTime") != Some(false)
}

/// "1,705 h", or nothing until the TV says.
pub fn panel_hours(c: &Ctx) -> String {
    match c.docs.get("panel").get("panelUsageTime").and_then(Value::as_f64) {
        Some(t) if t >= 0.0 => format!("{} h", grouped((t / 6.0).floor() as u64)),
        _ => String::new(),
    }
}

/// 1705 -> "1,705".
pub fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A row dimmed with the words that say what it waits for.
pub fn waits(mut row: Row, ready: bool, why: &str) -> Row {
    if !ready {
        row.enabled = false;
        row.detail = why.to_string();
    }
    row
}

/// The choices of the idle timers' rows in minutes, Off (0) last: the waits the compositor runs (`oledcare::Settings::from_theme`).
pub const IDLE_WAITS: [u64; 9] = [1, 3, 5, 10, 15, 30, 60, 180, 0];
/// The compositor's waits while the theme does not say: Home dims after 3 minutes (`oledcare.dimAfterMin`), the screensaver comes after 5
/// (`screensaver.afterMin`).
pub const DIM_AFTER_MIN: u64 = 3;
pub const SAVER_AFTER_MIN: u64 = 5;

/// "Off", "1 minute", "10 minutes", "1 hour", "3 hours".
pub fn idle_wait_text(min: u64) -> String {
    match min {
        0 => "Off".into(),
        1 => "1 minute".into(),
        60 => "1 hour".into(),
        m if m % 60 == 0 => format!("{} hours", m / 60),
        m => format!("{m} minutes"),
    }
}

/// The minutes of an idle timer written at `path` in the theme, `default` while it is not: 0 is Off, the rest held to the choices' range
/// as the compositor holds it.
pub fn idle_wait(c: &Ctx, path: &str, default: u64) -> u64 {
    match c.theme_at(path).and_then(Value::as_f64).filter(|v| v.is_finite()) {
        None => default,
        Some(v) if v <= 0.0 => 0,
        Some(v) => v.clamp(1.0, 180.0).round() as u64,
    }
}

/// When the screensaver comes, in minutes: 0 (never) with its wait Off or its style Off, as the compositor reads them.
pub fn saver_wait(c: &Ctx) -> u64 {
    if c.theme_str("screensaver.style") == Some("off") { 0 } else { idle_wait(c, "screensaver.afterMin", SAVER_AFTER_MIN) }
}

/// An idle timer's row (id, tab, icon, label, help) on the theme's `path`: a picker of [`IDLE_WAITS`] whose pick is a patch of that one key.
pub fn idle_wait_row(c: &Ctx, row: (&str, usize, &str, &str, &str), path: &str, default: u64) -> Row {
    let (id, tab, icon, label, help) = row;
    let min = idle_wait(c, path, default);
    let path = path.to_string();
    Row::link(id).tab(tab).icon(icon).label(label).help(help).value(&idle_wait_text(min)).does(vec![Op::Pick(Rc::new(ChoiceSpec {
        title: label.into(),
        options: IDLE_WAITS.iter().map(|m| Pick::new(&m.to_string(), &idle_wait_text(*m))).collect(),
        current: min.to_string(),
        write: Box::new(move |v| match v.parse::<u64>() {
            Ok(m) if m != min => vec![Op::Patch(patch_of(&path, json!(m)))],
            _ => Vec::new(),
        }),
    }))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;

    #[test]
    fn a_thousand_is_grouped_and_the_panel_row_waits_for_the_tv() {
        assert_eq!(grouped(5), "5");
        assert_eq!(grouped(1705), "1,705");
        assert_eq!(grouped(1_234_567), "1,234,567");
        let mut w = World::new(crate::host::sio::Sio::new(0));
        assert!(panel_shown(&w.ctx()), "shown until the config says it is not supported");
        assert_eq!(panel_hours(&w.ctx()), "");
        w.docs.set("panelcfg", json!({"configs": {"tv.conti.supportUsedTime": "false"}}));
        assert!(!panel_shown(&w.ctx()));
        w.docs.set("panel", json!({"returnValue": true, "panelUsageTime": 10230}));
        assert_eq!(panel_hours(&w.ctx()), "1,705 h");
    }

    #[test]
    fn an_atsc3_tuner_is_there_only_when_the_config_says_so() {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        assert!(!atsc3(&w.ctx()));
        w.docs.set("atsc3", json!({"configs": {"system.supportATSC3tuner": "true"}}));
        assert!(atsc3(&w.ctx()));
        w.docs.set("atsc3", json!({"configs": {"system.supportATSC3tuner": false}}));
        assert!(!atsc3(&w.ctx()));
    }

    #[test]
    fn an_idle_timer_offers_a_minute_to_three_hours_and_off_and_writes_only_its_theme_key() {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        let row = |w: &World| idle_wait_row(&w.ctx(), ("saver", 2, "timer", "Screensaver after", ""), "screensaver.afterMin", 5);
        let r = row(&w);
        assert_eq!(r.value, "5 minutes", "the compositor's default while the theme says nothing");
        let ops = (r.act.as_ref().unwrap())(crate::settings::row::Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(spec.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), ["1 minute", "3 minutes", "5 minutes", "10 minutes", "15 minutes", "30 minutes", "1 hour", "3 hours", "Off"]);
        assert_eq!((spec.title.as_str(), spec.current.as_str()), ("Screensaver after", "5"));
        assert!((spec.write)("5").is_empty(), "the wait in force is not written again");
        for (v, m) in [("1", 1), ("180", 180), ("0", 0)] {
            let written = (spec.write)(v);
            assert!(matches!(&written[..], [Op::Patch(p)] if p == &json!({"screensaver": {"afterMin": m}})), "a patch of the one key and nothing of LG's: {written:?}");
        }
        w.theme = json!({"screensaver": {"afterMin": 0}});
        assert_eq!(row(&w).value, "Off");
        w.theme = json!({"screensaver": {"afterMin": 600}});
        assert_eq!(row(&w).value, "3 hours", "held where the compositor holds it");
    }
}
