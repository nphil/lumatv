//! Where the remote's button choices come from at the shell's start (`remote.buttons`, Settings > Remote & pointer > Buttons):
//!
//! * the choices of the app buttons that earlier versions kept as `remote.appKeys` (`{"netflix": "<app id>", "amazon": ""}`) move to
//!   `remote.buttons` in the words it uses (`app:<id>`, `nothing`);
//! * LG Input Hook's list (`/home/root/.config/lginputhook/keybinds.json`, `{"<code>": {"action": "ignore" | "launch" | "replace" |
//!   "exec", ...}}`) is copied once, so that the buttons behave the same after the app is removed. The list is read once: a file that was
//!   read, however empty, leaves a mark and is not read again, and a choice already in `remote.buttons` is never replaced by it. An
//!   `exec` entry is copied only when its command is a `luna-send` launch of an app, which is the one thing it did that a button now can;
//!   a command of any other kind is not run by a button and is said in the log.
//!
//! Home (its hold is the way out to LG's own Home) and the pointer's own reports are not buttons any choice reaches, so an entry for one is
//! left out.

use serde_json::{Value, json};
use std::path::Path;

/// Where LG Input Hook keeps its list.
pub const HOOK_FILE: &str = "/home/root/.config/lginputhook/keybinds.json";
/// Left in the theme directory once the list has been read.
const MARK: &str = "theme/inputhook-imported";

/// The buttons Settings lists, by LG's code (`network-input-service`'s table, as LG Input Hook's list of the Magic Remote gives them).
const KNOWN: [(&str, u32); 11] = [
    ("netflix", 1037),
    ("amazon", 1038),
    ("disneyplus", 1042),
    ("red", 398),
    ("green", 399),
    ("yellow", 400),
    ("blue", 401),
    ("chup", 402),
    ("chdown", 403),
    ("list", 787),
    ("more", 994),
];
const HOME: [u32; 3] = [102, 172, 773];
const MUTE: u32 = 113;
const POINTER: [u32; 2] = [1198, 1199];

/// The migration of the shell's start, for the data directory. Returns what it did, a line each, for the log.
pub fn migrate(data_dir: &Path) -> Vec<String> {
    migrate_from(data_dir, Path::new(HOOK_FILE))
}

fn migrate_from(data_dir: &Path, hook_file: &Path) -> Vec<String> {
    let mut said = Vec::new();
    let mark = data_dir.join(MARK);
    let hook = if mark.exists() { None } else { std::fs::read_to_string(hook_file).ok() };
    let mut parsed = None;
    if let Some(text) = &hook {
        match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(m)) => parsed = Some(m),
            _ => said.push(format!("{} is not a list of buttons; left alone", hook_file.display())),
        }
    }
    let mut moved = 0;
    let mut copied = Vec::new();
    let done = super::theme::update(data_dir, |theme| {
        let Some(root) = theme.as_object_mut() else { return false };
        let remote = root.entry("remote").or_insert_with(|| json!({}));
        let Some(remote) = remote.as_object_mut() else { return false };
        let mut buttons = remote.get("buttons").and_then(Value::as_object).cloned().unwrap_or_default();
        let before = buttons.clone();
        let legacy = remote.get("appKeys").is_some_and(Value::is_object);
        if let Some(Value::Object(old)) = remote.remove("appKeys").filter(|_| legacy) {
            for (id, v) in old {
                if let Some(app) = v.as_str() {
                    let word = if app.is_empty() { "nothing".to_string() } else { format!("app:{app}") };
                    buttons.entry(id).or_insert(json!(word));
                    moved += 1;
                }
            }
        }
        for (code, entry) in parsed.iter().flatten() {
            match convert(code, entry) {
                Ok((id, word)) if !buttons.contains_key(&id) => {
                    copied.push(format!("{id} {word}"));
                    buttons.insert(id, json!(word));
                }
                Ok(_) => {}
                Err(why) => said.push(format!("Input Hook entry {code}: {why}")),
            }
        }
        let changed = legacy || buttons != before;
        if changed {
            remote.insert("buttons".into(), Value::Object(buttons));
        }
        changed
    });
    match done {
        Ok(_) => {
            if moved > 0 {
                said.push(format!("{moved} app button choice(s) moved from remote.appKeys to remote.buttons"));
            }
            if parsed.is_some() {
                said.push(format!(
                    "LG Input Hook's list read: {} copied{}",
                    copied.len(),
                    if copied.is_empty() { String::new() } else { format!(" ({})", copied.join(", ")) }
                ));
                if let Err(e) = std::fs::write(&mark, b"") {
                    said.push(format!("{}: {e}", mark.display()));
                }
            }
        }
        Err(e) => said.push(format!("theme not changed: {e}")),
    }
    said
}

/// One Input Hook entry as a button id and an action word, or why it is left out.
fn convert(code: &str, entry: &Value) -> Result<(String, String), String> {
    let code: u32 = code.trim().parse().map_err(|_| "its code is not a number".to_string())?;
    if HOME.contains(&code) || POINTER.contains(&code) {
        return Err("Home and the pointer's reports are not remapped".into());
    }
    let id = KNOWN.iter().find(|(_, c)| *c == code).map_or_else(|| format!("code:{code}"), |(id, _)| (*id).to_string());
    let text = |k: &str| entry.get(k).and_then(Value::as_str).unwrap_or("");
    let word = match text("action") {
        "ignore" => "nothing".to_string(),
        "launch" if !text("id").is_empty() => format!("app:{}", text("id")),
        "replace" => {
            let to = entry.get("keycode").and_then(Value::as_u64).and_then(|k| u32::try_from(k).ok()).unwrap_or(0);
            if to == 0 || to == code {
                return Err("replaces the button with nothing".into());
            }
            if HOME.contains(&to) {
                "home".to_string()
            } else if to == MUTE {
                "mute".to_string()
            } else if POINTER.contains(&to) {
                return Err("acts as a pointer report".into());
            } else {
                format!("as:{}", KNOWN.iter().find(|(_, c)| *c == to).map_or_else(|| format!("code:{to}"), |(id, _)| (*id).to_string()))
            }
        }
        "exec" => match launched_app(text("command")) {
            Some(app) => format!("app:{app}"),
            None => return Err("runs a command, which a button does not".into()),
        },
        other => return Err(format!("action {other:?} is not one a button has")),
    };
    Ok((id, word))
}

/// The app a command launches, when it is a `luna-send` call of the app manager's `launch`: what Input Hook's own "launch an app"
/// wrote, and what people typed by hand.
fn launched_app(command: &str) -> Option<String> {
    let command = command.replace("\\\"", "\"");
    if !command.to_lowercase().contains("applicationmanager/launch") {
        return None;
    }
    let after = command.split_once("\"id\"")?.1.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let app = after.split('"').next()?;
    (!app.is_empty() && !app.contains(char::is_whitespace)).then(|| app.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumatv-buttons-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("theme")).unwrap();
        dir
    }

    fn theme(dir: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join("theme/theme.json")).unwrap()).unwrap()
    }

    #[test]
    fn the_app_key_choices_move_to_the_new_words_and_the_old_key_is_gone() {
        let dir = scratch("legacy");
        std::fs::write(
            dir.join("theme/theme.json"),
            r#"{"remote": {"appKeys": {"disneyplus": "com.beb.plxnative", "amazon": ""}, "recents": true}, "accent": "x"}"#,
        )
        .unwrap();
        let said = migrate_from(&dir, &dir.join("none.json"));
        assert_eq!(said, vec!["2 app button choice(s) moved from remote.appKeys to remote.buttons"]);
        let t = theme(&dir);
        assert_eq!(t["remote"]["buttons"], json!({"disneyplus": "app:com.beb.plxnative", "amazon": "nothing"}));
        assert!(t["remote"].get("appKeys").is_none());
        assert_eq!((t["remote"]["recents"].clone(), t["accent"].clone()), (json!(true), json!("x")), "the rest stays");
        assert!(migrate_from(&dir, &dir.join("none.json")).is_empty(), "nothing left to move");
        assert!(!dir.join(MARK).exists(), "no list was read, so no mark");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_empty_theme_and_no_list_are_left_untouched() {
        let dir = scratch("none");
        assert!(migrate_from(&dir, &dir.join("none.json")).is_empty());
        assert!(!dir.join("theme/theme.json").exists(), "nothing was written");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_empty_list_leaves_a_mark_and_changes_nothing() {
        let dir = scratch("empty");
        std::fs::write(dir.join("theme/theme.json"), r#"{"accent": "x"}"#).unwrap();
        let hook = dir.join("keybinds.json");
        std::fs::write(&hook, "{}").unwrap();
        let said = migrate_from(&dir, &hook);
        assert_eq!(said, vec!["LG Input Hook's list read: 0 copied"]);
        assert!(dir.join(MARK).exists());
        assert_eq!(theme(&dir), json!({"accent": "x"}), "nothing was added to the theme");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_list_is_copied_once_without_replacing_what_the_owner_chose() {
        let dir = scratch("copy");
        std::fs::write(dir.join("theme/theme.json"), r#"{"remote": {"buttons": {"red": "quick"}}}"#).unwrap();
        let hook = dir.join("keybinds.json");
        std::fs::write(
            &hook,
            json!({
                "398": {"action": "launch", "id": "plex"},
                "399": {"action": "ignore"},
                "400": {"action": "replace", "keycode": 773},
                "401": {"action": "replace", "keycode": 113},
                "402": {"action": "replace", "keycode": 399},
                "403": {"action": "replace", "keycode": 777},
                "1042": {"action": "exec", "command": "luna-send -n 1 \"luna://com.webos.applicationManager/launch\" '{\"id\":\"com.beb.plxnative\"}'"},
                "1234": {"action": "exec", "command": "reboot"},
                "773": {"action": "ignore"},
                "x": {"action": "ignore"},
                "9": {"action": "dance"}
            })
            .to_string(),
        )
        .unwrap();
        let said = migrate_from(&dir, &hook);
        let t = theme(&dir);
        assert_eq!(
            t["remote"]["buttons"],
            json!({"red": "quick", "green": "nothing", "yellow": "home", "blue": "mute", "chup": "as:green", "chdown": "as:code:777", "disneyplus": "app:com.beb.plxnative"}),
            "{said:?}"
        );
        assert!(said.iter().any(|l| l.starts_with("LG Input Hook's list read: 6 copied")), "{said:?}");
        for left_out in ["1234: runs a command", "773: Home", "x: its code", "9: action \"dance\""] {
            assert!(said.iter().any(|l| l.contains(left_out)), "{left_out}: {said:?}");
        }
        // The owner resets the buttons; the list is not read again.
        std::fs::write(dir.join("theme/theme.json"), r#"{"remote": {"buttons": {}}}"#).unwrap();
        assert!(migrate_from(&dir, &hook).is_empty());
        assert_eq!(theme(&dir)["remote"]["buttons"], json!({}));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_launch_command_is_found_in_its_usual_shapes() {
        assert_eq!(launched_app(r#"luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"netflix"}'"#).as_deref(), Some("netflix"));
        assert_eq!(
            launched_app(r#"luna-send -n 1 "luna://com.webos.service.applicationmanager/launch" "{\"id\": \"com.x.y\", \"params\": {}}""#).as_deref(),
            Some("com.x.y")
        );
        assert_eq!(launched_app(r#"luna-send -n 1 luna://com.webos.service.tvpower/power/turnOffScreen '{"id":"x"}'"#), None);
        assert_eq!(launched_app("reboot"), None);
    }

    #[test]
    fn a_theme_that_cannot_be_read_is_left_and_the_list_is_tried_again() {
        let dir = scratch("broken");
        std::fs::write(dir.join("theme/theme.json"), b"{\"remote\": {").unwrap();
        let hook = dir.join("keybinds.json");
        std::fs::write(&hook, r#"{"398": {"action": "ignore"}}"#).unwrap();
        let said = migrate_from(&dir, &hook);
        assert!(said.iter().any(|l| l.starts_with("theme not changed")), "{said:?}");
        assert!(!dir.join(MARK).exists(), "no mark, so the next start reads it again");
        assert_eq!(std::fs::read(dir.join("theme/theme.json")).unwrap(), b"{\"remote\": {");
        let _ = std::fs::remove_dir_all(dir);
    }
}
