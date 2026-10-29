//! Privacy: its own section of Settings (Nitin, 2026-10-29), from the helper's toolkit-neutral menu (`GET /v1/privacy/menu`,
//! `helper/privacy`: the four service groups, the hosts blocker, Prime Video's ad filter, LG's consent flags, the links) and LG's own
//! privacy rows that the QML Settings kept under Support (Do Not Sell, Limit AD Tracking, the AD ID, Live Plus and Live Promotion).
//! Every write is two requests with a person between them: `POST /v1/privacy/prepare` returns the exact change and a confirmation
//! token for 60 seconds, the question shows it, and only an answer of the owner sends `POST /v1/privacy/apply`. Nothing here
//! applies a change by itself and the helper refuses anyone but the owner; the rows are locked for another profile as well.

use crate::settings::ctx::{Ctx, Spec, ask};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Answer, Arg, Ask, Op, Row};
use serde_json::{Value, json};

/// The switches that block something (what the rail counts).
const BLOCKERS: [&str; 6] = ["ads", "settings", "uploaders", "push", "hosts", "amazoff"];

pub fn page() -> Page {
    Page::new("privacy", "Privacy", "shield-check").rail().tabs(&["Blockers", "Consent", "Ads & data"]).reads(reads).rows(rows).summary(summary).answered(answered)
}

/// The 16 settings LG keeps and the blocker holds off, as the helper observed them.
pub fn held() -> Page {
    Page::new("privacy-held", "Held settings", "list-checks").parent("privacy").back("Privacy").hidden_from_search().rows(held_rows)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![
        Read::Get { name: "privacy", path: "/v1/privacy/menu" },
        Read::lg("general", &["doNotSellMyPersonalInformation", "lmt"]),
        Read::lg("option", &["livePlus", "livePromotion"]),
        Read::lg("", &["eulaStatus"]),
        Read::call("ifa", "luna://com.webos.service.sm/deviceid/getIDs", json!({"idType": ["IFAID"]})),
    ]
}

/// The menu as the helper served it, once it has.
fn menu<'a>(c: &'a Ctx) -> Option<&'a Value> {
    c.docs.get("privacy").get("privacy").filter(|m| m.get("items").is_some())
}

fn items<'a>(c: &'a Ctx) -> Vec<&'a Value> {
    menu(c).and_then(|m| m.get("items")).and_then(Value::as_array).map(|l| l.iter().collect()).unwrap_or_default()
}

fn item<'a>(c: &'a Ctx, id: &str) -> Option<&'a Value> {
    items(c).into_iter().find(|i| i.get("id").and_then(Value::as_str) == Some(id))
}

fn text<'a>(it: &'a Value, key: &str) -> &'a str {
    it.get(key).and_then(Value::as_str).unwrap_or("")
}

fn summary(c: &Ctx) -> String {
    if menu(c).is_none() {
        return String::new();
    }
    let on = BLOCKERS.iter().filter(|id| item(c, id).and_then(|i| i.get("on")).and_then(Value::as_bool) == Some(true)).count();
    format!("{on} of {} on", BLOCKERS.len())
}

/// The second line of a switch: only what is true now and worth saying.
fn detail_words(it: &Value) -> String {
    let reason = text(it, "reason");
    if it.get("available").and_then(Value::as_bool) == Some(false) {
        return if reason.is_empty() || reason.len() > 60 { "Not available now".into() } else { reason.to_string() };
    }
    let on = it.get("on").and_then(Value::as_bool) == Some(true);
    match text(it, "id") {
        "hosts" if on => match (it.get("effective").and_then(Value::as_bool), it.get("count").and_then(Value::as_u64)) {
            (Some(true), Some(n)) => format!("{n} names blocked"),
            _ => "On; waiting for the hosts table".into(),
        },
        "amazoff" => {
            let words = match text(it, "protection") {
                "active" => "Filtering the current player",
                "partial" => "Filtering some players",
                "waiting" => "On; waiting for Prime Video to play",
                "unmodified" => "On; this Prime player is not changed",
                "unavailable" => "On; the filter is not running",
                _ => "",
            };
            match (words.is_empty(), reason.is_empty()) {
                (false, false) if reason.len() <= 60 => format!("{words}; {reason}"),
                (false, _) => words.to_string(),
                (true, false) if on && reason.len() <= 60 => reason.to_string(),
                _ => String::new(),
            }
        }
        _ => String::new(),
    }
}

fn icon_of(id: &str) -> &'static str {
    match id {
        "ads" => "eye-off",
        "settings" => "sliders-horizontal",
        "uploaders" => "activity",
        "push" => "bell-off",
        "hosts" => "ban",
        "amazoff" => "clapperboard",
        _ => "shield-check",
    }
}

/// A switch of the menu: its change is prepared, shown and only then applied.
fn switch(it: &Value, tab: usize, enabled: bool, detail: String, help: &str) -> Row {
    let help = first_clause(help);
    let help = help.as_str();
    let id = text(it, "id").to_string();
    let on = it.get("on").and_then(Value::as_bool) == Some(true);
    let ident = id.clone();
    Row::toggle(&format!("privacy:{id}"))
        .tab(tab)
        .icon(icon_of(&id))
        .label(text(it, "title"))
        .help(help)
        .detail(&detail)
        .on(on)
        .enabled(enabled)
        .admin()
        .act(move |a| match a {
            Arg::Toggle(v) => vec![Op::Send { name: Some("privacy.prepare".into()), method: "POST", path: "/v1/privacy/prepare".into(), body: Some(json!({"id": ident, "enabled": v})) }],
            _ => Vec::new(),
        })
}

/// The first clause of a sentence: what the row does, in a few words (the rest is the helper's detail).
fn first_clause(text: &str) -> String {
    let cut = text.find("; ").or_else(|| text.find(". ")).unwrap_or(text.len());
    text[..cut].trim_end_matches('.').to_string()
}

fn rows(c: &Ctx) -> Vec<Row> {
    let mut rows = Vec::new();
    let menu = menu(c);
    // the blockers
    if menu.is_none() {
        let words = if c.topics.helper_up { "Reading the menu" } else { "The helper is not answering" };
        rows.push(Row::empty("Not available", words, "shield-alert", 0));
    }
    for id in BLOCKERS {
        let Some(it) = item(c, id) else { continue };
        let available = it.get("available").and_then(Value::as_bool) != Some(false);
        rows.push(switch(it, 0, available, detail_words(it), text(it, "detail")));
        if id == "settings" {
            let list = it.get("settings").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
            let held = list.iter().filter(|s| s.get("value") == s.get("target")).count();
            if !list.is_empty() {
                rows.push(Row::link("privacy:held").tab(0).icon("list-checks").label("Held settings").value(&format!("{held} of {}", list.len())).does(vec![Op::Open { page: "privacy-held".into(), arg: Default::default() }]));
            }
        }
    }
    // LG's consent flags
    let consents: Vec<&Value> = items(c).into_iter().filter(|i| text(i, "id").starts_with("consent.")).collect();
    if let Some(u) = item(c, "consents").filter(|i| i.get("available").and_then(Value::as_bool) == Some(false)) {
        rows.push(Row::note(&format!("LG's consent flags: {}", if text(u, "reason").is_empty() { "not available" } else { text(u, "reason") }), 1));
    }
    for it in consents {
        let settable = it.get("settable").and_then(Value::as_bool) == Some(true);
        let titles: Vec<&str> = it.get("agreements").and_then(Value::as_array).map(|l| l.iter().filter_map(|a| a.get("title").and_then(Value::as_str)).collect()).unwrap_or_default();
        let help = if titles.is_empty() { String::new() } else { format!("Agreements: {}.", titles.join(", ")) };
        rows.push(switch(it, 1, settable, if settable { String::new() } else { "Only in LG's menus".into() }, &help));
    }
    // LG's own consent options
    rows.push(live_plus(c));
    rows.push(live_promotion(c));
    // ads and data
    rows.push(do_not_sell(c));
    rows.push(c.toggle(Spec::new("general.lmt").tab(2).icon("eye-off").label("Limit AD Tracking").help("Stops the tracking of ad-related data from this TV.")));
    let ad_id = if c.local.flag("adid_hidden") { String::new() } else { ad_id_of(c.docs.get("ifa")) };
    rows.push(Row::info("adid").tab(2).icon("hash").label("AD ID").value(if ad_id.is_empty() { "\u{2013}" } else { &ad_id }));
    rows.push(
        crate::settings::ctx::action("resetad", "Reset AD ID", "Reset", ask("rotate-ccw", "Reset the AD ID?", "The TV gets a new advertising ID.", "Reset", "Cancel", false, vec![
            Op::Act { uri: "luna://com.webos.service.admanager/resetIFA".into(), payload: json!({}) },
            Op::Set("adid_hidden".into(), json!(true)),
            Op::RefreshIn(700),
        ]))
        .tab(2)
        .icon("rotate-ccw")
        .help("Gives the TV a new advertising ID, and ads are tracked from the new one.")
        .admin(),
    );
    rows.push(Row::link("permissions").tab(2).icon("shield-check").label("App Permissions").help("Which apps may use the camera, the microphone, nearby devices and files.").does(vec![Op::Open { page: "permissions".into(), arg: Default::default() }]));
    rows.push(Row::link("deletedata").tab(2).icon("eye-off").label("Delete My Personal Information").help("Deletes the viewing and voice information LG collected from this TV.").does(vec![Op::Open { page: "deletedata".into(), arg: Default::default() }]));
    rows
}

fn ad_id_of(doc: &Value) -> String {
    doc.get("idList").and_then(Value::as_array).and_then(|l| l.iter().find(|i| i.get("idType").and_then(Value::as_str) == Some("IFAID"))).and_then(|i| i.get("idValue")).and_then(Value::as_str).unwrap_or("").to_string()
}

fn do_not_sell(c: &Ctx) -> Row {
    let r = c.toggle(
        Spec::new("general.doNotSellMyPersonalInformation")
            .tab(1 + 1)
            .icon("shield")
            .label("Do Not Sell My Personal Information")
            .help("Tells LG to ask its partners to stop selling data about this TV's use. Live Plus and LG Channels keep working."),
    );
    r.act(|a| match a {
        Arg::Toggle(v) => ask(
            "shield",
            if v { "Stop the sale of this TV's data?" } else { "Allow the sale of this TV's data?" },
            "LG's ad service is told now.",
            if v { "Turn on" } else { "Turn off" },
            "Cancel",
            false,
            vec![Op::Act { uri: "luna://com.webos.service.admanager/updateDNS".into(), payload: json!({"status": if v { "on" } else { "off" }}) }, Op::RefreshIn(700)],
        ),
        _ => Vec::new(),
    })
}

fn live_plus(c: &Ctx) -> Row {
    let r = c.toggle(Spec::new("option.livePlus").tab(1).icon("scan-eye").label("Live Plus").help("LG recognises what the TV shows and uses it for recommendations and ads.").lg_shown());
    r.act(|a| match a {
        Arg::Toggle(true) => ask(
            "scan-eye",
            "Turn Live Plus on?",
            "LG recognises what the TV shows and may use it for recommendations and ads.",
            "Turn on",
            "Cancel",
            false,
            vec![Op::Lg { ds: "option.livePlus".into(), value: json!("on"), app: None }],
        ),
        Arg::Toggle(false) => vec![Op::Lg { ds: "option.livePlus".into(), value: json!("off"), app: None }],
        _ => Vec::new(),
    })
}

/// Live Promotion follows Live Plus: it is dimmed until that is on.
fn live_promotion(c: &Ctx) -> Row {
    let plus = c.lg.on("option.livePlus");
    let mut r = c.toggle(Spec::new("option.livePromotion").tab(1).icon("star").label("Live Promotion").help("Turns custom ads and services on or off while watching. Needs Live Plus.").lg_shown());
    r.enabled = plus && c.lg.usable("option.livePromotion");
    r.detail = if !c.lg.has("option.livePromotion") { "TV not answering".into() } else if plus { String::new() } else { "Turn Live Plus on".into() };
    r
}

fn held_rows(c: &Ctx) -> Vec<Row> {
    let Some(list) = item(c, "settings").and_then(|i| i.get("settings")).and_then(Value::as_array) else {
        return vec![Row::empty("Not available", "The held settings are read from the helper.", "list-checks", 0)];
    };
    list.iter()
        .map(|s| {
            let key = text(s, "key");
            let detail = text(s, "detail");
            let label: String = detail.chars().next().map(|f| f.to_uppercase().collect::<String>() + &detail[f.len_utf8()..]).unwrap_or_else(|| key.to_string());
            let word = |v: Option<&Value>| match v {
                Some(Value::Bool(true)) => "On".to_string(),
                Some(Value::Bool(false)) => "Off".to_string(),
                Some(Value::String(s)) if s == "on" => "On".into(),
                Some(Value::String(s)) if s == "off" => "Off".into(),
                Some(other) if !other.is_null() => crate::host::sio::js::string(other),
                _ => "\u{2013}".to_string(),
            };
            let held = s.get("value") == s.get("target");
            Row::info(&format!("held:{key}"))
                .icon("sliders-horizontal")
                .label(&label)
                .value(&word(s.get("value")))
                .detail(if s.get("available").and_then(Value::as_bool) == Some(false) { "Not on this TV" } else if held { "" } else { "LG has it on" })
        })
        .collect()
}

/// What the helper's two answers do: a prepared change is shown as a question, an applied one is read back.
fn answered(c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    match name {
        "privacy.prepare" => match doc.get("confirmation").and_then(Value::as_str) {
            Some(token) => {
                let plan = doc.get("plan").cloned().unwrap_or(Value::Null);
                let (key, on) = (text(&plan, "key").to_string(), plan.get("on").and_then(Value::as_bool).unwrap_or(false));
                let title = item(c, &key).map(|i| text(i, "title").to_string()).unwrap_or_else(|| key.clone());
                let verb = if on { "Turn on" } else { "Turn off" };
                vec![Op::Ask(Ask {
                    icon: "shield-check".into(),
                    title: format!("{verb} {title}?"),
                    body: plan_words(c, &key, &plan),
                    answers: vec![
                        Answer { id: "no".into(), label: "Cancel".into(), danger: false, then: Vec::new() },
                        Answer {
                            id: "yes".into(),
                            label: verb.into(),
                            danger: false,
                            then: vec![Op::Send {
                                name: Some("privacy.apply".into()),
                                method: "POST",
                                path: "/v1/privacy/apply".into(),
                                body: Some(json!({"id": key, "enabled": on, "confirmation": token})),
                            }],
                        },
                    ],
                    focus: 0,
                })]
            }
            None => notice(doc),
        },
        "privacy.apply" => {
            let mut ops = if doc.get("ok").and_then(Value::as_bool) == Some(true) { Vec::new() } else { notice(doc) };
            ops.push(Op::Refresh);
            ops
        }
        _ => Vec::new(),
    }
}

/// The helper's words for a change it did not make, as a question with one answer.
fn notice(doc: &Value) -> Vec<Op> {
    let words = doc.pointer("/error/message").and_then(Value::as_str).unwrap_or("The helper did not make the change.");
    vec![Op::Ask(Ask {
        icon: "shield-alert".into(),
        title: "Not changed".into(),
        body: words.to_string(),
        answers: vec![Answer { id: "close".into(), label: "Close".into(), danger: false, then: Vec::new() }],
        focus: 0,
    })]
}

/// What the change is, in words: the item's own sentence and what the plan adds to it.
fn plan_words(c: &Ctx, key: &str, plan: &Value) -> String {
    let it = item(c, key);
    let base = it.map(|i| text(i, "detail")).unwrap_or("");
    match key {
        "hosts" => {
            let n = plan.get("hosts").and_then(Value::as_array).map_or(0, Vec::len);
            format!("{n} advertising and tracker names are blocked in the hosts table. LG's Content Store, ThinQ and system-update blocks stay as they are.")
        }
        "amazoff" => plan.get("notice").and_then(Value::as_str).unwrap_or(base).to_string(),
        k if k.starts_with("consent.") => {
            let changed: Vec<String> = plan
                .pointer("/changes/changed")
                .and_then(Value::as_array)
                .map(|l| l.iter().filter_map(Value::as_str).filter(|f| format!("consent.{f}") != key).map(|f| item(c, &format!("consent.{f}")).map(|i| text(i, "title").to_string()).unwrap_or_else(|| f.to_string())).collect())
                .unwrap_or_default();
            let notice = plan.get("notice").and_then(Value::as_str).unwrap_or("");
            if changed.is_empty() { notice.to_string() } else { format!("{notice} It also changes: {}.", changed.join(", ")) }
        }
        _ => base.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::Kind;

    fn menu_doc() -> Value {
        json!({"ok": true, "privacy": {"id": "privacy", "canAdmin": true, "items": [
            {"id": "ads", "title": "Content recognition & advertising services", "detail": "Stops ACR, content mining, ad overlays and banners.", "kind": "switch", "on": true, "available": true},
            {"id": "settings", "title": "Viewing information, ads & promotions", "detail": "Holds LG's 16 settings off.", "kind": "switch", "on": false, "available": true,
             "settings": [{"key": "option.livePlus", "target": "off", "detail": "Live Plus content recognition", "value": "off", "available": true}, {"key": "general.adCookie", "target": "off", "detail": "ad cookie", "value": "on", "available": true}]},
            {"id": "uploaders", "title": "Telemetry & diagnostic uploaders", "detail": "Stops LG's uploaders.", "kind": "switch", "on": false, "available": false, "reason": "declutter tool missing"},
            {"id": "hosts", "title": "Block ad & tracker hosts", "detail": "Glasshouse's ads tier.", "kind": "switch", "on": true, "available": true, "effective": true, "count": 212},
            {"id": "amazoff", "title": "Prime Video ads (Amazoff)", "detail": "Filters the current player.", "kind": "switch", "on": true, "available": true, "protection": "waiting"},
            {"id": "consent.acrAllowed", "title": "Screen content recognition", "kind": "switch", "on": true, "available": true, "settable": true, "agreements": [{"title": "Analytics"}]},
            {"id": "consent.networkAllowed", "title": "network", "kind": "switch", "on": true, "available": true, "settable": false},
            {"id": "permissions", "kind": "link"}]}})
    }

    fn world() -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.docs.set("privacy", menu_doc());
        w
    }

    #[test]
    fn the_blockers_come_from_the_menu_with_what_is_true_now_and_the_rail_counts_them() {
        let w = world();
        let c = w.ctx();
        assert_eq!(summary(&c), "3 of 6 on", "ads, hosts and amazoff are on");
        let r = rows(&c);
        let tab0: Vec<&Row> = r.iter().filter(|x| x.tab == 0).collect();
        assert_eq!(tab0.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), vec!["Content recognition & advertising services", "Viewing information, ads & promotions", "Held settings", "Telemetry & diagnostic uploaders", "Block ad & tracker hosts", "Prime Video ads (Amazoff)"]);
        let ads = tab0[0];
        assert!(ads.on && ads.enabled && ads.admin && ads.kind == Kind::Toggle);
        assert_eq!(ads.help, "Stops ACR, content mining, ad overlays and banners", "the first clause of the helper's sentence");
        assert_eq!(tab0[2].value, "1 of 2", "one of the two settings is held");
        let up = tab0[3];
        assert!(!up.enabled && up.detail == "declutter tool missing", "unavailable: dimmed with the helper's reason");
        assert_eq!(tab0[4].detail, "212 names blocked");
        assert_eq!(tab0[5].detail, "On; waiting for Prime Video to play");
    }

    #[test]
    fn a_switch_only_asks_the_helper_to_prepare_the_change() {
        let w = world();
        let c = w.ctx();
        let r = rows(&c);
        let ads = r.iter().find(|x| x.id == "privacy:ads").unwrap();
        let ops = (ads.act.as_ref().unwrap())(Arg::Toggle(false));
        let [Op::Send { name, method, path, body }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!((name.as_deref(), *method, path.as_str()), (Some("privacy.prepare"), "POST", "/v1/privacy/prepare"));
        assert_eq!(body.as_ref().unwrap(), &json!({"id": "ads", "enabled": false}));
    }

    #[test]
    fn the_prepared_change_is_shown_exactly_and_only_an_answer_of_the_owner_applies_it() {
        let w = world();
        let c = w.ctx();
        let prepared = json!({"ok": true, "confirmation": "tok123", "expiresInSeconds": 60, "plan": {"key": "hosts", "on": false, "hosts": ["a.example", "b.example"]}});
        let ops = answered(&c, "privacy.prepare", &prepared);
        let [Op::Ask(ask)] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(ask.title, "Turn off Block ad & tracker hosts?");
        assert!(ask.body.starts_with("2 advertising and tracker names are blocked"), "{}", ask.body);
        assert_eq!(ask.answers[0].id, "no", "the safe answer is first, and Back means it");
        assert!(ask.answers[0].then.is_empty());
        let [Op::Send { path, body, .. }] = &ask.answers[1].then[..] else { panic!() };
        assert_eq!(path, "/v1/privacy/apply");
        assert_eq!(body.as_ref().unwrap(), &json!({"id": "hosts", "enabled": false, "confirmation": "tok123"}));
        // a refusal is shown, not applied
        let ops = answered(&c, "privacy.prepare", &json!({"ok": false, "error": {"code": "privacy_change_refused", "message": "this consent can only be changed in LG's own menus"}}));
        let [Op::Ask(ask)] = &ops[..] else { panic!() };
        assert_eq!((ask.title.as_str(), ask.answers.len()), ("Not changed", 1));
        assert!(ask.body.contains("LG's own menus"));
        // an applied change is read back; a refused one is shown first
        assert!(matches!(&answered(&c, "privacy.apply", &json!({"ok": true}))[..], [Op::Refresh]));
        assert!(matches!(&answered(&c, "privacy.apply", &json!({"ok": false, "error": {"message": "expired"}}))[..], [Op::Ask(_), Op::Refresh]));
    }

    #[test]
    fn consent_flags_that_only_lg_may_change_are_dimmed_and_the_agreements_are_the_help() {
        let w = world();
        let c = w.ctx();
        let r = rows(&c);
        let acr = r.iter().find(|x| x.id == "privacy:consent.acrAllowed").unwrap();
        assert!(acr.enabled && acr.tab == 1 && acr.help == "Agreements: Analytics");
        let net = r.iter().find(|x| x.id == "privacy:consent.networkAllowed").unwrap();
        assert!(!net.enabled && net.detail == "Only in LG's menus");
    }

    #[test]
    fn lgs_own_privacy_rows_ask_before_they_turn_on_and_live_promotion_follows_live_plus() {
        let mut w = world();
        w.lg = crate::settings::ctx::fixtures::lg_with(&[("option", json!({"settings": {"livePlus": "off", "livePromotion": "on"}})), ("general", json!({"settings": {"doNotSellMyPersonalInformation": "off", "lmt": "off"}}))], &[]);
        let c = w.ctx();
        let r = rows(&c);
        let plus = r.iter().find(|x| x.id == "option.livePlus").unwrap();
        assert!(!plus.on);
        assert!(matches!(&(plus.act.as_ref().unwrap())(Arg::Toggle(true))[..], [Op::Ask(_)]), "turning it on asks first");
        assert!(matches!(&(plus.act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Lg { .. }]), "off does not");
        let promo = r.iter().find(|x| x.id == "option.livePromotion").unwrap();
        assert!(!promo.enabled && promo.detail == "Turn Live Plus on");
        let dns = r.iter().find(|x| x.id == "general.doNotSellMyPersonalInformation").unwrap();
        assert!(matches!(&(dns.act.as_ref().unwrap())(Arg::Toggle(true))[..], [Op::Ask(_)]));
    }

    #[test]
    fn without_a_menu_the_blockers_say_so_and_the_held_page_lists_what_the_helper_observed() {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        let r = rows(&w.ctx());
        assert!(r.iter().any(|x| x.kind == Kind::Empty && x.tab == 0));
        assert_eq!(summary(&w.ctx()), "");
        w.docs.set("privacy", menu_doc());
        let held = held_rows(&w.ctx());
        assert_eq!(held.iter().map(|x| (x.label.as_str(), x.value.as_str(), x.detail.as_str())).collect::<Vec<_>>(), vec![("Live Plus content recognition", "Off", ""), ("Ad cookie", "On", "LG has it on")]);
    }
}
