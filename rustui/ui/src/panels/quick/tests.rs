use super::*;
use crate::kit::screen::Input;
use serde_json::json;

const TILES: &str = r#"{"items": [{"id": "ambient", "label": "Ambient light", "icon": "base.icons/lamp", "state": {"type": "on"}, "action": "toggle", "hint": "Follows the picture"}]}"#;
const LOCK_TILES: &str = r#"{"items": [
    {"id": "door", "label": "Front door", "icon": "base.icons/lock", "state": {"type": "unavailable", "reason": "offline"}, "action": "lock"},
    {"id": "scene", "label": "Movie scene", "icon": "base.icons/home", "state": {"type": "unknown"}, "action": "run", "target": "movie"}]}"#;

pub(crate) fn feed(env: &mut Env, topic: &str, data: Value, now: u64) {
    env.update(&Input::Helper { topic: topic.into(), data }, now);
}

/// A panel on a TV that has answered its picture settings, with two plugins' tiles.
pub(crate) fn rig(popups: bool) -> (Quick, Env) {
    let mut env = Env::new(if popups { json!({"remote": {"quickPanelEdit": true}}) } else { json!({}) });
    feed(&mut env, "up", json!(true), 0);
    feed(&mut env, "plugins", json!({"plugins": [{"id": "ambient", "state": "connected"}, {"id": "lock", "state": "connected"}]}), 0);
    feed(&mut env, "plugin.ambient.tiles", serde_json::from_str(TILES).unwrap(), 0);
    feed(&mut env, "plugin.lock.tiles", serde_json::from_str(LOCK_TILES).unwrap(), 0);
    answer_settings(&mut env);
    let mut q = Quick::new();
    q.life.show(0);
    q.open(&mut env, 0);
    (q, env)
}

/// The settings service answers every read the panel asks for, as a TV that has all of them.
fn answer_settings(env: &mut Env) {
    let mut calls = Vec::new();
    calls.extend(env.sio.read("picture", &["pictureMode", "backlight", "energySaving", "eyeComfortMode"], None, 0));
    calls.extend(env.sio.read("general", &["remotePointer"], None, 0));
    calls.extend(env.sio.read("option", &["screenOff"], None, 0));
    let mut guard = 0;
    while let Some(Effect::Luna { tag, uri, payload, .. }) = calls.pop() {
        guard += 1;
        assert!(guard < 20);
        let cat = payload["category"].as_str().unwrap_or("").to_string();
        let answer = if uri.ends_with("getSystemSettingDesc") {
            json!({"results": []})
        } else {
            match cat.as_str() {
                "picture" => json!({"category": "picture", "dimension": {"dynamicRange": "sdr"}, "settings": {"pictureMode": "cinema", "backlight": "40", "energySaving": "auto", "eyeComfortMode": "off"}}),
                "general" => json!({"category": "general", "settings": {"remotePointer": "on"}}),
                _ => json!({"category": "option", "settings": {"screenOff": "off"}}),
            }
        };
        calls.extend(env.update(&Input::Luna { tag, payload: answer }, 1).0);
    }
}

fn sdr_modes(env: &mut Env) {
    env.sio.learn_desc("picture", &json!({"results": [{"key": "pictureMode", "values": {"arrayExt": [
        {"value": "filmMaker"}, {"value": "cinema"}, {"value": "normal"}, {"value": "vivid"},
        {"value": "game"}, {"value": "sports"}, {"value": "eco"}
    ]}}]}));
}

fn dolby_modes(env: &mut Env) {
    env.sio.learn("picture", &json!({"settings": {"pictureMode": "dolbyHdrCinema"}}));
    env.sio.learn_desc("picture", &json!({"results": [{"key": "pictureMode", "values": {"arrayExt": [
        {"value": "dolbyHdrCinema"}, {"value": "hiddenMode", "visible": false},
        {"value": "dolbyHdrGame", "active": false}, {"value": "filmMaker"}
    ]}}]}));
}

#[test]
fn native_picture_modes_step_with_keys_and_pointer_and_reopen_from_readback() {
    let (mut q, mut env) = rig(true);
    dolby_modes(&mut env);
    press(&mut q, &mut env, K::Right);
    assert_eq!(env.sio.text("picture.pictureMode"), "filmMaker");
    q.click(Target::Row(0), Some(-1), &mut env, 101);
    assert_eq!(env.sio.text("picture.pictureMode"), "dolbyHdrCinema");
    env.sio.learn("picture", &json!({"settings": {"pictureMode": "filmMaker"}}));
    q.open(&mut env, 200);
    let row = &q.rows(&env)[0];
    assert_eq!(row.options[row.index.unwrap()], "Filmmaker");
}

#[test]
fn native_picture_popup_keeps_the_displayed_choice_when_the_description_changes() {
    let (mut q, mut env) = rig(true);
    dolby_modes(&mut env);
    press(&mut q, &mut env, K::Ok);
    let Some(PopupView::Pick { options, .. }) = q.view(&env, 0).popup else { panic!("picture choices") };
    assert_eq!(options.len(), 3, "hidden mode is omitted, inactive mode remains visible");
    assert!(options[0].current);
    env.sio.learn_desc("picture", &json!({"results": [{"key": "pictureMode", "values": {"arrayExt": [
        {"value": "filmMaker"}, {"value": "dolbyHdrCinema"}
    ]}}]}));
    q.click(Target::Option(2), None, &mut env, 101);
    assert_eq!(env.sio.text("picture.pictureMode"), "filmMaker");
}

#[test]
fn inactive_native_picture_mode_cannot_be_selected_in_the_popup() {
    let (mut q, mut env) = rig(true);
    dolby_modes(&mut env);
    press(&mut q, &mut env, K::Ok);
    let out = q.click(Target::Option(1), None, &mut env, 101);
    assert!(out.effects.is_empty());
    assert_eq!(env.sio.text("picture.pictureMode"), "dolbyHdrCinema");
}

#[test]
fn missing_current_mode_stays_visible_and_available_choices_remain_selectable() {
    let (mut q, mut env) = rig(true);
    env.sio.learn("picture", &json!({"settings": {"pictureMode": "unlistedNative"}, "dimension": {"pictureMode": "unlistedNative", "dynamicRange": "dolbyHdr", "input": "hdmi1"}}));
    env.sio.learn_desc("picture", &json!({"key": "pictureMode", "values": {"arrayExt": [
        {"value": "dolbyHdrCinema"}, {"value": "dolbyHdrGame", "active": false}, {"value": "filmMaker"}
    ]}}));
    q.open(&mut env, 200);
    let row = &q.rows(&env)[0];
    assert_eq!(row.value, "unlistedNative");
    assert!(row.enabled);
    press(&mut q, &mut env, K::Ok);
    q.click(Target::Option(2), None, &mut env, 201);
    assert_eq!(env.sio.text("picture.pictureMode"), "filmMaker");
}

#[test]
fn popup_skips_inactive_and_rechecks_late_native_availability() {
    let (mut q, mut env) = rig(true);
    dolby_modes(&mut env);
    press(&mut q, &mut env, K::Ok);
    let Some(PopupView::Pick { options, .. }) = q.view(&env, 0).popup else { panic!() };
    assert!(!options[1].enabled);
    press(&mut q, &mut env, K::Down);
    assert!(matches!(q.view(&env, 0).popup, Some(PopupView::Pick { index: 2, .. })));
    env.sio.learn_desc("picture", &json!({"results": [{"key": "pictureMode", "values": {"arrayExt": [
        {"value": "filmMaker", "active": false}, {"value": "dolbyHdrCinema"}
    ]}}]}));
    assert!(press(&mut q, &mut env, K::Ok).effects.is_empty());
    assert_eq!(env.sio.text("picture.pictureMode"), "dolbyHdrCinema");
}

#[test]
fn unavailable_picture_modes_keep_native_readback_without_inventing_choices() {
    let (mut q, mut env) = rig(true);
    env.sio.learn("picture", &json!({"settings": {"pictureMode": "dolbyHdrCinema"}}));
    q.open(&mut env, 200);
    let row = &q.rows(&env)[0];
    assert!(row.options.is_empty());
    assert_eq!(row.value, "dolbyHdrCinema");
    assert!(q.click(Target::Row(0), Some(1), &mut env, 201).effects.is_empty());
    assert_eq!(env.sio.text("picture.pictureMode"), "dolbyHdrCinema");
}

#[test]
fn fast_sdr_arrows_keep_working_while_picture_context_refreshes() {
    let (mut q, mut env) = rig(true);
    press(&mut q, &mut env, K::Right);
    let row = &q.rows(&env)[0];
    assert_eq!(crate::panels::quick_layout::below_text(row).as_deref(), Some("Standard"));
    press(&mut q, &mut env, K::Right);
    assert_eq!(env.sio.text("picture.pictureMode"), "vivid");
}

#[test]
fn hdr_readback_and_empty_description_never_offer_sdr_fallback() {
    let (mut q, mut env) = rig(true);
    env.sio.learn("picture", &json!({"settings": {"pictureMode": "filmMaker"}, "dimension": {"dynamicRange": "dolbyHdr"}}));
    q.open(&mut env, 200);
    assert!(q.rows(&env)[0].options.is_empty());
    assert_eq!(q.rows(&env)[0].value, "Filmmaker");
    env.sio.learn("picture", &json!({"settings": {"pictureMode": "cinema"}, "dimension": {"dynamicRange": "sdr"}}));
    env.sio.learn_desc("picture", &json!({"results": [{"key": "pictureMode", "values": {"arrayExt": []}}]}));
    q.open(&mut env, 300);
    assert!(q.rows(&env)[0].options.is_empty());
    assert_eq!(q.rows(&env)[0].value, "Cinema");
}


fn focused(q: &Quick, env: &Env) -> String {
    let rows = q.rows(env);
    match q.view(env, 0).focus {
        Some(Focus::Row(i)) => rows[i].id.clone(),
        Some(Focus::Foot) => "foot".into(),
        Some(Focus::Strip(_)) => "strip".into(),
        other => panic!("not on a row: {other:?}"),
    }
}

pub(crate) fn press(q: &mut Quick, env: &mut Env, k: K) -> Out {
    q.key(k, env, 100)
}

fn walk_down(q: &mut Quick, env: &mut Env) -> Vec<String> {
    let mut seen = vec![focused(q, env)];
    for _ in 0..12 {
        press(q, env, K::Down);
        let now = focused(q, env);
        if seen.last() == Some(&now) {
            break;
        }
        seen.push(now);
    }
    seen
}

#[test]
fn the_first_tile_takes_the_focus_once_the_settings_service_has_answered() {
    let mut env = Env::new(json!({}));
    feed(&mut env, "up", json!(true), 0);
    let mut q = Quick::new();
    q.life.show(0);
    q.open(&mut env, 0);
    // the picture mode is not focusable until the service has answered, so the focus goes to the first tile that is
    q.sync(&env);
    let rows = q.rows(&env);
    assert_eq!(rows[q.stop().unwrap()].id, "receiver", "the picture tile cannot take it yet");
    answer_settings(&mut env);
    q.sync(&env);
    assert_eq!(q.rows(&env)[q.stop().unwrap()].id, "picture", "until a key is pressed the focus goes to the first tile when it can");
    press(&mut q, &mut env, K::Down);
    answer_settings(&mut env);
    q.sync(&env);
    assert_ne!(q.rows(&env)[q.stop().unwrap()].id, "picture", "after a key the focus is the person's");
}

#[test]
fn down_visits_the_tiles_in_order_a_tile_that_is_not_available_never_takes_the_focus_and_the_last_goes_on_to_all_settings() {
    let (mut q, mut env) = rig(false);
    let seen = walk_down(&mut q, &mut env);
    assert_eq!(seen, vec!["picture", "receiver", "sleep", "wifi", "plugin.ambient.ambient", "plugin.lock.scene", "ai", "foot"]);
    assert!(!seen.contains(&"plugin.lock.door".to_string()));
    assert_eq!(q.zone(), Zone::Foot);
    // up from the button comes back to AI access
    press(&mut q, &mut env, K::Up);
    assert_eq!(focused(&q, &env), "ai");
    // OK on All settings opens Settings at its first page and closes the panel
    press(&mut q, &mut env, K::Down);
    let out = press(&mut q, &mut env, K::Ok);
    assert_eq!(out.asks, vec![Ask::Settings(String::new())]);
    assert!(out.close);
}

#[test]
fn a_plugins_tile_ok_posts_its_action_and_a_switch_flips_by_the_same_post() {
    let (mut q, mut env) = rig(false);
    walk_down(&mut q, &mut env);
    press(&mut q, &mut env, K::Up); // the foot to AI access
    press(&mut q, &mut env, K::Up); // the movie scene
    let out = press(&mut q, &mut env, K::Ok);
    assert_eq!(out.effects, vec![Effect::Post { path: "/v1/plugins/lock/actions/run".into(), body: json!({"target": "movie"}) }]);
    press(&mut q, &mut env, K::Up); // the ambient tile
    let out = press(&mut q, &mut env, K::Ok);
    assert_eq!(out.effects, vec![Effect::Post { path: "/v1/plugins/ambient/actions/toggle".into(), body: json!({}) }]);
    assert!(!out.close);
}

#[test]
fn tiles_that_come_and_go_keep_the_focus_on_the_tile_it_was_on() {
    let (mut q, mut env) = rig(false);
    walk_down(&mut q, &mut env);
    press(&mut q, &mut env, K::Up);
    assert_eq!(focused(&q, &env), "ai");
    let more = json!({"items": [
        {"id": "ambient", "label": "Ambient light", "icon": "base.icons/lamp", "state": {"type": "on"}, "action": "toggle"},
        {"id": "mood", "label": "Mood", "icon": "base.icons/light", "state": {"type": "off"}, "action": "toggle"}]});
    feed(&mut env, "plugin.ambient.tiles", more, 500);
    q.sync(&env);
    let rows = q.rows(&env);
    assert!(rows.iter().any(|r| r.id == "plugin.ambient.mood"));
    assert_eq!(focused(&q, &env), "ai", "a tile that came does not move the focus");
}

#[test]
fn left_and_right_step_a_stepper_through_its_choices_and_wrap_and_the_receiver_nudges_the_tv_volume() {
    let (mut q, mut env) = rig(false);
    sdr_modes(&mut env);
    // picture mode: cinema (1); Right goes on, Left three times wraps to the last
    let out = press(&mut q, &mut env, K::Right);
    assert_eq!(out.effects.len(), 1);
    let Effect::Luna { uri, payload, .. } = &out.effects[0] else { panic!("{:?}", out.effects) };
    assert!(uri.ends_with("/setSystemSettings"));
    assert_eq!(payload["settings"]["pictureMode"], "normal", "Standard");
    assert_eq!(q.rows(&env)[0].index, Some(2), "a write shows at once");
    press(&mut q, &mut env, K::Left);
    press(&mut q, &mut env, K::Left);
    let out = press(&mut q, &mut env, K::Left);
    let Effect::Luna { payload, .. } = &out.effects[0] else { panic!() };
    assert_eq!(payload["settings"]["pictureMode"], "eco", "it wraps");
    // the receiver tile: Right is volume up through the TV and a nudge to the helper
    press(&mut q, &mut env, K::Down);
    let out = press(&mut q, &mut env, K::Right);
    assert_eq!(
        out.effects,
        vec![
            Effect::Luna { tag: tags::VOLUME_NUDGE, uri: "luna://com.webos.service.audio/master/volumeUp".into(), payload: json!({}), subscribe: false },
            Effect::Post { path: "/v1/receiver/nudge".into(), body: json!({}) },
        ]
    );
    let out = press(&mut q, &mut env, K::Left);
    assert!(matches!(&out.effects[0], Effect::Luna { uri, .. } if uri.ends_with("/master/volumeDown")));
}

#[test]
fn ok_on_the_receiver_opens_its_page_and_the_sleep_timer_steps_through_lgs_choices() {
    let (mut q, mut env) = rig(false);
    press(&mut q, &mut env, K::Down);
    let out = press(&mut q, &mut env, K::Ok);
    assert_eq!(out.asks, vec![Ask::Settings("receiver".into())]);
    assert!(out.close);
    let (mut q, mut env) = rig(false);
    press(&mut q, &mut env, K::Down);
    press(&mut q, &mut env, K::Down);
    let out = press(&mut q, &mut env, K::Right);
    let Effect::Luna { uri, payload, .. } = &out.effects[0] else { panic!() };
    assert!(uri.ends_with("/timer/sleep/add"));
    assert_eq!(payload, &json!({"sleepTimeMinute": "10"}));
    assert_eq!(q.rows(&env)[2].detail, "Off in 10 min");
}

#[test]
fn ok_on_a_stepper_does_nothing_until_the_owner_has_the_cards_on_and_then_opens_the_choices_beside_the_panel() {
    let (mut q, mut env) = rig(false);
    assert!(press(&mut q, &mut env, K::Ok).effects.is_empty());
    assert!(!q.popup_open());
    let (mut q, mut env) = rig(true);
    sdr_modes(&mut env);
    press(&mut q, &mut env, K::Ok);
    assert!(q.popup_open());
    let v = q.view(&env, 0);
    let Some(PopupView::Pick { title, options, index, text }) = v.popup else { panic!("a pick card") };
    assert_eq!((title.as_str(), options.len(), index), ("Picture mode", 7, 1));
    assert!(options[1].current && !options[0].current, "the current choice is ticked");
    assert_eq!(options[1].icon, "image");
    assert!(text.starts_with("How the picture looks"));
    press(&mut q, &mut env, K::Down);
    press(&mut q, &mut env, K::Down);
    let out = press(&mut q, &mut env, K::Ok);
    assert!(!q.popup_open(), "OK chooses and closes");
    let Effect::Luna { payload, .. } = &out.effects[0] else { panic!() };
    assert_eq!(payload["settings"]["pictureMode"], "vivid");
    // Back closes and chooses nothing
    press(&mut q, &mut env, K::Ok);
    press(&mut q, &mut env, K::Down);
    let out = press(&mut q, &mut env, K::Back);
    assert!(!q.popup_open() && out.effects.is_empty() && !out.close);
}

#[test]
fn the_sleep_cards_choices_say_when_the_tv_would_turn_off() {
    let (mut q, mut env) = rig(true);
    press(&mut q, &mut env, K::Down);
    press(&mut q, &mut env, K::Down);
    press(&mut q, &mut env, K::Ok);
    let Some(PopupView::Pick { options, .. }) = q.view(&env, 1_761_000_000_000).popup else { panic!() };
    assert_eq!(options[0].detail, "", "Off has no time");
    assert!(options[3].detail.ends_with("AM") || options[3].detail.ends_with("PM"), "{}", options[3].detail);
    assert!(options[0].current, "the timer is off now");
}

#[test]
fn the_oled_tile_opens_a_slider_whose_keys_move_the_backlight_by_five_and_the_card_follows_the_tv() {
    let (mut q, mut env) = rig(true);
    // an arrangement that has the tile
    env.theme = json!({"remote": {"quickPanelEdit": true}, "quickPanel": {"tiles": ["oled", "picture"]}});
    q.open(&mut env, 10);
    assert_eq!(q.rows(&env)[0].id, "oled");
    assert_eq!(q.rows(&env)[0].value, "40");
    press(&mut q, &mut env, K::Ok);
    let Some(PopupView::Slider { value, from, to, step, .. }) = q.view(&env, 0).popup else { panic!("a slider card") };
    assert_eq!((value, from, to, step), (40.0, 0.0, 100.0, 5.0));
    let out = press(&mut q, &mut env, K::Right);
    let Effect::Luna { payload, .. } = &out.effects[0] else { panic!() };
    assert_eq!(payload["settings"]["backlight"], "45");
    let Some(PopupView::Slider { value, .. }) = q.view(&env, 0).popup else { panic!() };
    assert_eq!(value, 45.0);
    // at the end it does not go on
    for _ in 0..20 {
        press(&mut q, &mut env, K::Right);
    }
    let Some(PopupView::Slider { value, .. }) = q.view(&env, 0).popup else { panic!() };
    assert_eq!(value, 100.0);
    // the TV says something else (a remote's own key): the card follows once no key has moved it for 800 ms
    env.sio.write("picture.backlight", &json!(30), None, 100);
    q.tick(&mut env, 100 + 500);
    let Some(PopupView::Slider { value, .. }) = q.view(&env, 0).popup else { panic!() };
    assert_eq!(value, 100.0, "a key moved it a moment ago");
    q.tick(&mut env, 100 + 1000);
    let Some(PopupView::Slider { value, .. }) = q.view(&env, 0).popup else { panic!() };
    assert_eq!(value, 30.0);
    press(&mut q, &mut env, K::Ok);
    assert!(!q.popup_open());
}

#[test]
fn a_held_ok_on_a_tile_picks_it_up_and_up_and_down_move_it_and_ok_drops_it_and_writes_the_order() {
    let (mut q, mut env) = rig(true);
    press(&mut q, &mut env, K::Down); // the receiver
    assert!(q.hold_ready(&env));
    q.held(&env);
    assert!(q.is_editing());
    let v = q.view(&env, 0);
    let e = v.edit.as_ref().unwrap();
    assert!(e.grab && e.index == 1, "the tile that was focused is picked up");
    assert_eq!(v.title, "Edit tiles");
    assert_eq!(v.clock, "", "the glyphs say what the keys do, the clock gives way");
    assert!(!v.hints_head.is_empty());
    // down moves it a place: shown at once, not yet written
    let out = press(&mut q, &mut env, K::Down);
    assert!(out.effects.is_empty());
    let ids: Vec<String> = q.rows(&env).into_iter().map(|r| r.id).collect();
    assert_eq!(&ids[..3], ["picture", "sleep", "receiver"]);
    // OK drops it and writes the order
    let out = press(&mut q, &mut env, K::Ok);
    assert_eq!(out.effects.len(), 1);
    let Effect::Theme { patch, .. } = &out.effects[0] else { panic!() };
    assert_eq!(patch["quickPanel"]["tiles"][2], "receiver");
    assert!(!q.view(&env, 0).edit.unwrap().grab);
    // Back is done: edit mode ends and nothing more is written
    let out = press(&mut q, &mut env, K::Back);
    assert!(!q.is_editing() && out.effects.is_empty());
}

#[test]
fn back_while_a_tile_is_picked_up_puts_it_back_where_it_was_and_keeps_the_focus() {
    let (mut q, mut env) = rig(true);
    press(&mut q, &mut env, K::Down);
    q.held(&env);
    press(&mut q, &mut env, K::Down);
    press(&mut q, &mut env, K::Down);
    let out = press(&mut q, &mut env, K::Back);
    let ids: Vec<String> = q.rows(&env).into_iter().map(|r| r.id).collect();
    assert_eq!(&ids[..3], ["picture", "receiver", "sleep"]);
    assert!(q.is_editing(), "Back put it back; a second Back is done");
    let v = q.view(&env, 0);
    assert_eq!(v.edit.unwrap().index, 1, "the tile keeps the focus");
    assert_eq!(out.effects.len(), 1, "the order is written as it was put back");
}

#[test]
fn the_remove_button_takes_a_tile_off_and_add_a_tile_puts_one_back_at_the_bottom() {
    let (mut q, mut env) = rig(true);
    press(&mut q, &mut env, K::Down);
    q.held(&env);
    press(&mut q, &mut env, K::Ok); // dropped where it was
    press(&mut q, &mut env, K::Right); // the remove button of the receiver's row
    assert!(q.view(&env, 0).edit.unwrap().on_remove);
    let out = press(&mut q, &mut env, K::Ok);
    let Effect::Theme { patch, .. } = &out.effects[0] else { panic!("{:?}", out.effects) };
    let tiles: Vec<String> = serde_json::from_value(patch["quickPanel"]["tiles"].clone()).unwrap();
    assert!(!tiles.contains(&"receiver".to_string()));
    assert_eq!(q.view(&env, 0).edit.unwrap().index, 1, "the next tile has the focus");
    // down to "Add a tile": the tiles the panel does not show
    for _ in 0..10 {
        press(&mut q, &mut env, K::Down);
    }
    assert_eq!(q.view(&env, 0).focus, Some(Focus::EditAdd));
    press(&mut q, &mut env, K::Ok);
    let Some(PopupView::Pick { title, options, text, .. }) = q.view(&env, 0).popup else { panic!("the add list") };
    assert_eq!(title, "Add a tile");
    assert_eq!(options[0].label, "Receiver", "the removed tile is the first offered");
    assert_eq!(text, "Tiles not on the panel.");
    let n = q.rows(&env).len();
    let out = press(&mut q, &mut env, K::Ok);
    assert!(!q.popup_open());
    let Effect::Theme { patch, .. } = &out.effects[0] else { panic!() };
    let tiles: Vec<String> = serde_json::from_value(patch["quickPanel"]["tiles"].clone()).unwrap();
    assert_eq!(tiles.len(), n + 1);
    assert_eq!(tiles.last().map(String::as_str), Some("receiver"), "OK adds it at the bottom");
    assert_eq!(q.view(&env, 0).edit.unwrap().index, n, "and the focus is on it");
}

#[test]
fn leaving_the_panel_while_an_edit_is_only_on_the_screen_writes_it() {
    let (mut q, mut env) = rig(true);
    press(&mut q, &mut env, K::Down);
    q.held(&env);
    press(&mut q, &mut env, K::Down); // moved, not dropped
    let out = q.close(&mut env, 500);
    assert_eq!(out.effects.len(), 1, "an arrangement that was only on the screen is written when the panel goes");
    assert!(matches!(&out.effects[0], Effect::Theme { .. }));
    assert!(!q.is_editing());
}

#[test]
fn a_held_ok_means_nothing_where_the_editor_is_off() {
    let (mut q, env) = rig(false);
    assert!(!q.hold_ready(&env));
    q.held(&env);
    assert!(!q.is_editing());
}

pub(crate) fn two_people(active: &str, locked: &[&str]) -> Value {
    let p = |id: &str, name: &str| {
        json!({"id": id, "name": name, "initial": &name[..1], "color": "#A79BF5", "avatar": {"kind": "initial"}, "avatarFile": null, "owner": id == "nitin", "active": id == active, "locked": locked.contains(&id), "firstRun": false})
    };
    json!({"ready": true, "active": active, "owner": "nitin", "canAdmin": active == "nitin", "profiles": [p("nitin", "Nitin"), p("isabel", "Isabel")]})
}

#[test]
fn the_profiles_strip_is_the_first_row_one_ok_switches_and_the_current_one_is_marked() {
    let (mut q, mut env) = rig(false);
    feed(&mut env, "profiles", two_people("nitin", &[]), 10);
    assert!(q.view(&env, 0).strip.is_some());
    // Up from the first tile reaches the strip, on the current person
    press(&mut q, &mut env, K::Up);
    assert_eq!(q.view(&env, 0).focus, Some(Focus::Strip(0)));
    press(&mut q, &mut env, K::Right);
    assert_eq!(q.view(&env, 0).focus, Some(Focus::Strip(1)));
    let out = press(&mut q, &mut env, K::Ok);
    assert_eq!(out.effects, vec![Effect::Request { tag: tags::PROFILE_SWITCH, method: "POST".into(), path: "/v1/profiles/switch".into(), body: Some(json!({"id": "isabel"})) }]);
    // the current person is already: OK on them does nothing
    press(&mut q, &mut env, K::Left);
    assert!(press(&mut q, &mut env, K::Ok).effects.is_empty());
    // the answer closes the panel
    let out = q.answer(&Input::Fetched { tag: tags::PROFILE_SWITCH, status: 200, data: json!({"ok": true}) }, &mut env, 200);
    assert!(out.close);
    // Down goes back to the list
    press(&mut q, &mut env, K::Down);
    assert!(matches!(q.view(&env, 0).focus, Some(Focus::Row(_))));
}

#[test]
fn a_profile_that_has_a_pin_asks_it_in_place_unless_the_owner_is_the_one_asking() {
    let (mut q, mut env) = rig(false);
    feed(&mut env, "profiles", two_people("isabel", &["nitin"]), 10);
    press(&mut q, &mut env, K::Up);
    assert_eq!(q.view(&env, 0).focus, Some(Focus::Strip(1)), "it opens on the current person");
    press(&mut q, &mut env, K::Left);
    let out = press(&mut q, &mut env, K::Ok);
    assert!(out.effects.is_empty() && q.pin().is_some(), "Nitin's profile has a PIN and Isabel is active: the pad is asked");
    let v = q.view(&env, 0);
    assert!(v.strip.is_none(), "the pad takes the place of the strip and the list");
    assert_eq!(v.pad.as_ref().map(|p| p.pin.name.as_str()), Some("Nitin"));
    // the digits fill the dots; the fourth submits with the PIN
    for d in 1..=3u8 {
        press(&mut q, &mut env, K::Digit(d));
    }
    let out = press(&mut q, &mut env, K::Digit(4));
    assert_eq!(out.effects, vec![Effect::Request { tag: tags::PROFILE_SWITCH, method: "POST".into(), path: "/v1/profiles/switch".into(), body: Some(json!({"id": "nitin", "pin": "1234"})) }]);
    // a wrong PIN shakes and clears; the wait shows
    q.answer(&Input::Fetched { tag: tags::PROFILE_SWITCH, status: 403, data: json!({"error": {"code": "wrong_pin", "triesLeft": 4}}) }, &mut env, 300);
    let p = q.view(&env, 300).pad.unwrap().pin;
    assert_eq!((p.filled, p.shake, p.note.as_str()), (0, 1, "Wrong PIN, 4 tries left"));
    // Back is the way out of the pad and keeps the panel
    let out = press(&mut q, &mut env, K::Back);
    assert!(!out.close && q.pin().is_none());
    // a right PIN switches and closes the panel
    press(&mut q, &mut env, K::Ok);
    for d in [1, 2, 3, 4] {
        press(&mut q, &mut env, K::Digit(d));
    }
    let out = q.answer(&Input::Fetched { tag: tags::PROFILE_SWITCH, status: 200, data: json!({"ok": true}) }, &mut env, 500);
    assert!(out.close && q.pin().is_none());
}

#[test]
fn the_owner_is_never_asked_for_a_pin_and_one_profile_has_no_strip() {
    let (mut q, mut env) = rig(false);
    feed(&mut env, "profiles", two_people("nitin", &["isabel"]), 10);
    press(&mut q, &mut env, K::Up);
    press(&mut q, &mut env, K::Right);
    let out = press(&mut q, &mut env, K::Ok);
    assert!(q.pin().is_none(), "the owner is active: Isabel's PIN is not asked");
    assert_eq!(out.effects.len(), 1);
    let (q, mut env) = rig(false);
    feed(&mut env, "profiles", json!({"ready": true, "active": "nitin", "owner": "nitin", "profiles": [{"id": "nitin", "name": "Nitin", "active": true}]}), 10);
    assert!(q.view(&env, 0).strip.is_none());
}

#[test]
fn the_readings_are_asked_for_only_once_the_slide_is_over() {
    let (mut q, mut env) = rig(false);
    // opened at 0: the slide is 110 ms, the readings come 200 ms after it
    assert!(q.tick(&mut env, 309).effects.is_empty());
    let out = q.tick(&mut env, 310);
    let uris: Vec<String> = out.effects.iter().filter_map(|e| if let Effect::Luna { uri, .. } = e { Some(uri.clone()) } else { None }).collect();
    assert!(uris.iter().any(|u| u.ends_with("/getSystemSettings")), "{uris:?}");
    assert!(uris.iter().any(|u| u.ends_with("/getAllInputStatus")), "the inputs are read");
    assert!(uris.iter().any(|u| u.ends_with("/timer/sleep/getRemainTime")), "the sleep timer is read");
    assert!(out.effects.iter().any(|e| matches!(e, Effect::Fetch { path, .. } if path == "/v1/capture")));
    assert!(q.tick(&mut env, 400).effects.is_empty(), "once");
}

#[test]
fn the_screenshot_waits_for_the_panel_to_slide_away_and_a_recording_starts_as_it_closes() {
    let (mut q, mut env) = rig(true);
    env.theme = json!({"remote": {"quickPanelEdit": true}, "quickPanel": {"tiles": ["shot", "record"]}});
    env.update(&Input::Fetched { tag: tags::CAPTURE_READ, status: 200, data: json!({"capture": {"enabled": true, "available": true, "encoder": {"available": true}}}) }, 5);
    q.open(&mut env, 10);
    let out = press(&mut q, &mut env, K::Ok);
    assert!(out.close && out.effects.is_empty(), "the panel closes first");
    assert!(q.due_at().is_some());
    q.tick(&mut env, 400); // the readings of the opening
    assert!(q.tick(&mut env, 100 + 899).effects.is_empty());
    let out = q.tick(&mut env, 100 + 900);
    assert!(out.effects.iter().any(|e| matches!(e, Effect::Request { path, .. } if path == "/v1/capture/screenshot")));
    let (mut q, mut env) = rig(true);
    env.theme = json!({"remote": {"quickPanelEdit": true}, "quickPanel": {"tiles": ["record"]}});
    env.update(&Input::Fetched { tag: tags::CAPTURE_READ, status: 200, data: json!({"capture": {"enabled": true, "available": true, "encoder": {"available": true}}}) }, 5);
    q.open(&mut env, 10);
    let out = press(&mut q, &mut env, K::Ok);
    assert!(out.close);
    assert!(matches!(&out.effects[0], Effect::Request { path, .. } if path == "/v1/capture/record/start"));
}

#[test]
fn the_panels_ids_name_what_the_pointer_lands_on() {
    for t in [Target::Strip(3), Target::Row(7), Target::Foot, Target::Option(2), Target::EditRow(4), Target::EditMove(5, -1), Target::EditMove(5, 1), Target::EditRemove(6), Target::EditAdd, Target::Pad(11), Target::Backdrop] {
        assert_eq!(Target::from_id(t.id()), Some(t), "{t:?}");
    }
    assert_eq!(Target::from_id(Id(5)), None);
}

#[test]
fn a_click_focuses_and_acts_and_a_click_on_a_steppers_chevron_steps_it() {
    let (mut q, mut env) = rig(false);
    // a click on the wifi tile (row 3) opens its page
    let out = q.click(Target::Row(3), None, &mut env, 100);
    assert_eq!(out.asks, vec![Ask::Settings("network".into())]);
    assert!(out.close);
    let (mut q, mut env) = rig(false);
    // a click on the right chevron of the picture mode steps it
    let out = q.click(Target::Row(0), Some(1), &mut env, 100);
    let Effect::Luna { payload, .. } = &out.effects[0] else { panic!() };
    assert_eq!(payload["settings"]["pictureMode"], "normal");
    // a click on a tile that cannot take the focus does nothing
    let out = q.click(Target::Row(99), None, &mut env, 100);
    assert!(out.effects.is_empty() && !out.close);
}

#[test]
fn the_view_says_where_the_panel_sits_and_what_the_head_says() {
    let (q, mut env) = rig(false);
    env.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "netflix", "home": false}) }, 1);
    let v = q.view(&env, 1_761_000_000_000);
    assert!(v.up && v.over_video && !v.editing);
    assert_eq!(v.title, "Quick settings");
    assert!(v.clock.contains(':'));
    assert!(v.hints_foot.is_empty(), "the hint row is the editor's, and it is off");
    let (q, env) = rig(true);
    let v = q.view(&env, 0);
    assert_eq!(v.hints_foot, vec![Hint::hold(Key::Ok, Action::Edit), Hint::key(Key::Back, Action::Close)]);
}

#[test]
fn video_reading_is_joined_to_the_visible_source_and_expires() {
    let (mut q, mut env) = rig(false);
    let foreground = |context: &str, app: &str| Input::Topic { topic: "foreground".into(), data: json!({"app":app,"home":false,"video":{"context":context,"app":app}}) };
    env.update(&foreground("one", "player"), 1);
    q.sync(&env);
    let due = q.info_due.unwrap();
    let request = q.tick(&mut env, due);
    let tag = request.effects.iter().find_map(|e| match e {
        Effect::Luna { tag, uri, .. } if uri.ends_with("videooutput/getStatus") => Some(*tag), _ => None,
    }).unwrap();
    let answer = Input::Luna { tag, payload: json!({"video":[{"sink":"MAIN","connected":true,"context":"one","appId":"player","width":1920,"height":1080,"frameRate":24}]}) };
    q.answer(&answer, &mut env, due + 1);
    assert_eq!(q.view(&env, due + 1).video_info, Some(VideoInfo { width: Some(1920), height: Some(1080), fps: Some(24.0) }));
    env.update(&foreground("two", "other"), due + 2);
    q.sync(&env);
    q.answer(&answer, &mut env, due + 3);
    assert!(q.view(&env, due + 3).video_info.is_none());
    let request = q.tick(&mut env, due + 1000);
    let fresh_tag = request.effects.iter().find_map(|e| match e { Effect::Luna { tag, uri, .. } if uri.ends_with("videooutput/getStatus") => Some(*tag), _ => None }).unwrap();
    q.answer(&Input::Luna { tag: fresh_tag, payload: json!({"video":[{"sink":"MAIN","connected":true,"context":"two","appId":"other","width":1280,"height":720,"frameRate":30}]}) }, &mut env, due + 1001);
    assert_eq!(q.view(&env, due + 1001).video_info, Some(VideoInfo { width: Some(1280), height: Some(720), fps: Some(30.0) }));
    assert!(q.view(&env, due + 4002).video_info.is_none());
    q.close(&mut env, due + 4003);
    q.open(&mut env, due + 4004);
    q.answer(&answer, &mut env, due + 4005);
    assert!(q.view(&env, due + 4005).video_info.is_none());
}

#[test]
fn disconnected_authority_and_mismatched_status_never_supply_video() {
    let (mut q, mut env) = rig(false);
    let due = q.info_due.unwrap();
    assert!(!q.tick(&mut env, due).effects.iter().any(|e| matches!(e, Effect::Luna { uri, .. } if uri.ends_with("videooutput/getStatus"))));
    env.update(&Input::Topic { topic: "foreground".into(), data: json!({"app":"player","video":{"context":"one","app":"player"}}) }, due + 1);
    q.sync(&env);
    let request = q.tick(&mut env, due + 1000);
    let tag = request.effects.iter().find_map(|e| match e { Effect::Luna { tag, uri, .. } if uri.ends_with("videooutput/getStatus") => Some(*tag), _ => None }).unwrap();
    q.answer(&Input::Luna { tag, payload: json!({"video":[{"sink":"MAIN","connected":true,"context":"one","appId":"other","width":3840,"height":2160}]}) }, &mut env, due + 1001);
    assert!(q.view(&env, due + 1001).video_info.is_none());
    env.update(&Input::Topic { topic: "channel.up".into(), data: json!(false) }, due + 1002);
    q.sync(&env);
    q.answer(&Input::Luna { tag, payload: json!({"video":[{"sink":"MAIN","connected":true,"context":"one","appId":"player","width":3840,"height":2160}]}) }, &mut env, due + 1003);
    assert!(q.view(&env, due + 1003).video_info.is_none());
}
