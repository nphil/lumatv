//! The one error type of the crate.

use std::fmt;

use fetch::FetchError;

/// Why a refresh gave no document.
#[derive(Clone, Debug, PartialEq)]
pub enum FeedError {
    /// Transport: offline, timeout, HTTP status, untrusted certificate and the like.
    Fetch(FetchError),
    /// A reply that is not what the service documents: one short line naming the problem, never body text.
    Parse(String),
    /// A setting that cannot be used, e.g. a zip code that is not 5 digits.
    Config(String),
    /// News: no feed gave a single item.
    Empty,
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedError::Fetch(e) => write!(f, "fetch failed: {e}"),
            FeedError::Parse(m) => write!(f, "unexpected reply: {m}"),
            FeedError::Config(m) => write!(f, "unusable setting: {m}"),
            FeedError::Empty => f.write_str("no feed gave an item"),
        }
    }
}

impl std::error::Error for FeedError {}

impl From<FetchError> for FeedError {
    fn from(e: FetchError) -> Self {
        FeedError::Fetch(e)
    }
}

impl FeedError {
    /// A short machine-readable name for status files and logs: `offline`, `timeout`, `too_large`, `http`, `untrusted`,
    /// `denied`, `fetch`, `parse`, `config` or `empty`.
    pub fn code(&self) -> &'static str {
        match self {
            FeedError::Fetch(e) => match e {
                FetchError::Offline => "offline",
                FetchError::Timeout => "timeout",
                FetchError::TooLarge => "too_large",
                FetchError::Http(_) => "http",
                FetchError::Untrusted(_) => "untrusted",
                FetchError::Denied(_) => "denied",
                FetchError::Failed(_) => "fetch",
            },
            FeedError::Parse(_) => "parse",
            FeedError::Config(_) => "config",
            FeedError::Empty => "empty",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_cover_every_variant() {
        let cases = [
            (FeedError::Fetch(FetchError::Offline), "offline"),
            (FeedError::Fetch(FetchError::Timeout), "timeout"),
            (FeedError::Fetch(FetchError::TooLarge), "too_large"),
            (FeedError::Fetch(FetchError::Http(404)), "http"),
            (FeedError::Fetch(FetchError::Untrusted("x".into())), "untrusted"),
            (FeedError::Fetch(FetchError::Denied("x".into())), "denied"),
            (FeedError::Fetch(FetchError::Failed("x".into())), "fetch"),
            (FeedError::Parse("x".into()), "parse"),
            (FeedError::Config("x".into()), "config"),
            (FeedError::Empty, "empty"),
        ];
        for (err, code) in cases {
            assert_eq!(err.code(), code);
            assert!(!err.to_string().is_empty());
        }
    }
}
