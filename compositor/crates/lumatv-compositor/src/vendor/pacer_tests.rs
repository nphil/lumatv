// The pacer's tests, included once for each panel the Show is sold with (`pacer::tests`): `PERIOD` is that panel's refresh.

use crate::pacer::*;

const MS: u64 = 1_000_000;

/// A display whose refreshes are at `offset + k * PERIOD` and a flip request that lands on the first refresh
/// at least `lead` after it is made, returning `lag` after that refresh.
struct Display {
    offset: u64,
    lead: u64,
    jitter: u64,
    seed: u64,
}

impl Display {
    fn flip(&mut self, requested_at: u64) -> u64 {
        let earliest = requested_at + self.lead;
        let k = earliest.saturating_sub(self.offset).div_ceil(PERIOD);
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let lag = (self.seed >> 33) % (self.jitter + 1);
        self.offset + k * PERIOD + lag
    }
}

struct Frame {
    slot: u64,
    presented: u64,
    /// Refreshes after its slot it landed, and refreshes it lost (`Pacer::presented`).
    late: i32,
    lost: u32,
    skipped: u32,
}

/// What asks for a frame.
#[derive(Clone, Copy)]
enum Wanted {
    /// A running animation: the frame waits for its latch.
    Animation,
    /// A commit that reached the compositor at this time: the frame starts as soon as it can after that.
    Commit(u64),
}

/// The render/present loop, as `engine::FrameLoop::run` works it, one frame at a time: one frame in flight, the
/// frame before reported (the loop waits for it) before the next latch is worked out, animation frames latched
/// late.
struct Loop<'a> {
    pacer: &'a mut Pacer,
    display: &'a mut Display,
    now: u64,
    /// The previous frame's report, waiting for the moment the render thread looks up.
    unreported: Option<Shown>,
    frames: Vec<Frame>,
}

impl<'a> Loop<'a> {
    fn new(pacer: &'a mut Pacer, display: &'a mut Display, start: u64) -> Self {
        Self {
            pacer,
            display,
            now: start,
            unreported: None,
            frames: Vec::new(),
        }
    }

    /// The render thread's own work on a frame, before it is handed over.
    const SUBMIT: u64 = MS;

    fn report(&mut self) {
        if let Some(shown) = self.unreported.take() {
            let landing = self.pacer.presented(&shown);
            let last = self.frames.last_mut().unwrap();
            (last.late, last.lost) = (landing.late, landing.lost);
            self.now = self.now.max(shown.presented_ns);
        }
    }

    /// One frame with a pipeline of `cost`, the render thread woken `late` after the time it asked for. A frame
    /// a commit asked for starts as soon as the frame before is done and the commit is there; one that only a
    /// running animation asks for waits for its latch.
    fn frame(&mut self, wanted: Wanted, cost: u64, late: u64) {
        self.report();
        let content = match wanted {
            Wanted::Animation => {
                self.now = self.now.max(self.pacer.latch_at(self.now));
                None
            }
            Wanted::Commit(at) => {
                self.now = self.now.max(at);
                Some(at)
            }
        };
        self.now += late;
        let plan = self.pacer.plan(self.now);
        let presented = self.display.flip(self.now + cost);
        self.frames.push(Frame {
            slot: plan.slot_ns,
            presented,
            late: 0,
            lost: 0,
            skipped: plan.skipped,
        });
        self.unreported = Some(Shown {
            slot_ns: plan.slot_ns,
            started_ns: self.now,
            ready_ns: self.now + cost,
            presented_ns: presented,
            content_ns: content,
        });
        self.now += Self::SUBMIT;
    }

    fn finish(mut self) -> Vec<Frame> {
        self.report();
        self.frames
    }
}

/// An animation: `frames` frames, each waiting for its latch.
fn run(
    pacer: &mut Pacer,
    display: &mut Display,
    start: u64,
    frames: usize,
    cost: impl Fn(usize) -> u64,
) -> Vec<Frame> {
    run_late(pacer, display, start, frames, cost, |_| 0)
}

/// The same, with the render thread woken `late(k)` after the time it asked for on frame `k`.
fn run_late(
    pacer: &mut Pacer,
    display: &mut Display,
    start: u64,
    frames: usize,
    cost: impl Fn(usize) -> u64,
    late: impl Fn(usize) -> u64,
) -> Vec<Frame> {
    let mut frame_loop = Loop::new(pacer, display, start);
    for k in 0..frames {
        frame_loop.frame(Wanted::Animation, cost(k), late(k));
    }
    frame_loop.finish()
}

fn display() -> Display {
    Display {
        offset: 3_300_000,
        lead: 500_000,
        jitter: 100_000,
        seed: 7,
    }
}

#[test]
fn a_steady_animation_gets_even_slots_and_a_frame_on_every_refresh() {
    let mut pacer = Pacer::new(PERIOD);
    let frames = run(&mut pacer, &mut display(), 1_000 * MS, 600, |_| 5 * MS);
    for pair in frames.windows(2) {
        assert_eq!(
            pair[1].slot - pair[0].slot,
            PERIOD,
            "slots step by exactly one refresh"
        );
        let gap = pair[1].presented - pair[0].presented;
        assert!(
            gap.abs_diff(PERIOD) <= 100_000,
            "frames are one refresh apart, got {gap}"
        );
    }
    assert!(frames.iter().all(|f| f.late == 0 && f.lost == 0));
}

#[test]
fn a_frame_started_too_late_for_the_next_refresh_is_planned_for_a_later_one_and_says_how_many_it_skipped()
 {
    let mut pacer = Pacer::new(PERIOD);
    // Frame 100 is started 8 ms after it should have been: too late for its refresh, in time for the one after.
    let frames = run_late(
        &mut pacer,
        &mut display(),
        1_000 * MS,
        200,
        |_| 5 * MS,
        |k| if k == 100 { 8 * MS } else { 0 },
    );
    assert_eq!(frames[100].skipped, 1);
    assert_eq!(frames[100].slot - frames[99].slot, 2 * PERIOD);
    // It was planned for that later refresh and arrived on it, so it is not late by its own plan; but a compositor
    // that had started it when the frame before was out of the way would have made the refresh it gave up, so
    // that refresh was lost.
    assert_eq!((frames[100].late, frames[100].lost), (0, 1));
    let gap = frames[100].presented - frames[99].presented;
    assert!(gap.abs_diff(2 * PERIOD) <= 100_000, "{gap}");
    assert_eq!(frames.iter().map(|f| f.skipped).sum::<u32>(), 1);
    // The frames after it are back on every refresh.
    for pair in frames[101..].windows(2) {
        assert_eq!(pair[1].slot - pair[0].slot, PERIOD);
    }
}

#[test]
fn a_frame_woken_a_little_after_its_latch_still_makes_its_refresh() {
    let mut pacer = Pacer::new(PERIOD);
    // Woken 2.4 ms late, inside the slack: the frame is still drawn for its refresh and gets there.
    let frames = run_late(
        &mut pacer,
        &mut display(),
        1_000 * MS,
        200,
        |_| 5 * MS,
        |k| if k % 10 == 5 { 2_400_000 } else { 0 },
    );
    assert!(frames.iter().all(|f| f.skipped == 0 && f.lost == 0));
    for pair in frames.windows(2) {
        assert_eq!(pair[1].slot - pair[0].slot, PERIOD);
        assert!((pair[1].presented - pair[0].presented).abs_diff(PERIOD) <= 100_000);
    }
}

#[test]
fn slow_frames_do_not_make_the_frames_after_them_give_up_refreshes() {
    let mut pacer = Pacer::new(PERIOD);
    // Two frames in a row take 2 ms longer than the rest. The next ones start earlier to be safe, but none of
    // them is planned for a later refresh than the next.
    let frames = run(&mut pacer, &mut display(), 1_000 * MS, 300, |k| {
        if (100..102).contains(&k) {
            7 * MS
        } else {
            5 * MS
        }
    });
    assert!(frames.iter().all(|f| f.skipped == 0 && f.lost == 0));
    for pair in frames.windows(2) {
        assert_eq!(pair[1].slot - pair[0].slot, PERIOD);
        assert!((pair[1].presented - pair[0].presented).abs_diff(PERIOD) <= 100_000);
    }
}

#[test]
fn a_frame_that_lands_a_refresh_before_its_plan_does_not_make_the_next_one_give_up_a_refresh() {
    // Slow frames make the pacer plan for a 12 ms pipeline. After a pause a commit starts an animation and its
    // first frame, which takes 5 ms, is planned for a refresh a whole one after the first it could make whenever
    // the display has one in between, and lands on that earlier one. The frame after it must still start in time
    // for the refresh that follows the one the first really landed on. Every alignment of the pause with the
    // refresh grid is tried.
    for pause in 300..318 {
        let mut pacer = Pacer::new(PERIOD);
        let mut d = display();
        let slow = run(&mut pacer, &mut d, 1_000 * MS, 30, |_| 12 * MS);
        let resume = slow.last().unwrap().presented + pause * MS;
        let mut frame_loop = Loop::new(&mut pacer, &mut d, resume);
        frame_loop.frame(Wanted::Commit(resume), 5 * MS, 0);
        for _ in 0..20 {
            frame_loop.frame(Wanted::Animation, 5 * MS, 0);
        }
        let frames = frame_loop.finish();
        for k in 1..frames.len() {
            assert_eq!(
                (frames[k].skipped, frames[k].lost),
                (0, 0),
                "pause {pause} ms, frame {k}"
            );
            let gap = frames[k].presented - frames[k - 1].presented;
            assert!(
                gap.abs_diff(PERIOD) <= 100_000,
                "pause {pause} ms, frame {k}: gap {gap}"
            );
        }
    }
}

/// A pacer that has seen frames at every refresh, in a loop ready for the next frame.
fn flowing_loop<'a>(pacer: &'a mut Pacer, display: &'a mut Display) -> Loop<'a> {
    let mut frame_loop = Loop::new(pacer, display, 1_000 * MS);
    for _ in 0..30 {
        frame_loop.frame(Wanted::Animation, 5 * MS, 0);
    }
    frame_loop.report();
    frame_loop
}

/// The lab's alarm screen at T3 (timeline trace, 10-01): frames of 6.6 ms, then a tap's commit (an animation started)
/// drawn at 6.7 ms, then the animation's first step bringing ten more layers into view at 13.1 ms. Returns the
/// refreshes that step lost, with or without the engine telling the pacer the commit reshaped the scene.
fn costlier_step_after_a_commit_loses(tell_reshaped: bool) -> u32 {
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let mut frame_loop = Loop::new(&mut pacer, &mut d, 1_000 * MS);
    for _ in 0..30 {
        frame_loop.frame(Wanted::Animation, 6_600_000, 0);
    }
    frame_loop.report();
    let commit = frame_loop.now;
    frame_loop.frame(Wanted::Commit(commit), 6_700_000, 0);
    if tell_reshaped {
        frame_loop.pacer.reshaped();
    }
    frame_loop.frame(Wanted::Animation, 13_100_000, 0);
    frame_loop.report();
    frame_loop.frames.last().unwrap().lost
}

#[test]
fn the_first_animation_step_after_a_reshaping_commit_starts_at_once_and_keeps_its_refresh() {
    assert_eq!(costlier_step_after_a_commit_loses(false), 1);
    assert_eq!(costlier_step_after_a_commit_loses(true), 0);
}

#[test]
fn only_the_frame_right_after_a_reshaping_commit_gives_up_the_latch() {
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let mut frame_loop = flowing_loop(&mut pacer, &mut d);
    frame_loop.pacer.reshaped();
    let now = frame_loop.now;
    assert_eq!(frame_loop.pacer.latch_at(now), now);
    frame_loop.frame(Wanted::Animation, 5 * MS, 0);
    frame_loop.report();
    let now = frame_loop.now;
    assert!(frame_loop.pacer.latch_at(now) > now);
}

#[test]
fn a_refresh_the_content_came_too_late_for_is_not_lost() {
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let mut frame_loop = flowing_loop(&mut pacer, &mut d);
    // A video at half the refresh rate: each commit reaches the compositor 12 ms after the frame before was on the
    // screen, too late for the next refresh, and the frame starts at once. It gives up a refresh that no commit
    // could have made.
    for _ in 0..20 {
        let last = frame_loop.frames.last().unwrap().presented;
        frame_loop.frame(Wanted::Commit(last + 12 * MS), 5 * MS, 0);
        frame_loop.report();
        let f = frame_loop.frames.last().unwrap();
        assert_eq!((f.skipped, f.lost), (1, 0));
    }
}

#[test]
fn a_refresh_the_compositor_started_too_late_for_is_lost_whatever_asked_for_the_frame() {
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let mut frame_loop = flowing_loop(&mut pacer, &mut d);
    // A commit that was there 2 ms after the frame before was shown, taken by a render thread that only got round
    // to it 12 ms later: the refresh it gave up could have been made.
    let last = frame_loop.frames.last().unwrap().presented;
    frame_loop.frame(Wanted::Commit(last + 2 * MS), 5 * MS, 12 * MS);
    frame_loop.report();
    let f = frame_loop.frames.last().unwrap();
    assert_eq!((f.skipped, f.lost), (1, 1));
    // An animation has something new at every refresh; one whose frame starts 23 ms after its latch loses two.
    frame_loop.frame(Wanted::Animation, 5 * MS, 23 * MS);
    frame_loop.report();
    let f = frame_loop.frames.last().unwrap();
    assert_eq!((f.skipped, f.lost), (2, 2));
}

#[test]
fn a_frame_that_costs_more_than_it_was_planned_for_is_late_but_loses_nothing_a_compositor_could_have_saved()
 {
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let mut frame_loop = flowing_loop(&mut pacer, &mut d);
    // A commit arrives 5.9 ms before the refresh after the frame before (10.9 ms after it at 59.64 Hz). A typical frame (5 ms and
    // the flip lead) makes that refresh with 0.4 ms to spare, so the frame is planned for it; this one costs 7 ms and lands on the refresh
    // after. Started at once it could not have done better: nothing was lost.
    let last = frame_loop.frames.last().unwrap().presented;
    frame_loop.frame(Wanted::Commit(last + PERIOD - 5_867_000), 7 * MS, 0);
    frame_loop.report();
    let f = frame_loop.frames.last().unwrap();
    assert_eq!((f.skipped, f.late, f.lost), (0, 1, 0));
}

#[test]
fn an_animation_frame_that_costs_more_than_the_latch_allowed_for_loses_a_refresh_a_prompt_start_would_have_kept()
 {
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let mut frame_loop = flowing_loop(&mut pacer, &mut d);
    // The latch starts a frame as late as the slowest recent frame and the slack allow. A frame that costs 9 ms
    // more starts too late for its refresh; started when the frame before was shown, it would have made it.
    frame_loop.frame(Wanted::Animation, 5 * MS + 9 * MS, 0);
    frame_loop.report();
    let f = frame_loop.frames.last().unwrap();
    assert_eq!(f.lost, 1);
}

#[test]
fn a_frame_that_costs_three_and_a_half_milliseconds_more_than_the_slowest_recent_still_makes_its_refresh()
 {
    // The frames the timeline trace found losing a refresh at T2 (a picture uploaded on the frame that shows it, a
    // backdrop made again): 2 to 4 ms dearer than any of the eight before them. The latch has to leave room for that.
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let mut frame_loop = flowing_loop(&mut pacer, &mut d);
    frame_loop.frame(Wanted::Animation, 5 * MS + 3_500_000, 0);
    frame_loop.report();
    assert_eq!(frame_loop.frames.last().unwrap().lost, 0);
    // The frames after it start no later for it: the slowest of the recent ones is now that one.
    for _ in 0..3 {
        frame_loop.frame(Wanted::Animation, 5 * MS, 0);
    }
    frame_loop.report();
    assert!(frame_loop.frames.iter().rev().take(3).all(|f| f.lost == 0));
}

#[test]
fn a_frame_that_takes_more_than_a_refresh_loses_none_to_the_compositor() {
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let mut frame_loop = flowing_loop(&mut pacer, &mut d);
    for _ in 0..10 {
        frame_loop.frame(Wanted::Animation, 20 * MS, 0);
    }
    // No compositor shows a 20 ms frame at every refresh; how much a frame may cost is the render gate's to say.
    frame_loop.report();
    assert!(frame_loop.frames[31..].iter().all(|f| f.lost == 0));
}

#[test]
fn a_frame_is_meant_for_the_refresh_it_lands_on() {
    let mut pacer = Pacer::new(PERIOD);
    let frames = run(&mut pacer, &mut display(), 1_000 * MS, 200, |_| 5 * MS);
    // Once the phase is learned, slot + phase is where the frame really appeared, to within the jitter.
    let phase = frames[0].presented as i64 - frames[0].slot as i64;
    for f in &frames[10..] {
        let error = f.presented as i64 - (f.slot as i64 + phase);
        assert!(error.abs() <= 200_000, "error {error}");
    }
}

#[test]
fn rendering_starts_late_enough_to_take_the_latest_requests_but_early_enough_to_land() {
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let frames = run(&mut pacer, &mut d, 1_000 * MS, 100, |_| 4 * MS);
    // Once the pacer knows the pipeline takes 4 ms, the latch leaves that plus the flip lead and the slack
    // before the refresh: nowhere near a whole period earlier, and not so late that the frame misses.
    let now = frames.last().unwrap().presented;
    let latch = pacer.latch_at(now) as i64;
    let next_refresh = (pacer.last_slot_ns + PERIOD) as i64 + pacer.phase_ns.unwrap();
    let left = next_refresh - latch;
    assert_eq!(left, (4 * MS + FLIP_LEAD_NS + SLACK_NS) as i64);
    assert!(
        latch > now as i64,
        "the frame starts after the last one was shown"
    );
}

#[test]
fn a_frame_wanted_for_a_later_refresh_waits_for_it_and_one_wanted_for_a_past_one_does_not() {
    let mut pacer = Pacer::new(PERIOD);
    let frames = run(&mut pacer, &mut display(), 1_000 * MS, 100, |_| 4 * MS);
    let now = frames.last().unwrap().presented;
    let next = pacer.last_slot_ns + PERIOD;
    // Asked for the refresh after next, the latch is one refresh later than for the next.
    assert_eq!(
        pacer.latch_for(now, next + PERIOD),
        pacer.latch_at(now) + PERIOD
    );
    assert_eq!(
        pacer.latch_for(now, next + 3 * PERIOD),
        pacer.latch_at(now) + 3 * PERIOD
    );
    // Asked for a refresh that has gone, it is the next one that is meant.
    assert_eq!(pacer.latch_for(now, 0), pacer.latch_at(now));
    assert_eq!(pacer.latch_at(now), pacer.latch_for(now, next));
    // With no timeline at all there is nothing to wait for.
    assert_eq!(Pacer::new(PERIOD).latch_for(5, 1_000_000_000), 5);
}

#[test]
fn frames_wanted_every_second_refresh_are_drawn_for_every_second_refresh_and_none_is_missed() {
    let mut pacer = Pacer::new(PERIOD);
    let mut display = display();
    // A run at every refresh first, so that the timeline is known, then only every second refresh.
    let frames = run(&mut pacer, &mut display, 1_000 * MS, 30, |_| 4 * MS);
    let mut now = frames.last().unwrap().presented;
    let mut slots = Vec::new();
    for _ in 0..100 {
        let wanted = pacer.last_slot_ns + 2 * PERIOD;
        now = now.max(pacer.latch_for(now, wanted));
        let plan = pacer.plan(now);
        let presented = display.flip(now + 4 * MS);
        let landing = pacer.presented(&Shown {
            slot_ns: plan.slot_ns,
            started_ns: now,
            ready_ns: now + 4 * MS,
            presented_ns: presented,
            content_ns: None,
        });
        slots.push((plan.slot_ns, plan.skipped, landing.late));
        now = presented;
    }
    // Each frame is planned for the refresh it was wanted for, one refresh was skipped between frames (a
    // refresh nobody wanted a frame for), and none arrived late.
    for pair in slots.windows(2) {
        assert_eq!(pair[1].0 - pair[0].0, 2 * PERIOD);
    }
    assert!(slots.iter().all(|s| s.2 == 0), "{slots:?}");
}

#[test]
fn a_frame_that_takes_too_long_is_counted_late_and_the_timeline_realigns() {
    let mut pacer = Pacer::new(PERIOD);
    let frames = run(&mut pacer, &mut display(), 1_000 * MS, 300, |k| {
        if k == 100 { 40 * MS } else { 5 * MS }
    });
    let late = frames[100].late;
    assert!(late >= 1, "the stall is noticed");
    assert!(frames[..100].iter().all(|f| f.late == 0));
    assert!(
        frames[101..].iter().all(|f| f.late == 0),
        "and nothing after it"
    );
    // The refreshes it took are exactly the gap that was seen on screen.
    let gap = frames[100].presented - frames[99].presented;
    assert_eq!(late as u64, (gap + PERIOD / 2) / PERIOD - 1);
    // After the stall the slots step evenly again and follow the display.
    for pair in frames[101..].windows(2) {
        assert_eq!(pair[1].slot - pair[0].slot, PERIOD);
    }
    // The slots moved on by the refreshes that went by, so they follow the display again.
    let phase = frames[0].presented as i64 - frames[0].slot as i64;
    let error = frames[299].presented as i64 - (frames[299].slot as i64 + phase);
    assert!(error.abs() <= 200_000, "error {error}");
    let skipped = frames[299].slot - frames[0].slot;
    assert_eq!(skipped, 299 * PERIOD + late as u64 * PERIOD);
}

#[test]
fn a_frame_after_a_pause_starts_a_new_run_and_is_not_counted_late() {
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let first = run(&mut pacer, &mut d, 1_000 * MS, 30, |_| 5 * MS);
    let resume = first.last().unwrap().presented + 5_000 * MS;
    let second = run(&mut pacer, &mut d, resume, 30, |_| 5 * MS);
    assert!(second.iter().all(|f| f.late == 0));
    for pair in second.windows(2) {
        assert_eq!(pair[1].slot - pair[0].slot, PERIOD);
    }
    // The first frame after a pause is drawn for the earliest refresh it can make.
    assert!(second[0].slot >= resume + 5 * MS + FLIP_LEAD_NS);
    assert!(second[0].slot < resume + 5 * MS + FLIP_LEAD_NS + PERIOD);
}

#[test]
fn a_short_gap_keeps_the_timeline_and_lands_on_a_later_refresh_without_counting_late() {
    let mut pacer = Pacer::new(PERIOD);
    let mut d = display();
    let first = run(&mut pacer, &mut d, 1_000 * MS, 30, |_| 5 * MS);
    // A commit turns up 100 ms after the last frame: the next frame is for a refresh 5+ refreshes on.
    let resume = first.last().unwrap().presented + 100 * MS;
    let second = run(&mut pacer, &mut d, resume, 5, |_| 5 * MS);
    assert!(second.iter().all(|f| f.late == 0));
    let gap = second[0].slot - first.last().unwrap().slot;
    assert_eq!(gap % PERIOD, 0, "still on the same refresh grid");
    assert!(gap >= 100 * MS);
}

#[test]
fn a_slow_pipeline_makes_the_pacer_start_earlier_rather_than_miss_every_refresh() {
    let mut pacer = Pacer::new(PERIOD);
    let frames = run(&mut pacer, &mut display(), 1_000 * MS, 200, |_| 12 * MS);
    let after_warmup: u32 = frames[16..].iter().map(|f| f.lost).sum();
    assert_eq!(after_warmup, 0);
    for pair in frames[16..].windows(2) {
        let gap = pair[1].presented - pair[0].presented;
        assert!(gap.abs_diff(PERIOD) <= 100_000, "{gap}");
    }
}

#[test]
fn a_pipeline_longer_than_a_refresh_is_late_every_time_and_says_so() {
    let mut pacer = Pacer::new(PERIOD);
    let frames = run(&mut pacer, &mut display(), 1_000 * MS, 100, |_| 20 * MS);
    // The pacer cannot help: each frame takes more than a refresh. Presentation is spaced at two refreshes.
    let gaps: Vec<u64> = frames[20..]
        .windows(2)
        .map(|p| p[1].presented - p[0].presented)
        .collect();
    assert!(
        gaps.iter().all(|g| g.abs_diff(2 * PERIOD) <= 100_000),
        "{gaps:?}"
    );
}

#[test]
fn frames_are_flowing_until_a_pause_of_more_than_a_refresh_and_a_half() {
    let mut pacer = Pacer::new(PERIOD);
    assert!(!pacer.flowing(1_000 * MS), "nothing has been shown yet");
    let frames = run(&mut pacer, &mut display(), 1_000 * MS, 20, |_| 5 * MS);
    let last = frames.last().unwrap().presented;
    assert!(pacer.flowing(last + 3 * MS));
    assert!(pacer.flowing(last + PERIOD * 3 / 2));
    assert!(!pacer.flowing(last + PERIOD * 3 / 2 + 1));
    assert!(!pacer.flowing(last + 100 * MS));
}

#[test]
fn rounding_of_whole_refreshes_works_in_both_directions() {
    assert_eq!(round_div(0, 10), 0);
    assert_eq!(round_div(4, 10), 0);
    assert_eq!(round_div(5, 10), 1);
    assert_eq!(round_div(-4, 10), 0);
    assert_eq!(round_div(-6, 10), -1);
    assert_eq!(round_div(-25, 10), -2);
}

/// The line the pacer's prediction of a late frame is drawn at: a refresh less what a flip needs and the slack it gives a frame.
const LINE: u64 = PERIOD - FLIP_LEAD_NS - SLACK_NS;

#[test]
fn a_frame_is_predicted_late_when_a_typical_recent_frame_and_the_slack_do_not_fit_a_refresh() {
    for (cost, late) in [
        (5 * MS, false),
        (LINE - MS, false),
        (LINE + MS, true),
        (12 * MS, true),
        (PERIOD, true),
    ] {
        let mut pacer = Pacer::new(PERIOD);
        run(&mut pacer, &mut display(), 1_000 * MS, 40, |_| cost);
        let forecast = pacer.forecast();
        assert_eq!(forecast.headroom_ns < 0, late, "frames of {cost} ns");
        assert_eq!(forecast.headroom_ns, LINE as i64 - cost as i64);
        assert_eq!(forecast.typical_ns, cost);
    }
}

#[test]
fn one_slow_frame_among_quick_ones_predicts_nothing_and_a_run_of_slow_ones_does() {
    // A screen's first frame, a picture uploaded, a backdrop made again: the slack is there for them, the prediction is not.
    let mut pacer = Pacer::new(PERIOD);
    run(&mut pacer, &mut display(), 1_000 * MS, 40, |k| {
        if k % 8 == 3 { 20 * MS } else { 5 * MS }
    });
    assert!(pacer.forecast().headroom_ns > 0, "{}", pacer.forecast().headroom_ns);
    // Three slow frames in the eight are still not what a typical frame costs (the window's median is the fifth of its eight
    // frames, by cost); four are.
    let mut three = Pacer::new(PERIOD);
    run(&mut three, &mut display(), 1_000 * MS, 40, |k| {
        if k >= 37 { 13 * MS } else { 5 * MS }
    });
    assert!(three.forecast().headroom_ns > 0, "{}", three.forecast().headroom_ns);
    let mut four = Pacer::new(PERIOD);
    run(&mut four, &mut display(), 1_000 * MS, 40, |k| {
        if k >= 36 { 13 * MS } else { 5 * MS }
    });
    assert!(four.forecast().headroom_ns < 0, "{}", four.forecast().headroom_ns);
}

#[test]
fn the_prediction_is_the_pacers_own_the_latch_that_has_no_time_to_wait_is_the_frame_it_calls_late() {
    // With frames that cost what they do now the pacer waits for the latch when there is headroom, and, with none, has to start
    // the frame the moment the one before has been shown: the same fact, from the other side.
    for (cost, waits) in [(5 * MS, true), (13 * MS, false)] {
        let mut pacer = Pacer::new(PERIOD);
        let mut d = display();
        let mut frame_loop = Loop::new(&mut pacer, &mut d, 1_000 * MS);
        for _ in 0..30 {
            frame_loop.frame(Wanted::Animation, cost, 0);
        }
        frame_loop.report();
        let now = frame_loop.now;
        assert_eq!(frame_loop.pacer.latch_at(now) > now, waits, "{cost} ns");
        assert_eq!(frame_loop.pacer.forecast().headroom_ns > 0, waits, "{cost} ns");
    }
}

#[test]
fn asking_for_the_typical_frame_allocates_nothing() {
    // Every frame asks (the plan and the ladder), so it is worked out on the stack: nothing is allocated.
    let mut pacer = Pacer::new(PERIOD);
    run(&mut pacer, &mut display(), 1_000 * MS, 20, |_| 6 * MS);
    let mut sum = 0i64;
    let allocations = crate::alloc_tests::counted(|| {
        for _ in 0..1000 {
            sum += pacer.forecast().headroom_ns;
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!(sum, 1000 * pacer.forecast().headroom_ns);
}

