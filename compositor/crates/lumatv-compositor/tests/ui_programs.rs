//! Native engine programs must link off the loop thread at startup and restart. The set's Mali takes 1.3 s to link them.
//! `LUMATV_COMPILE_DELAY_MS` simulates that delay for each program linked by a renderer.
#![cfg(feature = "ui")]

use lumatv_compositor::output::Output;
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl};
use lumatv_compositor::ui::Ui;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
const LINK_MS: u64 = 20;
/// What one pass of the loop may take for an attach: a frame at 60 Hz is 16.7 ms and this is a loaded host's allowance on top of it, an
/// order under what linking the programs on the loop's thread takes at the stand-in's speed (over 900 ms).
const PASS_BUDGET: Duration = Duration::from_millis(100);

/// The tests share one process, and with it the stand-in's setting.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn gl() -> Option<Gl> {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return None;
    }
    // SAFETY: nothing else in this process reads the environment while it is set, and the tests that set it hold the lock.
    unsafe {
        std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        std::env::set_var("LUMATV_COMPILE_DELAY_MS", LINK_MS.to_string());
    }
    Some(Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).expect("Mesa's GL"))
}

fn would_block<T>(answer: &std::io::Result<T>) -> bool {
    answer.as_ref().err().is_some_and(|e| e.kind() == ErrorKind::WouldBlock)
}

#[test]
fn an_attach_while_the_loop_runs_links_nothing_on_the_loop_thread_and_a_restart_links_nothing_at_all() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut gl) = gl() else { return };
    let shm = std::env::temp_dir().join(format!("lumatv-ui-programs-{}", std::process::id()));
    std::fs::create_dir_all(&shm).unwrap();

    // The first ask starts the thread that links the programs and answers at once; the loop asks again at each tick.
    let began = Instant::now();
    let mut worst = Duration::ZERO;
    let mut asks = 0;
    let app = loop {
        let asked = Instant::now();
        let answer = gl.ui_attach(MESA, 1000, shm.clone());
        worst = worst.max(asked.elapsed());
        asks += 1;
        if would_block(&answer) {
            assert!(began.elapsed() < Duration::from_secs(60), "the programs are not linked after a minute");
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        break answer.expect("the engine attaches once its programs are linked");
    };
    // Over 40 programs at the stand-in's 20 ms each: they were linked, on the thread, and the loop was not held for any of it.
    assert!(began.elapsed() >= Duration::from_millis(40 * LINK_MS), "the stand-in's time was not spent: {:?}", began.elapsed());
    assert!(asks > 5, "{asks} asks: the first answered after the programs were linked, so the loop was held for them");
    assert!(worst < PASS_BUDGET, "the loop's thread was held {worst:?} by an attach (a pass may take {PASS_BUDGET:?})");
    let (linked, taken) = {
        let gles = gl.ui().expect("the engine is attached").engine().gles();
        (gles.programs_linked(), gles.programs_taken())
    };
    assert_eq!(linked, 0, "the engine linked {linked} programs itself, on the loop's thread");
    assert!(taken >= 40, "the engine took {taken} programs from the thread's cache");

    // A client that restarts attaches a new engine each time: its programs are the old one's, back in the cache.
    drop(app);
    gl.ui_detach();
    let again = Instant::now();
    let app = gl.ui_attach(MESA, 1000, shm.clone()).expect("the restart's engine attaches at once, with nothing to link");
    assert!(again.elapsed() < PASS_BUDGET, "the restart's attach took {:?}", again.elapsed());
    let gles = gl.ui().expect("the engine is attached").engine().gles();
    assert_eq!((gles.programs_linked(), gles.programs_taken()), (0, taken), "the restart's engine took the old one's programs");
    drop(app);
    drop(gl);
    let _ = std::fs::remove_dir_all(&shm);
}

#[test]
fn a_compositor_that_ends_while_the_programs_are_being_linked_does_not_wait_for_all_of_them() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut gl) = gl() else { return };
    let shm = std::env::temp_dir().join(format!("lumatv-ui-programs-end-{}", std::process::id()));
    std::fs::create_dir_all(&shm).unwrap();
    assert!(would_block(&gl.ui_attach(MESA, 1000, shm.clone())), "the first ask starts the thread");
    // let it link a few programs first
    std::thread::sleep(Duration::from_millis(100));
    let began = Instant::now();
    drop(gl);
    assert!(
        began.elapsed() < Duration::from_millis(400),
        "ending took {:?}: the thread linked on to its end (over 900 ms at the stand-in's speed)",
        began.elapsed()
    );
    let _ = std::fs::remove_dir_all(&shm);
}

#[test]
fn an_engine_built_from_a_complete_cache_that_lacks_a_program_is_refused_and_links_nothing() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // The compositor's context, current on this thread for as long as `_gl` lives.
    let Some(_gl) = gl() else { return };
    let shm = std::env::temp_dir().join(format!("lumatv-ui-programs-strict-{}", std::process::id()));
    std::fs::create_dir_all(&shm).unwrap();
    let driver = std::ffi::CString::new(MESA).unwrap();
    let cache = lumatv_render::programs::Cache::new();
    // SAFETY: `_gl` made the context current on this thread and outlives every use of it here.
    let gles = unsafe { lumatv_render::Gles::current(&driver) }.expect("the context's GL");
    let made = lumatv_render::host::precompile(gles, &cache, 480, 270).expect("the programs are linked");
    assert!(cache.complete() && made.programs >= 40, "{made:?}");

    // The first engine takes every program in the cache; the second finds none and, the cache being complete, links none.
    // SAFETY: as above.
    let first = unsafe { Ui::new(MESA, 1920, 1080, 1000, shm.clone(), &cache) }.expect("the first engine takes the cache's programs");
    let asked = Instant::now();
    // SAFETY: as above.
    let second = unsafe { Ui::new(MESA, 1920, 1080, 1000, shm.clone(), &cache) };
    let error = second.err().expect("an engine that lacks a program is not built");
    assert!(error.to_string().contains("complete cache"), "{error}");
    assert!(asked.elapsed() < PASS_BUDGET, "refusing took {:?}: the engine linked programs", asked.elapsed());
    drop(first);
    let _ = std::fs::remove_dir_all(&shm);
}
