//! The tool catalogue: every tool an assistant can name, its schema, and the strict parsing of its arguments into a [`Call`].
//!
//! There is no tool that is not in this file. Arguments are validated here, once, before anything else looks at them: an
//! unknown field, a wrong type, a value out of range or an identifier with a character outside `[A-Za-z0-9._-]` is refused, so
//! nothing downstream (a luna payload, a receiver command, a prompt) is ever built from text this module has not checked.

use super::types::{AppInfo, Call, Group, InputInfo, MediaKey, ToolDef};
use serde_json::{Map, Value, json};

/// The longest `reason` an assistant may give (it is shown on the TV as the assistant's own words).
pub const REASON_MAX: usize = 200;
/// The receiver's volume scale (what its panel shows in Absolute mode).
pub const VOLUME_MAX: f64 = 98.0;
/// One volume step on that scale: what the receiver's own volume key changes.
pub const VOLUME_STEP: f64 = 0.5;
/// The most steps one `receiver_change_volume` may take.
pub const STEPS_MAX: u64 = 10;

/// A refused call: `code` is a stable word, `message` one plain line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
}

impl ToolError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_arguments",
            message: message.into(),
        }
    }
}

/// A call as parsed, with the assistant's reason (cleaned) when it gave one.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub call: Call,
    pub reason: Option<String>,
}

/// Text from an assistant or from the TV's own apps, made safe to show: control characters dropped, runs of white space made one
/// space, trimmed, and cut to `max` characters.
pub fn clean_text(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in text
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
    {
        if c.is_whitespace() {
            space = !out.is_empty();
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
        if out.chars().count() >= max {
            break;
        }
    }
    out.chars().take(max).collect()
}

/// Whether `text` is an identifier of the shapes the TV uses for apps and inputs.
pub fn valid_id(text: &str, max: usize) -> bool {
    !text.is_empty()
        && text.len() <= max
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A volume on the nearest half step (a quarter and above rounds up).
pub fn round_half(level: f64) -> f64 {
    (level / VOLUME_STEP).round() * VOLUME_STEP
}

fn object<'a>(arguments: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>, ToolError> {
    let map = match arguments {
        Value::Object(map) => map,
        Value::Null => {
            static EMPTY: std::sync::LazyLock<Map<String, Value>> =
                std::sync::LazyLock::new(Map::new);
            &EMPTY
        }
        _ => return Err(ToolError::invalid("the arguments must be an object")),
    };
    if let Some(extra) = map
        .keys()
        .find(|k| !allowed.contains(&k.as_str()) && k.as_str() != "reason")
    {
        return Err(ToolError::invalid(format!(
            "unexpected argument '{}'",
            clean_text(extra, 40)
        )));
    }
    Ok(map)
}

fn reason_of(map: &Map<String, Value>) -> Result<Option<String>, ToolError> {
    match map.get("reason") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            let cleaned = clean_text(text, REASON_MAX);
            Ok((!cleaned.is_empty()).then_some(cleaned))
        }
        Some(_) => Err(ToolError::invalid("reason must be text")),
    }
}

fn text_arg<'a>(map: &'a Map<String, Value>, key: &str, max: usize) -> Result<&'a str, ToolError> {
    let text = map
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::invalid(format!("{key} must be text")))?;
    if valid_id(text, max) {
        Ok(text)
    } else {
        Err(ToolError::invalid(format!(
            "{key} is not an identifier (letters, digits, dots, dashes and underscores, at most {max})"
        )))
    }
}

/// Parses one tool call. An unknown tool is `not_found`; anything wrong with the arguments is `invalid_arguments`.
pub fn parse(name: &str, arguments: &Value) -> Result<Parsed, ToolError> {
    let call = |call: Call, map: &Map<String, Value>| -> Result<Parsed, ToolError> {
        Ok(Parsed {
            call,
            reason: reason_of(map)?,
        })
    };
    match name {
        "tv_get_power" => call(Call::GetPower, object(arguments, &[])?),
        "tv_get_foreground_app" => call(Call::GetForegroundApp, object(arguments, &[])?),
        "tv_list_inputs" => call(Call::ListInputs, object(arguments, &[])?),
        "tv_list_apps" => call(Call::ListApps, object(arguments, &[])?),
        "tv_get_now_playing" => call(Call::GetNowPlaying, object(arguments, &[])?),
        "receiver_get_status" => call(Call::GetReceiver, object(arguments, &[])?),
        "receiver_set_volume" => {
            let map = object(arguments, &["level"])?;
            let level = map
                .get("level")
                .and_then(Value::as_f64)
                .ok_or_else(|| ToolError::invalid("level must be a number"))?;
            if !level.is_finite() || !(0.0..=VOLUME_MAX).contains(&level) {
                return Err(ToolError::invalid(format!(
                    "level must be between 0 and {VOLUME_MAX}"
                )));
            }
            call(
                Call::SetVolume {
                    level: round_half(level),
                },
                map,
            )
        }
        "receiver_change_volume" => {
            let map = object(arguments, &["direction", "steps"])?;
            let up = match map.get("direction").and_then(Value::as_str) {
                Some("up") => true,
                Some("down") => false,
                _ => return Err(ToolError::invalid("direction must be \"up\" or \"down\"")),
            };
            let steps = match map.get("steps") {
                None => 1,
                Some(v) => v
                    .as_u64()
                    .filter(|n| (1..=STEPS_MAX).contains(n))
                    .ok_or_else(|| {
                        ToolError::invalid(format!(
                            "steps must be a whole number from 1 to {STEPS_MAX}"
                        ))
                    })?,
            };
            call(
                Call::ChangeVolume {
                    up,
                    steps: steps as u8,
                },
                map,
            )
        }
        "receiver_set_mute" => {
            let map = object(arguments, &["muted"])?;
            let muted = map
                .get("muted")
                .and_then(Value::as_bool)
                .ok_or_else(|| ToolError::invalid("muted must be true or false"))?;
            call(Call::SetMute { muted }, map)
        }
        "tv_media_key" => {
            let map = object(arguments, &["key"])?;
            let key = map
                .get("key")
                .and_then(Value::as_str)
                .and_then(MediaKey::parse)
                .ok_or_else(|| {
                    ToolError::invalid("key must be one of play, pause, stop, rewind, fast_forward")
                })?;
            call(Call::MediaKey { key }, map)
        }
        "tv_launch_app" => {
            let map = object(arguments, &["app_id"])?;
            let app_id = text_arg(map, "app_id", 128)?.to_owned();
            call(Call::LaunchApp { app_id }, map)
        }
        "tv_switch_input" => {
            let map = object(arguments, &["input_id"])?;
            let input_id = text_arg(map, "input_id", 64)?.to_owned();
            call(Call::SwitchInput { input_id }, map)
        }
        "tv_standby" => call(Call::Standby, object(arguments, &[])?),
        _ => Err(ToolError {
            code: "not_found",
            message: "there is no such tool".to_owned(),
        }),
    }
}

/// The words the TV's prompt uses for a call: a title and one line of detail. Built here from the *validated* call and the
/// names the TV itself reports, never from anything the assistant wrote.
pub fn describe(call: &Call, app: Option<&AppInfo>, input: Option<&InputInfo>) -> (String, String) {
    let shown = |s: &str| clean_text(s, 60);
    match call {
        Call::GetPower | Call::GetForegroundApp | Call::ListInputs | Call::ListApps | Call::GetNowPlaying | Call::GetReceiver => (
            "Look at what the TV is doing".to_owned(),
            "Read the power state, the app in front, the inputs or the receiver. Nothing is changed.".to_owned(),
        ),
        Call::SetVolume { level } => (format!("Set the receiver volume to {level}"), "On the receiver's own scale, as its display shows it.".to_owned()),
        Call::ChangeVolume { up, steps } => (
            format!("Turn the receiver {} by {} step{}", if *up { "up" } else { "down" }, steps, if *steps == 1 { "" } else { "s" }),
            "One step is what the receiver's volume key changes.".to_owned(),
        ),
        Call::SetMute { muted } => (
            if *muted { "Mute the receiver" } else { "Unmute the receiver" }.to_owned(),
            "The receiver's own mute.".to_owned(),
        ),
        Call::MediaKey { key } => (format!("Press {} on the TV", key.id().replace('_', " ")), "A playback key, as the remote's own.".to_owned()),
        Call::LaunchApp { app_id } => (
            format!("Open {}", app.map_or_else(|| shown(app_id), |a| shown(&a.title))),
            "The app opens in front of what is on now.".to_owned(),
        ),
        Call::SwitchInput { input_id } => (
            format!("Switch the TV to {}", input.map_or_else(|| shown(input_id), |i| shown(&i.label))),
            "What is on now is replaced by that input.".to_owned(),
        ),
        Call::Standby => ("Turn the TV off".to_owned(), "The screen goes dark; the TV stays ready.".to_owned()),
    }
}

fn schema(properties: Value, required: &[&str]) -> Value {
    let mut properties = properties.as_object().cloned().unwrap_or_default();
    properties.insert(
        "reason".to_owned(),
        json!({"type": "string", "maxLength": REASON_MAX, "description": "Why, in a few words. It is shown on the TV as your own words when a request needs approval."}),
    );
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

fn def(
    call: &Call,
    title: &'static str,
    description: &str,
    input_schema: Value,
) -> (Group, ToolDef) {
    (
        call.group(),
        ToolDef {
            name: call.tool_name(),
            title,
            description: description.to_owned(),
            input_schema,
            read_only: call.is_read_only(),
        },
    )
}

/// Every tool with its group, in the order they are listed: the one table the group of a tool is read from.
fn entries() -> Vec<(Group, ToolDef)> {
    vec![
        def(
            &Call::GetPower,
            "TV power state",
            "Whether the TV's screen is on, and its power state (Active, Active Standby, Screen Saver).",
            schema(json!({}), &[]),
        ),
        def(
            &Call::GetForegroundApp,
            "App in front",
            "The app that is in front on the TV now (its id and name), or none when Home or nothing is shown.",
            schema(json!({}), &[]),
        ),
        def(
            &Call::ListInputs,
            "TV inputs",
            "The TV's inputs (HDMI 1 to 4) with their names and whether something is connected.",
            schema(json!({}), &[]),
        ),
        def(
            &Call::ListApps,
            "Apps that can be opened",
            "The apps an assistant may open on the TV, with the id to pass to tv_launch_app.",
            schema(json!({}), &[]),
        ),
        def(
            &Call::GetNowPlaying,
            "What is on",
            "What the TV is showing: the app or input in front and what the receiver is set to. Titles of what an app plays are not available.",
            schema(json!({}), &[]),
        ),
        def(
            &Call::GetReceiver,
            "Receiver status",
            "The Denon receiver's power, input, volume on its 0 to 98 scale, and mute, as last read (a value older than 6 seconds is not given).",
            schema(json!({}), &[]),
        ),
        def(
            &Call::SetVolume { level: 0.0 },
            "Set receiver volume",
            "Sets the receiver's volume on its 0 to 98 scale (the number its display shows). It cannot go above the limit set on the TV.",
            schema(
                json!({"level": {"type": "number", "minimum": 0, "maximum": VOLUME_MAX, "description": "Volume on the receiver's own scale; halves are kept."}}),
                &["level"],
            ),
        ),
        def(
            &Call::ChangeVolume { up: true, steps: 1 },
            "Change receiver volume",
            "Turns the receiver up or down by steps of 0.5 on its scale (what its volume key does). It cannot go above the limit set on the TV.",
            schema(
                json!({
                    "direction": {"type": "string", "enum": ["up", "down"]},
                    "steps": {"type": "integer", "minimum": 1, "maximum": STEPS_MAX, "default": 1},
                }),
                &["direction"],
            ),
        ),
        def(
            &Call::SetMute { muted: true },
            "Mute the receiver",
            "Mutes or unmutes the receiver.",
            schema(json!({"muted": {"type": "boolean"}}), &["muted"]),
        ),
        def(
            &Call::MediaKey {
                key: MediaKey::Play,
            },
            "Playback key",
            "Presses a playback key on the TV: play, pause, stop, rewind or fast_forward. Menus and navigation cannot be driven.",
            schema(
                json!({"key": {"type": "string", "enum": MediaKey::ALL.map(MediaKey::id)}}),
                &["key"],
            ),
        ),
        def(
            &Call::LaunchApp {
                app_id: String::new(),
            },
            "Open an app",
            "Opens an app on the TV by its id (see tv_list_apps). Apps that change the TV's own settings cannot be opened.",
            schema(
                json!({"app_id": {"type": "string", "maxLength": 128, "description": "An id from tv_list_apps, such as netflix."}}),
                &["app_id"],
            ),
        ),
        def(
            &Call::SwitchInput {
                input_id: String::new(),
            },
            "Switch input",
            "Switches the TV to one of its inputs that has something connected (see tv_list_inputs).",
            schema(
                json!({"input_id": {"type": "string", "maxLength": 64, "description": "An id from tv_list_inputs, such as HDMI_2."}}),
                &["input_id"],
            ),
        ),
        def(
            &Call::Standby,
            "Turn the TV off",
            "Puts the TV in standby: the screen goes dark and the TV stays ready.",
            schema(json!({}), &[]),
        ),
    ]
}

/// Every tool, in the order they are listed. `list_tools` of the broker filters this by what an assistant may see.
pub fn catalogue() -> Vec<ToolDef> {
    entries().into_iter().map(|(_, def)| def).collect()
}

/// The group a tool belongs to, by name (`None` for a name that is not a tool).
pub fn group_of(tool: &str) -> Option<Group> {
    entries()
        .into_iter()
        .find(|(_, def)| def.name == tool)
        .map(|(group, _)| group)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(name: &str, args: Value) -> Parsed {
        parse(name, &args).unwrap_or_else(|e| panic!("{name} {args}: {e:?}"))
    }

    fn bad(name: &str, args: Value) -> ToolError {
        parse(name, &args).expect_err(&format!("{name} {args} was accepted"))
    }

    #[test]
    fn every_catalogued_tool_has_a_schema_and_parses_with_its_required_arguments() {
        let samples = [
            ("tv_get_power", json!({})),
            ("tv_get_foreground_app", json!({})),
            ("tv_list_inputs", json!({})),
            ("tv_list_apps", json!({})),
            ("tv_get_now_playing", json!({})),
            ("receiver_get_status", json!({})),
            ("receiver_set_volume", json!({"level": 30})),
            ("receiver_change_volume", json!({"direction": "up"})),
            ("receiver_set_mute", json!({"muted": true})),
            ("tv_media_key", json!({"key": "pause"})),
            ("tv_launch_app", json!({"app_id": "netflix"})),
            ("tv_switch_input", json!({"input_id": "HDMI_2"})),
            ("tv_standby", json!({})),
        ];
        let catalogue = catalogue();
        assert_eq!(catalogue.len(), samples.len());
        for (name, args) in samples {
            let parsed = ok(name, args);
            assert_eq!(parsed.call.tool_name(), name);
            let def = catalogue
                .iter()
                .find(|d| d.name == name)
                .unwrap_or_else(|| panic!("{name} is not catalogued"));
            assert_eq!(def.read_only, parsed.call.is_read_only(), "{name}");
            assert_eq!(def.input_schema["type"], "object", "{name}");
            assert_eq!(def.input_schema["additionalProperties"], false, "{name}");
            assert!(
                def.input_schema["properties"]["reason"].is_object(),
                "{name}"
            );
            assert_eq!(group_of(name), Some(parsed.call.group()), "{name}");
        }
        assert_eq!(group_of("tv_launch_app"), Some(Group::Launch));
        assert_eq!(group_of("nope"), None);
    }

    #[test]
    fn the_catalogue_contains_no_tool_that_could_reach_files_the_shell_or_settings() {
        for def in catalogue() {
            let n = def.name;
            assert!(n.starts_with("tv_") || n.starts_with("receiver_"), "{n}");
            for forbidden in [
                "shell",
                "exec",
                "luna",
                "file",
                "read_file",
                "write",
                "setting",
                "network",
                "firewall",
                "key_press",
                "inject",
                "pair",
                "token",
                "root",
            ] {
                assert!(!n.contains(forbidden), "{n} mentions {forbidden}");
            }
            let props = def.input_schema["properties"].as_object().unwrap();
            for key in props.keys() {
                assert!(
                    [
                        "reason",
                        "level",
                        "direction",
                        "steps",
                        "muted",
                        "key",
                        "app_id",
                        "input_id"
                    ]
                    .contains(&key.as_str()),
                    "{n} takes an argument {key}"
                );
            }
        }
    }

    #[test]
    fn media_keys_are_the_transport_keys_and_only_those() {
        let def = catalogue()
            .into_iter()
            .find(|d| d.name == "tv_media_key")
            .unwrap();
        assert_eq!(
            def.input_schema["properties"]["key"]["enum"],
            json!(["play", "pause", "stop", "rewind", "fast_forward"])
        );
        for key in [
            "up",
            "down",
            "left",
            "right",
            "ok",
            "enter",
            "back",
            "home",
            "menu",
            "power",
            "volume_up",
            "KEY_ENTER",
            "28",
            "",
        ] {
            assert_eq!(
                bad("tv_media_key", json!({"key": key})).code,
                "invalid_arguments",
                "{key}"
            );
        }
        assert_eq!(
            ok("tv_media_key", json!({"key": "fast_forward"})).call,
            Call::MediaKey {
                key: MediaKey::FastForward
            }
        );
    }

    #[test]
    fn unknown_fields_and_wrong_types_are_refused() {
        assert_eq!(
            bad("tv_get_power", json!({"verbose": true})).message,
            "unexpected argument 'verbose'"
        );
        assert_eq!(bad("tv_standby", json!([1])).code, "invalid_arguments");
        assert_eq!(bad("tv_standby", json!("now")).code, "invalid_arguments");
        assert_eq!(
            bad("receiver_set_mute", json!({"muted": "yes"})).code,
            "invalid_arguments"
        );
        assert_eq!(
            bad("receiver_set_mute", json!({})).code,
            "invalid_arguments"
        );
        assert_eq!(
            bad("tv_launch_app", json!({"app_id": 5})).code,
            "invalid_arguments"
        );
        assert_eq!(bad("nope", json!({})).code, "not_found");
        // `null` arguments mean none.
        assert_eq!(ok("tv_get_power", Value::Null).call, Call::GetPower);
        assert_eq!(ok("tv_get_power", json!({})).call, Call::GetPower);
    }

    #[test]
    fn volume_is_checked_and_rounded_to_halves() {
        assert_eq!(
            ok("receiver_set_volume", json!({"level": 35})).call,
            Call::SetVolume { level: 35.0 }
        );
        assert_eq!(
            ok("receiver_set_volume", json!({"level": 35.24})).call,
            Call::SetVolume { level: 35.0 }
        );
        assert_eq!(
            ok("receiver_set_volume", json!({"level": 35.25})).call,
            Call::SetVolume { level: 35.5 }
        );
        assert_eq!(
            ok("receiver_set_volume", json!({"level": 35.74})).call,
            Call::SetVolume { level: 35.5 }
        );
        assert_eq!(
            ok("receiver_set_volume", json!({"level": 35.75})).call,
            Call::SetVolume { level: 36.0 }
        );
        assert_eq!(
            ok("receiver_set_volume", json!({"level": 0})).call,
            Call::SetVolume { level: 0.0 }
        );
        assert_eq!(
            ok("receiver_set_volume", json!({"level": 98})).call,
            Call::SetVolume { level: 98.0 }
        );
        for level in [
            json!(-0.1),
            json!(98.1),
            json!(1e9),
            json!("30"),
            json!(null),
            json!(true),
        ] {
            assert_eq!(
                bad("receiver_set_volume", json!({"level": level})).code,
                "invalid_arguments",
                "{level}"
            );
        }
    }

    #[test]
    fn volume_steps_default_to_one_and_stay_in_range() {
        assert_eq!(
            ok("receiver_change_volume", json!({"direction": "down"})).call,
            Call::ChangeVolume {
                up: false,
                steps: 1
            }
        );
        assert_eq!(
            ok(
                "receiver_change_volume",
                json!({"direction": "up", "steps": 10})
            )
            .call,
            Call::ChangeVolume {
                up: true,
                steps: 10
            }
        );
        for args in [
            json!({"direction": "sideways"}),
            json!({"direction": "up", "steps": 0}),
            json!({"direction": "up", "steps": 11}),
            json!({"direction": "up", "steps": 1.5}),
            json!({"direction": "up", "steps": -1}),
            json!({"direction": "up", "steps": "2"}),
            json!({"steps": 2}),
        ] {
            assert_eq!(
                bad("receiver_change_volume", args.clone()).code,
                "invalid_arguments",
                "{args}"
            );
        }
    }

    #[test]
    fn identifiers_have_one_safe_shape() {
        for good in [
            "netflix",
            "com.webos.app.hdmi2",
            "youtube.leanback.v4",
            "a-b_c.d9",
        ] {
            assert!(valid_id(good, 128), "{good}");
            assert_eq!(
                ok("tv_launch_app", json!({"app_id": good})).call,
                Call::LaunchApp {
                    app_id: good.to_owned()
                }
            );
        }
        for evil in [
            "",
            "a b",
            "a\"b",
            "a'; rm -rf /",
            "../../etc/passwd",
            "a/b",
            "a\nb",
            "\u{e9}t\u{e9}",
            "a\u{0}b",
            "{\"id\":1}",
            &"x".repeat(129),
        ] {
            assert!(!valid_id(evil, 128), "{evil:?}");
            assert_eq!(
                bad("tv_launch_app", json!({"app_id": evil})).code,
                "invalid_arguments",
                "{evil:?}"
            );
        }
        assert!(valid_id(&"x".repeat(64), 64) && !valid_id(&"x".repeat(65), 64));
        assert_eq!(
            bad("tv_switch_input", json!({"input_id": "HDMI 2"})).code,
            "invalid_arguments"
        );
        assert_eq!(
            ok("tv_switch_input", json!({"input_id": "HDMI_2"})).call,
            Call::SwitchInput {
                input_id: "HDMI_2".into()
            }
        );
    }

    #[test]
    fn a_reason_is_cleaned_cut_and_optional() {
        assert_eq!(ok("tv_standby", json!({})).reason, None);
        assert_eq!(ok("tv_standby", json!({"reason": null})).reason, None);
        assert_eq!(ok("tv_standby", json!({"reason": "   "})).reason, None);
        assert_eq!(
            ok(
                "tv_standby",
                json!({"reason": "  Nitin\nasked \t me   to\u{7} "})
            )
            .reason
            .as_deref(),
            Some("Nitin asked me to")
        );
        let long = "word ".repeat(100);
        assert_eq!(
            ok("tv_standby", json!({"reason": long}))
                .reason
                .unwrap()
                .chars()
                .count(),
            REASON_MAX
        );
        assert_eq!(
            bad("tv_standby", json!({"reason": 5})).code,
            "invalid_arguments"
        );
    }

    #[test]
    fn clean_text_is_safe_to_show() {
        assert_eq!(clean_text("  a \n\t b  ", 40), "a b");
        assert_eq!(clean_text("\u{7}\u{1b}[31mred", 40), "[31mred");
        assert_eq!(clean_text("abcdef", 3), "abc");
        assert_eq!(clean_text("", 5), "");
        assert_eq!(clean_text("caf\u{e9} \u{1f600}", 40), "caf\u{e9} \u{1f600}");
    }

    #[test]
    fn prompt_words_come_from_the_tvs_names_not_the_assistants() {
        let app = AppInfo {
            id: "netflix".into(),
            title: "Netflix".into(),
        };
        let input = InputInfo {
            id: "HDMI_2".into(),
            label: "AVR-S760H".into(),
            connected: true,
            app_id: Some("com.webos.app.hdmi2".into()),
        };
        let (title, _) = describe(
            &Call::LaunchApp {
                app_id: "netflix".into(),
            },
            Some(&app),
            None,
        );
        assert_eq!(title, "Open Netflix");
        let (title, _) = describe(
            &Call::SwitchInput {
                input_id: "HDMI_2".into(),
            },
            None,
            Some(&input),
        );
        assert_eq!(title, "Switch the TV to AVR-S760H");
        assert_eq!(describe(&Call::Standby, None, None).0, "Turn the TV off");
        assert_eq!(
            describe(&Call::SetVolume { level: 35.5 }, None, None).0,
            "Set the receiver volume to 35.5"
        );
        assert_eq!(
            describe(
                &Call::ChangeVolume {
                    up: false,
                    steps: 3
                },
                None,
                None
            )
            .0,
            "Turn the receiver down by 3 steps"
        );
        assert_eq!(
            describe(&Call::ChangeVolume { up: true, steps: 1 }, None, None).0,
            "Turn the receiver up by 1 step"
        );
        assert_eq!(
            describe(&Call::SetMute { muted: false }, None, None).0,
            "Unmute the receiver"
        );
        assert_eq!(
            describe(
                &Call::MediaKey {
                    key: MediaKey::FastForward
                },
                None,
                None
            )
            .0,
            "Press fast forward on the TV"
        );
        // A title from the TV is cleaned too.
        let hostile = AppInfo {
            id: "x".into(),
            title: "Evil\u{7}\n App".into(),
        };
        assert_eq!(
            describe(
                &Call::LaunchApp { app_id: "x".into() },
                Some(&hostile),
                None
            )
            .0,
            "Open Evil App"
        );
    }
}
