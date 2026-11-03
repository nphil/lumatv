#!/bin/sh
# Runs every LumaTV test suite and clippy at low priority, one summary line per suite; full logs in $OUT (default build/check).
# Run before pushing changes that touch more than one crate, and after any sync of installed code.
# The compositor is checked with `--features ui`, the shipping set (compositor/tools/build-tv.sh); without it the
# shell_rust*, ui* and transitions_rust_home tests compile to nothing.
# Compositor and helper logs go to the collector at /tmp/lumatv-log.sock, not stderr: to see why a test's compositor
# exited, bind a SOCK_DGRAM Unix socket there for the run.
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
out=${OUT:-$root/build/check}
mkdir -p "$out"
: > "$out/SUMMARY"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
run() { # name dir command...
  name=$1; dir=$2; shift 2
  start=$(date +%s)
  (cd "$dir" && nice -n 19 "$@") > "$out/$name.log" 2>&1
  code=$?
  tests=$(grep -oE 'test result: [a-zA-Z]+\. [0-9]+ passed; [0-9]+ failed; [0-9]+ ignored' "$out/$name.log" |
    awk '{p+=$4; f+=$6; i+=$8} END{if (NR) print p" passed, "f" failed, "i" ignored"}')
  echo "$name: exit $code, $(( $(date +%s) - start )) s${tests:+, $tests}" | tee -a "$out/SUMMARY"
}
run compositor-test   "$root/compositor" cargo test --workspace --features ui --no-fail-fast
run compositor-clippy "$root/compositor" cargo clippy --workspace --features ui --all-targets -q
run rustui-test       "$root/rustui" cargo test --workspace --no-fail-fast
run rustui-clippy     "$root/rustui" cargo clippy --workspace --all-targets -q
run helper-test       "$root/helper" cargo test --workspace --no-fail-fast
run helper-clippy     "$root/helper" cargo clippy --workspace --all-targets -q
# lumatv-render is vendored from LumaShow with its unit-test files stripped on purpose (sync-lumashow.sh, `[lib] test = false`),
# so its #[cfg(test)] modules cannot compile under --all-targets; its library is still checked as a dependency.
run rustui-clippy     "$root/rustui" cargo clippy --workspace --all-targets --exclude lumatv-render -q
grep -q 'exit [^0]' "$out/SUMMARY" && exit 1 || exit 0
