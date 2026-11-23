//! Game page and chooser shots.
use super::*;

pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    for (name, tab, light) in
        [("08-hub", 0, false), ("08-hub-light", 0, true), ("08-hub-ways", 1, false), ("08-hub-media", 2, false), ("08-hub-media-light", 2, true)]
    {
        let mut m = base.clone();
        m.view.page = Page::Hub;
        m.view.hub_tab = tab;
        m.prefs.light = light;
        m.view.focus = if tab == 1 {
            Focus::Way(0)
        } else if tab == 2 {
            Focus::Media(0)
        } else {
            Focus::HubAction(0)
        };
        add(name, m);
    }
    let mut m = base.clone();
    m.view.page = Page::Hub;
    m.view.selected = Some(id(&m, "Quake"));
    m.view.sheet = Some(Layer::Chooser);
    m.view.chooser_selected = m.selected_game().unwrap().sources.first().map(|s| s.key());
    m.view.focus = Focus::SheetRow(1);
    add("09-chooser", m.clone());
    m.prefs.text_percent = 200;
    add("32-chooser-text200", m);
}
pub(super) fn failed(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = base.clone();
    m.view.page = Page::Hub;
    m.view.notice = Some("The connection was interrupted. Check the network, then choose Play to try again.".into());
    add("20-state-failed", m);
}

#[test]
fn in_the_light_theme_the_hubs_status_line_and_tabs_keep_clear_of_the_hero_arts_fade() {
    use ui::lucent::palette::Mode;
    let models=super::accounts::steam_screens(&fixture());
    let hubs=models.iter().filter(|(name,_)| name.starts_with("64-steam-hub") && name.ends_with("light")).collect::<Vec<_>>();
    assert_eq!(hubs.len(),2,"both text sizes");
    for (name,m) in hubs {
        let plan=crate::game::screens::plan_in(m,Mode::Light);
        for id in [150,200,201,203,204,206,207] {
            let Some(node)=plan.nodes.iter().find(|n| n.id==id) else {continue};
            assert!(node.frame.y+node.frame.h<=740.0 || node.frame.y>=800.0,"{name}: {id} sits across the hero's fade: {:?}",node.frame);
        }
        assert!(plan.nodes.iter().any(|n| n.id==201),"{name}: the tabs are drawn");
    }
}

#[test]
fn large_text_stacks_actions_and_down_follows_the_visible_column() {
    let mut m = fixture();
    m.prefs.text_percent = 200;
    for (page, first, second) in [
        (Page::Home, Focus::Hero(0), Focus::Hero(1)),
        (Page::Hub, Focus::HubAction(0), Focus::HubAction(1)),
    ] {
        m.view.page = page;
        m.view.focus = first;
        m.rebuild_navigation();
        m.step(Input::Key { key: Key::Down, down: true });
        assert_eq!(m.view.focus, second);
    }
}
