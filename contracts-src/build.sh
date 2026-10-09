#!/usr/bin/env bash
# Reproducible build of this repo's own contracts, with freenet-contracts' ONE build definition (its
# contract-build.sh at the revision the workspace pins): same flags, same remaps, same binaryen, so a contract built
# here is built exactly as freenet-contracts builds its own. The script resolves the contract's directory from its OWN
# location, so it is linked here for the run (the link is not committed).
#
#   contracts-src/build.sh idlog     ->  contracts-src/build/idlog.wasm, copied to contracts/idlog.wasm
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# freenet-contracts' build definition, copied into this repo at 689ca61 (sdk/freenet-contracts).
ck="$here/../sdk/freenet-contracts"
case $1 in
  idlog) crate=craftworks-idlog-contract ;;
  sealed) crate=craftworks-sealed-contract ;;
  bag) crate=craftworks-bag-contract ;;
  piece) crate=craftworks-piece-contract ;;
  token) crate=craftworks-token-contract ;;
  devices) crate=craftworks-devices-contract ;;
  beacon) crate=craftworks-beacon-contract ;;
  standing) crate=craftworks-standing-contract ;;
  pool) crate=craftworks-pool-contract ;;
  swap) crate=craftworks-swap-contract ;;
  *) echo "unknown contract: $1" >&2; exit 1 ;;
esac
# FROM ONE FIXED PATH (measured 10-09): Cargo's metadata for a PATH dependency (identity, standing, sdk — crates at
# the repo root) includes its absolute path, so the same source built in two checkouts gave two hashes (the remaps do
# not reach it: no path is in the bytes, the code layout moves). So the sources — no targets, no .git — are copied to
# one path every machine shares and built there (its targets kept for the next build); a lock keeps two builds apart.
fixed=/tmp/craftworks-contract-build
until mkdir "$fixed.lock" 2>/dev/null; do sleep 1; done
trap 'rm -rf "$fixed.lock"' EXIT
mkdir -p "$fixed"
rsync -a --delete --exclude target --exclude .git --exclude node_modules --exclude /contracts-src/build "$here/../" "$fixed/"
ln -sf "$fixed/sdk/freenet-contracts/contract-build.sh" "$fixed/contracts-src/.contract-build.sh"
(cd "$fixed/contracts-src" && ./.contract-build.sh "$1" "$crate")
mkdir -p "$here/build"
cp "$fixed/contracts-src/build/$1.wasm" "$here/build/$1.wasm"
cp "$here/build/$1.wasm" "$here/../contracts/$1.wasm"
