//! The owner's own achievements in one game, for the game's page. The game's list of achievements, their names and how rare each is
//! come from public sources the app asks itself; this is only which of them the signed-in account has unlocked.

use crate::auth::Refusal;
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
        let Some(account) = &self.account else {
            return self.error(seq, ErrorCode::BadRequest, "No Steam account is signed in");
        };
        if account.steamid != target.as_str() {
            return self.error(seq, ErrorCode::BadRequest, "That is not the signed-in Steam account");
        }
        let steamid = account.steamid.clone();
        let Some(appid) = game.as_str().parse::<u32>().ok().filter(|appid| *appid != 0) else {
            return self.error(seq, ErrorCode::BadRequest, "That is not a Steam game");
        };
        let token = match self.access_token() {
            Ok(token) => token,
            Err(Refusal::Expired) => return self.error(seq, ErrorCode::SignIn, "The Steam sign-in has expired"),
            Err(Refusal::Trouble(_)) => return self.error(seq, ErrorCode::Network, "Steam is not reachable"),
        };
        let query = http::form(&[("access_token", &token), ("steamid", &steamid), ("appid", &appid.to_string()), ("format", "json")]);
        let reply = self.http.get(&http::api(&format!("/ISteamUserStats/GetPlayerAchievements/v1/?{query}")), http::SMALL_BODY);
        let resp = match reply {
            Ok(resp) => resp,
            Err(_) => return self.error(seq, ErrorCode::Network, "Steam is not reachable"),
        };
        if resp.status == 401 || (resp.status == 200 && resp.eresult == Some(15)) {
            self.expire();
            return self.error(seq, ErrorCode::SignIn, "The Steam sign-in has expired");
        }
        let unlocked = match resp.status {
            200 => match parse(&resp.body) {
                Some(unlocked) => unlocked,
                None => return self.error(seq, ErrorCode::Internal, "Steam sent an answer the TV could not read"),
            },
            400 | 403 => Vec::new(),
            _ => return self.error(seq, ErrorCode::Network, "Steam is not reachable"),
        };
        self.answer(seq, Event::Achievements { target, game, unlocked });
    }
}

#[cfg(test)]
mod tests {
    use crate::fake::{self, Rig, STEAMID, UNIX0};
    use lumagame_core::ids::{ProviderGameId, TargetId};
    use lumagame_core::ipc::{ErrorCode, Event, FromEngine, Request, Unlocked};

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
    fn no_stats_is_an_empty_list_a_refused_token_is_a_sign_in_error_and_no_connection_is_a_network_one() {
        let mut rig = rig();
        rig.http.on("GetPlayerAchievements", [fake::status(400)]);
        assert!(matches!(&ask(&mut rig)[..], [FromEngine { event: Event::Achievements { unlocked, .. }, .. }] if unlocked.is_empty()));
        rig.http.on("GetPlayerAchievements", [fake::down()]);
        assert!(matches!(&ask(&mut rig)[..], [FromEngine { event: Event::Error { code: ErrorCode::Network, .. }, .. }]));
        rig.http.on("GetPlayerAchievements", [fake::status(401)]);
        let events = ask(&mut rig);
        assert!(events.iter().any(|e| matches!(e.event, Event::Error { code: ErrorCode::SignIn, .. })), "{events:?}");
    }
}
