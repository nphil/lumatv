//! Quick settings shots.
use super::*;

#[test]
fn choosing_a_mode_in_quick_settings_mid_game_starts_and_stops_sampling() {
    use crate::game::stats::{Mode,Overlay};
    let mut m=fixture();
    m.view.page=Page::Game;m.view.selected=Some(id(&m,"Cyberpunk 2077"));m.view.focus=Focus::HubAction(0);
    activate(&mut m);
    let session=m.session().unwrap().id;
    event(&mut m,"gfn",ipc::Event::Session {session,state:SessionState::Running {stream:None}});
    let flags=|commands:Vec<Command>| commands.into_iter().find_map(|c| match c {
        Command::Send {request:ipc::Request::Stats {on,pad_timing,..},..}=>Some((on,pad_timing)),
        _=>None,
    });
    assert_eq!(m.view.page,Page::Running);
    assert_eq!(flags(m.step(Input::StatsSetting(Overlay {mode:Mode::Full,changed:1}))),Some((true,true)),"the shared setting alone must ask the engine for stream and controller reports");
    assert_eq!(flags(m.step(Input::StatsSetting(Overlay {mode:Mode::Off,changed:2}))),Some((false,false)));
}
