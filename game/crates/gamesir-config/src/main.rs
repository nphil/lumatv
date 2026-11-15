use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use gamesir_config::Error;
use gamesir_config::device::{self, Check, Controller, Hidraw, Timing, Written};
use gamesir_config::frame::{Link, Model, Region};
use gamesir_config::profile::{
    Button, Deadzone, Flag, HairMode, HairTrigger, LightProfile, Profile, ReportRate, Side, StickConfig, TriggerConfig, output_code, output_name,
};

const USAGE: &str = "usage: gamesir-config COMMAND [--device /dev/hidrawN]
  list                                  GameSir HID nodes and which offer the configuration interface
  info [--profile 1-4|shift]            firmware versions, active profile and its settings
  dump [--region 1-4|shift|light]       raw bytes of a settings block
  set [--profile 1-4|shift] [--dry-run] SETTING=VALUE...
  switch 1-4                            make another profile active
  rumble LEFT RIGHT                     run the motors (0-255) for half a second

settings:
  report-rate=250|500|1000              rumble=LEFT,RIGHT (levels 0-4)
  dpad-diagonal-lock=on|off             xinput-abxy-swap=on|off      switch-abxy-swap=on|off
  ls-deadzone=INNER,OUTER[,ANTI_INNER,ANTI_OUTER]  likewise rs-, lt-, rt-deadzone (percent)
  lt-hair=off|rapid:START,END|fixed:START,END     likewise rt-hair (percent of travel)
  map.BUTTON=OUTPUT[+OUTPUT[+OUTPUT]]|default      turbo.BUTTON=0-30 (presses per second)
  light-brightness=0-100   light-speed=0-20   light-color=RRGGBB   light-standby=0|1|5|10|20   light-slot=0-3
buttons: up down left right lb rb ls rs a b x y home view menu share l4 r4
outputs: the buttons, lt rt ls-up ls-down ls-left ls-right rs-up rs-down rs-left rs-right disabled, or a raw code";

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct Options {
    device: Option<PathBuf>,
    region: Option<Region>,
    dry_run: bool,
    rest: Vec<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("gamesir-config: {e}");
            if let Some(source) = e.source() {
                eprintln!("  caused by: {source}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Fallible<ExitCode> {
    let Some((command, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return Ok(ExitCode::FAILURE);
    };
    let options = parse_options(rest)?;
    match command.as_str() {
        "list" => list(),
        "info" => info(&options),
        "dump" => dump(&options),
        "set" => set(&options),
        "switch" => switch(&options),
        "rumble" => rumble(&options),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command {other:?}\n{USAGE}").into()),
    }
}

fn parse_options(args: &[String]) -> Fallible<Options> {
    let mut options = Options::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--device" => options.device = Some(args.next().ok_or("--device needs a path")?.into()),
            "--profile" | "--region" => options.region = Some(region(args.next().ok_or("--profile needs a value")?)?),
            "--dry-run" => options.dry_run = true,
            _ => options.rest.push(arg.clone()),
        }
    }
    Ok(options)
}

fn region(value: &str) -> Fallible<Region> {
    match value {
        "shift" => Ok(Region::SHIFT),
        "light" => Ok(Region::LIGHT),
        n => n.parse().ok().and_then(Region::profile).ok_or_else(|| format!("{n:?} is not a profile 1-4, shift or light").into()),
    }
}

fn open(options: &Options) -> Fallible<Controller<Hidraw>> {
    if let Some(path) = &options.device {
        return Ok(Controller::new(Hidraw::open(path).map_err(|e| format!("{}: {e}", path.display()))?));
    }
    // A Cyclone 2 can show two configuration nodes of which only one answers (gamesir-linux-tools), so each is asked for its
    // firmware version before it is used.
    let probe = Timing { resend_after: Duration::from_millis(200), max_sends: 2 };
    let mut tried = Vec::new();
    for found in device::discover()?.into_iter().filter(|found| found.config) {
        let answered = Hidraw::open(&found.node).map_err(Error::from).and_then(|io| {
            let mut controller = Controller::with_timing(io, probe);
            controller.firmware_version().map(|_| controller.into_inner())
        });
        match answered {
            Ok(io) => return Ok(Controller::new(io)),
            Err(e) => tried.push(format!("{}: {e}", found.node.display())),
        }
    }
    if tried.is_empty() {
        return Err("no Cyclone 2 configuration interface found; `gamesir-config list` shows what is attached".into());
    }
    Err(format!("no configuration interface answered ({}); a sleeping controller answers once a button wakes it", tried.join("; ")).into())
}

fn list() -> Fallible<ExitCode> {
    for found in device::discover()? {
        let mode = match found.model {
            Some((model, link)) => format!(
                "{} over the {}, {}",
                match model {
                    Model::Cyclone2 => "Cyclone 2",
                    Model::Cyclone2Adc => "Cyclone 2 ADC",
                },
                match link {
                    Link::Cable => "cable",
                    Link::Receiver => "receiver",
                },
                if found.config { "configurable" } else { "no configuration reports on this node" }
            ),
            // Its vendor channel streams zeros and answers nothing (gamesir-linux-tools, cyclone2-linux).
            None if (found.vendor, found.product) == (0x3537, 0x0575) => "HID mode, or the receiver with the controller off".to_owned(),
            None => "a mode without the configuration interface (DS4, Switch or Bluetooth)".to_owned(),
        };
        println!("{}  {:04x}:{:04x}  {}  ({mode})", found.node.display(), found.vendor, found.product, found.name);
    }
    Ok(ExitCode::SUCCESS)
}

fn active_region(controller: &mut Controller<Hidraw>, options: &Options) -> Fallible<Region> {
    match options.region {
        Some(region) => Ok(region),
        None => Ok(Region::profile(controller.current_profile()?).ok_or("the controller reported an active profile outside 1-4")?),
    }
}

fn info(options: &Options) -> Fallible<ExitCode> {
    let mut controller = open(options)?;
    let version = controller.firmware_version()?;
    let active = controller.current_profile()?;
    println!("firmware: controller {}, receiver {}", version.controller, version.receiver);
    println!("active profile: {active}");
    let region = match options.region {
        Some(region) => region,
        None => Region::profile(active).ok_or("the controller reported an active profile outside 1-4")?,
    };
    if region == Region::LIGHT {
        return Err("info shows a profile; use `dump --region light` for the light block".into());
    }
    let p = Profile::from_bytes(&controller.read(region)?)?;
    println!("profile {} {:?}", region.number(), p.name());
    println!("  report rate: {}", p.report_rate().map_or("unknown".to_owned(), |r| format!("{} Hz", r.hz())));
    println!("  rumble levels: {:?}", p.rumble());
    for flag in [Flag::DpadDiagonalLock, Flag::XinputAbxySwap, Flag::SwitchAbxySwap] {
        println!("  {flag:?}: {}", p.flag(flag));
    }
    for (name, side) in [("ls", Side::Left), ("rs", Side::Right)] {
        let s = p.stick(side);
        println!("  {name}: {:?}, raw trace {}, curve {:?}", s.response.deadzone, s.raw, s.response.curve);
    }
    for (name, side) in [("lt", Side::Left), ("rt", Side::Right)] {
        let t = p.trigger(side);
        println!("  {name}: {:?}, hair trigger {:?}", t.response.deadzone, t.hair);
    }
    for button in Button::ALL {
        let b = p.button(button);
        if b != Default::default() {
            let outputs: Vec<String> = b.outputs.iter().filter(|&&c| c != 0).map(|&c| output_name(c).map_or(c.to_string(), str::to_owned)).collect();
            println!("  {}: outputs {outputs:?}, turbo {} Hz, toggle {}", button.name(), b.turbo_hz, b.toggle);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn dump(options: &Options) -> Fallible<ExitCode> {
    let mut controller = open(options)?;
    let region = active_region(&mut controller, options)?;
    for (row, bytes) in controller.read(region)?.chunks(16).enumerate() {
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        println!("{:04}: {}", row * 16, hex.join(" "));
    }
    Ok(ExitCode::SUCCESS)
}

fn switch(options: &Options) -> Fallible<ExitCode> {
    let [slot] = options.rest.as_slice() else { return Err("switch takes one profile number".into()) };
    let slot: u8 = slot.parse()?;
    open(options)?.switch_profile(slot)?;
    println!("active profile: {slot}");
    Ok(ExitCode::SUCCESS)
}

fn rumble(options: &Options) -> Fallible<ExitCode> {
    let [left, right] = options.rest.as_slice() else { return Err("rumble takes LEFT and RIGHT strengths".into()) };
    let (left, right) = (left.parse()?, right.parse()?);
    let mut controller = open(options)?;
    controller.vibrate(left, right)?;
    thread::sleep(Duration::from_millis(500));
    controller.vibrate(0, 0)?;
    Ok(ExitCode::SUCCESS)
}

/// Every setting is applied to in-memory copies first, so a bad one stops the command before anything is written.
fn set(options: &Options) -> Fallible<ExitCode> {
    if options.rest.is_empty() {
        return Err("set needs at least one SETTING=VALUE".into());
    }
    let mut settings = Vec::new();
    for arg in &options.rest {
        settings.push(arg.split_once('=').ok_or_else(|| format!("{arg:?} is not SETTING=VALUE"))?);
    }
    let (light, profile): (Vec<_>, Vec<_>) = settings.into_iter().partition(|(key, _)| key.starts_with("light-"));

    let mut controller = open(options)?;
    let mut writes = Vec::new();
    if !profile.is_empty() {
        let region = active_region(&mut controller, options)?;
        if region == Region::LIGHT {
            return Err("profile settings need --profile 1-4 or shift".into());
        }
        let before = controller.read(region)?;
        let mut p = Profile::from_bytes(&before)?;
        for &(key, value) in &profile {
            apply(&mut p, key, value).map_err(|e| format!("{key}={value}: {e}"))?;
        }
        writes.push((region, before, p.as_bytes().to_vec()));
    }
    if !light.is_empty() {
        let before = controller.read(Region::LIGHT)?;
        let mut l = LightProfile::from_bytes(&before)?;
        for &(key, value) in &light {
            apply_light(&mut l, key, value).map_err(|e| format!("{key}={value}: {e}"))?;
        }
        writes.push((Region::LIGHT, before, l.as_bytes().to_vec()));
    }

    let mut ok = true;
    for (region, before, after) in writes {
        ok &= report(region, &controller.write(region, &before, &after, options.dry_run)?);
    }
    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn report(region: Region, written: &Written) -> bool {
    for r in &written.reports {
        let hex: Vec<String> = r[..6 + usize::from(r[5])].iter().map(|b| format!("{b:02x}")).collect();
        println!("region {}: {}", region.number(), hex.join(" "));
    }
    if written.unacked > 0 {
        println!("region {}: {} write report(s) drew no acknowledgement", region.number(), written.unacked);
    }
    match &written.check {
        None if written.reports.is_empty() => println!("region {}: already set", region.number()),
        None => println!("region {}: dry run, nothing sent", region.number()),
        Some(Check::Match) => println!("region {}: written and read back", region.number()),
        Some(Check::Mismatch { offset, wanted, found }) => {
            println!("region {}: byte {offset} reads back {found:#04x}, wrote {wanted:#04x}", region.number());
            return false;
        }
    }
    true
}

fn numbers(value: &str) -> Fallible<Vec<u8>> {
    value.split(',').map(|n| Ok(n.trim().parse()?)).collect()
}

fn on_off(value: &str) -> Fallible<bool> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err("expected on or off".into()),
    }
}

fn side(prefix: &str) -> Side {
    if prefix.starts_with('l') { Side::Left } else { Side::Right }
}

fn deadzone(current: Deadzone, value: &str) -> Fallible<Deadzone> {
    match *numbers(value)?.as_slice() {
        [inner, outer] => Ok(Deadzone { inner, outer, ..current }),
        [inner, outer, anti_inner, anti_outer] => Ok(Deadzone { inner, outer, anti_inner, anti_outer }),
        _ => Err("expected INNER,OUTER or INNER,OUTER,ANTI_INNER,ANTI_OUTER".into()),
    }
}

fn hair(value: &str) -> Fallible<Option<HairTrigger>> {
    if value == "off" {
        return Ok(None);
    }
    let (mode, range) = value.split_once(':').ok_or("expected off, rapid:START,END or fixed:START,END")?;
    let mode = match mode {
        "rapid" => HairMode::Rapid,
        "fixed" => HairMode::Fixed,
        _ => return Err("the hair trigger mode is rapid or fixed".into()),
    };
    let &[start, end] = numbers(range)?.as_slice() else { return Err("expected START,END".into()) };
    Ok(Some(HairTrigger { mode, start, end }))
}

fn apply(p: &mut Profile, key: &str, value: &str) -> Fallible<()> {
    match key {
        "report-rate" => p.set_report_rate(match value {
            "250" => ReportRate::Hz250,
            "500" => ReportRate::Hz500,
            "1000" => ReportRate::Hz1000,
            _ => return Err("the report rate is 250, 500 or 1000".into()),
        }),
        "rumble" => {
            let &[left, right] = numbers(value)?.as_slice() else { return Err("expected LEFT,RIGHT".into()) };
            p.set_rumble(left, right)?;
        }
        "dpad-diagonal-lock" => p.set_flag(Flag::DpadDiagonalLock, on_off(value)?),
        "xinput-abxy-swap" => p.set_flag(Flag::XinputAbxySwap, on_off(value)?),
        "switch-abxy-swap" => p.set_flag(Flag::SwitchAbxySwap, on_off(value)?),
        "ls-deadzone" | "rs-deadzone" => {
            let side = side(key);
            let stick = p.stick(side);
            let response = stick.response.with_deadzone(deadzone(stick.response.deadzone, value)?);
            p.set_stick(side, &StickConfig { response, ..stick })?;
        }
        "lt-deadzone" | "rt-deadzone" => {
            let side = side(key);
            let trigger = p.trigger(side);
            let response = trigger.response.with_deadzone(deadzone(trigger.response.deadzone, value)?);
            p.set_trigger(side, &TriggerConfig { response, ..trigger })?;
        }
        "lt-hair" | "rt-hair" => {
            let side = side(key);
            p.set_trigger(side, &TriggerConfig { hair: hair(value)?, ..p.trigger(side) })?;
        }
        _ => {
            let (kind, name) = key.split_once('.').ok_or("unknown setting")?;
            let button = Button::from_name(name).ok_or_else(|| format!("unknown button {name:?}"))?;
            let mut config = p.button(button);
            match kind {
                "map" if value == "default" => config.outputs = [0; 3],
                "map" => {
                    let codes: Vec<u8> = value
                        .split('+')
                        .map(|o| output_code(o).or_else(|| o.parse().ok()).ok_or_else(|| format!("unknown output {o:?}")))
                        .collect::<Result<_, _>>()?;
                    if codes.len() > 3 {
                        return Err("a button sends at most three outputs".into());
                    }
                    config.outputs = [0; 3];
                    config.outputs[..codes.len()].copy_from_slice(&codes);
                }
                "turbo" => config.turbo_hz = value.parse()?,
                _ => return Err("unknown setting".into()),
            }
            p.set_button(button, &config)?;
        }
    }
    Ok(())
}

fn apply_light(l: &mut LightProfile, key: &str, value: &str) -> Fallible<()> {
    let slot = l.active_slot();
    match key {
        "light-brightness" => l.set_brightness(slot, value.parse()?)?,
        "light-speed" => l.set_speed(slot, value.parse()?)?,
        "light-standby" => l.set_standby_minutes(value.parse()?)?,
        "light-slot" => l.set_active_slot(value.parse()?)?,
        "light-color" => {
            let rgb =
                u32::from_str_radix(value.trim_start_matches('#'), 16).ok().filter(|_| value.trim_start_matches('#').len() == 6).ok_or("expected RRGGBB")?;
            let [_, r, g, b] = rgb.to_be_bytes();
            l.set_color(slot, [r, g, b])?;
        }
        _ => return Err("unknown setting".into()),
    }
    Ok(())
}
