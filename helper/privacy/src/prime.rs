//! Prime's asset host only. No system trust-store, DRM, licence or rootfs writes; the only Prime configuration written is the removal of
//! an earlier AmazOff's certificate-check override (`legacy::clear_tls_override`).
use crate::{
    Paths, Platform,
    assets::{Amazon, AssetSource},
    atomic, hosts, legacy, noads, read_json, remove, sha, text,
};
use parking_lot::Mutex;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, GeneralSubtree, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, NameConstraints,
};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
pub const HOST: &str = "cloudfront.xp-assets.aiv-cdn.net";
pub const CERT_DIR: &str = "/media/cryptofs/apps/usr/palm/applications/amazon/bin/certs";
pub const JAIL_HOSTS: &str = "/var/palm/jail/amazon/etc/hosts";
/// Prime Video's application id.
pub const APP: &str = "amazon";
const OLD_CA_SHA: &str = "4ca2f57b4a9958bf7743c9234aa12e80efaf7dcd8351ad624874666b700ef4f6";
#[derive(Serialize, Deserialize)]
struct Identity {
    ca: String,
    leaf: String,
    key: Vec<u8>,
    pin: Option<String>,
}
fn identity() -> Result<Identity, String> {
    let mut ca = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    ca.distinguished_name
        .push(DnType::CommonName, "LumaTV per-TV Prime asset CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![GeneralSubtree::DnsName(HOST.into())],
        excluded_subtrees: vec![],
    });
    ca.not_before = rcgen::date_time_ymd(2023, 1, 1);
    ca.not_after = rcgen::date_time_ymd(2046, 1, 1);
    let ca_key = KeyPair::generate().map_err(|e| e.to_string())?;
    let ca_cert = ca.self_signed(&ca_key).map_err(|e| e.to_string())?;
    let mut params = CertificateParams::new(vec![HOST.into()]).map_err(|e| e.to_string())?;
    params.distinguished_name.push(DnType::CommonName, HOST);
    params.not_before = rcgen::date_time_ymd(2023, 1, 1);
    params.not_after = rcgen::date_time_ymd(2045, 12, 31);
    let leaf_key = KeyPair::generate().map_err(|e| e.to_string())?;
    let issuer = Issuer::from_params(&ca, ca_key);
    let leaf = params
        .signed_by(&leaf_key, &issuer)
        .map_err(|e| e.to_string())?;
    // ca_key lives only in issuer, which is dropped here. It is never serialized or written.
    Ok(Identity {
        ca: ca_cert.pem(),
        leaf: leaf.pem(),
        key: leaf_key.serialize_der(),
        pin: None,
    })
}
fn server(identity: &Identity) -> Result<Arc<ServerConfig>, String> {
    let certs = rustls_pemfile::certs(&mut identity.leaf.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.key.clone());
    ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key.into())
        .map(Arc::new)
        .map_err(|e| e.to_string())
}
fn remove_legacy(paths: &Paths) -> Result<(), String> {
    for entry in std::fs::read_dir(paths.system(CERT_DIR)).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file()
            || entry.metadata().map_err(|e| e.to_string())?.len() > 65536
        {
            continue;
        }
        let pem = std::fs::read(entry.path()).map_err(|e| e.to_string())?;
        let certs = rustls_pemfile::certs(&mut pem.as_slice()).collect::<Result<Vec<_>, _>>();
        if let Ok(certs) = certs {
            if certs.len() == 1 && sha(certs[0].as_ref()) == OLD_CA_SHA {
                remove(&entry.path())?;
            }
        }
    }
    Ok(())
}
pub fn remove_redirect(paths: &Paths) -> Result<(), String> {
    let path = paths.system(JAIL_HOSTS);
    let current = text(&path)?;
    let next = hosts::prime_hosts(&current, false);
    // Jail hosts is not rootfs. Write in place so a jail's existing bind retains this inode.
    std::fs::write(path, next).map_err(|e| e.to_string())
}
/// Everything the switch does before it listens, so the owner has nothing to do first: Prime Video is closed (its certificates and
/// hosts are read when it starts), what an earlier AmazOff left running or set to start is stopped, and its one change to Prime's
/// configuration, which switched certificate checks off, is taken back. Nothing else of Prime's configuration is written.
pub fn retire_old(paths: &Paths, platform: &dyn Platform) -> Result<(), String> {
    if !paths.system(CERT_DIR).is_dir() || !paths.system(JAIL_HOSTS).is_file() {
        return Err("Prime Video has not run on this TV yet".into());
    }
    let _ = platform.close_app(APP);
    legacy::retire_autostart(paths)?;
    platform.stop_old_amazoff()?;
    legacy::clear_tls_override(paths)?;
    Ok(())
}
pub fn enable(paths: &Paths, platform: &dyn Platform) -> Result<Front, String> {
    if !paths.system(CERT_DIR).is_dir() || !paths.system(JAIL_HOSTS).is_file() {
        return Err("Prime Video has not run on this TV yet".into());
    }
    let config = paths.system(legacy::PRIME_CONFIG);
    if config.is_file() {
        let v = read_json(&config)?;
        if v["disable-ssl-cert"] == "1"
            || v["disable-ssl-cert"] == true
            || v["disable-ssl-cert"] == 1
        {
            return Err("Prime Video still has its certificate checks switched off".into());
        }
    }
    let file = paths.state("prime-identity.json");
    let mut id = if file.is_file() {
        serde_json::from_value(read_json(&file)?)
            .map_err(|e| format!("invalid per-TV identity: {e}"))?
    } else {
        identity()?
    };
    let tls = server(&id)?;
    let source = Arc::new(Amazon::new(
        &std::fs::read(paths.system("/etc/ssl/certs/ca-certificates.crt"))
            .map_err(|e| e.to_string())?,
        &text(&paths.system("/etc/resolv.conf"))?,
    )?);
    // Bind before any trust/hosts changes. `retire_old` has stopped an old AmazOff; anything else on the port is a visible conflict.
    let front = Front::start("127.0.0.1:443".parse().unwrap(), tls, source)?;
    let ca_file = paths.state("prime-ca.pem");
    atomic(&ca_file, id.ca.as_bytes(), false)?;
    let hash = platform.cert_hash(&ca_file)?;
    if hash.len() != 8 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("openssl returned an invalid certificate hash".into());
    }
    remove_legacy(paths)?;
    if let Some(pin) = &id.pin {
        if !valid_pin(pin) {
            return Err("invalid saved Prime certificate filename".into());
        }
        let target = paths.system(CERT_DIR).join(pin);
        if target.is_file() && text(&target)? != id.ca {
            return Err(
                "the saved Prime certificate filename is now owned by another certificate".into(),
            );
        }
    } else {
        for suffix in 0..100 {
            let pin = format!("{hash}.{suffix}");
            if !paths.system(CERT_DIR).join(&pin).exists() {
                id.pin = Some(pin);
                break;
            }
        }
    }
    let pin = id
        .pin
        .as_ref()
        .ok_or("Prime certificate hash slots are full")?;
    // Save the exact pin before installation so interruption can always be undone by Off.
    atomic(
        &file,
        &serde_json::to_vec(&id).map_err(|e| e.to_string())?,
        true,
    )?;
    let target = paths.system(CERT_DIR).join(pin);
    let before = text(&paths.system(JAIL_HOSTS))?;
    let activated = (|| {
        atomic(&target, id.ca.as_bytes(), false)?;
        std::fs::write(paths.system(JAIL_HOSTS), hosts::prime_hosts(&before, true))
            .map_err(|e| e.to_string())?;
        atomic(&paths.state("prime.enabled"), b"on\n", false)
    })();
    if let Err(e) = activated {
        let _ = remove(&target);
        let _ = std::fs::write(paths.system(JAIL_HOSTS), hosts::prime_hosts(&before, false));
        return Err(e);
    }
    Ok(front)
}
fn valid_pin(pin: &str) -> bool {
    let Some((hash, n)) = pin.split_once('.') else {
        return false;
    };
    hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && n.parse::<u8>().is_ok()
}
pub fn disable(paths: &Paths) -> Result<(), String> {
    // Redirect first: Prime must never be left pointing to a listener that is already gone.
    if paths.system(JAIL_HOSTS).exists() {
        remove_redirect(paths)?;
    }
    let identity_file = paths.state("prime-identity.json");
    if identity_file.is_file() {
        let id: Identity =
            serde_json::from_value(read_json(&identity_file)?).map_err(|e| e.to_string())?;
        if let Some(pin) = id.pin {
            if !valid_pin(&pin) {
                return Err("invalid saved Prime pin".into());
            }
            let path = paths.system(CERT_DIR).join(pin);
            if path.is_file() {
                if text(&path)? != id.ca {
                    return Err(
                        "Prime's certificate was changed externally; it was not removed".into(),
                    );
                }
                remove(&path)?;
            }
        }
    }
    if paths.system(CERT_DIR).is_dir() {
        remove_legacy(paths)?;
    }
    remove(&paths.state("prime.enabled"))?;
    remove(&paths.state("prime-identity.json"))?;
    remove(&paths.state("prime-ca.pem"))
}

pub struct Front {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    routes: Arc<Mutex<BTreeMap<String, Value>>>,
}
impl Front {
    pub fn start(
        addr: SocketAddr,
        tls: Arc<ServerConfig>,
        source: Arc<dyn AssetSource>,
    ) -> Result<Self, String> {
        if !addr.ip().is_loopback() {
            return Err("Prime assets may only listen on loopback".into());
        }
        let listener = TcpListener::bind(addr).map_err(|e| {
            format!("Another program is using the port Prime Video ads need: {e}")
        })?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let routes = Arc::new(Mutex::new(BTreeMap::new()));
        let s = stop.clone();
        let r = routes.clone();
        let active = Arc::new(AtomicUsize::new(0));
        let thread = thread::Builder::new()
            .name("prime-assets".into())
            .spawn(move || {
                while !s.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((socket, _)) => {
                            if active.load(Ordering::Acquire) >= 4 {
                                drop(socket);
                                continue;
                            }
                            active.fetch_add(1, Ordering::AcqRel);
                            let t = tls.clone();
                            let src = source.clone();
                            let r = r.clone();
                            let a = active.clone();
                            if thread::Builder::new()
                                .name("prime-asset".into())
                                .spawn(move || {
                                    let _ = serve(socket, t, src.as_ref(), &r);
                                    a.fetch_sub(1, Ordering::AcqRel);
                                })
                                .is_err()
                            {
                                active.fetch_sub(1, Ordering::AcqRel);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(50))
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            thread: Some(thread),
            routes,
        })
    }
    pub fn routes(&self) -> parking_lot::MutexGuard<'_, BTreeMap<String, Value>> {
        self.routes.lock()
    }
}
impl Drop for Front {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn serve(
    socket: TcpStream,
    tls: Arc<ServerConfig>,
    source: &dyn AssetSource,
    routes: &Mutex<BTreeMap<String, Value>>,
) -> Result<(), String> {
    socket
        .set_read_timeout(Some(Duration::from_secs(8)))
        .map_err(|e| e.to_string())?;
    socket
        .set_write_timeout(Some(Duration::from_secs(8)))
        .map_err(|e| e.to_string())?;
    let conn = ServerConnection::new(tls).map_err(|e| e.to_string())?;
    let mut stream = StreamOwned::new(conn, socket);
    let mut header = Vec::with_capacity(2048);
    let mut byte = [0; 1];
    while !header.ends_with(b"\r\n\r\n") && header.len() < 16384 {
        if stream.read(&mut byte).map_err(|e| e.to_string())? == 0 {
            return Err("incomplete request".into());
        }
        header.push(byte[0]);
    }
    let req = std::str::from_utf8(&header).map_err(|_| "non-UTF8 asset request")?;
    let mut first = req.lines().next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("");
    let path = first.next().unwrap_or("");
    let host = req.lines().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case("host").then_some(v.trim())
    });
    if !header.ends_with(b"\r\n\r\n")
        || !matches!(method, "GET" | "HEAD")
        || !crate::assets::safe_path(path)
        || !host.is_some_and(|h| {
            h.eq_ignore_ascii_case(HOST) || h.eq_ignore_ascii_case(&format!("{HOST}:443"))
        })
    {
        return Err("not a request for the Prime asset host".into());
    }
    let mut asset = match source.fetch(path) {
        Ok(a) => a,
        Err(e) => {
            write!(
                stream,
                "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .map_err(|e| e.to_string())?;
            return Err(e);
        }
    };
    if let Some(route) = noads::player_route(path) {
        let source_sha = sha(asset.bytes());
        let result = if asset.status == 200 {
            std::str::from_utf8(asset.bytes())
                .map_err(|_| "player is not UTF8".to_owned())
                .and_then(noads::transform)
        } else {
            Err(format!("upstream HTTP {}", asset.status))
        };
        let entry = match result {
            Ok(patched) => {
                asset.body = patched.into_bytes();
                asset.body_start = 0;
                json!({"state":"patched","sourceSha256":source_sha,"patchedSha256":sha(asset.bytes())})
            }
            Err(reason) => json!({"state":"unmodified","reason":reason,"sourceSha256":source_sha}),
        };
        let mut r = routes.lock();
        if r.len() >= 32 && !r.contains_key(route) {
            r.pop_first();
        }
        r.insert(route.into(), entry);
    }
    write!(stream,"HTTP/1.1 {} Asset\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",asset.status,asset.content_type,asset.bytes().len()).map_err(|e|e.to_string())?;
    for (k, v) in &asset.headers {
        write!(stream, "{k}: {v}\r\n").map_err(|e| e.to_string())?;
    }
    stream.write_all(b"\r\n").map_err(|e| e.to_string())?;
    if method == "GET" {
        stream.write_all(asset.bytes()).map_err(|e| e.to_string())?;
    }
    stream.conn.send_close_notify();
    stream.flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn per_tv_ca_is_unique_and_server_has_no_ca_key() {
        let a = identity().unwrap();
        let b = identity().unwrap();
        assert_ne!(a.ca, b.ca);
        assert_ne!(a.key, b.key);
        assert!(!a.ca.contains("PRIVATE KEY"));
        assert!(!a.leaf.contains("PRIVATE KEY"));
        assert!(server(&a).is_ok());
        let j = serde_json::to_value(&a).unwrap();
        assert_eq!(j.as_object().unwrap().len(), 4);
        assert!(!valid_pin("../../etc/passwd"));
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("ca.pem");
        std::fs::write(&cert, &a.ca).unwrap();
        let output = std::process::Command::new("openssl")
            .args(["x509", "-in"])
            .arg(&cert)
            .args(["-text", "-noout"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let public = String::from_utf8(output.stdout).unwrap();
        assert!(public.contains("X509v3 Name Constraints: critical"));
        assert!(public.contains(&format!("DNS:{HOST}")));
        assert!(public.contains("CA:TRUE, pathlen:0"));
    }
    struct Source;
    impl AssetSource for Source {
        fn fetch(&self, path: &str) -> Result<crate::assets::Asset, String> {
            Ok(crate::assets::Asset {
                status: 200,
                content_type: "application/javascript".into(),
                body_start: 0,
                headers: vec![],
                body: if path.contains("wave3") {
                    include_bytes!("../tests/player.js").to_vec()
                } else {
                    b"original asset bytes".to_vec()
                },
            })
        }
    }
    #[test]
    fn verified_tls_filters_the_requested_route_and_passes_other_assets_through() {
        for path in [
            "/family/lg/ATVUnfPlayerBundle-1.0/wave3/js/ATVUnfPlayerBundle.js",
            "/family/lg/ATVUnfPlayerBundle-1.0/new/js/ATVUnfPlayerBundle.js",
            "/packages/SyeCorePlayerNativeModule-1.0/prod/ignite/module.so",
        ] {
            let id = identity().unwrap();
            let config = server(&id).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let routes = Arc::new(Mutex::new(BTreeMap::new()));
            let r = routes.clone();
            let worker = thread::spawn(move || {
                let (socket, _) = listener.accept().unwrap();
                serve(socket, config, &Source, &r).unwrap();
            });
            let mut roots = rustls::RootCertStore::empty();
            for cert in rustls_pemfile::certs(&mut id.ca.as_bytes()) {
                roots.add(cert.unwrap()).unwrap();
            }
            let client = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
            let socket = TcpStream::connect(address).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let conn =
                rustls::ClientConnection::new(Arc::new(client), HOST.try_into().unwrap()).unwrap();
            let mut tls = StreamOwned::new(conn, socket);
            write!(tls, "GET {path} HTTP/1.1\r\nHost: {HOST}\r\n\r\n").unwrap();
            let mut response = Vec::new();
            tls.read_to_end(&mut response).unwrap();
            worker.join().unwrap();
            let response = String::from_utf8(response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"));
            assert!(response.contains("Cache-Control: no-store"));
            if path.contains("wave3") {
                assert!(response.contains("__HOMEBREW_NOADS"));
                assert_eq!(routes.lock()["wave3"]["state"], "patched");
            } else {
                assert!(response.ends_with("original asset bytes"));
                if path.contains("/new/") {
                    assert_eq!(routes.lock()["new"]["state"], "unmodified");
                } else {
                    assert!(routes.lock().is_empty());
                }
            }
        }
    }
    #[test]
    fn off_removes_only_owned_cert_and_redirect_and_destroys_leaf_key() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            root: dir.path().join("root"),
            data: dir.path().join("state"),
        };
        let mut id = identity().unwrap();
        id.pin = Some("01234567.0".into());
        atomic(
            &paths.state("prime-identity.json"),
            &serde_json::to_vec(&id).unwrap(),
            true,
        )
        .unwrap();
        atomic(&paths.state("prime.enabled"), b"on", false).unwrap();
        atomic(
            &paths.system(CERT_DIR).join("01234567.0"),
            id.ca.as_bytes(),
            false,
        )
        .unwrap();
        atomic(
            &paths.system(CERT_DIR).join("keep.0"),
            b"unrelated certificate",
            false,
        )
        .unwrap();
        atomic(
            &paths.system(JAIL_HOSTS),
            format!("127.0.0.1 localhost\n127.0.0.1 {HOST}\n::1 {HOST}\n").as_bytes(),
            false,
        )
        .unwrap();
        disable(&paths).unwrap();
        assert!(!paths.state("prime-identity.json").exists());
        assert!(!paths.state("prime.enabled").exists());
        assert!(!paths.system(CERT_DIR).join("01234567.0").exists());
        assert!(paths.system(CERT_DIR).join("keep.0").exists());
        assert_eq!(
            text(&paths.system(JAIL_HOSTS)).unwrap(),
            "127.0.0.1 localhost\n"
        );
        disable(&paths).unwrap();
    }
    #[test]
    fn legacy_public_ca_is_removed_by_fingerprint_not_filename_when_available() {
        let Some(dir) = std::env::var_os("PRIVACY_REFERENCE_DIR") else {
            return;
        };
        let pem = std::fs::read(
            std::path::PathBuf::from(dir).join("upstream/resources/amazon/tls/fe1de658.0"),
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            root: dir.path().join("root"),
            data: dir.path().join("state"),
        };
        let cert_dir = paths.system(CERT_DIR);
        atomic(&cert_dir.join("legacy-renamed.3"), &pem, false).unwrap();
        atomic(
            &cert_dir.join("keep.0"),
            identity().unwrap().ca.as_bytes(),
            false,
        )
        .unwrap();
        remove_legacy(&paths).unwrap();
        assert!(!cert_dir.join("legacy-renamed.3").exists());
        assert!(cert_dir.join("keep.0").exists());
    }
    #[test]
    fn damaged_ownership_record_is_not_silently_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            root: dir.path().join("root"),
            data: dir.path().join("state"),
        };
        atomic(&paths.state("prime-identity.json"), b"broken", true).unwrap();
        assert!(disable(&paths).is_err());
        assert!(paths.state("prime-identity.json").exists());
    }
}
