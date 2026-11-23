//! The parts gallery: each shared part on its own, dark and light, to judge against the mock apart from any screen's layout. It runs
//! only with `GAME_SHOTS` set (`parts` or `all`); a shot paints one part through `GameUi`'s test hook in place of a page.
use super::*;
use crate::game::model::nav::Hints;
use crate::game::screens::{
    self,
    paint::{Fill, H, Paint, W},
    parts::{
        Side,
        card::{self, Card, Headline, Source},
        dialog::{self, Action},
        drawer, hints, keyboard::{self, TextEntry}, layer,
        rail::{self, Place, RailSpec},
        row::{self, Leading, Line},
        strip::{self, Item, Sign},
        topbar::{self, Here},
    },
};
use lumatv_tokens::Radius;
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};

struct Shot {
    name: &'static str,
    setup: fn(&mut Model),
    paint: fn(&mut Paint<'_>),
}

fn canvas(p: &mut Paint<'_>) {
    let colour = p.ink(Role::Canvas, false);
    p.solid(2, Frame::new(0.0, 0.0, W, H), colour, 0.0, 0);
}
/// Stand-in for a game's art: bands of colour on the canvas, so the glass and the scrims over them show.
fn art(p: &mut Paint<'_>) {
    canvas(p);
    p.art = true;
    let bands = [(0.0, 0.0, 0.58, 1.0, Role::Info), (0.42, 0.0, 0.58, 0.62, Role::Warning), (0.3, 0.55, 0.7, 0.45, Role::Positive)];
    for (i, (x, y, w, h, role)) in bands.into_iter().enumerate() {
        let colour = p.ink(role, true);
        p.solid(3 + i as u64, Frame::new(W * x, H * y, W * w, H * h), colour, 0.0, 1);
    }
}
fn keys() -> Hints {
    Hints(vec![(Key::Back, "Back"), (Key::Ok, "Choose"), (Key::Menu, "Menu")])
}
fn pad(m: &mut Model) {
    m.step(Input::Controllers([Some("Pad".to_owned()), None, None, None]));
}

fn bar_over_art(p: &mut Paint<'_>) {
    art(p);
    topbar::bar(p, Here::Library, true);
}
fn bar_flat(p: &mut Paint<'_>) {
    canvas(p);
    topbar::bar(p, Here::Home, false);
}
fn hints_corner(p: &mut Paint<'_>) {
    art(p);
    hints::bar(p, &keys(), Side::Right, false);
}
fn hints_resting(p: &mut Paint<'_>) {
    art(p);
    hints::bar(p, &keys(), Side::Left, true);
}
fn cards(p: &mut Paint<'_>) {
    art(p);
    let frame = |i: usize| Frame::new(p.x + i as f32 * 544.0, 372.0, 520.0, 330.0);
    let thumbs = [assets().join("1091500/frame.jpg"), assets().join("1091500/frame.jpg"), assets().join("1091500/frame.jpg")];
    let first = Card {
        id: 7000,
        label: "Achievements",
        headline: Headline::Big("12 of 33"),
        detail: Some("Latest: Tipping the scales"),
        progress: Some(0.36),
        thumbs: &thumbs,
        caption: None,
        source: Some(Source { name: "Steam", stale: None }),
    };
    let second = Card { id: 7020, label: "Last played", headline: Headline::Big("2 h ago"), detail: Some("1 h 12 min on Desk PC"), progress: None, thumbs: &[], caption: Some("31 h in total"), source: None };
    let third = Card {
        id: 7040,
        label: "News, 12 Nov",
        headline: Headline::Text("Patch 1.2 brings a new weapon and a fix for the boss"),
        detail: Some("The long-awaited update is out today."),
        progress: None,
        thumbs: &[],
        caption: None,
        source: Some(Source { name: "Steam", stale: Some("2 days ago") }),
    };
    let (a, b, c) = (frame(0), frame(1), frame(2));
    card::glance(p, &first, a, true);
    card::glance(p, &second, b, false);
    card::glance(p, &third, c, false);
}
fn rows(p: &mut Paint<'_>) {
    canvas(p);
    let pane = Frame::new(p.x - 16.0, 150.0, 932.0, 620.0);
    p.surface(9000, pane, Fill::Sheet, Radius::Sheet, false, false, false, 2);
    let x = p.x;
    let at = |i: usize| Frame::new(x, 170.0 + i as f32 * 84.0, 900.0, 80.0);
    let line = |lead, name, sub| Line { lead, name, sub };
    let picture = assets().join("1091500/frame.jpg");
    row::link(p, 7100, at(0), &line(Leading::Glyph(Icon::Cog), "All settings", Some("Streaming, appearance, controllers")), false);
    row::value(p, 7120, at(1), &line(Leading::Glyph(Icon::Person), "Accounts and PCs", None), "Ready", Some((Icon::Check, Role::Positive)), false);
    row::switch(p, 7140, at(2), &line(Leading::Glyph(Icon::Speaker), "HDR", Some("Needs a display that supports it")), true, false);
    row::stepper(p, 7160, at(3), &line(Leading::Glyph(Icon::Gauge), "Stream", Some("Resolution and frame rate")), "4K, 120 fps", true);
    row::link(p, 7180, at(4), &line(Leading::Picture(&picture), "Hades II", Some("Played 2 h ago")), false);
}
fn drawer_menu(p: &mut Paint<'_>) {
    art(p);
    let content = drawer::open(p, Side::Right, drawer::WIDTH, "Menu");
    let at = |i: usize| Frame::new(content.x, content.y + i as f32 * 84.0, content.w, 80.0);
    let line = |lead, name, sub| Line { lead, name, sub };
    row::link(p, 7100, at(0), &line(Leading::Glyph(Icon::Refresh), "Refresh libraries", Some("Updated 2 min ago")), true);
    row::link(p, 7120, at(1), &line(Leading::Glyph(Icon::Person), "Accounts and PCs", Some("Desk PC is offline")), false);
    row::link(p, 7140, at(2), &line(Leading::Glyph(Icon::Cog), "Settings", None), false);
    hints::bar(p, &Hints(vec![(Key::Back, "Close")]), Side::Left, false);
}
fn confirm(p: &mut Paint<'_>) {
    art(p);
    let actions = [
        Action { label: "Keep playing", icon: None, focus: Focus::SheetAction(0) },
        Action { label: "End session", icon: Some(Icon::Power), focus: Focus::SheetAction(1) },
    ];
    dialog::modal(p, "End this session?", "The game closes on the PC. Unsaved progress is lost.", &actions, 0);
}
fn quick_strip(p: &mut Paint<'_>) {
    art(p);
    let items = [
        Item { icon: Sign::Icon(Icon::Gauge), label: Some("Stats"), focus: Focus::Strip(0), danger: false },
        Item { icon: Sign::Icon(Icon::Sliders), label: Some("Settings"), focus: Focus::Strip(1), danger: false },
        Item { icon: Sign::Icon(Icon::Power), label: Some("End session"), focus: Focus::Strip(2), danger: true },
    ];
    strip::bar(p, &items, Some(1));
}
fn reader(p: &mut Paint<'_>) {
    art(p);
    let slot = layer::section(p, &["Activity", "Community", "Media", "About"], 1);
    row::value(p, 7100, Frame::new(slot.x, slot.y, slot.w, 80.0), &Line { lead: Leading::Glyph(Icon::Message), name: "Patch notes", sub: Some("12 Nov") }, "128 replies", None, false);
}
fn settings_shell(p: &mut Paint<'_>) {
    canvas(p);
    let places = [
        Place { icon: Icon::Speaker, name: "Streaming", state: None },
        Place { icon: Icon::Palette, name: "Appearance", state: None },
        Place { icon: Icon::Cards, name: "Controllers", state: None },
        Place { icon: Icon::Person, name: "Accounts and PCs", state: Some("1 needs attention") },
        Place { icon: Icon::Info, name: "About", state: None },
    ];
    let hints = keys();
    let spec = RailSpec { title: "Settings", places: &places, current: 3, crumb: None, heading: "Accounts and PCs", chip: Some("Steam linked"), hints: &hints };
    let pane = rail::shell(p, &spec);
    row::link(p, 7100, Frame::new(pane.x, pane.y, pane.w, 80.0), &Line { lead: Leading::Glyph(Icon::Person), name: "Steam", sub: Some("Linked as nitin") }, true);
    hints::bar(p, &hints, Side::Right, false);
}
fn entry(p: &mut Paint<'_>) {
    art(p);
    let entry = TextEntry {
        title: "Add a PC",
        label: "Address",
        value: "192.168.1.",
        placeholder: "Name or address",
        secret: false,
        numeric: false,
        lower: true,
        action: Some(("Add", Icon::Plus)),
        notice: None,
    };
    keyboard::plan(p, &entry);
}

fn gallery() -> Vec<Shot> {
    let none: fn(&mut Model) = |_| {};
    vec![
        Shot { name: "topbar", setup: |m| m.view.focus = Focus::Nav(2), paint: bar_over_art },
        Shot { name: "topbar-flat-pad", setup: pad, paint: bar_flat },
        Shot { name: "hints", setup: none, paint: hints_corner },
        Shot { name: "hints-resting", setup: none, paint: hints_resting },
        Shot { name: "cards", setup: none, paint: cards },
        Shot { name: "rows", setup: none, paint: rows },
        Shot { name: "drawer", setup: none, paint: drawer_menu },
        Shot { name: "dialog", setup: |m| m.view.focus = Focus::SheetAction(0), paint: confirm },
        Shot { name: "strip", setup: |m| m.view.focus = Focus::Strip(1), paint: quick_strip },
        Shot { name: "section", setup: none, paint: reader },
        Shot { name: "rail", setup: |m| m.view.focus = Focus::Rail(3), paint: settings_shell },
        Shot { name: "keyboard", setup: |m| m.view.focus = Focus::Keyboard(0), paint: entry },
    ]
}

#[test]
fn parts_gallery() -> io::Result<()> {
    let selection = std::env::var("GAME_SHOTS").unwrap_or_default();
    if selection.is_empty() {
        return Ok(());
    }
    let wanted = |name: &str| selection == "all" || selection.split(',').any(|s| name.contains(s));
    let scratch = std::env::var_os("CODY_SCRATCH").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let out = scratch.join("lumagame-native-shots");
    std::fs::create_dir_all(&out)?;
    let mut shots = Shots::new()?;
    let base = fixture();
    for shot in gallery() {
        for light in [false, true] {
            let name = format!("parts-{}{}", shot.name, if light { "-light" } else { "" });
            if !wanted(&name) {
                continue;
            }
            let mut model = base.clone();
            model.prefs.light = light;
            (shot.setup)(&mut model);
            model.rebuild_navigation();
            screens::gallery(Some(shot.paint));
            let pixels = shots.shot(&model)?;
            png(&out.join(format!("{name}.png")), 1920, 1080, &pixels)?;
            println!("{}", out.join(format!("{name}.png")).display());
        }
    }
    // A layer let go: the drawer 100 ms into its 180 ms out, which only the frames move (the model's clock is not ticked).
    if wanted("parts-leaving") {
        let mut model = base.clone();
        model.rebuild_navigation();
        screens::gallery(Some(drawer_menu));
        shots.shot(&model)?;
        screens::gallery(Some(art));
        shots.page.invalidate();
        shots.page.show(&model)?;
        for _ in 0..6 {
            shots.frame()?;
        }
        png(&out.join("parts-leaving.png"), 1920, 1080, &shots.engine.read_page(0))?;
        println!("{}", out.join("parts-leaving.png").display());
    }
    screens::gallery(None);
    Ok(())
}
