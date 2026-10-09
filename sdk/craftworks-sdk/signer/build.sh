#!/usr/bin/env bash
# Build the SIGNER delegate and GATE it, in one step: a delegate importing a host
# function the node does not define fails to INSTANTIATE and is simply never called, so building without checking
# produces exactly the artefact that fails that way. The signer imports only secrets and the synchronous
# `get_contract_state`; the gate proves it (0.2.136 removed put/update/subscribe_contract, which it never calls).
set -euo pipefail
cd "$(dirname "$0")/.."

# Where cargo puts the build (tools/target-dir.sh): CARGO_TARGET_DIR is honoured, never unset.
target=$(tools/target-dir.sh)
cargo build -p signer --release --target wasm32-unknown-unknown --features freenet-main-delegate
cargo build -p probe --release --bin import-gate

raw=$target/wasm32-unknown-unknown/release/signer.wasm
wasm=$target/wasm32-unknown-unknown/release/signer.stripped.wasm
if command -v wasm-tools > /dev/null; then
  wasm-tools strip --all "$raw" -o "$wasm"
else
  echo "note: wasm-tools not found — shipping unstripped" >&2
  cp "$raw" "$wasm"
fi
echo "signer: $(wc -c < "$wasm" | tr -d ' ') B stripped ($(wc -c < "$raw" | tr -d ' ') B before)"
"$target/release/import-gate" "$wasm"
