//! The reader of the bridge's push channel (`GET /v1/events/stream`, `text/event-stream`): events with an `id`, a name and a JSON `data`, and
//! comments (`: ping` every 15 s), which say the channel is alive and are nothing else.

use std::io::{self, BufRead, Read};

/// The longest a line or an event may be: the bridge's events are a few hundred bytes.
const MAX_LINE: usize = 64 * 1024;

/// One thing the channel said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sse {
    Event {
        id: Option<String>,
        name: String,
        data: String,
    },
    /// A comment line: the bridge's keep-alive.
    Comment,
    /// The read timed out with nothing more to read yet: the caller looks at the clock and reads again, and nothing is lost.
    Idle,
}

/// Reads events from a body.
pub struct SseReader<R> {
    r: R,
    line: String,
    // the event being put together: a comment in the middle of it does not lose what came before
    id: Option<String>,
    name: String,
    data: Vec<String>,
    size: usize,
}

impl<R: BufRead> SseReader<R> {
    pub fn new(r: R) -> Self {
        Self {
            r,
            line: String::new(),
            id: None,
            name: String::new(),
            data: Vec::new(),
            size: 0,
        }
    }

    /// The next event, comment or idle moment; `None` at the end of the channel. A read that fails (the connection dropped) is the error; one
    /// that merely timed out is [`Sse::Idle`], and what was half read is kept for the next call.
    pub fn next_event(&mut self) -> io::Result<Option<Sse>> {
        loop {
            let read = self
                .r
                .by_ref()
                .take(MAX_LINE as u64 + 1)
                .read_line(&mut self.line);
            match read {
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(Some(Sse::Idle));
                }
                Err(e) => return Err(e),
            }
            if self.line.len() > MAX_LINE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a line that is too long",
                ));
            }
            if !self.line.ends_with('\n') {
                // the end of the channel in the middle of a line: what was half said is not an event
                self.line.clear();
                return Ok(None);
            }
            let line = std::mem::take(&mut self.line);
            let text = line.trim_end_matches(['\n', '\r']);
            if text.is_empty() {
                // a blank line ends an event; one with nothing in it is not an event
                let (id, name, data) = (
                    self.id.take(),
                    std::mem::take(&mut self.name),
                    std::mem::take(&mut self.data),
                );
                self.size = 0;
                if !data.is_empty() || !name.is_empty() {
                    return Ok(Some(Sse::Event {
                        id,
                        name: if name.is_empty() {
                            "message".into()
                        } else {
                            name
                        },
                        data: data.join("\n"),
                    }));
                }
                continue;
            }
            if text.starts_with(':') {
                return Ok(Some(Sse::Comment));
            }
            let (field, value) = match text.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (text, ""),
            };
            match field {
                "event" => self.name = value.to_owned(),
                "id" => self.id = Some(value.to_owned()),
                "data" => {
                    self.size += value.len();
                    if self.size > MAX_LINE * 4 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "an event that is too long",
                        ));
                    }
                    self.data.push(value.to_owned());
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn all(text: &str) -> Vec<Sse> {
        let mut reader = SseReader::new(Cursor::new(text.as_bytes().to_vec()));
        let mut out = Vec::new();
        while let Some(item) = reader.next_event().unwrap() {
            out.push(item);
        }
        out
    }

    #[test]
    fn events_come_out_with_their_id_name_and_data_and_comments_are_alive_marks() {
        let got = all(
            "event: hello\ndata: {\"now\": 1}\n\n: ping\n\nid: 4021\nevent: event\ndata: {\"id\": \"e4021\"}\n\nid: 4022\ndata: a\ndata: b\n\n",
        );
        assert_eq!(
            got,
            vec![
                Sse::Event {
                    id: None,
                    name: "hello".into(),
                    data: "{\"now\": 1}".into()
                },
                Sse::Comment,
                Sse::Event {
                    id: Some("4021".into()),
                    name: "event".into(),
                    data: "{\"id\": \"e4021\"}".into()
                },
                Sse::Event {
                    id: Some("4022".into()),
                    name: "message".into(),
                    data: "a\nb".into()
                },
            ]
        );
    }

    #[test]
    fn crlf_lines_a_missing_space_and_a_half_event_at_the_end_are_handled() {
        let got = all("event:x\r\ndata:y\r\n\r\nevent: z\ndata: cut");
        assert_eq!(
            got,
            vec![Sse::Event {
                id: None,
                name: "x".into(),
                data: "y".into()
            }]
        );
    }

    /// A reader that gives its bytes a piece at a time and says it timed out between the pieces.
    struct Pieces {
        pieces: Vec<Vec<u8>>,
        at: usize,
        gap: bool,
    }

    impl io::Read for Pieces {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.gap {
                self.gap = false;
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let Some(piece) = self.pieces.get(self.at) else {
                return Ok(0);
            };
            self.at += 1;
            self.gap = true;
            out[..piece.len()].copy_from_slice(piece);
            Ok(piece.len())
        }
    }

    #[test]
    fn a_read_that_times_out_in_the_middle_of_an_event_loses_nothing() {
        let wire = b"event: event\nid: 7\ndata: {\"a\": 1}\n\n: ping\n\n";
        // cut in the middle of a line, and between the lines
        let pieces = vec![
            wire[..8].to_vec(),
            wire[8..22].to_vec(),
            wire[22..].to_vec(),
        ];
        let mut reader = SseReader::new(io::BufReader::new(Pieces {
            pieces,
            at: 0,
            gap: false,
        }));
        let mut got = Vec::new();
        for _ in 0..40 {
            match reader.next_event().unwrap() {
                None => break,
                Some(Sse::Idle) => {}
                Some(other) => got.push(other),
            }
        }
        assert_eq!(
            got,
            vec![
                Sse::Event {
                    id: Some("7".into()),
                    name: "event".into(),
                    data: "{\"a\": 1}".into()
                },
                Sse::Comment
            ]
        );
    }

    #[test]
    fn a_line_that_is_too_long_ends_the_channel_with_an_error() {
        let long = format!("data: {}\n\n", "a".repeat(MAX_LINE + 10));
        let mut reader = SseReader::new(Cursor::new(long.into_bytes()));
        assert!(reader.next_event().is_err());
    }
}
