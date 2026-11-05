//! The slates' look: the active theme as the shell reads it (`rustui/tokens`): the shipped `payload/theme/theme.json` with the owner's
//! `theme/theme.json` merged over it object by object, its dark roles unless its material is light, and its faces from the package's
//! `payload/fonts/`. The owner's file is read again when it changes, so a new theme reaches the next stream.

use lumatuner::{Look, Type};
use parking_lot::Mutex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const SHIPPED: &str = include_str!("../../../../payload/theme/theme.json");

/// The fonts the package installs.
pub fn fonts_dir(app_id: &str) -> PathBuf {
    PathBuf::from(format!("/media/developer/apps/usr/palm/applications/{app_id}/payload/fonts"))
}

pub struct Theme {
    owner: PathBuf,
    fonts: PathBuf,
    /// The look last made, and the owner's file's time it was made from.
    made: Mutex<Option<(Option<SystemTime>, Option<Look>)>>,
}

impl Theme {
    pub fn new(owner: PathBuf, fonts: PathBuf) -> Self {
        Self {
            owner,
            fonts,
            made: Mutex::new(None),
        }
    }

    /// The look of the theme as it is now; none when its faces cannot be read.
    pub fn look(&self) -> Option<Look> {
        let stamp = std::fs::metadata(&self.owner).and_then(|m| m.modified()).ok();
        let mut made = self.made.lock();
        if let Some((at, look)) = made.as_ref()
            && *at == stamp
        {
            return look.clone();
        }
        let mut theme: Value = serde_json::from_str(SHIPPED).expect("the shipped theme is JSON");
        if let Some(own) = std::fs::read(&self.owner).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
            merge(&mut theme, own);
        }
        let look = look_of(&theme, &self.fonts);
        if look.is_none() {
            crate::log_warn!("tuner: no slates: the theme's faces are not in {}", self.fonts.display());
        }
        *made = Some((stamp, look.clone()));
        look
    }
}

/// `over` merged into `base` object by object; anything else in `over` replaces what `base` has.
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                merge(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (base, over) => *base = over,
    }
}

/// A QML colour, `#RRGGBB` or `#AARRGGBB`: its opacity and its colour.
fn colour(text: &str) -> Option<(f32, [u8; 3])> {
    let hex = text.strip_prefix('#')?;
    let n = u32::from_str_radix(hex, 16).ok()?;
    let alpha = match hex.len() {
        6 => 255,
        8 => n >> 24,
        _ => return None,
    };
    Some((alpha as f32 / 255.0, [(n >> 16) as u8, (n >> 8) as u8, n as u8]))
}

fn look_of(theme: &Value, fonts: &Path) -> Option<Look> {
    let roles = &theme[if theme["material"] == "light" { "light" } else { "dark" }];
    let canvas = colour(roles["canvas"].as_str()?)?.1;
    // a role with opacity is drawn over the canvas
    let role = |name: &str| {
        let (alpha, rgb) = colour(roles[name].as_str()?)?;
        Some(std::array::from_fn(|i| (f32::from(rgb[i]) * alpha + f32::from(canvas[i]) * (1.0 - alpha)).round() as u8))
    };
    let face = |role: &str| {
        let spec = &theme["type"][role];
        let weight = spec["weight"].as_str().unwrap_or("medium");
        let file = theme["font"]["files"].as_array()?.iter().filter_map(Value::as_str).find(|f| {
            !f.contains('/') && f.rsplit_once('.').and_then(|(stem, _)| stem.rsplit_once('-')).is_some_and(|(_, w)| w.eq_ignore_ascii_case(weight))
        })?;
        Type::new(&std::fs::read(fonts.join(file)).ok()?, spec["size"].as_f64()? as f32).ok()
    };
    Some(Look {
        canvas,
        ink: role("ink")?,
        quiet: role("ink2")?,
        track: role("trackOff")?,
        accent: role("accent")?,
        warning: role("warning")?,
        danger: role("danger")?,
        headline: face("display")?,
        line: face("title")?,
        small: face("body")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../payload/fonts")
    }

    #[test]
    fn the_look_is_the_shipped_theme_with_the_owners_colours_over_it_read_again_when_the_owners_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let owner = dir.path().join("theme.json");
        let theme = Theme::new(owner.clone(), fonts());
        let shipped = theme.look().unwrap();
        assert_eq!(shipped.canvas, [0x19, 0x17, 0x24]);
        // ink2 is #A8F2EFF6 and the track #0FFFFFFF, over the canvas
        assert_eq!(shipped.quiet, [168, 165, 174]);
        assert_eq!(shipped.track, [39, 37, 49]);
        assert_eq!((shipped.headline.size, shipped.line.size, shipped.small.size), (92.0, 40.0, 28.0));
        std::fs::write(&owner, r##"{"dark": {"accent": "#102030"}, "type": {"display": {"size": 80}}}"##).unwrap();
        let own = theme.look().unwrap();
        assert_eq!((own.accent, own.ink, own.headline.size), ([0x10, 0x20, 0x30], shipped.ink, 80.0));
        std::fs::write(&owner, r##"{"material": "light"}"##).unwrap();
        let file = std::fs::File::options().write(true).open(&owner).unwrap();
        file.set_modified(SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
        assert_ne!(theme.look().unwrap().canvas, shipped.canvas, "the light roles");
    }

    #[test]
    fn without_its_faces_there_is_no_look() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Theme::new(dir.path().join("theme.json"), dir.path().to_owned()).look().is_none());
    }
}
