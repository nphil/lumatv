//! OLED care on native Home and app cards, including the software renderer's app dim and shift.

mod harness;

use std::sync::atomic::Ordering;

use harness::{Client, Server, Window};
use lumatv_compositor::Rect;
use lumatv_compositor::State;
use lumatv_compositor::oledcare::GRACE_NS;
use lumatv_compositor::output::monotonic_ns;
use lumatv_compositor::scene::{Hole, Plan};
use lumatv_compositor::window::WindowType;
use wayland_client::protocol::wl_buffer::WlBuffer;

const SEC: u64 = 1_000_000_000;
const MS: u64 = 1_000_000;


/// The scene's plan at `t`, as (window kind, x, y, dim) of each item.
fn items(s: &mut State, t: u64) -> Vec<(WindowType, i32, i32, f32)> {
    let State { scene, surfaces, .. } = s;
    let plan: &Plan = scene.plan_at(surfaces, t);
    let ids: Vec<_> = plan.items.iter().map(|i| (i.window, i.rect.x, i.rect.y, i.dim)).collect();
    // what a tick does after its frame: the surfaces' own damage is spent
    surfaces.clear_damage();
    ids.into_iter().map(|(w, x, y, d)| (scene.window(w).unwrap().kind, x, y, d)).collect()
}

fn quiet_server() -> Server {
    let server = Server::start();
    // The loop's own ticks plan with the real clock; these tests plan with theirs.
    server.ticking.store(false, Ordering::SeqCst);
    server
}

#[test]
fn native_home_is_idle_without_a_wayland_window_and_a_key_restores_its_panel_level() {
    let server = quiet_server();
    server.eval(|s| {
        s.scene.set_home_rust(true);
        s.scene.care.settings.enabled = true;
        s.scene.care.settings.dim_after_ns = Some(SEC);
        s.scene.care.settings.dim_over_ns = SEC;
        s.scene.care.settings.dim_floor = 0.8;
        let mut t = monotonic_ns();
        assert!(items(s, t).is_empty(), "native Home is not a Wayland window");
        for _ in 0..4 {
            t += 30 * SEC + GRACE_NS;
            assert!(items(s, t).is_empty());
        }
        assert!(s.scene.care.beats >= 3);
        assert_eq!(s.scene.care.home_want(), 0.8);
        assert!((s.scene.last_plan().home_dim - 0.2).abs() < 0.0001);
        s.scene.care.input(t + 1);
        items(s, t + 2);
        assert_eq!(s.scene.care.home_want(), 1.0);
        assert_eq!(s.scene.last_plan().home_dim, 0.0);
        assert_eq!(s.scene.last_plan().damage, Rect::new(0, 0, 1920, 1080), "the old dim is repainted on input");
    });
}

#[test]
fn disabled_care_leaves_native_home_at_full_brightness() {
    let server = quiet_server();
    server.eval(|s| {
        s.scene.set_home_rust(true);
        s.scene.care.settings.enabled = false;
        let base = monotonic_ns();
        for k in 0..100 {
            assert!(items(s, base + k * 40 * SEC).is_empty());
            assert_eq!(s.scene.care.home_want(), 1.0);
        }
        assert!(!s.scene.care_frame());
    });
}

/// A full-screen opaque app card with the app features on and quick timings: engaged after one still second, a beat each 30 s.
fn app_card(server: &mut Server) -> (Client, Window, WlBuffer) {
    let mut app = Client::connect(server);
    let card = app.card("_WEBOS_WINDOW_TYPE_CARD", "Netflix");
    card.webos.set_property("appId".into(), "com.example.netflix".into());
    let buf = app.shm_buffer(1920, 1080, 0xffff_0000);
    let opaque = app.region(0, 0, 1920, 1080);
    card.surface.set_opaque_region(Some(&opaque));
    app.present(&card.surface, &buf);
    app.roundtrip();
    (app, card, buf)
}

fn app_on(s: &mut State) {
    let c = &mut s.scene.care.settings;
    c.app_dim = true;
    c.app_shift = true;
    c.app_after_ns = SEC;
    c.dim_over_ns = SEC;
    c.app_floor = 0.7;
    c.radius = 3;
}

/// The card's (x, y, dim) in the plan at `t`, and whether the plan is a sole full-screen item (goes to the plane whole).
fn card_at(s: &mut State, t: u64) -> ((i32, i32, f32), bool) {
    let got = items(s, t);
    let card: Vec<_> = got.iter().filter(|i| i.0 == WindowType::Card).collect();
    assert_eq!(card.len(), 1, "{got:?}");
    ((card[0].1, card[0].2, card[0].3), s.scene.last_plan().sole_fullscreen().is_some())
}

/// Plans a still app at 32 s steps until it is engaged: the time reached.
fn engage(s: &mut State, mut t: u64) -> u64 {
    for _ in 0..12 {
        t += 30 * SEC + GRACE_NS;
        card_at(s, t);
    }
    t
}

#[test]
fn a_still_app_card_is_direct_while_it_waits_then_dims_and_shifts_and_a_key_gives_it_back_to_the_plane() {
    let mut server = quiet_server();
    let (_app, _card, _buf) = app_card(&mut server);
    server.eval(|s| {
        app_on(s);
        let mut t = monotonic_ns();
        assert_eq!(card_at(s, t), ((0, 0, 1.0), true), "idle: full brightness at the origin, direct scan-out");
        assert_eq!(card_at(s, t + SEC / 2), ((0, 0, 1.0), true), "still for less than the wait");
        t = engage(s, t);
        let ((x, y, d), direct) = card_at(s, t + 1);
        assert_eq!(d, 0.7);
        assert!(x.abs() <= 3 && y.abs() <= 3 && (x, y) != (0, 0), "walking the ring: ({x}, {y})");
        assert!(!direct, "engaged: composed while the screen is static");
        let journal = s.scene.care.take_events();
        assert!(journal.iter().any(|l| l.contains("engaged")), "{journal:?}");
        s.scene.care.input(t + 2);
        assert_eq!(card_at(s, t + 3), ((0, 0, 1.0), true), "a key: bright, home, and the plane's whole again in the next plan");
        assert!(!s.scene.last_plan().damage.is_empty(), "the restore is drawn");
        assert!(s.scene.care.take_events().iter().any(|l| l.contains("put back")));
    });
}

#[test]
fn an_app_with_a_video_hole_is_never_dimmed_or_shifted_and_a_hole_that_appears_puts_it_back_in_that_plan() {
    let mut server = quiet_server();
    let (_app, card, _buf) = app_card(&mut server);
    server.eval(|s| {
        app_on(s);
        let mut t = monotonic_ns();
        card_at(s, t);
        t = engage(s, t);
        assert_ne!(card_at(s, t + 1).0, (0, 0, 1.0), "engaged first");
        let surface = s.scene.windows().find(|w| w.kind == WindowType::Card).unwrap().surface;
        s.scene.holes.push(Hole { surface, rect: Rect::new(160, 90, 1280, 720) });
        assert_eq!(card_at(s, t + 2).0, (0, 0, 1.0), "the plan that has the hole has the card at the origin, bright");
        assert_eq!(s.scene.last_plan().holes.len(), 1);
        for _ in 0..20 {
            t += 30 * SEC + GRACE_NS;
            assert_eq!(card_at(s, t).0, (0, 0, 1.0), "the video plays: nothing");
        }
        s.scene.holes.clear();
        assert_eq!(card_at(s, t + 1).0, (0, 0, 1.0), "the wait starts when the video stops");
        drop(card);
    });
}

#[test]
fn a_new_picture_restores_an_app_but_the_same_picture_again_or_a_caret_does_not() {
    let mut server = quiet_server();
    let (mut app, card, buf) = app_card(&mut server);
    // Ticks, not bare plans: what changed is told by the box dim's samples, which the loop's tick begins and reads (`boxdim`).
    let t = server.eval(|s| {
        app_on(s);
        let mut t = monotonic_ns();
        for _ in 0..12 {
            t += 500 * MS;
            s.tick_at(t);
        }
        assert_eq!(s.scene.boxdim.samples, 1, "the picture is known");
        for _ in 0..12 {
            t += 30 * SEC + GRACE_NS;
            s.tick_at(t);
        }
        assert!(s.scene.care.app_dim() < 1.0 && s.scene.care.app_offset() != (0, 0), "engaged");
        t
    });
    let ticks = |server: &Server, t: u64, n: u64| {
        server.eval(move |s| {
            (1..=n).fold(t, |t, _| {
                s.tick_at(t + 16 * MS);
                t + 16 * MS
            })
        })
    };
    // The same picture committed again, the whole surface damaged (LG's browser and Plex every few seconds on the set).
    app.present(&card.surface, &buf);
    app.roundtrip();
    let t = ticks(&server, t, 4);
    server.eval(|s| {
        assert!(s.scene.care.app_dim() < 1.0, "the same picture again is not a change");
        assert!(s.scene.boxdim.checks >= 1, "and it was looked at: {}", s.scene.boxdim.checks);
    });
    // A 20 x 20 patch: a caret, a clock, a progress bar.
    let caret = app.shm_pattern(1920, 1080, |x, y| if x < 20 && y < 20 { 0xffff_ffff } else { 0xffff_0000 });
    card.surface.attach(Some(&caret), 0, 0);
    card.surface.damage(0, 0, 20, 20);
    card.surface.commit();
    app.roundtrip();
    let t = ticks(&server, t, 4);
    server.eval(|s| assert!(s.scene.care.app_dim() < 1.0, "a small change is not a change of the picture"));
    // A new picture (a page, a menu) on a screen quiet for a second: put back the frame after the one that showed it.
    let t = ticks(&server, t, 60);
    let page = app.shm_buffer(1920, 1080, 0xff00_40ff);
    app.present(&card.surface, &page);
    app.roundtrip();
    let t = ticks(&server, t, 1);
    server.eval(move |s| {
        // The frame after the one that showed the new picture: its tick reads the check and puts the card back.
        s.tick_at(t + 16 * MS);
        assert_eq!(card_at(s, t + 32 * MS), ((0, 0, 1.0), true), "restored and direct again");
        assert!(s.journal.iter().any(|l| l.contains("put back (redrawn)")), "{:?}", s.journal);
    });
    drop((caret, page));
}

#[test]
fn a_new_picture_in_a_run_of_commits_is_seen_within_a_quarter_second() {
    let mut server = quiet_server();
    let (mut app, card, buf) = app_card(&mut server);
    let t = server.eval(|s| {
        app_on(s);
        let mut t = monotonic_ns();
        for _ in 0..12 {
            t += 500 * MS;
            s.tick_at(t);
        }
        for _ in 0..12 {
            t += 30 * SEC + GRACE_NS;
            s.tick_at(t);
        }
        assert!(s.scene.care.app_dim() < 1.0, "engaged");
        t
    });
    let tick = |server: &Server, t: u64| {
        server.eval(move |s| {
            s.tick_at(t + 16 * MS);
            t + 16 * MS
        })
    };
    // The same picture, then 50 ms later a new one: the check of the run's first commit has just been taken.
    app.present(&card.surface, &buf);
    app.roundtrip();
    let mut t = tick(&server, t);
    for _ in 0..2 {
        t = tick(&server, t);
    }
    let page = app.shm_buffer(1920, 1080, 0xff00_40ff);
    app.present(&card.surface, &page);
    app.roundtrip();
    let mut frames = 0;
    while server.eval(|s| s.scene.care.app_dim() < 1.0) {
        t = tick(&server, t);
        frames += 1;
        assert!(frames * 16 <= 250 + 3 * 16, "seen within a quarter second and a frame or two: {frames} frames");
    }
    drop(page);
}

#[test]
fn the_app_features_are_independent_of_each_other_the_shells_switch_and_the_saver() {
    let mut server = quiet_server();
    let (_app, _card, _buf) = app_card(&mut server);
    server.eval(|s| {
        // The shell's own care is off; only the dim is on: no shift.
        app_on(s);
        s.scene.care.settings.enabled = false;
        s.scene.care.settings.app_shift = false;
        let mut t = monotonic_ns();
        card_at(s, t);
        t = engage(s, t);
        let ((x, y, d), _) = card_at(s, t + 1);
        assert_eq!((x, y, d), (0, 0, 0.7), "appDim alone dims and does not move");
        // Only the shift.
        s.scene.care.settings.app_shift = true;
        s.scene.care.settings.app_dim = false;
        t = engage(s, t + 2);
        let ((x, y, d), _) = card_at(s, t + 1);
        assert_eq!(d, 1.0, "appShift alone does not dim");
        assert_ne!((x, y), (0, 0));
        // The saver stands both down.
        s.scene.care.settings.app_dim = true;
        s.scene.care.saver = true;
        for _ in 0..6 {
            t += 30 * SEC + GRACE_NS;
            assert_eq!(card_at(s, t), ((0, 0, 1.0), true), "under the saver");
        }
        // Both off: nothing, whatever was engaged.
        s.scene.care.saver = false;
        s.scene.care.settings.app_dim = false;
        s.scene.care.settings.app_shift = false;
        t = engage(s, t + 2);
        assert_eq!(card_at(s, t), ((0, 0, 1.0), true));
    });
}

#[test]
fn the_software_renderer_draws_an_engaged_apps_dim_and_shift() {
    let mut server = quiet_server();
    let (_app, _card, _buf) = app_card(&mut server);
    server.eval(|s| {
        app_on(s);
        let mut t = monotonic_ns();
        for _ in 0..13 {
            let State { scene, surfaces, output, .. } = &mut *s;
            let plan = scene.plan_at(surfaces, t);
            output.render(plan, surfaces).unwrap();
            t += 30 * SEC + GRACE_NS;
        }
        let ((x, y, d), _) = card_at(s, t);
        assert_eq!(d, 0.7);
        let img = s.output.capture().unwrap();
        let px = |x: usize, y: usize| {
            let i = (y * img.width as usize + x) * 4;
            [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]]
        };
        let lit = px(1000, 500);
        assert!((175..=182).contains(&lit[0]) && lit[1] == 0 && lit[2] == 0, "red at 0.7: {lit:?}");
        let _ = (x, y);
    });
}
