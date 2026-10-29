//! The hardware the lab touches, and the only place bytes leave for it: [`Regs`] reads and writes a [`Target`], and [`LgRegs`] is the set's.
//!
//! A register is reached by mapping its one page of `/dev/mem` for the length of one access and unmapping it again (nothing is held open
//! between accesses, and `/dev/mem` is never opened for any address but a target's); a knob is a file under `/proc/lg/fbdev/chip/`, written
//! as the session's `echo` did and read back from `chip/diag`, where the driver reports what it holds (`sr_ctrl[0].ti_gain`,
//! `osd_sr.sw_status`). Everything goes through [`allowed`] first. `LgRegs` refuses to do anything on a machine where LG's `/proc/lg/fbdev`
//! nodes are not there, so a host that is not a TV can never be handed an address by accident: a physical read on a PC is not a harmless thing.
#![allow(unsafe_code)]

use super::allow::{REGISTER_PAGE, Target, allowed};
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// What the lab needs of the hardware.
pub trait Regs: Send + Sync {
    /// Whether this is the set the lab was written for and it can be reached; `Err` says why not, and the lab then touches nothing.
    fn check(&self) -> Result<(), String>;
    fn read(&self, target: Target) -> io::Result<u32>;
    /// Writes `value`, which has to be on the list ([`allowed`]); anything else is refused with `InvalidInput` before any access.
    fn write(&self, target: Target, value: u32) -> io::Result<()>;
}

/// The set's own: LG's driver nodes and `/dev/mem`.
pub struct LgRegs {
    chip: PathBuf,
    mem: PathBuf,
}

impl LgRegs {
    pub fn set() -> Self {
        Self::with_roots("/proc/lg/fbdev/chip", "/dev/mem")
    }

    /// Other roots, for a test that stands in for the driver's files. A `mem` that is not a character device is never mapped.
    pub fn with_roots(chip: impl Into<PathBuf>, mem: impl Into<PathBuf>) -> Self {
        Self {
            chip: chip.into(),
            mem: mem.into(),
        }
    }

    fn knob_path(&self, t: Target) -> io::Result<PathBuf> {
        t.knob().map(|k| self.chip.join(k)).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a knob", t.name()),
            )
        })
    }

    /// Runs `f` on the register at `addr` with its page mapped; `f` gets the register's pointer.
    fn with_register<R>(&self, addr: u32, f: impl FnOnce(*mut u32) -> R) -> io::Result<R> {
        if addr & !0xFFF != REGISTER_PAGE || addr % 4 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an address outside the lab's page",
            ));
        }
        self.check().map_err(io::Error::other)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_SYNC | libc::O_CLOEXEC)
            .open(&self.mem)?;
        if !file.metadata()?.file_type().is_char_device() {
            return Err(io::Error::other(
                "the memory device is not a character device",
            ));
        }
        // SAFETY: one page of a character device the checks above name, mapped shared and read-write for this call only; `addr` is a
        // 4-aligned offset inside that page (checked above), so the pointer is valid and aligned for a 32-bit volatile access until the
        // unmap below, and nothing else in this process holds the mapping.
        unsafe {
            let map = libc::mmap(
                std::ptr::null_mut(),
                4096,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                libc::off_t::from(REGISTER_PAGE),
            );
            if map == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            let r = f(map.cast::<u8>().add((addr & 0xFFF) as usize).cast::<u32>());
            libc::munmap(map, 4096);
            Ok(r)
        }
    }
}

/// `0x00000010`, `0x1` or `16` as a number.
fn parse_number(text: &str) -> Option<u32> {
    let t = text.trim();
    match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => t.parse().ok(),
    }
}

/// What `chip/diag` says of the sharpener: the gain (`sr_ctrl[0].ti_gain`) and whether it is on (`osd_sr.sw_status`).
pub fn parse_diag(text: &str) -> (Option<u32>, Option<u32>) {
    let (mut gain, mut on) = (None, None);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("sr_ctrl[0].ti_gain:") {
            gain = parse_number(rest);
        } else if let Some(rest) = line.strip_prefix("osd_sr.sw_status:") {
            on = parse_number(rest);
        }
    }
    (gain, on)
}

impl Regs for LgRegs {
    fn check(&self) -> Result<(), String> {
        for name in ["osd_sr_gain", "osd_sr", "diag"] {
            if !self.chip.join(name).exists() {
                return Err(format!(
                    "not an LG set: {} is missing",
                    self.chip.join(name).display()
                ));
            }
        }
        let regdump = self
            .chip
            .parent()
            .map(|p| p.join("regdump"))
            .unwrap_or_default();
        if !regdump.exists() {
            return Err(format!("not an LG set: {} is missing", regdump.display()));
        }
        match std::fs::metadata(&self.mem) {
            Ok(m) if m.file_type().is_char_device() => Ok(()),
            Ok(_) => Err(format!("{} is not a memory device", self.mem.display())),
            Err(e) => Err(format!("{}: {e}", self.mem.display())),
        }
    }

    fn read(&self, target: Target) -> io::Result<u32> {
        match target.address() {
            // SAFETY: `with_register` hands over a valid, aligned pointer into the mapped page for the length of the call.
            Some(addr) => self.with_register(addr, |p| unsafe { p.read_volatile() }),
            None => {
                let text = std::fs::read_to_string(self.chip.join("diag"))?;
                let (gain, on) = parse_diag(&text);
                let found = if target == Target::Gain { gain } else { on };
                found.ok_or_else(|| {
                    io::Error::other(format!(
                        "chip/diag does not say what {} holds",
                        target.name()
                    ))
                })
            }
        }
    }

    fn write(&self, target: Target, value: u32) -> io::Result<()> {
        if !allowed(target, value) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{value:#x} is not on the list for {}", target.name()),
            ));
        }
        match target.address() {
            // SAFETY: as in `read`.
            Some(addr) => self.with_register(addr, |p| unsafe { p.write_volatile(value) }),
            None => {
                use std::io::Write;
                let path = self.knob_path(target)?;
                // as `echo 0x80 > osd_sr_gain` and `echo 0 > osd_sr` did on the set: one write of the text and a newline
                let text = if target == Target::Gain {
                    format!("0x{value:02x}\n")
                } else {
                    format!("{value}\n")
                };
                let mut file = OpenOptions::new().write(true).open(Path::new(&path))?;
                file.write_all(text.as_bytes())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `docs/research/compositor/layerdeep-2026-10-29/chip-diag.txt`, the lines the lab reads.
    const DIAG: &str = "sr_ctrl[0].status: 0x00000001\nsr_ctrl[0].ti_gain: 0x00000010\nsr_ctrl[1].status: 0x00000000\nsr_ctrl[1].ti_gain: 0xffffffff\n\nosd_sr.sw_status: 0x1\nosd_sr.hw_status: 0x1\n";

    fn fake_chip() -> (tempfile::TempDir, LgRegs) {
        let dir = tempfile::tempdir().unwrap();
        let chip = dir.path().join("fbdev/chip");
        std::fs::create_dir_all(&chip).unwrap();
        for f in ["osd_sr_gain", "osd_sr"] {
            std::fs::write(chip.join(f), "").unwrap();
        }
        std::fs::write(chip.join("diag"), DIAG).unwrap();
        std::fs::write(dir.path().join("fbdev/regdump"), "").unwrap();
        // a plain file in the place of /dev/mem: never a character device, so never mapped
        std::fs::write(dir.path().join("mem"), "").unwrap();
        let regs = LgRegs::with_roots(&chip, dir.path().join("mem"));
        (dir, regs)
    }

    #[test]
    fn the_diag_lines_say_what_the_driver_holds() {
        assert_eq!(
            parse_diag(DIAG),
            (Some(0x10), Some(1)),
            "the gain of sr_ctrl[0], not the unused [1]"
        );
        assert_eq!(parse_diag("osd_sr.sw_status: 0x0\n"), (None, Some(0)));
        assert_eq!(parse_diag("nothing of the kind"), (None, None));
        assert_eq!(parse_number("0x80"), Some(0x80));
        assert_eq!(parse_number(" 16 "), Some(16));
        assert_eq!(parse_number("0xzz"), None);
    }

    #[test]
    fn a_machine_that_is_not_the_set_is_not_touched() {
        let none = LgRegs::with_roots("/nonexistent/lg/fbdev/chip", "/nonexistent/mem");
        let why = none.check().unwrap_err();
        assert!(why.contains("not an LG set"), "{why}");
        assert!(none.read(Target::Vsp).is_err() && none.write(Target::Vsp, 0x0100_0001).is_err());
        // the real nodes of this machine: on a PC they are not there, and the lab stays out
        if !Path::new("/proc/lg/fbdev/chip/diag").exists() {
            assert!(LgRegs::set().check().is_err());
            assert!(
                LgRegs::set().read(Target::Vsp).is_err(),
                "no mapping of /dev/mem on a machine without LG's nodes"
            );
        }
    }

    #[test]
    fn a_mem_that_is_not_a_character_device_is_never_mapped() {
        let (_dir, regs) = fake_chip();
        assert!(regs.check().unwrap_err().contains("not a memory device"));
        let e = regs.read(Target::Vsp).unwrap_err();
        assert!(e.to_string().contains("not a memory device"), "{e}");
        assert!(regs.write(Target::HoleSel, 1).is_err());
    }

    #[test]
    fn the_knobs_are_read_from_diag_and_written_as_the_session_wrote_them() {
        let (dir, regs) = fake_chip();
        assert_eq!(regs.read(Target::Gain).unwrap(), 0x10);
        assert_eq!(regs.read(Target::Sharpener).unwrap(), 1);
        regs.write(Target::Gain, 0x80).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("fbdev/chip/osd_sr_gain")).unwrap(),
            "0x80\n"
        );
        regs.write(Target::Gain, 0x00).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("fbdev/chip/osd_sr_gain")).unwrap(),
            "0x00\n"
        );
        regs.write(Target::Sharpener, 0).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("fbdev/chip/osd_sr")).unwrap(),
            "0\n"
        );
        // the original is a plausible value too: the set's own 0x10 goes back as it was
        regs.write(Target::Gain, 0x10).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("fbdev/chip/osd_sr_gain")).unwrap(),
            "0x10\n"
        );
    }

    #[test]
    fn a_value_off_the_list_is_refused_before_any_file_or_page_is_touched() {
        let (dir, regs) = fake_chip();
        for (t, v) in [
            (Target::Gain, 0x1FF),
            (Target::Gain, 0xFFFF_FFFF),
            (Target::Sharpener, 2),
            (Target::Vsp, 0xFFFF_FFFF),
            (Target::Vsp, 0x0400_0001),
            (Target::HoleX, 0x1234_5678),
            (Target::HoleSel, 0x7),
        ] {
            let e = regs.write(t, v).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{t:?} {v:#x}: {e}");
            assert!(e.to_string().contains("not on the list"), "{e}");
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("fbdev/chip/osd_sr_gain")).unwrap(),
            "",
            "nothing was written"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("fbdev/chip/osd_sr")).unwrap(),
            ""
        );
    }

    #[test]
    fn a_register_of_another_page_cannot_be_named() {
        // `with_register` is private, but its guard is the last word on an address
        let (_dir, regs) = fake_chip();
        assert!(regs.with_register(0xCC48_1000, |_| ()).is_err());
        assert!(
            regs.with_register(0xCC48_0361, |_| ()).is_err(),
            "misaligned"
        );
        assert!(regs.with_register(0, |_| ()).is_err());
        assert!(regs.with_register(0x1000_0000, |_| ()).is_err());
    }
}

/// A set in memory for the lab's tests: every target a number, every write kept, and the ways hardware misbehaves (a register the firmware
/// takes back, a write or a read that fails) on switches. Its writes go through the same list as the real ones: a value that is not on it is
/// refused here too, so a test that passes has shown the lab never asks for one.
#[cfg(test)]
pub mod fake {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicUsize, Ordering};

    type Hook = Box<dyn Fn(Target, u32) + Send + Sync>;

    pub struct FakeRegs {
        available: bool,
        original: BTreeMap<Target, u32>,
        values: Mutex<BTreeMap<Target, u32>>,
        writes: Mutex<Vec<(Target, u32)>>,
        reads: AtomicUsize,
        sticky: Mutex<BTreeSet<Target>>,
        failing_writes: Mutex<BTreeSet<Target>>,
        failing_reads: Mutex<BTreeSet<Target>>,
        hook: Mutex<Option<Hook>>,
    }

    impl FakeRegs {
        fn with(available: bool) -> Self {
            // what the set read on 2026-10-29: the gain 0x10, the sharpener on, the VSP clear on this boot, the holes clear
            let original: BTreeMap<Target, u32> = [
                (Target::Gain, 0x10),
                (Target::Sharpener, 1),
                (Target::Vsp, 0),
                (Target::HoleX, 0),
                (Target::HoleY, 0),
                (Target::HoleSel, 0),
            ]
            .into();
            Self {
                available,
                values: Mutex::new(original.clone()),
                original,
                writes: Mutex::new(Vec::new()),
                reads: AtomicUsize::new(0),
                sticky: Mutex::new(BTreeSet::new()),
                failing_writes: Mutex::new(BTreeSet::new()),
                failing_reads: Mutex::new(BTreeSet::new()),
                hook: Mutex::new(None),
            }
        }

        pub fn the_set() -> Self {
            Self::with(true)
        }

        pub fn not_the_set() -> Self {
            Self::with(false)
        }

        pub fn get(&self, t: Target) -> u32 {
            self.values.lock()[&t]
        }

        pub fn original(&self, t: Target) -> u32 {
            self.original[&t]
        }

        /// Somebody else changes a target (the firmware, a reboot): not a write of the lab's.
        pub fn force(&self, t: Target, v: u32) {
            self.values.lock().insert(t, v);
        }

        pub fn writes(&self) -> Vec<(Target, u32)> {
            self.writes.lock().clone()
        }

        pub fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }

        pub fn dump(&self) -> String {
            self.values
                .lock()
                .iter()
                .map(|(t, v)| format!("{}={v:#x}", t.name()))
                .collect::<Vec<_>>()
                .join(" ")
        }

        /// Called with each write just before it is applied.
        pub fn on_write(&self, f: impl Fn(Target, u32) + Send + Sync + 'static) {
            *self.hook.lock() = Some(Box::new(f));
        }

        /// The target takes writes and ignores them, as a register the firmware rewrites does.
        pub fn stick(&self, t: Target) {
            self.sticky.lock().insert(t);
        }

        pub fn unstick(&self, t: Target) {
            self.sticky.lock().remove(&t);
        }

        pub fn fail_writes(&self, t: Target) {
            self.failing_writes.lock().insert(t);
        }

        pub fn fail_writes_off(&self) {
            self.failing_writes.lock().clear();
        }

        pub fn fail_reads(&self, t: Target) {
            self.failing_reads.lock().insert(t);
        }
    }

    impl Regs for FakeRegs {
        fn check(&self) -> Result<(), String> {
            if self.available {
                Ok(())
            } else {
                Err("not an LG set: the fake has no LG nodes".to_owned())
            }
        }

        fn read(&self, target: Target) -> io::Result<u32> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.failing_reads.lock().contains(&target) {
                return Err(io::Error::other("a read that fails"));
            }
            Ok(self.get(target))
        }

        fn write(&self, target: Target, value: u32) -> io::Result<()> {
            if !allowed(target, value) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{value:#x} is not on the list for {}", target.name()),
                ));
            }
            if self.failing_writes.lock().contains(&target) {
                return Err(io::Error::other("a write that fails"));
            }
            if let Some(hook) = self.hook.lock().as_ref() {
                hook(target, value);
            }
            self.writes.lock().push((target, value));
            if !self.sticky.lock().contains(&target) {
                self.values.lock().insert(target, value);
            }
            Ok(())
        }
    }
}
