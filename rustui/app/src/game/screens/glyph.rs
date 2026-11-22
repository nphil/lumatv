//! What a hint's key looks like: the remote's icon, or the controller's face button when a controller is the last thing used.
use super::{paint::{Paint, Word}, symbols::Symbol};
use crate::game::{
    input::{Face, PadKind, Source},
    model::Key,
};
use lumatv_tokens::{TypeRole, themes::{composite, contrast}};
use ui::{
    Type,
    components::{Align, Icon, line_height},
    geometry::Frame,
    lucent::palette::{Mode, PaletteRole as Role, Roles, Straight},
};

/// The mark that leads a hint or a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lead {
    Icon(Icon),
    Pad(Face),
}
impl From<Icon> for Lead {
    fn from(icon: Icon) -> Self {
        Self::Icon(icon)
    }
}

/// The colours of a face-button glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceInk {
    pub disc: Straight,
    pub letter: Straight,
}

/// Xbox's face-button colours, in the shade that holds against the mode's surfaces: the bright shades on dark, deeper ones on light.
fn brand(face: Face, mode: Mode) -> Straight {
    let [r, g, b] = match (mode, face) {
        (Mode::Dark, Face::A) => [0x5B, 0xBF, 0x4A],
        (Mode::Dark, Face::B) => [0xF0, 0x5A, 0x4F],
        (Mode::Dark, Face::X) => [0x4C, 0x9B, 0xEB],
        (Mode::Dark, Face::Y) => [0xF5, 0xC5, 0x23],
        (Mode::Light, Face::A) => [0x13, 0x7A, 0x1B],
        (Mode::Light, Face::B) => [0xC6, 0x2F, 0x34],
        (Mode::Light, Face::X) => [0x1F, 0x5F, 0xBD],
        (Mode::Light, Face::Y) => [0x8A, 0x64, 0x00],
    };
    Straight::new(r, g, b, 255)
}

/// The brand colour with a black or white letter when the disc holds 3:1 against the canvas and the sheet (non-text) and the letter holds
/// 4.5:1 on the disc; otherwise the theme's ink with a canvas letter, which its roles already fit.
pub fn face_ink(roles: &Roles, mode: Mode, face: Face) -> FaceInk {
    let canvas = roles[Role::Canvas];
    let sheet = composite(roles[Role::Sheet], canvas);
    let disc = brand(face, mode);
    let (black, white) = (Straight::new(0, 0, 0, 255), Straight::new(255, 255, 255, 255));
    let letter = if contrast(black, disc) >= contrast(white, disc) { black } else { white };
    if contrast(letter, disc) >= 4.5 && contrast(disc, canvas) >= 3.0 && contrast(disc, sheet) >= 3.0 {
        FaceInk { disc, letter }
    } else {
        FaceInk { disc: roles[Role::Ink], letter: canvas }
    }
}

/// How a pad with black buttons and uncoloured markings (the GameSir Cyclone 2, the Leadjoy Xeno Plus) draws a face button: white on
/// black, which a theme maps to its ink and canvas roles. On a dark sheet the button would vanish, so it gets an ink rim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkInk {
    pub disc: Straight,
    pub mark: Straight,
    pub rim: bool,
}

pub fn mark_ink(roles: &Roles, mode: Mode) -> MarkInk {
    match mode {
        Mode::Light => MarkInk { disc: roles[Role::Ink], mark: roles[Role::Canvas], rim: false },
        Mode::Dark => MarkInk { disc: roles[Role::Canvas], mark: roles[Role::Ink], rim: true },
    }
}

impl Paint<'_> {
    /// The face button `key` is when the controller is what the person last used and the key has one.
    pub fn pad_face(&self, key: Key) -> Option<Face> {
        key.face().filter(|_| self.model.input_source() == Source::Controller)
    }
    /// The mark for `key`: its face button on a controller, otherwise the remote's `icon`.
    pub fn lead(&self, key: Key, icon: Icon) -> Lead {
        self.pad_face(key).map_or(Lead::Icon(icon), Lead::Pad)
    }
    /// A face button: a disc with its marking filling the square `at`, in nodes `id` and `id + 1`. The marking is the pad in use's
    /// (see `Model::pad_kind`). On a primary button the disc takes the button's ink and the marking its fill, which the primary pair
    /// already holds at 7:1.
    pub fn pad_glyph(&mut self, id: u64, face: Face, at: Frame, primary: bool, stage: bool, z: i32) {
        let mode = self.mode_at(stage);
        let kind = self.model.pad_kind();
        let side = at.w;
        if kind != PadKind::Xbox {
            let (disc, mark, rim) = if primary {
                let (fill, ink) = self.primary(stage);
                (ink, fill, false)
            } else {
                let ink = mark_ink(self.tokens.roles(mode), mode);
                (ink.disc.premultiply(0.0), ink.mark.premultiply(0.0), ink.rim)
            };
            self.solid(id, at, disc, side / 2.0, z);
            // The rim and the marking are one layer, so one colour: the ink.
            self.symbol(id + 1, Symbol::Face { kind, face, ring: rim }, at, mark, z + 1);
            return;
        }
        let (disc, letter) = if primary {
            let (fill, ink) = self.primary(stage);
            (ink, fill)
        } else {
            let ink = face_ink(self.tokens.roles(mode), mode, face);
            (ink.disc.premultiply(0.0), ink.letter.premultiply(0.0))
        };
        self.solid(id, at, disc, side / 2.0, z);
        let ty = Type { px: (side * 0.56).round().max(1.0) as u32, ..self.bold(TypeRole::Label) };
        let letter = Word { text: face.letter().into(), ty, colour: letter, x: 0.0, y: (side - line_height(ty)) / 2.0, w: side, align: Align::Centre };
        self.words(id + 1, at, vec![letter], vec![], z + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        input::Labels,
        model::{Input, Model},
        screens::paint::Body,
    };
    use lumagame_core::{catalog::Listing, state::State};
    use lumatv_tokens::{Tokens, themes};

    fn pads(names: &[&str]) -> [Option<String>; lumatv_gfn_pad::MAX_PADS] {
        std::array::from_fn(|i| names.get(i).map(|n| (*n).to_owned()))
    }

    fn model(controller: bool) -> Model {
        let mut m = Model::new(State::default(), Listing::default(), Vec::new());
        if controller {
            m.step(Input::Controllers(pads(&["Pad"])));
        }
        m
    }

    #[test]
    fn a_face_glyph_keeps_its_colour_only_where_it_holds_and_never_drops_below_the_ink_floor() {
        let mut coloured = 0;
        for family in themes::game_catalog() {
            for mode in [Mode::Dark, Mode::Light] {
                let roles = family.roles(mode);
                let canvas = roles[Role::Canvas];
                let sheet = composite(roles[Role::Sheet], canvas);
                for face in Face::ALL {
                    let ink = face_ink(roles, mode, face);
                    let what = format!("{} {mode:?} {face:?}", family.id);
                    assert!(contrast(ink.letter, ink.disc) >= 4.5, "{what}: letter");
                    assert!(contrast(ink.disc, canvas) >= 3.0 && contrast(ink.disc, sheet) >= 3.0, "{what}: disc");
                    coloured += usize::from(ink.disc == brand(face, mode));
                }
            }
        }
        let total = themes::game_catalog().len() * 2 * Face::ALL.len();
        assert!(coloured * 10 >= total * 9, "the accents hold on most themes: {coloured} of {total}");
        let rose = themes::game_find("rose-pine").unwrap();
        for mode in [Mode::Dark, Mode::Light] {
            for face in Face::ALL {
                assert_eq!(face_ink(rose.roles(mode), mode, face).disc, brand(face, mode), "{mode:?} {face:?} keeps its colour on the default theme");
            }
        }
    }

    #[test]
    fn a_hint_is_the_same_size_with_a_remote_icon_or_a_face_button() {
        let tokens = themes::game_tokens("rose-pine", Mode::Dark, &Tokens::shipped()).unwrap();
        let mut fonts = ui::Fonts::new();
        for percent in [100, 200] {
            let mut widths = Vec::new();
            for controller in [false, true] {
                let mut m = model(controller);
                m.prefs.text_percent = percent;
                let mut p = Paint::new(&tokens, &m, &mut fonts);
                let lead = p.lead(Key::Ok, Icon::Circle);
                assert_eq!(lead, if controller { Lead::Pad(Face::A) } else { Lead::Icon(Icon::Circle) });
                let w = p.hint(10, "Choose", lead, 0.0, 0.0, false, 1);
                let letters: Vec<_> = p.plan.nodes.iter().filter_map(|n| match &n.body {
                    Body::Text { words, .. } if words.iter().any(|w| w.text == "A") => Some(n.id),
                    _ => None,
                }).collect();
                assert_eq!(letters.is_empty(), !controller);
                widths.push(w);
            }
            assert_eq!(widths[0], widths[1], "{percent}%: switching must not move the layout");
        }
    }

    #[test]
    fn the_markings_follow_the_pad_in_use_or_the_setting() {
        let cyclone = "Guangzhou Chicken Run Network Technology Co., Ltd. Wireless Controller";
        let mut m = model(false);
        m.step(Input::Controllers(pads(&["Microsoft X-Box pad", cyclone])));
        assert_eq!(m.pad_kind(), PadKind::Xbox);
        m.step(Input::PadKey { slot: 1, key: Key::Ok, down: true });
        assert_eq!(m.pad_kind(), PadKind::GameSir);
        m.prefs.button_labels = Labels::Fixed(PadKind::Leadjoy);
        assert_eq!(m.pad_kind(), PadKind::Leadjoy, "the setting wins");
    }

    #[test]
    fn a_hint_is_the_same_size_for_every_marking_and_draws_it_as_one_symbol() {
        let tokens = themes::game_tokens("rose-pine", Mode::Dark, &Tokens::shipped()).unwrap();
        let mut fonts = ui::Fonts::new();
        let mut widths = Vec::new();
        for labels in Labels::ALL {
            for percent in [100, 200] {
                let mut m = model(true);
                m.prefs.text_percent = percent;
                m.prefs.button_labels = labels;
                let mut p = Paint::new(&tokens, &m, &mut fonts);
                let w = p.hint(10, "Choose", Lead::Pad(Face::A), 0.0, 0.0, false, 1);
                let symbols = p.plan.nodes.iter().filter(|n| matches!(n.body, Body::Symbol(Symbol::Face { .. }, _))).count();
                assert_eq!(symbols, usize::from(labels != Labels::Automatic && labels != Labels::Fixed(PadKind::Xbox)), "{labels:?}");
                widths.push((percent, w));
            }
        }
        for percent in [100, 200] {
            let of_percent: Vec<_> = widths.iter().filter(|(p, _)| *p == percent).map(|(_, w)| *w).collect();
            assert!(of_percent.windows(2).all(|w| w[0] == w[1]), "{percent}%: {of_percent:?}");
        }
    }

    #[test]
    fn black_buttons_hold_their_marking_and_their_edge_on_every_theme() {
        for family in themes::game_catalog() {
            for mode in [Mode::Dark, Mode::Light] {
                let roles = family.roles(mode);
                let ink = mark_ink(roles, mode);
                let sheet = composite(roles[Role::Sheet], roles[Role::Canvas]);
                let what = format!("{} {mode:?}", family.id);
                assert!(contrast(ink.mark, ink.disc) >= 4.5, "{what}: marking");
                // The edge is the disc itself, or the rim in the marking's ink when the disc is the canvas.
                let edge = if ink.rim { ink.mark } else { ink.disc };
                assert!(contrast(edge, sheet) >= 3.0, "{what}: edge");
            }
        }
    }

    #[test]
    fn keys_without_a_face_button_keep_the_remote_icon() {
        let tokens = themes::game_tokens("rose-pine", Mode::Dark, &Tokens::shipped()).unwrap();
        let mut fonts = ui::Fonts::new();
        let m = model(true);
        let p = Paint::new(&tokens, &m, &mut fonts);
        assert_eq!(p.lead(Key::Left, Icon::Back), Lead::Icon(Icon::Back));
        assert_eq!(p.lead(Key::Back, Icon::Back), Lead::Pad(Face::B));
        assert_eq!(p.lead(Key::Menu, Icon::Sliders), Lead::Pad(Face::Y));
    }

    #[test]
    fn the_steam_overlay_hint_is_x_in_the_active_style_and_the_remote_icon_otherwise() {
        let tokens = themes::game_tokens("rose-pine", Mode::Dark, &Tokens::shipped()).unwrap();
        let mut fonts = ui::Fonts::new();
        let remote = model(false);
        assert_eq!(Paint::new(&tokens, &remote, &mut fonts).lead(Key::Alt, Icon::Apps), Lead::Icon(Icon::Apps));
        for kind in [PadKind::Xbox, PadKind::PlayStation, PadKind::GameSir, PadKind::Leadjoy] {
            let mut m = model(true);
            m.prefs.button_labels = Labels::Fixed(kind);
            let mut p = Paint::new(&tokens, &m, &mut fonts);
            let lead = p.lead(Key::Alt, Icon::Apps);
            assert_eq!(lead, Lead::Pad(Face::X), "{kind:?}");
            p.hint(10, "Steam overlay", lead, 0.0, 0.0, false, 1);
            let marked = p.plan.nodes.iter().any(|n| matches!(n.body, Body::Symbol(Symbol::Face { kind: k, face: Face::X, .. }, _) if k == kind));
            let lettered = p.plan.nodes.iter().any(|n| matches!(&n.body, Body::Text { words, .. } if words.iter().any(|w| w.text == "X")));
            assert_eq!((marked, lettered), (kind != PadKind::Xbox, kind == PadKind::Xbox), "{kind:?}");
        }
    }

    #[test]
    fn the_last_kind_of_press_picks_the_glyphs() {
        let mut m = model(false);
        assert_eq!(m.input_source(), Source::Remote);
        let step = |m: &mut Model, input: Input| {
            m.step(input);
            m.input_source()
        };
        assert_eq!(step(&mut m, Input::Controllers(pads(&["Pad"]))), Source::Controller);
        assert_eq!(step(&mut m, Input::Key { key: Key::Left, down: true }), Source::Remote);
        assert_eq!(step(&mut m, Input::Key { key: Key::Left, down: false }), Source::Remote);
        assert_eq!(step(&mut m, Input::PadKey { slot: 0, key: Key::Back, down: false }), Source::Remote, "a release never switches");
        assert_eq!(step(&mut m, Input::PadKey { slot: 0, key: Key::Down, down: true }), Source::Controller);
        assert_eq!(step(&mut m, Input::Controllers(pads(&[]))), Source::Remote);
    }
}
