//! The TV kit: what every screen of the Rust shell is built from (`docs/research/rust-shell-kit.md`). The toolkit is LumaShow's
//! `ui` crate (components, glass, glyph text, motion tracks); the kit adds what the TV needs on top: the one token layer
//! ([`lumatv_tokens`]) resolved into the toolkit's look, the focus engine ([`focus`] with the flowing motion of [`flow`]), the
//! remote-key hint glyphs ([`glyph`], [`hint`]), the TV motion set ([`motion`]), panes of the TV's glass ([`pane`]), text in the
//! TV's type roles ([`text`]) and the screen contract ([`screen`]).

pub mod avatar;
pub mod button;
pub mod flow;
pub mod focus;
pub mod glyph;
pub mod gridfit;
pub mod hint;
pub mod icon;
pub mod lint;
pub mod motion;
pub mod pane;
pub mod picture;
pub mod pointer;
pub mod screen;
#[cfg(test)]
pub mod testkit;
pub mod text;
pub mod vector_icon;
pub mod viewport;
pub mod widgets;
pub mod wrap;

use lumatv_tokens::{Tokens, TypeRole};
use ui::context::Ui;
use ui::lucent::material::MaterialTier;
use ui::lucent::palette::Mode;
use ui::scene::Scene;
use ui::text::{Type, Weight};

/// Where each kind draws in the engine's z, as offsets from the compositor's `system_z` (everything at `system_z` or more is
/// stack 1, over the windows). The system layer (the volume display, the toasts, the alerts and cards) is the topmost band,
/// so a bar or a toast stays over a panel or Settings; each band is a kind's own, 200 deep.
pub mod z {
    /// The channel guide (a full page over the live picture): under the panels, so the quick panel can be opened over it.
    pub const GUIDE: i32 = 60;
    /// The quick panel, the input picker, Quick Access, the drawer, the context menu, confirm, the switcher's chrome.
    pub const PANELS: i32 = 100;
    /// The Cameras app (the grid, the full view, the strip, the sheet): over the panels, under Settings.
    pub const CAMERAS: i32 = 200;
    pub const SETTINGS: i32 = 300;
    /// The display lab (a test card over the whole screen, one sheet beside it): over Settings it is opened from, under the overlay editor.
    pub const LAB: i32 = 400;
    /// The overlay layout editor.
    pub const EDITOR: i32 = 500;
    pub const KEYBOARD: i32 = 600;
    /// The volume display, the toasts, the alerts, the cards, the capture mark, the saver.
    pub const SYSTEM: i32 = 800;
}

pub use flow::{Flow, Lifted, Spec};
pub use focus::{Dir, Focus, Id, Moved, Region, Scope};

/// What a screen is built with: the toolkit's context (scene, fonts, look) and the TV's tokens.
pub struct Kit {
    pub ui: Ui,
    pub tokens: Tokens,
    pub mode: Mode,
    pub tier: MaterialTier,
    pub text_scale: f32,
    /// The Lucide icons of the module (`kit/icon.rs`); empty until the shell is given its module directory.
    pub icons: icon::Icons,
    pub(crate) theme_thumbnails: crate::settings::surface::rows::thumbnails::Cache,
    generation: u64,
    /// The blurred wallpaper picture alone, as a prepared set Home has made (the `clear` card glass): the glass of Home's panes shows it, not the page below
    /// them, which has the scrim on it. The QML Home's glass samples the graded picture and not the scrim over it.
    pub wall_backdrop: Option<compositor_protocol::BackdropId>,
    /// The Hz the output runs at, from the compositor's `refresh` topic (60 until it says): at 100 or more a full screen under a changing alpha costs
    /// more than a refresh ([`Kit::modal_beat_ms`]).
    pub refresh_hz: f32,
}

impl Kit {
    /// A kit on `scene` with `tokens` in `mode`, glyph text on and the TV's type roles warmed, so no screen rasters a glyph on a
    /// frame.
    pub fn new(scene: Scene, tokens: Tokens, mode: Mode, tier: MaterialTier, text_scale: f32) -> std::io::Result<Self> {
        let look = tokens.look(mode, 1, tier, text_scale);
        let mut ui = Ui::with_look(scene, look);
        if tokens.grid_fit() {
            ui.fonts.set_glyph_source(Some(Box::new(gridfit::GridFit::new())));
        }
        ui.enable_glyph_text().map_err(|e| std::io::Error::other(format!("{e}")))?;
        let mut kit = Self { ui, tokens, mode, tier, text_scale, icons: icon::Icons::default(), theme_thumbnails: Default::default(), generation: 1, wall_backdrop: None, refresh_hz: 60.0 };
        kit.warm();
        Ok(kit)
    }

    /// The roles the screens draw, the label in the heavier weight too (a level's number, a button's word).
    fn warm(&mut self) {
        let mut types: Vec<Type> = [TypeRole::Label, TypeRole::Caption, TypeRole::Body, TypeRole::Title].iter().map(|r| self.ty(*r)).collect();
        types.push(Type { weight: Weight::SemiBold, ..self.ty(TypeRole::Label) });
        self.ui.warm(&types);
    }

    /// A type role at the TV's size and the person's text size.
    pub fn ty(&self, role: TypeRole) -> Type {
        self.ui.look.ty(self.tokens.ty(role))
    }

    /// The line height of a role, as labels of it are laid out.
    pub fn line(&self, role: TypeRole) -> f32 {
        ui::components::line_height(self.ty(role))
    }

    /// `Lu.space(n)`.
    pub fn space(&self, n: usize) -> f32 {
        self.tokens.space(n)
    }

    /// Installs another theme or mode or text size: a new look generation; screens restyle what they show.
    pub fn retheme(&mut self, tokens: Tokens, mode: Mode, text_scale: f32) {
        let roles = [TypeRole::Label, TypeRole::Caption, TypeRole::Body, TypeRole::Title];
        let before = roles.map(|role| self.ty(role));
        self.generation += 1;
        self.tokens = tokens;
        self.mode = mode;
        self.text_scale = text_scale;
        self.ui.look = self.tokens.look(mode, self.generation, self.tier, text_scale);
        if before != roles.map(|role| self.ty(role)) {
            self.warm();
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// What the one scene every kind shares has left of the compositor's ceilings: layers, groups and motion tracks
    /// (`ui::limits`: 256, 96, 32). A modal kind asserts its budget against this when it opens.
    pub fn room(&self) -> Room {
        let scene = &self.ui.scene;
        Room {
            layers: scene.layer_room(),
            groups: ui::limits::MAX_GROUP_NODES.saturating_sub(scene.groups_alive()),
            tracks: ui::limits::MAX_MOTION_TRACKS.saturating_sub(scene.motions_alive()),
        }
    }

    /// The flowing-focus spring as the theme sets it: the reduced-motion crossfade when the theme asks for it.
    pub fn flow_spec(&self) -> Spec {
        Spec { reduced: self.ui.look.reduced_motion, ..Spec::default() }
    }
}

/// What is left of the scene's ceilings ([`Kit::room`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Room {
    pub layers: usize,
    pub groups: usize,
    pub tracks: usize,
}

/// The focus engine every screen gets: the registry and its rules ([`Focus`]) with the flowing motion ([`Flow`]) that shows every
/// move, so a key on any screen moves the focus and the lift the same way.
pub struct Engine {
    pub focus: Focus,
    pub flow: Flow,
}

impl Engine {
    pub fn new(kit: &Kit) -> Result<Self, ui::error::UiError> {
        Ok(Self { focus: Focus::new(), flow: Flow::new(&kit.ui.scene, kit.flow_spec(), flow::POOL)? })
    }

    /// A D-pad key at `now_ns`: the focus moves by the rules, and the outgoing and incoming focusables flow.
    pub fn step(&mut self, dir: Dir, now_ns: u64) -> Moved {
        let moved = self.focus.step(dir);
        if let Moved::To(from, to) = moved {
            let _ = self.flow.go(Some(from), Some(to), now_ns);
        } else if let Moved::Stayed(id) = moved {
            let _ = self.flow.go(None, Some(id), now_ns);
        }
        moved
    }

    /// Focuses `id` outright (a screen's initial focus, a click): the move flows from what was focused.
    pub fn focus(&mut self, id: Id, now_ns: u64) -> bool {
        let from = self.focus.current();
        if !self.focus.focus(id) {
            return false;
        }
        let _ = self.flow.go(from, Some(id), now_ns);
        true
    }

    /// A click at (`x`, `y`): focuses what is under it and returns it to activate.
    pub fn click(&mut self, x: f32, y: f32, now_ns: u64) -> Option<Id> {
        let from = self.focus.current();
        let id = self.focus.click(x, y)?;
        let _ = self.flow.go(from, Some(id), now_ns);
        Some(id)
    }

    /// Clears the focus: what was lifted settles.
    pub fn clear(&mut self, now_ns: u64) {
        let from = self.focus.current();
        self.focus.clear();
        let _ = self.flow.go(from, None, now_ns);
    }
}
