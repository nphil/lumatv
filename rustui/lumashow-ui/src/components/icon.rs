//! Icons for the lab's screens, drawn once into a small image layer. Each icon is a handful of strokes and fills
//! on Lucent's 24-unit grid (one rounded-stroke family, 1.9-unit strokes with round caps), evaluated per pixel as
//! signed distances so the edges are anti-aliased at any size. Icons are content raster like text: drawn when a
//! layer is made and again only when the icon or its colour changes.

use super::group::Part;
use crate::context::Ui;
use crate::geometry::Frame;
use crate::scene::ImageLayer;
use compositor_protocol::{Props, Rgba};
use std::io;

/// The design grid: every coordinate below is in 1/24ths of the icon's side.
const GRID: f32 = 24.0;
/// Stroke width in grid units (Lucent: 1.75-2 units, matching Manrope Medium).
const STROKE: f32 = 1.9;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Play,
    Pause,
    Next,
    Previous,
    Back,
    Close,
    Alarm,
    Timer,
    Phone,
    Mic,
    MicOff,
    Bulb,
    Thermometer,
    Droplet,
    Lock,
    Room,
    Camera,
    Expand,
    Sun,
    Cloud,
    Rain,
    Storm,
    Snow,
    Fog,
    Check,
    /// The tick on a toggle's knob as the mock-ups draw it (`.sw i .ic`: the path `M5 12.5l4.5 4.5L19 7.5` in strokes of 2.6 units):
    /// the same check as [`Icon::Check`], heavier.
    Tick,
    ChevronRight,
    ChevronDown,
    ChevronUp,
    Plus,
    Minus,
    Warning,
    Info,
    Tray,
    Sliders,
    CloudOff,
    Hourglass,
    /// The shift key: an arrow up.
    Shift,
    /// The backspace key: a tag with a cross, pointing left.
    Backspace,
    /// The enter key: an arrow that goes down, then left.
    Enter,
    /// An open eye: show what is hidden.
    Eye,
    /// A struck-through eye: hide what is shown.
    EyeOff,
    /// A triangle with an exclamation mark: something is wrong.
    Alert,
    /// A house: the home destination.
    Home,
    /// Four rounded squares: the launcher.
    Apps,
    /// Three arcs over a dot: a Wi-Fi link.
    Wifi,
    /// The same, struck through: no link.
    WifiOff,
    /// A cloud with the sun above it.
    PartlyCloudy,
    /// A bell: a reminder or an alert.
    Bell,
    /// An empty ring: a thing not yet done, a reminder's check before it is ticked.
    Circle,
    /// A camera, struck through: its lens is covered.
    CameraOff,
    /// An artist's palette: appearance and themes.
    Palette,
    /// A speaker with two sound arcs: audio.
    Speaker,
    /// A clock face: the time.
    Clock,
    /// Three rows of tick and line: a list of tasks or checks.
    Checklist,
    /// An arrow down onto a tray line: a download or an update.
    Download,
    /// A beetle: the debug page.
    Bug,
    /// A plug with two prongs and a cord: a plug-in.
    Puzzle,
    /// A magnifier: search.
    Search,
    /// A circular arrow: try again.
    Refresh,
    /// A crescent: night.
    Moon,
    /// A bell struck through: do not disturb.
    BellOff,
    /// A ring open at the top with a stroke up through the gap: power, and the screen switched off.
    Power,
    /// A small sun, a ring and four short rays: the dim end of the brightness slider.
    SunSmall,
    /// A cooking pot with its lid: a running timer.
    Pot,
    /// A page of a calendar: an appointment.
    Calendar,
    /// A speech bubble: a message.
    Message,
    /// A box: something delivered.
    Package,
    /// A drawing pin: what stays at the top of a list.
    Pin,
    /// An arrow that turns back: take the last thing back.
    Undo,
    /// A house with a plus: the home automation.
    HomeAssistant,
    /// A rounded square: stop what rings or plays.
    Stop,
    /// A still camera: a body with its viewfinder bump and a ring for the lens (the video camera is [`Icon::Camera`]).
    StillCamera,
    /// A padlock drawn as an outline, its body a frame and not a fill: the lock of a private notification.
    LockOutline,
    /// An alarm clock drawn as the mock-ups draw theirs: a ring with its hand, and a short stroke over each top corner for the bells,
    /// in strokes of 2 units (the night clock's next alarm; [`Icon::Alarm`] is the clock with its feet that the cards use).
    AlarmOutline,
    /// A ring that is open at its top left, three quarters of a turn: something under way. The shell turns nothing: a looking is a
    /// ring and a word.
    Spinner,
    /// A circular arrow with a square head: restart, check again (the mock-ups' `restart`; [`Icon::Refresh`] is the filled-head arrow).
    Restart,
    /// A ring with a stroke across it: what the Show has switched off and will not start again by itself (the mock-ups' `stop`;
    /// [`Icon::Stop`] is the rounded square that stops what rings).
    Block,
    /// A speedometer: an arc open at the bottom with its needle (the Performance page; the mock-ups' `gauge`).
    Gauge,
    /// A chip: a square with a row of pins on each side (the processor; the mock-ups' `cpu`).
    Cpu,
    /// A picture: a frame with a sun and a hill line (the mock-ups' `photo`).
    Photo,
    /// A cloud as an outline, the way the mock-ups draw `weather` in a list of effects ([`Icon::Cloud`] is the filled one).
    CloudOutline,
    /// A screen on its stand (the mock-ups' `cards`: raised buttons and cards).
    Cards,
    /// Three rounded squares in a corner: glass tiles (the mock-ups' `tiles`).
    Tiles,
    /// A shield: what keeps the Show safe (the mock-ups' `shield`).
    Shield,
    /// Two upright strokes: something held still ([`Icon::Pause`] has filled bars; the mock-ups' `pause` in a list of effects is two strokes).
    Still,
    /// A head and shoulders with an arc to either side of them: somebody is noticed (the Presence page; the mock-ups' `presence`).
    Presence,
    /// A head and shoulders: a person (the mock-ups' `person`).
    Person,
    /// Two stacked slabs, each with its light: a computer on the network (the mock-ups' `server`).
    Server,
    /// A small ring with eight short rays round it, the way the mock-ups draw `cog` (a setting of how something is reached).
    Cog,
    /// A video camera as the Presence mock-up draws its `camera`: a body and a wedge that is an outline, open to the body.
    Video,
    /// The same a little smaller, the mock-up's `sendcam` (a camera that sends).
    Sendcam,
    /// [`Icon::Sendcam`] struck through: the mock-up's `covered`, the camera behind its closed cover.
    SendcamOff,
    /// A triangle drawn as an outline, pointing right: play the sound (the mock-ups' `play`; [`Icon::Play`] is the filled one).
    PlayOutline,
}

/// Three arcs over a dot, the signal strength of a Wi-Fi link.
fn wifi() -> Vec<Prim> {
    vec![
        Prim::Disc((12.0, 18.8), 1.5),
        Prim::Arc((12.0, 18.8), 5.4, 225.0, 315.0),
        Prim::Arc((12.0, 18.8), 9.4, 225.0, 315.0),
        Prim::Arc((12.0, 18.8), 13.4, 225.0, 315.0),
    ]
}

/// The outline of an eye (two arcs meeting at its corners) and its pupil.
fn eye() -> Vec<Prim> {
    vec![
        Prim::Arc((12.0, 21.4), 13.0, 226.2, 313.8),
        Prim::Arc((12.0, 2.6), 13.0, 46.2, 133.8),
        Prim::Disc((12.0, 12.0), 2.6),
    ]
}

/// A circular arrow: an open ring with a head on its clockwise end.
fn refresh() -> Vec<Prim> {
    let (centre, r, end) = ((12.0f32, 12.0f32), 8.0f32, 325.0f32);
    let (s, c) = end.to_radians().sin_cos();
    let tip_at = (centre.0 + r * c, centre.1 + r * s);
    // The clockwise tangent at the end, and the outward radial direction there.
    let (t, n) = ((-s, c), (c, s));
    vec![
        Prim::Arc(centre, r, 40.0, end),
        Prim::Tri(
            (tip_at.0 + t.0 * 3.4, tip_at.1 + t.1 * 3.4),
            (tip_at.0 + n.0 * 3.4, tip_at.1 + n.1 * 3.4),
            (tip_at.0 - n.0 * 3.4, tip_at.1 - n.1 * 3.4),
            0.5,
        ),
    ]
}

/// A cubic curve from `p0` to `p3` (control points `p1`, `p2`) as ten short strokes: with round ends they join as one smooth line.
fn cubic(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)) -> Vec<Prim> {
    const STEPS: usize = 10;
    let at = |t: f32| {
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        (
            a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
            a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
        )
    };
    (0..STEPS)
        .map(|i| {
            Prim::Line(
                at(i as f32 / STEPS as f32),
                at((i + 1) as f32 / STEPS as f32),
            )
        })
        .collect()
}

/// The mock-ups' `person`: `circle 12 8 r3.6` and the shoulders `M5 20c.8-4 3.6-6 7-6s6.2 2 7 6`.
fn person() -> Vec<Prim> {
    let mut prims = vec![Prim::Ring((12.0, 8.0), 3.6)];
    prims.extend(cubic((5.0, 20.0), (5.8, 16.0), (8.6, 14.0), (12.0, 14.0)));
    prims.extend(cubic(
        (12.0, 14.0),
        (15.4, 14.0),
        (18.2, 16.0),
        (19.0, 20.0),
    ));
    prims
}

/// The Presence mock-up's `sendcam`: `rect 3 7 12 10 rx2.4` and the wedge `M15 10.5l5-2.8v8.6l-5-2.8`.
fn sendcam() -> Vec<Prim> {
    vec![
        Prim::Frame(3.0, 7.0, 12.0, 10.0, 2.4),
        Prim::Line((15.0, 10.5), (20.0, 7.7)),
        Prim::Line((20.0, 7.7), (20.0, 16.3)),
        Prim::Line((20.0, 16.3), (15.0, 13.5)),
    ]
}

type P = (f32, f32);

/// One stroke or fill of an icon, in grid units.
#[derive(Clone, Copy, Debug)]
enum Prim {
    /// A stroked segment.
    Line(P, P),
    /// A stroked circle.
    Ring(P, f32),
    /// A filled circle.
    Disc(P, f32),
    /// A stroked arc of a circle between two angles in degrees (0 = right, 90 = down, as on screen).
    Arc(P, f32, f32, f32),
    /// A filled triangle with its corners rounded by `r`.
    Tri(P, P, P, f32),
    /// A filled rounded rectangle: x, y, width, height, corner radius.
    Fill(f32, f32, f32, f32, f32),
    /// A stroked rounded rectangle.
    Frame(f32, f32, f32, f32, f32),
}

const CLOUD: [Prim; 4] = [
    Prim::Disc((8.0, 13.0), 4.0),
    Prim::Disc((12.5, 10.5), 5.0),
    Prim::Disc((17.0, 13.5), 3.6),
    Prim::Fill(8.0, 12.5, 9.5, 4.6, 2.3),
];

impl Icon {
    fn prims(self) -> Vec<Prim> {
        use Prim::*;
        let cloud = || CLOUD.to_vec();
        match self {
            Icon::Play => vec![Tri((7.5, 5.0), (19.0, 12.0), (7.5, 19.0), 1.6)],
            Icon::Pause => vec![
                Fill(6.5, 5.0, 4.0, 14.0, 1.6),
                Fill(13.5, 5.0, 4.0, 14.0, 1.6),
            ],
            Icon::Next => vec![
                Tri((5.5, 5.0), (15.5, 12.0), (5.5, 19.0), 1.4),
                Fill(16.5, 5.0, 2.6, 14.0, 1.2),
            ],
            Icon::Previous => vec![
                Tri((18.5, 5.0), (8.5, 12.0), (18.5, 19.0), 1.4),
                Fill(4.9, 5.0, 2.6, 14.0, 1.2),
            ],
            Icon::Back => vec![
                Line((15.0, 5.0), (8.0, 12.0)),
                Line((8.0, 12.0), (15.0, 19.0)),
            ],
            Icon::Close => vec![
                Line((6.0, 6.0), (18.0, 18.0)),
                Line((18.0, 6.0), (6.0, 18.0)),
            ],
            Icon::Alarm => vec![
                Ring((12.0, 13.0), 7.0),
                Line((12.0, 13.0), (12.0, 9.0)),
                Line((12.0, 13.0), (15.0, 14.5)),
                Disc((4.6, 5.4), 2.0),
                Disc((19.4, 5.4), 2.0),
                Line((6.6, 20.0), (5.2, 21.6)),
                Line((17.4, 20.0), (18.8, 21.6)),
            ],
            Icon::Timer => vec![
                Ring((12.0, 13.5), 7.5),
                Line((9.5, 3.0), (14.5, 3.0)),
                Line((12.0, 13.5), (15.2, 10.2)),
                Line((18.0, 6.0), (19.4, 4.6)),
            ],
            Icon::Phone => vec![
                Arc((17.0, 7.0), 11.0, 105.0, 165.0),
                Disc((14.2, 17.6), 2.6),
                Disc((6.4, 9.9), 2.6),
            ],
            Icon::Mic => vec![
                Frame(9.3, 3.0, 5.4, 11.0, 2.7),
                Arc((12.0, 11.0), 6.6, 20.0, 160.0),
                Line((12.0, 17.6), (12.0, 21.0)),
            ],
            Icon::MicOff => vec![
                Frame(9.3, 3.0, 5.4, 11.0, 2.7),
                Arc((12.0, 11.0), 6.6, 20.0, 160.0),
                Line((12.0, 17.6), (12.0, 21.0)),
                Line((4.5, 4.5), (19.5, 19.5)),
            ],
            Icon::Bulb => vec![
                Ring((12.0, 9.5), 5.6),
                Line((9.6, 16.5), (14.4, 16.5)),
                Line((10.2, 19.6), (13.8, 19.6)),
                Line((9.6, 14.4), (9.6, 16.5)),
                Line((14.4, 14.4), (14.4, 16.5)),
            ],
            Icon::Thermometer => vec![
                Frame(9.6, 2.8, 4.8, 13.4, 2.4),
                Disc((12.0, 17.6), 3.5),
                Line((12.0, 9.0), (12.0, 16.0)),
            ],
            Icon::Droplet => vec![
                Disc((12.0, 15.0), 5.6),
                Tri((12.0, 3.2), (6.6, 12.8), (17.4, 12.8), 0.8),
            ],
            Icon::Lock => vec![
                Fill(5.5, 11.0, 13.0, 9.5, 2.2),
                Arc((12.0, 11.0), 4.2, 180.0, 360.0),
            ],
            Icon::Room => vec![
                Line((4.0, 11.5), (12.0, 4.5)),
                Line((12.0, 4.5), (20.0, 11.5)),
                Line((6.0, 10.0), (6.0, 19.5)),
                Line((18.0, 10.0), (18.0, 19.5)),
                Line((6.0, 19.5), (18.0, 19.5)),
                Line((10.0, 19.5), (10.0, 14.5)),
                Line((14.0, 19.5), (14.0, 14.5)),
                Line((10.0, 14.5), (14.0, 14.5)),
            ],
            Icon::Camera => vec![
                Frame(3.0, 6.5, 13.5, 11.0, 2.6),
                Tri((17.0, 11.0), (21.2, 8.4), (21.2, 15.6), 0.6),
            ],
            Icon::Expand => vec![
                Line((4.0, 9.0), (4.0, 4.0)),
                Line((4.0, 4.0), (9.0, 4.0)),
                Line((15.0, 4.0), (20.0, 4.0)),
                Line((20.0, 4.0), (20.0, 9.0)),
                Line((20.0, 15.0), (20.0, 20.0)),
                Line((20.0, 20.0), (15.0, 20.0)),
                Line((9.0, 20.0), (4.0, 20.0)),
                Line((4.0, 20.0), (4.0, 15.0)),
            ],
            Icon::Sun => {
                let mut prims = vec![Ring((12.0, 12.0), 4.2)];
                for i in 0..8 {
                    let a = (i as f32 * 45.0).to_radians();
                    let (s, c) = a.sin_cos();
                    prims.push(Line(
                        (12.0 + 7.2 * c, 12.0 + 7.2 * s),
                        (12.0 + 9.6 * c, 12.0 + 9.6 * s),
                    ));
                }
                prims
            }
            Icon::Cloud => cloud(),
            Icon::Rain => {
                let mut prims = shifted(&CLOUD, -2.5);
                prims.extend([
                    Line((8.0, 17.5), (6.8, 21.0)),
                    Line((12.5, 17.5), (11.3, 21.0)),
                    Line((17.0, 17.5), (15.8, 21.0)),
                ]);
                prims
            }
            Icon::Storm => {
                let mut prims = shifted(&CLOUD, -2.5);
                prims.extend([
                    Line((13.4, 15.6), (10.4, 19.6)),
                    Line((10.4, 19.6), (14.2, 19.6)),
                    Line((14.2, 19.6), (11.6, 22.0)),
                ]);
                prims
            }
            Icon::Snow => {
                let mut prims = shifted(&CLOUD, -2.5);
                prims.extend([
                    Disc((8.0, 18.6), 1.1),
                    Disc((12.5, 20.8), 1.1),
                    Disc((17.0, 18.6), 1.1),
                ]);
                prims
            }
            Icon::Fog => vec![
                Line((4.0, 8.0), (20.0, 8.0)),
                Line((6.0, 12.5), (18.0, 12.5)),
                Line((4.0, 17.0), (20.0, 17.0)),
            ],
            Icon::Circle => vec![Ring((12.0, 12.0), 8.0)],
            Icon::Check => vec![
                Line((5.0, 12.5), (10.0, 17.5)),
                Line((10.0, 17.5), (19.0, 7.0)),
            ],
            Icon::Tick => vec![
                Line((5.0, 12.5), (9.5, 17.0)),
                Line((9.5, 17.0), (19.0, 7.5)),
            ],
            Icon::ChevronRight => vec![
                Line((9.0, 5.0), (16.0, 12.0)),
                Line((16.0, 12.0), (9.0, 19.0)),
            ],
            Icon::ChevronDown => vec![
                Line((5.0, 9.0), (12.0, 16.0)),
                Line((12.0, 16.0), (19.0, 9.0)),
            ],
            Icon::ChevronUp => vec![
                Line((5.0, 15.0), (12.0, 8.0)),
                Line((12.0, 8.0), (19.0, 15.0)),
            ],
            Icon::Plus => vec![
                Line((12.0, 5.0), (12.0, 19.0)),
                Line((5.0, 12.0), (19.0, 12.0)),
            ],
            Icon::Minus => vec![Line((5.0, 12.0), (19.0, 12.0))],
            Icon::Warning => vec![
                Line((12.0, 4.0), (21.0, 19.5)),
                Line((21.0, 19.5), (3.0, 19.5)),
                Line((3.0, 19.5), (12.0, 4.0)),
                Line((12.0, 9.5), (12.0, 14.0)),
                Disc((12.0, 16.8), 1.1),
            ],
            Icon::Info => vec![
                Ring((12.0, 12.0), 9.0),
                Line((12.0, 11.0), (12.0, 16.5)),
                Disc((12.0, 7.8), 1.1),
            ],
            Icon::Tray => vec![
                Frame(3.5, 5.5, 17.0, 13.0, 3.0),
                Line((3.5, 13.0), (8.5, 13.0)),
                Line((8.5, 13.0), (10.0, 15.5)),
                Line((10.0, 15.5), (14.0, 15.5)),
                Line((14.0, 15.5), (15.5, 13.0)),
                Line((15.5, 13.0), (20.5, 13.0)),
            ],
            Icon::Sliders => vec![
                Line((4.0, 8.0), (20.0, 8.0)),
                Line((4.0, 16.0), (20.0, 16.0)),
                Disc((9.0, 8.0), 2.6),
                Disc((15.0, 16.0), 2.6),
            ],
            Icon::CloudOff => {
                let mut prims = cloud();
                prims.push(Line((4.5, 4.5), (19.5, 19.5)));
                prims
            }
            Icon::Hourglass => vec![
                Line((6.5, 4.0), (17.5, 4.0)),
                Line((6.5, 20.0), (17.5, 20.0)),
                Line((7.5, 4.0), (7.5, 7.5)),
                Line((7.5, 7.5), (12.0, 12.0)),
                Line((12.0, 12.0), (7.5, 16.5)),
                Line((7.5, 16.5), (7.5, 20.0)),
                Line((16.5, 4.0), (16.5, 7.5)),
                Line((16.5, 7.5), (12.0, 12.0)),
                Line((12.0, 12.0), (16.5, 16.5)),
                Line((16.5, 16.5), (16.5, 20.0)),
            ],
            Icon::Shift => vec![
                Line((12.0, 19.0), (12.0, 5.5)),
                Line((5.5, 11.5), (12.0, 5.0)),
                Line((18.5, 11.5), (12.0, 5.0)),
            ],
            Icon::Backspace => vec![
                Line((9.0, 6.0), (20.0, 6.0)),
                Line((20.0, 6.0), (20.0, 18.0)),
                Line((20.0, 18.0), (9.0, 18.0)),
                Line((9.0, 18.0), (3.5, 12.0)),
                Line((3.5, 12.0), (9.0, 6.0)),
                Line((11.5, 9.5), (16.5, 14.5)),
                Line((16.5, 9.5), (11.5, 14.5)),
            ],
            Icon::Enter => vec![
                Line((19.0, 6.0), (19.0, 13.0)),
                Line((19.0, 13.0), (6.0, 13.0)),
                Line((6.0, 13.0), (10.5, 8.5)),
                Line((6.0, 13.0), (10.5, 17.5)),
            ],
            Icon::Eye => eye(),
            Icon::EyeOff => {
                let mut prims = eye();
                prims.push(Line((4.5, 4.5), (19.5, 19.5)));
                prims
            }
            Icon::Alert => vec![
                Line((12.0, 4.5), (20.5, 19.0)),
                Line((20.5, 19.0), (3.5, 19.0)),
                Line((3.5, 19.0), (12.0, 4.5)),
                Line((12.0, 10.0), (12.0, 13.5)),
                Disc((12.0, 16.2), 1.0),
            ],
            Icon::Home => vec![
                Line((3.5, 11.0), (12.0, 4.0)),
                Line((12.0, 4.0), (20.5, 11.0)),
                Line((5.8, 9.8), (5.8, 20.0)),
                Line((18.2, 9.8), (18.2, 20.0)),
                Line((5.8, 20.0), (18.2, 20.0)),
                Fill(10.0, 14.0, 4.0, 6.0, 1.0),
            ],
            Icon::Apps => vec![
                Frame(4.0, 4.0, 6.5, 6.5, 1.6),
                Frame(13.5, 4.0, 6.5, 6.5, 1.6),
                Frame(4.0, 13.5, 6.5, 6.5, 1.6),
                Frame(13.5, 13.5, 6.5, 6.5, 1.6),
            ],
            Icon::Wifi => wifi(),
            Icon::WifiOff => {
                let mut prims = wifi();
                prims.push(Line((4.5, 4.5), (19.5, 19.5)));
                prims
            }
            Icon::PartlyCloudy => {
                let mut prims = shifted(&CLOUD, 2.5);
                prims.push(Ring((16.0, 7.4), 2.8));
                for deg in [180.0f32, 225.0, 270.0, 315.0, 0.0] {
                    let (s, c) = deg.to_radians().sin_cos();
                    prims.push(Line(
                        (16.0 + 4.2 * c, 7.4 + 4.2 * s),
                        (16.0 + 5.4 * c, 7.4 + 5.4 * s),
                    ));
                }
                prims
            }
            Icon::CameraOff => vec![
                Frame(3.0, 6.5, 13.5, 11.0, 2.6),
                Tri((17.0, 11.0), (21.2, 8.4), (21.2, 15.6), 0.6),
                Line((4.5, 4.5), (19.5, 19.5)),
            ],
            Icon::Palette => vec![
                Arc((12.0, 12.0), 9.0, 70.0, 380.0),
                Line((15.1, 20.4), (17.4, 17.4)),
                Line((17.4, 17.4), (20.4, 15.1)),
                Disc((7.8, 12.0), 1.2),
                Disc((9.6, 7.8), 1.2),
                Disc((14.6, 7.6), 1.2),
                Disc((17.4, 11.6), 1.2),
            ],
            Icon::Speaker => vec![
                Line((4.0, 9.5), (8.0, 9.5)),
                Line((8.0, 9.5), (13.0, 5.5)),
                Line((13.0, 5.5), (13.0, 18.5)),
                Line((13.0, 18.5), (8.0, 14.5)),
                Line((8.0, 14.5), (4.0, 14.5)),
                Line((4.0, 14.5), (4.0, 9.5)),
                Arc((13.0, 12.0), 4.2, 315.0, 405.0),
                Arc((13.0, 12.0), 7.6, 315.0, 405.0),
            ],
            Icon::Clock => vec![
                Ring((12.0, 12.0), 9.0),
                Line((12.0, 12.0), (12.0, 6.8)),
                Line((12.0, 12.0), (15.8, 14.4)),
            ],
            Icon::Checklist => vec![
                Line((3.8, 6.6), (5.2, 8.0)),
                Line((5.2, 8.0), (7.6, 5.2)),
                Line((3.8, 12.1), (5.2, 13.5)),
                Line((5.2, 13.5), (7.6, 10.7)),
                Disc((5.6, 18.0), 1.4),
                Line((11.5, 6.5), (20.5, 6.5)),
                Line((11.5, 12.0), (20.5, 12.0)),
                Line((11.5, 18.0), (20.5, 18.0)),
            ],
            // The mock-ups' `updates`: an arrow down onto a flat line (no lips at its ends).
            Icon::Download => vec![
                Line((12.0, 4.0), (12.0, 15.0)),
                Line((7.0, 10.5), (12.0, 15.5)),
                Line((17.0, 10.5), (12.0, 15.5)),
                Line((5.0, 20.0), (19.0, 20.0)),
            ],
            Icon::Bug => vec![
                Arc((12.0, 9.0), 3.2, 180.0, 360.0),
                Frame(7.5, 9.0, 9.0, 11.0, 4.0),
                Line((12.0, 9.5), (12.0, 20.0)),
                Line((9.7, 6.7), (8.0, 4.2)),
                Line((14.3, 6.7), (16.0, 4.2)),
                Line((7.5, 12.0), (4.0, 10.5)),
                Line((7.5, 15.0), (3.8, 15.0)),
                Line((8.5, 18.5), (5.0, 20.5)),
                Line((16.5, 12.0), (20.0, 10.5)),
                Line((16.5, 15.0), (20.2, 15.0)),
                Line((15.5, 18.5), (19.0, 20.5)),
            ],
            Icon::Puzzle => vec![
                Frame(7.0, 7.5, 10.0, 7.0, 2.5),
                Line((10.0, 7.5), (10.0, 3.5)),
                Line((14.0, 7.5), (14.0, 3.5)),
                Line((12.0, 14.5), (12.0, 16.5)),
                Arc((16.0, 16.5), 4.0, 90.0, 180.0),
                Line((16.0, 20.5), (20.5, 20.5)),
            ],
            Icon::Search => vec![Ring((10.5, 10.5), 6.5), Line((15.3, 15.3), (20.0, 20.0))],
            Icon::Refresh => refresh(),
            Icon::Moon => vec![
                Arc((12.0, 12.0), 9.0, 5.0, 275.0),
                Arc((16.8, 7.2), 7.0, 53.0, 217.0),
            ],
            Icon::Stop => vec![Fill(6.5, 6.5, 11.0, 11.0, 2.4)],
            Icon::Bell => vec![
                Arc((12.0, 10.0), 6.0, 180.0, 360.0),
                Line((6.0, 10.0), (4.8, 16.5)),
                Line((18.0, 10.0), (19.2, 16.5)),
                Line((4.5, 16.5), (19.5, 16.5)),
                Disc((12.0, 19.6), 1.5),
                Line((12.0, 3.4), (12.0, 4.2)),
            ],
            Icon::BellOff => {
                let mut prims = Icon::Bell.prims();
                prims.push(Line((4.0, 4.0), (20.0, 20.0)));
                prims
            }
            Icon::Power => vec![
                Line((12.0, 3.5), (12.0, 11.5)),
                Arc((12.0, 12.0), 7.5, 311.8, 588.2),
            ],
            Icon::SunSmall => vec![
                Ring((12.0, 12.0), 3.2),
                Line((12.0, 4.5), (12.0, 6.1)),
                Line((12.0, 17.9), (12.0, 19.5)),
                Line((4.5, 12.0), (6.1, 12.0)),
                Line((17.9, 12.0), (19.5, 12.0)),
            ],
            Icon::Pot => vec![
                Line((5.5, 10.0), (18.5, 10.0)),
                Line((18.5, 10.0), (18.5, 16.0)),
                Arc((14.5, 16.0), 4.0, 0.0, 90.0),
                Line((14.5, 20.0), (9.5, 20.0)),
                Arc((9.5, 16.0), 4.0, 90.0, 180.0),
                Line((5.5, 16.0), (5.5, 10.0)),
                Line((3.0, 12.2), (5.5, 12.2)),
                Line((18.5, 12.2), (21.0, 12.2)),
                Line((8.5, 7.4), (15.5, 7.4)),
                Line((12.0, 7.4), (12.0, 5.8)),
            ],
            Icon::Calendar => vec![
                Frame(4.5, 5.0, 15.0, 15.5, 1.5),
                Line((4.5, 10.0), (19.5, 10.0)),
                Line((8.5, 3.0), (8.5, 7.0)),
                Line((15.5, 3.0), (15.5, 7.0)),
            ],
            Icon::Message => vec![
                Line((5.5, 5.0), (18.5, 5.0)),
                Arc((18.5, 6.5), 1.5, 270.0, 360.0),
                Line((20.0, 6.5), (20.0, 14.5)),
                Arc((18.5, 14.5), 1.5, 0.0, 90.0),
                Line((18.5, 16.0), (11.0, 16.0)),
                Line((11.0, 16.0), (6.5, 19.7)),
                Line((6.5, 19.7), (6.5, 16.0)),
                Line((6.5, 16.0), (5.5, 16.0)),
                Arc((5.5, 14.5), 1.5, 90.0, 180.0),
                Line((4.0, 14.5), (4.0, 6.5)),
                Arc((5.5, 6.5), 1.5, 180.0, 270.0),
            ],
            Icon::Package => vec![
                Line((12.0, 3.0), (20.0, 7.2)),
                Line((20.0, 7.2), (20.0, 16.8)),
                Line((20.0, 16.8), (12.0, 21.0)),
                Line((12.0, 21.0), (4.0, 16.8)),
                Line((4.0, 16.8), (4.0, 7.2)),
                Line((4.0, 7.2), (12.0, 3.0)),
                Line((4.0, 7.2), (12.0, 11.5)),
                Line((12.0, 11.5), (20.0, 7.2)),
                Line((12.0, 11.5), (12.0, 21.0)),
            ],
            Icon::Pin => vec![
                Line((12.0, 16.5), (12.0, 21.0)),
                Line((8.0, 3.5), (16.0, 3.5)),
                Line((9.0, 3.5), (9.7, 9.2)),
                Line((9.7, 9.2), (6.5, 12.5)),
                Line((6.5, 12.5), (17.5, 12.5)),
                Line((17.5, 12.5), (14.3, 9.2)),
                Line((14.3, 9.2), (15.0, 3.5)),
            ],
            Icon::Undo => vec![
                Line((4.0, 10.0), (14.0, 10.0)),
                Arc((14.0, 15.0), 5.0, 270.0, 450.0),
                Line((14.0, 20.0), (11.0, 20.0)),
                Line((4.0, 10.0), (8.0, 6.0)),
                Line((4.0, 10.0), (8.0, 14.0)),
            ],
            Icon::HomeAssistant => vec![
                Line((3.5, 11.0), (12.0, 3.5)),
                Line((12.0, 3.5), (20.5, 11.0)),
                Line((6.0, 10.0), (6.0, 20.0)),
                Line((6.0, 20.0), (18.0, 20.0)),
                Line((18.0, 20.0), (18.0, 10.0)),
                Line((12.0, 12.0), (12.0, 17.0)),
                Line((9.5, 14.5), (14.5, 14.5)),
            ],
            Icon::StillCamera => vec![
                Line((4.0, 8.0), (7.0, 8.0)),
                Line((7.0, 8.0), (8.4, 5.6)),
                Line((8.4, 5.6), (15.6, 5.6)),
                Line((15.6, 5.6), (17.0, 8.0)),
                Line((17.0, 8.0), (20.0, 8.0)),
                Arc((20.0, 9.0), 1.0, 270.0, 360.0),
                Line((21.0, 9.0), (21.0, 18.0)),
                Arc((20.0, 18.0), 1.0, 0.0, 90.0),
                Line((20.0, 19.0), (4.0, 19.0)),
                Arc((4.0, 18.0), 1.0, 90.0, 180.0),
                Line((3.0, 18.0), (3.0, 9.0)),
                Arc((4.0, 9.0), 1.0, 180.0, 270.0),
                Ring((12.0, 13.4), 3.5),
            ],
            Icon::LockOutline => vec![
                Frame(6.5, 11.0, 11.0, 8.5, 2.0),
                Line((8.5, 11.0), (8.5, 8.0)),
                Arc((12.0, 8.0), 3.5, 180.0, 360.0),
                Line((15.5, 8.0), (15.5, 11.0)),
            ],
            Icon::AlarmOutline => vec![
                Ring((12.0, 13.5), 7.0),
                Line((12.0, 10.0), (12.0, 13.5)),
                Line((12.0, 13.5), (14.2, 14.8)),
                Line((4.0, 5.5), (7.0, 3.0)),
                Line((20.0, 5.5), (17.0, 3.0)),
            ],
            // A ring open at its top left, three quarters of a turn: something under way (the mock-ups' `spinner`).
            Icon::Spinner => vec![Arc((12.0, 12.0), 9.0, 270.0, 540.0)],
            // An open ring with a square corner at its start, the way the mock-ups draw "again" and "restart".
            Icon::Restart => vec![
                Arc((12.0, 12.0), 8.0, 0.0, 312.5),
                Line((20.0, 4.0), (20.0, 9.0)),
                Line((20.0, 9.0), (15.0, 9.0)),
            ],
            // A ring with a short stroke across it, the mock-ups' `stop`: `circle r=9` and `M8.5 8.5l7 7`.
            Icon::Block => vec![Ring((12.0, 12.0), 9.0), Line((8.5, 8.5), (15.5, 15.5))],
            // The mock-ups' `gauge`: `M4.5 17a8.5 8.5 0 1 1 15 0` (an arc over the top of a circle centred at 12, 13), the needle
            // `M12 13.5l4-4.5` and the hub, a disc of 1.2.
            Icon::Gauge => vec![
                Arc((12.0, 13.0), 8.5, 151.93, 388.07),
                Line((12.0, 13.5), (16.0, 9.0)),
                Disc((12.0, 13.5), 1.2),
            ],
            // `rect 7 7 10 10 rx2` and eight pins of 3 units.
            Icon::Cpu => vec![
                Frame(7.0, 7.0, 10.0, 10.0, 2.0),
                Line((10.0, 3.5), (10.0, 6.5)),
                Line((14.0, 3.5), (14.0, 6.5)),
                Line((10.0, 17.5), (10.0, 20.5)),
                Line((14.0, 17.5), (14.0, 20.5)),
                Line((3.5, 10.0), (6.5, 10.0)),
                Line((3.5, 14.0), (6.5, 14.0)),
                Line((17.5, 10.0), (20.5, 10.0)),
                Line((17.5, 14.0), (20.5, 14.0)),
            ],
            // `rect 3.5 5 17 14 rx3`, a circle of 1.6 at (9, 10.5) and `M4 17l5-4.5 4 3.5 3-2.5 4 3`.
            Icon::Photo => vec![
                Frame(3.5, 5.0, 17.0, 14.0, 3.0),
                Ring((9.0, 10.5), 1.6),
                Line((4.0, 17.0), (9.0, 12.5)),
                Line((9.0, 12.5), (13.0, 16.0)),
                Line((13.0, 16.0), (16.0, 13.5)),
                Line((16.0, 13.5), (20.0, 16.5)),
            ],
            // `M7 19h10a4 4 0 0 0 .6-7.95A5.5 5.5 0 0 0 7.2 10.4 4.3 4.3 0 0 0 7 19z`: the floor, then the right, top and left lobes.
            Icon::CloudOutline => vec![
                Line((7.0, 19.0), (17.0, 19.0)),
                Arc((16.8, 15.0), 4.0, 281.5, 450.0),
                Arc((12.35, 12.5), 5.5, 202.2, 344.6),
                Arc((7.1, 14.7), 4.3, 90.0, 270.0),
            ],
            // `rect 4 5 16 11 rx3` and its stand `M8 20h8`.
            Icon::Cards => vec![
                Frame(4.0, 5.0, 16.0, 11.0, 3.0),
                Line((8.0, 20.0), (16.0, 20.0)),
            ],
            // Three of four 7-unit squares, rx 2.
            Icon::Tiles => vec![
                Frame(4.0, 4.0, 7.0, 7.0, 2.0),
                Frame(13.0, 4.0, 7.0, 7.0, 2.0),
                Frame(4.0, 13.0, 7.0, 7.0, 2.0),
            ],
            // `M12 3.5l7 2.5v5.5c0 4.5-3 7.5-7 9-4-1.5-7-4.5-7-9V6z`: the curves of its foot are cut to the segments they pass through.
            Icon::Shield => vec![
                Line((12.0, 3.5), (19.0, 6.0)),
                Line((19.0, 6.0), (19.0, 11.5)),
                Line((19.0, 11.5), (18.47, 14.59)),
                Line((18.47, 14.59), (17.0, 17.12)),
                Line((17.0, 17.12), (14.78, 19.09)),
                Line((14.78, 19.09), (12.0, 20.5)),
                Line((12.0, 20.5), (9.22, 19.09)),
                Line((9.22, 19.09), (7.0, 17.12)),
                Line((7.0, 17.12), (5.53, 14.59)),
                Line((5.53, 14.59), (5.0, 11.5)),
                Line((5.0, 11.5), (5.0, 6.0)),
                Line((5.0, 6.0), (12.0, 3.5)),
            ],
            // `M8.5 5.5v13M15.5 5.5v13`.
            Icon::Still => vec![
                Line((8.5, 5.5), (8.5, 18.5)),
                Line((15.5, 5.5), (15.5, 18.5)),
            ],
            Icon::Person => person(),
            // The person and the two arcs `M3.5 6.5a10 10 0 0 1 3-3M20.5 6.5a10 10 0 0 0-3-3`: each a short stretch of the circle of 10
            // that passes through both its ends (about (11.91, 11.91) and its mirror), one to either side of the head.
            Icon::Presence => {
                let mut prims = person();
                prims.extend([
                    Arc((11.91, 11.91), 10.0, 212.75, 237.25),
                    Arc((12.09, 11.91), 10.0, 302.75, 327.25),
                ]);
                prims
            }
            // `rect 4 4.5 16 6 rx2`, `rect 4 13.5 16 6 rx2` and the two lights `M7.5 7.5h.01M7.5 16.5h.01` (a round dot of the stroke's width).
            Icon::Server => vec![
                Frame(4.0, 4.5, 16.0, 6.0, 2.0),
                Frame(4.0, 13.5, 16.0, 6.0, 2.0),
                Line((7.5, 7.5), (7.51, 7.5)),
                Line((7.5, 16.5), (7.51, 16.5)),
            ],
            // `circle 12 12 r3` and the eight rays `M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M18.4 5.6l-2.1 2.1M7.7 16.3l-2.1 2.1`.
            Icon::Cog => vec![
                Ring((12.0, 12.0), 3.0),
                Line((12.0, 3.0), (12.0, 6.0)),
                Line((12.0, 18.0), (12.0, 21.0)),
                Line((3.0, 12.0), (6.0, 12.0)),
                Line((18.0, 12.0), (21.0, 12.0)),
                Line((5.6, 5.6), (7.7, 7.7)),
                Line((16.3, 16.3), (18.4, 18.4)),
                Line((18.4, 5.6), (16.3, 7.7)),
                Line((7.7, 16.3), (5.6, 18.4)),
            ],
            // `rect 3 6 12 12 rx2` and the wedge `M15 11l6-3.5v9L15 13`.
            Icon::Video => vec![
                Frame(3.0, 6.0, 12.0, 12.0, 2.0),
                Line((15.0, 11.0), (21.0, 7.5)),
                Line((21.0, 7.5), (21.0, 16.5)),
                Line((21.0, 16.5), (15.0, 13.0)),
            ],
            // `rect 3 7 12 10 rx2.4` and the wedge `M15 10.5l5-2.8v8.6l-5-2.8`.
            Icon::Sendcam => sendcam(),
            // The same and the stroke `M3 3l18 18` across it.
            Icon::SendcamOff => {
                let mut prims = sendcam();
                prims.push(Line((3.0, 3.0), (21.0, 21.0)));
                prims
            }
            // `M8 5l11 7-11 7z`.
            Icon::PlayOutline => vec![
                Line((8.0, 5.0), (19.0, 12.0)),
                Line((19.0, 12.0), (8.0, 19.0)),
                Line((8.0, 19.0), (8.0, 5.0)),
            ],
        }
    }

    /// The width of the icon's strokes in grid units: the family's, but for the icon that is drawn as a mock-up draws it.
    fn stroke(self) -> f32 {
        match self {
            Icon::AlarmOutline => 2.0,
            Icon::Tick => 2.6,
            _ => STROKE,
        }
    }

    /// Draws the icon into `pixels` (premultiplied RGBA, `size` x `size`), replacing what was there.
    pub fn draw(self, pixels: &mut [u8], size: u32, colour: Rgba) {
        let prims = self.prims();
        let (stroke, scale) = (self.stroke(), size as f32 / GRID);
        for y in 0..size {
            for x in 0..size {
                let p = ((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale);
                let coverage = prims
                    .iter()
                    .map(|prim| coverage(prim, p, scale, stroke))
                    .fold(0.0f32, f32::max);
                let at = (y * size + x) as usize * 4;
                let a = (coverage * 255.0 + 0.5) as u32;
                let px = [colour.0, colour.1, colour.2, colour.3]
                    .map(|c| (u32::from(c) * a / 255) as u8);
                pixels[at..at + 4].copy_from_slice(&px);
            }
        }
    }
}

/// `prims` moved by `dy` grid units (the weather icons put their cloud higher to leave room below it).
fn shifted(prims: &[Prim], dy: f32) -> Vec<Prim> {
    prims
        .iter()
        .map(|prim| match *prim {
            Prim::Line(a, b) => Prim::Line((a.0, a.1 + dy), (b.0, b.1 + dy)),
            Prim::Ring(c, r) => Prim::Ring((c.0, c.1 + dy), r),
            Prim::Disc(c, r) => Prim::Disc((c.0, c.1 + dy), r),
            Prim::Arc(c, r, a0, a1) => Prim::Arc((c.0, c.1 + dy), r, a0, a1),
            Prim::Tri(a, b, c, r) => {
                Prim::Tri((a.0, a.1 + dy), (b.0, b.1 + dy), (c.0, c.1 + dy), r)
            }
            Prim::Fill(x, y, w, h, r) => Prim::Fill(x, y + dy, w, h, r),
            Prim::Frame(x, y, w, h, r) => Prim::Frame(x, y + dy, w, h, r),
        })
        .collect()
}

/// How much of a pixel at `p` (grid units) a primitive covers, 0..=1, with a one-pixel-wide soft edge (`scale`
/// pixels per grid unit); its strokes are `width` grid units wide.
fn coverage(prim: &Prim, p: P, scale: f32, width: f32) -> f32 {
    let half = width / 2.0;
    let stroke = |d: f32| (0.5 + (half - d) * scale).clamp(0.0, 1.0);
    let fill = |sd: f32| (0.5 - sd * scale).clamp(0.0, 1.0);
    match *prim {
        Prim::Line(a, b) => stroke(dist_segment(p, a, b)),
        Prim::Ring(c, r) => stroke((dist(p, c) - r).abs()),
        Prim::Disc(c, r) => fill(dist(p, c) - r),
        Prim::Arc(c, r, a0, a1) => stroke(dist_arc(p, c, r, a0, a1)),
        Prim::Tri(a, b, c, round) => fill(sd_triangle(p, a, b, c, round)),
        Prim::Fill(x, y, w, h, r) => fill(sd_rrect(p, x, y, w, h, r)),
        Prim::Frame(x, y, w, h, r) => stroke(sd_rrect(p, x, y, w, h, r).abs()),
    }
}

fn dist(a: P, b: P) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn dist_segment(p: P, a: P, b: P) -> f32 {
    let (ab, ap) = ((b.0 - a.0, b.1 - a.1), (p.0 - a.0, p.1 - a.1));
    let len2 = ab.0 * ab.0 + ab.1 * ab.1;
    let t = if len2 == 0.0 {
        0.0
    } else {
        ((ap.0 * ab.0 + ap.1 * ab.1) / len2).clamp(0.0, 1.0)
    };
    dist(p, (a.0 + ab.0 * t, a.1 + ab.1 * t))
}

/// Distance from `p` to the arc of the circle (`c`, `r`) that runs clockwise on screen from `a0` to `a1` degrees.
fn dist_arc(p: P, c: P, r: f32, a0: f32, a1: f32) -> f32 {
    let mut angle = (p.1 - c.1).atan2(p.0 - c.0).to_degrees();
    while angle < a0 {
        angle += 360.0;
    }
    if angle <= a1 || angle - 360.0 >= a0 {
        return (dist(p, c) - r).abs();
    }
    let end = |a: f32| {
        let (s, co) = a.to_radians().sin_cos();
        (c.0 + r * co, c.1 + r * s)
    };
    dist(p, end(a0)).min(dist(p, end(a1)))
}

/// Signed distance to a rounded rectangle (negative inside).
fn sd_rrect(p: P, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let r = r.min(w / 2.0).min(h / 2.0);
    let q = (
        (p.0 - cx).abs() - (w / 2.0 - r),
        (p.1 - cy).abs() - (h / 2.0 - r),
    );
    let outside = (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt();
    outside + q.0.max(q.1).min(0.0) - r
}

/// Signed distance to a triangle whose corners are rounded by `round` (the triangle is drawn as its inset by
/// `round` grown by `round`).
fn sd_triangle(p: P, a: P, b: P, c: P, round: f32) -> f32 {
    let centre = ((a.0 + b.0 + c.0) / 3.0, (a.1 + b.1 + c.1) / 3.0);
    let inset = |v: P| {
        let d = (centre.0 - v.0, centre.1 - v.1);
        let len = (d.0 * d.0 + d.1 * d.1).sqrt();
        // Moving every corner towards the centre by round / sin(half the corner angle) shrinks each side by
        // `round`; the centroid direction bisects the corner of an isosceles-ish triangle well enough for icons.
        let k = (round * 2.0 / len.max(1e-3)).min(0.9);
        (v.0 + d.0 * k, v.1 + d.1 * k)
    };
    let (a, b, c) = (inset(a), inset(b), inset(c));
    let edge = |u: P, v: P| {
        let (e, w) = ((v.0 - u.0, v.1 - u.1), (p.0 - u.0, p.1 - u.1));
        let t = ((w.0 * e.0 + w.1 * e.1) / (e.0 * e.0 + e.1 * e.1)).clamp(0.0, 1.0);
        let d = (w.0 - e.0 * t, w.1 - e.1 * t);
        let cross = e.0 * w.1 - e.1 * w.0;
        ((d.0 * d.0 + d.1 * d.1).sqrt(), cross)
    };
    let orientation = ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).signum();
    let mut nearest = f32::MAX;
    let mut inside = true;
    for (u, v) in [(a, b), (b, c), (c, a)] {
        let (d, cross) = edge(u, v);
        nearest = nearest.min(d);
        inside &= cross * orientation >= 0.0;
    }
    (if inside { -nearest } else { nearest }) - round
}

/// An icon as a square image layer at `frame` (its side is `frame.w`), drawn in `colour`, as a part.
pub fn icon_part(ui: &mut Ui, icon: Icon, frame: Frame, z: i32, colour: Rgba) -> io::Result<Part> {
    let mut layer = ui.scene.image(
        frame.w as u32,
        frame.h as u32,
        z,
        Props::at(frame.x, frame.y),
    )?;
    paint(&mut layer, icon, colour);
    Ok(Part::new(layer, frame))
}

/// Draws `icon` in `colour` into a square icon layer, replacing what it showed.
pub fn paint(layer: &mut ImageLayer, icon: Icon, colour: Rgba) {
    let size = layer.width();
    icon.draw(layer.pixels_mut(), size, colour);
    layer.damage(None);
}

/// Draws `icon` over what `layer` already shows, `size` pixels square with its top left at (`x`, `y`) in the layer's pixels (what
/// falls outside the layer is skipped). For an icon that shares a layer with text (a key's legend, the mark in front of a message);
/// the caller tells the compositor with `damage` once everything is drawn.
pub fn paint_over(layer: &mut ImageLayer, icon: Icon, x: i32, y: i32, size: u32, colour: Rgba) {
    let mut glyph = vec![0u8; (size * size * 4) as usize];
    icon.draw(&mut glyph, size, colour);
    let (w, h) = (layer.width() as i32, layer.height() as i32);
    let pixels = layer.pixels_mut();
    for gy in 0..size as i32 {
        for gx in 0..size as i32 {
            let (dx, dy) = (x + gx, y + gy);
            if dx < 0 || dy < 0 || dx >= w || dy >= h {
                continue;
            }
            let src = &glyph[((gy as u32 * size + gx as u32) * 4) as usize..][..4];
            let keep = 255 - u32::from(src[3]);
            let dst = &mut pixels[((dy * w + dx) * 4) as usize..][..4];
            for (d, s) in dst.iter_mut().zip(src) {
                *d = (u32::from(*s) + u32::from(*d) * keep / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Rgba = Rgba(255, 255, 255, 255);
    const ALL: [Icon; 79] = [
        Icon::Play,
        Icon::Pause,
        Icon::Next,
        Icon::Previous,
        Icon::Back,
        Icon::Close,
        Icon::Alarm,
        Icon::Timer,
        Icon::Phone,
        Icon::Mic,
        Icon::MicOff,
        Icon::Bulb,
        Icon::Thermometer,
        Icon::Droplet,
        Icon::Lock,
        Icon::Room,
        Icon::Camera,
        Icon::Expand,
        Icon::Sun,
        Icon::Cloud,
        Icon::Rain,
        Icon::Storm,
        Icon::Snow,
        Icon::Fog,
        Icon::Check,
        Icon::Tick,
        Icon::ChevronRight,
        Icon::ChevronDown,
        Icon::ChevronUp,
        Icon::Plus,
        Icon::Minus,
        Icon::Warning,
        Icon::Info,
        Icon::Tray,
        Icon::Sliders,
        Icon::CloudOff,
        Icon::Hourglass,
        Icon::Shift,
        Icon::Backspace,
        Icon::Enter,
        Icon::Eye,
        Icon::EyeOff,
        Icon::Alert,
        Icon::Home,
        Icon::Apps,
        Icon::Wifi,
        Icon::WifiOff,
        Icon::PartlyCloudy,
        Icon::Bell,
        Icon::Circle,
        Icon::CameraOff,
        Icon::Palette,
        Icon::Speaker,
        Icon::Clock,
        Icon::Checklist,
        Icon::Download,
        Icon::Bug,
        Icon::Puzzle,
        Icon::Search,
        Icon::Refresh,
        Icon::Moon,
        Icon::Stop,
        Icon::Block,
        Icon::Gauge,
        Icon::Cpu,
        Icon::Photo,
        Icon::CloudOutline,
        Icon::Cards,
        Icon::Tiles,
        Icon::Shield,
        Icon::Still,
        Icon::Presence,
        Icon::Person,
        Icon::Server,
        Icon::Cog,
        Icon::Video,
        Icon::Sendcam,
        Icon::SendcamOff,
        Icon::PlayOutline,
    ];

    fn render(icon: Icon, size: u32) -> Vec<u8> {
        let mut px = vec![0u8; (size * size * 4) as usize];
        icon.draw(&mut px, size, WHITE);
        px
    }

    #[test]
    fn the_outline_of_a_play_is_a_triangle_with_nothing_in_it_and_is_not_the_filled_one() {
        let (outline, filled) = (render(Icon::PlayOutline, 48), render(Icon::Play, 48));
        let on = |px: &[u8], x: usize, y: usize| px[(y * 48 + x) * 4 + 3] > 127;
        // The middle of the triangle (a third of the way from its flat side) is ink in the filled one and open in the outline; the
        // flat side's middle is ink in both.
        assert!(on(&filled, 24, 24) && !on(&outline, 24, 24));
        assert!(on(&outline, 16, 24) && on(&filled, 16, 24));
        // The tip is where the mock's path puts it: 19 of 24 across, half way down.
        assert!(on(&outline, 37, 24) && !on(&outline, 41, 24));
        assert_ne!(outline, filled);
    }

    fn inked(px: &[u8]) -> usize {
        px.chunks_exact(4).filter(|p| p[3] > 127).count()
    }

    #[test]
    fn every_icon_draws_something_that_fits_the_layer_and_is_not_a_solid_block() {
        for icon in ALL {
            let px = render(icon, 32);
            let n = inked(&px);
            assert!(n > 30, "{icon:?} draws only {n} pixels");
            assert!(n < 32 * 32 * 3 / 4, "{icon:?} fills the whole layer: {n}");
            // Nothing touches the outermost ring of pixels: the design grid keeps a margin so a scaled press
            // never shows a clipped icon.
            for i in 0..32usize {
                for edge in [i, 31 * 32 + i, i * 32, i * 32 + 31] {
                    assert!(px[edge * 4 + 3] < 40, "{icon:?} is clipped at pixel {edge}");
                }
            }
        }
    }

    #[test]
    fn a_pause_is_two_bars_with_a_gap_and_a_play_is_one_triangle() {
        let pause = render(Icon::Pause, 48);
        let row = 24 * 48;
        let bars: Vec<bool> = (0..48).map(|x| pause[(row + x) * 4 + 3] > 127).collect();
        let runs = bars.windows(2).filter(|w| !w[0] && w[1]).count();
        assert_eq!(runs, 2, "{bars:?}");
        let play = render(Icon::Play, 48);
        let widths: Vec<usize> = [14usize, 24, 34]
            .iter()
            .map(|y| (0..48).filter(|x| play[(y * 48 + x) * 4 + 3] > 127).count())
            .collect();
        assert!(widths[1] > widths[0] && widths[1] > widths[2], "{widths:?}");
    }

    #[test]
    fn the_colour_is_premultiplied_by_coverage_and_never_exceeds_alpha() {
        let mut px = vec![0u8; 32 * 32 * 4];
        Icon::Close.draw(&mut px, 32, Rgba(200, 100, 50, 255));
        for p in px.chunks_exact(4) {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
            assert!(u32::from(p[0]) * 255 <= 200 * u32::from(p[3]) + 255);
        }
    }

    #[test]
    fn a_mic_with_a_slash_covers_more_than_the_same_mic() {
        assert!(inked(&render(Icon::MicOff, 48)) > inked(&render(Icon::Mic, 48)));
    }

    #[test]
    fn an_arc_covers_only_the_angles_it_was_asked_for() {
        // The upper half circle (180..360 degrees, through 270 = up): a point above the centre is on it, a point
        // below is not.
        let c = (12.0, 12.0);
        assert!(dist_arc((12.0, 6.0), c, 6.0, 180.0, 360.0) < 0.01);
        assert!(dist_arc((12.0, 18.0), c, 6.0, 180.0, 360.0) > 5.0);
        assert!(
            dist_arc((6.0, 12.0), c, 6.0, 180.0, 360.0) < 0.01,
            "the start of the arc"
        );
    }

    #[test]
    fn a_struck_eye_covers_more_than_the_open_one_and_the_shift_arrow_points_up() {
        assert!(inked(&render(Icon::EyeOff, 48)) > inked(&render(Icon::Eye, 48)));
        // The arrow's head is narrow at the top and wide lower down; its shaft is a single stroke below the head.
        let arrow = render(Icon::Shift, 48);
        let width = |y: usize| {
            (0..48)
                .filter(|x| arrow[(y * 48 + x) * 4 + 3] > 127)
                .count()
        };
        assert!(width(10) < width(22), "{} {}", width(10), width(22));
        assert!(width(22) > width(40));
    }
}

#[cfg(test)]
mod settings_icon_tests {
    use super::*;

    const NEW: [Icon; 10] = [
        Icon::Palette,
        Icon::Speaker,
        Icon::Clock,
        Icon::Checklist,
        Icon::Download,
        Icon::Bug,
        Icon::Puzzle,
        Icon::Search,
        Icon::Refresh,
        Icon::Moon,
    ];

    fn render(icon: Icon, size: u32) -> Vec<u8> {
        let mut px = vec![0u8; (size * size * 4) as usize];
        icon.draw(&mut px, size, Rgba(255, 255, 255, 255));
        px
    }

    #[test]
    fn each_settings_icon_draws_pixels_at_both_sizes_and_no_two_look_alike() {
        for size in [24u32, 32] {
            let drawn: Vec<Vec<u8>> = NEW.iter().map(|icon| render(*icon, size)).collect();
            for (icon, px) in NEW.iter().zip(&drawn) {
                let inked = px.chunks_exact(4).filter(|p| p[3] > 127).count();
                assert!(inked > 20, "{icon:?} draws only {inked} pixels at {size}");
            }
            for (i, a) in drawn.iter().enumerate() {
                for (j, b) in drawn.iter().enumerate().skip(i + 1) {
                    assert_ne!(a, b, "{:?} and {:?} look alike at {size}", NEW[i], NEW[j]);
                }
            }
        }
    }
}

#[cfg(test)]
mod shade_icon_tests {
    use super::*;

    /// The glyphs the pull-down shade draws that the toolkit did not have: tiles, owners, pins and Undo, the still camera and the
    /// outline lock.
    const SHADE: [Icon; 12] = [
        Icon::BellOff,
        Icon::Power,
        Icon::SunSmall,
        Icon::Pot,
        Icon::Calendar,
        Icon::Message,
        Icon::Package,
        Icon::Pin,
        Icon::Undo,
        Icon::HomeAssistant,
        Icon::StillCamera,
        Icon::LockOutline,
    ];

    fn render(icon: Icon, size: u32) -> Vec<u8> {
        let mut px = vec![0u8; (size * size * 4) as usize];
        icon.draw(&mut px, size, Rgba(255, 255, 255, 255));
        px
    }

    fn inked(px: &[u8]) -> usize {
        px.chunks_exact(4).filter(|p| p[3] > 127).count()
    }

    #[test]
    fn each_shade_icon_draws_something_that_stays_inside_its_square_and_none_looks_like_another_or_like_its_plain_twin()
     {
        for size in [16u32, 24, 28, 30] {
            let drawn: Vec<Vec<u8>> = SHADE.iter().map(|icon| render(*icon, size)).collect();
            for (icon, px) in SHADE.iter().zip(&drawn) {
                let n = inked(px);
                assert!(n > 12, "{icon:?} draws only {n} pixels at {size}");
                assert!(
                    n < (size * size) as usize * 3 / 4,
                    "{icon:?} is a block at {size}"
                );
                for i in 0..size as usize {
                    let s = size as usize;
                    for edge in [i, (s - 1) * s + i, i * s, i * s + s - 1] {
                        assert!(
                            px[edge * 4 + 3] < 40,
                            "{icon:?} is clipped at pixel {edge} of {size}"
                        );
                    }
                }
            }
            for (i, a) in drawn.iter().enumerate() {
                for (j, b) in drawn.iter().enumerate().skip(i + 1) {
                    assert_ne!(
                        a, b,
                        "{:?} and {:?} look alike at {size}",
                        SHADE[i], SHADE[j]
                    );
                }
            }
        }
        // Struck through means more ink than the thing it strikes, and it is not the same picture.
        assert!(inked(&render(Icon::BellOff, 48)) > inked(&render(Icon::Bell, 48)));
        assert_ne!(render(Icon::SunSmall, 48), render(Icon::Sun, 48));
        assert_ne!(render(Icon::StillCamera, 48), render(Icon::Camera, 48));
        assert_ne!(render(Icon::LockOutline, 48), render(Icon::Lock, 48));
    }

    /// Whether the pixel at the grid point (`x`, `y`) of a 48-pixel render is inked.
    fn on(px: &[u8], x: f32, y: f32) -> bool {
        let (cx, cy) = ((x * 2.0) as usize, (y * 2.0) as usize);
        px[(cy * 48 + cx) * 4 + 3] > 127
    }

    #[test]
    fn the_still_camera_has_a_lens_and_a_viewfinder_bump_and_the_video_camera_has_neither() {
        let still = render(Icon::StillCamera, 48);
        // The lens is a ring: its middle is clear and its rim is ink.
        assert!(!on(&still, 12.0, 13.4), "the lens is open at its middle");
        assert!(on(&still, 12.0, 13.4 - 3.5), "the rim of the lens");
        assert!(on(&still, 12.0, 13.4 + 3.5), "the rim of the lens");
        // The bump above the body: ink on its top edge at the middle, none above it, and the body's top edge beside it at a lower line.
        assert!(on(&still, 12.0, 5.6), "the top of the viewfinder bump");
        assert!(!on(&still, 12.0, 3.5), "nothing above the bump");
        assert!(
            on(&still, 5.0, 8.0) && !on(&still, 5.0, 5.6),
            "the body is lower than the bump"
        );
        // The video camera is a body and a wedge: no ring where the still camera's lens has its rim.
        let video = render(Icon::Camera, 48);
        assert!(on(&still, 12.0, 13.4 - 3.5) && !on(&video, 12.0, 13.4 - 3.5));
    }

    #[test]
    fn the_outline_lock_is_hollow_where_the_plain_lock_is_solid_and_has_the_same_shackle_room() {
        let outline = render(Icon::LockOutline, 48);
        let plain = render(Icon::Lock, 48);
        // The middle of the body: the plain lock is filled, the outline is not.
        assert!(on(&plain, 12.0, 15.5) && !on(&outline, 12.0, 15.5));
        // The body's frame and the shackle's crown are ink in the outline, and the shackle's inside is clear.
        assert!(on(&outline, 6.5, 15.0) && on(&outline, 17.5, 15.0));
        assert!(
            on(&outline, 12.0, 11.0) && on(&outline, 12.0, 19.5),
            "the body's top and bottom edges"
        );
        assert!(on(&outline, 12.0, 4.5), "the crown of the shackle");
        assert!(!on(&outline, 12.0, 7.0), "the shackle is open inside");
    }
}

#[cfg(test)]
mod night_icon_tests {
    use super::*;

    fn render(icon: Icon, size: u32) -> Vec<u8> {
        let mut px = vec![0u8; (size * size * 4) as usize];
        icon.draw(&mut px, size, Rgba(227, 59, 53, 255));
        px
    }

    /// The columns and rows (first, last + 1) where a square render has ink (a coverage above 3.5 %, as a level of 8 in the red
    /// of the mock's shot is), and the sum of its coverage in whole pixels.
    fn ink(px: &[u8], size: usize) -> ((usize, usize), (usize, usize), f32) {
        let (mut cols, mut rows, mut area) = ((usize::MAX, 0), (usize::MAX, 0), 0.0);
        for (i, p) in px.chunks_exact(4).enumerate() {
            area += f32::from(p[3]) / 255.0;
            if p[3] > 9 {
                let (x, y) = (i % size, i / size);
                cols = (cols.0.min(x), cols.1.max(x + 1));
                rows = (rows.0.min(y), rows.1.max(y + 1));
            }
        }
        (cols, rows, area)
    }

    #[test]
    fn the_alarm_outline_is_inked_where_the_night_clock_mock_inks_it_and_is_not_the_cards_alarm() {
        // The mock-up draws it in a 36 px square (24 units of 1.5 px) from a ring of radius 7 at (12, 13.5), a hand and two bell
        // strokes, 2 units wide with round caps: its shot (`night-clock/shots/01-b-rest.png`) has ink in columns 4 to 31 and rows 3 to
        // 32 of the square, and 288.8 pixels of coverage in all.
        let px = render(Icon::AlarmOutline, 36);
        let (cols, rows, area) = ink(&px, 36);
        assert_eq!((cols, rows), ((4, 32), (3, 33)));
        assert!(
            (area - 288.8).abs() < 288.8 * 0.04,
            "{area} pixels of coverage"
        );
        // The ring is open at its middle, the hand is ink, and the card's alarm is another picture.
        let on = |x: f32, y: f32| px[((y * 1.5) as usize * 36 + (x * 1.5) as usize) * 4 + 3] > 127;
        assert!(
            !on(9.0, 15.5) && on(12.0, 11.5),
            "the face is open and the hand is in it"
        );
        assert!(
            on(12.0, 6.5) && on(12.0, 20.5),
            "the ring's top and its foot"
        );
        assert_ne!(px, render(Icon::Alarm, 36));
    }
}

#[cfg(test)]
mod presence_icon_tests {
    use super::*;

    const NEW: [Icon; 7] = [
        Icon::Presence,
        Icon::Person,
        Icon::Server,
        Icon::Cog,
        Icon::Video,
        Icon::Sendcam,
        Icon::SendcamOff,
    ];

    fn render(icon: Icon, size: u32) -> Vec<u8> {
        let mut px = vec![0u8; (size * size * 4) as usize];
        icon.draw(&mut px, size, Rgba(255, 255, 255, 255));
        px
    }

    /// Whether the pixel at the grid point (`x`, `y`) of a 48-pixel render is inked.
    fn on(px: &[u8], x: f32, y: f32) -> bool {
        let (cx, cy) = ((x * 2.0) as usize, (y * 2.0) as usize);
        px[(cy * 48 + cx) * 4 + 3] > 127
    }

    #[test]
    fn the_four_glyphs_of_the_presence_page_are_not_alike_at_any_size_the_page_draws_them() {
        for size in [22u32, 24, 28, 32] {
            let drawn: Vec<Vec<u8>> = NEW.iter().map(|icon| render(*icon, size)).collect();
            for (i, a) in drawn.iter().enumerate() {
                for (j, b) in drawn.iter().enumerate().skip(i + 1) {
                    assert_ne!(a, b, "{:?} and {:?} look alike at {size}", NEW[i], NEW[j]);
                }
            }
        }
    }

    #[test]
    fn a_person_is_a_ring_for_a_head_over_shoulders_and_presence_adds_an_arc_to_either_side_of_it()
    {
        let person = render(Icon::Person, 48);
        // The head is a ring (open at its middle), the shoulders a line whose crown is at 14 and whose ends are at 5 and 19 on 20.
        assert!(on(&person, 12.0, 4.4) && !on(&person, 12.0, 8.0));
        assert!(on(&person, 12.0, 14.0) && on(&person, 5.0, 20.0) && on(&person, 19.0, 20.0));
        assert!(
            !on(&person, 12.0, 17.0),
            "the shoulders are a line, not a body"
        );
        assert!(
            !on(&person, 4.84, 4.84) && !on(&person, 19.16, 4.84),
            "the plain person has no arcs"
        );
        let presence = render(Icon::Presence, 48);
        for (x, y) in [(4.84, 4.84), (19.16, 4.84), (12.0, 4.4), (12.0, 14.0)] {
            assert!(on(&presence, x, y), "ink at ({x}, {y})");
        }
        assert!(!on(&presence, 12.0, 8.0), "and the head is still a ring");
    }

    #[test]
    fn a_server_is_two_slabs_with_a_light_each_and_a_cog_is_a_small_ring_with_eight_rays() {
        let server = render(Icon::Server, 48);
        // The slabs' edges, the room between them, and a light at the left of each.
        assert!(on(&server, 12.0, 4.5) && on(&server, 12.0, 10.5) && on(&server, 12.0, 13.5));
        assert!(!on(&server, 12.0, 12.0), "clear between the slabs");
        assert!(
            on(&server, 7.5, 7.5) && on(&server, 7.5, 16.5),
            "the lights"
        );
        assert!(!on(&server, 12.0, 7.5), "a slab is a frame");
        let cog = render(Icon::Cog, 48);
        assert!(
            on(&cog, 12.0, 9.0) && !on(&cog, 12.0, 12.0),
            "the ring of 3"
        );
        // A ray straight up and one on each diagonal, with a gap between the ring and the rays.
        assert!(on(&cog, 12.0, 4.5) && on(&cog, 6.65, 6.65) && on(&cog, 17.35, 17.35));
        assert!(!on(&cog, 12.0, 7.5), "the rays start at 6 from the middle");
    }
}

#[cfg(test)]
mod video_icon_tests {
    use super::*;

    fn render(icon: Icon) -> Vec<u8> {
        let mut px = vec![0u8; 48 * 48 * 4];
        icon.draw(&mut px, 48, Rgba(255, 255, 255, 255));
        px
    }

    fn on(px: &[u8], x: f32, y: f32) -> bool {
        px[(((y * 2.0) as usize) * 48 + (x * 2.0) as usize) * 4 + 3] > 127
    }

    #[test]
    fn the_presence_cameras_have_an_outline_wedge_where_the_toolkits_camera_has_a_solid_one_and_covered_is_struck_through()
     {
        let (video, sendcam, off, solid) = (
            render(Icon::Video),
            render(Icon::Sendcam),
            render(Icon::SendcamOff),
            render(Icon::Camera),
        );
        // The middle of the wedge: clear in the mock's outline, ink in the toolkit's filled one.
        assert!(!on(&video, 19.0, 12.0) && !on(&sendcam, 18.0, 12.0) && on(&solid, 19.0, 12.0));
        // The wedge's far side and its corners are ink, and so are the body's top and bottom edges.
        assert!(on(&video, 21.0, 12.0) && on(&sendcam, 20.0, 12.0));
        assert!(on(&video, 9.0, 6.0) && on(&video, 9.0, 18.0));
        assert!(on(&sendcam, 9.0, 7.0) && on(&sendcam, 9.0, 17.0));
        // `camera` has a body of 12 by 12 and `sendcam` one of 12 by 10.
        assert!(on(&video, 9.0, 5.6) && !on(&sendcam, 9.0, 5.6));
        // Covered is the sendcam with its stroke across, and nothing else.
        assert!(on(&off, 12.0, 12.0) && !on(&sendcam, 12.0, 12.0));
        assert_ne!(sendcam, off);
    }
}
