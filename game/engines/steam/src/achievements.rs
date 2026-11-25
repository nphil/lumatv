//! The owner's own achievements in one game, for the game's page. The game's list of achievements, their names and how rare each is
//! come from public sources the app asks itself; this is only which of them the signed-in account has unlocked.

use crate::auth::{Refusal, denied};
use crate::http::{self, Http};
use crate::{Adapter, Clock};
use lumagame_core::ids::{ProviderGameId, TargetId};
use lumagame_core::ipc::{ErrorCode, Event, Unlocked};
use serde::Deserialize;
use serde_json::Value;

/// The most achievements one answer is cut to. A game has a few hundred at most.
const MAX_UNLOCKED: usize = 2000;
const NAME_CHARS: usize = 128;

/// The unlocked achievements in a `GetPlayerAchievements` answer; `None` when the answer is not one.
fn parse(body: &[u8]) -> Option<Vec<Unlocked>> {
    #[derive(Deserialize)]
    struct Body {
        playerstats: Stats,
    }
    #[derive(Deserialize)]
    struct Stats {
        #[serde(default)]
        achievements: Vec<Raw>,
    }
    // Read loosely, like the library: one odd value must not hide the rest.
    #[derive(Deserialize)]
    struct Raw {
        apiname: Option<Value>,
        achieved: Option<Value>,
        unlocktime: Option<Value>,
    }
    let Body { playerstats } = serde_json::from_slice(body).ok()?;
    let unlocked = playerstats
        .achievements
        .into_iter()
        .filter(|raw| raw.achieved.as_ref().and_then(Value::as_u64) == Some(1))
        .filter_map(|raw| {
            let name: String = raw.apiname.as_ref()?.as_str()?.chars().filter(|c| !c.is_control()).take(NAME_CHARS).collect();
            (!name.is_empty()).then(|| Unlocked { name, at: raw.unlocktime.as_ref().and_then(Value::as_u64).filter(|at| *at > 0) })
        })
        .take(MAX_UNLOCKED)
        .collect();
    Some(unlocked)
}

impl<H: Http, C: Clock> Adapter<H, C> {
    /// What the owner has unlocked in `game`. A game without stats, a private profile and a refusal that is not about the sign-in all
    /// answer with an empty list: no progress is shown, nothing is invented, and the sign-in is never given up over one game.
    pub(crate) fn achievements(&mut self, seq: u64, target: TargetId, game: ProviderGameId) {
        let Some(steamid) = self.steamid() else {
            return self.error(seq, ErrorCode::BadRequest, "No Steam account is signed in");
        };
        if steamid != target.as_str() {
            return self.error(seq, ErrorCode::BadRequest, "That is not the signed-in Steam account");
        }
        let Some(appid) = game.as_str().parse::<u32>().ok().filter(|appid| *appid != 0) else {
            return self.error(seq, ErrorCode::BadRequest, "That is not a Steam game");
        };
        let appid = appid.to_string();
        let reply = if self.reads_with_key() {
            self.read_with_key("/ISteamUserStats/GetPlayerAchievements/v1/", &[("steamid", &steamid), ("appid", &appid)], http::SMALL_BODY)
        } else if self.key.is_some() && self.account.as_ref().is_none_or(|account| account.steamid != steamid) {
            Err(Refusal::Key)
        } else {
            self.read_with_access(
                |token| {
                    let query = http::form(&[("access_token", token), ("steamid", &steamid), ("appid", &appid), ("format", "json")]);
                    http::api(&format!("/ISteamUserStats/GetPlayerAchievements/v1/?{query}"))
                },
                http::SMALL_BODY,
            )
        };
        let resp = match reply {
            Ok(resp) => resp,
            Err(Refusal::Expired) => return self.error(seq, ErrorCode::SignIn, "Sign in again to renew Steam access"),
            // Steam would not renew access just now, or does not accept the key; the account's own state tells the owner, and the page
            // shows no progress until it is mended.
            Err(Refusal::Renewal | Refusal::ReadDenied | Refusal::Key | Refusal::Hidden) => {
                return self.answer(seq, Event::Achievements { target, game, unlocked: Vec::new() });
            }
            Err(Refusal::Trouble(_)) => return self.error(seq, ErrorCode::Network, "Steam is not reachable"),
        };
        // A refusal of a freshly made access token is Steam's answer for this game (no stats, private details), not for the sign-in.
        let unlocked = match resp.status {
            200 if denied(&resp) => Vec::new(),
            200 => match parse(&resp.body) {
                Some(unlocked) => unlocked,
                None => return self.error(seq, ErrorCode::Internal, "Steam sent an answer the TV could not read"),
            },
            400 | 401 | 403 => Vec::new(),
            _ => return self.error(seq, ErrorCode::Network, "Steam is not reachable"),
        };
        self.answer(seq, Event::Achievements { target, game, unlocked });
    }
}

#[cfg(test)]
mod tests {
    use crate::fake::{self, Rig, STEAMID, UNIX0};
    use lumagame_core::ids::{ProviderGameId, TargetId};
    use lumagame_core::ipc::{ErrorCode, Event, FromEngine, Link, Request, Unlocked};

    fn ask(rig: &mut Rig) -> Vec<FromEngine> {
        rig.send(Request::Achievements { target: TargetId::new(STEAMID).unwrap(), game: ProviderGameId::new("1145350").unwrap() })
    }

    fn rig() -> Rig {
        let rig = Rig::signed_in();
        let access = fake::jwt(STEAMID, Some(UNIX0 + 86_400));
        rig.http.on("GenerateAccessTokenForApp", [fake::ok(&format!(r#"{{"response":{{"access_token":"{access}"}}}}"#))]);
        rig
    }

    #[test]
    fn the_unlocked_ones_are_reported_and_the_locked_and_the_odd_are_left_out() {
        let mut rig = rig();
        let body = r#"{"playerstats":{"success":true,"achievements":[{"apiname":"WIN","achieved":1,"unlocktime":1700000000},{"apiname":"LOSE","achieved":0,"unlocktime":0},{"apiname":7,"achieved":1},{"apiname":"HID","achieved":1,"unlocktime":0}]}}"#;
        rig.http.on("GetPlayerAchievements", [fake::ok(body)]);
        let events = ask(&mut rig);
        let [FromEngine { event: Event::Achievements { unlocked, .. }, .. }] = &events[..] else { panic!("{events:?}") };
        assert_eq!(unlocked, &[Unlocked { name: "WIN".into(), at: Some(1_700_000_000) }, Unlocked { name: "HID".into(), at: None }]);
        let sent = &rig.http.to("GetPlayerAchievements")[0];
        assert!(sent.url.contains("appid=1145350") && sent.url.contains(&format!("steamid={STEAMID}")), "{}", sent.url);
    }

    #[test]
    fn unavailable_stats_and_network_failure_are_distinct() {
        let mut rig = rig();
        rig.http.on("GetPlayerAchievements", [fake::status(400)]);
        assert!(matches!(&ask(&mut rig)[..], [FromEngine { event: Event::Achievements { unlocked, .. }, .. }] if unlocked.is_empty()));
        rig.http.on("GetPlayerAchievements", [fake::down()]);
        assert!(matches!(&ask(&mut rig)[..], [FromEngine { event: Event::Error { code: ErrorCode::Network, .. }, .. }]));
    }

    fn is_empty(events: &[FromEngine]) -> bool {
        matches!(events, [FromEngine { event: Event::Achievements { unlocked, .. }, .. }] if unlocked.is_empty())
    }

    fn linked(rig: &mut Rig) -> bool {
        let events = rig.send(Request::Targets);
        matches!(&events[..], [FromEngine { event: Event::Targets { targets }, .. }] if targets[0].link == Link::Linked)
    }

    #[test]
    fn a_games_refusal_after_a_fresh_renewal_shows_no_achievements_and_leaves_the_sign_in_alone() {
        for refusal in [fake::status(401), fake::eresult(15, "{}")] {
            let mut rig = rig();
            rig.http.on("GetPlayerAchievements", [refusal]);
            assert!(is_empty(&ask(&mut rig)));
            assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 1);
            assert_eq!(rig.http.count("GetPlayerAchievements"), 1, "a token just made is not tried twice");
            assert!(linked(&mut rig));
        }
    }

    #[test]
    fn a_refused_old_access_token_is_renewed_once_and_the_read_tried_again() {
        let mut rig = rig();
        let body = r#"{"playerstats":{"achievements":[{"apiname":"WIN","achieved":1,"unlocktime":5}]}}"#;
        rig.http.on("GetPlayerAchievements", [fake::ok(body)]);
        ask(&mut rig);
        rig.http.on("GetPlayerAchievements", [fake::status(401), fake::ok(body)]);
        let events = ask(&mut rig);
        assert!(matches!(&events[..], [FromEngine { event: Event::Achievements { unlocked, .. }, .. }] if unlocked.len() == 1), "{events:?}");
        assert_eq!((rig.http.count("GenerateAccessTokenForApp"), rig.http.count("GetPlayerAchievements")), (2, 3));
        // The next refusal within half an hour is the game's: no further renewal.
        rig.http.on("GetPlayerAchievements", [fake::status(401)]);
        assert!(is_empty(&ask(&mut rig)));
        assert_eq!((rig.http.count("GenerateAccessTokenForApp"), rig.http.count("GetPlayerAchievements")), (2, 4));
        assert!(linked(&mut rig));
    }

    #[test]
    fn a_renewal_steam_refuses_shows_no_achievements_and_nothing_else() {
        let mut rig = Rig::signed_in();
        rig.http.on("GenerateAccessTokenForApp", [fake::eresult(15, r#"{"response":{}}"#)]);
        assert!(is_empty(&ask(&mut rig)));
        assert_eq!(rig.http.count("GetPlayerAchievements"), 0);
        assert!(linked(&mut rig));
        assert!(rig.data().join("token.json").exists());
    }
}
