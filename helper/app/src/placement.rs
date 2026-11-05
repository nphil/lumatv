//! Where the overlay items sit and how the owner moves them, for any toolkit: the volume bar, the mute mark, the camera window, the notification
//! cards and the toasts. The rules are `lumatv-placement`'s (`placement/`); this is the JSON around them and the one editor session the TV has.
//!
//! Nothing here is saved. The caller owns the theme: it sends what the owner placed (`layout.overlays`, as it has it from its theme, shared or
//! the profile's) and writes what an answer's `effects` ask it to write. A client that links the crate needs none of this.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/placement` | `{registry, frame}`: the items (id, title, icon, zone, edges, default, sample size) and the constants of the rules; the frame from Home's shipped numbers |
//! | `POST /v1/placement/layout` | `{frame, customised, items: {id: {rect, placement, custom, waits, words, slide, lift, spots?}}}` for `{layout?, limit?, user?, sizes?, overApp?, work?: {id, placement}, avoid?: [rect]}` |
//! | `POST /v1/placement/editor/open` | `{view, effects, panel?}` for `{layout?, limit?, user?, sizes?, item?, panel?: {w, h}}`: starts the editor on what the owner placed |
//! | `GET /v1/placement/editor` | the same, as it is now (`?panelW=&panelH=` for the panel's place); 409 `no_editor` when it is not open |
//! | `POST /v1/placement/editor/key` | `{key: left, right, up, down, ok or back, t?}`: `effects` is `[{save: <layout.overlays>}]` and/or `[{close: true}]` |
//! | `POST /v1/placement/editor/pointer` | `{phase: down, move or up, item?, x, y, pressed?, t?}` in screen pixels |
//! | `POST /v1/placement/editor/button` | `{index: 0 Reset this, 1 Reset all, 2 Done, t?}` for a pointer on a button |
//! | `POST /v1/placement/editor/sizes` | `{sizes}`: the stand-ins were measured again (text size) |
//! | `POST /v1/placement/editor/close` | `{closed}` |
//!
//! A size is `{w, h}` in whole pixels; an item that lives on edges (the volume bar) takes `{left, bottom, right}` of them. `t` is the client's clock in
//! ms (a held key is told from a tap by the gap between presses); the helper's own is used when it is left out. `panel` is the size of the toolkit's
//! heads-up sheet, and the answer's `panel` is where it goes.

use crate::API_VERSION;
use crate::http::{Request, Response, error_response};
use crate::timeutil::now_ms;
use lumatv_placement::{Editor, Effect, Frame, Key, Placement, Rect, Registry, Size, Sizes, User};
use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/placement",
        "the overlay items (volume bar, mute mark, camera window, notification cards, toasts) and the constants of where they may sit",
    ),
    (
        "POST",
        "/v1/placement/layout",
        "where every item is for what the owner placed, the sizes the toolkit measured and Home's layout: rectangles, words, the way each arrives from",
    ),
    (
        "POST",
        "/v1/placement/editor/open",
        "opens the layout editor on what the owner placed; the answer is what to draw",
    ),
    (
        "GET",
        "/v1/placement/editor",
        "what the layout editor shows now, for a toolkit that draws it",
    ),
    (
        "POST",
        "/v1/placement/editor/key",
        "a remote key for the layout editor: {key}; the answer says what to save and whether to close",
    ),
    (
        "POST",
        "/v1/placement/editor/pointer",
        "a pointer for the layout editor: {phase: down|move|up, item, x, y}",
    ),
    (
        "POST",
        "/v1/placement/editor/button",
        "a press on one of the editor's buttons: {index: 0 Reset this, 1 Reset all, 2 Done}",
    ),
    (
        "POST",
        "/v1/placement/editor/sizes",
        "the sizes the toolkit measured for the editor's stand-ins changed: {sizes}",
    ),
    (
        "POST",
        "/v1/placement/editor/close",
        "closes the layout editor",
    ),
];

/// The TV's one editor.
#[derive(Default)]
pub struct Session {
    editor: Mutex<Option<Editor>>,
}

static SESSION: LazyLock<Session> = LazyLock::new(Session::default);

pub fn handle(request: &Request, rest: &[&str]) -> Response {
    handle_with(&SESSION, request, rest)
}

fn ok(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}

fn bad(why: &str) -> Response {
    error_response(400, "bad_request", why)
}

fn body_of(request: &Request) -> Result<Map<String, Value>, Response> {
    if request.body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(&request.body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(bad("the body must be a JSON object")),
        Err(e) => Err(bad(&format!("the body is not JSON: {e}"))),
    }
}

fn frame_of(body: &Map<String, Value>) -> Frame {
    Frame::from_json(
        body.get("layout").unwrap_or(&Value::Null),
        body.get("limit"),
    )
}

fn user_of(body: &Map<String, Value>) -> User {
    Registry::shipped().user_from_json(body.get("user").unwrap_or(&Value::Null))
}

fn sizes_of(body: &Map<String, Value>) -> Sizes {
    Sizes::from_json(body.get("sizes").unwrap_or(&Value::Null))
}

fn time_of(body: &Map<String, Value>) -> u64 {
    body.get("t").and_then(Value::as_u64).unwrap_or_else(now_ms)
}

fn panel_of(body: &Map<String, Value>) -> Option<Size> {
    let p = body.get("panel")?;
    let (w, h) = (p.get("w")?.as_i64()?, p.get("h")?.as_i64()?);
    ((1..=1920).contains(&w) && (1..=1080).contains(&h)).then(|| Size::new(w as i32, h as i32))
}

/// Where every item is, for one request's numbers.
fn layout(body: &Map<String, Value>) -> Response {
    let reg = Registry::shipped();
    let (frame, user, sizes) = (frame_of(body), user_of(body), sizes_of(body));
    let over_app = body
        .get("overApp")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let work = body.get("work").and_then(|w| {
        let id = w.get("id")?.as_str()?;
        Some((reg.item(id)?.id, Placement::from_json(w.get("placement")?)?))
    });
    let avoid: Vec<Rect> = body
        .get("avoid")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |a| a.iter().filter_map(Rect::from_json).collect());
    let laid = reg.layout_all(&user, &frame, &sizes, work, over_app);
    let effective = match work {
        Some((id, p)) => reg.with_placement(&user, id, p),
        None => user.clone(),
    };
    let mut items = Map::new();
    for l in &laid.items {
        let it = reg.item(l.id).expect("a laid-out item is in the registry");
        let mut item = json!({
            "rect": l.rect, "placement": l.placement, "custom": effective.has(l.id), "waits": l.waits,
            "words": reg.words(l.id, &effective), "slide": reg.slide_of(l.id, &effective), "lift": reg.lift_over(l.id, &effective, over_app),
        });
        if it.drift > 0 && !reg.is_auto(l.id, &effective) {
            item["spots"] = json!(reg.drift_spots(
                l.id,
                &effective,
                &frame,
                Size::new(l.rect.w, l.rect.h),
                &avoid
            ));
        }
        items.insert(l.id.to_string(), item);
    }
    ok(json!({"frame": frame, "customised": effective.customised(), "items": items}))
}

fn effects_json(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .map(|e| match e {
            Effect::Save(map) => json!({"save": map}),
            Effect::Close => json!({"close": true}),
        })
        .collect()
}

impl Session {
    fn answer(editor: &Editor, effects: &[Effect], now: u64, panel: Option<Size>) -> Response {
        let mut out = json!({"view": editor.view(now), "effects": effects_json(effects)});
        if let Some(p) = panel {
            out["panel"] = json!(editor.panel_spot(p.w, p.h));
        }
        ok(out)
    }

    fn no_editor() -> Response {
        error_response(409, "no_editor", "the layout editor is not open")
    }

    /// Runs `f` on the open editor and answers with its view and what `f` asks the caller to do.
    fn with_editor(
        &self,
        body: &Map<String, Value>,
        f: impl FnOnce(&mut Editor, u64) -> Result<Vec<Effect>, Response>,
    ) -> Response {
        let mut guard = self.editor.lock();
        let Some(editor) = guard.as_mut() else {
            return Self::no_editor();
        };
        let now = time_of(body);
        match f(editor, now) {
            Ok(effects) => {
                let closed = effects.contains(&Effect::Close);
                let answer = Self::answer(editor, &effects, now, panel_of(body));
                if closed {
                    *guard = None;
                }
                answer
            }
            Err(r) => r,
        }
    }

    fn open(&self, body: &Map<String, Value>) -> Response {
        let reg = Registry::shipped().clone();
        let mut editor = Editor::new(reg, user_of(body), frame_of(body), sizes_of(body));
        editor.start(body.get("item").and_then(Value::as_str));
        let answer = Self::answer(&editor, &[], time_of(body), panel_of(body));
        *self.editor.lock() = Some(editor);
        answer
    }

    fn view(&self, request: &Request) -> Response {
        let query = |name: &str| {
            request
                .query
                .iter()
                .find(|(k, _)| k == name)
                .and_then(|(_, v)| v.parse::<i32>().ok())
        };
        let panel = query("panelW")
            .zip(query("panelH"))
            .filter(|(w, h)| (1..=1920).contains(w) && (1..=1080).contains(h))
            .map(|(w, h)| Size::new(w, h));
        let guard = self.editor.lock();
        match guard.as_ref() {
            Some(editor) => Self::answer(editor, &[], now_ms(), panel),
            None => Self::no_editor(),
        }
    }

    fn key(&self, body: &Map<String, Value>) -> Response {
        let Some(key) = body.get("key").and_then(Value::as_str).and_then(Key::parse) else {
            return bad("key must be left, right, up, down, ok or back");
        };
        self.with_editor(body, |e, now| Ok(e.key(key, now)))
    }

    fn pointer(&self, body: &Map<String, Value>) -> Response {
        let num = |k: &str| {
            body.get(k)
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && n.abs() < 1e5)
        };
        let phase = body.get("phase").and_then(Value::as_str).unwrap_or("");
        let at = num("x").zip(num("y"));
        let pressed = body.get("pressed").and_then(Value::as_bool).unwrap_or(true);
        let item = body.get("item").and_then(Value::as_str);
        match (phase, at) {
            ("down", Some(_)) if item.is_none() => {
                bad("a pointer that goes down names the item it is on")
            }
            ("down" | "move", None) => bad("x and y are the pointer's, in screen pixels"),
            ("down", Some((x, y))) => self.with_editor(body, |e, _| {
                e.pointer_down(item.unwrap_or(""), x, y);
                Ok(Vec::new())
            }),
            ("move", Some((x, y))) => self.with_editor(body, |e, now| {
                e.pointer_move(x, y, pressed, now);
                Ok(Vec::new())
            }),
            ("up", _) => self.with_editor(body, |e, _| Ok(e.pointer_up())),
            _ => bad("phase must be down, move or up"),
        }
    }

    fn button(&self, body: &Map<String, Value>) -> Response {
        let Some(index) = body.get("index").and_then(Value::as_u64).filter(|i| *i < 3) else {
            return bad("index is 0 (Reset this), 1 (Reset all) or 2 (Done)");
        };
        self.with_editor(body, |e, now| Ok(e.press_button(index as usize, now)))
    }

    fn sizes(&self, body: &Map<String, Value>) -> Response {
        self.with_editor(body, |e, _| {
            e.set_sizes(sizes_of(body));
            Ok(Vec::new())
        })
    }

    fn close(&self) -> Response {
        let was = self.editor.lock().take().is_some();
        ok(json!({"closed": was}))
    }
}

/// Serves `/v1/placement/...` on `session`.
pub fn handle_with(session: &Session, request: &Request, rest: &[&str]) -> Response {
    let (get, post) = (request.method == "GET", request.method == "POST");
    let with_body = |f: &dyn Fn(&Map<String, Value>) -> Response| match body_of(request) {
        Ok(body) => f(&body),
        Err(r) => r,
    };
    match rest {
        [] if get => {
            ok(json!({"registry": Registry::shipped().describe(), "frame": Frame::default()}))
        }
        ["layout"] if post => with_body(&layout),
        ["editor"] if get => session.view(request),
        ["editor", "open"] if post => with_body(&|b| session.open(b)),
        ["editor", "key"] if post => with_body(&|b| session.key(b)),
        ["editor", "pointer"] if post => with_body(&|b| session.pointer(b)),
        ["editor", "button"] if post => with_body(&|b| session.button(b)),
        ["editor", "sizes"] if post => with_body(&|b| session.sizes(b)),
        ["editor", "close"] if post => session.close(),
        [] | ["editor"] => {
            error_response(405, "method_not_allowed", "that method is not served here")
                .with_header("Allow", "GET")
        }
        ["layout"]
        | [
            "editor",
            "open" | "key" | "pointer" | "button" | "sizes" | "close",
        ] => error_response(405, "method_not_allowed", "that method is not served here")
            .with_header("Allow", "POST"),
        _ => error_response(404, "not_found", "no such endpoint"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, path: &str, body: &str) -> Request {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        Request {
            method: method.into(),
            path: path.into(),
            query: query
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            peer: "127.0.0.1".parse().unwrap(),
        }
    }

    fn call(session: &Session, method: &str, path: &str, body: &str) -> (u16, Value) {
        let r = handle_with(
            session,
            &request(method, path.trim_start_matches("/v1/placement"), body),
            &segments(path),
        );
        (r.status, serde_json::from_slice(&r.body).unwrap())
    }

    fn segments(path: &str) -> Vec<&str> {
        path.split('?')
            .next()
            .unwrap()
            .trim_start_matches("/v1/placement")
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn key(session: &Session, k: &str, t: u64) -> Value {
        let (status, b) = call(
            session,
            "POST",
            "/v1/placement/editor/key",
            &format!(r#"{{"key":"{k}","t":{t}}}"#),
        );
        assert_eq!(status, 200, "{b}");
        b
    }

    #[test]
    fn the_registry_and_the_frame_are_served() {
        let s = Session::default();
        let (status, b) = call(&s, "GET", "/v1/placement", "");
        assert_eq!(status, 200);
        assert_eq!(b["ok"], true);
        assert_eq!(b["registry"]["items"].as_array().unwrap().len(), 6);
        assert_eq!(
            b["frame"],
            json!({"safeX": 64, "safeY": 54, "bar": 60, "top": 134, "limit": 810})
        );
    }

    #[test]
    fn the_layout_is_every_item_where_it_is_for_the_numbers_sent() {
        let s = Session::default();
        let (status, b) = call(&s, "POST", "/v1/placement/layout", "{}");
        assert_eq!(status, 200, "{b}");
        let items = &b["items"];
        assert_eq!(
            items["popup"]["rect"],
            json!({"x": 1328, "y": 32, "w": 560, "h": 469})
        );
        assert_eq!(items["cards"]["rect"]["y"], 32 + 469 + 16);
        // centred at the foot, above the volume bar it stacks on (988 - 16 - 66)
        assert_eq!(
            items["toast"]["rect"],
            json!({"x": 800, "y": 906, "w": 320, "h": 66})
        );
        assert_eq!(
            items["volume"]["rect"],
            json!({"x": 740, "y": 988, "w": 440, "h": 60})
        );
        assert_eq!(items["mute"]["words"], "Corners");
        assert_eq!(items["popup"]["slide"], json!({"x": 1, "y": 0}));
        assert_eq!(
            (
                b["customised"].as_i64(),
                items["mute"].get("spots").is_none()
            ),
            (Some(0), true)
        );

        // the owner's map, Home's own layout, an app in front and a size the toolkit measured
        let sent = r#"{
            "layout": {"safe": {"x": 96, "y": 54}, "statusBar": {"height": 60, "gap": 20}},
            "user": {"popup": {"anchor": "top-left", "dx": 0, "dy": 0}, "volume": {"anchor": "right", "dx": 0, "dy": 0}, "mute": {"anchor": "top-left", "dx": 0, "dy": 0}},
            "sizes": {"popup": {"w": 560, "h": 640}, "volume": {"right": {"w": 96, "h": 440}}},
            "overApp": true, "avoid": [{"x": 100, "y": 120, "w": 200, "h": 40}]
        }"#;
        let (status, b) = call(&s, "POST", "/v1/placement/layout", sent);
        assert_eq!(status, 200, "{b}");
        let items = &b["items"];
        // the column keeps half of the safe margin Home sent (96) from the corner
        assert_eq!(
            items["popup"]["rect"],
            json!({"x": 48, "y": 48, "w": 560, "h": 640})
        );
        assert_eq!(items["popup"]["slide"], json!({"x": -1, "y": 0}));
        assert_eq!(items["volume"]["rect"]["x"], 1920 - 96 - 96);
        assert_eq!(items["volume"]["lift"], 0); // a side edge does not rise over an app
        assert_eq!(b["customised"], 3);
        assert_eq!(
            items["mute"]["spots"].as_array().unwrap().len(),
            2,
            "two of the four places are under the card that is up"
        );
        // an item being moved, not in the map yet
        let (_, b) = call(
            &s,
            "POST",
            "/v1/placement/layout",
            r#"{"work": {"id": "toast", "placement": {"anchor": "bottom-left", "dx": 0, "dy": 0}}}"#,
        );
        assert_eq!(b["items"]["toast"]["rect"]["x"], 24);
        assert_eq!(b["items"]["toast"]["custom"], true);
    }

    #[test]
    fn a_body_that_is_not_an_object_is_refused_and_an_unknown_path_is_not_found() {
        let s = Session::default();
        assert_eq!(call(&s, "POST", "/v1/placement/layout", "[1]").0, 400);
        assert_eq!(call(&s, "POST", "/v1/placement/layout", "{").0, 400);
        assert_eq!(call(&s, "GET", "/v1/placement/nope", "").0, 404);
        assert_eq!(call(&s, "GET", "/v1/placement/layout", "").0, 405);
        assert_eq!(call(&s, "POST", "/v1/placement", "").0, 405);
    }

    #[test]
    fn the_editor_is_driven_by_keys_and_asks_the_caller_to_save() {
        let s = Session::default();
        assert_eq!(
            call(&s, "GET", "/v1/placement/editor", "").1["error"]["code"],
            "no_editor"
        );
        assert_eq!(
            call(&s, "POST", "/v1/placement/editor/key", r#"{"key":"ok"}"#).0,
            409
        );
        let (status, b) = call(
            &s,
            "POST",
            "/v1/placement/editor/open",
            r#"{"item": "popup", "panel": {"w": 760, "h": 300}}"#,
        );
        assert_eq!(status, 200, "{b}");
        assert_eq!(
            (b["view"]["selected"].as_str(), b["view"]["mode"].as_str()),
            (Some("popup"), Some("browse"))
        );
        assert_eq!(b["effects"], json!([]));
        assert!(b["panel"]["x"].is_i64() && b["panel"]["y"].is_i64());
        // pick up, one fine step left, put down
        key(&s, "ok", 1000);
        let b = key(&s, "left", 2000);
        assert_eq!(b["view"]["mode"], "move");
        assert_eq!(b["view"]["items"][2]["rect"]["x"], 1328 - 12);
        let b = key(&s, "ok", 3000);
        let saved = &b["effects"][0]["save"];
        assert_eq!(saved["popup"]["anchor"], "top-right");
        assert!(saved["popup"]["dx"].as_f64().unwrap() < 0.0);
        assert_eq!(
            call(&s, "GET", "/v1/placement/editor?panelW=760&panelH=300", "").1["view"]["customised"],
            1
        );
        // a refusal is told in words
        key(&s, "ok", 4000);
        let mut t = 5000;
        let mut last = Value::Null;
        for _ in 0..300 {
            t += 40;
            last = key(&s, "left", t);
        }
        assert_eq!(last["view"]["notice"], "Mute mark is in the way");
        key(&s, "back", t + 5000);
        // Back from the items closes the editor, and a second request finds none
        let b = key(&s, "back", t + 6000);
        assert_eq!(b["effects"], json!([{"close": true}]));
        assert_eq!(call(&s, "GET", "/v1/placement/editor", "").0, 409);
        assert_eq!(
            call(
                &s,
                "POST",
                "/v1/placement/editor/key",
                r#"{"key":"sideways"}"#
            )
            .0,
            400
        );
    }

    #[test]
    fn the_editor_takes_the_pointer_and_the_buttons() {
        let s = Session::default();
        call(
            &s,
            "POST",
            "/v1/placement/editor/open",
            r#"{"item": "popup", "user": {"toast": {"anchor": "bottom-right", "dx": 0, "dy": 0}}}"#,
        );
        let post = |path: &str, body: &str| {
            call(&s, "POST", &format!("/v1/placement/editor/{path}"), body)
        };
        // press on the camera window, drag it to the middle of the zone, let go
        assert_eq!(
            post(
                "pointer",
                r#"{"phase": "down", "item": "popup", "x": 1396, "y": 214, "t": 1}"#
            )
            .0,
            200
        );
        let (_, b) = post(
            "pointer",
            r#"{"phase": "move", "x": 780, "y": 317.5, "t": 2}"#,
        );
        assert_eq!(b["view"]["mode"], "move");
        let (_, b) = post("pointer", r#"{"phase": "up"}"#);
        assert_eq!(b["effects"][0]["save"]["popup"]["anchor"], "center");
        assert_eq!(
            b["effects"][0]["save"]["toast"]["anchor"], "bottom-right",
            "the whole map is saved"
        );
        assert_eq!(
            post("pointer", r#"{"phase": "down", "x": 1, "y": 2}"#).0,
            400
        );
        assert_eq!(post("pointer", r#"{"phase": "sideways"}"#).0, 400);
        // Reset all through its button, then Done
        let (_, b) = post("button", r#"{"index": 1}"#);
        assert_eq!(b["effects"], json!([{"save": {}}]));
        assert_eq!(post("button", r#"{"index": 7}"#).0, 400);
        // the stand-ins measured again at 200 % text: the camera window is taller
        let (_, b) = post("sizes", r#"{"sizes": {"popup": {"w": 560, "h": 640}}}"#);
        assert_eq!(b["view"]["items"][2]["rect"]["h"], 640);
        let (_, b) = post("button", r#"{"index": 2}"#);
        assert_eq!(b["effects"], json!([{"close": true}]));
        assert_eq!(post("close", "").1["closed"], false);
    }
}
