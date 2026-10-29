use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

pub const MAX_BODY: usize = 32 * 1024 * 1024;
pub struct Asset {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub body_start: usize,
    pub headers: Vec<(String, String)>,
}
impl Asset {
    pub fn bytes(&self) -> &[u8] {
        &self.body[self.body_start..]
    }
}
pub trait AssetSource: Send + Sync {
    fn fetch(&self, path: &str) -> Result<Asset, String>;
}
pub struct Amazon {
    tls: Arc<ClientConfig>,
    resolvers: Vec<SocketAddr>,
}
impl Amazon {
    pub fn new(ca_bundle: &[u8], resolv: &str) -> Result<Self, String> {
        let mut roots = RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut &*ca_bundle) {
            roots
                .add(cert.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
        if roots.is_empty() {
            return Err("system TLS CA bundle is empty".into());
        }
        let tls =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|e| e.to_string())?
                .with_root_certificates(roots)
                .with_no_client_auth();
        let resolvers: Vec<SocketAddr> = resolv
            .lines()
            .filter_map(|l| {
                let mut f = l.split_whitespace();
                (f.next()? == "nameserver").then_some(())?;
                Some(SocketAddr::new(f.next()?.parse::<IpAddr>().ok()?, 53))
            })
            .take(2)
            .collect();
        if resolvers.is_empty() {
            return Err("no DNS server is configured".into());
        }
        Ok(Self {
            tls: Arc::new(tls),
            resolvers,
        })
    }
}
impl AssetSource for Amazon {
    fn fetch(&self, path: &str) -> Result<Asset, String> {
        if !safe_path(path) {
            return Err("not an asset path".into());
        }
        // DNS is asked directly: ToSocketAddrs would resolve our own jail/legacy hosts override back to this listener.
        let mut address = None;
        for resolver in &self.resolvers {
            if let Ok(ip) = resolve(*resolver, crate::prime::HOST) {
                address = Some(SocketAddr::new(IpAddr::V4(ip), 443));
                break;
            }
        }
        let address = address.ok_or("Amazon asset DNS did not return a public IPv4 address")?;
        let socket = TcpStream::connect_timeout(&address, Duration::from_secs(8))
            .map_err(|e| e.to_string())?;
        socket
            .set_read_timeout(Some(Duration::from_secs(8)))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(Duration::from_secs(8)))
            .map_err(|e| e.to_string())?;
        let conn = ClientConnection::new(self.tls.clone(), crate::prime::HOST.try_into().unwrap())
            .map_err(|e| e.to_string())?;
        let mut tls = StreamOwned::new(conn, socket);
        write!(tls,"GET {path} HTTP/1.1\r\nHost: {}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",crate::prime::HOST).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        let complete = match tls.take((MAX_BODY + 65537) as u64).read_to_end(&mut bytes) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => false,
            Err(e) => return Err(e.to_string()),
        };
        parse_response(bytes, complete)
    }
}
pub fn safe_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && path.len() <= 4096
        && path
            .bytes()
            .all(|b| (0x21..=0x7e).contains(&b) && b != b'\\')
}
fn skip_name(bytes: &[u8], at: &mut usize) -> Result<(), String> {
    for _ in 0..128 {
        let n = *bytes.get(*at).ok_or("short DNS name")? as usize;
        *at += 1;
        if n == 0 {
            return Ok(());
        }
        if n & 0xc0 == 0xc0 {
            *at += 1;
            return if *at <= bytes.len() {
                Ok(())
            } else {
                Err("short DNS pointer".into())
            };
        }
        if n > 63 {
            return Err("bad DNS label".into());
        }
        *at += n;
        if *at > bytes.len() {
            return Err("short DNS label".into());
        }
    }
    Err("DNS name too long".into())
}
fn resolve(server: SocketAddr, host: &str) -> Result<Ipv4Addr, String> {
    let mut id = [0; 2];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut id))
        .map_err(|e| e.to_string())?;
    let mut query = vec![id[0], id[1], 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in host.split('.') {
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 1, 0, 1]);
    let udp = UdpSocket::bind(if server.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .map_err(|e| e.to_string())?;
    udp.set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    udp.connect(server).map_err(|e| e.to_string())?;
    udp.send(&query).map_err(|e| e.to_string())?;
    let mut reply = [0; 4096];
    let n = udp.recv(&mut reply).map_err(|e| e.to_string())?;
    let b = &reply[..n];
    if b.len() < 12 || b[..2] != id || b[2] & 0x82 != 0x80 || b[3] & 15 != 0 || b[4..6] != [0, 1] {
        return Err("invalid DNS answer".into());
    }
    let count = u16::from_be_bytes([b[6], b[7]]);
    let mut at = 12;
    skip_name(b, &mut at)?;
    at += 4;
    for _ in 0..count {
        skip_name(b, &mut at)?;
        let hdr = b.get(at..at + 10).ok_or("short DNS record")?;
        at += 10;
        let len = u16::from_be_bytes([hdr[8], hdr[9]]) as usize;
        let data = b.get(at..at + len).ok_or("short DNS data")?;
        at += len;
        if hdr[..4] == [0, 1, 0, 1] && len == 4 {
            let ip = Ipv4Addr::new(data[0], data[1], data[2], data[3]);
            if !ip.is_loopback()
                && !ip.is_private()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
            {
                return Ok(ip);
            }
        }
    }
    Err("no public A record".into())
}
fn parse_response(mut bytes: Vec<u8>, complete: bool) -> Result<Asset, String> {
    let split = bytes
        .windows(4)
        .position(|b| b == b"\r\n\r\n")
        .filter(|n| *n <= 65536)
        .ok_or("invalid asset response headers")?;
    let header = std::str::from_utf8(&bytes[..split]).map_err(|e| e.to_string())?;
    let status = header
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|s| (100..=599).contains(s))
        .ok_or("invalid asset HTTP status")?;
    let mut content_type = "application/octet-stream".to_owned();
    let mut chunked = false;
    let mut length = None;
    let mut headers = Vec::new();
    for l in header.lines().skip(1) {
        let Some((k, v)) = l.split_once(':') else {
            return Err("invalid asset header".into());
        };
        let v = v.trim();
        if v.bytes().any(|b| b < 32 || b == 127) {
            return Err("control character in asset response header".into());
        }
        match k.to_ascii_lowercase().as_str() {
            "content-type" => content_type = v.into(),
            "content-length" => {
                length = Some(v.parse::<usize>().map_err(|_| "invalid asset length")?)
            }
            "transfer-encoding" if v.eq_ignore_ascii_case("chunked") => chunked = true,
            "location" | "content-range" | "accept-ranges" => {
                headers.push((k.to_owned(), v.to_owned()))
            }
            "content-encoding" if !v.eq_ignore_ascii_case("identity") => {
                return Err("unexpected asset compression".into());
            }
            _ => {}
        }
    }
    let mut body_start = split + 4;
    if chunked {
        bytes = chunks(&bytes[body_start..])?;
        body_start = 0;
    } else {
        if length.is_some_and(|n| n != bytes.len() - body_start) || (!complete && length.is_none())
        {
            return Err("incomplete asset response".into());
        }
    }
    if bytes.len() - body_start > MAX_BODY {
        return Err("asset exceeds 32 MB".into());
    }
    Ok(Asset {
        status,
        content_type,
        body: bytes,
        body_start,
        headers,
    })
}
fn chunks(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let end = data
            .windows(2)
            .position(|b| b == b"\r\n")
            .ok_or("short HTTP chunk")?;
        let len = std::str::from_utf8(&data[..end])
            .ok()
            .and_then(|s| usize::from_str_radix(s.split(';').next()?, 16).ok())
            .ok_or("invalid HTTP chunk")?;
        if len == 0 {
            let tail = &data[end + 2..];
            if !tail.starts_with(b"\r\n") && !tail.ends_with(b"\r\n\r\n") {
                return Err("incomplete final HTTP chunk".into());
            }
            break;
        }
        data = &data[end + 2..];
        if len > MAX_BODY - body.len() || data.len() < len + 2 || &data[len..len + 2] != b"\r\n" {
            return Err("invalid HTTP chunk size".into());
        }
        body.extend_from_slice(&data[..len]);
        data = &data[len + 2..];
    }
    Ok(body)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_http_is_bounded_and_decoded() {
        assert_eq!(
            parse_response(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n"
                    .to_vec(),
                true
            )
            .unwrap()
            .bytes(),
            b"abc"
        );
        assert!(
            parse_response(
                b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nabc".to_vec(),
                true
            )
            .is_err()
        );
        assert!(!safe_path("//evil.example/path"));
        assert!(!safe_path("/x\r\nHost:evil"));
    }
}
