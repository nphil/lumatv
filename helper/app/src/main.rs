fn main() {
    let internal: Vec<String> = std::env::args().skip(1).collect();
    if let [command, host, port] = internal.as_slice() {
        if command == "__logging-resolve" { std::process::exit(lumatv_helper::logging::resolver_entry(host, port)); }
    }
    std::process::exit(lumatv_helper::cli::main(internal));
}
