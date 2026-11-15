//! Report framing, as GameSir Connect 1.14.5 builds it (`Ro` and the Cyclone 2 proxy in its `main.js`). Requests are 64-byte
//! output reports with id 0x0F; replies arrive as input report 0x10 and gamepad state as 0x12. Byte 1 is the command, or in a reply
//! its kind. Nothing is checksummed.

use crate::Error;

pub const VENDOR_ID: u16 = 0x3537;
pub const CONFIG_USAGE_PAGE: u16 = 0xFFF0;
pub const REQUEST_ID: u8 = 0x0F;
/// Connect takes every input report but gamepad state as a reply; captures on hardware (gamesir-linux-tools) show replies as 0x10.
pub const REPLY_ID: u8 = 0x10;
pub const REPORT_LEN: usize = 64;
/// Settings bytes per read or write report: what is left after id, command, region, 16-bit offset and length.
pub const CHUNK_LEN: usize = 58;
pub const PROFILE_LEN: usize = 680;
pub const LIGHT_LEN: usize = 635;

pub type Report = [u8; REPORT_LEN];

pub mod cmd {
    pub const WRITE: u8 = 0x03;
    pub const READ: u8 = 0x04;
    pub const SWITCH_PROFILE: u8 = 0x07;
    pub const READ_FIRMWARE_VERSION: u8 = 0x09;
    pub const READ_CURRENT_PROFILE: u8 = 0x0B;
    pub const VIBRATE: u8 = 0x20;
    pub const HEARTBEAT: u8 = 0xF2;
}

pub mod reply {
    pub const DATA: u8 = 0x05;
    pub const ACK: u8 = 0x06;
    pub const FIRMWARE_VERSION: u8 = 0x0A;
    pub const CURRENT_PROFILE: u8 = 0x0C;
    pub const PROFILE_CHANGED: u8 = 0x0F;
}

/// The region number a profile-changed notice carries when the active profile itself changed.
pub const ACTIVE_PROFILE_NOTICE: u8 = 0x30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Cyclone2,
    Cyclone2Adc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Cable,
    Receiver,
}

/// The configuration-mode product ids in GameSir Connect's model table.
pub fn identify(vendor: u16, product: u16) -> Option<(Model, Link)> {
    if vendor != VENDOR_ID {
        return None;
    }
    match product {
        0x101D => Some((Model::Cyclone2, Link::Cable)),
        0x102A => Some((Model::Cyclone2, Link::Receiver)),
        0x1053 => Some((Model::Cyclone2Adc, Link::Cable)),
        0x100B => Some((Model::Cyclone2Adc, Link::Receiver)),
        _ => None,
    }
}

/// A stored block of settings: profiles 1-4, the shift layer or the lights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region(u8);

impl Region {
    pub const SHIFT: Region = Region(5);
    pub const LIGHT: Region = Region(32);

    pub fn profile(slot: u8) -> Option<Region> {
        (1..=4).contains(&slot).then_some(Region(slot))
    }

    /// The profile slot 1-4 this region stores; `None` for the shift layer and the lights.
    pub fn slot(self) -> Option<u8> {
        Region::profile(self.0).map(|r| r.0)
    }

    pub fn number(self) -> u8 {
        self.0
    }

    pub fn size(self) -> usize {
        if self == Self::LIGHT { LIGHT_LEN } else { PROFILE_LEN }
    }
}

fn report(command: u8, args: &[u8]) -> Report {
    let mut r = [0; REPORT_LEN];
    r[0] = REQUEST_ID;
    r[1] = command;
    r[2..2 + args.len()].copy_from_slice(args);
    r
}

/// Connect sends this when it opens the device and every second after; the argument 1 is its button-test mode.
pub fn heartbeat() -> Report {
    report(cmd::HEARTBEAT, &[0])
}

pub fn read_firmware_version() -> Report {
    report(cmd::READ_FIRMWARE_VERSION, &[])
}

pub fn read_current_profile() -> Report {
    report(cmd::READ_CURRENT_PROFILE, &[])
}

pub fn switch_profile(region: Region) -> Report {
    report(cmd::SWITCH_PROFILE, &[region.number()])
}

/// Motor strengths 0-255. The motors keep running until the next vibrate report.
pub fn vibrate(left: u8, right: u8) -> Report {
    report(cmd::VIBRATE, &[0x66, 0x55, left, right])
}

fn chunk(command: u8, region: Region, offset: usize, len: usize) -> Report {
    let [hi, lo] = (offset as u16).to_be_bytes();
    report(command, &[region.number(), hi, lo, len as u8])
}

/// The read requests that fetch a whole region, in order.
pub fn read_plan(region: Region) -> Vec<Report> {
    (0..region.size()).step_by(CHUNK_LEN).map(|offset| chunk(cmd::READ, region, offset, CHUNK_LEN.min(region.size() - offset))).collect()
}

/// The write requests that turn `before` into `after`. Like Connect, only the span from the first to the last changed byte is sent.
pub fn write_plan(region: Region, before: &[u8], after: &[u8]) -> Result<Vec<Report>, Error> {
    for bytes in [before, after] {
        if bytes.len() != region.size() {
            return Err(Error::Length { expected: region.size(), actual: bytes.len() });
        }
    }
    let differs = |(a, b): (&u8, &u8)| a != b;
    let Some(start) = before.iter().zip(after).position(differs) else { return Ok(Vec::new()) };
    let end = before.iter().zip(after).rposition(differs).unwrap_or(start);
    Ok(after[start..=end]
        .chunks(CHUNK_LEN)
        .enumerate()
        .map(|(i, data)| {
            let mut r = chunk(cmd::WRITE, region, start + i * CHUNK_LEN, data.len());
            r[6..6 + data.len()].copy_from_slice(data);
            r
        })
        .collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirmwareVersion {
    pub controller: String,
    pub receiver: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Response<'a> {
    Data {
        region: u8,
        offset: u16,
        data: &'a [u8],
    },
    Ack {
        busy: bool,
    },
    FirmwareVersion(FirmwareVersion),
    CurrentProfile(u8),
    /// The controller changed stored settings itself, from its own buttons.
    ProfileChanged {
        region: u8,
        offset: u16,
        len: u16,
    },
    Other(u8),
}

/// Reads a configuration reply. Gamepad state (report 0x12), anything else not sent as report 0x10, and malformed reports give `None`.
pub fn parse(report: &[u8]) -> Option<Response<'_>> {
    if report.len() < 3 || report[0] != REPLY_ID {
        return None;
    }
    let be = |at: usize| Some(u16::from_be_bytes([*report.get(at)?, *report.get(at + 1)?]));
    Some(match report[1] {
        reply::DATA => {
            let len = usize::from(*report.get(5)?);
            Response::Data { region: report[2], offset: be(3)?, data: report.get(6..6 + len)? }
        }
        reply::ACK => Response::Ack { busy: report[2] == 1 },
        reply::FIRMWARE_VERSION => {
            Response::FirmwareVersion(FirmwareVersion { controller: version(report.get(4..9)?), receiver: version(report.get(12..17)?) })
        }
        reply::CURRENT_PROFILE => Response::CurrentProfile(report[2]),
        reply::PROFILE_CHANGED => Response::ProfileChanged { region: report[2], offset: be(3)?, len: be(5)? },
        kind => Response::Other(kind),
    })
}

/// Versions arrive as ASCII digits padded with spaces or NULs; Connect shows each digit as a dotted part, so "123" is 1.2.3.
fn version(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes.iter().filter(|b| b.is_ascii_graphic()) {
        if !out.is_empty() {
            out.push('.');
        }
        out.push(char::from(b));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_configuration_products_only() {
        assert_eq!(identify(0x3537, 0x102A), Some((Model::Cyclone2, Link::Receiver)));
        assert_eq!(identify(0x3537, 0x1053), Some((Model::Cyclone2Adc, Link::Cable)));
        assert_eq!(identify(0x054C, 0x09CC), None, "the receiver in DS4 mode has no configuration interface");
    }

    #[test]
    fn commands_match_connect() {
        assert_eq!(heartbeat()[..3], [0x0F, 0xF2, 0]);
        assert_eq!(read_firmware_version()[..2], [0x0F, 0x09]);
        assert_eq!(read_current_profile()[..2], [0x0F, 0x0B]);
        assert_eq!(switch_profile(Region::profile(3).unwrap())[..3], [0x0F, 0x07, 3]);
        assert_eq!(vibrate(0x80, 0x40)[..6], [0x0F, 0x20, 0x66, 0x55, 0x80, 0x40]);
        assert!(vibrate(1, 2)[6..].iter().all(|&b| b == 0), "reports are zero padded to 64 bytes");
        assert_eq!(Region::profile(5), None);
    }

    #[test]
    fn read_plan_walks_each_region_in_58_byte_chunks() {
        let profile = read_plan(Region::profile(2).unwrap());
        assert_eq!(profile.len(), 12);
        assert_eq!(profile[0][..6], [0x0F, 0x04, 2, 0, 0, 58]);
        assert_eq!(profile[11][..6], [0x0F, 0x04, 2, 0x02, 0x7E, 42], "680 = 11 * 58 + 42, last offset 638");

        let light = read_plan(Region::LIGHT);
        assert_eq!(light.len(), 11);
        assert_eq!(light[10][..6], [0x0F, 0x04, 32, 0x02, 0x44, 55], "635 = 10 * 58 + 55");
    }

    #[test]
    fn write_plan_sends_only_the_changed_span() {
        let region = Region::profile(1).unwrap();
        let before = vec![0; PROFILE_LEN];
        assert!(write_plan(region, &before, &before).unwrap().is_empty());

        let mut after = before.clone();
        after[46] = 2;
        after[100] = 7;
        let one = write_plan(region, &before, &after).unwrap();
        assert_eq!(one.len(), 1, "bytes 46..=100 fit one report");
        assert_eq!(one[0][..8], [0x0F, 0x03, 1, 0, 46, 55, 2, 0]);
        assert_eq!(one[0][6 + 54], 7);

        after[0] = 1;
        let two = write_plan(region, &before, &after).unwrap();
        assert_eq!(two.len(), 2, "bytes 0..=100 need 58 + 43");
        assert_eq!(two[0][..7], [0x0F, 0x03, 1, 0, 0, 58, 1]);
        assert_eq!(two[1][..6], [0x0F, 0x03, 1, 0, 58, 43]);
        assert_eq!(two[1][6 + 42], 7);
    }

    #[test]
    fn write_plan_rejects_wrong_sizes() {
        assert!(matches!(write_plan(Region::LIGHT, &[0; PROFILE_LEN], &[0; PROFILE_LEN]), Err(Error::Length { expected: LIGHT_LEN, .. })));
    }

    #[test]
    fn parses_replies() {
        let mut data = [0u8; REPORT_LEN];
        data[..9].copy_from_slice(&[0x10, 0x05, 1, 0x02, 0x7E, 3, 0xAA, 0xBB, 0xCC]);
        assert_eq!(parse(&data), Some(Response::Data { region: 1, offset: 638, data: &[0xAA, 0xBB, 0xCC] }));

        assert_eq!(parse(&[0x10, 0x06, 1]), Some(Response::Ack { busy: true }));
        assert_eq!(parse(&[0x10, 0x06, 0]), Some(Response::Ack { busy: false }));
        assert_eq!(parse(&[0x10, 0x0C, 2]), Some(Response::CurrentProfile(2)));
        assert_eq!(parse(&[0x10, 0x0F, 0x30, 0, 0, 0, 1]), Some(Response::ProfileChanged { region: 0x30, offset: 0, len: 1 }));
        assert_eq!(parse(&[0x10, 0xF2, 0]), Some(Response::Other(0xF2)));

        let mut fw = [0u8; 20];
        fw[..2].copy_from_slice(&[0x10, 0x0A]);
        fw[4..9].copy_from_slice(b"123  ");
        fw[12..17].copy_from_slice(b"121\0\0");
        let expected = FirmwareVersion { controller: "1.2.3".into(), receiver: "1.2.1".into() };
        assert_eq!(parse(&fw), Some(Response::FirmwareVersion(expected)));

        assert_eq!(parse(&[0x12, 0x80, 0x80]), None, "gamepad state is not a reply");
        assert_eq!(parse(&[0x0F, 0x06, 0]), None, "replies come as report 0x10, not with the request id");
        assert_eq!(parse(&[0x10, 0x05, 1, 0, 0, 58, 0]), None, "a data reply shorter than its length");
    }
}
