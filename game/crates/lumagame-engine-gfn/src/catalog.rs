use lumagame_core::activity::{Activity, genre_label, parse_when};
use lumagame_core::ids::{ExternalId, ProviderGameId};
use lumagame_core::ipc::{Art, LibraryEntry, Readiness, Role};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Selection {
    pub parent: String,
    pub variant: String,
    pub title: String,
    pub store: String,
}

pub fn selections(game: &Value) -> Result<Vec<(LibraryEntry, Selection)>, String> {
    let parent = identity(&game["id"]).ok_or("Catalog game has no stable id")?;
    let title = game["title"].as_str().filter(|s| !s.trim().is_empty()).ok_or("Catalog game has no title")?;
    let variants = game["variants"].as_array().ok_or("Catalog game has no variants")?;
    let mut result = Vec::new();
    let mut activity = Some(activity(game));
    for variant in variants {
        let id = identity(&variant["id"]).ok_or("Catalog variant has no stable id")?;
        // JSON's escaping makes this pair unambiguous even when an upstream id contains a separator.
        let provider = ProviderGameId::new(serde_json::to_string(&[&parent, &id]).unwrap()).map_err(|e| e.to_string())?;
        let store = variant["store"].as_str().unwrap_or("unknown");
        let mut external = vec![ExternalId::new("gfn", &parent).map_err(|e| e.to_string())?];
        if store.eq_ignore_ascii_case("steam") {
            if let Some(steam) = steam_id(variant) {
                external.push(ExternalId::new("steam", steam).map_err(|e| e.to_string())?);
            }
        }
        result.push((LibraryEntry {
            id: provider,
            title: title.to_owned(),
            role: Role::Game,
            external,
            store: Some(store.to_ascii_lowercase()),
            store_account: None,
            art: Art {
                cover: https(&game["imageUrl"]).or_else(|| https(&game["keyArtUrl"])),
                hero: https(&game["heroImageUrl"]),
                ..Art::default()
            },
            // Ownership is not a launch decision: inspect checks patching, plan and store links.
            readiness: Readiness::Unknown,
            resumable: false,
            // The same for every store a game is in, so it is reported once.
            activity: activity.take().unwrap_or_default(),
        }, Selection { parent:parent.clone(), variant:id, title:title.to_owned(), store:store.to_owned() }));
    }
    if result.is_empty() { return Err("Catalog game has no variants".into()); }
    Ok(result)
}

pub fn parse_provider(id: &ProviderGameId) -> Result<Selection, String> {
    let [parent, variant]: [String; 2] = serde_json::from_str(id.as_str()).map_err(|_| "Unknown GFN game identity")?;
    if parent.is_empty() || variant.is_empty() { return Err("Empty GFN game identity".into()); }
    Ok(Selection { parent,variant,title:String::new(),store:String::new() })
}

fn identity(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control) => Some(s.clone()),
        Value::Number(n) if n.as_u64().is_some() => Some(n.to_string()),
        _ => None,
    }
}

/// What GeForce NOW says of the game itself: when it was last played (a date, or a date and time), whether it is marked a favourite
/// there, and its genres.
fn activity(game: &Value) -> Activity {
    let when = game["lastPlayed"].as_str().and_then(parse_when);
    let mut activity = Activity {
        last_played: when.map(|(at, _)| at),
        day_only: when.is_some_and(|(_, day)| day),
        favourite: game["favorited"].as_bool() == Some(true),
        genres: game["genres"].as_array().into_iter().flatten().filter_map(Value::as_str).filter_map(genre_label).collect(),
        ..Activity::default()
    };
    activity.clamp();
    activity
}

fn https(value: &Value) -> Option<String> {
    value.as_str().filter(|s| s.starts_with("https://")).map(str::to_owned)
}

fn steam_id(variant: &Value) -> Option<String> {
    // The normalized OpenNOW catalog retains storeUrl; a GFN variant id is never a Steam app id.
    let url = variant["storeUrl"].as_str()?;
    let tail = url.strip_prefix("https://store.steampowered.com/app/")
        .or_else(|| url.strip_prefix("http://store.steampowered.com/app/"))?;
    let id = tail.split(['/', '?', '#']).next()?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) && id.parse::<u32>().ok()? > 0).then(|| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn identities_are_exact_variant_pairs_and_only_steam_urls_supply_steam_ids() {
        let game = json!({"id":"g:a","title":"Game","variants":[
            {"id":"123","store":"STEAM","inLibrary":true,"storeUrl":"https://store.steampowered.com/app/1245620/"},
            {"id":"456","store":"EPIC","inLibrary":true,"storeUrl":"https://store.steampowered.com/app/1/"}]});
        let entries = selections(&game).unwrap();
        assert_eq!(entries[0].0.external.iter().map(ToString::to_string).collect::<Vec<_>>(), ["gfn:g:a", "steam:1245620"]);
        assert_eq!(entries[1].0.external.len(), 1);
        assert_ne!(entries[0].0.id, entries[1].0.id);
        assert_eq!(parse_provider(&entries[0].0.id).unwrap().variant, "123");
        assert_eq!(entries[0].0.id, selections(&game).unwrap()[0].0.id);
    }

    #[test]
    fn play_favourite_and_genres_are_reported_once_per_game_and_only_when_the_catalog_has_them() {
        let game = json!({"id":"g:a","title":"Game","lastPlayed":"2026-11-21","favorited":true,"genres":["ROLE_PLAYING","Action"," "],
            "variants":[{"id":"1","store":"STEAM"},{"id":"2","store":"EPIC"}]});
        let entries = selections(&game).unwrap();
        let want = Activity { last_played: Some(1_795_219_200), day_only: true, favourite: true, genres: vec!["RPG".into(), "Action".into()], ..Activity::default() };
        assert_eq!(entries[0].0.activity, want);
        assert!(entries[1].0.activity.is_empty(), "the second store of the same game adds nothing");

        let timed = json!({"id":"g:b","title":"Game","lastPlayed":"2026-11-21T19:04:05Z","variants":[{"id":"1","store":"STEAM"}]});
        let activity = selections(&timed).unwrap().remove(0).0.activity;
        assert_eq!((activity.last_played, activity.day_only, activity.favourite), (Some(1_795_287_845), false, false));

        let bare = json!({"id":"g:c","title":"Game","favorited":null,"lastPlayed":"never","variants":[{"id":"1","store":"STEAM"}]});
        assert!(selections(&bare).unwrap()[0].0.activity.is_empty());
    }
}
