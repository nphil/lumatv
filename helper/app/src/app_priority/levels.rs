//! LG's memory levels as its callers know them (`getCurrentMemState`, `thresholdChanged`), judged on the kernel's `usable_pages`
//! with memchute's hysteresis. The tables are this set's effective thresholds (hd2160, normal mode), in 4 KB pages, and match
//! `helper/mm/lumatv-mm.c`, which answers from them while the helper is away.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    #[default]
    Normal,
    Medium,
    Low,
    Critical,
    Reboot,
}

const LEVELS: [Level; 5] = [Level::Normal, Level::Medium, Level::Low, Level::Critical, Level::Reboot];
/// A level is entered below its `ENTER` and left above its `LEAVE`; reboot is never entered from here.
const ENTER: [u64; 5] = [0, 64_000, 51_200, 11_706, 0];
const LEAVE: [u64; 5] = [0, 74_453, 56_320, 17_033, 2_978];

pub const PAGES_PER_MB: u64 = 256;

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Normal => "normal",
            Level::Medium => "medium",
            Level::Low => "low",
            Level::Critical => "critical",
            Level::Reboot => "reboot",
        }
    }

    /// The level after `usable` pages, from this one.
    pub fn step(self, usable: u64) -> Level {
        let mut i = self as usize;
        while i < 4 && usable < ENTER[i + 1] {
            i += 1;
        }
        while i > 0 && usable > LEAVE[i] {
            i -= 1;
        }
        LEVELS[i]
    }
}

/// The table as `getPolicy` shows it, in MB.
pub fn table() -> serde_json::Value {
    let mb = |pages: u64| pages / PAGES_PER_MB;
    let rows: Vec<serde_json::Value> = LEVELS[1..]
        .iter()
        .map(|l| serde_json::json!({"level": l.name(), "enterMB": mb(ENTER[*l as usize]), "leaveMB": mb(LEAVE[*l as usize])}))
        .collect();
    serde_json::Value::Array(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_follow_the_enter_and_leave_table_with_hysteresis() {
        let mb = |m: u64| m * PAGES_PER_MB;
        assert_eq!(Level::Normal.step(mb(260)), Level::Normal);
        assert_eq!(Level::Normal.step(mb(240)), Level::Medium);
        // Between medium's enter (250) and leave (290): stays.
        assert_eq!(Level::Medium.step(mb(280)), Level::Medium);
        assert_eq!(Level::Medium.step(mb(291)), Level::Normal);
        // A fall straight through several levels lands on the deepest.
        assert_eq!(Level::Normal.step(mb(40)), Level::Critical);
        assert_eq!(Level::Critical.step(mb(60)), Level::Critical);
        assert_eq!(Level::Critical.step(mb(210)), Level::Low);
        assert_eq!(Level::Low.step(mb(400)), Level::Normal);
        assert_eq!(Level::Normal.step(0), Level::Critical);
    }
}
