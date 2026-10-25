fn main() {
    std::process::exit(lumatv_helper::cli::main(std::env::args().skip(1).collect()));
}
