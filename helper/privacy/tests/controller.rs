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
    old: Mutex<Vec<String>>,
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
    fn close_app(&self, id: &str) -> Result<(), String> {
        self.calls.lock().push(json!({"close":id}));
        Ok(())
    }
    fn stop_old_amazoff(&self) -> Result<Vec<String>, String> {
        self.calls.lock().push(json!({"stop_old_amazoff":true}));
        Ok(self.old.lock().clone())
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
            json!({"hook":false,"pending":false,"thisBoot":false,"last":null}),
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
        "remoteservice",
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
fn lg_remote_service_is_blocked_until_the_owner_confirms_and_blocked_again_when_turned_off() {
    let (_d, paths, p, privacy) = rig();
    let row = |privacy: &Privacy| privacy.status(true)["items"].as_array().unwrap().iter().find(|r| r["id"] == "remoteservice").cloned().unwrap();
    assert_eq!((row(&privacy)["on"].clone(), row(&privacy)["admin"].clone()), (json!(false), json!(true)), "blocked by default");
    // preparing changes nothing, and a token for another item or the other state does not apply it
    let token = privacy.prepare("remoteservice", true).unwrap()["confirmation"].as_str().unwrap().to_owned();
    assert!(!paths.state("remoteservice.allowed").exists());
    assert!(privacy.apply("remoteservice", false, &token).is_err());
    assert!(!paths.state("remoteservice.allowed").exists());
    let token = privacy.prepare("remoteservice", true).unwrap()["confirmation"].as_str().unwrap().to_owned();
    privacy.apply("remoteservice", true, &token).unwrap();
    assert!(paths.state("remoteservice.allowed").is_file());
    assert_eq!(row(&privacy)["on"], json!(true));
    assert!(privacy.apply("remoteservice", true, &token).is_err(), "single use");
    let token = privacy.prepare("remoteservice", false).unwrap()["confirmation"].as_str().unwrap().to_owned();
    privacy.apply("remoteservice", false, &token).unwrap();
    assert!(!paths.state("remoteservice.allowed").exists());
    assert!(p.calls.lock().iter().all(|v| v["method"] != "setSystemSettings"), "no LG setting is written for it");
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
fn prime_files(paths: &Paths, config: &[u8]) {
    std::fs::create_dir_all(paths.system(lumatv_privacy::prime::CERT_DIR)).unwrap();
    atomic(
        &paths.system(lumatv_privacy::prime::JAIL_HOSTS),
        b"127.0.0.1 localhost\n",
        false,
    )
    .unwrap();
    atomic(
        &paths.system(lumatv_privacy::legacy::PRIME_CONFIG),
        config,
        false,
    )
    .unwrap();
}
#[test]
fn turning_prime_ads_on_closes_prime_and_retires_the_old_amazoff_itself() {
    let (_d, paths, p, _privacy) = rig();
    prime_files(&paths, br#"{"disable-ssl-cert":"1","keep":"this"}"#);
    atomic(
        &paths.system("/var/lib/webosbrew/init.d/50-amazoff"),
        b"#!/bin/sh\nstart amazoff\n",
        false,
    )
    .unwrap();
    atomic(
        &paths.system("/var/lib/webosbrew/init.d/10-other"),
        b"#!/bin/sh\necho hi\n",
        false,
    )
    .unwrap();
    *p.old.lock() = vec!["nginx".into(), "amazoff-monitor".into()];
    lumatv_privacy::prime::retire_old(&paths, p.as_ref()).unwrap();
    assert_eq!(
        *p.calls.lock(),
        vec![json!({"close":"amazon"}), json!({"stop_old_amazoff":true})],
        "Prime is closed first, then the old app stopped"
    );
    let config: Value = serde_json::from_slice(
        &std::fs::read(paths.system(lumatv_privacy::legacy::PRIME_CONFIG)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        config,
        json!({"keep":"this"}),
        "only the certificate-check override is removed"
    );
    assert!(
        !paths
            .system("/var/lib/webosbrew/init.d/50-amazoff")
            .exists(),
        "its start-up script no longer runs at boot"
    );
    assert!(
        paths.state("retired/50-amazoff").exists(),
        "and is kept, not deleted"
    );
    assert!(
        paths.system("/var/lib/webosbrew/init.d/10-other").exists(),
        "another start-up script is left alone"
    );
}
#[test]
fn applying_prime_ads_asks_nothing_of_the_owner_and_never_leaves_prime_open() {
    let (_d, paths, p, privacy) = rig();
    prime_files(&paths, br#"{"disable-ssl-cert":true}"#);
    let plan = privacy.prepare("amazoff", true).unwrap();
    let notice = plan["plan"]["notice"].as_str().unwrap();
    assert!(notice.contains("Prime Video restarts"), "{notice}");
    for jargon in ["CA", "goproxy", "rootfs", "DRM", "cloudfront"] {
        assert!(!notice.contains(jargon), "{notice}");
    }
    assert!(
        !notice.contains("before enabling"),
        "no step is left to the owner: {notice}"
    );
    let token = plan["confirmation"].as_str().unwrap().to_owned();
    // the port is the only thing a test cannot give: whatever the outcome, the old app was dealt with first
    let _ = privacy.apply("amazoff", true, &token);
    let calls = p.calls.lock().clone();
    assert_eq!(
        calls[..2],
        [json!({"close":"amazon"}), json!({"stop_old_amazoff":true})]
    );
    let config: Value = serde_json::from_slice(
        &std::fs::read(paths.system(lumatv_privacy::legacy::PRIME_CONFIG)).unwrap(),
    )
    .unwrap();
    assert_eq!(config.get("disable-ssl-cert"), None);
}
#[test]
fn prime_ads_say_plainly_when_the_tv_cannot_have_them() {
    let (_d, paths, _p, privacy) = rig();
    let row = |privacy: &Privacy| {
        privacy.status(true)["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "amazoff")
            .unwrap()
            .clone()
    };
    let r = row(&privacy);
    assert_eq!(
        (r["available"].clone(), r["reason"].as_str().unwrap()),
        (json!(false), "Prime Video is not installed on this TV")
    );
    std::fs::create_dir_all(paths.system(lumatv_privacy::prime::CERT_DIR)).unwrap();
    assert_eq!(
        row(&privacy)["reason"],
        "Prime Video has not run on this TV yet"
    );
    assert!(privacy.apply("amazoff", true, "x").is_err());
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
    json!({"ran":ran,"why":if ran{""}else{"the skip file is present"},"startSec":13.5,"stubbedSec":13.6,"maskedSec":14.0,"uptimeSec":14.2,"masked":[],"stubbed":[],"running":running})
}
#[test]
fn each_item_says_whether_it_is_blocked_from_boot_and_why() {
    let (_d, _paths, p, privacy) = rig();
    let set = |early: Value| {
        *p.dc.lock() = status_json(true, early);
    };
    // Without the early step only the boot pass stops LG's services, after LG started them.
    set(json!({"hook":false,"pending":false,"thisBoot":false,"last":null}));
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
    set(json!({"hook":true,"pending":false,"thisBoot":false,"last":null}));
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
    set(json!({"hook":true,"pending":false,"thisBoot":true,"last":pass(true,json!([]))}));
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
    assert_eq!(ads["lastPass"]["uptimeSec"], 14.2);
    assert_eq!(ads["lastPass"]["stubbedSec"], 13.6);
    // LG had started pbs and the content miner before the step could act: those are not blocked from boot, the others in other groups are.
    set(
        json!({"hook":true,"pending":false,"thisBoot":true,"last":pass(true,json!(["/usr/sbin/pbs","contentminer.service"]))}),
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
    set(json!({"hook":true,"pending":false,"thisBoot":true,"last":pass(false,json!([]))}));
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
        json!({"earlyStep":true,"pending":false,"thisBoot":true,"lastPass":{"ran":false,"why":"the skip file is present","startSec":13.5,"stubbedSec":13.6,"maskedSec":14.0,"uptimeSec":14.2}})
    );
    // A record from an earlier boot proves nothing about this one: not blocked until a pass has run since it started.
    set(json!({"hook":true,"pending":false,"thisBoot":false,"last":pass(true,json!([]))}));
    let ads = boot_of(&privacy, "ads");
    assert_eq!(
        (ads["blocked"].clone(), ads["verified"].clone()),
        (json!(false), Value::Null)
    );
    assert!(
        ads["reason"]
            .as_str()
            .unwrap()
            .contains("since this boot started")
    );
    assert!(
        ads["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["blocked"] == false)
    );
    assert_eq!(
        boot_of(&privacy, "settings")["blocked"],
        true,
        "settings persist across a boot, no pass needed"
    );
    assert_eq!(privacy.status(true)["boot"]["thisBoot"], false);
    // The tool not answering is not "blocked".
    *p.fail.lock() = true;
    let ads = boot_of(&privacy, "ads");
    assert_eq!(ads["blocked"], false);
    assert_eq!(ads["reason"], "The TV's privacy tool did not answer.");
}
#[test]
fn the_push_row_lists_the_iot_client_that_ls_hubd_launched_although_its_unit_is_masked() {
    let (_d, _paths, p, privacy) = rig();
    let iot = "/usr/palm/services/com.webos.service.iotclient/iot-client";
    let set = |running: Value| {
        let early = json!({"hook":true,"pending":false,"thisBoot":true,"last":pass(true,running)});
        let mut status = status_json(true, early);
        status["groups"]["push"]["on"] = json!(true);
        *p.dc.lock() = status;
    };
    let item = |push: &Value| {
        push["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["name"] == iot)
            .cloned()
            .expect("the IoT client is an item of the push row")
    };
    set(json!([]));
    let push = boot_of(&privacy, "push");
    let it = item(&push);
    assert_eq!(
        (&it["kind"], &it["blocked"], &it["startedBefore"]),
        (&json!("program"), &json!(true), &json!(false))
    );
    assert_eq!(push["blocked"], true);
    // LG's service hub had launched it before the step could act: not blocked from boot, and the row says which.
    set(json!([iot]));
    let push = boot_of(&privacy, "push");
    let it = item(&push);
    assert_eq!(
        (&it["blocked"], &it["startedBefore"]),
        (&json!(false), &json!(true))
    );
    assert_eq!(push["blocked"], false);
    assert!(push["reason"].as_str().unwrap().contains(iot));
}
