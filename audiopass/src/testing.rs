//! A scripted TV: LG's sound card, audiooutputd and the processes around the route, simulated closely enough to drive the route's
//! arming, connect sequence, monitoring and restoration without a TV. Built for this crate's tests and, behind the `testing`
//! feature, for the crates that drive the route.
//!
//! The behaviours that decide whether the route works are the ones modelled: the kernel reports `Pass-Through` only while a
//! converter has been configured and the eARC words are un-mixed; the granted ADEC connects with its kernel mute as it was;
//! audiooutputd forwards a mute call to the kernel only when it differs from what audiooutputd believes, so a lone unmute after
//! the connect does nothing, and refuses a mute call once the input is disconnected.

use crate::host::{Host, Stuck, WriteError};
use crate::route::{AREN, BOOT_ID, GRAPH, INSTANCES};
use crate::sndout::{SNDOUT, Words, adec_path};
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
/// The decoder input the proven player's renderer was granted.
pub const ADEC: u8 = 1;
/// The player's app, as SAM lists it.
pub const APP: &str = "com.beb.plxnative";
/// The uMS pipeline of the player's Load, and the PCM mix channel it booked: LG plays it through `AMIXER4`.
pub const PIPELINE: &str = "_f4rXIG4rerDUwJ";
pub const PORT: u8 = 4;

/// One uMS pipeline: its id, its app and the resources it holds, by name and index.
#[derive(Clone, Debug)]
pub struct Pipe {
    pub id: String,
    pub app: String,
    pub resources: Vec<(String, u8)>,
}

/// How the scripted TV or helper fails from some point on (`Sim::fault`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The helper is gone: nothing it does after this lands.
    Dead,
    /// Reads and writes of LG's driver files block until their deadline.
    Blocked,
    /// audiooutputd does not answer luna calls.
    Silent,
}

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
    /// The decoder input the player's renderer was granted: the only one the sim models.
    pub adec: u8,
    pub adec_connected: bool,
    pub adec_input: String,
    pub adec_start: bool,
    /// What the kernel reports for the granted ADEC's mute.
    pub adec_mute: bool,
    /// What audiooutputd believes about its mute.
    pub belief_muted: bool,
    /// Other decoder inputs LG has connected to the main output (an HDMI input or an app), by number.
    pub others: Vec<u8>,
    /// LG's mixer input for a stream (`AMIXER<n>`), connected in audiooutputd and the kernel.
    pub mixer: Option<u8>,
    /// audiooutputd lists the mixer input from this time on, which may be a moment after the kernel shows it.
    pub listed_at: u64,
    /// SAM's running apps, by id and process.
    pub apps: Vec<(String, u32)>,
    pub pipelines: Vec<Pipe>,
    /// Whether LG's HDMI input service answers, and how often it was asked.
    pub hdmi_ok: bool,
    pub hdmi_calls: u32,
    pub stuck: Vec<Stuck>,
    /// Whether writing the words wedges `extinput-integration` in the sound card's control lock, as it did on the TV.
    pub wedge_on_words: bool,
    pub status_ok: bool,
    pub connect_ok: bool,
    pub disconnect_ok: bool,
    /// Whether a converter takes its command.
    pub cmd_ok: bool,
    pub words_error: Option<WriteError>,
    /// The graph shows the converter chain from this time on.
    pub graph_at: u64,
    pub chain_graph: String,
    pub idle_graph: String,
    /// Every mutation and luna call, with the time it happened.
    pub log: Vec<(u64, String)>,
    /// How often `sndout` was read.
    pub sndout_reads: u32,
    /// How often luna was called, whatever it was asked.
    pub luna_calls: u32,
    /// What one luna call and one scan of the tasks cost on the fake clock.
    pub luna_ms: u64,
    pub scan_ms: u64,
    /// The stuck tasks are gone after this many scans: a wait that lasted a moment.
    pub stuck_scans: Option<u32>,
    pub fault: Option<Fault>,
    /// `fault` starts right after this entry is logged.
    pub trip: Option<(String, Fault)>,
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
             [Misc]\nConnectedAdec={}\nConnectedAmixer=5,6,7{}\nMainAudioOutput=ADEC1\nConnectedMixerPort=0x0\n",
            if self.earc_on { "On" } else { "Off" },
            w.set_type,
            w.codec,
            w.channels,
            w.rate,
            w.mix,
            self.connected_list(),
            self.mixer
                .map(|port| format!(",{port}"))
                .unwrap_or_default(),
        )
    }

    fn render_adec(&self) -> String {
        format!(
            "Open=1 Connect={} Start={} Gain=0x800000 Mute={}\n",
            self.adec_input,
            u8::from(self.adec_start),
            u8::from(self.adec_mute)
        )
    }

    /// The kernel's `ConnectedAdec` list.
    fn connected_list(&self) -> String {
        self.adec_connected
            .then_some(self.adec)
            .into_iter()
            .chain(self.others.iter().copied())
            .map(|adec| adec.to_string())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// audiooutputd's connection list, as `audio/getStatus` prints it.
    fn render_umi(&self) -> String {
        let entry = |source: String, muted: bool| {
            format!(r#"{{"source":"{source}","muted":{muted},"sink":"MAIN","outputMode":"null"}}"#)
        };
        let list: Vec<String> = self
            .mixer
            .filter(|_| self.now >= self.listed_at)
            .map(|port| entry(format!("AMIXER{port}"), false))
            .into_iter()
            .chain(
                self.adec_connected
                    .then(|| entry(format!("ADEC{}", self.adec), self.belief_muted)),
            )
            .chain(
                self.others
                    .iter()
                    .map(|adec| entry(format!("ADEC{adec}"), false)),
            )
            .collect();
        format!(r#"{{"returnValue":true,"audio":[{}]}}"#, list.join(","))
    }

    fn render_running(&self) -> String {
        let list: Vec<String> = self
            .apps
            .iter()
            .map(|(id, pid)| {
                format!(
                    r#"{{"id":"{id}","processid":"{pid}","webprocessid":"","appType":"native"}}"#
                )
            })
            .collect();
        format!(r#"{{"running":[{}],"returnValue":true}}"#, list.join(","))
    }

    fn render_pipelines(&self) -> String {
        if self.pipelines.is_empty() {
            return r#"{"errorCode":0,"returnValue":true,"errorText":"No Error","data":"pipeline empty"}"#.into();
        }
        let list: Vec<String> = self
            .pipelines
            .iter()
            .map(|pipe| {
                let held: Vec<String> = pipe
                    .resources
                    .iter()
                    .map(|(name, index)| format!(r#"{{"index":{index},"resource":"{name}"}}"#))
                    .collect();
                format!(
                    r#"{{"id":"{}","type":"media","currentAppId":"{}","resource":[{}]}}"#,
                    pipe.id,
                    pipe.app,
                    held.join(",")
                )
            })
            .collect();
        format!("[{}]", list.join(","))
    }

    fn note(&mut self, entry: impl Into<String>) {
        let entry = entry.into();
        match self.trip.take() {
            Some((at, fault)) if at == entry => self.fault = Some(fault),
            other => self.trip = other,
        }
        self.log.push((self.now, entry));
    }

    /// A read or write of LG's driver files.
    fn driver_io(&self) -> io::Result<()> {
        match self.fault {
            Some(Fault::Dead) => Err(io::ErrorKind::BrokenPipe.into()),
            Some(Fault::Blocked) => Err(io::ErrorKind::TimedOut.into()),
            _ => Ok(()),
        }
    }

    fn gone(&self) -> bool {
        self.fault == Some(Fault::Dead)
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
                adec: ADEC,
                adec_connected: false,
                adec_input: "HDMI_PORT2".into(),
                adec_start: false,
                adec_mute: true,
                belief_muted: false,
                others: Vec::new(),
                mixer: None,
                listed_at: 0,
                apps: vec![(APP.to_owned(), PID)],
                pipelines: Vec::new(),
                hdmi_ok: true,
                hdmi_calls: 0,
                stuck: Vec::new(),
                wedge_on_words: false,
                status_ok: true,
                connect_ok: true,
                disconnect_ok: true,
                cmd_ok: true,
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
                luna_calls: 0,
                luna_ms: 0,
                scan_ms: 0,
                stuck_scans: None,
                fault: None,
                trip: None,
            }),
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut Sim) -> R) -> R {
        f(&mut self.sim.lock().unwrap())
    }

    /// LG connects its mixer input for the player's Load and mixes the words, as at the Load: `AMIXER4`, for the pipeline that
    /// holds `PCMMC4`.
    pub fn lg_mixer(&self) {
        self.with(|sim| {
            sim.mixer = Some(PORT);
            sim.words.mix = 1;
            if !sim.pipelines.iter().any(|pipe| pipe.id == PIPELINE) {
                sim.pipelines.push(Pipe {
                    id: PIPELINE.into(),
                    app: APP.into(),
                    resources: vec![("VDEC".into(), 0), ("PCMMC".into(), PORT)],
                });
            }
        });
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
        if sim.gone() {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        match path {
            SNDOUT => {
                sim.driver_io()?;
                sim.sndout_reads += 1;
                Ok(sim.render_sndout())
            }
            path if path == adec_path(sim.adec) => {
                sim.driver_io()?;
                Ok(sim.render_adec())
            }
            GRAPH => {
                sim.driver_io()?;
                Ok(if sim.now >= sim.graph_at {
                    sim.chain_graph.clone()
                } else {
                    sim.idle_graph.clone()
                })
            }
            _ => sim.files.get(path).cloned().ok_or_else(not_found),
        }
    }

    fn is_dir(&self, path: &str) -> bool {
        self.with(|sim| sim.driver_io().is_ok() && sim.dirs.contains(path))
    }

    fn write(&self, path: &str, _text: &str) -> io::Result<()> {
        self.with(|sim| {
            sim.driver_io()?;
            let converter = path
                .strip_prefix(INSTANCES)
                .and_then(|rest| rest.strip_prefix('/'))
                .and_then(|rest| rest.strip_suffix("/cmd"))
                .ok_or_else(not_found)?;
            if !sim.dirs.contains(&format!("{INSTANCES}/{converter}")) || !sim.cmd_ok {
                return Err(not_found());
            }
            sim.configured.insert(converter.to_owned());
            sim.note(format!("cmd {converter}"));
            Ok(())
        })
    }

    fn save(&self, path: &str, text: &str) -> io::Result<()> {
        self.with(|sim| {
            if sim.gone() {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            sim.files.insert(path.to_owned(), text.to_owned());
            sim.note("save");
            Ok(())
        })
    }

    fn remove(&self, path: &str) -> io::Result<()> {
        self.with(|sim| {
            if sim.gone() {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            if sim.files.remove(path).is_some() {
                sim.note("remove");
            }
            Ok(())
        })
    }

    fn sha256(&self, path: &str) -> io::Result<String> {
        self.with(|sim| sim.hashes.get(path).cloned().ok_or_else(not_found))
    }

    fn fds_on(&self, pid: u32, node: &str) -> io::Result<usize> {
        self.with(|sim| {
            sim.procs
                .get(&pid)
                .map(|(_, fds)| fds.iter().filter(|target| *target == node).count())
                .ok_or_else(not_found)
        })
    }

    fn start_time(&self, pid: u32) -> Option<u64> {
        self.with(|sim| sim.procs.get(&pid).map(|(start, _)| *start))
    }

    fn set_earc_words(&self, words: Words) -> Result<(), WriteError> {
        self.with(|sim| {
            if sim.gone() {
                return Err(WriteError::Failed("the helper is gone".into()));
            }
            if let Some(error) = sim.words_error.clone() {
                return Err(error);
            }
            sim.words = words;
            sim.note(format!("words {words}"));
            if sim.wedge_on_words {
                sim.stuck.push(Stuck {
                    comm: "extinput-integration".into(),
                    wchan: "snd_ctl_elem_info".into(),
                    stack: String::new(),
                });
            }
            Ok(())
        })
    }

    fn luna(&self, uri: &str, payload: &str) -> Option<String> {
        const OK: &str = r#"{"returnValue":true}"#;
        const NO: &str = r#"{"returnValue":false,"errorText":"Audio not connected"}"#;
        const HDMI_METHODS: &str = r#"{"/":{"open":"METHOD","getVRRInfo":"METHOD"}}"#;
        self.with(|sim| {
            // The call takes `luna_ms` of the fake clock.
            sim.luna_calls += 1;
            sim.now += sim.luna_ms;
            if matches!(sim.fault, Some(Fault::Dead | Fault::Silent)) {
                return None;
            }
            let verb = uri.rsplit('/').next().unwrap_or_default();
            let mixer = payload
                .split(r#""source":"AMIXER"#)
                .nth(1)
                .and_then(|rest| rest.split('"').next()?.parse::<u8>().ok());
            if let Some(port) = mixer {
                sim.note(format!("luna {verb} AMIXER{port}"));
                let done = match verb {
                    "disconnect" if sim.disconnect_ok && sim.mixer == Some(port) => {
                        sim.mixer = None;
                        true
                    }
                    "connect" if sim.connect_ok && sim.mixer.is_none() => {
                        sim.mixer = Some(port);
                        true
                    }
                    _ => false,
                };
                return Some(if done { OK } else { NO }.into());
            }
            // A call that names another decoder input than the granted one finds nothing to act on.
            let ours = payload.contains(&format!(r#""source":"ADEC{}""#, sim.adec));
            match verb {
                "introspection" if uri == crate::health::HDMI_INTROSPECTION => {
                    sim.hdmi_calls += 1;
                    sim.hdmi_ok.then(|| HDMI_METHODS.to_owned())
                }
                "getStatus" => sim.status_ok.then(|| sim.render_umi()),
                "running" => Some(sim.render_running()),
                "getActivePipelines" => Some(sim.render_pipelines()),
                "connect" => {
                    sim.note("luna connect");
                    if !sim.connect_ok || !ours {
                        return Some(NO.into());
                    }
                    sim.adec_connected = true;
                    sim.belief_muted = false;
                    Some(OK.into())
                }
                "disconnect" => {
                    sim.note("luna disconnect");
                    if !sim.disconnect_ok || !ours {
                        return Some(NO.into());
                    }
                    sim.adec_connected = false;
                    Some(OK.into())
                }
                "mute" => {
                    let mute = payload.contains(r#""mute":true"#);
                    sim.note(format!("luna mute {mute}"));
                    if !sim.adec_connected || !ours {
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
        self.with(|sim| {
            sim.now += sim.scan_ms;
            if sim.gone() {
                return Vec::new();
            }
            let seen = sim.stuck.clone();
            if let Some(left) = sim.stuck_scans {
                sim.stuck_scans = (left > 1).then_some(left - 1);
                if left <= 1 {
                    sim.stuck.clear();
                }
            }
            seen
        })
    }

    fn driver_busy(&self) -> bool {
        self.with(|sim| sim.fault == Some(Fault::Blocked))
    }

    fn now_ms(&self) -> u64 {
        self.now()
    }

    fn sleep_ms(&self, ms: u64) {
        self.with(|sim| sim.now += ms);
    }
}
