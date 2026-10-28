//! The frame tick wakes the loop ten times a second, not sixty, on a screen at rest, and the first event puts it back (`ticker`).
//! The output here is a still one that says it is at rest when the test says so; the timer is a real timerfd, counted for a second
//! each way, so the wake-ups a second the doc note quotes are measured at the timer.

mod harness;

use harness::{Client, Server};
use lumatv_compositor::output::{Image, Output, Presented, RenderStats, monotonic_ns};
use lumatv_compositor::scene::Plan;
use lumatv_compositor::surface::Surfaces;
use lumatv_compositor::ticker::{IDLE_TICK_NS, Tick};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use wayland_server::protocol::wl_buffer::WlBuffer;

const REFRESH_NS: u64 = 16_666_667;

/// An output that draws nothing and calls the screen at rest when `at_rest` says so.
struct Still {
    at_rest: Arc<AtomicBool>,
}

impl Output for Still {
    fn name(&self) -> &'static str {
        "still"
    }
    fn size(&self) -> (u32, u32) {
        (1920, 1080)
    }
    fn query_buffer(&mut self, _: &WlBuffer) -> Option<((i32, i32), bool)> {
        None
    }
    fn forget_buffer(&mut self, _: &WlBuffer) {}
    fn render(&mut self, _: &Plan, _: &Surfaces) -> io::Result<RenderStats> {
        Ok(RenderStats::default())
    }
    fn present(&mut self) -> io::Result<Presented> {
        Ok(Presented::default())
    }
    fn capture(&mut self) -> io::Result<Image> {
        Err(io::Error::other("nothing drawn"))
    }
    fn idle_ok(&self) -> bool {
        self.at_rest.load(Ordering::SeqCst)
    }
}

/// How many times the timer fires in `span`, reading it the way the loop does.
fn wakeups(tick: &Tick, span: Duration) -> u64 {
    let end = Instant::now() + span;
    let mut n = 0;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        let mut p = libc::pollfd { fd: tick.raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd on the timer's own descriptor.
        if unsafe { libc::poll(&mut p, 1, left.as_millis().max(1) as i32) } == 1 {
            n += tick.read_expirations();
        }
    }
    n
}

#[test]
fn a_screen_at_rest_wakes_the_loop_ten_times_a_second_and_an_event_puts_the_panels_rate_back() {
    let at_rest = Arc::new(AtomicBool::new(false));
    let flag = at_rest.clone();
    let server = Server::start_with(Box::new(move || Box::new(Still { at_rest: flag })));
    server.ticking.store(false, Ordering::SeqCst);

    // Busy output: the panel's rate, whatever the calm.
    let (period, slowed) = server.eval(|s| {
        let mut tick = Tick::new(REFRESH_NS).unwrap();
        tick.rearm(s).unwrap();
        (tick.period_ns(), tick.is_slowed())
    });
    assert_eq!((period, slowed), (REFRESH_NS, false), "the output has not said it is at rest");

    // At rest and no event for a quarter second: ten a second.
    at_rest.store(true, Ordering::SeqCst);
    let tick = server.eval(|s| {
        let mut tick = Tick::new(REFRESH_NS).unwrap();
        tick.calm.event(monotonic_ns().saturating_sub(1_000_000_000), false);
        tick.rearm(s).unwrap();
        tick
    });
    assert_eq!((tick.period_ns(), tick.is_slowed()), (IDLE_TICK_NS, true));
    let slow = wakeups(&tick, Duration::from_secs(1));
    assert!((9..=11).contains(&slow), "{slow} expirations in a second at {} ms", IDLE_TICK_NS / 1_000_000);

    // An event, and the same pass re-arms the timer at the panel's rate.
    let mut tick = tick;
    let tick = server.eval(move |s| {
        tick.calm.event(monotonic_ns(), false);
        tick.rearm(s).unwrap();
        tick
    });
    assert_eq!((tick.period_ns(), tick.is_slowed()), (REFRESH_NS, false), "a commit, a key, a flip: not calm");
    let fast = wakeups(&tick, Duration::from_secs(1));
    assert!((58..=62).contains(&fast), "{fast} expirations in a second at the panel's rate");

    // A key held: not calm for as long as a hold is measured, even with no event since.
    let mut tick = tick;
    let tick = server.eval(move |s| {
        tick.calm.event(monotonic_ns().saturating_sub(1_000_000_000), true);
        tick.rearm(s).unwrap();
        tick
    });
    assert!(!tick.is_slowed(), "a key went down a second ago: its hold is being measured");

    // The setting: 0 keeps the panel's rate for good (`ctl idle 0`, `--idle-tick-ms 0`).
    let mut tick = tick;
    let tick = server.eval(move |s| {
        s.config.idle_tick_ns = 0;
        tick.calm.event(0, false);
        tick.calm.event(0, true);
        tick.rearm(s).unwrap();
        tick
    });
    assert_eq!((tick.period_ns(), tick.is_slowed()), (REFRESH_NS, false));
}

#[test]
fn a_commit_waiting_to_be_drawn_keeps_the_panels_rate_even_at_rest() {
    let at_rest = Arc::new(AtomicBool::new(true));
    let flag = at_rest.clone();
    let mut server = Server::start_with(Box::new(move || Box::new(Still { at_rest: flag })));
    server.ticking.store(false, Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Owed");
    let buffer = client.shm_buffer(64, 64, 0xff19_80e6);
    client.present(&card.surface, &buffer);
    client.roundtrip();
    let (slowed, period) = server.eval(|s| {
        let mut tick = Tick::new(REFRESH_NS).unwrap();
        tick.calm.event(0, false);
        tick.rearm(s).unwrap();
        (tick.is_slowed(), tick.period_ns())
    });
    assert!(!slowed && period == REFRESH_NS, "a commit no frame has shown yet is drawn at the next tick, which must not be 100 ms away");
    // Once the frame has shown it, the tick may slow.
    let slowed = server.eval(|s| {
        s.tick();
        let mut tick = Tick::new(REFRESH_NS).unwrap();
        tick.calm.event(0, false);
        tick.rearm(s).unwrap();
        tick.is_slowed()
    });
    assert!(slowed, "drawn, and the screen at rest");
}
