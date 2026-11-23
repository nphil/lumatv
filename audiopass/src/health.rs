//! The audio-stack health gate that runs before every step that touches LG's route. The C4's sound card control lock can wedge for
//! good (a kernel task stuck in uninterruptible sleep inside `snd_ctl`), and nothing that touches it afterwards can be cancelled.
//! Port of the HEALTH check of the TrueHD study's kit (WEDGE.md).

use crate::host::Host;

const STATUS: &str = "luna://com.webos.service.audiooutput/audio/getStatus";
/// LG's HDMI input service. Not yet read back from a C4: a reply of any kind, an error reply included, counts as answering.
const HDMI_STATUS: &str = "luna://com.webos.service.extinputs.hdmi/getStatus";
/// A stuck task whose kernel stack or wait channel names one of these holds the control lock.
const LOCK: [&str; 3] = ["snd_ctl", "sndout", "rwsem"];
/// Audio daemons that sit in `D` for a moment while LG reconfigures its output.
const DAEMONS: [&str; 4] = [
    "audiooutputd",
    "extinput-integr",
    "extinput-integration",
    "amixer",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Health {
    Ok,
    /// Something is busy right now; looking again in a moment is reasonable.
    Busy(String),
    /// The control lock is held by a task that will not come back; nothing may touch the route until the TV restarts.
    Wedged(String),
}

/// Whether a luna reply says `"returnValue":true`.
pub fn returned_true(reply: &str) -> bool {
    reply.replace(' ', "").contains("\"returnValue\":true")
}

/// What audiooutputd has connected, as source names such as `ADEC1`; empty when nothing is. `None` when the service does not answer
/// or the reply is not a status.
pub fn connections(host: &dyn Host) -> Option<Vec<String>> {
    let reply = host.luna(STATUS, "{}")?;
    if !returned_true(&reply) {
        return None;
    }
    let reply = reply.replace(' ', "");
    let list = reply.split("\"audio\":[").nth(1)?;
    let list = &list[..list.find(']')?];
    let sources: Vec<String> = list
        .split("\"source\":\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_owned)
        .collect();
    // A connection of a shape this does not know still counts as one.
    if sources.is_empty() && !list.is_empty() {
        return Some(vec!["an unknown connection".to_owned()]);
    }
    Some(sources)
}

/// Whether LG's HDMI input service answers a status call. It is the service that stopped answering after a badly ended session, so
/// a reply of any kind is the sign of life; only silence is a fault.
pub fn hdmi_answers(host: &dyn Host) -> bool {
    host.luna(HDMI_STATUS, "{}").is_some()
}

pub fn check(host: &dyn Host) -> Health {
    let mut busy = None;
    for task in host.stuck_tasks() {
        if LOCK
            .iter()
            .any(|word| task.stack.contains(word) || task.wchan.contains(word))
        {
            return Health::Wedged(format!(
                "{} is stuck in the sound card's control lock",
                task.comm
            ));
        }
        if busy.is_none() && DAEMONS.iter().any(|daemon| task.comm.starts_with(daemon)) {
            busy = Some(format!("{} is waiting on the kernel", task.comm));
        }
    }
    if let Some(why) = busy {
        return Health::Busy(why);
    }
    match host.luna(STATUS, "{}") {
        Some(reply) if returned_true(&reply) => Health::Ok,
        _ => Health::Busy("the audio service does not answer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Stuck;
    use crate::testing::FakeHost;

    fn stuck(comm: &str, stack: &str) -> Stuck {
        Stuck {
            comm: comm.into(),
            wchan: String::new(),
            stack: stack.into(),
        }
    }

    #[test]
    fn healthy_tv_passes() {
        assert_eq!(check(&FakeHost::c4()), Health::Ok);
    }

    #[test]
    fn a_task_in_the_control_lock_is_a_wedge_whatever_its_name() {
        let host = FakeHost::c4();
        host.with(|sim| {
            sim.stuck.push(stuck(
                "kworker/1:2",
                "[<0>] rwsem_down_read_slowpath+0x1d4/0x420\n",
            ))
        });
        assert!(matches!(check(&host), Health::Wedged(why) if why.contains("kworker/1:2")));
    }

    #[test]
    fn audio_daemons_waiting_are_transient() {
        let host = FakeHost::c4();
        host.with(|sim| {
            sim.stuck
                .push(stuck("audiooutputd", "[<0>] msleep+0x1c/0x40\n"))
        });
        assert!(matches!(check(&host), Health::Busy(why) if why.contains("audiooutputd")));
    }

    #[test]
    fn unrelated_stuck_tasks_do_not_block() {
        let host = FakeHost::c4();
        host.with(|sim| {
            sim.stuck
                .push(stuck("jbd2/mmcblk0p1", "[<0>] io_schedule+0x10/0x40\n"))
        });
        assert_eq!(check(&host), Health::Ok);
    }

    #[test]
    fn an_unanswered_audio_service_is_not_healthy() {
        let host = FakeHost::c4();
        host.with(|sim| sim.status_ok = false);
        assert!(matches!(check(&host), Health::Busy(why) if why.contains("does not answer")));
        assert!(returned_true(r#"{"returnValue": true,"audio":[]}"#));
        assert!(!returned_true(r#"{"returnValue":false,"errorText":"x"}"#));
    }

    #[test]
    fn the_connection_list_names_every_source_and_an_empty_one_is_empty() {
        let host = FakeHost::c4();
        assert_eq!(connections(&host), Some(vec![]));
        host.with(|sim| {
            sim.adec_connected = true;
            sim.others = vec![0];
        });
        assert_eq!(
            connections(&host),
            Some(vec!["ADEC1".to_owned(), "ADEC0".to_owned()])
        );
        host.with(|sim| sim.status_ok = false);
        assert_eq!(connections(&host), None);
    }

    #[test]
    fn the_hdmi_service_answers_or_it_does_not() {
        let host = FakeHost::c4();
        assert!(hdmi_answers(&host));
        host.with(|sim| sim.hdmi_ok = false);
        assert!(!hdmi_answers(&host));
    }
}
