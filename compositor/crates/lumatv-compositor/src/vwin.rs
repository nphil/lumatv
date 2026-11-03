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
use crate::luna::{Outbound, ToBridge};
use crate::window::{WindowId, WindowType};

/// The most windows a policy may name; the display has two.
pub const MAX_SLOTS: usize = 4;
pub const DEFAULT_SLOTS: usize = 2;

/// Where `ctl vwin MODE` keeps the mode between starts, under the data directory: one word.
pub const MODE_FILE: &str = "compositor/vwin.conf";

/// The mode kept in `data_dir`, none when there is no file (or it holds something else).
pub fn read_mode(data_dir: &std::path::Path) -> Option<Mode> {
    Mode::parse(std::fs::read_to_string(data_dir.join(MODE_FILE)).ok()?.trim())
}

pub fn write_mode(data_dir: &std::path::Path, mode: Mode) -> std::io::Result<()> {
    let path = data_dir.join(MODE_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", mode.name()))
}

/// What a compositor on the set does when `--vwin` is not given: the mode `ctl vwin` kept, else `idle`, which gives the camera pop-up and the Cameras live
/// view a window only where no app has a video on the screen (the mode that never starts a decoder beside an app's).
pub fn default_mode(data_dir: &std::path::Path) -> Mode {
    read_mode(data_dir).unwrap_or(Mode::Idle)
}

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
    /// Whether a pop-up or a picture-in-picture that no hardware window can carry (an app's video holds them, a 4K120 game holds both scalers, the
    /// stream is too large for the free one) is granted a texture: the feeder decodes the camera's small stream on the CPU and the compositor draws it in
    /// the claim's rectangle (`ctl vwin texture on|off`).
    pub texture: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { mode: Mode::Off, slots: DEFAULT_SLOTS, texture: true }
    }
}

/// What a texture takes: the stream the feeder decodes on the CPU and scales into the claim's rectangle. 1280x720 at 30 a second is a camera's second
/// stream; the cost is the feeder's (`docs/research/multiview-result.md`), not a decoder's, so there is no hardware window to run out of.
pub const TEXTURE_LIMITS: Limits = Limits { w: 1280, h: 720, fps: 30 };

/// How many claims are textures at once: each is a decoder on the CPU.
pub const MAX_TEXTURES: usize = 2;

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

    /// Whether a claim of this kind may be a texture when no window can be had: a small picture over what is on the screen. The full view is the
    /// camera's own stream at the screen's size, and goes to a hardware window or to stills.
    pub fn allows_texture(self) -> bool {
        !matches!(self, Kind::CameraFull)
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
    /// The claim asked for a texture and no window (`Request::texture_only`).
    TextureOnly,
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
            Reason::TextureOnly => "texture-only",
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
            Reason::TextureOnly => "the claim asked for no window: its stream would re-time the panel",
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
    /// The caller's stream is one a hardware window would drag the panel's rate along with (a decoder at 25 fps, which the kernel follows to 100 Hz): no
    /// window is offered, only a texture, which the CPU decodes and the panel never sees.
    pub texture_only: bool,
}

impl Request {
    /// A request from the UI channel or `ctl`: `{"id", "kind", "app"?, "rect": {"x","y","w","h"}, "need"?: {"codec","w","h","fps"}, "texture_only"?: bool}`.
    pub fn from_json(v: &Value) -> Result<Request, String> {
        let id = v.get("id").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 64).ok_or("a claim needs an id")?.to_string();
        let kind = v.get("kind").and_then(Value::as_str).and_then(Kind::parse).ok_or("kind is camera-full, camera-popup or pip")?;
        let rect = rect_of(v.get("rect").ok_or("a claim needs a rect")?)?;
        let need = v.get("need").and_then(Need::from_json);
        let app = v.get("app").and_then(Value::as_str).unwrap_or("").to_string();
        let texture_only = v.get("texture_only").and_then(Value::as_bool) == Some(true);
        Ok(Request { id, kind, app, rect, need, texture_only })
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
    /// The picture is a texture (no window is held: `slot` is none and `limits` are [`TEXTURE_LIMITS`]).
    pub texture: bool,
    /// The window (`WindowId`) that plays it, once one has been bound. Kept when the claim ends, for the caller to unplace and close it, then cleared.
    pub window: Option<u32>,
    /// Its video uses the main and the sub scaler (`videooutputd` says `subScalerOccupied` for its sink): it holds two windows.
    pub wide: bool,
    pub seq: u64,
}

/// What the screen holds besides LumaTV's own claims.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct World {
    /// Exported video elements of windows that are not LumaTV's own that the idle switch counts: the ones a pipeline is attached to (an app that plays)
    /// and the idle ones of the window in front, Home aside (an app about to play). Home and the apps behind it keep elements that hold no decoder.
    pub app_elements: u32,
    /// Of those, the ones whose pipeline is attached (a punch-through context): an app that plays.
    pub app_playing: u32,
    /// Video sinks `videooutputd` reports connected that this compositor does not know (LG's own multi view, a PiP of another service).
    pub other_connected: u32,
    /// The app's video uses the main and the sub scaler (`subScalerOccupied`).
    pub both_scalers: bool,
    /// The wallpaper's decoder is attached.
    pub wall: bool,
    /// ... and its video uses both scalers.
    pub wall_wide: bool,
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
    /// No window, a texture instead (the reason the windows said no is in the journal line the caller writes).
    Texture { limits: Limits, because: Reason },
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

    pub fn claims_mut(&mut self) -> impl Iterator<Item = &mut Claim> {
        self.claims.iter_mut()
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
        for c in self.granted().filter(|c| !c.texture) {
            let at = c.slot.filter(|s| occ.get(*s).is_some_and(Option::is_none)).or_else(|| first_free(&occ));
            if let Some(i) = at {
                occ[i] = Some(Holder::Claim(c.id.clone()));
                // a video that uses both scalers holds the next window too
                if c.wide
                    && let Some(j) = first_free(&occ)
                {
                    occ[j] = Some(Holder::Claim(c.id.clone()));
                }
            }
        }
        if world.wall
            && let Some(i) = first_free(&occ)
        {
            occ[i] = Some(Holder::Wall);
            if world.wall_wide
                && let Some(j) = first_free(&occ)
            {
                occ[j] = Some(Holder::Wall);
            }
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
            texture: false,
            window: None,
            wide: false,
            seq,
        };
        let mut decision = if req.texture_only { Decision::Denied(self.gate(world).unwrap_or(Reason::TextureOnly), None) } else { self.decide(world, &req) };
        if let Decision::Denied(reason, _) = decision
            && self.texture_for(&req, reason)
        {
            decision = Decision::Texture { limits: TEXTURE_LIMITS, because: reason };
        }
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
                    }
                }
            }
            Decision::Texture { limits, .. } => {
                claim.state = ClaimState::Granted;
                claim.limits = Some(*limits);
                claim.texture = true;
            }
            Decision::Denied(reason, limits) => {
                claim.state = ClaimState::Denied(*reason);
                claim.limits = *limits;
            }
        }
        self.claims.push(claim);
        decision
    }

    /// Whether a request the windows refused for `reason` is a texture: the switch is on (and the mode is not `off`, which grants nothing), the kind may,
    /// the screen is on, and there is a decoder to spare.
    fn texture_for(&self, req: &Request, reason: Reason) -> bool {
        self.policy.texture
            && req.kind.allows_texture()
            && !matches!(reason, Reason::Standby | Reason::Off)
            && self.claims.iter().filter(|c| c.texture && c.state == ClaimState::Granted && c.id != req.id).count() < MAX_TEXTURES
    }

    fn decide(&self, world: &World, req: &Request) -> Decision {
        if let Some(reason) = self.gate(world) {
            return Decision::Denied(reason, None);
        }
        let occ = self.occupancy(world);
        let prio = req.kind.priority();
        let fits = |slot: usize| req.need.is_none_or(|n| n.fits(limits(slot, n.codec)));
        let cap = |slot: usize| limits(slot, req.need.map_or(Codec::H264, |n| n.codec));
        // LumaTV's own sessions of the decoder hold both scalers (`subScalerOccupied` reads true once one plays: TV 2026-10-29), so a second one does not
        // start beside the wallpaper's: the claim takes the wallpaper's window and the wallpaper's session is closed
        if let Some(slot) = occ.iter().position(|h| *h == Some(Holder::Wall))
            && fits(slot)
        {
            return Decision::Granted { slot, limits: cap(slot), victims: vec![Victim::Wall] };
        }
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
        // a texture holds no window: the screen's video does not end it; standby, `off` and its own switch do
        let texture_stop = if world.standby {
            Some(Reason::Standby)
        } else if self.policy.mode == Mode::Off || !self.policy.texture {
            Some(Reason::Off)
        } else {
            None
        };
        if let Some(reason) = texture_stop {
            for c in self.claims.iter_mut().filter(|c| c.texture && c.state == ClaimState::Granted) {
                c.state = ClaimState::Preempted(reason);
                ended.push(c.id.clone());
            }
        }
        let gate = self.gate(world);
        let mut order: Vec<usize> = (0..self.claims.len()).filter(|i| self.claims[*i].state == ClaimState::Granted && !self.claims[*i].texture).collect();
        if let Some(reason) = gate {
            for i in order {
                self.claims[i].state = ClaimState::Preempted(reason);
                ended.push(self.claims[i].id.clone());
            }
            return ended;
        }
        // more holders than windows: the lowest claims give theirs up. The wallpaper is not counted: a claim outranks it, and the grant that took its window
        // had it closed (the decoder is still there for the moment its process takes to go).
        let apps = world.app_slots(self.policy.mode);
        let width = |c: &Claim| 1 + usize::from(c.wide);
        let mut held = apps + order.iter().map(|i| width(&self.claims[*i])).sum::<usize>();
        order.sort_by_key(|i| (self.claims[*i].kind.priority(), std::cmp::Reverse(self.claims[*i].seq)));
        for i in order {
            if held <= self.policy.slots {
                break;
            }
            held -= width(&self.claims[i]);
            self.claims[i].state = ClaimState::Preempted(Reason::NoWindow);
            ended.push(self.claims[i].id.clone());
        }
        ended
    }

    /// A claim's own record for the topic and `ctl`.
    pub fn claim_json(c: &Claim, placed: bool, sink: Option<&str>) -> Value {
        // (`wide`: it holds the sub window as well)
        json!({
            "id": c.id,
            "kind": c.kind.name(),
            "app": c.app,
            "state": c.state.word(),
            "reason": c.state.reason().map_or("", Reason::word),
            "text": c.state.reason().map_or("", Reason::text),
            "slot": if c.texture { Some("texture") } else { c.slot.map(slot_name) },
            "texture": c.texture,
            "limits": c.limits.map(|l| json!({"w": l.w, "h": l.h, "fps": l.fps})),
            "rect": {"x": c.rect.x, "y": c.rect.y, "w": c.rect.w, "h": c.rect.h},
            "need": c.need.map(|n| json!({"codec": n.codec.name(), "w": n.w, "h": n.h, "fps": n.fps})),
            "window": c.window,
            "wide": c.wide,
            "placed": placed,
            "sink": sink,
        })
    }
}

// ---- the compositor's side: the sinks `videooutputd` reports, the windows the claims are played in, the shell's and `ctl`'s requests ----------

/// One video sink of `videooutputd`'s status (`com.webos.service.videooutput/getStatus`, read on the set: `video` lists `MAIN` at z 0 and `SUB0` at z 1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sink {
    pub name: String,
    pub connected: bool,
    pub context: String,
    pub app: String,
    pub z: i64,
    /// `videoInfo.subScalerOccupied`: this video uses the main and the sub scaler.
    pub occupied: bool,
    pub width: u64,
    pub height: u64,
    pub fps: f64,
    /// `displayOutput`: where videooutputd shows this video, in the panel's pixels (x, y, width, height): the window it was asked for, scaled by the panel's ratio
    /// and clamped to the panel. A window asked for in the panel's own pixels shows here at twice its size, off the panel for one in a corner: 3840,988 0x1168 for
    /// a claim at 1316,247 520x292 (TV 2026-10-30, the storm of "placed again").
    pub dest: (i64, i64, i64, i64),
    /// `fullScreen`: the video was placed full screen. Its `displayOutput` may be what the sink held before (a window the claim had), not the panel.
    pub full_screen: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sinks {
    pub video: Vec<Sink>,
}

impl Sinks {
    /// The `video` list of a status answer; `None` for an answer that has none.
    pub fn parse(payload: &Value) -> Option<Sinks> {
        let list = payload.get("video")?.as_array()?;
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Some(Sinks {
            video: list
                .iter()
                .map(|v| Sink {
                    name: text(v, "sink"),
                    connected: v.get("connected").and_then(Value::as_bool).unwrap_or(false),
                    context: text(v, "context"),
                    app: text(v, "appId"),
                    z: v.get("zOrder").and_then(Value::as_i64).unwrap_or(0),
                    occupied: v.pointer("/videoInfo/subScalerOccupied").and_then(Value::as_bool).unwrap_or(false),
                    width: v.get("width").and_then(Value::as_u64).unwrap_or(0),
                    height: v.get("height").and_then(Value::as_u64).unwrap_or(0),
                    fps: v.get("frameRate").and_then(Value::as_f64).unwrap_or(0.0),
                    dest: {
                        let n = |k: &str| v.pointer(&format!("/displayOutput/{k}")).and_then(Value::as_i64).unwrap_or(0);
                        (n("x"), n("y"), n("width"), n("height"))
                    },
                    full_screen: v.get("fullScreen").and_then(Value::as_bool).unwrap_or(false),
                })
                .collect(),
        })
    }

    /// The connected sink one of `contexts` is on.
    pub fn of(&self, contexts: &[String]) -> Option<&Sink> {
        self.video.iter().find(|s| s.connected && contexts.contains(&s.context))
    }

    pub fn json(&self) -> Value {
        Value::Array(
            self.video
                .iter()
                .map(|s| {
                    json!({"sink": s.name, "connected": s.connected, "z": s.z, "app": s.app, "occupiesBoth": s.occupied, "size": [s.width, s.height], "fps": s.fps,
                           "displayOutput": [s.dest.0, s.dest.1, s.dest.2, s.dest.3], "fullScreen": s.full_screen, "context": s.context})
                })
                .collect(),
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Of the grants, the ones that were a texture.
    pub textures: u64,
    pub granted: u64,
    pub denied: u64,
    pub ended: u64,
    pub released: u64,
    pub windows_closed: u64,
}

/// What the compositor keeps beside the allocator.
pub struct Vwin {
    pub alloc: Allocator,
    pub sinks: Sinks,
    /// The token of the status subscription, once it is made.
    pub status_token: Option<u64>,
    last_topic: Value,
    pub stats: Stats,
    /// The claims whose video is on a connected sink somewhere else than they say: since when (so that the sink's answer is read a moment after the
    /// placement and the connection, not at the first frame), how often the placement was made again, and for which rectangle.
    watch: std::collections::BTreeMap<String, Watch>,
    /// When the placement of each of LumaTV's own video windows' elements last went out (monotonic nanoseconds): what the sink says before it is not an answer to it.
    pub placed_at: std::collections::BTreeMap<crate::surface::SurfaceId, u64>,
    /// The contexts of LumaTV's own video elements that went (with the monotonic nanosecond they are forgotten at): videooutputd lists a sink for a moment
    /// after its element is destroyed, and a video of ours that is still going is no app's.
    gone: Vec<(String, u64)>,
    /// How long after a placement, and after its video was first seen on a sink, the sink's answer is believed (nanoseconds); a test makes it nothing.
    pub settle_ns: u64,
}

impl Vwin {
    pub fn new(policy: Policy) -> Vwin {
        Vwin {
            alloc: Allocator::new(policy),
            sinks: Sinks::default(),
            status_token: None,
            last_topic: Value::Null,
            stats: Stats::default(),
            watch: std::collections::BTreeMap::new(),
            placed_at: std::collections::BTreeMap::new(),
            gone: Vec::new(),
            settle_ns: SETTLE_NS,
        }
    }
}

/// A video window nobody has claimed for this long is closed: whoever started it did not ask first (or the claim was refused and it started anyway).
const NEVER_BOUND_NS: u64 = 3_000_000_000;
/// How long the context of a video element of ours that went stays ours in the sinks' list.
const GONE_NS: u64 = 5_000_000_000;
/// The size of the window a feeder opens a video window with: it draws nothing until a texture claim makes it larger.
pub const TEXTURE_WINDOW_PX: i32 = 16;
/// How many times a claim's video is placed again where videooutputd held it elsewhere, and how long after a placement its sink is believed. A placement that
/// is lost is one lost once; a sink that stays where it is after two is not going to move for a third. videooutputd applies a placement within tens of
/// milliseconds once the pipeline has said what the video is (0.5 s after the attach on LG's own captures), and its status follows.
const REPLACE_MAX: u32 = 2;
const SETTLE_NS: u64 = 700_000_000;

/// What is known of one claim whose video is not where it says.
#[derive(Clone, Copy, Debug, Default)]
struct Watch {
    /// When it was first seen so, or last placed again (monotonic nanoseconds).
    since: u64,
    /// How often it was placed again for `want`.
    tries: u32,
    /// The rectangle (the panel's pixels) the claim wants; a claim that moves starts over.
    want: Rect,
    /// Whether the giving up was said in the journal.
    said: bool,
}
const STATUS_URI: &str = "luna://com.webos.service.videooutput/getStatus";

impl crate::State {
    /// What the screen holds besides the claims: the exported elements by the kind of window that owns them, and what `videooutputd` says.
    pub fn vwin_world(&self) -> World {
        let mut w = World { standby: !self.luna_state.power.rendering, ..World::default() };
        let mut known: Vec<&str> = Vec::new();
        let mut ours: Vec<&str> = Vec::new();
        let mut wall: Vec<&str> = Vec::new();
        let now = crate::output::monotonic_ns();
        for (context, until) in &self.vwin.gone {
            if *until > now {
                known.push(context);
                ours.push(context);
            }
        }
        let front = self.scene.foreground(&self.surfaces).map(|w| w.id);
        for e in self.foreign.exported.values() {
            known.extend(e.contexts.iter().map(String::as_str));
            let window = self.scene.window_of(e.surface).and_then(|id| self.scene.window(id));
            match window.map(|w| w.kind) {
                Some(WindowType::VideoWindow) => ours.extend(e.contexts.iter().map(String::as_str)),
                Some(WindowType::Wallpaper) => {
                    ours.extend(e.contexts.iter().map(String::as_str));
                    if !e.contexts.is_empty() {
                        w.wall = true;
                        wall.extend(e.contexts.iter().map(String::as_str));
                    }
                }
                _ => {
                    let playing = !e.contexts.is_empty();
                    if self.vwin_element_counts(window, front, playing) {
                        w.app_elements += 1;
                    }
                    if playing {
                        w.app_playing += 1;
                    }
                }
            }
        }
        let sinks = &self.vwin.sinks.video;
        w.other_connected = sinks.iter().filter(|s| s.connected && !known.contains(&s.context.as_str())).count() as u32;
        w.both_scalers = sinks.iter().any(|s| s.connected && s.occupied && !ours.contains(&s.context.as_str()));
        w.wall_wide = sinks.iter().any(|s| s.connected && s.occupied && wall.contains(&s.context.as_str()));
        w
    }

    /// Whether an element that is no part of LumaTV's own video counts against the idle switch: it has a decoder, or its window is the one in front and
    /// is no Home (LG's Home and the apps behind the front one keep elements that nothing plays on, TV 2026-10-29: an element nobody played on denied
    /// every claim over Home).
    fn vwin_element_counts(&self, window: Option<&crate::window::Window>, front: Option<WindowId>, playing: bool) -> bool {
        playing || window.is_some_and(|w| Some(w.id) == front && w.app_id() != crate::scene::HOME_APP)
    }

    /// The exported elements as the switch sees them, for `ctl vwin`: which window owns each, whether a decoder is attached, whether it is the window
    /// in front and whether it counts against the idle switch.
    fn vwin_elements_json(&self) -> Value {
        let front = self.scene.foreground(&self.surfaces).map(|w| w.id);
        Value::Array(
            self.foreign
                .exported
                .values()
                .map(|e| {
                    let window = self.scene.window_of(e.surface).and_then(|id| self.scene.window(id));
                    let playing = !e.contexts.is_empty();
                    let ours = window.is_some_and(|w| matches!(w.kind, WindowType::VideoWindow | WindowType::Wallpaper));
                    json!({
                        "window": e.window_id,
                        "app": window.map_or("", |w| w.app_id()),
                        "kind": window.map_or_else(|| "none".to_string(), |w| format!("{:?}", w.kind)),
                        "playing": playing,
                        "front": window.is_some_and(|w| Some(w.id) == front),
                        "counted": !ours && self.vwin_element_counts(window, front, playing),
                    })
                })
                .collect(),
        )
    }

    /// An element of LumaTV's own video went: its sink may be listed a moment longer, and it is still ours until videooutputd lets it go.
    pub(crate) fn vwin_note_gone(&mut self, contexts: &[String]) {
        let now = crate::output::monotonic_ns();
        self.vwin.gone.retain(|(_, until)| *until > now);
        self.vwin.gone.extend(contexts.iter().map(|c| (c.clone(), now + GONE_NS)));
    }

    /// The contexts the elements of a window carry.
    fn vwin_contexts(&self, window: WindowId) -> Vec<String> {
        let Some(surface) = self.scene.window(window).map(|w| w.surface) else { return Vec::new() };
        self.foreign.exported.values().filter(|e| e.surface == surface).flat_map(|e| e.contexts.iter().cloned()).collect()
    }

    /// The sink a claim's video is on, read back from `videooutputd`'s status: the proof of which window LG's resource manager gave it.
    fn vwin_sink_of(&self, c: &Claim) -> Option<String> {
        let w = c.window.map(WindowId)?;
        if c.texture {
            return self.vwin_placed(c).then(|| "texture".to_string());
        }
        self.vwin.sinks.of(&self.vwin_contexts(w)).map(|s| s.name.clone())
    }

    /// Whether a claim's picture is where the claim says: a window's video is placed once its window is live and mapped; a texture is placed once the feeder
    /// has painted its window at a size of its own (the window it opens with is `TEXTURE_WINDOW_PX` square and draws nothing).
    fn vwin_placed(&self, c: &Claim) -> bool {
        let Some(w) = c.window.and_then(|w| self.scene.window(WindowId(w))) else { return false };
        let live = w.vwin_live && w.map_serial > 0;
        if !c.texture {
            return live;
        }
        live && self.surfaces.get(w.surface).is_some_and(|s| s.size().0 > TEXTURE_WINDOW_PX)
    }

    /// Brings the windows and the claims into step: the claims the screen no longer allows end, each claim is bound to the video window that plays it
    /// (by app id; an empty one takes the first window without a claim), that window is drawn with its hole and placement at the claim's rectangle, a window
    /// without a claim is not, and one nobody ever claimed is closed. Called wherever the world or a claim changed and at each tick; with the switch off
    /// and nothing to look after it returns at once.
    pub fn vwin_sync(&mut self) {
        let has_windows = self.scene.windows().any(|w| w.kind == WindowType::VideoWindow);
        if self.vwin.alloc.policy.mode == Mode::Off && self.vwin.alloc.claims().is_empty() && !has_windows {
            return;
        }
        // a claim's video that uses both scalers holds two windows (the status says)
        let wides: Vec<(String, bool)> = self
            .vwin
            .alloc
            .granted()
            .filter_map(|c| {
                let w = c.window.map(WindowId)?;
                Some((c.id.clone(), self.vwin.sinks.of(&self.vwin_contexts(w)).is_some_and(|s| s.occupied)))
            })
            .collect();
        for (id, wide) in wides {
            if let Some(c) = self.vwin.alloc.claim_mut(&id) {
                c.wide = wide;
            }
        }
        let world = self.vwin_world();
        for id in self.vwin.alloc.settle(&world) {
            self.vwin_end(&id, "the screen no longer allows it");
        }
        self.vwin_bind();
        self.vwin_close_unclaimed();
        self.vwin_replace_parked();
        let topic = self.vwin_topic();
        if topic != self.vwin.last_topic {
            self.vwin.last_topic = topic;
            self.shell_sync_topics();
        }
    }

    /// A claim has ended (it is marked): its window stops being drawn and placed, and is closed so that the video it plays ends and the decoder is free
    /// for whoever took the window.
    fn vwin_end(&mut self, id: &str, why: &str) {
        let Some(c) = self.vwin.alloc.claim_mut(id) else { return };
        let (window, reason) = (c.window.take(), c.state.reason().map_or("", Reason::word));
        self.vwin.stats.ended += 1;
        self.journal(format!("vwin: claim {id} ended ({reason}): {why}"));
        if let Some(w) = window {
            let wid = WindowId(w);
            self.vwin_unplace(wid);
            self.vwin_close(wid, &format!("claim {id} ended"));
        }
    }

    /// The window is not drawn, and its elements are hidden (a placement of 0x0); it stays, for the next claim.
    fn vwin_unplace(&mut self, wid: WindowId) {
        let Some(w) = self.scene.window_mut(wid) else { return };
        let was = std::mem::replace(&mut w.vwin_live, false);
        w.vwin_texture = false;
        let surface = w.surface;
        if was {
            self.vwin_elements_changed(surface);
        }
    }

    fn vwin_elements_changed(&mut self, surface: crate::surface::SurfaceId) {
        let ids: Vec<String> = self.foreign.exported.values().filter(|e| e.surface == surface).map(|e| e.window_id.clone()).collect();
        for id in ids {
            self.exported_changed(&id);
        }
    }

    /// Asks the window to close (`wl_webos_shell_surface.close`): the process that owns it ends its video and goes.
    fn vwin_close(&mut self, wid: WindowId, why: &str) {
        if let Some(w) = self.scene.window_mut(wid) {
            w.vwin_closed = true;
        }
        if let Some(res) = self.webos_shell_resource(wid) {
            res.close();
            self.vwin.stats.windows_closed += 1;
            self.journal(format!("vwin: close sent to window {} ({why})", wid.0));
        }
    }

    /// The wallpaper's window is taken by a more important video: the same close `wall_handover` sends when Home leaves the screen.
    fn vwin_close_wall(&mut self) {
        let ids: Vec<WindowId> = self.scene.windows().filter(|w| w.kind == WindowType::Wallpaper && w.map_serial > 0).map(|w| w.id).collect();
        for id in ids {
            if let Some(res) = self.webos_shell_resource(id) {
                res.close();
                self.journal(format!("vwin: close sent to the wallpaper's window {} (a video of a claim takes its window)", id.0));
            }
        }
    }

    fn vwin_bind(&mut self) {
        let alive: Vec<u32> = self.scene.windows().map(|w| w.id.0).collect();
        for c in self.vwin.alloc.claims_mut() {
            if c.window.is_some_and(|w| !alive.contains(&w)) {
                c.window = None;
            }
        }
        let windows: Vec<(WindowId, crate::surface::SurfaceId, String)> =
            self.scene.windows().filter(|w| w.kind == WindowType::VideoWindow && !w.vwin_closed).map(|w| (w.id, w.surface, w.app_id().to_string())).collect();
        for (wid, surface, app) in windows {
            let claim = self
                .vwin
                .alloc
                .granted()
                .find(|c| c.window == Some(wid.0) || (c.window.is_none() && (c.app.is_empty() || c.app == app)))
                .map(|c| (c.id.clone(), c.rect, c.texture));
            let (live, rect, texture) = claim.as_ref().map_or((false, crate::Rect::default(), false), |(_, r, t)| (true, *r, *t));
            if let Some((id, ..)) = &claim
                && let Some(c) = self.vwin.alloc.claim_mut(id)
            {
                c.window = Some(wid.0);
            }
            let Some(w) = self.scene.window_mut(wid) else { continue };
            let changed = w.vwin_live != live || (live && w.vwin_rect != rect) || w.vwin_texture != texture;
            w.vwin_live = live;
            w.vwin_rect = rect;
            w.vwin_texture = texture;
            w.vwin_bound |= live;
            if changed {
                if let Some((id, ..)) = &claim {
                    self.journal(format!("vwin: window {} plays claim {id} at {},{} {}x{}", wid.0, rect.x, rect.y, rect.w, rect.h));
                }
                self.vwin_elements_changed(surface);
            }
        }
    }

    fn vwin_close_unclaimed(&mut self) {
        let now = crate::output::monotonic_ns();
        let stale: Vec<WindowId> = self
            .scene
            .windows()
            .filter(|w| w.kind == WindowType::VideoWindow && w.map_serial > 0 && !w.vwin_bound && !w.vwin_closed && !w.vwin_live)
            .map(|w| w.id)
            .collect();
        for id in stale {
            let seen = self.scene.window(id).map_or(0, |w| w.vwin_seen_ns);
            if seen == 0 {
                if let Some(w) = self.scene.window_mut(id) {
                    w.vwin_seen_ns = now;
                }
            } else if now.saturating_sub(seen) > NEVER_BOUND_NS {
                self.vwin_close(id, "no claim was granted for it");
            }
        }
    }

    /// The claims and the windows as the shell reads them (topic `vwin`).
    pub fn vwin_topic(&self) -> Value {
        let world = self.vwin_world();
        let occupied = self.vwin.alloc.occupancy(&world);
        let claims: Vec<Value> = self
            .vwin
            .alloc
            .claims()
            .iter()
            .map(|c| Allocator::claim_json(c, self.vwin_placed(c), self.vwin_sink_of(c).as_deref()))
            .collect();
        json!({
            "mode": self.vwin.alloc.policy.mode.name(),
            "texture": self.vwin.alloc.policy.texture,
            "slots": self.vwin.alloc.policy.slots,
            "free": occupied.iter().filter(|o| o.is_none()).count(),
            "apps": world.app_slots(self.vwin.alloc.policy.mode),
            // apps with a video playing (a decoder attached), whatever the mode counts: the Cameras full view shares the screen with one and never takes its window
            "appsPlaying": world.app_playing,
            "wall": world.wall,
            "claims": claims,
        })
    }

    /// `ctl vwin`: the topic and what is behind it.
    pub fn vwin_report(&self) -> Value {
        let world = self.vwin_world();
        let mut r = self.vwin_topic();
        r["world"] = json!({
            "appElements": world.app_elements, "appPlaying": world.app_playing, "otherConnected": world.other_connected, "bothScalers": world.both_scalers,
            "wall": world.wall, "wallBoth": world.wall_wide, "standby": world.standby,
        });
        r["sinks"] = self.vwin.sinks.json();
        // the claims whose video was placed again where videooutputd held it elsewhere, and how often
        r["replaced"] = Value::Object(self.vwin.watch.iter().filter(|(_, w)| w.tries > 0).map(|(id, w)| (id.clone(), json!(w.tries))).collect());
        r["elements"] = self.vwin_elements_json();
        r["subscribed"] = self.vwin.status_token.is_some().into();
        r["windows"] = Value::Array(
            self.scene
                .windows()
                .filter(|w| w.kind == WindowType::VideoWindow)
                .map(|w| {
                    json!({"id": w.id.0, "app": w.app_id(), "mapped": w.map_serial > 0, "live": w.vwin_live, "bound": w.vwin_bound, "closed": w.vwin_closed,
                           "rect": [w.vwin_rect.x, w.vwin_rect.y, w.vwin_rect.w, w.vwin_rect.h]})
                })
                .collect(),
        );
        r["stats"] = json!({
            "granted": self.vwin.stats.granted, "denied": self.vwin.stats.denied, "ended": self.vwin.stats.ended, "released": self.vwin.stats.released,
            "windowsClosed": self.vwin.stats.windows_closed, "textures": self.vwin.stats.textures,
        });
        r["limits"] = json!({
            "main": {"h264": [4096, 2304, 60], "h265": [4096, 2304, 120]},
            "sub": {"h264": [1920, 1088, 30], "h265": [1920, 1088, 30], "vp9": [1920, 1088, 60]},
            "source": "device_codec_capability_config.json, multiview_sub",
        });
        r
    }

    fn vwin_claim_json(&self, id: &str) -> Value {
        self.vwin.alloc.claim(id).map_or(Value::Null, |c| Allocator::claim_json(c, self.vwin_placed(c), self.vwin_sink_of(c).as_deref()))
    }

    /// One request of the shell (`POST /v1/vwin`) or of `ctl vwin`: `{"op": "claim", ...}`, `move`, `release`, `releaseAll`. The answer is the claim as it is
    /// now; the shell reads the same in the topic.
    pub fn vwin_command(&mut self, body: &Value) -> Value {
        let op = body.get("op").and_then(Value::as_str).unwrap_or("");
        let id = body.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        match op {
            "claim" => match Request::from_json(body) {
                Ok(req) => {
                    let world = self.vwin_world();
                    let id = req.id.clone();
                    let (kind, rect) = (req.kind, req.rect);
                    let before = self.vwin.alloc.claim(&id).and_then(|c| c.window.map(|w| (w, c.texture, c.slot)));
                    let decision = self.vwin.alloc.request(&world, req);
                    // a claim asked again for another kind of window (a sub window for a texture, Home having come to the front): the window it was
                    // bound to is the old stream's and is not drawn for the new one
                    if let Some((w, texture, slot)) = before
                        && self.vwin.alloc.claim(&id).is_some_and(|c| (c.texture, c.slot) != (texture, slot))
                    {
                        self.vwin_unplace(WindowId(w));
                    }
                    match decision {
                        Decision::Granted { slot, limits, victims } => {
                            self.vwin.stats.granted += 1;
                            self.journal(format!(
                                "vwin: claim {id} ({}) granted the {} window (up to {}x{} at {} a second) at {},{} {}x{}",
                                kind.name(),
                                slot_name(slot),
                                limits.w,
                                limits.h,
                                limits.fps,
                                rect.x,
                                rect.y,
                                rect.w,
                                rect.h
                            ));
                            for v in victims {
                                match v {
                                    Victim::Wall => self.vwin_close_wall(),
                                    Victim::Claim(other) => self.vwin_end(&other, &format!("claim {id} is more important")),
                                }
                            }
                        }
                        Decision::Texture { limits, because } => {
                            self.vwin.stats.granted += 1;
                            self.vwin.stats.textures += 1;
                            self.journal(format!(
                                "vwin: claim {id} ({}) granted a texture (no window: {}) up to {}x{} at {} a second at {},{} {}x{}",
                                kind.name(),
                                because.word(),
                                limits.w,
                                limits.h,
                                limits.fps,
                                rect.x,
                                rect.y,
                                rect.w,
                                rect.h
                            ));
                        }
                        Decision::Denied(reason, _) => {
                            self.vwin.stats.denied += 1;
                            self.journal(format!("vwin: claim {id} ({}) denied ({}): {}", kind.name(), reason.word(), reason.text()));
                        }
                    }
                    self.vwin_sync();
                    json!({"ok": true, "claim": self.vwin_claim_json(&id)})
                }
                Err(e) => json!({"ok": false, "error": e}),
            },
            "move" => {
                let rect = body.get("rect").map(rect_of).transpose();
                let need = body.get("need").and_then(Need::from_json);
                match rect {
                    Err(e) => json!({"ok": false, "error": e}),
                    Ok(rect) => {
                        if self.vwin.alloc.claim(&id).is_none() {
                            json!({"ok": false, "error": format!("no claim {id}")})
                        } else {
                            if !self.vwin.alloc.update(&id, rect, need) && self.vwin.alloc.claim(&id).is_some_and(|c| c.window.is_some()) {
                                self.vwin_end(&id, "the stream no longer fits its window");
                            }
                            self.vwin_sync();
                            json!({"ok": true, "claim": self.vwin_claim_json(&id)})
                        }
                    }
                }
            }
            "release" => {
                let window = self.vwin.alloc.claim(&id).and_then(|c| c.window);
                let had = self.vwin.alloc.release(&id);
                if had {
                    self.vwin.stats.released += 1;
                    self.journal(format!("vwin: claim {id} released"));
                }
                if let Some(w) = window {
                    self.vwin_unplace(WindowId(w));
                }
                self.vwin_sync();
                json!({"ok": true, "released": had})
            }
            "releaseAll" => {
                let windows: Vec<u32> = self.vwin.alloc.claims().iter().filter_map(|c| c.window).collect();
                let n = self.vwin.alloc.release_all();
                self.vwin.stats.released += n as u64;
                for w in windows {
                    self.vwin_unplace(WindowId(w));
                }
                self.vwin_sync();
                json!({"ok": true, "released": n})
            }
            _ => json!({"ok": false, "error": "op is claim, move, release or releaseAll"}),
        }
    }

    /// The switch (`ctl vwin off|idle|apps`, the config at start): every claim of a mode that does not hold ends, and the status of the sinks is read
    /// while the switch is on.
    pub fn vwin_set_mode(&mut self, mode: Mode) {
        let was = self.vwin.alloc.policy.mode;
        self.vwin.alloc.policy.mode = mode;
        self.config.vwin.mode = mode;
        if was != mode {
            self.journal(format!("vwin: {} (was {})", mode.name(), was.name()));
        }
        if mode == Mode::Off {
            if let Some(token) = self.vwin.status_token.take() {
                self.luna.send(ToBridge::Cancel { token });
            }
            self.vwin.sinks = Sinks::default();
        } else {
            self.vwin_subscribe();
        }
        self.vwin_sync();
    }

    /// Subscribes to `videooutputd`'s status once (the bridge must be up): the sinks and which of them are connected and use both scalers.
    pub fn vwin_subscribe(&mut self) {
        if self.vwin.status_token.is_some() || self.vwin.alloc.policy.mode == Mode::Off || !self.luna.is_connected() {
            return;
        }
        let token = self.luna_token();
        self.vwin.status_token = Some(token);
        self.luna.send(ToBridge::Outbound(Outbound { token, uri: STATUS_URI.into(), payload: json!({"subscribe": true}), subscribe: true }));
    }

    /// A push of the status: the sinks as they are, and the claims judged against them.
    pub fn vwin_status(&mut self, payload: &Value) {
        if let Some(sinks) = Sinks::parse(payload)
            && sinks != self.vwin.sinks
        {
            self.vwin.sinks = sinks;
            self.vwin_sync();
        }
    }

    /// A video that is connected but that videooutputd holds somewhere else than its claim says: the placement is made again, at most [`REPLACE_MAX`] times for a
    /// claim and a rectangle, and then left as it is, with one line of the journal saying so. What the sink holds is in the panel's pixels (the claim's
    /// rectangle scaled by the panel's ratio); a claim that covers the output is placed full screen, and a sink that says `fullScreen` is where it is meant to
    /// be whatever rectangle it held before (TV 2026-10-30: a camera pop-up's video, then the full view's on the same sink, was "held" at the pop-up's doubled
    /// rectangle and placed again twelve times). The sink's answer is believed `settle_ns` after the video was last placed and first seen on it: this runs at
    /// every frame and the answer may be older than the placement. A claim whose video is where it says is let go of, and one that moved starts over.
    fn vwin_replace_parked(&mut self) {
        let now = crate::output::monotonic_ns();
        // a claim, what its video's sink holds and whether that is where it is meant to be, and when its video was last placed
        struct Look {
            id: String,
            surface: crate::surface::SurfaceId,
            want: Rect,
            held: (i64, i64, i64, i64),
            full_screen: bool,
            there: bool,
            placed_at: u64,
        }
        let mut looks: Vec<Look> = Vec::new();
        for c in self.vwin.alloc.granted() {
            let Some(wid) = c.window.map(WindowId) else { continue };
            let Some(window) = self.scene.window(wid).filter(|w| w.vwin_live) else { continue };
            let Some(sink) = self.vwin.sinks.of(&self.vwin_contexts(wid)) else { continue };
            let want = self.panel_rect(&c.rect);
            let near = |held: i64, wanted: i32| (held - i64::from(wanted)).abs() <= 2;
            let there = near(sink.dest.0, want.x) && near(sink.dest.1, want.y) && near(sink.dest.2, want.w) && near(sink.dest.3, want.h);
            let there = there || (sink.full_screen && self.covers_output(&c.rect));
            let placed_at = self.vwin.placed_at.get(&window.surface).copied().unwrap_or(0);
            looks.push(Look { id: c.id.clone(), surface: window.surface, want, held: sink.dest, full_screen: sink.full_screen, there, placed_at });
        }
        // a claim that is gone, or whose video is on no sink, leaves nothing of its count behind
        self.vwin.watch.retain(|id, _| looks.iter().any(|l| &l.id == id));
        enum Do {
            Wait,
            Say,
            Again(u32),
        }
        for look in looks {
            if look.there {
                self.vwin.watch.remove(&look.id);
                continue;
            }
            let settle = self.vwin.settle_ns;
            let entry = self.vwin.watch.entry(look.id.clone()).or_default();
            if entry.since == 0 || entry.want != look.want {
                *entry = Watch { since: now, want: look.want, ..Watch::default() };
            }
            let act = if now.saturating_sub(entry.since.max(look.placed_at)) < settle {
                Do::Wait
            } else if entry.tries >= REPLACE_MAX {
                if std::mem::replace(&mut entry.said, true) { Do::Wait } else { Do::Say }
            } else {
                entry.tries += 1;
                entry.since = now;
                Do::Again(entry.tries)
            };
            let (id, held, want) = (&look.id, look.held, look.want);
            match act {
                Do::Wait => {}
                Do::Say => self.journal(format!(
                    "vwin: claim {id}'s video is held at {},{} {}x{}{}, not {},{} {}x{}: placed again {REPLACE_MAX} times without effect, left as it is",
                    held.0,
                    held.1,
                    held.2,
                    held.3,
                    if look.full_screen { " (full screen)" } else { "" },
                    want.x,
                    want.y,
                    want.w,
                    want.h
                )),
                Do::Again(tries) => {
                    self.journal(format!(
                        "vwin: claim {id}'s video is held at {},{} {}x{}, not {},{} {}x{}: placed again ({tries})",
                        held.0, held.1, held.2, held.3, want.x, want.y, want.w, want.h
                    ));
                    self.vwin_elements_changed(look.surface);
                }
            }
        }
    }

    /// `ctl vwin ...` as words: no word reads; `off|idle|apps` switches; `texture on|off`; `slots N`; `claim ID KIND X Y W H [APP] [CODEC] [WxH@FPS]`; `move ID X Y W H`;
    /// `release ID|all`.
    pub fn vwin_ctl(&mut self, words: &[&str]) -> Value {
        let ask = |s: &mut Self, v: Value| {
            let r = s.vwin_command(&v);
            // a frame for it now, on a still screen, as the shell's own requests get
            if s.output.draws_on_damage(std::time::Instant::now()) {
                s.tick();
            }
            r
        };
        let num = |w: &[&str], i: usize| w.get(i).and_then(|t| t.parse::<i64>().ok());
        match words {
            [] | ["status"] => json!({"ok": true, "vwin": self.vwin_report()}),
            [m @ ("off" | "idle" | "apps")] => {
                let mode = Mode::parse(m).expect("matched");
                self.vwin_set_mode(mode);
                // kept for the next start, as the Rust shell's kinds are
                if let Err(e) = write_mode(&self.config.data_dir, mode) {
                    self.journal(format!("vwin: the mode was not kept for the next start: {e}"));
                }
                json!({"ok": true, "vwin": self.vwin_report()})
            }
            ["texture", v @ ("on" | "off")] => {
                let on = *v == "on";
                self.vwin.alloc.policy.texture = on;
                self.config.vwin.texture = on;
                self.journal(format!("vwin: textures {}", if on { "on" } else { "off" }));
                self.vwin_sync();
                json!({"ok": true, "vwin": self.vwin_report()})
            }
            ["slots", n] => match n.parse::<usize>().ok().filter(|n| (1..=MAX_SLOTS).contains(n)) {
                Some(n) => {
                    self.vwin.alloc.policy.slots = n;
                    self.config.vwin.slots = n;
                    self.vwin_sync();
                    json!({"ok": true, "vwin": self.vwin_report()})
                }
                None => json!({"ok": false, "error": format!("slots takes 1 to {MAX_SLOTS}")}),
            },
            ["claim", id, kind, rest @ ..] => {
                let (Some(x), Some(y), Some(w), Some(h)) = (num(rest, 0), num(rest, 1), num(rest, 2), num(rest, 3)) else {
                    return json!({"ok": false, "error": "vwin claim ID KIND X Y W H [APP] [CODEC] [WxH@FPS] [texture-only]"});
                };
                let mut v = json!({"op": "claim", "id": id, "kind": kind, "rect": {"x": x, "y": y, "w": w, "h": h}});
                let (mut codec, mut size) = (None, None);
                for t in &rest[4..] {
                    if Codec::parse(t).is_some() {
                        codec = Some(*t);
                    } else if *t == "texture-only" {
                        v["texture_only"] = json!(true);
                    } else if let Some((wh, fps)) = t.split_once('@')
                        && let Some((nw, nh)) = wh.split_once('x')
                    {
                        size = Some((nw.parse::<u64>().unwrap_or(0), nh.parse::<u64>().unwrap_or(0), fps.parse::<u64>().unwrap_or(0)));
                    } else {
                        v["app"] = json!(t);
                    }
                }
                if let Some((nw, nh, fps)) = size {
                    v["need"] = json!({"codec": codec.unwrap_or("h264"), "w": nw, "h": nh, "fps": fps});
                }
                ask(self, v)
            }
            ["move", id, rest @ ..] => {
                let (Some(x), Some(y), Some(w), Some(h)) = (num(rest, 0), num(rest, 1), num(rest, 2), num(rest, 3)) else {
                    return json!({"ok": false, "error": "vwin move ID X Y W H"});
                };
                ask(self, json!({"op": "move", "id": id, "rect": {"x": x, "y": y, "w": w, "h": h}}))
            }
            ["release", "all"] => ask(self, json!({"op": "releaseAll"})),
            ["release", id] => ask(self, json!({"op": "release", "id": id})),
            _ => json!({"ok": false, "error": "vwin [off|idle|apps | texture on|off | slots N | claim ID camera-full|camera-popup|pip X Y W H [APP] [CODEC] [WxH@FPS] [texture-only] | move ID X Y W H | release ID|all]"}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(mode: Mode) -> Policy {
        Policy { mode, slots: 2, texture: false }
    }

    fn req(id: &str, kind: Kind, need: Option<Need>) -> Request {
        Request { id: id.into(), kind, app: String::new(), rect: Rect::new(1200, 134, 520, 292), need, texture_only: false }
    }

    fn need(codec: Codec, w: u32, h: u32, fps: u32) -> Option<Need> {
        Some(Need { codec, w, h, fps })
    }

    fn granted(d: &Decision) -> Option<(usize, Vec<Victim>)> {
        match d {
            Decision::Granted { slot, victims, .. } => Some((*slot, victims.clone())),
            Decision::Texture { .. } | Decision::Denied(..) => None,
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
    fn a_claim_takes_the_wallpapers_window_though_one_is_free_and_a_full_view_takes_main_from_the_popup() {
        let mut a = Allocator::new(policy(Mode::Idle));
        let w = World { wall: true, ..World::default() };
        // the wallpaper holds main and the sub window is free, but LumaTV's own sessions of the decoder hold both scalers: the pop-up takes the wallpaper's
        // window (the caller closes it) and does not start a second session beside it
        let d = a.request(&w, req("popup", Kind::CameraPopup, need(Codec::H264, 1280, 720, 15)));
        assert_eq!(granted(&d), Some((0, vec![Victim::Wall])));
        // (the world still lists the wallpaper here, as it does until its window is closed) a 4K full view needs main: the pop-up, lower, is its holder
        let d = a.request(&w, req("full", Kind::CameraFull, need(Codec::H265, 3840, 2160, 15)));
        assert_eq!(granted(&d), Some((0, vec![Victim::Claim("popup".into())])));
        // a stream the wallpaper's window is too small for is not taken from it: it goes to the free one that takes it
        let mut b = Allocator::new(policy(Mode::Idle));
        let d = b.request(&w, req("big", Kind::Pip, need(Codec::H265, 3840, 2160, 15)));
        assert_eq!(granted(&d), Some((0, vec![Victim::Wall])), "main takes it, and main is the wallpaper's");
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
    fn a_wallpaper_still_there_after_its_window_was_taken_does_not_end_the_claim_that_took_it() {
        let mut a = Allocator::new(Policy { mode: Mode::Apps, slots: 1, texture: false });
        let wall = World { wall: true, ..World::default() };
        let d = a.request(&wall, req("pop", Kind::CameraPopup, None));
        assert_eq!(granted(&d), Some((0, vec![Victim::Wall])));
        assert_eq!(a.settle(&wall), Vec::<String>::new(), "its process has not gone yet");
        assert_eq!(a.claim("pop").unwrap().state, ClaimState::Granted);
        assert_eq!(a.occupancy(&wall), vec![Some(Holder::Claim("pop".to_string()))], "the claim holds the window, the wallpaper none");
    }

    #[test]
    fn the_sinks_are_read_from_the_status_videooutputd_pushes() {
        // as recorded on the set (round8/tv-2026-10-25/log.txt, LumaTV's own session), trimmed
        let status = json!({"video": [
            {"sink": "MAIN", "connected": true, "context": "_LvZICq56d0kLzl", "appId": "org.nphil.lumatv.vplane", "zOrder": 0, "width": 1856, "height": 1024,
             "frameRate": 60, "videoInfo": {"lowDelayMode": true, "isGameMode": true, "subScalerOccupied": true}},
            {"sink": "SUB0", "connected": false, "zOrder": 1, "videoInfo": {}}], "returnValue": true});
        let sinks = Sinks::parse(&status).expect("a status with a video list");
        assert_eq!(sinks.video.len(), 2);
        assert_eq!((sinks.video[0].name.as_str(), sinks.video[0].connected, sinks.video[0].occupied, sinks.video[0].z), ("MAIN", true, true, 0));
        assert_eq!((sinks.video[0].width, sinks.video[0].height, sinks.video[0].fps), (1856, 1024, 60.0));
        assert!(!sinks.video[1].connected && !sinks.video[1].occupied && sinks.video[1].context.is_empty());
        let on_main = sinks.of(&["_LvZICq56d0kLzl".to_string()]).map(|s| s.name.as_str());
        assert_eq!(on_main, Some("MAIN"));
        assert!(sinks.of(&["_other".to_string()]).is_none(), "a context that is on no connected sink");
        assert_eq!(sinks.json()[0]["occupiesBoth"], true);
        assert_eq!(Sinks::parse(&json!({"returnValue": true})), None, "an answer without the list is not a status");
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
            texture: false,
            window: None,
            wide: false,
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

    fn texture_policy(mode: Mode) -> Policy {
        Policy { mode, slots: 2, texture: true }
    }

    #[test]
    fn a_popup_no_window_can_carry_is_a_texture_and_holds_no_window() {
        let playing = World { app_elements: 1, app_playing: 1, ..World::default() };
        let mut a = Allocator::new(texture_policy(Mode::Idle));
        // an app's video: idle leaves the claim no window, so it is a texture (the camera's small stream, decoded by the feeder)
        let d = a.request(&playing, req("pop", Kind::CameraPopup, need(Codec::H264, 1920, 1080, 30)));
        assert_eq!(d, Decision::Texture { limits: TEXTURE_LIMITS, because: Reason::AppVideo });
        let c = a.claim("pop").unwrap();
        assert!(c.texture && c.state == ClaimState::Granted && c.slot.is_none() && c.limits == Some(TEXTURE_LIMITS));
        let j = Allocator::claim_json(c, true, Some("texture"));
        assert_eq!((j["slot"].as_str(), j["texture"].as_bool(), j["limits"]["w"].as_u64()), (Some("texture"), Some(true), Some(1280)));
        // it takes no window: the occupancy is the app's alone
        assert_eq!(a.occupancy(&playing), vec![Some(Holder::App), None]);
        // and what ends a window claim does not end it: the app's video, both scalers, the switch
        for world in [playing.clone(), World { both_scalers: true, ..playing.clone() }] {
            assert!(a.settle(&world).is_empty(), "{world:?}");
        }
        // `off` grants and keeps nothing; the textures' own switch is `texture`
        assert_eq!(a.request(&World::default(), req("x", Kind::CameraPopup, None)), Decision::Granted { slot: 0, limits: limits(0, Codec::H264), victims: vec![] });
        a.policy.mode = Mode::Off;
        let mut ended = a.settle(&playing);
        ended.sort();
        assert_eq!(ended, ["pop", "x"]);
        assert_eq!(a.request(&playing, req("y", Kind::CameraPopup, None)), Decision::Denied(Reason::Off, None));
    }

    #[test]
    fn a_texture_only_claim_is_a_texture_even_where_a_window_is_free_and_never_a_window() {
        let only = |id: &str| Request { texture_only: true, ..req(id, Kind::CameraPopup, need(Codec::H264, 1920, 1088, 30)) };
        let mut a = Allocator::new(texture_policy(Mode::Apps));
        // Home with every window free: a window would be granted, and the texture is what is wanted
        assert!(matches!(a.request(&World::default(), only("a")), Decision::Texture { because: Reason::TextureOnly, .. }));
        assert!(a.claim("a").is_some_and(|c| c.texture && c.slot.is_none()), "no window is held");
        // over an app's video it is the same texture
        let playing = World { app_elements: 1, app_playing: 1, ..World::default() };
        assert!(matches!(a.request(&playing, only("b")), Decision::Texture { .. }));
        // with textures off, or the screen off, or the whole switch off, it has nothing: never a window
        a.policy.texture = false;
        assert_eq!(a.request(&World::default(), only("c")), Decision::Denied(Reason::TextureOnly, None));
        a.policy.texture = true;
        assert_eq!(a.request(&World { standby: true, ..World::default() }, only("d")), Decision::Denied(Reason::Standby, None));
        let mut off = Allocator::new(texture_policy(Mode::Off));
        assert_eq!(off.request(&World::default(), only("e")), Decision::Denied(Reason::Off, None));
        // and the claim the UI posts says so in JSON
        let v = json!({"id": "p", "kind": "camera-popup", "rect": {"x": 1200, "y": 134, "w": 520, "h": 292}, "texture_only": true});
        assert!(Request::from_json(&v).unwrap().texture_only);
        assert!(!Request::from_json(&json!({"id": "p", "kind": "camera-popup", "rect": {"x": 1200, "y": 134, "w": 520, "h": 292}})).unwrap().texture_only);
    }

    #[test]
    fn every_reason_the_windows_give_is_a_texture_but_the_screen_being_off_and_the_full_view_is_never_one() {
        let mut a = Allocator::new(texture_policy(Mode::Apps));
        // 4K120 holds both scalers
        let hfr = World { app_elements: 1, app_playing: 1, both_scalers: true, ..World::default() };
        assert!(matches!(a.request(&hfr, req("a", Kind::Pip, None)), Decision::Texture { because: Reason::BothScalers, .. }));
        // the stream is too big for the sub window: a texture is the answer only for kinds that may (the texture's own limits are the feeder's business)
        let playing = World { app_elements: 1, app_playing: 1, ..World::default() };
        assert!(matches!(a.request(&playing, req("b", Kind::CameraPopup, need(Codec::H264, 2560, 1440, 15))), Decision::Texture { because: Reason::TooBig, .. }));
        // the full view is the camera's own stream at the screen's size: a window or stills
        assert_eq!(a.request(&hfr, req("full", Kind::CameraFull, need(Codec::H264, 3840, 2160, 15))), Decision::Denied(Reason::BothScalers, None));
        // the switch for textures is its own, and standby is no time for a decoder
        a.policy.texture = false;
        assert_eq!(a.request(&hfr, req("c", Kind::Pip, None)), Decision::Denied(Reason::BothScalers, None));
        a.policy.texture = true;
        let off = World { standby: true, ..World::default() };
        assert_eq!(a.request(&off, req("d", Kind::Pip, None)), Decision::Denied(Reason::Standby, None));
        // a window still wins when there is one
        assert!(matches!(a.request(&World::default(), req("e", Kind::Pip, None)), Decision::Granted { slot: 0, .. }));
    }

    #[test]
    fn textures_are_few_end_in_standby_and_when_their_switch_goes_off() {
        let hfr = World { app_elements: 1, app_playing: 1, both_scalers: true, ..World::default() };
        let mut a = Allocator::new(texture_policy(Mode::Apps));
        assert!(matches!(a.request(&hfr, req("a", Kind::Pip, None)), Decision::Texture { .. }));
        assert!(matches!(a.request(&hfr, req("b", Kind::CameraPopup, None)), Decision::Texture { .. }));
        assert_eq!(a.request(&hfr, req("c", Kind::Pip, None)), Decision::Denied(Reason::BothScalers, None), "{MAX_TEXTURES} decoders are the most");
        // asking again for a claim it holds is not a third
        assert!(matches!(a.request(&hfr, req("a", Kind::Pip, None)), Decision::Texture { .. }));
        let standby = World { standby: true, ..hfr.clone() };
        let mut ended = a.settle(&standby);
        ended.sort();
        assert_eq!(ended, ["a", "b"]);
        assert_eq!(a.claim("a").unwrap().state, ClaimState::Preempted(Reason::Standby));
        let mut a = Allocator::new(texture_policy(Mode::Apps));
        a.request(&hfr, req("a", Kind::Pip, None));
        a.policy.texture = false;
        assert_eq!(a.settle(&hfr), ["a"]);
        assert_eq!(a.claim("a").unwrap().state, ClaimState::Preempted(Reason::Off));
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
