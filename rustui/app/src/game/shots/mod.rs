//! Explicit `GAME_SHOTS=all` visual acceptance run. Illustrative catalog/history mirrors the HTML mocks, never production data.
mod accounts;
mod budget;
mod fixtures;
mod foundation;
mod game;
mod home;
mod launch;
mod library;
mod media;
mod quick;
mod settings;

use super::{model::*, screens::GameUi, uplink::{Medium, Uplink}};
use super::model::{home::Spot, settings::{Pane, Place}};
use super::helper::{GameSirState, LeadjoyState, gamesir_fixture, leadjoy_fixture};
use compositor_protocol::Event;
use lumagame_core::{
    catalog::{self, Listing},
    engine::{Descriptor, Feature, Manifest},
    history::Played,
    ids::{EngineId, GameId, TargetId},
    ipc::{self, AuthState, FromEngine, Readiness, SessionState},
    settings::{SettingsSchema, Value},
    state::State,
};
use lumatv_render::{
    Gles,
    host::{Engine, HostConfig},
    now_ns,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};
use ui::{SceneOptions, Ui};

use fixtures::*;

fn screens() -> Vec<(String, Model)> {
    let base = fixture();
    let mut out: Vec<(String, Model)> = Vec::new();
    let mut add = |name: &str, mut model: Model| {
        model.rebuild_navigation();
        out.push((name.into(), model));
    };
    home::screens(&base, &mut add);
    library::screens(&base, &mut add);
    game::screens(&base, &mut add);
    settings::screens(&base, &mut add);
    accounts::linking(&base, &mut add);
    launch::screens(&base, &mut add);
    game::failed(&base, &mut add);
    accounts::screens(&base, &mut add);
    home::text200(&base, &mut add);
    media::screens(&base, &mut add);
    settings::families(&base, &mut add);
    accounts::editing(&base, &mut add);
    settings::controllers(&base, &mut add);
    quick::screens(&base, &mut add);
    let large = out
        .iter()
        .filter(|(name, _)| {
            [
                "51-stats-full-stale",
                "settings-a-picture",
                "11-pairing",
                "12-signin",
                "28-enter-pin",
                "29-approval",
                "home-a-empty", "settings-a-first-run",
                "41-add-pc",
                "42-edit-path",
                "settings-a-appearance",
                "settings-a-controllers",
                "settings-a-system",
                "settings-a-diagnostics-light",
                "43-controller-lab",
                "43-controller-lab-light",
                "48-stats-compact-healthy",
                "48-stats-compact-healthy-light",
                "48-stats-compact-healthy-bright",
                "48-stats-full-healthy",
                "48-stats-full-healthy-light",
                "48-stats-full-healthy-bright",
                "48-stats-compact-network",
                "48-stats-compact-network-light",
                "48-stats-compact-network-bright",
                "48-stats-full-network",
                "48-stats-full-network-light",
                "48-stats-full-network-bright",
                "48-stats-compact-host",
                "48-stats-compact-host-light",
                "48-stats-compact-host-bright",
                "48-stats-full-host",
                "48-stats-full-host-light",
                "48-stats-full-host-bright",
                "48-stats-compact-tv",
                "48-stats-compact-tv-light",
                "48-stats-compact-tv-bright",
                "48-stats-full-tv",
                "48-stats-full-tv-light",
                "48-stats-full-tv-bright",
                "44-controller-flash",
                "45-controller-lab-stream",
                "46-gamesir-ready",
                "46-gamesir-ready-light",
                "46-gamesir-ds4",
                "43-controller-sticks", "43-controller-buttons", "43-controller-timing", "43-controller-shaping", "43-controller-captured",
                "45-controller-lab-stream-bottom", "46-gamesir-bottom", "46-gamesir-pending", "46-gamesir-error",
                "47-leadjoy-ready", "47-leadjoy-light", "47-leadjoy-pending", "47-leadjoy-error", "47-leadjoy-buttons", "47-leadjoy-sticks",
                "47-leadjoy-rename", "47-leadjoy-macros", "47-leadjoy-motion", "47-leadjoy-delete",
                "47-leadjoy-keyboard-dark", "47-leadjoy-keyboard-light", "47-leadjoy-mouse-dark", "47-leadjoy-mouse-light",
                "47-leadjoy-motion-long-dark", "47-leadjoy-motion-long-light", "47-leadjoy-sticks-missing-dark", "47-leadjoy-sticks-missing-light",
                "47-leadjoy-profiles-sub-dark", "47-leadjoy-profiles-sub-light",
            ]
            .contains(&name.as_str())
        })
        .map(|(name, m)| {
            let mut m = m.clone();
            m.prefs.text_percent = 200;
            if m.view.page == Page::Settings && m.view.layers.is_empty() {
                m.view.focus = Focus::Setting(if m.view.place == Place::Appearance { 3 } else { 0 });
            }
            m.rebuild_navigation();
            (format!("{name}-text200"), m)
        })
        .collect::<Vec<_>>();
    out.extend(large);
    let pads = out
        .iter()
        .filter(|(name, _)| {
            [
                "home-a-spotlight", "library-b-panel", "09-chooser", "32-chooser-text200", "11-pairing", "12-signin", "29-approval", "launch-c-strip", "launch-c-strip-text200",
                "11-pairing-text200", "12-signin-text200", "launch-c-strip-menu", "launch-c-strip-menu-text200", "launch-b-drawer-end",
            ]
            .contains(&name.as_str())
        })
        .flat_map(|(name, m)| {
            let mut dark = m.clone();
            // The pad's first press, so a remote key earlier in the fixture does not decide the glyphs.
            let report = lumatv_gfn_pad::RawReport { buttons: lumatv_gfn_pad::button::A, ..Default::default() };
            dark.step(Input::Lab { now_ns: 0, reports: vec![(0, report)], names: Default::default(), frame: 0 });
            let mut light = dark.clone();
            light.prefs.light = true;
            [(format!("{name}-pad"), dark), (format!("{name}-pad-light"), light)]
        })
        .collect::<Vec<_>>();
    out.extend(pads);
    // Each owned pad's markings, found from the name its kernel node carries; PlayStation only through the Button labels setting.
    let marked = out
        .iter()
        .filter(|(name, _)| ["09-chooser-pad", "32-chooser-text200-pad", "15-overlay-pad", "15-overlay-text200-pad"].iter().any(|s| name.starts_with(s)))
        .flat_map(|(name, m)| {
            [
                ("playstation", None),
                ("gamesir", Some("Guangzhou Chicken Run Network Technology Co., Ltd. Wireless Controller")),
                ("leadjoy", Some("Leadjoy Xeno Plus")),
            ]
            .map(|(id, detected)| {
                let mut m = m.clone();
                match detected {
                    Some(pad) => {
                        m.step(Input::Controllers([Some(pad.to_owned()), None, None, None]));
                    }
                    None => m.prefs.button_labels = super::input::Labels::Fixed(super::input::PadKind::PlayStation),
                }
                (format!("{name}-{id}"), m)
            })
        })
        .collect::<Vec<_>>();
    out.extend(marked);
    out.extend(accounts::steam_screens(&base));
    out.extend(library::refresh_screens(&base));
    out
}
#[test]
fn every_native_screen() -> io::Result<()> {
    let selection=std::env::var("GAME_SHOTS").unwrap_or_default();
    if selection.is_empty() { return Ok(()); }
    let scratch = std::env::var_os("CODY_SCRATCH").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let out = scratch.join("lumagame-native-shots");
    std::fs::create_dir_all(&out)?;
    let mocks = scratch.join("lumagame-shots");
    let mut shots = Shots::new()?;
    for (name, model) in screens() {
        if selection!="all" && !selection.split(',').any(|s| name.contains(s)) {continue;}
        let mut pixels = shots.shot(&model)?;
        if name.contains("-stats-") {assert!(!shots.page.show(&model)?, "unchanged HUD repainted");}
        if name.contains("-stats-") || name.contains("-idle-warning") {
            let app=if name.contains("bright") {1551360} else {1091500};
            let background=lumagame_meta::decode_image(&std::fs::read(assets().join(format!("{app}/frame.jpg")))?,
                lumagame_meta::ImageSpec {width:1920,height:1080,contain:false,trim_alpha:false}).map_err(io::Error::other)?;
            for (fg,bg) in pixels.chunks_exact_mut(4).zip(background.rgba.chunks_exact(4)) {
                let alpha=u32::from(fg[3]);
                for i in 0..3 {fg[i]=(u32::from(fg[i])+u32::from(bg[i])*(255-alpha)/255).min(255) as u8;}
                fg[3]=255;
            }
        }
        png(&out.join(format!("{name}.png")), 1920, 1080, &pixels)?;
        let mock = mocks.join(format!("{name}.png"));
        if mock.exists() {
            let reference =
                lumagame_meta::decode_image(&std::fs::read(mock)?, lumagame_meta::ImageSpec { width: 1920, height: 1080, contain: false, trim_alpha: false })
                    .map_err(io::Error::other)?;
            let mut side = vec![0; 3840 * 1080 * 4];
            for y in 0..1080 {
                side[y * 3840 * 4..(y * 3840 + 1920) * 4].copy_from_slice(&reference.rgba[y * 1920 * 4..(y + 1) * 1920 * 4]);
                side[(y * 3840 + 1920) * 4..(y + 1) * 3840 * 4].copy_from_slice(&pixels[y * 1920 * 4..(y + 1) * 1920 * 4]);
            }
            png(&out.join(format!("{name}-compare.png")), 3840, 1080, &side)?;
        }
        println!("{}", out.join(format!("{name}.png")).display());
    }
    Ok(())
}
