//! Privacy: its own section of Settings (Nitin, 2026-10-29), from the helper's toolkit-neutral menu (`GET /v1/privacy/menu`,
//! `helper/privacy`: the four service groups, the hosts blocker, Prime Video's ad filter, LG's consent flags, the links) and LG's own
//! privacy rows that the QML Settings kept under Support (Do Not Sell, Limit AD Tracking, the AD ID, Live Plus and Live Promotion).
//!
//! Wording (`docs/research/settings-wording-audit.md`): a switch names what is ON when it is on. A blocker says "Block ...", a consent
//! says "Allow ...". A switch flips in place and a toast says what was done; it never opens a question. A change the helper needs two
//! requests for (`POST /v1/privacy/prepare`, then `/apply` with the single-use token it returns) is sent as both at once: the owner's
//! press is the answer, the helper still refuses anyone but the owner, and the rows are locked for another profile. What restarts
//! something (the Prime Video ad blocker) is a status row with a chevron and one question, not a switch.

use crate::settings::ctx::{Ctx, Spec, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

/// The switches that block something (what the rail counts).
const BLOCKERS: [&str; 6] = ["ads", "settings", "uploaders", "push", "hosts", "amazoff"];

pub fn page() -> Page {
    Page::new("privacy", "Privacy", "shield-check").rail().tabs(&["Blocking", "LG may collect", "Ads & data"]).reads(reads).rows(rows).summary(summary).answered(answered)
}

/// The settings LG keeps and the first blocker holds off, as the helper observed them.
pub fn held() -> Page {
    Page::new("privacy-held", "LG privacy settings kept off", "list-checks").parent("privacy").back("Privacy").hidden_from_search().rows(held_rows)
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
    format!("{on} of {} blocking", BLOCKERS.len())
}

/// The second line of a switch: only what is true now and worth saying.
fn detail_words(it: &Value) -> String {
    let reason = text(it, "reason");
    if it.get("available").and_then(Value::as_bool) == Some(false) {
        // only the Prime Video row's reason is in plain words (the others name a tool of the helper)
        return if text(it, "id") != "amazoff" || reason.is_empty() || reason.len() > 60 { "Not available on this TV".into() } else { reason.to_string() };
    }
    let on = it.get("on").and_then(Value::as_bool) == Some(true);
    match text(it, "id") {
        "hosts" if on => match (it.get("effective").and_then(Value::as_bool), it.get("count").and_then(Value::as_u64)) {
            (Some(true), Some(n)) => format!("{n} names blocked"),
            _ => "Not active yet".into(),
        },
        "remoteservice" => if on { "Allowed".into() } else { "Blocked".into() },
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
        "remoteservice" => "cast",
        _ => "shield-check",
    }
}

/// What the toast says once a change is made: the blocker or the consent, and which way it went.
fn done_words(id: &str, title: &str, on: bool) -> String {
    let (blocked, allowed) = match id {
        "ads" => ("Content recognition and ads blocked", "Content recognition and ads allowed"),
        "settings" => ("LG's viewing and ad settings kept off", "LG's viewing and ad settings no longer kept off"),
        "uploaders" => ("Telemetry uploads blocked", "Telemetry uploads allowed"),
        "push" => ("LG push promotions blocked", "LG push promotions allowed"),
        "hosts" => ("Ad and tracker hosts blocked", "Ad and tracker hosts allowed"),
        "amazoff" => ("Prime Video ads blocked", "Prime Video ads allowed"),
        _ => {
            let what = title.strip_prefix("Allow ").or_else(|| title.strip_prefix("Accept ")).unwrap_or(title);
            let mut chars = what.chars();
            let what = chars.next().map(|f| f.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default();
            return if on { format!("{what} allowed") } else { format!("{what} no longer allowed") };
        }
    };
    (if on { blocked } else { allowed }).to_string()
}

fn pending_key(id: &str) -> String {
    format!("pend:{id}")
}

/// A switch of the menu: it flips at once (the page's own `pend:` answer holds it until the helper's menu agrees) and the change is
/// prepared and applied in one go.
fn switch(c: &Ctx, it: &Value, tab: usize, enabled: bool, detail: String, help: &str) -> Row {
    let help = first_clause(help);
    let help = help.as_str();
    let id = text(it, "id").to_string();
    let on = match c.local.text(&pending_key(&id)).as_str() {
        "on" => true,
        "off" => false,
        _ => it.get("on").and_then(Value::as_bool) == Some(true),
    };
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
            Arg::Toggle(v) => change(&ident, v),
            _ => Vec::new(),
        })
}

/// The first of the two requests; the answer to it sends the second (see [`answered`]).
fn change(id: &str, on: bool) -> Vec<Op> {
    vec![
        Op::Set(pending_key(id), json!(if on { "on" } else { "off" })),
        Op::Send { name: Some(format!("privacy.prepare:{id}")), method: "POST", path: "/v1/privacy/prepare".into(), body: Some(json!({"id": id, "enabled": on})) },
    ]
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
        if id == "amazoff" {
            rows.push(prime_ads(c, it));
            continue;
        }
        let available = it.get("available").and_then(Value::as_bool) != Some(false);
        rows.push(switch(c, it, 0, available, detail_words(it), text(it, "detail")));
        if id == "settings" {
            let list = it.get("settings").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
            let held = list.iter().filter(|s| s.get("available").and_then(Value::as_bool) != Some(false) && s.get("value") == s.get("target")).count();
            if !list.is_empty() {
                rows.push(Row::link("privacy:held").tab(0).icon("list-checks").label("LG privacy settings kept off").value(&format!("{held} of {} kept off", list.len())).does(vec![Op::Open { page: "privacy-held".into(), arg: Default::default() }]));
            }
        }
    }
    // what LG may collect
    let consents: Vec<&Value> = items(c).into_iter().filter(|i| text(i, "id").starts_with("consent.")).collect();
    if let Some(u) = item(c, "consents").filter(|i| i.get("available").and_then(Value::as_bool) == Some(false)) {
        rows.push(Row::note(&format!("LG's consent flags: {}", if text(u, "reason").is_empty() { "not available" } else { text(u, "reason") }), 1));
    }
    for it in consents {
        let settable = it.get("settable").and_then(Value::as_bool) == Some(true);
        let titles: Vec<&str> = it.get("agreements").and_then(Value::as_array).map(|l| l.iter().filter_map(|a| a.get("title").and_then(Value::as_str)).collect()).unwrap_or_default();
        let help = if titles.is_empty() { String::new() } else { format!("Agreements: {}.", titles.join(", ")) };
        rows.push(switch(c, it, 1, settable, if settable { String::new() } else { "Only in LG's menus".into() }, &help));
    }
    rows.push(remote_service(c));
    rows.push(live_plus(c, 1));
    // ads and data
    rows.push(live_promotion(c, 2));
    rows.push(do_not_sell(c, 2));
    rows.push(limit_tracking(c, 2));
    let ad_id = if c.local.flag("adid_hidden") { String::new() } else { ad_id_of(c.docs.get("ifa")) };
    rows.push(Row::info("adid").tab(2).icon("hash").label("AD ID").value(if ad_id.is_empty() { "\u{2013}" } else { &ad_id }));
    rows.push(
        crate::settings::ctx::action("resetad", "Reset AD ID", "Reset", ask("rotate-ccw", "Reset the AD ID?", "The TV gets a new advertising ID.", "Reset", "Cancel", false, vec![
            Op::Act { uri: "luna://com.webos.service.admanager/resetIFA".into(), payload: json!({}) },
            Op::Set("adid_hidden".into(), json!(true)),
            Op::RefreshIn(700),
            toast("Advertising ID reset"),
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

/// Prime Video's ad blocker restarts Prime Video and switches the old AmazOff app off, so it is a status with one question, not a
/// switch. The helper does the setup itself (`helper/privacy/src/prime.rs`).
fn prime_ads(c: &Ctx, it: &Value) -> Row {
    let available = it.get("available").and_then(Value::as_bool) != Some(false);
    let on = it.get("on").and_then(Value::as_bool) == Some(true);
    let busy = c.local.flag("busy:amazoff");
    let value = if busy {
        "Working\u{2026}"
    } else if !available {
        "Not available"
    } else if !on {
        "Allowed"
    } else {
        match text(it, "protection") {
            "partial" => "Partly blocked",
            "waiting" => "Ready",
            "unmodified" => "Cannot block here",
            "unavailable" => "Not running",
            _ => "Blocked",
        }
    };
    let detail = if !available {
        detail_words(it)
    } else if busy {
        String::new()
    } else {
        match (on, text(it, "protection")) {
            (true, "waiting") => "Starts when Prime Video plays".into(),
            (true, "unmodified") => "This version of Prime Video's player is left as it is".into(),
            (true, "unavailable") if !text(it, "reason").is_empty() && text(it, "reason").len() <= 60 => text(it, "reason").to_string(),
            _ => String::new(),
        }
    };
    let (title, body, yes) = if on {
        ("Allow Prime Video ads again?", "Prime Video restarts and shows its ads again.", "Allow ads")
    } else {
        ("Block Prime Video ads?", "Prime Video restarts, and the old AmazOff app, if it is installed, is switched off.", "Block ads")
    };
    let ops = ask("clapperboard", title, body, yes, "Cancel", false, vec![Op::Set("busy:amazoff".into(), json!(true)), Op::Send { name: Some("privacy.prepare:amazoff".into()), method: "POST", path: "/v1/privacy/prepare".into(), body: Some(json!({"id": "amazoff", "enabled": !on})) }]);
    let mut r = Row::link("privacy:amazoff").tab(0).icon(icon_of("amazoff")).label("Prime Video ads").value(value).detail(&detail).admin().enabled(available && !busy).does(ops);
    r.help = "Takes the ads out of Prime Video.".into();
    r
}

/// LG's Remote Service (LG staff see and control the TV once a code is read out; LG's hidden support menu was its only door). Blocked unless
/// the owner allows it: the helper's owner-only change writes the file the compositor looks for before it lets the app open. Turning it
/// on asks once, in the danger style; off does not. No assistant has a route to it, and nothing here opens the app. Without the helper's
/// item (an older helper) the row only says it is blocked, which the compositor does by itself.
fn remote_service(c: &Ctx) -> Row {
    const HELP: &str = "LG's remote support: with a code read out to LG, its staff can see and control this TV";
    match item(c, "remoteservice") {
        Some(it) => switch(c, it, 1, true, detail_words(it), HELP).act(|a| match a {
            Arg::Toggle(true) => ask("cast", "Allow LG Remote Service?", "With a code read out to LG, its staff can see the screen and control this TV. It stays blocked until it is turned off again.", "Allow", "Cancel", true, change("remoteservice", true)),
            Arg::Toggle(false) => change("remoteservice", false),
            _ => Vec::new(),
        }),
        None => Row::info("privacy:remoteservice").tab(1).icon(icon_of("remoteservice")).label("Allow LG Remote Service").value("Blocked").help(&format!("{HELP}. It stays blocked until the owner allows it here.")),
    }
}

fn ad_id_of(doc: &Value) -> String {
    doc.get("idList").and_then(Value::as_array).and_then(|l| l.iter().find(|i| i.get("idType").and_then(Value::as_str) == Some("IFAID"))).and_then(|i| i.get("idValue")).and_then(Value::as_str).unwrap_or("").to_string()
}

/// "Ask LG not to sell my data": on is the request made (`admanager updateDNS`).
pub fn do_not_sell(c: &Ctx, tab: usize) -> Row {
    let r = c.toggle(
        Spec::new("general.doNotSellMyPersonalInformation")
            .tab(tab)
            .icon("shield")
            .label("Ask LG not to sell my data")
            .help("LG's Do Not Sell My Personal Information. LG asks its partners to stop selling data about this TV's use. Live Plus and LG Channels keep working."),
    );
    r.act(|a| match a {
        Arg::Toggle(v) => vec![
            Op::Act { uri: "luna://com.webos.service.admanager/updateDNS".into(), payload: json!({"status": if v { "on" } else { "off" }}) },
            Op::RefreshIn(700),
            toast(if v { "LG asked not to sell this TV's data" } else { "LG may sell this TV's data" }),
        ],
        _ => Vec::new(),
    })
}

/// "Limit ad tracking": on is the limit in force.
pub fn limit_tracking(c: &Ctx, tab: usize) -> Row {
    c.toggle(Spec::new("general.lmt").tab(tab).icon("eye-off").label("Limit ad tracking").help("Stops the tracking of ad-related data from this TV."))
}

/// "Allow Live Plus (recognises what is on screen)": on is LG recognising what the TV shows.
pub fn live_plus(c: &Ctx, tab: usize) -> Row {
    let r = c.toggle(Spec::new("option.livePlus").tab(tab).icon("scan-eye").label("Allow Live Plus (recognises what is on screen)").help("LG recognises what the TV shows and uses it for recommendations and ads.").lg_shown());
    r.act(|a| match a {
        Arg::Toggle(v) => vec![Op::Lg { ds: "option.livePlus".into(), value: json!(if v { "on" } else { "off" }), app: None }, toast(if v { "Live Plus allowed" } else { "Live Plus no longer allowed" })],
        _ => Vec::new(),
    })
}

/// Live Promotion follows Live Plus: it is dimmed until that is on.
pub fn live_promotion(c: &Ctx, tab: usize) -> Row {
    let plus = c.lg.on("option.livePlus");
    let mut r = c.toggle(Spec::new("option.livePromotion").tab(tab).icon("star").label("Allow custom ads while watching (Live Promotion)").help("LG's custom ads and services while watching. Needs Live Plus.").lg_shown());
    r.enabled = plus && c.lg.usable("option.livePromotion");
    r.detail = if !c.lg.has("option.livePromotion") { "TV not answering".into() } else if plus { String::new() } else { "Allow Live Plus first".into() };
    r.act(|a| match a {
        Arg::Toggle(v) => vec![Op::Lg { ds: "option.livePromotion".into(), value: json!(if v { "on" } else { "off" }), app: None }, toast(if v { "Custom ads while watching allowed" } else { "Custom ads while watching no longer allowed" })],
        _ => Vec::new(),
    })
}

/// LG's 16 privacy settings by what LG would do with each, grouped by theme: (key, plain name).
const HELD_THEMES: [(&str, &[(&str, &str)]); 3] = [
    (
        "Viewing data",
        &[
            ("option.livePlus", "Recognise what is on screen (Live Plus)"),
            ("option.watchedListCollection", "Collect my viewing history"),
            ("option.thirdPartyCookie", "Third-party cookies"),
            ("option.dbgLogUpload", "Upload debug logs to LG"),
            ("option.usageCare", "Send usage statistics to LG"),
        ],
    ),
    (
        "Ads",
        &[
            ("general.customizedAd", "Personalised ads"),
            ("general.adCookie", "Advertising cookie"),
            ("option.livePromotion", "Custom ads while watching (Live Promotion)"),
            ("option.promotionStreamer", "Streaming-service promotions"),
            ("option.webOSPromotionVideo", "Promotion videos"),
            ("general.homePromotion", "Promotions on LG's Home"),
            ("general.screenSaverAd", "Ads on LG's screen saver"),
            ("general.sportsAlarm", "Sports alert promotions"),
        ],
    ),
    (
        "Tips and suggestions",
        &[("other.contentRecommendation", "Content recommendations"), ("general.aiNudge", "Feature tips on Home"), ("general.aiSettingsNudge", "Setting tips in LG's Settings")],
    ),
];

fn held_rows(c: &Ctx) -> Vec<Row> {
    let Some(it) = item(c, "settings") else {
        return vec![Row::empty("Not available", "The settings are read from the helper.", "list-checks", 0)];
    };
    let Some(list) = it.get("settings").and_then(Value::as_array) else {
        return vec![Row::empty("Not available", "The settings are read from the helper.", "list-checks", 0)];
    };
    let holding = it.get("on").and_then(Value::as_bool) == Some(true);
    let mut rows = vec![Row::note(
        if holding { "LumaTV switches these back off if LG turns them on." } else { "These are off only while \u{201c}Keep LG's viewing and ad settings off\u{201d} is on." },
        0,
    )];
    let state = |s: &Value| {
        let held = s.get("value") == s.get("target");
        if s.get("available").and_then(Value::as_bool) == Some(false) {
            "Not on this TV"
        } else if holding && held {
            "Kept off"
        } else if holding {
            "On now"
        } else if held {
            "Off"
        } else {
            "On"
        }
    };
    let mut seen: Vec<&str> = Vec::new();
    for (head, members) in HELD_THEMES {
        let here: Vec<(&Value, &str)> = members.iter().filter_map(|(key, name)| list.iter().find(|s| text(s, "key") == *key).map(|s| (s, *name))).collect();
        if here.is_empty() {
            continue;
        }
        rows.push(Row::head(head, 0));
        for (s, name) in here {
            seen.push(text(s, "key"));
            rows.push(Row::info(&format!("held:{}", text(s, "key"))).icon("sliders-horizontal").label(name).value(state(s)));
        }
    }
    // a setting the helper lists that this screen has no plain name for yet: its own words, last
    let rest: Vec<&Value> = list.iter().filter(|s| !seen.contains(&text(s, "key"))).collect();
    if !rest.is_empty() {
        rows.push(Row::head("More", 0));
        for s in rest {
            let detail = text(s, "detail");
            let name: String = detail.chars().next().map(|f| f.to_uppercase().collect::<String>() + &detail[f.len_utf8()..]).unwrap_or_else(|| text(s, "key").to_string());
            rows.push(Row::info(&format!("held:{}", text(s, "key"))).icon("sliders-horizontal").label(&name).value(state(s)));
        }
    }
    rows
}

/// What the helper's two answers do: a prepared change is applied at once, an applied one is read back and said in a toast.
fn answered(c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    let failed = |id: &str| {
        let words = doc.pointer("/error/message").and_then(Value::as_str).unwrap_or("The helper did not make the change");
        vec![Op::Set(pending_key(id), Value::Null), Op::Set(format!("busy:{id}"), Value::Null), toast(&format!("Not changed: {words}")), Op::Refresh]
    };
    if let Some(id) = name.strip_prefix("privacy.prepare:") {
        let Some(token) = doc.get("confirmation").and_then(Value::as_str) else { return failed(id) };
        let plan = doc.get("plan").cloned().unwrap_or(Value::Null);
        let on = plan.get("on").and_then(Value::as_bool).unwrap_or(false);
        return vec![
            Op::Set(format!("also:{id}"), json!(also_words(c, id, &plan))),
            Op::Send { name: Some(format!("privacy.apply:{id}")), method: "POST", path: "/v1/privacy/apply".into(), body: Some(json!({"id": id, "enabled": on, "confirmation": token})) },
        ];
    }
    if let Some(id) = name.strip_prefix("privacy.apply:") {
        if doc.get("ok").and_then(Value::as_bool) != Some(true) {
            return failed(id);
        }
        let on = doc.get("on").and_then(Value::as_bool).unwrap_or(false);
        let title = item(c, id).map(|i| text(i, "title")).unwrap_or("");
        let also = c.local.text(&format!("also:{id}"));
        let said = if also.is_empty() { done_words(id, title, on) } else { format!("{}. {also}", done_words(id, title, on)) };
        return vec![
            toast(&said),
            Op::Refresh,
            Op::Set(format!("busy:{id}"), Value::Null),
            Op::Later { key: pending_key(id), ms: 1500, then: vec![Op::Set(pending_key(id), Value::Null)] },
        ];
    }
    Vec::new()
}

/// A consent shares its agreements with others: the plan lists what else changes, and the toast says so.
fn also_words(c: &Ctx, key: &str, plan: &Value) -> String {
    if !key.starts_with("consent.") {
        return String::new();
    }
    let others: Vec<String> = plan
        .pointer("/changes/changed")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .filter(|f| format!("consent.{f}") != key)
                .map(|f| {
                    let title = item(c, &format!("consent.{f}")).map(|i| text(i, "title").to_string()).unwrap_or_else(|| f.to_string());
                    title.strip_prefix("Allow ").unwrap_or(&title).to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    if others.is_empty() { String::new() } else { format!("Also changed: {}", others.join(", ")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::{Ask, Kind};

    fn menu_doc() -> Value {
        json!({"ok": true, "privacy": {"id": "privacy", "canAdmin": true, "items": [
            {"id": "ads", "title": "Block content recognition (ACR) and ads", "detail": "Stops content recognition, ad overlays and banners.", "kind": "switch", "on": true, "available": true},
            {"id": "settings", "title": "Keep LG's viewing and ad settings off", "detail": "Switches LG's 16 settings off.", "kind": "switch", "on": true, "available": true,
             "settings": [{"key": "option.livePlus", "target": "off", "detail": "Live Plus content recognition", "value": "off", "available": true}, {"key": "general.adCookie", "target": "off", "detail": "ad cookie", "value": "on", "available": true}, {"key": "general.newThing", "target": "off", "detail": "new thing", "value": "off", "available": false}]},
            {"id": "uploaders", "title": "Block telemetry uploads", "detail": "Stops LG's uploads.", "kind": "switch", "on": false, "available": false, "reason": "declutter tool missing"},
            {"id": "hosts", "title": "Block ad & tracker hosts", "detail": "Blocks advertising names.", "kind": "switch", "on": true, "available": true, "effective": true, "count": 212},
            {"id": "amazoff", "title": "Block Prime Video ads", "detail": "Takes the ads out of Prime Video.", "kind": "switch", "on": true, "available": true, "protection": "waiting"},
            {"id": "remoteservice", "title": "Allow LG Remote Service", "detail": "LG's remote support.", "kind": "switch", "on": false, "available": true, "admin": true, "confirm": true},
            {"id": "consent.acrAllowed", "title": "Allow content recognition (ACR)", "kind": "switch", "on": true, "available": true, "settable": true, "agreements": [{"title": "Analytics"}]},
            {"id": "consent.customAdAllowed", "title": "Allow personalised ads", "kind": "switch", "on": true, "available": true, "settable": true},
            {"id": "consent.networkAllowed", "title": "Accept LG's network service terms", "kind": "switch", "on": true, "available": true, "settable": false},
            {"id": "permissions", "kind": "link"}]}})
    }

    fn world() -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.docs.set("privacy", menu_doc());
        w
    }

    fn toasts(ops: &[Op]) -> Vec<String> {
        ops.iter().filter_map(|o| match o { Op::Act { uri, payload } if uri.ends_with("/createToast") => payload["message"].as_str().map(str::to_string), _ => None }).collect()
    }

    fn ask_of(ops: &[Op]) -> &Ask {
        let [Op::Ask(a)] = ops else { panic!("{ops:?}") };
        a
    }

    #[test]
    fn every_blocker_says_what_on_means_and_the_rail_counts_the_blocking() {
        let w = world();
        let c = w.ctx();
        assert_eq!(summary(&c), "4 of 6 blocking", "ads, settings, hosts and amazoff are on");
        let r = rows(&c);
        let tab0: Vec<&Row> = r.iter().filter(|x| x.tab == 0).collect();
        assert_eq!(
            tab0.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(),
            vec!["Block content recognition (ACR) and ads", "Keep LG's viewing and ad settings off", "LG privacy settings kept off", "Block telemetry uploads", "Block ad & tracker hosts", "Prime Video ads"]
        );
        let ads = tab0[0];
        assert!(ads.on && ads.enabled && ads.admin && ads.kind == Kind::Toggle);
        assert_eq!(ads.help, "Stops content recognition, ad overlays and banners", "the first clause of the helper's sentence");
        assert_eq!(tab0[2].value, "1 of 3 kept off", "one of the three settings is held");
        let up = tab0[3];
        assert!(!up.enabled && up.detail == "Not available on this TV", "unavailable: dimmed, and the helper's jargon is not shown: {}", up.detail);
        assert_eq!(tab0[4].detail, "212 names blocked");
        for x in &tab0 {
            assert!(x.kind != Kind::Toggle || ["Block ", "Keep "].iter().any(|v| x.label.starts_with(v)), "a protection starts with its verb: {}", x.label);
        }
        for x in r.iter().filter(|x| x.tab == 1 && x.kind == Kind::Toggle) {
            assert!(x.label.starts_with("Allow ") || x.label.starts_with("Accept "), "a consent says what is allowed: {}", x.label);
        }
    }

    #[test]
    fn on_in_the_row_is_on_in_the_helper_for_every_privacy_switch_and_nothing_asks_first() {
        let w = world();
        let c = w.ctx();
        let r = rows(&c);
        for it in items(&c).into_iter().filter(|i| i.get("kind").and_then(Value::as_str) == Some("switch") && !["amazoff", "remoteservice"].contains(&text(i, "id"))) {
            let id = text(it, "id");
            let row = r.iter().find(|x| x.id == format!("privacy:{id}")).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(row.on, it["on"].as_bool().unwrap(), "{id}: the switch shows the helper's state, not its opposite");
            for want in [true, false] {
                let ops = (row.act.as_ref().unwrap())(Arg::Toggle(want));
                assert!(!ops.iter().any(|o| matches!(o, Op::Ask(_))), "{id}: a switch opens no question");
                assert!(matches!(&ops[0], Op::Set(k, v) if *k == format!("pend:{id}") && v == &json!(if want { "on" } else { "off" })), "{id}: it flips at once: {ops:?}");
                let Op::Send { name, method, path, body } = &ops[1] else { panic!("{ops:?}") };
                assert_eq!((name.as_deref().unwrap(), *method, path.as_str()), (format!("privacy.prepare:{id}").as_str(), "POST", "/v1/privacy/prepare"));
                assert_eq!(body.as_ref().unwrap(), &json!({"id": id, "enabled": want}), "{id}: ON in the UI sends enabled=true, and the helper's on is blocking/allowing");
            }
        }
    }

    #[test]
    fn a_pressed_switch_shows_the_new_state_at_once_and_the_prepared_change_is_applied_and_said_in_a_toast() {
        let mut w = world();
        w.docs.set("privacy", menu_doc());
        let c = w.ctx();
        let prepared = json!({"ok": true, "confirmation": "tok123", "expiresInSeconds": 60, "plan": {"key": "hosts", "on": false}});
        let ops = answered(&c, "privacy.prepare:hosts", &prepared);
        let Some(Op::Send { name, path, body, .. }) = ops.last() else { panic!("{ops:?}") };
        assert_eq!((name.as_deref(), path.as_str()), (Some("privacy.apply:hosts"), "/v1/privacy/apply"));
        assert_eq!(body.as_ref().unwrap(), &json!({"id": "hosts", "enabled": false, "confirmation": "tok123"}));
        assert!(!ops.iter().any(|o| matches!(o, Op::Ask(_))), "no question: the owner's press was the answer");
        // applied: a toast says what was done, in the direction it went
        let said = |id: &str, on: bool| toasts(&answered(&c, &format!("privacy.apply:{id}"), &json!({"ok": true, "changed": id, "on": on})));
        assert_eq!(said("ads", true), vec!["Content recognition and ads blocked"]);
        assert_eq!(said("ads", false), vec!["Content recognition and ads allowed"]);
        assert_eq!(said("uploaders", true), vec!["Telemetry uploads blocked"]);
        assert_eq!(said("hosts", false), vec!["Ad and tracker hosts allowed"]);
        assert_eq!(said("consent.acrAllowed", true), vec!["Content recognition (ACR) allowed"]);
        assert_eq!(said("consent.customAdAllowed", false), vec!["Personalised ads no longer allowed"]);
        // a refusal flips the switch back and says why
        let ops = answered(&c, "privacy.prepare:consent.customAdAllowed", &json!({"ok": false, "error": {"code": "privacy_change_refused", "message": "this consent can only be changed in LG's own menus"}}));
        assert!(ops.iter().any(|o| matches!(o, Op::Set(k, Value::Null) if k == "pend:consent.customAdAllowed")));
        assert_eq!(toasts(&ops), vec!["Not changed: this consent can only be changed in LG's own menus"]);
        let ops = answered(&c, "privacy.apply:ads", &json!({"ok": false, "error": {"message": "expired"}}));
        assert_eq!(toasts(&ops), vec!["Not changed: expired"]);
    }

    #[test]
    fn a_consent_that_shares_its_agreement_says_what_else_changed() {
        let w = world();
        let c = w.ctx();
        let plan = json!({"key": "consent.acrAllowed", "on": false, "changes": {"changed": ["acrAllowed", "customAdAllowed"]}});
        let ops = answered(&c, "privacy.prepare:consent.acrAllowed", &json!({"ok": true, "confirmation": "t", "plan": plan}));
        assert!(matches!(&ops[0], Op::Set(k, v) if k == "also:consent.acrAllowed" && v == &json!("Also changed: personalised ads")), "{ops:?}");
    }

    #[test]
    fn the_pending_state_holds_the_flip_until_the_menu_agrees() {
        let mut w = world();
        w.local.set("pend:uploaders", json!("on"));
        w.local.set("pend:ads", json!("off"));
        let c = w.ctx();
        let r = rows(&c);
        assert!(r.iter().find(|x| x.id == "privacy:ads").is_some_and(|x| !x.on), "the helper says on, the owner just pressed off");
        assert!(r.iter().find(|x| x.id == "privacy:uploaders").is_some_and(|x| x.on));
    }

    #[test]
    fn prime_video_ads_are_a_status_with_a_chevron_and_one_plain_question() {
        let mut w = world();
        let find = |w: &World| rows(&w.ctx()).into_iter().find(|x| x.id == "privacy:amazoff").unwrap();
        let on = find(&w);
        assert_eq!((on.kind, on.label.as_str(), on.value.as_str(), on.detail.as_str()), (Kind::Value, "Prime Video ads", "Ready", "Starts when Prime Video plays"));
        assert!(on.chevron && on.enabled);
        let ops = (on.act.as_ref().unwrap())(Arg::Press);
        let a = ask_of(&ops);
        assert_eq!((a.title.as_str(), a.answers[0].label.as_str(), a.answers[1].label.as_str()), ("Allow Prime Video ads again?", "Cancel", "Allow ads"));
        for jargon in ["CA", "goproxy", "rootfs", "DRM", "cloudfront", "certificate"] {
            assert!(!a.body.contains(jargon) && !a.title.contains(jargon), "{}", a.body);
        }
        // off: the question is to block
        let mut doc = menu_doc();
        doc["privacy"]["items"][4]["on"] = json!(false);
        w.docs.set("privacy", doc);
        let off = find(&w);
        assert_eq!((off.value.as_str(), off.detail.as_str()), ("Allowed", ""));
        let ops = (off.act.as_ref().unwrap())(Arg::Press);
        let a = ask_of(&ops);
        assert_eq!((a.title.as_str(), a.answers[1].label.as_str()), ("Block Prime Video ads?", "Block ads"));
        assert_eq!(a.body, "Prime Video restarts, and the old AmazOff app, if it is installed, is switched off.", "one sentence, and nothing left for the owner to do");
        assert!(a.answers[0].then.is_empty(), "Cancel does nothing");
        assert!(matches!(&a.answers[1].then[..], [Op::Set(k, _), Op::Send { body: Some(b), .. }] if k == "busy:amazoff" && b == &json!({"id": "amazoff", "enabled": true})), "ON in the question is the helper's on");
        // a TV without Prime Video says so in words, with no chevron to nowhere
        let mut doc = menu_doc();
        doc["privacy"]["items"][4]["available"] = json!(false);
        doc["privacy"]["items"][4]["reason"] = json!("Prime Video is not installed on this TV");
        w.docs.set("privacy", doc);
        let none = find(&w);
        assert_eq!((none.value.as_str(), none.detail.as_str(), none.enabled), ("Not available", "Prime Video is not installed on this TV", false));
    }

    #[test]
    fn lg_remote_service_is_blocked_asks_once_in_the_danger_style_to_allow_and_never_launches_anything() {
        let mut w = world();
        let r = rows(&w.ctx());
        let row = r.iter().find(|x| x.id == "privacy:remoteservice").unwrap();
        assert_eq!((row.label.as_str(), row.tab, row.on, row.detail.as_str(), row.admin, row.enabled), ("Allow LG Remote Service", 1, false, "Blocked", true, true));
        let ops = (row.act.as_ref().unwrap())(Arg::Toggle(true));
        let [Op::Ask(a)] = &ops[..] else { panic!("{ops:?}") };
        assert!(a.answers[0].then.is_empty() && a.answers[1].danger, "the safe answer first, and allowing is a danger");
        let [Op::Set(k, v), Op::Send { name, path, body, .. }] = &a.answers[1].then[..] else { panic!("{:?}", a.answers[1].then) };
        assert_eq!((k.as_str(), v), ("pend:remoteservice", &json!("on")));
        assert_eq!((name.as_deref(), path.as_str(), body.as_ref().unwrap()), (Some("privacy.prepare:remoteservice"), "/v1/privacy/prepare", &json!({"id": "remoteservice", "enabled": true})));
        // blocking again asks nothing
        let ops = (row.act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&ops[..], [Op::Set(..), Op::Send { body: Some(b), .. }] if b == &json!({"id": "remoteservice", "enabled": false})));
        assert!(r.iter().all(|x| x.act.as_ref().is_none_or(|a| a(Arg::Press).iter().all(|o| !matches!(o, Op::Launch { app, .. } if app.contains("remoteservice"))))), "no row opens the app");
        // what was done is said in the toast
        assert_eq!(toasts(&answered(&w.ctx(), "privacy.apply:remoteservice", &json!({"ok": true, "changed": "remoteservice", "on": true}))), vec!["LG Remote Service allowed"]);
        // without the helper's item (an older helper) it is still said to be blocked, and there is nothing to switch
        w.docs.set("privacy", json!({"ok": true, "privacy": {"id": "privacy", "canAdmin": true, "items": [{"id": "ads", "title": "x", "kind": "switch", "on": false, "available": true}]}}));
        let r = rows(&w.ctx());
        let row = r.iter().find(|x| x.id == "privacy:remoteservice").unwrap();
        assert_eq!((row.value.as_str(), row.act.is_none()), ("Blocked", true));
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
    fn lgs_own_privacy_rows_flip_in_place_say_what_on_means_and_toast() {
        let mut w = world();
        w.lg = crate::settings::ctx::fixtures::lg_with(&[("option", json!({"settings": {"livePlus": "off", "livePromotion": "on"}})), ("general", json!({"settings": {"doNotSellMyPersonalInformation": "off", "lmt": "off"}}))], &[]);
        let c = w.ctx();
        let r = rows(&c);
        let by = |id: &str| r.iter().find(|x| x.id == id).unwrap_or_else(|| panic!("{id}"));
        let plus = by("option.livePlus");
        assert_eq!((plus.label.as_str(), plus.tab, plus.on), ("Allow Live Plus (recognises what is on screen)", 1, false));
        let ops = (plus.act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[0], Op::Lg { ds, value, .. } if ds == "option.livePlus" && value == "on"), "ON in the row is LG's on");
        assert_eq!(toasts(&ops), vec!["Live Plus allowed"]);
        assert_eq!(toasts(&(plus.act.as_ref().unwrap())(Arg::Toggle(false))), vec!["Live Plus no longer allowed"]);
        let promo = by("option.livePromotion");
        assert_eq!((promo.label.as_str(), promo.tab, promo.enabled, promo.detail.as_str()), ("Allow custom ads while watching (Live Promotion)", 2, false, "Allow Live Plus first"));
        let dns = by("general.doNotSellMyPersonalInformation");
        assert_eq!(dns.label, "Ask LG not to sell my data");
        let ops = (dns.act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[0], Op::Act { payload, .. } if payload["status"] == "on"), "ON is the request made");
        assert_eq!(toasts(&ops), vec!["LG asked not to sell this TV's data"]);
        assert!(!(dns.act.as_ref().unwrap())(Arg::Toggle(false)).iter().any(|o| matches!(o, Op::Ask(_))));
        assert_eq!(by("general.lmt").label, "Limit ad tracking");
    }

    #[test]
    fn without_a_menu_the_blockers_say_so() {
        let w = World::new(crate::host::sio::Sio::new(0));
        let r = rows(&w.ctx());
        assert!(r.iter().any(|x| x.kind == Kind::Empty && x.tab == 0));
        assert_eq!(summary(&w.ctx()), "");
    }

    #[test]
    fn the_held_page_explains_itself_in_plain_words_by_theme_with_a_state_in_words() {
        let mut w = world();
        let held = held_rows(&w.ctx());
        assert_eq!(held[0].kind, Kind::Note);
        assert_eq!(held[0].text, "LumaTV switches these back off if LG turns them on.");
        let lines: Vec<(Kind, String, String)> = held.iter().skip(1).map(|x| (x.kind, if x.kind == Kind::Head { x.text.clone() } else { x.label.clone() }, x.value.clone())).collect();
        assert_eq!(
            lines,
            vec![
                (Kind::Head, "Viewing data".to_string(), String::new()),
                (Kind::Value, "Recognise what is on screen (Live Plus)".to_string(), "Kept off".to_string()),
                (Kind::Head, "Ads".to_string(), String::new()),
                (Kind::Value, "Advertising cookie".to_string(), "On now".to_string()),
                (Kind::Head, "More".to_string(), String::new()),
                (Kind::Value, "New thing".to_string(), "Not on this TV".to_string()),
            ]
        );
        // with the switch off nothing is held: the words are plain On and Off
        let mut doc = menu_doc();
        doc["privacy"]["items"][1]["on"] = json!(false);
        w.docs.set("privacy", doc);
        let held = held_rows(&w.ctx());
        assert!(held[0].text.contains("only while"), "{}", held[0].text);
        assert_eq!(held[2].value, "Off");
        assert_eq!(held[4].value, "On");
        // every setting the helper has at the time of writing has a plain name here
        let names: Vec<&str> = HELD_THEMES.iter().flat_map(|(_, m)| m.iter().map(|(k, _)| *k)).collect();
        for line in include_str!("../../../../../payload/declutter/items.txt").lines().filter(|l| l.starts_with("setting|settings|")) {
            let key = line.split('|').nth(2).unwrap();
            assert!(names.contains(&key), "{key} has no plain name");
        }
    }
}
