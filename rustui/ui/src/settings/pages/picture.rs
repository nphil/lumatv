//! Picture (`settings/PagePicture.qml`, `docs/mocks/01-settings-picture`, `docs/research/settings-coverage.md`): LG's Picture menu, written
//! to the TV's own settings service (so LG's picture processing, panel care and stored values are unchanged; only the screen is new). The
//! keys, categories and value lists are LG's own: a short list is the TV's own list for the key, in LG's names, and a row the TV describes
//! as not visible (HDR-only rows while an SDR picture plays, the ones this set never shows) is left out as LG leaves it out. A row the
//! service has no key for is left out, one it has not answered is dimmed; the aspect ratio and TruMotion are per-input settings (the
//! service answers them only for an app), so they are offered while an input is in front, and the head's chip names that input. The rail
//! shows the mode in force. Fine Tune, White Balance (Colour tab) and HDMI Signalling (Expert tab) are pages of their own.

use crate::host::sio::SERVICE;
use crate::settings::ctx::{Ctx, Spec, ask};
use crate::settings::names;
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, PageArg, Row, ShellRequest};
use serde_json::{Map, Value, json};
use std::rc::Rc;
use std::sync::LazyLock;

pub fn page() -> Page {
    Page::new("picture", "Picture", "image").rail().tabs(&["Mode", "Brightness", "Colour", "Clarity", "Motion", "Expert"]).reads(reads).rows(rows).summary(summary).chip(chip)
}

const MODES: [&str; 9] = ["filmMaker", "cinema", "normal", "vivid", "game", "sports", "eco", "expert1", "expert2"];
const MODE_NAMES: names::Table = &[
    ("filmMaker", "Filmmaker"),
    ("cinema", "Cinema"),
    ("normal", "Standard"),
    ("vivid", "Vivid"),
    ("game", "Game"),
    ("sports", "Sports"),
    ("eco", "Eco"),
    ("expert1", "Expert \u{b7} day"),
    ("expert2", "Expert \u{b7} night"),
];

fn summary(c: &Ctx) -> String {
    c.lg.opt_text("picture.pictureMode").map(|m| names::name_or(MODE_NAMES, &m, "")).unwrap_or_default()
}

fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    let port = c.input_port();
    if c.input_app().is_some() && !port.is_empty() { Some((port, "info")) } else { None }
}

fn reads(c: &Ctx) -> Vec<Read> {
    let mut r = vec![
        Read::lg("picture", &["pictureMode", "backlight", "energySaving", "eyeComfortMode", "filmmakerAmbientLight", "contrast", "brightness", "color", "tint", "sharpness"]),
        Read::lg("picture", &["dynamicContrast", "peakBrightness", "gamma", "nearBlackDetail", "motionEyeCare", "colorTemperature", "colorGamut", "dynamicColor", "blackLevel", "energySavingAutoMin"]),
        Read::lg("picture", &["superResolution", "noiseReduction", "mpegNoiseReduction", "smoothGradation", "realCinema", "dolbyPrecisionDetail"]),
        Read::lg("picture", &["truMotionJudder", "truMotionBlur", "motionPro", "filmMakerMode", "hdrDynamicToneMapping", "expressionEnhancer", "applyToAllInput"]),
        Read::lg("aiPicture", &["ai_Picture", "ai_Genre", "ai_Brightness"]),
        Read::lg("other", &["colorimetry"]),
    ];
    if let Some(app) = c.input_app() {
        r.push(Read::lg_app("aspectRatio", &["arcPerApp", "justScan"], &app));
        r.push(Read::lg_app("picture", &["truMotionMode"], &app));
    }
    r
}

/// A per-input row: offered while an input is in front, else it says why not; the read with that app's id answers it.
fn per_app(c: &Ctx, mut r: Row, ds: &str) -> Row {
    let app = c.input_app();
    r.enabled = app.is_some() && c.lg.usable(ds);
    if app.is_none() {
        r.detail = "No input open".into();
    }
    r
}

fn rows(c: &Ctx) -> Vec<Row> {
    let app = c.input_app();
    let user_motion = c.lg.is("picture.truMotionMode", "user");
    let mut rows = Vec::new();
    // ---- Mode
    let cards = [
        ("Filmmaker", "clapperboard"),
        ("Cinema", "film"),
        ("Standard", "tv"),
        ("Vivid", "sun"),
        ("Game", "gamepad-2"),
        ("Sports", "activity"),
        ("Eco", "leaf"),
        ("Expert \u{b7} day", "sun-medium"),
        ("Expert \u{b7} night", "moon-star"),
    ];
    rows.push(c.cards(Spec::new("picture.pictureMode").tab(0).strip(4).values(&MODES).cards(&cards)));
    rows.push(c.slider(Spec::new("picture.backlight").tab(0).icon("sun").label("OLED pixel brightness").range(0.0, 100.0, 1.0)));
    rows.push(c.toggle(Spec::new("picture.filmmakerAmbientLight").tab(0).icon("lamp").label("Filmmaker ambient light").help("Lifts dark scenes a little when the room is bright.")));
    rows.push(c.toggle(Spec::new("aiPicture.ai_Picture").tab(0).icon("sparkles").label("AI Picture Pro").help("LG's sharpening and upscaling. Off keeps the picture as the film was made.")));
    rows.push(per_app(
        c,
        c.seg(Spec::new("aspectRatio.arcPerApp").tab(0).icon("rectangle-horizontal").label("Aspect ratio").app(app.clone()).help("Original keeps the film's own shape. It is set for the input in front.").options(&["16:9", "Original", "4:3"]).values(&["16x9", "original", "4x3"])),
        "aspectRatio.arcPerApp",
    ));
    rows.push(c.seg(Spec::new("picture.energySaving").tab(0).icon("leaf").label("Energy saving").options(&["Auto", "Off", "Minimum", "Medium", "Maximum"]).values(&["auto", "off", "min", "med", "max"])));
    let auto = c.lg.is("picture.energySaving", "auto");
    let mut min = c.range(Spec::new("picture.energySavingAutoMin").tab(0).icon("leaf").label("Minimum Brightness").range(0.0, 10.0, 1.0).help("How dark Auto energy saving may make the picture."));
    min.enabled = auto && c.lg.usable("picture.energySavingAutoMin");
    min.detail = if !c.lg.has("picture.energySavingAutoMin") { "TV not answering".into() } else if auto { String::new() } else { "Energy saving is not Auto".into() };
    rows.push(min);
    // Just Scan is per input like the aspect ratio: LG reads and writes it with the app's id, and the TV answers nothing to a read without one
    rows.push(per_app(c, c.toggle(Spec::new("aspectRatio.justScan").tab(0).icon("monitor").label("Just Scan").app(app.clone()).help("Shows the picture whole, without cropping its edges. Turn it off if a line shows at an edge.")), "aspectRatio.justScan"));

    // ---- Brightness
    rows.push(c.slider(Spec::new("picture.contrast").tab(1).icon("contrast").label("Contrast").range(0.0, 100.0, 1.0)));
    rows.push(c.slider(Spec::new("picture.brightness").tab(1).icon("sun-dim").label("Brightness").range(0.0, 100.0, 1.0).help("Lifts or deepens the dark parts. The panel's own light is OLED pixel brightness.")));
    rows.push(c.toggle(Spec::new("picture.eyeComfortMode").tab(1).icon("eye").label("Eye comfort").help("A warmer picture for the evening.")));
    rows.push(c.seg_tv(Spec::new("picture.dynamicContrast").tab(1).icon("contrast").label("Auto Dynamic Contrast").names(names::LMHA).help("Adjusts the contrast to how bright the picture is.")));
    rows.push(c.seg_tv(Spec::new("picture.peakBrightness").tab(1).icon("sun").label("Peak Brightness").names(names::LMHA).help("How bright the brightest parts of the picture may get.")));
    rows.push(c.seg_tv(Spec::new("picture.gamma").tab(1).icon("gauge").label("Gamma").names(names::GAMMA).help("The lower the number, the brighter the middle tones.")));
    rows.push(c.slider(Spec::new("picture.nearBlackDetail").tab(1).icon("moon").label("Near Black Detail").range(-3.0, 3.0, 1.0).lg_shown()));
    rows.push(c.seg_tv(Spec::new("picture.blackLevel").tab(1).icon("contrast").label("Video Range").names(names::BLACK_LEVEL).help("Limited for video from a player, Full for a PC or console. Auto lets the signal decide.")));
    rows.push(c.seg_tv(Spec::new("picture.motionEyeCare").tab(1).icon("eye").label("Motion Eye Care").names(names::LMHA).help("Dims the picture a little in fast scenes to ease glare.")));

    // ---- Colour
    rows.push(c.slider(Spec::new("picture.color").tab(2).icon("palette").label("Colour").range(0.0, 100.0, 1.0)));
    rows.push(c.slider(Spec::new("picture.tint").tab(2).icon("blend").label("Tint").range(-50.0, 50.0, 1.0).help("Shifts skin tones between green and red.")));
    rows.push(c.slider(Spec::new("picture.colorTemperature").tab(2).icon("sun-medium").label("Color Temperature").range(-50.0, 50.0, 1.0).help("Below zero is cooler (bluer), above zero is warmer (redder).")));
    rows.push(c.seg_tv(Spec::new("picture.colorGamut").tab(2).icon("palette").label("Color Gamut").names(names::COLOR_GAMUT)));
    rows.push(c.seg_tv(Spec::new("picture.dynamicColor").tab(2).icon("sliders-horizontal").label("Color Adjustment").names(names::LMHA)));
    rows.push(Row::link("finetune").tab(2).icon("sliders-horizontal").label("Fine Tune").help("Saturation, tint and luminance of each of the six colours.").does(go("finetune")));
    rows.push(Row::link("whitebalance").tab(2).icon("sun-medium").label("White Balance").help("The grey scale's red, green and blue, at 2, 10 or 22 points.").does(go("whitebalance")));

    // ---- Clarity
    rows.push(c.slider(Spec::new("picture.sharpness").tab(3).icon("aperture").label("Sharpness").range(0.0, 50.0, 1.0).help("Filmmaker keeps the picture as made. More adds edges.")));
    rows.push(c.seg_tv(Spec::new("picture.superResolution").tab(3).icon("image").label("Super Resolution").names(names::LMHA).help("Sharpens a soft picture, such as a low-resolution broadcast.")));
    rows.push(c.seg_tv(Spec::new("picture.noiseReduction").tab(3).icon("zap").label("Noise Reduction").names(names::LMHA)));
    rows.push(c.seg_tv(Spec::new("picture.mpegNoiseReduction").tab(3).icon("cpu").label("MPEG Noise Reduction").names(names::LMHA).help("Cleans up blocks in compressed video.")));
    rows.push(c.seg_tv(Spec::new("picture.smoothGradation").tab(3).icon("blend").label("Smooth Gradation").names(names::LMHA).help("Smooths the bands in a sky or a dark gradient.")));
    rows.push(c.toggle(Spec::new("picture.realCinema").tab(3).icon("film").label("Real Cinema").help("Plays film as 24 frames a second, the way it was shot.")));
    rows.push(c.toggle(Spec::new("picture.dolbyPrecisionDetail").tab(3).icon("image").label("Precision Detail").help("More detail in Dolby Vision pictures.").lg_shown()));

    // ---- Motion
    rows.push(per_app(c, c.choice(Spec::new("picture.truMotionMode").tab(4).icon("fast-forward").label("TruMotion").names(names::TRU_MOTION).app(app.clone()).help("Smooths motion. Off keeps the film's own look.")), "picture.truMotionMode"));
    let mut judder = c.slider(Spec::new("picture.truMotionJudder").tab(4).icon("timer").label("De-Judder").range(0.0, 10.0, 1.0).help("Smooths the stutter of film on a 60 Hz screen."));
    judder.visible = user_motion && !c.lg.missing("picture.truMotionJudder");
    rows.push(judder);
    let mut blur = c.slider(Spec::new("picture.truMotionBlur").tab(4).icon("activity").label("De-Blur").range(0.0, 10.0, 1.0).help("Sharpens moving objects."));
    blur.visible = user_motion && !c.lg.missing("picture.truMotionBlur");
    rows.push(blur);
    rows.push(c.toggle(Spec::new("picture.motionPro").tab(4).icon("fast-forward").label("OLED Motion Pro").help("Reduces blur in fast motion by dimming the screen a little.")));

    // ---- Expert
    rows.push(c.toggle(Spec::new("picture.filmMakerMode").tab(5).icon("clapperboard").label("Filmmaker Auto Start").help("Switches to Filmmaker mode by itself when a film asks for it.")));
    rows.push(c.seg_tv(Spec::new("picture.hdrDynamicToneMapping").tab(5).icon("sun-medium").label("Dynamic Tone Mapping").names(names::TONE_MAPPING).help("Adjusts the contrast of HDR pictures scene by scene.")));
    rows.push(c.seg_tv(Spec::new("picture.expressionEnhancer").tab(5).icon("sparkles").label("Expression Enhancer").names(names::EXPRESSION)));
    rows.push(c.toggle(Spec::new("aiPicture.ai_Genre").tab(5).icon("wand-sparkles").label("AI genre selection").help("Picks a look for sport, film or news by what is playing.")));
    rows.push(c.toggle(Spec::new("aiPicture.ai_Brightness").tab(5).icon("sun-medium").label("AI brightness").help("Follows the room light and the scene.")));
    let mut signalling = Row::link("signalling").tab(5).icon("cable").label("HDMI Signalling").help("What the TV is told an HDMI picture is: colorimetry, EOTF and the HDR values. AUTO is the source's own.").does(go("signalling"));
    signalling.visible = c.lg.shown("other.colorimetry");
    rows.push(signalling);
    rows.push(Row::link("displaylab").tab(5).icon("scan-eye").label("Display lab").help("Compare two looks of the screen side by side on test cards, one step at a time.").does(vec![Op::Shell(ShellRequest::DisplayLab)]));
    let mut apply = Row::action("applyall", "Apply").tab(5).icon("list-checks").label("Apply to All Inputs").help("Gives every input this mode's settings.");
    apply.enabled = c.lg.usable("picture.applyToAllInput");
    rows.push(apply.does(ask("list-checks", "Apply to all inputs?", "Every input gets this mode's settings.", "Apply", "Cancel", false, vec![Op::Lg { ds: "picture.applyToAllInput".into(), value: json!("picture"), app: None }])));
    let mut reset = Row::action("reset", "Reset").tab(5).icon("rotate-ccw").label("Reset").danger();
    reset.enabled = c.lg.has("picture.pictureMode");
    let dim = c.lg.dimension("picture").cloned().unwrap_or(Value::Null);
    let mode = c.lg.opt_text("picture.pictureMode").unwrap_or_default();
    rows.push(reset.does(ask(
        "rotate-ccw",
        "Reset this picture mode?",
        "Its settings go back to LG's.",
        "Reset",
        "Cancel",
        true,
        vec![
            // what a picture mode's reset names: the input and the mode in force, as LG's own Reset sends them
            Op::Act { uri: format!("{SERVICE}/resetSystemSettings"), payload: json!({"category": "picture", "dimension": {"input": dim.get("input"), "pictureMode": mode}}) },
            Op::Refresh,
        ],
    )));
    rows
}

fn go(page: &str) -> Vec<Op> {
    vec![Op::Open { page: page.into(), arg: PageArg::None }]
}

/// The `pictureSettingModified` LG sends along with a white balance or Fine Tune change (its `wG`): the table of picture modes the owner has
/// changed, with the mode in force added. None when the mode is already marked, so nothing extra is written.
fn modified(c: &Ctx) -> Option<Value> {
    let mode = c.lg.opt_text("picture.pictureMode").unwrap_or_default();
    let cur = c.lg.get("picture.pictureSettingModified");
    if !c.lg.has("picture.pictureSettingModified") || mode.is_empty() || mode == "undefined" || cur.and_then(|v| v.get(&mode)).is_some_and(|v| !v.is_null() && v != &json!(false)) {
        return None;
    }
    let mut out = cur.and_then(Value::as_object).cloned().unwrap_or_default();
    out.insert(mode, json!(true));
    Some(Value::Object(out))
}

fn child(id: &'static str, title: &'static str, icon: &'static str) -> Page {
    Page::new(id, title, icon).parent("picture").back("Picture").hidden_from_search()
}

// ---------------------------------------------------------------------------------------------------------- Fine Tune

pub fn fine_tune() -> Page {
    child("finetune", "Fine Tune", "sliders-horizontal").reads(fine_reads).rows(fine_rows)
}

const COLOURS: [&str; 6] = ["Red", "Green", "Blue", "Cyan", "Magenta", "Yellow"];

/// The eighteen keys of Fine Tune: `colorManagement<Part><Colour>` for each part (Saturation, Hue, Luminance) and each of the six colours.
const COLOUR_TABLE: [[&str; 6]; 3] = [
    ["colorManagementSaturationRed", "colorManagementSaturationGreen", "colorManagementSaturationBlue", "colorManagementSaturationCyan", "colorManagementSaturationMagenta", "colorManagementSaturationYellow"],
    ["colorManagementHueRed", "colorManagementHueGreen", "colorManagementHueBlue", "colorManagementHueCyan", "colorManagementHueMagenta", "colorManagementHueYellow"],
    ["colorManagementLuminanceRed", "colorManagementLuminanceGreen", "colorManagementLuminanceBlue", "colorManagementLuminanceCyan", "colorManagementLuminanceMagenta", "colorManagementLuminanceYellow"],
];

static FINE_KEYS: LazyLock<Vec<&'static str>> = LazyLock::new(|| COLOUR_TABLE.iter().flatten().copied().collect());

fn fine_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::lg("picture", &["pictureMode", "pictureSettingModified", "keyListForCopyCms", "dynamicColor", "colorManagementColorSystem"]), Read::lg("picture", &FINE_KEYS)]
}

fn colour_key(part: &str, colour: &str) -> &'static str {
    let p = match part {
        "Saturation" => 0,
        "Hue" => 1,
        _ => 2,
    };
    COLOUR_TABLE[p][COLOURS.iter().position(|c| *c == colour).unwrap_or(0)]
}

fn reset_keys(c: &Ctx, ds: &str) -> Vec<Value> {
    c.lg.get(ds).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// A reset LG's own way: the keys the TV itself lists, sent to `resetSystemSettings`, then read again.
fn reset_row(c: &Ctx, list_ds: &str, title: &str, body: &str, help: &str) -> Row {
    let keys = reset_keys(c, list_ds);
    let mut r = Row::action("reset", "Reset").tab(0).icon("rotate-ccw").label("Reset").help(help).danger();
    r.enabled = !keys.is_empty();
    r.detail = if c.lg.has(list_ds) { String::new() } else { "TV not answering".into() };
    r.does(ask("rotate-ccw", title, body, "Reset", "Cancel", true, vec![Op::Act { uri: format!("{SERVICE}/resetSystemSettings"), payload: json!({"category": "picture", "keys": keys}) }, Op::Refresh]))
}

fn fine_rows(c: &Ctx) -> Vec<Row> {
    let user = c.lg.is("picture.dynamicColor", "user");
    let colour = c.lg.opt_text("picture.colorManagementColorSystem").unwrap_or_default();
    let suffix: String = colour.chars().next().map(|f| f.to_uppercase().collect::<String>() + &colour[f.len_utf8()..]).unwrap_or_default();
    let mut rows = Vec::new();
    let mut select = c.choice(Spec::new("picture.colorManagementColorSystem").tab(0).icon("palette").label("Select Color").names(names::COLOR_SYSTEM).help("The colour the three rows below adjust. They work while Color Adjustment is User."));
    select.enabled = user && c.lg.usable("picture.colorManagementColorSystem");
    select.detail = if !c.lg.has("picture.colorManagementColorSystem") { "TV not answering".into() } else if user { String::new() } else { "Set Color Adjustment to User".into() };
    rows.push(select);
    for (part, label, icon, help) in [
        ("Saturation", "Adjust Saturation", "blend", "Makes the chosen colour clearer or duller."),
        ("Hue", "Adjust Tint", "sliders-horizontal", "Turns the chosen colour towards its neighbour on the colour wheel."),
        ("Luminance", "Adjust Luminance", "sun", "Makes the chosen colour lighter or darker."),
    ] {
        let key = if suffix.is_empty() { colour_key(part, "Red") } else { colour_key(part, &suffix) };
        let ds = format!("picture.{key}");
        let mut r = c.range(Spec::new(&ds).id(part).tab(0).icon(icon).label(label).help(help).range(-30.0, 30.0, 1.0));
        r.visible = !suffix.is_empty() && !c.lg.missing(&ds) && c.lg.shown(&ds);
        r.enabled = user && c.lg.usable(&ds);
        r.detail = if !c.lg.has(&ds) { "TV not answering".into() } else if user { String::new() } else { "Set Color Adjustment to User".into() };
        let (k, mark) = (key.to_string(), modified(c));
        r.act = Some(Rc::new(move |a| match a {
            Arg::Value(v) => {
                let mut w = Map::new();
                w.insert(k.clone(), json!((v.round() as i64).to_string()));
                if let Some(m) = &mark {
                    w.insert("pictureSettingModified".into(), m.clone());
                }
                vec![Op::LgMany { category: "picture".into(), settings: w, app: None }]
            }
            _ => Vec::new(),
        }));
        rows.push(r);
    }
    rows.push(reset_row(c, "picture.keyListForCopyCms", "Reset Fine Tune?", "The six colours go back to LG's.", "Puts the six colours' saturation, tint and luminance back to LG's."));
    rows
}

// ---------------------------------------------------------------------------------------------------------- White Balance

pub fn white_balance() -> Page {
    child("whitebalance", "White balance", "sun-medium").reads(wb_reads).rows(wb_rows)
}

fn wb_reads(_: &Ctx) -> Vec<Read> {
    vec![
        Read::lg("picture", &["pictureMode", "pictureSettingModified", "keyListForCopyWb", "whiteBalanceMethod", "whiteBalancePoint", "whiteBalanceIre", "whiteBalanceIre10pt", "whiteBalanceLuminance"]),
        Read::lg("picture", &["whiteBalanceRed", "whiteBalanceGreen", "whiteBalanceBlue", "adjustingLuminance", "whiteBalanceRed10pt", "whiteBalanceGreen10pt", "whiteBalanceBlue10pt", "adjustingLuminance10pt"]),
        Read::lg("picture", &["whiteBalanceRedGain", "whiteBalanceGreenGain", "whiteBalanceBlueGain", "whiteBalanceRedOffset", "whiteBalanceGreenOffset", "whiteBalanceBlueOffset"]),
    ]
}

/// The state of the grey scale the page adjusts: the method, the point, the signal level in force and the levels the TV lists.
struct Wb {
    method: String,
    two: bool,
    high: bool,
    ire_key: &'static str,
    levels: Vec<String>,
    level: Option<usize>,
}

fn wb(c: &Ctx) -> Wb {
    let method = c.lg.opt_text("picture.whiteBalanceMethod").unwrap_or_default();
    let ire_key = if method == "22" { "whiteBalanceIre" } else { "whiteBalanceIre10pt" };
    let levels: Vec<String> = c.lg.list(&format!("picture.{ire_key}")).unwrap_or_default().into_iter().filter(|l| l.visible).map(|l| l.value).collect();
    let current = c.lg.opt_text(&format!("picture.{ire_key}")).unwrap_or_default();
    let level = levels.iter().position(|l| *l == current);
    Wb { two: method == "2", high: c.lg.is("picture.whiteBalancePoint", "high"), method, ire_key, levels, level }
}

impl Wb {
    /// The key that holds a colour's number now (and whether it is a list).
    fn colour_key(&self, name: &str) -> String {
        if self.two {
            format!("whiteBalance{name}{}", if self.high { "Gain" } else { "Offset" })
        } else if self.method == "10" {
            format!("whiteBalance{name}10pt")
        } else {
            format!("whiteBalance{name}")
        }
    }

    fn lum_key(&self) -> &'static str {
        if self.method == "10" { "adjustingLuminance10pt" } else { "adjustingLuminance" }
    }

    /// The number of a key: an element of the list at the level in force, or the number itself.
    fn number(&self, c: &Ctx, key: &str) -> f64 {
        let v = c.lg.get(&format!("picture.{key}"));
        let n = match v {
            Some(Value::Array(a)) => a.get(self.level.unwrap_or(0)).and_then(crate::host::sio::js::float),
            Some(other) => crate::host::sio::js::float(other),
            None => None,
        };
        n.unwrap_or(0.0)
    }
}

fn wb_slider(c: &Ctx, w: &Wb, id: &str, label: &str, icon: &str, key: String, help: &str) -> Row {
    let ds = format!("picture.{key}");
    let lim = c.lg.range(&ds);
    let (min, max) = lim.map_or((-50.0, 50.0), |l| (l.min, l.max));
    let mut r = Row::slider(id, min, max, 1.0).tab(0).icon(icon).label(label).help(help);
    r.visible = !c.lg.missing(&ds) && c.lg.shown(&ds);
    r.enabled = c.lg.usable(&ds);
    r.detail = if c.lg.has(&ds) { String::new() } else { "TV not answering".into() };
    r.num = w.number(c, &key);
    r.value_text = Some(if c.lg.has(&ds) { (r.num.round() as i64).to_string() } else { "\u{2013}".into() });
    let (level, mark, list, k) = (w.level, modified(c), c.lg.get(&ds).cloned(), key);
    r.act = Some(Rc::new(move |a| match a {
        Arg::Value(v) => {
            let n = v.round() as i64;
            let mut wr = Map::new();
            match &list {
                Some(Value::Array(arr)) => {
                    let mut arr = arr.clone();
                    let at = level.unwrap_or(0);
                    if at < arr.len() {
                        arr[at] = json!(n);
                    }
                    wr.insert(k.clone(), Value::Array(arr));
                }
                _ => {
                    wr.insert(k.clone(), json!(n.to_string()));
                }
            }
            if let Some(m) = &mark {
                wr.insert("pictureSettingModified".into(), m.clone());
            }
            vec![Op::LgMany { category: "picture".into(), settings: wr, app: None }]
        }
        _ => Vec::new(),
    }));
    r
}

fn wb_rows(c: &Ctx) -> Vec<Row> {
    let w = wb(c);
    let mut rows = vec![c.choice(Spec::new("picture.whiteBalanceMethod").tab(0).icon("sliders-horizontal").label("Method").names(names::WB_METHOD).help("How many points of the grey scale are adjusted. More points, finer control."))];
    let mut point = c.seg_tv(Spec::new("picture.whiteBalancePoint").tab(0).icon("contrast").label("Point").names(names::WB_POINT).help("Low adjusts the dark end of the grey scale, High the bright end."));
    point.visible = w.two && c.lg.shown("picture.whiteBalancePoint");
    rows.push(point);
    let ire = format!("picture.{}", w.ire_key);
    let mut level = Row::nudge("level").tab(0).icon("gauge").label("Signal Level(%)").help("The brightness of the grey the rows below adjust.");
    level.visible = !w.two && !w.levels.is_empty();
    level.enabled = c.lg.usable(&ire);
    level.value = w.level.map(|i| format!("{}%", w.levels[i])).unwrap_or_default();
    let (levels, at, ire_key) = (w.levels.clone(), w.level, w.ire_key);
    rows.push(level.act(move |a| match a {
        Arg::Nudge(d) => {
            if levels.is_empty() {
                return Vec::new();
            }
            let from = at.unwrap_or(levels.len() - 1) as i64;
            let i = (from + i64::from(d)).clamp(0, levels.len() as i64 - 1) as usize;
            let mut m = Map::new();
            m.insert(ire_key.into(), json!(levels[i]));
            vec![Op::LgMany { category: "picture".into(), settings: m, app: None }]
        }
        _ => Vec::new(),
    }));
    let mut lum = wb_slider(c, &w, "lum", "Luminance", "sun", w.lum_key().to_string(), "Makes the grey at this signal level lighter or darker.");
    lum.visible = !w.two && !c.lg.missing(&format!("picture.{}", w.lum_key()));
    rows.push(lum);
    rows.push(wb_slider(c, &w, "red", "Red", "palette", w.colour_key("Red"), "Less or more red in the grey at this level."));
    rows.push(wb_slider(c, &w, "green", "Green", "palette", w.colour_key("Green"), "Less or more green in the grey at this level."));
    rows.push(wb_slider(c, &w, "blue", "Blue", "palette", w.colour_key("Blue"), "Less or more blue in the grey at this level."));
    let at_100 = w.level.is_some_and(|i| w.levels[i] == "100");
    let mut bright = c.range(Spec::new("picture.whiteBalanceLuminance").tab(0).icon("sun").label("Brightness at 100%").range(50.0, 500.0, 1.0).help("How bright the white at the top of the scale is. It answers only at the 100% signal level."));
    bright.visible = !w.two && c.lg.shown("picture.whiteBalanceLuminance");
    bright.enabled = c.lg.usable("picture.whiteBalanceLuminance") && !w.levels.is_empty() && at_100;
    bright.detail = if !c.lg.has("picture.whiteBalanceLuminance") { "TV not answering".into() } else if at_100 { String::new() } else { "Only at 100%".into() };
    let (mark, list) = (modified(c), c.lg.get("picture.whiteBalanceLuminance").cloned());
    bright.act = Some(Rc::new(move |a| match a {
        Arg::Value(v) => {
            let mut m = Map::new();
            m.insert("whiteBalanceLuminance".into(), match &list {
                Some(Value::Array(arr)) => Value::Array(arr.clone()),
                _ => json!((v.round() as i64).to_string()),
            });
            if let Some(mk) = &mark {
                m.insert("pictureSettingModified".into(), mk.clone());
            }
            vec![Op::LgMany { category: "picture".into(), settings: m, app: None }]
        }
        _ => Vec::new(),
    }));
    rows.push(bright);
    rows.push(reset_row(c, "picture.keyListForCopyWb", "Reset the white balance?", "Its settings go back to LG's.", "Puts the white balance back to LG's."));
    rows
}

// ---------------------------------------------------------------------------------------------------------- HDMI Signalling

pub fn signalling() -> Page {
    child("signalling", "HDMI Signalling", "cable").reads(signal_reads).rows(signal_rows).chip(chip)
}

const SIGNAL_KEYS: [&str; 7] = ["colorimetry", "eotf", "masteringPeak", "masteringColor", "maxCLL", "maxFALL", "dolbyVSVDBVer"];

fn signal_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::lg("other", &SIGNAL_KEYS)]
}

const AUTO_ONLY: names::Table = &[("auto", "AUTO")];

fn signal_rows(c: &Ctx) -> Vec<Row> {
    let choose = |key: &str, label: &str, icon: &str, table: names::Table, help: &str| c.choice(Spec::new(&format!("other.{key}")).tab(0).icon(icon).label(label).names(table).help(help));
    let mut rows = vec![
        choose("colorimetry", "Colorimetry", "palette", &[("auto", "AUTO"), ("bt709", "BT709"), ("bt601", "BT601"), ("hdrGamma", "HDR Gamma"), ("bt2020", "BT2020"), ("p3D65", "P3 D65")], "The colour space the picture is read in."),
        choose("eotf", "EOTF", "gauge", &[("auto", "AUTO"), ("sdrGamma", "SDR Gamma"), ("hdrGamma", "HDR Gamma"), ("st2084", "ST2084"), ("hlg", "HLG")], "The brightness curve the picture is read with."),
        choose("masteringPeak", "Mastering Peak", "sun", AUTO_ONLY, "The peak brightness, in nits, the picture was mastered at."),
        choose("masteringColor", "Mastering Color", "palette", &[("auto", "AUTO"), ("p3D65", "P3 D65"), ("bt2020D65", "BT2020 D65"), ("bt709D65", "BT709 D65")], "The colour space the picture was mastered in."),
        choose("maxCLL", "MaxCLL", "sun-dim", AUTO_ONLY, "The brightest single point of the picture, in nits."),
        choose("maxFALL", "MaxFALL", "sun-dim", AUTO_ONLY, "The brightest average frame of the picture, in nits."),
        choose("dolbyVSVDBVer", "Dolby VSVDB", "film", &[("v1", "V1"), ("v2", "V2")], "The version of the Dolby Vision capability the TV reports to the source."),
    ];
    rows.push(Row::action("reset", "Reset").tab(0).icon("rotate-ccw").label("Reset").help("Puts the seven values back to LG's.").danger().does(ask(
        "rotate-ccw",
        "Reset the HDMI signalling?",
        "The seven values go back to LG's.",
        "Reset",
        "Cancel",
        true,
        vec![Op::Act { uri: format!("{SERVICE}/resetSystemSettings"), payload: json!({"category": "other", "keys": SIGNAL_KEYS}) }, Op::Refresh],
    )));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};
    use crate::settings::row::Kind;

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

    fn picture_world() -> World {
        let mut w = World::new(lg_with(
            &[
                ("picture", json!({"settings": {"pictureMode": "cinema", "backlight": 40, "energySaving": "off", "contrast": 85, "brightness": 50, "gamma": "medium", "truMotionMode": "user", "truMotionJudder": 3, "dynamicColor": "user", "pictureSettingModified": {"vivid": true}, "applyToAllInput": "none"}, "dimension": {"input": "hdmi1", "pictureMode": "cinema"}})),
                ("aiPicture", json!({"settings": {"ai_Picture": "off"}})),
            ],
            &[("picture", json!({"results": [{"key": "gamma", "values": {"arrayExt": [{"value": "low"}, {"value": "medium"}, {"value": "high2", "active": false}]}}, {"key": "contrast", "values": {"range": {"min": 0, "max": 100}}}]}))],
        ));
        w.topics.set_channel("foreground", json!({"app": "com.webos.app.hdmi1", "home": false}));
        w
    }

    #[test]
    fn the_mode_cards_come_first_the_rail_says_the_mode_and_the_head_names_the_input() {
        let w = picture_world();
        let r = rows(&w.ctx());
        let modes = by_id(&r, "picture.pictureMode");
        assert_eq!((modes.kind, modes.cards.len(), modes.strip), (Kind::Cards, 9, true));
        assert_eq!(modes.index, Some(1), "cinema");
        assert_eq!(summary(&w.ctx()), "Cinema");
        assert_eq!(chip(&w.ctx()), Some(("HDMI 1".into(), "info")));
        let ops = (modes.act.as_ref().unwrap())(Arg::Index(0));
        assert!(matches!(&ops[..], [Op::Lg { ds, value, .. }] if ds == "picture.pictureMode" && value == "filmMaker"));
        assert_eq!(by_id(&r, "picture.backlight").num, 40.0);
        assert_eq!(by_id(&r, "picture.gamma").options, vec!["1.9", "2.2", "BT.1886"]);
    }

    #[test]
    fn a_per_input_row_waits_for_an_input_and_is_read_with_its_app() {
        let mut w = picture_world();
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "aspectRatio.arcPerApp").detail, "TV not answering", "asked, not answered yet");
        let reads = reads(&w.ctx());
        assert!(reads.iter().any(|x| matches!(x, Read::Lg { category, app: Some(a), .. } if *category == "aspectRatio" && a == "com.webos.app.hdmi1")));
        w.topics.set_channel("foreground", json!({"app": "netflix"}));
        let r = rows(&w.ctx());
        assert!(!by_id(&r, "aspectRatio.arcPerApp").enabled && by_id(&r, "aspectRatio.arcPerApp").detail == "No input open");
        assert!(!reads_have_app(&self::reads(&w.ctx())));
        assert_eq!(chip(&w.ctx()), None);
    }

    fn reads_have_app(r: &[Read]) -> bool {
        r.iter().any(|x| matches!(x, Read::Lg { app: Some(_), .. }))
    }

    #[test]
    fn de_judder_and_de_blur_show_only_for_the_user_motion_mode_and_energy_saving_minimum_waits_for_auto() {
        let mut w = picture_world();
        let r = rows(&w.ctx());
        assert!(by_id(&r, "picture.truMotionJudder").visible, "the user motion mode shows the two sliders it has");
        w.lg.learn("picture", &json!({"settings": {"truMotionMode": "off"}}));
        let r2 = rows(&w.ctx());
        assert!(!by_id(&r2, "picture.truMotionJudder").visible && !by_id(&r2, "picture.truMotionBlur").visible);
        let min = by_id(&r, "picture.energySavingAutoMin");
        assert!(!min.enabled);
        assert_eq!(min.detail, "TV not answering");
    }

    #[test]
    fn a_reset_names_the_input_and_the_mode_in_force_after_a_question() {
        let w = picture_world();
        let r = rows(&w.ctx());
        let ops = (by_id(&r, "reset").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert!(a.answers[1].danger);
        assert!(matches!(&a.answers[1].then[0], Op::Act { uri, payload } if uri.ends_with("resetSystemSettings") && payload["dimension"]["input"] == "hdmi1" && payload["dimension"]["pictureMode"] == "cinema"));
        let apply = by_id(&r, "applyall");
        assert!(matches!(&(apply.act.as_ref().unwrap())(Arg::Press)[..], [Op::Ask(_)]));
    }

    #[test]
    fn fine_tune_adjusts_the_chosen_colour_only_in_user_mode_and_marks_the_picture_mode_changed() {
        let mut w = World::new(lg_with(
            &[("picture", json!({"settings": {"pictureMode": "cinema", "dynamicColor": "user", "colorManagementColorSystem": "red", "colorManagementSaturationRed": 4, "colorManagementHueRed": 0, "colorManagementLuminanceRed": -2, "pictureSettingModified": {}, "keyListForCopyCms": ["a", "b"]}}))],
            &[],
        ));
        let r = fine_rows(&w.ctx());
        assert!(by_id(&r, "Saturation").enabled && by_id(&r, "Saturation").visible);
        assert_eq!(by_id(&r, "Saturation").num, 4.0);
        assert_eq!(by_id(&r, "Luminance").value_text.as_deref(), Some("-2"));
        let ops = (by_id(&r, "Saturation").act.as_ref().unwrap())(Arg::Value(7.0));
        let [Op::LgMany { category, settings, .. }] = &ops[..] else { panic!() };
        assert_eq!(category, "picture");
        assert_eq!(settings["colorManagementSaturationRed"], "7");
        assert_eq!(settings["pictureSettingModified"], json!({"cinema": true}), "the mode in force is marked");
        let ops = (by_id(&r, "reset").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert!(matches!(&a.answers[1].then[0], Op::Act { payload, .. } if payload["keys"] == json!(["a", "b"])));
        w.lg.learn("picture", &json!({"settings": {"dynamicColor": "off"}}));
        let r = fine_rows(&w.ctx());
        assert!(!by_id(&r, "Saturation").enabled && by_id(&r, "Saturation").detail == "Set Color Adjustment to User");
        assert!(!by_id(&r, "picture.colorManagementColorSystem").enabled);
        // no colour chosen: no slider
        w.lg.learn("picture", &json!({"settings": {"colorManagementColorSystem": ""}}));
        assert!(!by_id(&fine_rows(&w.ctx()), "Hue").visible);
    }

    #[test]
    fn white_balance_at_ten_points_changes_one_number_of_a_list_at_the_level_in_force() {
        let w = World::new(lg_with(
            &[(
                "picture",
                json!({"settings": {"pictureMode": "cinema", "whiteBalanceMethod": "10", "whiteBalanceIre10pt": "50", "whiteBalanceRed10pt": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9], "whiteBalanceGreen10pt": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], "whiteBalanceBlue10pt": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], "adjustingLuminance10pt": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], "keyListForCopyWb": ["x"], "pictureSettingModified": {"cinema": true}}}),
            )],
            &[("picture", json!({"results": [{"key": "whiteBalanceIre10pt", "values": {"arrayExt": [{"value": "10"}, {"value": "20"}, {"value": "50"}, {"value": "100"}, {"value": "5", "visible": false}]}}]}))],
        ));
        let r = wb_rows(&w.ctx());
        assert!(by_id(&r, "level").visible && by_id(&r, "level").value == "50%");
        assert!(by_id(&r, "lum").visible);
        assert_eq!(by_id(&r, "red").num, 2.0, "the number at the level in force: the third of the list");
        assert!(!by_id(&r, "picture.whiteBalancePoint").visible, "the point is for two points");
        let ops = (by_id(&r, "red").act.as_ref().unwrap())(Arg::Value(9.0));
        let [Op::LgMany { settings, .. }] = &ops[..] else { panic!() };
        assert_eq!(settings["whiteBalanceRed10pt"], json!([0, 1, 9, 3, 4, 5, 6, 7, 8, 9]));
        assert!(settings.get("pictureSettingModified").is_none(), "the mode is marked already");
        let ops = (by_id(&r, "level").act.as_ref().unwrap())(Arg::Nudge(1));
        assert!(matches!(&ops[..], [Op::LgMany { settings, .. }] if settings["whiteBalanceIre10pt"] == "100"));
        let ops = (by_id(&r, "level").act.as_ref().unwrap())(Arg::Nudge(-3));
        assert!(matches!(&ops[..], [Op::LgMany { settings, .. }] if settings["whiteBalanceIre10pt"] == "10"), "held to the first level");
        assert!(!by_id(&r, "picture.whiteBalanceLuminance").enabled || !c_shown(&w), "brightness answers only at 100 %");
    }

    fn c_shown(w: &World) -> bool {
        w.lg.shown("picture.whiteBalanceLuminance")
    }

    #[test]
    fn white_balance_at_two_points_uses_the_offset_or_the_gain_of_the_point() {
        let mut w = World::new(lg_with(&[("picture", json!({"settings": {"pictureMode": "cinema", "whiteBalanceMethod": "2", "whiteBalancePoint": "low", "whiteBalanceRedOffset": 3, "whiteBalanceRedGain": -4}}))], &[]));
        let r = wb_rows(&w.ctx());
        assert!(by_id(&r, "picture.whiteBalancePoint").visible && !by_id(&r, "level").visible && !by_id(&r, "lum").visible);
        assert_eq!(by_id(&r, "red").num, 3.0);
        w.lg.learn("picture", &json!({"settings": {"whiteBalancePoint": "high"}}));
        let r = wb_rows(&w.ctx());
        assert_eq!(by_id(&r, "red").num, -4.0);
        let ops = (by_id(&r, "red").act.as_ref().unwrap())(Arg::Value(1.0));
        assert!(matches!(&ops[..], [Op::LgMany { settings, .. }] if settings["whiteBalanceRedGain"] == "1"));
    }

    #[test]
    fn signalling_is_seven_choices_in_lgs_names_and_a_reset_of_the_seven_keys() {
        let w = World::new(lg_with(&[("other", json!({"settings": {"colorimetry": "auto", "eotf": "st2084", "dolbyVSVDBVer": "v2"}}))], &[]));
        let r = signal_rows(&w.ctx());
        assert_eq!(by_id(&r, "other.eotf").value, "ST2084");
        assert_eq!(by_id(&r, "other.colorimetry").value, "AUTO");
        assert_eq!(r.iter().filter(|x| x.kind == Kind::Value).count(), 7);
        let ops = (by_id(&r, "reset").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert!(matches!(&a.answers[1].then[0], Op::Act { payload, .. } if payload["category"] == "other" && payload["keys"].as_array().map(|k| k.len()) == Some(7)));
        let _ = PageArg::None;
    }
}
