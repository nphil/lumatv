//! The latency tool: `lumatv-gfn-latency run --dir /var/lib/lumatv-gfn --seconds 30` while a stream is in front
//! (docs/research/gfn-input-result.md, "Phase 4").
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(lumatv_gfn_pad::tool::main(&args, &mut std::io::stdout()));
}
