//! The words of the Cameras screens (`docs/research/cameras-ui-spec.md`, "Rules for every screen" and "Words of a refusal"): what a
//! state, a kind, a time and a refusal are called. All copy of the app is here, as one table per area (plan section 2.9), so the
//! surfaces draw text and never compose it, and a test states every rule. English ships; the table is the extension point.
//!
//! Times follow `theme.clock.twelveHour` (default on) in the TV's own zone: "2:31 PM"; today has no day word, then "Yesterday", then
//! "Oct 27". Ago: under 60 s "Just now", under 90 min "N min ago", under 24 h "N h ago", then "Yesterday", "N days ago". No word
//! carries an address, a token or a stream address.

/// The TV's time zone, for the clock a person reads. A fixed offset serves the tests; the system's zone serves the set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    fixed: Option<i32>,
}

impl Zone {
    pub const UTC: Zone = Zone { fixed: Some(0) };

    /// A fixed offset from UTC in seconds (east is positive).
    pub fn fixed(seconds: i32) -> Self {
        Self { fixed: Some(seconds) }
    }

    /// The zone the system is set to (`TZ`, `/etc/localtime`), as `localtime_r` reads it at the moment asked about.
    pub fn system() -> Self {
        Self { fixed: None }
    }

    /// The offset from UTC at `epoch_s`, in seconds.
    pub fn offset_s(&self, epoch_s: i64) -> i32 {
        if let Some(o) = self.fixed {
            return o;
        }
        let t: libc::time_t = epoch_s as libc::time_t;
        // SAFETY: `localtime_r` writes only into the `tm` it is given and reads `t` by reference; both are live locals.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&t, &mut tm).is_null() {
                return 0;
            }
            tm.tm_gmtoff as i32
        }
    }

    /// The local calendar fields of `epoch_ms`.
    pub fn civil(&self, epoch_ms: i64) -> Civil {
        let s = epoch_ms.div_euclid(1000);
        let local = s + i64::from(self.offset_s(s));
        let days = local.div_euclid(86_400);
        let secs = local.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Civil { days, year, month, day, hour: (secs / 3600) as u32, minute: (secs % 3600 / 60) as u32, second: (secs % 60) as u32 }
    }

    /// Epoch milliseconds of the first moment of local day `days` (days since 1970-01-01 on the local calendar).
    pub fn day_start_ms(&self, days: i64) -> i64 {
        // the offset at the day's start: ask at the UTC reading first, then correct once for a zone that changes on that day
        let guess = days * 86_400 - i64::from(self.offset_s(days * 86_400));
        let corrected = days * 86_400 - i64::from(self.offset_s(guess));
        corrected * 1000
    }

    /// The local seconds since local midnight at `epoch_ms` plus the zone's offset: what multiples of a step are counted from.
    pub fn local_ms(&self, epoch_ms: i64) -> i64 {
        epoch_ms + i64::from(self.offset_s(epoch_ms.div_euclid(1000))) * 1000
    }
}

/// A moment on the local calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    /// Days since 1970-01-01 on the local calendar.
    pub days: i64,
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// How times are written: the owner's 12 or 24 hour clock in the TV's zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    pub twelve_hour: bool,
    pub zone: Zone,
}

impl Default for Clock {
    fn default() -> Self {
        Self { twelve_hour: true, zone: Zone::system() }
    }
}

impl Clock {
    fn hm(&self, c: &Civil, seconds: bool) -> String {
        let tail = if seconds { format!(":{:02}", c.second) } else { String::new() };
        if self.twelve_hour {
            let h = c.hour % 12;
            format!("{}:{:02}{} {}", if h == 0 { 12 } else { h }, c.minute, tail, if c.hour < 12 { "AM" } else { "PM" })
        } else {
            format!("{:02}:{:02}{}", c.hour, c.minute, tail)
        }
    }

    /// "2:31 PM" (or "14:31").
    pub fn clock(&self, ms: i64) -> String {
        self.hm(&self.zone.civil(ms), false)
    }

    /// "2:12:20 PM": the playhead of a recording that plays.
    pub fn clock_seconds(&self, ms: i64) -> String {
        self.hm(&self.zone.civil(ms), true)
    }

    /// `None` for today, "Yesterday", else "Oct 27".
    pub fn day_word(&self, ms: i64, now_ms: i64) -> Option<String> {
        let (d, n) = (self.zone.civil(ms), self.zone.civil(now_ms));
        match n.days - d.days {
            0 => None,
            1 => Some("Yesterday".to_string()),
            _ => Some(format!("{} {}", MONTHS[(d.month - 1) as usize], d.day)),
        }
    }

    /// The moment with its day: "2:31 PM", "Yesterday 10:10 PM", "Oct 27 6:47 PM".
    pub fn stamp(&self, ms: i64, now_ms: i64) -> String {
        match self.day_word(ms, now_ms) {
            None => self.clock(ms),
            Some(day) => format!("{day} {}", self.clock(ms)),
        }
    }

    /// The second line of an event's or a clip's card: today the time and how long ago ("2:31 PM · 6 min ago"), another day its date
    /// and time.
    pub fn card_line(&self, ms: i64, now_ms: i64) -> String {
        match self.day_word(ms, now_ms) {
            None => format!("{} \u{b7} {}", self.clock(ms), ago(now_ms - ms)),
            Some(_) => self.stamp(ms, now_ms),
        }
    }
}

/// How long ago, for a moment that is `delta_ms` behind now: a bridge clock a little ahead never shows a negative age.
pub fn ago(delta_ms: i64) -> String {
    let s = delta_ms.max(0) / 1000;
    if s < 60 {
        return "Just now".to_string();
    }
    if s < 90 * 60 {
        return format!("{} min ago", (s + 30) / 60);
    }
    let h = s / 3600;
    if h < 24 {
        return format!("{h} h ago");
    }
    match h / 24 {
        1 => "Yesterday".to_string(),
        d => format!("{d} days ago"),
    }
}

/// How old a picture is, for a tile that shows its last picture dimmed: "12 s ago", "3 min ago", "2 h ago".
pub fn age(delta_ms: i64) -> String {
    let s = delta_ms.max(0) / 1000;
    if s < 5 {
        "Just now".to_string()
    } else if s < 60 {
        format!("{s} s ago")
    } else {
        ago(delta_ms)
    }
}

/// How far behind live the cursor is: "Now", "42 s ago", "5 min ago", "3 h 25 min ago".
pub fn behind(delta_ms: i64) -> String {
    let s = delta_ms.max(0) / 1000;
    if s < 5 {
        "Now".to_string()
    } else if s < 60 {
        format!("{s} s ago")
    } else if s < 3600 {
        format!("{} min ago", s / 60)
    } else {
        let (h, m) = (s / 3600, s % 3600 / 60);
        if m == 0 { format!("{h} h ago") } else { format!("{h} h {m} min ago") }
    }
}

/// A clip's length: "0:30".
pub fn length(ms: i64) -> String {
    let s = (ms.max(0) + 500) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// "8 cameras", "1 camera".
pub fn count(n: usize) -> String {
    format!("{n} camera{}", if n == 1 { "" } else { "s" })
}

/// "2 of 8".
pub fn of(position: usize, total: usize) -> String {
    format!("{position} of {total}")
}

/// The range of cameras still below the shown rows: "9 to 12".
pub fn range_below(first: usize, last: usize) -> String {
    if first == last { format!("{first}") } else { format!("{first} to {last}") }
}

/// What the link is, in words that carry no address and no token (`cameras.state`).
pub fn link_words(state: &str) -> &'static str {
    match state {
        "unlinked" => "Scrypted isn\u{2019}t linked",
        "connecting" => "Connecting to Scrypted",
        "refused" => "Scrypted refused this TV",
        "untrusted" => "Can\u{2019}t verify Scrypted",
        "offline" => "Can\u{2019}t reach Scrypted",
        _ => "",
    }
}

/// Linked and answering, with no camera shared.
pub const NO_CAMERAS: &str = "No cameras shared";

/// The kinds an event has (`cameras.popup`, the bridge's `kind`), in the order Settings lists them.
pub const KINDS: [&str; 6] = ["person", "vehicle", "animal", "package", "doorbell", "motion"];

/// The word of a kind: "Person", "Vehicle", "Animal", "Package", "Doorbell rang", "Motion"; any other kind is "Event".
pub fn kind_word(kind: &str) -> &'static str {
    match kind {
        "person" => "Person",
        "vehicle" => "Vehicle",
        "animal" => "Animal",
        "package" => "Package",
        "doorbell" => "Doorbell rang",
        "motion" => "Motion",
        _ => "Event",
    }
}

/// A kind as Settings names it: "Doorbell", not "Doorbell rang".
pub fn kind_name(kind: &str) -> &'static str {
    match kind {
        "doorbell" => "Doorbell",
        k => kind_word(k),
    }
}

/// The Lucide glyph of a kind.
pub fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "person" => "user",
        "vehicle" => "car",
        "animal" => "paw-print",
        "package" => "package",
        "doorbell" => "bell-ring",
        "motion" => "activity",
        _ => "camera",
    }
}

/// A recorder's label as words: a class name used as a label (`dog_cat`) reads as "Dog cat".
fn label_words(label: &str) -> String {
    let plain = label.trim().replace(['_', '-'], " ");
    let mut chars = plain.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// What was seen: the kind's word, then " \u{b7} " and the label when the recorder named it and the label is not the class again
/// ("Animal \u{b7} Domestic Cat"; "Person" for a person labelled `person`).
pub fn what(kind: &str, label: &str) -> String {
    let word = kind_word(kind);
    let label = label.trim();
    let lower = label.to_lowercase();
    if label.is_empty() || lower == kind || lower == word.to_lowercase() {
        return word.to_string();
    }
    format!("{word} \u{b7} {}", label_words(label))
}

/// The title of an event's card: its kind and label; for the recent list of every camera the camera's name leads ("Front Door \u{b7} Person").
pub fn event_title(camera: Option<&str>, kind: &str, label: &str) -> String {
    let w = what(kind, label);
    match camera {
        Some(name) if !name.is_empty() => format!("{name} \u{b7} {w}"),
        _ => w,
    }
}

/// The words of a refusal of a `POST` (the bridge's `error.code`, passed through by the helper): a toast of one line. A code this table
/// lacks shows the helper's own words; `None` says nothing at all (`event_not_found` on a picture: the card has no picture and says
/// nothing about it).
pub fn refusal(code: &str, helper_words: &str) -> Option<String> {
    Some(
        match code {
            "offline" => "The camera is offline",
            "not_supported" => "No stream fits this TV",
            "no_recording" => "No recording at that time",
            "camera_not_found" => "Scrypted no longer shares this camera",
            "busy" => "Scrypted is busy",
            "timeout" => "Scrypted didn\u{2019}t answer",
            "upstream_error" => "Scrypted couldn\u{2019}t start it",
            "event_not_found" => return None,
            _ => {
                let w = helper_words.trim();
                return if w.is_empty() { None } else { Some(w.to_string()) };
            }
        }
        .to_string(),
    )
}

/// The state of a camera's live picture on the name pill.
pub const LIVE: &str = "Live";
pub const STARTING: &str = "Starting";
pub const RECORDED: &str = "Recorded";
pub const CANT_PLAY: &str = "Can\u{2019}t play";
pub const OPENS_SOON: &str = "Opens in a few seconds";
pub const FAILED_TITLE: &str = "Can\u{2019}t play this camera";
pub const TRY_AGAIN: &str = "Try again";
/// A camera on a battery: its latest picture until the owner connects, and the wait while it wakes (up to 45 s).
pub const STILL: &str = "Picture";
pub const WAKING: &str = "Connecting\u{2026}";
pub const WAKING_LINE: &str = "Waking the camera, up to 45 seconds";
pub const BATTERY_TITLE: &str = "This camera runs on a battery";
pub const BATTERY_DETAIL: &str = "Showing its latest picture. Going live wakes the camera and uses its battery.";
pub const CONNECT: &str = "Connect";
/// The note over a camera that Scrypted says is offline, and what the screen does about it.
pub const OFFLINE_TITLE: &str = "This camera is offline";
pub const OFFLINE_RETRYING: &str = "Looking again every few seconds.";
pub const OFFLINE_GIVEN_UP: &str = "Try again when it is back on.";
/// After the reason, while the camera is asked for again by itself.
pub const RETRYING: &str = "Trying again soon.";
pub const NO_PICTURE: &str = "No picture";
pub const CONNECTING: &str = "Connecting";
pub const OFFLINE: &str = "Offline";
pub const NO_RECORDING: &str = "No recording at that time";
pub const END_OF_RECORDING: &str = "End of recording";

/// "Codec" as a person reads it: "H.265", "H.264", "AV1"; anything else as the stream says it, upper-cased.
pub fn codec_word(codec: &str) -> String {
    match codec.to_lowercase().replace(['.', '-', ' '], "").as_str() {
        "h265" | "hevc" => "H.265".to_string(),
        "h264" | "avc" => "H.264".to_string(),
        "av1" => "AV1".to_string(),
        "mjpeg" | "mjpg" => "MJPEG".to_string(),
        other => other.to_uppercase(),
    }
}

/// The info pill: `3840\u{d7}2160 \u{b7} H.265 \u{b7} 15 fps`; a member the stream does not give is left out, and with none the pill is not drawn.
pub fn stream_words(width: u32, height: u32, codec: &str, fps: u32, rate: f32) -> String {
    let mut parts: Vec<String> = Vec::new();
    if width > 0 && height > 0 {
        parts.push(format!("{width}\u{d7}{height}"));
    }
    if !codec.is_empty() {
        parts.push(codec_word(codec));
    }
    if fps > 0 {
        parts.push(format!("{fps} fps"));
    }
    if !parts.is_empty() && (rate - 1.0).abs() > 0.01 {
        parts.push(rate_word(rate));
    }
    parts.join(" \u{b7} ")
}

/// "15 min", "1 h", "6 h", "24 h".
pub fn span_word(minutes: u32) -> String {
    if minutes >= 60 { format!("{} h", minutes / 60) } else { format!("{minutes} min") }
}

/// "1\u{d7}", "2\u{d7}", "4\u{d7}".
pub fn rate_word(rate: f32) -> String {
    if (rate - rate.round()).abs() < 0.01 { format!("{}\u{d7}", rate.round() as i32) } else { format!("{rate}\u{d7}") }
}

/// The value of a camera's "Notify for" row: "Person, Doorbell" or "Off".
pub fn notify_value(kinds: &[String]) -> String {
    if kinds.is_empty() { "Off".to_string() } else { kinds.iter().map(|k| kind_name(k)).collect::<Vec<_>>().join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const HOUR: i64 = 3_600_000;

    /// 2026-10-29 14:37:00 in UTC.
    fn t0() -> i64 {
        let days = 20_755; // 2026-10-29
        (days * 86_400 + 14 * 3600 + 37 * 60) * 1000
    }

    fn clock() -> Clock {
        Clock { twelve_hour: true, zone: Zone::UTC }
    }

    #[test]
    fn the_calendar_reads_the_epoch_as_a_person_does() {
        let c = Zone::UTC.civil(t0());
        assert_eq!((c.year, c.month, c.day, c.hour, c.minute), (2026, 10, 29, 14, 37));
        let c = Zone::fixed(-4 * 3600).civil(t0());
        assert_eq!((c.hour, c.minute), (10, 37));
        assert_eq!(Zone::UTC.civil(0).year, 1970);
        // a moment before the epoch is still on a calendar
        let c = Zone::UTC.civil(-1000);
        assert_eq!((c.year, c.month, c.day, c.hour, c.minute, c.second), (1969, 12, 31, 23, 59, 59));
    }

    #[test]
    fn times_are_twelve_hour_by_default_and_twenty_four_when_the_owner_says() {
        assert_eq!(clock().clock(t0() - 6 * MIN), "2:31 PM");
        assert_eq!(clock().clock(t0() - 14 * HOUR - 37 * MIN), "12:00 AM");
        assert_eq!(clock().clock(t0() - 2 * HOUR - 25 * MIN), "12:12 PM");
        assert_eq!(clock().clock_seconds(t0() - 24 * MIN + 20_000), "2:13:20 PM");
        let c24 = Clock { twelve_hour: false, zone: Zone::UTC };
        assert_eq!(c24.clock(t0() - 6 * MIN), "14:31");
        assert_eq!(c24.clock(t0() - 14 * HOUR - 37 * MIN), "00:00");
    }

    #[test]
    fn today_has_no_day_word_then_yesterday_then_the_date() {
        let c = clock();
        assert_eq!(c.day_word(t0() - 6 * MIN, t0()), None);
        assert_eq!(c.stamp(t0() - 6 * MIN, t0()), "2:31 PM");
        assert_eq!(c.stamp(t0() - 16 * HOUR - 27 * MIN, t0()), "Yesterday 10:10 PM");
        assert_eq!(c.stamp(t0() - 2 * 24 * HOUR, t0()), "Oct 27 2:37 PM");
        assert_eq!(c.stamp(t0() - 40 * 24 * HOUR, t0()), "Sep 19 2:37 PM");
    }

    #[test]
    fn the_card_line_gives_the_time_and_how_long_ago_for_today_and_the_date_for_the_rest() {
        let c = clock();
        assert_eq!(c.card_line(t0() - 6 * MIN, t0()), "2:31 PM \u{b7} 6 min ago");
        assert_eq!(c.card_line(t0() - 39 * MIN, t0()), "1:58 PM \u{b7} 39 min ago");
        assert_eq!(c.card_line(t0() - 2 * HOUR - 25 * MIN, t0()), "12:12 PM \u{b7} 2 h ago");
        assert_eq!(c.card_line(t0() - 16 * HOUR - 27 * MIN, t0()), "Yesterday 10:10 PM");
    }

    #[test]
    fn ago_follows_the_ladder_and_a_clock_a_little_ahead_is_never_negative() {
        assert_eq!(ago(-5000), "Just now");
        assert_eq!(ago(0), "Just now");
        assert_eq!(ago(59_999), "Just now");
        assert_eq!(ago(60_000), "1 min ago");
        assert_eq!(ago(125_000), "2 min ago");
        assert_eq!(ago(89 * MIN), "89 min ago");
        assert_eq!(ago(90 * MIN), "1 h ago");
        assert_eq!(ago(5 * HOUR + 40 * MIN), "5 h ago");
        assert_eq!(ago(23 * HOUR + 59 * MIN), "23 h ago");
        assert_eq!(ago(24 * HOUR), "Yesterday");
        assert_eq!(ago(47 * HOUR), "Yesterday");
        assert_eq!(ago(48 * HOUR), "2 days ago");
        assert_eq!(ago(9 * 24 * HOUR), "9 days ago");
    }

    #[test]
    fn the_age_of_a_picture_counts_seconds_then_minutes() {
        assert_eq!(age(2_000), "Just now");
        assert_eq!(age(13_000), "13 s ago");
        assert_eq!(age(4 * MIN), "4 min ago");
        assert_eq!(age(2 * HOUR), "2 h ago");
    }

    #[test]
    fn behind_live_reads_now_seconds_minutes_and_hours() {
        assert_eq!(behind(0), "Now");
        assert_eq!(behind(4_999), "Now");
        assert_eq!(behind(42_000), "42 s ago");
        assert_eq!(behind(24 * MIN), "24 min ago");
        assert_eq!(behind(3 * HOUR + 25 * MIN), "3 h 25 min ago");
        assert_eq!(behind(2 * HOUR), "2 h ago");
        assert_eq!(behind(-9000), "Now");
    }

    #[test]
    fn a_clip_has_a_length_and_the_count_a_plural() {
        assert_eq!(length(30_000), "0:30");
        assert_eq!(length(42_400), "0:42");
        assert_eq!(length(65_000), "1:05");
        assert_eq!(count(8), "8 cameras");
        assert_eq!(count(1), "1 camera");
        assert_eq!(of(2, 8), "2 of 8");
        assert_eq!(range_below(9, 12), "9 to 12");
        assert_eq!(range_below(9, 9), "9");
    }

    #[test]
    fn what_was_seen_is_the_kind_then_the_label_unless_the_label_is_the_class_again() {
        assert_eq!(what("person", ""), "Person");
        assert_eq!(what("person", "person"), "Person");
        assert_eq!(what("person", "Person"), "Person");
        assert_eq!(what("animal", "Animal"), "Animal");
        assert_eq!(what("animal", "Domestic Cat"), "Animal \u{b7} Domestic Cat");
        assert_eq!(what("doorbell", "doorbell"), "Doorbell rang");
        assert_eq!(what("vehicle", "Vehicle"), "Vehicle");
        assert_eq!(what("mystery", ""), "Event");
        assert_eq!(what("mystery", "Something"), "Event \u{b7} Something");
        assert_eq!(what("person", "Nitin"), "Person \u{b7} Nitin");
    }

    #[test]
    fn a_class_name_used_as_a_label_reads_as_words() {
        assert_eq!(what("animal", "dog_cat"), "Animal \u{b7} Dog cat");
        assert_eq!(what("animal", "bird"), "Animal \u{b7} Bird");
    }

    #[test]
    fn the_recent_list_names_the_camera_first() {
        assert_eq!(event_title(Some("Front Door"), "person", "person"), "Front Door \u{b7} Person");
        assert_eq!(event_title(Some("Backyard"), "animal", ""), "Backyard \u{b7} Animal");
        assert_eq!(event_title(None, "animal", "Domestic Cat"), "Animal \u{b7} Domestic Cat");
    }

    #[test]
    fn kinds_have_a_word_a_name_and_a_glyph() {
        assert_eq!(kind_word("doorbell"), "Doorbell rang");
        assert_eq!(kind_name("doorbell"), "Doorbell");
        assert_eq!(kind_icon("person"), "user");
        assert_eq!(kind_icon("vehicle"), "car");
        assert_eq!(kind_icon("animal"), "paw-print");
        assert_eq!(kind_icon("package"), "package");
        assert_eq!(kind_icon("doorbell"), "bell-ring");
        assert_eq!(kind_icon("motion"), "activity");
        assert_eq!(kind_icon("other"), "camera");
        assert_eq!(kind_word("other"), "Event");
    }

    #[test]
    fn the_refusals_have_the_specs_words_and_an_unknown_code_shows_the_helpers() {
        assert_eq!(refusal("offline", "x").as_deref(), Some("The camera is offline"));
        assert_eq!(refusal("not_supported", "x").as_deref(), Some("No stream fits this TV"));
        assert_eq!(refusal("no_recording", "x").as_deref(), Some("No recording at that time"));
        assert_eq!(refusal("camera_not_found", "x").as_deref(), Some("Scrypted no longer shares this camera"));
        assert_eq!(refusal("busy", "x").as_deref(), Some("Scrypted is busy"));
        assert_eq!(refusal("timeout", "x").as_deref(), Some("Scrypted didn\u{2019}t answer"));
        assert_eq!(refusal("upstream_error", "x").as_deref(), Some("Scrypted couldn\u{2019}t start it"));
        assert_eq!(refusal("event_not_found", "gone"), None, "no words: the card has no picture and says nothing about it");
        assert_eq!(refusal("weird", "The bridge said so").as_deref(), Some("The bridge said so"));
        assert_eq!(refusal("weird", " "), None);
    }

    #[test]
    fn the_links_words_carry_no_address_and_a_ready_link_has_none() {
        assert_eq!(link_words("unlinked"), "Scrypted isn\u{2019}t linked");
        assert_eq!(link_words("connecting"), "Connecting to Scrypted");
        assert_eq!(link_words("refused"), "Scrypted refused this TV");
        assert_eq!(link_words("untrusted"), "Can\u{2019}t verify Scrypted");
        assert_eq!(link_words("offline"), "Can\u{2019}t reach Scrypted");
        assert_eq!(link_words("ready"), "");
    }

    #[test]
    fn the_info_pill_leaves_out_what_the_stream_does_not_give() {
        assert_eq!(stream_words(3840, 2160, "h265", 15, 1.0), "3840\u{d7}2160 \u{b7} H.265 \u{b7} 15 fps");
        assert_eq!(stream_words(3840, 2160, "h265", 15, 2.0), "3840\u{d7}2160 \u{b7} H.265 \u{b7} 15 fps \u{b7} 2\u{d7}");
        assert_eq!(stream_words(1920, 1080, "h264", 0, 1.0), "1920\u{d7}1080 \u{b7} H.264");
        assert_eq!(stream_words(0, 0, "", 0, 1.0), "");
        assert_eq!(stream_words(0, 0, "", 0, 4.0), "", "a rate alone is not a pill");
    }

    #[test]
    fn spans_rates_and_notify_values_read_as_the_mocks_do() {
        assert_eq!(span_word(15), "15 min");
        assert_eq!(span_word(60), "1 h");
        assert_eq!(span_word(1440), "24 h");
        assert_eq!(rate_word(1.0), "1\u{d7}");
        assert_eq!(rate_word(4.0), "4\u{d7}");
        assert_eq!(notify_value(&["person".into(), "doorbell".into()]), "Person, Doorbell");
        assert_eq!(notify_value(&[]), "Off");
    }
}
