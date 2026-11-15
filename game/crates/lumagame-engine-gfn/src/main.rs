use lumagame_engine_gfn::runtime::{Options,run};

fn main() {
    let result = Options::parse(std::env::args().skip(1))
        .map_err(std::io::Error::other)
        .and_then(|options| run(options,std::io::stdin(),std::io::stdout().lock()));
    if let Err(error) = result {
        eprintln!("lumagame gfn: {error}");
        std::process::exit(1);
    }
}
