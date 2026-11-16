use super::{View, Section, failed};
use crate::http::Response;
use leadjoy_config::{Change, Command, Controller, Profile, Request as Query, Transport, settings::{ButtonAction, ControllerMode, MacroDevice, MacroStep}};
use serde_json::{Value, json};

pub(super) const SECTIONS: &[(&str, &str)] = &[("general","General and logo"),("profiles","Profiles"),("buttons","Buttons and turbo"),("macros","Macros"),("sticks","Sticks"),("curves","Stick curves"),("directions","Stick direction bindings"),("triggers","Triggers"),("motion","Motion"),("gestures","Motion gestures"),("rumble","Rumble")];
pub(super) const BUTTONS: &[(i64, &str)] = &[(26,"R4 / M3"),(27,"L4 / M4"),(24,"R5 / M1"),(25,"L5 / M2"),(1,"A"),(2,"B"),(4,"X"),(5,"Y"),(14,"Left stick click"),(15,"Right stick click"),(17,"D-pad up"),(18,"D-pad down"),(19,"D-pad left"),(20,"D-pad right"),(7,"LB"),(8,"RB"),(9,"LT"),(10,"RT"),(11,"Select"),(16,"Capture"),(12,"Start"),(3,"C")];
const KEYS: &[(i64, &str)] = &[(0,"None"),(1,"A"),(2,"B"),(3,"C"),(4,"X"),(5,"Y"),(6,"Z"),(7,"LB"),(8,"RB"),(9,"LT"),(10,"RT"),(11,"Select"),(12,"Start"),(13,"Home"),(14,"Left stick click"),(15,"Right stick click"),(16,"Capture"),(17,"D-pad up"),(18,"D-pad down"),(19,"D-pad left"),(20,"D-pad right"),(21,"Menu"),(22,"Mode"),(23,"FN"),(24,"R5 / M1"),(25,"L5 / M2"),(26,"R4 / M3"),(27,"L4 / M4"),(32,"Power")];
const GESTURES: [&str; 6] = ["Yaw left","Yaw right","Roll left","Roll right","Pitch down","Pitch up"];
// c60 keeps 32 bytes of a name and c61 takes no more; the vendor app also refuses \ / : * ? " < > |.
const MACRO_NAME: usize = 32;
// What a new macro starts as; the vendor converter has no default macro to copy.
const FIRST_STEP: MacroStep = MacroStep { device: MacroDevice::Gamepad, key: 1, interval: 0, duration: 100 };
const PAGE_SIZE: usize = 24;
const OPTION_WINDOW: usize = 32;
const MOUSE: &[(i64, &str)] = &[(0,"None"),(1,"Left button"),(2,"Right button"),(3,"Middle button"),(4,"Side button 1"),(5,"Side button 2")];
// The vendor's cy keyboard allow-list, with 0 for an empty action slot.
const KEYBOARD: &[(i64, &str)] = &[
    (0,"None"),(53,"~"),(30,"1"),(31,"2"),(32,"3"),(33,"4"),(34,"5"),(35,"6"),(36,"7"),(37,"8"),(38,"9"),(39,"0"),
    (4,"A"),(5,"B"),(6,"C"),(7,"D"),(8,"E"),(9,"F"),(10,"G"),(11,"H"),(12,"I"),(13,"J"),(14,"K"),(15,"L"),(16,"M"),
    (17,"N"),(18,"O"),(19,"P"),(20,"Q"),(21,"R"),(22,"S"),(23,"T"),(24,"U"),(25,"V"),(26,"W"),(27,"X"),(28,"Y"),(29,"Z"),
    (89,"Num 1"),(90,"Num 2"),(91,"Num 3"),(92,"Num 4"),(93,"Num 5"),(94,"Num 6"),(95,"Num 7"),(96,"Num 8"),(97,"Num 9"),(98,"Num 0"),
    (70,"Print"),(44,"Space"),(41,"Esc"),(43,"Tab"),(57,"Caps Lock"),(80,"Left arrow"),(79,"Right arrow"),(82,"Up arrow"),(81,"Down arrow"),
    (58,"F1"),(59,"F2"),(60,"F3"),(61,"F4"),(62,"F5"),(63,"F6"),(64,"F7"),(65,"F8"),(66,"F9"),(67,"F10"),(68,"F11"),(69,"F12"),
    (77,"End"),(224,"Ctrl"),(225,"Shift"),(226,"Alt"),(227,"Win"),
];
const RANGE: &str = "value is outside this setting's supported range";

pub(super) enum Kind {
    Number(i64, i64, i64, &'static str), Toggle, Choice(&'static [(i64, &'static str)]), Keyboard, Text, Activate, Note(&'static str),
    SubKey(usize), ReportedKey,
    /// What an action does. Switching to or from a macro rewrites the whole action, as the vendor converter does: play once, no repeat interval.
    Target(Vec<(String, u16)>),
    /// The macro an action plays; its step count goes with it.
    Macro(Vec<(String, u16)>),
    NewMacro(Vec<String>),
    /// Copy macro step N after itself, or remove it.
    Insert(usize), Remove(usize),
    Delete(String),
}
pub(super) struct Field { pub key: String, label: String, path: String, kind: Kind, help: Option<&'static str>, pub group: usize }
pub(super) struct Group { pub read: Query, pub value: Value, set: Command, argument: &'static str }
pub(super) struct Settings { pub groups: Vec<Group>, pub fields: Vec<Field>, items: Vec<Value>, steps: u16, pages: u16 }
impl Group {
    fn change(&self, value: Value, key: &str, view: &View) -> Result<Change, Response> {
        match self.read.command() {
            Command::Lighting => {
                let mut fields = serde_json::Map::new(); fields.insert("a".into(), json!(1)); fields.insert("s".into(), value.clone());
                Query::new(Command::Lighting, fields).map(|write| Change { write, read: self.read.clone(), expected: value })
            }
            Command::ProfileSummary => {
                let version = if value.get("d2").is_some() { "d2" } else { "d1" };
                if key.starts_with("sub-") {
                    let entries = value.pointer("/d2/cfg").and_then(Value::as_array).ok_or_else(|| failed("controller omitted sub-configurations"))?;
                    let mut matching = entries.iter().filter(|entry| entry["c"] == view.sub);
                    let mut entry = matching.next().cloned().ok_or_else(|| failed("controller omitted the selected sub-configuration"))?;
                    if matching.next().is_some() { return Err(failed("controller returned ambiguous sub-configurations")); }
                    entry["b"] = json!(view.profile);
                    Change::sub_config(entry, &self.value)
                } else if key == "profile-activate" {
                    Query::argument(Command::ActivateProfile, "s", json!(view.profile)).map(|write| Change { write, read: self.read.clone(), expected: json!({version:{"a":1}}) })
                } else {
                    let name = value[version]["n"].clone();
                    Query::argument(Command::SetProfileName, "s", json!({"b":view.profile,"n":name})).map(|write| Change { write, read: self.read.clone(), expected: json!({version:{"n":name}}) })
                }
            }
            Command::ControllerMode => Change::controller_mode(match value["m"].as_u64() {
                Some(0) => ControllerMode::Xinput, Some(1) => ControllerMode::Switch, Some(2) => ControllerMode::Ps4, Some(3) => ControllerMode::Ps5, Some(4) => ControllerMode::MobileTouch,
                _ => return Err(failed("controller returned an unsupported mode")),
            }),
            Command::JoystickDpad => Change::joystick_dpad(value),
            Command::MacroSummary if key == "delete-macro" => {
                let name = self.value.get(usize::from(view.button) - 1).and_then(|entry| entry["n"].as_str()).ok_or_else(|| failed("controller omitted the selected macro"))?;
                Change::delete_macro(name, &self.value)
            }
            Command::MacroSummary => Change::classic_macro(value.as_str().unwrap_or_default(), &[FIRST_STEP]),
            Command::MacroDetail => Change::classic_macro(value["n"].as_str().unwrap_or_default(), &steps(&value).ok_or_else(|| failed("controller returned an unsupported macro"))?),
            _ if self.argument == "w" => Change::profile_settings(self.set, self.read.clone(), value),
            _ => Query::argument(self.set, self.argument, value.clone()).map(|write| Change { write, read: self.read.clone(), expected: value }),
        }.map_err(failed)
    }
}
impl Field {
    pub fn apply(&self, value: &Value, original: &Value) -> Result<Value, &'static str> {
        let mut next = original.clone();
        let slot = next.pointer_mut(&self.path).ok_or("controller did not report this setting")?;
        match &self.kind {
            Kind::Number(min, max, ..) => *slot = value.as_i64().filter(|n| (*min..=*max).contains(n)).ok_or(RANGE)?.into(),
            Kind::Toggle => *slot = u8::from(value.as_bool().ok_or(RANGE)?).into(),
            Kind::Choice(options) => *slot = value.as_str().and_then(|s| s.parse::<i64>().ok()).filter(|n| options.iter().any(|(id,_)| id == n)).ok_or(RANGE)?.into(),
            Kind::Keyboard => *slot = value.as_str().and_then(|s| s.parse::<i64>().ok()).filter(|n| KEYBOARD.iter().any(|(id,_)| id == n)).ok_or(RANGE)?.into(),
            Kind::ReportedKey => {
                let current = slot.as_i64().filter(|n| (0..=32).contains(n)).ok_or(RANGE)?;
                let selected = value.as_str().and_then(|s| s.parse::<i64>().ok()).filter(|n| *n == current || KEYS.iter().any(|(id,_)| id == n)).ok_or(RANGE)?;
                *slot = selected.into();
            }
            Kind::SubKey(index) => {
                let keys = slot.as_array_mut().ok_or("controller did not report trigger keys")?;
                let old = keys.get(*index).and_then(Value::as_i64);
                let selected = value.as_str().and_then(|s| s.parse::<i64>().ok()).ok_or(RANGE)?;
                if *index > keys.len() || (*index == keys.len() && keys.len() >= 2)
                    || (selected != 0 && !subkey_allowed(selected) && old != Some(selected))
                    || (selected != 0 && keys.iter().enumerate().any(|(i, key)| i != *index && key.as_i64() == Some(selected))) {
                    return Err(RANGE);
                }
                if selected == 0 {
                    if *index < keys.len() { keys.remove(*index); }
                } else if *index == keys.len() {
                    keys.push(json!(selected));
                } else {
                    keys[*index] = json!(selected);
                }
            }
            Kind::Text => *slot = value.as_str().filter(|s| text(s, 128)).ok_or(RANGE)?.into(),
            Kind::Activate | Kind::Delete(_) if !value.is_null() => return Err(RANGE),
            Kind::Activate => *slot = 1.into(),
            Kind::Delete(_) => {}
            Kind::Note(_) => return Err("this setting is read only"),
            Kind::Target(macros) => {
                let target = value.as_str().and_then(|s| s.parse::<u8>().ok()).ok_or(RANGE)?;
                let current = slot["t"].as_u64();
                if current == Some(target.into()) { return Ok(next); }
                let action = match (target, macros.first()) {
                    (0 | 2, _) if matches!(current, Some(0 | 2)) => { slot["t"] = target.into(); return Ok(next); }
                    (0 | 2..=4, _) if matches!(current, Some(0 | 2..=4)) => {
                        let action = ButtonAction::mapping(target, [0; 4], 0).map_err(|_| RANGE)?;
                        slot["t"] = action.target.into(); slot["v"] = json!(action.keys); return Ok(next);
                    }
                    (0 | 2..=4, _) => ButtonAction::mapping(target, [0; 4], 0),
                    (1, Some((name, steps))) => ButtonAction::macro_action(name.clone(), 1, *steps, 0),
                    _ => return Err(RANGE),
                }.map_err(|_| RANGE)?;
                if let (Some(object), Ok(Value::Object(fields))) = (slot.as_object_mut(), serde_json::to_value(action)) { object.extend(fields); }
            }
            Kind::Macro(macros) => {
                let (name, steps) = value.as_str().and_then(|s| macros.iter().find(|(name,_)| name == s)).ok_or(RANGE)?;
                slot["n"] = name.as_str().into(); slot["ms"] = (*steps).into();
            }
            Kind::NewMacro(names) => return value.as_str().map(str::trim)
                .filter(|s| !s.is_empty() && s.len() <= MACRO_NAME && !s.contains(|c: char| c.is_control() || "\\/:*?\"<>|".contains(c)) && !names.iter().any(|name| name == s))
                .map(Value::from).ok_or("a new macro needs a name no other macro uses, up to 32 bytes, without \\ / : * ? \" < > |"),
            Kind::Insert(_) | Kind::Remove(_) if !value.is_null() => return Err(RANGE),
            Kind::Insert(i) | Kind::Remove(i) => {
                let data = slot["d"].as_array_mut().ok_or("controller did not report this setting")?;
                let at = i * 5;
                if matches!(self.kind, Kind::Insert(_)) { let step = data[at..at + 5].to_vec(); data.splice(at + 5..at + 5, step); } else { data.drain(at..at + 5); }
                let count = data.len() / 5;
                slot["s"] = count.into();
            }
        }
        Ok(next)
    }
    fn display(&self, original: &Value) -> Result<(Value, Option<Value>), Response> {
        if let Kind::Note(help) = &self.kind {
            return Ok((json!({"key":self.key,"label":self.label,"type":"note","help":help}), None));
        }
        let value = original.pointer(&self.path).ok_or_else(|| failed(format!("controller omitted {}", self.key)))?;
        let mut field = json!({"key":self.key,"label":self.label});
        if let Some(help) = self.help { field["help"] = json!(help); }
        let unsupported = || failed(format!("controller returned an unsupported {} value", self.key));
        let mut choice = |options: Vec<(String, String)>, current: String| {
            if !options.iter().any(|(id,_)| *id == current) { return Err(unsupported()); }
            field["type"] = json!("choice"); field["default"] = json!(current);
            field["options"] = json!(options.into_iter().map(|(id,label)| json!({"id":id,"label":label})).collect::<Vec<_>>());
            Ok(Some(json!(current)))
        };
        let shown = match &self.kind {
            Kind::Choice(options) => choice(options.iter().map(|(id,label)| (id.to_string(), (*label).into())).collect(), value.as_i64().ok_or_else(unsupported)?.to_string())?,
            Kind::Keyboard => {
                let current = value.as_i64().ok_or_else(unsupported)?;
                let at = KEYBOARD.iter().position(|(id,_)| *id == current).ok_or_else(unsupported)?;
                let count = KEYBOARD.len().min(OPTION_WINDOW);
                let start = (at + KEYBOARD.len() - count / 2) % KEYBOARD.len();
                choice((0..count).map(|i| KEYBOARD[(start + i) % KEYBOARD.len()]).map(|(id,label)| (id.to_string(), label.into())).collect(), current.to_string())?
            }
            Kind::ReportedKey => {
                let current = value.as_i64().filter(|n| (0..=32).contains(n)).ok_or_else(unsupported)?;
                let mut options: Vec<(String,String)> = KEYS.iter().map(|(id,label)| (id.to_string(),(*label).into())).collect();
                if !KEYS.iter().any(|(id,_)| *id == current) { options.push((current.to_string(),format!("Other button {current} (retained)"))); }
                choice(options,current.to_string())?
            }
            Kind::SubKey(index) => {
                let keys = value.as_array().ok_or_else(unsupported)?;
                if *index > keys.len() || (*index == keys.len() && keys.len() >= 2) { return Err(unsupported()); }
                let current = keys.get(*index).map_or(Some(0), Value::as_i64).ok_or_else(unsupported)?;
                let mut options: Vec<(String,String)> = KEYS.iter().filter(|(id,_)| *id == 0 || subkey_allowed(*id))
                    .filter(|(id,_)| *id == 0 || *id == current || !keys.iter().any(|key| key.as_i64() == Some(*id)))
                    .map(|(id,label)| (id.to_string(), (*label).into())).collect();
                if current != 0 && !options.iter().any(|(id,_)| *id == current.to_string()) {
                    let label = KEYS.iter().find(|(id,_)| *id == current).map_or_else(|| format!("Other button {current}"), |(_,label)| (*label).into());
                    options.push((current.to_string(),format!("{label} (retained)")));
                }
                choice(options,current.to_string())?
            }
            Kind::Target(macros) => {
                let current = value["t"].as_u64().ok_or_else(unsupported)?;
                let mut options = vec![("0".into(), "Original input".into()), ("2".into(), "Gamepad".into()),
                    ("3".into(), "Keyboard".into()), ("4".into(), "Mouse".into())];
                if !macros.is_empty() || current == 1 { options.push(("1".into(), "Macro".into())); }
                choice(options, current.to_string())?
            }
            Kind::Macro(macros) => {
                let current = value["n"].as_str().ok_or_else(unsupported)?;
                let mut options: Vec<(String, String)> = macros.iter().map(|(name,_)| (name.clone(), name.clone())).collect();
                if !macros.iter().any(|(name,_)| name == current) { options.push((current.into(), format!("{current} (not on the controller)"))); }
                let at = options.iter().position(|(id,_)| id == current).ok_or_else(unsupported)?;
                choice(options[window(options.len(),at)].to_vec(), current.into())?
            }
            Kind::Number(min,max,step,unit) => {
                let n = value.as_i64().filter(|n| (*min..=*max).contains(n)).ok_or_else(unsupported)?;
                field["type"] = json!("number"); field["min"] = json!(min); field["max"] = json!(max);
                field["step"] = json!(if (n - min) % step == 0 { *step } else { 1 }); field["default"] = json!(n);
                if !unit.is_empty() { field["unit"] = json!(unit); }
                Some(json!(n))
            }
            Kind::Toggle => {
                let n = value.as_u64().filter(|n| *n <= 1).ok_or_else(|| failed(format!("controller returned an invalid {} switch", self.key)))?;
                field["type"] = json!("toggle"); field["default"] = json!(n == 1); Some(json!(n == 1))
            }
            Kind::Text => {
                let name = value.as_str().filter(|name| text(name,128)).ok_or_else(|| failed("controller returned an invalid profile name"))?;
                field["type"] = json!("text"); field["max_chars"] = json!(128); Some(json!(name))
            }
            Kind::NewMacro(_) => {
                field["type"] = json!("text"); field["max_chars"] = json!(MACRO_NAME);
                field["help"] = json!("Name a new macro. It starts as one 100 ms press of A."); Some(json!(""))
            }
            Kind::Activate => { field["type"] = json!("action"); field["help"] = json!(if value == 1 { "This profile is active." } else { "Make this the controller's active profile." }); None }
            Kind::Insert(_) | Kind::Remove(_) => { field["type"] = json!("action"); None }
            Kind::Delete(name) => { field["type"] = json!("action"); field["help"] = json!(format!("The controller forgets the macro “{name}”.")); None }
            Kind::Note(_) => unreachable!(),
        };
        Ok((field, shown))
    }
}
impl Settings {
    /// Selectors that point past what the controller has are moved onto what it has.
    pub fn read<T: Transport>(controller: &mut Controller<T>, view: &mut View) -> Result<Self, Response> {
        let mut this = Self { groups: vec![], fields: vec![], items: vec![], steps: 0, pages: 0 };
        let profile = Profile::new(view.profile, view.sub).map_err(failed)?;
        match view.section {
            Section::General => {
                let g = this.group(controller, Query::lighting(), Command::Lighting, "s")?;
                let lighting: leadjoy_config::Lighting = serde_json::from_value(this.groups[g].value.clone()).map_err(failed)?;
                if lighting.brightness > 100 { return Err(failed("controller returned an invalid logo brightness")); }
                this.number(g,"logo-brightness","Logo brightness","/brt",(0,100),"%");
                for (i,key,label) in [(0,"red","Red"),(1,"green","Green"),(2,"blue","Blue")] { this.number(g,&format!("logo-{key}"),&format!("Logo colour · {label}"),&format!("/rgb/{i}"),(0,255),""); }
                let g = this.group(controller, Query::query(Command::ControllerMode), Command::SetControllerMode,"s")?;
                let mode_group = g;
                this.choice(g,"mode","Controller mode","/m",&[(0,"Xbox (XInput)"),(1,"Switch"),(2,"PS4"),(3,"PS5"),(4,"Mobile touch")]);
                this.help("The controller reconnects as the chosen type.");
                let g = this.group(controller, Query::query(Command::PollingRate), Command::SetPollingRate,"s")?;
                this.choice(g,"polling","Report rate","",&[(125,"125 Hz"),(250,"250 Hz"),(500,"500 Hz"),(1000,"1000 Hz")]);
                let g = this.group(controller, Query::query(Command::StepSize), Command::SetStepSize,"s")?;
                this.choice(g,"precision","Step precision","",&[(0,"Adaptive"),(32,"32 steps"),(64,"64 steps"),(128,"128 steps"),(256,"256 steps")]);
                let g = this.group(controller, Query::query(Command::DualOutput), Command::SetDualOutput,"s")?;
                this.field(g,"dual-output","Dual output","",Kind::Toggle);
                let g = this.group(controller, Query::query(Command::JoystickDpad), Command::SetJoystickDpad,"s")?;
                this.field(g,"dpad-swap","Swap D-pad and stick","/std",Kind::Toggle);
                this.field(g,"dpad-lock","D-pad diagonal lock","/ddl",Kind::Toggle);
                for (key,label) in [("ljod","Left"),("rjod","Right")] { this.choice(g,&format!("{}-directions",label.to_lowercase()),&format!("{label} stick directions"),&format!("/{key}"),&[(0,"Default"),(1,"Four-way"),(2,"Eight-way")]); }
                let connection = this.groups[mode_group].value.get("l").and_then(Value::as_u64);
                let label = match connection.filter(|n| *n <= u64::from(u8::MAX)) {
                    Some(0) => "Connection type · Bluetooth",
                    Some(1) => "Connection type · USB wired",
                    Some(_) => "Connection type · Dongle",
                    None => "Connection type · not reported",
                };
                this.field(mode_group,"connection-type",label,"",Kind::Note("Read only; connection type is reported separately from controller mode."));
            }
            Section::Profiles => {
                let g = this.group(controller, Query::profile_summary(view.profile).map_err(failed)?, Command::SetProfileName,"s")?;
                let version = if this.groups[g].value.get("d2").is_some() { "d2" } else { "d1" };
                this.field(g,"profile-name","Profile name",&format!("/{version}/n"),Kind::Text);
                this.field(g,"profile-activate","Activate profile",&format!("/{version}/a"),Kind::Activate);
                if view.sub != 0 {
                    let entries = this.groups[g].value.pointer("/d2/cfg").and_then(Value::as_array);
                    let matches: Vec<_> = entries.into_iter().flatten().enumerate().filter(|(_, entry)| entry["c"] == view.sub).map(|(i,_)| i).collect();
                    let valid = if let [i] = matches.as_slice() {
                        let entry = &this.groups[g].value["d2"]["cfg"][*i];
                        let mut seen = std::collections::BTreeSet::new();
                        entry["ko"].as_u64().is_some_and(|v| v <= 1) && entry["at"].as_u64().is_some_and(|v| v <= 1)
                            && entry["k"].as_array().is_some_and(|keys| keys.len() <= 32 && keys.iter().all(|v|
                                v.as_u64().filter(|n| (1..=32).contains(n)).is_some_and(|n| seen.insert(n))))
                    } else { false };
                    if valid {
                        let path = format!("/d2/cfg/{}",matches[0]);
                        this.field(g,"sub-keep","Keep original input",&format!("{path}/ko"),Kind::Toggle);
                        this.choice(g,"sub-activation","Activate configuration",&format!("{path}/at"),&[(0,"Hold"),(1,"Toggle")]);
                        let keys = this.groups[g].value.pointer(&format!("{path}/k")).and_then(Value::as_array).unwrap().len();
                        for i in 0..keys + usize::from(keys < 2) {
                            this.field(g,&format!("sub-key-{i}"),&format!("Activation button {}",i + 1),&format!("{path}/k"),Kind::SubKey(i));
                        }
                    } else {
                        this.field(g,"sub-activation-unavailable","Sub-configuration activation","",Kind::Note("The controller did not report a unique, complete activation configuration."));
                    }
                }
            }
            Section::Buttons => {
                let macros = macros(controller)?;
                let g = this.group(controller, Query::buttons(profile,&[view.button]).map_err(failed)?,Command::SetButton,"w")?;
                let items = this.groups[g].value["d1"].as_array().ok_or_else(|| failed("controller returned invalid button mappings"))?;
                let i = items.iter().position(|v| v["k"] == view.button).ok_or_else(|| failed("controller omitted the requested button"))?;
                this.action(g,&format!("/d1/{i}"),view.action,true,&macros)?;
            }
            Section::Macros => {
                let g = this.group(controller, Query::query(Command::MacroSummary), Command::DeleteMacro,"n")?;
                let macros = summary(&this.groups[g].value)?;
                let names = macros.iter().map(|(name,_)| name.clone()).collect();
                let mut delete = None;
                if macros.is_empty() { view.button = 1; view.step = 1; }
                else {
                    view.button = view.button.clamp(1, u8::try_from(macros.len()).unwrap_or(u8::MAX));
                    this.items = macros.iter().enumerate().map(|(i,(name,_))| json!({"id":(i + 1).to_string(),"label":name})).collect();
                    let name = macros[usize::from(view.button) - 1].0.clone();
                    let detail = this.group(controller, Query::macro_detail(&name).map_err(failed)?, Command::WriteMacro,"w")?;
                    if let Some(steps) = steps(&this.groups[detail].value) {
                        this.steps = u16::try_from(steps.len()).map_err(failed)?;
                        view.step = view.step.clamp(1, this.steps);
                        let i = usize::from(view.step - 1);
                        let at = |n: usize| format!("/d/{}", i * 5 + n);
                        this.choice(detail,"step-input","Input",&at(0),&[(0,"Controller"),(1,"Keyboard"),(2,"Mouse")]);
                        if steps[i].device == MacroDevice::Gamepad && KEYS.iter().any(|(id,_)| *id == i64::from(steps[i].key)) { this.choice(detail,"step-key","Button",&at(1),KEYS); }
                        else { this.number(detail,"step-key","Key code",&at(1),(0,255),""); this.help("A keyboard usage code, or mouse button 1 to 5."); }
                        this.field(detail,"step-interval","Interval",&at(2),Kind::Number(0,u16::MAX.into(),10,"ms"));
                        this.field(detail,"step-duration","Duration",&at(3),Kind::Number(0,u16::MAX.into(),10,"ms"));
                        this.field(detail,"step-insert","Copy this step","",Kind::Insert(i));
                        if steps.len() > 1 { this.field(detail,"step-remove","Remove this step","",Kind::Remove(i)); }
                        delete = Some(name);
                    } else {
                        view.step = 1;
                        this.field(detail,"macro-unavailable","Macro","",Kind::Note("Advanced or unrecognised macros are preserved without editing."));
                    }
                }
                this.field(g,"macro-new","New macro","",Kind::NewMacro(names));
                if let Some(name) = delete { this.field(g,"delete-macro","Delete macro","",Kind::Delete(name)); }
            }
            Section::Sticks | Section::Curves | Section::Directions => {
                let macros = if view.section == Section::Directions { macros(controller)? } else { vec![] };
                let g = this.group(controller, Query::joystick(profile,view.side).map_err(failed)?,Command::SetJoystick,"w")?;
                match view.section {
                    Section::Sticks => {
                        this.field(g,"centre-x","Centre X","",Kind::Note("The centre scale depends on the exact model and firmware; the reported value is preserved."));
                        this.field(g,"centre-y","Centre Y","",Kind::Note("The centre scale depends on the exact model and firmware; the reported value is preserved."));
                        this.choice(g,"shape","Stick shape","/d1/ci",&[(0,"Rounded rectangle"),(1,"Circle")]);
                        this.field(g,"mouse","Mouse mapping","/d1/mm",Kind::Toggle);
                        this.alias(g,"mouse-mode","Mouse operation","mo","mom",Kind::Choice(&[(0,"Vector"),(1,"Fixed")]));
                        this.alias(g,"invert-x","Invert X","mx","xr",Kind::Toggle);
                        this.alias(g,"invert-y","Invert Y","ry","yr",Kind::Toggle);
                        this.field(g,"release-cut","Release cut","/d1/rjc",Kind::Toggle);
                        this.number(g,"mouse-sensitivity","Mouse sensitivity","/d1/mse",(0,100),"%");
                        this.number(g,"anti-shake","Anti-shake","/d1/asf",(-10,10),"");
                        for (key,label) in [("cpl","Left"),("cpu","Up"),("cpr","Right"),("cpd","Down")] { this.number(g,key,&format!("{label} binding radius"),&format!("/d1/{key}"),(0,100),"%"); }
                    }
                    Section::Curves => {
                        this.field(g,"curve-format","Stick curve points","",Kind::Note("Point editing is unavailable until the exact model and firmware scale are identified. Reported points are preserved."));
                        if this.groups[g].value.pointer("/d1/ct").and_then(Value::as_u64) == Some(1) {
                            this.choice(g,"response","Smooth response","/d1/rt",&[(0,"Linear"),(1,"Convex"),(2,"Concave"),(3,"S-curve")]);
                            if matches!(this.groups[g].value.pointer("/d1/rt").and_then(Value::as_u64),Some(1..=3)) {
                                this.number(g,"curvature","Curvature","/d1/rc",(0,100),"%");
                            } else {
                                this.field(g,"curvature","Curvature","",Kind::Note("Curvature is unavailable for a linear or unreported response."));
                            }
                        } else {
                            this.field(g,"response","Smooth response","",Kind::Note("The controller did not report a smooth stick curve."));
                            this.field(g,"curvature","Curvature","",Kind::Note("The controller did not report a smooth stick curve."));
                        }
                    }
                    Section::Directions => {
                        let key = ["tkl","tku","tkr","tkd"][usize::from(view.button - 1)];
                        this.action(g,&format!("/d1/{key}"),view.action,true,&macros)?;
                    }
                    _ => unreachable!(),
                }
            }
            Section::Triggers => {
                let g = this.group(controller, Query::profile_settings(Command::Trigger,profile).map_err(failed)?,Command::SetTrigger,"w")?;
                for (suffix,label) in [("l","Left"),("r","Right")] {
                    this.choice(g,&format!("mode-{suffix}"),&format!("{label} trigger mode"),&format!("/d1/mo{suffix}"),&[(0,"Adaptive"),(1,"Rapid fire"),(2,"Linear")]);
                    for (i,end) in [(0,"Inner"),(1,"Outer")] { this.number(g,&format!("deadzone-{suffix}-{i}"),&format!("{label} trigger · {end} dead zone"),&format!("/d1/dz{suffix}/{i}"),(0,100),"%"); }
                    this.number(g,&format!("threshold-{suffix}"),&format!("{label} vibration threshold"),&format!("/d1/vt{suffix}"),(0,100),"%");
                }
            }
            Section::Motion => {
                let g = this.group(controller, Query::profile_settings(Command::Motion,profile).map_err(failed)?,Command::SetMotion,"w")?;
                this.choice(g,"motion","Motion control","/d1/sj",&[(0,"Off"),(1,"Left stick"),(2,"Right stick"),(3,"Buttons"),(4,"Mouse")]);
                this.choice(g,"sensor","Sensor","/d1/st",&[(0,"Accelerometer"),(1,"Gyroscope")]);
                this.choice(g,"activation","Activation","/d1/tm",&[(0,"Off"),(1,"Toggle with the trigger keys"),(2,"While the trigger keys are held"),(3,"Always on"),(4,"Always on, paused while held")]);
                if let Some(keys) = this.groups[g].value["d1"]["tk"].as_array().map(Vec::len) {
                    if keys == 0 {
                        this.field(g,"trigger-keys","Trigger keys","",Kind::Note("No trigger keys were reported."));
                    } else {
                        this.pages = u16::try_from(keys.div_ceil(PAGE_SIZE)).map_err(failed)?;
                        view.page = view.page.clamp(1, this.pages);
                        let start = usize::from(view.page - 1) * PAGE_SIZE;
                        for i in start..(start + PAGE_SIZE).min(keys) { this.field(g,&format!("trigger-key-{i}"),&format!("Trigger key {}",i + 1),&format!("/d1/tk/{i}"),Kind::ReportedKey); }
                    }
                } else {
                    this.field(g,"trigger-keys","Trigger keys","",Kind::Note("The controller did not report a valid trigger-key list."));
                }
                this.choice(g,"mouse-stick","Mouse-linked stick","/d1/mlj",&[(0,"Off"),(1,"Left stick"),(2,"Right stick")]);
                this.choice(g,"response","Response curve","/d1/rt",&[(0,"Linear"),(1,"Convex"),(2,"Concave"),(3,"S-curve")]);
                if matches!(this.groups[g].value.pointer("/d1/rt").and_then(Value::as_u64),Some(1..=3)) {
                    this.number(g,"curvature","Curvature","/d1/rc",(0,100),"%");
                } else {
                    this.field(g,"curvature","Curvature","",Kind::Note("Curvature is unavailable for a linear or unreported response."));
                }
                this.number(g,"sensitivity","Sensitivity","/d1/se",(0,100),"%");
                this.number(g,"ratio","Horizontal to vertical ratio","/d1/hvr",(0,100),"%");
                this.choice(g,"horizontal-axis","Horizontal axis","/d1/had",&[(0,"Yaw"),(1,"Roll"),(2,"Yaw and roll")]);
                this.field(g,"reverse-swing","Reverse horizontal swing","/d1/hsr",Kind::Toggle);
                this.field(g,"reverse-roll","Reverse horizontal roll","/d1/hrr",Kind::Toggle);
                this.field(g,"reverse-vertical","Reverse vertical","/d1/vr",Kind::Toggle);
                if let Some(thresholds) = this.groups[g].value["d1"]["ec"].as_array().cloned() {
                    let mut seen = [false; 3];
                    let mut omitted = false;
                    for (i,entry) in thresholds.iter().enumerate() {
                        let Some(index) = entry["et"].as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < seen.len()) else { omitted = true; continue };
                        if std::mem::replace(&mut seen[index], true) { omitted = true; continue; }
                        let axis = ["Yaw","Roll","Pitch"][index];
                        this.number(g,&format!("acceleration-{i}"),&format!("Gesture threshold · {axis} acceleration"),&format!("/d1/ec/{i}/acc"),(0,100),"%");
                        this.number(g,&format!("angle-{i}"),&format!("Gesture threshold · {axis} angle"),&format!("/d1/ec/{i}/ag"),(0,100),"%");
                    }
                    if omitted { this.field(g,"other-thresholds","Other gesture thresholds","",Kind::Note("Unknown or repeated axes are preserved without editing.")); }
                } else {
                    this.field(g,"thresholds","Gesture thresholds","",Kind::Note("The controller did not report valid gesture thresholds."));
                }
            }
            Section::Gestures => {
                let macros = macros(controller)?;
                let g = this.group(controller, Query::profile_settings(Command::Motion,profile).map_err(failed)?,Command::SetMotion,"w")?;
                let entries = this.groups[g].value["d1"]["el"].as_array().ok_or_else(|| failed("controller returned invalid motion gestures"))?;
                let mut counts = [0usize; 6];
                for entry in entries { if let Some(event) = entry["e"].as_u64().filter(|n| *n < 6) { counts[event as usize] += 1; } }
                let events: Vec<(usize, u8)> = entries.iter().enumerate().filter_map(|(i,entry)| {
                    entry["e"].as_u64().filter(|e| *e < 6 && counts[*e as usize] == 1).map(|e| (i, e as u8 + 1))
                }).collect();
                let omitted = entries.len() != events.len();
                if let Some(&(i,button)) = events.iter().find(|(_,button)| *button == view.button).or(events.first()) {
                    view.button = button;
                    this.items = events.iter().map(|(_,button)| json!({"id":button.to_string(),"label":GESTURES[usize::from(button - 1)]})).collect();
                    this.action(g,&format!("/d1/el/{i}"),view.action,false,&macros)?;
                }
                if omitted || events.is_empty() {
                    this.field(g,"other-gestures","Other motion gestures","",Kind::Note("Unknown or repeated gesture events are preserved without editing."));
                }
            }
            Section::Rumble => {
                let g = this.group(controller, Query::profile_settings(Command::Vibration,profile).map_err(failed)?,Command::SetVibration,"w")?;
                for (suffix,label) in [("l","Left"),("r","Right")] {
                    this.field(g,&format!("enable-{suffix}"),&format!("{label} rumble"),&format!("/d1/en{suffix}"),Kind::Toggle);
                    for (i,end) in [(0,"Start"),(1,"Maximum")] { this.number(g,&format!("range-{suffix}-{i}"),&format!("{label} grip · {end}"),&format!("/d1/gr{suffix}/{i}"),(0,100),"%"); }
                }
            }
        }
        Ok(this)
    }
    /// The writes that make a field's new value, in order, and the macro they leave behind.
    pub fn changes(&self, field: &Field, value: Value, view: &View) -> Result<(Vec<Change>, Option<String>), Response> {
        let group = &self.groups[field.group];
        let written = match group.read.command() { Command::MacroSummary => value.as_str(), Command::MacroDetail => value["n"].as_str(), _ => None }.map(str::to_owned);
        let change = group.change(value, &field.key, view)?;
        if !matches!(group.read.command(), Command::MacroDetail) { return Ok((vec![change], written)); }
        // c172 appends and the first macro of a name wins, so an edited macro is deleted before it is written again.
        let summary = self.groups.iter().find(|group| matches!(group.read.command(), Command::MacroSummary)).ok_or_else(|| failed("controller macro list was not read"))?;
        let name = group.value["n"].as_str().ok_or_else(|| failed("controller omitted the macro name"))?;
        Ok((vec![Change::delete_macro(name, &summary.value).map_err(failed)?, change], written))
    }
    fn group<T: Transport>(&mut self, controller: &mut Controller<T>, read: Query, set: Command, argument: &'static str) -> Result<usize, Response> {
        let value = controller.read(&read).map_err(failed)?.result;
        let args = read.value();
        for key in ["b","c","d"] { if let Some(expected) = args["g"].get(key) { if value.get(key) != Some(expected) { return Err(failed("controller returned settings for a different profile or side")); } } }
        let index = self.groups.len(); self.groups.push(Group { read, value, set, argument }); Ok(index)
    }
    fn field(&mut self, group: usize, key: &str, label: &str, path: &str, kind: Kind) {
        let mut field = Field { group, key:key.into(), label:label.into(), path:path.into(), kind, help: None };
        if field.display(&self.groups[group].value).is_err() {
            field.kind = Kind::Note("Not reported or unsupported by this controller.");
        }
        self.fields.push(field);
    }
    fn alias(&mut self, group: usize, key: &str, label: &str, first: &str, second: &str, kind: Kind) {
        let object = self.groups[group].value.get("d1").and_then(Value::as_object);
        let present = |name| object.is_some_and(|v| v.contains_key(name));
        match (present(first), present(second)) {
            (true, false) => self.field(group,key,label,&format!("/d1/{first}"),kind),
            (false, true) => self.field(group,key,label,&format!("/d1/{second}"),kind),
            _ => self.field(group,key,label,"",Kind::Note("Not reported, or both field spellings were reported; the original values are preserved.")),
        }
    }
    fn help(&mut self, text: &'static str) { if let Some(field) = self.fields.last_mut() { field.help = Some(text); } }
    fn number(&mut self, group: usize, key: &str, label: &str, path: &str, range: (i64,i64), unit: &'static str) { self.field(group,key,label,path,Kind::Number(range.0,range.1,1,unit)); }
    fn choice(&mut self, group: usize, key: &str, label: &str, path: &str, options: &'static [(i64,&'static str)]) { self.field(group,key,label,path,Kind::Choice(options)); }
    fn action(&mut self, group: usize, path: &str, side: u8, typed: bool, macros: &[(String, u16)]) -> Result<(), Response> {
        if typed { self.choice(group,"action-type","Button action",&format!("{path}/a"),&[(0,"Press"),(1,"Press and release")]); }
        let path = format!("{path}/{}",if side == 0 { "dn" } else { "up" });
        let action = self.groups[group].value.pointer(&path);
        let target = action.and_then(|a| a["t"].as_u64());
        let repeat = action.and_then(|a| a["r"].as_u64());
        let repeats = action.is_some_and(|a| matches!(a["mr"].as_u64(), Some(3 | 4)));
        let valid_keys = action.and_then(|a| a["v"].as_array()).is_some_and(|keys|
            keys.len() == 4 && keys.iter().all(|v| v.as_u64().is_some_and(|n| n <= u64::from(u8::MAX))));
        if !valid_keys {
            self.field(group,"target","Mapping target","",Kind::Note("The controller did not report a valid four-slot action."));
            return Ok(());
        }
        match (target, repeat) {
            (Some(target @ (0|2|3|4)), Some(0|2)) => {
                self.field(group,"target","Mapping target",&path,Kind::Target(macros.to_vec()));
                for i in 0..4 {
                    let key = format!("key-{i}"); let label = format!("Mapped {} {}", if target == 3 { "key" } else if target == 4 { "mouse button" } else { "button" },i+1);
                    let path = format!("{path}/v/{i}");
                    match target {
                        3 => self.field(group,&key,&label,&path,Kind::Keyboard),
                        4 => self.choice(group,&key,&label,&path,MOUSE),
                        _ => self.choice(group,&key,&label,&path,KEYS),
                    }
                }
                self.choice(group,"turbo","Repeat mode",&format!("{path}/r"),&[(0,"Normal"),(2,"Turbo")]);
                self.choice(group,"turbo-rate","Turbo rate",&format!("{path}/sp"),&[(0,"Off"),(5,"5 / second"),(10,"10 / second"),(20,"20 / second"),(30,"30 / second")]);
            }
            (Some(1), Some(1)) => {
                self.field(group,"target","Mapping target",&path,Kind::Target(macros.to_vec()));
                self.field(group,"macro","Macro",&path,Kind::Macro(macros.to_vec()));
                self.choice(group,"macro-playback","Playback",&format!("{path}/mr"),&[(0,"Off"),(1,"Tap: play once"),(2,"Hold: play once"),(3,"Hold: repeat"),(4,"Tap: repeat")]);
                // The firmware reads the repeat interval only while a macro repeats; the vendor app offers 0–36000 ms.
                if repeats { self.field(group,"macro-interval","Repeat interval",&format!("{path}/ri"),Kind::Number(0,36000,10,"ms")); }
            }
            _ => self.field(group,"target","Mapping target","",Kind::Note("This action encoding is not supported; it is preserved without editing.")),
        }
        Ok(())
    }
    pub fn display(&self, view: &View, profiles: u8) -> Result<(Value, Value), Response> {
        let mut fields = Vec::new(); let mut values = serde_json::Map::new();
        let mut selector = |key: &str, label: &str, options: Vec<Value>, value: String| { fields.push(json!({"key":key,"label":label,"type":"choice","options":options,"default":value})); values.insert(key.into(),json!(value)); };
        selector("view-section","Settings",SECTIONS.iter().map(|(id,label)| json!({"id":id,"label":label})).collect(),serde_json::to_value(view.section).map_err(failed)?.as_str().unwrap().into());
        if !matches!(view.section,Section::General|Section::Macros) {
            selector("view-profile","Profile",(1..=profiles).map(|n| json!({"id":n.to_string(),"label":format!("Profile {n}")})).collect(),view.profile.to_string());
            selector("view-sub","Configuration",[(0,"Main"),(1,"Sub 1"),(2,"Sub 2")].into_iter().map(|(n,label)| json!({"id":n.to_string(),"label":label})).collect(),view.sub.to_string());
        }
        if matches!(view.section,Section::Sticks|Section::Curves|Section::Directions) {
            selector("view-side","Stick",["Left","Right"].into_iter().enumerate().map(|(i,label)| json!({"id":i.to_string(),"label":label})).collect(),view.side.to_string());
        }
        if matches!(view.section,Section::Directions|Section::Buttons) || (view.section == Section::Gestures && !self.items.is_empty()) {
            selector("view-action","Action to edit",["Press","Release"].into_iter().enumerate().map(|(i,label)| json!({"id":i.to_string(),"label":label})).collect(),view.action.to_string());
        }
        let items = |options: &[(i64, &str)]| options.iter().map(|(i,label)| json!({"id":i.to_string(),"label":label})).collect();
        match view.section {
            Section::Buttons => selector("view-button","Button",items(BUTTONS),view.button.to_string()),
            Section::Directions => selector("view-button","Direction",items(&[(1,"Left"),(2,"Up"),(3,"Right"),(4,"Down")]),view.button.to_string()),
            Section::Gestures | Section::Macros if !self.items.is_empty() => {
                let selected = view.button.to_string();
                let at = self.items.iter().position(|item| item["id"].as_str() == Some(selected.as_str())).unwrap_or(0);
                selector("view-button",if view.section == Section::Macros { "Macro" } else { "Gesture" },self.items[window(self.items.len(),at)].to_vec(),selected);
            }
            _ => {}
        }
        if self.steps > 0 {
            let range = window(usize::from(self.steps),usize::from(view.step - 1));
            selector("view-step","Step",range.map(|i| { let n = i + 1; json!({"id":n.to_string(),"label":format!("Step {n} of {}",self.steps)}) }).collect(),view.step.to_string());
        }
        if self.pages > 1 {
            fields.push(json!({"key":"view-page","label":"Trigger-key page","type":"number","min":1,"max":self.pages,"step":1,"default":view.page}));
            values.insert("view-page".into(),json!(view.page));
        }
        for field in &self.fields { let (definition,value) = field.display(&self.groups[field.group].value)?; if let Some(value) = value { values.insert(field.key.clone(),value); } fields.push(definition); }
        Ok((json!({"sections":[{"title":"Controller","fields":fields}]}),Value::Object(values)))
    }
}
fn subkey_allowed(key: i64) -> bool { matches!(key, 1..=12 | 14..=20 | 24..=27) }

fn window(len: usize, selected: usize) -> std::ops::Range<usize> {
    let count = len.min(OPTION_WINDOW);
    let start = selected.saturating_sub(count / 2).min(len - count);
    start..start + count
}
fn text(value: &str, max: usize) -> bool { value.chars().count() <= max && !value.chars().any(char::is_control) }
/// The c170 list as (name, step count).
fn summary(value: &Value) -> Result<Vec<(String, u16)>, Response> {
    value.as_array().and_then(|entries| entries.iter().map(|entry| Some((entry["n"].as_str()?.to_owned(), u16::try_from(entry["s"].as_u64()?).ok()?))).collect::<Option<Vec<_>>>())
        .ok_or_else(|| failed("controller returned an invalid macro list"))
}
pub(super) fn macros<T: Transport>(controller: &mut Controller<T>) -> Result<Vec<(String, u16)>, Response> {
    summary(&controller.read(&Query::query(Command::MacroSummary)).map_err(failed)?.result)
}
/// A classic macro's steps; `None` for advanced or unrecognised layouts, which are never rewritten.
fn steps(detail: &Value) -> Option<Vec<MacroStep>> {
    let data = detail["d"].as_array()?;
    if detail["t"] != 0 || data.is_empty() || detail["s"].as_u64()? != data.len() as u64 / 5 || data.len() % 5 != 0 { return None; }
    data.chunks(5).map(|step| {
        let n = |i: usize| step[i].as_u64();
        let device = match n(0)? { 0 => MacroDevice::Gamepad, 1 => MacroDevice::Keyboard, 2 => MacroDevice::Mouse, _ => return None };
        if n(4)? != 1 { return None; }
        Some(MacroStep { device, key: u8::try_from(n(1)?).ok()?, interval: u32::try_from(n(2)?).ok()?, duration: u32::try_from(n(3)?).ok()? })
    }).collect()
}
