use lumagame_core::ids::{ExternalId, ProviderGameId};
use lumagame_core::ipc::{Art, LibraryEntry, Readiness, Role};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Selection {
    pub parent: String,
    pub variant: String,
    pub title: String,
    pub store: String,
    pub owned: bool,
}

pub fn selections(game: &Value) -> Result<Vec<(LibraryEntry, Selection)>, String> {
    let parent = identity(&game["id"]).ok_or("Catalog game has no stable id")?;
    let title = game["title"].as_str().filter(|s| !s.trim().is_empty()).ok_or("Catalog game has no title")?;
    let variants = game["variants"].as_array().ok_or("Catalog game has no variants")?;
    let mut result = Vec::new();
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
        let owned = variant["inLibrary"] == true;
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
        }, Selection { parent:parent.clone(), variant:id, title:title.to_owned(), store:store.to_owned(), owned }));
    }
    if result.is_empty() { return Err("Catalog game has no variants".into()); }
    Ok(result)
}

pub fn parse_provider(id: &ProviderGameId) -> Result<Selection, String> {
    let [parent, variant]: [String; 2] = serde_json::from_str(id.as_str()).map_err(|_| "Unknown GFN game identity")?;
    if parent.is_empty() || variant.is_empty() { return Err("Empty GFN game identity".into()); }
    Ok(Selection { parent,variant,title:String::new(),store:String::new(),owned:false })
}

fn identity(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control) => Some(s.clone()),
        Value::Number(n) if n.as_u64().is_some() => Some(n.to_string()),
        _ => None,
    }
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
}
