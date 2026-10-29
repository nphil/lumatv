//! The input picker (`docs/mocks/08-input-picker`, `panels/InputPicker.qml` of the QML shell): a sheet along the bottom with one tile for
//! Live TV and one for each HDMI port the owner has not hidden. Each tile shows the port, a device icon chosen from the name the device
//! gave itself over HDMI-CEC, that name, and a state line: connected, in front. The input the sound goes out of is marked, and the sound
//! output is named on the right of the head.
//!
//! OK on a tile launches that input's app (`com.webos.app.hdmiN`, `com.webos.app.livetv`); pressing the input key again, or Right,
//! goes on to the next tile; Back closes. The tile of the input in front is chosen first, so OK on opening keeps what is on.

use super::env::Env;
use super::life::Life;
use super::{K, Out};
use crate::host::inputs::Tile;
use crate::kit::screen::Effect;
use serde_json::{Value, json};

/// How long after it opens the picker waits to ask the TV for its readings: after the slide and the burst of frames the key started.
pub const READ_AFTER_MS: u64 = 200;

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub up: bool,
    pub title: &'static str,
    pub sound: String,
    pub tiles: Vec<Tile>,
    pub index: usize,
    pub over_video: bool,
}

pub struct Picker {
    pub life: Life,
    index: usize,
    /// A key has moved the choice since the picker came up.
    moved: bool,
    read_due: Option<u64>,
}

impl Default for Picker {
    fn default() -> Self {
        Self::new()
    }
}

/// "Sound: Denon on HDMI 2" once the receiver is found among the inputs, else only that the sound goes out over eARC.
pub fn sound_line(env: &Env, tiles: &[Tile]) -> String {
    if env.sio.text("sound.soundOutput") != "external_arc" {
        return if env.sio.has("sound.soundOutput") { "Sound: TV speakers".into() } else { String::new() };
    }
    match tiles.iter().find(|t| t.sound) {
        Some(t) => {
            let word: String = t.name.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            let who = if word.is_empty() { "Receiver".to_string() } else { word };
            format!("Sound: {who} on {}", t.port.split(" \u{b7} ").next().unwrap_or(""))
        }
        None => "Sound: receiver on eARC".into(),
    }
}

impl Picker {
    pub fn new() -> Self {
        Self { life: Life::default(), index: 0, moved: false, read_due: None }
    }

    pub fn tiles(env: &Env) -> Vec<Tile> {
        env.inputs.tiles(env.theme.get("inputs").unwrap_or(&Value::Null))
    }

    fn pick_front(&mut self, tiles: &[Tile]) {
        self.index = tiles.iter().position(|t| t.front).unwrap_or(0);
    }

    /// The picker is wanted: the tile of the input in front is chosen first; the readings wait for the slide to be over.
    pub fn open(&mut self, env: &mut Env, now: u64) -> Out {
        self.moved = false;
        let tiles = Self::tiles(env);
        self.pick_front(&tiles);
        self.read_due = Some(now + u64::from(self.life.enter_ms) + READ_AFTER_MS);
        env.inputs.watch(true, now);
        Out::default()
    }

    pub fn close(&mut self, env: &mut Env, now: u64) -> Out {
        self.read_due = None;
        env.inputs.watch(false, now);
        Out::default()
    }

    /// The inputs changed while the picker is up: if the input in front turns out to be another and no key has moved the choice, the
    /// choice follows it; a list that got shorter keeps the choice inside it.
    pub fn sync(&mut self, env: &Env) {
        if !self.life.presented() {
            return;
        }
        let tiles = Self::tiles(env);
        if !self.moved {
            self.pick_front(&tiles);
        }
        self.index = self.index.min(tiles.len().saturating_sub(1));
    }

    /// The input key again: on to the next tile (it wraps).
    pub fn next(&mut self, env: &Env) {
        self.moved = true;
        let n = Self::tiles(env).len();
        self.index = if n == 0 { 0 } else { (self.index + 1) % n };
    }

    pub fn key(&mut self, k: K, env: &Env) -> Out {
        let tiles = Self::tiles(env);
        if k == K::Back {
            return Out::close();
        }
        self.moved = true;
        match k {
            K::Left => self.index = self.index.saturating_sub(1),
            K::Right => self.index = (self.index + 1).min(tiles.len().saturating_sub(1)),
            K::Ok => return self.activate(&tiles, self.index),
            _ => {}
        }
        Out::default()
    }

    fn activate(&mut self, tiles: &[Tile], i: usize) -> Out {
        let Some(t) = tiles.get(i) else { return Out::default() };
        let mut out = Out::effects(vec![Effect::Luna { tag: super::env::tags::LAUNCH, uri: "luna://com.webos.applicationManager/launch".into(), payload: json!({"id": t.app_id}), subscribe: false }]);
        out.close = true;
        out
    }

    /// A click on tile `i`.
    pub fn click(&mut self, i: usize, env: &Env) -> Out {
        self.moved = true;
        self.index = i;
        self.activate(&Self::tiles(env), i)
    }

    pub fn tick(&mut self, env: &mut Env, now: u64) -> Out {
        let mut out = Out::default();
        if self.read_due.is_some_and(|d| now >= d) {
            self.read_due = None;
            out.effects.extend(env.inputs.refresh());
            out.effects.extend(env.sio.read("sound", &["soundOutput"], None, now));
        }
        out
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.read_due, self.life.due_at()].into_iter().flatten().min()
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn view(&self, env: &Env) -> View {
        let tiles = Self::tiles(env);
        View { up: self.life.presented(), title: "Inputs", sound: sound_line(env, &tiles), index: self.index.min(tiles.len().saturating_sub(1)), tiles, over_video: !env.fg.home }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::screen::Input;

    fn tv(choices: Value) -> Env {
        let mut env = Env::new(json!({"inputs": choices}));
        env.inputs.parse(&json!({"devices": [
            {"appId": "com.webos.app.hdmi1", "label": "Apple TV 4K", "connected": true},
            {"appId": "com.webos.app.hdmi2", "label": "Denon AVR-S760H", "connected": true},
            {"appId": "com.webos.app.hdmi3", "label": "PlayStation 5", "connected": true},
            {"appId": "com.webos.app.hdmi4", "label": "", "connected": false},
        ]}));
        env.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.hdmi2", "home": false}) }, 0);
        env
    }

    fn opened(env: &mut Env) -> Picker {
        let mut p = Picker::new();
        p.life.show(0);
        p.open(env, 0);
        p
    }

    #[test]
    fn it_opens_on_the_input_in_front_and_ok_launches_the_one_picked() {
        let mut env = tv(json!({}));
        let mut p = opened(&mut env);
        assert_eq!(p.index(), 2, "the front input is HDMI 2 (index 2: Live TV, HDMI 1, HDMI 2)");
        p.key(K::Right, &env);
        let out = p.key(K::Ok, &env);
        assert_eq!(out.effects, vec![Effect::Luna { tag: super::super::env::tags::LAUNCH, uri: "luna://com.webos.applicationManager/launch".into(), payload: json!({"id": "com.webos.app.hdmi3"}), subscribe: false }]);
        assert!(out.close, "launching closes the picker");
    }

    #[test]
    fn right_and_left_hold_at_the_ends_and_back_closes_without_launching() {
        let mut env = tv(json!({}));
        let mut p = opened(&mut env);
        for _ in 0..10 {
            p.key(K::Right, &env);
        }
        assert_eq!(p.index(), 3);
        for _ in 0..10 {
            p.key(K::Left, &env);
        }
        assert_eq!(p.index(), 0);
        let out = p.key(K::Back, &env);
        assert!(out.close && out.effects.is_empty());
    }

    #[test]
    fn the_input_key_again_goes_on_to_the_next_tile_and_round_the_end() {
        let mut env = tv(json!({}));
        let mut p = opened(&mut env);
        let n = Picker::tiles(&env).len();
        assert_eq!((p.index(), n), (2, 4));
        p.next(&env);
        assert_eq!(p.index(), 3);
        p.next(&env);
        assert_eq!(p.index(), 0, "it wraps");
        for _ in 0..n {
            p.next(&env);
        }
        assert_eq!(p.index(), 0, "going round all the tiles comes back to the same one");
    }

    #[test]
    fn the_choice_follows_the_input_in_front_until_a_key_has_moved_it() {
        let mut env = tv(json!({}));
        let mut p = opened(&mut env);
        env.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.hdmi3", "home": false}) }, 10);
        p.sync(&env);
        assert_eq!(p.index(), 3, "no key has moved it: it follows");
        p.key(K::Left, &env);
        env.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.hdmi1", "home": false}) }, 20);
        p.sync(&env);
        assert_eq!(p.index(), 2, "after a key the choice is the person's");
    }

    #[test]
    fn the_readings_wait_for_the_slide_and_then_ask_the_inputs_and_the_sound_output_once() {
        let mut env = tv(json!({}));
        let mut p = opened(&mut env);
        assert!(p.tick(&mut env, 309).effects.is_empty());
        let out = p.tick(&mut env, 310);
        let uris: Vec<String> = out.effects.iter().filter_map(|e| if let Effect::Luna { uri, .. } = e { Some(uri.clone()) } else { None }).collect();
        assert!(uris.iter().any(|u| u.ends_with("/getAllInputStatus")) && uris.iter().any(|u| u.ends_with("/getSystemSettings")), "{uris:?}");
        assert!(p.tick(&mut env, 400).effects.is_empty());
    }

    #[test]
    fn the_sound_line_names_the_receiver_and_its_port_or_the_tv_speakers() {
        let mut env = tv(json!({}));
        let tiles = Picker::tiles(&env);
        assert_eq!(sound_line(&env, &tiles), "", "nothing is said before the service has");
        let reads = env.sio.read("sound", &["soundOutput"], None, 0);
        let Some(Effect::Luna { tag, .. }) = reads.first() else { panic!() };
        env.update(&Input::Luna { tag: *tag, payload: json!({"settings": {"soundOutput": "tv_speaker"}}) }, 1);
        assert_eq!(sound_line(&env, &tiles), "Sound: TV speakers");
        env.sio.write("sound.soundOutput", &json!("external_arc"), None);
        assert_eq!(sound_line(&env, &tiles), "Sound: Denon on HDMI 2");
        let none = tv(json!({"hdmi2": "hidden"}));
        let mut env2 = none;
        env2.sio.write("sound.soundOutput", &json!("external_arc"), None);
        assert_eq!(sound_line(&env2, &Picker::tiles(&env2)), "Sound: receiver on eARC");
    }

    #[test]
    fn the_owners_choices_decide_the_tiles_and_a_click_launches_that_one() {
        let mut env = tv(json!({"hdmi4": "always", "hdmi3": "hidden", "livetv": "hidden"}));
        let mut p = opened(&mut env);
        let v = p.view(&env);
        assert_eq!(v.tiles.iter().map(|t| t.port.as_str()).collect::<Vec<_>>(), vec!["HDMI 1", "HDMI 2 \u{b7} eARC", "HDMI 4"]);
        assert_eq!(v.tiles[2].icon, "unplug");
        let out = p.click(0, &env);
        assert!(matches!(&out.effects[0], Effect::Luna { payload, .. } if payload["id"] == "com.webos.app.hdmi1"));
        assert!(out.close);
    }
}
