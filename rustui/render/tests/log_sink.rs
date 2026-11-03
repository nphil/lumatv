//! A host-installed sink receives real renderer events instead of the standalone stderr path.
use std::sync::atomic::{AtomicBool, Ordering};

use device_profile::Rotation;
use lumatv_render::{Gles, Panel, Renderer, set_log_sink};

static COMPILED: AtomicBool = AtomicBool::new(false);

#[test]
fn renderer_construction_sends_its_events_to_the_hosts_sink() {
    fn sink(message: &dyn std::fmt::Display) {
        if message.to_string().starts_with("shader programs compiled and linked in ") {
            COMPILED.store(true, Ordering::Relaxed);
        }
    }
    set_log_sink(sink).unwrap();
    let gl = Gles::software(c"libEGL.so.1").expect("Mesa software GL is required");
    let panel = Panel { rotation: Rotation::R0, logical_width: 64.0, logical_height: 64.0, width: 64, height: 64 };
    let _renderer = Renderer::new(gl, panel).unwrap();
    assert!(COMPILED.load(Ordering::Relaxed), "the renderer event did not reach its host sink");
    assert!(set_log_sink(sink).is_err(), "an installed host sink cannot be silently replaced");
}

#[test]
fn a_full_nonblocking_stderr_does_not_terminate_the_renderer() {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::process::{Command, Stdio};
    const CHILD: &str = "LUMATV_TEST_RENDER_STDERR";
    if std::env::var_os(CHILD).is_some() {
        let gl = Gles::software(c"libEGL.so.1").expect("Mesa software GL is required");
        let panel = Panel { rotation: Rotation::R0, logical_width: 64.0, logical_height: 64.0, width: 64, height: 64 };
        let _renderer = Renderer::new(gl, panel).unwrap();
        return;
    }
    let mut fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) }, 0);
    let (_reader, writer) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let bytes = [0u8; 4096];
    while unsafe { libc::write(writer.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) } >= 0 {}
    assert_eq!(std::io::Error::last_os_error().kind(), std::io::ErrorKind::WouldBlock);
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "a_full_nonblocking_stderr_does_not_terminate_the_renderer", "--nocapture"])
        .env(CHILD, "1").stdout(Stdio::null()).stderr(Stdio::from(writer)).status().unwrap();
    assert!(status.success(), "a full inherited log pipe must drop an event, not terminate the renderer");
}

