#!/usr/bin/env bash
# The loader's RACING PARTS, the SDK's own, at the ONE pinned SDK revision (the rev in Cargo.toml):
#   served.js (the one fetch; a 503 is "not yet"; raceK), rto.js + instrument-vocab.js (its generated constants),
#   pieces.js (open a piece set) and decoder.wasm (the erasure decoder).
# They are generated in a checkout of that revision and kept in loader/sdk/, so a build needs no SDK checkout.
#
#   tools/sdk-racing-parts.sh <path to a craftworks-sdk git repo>
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
repo="${1:?usage: tools/sdk-racing-parts.sh <craftworks-sdk repo>}"
rev=$(sed -n 's/^wire = .*rev = "\([0-9a-f]*\)".*/\1/p' "$here/Cargo.toml")
[ -n "$rev" ] || { echo "no SDK rev in Cargo.toml" >&2; exit 1; }
wt=$(mktemp -d "${TMPDIR:-/tmp}/sdk-$rev.XXXX")
git -C "$repo" worktree add --detach "$wt" "$rev" >/dev/null
trap 'git -C "$repo" worktree remove --force "$wt"' EXIT
out="$here/loader/sdk"
mkdir -p "$out"
cd "$wt"
target="$wt/target"
# The SDK builds against a contracts build: this repo's own, the same pinned contracts.
export CRAFTWORKS_CONTRACTS="$here/contracts"
CARGO_TARGET_DIR=$target cargo run -q -p page --example rto_js > "$out/rto.js"
CARGO_TARGET_DIR=$target cargo run -q -p page --example instrument_js > "$out/instrument-vocab.js"
grep -q '^export const RTO_SCHEDULE_MS = ' "$out/rto.js"
grep -q '^export const STRIDE = ' "$out/instrument-vocab.js"
cp js/served.js js/pieces.js "$out/"
CARGO_TARGET_DIR=$target cargo build -q --profile decoder -p decoder --target wasm32-unknown-unknown
tools/optimise-decoder.sh "$target/wasm32-unknown-unknown/decoder/decoder.wasm" "$out/decoder.wasm"
echo "$rev" > "$out/REV"
(cd "$out" && shasum -a 256 served.js pieces.js rto.js instrument-vocab.js decoder.wasm)
