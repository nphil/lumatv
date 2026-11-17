use crate::Adapter;
use lumagame_core::ipc::{self, ErrorCode, Event, FromEngine, Hello, LineBuffer, Request, ToEngine};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixDatagram;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct Options {
    pub core: PathBuf,
    pub core_args: Vec<String>,
    pub streamer: PathBuf,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item=String>) -> Result<Self,String> {
        let base = std::env::current_exe().map_err(|e| e.to_string())?.parent().unwrap_or(Path::new(".")).to_owned();
        let mut options = Self { core:base.join("opennow-core"),core_args:Vec::new(),streamer:base.join("opennow-streamer") };
        let mut args = args.into_iter();
        while let Some(key) = args.next() {
            let value = args.next().ok_or_else(|| format!("Missing value for {key}"))?;
            match key.as_str() {
                "--core" => options.core = value.into(),
                "--core-arg" => options.core_args.push(value),
                "--streamer" => options.streamer = value.into(),
                _ => return Err(format!("Unknown argument {key}")),
            }
        }
        // The app starts engines in their own directory, and under the package's loader current_exe() names the loader, so the
        // manifest's relative paths are taken from there. The core hands the streamer path on as a setting, so it must be absolute.
        let directory = std::env::current_dir().map_err(|e| e.to_string())?;
        options.core = directory.join(&options.core);
        options.streamer = directory.join(&options.streamer);
        Ok(options)
    }
}

enum Input { App(String), Core(String), Pad(String), AppEof, CoreEof, Invalid(bool) }

fn reader(mut input: impl Read, tx: SyncSender<Input>, app: bool) {
    let mut lines = LineBuffer::default();
    let mut bytes = [0u8;8192];
    loop {
        match input.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => {
                let before = lines.dropped;
                let mut disconnected = false;
                lines.push(&bytes[..n],|line| {
                    let message = if app {Input::App(line.into())} else {Input::Core(line.into())};
                    if tx.send(message).is_err() { disconnected = true; }
                });
                if disconnected { return; }
                if lines.dropped != before { let _ = tx.send(Input::Invalid(app)); return; }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let _ = tx.send(if app {Input::AppEof} else {Input::CoreEof});
}

/// Datagrams from the streamer's pad bridge; ends when [`run`] shuts the socket down.
fn pad_reader(socket: UnixDatagram, tx: SyncSender<Input>, stats: Arc<AtomicBool>) {
    let mut bytes = [0u8;512];
    while let Ok((n @ 1.., peer)) = socket.recv_from(&mut bytes) {
        if &bytes[..n] == b"stats" {
            let _ = socket.send_to_addr(&[u8::from(stats.load(Ordering::Acquire))], &peer);
            continue;
        }
        let Ok(line) = std::str::from_utf8(&bytes[..n]) else { continue };
        if tx.send(Input::Pad(line.into())).is_err() { return; }
    }
}

pub fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().try_into().unwrap_or(u64::MAX)
}

fn private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path,fs::Permissions::from_mode(0o700))
}

/// The core, and the socket its streamer sends pad events to. Without the socket (a path too long to bind) the stream still plays;
/// the controller's Guide button, the overlay's pad keys, the stream's telemetry and word of a stream that ended on its own are lost.
fn spawn(options: &Options, hello: &Hello) -> io::Result<(Child, Option<UnixDatagram>)> {
    let data = Path::new(&hello.data_dir);
    private_dir(data)?;
    for directory in ["config","data","cache"] { private_dir(&data.join(directory))?; }
    let mut command = Command::new(&options.core);
    command.args(&options.core_args).env("HOME",data)
        .env("XDG_CONFIG_HOME",data.join("config")).env("XDG_DATA_HOME",data.join("data"))
        .env("XDG_CACHE_HOME",data.join("cache"))
        .env("LUMATV_GFN_PREFS",data.join("runtime-prefs.json"))
        .env("OPENNOW_STREAMER_LOG_FILE",data.join("streamer.log"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    if let Some(socket) = &hello.feeder { command.env("LUMATV_GFN_FEEDER_SOCKET",socket); }
    let side = data.join("pad.sock");
    let _ = fs::remove_file(&side);
    let pads = UnixDatagram::bind(&side).ok();
    if pads.is_some() { command.env("LUMATV_GFN_PAD_SIDE",&side); }
    // The kernel closes the core if the shim dies before its ordinary stdin shutdown path.
    unsafe {
        command.pre_exec(|| {
            unsafe extern "C" { fn prctl(option:i32, ...) -> i32; fn getppid() -> i32; }
            if prctl(1,9,0,0,0) != 0 { return Err(io::Error::last_os_error()); }
            if getppid() == 1 { return Err(io::Error::other("The engine's parent exited")); }
            Ok(())
        });
    }
    Ok((command.spawn()?,pads))
}

fn app_error(output: &mut impl Write, re: Option<u64>, code: ErrorCode, message: &str) -> io::Result<()> {
    output.write_all(ipc::encode(&FromEngine { re,event:Event::Error { code,message:message.into() } }).as_bytes())?;
    output.flush()
}

/// Runs until the shell closes stdin, the core fails, or the output pipe closes. Only this thread writes either protocol pipe.
pub fn run(options: Options, input: impl Read+Send+'static, mut output: impl Write) -> io::Result<()> {
    let (tx,rx) = mpsc::sync_channel(64);
    let app_tx = tx.clone();
    thread::Builder::new().name("gfn-shell-reader".into()).spawn(move || reader(input,app_tx,true))?;
    let hello = match rx.recv() {
        Ok(Input::App(line)) => match ipc::decode::<ToEngine>(&line) {
            Ok(ToEngine { seq,request:Request::Hello(hello) }) if hello.protocol == ipc::PROTOCOL => (seq,hello),
            Ok(message) => { app_error(&mut output,Some(message.seq),ErrorCode::Protocol,"The first request must be a compatible hello")?; return Ok(()); }
            Err(_) => { app_error(&mut output,None,ErrorCode::BadRequest,"The first request is invalid")?; return Ok(()); }
        },
        Ok(Input::Invalid(_)) => { app_error(&mut output,None,ErrorCode::BadRequest,"The protocol line is invalid or too long")?; return Ok(()); }
        _ => return Ok(()),
    };
    if !Path::new(&hello.1.data_dir).is_absolute() || hello.1.feeder.as_ref().is_none_or(|s| !Path::new(s).is_absolute())
        || hello.1.display.width == 0 || hello.1.display.height == 0 || hello.1.display.refresh_hz == 0 || hello.1.display.codecs.is_empty() {
        app_error(&mut output,Some(hello.0),ErrorCode::BadRequest,"The engine needs an absolute data directory, feeder socket and valid display capabilities")?;
        return Ok(());
    }
    let (mut child,pads) = match spawn(&options,&hello.1) {
        Ok(spawned) => spawned,
        Err(_) => { app_error(&mut output,Some(hello.0),ErrorCode::Internal,"The GeForce NOW core could not be started")?; return Ok(()); }
    };
    let mut core_input = child.stdin.take().ok_or_else(|| io::Error::other("The core has no stdin"))?;
    let core_output = child.stdout.take().ok_or_else(|| io::Error::other("The core has no stdout"))?;
    let pad_stats = Arc::new(AtomicBool::new(false));
    let pads = match pads {
        Some(socket) => {
            let control = socket.try_clone()?;
            let pad_tx = tx.clone();
            let stats = Arc::clone(&pad_stats);
            thread::Builder::new().name("gfn-pad-reader".into()).spawn(move || pad_reader(socket,pad_tx,stats))?;
            Some(control)
        }
        None => None,
    };
    thread::Builder::new().name("gfn-core-reader".into()).spawn(move || reader(core_output,tx,false))?;
    let data = PathBuf::from(&hello.1.data_dir);
    let prefs = fs::read(data.join("prefs.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
    let mut adapter = Adapter::new(hello.1,hello.0,&prefs,options.streamer.to_string_lossy().into_owned());
    let started = Instant::now();
    let mut written_pad_prefs = Value::Null;
    let result: io::Result<()> = (|| {
        loop {
            let batch = adapter.drain();
            if !batch.core.is_empty() {
                let prefs = adapter.pad_prefs();
                if prefs != written_pad_prefs {
                    write_pad_prefs(&data,&prefs)?;
                    written_pad_prefs = prefs;
                }
                for line in batch.core { core_input.write_all(line.as_bytes())?; }
                core_input.flush()?;
            }
            for event in batch.app { output.write_all(ipc::encode(&event).as_bytes())?; }
            output.flush()?;
            if adapter.failed() { break; }
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(Input::App(line)) => match ipc::decode::<ToEngine>(&line) {
                    Ok(request) => adapter.request(request),
                    Err(_) => {
                        let seq = serde_json::from_str::<Value>(&line).ok().and_then(|v| v["seq"].as_u64());
                        app_error(&mut output,seq,ErrorCode::Unsupported,"Unknown or malformed engine request")?;
                    }
                },
                Ok(Input::Core(line)) => adapter.core_line(&line),
                Ok(Input::Pad(line)) => adapter.pad_line(&line),
                Ok(Input::AppEof) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Input::CoreEof) => adapter.core_gone("The GeForce NOW core exited"),
                Ok(Input::Invalid(app)) => {
                    if app { app_error(&mut output,None,ErrorCode::BadRequest,"The protocol line is invalid or too long")?; break; }
                    adapter.core_gone("The GeForce NOW core sent an invalid or oversized line");
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            adapter.tick(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX));
            pad_stats.store(adapter.pad_stats_on(), Ordering::Release);
        }
        Ok(())
    })();
    // End of stdin is the core's only shutdown: its request loop ends, and dropping its streamer service stops the streamer.
    drop(core_input);
    let deadline = Instant::now()+Duration::from_secs(3);
    while child.try_wait()?.is_none() && Instant::now() < deadline { thread::sleep(Duration::from_millis(10)); }
    if child.try_wait()?.is_none() { let _ = child.kill(); }
    let _ = child.wait();
    if let Some(pads) = pads {
        let _ = pads.shutdown(std::net::Shutdown::Both);
        let _ = fs::remove_file(data.join("pad.sock"));
    }
    match result {
        Err(error) if matches!(error.kind(),io::ErrorKind::BrokenPipe|io::ErrorKind::ConnectionReset) => Ok(()),
        result => result,
    }
}

fn write_pad_prefs(data: &Path, prefs: &Value) -> io::Result<()> {
    let path = data.join("runtime-prefs.json.tmp");
    let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    serde_json::to_writer(&mut file,prefs)?;
    file.flush()?;
    fs::rename(path,data.join("runtime-prefs.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_paths_are_taken_from_the_engine_directory() {
        let options = Options::parse(["--core","bin/opennow-core","--streamer","/opt/streamer"].map(String::from)).unwrap();
        assert_eq!(options.core,std::env::current_dir().unwrap().join("bin/opennow-core"));
        assert_eq!(options.streamer,Path::new("/opt/streamer"));
    }

    #[test]
    fn the_shipped_manifest_is_valid_and_its_arguments_parse() {
        let manifest = lumagame_core::engine::Manifest::parse("gfn",include_str!("../engine.json")).unwrap();
        let options = Options::parse(manifest.args).unwrap();
        assert!(options.core.ends_with("bin/opennow-core") && options.streamer.ends_with("bin/opennow-streamer"));
    }
}
