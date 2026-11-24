//! The destination BeastNAS takes: one HTTP/1.1 `PUT` per file over plain TCP with a chunked body, so the file streams as it is muxed
//! and its length need not be known. There is no TLS and no login: the destination is a receiver on the LAN.
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::{Destination, Upload};

const CONNECT: Duration = Duration::from_secs(3);
/// A write or the answer that makes no progress for this long fails the upload, so a dead NAS cannot hold a job open.
const STALL: Duration = Duration::from_secs(10);

pub struct HttpDestination {
    host: String,
    port: u16,
    /// Always starts with `/` and ends without one.
    folder: String,
}

impl HttpDestination {
    /// `url` is `http://host[:port][/path]`; `folder` is appended to its path. Anything else (https, a bare host) is refused.
    pub fn new(url: &str, folder: &str) -> Result<Self, String> {
        let rest = url.strip_prefix("http://").ok_or("the address must start with http:// (clips go over the LAN, without TLS)")?;
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "the address has a port that is not a number")?),
            None => (authority, 80),
        };
        if host.is_empty() {
            return Err("the address names no host".to_owned());
        }
        let mut segments = Vec::new();
        for part in path.split('/').chain(folder.split('/')) {
            match part {
                "" | "." => {}
                ".." => return Err("the folder may not climb out with ..".to_owned()),
                p if p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.~ ".contains(&b)) => segments.push(p.replace(' ', "%20")),
                _ => return Err("the folder may use letters, digits and - _ . ~ only".to_owned()),
            }
        }
        let folder = segments.iter().fold(String::new(), |a, s| a + "/" + s);
        Ok(Self { host: host.to_owned(), port, folder })
    }
}

impl Destination for HttpDestination {
    fn open(&self, name: &str) -> io::Result<Box<dyn Upload>> {
        let addr = (self.host.as_str(), self.port).to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the host did not resolve"))?;
        let mut stream = TcpStream::connect_timeout(&addr, CONNECT)?;
        stream.set_write_timeout(Some(STALL))?;
        stream.set_read_timeout(Some(STALL))?;
        write!(
            stream,
            "PUT {}/{name} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: video/mp4\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            self.folder, self.host, self.port
        )?;
        Ok(Box::new(Put { stream }))
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
    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.stream.write_all(b"0\r\n\r\n")?;
        self.stream.flush()?;
        let mut status = String::new();
        // At most 256 bytes of the answer are read, so a broken server cannot make the TV buffer more.
        BufReader::new(io::Read::take(&self.stream, 256)).read_line(&mut status)?;
        match status.split_whitespace().nth(1) {
            Some("200" | "201" | "204") => Ok(()),
            _ => Err(io::Error::other(format!("the server answered {:?}", status.trim_end()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::thread;

    /// Accepts one request, returns (request line and headers, decoded body) and answers `status`.
    fn receiver(status: &'static str) -> (u16, thread::JoinHandle<(String, Vec<u8>)>) {
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
            let mut body = Vec::new();
            loop {
                let mut size = String::new();
                reader.read_line(&mut size).unwrap();
                let n = usize::from_str_radix(size.trim(), 16).unwrap();
                let mut chunk = vec![0; n + 2];
                reader.read_exact(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
            reader.get_mut().write_all(format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\n\r\n").as_bytes()).unwrap();
            (head, body)
        });
        (port, handle)
    }

    #[test]
    fn a_file_goes_up_as_one_chunked_put_and_the_server_answer_decides() {
        let (port, server) = receiver("201 Created");
        let dest = HttpDestination::new(&format!("http://127.0.0.1:{port}/clips"), "lumagame/tv one").unwrap();
        let mut up = dest.open("a.mp4").unwrap();
        up.write_all(b"hello ").unwrap();
        up.write_all(b"").unwrap();
        up.write_all(b"world").unwrap();
        up.finish().unwrap();
        let (head, body) = server.join().unwrap();
        assert!(head.starts_with("PUT /clips/lumagame/tv%20one/a.mp4 HTTP/1.1\r\n"), "{head}");
        assert!(head.contains("Transfer-Encoding: chunked\r\n"));
        assert_eq!(body, b"hello world");

        let (port, server) = receiver("403 Forbidden");
        let mut up = HttpDestination::new(&format!("http://127.0.0.1:{port}"), "").unwrap().open("b.mp4").unwrap();
        up.write_all(b"x").unwrap();
        assert!(up.finish().unwrap_err().to_string().contains("403"));
        server.join().unwrap();
    }

    #[test]
    fn addresses_that_are_not_plain_http_or_that_climb_out_of_the_folder_are_refused() {
        assert!(HttpDestination::new("https://nas/clips", "").is_err());
        assert!(HttpDestination::new("nas:8080", "").is_err());
        assert!(HttpDestination::new("http://nas:80x/", "").is_err());
        assert!(HttpDestination::new("http://nas", "../etc").is_err());
        assert!(HttpDestination::new("http://nas", "a?b").is_err());
        let d = HttpDestination::new("http://nas.local:9/x/", "/y//z/").unwrap();
        assert_eq!((d.port, d.folder.as_str()), (9, "/x/y/z"));
    }
}
