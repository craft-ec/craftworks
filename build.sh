#!/usr/bin/env bash
# Build the Craftworks app's compiled packages into packages/build/:
# - the core (wasm + its JS glue); everything else in packages/ is published as it is written;
# - the identity delegate, stripped and GATED (a delegate importing a host function the node lacks never instantiates).
set -euo pipefail
root="$(cd "$(dirname "$0")" && pwd)"
cd "$root"
target="${CARGO_TARGET_DIR:-$root/target}"
cargo build -q --release -p craftworks-core --target wasm32-unknown-unknown
wasm-bindgen --target web --out-name craftworks_core --out-dir packages/build "$target/wasm32-unknown-unknown/release/craftworks_core.wasm"

cargo build -q --release -p craftworks-identity --target wasm32-unknown-unknown --features freenet-main-delegate
cargo build -q --release -p probe --bin import-gate
wasm-tools strip --all "$target/wasm32-unknown-unknown/release/craftworks_identity.wasm" -o packages/build/identity.wasm
"$target/release/import-gate" packages/build/identity.wasm
echo "identity: $(wc -c < packages/build/identity.wasm | tr -d ' ') B, sha256 $(shasum -a 256 packages/build/identity.wasm | cut -c1-16)…"
