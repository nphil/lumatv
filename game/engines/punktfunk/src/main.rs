fn main() {
    if let Err(error) = lumagame_engine_punktfunk::run() {
        eprintln!("Punktfunk engine: {error:#}");
        std::process::exit(1);
    }
}
