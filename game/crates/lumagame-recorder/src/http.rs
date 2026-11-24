//! The receiver on BeastNAS, over plain HTTP/1.1 on the LAN: an upload is one `PUT` with a chunked body, so it streams as it is muxed
//! and its length need not be known; everything else is one short `POST`. There is no TLS; a token in the address is sent as a bearer.
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::{Answer, AskError, Destination, Upload};

const CONNECT: Duration = Duration::from_secs(3);
/// A write or the answer that makes no progress for this long fails the upload, so a dead NAS cannot hold a job open.
const STALL: Duration = Duration::from_secs(10);
/// A short request is answered at once or not at all: the live feed asks between two units and may not fall behind the ring for it.
const ASK: Duration = Duration::from_secs(3);
/// The most of an answer that is read, so a broken server cannot make the TV buffer more.
const ANSWER_BYTES: u64 = 4096;

pub struct HttpDestination {
    host: String,
    port: u16,
    /// The `Authorization` header line, or nothing.
    auth: String,
}

impl HttpDestination {
    /// `url` is `http://[token@]host[:port]`. Anything else (https, a bare host, a path) is refused.
    pub fn new(url: &str) -> Result<Self, String> {
        let rest = url.strip_prefix("http://").ok_or("the address must start with http:// (the stream goes over the LAN, without TLS)")?;
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains('/') {
            return Err("the address is the receiver's host and port, without a path".to_owned());
        }
        let (token, authority) = rest.rsplit_once('@').unwrap_or(("", rest));
        if !token.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.~".contains(&b)) {
            return Err("the token may use letters, digits and - _ . ~ only".to_owned());
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "the address has a port that is not a number")?),
            None => (authority, 80),
        };
        if host.is_empty() || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b)) {
            return Err("the address names no host".to_owned());
        }
        let auth = if token.is_empty() { String::new() } else { format!("Authorization: Bearer {token}\r\n") };
        Ok(Self { host: host.to_owned(), port, auth })
    }

    fn connect(&self, patience: Duration) -> io::Result<TcpStream> {
        let addr = (self.host.as_str(), self.port).to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the host did not resolve"))?;
        let stream = TcpStream::connect_timeout(&addr, CONNECT)?;
        stream.set_write_timeout(Some(patience))?;
        stream.set_read_timeout(Some(patience))?;
        Ok(stream)
    }
}

/// Percent-encodes a query value.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Reads the status and the body's `key=value` lines.
fn answer(stream: &TcpStream) -> io::Result<(u16, Answer)> {
    let mut text = Vec::new();
    stream.take(ANSWER_BYTES).read_to_end(&mut text)?;
    let text = String::from_utf8_lossy(&text);
    let status = text.split_whitespace().nth(1).and_then(|s| s.parse::<u16>().ok()).ok_or_else(|| io::Error::other("the server sent no answer"))?;
    Ok((status, Answer::parse(text.split_once("\r\n\r\n").map_or("", |(_, body)| body))))
}

fn refusal(status: u16, answer: &Answer) -> String {
    match answer.get("note") {
        "" => format!("the NAS answered {status}"),
        note => note.to_owned(),
    }
}

impl Destination for HttpDestination {
    fn open(&self, path: &str) -> io::Result<Box<dyn Upload>> {
        let mut stream = self.connect(STALL)?;
        write!(
            stream,
            "PUT {path} HTTP/1.1\r\nHost: {}:{}\r\n{}Content-Type: video/mp4\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            self.host, self.port, self.auth
        )?;
        Ok(Box::new(Put { stream }))
    }

    fn ask(&self, path: &str) -> Result<Answer, AskError> {
        let down = |e: io::Error| AskError::Down(e.to_string());
        let mut stream = self.connect(ASK).map_err(down)?;
        write!(stream, "POST {path} HTTP/1.1\r\nHost: {}:{}\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n", self.host, self.port, self.auth).map_err(down)?;
        match answer(&stream).map_err(down)? {
            (200..=299, answer) => Ok(answer),
            (status, answer) => Err(AskError::Refused(refusal(status, &answer))),
        }
    }
}

struct Put {
    stream: TcpStream,
}

impl Write for Put {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !buf.is_empty() {
            write!(self.stream, "{:x}\r\n", buf.len())?;
            self.stream.write_all(buf)?;
            self.stream.write_all(b"\r\n")?;
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl Upload for Put {
    fn finish(mut self: Box<Self>) -> io::Result<Answer> {
        self.stream.write_all(b"0\r\n\r\n")?;
        self.stream.flush()?;
        match answer(&self.stream)? {
            (200..=299, answer) => Ok(answer),
            (status, answer) => Err(io::Error::other(refusal(status, &answer))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::thread;

    /// Accepts one request, returns (request line and headers, decoded body) and answers `status` with `body`.
    fn receiver(status: &'static str, body: &'static str) -> (u16, thread::JoinHandle<(String, Vec<u8>)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut head = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let mut sent = Vec::new();
            while head.contains("chunked") {
                let mut size = String::new();
                reader.read_line(&mut size).unwrap();
                let n = usize::from_str_radix(size.trim(), 16).unwrap();
                let mut chunk = vec![0; n + 2];
                reader.read_exact(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                sent.extend_from_slice(&chunk[..n]);
            }
            reader.get_mut().write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
            (head, sent)
        });
        (port, handle)
    }

    #[test]
    fn an_upload_is_one_chunked_put_and_the_answer_names_the_file() {
        let (port, server) = receiver("200 OK", "name=Hades/a.mp4\nbytes=11\n");
        let dest = HttpDestination::new(&format!("http://s3cret@127.0.0.1:{port}/")).unwrap();
        let mut up = dest.open(&format!("/clip?game={}", encode("Hades II: Démo"))).unwrap();
        up.write_all(b"hello ").unwrap();
        up.write_all(b"").unwrap();
        up.write_all(b"world").unwrap();
        let told = up.finish().unwrap();
        assert_eq!((told.get("name"), told.number("bytes"), told.get("absent")), ("Hades/a.mp4", 11, ""));
        let (head, body) = server.join().unwrap();
        assert!(head.starts_with("PUT /clip?game=Hades%20II%3A%20D%C3%A9mo HTTP/1.1\r\n"), "{head}");
        assert!(head.contains("Transfer-Encoding: chunked\r\n") && head.contains("Authorization: Bearer s3cret\r\n"));
        assert_eq!(body, b"hello world");
    }

    #[test]
    fn a_request_is_answered_refused_with_the_receivers_sentence_or_not_at_all() {
        let (port, server) = receiver("200 OK", "reach=300\nnote=\n");
        let dest = HttpDestination::new(&format!("http://127.0.0.1:{port}")).unwrap();
        assert_eq!(dest.ask("/open?session=a").unwrap().number("reach"), 300);
        let (head, _) = server.join().unwrap();
        assert!(head.starts_with("POST /open?session=a HTTP/1.1\r\n") && !head.contains("Authorization"), "{head}");

        let (port, server) = receiver("507 Insufficient Storage", "note=The recordings share is nearly full\n");
        let dest = HttpDestination::new(&format!("http://127.0.0.1:{port}")).unwrap();
        assert_eq!(dest.ask("/record/start").unwrap_err(), AskError::Refused("The recordings share is nearly full".into()));
        server.join().unwrap();
        assert!(matches!(dest.ask("/open"), Err(AskError::Down(_))), "nothing listens there any more");
    }

    #[test]
    fn addresses_that_are_not_a_plain_http_host_and_port_are_refused() {
        for bad in ["https://nas", "nas:8080", "http://nas:80x", "http://nas/clips", "http://", "http://to ken@nas", "http://na s"] {
            assert!(HttpDestination::new(bad).is_err(), "{bad}");
        }
        let d = HttpDestination::new("http://nas.local:9/").unwrap();
        assert_eq!((d.host.as_str(), d.port, d.auth.as_str()), ("nas.local", 9, ""));
        assert_eq!(HttpDestination::new("http://192.168.1.69").unwrap().port, 80);
    }
}
