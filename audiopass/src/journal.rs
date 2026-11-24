//! What the route changed, written before it changes anything so that a helper that dies mid-session can put LG's settings back.

use crate::mixer::Taken;
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
    /// The ADEC the player's renderer was granted: the only decoder input the session connects.
    pub adec: u8,
    /// That ADEC's `adec.p<N>` line before the session touched it. A restore puts its mute back and compares what it finds with it.
    pub kernel: String,
    /// Whether the session has asked audiooutputd to connect the ADEC. Written before the call, so that a restore after a crash
    /// knows the connection can be the session's.
    pub connected: bool,
    /// LG's mixer input for the player's own stream, when the session disconnected it. Written before the call.
    pub mixer: Option<Taken>,
}

const HEAD: &str = "audiopass-journal 2";

impl Journal {
    pub fn render(&self) -> String {
        let mut text = format!(
            "{HEAD}\nboot={}\npid={}\nbefore={}\napplied={}\nadec={}\nkernel={}\nconnected={}\n",
            self.boot,
            self.pid,
            self.before,
            self.applied,
            self.adec,
            self.kernel,
            u8::from(self.connected)
        );
        if let Some(taken) = &self.mixer {
            text.push_str(&format!("mixer={} {}\n", taken.port, taken.pipeline));
        }
        text
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != HEAD {
            return None;
        }
        let (mut boot, mut pid, mut before, mut applied) = (None, None, None, None);
        let (mut adec, mut kernel, mut connected, mut mixer) = (None, None, None, None);
        for line in lines {
            match line.split_once('=')? {
                ("boot", v) => boot = Some(v.to_owned()),
                ("pid", v) => pid = v.parse().ok(),
                ("before", v) => before = v.parse().ok(),
                ("applied", v) => applied = v.parse().ok(),
                ("adec", v) => adec = v.parse().ok(),
                ("kernel", v) => kernel = Some(v.to_owned()),
                ("connected", v) => connected = Some(v == "1"),
                ("mixer", v) => mixer = Some(Taken::parse(v)?),
                _ => {}
            }
        }
        Some(Self {
            boot: boot?,
            pid: pid?,
            before: before?,
            applied: applied?,
            adec: adec?,
            kernel: kernel?,
            connected: connected?,
            mixer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(connected: bool) -> Journal {
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
            adec: 3,
            kernel: "Open=1 Connect=HDMI_PORT2 Start=0 Gain=0x800000 Mute=1".into(),
            connected,
            mixer: None,
        }
    }

    #[test]
    fn round_trips_before_and_after_the_connect() {
        for connected in [false, true] {
            let journal = sample(connected);
            assert_eq!(Journal::parse(&journal.render()), Some(journal));
        }
        let mut journal = sample(false);
        journal.mixer = Some(Taken {
            port: 4,
            pipeline: "_f4rXIG4rerDUwJ".into(),
        });
        assert!(journal.render().contains("\nmixer=4 _f4rXIG4rerDUwJ\n"));
        assert_eq!(Journal::parse(&journal.render()), Some(journal));
    }

    #[test]
    fn a_journal_is_a_few_hundred_bytes() {
        let mut journal = sample(true);
        journal.mixer = Some(Taken {
            port: 4,
            pipeline: "_".repeat(32),
        });
        journal.boot = "x".repeat(36);
        journal.pid = u32::MAX;
        assert!(journal.render().len() < 512);
    }

    #[test]
    fn damaged_journals_are_rejected() {
        let text = sample(false).render();
        assert_eq!(Journal::parse(""), None);
        assert_eq!(Journal::parse(&text.replace("pid=4242\n", "")), None);
        assert_eq!(
            Journal::parse(&text.replace("before=2,1,0,0,1", "before=2,1,0")),
            None
        );
        assert_eq!(Journal::parse(&text.replace("adec=3\n", "")), None);
        assert_eq!(
            Journal::parse(&text.replace(HEAD, "audiopass-journal 1")),
            None
        );
        assert_eq!(Journal::parse(&format!("{text}mixer=4\n")), None);
        assert_eq!(Journal::parse(&format!("{text}mixer=9 _f4r\n")), None);
    }
}
