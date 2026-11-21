//! The words of an alert before it is drawn: LG's services post markup (`<br>`), several lines of padding, and often no title at all,
//! when `notificationmgr` then hands the caller's service name over as `sourceId` (`com.webos.service.nop`,
//! `com.webos.service.tvReservationAgent`). The sheet must never show such an id as its title, so a missing or id-like title becomes a
//! plain one, and LG's power-off warnings get a title and a short sentence of their own.
//!
//! The warnings, as the TV's services post them (read from `/etc/palm/nop-conf.json` and the strings of `tv-reservation-agent`):
//! - `nopd` (Auto Power Off, and the no-operation off after hours): no title, `message` "Your TV will be turned off in 1 minute due
//!   to the TV's Auto Power Off settings. To keep the TV on, press any buttons except the Power button.", one OK button;
//! - `tv-reservation-agent` (sleep timer, power-off timer): no title, `message` "Your TV will be turned off in 1 minute due to the TV's
//!   timer settings. Click 'Cancel' to keep the TV on or 'OK' to let it turn off." (the power-off timer's copy breaks the line after
//!   "settings."), buttons OK and Cancel, Cancel carrying the call that aborts the off.

/// What a title falls back to when the service sent none and the message is not a known one.
const GENERIC_TITLE: &str = "TV message";

/// LG's power-off warnings, by the setting that raised them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Power {
    AutoPowerOff,
    SleepTimer,
    OffTimer,
    NoSignal,
    AutomaticStandby,
    /// Says the TV turns off but names no setting.
    Other,
}

impl Power {
    fn title(self) -> &'static str {
        match self {
            Power::AutoPowerOff => "Auto power off",
            Power::SleepTimer => "Sleep timer",
            Power::OffTimer => "Power-off timer",
            Power::NoSignal => "No signal",
            Power::AutomaticStandby => "Automatic standby",
            Power::Other => "Power off",
        }
    }
}

/// Markup and padding out of a text: `<br>` becomes a line break, other tags go, the common entities are read, runs of blanks shrink to
/// one space and runs of line breaks to one blank line.
pub fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let (before, tag_and_after) = rest.split_at(open);
        out.push_str(before);
        // `<` that does not open a tag (a lone comparison) is text
        match tag_and_after.find('>') {
            Some(close) if tag_and_after[1..close].chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!') => {
                let name: String = tag_and_after[1..close].trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                if name.eq_ignore_ascii_case("br") || name.eq_ignore_ascii_case("p") || name.eq_ignore_ascii_case("div") {
                    out.push('\n');
                }
                rest = &tag_and_after[close + 1..];
            }
            _ => {
                out.push('<');
                rest = &tag_and_after[1..];
            }
        }
    }
    out.push_str(rest);
    let out = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    let mut lines: Vec<String> = Vec::new();
    for line in out.replace("\r\n", "\n").split(['\n', '\r']) {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        // at most one blank line in a row, none first
        if line.is_empty() && lines.last().is_none_or(String::is_empty) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// A "title" that is a name for a program, not for a person: a service id, a resource key, a placeholder, or the `sourceId` itself.
fn id_like(title: &str, source_id: &str) -> bool {
    if title.is_empty() || title.contains(char::is_whitespace) {
        return title.is_empty();
    }
    title.eq_ignore_ascii_case(source_id)
        || title.matches('.').count() >= 2
        || title.contains('_')
        || title.starts_with(['$', '@', '%', '{', '#'])
}

/// "1 minute", "5 minutes", "30 seconds" after the first "<verb> off in " (or "standby in ") of the message.
fn time_left(lower: &str) -> Option<String> {
    let at = ["turned off in ", "turn off in ", "turns off in ", "powered off in ", "go to standby in ", "goes to standby in "]
        .iter()
        .filter_map(|p| lower.find(p).map(|i| i + p.len()))
        .min()?;
    let mut words = lower[at..].split(|c: char| !c.is_alphanumeric());
    let n: u32 = match words.next()? {
        "one" | "a" | "an" => 1,
        w => w.parse().ok().filter(|n| *n > 0)?,
    };
    let unit = words.next()?;
    let unit = if unit.starts_with("minute") {
        "minute"
    } else if unit.starts_with("second") {
        "second"
    } else if unit.starts_with("hour") {
        "hour"
    } else {
        return None;
    };
    Some(if n == 1 { format!("1 {unit}") } else { format!("{n} {unit}s") })
}

fn power(lower: &str, source_id: &str) -> Option<Power> {
    time_left(lower)?;
    let has = |s: &str| lower.contains(s);
    let src = source_id.to_ascii_lowercase();
    Some(if has("auto power off") || has("auto power-off") {
        Power::AutoPowerOff
    } else if has("no signal") || has("no-signal") {
        Power::NoSignal
    } else if has("automatic standby") {
        Power::AutomaticStandby
    } else if has("sleep timer") || has("sleep time") {
        Power::SleepTimer
    } else if has("timer settings") || has("timer") || src.contains("reservation") {
        // the sleep timer's copy quotes its buttons ('Cancel'), the power-off timer's does not
        if has("'cancel'") { Power::SleepTimer } else { Power::OffTimer }
    } else {
        Power::Other
    })
}

/// The title and message an alert is drawn with. `labels` are the button labels, as LG sent them.
pub fn present(source_id: &str, title: &str, message: &str, labels: &[&str]) -> (String, String) {
    let title = clean(title).replace('\n', " ");
    let message = clean(message);
    let lower = message.to_lowercase();
    let known = power(&lower, source_id);
    let sentence = known.and_then(|_| time_left(&lower)).map(|t| format!("The TV turns off in {t}."));
    let cancel = labels.iter().any(|l| l.trim().eq_ignore_ascii_case("cancel"));
    // the two families seen on the TV get a sentence of ours; any other power-off message keeps LG's words
    let message = match (known, sentence) {
        (Some(Power::AutoPowerOff), Some(s)) if lower.contains("press any button") => format!("{s} To keep it on, press any button except Power."),
        (Some(Power::AutoPowerOff), Some(s)) => s,
        (Some(Power::SleepTimer | Power::OffTimer), Some(s)) if cancel => format!("{s} Select Cancel to keep it on."),
        (Some(Power::SleepTimer | Power::OffTimer), Some(s)) => s,
        _ => message,
    };
    let title = if id_like(&title, source_id) {
        known.map_or(GENERIC_TITLE, Power::title).to_string()
    } else {
        title
    };
    (title, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOPD: &str = "Your TV will be turned off in 1 minute due to the TV's Auto Power Off settings. To keep the TV on, press any buttons except the Power button.";
    const SLEEP: &str = "Your TV will be turned off in 1 minute due to the TV's timer settings. Click 'Cancel' to keep the TV on or 'OK' to let it turn off.";
    const OFF_TIMER: &str = "Your TV will be turned off in 1 minute due to the TV's timer settings.<br>Click Cancel to keep the TV on or OK to let it turn off.";

    #[test]
    fn markup_entities_and_padding_are_cleaned() {
        assert_eq!(clean("  a<br>b<BR/>\n\n\n c &amp; d&nbsp;e <b>bold</b> 1 < 2  "), "a\nb\n\nc & d e bold 1 < 2");
        assert_eq!(clean("<br><br>"), "");
        assert_eq!(clean("one\r\ntwo"), "one\ntwo");
    }

    #[test]
    fn nopd_auto_power_off_has_no_title_and_gets_ours() {
        let (t, m) = present("com.webos.service.nop", "", NOPD, &["OK"]);
        assert_eq!(t, "Auto power off");
        assert_eq!(m, "The TV turns off in 1 minute. To keep it on, press any button except Power.");
    }

    #[test]
    fn the_sleep_timer_and_the_power_off_timer_are_told_apart_and_name_cancel() {
        let (t, m) = present("com.webos.service.tvReservationAgent", "", SLEEP, &["OK", "Cancel"]);
        assert_eq!((t.as_str(), m.as_str()), ("Sleep timer", "The TV turns off in 1 minute. Select Cancel to keep it on."));
        let (t, m) = present("com.webos.service.tvReservationAgent", "", OFF_TIMER, &["OK", "Cancel"]);
        assert_eq!((t.as_str(), m.as_str()), ("Power-off timer", "The TV turns off in 1 minute. Select Cancel to keep it on."));
        let (_, m) = present("com.webos.service.tvReservationAgent", "", SLEEP, &["OK"]);
        assert_eq!(m, "The TV turns off in 1 minute.", "Cancel is named only when there is a Cancel button");
    }

    #[test]
    fn an_id_a_resource_key_or_the_source_is_never_the_title() {
        for bad in ["com.webos.service.nop", "SLEEP_TIMER_TITLE", "$title", "tvReservationAgent"] {
            let (t, _) = present("tvReservationAgent", bad, "Something happened.", &[]);
            assert_eq!(t, GENERIC_TITLE, "{bad}");
        }
        let (t, m) = present("com.webos.app.home", "", "Something happened.", &[]);
        assert_eq!((t.as_str(), m.as_str()), (GENERIC_TITLE, "Something happened."));
    }

    #[test]
    fn a_real_title_and_other_messages_are_kept() {
        let (t, m) = present("com.webos.app.home", "Uninstall <b>Crunchyroll</b>?", "The app<br>is removed.", &["Keep it", "Uninstall"]);
        assert_eq!((t.as_str(), m.as_str()), ("Uninstall Crunchyroll?", "The app\nis removed."));
        let (t, m) = present("x", "Sleep timer", SLEEP, &["OK", "Cancel"]);
        assert_eq!(t, "Sleep timer");
        assert_eq!(m, "The TV turns off in 1 minute. Select Cancel to keep it on.");
    }

    #[test]
    fn other_power_warnings_get_a_title_and_keep_lg_words() {
        let msg = "No signal. The TV will be turned off in 5 minutes if no input is detected.";
        let (t, m) = present("com.webos.app.inputcommon", "", msg, &["OK"]);
        assert_eq!((t.as_str(), m.as_str()), ("No signal", msg));
        let msg = "Automatic Standby: the TV will turn off in 30 seconds.";
        assert_eq!(present("s", "", msg, &[]).0, "Automatic standby");
        assert_eq!(present("s", "", "The TV will be turned off in 2 minutes.", &[]).0, "Power off");
    }
}
