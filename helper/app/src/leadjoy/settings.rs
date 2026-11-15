use super::{View, Section, failed};
use crate::http::Response;
use leadjoy_config::{Change, Command, Controller, Profile, Request as Query, Transport};
use serde_json::{Value, json};

pub(super) const SECTIONS: &[(&str, &str)] = &[("general","General and logo"),("profiles","Profiles"),("buttons","Buttons and turbo"),("sticks","Sticks"),("curves","Stick curves"),("directions","Stick direction bindings"),("triggers","Triggers"),("rumble","Rumble")];
pub(super) const BUTTONS: &[(i64, &str)] = &[(26,"R4 / M3"),(27,"L4 / M4"),(24,"R5 / M1"),(25,"L5 / M2"),(1,"A"),(2,"B"),(4,"X"),(5,"Y"),(14,"Left stick click"),(15,"Right stick click"),(17,"D-pad up"),(18,"D-pad down"),(19,"D-pad left"),(20,"D-pad right"),(7,"LB"),(8,"RB"),(9,"LT"),(10,"RT"),(11,"Select"),(16,"Capture"),(12,"Start"),(3,"C")];
const KEYS: &[(i64, &str)] = &[(0,"None"),(1,"A"),(2,"B"),(3,"C"),(4,"X"),(5,"Y"),(6,"Z"),(7,"LB"),(8,"RB"),(9,"LT"),(10,"RT"),(11,"Select"),(12,"Start"),(13,"Home"),(14,"Left stick click"),(15,"Right stick click"),(16,"Capture"),(17,"D-pad up"),(18,"D-pad down"),(19,"D-pad left"),(20,"D-pad right"),(21,"Menu"),(22,"Mode"),(23,"FN"),(24,"R5 / M1"),(25,"L5 / M2"),(26,"R4 / M3"),(27,"L4 / M4"),(32,"Power")];

pub(super) enum Kind { Number(i64, i64, &'static str), Toggle, Choice(&'static [(i64, &'static str)]), Text, Activate }
pub(super) struct Field { pub key: String, label: String, path: String, kind: Kind, pub group: usize }
pub(super) struct Group { pub read: Query, pub value: Value, set: Command, argument: &'static str }
pub(super) struct Settings { pub groups: Vec<Group>, pub fields: Vec<Field> }
impl Group {
    pub fn change(&self, value: Value, key: &str, view: &View) -> Result<Change, Response> {
        let (write, expected) = if self.read.command() == Command::Lighting {
            let mut fields = serde_json::Map::new(); fields.insert("a".into(), json!(1)); fields.insert("s".into(), value.clone());
            (Query::new(Command::Lighting, fields).map_err(failed)?, value)
        } else if self.read.command() == Command::ProfileSummary {
            let version = if value.get("d2").is_some() { "d2" } else { "d1" };
            if key == "profile-activate" {
                (Query::argument(Command::ActivateProfile, "s", json!(view.profile)).map_err(failed)?, json!({version:{"a":1}}))
            } else {
                let name = value[version]["n"].clone();
                (Query::argument(Command::SetProfileName, "s", json!({"b":view.profile,"n":name})).map_err(failed)?, json!({version:{"n":name}}))
            }
        } else {
            (Query::argument(self.set, self.argument, value.clone()).map_err(failed)?, value)
        };
        Ok(Change { write, read: self.read.clone(), expected })
    }
}
impl Field {
    pub fn apply(&self, value: &Value, original: &Value) -> Result<Value, &'static str> {
        let wire = match &self.kind {
            Kind::Number(min, max, _) => value.as_i64().filter(|n| (*min..=*max).contains(n)).map(|n| json!(n)),
            Kind::Toggle => value.as_bool().map(|v| json!(u8::from(v))),
            Kind::Choice(options) => value.as_str().and_then(|s| s.parse::<i64>().ok()).filter(|n| options.iter().any(|(id,_)| id == n)).map(|n| json!(n)),
            Kind::Text => value.as_str().filter(|s| s.chars().count() <= 128 && !s.chars().any(char::is_control)).map(|s| json!(s)),
            Kind::Activate => value.is_null().then(|| json!(1)),
        }.ok_or("value is outside this setting's supported range")?;
        let mut next = original.clone();
        *next.pointer_mut(&self.path).ok_or("controller did not report this setting")? = wire;
        Ok(next)
    }
    fn display(&self, original: &Value) -> Result<(Value, Option<Value>), Response> {
        let value = original.pointer(&self.path).ok_or_else(|| failed(format!("controller omitted {}", self.key)))?;
        let mut field = json!({"key":self.key,"label":self.label});
        let shown = match &self.kind {
            Kind::Number(min,max,unit) => {
                let n = value.as_i64().filter(|n| (*min..=*max).contains(n)).ok_or_else(|| failed(format!("controller returned an unsupported {} value", self.key)))?;
                field["type"] = json!("number"); field["min"] = json!(min); field["max"] = json!(max); field["step"] = json!(1); field["default"] = json!(n);
                if !unit.is_empty() { field["unit"] = json!(unit); }
                Some(json!(n))
            }
            Kind::Toggle => {
                let n = value.as_u64().filter(|n| *n <= 1).ok_or_else(|| failed(format!("controller returned an invalid {} switch", self.key)))?;
                field["type"] = json!("toggle"); field["default"] = json!(n == 1); Some(json!(n == 1))
            }
            Kind::Choice(options) => {
                let n = value.as_i64().filter(|n| options.iter().any(|(id,_)| id == n)).ok_or_else(|| failed(format!("controller returned an unsupported {} choice", self.key)))?;
                field["type"] = json!("choice"); field["default"] = json!(n.to_string());
                field["options"] = json!(options.iter().map(|(id,label)| json!({"id":id.to_string(),"label":label})).collect::<Vec<_>>());
                Some(json!(n.to_string()))
            }
            Kind::Text => { field["type"] = json!("text"); field["max_chars"] = json!(128); Some(json!(value.as_str().ok_or_else(|| failed("controller returned an invalid profile name"))?)) }
            Kind::Activate => { field["type"] = json!("action"); field["help"] = json!(if value == 1 { "This profile is active." } else { "Make this the controller's active profile." }); None }
        };
        Ok((field, shown))
    }
}
impl Settings {
    pub fn read<T: Transport>(controller: &mut Controller<T>, view: &View) -> Result<Self, Response> {
        let mut this = Self { groups: vec![], fields: vec![] };
        let profile = Profile::new(view.profile, view.sub).map_err(failed)?;
        match view.section {
            Section::General => {
                let g = this.group(controller, Query::lighting(), Command::Lighting, "s")?;
                let lighting: leadjoy_config::Lighting = serde_json::from_value(this.groups[g].value.clone()).map_err(failed)?;
                if lighting.brightness > 100 { return Err(failed("controller returned an invalid logo brightness")); }
                this.number(g,"logo-brightness","Logo brightness","/brt",(0,100),"%");
                for (i,key,label) in [(0,"red","Red"),(1,"green","Green"),(2,"blue","Blue")] { this.number(g,&format!("logo-{key}"),&format!("Logo colour · {label}"),&format!("/rgb/{i}"),(0,255),""); }
                let g = this.group(controller, Query::query(Command::PollingRate), Command::SetPollingRate,"s")?;
                this.choice(g,"polling","Report rate","",&[(125,"125 Hz"),(250,"250 Hz"),(500,"500 Hz"),(1000,"1000 Hz")]);
                let g = this.group(controller, Query::query(Command::StepSize), Command::SetStepSize,"s")?;
                this.choice(g,"precision","Step precision","",&[(0,"Adaptive"),(32,"32 steps"),(64,"64 steps"),(128,"128 steps"),(256,"256 steps")]);
                let g = this.group(controller, Query::query(Command::DualOutput), Command::SetDualOutput,"s")?;
                this.field(g,"dual-output","Dual output","",Kind::Toggle);
                let g = this.group(controller, Query::query(Command::JoystickDpad), Command::SetJoystickDpad,"s")?;
                this.field(g,"dpad-swap","Swap D-pad and stick","/std",Kind::Toggle);
                this.field(g,"dpad-lock","D-pad diagonal lock","/ddl",Kind::Toggle);
            }
            Section::Profiles => {
                let g = this.group(controller, Query::profile_summary(view.profile).map_err(failed)?, Command::SetProfileName,"s")?;
                let version = if this.groups[g].value.get("d2").is_some() { "d2" } else { "d1" };
                this.field(g,"profile-name","Profile name",&format!("/{version}/n"),Kind::Text);
                this.field(g,"profile-activate","Activate profile",&format!("/{version}/a"),Kind::Activate);
            }
            Section::Buttons => {
                let g = this.group(controller, Query::buttons(profile,&[view.button]).map_err(failed)?,Command::SetButton,"w")?;
                let items = this.groups[g].value["d1"].as_array().ok_or_else(|| failed("controller returned invalid button mappings"))?;
                let i = items.iter().position(|v| v["k"] == view.button).ok_or_else(|| failed("controller omitted the requested button"))?;
                this.action(g,&format!("/d1/{i}"),view.action)?;
            }
            Section::Sticks | Section::Curves | Section::Directions => {
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
                        this.action(g,&format!("/d1/{key}"),view.action)?;
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
    fn field(&mut self, group: usize, key: &str, label: &str, path: &str, kind: Kind) { self.fields.push(Field { group, key:key.into(), label:label.into(), path:path.into(), kind }); }
    fn number(&mut self, group: usize, key: &str, label: &str, path: &str, range: (i64,i64), unit: &'static str) { self.field(group,key,label,path,Kind::Number(range.0,range.1,unit)); }
    fn choice(&mut self, group: usize, key: &str, label: &str, path: &str, options: &'static [(i64,&'static str)]) { self.field(group,key,label,path,Kind::Choice(options)); }
    fn action(&mut self, group: usize, path: &str, side: u8) -> Result<(), Response> {
        self.choice(group,"action-type","Button action",&format!("{path}/a"),&[(0,"Press"),(1,"Press and release")]);
        let path = format!("{path}/{}",if side == 0 { "dn" } else { "up" });
        let action = self.groups[group].value.pointer(&path).ok_or_else(|| failed("controller omitted the requested action"))?;
        // Macro/keyboard/mouse encodings are retained, not replaced with guessed gamepad defaults.
        if !matches!(action["t"].as_u64(),Some(0|2)) || !matches!(action["r"].as_u64(),Some(0|2)) { return Ok(()); }
        self.choice(group,"target","Mapping target",&format!("{path}/t"),&[(0,"Original input"),(2,"Gamepad")]);
        for i in 0..4 { self.choice(group,&format!("key-{i}"),&format!("Mapped button {}",i+1),&format!("{path}/v/{i}"),KEYS); }
        self.choice(group,"turbo","Repeat mode",&format!("{path}/r"),&[(0,"Normal"),(2,"Turbo")]);
        self.choice(group,"turbo-rate","Turbo rate",&format!("{path}/sp"),&[(0,"Off"),(5,"5 / second"),(10,"10 / second"),(20,"20 / second"),(30,"30 / second")]);
        Ok(())
    }
    pub fn display(&self, view: &View, profiles: u8) -> Result<(Value, Value), Response> {
        let mut fields = Vec::new(); let mut values = serde_json::Map::new();
        let mut selector = |key: &str, label: &str, options: Vec<Value>, value: String| { fields.push(json!({"key":key,"label":label,"type":"choice","options":options,"default":value})); values.insert(key.into(),json!(value)); };
        selector("view-section","Settings",SECTIONS.iter().map(|(id,label)| json!({"id":id,"label":label})).collect(),serde_json::to_value(view.section).map_err(failed)?.as_str().unwrap().into());
        if view.section != Section::General {
            selector("view-profile","Profile",(1..=profiles).map(|n| json!({"id":n.to_string(),"label":format!("Profile {n}")})).collect(),view.profile.to_string());
            if view.section != Section::Profiles { selector("view-sub","Configuration",[(0,"Main"),(1,"Sub 1"),(2,"Sub 2")].into_iter().map(|(n,label)| json!({"id":n.to_string(),"label":label})).collect(),view.sub.to_string()); }
        }
        if matches!(view.section,Section::Sticks|Section::Curves|Section::Directions) {
            selector("view-side","Stick",["Left","Right"].into_iter().enumerate().map(|(i,label)| json!({"id":i.to_string(),"label":label})).collect(),view.side.to_string());
        }
        if matches!(view.section,Section::Directions|Section::Buttons) {
            selector("view-action","Action to edit",["Press","Release"].into_iter().enumerate().map(|(i,label)| json!({"id":i.to_string(),"label":label})).collect(),view.action.to_string());
        }
        if matches!(view.section,Section::Buttons|Section::Directions) {
            let options = if view.section == Section::Buttons { BUTTONS } else { &[(1,"Left"),(2,"Up"),(3,"Right"),(4,"Down")] };
            selector("view-button",if view.section == Section::Buttons { "Button" } else { "Direction" },options.iter().map(|(i,label)| json!({"id":i.to_string(),"label":label})).collect(),view.button.to_string());
        }
        for field in &self.fields { let (definition,value) = field.display(&self.groups[field.group].value)?; if let Some(value) = value { values.insert(field.key.clone(),value); } fields.push(definition); }
        Ok((json!({"sections":[{"title":"Controller","fields":fields}]}),Value::Object(values)))
    }
}
