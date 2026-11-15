use anyhow::{Context, Result, bail, ensure};
use lumagame_core::{ids::TargetId, ipc::{Link, Reach, Target}};
use pf_client_core::trust::KnownHost;
use punktfunk_core::{discovery::DiscoveredHost, fp, quic::endpoint};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, os::unix::fs::{OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}};

#[derive(Clone, Serialize, Deserialize)]
pub struct Host {
    pub known: KnownHost,
    #[serde(default)]
    pub advertised_pin: String,
    #[serde(default)]
    pub fullname: String,
    #[serde(skip)]
    pub online: Option<bool>,
}

impl Host {
    pub fn id(&self) -> TargetId {
        TargetId::new(self.known.id.as_ref().expect("validated record id").clone()).expect("validated record id")
    }
    pub fn pin(&self) -> Result<[u8; 32]> {
        ensure!(self.known.paired, "Pair this PC before playing.");
        fp::parse_hex32(&self.known.fp_hex).context("The saved host fingerprint is invalid.")
    }
    pub fn target(&self) -> Target {
        Target {
            id: self.id(), name: self.known.name.clone(),
            detail: Some(punktfunk_core::discovery::join_host_port(&self.known.addr, self.known.port)),
            link: if self.known.paired { Link::Linked } else { Link::Found },
            reach: match self.online {
                Some(true) => Reach::Online,
                Some(false) if !self.known.mac.is_empty() => Reach::Asleep,
                Some(false) => Reach::Offline,
                None => Reach::Unknown,
            },
        }
    }
}

pub struct Store {
    dir: PathBuf,
    pub identity: (String, String),
    pub hosts: Vec<Host>,
}

pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("new");
    let result = (|| -> Result<()> {
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&tmp)?;
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        fs::File::open(path.parent().context("The store file has no parent directory")?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(tmp); }
    result
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        let identity_path = dir.join("identity.json");
        let identity: (String, String) = match fs::read(&identity_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("Read the saved client identity")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let identity = endpoint::generate_identity()?;
                private_write(&identity_path, &serde_json::to_vec(&identity)?)?;
                identity
            }
            Err(e) => return Err(e.into()),
        };
        endpoint::fingerprint_of_pem(&identity.0).context("Read the saved client certificate")?;
        ensure!(identity.1.contains("-----END "), "The saved private key is incomplete.");
        fs::set_permissions(identity_path, fs::Permissions::from_mode(0o600))?;
        let hosts: Vec<Host> = match fs::read(dir.join("hosts.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("Read the saved host pairings")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let mut ids = std::collections::BTreeSet::new();
        for host in &hosts {
            let id = TargetId::new(host.known.id.clone().context("Host record has no id")?)?;
            ensure!(ids.insert(id), "Duplicate host record id");
            if host.known.paired { host.pin()?; }
        }
        Ok(Self { dir: dir.into(), identity, hosts })
    }
    pub fn save(&self) -> Result<()> {
        private_write(&self.dir.join("hosts.json"), &serde_json::to_vec(&self.hosts)?)
    }
    pub fn host(&self, id: &TargetId) -> Result<&Host> {
        self.hosts.iter().find(|h| h.id() == *id).context("This PC is no longer saved.")
    }
    pub fn host_mut(&mut self, id: &TargetId) -> Result<&mut Host> {
        self.hosts.iter_mut().find(|h| h.id() == *id).context("This PC is no longer saved.")
    }
    pub fn manual(&mut self, address: &str) -> Result<TargetId> {
        let (addr, port) = parse_address(address)?;
        if let Some(host) = self.hosts.iter().find(|h| h.known.addr == addr && h.known.port == port) { return Ok(host.id()); }
        ensure!(self.hosts.len() < 128, "The host list is full.");
        let host = Host { known: KnownHost { name: addr.clone(), addr, port, ..Default::default() }, advertised_pin: String::new(), fullname: String::new(), online: None };
        let id = host.id();
        self.hosts.push(host);
        self.save()?;
        Ok(id)
    }
    pub fn discover(&mut self, d: DiscoveredHost) -> Result<()> {
        let advertised = fp::parse_hex32(&d.fp_hex).map(|p| fp::hex(&p)).unwrap_or_default();
        let at = self.hosts.iter().position(|h| {
            if h.known.paired {
                !advertised.is_empty() && h.known.fp_hex == advertised
            } else {
                h.known.addr == d.addr && h.known.port == d.port || !advertised.is_empty() && h.advertised_pin == advertised
            }
        });
        let h = match at {
            Some(at) => &mut self.hosts[at],
            None => {
                ensure!(self.hosts.len() < 128, "The host list is full.");
                self.hosts.push(Host { known: KnownHost::default(), advertised_pin: String::new(), fullname: String::new(), online: None });
                self.hosts.last_mut().expect("just inserted")
            }
        };
        if h.known.name.is_empty() || h.known.name == h.known.advert_name { h.known.name.clone_from(&d.name); }
        h.known.advert_name = d.name;
        h.known.move_to(&d.addr, d.port);
        h.known.mgmt_port = d.mgmt_port;
        h.known.mac = d.mac.into_iter().filter(|m| punktfunk_core::wol::parse_mac(m).is_some()).take(16).collect();
        h.advertised_pin = advertised;
        h.fullname = d.fullname;
        h.online = Some(true);
        self.save()
    }
    pub fn paired(&mut self, id: &TargetId, pin: [u8; 32]) -> Result<()> {
        let old = self.host(id)?.clone();
        let h = self.host_mut(id)?;
        h.known.fp_hex = fp::hex(&pin);
        h.known.paired = true;
        h.online = Some(true);
        if let Err(e) = self.save() { *self.host_mut(id)? = old; return Err(e); }
        Ok(())
    }
    pub fn forget(&mut self, id: &TargetId) -> Result<()> {
        let index = self.hosts.iter().position(|h|h.id() == *id).context("Unknown host")?;
        let old = self.hosts.remove(index);
        if let Err(e) = self.save() { self.hosts.insert(index,old); return Err(e); }
        Ok(())
    }
}

pub fn parse_address(raw: &str) -> Result<(String, u16)> {
    let raw = raw.trim();
    ensure!(!raw.is_empty() && raw.len() <= 253 && !raw.chars().any(char::is_whitespace), "Enter a PC address, optionally followed by :port.");
    if let Ok(socket) = raw.parse::<std::net::SocketAddr>() { ensure!(socket.port() != 0, "Port must not be zero."); return Ok((socket.ip().to_string(), socket.port())); }
    if let Ok(ip) = raw.parse::<std::net::IpAddr>() { return Ok((ip.to_string(), 9777)); }
    let (addr, port) = match raw.rsplit_once(':') {
        Some((a, p)) => (a, p.parse::<u16>().context("Invalid port")?),
        None => (raw, 9777),
    };
    if port == 0 || addr.is_empty() || !addr.bytes().all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)) { bail!("Invalid PC address."); }
    Ok((addr.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_and_pairings_are_private_and_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path()).unwrap();
        let id = s.manual("pc.local:9888").unwrap();
        assert!(s.host(&id).unwrap().pin().is_err());
        s.paired(&id, [42;32]).unwrap();
        let reopened = Store::open(dir.path()).unwrap();
        assert_eq!(reopened.identity, s.identity);
        assert_eq!(reopened.host(&id).unwrap().pin().unwrap(), [42;32]);
        for name in ["identity.json", "hosts.json"] { assert_eq!(fs::metadata(dir.path().join(name)).unwrap().permissions().mode() & 0o777, 0o600); }
        s.forget(&id).unwrap();
        assert!(Store::open(dir.path()).unwrap().hosts.is_empty());
    }
    #[test]
    fn manual_addresses_are_not_urls_or_commands() {
        assert_eq!(parse_address("[::1]:9778").unwrap(), ("::1".into(),9778));
        assert_eq!(parse_address("gaming-pc.local").unwrap(), ("gaming-pc.local".into(),9777));
        for bad in ["", "https://host", "host:0", "host/path", "a b", "a:99999"] { assert!(parse_address(bad).is_err()); }
    }
    #[test]
    fn discovery_never_replaces_a_saved_pin_and_address_changes_keep_ids() {
        let dir=tempfile::tempdir().unwrap();let mut s=Store::open(dir.path()).unwrap();
        let id=s.manual("192.0.2.1").unwrap();s.paired(&id,[42;32]).unwrap();
        let mut advert=DiscoveredHost {key:"pc".into(),fullname:"pc._punktfunk._udp.local.".into(),name:"PC".into(),addr:"192.0.2.1".into(),port:9777,
            fp_hex:fp::hex(&[43;32]),pair:"required".into(),mgmt_port:Some(47990),mac:vec!["00:11:22:33:44:55".into()],os:"linux".into(),wire:vec![2]};
        s.discover(advert.clone()).unwrap();
        assert_eq!(s.host(&id).unwrap().pin().unwrap(),[42;32]);assert_eq!(s.hosts.len(),2);
        assert!(s.hosts.iter().find(|h|h.id()!=id).unwrap().pin().is_err());
        advert.fp_hex=fp::hex(&[42;32]);advert.addr="192.0.2.2".into();s.discover(advert).unwrap();
        assert_eq!(s.host(&id).unwrap().known.addr,"192.0.2.2");assert_eq!(s.hosts.len(),2);
    }
}
