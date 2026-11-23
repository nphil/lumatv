//! A game's trailers and screenshots: the media section of its page and the full-screen viewer.
use crate::game::model::nav::Slice;
use crate::game::model::{Command, Focus, Layer, Model};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Media {
    pub title: String,
    pub image: String,
    pub video: Option<String>,
}
pub struct Rules;
impl Slice for Rules {
    fn rows(_m: &Model, _layer: Option<Layer>) -> Vec<Vec<Focus>> {
        vec![vec![Focus::Media(0)]]
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        if f == Focus::Media(0) {
            m.go_back(out);
        }
    }
    fn adjust(_m: &mut Model, _f: Focus, _forward: bool, _out: &mut Vec<Command>) -> bool {
        false
    }
}
impl Model {
    /// A trailer plays at once; a screenshot opens the viewer.
    pub(in crate::game::model) fn open_media(&mut self, i: usize, out: &mut Vec<Command>) {
        if let Some(media) = self.view.selected.and_then(|id| self.details.get(&id)).and_then(|d| d.media.get(i)) {
            if let Some(url) = &media.video {
                out.push(Command::PlayMedia { url: url.clone() });
            } else {
                self.view.media = i;
                self.push_layer(Layer::Viewer, Focus::Media(0));
            }
        }
    }
}
