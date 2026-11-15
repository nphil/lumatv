use super::*;
use std::{io::{Read,Write},os::unix::net::{UnixListener,UnixStream},sync::mpsc};
fn send(s:&mut UnixStream,kind:u8,data:&[u8]) {
    s.write_all(&[kind,0,0,0]).unwrap();s.write_all(&(data.len() as u32).to_le_bytes()).unwrap();s.write_all(data).unwrap();
}
fn recv(s:&mut UnixStream)->(u8,Vec<u8>) {
    let mut h=[0;8];s.read_exact(&mut h).unwrap();let n=u32::from_le_bytes(h[4..].try_into().unwrap()) as usize;assert!(n<1024);
    let mut body=vec![0;n];s.read_exact(&mut body).unwrap();(h[0],body)
}
#[test]
fn picture_ingest_precedes_load_completion_and_audio_then_flows_without_flush() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("feeder.sock");let listener=UnixListener::bind(&path).unwrap();
    let host=thread::spawn(move || {
        let (mut s,_)=listener.accept().unwrap();s.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let mut greeting=[0;112];greeting[0..4].copy_from_slice(&1u32.to_le_bytes());greeting[4..8].copy_from_slice(&1u32.to_le_bytes());send(&mut s,msg::HELLO,&greeting);
        let (kind,body)=recv(&mut s);assert_eq!((kind,body.len()),(msg::LOAD,100));assert_eq!(u32::from_le_bytes(body[16..20].try_into().unwrap()),audio::OPUS);assert_eq!(i32::from_le_bytes(body[96..100].try_into().unwrap()),pts_unit::MS);
        send(&mut s,msg::LOADED,&1i32.to_le_bytes());
        let (kind,body)=recv(&mut s);assert_eq!(kind,msg::VIDEO);assert_eq!(body[8],1);assert_eq!(&body[16..],&[0,0,0,1,0x65,0x80]);
        let stamp=i64::from_le_bytes(body[..8].try_into().unwrap());assert!(stamp>0 && stamp%1_000_000==0);
        let mut event=[0;16];event[..4].copy_from_slice(&22i32.to_le_bytes());send(&mut s,msg::EVENT,&event);
        let mut accept=[0;16];accept[..8].copy_from_slice(&stamp.to_le_bytes());send(&mut s,msg::ACCEPT,&accept);
        let (kind,body)=recv(&mut s);assert_eq!(kind,msg::AUDIO);assert_eq!(&body[16..],&[0xf8,0xff,0xfe]);
        send(&mut s,msg::KEYFRAME,&[]);
        assert_eq!(recv(&mut s).0,msg::UNLOAD);
    });
    let (mut client,_)=FeederClient::connect(&path,Duration::from_millis(500)).unwrap();
    let player=Arc::new(Player::default());let p=Arc::clone(&player);let (tx,rx)=mpsc::channel();
    client.spawn_reader(move |event| {p.event(event);let _=tx.send(());}).unwrap();
    let stream=StreamInfo {codec:Codec::H264,width:1920,height:1080,fps:60,hdr:false,audio_channels:2,sdr_reason:None};client.send_load(&load(&stream,None)).unwrap();
    rx.recv_timeout(Duration::from_millis(500)).unwrap();assert!(!player.loaded.load(Ordering::Acquire));
    let mut pacing=stage::Pacing::new(60,false,false);let stamp=pacing.map(1,monotonic_ns());
    client.send_media(msg::VIDEO,stamp as i64,true,&[0,0,0,1,0x65,0x80]).unwrap();
    rx.recv_timeout(Duration::from_millis(500)).unwrap();rx.recv_timeout(Duration::from_millis(500)).unwrap();
    assert!(player.loaded.load(Ordering::Acquire));assert_eq!(player.accepted.load(Ordering::Relaxed),1);
    client.send_media(msg::AUDIO,monotonic_ns() as i64,false,&[0xf8,0xff,0xfe]).unwrap();
    rx.recv_timeout(Duration::from_millis(500)).unwrap();assert!(player.keyframe.load(Ordering::Acquire));
    client.send_unload().unwrap();drop(client);host.join().unwrap();
}
