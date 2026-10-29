//! The reader of the bridge's stills (`GET /v1/stills`, `multipart/x-mixed-replace; boundary=lumashow`): a JPEG a part, each with its
//! `Content-Length` (so nothing is scanned for a boundary) and the moment it was taken (`X-Captured-At`, epoch milliseconds).

use std::io::{self, BufRead, Read};

/// One picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub content_type: String,
    pub captured_at: Option<u64>,
    pub body: Vec<u8>,
}

/// The boundary a `Content-Type` names (`multipart/x-mixed-replace; boundary=lumashow`), quoted or not.
pub fn boundary_of(content_type: &str) -> Option<String> {
    let lower = content_type.to_ascii_lowercase();
    if !lower.starts_with("multipart/") {
        return None;
    }
    let at = lower.find("boundary=")?;
    let rest = content_type[at + "boundary=".len()..].trim();
    let rest = rest
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"');
    (!rest.is_empty()).then(|| rest.to_owned())
}

pub struct PartReader<R> {
    r: R,
    delimiter: String,
    max_part: usize,
}

impl<R: BufRead> PartReader<R> {
    /// `max_part` is the most one picture's bytes may be.
    pub fn new(r: R, boundary: &str, max_part: usize) -> Self {
        Self {
            r,
            delimiter: format!("--{boundary}"),
            max_part,
        }
    }

    fn line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        let n = self.r.by_ref().take(4096).read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        if !line.ends_with('\n') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a line that is too long",
            ));
        }
        Ok(Some(line.trim_end_matches(['\r', '\n']).to_owned()))
    }

    /// The next picture; `None` at the stream's end (the closing delimiter or the connection).
    pub fn next_part(&mut self) -> io::Result<Option<Part>> {
        // up to the delimiter (what is before it, the CRLF that ended the last part and any preamble, is nothing)
        loop {
            let Some(line) = self.line()? else {
                return Ok(None);
            };
            if line == format!("{}--", self.delimiter) {
                return Ok(None);
            }
            if line == self.delimiter {
                break;
            }
        }
        let (mut content_type, mut length, mut captured_at) = (String::new(), None, None);
        loop {
            let Some(line) = self.line()? else {
                return Ok(None);
            };
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                let value = value.trim();
                match name.trim().to_ascii_lowercase().as_str() {
                    "content-type" => content_type = value.to_owned(),
                    "content-length" => length = value.parse::<usize>().ok(),
                    "x-captured-at" => captured_at = value.parse::<u64>().ok(),
                    _ => {}
                }
            }
        }
        let Some(length) = length else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a part with no length",
            ));
        };
        if length > self.max_part {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a picture over the limit",
            ));
        }
        let mut body = vec![0u8; length];
        self.r.read_exact(&mut body)?;
        Ok(Some(Part {
            content_type,
            captured_at,
            body,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn part(body: &[u8], at: u64) -> Vec<u8> {
        let mut out = format!(
            "--lumashow\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nX-Captured-At: {at}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\r\n");
        out
    }

    #[test]
    fn the_boundary_is_read_from_the_content_type() {
        assert_eq!(
            boundary_of("multipart/x-mixed-replace; boundary=lumashow").as_deref(),
            Some("lumashow")
        );
        assert_eq!(
            boundary_of("multipart/x-mixed-replace;boundary=\"a b\"; x=1").as_deref(),
            Some("a b")
        );
        assert_eq!(boundary_of("image/jpeg"), None);
        assert_eq!(boundary_of("multipart/x-mixed-replace"), None);
    }

    #[test]
    fn pictures_come_out_whole_with_their_time_even_when_their_bytes_look_like_a_boundary() {
        let tricky = b"\xff\xd8--lumashow\r\n\r\n\xff\xd9".to_vec();
        let mut wire = Vec::new();
        wire.extend(part(b"first", 1_000));
        wire.extend(part(&tricky, 1_200));
        wire.extend_from_slice(b"--lumashow--\r\n");
        let mut reader = PartReader::new(Cursor::new(wire), "lumashow", 1024);
        let a = reader.next_part().unwrap().unwrap();
        assert_eq!(
            (a.body.as_slice(), a.captured_at, a.content_type.as_str()),
            (&b"first"[..], Some(1_000), "image/jpeg")
        );
        assert_eq!(reader.next_part().unwrap().unwrap().body, tricky);
        assert_eq!(reader.next_part().unwrap(), None);
    }

    #[test]
    fn a_stream_that_is_cut_a_part_with_no_length_and_a_picture_over_the_limit_are_errors() {
        let mut cut = part(b"0123456789", 1);
        cut.truncate(cut.len() - 6);
        assert!(
            PartReader::new(Cursor::new(cut), "lumashow", 1024)
                .next_part()
                .is_err()
        );
        let wire = b"--lumashow\r\nContent-Type: image/jpeg\r\n\r\nxx".to_vec();
        assert!(
            PartReader::new(Cursor::new(wire), "lumashow", 1024)
                .next_part()
                .is_err()
        );
        assert!(
            PartReader::new(Cursor::new(part(&[0; 100], 1)), "lumashow", 50)
                .next_part()
                .is_err()
        );
        assert_eq!(
            PartReader::new(Cursor::new(Vec::new()), "lumashow", 50)
                .next_part()
                .unwrap(),
            None
        );
    }
}
