//! The layer drawn over the page: picks the open layer's painter.
use crate::game::screens::paint::Paint;
use crate::game::model::{Layer, library::Sort};
use crate::game::screens::parts::{dialog::*, keyboard::*};
use crate::game::screens::{accounts::auth, game::chooser, launch::end_session, library::list, settings::{delete, themes}};

pub fn sheet(p: &mut Paint<'_>) {
    let Some(sheet) = p.model.view.sheet else {
        return;
    };
    curtain(p);
    match sheet {
        Layer::Chooser => chooser(p),
        Layer::Sort => list(p, "Sort by", Sort::ALL.iter().map(|s| s.name().into()).collect()),
        Layer::Genre => {
            let mut names = vec!["All genres".into()];
            names.extend(p.model.genres());
            list(p, "Genre", names);
        }
        Layer::Theme => themes(p),
        Layer::Auth => auth(p),
        Layer::EndSession => end_session(p),
        Layer::AddHost | Layer::EditText => edit_text(p),
        Layer::Delete => delete(p),
    }
}
