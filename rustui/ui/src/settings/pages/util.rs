//! What several pages ask the TV the same way: whether it has an ATSC 3.0 tuner, the panel's hours, a row that waits for another.

use crate::settings::ctx::Ctx;
use crate::settings::page::Read;
use crate::settings::row::Row;
use serde_json::{Value, json};

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
}
