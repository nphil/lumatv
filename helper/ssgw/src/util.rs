//! Small pieces LG's JavaScript gets from Node: truthiness, base64 as `Buffer` reads it, `path.join`/`path.basename`, HTTP dates.

use ring::digest;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

/// JavaScript truthiness of an optional JSON value (`undefined` is `None`).
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

pub fn sha1_hex(data: &[u8]) -> String {
    hex(digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, data).as_ref())
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, data).as_ref())
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    // the kernel's getrandom: failure would mean no entropy source at all
    SystemRandom::new().fill(&mut out).expect("no system randomness");
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = match chunk.len() {
            3 => (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]),
            2 => (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8),
            _ => u32::from(chunk[0]) << 16,
        };
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decodes base64 the way Node's `Buffer.from(s, "base64")` does: both alphabets, characters outside them skipped, stops at
/// the first `=`.
pub fn b64_decode_lenient(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// `Buffer.byteLength(s, "base64")`: the length the string claims, not what it decodes to.
pub fn b64_byte_length(s: &str) -> usize {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut n = units.len();
    if n > 0 && units[n - 1] == u16::from(b'=') {
        n -= 1;
    }
    if n > 1 && units[n - 1] == u16::from(b'=') {
        n -= 1;
    }
    ((n as u64 * 3) >> 2) as usize
}

/// Node's `path.basename` for a POSIX path.
pub fn basename(p: &str) -> &str {
    let trimmed = p.trim_end_matches('/');
    if trimmed.is_empty() {
        return "";
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// Node's `path.join(a, b)` for POSIX paths: joined with `/`, then normalised.
pub fn path_join(a: &str, b: &str) -> String {
    let joined = match (a.is_empty(), b.is_empty()) {
        (true, true) => return ".".to_owned(),
        (true, false) => b.to_owned(),
        (false, true) => a.to_owned(),
        (false, false) => format!("{a}/{b}"),
    };
    normalize(&joined)
}

fn normalize(p: &str) -> String {
    let absolute = p.starts_with('/');
    let trailing = p.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    let mut out = parts.join("/");
    if absolute {
        out.insert(0, '/');
    }
    if out.is_empty() {
        out.push('.');
    }
    if trailing && !out.ends_with('/') {
        out.push('/');
    }
    out
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `Date.prototype.toUTCString`: `Thu, 01 Jan 1970 00:00:00 GMT`.
pub fn http_date(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[days.rem_euclid(7) as usize],
        d,
        MONTHS[(m - 1) as usize],
        y,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// An IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`) as Unix milliseconds.
pub fn parse_http_date(s: &str) -> Option<i64> {
    let rest = s.trim().split_once(", ")?.1;
    let mut it = rest.split(' ');
    let d: u32 = it.next()?.parse().ok()?;
    let month = it.next()?;
    let m = MONTHS.iter().position(|x| x.eq_ignore_ascii_case(month))? as u32 + 1;
    let y: i64 = it.next()?.parse().ok()?;
    let mut hms = it.next()?.split(':');
    let h: i64 = hms.next()?.parse().ok()?;
    let mi: i64 = hms.next()?.parse().ok()?;
    let sec: i64 = hms.next()?.parse().ok()?;
    if it.next()? != "GMT" || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    Some((days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60 + sec) * 1000)
}

/// The set's local time: (year, month 1-12, day 1-31, hour 0-23).
pub fn local_now() -> (i64, i64, i64, i64) {
    // SAFETY: a zeroed tm is a valid value; time with a null pointer only returns the time; localtime_r writes only
    // into tm.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t = libc::time(std::ptr::null_mut());
        libc::localtime_r(&t, &mut tm);
        tm
    };
    (i64::from(tm.tm_year) + 1900, i64::from(tm.tm_mon) + 1, i64::from(tm.tm_mday), i64::from(tm.tm_hour))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base64_matches_node() {
        assert_eq!(b64_encode(b"com.webos.app.livetv"), "Y29tLndlYm9zLmFwcC5saXZldHY=");
        assert_eq!(b64_encode(b"ab"), "YWI=");
        assert_eq!(b64_decode_lenient("YW\nJj-_=garbage"), b"abc\xfb".to_vec());
        assert_eq!(b64_byte_length("YWJj"), 3);
        assert_eq!(b64_byte_length("YWI="), 2);
        assert_eq!(b64_byte_length("YQ=="), 1);
    }

    #[test]
    fn paths_match_node() {
        assert_eq!(path_join("/usr/palm/plugins/inputapps/assets/small/", "HDMI_1.png"), "/usr/palm/plugins/inputapps/assets/small/HDMI_1.png");
        assert_eq!(path_join("/a//b/../c", "./d"), "/a/c/d");
        assert_eq!(basename("/usr/palm/icon.png"), "icon.png");
        assert_eq!(basename("/a/b/"), "b");
    }

    #[test]
    fn http_dates_round_trip() {
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(http_date(784_111_777), "Sun, 06 Nov 1994 08:49:37 GMT");
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"), Some(784_111_777_000));
        assert_eq!(parse_http_date("yesterday"), None);
    }

    #[test]
    fn truthiness_matches_javascript() {
        assert!(!truthy(None));
        assert!(!truthy(Some(&json!(0))));
        assert!(!truthy(Some(&json!(""))));
        assert!(truthy(Some(&json!("0"))));
        assert!(truthy(Some(&json!([]))));
    }
}
