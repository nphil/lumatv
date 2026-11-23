//! `audiopass status`: what the route sees on this TV. Reads only; never changes anything.

use audiopass::host::{Host, System};
use audiopass::{firmware, graph, health, sndout};
use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("status") => {
            status();
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: audiopass status");
            ExitCode::from(2)
        }
    }
}

fn status() {
    let host = System::default();
    let fw = firmware::check(&host);
    println!(
        "release   {}",
        fw.release.as_deref().unwrap_or("unreadable")
    );
    println!("kernel    {}", fw.kernel.as_deref().unwrap_or("unreadable"));
    match &fw.verdict {
        Ok(()) => println!("firmware  matches the proven build"),
        Err(why) => println!("firmware  refused: {why}"),
    }
    match host
        .read(sndout::SNDOUT)
        .ok()
        .map(|text| sndout::parse(&text))
    {
        Some(snd) => {
            println!(
                "earc      {}",
                snd.earc_on
                    .map_or("unknown", |on| if on { "on" } else { "off" })
            );
            println!(
                "words     {}",
                snd.earc
                    .map_or_else(|| "unreadable".into(), |words| words.to_string())
            );
            println!(
                "codec     {}",
                snd.arc_codec.as_deref().unwrap_or("unreadable")
            );
            println!(
                "adec      {}",
                match snd.connected_adecs().as_slice() {
                    [] => "none connected".to_owned(),
                    list => format!("ADEC{} connected", list.join(", ADEC")),
                }
            );
        }
        None => println!("sndout    unreadable"),
    }
    println!("health    {:?}", health::check(&host));
    let chain = host
        .read(audiopass::route::GRAPH)
        .map(|text| graph::bypass_converter(&text, None));
    println!("converter {chain:?}");
}
