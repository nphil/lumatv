//! UTC days and timestamps, without a calendar crate: the listings speak ISO 8601 in UTC and Unix seconds, nothing else.

pub const MINUTE_MS: i64 = 60_000;
pub const HOUR_MS: i64 = 60 * MINUTE_MS;
pub const DAY_MS: i64 = 24 * HOUR_MS;

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day since 1970-01-01 (`civil_from_days`).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

/// The start of the UTC day of `ms`.
pub fn day_start(ms: i64) -> i64 {
    ms.div_euclid(DAY_MS) * DAY_MS
}

/// `2026-11-05`: the UTC date of `ms`.
pub fn date(ms: i64) -> String {
    let (y, m, d) = civil_from_days(ms.div_euclid(DAY_MS));
    format!("{y:04}-{m:02}-{d:02}")
}

/// The instant of `2026-11-05T18:00:00Z` or `2026-11-05T18:00Z`; anything else, a zone other than `Z` included, is `None`.
pub fn parse_utc(text: &str) -> Option<i64> {
    let text = text.trim().strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let mo: u32 = d.next()?.parse().ok()?;
    let da: u32 = d.next()?.parse().ok()?;
    let mut t = time.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let mi: i64 = t.next()?.parse().ok()?;
    let s: i64 = match t.next() {
        Some(s) => s.split('.').next()?.parse().ok()?,
        None => 0,
    };
    if d.next().is_some()
        || t.next().is_some()
        || !(1..=12).contains(&mo)
        || !(1..=31).contains(&da)
        || h > 23
        || mi > 59
        || s > 60
    {
        return None;
    }
    Some(((days_from_civil(y, mo, da) * 24 + h) * 60 + mi) * MINUTE_MS + s * 1000)
}

/// The start of the UTC day `2026-11-05`.
pub fn parse_date(text: &str) -> Option<i64> {
    parse_utc(&format!("{}T00:00Z", text.trim()))
}

/// XMLTV's `20261105180000 +0000`.
pub fn xmltv(ms: i64) -> String {
    let (y, m, d) = civil_from_days(ms.div_euclid(DAY_MS));
    let secs = ms.rem_euclid(DAY_MS) / 1000;
    format!(
        "{y:04}{m:02}{d:02}{:02}{:02}{:02} +0000",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_instants_round_trip() {
        let t = parse_utc("2026-11-05T18:00:00Z").unwrap();
        assert_eq!(t, 1_793_901_600_000);
        assert_eq!(parse_utc("2026-11-05T18:00Z"), Some(t));
        assert_eq!(date(t), "2026-11-05");
        assert_eq!(day_start(t), parse_date("2026-11-05").unwrap());
        assert_eq!(date(parse_date("2028-02-29").unwrap()), "2028-02-29");
        assert_eq!(xmltv(t + 61_000), "20261105180101 +0000");
        assert_eq!(parse_utc("2026-11-05T18:00:00+01:00"), None);
        assert_eq!(parse_utc("2026-13-05T18:00:00Z"), None);
    }
}
