//! Game Optimizer, a page of Inputs & devices: what LG's own Game Optimizer app changes, as native rows (the map of keys, dimensions and
//! LG's greying rules is `docs/research/game-optimizer.md`). The settings service keeps one value per dimension, so a key is read and
//! written with exactly the dimension it declares: `{gameInput}` for the per-input keys, `{input, pictureMode}` for the game picture modes,
//! `{audioInput: "<port>_game"}` for the game sound. Those go as luna calls of their own, so the page can name an input that is not in
//! front. The per-genre keys follow the live signal, which no call can name: they are written through the shared reader's context and
//! offered only while the chosen input is the one in front. A write shows at once (an override of the row) and is read back after the
//! service answers.

use super::util::{inputs, inputs_read};
use crate::host::sio::js;
use crate::settings::ctx::{Ctx, Spec, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, ChoiceSpec, Op, Pick, Row, settings};
use serde_json::{Map, Value, json};
use std::rc::Rc;

const GET: &str = "luna://com.webos.service.settings/getSystemSettings";
const SET: &str = "luna://com.webos.service.settings/setSystemSettings";
const RESET: &str = "luna://com.webos.service.settings/resetSystemSettings";
const CONFIG: &str = "luna://com.webos.service.config/getConfigs";

const PORTS: [&str; 4] = ["hdmi1", "hdmi2", "hdmi3", "hdmi4"];
/// The game picture modes, each with the document its read is kept under.
const MODES: [(&str, &str); 3] = [("game", "go.pic.game"), ("hdrGame", "go.pic.hdr"), ("dolbyHdrGame", "go.pic.dv")];
const GAME_PICTURE: [&str; 3] = ["game", "hdrGame", "dolbyHdrGame"];

/// Keys stored per input (`{gameInput}`), read for the chosen input whether or not it is in front.
const PER_INPUT: [&str; 14] = [
    "gameGenre",
    "inputOptimization",
    "gameOptimization",
    "freesync",
    "enableALLM",
    "enableQFT",
    "enable144HzBooster",
    "darkMode",
    "blueLight",
    "gameUIColor",
    "gameScreenRatio",
    "gameScreenPosition",
    "enableQuickGame",
    "gameSettingModified",
];
/// What the shared reader asks for: the global keys, the per-genre keys (their value is the live signal's), and the rest for their descriptions.
const SHARED: [&str; 25] = [
    "gameMode",
    "gameDashboardStatusList",
    "gameSettingModified",
    "gameGenre",
    "inputOptimization",
    "gameOptimization",
    "freesync",
    "enableALLM",
    "enableQFT",
    "enable144HzBooster",
    "darkMode",
    "blueLight",
    "gameUIColor",
    "gameScreenRatio",
    "gameScreenPosition",
    "enableQuickGame",
    "blackStabilizer",
    "whiteStabilizer",
    "lowLevelAdjustment",
    "gameAdjustContrast",
    "gameBlackLevel",
    "gameSharpness",
    "gameColorDepth",
    "oledCareMode",
    "gameGenre",
];
/// What Reset puts back for a genre: LG's own list.
const RESET_KEYS: [&str; 6] = ["blackStabilizer", "whiteStabilizer", "gameAdjustContrast", "gameBlackLevel", "gameSharpness", "gameColorDepth"];

const GENRES: [(&str, &str); 6] = [("Standard", "Standard"), ("FPS", "FPS"), ("RPG", "RPG"), ("RTS", "RTS"), ("Sports", "Sports"), ("USER", "User")];
const LEVELS: [(&str, &str); 3] = [("off", "Off"), ("level1", "Level 1"), ("level2", "Level 2")];
const TILES: [(&str, &str); 9] = [
    ("fps", "FPS"),
    ("vrr_aiGameSound_whiteStabilizer", "VRR"),
    ("blackStabilizer", "Black Stabilizer"),
    ("lowLatency", "Low Latency"),
    ("aiGameSound", "AI Game Sound"),
    ("hdr", "HDR"),
    ("resolution", "Resolution"),
    ("dolbyAtmos", "Dolby Atmos"),
    ("inputLag", "Input Lag"),
];
const BANDS: [&str; 10] = ["32 Hz", "64 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz"];

pub fn page() -> Page {
    Page::new("gameopt", "Game Optimizer", "gauge")
        .parent("inputs")
        .back("Inputs & devices")
        .tabs(&["Game", "Picture", "Sound", "Dashboard"])
        .reads(reads)
        .rows(rows)
        .answered(answered)
        .closed(closed)
}

/// The value of the Devices row that opens the page: Off, or the inputs it is on for.
pub fn summary(c: &Ctx) -> String {
    let on = modes(c);
    let ports: Vec<String> = (0..4).filter(|i| on[*i]).map(|i| (i + 1).to_string()).collect();
    if ports.is_empty() { "Off".into() } else { format!("HDMI {}", ports.join(", ")) }
}

fn modes(c: &Ctx) -> [bool; 4] {
    let m = c.lg.get("other.gameMode");
    let mut on = [false; 4];
    for (i, p) in PORTS.iter().enumerate() {
        on[i] = m.and_then(|m| m.get(*p)).and_then(Value::as_str) == Some("on");
    }
    on
}

// ------------------------------------------------------------------------------------------------------------------ what is read

/// One document of the settings service for a dimension: where it is read, how it is written, and the answer kept for it.
#[derive(Clone)]
struct Target {
    doc: &'static str,
    category: &'static str,
    dim: Value,
    app: Option<String>,
}

impl Target {
    fn new(doc: &'static str, category: &'static str, dim: Value) -> Self {
        Target { doc, category, dim, app: None }
    }

    fn app(mut self, app: &str) -> Self {
        self.app = Some(app.to_string());
        self
    }

    fn payload(&self, field: &str, v: Value) -> Value {
        let mut p = json!({"category": self.category, field: v});
        if !self.dim.is_null() {
            p["dimension"] = self.dim.clone();
        }
        if let Some(a) = &self.app {
            p["app_id"] = json!(a);
        }
        p
    }

    fn read(&self, keys: &[&str]) -> Read {
        Read::call(self.doc, GET, self.payload("keys", json!(keys)))
    }

    fn id(&self, key: &str) -> String {
        format!("{}|{}|{}|{key}", self.category, self.dim, self.app.as_deref().unwrap_or(""))
    }

    /// The value in force: the owner's last change while the service has not been read back, else the answer, unless the answer was
    /// given for another dimension.
    fn get(&self, c: &Ctx, key: &str) -> Option<Value> {
        if let Some(v) = c.local.get("go.ov").get(self.id(key)).and_then(|e| e.get("v")) {
            return Some(v.clone());
        }
        let doc = c.docs.get(self.doc);
        if let (Some(echo), Some(want)) = (doc.get("dimension").and_then(Value::as_object), self.dim.as_object())
            && want.iter().any(|(k, v)| echo.get(k).is_some_and(|e| e != v))
        {
            return None;
        }
        doc.get("settings")?.get(key).cloned()
    }

    fn text(&self, c: &Ctx, key: &str) -> Option<String> {
        self.get(c, key).map(|v| js::string(&v))
    }

    fn is_on(&self, c: &Ctx, key: &str) -> Option<bool> {
        self.text(c, key).map(|t| t == "on")
    }
}

/// What the chosen input is: its port, whether Game Optimizer is on for it and whether it is the input in front, with the documents of
/// its keys.
struct Sel {
    i: usize,
    port: &'static str,
    on: bool,
    front: bool,
    /// The chosen input is the one in front and the shared reader's context is that input's.
    live: bool,
    input: Target,
    sound: Target,
    motion: Target,
    pics: Vec<Target>,
    current: Target,
}

fn front_index(c: &Ctx) -> Option<usize> {
    let n: usize = c.input_app()?.strip_prefix("com.webos.app.hdmi")?.parse().ok()?;
    (1..=4).contains(&n).then(|| n - 1)
}

fn sel(c: &Ctx) -> Sel {
    let on = modes(c);
    let i = c
        .local
        .get("go.input")
        .as_str()
        .and_then(|p| PORTS.iter().position(|x| *x == p))
        .or_else(|| front_index(c))
        .or_else(|| (0..4).find(|i| on[*i]))
        .unwrap_or(0);
    let port = PORTS[i];
    let front = front_index(c) == Some(i);
    let live = front && c.lg.dimension("other").and_then(|d| d.get("gameInput")).and_then(Value::as_str) == Some(port);
    Sel {
        i,
        port,
        on: on[i],
        front,
        live,
        input: Target::new("go.in", "other", json!({"gameInput": port})),
        sound: Target::new("go.snd", "sound", json!({"audioInput": format!("{port}_game")})),
        motion: Target::new("go.tm", "picture", Value::Null).app(&format!("com.webos.app.{port}")),
        pics: MODES.iter().map(|(m, doc)| Target::new(doc, "picture", json!({"input": port, "pictureMode": m}))).collect(),
        current: Target::new("go.cur", "picture", Value::Null),
    }
}

fn reads(c: &Ctx) -> Vec<Read> {
    let s = sel(c);
    let mut r = vec![
        Read::lg("other", &SHARED),
        s.input.read(&PER_INPUT),
        Read::call(
            "go.cfg",
            CONFIG,
            json!({"configNames": ["tv.model.motionProMode", "tv.model.supportVRR", "tv.hw.turnkeyVersion", "tv.model.supportHighFrameRateMode", "tv.model.soundModeType"]}),
        ),
        s.current.read(&["pictureMode", "eyeComfortMode"]),
        Target::new("go.sm", "sound", Value::Null).read(&["soundMode"]),
        s.sound.read(&["aigamesound", "gameTVEqualizer", "gameTVEqualizer10band"]),
        s.motion.read(&["truMotionMode"]),
        inputs_read(),
    ];
    r.extend(s.pics.iter().map(|t| t.read(&["motionPro", "hdrDynamicToneMapping"])));
    r
}

fn cfg<'a>(c: &'a Ctx, name: &str) -> Option<&'a str> {
    c.docs.get("go.cfg").get("configs")?.get(name)?.as_str()
}

// ------------------------------------------------------------------------------------------------------------------- what is written

/// The changes one press makes: each is shown at once (an override of the row, until the service has been read back) and sent as a call of
/// the settings service with the dimension of its target.
#[derive(Clone)]
struct Batch {
    ov: Map<String, Value>,
    pending: u64,
    calls: Vec<Op>,
}

impl Batch {
    fn new(c: &Ctx) -> Self {
        Batch { ov: c.local.get("go.ov").as_object().cloned().unwrap_or_default(), pending: c.local.get("go.pending").as_u64().unwrap_or(0), calls: Vec::new() }
    }

    fn put(mut self, t: &Target, pairs: &[(&str, Value)]) -> Self {
        let mut s = Map::new();
        for (k, v) in pairs {
            self.ov.insert(t.id(k), json!({"v": v, "doc": t.doc}));
            s.insert((*k).to_string(), v.clone());
        }
        self.pending += 1;
        self.calls.push(Op::Call { name: "go.w".into(), uri: SET.into(), payload: t.payload("settings", Value::Object(s)), subscribe: false });
        self
    }

    fn ops(self) -> Vec<Op> {
        let mut v = vec![Op::Set("go.ov".into(), Value::Object(self.ov)), Op::Set("go.pending".into(), json!(self.pending))];
        v.extend(self.calls);
        v
    }

    /// The same, sent after a quiet moment (a slider held down sends its last step).
    fn later(self, key: &str, ms: u64) -> Vec<Op> {
        let shown = Op::Set("go.ov".into(), Value::Object(self.ov));
        let mut then = vec![Op::Set("go.pending".into(), json!(self.pending))];
        then.extend(self.calls);
        vec![shown, Op::Later { key: key.into(), ms, then }]
    }
}

fn answered(c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    if name == "go.w" {
        let refused = doc.get("returnValue").and_then(Value::as_bool) == Some(false) || doc.get("errorKey").and_then(Value::as_array).is_some_and(|k| !k.is_empty());
        if refused {
            let why = doc.get("errorText").and_then(Value::as_str).unwrap_or("the TV did not answer");
            return vec![Op::Set("go.ov".into(), Value::Null), Op::Set("go.pending".into(), json!(0)), Op::Set("go.settled".into(), json!(false)), toast(&format!("Not changed: {why}")), Op::RefreshIn(250)];
        }
        let left = c.local.get("go.pending").as_u64().unwrap_or(1).saturating_sub(1);
        let mut ops = vec![Op::Set("go.pending".into(), json!(left))];
        if left == 0 {
            ops.push(Op::Set("go.settled".into(), json!(true)));
            ops.push(Op::RefreshIn(250));
        }
        return ops;
    }
    // a document read after the last write came back: the service's value replaces the owner's change
    if name.starts_with("go.") && c.local.flag("go.settled") {
        let mut ov = c.local.get("go.ov").as_object().cloned().unwrap_or_default();
        ov.retain(|_, e| e.get("doc").and_then(Value::as_str) != Some(name));
        let done = ov.is_empty();
        let mut ops = vec![Op::Set("go.ov".into(), Value::Object(ov))];
        if done {
            ops.push(Op::Set("go.settled".into(), json!(false)));
        }
        return ops;
    }
    Vec::new()
}

fn closed(_: &Ctx) -> Vec<Op> {
    ["go.input", "go.ov", "go.pending", "go.settled"].iter().map(|k| Op::Set((*k).into(), Value::Null)).collect()
}

// ------------------------------------------------------------------------------------------------------------------------- rows

fn grey(mut r: Row, why: &str) -> Row {
    r.enabled = false;
    if !why.is_empty() {
        r.detail = why.into();
    }
    r
}

/// The common rule of a per-input row: grey while Game Optimizer is off for the input (one note says so) or the service has no value.
fn gated(mut r: Row, s: &Sel, known: bool) -> Row {
    if !s.on {
        r = grey(r, "");
    }
    if !known {
        r = grey(r, "Not available now");
    }
    r
}

fn desc_shown(c: &Ctx, ds: &str) -> bool {
    c.lg.desc(ds).is_some_and(|d| d.visible != Some(false))
}

fn input_row(c: &Ctx, s: &Sel, id: &str, tab: usize, device: &str) -> Row {
    let _ = c;
    Row::seg(id, &["HDMI 1", "HDMI 2", "HDMI 3", "HDMI 4"]).tab(tab).icon("cable").label("Input").detail(device).index(Some(s.i)).act(|a| match a {
        Arg::Index(i) => PORTS.get(i).map(|p| vec![Op::Set("go.input".into(), json!(p)), Op::Refresh]).unwrap_or_default(),
        _ => Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn switch(c: &Ctx, s: &Sel, t: &Target, key: &str, tab: usize, icon: &str, label: &str, help: &str) -> Row {
    let cur = t.is_on(c, key);
    let (base, t, key) = (Batch::new(c), t.clone(), key.to_string());
    let r = Row::toggle(&format!("{}.{key}", t.category)).tab(tab).icon(icon).label(label).help(help).on(cur == Some(true)).admin().act(move |a| match a {
        Arg::Toggle(v) => base.clone().put(&t, &[(&key, json!(if v { "on" } else { "off" }))]).ops(),
        _ => Vec::new(),
    });
    gated(r, s, cur.is_some())
}

#[allow(clippy::too_many_arguments)]
fn choice(c: &Ctx, s: &Sel, key: &str, tab: usize, icon: &str, label: &str, help: &str, options: &[(&str, &str)]) -> Row {
    let cur = s.input.text(c, key);
    let (base, t, key) = (Batch::new(c), s.input.clone(), key.to_string());
    let values: Vec<String> = options.iter().map(|o| o.0.to_string()).collect();
    let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
    let index = cur.as_ref().and_then(|v| values.iter().position(|x| x == v));
    let r = Row::seg(&format!("other.{key}"), &labels).tab(tab).icon(icon).label(label).help(help).index(index).admin().act(move |a| match a {
        Arg::Index(i) => values.get(i).map(|v| base.clone().put(&t, &[(&key, json!(v))]).ops()).unwrap_or_default(),
        _ => Vec::new(),
    });
    gated(r, s, index.is_some())
}

fn rows(c: &Ctx) -> Vec<Row> {
    let s = sel(c);
    let list = inputs(c);
    let device = match list.list().iter().find(|e| e.app_id == format!("com.webos.app.{}", s.port)) {
        Some(e) if e.connected && !e.label.is_empty() => e.label.clone(),
        Some(e) if e.connected => "Connected".into(),
        _ => "Nothing connected".into(),
    };
    let mut rows = Vec::new();
    game_tab(c, &s, &device, &mut rows);
    picture_tab(c, &s, &device, &mut rows);
    sound_tab(c, &s, &device, &mut rows);
    dashboard_tab(c, &s, &device, &mut rows);
    rows
}

// ----------------------------------------------------------------------------------------------------------------------- Game tab

fn game_tab(c: &Ctx, s: &Sel, device: &str, rows: &mut Vec<Row>) {
    rows.push(input_row(c, s, "input", 0, device));
    let ready = c.lg.has("other.gameMode") && c.lg.desc("other.gameMode").is_some();
    let on = modes(c);
    let i = s.i;
    let whole = c.lg.get("other.gameMode").and_then(Value::as_object).cloned().unwrap_or_default();
    let mode = Row::toggle("mode")
        .tab(0)
        .icon("gauge")
        .label("Game Optimizer")
        .help("Tunes this input for games: lower delay, variable refresh and the game picture and sound options below. LG turns it on per input.")
        .on(on[i])
        .admin()
        .act(move |a| match a {
            Arg::Toggle(v) => {
                let mut m = whole.clone();
                for p in PORTS {
                    m.entry(p).or_insert(json!("off"));
                }
                m.insert(PORTS[i].into(), json!(if v { "on" } else { "off" }));
                vec![Op::LgMany { category: "other".into(), settings: settings(&[("gameMode", Value::Object(m))]), app: None }, Op::RefreshIn(400)]
            }
            _ => Vec::new(),
        });
    rows.push(if ready { mode } else { grey(mode, "Not available now") });
    if ready && !s.on {
        rows.push(Row::note("Turn on Game Optimizer to change the rest.", 0));
    }

    let genre = s.input.text(c, "gameGenre");
    let changed = genre.as_deref().is_some_and(|g| s.input.get(c, "gameSettingModified").and_then(|m| m.get(g).and_then(Value::as_bool)) == Some(true));
    let mut g = choice(c, s, "gameGenre", 0, "trophy", "Game Genre", "The picture and sound mode must be Game for a genre to apply.", &GENRES);
    if changed && s.on {
        g.detail = "Settings changed".into();
    }
    rows.push(g);
    rows.push(choice(
        c,
        s,
        "inputOptimization",
        0,
        "timer",
        "Prevent Input Delay",
        "Boost cuts the delay further; Standard keeps more of the picture processing. Back to Standard while Game Optimizer is off.",
        &[("auto", "Standard"), ("on", "Boost")],
    ));
    let vrr = if cfg(c, "tv.model.supportVRR") == Some("Gsync") { "VRR & G-Sync" } else { "VRR" };
    rows.push(switch(c, s, &s.input, "gameOptimization", 0, "activity", vrr, "Matches the screen's refresh to the game's frame rate, so motion stays smooth and tearing is gone."));
    let sync = if cfg(c, "tv.hw.turnkeyVersion") == Some("T2.0") { "AMD FreeSync" } else { "AMD FreeSync Premium" };
    let mut free = switch(c, s, &s.input, "freesync", 0, "activity", sync, "Variable refresh for sources that use AMD FreeSync.");
    free.visible = desc_shown(c, "other.freesync");
    rows.push(free);
    rows.push(switch(c, s, &s.input, "enableALLM", 0, "zap", "Instant Game Response (ALLM)", "The TV switches to its lowest-delay settings when a console asks for it."));
    let mut qft = switch(c, s, &s.input, "enableQFT", 0, "fast-forward", "Quick Frame Transport", "Sends frames sooner when the source supports it.");
    qft.visible = desc_shown(c, "other.enableQFT");
    rows.push(qft);
    let hz: String = cfg(c, "tv.model.supportHighFrameRateMode").unwrap_or("").chars().take_while(char::is_ascii_digit).collect();
    let hz = if hz.is_empty() { "144".to_string() } else { hz };
    let mut boost = switch(
        c,
        s,
        &s.input,
        "enable144HzBooster",
        0,
        "signal",
        &format!("{hz}Hz Mode"),
        &format!("A smoother picture while playing. If the game or PC does not support {hz}Hz, video and audio quality may drop. LG asks to keep the change; turn it off here if the picture does not return."),
    );
    boost.visible = desc_shown(c, "other.enable144HzBooster");
    rows.push(boost);
}

// -------------------------------------------------------------------------------------------------------------------- Picture tab

fn picture_tab(c: &Ctx, s: &Sel, device: &str, rows: &mut Vec<Row>) {
    rows.push(input_row(c, s, "input.p", 1, device));
    rows.push(motion(c, s));

    let mut tone = {
        let t = &s.pics[1];
        let cur = t.text(c, "hdrDynamicToneMapping");
        let values = ["on", "off", "HGIG"];
        let index = cur.as_ref().and_then(|v| values.iter().position(|x| x == v));
        let (base, t) = (Batch::new(c), t.clone());
        let r = Row::seg("picture.hdrDynamicToneMapping", &["On", "Off", "HGiG"])
            .tab(1)
            .icon("contrast")
            .label("Dynamic Tone Mapping")
            .help("On adapts bright HDR highlights to the screen. HGiG follows the console's own tone mapping, for games that offer it.")
            .index(index)
            .admin()
            .act(move |a| match a {
                Arg::Index(i) => values.get(i).map(|v| base.clone().put(&t, &[("hdrDynamicToneMapping", json!(v))]).ops()).unwrap_or_default(),
                _ => Vec::new(),
            });
        gated(r, s, index.is_some())
    };
    tone.help = tone.help.clone();
    rows.push(tone);

    rows.push(choice(c, s, "darkMode", 1, "moon", "Dark Room Mode", "Dims the picture for a dark room.", &LEVELS));
    let mut blue = choice(c, s, "blueLight", 1, "sun-dim", "Reduce Blue Light", "Warms the picture to be easier on the eyes.", &LEVELS);
    if s.input.text(c, "gameOptimization").is_some() && s.current.text(c, "eyeComfortMode").is_some_and(|m| m != "off") {
        blue = grey(blue, "Eye Comfort is on");
    }
    rows.push(blue);
    rows.push(choice(c, s, "gameScreenRatio", 1, "maximize-2", "Wide Aspect Ratio", "For a game that sends an ultra-wide picture.", &[("16:9", "16:9"), ("21:9", "21:9"), ("32:9", "32:9")]));
    rows.push(choice(c, s, "gameScreenPosition", 1, "scan-eye", "Screen Position", "Where a narrower picture sits on the screen.", &[("top", "Top"), ("middle", "Middle"), ("bottom", "Bottom")]));

    let genre = c.lg.opt_text("other.gameGenre").unwrap_or_else(|| "Standard".into());
    rows.push(Row::head(&if s.live { format!("{} settings", GENRES.iter().find(|g| g.0 == genre).map_or(genre.as_str(), |g| g.1)) } else { "Genre settings".into() }, 1));
    if !s.live {
        rows.push(Row::note("These follow the game's genre. Put this input in front and start the game to change them.", 1));
    }
    let free = s.input.text(c, "freesync").as_deref() == Some("on");
    let vrr = s.input.text(c, "gameOptimization").as_deref() == Some("on");
    rows.push(genre_slider(c, s, "blackStabilizer", "Black Stabilizer", "moon-star", 0.0, 20.0, None));
    rows.push(genre_slider(c, s, "whiteStabilizer", "White Stabilizer", "sun-dim", 0.0, 20.0, None));
    rows.push(genre_slider(c, s, "lowLevelAdjustment", "Fine Tune Dark Areas", "moon", -30.0, 30.0, (!(vrr || free)).then_some("Turn on VRR or FreeSync")));
    rows.push(genre_slider(c, s, "gameAdjustContrast", "Game Contrast", "contrast", 0.0, 100.0, None));
    rows.push(genre_slider(c, s, "gameBlackLevel", "Game Black Level", "moon-star", 0.0, 100.0, None));
    rows.push(genre_slider(c, s, "gameSharpness", "Game Sharpness", "scan-eye", 0.0, 50.0, None));
    rows.push(genre_slider(c, s, "gameColorDepth", "Game Color Depth", "palette", 0.0, 100.0, None));

    let dim = c.lg.dimension("other").cloned().unwrap_or(Value::Null);
    let marks = {
        let mut m = c.lg.get("other.gameSettingModified").and_then(Value::as_object).cloned().unwrap_or_default();
        m.insert(genre.clone(), json!(false));
        Value::Object(m)
    };
    let mut reset = Row::action("g.reset", "Reset").tab(1).icon("rotate-ccw").label(&format!("Reset {genre}")).help("Puts this genre's settings above back to LG's defaults.").admin().danger();
    reset = reset.does(ask(
        "rotate-ccw",
        &format!("Reset {genre}?"),
        "The stabilizers and the game picture settings of this genre go back to LG's defaults.",
        "Reset",
        "Cancel",
        true,
        vec![
            Op::LgMany { category: "other".into(), settings: settings(&[("gameSettingModified", marks)]), app: None },
            Op::Act { uri: RESET.into(), payload: json!({"category": "other", "keys": RESET_KEYS, "dimension": dim}) },
            Op::Later { key: "go.rb".into(), ms: 400, then: vec![Op::Refresh] },
        ],
    ));
    if !s.live || !s.on {
        reset = grey(reset, "");
    }
    rows.push(reset);
}

#[allow(clippy::too_many_arguments)]
fn genre_slider(c: &Ctx, s: &Sel, key: &str, label: &str, icon: &str, min: f64, max: f64, why: Option<&str>) -> Row {
    let ds = format!("other.{key}");
    let mut r = c.range(Spec::new(&ds).id(&format!("genre.{key}")).tab(1).icon(icon).label(label).range(min, max, 1.0));
    let genre = c.lg.opt_text("other.gameGenre").unwrap_or_else(|| "Standard".into());
    let mut marks = c.lg.get("other.gameSettingModified").and_then(Value::as_object).cloned().unwrap_or_default();
    marks.insert(genre, json!(true));
    let key = key.to_string();
    r = r.act(move |a| match a {
        Arg::Value(v) => vec![
            Op::LgMany { category: "other".into(), settings: settings(&[(&key, json!(v.round() as i64))]), app: None },
            Op::LgMany { category: "other".into(), settings: settings(&[("gameSettingModified", Value::Object(marks.clone()))]), app: None },
            Op::Later { key: "go.rb".into(), ms: 600, then: vec![Op::Refresh] },
        ],
        _ => Vec::new(),
    });
    if !s.live {
        r.value_text = Some("\u{2013}".into());
        r = grey(r, "");
        r.detail.clear();
    } else if !s.on {
        r = grey(r, "");
        r.detail.clear();
    } else if let Some(w) = why {
        r = grey(r, w);
    }
    r
}

/// OLED Motion (or Pro, as the TV names it): one switch for the three game picture modes. It counts only while TruMotion is on User, so
/// turning it on sets that first.
fn motion(c: &Ctx, s: &Sel) -> Row {
    let label = if cfg(c, "tv.model.motionProMode") == Some("OLED Motion Pro") { "OLED Motion Pro" } else { "OLED Motion" };
    let tm = s.motion.text(c, "truMotionMode");
    let known = s.pics.iter().any(|t| t.get(c, "motionPro").is_some());
    let any_on = s.pics.iter().any(|t| t.is_on(c, "motionPro") == Some(true));
    let (base, motion, pics) = (Batch::new(c), s.motion.clone(), s.pics.clone());
    let user = tm.as_deref() == Some("user");
    let r = Row::toggle("picture.motionPro")
        .tab(1)
        .icon("waves")
        .label(label)
        .help("Reduces blur in fast motion by dimming the screen a little. Turning it on sets TruMotion to User for this input.")
        .on(any_on && user)
        .admin()
        .act(move |a| match a {
            Arg::Toggle(v) => {
                let mut b = base.clone();
                if v && !user {
                    b = b.put(&motion, &[("truMotionMode", json!("user"))]);
                }
                for t in &pics {
                    b = b.put(t, &[("motionPro", json!(if v { "on" } else { "off" }))]);
                }
                b.ops()
            }
            _ => Vec::new(),
        });
    gated(r, s, known)
}

// ---------------------------------------------------------------------------------------------------------------------- Sound tab

/// The sound mode in force, known only for the input in front.
fn front_sound_mode(c: &Ctx, s: &Sel) -> Option<String> {
    if !s.front {
        return None;
    }
    c.docs.get("go.sm").get("settings")?.get("soundMode").map(js::string)
}

fn sound_tab(c: &Ctx, s: &Sel, device: &str, rows: &mut Vec<Row>) {
    rows.push(input_row(c, s, "input.s", 2, device));
    let mode = front_sound_mode(c, s);
    let mut ai = switch(c, s, &s.sound, "aigamesound", 2, "audio-lines", "AI Game Sound", "Shapes the sound for games: footsteps and voices stand out.");
    if mode.as_deref().is_some_and(|m| m != "game") {
        ai = grey(ai, "Sound mode is not Game");
    }
    rows.push(ai);
    let ai_on = s.sound.is_on(c, "aigamesound") == Some(true);
    let mut eq = switch(c, s, &s.sound, "gameTVEqualizer", 2, "sliders-vertical", "Game Equalizer", "Sets the ten bands of the game sound yourself. It replaces AI Game Sound.");
    if ai_on {
        eq = grey(eq, "Turn off AI Game Sound");
    }
    let eq_on = s.sound.is_on(c, "gameTVEqualizer") == Some(true);
    rows.push(eq);
    if !eq_on {
        return;
    }
    let bands: Vec<f64> = s.sound.get(c, "gameTVEqualizer10band").and_then(|v| v.as_array().map(|a| a.iter().map(|x| js::float(x).unwrap_or(0.0)).collect())).unwrap_or_default();
    for (n, name) in BANDS.iter().enumerate() {
        let known = bands.len() == 10;
        let (base, t, now) = (Batch::new(c), s.sound.clone(), bands.clone());
        let mut r = Row::slider(&format!("eq.{n}"), -10.0, 10.0, 1.0).tab(2).icon("sliders-vertical").label(name).admin();
        r.num = now.get(n).copied().unwrap_or(0.0);
        r.value_text = Some(format!("{}", r.num.round() as i64));
        r = r.act(move |a| match a {
            Arg::Value(v) if now.len() == 10 => {
                let mut next = now.clone();
                next[n] = v.round();
                base.clone().put(&t, &[("gameTVEqualizer10band", json!(next.iter().map(|x| *x as i64).collect::<Vec<_>>()))]).later("go.eq", 300)
            }
            _ => Vec::new(),
        });
        rows.push(gated(r, s, known));
    }
}

// ----------------------------------------------------------------------------------------------------------------- Dashboard tab

fn dashboard_tab(c: &Ctx, s: &Sel, device: &str, rows: &mut Vec<Row>) {
    rows.push(input_row(c, s, "input.d", 3, device));
    let mut board = switch(c, s, &s.input, "enableQuickGame", 3, "gauge", "Game Dashboard", "Shows frame rate and signal tiles over the game.");
    let picture = s.current.text(c, "pictureMode");
    let sound = front_sound_mode(c, s);
    if let (Some(p), Some(m)) = (picture, sound)
        && s.front
        && !GAME_PICTURE.contains(&p.as_str())
        && m != "game"
    {
        board = grey(board, "Needs the Game picture or sound mode");
    }
    rows.push(board);

    let list: Vec<String> = c.lg.get("other.gameDashboardStatusList").and_then(Value::as_array).map(|a| a.iter().map(js::string).collect()).unwrap_or_default();
    let atmos = cfg(c, "tv.model.soundModeType") == Some("Dolby Atmos");
    let offered: Vec<(&str, &str)> = TILES.iter().copied().filter(|t| t.0 != "dolbyAtmos" || atmos).collect();
    let ready = list.len() == 4 && c.lg.desc("other.gameDashboardStatusList").is_some();
    for slot in 0..4 {
        let current = list.get(slot).cloned().unwrap_or_default();
        let name = TILES.iter().find(|t| t.0 == current).map_or(current.as_str(), |t| t.1).to_string();
        let (offer, whole) = (offered.clone(), list.clone());
        let title = format!("Tile {}", slot + 1);
        let r = Row::link(&format!("tile.{slot}")).tab(3).icon("layout-grid").label(&title).value(&name).admin().act(move |_| {
            let whole = whole.clone();
            let cur = whole.get(slot).cloned().unwrap_or_default();
            vec![Op::Pick(Rc::new(ChoiceSpec {
                title: format!("Tile {}", slot + 1),
                options: offer.iter().map(|(id, label)| Pick::new(id, label)).collect(),
                current: cur,
                write: Box::new(move |v| {
                    let mut next = whole.clone();
                    match next.iter().position(|t| t == v) {
                        Some(j) => next.swap(slot, j),
                        None => next[slot] = v.to_string(),
                    }
                    vec![Op::LgMany { category: "other".into(), settings: settings(&[("gameDashboardStatusList", json!(next))]), app: None }, Op::RefreshIn(400)]
                }),
            }))]
        });
        rows.push(if ready { r } else { grey(r, "Not available now") });
    }
    rows.push(choice(c, s, "gameUIColor", 3, "palette", "Menu Color", "The colour of the dashboard and of LG's game menu.", &[("violet", "Violet"), ("orange", "Orange"), ("green", "Green")]));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

    fn press(r: &Row, a: Arg) -> Vec<Op> {
        (r.act.as_ref().unwrap())(a)
    }

    fn shared(mode: Value, dim: Value) -> Value {
        json!({"settings": {"gameMode": mode, "gameGenre": "FPS", "blackStabilizer": 10, "whiteStabilizer": 10, "lowLevelAdjustment": 0, "gameAdjustContrast": 85,
                            "gameBlackLevel": 50, "gameSharpness": 10, "gameColorDepth": 55, "gameSettingModified": {"FPS": false, "Standard": false},
                            "gameDashboardStatusList": ["fps", "vrr_aiGameSound_whiteStabilizer", "blackStabilizer", "lowLatency"]}, "dimension": dim})
    }

    fn descs() -> Vec<(&'static str, Value)> {
        vec![(
            "other",
            json!({"results": [
                {"category": "other", "key": "gameMode", "dimension": [], "ui": {"visible": true, "active": true}},
                {"category": "other", "key": "gameDashboardStatusList", "dimension": [], "ui": {"visible": true, "active": true}},
                {"category": "other", "key": "gameSettingModified", "dimension": ["gameInput"], "ui": {"visible": true, "active": true}},
                {"category": "other", "key": "blackStabilizer", "dimension": ["gameInput", "dynamicRange", "gameGenre"], "ui": {"visible": true, "active": true}, "values": {"range": {"min": 0, "max": 20, "interval": 1}}},
                {"category": "other", "key": "lowLevelAdjustment", "dimension": ["gameInput", "dynamicRange", "gameGenre"], "ui": {"visible": true, "active": true}, "values": {"range": {"min": -30, "max": 30, "interval": 1}}},
                {"category": "other", "key": "enableQFT", "dimension": ["gameInput"], "ui": {"visible": false, "active": true}},
                {"category": "other", "key": "freesync", "dimension": ["gameInput"], "ui": {"visible": true, "active": true}}]}),
        )]
    }

    /// HDMI 2 has Game Optimizer on and is in front with the live context; HDMI 1 is off.
    fn world(front: bool) -> World {
        let dim = if front { json!({"gameInput": "hdmi2", "gameGenre": "FPS", "dynamicRange": "sdr"}) } else { json!({"gameInput": "none", "gameGenre": "Standard", "dynamicRange": "sdr"}) };
        let mut w = World::new(lg_with(&[("other", shared(json!({"hdmi1": "off", "hdmi2": "on", "hdmi3": "off", "hdmi4": "off"}), dim))], &descs()));
        w.docs.set("go.in", json!({"dimension": {"gameInput": "hdmi2"}, "settings": {"gameGenre": "FPS", "inputOptimization": "auto", "gameOptimization": "on", "freesync": "off", "enableALLM": "on",
            "enableQFT": "off", "enable144HzBooster": "off", "darkMode": "off", "blueLight": "off", "gameUIColor": "violet", "gameScreenRatio": "16:9", "gameScreenPosition": "middle",
            "enableQuickGame": "on", "gameSettingModified": {"FPS": true}}}));
        w.docs.set("go.snd", json!({"dimension": {"audioInput": "hdmi2_game"}, "settings": {"aigamesound": "off", "gameTVEqualizer": "on", "gameTVEqualizer10band": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]}}));
        w.docs.set("go.tm", json!({"settings": {"truMotionMode": "off"}}));
        w.docs.set("go.pic.game", json!({"dimension": {"input": "hdmi2", "pictureMode": "game"}, "settings": {"motionPro": "off"}}));
        w.docs.set("go.pic.hdr", json!({"dimension": {"input": "hdmi2", "pictureMode": "hdrGame"}, "settings": {"motionPro": "off", "hdrDynamicToneMapping": "on"}}));
        w.docs.set("go.pic.dv", json!({"dimension": {"input": "hdmi2", "pictureMode": "dolbyHdrGame"}, "settings": {"motionPro": "off"}}));
        w.docs.set("go.cur", json!({"settings": {"pictureMode": "game", "eyeComfortMode": "off"}}));
        w.docs.set("go.cfg", json!({"configs": {"tv.model.motionProMode": "OLED Motion", "tv.model.supportVRR": "Gsync", "tv.hw.turnkeyVersion": "LG"}}));
        if front {
            w.topics.set_channel("foreground", json!({"app": "com.webos.app.hdmi2"}));
        }
        w
    }

    fn calls(ops: &[Op]) -> Vec<(String, Value)> {
        ops.iter().filter_map(|o| if let Op::Call { name, payload, .. } = o { Some((name.clone(), payload.clone())) } else { None }).collect()
    }

    #[test]
    fn the_chosen_input_is_the_owners_pick_then_the_one_in_front_then_the_first_that_is_on() {
        let mut w = world(false);
        assert_eq!(sel(&w.ctx()).port, "hdmi2", "the first with Game Optimizer on");
        w.topics.set_channel("foreground", json!({"app": "com.webos.app.hdmi3"}));
        assert_eq!(sel(&w.ctx()).port, "hdmi3", "the one in front");
        w.local.set("go.input", json!("hdmi1"));
        let c = w.ctx();
        assert_eq!(sel(&c).port, "hdmi1", "the owner's pick");
        let r = rows(&c);
        let ops = press(by_id(&r, "input"), Arg::Index(3));
        assert!(matches!(&ops[..], [Op::Set(k, v), Op::Refresh] if k == "go.input" && v == "hdmi4"));
        assert!(closed(&c).iter().any(|o| matches!(o, Op::Set(k, v) if k == "go.input" && v.is_null())), "the pick is let go when the page closes");
        assert_eq!(summary(&c), "HDMI 2");
    }

    #[test]
    fn the_master_switch_writes_the_whole_game_mode_object_and_off_greys_the_rest_under_one_note() {
        let w = world(false);
        let r = rows(&w.ctx());
        let ops = press(by_id(&r, "mode"), Arg::Toggle(false));
        let [Op::LgMany { category, settings, .. }, Op::RefreshIn(400)] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!((category.as_str(), &settings["gameMode"]), ("other", &json!({"hdmi1": "off", "hdmi2": "off", "hdmi3": "off", "hdmi4": "off"})));
        let ops = press(by_id(&r, "mode"), Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::LgMany { settings, .. }, _] if settings["gameMode"]["hdmi2"] == "on"));
        assert!(by_id(&r, "other.gameOptimization").enabled && !r.iter().any(|x| x.id.starts_with("note:Turn on")));
        let mut w = world(false);
        w.local.set("go.input", json!("hdmi1"));
        let r = rows(&w.ctx());
        assert!(!by_id(&r, "other.gameOptimization").enabled && !by_id(&r, "other.inputOptimization").enabled && by_id(&r, "mode").enabled);
        assert_eq!(r.iter().filter(|x| x.id.starts_with("note:Turn on Game Optimizer")).count(), 1);
        assert_eq!(summary(&w.ctx()), "HDMI 2");
    }

    #[test]
    fn a_per_input_write_names_the_dimension_shows_at_once_and_is_read_back_after_the_answer() {
        let mut w = world(false);
        let r = rows(&w.ctx());
        let vrr = by_id(&r, "other.gameOptimization");
        assert_eq!(vrr.label, "VRR & G-Sync");
        let ops = press(vrr, Arg::Toggle(false));
        let c = calls(&ops);
        assert_eq!(c[0].0, "go.w");
        assert_eq!(c[0].1, json!({"category": "other", "settings": {"gameOptimization": "off"}, "dimension": {"gameInput": "hdmi2"}}));
        // the owner's change is on screen before the service has answered
        for op in &ops {
            if let Op::Set(k, v) = op {
                w.local.set(k, v.clone());
            }
        }
        assert!(!by_id(&rows(&w.ctx()), "other.gameOptimization").on);
        // the answer: when the last write is back, the read follows and the override goes with the read-back document
        let ops = answered(&w.ctx(), "go.w", &json!({"returnValue": true}));
        assert!(ops.iter().any(|o| matches!(o, Op::RefreshIn(250))));
        for op in &ops {
            if let Op::Set(k, v) = op {
                w.local.set(k, v.clone());
            }
        }
        let ops = answered(&w.ctx(), "go.in", &json!({}));
        let Some(Op::Set(_, ov)) = ops.first() else { panic!("{ops:?}") };
        assert_eq!(ov, &json!({}));
        // a refused write clears the override and says why
        let ops = answered(&w.ctx(), "go.w", &json!({"returnValue": false, "errorText": "No such key"}));
        assert!(ops.iter().any(|o| matches!(o, Op::Act { payload, .. } if payload["message"] == "Not changed: No such key")));
        assert!(matches!(&ops[0], Op::Set(k, v) if k == "go.ov" && v.is_null()));
    }

    #[test]
    fn an_answer_given_for_another_input_is_not_this_inputs_value() {
        let mut w = world(false);
        w.docs.set("go.in", json!({"dimension": {"gameInput": "hdmi3"}, "settings": {"gameOptimization": "on"}}));
        let r = rows(&w.ctx());
        let vrr = by_id(&r, "other.gameOptimization");
        assert!(!vrr.enabled && vrr.detail == "Not available now");
    }

    #[test]
    fn genre_rows_are_greyed_unless_the_chosen_input_is_in_front_and_numbers_are_written_as_numbers_with_the_genre_marked() {
        let w = world(false);
        let r = rows(&w.ctx());
        let black = by_id(&r, "genre.blackStabilizer");
        assert!(!black.enabled && black.value_text.as_deref() == Some("\u{2013}"));
        assert!(!by_id(&r, "g.reset").enabled);
        assert!(r.iter().any(|x| x.id.starts_with("note:These follow the game's genre")));

        let w = world(true);
        let r = rows(&w.ctx());
        let black = by_id(&r, "genre.blackStabilizer");
        assert!(black.enabled && black.value_text.as_deref() == Some("10") && (black.min, black.max) == (0.0, 20.0));
        let ops = press(black, Arg::Value(12.0));
        let [Op::LgMany { settings: a, .. }, Op::LgMany { settings: b, .. }, Op::Later { .. }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(a["blackStabilizer"], json!(12), "a number, not a string");
        assert_eq!(b["gameSettingModified"], json!({"FPS": true, "Standard": false}));
        assert!(by_id(&r, "genre.lowLevelAdjustment").enabled, "VRR is on");
        let reset = press(by_id(&r, "g.reset"), Arg::Press);
        let [Op::Ask(ask)] = &reset[..] else { panic!() };
        let then = &ask.answers[1].then;
        assert!(matches!(&then[1], Op::Act { uri, payload } if uri.ends_with("/resetSystemSettings") && payload["dimension"]["gameInput"] == "hdmi2"));
        assert!(matches!(&then[0], Op::LgMany { settings, .. } if settings["gameSettingModified"]["FPS"] == json!(false)));
    }

    #[test]
    fn fine_tune_dark_areas_waits_for_vrr_or_freesync_and_qft_stays_out_while_lg_hides_it() {
        let mut w = world(true);
        w.docs.set("go.in", json!({"dimension": {"gameInput": "hdmi2"}, "settings": {"gameOptimization": "off", "freesync": "off"}}));
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "genre.lowLevelAdjustment").detail, "Turn on VRR or FreeSync");
        assert!(!by_id(&r, "genre.lowLevelAdjustment").enabled);
        assert!(!by_id(&r, "other.enableQFT").visible, "the description says it is not visible");
        assert!(by_id(&r, "other.freesync").visible);
        assert_eq!(by_id(&r, "other.freesync").label, "AMD FreeSync Premium");
    }

    #[test]
    fn oled_motion_sets_truemotion_to_user_first_and_then_the_three_game_modes() {
        let w = world(false);
        let r = rows(&w.ctx());
        let m = by_id(&r, "picture.motionPro");
        assert_eq!(m.label, "OLED Motion");
        let c = calls(&press(m, Arg::Toggle(true)));
        assert_eq!(c.len(), 4);
        assert_eq!(c[0].1, json!({"category": "picture", "settings": {"truMotionMode": "user"}, "app_id": "com.webos.app.hdmi2"}));
        assert_eq!(c[1].1["dimension"], json!({"input": "hdmi2", "pictureMode": "game"}));
        assert_eq!(c[2].1["dimension"]["pictureMode"], "hdrGame");
        assert_eq!(c[3].1, json!({"category": "picture", "settings": {"motionPro": "on"}, "dimension": {"input": "hdmi2", "pictureMode": "dolbyHdrGame"}}));
        // already on User: only the switch; turning off leaves TruMotion alone
        let mut w = world(false);
        w.docs.set("go.tm", json!({"settings": {"truMotionMode": "user"}}));
        let r = rows(&w.ctx());
        assert_eq!(calls(&press(by_id(&r, "picture.motionPro"), Arg::Toggle(true))).len(), 3);
        w.docs.set("go.pic.hdr", json!({"dimension": {"input": "hdmi2", "pictureMode": "hdrGame"}, "settings": {"motionPro": "on", "hdrDynamicToneMapping": "HGIG"}}));
        let r = rows(&w.ctx());
        assert!(by_id(&r, "picture.motionPro").on);
        assert_eq!(by_id(&r, "picture.hdrDynamicToneMapping").index, Some(2));
        assert!(calls(&press(by_id(&r, "picture.motionPro"), Arg::Toggle(false))).iter().all(|(_, p)| p["settings"] == json!({"motionPro": "off"})));
    }

    #[test]
    fn reduce_blue_light_waits_for_eye_comfort_to_be_off() {
        let mut w = world(false);
        w.docs.set("go.cur", json!({"settings": {"pictureMode": "game", "eyeComfortMode": "on"}}));
        let r = rows(&w.ctx());
        assert!(!by_id(&r, "other.blueLight").enabled && by_id(&r, "other.blueLight").detail == "Eye Comfort is on");
    }

    #[test]
    fn the_equalizer_excludes_ai_game_sound_and_a_band_writes_the_whole_array_after_a_quiet_moment() {
        let mut w = world(false);
        w.docs.set("go.snd", json!({"dimension": {"audioInput": "hdmi2_game"}, "settings": {"aigamesound": "on", "gameTVEqualizer": "off", "gameTVEqualizer10band": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]}}));
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "sound.gameTVEqualizer").detail, "Turn off AI Game Sound");
        assert!(!r.iter().any(|x| x.id.starts_with("eq.")), "the bands show only while the equalizer is on");
        let w = world(false);
        let r = rows(&w.ctx());
        assert_eq!(r.iter().filter(|x| x.id.starts_with("eq.")).count(), 10);
        assert_eq!(by_id(&r, "eq.5").label, "1 kHz");
        let ops = press(by_id(&r, "eq.2"), Arg::Value(4.0));
        let [Op::Set(..), Op::Later { key, ms: 300, then }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(key, "go.eq");
        assert_eq!(calls(then)[0].1, json!({"category": "sound", "settings": {"gameTVEqualizer10band": [0, 0, 4, 0, 0, 0, 0, 0, 0, 0]}, "dimension": {"audioInput": "hdmi2_game"}}));
    }

    #[test]
    fn a_dashboard_tile_used_elsewhere_swaps_and_the_whole_list_is_written() {
        let w = world(false);
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "tile.0").value, "FPS");
        let ops = press(by_id(&r, "tile.0"), Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!() };
        assert!(!spec.options.iter().any(|p| p.value == "dolbyAtmos"), "only where the TV's sound mode is Dolby Atmos");
        let written = (spec.write)("lowLatency");
        let Op::LgMany { settings, .. } = &written[0] else { panic!() };
        assert_eq!(settings["gameDashboardStatusList"], json!(["lowLatency", "vrr_aiGameSound_whiteStabilizer", "blackStabilizer", "fps"]));
        let written = (spec.write)("hdr");
        let Op::LgMany { settings, .. } = &written[0] else { panic!() };
        assert_eq!(settings["gameDashboardStatusList"][0], "hdr");
    }

    #[test]
    fn the_page_reads_each_document_with_its_dimension() {
        let w = world(false);
        let r = reads(&w.ctx());
        let find = |name: &str| r.iter().find_map(|x| if let Read::Call { name: n, payload, .. } = x { (*n == name).then(|| payload.clone()) } else { None }).unwrap();
        assert_eq!(find("go.in")["dimension"], json!({"gameInput": "hdmi2"}));
        assert_eq!(find("go.snd")["dimension"], json!({"audioInput": "hdmi2_game"}));
        assert_eq!(find("go.pic.dv")["dimension"], json!({"input": "hdmi2", "pictureMode": "dolbyHdrGame"}));
        assert_eq!(find("go.tm")["app_id"], "com.webos.app.hdmi2");
        assert!(r.iter().any(|x| matches!(x, Read::Lg { category: "other", keys, .. } if keys.contains(&"gameMode"))));
    }
}
