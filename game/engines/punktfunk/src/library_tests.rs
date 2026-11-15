use crate::{library::{Library,DESKTOP},store::Store};
use lumagame_core::{ids::ProviderGameId,ipc::Readiness};
use punktfunk_core::quic::endpoint;
use rustls::{RootCertStore,ServerConfig,ServerConnection,StreamOwned,server::WebPkiClientVerifier,pki_types::{CertificateDer,PrivateKeyDer,pem::PemObject}};
use std::{io::{Read,Write},net::TcpListener,sync::Arc,time::Duration};

#[test]
fn library_uses_pinned_mutual_tls_pages_and_the_original_launch_id() {
    punktfunk_core::tls::install_default_provider();
    let dir=tempfile::tempdir().unwrap();let mut store=Store::open(dir.path()).unwrap();let id=store.manual("127.0.0.1").unwrap();
    let server=endpoint::generate_identity().unwrap();let pin=endpoint::fingerprint_of_pem(&server.0).unwrap();store.paired(&id,pin).unwrap();
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();store.host_mut(&id).unwrap().known.mgmt_port=Some(listener.local_addr().unwrap().port());
    let mut roots=RootCertStore::empty();roots.add(CertificateDer::from_pem_slice(store.identity.0.as_bytes()).unwrap()).unwrap();
    let verifier=WebPkiClientVerifier::builder(Arc::new(roots)).build().unwrap();
    let config=Arc::new(ServerConfig::builder().with_client_cert_verifier(verifier).with_single_cert(vec![CertificateDer::from_pem_slice(server.0.as_bytes()).unwrap()],PrivateKeyDer::from_pem_slice(server.1.as_bytes()).unwrap()).unwrap());
    let serving=std::thread::spawn(move || {
        let cases=[
            ("GET /api/v1/library/page?limit=100 ",r#"{"items":[{"id":"steam:570","store":"steam","title":"Dota"}],"next_cursor":"second"}"#),
            ("GET /api/v1/library/page?limit=100&cursor=second ",r#"{"items":[{"id":"custom:offline","store":"custom","title":"Missing","install":{"state":"missing"}}]}"#),
            ("GET /api/v1/status ",r#"{"games":[{"app_id":"steam:570","state":"running"},{"app_id":"custom:offline","state":"exited"}]}"#),
            ("POST /api/v1/game/end ",r#"{}"#),
            ("", ""),
        ];
        for (expected,body) in cases {
            let (tcp,_)=listener.accept().unwrap();tcp.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
            let mut stream=StreamOwned::new(ServerConnection::new(Arc::clone(&config)).unwrap(),tcp);
            let mut request=Vec::new();let mut byte=[0];
            let result=(||->std::io::Result<()> {while !request.ends_with(b"\r\n\r\n") {stream.read_exact(&mut byte)?;request.push(byte[0]);assert!(request.len()<8192);}Ok(())})();
            if expected.is_empty() {assert!(result.is_err(),"An incorrect pin reached HTTP");continue;}
            result.unwrap();assert!(String::from_utf8_lossy(&request).starts_with(expected));
            assert_eq!(stream.conn.peer_certificates().unwrap().len(),1);
            if expected.starts_with("POST") {
                let text=String::from_utf8(request).unwrap();let size=text.lines().find_map(|l|l.to_ascii_lowercase().strip_prefix("content-length:").map(|v|v.trim().parse::<usize>().unwrap())).unwrap();
                let mut b=vec![0;size];stream.read_exact(&mut b).unwrap();let b:serde_json::Value=serde_json::from_slice(&b).unwrap();assert_eq!(b["app_id"],"steam:570");assert_eq!(b["streaming"],true);
            }
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();stream.flush().unwrap();
        }
    });
    let host=store.host(&id).unwrap();let lib=Library::new(host,&store.identity).unwrap();
    let (first,next)=lib.page(None).unwrap();assert_eq!(first[0].id.as_str(),DESKTOP);assert_eq!(first[1].external[0].id(),"570");
    let (second,end)=lib.page(next.as_deref()).unwrap();assert!(end.is_none());assert_eq!(second[0].readiness,Readiness::Install);
    assert_eq!(lib.running().unwrap(),vec!["steam:570"]);lib.end_game(&ProviderGameId::new("steam:570").unwrap()).unwrap();
    let mut changed=host.clone();changed.known.fp_hex="00".repeat(32);assert!(Library::new(&changed,&store.identity).unwrap().page(None).is_err());
    serving.join().unwrap();
}
