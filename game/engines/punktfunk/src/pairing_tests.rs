use punktfunk_core::{PunktfunkError,client::NativeClient,quic::{endpoint,pake,PairRequest,PairChallenge,PairProof,PairResult,v2::{io,registry,msg::decode}}};
use std::time::Duration;

async fn ceremony(pin:&str,host_pin:&str)->Result<[u8;32],PunktfunkError> {
    let host=endpoint::generate_identity().unwrap();let client=endpoint::generate_identity().unwrap();
    let host_fp=endpoint::fingerprint_of_pem(&host.0).unwrap();
    let (server,_socket)=endpoint::server_shared("127.0.0.1:0".parse().unwrap(),&host.0,&host.1,Duration::from_secs(1),&[registry::ALPN]).unwrap();
    let port=server.local_addr().unwrap().port();let pin=pin.to_owned();let host_pin=host_pin.to_owned();
    let connecting=tokio::task::spawn_blocking(move ||NativeClient::pair("127.0.0.1",port,(&client.0,&client.1),&pin,"LumaGame protocol test",Duration::from_secs(1)));
    let conn=server.accept().await.unwrap().await.unwrap();
    let client_fp=endpoint::peer_fingerprint(&conn).unwrap();
    let (mut send,mut recv)=conn.accept_bi().await.unwrap();
    assert_eq!(io::read_stream_type(&mut recv).await.unwrap(),registry::STREAM_CONTROL);
    let mut recv=io::FrameReader::new(recv);
    let (ty,bytes)=recv.read_frame().await.unwrap();let req=decode::<PairRequest>(ty,&bytes).unwrap();
    assert!(req.device_key.is_empty());
    let (pake,spake_b)=pake::start(false,&host_pin,&client_fp,&host_fp);let confirm=pake.finish(&req.spake_a).unwrap();
    io::send(&mut send,&PairChallenge {spake_b,confirm:confirm.host}).await.unwrap();
    if let Ok((ty,bytes))=recv.read_frame().await {
        let proof=decode::<PairProof>(ty,&bytes).unwrap();
        let ok=pake::verify(&confirm.client,&proof.confirm);
        io::send(&mut send,&PairResult {ok}).await.unwrap();
    }
    conn.closed().await;
    let result=connecting.await.unwrap();
    if let Ok(pin)=result {assert_eq!(pin,host_fp);}
    result
}
#[tokio::test]
async fn native_pairing_proves_both_identities_without_starting_a_session() {
    let verified=ceremony("4321","4321").await.unwrap();
    let dir=tempfile::tempdir().unwrap();let mut store=crate::store::Store::open(dir.path()).unwrap();
    let id=store.manual("127.0.0.1").unwrap();store.paired(&id,verified).unwrap();
    assert_eq!(crate::store::Store::open(dir.path()).unwrap().host(&id).unwrap().pin().unwrap(),verified);
}
#[tokio::test]
async fn wrong_pin_never_returns_a_trusted_identity() {assert!(matches!(ceremony("1234","4321").await,Err(PunktfunkError::Crypto)));}
