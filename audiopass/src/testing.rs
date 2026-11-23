//! A scripted TV: LG's sound card, audiooutputd and the processes around the route, simulated closely enough to drive the route's
//! arming, connect sequence, monitoring and restoration without a TV. Built for this crate's tests and, behind the `testing`
//! feature, for the crates that drive the route.
//!
//! The behaviours that decide whether the route works are the ones modelled: the kernel reports `Pass-Through` only while a
//! converter has been configured and the eARC words are un-mixed; ADEC1 connects muted; and audiooutputd forwards a mute call to
//! the kernel only when it differs from what audiooutputd believes, so a lone unmute after the connect does nothing.

use crate::host::{Host, Stuck, WriteError};
use crate::route::{AREN, BOOT_ID, GRAPH, INSTANCES};
use crate::sndout::{ADEC, SNDOUT, Words};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::Mutex;

/// The renderer's lipsync stage, as the proven window saw it.
pub const TAIL: &str = "0424000e";
/// The converter feeding that stage.
pub const CONVERTER: &str = "0427000d";
/// The player's process.
pub const PID: u32 = 4242;
pub const PLAYER_START: u64 = 123_456;

pub struct Sim {
    pub now: u64,
    pub files: BTreeMap<String, String>,
    pub hashes: BTreeMap<String, String>,
    pub dirs: BTreeSet<String>,
    /// pid to (start time, open descriptors).
    pub procs: BTreeMap<u32, (u64, Vec<String>)>,
    pub earc_on: bool,
    pub words: Words,
    /// Converters that have been sent the bypass command.
    pub configured: BTreeSet<String>,
    pub adec_connected: bool,
    pub adec_input: String,
    /// What the kernel reports for ADEC1.
    pub adec_mute: bool,
    /// What audiooutputd believes about ADEC1's mute.
    pub belief_muted: bool,
    pub stuck: Vec<Stuck>,
    pub status_ok: bool,
    pub connect_ok: bool,
    pub disconnect_ok: bool,
    pub words_error: Option<WriteError>,
    /// The graph shows the converter chain from this time on.
    pub graph_at: u64,
    pub chain_graph: String,
    pub idle_graph: String,
    /// Every mutation and luna call, with the time it happened.
    pub log: Vec<(u64, String)>,
    /// How often `sndout` was read.
    pub sndout_reads: u32,
}

pub fn graph_with(tail: &str, converter: &str) -> String {
    format!(
        " queue_041802c0_000000001453d6e5_out0 -> srcbin_04270174_0000000007f205f2_in0;\n \
         srcbin_04270174_00000000271faa81_out0 -> sola_04250174_00000000f882c51d_in0;\n \
         sola_04250174_000000003e35966d_out0 -> lipsync_042402d4_0000000021e47c8b_in0;\n \
         rts_0426001b_00000000316e9257_out0 -> srcbin_{converter}_00000000ae6884cf_in0;\n \
         srcbin_{converter}_000000008b38a937_out0 -> lipsync_{tail}_00000000117c1e96_in0;\n \
         gluepr_041002e6_00000000e2dc17e9_out0 -> rts_0426001b_00000000da81be92_in0;\n"
    )
}

impl Sim {
    fn render_sndout(&self) -> String {
        let codec = if self.words.mix == 0 && !self.configured.is_empty() {
            "Pass-Through"
        } else {
            "PCM"
        };
        let w = self.words;
        // The [ARC] section's own words are the decoys the parser must not read as eARC's.
        format!(
            "[ARC]\nOutputCodec={codec}\nOutputChannel=0\nSet_type=1\nCodec_type=1\nMix_option=1\nUnderflow=51314\n\
             [EARC]\nEArcOnOff={}\nSet_type={}\nCodec_type={}\nChannel_num={}\nSample_rate={}\nMix_option={}\nUnderflow=0\n\
             [Misc]\nConnectedAdec={}\nConnectedAmixer=5,6,7\nMainAudioOutput=ADEC1\nConnectedMixerPort=0x0\n",
            if self.earc_on { "On" } else { "Off" },
            w.set_type,
            w.codec,
            w.channels,
            w.rate,
            w.mix,
            if self.adec_connected { "1" } else { "" },
        )
    }

    fn render_adec(&self) -> String {
        format!(
            "Open=1 Connect={} Start=0 Gain=0x800000 Mute={}\n",
            self.adec_input,
            u8::from(self.adec_mute)
        )
    }

    fn note(&mut self, entry: impl Into<String>) {
        self.log.push((self.now, entry.into()));
    }
}

pub struct FakeHost {
    sim: Mutex<Sim>,
}

impl FakeHost {
    /// The C4 as the proven window found it, with a player holding the renderer and the chain already in LG's graph.
    pub fn c4() -> Self {
        let mut files = BTreeMap::new();
        files.insert(
            "/etc/starfish-release".into(),
            "Rockhopper release 10.2.1-5202 (ponytail-paparoa)\nbuild\n".into(),
        );
        files.insert(
            "/proc/sys/kernel/osrelease".into(),
            "5.4.268-329.ptl4tv.2\n".into(),
        );
        files.insert(BOOT_ID.into(), "boot-1\n".into());
        let mut hashes = BTreeMap::new();
        hashes.insert(
            "/usr/lib/gstreamer-1.0/libgstlgaudiosink.so".into(),
            "61c269de790c0872b96436c555f7807ef2357459b9969262b1b775939cacc9d2".into(),
        );
        hashes.insert(
            "/usr/lib/libpf-1.0.so.1".into(),
            "e8a966b26476f647bfcebf02b08404fc52f0198c52195e37354baa3d712fc36f".into(),
        );
        let mut procs = BTreeMap::new();
        procs.insert(
            PID,
            (PLAYER_START, vec!["/dev/null".to_owned(), AREN.to_owned()]),
        );
        Self {
            sim: Mutex::new(Sim {
                now: 100_000,
                files,
                hashes,
                dirs: BTreeSet::from([format!("{INSTANCES}/{CONVERTER}")]),
                procs,
                earc_on: true,
                words: Words {
                    set_type: 2,
                    codec: 1,
                    channels: 0,
                    rate: 0,
                    mix: 1,
                },
                configured: BTreeSet::new(),
                adec_connected: false,
                adec_input: "HDMI_PORT2".into(),
                adec_mute: true,
                belief_muted: false,
                stuck: Vec::new(),
                status_ok: true,
                connect_ok: true,
                disconnect_ok: true,
                words_error: None,
                graph_at: 0,
                chain_graph: graph_with(TAIL, CONVERTER),
                idle_graph: graph_with("04240000", "04270000")
                    .lines()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("\n"),
                log: Vec::new(),
                sndout_reads: 0,
            }),
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut Sim) -> R) -> R {
        f(&mut self.sim.lock().unwrap())
    }

    /// The names of everything that was done, in order.
    pub fn log(&self) -> Vec<String> {
        self.with(|sim| sim.log.iter().map(|(_, entry)| entry.clone()).collect())
    }

    /// When the first (or `last`) entry equal to `entry` happened.
    pub fn at(&self, entry: &str) -> Option<u64> {
        self.with(|sim| sim.log.iter().find(|(_, e)| e == entry).map(|(at, _)| *at))
    }

    pub fn now(&self) -> u64 {
        self.with(|sim| sim.now)
    }
}

fn not_found() -> io::Error {
    io::ErrorKind::NotFound.into()
}

impl Host for FakeHost {
    fn read(&self, path: &str) -> io::Result<String> {
        let mut sim = self.sim.lock().unwrap();
        match path {
            SNDOUT => {
                sim.sndout_reads += 1;
                Ok(sim.render_sndout())
            }
            ADEC => Ok(sim.render_adec()),
            GRAPH => Ok(if sim.now >= sim.graph_at {
                sim.chain_graph.clone()
            } else {
                sim.idle_graph.clone()
            }),
            _ => sim.files.get(path).cloned().ok_or_else(not_found),
        }
    }

    fn is_dir(&self, path: &str) -> bool {
        self.with(|sim| sim.dirs.contains(path))
    }

    fn write(&self, path: &str, _text: &str) -> io::Result<()> {
        self.with(|sim| {
            let converter = path
                .strip_prefix(INSTANCES)
                .and_then(|rest| rest.strip_prefix('/'))
                .and_then(|rest| rest.strip_suffix("/cmd"))
                .ok_or_else(not_found)?;
            if !sim.dirs.contains(&format!("{INSTANCES}/{converter}")) {
                return Err(not_found());
            }
            sim.configured.insert(converter.to_owned());
            sim.note(format!("cmd {converter}"));
            Ok(())
        })
    }

    fn save(&self, path: &str, text: &str) -> io::Result<()> {
        self.with(|sim| {
            sim.files.insert(path.to_owned(), text.to_owned());
            sim.note("save");
            Ok(())
        })
    }

    fn remove(&self, path: &str) -> io::Result<()> {
        self.with(|sim| {
            if sim.files.remove(path).is_some() {
                sim.note("remove");
            }
            Ok(())
        })
    }

    fn sha256(&self, path: &str) -> io::Result<String> {
        self.with(|sim| sim.hashes.get(path).cloned().ok_or_else(not_found))
    }

    fn fd_targets(&self, pid: u32) -> io::Result<Vec<String>> {
        self.with(|sim| {
            sim.procs
                .get(&pid)
                .map(|(_, fds)| fds.clone())
                .ok_or_else(not_found)
        })
    }

    fn start_time(&self, pid: u32) -> Option<u64> {
        self.with(|sim| sim.procs.get(&pid).map(|(start, _)| *start))
    }

    fn set_earc_words(&self, words: Words) -> Result<(), WriteError> {
        self.with(|sim| {
            if let Some(error) = sim.words_error.clone() {
                return Err(error);
            }
            sim.words = words;
            sim.note(format!("words {words}"));
            Ok(())
        })
    }

    fn luna(&self, uri: &str, payload: &str) -> Option<String> {
        const OK: &str = r#"{"returnValue":true}"#;
        const NO: &str = r#"{"returnValue":false,"errorText":"Audio not connected"}"#;
        self.with(|sim| {
            let verb = uri.rsplit('/').next().unwrap_or_default();
            match verb {
                "getStatus" => sim
                    .status_ok
                    .then(|| r#"{"returnValue":true,"audio":[]}"#.to_owned()),
                "connect" => {
                    sim.note("luna connect");
                    if !sim.connect_ok {
                        return Some(NO.into());
                    }
                    sim.adec_connected = true;
                    sim.belief_muted = false;
                    Some(OK.into())
                }
                "disconnect" => {
                    sim.note("luna disconnect");
                    if !sim.disconnect_ok {
                        return Some(NO.into());
                    }
                    sim.adec_connected = false;
                    Some(OK.into())
                }
                "mute" => {
                    let mute = payload.contains(r#""mute":true"#);
                    sim.note(format!("luna mute {mute}"));
                    if !sim.adec_connected {
                        return Some(NO.into());
                    }
                    if mute != sim.belief_muted {
                        sim.belief_muted = mute;
                        sim.adec_mute = mute;
                    }
                    Some(OK.into())
                }
                _ => None,
            }
        })
    }

    fn stuck_tasks(&self) -> Vec<Stuck> {
        self.with(|sim| sim.stuck.clone())
    }

    fn now_ms(&self) -> u64 {
        self.now()
    }

    fn sleep_ms(&self, ms: u64) {
        self.with(|sim| sim.now += ms);
    }
}
