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
# MLS: its own package (the keys capability's), never in the core.
cargo build -q --release -p craftworks-mls --target wasm32-unknown-unknown
wasm-bindgen --target web --out-name craftworks_mls --out-dir packages/build "$target/wasm32-unknown-unknown/release/craftworks_mls.wasm"

# FEEDS: its own package (the storage capability's): a row as a version, and the merge of writers' feeds.
cargo build -q --release -p craftworks-feed --target wasm32-unknown-unknown
wasm-bindgen --target web --out-name craftworks_feed --out-dir packages/build "$target/wasm32-unknown-unknown/release/craftworks_feed.wasm"

for name in craftworks_core craftworks_mls craftworks_feed; do
  # Optimised for size (binaryen, the same flags freenet-contracts uses): every page load fetches these.
  wasm-opt -Os --enable-bulk-memory-opt --enable-bulk-memory "packages/build/${name}_bg.wasm" -o "packages/build/${name}_bg.opt.wasm"
  [ -s "packages/build/${name}_bg.opt.wasm" ] || { echo "wasm-opt wrote nothing for $name" >&2; exit 1; }
  mv "packages/build/${name}_bg.opt.wasm" "packages/build/${name}_bg.wasm"
  # A package is loaded as bytes, not as a file beside others: wasm-bindgen's snippet files (inline JS a crate asked
  # for) are put INTO the glue, so it imports nothing by path.
  python3 tools/inline-snippets.py "packages/build/${name}.js"
  echo "$name: $(wc -c < "packages/build/${name}_bg.wasm" | tr -d ' ') B"
done

cargo build -q --release -p craftworks-delegate --target wasm32-unknown-unknown --features freenet-main-delegate
cargo build -q --release -p probe --bin import-gate
# Every custom section but the MANIFEST (`freenet-manifest`: the delegate's wake-ups) removed.
wasm-tools strip -d '^(name|producers|target_features|\.debug.*|linking|reloc\..*|component-type.*)$' "$target/wasm32-unknown-unknown/release/craftworks_delegate.wasm" -o packages/build/identity.wasm
"$target/release/import-gate" packages/build/identity.wasm
# AND nothing imported but the node's own modules (`freenet_*`): the gate above names only what the node defines and
# let a wasm-bindgen placeholder import through (a crate's JS bindings linked in) — a delegate that never instantiates.
python3 tools/wasm-imports.py packages/build/identity.wasm | awk '$1 !~ /^freenet_/ { bad = 1; print "identity.wasm imports " $1 "::" $2 > "/dev/stderr" } END { exit bad }' || { echo "the identity delegate imports what no node provides" >&2; exit 1; }
echo "identity: $(wc -c < packages/build/identity.wasm | tr -d ' ') B, sha256 $(shasum -a 256 packages/build/identity.wasm | cut -c1-16)…"
