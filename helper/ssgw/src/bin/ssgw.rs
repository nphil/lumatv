//! The gateway daemon. Its environment, sockets and exit statuses are described in the crate README.

use std::os::fd::{FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};

static STOP_WRITE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(_: libc::c_int) {
    let fd = STOP_WRITE.load(Ordering::Relaxed);
    // SAFETY: write(2) is async-signal-safe; the pipe outlives the handler.
    unsafe { libc::write(fd, b"s".as_ptr().cast(), 1) };
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_owned())
}

fn main() {
    std::panic::set_hook(Box::new(|info| ssgw::log::emit("ERROR", format_args!("panic: {info}"))));
    ssgw::log::set_debug(std::env::var("SSGW_LOG").is_ok_and(|v| v == "debug"));

    let broker = match std::env::var("SSGW_BROKER_FD") {
        Err(_) => None,
        Ok(v) => match v.parse::<i32>() {
            // SAFETY: F_GETFD only reads the descriptor's flags.
            Ok(fd) if fd > 2 && unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0 => {
                // SAFETY: the supervisor passed this descriptor for the gateway alone to own.
                Some(unsafe { OwnedFd::from_raw_fd(fd) })
            }
            _ => {
                ssgw::error!("SSGW_BROKER_FD={v} is not an open descriptor");
                std::process::exit(ssgw::EXIT_UNUSABLE);
            }
        },
    };

    let mut pipe = [0i32; 2];
    // SAFETY: pipe2 fills the two-element array.
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        ssgw::error!("pipe: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    STOP_WRITE.store(pipe[1], Ordering::Relaxed);
    for sig in [libc::SIGTERM, libc::SIGINT] {
        // SAFETY: installs a handler that only writes to the stop pipe.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
            sa.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }

    let cfg = ssgw::Config {
        state_dir: PathBuf::from(env_or("SSGW_STATE_DIR", "/var/lib/lumatv/ssgw")),
        run_dir: PathBuf::from(env_or("SSGW_RUN_DIR", "/tmp/lumatv-ssgw")),
        lg_root: PathBuf::from(env_or("SSGW_LG_ROOT", "/")),
        lg_http: env_or("SSGW_LG_HTTP", "127.0.0.1:3000"),
        luna_send: PathBuf::from(env_or("SSGW_LUNA_SEND", "/usr/bin/luna-send")),
        tls: true,
        check_pins: true,
        broker,
        version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    // SAFETY: the read end was just created and is owned here alone.
    let stop = unsafe { OwnedFd::from_raw_fd(pipe[0]) };
    std::process::exit(ssgw::run(cfg, stop));
}
