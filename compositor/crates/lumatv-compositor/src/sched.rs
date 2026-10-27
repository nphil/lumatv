//! The frame loop's scheduling class. On the set the loop shared the CPUs at nice 0 with the clients and LG's daemons
//! (LG's own compositor is reniced to -19 minutes after it starts, `perf-tweaks.md`): over X1's 60 s on the ninth gate
//! the loop thread waited 1178 ms in the run queue, and its one 20.1 ms frame gap came with a flip the loop reached 3.4 ms
//! after it landed. SCHED_FIFO puts every wake-up of the loop ahead of every time-shared task; LG's input threads
//! (SCHED_RR 99) and the HDMI interrupt thread (FIFO 50) stay ahead of it, and the kernel's 95 % cap on real-time
//! time (`sched_rt_runtime_us` 950000 on the set) bounds a loop that spins.

use std::io;

/// Makes the calling thread SCHED_FIFO at `priority` (1-99). Needs CAP_SYS_NICE (root on the set); elsewhere the
/// error says so and the thread stays time-shared.
pub fn fifo(priority: i32) -> io::Result<()> {
    let param = libc::sched_param { sched_priority: priority };
    // SAFETY: a stack sched_param; pid 0 is the calling thread.
    if unsafe { libc::sched_setscheduler(0, libc::SCHED_FIFO, &param) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Puts the calling thread back in the time-shared class: a thread the loop spawns (a control job, a PNG write)
/// inherits the loop's class and would otherwise run ahead of every client at the loop's priority.
pub fn time_shared() {
    let param = libc::sched_param { sched_priority: 0 };
    // SAFETY: as above.
    unsafe { libc::sched_setscheduler(0, libc::SCHED_OTHER, &param) };
}

/// The calling thread's class and priority as `/proc` would show them: `"SCHED_FIFO 20"`, `"SCHED_OTHER"`.
pub fn describe() -> String {
    // SAFETY: plain queries of the calling thread.
    let (policy, prio) = unsafe {
        let mut param = libc::sched_param { sched_priority: 0 };
        libc::sched_getparam(0, &mut param);
        (libc::sched_getscheduler(0), param.sched_priority)
    };
    match policy {
        libc::SCHED_FIFO => format!("SCHED_FIFO {prio}"),
        libc::SCHED_RR => format!("SCHED_RR {prio}"),
        libc::SCHED_OTHER => "SCHED_OTHER".into(),
        other => format!("policy {other}"),
    }
}
