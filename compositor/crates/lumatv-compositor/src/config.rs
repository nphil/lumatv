//! Command line of `lumatv-compositor`.

use std::path::PathBuf;

use crate::render::egl::Platform;
use crate::render::gl::BufferPath;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererKind {
    Software,
    Gl,
    /// The GL renderer scanning out through KMS as DRM master (C3; only in a maintenance window until C4).
    Kms,
}

/// A self-check of what reached the output, for the scripted TV runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    None,
    /// The X2 client clears frame k to R = 25k, G = 0x80, B = 255 - 25k, A = 255; every drawn frame's centre pixel
    /// must match the frame count.
    X2,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub runtime_dir: PathBuf,
    pub socket: String,
    pub renderer: RendererKind,
    pub driver: Option<String>,
    pub card: String,
    pub platform: Platform,
    pub buffers: BufferPath,
    pub seconds: Option<u64>,
    pub frames: Option<u64>,
    pub capture_dir: Option<PathBuf>,
    pub capture_every: u64,
    pub check: Check,
    pub log_protocol: bool,
    pub width: u32,
    pub height: u32,
    /// The panel's pixels: videooutputd places video in them, not in the output's (a 4K panel under a 1080p output).
    pub panel: (u32, u32),
    pub tick_ns: u64,
    pub control: bool,
    pub exit_on_disconnect: bool,
    /// Read the set's input devices and route keys (C4). Off by default: beside LG's compositor it would only
    /// feed the test clients.
    pub input: bool,
    /// Where `--input` finds its `event*` devices.
    pub input_dir: PathBuf,
    /// Answer lginput2's Input Manager handshake over LG's message queue (`im_queue`) while `--input` is on; `--no-im-queue` leaves the
    /// queue to LG's compositor.
    pub im_queue: bool,
    /// Exercise the optional system-layer renderer without replacing any TV surfaces.
    pub ui_demo: bool,
    /// The kinds the Rust shell (`lumatv-ui`, `shell_rust.rs`) answers instead of the QML client: `osd` (the volume and mute display),
    /// `toast` (LG's toast feed); `ctl shell rust +kind -kind` changes it at run time. Empty: the Rust shell is not started.
    pub shell_rust: Vec<String>,
    /// LumaTV's data directory (`/var/lib/lumatv` on the set): the theme the remote's app buttons are read from (`theme/theme.json`) and
    /// the key codes taught to the routing (`compositor/keys.conf`).
    pub data_dir: PathBuf,
    /// SCHED_FIFO priority of the frame loop's thread (`sched`); 0 leaves it time-shared.
    pub rt_priority: i32,
    /// SCHED_FIFO priority of the Mali blob's submission threads (`sched::mali_threads`); 0 leaves them time-shared.
    pub rt_mali: i32,
    /// The shell client (`shell.rs`): `Auto` runs it live (`--input`) when the bundle beside this executable has `shell/Shell.qml`,
    /// `Off` never, `At(path)` runs that root QML.
    pub shell: ShellMode,
    /// The loopback port of the UI channel the shell client speaks to.
    pub shell_port: u16,
    /// Whether what the compositor touches is pinned in RAM (`mem::lock`): `auto` locks it when the set's limit and headroom allow.
    pub mlock: crate::mem::Mlock,
    /// Where the trace ring is kept (`ring`): by default `lumatv-trace.ring` in the runtime directory, where the supervisor finds it.
    pub ring: RingSetting,
    /// The tick's period once the screen is at rest and nothing has happened for a while (`ticker`), ns; 0 keeps the panel's rate.
    /// `ctl idle MS` changes it while running.
    pub idle_tick_ns: u64,
    /// The refresh the output is asked to run at, Hz: 60 (the set's OSD vblank as LG leaves it) or 120 (`refresh`: the test mode, the
    /// OSD's vblank divider at 1). `ctl refresh HZ` changes it while running.
    pub refresh_hz: u32,
    /// Direct scan-out of a full-screen client buffer (`scanout`): the switch (off by default), how the client's rendering is waited for, the
    /// modifiers tried beyond the plane's list. `ctl scanout` switches it while running.
    pub scanout: crate::scanout::Policy,
    /// The off-screen proof of route A: record and PRIME-import every driver-served buffer's descriptor, without taking the screen
    /// (`scanout::probe`).
    pub scanout_probe: bool,
    /// More environment for the shell client, so that its window buffers are ones the plane can take (`--scanout-shell-env`).
    pub scanout_shell_env: Vec<(String, String)>,
    /// Where the Magic Remote's pointer is drawn: into the frame (`gpu`, the default) or by the cursor plane (`plane`, `pointer`). `ctl pointer
    /// plane|gpu` switches it while running.
    pub pointer: crate::pointer::Mode,
    /// How the cursor plane reads its image's alpha (`--pointer-alpha`).
    pub pointer_alpha: crate::pointer::Alpha,
    /// The primary plane's pages carry alpha (AR24) instead of ignoring it (XR24), which the display driver reads as a constant alpha of 0xff:
    /// a hole cleared to transparent is opaque black on the panel with XR24 (`hw-planes-result.md`, "The holes"). Implied by `--vwin` and
    /// `--afbc on`.
    pub page_alpha: bool,
    /// The scan-out pages as AFBC (`afbc`): `off`, `probe` (query only) or `on`.
    pub afbc: crate::afbc::Mode,
    /// The hardware video windows beyond the app's own video (`vwin`): `off`, `idle` or `apps`, and how many windows the display has.
    pub vwin: crate::vwin::Policy,
}

/// Where the trace ring lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RingSetting {
    Auto,
    Off,
    At(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellMode {
    Auto,
    Off,
    At(PathBuf),
}

impl Default for Config {
    fn default() -> Self {
        Config {
            runtime_dir: PathBuf::from("/tmp/lumatv"),
            socket: "wayland-lumatv".into(),
            renderer: RendererKind::Software,
            driver: None,
            card: "/dev/dri/card0".into(),
            platform: Platform::GbmNull,
            buffers: BufferPath::Driver,
            seconds: None,
            frames: None,
            capture_dir: None,
            capture_every: 1,
            check: Check::None,
            log_protocol: false,
            width: crate::OUTPUT_WIDTH,
            height: crate::OUTPUT_HEIGHT,
            panel: (3840, 2160),
            tick_ns: 16_666_667,
            control: true,
            exit_on_disconnect: false,
            input: false,
            input_dir: PathBuf::from("/dev/input"),
            im_queue: true,
            ui_demo: false,
            shell_rust: Vec::new(),
            data_dir: PathBuf::from("/var/lib/lumatv"),
            rt_priority: 20,
            rt_mali: 19,
            shell: ShellMode::Auto,
            shell_port: 18750,
            mlock: crate::mem::Mlock::Auto,
            ring: RingSetting::Auto,
            idle_tick_ns: crate::ticker::IDLE_TICK_NS,
            refresh_hz: crate::refresh::DEFAULT_HZ,
            scanout: crate::scanout::Policy::default(),
            scanout_probe: false,
            scanout_shell_env: Vec::new(),
            pointer: crate::pointer::Mode::default(),
            pointer_alpha: crate::pointer::Alpha::default(),
            page_alpha: false,
            afbc: crate::afbc::Mode::default(),
            vwin: crate::vwin::Policy::default(),
        }
    }
}

pub const USAGE: &str = "lumatv-compositor [options]
  --runtime-dir DIR      XDG_RUNTIME_DIR for the socket (default /tmp/lumatv)
  --socket NAME          WAYLAND_DISPLAY name (default wayland-lumatv)
  --renderer software|gl|kms (default software; gl when --driver is given; kms = gl + DRM master scan-out)
  --card PATH            the DRM device for --renderer kms (default /dev/dri/card0)
  --driver PATH          the 64-bit Mali blob to dlopen for the GL renderer
  --egl-platform gbm|surfaceless  how the EGL display is made (gbm with a null device for the blob; surfaceless for Mesa)
  --buffers driver|own   who serves mali_buffer_sharing: the driver (eglBindWaylandDisplayWL) or this compositor
  --seconds N            exit after N seconds
  --frames N             exit after N frames with content were presented
  --exit-on-disconnect   exit when the last client leaves
  --capture-dir DIR      write every K-th drawn frame as PNG (frame-NNNN.png) into DIR
  --capture-every K      (default 1)
  --check x2             verify each drawn frame against the X2 client's colour pattern
  --log-protocol         log every request
  --output WxH           output size (default 1920x1080)
  --panel WxH            the panel's size, the space of videooutputd's display windows (default 3840x2160)
  --tick-hz HZ           frame tick (default 60)
  --no-control           no control socket
  --input                read /dev/input/event* and route keys to the focused window (C4)
  --input-dir DIR        read the event* devices of DIR instead of /dev/input
  --no-im-queue          leave LG's Input Manager queue (/IM_MQ_WEBOS) alone: with --input the compositor answers lginput2's
                         handshake on it, as LG's compositor did, so a reconnected Magic Remote works at once
  --data-dir DIR         LumaTV's data directory (default /var/lib/lumatv): theme/theme.json holds Settings > Remote's app buttons,
                         compositor/keys.conf the key codes the routing was taught (`NAME CODE` per line)
  --ui-demo              draw the system-layer diagnostic (requires feature ui and gl|kms); LUMATV_UI_DEMO_HZ (ticker rate,
                         default 60) and LUMATV_UI_DEMO_VOLUME_S (the volume bar up 2 s every N s) pace it
  --shell-rust LIST      the kinds the Rust shell (bin/lumatv-ui, drawn by the engine inside the compositor) answers instead of
                         the QML client: osd, toast (comma-separated; requires feature ui and gl|kms); `ctl shell rust +osd -toast`
                         changes the list at run time. Default: none, the QML shell answers everything
  --rt-priority N        SCHED_FIFO priority of the frame loop's thread, 1-99 (default 20; 0 keeps it time-shared)
  --mlock off|auto       pin the pages the compositor touches in RAM, so memory pressure from the apps in front never evicts or swaps them
                         (default auto: mlockall on fault, when RLIMIT_MEMLOCK or CAP_IPC_LOCK and the free memory allow)
  --ring off|auto|PATH   the trace ring: the last seconds of what the loop did, mapped from a file that outlives a crash (default auto:
                         lumatv-trace.ring in the runtime directory, which the supervisor copies into the crash bundle)
  --idle-tick-ms N       the tick's period while the screen is at rest and nothing has happened for 250 ms (default 100; 0 keeps the panel's
                         rate, 60 wake-ups a second doing nothing)
  --refresh 60|120       the refresh the output runs at (default 60): 120 is the test mode (docs/research/ui120-result.md), the OSD
                         vblank divider set to 1 so page flips land at every panel refresh; `ctl refresh 60|120` switches it while running
  --rt-mali N            SCHED_FIFO priority of the Mali blob's submission threads, 1-99 (default 19; 0 keeps them time-shared)
  --shell off|auto|PATH  the shell client (LumaTV's QML on LG's Qt through /usr/bin/qml-runner): auto (default) runs it with
                         --input when <bundle>/shell/Shell.qml sits beside this executable's bin/; PATH names another root QML
  --shell-port N         the loopback port of the UI channel (default 18750)
  --scanout off|direct   direct scan-out of a full-screen client buffer, so that the compositor does not copy it (default off; needs --renderer kms):
                         `ctl scanout [off|direct]` reads the counters and switches it while running (docs/research/scanout-research.md)
  --scanout-acquire sample|none  how the client's rendering is waited for before the plane reads its buffer (default sample: the GL side samples
                         the buffer and the commit carries that job's fence; none trusts the commit, for a measurement)
  --scanout-trust 0xMOD  try this modifier although the primary plane does not list it (repeatable): the experiment that shows whether the
                         OSD decodes a client's layout; never for production
  --scanout-offset       let a direct frame carry the OLED care's orbit as the primary plane's position (CRTC_X/Y), so a shifted Home stays
                         direct; shifts right or down only (default off; `ctl scanout offset on|off` switches it; docs/research/layer-deep-result.md)
  --scanout-probe        the off-screen proof: record every driver-served buffer's descriptor and PRIME-import it into the display driver
                         without taking master or showing anything (tools/tv-scanout-proof.sh; needs --buffers driver)
  --scanout-shell-env K=V  one more environment variable for the shell client (repeatable), the way to ask its window for a layout the plane takes
  --pointer gpu|plane    where the Magic Remote's pointer is drawn (default gpu: a sprite in the frame; plane: the cursor plane, which moves it with no frame
                         drawn; needs --renderer kms). `ctl pointer [plane|gpu|X Y]` reads the counters, switches it or moves it (docs/research/hw-planes-result.md)
  --pointer-alpha premultiplied|straight  how the cursor plane reads its image's alpha (default premultiplied, as the frame is)
  --page-alpha on|off    the primary plane's pages carry alpha (AR24, default off: XR24, whose holes the display driver shows opaque black); implied by --vwin and
                         --afbc on. Starts with the compositor: the pages are made once
  --afbc off|probe|on    the scan-out pages as ARM AFBC (modifier 0x0800000000000062, AB24; default off; needs --renderer kms): probe only asks the GPU driver and
                         the plane what they list (`ctl afbc`), on makes the pages AFBC and falls back to linear ones with the reason when the driver refuses
  --vwin off|idle|apps   the display's spare video windows for LumaTV's own video (default off): idle gives the camera pop-up, the Cameras live view and a
                         picture-in-picture a window while no app has a video element, apps also next to an app's video (the sub window, 1080p30);
                         `ctl vwin [off|idle|apps|claim|release|...]` and POST /v1/vwin on the UI channel (docs/research/hw-planes-result.md)
  --vwin-slots N         how many video windows the display is taken to have (default 2: main and sub; the O22 source has NUM_WIN_MAX 2, win2 and win3 are proc
                         slots only)
  ctl CMD                talk to a running compositor's control socket (status, windows, capture PATH, quit)
";

impl Config {
    /// The file the trace ring is mapped from; `None` when it is off.
    pub fn ring_path(&self) -> Option<PathBuf> {
        match &self.ring {
            RingSetting::Off => None,
            RingSetting::Auto => Some(self.runtime_dir.join(crate::ring::FILE_NAME)),
            RingSetting::At(p) => Some(p.clone()),
        }
    }

    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Config, String> {
        let mut c = Config::default();
        let mut it = args.into_iter();
        let mut renderer_set = false;
        let mut page_alpha_set: Option<bool> = None;
        while let Some(a) = it.next() {
            let mut value = || it.next().ok_or_else(|| format!("{a} needs a value"));
            match a.as_str() {
                "--runtime-dir" => c.runtime_dir = PathBuf::from(value()?),
                "--socket" => c.socket = value()?,
                "--renderer" => {
                    renderer_set = true;
                    c.renderer = match value()?.as_str() {
                        "software" => RendererKind::Software,
                        "gl" => RendererKind::Gl,
                        "kms" => RendererKind::Kms,
                        o => return Err(format!("unknown renderer {o}")),
                    }
                }
                "--driver" => c.driver = Some(value()?),
                "--card" => c.card = value()?,
                "--egl-platform" => {
                    c.platform = match value()?.as_str() {
                        "gbm" => Platform::GbmNull,
                        "surfaceless" => Platform::Surfaceless,
                        o => return Err(format!("unknown EGL platform {o}")),
                    }
                }
                "--buffers" => {
                    c.buffers = match value()?.as_str() {
                        "driver" => BufferPath::Driver,
                        "own" => BufferPath::Own,
                        o => return Err(format!("unknown buffer path {o}")),
                    }
                }
                "--seconds" => c.seconds = Some(value()?.parse().map_err(|_| "--seconds")?),
                "--frames" => c.frames = Some(value()?.parse().map_err(|_| "--frames")?),
                "--exit-on-disconnect" => c.exit_on_disconnect = true,
                "--capture-dir" => c.capture_dir = Some(PathBuf::from(value()?)),
                "--capture-every" => c.capture_every = value()?.parse().map_err(|_| "--capture-every")?,
                "--check" => {
                    c.check = match value()?.as_str() {
                        "x2" => Check::X2,
                        "none" => Check::None,
                        o => return Err(format!("unknown check {o}")),
                    }
                }
                "--log-protocol" => c.log_protocol = true,
                "--output" => {
                    let v = value()?;
                    let (w, h) = v.split_once('x').ok_or("--output WxH")?;
                    c.width = w.parse().map_err(|_| "--output")?;
                    c.height = h.parse().map_err(|_| "--output")?;
                }
                "--panel" => {
                    let v = value()?;
                    let (w, h) = v.split_once('x').ok_or("--panel WxH")?;
                    c.panel = (w.parse().map_err(|_| "--panel")?, h.parse().map_err(|_| "--panel")?);
                }
                "--tick-hz" => {
                    let hz: f64 = value()?.parse().map_err(|_| "--tick-hz")?;
                    c.tick_ns = (1e9 / hz) as u64;
                }
                "--no-control" => c.control = false,
                "--input" => c.input = true,
                "--input-dir" => c.input_dir = PathBuf::from(value()?),
                "--no-im-queue" => c.im_queue = false,
                "--data-dir" => c.data_dir = PathBuf::from(value()?),
                "--ui-demo" => c.ui_demo = true,
                "--shell-rust" => c.shell_rust = crate::shell_rust::parse_kinds(&value()?),
                "--rt-priority" => {
                    c.rt_priority = value()?.parse().map_err(|_| "--rt-priority")?;
                    if !(0..=99).contains(&c.rt_priority) {
                        return Err("--rt-priority takes 0 to 99".into());
                    }
                }
                "--rt-mali" => {
                    c.rt_mali = value()?.parse().map_err(|_| "--rt-mali")?;
                    if !(0..=99).contains(&c.rt_mali) {
                        return Err("--rt-mali takes 0 to 99".into());
                    }
                }
                "--shell" => {
                    c.shell = match value()?.as_str() {
                        "off" => ShellMode::Off,
                        "auto" => ShellMode::Auto,
                        p => ShellMode::At(PathBuf::from(p)),
                    }
                }
                "--mlock" => {
                    c.mlock = match value()?.as_str() {
                        "off" => crate::mem::Mlock::Off,
                        "auto" => crate::mem::Mlock::Auto,
                        o => return Err(format!("--mlock takes off or auto, not {o}")),
                    }
                }
                "--ring" => {
                    c.ring = match value()?.as_str() {
                        "off" => RingSetting::Off,
                        "auto" => RingSetting::Auto,
                        p => RingSetting::At(PathBuf::from(p)),
                    }
                }
                "--idle-tick-ms" => c.idle_tick_ns = value()?.parse::<u64>().map_err(|_| "--idle-tick-ms")? * 1_000_000,
                "--refresh" => {
                    let hz: u32 = value()?.parse().map_err(|_| "--refresh")?;
                    if !crate::refresh::supported(hz) {
                        return Err("--refresh takes 60 or 120".into());
                    }
                    c.refresh_hz = hz;
                    c.tick_ns = crate::refresh::period_ns(hz);
                }
                "--shell-port" => c.shell_port = value()?.parse().map_err(|_| "--shell-port")?,
                "--scanout" => {
                    let v = value()?;
                    c.scanout.mode = crate::scanout::Mode::parse(&v).ok_or_else(|| format!("--scanout takes off or direct, not {v}"))?;
                }
                "--scanout-acquire" => {
                    let v = value()?;
                    c.scanout.acquire = crate::scanout::Acquire::parse(&v).ok_or_else(|| format!("--scanout-acquire takes sample or none, not {v}"))?;
                }
                "--scanout-trust" => {
                    let v = value()?;
                    let m = u64::from_str_radix(v.trim_start_matches("0x"), 16).map_err(|_| format!("--scanout-trust takes a hex modifier, not {v}"))?;
                    c.scanout.trust.push(m);
                }
                "--scanout-offset" => c.scanout.offset = true,
                "--scanout-probe" => c.scanout_probe = true,
                "--scanout-shell-env" => {
                    let v = value()?;
                    let (k, val) = v.split_once('=').filter(|(k, _)| !k.is_empty()).ok_or("--scanout-shell-env NAME=VALUE")?;
                    c.scanout_shell_env.push((k.to_string(), val.to_string()));
                }
                "--pointer" => {
                    let v = value()?;
                    c.pointer = crate::pointer::Mode::parse(&v).ok_or_else(|| format!("--pointer takes gpu or plane, not {v}"))?;
                }
                "--pointer-alpha" => {
                    let v = value()?;
                    c.pointer_alpha = crate::pointer::Alpha::parse(&v).ok_or_else(|| format!("--pointer-alpha takes premultiplied or straight, not {v}"))?;
                }
                "--page-alpha" => {
                    c.page_alpha = match value()?.as_str() {
                        "on" => true,
                        "off" => false,
                        o => return Err(format!("--page-alpha takes on or off, not {o}")),
                    };
                    page_alpha_set = Some(c.page_alpha);
                }
                "--afbc" => {
                    let v = value()?;
                    c.afbc = crate::afbc::Mode::parse(&v).ok_or_else(|| format!("--afbc takes off, probe or on, not {v}"))?;
                }
                "--vwin" => {
                    let v = value()?;
                    c.vwin.mode = crate::vwin::Mode::parse(&v).ok_or_else(|| format!("--vwin takes off, idle or apps, not {v}"))?;
                }
                "--vwin-slots" => {
                    let n: usize = value()?.parse().map_err(|_| "--vwin-slots takes a number")?;
                    if !(1..=crate::vwin::MAX_SLOTS).contains(&n) {
                        return Err(format!("--vwin-slots takes 1 to {}", crate::vwin::MAX_SLOTS));
                    }
                    c.vwin.slots = n;
                }
                "-h" | "--help" => return Err(USAGE.to_string()),
                o => return Err(format!("unknown option {o}\n{USAGE}")),
            }
        }
        if !renderer_set && c.driver.is_some() {
            c.renderer = RendererKind::Gl;
        }
        if matches!(c.renderer, RendererKind::Gl | RendererKind::Kms) && c.driver.is_none() {
            return Err("--renderer gl|kms needs --driver PATH".into());
        }
        if c.ui_demo && (!cfg!(feature = "ui") || c.renderer == RendererKind::Software) {
            return Err("--ui-demo requires a build with feature ui and --renderer gl|kms".into());
        }
        // Without the flag, the kinds `ctl shell rust` kept in the data directory (a flip from the workstation survives a restart).
        if c.shell_rust.is_empty() && cfg!(feature = "ui") && c.renderer != RendererKind::Software {
            c.shell_rust = crate::shell_rust::read_kinds(&c.data_dir);
        }
        if !c.shell_rust.is_empty() && (!cfg!(feature = "ui") || c.renderer == RendererKind::Software) {
            return Err("--shell-rust requires a build with feature ui and --renderer gl|kms".into());
        }
        if c.ui_demo && !c.shell_rust.is_empty() {
            return Err("--ui-demo and --shell-rust both want the engine; one or the other".into());
        }
        if c.scanout.mode == crate::scanout::Mode::Direct && c.renderer != RendererKind::Kms {
            return Err("--scanout direct needs --renderer kms (the plane it scans out on)".into());
        }
        if c.scanout_probe && c.renderer == RendererKind::Software {
            return Err("--scanout-probe needs --renderer gl|kms and --buffers driver (the driver's buffers are what it records)".into());
        }
        if c.pointer == crate::pointer::Mode::Plane && c.renderer != RendererKind::Kms {
            return Err("--pointer plane needs --renderer kms (the cursor plane it drives)".into());
        }
        if c.afbc != crate::afbc::Mode::Off && c.renderer != RendererKind::Kms {
            return Err("--afbc needs --renderer kms (the plane that scans the pages out)".into());
        }
        // The holes a video window needs and the AB24 pages of AFBC both carry alpha: they ask for it (`--page-alpha off` after them is refused,
        // since the combination shows black holes).
        if c.vwin.mode != crate::vwin::Mode::Off || c.afbc == crate::afbc::Mode::On {
            if page_alpha_set == Some(false) {
                return Err("--page-alpha off cannot go with --vwin or --afbc on: their holes and pages need alpha".into());
            }
            c.page_alpha = true;
        }
        if c.capture_every == 0 {
            c.capture_every = 1;
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_tv_run() {
        let c = Config::parse(
            ["--driver", "/tmp/m64/lib/libmali.so", "--buffers", "own", "--frames", "6", "--check", "x2", "--capture-dir", "/tmp/out"].map(String::from),
        )
        .unwrap();
        assert_eq!(c.renderer, RendererKind::Gl);
        assert_eq!(c.buffers, BufferPath::Own);
        assert_eq!(c.frames, Some(6));
        assert_eq!(c.check, Check::X2);
        assert!(Config::parse(["--renderer", "gl"].map(String::from)).is_err());
        let c = Config::parse(["--refresh", "120"].map(String::from)).unwrap();
        assert_eq!((c.refresh_hz, c.tick_ns), (120, 8_333_333));
        assert!(Config::parse(["--refresh", "90"].map(String::from)).is_err());
    }

    #[test]
    fn the_scanout_switch_is_off_unless_asked_and_needs_the_plane() {
        let c = Config::parse(["--driver", "/x/libmali.so"].map(String::from)).unwrap();
        assert_eq!(c.scanout, crate::scanout::Policy::default());
        assert!(c.scanout.mode == crate::scanout::Mode::Off && !c.scanout_probe && c.scanout_shell_env.is_empty());
        let c = Config::parse(
            [
                "--renderer",
                "kms",
                "--driver",
                "/x/libmali.so",
                "--scanout",
                "direct",
                "--scanout-acquire",
                "none",
                "--scanout-trust",
                "0x0800000000000151",
                "--scanout-shell-env",
                "WAYLAND_AFBC=0",
                "--scanout-offset",
            ]
            .map(String::from),
        )
        .unwrap();
        assert_eq!(c.scanout.mode, crate::scanout::Mode::Direct);
        assert_eq!(c.scanout.acquire, crate::scanout::Acquire::Trust);
        assert_eq!(c.scanout.trust, vec![0x0800_0000_0000_0151]);
        assert_eq!(c.scanout_shell_env, vec![("WAYLAND_AFBC".to_string(), "0".to_string())]);
        assert!(c.scanout.offset);
        let direct_without_the_plane = ["--renderer", "gl", "--driver", "/x/libmali.so", "--scanout", "direct"].map(String::from);
        assert!(Config::parse(direct_without_the_plane).unwrap_err().contains("--renderer kms"));
        assert!(Config::parse(["--scanout", "on"].map(String::from)).is_err());
        assert!(Config::parse(["--scanout-acquire", "fence"].map(String::from)).is_err());
        assert!(Config::parse(["--scanout-trust", "zz"].map(String::from)).is_err());
        assert!(Config::parse(["--scanout-shell-env", "NOEQUALS"].map(String::from)).is_err());
        assert!(Config::parse(["--scanout-shell-env", "=1"].map(String::from)).is_err());
        let probe = Config::parse(["--renderer", "gl", "--driver", "/x/libmali.so", "--buffers", "driver", "--scanout-probe"].map(String::from)).unwrap();
        assert!(probe.scanout_probe && probe.scanout.mode == crate::scanout::Mode::Off, "the proof shows nothing: the switch stays off");
        assert!(Config::parse(["--scanout-probe"].map(String::from)).is_err(), "no renderer, no driver buffers to record");
    }
}
