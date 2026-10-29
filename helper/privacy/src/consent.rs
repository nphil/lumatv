//! Agreement-aware consent changes. Platform terms and voice never change as a side effect.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
pub const LOCKED: &[&str] = &[
    "generalTermsAllowed",
    "networkAllowed",
    "firstUseAllowed",
    "allAllowed",
    "chpAllowed",
];
/// A consent's switch says what is allowed when it is on.
pub const NAMED: &[(&str, &str)] = &[
    ("acrAllowed", "Allow content recognition (ACR)"),
    ("acrOnAllowed", "Allow content recognition (main consent)"),
    ("acrGdprAllowed", "Allow content recognition (EU consent)"),
    ("acrAdAllowed", "Allow ads based on what I watch"),
    ("customAdAllowed", "Allow personalised ads"),
    (
        "customadsAllowed",
        "Allow personalised ads (second consent)",
    ),
    ("cookiesAllowed", "Allow advertising cookies"),
    (
        "thirdPartySharingAllowed",
        "Allow sharing data with other companies",
    ),
    ("additionalDataAllowed", "Allow extra usage data"),
    ("remoteDiagAllowed", "Allow remote diagnostics uploads"),
    ("voiceAllowed", "Allow voice recordings"),
    ("voice2Allowed", "Allow voice recordings (second consent)"),
    ("generalTermsAllowed", "Accept LG's general terms"),
    ("networkAllowed", "Accept LG's network service terms"),
    ("firstUseAllowed", "Accept LG's first-use terms"),
    ("allAllowed", "Accept all of LG's terms"),
];
pub type Groups = BTreeMap<String, Vec<String>>;
pub fn groups(mapping: &Value) -> Groups {
    mapping
        .pointer("/eulaMappingList/eulaInfo")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry.get("settingKey")?.as_str()?.to_owned(),
                entry
                    .get("mandatory")
                    .or_else(|| entry.get("generalSelectAll"))
                    .and_then(Value::as_array)?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            ))
        })
        .collect()
}
pub fn settable(key: &str, groups: &Groups) -> bool {
    !LOCKED.contains(&key) && (NAMED.iter().any(|(k, _)| *k == key) || groups.contains_key(key))
}
pub fn label(key: &str, groups: &Groups) -> String {
    if let Some((_, label)) = NAMED.iter().find(|(k, _)| *k == key) {
        return (*label).to_owned();
    }
    for id in groups.get(key).into_iter().flatten() {
        match id.as_str() {
            "S_DPA" => return "Allow sharing viewing data with data partners".into(),
            "S_TAK" => return "Allow Who.Where.What? suggestions".into(),
            "S_MKT" => return "Allow marketing messages".into(),
            _ => {}
        }
    }
    // a consent this list does not know: its key in words (`someNewAllowed` is "Allow some new")
    let stem = key.strip_suffix("Allowed").unwrap_or(key);
    let mut words = String::new();
    for c in stem.chars() {
        if c.is_ascii_uppercase() && !words.is_empty() {
            words.push(' ');
        }
        words.push(c.to_ascii_lowercase());
    }
    format!("Allow {words}")
}
pub fn plan(
    key: &str,
    on: bool,
    flags: &Value,
    docs: &Value,
    groups: &Groups,
) -> Result<Value, String> {
    if !settable(key, groups) {
        return Err("this consent can only be changed in LG's own menus".into());
    }
    let flags = flags
        .as_object()
        .ok_or("LG consent flags are unavailable")?;
    if flags.get(key).and_then(Value::as_bool).is_none() {
        return Err("no such consent on this firmware".into());
    }
    let needs = groups
        .get(key)
        .filter(|n| !n.is_empty())
        .ok_or("LG publishes no agreement mapping for this consent")?;
    let list = docs
        .get("eulaList")
        .and_then(Value::as_array)
        .ok_or("LG agreement documents are unavailable")?;
    let ids: BTreeSet<&str> = list.iter().filter_map(|d| d["id"].as_str()).collect();
    if needs.iter().any(|n| !ids.contains(n.as_str())) {
        return Err("a required agreement document is missing".into());
    }
    let mut accepted: BTreeSet<String> = list
        .iter()
        .filter(|d| d["accepted"] == true)
        .filter_map(|d| d["id"].as_str().map(str::to_owned))
        .collect();
    if on {
        accepted.extend(needs.iter().cloned());
    } else {
        let protected: BTreeSet<&str> = groups
            .iter()
            .filter(|(k, _)| LOCKED.contains(&k.as_str()) && flags.get(*k) == Some(&json!(true)))
            .flat_map(|(_, v)| v.iter().map(String::as_str))
            .collect();
        for id in needs {
            if !protected.contains(id.as_str()) {
                accepted.remove(id);
            }
        }
    }
    let mut next = flags.clone();
    for (k, need) in groups {
        if !LOCKED.contains(&k.as_str())
            && next.get(k) == Some(&json!(true))
            && !need.is_empty()
            && !need.iter().all(|d| accepted.contains(d))
        {
            next.insert(k.clone(), json!(false));
        }
    }
    next.insert(key.into(), json!(on));
    for voice in ["voiceAllowed", "voice2Allowed"] {
        let touched = groups.get(voice).into_iter().flatten().any(|id| {
            list.iter()
                .find(|d| d["id"].as_str() == Some(id.as_str()))
                .is_some_and(|d| (d["accepted"] == true) != accepted.contains(id))
        });
        if key != voice && (next.get(voice) != flags.get(voice) || touched) {
            return Err("this agreement is shared with voice; change it in LG's menus".into());
        }
    }
    let mut next_docs = docs.clone();
    for doc in next_docs["eulaList"].as_array_mut().unwrap() {
        if let Some(id) = doc["id"].as_str() {
            doc["accepted"] = json!(accepted.contains(id));
        }
    }
    let changed: Vec<_> = next
        .iter()
        .filter(|(k, v)| flags.get(*k) != Some(*v))
        .map(|(k, _)| k.clone())
        .collect();
    Ok(json!({"eulaStatus": next, "eulaInfoNetwork": next_docs, "changed": changed}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_agreement_revocation_cascades_but_protects_terms() {
        let groups = BTreeMap::from([
            ("acrAllowed".into(), vec!["A".into(), "T".into()]),
            ("customAdAllowed".into(), vec!["A".into()]),
            ("networkAllowed".into(), vec!["T".into()]),
        ]);
        let flags = json!({"acrAllowed":true,"customAdAllowed":true,"networkAllowed":true,"voiceAllowed":true});
        let docs = json!({"eulaList":[{"id":"A","accepted":true,"title":"Analytics"},{"id":"T","accepted":true}]});
        let p = plan("acrAllowed", false, &flags, &docs, &groups).unwrap();
        assert_eq!(p["eulaStatus"]["customAdAllowed"], false);
        assert_eq!(p["eulaStatus"]["networkAllowed"], true);
        assert_eq!(p["eulaInfoNetwork"]["eulaList"][1]["accepted"], true);
        assert_eq!(p["eulaInfoNetwork"]["eulaList"][0]["title"], "Analytics");
        assert!(plan("networkAllowed", false, &flags, &docs, &groups).is_err());
    }
    #[test]
    fn every_consent_switch_says_what_is_allowed_when_it_is_on() {
        let none = Groups::new();
        for (key, _) in NAMED {
            let l = label(key, &none);
            assert!(
                l.starts_with("Allow ") || l.starts_with("Accept "),
                "{key}: {l}"
            );
        }
        assert_eq!(
            label("someNewConsentAllowed", &none),
            "Allow some new consent"
        );
        let g = BTreeMap::from([("odd".to_owned(), vec!["S_TAK".to_owned()])]);
        assert_eq!(label("odd", &g), "Allow Who.Where.What? suggestions");
    }
    #[test]
    fn voice_cannot_be_changed_by_another_consent() {
        let g = BTreeMap::from([
            ("acrAllowed".into(), vec!["A".into()]),
            ("voiceAllowed".into(), vec!["A".into()]),
        ]);
        assert!(
            plan(
                "acrAllowed",
                false,
                &json!({"acrAllowed":true,"voiceAllowed":true}),
                &json!({"eulaList":[{"id":"A","accepted":true}]}),
                &g
            )
            .is_err()
        );
        assert!(
            plan(
                "acrAllowed",
                true,
                &json!({"acrAllowed":false,"voiceAllowed":false}),
                &json!({"eulaList":[{"id":"A","accepted":false}]}),
                &g
            )
            .is_err()
        );
    }
}
