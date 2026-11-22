#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
cargo zigbuild --locked --release --target aarch64-unknown-linux-gnu.2.36 "$@"
mkdir -p bin
cp "${CARGO_TARGET_DIR:-target}/aarch64-unknown-linux-gnu/release/lumagame-engine-steam" bin/lumagame-engine-steam
sha256sum bin/lumagame-engine-steam
