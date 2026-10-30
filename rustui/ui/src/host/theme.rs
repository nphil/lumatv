//! The owner's theme file (`<data dir>/theme/theme.json`, the working copy Home, `lumatv theme set` and the profiles module read and
//! write): a patch is merged into what is on disk one level deep, as LumaHome's `persistTheme` did (an object value replaces the key
//! of the same name inside the section, so a map such as `layout.overlays` is written whole and a key left out of it is gone), and
//! written atomically. A file that is there and cannot be read is never taken for an empty one (profiles-design: a working copy
//! that cannot be read is waited for): the patch is refused and the file is left as it is.

use serde_json::{Map, Value};
use std::io;
use std::path::{Path, PathBuf};

pub fn theme_path(data_dir: &Path) -> PathBuf {
    data_dir.join("theme/theme.json")
}

/// `patch` merged into `file` one level deep: for each key of the patch, an object value is merged key by key into the object of the
/// same name (made if it is not one), anything else replaces.
pub fn merge_one_level(file: &mut Value, patch: &Value) {
    let Some(file_obj) = file.as_object_mut() else {
        *file = patch.clone();
        return;
    };
    for (k, v) in patch.as_object().into_iter().flatten() {
        match v {
            Value::Object(section) => {
                let slot = file_obj.entry(k.clone()).or_insert_with(|| Value::Object(Map::new()));
                if !slot.is_object() {
                    *slot = Value::Object(Map::new());
                }
                if let Some(into) = slot.as_object_mut() {
                    for (k2, v2) in section {
                        into.insert(k2.clone(), v2.clone());
                    }
                }
            }
            other => {
                file_obj.insert(k.clone(), other.clone());
            }
        }
    }
}

/// Writes `patch` into the theme file and returns the merged file. `Err` for a file that exists and is not a JSON object, and for a
/// write that failed.
pub fn persist(data_dir: &Path, patch: &Value) -> io::Result<Value> {
    update(data_dir, |file| {
        merge_one_level(file, patch);
        true
    })
    .map(Option::unwrap_or_default)
}

/// Changes the theme file with `change`, which says whether it did: only then is the file written (atomically). Returns the file as it
/// is now when it was written. The same refusals as [`persist`]; a file that is not there is an empty one.
pub fn update(data_dir: &Path, change: impl FnOnce(&mut Value) -> bool) -> io::Result<Option<Value>> {
    let path = theme_path(data_dir);
    let mut file = match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) if v.is_object() => v,
            Ok(_) => return Err(io::Error::other("theme.json is not an object")),
            Err(e) => return Err(io::Error::other(format!("theme.json cannot be read just now: {e}"))),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(e) => return Err(e),
    };
    if !change(&mut file) {
        return Ok(None);
    }
    let text = serde_json::to_string_pretty(&file).map_err(io::Error::other)? + "\n";
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(Some(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumatv-theme-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("theme")).unwrap();
        dir
    }

    #[test]
    fn a_section_is_merged_key_by_key_and_a_map_inside_it_is_replaced_whole() {
        let mut file = json!({"layout": {"overlays": {"volume": {"anchor": "left"}, "popup": {"anchor": "top-left"}}, "keep": 1}, "other": true});
        merge_one_level(&mut file, &json!({"layout": {"overlays": {"popup": {"anchor": "bottom"}}}}));
        assert_eq!(file["layout"]["overlays"], json!({"popup": {"anchor": "bottom"}}), "Reset this: the volume key is gone with the map replaced");
        assert_eq!(file["layout"]["keep"], 1, "the section's other keys stay");
        assert_eq!(file["other"], true);
        merge_one_level(&mut file, &json!({"name": "x", "layout": {"overlays": {}}}));
        assert_eq!(file["name"], "x");
        assert_eq!(file["layout"]["overlays"], json!({}));
    }

    #[test]
    fn persist_writes_atomically_answers_the_merged_file_and_refuses_one_it_cannot_read() {
        let dir = scratch("persist");
        let merged = persist(&dir, &json!({"layout": {"overlays": {"volume": {"anchor": "right", "dx": 0, "dy": 0.1}}}})).expect("written");
        assert_eq!(merged["layout"]["overlays"]["volume"]["anchor"], "right");
        let on_disk: Value = serde_json::from_slice(&std::fs::read(theme_path(&dir)).unwrap()).unwrap();
        assert_eq!(on_disk, merged);
        assert!(!dir.join("theme/theme.json.tmp").exists(), "no temporary is left");
        let again = persist(&dir, &json!({"clock": {"twelveHour": false}})).expect("written again");
        assert_eq!(again["layout"]["overlays"]["volume"]["anchor"], "right", "what was there stays");
        std::fs::write(theme_path(&dir), b"{\"layout\": {").unwrap();
        let refused = persist(&dir, &json!({"a": 1}));
        assert!(refused.is_err(), "a cut-short working copy is not taken for an empty one");
        assert_eq!(std::fs::read(theme_path(&dir)).unwrap(), b"{\"layout\": {", "and is left as it is");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
