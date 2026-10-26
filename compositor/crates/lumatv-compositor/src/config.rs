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
    pub starfish: bool,
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
    /// Exercise the optional system-layer renderer without replacing any TV surfaces.
    pub ui_demo: bool,
    /// LumaTV's data directory (`/var/lib/lumatv` on the set): the theme the remote's app buttons are read from (`theme/theme.json`) and
    /// the key codes taught to the routing (`compositor/keys.conf`).
    pub data_dir: PathBuf,
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
            starfish: false,
            log_protocol: false,
            width: crate::OUTPUT_WIDTH,
            height: crate::OUTPUT_HEIGHT,
            panel: (3840, 2160),
            tick_ns: 16_666_667,
            control: true,
            exit_on_disconnect: false,
            input: false,
            ui_demo: false,
            data_dir: PathBuf::from("/var/lib/lumatv"),
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
  --starfish             also advertise wl_starfish_pointer/output (WebAppMgr binds them on LG's compositor)
  --log-protocol         log every request
  --output WxH           output size (default 1920x1080)
  --panel WxH            the panel's size, the space of videooutputd's display windows (default 3840x2160)
  --tick-hz HZ           frame tick (default 60)
  --no-control           no control socket
  --input                read /dev/input/event* and route keys to the focused window (C4)
  --data-dir DIR         LumaTV's data directory (default /var/lib/lumatv): theme/theme.json holds Settings > Remote's app buttons,
                         compositor/keys.conf the key codes the routing was taught (`NAME CODE` per line)
  --ui-demo              draw the system-layer diagnostic (requires feature ui and gl|kms); LUMATV_UI_DEMO_HZ (ticker rate,
                         default 60) and LUMATV_UI_DEMO_VOLUME_S (the volume bar up 2 s every N s) pace it
  ctl CMD                talk to a running compositor's control socket (status, windows, capture PATH, quit)
";

impl Config {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Config, String> {
        let mut c = Config::default();
        let mut it = args.into_iter();
        let mut renderer_set = false;
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
                "--starfish" => c.starfish = true,
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
                "--data-dir" => c.data_dir = PathBuf::from(value()?),
                "--ui-demo" => c.ui_demo = true,
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
    }
}
