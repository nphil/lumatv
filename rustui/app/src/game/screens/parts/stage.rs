//! The page canvas, the game's art on the stage, its logo, the target line and the cover tile.
use crate::game::screens::{art::Stage, paint::{Fill, H, Paint, status_word, W}};
use crate::game::model::Focus;
use lumagame_core::{catalog::ArtKind, engine::EngineKind, ids::GameId};
use lumatv_tokens::{Radius, TypeRole as Type};
use std::path::PathBuf;
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};

/// The canvas layer's id: the one layer under every page's art, which the full-screen count of the budget walks leaves out.
pub const CANVAS: u64 = 1;
/// The page's canvas. A curtain page is all stage: its text is light in every theme, so it never sits on a light canvas.
pub fn canvas(p: &mut Paint<'_>, stage: bool) {
    p.solid(CANVAS, Frame::new(0.0, 0.0, W, H), p.ink(Role::Canvas, stage), 0.0, 0);
}
/// The id of the page's stage art. It does not become a retained layer: the art controller holds the one art layer.
pub const ART: u64 = 2;
/// The game's hero art on the stage, behind everything: the picture the page's one art layer shows. It changes by a dip, not at once.
pub fn art(p: &mut Paint<'_>, game: GameId, art_h: f32, fade_from: f32, fade_to: f32, curtain: bool) {
    hero(p, game, art_h, fade_from, fade_to, false, curtain);
}
/// The same art under the theme's scrim, for a page that has a layer open over it: the layer dips to the copy decoded with the
/// picture, where a scrim layer drawn over the art would cost a full-screen blend every frame.
pub fn over_art(p: &mut Paint<'_>, game: GameId, art_h: f32, fade_from: f32, fade_to: f32, curtain: bool) {
    hero(p, game, art_h, fade_from, fade_to, true, curtain);
}
fn hero(p: &mut Paint<'_>, game: GameId, art_h: f32, fade_from: f32, fade_to: f32, dim: bool, curtain: bool) {
    if let Some(path) = p.model.art_url(game, ArtKind::Hero).and_then(|url| p.model.art_path(&url)) {
        let canvas = p.straight(Role::Canvas, curtain);
        let scrim = p.straight(Role::Scrim, curtain);
        p.image(
            ART,
            path,
            Frame::new(0.0, 0.0, W, H),
            0.0,
            false,
            Some(Stage {
                art_height: art_h as u32,
                fade_from: fade_from as u32,
                fade_to: fade_to as u32,
                canvas: [canvas.r, canvas.g, canvas.b],
                scrim: [scrim.r, scrim.g, scrim.b, scrim.a],
                dim,
                curtain,
            }),
            1,
        );
    }
}
/// The game's logo art, when it has some.
pub fn logo_art(p: &Paint<'_>, game: GameId) -> Option<PathBuf> {
    p.model.art_url(game, ArtKind::Logo).and_then(|url| p.model.art_path(&url))
}
/// Whether the game has hero art, which the text over it takes its (stage) ink from; without art the page's own ink applies.
pub fn has_hero(p: &Paint<'_>, game: GameId) -> bool {
    p.model.art_url(game, ArtKind::Hero).and_then(|url| p.model.art_path(&url)).is_some()
}
/// The game's logo, or its title in text when it has no logo art: as many lines as `frame` is tall, never more, so a long title
/// ends in an ellipsis instead of running down into what sits under the frame.
pub fn logo(p: &mut Paint<'_>, id: u64, game: GameId, frame: Frame) {
    let on_art = has_hero(p, game);
    if let Some(path) = logo_art(p, game) {
        p.image(id, path, frame, 0.0, true, None, 10);
    } else if let Some(game) = p.model.game(game) {
        let lines = p.fit(&game.title, Type::Title, frame.w, p.rows(Type::Title, frame.h));
        p.lines(id, lines, frame, Type::Title, Role::Ink, on_art, 10);
    }
}
pub fn target_label(p: &Paint<'_>, game: GameId) -> String {
    p.model.play_target(game).map(|key| p.model.engine_name(&key.engine).to_string()).unwrap_or_else(|| "Choose where to play".into())
}
#[allow(clippy::too_many_arguments)]
pub fn target_line(p: &mut Paint<'_>, id: u64, game: GameId, x: f32, y: f32, w: f32, stage: bool, backed: bool, with_target: bool) {
    let Some(key) = p.model.play_target(game) else {
        return;
    };
    let target = p.model.target(&key);
    let mut text = p.model.engine_name(&key.engine).to_string();
    if with_target {
        if let Some(target) = target {
            if target.name != text {
                text.push_str("  ·  ");
                text.push_str(&target.name);
            }
        }
    }
    let (ready, role) = status_word(p.model, &key);
    let base = p.width(&text, Type::Caption) + 46.0;
    let f = Frame::new(x, y, w, p.line(Type::Caption));
    let icon = match p.model.integration(&key.engine).map(|e| e.manifest.kind) {
        Some(EngineKind::Cloud) => Icon::CloudOutline,
        Some(EngineKind::Host) => Icon::Cards,
        Some(EngineKind::Account) => Icon::Person,
        _ => Icon::Package,
    };
    if backed {
        let width = base + 22.0 + p.width(ready, Type::Caption) + 30.0;
        p.surface(id + 100, Frame::new(x - 14.0, y - 6.0, width, f.h + 12.0), Fill::Sheet, Radius::Pill, false, false, false, 10);
    }
    p.words(
        id,
        f,
        vec![
            p.word(text, Type::Caption, p.ink(Role::Ink2, stage), 38.0, 0.0, base - 38.0),
            p.word(ready, Type::Caption, p.ink(Role::Ink, stage), base + 22.0, 0.0, w - base - 22.0),
        ],
        vec![
            p.mark(icon, 26.0, 0.0, 4.0, Role::Ink2, stage),
            crate::game::screens::paint::Mark { form: ui::components::Form::Disc(12), colour: p.ink(role, stage), x: base, y: 11.0 },
        ],
        11,
    );
}
pub fn cover(p: &mut Paint<'_>, id: u64, game: GameId, focus: Focus, x: f32, y: f32, w: f32) {
    let Some(game) = p.model.game(game) else {
        return;
    };
    let f = Frame::new(x, y, w, w * 1.5);
    let focused = p.focused(focus);
    let start = p.plan.nodes.len();
    p.surface(id, f, Fill::Sheet, Radius::Tile, false, focused, false, 10);
    let count = ui::components::wrap(p.fonts, &game.title, p.bold(Type::Body), w - 48.0).len();
    let h = count as f32 * p.line(Type::Body);
    p.wrapped(id + 1, &game.title, Frame::new(x + 24.0, y + (f.h - h) / 2.0, w - 48.0, h), Type::Body, Role::Ink, false, 11);
    if let Some(path) = p.model.art_url(game.id, ArtKind::Cover).and_then(|url| p.model.art_path(&url)) {
        p.image(id + 5, path, f, p.tokens.radius(Radius::Tile), false, None, 12);
    }
    if focused {
        p.zoom(start, f, p.tokens.focus.tile_scale);
        let caption = Frame::new(x, y + f.h + 26.0, (w * 2.2).min(W - p.x - x), p.line(Type::Body));
        p.text(id + 3, &game.title, caption, Type::Body, Role::Ink, false, 15);
        target_line(p, id + 4, game.id, x, caption.y + caption.h, w * 2.2, false, false, false);
    }
    p.hit(focus, f);
}
