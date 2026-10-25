//! The generated server tables against LG's own (`protocol/lg-tables.txt`, read on the set): opcodes are positional,
//! so every request and event must sit at LG's index with LG's signature, and no message may carry a since mark a
//! version-1 object from LG's libraries would trip over.

use std::collections::BTreeMap;

use wayland_backend::protocol::{ArgumentType, Interface};

use lumatv_wl_protocols::{input_method, starfish, text, webos_extension, webos_foreign, webos_input_manager, webos_shell, webos_surface_group};

fn interfaces() -> Vec<&'static Interface> {
    use input_method::server::__interfaces as im;
    use starfish::server::__interfaces as sf;
    use text::server::__interfaces as tx;
    use webos_extension::server::__interfaces as ext;
    use webos_foreign::server::__interfaces as fo;
    use webos_input_manager::server::__interfaces as inm;
    use webos_shell::server::__interfaces as sh;
    use webos_surface_group::server::__interfaces as sg;
    vec![
        &sh::WL_WEBOS_SHELL_INTERFACE,
        &sh::WL_WEBOS_SHELL_SURFACE_INTERFACE,
        &fo::WL_WEBOS_FOREIGN_INTERFACE,
        &fo::WL_WEBOS_EXPORTED_INTERFACE,
        &fo::WL_WEBOS_IMPORTED_INTERFACE,
        &sg::WL_WEBOS_SURFACE_GROUP_COMPOSITOR_INTERFACE,
        &sg::WL_WEBOS_SURFACE_GROUP_INTERFACE,
        &sg::WL_WEBOS_SURFACE_GROUP_LAYER_INTERFACE,
        &inm::WL_WEBOS_INPUT_MANAGER_INTERFACE,
        &inm::WL_WEBOS_SEAT_INTERFACE,
        &inm::WL_WEBOS_GYROSCOPE_INTERFACE,
        &inm::WL_WEBOS_ACCELEROMETER_INTERFACE,
        &ext::WL_WEBOS_XINPUT_INTERFACE,
        &ext::WL_WEBOS_XINPUT_EXTENSION_INTERFACE,
        &tx::TEXT_MODEL_INTERFACE,
        &tx::TEXT_MODEL_FACTORY_INTERFACE,
        &im::INPUT_METHOD_INTERFACE,
        &im::INPUT_METHOD_CONTEXT_INTERFACE,
        &im::INPUT_PANEL_INTERFACE,
        &im::INPUT_PANEL_SURFACE_INTERFACE,
        &sf::WL_STARFISH_POINTER_INTERFACE,
        &sf::WL_STARFISH_OUTPUT_INTERFACE,
    ]
}

fn signature(args: &[ArgumentType]) -> String {
    let s: String = args
        .iter()
        .map(|a| match a {
            ArgumentType::Int => 'i',
            ArgumentType::Uint => 'u',
            ArgumentType::Fixed => 'f',
            ArgumentType::Str(_) => 's',
            ArgumentType::Object(_) => 'o',
            ArgumentType::NewId => 'n',
            ArgumentType::Array => 'a',
            ArgumentType::Fd => 'h',
        })
        .collect();
    if s.is_empty() { "-".into() } else { s }
}

/// `(interface, kind)` → version and the messages in opcode order.
type Table = BTreeMap<(String, String), (u32, Vec<(String, String)>)>;

fn lg_table() -> Table {
    let mut t = Table::new();
    for line in include_str!("../../../protocol/lg-tables.txt").lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let [iface, version, kind, opcode, name, sig] = f[..] else { panic!("bad line {line:?}") };
        let e = t.entry((iface.to_string(), kind.to_string())).or_insert((version.parse().unwrap(), Vec::new()));
        assert_eq!(e.1.len(), opcode.parse::<usize>().unwrap(), "{line}: opcodes out of order in the file");
        e.1.push((name.to_string(), sig.to_string()));
    }
    t
}

#[test]
fn generated_tables_match_lg() {
    let lg = lg_table();
    let ours = interfaces();
    let mut checked = 0;
    for ((iface, kind), (version, messages)) in &lg {
        let i = ours.iter().find(|i| i.name == iface).unwrap_or_else(|| panic!("{iface}: in LG's table, not generated"));
        assert_eq!(i.version, *version, "{iface}: version");
        let generated = if kind == "request" { i.requests } else { i.events };
        let generated: Vec<(String, String)> = generated.iter().map(|m| (m.name.to_string(), signature(m.signature))).collect();
        assert_eq!(&generated, messages, "{iface} {kind}s: opcode order or signature differs from LG's");
        checked += 1;
    }
    // A kind with no lines in the file has no messages on LG's side either.
    for i in &ours {
        assert!(lg.keys().any(|(n, _)| n == i.name), "{}: generated but not in LG's table", i.name);
        for (kind, msgs) in [("request", i.requests), ("event", i.events)] {
            assert!(msgs.is_empty() || lg.contains_key(&(i.name.to_string(), kind.to_string())), "{} has {kind}s LG's does not", i.name);
        }
    }
    assert!(checked >= 30, "only {checked} tables checked");
}

#[test]
fn no_since_marks_on_child_objects() {
    // Child objects inherit their parent's version (1 for every LG global but input_method), so a since mark above
    // 1 would make libwayland refuse a request LG's clients send. input_method is a global offered at version 2.
    for i in interfaces().into_iter().filter(|i| i.name != "input_method") {
        for m in i.requests.iter().chain(i.events) {
            assert!(m.since <= 1, "{}.{} has since {}", i.name, m.name, m.since);
        }
    }
}
