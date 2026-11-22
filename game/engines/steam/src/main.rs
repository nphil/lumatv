use lumagame_engine_steam::http::Ureq;
use lumagame_engine_steam::{Adapter, SystemClock, runtime};

fn main() {
    let adapter = Adapter::new(Ureq::new(), SystemClock::new(), |line| eprintln!("lumagame steam: {line}"));
    if let Err(error) = runtime::run(adapter, std::io::stdin(), std::io::stdout().lock()) {
        eprintln!("lumagame steam: {error}");
        std::process::exit(1);
    }
}
