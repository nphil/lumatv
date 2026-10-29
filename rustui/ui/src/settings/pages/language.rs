//! Language & location (`settings/PageLanguage.qml`; LG: General › System › Language and Location). LG keeps every language in one
//! setting, `localeInfo`, read and written with no category: `locales` holds the menu language (UI, with FMT and TV) and the audio
//! language (AUD), `keyboards` the on-screen keyboard languages. The lists to choose from come from the settings service
//! (`getSystemSettingValues {key: "locale"}`: `locale`, `audio` and `vkb`, each a list of languages with their countries). LG's menu
//! language row writes UI, FMT and TV together and sets the keyboards to English and the language's own; its keyboards panel adds and
//! removes a keyboard, English and the one of the menu language staying. Location: the broadcast country only shows (changing it resets
//! the TV and its channels); the LG services country is a switch (`option.autoSmartServiceCountry`) that asks the TV's sdx service to find
//! the country by itself; the zip code is `option.zipcode`. Changing the menu language reloads LG's menus, so it asks first.

use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, ChoiceSpec, Op, Pick, Row, settings};
use serde_json::{Value, json};
use std::rc::Rc;

pub fn page() -> Page {
    Page::new("language", "Language & location", "languages").parent("system").back("System").hidden_from_search().tabs(&["Language", "Keyboards", "Location"]).reads(reads).rows(rows).answered(answered)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![
        Read::lg("", &["localeInfo"]),
        Read::lg("option", &["country", "autoSmartServiceCountry", "zipcode"]),
        Read::call("locale", "luna://com.webos.service.settings/getSystemSettingValues", json!({"key": "locale"})),
    ]
}

const COUNTRIES: [(&str, &str); 14] = [
    ("USA", "United States"),
    ("CAN", "Canada"),
    ("MEX", "Mexico"),
    ("GBR", "United Kingdom"),
    ("AUS", "Australia"),
    ("NZL", "New Zealand"),
    ("DEU", "Germany"),
    ("FRA", "France"),
    ("ESP", "Spain"),
    ("ITA", "Italy"),
    ("JPN", "Japan"),
    ("KOR", "South Korea"),
    ("BRA", "Brazil"),
    ("IND", "India"),
];

fn str_of(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// `{value, label}` from a list of languages with their countries (LG's `ZM`): "English - United States", or the language alone.
fn flatten(list: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for l in list.as_array().into_iter().flatten() {
        let name = str_of(l, "name");
        match l.get("countries").and_then(Value::as_array).filter(|c| !c.is_empty()) {
            Some(countries) => {
                for c in countries {
                    let cn = str_of(c, "name");
                    out.push((str_of(c, "spec"), if cn.is_empty() { name.clone() } else { format!("{name} - {cn}") }));
                }
            }
            None => {
                let code = if l.get("languageCode").is_some_and(|v| !v.is_null()) { str_of(l, "languageCode") } else { str_of(l, "spec") };
                out.push((code, name));
            }
        }
    }
    out
}

fn info(c: &Ctx) -> Value {
    c.lg.get("localeInfo").cloned().unwrap_or_else(|| json!({}))
}

fn locale(c: &Ctx, key: &str) -> String {
    str_of(info(c).get("locales").unwrap_or(&Value::Null), key)
}

/// An on-screen keyboard that stays: English and the menu language's own (LG marks them Deactivated).
fn locked(c: &Ctx, spec: &str) -> bool {
    let ui = locale(c, "UI");
    spec == "en" || (!ui.is_empty() && ui.contains(spec))
}

fn label_for(options: &[(String, String)], value: &str) -> String {
    options.iter().find(|(v, _)| v == value).map_or_else(|| value.to_string(), |(_, l)| l.clone())
}

/// A language row has the shape of a choice row: its value, its options, the one in force and a pick.
fn lang_row(c: &Ctx, id: &str, label: &str, options: Vec<(String, String)>, current: String, pick: impl Fn(&str) -> Vec<Op> + 'static) -> Row {
    let have = c.lg.has("localeInfo");
    let mut r = Row::link(id).tab(0).icon("languages").label(label).value(&label_for(&options, &current)).enabled(!options.is_empty() && have);
    r.detail = if !have { "TV not answering".into() } else if options.is_empty() { "No list from the TV".into() } else { String::new() };
    let spec_options: Vec<Pick> = options.iter().map(|(v, l)| Pick::new(v, l)).collect();
    let cur = current.clone();
    let pick = Rc::new(pick);
    r.does(vec![Op::Pick(Rc::new(ChoiceSpec {
        title: label.into(),
        options: spec_options,
        current,
        write: Box::new(move |v| if v == cur { Vec::new() } else { pick(v) }),
    }))])
}

fn rows(c: &Ctx) -> Vec<Row> {
    let mut out = Vec::new();
    let menu_options = flatten(c.docs.field("locale", "locale"));
    let audio_options = flatten(c.docs.field("locale", "audio"));
    let titles = menu_options.clone();
    let menu_info = info(c);
    let menu_op = {
        // everything `set_menu` needs from the page's state, taken now: a pick runs later
        let vkb = c.docs.field("locale", "vkb").clone();
        move |spec: &str| {
            let key = if ["as", "bn", "gu", "kn", "ml", "mr", "pa", "ta", "te", "ur"].contains(&spec) { "hi" } else { spec };
            let mut own = "en".to_string();
            for k in vkb.as_array().into_iter().flatten() {
                let s = if k.get("languageCode").is_some_and(|v| !v.is_null()) { str_of(k, "languageCode") } else { str_of(k, "spec") };
                if key.contains(&s) {
                    own = s;
                    break;
                }
            }
            let mut copy = menu_info.clone();
            if copy.get("locales").is_none() {
                copy["locales"] = json!({});
            }
            for k in ["UI", "FMT", "TV"] {
                copy["locales"][k] = json!(spec);
            }
            copy["keyboards"] = json!(if own == "en" { vec!["en".to_string()] } else { vec!["en".to_string(), own] });
            Op::LgMany { category: String::new(), settings: settings(&[("localeInfo", copy)]), app: None }
        }
    };
    let mut menu = lang_row(c, "menu", "Menu Language", menu_options, locale(c, "UI"), move |v| {
        ask("languages", &format!("Change to {}?", label_for(&titles, v)), "LG's menus reload in the new language.", "Change", "Cancel", false, vec![menu_op(v)])
    });
    menu.help = "The language of LG's menus and apps. LumaTV's own text stays English.".into();
    out.push(menu);
    let audio_info = info(c);
    out.push(lang_row(c, "audio", "Audio Language", audio_options, locale(c, "AUD"), move |v| {
        let mut copy = audio_info.clone();
        if copy.get("locales").is_none() {
            copy["locales"] = json!({});
        }
        copy["locales"]["AUD"] = json!(v);
        vec![Op::LgMany { category: String::new(), settings: settings(&[("localeInfo", copy)]), app: None }]
    }));
    // the keyboards
    let have = c.lg.has("localeInfo");
    let keyboards: Vec<String> = info(c).get("keyboards").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    for k in c.docs.field("locale", "vkb").as_array().into_iter().flatten() {
        let spec = if k.get("languageCode").is_some_and(|v| !v.is_null()) { str_of(k, "languageCode") } else { str_of(k, "spec") };
        let name = if k.get("name").is_some_and(|v| !v.is_null()) { str_of(k, "name") } else { str_of(k, "displayValue") };
        let lock = locked(c, &spec);
        let mut r = Row::toggle(&format!("kb:{spec}")).tab(1).icon("keyboard").label(&name).on(keyboards.contains(&spec));
        r.detail = if lock { "In use".into() } else { String::new() };
        r.enabled = !lock && have;
        let base = info(c);
        let (list, id) = (keyboards.clone(), spec.clone());
        out.push(r.act(move |a| match a {
            Arg::Toggle(on) => {
                let mut l = list.clone();
                let at = l.iter().position(|x| *x == id);
                match (on, at) {
                    (true, None) => l.push(id.clone()),
                    (false, Some(i)) => {
                        l.remove(i);
                    }
                    _ => {}
                }
                let mut copy = base.clone();
                copy["keyboards"] = json!(l);
                vec![Op::LgMany { category: String::new(), settings: settings(&[("localeInfo", copy)]), app: None }]
            }
            _ => Vec::new(),
        }));
    }
    // the location
    let country = c.lg.opt_text("option.country").unwrap_or_default();
    let mut bc = Row::info("country").tab(2).icon("globe").label("Broadcast Country").help("Changing it resets the TV and erases its channels. That is done in LG's settings.").value(&crate::settings::names::name_or(&COUNTRIES, &country, &country));
    bc.chevron = false;
    bc.visible = c.lg.has("option.country");
    out.push(bc);
    let mut svc = Row::toggle("svcCountry").tab(2).icon("globe-2").label("Set LG services country automatically").help("On: the TV finds the country of its LG services itself. Changing it restarts those services.").on(c.lg.is("option.autoSmartServiceCountry", "on"));
    svc.detail = c.unavailable("option.autoSmartServiceCountry", "");
    svc.enabled = c.lg.usable("option.autoSmartServiceCountry");
    svc.visible = c.lg.shown("option.autoSmartServiceCountry");
    out.push(svc.act(|a| match a {
        Arg::Toggle(v) => {
            let mut then = vec![Op::Lg { ds: "option.autoSmartServiceCountry".into(), value: json!(if v { "on" } else { "off" }), app: None }];
            if v {
                then.push(Op::Call { name: "country".into(), uri: "luna://com.webos.service.sdx/setCountrySettingByAuto".into(), payload: json!({"uniqueIndex": "2013"}), subscribe: false });
            }
            then.push(toast(if v { "LG services country is set automatically" } else { "LG services country is no longer set automatically" }));
            then
        }
        _ => Vec::new(),
    }));
    let bad = c.local.flag("zipbad");
    let mut zip = Row::link("zip").tab(2).icon("hash").label("Zip Code").help("Five digits, typed on the keyboard. LG's services use it for the weather and the local news.").value(&c.lg.opt_text("option.zipcode").unwrap_or_default());
    zip.detail = if !c.lg.has("option.zipcode") { "TV not answering".into() } else if bad { "Needs 5 digits".into() } else { String::new() };
    zip.enabled = c.lg.usable("option.zipcode");
    zip.visible = c.lg.shown("option.zipcode");
    let current = c.lg.opt_text("option.zipcode").unwrap_or_default();
    out.push(zip.does(vec![
        Op::Set("zipbad".into(), json!(false)),
        Op::Entry {
            prompt: "Zip code".into(),
            text: current,
            secret: false,
            then: Rc::new(|t| {
                let z = t.trim().to_string();
                if z.len() == 5 && z.bytes().all(|b| b.is_ascii_digit()) { vec![Op::Lg { ds: "option.zipcode".into(), value: json!(z), app: None }] } else { vec![Op::Set("zipbad".into(), json!(true))] }
            }),
        },
    ]));
    out
}

/// LG sets the services country first and turns it back off when the TV's service does not find a country.
fn answered(_: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    if name != "country" {
        return Vec::new();
    }
    let ok = doc.get("returnValue").and_then(Value::as_bool) != Some(false) && doc.get("serverResponse").is_none_or(|r| r.get("code").map(|c| c.to_string().trim_matches('"').to_string()) == Some("200".into()));
    if ok { Vec::new() } else { vec![Op::Lg { ds: "option.autoSmartServiceCountry".into(), value: json!("off"), app: None }] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};
    use crate::settings::row::PageArg;

    fn world() -> World {
        let mut w = World::new(lg_with(
            &[
                ("", json!({"settings": {"localeInfo": {"locales": {"UI": "en-US", "FMT": "en-US", "TV": "en-US", "AUD": "en-US"}, "keyboards": ["en"]}}})),
                ("option", json!({"settings": {"country": "USA", "autoSmartServiceCountry": "off", "zipcode": "30311"}})),
            ],
            &[],
        ));
        w.docs.set(
            "locale",
            json!({"returnValue": true,
                "locale": [{"name": "English", "countries": [{"spec": "en-US", "name": "United States"}, {"spec": "en-GB", "name": "United Kingdom"}]}, {"name": "Hindi", "countries": [{"spec": "hi-IN", "name": ""}]}],
                "audio": [{"name": "English", "languageCode": "en-US"}, {"name": "Spanish", "languageCode": "es-US"}],
                "vkb": [{"languageCode": "en", "name": "English"}, {"languageCode": "hi", "name": "Hindi"}, {"languageCode": "es", "name": "Spanish"}]}),
        );
        w.arg = PageArg::None;
        w
    }

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn a_language_is_named_with_its_country_and_a_pick_of_the_menu_one_asks_then_writes_all_three_with_the_keyboards() {
        let w = world();
        let r = rows(&w.ctx());
        let menu = by_id(&r, "menu");
        assert_eq!(menu.value, "English - United States");
        assert!(menu.enabled);
        let ops = (menu.act.as_ref().unwrap())(Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!() };
        assert_eq!(spec.options.iter().map(|o| (o.value.as_str(), o.label.as_str())).collect::<Vec<_>>(), vec![("en-US", "English - United States"), ("en-GB", "English - United Kingdom"), ("hi-IN", "Hindi")]);
        assert!((spec.write)("en-US").is_empty(), "the one in force is not written again");
        let asked = (spec.write)("hi-IN");
        let [Op::Ask(a)] = &asked[..] else { panic!() };
        assert_eq!(a.title, "Change to Hindi?");
        let [Op::LgMany { category, settings, .. }] = &a.answers[1].then[..] else { panic!() };
        assert_eq!(category, "");
        let copy = &settings["localeInfo"];
        assert_eq!((copy["locales"]["UI"].as_str(), copy["locales"]["FMT"].as_str(), copy["locales"]["TV"].as_str(), copy["locales"]["AUD"].as_str()), (Some("hi-IN"), Some("hi-IN"), Some("hi-IN"), Some("en-US")));
        assert_eq!(copy["keyboards"], json!(["en", "hi"]), "English and the language's own");
        let audio = by_id(&r, "audio");
        let ops = (audio.act.as_ref().unwrap())(Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!() };
        let wrote = (spec.write)("es-US");
        assert!(matches!(&wrote[..], [Op::LgMany { settings, .. }] if settings["localeInfo"]["locales"]["AUD"] == "es-US" && settings["localeInfo"]["locales"]["UI"] == "en-US"));
    }

    #[test]
    fn the_keyboards_of_the_menu_language_stay_and_the_others_toggle_in_the_list() {
        let w = world();
        let r = rows(&w.ctx());
        let en = by_id(&r, "kb:en");
        assert!(en.on && !en.enabled && en.detail == "In use");
        let hi = by_id(&r, "kb:hi");
        assert!(!hi.on && hi.enabled, "Hindi is not the menu language here");
        let ops = (hi.act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::LgMany { settings, .. }] if settings["localeInfo"]["keyboards"] == json!(["en", "hi"])));
    }

    #[test]
    fn the_location_shows_the_country_flips_the_services_country_in_place_and_takes_five_digits() {
        let w = world();
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "country").value, "United States");
        let svc = by_id(&r, "svcCountry");
        let ops = (svc.act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::Lg { value, .. }, Op::Call { uri, .. }, Op::Act { .. }] if value == "on" && uri.ends_with("setCountrySettingByAuto")), "{ops:?}");
        assert_eq!(svc.label, "Set LG services country automatically");
        let zip = by_id(&r, "zip");
        assert_eq!(zip.value, "30311");
        let ops = (zip.act.as_ref().unwrap())(Arg::Press);
        let [Op::Set(_, _), Op::Entry { text, then, .. }] = &ops[..] else { panic!() };
        assert_eq!(text, "30311");
        assert!(matches!(&then(" 12345 ".into())[..], [Op::Lg { value, .. }] if value == "12345"));
        assert!(matches!(&then("1234".into())[..], [Op::Set(k, _)] if k == "zipbad"));
        assert!(matches!(&then("12a45".into())[..], [Op::Set(..)]));
    }

    #[test]
    fn a_country_the_tvs_service_did_not_find_turns_the_switch_back_off() {
        let w = world();
        assert!(answered(&w.ctx(), "country", &json!({"returnValue": true, "serverResponse": {"code": 200}})).is_empty());
        assert!(answered(&w.ctx(), "country", &json!({"returnValue": true})).is_empty());
        let back = answered(&w.ctx(), "country", &json!({"returnValue": true, "serverResponse": {"code": 404}}));
        assert!(matches!(&back[..], [Op::Lg { value, .. }] if value == "off"));
        assert!(!answered(&w.ctx(), "country", &json!({"returnValue": false})).is_empty());
    }
}
