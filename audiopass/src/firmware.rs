//! The firmware allowlist. The route is proven on one build of LG's software and trusts nothing else: it reads LG's files, never
//! changes them, and refuses on any difference.

use crate::host::Host;

const RELEASE_PATH: &str = "/etc/starfish-release";
const RELEASE: &str = "Rockhopper release 10.2.1-5202 (ponytail-paparoa)";
const KERNEL_PATH: &str = "/proc/sys/kernel/osrelease";
const KERNEL: &str = "5.4.268-329.ptl4tv.2";
/// The sink and the platform library that name the converter command's layout and the audio route's controls.
const FILES: [(&str, &str); 2] = [
    (
        "/usr/lib/gstreamer-1.0/libgstlgaudiosink.so",
        "61c269de790c0872b96436c555f7807ef2357459b9969262b1b775939cacc9d2",
    ),
    (
        "/usr/lib/libpf-1.0.so.1",
        "e8a966b26476f647bfcebf02b08404fc52f0198c52195e37354baa3d712fc36f",
    ),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Firmware {
    /// The first line of `/etc/starfish-release`, whatever it is.
    pub release: Option<String>,
    pub kernel: Option<String>,
    /// `Ok` when every version and file matches, otherwise the first difference in words.
    pub verdict: Result<(), String>,
    /// The verdict is an error because a version or a file could not be read, not because it differs. That says nothing about the
    /// build, so the route looks again instead of refusing.
    pub unread: bool,
}

fn short(text: &str) -> &str {
    text.get(..8).unwrap_or(text)
}

fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Why a firmware is not accepted.
enum Refused {
    /// A version or a file is not the proven one.
    Differs(String),
    /// A version or a file could not be read.
    Unreadable(String),
}

pub fn check(host: &dyn Host) -> Firmware {
    let release = host
        .read(RELEASE_PATH)
        .ok()
        .and_then(|text| text.lines().next().map(|line| line.trim().to_owned()));
    let kernel = host
        .read(KERNEL_PATH)
        .ok()
        .map(|text| text.trim().to_owned());
    let (verdict, unread) = match verdict(host, release.as_deref(), kernel.as_deref()) {
        Ok(()) => (Ok(()), false),
        Err(Refused::Differs(why)) => (Err(why), false),
        Err(Refused::Unreadable(why)) => (Err(why), true),
    };
    Firmware {
        release,
        kernel,
        verdict,
        unread,
    }
}

fn verdict(host: &dyn Host, release: Option<&str>, kernel: Option<&str>) -> Result<(), Refused> {
    match release {
        Some(RELEASE) => {}
        Some(other) => {
            return Err(Refused::Differs(format!(
                "TV software is \"{other}\"; the route is proven only on \"{RELEASE}\""
            )));
        }
        None => {
            return Err(Refused::Unreadable(format!("cannot read {RELEASE_PATH}")));
        }
    }
    match kernel {
        Some(KERNEL) => {}
        Some(other) => {
            return Err(Refused::Differs(format!(
                "kernel is {other}; the route is proven only on {KERNEL}"
            )));
        }
        None => {
            return Err(Refused::Unreadable(format!("cannot read {KERNEL_PATH}")));
        }
    }
    for (path, want) in FILES {
        match host.sha256(path) {
            Ok(found) if found == want => {}
            Ok(found) => {
                return Err(Refused::Differs(format!(
                    "{} is not the build this route is proven on (sha256 {}…, expected {}…)",
                    name(path),
                    short(&found),
                    short(want)
                )));
            }
            Err(e) => return Err(Refused::Unreadable(format!("cannot read {path}: {e}"))),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHost;

    #[test]
    fn the_proven_build_matches() {
        let host = FakeHost::c4();
        let fw = check(&host);
        assert_eq!(fw.verdict, Ok(()));
        assert!(!fw.unread);
        assert_eq!(fw.release.as_deref(), Some(RELEASE));
        assert_eq!(fw.kernel.as_deref(), Some(KERNEL));
    }

    #[test]
    fn each_difference_refuses_and_says_which() {
        for (what, expect) in [
            ("release", "TV software is"),
            ("kernel", "kernel is"),
            ("sink", "libgstlgaudiosink.so is not the build"),
            ("pf", "libpf-1.0.so.1 is not the build"),
        ] {
            let host = FakeHost::c4();
            host.with(|sim| match what {
                "release" => {
                    sim.files.insert(
                        RELEASE_PATH.into(),
                        "Rockhopper release 10.2.2-5300 (x)\n".into(),
                    );
                }
                "kernel" => {
                    sim.files.insert(KERNEL_PATH.into(), "5.4.300\n".into());
                }
                "sink" => {
                    sim.hashes.insert(FILES[0].0.into(), "0".repeat(64));
                }
                _ => {
                    sim.hashes.insert(FILES[1].0.into(), "f".repeat(64));
                }
            });
            let fw = check(&host);
            let reason = fw.verdict.clone().unwrap_err();
            assert!(reason.contains(expect), "{what}: {reason}");
            assert!(!fw.unread, "{what} differs, it is not unreadable");
        }
    }

    #[test]
    fn an_identity_that_cannot_be_read_is_not_accepted_and_is_marked_unread() {
        let host = FakeHost::c4();
        host.with(|sim| {
            sim.files.remove(RELEASE_PATH);
        });
        let fw = check(&host);
        assert!(
            fw.verdict
                .clone()
                .unwrap_err()
                .contains("cannot read /etc/starfish-release")
        );
        assert!(fw.unread);
        let host = FakeHost::c4();
        host.with(|sim| {
            sim.hashes.remove(FILES[0].0);
        });
        let fw = check(&host);
        assert!(
            fw.verdict
                .clone()
                .unwrap_err()
                .contains("cannot read /usr/lib/gstreamer-1.0/libgstlgaudiosink.so")
        );
        assert!(fw.unread);
    }
}
