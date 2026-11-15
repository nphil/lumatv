use super::*;
use parking_lot::Mutex;
use std::os::fd::OwnedFd;
use std::sync::Arc;

#[derive(Default)]
struct Hardware {
    values: BTreeMap<Control, i32>,
    calls: Vec<String>,
    fail_set: Option<Control>,
    fail_restore: bool,
    bad_readback: bool,
    device_changed: bool,
}

#[derive(Clone)]
struct FakeDevices {
    hardware: Arc<Mutex<Hardware>>,
    journal: PathBuf,
}

struct FakeFd {
    devices: FakeDevices,
    identity: String,
    _fd: OwnedFd,
}

impl Devices for FakeDevices {
    fn open(&self, controls: &[Control]) -> io::Result<Box<dyn ControlFd>> {
        let mut hardware = self.hardware.lock();
        hardware.calls.push(format!("open {:?}", controls[0]));
        let identity = if hardware.device_changed { "different-device" } else { "fake-vpq:81:0" }.to_owned();
        Ok(Box::new(FakeFd { devices: self.clone(), identity, _fd: File::open("/dev/null")?.into() }))
    }
}

impl ControlFd for FakeFd {
    fn identity(&self) -> &str { &self.identity }

    fn read(&mut self, control: Control) -> io::Result<i32> {
        let mut hardware = self.devices.hardware.lock();
        hardware.calls.push(format!("read {control:?}"));
        let value = *hardware.values.get(&control).unwrap_or(&(1 - control.target()));
        if hardware.bad_readback && value == control.target() { Ok(1 - value) } else { Ok(value) }
    }

    fn write(&mut self, control: Control, value: i32) -> io::Result<()> {
        assert!(self.devices.journal.exists(), "rollback must be saved before every device set");
        let mut hardware = self.devices.hardware.lock();
        hardware.calls.push(format!("write {control:?} {value}"));
        if hardware.fail_restore && value != control.target() { return Err(io::Error::other("restore failed")); }
        hardware.values.insert(control, value);
        if hardware.fail_set == Some(control) && value == control.target() { return Err(io::Error::other("set failed after changing hardware")); }
        Ok(())
    }
}

struct Rig {
    dir: tempfile::TempDir,
    devices: FakeDevices,
    state: State,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let devices = FakeDevices { hardware: Arc::new(Mutex::new(Hardware::default())), journal: dir.path().join("run/rollback.json") };
        let mut state = State::new(devices.journal.clone(), dir.path().join("qualifications.json"), "boot-one".into());
        state.enabled = true;
        Self { dir, devices, state }
    }

    fn qualify(&mut self, control: Control) -> Value {
        self.state.qualify(control, &self.devices, Some("tv/live-hardware-receipt".into()), Some("tv/lifecycle-receipt".into())).unwrap()
    }

    fn acquire(&mut self, now: Instant, controls: Vec<Control>) -> Value {
        self.state.acquire(holder(), controls, &self.devices, now).unwrap()
    }

    fn restarted(&self, boot: &str) -> State {
        State::new(self.devices.journal.clone(), self.dir.path().join("qualifications.json"), boot.into())
    }
}

fn holder() -> Holder {
    Holder { pid: 100, fd: File::open("/dev/null").unwrap().into() }
}

#[test]
fn unqualified_lease_opens_no_devices_and_reports_every_skipped_control() {
    let mut rig = Rig::new();
    let lease = rig.acquire(Instant::now(), Control::ALL.to_vec());
    assert_eq!(lease["applied"], json!([]));
    assert_eq!(lease["skipped"].as_array().unwrap().len(), 4);
    assert!(rig.devices.hardware.lock().calls.is_empty());
    assert!(!rig.devices.journal.exists());
}

#[test]
fn qualification_records_the_read_set_read_restore_read_sequence_and_requires_lifecycle_receipt() {
    let mut rig = Rig::new();
    let control = Control::MemcLowDelay;
    let result = rig.state.qualify(control, &rig.devices, None, None).unwrap();
    assert_eq!(result["sequencePassed"], true);
    assert_eq!(result["qualified"], false);
    assert_eq!(rig.devices.hardware.lock().calls, ["open MemcLowDelay", "read MemcLowDelay", "write MemcLowDelay 1", "read MemcLowDelay", "write MemcLowDelay 0", "read MemcLowDelay"]);
    assert_eq!(rig.qualify(control)["qualified"], true);
    assert!(!rig.devices.journal.exists());
    assert!(rig.restarted("boot-one").records[&control].qualified);
}

#[test]
fn cached_low_delay_getters_need_live_hardware_receipts_not_just_round_trips() {
    for control in [Control::VpqLowDelay, Control::DeLowDelay] {
        let mut rig = Rig::new();
        let result = rig.state.qualify(control, &rig.devices, None, Some("tv/lifecycle".into())).unwrap();
        assert_eq!(result["sequencePassed"], true);
        assert_eq!(result["qualified"], false);
        assert_eq!(rig.qualify(control)["qualified"], true);
    }
}

#[test]
fn renew_extends_six_second_lease_and_expiry_restores_exact_originals() {
    let mut rig = Rig::new();
    rig.qualify(Control::MemcLowDelay);
    let now = Instant::now();
    let lease = rig.acquire(now, vec![Control::MemcLowDelay]);
    let id = lease["id"].as_str().unwrap();
    assert_eq!(lease["renewEverySecs"], 2);
    rig.state.renew(id, now + RENEW).unwrap();
    rig.state.tick(now + EXPIRY, false, true).unwrap();
    assert!(rig.state.lease.is_some());
    rig.state.tick(now + RENEW + EXPIRY, false, true).unwrap();
    assert!(rig.state.lease.is_none());
    assert_eq!(rig.devices.hardware.lock().values[&Control::MemcLowDelay], 0);
    assert_eq!(rig.state.last_restore["controls"][0]["readback"], 0);
    assert!(!rig.devices.journal.exists());
}

#[test]
fn holder_death_standby_plugin_off_and_stop_restore_the_picture() {
    for reason in ["holder_died", "standby", "plugin_off", "stop"] {
        let mut rig = Rig::new();
        rig.qualify(Control::MotionPro);
        let now = Instant::now();
        let lease = rig.acquire(now, vec![Control::MotionPro]);
        match reason {
            "holder_died" => rig.state.tick(now, true, true).unwrap(),
            "standby" => rig.state.tick(now, false, false).unwrap(),
            "plugin_off" => { rig.state.enabled = false; rig.state.tick(now, false, true).unwrap(); }
            _ => rig.state.release(lease["id"].as_str().unwrap()).unwrap(),
        }
        assert!(rig.state.lease.is_none());
        assert_eq!(rig.state.last_restore["reason"], reason);
        assert_eq!(rig.devices.hardware.lock().values[&Control::MotionPro], 1);
    }
}

#[test]
fn failed_set_rolls_back_even_when_ioctl_changed_hardware_before_returning_an_error() {
    let mut rig = Rig::new();
    rig.qualify(Control::MemcLowDelay);
    rig.qualify(Control::MotionPro);
    rig.devices.hardware.lock().fail_set = Some(Control::MotionPro);
    let result = rig.state.acquire(holder(), vec![Control::MemcLowDelay, Control::MotionPro], &rig.devices, Instant::now());
    assert!(result.is_err());
    assert!(rig.state.lease.is_none());
    assert!(rig.state.entries.is_empty());
    let hardware = rig.devices.hardware.lock();
    assert_eq!(hardware.values[&Control::MemcLowDelay], 0);
    assert_eq!(hardware.values[&Control::MotionPro], 1);
}

#[test]
fn failed_restore_retains_original_and_blocks_new_sets_until_replayed() {
    let mut rig = Rig::new();
    rig.qualify(Control::MemcLowDelay);
    let lease = rig.acquire(Instant::now(), vec![Control::MemcLowDelay]);
    rig.devices.hardware.lock().fail_restore = true;
    assert!(rig.state.release(lease["id"].as_str().unwrap()).is_err());
    assert!(rig.devices.journal.exists());
    assert!(rig.state.acquire(holder(), vec![], &rig.devices, Instant::now()).is_err());
    let mut restarted = rig.restarted("boot-one");
    assert!(restarted.recover(&rig.devices).is_err());
    rig.devices.hardware.lock().fail_restore = false;
    restarted.recover(&rig.devices).unwrap();
    assert!(!rig.devices.journal.exists());
    assert_eq!(rig.devices.hardware.lock().values[&Control::MemcLowDelay], 0);
    assert!(restarted.error.is_none());
}

#[test]
fn helper_restart_restores_same_boot_but_discards_an_older_boot_journal() {
    for same_boot in [true, false] {
        let mut rig = Rig::new();
        rig.qualify(Control::MemcLowDelay);
        rig.acquire(Instant::now(), vec![Control::MemcLowDelay]);
        let mut restarted = rig.restarted(if same_boot { "boot-one" } else { "boot-two" });
        restarted.recover(&rig.devices).unwrap();
        assert_eq!(rig.devices.hardware.lock().values[&Control::MemcLowDelay], i32::from(!same_boot));
        assert!(!rig.devices.journal.exists());
    }
}

#[test]
fn changed_device_is_never_written_on_lease_or_journal_replay() {
    let mut rig = Rig::new();
    rig.qualify(Control::MemcLowDelay);
    rig.devices.hardware.lock().device_changed = true;
    let lease = rig.acquire(Instant::now(), vec![Control::MemcLowDelay]);
    assert_eq!(lease["skipped"][0]["reason"], "device_changed");
    rig.state.release(lease["id"].as_str().unwrap()).unwrap();
    rig.devices.hardware.lock().device_changed = false;
    rig.acquire(Instant::now(), vec![Control::MemcLowDelay]);
    rig.devices.hardware.lock().device_changed = true;
    let mut restarted = rig.restarted("boot-one");
    assert!(restarted.recover(&rig.devices).is_err());
    assert!(rig.devices.journal.exists());
}

#[test]
fn wrong_lease_or_expired_renewal_cannot_extend_or_release_the_owner() {
    let mut rig = Rig::new();
    let now = Instant::now();
    let lease = rig.acquire(now, vec![]);
    assert!(rig.state.renew("wrong", now).is_err());
    assert!(rig.state.release("wrong").is_err());
    assert!(rig.state.lease.is_some());
    assert!(rig.state.renew(lease["id"].as_str().unwrap(), now + EXPIRY).is_err());
    assert!(rig.state.lease.is_none());
}

#[test]
fn readback_mismatch_fails_qualification_and_still_restores() {
    let mut rig = Rig::new();
    rig.devices.hardware.lock().bad_readback = true;
    let result = rig.qualify(Control::MemcLowDelay);
    assert_eq!(result["qualified"], false);
    assert_eq!(result["sequencePassed"], false);
    assert_eq!(result["restored"], 0);
    assert!(!rig.devices.journal.exists());
}

#[test]
fn journal_failure_prevents_every_set_and_bad_journal_survives_plugin_off() {
    let mut rig = Rig::new();
    fs::create_dir_all(rig.devices.journal.parent().unwrap()).unwrap();
    fs::create_dir(&rig.devices.journal).unwrap();
    assert!(!rig.qualify(Control::MemcLowDelay)["qualified"].as_bool().unwrap());
    assert!(!rig.devices.hardware.lock().calls.iter().any(|call| call.starts_with("write")));
    fs::remove_dir(&rig.devices.journal).unwrap();
    fs::write(&rig.devices.journal, "broken").unwrap();
    let mut restarted = rig.restarted("boot-one");
    let error = restarted.recover(&rig.devices).unwrap_err();
    restarted.error = Some(error);
    assert!(restarted.restore("plugin_off").is_err());
    assert_eq!(fs::read_to_string(&rig.devices.journal).unwrap(), "broken");
}
