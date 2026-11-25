#[cfg(test)] mod pairing_tests;
#[cfg(test)] mod library_tests;
mod dualsense;
mod input;
mod library;
mod logging;
mod session;
mod settings;
mod store;

use anyhow::{Context, Result, bail, ensure};
use lumagame_core::{engine::AuthMethod, ids::{ProviderGameId,SessionId,TargetId}, ipc::{self,AuthFail,AuthState,EndReason,ErrorCode,Event,FailReason,FromEngine,Hello,InputRoute,Readiness,Request,SessionState,ToEngine}};
use pf_client_core::discovery::{self,DiscoveryEvent};
use punktfunk_core::{PunktfunkError,client::NativeClient,wol};
use std::{io::{self,Read,Write},path::Path,sync::{Arc,atomic::Ordering,mpsc::{self,Sender}},thread,time::Duration};
use store::{Host,Store};

pub(crate) enum Notice {
    Input(Option<Result<ToEngine,ParseError>>),
    Emit(FromEngine),
    Job {seq:u64,target:TargetId,pin:String,result:Result<Event>},
    Paired {generation:u64,result:Result<[u8;32],PunktfunkError>},
    Checked {generation:u64,result:Result<Readiness>},
    Finished(SessionId,SessionState),
}
struct Linking {target:TargetId,seq:u64,generation:u64}
struct Playing {id:SessionId,target:TargetId,game:ProviderGameId,seq:u64,generation:u64,control:Arc<session::Control>,thread:Option<thread::JoinHandle<()>>,checking:bool}
struct Engine {
    hello:Hello,store:Store,out:Sender<Notice>,link:Option<Linking>,pair_running:bool,generation:u64,
    play:Option<Playing>,jobs:usize,discovery:Option<(async_channel::Receiver<DiscoveryEvent>,discovery::Rescan)>,subscribed:bool,
}
#[derive(Debug)]
pub(crate) struct ParseError {seq:Option<u64>,code:ErrorCode,message:String}
impl ParseError {fn event(self)->FromEngine {error(self.seq,self.code,self.message)}}
fn error(seq:Option<u64>,code:ErrorCode,message:impl Into<String>)->FromEngine {FromEngine {re:seq,event:Event::Error {code,message:message.into()}}}
fn parse(line:&str)->Result<ToEngine,ParseError> {
    ipc::decode(line).map_err(|reason| {
        let raw=serde_json::from_str::<serde_json::Value>(line).ok();
        let seq=raw.as_ref().and_then(|r|r.get("seq")).and_then(|v|v.as_u64());
        let code=if reason.contains("unknown variant") {ErrorCode::Unsupported}else{ErrorCode::BadRequest};
        ParseError {seq,code,message:reason}
    })
}
fn stdin(out:Sender<Notice>) {
    let mut lines=ipc::LineBuffer::default();let mut buf=[0;8192];let mut input=io::stdin().lock();
    loop {
        match input.read(&mut buf) {
            Err(e) if e.kind()==io::ErrorKind::Interrupted=>continue,
            Ok(0)|Err(_)=>{let _=out.send(Notice::Input(None));break;}
            Ok(n)=>{
                let dropped=lines.dropped;
                lines.push(&buf[..n],|line| {let _=out.send(Notice::Input(Some(parse(line))));});
                if lines.dropped!=dropped {let _=out.send(Notice::Input(Some(Err(ParseError {seq:None,code:ErrorCode::BadRequest,message:"The IPC line is too large or is not UTF-8.".into()}))));}
            }
        }
    }
}
fn write(out:&mut impl Write,event:&FromEngine)->Result<()> {
    let line=ipc::encode(event);ensure!(line.len()<=ipc::MAX_LINE_BYTES,"An engine reply exceeded the IPC limit.");
    out.write_all(line.as_bytes())?;out.flush()?;Ok(())
}
pub fn run()->Result<()> {
    logging::install();
    punktfunk_core::tls::install_default_provider();
    punktfunk_core::client::set_client_label(concat!("LumaGame ",env!("CARGO_PKG_VERSION")));
    let (tx,rx)=mpsc::channel();
    let reader=tx.clone();thread::Builder::new().name("punktfunk-ipc".into()).spawn(move || stdin(reader))?;
    let mut stdout=io::stdout().lock();
    let (hello,seq)=loop { match rx.recv()? {
        Notice::Input(Some(Ok(ToEngine {seq,request:Request::Hello(hello)})))=>break (hello,seq),
        Notice::Input(None)=>return Ok(()),
        Notice::Input(Some(Err(e)))=>write(&mut stdout,&e.event())?,
        Notice::Input(Some(Ok(r)))=>write(&mut stdout,&error(Some(r.seq),ErrorCode::Protocol,"Hello must be the first request."))?,
        _=>{}
    }};
    if hello.protocol!=ipc::PROTOCOL {write(&mut stdout,&error(Some(seq),ErrorCode::Protocol,"This engine requires protocol 1."))?;return Ok(());}
    let store=match Store::open(Path::new(&hello.data_dir)) {Ok(s)=>s,Err(e)=>{write(&mut stdout,&error(Some(seq),ErrorCode::Internal,format!("The engine's private store is unavailable: {e:#}")))?;return Ok(());}};
    let mut engine=Engine {hello,store,out:tx,link:None,pair_running:false,generation:0,play:None,jobs:0,discovery:discovery::try_browse(),subscribed:false};
    write(&mut stdout,&FromEngine::answer(seq,Event::Ready {descriptor:settings::descriptor(),schema:settings::schema()}))?;
    let result=(||->Result<()> {
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Notice::Input(None))=>break,
                Ok(Notice::Input(Some(Err(e))))=>write(&mut stdout,&e.event())?,
                Ok(Notice::Input(Some(Ok(r))))=>{let seq=r.seq;if let Err(e)=engine.request(r) {engine.emit(error(Some(seq),ErrorCode::BadRequest,format!("{e:#}")));}},
                Ok(Notice::Emit(e))=>write(&mut stdout,&e)?,
                Ok(notice)=>engine.complete(notice),
                Err(mpsc::RecvTimeoutError::Timeout)=>{},
                Err(mpsc::RecvTimeoutError::Disconnected)=>break,
            }
            if let Some((events,_))=&engine.discovery {
                let events:Vec<_>=(0..128).filter_map(|_|events.try_recv().ok()).collect();
                for event in events {
                    match event {
                        DiscoveryEvent::Resolved(host)=>{if let Err(e)=engine.store.discover(host) {engine.emit(error(None,ErrorCode::Internal,e.to_string()));continue;}},
                        DiscoveryEvent::Removed {fullname}=>{for h in &mut engine.store.hosts {if h.fullname==fullname {h.online=Some(false);}}},
                    }
                    if engine.subscribed {engine.targets(None);}
                }
            }
        }
        Ok(())
    })();
    engine.shutdown();result
}
impl Engine {
    fn emit(&self,e:FromEngine) {let _=self.out.send(Notice::Emit(e));}
    fn reply(&self,seq:u64,event:Event) {self.emit(FromEngine::answer(seq,event));}
    fn targets(&self,seq:Option<u64>) {self.emit(FromEngine {re:seq,event:Event::Targets {targets:self.store.hosts.iter().map(Host::target).collect()}});}
    fn auth(&self,seq:u64,state:AuthState) {self.reply(seq,Event::Auth {state});}
    fn session(&self,id:SessionId,seq:u64,state:SessionState) {self.reply(seq,Event::Session {session:id,state});}
    fn request(&mut self,request:ToEngine)->Result<()> {
        let seq=request.seq;
        match request.request {
            Request::Hello(_)=>bail!("Hello has already been accepted."),
            Request::Settings {values}=>{self.hello.settings=settings::schema().resolve(&values);self.reply(seq,Event::Ready {descriptor:settings::descriptor(),schema:settings::schema()});}
            Request::Targets=>{self.subscribed=true;self.targets(Some(seq));}
            Request::AddTarget {address}=>{self.store.manual(&address)?;self.targets(Some(seq));}
            Request::Forget {target}=>{
                ensure!(self.play.as_ref().is_none_or(|p|p.target!=target),"Stop the session before unlinking this host.");
                if self.link.as_ref().is_some_and(|l|l.target==target) {self.cancel_auth();}
                self.store.forget(&target)?;self.targets(Some(seq));
            }
            Request::Wake {target}=>{
                let host=self.store.host(&target)?;
                let macs:Vec<_>=host.known.mac.iter().filter_map(|m|wol::parse_mac(m)).collect();
                ensure!(!macs.is_empty(),"This host has not advertised a wake address.");
                wol::send_magic_packet(&macs,host.known.addr.parse().ok())?;
                if let Some((_,r))=&self.discovery {r.request();}self.targets(Some(seq));
            }
            Request::Auth {target,method}=>{
                let target=target.or_else(||(self.store.hosts.len()==1).then(||self.store.hosts[0].id())).context("Choose a host to link.")?;
                self.begin_auth(seq,target,method)?;
            }
            Request::AuthPin {pin}=>{
                ensure!(pin.len()==4 && pin.bytes().all(|b|b.is_ascii_digit()),"Enter the four-digit PIN shown on the host.");
                let link=self.link.as_mut().context("There is no PIN link waiting.")?;
                ensure!(!self.pair_running,"A link attempt is already running.");link.seq=seq;
                self.pair(pin)?;
            }
            Request::AuthCancel=>{
                self.cancel_auth();self.auth(seq,AuthState::Failed {reason:AuthFail::Cancelled,message:"Link cancelled.".into()});
                if self.play.as_ref().is_some_and(|p|p.thread.is_none()) {self.fail_launch(FailReason::Auth,"Host linking was cancelled.");}
            }
            Request::Library {target,cursor}=>{
                ensure!(self.jobs<4,"Too many host requests are running.");let host=self.store.host(&target)?.clone();host.pin()?;
                let identity=self.store.identity.clone();let out=self.out.clone();let pin=host.known.fp_hex.clone();self.jobs+=1;
                thread::spawn(move || {
                    let result=(|| {let lib=library::Library::new(&host,&identity)?;let (mut entries,next)=lib.page(cursor.as_deref())?;let running=lib.running()?;
                        for e in &mut entries {e.resumable=running.iter().any(|id|id==e.id.as_str());}Ok(Event::Library {target:target.clone(),entries,next})})();
                    let _=out.send(Notice::Job {seq,target,pin,result});
                });
            }
            Request::Check {target,game}=>{
                ensure!(self.jobs<4,"Too many host requests are running.");let host=self.store.host(&target)?.clone();
                if host.pin().is_err() {self.reply(seq,Event::Readiness {target,game,readiness:Readiness::SignIn});return Ok(());}
                let identity=self.store.identity.clone();let out=self.out.clone();let pin=host.known.fp_hex.clone();self.jobs+=1;
                thread::spawn(move || {let readiness=library::Library::new(&host,&identity).and_then(|l|l.check(&game)).unwrap_or_else(|e|library::readiness_error(&e,&host));
                    let result=Ok(Event::Readiness {target:target.clone(),game,readiness});let _=out.send(Notice::Job {seq,target,pin,result});});
            }
            Request::Launch {session,target,game}=>{
                if self.play.is_some() {self.session(session,seq,SessionState::Failed {reason:FailReason::Busy,message:"A session is already running.".into()});return Ok(());}
                let paired=self.store.host(&target)?.known.paired;
                if !paired {ensure!(self.link.is_none() && !self.pair_running,"Finish the current link attempt first.");}
                ensure!(self.jobs<4,"Too many host requests are running.");
                self.generation=self.generation.wrapping_add(1);
                self.play=Some(Playing {id:session,target:target.clone(),game,seq,generation:self.generation,control:Arc::new(session::Control::default()),thread:None,checking:false});
                self.session(session,seq,SessionState::Checking);
                if paired {if let Err(e)=self.check_launch() {self.fail_launch(FailReason::Engine,&e.to_string());}} else {self.session(session,seq,SessionState::SignIn);self.begin_auth(seq,target,AuthMethod::EnterPin)?;}
            }
            Request::Stop {session,end_game}=>{
                let p=self.play.as_mut().filter(|p|p.id==session).context("There is no matching session.")?;
                p.seq=seq;p.control.end_game.store(end_game,Ordering::Release);p.control.stop.store(true,Ordering::Release);
                if p.thread.is_none() {self.cancel_auth();let p=self.play.take().unwrap();self.session(session,seq,SessionState::Ended {reason:EndReason::Stopped});drop(p);}
            }
            Request::Route {session,input}=>{let p=self.play.as_ref().filter(|p|p.id==session).context("There is no matching session.")?;p.control.app.store(input!=InputRoute::Game,Ordering::Release);}
            Request::Stats {session,on,pad_timing}=>{let p=self.play.as_ref().filter(|p|p.id==session).context("There is no matching session.")?;p.control.set_stats(on,pad_timing);}
            Request::RoundTrip {session}=>{
                let p=self.play.as_ref().filter(|p|p.id==session).context("There is no matching session.")?;
                let trip=p.control.trip.lock().unwrap().clone();
                self.reply(seq,Event::RoundTrip {session,trip});
            }
            Request::Action {key} if key=="rescan"=>{
                if self.discovery.is_none(){self.discovery=discovery::try_browse();}
                if let Some((_,r))=&self.discovery{r.request();}
                self.targets(Some(seq));
            }
            Request::Action {..}=>self.emit(error(Some(seq),ErrorCode::Unsupported,"Unknown settings action.")),
            Request::SteamOverlay {..}=>self.emit(error(Some(seq),ErrorCode::Unsupported,"This engine does not open Steam's overlay.")),
            Request::Recorder {session,action,title}=>{
                let p=self.play.as_ref().filter(|p|p.id==session).context("There is no matching session.")?;
                if let Err(text)=p.control.recorder(action,&title) {self.reply(seq,Event::Clip {session,saved:false,text});}
            }
            Request::Activity {..}=>self.emit(error(Some(seq),ErrorCode::Unsupported,"This engine sends no idle warning.")),
            Request::Achievements {..}=>self.emit(error(Some(seq),ErrorCode::Unsupported,"This engine has no achievements.")),
            Request::AuthKey {..}=>self.emit(error(Some(seq),ErrorCode::Unsupported,"This engine signs in without a key.")),
            Request::Community {..} | Request::CommunityThread {..}=>self.emit(error(Some(seq),ErrorCode::Unsupported,"This engine has no community pages.")),
        }
        Ok(())
    }
    fn begin_auth(&mut self,seq:u64,target:TargetId,method:AuthMethod)->Result<()> {
        ensure!(self.link.is_none() && !self.pair_running,"A link attempt is already running.");self.store.host(&target)?;
        ensure!(self.play.as_ref().is_none_or(|p|p.thread.is_none() && !p.checking),"Stop the session before linking a host.");
        ensure!(method==AuthMethod::EnterPin,"This host links with the PIN shown on its screen.");
        self.generation=self.generation.wrapping_add(1);
        self.link=Some(Linking {target,seq,generation:self.generation});
        self.auth(seq,AuthState::EnterPin {digits:4});
        Ok(())
    }
    fn pair(&mut self,pin:String)->Result<()> {
        let l=self.link.as_ref().context("No link waiting.")?;let host=self.store.host(&l.target)?.clone();let identity=self.store.identity.clone();
        let generation=l.generation;let out=self.out.clone();self.pair_running=true;
        thread::spawn(move || {
            let result=NativeClient::pair(&host.known.addr,host.known.port,(&identity.0,&identity.1),&pin,"LumaGame TV",Duration::from_secs(10));
            let _=out.send(Notice::Paired {generation,result});
        });Ok(())
    }
    fn cancel_auth(&mut self) {self.link.take();}
    fn check_launch(&mut self)->Result<()> {
        ensure!(self.jobs<4,"Too many host requests are running.");
        let p=self.play.as_mut().context("No launch pending.")?;let host=self.store.host(&p.target)?.clone();let game=p.game.clone();let identity=self.store.identity.clone();let out=self.out.clone();let generation=p.generation;p.checking=true;self.jobs+=1;
        thread::spawn(move || {let result=library::Library::new(&host,&identity).and_then(|l|l.check(&game)).or_else(|e| {let r=library::readiness_error(&e,&host);if r==Readiness::Unknown {Err(e)} else {Ok(r)}});let _=out.send(Notice::Checked {generation,result});});Ok(())
    }
    fn fail_launch(&mut self,reason:FailReason,message:&str) {if let Some(p)=self.play.take() {self.session(p.id,p.seq,SessionState::Failed {reason,message:message.into()});}}
    fn complete(&mut self,notice:Notice) {
        match notice {
            Notice::Emit(e)=>self.emit(e),
            Notice::Job {seq,target,pin,result}=>{
                self.jobs=self.jobs.saturating_sub(1);
                if !self.store.host(&target).is_ok_and(|h|h.known.fp_hex==pin) {self.emit(error(Some(seq),ErrorCode::BadRequest,"The host link changed during the request."));return;}
                match result {Ok(event)=>self.reply(seq,event),Err(e)=>self.emit(error(Some(seq),ErrorCode::BadRequest,format!("{e:#}")))}
            }
            Notice::Paired {generation,result}=>{
                self.pair_running=false;
                if self.link.as_ref().is_none_or(|l|l.generation!=generation) {return;}
                let link=self.link.take().unwrap();
                match result {
                    Ok(pin)=>match self.store.paired(&link.target,pin) {
                        Ok(())=>{
                            self.auth(link.seq,AuthState::Done {target:link.target.clone()});self.targets(None);
                            if self.play.as_ref().is_some_and(|p|p.target==link.target) && let Err(e)=self.check_launch() {self.fail_launch(FailReason::Engine,&e.to_string());}
                        }
                        Err(e)=>{self.auth(link.seq,AuthState::Failed {reason:AuthFail::Network,message:format!("Could not save the host's verified identity: {e}")});self.fail_launch(FailReason::Auth,"The verified host identity could not be saved.");}
                    },
                    Err(e)=>{let reason=match e {PunktfunkError::Crypto=>AuthFail::WrongPin,PunktfunkError::Timeout=>AuthFail::Expired,PunktfunkError::Rejected(..)=>AuthFail::Denied,_=>AuthFail::Network};
                        self.auth(link.seq,AuthState::Failed {reason,message:e.to_string()});self.fail_launch(FailReason::Auth,"Host linking failed.");}
                }
            }
            Notice::Checked {generation,result}=>{
                self.jobs=self.jobs.saturating_sub(1);
                let Some(p)=self.play.as_mut().filter(|p|p.generation==generation && p.checking) else {return};p.checking=false;
                let session=p.id;
                let readiness=match result {Ok(r)=>r,Err(e)=>{self.fail_launch(FailReason::Network,&e.to_string());return;}};
                if readiness!=Readiness::Ready {let reason=match readiness {Readiness::SignIn=>FailReason::Auth,Readiness::Install=>FailReason::NotInstalled,Readiness::Offline|Readiness::Asleep=>FailReason::Offline,_=>FailReason::Network};self.fail_launch(reason,"The host cannot start this game yet.");return;}
                let host=match self.store.host(&p.target) {Ok(h)=>h.clone(),Err(e)=>{self.fail_launch(FailReason::Engine,&e.to_string());return;}};
                let game=p.game.clone();let identity=self.store.identity.clone();let hello=self.hello.clone();let control=Arc::clone(&p.control);let out=self.out.clone();
                p.thread=Some(thread::spawn(move ||session::run(session,host,game,identity,hello,control,out)));
            }
            Notice::Finished(id,state)=>{if self.play.as_ref().is_some_and(|p|p.id==id) {let p=self.play.take().unwrap();if let Some(t)=p.thread {let _=t.join();}self.session(id,p.seq,state);}}
            Notice::Input(_)=>unreachable!("Input is dispatched on the IPC thread"),
        }
    }
    fn shutdown(&mut self) {
        self.cancel_auth();self.discovery.take();
        if let Some(p)=self.play.take() {p.control.stop.store(true,Ordering::Release);if let Some(t)=p.thread {let _=t.join();}}
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn protocol_distinguishes_unknown_requests_from_invalid_values() {
        assert_eq!(parse(r#"{"seq":7,"type":"future"}"#).unwrap_err().code,ErrorCode::Unsupported);
        assert_eq!(parse(r#"{"seq":8,"type":"wake","target":"bad|id"}"#).unwrap_err().seq,Some(8));
    }
    fn engine()->(tempfile::TempDir,Engine,mpsc::Receiver<Notice>) {
        let dir=tempfile::tempdir().unwrap();
        let store=Store::open(dir.path()).unwrap();let (out,events)=mpsc::channel();
        let hello=serde_json::from_value(serde_json::json!({"protocol":1,"app_version":"test","data_dir":dir.path(),"cache_dir":dir.path(),
            "locale":"en","display":{"width":3840,"height":2160,"refresh_hz":120,"hdr10":true,"codecs":["h264","h265","av1"]}})).unwrap();
        (dir,Engine {hello,store,out,link:None,pair_running:false,generation:0,play:None,jobs:0,discovery:None,subscribed:false},events)
    }
    #[test] fn link_is_not_launch_and_cancel_discards_a_late_pairing() {
        let (_dir,mut e,_events)=engine();let target=e.store.manual("192.0.2.1").unwrap();
        e.request(ToEngine {seq:1,request:Request::Auth {target:Some(target.clone()),method:AuthMethod::EnterPin}}).unwrap();
        let generation=e.link.as_ref().unwrap().generation;
        e.complete(Notice::Paired {generation,result:Ok([3;32])});
        assert!(e.play.is_none());assert_eq!(e.store.host(&target).unwrap().pin().unwrap(),[3;32]);
        e.request(ToEngine {seq:2,request:Request::Auth {target:Some(target.clone()),method:AuthMethod::EnterPin}}).unwrap();
        let generation=e.link.as_ref().unwrap().generation;
        e.request(ToEngine {seq:3,request:Request::AuthCancel}).unwrap();
        e.complete(Notice::Paired {generation,result:Ok([4;32])});
        assert_eq!(e.store.host(&target).unwrap().pin().unwrap(),[3;32]);
    }
    #[test] fn busy_launch_preserves_the_first_and_stop_cancels_its_link() {
        let (_dir,mut e,events)=engine();let target=e.store.manual("192.0.2.1").unwrap();let game=ProviderGameId::new("steam:570").unwrap();
        e.request(ToEngine {seq:1,request:Request::Launch {session:SessionId(1),target:target.clone(),game:game.clone()}}).unwrap();
        e.request(ToEngine {seq:2,request:Request::Launch {session:SessionId(2),target,game}}).unwrap();
        assert_eq!(e.play.as_ref().unwrap().id,SessionId(1));
        assert!(events.try_iter().any(|n|matches!(n,Notice::Emit(FromEngine {re:Some(2),event:Event::Session {session:SessionId(2),state:SessionState::Failed {reason:FailReason::Busy,..}}}))));
        e.request(ToEngine {seq:3,request:Request::Stop {session:SessionId(1),end_game:false}}).unwrap();
        assert!(e.play.is_none() && e.link.is_none());
    }
    #[test] fn round_trip_answers_from_the_sessions_latest_legs() {
        let (_dir,mut e,events)=engine();let target=e.store.manual("192.0.2.1").unwrap();
        e.request(ToEngine {seq:1,request:Request::Launch {session:SessionId(1),target,game:ProviderGameId::new("steam:570").unwrap()}}).unwrap();
        *e.play.as_ref().unwrap().control.trip.lock().unwrap()=ipc::RoundTrip {network_ms:Some(3.5),..ipc::RoundTrip::default()};
        e.request(ToEngine {seq:2,request:Request::RoundTrip {session:SessionId(1)}}).unwrap();
        assert!(events.try_iter().any(|n|matches!(n,Notice::Emit(FromEngine {re:Some(2),event:Event::RoundTrip {trip,..}}) if trip.network_ms==Some(3.5))));
        assert!(e.request(ToEngine {seq:3,request:Request::RoundTrip {session:SessionId(9)}}).is_err());
    }
}
