//! The camera pop-up drawn (`panels/Cameras.qml`'s card, the mock `docs/mocks/cameras/popup`, spec "The pop-up"): a glass card 560 wide, flush in the
//! corner the placement crate gives the camera window (`layout.overlays.popup`; top right by default, ending where Home's status bar does), with the
//! camera's name and what was seen in the head, the picture (a still, 520 by 292 at 100 % text, giving height back, never under 120, where the type is
//! large so the card ends above the zone's bottom) and the picture's state and the keys in the foot. Nothing is drawn in the picture box until a
//! picture arrives but a quiet plate with the camera's glyph, so the card never shows an empty hole.
//!
//! **The picture box is a hole in the glass.** The card's glass is four bands of one pane (above the box, below it and at each side), so that the box is
//! clear: the live video (`popup_live.rs`) plays on the display's video plane under it and shows through. The box's four corners are masks in the glass's
//! own colour outside an arc of the tile's radius, since the plane's picture has square corners. While no video is the picture the box holds the still, the
//! event's picture or the plate, a layer of the shell's that covers the hole, and the card looks the same whichever it is.
//!
//! The card is made when it comes and dropped when it has gone (the scene's budget): it fades in and out again quicker (the glass is clipped in the screen's
//! own pixels, so it does not slide: a slide would move the glass and not its clip); its words are made again in place when they change (the age), and only
//! the picture, the state and the dot change after that.

use super::parts::{HIDDEN, Sheet, hint_into, hint_width};
use super::popup::{Face, View, PICTURE};
use crate::kit::hint::{Action, Hint, Key};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{self, Pane, PaneKind};
use crate::kit::picture::{Fit, Picture};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::{Kit, wrap};
use compositor_protocol::{Props, Rect, Rgba};
use lumatv_placement::{Frame as PlacementFrame, Rect as PlaceRect, Registry, Size, User};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::text::Weight;

pub const WIDTH: f32 = 560.0;
/// The head's glyph: 36 at 100 % text, growing with the type but not as fast.
fn glyph_px(kit: &Kit) -> f32 {
    (36.0 * (1.0 + 0.6 * (kit.text_scale - 1.0))).round()
}
const DOT: f32 = 12.0;
/// The card fades; it does not slide (see the module's note on the clipped glass).
const SLIDE: f32 = 0.0;
const FACES: [Face; 4] = [Face::Connecting, Face::EventPicture, Face::Live, Face::NoPicture];

/// Everything about the card that does not change while the same words are on it.
struct Plan {
    height: f32,
    pad: f32,
    name: (Frame, String, Style),
    what: Vec<(Frame, String, Style)>,
    picture: Frame,
    state: (Frame, Style),
    dot: (f32, f32),
    hints: Vec<(f32, f32, f32, Hint)>,
    icon: &'static str,
}

fn hints_of(can_expand: bool) -> Vec<Hint> {
    if can_expand { vec![Hint::key(Key::Ok, Action::Expand), Hint::key(Key::Back, Action::Close)] } else { vec![Hint::key(Key::Ok, Action::Close)] }
}

/// The card as it is laid out for `view`, with `room` the height from the card's top to the bottom of its zone.
fn plan(kit: &mut Kit, view: &View, room: f32) -> Plan {
    let pad = kit.space(5);
    let inner = WIDTH - 2.0 * pad;
    let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
    let name_style = role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left);
    let what_style = role_style(kit, TypeRole::Label, None, ink2, Align::Left);
    let glyph = glyph_px(kit);
    let text_x = pad + glyph + kit.space(4);
    let text_w = inner - glyph - kit.space(4);
    let name = wrap::elide(kit, name_style, &view.name, text_w);
    // what was seen and how long ago share a line when they fit, else the age takes the second
    let joined = format!("{}  \u{b7}  {}", view.what, view.age);
    let what = if width_of(kit, what_style, &joined) <= text_w { vec![joined] } else { vec![wrap::elide(kit, what_style, &view.what, text_w), view.age.clone()] };
    let (nh, wh) = (kit.line(TypeRole::Body), kit.line(TypeRole::Label));
    let head_h = glyph.max(nh + kit.space(1) + what.len() as f32 * wh);
    let caption = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
    // the state's box is as wide as the widest of its words, so the foot does not move as the picture comes
    let state_w = FACES.iter().map(|f| width_of(kit, caption, f.words())).fold(0.0, f32::max) + DOT + kit.space(2);
    let hints = hints_of(view.can_expand);
    let hint_h = crate::kit::hint::glyph_side(kit, TypeRole::Caption).max(kit.line(TypeRole::Caption) as u32) as f32;
    let widths: Vec<f32> = hints.iter().map(|h| hint_width(kit, h, TypeRole::Caption)).collect();
    let sp = kit.space(5);
    let keys_w = widths.iter().sum::<f32>() + sp * widths.len().saturating_sub(1) as f32;
    let line = kit.line(TypeRole::Caption);
    let one_line = state_w + kit.space(4) + keys_w <= inner;
    let stacked = keys_w > inner;
    let foot_h = if one_line {
        line.max(hint_h)
    } else {
        line + kit.space(1) + if stacked { hints.len() as f32 * hint_h + hints.len().saturating_sub(1) as f32 * kit.space(1) } else { hint_h }
    };
    // 292: an even height, as the decoders like it; where the head and the foot grow the picture gives up height, never below 120
    let natural = (inner * 9.0 / 16.0).floor();
    let spare = room - (2.0 * pad + head_h + 2.0 * kit.space(3) + foot_h);
    let prev_h = natural.min(((spare as i32) >> 1 << 1) as f32).max(120.0);
    let pic_y = (pad + head_h + kit.space(3)).round();
    let foot_y = pic_y + prev_h + kit.space(3);
    let mut placed = Vec::new();
    let (mut x, mut y) = (if one_line { pad + inner - keys_w } else { pad }, if one_line { foot_y } else { foot_y + line + kit.space(1) });
    for (hint, w) in hints.into_iter().zip(widths) {
        placed.push((x, y, hint_h, hint));
        if stacked {
            y += hint_h + kit.space(1);
        } else {
            x += w + sp;
        }
    }
    let what_frames = what.into_iter().enumerate().map(|(i, t)| (Frame::new(text_x, pad + nh + kit.space(1) + i as f32 * wh, text_w, wh), t, what_style)).collect();
    Plan {
        height: foot_y + foot_h + pad,
        pad,
        name: (Frame::new(text_x, pad, text_w, nh), name, name_style),
        what: what_frames,
        picture: Frame::new(pad.round(), pic_y, WIDTH - 2.0 * pad.round(), prev_h),
        state: (Frame::new(pad + DOT + kit.space(2), foot_y, state_w - DOT - kit.space(2), line), caption),
        dot: (pad, foot_y + ((line - DOT) / 2.0).round()),
        hints: placed,
        icon: view.icon,
    }
}

/// A quiet plate with the camera's glyph, for the box while no picture is there.
fn placeholder(kit: &mut Kit, w: u32, h: u32, radius: f32) -> Picture {
    let ink = kit.ui.look.ink();
    let wash = crate::kit::button::faded(ink, 0.10);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&[wash.0, wash.1, wash.2, wash.3]);
    }
    let side = 72u32.min(h / 2).max(1);
    if let Some(icon) = kit.icons.draw("video", side, kit.ui.look.ink3()) {
        let (ox, oy) = ((w - side) / 2, (h - side) / 2);
        for y in 0..side {
            for x in 0..side {
                let (i, j) = (((y * side + x) * 4) as usize, (((oy + y) * w + ox + x) * 4) as usize);
                let k = 255 - u32::from(icon[i + 3]);
                for c in 0..4 {
                    rgba[j + c] = (u32::from(icon[i + c]) + u32::from(rgba[j + c]) * k / 255).min(255) as u8;
                }
            }
        }
    }
    let mut p = Picture { width: w, height: h, rgba };
    p.round_corners(radius);
    p
}

/// One corner of the picture box that the video's square corner must not show at: the glass's own colour outside an arc of `r` pixels, transparent inside it
/// (`corner` 0 top left, 1 top right, 2 bottom left, 3 bottom right).
fn corner_mask(r: u32, corner: u32, tint: Rgba) -> Picture {
    let rf = r as f32;
    let mut rgba = vec![0u8; (r * r * 4) as usize];
    for y in 0..r {
        for x in 0..r {
            // mirrored into the top left quadrant, where the arc's centre is r in from both edges
            let mx = if corner & 1 == 1 { r - 1 - x } else { x };
            let my = if corner & 2 == 2 { r - 1 - y } else { y };
            let dist = ((mx as f32 + 0.5 - rf).powi(2) + (my as f32 + 0.5 - rf).powi(2)).sqrt();
            let outside = 1.0 - (rf - dist + 0.5).clamp(0.0, 1.0);
            let i = ((y * r + x) * 4) as usize;
            for (c, v) in [tint.0, tint.1, tint.2, tint.3].into_iter().enumerate() {
                rgba[i + c] = (f32::from(v) * outside).round() as u8;
            }
        }
    }
    Picture { width: r, height: r, rgba }
}

struct Word {
    text: String,
    frame: Frame,
    style: Style,
}

struct Built {
    sig: String,
    generation: u64,
    over_video: bool,
    sheet: Sheet,
    frame: Frame,
    paint: Rgba,
    words: Vec<Word>,
    picture: usize,
    state: usize,
    dot: usize,
    painted: Option<(u64, Face)>,
    face: Face,
    state_word: usize,
    picture_frame: Frame,
    /// Where the picture's layer rests while it shows: it is taken out of the picture box (the hole) while the video is the picture.
    picture_rest: Props,
    /// The picture box is a hole at the moment.
    video: bool,
}

pub struct PopupSurface {
    z: i32,
    registry: Registry,
    user: User,
    frame: PlacementFrame,
    current: Option<Built>,
    leaving: Vec<(Sheet, u64)>,
}

impl PopupSurface {
    pub fn new(z: i32, theme: &Value, layout: &Value) -> Self {
        let registry = Registry::shipped().clone();
        let user = registry.user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        let frame = PlacementFrame::from_json(layout, theme.pointer("/overlays/limit"));
        Self { z, registry, user, frame, current: None, leaving: Vec::new() }
    }

    /// The owner's places changed (`layout.overlays`).
    pub fn set_user(&mut self, user: User) {
        self.user = user;
    }

    fn room(&self) -> f32 {
        let zone = self.registry.zone_of("popup", &self.frame);
        zone.h as f32
    }

    /// The card's size for `view` at the kit's text size, and the picture box the stills are asked for at.
    pub fn measure(&self, kit: &mut Kit, view: &View) -> (Size, (u32, u32)) {
        let p = plan(kit, view, self.room());
        (Size::new(WIDTH as i32, p.height.ceil() as i32), (p.picture.w as u32, p.picture.h as u32))
    }

    /// Where the card is.
    pub fn rect(&self, size: Size) -> PlaceRect {
        self.registry.rect_of("popup", &self.user, &self.frame, size).unwrap_or(PlaceRect::new(0, 0, size.w, size.h))
    }

    /// Draws `view`. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, view: &View, picture: Option<&Picture>, revision: u64, over_video: bool, now_ms: u64) -> std::io::Result<bool> {
        if let Some(c) = &self.current
            && (c.generation != kit.generation() || c.over_video != over_video)
        {
            self.current = None;
        }
        if !view.up {
            if let Some(mut gone) = self.current.take() {
                // the video goes with its claim, and what a hole shows then is the display behind it: the box shows its plate while the card fades
                if gone.video {
                    let radius = kit.tokens.radius(Radius::Tile);
                    gone.sheet.move_part(gone.picture, gone.picture_rest);
                    let size = gone.sheet.image_mut(gone.picture).map(|l| (l.width(), l.height()));
                    if let Some((pw, ph)) = size {
                        let plate = placeholder(kit, pw, ph, radius);
                        if let Some(l) = gone.sheet.image_mut(gone.picture) {
                            plate.paint(l);
                        }
                    }
                }
                let slide = if kit.ui.look.reduced_motion { 0.0 } else { SLIDE };
                gone.sheet.slide_out(kit, slide);
                self.leaving.push((gone.sheet, now_ms + u64::from(kit.beat_ms(crate::kit::motion::Beat::Rest)) + 40));
            }
            return Ok(!self.leaving.is_empty());
        }
        let (size, _) = self.measure(kit, view);
        let rect = self.rect(size);
        let sig = format!("{:?}", (&view.camera, &view.name, &view.what, &view.age, view.icon, view.can_expand, rect.x, rect.y, size.h));
        let came = self.current.is_none();
        if self.current.as_ref().is_some_and(|c| c.sig != sig) {
            self.current = None;
        }
        if self.current.is_none() {
            let p = plan(kit, view, self.room());
            let mut built = Self::build(kit, self.z, view, &p, (rect.x as f32, rect.y as f32), sig, over_video)?;
            let slide = if kit.ui.look.reduced_motion { 0.0 } else { SLIDE };
            built.sheet.slide_in(kit, slide, !came);
            self.current = Some(built);
        }
        let b = self.current.as_mut().expect("built above");
        // the picture box: a hole over the video plane while the stream is the picture, else the best picture there is, else the plate
        if b.video != view.video {
            b.video = view.video;
            let rest = b.picture_rest;
            b.sheet.move_part(b.picture, if view.video { Props { opacity: 0.0, visible: false, ..rest } } else { rest });
            b.painted = None;
        }
        if !view.video && b.painted != Some((revision, view.face)) {
            let radius = kit.tokens.radius(Radius::Tile);
            let (pw, ph) = match b.sheet.image_mut(b.picture) {
                Some(l) => (l.width(), l.height()),
                None => (0, 0),
            };
            let shown = match picture {
                Some(p) => {
                    let mut f = p.fitted(pw, ph, Fit::Cover);
                    f.round_corners(radius);
                    f
                }
                None => placeholder(kit, pw, ph, radius),
            };
            if let Some(l) = b.sheet.image_mut(b.picture) {
                shown.paint(l);
            }
            b.painted = Some((revision, view.face));
        }
        if b.face != view.face {
            b.face = view.face;
            let words = view.face.words();
            if let Some(t) = b.sheet.text_mut(b.state) {
                t.set(kit, words);
            }
            if let Some(w) = b.words.get_mut(b.state_word) {
                w.text = words.to_string();
            }
            if let Some(at) = b.sheet.at(b.dot) {
                let on = view.face == Face::Live;
                if let Some(d) = b.sheet.shape_mut(b.dot) {
                    d.set(if on { at } else { Props { opacity: 0.0, visible: false, ..at } });
                }
            }
        }
        Ok(true)
    }

    fn build(kit: &mut Kit, z: i32, view: &View, p: &Plan, at: (f32, f32), sig: String, over_video: bool) -> std::io::Result<Built> {
        let frame = Frame::new(at.0, at.1, WIDTH, p.height);
        let mut sheet = Sheet::new();
        // the glass is four bands of one pane, clear where the picture box is (a hole the video plays through), and the box's corners are masks
        let pic = Frame::new(at.0 + p.picture.x, at.1 + p.picture.y, p.picture.w, p.picture.h);
        let reach = 64.0;
        let (left, top, right, bottom) = (frame.x - reach, frame.y - reach, frame.x + frame.w + reach, frame.y + frame.h + reach);
        for clip in [
            Rect { x: left, y: top, w: right - left, h: pic.y - top },
            Rect { x: left, y: pic.y + pic.h, w: right - left, h: bottom - pic.y - pic.h },
            Rect { x: left, y: pic.y, w: pic.x - left, h: pic.h },
            Rect { x: pic.x + pic.w, y: pic.y, w: right - pic.x - pic.w, h: pic.h },
        ] {
            sheet.pane(Pane::clipped(kit, frame, z, PaneKind::Card, over_video, clip));
        }
        let radius = kit.tokens.radius(Radius::Tile).min(pic.w / 2.0).min(pic.h / 2.0).ceil();
        if radius >= 1.0 {
            let tint = pane::glass(kit, PaneKind::Card, frame.h, over_video).tint;
            let r = radius as u32;
            for corner in 0..4u32 {
                let cx = if corner & 1 == 1 { pic.x + pic.w - radius } else { pic.x };
                let cy = if corner & 2 == 2 { pic.y + pic.h - radius } else { pic.y };
                let mut layer = kit.ui.scene.image(r, r, z + 1, HIDDEN)?;
                corner_mask(r, corner, tint).paint(&mut layer);
                sheet.image(layer, (cx, cy));
            }
        }
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        if let Some(g) = icon::layer(kit, p.icon, glyph_px(kit) as u32, ink, z + 1, HIDDEN)? {
            sheet.image(g, (at.0 + p.pad, at.1 + p.pad));
        }
        let mut words = Vec::new();
        let put = |kit: &mut Kit, sheet: &mut Sheet, f: &Frame, text: &str, style: Style, words: &mut Vec<Word>| -> std::io::Result<usize> {
            let frame = Frame::new(at.0 + f.x, at.1 + f.y, f.w, f.h);
            let i = sheet.text(Text::new(kit, frame, z + 2, style, text)?);
            words.push(Word { text: text.to_string(), frame, style });
            Ok(i)
        };
        put(kit, &mut sheet, &p.name.0, &p.name.1, p.name.2, &mut words)?;
        for (f, t, s) in &p.what {
            put(kit, &mut sheet, f, t, *s, &mut words)?;
        }
        // the picture box
        let (pw, ph) = (p.picture.w as u32, p.picture.h as u32);
        let layer = kit.ui.scene.image(pw, ph, z + 1, HIDDEN)?;
        let picture = sheet.image(layer, (at.0 + p.picture.x, at.1 + p.picture.y));
        let picture_rest = sheet.at(picture).unwrap_or(Props::at(0.0, 0.0));
        // the state: its dot while a still is on screen, its words
        let live = kit.ui.look.colour(PaletteRole::Live);
        let dot_layer = plate::shape(&kit.ui, Frame::new(0.0, 0.0, DOT, DOT), z + 2, plate::solid(live, DOT / 2.0));
        let dot = sheet.shape(dot_layer, (at.0 + p.dot.0, at.1 + p.dot.1));
        let state_word = words.len();
        let state = put(kit, &mut sheet, &p.state.0, view.face.words(), p.state.1, &mut words)?;
        if view.face != Face::Live {
            // the dot is part of the sheet's rest; it is hidden until a still is on screen
            if let Some(d) = sheet.shape_mut(dot) {
                d.set(HIDDEN);
            }
        }
        for (x, y, h, hint) in &p.hints {
            hint_into(kit, &mut sheet, hint, TypeRole::Caption, at.0 + x, at.1 + y, *h, z + 2, ink2)?;
        }
        let paint = kit.ui.look.colour(PaletteRole::Sheet);
        if kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the camera card could not be drawn whole"));
        }
        Ok(Built {
            sig,
            generation: kit.generation(),
            over_video,
            sheet,
            frame,
            paint,
            words,
            picture,
            state,
            dot,
            painted: None,
            face: view.face,
            state_word,
            picture_frame: p.picture,
            picture_rest,
            video: false,
        })
    }

    pub fn tick(&mut self, now_ms: u64) -> bool {
        let n = self.leaving.len();
        self.leaving.retain(|(_, until)| *until > now_ms);
        n != self.leaving.len()
    }

    pub fn due_at(&self) -> Option<u64> {
        self.leaving.iter().map(|(_, until)| *until).min()
    }

    pub fn is_up(&self) -> bool {
        self.current.is_some()
    }

    /// The card's rectangle while it is up: what the cards go under and the mute mark keeps clear of.
    pub fn frame(&self) -> Option<PlaceRect> {
        self.current.as_ref().map(|b| PlaceRect::new(b.frame.x as i32, b.frame.y as i32, b.frame.w as i32, b.frame.h as i32))
    }

    /// The picture box on the screen in output pixels while the card is up: what the live video's claim asks the compositor for.
    pub fn picture_rect(&self) -> Option<(i32, i32, i32, i32)> {
        self.current.as_ref().map(|b| {
            ((b.frame.x + b.picture_frame.x).round() as i32, (b.frame.y + b.picture_frame.y).round() as i32, b.picture_frame.w.round() as i32, b.picture_frame.h.round() as i32)
        })
    }

    pub fn geometry(&self) -> Value {
        match &self.current {
            Some(b) => json!({
                "x": b.frame.x, "y": b.frame.y, "w": b.frame.w, "h": b.frame.h,
                "picture": {"x": b.frame.x + b.picture_frame.x, "y": b.frame.y + b.picture_frame.y, "w": b.picture_frame.w, "h": b.picture_frame.h},
                "face": b.face.words(),
                "video": b.video,
            }),
            None => Value::Null,
        }
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        if let Some(b) = &self.current {
            let panel = lint.panel("camera card", b.frame, b.paint, true);
            for (i, w) in b.words.iter().enumerate() {
                lint.word(kit, &format!("camera card words {i}"), &w.text, w.frame, w.style, Some(panel));
            }
        }
    }

    pub fn layers(&self) -> usize {
        self.current.iter().map(|b| b.sheet.layers()).sum::<usize>() + self.leaving.iter().map(|(s, _)| s.layers()).sum::<usize>()
    }

    pub fn restyle(&mut self) {
        self.current = None;
        self.leaving.clear();
    }

    /// The default picture box, before a card has been measured.
    pub fn default_picture() -> (u32, u32) {
        PICTURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::system::popup::Popup;
    use crate::kit::screen::Input;
    use ui::lucent::palette::Mode;

    const T0: u64 = 1_800_000_000_000;

    fn layout() -> Value {
        serde_json::from_str(include_str!("../../../../payload/theme/layout.json")).unwrap()
    }

    fn up(expand: bool) -> Popup {
        let mut p = Popup::new();
        p.set_can_expand(expand);
        p.update(
            &Input::Helper {
                topic: "cameras.popup".into(),
                data: json!({"id": 41, "event": "e41", "camera": "273", "name": "Front door", "kind": "person", "at": T0, "untilMs": T0 + 60_000, "thumbnail": true}),
            },
            T0,
        );
        p
    }

    fn lint_of(kit: &mut Kit, s: &PopupSurface) -> Vec<String> {
        let mut lint = Lint::new();
        s.feed_lint(kit, &mut lint);
        lint.findings().iter().map(ToString::to_string).collect()
    }

    #[test]
    fn the_card_is_flush_in_the_top_right_corner_with_the_specs_picture_and_reads_clean_in_both_materials_and_sizes() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut s = PopupSurface::new(1090, &json!({}), &layout());
                let p = up(true);
                assert!(s.apply(&mut kit, &p.view(), None, 0, false, 1000).unwrap());
                let g = s.geometry();
                assert_eq!(g["x"].as_f64().unwrap() + g["w"].as_f64().unwrap(), 1856.0, "ends where Home's status bar does: {g}");
                assert_eq!(g["y"].as_f64().unwrap(), 134.0);
                assert!(g["y"].as_f64().unwrap() + g["h"].as_f64().unwrap() <= 810.0, "ends above the bottom quarter at {scale}: {g}");
                let pic = &g["picture"];
                if scale == 1.0 {
                    assert_eq!((pic["w"].as_f64().unwrap(), pic["h"].as_f64().unwrap()), (520.0, 292.0), "the spec's picture: {g}");
                } else {
                    assert!(pic["h"].as_f64().unwrap() >= 120.0 && pic["h"].as_f64().unwrap() <= 292.0, "the picture keeps its height unless the card would pass the zone, never under 120: {g}");
                }
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
            }
        }
    }

    #[test]
    fn it_is_flush_in_the_corner_the_owner_chose_at_any_size() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit(scale);
            let reg = Registry::shipped();
            let user = reg.with_placement(&User::default(), "popup", lumatv_placement::Placement::new(lumatv_placement::Anchor::BottomLeft, 0.0, 0.0));
            let theme = json!({"layout": {"overlays": user.to_json()}});
            let mut s = PopupSurface::new(1090, &theme, &layout());
            let p = up(false);
            s.apply(&mut kit, &p.view(), None, 0, false, 1000).unwrap();
            let g = s.geometry();
            assert_eq!(g["x"].as_f64().unwrap(), 64.0, "flush left at {scale}: {g}");
            assert_eq!(g["y"].as_f64().unwrap() + g["h"].as_f64().unwrap(), 810.0, "flush at the zone's bottom at {scale}: {g}");
        }
    }

    #[test]
    fn the_picture_box_is_a_hole_while_the_video_is_the_picture_and_the_card_has_the_same_layers_either_way() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit(scale);
            let mut s = PopupSurface::new(1090, &json!({}), &layout());
            let p = up(true);
            s.apply(&mut kit, &p.view(), None, 0, false, 1000).unwrap();
            let layers = s.layers();
            let (x, y, w, h) = s.picture_rect().expect("the card is up");
            let g = s.geometry();
            assert_eq!((f64::from(x), f64::from(y), f64::from(w), f64::from(h)), (g["picture"]["x"].as_f64().unwrap(), g["picture"]["y"].as_f64().unwrap(), g["picture"]["w"].as_f64().unwrap(), g["picture"]["h"].as_f64().unwrap()), "whole pixels, as the claim needs");
            assert_eq!((w % 2, h % 2), (0, 0), "an even size, as the decoders like it");
            {
                let b = s.current.as_ref().unwrap();
                assert!(b.sheet.at(b.picture).unwrap().visible && !b.video, "the shell's picture covers the box at first");
            }
            let mut v = p.view();
            v.video = true;
            v.face = Face::Live;
            s.apply(&mut kit, &v, None, 1, false, 1100).unwrap();
            let b = s.current.as_ref().unwrap();
            assert!(b.video && !b.sheet.at(b.picture).unwrap().visible, "the picture is taken out of the box: the video shows through");
            assert_eq!(s.geometry()["video"], true);
            assert_eq!(s.layers(), layers, "a hole is a layer taken out of the box, not a layer more or less");
            assert!(lint_of(&mut kit, &s).is_empty(), "at {scale}");
            // the video goes: the shell's picture is back
            v.video = false;
            s.apply(&mut kit, &v, None, 2, false, 1200).unwrap();
            let b = s.current.as_ref().unwrap();
            assert!(!b.video && b.sheet.at(b.picture).unwrap().visible);
            // and the card leaves with its plate in the box, not the display behind it
            v.video = true;
            s.apply(&mut kit, &v, None, 3, false, 1300).unwrap();
            v.up = false;
            s.apply(&mut kit, &v, None, 4, false, 1400).unwrap();
            assert!(!s.is_up() && s.layers() > 0, "it fades out: its layers are kept until it has");
            assert!(s.tick(10_000));
        }
    }

    #[test]
    fn the_corners_of_the_box_are_masks_that_are_clear_inside_the_arc_and_the_glass_outside_it() {
        let tint = Rgba(10, 20, 30, 200);
        for corner in 0..4u32 {
            let m = corner_mask(12, corner, tint);
            // the outer corner pixel is the glass, the pixel at the inner corner of the square (inside the arc) is clear
            let outer = match corner {
                0 => m.at(0, 0),
                1 => m.at(11, 0),
                2 => m.at(0, 11),
                _ => m.at(11, 11),
            };
            let inner = match corner {
                0 => m.at(11, 11),
                1 => m.at(0, 11),
                2 => m.at(11, 0),
                _ => m.at(0, 0),
            };
            assert_eq!(outer, [10, 20, 30, 200], "the glass at the outer corner of corner {corner}");
            assert_eq!(inner, [0, 0, 0, 0], "clear at the inner corner of corner {corner}");
        }
    }

    #[test]
    fn the_state_follows_the_picture_and_the_card_is_dropped_when_it_has_gone() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut s = PopupSurface::new(1090, &json!({}), &layout());
        let mut p = up(true);
        s.apply(&mut kit, &p.view(), None, 0, false, 1000).unwrap();
        assert_eq!(s.geometry()["face"], "Connecting");
        let still = crate::kit::picture::Picture { width: 4, height: 4, rgba: vec![200; 64] };
        p.set_can_expand(true);
        let mut v = p.view();
        v.face = Face::Live;
        s.apply(&mut kit, &v, Some(&still), 1, false, 1100).unwrap();
        assert_eq!(s.geometry()["face"], "Live");
        assert!(lint_of(&mut kit, &s).is_empty());
        v.up = false;
        s.apply(&mut kit, &v, None, 2, false, 2000).unwrap();
        assert!(!s.is_up() && s.frame().is_none());
        assert!(s.tick(10_000));
        assert_eq!(s.layers(), 0, "nothing hidden is kept");
    }
}
