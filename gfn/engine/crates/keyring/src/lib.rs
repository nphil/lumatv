//! The TV has no Secret Service and no D-Bus session for one, and the real `keyring` crate would link libdbus for the target. This
//! crate answers every call with "no storage access": opennow-core then takes the `local-file` path its protocol documents
//! (`fallback-sessions/<sha256 user>.json`, mode 0600, under its data directory), which is the store the app relies on. Only the
//! parts of the crate's API that opennow-core uses exist.
use std::fmt;

#[derive(Debug)]
pub enum Error {
    NoEntry,
    NoStorageAccess(Box<dyn std::error::Error + Send + Sync>),
    PlatformFailure(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoEntry => write!(f, "no entry"),
            Error::NoStorageAccess(e) => write!(f, "no storage access: {e}"),
            Error::PlatformFailure(e) => write!(f, "platform failure: {e}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Entry {
    service: String,
    user: String,
}

fn unavailable() -> Error {
    Error::NoStorageAccess("this device has no OS credential store".into())
}

impl Entry {
    pub fn new(service: &str, user: &str) -> Result<Entry> {
        Ok(Entry { service: service.to_owned(), user: user.to_owned() })
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    pub fn set_password(&self, _password: &str) -> Result<()> {
        Err(unavailable())
    }

    pub fn get_password(&self) -> Result<String> {
        Err(unavailable())
    }

    pub fn delete_credential(&self) -> Result<()> {
        Err(unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_call_reports_no_storage() {
        let e = Entry::new("app.opennow.auth", "user").unwrap();
        assert!(matches!(e.get_password(), Err(Error::NoStorageAccess(_))));
        assert!(matches!(e.set_password("x"), Err(Error::NoStorageAccess(_))));
        assert!(matches!(e.delete_credential(), Err(Error::NoStorageAccess(_))));
        assert_eq!((e.service(), e.user()), ("app.opennow.auth", "user"));
    }
}
