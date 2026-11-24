use super::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Default)]
struct Fake {
    values: Rc<RefCell<Vec<(Key, Value)>>>,
    writes: Rc<Cell<usize>>,
    refuse: Rc<Cell<bool>>,
    disabled: Rc<Cell<bool>>,
    revoke_on_read: Rc<Cell<bool>>,
}
impl Fake {
    fn put(&self, key: Key, value: Value) {
        let mut values = self.values.borrow_mut();
        if let Some((_, v)) = values.iter_mut().find(|(k, _)| *k == key) { *v = value; } else { values.push((key, value)); }
    }
}
impl Device for Fake {
    fn allowed(&self) -> bool { !self.disabled.get() }
    fn read(&self, key: &Key) -> Result<Value> {
        if self.revoke_on_read.get() { self.disabled.set(true); }
        Ok(self.values.borrow().iter().find(|(k, _)| k == key).map_or_else(|| if matches!(key, Key::HostRoute { .. }) { Value::Route(None) } else { Value::Number(0) }, |(_, v)| v.clone()))
    }
    fn write(&self, key: &Key, value: &Value) -> Result<()> {
        self.writes.set(self.writes.get() + 1);
        if self.refuse.get() { return Err("write refused".into()); }
        self.put(key.clone(), value.clone());
        Ok(())
    }
}
#[derive(Clone, Default)]
struct Store { journal: Rc<RefCell<Option<Journal>>>, fail: Rc<Cell<bool>> }
impl JournalStore for Store {
    fn load(&self) -> Result<Option<Journal>> { Ok(self.journal.borrow().clone()) }
    fn save(&self, journal: &Journal) -> Result<()> {
        if self.fail.get() { return Err("disk full".into()); }
        *self.journal.borrow_mut() = Some(journal.clone());
        Ok(())
    }
}
fn arp(name: &str) -> Key { Key::Sysctl { interface: name.into(), knob: Knob::ArpIgnore } }
fn link(name: &str, kind: Kind, last: u8) -> Link {
    Link { interface: name.into(), kind, address: Ipv4Addr::new(192,168,1,last), prefix: 24, carrier_changes: 1,
        nominal_mbps: if kind == Kind::Wifi { None } else { Some(100) }, signal_dbm: Some(-49), rx_bytes: 0, tx_bytes: 0 }
}
fn network() -> Snapshot {
    Snapshot { links: vec![link("eth0", Kind::Ethernet, 242), link("wlan0", Kind::Wifi, 180)], default_interface: "eth0".into(),
        gateway: "192.168.1.1".parse().unwrap(), host_routes: Vec::new(), all_arp_ignore: 0, all_arp_announce: 0 }
}
fn route() -> Route {
    Route { destination: "192.168.5.10".parse().unwrap(), gateway: "192.168.1.1".parse().unwrap(), interface: "wlan0".into(),
        source: Some("192.168.1.180".parse().unwrap()), metric: 0, protocol: None }
}

#[test]
fn wired_defaults_and_only_explicit_local_interfaces_are_eligible() {
    let mut n = network();
    assert_eq!(plan(&n, true, None).unwrap().candidates.len(), 2);
    for bad in ["p2p0", "lo", "sit0", "../../all", "eth0;reboot", "all"] { assert!(!valid_interface(bad)); }
    n.default_interface = "wlan0".into();
    assert!(plan(&n, true, None).is_err());
    n.default_interface = "eth0".into();
    n.all_arp_ignore = 8;
    assert!(plan(&n, true, None).is_err());
}

#[test]
fn weak_wifi_drop_and_screen_sharing_keep_only_the_wire() {
    let mut n = network();
    n.links[1].signal_dbm = Some(-76);
    assert_eq!(plan(&n, true, None).unwrap().candidates.len(), 1);
    n.links[1].signal_dbm = Some(-49);
    assert_eq!(plan(&n, false, None).unwrap().candidates.len(), 1);
    n.links.pop();
    assert_eq!(plan(&n, true, None).unwrap().control, "eth0");
}

#[test]
fn usb_gigabit_is_preferred_and_another_subnet_is_not_assumed_to_work() {
    let mut n = network();
    let mut usb = link("eth1", Kind::UsbEthernet, 50);
    usb.nominal_mbps = Some(1000);
    n.links.push(usb);
    let p = plan(&n, true, None).unwrap();
    assert_eq!(p.candidates[0].interface, "eth1");
    assert_eq!(p.control, "eth0");
    n.links[2].address = "192.168.9.50".parse().unwrap();
    assert_eq!(plan(&n, true, None).unwrap().candidates.len(), 2);
}

#[test]
fn dhcp_renew_and_roam_invalidate_old_rates_and_use_the_new_source() {
    let mut n = network();
    let mut meter = Meter::default();
    assert!(meter.sample(1000, &n.links).is_empty());
    n.links[1].rx_bytes = 20_000_000;
    assert_eq!(meter.sample(2000, &n.links)[1].rx_mbps, 160.0);
    n.links[1].address = "192.168.1.181".parse().unwrap();
    n.links[1].rx_bytes += 20_000_000;
    assert_eq!(meter.sample(3000, &n.links)[1].peak_rx_mbps, 0.0);
    assert_eq!(plan(&n, true, None).unwrap().candidates.iter().find(|l| l.kind == Kind::Wifi).unwrap().address, n.links[1].address);
    n.links[1].carrier_changes += 2;
    assert_eq!(meter.sample(4000, &n.links)[1].rx_mbps, 0.0);
    n.links[1].rx_bytes = 0;
    assert_eq!(meter.sample(4500, &n.links)[1].rx_mbps, 0.0);
}

#[test]
fn host_route_is_journalled_without_pinning_all_sockets_to_the_wifi_source() {
    let mut n = network();
    let old = route();
    n.host_routes.push(old.clone());
    let device = Fake::default();
    let key = Key::HostRoute { destination: old.destination };
    device.put(key.clone(), Value::Route(Some(old.clone())));
    let mut tx = Transaction::open(device.clone(), Store::default(), "boot".into()).unwrap();
    tx.network(&plan(&n, true, None).unwrap().settings).unwrap();
    let Value::Route(Some(wired)) = device.read(&key).unwrap() else { panic!() };
    assert_eq!(wired.interface, "eth0");
    assert_eq!(wired.source, None);
    n.host_routes.clear();
    tx.network(&plan(&n, true, None).unwrap().settings).unwrap();
    assert_eq!(device.read(&key).unwrap(), Value::Route(Some(wired)));
    tx.restore().unwrap();
    assert_eq!(device.read(&key).unwrap(), Value::Route(Some(old)));
}

#[test]
fn a_pinned_server_rides_wifi_through_the_wired_gateway_and_follows_its_address_and_the_link() {
    let device = Fake::default();
    let mut tx = Transaction::open(device.clone(), Store::default(), "boot".into()).unwrap();
    let mut n = network();
    let key = |ip: &str| Key::HostRoute { destination: ip.parse().unwrap() };
    let first = Some("192.168.5.10".parse().unwrap());
    tx.network(&plan(&n, true, first).unwrap().settings).unwrap();
    assert_eq!(device.read(&key("192.168.5.10")).unwrap(), Value::Route(Some(route())));
    // A restart finds the same route: nothing is rewritten.
    let writes = device.writes.get();
    tx.network(&plan(&n, true, first).unwrap().settings).unwrap();
    assert_eq!(device.writes.get(), writes);
    // The server moves: the old pin goes, the new one comes.
    tx.network(&plan(&n, true, Some("192.168.5.20".parse().unwrap())).unwrap().settings).unwrap();
    assert_eq!(device.read(&key("192.168.5.10")).unwrap(), Value::Route(None));
    let Value::Route(Some(moved)) = device.read(&key("192.168.5.20")).unwrap() else { panic!() };
    assert_eq!((moved.interface.as_str(), moved.gateway, moved.source), ("wlan0", n.gateway, Some(n.links[1].address)));
    // Wi-Fi leaves, or the address is not private or is the TV's own: no pin.
    n.links.pop();
    tx.network(&plan(&n, true, Some(moved.destination)).unwrap().settings).unwrap();
    assert_eq!(device.read(&key("192.168.5.20")).unwrap(), Value::Route(None));
    let n = network();
    for bad in ["8.8.8.8", "192.168.1.180"] {
        assert!(!plan(&n, true, Some(bad.parse().unwrap())).unwrap().settings.iter().any(|(k, _)| matches!(k, Key::HostRoute { .. })));
    }
    tx.network(&plan(&n, true, first).unwrap().settings).unwrap();
    tx.restore().unwrap();
    assert_eq!(device.read(&key("192.168.5.10")).unwrap(), Value::Route(None));
}

#[test]
fn established_peers_on_a_port_are_the_private_remote_addresses() {
    let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
        0: B401A8C0:D1DA 0A05A8C0:7E90 01 00000000:00000000 00:00000000 00000000 0 0 1\n\
        1: B401A8C0:D1DB 0A05A8C0:7E90 01 00000000:00000000 00:00000000 00000000 0 0 1\n\
        2: B401A8C0:D1DC 0A05A8C0:7E90 06 00000000:00000000 00:00000000 00000000 0 0 1\n\
        3: B401A8C0:D1DD 0605A8C0:7E90 01 00000000:00000000 00:00000000 00000000 0 0 1\n\
        4: B401A8C0:D1DE 0A05A8C0:0016 01 00000000:00000000 00:00000000 00000000 0 0 1\n\
        5: B401A8C0:D1DF 22656B34:7E90 01 00000000:00000000 00:00000000 00000000 0 0 1\n";
    let peers = linux::established_peers(table, 32400);
    assert_eq!(peers, ["192.168.5.10".parse::<Ipv4Addr>().unwrap(), "192.168.5.6".parse().unwrap()]);
}

#[test]
fn standby_disable_and_exit_restore_then_wake_can_prepare_again() {
    let device = Fake::default();
    let mut tx = Transaction::open(device.clone(), Store::default(), "boot".into()).unwrap();
    for _ in 0..3 {
        tx.set(Key::WifiPower, Value::Number(1)).unwrap();
        tx.set(Key::WifiProfile, Value::Number(777)).unwrap();
        tx.network(&plan(&network(), true, None).unwrap().settings).unwrap();
        tx.restore().unwrap();
        assert!(!tx.pending());
        assert_eq!(device.read(&Key::WifiPower).unwrap(), Value::Number(0));
        assert_eq!(device.read(&arp("wlan0")).unwrap(), Value::Number(0));
    }
}

#[test]
fn a_failed_journal_never_performs_the_change() {
    let device = Fake::default();
    let store = Store::default();
    let mut tx = Transaction::open(device.clone(), store.clone(), "boot".into()).unwrap();
    store.fail.set(true);
    assert!(tx.set(arp("eth0"), Value::Number(1)).is_err());
    assert_eq!(device.writes.get(), 0);
}

#[test]
fn revoked_permission_prevents_writes_but_never_blocks_rollback() {
    let device = Fake::default();
    let mut tx = Transaction::open(device.clone(), Store::default(), "boot".into()).unwrap();
    device.revoke_on_read.set(true);
    assert!(tx.set(arp("eth0"), Value::Number(1)).is_err());
    assert_eq!(device.writes.get(), 0, "permission changed after the initial check");
    tx.restore().unwrap();
    device.revoke_on_read.set(false);
    device.disabled.set(false);
    tx.set(arp("eth0"), Value::Number(1)).unwrap();
    device.disabled.set(true);
    tx.restore().unwrap();
    assert_eq!(device.read(&arp("eth0")).unwrap(), Value::Number(0));
}

#[test]
fn crash_recovery_preserves_originals_and_retries_failed_restoration() {
    let device = Fake::default();
    let store = Store::default();
    let mut first = Transaction::open(device.clone(), store.clone(), "boot".into()).unwrap();
    first.set(arp("eth0"), Value::Number(1)).unwrap();
    let mut recovered = Transaction::open(device.clone(), store, "boot".into()).unwrap();
    device.refuse.set(true);
    assert!(recovered.restore().is_err());
    assert!(recovered.pending());
    device.refuse.set(false);
    recovered.restore().unwrap();
    assert_eq!(device.read(&arp("eth0")).unwrap(), Value::Number(0));
}

#[test]
fn another_owner_and_new_boot_values_are_never_overwritten() {
    let device = Fake::default();
    let store = Store::default();
    let mut first = Transaction::open(device.clone(), store.clone(), "old".into()).unwrap();
    first.set(arp("eth0"), Value::Number(1)).unwrap();
    first.set(Key::WifiPower, Value::Number(1)).unwrap();
    device.put(arp("eth0"), Value::Number(2));
    let mut next = Transaction::open(device.clone(), store, "new".into()).unwrap();
    next.restore().unwrap();
    assert_eq!(device.read(&arp("eth0")).unwrap(), Value::Number(2));
    assert_eq!(device.read(&Key::WifiPower).unwrap(), Value::Number(0));
    next.set(arp("wlan0"), Value::Number(1)).unwrap();
    device.put(arp("wlan0"), Value::Number(2));
    next.restore().unwrap();
    assert_eq!(device.read(&arp("wlan0")).unwrap(), Value::Number(2));
}

#[test]
fn wifi_drop_removes_its_preparation_and_usb_removal_is_not_a_new_write() {
    let device = Fake::default();
    let mut tx = Transaction::open(device.clone(), Store::default(), "boot".into()).unwrap();
    let mut n = network();
    tx.network(&plan(&n, true, None).unwrap().settings).unwrap();
    n.links.pop();
    tx.network(&plan(&n, true, None).unwrap().settings).unwrap();
    assert_eq!(device.read(&arp("wlan0")).unwrap(), Value::Number(0));
    assert_eq!(device.read(&arp("eth0")).unwrap(), Value::Number(0));
    device.put(arp("eth1"), Value::Number(u32::MAX));
    assert!(tx.set(arp("eth1"), Value::Number(1)).is_err());
}

#[test]
fn route_parser_rejects_unrestorable_or_nonlocal_routes_and_ambiguous_defaults() {
    let r = linux::parse_host_route("192.168.5.10 via 192.168.1.1 dev wlan0 src 192.168.1.180").unwrap();
    assert_eq!(r, route());
    assert!(linux::parse_host_route("192.168.5.10 via 192.168.1.1 dev wlan0 mtu 1300").is_none());
    assert!(linux::parse_host_route("44.228.34.20 via 192.168.1.1 dev wlan0").is_none());
    let one = "Iface Destination Gateway Flags RefCnt Use Metric Mask\neth0 00000000 0101A8C0 0003 0 0 0 00000000\n";
    assert_eq!(linux::default_route(one).unwrap(), ("eth0".into(), "192.168.1.1".parse().unwrap()));
    assert!(linux::default_route(&format!("{one}wlan0 00000000 0101A8C0 0003 0 0 0 00000000\n")).is_none());
}

#[test]
fn file_journal_is_private_and_corruption_fails_closed() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let mut tx = Transaction::open(Fake::default(), FileJournal::new(path.clone()), "boot".into()).unwrap();
    tx.set(arp("eth0"), Value::Number(1)).unwrap();
    assert_eq!(std::fs::metadata(path.join("rollback.json")).unwrap().permissions().mode() & 0o777, 0o600);
    std::fs::write(path.join("rollback.json"), "broken").unwrap();
    assert!(Transaction::open(Fake::default(), FileJournal::new(path), "boot".into()).is_err());
}
