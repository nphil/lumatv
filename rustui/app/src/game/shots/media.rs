//! Media viewer shots.
use super::*;

pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = base.clone();
    m.view.page = Page::Game;
    m.view.media = 2;
    m.view.push(Layer::Viewer, Focus::HubTop(0));
    add("39-media-image", m);
}
