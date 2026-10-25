//! Talks to the real services; run by hand:
//! `cargo test -p lumatv-feeds -- --ignored live --nocapture`

use std::time::{SystemTime, UNIX_EPOCH};

use fetch::TlsFetch;
use lumatv_feeds::news::{self, NewsConfig};
use lumatv_feeds::weather::{self, Units, WeatherConfig};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[test]
#[ignore = "needs the network and /usr/bin/openssl"]
fn live_pipeline() {
    let fetch = TlsFetch::new("/usr/bin/openssl", "/etc/ssl/certs/ca-certificates.crt");
    let now = now_ms();

    let cfg = WeatherConfig {
        zip: "30311".to_owned(),
        units: Units::Fahrenheit,
    };
    let w = weather::refresh(&fetch, &cfg, None, now).expect("weather refresh");
    println!("weather: {} ({}, {})", w.place, w.lat, w.lon);
    println!(
        "  {} {} | {} | icon {}",
        w.display.temp, w.display.cond, w.display.stats, w.display.now_kind
    );
    for d in &w.display.days {
        println!("  {:<5} {:<8} {} / {}", d.day, d.kind, d.hi, d.lo);
    }
    assert_eq!(w.zip, "30311");
    assert!(!w.place.is_empty());
    assert!(!w.daily.is_empty());

    let n = news::refresh(&fetch, &NewsConfig::standard(), now).expect("news refresh");
    println!("news: {} stories from {:?}", n.items.len(), n.feeds);
    for item in &n.items {
        println!(
            "  [{} | {}] {} ({:?})",
            item.tag, item.src, item.title, item.pub_ms
        );
        println!("    link {}", item.link);
        println!("    img  {}", item.img);
    }
    assert!(!n.items.is_empty());
}
