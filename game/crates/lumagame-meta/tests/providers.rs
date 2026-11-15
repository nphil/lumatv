use lumagame_core::{catalog::{Game, Source}, ids::{ExternalId, GameId}};
use lumagame_meta::{Client, Error, Kind, keyless_art, providers::{Request, Response, Transport}, secrets::Secrets};
use std::{collections::VecDeque, sync::{Arc, Mutex}};

struct Replay { replies: Mutex<VecDeque<(u16, Vec<u8>)>>, requests: Arc<Mutex<Vec<Request>>> }
impl Transport for Replay {
    fn execute(&self, request: Request) -> lumagame_meta::Result<Response> {
        self.requests.lock().unwrap().push(request);
        let (status, body) = self.replies.lock().unwrap().pop_front().expect("unexpected network request");
        Ok(Response { status, body })
    }
}
type Requests = Arc<Mutex<Vec<Request>>>;
fn client(replies: Vec<(u16, Vec<u8>)>, secrets: Secrets) -> (Client<Replay>, Requests) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    (Client::with_transport(Replay { replies: Mutex::new(replies.into()), requests: requests.clone() }, secrets), requests)
}
fn secrets() -> Secrets { Secrets { steamgriddb_key: "sgdb-fixture".into(), twitch_client_id: "fixture-client".into(), twitch_client_secret: "fixture&secret".into() } }
fn id(value: &str) -> ExternalId { value.parse().unwrap() }
const TOKEN: &[u8] = include_bytes!("fixtures/twitch-token.json");
const GAME: &[u8] = include_bytes!("fixtures/igdb-games.json");
const SGDB: &[u8] = include_bytes!("fixtures/steamgriddb.json");

#[test]
fn providers_are_off_without_owner_keys() {
    let (client, requests) = client(vec![], Secrets::default());
    assert!(matches!(client.igdb(&[id("igdb:1942")]), Err(Error::Disabled)));
    assert!(matches!(client.steamgriddb(&[id("steam:292030")], Kind::Cover), Err(Error::Disabled)));
    assert!(requests.lock().unwrap().is_empty());
}
#[test]
fn igdb_replays_reference_response_and_reuses_token() {
    let (client, requests) = client(vec![(200, TOKEN.into()), (200, GAME.into()), (200, GAME.into())], secrets());
    for _ in 0..2 {
        let metadata = client.igdb(&[id("igdb:1942")]).unwrap().unwrap();
        assert_eq!(metadata.title, "The Witcher 3: Wild Hunt");
        assert_eq!(metadata.genres, ["Role-playing (RPG)", "Adventure"]);
        assert_eq!(metadata.external, [id("igdb:1942")]);
        assert!(metadata.summary.is_empty());
        assert!(metadata.credit.contains("IGDB"));
    }
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].url, "https://id.twitch.tv/oauth2/token");
    assert!(String::from_utf8_lossy(&requests[0].body).contains("fixture%26secret"));
    assert!(!requests[0].url.contains("secret"));
    assert!(String::from_utf8_lossy(&requests[1].body).contains("where id = 1942"));
    assert!(requests[1].headers.iter().any(|(key, value)| key == "Client-ID" && value == "fixture-client"));
}
#[test]
fn matching_requires_shared_ids_not_titles() {
    let (client, requests) = client(vec![(200, TOKEN.into()), (200, GAME.into())], secrets());
    assert!(client.igdb(&[id("gfn:1942")]).unwrap().is_none());
    assert!(client.steamgriddb(&[id("igdb:1942")], Kind::Cover).unwrap().is_empty());
    assert!(requests.lock().unwrap().is_empty());
    // Even a provider's erroneous response with a familiar name cannot override the requested identity.
    assert!(client.igdb(&[id("igdb:123")]).unwrap().is_none());
}
#[test]
fn steam_mapping_checks_source_uid_and_ambiguity() {
    let game: serde_json::Value = serde_json::from_slice(GAME).unwrap();
    for (source, uid, expected) in [(1, "292030", true), (2, "292030", false), (1, "999", false)] {
        let external = serde_json::json!([{"uid":uid,"external_game_source":source,"game":game[0]}]);
        let (client, requests) = client(vec![(200, TOKEN.into()), (200, serde_json::to_vec(&external).unwrap())], secrets());
        let metadata = client.igdb(&[id("steam:292030")]).unwrap();
        assert_eq!(metadata.is_some(), expected);
        if let Some(metadata) = metadata { assert!(metadata.external.contains(&id("steam:292030"))); }
        let requests = requests.lock().unwrap();
        assert!(String::from_utf8_lossy(&requests[1].body).contains("external_game_source = 1 & uid = \"292030\""));
        assert!(!String::from_utf8_lossy(&requests[1].body).contains("search"));
    }
    let mut other = game[0].clone();
    other["id"] = 99.into();
    let ambiguous = serde_json::json!([
        {"uid":"292030","external_game_source":1,"game":game[0]},
        {"uid":"292030","external_game_source":1,"game":other}
    ]);
    let (client, _) = client(vec![(200, TOKEN.into()), (200, serde_json::to_vec(&ambiguous).unwrap())], secrets());
    assert!(client.igdb(&[id("steam:292030")]).unwrap().is_none());
}
#[test]
fn sgdb_keeps_contributor_credit_and_queries_exact_id() {
    let (client, requests) = client(vec![(200, SGDB.into())], secrets());
    let art = client.steamgriddb(&[id("steam:292030")], Kind::Cover).unwrap();
    assert_eq!(art.len(), 1);
    assert!(art[0].credit.contains("Fixture contributor"));
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].url, "https://www.steamgriddb.com/api/v2/grids/steam/292030?dimensions=600x900&types=static");
}
#[test]
fn metadata_preserves_fields_and_uses_canonical_image_urls() {
    let mut game: serde_json::Value = serde_json::from_slice(GAME).unwrap();
    game[0]["summary"] = "Recorded summary field".into();
    game[0]["first_release_date"] = 1431993600_i64.into();
    game[0]["involved_companies"] = serde_json::json!([{"developer":true,"publisher":true,"company":{"name":"Fixture studio"}}]);
    game[0]["cover"] = serde_json::json!({"image_id":"co_fixture"});
    game[0]["artworks"] = serde_json::json!([{"image_id":"../escape"},{"image_id":"hero_fixture"}]);
    game[0]["screenshots"] = serde_json::json!([{"image_id":"screen_fixture"}]);
    let (client, _) = client(vec![(200, TOKEN.into()), (200, serde_json::to_vec(&game).unwrap())], secrets());
    let meta = client.igdb(&[id("igdb:1942")]).unwrap().unwrap();
    assert_eq!(meta.release_date, Some(1431993600));
    assert_eq!(meta.developers, ["Fixture studio"]);
    assert_eq!(meta.publishers, ["Fixture studio"]);
    assert_eq!(meta.artwork.len(), 3);
    assert!(meta.artwork[0].url.ends_with("t_cover_big_2x/co_fixture.jpg"));
}
#[test]
fn keyless_steam_art_precedes_engine_art_and_supplies_header() {
    let source: Source = serde_json::from_value(serde_json::json!({
        "engine":"gfn", "target":"account", "entry":{
            "id":"fixture", "title":"The Witcher 3", "role":"game", "external":["steam:292030"],
            "art":{"cover":"https://engine.invalid/cover.jpg","hero":"https://engine.invalid/hero.jpg"},
            "readiness":"unknown"
        }
    })).unwrap();
    let game = Game { id: GameId(1), role: lumagame_core::ipc::Role::Game, title: "The Witcher 3".into(), sources: vec![source] };
    let art = keyless_art(&game, Kind::Cover);
    assert!(art[0].url.ends_with("292030/library_600x900.jpg"));
    assert!(art[1].url.ends_with("292030/library_600x900_2x.jpg"));
    assert_eq!(art[2].url, "https://engine.invalid/cover.jpg");
    assert!(keyless_art(&game, Kind::Hero)[0].url.ends_with("library_hero.jpg"));
    assert!(keyless_art(&game, Kind::Logo)[0].url.ends_with("logo.png"));
    assert!(keyless_art(&game, Kind::Header)[0].url.ends_with("header.jpg"));
}
#[test]
fn provider_failures_do_not_leak_credentials() {
    let (client, _) = client(vec![(401, b"private provider error body".to_vec())], secrets());
    let error = client.steamgriddb(&[id("steam:292030")], Kind::Cover).unwrap_err();
    assert_eq!(error.to_string(), "artwork provider HTTP 401");
}
