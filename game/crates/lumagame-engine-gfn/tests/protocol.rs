use lumagame_core::engine::{AuthMethod, Codec};
use lumagame_core::ids::{ProviderGameId, SessionId, TargetId};
use lumagame_core::ipc::*;
use lumagame_core::settings::Values;
use lumagame_engine_gfn::Adapter;
use serde_json::{Value,json};
use std::collections::VecDeque;

struct FakeCore {
    adapter: Adapter,
    requests: VecDeque<Value>,
    events: Vec<FromEngine>,
    receipts: Vec<Value>,
}

fn envelope(generation:u64) -> Value {
    json!({"generation":generation,"session":{"user":{"userId":"u","displayName":"Nitin","membershipTier":"FREE"},"provider":{"idpId":"nvidia"}}})
}
fn scope(generation:u64) -> Value { json!({"generation":generation,"userId":"u","providerIdpId":"nvidia"}) }
fn game() -> Value {
    json!({"id":"parent","title":"A Game","imageUrl":"https://cdn.example/cover.jpg","variants":[
        {"id":"123","store":"STEAM","inLibrary":true,"storeUrl":"https://store.steampowered.com/app/10/","features":["hdr"]}]})
}
fn provider() -> ProviderGameId { ProviderGameId::new("[\"parent\",\"123\"]").unwrap() }

impl FakeCore {
    fn new(signed_in:bool) -> Self {
        let hello = Hello {
            protocol:1,app_version:"test".into(),data_dir:"/data/engines/gfn".into(),cache_dir:"/data/cache/gfn".into(),
            feeder:Some("/run/game/feed.sock".into()),locale:"en-US".into(),
            display:Display { width:3840,height:2160,refresh_hz:120,hdr10:true,codecs:vec![Codec::H265,Codec::Av1] },settings:Values::new(),
        };
        let adapter = Adapter::new(hello,1,&Value::Null,"/app/engines/gfn/opennow-streamer".into());
        let mut fake = Self {adapter,requests:VecDeque::new(),events:Vec::new(),receipts:Vec::new()};
        fake.reply("core.hello",json!({"protocolVersion":5}));
        fake.settings();
        fake.reply("streamer.detect",json!({"available":true,"capabilities":{"protocolVersion":7,"videoBackends":[{"backend":"ndl"}]}}));
        fake.reply("auth.session.get",if signed_in {envelope(1)} else {json!({"generation":1,"session":null})});
        if signed_in {
            fake.reply("account.subscription.get",json!({"subscription":{"membershipTier":"ULTIMATE"},"scope":scope(2)}));
            fake.reply("network.regions.list",json!({"regions":[{"name":"EU","url":"https://stream.example"}]}));
        }
        fake
    }
    fn collect(&mut self) {
        let batch = self.adapter.drain();
        self.events.extend(batch.app);
        for line in batch.core {
            let v:Value = serde_json::from_str(&line).unwrap();
            if v["type"] == "ack" {self.receipts.push(v);} else {self.requests.push_back(v);}
        }
    }
    fn next(&mut self, method:&str) -> Value {
        self.collect();
        let request = self.requests.pop_front().unwrap_or_else(|| panic!("expected {method}"));
        assert_eq!(request["method"],method,"{request}");
        request
    }
    /// The core answers and announces the streamer as `{"streamer": {...}}`.
    fn core_shape(method:&str, value:Value) -> Value {
        if matches!(method,"streamer.start"|"streamer.stop"|"streamer.status.get"|"streamer.changed") {json!({"streamer":value})} else {value}
    }
    fn reply(&mut self, method:&str, result:Value) -> Value {
        let request = self.next(method);
        self.adapter.core_line(&json!({"type":"response","id":request["id"],"ok":true,"result":Self::core_shape(method,result)}).to_string());
        request
    }
    fn reject(&mut self, method:&str, code:&str) {
        let request = self.next(method);
        self.adapter.core_line(&json!({"type":"response","id":request["id"],"ok":false,"error":{"code":code}}).to_string());
    }
    /// Answers every queued `settings.set`; returns the key and value each carried.
    fn settings(&mut self) -> Vec<(String, Value)> {
        let mut set = Vec::new();
        loop {
            self.collect();
            if self.requests.front().is_none_or(|r| r["method"] != "settings.set") {break;}
            let request = self.reply("settings.set",json!({}));
            set.push((request["params"]["key"].as_str().unwrap().to_owned(), request["params"]["value"].clone()));
        }
        set
    }
    fn send(&mut self, seq:u64, request:Request) {self.adapter.request(ToEngine{seq,request});self.collect();}
    fn event(&mut self, name:&str, payload:Value) {
        self.adapter.core_line(&json!({"type":"event","name":name,"payload":Self::core_shape(name,payload)}).to_string());
        self.collect();
    }
    fn target(&mut self) -> TargetId {
        self.collect();
        self.events.iter().rev().find_map(|e| match &e.event {Event::Targets{targets}=>targets.first().map(|t| t.id.clone()),_=>None}).unwrap()
    }
    fn inspect_reply(&mut self) -> Vec<(String, Value)> {
        self.reply("catalog.launch.inspect",json!({"appId":"parent","variantId":"123","decision":{"status":"ready"},"game":game(),"scope":scope(3)}));
        self.settings()
    }
    fn launch(&mut self) -> Vec<(String, Value)> {
        let target=self.target();
        self.send(20,Request::Launch{session:SessionId(9),target,game:provider()});
        self.inspect_reply()
    }
    fn session_states(&mut self) -> Vec<SessionState> {
        self.collect();
        self.events.iter().filter_map(|e| if let Event::Session{session:SessionId(9),state}=&e.event {Some(state.clone())} else {None}).collect()
    }
}

#[test]
fn device_code_links_the_complete_url_and_polls_once_per_interval() {
    let mut f=FakeCore::new(false);
    f.send(2,Request::Auth{target:None,method:AuthMethod::DeviceCode});
    f.reply("auth.device.start",json!({"attemptId":"attempt","userCode":"ABCD","verificationUri":"https://nvidia.com/activate",
        "verificationUriComplete":"https://nvidia.com/activate?code=ABCD","intervalSeconds":5,"expiresAt":u64::MAX}));
    f.collect();
    assert!(f.events.iter().any(|e| matches!(&e.event,Event::Auth{state:AuthState::DeviceCode{qr:Some(qr),code,..}} if qr.ends_with("code=ABCD") && code=="ABCD")));
    f.adapter.tick(4999);f.collect();assert!(f.requests.is_empty());
    f.adapter.tick(5000);
    let poll=f.reply("auth.device.poll",json!({"status":"pending","retryAfterMs":7000}));
    assert_eq!(poll["params"],json!({"attemptId":"attempt"}));
    f.adapter.tick(11999);f.collect();assert!(f.requests.is_empty());
    f.adapter.tick(12000);
    f.reply("auth.device.poll",json!({"status":"authorized"}));
    f.reply("auth.device.complete",envelope(2));
    f.collect();
    assert!(f.events.iter().any(|e| matches!(e.event,Event::Auth{state:AuthState::Done{..}})));
    assert_eq!(f.target().as_str(),"account:6:nvidiau");
}

#[test]
fn paged_library_preserves_variant_ids_steam_ids_and_context() {
    let mut f=FakeCore::new(true);let target=f.target();
    f.send(3,Request::Library{target:target.clone(),cursor:None});
    f.reply("catalog.library.list",json!({"games":[game()],"nextCursor":"next","hasNextPage":true,"catalogRevision":7,"catalogContext":"ctx","scope":scope(4)}));
    f.collect();
    let page=f.events.iter().find_map(|e| match &e.event{Event::Library{entries,..}=>Some(entries),_=>None}).unwrap();
    assert_eq!(page[0].id,provider());
    assert_eq!(page[0].external.iter().map(ToString::to_string).collect::<Vec<_>>(),["gfn:parent","steam:10"]);
    f.send(4,Request::Library{target,cursor:Some("next".into())});
    let request=f.reply("catalog.library.list",json!({"games":[],"hasNextPage":false,"nextCursor":"","catalogRevision":7,"catalogContext":"ctx","scope":scope(4)}));
    assert_eq!(request["params"],json!({"limit":60,"cursor":"next","catalogRevision":7,"catalogContext":"ctx"}));
}

#[test]
fn launch_ack_queue_start_running_stats_and_remote_end() {
    let mut f=FakeCore::new(true);f.launch();
    let create=f.reply("session.create",json!({"sessionId":"seat","status":1,"phase":"queued","queuePosition":8}));
    assert_eq!(create["params"]["scope"],scope(3));
    assert_eq!(create["params"]["runtimeCapabilities"]["nativeHdrSupported"],true);
    assert_eq!(create["params"]["appLaunchMode"],"gamepadFriendly","what NVIDIA's own client asks for on a TV");
    assert!(create["params"].get("accountLinked").is_none(),"the core decides it from the store's sign-on link");
    let display=&create["params"]["runtimeCapabilities"]["nativeHdrDisplay"];
    for key in ["minimumNits","maximumNits","maximumFullFrameNits","redX","redY","greenX","greenY","blueX","blueY","whiteX","whiteY"] {
        assert!(display[key].as_f64().is_some(),"{key}: the core drops the whole group, and with it the frame average, unless every field is present");
    }
    f.collect();assert_eq!(f.receipts.len(),1);
    f.event("session.changed",json!({"sessionId":"seat","status":1,"phase":"queued","queuePosition":3}));
    f.adapter.tick(1000);
    f.reply("session.poll",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"],
        "hdrDowngrade":"game-unsupported","negotiatedStreamProfile":{"codec":"h265","enableHdr":false,"resolution":"3840x2160","fps":120}}));
    f.reply("streamer.start",json!({"status":"running"}));
    f.send(21,Request::Stats{session:SessionId(9),on:true,pad_timing:false});
    f.adapter.pad_line(&json!({"stream":"telemetry","fps":119.8,"bitrate_mbps":78.5,"ping_ms":21.5,"jitter_ms":0.2,"loss_pct":0.1}).to_string());
    f.collect();
    assert!(f.events.iter().any(|e| matches!(&e.event,Event::Stats{stats,..} if stats.bitrate_kbps==Some(78500) && stats.fps==Some(119.8)
        && stats.rtt_ms==Some(21.5) && stats.jitter_ms==Some(0.2) && stats.loss_pct==Some(0.1))));
    f.event("session.changed",json!({"sessionId":"seat","status":7,"phase":"finished"}));
    let states=f.session_states();
    assert!(matches!(states[0],SessionState::Checking));
    assert!(states.contains(&SessionState::Queue{position:Some(8),eta_s:None}));
    assert!(states.contains(&SessionState::Queue{position:Some(3),eta_s:None}));
    assert!(states.contains(&SessionState::Starting));
    assert!(states.iter().any(|s| matches!(s,SessionState::Running{stream:Some(p)} if !p.hdr && p.sdr_reason.as_deref()==Some("SDR: no HDR for this game"))));
    assert_eq!(states.last(),Some(&SessionState::Ended{reason:EndReason::Remote}));
    f.reply("streamer.stop",json!({}));
}

#[test]
fn the_hdr_setting_goes_to_the_core_before_the_session_is_created() {
    for hdr in [true,false] {
        let mut f=FakeCore::new(true);
        f.send(4,Request::Settings{values:Values::from([("hdr".into(),lumagame_core::settings::Value::Bool(hdr))])});
        let set=f.launch();
        assert!(set.contains(&("enableHdr".into(),json!(hdr))),"{set:?}");
        assert!(set.contains(&("colorQuality".into(),json!("10bit_420"))),"HEVC streams 10-bit, HDR or not: {set:?}");
        assert!(set.contains(&("codec".into(),json!("h265"))),"HDR needs a codec that carries it: {set:?}");
        f.reply("session.create",json!({"sessionId":"seat","status":1,"phase":"queued"}));
    }
}

#[test]
fn a_stream_the_server_ended_ends_the_session_and_a_lost_connection_fails_it() {
    let running=|| {
        let mut f=FakeCore::new(true);f.launch();
        f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
        f.reply("streamer.start",json!({"status":"running"}));
        f
    };
    let mut f=running();
    f.adapter.pad_line(&json!({"stream":"ended","source":"nvst-server","resumable":false}).to_string());
    assert_eq!(f.session_states().last(),Some(&SessionState::Ended{reason:EndReason::Remote}));
    assert_eq!(f.reply("streamer.stop",json!({}))["params"]["reason"],"the stream ended","the finished worker is reaped for the next launch");
    let mut f=running();
    f.adapter.pad_line(&json!({"stream":"ended","source":"nvst-transport","resumable":null}).to_string());
    assert!(matches!(f.session_states().last(),Some(SessionState::Failed{reason:FailReason::Network,..})));
}

/// The core refuses `session.create` while it holds an active seat (`session_update_busy`), and only `session.ended` or `session.stop`
/// releases one. 2026-11-22: after Doom was quit from inside the game every relaunch failed until the app was restarted.
#[test]
fn a_game_the_host_ended_releases_its_seat_before_the_next_launch_is_checked() {
    let mut f=FakeCore::new(true);f.launch();
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reply("streamer.start",json!({"status":"running"}));
    f.adapter.pad_line(&json!({"stream":"ended","source":"nvst-server","resumable":false}).to_string());
    assert_eq!(f.session_states().last(),Some(&SessionState::Ended{reason:EndReason::Remote}));
    // The next launch arrives before the core has answered anything.
    let target=f.target();
    f.send(21,Request::Launch{session:SessionId(10),target,game:provider()});
    f.reply("streamer.stop",json!({}));
    let ended=f.reply("session.ended",json!({"sessionId":"seat","cleared":true}));
    assert_eq!(ended["params"],json!({"sessionId":"seat","ownerScope":{"userId":"u","providerIdpId":"nvidia"}}));
    f.inspect_reply();
    f.reply("session.create",json!({"sessionId":"seat2","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reply("streamer.start",json!({"status":"running"}));
    f.collect();
    let second:Vec<_>=f.events.iter().filter_map(|e| match &e.event {Event::Session{session:SessionId(10),state}=>Some(state.clone()),_=>None}).collect();
    assert!(second.iter().any(|s| matches!(s,SessionState::Running{..})),"{second:?}");
    assert!(!second.iter().any(|s| matches!(s,SessionState::Failed{..})),"{second:?}");
}

#[test]
fn the_seat_release_is_retried_while_the_handoff_completes_and_sent_only_for_an_end_the_far_side_made() {
    let running=|| {
        let mut f=FakeCore::new(true);f.launch();
        f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
        f.reply("streamer.start",json!({"status":"running"}));
        f
    };
    let mut f=running();
    f.event("session.changed",json!({"sessionId":"seat","status":7,"phase":"finished"}));
    f.reply("streamer.stop",json!({}));
    f.reject("session.ended","session_update_busy");
    f.reply("session.ended",json!({"sessionId":"seat","cleared":true}));
    f.collect();assert!(f.requests.is_empty(),"released once it was cleared");

    let mut f=running();
    f.adapter.pad_line(&json!({"stream":"ended","source":"nvst-transport","resumable":null}).to_string());
    f.reply("streamer.stop",json!({}));
    f.collect();assert!(f.requests.is_empty(),"a lost connection leaves the seat to be resumed");
}

fn running() -> FakeCore {
    let mut f=FakeCore::new(true);f.launch();
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reply("streamer.start",json!({"status":"running"}));
    f
}

/// The termination reason the official client names `NVB_SN_EXITED_DUE_TO_USER_IDLE_TIMEOUT` is an idle end, not the game quitting. It
/// frees the seat the same way, so the next launch is not refused with `session_update_busy`.
#[test]
fn an_idle_end_is_named_idle_and_releases_the_seat_for_the_next_launch() {
    let mut f=running();
    f.adapter.pad_line(&json!({"stream":"ended","source":"nvst-server","reason":56,"resumable":false}).to_string());
    assert_eq!(f.session_states().last(),Some(&SessionState::Ended{reason:EndReason::Idle}));
    let target=f.target();
    f.send(21,Request::Launch{session:SessionId(10),target,game:provider()});
    f.reply("streamer.stop",json!({}));
    assert_eq!(f.reply("session.ended",json!({"sessionId":"seat","cleared":true}))["params"]["sessionId"],"seat");
    f.inspect_reply();
    f.reply("session.create",json!({"sessionId":"seat2","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reply("streamer.start",json!({"status":"running"}));
    f.collect();
    assert!(f.events.iter().any(|e| matches!(&e.event,Event::Session{session:SessionId(10),state:SessionState::Running{..}})));
}

/// A far side that closes the connection when the countdown runs out may name no reason; the countdown the engine saw is the proof.
#[test]
fn a_stream_that_ends_after_its_countdown_ran_out_is_an_idle_end_and_one_that_ends_early_is_not() {
    let lost=json!({"stream":"ended","source":"nvst-transport","resumable":null}).to_string();
    let warn=|f:&mut FakeCore,seconds:Value| f.adapter.pad_line(&json!({"stream":"idle","seconds":seconds}).to_string());
    let mut f=running();
    f.adapter.tick(1000);warn(&mut f,json!(10));
    f.adapter.tick(11_000);f.adapter.pad_line(&lost);
    assert_eq!(f.session_states().last(),Some(&SessionState::Ended{reason:EndReason::Idle}));
    f.reply("streamer.stop",json!({}));
    assert_eq!(f.reply("session.ended",json!({"sessionId":"seat","cleared":true}))["params"]["sessionId"],"seat");
    let mut f=running();
    f.adapter.tick(1000);warn(&mut f,json!(10));
    f.adapter.tick(7000);f.adapter.pad_line(&lost);
    assert!(matches!(f.session_states().last(),Some(SessionState::Failed{reason:FailReason::Network,..})),"seven seconds early is a lost connection");
    let mut f=running();
    f.adapter.tick(1000);warn(&mut f,json!(10));warn(&mut f,Value::Null);
    f.adapter.tick(20_000);f.adapter.pad_line(&lost);
    assert!(matches!(f.session_states().last(),Some(SessionState::Failed{reason:FailReason::Network,..})),"a cleared warning is no countdown");
}

#[test]
fn the_idle_warning_is_forwarded_and_clears_and_a_press_sends_one_input_only_while_it_is_up() {
    let mut f=running();
    f.send(30,Request::Activity{session:SessionId(9)});
    f.collect();assert!(f.requests.is_empty(),"no warning, nothing to answer");
    f.adapter.pad_line(&json!({"stream":"idle","seconds":59}).to_string());
    f.send(31,Request::Activity{session:SessionId(9)});
    assert_eq!(f.next("streamer.control")["params"],json!({"action":"anti-afk-pulse"}));
    f.adapter.pad_line(&json!({"stream":"idle","seconds":null}).to_string());
    f.send(32,Request::Activity{session:SessionId(9)});
    f.collect();assert!(f.requests.is_empty(),"the warning cleared");
    let seen:Vec<_>=f.events.iter().filter_map(|e| match &e.event {Event::IdleWarning{session:SessionId(9),seconds}=>Some(*seconds),_=>None}).collect();
    assert_eq!(seen,[Some(59),None]);
}

#[test]
fn a_refused_launch_carries_the_cores_error_code_in_its_message() {
    let mut f=FakeCore::new(true);f.launch();
    f.reject("session.create","session_update_busy");
    let Some(SessionState::Failed{reason:FailReason::Engine,message})=f.session_states().last().cloned() else { panic!("{:?}",f.session_states()) };
    assert_eq!(failure_code(&message),Some("session_update_busy"),"{message}");
    f.reply("streamer.stop",json!({}));
}

#[test]
fn guide_opens_the_overlay_and_the_overlay_reads_the_pads_as_menu_keys() {
    use lumatv_gfn_pad::button::{A,DPAD_DOWN,Y};
    let mut f=FakeCore::new(true);f.launch();
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reply("streamer.start",json!({"status":"running"}));
    let mut pad=|line:Value| {f.collect();f.events.clear();f.adapter.pad_line(&line.to_string());f.collect();f.events.iter().map(|e| e.event.clone()).collect::<Vec<_>>()};
    let report=|buttons:u16| json!({"pad":"report","slot":0,"buttons":buttons,"lt":0,"rt":0,"lx":0,"ly":0,"rx":0,"ry":0});
    assert!(pad(report(A)).is_empty(),"the game has the pads");
    assert_eq!(pad(json!({"pad":"guide","slot":0,"pressed":true})),[Event::Overlay{session:SessionId(9)}]);
    assert!(pad(json!({"pad":"guide","slot":0,"pressed":false})).is_empty());
    f.send(22,Request::Route{session:SessionId(9),input:InputRoute::App});
    assert_eq!(f.reply("streamer.input.pause",json!({}))["params"],json!({"paused":true}));
    let mut pad=|line:Value| {f.collect();f.events.clear();f.adapter.pad_line(&line.to_string());f.collect();f.events.iter().map(|e| e.event.clone()).collect::<Vec<_>>()};
    let key=|key| [Event::Key{key,down:true},Event::Key{key,down:false}];
    assert_eq!(pad(report(A)),key(PadKey::Ok),"a button held into the overlay is pressed afresh");
    assert!(pad(report(A)).is_empty());
    assert_eq!(pad(report(DPAD_DOWN)),key(PadKey::Down));
    assert_eq!(pad(report(Y)),key(PadKey::Menu));
}

#[test]
fn pad_timing_is_opt_in_and_ages_with_the_kernel_report() {
    let mut f=FakeCore::new(true);f.launch();
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reply("streamer.start",json!({"status":"running"}));
    assert!(!f.adapter.pad_stats_on());
    f.send(21,Request::Stats{session:SessionId(9),on:true,pad_timing:false});
    assert!(!f.adapter.pad_stats_on(),"ordinary menu statistics must not start controller timing");
    f.send(22,Request::Stats{session:SessionId(9),on:true,pad_timing:true});
    assert!(f.adapter.pad_stats_on());
    f.adapter.pad_line(&json!({"pad":"timing","name":"DualShock 4","reports":125,"rate_hz":250.0,"p99_us":4000,"bursts":0,"burst_gap_us":0,"age_ms":4}).to_string());
    f.adapter.tick(1000);
    f.adapter.pad_line(&json!({"stream":"telemetry","fps":120}).to_string());f.collect();
    let pad=f.events.iter().rev().find_map(|e| match &e.event {Event::Stats{stats,..}=>stats.pad.as_ref(),_=>None}).unwrap();
    assert_eq!((pad.name.as_str(),pad.p99_us,pad.age_ms),("DualShock 4",4000,1004));
    f.send(23,Request::Stats{session:SessionId(9),on:true,pad_timing:false});
    assert!(!f.adapter.pad_stats_on());
    f.adapter.pad_line(&json!({"pad":"timing","name":"late report","reports":1,"rate_hz":250.0,"p99_us":4000,"bursts":0,"burst_gap_us":0,"age_ms":0}).to_string());
    f.adapter.pad_line(&json!({"stream":"telemetry","fps":120}).to_string());f.collect();
    assert!(f.events.iter().rev().find_map(|e| match &e.event {Event::Stats{stats,..}=>Some(stats.pad.is_none()),_=>None}).unwrap());
    f.send(24,Request::Stats{session:SessionId(9),on:false,pad_timing:true});
    assert!(!f.adapter.pad_stats_on());
    f.send(25,Request::Stats{session:SessionId(9),on:true,pad_timing:true});
    assert!(f.adapter.pad_stats_on());
}

#[test]
fn round_trip_answers_with_the_latest_stream_and_press_legs_once_running() {
    let mut f=FakeCore::new(true);f.launch();
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.send(23,Request::RoundTrip{session:SessionId(9)});
    assert!(f.events.iter().any(|e| e.re==Some(23) && matches!(e.event,Event::Error{code:ErrorCode::BadRequest,..})),"not running yet");
    f.reply("streamer.start",json!({"status":"running"}));
    f.adapter.pad_line(&json!({"stream":"telemetry","fps":120,"ping_ms":21.5}).to_string());
    f.adapter.pad_line(&json!({"stream":"telemetry","fps":120}).to_string());
    f.adapter.pad_line(&json!({"pad":"latency","stage":"send","n":14,"p50":420,"p90":800,"p99":1900,"max":2500}).to_string());
    f.send(24,Request::RoundTrip{session:SessionId(9)});
    let trip=f.events.iter().find_map(|e| match &e.event {Event::RoundTrip{trip,..} if e.re==Some(24)=>Some(trip.clone()),_=>None}).unwrap();
    assert_eq!(trip,RoundTrip{presses:Some(14),press_p50_ms:Some(0.42),press_p99_ms:Some(1.9),network_ms:Some(21.5),..RoundTrip::default()});
}

#[test]
fn steam_overlay_asks_the_running_streamer_for_the_shortcut() {
    let mut f=FakeCore::new(true);f.launch();
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.send(23,Request::SteamOverlay{session:SessionId(9)});
    assert!(f.events.iter().any(|e| e.re==Some(23) && matches!(e.event,Event::Error{code:ErrorCode::BadRequest,..})),"not running yet");
    f.reply("streamer.start",json!({"status":"running"}));
    f.send(24,Request::SteamOverlay{session:SessionId(9)});
    assert_eq!(f.reply("streamer.control",json!({"action":"steam-overlay","streamerRunning":true}))["params"],json!({"action":"steam-overlay"}));
}

#[test]
fn busy_streamer_is_stopped_before_exactly_one_retry() {
    let mut f=FakeCore::new(true);f.launch();
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reject("streamer.start","streamer_busy");
    f.event("streamer.changed",json!({"status":"stopped"}));
    f.reply("streamer.stop",json!({}));
    f.reply("streamer.start",json!({"status":"running"}));
    assert!(f.session_states().iter().any(|s| matches!(s,SessionState::Running{..})));
    let target=f.target();
    f.send(22,Request::Launch{session:SessionId(10),target,game:provider()});
    assert!(f.events.iter().any(|e| matches!(&e.event,Event::Session{session:SessionId(10),state:SessionState::Failed{reason:FailReason::Busy,..}})));
    f.send(23,Request::Stop{session:SessionId(9),end_game:false});
    f.event("streamer.changed",json!({"status":"stopped"}));
    f.reply("streamer.stop",json!({}));
    assert_eq!(f.session_states().last(),Some(&SessionState::Ended{reason:EndReason::Suspended}));
}

#[test]
fn stale_scope_refreshes_account_and_reinspects_before_retry() {
    let mut f=FakeCore::new(true);f.launch();
    f.reject("session.create","stale_account");
    f.reply("auth.session.get",envelope(5));
    f.inspect_reply();
    let create=f.next("session.create");
    assert_eq!(create["params"]["scope"],scope(5),"older inspect scope cannot move generation backwards");
    f.adapter.core_line(&json!({"type":"response","id":create["id"],"ok":false,"error":{"code":"stale_account"}}).to_string());
    assert!(matches!(f.session_states().last(),Some(SessionState::Failed{reason:FailReason::Auth,..})));
    f.reply("streamer.stop",json!({}));
}

#[test]
fn a_seat_held_by_the_same_game_is_claimed_and_another_game_is_reported_busy() {
    let mut f=FakeCore::new(true);f.launch();
    f.reject("session.create","session_conflict");
    f.reply("session.remote.list",json!({"sessions":[{"sessionId":"other","appId":999,"streamingBaseUrl":"https://a"},{"sessionId":"left","appId":123,"streamingBaseUrl":"https://b"}]}));
    let claim=f.reply("session.claim",json!({"sessionId":"left","status":2,"rtspsEndpoints":["rtsp://server"]}));
    assert_eq!(claim["params"]["sessionId"],"left");assert_eq!(claim["params"]["streamingBaseUrl"],"https://b");
    f.reply("streamer.start",json!({"status":"running"}));
    assert!(f.session_states().iter().any(|s| matches!(s,SessionState::Running{..})));

    let mut f=FakeCore::new(true);f.launch();
    f.reject("session.create","session_conflict");
    f.reply("session.remote.list",json!({"sessions":[{"sessionId":"other","appId":999,"streamingBaseUrl":"https://a"}]}));
    assert!(matches!(f.session_states().last(),Some(SessionState::Failed{reason:FailReason::Busy,..})));
    f.reply("streamer.stop",json!({}));
}

#[test]
fn cancelling_an_allocating_session_never_starts_its_late_stream() {
    let mut f=FakeCore::new(true);f.launch();
    f.send(30,Request::Stop{session:SessionId(9),end_game:true});
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reply("streamer.stop",json!({}));
    let stop=f.reply("session.stop",json!({}));assert_eq!(stop["params"]["sessionId"],"seat");
    assert_eq!(f.session_states().last(),Some(&SessionState::Ended{reason:EndReason::Stopped}));
    assert!(!f.session_states().iter().any(|s| matches!(s,SessionState::Running{..})));
}

#[test]
fn repeated_busy_is_reported_not_retried_forever() {
    let mut f=FakeCore::new(true);f.launch();
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reject("streamer.start","streamer_busy");f.reply("streamer.stop",json!({}));f.reject("streamer.start","streamer_busy");
    assert!(matches!(f.session_states().last(),Some(SessionState::Failed{reason:FailReason::Busy,..})));
    f.reply("streamer.stop",json!({}));
}

#[test]
fn protocol_deadline_and_foreign_library_are_not_silent_success() {
    let mut f=FakeCore::new(true);let target=f.target();
    f.send(4,Request::Library{target,cursor:None});
    let mut foreign=scope(4);foreign["userId"]=json!("other");
    f.reply("catalog.library.list",json!({"games":[game()],"scope":foreign}));f.collect();
    assert!(f.events.iter().any(|e| matches!(e.event,Event::Error{..})));
    assert!(!f.events.iter().any(|e| matches!(e.event,Event::Library{..})));
    f.send(5,Request::Targets);f.adapter.tick(120000);f.collect();assert!(f.adapter.failed());
}

#[test]
fn a_stop_error_still_allows_the_single_busy_retry() {
    let mut f=FakeCore::new(true);f.launch();
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reject("streamer.start","streamer_busy");f.reject("streamer.stop","streamer_failed");
    f.reply("streamer.start",json!({"status":"running"}));
    assert!(f.session_states().iter().any(|s| matches!(s,SessionState::Running{..})));
}

#[test]
fn settings_changed_mid_launch_apply_only_to_the_next_launch() {
    let mut f=FakeCore::new(true);f.launch();
    let values=Values::from([("hdr".into(),lumagame_core::settings::Value::Bool(false))]);
    f.send(25,Request::Settings{values});
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reply("streamer.start",json!({"status":"running"}));
    assert!(f.session_states().iter().any(|s| matches!(s,SessionState::Running{stream:Some(p)} if p.hdr)));
}

#[test]
fn signout_discards_an_inflight_library_and_old_session_events() {
    let mut f=FakeCore::new(true);let target=f.target();
    f.send(3,Request::Library{target,cursor:None});
    f.event("auth.session.changed",json!({"generation":3,"session":null}));
    f.reply("catalog.library.list",json!({"games":[game()],"scope":scope(2)}));
    f.collect();
    assert!(!f.events.iter().any(|e| matches!(e.event,Event::Library{..})));
    f.event("auth.session.changed",envelope(4));
    f.launch();
    f.event("session.changed",json!({"sessionId":"previous-seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reply("session.create",json!({"sessionId":"new-seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    let start=f.reply("streamer.start",json!({"status":"running"}));
    assert_eq!(start["params"]["session"]["sessionId"],"new-seat");
}

#[test]
fn stream_numbers_follow_the_stats_request_and_the_core_snapshot_ends_a_dead_stream() {
    let mut f=FakeCore::new(true);f.launch();
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.reply("streamer.start",json!({"status":"running"}));
    let line=json!({"stream":"telemetry","fps":59.9,"bitrate_mbps":40.0,"ping_ms":9.5}).to_string();
    f.adapter.pad_line(&line);f.collect();
    assert!(!f.events.iter().any(|e| matches!(e.event,Event::Stats{..})),"nothing is reported while statistics are off");
    f.send(21,Request::Stats{session:SessionId(9),on:true,pad_timing:false});
    f.adapter.pad_line(&line);f.collect();
    assert!(f.events.iter().any(|e| matches!(&e.event,Event::Stats{stats,..} if stats.fps==Some(59.9) && stats.rtt_ms==Some(9.5) && stats.jitter_ms.is_none())));
    f.event("streamer.changed",json!({"status":"error"}));
    assert!(matches!(f.session_states().last(),Some(SessionState::Failed{reason:FailReason::Video,..})),"the core's wrapped snapshot names a dead streamer");
}

fn colour(transfer:u8) -> String {
    json!({"stream":"colour","codec":"h265","primaries":9,"transfer":transfer,"matrix":9,"full_range":false}).to_string()
}
fn running_streams(f:&mut FakeCore) -> Vec<StreamInfo> {
    f.session_states().into_iter().filter_map(|s| if let SessionState::Running{stream:Some(p)}=s {Some(p)} else {None}).collect()
}

#[test]
fn the_streams_colour_repeats_running_once_per_change_and_nothing_else_follows_it() {
    let mut f=running();
    let first=running_streams(&mut f).remove(0);
    assert_eq!(first.vui,None);
    f.adapter.pad_line(&colour(16));
    let streams=running_streams(&mut f);
    assert_eq!(streams.len(),2,"one more Running, carrying the colour");
    assert_eq!(streams[1],StreamInfo{vui:Some(Vui{primaries:9,transfer:16,matrix:9,full_range:false}),..first.clone()},"hdr and sdr_reason are the engine's own");
    f.adapter.pad_line(&colour(16));
    assert_eq!(running_streams(&mut f).len(),2,"the same colour again says nothing new");
    f.adapter.pad_line(&colour(18));
    let streams=running_streams(&mut f);
    assert_eq!(streams.len(),3);
    assert_eq!(streams[2].vui.map(|v| v.transfer),Some(18));
    f.adapter.pad_line(&json!({"stream":"colour","primaries":9,"transfer":999,"matrix":9,"full_range":false}).to_string());
    f.adapter.pad_line(&json!({"stream":"colour","primaries":9,"transfer":16,"matrix":9,"full_range":"no"}).to_string());
    f.adapter.pad_line(&json!({"stream":"colour","primaries":9,"transfer":16}).to_string());
    assert_eq!(running_streams(&mut f).len(),3,"a malformed line changes nothing");
}

#[test]
fn a_colour_that_arrives_before_running_is_attached_to_it_and_an_ended_session_ignores_one() {
    let mut f=FakeCore::new(true);f.launch();
    f.adapter.pad_line(&colour(18));
    f.reply("session.create",json!({"sessionId":"seat","status":2,"rtspsEndpoints":["rtsp://server"]}));
    f.adapter.pad_line(&colour(16));
    f.reply("streamer.start",json!({"status":"running"}));
    let streams=running_streams(&mut f);
    assert_eq!(streams.len(),1,"the colour adds no event of its own before Running");
    assert_eq!(streams[0].vui.map(|v| v.transfer),Some(16),"a line from before the streamer was asked to start is not counted");
    f.event("session.changed",json!({"sessionId":"seat","status":7,"phase":"finished"}));
    f.reply("streamer.stop",json!({}));
    f.adapter.pad_line(&colour(18));
    assert_eq!(running_streams(&mut f).len(),1);
}

fn refreshes(f:&FakeCore) -> Vec<(Option<String>,Option<String>)> {
    f.events.iter().filter_map(|e| match &e.event {Event::Refresh{step,note}=>Some((step.clone(),note.clone())),_=>None}).collect()
}
fn stores() -> Value {
    json!({"accounts":[
        {"provider":"STEAM","label":"Steam","isConnected":true,"status":"connected","supportsSync":true},
        {"provider":"EPIC","label":"Epic","isConnected":true,"status":"connected","supportsSync":false},
        {"provider":"GOG","label":"GOG","isConnected":false,"status":"not_connected","supportsSync":true}]})
}

#[test]
fn a_refresh_syncs_the_linked_store_waits_for_the_core_and_closes_the_sync_after_the_new_library() {
    let mut f=FakeCore::new(true);let target=f.target();
    f.send(30,Request::Action{key:"refresh".into()});
    f.reply("account.connections.list",stores());
    let sync=f.reply("account.connections.sync",json!({"operationId":"op1","provider":"STEAM","phase":"waiting_remote","retryAfterMs":2000}));
    assert_eq!(sync["params"],json!({"provider":"STEAM"}), "only the connected store that can sync");
    f.adapter.tick(1999);f.collect();assert!(f.requests.is_empty(),"the core's pause is kept");
    f.adapter.tick(2000);
    let poll=f.reply("account.connections.sync.status",json!({"operationId":"op1","phase":"refreshing_library","catalogRevision":8}));
    assert_eq!(poll["params"],json!({"operationId":"op1"}));
    f.collect();
    let steps=refreshes(&f);
    assert_eq!(steps.first(),Some(&(Some("Checking your linked stores".into()),None)));
    assert_eq!(steps.last(),Some(&(None,None)), "the engine's part is over and nothing went wrong");
    f.send(31,Request::Library{target,cursor:None});
    f.reply("catalog.library.list",json!({"games":[game()],"nextCursor":"","hasNextPage":false,"catalogRevision":8,"catalogContext":"ctx","scope":scope(4)}));
    let ack=f.next("account.connections.sync.status");
    assert_eq!(ack["params"],json!({"operationId":"op1","libraryRefreshed":true}));
}

#[test]
fn a_store_that_fails_or_never_confirms_is_reported_and_no_linked_store_goes_straight_to_the_library() {
    let mut f=FakeCore::new(true);
    f.send(30,Request::Action{key:"refresh".into()});
    f.reply("account.connections.list",json!({"accounts":[]}));
    f.collect();
    assert_eq!(refreshes(&f).last(),Some(&(None,None)));

    let mut f=FakeCore::new(true);
    f.send(30,Request::Action{key:"refresh".into()});
    f.reply("account.connections.list",stores());
    f.reply("account.connections.sync",json!({"operationId":"op1","provider":"STEAM","phase":"waiting_remote","retryAfterMs":1000}));
    f.adapter.tick(1000);
    f.reply("account.connections.sync.status",json!({"operationId":"op1","phase":"failed","message":"x"}));
    f.collect();
    assert_eq!(refreshes(&f).last(),Some(&(None,Some("GeForce NOW could not refresh Steam.".into()))));

    let mut f=FakeCore::new(true);
    f.send(30,Request::Action{key:"refresh".into()});
    f.reply("account.connections.list",stores());
    f.reply("account.connections.sync",json!({"operationId":"op1","provider":"STEAM","phase":"waiting_remote"}));
    f.adapter.tick(135_000);f.collect();
    assert_eq!(refreshes(&f).last(),Some(&(None,Some("GeForce NOW was still checking Steam when the wait ended.".into()))));
}

#[test]
fn a_refresh_that_cannot_reach_or_sign_in_to_geforce_now_is_a_named_error_and_a_signed_out_engine_refuses_it() {
    for (code,expected) in [("network_error",ErrorCode::Network),("http_unauthorized",ErrorCode::SignIn)] {
        let mut f=FakeCore::new(true);
        f.send(30,Request::Action{key:"refresh".into()});
        f.reject("account.connections.list",code);
        f.collect();
        assert!(f.events.iter().any(|e| matches!(&e.event,Event::Error{code,..} if *code==expected)),"{code}");
        assert!(refreshes(&f).iter().all(|(step,_)| step.is_some()),"no end is announced: the app ends the run on the error");
    }
    let mut f=FakeCore::new(false);
    f.send(30,Request::Action{key:"refresh".into()});
    assert!(f.events.iter().any(|e| matches!(&e.event,Event::Error{code:ErrorCode::SignIn,..})));
}
