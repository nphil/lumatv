//! The library is only useful preloaded, so the test preloads it: the test binary runs itself again with `LD_PRELOAD` set, and the
//! second run (the "child") sends UDP datagrams and asks the library when they returned.
use std::net::UdpSocket;
use std::process::Command;

const CHILD: &str = "SENDTAP_CHILD";

fn mono_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

fn symbol<T>(name: &std::ffi::CStr) -> Option<T> {
    let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    // SAFETY: T is a function pointer type chosen by the caller for this symbol.
    (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut libc::c_void, T>(&p) })
}

fn child() {
    let first_after: extern "C" fn(u64) -> u64 = symbol(c"lumatv_sendtap_first_after").expect("the preload is not in the process");
    let count: extern "C" fn() -> u64 = symbol(c"lumatv_sendtap_count").unwrap();
    let a = UdpSocket::bind("127.0.0.1:0").unwrap();
    let b = UdpSocket::bind("127.0.0.1:0").unwrap();
    let c0 = count();
    let before = mono_ns();
    assert_eq!(a.send_to(b"hello", b.local_addr().unwrap()).unwrap(), 5);
    let after = mono_ns();
    let t = first_after(before);
    assert!(t >= before && t <= after, "the send returned in [{before}, {after}], the tap says {t}");
    assert_eq!(count(), c0 + 1);
    let mut buf = [0u8; 16];
    assert_eq!(b.recv(&mut buf).unwrap(), 5, "the wrapper passes the datagram through");
    // a connected socket's `send` is stamped too, and nothing is stamped for a send that fails
    a.connect(b.local_addr().unwrap()).unwrap();
    a.send(b"x").unwrap();
    assert_eq!(count(), c0 + 2);
    let bad = unsafe { libc::sendto(-1, b"x".as_ptr().cast(), 1, 0, std::ptr::null(), 0) };
    assert_eq!(bad, -1);
    assert_eq!(count(), c0 + 2);
    assert_eq!(first_after(mono_ns() + 1_000_000_000), 0, "no send after a time in the future");
}

#[test]
fn the_preloaded_library_stamps_each_send_return() {
    if std::env::var_os(CHILD).is_some() {
        child();
        return;
    }
    let exe = std::env::current_exe().unwrap();
    // `cargo test` does not build a cdylib for an integration test, so the library is built here, into a target directory of its own.
    let target = std::env::temp_dir().join("lumatv-gfn-sendtap-test-target");
    let built = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["build", "--quiet", "--manifest-path", concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"), "--target-dir"])
        .arg(&target)
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let so = target.join("debug/liblumatv_gfn_sendtap.so");
    assert!(so.exists(), "{} is not built", so.display());
    let out = Command::new(&exe)
        .args(["--exact", "the_preloaded_library_stamps_each_send_return", "--test-threads=1"])
        .env(CHILD, "1")
        .env("LD_PRELOAD", &so)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}
