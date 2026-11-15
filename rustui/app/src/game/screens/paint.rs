use super::{art, symbols::Symbol};
use crate::game::model::{Focus, Model};
use compositor_protocol::{Backdrop, Edges, Glass, Props, Rect, Rgba};
use lumatv_tokens::{Radius, Tokens, TypeRole};
use ui::{
    Type, Weight,
    components::{Align, Form, Icon, line_height, wrap},
    geometry::Frame,
    lucent::{
        data::MATERIALS,
        palette::{Mode, PaletteRole as Role, Straight},
    },
};

pub const W: f32 = 1920.0;
pub const H: f32 = 1080.0;
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub ty: Type,
    pub colour: Rgba,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub align: Align,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub form: Form,
    pub colour: Rgba,
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Surface(Glass),
    Text { words: Vec<Word>, marks: Vec<Mark> },
    Image(art::Key),
    Qr(String),
    Symbol(Symbol, Rgba),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: u64,
    pub frame: Frame,
    pub props: Props,
    pub z: i32,
    pub body: Body,
}
#[derive(Default)]
pub struct Plan {
    pub nodes: Vec<Node>,
    pub hits: Vec<(Focus, Frame)>,
}
#[derive(Clone, Copy)]
pub enum Fill {
    Canvas,
    Sheet,
    Glass,
    Tint(f32),
    Primary,
    Accent,
    Track,
}
pub struct Paint<'a> {
    pub plan: Plan,
    pub tokens: &'a Tokens,
    pub model: &'a Model,
    pub fonts: &'a mut ui::Fonts,
    pub mode: Mode,
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub gap: f32,
}
// Keep drawing coordinates and token choices explicit, like the underlying toolkit.
#[allow(clippy::too_many_arguments)]
impl<'a> Paint<'a> {
    pub fn new(tokens: &'a Tokens, model: &'a Model, fonts: &'a mut ui::Fonts) -> Self {
        let layout = &tokens.json()["layout"];
        Self {
            plan: Plan::default(),
            tokens,
            model,
            fonts,
            mode: tokens.mode(None),
            scale: model.prefs.text_scale(),
            x: layout["edgeX"].as_f64().unwrap_or(96.0) as f32,
            y: layout["edgeY"].as_f64().unwrap_or(54.0) as f32,
            gap: layout["gutter"].as_f64().unwrap_or(24.0) as f32,
        }
    }
    pub fn mode_at(&self, stage: bool) -> Mode {
        if stage { Mode::Dark } else { self.mode }
    }
    pub fn straight(&self, role: Role, stage: bool) -> Straight {
        self.tokens.roles(self.mode_at(stage))[role]
    }
    pub fn ink(&self, role: Role, stage: bool) -> Rgba {
        self.straight(role, stage).premultiply(0.0)
    }
    pub fn ty(&self, role: TypeRole) -> Type {
        let mut ty = self.tokens.ty(role);
        ty.px = (ty.px as f32 * self.scale).round() as u32;
        ty
    }
    pub fn bold(&self, role: TypeRole) -> Type {
        Type { weight: Weight::SemiBold, ..self.ty(role) }
    }
    pub fn line(&self, role: TypeRole) -> f32 {
        self.ty(role).px as f32 * self.tokens.spec(role).line
    }
    pub fn focused(&self, focus: Focus) -> bool {
        self.model.view.focus == focus
    }
    pub fn reduced(&self) -> bool {
        self.model.prefs.reduce_motion || self.tokens.motion.reduced
    }
    pub fn symbol(&mut self, id: u64, symbol: Symbol, frame: Frame, colour: Rgba, z: i32) {
        self.plan.nodes.push(Node { id, frame, props: Props::at(frame.x, frame.y), z, body: Body::Symbol(symbol, colour) });
    }
    pub fn symbol_button(&mut self, id: u64, focus: Focus, symbol: Symbol, selected: bool, x: f32, y: f32, stage: bool, z: i32) {
        let h = self.tokens.target;
        let f = Frame::new(x, y, h, h);
        self.surface(id, f, Fill::Glass, Radius::Pill, stage, self.focused(focus), true, z);
        let side = (30.0 * self.scale).min(48.0);
        self.symbol(id + 1, symbol, Frame::new(x + (h - side) / 2.0, y + (h - side) / 2.0, side, side), self.ink(if selected { Role::Accent } else { Role::Ink }, stage), z + 1);
        if self.focused(focus) {
            self.land(id + 2, f, self.ink(Role::Ink, stage), 22.0, 8.0, z + 2);
        }
        self.hit(focus, f);
    }
    pub fn fit_focus(&mut self) {
        use crate::game::model::Page;
        if self.model.view.sheet.is_some() {
            return;
        }
        let Some((_, focus)) = self.plan.hits.iter().find(|(id, _)| *id == self.model.view.focus).copied() else {
            return;
        };
        let caption = if matches!(self.model.view.focus, Focus::Home(_, _) | Focus::Grid(_)) {
            self.line(TypeRole::Body) + self.line(TypeRole::Caption) + 40.0
        } else {
            0.0
        };
        let shift = if matches!(self.model.view.focus, Focus::Home(_, _)) { 0.0 } else { (focus.y + focus.h + caption - (H - self.y)).max(0.0) };
        let scrollable = |id: u64| match self.model.view.page {
            Page::Home | Page::Hub | Page::Integration => id >= 100,
            Page::Settings => id >= 200,
            Page::Library | Page::Search => id >= 1000,
            _ => false,
        };
        for node in &mut self.plan.nodes {
            if scrollable(node.id) {
                node.frame.y -= shift;
                node.props.y -= shift;
                node.props.clip = Some(Rect { x: 0.0, y: 150.0, w: W, h: H - 150.0 });
                node.props.feather = Edges::new(0.0, 0.0, 16.0, 24.0);
            }
        }
        for (id, frame) in &mut self.plan.hits {
            if !matches!(id, Focus::Nav(_) | Focus::Rail(_) | Focus::HubTop(_)) {
                frame.y -= shift;
            }
        }
        self.plan.nodes.retain(|n| n.id < 100 || n.frame.y + n.frame.h > 150.0 && n.frame.y < H);
    }
    pub fn primary(&self, stage: bool) -> (Rgba, Rgba) {
        let pair = lumatv_tokens::themes::game_primary(self.tokens.roles(self.mode_at(stage)));
        (pair.fill.premultiply(1.0), pair.ink.premultiply(1.0))
    }
    pub fn colour(&self, fill: Fill, stage: bool) -> Rgba {
        match fill {
            Fill::Canvas => self.ink(Role::Canvas, stage),
            Fill::Sheet => self.straight(Role::Sheet, stage).premultiply(0.96),
            Fill::Glass => self.ink(Role::GlassRaised, stage),
            Fill::Primary => self.primary(stage).0,
            Fill::Accent => self.ink(Role::Accent, stage),
            Fill::Track => self.ink(Role::TrackOff, stage),
            Fill::Tint(a) => {
                let mut c = self.straight(Role::Ink, stage);
                c.a = (a * 255.0).round() as u8;
                c.premultiply(0.0)
            }
        }
    }
    pub fn surface(&mut self, id: u64, frame: Frame, fill: Fill, radius: Radius, stage: bool, focused: bool, cached: bool, z: i32) {
        let f = MATERIALS.field_tokens(self.mode_at(stage));
        let glass = Glass {
            corner_radius: self.tokens.radius(radius).min(frame.h / 2.0),
            tint: self.colour(if matches!(fill, Fill::Glass) && !cached { Fill::Tint(0.06) } else { fill }, stage),
            edge: self.ink(if focused { Role::EdgeRaised } else { Role::Edge }, stage),
            edge_width: if matches!(fill, Fill::Canvas | Fill::Tint(_)) && !focused { 0.0 } else { f.edge_width },
            highlight: if focused {
                f.highlight_raised
            } else if cached {
                f.highlight_rest
            } else {
                Rgba(0, 0, 0, 0)
            },
            sheen: None,
            shadows: if focused {
                f.shadow_raised
            } else if cached {
                f.shadow_rest
            } else {
                [None, None]
            },
            depth: None,
            backdrop: if cached { Backdrop::Cached { radius: self.tokens.blur } } else { Backdrop::None },
            saturation: self.tokens.saturation,
            refraction: 0.0,
            edge_lens: None,
            tint_gradient: None,
        };
        self.plan.nodes.push(Node { id, frame, props: Props::at(frame.x, frame.y), z, body: Body::Surface(glass) });
    }
    pub fn solid(&mut self, id: u64, frame: Frame, colour: Rgba, radius: f32, z: i32) {
        self.plan.nodes.push(Node {
            id,
            frame,
            props: Props::at(frame.x, frame.y),
            z,
            body: Body::Surface(Glass {
                corner_radius: radius,
                tint: colour,
                edge: Rgba(0, 0, 0, 0),
                edge_width: 0.0,
                highlight: Rgba(0, 0, 0, 0),
                sheen: None,
                shadows: [None, None],
                depth: None,
                backdrop: Backdrop::None,
                saturation: 1.0,
                refraction: 0.0,
                edge_lens: None,
                tint_gradient: None,
            }),
        });
    }
    pub fn words(&mut self, id: u64, frame: Frame, words: Vec<Word>, marks: Vec<Mark>, z: i32) {
        self.plan.nodes.push(Node { id, frame, props: Props::at(frame.x, frame.y), z, body: Body::Text { words, marks } });
    }
    pub fn word(&self, text: impl Into<String>, role: TypeRole, colour: Rgba, x: f32, y: f32, w: f32) -> Word {
        Word { text: text.into(), ty: self.ty(role), colour, x, y, w, align: Align::Left }
    }
    pub fn text(&mut self, id: u64, text: &str, frame: Frame, role: TypeRole, colour: Role, stage: bool, z: i32) {
        let word = self.word(text, role, self.ink(colour, stage), 0.0, 0.0, frame.w);
        self.words(id, frame, vec![word], vec![], z);
    }
    pub fn wrapped(&mut self, id: u64, text: &str, frame: Frame, role: TypeRole, colour: Role, stage: bool, z: i32) -> f32 {
        let lines = wrap(self.fonts, text, self.ty(role), frame.w);
        self.lines(id, lines, frame, role, colour, stage, z)
    }
    /// An address to type on a phone, without its scheme, broken after a `/` or `.` where it is wider than the frame: `wrap` breaks
    /// only at spaces, and an address has none.
    pub fn address(&mut self, id: u64, url: &str, frame: Frame, role: TypeRole, colour: Role, stage: bool, z: i32) -> f32 {
        let shown = url.strip_prefix("https://").unwrap_or(url);
        let lines = break_address(self.fonts, shown, self.ty(role), frame.w);
        self.lines(id, lines, frame, role, colour, stage, z)
    }
    /// Lines `line(role)` apart; the layer is as tall as the last line's drawn box, which is 1.4 em (`line_height`): a role with a
    /// tighter line (display is 1.0) otherwise loses its descenders.
    fn lines(&mut self, id: u64, lines: Vec<String>, frame: Frame, role: TypeRole, colour: Role, stage: bool, z: i32) -> f32 {
        let line = self.line(role);
        let height = lines.len() as f32 * line;
        let drawn = height - line + line_height(self.ty(role)).max(line);
        let words = lines.into_iter().enumerate().map(|(i, text)| self.word(text, role, self.ink(colour, stage), 0.0, i as f32 * line, frame.w)).collect();
        self.words(id, Frame { h: drawn, ..frame }, words, vec![], z);
        height
    }
    pub fn mark(&self, icon: Icon, side: f32, x: f32, y: f32, role: Role, stage: bool) -> Mark {
        Mark { form: Form::Icon(icon, side.round() as u32), colour: self.ink(role, stage), x, y }
    }
    pub fn width(&mut self, text: &str, role: TypeRole) -> f32 {
        let ty = self.ty(role);
        self.fonts.measure(text, ty).width.ceil()
    }
    pub fn hit(&mut self, focus: Focus, frame: Frame) {
        self.plan.hits.push((focus, frame));
    }
    pub fn land(&mut self, id: u64, frame: Frame, colour: Rgba, inset: f32, bottom: f32, z: i32) {
        let h = self.tokens.focus.landing;
        self.solid(id, Frame::new(frame.x + inset, frame.y + frame.h - bottom - h, (frame.w - inset * 2.0).max(1.0), h), colour, h / 2.0, z);
    }
    pub fn button(&mut self, id: u64, focus: Focus, label: &str, icon: Option<Icon>, x: f32, y: f32, primary: bool, stage: bool, z: i32) -> f32 {
        let side = 30.0 * self.scale;
        let h = self.tokens.target.max(self.line(TypeRole::Label) + 24.0);
        let w = self.width(label, TypeRole::Label) + if icon.is_some() { side + 14.0 } else { 0.0 } + 62.0;
        let frame = Frame::new(x, y, w, h);
        let focused = self.focused(focus);
        self.surface(id, frame, if primary { Fill::Primary } else { Fill::Glass }, Radius::Pill, stage, focused, !primary, z);
        let ink = if primary { self.primary(stage).1 } else { self.ink(Role::Ink, stage) };
        let mut word =
            self.word(label, TypeRole::Label, ink, 28.0 + if icon.is_some() { side + 14.0 } else { 0.0 }, (h - self.line(TypeRole::Label)) / 2.0, w - 62.0);
        word.ty = self.bold(TypeRole::Label);
        let marks = icon.map(|icon| vec![Mark { form: Form::Icon(icon, side as u32), colour: ink, x: 28.0, y: (h - side) / 2.0 }]).unwrap_or_default();
        self.words(id + 1, frame, vec![word], marks, z + 2);
        if focused {
            self.land(id + 2, frame, ink, 26.0, 8.0, z + 3);
            if !self.reduced() {
                for node in self.plan.nodes.iter_mut().rev().take(3) {
                    node.props.y -= self.tokens.focus.card_lift;
                    node.props.scale = self.tokens.focus.card_scale;
                }
            }
        }
        self.hit(focus, frame);
        w
    }
    pub fn icon_button(&mut self, id: u64, focus: Focus, icon: Icon, x: f32, y: f32, stage: bool, z: i32) {
        let h = self.tokens.target;
        let f = Frame::new(x, y, h, h);
        self.surface(id, f, Fill::Glass, Radius::Pill, stage, self.focused(focus), true, z);
        self.words(id + 1, f, vec![], vec![self.mark(icon, 30.0, 17.0, 17.0, Role::Ink, stage)], z + 1);
        if self.focused(focus) {
            self.land(id + 2, f, self.ink(Role::Ink, stage), 22.0, 8.0, z + 2);
        }
        self.hit(focus, f);
    }
    pub fn toggle(&mut self, id: u64, x: f32, y: f32, on: bool, stage: bool, z: i32) {
        let f = Frame::new(x, y, 82.0, 48.0);
        self.surface(id, f, if on { Fill::Accent } else { Fill::Track }, Radius::Pill, stage, false, false, z);
        let ink = self.ink(if on { Role::AccentInk } else { Role::Ink2 }, stage);
        self.solid(id + 1, Frame::new(x + if on { 40.0 } else { 6.0 }, y + 6.0, 36.0, 36.0), ink, 18.0, z + 1);
        if on {
            self.words(id + 2, f, vec![], vec![self.mark(Icon::Tick, 24.0, 46.0, 12.0, Role::Accent, stage)], z + 2);
        }
    }
    pub fn image(&mut self, id: u64, path: std::path::PathBuf, frame: Frame, radius: f32, logo: bool, stage: Option<art::Stage>, z: i32) {
        let spec = art::Spec { width: frame.w.round() as u32, height: frame.h.round() as u32, radius: radius.round() as u32, logo, stage };
        self.plan.nodes.push(Node { id, frame, props: Props::at(frame.x, frame.y), z, body: Body::Image(art::Key { path, spec }) });
    }
    pub fn status(&mut self, id: u64, label: &str, role: Role, x: f32, y: f32, stage: bool, z: i32) -> f32 {
        let h = 44.0f32.max(self.line(TypeRole::Caption) + 8.0);
        let side = 26.0 * self.scale;
        let w = self.width(label, TypeRole::Caption) + side + 34.0;
        let f = Frame::new(x, y, w, h);
        self.surface(id, f, Fill::Tint(0.09), Radius::Pill, stage, false, false, z);
        self.words(
            id + 1,
            f,
            vec![self.word(label, TypeRole::Caption, self.ink(Role::Ink, stage), side + 22.0, (h - self.line(TypeRole::Caption)) / 2.0, w - side - 34.0)],
            vec![Mark {
                form: Form::Icon(Icon::Tick, side as u32),
                colour: self.ink(role, stage),
                x: 12.0,
                y: (h - side) / 2.0,
            }],
            z + 1,
        );
        w
    }
    pub fn hint(&mut self, id: u64, label: &str, icon: Icon, x: f32, y: f32, stage: bool, z: i32) -> f32 {
        let side = 37.0 * self.scale;
        let w = self.width(label, TypeRole::Label) + side + 10.0;
        let frame = Frame::new(x, y, w, side.max(self.line(TypeRole::Label)));
        self.words(
            id,
            frame,
            vec![self.word(label, TypeRole::Label, self.ink(Role::Ink2, stage), side + 10.0, (side - self.line(TypeRole::Label)) / 2.0, w - side - 10.0)],
            vec![self.mark(icon, side, 0.0, 0.0, Role::Ink, stage)],
            z,
        );
        w
    }
}
pub fn ready_word(ready: lumagame_core::ipc::Readiness) -> (&'static str, Role) {
    use lumagame_core::ipc::Readiness::*;
    match ready {
        Ready => ("Ready", Role::Positive),
        Asleep => ("Asleep", Role::Info),
        SignIn => ("Sign in", Role::Warning),
        Offline => ("Offline", Role::Danger),
        Install => ("Install", Role::Ink2),
        Unknown => ("Unknown", Role::Ink2),
    }
}

/// `text` in lines no wider than `max_w`, each broken after its last `/` or `.` that fits, or after the last character that fits when
/// there is none.
fn break_address(fonts: &mut ui::Fonts, text: &str, ty: Type, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest: Vec<char> = text.chars().collect();
    while !rest.is_empty() {
        let mut fit = 1;
        while fit < rest.len() && fonts.measure(&rest[..=fit].iter().collect::<String>(), ty).width <= max_w {
            fit += 1;
        }
        if fit < rest.len() {
            if let Some(at) = rest[..fit].iter().rposition(|c| matches!(c, '/' | '.')) {
                fit = at + 1;
            }
        }
        lines.push(rest.drain(..fit).collect());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_wider_than_the_frame_breaks_after_a_slash_and_keeps_every_character() {
        let mut fonts = ui::Fonts::new();
        let ty = Type { px: 40, weight: Weight::Regular };
        let url = "static-login.nvidia.com/service/gfn/pin";
        let max_w = fonts.measure("static-login.nvidia.com/service/", ty).width + 1.0;
        let lines = break_address(&mut fonts, url, ty, max_w);
        assert_eq!(lines, ["static-login.nvidia.com/service/", "gfn/pin"]);
        assert_eq!(break_address(&mut fonts, url, ty, 4000.0), [url]);
    }
}
