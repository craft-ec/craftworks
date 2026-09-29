#!/usr/bin/env bash
# Reproducible build of this repo's own contracts, with freenet-contracts' ONE build definition (its
# contract-build.sh at the revision the workspace pins): same flags, same remaps, same binaryen, so a contract built
# here is built exactly as freenet-contracts builds its own. The script resolves the contract's directory from its OWN
# location, so it is linked here for the run (the link is not committed: it names this machine's cargo home).
#
#   contracts-src/build.sh idlog     ->  contracts-src/build/idlog.wasm, copied to contracts/idlog.wasm
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
rev=689ca61
ck=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/git/checkouts/freenet-contracts-*/"$rev" 2>/dev/null | head -1)
[ -n "$ck" ] || { echo "no freenet-contracts checkout at $rev: run a cargo build of the workspace first" >&2; exit 1; }
case $1 in
  idlog) crate=craftworks-idlog-contract ;;
  sealed) crate=craftworks-sealed-contract ;;
  bag) crate=craftworks-bag-contract ;;
  piece) crate=craftworks-piece-contract ;;
  *) echo "unknown contract: $1" >&2; exit 1 ;;
esac
ln -sf "$ck/contract-build.sh" "$here/.contract-build.sh"
trap 'rm -f "$here/.contract-build.sh"' EXIT
"$here/.contract-build.sh" "$1" "$crate"
cp "$here/build/$1.wasm" "$here/../contracts/$1.wasm"
