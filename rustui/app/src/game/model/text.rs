//! The on-screen keyboard and the text entry it drives: search, the PIN, a host's address and the text fields of Settings and
//! the integrations.
use super::nav::Slice;
use super::settings::Place;
use super::*;

pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        let mut rows: Vec<Vec<Focus>> = Vec::new();
        let count = if layer == Some(Layer::EditText) && m.view.page == Page::Settings && m.view.place == Place::Diagnostics { 45 } else { 44 };
        for i in (0..count).step_by(10) {
            rows.push((i..(i + 10).min(count)).map(Focus::Keyboard).collect());
        }
        rows.push(vec![Focus::SheetAction(0)]);
        rows
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        m.text_activate(f, out);
    }
    fn adjust(_m: &mut Model, _f: Focus, _forward: bool, _out: &mut Vec<Command>) -> bool {
        false
    }
}
impl Model {
    fn text_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match f {
            Focus::Keyboard(i) => self.keyboard(i, out),
            Focus::SheetAction(_) if self.view.top() == Some(Layer::EditText) => self.save_text(out),
            Focus::SheetAction(_) if self.view.top() == Some(Layer::AddHost) && !self.view.address.is_empty() => {
                if let Some(id) = &self.view.selected_engine {
                    Self::send(out, id, Request::AddTarget { address: self.view.address.clone() });
                }
                self.pop_layer();
            }
            _ => {}
        }
    }
    /// Saves the open text field to whichever slice owns it.
    fn save_text(&mut self, out: &mut Vec<Command>) {
        if self.view.pane == Some(settings::Pane::Leadjoy) {
            self.save_leadjoy_text(out);
        } else if self.view.page == Page::Settings && self.view.place == Place::Diagnostics {
            self.save_diagnostics_text(out);
        } else {
            self.save_engine_text(out);
        }
    }
    pub(super) fn keyboard(&mut self, i: usize, out: &mut Vec<Command>) {
        let numeric = self.view.top() == Some(Layer::Auth);
        let diagnostic = self.view.page == Page::Settings && self.view.place == Place::Diagnostics && self.view.top() == Some(Layer::EditText);
        let chars = if numeric {
            "1234567890"
        } else {
            match (self.view.keyboard_lower, diagnostic) {
                (true, true) => "abcdefghijklmnopqrstuvwxyz0123456789.-:/_",
                (false, true) => "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/_",
                (true, false) => "abcdefghijklmnopqrstuvwxyz0123456789.-:/",
                (false, false) => "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/",
            }
        };
        if !numeric && i == chars.len() + 3 {
            self.view.keyboard_lower = !self.view.keyboard_lower;
            return;
        }
        let limit = if numeric {
            self.view
                .auth
                .as_ref()
                .and_then(|a| match a.state {
                    Some(AuthState::EnterPin { digits }) => Some(usize::from(digits)),
                    _ => None,
                })
                .unwrap_or(12)
        } else if !self.view.layers.is_empty() {
            255
        } else {
            128
        };
        let text = match self.view.top() {
            Some(Layer::Auth) => self.view.auth.as_mut().map(|a| &mut a.pin),
            Some(Layer::AddHost | Layer::EditText) => Some(&mut self.view.address),
            _ => Some(&mut self.view.query),
        };
        if let Some(text) = text {
            if let Some(c) = chars.chars().nth(i) {
                if text.chars().count() < limit {
                    text.push(c);
                }
            } else if i == chars.len() {
                text.pop();
            } else if i == chars.len() + 1 && !numeric {
                if text.chars().count() < limit {
                    text.push(' ');
                }
            } else if numeric {
                self.auth_action(0, out);
            } else if self.view.top() == Some(Layer::EditText) {
                self.save_text(out);
            } else if self.view.top() == Some(Layer::AddHost) {
                if let Some(id) = &self.view.selected_engine {
                    Self::send(out, id, Request::AddTarget { address: self.view.address.clone() });
                }
                self.pop_layer();
            } else {
                self.view.focus = Focus::Grid(0);
            }
        }
        if self.view.layers.is_empty() {
            self.rebuild_filter();
        }
    }
}
