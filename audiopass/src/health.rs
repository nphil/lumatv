//! The audio-stack health gate that runs before every step that touches LG's route. The C4's sound card control lock can wedge for
//! good (a kernel task stuck in uninterruptible sleep inside `snd_ctl`), and nothing that touches it afterwards can be cancelled.
//! Port of the HEALTH check of the TrueHD study's kit (WEDGE.md).

use crate::host::Host;

const STATUS: &str = "luna://com.webos.service.audiooutput/audio/getStatus";
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
}
