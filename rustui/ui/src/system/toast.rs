//! The toasts' model (`panels/Notify.qml` of the QML shell, the toast part; the mock `docs/mocks/07-toast`): LG's toast feed
//! (`luna://com.webos.notification/getToastNotification`, subscribed) drawn by LumaTV under its own compositor. Nothing is filtered
//! or delayed: a remote-battery or pairing toast is shown as soon as the service pushes it, in the upper safe corner, newest
//! first, at most three, each gone after its own timeout (LG's 5 s). A toast of the same words replaces the one up rather than
//! stacking (LG's ToastModel.js). A toast never takes a key or the focus.

use crate::kit::screen::{Effect, Input, Model};
use serde_json::{Value, json};

/// The tag the feed's subscription answers come back with.
pub const FEED_TAG: u64 = 0x7041;
pub const FEED_URI: &str = "luna://com.webos.notification/getToastNotification";
const KEEP: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub title: String,
    pub body: String,
    /// A picture of the service's own (a file path or a URL), or "" for LG's stock icon, which is drawn as the kit's info tile.
    pub icon: String,
    pub expires: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Toasts {
    list: Vec<Toast>,
    serial: u64,
    subscribed: bool,
    changed: bool,
}

/// What the surface draws: the toasts, newest first.
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
        let text = m.get("message").and_then(Value::as_str).unwrap_or("");
        let text: String = {
            let mut t = text.to_string();
            for br in ["<br/>", "<br />", "<br>", "<BR>", "<BR/>"] {
                t = t.replace(br, "\n");
            }
            t.trim().to_string()
        };
        if text.is_empty() {
            return;
        }
        let mut lines = text.split('\n');
        let title = lines.next().unwrap_or("").trim().to_string();
        let body = lines.map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
        let icon = m.get("iconPath").and_then(Value::as_str).unwrap_or("").to_string();
        let own = icon.is_empty() || {
            let file = icon.rsplit('/').next().unwrap_or("").to_lowercase();
            ["info.", "notice.", "default."].iter().any(|p| file.starts_with(p))
        };
        let timeout = m.get("timeout").and_then(Value::as_f64).filter(|t| *t > 0.0).map_or(5000.0, |t| t * 1000.0);
        self.serial += 1;
        let toast = Toast { id: self.serial, title, body, icon: if own { String::new() } else { icon }, expires: now + timeout as u64 };
        self.list.retain(|t| !(t.title == toast.title && t.body == toast.body));
        self.list.insert(0, toast);
        self.list.truncate(KEEP);
        self.changed = true;
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// The state for the compositor's gate (`ctl shell`'s `ui.toasts`).
    pub fn state(&self) -> Value {
        json!(self.list.len())
    }
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
            Input::Tick => {
                let n = self.list.len();
                self.list.retain(|t| t.expires > now);
                self.changed = n != self.list.len();
            }
            _ => {}
        }
        if self.changed { vec![Effect::Redraw] } else { Vec::new() }
    }

    fn view(&self) -> View {
        self.list.clone()
    }

    fn due_at(&self) -> Option<u64> {
        self.list.iter().map(|t| t.expires).min()
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
    fn the_first_line_is_the_title_the_rest_the_body_and_the_same_words_replace_rather_than_stack() {
        let mut t = Toasts::new();
        push(&mut t, "Bluetooth<br>Headphones connected", None, 1000);
        assert_eq!(t.view()[0].title, "Bluetooth");
        assert_eq!(t.view()[0].body, "Headphones connected");
        assert_eq!(t.view()[0].expires, 6000, "LG's 5 s");
        push(&mut t, "Bluetooth<br>Headphones connected", None, 2000);
        assert_eq!(t.len(), 1, "replaced, not stacked");
        assert_eq!(t.view()[0].expires, 7000);
        for i in 0..4 {
            push(&mut t, &format!("Toast {i}"), None, 3000 + i);
        }
        assert_eq!(t.len(), 3, "at most three");
        assert_eq!(t.view()[0].title, "Toast 3", "newest first");
        assert!(push(&mut t, "   ", None, 4000).is_empty(), "an empty message is nothing");
    }

    #[test]
    fn lgs_stock_icons_are_the_kits_tile_and_a_services_own_picture_is_kept() {
        let mut t = Toasts::new();
        t.update(&Input::Luna { tag: FEED_TAG, payload: json!({"message": "A", "iconPath": "/usr/share/icons/info.png"}) }, 1);
        assert_eq!(t.view()[0].icon, "");
        t.update(&Input::Luna { tag: FEED_TAG, payload: json!({"message": "B", "iconPath": "/media/app/icon.png"}) }, 2);
        assert_eq!(t.view()[0].icon, "/media/app/icon.png");
    }
}
