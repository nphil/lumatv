//! Ads tier from rorygallagher2024/glasshouse, server/lib/privacy.js (MIT; payload/privacy/NOTICE).
//! Only whole names: never block the Content Store's lgtvsdp.com/nextlgsdp.com families.
use std::collections::BTreeSet;
pub const BEGIN: &str = "# LumaTV privacy: ads and trackers begin";
pub const END: &str = "# LumaTV privacy: ads and trackers end";
const BASE: &[&str] = &[
    "ad.lgsmartad.com","ibis.lgappstv.com","ibs.lgappstv.com","lgsmartad.com","rdx.lgtvcommon.com","aic.lgtvcommon.com","smartad.lge.com","smartclip.com","smartclip-services.com","yumenetworks.com",
    "alphonso.tv","prov-lg.alphonso.tv","prov.alphonso.tv","eulacheck.alphonso.tv","bl-server.alphonso.tv","clockskew.alphonso.tv","acrdb.alphonso.tv","ads.alphonso.tv","api.alphonso.tv","tvads.alphonso.tv","sync.alphonso.tv","tr.alphonso.tv","tn.alphonso.tv","insights.alphonso.tv","bwlkup.alphonso.tv","eu-acr86.alphonso.tv"
];
const REGIONAL: &[&str] = &["info.lgsmartad.com","rdx2.lgtvsdp.com","ibs.lgappstv.com","ibsstat.lgappstv.com","cdpbeacon.lgtvcommon.com","cdpsvc.lgtvcommon.com","wau.lgtvcommon.com"];
pub fn names(country: &str) -> BTreeSet<String> {
    let mut prefixes = BTreeSet::from(["aic","eic","kic","eu"]);
    let country = country.trim();
    if country.len() == 2 && country.bytes().all(|b| b.is_ascii_lowercase()) { prefixes.insert(country); }
    else { prefixes.extend(["us","gb","au","br","ca","de","fr"]); }
    let mut names: BTreeSet<String> = BASE.iter().map(|s| (*s).into()).collect();
    for domain in REGIONAL {
        names.insert((*domain).into());
        for prefix in &prefixes { names.insert(format!("{prefix}.{domain}")); }
    }
    names
}
pub fn table(current: &str, enabled: bool, country: &str) -> Result<String, String> {
    let mut out = String::with_capacity(current.len() + 8192);
    let mut own = false;
    let mut seen = false;
    for line in current.lines() {
        if line == BEGIN {
            if own || seen { return Err("duplicate privacy hosts marker".into()); }
            seen = true; own = true;
        } else if line == END {
            if !own { return Err("unmatched privacy hosts marker".into()); }
            own = false;
        } else if !own { out.push_str(line); out.push('\n'); }
    }
    if own { return Err("unfinished privacy hosts block".into()); }
    if enabled {
        out.push_str(BEGIN); out.push('\n');
        for name in names(country) { out.push_str(&format!("0.0.0.0 {name}\n:: {name}\n")); }
        out.push_str(END); out.push('\n');
    }
    Ok(out)
}
pub fn prime_hosts(current: &str, enabled: bool) -> String {
    // Remove only the asset host, including old AmazOff IPv4/IPv6 entries; preserve aliases on the same line.
    let mut out=String::new();
    for line in current.lines() {
        let (entry, comment) = line.split_once('#').map(|(a,b)|(a,Some(b))).unwrap_or((line,None));
        let mut fields = entry.split_whitespace();
        let Some(addr) = fields.next() else { out.push_str(line); out.push('\n'); continue; };
        let hosts: Vec<_> = fields.collect();
        if !hosts.iter().any(|h| h.eq_ignore_ascii_case(crate::prime::HOST)) { out.push_str(line); out.push('\n'); continue; }
        let kept: Vec<_> = hosts.into_iter().filter(|h| !h.eq_ignore_ascii_case(crate::prime::HOST)).collect();
        if !kept.is_empty() {
            out.push_str(addr); out.push(' '); out.push_str(&kept.join(" "));
            if let Some(c)=comment {out.push_str(" #"); out.push_str(c);}
            out.push('\n');
        }
    }
    if enabled { out.push_str(&format!("127.0.0.1 {} # LumaTV Prime assets\n", crate::prime::HOST)); }
    out
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn preserves_update_blocks_and_foreign_entries() {
        let old="127.0.0.1 localhost\n0.0.0.0 su.lge.com # HBC\n0.0.0.0 custom.example\n";
        let on=table(old,true,"us").unwrap();
        assert_eq!(table(&on,false,"us").unwrap(),old);
        assert_eq!(table(&on,true,"us").unwrap(),on);
        assert!(on.contains(":: us.info.lgsmartad.com"));
        assert!(!names("us").contains("lgtvsdp.com"));
        assert!(!names("us").contains(crate::prime::HOST));
        assert!(table(BEGIN,false,"us").is_err());
    }
    #[test] fn prime_off_never_restores_old_override() {
        let s=format!("127.0.0.1 {} alias # old\n::1 {}\n",crate::prime::HOST,crate::prime::HOST);
        let on=prime_hosts(&s,true);
        assert_eq!(prime_hosts(&on,false),"127.0.0.1 alias # old\n");
        assert_eq!(on.matches(crate::prime::HOST).count(),1);
    }
}
