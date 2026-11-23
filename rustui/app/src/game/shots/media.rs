//! Media viewer shots.
use super::*;

pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = base.clone();
    m.view.page = Page::Media;
    m.view.media = 2;
    add("39-media-image", m);
}
