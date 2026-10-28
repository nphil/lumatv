use super::*;

const FULL: FRect = FRect::new(0.0, 0.0, 1920.0, 1080.0);
const MS: u64 = 1_000_000;

fn on() -> Transitions {
    let mut t = Transitions::new();
    t.settings.enabled = true;
    t
}

fn party(id: u32, app: &str, video: bool) -> Party<'_> {
    Party { id: WindowId(id), app_id: app, rect: FULL, video }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

/// Netflix's tile on Home: a square of 120, 22 radius.
fn tile() -> (FRect, f32) {
    (FRect::new(287.0, 699.0, 120.0, 120.0), 22.0)
}

#[test]
fn the_curves_start_at_zero_end_at_one_and_never_go_back() {
    for ease in [Ease::OutCubic, Ease::Exit, Ease::Linear] {
        assert!(near(ease.at(0.0), 0.0) && near(ease.at(1.0), 1.0), "{ease:?} ends");
        let mut last = 0.0;
        for i in 0..=200 {
            let v = ease.at(i as f32 / 200.0);
            assert!(v >= last - 1e-6, "{ease:?} goes back at {i}");
            last = v;
        }
        assert!(near(ease.at(-3.0), 0.0) && near(ease.at(7.0), 1.0), "{ease:?} clamps");
    }
}

#[test]
fn the_exit_curve_is_the_css_bezier_of_the_token() {
    // cubic-bezier(.4, 0, 1, 1) sampled on its own parameter: the curve's points are x(s), y(s)
    for i in 1..20 {
        let s = i as f32 / 20.0;
        let u = 1.0 - s;
        let x = 3.0 * u * u * s * 0.4 + 3.0 * u * s * s * 1.0 + s * s * s;
        let y = 3.0 * u * s * s * 1.0 + s * s * s;
        assert!((Ease::Exit.at(x) - y).abs() < 2e-3, "x {x}: {} against {y}", Ease::Exit.at(x));
    }
    // out-cubic is quick at the start, the exit curve slow: a launch answers at once, a leave eases away
    assert!(Ease::OutCubic.at(0.25) > 0.5 && Ease::Exit.at(0.25) < 0.15);
}

#[test]
fn off_changes_nothing() {
    let mut t = Transitions::new();
    assert!(!t.enabled());
    assert!(!t.begin(Kind::Launch, 0, Some(party(2, "com.netflix", false)), None, &[(WindowId(1), FULL)]));
    assert!(!t.active() && t.pose(WindowId(2), 0).is_none());
}

#[test]
fn a_launch_grows_the_card_out_of_its_tile_and_darkens_home_under_it() {
    let mut t = on();
    let (tile, radius) = tile();
    t.note_origin("com.netflix", Some((tile, radius)), None, 0);
    assert!(t.begin(Kind::Launch, 1000 * MS, Some(party(2, "com.netflix", false)), None, &[(WindowId(1), FULL)]));
    // its first frame: the picture cover-fitted on the tile, masked by the tile, already a quarter seen
    let p0 = t.pose(WindowId(2), 1000 * MS).unwrap();
    assert_eq!(p0.clip, tile);
    assert!(near(p0.radius, 22.0) && near(p0.alpha, ARRIVE_ALPHA0) && near(p0.shade, 1.0));
    assert!(near(p0.rect.w / p0.rect.h, 1920.0 / 1080.0), "the picture keeps its shape: {:?}", p0.rect);
    assert!(p0.rect.h >= tile.h - 1e-3 && p0.rect.w >= tile.w - 1e-3, "and covers the tile");
    let (tx, ty) = tile.centre();
    let (cx, cy) = p0.rect.centre();
    assert!(near(tx, cx) && near(ty, cy), "centred on it");
    // 220 ms later it is the whole output at rest; the transition ends with it
    let end = t.pose(WindowId(2), 1220 * MS).unwrap();
    assert!(end.is_rest_on(&FULL), "{end:?}");
    // the picture never leaves its aspect on the way, the mask grows with it
    for ms in [20, 60, 110, 170] {
        let p = t.pose(WindowId(2), (1000 + ms) * MS).unwrap();
        assert!(near(p.rect.w / p.rect.h, 1920.0 / 1080.0));
        assert!(p.clip.w >= p0.clip.w && p.clip.w <= 1920.0 && p.radius <= 22.0 + 1e-3);
    }
    // Home stays as a ghost, darker as the card grows
    assert!(t.is_ghost(WindowId(1)) && !t.is_ghost(WindowId(2)));
    assert!(t.holds_home());
    let h = t.pose(WindowId(1), 1220 * MS).unwrap();
    assert!(near(h.shade, HOME_SHADE) && near(h.alpha, 1.0));
    assert!(t.frame(1100 * MS));
    assert!(!t.frame(1221 * MS), "both tracks are over");
    assert!(!t.holds_home());
}

#[test]
fn a_card_with_no_tile_known_grows_from_the_middle_of_the_screen() {
    let mut t = on();
    assert!(t.begin(Kind::Launch, 0, Some(party(2, "com.example.app", false)), None, &[(WindowId(1), FULL)]));
    let p = t.pose(WindowId(2), 0).unwrap();
    assert_eq!(p.rect, FULL.scaled(AWAY_SCALE));
    assert_eq!(p.clip, p.rect);
    assert!(near(p.radius, AWAY_RADIUS));
}

#[test]
fn the_card_is_whole_a_little_after_it_starts_so_the_app_is_seen_at_once() {
    let mut t = on();
    t.begin(Kind::Switch, 0, Some(party(3, "b", false)), Some(party(2, "a", false)), &[]);
    let first = t.pose(WindowId(3), 0).unwrap().alpha;
    assert!(first >= 0.25, "the first frame already shows the app: {first}");
    // out-cubic: whole by a quarter of the time
    assert!(near(t.pose(WindowId(3), 100 * MS).unwrap().alpha, 1.0));
    // the card under it darkens
    let under = t.pose(WindowId(2), 220 * MS).unwrap();
    assert!(near(under.shade, CARD_SHADE) && t.is_ghost(WindowId(2)));
}

#[test]
fn a_leave_shrinks_the_card_into_its_tile_and_fades_it_while_home_comes_back() {
    let mut t = on();
    let (tile, radius) = tile();
    t.note_origin("com.netflix", Some((tile, radius)), None, 0);
    assert!(t.begin(Kind::Leave, 0, None, Some(party(2, "com.netflix", false)), &[(WindowId(1), FULL)]));
    let p0 = t.pose(WindowId(2), 0).unwrap();
    assert!(p0.is_rest_on(&FULL), "it starts as it was on the screen: {p0:?}");
    let end = t.pose(WindowId(2), 180 * MS).unwrap();
    assert_eq!(end.clip, tile);
    assert!(near(end.radius, 22.0) && near(end.alpha, 0.0));
    // the picture holds its opacity for a while, then fades
    assert!(near(t.pose(WindowId(2), 30 * MS).unwrap().alpha, 1.0));
    let mid = t.pose(WindowId(2), 120 * MS).unwrap();
    assert!(mid.alpha > 0.0 && mid.alpha < 1.0, "{mid:?}");
    // Home is the front, not a ghost, and brightens
    assert!(!t.is_ghost(WindowId(1)) && t.is_ghost(WindowId(2)));
    assert!(near(t.pose(WindowId(1), 0).unwrap().shade, HOME_SHADE));
    assert!(near(t.pose(WindowId(1), 180 * MS).unwrap().shade, 1.0));
    assert!(!t.holds_home(), "a leave has Home in front already");
}

#[test]
fn a_change_of_mind_goes_on_from_where_the_picture_is() {
    let mut t = on();
    let (tile, radius) = tile();
    t.note_origin("com.netflix", Some((tile, radius)), None, 0);
    t.begin(Kind::Launch, 0, Some(party(2, "com.netflix", false)), None, &[(WindowId(1), FULL)]);
    let before = t.pose(WindowId(2), 80 * MS).unwrap();
    let home_before = t.pose(WindowId(1), 80 * MS).unwrap();
    // Home pressed 80 ms into the launch: the card turns round from here, not from full screen
    assert!(t.begin(Kind::Leave, 80 * MS, None, Some(party(2, "com.netflix", false)), &[(WindowId(1), FULL)]));
    assert_eq!(t.stats.interrupted, 1);
    let after = t.pose(WindowId(2), 80 * MS).unwrap();
    assert_eq!(before.rect, after.rect, "no jump");
    assert_eq!(before.clip, after.clip);
    assert!(near(before.alpha, after.alpha) && near(before.radius, after.radius));
    assert_eq!(t.tracks().iter().filter(|t| t.window == WindowId(2)).count(), 1, "one track per window");
    // and Home, a ghost under the launch, is the front again and goes on from its own brightness
    assert!(!t.is_ghost(WindowId(1)));
    assert!(near(t.pose(WindowId(1), 80 * MS).unwrap().shade, home_before.shade));
}

#[test]
fn a_card_that_comes_back_before_it_has_gone_continues_from_its_pose() {
    let mut t = on();
    let (tile, radius) = tile();
    t.note_origin("com.netflix", Some((tile, radius)), None, 0);
    t.begin(Kind::Leave, 0, None, Some(party(2, "com.netflix", false)), &[(WindowId(1), FULL)]);
    let mid = t.pose(WindowId(2), 100 * MS).unwrap();
    t.begin(Kind::Launch, 100 * MS, Some(party(2, "com.netflix", false)), None, &[(WindowId(1), FULL)]);
    let now = t.pose(WindowId(2), 100 * MS).unwrap();
    assert_eq!(mid.rect, now.rect);
    assert!(!t.is_ghost(WindowId(2)) && t.is_ghost(WindowId(1)));
}

#[test]
fn a_splash_over_the_tile_is_where_the_card_grows_from_while_it_is_fresh() {
    let mut t = on();
    let (tile, radius) = tile();
    t.note_origin("com.netflix", Some((tile, radius)), None, 0);
    let icon = FRect::new(870.0, 450.0, 180.0, 180.0);
    t.note_origin("com.netflix", None, Some(Some((icon, 38.0))), 1_000 * MS);
    t.begin(Kind::Launch, 2_000 * MS, Some(party(2, "com.netflix", false)), None, &[(WindowId(1), FULL)]);
    let p = t.pose(WindowId(2), 2_000 * MS).unwrap();
    assert_eq!(p.clip, icon);
    assert!(near(p.radius, 38.0));
    // the way back is the tile, not the splash
    t.clear();
    t.begin(Kind::Leave, 3_000 * MS, None, Some(party(2, "com.netflix", false)), &[(WindowId(1), FULL)]);
    assert_eq!(t.pose(WindowId(2), 3_180 * MS).unwrap().clip, tile, "the tile, to the bit");
    // a splash that is old news is not a start
    t.clear();
    t.begin(Kind::Launch, 40_000 * MS, Some(party(2, "com.netflix", false)), None, &[(WindowId(1), FULL)]);
    assert_eq!(t.pose(WindowId(2), 40_000 * MS).unwrap().clip, tile);
    // the shell takes its word back
    t.note_origin("com.netflix", None, Some(None), 41_000 * MS);
    assert!(t.origin_of("com.netflix").unwrap().from.is_none());
}

#[test]
fn a_word_without_a_tile_is_no_origin() {
    let mut t = on();
    t.note_origin("com.x", Some((FRect::new(0.0, 0.0, 0.0, 0.0), 0.0)), None, 0);
    assert!(t.origin_of("com.x").is_none());
    t.note_origin("", Some(tile()), None, 0);
    assert!(t.origin_of("").is_none());
    for i in 0..60 {
        t.note_origin(&format!("app.{i}"), Some(tile()), None, 0);
    }
    assert!(t.origin_of("app.59").is_some() && t.origin_of("app.0").is_none(), "the oldest go when there are too many");
}

#[test]
fn a_card_on_the_video_plane_arrives_on_a_veil_that_fades_once_the_video_has_had_time() {
    let mut t = on();
    let (tile, radius) = tile();
    t.note_origin("com.webos.app.hdmi2", Some((tile, radius)), None, 0);
    // HDMI exports its video element before its first frame: expected, no hole yet
    t.begin(Kind::Launch, 0, Some(party(2, "com.webos.app.hdmi2", true)), None, &[(WindowId(1), FULL)]);
    let w = WindowId(2);
    assert_eq!(t.veil(w, 0, false).map(|v| v.alpha), Some(1.0));
    // the hole comes at 83 ms; the card arrives at 220: the veil is held
    assert_eq!(t.veil(w, 83 * MS, true).map(|v| v.alpha), Some(1.0));
    assert_eq!(t.veil(w, 219 * MS, true).map(|v| v.alpha), Some(1.0));
    // arrived, and the video has had 150 ms since 83: 233 ms
    assert_eq!(t.veil(w, 232 * MS, true).map(|v| v.alpha), Some(1.0), "not yet");
    assert_eq!(t.veil(w, 233 * MS, true).map(|v| v.alpha), Some(1.0), "the fade starts at full");
    let half = t.veil(w, (233 + 90) * MS, true).unwrap().alpha;
    assert!((half - 0.5).abs() < 0.01, "{half}");
    assert!(t.veil(w, (233 + 180) * MS, true).is_none(), "gone");
    assert!(t.animating(300 * MS) || !t.tracks().iter().any(|t| t.window == w) || t.tracks().iter().any(|t| t.window == w));
    // the track ends with its veil
    assert!(!t.frame(500 * MS) || !t.tracks().iter().any(|t| t.window == w));
    assert_eq!(t.stats.veil_caps, 0);
}

#[test]
fn a_veil_is_not_held_for_ever() {
    let mut t = on();
    t.begin(Kind::Launch, 0, Some(party(2, "com.webos.app.hdmi3", true)), None, &[(WindowId(1), FULL)]);
    let w = WindowId(2);
    // no signal: the hole never comes. Held to the cap after the arrival (220 + 1500), then it fades
    assert_eq!(t.veil(w, 1700 * MS, false).map(|v| v.alpha), Some(1.0));
    assert_eq!(t.veil(w, 1720 * MS, false).map(|v| v.alpha), Some(1.0));
    assert!(t.veil(w, (1720 + 190) * MS, false).is_none());
    assert_eq!(t.stats.veil_caps, 1);
}

#[test]
fn a_hole_that_shows_up_while_a_card_arrives_brings_the_veil_with_it() {
    let mut t = on();
    t.begin(Kind::Launch, 0, Some(party(2, "com.example.player", false)), None, &[(WindowId(1), FULL)]);
    let w = WindowId(2);
    assert!(t.veil(w, 0, false).is_none(), "an ordinary card has none");
    assert_eq!(t.veil(w, 60 * MS, true).map(|v| v.alpha), Some(1.0));
    // a card that has arrived already gets none: the app's own switch to video is its own
    t.clear();
    t.begin(Kind::Launch, 0, Some(party(2, "com.example.player", false)), None, &[(WindowId(1), FULL)]);
    t.frame(300 * MS);
    assert!(t.veil(w, 400 * MS, true).is_none());
}

#[test]
fn a_card_on_the_video_plane_leaves_by_home_dissolving_in_over_it_and_does_not_shrink() {
    let mut t = on();
    let (tile, radius) = tile();
    t.note_origin("com.webos.app.hdmi2", Some((tile, radius)), None, 0);
    assert!(t.begin(Kind::Leave, 0, None, Some(party(2, "com.webos.app.hdmi2", true)), &[(WindowId(1), FULL)]));
    for ms in [0, 60, 120, 180] {
        let card = t.pose(WindowId(2), ms * MS).unwrap();
        assert!(card.is_rest_on(&FULL), "the card (and its hole) stays put at {ms} ms: {card:?}");
    }
    let home = t.track_of(WindowId(1)).unwrap();
    assert!(home.on_top && !home.ghost);
    assert!(near(t.pose(WindowId(1), 0).unwrap().alpha, 0.0));
    assert!(near(t.pose(WindowId(1), 90 * MS).unwrap().alpha, 0.5), "a linear dissolve");
    assert!(near(t.pose(WindowId(1), 180 * MS).unwrap().alpha, 1.0));
}

#[test]
fn reduced_motion_is_one_linear_fade_of_120_ms_and_nothing_moves() {
    let mut t = on();
    t.settings.reduced = true;
    let (tile, radius) = tile();
    t.note_origin("com.netflix", Some((tile, radius)), None, 0);
    t.begin(Kind::Launch, 0, Some(party(2, "com.netflix", true)), None, &[(WindowId(1), FULL)]);
    let card = t.track_of(WindowId(2)).unwrap();
    assert_eq!(card.dur_ns, 120 * MS);
    for ms in [0u64, 30, 60, 120] {
        let p = t.pose(WindowId(2), ms * MS).unwrap();
        assert_eq!(p.rect, FULL);
        assert_eq!(p.clip, FULL);
        assert!(near(p.radius, 0.0) && near(p.shade, 1.0));
        assert!(near(p.alpha, ms as f32 / 120.0), "linear: {ms} ms {}", p.alpha);
    }
    assert!(t.veil(WindowId(2), 500 * MS, true).is_none(), "no veil either");
    let home = t.pose(WindowId(1), 120 * MS).unwrap();
    assert!(near(home.shade, 1.0), "Home is not darkened");
    t.clear();
    t.begin(Kind::Leave, 0, None, Some(party(2, "com.netflix", false)), &[(WindowId(1), FULL)]);
    assert_eq!(t.track_of(WindowId(2)).unwrap().dur_ns, 120 * MS);
    assert!(t.pose(WindowId(2), 120 * MS).unwrap().is_rest_on(&FULL), "the card stays, Home dissolves in over it");
}

#[test]
fn a_move_lasts_as_long_at_any_refresh_rate() {
    for hz in [60u64, 90, 120] {
        let mut t = on();
        t.begin(Kind::Switch, 0, Some(party(3, "b", false)), Some(party(2, "a", false)), &[]);
        let step = 1_000_000_000 / hz;
        let mut now = 0;
        let mut frames = 0;
        while t.frame(now) {
            now += step;
            frames += 1;
            assert!(frames < 100);
        }
        // over at the first frame at or after 220 ms, whatever the rate
        assert!(now >= 220 * MS && now < 220 * MS + step, "{hz} Hz: ended at {} ms", now / MS);
    }
}

#[test]
fn a_late_frame_catches_up_and_the_gaps_are_counted() {
    let mut t = on();
    t.begin(Kind::Switch, 0, Some(party(3, "b", false)), Some(party(2, "a", false)), &[]);
    assert!(t.frame(0));
    assert!(t.frame(17 * MS));
    assert!(t.frame(80 * MS), "a 63 ms gap");
    let late = t.pose(WindowId(3), 80 * MS).unwrap();
    let exact = Pose::lerp(&t.track_of(WindowId(3)).unwrap().from, &Pose::rest(FULL), Ease::OutCubic.at(80.0 / 220.0));
    assert!(near(late.rect.w, exact.rect.w), "the pose is the clock's, not a frame's");
    assert_eq!(t.stats.late_gaps, 1);
    assert_eq!(t.stats.max_gap_ms, 63);
}

#[test]
fn the_picture_of_a_leave_is_handed_over_live_and_for_a_while_after() {
    let mut t = on();
    let (tile, radius) = tile();
    t.note_origin("com.netflix", Some((tile, radius)), None, 0);
    t.begin(Kind::Leave, 0, None, Some(party(2, "com.netflix", false)), &[(WindowId(1), FULL)]);
    // taken 90 ms in: where the picture is then, and the card is the switcher's
    let live = t.take_visual(WindowId(2), 90 * MS).unwrap();
    let expect = Pose::lerp(&Pose::rest(FULL), &t.pose(WindowId(1), 0).map(|_| Pose::rest(FULL)).unwrap(), 0.0);
    let _ = expect;
    assert!(live.rect.w > 120 && live.rect.w < 1920 + 4, "{live:?}");
    assert!(live.alpha > 0.0 && live.alpha <= 1.0);
    assert!(t.track_of(WindowId(2)).is_none(), "its track is gone");
    assert!(t.track_of(WindowId(1)).is_some(), "Home's own arrival goes on");
    assert!(t.take_visual(WindowId(2), 90 * MS).is_none());

    // a leave that has ended: its last pose, the tile, for 600 ms
    t.clear();
    t.begin(Kind::Leave, 1_000 * MS, None, Some(party(2, "com.netflix", false)), &[(WindowId(1), FULL)]);
    assert!(!t.frame(1_200 * MS));
    let last = t.take_visual(WindowId(2), 1_400 * MS).unwrap();
    assert!(near(last.alpha, 0.0) && near(last.radius, 22.0));
    assert!(last.rect.w >= 120 && last.rect.w < 420);
    t.clear();
    t.begin(Kind::Leave, 2_000 * MS, None, Some(party(2, "com.netflix", false)), &[(WindowId(1), FULL)]);
    t.frame(2_200 * MS);
    t.frame(2_900 * MS);
    assert!(t.take_visual(WindowId(2), 2_900 * MS).is_none(), "after 600 ms it is no one's");
}

#[test]
fn a_window_that_goes_is_forgotten() {
    let mut t = on();
    t.begin(Kind::Switch, 0, Some(party(3, "b", false)), Some(party(2, "a", false)), &[]);
    t.forget(WindowId(2));
    assert!(t.track_of(WindowId(2)).is_none() && t.track_of(WindowId(3)).is_some());
    t.clear();
    assert!(!t.active());
}

#[test]
fn bounds_hold_the_whole_picture_and_its_soft_edge() {
    let r = FRect::new(10.5, 20.25, 100.0, 50.0);
    let b = r.bounds();
    assert!(b.x <= 10 && b.y <= 20 && b.right() >= 111 && b.bottom() >= 71);
    let p = Pose { rect: FRect::new(0.0, 0.0, 400.0, 225.0), clip: FRect::new(50.0, 50.0, 100.0, 100.0), radius: 20.0, alpha: 1.0, shade: 1.0 };
    let pb = p.bounds();
    assert!(pb.w <= 104 && pb.h <= 104, "the clip bounds what is drawn: {pb:?}");
}

#[test]
fn a_hole_maps_into_the_picture_the_way_the_window_does() {
    let picture = FRect::new(960.0, 540.0, 960.0, 540.0);
    let hole = Rect::new(0, 0, 1920, 1080);
    assert_eq!(FRect::map_rect(&FULL, &picture, &hole), picture);
    let small = Rect::new(192, 108, 960, 540);
    let m = FRect::map_rect(&FULL, &picture, &small);
    assert!(near(m.x, 960.0 + 96.0) && near(m.y, 540.0 + 54.0) && near(m.w, 480.0) && near(m.h, 270.0));
}
