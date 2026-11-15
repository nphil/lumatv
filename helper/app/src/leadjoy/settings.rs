use super::{View, Section, failed};
use crate::http::Response;
use leadjoy_config::{Change, Command, Controller, Profile, Request as Query, Transport, settings::{ButtonAction, ControllerMode, MacroDevice, MacroStep}};
use serde_json::{Value, json};

pub(super) const SECTIONS: &[(&str, &str)] = &[("general","General and logo"),("profiles","Profiles"),("buttons","Buttons and turbo"),("macros","Macros"),("sticks","Sticks"),("curves","Stick curves"),("directions","Stick direction bindings"),("triggers","Triggers"),("motion","Motion"),("gestures","Motion gestures"),("rumble","Rumble")];
pub(super) const BUTTONS: &[(i64, &str)] = &[(26,"R4 / M3"),(27,"L4 / M4"),(24,"R5 / M1"),(25,"L5 / M2"),(1,"A"),(2,"B"),(4,"X"),(5,"Y"),(14,"Left stick click"),(15,"Right stick click"),(17,"D-pad up"),(18,"D-pad down"),(19,"D-pad left"),(20,"D-pad right"),(7,"LB"),(8,"RB"),(9,"LT"),(10,"RT"),(11,"Select"),(16,"Capture"),(12,"Start"),(3,"C")];
const KEYS: &[(i64, &str)] = &[(0,"None"),(1,"A"),(2,"B"),(3,"C"),(4,"X"),(5,"Y"),(6,"Z"),(7,"LB"),(8,"RB"),(9,"LT"),(10,"RT"),(11,"Select"),(12,"Start"),(13,"Home"),(14,"Left stick click"),(15,"Right stick click"),(16,"Capture"),(17,"D-pad up"),(18,"D-pad down"),(19,"D-pad left"),(20,"D-pad right"),(21,"Menu"),(22,"Mode"),(23,"FN"),(24,"R5 / M1"),(25,"L5 / M2"),(26,"R4 / M3"),(27,"L4 / M4"),(32,"Power")];
const GESTURES: [&str; 6] = ["Yaw left","Yaw right","Roll left","Roll right","Pitch down","Pitch up"];
// The protocol documents no name limit; a name the firmware shortens fails the read-back rather than saving under another name.
const MACRO_NAME: usize = 32;
// What a new macro starts as; the vendor converter has no default macro to copy.
const FIRST_STEP: MacroStep = MacroStep { device: MacroDevice::Gamepad, key: 1, interval: 0, duration: 100 };
const RANGE: &str = "value is outside this setting's supported range";

pub(super) enum Kind {
    Number(i64, i64, i64, &'static str), Toggle, Choice(&'static [(i64, &'static str)]), Text, Activate,
    /// What an action does. Switching to or from a macro rewrites the whole action, as the vendor converter does.
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
pub(super) struct Settings { pub groups: Vec<Group>, pub fields: Vec<Field>, items: Vec<Value>, steps: u16 }
impl Group {
    pub fn change(&self, value: Value, key: &str, view: &View) -> Result<Change, Response> {
        match self.read.command() {
            Command::Lighting => {
                let mut fields = serde_json::Map::new(); fields.insert("a".into(), json!(1)); fields.insert("s".into(), value.clone());
                Query::new(Command::Lighting, fields).map(|write| Change { write, read: self.read.clone(), expected: value })
            }
            Command::ProfileSummary => {
                let version = if value.get("d2").is_some() { "d2" } else { "d1" };
                if key == "profile-activate" {
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
            Kind::Text => *slot = value.as_str().filter(|s| text(s, 128)).ok_or(RANGE)?.into(),
            Kind::Activate | Kind::Delete(_) if !value.is_null() => return Err(RANGE),
            Kind::Activate => *slot = 1.into(),
            Kind::Delete(_) => {}
            Kind::Target(macros) => {
                let target = value.as_str().and_then(|s| s.parse::<u8>().ok()).ok_or(RANGE)?;
                let current = slot["t"].as_u64();
                if current == Some(target.into()) { return Ok(next); }
                let action = match (target, macros.first()) {
                    (0 | 2, _) if current != Some(1) => { slot["t"] = target.into(); return Ok(next); }
                    (0 | 2, _) => ButtonAction::mapping(target, [0; 4], 0),
                    (1, Some((name, steps))) => ButtonAction::macro_action(name.clone(), 1, *steps, slot["ri"].as_u64().and_then(|n| u16::try_from(n).ok()).unwrap_or(0)),
                    _ => return Err(RANGE),
                }.map_err(|_| RANGE)?;
                if let (Some(object), Ok(Value::Object(fields))) = (slot.as_object_mut(), serde_json::to_value(action)) { object.extend(fields); }
            }
            Kind::Macro(macros) => {
                let (name, steps) = value.as_str().and_then(|s| macros.iter().find(|(name,_)| name == s)).ok_or(RANGE)?;
                slot["n"] = name.as_str().into(); slot["ms"] = (*steps).into();
            }
            Kind::NewMacro(names) => return value.as_str().filter(|s| !s.is_empty() && text(s, MACRO_NAME) && !names.iter().any(|name| name == s)).map(Value::from)
                .ok_or("a new macro needs a name that no other macro uses"),
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
            Kind::Target(macros) => {
                let current = value["t"].as_u64().ok_or_else(unsupported)?;
                let mut options = vec![("0".into(), "Original input".into()), ("2".into(), "Gamepad".into())];
                if !macros.is_empty() || current == 1 { options.push(("1".into(), "Macro".into())); }
                choice(options, current.to_string())?
            }
            Kind::Macro(macros) => {
                let current = value["n"].as_str().ok_or_else(unsupported)?;
                let mut options: Vec<(String, String)> = macros.iter().map(|(name,_)| (name.clone(), name.clone())).collect();
                if !macros.iter().any(|(name,_)| name == current) { options.push((current.into(), format!("{current} (not on the controller)"))); }
                choice(options, current.into())?
            }
            Kind::Number(min,max,step,unit) => {
                let n = value.as_i64().filter(|n| (*min..=*max).contains(n)).ok_or_else(unsupported)?;
                field["type"] = json!("number"); field["min"] = json!(min); field["max"] = json!(max); field["step"] = json!(step); field["default"] = json!(n);
                if !unit.is_empty() { field["unit"] = json!(unit); }
                Some(json!(n))
            }
            Kind::Toggle => {
                let n = value.as_u64().filter(|n| *n <= 1).ok_or_else(|| failed(format!("controller returned an invalid {} switch", self.key)))?;
                field["type"] = json!("toggle"); field["default"] = json!(n == 1); Some(json!(n == 1))
            }
            Kind::Text => { field["type"] = json!("text"); field["max_chars"] = json!(128); Some(json!(value.as_str().ok_or_else(|| failed("controller returned an invalid profile name"))?)) }
            Kind::NewMacro(_) => {
                field["type"] = json!("text"); field["max_chars"] = json!(MACRO_NAME);
                field["help"] = json!("Name a new macro. It starts as one 100 ms press of A."); Some(json!(""))
            }
            Kind::Activate => { field["type"] = json!("action"); field["help"] = json!(if value == 1 { "This profile is active." } else { "Make this the controller's active profile." }); None }
            Kind::Insert(_) | Kind::Remove(_) => { field["type"] = json!("action"); None }
            Kind::Delete(name) => { field["type"] = json!("action"); field["help"] = json!(format!("The controller forgets the macro “{name}”.")); None }
        };
        Ok((field, shown))
    }
}
impl Settings {
    /// Selectors that point past what the controller has are moved onto what it has.
    pub fn read<T: Transport>(controller: &mut Controller<T>, view: &mut View) -> Result<Self, Response> {
        let mut this = Self { groups: vec![], fields: vec![], items: vec![], steps: 0 };
        let profile = Profile::new(view.profile, view.sub).map_err(failed)?;
        match view.section {
            Section::General => {
                let g = this.group(controller, Query::lighting(), Command::Lighting, "s")?;
                let lighting: leadjoy_config::Lighting = serde_json::from_value(this.groups[g].value.clone()).map_err(failed)?;
                if lighting.brightness > 100 { return Err(failed("controller returned an invalid logo brightness")); }
                this.number(g,"logo-brightness","Logo brightness","/brt",(0,100),"%");
                for (i,key,label) in [(0,"red","Red"),(1,"green","Green"),(2,"blue","Blue")] { this.number(g,&format!("logo-{key}"),&format!("Logo colour · {label}"),&format!("/rgb/{i}"),(0,255),""); }
                let g = this.group(controller, Query::query(Command::ControllerMode), Command::SetControllerMode,"s")?;
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
            }
            Section::Profiles => {
                let g = this.group(controller, Query::profile_summary(view.profile).map_err(failed)?, Command::SetProfileName,"s")?;
                let version = if this.groups[g].value.get("d2").is_some() { "d2" } else { "d1" };
                this.field(g,"profile-name","Profile name",&format!("/{version}/n"),Kind::Text);
                this.field(g,"profile-activate","Activate profile",&format!("/{version}/a"),Kind::Activate);
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
                    // Advanced (t=1) and unrecognised macros are kept as they are: only removable.
                    if let Some(steps) = steps(&this.groups[detail].value) {
                        this.steps = u16::try_from(steps.len()).map_err(failed)?;
                        view.step = view.step.clamp(1, this.steps);
                        let i = usize::from(view.step - 1);
                        let at = |n: usize| format!("/d/{}", i * 5 + n);
                        this.choice(detail,"step-input","Input",&at(0),&[(0,"Controller"),(1,"Keyboard"),(2,"Mouse")]);
                        if steps[i].device == MacroDevice::Gamepad && KEYS.iter().any(|(id,_)| *id == i64::from(steps[i].key)) { this.choice(detail,"step-key","Button",&at(1),KEYS); }
                        else { this.number(detail,"step-key","Key code",&at(1),(0,255),""); this.help("A keyboard usage code, or mouse button 1 to 5."); }
                        this.field(detail,"step-interval","Interval",&at(2),Kind::Number(0,u32::MAX.into(),10,"ms"));
                        this.field(detail,"step-duration","Duration",&at(3),Kind::Number(0,u32::MAX.into(),10,"ms"));
                        this.field(detail,"step-insert","Copy this step","",Kind::Insert(i));
                        if steps.len() > 1 { this.field(detail,"step-remove","Remove this step","",Kind::Remove(i)); }
                    } else { view.step = 1; }
                    delete = Some(name);
                }
                this.field(g,"macro-new","New macro","",Kind::NewMacro(names));
                if let Some(name) = delete { this.field(g,"delete-macro","Delete macro","",Kind::Delete(name)); }
            }
            Section::Sticks | Section::Curves | Section::Directions => {
                let macros = if view.section == Section::Directions { macros(controller)? } else { vec![] };
                let g = this.group(controller, Query::joystick(profile,view.side).map_err(failed)?,Command::SetJoystick,"w")?;
                match view.section {
                    Section::Sticks => {
                        this.number(g,"centre-x","Centre X · thousandths","/d1/co/0",(-100,100),"");
                        this.number(g,"centre-y","Centre Y · thousandths","/d1/co/1",(-100,100),"");
                        this.choice(g,"shape","Stick shape","/d1/ci",&[(0,"Rounded rectangle"),(1,"Circle")]);
                        this.field(g,"mouse","Mouse mapping","/d1/mm",Kind::Toggle);
                        this.choice(g,"mouse-mode","Mouse operation","/d1/mo",&[(0,"Vector"),(1,"Fixed")]);
                        this.field(g,"invert-x","Invert X","/d1/mx",Kind::Toggle);
                        this.field(g,"invert-y","Invert Y","/d1/ry",Kind::Toggle);
                        this.number(g,"anti-shake","Anti-shake","/d1/asf",(-10,10),"");
                        for (key,label) in [("cpl","Left"),("cpu","Up"),("cpr","Right"),("cpd","Down")] { this.number(g,key,&format!("{label} binding radius"),&format!("/d1/{key}"),(0,100),"%"); }
                    }
                    Section::Curves => {
                        this.choice(g,"curve-format","Curve format","/d1/ct",&[(0,"Smooth"),(1,"Point-defined")]);
                        this.number(g,"curvature","Smooth curvature","/d1/rc",(0,100),"%");
                        let count = this.groups[g].value["d1"]["cv"].as_array().map(Vec::len).filter(|n| matches!(n,10|16)).ok_or_else(|| failed("controller returned an unsupported curve point count"))?;
                        for i in 0..count { this.number(g,&format!("curve-{i}"),&format!("Point {} · {}",i / 2 + 1,if i % 2 == 0 { "Input" } else { "Output" }),&format!("/d1/cv/{i}"),(0,100),"%"); }
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
                let keys = this.groups[g].value["d1"]["tk"].as_array().ok_or_else(|| failed("controller returned invalid motion trigger keys"))?.len().min(2);
                for i in 0..keys { this.choice(g,&format!("trigger-key-{i}"),&format!("Trigger key {}",i + 1),&format!("/d1/tk/{i}"),KEYS); }
                this.choice(g,"trigger-logic","Trigger keys needed","/d1/kao",&[(0,"Both"),(1,"Either")]);
                this.choice(g,"mouse-stick","Mouse-linked stick","/d1/mlj",&[(0,"Off"),(1,"Left stick"),(2,"Right stick")]);
                this.choice(g,"response","Response curve","/d1/rt",&[(0,"Linear"),(1,"Convex"),(2,"Concave"),(3,"S-curve")]);
                this.number(g,"curvature","Curvature","/d1/rc",(0,100),"%");
                this.number(g,"sensitivity","Sensitivity","/d1/se",(0,100),"%");
                this.number(g,"ratio","Horizontal to vertical ratio","/d1/hvr",(0,100),"%");
                this.choice(g,"horizontal-axis","Horizontal axis","/d1/had",&[(0,"Yaw"),(1,"Roll"),(2,"Yaw and roll")]);
                this.field(g,"reverse-swing","Reverse horizontal swing","/d1/hsr",Kind::Toggle);
                this.field(g,"reverse-roll","Reverse horizontal roll","/d1/hrr",Kind::Toggle);
                this.field(g,"reverse-vertical","Reverse vertical","/d1/vr",Kind::Toggle);
                let thresholds = this.groups[g].value["d1"]["ec"].as_array().ok_or_else(|| failed("controller returned invalid motion thresholds"))?.clone();
                for (i,entry) in thresholds.iter().enumerate() {
                    // An axis this list does not name is kept as it is.
                    let Some(axis) = ["Yaw","Roll","Pitch"].get(entry["et"].as_u64().unwrap_or(u64::MAX) as usize) else { continue };
                    this.number(g,&format!("acceleration-{i}"),&format!("Gesture threshold · {axis} acceleration"),&format!("/d1/ec/{i}/acc"),(0,100),"%");
                    this.number(g,&format!("angle-{i}"),&format!("Gesture threshold · {axis} angle"),&format!("/d1/ec/{i}/ag"),(0,100),"%");
                }
            }
            Section::Gestures => {
                let macros = macros(controller)?;
                let g = this.group(controller, Query::profile_settings(Command::Motion,profile).map_err(failed)?,Command::SetMotion,"w")?;
                let events: Vec<(usize, u8)> = this.groups[g].value["d1"]["el"].as_array().ok_or_else(|| failed("controller returned invalid motion gestures"))?
                    .iter().enumerate().filter_map(|(i,entry)| entry["e"].as_u64().filter(|e| *e < 6).map(|e| (i, e as u8 + 1))).collect();
                if let Some(&(i,button)) = events.iter().find(|(_,button)| *button == view.button).or(events.first()) {
                    view.button = button;
                    this.items = events.iter().map(|(_,button)| json!({"id":button.to_string(),"label":GESTURES[usize::from(button - 1)]})).collect();
                    this.action(g,&format!("/d1/el/{i}"),view.action,false,&macros)?;
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
    fn group<T: Transport>(&mut self, controller: &mut Controller<T>, read: Query, set: Command, argument: &'static str) -> Result<usize, Response> {
        let value = controller.read(&read).map_err(failed)?.result;
        let args = read.value();
        for key in ["b","c","d"] { if let Some(expected) = args["g"].get(key) { if value.get(key) != Some(expected) { return Err(failed("controller returned settings for a different profile or side")); } } }
        let index = self.groups.len(); self.groups.push(Group { read, value, set, argument }); Ok(index)
    }
    fn field(&mut self, group: usize, key: &str, label: &str, path: &str, kind: Kind) { self.fields.push(Field { group, key:key.into(), label:label.into(), path:path.into(), kind, help: None }); }
    fn help(&mut self, text: &'static str) { if let Some(field) = self.fields.last_mut() { field.help = Some(text); } }
    fn number(&mut self, group: usize, key: &str, label: &str, path: &str, range: (i64,i64), unit: &'static str) { self.field(group,key,label,path,Kind::Number(range.0,range.1,1,unit)); }
    fn choice(&mut self, group: usize, key: &str, label: &str, path: &str, options: &'static [(i64,&'static str)]) { self.field(group,key,label,path,Kind::Choice(options)); }
    fn action(&mut self, group: usize, path: &str, side: u8, typed: bool, macros: &[(String, u16)]) -> Result<(), Response> {
        if typed { self.choice(group,"action-type","Button action",&format!("{path}/a"),&[(0,"Press"),(1,"Press and release")]); }
        let path = format!("{path}/{}",if side == 0 { "dn" } else { "up" });
        let action = self.groups[group].value.pointer(&path).ok_or_else(|| failed("controller omitted the requested action"))?;
        match (action["t"].as_u64(), action["r"].as_u64()) {
            (Some(0|2), Some(0|2)) => {
                self.field(group,"target","Mapping target",&path,Kind::Target(macros.to_vec()));
                for i in 0..4 { self.choice(group,&format!("key-{i}"),&format!("Mapped button {}",i+1),&format!("{path}/v/{i}"),KEYS); }
                self.choice(group,"turbo","Repeat mode",&format!("{path}/r"),&[(0,"Normal"),(2,"Turbo")]);
                self.choice(group,"turbo-rate","Turbo rate",&format!("{path}/sp"),&[(0,"Off"),(5,"5 / second"),(10,"10 / second"),(20,"20 / second"),(30,"30 / second")]);
            }
            (Some(1), Some(1)) => {
                self.field(group,"target","Mapping target",&path,Kind::Target(macros.to_vec()));
                self.field(group,"macro","Macro",&path,Kind::Macro(macros.to_vec()));
                self.choice(group,"macro-playback","Playback",&format!("{path}/mr"),&[(0,"Off"),(1,"Tap: play once"),(2,"Hold: play once"),(3,"Hold: repeat"),(4,"Tap: repeat")]);
                self.field(group,"macro-interval","Repeat interval",&format!("{path}/ri"),Kind::Number(0,u16::MAX.into(),10,"ms"));
            }
            // Keyboard and mouse encodings are retained, not replaced with guessed gamepad defaults.
            _ => {}
        }
        Ok(())
    }
    pub fn display(&self, view: &View, profiles: u8) -> Result<(Value, Value), Response> {
        let mut fields = Vec::new(); let mut values = serde_json::Map::new();
        let mut selector = |key: &str, label: &str, options: Vec<Value>, value: String| { fields.push(json!({"key":key,"label":label,"type":"choice","options":options,"default":value})); values.insert(key.into(),json!(value)); };
        selector("view-section","Settings",SECTIONS.iter().map(|(id,label)| json!({"id":id,"label":label})).collect(),serde_json::to_value(view.section).map_err(failed)?.as_str().unwrap().into());
        if !matches!(view.section,Section::General|Section::Macros) {
            selector("view-profile","Profile",(1..=profiles).map(|n| json!({"id":n.to_string(),"label":format!("Profile {n}")})).collect(),view.profile.to_string());
            if view.section != Section::Profiles { selector("view-sub","Configuration",[(0,"Main"),(1,"Sub 1"),(2,"Sub 2")].into_iter().map(|(n,label)| json!({"id":n.to_string(),"label":label})).collect(),view.sub.to_string()); }
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
            Section::Gestures | Section::Macros if !self.items.is_empty() => selector("view-button",if view.section == Section::Macros { "Macro" } else { "Gesture" },self.items.clone(),view.button.to_string()),
            _ => {}
        }
        if self.steps > 0 { selector("view-step","Step",(1..=self.steps).map(|n| json!({"id":n.to_string(),"label":format!("Step {n} of {}",self.steps)})).collect(),view.step.to_string()); }
        for field in &self.fields { let (definition,value) = field.display(&self.groups[field.group].value)?; if let Some(value) = value { values.insert(field.key.clone(),value); } fields.push(definition); }
        Ok((json!({"sections":[{"title":"Controller","fields":fields}]}),Value::Object(values)))
    }
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
