//! `ssgw-accept` as xinetd runs it: the connection on its standard input, handed to the gateway or to LG's `nsl`.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(5);

/// An `nsl` that reports its arguments, then copies five bytes of its input, proving it was given the connection.
fn fake_nsl(dir: &Path) -> std::path::PathBuf {
    let nsl = dir.join("nsl");
    std::fs::write(&nsl, "#!/bin/sh\nprintf 'nsl %s\\n' \"$*\"\nhead -c 5\n").unwrap();
    std::fs::set_permissions(&nsl, std::fs::Permissions::from_mode(0o755)).unwrap();
    nsl
}

/// Runs `ssgw-accept` on one end of a socket pair; returns it and the client's end.
fn accept(dir: &Path, args: &[&str]) -> (Child, UnixStream) {
    let (client, conn) = UnixStream::pair().unwrap();
    client.set_read_timeout(Some(WAIT)).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_ssgw-accept"))
        .args(args)
        .env("SSGW_RUN_DIR", dir.join("run"))
        .env("SSGW_NSL", fake_nsl(dir))
        .env("SSGW_FALLBACK_DIR", dir.join("fallback"))
        .stdin(Stdio::from(OwnedFd::from(conn)))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    (child, client)
}

fn run_dir(dir: &Path, mode: u32) -> UnixListener {
    let run = dir.join("run");
    std::fs::create_dir(&run).unwrap();
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(mode)).unwrap();
    UnixListener::bind(run.join("accept.sock")).unwrap()
}

fn recv_fd(sock: RawFd) -> (u8, OwnedFd) {
    let mut byte = [0u8];
    let mut iov = libc::iovec { iov_base: byte.as_mut_ptr().cast(), iov_len: 1 };
    let mut control = [0u64; 8];
    // SAFETY: the message points at live locals; one descriptor comes back in the control buffer.
    unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = std::mem::size_of_val(&control) as _;
        assert_eq!(libc::recvmsg(sock, &mut msg, 0), 1);
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        assert!(!cmsg.is_null() && (*cmsg).cmsg_type == libc::SCM_RIGHTS);
        (byte[0], OwnedFd::from_raw_fd(std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>())))
    }
}

fn output(child: Child) -> (i32, String) {
    let out = child.wait_with_output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn a_ready_gateway_takes_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let gateway = run_dir(dir.path(), 0o755);
    let (child, mut client) = accept(dir.path(), &["3001"]);
    let (mut s, _) = gateway.accept().unwrap();
    s.write_all(b"R").unwrap();
    let (mode, fd) = recv_fd(s.as_raw_fd());
    s.write_all(b"T").unwrap();
    assert_eq!(mode, b'T');
    assert_eq!(output(child), (0, String::new()));

    let mut taken = UnixStream::from(fd);
    taken.write_all(b"hello").unwrap();
    let mut got = [0u8; 5];
    client.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"hello", "the gateway holds the client's connection");
}

#[test]
fn without_a_gateway_lg_serves_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let (child, mut client) = accept(dir.path(), &["3001", "--flag"]);
    client.write_all(b"bytes").unwrap();
    assert_eq!(output(child), (0, "nsl 3001 --flag\nbytes".into()));

    let (child, mut client) = accept(dir.path(), &["3000"]);
    client.write_all(b"plain").unwrap();
    assert_eq!(output(child), (0, "nsl 3000\nplain".into()), "other ports are LG's alone");
}

#[test]
fn a_gateway_that_refuses_or_is_untrusted_leaves_the_connection_to_lg() {
    let dir = tempfile::tempdir().unwrap();
    let gateway = run_dir(dir.path(), 0o755);
    let (child, mut client) = accept(dir.path(), &["3001"]);
    drop(gateway.accept().unwrap());
    client.write_all(b"after").unwrap();
    assert_eq!(output(child), (0, "nsl 3001\nafter".into()), "closed without a ready byte");

    let open = tempfile::tempdir().unwrap();
    let _gateway = run_dir(open.path(), 0o777);
    let (child, mut client) = accept(open.path(), &["3001"]);
    client.write_all(b"other").unwrap();
    assert_eq!(output(child), (0, "nsl 3001\nother".into()), "a run directory anyone can write to is not trusted");
}

#[test]
fn a_gateway_that_never_confirms_gets_the_connection_closed() {
    let dir = tempfile::tempdir().unwrap();
    let gateway = run_dir(dir.path(), 0o755);
    let (child, client) = accept(dir.path(), &["3001"]);
    let (mut s, _) = gateway.accept().unwrap();
    s.write_all(b"R").unwrap();
    let (_, fd) = recv_fd(s.as_raw_fd());
    drop(s);
    assert_eq!(output(child), (1, String::new()), "LG does not answer a connection the gateway may hold");
    drop(fd);
    let mut rest = Vec::new();
    (&client).read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty());
}

/// The fall-back records `ssgw-accept` kept under `dir`, newest last.
fn records(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir.join("fallback"))
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect())
        .unwrap_or_default();
    names.sort();
    names.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect()
}

fn field<'a>(record: &'a str, key: &str) -> &'a str {
    record.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(": ")).unwrap_or_else(|| panic!("no {key} in {record}"))
}

#[test]
fn a_fall_back_is_recorded_with_why_and_the_gateways_state_and_repeats_are_counted() {
    let dir = tempfile::tempdir().unwrap();
    // the gateway's socket exists but nothing listens on it
    drop(run_dir(dir.path(), 0o755));
    for n in 1..=2 {
        let (child, mut client) = accept(dir.path(), &["3001", "--flag"]);
        client.write_all(b"bytes").unwrap();
        assert_eq!(output(child), (0, "nsl 3001 --flag\nbytes".into()), "LG serves it all the same");
        let kept = records(dir.path());
        assert_eq!(kept.len(), 1, "a run of the same reason is one episode");
        let r = &kept[0];
        assert_eq!(field(r, "code"), "not-running");
        assert!(field(r, "detail").contains("accept.sock"), "{r}");
        assert_eq!(field(r, "count"), n.to_string());
        assert_eq!(field(r, "connection"), "3001 --flag");
        let gateway = field(r, "gateway");
        assert!(gateway.contains("mode 0755") && gateway.contains("accept.sock present") && gateway.contains("control.sock missing"), "{gateway}");
        assert!(field(r, "first").ends_with('Z') && field(r, "last").ends_with('Z') && field(r, "first-unix") <= field(r, "last-unix"));
        assert!(!field(r, "boot").is_empty());
        assert!(field(r, "summary").contains("to LG's gateway"), "{r}");
    }
}

#[test]
fn each_reason_for_falling_back_is_told_apart() {
    let dir = tempfile::tempdir().unwrap();
    let (child, mut client) = accept(dir.path(), &["3001"]);
    client.write_all(b"12345").unwrap();
    output(child);
    assert_eq!(field(&records(dir.path())[0], "code"), "no-run-dir");

    let gateway = run_dir(dir.path(), 0o755);
    let (child, mut client) = accept(dir.path(), &["3001"]);
    drop(gateway.accept().unwrap());
    client.write_all(b"12345").unwrap();
    output(child);
    assert_eq!(field(&records(dir.path())[1], "code"), "refused");

    let open = tempfile::tempdir().unwrap();
    let _gateway = run_dir(open.path(), 0o777);
    let (child, mut client) = accept(open.path(), &["3001"]);
    client.write_all(b"12345").unwrap();
    output(child);
    assert_eq!(field(&records(open.path())[0], "code"), "untrusted-run-dir");
}

#[test]
fn no_fall_back_is_recorded_when_the_gateway_takes_the_connection_or_for_another_port() {
    let dir = tempfile::tempdir().unwrap();
    let gateway = run_dir(dir.path(), 0o755);
    let (child, _client) = accept(dir.path(), &["3001"]);
    let (mut s, _) = gateway.accept().unwrap();
    s.write_all(b"R").unwrap();
    recv_fd(s.as_raw_fd());
    s.write_all(b"T").unwrap();
    assert_eq!(output(child), (0, String::new()));
    assert!(records(dir.path()).is_empty(), "taken by the gateway");

    // a gateway that never confirms drops the connection: LG does not serve it, so it is no fall back
    let (child, _client) = accept(dir.path(), &["3001"]);
    let (mut s, _) = gateway.accept().unwrap();
    s.write_all(b"R").unwrap();
    recv_fd(s.as_raw_fd());
    drop(s);
    assert_eq!(output(child).0, 1);
    assert!(records(dir.path()).is_empty());

    let (child, mut client) = accept(dir.path(), &["3000"]);
    client.write_all(b"plain").unwrap();
    output(child);
    assert!(records(dir.path()).is_empty(), "port 3000 is LG's alone");
}
