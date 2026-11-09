//! What a page's function is given: LG's settings, the owners of the helper's, the topics, the answers the page asked for, the theme,
//! and who is using the TV; and the builders of the rows that read and write LG's keys (`settings/Rows.js` and `Choice.js`): a row
//! names a key as `category.key`, shows LG's name and the value in force, is dimmed while the service has not answered or says "not
//! now" and left out when the TV has no such key, as LG's own app leaves it out.

use super::names::{self, Table};
use super::row::{Arg, Card, ChoiceSpec, Kind, Op, PageArg, Pick, Row};
use super::src::{Bag, Owner, Topics};
use crate::host::sio::{Sio as Lg, js};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::rc::Rc;

/// Who is using the TV: the profiles topic's `canAdmin` and the owner's name. A TV with no profiles topic is the owner's.
#[derive(Clone, Debug, PartialEq)]
pub struct Who {
    pub can_admin: bool,
    pub owner: String,
    pub owner_id: String,
    pub active: String,
}

pub struct Ctx<'a> {
    pub lg: &'a Lg,
    pub power: &'a crate::host::power::Power,
    pub apps: &'a crate::host::apps::Apps,
    pub owners: &'a BTreeMap<String, Owner>,
    pub topics: &'a Topics,
    pub docs: &'a Bag,
    pub local: &'a Bag,
    pub theme: &'a Value,
    pub layout: &'a Value,
    pub arg: &'a PageArg,
    pub page_id: &'a str,
    pub page_title: &'a str,
    pub now: u64,
    /// The look in force is the light one (what "Auto" resolved to).
    pub light: bool,
}

impl Ctx<'_> {
    pub fn helper(&self, topic: &str) -> Option<&Value> {
        self.topics.helper(topic)
    }

    pub fn owner(&self, id: &str) -> Option<&Owner> {
        self.owners.get(id)
    }

    pub fn who(&self) -> Who {
        let p = self.helper("profiles");
        let owner_id = p.and_then(|p| p.get("owner")).and_then(Value::as_str).unwrap_or("");
        let owner = p
            .and_then(|p| p.get("profiles"))
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|e| e.get("owner").and_then(Value::as_bool) == Some(true) || e.get("id").and_then(Value::as_str) == Some(owner_id)))
            .and_then(|e| e.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("the owner");
        Who {
            can_admin: p.and_then(|p| p.get("canAdmin")).and_then(Value::as_bool).unwrap_or(true),
            owner: owner.to_string(),
            owner_id: owner_id.to_string(),
            active: p.and_then(|p| p.get("active")).and_then(Value::as_str).unwrap_or("").to_string(),
        }
    }

    /// A value of the merged theme by its dotted path.
    pub fn theme_at(&self, path: &str) -> Option<&Value> {
        let mut v = self.theme;
        for key in path.split('.') {
            v = v.get(key)?;
        }
        if v.is_null() { None } else { Some(v) }
    }

    pub fn theme_str(&self, path: &str) -> Option<&str> {
        self.theme_at(path).and_then(Value::as_str)
    }

    /// A boolean of the theme, `default` where it is not written.
    pub fn theme_flag(&self, path: &str, default: bool) -> bool {
        self.theme_at(path).and_then(Value::as_bool).unwrap_or(default)
    }

    pub fn theme_num(&self, path: &str, default: f64) -> f64 {
        self.theme_at(path).and_then(Value::as_f64).unwrap_or(default)
    }

    /// The app in front, when it is an input of the TV (`Inputs.current`): the per-input rows (aspect ratio, TruMotion) are offered
    /// while one is.
    pub fn input_app(&self) -> Option<String> {
        let app = self.topics.channel("foreground")?.get("app")?.as_str()?;
        (app.starts_with("com.webos.app.hdmi") || app.starts_with("com.webos.app.externalinput") || app == "com.webos.app.livetv").then(|| app.to_string())
    }

    /// The input's name as the chip shows it: HDMI 2, Live TV.
    pub fn input_port(&self) -> String {
        match self.input_app() {
            Some(a) if a.ends_with("livetv") => "Live TV".into(),
            Some(a) => match a.strip_prefix("com.webos.app.hdmi") {
                Some(n) => format!("HDMI {n}"),
                None => a,
            },
            None => String::new(),
        }
    }

    /// `unavailable(sio, ds, detail)`: the second line of a row that reads a key.
    pub fn unavailable(&self, ds: &str, detail: &str) -> String {
        if !self.lg.has(ds) {
            "TV not answering".into()
        } else if self.lg.usable(ds) {
            detail.to_string()
        } else {
            "Not available now".into()
        }
    }

    /// The values a choice row offers: the TV's own list for the key in the TV's order, the ones LG hides left out, each in LG's
    /// name. `order` narrows or orders it; the TV's list still decides what is on it. A key whose description has no list offers
    /// `order` as given.
    pub fn picks(&self, ds: &str, table: Table, order: &[String]) -> Vec<Pick> {
        if let Some(list) = self.lg.list(ds) {
            let visible: Vec<_> = list.iter().filter(|e| e.visible).collect();
            let order: Vec<String> = if order.is_empty() { visible.iter().map(|e| e.value.clone()).collect() } else { order.to_vec() };
            return order
                .iter()
                .filter_map(|v| visible.iter().find(|e| &e.value == v).map(|e| Pick { enabled: e.active, ..Pick::new(v, &names::name(table, v)) }))
                .collect();
        }
        order.iter().map(|v| Pick::new(v, &names::name(table, v))).collect()
    }
}

/// Everything an LG-bound row is made from (the `o` of `R.toggle(sio, o)` and `C.choice(sio, o)`).
#[derive(Clone)]
pub struct Spec {
    pub ds: String,
    pub id: Option<String>,
    pub tab: usize,
    pub icon: String,
    pub label: String,
    pub help: String,
    pub detail: String,
    pub app: Option<String>,
    pub unit: String,
    pub on: String,
    pub off: String,
    pub names: Table,
    pub values: Vec<String>,
    pub options: Vec<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub format: Option<fn(i64) -> String>,
    /// LG's own rule (`C.lgShown`): a row whose description says it is not visible is left out, not dimmed.
    pub shown: bool,
    /// Whether the household's owner alone may change it (every key of LG's settings is).
    pub admin: bool,
    pub strip: bool,
    pub cols: usize,
    pub cards: Vec<Card>,
    pub write: Option<Rc<super::row::Writer>>,
    /// The value a choice shows and marks as current when the key's own is known to lag behind what is in force (the sound out).
    pub in_force: Option<String>,
}

impl Spec {
    pub fn new(ds: &str) -> Self {
        Spec {
            ds: ds.into(),
            id: None,
            tab: 0,
            icon: String::new(),
            label: String::new(),
            help: String::new(),
            detail: String::new(),
            app: None,
            unit: String::new(),
            on: "on".into(),
            off: "off".into(),
            names: names::NONE,
            values: Vec::new(),
            options: Vec::new(),
            min: None,
            max: None,
            step: None,
            format: None,
            shown: false,
            admin: true,
            strip: false,
            cols: 4,
            cards: Vec::new(),
            write: None,
            in_force: None,
        }
    }

    pub fn in_force(mut self, value: &str) -> Self {
        self.in_force = Some(value.into());
        self
    }

    pub fn id(mut self, id: &str) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn tab(mut self, tab: usize) -> Self {
        self.tab = tab;
        self
    }

    pub fn icon(mut self, icon: &str) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn label(mut self, label: &str) -> Self {
        self.label = label.into();
        self
    }

    pub fn help(mut self, help: &str) -> Self {
        self.help = help.into();
        self
    }

    pub fn detail(mut self, detail: &str) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn app(mut self, app: Option<String>) -> Self {
        self.app = app;
        self
    }

    pub fn unit(mut self, unit: &str) -> Self {
        self.unit = unit.into();
        self
    }

    /// The values a switch writes (`on: "true", off: "false"`).
    pub fn on_off(mut self, on: &str, off: &str) -> Self {
        self.on = on.into();
        self.off = off.into();
        self
    }

    pub fn names(mut self, names: Table) -> Self {
        self.names = names;
        self
    }

    pub fn values(mut self, values: &[&str]) -> Self {
        self.values = values.iter().map(|v| v.to_string()).collect();
        self
    }

    pub fn options(mut self, options: &[&str]) -> Self {
        self.options = options.iter().map(|v| v.to_string()).collect();
        self
    }

    pub fn range(mut self, min: f64, max: f64, step: f64) -> Self {
        (self.min, self.max, self.step) = (Some(min), Some(max), Some(step));
        self
    }

    pub fn format(mut self, f: fn(i64) -> String) -> Self {
        self.format = Some(f);
        self
    }

    pub fn lg_shown(mut self) -> Self {
        self.shown = true;
        self
    }

    pub fn mine(mut self) -> Self {
        self.admin = false;
        self
    }

    pub fn strip(mut self, in_view: usize) -> Self {
        self.strip = true;
        self.cols = in_view;
        self
    }

    pub fn cols(mut self, cols: usize) -> Self {
        self.cols = cols;
        self
    }

    pub fn cards(mut self, cards: &[(&str, &str)]) -> Self {
        self.cards = cards.iter().map(|(t, i)| Card { title: (*t).into(), icon: (*i).into(), sub: String::new(), face: None }).collect();
        self
    }

    pub fn write(mut self, f: impl Fn(&str) -> Vec<Op> + 'static) -> Self {
        self.write = Some(Rc::new(f));
        self
    }

    fn row_id(&self) -> String {
        self.id.clone().unwrap_or_else(|| self.ds.clone())
    }
}

impl Ctx<'_> {
    fn base(&self, mut row: Row, o: &Spec) -> Row {
        row.tab = o.tab;
        row.icon = o.icon.clone();
        row.label = o.label.clone();
        row.help = o.help.clone();
        row.admin = o.admin;
        row.detail = self.unavailable(&o.ds, &o.detail);
        row.enabled = self.lg.usable(&o.ds);
        row.visible = if o.shown { self.lg.shown(&o.ds) } else { !self.lg.missing(&o.ds) };
        row
    }

    fn write_op(o: &Spec, value: Value) -> Op {
        Op::Lg { ds: o.ds.clone(), value, app: o.app.clone() }
    }

    /// `R.toggle`: a switch; on is the value `o.on` ("on").
    pub fn toggle(&self, o: Spec) -> Row {
        let mut r = self.base(Row::toggle(&o.row_id()), &o);
        r.on = self.lg.is(&o.ds, &o.on);
        let (on, off) = (o.on.clone(), o.off.clone());
        r.act(move |a| match a {
            Arg::Toggle(v) => vec![Self::write_op(&o, json!(if v { &on } else { &off }))],
            _ => Vec::new(),
        })
    }

    /// `R.seg`: a few options in one row, written as the value at the same place of `values`.
    pub fn seg(&self, o: Spec) -> Row {
        let mut r = self.base(Row::new(Kind::Seg, &o.row_id()), &o);
        r.options = o.options.clone();
        r.index = self.lg.opt_text(&o.ds).and_then(|t| o.values.iter().position(|v| *v == t));
        r.act(move |a| match a {
            Arg::Index(i) => o.values.get(i).map(|v| vec![Self::write_op(&o, json!(v))]).unwrap_or_default(),
            _ => Vec::new(),
        })
    }

    /// `R.slider`: a number with its limits; an unanswered key shows an en dash, not a zero.
    pub fn slider(&self, o: Spec) -> Row {
        let (min, max, step) = (o.min.unwrap_or(0.0), o.max.unwrap_or(100.0), o.step.unwrap_or(1.0));
        let mut r = self.base(Row::slider(&o.row_id(), min, max, step), &o);
        r.num = self.lg.num(&o.ds, min);
        r.unit = o.unit.clone();
        r.value_text = Some(if self.lg.has(&o.ds) { format!("{}{}", r.num.round() as i64, o.unit) } else { "\u{2013}".into() });
        r.act(move |a| match a {
            Arg::Value(v) => vec![Self::write_op(&o, json!(v))],
            _ => Vec::new(),
        })
    }

    /// `R.cards`: one card is the current setting; `strip` is one row of them with `cols` in view.
    pub fn cards(&self, o: Spec) -> Row {
        let mut r = self.base(Row::cards(&o.row_id(), o.cards.clone()), &o);
        r.cols = if o.strip { 1000 } else { o.cols };
        r.strip = o.strip;
        r.in_view = o.cols;
        r.index = self.lg.opt_text(&o.ds).and_then(|t| o.values.iter().position(|v| *v == t));
        r.act(move |a| match a {
            Arg::Index(i) => o.values.get(i).map(|v| vec![Self::write_op(&o, json!(v))]).unwrap_or_default(),
            _ => Vec::new(),
        })
    }

    /// `C.seg`: a short list on one row built from the TV's own list for the key, each value named as LG names it.
    pub fn seg_tv(&self, o: Spec) -> Row {
        let picks = self.picks(&o.ds, o.names, &o.values);
        let mut o = o;
        o.shown = true;
        let mut r = self.base(Row::new(Kind::Seg, &o.row_id()), &o);
        r.options = picks.iter().map(|p| p.label.clone()).collect();
        let cur = self.lg.opt_text(&o.ds);
        r.index = cur.and_then(|t| picks.iter().position(|p| p.value == t));
        r.act(move |a| match a {
            Arg::Index(i) => picks.get(i).filter(|p| p.enabled).map(|p| vec![Self::write_op(&o, json!(p.value))]).unwrap_or_default(),
            _ => Vec::new(),
        })
    }

    /// `C.range`: a slider whose limits are the TV's own (the key's description gives its range).
    pub fn range(&self, o: Spec) -> Row {
        let lim = self.lg.range(&o.ds);
        let min = lim.map_or(o.min.unwrap_or(0.0), |l| l.min);
        let max = lim.map_or(o.max.unwrap_or(100.0), |l| l.max);
        let step = lim.and_then(|l| l.interval).unwrap_or(o.step.unwrap_or(1.0));
        let mut o = o;
        o.shown = true;
        let mut r = self.base(Row::slider(&o.row_id(), min, max, step), &o);
        r.num = self.lg.num(&o.ds, min);
        r.unit = o.unit.clone();
        r.value_text = Some(if self.lg.has(&o.ds) {
            let n = r.num.round() as i64;
            format!("{}{}", o.format.map_or_else(|| n.to_string(), |f| f(n)), o.unit)
        } else {
            "\u{2013}".into()
        });
        r.act(move |a| match a {
            Arg::Value(v) => vec![Self::write_op(&o, json!(v))],
            _ => Vec::new(),
        })
    }

    /// `C.choice`: a row that shows the value in force and opens the chooser on the TV's list.
    pub fn choice(&self, o: Spec) -> Row {
        let picks = self.picks(&o.ds, o.names, &o.values);
        let mut o = o;
        o.shown = true;
        let mut r = self.base(Row::link(&o.row_id()), &o);
        let current = o.in_force.clone().unwrap_or_else(|| self.lg.opt_text(&o.ds).unwrap_or_default());
        r.value = if self.lg.has(&o.ds) { names::name(o.names, &current) } else { String::new() };
        r.act(move |_| {
            let (ds, app) = (o.ds.clone(), o.app.clone());
            let custom = o.write.clone();
            vec![Op::Pick(Rc::new(ChoiceSpec {
                title: o.label.clone(),
                options: picks.clone(),
                current: current.clone(),
                write: Box::new(move |v| match &custom {
                    Some(f) => f(v),
                    None => vec![Op::Lg { ds: ds.clone(), value: json!(v), app: app.clone() }],
                }),
            }))]
        })
    }

    /// `C.display`: the five switches of LG's Accessibility that exclude each other (Text with High Contrast, Focus with High Contrast,
    /// Screen Zoom, Grayscale, Invert Colors). A switch writes the keys LG's own row writes with it.
    pub fn display(&self, key: &str, o: Spec) -> Row {
        let pair = key == "blackAndWhite" || key == "colorInversion";
        let ds = if pair { "option.adjustments".to_string() } else { format!("option.{key}") };
        let mut o = o;
        o.ds = ds.clone();
        o.shown = true;
        let mut r = self.base(Row::toggle(&o.id.clone().unwrap_or_else(|| if pair { format!("{ds}.{key}") } else { ds.clone() })), &o);
        r.on = match self.lg.get(&ds) {
            Some(v) if pair => v.get(key).and_then(Value::as_str) == Some("on"),
            Some(v) => js::string(v) == "on",
            None => false,
        };
        let key = key.to_string();
        r.act(move |a| {
            let Arg::Toggle(v) = a else { return Vec::new() };
            let s = json!(if v { "on" } else { "off" });
            if !pair {
                let mut m = Map::new();
                m.insert("adjustments".into(), json!({"blackAndWhite": "off", "colorInversion": "off"}));
                m.insert(key.clone(), s);
                return vec![Op::LgMany { category: "option".into(), settings: m, app: None }];
            }
            let mut zoom = Map::new();
            zoom.insert("screenMagnification".into(), json!("off"));
            let mut adj = json!({"blackAndWhite": "off", "colorInversion": "off"});
            adj[&key] = s;
            let mut m = Map::new();
            m.insert("adjustments".into(), adj);
            m.insert("highContrast".into(), json!("off"));
            m.insert("focusEmphasis".into(), json!("off"));
            vec![Op::LgMany { category: "option".into(), settings: zoom, app: None }, Op::LgMany { category: "option".into(), settings: m, app: None }]
        })
    }
}

/// The text of a row that opens LG's Settings app at a screen of its own (`R.lg`): the row is LG's name for the screen.
pub fn lg_launch(id: &str, label: &str, target: &str, app: &str) -> Row {
    let params = if target.is_empty() { json!({}) } else { json!({"target": target}) };
    let app = if app.is_empty() { "com.palm.app.settings".to_string() } else { app.to_string() };
    Row::link(id).icon("square-arrow-out-up-right").label(label).does(vec![Op::Launch { app, params }])
}

/// A row with a button that does something once (`C.action`); anything that cannot be undone goes through [`super::row::Ask`].
pub fn action(id: &str, label: &str, button: &str, ops: Vec<Op>) -> Row {
    Row::action(id, button).label(label).does(ops)
}

/// The question a row asks before it does `then` (`C.ask`): the safe answer first, which is also what Back means.
pub fn ask(icon: &str, title: &str, body: &str, yes: &str, no: &str, danger: bool, then: Vec<Op>) -> Vec<Op> {
    vec![Op::Ask(super::row::Ask::yes_no(icon, title, body, yes, no, danger, then))]
}

/// A toast that says what a switch just did (`PluginHub.toast`: LG's toast service, drawn by LumaTV). A switch never asks first; it flips
/// and this says so.
pub fn toast(text: &str) -> Op {
    Op::Act { uri: "luna://com.webos.notification/createToast".into(), payload: json!({"sourceId": "com.webos.app.home", "message": text, "noaction": true, "persistent": false}) }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    /// LG's service as a test sets it up: `settings` are (category, reply to a read of values), `descs` (category, reply to a read of
    /// descriptions).
    pub fn lg_with(settings: &[(&str, Value)], descs: &[(&str, Value)]) -> Lg {
        let mut lg = Lg::new(0);
        for (cat, doc) in settings {
            lg.learn(cat, doc);
        }
        for (cat, doc) in descs {
            lg.learn_desc(cat, doc);
        }
        lg
    }

    pub struct World {
        pub lg: Lg,
        pub owners: BTreeMap<String, Owner>,
        pub topics: Topics,
        pub docs: Bag,
        pub local: Bag,
        pub theme: Value,
        pub layout: Value,
        pub arg: PageArg,
        pub power: crate::host::power::Power,
        pub apps: crate::host::apps::Apps,
        pub now: u64,
    }

    impl World {
        pub fn new(lg: Lg) -> Self {
            World { lg, owners: BTreeMap::new(), topics: Topics::default(), docs: Bag::default(), local: Bag::default(), theme: json!({}), layout: json!({}), arg: PageArg::None, power: crate::host::power::Power::new(), apps: crate::host::apps::Apps::new(0), now: 0 }
        }

        pub fn ctx(&self) -> Ctx<'_> {
            Ctx {
                lg: &self.lg,
                power: &self.power,
                apps: &self.apps,
                owners: &self.owners,
                topics: &self.topics,
                docs: &self.docs,
                local: &self.local,
                theme: &self.theme,
                layout: &self.layout,
                arg: &self.arg,
                page_id: "picture",
                page_title: "Picture",
                now: self.now,
                light: false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn picture() -> Lg {
        lg_with(
            &[("picture", json!({"settings": {"eyeComfortMode": "on", "backlight": "60", "energySaving": "auto", "gamma": "medium"}, "errorKey": ["hgig"]}))],
            &[(
                "picture",
                json!({"results": [
                    {"category": "picture", "key": "gamma", "values": {"arrayExt": [{"value": "low"}, {"value": "medium"}, {"value": "high1", "visible": false}, {"value": "high2", "active": false}]}},
                    {"category": "picture", "key": "backlight", "ui": {"active": false}, "values": {"range": {"min": 0, "max": 80, "interval": 5}}},
                    {"category": "picture", "key": "nearBlackDetail", "ui": {"visible": false}}]}),
            )],
        )
    }

    #[test]
    fn a_switch_reads_the_value_in_force_and_writes_on_or_off() {
        let w = World::new(picture());
        let c = w.ctx();
        let r = c.toggle(Spec::new("picture.eyeComfortMode").tab(1).icon("eye").label("Eye comfort"));
        assert!(r.on && r.enabled && r.visible && r.detail.is_empty());
        assert_eq!(r.tab, 1);
        let ops = (r.act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&ops[..], [Op::Lg { ds, value, app: None }] if ds == "picture.eyeComfortMode" && value == "off"));
        // a key the service did not answer is dimmed with its reason, one it said it lacks is left out
        let r = c.toggle(Spec::new("picture.realCinema"));
        assert!(!r.enabled && r.detail == "TV not answering" && r.visible);
        let r = c.toggle(Spec::new("picture.hgig"));
        assert!(!r.visible, "the TV has no such key");
        // a value that is not on reads as off
        let r = c.toggle(Spec::new("picture.energySaving"));
        assert!(!r.on);
    }

    #[test]
    fn a_description_that_says_not_now_dims_the_row_and_one_that_says_not_visible_leaves_it_out() {
        let w = World::new(picture());
        let c = w.ctx();
        let r = c.slider(Spec::new("picture.backlight").range(0.0, 100.0, 1.0));
        assert!(!r.enabled && r.detail == "Not available now");
        assert_eq!(r.value_text.as_deref(), Some("60"));
        // the TV's own limits are the range row's, not the page's guess
        let r = c.range(Spec::new("picture.backlight").range(0.0, 100.0, 1.0));
        assert_eq!((r.min, r.max, r.step), (0.0, 80.0, 5.0));
        let r = c.range(Spec::new("picture.nearBlackDetail"));
        assert!(!r.visible, "the description says it is not visible");
        let r = c.slider(Spec::new("picture.nearBlackDetail"));
        assert!(r.visible, "a row built without LG's rule is only dimmed: the plain builders leave out only a missing key");
        assert_eq!(c.slider(Spec::new("picture.contrast")).value_text.as_deref(), Some("\u{2013}"));
    }

    #[test]
    fn a_segmented_row_from_the_tvs_list_keeps_its_order_and_a_value_it_does_not_offer_is_never_written() {
        let w = World::new(picture());
        let c = w.ctx();
        let r = c.seg_tv(Spec::new("picture.gamma").label("Gamma").names(names::GAMMA));
        assert_eq!(r.options, vec!["1.9", "2.2", "BT.1886"], "the hidden one is left out, the inactive one stays");
        assert_eq!(r.index, Some(1));
        let act = r.act.as_ref().unwrap();
        assert!(matches!(&act(Arg::Index(0))[..], [Op::Lg { value, .. }] if value == "low"));
        assert!(act(Arg::Index(2)).is_empty(), "inactive: nothing is written");
        // `values` narrows and orders the TV's list
        let r = c.seg_tv(Spec::new("picture.gamma").names(names::GAMMA).values(&["medium", "low", "nothere"]));
        assert_eq!(r.options, vec!["2.2", "1.9"]);
    }

    #[test]
    fn a_choice_row_shows_the_name_of_the_value_and_opens_the_picker_with_the_way_to_write_a_pick() {
        let w = World::new(picture());
        let c = w.ctx();
        let r = c.choice(Spec::new("picture.gamma").label("Gamma").names(names::GAMMA));
        assert_eq!(r.value, "2.2");
        assert!(r.chevron);
        let ops = (r.act.as_ref().unwrap())(Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!("opens the picker: {ops:?}") };
        assert_eq!((spec.title.as_str(), spec.current.as_str()), ("Gamma", "medium"));
        assert_eq!(spec.options.iter().map(|p| (p.value.as_str(), p.enabled)).collect::<Vec<_>>(), vec![("low", true), ("medium", true), ("high2", false)]);
        let written = (spec.write)("low");
        assert!(matches!(&written[..], [Op::Lg { ds, value, .. }] if ds == "picture.gamma" && value == "low"));
    }

    #[test]
    fn the_accessibility_switches_write_the_keys_lgs_own_rows_write_with_them() {
        let lg = lg_with(&[("option", json!({"settings": {"highContrast": "off", "adjustments": {"blackAndWhite": "on", "colorInversion": "off"}}}))], &[]);
        let w = World::new(lg);
        let c = w.ctx();
        let hc = c.display("highContrast", Spec::new("").label("Text with High Contrast"));
        assert!(!hc.on);
        let ops = (hc.act.as_ref().unwrap())(Arg::Toggle(true));
        let [Op::LgMany { category, settings, .. }] = &ops[..] else { panic!() };
        assert_eq!(category, "option");
        assert_eq!(settings["highContrast"], "on");
        assert_eq!(settings["adjustments"], json!({"blackAndWhite": "off", "colorInversion": "off"}), "the pair goes off with it");
        let gray = c.display("blackAndWhite", Spec::new("").label("Grayscale"));
        assert!(gray.on);
        let ops = (gray.act.as_ref().unwrap())(Arg::Toggle(false));
        assert_eq!(ops.len(), 2, "the zoom is switched off first, then the pair and the two contrast switches");
        let Op::LgMany { settings, .. } = &ops[0] else { panic!() };
        assert_eq!(settings["screenMagnification"], "off");
    }

    #[test]
    fn who_is_using_the_tv_comes_from_the_profiles_topic_and_an_absent_topic_is_the_owners() {
        let mut w = World::new(Lg::new(0));
        assert!(w.ctx().who().can_admin);
        w.topics.set_helper("profiles", json!({"active": "isabel", "owner": "nitin", "canAdmin": false, "profiles": [{"id": "nitin", "name": "Nitin", "owner": true}, {"id": "isabel", "name": "Isabel"}]}));
        let who = w.ctx().who();
        assert_eq!((who.can_admin, who.owner.as_str(), who.owner_id.as_str(), who.active.as_str()), (false, "Nitin", "nitin", "isabel"));
    }

    #[test]
    fn an_input_in_front_names_the_port_and_a_plain_app_is_no_input() {
        let mut w = World::new(Lg::new(0));
        w.topics.set_channel("foreground", json!({"app": "com.webos.app.hdmi2", "home": false}));
        assert_eq!((w.ctx().input_app().as_deref(), w.ctx().input_port().as_str()), (Some("com.webos.app.hdmi2"), "HDMI 2"));
        w.topics.set_channel("foreground", json!({"app": "netflix", "home": false}));
        assert_eq!(w.ctx().input_app(), None);
        w.topics.set_channel("foreground", json!({"app": "com.webos.app.livetv"}));
        assert_eq!(w.ctx().input_port(), "Live TV");
    }
}
