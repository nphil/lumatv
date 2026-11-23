//! What the route changed, written before it changes anything so that a helper that dies mid-session can put LG's settings back.

use crate::sndout::Words;

/// One session's changes. A journal from an earlier boot is stale: the kernel state it describes is gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Journal {
    pub boot: String,
    pub pid: u32,
    /// LG's eARC words before the session touched them.
    pub before: Words,
    /// The words the session wrote.
    pub applied: Words,
    /// The input ADEC1 was fed from when the session connected it, `None` while the session has not.
    pub adec: Option<String>,
}

const HEAD: &str = "audiopass-journal 1";

impl Journal {
    pub fn render(&self) -> String {
        let mut text = format!(
            "{HEAD}\nboot={}\npid={}\nbefore={}\napplied={}\n",
            self.boot, self.pid, self.before, self.applied
        );
        if let Some(adec) = &self.adec {
            text.push_str(&format!("adec={adec}\n"));
        }
        text
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != HEAD {
            return None;
        }
        let (mut boot, mut pid, mut before, mut applied, mut adec) = (None, None, None, None, None);
        for line in lines {
            match line.split_once('=')? {
                ("boot", v) => boot = Some(v.to_owned()),
                ("pid", v) => pid = v.parse().ok(),
                ("before", v) => before = v.parse().ok(),
                ("applied", v) => applied = v.parse().ok(),
                ("adec", v) => adec = Some(v.to_owned()),
                _ => {}
            }
        }
        Some(Self {
            boot: boot?,
            pid: pid?,
            before: before?,
            applied: applied?,
            adec,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(adec: Option<&str>) -> Journal {
        Journal {
            boot: "1f0e-boot".into(),
            pid: 4242,
            before: Words {
                set_type: 2,
                codec: 1,
                channels: 0,
                rate: 0,
                mix: 1,
            },
            applied: Words {
                set_type: 2,
                codec: 1,
                channels: 0,
                rate: 0,
                mix: 0,
            },
            adec: adec.map(str::to_owned),
        }
    }

    #[test]
    fn round_trips_with_and_without_adec() {
        for adec in [None, Some("HDMI_PORT2")] {
            let journal = sample(adec);
            assert_eq!(Journal::parse(&journal.render()), Some(journal));
        }
    }

    #[test]
    fn damaged_journals_are_rejected() {
        let text = sample(None).render();
        assert_eq!(Journal::parse(""), None);
        assert_eq!(Journal::parse(&text.replace("pid=4242\n", "")), None);
        assert_eq!(
            Journal::parse(&text.replace("before=2,1,0,0,1", "before=2,1,0")),
            None
        );
        assert_eq!(
            Journal::parse(&text.replace(HEAD, "audiopass-journal 2")),
            None
        );
    }
}
