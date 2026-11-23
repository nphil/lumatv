//! The account's profile picture: one small image in the engine's cache directory, named after its address so that a changed picture
//! is a new file for the app, and replaced as a whole so that the directory never holds more than one.

use crate::{Adapter, Clock, files, http::{self, Http}};
use std::fs;
use std::path::{Path, PathBuf};

/// The largest picture kept. Steam's 64 px "medium" picture is a few kilobytes.
const MAX_BYTES: usize = 64 << 10;
const PREFIX: &str = "avatar-";

/// The address of the picture in a community profile's XML (`<avatarMedium>`), when it is an https address on Steam's image hosts and
/// names a JPEG or PNG.
pub fn avatar_url(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let rest = &text[text.find("<avatarMedium>")? + "<avatarMedium>".len()..];
    let inner = rest[..rest.find("</avatarMedium>")?].trim();
    let url = inner.strip_prefix("<![CDATA[").and_then(|cdata| cdata.strip_suffix("]]>")).unwrap_or(inner).trim();
    let host = url.strip_prefix("https://")?.split('/').next()?;
    let steam_host = host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && (host.ends_with(".steamstatic.com") || host.ends_with(".akamaihd.net"));
    (steam_host && extension(url).is_some()).then(|| url.to_owned())
}

fn extension(url: &str) -> Option<&'static str> {
    match url.rsplit('.').next()? {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        _ => None,
    }
}

fn is_image(bytes: &[u8], extension: &str) -> bool {
    match extension {
        "jpg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        _ => bytes.starts_with(&[0x89, b'P', b'N', b'G']),
    }
}

fn file_name(url: &str, extension: &str) -> String {
    let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3));
    format!("{PREFIX}{hash:016x}.{extension}")
}

/// Deletes every picture in `dir` but `keep`.
fn remove_others(dir: &Path, keep: Option<&str>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_str().is_some_and(|name| name.starts_with(PREFIX) && Some(name) != keep) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The picture a restart finds in `dir`.
pub fn saved(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_str().is_some_and(|name| name.starts_with(PREFIX))).map(|e| e.path())
}

impl<H: Http, C: Clock> Adapter<H, C> {
    /// The account's community profile page. Best effort: any trouble is no page.
    pub(crate) fn profile_page(&mut self, steamid: &str) -> Option<Vec<u8>> {
        let url = format!("{}/profiles/{steamid}/?xml=1", http::COMMUNITY);
        self.http.get(&url, http::PERSONA_BODY).ok().filter(|resp| resp.status == 200).map(|resp| resp.body)
    }

    /// Keeps the picture the profile `page` names, unless that picture is already the one kept. Trouble leaves what was kept as it was.
    pub(crate) fn keep_avatar(&mut self, page: &[u8]) {
        let Some(url) = avatar_url(page) else { return };
        let Some(extension) = extension(&url) else { return };
        let name = file_name(&url, extension);
        if self.avatar.as_ref().is_some_and(|kept| kept.file_name().is_some_and(|kept| kept == name.as_str())) {
            return;
        }
        let Some(resp) = self.http.get(&url, MAX_BYTES).ok().filter(|resp| resp.status == 200 && is_image(&resp.body, extension)) else {
            return self.note("avatar: the picture could not be fetched");
        };
        if let Err(error) = files::write(&self.cache, &name, &resp.body) {
            return self.note(&format!("avatar: the picture could not be saved ({:?})", error.kind()));
        }
        remove_others(&self.cache, Some(&name));
        self.avatar = Some(self.cache.join(name));
        self.targets_changed();
    }

    /// The signed-in account's picture is looked up again: once after the engine starts, and whenever the owner asks for a refresh.
    pub(crate) fn refresh_avatar(&mut self) {
        let Some(account) = self.account.as_ref().filter(|account| !account.expired) else { return };
        let steamid = account.steamid.clone();
        if let Some(page) = self.profile_page(&steamid) {
            self.keep_avatar(&page);
        }
    }

    /// Signing out takes the picture with it.
    pub(crate) fn forget_avatar(&mut self) {
        self.avatar = None;
        remove_others(&self.cache, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{self, Rig};
    use lumagame_core::ipc::{Event, Request};

    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];

    fn page(url: &str) -> String {
        format!("<profile><steamID><![CDATA[Rig]]></steamID><avatarMedium><![CDATA[{url}]]></avatarMedium></profile>")
    }

    fn picture(bytes: &[u8]) -> Result<http::Resp, http::HttpError> {
        Ok(http::Resp { status: 200, eresult: None, body: bytes.to_vec() })
    }

    fn pictures(dir: &Path) -> Vec<String> {
        fs::read_dir(dir).unwrap().flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|name| name.starts_with(PREFIX)).collect()
    }

    fn avatar_of(events: &[lumagame_core::ipc::FromEngine]) -> Option<String> {
        events.iter().find_map(|e| match &e.event {
            Event::Targets { targets } => Some(targets[0].avatar.clone()),
            _ => None,
        })?
    }

    #[test]
    fn only_a_small_picture_from_steams_hosts_is_taken() {
        let url = |u: &str| avatar_url(page(u).as_bytes());
        assert!(url("https://avatars.fastly.steamstatic.com/abc_medium.jpg").is_some());
        assert!(url("https://steamcdn-a.akamaihd.net/steamcommunity/public/images/avatars/ab/abc_medium.png").is_some());
        assert_eq!(url("http://avatars.steamstatic.com/abc_medium.jpg"), None);
        assert_eq!(url("https://evil.example/steamstatic.com/abc.jpg"), None);
        assert_eq!(url("https://avatars.steamstatic.com@evil.example/abc.jpg"), None);
        assert_eq!(url("https://avatars.steamstatic.com/abc.gif"), None);
    }

    #[test]
    fn the_cache_holds_one_picture_that_follows_the_profile_and_goes_with_the_account() {
        let mut rig = Rig::signed_in();
        let (first, second) = ("https://avatars.steamstatic.com/one_medium.jpg", "https://avatars.steamstatic.com/two_medium.jpg");
        rig.http.on("xml=1", [fake::ok(&page(first))]);
        rig.http.on("one_medium", [picture(JPEG)]);
        rig.http.on("two_medium", [picture(b"<html>not a picture</html>")]);
        rig.adapter.refresh_avatar();
        let kept = rig.adapter.avatar.clone().expect("the picture is kept");
        assert_eq!(fs::read(&kept).unwrap(), JPEG);
        assert_eq!(avatar_of(&rig.send(Request::Targets)), Some(kept.to_string_lossy().into_owned()));

        rig.http.forget();
        rig.adapter.refresh_avatar();
        assert_eq!(rig.http.count("one_medium"), 0, "the same picture is not fetched again");

        rig.http.on("xml=1", [fake::ok(&page(second))]);
        rig.adapter.refresh_avatar();
        assert_eq!(rig.adapter.avatar.as_ref(), Some(&kept), "a reply that is no picture changes nothing");
        rig.http.on("two_medium", [picture(&[0xff, 0xd8, 0xff, 9])]);
        rig.adapter.refresh_avatar();
        assert_eq!(pictures(&rig.cache()).len(), 1, "the new picture replaced the old");
        assert_ne!(rig.adapter.avatar.as_ref(), Some(&kept));

        rig.restart();
        rig.hello();
        assert!(rig.adapter.avatar.is_some(), "a restart finds the picture again");
        let target = rig.adapter.account.as_ref().unwrap().target_id();
        rig.send(Request::Forget { target });
        assert!(pictures(&rig.cache()).is_empty());
    }
}
