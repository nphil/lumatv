//! `GET` of one address of the stream (a playlist, the initialisation segment, a part): plain HTTP to the home network only, judged by
//! what the name resolves to (`ha_link::AddressPolicy`, the rule every address Home Assistant hands the TV is held to), one connection
//! per request, no redirect, a cap on the body. Home Assistant's internal address is plain HTTP on the LAN; an `https` address is refused
//! in words (the live path does not carry a TLS client; the pictures' route does, through the Show's `fetch_signed`).

use ha_link::{AddressPolicy, Endpoint};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Fetched {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// Splits `url` into its origin (`http://host:port`) and its request target (`/path?query`).
fn split(url: &str) -> Result<(&str, &str), String> {
    let after = url.find("://").ok_or("the address has no scheme")? + 3;
    let slash = url[after..]
        .find('/')
        .map(|i| i + after)
        .unwrap_or(url.len());
    let (origin, target) = url.split_at(slash);
    Ok((origin, if target.is_empty() { "/" } else { target }))
}

pub fn get(
    url: &str,
    policy: AddressPolicy,
    timeout: Duration,
    max_bytes: usize,
) -> Result<Fetched, String> {
    let (origin, target) = split(url)?;
    let endpoint = Endpoint::parse(origin).map_err(|e| e.to_string())?;
    if endpoint.secure {
        return Err("the live path speaks plain HTTP only; Home Assistant's internal address is an https one".to_owned());
    }
    if target.bytes().any(|b| b.is_ascii_control() || b == b' ') {
        return Err("the address holds a character that cannot go in a request".to_owned());
    }
    let addrs = endpoint.resolve(policy).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    let mut last = String::from("no address");
    let mut stream = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout.min(Duration::from_secs(3))) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = e.to_string(),
        }
    }
    let mut stream = stream.ok_or_else(|| format!("cannot connect: {last}"))?;
    stream.set_nodelay(true).ok();
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let request = format!(
        "GET {target} HTTP/1.1\r\nHost: {}\r\nAccept: */*\r\nAccept-Encoding: identity\r\nUser-Agent: lumatv\r\nConnection: close\r\n\r\n",
        endpoint.authority()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("cannot send: {e}"))?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 16384];
    let head_end = loop {
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if raw.len() > 64 * 1024 {
            return Err("the answer's head is too long".to_owned());
        }
        if Instant::now() > deadline {
            return Err("the answer took too long".to_owned());
        }
        let n = stream
            .read(&mut buf)
            .map_err(|e| format!("cannot read: {e}"))?;
        if n == 0 {
            return Err("the connection closed before the head".to_owned());
        }
        raw.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("not an HTTP answer: {status_line:?}"))?;
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    let mut content_type = String::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        match name.as_str() {
            "content-length" => content_length = value.parse().ok(),
            "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
            "content-type" => {
                content_type = value
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            }
            _ => {}
        }
    }
    let mut body = raw[head_end..].to_vec();
    // the rest of the body: to the length, or to the close
    loop {
        if let Some(n) = content_length
            && !chunked
            && body.len() >= n
        {
            body.truncate(n);
            break;
        }
        if body.len() > max_bytes {
            return Err("the answer is larger than allowed".to_owned());
        }
        if Instant::now() > deadline {
            return Err("the answer took too long".to_owned());
        }
        let n = stream
            .read(&mut buf)
            .map_err(|e| format!("cannot read: {e}"))?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    if chunked {
        body = dechunk(&body)?;
    }
    Ok(Fetched {
        status,
        content_type,
        body,
    })
}

fn dechunk(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(data.len());
    let mut at = 0;
    loop {
        let line_end = data[at..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("a chunk size line is cut")?
            + at;
        let size_text =
            std::str::from_utf8(&data[at..line_end]).map_err(|_| "a chunk size is not text")?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "a chunk size is not a number")?;
        at = line_end + 2;
        if size == 0 {
            return Ok(out);
        }
        let chunk = data.get(at..at + size).ok_or("a chunk is cut")?;
        out.extend_from_slice(chunk);
        at += size + 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve(reply: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut b = [0u8; 2048];
                let _ = s.read(&mut b);
                let _ = s.write_all(reply);
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn a_plain_answer_and_a_chunked_one_are_read_and_https_is_refused() {
        let base = serve(b"HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: 8\r\nConnection: close\r\n\r\n#EXTM3U\n");
        let f = get(
            &format!("{base}/playlist.m3u8"),
            AddressPolicy::TESTING,
            Duration::from_secs(2),
            1 << 20,
        )
        .unwrap();
        assert_eq!(
            (f.status, f.content_type.as_str(), f.body.as_slice()),
            (200, "application/vnd.apple.mpegurl", &b"#EXTM3U\n"[..])
        );
        let base = serve(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n2\r\nef\r\n0\r\n\r\n");
        let f = get(
            &format!("{base}/x"),
            AddressPolicy::TESTING,
            Duration::from_secs(2),
            1 << 20,
        )
        .unwrap();
        assert_eq!(f.body, b"abcdef");
        assert!(
            get(
                "https://127.0.0.1:1/x",
                AddressPolicy::TESTING,
                Duration::from_secs(1),
                10
            )
            .unwrap_err()
            .contains("plain HTTP")
        );
        assert!(
            get(
                "http://127.0.0.1:1/x y",
                AddressPolicy::TESTING,
                Duration::from_secs(1),
                10
            )
            .is_err()
        );
    }
}
