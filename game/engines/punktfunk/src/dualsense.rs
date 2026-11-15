// USB report layout adapted from client-webos platform/webos/{hidraw,dualsense}.rs.
use std::{fs::{File, OpenOptions}, io::{self, Write}, os::{fd::AsRawFd, unix::fs::OpenOptionsExt}};
use punktfunk_core::quic::HidOutput;

pub struct DualSense { file: File, report: [u8;63] }
fn ioctl(nr: u32, size: u32) -> libc::c_ulong { ((2 << 30) | (size << 16) | (u32::from(b'H') << 8) | nr) as libc::c_ulong }
impl DualSense {
    pub fn find(uniq: &str) -> Option<Self> {
        if uniq.is_empty() { return None; }
        for n in 0..10 {
            let Ok(file) = OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(format!("/dev/hidraw{n}")) else { continue; };
            #[repr(C)] #[derive(Default)] struct Info { bus:u32, vendor:i16, product:i16 }
            let mut info = Info::default();
            // HIDIOCGRAWINFO writes exactly the encoded eight-byte struct.
            if unsafe { libc::ioctl(file.as_raw_fd(), ioctl(3,8), &mut info) } < 0 || info.bus != 3 || info.vendor as u16 != 0x054c || ![0x0ce6,0x0df2].contains(&(info.product as u16)) { continue; }
            let mut buf = [0u8;128];
            // HIDIOCGRAWUNIQ is bounded by the buffer length in the request.
            if unsafe { libc::ioctl(file.as_raw_fd(), ioctl(8,128), buf.as_mut_ptr()) } <= 0 { continue; }
            let value = String::from_utf8_lossy(&buf[..buf.iter().position(|b| *b==0).unwrap_or(buf.len())]).trim().to_lowercase();
            if !value.eq_ignore_ascii_case(uniq) { continue; }
            let mut report = [0u8;63]; report[0]=2;
            return Some(Self {file,report});
        }
        None
    }
    pub fn apply(&mut self, output: &HidOutput) -> io::Result<()> {
        if !update(&mut self.report,output) { return Ok(()); }
        let n = self.file.write(&self.report)?;
        if n != self.report.len() { return Err(io::Error::new(io::ErrorKind::WriteZero,"Short DualSense output report")); }
        Ok(())
    }
    pub fn release(&mut self) -> io::Result<()> {
        self.report[1]|=0x0c;
        self.report[11..33].fill(0);
        self.file.write_all(&self.report)
    }
}
impl Drop for DualSense {
    fn drop(&mut self) {
        // Relinquish both triggers even when the last effect was still resisting.
        let mut release = [0u8;63]; release[0]=2; release[1]=0x0c;
        let _ = self.file.write(&release);
    }
}
fn update(report: &mut [u8;63], output: &HidOutput) -> bool {
    match output {
        HidOutput::Led {r,g,b,..} => { report[2] |= 4; report[45..48].copy_from_slice(&[*r,*g,*b]); }
        HidOutput::PlayerLeds {bits,..} => { report[2] |= 0x10; report[44] = bits & 0x1f; }
        HidOutput::Trigger {which,effect,..} if *which < 2 && effect.len() <= 11 => {
            let offset = if *which == 0 {22} else {11};
            report[1] |= if *which == 0 {8} else {4};
            report[offset..offset+11].fill(0);
            report[offset..offset+effect.len()].copy_from_slice(effect);
        }
        HidOutput::MicLed {mode,..} if *mode <= 2 => { report[2] |= 1; report[9] = *mode; }
        _ => return false,
    }
    true
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn usb_reports_keep_both_effects_and_never_copy_arbitrary_hid() {
        let mut r = [0;63]; r[0]=2;
        assert!(update(&mut r,&HidOutput::Trigger {pad:0,which:0,effect:vec![1,2,3]}));
        assert!(update(&mut r,&HidOutput::Led {pad:0,r:10,g:20,b:30}));
        assert_eq!(&r[22..25], &[1,2,3]); assert_eq!(&r[45..48], &[10,20,30]);
        assert!(!update(&mut r,&HidOutput::Trigger {pad:0,which:1,effect:vec![1;12]}));
        assert_eq!(r[1],8); assert_eq!(r[2],4);
    }
}
