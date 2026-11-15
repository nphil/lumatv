//! Zhiwan USB packets from Leadjoy PC 1.4.0, `pipe/zhiwan_usb.rs`.
//! These are endpoint payloads, not hidraw buffers with an extra report-id byte.

use crate::{Error, Result};

pub const REPORT_LEN: usize = 64;
pub const SHORT_PAYLOAD: usize = 60;
pub const LONG_PAYLOAD: usize = 59;
// Bit 7 is the start flag. The PC writer checks 255, but truncates the count to seven bits.
pub const MAX_PAYLOAD: usize = LONG_PAYLOAD * 127;
pub type Report = [u8; REPORT_LEN];

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, &byte| sum.wrapping_add(byte))
}

pub fn encode(payload: &[u8]) -> Result<Vec<Report>> {
    if payload.is_empty() || payload.len() > MAX_PAYLOAD {
        return Err(Error::Invalid("payload must contain 1..=7493 bytes"));
    }
    if payload.len() <= SHORT_PAYLOAD {
        let mut report = [0; REPORT_LEN];
        let len = payload.len() + 4;
        report[..3].copy_from_slice(&[0xa5, len as u8, 0xcc]);
        report[3..len - 1].copy_from_slice(payload);
        report[len - 1] = checksum(&report[..len - 1]);
        return Ok(vec![report]);
    }
    let count = payload.len().div_ceil(LONG_PAYLOAD);
    let mut reports = Vec::with_capacity(count);
    for (index, chunk) in payload.chunks(LONG_PAYLOAD).enumerate() {
        let mut report = [0; REPORT_LEN];
        let len = chunk.len() + 5;
        let remaining = (count - index) as u8 | if index == 0 { 0x80 } else { 0 };
        report[..4].copy_from_slice(&[0xa4, len as u8, 0xcc, remaining]);
        report[4..len - 1].copy_from_slice(chunk);
        report[len - 1] = checksum(&report[..len - 1]);
        reports.push(report);
    }
    Ok(reports)
}

/// One in-flight response. A malformed or interrupted sequence discards the partial message.
#[derive(Default)]
pub struct Decoder {
    remaining: u8,
    bytes: Vec<u8>,
}

impl Decoder {
    pub fn push(&mut self, report: &[u8]) -> Result<Option<Vec<u8>>> {
        let result = self.accept(report);
        if result.is_err() {
            self.remaining = 0;
            self.bytes.clear();
        }
        result
    }

    fn accept(&mut self, report: &[u8]) -> Result<Option<Vec<u8>>> {
        if report.len() < 5 || report.len() > REPORT_LEN {
            return Err(Error::Invalid("USB packet length"));
        }
        let len = usize::from(report[1]);
        if len > report.len() || len < 5 || report[2] != 0xcc {
            return Err(Error::Invalid("USB packet header or declared length"));
        }
        if checksum(&report[..len - 1]) != report[len - 1] {
            return Err(Error::Invalid("USB packet checksum"));
        }
        match report[0] {
            0xa5 if self.remaining == 0 => Ok(Some(report[3..len - 1].to_vec())),
            0xa4 if len >= 6 => {
                let count = report[3] & 0x7f;
                let start = report[3] & 0x80 != 0;
                if count == 0 || (count > 1 && len != REPORT_LEN) {
                    return Err(Error::Invalid("long packet count or chunk length"));
                }
                if start {
                    if self.remaining != 0 || count < 2 {
                        return Err(Error::Invalid("unexpected long packet start"));
                    }
                    self.bytes.clear();
                    self.bytes.reserve(usize::from(count) * LONG_PAYLOAD);
                } else if self.remaining == 0 || count != self.remaining {
                    return Err(Error::Invalid("out-of-order long packet"));
                }
                self.bytes.extend_from_slice(&report[4..len - 1]);
                self.remaining = count - 1;
                Ok((self.remaining == 0).then(|| std::mem::take(&mut self.bytes)))
            }
            _ => Err(Error::Invalid("unexpected USB packet type")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_request_matches_pc_constructor() {
        let reports = encode(&[0x81, 0xa1, b'c', 12]).unwrap();
        assert_eq!(&reports[0][..8], &[0xa5, 8, 0xcc, 0x81, 0xa1, 0x63, 12, 0x0a]);
        assert!(reports[0][8..].iter().all(|&b| b == 0));
        assert_eq!(Decoder::default().push(&reports[0]).unwrap(), Some(vec![0x81, 0xa1, b'c', 12]));
    }

    #[test]
    fn boundary_sizes_round_trip() {
        for len in [1, 60, 61, 118, 119, MAX_PAYLOAD] {
            let payload: Vec<u8> = (0..len).map(|n| n as u8).collect();
            let reports = encode(&payload).unwrap();
            let mut decoder = Decoder::default();
            for report in &reports[..reports.len() - 1] {
                assert_eq!(decoder.push(report).unwrap(), None);
            }
            assert_eq!(decoder.push(reports.last().unwrap()).unwrap(), Some(payload));
        }
        let reports = encode(&[9; 61]).unwrap();
        assert_eq!(&reports[0][..4], &[0xa4, 64, 0xcc, 0x82]);
        assert_eq!(&reports[1][..4], &[0xa4, 7, 0xcc, 1]);
        assert!(encode(&[]).is_err());
        assert!(encode(&[0; MAX_PAYLOAD + 1]).is_err());
    }

    #[test]
    fn malformed_packets_and_sequence_errors_are_rejected() {
        let reports = encode(&[7; 119]).unwrap();
        let short = encode(&[1]).unwrap()[0];
        let mut decoder = Decoder::default();
        assert!(decoder.push(&reports[1]).is_err());
        assert!(decoder.push(&reports[0]).unwrap().is_none());
        assert!(decoder.push(&reports[2]).is_err());
        assert_eq!(decoder.push(&short).unwrap(), Some(vec![1]));
        assert!(decoder.push(&reports[0]).unwrap().is_none());
        assert!(decoder.push(&short).is_err());
        for (offset, value) in [(0, 0), (1, 0), (1, 65), (2, 0), (4, 0)] {
            let mut invalid = short;
            invalid[offset] = value;
            assert!(decoder.push(&invalid).is_err());
        }
        for len in 0..5 {
            assert!(decoder.push(&short[..len]).is_err());
        }
        let mut invalid = reports[0];
        invalid[3] = 0x80;
        invalid[63] = checksum(&invalid[..63]);
        assert!(decoder.push(&invalid).is_err());
    }
}
