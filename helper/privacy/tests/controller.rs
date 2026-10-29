use lumatv_privacy::{Paths, Platform, Privacy, atomic, hosts};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<Value>>,
    settings: Mutex<Value>,
    fail: Mutex<bool>,
    underlay: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
    dc: Mutex<Value>,
}
impl Platform for Fake {
    fn luna(&self, method: &str, body: &Value) -> Result<Value, String> {
        self.calls.lock().push(json!({"method":method,"body":body}));
        if method == "setSystemSettings" {
            if *self.fail.lock() {
                return Err("LG refused the change".into());
            }
            *self.settings.lock() = body["settings"].clone();
        }
        Ok(json!({"returnValue":true,"settings":*self.settings.lock()}))
    }
    fn declutter(&self, args: &[&str]) -> Result<Value, String> {
        self.calls.lock().push(json!({"declutter":args}));
        if *self.fail.lock() {
            return Err("declutter failed".into());
        }
        Ok(self.dc.lock().clone())
    }
    fn bind(&self, source: &Path, target: &Path) -> Result<(), String> {
        self.calls.lock().push(json!({"bind":[source,target]}));
        self.underlay.lock().insert(
            target.to_owned(),
            std::fs::read(target).map_err(|e| e.to_string())?,
        );
        std::fs::remove_file(target).map_err(|e| e.to_string())?;
        std::fs::hard_link(source, target).map_err(|e| e.to_string())
    }
    fn unmount(&self, target: &Path) -> Result<(), String> {
        self.calls.lock().push(json!({"unmount":target}));
        let bytes = self
            .underlay
            .lock()
            .remove(target)
            .ok_or("not a fake mount")?;
        std::fs::remove_file(target).map_err(|e| e.to_string())?;
        std::fs::write(target, bytes).map_err(|e| e.to_string())
    }
    fn cert_hash(&self, _: &Path) -> Result<String, String> {
        Ok("01234567".into())
    }
}
fn status_json(ads_on: bool, early: Value) -> Value {
    json!({"ok":true,"groups":{"ads":{"on":ads_on,"inTarget":"6/6"},"settings":{"on":true,"inTarget":"16/16"},"uploaders":{"on":true,"inTarget":"7/7"},"push":{"on":false,"inTarget":"0/5"}},"early":early})
}
fn rig() -> (tempfile::TempDir, Paths, Arc<Fake>, Privacy) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        root: dir.path().join("root"),
        data: dir.path().join("state"),
    };
    atomic(
        &paths.system("/etc/hosts"),
        b"127.0.0.1 localhost\n0.0.0.0 su.lge.com # HBC\n0.0.0.0 foreign.example\n",
        false,
    )
    .unwrap();
    atomic(&paths.system("/var/palm/license/eulaInfoNetwork.json"),br#"{"eulaMappingList":{"eulaInfo":[{"settingKey":"acrAllowed","mandatory":["A"]},{"settingKey":"customAdAllowed","mandatory":["A"]}]}}"#,false).unwrap();
    let platform = Arc::new(Fake {
        settings: Mutex::new(
            json!({"eulaStatus":{"acrAllowed":true,"customAdAllowed":true,"voiceAllowed":true,"networkAllowed":true},"eulaInfoNetwork":{"eulaList":[{"id":"A","accepted":true,"title":"Analytics"}]}}),
        ),
        dc: Mutex::new(status_json(
            false,
            json!({"hook":false,"pending":false,"last":null}),
        )),
        ..Fake::default()
    });
    let privacy = Privacy::new(paths.clone(), platform.clone());
    (dir, paths, platform, privacy)
}
#[test]
fn read_is_nonintrusive_and_menu_has_every_item_and_existing_links() {
    let (_d, paths, p, privacy) = rig();
    let before = std::fs::read(paths.system("/etc/hosts")).unwrap();
    let status = privacy.status(false);
    let items = status["items"].as_array().unwrap();
    for id in [
        "ads",
        "settings",
        "uploaders",
        "push",
        "hosts",
        "amazoff",
        "consent.acrAllowed",
        "permissions",
        "deletedata",
    ] {
        assert!(items.iter().any(|r| r["id"] == id));
    }
    assert_eq!(status["canAdmin"], false);
    assert_eq!(items[1]["settings"].as_array().unwrap().len(), 16);
    assert_eq!(before, std::fs::read(paths.system("/etc/hosts")).unwrap());
    assert!(
        p.calls
            .lock()
            .iter()
            .all(|v| v["method"] == "getSystemSettings" || v["declutter"] == json!(["status"]))
    );
    assert!(!paths.state("prime-identity.json").exists());
}
#[test]
fn confirmation_is_exact_single_use_and_declutter_is_reused() {
    let (_d, _paths, p, privacy) = rig();
    let token = privacy.prepare("uploaders", true).unwrap()["confirmation"]
        .as_str()
        .unwrap()
        .to_owned();
    privacy.apply("uploaders", true, &token).unwrap();
    assert!(privacy.apply("uploaders", true, &token).is_err());
    assert_eq!(
        *p.calls.lock(),
        vec![
            json!({"declutter":["on","--group","uploaders"]}),
            json!({"declutter":["persist","on"]})
        ]
    );
    let token = privacy.prepare("ads", true).unwrap()["confirmation"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(privacy.apply("ads", false, &token).is_err());
    assert_eq!(p.calls.lock().len(), 2);
    assert!(privacy.prepare("apps", true).is_err());
    assert!(privacy.prepare("voice", true).is_err());
}
#[test]
fn consent_writes_both_documents_and_flags_and_rejects_changed_agreements() {
    let (_d, _paths, p, privacy) = rig();
    let token = privacy.prepare("consent.acrAllowed", false).unwrap()["confirmation"]
        .as_str()
        .unwrap()
        .to_owned();
    privacy.apply("consent.acrAllowed", false, &token).unwrap();
    assert_eq!(p.settings.lock()["eulaStatus"]["acrAllowed"], false);
    assert_eq!(p.settings.lock()["eulaStatus"]["customAdAllowed"], false);
    assert_eq!(p.settings.lock()["eulaStatus"]["voiceAllowed"], true);
    let token = privacy.prepare("consent.acrAllowed", true).unwrap()["confirmation"]
        .as_str()
        .unwrap()
        .to_owned();
    p.settings.lock()["eulaInfoNetwork"]["eulaList"][0]["title"] = json!("A changed agreement");
    assert!(
        privacy
            .apply("consent.acrAllowed", true, &token)
            .unwrap_err()
            .contains("agreements changed")
    );
    assert!(privacy.prepare("consent.networkAllowed", false).is_err());
}
#[test]
fn hosts_on_off_preserves_other_owners_and_boot_resumes_only_saved_choices() {
    let (_d, paths, p, privacy) = rig();
    let original = std::fs::read(paths.system("/etc/hosts")).unwrap();
    privacy.resume();
    assert!(p.calls.lock().is_empty());
    let token = privacy.prepare("hosts", true).unwrap()["confirmation"]
        .as_str()
        .unwrap()
        .to_owned();
    privacy.apply("hosts", true, &token).unwrap();
    assert!(paths.state("hosts.enabled").exists());
    assert!(
        std::fs::read_to_string(paths.system("/etc/hosts"))
            .unwrap()
            .contains(hosts::BEGIN)
    );
    std::fs::remove_file(paths.system("/etc/hosts")).unwrap();
    std::fs::write(paths.system("/etc/hosts"), &original).unwrap();
    privacy.maintain();
    assert!(
        std::fs::read_to_string(paths.system("/etc/hosts"))
            .unwrap()
            .contains(hosts::BEGIN)
    );
    let token = privacy.prepare("hosts", false).unwrap()["confirmation"]
        .as_str()
        .unwrap()
        .to_owned();
    privacy.apply("hosts", false, &token).unwrap();
    assert!(!paths.state("hosts.enabled").exists());
    assert_eq!(std::fs::read(paths.system("/etc/hosts")).unwrap(), original);
}
#[test]
fn platform_failure_is_returned_not_reported_as_success() {
    let (_d, _paths, p, privacy) = rig();
    let token = privacy.prepare("uploaders", true).unwrap()["confirmation"]
        .as_str()
        .unwrap()
        .to_owned();
    *p.fail.lock() = true;
    assert_eq!(
        privacy.apply("uploaders", true, &token).unwrap_err(),
        "declutter failed"
    );
}
#[test]
fn old_tls_bypass_refuses_activation_without_touching_prime_or_platform() {
    let (_d, paths, p, privacy) = rig();
    std::fs::create_dir_all(paths.system(lumatv_privacy::prime::CERT_DIR)).unwrap();
    atomic(
        &paths.system(lumatv_privacy::prime::JAIL_HOSTS),
        b"127.0.0.1 localhost\n",
        false,
    )
    .unwrap();
    atomic(
        &paths.system("/media/cryptofs/apps/usr/palm/applications/amazon/default_config.json"),
        br#"{"disable-ssl-cert":"1"}"#,
        false,
    )
    .unwrap();
    let token = privacy.prepare("amazoff", true).unwrap()["confirmation"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        privacy
            .apply("amazoff", true, &token)
            .unwrap_err()
            .contains("TLS checks")
    );
    assert!(p.calls.lock().is_empty());
    assert!(!paths.state("prime.enabled").exists());
    assert!(!paths.state("prime-identity.json").exists());
}
#[test]
fn hosts_off_preserves_another_owners_changes_below_the_overlay() {
    let (_d, paths, p, privacy) = rig();
    let token = privacy.prepare("hosts", true).unwrap()["confirmation"]
        .as_str()
        .unwrap()
        .to_owned();
    privacy.apply("hosts", true, &token).unwrap();
    let changed=b"127.0.0.1 localhost\n0.0.0.0 su.lge.com # updated HBC block\n0.0.0.0 new.foreign.example\n";
    p.underlay
        .lock()
        .insert(paths.system("/etc/hosts"), changed.to_vec());
    let token = privacy.prepare("hosts", false).unwrap()["confirmation"]
        .as_str()
        .unwrap()
        .to_owned();
    privacy.apply("hosts", false, &token).unwrap();
    assert_eq!(std::fs::read(paths.system("/etc/hosts")).unwrap(), changed);
    assert!(!paths.state("hosts.enabled").exists());
}
fn boot_of(privacy: &Privacy, id: &str) -> Value {
    privacy.status(true)["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap()["boot"]
        .clone()
}
fn pass(ran: bool, running: Value) -> Value {
    json!({"ran":ran,"why":if ran{""}else{"the skip file is present"},"uptimeSec":14,"masked":[],"stubbed":[],"running":running})
}
#[test]
fn each_item_says_whether_it_is_blocked_from_boot_and_why() {
    let (_d, _paths, p, privacy) = rig();
    let set = |early: Value| {
        *p.dc.lock() = status_json(true, early);
    };
    // Without the early step only the boot pass stops LG's services, after LG started them.
    set(json!({"hook":false,"pending":false,"last":null}));
    let ads = boot_of(&privacy, "ads");
    assert_eq!(ads["blocked"], false);
    assert!(ads["reason"].as_str().unwrap().contains("not installed"));
    assert_eq!(
        boot_of(&privacy, "settings")["blocked"],
        true,
        "LG keeps settings across a reboot"
    );
    assert_eq!(boot_of(&privacy, "uploaders")["blocked"], false);
    assert_eq!(boot_of(&privacy, "push")["blocked"], false, "off");
    assert_eq!(boot_of(&privacy, "push")["reason"], "Off.");
    // Installed, no boot since: promised for the next boot, not yet shown.
    set(json!({"hook":true,"pending":false,"last":null}));
    let ads = boot_of(&privacy, "ads");
    assert_eq!(
        (ads["blocked"].clone(), ads["verified"].clone()),
        (json!(true), Value::Null)
    );
    let names: Vec<&str> = ads["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    for want in [
        "contentminer.service",
        "/usr/sbin/admanager",
        "/usr/sbin/adoverlay-service",
        "/usr/sbin/acr2",
        "/usr/sbin/sportsalarm",
        "/usr/sbin/pbs",
    ] {
        assert!(names.contains(&want), "{want}");
    }
    // The last pass found nothing running: shown.
    set(json!({"hook":true,"pending":false,"last":pass(true,json!([]))}));
    let ads = boot_of(&privacy, "ads");
    assert_eq!(
        (ads["blocked"].clone(), ads["verified"].clone()),
        (json!(true), json!(true))
    );
    assert!(
        ads["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["blocked"] == true && i["startedBefore"] == false)
    );
    assert_eq!(ads["lastPass"]["uptimeSec"], 14);
    // LG had started pbs and the content miner before the step could act: those are not blocked from boot, the others in other groups are.
    set(
        json!({"hook":true,"pending":false,"last":pass(true,json!(["/usr/sbin/pbs","contentminer.service"]))}),
    );
    let ads = boot_of(&privacy, "ads");
    assert_eq!(
        (ads["blocked"].clone(), ads["verified"].clone()),
        (json!(false), json!(false))
    );
    assert!(
        ads["reason"].as_str().unwrap().contains("/usr/sbin/pbs")
            && ads["reason"]
                .as_str()
                .unwrap()
                .contains("contentminer.service")
    );
    let started: Vec<&str> = ads["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["startedBefore"] == true)
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert_eq!(started, ["contentminer.service", "/usr/sbin/pbs"]);
    assert_eq!(boot_of(&privacy, "uploaders")["blocked"], true);
    // A boot the step skipped is not covered, and says why.
    set(json!({"hook":true,"pending":false,"last":pass(false,json!([]))}));
    let ads = boot_of(&privacy, "ads");
    assert_eq!(ads["blocked"], false);
    assert!(
        ads["reason"]
            .as_str()
            .unwrap()
            .contains("the skip file is present")
    );
    // The helper applies the hosts block itself, after the early step.
    assert_eq!(boot_of(&privacy, "hosts")["blocked"], false);
    assert_eq!(
        privacy.status(true)["boot"],
        json!({"earlyStep":true,"pending":false,"lastPass":pass(false,json!([]))})
    );
    // The tool not answering is not "blocked".
    *p.fail.lock() = true;
    let ads = boot_of(&privacy, "ads");
    assert_eq!(ads["blocked"], false);
    assert_eq!(ads["reason"], "The TV's privacy tool did not answer.");
}
