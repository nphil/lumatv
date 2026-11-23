//! Integration pages, linking, first run and text-entry shots.
use super::*;

pub(super) fn linking(base: &Model, add: &mut impl FnMut(&str, Model)) {
    for (name, state, engine) in [
        ("11-pairing", AuthState::ShowPin { pin: "7249".into() }, "gamestream"),
        ("12-signin", AuthState::DeviceCode { url: "https://nvidia.com/link".into(), code: "ABCD-EFGH".into(), qr: None, expires_s: 420 }, "gfn"),
        ("28-enter-pin", AuthState::EnterPin { digits: 4 }, "punktfunk"),
        ("29-approval", AuthState::Approval { hint: Some("Allow LumaGame in the Punktfunk request on Desk-PC.".into()) }, "punktfunk"),
    ] {
        let mut m = base.clone();
        m.view.page = Page::Settings;
        m.view.place = Place::Integrations;
        event(&mut m, engine, ipc::Event::Auth { state });
        add(name, m);
    }
}
pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = base.clone();
    m.view.page = Page::Accounts;
    m.view.selected_engine = Some(EngineId::new("gamestream").unwrap());
    if let Some(e) = m.integrations.iter_mut().find(|e| e.manifest.id.as_str() == "gamestream") {
        e.targets[0].reach = ipc::Reach::Offline;
    }
    m.view.focus = Focus::Setting(0);
    add("21-state-offline-pc", m);
    let mut m = Model::new(State::default(), Listing::default(), base.integrations.iter().map(|e| e.manifest.clone()).collect());
    m.view.focus = Focus::FirstRun(0);
    add("22-state-first-run", m);
}
pub(super) fn editing(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = base.clone();
    m.view.page = Page::Accounts;
    m.view.selected_engine = Some(EngineId::new("gamestream").unwrap());
    m.view.push(Layer::AddHost, m.view.focus);
    m.view.focus = Focus::Keyboard(0);
    add("41-add-pc", m);
    let mut m = base.clone();
    m.view.page = Page::Accounts;
    m.view.selected_engine = Some(EngineId::new("native").unwrap());
    m.view.push(Layer::EditText, m.view.focus);
    m.view.focus = Focus::Keyboard(0);
    m.view.address = "/media/games/quake".into();
    add("42-edit-path", m);
}
/// The Steam integration's page signed out, with its QR sheet and signed in, and the Home and game pages its records feed, in both
/// themes at 100% and 200% text.
pub(super) fn steam_screens(base: &Model) -> Vec<(String, Model)> {
    use crate::game::model::tests::{SteamShot, with_steam};
    let mut out = Vec::new();
    for (name, shot, page, title) in [
        ("60-steam-signed-out", SteamShot::SignedOut, None, ""),
        ("61-steam-qr", SteamShot::Qr, None, ""),
        ("62-steam-signed-in", SteamShot::SignedIn, None, ""),
        ("63-steam-home", SteamShot::SignedIn, Some(Page::Home), ""),
        ("64-steam-hub", SteamShot::SignedIn, Some(Page::Game), "Baldur's Gate 3"),
        ("65-steam-home-favourites", SteamShot::SignedIn, Some(Page::Home), "Favourites"),
    ] {
        for (suffix, light, text) in [("", false, 100), ("-light", true, 100), ("-200", false, 200), ("-200-light", true, 200)] {
            let mut m = with_steam(base.clone(), shot);
            match page {
                Some(Page::Game) => {
                    m.view.page = Page::Game;
                    m.view.selected = Some(id(&m, title));
                    m.view.hub_tab = 0;
                    m.view.focus = Focus::HubAction(0);
                }
                Some(page) => {
                    m.view.page = page;
                    let row = m.home_rows.iter().position(|r| r.title == title).unwrap_or(0);
                    m.view.focus = Focus::Home(row, 0);
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
    m.view.page = Page::Accounts;
    m.view.selected_engine = Some(native);
    m.view.focus = Focus::EngineField(0);
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
    assert_eq!(m.view.focus, Focus::EngineField(0));
}
