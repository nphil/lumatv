//! `CLOCK_MONOTONIC` in nanoseconds: the clock every timestamp in the protocol is measured on, and the one the
//! kernel stamps input events with.

pub fn now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a plain clock read into a stack struct.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    // Neither field is negative for CLOCK_MONOTONIC.
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_only_moves_forward() {
        let a = now_ns();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = now_ns();
        assert!(b >= a + 1_500_000, "{a} -> {b}");
    }
}
