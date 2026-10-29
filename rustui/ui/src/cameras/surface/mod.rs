//! The Cameras app drawn: the surfaces of the grid, the full view, the strip, the events and clips sheet and the options page, each built
//! from the TV kit and kept in step with the model's [`View`] (`docs/research/cameras-ui-spec.md`, the mocks in `docs/mocks/cameras-scrypted/`).
//!
//! The rules the surfaces keep:
//!
//! - **Built when the app opens, dropped when it closes** (the scene's budget rule, `docs/research/rust-shell-kit.md`): nothing of the app is
//!   kept alive while it is closed but the decoded pictures, so the grid paints the last picture of every camera in its first frame. A piece
//!   of a screen is made again only when what it says changes (its signature); a move of the focus, a new picture or a scroll changes layers
//!   that are there.
//! - **The model owns the focus**: the surfaces register every focusable with the shared [`Engine`] (for the pointer's hit tests and the
//!   flowing lift) and follow the model's focus with [`Engine::focus`]; the D-pad's rules are the model's and the spec's.
//! - **Pictures are scaled by the compositor** ([`pic`]); the chrome over the picture is one reading surface that fades after 2.5 s
//!   ([`full`]); no sentence of instruction anywhere, hints are glyphs ([`pill`]).

pub mod common;
pub mod full;
pub mod grid;
pub mod options;
pub mod pic;
pub mod pill;
pub mod root;
pub mod sheet;
pub mod strip;
pub mod toast;

use crate::kit::Engine;
use crate::kit::Kit;
use crate::kit::focus::{Id, Region, Scope};
use pic::PictureStore;

pub use root::CamerasSurface;

/// The identity of everything the surfaces register with the focus engine: the base of the cameras' ids, then the screen's.
pub mod ids {
    use crate::kit::focus::Id;

    pub const BASE: u32 = 0x4341_0000;
    pub const HEADER_EVENTS: Id = Id(BASE + 1);
    pub const HEADER_SETTINGS: Id = Id(BASE + 2);
    pub const EMPTY_SETTINGS: Id = Id(BASE + 3);
    pub const RETRY: Id = Id(BASE + 10);
    pub const BACKDROP: Id = Id(BASE + 11);
    pub const STRIP_SPAN: Id = Id(BASE + 20);
    pub const STRIP_RATE: Id = Id(BASE + 21);
    pub const STRIP_BAR: Id = Id(BASE + 22);
    pub const STRIP_LIVE: Id = Id(BASE + 23);
    pub const STRIP_BACK: Id = Id(BASE + 24);
    pub const STRIP_PLAY: Id = Id(BASE + 25);
    pub const STRIP_FORWARD: Id = Id(BASE + 26);
    pub const STRIP_PREV: Id = Id(BASE + 27);
    pub const STRIP_NEXT: Id = Id(BASE + 28);
    pub const TAB_EVENTS: Id = Id(BASE + 40);
    pub const TAB_CLIPS: Id = Id(BASE + 41);
    pub const OPTIONS_BACK: Id = Id(BASE + 50);

    pub fn tile(i: usize) -> Id {
        Id(BASE + 0x100 + i as u32)
    }

    pub fn card(i: usize) -> Id {
        Id(BASE + 0x200 + i as u32)
    }

    pub fn option(i: usize) -> Id {
        Id(BASE + 0x300 + i as u32)
    }
}

pub const REGION_TILES: Region = Region(210);
pub const REGION_HEADER: Region = Region(211);
pub const REGION_FULL: Region = Region(212);
pub const REGION_STRIP: Region = Region(213);
pub const REGION_SHEET: Region = Region(214);
pub const REGION_CARDS: Region = Region(215);
pub const REGION_OPTIONS: Region = Region(216);

pub const SCOPE_BASE: Scope = Scope(200);
pub const SCOPE_STRIP: Scope = Scope(201);
pub const SCOPE_SHEET: Scope = Scope(202);
pub const SCOPE_OPTIONS: Scope = Scope(203);

/// The depths of the parts of the app, from the band's base (`system_z + kit::z::CAMERAS`; the band is 100 deep).
pub mod depth {
    pub const BLACK: i32 = 0;
    pub const PICTURE: i32 = 2;
    pub const VEIL: i32 = 4;
    pub const CONTENT: i32 = 10;
    pub const HEADER: i32 = 30;
    pub const CHROME: i32 = 40;
    pub const STRIP: i32 = 50;
    pub const BUBBLE: i32 = 60;
    pub const SHEET: i32 = 70;
    pub const OPTIONS: i32 = 80;
    pub const NOTE: i32 = 88;
    pub const TOAST: i32 = 92;
}

/// What a surface needs to build or follow its view.
pub struct Cx<'a> {
    pub kit: &'a mut Kit,
    pub engine: &'a mut Engine,
    pub store: &'a PictureStore,
    /// The band's base z.
    pub z: i32,
    pub now_ms: u64,
    pub now_ns: u64,
}

impl Cx<'_> {
    pub fn at(&self, depth: i32) -> i32 {
        self.z + depth
    }

    /// Moves the engine's focus to `id` when it is not there yet (the model owns the focus; the surface follows it with the flow).
    pub fn follow(&mut self, id: Id) {
        if self.engine.focus.current() != Some(id) {
            self.engine.focus(id, self.now_ns);
        }
    }
}
