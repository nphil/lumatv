#[cfg(test)]
mod feeder_tests;
use anyhow::{Context, Result, bail, ensure};
use lumagame_core::{engine::Codec, ids::{ProviderGameId, SessionId}, ipc::{EndReason, Event, FailReason, FromEngine, Hello, RoundTrip, SessionState, Stats, StreamInfo}};
use lumagame_feeder::{FeederClient, FeederMessage, Hdr, Load, audio, msg, pts_unit};
use lumagame_recorder::{HttpDestination, RING_CAP, Recorder};
use lumatv_gfn_pad::PadRuntime;
use punktfunk_core::{client::NativeClient, packet::{FLAG_SOF,USER_FLAG_RECOVERY_ANCHOR}, quic, PunktfunkError};
use std::{path::Path, sync::{Arc, Mutex, atomic::{AtomicBool,AtomicU64,Ordering}, mpsc::Sender}, thread, time::{Duration,Instant}};
use crate::{Notice, input::Pads, library::{Library,DESKTOP}, settings, store::Host};
pub mod stage;

#[derive(Default)]
pub struct Control { pub stop:Arc<AtomicBool>, pub end_game:AtomicBool, pub app:AtomicBool, pub stats:AtomicBool, pad_stats:AtomicBool, pub trip:Mutex<RoundTrip>, recorder:Mutex<Option<(Arc<Recorder>,u32)>> }
impl Control {
    /// Asks the recorder for a clip; the outcome comes later as a `clip` event, a refusal now.
    pub fn save_clip(&self)->Result<(),String> {
        let guard=self.recorder.lock().unwrap();
        let (recorder,seconds)=guard.as_ref().ok_or("Turn on Save clips and set the receiver address in Settings")?;
        recorder.save_last(*seconds).map_err(|e|e.to_string())
    }
    pub fn set_stats(&self,on:bool,pad_timing:bool) {
        self.pad_stats.store(on && pad_timing,Ordering::Release);
        self.stats.store(on,Ordering::Release);
    }
}
#[derive(Default)]
struct Player { loaded:AtomicBool, accepted:AtomicU64, dropped:AtomicU64, keyframe:AtomicBool, fatal:Mutex<Option<String>> }
impl Player {
    fn event(&self, event:Option<FeederMessage>) {
        match event {
            Some(FeederMessage::Loaded {ok:false,text})|Some(FeederMessage::Error {text,..}) => { *self.fatal.lock().unwrap()=Some(text); }
            Some(FeederMessage::Event {kind:22,..}) => {self.loaded.store(true,Ordering::Release);}
            Some(FeederMessage::Accept(_)) => {self.accepted.fetch_add(1,Ordering::Relaxed);}
            Some(FeederMessage::Drop(_)) => {self.dropped.fetch_add(1,Ordering::Relaxed);self.keyframe.store(true,Ordering::Release);}
            Some(FeederMessage::Keyframe) => {self.keyframe.store(true,Ordering::Release);}
            None => {*self.fatal.lock().unwrap()=Some("The video feeder disconnected.".into());}
            _=>{}
        }
    }
    fn check(&self) -> Result<()> { if let Some(e)=self.fatal.lock().unwrap().as_ref() {bail!("{e}")} Ok(()) }
}

pub fn monotonic_ns() -> u64 {
    let mut t=libc::timespec {tv_sec:0,tv_nsec:0};
    // CLOCK_MONOTONIC is shared by the 64-bit engine and 32-bit feeder, unlike Rust Instant's opaque epoch.
    let rc=unsafe {libc::clock_gettime(libc::CLOCK_MONOTONIC,&mut t)};
    assert_eq!(rc,0,"CLOCK_MONOTONIC");
    t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64
}
fn emit(out:&Sender<Notice>,id:SessionId,state:SessionState) { let _=out.send(Notice::Emit(FromEngine::told(Event::Session {session:id,state}))); }

pub fn run(id:SessionId,host:Host,game:ProviderGameId,identity:(String,String),hello:Hello,control:Arc<Control>,out:Sender<Notice>) {
    let result=stream(id,&host,&game,&identity,&hello,&control,&out);
    let state=match result {
        Ok(reason)=>SessionState::Ended {reason},
        Err((reason,error))=>SessionState::Failed {reason,message:format!("{error:#}")},
    };
    let _=out.send(Notice::Finished(id,state));
}

type Failure=(FailReason,anyhow::Error);
fn stream(id:SessionId,host:&Host,game:&ProviderGameId,identity:&(String,String),hello:&Hello,control:&Arc<Control>,out:&Sender<Notice>) -> Result<EndReason,Failure> {
    let pin=host.pin().map_err(|e|(FailReason::Auth,e))?;
    let mut params=settings::params(&hello.display,&hello.settings,&host.known.addr,host.known.port).map_err(|e|(FailReason::Video,e))?;
    params.pin=Some(pin); params.identity=Some(identity.clone()); params.cancel=Some(Arc::clone(&control.stop));
    params.launch=(game.as_str()!=DESKTOP).then(||game.to_string());
    let smooth=settings::text(&settings::schema().resolve(&hello.settings),"pacing")=="smooth";
    emit(out,id,SessionState::Starting);
    let client=match NativeClient::connect(params) {
        Ok(c)=>Arc::new(c),
        Err(_) if control.stop.load(Ordering::Acquire)=>return Ok(EndReason::Stopped),
        Err(e)=>return Err((connect_reason(&e),e.into())),
    };
    client.set_hud_enabled(false);
    let result=media(id,Arc::clone(&client),hello,control,out,smooth);
    let ending=if control.end_game.load(Ordering::Acquire) {
        Library::new(host,identity).and_then(|lib|lib.end_game(game)).map_err(|e|(FailReason::Network,e))
    } else {Ok(())};
    // Code 0 stops this stream but leaves the far-side game available to resume.
    client.close();
    ending?;
    result
}
fn connect_reason(e:&PunktfunkError)->FailReason {
    match e {PunktfunkError::Crypto=>FailReason::Auth,PunktfunkError::Timeout=>FailReason::Timeout,_=>FailReason::Network}
}
fn media(id:SessionId,client:Arc<NativeClient>,hello:&Hello,control:&Arc<Control>,out:&Sender<Notice>,smooth:bool)->Result<EndReason,Failure> {
    let video=|e| (FailReason::Video,e);
    let path=hello.feeder.as_deref().context("The app did not supply its video feeder.").map_err(video)?;
    let (mut feeder,greeting)=FeederClient::connect(Path::new(path),Duration::from_secs(3)).map_err(|e|video(e.into()))?;
    if !matches!(greeting,FeederMessage::Hello {library:true,..}) {return Err(video(anyhow::anyhow!("The video decoder is unavailable.")));}
    let player=Arc::new(Player::default());
    let shared=Arc::clone(&player);
    feeder.spawn_reader(move |m|shared.event(m)).map_err(|e|video(e.into()))?;
    let format=client.video_format();
    let info=stream_info(&client,format).map_err(video)?;
    let hdr=client.latest_hdr_meta(Duration::ZERO).map(|m|metadata(m,format.color));
    if let Some(recorder)=tee(id,hello,control,out) {feeder.set_tee(recorder);}
    feeder.send_load(&load(&info,hdr)).map_err(|e|video(e.into()))?;
    let feeder=Arc::new(Mutex::new(feeder));
    let audio_stop=Arc::new(AtomicBool::new(false));
    let ac=Arc::clone(&client); let af=Arc::clone(&feeder); let ap=Arc::clone(&player); let stop=Arc::clone(&audio_stop);
    let audio=thread::Builder::new().name("punktfunk-audio".into()).spawn(move || {
        while !stop.load(Ordering::Acquire) {
            match ac.next_audio(Duration::from_millis(100)) {
                Ok(packet) if ap.loaded.load(Ordering::Acquire)=>{
                    if let Err(e)=af.lock().unwrap().send_media(msg::AUDIO,monotonic_ns() as i64,false,&packet.data) {
                        *ap.fatal.lock().unwrap()=Some(e.to_string()); break;
                    }
                }
                Ok(_)|Err(PunktfunkError::NoFrame)=>{},
                Err(PunktfunkError::Closed)=>break,
                Err(e)=>{*ap.fatal.lock().unwrap()=Some(format!("Audio transport: {e}"));break;}
            }
        }
    }).map_err(|e|video(e.into()))?;
    let pads=Arc::new(Pads::new(Arc::clone(&client),id,out.clone()));
    let pad_runtime=PadRuntime::start(settings::pad_config(&hello.settings),pads.clone());
    pad_runtime.set_grab(true);
    let result=video_loop(id,&client,&feeder,&player,control,out,&pads,&pad_runtime,info,smooth,hdr);
    pads.route(true);
    for slot in 0..4 { let _=pad_runtime.rumble(slot,0,0,0); }
    pad_runtime.set_grab(false);pad_runtime.stop();
    audio_stop.store(true,Ordering::Release);
    let _=audio.join();
    let _=feeder.lock().unwrap().send_unload();
    *control.recorder.lock().unwrap()=None;
    result
}
/// The game recorder for this stream when the viewer turned it on; what becomes of a clip goes to the app as `clip` events.
fn tee(id:SessionId,hello:&Hello,control:&Control,out:&Sender<Notice>)->Option<Arc<Recorder>> {
    let rec=settings::recorder(&hello.settings);
    if !rec.runs() {return None;}
    let tell={let out=out.clone();move |saved:bool,text:String| {let _=out.send(Notice::Emit(FromEngine::told(Event::Clip {session:id,saved,text})));}};
    let destination=match HttpDestination::new(&rec.url,&rec.folder) {Ok(d)=>d,Err(why)=>{tell(false,why);return None;}};
    let recorder=Arc::new(Recorder::new(RING_CAP,"lumagame",Arc::new(destination),move |event| if let Some((saved,text))=event.sentence() {tell(saved,text);}));
    *control.recorder.lock().unwrap()=Some((Arc::clone(&recorder),rec.seconds));
    Some(recorder)
}

#[allow(clippy::too_many_arguments)]
fn video_loop(id:SessionId,client:&NativeClient,feeder:&Mutex<FeederClient>,player:&Player,control:&Control,out:&Sender<Notice>,pads:&Pads,pad_runtime:&PadRuntime,mut info:StreamInfo,smooth:bool,mut last_hdr:Option<Hdr>)->Result<EndReason,Failure> {
    let video=|e| (FailReason::Video,e);
    let mut stage=stage::Stage::new(info.fps,smooth,codec_bit(info.codec));
    let mut loss=stage::Loss::new(client.frames_dropped());
    let mut start=Instant::now(); let mut running=false;
    let mut since=Instant::now(); let mut measured=false; let mut pad_measured=false; let mut frames=0u64; let mut bytes=0u64; let mut client_us=0u64; let mut timings=0u64;
    let mut host_us=None;
    let mut dropped=0;
    let mut app=false;
    let mut launch_check=Instant::now()-Duration::from_secs(1);
    let mut counted=Instant::now();
    loop {
        if control.stop.load(Ordering::Acquire) { return Ok(if control.end_game.load(Ordering::Acquire) {EndReason::Stopped}else{EndReason::Suspended}); }
        player.check().map_err(video)?;
        let next_app=control.app.load(Ordering::Acquire);
        if app!=next_app {
            pads.route(next_app);app=next_app;
            if app {for slot in 0..4 {let _=pad_runtime.rumble(slot,0,0,0);}}
        }
        if pads.failed.load(Ordering::Relaxed) {return Err((FailReason::Network,anyhow::anyhow!("The host stopped accepting controller input.")));}
        if player.keyframe.swap(false,Ordering::AcqRel) {
            let count=player.dropped.load(Ordering::Relaxed);
            if count!=dropped {stage.decoder_loss(monotonic_ns());dropped=count;} else {stage.hold(monotonic_ns(),false);}
            if stage.keyframe_due(monotonic_ns()) {client.request_keyframe().map_err(|e|(FailReason::Network,e.into()))?;}
        }
        while let Ok(r)=client.next_rumble_command(Duration::ZERO) {
            if !control.app.load(Ordering::Acquire) { let _=pad_runtime.rumble(r.pad as u8,r.low,r.high,r.backstop_ms); }
        }
        while let Ok(h)=client.next_hidout(Duration::ZERO) { pads.feedback(h); }
        let stats=control.stats.load(Ordering::Acquire);
        if stats != measured {client.set_hud_enabled(stats);measured=stats;since=Instant::now();frames=0;bytes=0;client_us=0;timings=0;host_us=None;}
        let pad_stats=control.pad_stats.load(Ordering::Acquire);
        if pad_stats != pad_measured {pads.sample(pad_stats);pad_measured=pad_stats;}
        while let Ok(t)=client.next_host_timing(Duration::ZERO) { if stats { host_us=Some(t.host_us); } }
        // Drain repeats even during SDR; only real HDR10 ever reaches NDL's mode-changing call.
        let meta=client.latest_hdr_meta(Duration::ZERO);
        if info.hdr && let Some(meta)=meta {
            let hdr=metadata(meta,client.video_format().color);
            if last_hdr!=Some(hdr) {feeder.lock().unwrap().send_hdr(&hdr).map_err(|e|video(e.into()))?;last_hdr=Some(hdr);}
        }
        if launch_check.elapsed()>=Duration::from_millis(250) {
            launch_check=Instant::now();
            if let Some(outcome)=client.launch_outcome()
                && matches!(outcome.kind,quic::LaunchOutcomeKind::Refused|quic::LaunchOutcomeKind::Failed) {
                return Err((FailReason::NotInstalled,anyhow::anyhow!("The host could not launch the game: {}",outcome.message)));
            }
        }
        if !running && player.loaded.load(Ordering::Acquire) && player.accepted.load(Ordering::Relaxed)>0 {
            emit(out,id,SessionState::Running {stream:Some(info.clone())});running=true;
        }
        if !player.loaded.load(Ordering::Acquire) && start.elapsed()>Duration::from_secs(15) {return Err(video(anyhow::anyhow!("The video decoder did not finish loading.")));}
        if !running && start.elapsed()>Duration::from_secs(30) {return Err((FailReason::Timeout,anyhow::anyhow!("The host did not send a playable picture.")));}
        if stats && since.elapsed()>=Duration::from_secs(1) {
            let seconds=since.elapsed().as_secs_f32();
            let network=client.hud_snapshot();
            let total=u64::from(network.received)+u64::from(network.lost);
            let snapshot=Stats {rtt_ms:Some(client.rtt_us() as f32 / 1000.0),jitter_ms:Some(stage.pacing.jitter_ms()),
                loss_pct:(total>0).then(||network.lost as f32*100.0/total as f32),
                bitrate_kbps:Some((bytes as f64*8.0/f64::from(seconds)/1000.0) as u32),fps:Some(frames as f32/seconds),
                dropped:Some(client.frames_dropped()+player.dropped.load(Ordering::Relaxed)),socket_drops:client.socket_drops(),
                client_ms:(timings>0).then(||client_us as f32/timings as f32/1000.0),host_ms:host_us.map(|u|u as f32/1000.0),pad:pads.timing(monotonic_ns()),..Stats::default()};
            *control.trip.lock().unwrap()=trip(&snapshot,&network,pads.presses());
            let _=out.send(Notice::Emit(FromEngine::told(Event::Stats {session:id,stats:snapshot})));
            since=Instant::now();frames=0;bytes=0;client_us=0;timings=0;
        }
        // A rising socket count means this TV threw packets away at the stream's receive buffer; the network did not lose them.
        if counted.elapsed()>=Duration::from_secs(10) {
            counted=Instant::now();
            tracing::info!("receive counters: socket drops {}, receive buffer {} KiB, frames dropped {}",
                client.socket_drops().map_or("not counted".to_owned(),|n|n.to_string()),client.recv_buffer_kb(),client.frames_dropped());
        }
        let frame=match client.next_frame(Duration::from_millis(20)) {
            Ok(f)=>f,
            Err(PunktfunkError::NoFrame)=>continue,
            Err(PunktfunkError::Closed) if client.end_reason().is_normal()=>return Ok(EndReason::Remote),
            Err(e)=>return Err((FailReason::Network,e.into())),
        };
        if loss.late(frame.frame_index) {continue;}
        if stats {frames+=1;bytes+=frame.data.len() as u64;}
        if let Some(format)=frame.new_format {
            info=stream_info(client,format).map_err(video)?;
            player.loaded.store(false,Ordering::Release);player.accepted.store(0,Ordering::Relaxed);
            last_hdr=meta.map(|m|metadata(m,format.color));
            let mut f=feeder.lock().unwrap();f.send_unload().map_err(|e|video(e.into()))?;f.send_load(&load(&info,last_hdr)).map_err(|e|video(e.into()))?;
            stage=stage::Stage::new(info.fps,smooth,format.codec);start=Instant::now();
        }
        stage.pacing.rate(client.mode().refresh_hz);
        let now=monotonic_ns();
        let gap=client.note_frame_index(frame.frame_index);
        let (lost,index)=loss.observe(frame.frame_index,gap,client.frames_dropped(),now);
        let anchor=stage.anchor(frame.flags & u32::from(FLAG_SOF)!=0,frame.flags & USER_FLAG_RECOVERY_ANCHOR!=0);
        if !frame.complete || frame.part.is_some() {stage.hold(now,true);continue;}
        let gate=stage.gate(anchor,lost,now);
        match gate {
            stage::Gate::Keyframe=>{client.request_keyframe().map_err(|e|(FailReason::Network,e.into()))?;continue;}
            stage::Gate::Hold=>continue,
            _=>{}
        }
        let stamp=stage.pacing.map(frame.pts_ns,now);
        let mut f=feeder.lock().unwrap();
        if gate==stage::Gate::Resume {
            let bridges=stage.bridges(&frame.data,index);
            for (i,b) in bridges.iter().enumerate() {
                f.send_media(msg::VIDEO,stamp.saturating_sub((bridges.len()-i) as u64*1_000_000) as i64,true,b).map_err(|e|video(e.into()))?;
            }
        }
        f.send_media(msg::VIDEO,stamp as i64,anchor,&frame.data).map_err(|e|video(e.into()))?;
        drop(f);
        stage.fed(&frame.data,index);
        if stats && frame.received_ns>0 {
            let elapsed=punktfunk_core::client::now_realtime_ns()-i128::from(frame.received_ns);
            if (0..10_000_000_000).contains(&elapsed) {client_us+=elapsed as u64/1000;timings+=1;}
        }
    }
}
/// What `round-trip` answers from this second's measurements. Capture to arrival needs the host's clock offset; without one the
/// figure would compare two unrelated clocks.
fn trip(stats:&Stats,network:&punktfunk_core::hud::StatsSnapshot,presses:Option<(u32,u32,u32)>)->RoundTrip {
    let ms=|us:u32| us as f32/1000.0;
    let arrival=(network.host_net.is_measured() && !network.same_host_clock).then_some(&network.host_net);
    RoundTrip {presses:presses.map(|p|p.0),press_p50_ms:presses.map(|p|ms(p.1)),press_p99_ms:presses.map(|p|ms(p.2)),
        network_ms:stats.rtt_ms,host_ms:stats.host_ms,arrival_p50_ms:arrival.map(|a|ms(a.p50_us)),arrival_p95_ms:arrival.map(|a|ms(a.p95_us)),
        client_ms:stats.client_ms}
}
fn codec_bit(codec:Codec)->u8 {match codec {Codec::H264=>1,Codec::H265=>2,Codec::Av1=>4,_=>0}}
fn stream_info(client:&NativeClient,format:quic::VideoFormat)->Result<StreamInfo> {
    let codec=match format.codec {1=>Codec::H264,2=>Codec::H265,4=>Codec::Av1,_=>bail!("The host chose an unsupported codec.")};
    ensure!(client.welcome().audio_channels<=2,"The feeder accepts only mono or stereo Opus.");
    ensure!(format.chroma_format==quic::CHROMA_IDC_420,"The TV cannot decode 4:4:4 video.");
    let mode=client.mode();
    ensure!(!format.color.is_hdr() || (codec==Codec::H265 && format.color.transfer==16),"The TV supports HEVC HDR10 only.");
    Ok(StreamInfo {codec,width:mode.width,height:mode.height,fps:mode.refresh_hz,hdr:format.color.is_hdr(),audio_channels:client.welcome().audio_channels,sdr_reason:None,vui:None})
}
fn load(info:&StreamInfo,hdr:Option<Hdr>)->Load {
    Load {codec:u32::from(codec_bit(info.codec)),width:info.width,height:info.height,fps:info.fps,audio:if info.audio_channels==0 {audio::NONE}else{audio::OPUS},channels:u32::from(info.audio_channels),sample_rate:48000,
        hdr:u32::from(info.hdr && hdr.is_some()),hdr_info:hdr.unwrap_or_default(),drop_threshold:-1,pts_unit:pts_unit::MS}
}
fn metadata(m:quic::HdrMeta,c:quic::ColorInfo)->Hdr {
    Hdr {display_primaries_x0:u32::from(m.display_primaries[0][0]),display_primaries_y0:u32::from(m.display_primaries[0][1]),
        display_primaries_x1:u32::from(m.display_primaries[1][0]),display_primaries_y1:u32::from(m.display_primaries[1][1]),
        display_primaries_x2:u32::from(m.display_primaries[2][0]),display_primaries_y2:u32::from(m.display_primaries[2][1]),
        white_point_x:u32::from(m.white_point[0]),white_point_y:u32::from(m.white_point[1]),max_display_mastering_luminance:m.max_display_mastering_luminance,
        min_display_mastering_luminance:m.min_display_mastering_luminance,max_content_light_level:u32::from(m.max_cll),max_pic_average_light_level:u32::from(m.max_fall),
        transfer_characteristics:u32::from(c.transfer),color_primaries:u32::from(c.primaries),matrix_coeffs:u32::from(c.matrix)}
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn controller_timing_is_independent_of_menu_statistics() {
        let control=Control::default();
        for (on,pad_timing,expected) in [(true,false,false),(true,true,true),(true,false,false),(false,true,false),(true,true,true),(false,false,false)] {
            control.set_stats(on,pad_timing);
            assert_eq!(control.stats.load(Ordering::Acquire),on);
            assert_eq!(control.pad_stats.load(Ordering::Acquire),expected);
        }
    }
    #[test] fn feeder_completion_and_first_accepted_picture_are_separate() {
        let p=Player::default();
        p.event(Some(FeederMessage::Loaded {ok:true,text:String::new()}));assert!(!p.loaded.load(Ordering::Acquire));
        p.event(Some(FeederMessage::Event {kind:22,num:0,text:String::new()}));assert!(p.loaded.load(Ordering::Acquire));
        p.event(Some(FeederMessage::Keyframe));assert!(p.keyframe.load(Ordering::Acquire));
        p.event(Some(FeederMessage::Error {code:1,text:"decoder failed".into()}));assert!(p.check().is_err());
    }
    #[test] fn sdr_never_sets_hdr_even_with_queued_metadata() {
        let s=StreamInfo {codec:Codec::H265,width:1920,height:1080,fps:120,hdr:false,audio_channels:2,sdr_reason:None,vui:None};
        assert_eq!(load(&s,Some(Hdr::hdr10(1000,400))).hdr,0);
        assert_eq!(load(&s,None).audio,audio::OPUS);
    }
}
