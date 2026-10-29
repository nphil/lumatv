use lumatv_privacy::{Paths,Platform,Privacy,atomic,hosts};
use parking_lot::Mutex;
use serde_json::{Value,json};
use std::path::Path;
use std::sync::Arc;
#[derive(Default)] struct Fake { calls:Mutex<Vec<Value>>, settings:Mutex<Value>, fail:Mutex<bool> }
impl Platform for Fake {
    fn luna(&self,method:&str,body:&Value)->Result<Value,String>{
        self.calls.lock().push(json!({"method":method,"body":body}));
        if method=="setSystemSettings" {if *self.fail.lock(){return Err("LG refused the change".into());} *self.settings.lock()=body["settings"].clone();}
        Ok(json!({"returnValue":true,"settings":*self.settings.lock()}))
    }
    fn declutter(&self,args:&[&str])->Result<Value,String>{
        self.calls.lock().push(json!({"declutter":args}));
        if *self.fail.lock(){return Err("declutter failed".into());}
        Ok(json!({"ok":true,"groups":{"ads":{"on":false,"inTarget":"0/6"},"settings":{"on":true,"inTarget":"16/16"},"uploaders":{"on":true,"inTarget":"7/7"},"push":{"on":false,"inTarget":"0/5"}}}))
    }
    fn bind(&self,source:&Path,target:&Path)->Result<(),String>{self.calls.lock().push(json!({"bind":[source,target]}));std::fs::copy(source,target).map(drop).map_err(|e|e.to_string())}
    fn unmount(&self,target:&Path)->Result<(),String>{self.calls.lock().push(json!({"unmount":target}));Ok(())}
    fn cert_hash(&self,_:&Path)->Result<String,String>{Ok("01234567".into())}
}
fn rig()->(tempfile::TempDir,Paths,Arc<Fake>,Privacy) {
    let dir=tempfile::tempdir().unwrap();let paths=Paths{root:dir.path().join("root"),data:dir.path().join("state")};
    atomic(&paths.system("/etc/hosts"),b"127.0.0.1 localhost\n0.0.0.0 su.lge.com # HBC\n0.0.0.0 foreign.example\n",false).unwrap();
    atomic(&paths.system("/var/palm/license/eulaInfoNetwork.json"),br#"{"eulaMappingList":{"eulaInfo":[{"settingKey":"acrAllowed","mandatory":["A"]},{"settingKey":"customAdAllowed","mandatory":["A"]}]}}"#,false).unwrap();
    let platform=Arc::new(Fake{settings:Mutex::new(json!({"eulaStatus":{"acrAllowed":true,"customAdAllowed":true,"voiceAllowed":true,"networkAllowed":true},"eulaInfoNetwork":{"eulaList":[{"id":"A","accepted":true,"title":"Analytics"}]}})),..Fake::default()});
    let privacy=Privacy::new(paths.clone(),platform.clone());(dir,paths,platform,privacy)
}
#[test] fn read_is_nonintrusive_and_menu_has_every_item_and_existing_links() {
    let (_d,paths,p,privacy)=rig();let before=std::fs::read(paths.system("/etc/hosts")).unwrap();
    let status=privacy.status(false);let items=status["items"].as_array().unwrap();
    for id in ["ads","settings","uploaders","push","hosts","amazoff","consent.acrAllowed","permissions","deletedata"] {assert!(items.iter().any(|r|r["id"]==id));}
    assert_eq!(status["canAdmin"],false);assert_eq!(items[1]["settings"].as_array().unwrap().len(),16);
    assert_eq!(before,std::fs::read(paths.system("/etc/hosts")).unwrap());
    assert!(p.calls.lock().iter().all(|v|v["method"]=="getSystemSettings"||v["declutter"]==json!(["status"])));
    assert!(!paths.state("prime-identity.json").exists());
}
#[test] fn confirmation_is_exact_single_use_and_declutter_is_reused() {
    let (_d,_paths,p,privacy)=rig();
    let token=privacy.prepare("uploaders",true).unwrap()["confirmation"].as_str().unwrap().to_owned();
    privacy.apply("uploaders",true,&token).unwrap();
    assert!(privacy.apply("uploaders",true,&token).is_err());
    assert_eq!(*p.calls.lock(),vec![json!({"declutter":["on","--group","uploaders"]}),json!({"declutter":["persist","on"]})]);
    let token=privacy.prepare("ads",true).unwrap()["confirmation"].as_str().unwrap().to_owned();
    assert!(privacy.apply("ads",false,&token).is_err());assert_eq!(p.calls.lock().len(),2);
    assert!(privacy.prepare("apps",true).is_err());assert!(privacy.prepare("voice",true).is_err());
}
#[test] fn consent_writes_both_documents_and_flags_and_rejects_changed_agreements() {
    let (_d,_paths,p,privacy)=rig();
    let token=privacy.prepare("consent.acrAllowed",false).unwrap()["confirmation"].as_str().unwrap().to_owned();
    privacy.apply("consent.acrAllowed",false,&token).unwrap();
    assert_eq!(p.settings.lock()["eulaStatus"]["acrAllowed"],false);
    assert_eq!(p.settings.lock()["eulaStatus"]["customAdAllowed"],false);
    assert_eq!(p.settings.lock()["eulaStatus"]["voiceAllowed"],true);
    let token=privacy.prepare("consent.acrAllowed",true).unwrap()["confirmation"].as_str().unwrap().to_owned();
    p.settings.lock()["eulaInfoNetwork"]["eulaList"][0]["title"]=json!("A changed agreement");
    assert!(privacy.apply("consent.acrAllowed",true,&token).unwrap_err().contains("agreements changed"));
    assert!(privacy.prepare("consent.networkAllowed",false).is_err());
}
#[test] fn hosts_on_off_preserves_other_owners_and_boot_resumes_only_saved_choices() {
    let (_d,paths,p,privacy)=rig();let original=std::fs::read(paths.system("/etc/hosts")).unwrap();
    privacy.resume();assert!(p.calls.lock().is_empty());
    let token=privacy.prepare("hosts",true).unwrap()["confirmation"].as_str().unwrap().to_owned();privacy.apply("hosts",true,&token).unwrap();
    assert!(paths.state("hosts.enabled").exists());assert!(std::fs::read_to_string(paths.system("/etc/hosts")).unwrap().contains(hosts::BEGIN));
    std::fs::write(paths.system("/etc/hosts"),&original).unwrap();privacy.maintain();
    assert!(std::fs::read_to_string(paths.system("/etc/hosts")).unwrap().contains(hosts::BEGIN));
    let token=privacy.prepare("hosts",false).unwrap()["confirmation"].as_str().unwrap().to_owned();privacy.apply("hosts",false,&token).unwrap();
    assert!(!paths.state("hosts.enabled").exists());assert_eq!(std::fs::read(paths.system("/etc/hosts")).unwrap(),original);
}
#[test] fn platform_failure_is_returned_not_reported_as_success() {
    let (_d,_paths,p,privacy)=rig();let token=privacy.prepare("uploaders",true).unwrap()["confirmation"].as_str().unwrap().to_owned();
    *p.fail.lock()=true;assert_eq!(privacy.apply("uploaders",true,&token).unwrap_err(),"declutter failed");
}
#[test] fn old_tls_bypass_refuses_activation_without_touching_prime_or_platform() {
    let (_d,paths,p,privacy)=rig();std::fs::create_dir_all(paths.system(lumatv_privacy::prime::CERT_DIR)).unwrap();
    atomic(&paths.system(lumatv_privacy::prime::JAIL_HOSTS),b"127.0.0.1 localhost\n",false).unwrap();
    atomic(&paths.system("/media/cryptofs/apps/usr/palm/applications/amazon/default_config.json"),br#"{"disable-ssl-cert":"1"}"#,false).unwrap();
    let token=privacy.prepare("amazoff",true).unwrap()["confirmation"].as_str().unwrap().to_owned();
    assert!(privacy.apply("amazoff",true,&token).unwrap_err().contains("TLS checks"));assert!(p.calls.lock().is_empty());
    assert!(!paths.state("prime.enabled").exists());assert!(!paths.state("prime-identity.json").exists());
}
