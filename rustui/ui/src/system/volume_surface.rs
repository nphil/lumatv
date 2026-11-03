//! The volume display drawn: a slim pill with the format's mark, the level as a number and a thin bar, the slashed
//! speaker in place of the number while muted; the mute mark that stays and hops. Where it sits is the placement crate's
//! (`placement/`: `layout.overlays.volume` in the theme, the bottom edge by default, the left or right edge as a vertical bar,
//! lifted 60 px over an app's picture on the bottom edge: 92 px from the bottom edge in all). The layers are made once per layout (a new
//! width, edge or look makes them again, hidden) and moved, shown, hidden and restyled from then on; a key's bar appears on its
//! own frame, without a fade.

use super::volume::{Source, Subtitles, View};
use crate::kit::Kit;
use crate::kit::glyph;
use crate::kit::motion::Beat;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::text::{Text, role_style, widest};
use compositor_protocol::{Props, Rect, Rgba};
use lumatv_placement::{Anchor, Frame as PlacementFrame, Registry, Size, Sizes, User};
use lumatv_tokens::TypeRole;
use serde_json::Value;
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

const GLYPH: u32 = 32;
const BAR_MIN: f32 = 140.0;
const MARK: f32 = 64.0;

/// The format marks (`icons/audio`), black on transparent, read once and tinted per look.
struct LogoArt {
    name: &'static str,
    width: u32,
    height: u32,
    alpha: Vec<u8>,
}

fn load_logo(dir: &std::path::Path, name: &'static str) -> Option<LogoArt> {
    let file = std::fs::File::open(dir.join(format!("{name}.png"))).ok()?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let alpha = buf[..info.buffer_size()].chunks_exact(4).map(|p| p[3]).collect();
    Some(LogoArt { name, width: info.width, height: info.height, alpha })
}

impl LogoArt {
    /// The mark `h` pixels high (its width by its aspect, at most `max_w`), in `ink`, box-filtered.
    fn render(&self, h: u32, max_w: f32, ink: Rgba) -> (u32, u32, Vec<u8>) {
        let aspect = self.width as f32 / self.height.max(1) as f32;
        let h = (h as f32).min(max_w / aspect).round().max(1.0) as u32;
        let w = (h as f32 * aspect).round().max(1.0) as u32;
        let mut out = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            let (sy0, sy1) = ((y * self.height / h) as usize, (((y + 1) * self.height).div_ceil(h) as usize).min(self.height as usize));
            for x in 0..w {
                let (sx0, sx1) = ((x * self.width / w) as usize, (((x + 1) * self.width).div_ceil(w) as usize).min(self.width as usize));
                let mut sum = 0u32;
                let mut n = 0u32;
                for sy in sy0..sy1.max(sy0 + 1) {
                    for sx in sx0..sx1.max(sx0 + 1) {
                        sum += u32::from(self.alpha[sy * self.width as usize + sx]);
                        n += 1;
                    }
                }
                let a = sum / n.max(1);
                let i = ((y * w + x) * 4) as usize;
                out[i] = (u32::from(ink.0) * a / 255) as u8;
                out[i + 1] = (u32::from(ink.1) * a / 255) as u8;
                out[i + 2] = (u32::from(ink.2) * a / 255) as u8;
                out[i + 3] = (u32::from(ink.3) * a / 255) as u8;
            }
        }
        (w, h, out)
    }
}

/// What the pane holds, as laid out for one width.
struct Built {
    key: (u32, u32, bool, String, u64),
    pane: Pane,
    number: Text,
    muted: ImageLayer,
    track: ShapeLayer,
    fill: ShapeLayer,
    logo: Option<ImageLayer>,
    caption: Option<Text>,
    caption_at: (f32, f32),
    offline: Text,
    /// Where the parts sit relative to the pane's corner.
    number_at: (f32, f32),
    muted_at: (f32, f32),
    track_rect: Frame,
    logo_at: (f32, f32),
    offline_at: (f32, f32),
    vertical: bool,
}

/// The mute mark: a round pane with the slashed speaker.
struct Mark {
    pane: Pane,
    icon: ImageLayer,
    place: usize,
    shown: bool,
    /// A hop in progress: the place to move to and when to come back in.
    pending: Option<(usize, u64)>,
}

pub struct VolumeSurface {
    z: i32,
    logos: Vec<LogoArt>,
    registry: Registry,
    user: User,
    frame: PlacementFrame,
    built: Option<Built>,
    mark: Option<Mark>,
    shown: bool,
    raised: f32,
    last: Option<View>,
    pub edge: Anchor,
}

impl VolumeSurface {
    pub(crate) fn pill_visible(&self) -> bool { self.shown }

    /// `icons` is where the format marks are (`<module>/icons/audio`); `theme` the merged theme (its `layout.overlays` places the
    /// bar), `layout` Home's `layout.json` for the safe area.
    pub fn new(z: i32, icons: &std::path::Path, theme: &Value, layout: &Value) -> Self {
        let registry = Registry::shipped().clone();
        let user = registry.user_from_json(theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        let frame = PlacementFrame::from_json(layout, theme.pointer("/overlays/limit"));
        let logos = ["atmos", "dolby", "dtsx", "dts"].into_iter().filter_map(|n| load_logo(icons, n)).collect();
        let edge = registry.placement_of("volume", &user).map_or(Anchor::Bottom, |p| p.placement.anchor);
        Self { z, logos, registry, user, frame, built: None, mark: None, shown: false, raised: 0.0, last: None, edge }
    }

    /// The bar's size on each edge as the editor draws its stand-in: along the bottom and, turned, up the sides (`Sizes::set_edge`).
    pub fn sample_sizes(&self, kit: &mut Kit, sizes: &mut Sizes) {
        let (w, h, _, _, _) = self.measure(kit, &View::sample());
        let (w, h) = (w as i32, h as i32);
        sizes.set("volume", Size::new(w, h));
        for edge in [Anchor::Left, Anchor::Bottom, Anchor::Right] {
            sizes.set_edge("volume", edge, if edge == Anchor::Bottom { Size::new(w, h) } else { Size::new(h, w) });
        }
    }

    /// The owner's places changed (`layout.overlays`): the bar is made again for its edge at the next draw.
    pub fn set_user(&mut self, user: User) {
        self.edge = self.registry.placement_of("volume", &user).map_or(Anchor::Bottom, |p| p.placement.anchor);
        self.user = user;
        self.built = None;
        self.shown = false;
    }

    /// Where a mute mark the owner placed drifts to (top left corners; `avoid`: the cards that are up are kept clear of), none while the mark
    /// has its own corners.
    pub fn mark_places(&self, avoid: &[lumatv_placement::Rect]) -> Option<Vec<(i32, i32)>> {
        if self.registry.is_auto("mute", &self.user) {
            return None;
        }
        let size = Size::new(MARK as i32, MARK as i32);
        Some(self.registry.drift_spots("mute", &self.user, &self.frame, size, avoid).into_iter().map(|p| (p.x, p.y)).collect())
    }

    fn caption<'a>(&self, view: &'a View) -> &'a str {
        if view.logo.is_empty() || !(view.format.starts_with("PCM") || view.format.starts_with("Multi Ch In")) {
            ""
        } else if self.vertical() {
            view.format.split('·').next().unwrap_or("").trim()
        } else {
            &view.format
        }
    }

    /// The pane's size for `view` at the kit's text size, before any turn to an edge.
    fn measure(&self, kit: &mut Kit, view: &View) -> (f32, f32, f32, f32, f32) {
        let pad_x = kit.space(6);
        let pad_y = kit.space(3);
        let gap = kit.space(5);
        let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), kit.ui.look.ink(), Align::Right);
        let num_w = widest(kit, style, &["100", "88.5"]).max(GLYPH as f32);
        let logo_h = (kit.ty(TypeRole::Label).px as f32 * 1.08).round();
        let logo_w = self.logo_width(view, logo_h);
        let row_h = kit.line(TypeRole::Label).max(logo_h).max(GLYPH as f32);
        let w = (2.0 * pad_x + num_w + gap + BAR_MIN + if logo_w > 0.0 { gap + logo_w } else { 0.0 }).max(440.0);
        let h = (row_h.ceil() + 2.0 * pad_y).max(56.0);
        let caption = self.caption(view);
        let (w, h) = if caption.is_empty() { (w, h) } else {
            let style = role_style(kit, TypeRole::Caption, None, kit.ui.look.ink(), Align::Centre);
            let text_w = widest(kit, style, &[caption]);
            let extra = kit.line(TypeRole::Caption) + kit.space(2);
            if self.vertical() { (w + extra, h.max(text_w + 2.0 * kit.space(2))) }
            else { (w.max(text_w + 2.0 * pad_x), h + extra) }
        };
        (w, h, num_w, logo_w, logo_h)
    }

    fn logo_width(&self, view: &View, logo_h: f32) -> f32 {
        if view.logo.is_empty() {
            return 0.0;
        }
        self.logos.iter().find(|l| l.name == view.logo).map_or(0.0, |l| {
            let aspect = l.width as f32 / l.height.max(1) as f32;
            let h = logo_h.min(logo_h * 7.0 / aspect).round();
            (h * aspect).round()
        })
    }

    fn vertical(&self) -> bool {
        matches!(self.edge, Anchor::Left | Anchor::Right)
    }

    /// Where the pane is: the placement crate's answer for its size on its edge, lifted over an app.
    fn place(&self, w: f32, h: f32, over_video: bool) -> Rect {
        let mut sizes = Sizes::new();
        let size = if self.vertical() { Size::new(h as i32, w as i32) } else { Size::new(w as i32, h as i32) };
        sizes.set("volume", size);
        for edge in [Anchor::Left, Anchor::Bottom, Anchor::Right] {
            sizes.set_edge("volume", edge, if edge == Anchor::Bottom { Size::new(w as i32, h as i32) } else { Size::new(h as i32, w as i32) });
        }
        let laid = self.registry.layout_all(&self.user, &self.frame, &sizes, None, over_video);
        let r = laid.rect("volume").unwrap_or(lumatv_placement::Rect::new((1920 - w as i32) / 2, 1080 - 32 - h as i32, w as i32, h as i32));
        Rect { x: r.x as f32, y: r.y as f32, w: r.w as f32, h: r.h as f32 }
    }

    // Keep the highest position for this showing, so gaps between cues do not make the bar jump.
    fn avoid_subtitles(&mut self, mut at: Rect, view: &View, gap: f32, extent: [f32; 4]) -> Option<Rect> {
        let subtitles = match view.subtitles {
            Subtitles::Unknown if view.over_video => Some(Rect { x: 0.0, y: lumatv_placement::model::SUBS.top as f32, w: 1920.0, h: (lumatv_placement::model::SUBS.bottom - lumatv_placement::model::SUBS.top) as f32 }),
            Subtitles::Bounds { x, y, w, h } => Some(Rect { x, y, w, h }),
            _ => None,
        };
        let mark = self.mark.as_ref().filter(|m| m.shown).map(|m| {
            let f = m.pane.frame();
            Rect { x: f.x, y: f.y, w: f.w, h: f.h }
        });
        let mut raised = self.raised;
        at.y -= raised;
        // An upward move can encounter an obstacle previously above the bar.
        for _ in 0..2 {
            for r in [subtitles, mark].into_iter().flatten() {
                let left = (at.x + extent[0]).floor();
                let top = (at.y + extent[1]).floor();
                let right = (at.x + extent[2]).ceil();
                let bottom = (at.y + extent[3]).ceil();
                if left < r.x + r.w + gap && right + gap > r.x
                    && top < r.y + r.h + gap && bottom + gap > r.y
                {
                    let lift = (bottom + gap - r.y).ceil();
                    raised += lift;
                    at.y -= lift;
                }
            }
        }
        if at.y < self.frame.safe_y as f32 { return None; }
        self.raised = raised;
        Some(at)
    }

    fn build(&mut self, kit: &mut Kit, view: &View) -> std::io::Result<Built> {
        let (w, h, num_w, logo_w, logo_h) = self.measure(kit, view);
        let vertical = self.vertical();
        let (pw, ph) = if vertical { (h, w) } else { (w, h) };
        let caption_text = self.caption(view);
        let caption_h = if caption_text.is_empty() { 0.0 } else { kit.line(TypeRole::Caption) + kit.space(2) };
        let body_h = ph - caption_h;
        let z = self.z;
        let pad_x = kit.space(6);
        let gap = kit.space(5);
        let ink = kit.ui.look.ink();
        let hidden = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
        let pane = Pane::new(kit, Frame::new(0.0, 0.0, pw, ph), z, PaneKind::Pill, view.over_video);
        pane.layer().set(hidden);
        let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, if vertical { Align::Centre } else { Align::Right });
        let line = kit.line(TypeRole::Label);
        let track_h = kit.space(2);
        // The vertical edge keeps the number at the top and the mark at the bottom.
        let (number_at, number_w, track_rect, logo_at, muted_at) = if vertical {
            let number_at = (0.0, pad_x);
            let bar_top = pad_x + line + gap;
            let bar_bottom = body_h - pad_x - if logo_w > 0.0 { gap + logo_h } else { 0.0 };
            (
                number_at,
                pw,
                Frame::new(((pw - track_h) / 2.0).round(), bar_top, track_h, (bar_bottom - bar_top).max(BAR_MIN / 2.0)),
                (((pw - logo_h) / 2.0).round(), body_h - pad_x - logo_h),
                (((pw - GLYPH as f32) / 2.0).round(), pad_x + ((line - GLYPH as f32) / 2.0).round()),
            )
        } else {
            let number_x = pad_x + if logo_w > 0.0 { logo_w + gap } else { 0.0 };
            let x = number_x + num_w + gap;
            let track_w = pw - x - pad_x;
            (
                (number_x, ((body_h - line) / 2.0).round()),
                num_w,
                Frame::new(x, ((body_h - track_h) / 2.0).round(), track_w, track_h),
                (pad_x, ((body_h - logo_h) / 2.0).round()),
                (number_x + ((num_w - GLYPH as f32) / 2.0).round(), ((body_h - GLYPH as f32) / 2.0).round()),
            )
        };
        let mut number = Text::new(kit, Frame::new(number_at.0, number_at.1, number_w, line), z + 3, style, "\u{2013}")?;
        number.set_props(hidden);
        let mut muted = kit.ui.scene.image(GLYPH, GLYPH, z + 3, hidden)?;
        glyph::draw("volume-x", GLYPH, ink, muted.pixels_mut());
        muted.damage(None);
        let track_off = kit.ui.look.theme.materials.track_off;
        let radius = track_h / 2.0;
        let track = plate::shape(&kit.ui, track_rect, z + 1, plate::solid(track_off, radius));
        track.set(hidden);
        let fill = plate::shape(&kit.ui, track_rect, z + 2, plate::solid(ink, radius));
        fill.set(hidden);
        let logo = match self.logos.iter().find(|l| l.name == view.logo) {
            Some(art) if logo_w > 0.0 => {
                let (lw, lh, pixels) = art.render(logo_h as u32, logo_h * 7.0, ink);
                let mut layer = kit.ui.scene.image(lw, lh, z + 3, hidden)?;
                layer.pixels_mut().copy_from_slice(&pixels);
                layer.damage(None);
                Some(layer)
            }
            _ => None,
        };
        let caption_style = role_style(kit, TypeRole::Caption, None, ink, Align::Centre);
        let cap_line = kit.line(TypeRole::Caption);
        let offline_at = if vertical { (0.0, ((ph - cap_line) / 2.0).round()) } else { (pad_x, ((ph - cap_line) / 2.0).round()) };
        let caption_at = (if vertical { 0.0 } else { pad_x }, ph - if vertical { pad_x } else { kit.space(3) } - cap_line);
        let caption = if caption_text.is_empty() { None } else {
            let mut text = Text::new(kit, Frame::new(caption_at.0, caption_at.1, if vertical { pw } else { pw - 2.0 * pad_x }, cap_line), z + 3, caption_style, caption_text)?;
            text.set_props(hidden);
            Some(text)
        };
        let mut offline = Text::new(
            kit,
            Frame::new(offline_at.0, offline_at.1, if vertical { pw } else { pw - 2.0 * pad_x }, cap_line),
            z + 3,
            caption_style,
            "Receiver not answering",
        )?;
        offline.set_props(hidden);
        Ok(Built {
            key: (pw as u32, ph as u32, view.over_video, view.logo.clone(), kit.generation()),
            pane,
            number,
            muted,
            track,
            fill,
            logo,
            caption,
            caption_at,
            offline,
            number_at,
            muted_at,
            track_rect,
            logo_at,
            offline_at,
            vertical,
        })
    }

    /// Draws `view`, at `now` (monotonic ms). Returns whether anything of the display is on screen.
    pub fn apply(&mut self, kit: &mut Kit, view: &View, now: u64) -> std::io::Result<bool> {
        if !view.up { self.raised = 0.0; }
        let (w, h, _, _, _) = self.measure(kit, view);
        let base = self.place(w, h, view.over_video);
        let (pw, ph) = if self.vertical() { (h, w) } else { (w, h) };
        let glass = crate::kit::pane::glass(kit, PaneKind::Pill, ph, view.over_video);
        let extent = lumatv_render::renderer::glass_extent(&glass, (pw.round(), ph.round()), 1.0);
        let at = if view.up { self.avoid_subtitles(base, view, kit.space(2), extent) } else { None };
        let blocked = view.up && at.is_none();
        if let Some(at) = at {
            let (pw, ph) = if self.vertical() { (h, w) } else { (w, h) };
            let key = (pw as u32, ph as u32, view.over_video, view.logo.clone(), kit.generation());
            if self.built.as_ref().is_none_or(|b| b.key != key) {
                let built = self.build(kit, view)?;
                self.built = Some(built);
                self.shown = false;
            }
            let caption_text = self.caption(view);
            let snap = self.raised > 0.0 || matches!(view.subtitles, Subtitles::Bounds { .. }) || (view.over_video && view.subtitles == Subtitles::Unknown);
            let b = self.built.as_mut().expect("built above");
            let muted = view.muted && view.source != Source::None;
            let number_props = Props::at(at.x + b.number_at.0, at.y + b.number_at.1);
            let muted_props = Props::at(at.x + b.muted_at.0, at.y + b.muted_at.1);
            let track_props = Props::at(at.x + b.track_rect.x, at.y + b.track_rect.y);
            let fraction = view.fraction.clamp(0.0, 1.0);
            let fill_clip = if b.vertical {
                let fh = (b.track_rect.h * fraction).round();
                Rect { x: track_props.x, y: track_props.y + b.track_rect.h - fh, w: b.track_rect.w, h: fh }
            } else {
                Rect { x: track_props.x, y: track_props.y, w: (b.track_rect.w * fraction).round(), h: b.track_rect.h }
            };
            let fill_props = Props { clip: Some(fill_clip), ..track_props };
            let ink = if muted { kit.ui.look.ink3() } else { kit.ui.look.ink() };
            b.fill.reshape(plate::solid(ink, b.track_rect.h / 2.0));
            b.number.set(kit, if view.level.is_empty() { "\u{2013}" } else { &view.level });
            let pane_props = Props::at(at.x, at.y);
            let was_shown = self.shown;
            let moved = b.pane.frame().x != at.x || b.pane.frame().y != at.y;
            if !was_shown {
                // in on the key's own frame
                kit.arrive(b.pane.layer(), pane_props, Beat::Rest, true);
                b.pane.move_to(at.x, at.y);
                b.track.set(track_props);
                b.fill.set(fill_props);
            } else if snap {
                b.pane.layer().set(pane_props);
                b.pane.move_to(at.x, at.y);
                b.track.set(track_props);
                b.fill.set(fill_props);
            } else if moved {
                // the app came or went under a bar that is showing: it glides
                kit.glide(b.pane.layer(), pane_props, Beat::Card);
                b.pane.move_to(at.x, at.y);
                kit.glide(&b.track, track_props, Beat::Card);
                kit.glide(&b.fill, fill_props, Beat::Card);
            } else {
                // the level changes: the fill follows over the focus beat
                kit.glide(&b.fill, fill_props, Beat::Focus);
            }
            let show = |layer: &ui::scene::Layer, p: Props, on: bool| layer.set(if on { p } else { Props { opacity: 0.0, visible: false, ..p } });
            let readable = view.source != Source::None;
            show(&b.track, track_props, readable);
            show(&b.fill, fill_props, readable);
            show(b.number.layer(), number_props, readable && !muted);
            show(&b.muted, muted_props, readable && muted);
            if let Some(logo) = &b.logo {
                show(logo, Props::at(at.x + b.logo_at.0, at.y + b.logo_at.1), readable && !view.logo.is_empty());
            }
            if let Some(caption) = &mut b.caption {
                caption.set(kit, caption_text);
                show(caption.layer(), Props::at(at.x + b.caption_at.0, at.y + b.caption_at.1), readable && !muted && !view.logo.is_empty());
            }
            show(b.offline.layer(), Props::at(at.x + b.offline_at.0, at.y + b.offline_at.1), view.offline);
            self.shown = true;
        } else if self.shown {
            if let Some(b) = &self.built {
                let from = b.pane.at();
                if blocked {
                    b.pane.layer().set(Props { opacity: 0.0, visible: false, ..from });
                } else {
                    kit.leave(b.pane.layer(), from, Beat::Rest);
                }
                for layer in [&*b.track, &*b.fill, b.number.layer(), &*b.muted, b.offline.layer()] {
                    layer.set(Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) });
                }
                if let Some(logo) = &b.logo {
                    logo.set(Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) });
                }
                if let Some(caption) = &b.caption {
                    caption.layer().set(Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) });
                }
            }
            self.shown = false;
        }
        self.apply_mark(kit, view, now)?;
        self.last = Some(view.clone());
        Ok(self.shown || self.mark.as_ref().is_some_and(|m| m.shown))
    }

    fn mark_spot(view: &View, place: usize) -> (f32, f32) {
        let (x, y) = view.spots[place.min(view.spots.len().saturating_sub(1))];
        (x as f32, y as f32)
    }

    fn apply_mark(&mut self, kit: &mut Kit, view: &View, now: u64) -> std::io::Result<()> {
        if view.mark_up {
            if self.mark.as_ref().is_none_or(|m| m.pane.over_video() != view.over_video) {
                let hidden = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
                let pane = Pane::new(kit, Frame::new(0.0, 0.0, MARK, MARK), self.z + 4, PaneKind::Pill, view.over_video);
                pane.layer().set(hidden);
                let mut icon = kit.ui.scene.image(GLYPH, GLYPH, self.z + 5, hidden)?;
                glyph::draw("volume-x", GLYPH, kit.ui.look.ink(), icon.pixels_mut());
                icon.damage(None);
                self.mark = Some(Mark { pane, icon, place: view.mark_place, shown: false, pending: None });
            }
            let m = self.mark.as_mut().expect("made above");
            if !m.shown {
                m.place = view.mark_place;
                let (x, y) = Self::mark_spot(view, m.place);
                m.pane.move_to(x, y);
                kit.arrive(m.pane.layer(), Props::at(x, y), Beat::Rest, false);
                kit.arrive(&m.icon, Props::at(x + (MARK - GLYPH as f32) / 2.0, y + (MARK - GLYPH as f32) / 2.0), Beat::Rest, false);
                m.shown = true;
            } else if m.place != view.mark_place && m.pending.is_none() {
                // out, to the next place (the glass is told where it is now), and back in after a pause
                let from = m.pane.at();
                kit.leave(m.pane.layer(), from, Beat::Rest);
                kit.leave(&m.icon, Props::at(from.x + (MARK - GLYPH as f32) / 2.0, from.y + (MARK - GLYPH as f32) / 2.0), Beat::Rest);
                m.pending = Some((view.mark_place, now + u64::from(kit.beat_ms(Beat::Rest)) + 400));
            }
        } else if let Some(m) = &mut self.mark
            && m.shown
        {
            let from = m.pane.at();
            kit.leave(m.pane.layer(), from, Beat::Rest);
            kit.leave(&m.icon, Props::at(from.x + (MARK - GLYPH as f32) / 2.0, from.y + (MARK - GLYPH as f32) / 2.0), Beat::Rest);
            m.shown = false;
            m.pending = None;
        }
        Ok(())
    }

    /// Finishes a hop whose pause is over. Returns whether anything changed.
    pub fn tick(&mut self, kit: &mut Kit, now: u64) -> bool {
        let Some(view) = self.last.clone() else { return false };
        let Some(m) = &mut self.mark else { return false };
        let Some((place, at)) = m.pending else { return false };
        if now < at {
            return false;
        }
        m.pending = None;
        m.place = place;
        let (x, y) = Self::mark_spot(&view, place);
        m.pane.move_to(x, y);
        if m.shown && view.mark_up {
            kit.arrive(m.pane.layer(), Props::at(x, y), Beat::Rest, false);
            kit.arrive(&m.icon, Props::at(x + (MARK - GLYPH as f32) / 2.0, y + (MARK - GLYPH as f32) / 2.0), Beat::Rest, false);
        }
        true
    }

    /// When the surface wants a tick of its own.
    pub fn due_at(&self) -> Option<u64> {
        self.mark.as_ref().and_then(|m| m.pending.map(|(_, at)| at))
    }

    /// The pane's rectangle while it is up (the tests read where it is).
    pub fn pane_rect(&self) -> Option<Rect> {
        self.built.as_ref().filter(|_| self.shown).map(|b| {
            let f = b.pane.frame();
            Rect { x: f.x, y: f.y, w: f.w, h: f.h }
        })
    }

    pub fn is_up(&self) -> bool {
        self.shown
    }

    /// The look changed: everything is made again at the next draw.
    pub fn restyle(&mut self) {
        self.built = None;
        self.mark = None;
        self.shown = false;
    }

    /// The ink of the number: for the host's checks of the picture.
    pub fn ink(kit: &Kit) -> Rgba {
        kit.ui.look.colour(PaletteRole::Ink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use lumatv_placement::{Placement, Rect as PlaceRect};
    use serde_json::json;

    fn layout() -> Value {
        serde_json::from_str(include_str!("../../../../payload/theme/layout.json")).unwrap()
    }

    #[test]
    fn subtitle_collision_is_minimal_stable_and_hides_when_no_safe_position_exists() {
        let (_host, mut kit) = testkit::kit(1.0);
        let mut surface = VolumeSurface::new(1000, std::path::Path::new("/nonexistent"), &json!({}), &layout());
        let mut view = View::sample();
        view.over_video = true;
        view.subtitles = Subtitles::Empty;
        surface.apply(&mut kit, &view, 0).unwrap();
        let base = surface.pane_rect().unwrap();
        let gap = kit.space(2);
        view.subtitles = Subtitles::Bounds { x: base.x, y: base.y + base.h - 10.0, w: base.w, h: 60.0 };
        surface.apply(&mut kit, &view, 1).unwrap();
        let raised = surface.pane_rect().unwrap();
        let glass = crate::kit::pane::glass(&kit, PaneKind::Pill, base.h, true);
        let extent = lumatv_render::renderer::glass_extent(&glass, (base.w, base.h), 1.0);
        assert_eq!(raised.y, base.y - (10.0 + gap + extent[3] - base.h).ceil());
        view.subtitles = Subtitles::Empty;
        surface.apply(&mut kit, &view, 2).unwrap();
        assert_eq!(surface.pane_rect().unwrap().y, raised.y);
        view.subtitles = Subtitles::Bounds { x: 0.0, y: 1.0, w: 1920.0, h: 1079.0 };
        surface.apply(&mut kit, &view, 3).unwrap();
        assert!(surface.pane_rect().is_none());
        view.subtitles = Subtitles::Empty;
        surface.apply(&mut kit, &view, 4).unwrap();
        assert_eq!(surface.pane_rect().unwrap().y, raised.y);
        view.up = false;
        surface.apply(&mut kit, &view, 5).unwrap();
        view.up = true;
        surface.apply(&mut kit, &view, 6).unwrap();
        assert_eq!(surface.pane_rect().unwrap().y, base.y);
    }

    #[test]
    fn unknown_uses_the_fallback_band_but_side_bars_and_distant_cues_stay_put() {
        let (_host, mut kit) = testkit::kit(1.0);
        let mut surface = VolumeSurface::new(1000, std::path::Path::new("/nonexistent"), &json!({}), &layout());
        let mut view = View::sample();
        view.over_video = true;
        view.subtitles = Subtitles::Empty;
        surface.apply(&mut kit, &view, 0).unwrap();
        let base = surface.pane_rect().unwrap();
        view.subtitles = Subtitles::Bounds { x: 0.0, y: base.y, w: 50.0, h: base.h };
        surface.apply(&mut kit, &view, 1).unwrap();
        assert_eq!(surface.pane_rect().unwrap().y, base.y);
        view.subtitles = Subtitles::Unknown;
        surface.apply(&mut kit, &view, 2).unwrap();
        let raised = surface.pane_rect().unwrap();
        assert!(raised.y + raised.h + kit.space(2) <= lumatv_placement::model::SUBS.top as f32);
        // Vertical bars use the same painted-bound protection.
        for edge in [Anchor::Left, Anchor::Right] {
            surface.edge = edge;
            let r = Rect { x: 100.0, y: 400.0, w: 56.0, h: 440.0 };
            let extent = [0.0, 0.0, r.w, r.h];
            assert!(surface.avoid_subtitles(r, &view, kit.space(2), extent).is_some());
        }
    }

    #[test]
    fn raised_bar_keeps_clear_of_the_disappearing_mute_mark() {
        let (_host, mut kit) = testkit::kit(1.0);
        let mut surface = VolumeSurface::new(1000, std::path::Path::new("/nonexistent"), &json!({}), &layout());
        let mut view = View::sample();
        view.up = false;
        view.mark_up = true;
        view.spots = vec![(740, 200)];
        surface.apply(&mut kit, &view, 0).unwrap();
        view.up = true;
        view.mark_up = false;
        view.subtitles = Subtitles::Bounds { x: 0.0, y: 250.0, w: 1920.0, h: 750.0 };
        surface.apply(&mut kit, &view, 1).unwrap();
        let bar = surface.pane_rect().unwrap();
        assert!(bar.y + bar.h + kit.space(2) <= 200.0);
    }
    #[test]
    fn caption_safe_pill_survives_native_frame_guard() {
        use lumatv_render::host::{Engine, Exclusion, HostConfig};
        use lumatv_render::tiles::TileSet;
        use lumatv_render::Gles;
        use ui::lucent::{material::MaterialTier, palette::Mode};
        use ui::SceneOptions;

        let gl = Gles::software_pbuffer(c"libEGL.so.1", 1920, 1080).unwrap();
        let target = gl.new_render_target(1920, 1080).unwrap();
        let wrapped = unsafe { Gles::current(c"libEGL.so.1") }.unwrap();
        let shm = std::env::temp_dir().join(format!("lumatv-volume-caption-{}", std::process::id()));
        std::fs::create_dir_all(&shm).unwrap();
        let (mut engine, socket) = Engine::new(wrapped, HostConfig {
            width: 1920, height: 1080, refresh_hz: 60.0, shm_dir: shm.clone(), direct: false,
            gpu_sync: false, idle_trim: false, pages: 0, reference: false, blit: false,
            external: true, system_z: 100,
        }).unwrap();
        let (scene, _, events) = ui::Scene::over_with(socket, SceneOptions { shm_dir: Some(shm.clone()), ..Default::default() }).unwrap();
        engine.finish_handshake().unwrap();
        let mut tokens = lumatv_tokens::Tokens::shipped();
        let mut now = lumatv_render::now_ns();
        for depth in [1.0, 1.5, 0.0] {
            tokens.depth_scale = depth;
            let mut kit = Kit::new(scene.clone(), tokens.clone(), Mode::Dark, MaterialTier::T1, 1.0).unwrap();
            let _ground = kit.ui.scene.shape(1920, 1080, 0, ui::components::plate::solid(Rgba(20, 30, 40, 255), 0.0), Props::at(0.0, 0.0));
            let mut surface = VolumeSurface::new(100, std::path::Path::new("/nonexistent"), &json!({}), &layout());
            let mut view = View::sample();
            view.over_video = true;
            for y in [938, 900] {
                view.subtitles = Subtitles::Bounds { x: 709.0, y: y as f32, w: 518.0, h: 55.0 };
                surface.apply(&mut kit, &view, 0).unwrap();
                let pill = surface.pane_rect().unwrap();
                engine.set_exclusion(Some(Exclusion { area: [709, y, 1227, y + 55], popup_z: (100, 104), marker_z: (104, 106) }));
                for _ in 0..30 {
                    while let Ok(event) = events.try_recv() { kit.ui.scene.observe(&event); }
                    if let Some(atlas) = kit.ui.text_atlas() { atlas.pump(); }
                    if kit.ui.scene.has_pending() { let _ = kit.ui.try_commit().unwrap(); }
                    now += 16_666_667;
                    engine.pump().unwrap();
                    engine.plan(now).unwrap();
                    let mut damage = TileSet::new(1920, 1080);
                    damage.fill();
                    let mut areas = Vec::new();
                    damage.rects(&mut areas);
                    engine.draw_stack_direct(0, target.framebuffer, &areas).unwrap();
                    engine.draw_stack(1, target.framebuffer, &areas).unwrap();
                    let cost = engine.finish().unwrap();
                    assert_eq!(cost.gl_error, None);
                    engine.frame_done(now, &cost);
                    engine.tidy();
                    gl.bind_framebuffer(target.framebuffer);
                    let centre = gl.read_pixels((pill.x + pill.w / 2.0) as i32, (pill.y + pill.h / 2.0) as i32, 1, 1);
                    let ground = gl.read_pixels(0, 0, 1, 1);
                    if ground == [20, 30, 40, 255] && centre != ground { break; }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                gl.bind_framebuffer(target.framebuffer);
                assert_eq!(gl.read_pixels(0, 0, 1, 1), [20, 30, 40, 255], "ground did not arrive");
                assert_ne!(gl.read_pixels((pill.x + pill.w / 2.0) as i32, (pill.y + pill.h / 2.0) as i32, 1, 1), [20, 30, 40, 255], "popup suppressed at depth {depth}, cue {y}");
                assert!(gl.read_pixels(709, y, 518, 55).chunks_exact(4).all(|p| p == [20, 30, 40, 255]), "popup painted over caption at depth {depth}, cue {y}");
            }
        }
        let _ = std::fs::remove_dir_all(shm);
    }


    #[test]
    fn real_format_marks_lead_the_horizontal_bar_and_keep_the_vertical_order() {
        let (_h, mut kit) = testkit::kit(1.0);
        let icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../compositor/assets/ui/icons/audio");
        let mut surface = VolumeSurface::new(1000, &icons, &json!({}), &layout());
        let mut view = View::sample();
        view.source = Source::Receiver;
        let pad = kit.space(6);
        let gap = kit.space(5);

        for (name, width) in [("atmos", 438), ("dolby", 984), ("dts", 286), ("dtsx", 316)] {
            view.logo = name.into();
            let art = surface.logos.iter().find(|art| art.name == name).expect("real mark decoded");
            assert_eq!((art.width, art.height), (width, 112));
            let (_, _, num_w, logo_w, logo_h) = surface.measure(&mut kit, &view);
            let (render_w, render_h, pixels) = art.render(logo_h as u32, logo_h * 7.0, Rgba(90, 120, 150, 255));
            assert_eq!(render_w as f32, logo_w);
            assert!(render_h > 0 && pixels.chunks_exact(4).any(|p| p[3] > 0));
            assert!(pixels.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
            let built = surface.build(&mut kit, &view).unwrap();
            assert!(built.logo.is_some());
            assert_eq!(built.logo_at.0, pad);
            assert_eq!(built.number_at.0, pad + logo_w + gap);
            assert_eq!(built.track_rect.x, built.number_at.0 + num_w + gap);
            assert_eq!(built.muted_at.0, built.number_at.0 + ((num_w - GLYPH as f32) / 2.0).round());
            assert!(built.track_rect.w >= BAR_MIN);
        }

        for logo in ["", "unknown", "pcm"] {
            view.logo = logo.into();
            view.muted = logo.is_empty();
            let built = surface.build(&mut kit, &view).unwrap();
            assert!(built.logo.is_none());
            assert_eq!(built.number_at.0, pad);
            assert_eq!(built.muted_at.0, pad + ((surface.measure(&mut kit, &view).2 - GLYPH as f32) / 2.0).round());
        }

        view.muted = false;
        view.logo = "atmos".into();
        for edge in [Anchor::Left, Anchor::Right] {
            surface.edge = edge;
            let built = surface.build(&mut kit, &view).unwrap();
            assert!(built.vertical);
            assert!(built.number_at.1 < built.track_rect.y);
            assert!(built.track_rect.y + built.track_rect.h < built.logo_at.1);
        }
    }

    #[test]
    fn a_mark_the_owner_has_not_placed_has_its_own_corners_and_one_that_is_placed_drifts_in_a_small_square_leaning_into_the_screen() {
        let (_h, _kit) = testkit::kit(1.0);
        let icons = std::path::Path::new("/nonexistent");
        let auto = VolumeSurface::new(1000, icons, &json!({}), &layout());
        assert_eq!(auto.mark_places(&[]), None, "not placed: the corners the mark had before");
        let user = Registry::shipped().with_placement(&User::default(), "mute", Placement::new(Anchor::TopRight, 0.0, 0.0));
        let theme = json!({"layout": {"overlays": user.to_json()}});
        let placed = VolumeSurface::new(1000, icons, &theme, &layout());
        let spots = placed.mark_places(&[]).expect("placed: four places");
        assert_eq!(spots.len(), 4);
        let (xs, ys): (Vec<i32>, Vec<i32>) = spots.iter().copied().unzip();
        assert_eq!(xs.iter().max(), Some(&(1920 - 64 - 64)), "flush at the zone's right edge, 64 in from the screen's: {spots:?}");
        assert_eq!(xs.iter().max().unwrap() - xs.iter().min().unwrap(), 48, "a square of 48");
        assert_eq!(ys.iter().min(), Some(&134), "never above the status bar's row");
        // a card over the first place: the mark keeps off it
        let card = PlaceRect::new(spots[0].0 + 38, spots[0].1, 20, 26);
        let clear = placed.mark_places(&[card]).unwrap();
        assert_eq!(clear.len(), 3, "the place under the card is left out: {clear:?}");
        assert!(clear.iter().all(|s| !PlaceRect::new(s.0, s.1, 64, 64).meets(&card, 0)));
    }
}
