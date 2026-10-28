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

/// The Mali blob's threads that carry the loop's GPU work: `mali-cmar-backe` submits every job (a frame's draw, a
/// buffer's first use), `mali-utility-wo` are its workers. Time-shared, they wait behind the clients for a CPU: on the
/// tenth gate's third x1 run a frame's fence export (the loop waiting for the submission) took 7.7 ms and the frame
/// missed its refresh; a new buffer's first job took 11-30 ms to signal.
const MALI_THREADS: [&str; 2] = ["mali-cmar-backe", "mali-utility-wo"];

/// Puts the Mali blob's submission threads in SCHED_FIFO at `fifo_priority`, or back in the time-shared class with
/// `None`. One line per thread says what happened; none means the blob has no such threads (the host).
pub fn mali_threads(fifo_priority: Option<i32>) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else { return out };
    for task in tasks.flatten() {
        let name = std::fs::read_to_string(task.path().join("comm")).unwrap_or_default();
        let name = name.trim();
        if !MALI_THREADS.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let Ok(tid) = task.file_name().to_string_lossy().parse::<libc::pid_t>() else { continue };
        let (policy, prio) = match fifo_priority {
            Some(p) => (libc::SCHED_FIFO, p),
            None => (libc::SCHED_OTHER, 0),
        };
        let param = libc::sched_param { sched_priority: prio };
        // SAFETY: a stack sched_param; the tid is a thread of this process read from /proc.
        let r = unsafe { libc::sched_setscheduler(tid, policy, &param) };
        let what = match (fifo_priority, r) {
            (Some(p), 0) => format!("SCHED_FIFO {p}"),
            (None, 0) => "SCHED_OTHER".into(),
            (_, _) => format!("refused ({})", io::Error::last_os_error()),
        };
        out.push(format!("{name} ({tid}): {what}"));
    }
    out
}
