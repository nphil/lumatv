//! LG's own channel banner on Live TV, which the compositor leaves undrawn (`mask.rs` in the compositor) while this one stands in for it.
//!
//! `inputcommon` draws LG's banner into Live TV's one picture, over the video. While Live TV is in front and the owner's banners are on, this
//! claims the banner's place on that picture (`POST /v1/mask`): the compositor blanks it for a hold after a channel key reaches Live TV, and for
//! a launch, and gives it back the moment the claim is released or this client is gone, so the stock banner returns whenever the UI is not
//! running. Where LG puts the banner is the setting `option.bannerPosition`.

use super::channel_feed::LAST_TAG;
use crate::host::channels::LIVE_TV_APP;
use crate::kit::screen::Effect;
use serde_json::{Value, json};

/// The one-shot read of where LG puts its banner.
pub const POSITION_TAG: u64 = 0x4A00_0020;
const SETTINGS_URI: &str = "luna://com.webos.service.settings/getSystemSettings";
const CHANNEL_PATH: &str = "/v1/mask";

/// LG's banner at 1920 x 1080 (`GlobalStyle.qml`): a bar 156 high and as wide as 960 for a tuner channel and 1500 for an internet one, flush to
/// the left or the right edge, at the top (`tZappingBannerTopMargin` 0) or from 562. Its background is a flat 90 % bar with no shadow
/// (`zappingbanner_bg_left_m.png`, alpha 230 over 156 rows), so the claim is the bar and a small margin.
const BAR_H: i32 = 156;
const BAR_W: i32 = 1500;
const BAR_TOP_LOW: i32 = 562;
const MARGIN: i32 = 6;
const SCREEN_W: i32 = 1920;
/// CH+ and CH-: LG draws its banner when the tuner has answered either.
const KEYS: [u32; 2] = [402, 403];
/// LG's banner is up five seconds from when it is drawn (`BannerInterfaces.qml`), and it is drawn when the tuner has answered the key.
const HOLD_MS: u64 = 6500;

/// The claim for `position` (`lefttop`, `righttop`, `leftbottom`, `rightbottom`; anything else is LG's default, at the left and low).
fn claim_body(position: &str) -> Value {
    let top = position.ends_with("top");
    let right = position.starts_with("right");
    let (w, h) = (BAR_W + MARGIN, BAR_H + 2 * MARGIN);
    let y = if top { 0 } else { BAR_TOP_LOW - MARGIN };
    let x = if right { SCREEN_W - w } else { 0 };
    json!({"op": "claim", "app": LIVE_TV_APP, "rects": [{"x": x, "y": y, "w": w, "h": h}], "keys": KEYS, "holdMs": HOLD_MS})
}

fn post(body: Value) -> Effect {
    Effect::Channel { path: CHANNEL_PATH.into(), body }
}

/// What the compositor has been told of LG's banner.
#[derive(Debug, Default)]
pub struct LgMask {
    /// The position the claim was made for; empty is the default.
    position: String,
    on: bool,
}

impl LgMask {
    pub fn on(&self) -> bool {
        self.on
    }

    /// Live TV came to the front: LG's banner for a launch is blanked, and the position it is at is read.
    pub fn engage(&mut self) -> Vec<Effect> {
        self.on = true;
        vec![
            post(claim_body(&self.position)),
            post(json!({"op": "engage", "ms": HOLD_MS})),
            Effect::Luna { tag: POSITION_TAG, uri: SETTINGS_URI.into(), payload: json!({"category": "option", "keys": ["bannerPosition"]}), subscribe: false },
        ]
    }

    /// The tuner changed channel (CH+ or CH-, a number): LG draws its own banner for it as it is tuned, so the blanking runs from now for as long
    /// as that lasts.
    pub fn tuned(&self) -> Vec<Effect> {
        if self.on { vec![post(json!({"op": "engage", "ms": HOLD_MS}))] } else { Vec::new() }
    }

    /// Live TV left the front, the owner's banners went off, or the tuner cannot be followed: LG's banner is its own again.
    pub fn release(&mut self) -> Vec<Effect> {
        if !std::mem::take(&mut self.on) {
            return Vec::new();
        }
        vec![Effect::Cancel { tag: POSITION_TAG }, post(json!({"op": "release"}))]
    }

    /// The answer to the position read: a claim on the other place when LG's banner is not at the one assumed.
    pub fn position(&mut self, payload: &Value) -> Vec<Effect> {
        let Some(position) = payload.pointer("/settings/bannerPosition").and_then(Value::as_str) else { return Vec::new() };
        if !self.on || position == self.position {
            return Vec::new();
        }
        self.position = position.to_string();
        vec![post(claim_body(position))]
    }

    /// The tuner could not be asked which channel it is on: nothing here would say what LG's banner says, so LG's stays.
    pub fn tuner_answer(&mut self, tag: u64, payload: &Value) -> Vec<Effect> {
        if tag == LAST_TAG && payload.get("returnValue").and_then(Value::as_bool) == Some(false) { self.release() } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim_of(fx: &[Effect]) -> Value {
        let Some(Effect::Channel { path, body }) = fx.iter().find(|e| matches!(e, Effect::Channel { body, .. } if body["op"] == "claim")) else { panic!("a claim in {fx:?}") };
        assert_eq!(path, "/v1/mask");
        body.clone()
    }

    fn rect(body: &Value) -> (i64, i64, i64, i64) {
        let r = &body["rects"][0];
        (r["x"].as_i64().unwrap(), r["y"].as_i64().unwrap(), r["w"].as_i64().unwrap(), r["h"].as_i64().unwrap())
    }

    #[test]
    fn live_tv_in_front_claims_lgs_banner_for_the_channel_keys_and_a_launch_and_leaving_gives_it_back() {
        let mut m = LgMask::default();
        let fx = m.engage();
        let claim = claim_of(&fx);
        assert_eq!((claim["app"].as_str(), claim["keys"].clone(), claim["holdMs"].as_u64()), (Some("com.webos.app.livetv"), json!([402, 403]), Some(6500)));
        assert_eq!(rect(&claim), (0, 556, 1506, 168), "LG's default: the left, from 562, with a margin");
        assert!(fx.iter().any(|e| matches!(e, Effect::Channel { body, .. } if body["op"] == "engage" && body["ms"] == 6500)), "a launch's banner too");
        assert!(fx.iter().any(|e| matches!(e, Effect::Luna { tag: POSITION_TAG, subscribe: false, .. })), "where LG puts it is read");
        assert!(m.on());
        assert_eq!(m.release(), vec![Effect::Cancel { tag: POSITION_TAG }, post(json!({"op": "release"}))]);
        assert!(!m.on() && m.release().is_empty(), "once");
    }

    #[test]
    fn the_claim_follows_the_position_the_owner_set_and_only_while_it_is_on() {
        let mut m = LgMask::default();
        let at = |p: &str| json!({"returnValue": true, "category": "option", "settings": {"bannerPosition": p}});
        assert!(m.position(&at("righttop")).is_empty(), "nothing is claimed while Live TV is not in front");
        m.engage();
        for (p, want) in [("lefttop", (0, 0, 1506, 168)), ("righttop", (414, 0, 1506, 168)), ("leftbottom", (0, 556, 1506, 168)), ("rightbottom", (414, 556, 1506, 168))] {
            let fx = m.position(&at(p));
            assert_eq!(fx.len(), 1, "one claim per change: {fx:?}");
            assert_eq!(rect(&claim_of(&fx)), want, "{p}");
        }
        assert!(m.position(&at("rightbottom")).is_empty(), "no change, no claim");
        assert!(m.position(&json!({"returnValue": false})).is_empty());
        let fx = m.position(&at("none"));
        assert_eq!(rect(&claim_of(&fx)), (0, 556, 1506, 168), "LG's `none` is the default");
    }

    #[test]
    fn a_channel_change_runs_the_blanking_on_and_only_while_it_is_claimed() {
        let mut m = LgMask::default();
        assert!(m.tuned().is_empty());
        m.engage();
        assert_eq!(m.tuned(), vec![post(json!({"op": "engage", "ms": 6500}))]);
    }

    #[test]
    fn a_tuner_that_cannot_be_asked_leaves_lgs_banner_alone() {
        let mut m = LgMask::default();
        m.engage();
        assert!(m.tuner_answer(LAST_TAG, &json!({"returnValue": true, "channelId": "1_1"})).is_empty() && m.on());
        assert!(m.tuner_answer(0, &json!({"returnValue": false})).is_empty() && m.on(), "another ask's failure is not the tuner's");
        let fx = m.tuner_answer(LAST_TAG, &json!({"returnValue": false, "errorCode": -106}));
        assert!(fx.contains(&post(json!({"op": "release"}))) && !m.on());
    }

    /// The geometry is LG's (`GlobalStyle.qml`), the claim never leaves the screen, and the whole bar is in it.
    #[test]
    fn every_claim_covers_the_bar_and_stays_on_the_screen() {
        for p in ["lefttop", "righttop", "leftbottom", "rightbottom", "", "weird"] {
            let (x, y, w, h) = rect(&claim_body(p));
            assert!(x >= 0 && y >= 0 && x + w <= 1920 && y + h <= 1080, "{p}");
            let bar_y = if p.ends_with("top") { 0 } else { 562 };
            let bar_x = if p.starts_with("right") { 1920 - 1500 } else { 0 };
            assert!(x <= bar_x && y <= bar_y && x + w >= bar_x + 1500 && y + h >= bar_y + 156, "{p}: the bar is inside the claim");
        }
    }
}
