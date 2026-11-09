//! The session's messages from the phone, checked and typed. Unknown members are ignored; an unknown `t`, key or action is refused with
//! its own code, so a phone can probe what a TV speaks.

use serde_json::{Map, Value};

use crate::Fault;

/// The remote's keys by the names phones send, with the Linux code the TV's remote delivers.
pub const KEYS: [(&str, u32); 42] = [
    ("up", 103),
    ("down", 108),
    ("left", 105),
    ("right", 106),
    ("ok", 28),
    ("back", 412),
    ("home", 773),
    ("exit", 174),
    ("menu", 139),
    ("info", 358),
    ("guide", 362),
    ("list", 787),
    ("more", 994),
    ("input", 241),
    ("power", 116),
    ("volup", 115),
    ("voldown", 114),
    ("mute", 113),
    ("chup", 402),
    ("chdown", 403),
    ("0", 11),
    ("1", 2),
    ("2", 3),
    ("3", 4),
    ("4", 5),
    ("5", 6),
    ("6", 7),
    ("7", 8),
    ("8", 9),
    ("9", 10),
    ("red", 398),
    ("green", 399),
    ("yellow", 400),
    ("blue", 401),
    ("play", 207),
    ("pause", 119),
    ("playpause", 164),
    ("stop", 128),
    ("rew", 168),
    ("ff", 208),
    ("netflix", 1037),
    ("amazon", 1038),
];

/// Keys that take `down` and `up`: the ones meant to be held.
const HOLDABLE: [&str; 10] = ["up", "down", "left", "right", "volup", "voldown", "chup", "chdown", "rew", "ff"];

pub const POWER: u32 = 116;

pub fn key_code(name: &str) -> Option<u32> {
    KEYS.iter().find(|(n, _)| *n == name).map(|(_, code)| *code)
}

/// The longest text one `text` carries, in bytes.
pub const MAX_TEXT: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Pressed and released, shorter than the remote's hold.
    Tap,
    /// Pressed and held past the remote's hold, then released: timed on the TV, so Wi-Fi jitter cannot turn one into the other.
    Hold,
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// 1–256 characters before the cursor.
    Backspace(u16),
    Enter,
    Tab,
    Left,
    Right,
    Clear,
    /// The TV's on-screen keyboard.
    Hide,
    Show,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    Home,
    Quick,
    Settings(Option<String>),
    Notifications,
    Input,
    PictureOff,
    Cameras,
    Camera(String),
    CameraClose,
    Guide,
    Tune(String),
    Launch(String),
    /// The receiver's level in half steps (`level` 41.5 is 83): its scale moves by 0.5.
    Volume(u16),
    Power(bool),
}

impl Act {
    /// Its `do`, which is also its name in the state's `acts`.
    pub fn name(&self) -> &'static str {
        match self {
            Act::Home => "home",
            Act::Quick => "quick",
            Act::Settings(_) => "settings",
            Act::Notifications => "notifications",
            Act::Input => "input",
            Act::PictureOff => "pictureoff",
            Act::Cameras => "cameras",
            Act::Camera(_) => "camera",
            Act::CameraClose => "camera-close",
            Act::Guide => "guide",
            Act::Tune(_) => "tune",
            Act::Launch(_) => "launch",
            Act::Volume(_) => "volume",
            Act::Power(_) => "power",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Apps,
    Channels,
    Cameras,
}

impl List {
    pub fn name(self) -> &'static str {
        match self {
            List::Apps => "apps",
            List::Channels => "channels",
            List::Cameras => "cameras",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// The first frame; each member at most 64 characters (`name` 40).
    Hello { name: String, model: String, app: String, kit: String },
    Key { name: String, code: u32, press: Press },
    Text(String),
    Edit(Edit),
    Act { id: u32, act: Act },
    Get { id: u32, what: List },
    /// `n`, echoed as it came.
    Ping(Value),
    Bye,
}

/// A refused frame: answered with `done` and the code when it has an `id` (`act`), else with `error` and `about`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub fault: Fault,
    pub about: String,
    pub id: Option<u32>,
}

impl Refusal {
    /// The frame that answers it.
    pub fn reply(&self) -> Value {
        match self.id {
            Some(id) if self.about == "act" => serde_json::json!({"t": "done", "id": id, "error": self.fault.code()}),
            _ => serde_json::json!({"t": "error", "code": self.fault.code(), "about": self.about}),
        }
    }
}

fn text_member(m: &Map<String, Value>, key: &str, max: usize) -> String {
    m.get(key).and_then(Value::as_str).unwrap_or("").trim().chars().take(max).collect()
}

/// One frame's plaintext.
pub fn parse(frame: &[u8]) -> Result<Request, Refusal> {
    let refuse = |fault, about: &str, id| Refusal { fault, about: about.to_string(), id };
    let value: Value = serde_json::from_slice(frame).map_err(|_| refuse(Fault::BadMessage, "", None))?;
    let m = value.as_object().ok_or_else(|| refuse(Fault::BadMessage, "", None))?;
    let t = m.get("t").and_then(Value::as_str).ok_or_else(|| refuse(Fault::BadMessage, "", None))?;
    let id = m.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok());
    let bad = |fault| refuse(fault, t, id);
    let string = |key: &str| m.get(key).and_then(Value::as_str);
    Ok(match t {
        "hello" => Request::Hello {
            name: text_member(m, "name", 40),
            model: text_member(m, "model", 64),
            app: text_member(m, "app", 64),
            kit: text_member(m, "kit", 64),
        },
        "key" => {
            let name = string("k").ok_or_else(|| bad(Fault::BadMessage))?;
            let code = key_code(name).ok_or_else(|| bad(Fault::BadKey))?;
            let press = match string("a").unwrap_or("tap") {
                "tap" => Press::Tap,
                "hold" => Press::Hold,
                "down" => Press::Down,
                "up" => Press::Up,
                _ => return Err(bad(Fault::BadMessage)),
            };
            let allowed = match press {
                Press::Tap => true,
                Press::Hold => name != "home",
                Press::Down | Press::Up => HOLDABLE.contains(&name),
            };
            if !allowed {
                return Err(bad(Fault::BadKey));
            }
            Request::Key { name: name.to_string(), code, press }
        }
        "text" => {
            let s = string("s").ok_or_else(|| bad(Fault::BadMessage))?;
            if s.len() > MAX_TEXT {
                return Err(bad(Fault::TooLong));
            }
            if s.is_empty() || s.chars().any(char::is_control) {
                return Err(bad(Fault::BadMessage));
            }
            Request::Text(s.to_string())
        }
        "edit" => Request::Edit(match string("op").ok_or_else(|| bad(Fault::BadMessage))? {
            "backspace" => {
                let n = match m.get("n") {
                    None => 1,
                    Some(n) => n.as_u64().filter(|n| (1..=MAX_TEXT as u64).contains(n)).ok_or_else(|| bad(Fault::BadMessage))?,
                };
                Edit::Backspace(n as u16)
            }
            "enter" => Edit::Enter,
            "tab" => Edit::Tab,
            "left" => Edit::Left,
            "right" => Edit::Right,
            "clear" => Edit::Clear,
            "hide" => Edit::Hide,
            "show" => Edit::Show,
            _ => return Err(bad(Fault::BadMessage)),
        }),
        "act" => {
            let id = id.ok_or_else(|| bad(Fault::BadMessage))?;
            let arg = |key: &str| -> Result<String, Refusal> {
                match m.get(key) {
                    Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
                    Some(Value::Number(n)) => Ok(n.to_string()),
                    _ => Err(bad(Fault::BadMessage)),
                }
            };
            let act = match string("do").ok_or_else(|| bad(Fault::BadMessage))? {
                "home" => Act::Home,
                "quick" => Act::Quick,
                "settings" => Act::Settings(string("page").map(str::to_string)),
                "notifications" => Act::Notifications,
                "input" => Act::Input,
                "pictureoff" => Act::PictureOff,
                "cameras" => Act::Cameras,
                "camera" => Act::Camera(arg("camera")?),
                "camera-close" => Act::CameraClose,
                "guide" => Act::Guide,
                "tune" => Act::Tune(arg("channel")?),
                "launch" => Act::Launch(arg("app")?),
                "volume" => Act::Volume(
                    m.get("level")
                        .and_then(Value::as_f64)
                        .filter(|l| l.is_finite() && (0.0..=f64::from(u16::MAX / 2)).contains(l))
                        .map(|l| (l * 2.0).round() as u16)
                        .ok_or_else(|| bad(Fault::BadMessage))?,
                ),
                "power" => Act::Power(match string("to") {
                    Some("on") => true,
                    Some("off") => false,
                    _ => return Err(bad(Fault::BadMessage)),
                }),
                _ => return Err(bad(Fault::BadAction)),
            };
            Request::Act { id, act }
        }
        "get" => {
            let id = id.ok_or_else(|| bad(Fault::BadMessage))?;
            let what = match string("what") {
                Some("apps") => List::Apps,
                Some("channels") => List::Channels,
                Some("cameras") => List::Cameras,
                _ => return Err(bad(Fault::BadMessage)),
            };
            Request::Get { id, what }
        }
        "ping" => Request::Ping(m.get("n").cloned().unwrap_or(Value::Null)),
        "bye" => Request::Bye,
        _ => return Err(bad(Fault::BadMessage)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(v: Value) -> Result<Request, Refusal> {
        parse(v.to_string().as_bytes())
    }

    fn fault(v: Value) -> Fault {
        p(v).unwrap_err().fault
    }

    #[test]
    fn keys_map_to_the_remotes_codes_and_only_held_keys_go_down() {
        assert_eq!(p(json!({"t": "key", "k": "ok"})), Ok(Request::Key { name: "ok".into(), code: 28, press: Press::Tap }));
        assert_eq!(p(json!({"t": "key", "k": "left", "a": "down"})), Ok(Request::Key { name: "left".into(), code: 105, press: Press::Down }));
        assert_eq!(p(json!({"t": "key", "k": "back", "a": "hold"})), Ok(Request::Key { name: "back".into(), code: 412, press: Press::Hold }));
        assert_eq!(key_code("ff"), Some(208));
        assert_eq!(key_code("amazon"), Some(1038));
        assert_eq!(key_code("7"), Some(8));
        assert_eq!(fault(json!({"t": "key", "k": "ok", "a": "down"})), Fault::BadKey);
        assert_eq!(fault(json!({"t": "key", "k": "home", "a": "hold"})), Fault::BadKey, "holding Home is the remote's way out to LG");
        assert_eq!(fault(json!({"t": "key", "k": "turbo"})), Fault::BadKey);
        assert_eq!(fault(json!({"t": "key", "k": "ok", "a": "twice"})), Fault::BadMessage);
    }

    #[test]
    fn text_and_edits_are_bounded() {
        assert_eq!(p(json!({"t": "text", "s": "héllo"})), Ok(Request::Text("héllo".into())));
        assert_eq!(fault(json!({"t": "text", "s": "x".repeat(257)})), Fault::TooLong);
        assert_eq!(fault(json!({"t": "text", "s": "a\nb"})), Fault::BadMessage);
        assert_eq!(fault(json!({"t": "text", "s": ""})), Fault::BadMessage);
        assert_eq!(p(json!({"t": "edit", "op": "backspace"})), Ok(Request::Edit(Edit::Backspace(1))));
        assert_eq!(p(json!({"t": "edit", "op": "backspace", "n": 256})), Ok(Request::Edit(Edit::Backspace(256))));
        assert_eq!(fault(json!({"t": "edit", "op": "backspace", "n": 0})), Fault::BadMessage);
        assert_eq!(p(json!({"t": "edit", "op": "hide"})), Ok(Request::Edit(Edit::Hide)));
    }

    #[test]
    fn actions_carry_their_id_into_refusals_and_unknown_ones_say_so() {
        assert_eq!(p(json!({"t": "act", "id": 4, "do": "launch", "app": "netflix"})), Ok(Request::Act { id: 4, act: Act::Launch("netflix".into()) }));
        assert_eq!(p(json!({"t": "act", "id": 5, "do": "tune", "channel": 31})), Ok(Request::Act { id: 5, act: Act::Tune("31".into()) }));
        assert_eq!(p(json!({"t": "act", "id": 6, "do": "power", "to": "off"})), Ok(Request::Act { id: 6, act: Act::Power(false) }));
        // the receiver's scale moves by half steps: a whole number and a half both arrive exact, anything finer snaps to the nearest half
        assert_eq!(p(json!({"t": "act", "id": 10, "do": "volume", "level": 50})), Ok(Request::Act { id: 10, act: Act::Volume(100) }));
        assert_eq!(p(json!({"t": "act", "id": 11, "do": "volume", "level": 41.5})), Ok(Request::Act { id: 11, act: Act::Volume(83) }));
        assert_eq!(p(json!({"t": "act", "id": 12, "do": "volume", "level": 41.6})), Ok(Request::Act { id: 12, act: Act::Volume(83) }));
        assert_eq!(p(json!({"t": "act", "id": 13, "do": "volume", "level": -1})).unwrap_err().reply(), json!({"t": "done", "id": 13, "error": "bad-message"}));
        let refusal = p(json!({"t": "act", "id": 7, "do": "fly"})).unwrap_err();
        assert_eq!(refusal.reply(), json!({"t": "done", "id": 7, "error": "bad-action"}));
        assert_eq!(p(json!({"t": "act", "id": 8, "do": "launch"})).unwrap_err().reply(), json!({"t": "done", "id": 8, "error": "bad-message"}));
        assert_eq!(p(json!({"t": "get", "id": 9, "what": "apps"})), Ok(Request::Get { id: 9, what: List::Apps }));
        assert_eq!(p(json!({"t": "teleport"})).unwrap_err().reply(), json!({"t": "error", "code": "bad-message", "about": "teleport"}));
        assert_eq!(parse(b"not json").unwrap_err().fault, Fault::BadMessage);
        assert_eq!(p(json!({"t": "ping", "n": 3, "extra": true})), Ok(Request::Ping(json!(3))), "unknown members are ignored");
    }
}
