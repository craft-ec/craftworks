#!/usr/bin/env bash
# Build the Craftworks app's one compiled package: the core (wasm + its JS glue) into packages/build/. Everything
# else in packages/ is published as it is written.
set -euo pipefail
root="$(cd "$(dirname "$0")" && pwd)"
cd "$root"
target="${CARGO_TARGET_DIR:-$root/target}"
cargo build -q --release -p craftworks-core --target wasm32-unknown-unknown
wasm-bindgen --target web --out-name craftworks_core --out-dir packages/build "$target/wasm32-unknown-unknown/release/craftworks_core.wasm"
