//! What Home draws around itself, drawn ([`super::extras`] is the model): the launch screen, the Quick Access toast and question, the on-screen
//! remote, the push-to-talk voice bar and the mini frame. They are Home's own, so they hang in Home's stack (below `system_z`): over Home and its
//! layers, under every panel and under the windows of the apps the compositor puts over Home (the mini frame's web half paints into the room its
//! frame leaves clear). One surface per screen, each made when its view says it is up and dropped when it goes.

mod access;
mod frame;
mod remote;
mod splash;
pub mod spinner;
mod voice;

use super::extras::{SPIN_MS, Tile, View};
use crate::kit::pointer::Sample;
use access::{AccessAsk, AccessToast};
use frame::FrameSurface;
use remote::RemoteSurface;
use voice::VoiceSurface;
use crate::kit::lint::Lint;
use crate::kit::widgets::tile;
use crate::kit::{Engine, Kit, avatar};
use compositor_protocol::Props;
use serde_json::{Value, json};
use splash::Splash;
use ui::scene::ImageLayer;

/// The depths of Home's stack the extras take: over the layers of Home (`home::surface::Z_LAYERS` and the 90 above it).
pub mod z {
    pub const SPLASH: i32 = 300;
    pub const TOAST: i32 = 320;
    pub const ASK: i32 = 340;
    pub const REMOTE: i32 = 380;
    pub const VOICE: i32 = 420;
    pub const FRAME: i32 = 460;
}

/// What a click of the pointer landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// A card of the question (the apps, then More), or its Not now button.
    Pick(usize),
    Later,
    /// A key of the on-screen remote.
    Key(usize),
}

/// Where the ring's head is at `now_ms`: a step every [`SPIN_MS`].
pub fn spin_head(now_ms: u64) -> usize {
    (now_ms / SPIN_MS) as usize
}

/// An app's icon of `side` with its corners rounded by `radius`: the picture of the launch point when there is one, else the plain mark of an app.
pub fn app_icon(kit: &mut Kit, file: &str, side: u32, radius: f32, z: i32, at: Props) -> std::io::Result<ImageLayer> {
    if let Some(mut p) = avatar::contained(file, side, side) {
        p.round_corners(radius);
        let mut layer = kit.ui.scene.image(p.width, p.height, z, at)?;
        p.paint(&mut layer);
        return Ok(layer);
    }
    tile(kit, side, "layout-grid", false).layer(kit, z, at)
}

pub struct ExtrasSurface {
    splash: Splash,
    toast: AccessToast,
    ask: AccessAsk,
    remote: RemoteSurface,
    voice: VoiceSurface,
    frame: FrameSurface,
    interactive_visible: bool,
}

impl ExtrasSurface {
    pub fn new() -> Self {
        Self { splash: Splash::new(z::SPLASH), toast: AccessToast::new(z::TOAST), ask: AccessAsk::new(z::ASK), remote: RemoteSurface::new(z::REMOTE), voice: VoiceSurface::new(z::VOICE), frame: FrameSurface::new(z::FRAME), interactive_visible: false }
    }

    /// Whether anything of the extras is on the screen.
    pub fn is_up(&self) -> bool {
        self.splash.is_up() || self.toast.is_up() || self.ask.is_up() || self.remote.is_up() || self.voice.is_up() || self.frame.is_up()
    }

    pub(crate) fn interactive_visible(&self) -> bool { self.interactive_visible }

    /// Whether a screen that takes the pointer is up (the pointer is not Home's while it is).
    pub fn has_pointer(&self) -> bool {
        self.ask.is_up() || self.remote.is_up()
    }

    /// Where the launch screen's icon is, once it is drawn: what the compositor's card grows from.
    pub fn splash_icon(&self) -> Option<Tile> {
        self.splash.icon_rect()
    }

    /// Fits the screens to the view. Returns whether anything of them is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, now_ms: u64, now_ns: u64) -> Result<bool, String> {
        let mut live = false;
        match self.splash.apply(kit, view.splash.as_ref(), now_ms) {
            Ok(on) => live |= on,
            Err(e) => { self.interactive_visible |= live; return Err(format!("launch screen: {e}")); },
        }
        match self.toast.apply(kit, view.toast.as_ref(), now_ms) {
            Ok(on) => live |= on,
            Err(e) => { self.interactive_visible |= live; return Err(format!("Quick Access toast: {e}")); },
        }
        match self.ask.apply(kit, engine, view.ask.as_ref(), now_ns) {
            Ok(on) => live |= on,
            Err(e) => { self.interactive_visible |= live; return Err(format!("Quick Access question: {e}")); },
        }
        match self.remote.apply(kit, engine, view.remote, now_ns) {
            Ok(on) => live |= on,
            Err(e) => { self.interactive_visible |= live; return Err(format!("on-screen remote: {e}")); },
        }
        match self.voice.apply(kit, view.voice.as_ref(), view.mic, now_ms) {
            Ok(on) => live |= on,
            Err(e) => { self.interactive_visible |= live || view.voice.is_some() || view.mic; return Err(format!("voice bar: {e}")); },
        }
        match self.frame.apply(kit, view.frame.as_ref(), now_ms) {
            Ok(on) => live |= on,
            Err(e) => { self.interactive_visible |= live; return Err(format!("mini frame: {e}")); },
        }
        self.interactive_visible = live;
        Ok(live)
    }

    /// The timers of the surfaces: the rings step in place (nothing is made again) and what faded out is dropped. Returns whether a redraw is wanted.
    pub fn tick(&mut self, kit: &Kit, now_ms: u64) -> bool {
        self.splash.tick(now_ms, kit.ui.look.reduced_motion);
        let (voice, frame) = (self.voice.tick(kit, now_ms), self.frame.tick(kit, now_ms));
        self.toast.tick(now_ms) | voice | frame
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.toast.due_at(), self.voice.due_at(), self.frame.due_at()].into_iter().flatten().min()
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.ask.on_key(kit, engine);
        self.remote.on_key(engine);
    }

    /// A sample of the pointer: the cue follows it over what takes it, and a click is what it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Hit> {
        if self.ask.is_up() {
            return match self.ask.pointer(kit, engine, now_ns, s)? {
                access::Target::Pick(i) => Some(Hit::Pick(i)),
                access::Target::Later => Some(Hit::Later),
                access::Target::Backdrop => None,
            };
        }
        if self.remote.is_up() {
            return self.remote.pointer(kit, engine, now_ns, s).map(Hit::Key);
        }
        None
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.splash.restyle();
        self.toast.restyle();
        self.ask.restyle(engine);
        self.remote.restyle(engine);
        self.voice.restyle();
        self.frame.restyle();
    }

    pub fn geometry(&self, engine: &Engine) -> Value {
        json!({"splash": self.splash.geometry(), "toast": self.toast.geometry(), "ask": self.ask.geometry(engine), "remote": self.remote.geometry(engine), "voice": self.voice.geometry(), "frame": self.frame.geometry()})
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        self.splash.feed_lint(kit, lint);
        self.toast.feed_lint(kit, lint);
        self.ask.feed_lint(kit, lint);
        self.remote.feed_lint(kit, lint);
        self.voice.feed_lint(kit, lint);
        self.frame.feed_lint(kit, lint);
    }
}

impl Default for ExtrasSurface {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::system::extras::{SplashView, VoiceState};
    use ui::lucent::palette::Mode;

    struct Up {
        _h: ui::testing::Harness,
        kit: Kit,
        engine: Engine,
        s: ExtrasSurface,
    }

    impl Up {
        fn new(mode: Mode, scale: f32) -> Self {
            let (h, kit) = testkit::kit_in(mode, scale);
            let engine = Engine::new(&kit).expect("an engine");
            Self { _h: h, kit, engine, s: ExtrasSurface::new() }
        }

        fn apply(&mut self, view: &View, now_ms: u64) -> bool {
            self.s.apply(&mut self.kit, &mut self.engine, view, now_ms, now_ms * 1_000_000).expect("the extras are drawn")
        }

        fn lint(&mut self) -> Vec<String> {
            let mut lint = Lint::new();
            self.s.feed_lint(&mut self.kit, &mut lint);
            lint.findings().iter().map(ToString::to_string).collect()
        }
    }

    fn launching(name: &str, fade: bool) -> View {
        View { splash: Some(SplashView { name: name.into(), icon: "/no/such/icon.png".into(), fade }), ..View::default() }
    }

    #[test]
    fn successful_splash_is_reported_when_a_later_toast_cannot_be_allocated() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let mut view = launching("Netflix", false);
        assert!(u.s.splash.apply(&mut u.kit, view.splash.as_ref(), 1000).unwrap());
        assert!(!u.s.interactive_visible());
        let count = u.kit.room().layers;
        let shape = compositor_protocol::Shape { corner_radius: 0.0, fill: compositor_protocol::Fill::Solid(compositor_protocol::Rgba(0, 0, 0, 255)), border_width: 0.0, border: compositor_protocol::Rgba(0, 0, 0, 0) };
        let blockers: Vec<_> = (0..count).map(|_| u.kit.ui.scene.shape(1, 1, 0, shape, Props::at(0.0, 0.0))).collect();
        view.toast = Some(crate::system::extras::Toast { digit: "4".into(), title: "Plex".into(), line: "Opening".into() });
        assert!(u.s.apply(&mut u.kit, &mut u.engine, &view, 1100, 1_100_000_000).is_err());
        assert!(u.s.interactive_visible(), "the earlier successful splash remains visible");
        drop(blockers);
        assert!(u.apply(&View::default(), 1200), "the toast root is still leaving");
        assert!(u.s.interactive_visible());
        let due = u.s.due_at().expect("the toast schedules its removal");
        assert!(u.s.tick(&u.kit, due));
        assert!(!u.apply(&View::default(), due));
        assert!(!u.s.interactive_visible());
    }

    #[test]
    fn voice_bar_retains_pacing_demand_when_mic_mark_allocation_fails() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let view = speaking(VoiceState::Listening, true);
        assert!(u.s.voice.apply(&mut u.kit, view.voice.as_ref(), false, 1000).unwrap());
        assert!(!u.s.interactive_visible());
        let count = u.kit.room().groups;
        let _blockers: Vec<_> = (0..count).map(|_| u.kit.ui.scene.group(None, compositor_protocol::GroupPose::IDENTITY).unwrap()).collect();
        let error = u.s.apply(&mut u.kit, &mut u.engine, &view, 1100, 1_100_000_000).expect_err("the mic mark runs out of groups");
        assert!(error.starts_with("voice bar:"));
        assert!(u.s.voice.is_up(), "the bar was already shown");
        assert!(u.s.interactive_visible(), "a failed mic mark must not discard the bar's demand");
    }

    #[test]
    fn the_launch_screen_has_the_icon_in_the_middle_the_name_under_it_and_the_ring_beside_the_word_opening() {
        let mut u = Up::new(Mode::Dark, 1.0);
        assert!(u.apply(&launching("Netflix", false), 1000));
        let g = u.s.geometry(&u.engine)["splash"].clone();
        assert_eq!(g["icon"]["w"], 180.0);
        assert_eq!(g["icon"]["x"], 870.0, "centred");
        assert_eq!(g["radius"], 38.0);
        assert_eq!(g["name"], "Netflix");
        assert_eq!(g["caption"], "Opening");
        let (icon_y, ring_y) = (g["icon"]["y"].as_f64().unwrap(), g["ring"]["y"].as_f64().unwrap());
        assert!(icon_y > 200.0 && ring_y > icon_y + 180.0 + 32.0, "the icon, then the name, then the row: {g}");
        assert_eq!(g["ring"]["w"], 28.0);
        let centre = icon_y + 90.0 + (ring_y + 14.0 - icon_y - 90.0) / 2.0;
        assert!((centre - 540.0).abs() < 40.0, "the whole block sits in the middle of the screen: {centre}");
        let tile = u.s.splash_icon().expect("the icon's place");
        assert_eq!((tile.w, tile.h, tile.radius), (180.0, 180.0, 38.0));
    }

    #[test]
    fn the_ring_steps_round_in_place_and_a_launch_that_is_over_takes_every_layer_with_it() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.apply(&launching("Plex", false), 1000);
        let layers = u.kit.room().layers;
        let head = u.s.geometry(&u.engine)["splash"]["head"].as_u64().unwrap();
        u.s.tick(&u.kit, 1000 + SPIN_MS);
        assert_eq!(u.s.geometry(&u.engine)["splash"]["head"].as_u64().unwrap(), (head + 1) % 12, "a step of the ring every {SPIN_MS} ms");
        assert_eq!(u.kit.room().layers, layers, "the ring is stepped, nothing is made");
        u.apply(&View::default(), 2000);
        assert!(!u.s.is_up() && u.s.splash_icon().is_none());
        assert!(u.kit.room().layers > layers, "the layers are given back");
    }

    #[test]
    fn the_launch_screen_reads_clean_in_both_materials_at_100_and_200_percent_with_a_long_name() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let mut u = Up::new(mode, scale);
                u.apply(&launching("A rather long name of an app that goes on and on beyond what a title should hold on a screen of this size and then some", true), 1000);
                let f = u.lint();
                assert!(f.is_empty(), "{mode:?} {scale}: {f:#?}");
            }
        }
    }

    fn pick(id: &str, title: &str) -> crate::system::extras::Pick {
        crate::system::extras::Pick { id: id.into(), title: title.into(), icon: "/no/such/icon.png".into() }
    }

    fn asking(index: usize) -> View {
        let picks = vec![pick("a", "Plex"), pick("b", "Homebrew Channel"), pick("c", "LG Movies"), pick("d", "A rather longer name that wraps")];
        View { ask: Some(crate::system::extras::Ask { digit: "4".into(), picks, index }), ..View::default() }
    }

    fn at(x: f32, y: f32, b: u32) -> Sample {
        Sample { x, y, buttons: b, visible: true }
    }

    #[test]
    fn the_toast_is_a_pill_at_the_top_right_with_the_digit_on_a_plate_and_goes_after_a_fade() {
        let mut u = Up::new(Mode::Dark, 1.0);
        let toast = crate::system::extras::Toast { digit: "4".into(), title: "Plex".into(), line: "Opening.".into() };
        let on = View { toast: Some(toast), ..View::default() };
        assert!(u.apply(&on, 1000));
        let g = u.s.geometry(&u.engine)["toast"].clone();
        assert_eq!(g["y"], 137.0);
        assert_eq!(g["x"].as_f64().unwrap() + g["w"].as_f64().unwrap(), 1856.0, "64 from the right edge");
        assert!(g["w"].as_f64().unwrap() >= 560.0);
        assert_eq!((g["title"].as_str(), g["line"].as_str(), g["digit"].as_str()), (Some("4 \u{b7} Plex"), Some("Opening."), Some("4")));
        u.apply(&View::default(), 3400);
        assert!(u.s.is_up(), "it fades out first");
        assert!(u.s.due_at().is_some());
        assert!(u.s.tick(&u.kit, 3400 + 400), "and is dropped when the fade is over");
        u.apply(&View::default(), 3800);
        assert!(!u.s.is_up());
    }

    #[test]
    fn the_question_has_the_apps_and_more_as_cards_in_one_row_and_the_keys_as_glyphs_and_the_focus_follows_the_model() {
        let mut u = Up::new(Mode::Dark, 1.0);
        assert!(u.apply(&asking(0), 1000));
        let g = u.s.geometry(&u.engine)["ask"].clone();
        assert_eq!(g["card"]["w"], 820.0);
        assert_eq!(g["card"]["x"].as_f64().unwrap() + g["card"]["w"].as_f64().unwrap() / 2.0, 960.0, "in the middle");
        let picks = g["picks"].as_array().unwrap();
        assert_eq!(picks.len(), 5, "four apps and More");
        assert!(picks.iter().all(|p| p["w"] == 140.0 && p["y"] == picks[0]["y"] && p["h"].as_f64().unwrap() >= 150.0), "{picks:?}");
        assert_eq!(g["title"], json!(["Nothing on 4 yet"]));
        assert_eq!(g["hints"], json!(["Left and Right, move", "OK, confirm", "Back, close"]), "glyphs for the keys, no sentence");
        assert_eq!(g["focus"], 0);
        assert_eq!(u.engine.focus.modal(), Some(access::SCOPE), "the keys are the question's");
        u.apply(&asking(4), 1100);
        assert_eq!(u.s.geometry(&u.engine)["ask"]["focus"], 4, "More");
        u.apply(&asking(5), 1200);
        assert_eq!(u.s.geometry(&u.engine)["ask"]["focus"], "later", "Not now");
        let layers = u.kit.room().layers;
        u.apply(&asking(2), 1300);
        assert_eq!(u.kit.room().layers, layers, "the focus is the flowing lift: nothing is made");
        u.apply(&View::default(), 1400);
        assert!(!u.s.is_up() && u.engine.focus.modal().is_none(), "answered: the layers and the keys are given back");
    }

    #[test]
    fn a_click_is_the_card_or_the_button_it_ended_on_hover_is_only_a_cue_and_the_dim_answers_nothing() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.apply(&asking(0), 1000);
        let g = u.s.geometry(&u.engine)["ask"].clone();
        let (card, later) = (&g["picks"][2], &g["later"]);
        let centre = |f: &Value| (f["x"].as_f64().unwrap() as f32 + f["w"].as_f64().unwrap() as f32 / 2.0, f["y"].as_f64().unwrap() as f32 + f["h"].as_f64().unwrap() as f32 / 2.0);
        let t = |n: u64| (5000 + n) * 1_000_000;
        let (x, y) = centre(card);
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(0), at(x, y, 0)), None);
        assert_eq!(u.s.geometry(&u.engine)["ask"]["focus"], 0, "hover moves no focus");
        u.s.pointer(&u.kit, &mut u.engine, t(10), at(x, y, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(20), at(x, y, 0)), Some(Hit::Pick(2)));
        let (x, y) = centre(later);
        u.s.pointer(&u.kit, &mut u.engine, t(30), at(x, y, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(40), at(x, y, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(50), at(x, y, 0)), Some(Hit::Later));
        u.s.pointer(&u.kit, &mut u.engine, t(60), at(20.0, 20.0, 0));
        u.s.pointer(&u.kit, &mut u.engine, t(70), at(20.0, 20.0, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(80), at(20.0, 20.0, 0)), None, "a click on the dim is no answer");
    }

    #[test]
    fn the_toast_and_the_question_read_clean_in_both_materials_at_100_and_200_percent() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let mut u = Up::new(mode, scale);
                let toast = crate::system::extras::Toast { digit: "0".into(), title: "A very long name of an app that has to be cut somewhere".into(), line: "Assigned. Opening.".into() };
                u.apply(&View { toast: Some(toast), ..View::default() }, 1000);
                let f = u.lint();
                assert!(f.is_empty(), "toast {mode:?} {scale}: {f:#?}");
                // the question hides the page, the toast included: the model takes the toast away when it asks
                let mut u = Up::new(mode, scale);
                u.apply(&asking(1), 1100);
                let f = u.lint();
                assert!(f.is_empty(), "question {mode:?} {scale}: {f:#?}");
                let card = u.s.geometry(&u.engine)["ask"]["card"].clone();
                assert!(card["y"].as_f64().unwrap() >= 54.0 && card["y"].as_f64().unwrap() + card["h"].as_f64().unwrap() <= 1080.0 - 54.0, "inside the safe area: {card}");
            }
        }
    }

    fn remoting(i: usize) -> View {
        View { remote: Some(i), ..View::default() }
    }

    #[test]
    fn the_remote_is_a_sheet_along_the_foot_with_every_key_of_the_model_and_none_that_could_answer_a_prompt() {
        let mut u = Up::new(Mode::Dark, 1.0);
        assert!(u.apply(&remoting(0), 1000));
        let g = u.s.geometry(&u.engine)["remote"].clone();
        assert_eq!((g["sheet"]["x"].as_f64(), g["sheet"]["w"].as_f64(), g["sheet"]["h"].as_f64()), (Some(64.0), Some(1792.0), Some(360.0)));
        assert_eq!(g["sheet"]["y"].as_f64().unwrap() + 360.0, 1016.0, "64 from the foot");
        let names: Vec<&str> = g["keys"].as_array().unwrap().iter().map(|k| k["name"].as_str().unwrap()).collect();
        assert_eq!(names, crate::system::extras::REMOTE_KEYS.iter().map(|k| k.name).collect::<Vec<_>>());
        for banned in ["ok", "enter", "up", "down", "left", "right"] {
            assert!(!names.contains(&banned), "{banned}: nothing on the sheet can say the keys of a prompt (Principle 0)");
        }
        assert_eq!(g["hints"], json!(["Up and Down and Left and Right, move", "OK, confirm", "Back, close"]), "glyphs, not a sentence");
        assert_eq!(g["focus"], "rewind");
        assert_eq!(u.engine.focus.modal(), Some(remote::SCOPE));
        // the keys are where the model's grid puts them: the pad's rows of three, the colours at the right
        let at = |n: &str| g["keys"].as_array().unwrap().iter().find(|k| k["name"] == n).unwrap()["frame"].clone();
        assert!(at("1")["x"].as_f64().unwrap() < at("2")["x"].as_f64().unwrap() && at("1")["y"].as_f64().unwrap() < at("4")["y"].as_f64().unwrap());
        assert!(at("red")["x"].as_f64().unwrap() > at("9")["x"].as_f64().unwrap() && at("blue")["x"].as_f64().unwrap() > at("red")["x"].as_f64().unwrap());
        assert_eq!(at("play")["h"], 88.0);
        assert_eq!(at("5")["h"], 54.0);
    }

    #[test]
    fn the_focus_of_the_remote_follows_the_model_a_click_is_the_key_it_ended_on_and_closing_gives_everything_back() {
        let mut u = Up::new(Mode::Dark, 1.0);
        u.apply(&remoting(0), 1000);
        let layers = u.kit.room().layers;
        u.apply(&remoting(6), 1100);
        assert_eq!(u.s.geometry(&u.engine)["remote"]["focus"], "2");
        assert_eq!(u.kit.room().layers, layers, "the focus is the flowing lift: nothing is made");
        let key = u.s.geometry(&u.engine)["remote"]["keys"][1]["frame"].clone();
        let (x, y) = ((key["x"].as_f64().unwrap() + 20.0) as f32, (key["y"].as_f64().unwrap() + 20.0) as f32);
        let t = |n: u64| (6000 + n) * 1_000_000;
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(0), at(x, y, 0)), None, "hover is a cue");
        assert_eq!(u.s.geometry(&u.engine)["remote"]["focus"], "2");
        u.s.pointer(&u.kit, &mut u.engine, t(10), at(x, y, 1));
        assert_eq!(u.s.pointer(&u.kit, &mut u.engine, t(20), at(x, y, 0)), Some(Hit::Key(1)), "play");
        u.apply(&View::default(), 1200);
        assert!(!u.s.is_up() && u.engine.focus.modal().is_none());
    }

    #[test]
    fn the_remote_reads_clean_in_both_materials_at_100_and_200_percent_and_the_pad_grows_with_its_words() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let mut u = Up::new(mode, scale);
                u.apply(&remoting(14), 1000);
                let f = u.lint();
                assert!(f.is_empty(), "{mode:?} {scale}: {f:#?}");
                let g = u.s.geometry(&u.engine)["remote"].clone();
                let sheet = &g["sheet"];
                assert!(sheet["y"].as_f64().unwrap() >= 54.0 && sheet["x"].as_f64().unwrap() + sheet["w"].as_f64().unwrap() <= 1856.0, "inside the safe area: {sheet}");
                let last = g["keys"].as_array().unwrap().last().unwrap()["frame"].clone();
                assert!(last["x"].as_f64().unwrap() + last["w"].as_f64().unwrap() <= 1856.0 - 32.0 + 0.5, "the colours stay on the sheet: {last}");
                if scale > 1.5 {
                    assert!(sheet["h"].as_f64().unwrap() > 360.0, "taller at 200 %: {sheet}");
                }
            }
        }
    }

    fn speaking(state: VoiceState, mic: bool) -> View {
        View { voice: Some(crate::system::extras::VoiceView { state }), mic, ..View::default() }
    }

    #[test]
    fn the_voice_bar_is_at_the_foot_with_the_mic_on_a_tile_and_the_key_to_hold_as_a_glyph_and_the_mark_shows_while_the_microphone_is_open() {
        let mut u = Up::new(Mode::Dark, 1.0);
        assert!(u.apply(&speaking(VoiceState::Listening, true), 1000));
        let g = u.s.geometry(&u.engine)["voice"].clone();
        let bar = &g["bar"];
        assert_eq!((bar["frame"]["x"].as_f64(), bar["frame"]["w"].as_f64()), (Some(64.0), Some(760.0)));
        assert_eq!(bar["frame"]["y"].as_f64().unwrap() + bar["frame"]["h"].as_f64().unwrap(), 1024.0, "56 from the foot");
        assert_eq!((bar["caption"].as_str(), bar["title"][0].as_str(), bar["listening"].as_bool()), (Some("Listening"), Some("Speak now\u{2026}"), Some(true)));
        assert_eq!(bar["tile"]["w"], 72.0);
        assert_eq!((g["mark"]["x"].as_f64().unwrap() + 72.0, g["mark"]["y"].as_f64().unwrap() + 72.0), (1856.0, 1024.0), "64 from the right, 56 from the foot");
        // the state changes in place: the bar does not fade in again and nothing is made
        let layers = u.kit.room().layers;
        u.apply(&speaking(VoiceState::Working, true), 1500);
        let g = u.s.geometry(&u.engine)["voice"].clone();
        assert_eq!((g["bar"]["caption"].as_str(), g["bar"]["title"][0].as_str(), g["bar"]["working"].as_bool()), (Some("Working it out"), Some("One moment\u{2026}"), Some(true)));
        assert_eq!(u.kit.room().layers, layers);
        // the mic closed (the words came back): the mark fades out and goes, the bar says why when it failed
        u.apply(&speaking(VoiceState::Failed("Voice search is switched off".into()), false), 2000);
        let g = u.s.geometry(&u.engine)["voice"].clone();
        assert_eq!((g["bar"]["caption"].as_str(), g["bar"]["title"][0].as_str()), (Some("Voice search"), Some("Voice search is switched off")));
        assert!(u.s.tick(&u.kit, 2000 + 400));
        u.apply(&speaking(VoiceState::Failed("Voice search is switched off".into()), false), 2500);
        assert!(u.s.geometry(&u.engine)["voice"]["mark"].is_null(), "the microphone is shut: no mark");
        u.apply(&View::default(), 2600);
        u.s.tick(&u.kit, 3000);
        u.apply(&View::default(), 3000);
        assert!(!u.s.is_up());
    }

    #[test]
    fn the_voice_bar_reads_clean_in_both_materials_at_100_and_200_percent_with_a_long_reason() {
        let long = "The TV\u{2019}s helper is not running, so voice search cannot listen until it is started again";
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                for state in [VoiceState::Listening, VoiceState::Working, VoiceState::Failed(long.into())] {
                    let mut u = Up::new(mode, scale);
                    u.apply(&speaking(state.clone(), true), 1000);
                    let f = u.lint();
                    assert!(f.is_empty(), "{state:?} {mode:?} {scale}: {f:#?}");
                }
            }
        }
    }

    fn page(landed: bool, loading: bool) -> View {
        let f = crate::system::extras::FrameView {
            title: "Denmark says Russian warship fired flares at military helicopter over the Baltic Sea during a routine patrol, officials said on Thursday".into(),
            subtitle: "BBC News \u{b7} 3 h ago".into(),
            from: [668.0, 134.0, 735.0, 480.0],
            to: [200.0, 150.0, 1520.0, 860.0],
            landed,
            loading,
        };
        View { frame: Some(f), ..View::default() }
    }

    #[test]
    fn the_mini_frame_grows_out_of_its_card_to_its_place_with_its_interior_left_clear_and_its_words_come_when_it_has_landed() {
        let mut u = Up::new(Mode::Dark, 1.0);
        assert!(u.apply(&page(false, true), 1000));
        let g = u.s.geometry(&u.engine)["frame"].clone();
        assert_eq!((g["x"].as_f64(), g["y"].as_f64(), g["w"].as_f64(), g["h"].as_f64()), (Some(200.0), Some(150.0), Some(1520.0), Some(860.0)));
        assert_eq!((g["interior"]["x"].as_f64(), g["interior"]["y"].as_f64(), g["interior"]["w"].as_f64(), g["interior"]["h"].as_f64()), (Some(232.0), Some(214.0), Some(1456.0), Some(764.0)), "32 in from the sides and the foot, under a bar of 64");
        assert!(g["title"].is_null(), "no words yet: it is still growing");
        u.apply(&page(true, true), 1300);
        let g = u.s.geometry(&u.engine)["frame"].clone();
        assert!(g["title"].as_str().is_some_and(|t| t.starts_with("Denmark says") && t.ends_with('\u{2026}')), "a long title is cut to the bar: {g}");
        assert_eq!(g["subtitle"], "BBC News \u{b7} 3 h ago");
        assert_eq!(g["ring"]["w"], 64.0);
        let ring = &g["ring"];
        assert_eq!(ring["x"].as_f64().unwrap() + 32.0, 960.0, "the ring is in the middle of the room");
        let layers = u.kit.room().layers;
        u.apply(&page(true, false), 1600);
        assert_eq!(u.kit.room().layers, layers, "the ring goes when the web half is up, in place");
        u.apply(&View::default(), 2000);
        assert!(u.s.is_up(), "it shrinks back first");
        assert!(u.s.tick(&u.kit, 2000 + 400));
        assert!(!u.s.is_up());
    }

    #[test]
    fn the_mini_frame_reads_clean_in_both_materials_at_100_and_200_percent() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let mut u = Up::new(mode, scale);
                u.apply(&page(true, true), 1000);
                let f = u.lint();
                assert!(f.is_empty(), "{mode:?} {scale}: {f:#?}");
            }
        }
    }
}
