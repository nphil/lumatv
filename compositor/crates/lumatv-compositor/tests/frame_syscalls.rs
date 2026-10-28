//! The frame path asks the kernel for no memory once it is warm (LPC 2026: on the set's 5.4 kernel every `mmap`, `munmap`,
//! `mprotect` and `brk` of the compositor takes `mmap_sem`, the lock a page fault read back from flash or swap holds, so a frame
//! that trims its heap can wait behind a fault in another thread). Counted at the system-call level by `syscount`'s seccomp hook,
//! for the server thread (the compositor's loop) over 300 frames of a shm client after 40 of warm-up, the real `State::tick` on
//! the software output; with `mem::tune_malloc` and `mem::pregrow` applied as `run()` applies them.
//!
//! It runs without libtest (`harness = false`) so the work is on the process's first thread, in the main arena, as the loop is.

mod harness;
mod syscount;

use harness::{Client, Server};
use lumatv_compositor::mem;
use wayland_client::protocol::wl_buffer::WlBuffer;

/// Temporary buffers of growing size, as frames of growing damage make: what a malloc at its defaults asks the kernel for.
fn big_buffers() {
    for mib in [1usize, 3, 6, 9, 12] {
        let v = vec![1u8; mib << 20];
        std::hint::black_box(&v);
        drop(v);
    }
}

fn frames(server: &Server, client: &mut Client, card: &harness::Window, buffers: &[WlBuffer], n: usize) {
    for i in 0..n {
        client.present(&card.surface, &buffers[i % buffers.len()]);
        client.roundtrip();
        server.eval(|s| {
            s.tick();
        });
    }
}

fn run() -> Result<(), String> {
    // One arena for every thread, as the compositor's frame path is all on its first thread: the server thread of this test is
    // not the first thread, so its allocations are sent to the first thread's arena. Before any thread exists.
    // SAFETY: mallopt only sets the allocator's parameters.
    unsafe { libc::mallopt(libc::M_ARENA_MAX, 1) };
    if let Err(why) = syscount::install() {
        println!("skipped: {why}");
        return Ok(());
    }
    let me = syscount::gettid();

    // The hook sees what glibc does at its defaults for big temporary buffers.
    syscount::watch(me);
    big_buffers();
    let untuned = syscount::stop();
    println!("malloc at its defaults, five temporary buffers of 1 to 12 MiB: {untuned}");
    if untuned.0[0] < 5 || untuned.0[1] < 5 {
        return Err(format!("the hook should see an mmap and a munmap for each big buffer at glibc's defaults, saw {untuned}"));
    }

    // As `run()` begins.
    let refused = mem::tune_malloc();
    if !refused.is_empty() {
        return Err(format!("malloc refused {refused:?}"));
    }
    mem::pregrow(mem::PREGROW);
    syscount::watch(me);
    big_buffers();
    let tuned = syscount::stop();
    println!("tuned and pre-grown, the same buffers: {tuned}");
    if tuned.total() != 0 {
        return Err(format!("tuned malloc still asked the kernel for memory for buffers inside the heap it holds: {tuned}"));
    }

    let mut server = Server::start();
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Warm");
    let buffers: Vec<WlBuffer> = [0xff19_80e6u32, 0xffe6_8019, 0xff80_e619].iter().map(|&c| client.shm_buffer(640, 360, c)).collect();
    frames(&server, &mut client, &card, &buffers, 40);
    let tid = server.eval(|_| syscount::gettid());
    let drawn_before = server.eval(|s| s.stats.frames_drawn);

    syscount::watch(tid);
    frames(&server, &mut client, &card, &buffers, 300);
    let counted = syscount::stop();
    let drawn = server.eval(|s| s.stats.frames_drawn) - drawn_before;
    println!("the server thread over {drawn} warm frames drawn: {counted}");
    if drawn < 300 {
        return Err(format!("only {drawn} of the 300 frames were drawn: the count says nothing about a frame path that did not run"));
    }
    if counted.total() != 0 {
        return Err(format!("the frame path asked the kernel for memory over 300 warm frames: {counted}"));
    }

    // And the hook would have seen it: a thread made on the frame path, which the compositor does not do (the shell client's start is one
    // thread for the process's life, `shell-spawn`).
    syscount::watch(tid);
    server.eval(|_| {
        std::thread::spawn(|| ()).join().unwrap();
    });
    let spawned = syscount::stop();
    println!("a thread made and joined on the server thread: {spawned}");
    if spawned.0[0] == 0 || spawned.0[2] == 0 {
        return Err(format!("a thread's stack is an mmap and its guard an mprotect; the hook saw {spawned}"));
    }
    Ok(())
}

fn main() {
    match run() {
        Ok(()) => println!("ok"),
        Err(e) => {
            eprintln!("FAILED: {e}");
            std::process::exit(1);
        }
    }
}
