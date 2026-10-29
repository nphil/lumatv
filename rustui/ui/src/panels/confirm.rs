//! A question with two answers (`docs/mocks/06-confirm-dialog`, `panels/Confirm.qml` of the QML shell): "Uninstall Crunchyroll?" with the
//! app's icon, what happens, and Keep it / Uninstall. The safe answer is focused first; Back is that answer. The dialog is given its text
//! when it opens (`ask`); the answer is told to the owner as the event `confirm.answered {id}` and the dialog closes, so a dialog never
//! does anything itself. A danger answer is drawn in the danger colour.
//!
//! ```text
//! spec = { icon | iconUrl, title, body, answers: [{ id, label, danger, icon }], focus: 0 }
//! ```

use super::life::Life;
use super::{K, Out};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub id: String,
    pub label: String,
    pub danger: bool,
    pub icon: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub icon: String,
    pub icon_url: String,
    pub title: String,
    pub body: String,
    pub answers: Vec<Answer>,
    pub focus: usize,
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

impl Spec {
    pub fn from_json(v: &Value) -> Spec {
        Spec {
            icon: text(v, "icon"),
            icon_url: text(v, "iconUrl"),
            title: text(v, "title"),
            body: text(v, "body"),
            answers: v
                .get("answers")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|x| Answer { id: text(x, "id"), label: text(x, "label"), danger: x.get("danger").and_then(Value::as_bool) == Some(true), icon: text(x, "icon") }).collect())
                .unwrap_or_default(),
            focus: v.get("focus").and_then(Value::as_u64).unwrap_or(0) as usize,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub spec: Spec,
    pub index: usize,
    pub over_video: bool,
}

#[derive(Debug, Default)]
pub struct Confirm {
    pub life: Life,
    spec: Option<Spec>,
    index: usize,
    up: bool,
}

impl Confirm {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_up(&self) -> bool {
        self.up
    }

    pub fn spec(&self) -> Option<&Spec> {
        self.spec.as_ref()
    }

    pub fn ask(&mut self, spec: Spec) {
        self.index = spec.focus.min(spec.answers.len().saturating_sub(1));
        self.spec = Some(spec);
        self.up = true;
    }

    pub fn close(&mut self) {
        self.up = false;
    }

    pub fn index(&self) -> usize {
        self.index
    }

    fn answered(id: &str) -> Out {
        let mut out = Out::close();
        out.events.push(("confirm.answered".into(), json!({"id": id})));
        out
    }

    pub fn key(&mut self, k: K) -> Out {
        let Some(spec) = self.spec.as_ref() else { return Out::close() };
        match k {
            K::Left => self.index = self.index.saturating_sub(1),
            K::Right => self.index = (self.index + 1).min(spec.answers.len().saturating_sub(1)),
            K::Ok => {
                if let Some(a) = spec.answers.get(self.index) {
                    return Self::answered(&a.id);
                }
            }
            // Back is the safe answer: the one that was focused first
            K::Back => return Self::answered(spec.answers.get(spec.focus).map_or("", |a| a.id.as_str())),
            _ => {}
        }
        Out::default()
    }

    pub fn click(&mut self, i: usize) -> Out {
        self.index = i;
        self.key(K::Ok)
    }

    pub fn view(&self, over_video: bool) -> Option<View> {
        self.spec.as_ref().map(|s| View { spec: s.clone(), index: self.index, over_video })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uninstall() -> Spec {
        Spec::from_json(&json!({
            "iconUrl": "/media/cr.png", "title": "Uninstall Crunchyroll?", "body": "The app and its data are removed.",
            "answers": [{"id": "keep", "label": "Keep it"}, {"id": "uninstall", "label": "Uninstall", "danger": true}], "focus": 0
        }))
    }

    #[test]
    fn the_safe_answer_is_focused_first_ok_gives_it_and_the_dialog_closes() {
        let mut c = Confirm::new();
        c.ask(uninstall());
        assert_eq!(c.index(), 0);
        let out = c.key(K::Ok);
        assert!(out.close);
        assert_eq!(out.events, vec![("confirm.answered".to_string(), json!({"id": "keep"}))]);
    }

    #[test]
    fn right_chooses_the_danger_answer_and_the_end_holds() {
        let mut c = Confirm::new();
        c.ask(uninstall());
        c.key(K::Right);
        c.key(K::Right);
        assert_eq!(c.index(), 1);
        let out = c.key(K::Ok);
        assert_eq!(out.events, vec![("confirm.answered".to_string(), json!({"id": "uninstall"}))]);
        c.ask(uninstall());
        c.key(K::Left);
        assert_eq!(c.index(), 0);
    }

    #[test]
    fn back_is_the_answer_that_was_focused_first_whatever_is_focused_now() {
        let mut c = Confirm::new();
        c.ask(uninstall());
        c.key(K::Right);
        let out = c.key(K::Back);
        assert_eq!(out.events, vec![("confirm.answered".to_string(), json!({"id": "keep"}))]);
        assert!(out.close);
    }

    #[test]
    fn a_click_gives_the_answer_under_the_pointer() {
        let mut c = Confirm::new();
        c.ask(uninstall());
        let out = c.click(1);
        assert_eq!(out.events, vec![("confirm.answered".to_string(), json!({"id": "uninstall"}))]);
    }

    #[test]
    fn a_spec_reads_what_the_qml_dialog_read_and_the_focus_is_the_safe_one_it_names() {
        let s = uninstall();
        assert_eq!((s.title.as_str(), s.icon_url.as_str(), s.answers.len(), s.focus), ("Uninstall Crunchyroll?", "/media/cr.png", 2, 0));
        assert!(s.answers[1].danger && !s.answers[0].danger);
        let mut c = Confirm::new();
        c.ask(Spec::from_json(&json!({"title": "Quit?", "answers": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}], "focus": 1})));
        assert_eq!(c.index(), 1);
        assert!(c.view(false).is_some());
    }
}
