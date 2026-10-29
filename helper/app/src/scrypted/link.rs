//! `lumatv-helper scrypted-link` and `scrypted-unlink`: how the owner of the TV links it to the LumaShow bridge in Scrypted.
//!
//! ```text
//! lumatv-helper scrypted-link --brand ... --address 192.168.1.69:10444 --pin <the bridge's fingerprint> --new [--device LumaTV]
//! lumatv-helper scrypted-link --brand ... --check
//! lumatv-helper scrypted-unlink --brand ...
//! ```
//!
//! `--new` makes the TV's own secret (32 random bytes, kept in `<data dir>/scrypted/link.json`, mode 0600) and prints **only its hash**, the
//! verifier, and the device's name: the owner adds a device in the bridge's settings with that name and that verifier. The secret never
//! leaves the TV, never appears in an argument, in any output or in the log; what crosses between the two screens (a fingerprint and a hash)
//! is no secret (Principle 0: nothing but the owner's own command can write this file, and a link is a key). Run again with other flags the
//! command changes only what is given and keeps the token (the bridge's certificate was remade: `--pin` again); `--new` makes a new token and
//! prints its verifier. `--check` connects with what is saved and says `connected: N cameras` or why not, in words, and nothing else. The
//! helper that is running looks at the file every few seconds, so it is not told.

use super::bridge::{Bridge, Trouble};
use super::http::Client;
use super::store::{self, Link, Pin, Token};
use super::tls::Dialer;
use crate::config::Paths;
use ha_link::AddressPolicy;
use serde_json::{Value, json};

/// What the command line gave.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub address: Option<String>,
    pub pin: Option<String>,
    pub device: Option<String>,
    pub new: bool,
    pub check: bool,
}

/// What a command comes to: the exit code and the one object it prints.
pub type Outcome = (i32, Value);

fn fail(code: i32, message: impl Into<String>) -> Outcome {
    (code, json!({"ok": false, "error": message.into()}))
}

/// The words for a failed `--check`.
fn words(trouble: &Trouble) -> String {
    if trouble.untrusted() {
        "The bridge's certificate is not the pinned one: check the fingerprint in the bridge's settings, or run scrypted-link --pin again if the bridge made a new certificate".into()
    } else if trouble.refused() {
        "Scrypted refused this TV: add the device in the bridge's settings with the verifier that scrypted-link --new printed".into()
    } else if trouble.unreachable() {
        "Scrypted is not answering at that address: is the bridge installed, and its port open on the home network?".into()
    } else {
        trouble.words()
    }
}

/// Runs `scrypted-link`.
pub fn link(paths: &Paths, args: &Args, policy: AddressPolicy) -> Outcome {
    let file = paths.scrypted_link_file();
    let saved = match store::load(&file, policy) {
        Ok(saved) => saved,
        Err(e) => return fail(2, e.to_string()),
    };
    let changing =
        args.address.is_some() || args.pin.is_some() || args.device.is_some() || args.new;
    if !changing && !args.check {
        return fail(
            2,
            "give --address and --pin with --new to link, or --check to look at the link",
        );
    }
    if changing {
        match save(&file, saved, args, policy) {
            Ok(outcome) => {
                if !args.check {
                    return outcome;
                }
            }
            Err(outcome) => return outcome,
        }
    }
    check(&file, policy)
}

/// Makes or changes the link; the outcome to print when nothing else is asked.
fn save(
    file: &std::path::Path,
    saved: Option<Link>,
    args: &Args,
    policy: AddressPolicy,
) -> Result<Outcome, Outcome> {
    let address = args
        .address
        .as_deref()
        .map(|a| store::parse_address(a, policy))
        .transpose()
        .map_err(|e| fail(2, e.to_string()))?;
    let pin = args
        .pin
        .as_deref()
        .map(Pin::parse)
        .transpose()
        .map_err(|e| fail(2, e.to_string()))?;
    let device = args
        .device
        .as_deref()
        .map(store::check_device)
        .transpose()
        .map_err(|e| fail(2, e.to_string()))?;
    let (mut link, fresh) = match (saved, args.new) {
        (None, false) => {
            return Err(fail(
                2,
                "there is no link yet: give --address, --pin and --new",
            ));
        }
        (None, true) => {
            let (Some(address), Some(pin)) = (address, pin) else {
                return Err(fail(
                    2,
                    "a new link needs --address and --pin (the bridge's fingerprint, as its settings show it)",
                ));
            };
            let token = Token::make().map_err(|e| fail(1, e.to_string()))?;
            (
                Link {
                    address,
                    pin,
                    token,
                    device: device
                        .clone()
                        .unwrap_or_else(|| store::DEFAULT_DEVICE.to_owned()),
                },
                true,
            )
        }
        (Some(mut link), new) => {
            if let Some(address) = address {
                link.address = address;
            }
            if let Some(pin) = pin {
                link.pin = pin;
            }
            if let Some(device) = &device {
                link.device.clone_from(device);
            }
            if new {
                link.token = Token::make().map_err(|e| fail(1, e.to_string()))?;
            }
            (link, new)
        }
    };
    if let Err(e) = store::save(file, &link, policy) {
        return Err(fail(1, e.to_string()));
    }
    // what is printed is a hash and a name: the token is out of reach from here on
    let verifier = fresh.then(|| link.token.verifier());
    link.device = link.device.trim().to_owned();
    let mut out = json!({"ok": true, "linked": true, "device": link.device});
    if let Some(verifier) = verifier {
        out["verifier"] = json!(verifier);
        out["note"] = json!(
            "In the bridge's settings, Devices, Add a device: this name and this verifier. The verifier is a hash, not a secret."
        );
    } else {
        out["note"] = json!("Saved; the token is the one it was.");
    }
    Ok((0, out))
}

/// `--check`: connects with what is saved and says what came of it.
fn check(file: &std::path::Path, policy: AddressPolicy) -> Outcome {
    let link = match store::load(file, policy) {
        Ok(Some(link)) => link,
        Ok(None) => {
            return fail(
                1,
                "there is no link: run scrypted-link --address ... --pin ... --new",
            );
        }
        Err(e) => return fail(2, e.to_string()),
    };
    let client = Client::new(Dialer::new(link.address, link.pin), link.token.clone());
    let bridge = Bridge::new(client);
    let health = match bridge.health() {
        Ok(health) => health,
        Err(trouble) => return fail(1, words(&trouble)),
    };
    let cameras = match bridge.cameras() {
        Ok(cameras) => cameras,
        Err(trouble) => return fail(1, words(&trouble)),
    };
    let mut out = json!({"ok": true, "connected": true, "cameras": cameras.len(), "message": format!("connected: {} cameras", cameras.len())});
    if !(health.has("live") && health.has("stream")) {
        out["note"] = json!(
            "The bridge is older than 1.1: only the timeline works; live video, the grid and pop-ups need an update of the bridge in Scrypted."
        );
    }
    (0, out)
}

/// Runs `scrypted-unlink`: the file goes; the helper that is running closes everything within a few seconds.
pub fn unlink(paths: &Paths) -> Outcome {
    let had = store::clear(&paths.scrypted_link_file());
    (0, json!({"ok": true, "unlinked": true, "was": had}))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN: &str = "10:59:6F:3D:C9:A2:1A:30:1A:1C:C0:B8:A4:C5:5A:0B:B1:3B:44:10:A8:C6:F6:66:5B:F0:FD:41:73:3E:F7:6F";

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        (dir, paths)
    }

    fn args(address: Option<&str>, pin: Option<&str>, new: bool) -> Args {
        Args {
            address: address.map(str::to_owned),
            pin: pin.map(str::to_owned),
            new,
            ..Args::default()
        }
    }

    #[test]
    fn a_new_link_saves_a_token_and_prints_only_its_hash_and_the_device_name() {
        let (_dir, paths) = paths();
        let (code, out) = link(
            &paths,
            &args(Some("192.168.1.69:10444"), Some(PIN), true),
            AddressPolicy::HOME,
        );
        assert_eq!(code, 0, "{out}");
        let saved = store::load(&paths.scrypted_link_file(), AddressPolicy::HOME)
            .unwrap()
            .unwrap();
        assert_eq!(out["verifier"], saved.token.verifier());
        assert_eq!(out["device"], "LumaTV");
        let text = out.to_string();
        assert!(
            !text.contains(saved.token.expose()) && !text.contains(&saved.token.expose()[4..]),
            "{text}"
        );
        assert!(
            !text.contains("192.168"),
            "no address in what is printed: {text}"
        );
        assert_eq!(saved.address.port(), 10444);
        assert_eq!(saved.pin.hex(), PIN.replace(':', "").to_ascii_lowercase());
    }

    #[test]
    fn running_again_changes_only_what_is_given_and_new_makes_a_new_token() {
        let (_dir, paths) = paths();
        link(
            &paths,
            &args(Some("192.168.1.69"), Some(PIN), true),
            AddressPolicy::HOME,
        );
        let first = store::load(&paths.scrypted_link_file(), AddressPolicy::HOME)
            .unwrap()
            .unwrap();
        // the certificate was remade: the same token, a new pin
        let other = PIN.replace("10:59", "AA:BB");
        let (code, out) = link(
            &paths,
            &args(None, Some(&other), false),
            AddressPolicy::HOME,
        );
        assert_eq!(code, 0, "{out}");
        assert!(
            out.get("verifier").is_none(),
            "a token that was not made is not printed"
        );
        let second = store::load(&paths.scrypted_link_file(), AddressPolicy::HOME)
            .unwrap()
            .unwrap();
        assert_eq!(second.token, first.token);
        assert_ne!(second.pin, first.pin);
        let (code, out) = link(
            &paths,
            &Args {
                new: true,
                device: Some("Living room".into()),
                ..Args::default()
            },
            AddressPolicy::HOME,
        );
        assert_eq!(code, 0, "{out}");
        let third = store::load(&paths.scrypted_link_file(), AddressPolicy::HOME)
            .unwrap()
            .unwrap();
        assert_ne!(third.token, first.token);
        assert_eq!(out["verifier"], third.token.verifier());
        assert_eq!(
            (third.device.as_str(), third.pin),
            ("Living room", second.pin)
        );
    }

    #[test]
    fn a_bad_link_is_refused_in_words_and_saves_nothing() {
        let (_dir, paths) = paths();
        for (a, p, n) in [
            (Some("8.8.8.8:10444"), Some(PIN), true),
            (Some("cctv.nateshome.net"), Some(PIN), true),
            (Some("192.168.1.69"), Some("not a pin"), true),
            (Some("192.168.1.69"), None, true),
            (None, Some(PIN), true),
            (Some("192.168.1.69"), Some(PIN), false),
        ] {
            let (code, out) = link(&paths, &args(a, p, n), AddressPolicy::HOME);
            assert_eq!(code, 2, "{a:?} {p:?} {n}: {out}");
            assert_eq!(out["ok"], false);
        }
        assert!(!paths.scrypted_link_file().exists());
        let (code, _) = link(&paths, &Args::default(), AddressPolicy::HOME);
        assert_eq!(code, 2, "nothing asked is a usage error");
        let (code, out) = link(
            &paths,
            &Args {
                check: true,
                ..Args::default()
            },
            AddressPolicy::HOME,
        );
        assert_eq!(code, 1, "{out}");
    }

    #[test]
    fn unlink_removes_the_file_and_says_whether_there_was_one() {
        let (_dir, paths) = paths();
        assert_eq!(unlink(&paths).1["was"], false);
        link(
            &paths,
            &args(Some("192.168.1.69"), Some(PIN), true),
            AddressPolicy::HOME,
        );
        assert!(paths.scrypted_link_file().exists());
        assert_eq!(unlink(&paths).1["was"], true);
        assert!(!paths.scrypted_link_file().exists());
    }
}
