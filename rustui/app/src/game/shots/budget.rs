//! Plan-budget walks: a key script played at the TV's 120 Hz on the host's software GL, every frame held to the numbers the design
//! stands on (BUILD-PLAN §6.2). Software GL makes the times meaningless; what the renderer is asked to draw, upload and capture is not.
//!
//! `GAME_BUDGET=all` runs every walk; a comma list runs the walks whose name contains one of its entries:
//! `GAME_BUDGET=home cargo test -p lumatv-rustui --lib budget_walks -- --nocapture`. The peaks of each are printed as a markdown table.
//! A walk fails on a frame that draws more than [`LAYERS`] layers, captures a backdrop after the page's first frames, draws two
//! full-screen layers (the canvas excluded) or puts more than one stage-size picture on the art layer.
//!
//! A slice puts its walks in its own `shots/<slice>.rs` as `pub(super) fn walks() -> Vec<Walk>` and adds them to [`registry`]. Every
//! screen the shots show also gets a walk of its own ([`screen_walks`]), which names no page and so follows the screens as they change.
use super::*;
use lumatv_render::host::FrameCost;

/// The most layers a frame may draw: the TV's cost starts near 1.5 ms and climbs 0.085 ms a layer, and the cliff at the 8.33 ms of a
/// 120 Hz frame is near 80.
const LAYERS: u32 = 44;
/// How long a page may take to settle, its art decoded and its glyphs placed, before the walk gives up.
const SETTLE: Duration = Duration::from_secs(30);
/// The frames at rest between the page settling and the first key, so that a late draw is seen.
const REST_FRAMES: u32 = 20;
/// How many broken frames a walk names; the rest are counted.
const NAMED: usize = 4;

/// One input of a walk.
#[derive(Clone)]
pub(super) enum Step {
    /// The key goes down and up, and a frame is drawn.
    Tap(Key),
    /// Frames pass with no input for this many milliseconds.
    Wait(u32),
    /// Frames pass until the page's art and text have settled: a dip has run its course.
    Settle,
    /// The page must be in this state now, or the walk is broken.
    Expect(&'static str, fn(&Model, &GameUi) -> bool),
}

/// A key script played from `base`.
pub(super) struct Walk {
    pub(super) name: String,
    pub(super) base: Model,
    pub(super) script: Vec<Step>,
}

/// The largest each number of the renderer's frame cost got in one phase of a walk. The harness draws whole frames (no direct scanout), so
/// the drawn and damaged areas are always the full panel and are left out.
#[derive(Clone, Copy, Default)]
struct Peaks {
    layers: u32,
    full_screen: usize,
    draw_calls: u32,
    uploaded_bytes: u32,
}
impl Peaks {
    fn see(&mut self, cost: &FrameCost, full_screen: usize) {
        self.layers = self.layers.max(cost.layers);
        self.full_screen = self.full_screen.max(full_screen);
        self.draw_calls = self.draw_calls.max(cost.draw_calls);
        self.uploaded_bytes = self.uploaded_bytes.max(cost.uploaded_bytes);
    }
}

struct Report {
    name: String,
    frames: u32,
    /// The page's first frames, until its art and text settled, and then the rest of the walk.
    first: Peaks,
    keys: Peaks,
    /// Frames that captured a backdrop after the page's first frames.
    captures: u32,
    /// The first few broken frames, and how many there were.
    broken: Vec<String>,
    broken_frames: u32,
}

struct Run<'a> {
    shots: &'a mut Shots,
    model: Model,
    origin_ms: u64,
    elapsed_ns: u64,
    page: Option<Page>,
    /// From a page showing until its art and text have settled.
    entering: bool,
    report: Report,
}
impl Run<'_> {
    /// One frame of the walk: the model's clock moves on, the page is shown and drawn, and the cost is held to the budget.
    fn frame(&mut self) -> io::Result<()> {
        self.elapsed_ns += self.shots.frame_ns;
        let now_ms = self.origin_ms + self.elapsed_ns / 1_000_000;
        self.model.step(Input::Tick { now_ms, unix_s: 1_800_000_000 + now_ms / 1000 });
        if self.page != Some(self.model.view.page) {
            self.page = Some(self.model.view.page);
            self.entering = true;
        }
        self.shots.page.show(&self.model)?;
        let cost = self.shots.frame()?;
        let busy = self.shots.page.art_busy();
        let entry = self.entering;
        self.entering &= busy;
        let full_screen = self.shots.page.full_screen_layers();
        let frame = self.report.frames;
        self.report.frames += 1;
        let peaks = if entry { &mut self.report.first } else { &mut self.report.keys };
        peaks.see(&cost, full_screen);
        let uploads = self.shots.page.take_stage_uploads();
        let mut broken = Vec::new();
        if cost.layers > LAYERS {
            broken.push(format!("{} layers, the ceiling is {LAYERS}", cost.layers));
        }
        if cost.backdrops > 0 && !entry {
            self.report.captures += 1;
            broken.push(format!("{} backdrops captured after the page's first frames", cost.backdrops));
        }
        if full_screen > 1 {
            broken.push(format!("{full_screen} full-screen layers"));
        }
        if uploads > 1 {
            broken.push(format!("{uploads} stage pictures put on the art layer"));
        }
        for what in broken {
            self.flag(frame, what);
        }
        if busy {
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
    /// Counts a broken frame, and names the first few.
    fn flag(&mut self, frame: u32, what: String) {
        self.report.broken_frames += 1;
        if self.report.broken.len() < NAMED {
            self.report.broken.push(format!("frame {frame} on {:?}: {what}", self.model.view.page));
        }
    }
    /// Frames until the page's art and text have settled; the walk is broken if they never do.
    fn settle(&mut self) -> io::Result<()> {
        let end = Instant::now() + SETTLE;
        loop {
            self.frame()?;
            if !self.shots.page.art_busy() {
                return Ok(());
            }
            if Instant::now() > end {
                return Err(io::Error::other(format!("{}: the page did not settle", self.report.name)));
            }
        }
    }
    /// Frames for `ms` milliseconds of the panel's time.
    fn wait(&mut self, ms: u32) -> io::Result<()> {
        let frames = (u64::from(ms) * 1_000_000).div_ceil(self.shots.frame_ns);
        (0..frames).try_for_each(|_| self.frame())
    }
}

fn run(shots: &mut Shots, walk: &Walk) -> io::Result<Report> {
    shots.page.invalidate();
    let mut model = walk.base.clone();
    // The shots set a page and a focus on the fixture without rebuilding the focus graph the keys move through.
    model.rebuild_navigation();
    let mut run = Run {
        shots,
        origin_ms: model.now_ms(),
        model,
        elapsed_ns: 0,
        page: None,
        entering: true,
        report: Report {
            name: walk.name.clone(),
            frames: 0,
            first: Peaks::default(),
            keys: Peaks::default(),
            captures: 0,
            broken: Vec::new(),
            broken_frames: 0,
        },
    };
    run.settle()?;
    for _ in 0..REST_FRAMES {
        run.frame()?;
    }
    for step in &walk.script {
        match step {
            Step::Tap(key) => {
                run.model.step(Input::Key { key: *key, down: true });
                run.model.step(Input::Key { key: *key, down: false });
                run.frame()?;
            }
            Step::Wait(ms) => run.wait(*ms)?,
            Step::Settle => run.settle()?,
            Step::Expect(what, holds) => {
                if !holds(&run.model, &run.shots.page) {
                    let frame = run.report.frames;
                    run.flag(frame, format!("expected {what}"));
                }
            }
        }
    }
    Ok(run.report)
}

fn table(reports: &[Report]) -> String {
    let mb = |bytes: u32| f64::from(bytes) / 1e6;
    let mut out = String::from(
        "| walk | frames | layers | full-screen | draw calls | first-frames upload MB | key-frames upload MB | captures | broken |\n\
         |---|---|---|---|---|---|---|---|---|\n",
    );
    for r in reports {
        let (a, b) = (r.first, r.keys);
        out += &format!(
            "| `{}` | {} | {} | {} | {} | {:.1} | {:.1} | {} | {} |\n",
            r.name,
            r.frames,
            a.layers.max(b.layers),
            a.full_screen.max(b.full_screen),
            a.draw_calls.max(b.draw_calls),
            mb(a.uploaded_bytes),
            mb(b.uploaded_bytes),
            r.captures,
            r.broken_frames,
        );
    }
    out
}

/// Every walk there is: what each slice registers, then one for every screen.
fn registry() -> Vec<Walk> {
    let mut walks = super::game::walks();
    walks.extend(super::launch::walks());
    walks.extend(screen_walks());
    walks.extend(super::quick::walks());
    walks.extend(super::settings::walks());
    walks.extend(super::accounts::walks());
    walks.extend(super::library::walks());
    walks.extend(super::home::walks());
    walks.extend(super::media::walks());
    walks
}

/// A script of keys, each followed by a wait in milliseconds.
pub(super) fn taps(keys: &[(Key, u32)]) -> Vec<Step> {
    keys.iter().flat_map(|&(key, wait)| [Step::Tap(key), Step::Wait(wait)]).collect()
}

/// One walk for each screen the shots show: its first frames, a short D-pad sweep and an OK, so that the page's keys, its scroll and
/// whatever OK opens all pass through the budget. Theme, button-glyph and brightness variants draw the same layers and are left out.
fn screen_walks() -> Vec<Walk> {
    let sweep = taps(&[(Key::Down, 100), (Key::Down, 100), (Key::Right, 100), (Key::Right, 100), (Key::Up, 100), (Key::Left, 100), (Key::Ok, 100)])
        .into_iter()
        .chain([Step::Wait(300)])
        .collect::<Vec<_>>();
    screens()
        .into_iter()
        .filter(|(name, _)| !["-light", "-bright", "-pad"].iter().any(|variant| name.contains(variant)))
        .map(|(name, base)| Walk { name, base, script: sweep.clone() })
        .collect()
}

#[test]
fn budget_walks() -> io::Result<()> {
    let selection = std::env::var("GAME_BUDGET").unwrap_or_default();
    if selection.is_empty() {
        return Ok(());
    }
    let mut shots = Shots::at(120.0)?;
    let mut reports = Vec::new();
    for walk in registry() {
        if selection != "all" && !selection.split(',').any(|name| walk.name.contains(name)) {
            continue;
        }
        reports.push(run(&mut shots, &walk)?);
    }
    println!("{}", table(&reports));
    for report in reports.iter().filter(|r| r.broken_frames > 0) {
        println!("{}: {} broken frames", report.name, report.broken_frames);
        for frame in &report.broken {
            println!("    {frame}");
        }
    }
    let failed = reports.iter().filter(|r| r.broken_frames > 0).count();
    assert!(failed == 0, "{failed} of {} walks broke the budget", reports.len());
    Ok(())
}
