//! Dialogs, sheets and choice popups (design section 3.3, `OverlayModel { title, body, actions, dismissal }` -> result): a reading
//! surface over a scrim, with its own words and its own buttons.
//!
//! An overlay is **modal**: while it is shown it takes every contact (a host that has one shown sends it all input and nothing to
//! what is underneath, which is therefore inert), and it answers with one [`OverlayResult`] when the person acts or dismisses it.
//! It always has a visible way out: a Close button, unless it can only be left by one of its actions ([`Dismissal::ActionOnly`]).
//! A contact that begins and ends on the scrim dismisses it when the model allows that.
//!
//! The panel is the `Overlay` material (opaque enough to read on: the resolved 0.96 sheet tint with the raised pair and the single
//! cast shadow); its buttons are the toolkit's own and follow their own rows of the material table. Large text makes the panel
//! taller and wraps its words onto more lines; when the screen has no more height the explanation ends in an ellipsis, never in a
//! smaller type. Showing and hiding are one pose per layer (the panel rises a little and fades; reduced motion only fades).

use super::button::{Button, ButtonAction, ButtonKind, ButtonModel};
use super::choice::Router;
use super::group::{Group, Part, Pose};
use super::icon::{Icon, icon_part};
use super::plate::{shape, solid};
use super::row::{Row, RowAction, RowId, RowModel, Trailing};
use super::semantics::{SemanticKind, SemanticNode, SemanticsSink};
use super::surface::Surface;
use super::text::{Run, ellipsise, line_height, wrap_to};
use super::text_layer::{Item, TextLayer};
use super::{Component, Damage};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::{Event, TouchPhase};
use crate::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use crate::lucent::palette::PaletteRole;
use crate::lucent::tokens::{motion, smart, space, ty};
use crate::scene::ShapeLayer;
use crate::text::{Type, Weight};
use compositor_protocol::Props;
use std::io;

/// Which overlay it is: where it sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    /// Centred on the screen.
    Dialog,
    /// Anchored to the bottom edge, as wide as the screen allows.
    Sheet,
}

/// How an action looks and what it is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Normal,
    /// The one action the overlay is for: on the accent.
    Primary,
    /// An action that cannot be taken back: its words are in the danger colour.
    Destructive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverlayAction {
    pub id: String,
    pub label: String,
    pub kind: ActionKind,
    pub enabled: bool,
}

impl OverlayAction {
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: ActionKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
            enabled: true,
        }
    }
}

/// How the overlay can be left without taking an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismissal {
    /// A Close button, and a contact that begins and ends on the scrim.
    ScrimOrClose,
    /// A Close button only.
    CloseOnly,
    /// No Close button: only a contact that begins and ends on the scrim (and its actions) leaves it, as the mock's confirmations.
    Scrim,
    /// Only by one of its actions: it has no Close button (a confirmation that must be answered).
    ActionOnly,
}

/// A short list of things to choose one of, in a popup: a row each, the chosen one with a check.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// (id, label).
    pub items: Vec<(String, String)>,
    pub selected: Option<String>,
}

/// The most options a popup lists; a longer list is a page of its own.
pub const MAX_OPTIONS: usize = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct OverlayModel {
    pub title: String,
    pub body: String,
    pub options: Option<Options>,
    pub actions: Vec<OverlayAction>,
    pub dismissal: Dismissal,
    /// A line of what was verified under the body: a tick and the words ("Signed by lumashow · sha256 4f2a…c91e").
    pub verified: Option<String>,
    /// The look of a confirmation of the Settings (the approved mock's), see [`OverlayModel::confirmation`].
    pub confirm: bool,
}

impl OverlayModel {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            options: None,
            actions: Vec::new(),
            dismissal: Dismissal::ScrimOrClose,
            verified: None,
            confirm: false,
        }
    }

    pub fn action(mut self, action: OverlayAction) -> Self {
        self.actions.push(action);
        self
    }

    pub fn dismissal(mut self, dismissal: Dismissal) -> Self {
        self.dismissal = dismissal;
        self
    }

    /// A tick and `line` under the body: what the Show checked before asking.
    pub fn verified(mut self, line: impl Into<String>) -> Self {
        self.verified = Some(line.into());
        self
    }

    /// The look of a confirmation of the Settings, as the approved mock draws it: 560 wide with 28 above its words, 32 at the sides and
    /// 24 below, its words in the primary ink, and no Close button (Cancel answers it, or a touch outside).
    pub fn confirmation(mut self) -> Self {
        self.confirm = true;
        self.dismissal = Dismissal::Scrim;
        self
    }

    pub fn options(mut self, items: Vec<(String, String)>, selected: Option<&str>) -> Self {
        self.options = Some(Options {
            items,
            selected: selected.map(str::to_owned),
        });
        self
    }
}

/// What an overlay answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlayResult {
    /// An action's id.
    Action(String),
    /// An option of a popup was chosen: its id.
    Chosen(String),
    /// Closed without an action.
    Dismissed,
}

const PAD: f32 = space::S6;
/// The tick before a verified line, and its room with the gap after it.
const TICK: f32 = 22.0;
const TICK_SLOT: f32 = TICK + space::S2;
const CLOSE: f32 = 48.0;
const ACTION_H: f32 = 56.0;
const OPTION_GAP: f32 = space::S2;
const TITLE_LINES: usize = 2;
const MAX_BODY_LINES: usize = 8;
const RISE: f32 = space::S6;

/// Where everything goes for one model at one text size.
struct Layout {
    panel: Frame,
    title: Vec<String>,
    body: Vec<String>,
    text: Frame,
    close: Option<Frame>,
    options: Vec<Frame>,
    actions: Vec<Frame>,
    /// A confirmation of the Settings: its words are in the primary ink.
    confirm: bool,
    /// The verified line (cut to fit) and where it stands from the top of the words.
    verified: Option<(String, f32)>,
}

/// The room between the panel's edge and what it holds: the mock's confirmation has 28 above, 32 at the sides and 24 below.
struct Pad {
    x: f32,
    top: f32,
    bottom: f32,
}

impl Pad {
    fn of(model: &OverlayModel) -> Self {
        if model.confirm {
            Self {
                x: 32.0,
                top: 28.0,
                bottom: 24.0,
            }
        } else {
            Self {
                x: PAD,
                top: PAD,
                bottom: PAD,
            }
        }
    }
}

fn panel_width(kind: OverlayKind, screen: Frame, confirm: bool) -> f32 {
    match kind {
        OverlayKind::Dialog => {
            (if confirm { 560.0f32 } else { 600.0f32 }).min(screen.w - 2.0 * space::S6)
        }
        OverlayKind::Sheet => (screen.w - 2.0 * space::S6).min(880.0),
    }
}

/// How an overlay sets its words: the types, the height of a line of each, and the room between the parts.
///
/// A confirmation of the Settings is the approved mock's `.dlg`: the title 30 px in the heavier weight on a 1.25 line, the body 22 px
/// on a 1.35 line, the verified line 20 px on a 1.25 line, 12 between the title and the body, 14 before the verified line, 24 before
/// the buttons, which are at least 140 wide and 16 apart. Every other overlay keeps the toolkit's running text.
#[derive(Clone, Copy)]
struct Words {
    title: Type,
    body: Type,
    verified: Type,
    title_line: f32,
    body_line: f32,
    verified_line: f32,
    after_title: f32,
    before_verified: f32,
    before_actions: f32,
    action_min: f32,
    action_gap: f32,
}

impl Words {
    fn of(ui: &Ui, confirm: bool) -> Self {
        if confirm {
            let (title, body) = (
                ui.look.page_ty(30, Weight::SemiBold),
                ui.look.page_ty(22, Weight::Regular),
            );
            let verified = ui.look.ty(ty::CAPTION);
            Self {
                title,
                body,
                verified,
                title_line: title.px as f32 * 1.25,
                body_line: body.px as f32 * 1.35,
                verified_line: verified.px as f32 * 1.25,
                after_title: 12.0,
                before_verified: 14.0,
                before_actions: 24.0,
                action_min: 140.0,
                action_gap: 16.0,
            }
        } else {
            let (title, body, verified) = (
                ui.look.ty(ty::TITLE),
                ui.look.ty(ty::BODY),
                ui.look.ty(ty::CAPTION),
            );
            Self {
                title,
                body,
                verified,
                title_line: line_height(title),
                body_line: line_height(body),
                verified_line: line_height(verified),
                after_title: space::S2,
                before_verified: space::S3,
                before_actions: space::S5,
                action_min: 2.0 * smart::TARGET,
                action_gap: space::S3,
            }
        }
    }

    /// Where a run of `ty` starts so that its glyphs stand in the middle of a line `line` high that begins at `top` (a run's own box
    /// is Lucent's 1.4 line, which is taller than the mock's).
    fn run_y(top: f32, line: f32, ty: Type) -> f32 {
        top + ((line - line_height(ty)) / 2.0).round()
    }
}

/// Where a dialog of the Settings' look stands from the top of the screen, as the mock's `.dlg`: not centred, 110 down (lower
/// only when it is taller than what is left).
const CONFIRM_TOP: f32 = 110.0;

fn layout(ui: &mut Ui, screen: Frame, kind: OverlayKind, model: &OverlayModel) -> Layout {
    let pad = Pad::of(model);
    let width = panel_width(kind, screen, model.confirm);
    let inner = width - 2.0 * pad.x;
    let close = matches!(
        model.dismissal,
        Dismissal::ScrimOrClose | Dismissal::CloseOnly
    );
    let title_w = if close {
        inner - CLOSE - space::S3
    } else {
        inner
    };
    let words = Words::of(ui, model.confirm);
    let (tt, bt, ct) = (words.title, words.body, words.verified);
    let title = wrap_to(
        &mut ui.fonts,
        &model.title,
        tt,
        title_w.max(0.0),
        TITLE_LINES,
    );
    let title_h = (title.len() as f32 * words.title_line).max(if close { CLOSE } else { 0.0 });
    let verified_line = model
        .verified
        .as_ref()
        .map(|line| ellipsise(&mut ui.fonts, line, ct, (inner - TICK_SLOT).max(0.0)));
    let verified_h = if verified_line.is_some() {
        words.before_verified + words.verified_line
    } else {
        0.0
    };

    let options_h = model.options.as_ref().map_or(0.0, |o| {
        o.items.len() as f32 * (smart::TARGET + OPTION_GAP) - OPTION_GAP + space::S4
    });

    // Actions: side by side, right-aligned, if they fit; otherwise one under another.
    let widths: Vec<f32> = model
        .actions
        .iter()
        .map(|a| {
            Button::required_width(ui, button_kind(a.kind), &ButtonModel::labelled(&a.label))
                .max(words.action_min)
                .min(inner)
        })
        .collect();
    let row_w =
        widths.iter().sum::<f32>() + words.action_gap * widths.len().saturating_sub(1) as f32;
    let stacked = row_w > inner;
    let actions_h = match (model.actions.len(), stacked) {
        (0, _) => 0.0,
        (_, false) => ACTION_H,
        (n, true) => n as f32 * ACTION_H + (n - 1) as f32 * space::S2,
    };

    let max_h = screen.h - 2.0 * space::S4;
    let fixed = pad.top
        + title_h
        + words.after_title
        + options_h
        + verified_h
        + if actions_h > 0.0 {
            words.before_actions + actions_h
        } else {
            0.0
        }
        + pad.bottom;
    let room = max_h - fixed;
    let allowed = ((room / words.body_line).floor().max(0.0) as usize).min(MAX_BODY_LINES);
    let body = if model.body.is_empty() || allowed == 0 {
        Vec::new()
    } else {
        wrap_to(&mut ui.fonts, &model.body, bt, inner, allowed)
    };
    let body_h = body.len() as f32 * words.body_line;
    let height = (fixed + body_h).min(max_h);
    let panel = match kind {
        OverlayKind::Dialog => {
            let top = if model.confirm {
                // The mock's `.dlg` is 110 down whatever its height; the tallest (a six-line question) ends 12 above the screen's foot.
                (screen.y + CONFIRM_TOP).min(screen.bottom() - height - space::S3)
            } else {
                screen.y + (screen.h - height) / 2.0
            };
            Frame::new(screen.x + (screen.w - width) / 2.0, top, width, height)
        }
        OverlayKind::Sheet => Frame::new(
            screen.x + (screen.w - width) / 2.0,
            screen.bottom() - height - space::S4,
            width,
            height,
        ),
    };
    let text = Frame::new(
        panel.x + pad.x,
        panel.y + pad.top,
        inner,
        title_h + words.after_title + body_h + verified_h,
    );
    let close_frame = close.then(|| {
        Frame::new(
            panel.right() - pad.x - CLOSE,
            panel.y + pad.top,
            CLOSE,
            CLOSE,
        )
    });

    let mut y = text.bottom() + if options_h > 0.0 { space::S4 } else { 0.0 };
    let mut options = Vec::new();
    if let Some(o) = &model.options {
        for _ in &o.items {
            options.push(Frame::new(panel.x + pad.x, y, inner, smart::TARGET));
            y += smart::TARGET + OPTION_GAP;
        }
    }
    let mut actions = Vec::new();
    if actions_h > 0.0 {
        let top = panel.bottom() - pad.bottom - actions_h;
        if stacked {
            for i in 0..model.actions.len() {
                actions.push(Frame::new(
                    panel.x + pad.x,
                    top + i as f32 * (ACTION_H + space::S2),
                    inner,
                    ACTION_H,
                ));
            }
        } else {
            let mut right = panel.right() - pad.x;
            for &w in widths.iter().rev() {
                actions.push(Frame::new(right - w, top, w, ACTION_H));
                right -= w + words.action_gap;
            }
            actions.reverse();
        }
    }
    Layout {
        panel,
        title,
        body,
        text,
        close: close_frame,
        options,
        actions,
        confirm: model.confirm,
        verified: verified_line.map(|line| {
            (
                line,
                title_h + words.after_title + body_h + words.before_verified,
            )
        }),
    }
}

fn button_kind(kind: ActionKind) -> ButtonKind {
    match kind {
        ActionKind::Primary => ButtonKind::Primary,
        ActionKind::Normal | ActionKind::Destructive => ButtonKind::Standard,
    }
}

fn action_model(a: &OverlayAction) -> ButtonModel {
    ButtonModel::labelled(&a.label).enabled(a.enabled)
}

/// What a contact is doing to the overlay.
#[derive(Clone, Copy, Debug)]
enum Contact {
    None,
    /// The contact began on the scrim: a dismissal if it ends there.
    Scrim {
        id: u32,
    },
    /// A control of the panel owns it (the router says which).
    Control,
    /// It began on the panel's own surface and does nothing.
    Panel {
        id: u32,
    },
}

/// The title's lines in the ink and the body's in the second ink (a confirmation's in the ink too), one under the other with the
/// close button's room kept, and the verified line under them after its tick.
fn text_runs<'a>(ui: &Ui, layout: &'a Layout) -> Vec<Run<'a>> {
    let words = Words::of(ui, layout.confirm);
    let (tt, bt, ct) = (words.title, words.body, words.verified);
    let (ink, ink2) = (ui.look.ink(), ui.look.ink2());
    let body_ink = if layout.confirm { ink } else { ink2 };
    let w = layout.text.w;
    let title_w = if layout.close.is_some() {
        w - CLOSE - space::S3
    } else {
        w
    };
    let mut runs = Vec::new();
    let mut y = 0.0;
    for line in &layout.title {
        runs.push(Run::left(
            line,
            tt,
            ink,
            0.0,
            Words::run_y(y, words.title_line, tt),
            title_w,
        ));
        y += words.title_line;
    }
    y = y.max(if layout.close.is_some() { CLOSE } else { 0.0 }) + words.after_title;
    for line in &layout.body {
        runs.push(Run::left(
            line,
            bt,
            body_ink,
            0.0,
            Words::run_y(y, words.body_line, bt),
            w,
        ));
        y += words.body_line;
    }
    if let Some((line, at)) = &layout.verified {
        runs.push(Run::left(
            line,
            ct,
            ink2,
            TICK_SLOT,
            Words::run_y(*at, words.verified_line, ct),
            (w - TICK_SLOT).max(0.0),
        ));
    }
    runs
}

pub struct Overlay {
    kind: OverlayKind,
    screen: Frame,
    model: OverlayModel,
    layout: Layout,
    scrim: ShapeLayer,
    /// The panel's glass and its words.
    panel: Group,
    /// The panel's focus ring: a panel is never focused, so a pose never shows it.
    ring: usize,
    text: usize,
    close: Option<Button>,
    options: Vec<Row>,
    actions: Vec<Button>,
    shown: bool,
    /// When an exit that has begun is over.
    exit_until: Option<u64>,
    contact: Contact,
    router: Router,
}

impl Overlay {
    /// The overlay for `model` over `screen`, hidden: show it with [`Overlay::show`]. A popup may list at most [`MAX_OPTIONS`] options
    /// and needs at least one; every other model is shown whole.
    pub fn new(
        ui: &mut Ui,
        screen: Frame,
        z: i32,
        kind: OverlayKind,
        model: &OverlayModel,
    ) -> io::Result<Self> {
        if let Some(o) = &model.options {
            if o.items.is_empty() || o.items.len() > MAX_OPTIONS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "a popup lists 1 to {MAX_OPTIONS} options, not {}",
                        o.items.len()
                    ),
                ));
            }
        }
        let layout = layout(ui, screen, kind, model);
        let scrim = shape(ui, screen, z, solid(ui.look.scrim(), 0.0));
        let mut panel = Group::new(layout.panel.centre());
        let ring = Surface::add(
            ui,
            &mut panel,
            layout.panel,
            z + 1,
            SmartMaterial::Overlay,
            MaterialContext::Independent,
            ControlState::default(),
        )
        .ring_part();
        let runs = text_runs(ui, &layout);
        let text_layer = TextLayer::new(
            ui,
            layout.text,
            z + 2,
            &runs.iter().copied().map(Item::Text).collect::<Vec<_>>(),
        )?;
        let text = panel.push(Part::new(text_layer, layout.text));
        // The tick before the verified line.
        if let Some((_, at)) = &layout.verified {
            let line = Words::of(ui, layout.confirm).verified_line;
            let tick = Frame::new(
                layout.text.x,
                layout.text.y + at + (line - TICK) / 2.0,
                TICK,
                TICK,
            );
            let ink = ui.look.colour(PaletteRole::Positive);
            panel.push(icon_part(ui, Icon::Check, tick, z + 2, ink)?);
        }
        let close = match layout.close {
            Some(at) => Some(Button::icon_button(
                ui,
                at,
                z + 2,
                MaterialContext::Nested,
                &ButtonModel::icon(Icon::Close, "Close"),
            )?),
            None => None,
        };
        let mut options = Vec::new();
        for (i, at) in layout.options.iter().enumerate() {
            options.push(Row::new(ui, *at, z + 2, &Self::option_model(model, i))?);
        }
        let mut actions = Vec::new();
        for (a, at) in model.actions.iter().zip(&layout.actions) {
            let mut b = Button::new(
                ui,
                *at,
                z + 2,
                button_kind(a.kind),
                MaterialContext::Nested,
                &action_model(a),
            )?;
            if a.kind == ActionKind::Destructive {
                b.set_ink(ui, Some(PaletteRole::Danger));
            }
            actions.push(b);
        }
        let mut overlay = Self {
            kind,
            screen,
            model: model.clone(),
            layout,
            scrim,
            panel,
            ring,
            text,
            close,
            options,
            actions,
            shown: false,
            exit_until: None,
            contact: Contact::None,
            router: Router::default(),
        };
        overlay.pose_all(Pose::hidden(), None);
        Ok(overlay)
    }

    fn option_model(model: &OverlayModel, i: usize) -> RowModel {
        let o = model.options.as_ref().expect("options");
        let (id, label) = &o.items[i];
        RowModel::new(RowId(i as u64), label)
            .trailing(Trailing::Check(o.selected.as_deref() == Some(id.as_str())))
            .selected(o.selected.as_deref() == Some(id.as_str()))
            .actionable()
    }

    /// The pose of everything when the overlay is away: a dialog a little low and faded, a sheet fully below the screen's edge.
    fn away(kind: OverlayKind, panel: Frame, ui: &Ui) -> Pose {
        if !ui.look.moves() {
            return Pose::fade(0.0);
        }
        match kind {
            OverlayKind::Dialog => Pose::slid(0.0, RISE, 0.0),
            OverlayKind::Sheet => Pose::slid(0.0, panel.h + space::S4, 0.0),
        }
    }

    /// Puts the panel, its words, its buttons and the scrim at the pose of shown (`REST`) or away, at once, or over `ms`.
    fn pose_all(&mut self, pose: Pose, ms: Option<u32>) {
        let shown = pose == Pose::REST;
        let scrim_to = Props {
            opacity: if shown { 1.0 } else { 0.0 },
            ..Props::at(self.screen.x, self.screen.y)
        };
        match ms {
            Some(ms) => {
                self.scrim.animate(scrim_to, ms, motion::SETTLE);
                self.panel
                    .animate_in(self.panel.about, pose, &[self.ring], ms, motion::SETTLE);
                for b in self.close.iter_mut().chain(self.actions.iter_mut()) {
                    b.animate_pose(pose, ms, motion::SETTLE);
                }
                for r in &mut self.options {
                    r.animate_pose(pose, ms, motion::SETTLE);
                }
            }
            None => {
                self.scrim.set(Props {
                    visible: shown,
                    ..scrim_to
                });
                let pose = if shown { pose } else { Pose::hidden() };
                self.panel.set_in(self.panel.about, pose, &[self.ring]);
                for b in self.close.iter_mut().chain(self.actions.iter_mut()) {
                    b.set_pose(pose);
                }
                for r in &mut self.options {
                    r.set_pose(pose);
                }
            }
        }
    }

    fn draw_text(&mut self, ui: &mut Ui) {
        let runs = text_runs(ui, &self.layout);
        if let Some(layer) = self.panel.parts[self.text].lines_mut() {
            layer.draw_runs(ui, &runs);
        }
    }

    pub fn kind(&self) -> OverlayKind {
        self.kind
    }

    pub fn model(&self) -> &OverlayModel {
        &self.model
    }

    /// Where the panel is when shown.
    pub fn panel(&self) -> Frame {
        self.layout.panel
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// The lines of the title and of the body as drawn.
    pub fn lines(&self) -> (&[String], &[String]) {
        (&self.layout.title, &self.layout.body)
    }

    /// The action buttons, in the order of the model.
    pub fn buttons(&self) -> &[Button] {
        &self.actions
    }

    pub fn close_button(&self) -> Option<&Button> {
        self.close.as_ref()
    }

    pub fn option_rows(&self) -> &[Row] {
        &self.options
    }

    /// The height the overlay's panel needs for `model` on `screen` at the person's text size.
    pub fn required_height(
        ui: &mut Ui,
        screen: Frame,
        kind: OverlayKind,
        model: &OverlayModel,
    ) -> f32 {
        layout(ui, screen, kind, model).panel.h
    }

    /// Brings the overlay in or sends it away. With `Some(now_ns)` it moves over the layer (or exit) duration on the compositor's
    /// clock, and [`Overlay::tick`] puts it away for good once an exit is over; with `None` it changes at once. Hidden, it takes no
    /// contact and describes nothing.
    pub fn show(&mut self, ui: &Ui, shown: bool, now_ns: Option<u64>) {
        if self.shown == shown && self.exit_until.is_none() {
            return;
        }
        self.shown = shown;
        self.exit_until = None;
        self.cancel(ui);
        let Some(now) = now_ns else {
            self.pose_all(if shown { Pose::REST } else { Pose::hidden() }, None);
            return;
        };
        if shown {
            // From away to rest: the layers start where an entrance starts, visible and transparent.
            let away = Self::away(self.kind, self.layout.panel, ui);
            self.scrim.set(Props {
                opacity: 0.0,
                ..Props::at(self.screen.x, self.screen.y)
            });
            self.panel.set_in(self.panel.about, away, &[self.ring]);
            for b in self.close.iter_mut().chain(self.actions.iter_mut()) {
                b.set_pose(away);
            }
            for r in &mut self.options {
                r.set_pose(away);
            }
            self.pose_all(Pose::REST, Some(ui.look.duration_ms(motion::LAYER_MS)));
        } else {
            let ms = ui.look.duration_ms(motion::EXIT_MS);
            let away = Self::away(self.kind, self.layout.panel, ui);
            self.pose_all(away, Some(ms));
            self.exit_until = Some(now + u64::from(ms) * 1_000_000);
        }
    }

    /// Puts an overlay that has finished leaving away for good (its layers stop being drawn).
    pub fn tick(&mut self, now_ns: u64) {
        if self.exit_until.is_some_and(|t| now_ns >= t) {
            self.exit_until = None;
            self.pose_all(Pose::hidden(), None);
        }
    }

    /// Whether the overlay is on its way out.
    pub fn is_leaving(&self) -> bool {
        self.exit_until.is_some()
    }

    /// Ends a press that something else has taken (a hidden overlay lets go of everything).
    pub fn cancel(&mut self, ui: &Ui) {
        self.router.owner = None;
        self.contact = Contact::None;
        for b in self.close.iter_mut().chain(self.actions.iter_mut()) {
            b.cancel(ui);
        }
        for r in &mut self.options {
            r.cancel(ui);
        }
    }

    /// The look was replaced: restyle every surface and draw the words and the check marks again in the new colours.
    pub fn restyle(&mut self, ui: &mut Ui) {
        self.scrim.reshape(solid(ui.look.scrim(), 0.0));
        self.restyle_panel(ui);
        for b in self.close.iter_mut().chain(self.actions.iter_mut()) {
            b.restyle(ui);
        }
        for (b, a) in self.actions.iter_mut().zip(&self.model.actions) {
            if a.kind == ActionKind::Destructive {
                b.set_ink(ui, Some(PaletteRole::Danger));
            }
        }
        for r in &mut self.options {
            r.restyle(ui);
        }
        self.draw_text(ui);
    }

    fn restyle_panel(&mut self, ui: &Ui) {
        let pair = ui
            .look
            .material(
                SmartMaterial::Overlay,
                &ControlState::default(),
                MaterialContext::Independent,
            )
            .sized(self.layout.panel.w, self.layout.panel.h);
        if let Some(g) = self.panel.parts[0].glass() {
            g.restyle(pair.rest);
        }
    }

    fn controls(&self) -> impl Iterator<Item = Frame> + '_ {
        self.close
            .iter()
            .map(Button::hit)
            .chain(self.actions.iter().map(Button::hit))
            .chain(self.options.iter().map(Row::hit))
    }

    fn feed(&mut self, index: usize, event: &Event, ui: &mut Ui) -> Option<OverlayResult> {
        let close = usize::from(self.close.is_some());
        if index < close {
            let hit = self.close.as_mut()?.input(event, ui)?;
            return (hit == ButtonAction::Activated).then_some(OverlayResult::Dismissed);
        }
        let index = index - close;
        if index < self.actions.len() {
            let hit = self.actions[index].input(event, ui)?;
            return (hit == ButtonAction::Activated)
                .then(|| OverlayResult::Action(self.model.actions[index].id.clone()));
        }
        let option = index - self.actions.len();
        let action = self.options[option].input(event, ui)?;
        let id = &self.model.options.as_ref()?.items[option].0;
        (action == RowAction::Activated).then(|| OverlayResult::Chosen(id.clone()))
    }
}

impl Component for Overlay {
    type Model = OverlayModel;
    type Action = OverlayResult;

    /// Another model of the same shape (same actions, same options, same amount of words) is shown in place; anything else is a
    /// new overlay, which the caller makes with [`Overlay::new`].
    fn update(&mut self, model: &OverlayModel, ui: &mut Ui) -> Damage {
        if *model == self.model {
            return Damage::None;
        }
        let same_shape = model.actions.len() == self.model.actions.len()
            && model.dismissal == self.model.dismissal
            && model.confirm == self.model.confirm
            && model.verified.is_some() == self.model.verified.is_some()
            && model.options.as_ref().map(|o| o.items.len())
                == self.model.options.as_ref().map(|o| o.items.len());
        if !same_shape {
            return Damage::None;
        }
        let next = layout(ui, self.screen, self.kind, model);
        if next.panel != self.layout.panel
            || next.title.len() != self.layout.title.len()
            || next.body.len() != self.layout.body.len()
        {
            return Damage::None;
        }
        self.layout = next;
        let mut damage = Damage::Content;
        for (b, (a, old)) in self
            .actions
            .iter_mut()
            .zip(model.actions.iter().zip(&self.model.actions))
        {
            if a.kind != old.kind {
                return Damage::None;
            }
            damage = damage.max(b.update(&action_model(a), ui));
            if a.kind == ActionKind::Destructive {
                b.set_ink(ui, Some(PaletteRole::Danger));
            }
        }
        for i in 0..self.options.len() {
            let row = Self::option_model(model, i);
            self.options[i].update(&row, ui);
        }
        self.model = model.clone();
        self.draw_text(ui);
        damage
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<OverlayResult> {
        let Event::Touch(t) = event else {
            return None;
        };
        if !self.shown {
            return None;
        }
        // A contact that a control began is the control's until it ends.
        if matches!(self.contact, Contact::Control)
            || (t.phase == TouchPhase::Down && matches!(self.contact, Contact::None))
        {
            let frames: Vec<Frame> = self.controls().collect();
            if let Some(i) = self.router.route(frames.into_iter(), t) {
                self.contact = if matches!(t.phase, TouchPhase::Up | TouchPhase::Cancel) {
                    Contact::None
                } else {
                    Contact::Control
                };
                return self.feed(i, event, ui);
            }
        }
        match (t.phase, self.contact) {
            (TouchPhase::Down, Contact::None) => {
                self.contact = if self.layout.panel.contains(t.x, t.y) {
                    Contact::Panel { id: t.id }
                } else {
                    Contact::Scrim { id: t.id }
                };
                None
            }
            (TouchPhase::Up, Contact::Scrim { id }) if id == t.id => {
                self.contact = Contact::None;
                let outside = !self.layout.panel.contains(t.x, t.y);
                (outside
                    && matches!(
                        self.model.dismissal,
                        Dismissal::ScrimOrClose | Dismissal::Scrim
                    ))
                .then_some(OverlayResult::Dismissed)
            }
            (
                TouchPhase::Up | TouchPhase::Cancel,
                Contact::Scrim { id } | Contact::Panel { id },
            ) if id == t.id => {
                self.contact = Contact::None;
                None
            }
            _ => None,
        }
    }

    /// The whole screen while shown: nothing underneath takes a touch. Nothing when hidden.
    fn frame(&self) -> Frame {
        if self.shown {
            self.screen
        } else {
            Frame::default()
        }
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        if !self.shown {
            return;
        }
        let kind = match self.kind {
            OverlayKind::Dialog => SemanticKind::Dialog,
            OverlayKind::Sheet => SemanticKind::Sheet,
        };
        out.push(
            SemanticNode::new(kind, self.model.title.clone(), self.layout.panel).value(match &self
                .model
                .verified
            {
                Some(line) if self.model.body.is_empty() => line.clone(),
                Some(line) => format!("{}\n{line}", self.model.body),
                None => self.model.body.clone(),
            }),
        );
        if let Some(c) = &self.close {
            c.semantics(out);
        }
        for r in &self.options {
            r.semantics(out);
        }
        for b in &self.actions {
            b.semantics(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Look;
    use crate::input::TouchPhase::{Cancel, Down, Move, Up};
    use crate::lucent::palette::Mode;
    use crate::testing::{
        Harness, count_requests, harness, made_as_picture, pictures_made, pixels_damaged_in, touch,
        warm_glyph_harness,
    };
    use compositor_protocol::{Content, LayerId, Request};

    const SCREEN: Frame = Frame::new(0.0, 0.0, 960.0, 480.0);

    fn confirm() -> OverlayModel {
        OverlayModel::new(
            "Forget this network?",
            "The password is removed and the Show will not join it again.",
        )
        .action(OverlayAction::new("cancel", "Cancel", ActionKind::Normal))
        .action(OverlayAction::new(
            "forget",
            "Forget",
            ActionKind::Destructive,
        ))
    }

    fn overlay(h: &mut Harness, kind: OverlayKind, model: &OverlayModel) -> Overlay {
        let mut o = Overlay::new(&mut h.ui, SCREEN, 50, kind, model).unwrap();
        o.show(&h.ui, true, None);
        h.commit();
        o
    }

    fn text_id(o: &Overlay) -> LayerId {
        o.panel.parts[o.text].layer().id()
    }

    fn is_run(o: &mut Overlay) -> bool {
        o.panel.parts[o.text]
            .lines_mut()
            .is_some_and(|l| l.is_glyph_run())
    }

    /// A dialog and a sheet, each with a title that wraps (and a close button beside it), a body and two buttons; and a dialog
    /// with a title only.
    fn looks() -> [(OverlayKind, OverlayModel); 3] {
        [
            (OverlayKind::Dialog, confirm()),
            (OverlayKind::Sheet, confirm()),
            (
                OverlayKind::Dialog,
                OverlayModel::new("Discard changes to this alarm and go back to the list?", "")
                    .action(OverlayAction::new("ok", "Discard", ActionKind::Normal)),
            ),
        ]
    }

    fn warm(kind: OverlayKind, model: &OverlayModel) -> Harness {
        warm_glyph_harness(|h| overlay(h, kind, model))
    }

    #[test]
    fn an_overlays_words_are_the_same_pixels_as_glyph_quads_and_as_a_picture_in_every_look() {
        for (kind, model) in looks() {
            let mut run = warm(kind, &model);
            let mut glyphs = overlay(&mut run, kind, &model);
            assert!(is_run(&mut glyphs), "{:?}", model.title);
            let mut pic = harness();
            let mut picture = overlay(&mut pic, kind, &model);
            let (a, b) = (
                run.layer_pixels(text_id(&glyphs)),
                pic.layer_pixels(text_id(&picture)),
            );
            assert!(a.iter().any(|v| *v != 0), "{:?}", model.title);
            assert_eq!(a, b, "{kind:?} {:?}", model.title);
            // The light look draws the title and the body in other inks, again as the picture does.
            for h in [&mut run, &mut pic] {
                h.ui.look = Look::standard(Mode::Light);
            }
            glyphs.restyle(&mut run.ui);
            picture.restyle(&mut pic.ui);
            run.commit();
            pic.commit();
            let (c, d) = (
                run.layer_pixels(text_id(&glyphs)),
                pic.layer_pixels(text_id(&picture)),
            );
            assert_ne!(a, c, "the ink did not change");
            assert_eq!(c, d, "{kind:?} {:?} in the light look", model.title);
        }
    }

    #[test]
    fn on_glyph_text_an_overlays_words_make_no_picture_and_write_no_pixel_and_no_layer_more() {
        let creates =
            |sent: &[Request]| count_requests(sent, |r| matches!(r, Request::Create { .. }));
        for (kind, model) in looks() {
            let mut run = warm(kind, &model);
            let mut glyphs = Overlay::new(&mut run.ui, SCREEN, 50, kind, &model).unwrap();
            let sent = run.commit();
            assert!(is_run(&mut glyphs), "{:?}", model.title);
            let text = text_id(&glyphs);
            assert!(!made_as_picture(&sent, text), "{:?}", model.title);
            assert_eq!(pixels_damaged_in(&sent, text), 0.0, "{:?}", model.title);
            // The only picture is the close button's cross, an icon of its own.
            assert_eq!(
                pictures_made(&sent),
                usize::from(glyphs.close_button().is_some()),
                "{:?}: {sent:?}",
                model.title
            );
            assert_eq!(run.ui.glyph_text_fallbacks, 0);
            let mut pic = harness();
            let _picture = Overlay::new(&mut pic.ui, SCREEN, 50, kind, &model).unwrap();
            let as_pictures = pic.commit();
            assert!(creates(&sent) <= creates(&as_pictures));
            assert!(pictures_made(&as_pictures) > pictures_made(&sent));
        }
    }

    #[test]
    fn on_glyph_text_a_new_look_recolours_an_overlays_words_with_one_restyle_and_no_pixel() {
        let model = confirm();
        let mut run = warm(OverlayKind::Dialog, &model);
        let mut o = overlay(&mut run, OverlayKind::Dialog, &model);
        run.ui.look = Look::standard(Mode::Light);
        o.restyle(&mut run.ui);
        let sent = run.commit();
        assert_eq!(pixels_damaged_in(&sent, text_id(&o)), 0.0);
        assert!(!made_as_picture(&sent, text_id(&o)));
        // The title is in the ink and the body in the second ink: one restyle of the text layer carries both.
        let restyles = |sent: &[Request], id: LayerId| {
            count_requests(sent, |r| {
                matches!(
                    r,
                    Request::Restyle { id: hit, content: Content::Glyphs(_) } if *hit == id
                )
            })
        };
        assert_eq!(restyles(&sent, text_id(&o)), 1, "{sent:?}");
        assert_eq!(run.ui.glyph_text_fallbacks, 0);
    }

    fn tap(o: &mut Overlay, h: &mut Harness, (x, y): (f32, f32)) -> Option<OverlayResult> {
        o.input(&touch(1, Down, x, y, 0), &mut h.ui);
        o.input(&touch(1, Up, x, y, 40), &mut h.ui)
    }

    /// A panel is never focused: showing the overlay (at once or by its entrance) must not light the panel's focus ring, which a
    /// pose over every part of the group would.
    #[test]
    fn showing_an_overlay_never_lights_its_panels_focus_ring() {
        for animated in [false, true] {
            let mut h = harness();
            let mut o =
                Overlay::new(&mut h.ui, SCREEN, 50, OverlayKind::Dialog, &confirm()).unwrap();
            o.show(&h.ui, true, animated.then_some(1_000_000));
            h.commit();
            let ring = o.panel.parts[o.ring]
                .shape()
                .expect("the ring is a shape")
                .id();
            let shown = h
                .layers()
                .into_iter()
                .find(|(id, _, _)| *id == ring)
                .expect("the ring layer")
                .2;
            assert!(
                !shown.visible,
                "animated={animated}: the focus ring of an unfocused panel is shown"
            );
            let panel = o.panel.parts[0].glass().expect("the panel's glass").id();
            assert!(
                h.layers()
                    .into_iter()
                    .find(|(id, _, _)| *id == panel)
                    .unwrap()
                    .2
                    .visible,
                "the panel itself is"
            );
        }
    }

    #[test]
    fn an_action_answers_with_its_id_and_the_close_button_and_the_scrim_dismiss() {
        let mut h = harness();
        let mut o = overlay(&mut h, OverlayKind::Dialog, &confirm());
        let forget = o.buttons()[1].hit().centre();
        assert_eq!(
            tap(&mut o, &mut h, forget),
            Some(OverlayResult::Action("forget".into()))
        );
        let close = o.close_button().unwrap().hit().centre();
        assert_eq!(tap(&mut o, &mut h, close), Some(OverlayResult::Dismissed));
        assert_eq!(
            tap(&mut o, &mut h, (10.0, 10.0)),
            Some(OverlayResult::Dismissed),
            "a contact begun and ended on the scrim"
        );
        // Pressed on the scrim and lifted on the panel: nothing.
        o.input(&touch(1, Down, 10.0, 10.0, 0), &mut h.ui);
        let (cx, cy) = o.panel().centre();
        assert_eq!(o.input(&touch(1, Up, cx, cy - 10.0, 30), &mut h.ui), None);
        // Pressed on the panel's own surface: nothing, and not a dismissal.
        let top = o.panel().y + 6.0;
        assert_eq!(tap(&mut o, &mut h, (cx, top)), None);
    }

    #[test]
    fn it_is_modal_it_swallows_every_contact_and_a_hidden_one_takes_none() {
        let mut h = harness();
        let mut o = overlay(&mut h, OverlayKind::Dialog, &confirm());
        assert_eq!(
            o.frame(),
            SCREEN,
            "a host sends everything to it while it is shown"
        );
        assert_eq!(o.input(&touch(1, Down, 900.0, 440.0, 0), &mut h.ui), None);
        o.cancel(&h.ui);
        o.show(&h.ui, false, None);
        assert_eq!(o.frame(), Frame::default());
        let before = h.all().len();
        let centre = o.panel().centre();
        assert_eq!(tap(&mut o, &mut h, centre), None);
        let forget = o.buttons()[1].hit().centre();
        assert_eq!(tap(&mut o, &mut h, forget), None);
        assert_eq!(h.all().len(), before, "nothing was asked of the compositor");
        let mut sink = SemanticsSink::new();
        o.semantics(&mut sink);
        assert!(
            sink.nodes().is_empty(),
            "hidden content is not in the semantic tree"
        );
    }

    #[test]
    fn dismissal_is_what_the_model_allows_and_an_action_only_overlay_has_no_close_button() {
        let mut h = harness();
        let must = confirm().dismissal(Dismissal::ActionOnly);
        let mut o = overlay(&mut h, OverlayKind::Dialog, &must);
        assert!(o.close_button().is_none());
        assert_eq!(
            tap(&mut o, &mut h, (10.0, 10.0)),
            None,
            "the scrim does not dismiss it"
        );
        let close_only = confirm().dismissal(Dismissal::CloseOnly);
        let mut o = overlay(&mut h, OverlayKind::Dialog, &close_only);
        assert_eq!(tap(&mut o, &mut h, (10.0, 10.0)), None);
        let close = o.close_button().unwrap().hit().centre();
        assert_eq!(tap(&mut o, &mut h, close), Some(OverlayResult::Dismissed));
        // Every dismissible one has a visible way out.
        let sheet = overlay(&mut h, OverlayKind::Sheet, &confirm());
        assert!(sheet.close_button().is_some());
    }

    #[test]
    fn a_drag_off_a_button_and_a_cancel_answer_nothing() {
        let mut h = harness();
        let mut o = overlay(&mut h, OverlayKind::Dialog, &confirm());
        let (x, y) = o.buttons()[1].hit().centre();
        o.input(&touch(1, Down, x, y, 0), &mut h.ui);
        o.input(&touch(1, Move, x, y - 200.0, 16), &mut h.ui);
        assert_eq!(o.input(&touch(1, Up, x, y - 200.0, 32), &mut h.ui), None);
        o.input(&touch(1, Down, x, y, 100), &mut h.ui);
        assert_eq!(o.input(&touch(1, Cancel, 0.0, 0.0, 110), &mut h.ui), None);
        o.input(&touch(1, Down, x, y, 200), &mut h.ui);
        o.cancel(&h.ui);
        assert_eq!(o.input(&touch(1, Up, x, y, 210), &mut h.ui), None);
    }

    #[test]
    fn a_popup_chooses_one_of_its_options_and_marks_the_chosen_one_with_a_check() {
        let mut h = harness();
        let model = OverlayModel::new("Clock face", "").options(
            vec![
                ("quiet".into(), "Quiet digital".into()),
                ("compact".into(), "Compact".into()),
                ("analog".into(), "Analog".into()),
            ],
            Some("compact"),
        );
        let mut o = overlay(&mut h, OverlayKind::Dialog, &model);
        let rows: Vec<_> = o.option_rows().iter().map(|r| r.hit().centre()).collect();
        assert_eq!(
            tap(&mut o, &mut h, rows[2]),
            Some(OverlayResult::Chosen("analog".into()))
        );
        assert_eq!(
            o.model().options.as_ref().unwrap().selected.as_deref(),
            Some("compact"),
            "a choice is proposed, not applied"
        );
        let mut sink = SemanticsSink::new();
        o.semantics(&mut sink);
        assert!(sink.find("Compact").unwrap().selected);
        assert!(!sink.find("Analog").unwrap().selected);
        let none = OverlayModel::new("x", "").options(Vec::new(), None);
        assert!(Overlay::new(&mut h.ui, SCREEN, 50, OverlayKind::Dialog, &none).is_err());
        let many = OverlayModel::new("x", "").options(
            (0..7)
                .map(|i| (i.to_string(), format!("Option {i}")))
                .collect(),
            None,
        );
        assert!(Overlay::new(&mut h.ui, SCREEN, 50, OverlayKind::Dialog, &many).is_err());
    }

    #[test]
    fn the_panel_is_the_overlay_material_with_its_raised_pair_and_one_shadow_in_dark_and_light() {
        for mode in [
            crate::lucent::palette::Mode::Dark,
            crate::lucent::palette::Mode::Light,
        ] {
            let mut h = harness();
            h.ui.look = crate::context::Look::standard(mode);
            let _o = overlay(&mut h, OverlayKind::Dialog, &confirm());
            let panel = h
                .layers()
                .into_iter()
                .filter_map(|(_, c, _)| match c {
                    Content::Glass(g) if g.corner_radius >= 27.0 => Some(g),
                    _ => None,
                })
                .next()
                .unwrap_or_else(|| panic!("{mode:?}: no panel glass"));
            assert_eq!(panel.tint, h.ui.look.theme.materials.overlay, "{mode:?}");
            assert!(
                panel.tint.3 >= 245,
                "{mode:?}: the reading surface is at least .96 opaque: {}",
                panel.tint.3
            );
            assert!(
                panel.depth.is_some_and(|d| !d.inset) && panel.sheen.is_some(),
                "{mode:?}"
            );
            assert!(
                panel.shadows[0].is_some() && panel.shadows[1].is_none(),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn large_text_makes_the_panel_taller_stacks_the_actions_and_cuts_the_body_last_never_the_type()
    {
        let mut h = harness();
        let model = OverlayModel::new("Install the update and restart?", "The Show installs the new system, checks it and restarts. It takes about two minutes, and it goes back by itself if the new system does not start.")
            .action(OverlayAction::new("later", "Not now", ActionKind::Normal))
            .action(OverlayAction::new("go", "Install and restart", ActionKind::Primary));
        let normal = Overlay::required_height(&mut h.ui, SCREEN, OverlayKind::Dialog, &model);
        h.ui.look.text_scale = 2.0;
        let large = Overlay::required_height(&mut h.ui, SCREEN, OverlayKind::Dialog, &model);
        assert!(large > normal, "{normal} -> {large}");
        assert!(
            large <= SCREEN.h - 2.0 * space::S4 + 0.01,
            "it fits the screen: {large}"
        );
        let o = overlay(&mut h, OverlayKind::Dialog, &model);
        let (title, body) = o.lines();
        assert!(!title.is_empty());
        assert!(body.last().is_none_or(|l| !l.is_empty()));
        // At that size the buttons do not fit side by side: each has the panel's width, one under another.
        let (a, b) = (o.buttons()[0].hit(), o.buttons()[1].hit());
        assert!(
            a.bottom() <= b.y + 8.0 && b.bottom() <= o.panel().bottom() + 8.0,
            "{a:?} {b:?} in {:?}",
            o.panel()
        );
        assert!(o.panel().y >= 0.0 && o.panel().bottom() <= SCREEN.h);
    }

    #[test]
    fn showing_and_hiding_are_poses_with_no_layer_made_and_a_leaving_overlay_is_put_away_after_its_exit()
     {
        let mut h = harness();
        let mut o = Overlay::new(&mut h.ui, SCREEN, 50, OverlayKind::Sheet, &confirm()).unwrap();
        h.commit();
        let layers = h.layers().len();
        o.show(&h.ui, true, Some(1_000_000_000));
        let sent = h.commit();
        assert!(sent.iter().any(|r| matches!(r, Request::Animate { .. })));
        assert!(
            !sent
                .iter()
                .any(|r| matches!(r, Request::Create { .. } | Request::Damage { .. }))
        );
        assert!(o.is_shown());
        o.show(&h.ui, false, Some(2_000_000_000));
        assert!(o.is_leaving() && !o.is_shown());
        assert_eq!(
            o.input(&touch(1, Down, 10.0, 10.0, 2_000), &mut h.ui),
            None,
            "nothing is taken while it leaves"
        );
        o.tick(2_000_000_000 + 10_000_000);
        assert!(o.is_leaving(), "not yet");
        o.tick(3_000_000_000);
        assert!(!o.is_leaving());
        h.commit();
        assert_eq!(h.layers().len(), layers);
        // Reduced motion fades and does not travel.
        h.ui.look.reduced_motion = true;
        let mut quiet =
            Overlay::new(&mut h.ui, SCREEN, 50, OverlayKind::Dialog, &confirm()).unwrap();
        quiet.show(&h.ui, true, Some(0));
        let sent = h.commit();
        let moved = sent.iter().any(|r| matches!(r, Request::Animate { to, .. } if (to.y - quiet.panel().y).abs() > 0.5 && to.opacity > 0.0 && to.opacity < 1.0));
        assert!(!moved, "no travel with reduced motion");
    }

    #[test]
    fn a_disabled_action_cannot_be_pressed_and_a_destructive_one_is_in_the_danger_colour() {
        let mut h = harness();
        let mut model = confirm();
        model.actions[0].enabled = false;
        let mut o = overlay(&mut h, OverlayKind::Dialog, &model);
        let cancel = o.buttons()[0].hit().centre();
        assert_eq!(tap(&mut o, &mut h, cancel), None);
        let danger = h.ui.look.danger();
        let mut forget = overlay(&mut h, OverlayKind::Dialog, &confirm());
        assert_eq!(
            forget.actions[0].label_colour(),
            Some(h.ui.look.ink()),
            "a normal action keeps the primary ink"
        );
        let label = forget.actions[1].label_colour();
        assert_eq!(label, Some(danger));
        // Every action is a button with the raised pair.
        for b in forget.buttons() {
            assert!(b.surface().shown().0.depth.is_some_and(|d| !d.inset));
        }
    }

    #[test]
    fn a_confirmation_of_the_settings_is_the_mocks_560_wide_has_no_close_button_a_verified_line_and_a_touch_outside_dismisses_it()
     {
        let mut h = harness();
        let line = "Signed by lumashow \u{b7} sha256 4f2a\u{2026}c91e";
        let model = confirm().confirmation().verified(line);
        let mut o = overlay(&mut h, OverlayKind::Dialog, &model);
        assert_eq!(
            o.panel().w,
            560.0,
            "the mock's width; the other dialogs are 600"
        );
        assert!(o.close_button().is_none(), "no Close button");
        // The verified line is told to a person who cannot see its tick, after the words it belongs to.
        let mut sink = SemanticsSink::new();
        o.semantics(&mut sink);
        let dialog = sink
            .nodes()
            .iter()
            .find(|n| n.name == "Forget this network?")
            .expect("the dialog");
        assert!(
            dialog.value.as_deref().unwrap().ends_with(line),
            "{:?}",
            dialog.value
        );
        // The line and its gap make the panel taller by that much and nothing else.
        let plain = overlay(&mut h, OverlayKind::Dialog, &confirm().confirmation());
        assert!(o.panel().h > plain.panel().h + 20.0);
        // The mock's `.dv`: 14 above it and a line of 1.25 of its 20 px.
        assert_eq!(
            o.panel().h - plain.panel().h,
            14.0 + h.ui.look.ty(ty::CAPTION).px as f32 * 1.25
        );
        // A touch that begins and ends on the scrim dismisses it, with no Close button to do it.
        assert_eq!(
            tap(&mut o, &mut h, (10.0, 10.0)),
            Some(OverlayResult::Dismissed)
        );
        // Cancel and the action answer as they do in any dialog.
        let mut o = overlay(&mut h, OverlayKind::Dialog, &model);
        let forget = o.buttons()[1].hit().centre();
        assert_eq!(
            tap(&mut o, &mut h, forget),
            Some(OverlayResult::Action("forget".into()))
        );
        // The panel is made of the same layers as any other (the tick is a layer of its own, which the others do not have).
        let others = overlay(&mut h, OverlayKind::Dialog, &confirm());
        assert_eq!(o.panel().x, (960.0 - 560.0) / 2.0);
        assert!(others.panel().w > o.panel().w);
    }
}

#[cfg(test)]
mod confirmation_look_tests {
    use super::*;
    use crate::context::Look;
    use crate::lucent::palette::Mode;
    use crate::lucent::tokens::PAGE_SCALE;
    use crate::testing::{Harness, harness};
    use compositor_protocol::{Content, Fill};

    const SCREEN: Frame = Frame::new(0.0, 0.0, 960.0, 480.0);

    fn page(h: &mut Harness) {
        h.ui.look.text_scale = PAGE_SCALE;
        h.ui.look.page = true;
    }

    fn install() -> OverlayModel {
        OverlayModel::new(
            "Install v1-20261006i?",
            "It is written to slot A. The Show keeps running v1-20261006h until you choose to restart.",
        )
        .confirmation()
        .verified("Signed by lumashow \u{b7} sha256 4f2a\u{2026}c91e")
        .action(OverlayAction::new("cancel", "Cancel", ActionKind::Normal))
        .action(OverlayAction::new("go", "Install", ActionKind::Primary))
    }

    #[test]
    fn a_confirmation_is_the_mocks_dialog_title_30_body_22_buttons_140_wide_and_110_from_the_top() {
        let mut h = harness();
        page(&mut h);
        let words = Words::of(&h.ui, true);
        assert_eq!((words.title.px, words.title.weight), (30, Weight::SemiBold));
        assert_eq!((words.body.px, words.body.weight), (22, Weight::Regular));
        assert_eq!(words.verified.px, 20);
        let o = Overlay::new(&mut h.ui, SCREEN, 50, OverlayKind::Dialog, &install()).unwrap();
        let p = o.panel();
        assert_eq!((p.x, p.y, p.w), (200.0, 110.0, 560.0));
        // 28 above, the title's 37.5, 12, the body in 2 lines of 29.7, 14, the verified line's 25, 24, the buttons' 56 and 24 below:
        // the mock's 279.9.
        assert_eq!(o.layout.body.len(), 2, "{:?}", o.layout.body);
        assert!((p.h - 279.9).abs() < 0.05, "{}", p.h);
        let [cancel, go] = o.layout.actions.as_slice() else {
            panic!("two buttons");
        };
        assert!(cancel.w >= 140.0 && go.w >= 140.0, "{cancel:?} {go:?}");
        assert_eq!(go.x - cancel.right(), 16.0, "16 apart");
        assert_eq!(go.right(), p.right() - 32.0, "32 from the side");
        assert_eq!(go.bottom(), p.bottom() - 24.0, "24 from the foot");
    }

    #[test]
    fn every_other_overlay_keeps_the_toolkits_running_text_and_its_centre() {
        let mut h = harness();
        page(&mut h);
        let words = Words::of(&h.ui, false);
        assert_eq!(words.title, h.ui.look.ty(ty::TITLE));
        assert_eq!(words.body, h.ui.look.ty(ty::BODY));
        let model = OverlayModel::new(
            "Forget this network?",
            "It stays saved until you remove it.",
        )
        .action(OverlayAction::new("ok", "Forget", ActionKind::Destructive));
        let o = Overlay::new(&mut h.ui, SCREEN, 50, OverlayKind::Dialog, &model).unwrap();
        let p = o.panel();
        assert_eq!(p.y, (480.0 - p.h) / 2.0, "centred, not 110 down");
    }

    /// Decision 28 (2026-10-08 12:11): a light dialog keeps Lucent's light veil, not the mock's dark scrim. The veil is the palette's
    /// token in each look; this pins that the confirmation draws that token and that the light one is light.
    #[test]
    fn the_scrim_is_lucents_token_in_both_looks_and_in_the_light_one_a_light_veil() {
        for mode in [Mode::Dark, Mode::Light] {
            let mut h = harness();
            h.ui.look = Look::standard(mode);
            page(&mut h);
            let _o = Overlay::new(&mut h.ui, SCREEN, 50, OverlayKind::Dialog, &install()).unwrap();
            h.commit();
            let scrim = h.ui.look.scrim();
            assert!(
                h.layers().iter().any(
                    |(_, c, _)| matches!(c, Content::Shape(s) if s.fill == Fill::Solid(scrim))
                ),
                "{mode:?}: the scrim is the token"
            );
        }
        // The colours of a fill are premultiplied: the light veil is the palette's near-white at 30 %.
        let light = Look::standard(Mode::Light).scrim();
        let straight = |c: u8| u32::from(c) * 255 / u32::from(light.3);
        assert!(
            light.3 > 0
                && straight(light.0) > 200
                && straight(light.1) > 200
                && straight(light.2) > 200,
            "a light veil: {light:?}"
        );
        let dark = Look::standard(Mode::Dark).scrim();
        assert!(dark.0 < 60 && dark.1 < 60 && dark.2 < 60, "{dark:?}");
    }
}
