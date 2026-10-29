//! Rows over one owner of the helper's settings layer (`settings/HelperSettings.qml` with the row builders of `Rows.js`): a row names a
//! field by its key, shows the effective value the helper gave and writes one typed change. Until the helper answers with the owner the
//! row is dimmed as an LG key is; a field the schema lacks (an older helper) is dimmed, not left out. These are the household's settings,
//! so a profile that may not change them sees the rows locked.

use crate::settings::ctx::Ctx;
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

/// Which field of which owner a row is, and the words of its row.
#[derive(Clone, Copy)]
pub struct Field<'a> {
    pub owner: &'a str,
    pub key: &'a str,
    pub tab: usize,
    pub icon: &'a str,
    pub label: &'a str,
    pub help: &'a str,
}

impl<'a> Field<'a> {
    pub fn new(owner: &'a str, key: &'a str, tab: usize, icon: &'a str, label: &'a str) -> Self {
        Field { owner, key, tab, icon, label, help: "" }
    }

    pub fn help(mut self, help: &'a str) -> Self {
        self.help = help;
        self
    }

    fn has(&self, c: &Ctx) -> bool {
        c.owner(self.owner).is_some_and(|o| o.has(self.key))
    }

    fn base(&self, c: &Ctx, row: Row) -> Row {
        let has = self.has(c);
        let mut r = row.tab(self.tab).icon(self.icon).label(self.label).help(self.help).enabled(has).admin();
        if !has {
            r.detail = "TV not answering".into();
        }
        r
    }

    /// What a change of the field does, owning its names so a row's action outlives the page's borrow.
    fn writer(&self) -> impl Fn(Value) -> Vec<Op> + 'static {
        let (owner, key) = (self.owner.to_string(), self.key.to_string());
        move |value| vec![Op::Owner { owner: owner.clone(), key: key.clone(), value }]
    }
}

/// `R.toggle(hs, {on: "true", off: "false"})`: a switch on a boolean field.
pub fn toggle(c: &Ctx, f: Field) -> Row {
    let on = c.owner(f.owner).and_then(|o| o.get(f.key)).is_some_and(|v| v == &json!(true) || v == &json!("true"));
    let write = f.writer();
    f.base(c, Row::toggle(f.key)).on(on).act(move |a| match a {
        Arg::Toggle(v) => write(json!(v)),
        _ => Vec::new(),
    })
}

/// `R.slider`: a number with its limits; a field the helper has not answered shows an en dash, not a zero.
pub fn slider(c: &Ctx, f: Field, min: f64, max: f64, step: f64, unit: &str) -> Row {
    let n = c.owner(f.owner).map_or(min, |o| o.num(f.key, min));
    let mut r = f.base(c, Row::slider(f.key, min, max, step)).num(n).unit(unit);
    r.value_text = Some(if f.has(c) { format!("{}{unit}", n.round() as i64) } else { "\u{2013}".into() });
    let write = f.writer();
    r.act(move |a| match a {
        Arg::Value(v) => write(json!(v)),
        _ => Vec::new(),
    })
}

/// `R.seg`: a few options in one row, written as the value at the same place of `values`.
pub fn seg(c: &Ctx, f: Field, options: &[&str], values: &[&str]) -> Row {
    let now = c.owner(f.owner).and_then(|o| o.text(f.key));
    let index = now.and_then(|t| values.iter().position(|v| *v == t));
    let values: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    let write = f.writer();
    f.base(c, Row::seg(f.key, options)).index(index).act(move |a| match a {
        Arg::Index(i) => values.get(i).map(|v| write(json!(v))).unwrap_or_default(),
        _ => Vec::new(),
    })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::src::Owner;

    /// A world whose helper has answered with `owner`'s document.
    pub fn world_with(owner: &str, doc: &Value) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        let mut o = Owner::new(owner);
        o.take(doc);
        w.owners.insert(owner.to_string(), o);
        w
    }

    /// An owner's document from (key, kind, value).
    pub fn doc(fields: &[(&str, &str, Value)]) -> Value {
        let eff: serde_json::Map<String, Value> = fields.iter().map(|(k, _, v)| (k.to_string(), json!({"value": v}))).collect();
        let fs: Vec<Value> = fields.iter().map(|(k, t, _)| json!({"key": k, "kind": {"type": t}})).collect();
        json!({"ok": true, "version": 1, "effective": eff, "schema": {"sections": [{"id": "s", "fields": fs}]}})
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::settings::ctx::fixtures::World;

    #[test]
    fn a_row_shows_the_effective_value_writes_one_typed_change_and_is_dimmed_until_the_helper_answers() {
        let w = world_with("o", &doc(&[("on", "bool", json!(true)), ("level", "integer", json!(40)), ("mode", "choice", json!("colour"))]));
        let c = w.ctx();
        let t = toggle(&c, Field::new("o", "on", 0, "lamp", "On"));
        assert!(t.on && t.enabled && t.admin && t.detail.is_empty());
        assert!(matches!(&(t.act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Owner { owner, key, value }] if owner == "o" && key == "on" && value == &json!(false)));
        let s = slider(&c, Field::new("o", "level", 0, "sun", "Level"), 0.0, 100.0, 5.0, "%");
        assert_eq!((s.num, s.value_text.as_deref()), (40.0, Some("40%")));
        let m = seg(&c, Field::new("o", "mode", 0, "x", "Mode"), &["Picture", "Colour"], &["picture", "colour"]);
        assert_eq!(m.index, Some(1));
        assert!(matches!(&(m.act.as_ref().unwrap())(Arg::Index(0))[..], [Op::Owner { value, .. }] if value == &json!("picture")));
        let w = World::new(crate::host::sio::Sio::new(0));
        let c = w.ctx();
        let t = toggle(&c, Field::new("o", "on", 0, "lamp", "On"));
        assert!(!t.enabled && t.detail == "TV not answering");
        assert_eq!(slider(&c, Field::new("o", "level", 0, "sun", "Level"), 0.0, 100.0, 5.0, "%").value_text.as_deref(), Some("\u{2013}"));
    }
}
