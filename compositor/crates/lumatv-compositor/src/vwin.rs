//! The display's spare video windows for LumaTV's own video (`--vwin`): the camera pop-up, the Cameras live view and a picture-in-picture.
//!
//! The display engine has two windows (`NUM_WIN_MAX 2` in the O22 source: the main one and the sub one; `win2` and `win3` are proc slots with
//! no window behind them). LG's resource manager hands the first video the main window and the next the sub window, below every OSD pixel. An
//! app's video is one of them, the video wallpaper (`lumatv-vplane`) another. This module decides who gets which, and says no when none is
//! free, so that the shell shows stills instead: the GPU path that was always there.
//!
//! What it knows (`docs/research/hw-planes-result.md` has the evidence for each line):
//!
//! - **Two windows.** `videooutputd`'s status lists the sinks `MAIN` (z 0) and `SUB0` (z 1); the second reads `connected false` in every run of an
//!   app on the set (Netflix, Plex, YouTube at 1080p and 4K, 24 to 60 fps). The resource table has one main and one sub scaler.
//! - **The sub window is small.** The codec file's `multiview_sub` entries: 1920x1088 at 30 frames a second (60 for VP9), against 4096x2304 at
//!   60 (120 for H.265) on the main window. A camera's own 4K stream goes to the main window or not at all; the bridge's `accept` limits are
//!   the grant's limits.
//! - **A 4K120 video takes both.** `subScalerOccupied` is how `videooutputd` says a stream uses both paths (HFR over 2K), and LumaTV's own NDL
//!   session read `true` once, at 1856x1024 and 60 a second with low-delay on (round 8, the saver). Whether every NDL session occupies the sub
//!   scaler is the question the TV round answers first (check V2); until it does, `apps` mode is for that round.
//! - **Nothing of an app's is touched.** An app's video is never displaced; a second decoder next to it may or may not coexist (check V4), and
//!   `idle` mode never tries.
//!
//! The allocator is pure: it is told what the screen holds ([`World`]) and answers. The glue that feeds it, binds a grant to the window that
//! plays it and draws its hole is at the end of the file (`impl State`).

use serde_json::{Value, json};

use crate::Rect;

/// The most windows a policy may name; the display has two.
pub const MAX_SLOTS: usize = 4;
pub const DEFAULT_SLOTS: usize = 2;

/// `--vwin`: whether LumaTV's own video gets a hardware window, and when.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// No claim is granted: the shell's stills are the picture (the default).
    #[default]
    Off,
    /// A claim is granted while no app has a video element on the screen, and ends the moment one appears.
    Idle,
    /// Also next to an app's video, in the window it leaves, while the stream fits it and the app's video does not use both scalers.
    Apps,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "off" => Some(Mode::Off),
            "idle" => Some(Mode::Idle),
            "apps" => Some(Mode::Apps),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Idle => "idle",
            Mode::Apps => "apps",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub mode: Mode,
    /// How many windows the display is taken to have.
    pub slots: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { mode: Mode::Off, slots: DEFAULT_SLOTS }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
    Av1,
    Vp9,
}

impl Codec {
    pub fn parse(s: &str) -> Option<Codec> {
        match s.to_ascii_lowercase().as_str() {
            "h264" | "avc" | "h.264" => Some(Codec::H264),
            "h265" | "hevc" | "h.265" => Some(Codec::H265),
            "av1" => Some(Codec::Av1),
            "vp9" => Some(Codec::Vp9),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::H265 => "h265",
            Codec::Av1 => "av1",
            Codec::Vp9 => "vp9",
        }
    }
}

/// What a window takes: the largest picture and the fastest rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub w: u32,
    pub h: u32,
    pub fps: u32,
}

/// The window's name: `main`, `sub`, then the two the proc files name and the hardware may not have.
pub fn slot_name(slot: usize) -> &'static str {
    ["main", "sub", "win2", "win3"].get(slot).copied().unwrap_or("?")
}

/// The limits of window `slot` for `codec`, from the capability file the set ships (`device_codec_capability_config.json`, read on the set 2026-10-26):
/// the main window's are the codec's own, the other windows' its `multiview_sub` restriction.
pub fn limits(slot: usize, codec: Codec) -> Limits {
    if slot == 0 {
        match codec {
            Codec::H265 => Limits { w: 4096, h: 2304, fps: 120 },
            _ => Limits { w: 4096, h: 2304, fps: 60 },
        }
    } else {
        match codec {
            Codec::Vp9 => Limits { w: 1920, h: 1088, fps: 60 },
            _ => Limits { w: 1920, h: 1088, fps: 30 },
        }
    }
}

/// What a stream needs of its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Need {
    pub codec: Codec,
    pub w: u32,
    pub h: u32,
    pub fps: u32,
}

impl Need {
    pub fn fits(&self, l: Limits) -> bool {
        self.w <= l.w && self.h <= l.h && self.fps <= l.fps
    }

    pub fn from_json(v: &Value) -> Option<Need> {
        let num = |k: &str| v.get(k).and_then(Value::as_u64).map(|n| n.min(u64::from(u32::MAX)) as u32);
        let codec = v.get("codec").and_then(Value::as_str).and_then(Codec::parse).unwrap_or(Codec::H264);
        Some(Need { codec, w: num("w").or_else(|| num("width"))?, h: num("h").or_else(|| num("height"))?, fps: num("fps").unwrap_or(0) })
    }
}

/// What a claim is for. The kind sets its priority: a claim may take the window of one of lower priority, never of an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The Cameras app's full-screen live view.
    CameraFull,
    /// The camera pop-up's picture.
    CameraPopup,
    /// A picture-in-picture surface of the shell's (later).
    Pip,
}

/// The video wallpaper's priority, below every claim: it is behind Home, and Home hides it behind a full-screen view.
pub const WALL_PRIORITY: u8 = 10;

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "camera-full" => Some(Kind::CameraFull),
            "camera-popup" => Some(Kind::CameraPopup),
            "pip" => Some(Kind::Pip),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::CameraFull => "camera-full",
            Kind::CameraPopup => "camera-popup",
            Kind::Pip => "pip",
        }
    }

    pub fn priority(self) -> u8 {
        match self {
            Kind::CameraFull => 50,
            Kind::CameraPopup => 40,
            Kind::Pip => 30,
        }
    }
}

/// Why a claim has no window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The switch is off.
    Off,
    /// An app has a video element on the screen and the mode leaves it the video plane (`idle`).
    AppVideo,
    /// The app's video uses the main and the sub scaler.
    BothScalers,
    /// Every window is held and none of the holders is a lower claim.
    NoWindow,
    /// The stream is larger than the window that is free takes: the claim carries that window's limits.
    TooBig,
    /// The screen is off.
    Standby,
    /// A claim of higher priority took the window.
    Higher,
}

impl Reason {
    pub fn word(self) -> &'static str {
        match self {
            Reason::Off => "off",
            Reason::AppVideo => "app-video",
            Reason::BothScalers => "both-scalers",
            Reason::NoWindow => "no-window",
            Reason::TooBig => "too-big",
            Reason::Standby => "standby",
            Reason::Higher => "higher",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Reason::Off => "hardware video windows are off",
            Reason::AppVideo => "an app has a video on the screen",
            Reason::BothScalers => "the app's video uses both scalers",
            Reason::NoWindow => "no video window is free",
            Reason::TooBig => "the stream is larger than the free window takes",
            Reason::Standby => "the screen is off",
            Reason::Higher => "a more important video took the window",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimState {
    /// A window is reserved; the video plays in it once the window and its element exist.
    Granted,
    Denied(Reason),
    /// It was granted and has been taken back.
    Preempted(Reason),
}

impl ClaimState {
    pub fn word(self) -> &'static str {
        match self {
            ClaimState::Granted => "granted",
            ClaimState::Denied(_) => "denied",
            ClaimState::Preempted(_) => "preempted",
        }
    }

    pub fn reason(self) -> Option<Reason> {
        match self {
            ClaimState::Granted => None,
            ClaimState::Denied(r) | ClaimState::Preempted(r) => Some(r),
        }
    }
}

/// What a client asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub kind: Kind,
    /// The app id the video window will carry (`-a` of the feeder); empty: the first video window without a claim.
    pub app: String,
    /// Where the picture goes, in output pixels.
    pub rect: Rect,
    pub need: Option<Need>,
}

impl Request {
    /// A request from the UI channel or `ctl`: `{"id", "kind", "app"?, "rect": {"x","y","w","h"}, "need"?: {"codec","w","h","fps"}}`.
    pub fn from_json(v: &Value) -> Result<Request, String> {
        let id = v.get("id").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 64).ok_or("a claim needs an id")?.to_string();
        let kind = v.get("kind").and_then(Value::as_str).and_then(Kind::parse).ok_or("kind is camera-full, camera-popup or pip")?;
        let rect = rect_of(v.get("rect").ok_or("a claim needs a rect")?)?;
        let need = v.get("need").and_then(Need::from_json);
        let app = v.get("app").and_then(Value::as_str).unwrap_or("").to_string();
        Ok(Request { id, kind, app, rect, need })
    }
}

/// `{"x","y","w","h"}` (or `width`, `height`) as a rectangle in output pixels: not empty, not larger than the output, kept inside it.
pub fn rect_of(v: &Value) -> Result<Rect, String> {
    let n = |k: &str, alt: &str| v.get(k).or_else(|| v.get(alt)).and_then(Value::as_i64).and_then(|n| i32::try_from(n).ok());
    let (x, y) = (n("x", "x").ok_or("rect.x")?, n("y", "y").ok_or("rect.y")?);
    let (w, h) = (n("w", "width").ok_or("rect.w")?, n("h", "height").ok_or("rect.h")?);
    let r = Rect::new(x, y, w, h);
    let out = Rect::new(0, 0, crate::OUTPUT_WIDTH as i32, crate::OUTPUT_HEIGHT as i32);
    if w < MIN_EDGE || h < MIN_EDGE {
        return Err(format!("rect is smaller than {MIN_EDGE} pixels"));
    }
    if r.intersect(&out) != r {
        return Err("rect leaves the output".into());
    }
    Ok(r)
}

/// The smallest picture a claim may name, in output pixels. The display's scaler has no limit in the kernel's source (`limited_win_ratio_*` read 0);
/// this is the compositor's own floor, so that a typo is not a pixel (the TV round measures the real one, check V6).
pub const MIN_EDGE: i32 = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub id: String,
    pub kind: Kind,
    pub app: String,
    pub rect: Rect,
    pub need: Option<Need>,
    pub state: ClaimState,
    pub slot: Option<usize>,
    pub limits: Option<Limits>,
    /// The window (`WindowId`) that plays it, once one has been bound.
    pub window: Option<u32>,
    pub seq: u64,
}

/// What the screen holds besides LumaTV's own claims.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct World {
    /// Exported video elements of windows that are not LumaTV's own: an app that will play video.
    pub app_elements: u32,
    /// Of those, the ones whose pipeline is attached (a punch-through context): an app that plays.
    pub app_playing: u32,
    /// Video sinks `videooutputd` reports connected that this compositor does not know (LG's own multi view, a PiP of another service).
    pub other_connected: u32,
    /// The app's video uses the main and the sub scaler (`subScalerOccupied`).
    pub both_scalers: bool,
    /// The wallpaper's decoder is attached.
    pub wall: bool,
    /// The screen is off.
    pub standby: bool,
}

impl World {
    /// How many windows the apps hold: in `idle` an element counts the moment it exists (the pipeline follows it within a second); in `apps` only a
    /// pipeline that is attached does.
    pub fn app_slots(&self, mode: Mode) -> usize {
        let seen = match mode {
            Mode::Idle => self.app_elements.max(self.app_playing),
            _ => self.app_playing,
        }
        .max(self.other_connected);
        let n = seen as usize;
        if self.both_scalers && (n > 0 || mode == Mode::Apps) { n.max(2) } else { n }
    }
}

/// Who holds a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Holder {
    App,
    Wall,
    Claim(String),
}

/// What a grant displaces: the wallpaper (the caller closes its window) or a claim (already marked preempted).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Victim {
    Wall,
    Claim(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Granted { slot: usize, limits: Limits, victims: Vec<Victim> },
    /// `Some` limits are those of the best window there was: what a smaller stream has to fit.
    Denied(Reason, Option<Limits>),
}

#[derive(Debug, Default)]
pub struct Allocator {
    pub policy: Policy,
    claims: Vec<Claim>,
    seq: u64,
}

fn first_free(occ: &[Option<Holder>]) -> Option<usize> {
    occ.iter().position(Option::is_none)
}

impl Allocator {
    pub fn new(policy: Policy) -> Allocator {
        Allocator { policy, ..Allocator::default() }
    }

    pub fn claims(&self) -> &[Claim] {
        &self.claims
    }

    pub fn claim(&self, id: &str) -> Option<&Claim> {
        self.claims.iter().find(|c| c.id == id)
    }

    pub fn claim_mut(&mut self, id: &str) -> Option<&mut Claim> {
        self.claims.iter_mut().find(|c| c.id == id)
    }

    pub fn granted(&self) -> impl Iterator<Item = &Claim> {
        self.claims.iter().filter(|c| c.state == ClaimState::Granted)
    }

    /// Who holds which window now: the apps from the first, then the claims where they were put, then the wallpaper in the first that is left.
    pub fn occupancy(&self, world: &World) -> Vec<Option<Holder>> {
        let mut occ: Vec<Option<Holder>> = vec![None; self.policy.slots];
        for slot in occ.iter_mut().take(world.app_slots(self.policy.mode)) {
            *slot = Some(Holder::App);
        }
        for c in self.granted() {
            let at = c.slot.filter(|s| occ.get(*s).is_some_and(Option::is_none)).or_else(|| first_free(&occ));
            if let Some(i) = at {
                occ[i] = Some(Holder::Claim(c.id.clone()));
            }
        }
        if world.wall
            && let Some(i) = first_free(&occ)
        {
            occ[i] = Some(Holder::Wall);
        }
        occ
    }

    fn priority_of(&self, holder: &Holder) -> Option<u8> {
        match holder {
            Holder::App => None,
            Holder::Wall => Some(WALL_PRIORITY),
            Holder::Claim(id) => self.claim(id).map(|c| c.kind.priority()),
        }
    }

    /// What is wrong with granting anything at all.
    fn gate(&self, world: &World) -> Option<Reason> {
        if self.policy.mode == Mode::Off {
            return Some(Reason::Off);
        }
        if world.standby {
            return Some(Reason::Standby);
        }
        if self.policy.mode == Mode::Idle && world.app_slots(Mode::Idle) > 0 {
            return Some(Reason::AppVideo);
        }
        if world.both_scalers && world.app_slots(self.policy.mode) > 0 {
            return Some(Reason::BothScalers);
        }
        None
    }

    /// Decides a request. The claim is recorded either way (a denial too, so the shell reads its reason until it lets go); a grant that takes a window
    /// from a lower claim marks that claim preempted and names it, and one that takes the wallpaper's names the wallpaper.
    pub fn request(&mut self, world: &World, req: Request) -> Decision {
        self.claims.retain(|c| c.id != req.id);
        self.seq += 1;
        let seq = self.seq;
        let mut claim = Claim {
            id: req.id.clone(),
            kind: req.kind,
            app: req.app.clone(),
            rect: req.rect,
            need: req.need,
            state: ClaimState::Denied(Reason::NoWindow),
            slot: None,
            limits: None,
            window: None,
            seq,
        };
        let decision = self.decide(world, &req);
        match &decision {
            Decision::Granted { slot, limits, victims } => {
                claim.state = ClaimState::Granted;
                claim.slot = Some(*slot);
                claim.limits = Some(*limits);
                for v in victims {
                    if let Victim::Claim(id) = v
                        && let Some(c) = self.claim_mut(id)
                    {
                        c.state = ClaimState::Preempted(Reason::Higher);
                        c.window = None;
                    }
                }
            }
            Decision::Denied(reason, limits) => {
                claim.state = ClaimState::Denied(*reason);
                claim.limits = *limits;
            }
        }
        self.claims.push(claim);
        decision
    }

    fn decide(&self, world: &World, req: &Request) -> Decision {
        if let Some(reason) = self.gate(world) {
            return Decision::Denied(reason, None);
        }
        let occ = self.occupancy(world);
        let prio = req.kind.priority();
        let fits = |slot: usize| req.need.is_none_or(|n| n.fits(limits(slot, n.codec)));
        let cap = |slot: usize| limits(slot, req.need.map_or(Codec::H264, |n| n.codec));
        // a free window first: the first one is main, as the display's own resource manager hands them out
        let free: Vec<usize> = (0..occ.len()).filter(|i| occ[*i].is_none()).collect();
        if let Some(&slot) = free.iter().find(|s| fits(**s)) {
            return Decision::Granted { slot, limits: cap(slot), victims: Vec::new() };
        }
        // then the window of a lower claim or the wallpaper: the lowest holder, and of equals the first window
        let mut takeable: Vec<(u8, usize, Holder)> = (0..occ.len())
            .filter_map(|i| {
                let h = occ[i].clone()?;
                let held = self.priority_of(&h)?;
                (held < prio && fits(i)).then_some((held, i, h))
            })
            .collect();
        takeable.sort_by_key(|(held, i, _)| (*held, *i));
        if let Some((_, slot, holder)) = takeable.into_iter().next() {
            let victim = match holder {
                Holder::Wall => Victim::Wall,
                Holder::Claim(id) => Victim::Claim(id),
                Holder::App => unreachable!("an app's window has no priority"),
            };
            return Decision::Granted { slot, limits: cap(slot), victims: vec![victim] };
        }
        // nothing fits: say why, and what would have
        let reachable: Vec<usize> =
            (0..occ.len()).filter(|i| occ[*i].is_none() || occ[*i].as_ref().and_then(|h| self.priority_of(h)).is_some_and(|held| held < prio)).collect();
        match reachable.first() {
            Some(&best) => Decision::Denied(Reason::TooBig, Some(cap(best))),
            None => Decision::Denied(Reason::NoWindow, None),
        }
    }

    /// A claim that was granted for a different place: the picture moved or changed size (a pop-up that follows the overlay layout), and the stream
    /// it needs changed. Returns whether the claim is still granted: a need that no longer fits its window ends it (it was preempted by itself:
    /// the shell asks again).
    pub fn update(&mut self, id: &str, rect: Option<Rect>, need: Option<Need>) -> bool {
        let Some(c) = self.claims.iter_mut().find(|c| c.id == id && c.state == ClaimState::Granted) else { return false };
        if let Some(r) = rect {
            c.rect = r;
        }
        if let Some(n) = need {
            c.need = Some(n);
            if let Some(slot) = c.slot
                && !n.fits(limits(slot, n.codec))
            {
                c.state = ClaimState::Preempted(Reason::TooBig);
                c.window = None;
                return false;
            }
            c.limits = c.slot.map(|s| limits(s, n.codec));
        }
        true
    }

    pub fn release(&mut self, id: &str) -> bool {
        let before = self.claims.len();
        self.claims.retain(|c| c.id != id);
        self.claims.len() != before
    }

    pub fn release_all(&mut self) -> usize {
        std::mem::take(&mut self.claims).len()
    }

    /// The world or the policy changed: the claims that no longer hold end. Returns the ids that were granted and are not now.
    pub fn settle(&mut self, world: &World) -> Vec<String> {
        let mut ended = Vec::new();
        let gate = self.gate(world);
        let mut order: Vec<usize> = (0..self.claims.len()).filter(|i| self.claims[*i].state == ClaimState::Granted).collect();
        if let Some(reason) = gate {
            for i in order {
                self.claims[i].state = ClaimState::Preempted(reason);
                self.claims[i].window = None;
                ended.push(self.claims[i].id.clone());
            }
            return ended;
        }
        // more holders than windows: the lowest claims give theirs up
        let apps = world.app_slots(self.policy.mode);
        let wall = usize::from(world.wall);
        let mut held = apps + order.len() + wall;
        order.sort_by_key(|i| (self.claims[*i].kind.priority(), std::cmp::Reverse(self.claims[*i].seq)));
        for i in order {
            if held <= self.policy.slots {
                break;
            }
            self.claims[i].state = ClaimState::Preempted(Reason::NoWindow);
            self.claims[i].window = None;
            ended.push(self.claims[i].id.clone());
            held -= 1;
        }
        ended
    }

    /// A claim's own record for the topic and `ctl`.
    pub fn claim_json(c: &Claim, placed: bool, sink: Option<&str>) -> Value {
        json!({
            "id": c.id,
            "kind": c.kind.name(),
            "app": c.app,
            "state": c.state.word(),
            "reason": c.state.reason().map_or("", Reason::word),
            "text": c.state.reason().map_or("", Reason::text),
            "slot": c.slot.map(slot_name),
            "limits": c.limits.map(|l| json!({"w": l.w, "h": l.h, "fps": l.fps})),
            "rect": {"x": c.rect.x, "y": c.rect.y, "w": c.rect.w, "h": c.rect.h},
            "need": c.need.map(|n| json!({"codec": n.codec.name(), "w": n.w, "h": n.h, "fps": n.fps})),
            "window": c.window,
            "placed": placed,
            "sink": sink,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(mode: Mode) -> Policy {
        Policy { mode, slots: 2 }
    }

    fn req(id: &str, kind: Kind, need: Option<Need>) -> Request {
        Request { id: id.into(), kind, app: String::new(), rect: Rect::new(1200, 134, 520, 292), need }
    }

    fn need(codec: Codec, w: u32, h: u32, fps: u32) -> Option<Need> {
        Some(Need { codec, w, h, fps })
    }

    fn granted(d: &Decision) -> Option<(usize, Vec<Victim>)> {
        match d {
            Decision::Granted { slot, victims, .. } => Some((*slot, victims.clone())),
            Decision::Denied(..) => None,
        }
    }

    #[test]
    fn it_is_off_unless_asked_and_off_denies_everything() {
        assert_eq!(Policy::default().mode, Mode::Off);
        let mut a = Allocator::new(Policy::default());
        let d = a.request(&World::default(), req("p", Kind::CameraPopup, None));
        assert_eq!(d, Decision::Denied(Reason::Off, None));
        assert_eq!(a.claim("p").unwrap().state, ClaimState::Denied(Reason::Off), "the denial is kept for the shell to read");
    }

    #[test]
    fn the_limits_are_the_capability_files() {
        assert_eq!(limits(0, Codec::H264), Limits { w: 4096, h: 2304, fps: 60 });
        assert_eq!(limits(0, Codec::H265), Limits { w: 4096, h: 2304, fps: 120 });
        assert_eq!(limits(1, Codec::H265), Limits { w: 1920, h: 1088, fps: 30 });
        assert_eq!(limits(1, Codec::Vp9), Limits { w: 1920, h: 1088, fps: 60 });
        assert_eq!(limits(3, Codec::Av1), Limits { w: 1920, h: 1088, fps: 30 });
        let n = Need { codec: Codec::H265, w: 1920, h: 1080, fps: 30 };
        assert!(n.fits(limits(1, Codec::H265)));
        assert!(!Need { fps: 31, ..n }.fits(limits(1, Codec::H265)) && !Need { h: 1089, ..n }.fits(limits(1, Codec::H265)));
    }

    #[test]
    fn with_no_video_on_the_screen_a_claim_gets_the_main_window_and_the_next_one_the_sub() {
        let mut a = Allocator::new(policy(Mode::Idle));
        let w = World::default();
        let d = a.request(&w, req("full", Kind::CameraFull, need(Codec::H265, 3840, 2160, 15)));
        assert_eq!(granted(&d), Some((0, vec![])));
        let d = a.request(&w, req("popup", Kind::CameraPopup, need(Codec::H264, 1280, 720, 15)));
        assert_eq!(granted(&d), Some((1, vec![])), "the first window is taken");
        assert_eq!(a.claim("popup").unwrap().limits, Some(Limits { w: 1920, h: 1088, fps: 30 }));
        let d = a.request(&w, req("pip", Kind::Pip, need(Codec::H264, 640, 360, 10)));
        assert_eq!(d, Decision::Denied(Reason::NoWindow, None), "both windows are held by claims that are not lower");
    }

    #[test]
    fn a_big_stream_that_only_the_main_window_takes_is_refused_the_sub_one_with_its_limits() {
        let mut a = Allocator::new(policy(Mode::Idle));
        // an equal claim holds main
        a.request(&World::default(), req("a", Kind::CameraPopup, need(Codec::H264, 640, 360, 15)));
        let d = a.request(&World::default(), req("b", Kind::CameraPopup, need(Codec::H265, 3840, 2160, 15)));
        assert_eq!(d, Decision::Denied(Reason::TooBig, Some(Limits { w: 1920, h: 1088, fps: 30 })));
        let c = a.claim("b").unwrap();
        assert_eq!(c.limits, Some(Limits { w: 1920, h: 1088, fps: 30 }), "what a smaller stream has to fit");
    }

    #[test]
    fn a_full_view_takes_main_from_the_wallpaper_and_a_popup_does_not_disturb_it() {
        let mut a = Allocator::new(policy(Mode::Idle));
        let w = World { wall: true, ..World::default() };
        // the wallpaper holds main: a pop-up that fits the sub window takes that and leaves the wallpaper alone
        let d = a.request(&w, req("popup", Kind::CameraPopup, need(Codec::H264, 1280, 720, 15)));
        assert_eq!(granted(&d), Some((1, vec![])));
        // a 4K full view needs main: the wallpaper is the lowest holder that has it
        let d = a.request(&w, req("full", Kind::CameraFull, need(Codec::H265, 3840, 2160, 15)));
        assert_eq!(granted(&d), Some((0, vec![Victim::Wall])));
    }

    #[test]
    fn a_higher_claim_takes_a_lower_claims_window_and_marks_it() {
        let mut a = Allocator::new(policy(Mode::Idle));
        let w = World::default();
        a.request(&w, req("pip", Kind::Pip, need(Codec::H264, 640, 360, 15)));
        a.request(&w, req("popup", Kind::CameraPopup, need(Codec::H264, 640, 360, 15)));
        let d = a.request(&w, req("full", Kind::CameraFull, need(Codec::H265, 3840, 2160, 15)));
        assert_eq!(granted(&d), Some((0, vec![Victim::Claim("pip".into())])), "the lowest holder of main goes");
        assert_eq!(a.claim("pip").unwrap().state, ClaimState::Preempted(Reason::Higher));
        assert_eq!(a.claim("popup").unwrap().state, ClaimState::Granted);
        // an equal claim does not take it
        let d = a.request(&w, req("popup2", Kind::CameraPopup, need(Codec::H264, 640, 360, 15)));
        assert_eq!(d, Decision::Denied(Reason::NoWindow, None));
    }

    #[test]
    fn idle_never_shares_the_video_plane_with_an_app_and_apps_takes_the_window_the_app_leaves() {
        let playing = World { app_elements: 1, app_playing: 1, ..World::default() };
        let mut a = Allocator::new(policy(Mode::Idle));
        assert_eq!(a.request(&playing, req("p", Kind::CameraPopup, None)), Decision::Denied(Reason::AppVideo, None));
        // an element that exists is enough in idle: the pipeline follows it within a second
        let exported = World { app_elements: 1, ..World::default() };
        assert_eq!(a.request(&exported, req("p", Kind::CameraPopup, None)), Decision::Denied(Reason::AppVideo, None));
        let mut a = Allocator::new(policy(Mode::Apps));
        assert_eq!(a.request(&exported, req("p", Kind::CameraPopup, need(Codec::H264, 1280, 720, 15))), granted_main(), "apps looks at a pipeline that plays");
        let mut a = Allocator::new(policy(Mode::Apps));
        let d = a.request(&playing, req("p", Kind::CameraPopup, need(Codec::H264, 1280, 720, 15)));
        assert_eq!(granted(&d), Some((1, vec![])), "next to the app, in the sub window");
        assert_eq!(a.request(&playing, req("big", Kind::CameraPopup, need(Codec::H265, 3840, 2160, 15))), Decision::Denied(Reason::NoWindow, None));
    }

    fn granted_main() -> Decision {
        Decision::Granted { slot: 0, limits: limits(0, Codec::H264), victims: vec![] }
    }

    #[test]
    fn a_stream_for_the_sub_window_that_is_too_large_is_refused_with_the_sub_limits() {
        let mut a = Allocator::new(policy(Mode::Apps));
        let playing = World { app_elements: 1, app_playing: 1, ..World::default() };
        let d = a.request(&playing, req("p", Kind::CameraPopup, need(Codec::H264, 2560, 1440, 15)));
        assert_eq!(d, Decision::Denied(Reason::TooBig, Some(Limits { w: 1920, h: 1088, fps: 30 })));
    }

    #[test]
    fn an_app_that_uses_both_scalers_leaves_no_window() {
        let mut a = Allocator::new(policy(Mode::Apps));
        let hfr = World { app_elements: 1, app_playing: 1, both_scalers: true, ..World::default() };
        assert_eq!(hfr.app_slots(Mode::Apps), 2);
        assert_eq!(a.request(&hfr, req("p", Kind::CameraPopup, None)), Decision::Denied(Reason::BothScalers, None));
        // the flag alone, with nothing playing, is not an app's
        let stale = World { both_scalers: true, ..World::default() };
        assert_eq!(stale.app_slots(Mode::Idle), 0);
    }

    #[test]
    fn video_another_service_has_open_counts_as_an_app_s() {
        let mut a = Allocator::new(policy(Mode::Apps));
        let w = World { other_connected: 1, ..World::default() };
        assert_eq!(w.app_slots(Mode::Apps), 1);
        let d = a.request(&w, req("p", Kind::CameraPopup, need(Codec::H264, 640, 360, 15)));
        assert_eq!(granted(&d), Some((1, vec![])));
        let mut a = Allocator::new(policy(Mode::Idle));
        assert_eq!(a.request(&w, req("p", Kind::CameraPopup, None)), Decision::Denied(Reason::AppVideo, None));
    }

    #[test]
    fn standby_denies_and_ends_what_was_granted() {
        let mut a = Allocator::new(policy(Mode::Idle));
        let on = World::default();
        a.request(&on, req("p", Kind::CameraPopup, None));
        let off = World { standby: true, ..World::default() };
        assert_eq!(a.request(&off, req("q", Kind::Pip, None)), Decision::Denied(Reason::Standby, None));
        assert_eq!(a.settle(&off), vec!["p".to_string()]);
        assert_eq!(a.claim("p").unwrap().state, ClaimState::Preempted(Reason::Standby));
    }

    #[test]
    fn an_app_that_exports_a_video_ends_the_claims_in_idle_and_only_the_overflow_in_apps() {
        let mut a = Allocator::new(policy(Mode::Idle));
        a.request(&World::default(), req("pop", Kind::CameraPopup, None));
        a.request(&World::default(), req("pip", Kind::Pip, None));
        let app = World { app_elements: 1, ..World::default() };
        let mut ended = a.settle(&app);
        ended.sort();
        assert_eq!(ended, vec!["pip".to_string(), "pop".to_string()]);
        assert_eq!(a.claim("pop").unwrap().state, ClaimState::Preempted(Reason::AppVideo));
        assert_eq!(a.settle(&app), Vec::<String>::new(), "once");

        // apps: the app plays and two claims hold a window each: the app's, then one claim, then none: the lowest claim goes
        let mut a = Allocator::new(policy(Mode::Apps));
        a.request(&World::default(), req("pop", Kind::CameraPopup, None));
        a.request(&World::default(), req("pip", Kind::Pip, None));
        let playing = World { app_elements: 1, app_playing: 1, ..World::default() };
        assert_eq!(a.settle(&playing), vec!["pip".to_string()], "the lowest claim gives its window to the app");
        assert_eq!(a.claim("pop").unwrap().state, ClaimState::Granted);
        // and a stream that uses both scalers takes both
        let hfr = World { both_scalers: true, ..playing };
        assert_eq!(a.settle(&hfr), vec!["pop".to_string()]);
        assert_eq!(a.claim("pop").unwrap().state, ClaimState::Preempted(Reason::BothScalers));
    }

    #[test]
    fn switching_off_ends_everything_granted() {
        let mut a = Allocator::new(policy(Mode::Idle));
        a.request(&World::default(), req("p", Kind::CameraPopup, None));
        a.policy.mode = Mode::Off;
        assert_eq!(a.settle(&World::default()), vec!["p".to_string()]);
        assert_eq!(a.claim("p").unwrap().state, ClaimState::Preempted(Reason::Off));
    }

    #[test]
    fn a_claim_asked_again_replaces_itself_and_does_not_take_a_window_from_itself() {
        let mut a = Allocator::new(policy(Mode::Idle));
        let w = World::default();
        a.request(&w, req("p", Kind::CameraPopup, need(Codec::H264, 640, 360, 15)));
        let d = a.request(&w, req("p", Kind::CameraPopup, need(Codec::H264, 640, 360, 15)));
        assert_eq!(granted(&d), Some((0, vec![])));
        assert_eq!(a.claims().len(), 1);
    }

    #[test]
    fn a_granted_claim_can_move_and_a_need_that_outgrows_its_window_ends_it() {
        let mut a = Allocator::new(policy(Mode::Apps));
        let playing = World { app_elements: 1, app_playing: 1, ..World::default() };
        a.request(&playing, req("p", Kind::CameraPopup, need(Codec::H264, 640, 360, 15)));
        assert!(a.update("p", Some(Rect::new(100, 100, 400, 225)), None));
        assert_eq!(a.claim("p").unwrap().rect, Rect::new(100, 100, 400, 225));
        assert!(a.update("p", None, need(Codec::H264, 1920, 1080, 30)));
        assert!(!a.update("p", None, need(Codec::H264, 3840, 2160, 15)), "the sub window does not take 4K");
        assert_eq!(a.claim("p").unwrap().state, ClaimState::Preempted(Reason::TooBig));
        assert!(!a.update("p", Some(Rect::new(0, 0, 100, 100)), None), "an ended claim is not moved");
        assert!(!a.update("nobody", None, None));
    }

    #[test]
    fn release_forgets_a_claim_and_all_of_them() {
        let mut a = Allocator::new(policy(Mode::Idle));
        a.request(&World::default(), req("a", Kind::CameraPopup, None));
        a.request(&World::default(), req("b", Kind::Pip, None));
        assert!(a.release("a") && !a.release("a"));
        assert_eq!(a.release_all(), 1);
        assert!(a.claims().is_empty());
    }

    #[test]
    fn the_occupancy_puts_apps_first_claims_where_they_were_put_and_the_wallpaper_in_what_is_left() {
        let mut a = Allocator::new(policy(Mode::Apps));
        let w = World { app_elements: 1, app_playing: 1, wall: true, ..World::default() };
        assert_eq!(a.occupancy(&w), vec![Some(Holder::App), Some(Holder::Wall)]);
        a.claims.push(Claim {
            id: "c".into(),
            kind: Kind::Pip,
            app: String::new(),
            rect: Rect::new(0, 0, 100, 100),
            need: None,
            state: ClaimState::Granted,
            slot: Some(1),
            limits: None,
            window: None,
            seq: 1,
        });
        assert_eq!(a.occupancy(&w), vec![Some(Holder::App), Some(Holder::Claim("c".into()))], "the wallpaper has no window left");
    }

    #[test]
    fn a_request_reads_from_json_and_refuses_what_it_cannot_place() {
        let ok = json!({"id": "cam", "kind": "camera-popup", "app": "org.nphil.lumatv.cameras", "rect": {"x": 1200, "y": 134, "w": 520, "h": 292}, "need": {"codec": "hevc", "w": 1920, "h": 1080, "fps": 15}});
        let r = Request::from_json(&ok).unwrap();
        assert_eq!((r.id.as_str(), r.kind, r.rect), ("cam", Kind::CameraPopup, Rect::new(1200, 134, 520, 292)));
        assert_eq!(r.need, Some(Need { codec: Codec::H265, w: 1920, h: 1080, fps: 15 }));
        let bad = |v: Value| Request::from_json(&v).unwrap_err();
        assert!(bad(json!({"kind": "pip", "rect": {"x": 0, "y": 0, "w": 100, "h": 100}})).contains("id"));
        assert!(bad(json!({"id": "a", "kind": "tv", "rect": {"x": 0, "y": 0, "w": 100, "h": 100}})).contains("kind"));
        assert!(bad(json!({"id": "a", "kind": "pip"})).contains("rect"));
        assert!(bad(json!({"id": "a", "kind": "pip", "rect": {"x": 0, "y": 0, "w": 10, "h": 100}})).contains("smaller"));
        assert!(bad(json!({"id": "a", "kind": "pip", "rect": {"x": 1900, "y": 0, "w": 100, "h": 100}})).contains("leaves"));
        assert!(bad(json!({"id": "a", "kind": "pip", "rect": {"x": -5, "y": 0, "w": 100, "h": 100}})).contains("leaves"));
        // `width` and `height` are as good as `w` and `h`
        assert_eq!(rect_of(&json!({"x": 0, "y": 0, "width": 1920, "height": 1080})), Ok(Rect::new(0, 0, 1920, 1080)));
    }

    #[test]
    fn the_modes_codecs_and_kinds_parse() {
        assert_eq!((Mode::parse("off"), Mode::parse("idle"), Mode::parse("apps"), Mode::parse("on")), (Some(Mode::Off), Some(Mode::Idle), Some(Mode::Apps), None));
        assert_eq!((Codec::parse("HEVC"), Codec::parse("avc"), Codec::parse("vp8")), (Some(Codec::H265), Some(Codec::H264), None));
        assert!(Kind::CameraFull.priority() > Kind::CameraPopup.priority() && Kind::CameraPopup.priority() > Kind::Pip.priority());
        assert!(Kind::Pip.priority() > WALL_PRIORITY);
        assert_eq!(slot_name(0), "main");
        assert_eq!(slot_name(1), "sub");
    }
}
