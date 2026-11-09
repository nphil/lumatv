//! A phone's side from a terminal: pair with a TV, drive it, and write the protocol's test vectors.

use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use std::{env, fs, thread};

use esphome_api::base64;
use lumaremote::client::{self, Session};
use lumaremote::datagram::{self, Pointer};
use lumaremote::link::PairLink;
use lumaremote::{server, vectors};
use serde_json::{Value, json};

const USAGE: &str = "usage:
  lumaremote pair LINK [NAME]                  pair by the link in the TV's QR code; the key is kept in ~/.config/lumaremote/HOST.json
  lumaremote key HOST KEY [tap|hold|down|up]   a remote key: ok, up, back, home, volup, …
  lumaremote text HOST TEXT                    type into the active text field
  lumaremote act HOST DO [NAME=VALUE]…         an action: act HOST home, act HOST launch app=netflix
  lumaremote get HOST apps|channels|cameras    a list
  lumaremote point HOST DX DY [click]          move the pointer DX, DY pixels, then click
  lumaremote vectors FILE                      write the protocol's test vectors";

const APP: &str = concat!("lumaremote ", env!("CARGO_PKG_VERSION"));

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lumaremote: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, USAGE)
}

fn run(args: &[&str]) -> io::Result<()> {
    match args {
        ["pair", link, name @ ..] => pair(link, name.first().copied().unwrap_or("lumaremote")),
        ["key", host, key, press @ ..] => {
            let press = press.first().copied().unwrap_or("tap");
            let mut session = open(host)?;
            session.send(&json!({"t": "key", "k": key, "a": press}))?;
            if press == "hold" {
                // The TV times a hold, and a session that ends lets go of its keys.
                thread::sleep(server::HOLD_MUTE + Duration::from_millis(400));
            }
            finish(session)
        }
        ["text", host, text] => send(host, json!({"t": "text", "s": text})),
        ["act", host, act, args @ ..] => {
            let mut msg = json!({"t": "act", "id": 1, "do": act});
            for arg in args {
                let (name, value) = arg.split_once('=').ok_or_else(usage)?;
                msg[name] = value.parse::<u64>().map_or_else(|_| value.into(), Value::from);
            }
            send(host, msg)
        }
        ["get", host, what] => send(host, json!({"t": "get", "id": 1, "what": what})),
        ["point", host, dx, dy, click @ ..] => point(host, dx, dy, click == ["click"]),
        ["vectors", file] => fs::write(file, serde_json::to_string_pretty(&vectors::generate())? + "\n"),
        _ => Err(usage()),
    }
}

fn saved(host: &str) -> io::Result<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| io::Error::other("neither XDG_CONFIG_HOME nor HOME is set"))?;
    Ok(config.join("lumaremote").join(format!("{host}.json")))
}

fn pair(link: &str, name: &str) -> io::Result<()> {
    let link = PairLink::parse(link).ok_or_else(|| io::Error::other("not a lumaremote://pair link"))?;
    let paired = client::pair_link(&link, name, "lumaremote", APP)?;
    let path = saved(&link.host)?;
    fs::create_dir_all(path.parent().expect("a directory"))?;
    let record = json!({"port": link.port, "name": name, "device": paired.device, "key": base64::encode(&paired.key), "tv": paired.tv});
    fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?.write_all(record.to_string().as_bytes())?;
    println!("paired with {} as {}; the key is in {}", paired.tv["name"], paired.device, path.display());
    Ok(())
}

fn open(host: &str) -> io::Result<Session> {
    let path = saved(host)?;
    let record: Value = serde_json::from_slice(&fs::read(&path).map_err(|e| io::Error::other(format!("{}: {e}; pair first", path.display())))?)?;
    let key = record["key"].as_str().and_then(base64::decode_key).ok_or_else(|| io::Error::other(format!("{}: no key", path.display())))?;
    let port = record["port"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(lumaremote::PORT);
    let hello = json!({"t": "hello", "name": record["name"], "model": "lumaremote", "app": APP});
    Session::open(client::resolve(host, port)?, &key, &hello)
}

/// Pings, prints what the TV sends until the pong, and says `bye`.
fn finish(mut session: Session) -> io::Result<()> {
    session.send(&json!({"t": "ping", "n": 0}))?;
    loop {
        let frame = session.receive()?;
        if frame["t"] == "pong" {
            return Ok(());
        }
        println!("{frame}");
    }
}

fn send(host: &str, msg: Value) -> io::Result<()> {
    let mut session = open(host)?;
    session.send(&msg)?;
    finish(session)
}

fn point(host: &str, dx: &str, dy: &str, click: bool) -> io::Result<()> {
    let sixteenths = |px: &str| px.parse::<i32>().map(|px| px.saturating_mul(16)).map_err(|_| usage());
    let mut totals = Pointer { flags: datagram::ENGAGED, ..Pointer::default() };
    let mut session = open(host)?;
    session.pointer(&totals)?;
    totals.x = sixteenths(dx)?;
    totals.y = sixteenths(dy)?;
    session.pointer(&totals)?;
    if click {
        totals.left = 1;
        session.pointer(&totals)?;
    }
    // The pong comes after the TV has taken the datagrams before it.
    session.ping(1, 0)?;
    finish(session)
}
