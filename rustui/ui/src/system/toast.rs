//! The toasts' model (`panels/Notify.qml` of the QML shell, the toast part; the mock `docs/mocks/07-toast`): LG's toast feed
//! (`luna://com.webos.notification/getToastNotification`, subscribed) drawn by LumaTV under its own compositor. Nothing is delayed: the
//! newest toast replaces the current one and expires after its own timeout (LG's 5 s by default). LG's announcements of a picture's or a
//! sound's format are the format badge's ([`super::badge`]), not toasts.
//! A toast never takes a key or the focus.

use crate::kit::screen::{Effect, Input, Model};
use serde_json::{Value, json};

/// The tag the feed's subscription answers come back with.
pub const FEED_TAG: u64 = 0x7041;
pub const FEED_URI: &str = "luna://com.webos.notification/getToastNotification";

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub title: String,
    pub body: String,
    /// A picture of the service's own (a file path or a URL), or "" for LG's stock icon.
    pub icon: String,
    pub expires: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Toasts {
    current: Option<Toast>,
    serial: u64,
    subscribed: bool,
    changed: bool,
}

/// What the surface draws: at most the current toast.
pub type View = Vec<Toast>;

impl Toasts {
    pub fn new() -> Self {
        Self::default()
    }

    /// The subscription to the feed, asked once at the start.
    pub fn subscribe(&mut self) -> Vec<Effect> {
        if self.subscribed {
            return Vec::new();
        }
        self.subscribed = true;
        vec![Effect::Luna { tag: FEED_TAG, uri: FEED_URI.into(), payload: json!({"subscribe": true}), subscribe: true }]
    }

    fn take(&mut self, m: &Value, now: u64) {
        if super::badge::announced(m).is_some() {
            return;
        }
        let lines = plain_lines(m.get("message").and_then(Value::as_str).unwrap_or(""));
        let Some((title, body)) = lines.split_first() else {
            return;
        };
        let (title, body) = (title.clone(), body.join("\n"));
        let icon = m.get("iconPath").and_then(Value::as_str).unwrap_or("").to_string();
        let own = icon.is_empty() || {
            let file = icon.rsplit('/').next().unwrap_or("").to_lowercase();
            ["info.", "notice.", "default."].iter().any(|p| file.starts_with(p))
        };
        let timeout = m.get("timeout").and_then(Value::as_f64).filter(|t| *t > 0.0).map_or(5000.0, |t| t * 1000.0);
        self.serial += 1;
        let toast = Toast { id: self.serial, title, body, icon: if own { String::new() } else { icon }, expires: now + timeout as u64 };
        self.current = Some(toast);
        self.changed = true;
    }

    pub fn is_empty(&self) -> bool {
        self.current.is_none()
    }

    pub fn len(&self) -> usize {
        usize::from(self.current.is_some())
    }

    /// The state for the compositor's gate (`ctl shell`'s `ui.toasts`).
    pub fn state(&self) -> Value {
        json!(self.len())
    }
}

/// The lines of a toast's message: LG's `<br>` breaks it, its other markup (`<font size="1">`) is dropped and its text kept.
pub fn plain_lines(message: &str) -> Vec<String> {
    let mut text = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(i) = rest.find('<') {
        text.push_str(&rest[..i]);
        let tag = &rest[i..];
        match tag.find('>') {
            Some(j) if tag[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/') => {
                if tag[1..j].trim_end_matches('/').trim().eq_ignore_ascii_case("br") {
                    text.push('\n');
                }
                rest = &tag[j + 1..];
            }
            _ => {
                text.push('<');
                rest = &tag[1..];
            }
        }
    }
    text.push_str(rest);
    text.split('\n').map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

impl Model for Toasts {
    type View = View;

    fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        self.changed = false;
        match input {
            Input::Luna { tag, payload } if *tag == FEED_TAG => {
                if payload.get("message").is_some() {
                    self.take(payload, now);
                }
            }
            Input::Tick if self.current.as_ref().is_some_and(|t| t.expires <= now) => {
                self.current = None;
                self.changed = true;
            }
            _ => {}
        }
        if self.changed { vec![Effect::Redraw] } else { Vec::new() }
    }

    fn view(&self) -> View {
        self.current.iter().cloned().collect()
    }

    fn due_at(&self) -> Option<u64> {
        self.current.as_ref().map(|t| t.expires)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(t: &mut Toasts, message: &str, timeout: Option<f64>, now: u64) -> Vec<Effect> {
        let mut p = json!({"sourceId": "lg", "message": message});
        if let Some(s) = timeout {
            p["timeout"] = json!(s);
        }
        t.update(&Input::Luna { tag: FEED_TAG, payload: p }, now)
    }

    #[test]
    fn the_feed_is_subscribed_once_and_a_push_is_a_toast_for_its_timeout() {
        let mut t = Toasts::new();
        let e = t.subscribe();
        assert!(matches!(&e[0], Effect::Luna { uri, subscribe: true, .. } if uri == FEED_URI));
        assert!(t.subscribe().is_empty());
        assert_eq!(push(&mut t, "Remote battery low", Some(8.0), 1000), vec![Effect::Redraw]);
        let v = t.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].title, "Remote battery low");
        assert_eq!(v[0].body, "");
        assert_eq!(v[0].expires, 9000);
        assert_eq!(t.due_at(), Some(9000));
        assert!(t.update(&Input::Tick, 8999).is_empty());
        assert_eq!(t.update(&Input::Tick, 9000), vec![Effect::Redraw]);
        assert!(t.is_empty());
    }

    #[test]
    fn incoming_messages_replace_the_current_toast_and_expiry_never_revives_old_messages() {
        let mut t = Toasts::new();
        push(&mut t, "Bluetooth<br>Headphones connected<br>Ready", Some(20.0), 1000);
        assert_eq!(t.view()[0].body, "Headphones connected\nReady");
        push(&mut t, "New", Some(2.0), 2000);
        assert_eq!(t.len(), 1);
        assert_eq!(t.view()[0].title, "New");
        assert_eq!(t.due_at(), Some(4000));
        assert!(t.update(&Input::Tick, 3999).is_empty());
        assert_eq!(t.update(&Input::Tick, 4000), vec![Effect::Redraw]);
        assert!(t.is_empty());
        assert_eq!(t.due_at(), None);
        assert!(t.update(&Input::Tick, 21000).is_empty());
        push(&mut t, "Same", None, 22000);
        push(&mut t, "Same", None, 23000);
        assert_eq!(t.due_at(), Some(28000));
        assert!(push(&mut t, "   ", None, 24000).is_empty());
    }

    #[test]
    fn lgs_stock_icons_and_service_pictures_are_distinguished() {
        let mut t = Toasts::new();
        t.update(&Input::Luna { tag: FEED_TAG, payload: json!({"message": "A", "iconPath": "/usr/share/icons/info.png"}) }, 1);
        assert_eq!(t.view()[0].icon, "");
        t.update(&Input::Luna { tag: FEED_TAG, payload: json!({"message": "B", "iconPath": "/media/app/icon.png"}) }, 2);
        assert_eq!(t.view()[0].icon, "/media/app/icon.png");
    }

    #[test]
    fn markup_is_dropped_and_lgs_announcements_are_the_badges() {
        let mut t = Toasts::new();
        let audio = |message: &str| Input::Luna { tag: FEED_TAG, payload: json!({"sourceId": "com.webos.service.audio", "message": message}) };
        assert!(t.update(&audio("Dolby Vision<br>Dolby Atmos<br><font size=\"1\">via Audio Device</font>"), 1).is_empty());
        assert!(t.is_empty());
        t.update(&audio("Turn Dolby Atmos on<br/><font size=\"1\">in Sound</font> <b>now</b>"), 2);
        assert_eq!((t.view()[0].title.as_str(), t.view()[0].body.as_str()), ("Turn Dolby Atmos on", "in Sound now"));
        assert_eq!(plain_lines("a < b<BR />c"), ["a < b", "c"]);
    }
}
