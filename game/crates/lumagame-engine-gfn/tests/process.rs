use lumagame_core::engine::Codec;
use lumagame_core::ipc::{self,Display,Event,FromEngine,Hello,Request,ToEngine};
use lumagame_core::settings::Values;
use lumagame_engine_gfn::runtime::{Options,run};
use std::fs;
use std::io::{BufRead,BufReader,Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration,SystemTime,UNIX_EPOCH};

struct Scratch(PathBuf);
impl Drop for Scratch { fn drop(&mut self) {let _=fs::remove_dir_all(&self.0);} }

#[test]
fn process_handshake_confines_credentials_and_stdin_eof_stops_the_core() {
    let root=std::env::temp_dir().join(format!("lumagame-gfn-test-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir(&root).unwrap();let scratch=Scratch(root);
    let script=scratch.0.join("core");
    // A separate process speaks only scripted JSON lines; no NVIDIA account or network is involved.
    fs::write(&script,r#"#!/bin/sh
set -eu
printf '%s\n%s\n%s\n' "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" > "$HOME/environment"
while IFS= read -r line; do
  id=${line#*\"id\":\"}; id=${id%%\"*}
  case "$line" in
    *'"method":"core.hello"'*) result='{"protocolVersion":5}' ;;
    *'"method":"settings.set"'*) result='{}' ;;
    *'"method":"streamer.detect"'*) result='{"available":true,"capabilities":{"protocolVersion":7}}' ;;
    *'"method":"auth.session.get"'*) result='{"session":null,"generation":1}' ;;
    *'"method":"app.shutdown"'*) printf 'closed' > "$HOME/closed"; exit 0 ;;
    *) exit 9 ;;
  esac
  printf '{"type":"response","id":"%s","ok":true,"result":%s}\n' "$id" "$result"
done
printf 'closed' > "$HOME/closed"
"#).unwrap();
    fs::set_permissions(&script,fs::Permissions::from_mode(0o700)).unwrap();
    let data=scratch.0.join("engines/gfn");
    let (mut app,shim)=UnixStream::pair().unwrap();
    app.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let writer=shim.try_clone().unwrap();
    let worker=thread::spawn(move ||run(Options{core:script,core_args:Vec::new(),streamer:"/app/opennow-streamer".into()},shim,writer));
    let hello=Hello {
        protocol:1,app_version:"test".into(),data_dir:data.to_string_lossy().into_owned(),cache_dir:scratch.0.join("cache").to_string_lossy().into_owned(),
        feeder:Some("/run/game/feed.sock".into()),locale:"en-US".into(),settings:Values::new(),
        display:Display{width:3840,height:2160,refresh_hz:120,hdr10:true,codecs:vec![Codec::H265]},
    };
    app.write_all(ipc::encode(&ToEngine{seq:1,request:Request::Hello(hello)}).as_bytes()).unwrap();
    let mut reader=BufReader::new(app.try_clone().unwrap());
    let mut line=String::new();
    reader.read_line(&mut line).unwrap();
    assert!(matches!(ipc::decode::<FromEngine>(&line).unwrap().event,Event::Ready{..}));
    let expected=format!("{}\n{}\n{}\n",data.display(),data.join("config").display(),data.join("data").display());
    assert_eq!(fs::read_to_string(data.join("environment")).unwrap(),expected);
    assert_eq!(fs::metadata(&data).unwrap().permissions().mode()&0o777,0o700);
    drop(reader);drop(app);
    worker.join().unwrap().unwrap();
    assert_eq!(fs::read_to_string(data.join("closed")).unwrap(),"closed");
    assert_eq!(fs::metadata(data.join("runtime-prefs.json")).unwrap().permissions().mode()&0o777,0o600);
}
