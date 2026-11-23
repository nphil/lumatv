//! A game's screenshots and videos.
use crate::game::screens::paint::{H, Paint, W};
use crate::game::model::Focus;
use lumatv_tokens::TypeRole as Type;
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};

pub fn media(p: &mut Paint<'_>) {
    if let Some(item) = p.model.view.selected.and_then(|id| p.model.details.get(&id)).and_then(|d| d.media.get(p.model.view.media)) {
        if let Some(path) = p.model.art_path(&item.image) {
            p.image(100, path, Frame::new(p.x, 140.0, W - p.x * 2.0, H - 220.0), 0.0, true, None, 10);
        }
        p.text(101, &item.title, Frame::new(p.x + 88.0, p.y + 12.0, 1500.0, p.line(Type::Label)), Type::Label, Role::Ink, false, 12);
    }
    p.icon_button(110, Focus::HubTop(0), Icon::Back, p.x, p.y, false, 12);
}
