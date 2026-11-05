//! The network, as the host lends it: one request, one answer. The host decides what may be reached (the TV's helper lets the guide
//! reach only the services of the sources the owner switched on and the Plex server's typed address) and how; the engine only says
//! what it wants.

use std::fmt;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
        }
    }
}

/// One request. Headers may carry a credential (Plex's token, Schedules Direct's): its `Debug` says none of them, nor the query.
#[derive(Clone)]
pub struct Request<'a> {
    pub method: Method,
    /// `https://host/path?query`, or `http://` for a server on the home network.
    pub url: &'a str,
    pub headers: Vec<(&'a str, &'a str)>,
    /// A JSON body (sent with `Content-Type: application/json`).
    pub body: Option<&'a [u8]>,
    /// The most bytes of the answer's body that are wanted.
    pub max_bytes: usize,
    pub timeout: Duration,
}

impl<'a> Request<'a> {
    pub fn get(url: &'a str) -> Self {
        Self {
            method: Method::Get,
            url,
            headers: Vec::new(),
            body: None,
            max_bytes: 4 << 20,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn header(mut self, name: &'a str, value: &'a str) -> Self {
        self.headers.push((name, value));
        self
    }

    /// The scheme, host and port the request goes to, lowercased: what the host's rules judge.
    pub fn origin(&self) -> Option<Origin> {
        Origin::of(self.url)
    }
}

impl fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.url.split('?').next().unwrap_or("");
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &path)
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>(),
            )
            .field("body", &self.body.map(<[u8]>::len))
            .finish()
    }
}

/// Where a request goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub https: bool,
    pub host: String,
    pub port: u16,
}

impl Origin {
    /// The origin of an `http://` or `https://` address with no user information.
    pub fn of(url: &str) -> Option<Origin> {
        let (https, rest) = match url.split_once("://")? {
            ("https", rest) => (true, rest),
            ("http", rest) => (false, rest),
            _ => return None,
        };
        let authority = rest.split(['/', '?', '#']).next()?;
        if authority.is_empty() || authority.contains('@') {
            return None;
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok()?),
            None => (authority, if https { 443 } else { 80 }),
        };
        (!host.is_empty()).then(|| Origin {
            https,
            host: host.to_ascii_lowercase(),
            port,
        })
    }
}

/// The answer, whatever its status: a service says why in its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Why a request got no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    /// Nothing answered: no route, no name, a refused connection.
    Offline,
    Timeout,
    /// The answer was longer than the request allowed.
    TooLarge,
    /// The host does not let the guide reach that address, in words.
    NotAllowed(String),
    /// Anything else, in words.
    Failed(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Offline => f.write_str("no connection"),
            NetError::Timeout => f.write_str("the request took too long"),
            NetError::TooLarge => f.write_str("the answer was too large"),
            NetError::NotAllowed(why) | NetError::Failed(why) => f.write_str(why),
        }
    }
}

/// Makes requests: the host's.
pub trait Http: Send + Sync {
    fn send(&self, request: &Request<'_>) -> Result<Response, NetError>;
}

/// `text` for a query string: everything but the unreserved characters percent-encoded.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_origin_is_what_the_host_judges() {
        assert_eq!(
            Origin::of("https://Json.SchedulesDirect.org/20141201/token"),
            Some(Origin {
                https: true,
                host: "json.schedulesdirect.org".into(),
                port: 443
            })
        );
        assert_eq!(
            Origin::of("http://192.168.1.69:32400/livetv/dvrs"),
            Some(Origin {
                https: false,
                host: "192.168.1.69".into(),
                port: 32400
            })
        );
        assert_eq!(Origin::of("http://user@192.168.1.69/"), None);
        assert_eq!(Origin::of("ftp://example.com/"), None);
    }

    #[test]
    fn a_request_never_shows_its_query_or_its_header_values() {
        let r = Request::get("http://192.168.1.69:32400/grid?X-Plex-Token=secret")
            .header("X-Plex-Token", "secret");
        let shown = format!("{r:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("X-Plex-Token"), "{shown}");
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(
            encode("lineup://tv.plex/5fc7#Local Broadcast"),
            "lineup%3A%2F%2Ftv.plex%2F5fc7%23Local%20Broadcast"
        );
    }
}
