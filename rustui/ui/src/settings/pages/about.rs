//! About this TV (System > About): the four lines LG's hidden support menu showed on its first screen, read the way that menu reads them.
//! Model and serial number are the factory data (`factorymanager getFactoryOpt`, model and serial keys), Platform is the webOS of the
//! TV's core OS (`osInfo query core_os_release`, major version + 15: 9 is webOS 24), Software version is the firmware's own
//! (`config getConfigs tv.nyx.firmwareVersion`). Read-only: an unanswered line is an en dash, never a guess. The rest of what LG's TV
//! Information lists is under System > Support.

use crate::settings::ctx::Ctx;
use crate::settings::page::{Page, Read};
use crate::settings::row::Row;
use serde_json::{Value, json};

const DASH: &str = "\u{2013}";

pub fn page() -> Page {
    Page::new("about", "About this TV", "tv").parent("system").back("System").reads(reads).rows(rows)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![
        Read::call("factory", "luna://com.webos.service.factorymanager/getFactoryOpt", json!({"keys": ["modelName", "serialNumber"]})),
        Read::call("os", "luna://com.palm.systemservice/osInfo/query", json!({"parameters": ["core_os_release"]})),
        Read::call("firmware", "luna://com.webos.service.config/getConfigs", json!({"configNames": ["tv.nyx.firmwareVersion"]})),
    ]
}

/// A field of a call's answer, from the answer itself or the object the factory service may wrap its keys in; nothing when the call failed.
fn field(c: &Ctx, doc: &str, key: &str) -> String {
    let d = c.docs.get(doc);
    if d.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return String::new();
    }
    [d.get(key), d.pointer(&format!("/values/{key}")), d.pointer(&format!("/factoryOpt/{key}"))]
        .into_iter()
        .flatten()
        .find_map(|v| v.as_str().map(str::to_string).or_else(|| v.as_u64().map(|n| n.to_string())))
        .unwrap_or_default()
}

/// "webOS24" for core OS 9.x: LG numbers the platform by the year, which is the OS's major version + 15.
fn platform(c: &Ctx) -> String {
    let release = field(c, "os", "core_os_release");
    match release.split('.').next().and_then(|m| m.parse::<u32>().ok()) {
        Some(major) if !release.is_empty() => format!("webOS{}", major + 15),
        _ => String::new(),
    }
}

fn firmware(c: &Ctx) -> String {
    let d = c.docs.get("firmware");
    if d.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return String::new();
    }
    d.pointer("/configs/tv.nyx.firmwareVersion").and_then(Value::as_str).unwrap_or("").to_string()
}

fn line(id: &str, icon: &str, label: &str, value: String) -> Row {
    Row::info(id).icon(icon).label(label).value(if value.is_empty() { DASH } else { &value })
}

fn rows(c: &Ctx) -> Vec<Row> {
    vec![
        line("about:model", "tv", "Model", field(c, "factory", "modelName")),
        line("about:serial", "hash", "Serial number", field(c, "factory", "serialNumber")),
        line("about:platform", "cpu", "Platform", platform(c)),
        line("about:software", "settings", "Software version", firmware(c)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;

    fn world() -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.docs.set("factory", json!({"returnValue": true, "modelName": "WEBOS22.SKJY", "serialNumber": "403RMDV12345"}));
        w.docs.set("os", json!({"returnValue": true, "core_os_release": "9.1.0-12345"}));
        w.docs.set("firmware", json!({"returnValue": true, "configs": {"tv.nyx.firmwareVersion": "04.30.25"}}));
        w
    }

    #[test]
    fn the_four_lines_are_what_the_tv_answered_and_nothing_can_be_changed() {
        let w = world();
        let r = rows(&w.ctx());
        assert_eq!(
            r.iter().map(|x| (x.label.as_str(), x.value.as_str())).collect::<Vec<_>>(),
            vec![("Model", "WEBOS22.SKJY"), ("Serial number", "403RMDV12345"), ("Platform", "webOS24"), ("Software version", "04.30.25")]
        );
        assert!(r.iter().all(|x| x.info && x.act.is_none() && !x.chevron), "read-only");
    }

    #[test]
    fn a_line_the_tv_did_not_answer_is_an_en_dash_and_a_wrapped_factory_answer_is_read_too() {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        assert!(rows(&w.ctx()).iter().all(|x| x.value == DASH));
        w.docs.set("factory", json!({"returnValue": false, "modelName": "x"}));
        w.docs.set("os", json!({"returnValue": true, "core_os_release": ""}));
        assert!(rows(&w.ctx()).iter().all(|x| x.value == DASH), "a failed call shows nothing it carried");
        w.docs.set("factory", json!({"returnValue": true, "values": {"modelName": "OLED65C4", "serialNumber": 12345}}));
        let r = rows(&w.ctx());
        assert_eq!((r[0].value.as_str(), r[1].value.as_str()), ("OLED65C4", "12345"));
    }

    #[test]
    fn the_page_hangs_under_system_and_reads_the_three_calls_it_shows() {
        let p = page();
        assert_eq!((p.parent, p.id), (Some("system"), "about"));
        let w = world();
        let reads = (p.reads)(&w.ctx());
        let uris: Vec<&str> = reads.iter().filter_map(|r| if let Read::Call { uri, .. } = r { Some(*uri) } else { None }).collect();
        assert_eq!(uris, ["luna://com.webos.service.factorymanager/getFactoryOpt", "luna://com.palm.systemservice/osInfo/query", "luna://com.webos.service.config/getConfigs"]);
    }
}
