//! What an engine is to the app: a manifest shipped beside it, a description it gives once it runs, and one small trait.
//!
//! The app knows no engine by name. It reads the manifests in its package, starts the engines whose integration is switched on, and
//! from then on speaks only [`crate::ipc`] messages, so a new engine (an emulator, Steam Remote Play) is a new directory and a new
//! program, not a change to the app.

use crate::ids::EngineId;
use crate::ipc::{FromEngine, PROTOCOL, ToEngine};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// What the app drives: requests in, events out, and neither call waits. An engine inside the app's own process (the TV's native
/// games) implements this directly. One in a process of its own is reached through an adapter that writes each request as a line and
/// reads each event from one, so nothing above the trait can tell the two apart, and a scripted engine stands in for both in tests.
pub trait Engine {
    fn send(&mut self, message: ToEngine);
    fn poll(&mut self) -> Option<FromEngine>;
}

word_enum! {
    /// Where the game runs. It decides the words the app uses, never what the app does.
    pub enum EngineKind {
        /// In a data centre, under an account.
        Cloud => "cloud",
        /// On a PC the household owns.
        Host => "host",
        /// On this TV, as an app of its own.
        Native => "native",
        /// On this TV, inside the engine.
        Emulator => "emulator",
    }
    unknown => Other
}

word_enum! {
    /// What the engine's targets are, which is what the Play chooser calls them.
    pub enum TargetKind {
        Account => "account",
        Host => "host",
        /// This TV: one target, never listed for choosing.
        Device => "device",
    }
    unknown => Other
}

word_enum! {
    /// How a target is linked. The app owns every sign-in and pairing screen; the engine supplies only the code, the PIN or the wait.
    pub enum AuthMethod {
        /// The TV shows a code and an address; the owner approves on another device (GeForce NOW).
        DeviceCode => "device-code",
        /// The TV shows a PIN that is typed in on the host (Sunshine, Apollo, Steam).
        ShowPin => "show-pin",
        /// The host shows a PIN that is typed in on the TV (Punktfunk's pairing).
        EnterPin => "enter-pin",
        /// The TV asks and the owner allows it on the host (Punktfunk's access request).
        Approval => "approval",
    }
    unknown => Other
}

word_enum! {
    /// How a running game reaches the screen.
    pub enum Presentation {
        /// The engine sends coded video and audio to the app's feeder, which owns LG's player and its video plane.
        Feeder => "feeder",
        /// The engine hands the app finished frames to draw (emulators).
        Frames => "frames",
        /// The game is an app of its own and the TV shows it; LumaGame leaves the screen until it ends.
        App => "app",
    }
    unknown => Other
}

word_enum! {
    pub enum Codec {
        H264 => "h264",
        H265 => "h265",
        Av1 => "av1",
    }
    unknown => Other
}

word_enum! {
    /// Something an engine can do beyond listing and launching. The app offers a control only when its engine names the feature.
    pub enum Feature {
        /// Finds its hosts on the home network by itself.
        Discovery => "discovery",
        /// Takes a host by address.
        ManualHost => "manual-host",
        /// Can wake a host that sleeps.
        Wake => "wake",
        /// Several targets can be linked at once.
        ManyTargets => "many-targets",
        /// Reports per target whether a game is installed or owned, so readiness is known before Play.
        Readiness => "readiness",
        /// A launch may wait in a queue, with a position.
        Queue => "queue",
        /// A game left running on the far side can be picked up again.
        Resume => "resume",
        /// Can close the game on the far side, not only the stream.
        EndGame => "end-game",
        /// Reports stream measurements while playing.
        Stats => "stats",
        /// Answers `round-trip` with the legs of a control's trip it can time.
        RoundTrip => "round-trip",
        /// Hands controller keys to the app while the app's overlay holds the input.
        OverlayKeys => "overlay-keys",
        /// Opens Steam's overlay over a running Steam game ([`crate::ipc::Request::SteamOverlay`]).
        SteamOverlay => "steam-overlay",
        Rumble => "rumble",
        Keyboard => "keyboard",
        Mouse => "mouse",
        Surround => "surround",
    }
    unknown => Other
}

/// The best stream an engine can ask its far side for. The app offers no tier above it, and none above what the TV plays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoCaps {
    pub codecs: Vec<Codec>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub hdr10: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub kind: EngineKind,
    pub targets: TargetKind,
    /// Empty when nothing has to be linked.
    #[serde(default)]
    pub auth: Vec<AuthMethod>,
    pub presentation: Presentation,
    #[serde(default)]
    pub features: BTreeSet<Feature>,
    /// `None` for an engine that streams nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoCaps>,
}

impl Capabilities {
    pub fn has(&self, feature: Feature) -> bool {
        self.features.contains(&feature)
    }
}

/// What a running engine says of itself, once, when it is ready.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    pub id: EngineId,
    pub version: String,
    pub capabilities: Capabilities,
}

/// `engines/<id>/engine.json` in the package: enough to list an engine on the Integrations page, and to start it, without running it.
/// An engine that is switched off is never started, so everything shown about it comes from here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: EngineId,
    pub name: String,
    pub summary: String,
    pub kind: EngineKind,
    /// The contract version the engine was built for.
    pub protocol: u32,
    /// The program, relative to the manifest's directory. `None` for an engine that is part of the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// An SPDX expression.
    pub license: String,
    /// Where the exact source of the shipped program is. Required when the licence is a copyleft one: such an engine is a separate
    /// program that this package only carries, and the package has to say where its source lives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    Json(String),
    /// The directory it was found in is not named after its id, so one engine could pass for another.
    Directory,
    /// Built for a contract this app does not speak.
    Protocol(u32),
    /// `exec` is absolute or climbs out of the engine's directory.
    Exec,
    /// Empty name, summary or licence.
    Words,
    /// A copyleft licence without a source address.
    Source,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "not a manifest: {e}"),
            Self::Directory => f.write_str("the directory is not named after the engine"),
            Self::Protocol(found) => write!(f, "built for contract {found}, this app speaks {PROTOCOL}"),
            Self::Exec => f.write_str("the program must be a path inside the engine's directory"),
            Self::Words => f.write_str("name, summary and licence are required"),
            Self::Source => f.write_str("a copyleft engine must name where its source is"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// The licences that oblige whoever ships a program to offer its source: the GPL family (LGPL and AGPL contain the letters) and
/// the MPL.
fn is_copyleft(license: &str) -> bool {
    let license = license.to_ascii_uppercase();
    license.contains("GPL") || license.contains("MPL")
}

impl Manifest {
    /// Reads the manifest found in the directory `dir` of the package's `engines/`.
    pub fn parse(dir: &str, json: &str) -> Result<Manifest, ManifestError> {
        let manifest: Manifest = serde_json::from_str(json).map_err(|e| ManifestError::Json(e.to_string()))?;
        if manifest.id.as_str() != dir {
            return Err(ManifestError::Directory);
        }
        if manifest.protocol != PROTOCOL {
            return Err(ManifestError::Protocol(manifest.protocol));
        }
        if let Some(exec) = &manifest.exec {
            if exec.is_empty() || exec.starts_with('/') || exec.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
                return Err(ManifestError::Exec);
            }
        }
        if [&manifest.name, &manifest.summary, &manifest.license].iter().any(|words| words.trim().is_empty()) {
            return Err(ManifestError::Words);
        }
        if is_copyleft(&manifest.license) && manifest.source.as_deref().is_none_or(|s| s.trim().is_empty()) {
            return Err(ManifestError::Source);
        }
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAMESTREAM: &str = r#"{"id":"gamestream","name":"Sunshine and Apollo","summary":"Play from a PC that runs Sunshine or Apollo.","kind":"host",
        "protocol":1,"exec":"bin/lumagame-gamestream","license":"GPL-3.0-or-later","source":"https://example.invalid/lumagame-gamestream"}"#;

    fn edited(edit: impl FnOnce(&mut serde_json::Value)) -> Result<Manifest, ManifestError> {
        let mut json: serde_json::Value = serde_json::from_str(GAMESTREAM).unwrap();
        edit(&mut json);
        Manifest::parse("gamestream", &json.to_string())
    }

    #[test]
    fn a_manifest_names_a_program_inside_its_own_directory() {
        let m = Manifest::parse("gamestream", GAMESTREAM).unwrap();
        assert_eq!((m.id.as_str(), m.kind, m.exec.as_deref()), ("gamestream", EngineKind::Host, Some("bin/lumagame-gamestream")));
        assert_eq!(Manifest::parse("gfn", GAMESTREAM), Err(ManifestError::Directory));
        for exec in ["/bin/sh", "../gfn/bin/engine", "bin/../../x", "bin//x", ""] {
            assert_eq!(edited(|j| j["exec"] = exec.into()), Err(ManifestError::Exec), "{exec:?}");
        }
        assert_eq!(edited(|j| j["protocol"] = 2.into()), Err(ManifestError::Protocol(2)));
        assert_eq!(edited(|j| j["name"] = " ".into()), Err(ManifestError::Words));
        assert!(matches!(edited(|j| j["id"] = "Game Stream".into()), Err(ManifestError::Json(_))));
        let built_in = edited(|j| {
            j.as_object_mut().unwrap().remove("exec");
        });
        assert_eq!(built_in.unwrap().exec, None, "an engine inside the app has no program");
    }

    #[test]
    fn a_copyleft_engine_must_say_where_its_source_is() {
        let no_source = |j: &mut serde_json::Value| {
            j.as_object_mut().unwrap().remove("source");
        };
        assert_eq!(edited(no_source), Err(ManifestError::Source));
        for license in ["LGPL-3.0-only", "MPL-2.0", "gpl-3.0"] {
            let r = edited(|j| {
                no_source(j);
                j["license"] = license.into();
            });
            assert_eq!(r, Err(ManifestError::Source), "{license}");
        }
        let permissive = edited(|j| {
            no_source(j);
            j["license"] = "MIT OR Apache-2.0".into();
        });
        assert!(permissive.is_ok());
    }

    #[test]
    fn capabilities_outlive_a_word_from_a_newer_engine() {
        let caps: Capabilities = serde_json::from_str(
            r#"{"kind":"host","targets":"host","auth":["enter-pin","passkey"],"presentation":"feeder","features":["wake","discovery","gyro"],
                "video":{"codecs":["h265","vvc"],"width":3840,"height":2160,"fps":120,"hdr10":true}}"#,
        )
        .unwrap();
        assert_eq!(caps.auth, [AuthMethod::EnterPin, AuthMethod::Other]);
        assert!(caps.has(Feature::Wake) && caps.has(Feature::Discovery) && caps.has(Feature::Other) && !caps.has(Feature::Queue));
        assert_eq!(caps.video.as_ref().unwrap().codecs, [Codec::H265, Codec::Other]);
        assert_eq!(serde_json::to_value(&caps).unwrap()["features"], serde_json::json!(["discovery", "wake", "unknown"]));

        let native: Capabilities = serde_json::from_str(r#"{"kind":"native","targets":"device","presentation":"app"}"#).unwrap();
        assert!(native.auth.is_empty() && native.features.is_empty() && native.video.is_none());
    }
}
