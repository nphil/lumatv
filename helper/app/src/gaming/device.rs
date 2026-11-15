//! Fixed LG controls; no request supplies a device path, ioctl or arbitrary value.
#![allow(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    MemcLowDelay,
    VpqLowDelay,
    MotionPro,
    DeLowDelay,
}

impl Control {
    pub const ALL: [Self; 4] = [Self::MemcLowDelay, Self::VpqLowDelay, Self::MotionPro, Self::DeLowDelay];

    pub fn target(self) -> i32 {
        i32::from(self != Self::MotionPro)
    }

    fn id(self) -> Option<u32> {
        match self {
            Self::MemcLowDelay => Some(0x0098_3a01),
            Self::VpqLowDelay => Some(0x0098_3d0e),
            Self::MotionPro => Some(0x0098_3a03),
            Self::DeLowDelay => None,
        }
    }

    pub fn cache_caveat(self) -> bool {
        matches!(self, Self::VpqLowDelay | Self::DeLowDelay)
    }
}

/// An opened device set is held until restoration, including after a failed write.
pub trait ControlFd: Send {
    fn identity(&self) -> &str;
    fn read(&mut self, control: Control) -> io::Result<i32>;
    fn write(&mut self, control: Control, value: i32) -> io::Result<()>;
}

pub trait Devices: Send + Sync {
    fn open(&self, controls: &[Control]) -> io::Result<Box<dyn ControlFd>>;
}

pub struct LgDevices {
    sys: PathBuf,
    dev: PathBuf,
    de_status: PathBuf,
}

impl Default for LgDevices {
    fn default() -> Self {
        Self {
            sys: "/sys".into(),
            dev: "/dev".into(),
            de_status: "/proc/lg/de/win0/status".into(),
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn char_device(path: &Path, expected: &str) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.file_type().is_char_device() {
        return Err(invalid("gaming device is not a character device"));
    }
    let device = format!("{}:{}", libc::major(meta.rdev()), libc::minor(meta.rdev()));
    if device != expected.trim() {
        return Err(invalid("gaming device does not match its sysfs registration"));
    }
    Ok(file)
}

impl LgDevices {
    fn vpq(&self) -> io::Result<File> {
        let platform = fs::canonicalize(self.sys.join("devices/platform/v4l2_vpq"))?;
        let mut found = None;
        for entry in fs::read_dir(self.sys.join("class/video4linux"))? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.strip_prefix("video").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
                continue;
            }
            if fs::canonicalize(entry.path())?.starts_with(&platform) {
                if found.is_some() {
                    return Err(invalid("multiple VPQ video devices; qualification must resolve the device"));
                }
                found = Some(char_device(&self.dev.join(name.as_ref()), &fs::read_to_string(entry.path().join("dev"))?)?);
            }
        }
        found.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "VPQ video device is not registered"))
    }

    fn de(&self) -> io::Result<File> {
        // The DE ioctls are served by /dev/lg/de0, not a resource_mgr device node.
        let mut found = None;
        for entry in fs::read_dir(self.sys.join("dev/char"))? {
            let entry = entry?;
            let Ok(event) = fs::read_to_string(entry.path().join("uevent")) else { continue };
            let name = event.lines().find_map(|line| line.strip_prefix("DEVNAME="));
            if let Some(name @ "lg/de0") = name {
                if found.is_some() { return Err(invalid("multiple DE devices")); }
                found = Some(char_device(&self.dev.join(name), &entry.file_name().to_string_lossy())?);
            }
        }
        found.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "lg/de0 character device is not registered"))
    }
}

impl Devices for LgDevices {
    fn open(&self, controls: &[Control]) -> io::Result<Box<dyn ControlFd>> {
        let vpq = controls.iter().any(|c| c.id().is_some()).then(|| self.vpq()).transpose()?;
        let de = controls.contains(&Control::DeLowDelay).then(|| self.de()).transpose()?;
        let mut identity = String::new();
        for (name, file) in [("vpq", &vpq), ("lg/de0", &de)] {
            if let Some(file) = file {
                identity.push_str(&format!("{name}:{};", file.metadata()?.rdev()));
            }
        }
        Ok(Box::new(LgFd { vpq, de, de_status: self.de_status.clone(), identity }))
    }
}

struct LgFd {
    vpq: Option<File>,
    de: Option<File>,
    de_status: PathBuf,
    identity: String,
}

#[repr(C)]
struct V4l2Control {
    id: u32,
    value: i32,
}

fn ioctl<T>(file: &File, request: u32, value: &mut T) -> io::Result<()> {
    // SAFETY: payloads match the fixed ioctl; musl takes a signed request, glibc unsigned.
    if unsafe { libc::ioctl(file.as_raw_fd(), request as _, value as *mut T) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn required(file: &Option<File>) -> io::Result<&File> {
    file.as_ref().ok_or_else(|| invalid("control's device was not opened"))
}

impl ControlFd for LgFd {
    fn identity(&self) -> &str { &self.identity }

    fn read(&mut self, control: Control) -> io::Result<i32> {
        if let Some(id) = control.id() {
            let mut value = V4l2Control { id, value: 0 };
            ioctl(required(&self.vpq)?, 0xc008_561b, &mut value)?; // VIDIOC_G_CTRL
            Ok(value.value)
        } else {
            // No getter ioctl has been established. This driver's status read is explicitly
            // unqualified until a TV receipt proves it is not de_kwrap's cached value.
            fs::read_to_string(&self.de_status)?.lines()
                .find_map(|line| line.strip_prefix("lowdelaymode=").and_then(|v| v.trim().parse::<i32>().ok()))
                .filter(|value| matches!(value, 0 | 1))
                .ok_or_else(|| invalid("DE status has no boolean lowdelaymode value"))
        }
    }

    fn write(&mut self, control: Control, value: i32) -> io::Result<()> {
        if let Some(id) = control.id() {
            ioctl(required(&self.vpq)?, 0xc008_561c, &mut V4l2Control { id, value }) // VIDIOC_S_CTRL
        } else {
            // The DE driver translates this ioctl to DE IPC 0x49, with a four-byte payload.
            let mut value = u32::try_from(value).map_err(|_| invalid("negative DE low-delay value"))?;
            ioctl(required(&self.de)?, 0x4004_613c, &mut value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_control_ids_and_targets_match_the_qualification_contract() {
        assert_eq!(Control::MemcLowDelay.id(), Some(0x0098_3a01));
        assert_eq!(Control::VpqLowDelay.id(), Some(0x0098_3d0e));
        assert_eq!(Control::MotionPro.id(), Some(0x0098_3a03));
        assert_eq!(Control::MotionPro.target(), 0);
        assert_eq!(Control::DeLowDelay.target(), 1);
    }

    #[test]
    fn ordinary_files_are_never_accepted_as_device_fds() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        assert!(char_device(temp.path(), "1:3").is_err());
    }

    #[test]
    fn de_discovery_uses_the_registered_lg_de0_node_without_resource_mgr() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path().join("sys");
        let dev = dir.path().join("dev");
        fs::create_dir_all(sys.join("dev/char/1:3")).unwrap();
        fs::write(sys.join("dev/char/1:3/uevent"), "MAJOR=1\nMINOR=3\nDEVNAME=lg/de0\n").unwrap();
        fs::create_dir_all(dev.join("lg")).unwrap();
        fs::write(dev.join("lg/de0"), "").unwrap();
        let devices = LgDevices { sys, dev, de_status: dir.path().join("status") };
        let error = devices.de().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "gaming device is not a character device");
    }
}
