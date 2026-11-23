//! Accounts and PCs, linking, the empty Home and text-entry shots.
use super::*;
use crate::game::model::settings::{Act, Pref};

pub(super) fn linking(base: &Model, add: &mut impl FnMut(&str, Model)) {
    for (name, state, engine) in [
        ("11-pairing", AuthState::ShowPin { pin: "7249".into() }, "gamestream"),
        ("12-signin", AuthState::DeviceCode { url: "https://nvidia.com/link".into(), code: "ABCD-EFGH".into(), qr: None, expires_s: 420 }, "gfn"),
        ("28-enter-pin", AuthState::EnterPin { digits: 4 }, "punktfunk"),
        ("29-approval", AuthState::Approval { hint: Some("Allow LumaGame in the Punktfunk request on Desk-PC.".into()) }, "punktfunk"),
    ] {
        let mut m = base.clone();
        m.view.page = Page::Settings;
        m.view.place = Place::Accounts;
        event(&mut m, engine, ipc::Event::Auth { state });
        add(name, m);
    }
}
pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = base.clone();
    if let Some(e) = m.integrations.iter_mut().find(|e| e.manifest.id.as_str() == "gamestream") {
        e.targets[0].reach = ipc::Reach::Offline;
    }
    m.open_settings(Place::Accounts);
    m.view.focus = Focus::Setting(0);
    add("settings-a-accounts-offline-pc", m);
    let mut m = Model::new(State::default(), Listing::default(), base.integrations.iter().map(|e| e.manifest.clone()).collect());
    m.view.focus = Focus::Home(Spot::Add);
    add("home-a-empty", m.clone());
    m.open_settings(Place::Accounts);
    m.view.focus = Focus::Setting(0);
    add("settings-a-first-run", m);
}
pub(super) fn editing(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = base.clone();
    (m.view.page, m.view.place, m.view.pane) = (Page::Settings, Place::Accounts, Some(Pane::Service));
    m.view.selected_engine = Some(EngineId::new("gamestream").unwrap());
    m.view.push(Layer::AddHost, m.view.focus);
    m.view.focus = Focus::Keyboard(0);
    add("41-add-pc", m);
    let mut m = base.clone();
    (m.view.page, m.view.place, m.view.pane) = (Page::Settings, Place::Accounts, Some(Pane::Service));
    m.view.selected_engine = Some(EngineId::new("native").unwrap());
    m.view.push(Layer::EditText, m.view.focus);
    m.view.focus = Focus::Keyboard(0);
    m.view.address = "/media/games/quake".into();
    add("42-edit-path", m);
}
/// Focus on the first row of the open sheet that `act` picks.
fn focus_row(m: &mut Model, act: impl Fn(&Act) -> bool) {
    let sheet = m.settings_sheet();
    let row = sheet.rows().into_iter().find(|&i| sheet.item(i).is_some_and(|item| act(&item.act))).expect("the row");
    m.view.focus = Focus::Setting(row);
}
/// The Steam integration's page signed out, with its sign-in sheet, signed in and asking before signing out, and the Home and game
/// pages its records feed, in both themes at 100% and 200% text.
pub(super) fn steam_screens(base: &Model) -> Vec<(String, Model)> {
    use crate::game::model::tests::{SteamShot, with_steam};
    let mut out = Vec::new();
    for (name, shot, page, title) in [
        ("60-steam-signed-out", SteamShot::SignedOut, None, ""),
        ("steam-a-accounts-page-qr", SteamShot::Qr, None, ""),
        ("steam-a-accounts-page", SteamShot::SignedIn, None, ""),
        ("steam-a-sign-out", SteamShot::SignedIn, Some(Page::Settings), ""),
        ("63-steam-home", SteamShot::SignedIn, Some(Page::Home), ""),
        ("64-steam-hub", SteamShot::SignedIn, Some(Page::Game), "Baldur's Gate 3"),
    ] {
        for (suffix, light, text) in [("", false, 100), ("-light", true, 100), ("-200", false, 200), ("-200-light", true, 200)] {
            let mut m = with_steam(base.clone(), shot);
            match page {
                Some(Page::Game) => {
                    m.view.page = Page::Game;
                    m.view.selected = Some(id(&m, title));
                    m.view.focus = Focus::Play;
                }
                Some(Page::Settings) => {
                    focus_row(&mut m, |act| *act == Act::SignOut);
                    m.view.push(Layer::SignOut, m.view.focus);
                    m.view.focus = Focus::SheetAction(0);
                }
                Some(page) => {
                    m.view.page = page;
                    m.view.focus = m.home_focus();
                }
                None if matches!(shot, SteamShot::SignedIn) => focus_row(&mut m, |act| *act == Act::Pref(Pref::GamePages)),
                // As in the mock: the code shown 48 s into a five-minute attempt, so the ring shows the time gone.
                None if matches!(shot, SteamShot::Qr) => {
                    m.step(Input::Tick { now_ms: 48_000, unix_s: 1_800_000_048 });
                    let state = AuthState::Qr { url: "https://s.team/q/1/11860205094928508090".into(), expires_s: 252 };
                    event(&mut m, "steam", ipc::Event::Auth { state });
                }
                None => {}
            }
            m.prefs.light = light;
            m.prefs.text_percent = text;
            m.rebuild_navigation();
            out.push((format!("{name}{suffix}"), m));
        }
    }
    out
}
/// The sign-out question and the sign-in sheet opened from the Steam page and closed with Back, each leaving focus where it was.
pub(super) fn walks() -> Vec<super::budget::Walk> {
    use super::budget::{Step, Walk, taps};
    use crate::game::model::tests::{SteamShot, with_steam};
    let mut signed_in = with_steam(fixture(), SteamShot::SignedIn);
    focus_row(&mut signed_in, |act| *act == Act::SignOut);
    signed_in.rebuild_navigation();
    let mut script = taps(&[(Key::Ok, 300)]);
    script.push(Step::Expect("the question with Cancel lit", |m, _| (m.view.top(), m.view.focus) == (Some(Layer::SignOut), Focus::SheetAction(0))));
    script.extend(taps(&[(Key::Right, 150), (Key::Left, 150), (Key::Back, 300)]));
    script.push(Step::Expect("back on Sign out", |m, _| m.view.top().is_none() && m.view.pane == Some(Pane::Service)));
    let mut walks = vec![Walk { name: "accounts-dialog-open-close".into(), base: signed_in, script }];
    let mut signed_out = with_steam(fixture(), SteamShot::SignedOut);
    focus_row(&mut signed_out, |act| matches!(act, Act::Link(..)));
    signed_out.rebuild_navigation();
    let mut script = taps(&[(Key::Ok, 300)]);
    script.push(Step::Expect("the sign-in sheet", |m, _| m.view.top() == Some(Layer::Auth)));
    script.extend(taps(&[(Key::Back, 300)]));
    script.push(Step::Expect("back on the Steam page", |m, _| m.view.top().is_none() && m.view.pane == Some(Pane::Service)));
    walks.push(Walk { name: "accounts-sign-in-open-close".into(), base: signed_out, script });
    walks
}

#[test]
fn native_text_fields_activate_on_release_and_validate_before_saving() {
    use lumagame_core::settings::{Field, FieldKind, Section, SettingsSchema};
    let mut m = fixture();
    let native = EngineId::new("native").unwrap();
    m.integrations.iter_mut().find(|e| e.manifest.id == native).unwrap().schema = SettingsSchema {
        sections: vec![Section {
            title: "Games".into(),
            fields: vec![Field { key: "path".into(), label: "Game folder".into(), help: None, kind: FieldKind::Text { max_chars: 3, default: String::new() } }],
        }],
    };
    (m.view.page, m.view.place, m.view.pane) = (Page::Settings, Place::Accounts, Some(Pane::Service));
    m.view.selected_engine = Some(native);
    m.view.focus = Focus::Setting(1);
    m.rebuild_navigation();
    m.step(Input::Key { key: Key::Right, down: true });
    assert_eq!(m.view.top(), None);
    m.step(Input::Key { key: Key::Ok, down: true });
    assert_eq!(m.view.top(), None);
    m.step(Input::Key { key: Key::Ok, down: false });
    assert_eq!(m.view.top(), Some(Layer::EditText));
    m.step(Input::Text("abcd".into()));
    m.view.focus = Focus::SheetAction(0);
    activate(&mut m);
    assert_eq!(m.view.top(), Some(Layer::EditText));
    assert_eq!(m.view.notice.as_deref(), Some("Use at most 3 characters."));
    m.view.address = "abc".into();
    activate(&mut m);
    assert_eq!(m.view.top(), None);
    assert_eq!(m.view.focus, Focus::Setting(1));
}
