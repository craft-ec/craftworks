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
  allowance) crate=craftworks-allowance-contract ;;
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
# FROZEN (contracts/FROZEN): a contract whose code hash keys what it holds is not replaced by other bytes unless the
# change is DECLARED (CONTRACT_VERSION_BUMP=<name>: a version bump, with the owner's say; then its line there updated).
pin=$(awk -v f="$1.wasm" '$1 !~ /^#/ && $2 == f { print $1 }' "$here/../contracts/FROZEN")
got=$(shasum -a 256 "$here/build/$1.wasm" | cut -d' ' -f1)
if [ -n "$pin" ] && [ "$got" != "$pin" ] && [ "${CONTRACT_VERSION_BUMP:-}" != "$1" ]; then
  echo "$1.wasm is FROZEN (contracts/FROZEN): this build is $got, the pin $pin — every address it keys would move." >&2
  echo "Not copied into contracts/ (the build is in contracts-src/build/$1.wasm). A real change: CONTRACT_VERSION_BUMP=$1, the owner's say, and its FROZEN line." >&2
  exit 1
fi
cp "$here/build/$1.wasm" "$here/../contracts/$1.wasm"
