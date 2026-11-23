//! The input picker (`docs/mocks/08-input-picker`, `panels/InputPicker.qml` of the QML shell): a sheet along the bottom with one tile for
//! Live TV and one for each HDMI port the owner has not hidden. Each tile shows the port, a device icon chosen from the name the device
//! gave itself over HDMI-CEC, that name, and a state line: connected, in front. The input the sound goes out of is marked, and the sound
//! output is named on the right of the head.
//!
//! OK on a tile launches that input's app (`com.webos.app.hdmiN`, `com.webos.app.livetv`); pressing the input key again, or Right,
//! goes on to the next tile; Back closes. The tile of the input in front is chosen first, so OK on opening keeps what is on.
//!
//! A port with an audio system on it shows the devices behind it (HDMI-CEC) under its tile. With the Denon receiver plugin on, the one the
//! receiver is playing is marked, and Down reaches the devices whose name is an input of the receiver: OK on one launches the port's app
//! when it is not in front and sets the receiver to that input (`/v1/receiver/set`, the way Settings does); Up or Left and Right leave them.
//! With the plugin off the devices are only named.

use super::env::Env;
use super::life::Life;
use super::{K, Out};
use crate::host::inputs::Tile;
use crate::host::previews::Shot;
use crate::kit::screen::Effect;
use serde_json::{Value, json};

/// How long after it opens the picker waits to ask the TV for its readings: after the slide and the burst of frames the key started.
pub const READ_AFTER_MS: u64 = 200;
/// How often an open picker asks for the receiver's settings again (the helper keeps them for five seconds after each ask).
const SETTINGS_EVERY_MS: u64 = 2000;

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub up: bool,
    pub title: &'static str,
    pub sound: String,
    pub tiles: Vec<Tile>,
    pub index: usize,
    pub over_video: bool,
    /// The previews are on: the tiles are the shape of a picture, whether a tile has its picture yet or not.
    pub pictures: bool,
    /// What each tile shows of its picture (tile by tile); none when it has none.
    pub shots: Vec<Option<Shot>>,
    /// The device under the chosen tile that has the focus.
    pub child: Option<usize>,
}

pub struct Picker {
    pub life: Life,
    index: usize,
    /// A key has moved the choice since the picker came up.
    moved: bool,
    read_due: Option<u64>,
    /// The device under the chosen tile that has the focus, in its `children`; none while the tile has it.
    child: Option<usize>,
    /// When the receiver's settings are asked for again: the helper reads them only for five seconds after each ask.
    settings_due: Option<u64>,
}

impl Default for Picker {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the sound goes out now: the audio service's word on it (the way in effect, pushed with every volume state) before the setting's,
/// which can lag behind it or name it otherwise.
fn output(env: &Env) -> String {
    if env.audio_output.is_empty() { env.sio.opt_text("sound.soundOutput").unwrap_or_default() } else { env.audio_output.clone() }
}

/// Whether the sound goes out over HDMI to the audio system on one of the ports (its tile is the one marked). "TV speakers and an external
/// device" is that device when an audio system is on a port.
fn to_receiver(env: &Env) -> bool {
    match output(env).as_str() {
        "external_arc" | "tv_speaker_external_arc" => true,
        "tv_external_speaker" => env.inputs.receiver_name().is_some(),
        _ => false,
    }
}

/// "Sound: Denon AVR-S760H (HDMI ARC)": the way out, and the audio system by the name it gave itself over HDMI-CEC. A way out this does not
/// know is not described: nothing is said rather than the wrong thing.
pub fn sound_line(env: &Env) -> String {
    let receiver = env.inputs.receiver_name();
    let words = match output(env).as_str() {
        "external_arc" => format!("{} (HDMI ARC)", receiver.as_deref().unwrap_or("Receiver")),
        "tv_speaker_external_arc" => format!("TV and {} (HDMI ARC)", receiver.as_deref().unwrap_or("receiver")),
        "tv_external_speaker" => receiver.map_or_else(|| "TV and external speakers".to_string(), |r| format!("TV and {r}")),
        "tv_speaker" => "TV speakers".into(),
        "external_optical" => "Optical out".into(),
        "headphone" => "Headphones".into(),
        "tv_speaker_headphone" => "TV and headphones".into(),
        "bt_soundbar" => "Bluetooth speaker".into(),
        "tv_speaker_bluetooth" => "TV and Bluetooth speaker".into(),
        "lineout" => "Line out".into(),
        "usb_speaker" => "USB speaker".into(),
        "wow_cast" => "Wireless speaker".into(),
        "tv_speaker_wow_cast" => "TV and wireless speaker".into(),
        "wisa_speaker" => "WiSA speakers".into(),
        _ => return String::new(),
    };
    format!("Sound: {words}")
}

impl Picker {
    pub fn new() -> Self {
        Self { life: Life::default(), index: 0, moved: false, read_due: None, child: None, settings_due: None }
    }

    pub fn tiles(env: &Env) -> Vec<Tile> {
        env.inputs.tiles(env.theme.get("inputs").unwrap_or(&Value::Null), to_receiver(env), env.denon_switch().as_ref())
    }

    /// The devices of `tile` OK can switch to.
    pub fn stops(tile: &Tile) -> Vec<usize> {
        tile.children.iter().enumerate().filter(|(_, c)| c.can_switch).map(|(i, _)| i).collect()
    }

    fn pick_front(&mut self, tiles: &[Tile]) {
        self.index = tiles.iter().position(|t| t.front).unwrap_or(0);
    }

    /// The picker is wanted: the tile of the input in front is chosen first; the readings wait for the slide to be over.
    pub fn open(&mut self, env: &mut Env, now: u64) -> Out {
        self.moved = false;
        self.child = None;
        let tiles = Self::tiles(env);
        self.pick_front(&tiles);
        self.read_due = Some(now + u64::from(self.life.enter_ms) + READ_AFTER_MS);
        self.settings_due = self.read_due;
        env.inputs.watch(true, now);
        // the pictures: the index, and one of the input in front at once (the live tile follows); they are decoded once the sheet is in
        env.previews.hold_until(now + u64::from(self.life.enter_ms) + 100);
        let mut effects = env.previews_opened(now);
        effects.extend(env.gate.set_active(true, now));
        Out::effects(effects)
    }

    pub fn close(&mut self, env: &mut Env, now: u64) -> Out {
        self.read_due = None;
        self.settings_due = None;
        self.child = None;
        env.inputs.watch(false, now);
        env.previews_closed(now);
        Out::effects(env.gate.set_active(false, now))
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
        // a device that is no longer one OK can switch to loses the focus to its tile
        if let Some(c) = self.child
            && !tiles.get(self.index).is_some_and(|t| Self::stops(t).contains(&c))
        {
            self.child = None;
        }
    }

    /// The input key again: on to the next tile (it wraps).
    pub fn next(&mut self, env: &Env) {
        self.moved = true;
        self.child = None;
        let n = Self::tiles(env).len();
        self.index = if n == 0 { 0 } else { (self.index + 1) % n };
    }

    pub fn key(&mut self, k: K, env: &Env) -> Out {
        let tiles = Self::tiles(env);
        if k == K::Back {
            return Out::close();
        }
        self.moved = true;
        let stops = tiles.get(self.index).map(Self::stops).unwrap_or_default();
        match k {
            K::Left | K::Right => {
                self.child = None;
                self.index = if k == K::Left { self.index.saturating_sub(1) } else { (self.index + 1).min(tiles.len().saturating_sub(1)) };
            }
            K::Down => self.child = self.child.map_or_else(|| stops.first(), |c| stops.iter().find(|s| **s > c)).copied().or(self.child),
            K::Up => {
                if let Some(c) = self.child {
                    self.child = stops.iter().rev().find(|s| **s < c).copied();
                }
            }
            K::Ok => {
                return match self.child {
                    Some(c) => Self::activate_child(env, &tiles, self.index, c),
                    None => self.activate(&tiles, self.index),
                };
            }
            _ => {}
        }
        Out::default()
    }

    fn launch(app_id: &str) -> Effect {
        Effect::Luna { tag: super::env::tags::LAUNCH, uri: "luna://com.webos.applicationManager/launch".into(), payload: json!({"id": app_id}), subscribe: false }
    }

    /// OK on a device behind the receiver: its port's app is launched when not in front, then the receiver is set to the device's input.
    fn activate_child(env: &Env, tiles: &[Tile], i: usize, c: usize) -> Out {
        let Some(tile) = tiles.get(i) else { return Out::default() };
        let Some(code) = tile.children.get(c).and_then(|d| d.source.clone()) else { return Out::default() };
        let Some(switch) = env.denon_switch().filter(|s| s.writable) else { return Out::default() };
        let mut effects = Vec::new();
        if !tile.front {
            effects.push(Self::launch(&tile.app_id));
        }
        effects.push(Effect::Post { path: "/v1/receiver/set".into(), body: json!({"setting": "input", "value": code, "session": switch.session}) });
        let mut out = Out::effects(effects);
        out.close = true;
        out
    }

    fn activate(&mut self, tiles: &[Tile], i: usize) -> Out {
        let Some(t) = tiles.get(i) else { return Out::default() };
        let mut out = Out::effects(vec![Self::launch(&t.app_id)]);
        out.close = true;
        out
    }

    /// A click on tile `i`.
    pub fn click(&mut self, i: usize, env: &Env) -> Out {
        self.moved = true;
        self.index = i;
        self.child = None;
        self.activate(&Self::tiles(env), i)
    }

    /// A click on device `c` under tile `i`; a device with no switch does nothing.
    pub fn click_child(&mut self, i: usize, c: usize, env: &Env) -> Out {
        let tiles = Self::tiles(env);
        if !tiles.get(i).is_some_and(|t| Self::stops(t).contains(&c)) {
            return Out::default();
        }
        self.moved = true;
        self.index = i;
        self.child = Some(c);
        Self::activate_child(env, &tiles, i, c)
    }

    pub fn tick(&mut self, env: &mut Env, now: u64) -> Out {
        let mut out = Out::default();
        if self.read_due.is_some_and(|d| now >= d) {
            self.read_due = None;
            out.effects.extend(env.inputs.refresh());
            out.effects.extend(env.sio.read("sound", &["soundOutput"], None, now));
        }
        if self.settings_due.is_some_and(|d| now >= d) {
            self.settings_due = Some(now + SETTINGS_EVERY_MS);
            // only a plugin that is on is asked, and only when a port has devices behind it to name
            if env.denon_on() && env.inputs.list().iter().any(|e| e.connected && !e.devices.is_empty()) {
                out.effects.extend(env.gate.ask_settings());
            }
        }
        out
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.read_due, self.settings_due, self.life.due_at()].into_iter().flatten().min()
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// The picker as it is drawn at `now` (the wall clock: the age of a tile's picture).
    pub fn view(&self, env: &Env, now: u64) -> View {
        let tiles = Self::tiles(env);
        let shots = tiles.iter().map(|t| env.previews.shot(&t.key, now)).collect();
        View {
            up: self.life.presented(),
            title: "Inputs",
            sound: sound_line(env),
            index: self.index.min(tiles.len().saturating_sub(1)),
            child: self.child,
            tiles,
            over_video: !env.fg.home,
            pictures: env.previews.is_on(),
            shots,
        }
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

    /// The TV's status of this set: the receiver is a CEC device of HDMI 2 and says nothing of itself in the port's own label.
    fn tv_with_cec_receiver() -> Env {
        let mut env = Env::new(json!({}));
        env.inputs.parse(&json!({"devices": [{"appId": "com.webos.app.hdmi2", "label": "HDMI 2", "connected": true, "subList": [
            {"osdName": "Apple TV", "cecpDevType": 4, "vendor": 4346}, {"osdName": "AVR-S760H", "cecpDevType": 5, "vendor": 1485}]}]}));
        env
    }

    fn audio(out: &str) -> Input {
        Input::Topic { topic: "audio".into(), data: json!({"volumeStatus": {"volume": 30, "soundOutput": out}}) }
    }

    #[test]
    fn the_sound_line_names_the_receiver_by_its_cec_name_and_the_way_out_by_what_is_in_effect() {
        let mut env = tv_with_cec_receiver();
        assert_eq!(sound_line(&env), "", "nothing is said before the service has");
        let reads = env.sio.read("sound", &["soundOutput"], None, 0);
        let Some(Effect::Luna { tag, .. }) = reads.first() else { panic!() };
        env.update(&Input::Luna { tag: *tag, payload: json!({"settings": {"soundOutput": "tv_speaker"}}) }, 1);
        assert_eq!(sound_line(&env), "Sound: TV speakers");
        // the setting says the TV's speakers and an external device; the audio service says the receiver is in effect
        env.sio.write("sound.soundOutput", &json!("tv_external_speaker"), None, 1);
        assert_eq!(sound_line(&env), "Sound: TV and Denon AVR-S760H");
        let (_, changed) = env.update(&audio("external_arc"), 2);
        assert!(changed, "a change of the way out redraws the picker");
        assert_eq!(sound_line(&env), "Sound: Denon AVR-S760H (HDMI ARC)");
        assert!(!env.update(&audio("external_arc"), 3).1, "the same word again changes nothing");
        env.update(&audio("tv_speaker"), 4);
        assert_eq!(sound_line(&env), "Sound: TV speakers", "live: it follows the audio service back");
        env.update(&audio("external_optical"), 5);
        assert_eq!(sound_line(&env), "Sound: Optical out");
        env.update(&audio("something_new"), 6);
        assert_eq!(sound_line(&env), "", "a way out that is not known is not described as another");
        // no audio system on any port: the receiver has no name
        let mut bare = Env::new(json!({}));
        bare.update(&audio("external_arc"), 1);
        assert_eq!(sound_line(&bare), "Sound: Receiver (HDMI ARC)");
        bare.update(&audio("tv_external_speaker"), 2);
        assert_eq!(sound_line(&bare), "Sound: TV and external speakers");
    }

    #[test]
    fn the_port_the_sound_goes_out_of_is_marked_only_while_it_does() {
        let mut env = tv_with_cec_receiver();
        env.inputs.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.hdmi2"}) }, 0);
        let marked = |env: &Env| Picker::tiles(env).iter().filter(|t| t.sound).map(|t| t.port.clone()).collect::<Vec<_>>();
        env.update(&audio("tv_speaker"), 1);
        assert!(marked(&env).is_empty());
        env.update(&audio("external_arc"), 2);
        assert_eq!(marked(&env), vec!["HDMI 2 \u{b7} eARC".to_string()]);
    }

    #[test]
    fn the_owners_choices_decide_the_tiles_and_a_click_launches_that_one() {
        let mut env = tv(json!({"hdmi4": "always", "hdmi3": "hidden", "livetv": "hidden"}));
        let mut p = opened(&mut env);
        let v = p.view(&env, 0);
        assert_eq!(v.tiles.iter().map(|t| t.port.as_str()).collect::<Vec<_>>(), vec!["HDMI 1", "HDMI 2", "HDMI 4"]);
        assert_eq!(v.tiles[2].icon, "unplug");
        let out = p.click(0, &env);
        assert!(matches!(&out.effects[0], Effect::Luna { payload, .. } if payload["id"] == "com.webos.app.hdmi1"));
        assert!(out.close);
    }
}
