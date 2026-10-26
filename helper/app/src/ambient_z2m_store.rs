//! Where ambient light keeps its way to Zigbee2MQTT: the MQTT broker, the credentials for it, and which bulbs sit where.
//!
//! `<data dir>/ambient/z2m.json`, mode 0600: `{"host", "port", "username", "password", "baseTopic", "fps", "lights": [{"name",
//! "x", "y", "segments"}]}`. It is written by `lumatv-helper z2m-link` (the password comes on standard input) and removed by
//! `z2m-unlink`; the owner of the TV runs those, nothing in the helper does, and neither the settings API nor any other route can
//! change it. The password is never logged, never put in a topic or an API answer, never on a command line; its `Debug` says
//! nothing of it.

use crate::hue_store::{Secret, StoreError};
use lumatv_ambient::{Light, Target};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Write;
use std::net::Ipv4Addr;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// MQTT's plain port, Zigbee2MQTT's default base topic, and Bifrost's default frame rate.
pub const DEFAULT_PORT: u16 = 1883;
pub const DEFAULT_BASE_TOPIC: &str = "zigbee2mqtt";
pub const DEFAULT_FPS: u8 = 20;
pub const MAX_FPS: u8 = 60;
/// A strip has at most this many segments in a Hue frame (Hue's own frames carry ten records).
pub const MAX_SEGMENTS: u8 = 10;
/// How far beside the picture a bulb named by a word is placed (the strip's own sections sit at 0.04).
const BESIDE: f32 = 0.1;

/// `<data dir>/ambient/z2m.json`.
pub fn file(data_dir: &Path) -> PathBuf {
    data_dir.join("ambient").join("z2m.json")
}

/// One bulb (or gradient strip) that follows the picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Z2mLight {
    /// Zigbee2MQTT's friendly name, which is its topic.
    pub name: String,
    /// Where it sits in picture coordinates: (0, 0) the top left of the picture, (1, 1) the bottom right; outside that range is
    /// beside the set.
    pub x: f32,
    pub y: f32,
    /// 1 for a bulb; a gradient strip's segment count, each then a light of its own along the strip.
    #[serde(default = "one")]
    pub segments: u8,
}

fn one() -> u8 {
    1
}

/// The positions a person can name, in picture coordinates.
pub const PLACES: [(&str, f32, f32); 9] = [
    ("left", -BESIDE, 0.5),
    ("right", 1.0 + BESIDE, 0.5),
    ("top", 0.5, -BESIDE),
    ("bottom", 0.5, 1.0 + BESIDE),
    ("top-left", -BESIDE, -BESIDE),
    ("top-right", 1.0 + BESIDE, -BESIDE),
    ("bottom-left", -BESIDE, 1.0 + BESIDE),
    ("bottom-right", 1.0 + BESIDE, 1.0 + BESIDE),
    ("behind", 0.5, 0.5),
];

impl Z2mLight {
    /// `<friendly name>=<place>`: the place a word of [`PLACES`] or `x,y`, with `:<segments>` after it for a strip.
    pub fn parse(spec: &str) -> Result<Self, StoreError> {
        let Some((name, place)) = spec.rsplit_once('=') else {
            return Err(StoreError(format!(
                "a light is <Zigbee2MQTT name>=<place>, the place one of {} or x,y: {spec:?}",
                PLACES.map(|p| p.0).join(", ")
            )));
        };
        let name = name.trim();
        if name.is_empty() || name.len() > 128 || name.contains(['#', '+', '\0']) {
            return Err(StoreError(
                "a light's name is its Zigbee2MQTT friendly name, 1 to 128 characters without # or +".to_owned(),
            ));
        }
        let (place, segments) = match place.rsplit_once(':') {
            Some((p, s)) => (
                p,
                s.trim()
                    .parse::<u8>()
                    .ok()
                    .filter(|n| (1..=MAX_SEGMENTS).contains(n))
                    .ok_or_else(|| {
                        StoreError(format!("segments are 1 to {MAX_SEGMENTS}: {spec:?}"))
                    })?,
            ),
            None => (place, 1),
        };
        let place = place.trim().to_ascii_lowercase();
        let (x, y) = if let Some(p) = PLACES.iter().find(|p| p.0 == place) {
            (p.1, p.2)
        } else {
            let coords: Vec<f32> = place
                .split(',')
                .filter_map(|v| v.trim().parse::<f32>().ok())
                .filter(|v| v.is_finite() && (-2.0..=3.0).contains(v))
                .collect();
            match coords[..] {
                [x, y] => (x, y),
                _ => {
                    return Err(StoreError(format!(
                        "the place is one of {} or x,y between -2 and 3: {spec:?}",
                        PLACES.map(|p| p.0).join(", ")
                    )));
                }
            }
        };
        Ok(Z2mLight {
            name: name.to_owned(),
            x,
            y,
            segments,
        })
    }

    /// The engine's lights for this one: a bulb is one light; a strip's segments are spread a little along a line through its
    /// place (left to right), each a channel of its own. `first` is the channel of the first.
    pub fn lights(&self, first: u8) -> Vec<Light> {
        let n = usize::from(self.segments.max(1));
        (0..n)
            .map(|i| {
                let t = if n > 1 {
                    (i as f32 / (n - 1) as f32) - 0.5
                } else {
                    0.0
                };
                Light {
                    target: Target::Hue {
                        channel: first.saturating_add(u8::try_from(i).unwrap_or(u8::MAX)),
                    },
                    x: self.x + t * 0.6,
                    y: self.y,
                }
            })
            .collect()
    }
}

/// The broker, the credentials, and the lights.
#[derive(Clone, PartialEq)]
pub struct Z2mLink {
    pub host: Ipv4Addr,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<Secret>,
    pub base_topic: String,
    /// Frames a second (Bifrost's `streaming_fps`, default 20); the smoothing of each frame is set to match.
    pub fps: u8,
    pub lights: Vec<Z2mLight>,
}

impl fmt::Debug for Z2mLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Z2mLink({}:{}, user {:?}, password {}, base {:?}, {} fps, lights {:?})",
            self.host,
            self.port,
            self.username,
            if self.password.is_some() {
                "<redacted>"
            } else {
                "none"
            },
            self.base_topic,
            self.fps,
            self.lights
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
        )
    }
}

impl Z2mLink {
    /// The link a person gave, checked: a home-network address (the password and the picture's colours never leave it), a base
    /// topic without wildcards, one to ten lights of at most ten records between them.
    pub fn new(
        host: &str,
        port: u16,
        username: Option<&str>,
        password: Option<&str>,
        base_topic: &str,
        fps: u8,
        lights: Vec<Z2mLight>,
    ) -> Result<Self, StoreError> {
        let host: Ipv4Addr = host
            .trim()
            .parse()
            .map_err(|_| StoreError("the broker's address must be an IPv4 address".to_owned()))?;
        if !crate::sys::is_lan_address(host) {
            return Err(StoreError(
                "the broker's address must be a private (or loopback) IPv4 address".to_owned(),
            ));
        }
        if port == 0 {
            return Err(StoreError("a port of 0 is not a port".to_owned()));
        }
        let base_topic = base_topic.trim().trim_matches('/');
        if base_topic.is_empty()
            || base_topic.len() > 128
            || base_topic.contains(['#', '+', '\0'])
            || !base_topic.is_ascii()
        {
            return Err(StoreError(
                "the base topic is 1 to 128 ASCII characters without # or + (Zigbee2MQTT's default is zigbee2mqtt)".to_owned(),
            ));
        }
        let username = username.map(str::trim).filter(|u| !u.is_empty());
        if username.is_some_and(|u| u.len() > 256 || u.contains('\0')) {
            return Err(StoreError(
                "the user name is at most 256 characters".to_owned(),
            ));
        }
        let password = password.filter(|p| !p.is_empty());
        if password.is_some_and(|p| p.len() > 1024 || p.contains('\0')) {
            return Err(StoreError(
                "the password is at most 1024 characters".to_owned(),
            ));
        }
        if password.is_some() && username.is_none() {
            return Err(StoreError("a password needs a user name".to_owned()));
        }
        if !(1..=MAX_FPS).contains(&fps) {
            return Err(StoreError(format!(
                "the frame rate is 1 to {MAX_FPS} a second"
            )));
        }
        if lights.is_empty() {
            return Err(StoreError("name at least one light".to_owned()));
        }
        let records: usize = lights.iter().map(|l| usize::from(l.segments.max(1))).sum();
        if records > usize::from(MAX_SEGMENTS) {
            return Err(StoreError(format!(
                "{records} lights (segments counted) is more than one Hue frame carries ({MAX_SEGMENTS})"
            )));
        }
        for (i, l) in lights.iter().enumerate() {
            if lights[..i].iter().any(|o| o.name == l.name) {
                return Err(StoreError(format!("the light {:?} is named twice", l.name)));
            }
        }
        Ok(Self {
            host,
            port,
            username: username.map(str::to_owned),
            password: password.map(|p| Secret::new(p.to_owned())),
            base_topic: base_topic.to_owned(),
            fps,
            lights,
        })
    }

    /// The engine's lights for all of them, channel 0 upwards in the order given.
    pub fn engine_lights(&self) -> Vec<Light> {
        let mut out = Vec::new();
        for l in &self.lights {
            let first = u8::try_from(out.len()).unwrap_or(u8::MAX);
            out.extend(l.lights(first));
        }
        out
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Stored {
    host: String,
    port: u16,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    base_topic: String,
    fps: u8,
    lights: Vec<Z2mLight>,
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // `mode` above only applies to a new file.
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Saves the link; the file is replaced whole.
pub fn save(data_dir: &Path, link: &Z2mLink) -> Result<(), StoreError> {
    let folder = data_dir.join("ambient");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(&folder)
        .map_err(|e| StoreError(format!("{}: {e}", folder.display())))?;
    let stored = Stored {
        host: link.host.to_string(),
        port: link.port,
        username: link.username.clone(),
        password: link.password.as_ref().map(|p| p.expose().to_owned()),
        base_topic: link.base_topic.clone(),
        fps: link.fps,
        lights: link.lights.clone(),
    };
    let bytes = serde_json::to_vec(&stored).map_err(|e| StoreError(e.to_string()))?;
    let path = file(data_dir);
    write_private(&path, &bytes).map_err(|e| StoreError(format!("{}: {e}", path.display())))
}

/// The saved link: `Ok(None)` when there is none, an error (without the password) when the file is there and cannot be used.
pub fn load(data_dir: &Path) -> Result<Option<Z2mLink>, StoreError> {
    let path = file(data_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StoreError(format!("{}: {e}", path.display()))),
    };
    let stored: Stored = serde_json::from_str(&text).map_err(|_| {
        StoreError(format!(
            "{} is not a saved Zigbee2MQTT link",
            path.display()
        ))
    })?;
    Z2mLink::new(
        &stored.host,
        stored.port,
        stored.username.as_deref(),
        stored.password.as_deref(),
        &stored.base_topic,
        stored.fps,
        stored.lights,
    )
    .map(Some)
    .map_err(|e| StoreError(format!("{}: {e}", path.display())))
}

/// Removes the saved link. True when there was one.
pub fn clear(data_dir: &Path) -> bool {
    let path = file(data_dir);
    let _ = std::fs::remove_file(path.with_extension("tmp"));
    std::fs::remove_file(&path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lights() -> Vec<Z2mLight> {
        vec![
            Z2mLight::parse("TV left=left").unwrap(),
            Z2mLight::parse("TV right=right").unwrap(),
        ]
    }

    #[test]
    fn a_light_is_a_name_and_a_place() {
        let l = Z2mLight::parse("Hue TV left=left").unwrap();
        assert_eq!(
            (l.name.as_str(), l.x, l.y, l.segments),
            ("Hue TV left", -0.1, 0.5, 1)
        );
        let l = Z2mLight::parse("lamp=0.25, 1.2").unwrap();
        assert_eq!((l.x, l.y), (0.25, 1.2));
        let l = Z2mLight::parse("strip=top:7").unwrap();
        assert_eq!((l.segments, l.y), (7, -0.1));
        let xs: Vec<f32> = l.lights(2).iter().map(|l| l.x).collect();
        assert_eq!(xs.len(), 7);
        assert!(
            (xs[0] - 0.2).abs() < 1e-5 && (xs[6] - 0.8).abs() < 1e-5,
            "{xs:?}"
        );
        assert_eq!(l.lights(2)[3].target, Target::Hue { channel: 5 });
        // A name may carry '=' and '/': the place is after the last '='.
        let l = Z2mLight::parse("a=b/c=behind").unwrap();
        assert_eq!((l.name.as_str(), l.x, l.y), ("a=b/c", 0.5, 0.5));
        for bad in [
            "noplace",
            "x=nowhere",
            "x=1,2,3",
            "x=left:0",
            "x=left:11",
            "=left",
            "a#b=left",
            "x=9,9",
        ] {
            assert!(Z2mLight::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_link_is_checked_and_its_debug_hides_the_password() {
        let link = Z2mLink::new(
            "192.168.1.146",
            1883,
            Some("lumatv"),
            Some("hunter2"),
            "zigbee2mqtt",
            20,
            lights(),
        )
        .unwrap();
        let shown = format!("{link:?}");
        assert!(
            shown.contains("<redacted>") && !shown.contains("hunter2"),
            "{shown}"
        );
        assert_eq!(link.engine_lights().len(), 2);
        assert_eq!(link.engine_lights()[1].target, Target::Hue { channel: 1 });
        let anon = Z2mLink::new("10.0.0.5", 1883, None, None, "/z2m/", 5, lights()).unwrap();
        assert_eq!((anon.username, anon.base_topic.as_str()), (None, "z2m"));
        let bad = [
            Z2mLink::new("8.8.8.8", 1883, None, None, "zigbee2mqtt", 20, lights()),
            Z2mLink::new(
                "broker.local",
                1883,
                None,
                None,
                "zigbee2mqtt",
                20,
                lights(),
            ),
            Z2mLink::new("192.168.1.146", 0, None, None, "zigbee2mqtt", 20, lights()),
            Z2mLink::new(
                "192.168.1.146",
                1883,
                None,
                Some("p"),
                "zigbee2mqtt",
                20,
                lights(),
            ),
            Z2mLink::new("192.168.1.146", 1883, None, None, "z2m/#", 20, lights()),
            Z2mLink::new(
                "192.168.1.146",
                1883,
                None,
                None,
                "zigbee2mqtt",
                0,
                lights(),
            ),
            Z2mLink::new(
                "192.168.1.146",
                1883,
                None,
                None,
                "zigbee2mqtt",
                61,
                lights(),
            ),
            Z2mLink::new("192.168.1.146", 1883, None, None, "zigbee2mqtt", 20, vec![]),
            Z2mLink::new(
                "192.168.1.146",
                1883,
                None,
                None,
                "zigbee2mqtt",
                20,
                vec![lights()[0].clone(), lights()[0].clone()],
            ),
            Z2mLink::new(
                "192.168.1.146",
                1883,
                None,
                None,
                "zigbee2mqtt",
                20,
                vec![
                    Z2mLight::parse("a=top:7").unwrap(),
                    Z2mLight::parse("b=bottom:7").unwrap(),
                ],
            ),
        ];
        for (i, b) in bad.iter().enumerate() {
            assert!(b.is_err(), "{i}");
        }
    }

    #[test]
    fn the_file_is_private_and_comes_back_as_saved() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), None);
        let link = Z2mLink::new(
            "127.0.0.1",
            1884,
            Some("u"),
            Some("secret-words"),
            "zigbee2mqtt",
            25,
            lights(),
        )
        .unwrap();
        save(dir.path(), &link).unwrap();
        let path = file(dir.path());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(load(dir.path()).unwrap(), Some(link.clone()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\"baseTopic\":\"zigbee2mqtt\"") && text.contains("\"segments\":1"),
            "{text}"
        );
        assert!(clear(dir.path()));
        assert!(!clear(dir.path()));
        assert_eq!(load(dir.path()).unwrap(), None);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{\"host\":\"127.0.0.1\",\"password\":\"zzz\"}").unwrap();
        let err = load(dir.path()).unwrap_err().to_string();
        assert!(!err.contains("zzz"), "{err}");
    }
}
