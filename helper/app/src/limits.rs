//! Process limits set once at start.

use std::io;

/// Turns core dumps off for the helper and every program it starts (the plugins, the beszel agent, the camera feeder, the picture
/// decoder). SAM and the boot hook start processes with an unlimited core size and the kernel's core pattern is the relative `core`, so
/// a crash drops a 160-290 MB file (mode 0777, with whatever the process held) into the working directory, which survives a reinstall.
/// The hard limit goes too, so nothing below can raise it again.
pub fn no_core_dumps() -> io::Result<()> {
    let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: a stack rlimit for setrlimit to read.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_soft_and_the_hard_core_limit_are_zero() {
        no_core_dumps().unwrap();
        let mut now = libc::rlimit { rlim_cur: 1, rlim_max: 1 };
        // SAFETY: a stack rlimit for getrlimit to fill.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut now) }, 0);
        assert_eq!((now.rlim_cur, now.rlim_max), (0, 0));
    }
}
