//! LG's own banner on an input, which the compositor leaves undrawn (`mask.rs` in the compositor) while this one stands in for it.
//!
//! `inputcommon` draws LG's banner into the one picture of the input in front, over the video: on Live TV a bar and, for a launch and for OK, a "Guide" chip under it; on an
//! HDMI input the launch pill, a bar of the same height. While an input is in front and the owner's banners are on, this claims those places on that picture
//! (`POST /v1/mask`): the compositor blanks them for a hold after a channel key reaches Live TV, and for a launch, and gives them back the moment the claim is released or this
//! client is gone, so the stock banner returns whenever the UI is not running. Where LG puts the banner is the setting `option.bannerPosition`.

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
/// (`zappingbanner_bg_left_m.png`, alpha 230 over 156 rows), so the claim is the bar and a small margin. An input's pill is the same bar, 582 to 960 wide
/// (`inputSimpleMaxWidth`) and a little wider with OK's Options button: the same claim covers it.
const BAR_H: i32 = 156;
const BAR_W: i32 = 1500;
const BAR_TOP_LOW: i32 = 562;
const MARGIN: i32 = 6;
const SCREEN_W: i32 = 1920;
/// The "Guide" chip (`DeeplinkView.qml`): a flat grey button LG hangs under the bar for a launch and for OK, not for a channel key. On the set's
/// pictures of a launch and of the OK banner (2026-11-04) it is x 27 to 251, y 743 to 796 under the default bar (from 562): 225 by 54, 27 in from
/// the bar's left edge and 25 under its foot. It is a view of its own, outside the bar's rectangle, so the claim of the bar alone left it on the
/// picture at every launch.
const CHIP_W: i32 = 225;
const CHIP_H: i32 = 54;
const CHIP_DX: i32 = 27;
const CHIP_DY: i32 = 181;
/// CH+ and CH-: LG draws its banner when the tuner has answered either.
const KEYS: [u32; 2] = [402, 403];
/// LG's banner is up five seconds from when it is drawn (`BannerInterfaces.qml`), and it is drawn when the tuner has answered the key, or, on an input, about
/// 0.6 s after the switch (`inputcommon`'s log of 2026-11-11: the switch at 12506.02, the pill at 12506.63).
const HOLD_MS: u64 = 6500;

/// The claim for the window of `app` with LG's banner at `position` (`lefttop`, `righttop`, `leftbottom`, `rightbottom`; anything else is LG's default, at the left
/// and low). Live TV: the bar, then the chip under it, and the channel keys. The chip goes where the bar goes; at the right the bar is as wide as the channel makes it,
/// which is not known, so the chip's rectangle runs from where the widest bar would have it to the screen's edge. Another input: the bar alone, for a launch only; its
/// pill has no chip, and no key of the remote starts LG's banner there.
fn claim_body(app: &str, position: &str) -> Value {
    let top = position.ends_with("top");
    let right = position.starts_with("right");
    let (w, h) = (BAR_W + MARGIN, BAR_H + 2 * MARGIN);
    let bar_y = if top { 0 } else { BAR_TOP_LOW };
    let bar_x = if right { SCREEN_W - BAR_W } else { 0 };
    let (x, y) = ((bar_x - MARGIN).max(0), (bar_y - MARGIN).max(0));
    let bar = json!({"x": x, "y": y, "w": w, "h": h});
    if app != LIVE_TV_APP {
        return json!({"op": "claim", "app": app, "rects": [bar]});
    }
    let chip_x = bar_x + CHIP_DX - MARGIN;
    let chip_w = if right { SCREEN_W - chip_x } else { CHIP_W + 2 * MARGIN };
    let chip = json!({"x": chip_x, "y": bar_y + CHIP_DY - MARGIN, "w": chip_w, "h": CHIP_H + 2 * MARGIN});
    json!({"op": "claim", "app": app, "rects": [bar, chip], "keys": KEYS, "holdMs": HOLD_MS})
}

fn post(body: Value) -> Effect {
    Effect::Channel { path: CHANNEL_PATH.into(), body }
}

/// What the compositor has been told of LG's banner.
#[derive(Debug, Default)]
pub struct LgMask {
    /// The position the claim was made for; empty is the default.
    position: String,
    /// The input the claim is for.
    app: String,
    on: bool,
}

impl LgMask {
    pub fn on(&self) -> bool {
        self.on
    }

    /// An input came to the front: LG's banner for a launch is blanked on its window, and the position it is at is read.
    pub fn engage(&mut self, app: &str) -> Vec<Effect> {
        self.on = true;
        self.app = app.to_string();
        vec![
            post(claim_body(app, &self.position)),
            post(json!({"op": "engage", "ms": HOLD_MS})),
            Effect::Luna { tag: POSITION_TAG, uri: SETTINGS_URI.into(), payload: json!({"category": "option", "keys": ["bannerPosition"]}), subscribe: false },
        ]
    }

    /// The tuner changed channel (CH+ or CH-, a number): LG draws its own banner for it as it is tuned, so the blanking runs from now for as long
    /// as that lasts.
    pub fn tuned(&self) -> Vec<Effect> {
        if self.on && self.app == LIVE_TV_APP { vec![post(json!({"op": "engage", "ms": HOLD_MS}))] } else { Vec::new() }
    }

    /// The input left the front, the owner's banners went off, or the tuner cannot be followed: LG's banner is its own again.
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
        vec![post(claim_body(&self.app, position))]
    }

    /// The tuner could not be asked which channel it is on: nothing here would say what LG's banner says, so LG's stays. An input has no tuner: the answer of one that
    /// Live TV left behind is not its.
    pub fn tuner_answer(&mut self, tag: u64, payload: &Value) -> Vec<Effect> {
        if tag == LAST_TAG && self.app == LIVE_TV_APP && payload.get("returnValue").and_then(Value::as_bool) == Some(false) { self.release() } else { Vec::new() }
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

    fn rect_at(body: &Value, i: usize) -> (i64, i64, i64, i64) {
        let r = &body["rects"][i];
        (r["x"].as_i64().unwrap(), r["y"].as_i64().unwrap(), r["w"].as_i64().unwrap(), r["h"].as_i64().unwrap())
    }

    /// The bar's rectangle.
    fn rect(body: &Value) -> (i64, i64, i64, i64) {
        rect_at(body, 0)
    }

    fn inside(inner: (i64, i64, i64, i64), outer: (i64, i64, i64, i64)) -> bool {
        outer.0 <= inner.0 && outer.1 <= inner.1 && outer.0 + outer.2 >= inner.0 + inner.2 && outer.1 + outer.3 >= inner.1 + inner.3
    }

    #[test]
    fn live_tv_in_front_claims_lgs_banner_for_the_channel_keys_and_a_launch_and_leaving_gives_it_back() {
        let mut m = LgMask::default();
        let fx = m.engage(LIVE_TV_APP);
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
        m.engage(LIVE_TV_APP);
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
        m.engage(LIVE_TV_APP);
        assert_eq!(m.tuned(), vec![post(json!({"op": "engage", "ms": 6500}))]);
    }

    #[test]
    fn a_tuner_that_cannot_be_asked_leaves_lgs_banner_alone() {
        let mut m = LgMask::default();
        m.engage(LIVE_TV_APP);
        assert!(m.tuner_answer(LAST_TAG, &json!({"returnValue": true, "channelId": "1_1"})).is_empty() && m.on());
        assert!(m.tuner_answer(0, &json!({"returnValue": false})).is_empty() && m.on(), "another ask's failure is not the tuner's");
        let fx = m.tuner_answer(LAST_TAG, &json!({"returnValue": false, "errorCode": -106}));
        assert!(fx.contains(&post(json!({"op": "release"}))) && !m.on());
    }

    /// The geometry is LG's (`GlobalStyle.qml`), no claim leaves the screen, and the whole bar is in the first.
    #[test]
    fn every_claim_covers_the_bar_and_stays_on_the_screen() {
        for app in [LIVE_TV_APP, "com.webos.app.hdmi2"] {
            for p in ["lefttop", "righttop", "leftbottom", "rightbottom", "", "weird"] {
                let body = claim_body(app, p);
                for i in 0..body["rects"].as_array().map_or(0, Vec::len) {
                    let (x, y, w, h) = rect_at(&body, i);
                    assert!(x >= 0 && y >= 0 && x + w <= 1920 && y + h <= 1080, "{app} {p}: rectangle {i}");
                }
                let bar_y = if p.ends_with("top") { 0 } else { 562 };
                let bar_x = if p.starts_with("right") { 1920 - 1500 } else { 0 };
                assert!(inside((bar_x, bar_y, 1500, 156), rect(&body)), "{app} {p}: the bar is inside the claim");
            }
        }
    }

    /// LG's launch pill on an HDMI input is drawn into the input's own window (`com.webos.app.hdmi2`), a bar with no Guide chip under it, and no key of the remote starts it.
    #[test]
    fn an_hdmi_input_in_front_claims_the_bar_of_its_own_window_for_a_launch_alone_and_another_input_takes_the_claim_over() {
        let mut m = LgMask::default();
        let fx = m.engage("com.webos.app.hdmi2");
        let claim = claim_of(&fx);
        assert_eq!(claim["app"], "com.webos.app.hdmi2");
        assert_eq!(rect(&claim), (0, 556, 1506, 168), "the bar at LG's default place, as for Live TV");
        assert_eq!(claim["rects"].as_array().map(Vec::len), Some(1), "no Guide chip under an input's pill");
        assert!(claim["keys"].as_array().is_none_or(Vec::is_empty) && claim["holdMs"].is_null(), "no key of the remote starts LG's banner on an input: {claim}");
        assert!(fx.iter().any(|e| matches!(e, Effect::Channel { body, .. } if body["op"] == "engage" && body["ms"] == 6500)), "the launch's pill is blanked");
        assert!(fx.iter().any(|e| matches!(e, Effect::Luna { tag: POSITION_TAG, subscribe: false, .. })), "where LG puts it is read");
        // the position moves the claim of the input in front; it has no channel to change, and the failure of a tuner that Live TV left behind is not its
        let at = json!({"returnValue": true, "settings": {"bannerPosition": "righttop"}});
        let moved = claim_of(&m.position(&at));
        assert_eq!((moved["app"].as_str(), rect(&moved)), (Some("com.webos.app.hdmi2"), (414, 0, 1506, 168)));
        assert!(m.tuned().is_empty());
        assert!(m.tuner_answer(LAST_TAG, &json!({"returnValue": false, "errorCode": -106})).is_empty() && m.on());
        // another input and then Live TV: the claim goes to that window, at the place the owner put LG's banner
        let next = claim_of(&m.engage("com.webos.app.hdmi1"));
        assert_eq!((next["app"].as_str(), rect(&next)), (Some("com.webos.app.hdmi1"), (414, 0, 1506, 168)));
        let live = claim_of(&m.engage(LIVE_TV_APP));
        assert_eq!((live["app"].as_str(), live["rects"].as_array().map(Vec::len), live["keys"].clone()), (Some(LIVE_TV_APP), Some(2), json!([402, 403])));
        assert_eq!(m.tuned().len(), 1, "Live TV's channel changes run the blanking on again");
        assert!(!m.release().is_empty() && !m.on());
    }

    /// The "Guide" chip as the set drew it at a launch and under the OK banner (`G-bg-ok-banner` and `L-launch-e` of the TV session of 2026-11-04): a
    /// flat grey button at x 27 to 251, y 743 to 796 on the 1920 by 1080 picture, under a bar that starts at 562. It is outside the bar's rectangle, and
    /// the claim of the bar alone left it on the picture at every launch.
    const GUIDE_CHIP: (i64, i64, i64, i64) = (27, 743, 225, 54);

    #[test]
    fn the_guide_chip_lg_hangs_under_its_bar_is_claimed_with_it_and_goes_where_the_bar_goes() {
        // LG's default is where it was measured; the owner's setting reads `none`
        for p in ["", "none", "leftbottom", "weird"] {
            let body = claim_body(LIVE_TV_APP, p);
            assert_eq!(body["rects"].as_array().map(Vec::len), Some(2), "{p}: the bar and the chip");
            let chip = rect_at(&body, 1);
            assert!(inside(GUIDE_CHIP, chip), "{p}: the chip at {GUIDE_CHIP:?} is in its claim {chip:?}");
            assert!(chip.2 <= GUIDE_CHIP.2 + 2 * i64::from(MARGIN) && chip.3 <= GUIDE_CHIP.3 + 2 * i64::from(MARGIN), "{p}: the chip and a margin, not the band under the bar: {chip:?}");
        }
        // the other places: 181 under the bar's top, 27 in from its left edge, which at the right is anywhere along the bar
        for (p, bar_x, bar_y) in [("lefttop", 0, 0), ("righttop", 420, 0), ("rightbottom", 420, 562)] {
            let moved = (bar_x + 27, bar_y + GUIDE_CHIP.1 - 562, GUIDE_CHIP.2, GUIDE_CHIP.3);
            assert!(inside(moved, rect_at(&claim_body(LIVE_TV_APP, p), 1)), "{p}: the chip rides with the bar at {moved:?}");
        }
        // while Live TV is in front the claim that is made is this one
        let mut m = LgMask::default();
        assert!(inside(GUIDE_CHIP, rect_at(&claim_of(&m.engage(LIVE_TV_APP)), 1)));
    }
}
