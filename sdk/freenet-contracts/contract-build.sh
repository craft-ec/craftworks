#!/usr/bin/env bash
# One reproducible wasm build, shared by every contract.
#
# A contract's wasm hash is part of its network key, so the same source must
# produce the same bytes on any machine, from any directory. Three things would
# otherwise reach the binary:
#
#   * Panic LOCATIONS. Every `unwrap`, `expect`, index, `/`, `%` and
#     `#[track_caller]` callee (`copy_from_slice`, `split_at`, `chunks_exact`,
#     …) carries `file:line:col`, so adding one comment line to a source file
#     re-keys every instance of that contract in existence (measured, #16).
#     `-Zlocation-detail=none` drops all three fields.
#   * WHERE the sources lived: cargo's home, rustup's home, the registry index
#     directory, and the revision-named directory a git dependency is unpacked
#     into. Each is remapped to a fixed name, derived from what is on disk so
#     that a rev bump never needs an edit here.
#   * A freshly resolved dependency set — `--locked`.
#
# `-Z` on the pinned stable toolchain needs `RUSTC_BOOTSTRAP=1`, which ties the
# bytes to this exact toolchain. That is not a new exposure: a toolchain bump
# already changes every hash and is an epoch by definition (#8).
#
# RUSTFLAGS reaches only the wasm target, never build scripts or proc macros,
# because `--target` is passed; `cargo test` and native builds are untouched.
#
# Usage: contract-build.sh <dir> <crate-name> [extra cargo args…]
#        contract-build.sh --flags <dir>     print the RUSTFLAGS and exit
#
# `--flags` exists so that anything else building wasm from this repo — the
# timing harness in particular — measures the flags a contract actually ships
# with, from this one definition, rather than a second copy of them.
set -euo pipefail

# The binaryen release these bytes were produced with. Changing it changes every
# contract hash, which is a network EPOCH — see #39.
WANT_BINARYEN=125
# Measured in #39: --enable-nontrapping-float-to-int --enable-sign-ext makes
# wasm-opt write nothing at all, with no error and no exit status. These two are
# the set that works.
WASM_OPT_FLAGS=(--enable-bulk-memory-opt --enable-bulk-memory)

flags_only=
if [ "${1:-}" = --flags ]; then flags_only=1; dir=$2; crate=; shift 2
else dir=$1; crate=$2; shift 2; fi
cd "$(dirname "$0")/$dir"

[ -n "$flags_only" ] || command -v shasum >/dev/null ||
  { echo "contract-build: shasum is required" >&2; exit 1; }

cargo_home=${CARGO_HOME:-$HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}

# RUSTFLAGS is split on whitespace, so a path with a space in it would not be
# remapped — and the build would succeed, with different bytes. Refuse instead:
# a remap that silently did not happen is the exact failure this file exists to
# prevent, and it would show up as an unexplained hash.
for path in "$PWD" "$(cd .. && pwd)" "$cargo_home" "$rustup_home"; do
  case $path in *[[:space:]]*)
    echo "contract-build: cannot remap a path containing a space: $path" >&2
    echo "contract-build: build from a path without spaces" >&2
    exit 1 ;;
  esac
done

# ORDER MATTERS, AND IT IS LAST-MATCH-WINS.
#
# rustc applies the LAST `--remap-path-prefix` whose prefix matches, so these
# are emitted general first and specific last. Emitted the other way round the
# specific ones never apply at all: `$cargo_home=/cargo` swallows the registry
# and the git checkouts, and every dependency path comes out as
# `/cargo/registry/src/index.crates.io-<index hash>/…` or
# `/cargo/git/checkouts/<pkg>-<hash>/<rev>/…` — which is #11's revision name
# back in the binary. `hash-gate.sh` gates this directly, on a build with the
# remaps and nothing else, because `-Zlocation-detail=none` removes the panic
# locations that would otherwise show it and would mask the bug completely.
remaps() {
  local d name
  printf ' --remap-path-prefix=%s=/rustup' "$rustup_home"
  printf ' --remap-path-prefix=%s=/cargo' "$cargo_home"
  # The repo root, then the contract's own workspace root inside it. Cargo
  # passes the local crate's files RELATIVE to the package directory, so these
  # two are belt and braces — they matter for anything that reports an absolute
  # path (`file!()` in a dependency, a build script, a future root workspace).
  printf ' --remap-path-prefix=%s=/contracts' "$(cd .. && pwd)"
  printf ' --remap-path-prefix=%s=/%s' "$PWD" "$dir"
  # The registry's source directory carries an index hash that differs between
  # cargo versions, so it is mapped away rather than left under `/cargo`.
  for d in "$cargo_home/registry/src"/*/; do
    [ -d "$d" ] || continue
    printf ' --remap-path-prefix=%s=/registry' "${d%/}"
  done
  # Cargo unpacks each git dependency into a directory named after the
  # REVISION, and that name reached the binary through panic locations, so two
  # revisions with identical code produced different wasm (#11). Map each
  # checkout to its package name alone. Last, so nothing above can swallow it.
  for d in "$cargo_home/git/checkouts"/*/*/; do
    [ -d "$d" ] || continue
    name=$(basename "$(dirname "$d")")       # freenet-prolly-e710ec000d97f4d5
    printf ' --remap-path-prefix=%s=/dep/%s' "${d%/}" "${name%-*}"
  done
}

# The gate's negative control has to RUN rather than be described, so the
# hardening is a real parameter of this script. The output is named
# `.unhardened.wasm`, and says so on stdout, because bytes built this way must
# never be mistaken for a contract's.
if [ -n "$flags_only" ]; then printf '%s\n' "-Zlocation-detail=none$(remaps)"; exit 0; fi

out=$dir
if [ -n "${CONTRACT_BUILD_UNHARDENED:-}" ]; then
  echo "contract-build: UNHARDENED build ($dir) — for the hash gate's control only" >&2
  export RUSTFLAGS=""
  out="$dir.unhardened"
elif [ -n "${CONTRACT_BUILD_REMAPS_ONLY:-}" ]; then
  # The remaps without the location strip. This is the only build in which the
  # remaps are OBSERVABLE: with `-Zlocation-detail=none` there are no panic
  # locations left to carry a path, so a broken remap looks exactly like a
  # working one. The gate builds this and reads the paths out of it.
  echo "contract-build: REMAPS-ONLY build ($dir) — for the hash gate's remap check only" >&2
  export RUSTC_BOOTSTRAP=1
  export RUSTFLAGS="$(remaps)"
  out="$dir.remaps-only"
else
  export RUSTC_BOOTSTRAP=1
  export RUSTFLAGS="-Zlocation-detail=none$(remaps)"
fi

# --locked: the dependency versions this repo recorded, never a freshly
# resolved set — the wasm hash is part of the contract's network key.
cargo build --locked --release --target wasm32-unknown-unknown "$@"
mkdir -p ../build
wasm=../build/$out.wasm
cp "target/wasm32-unknown-unknown/release/${crate//-/_}.wasm" "$wasm"

# wasm-opt -Os. ~18% off every contract, and ~120 KiB of contract rides every
# PUT at every hop (F28). Measured in #39: -Os takes essentially all of -Oz's
# gain (-Oz beats it by 12-184 BYTES) and costs no validation time, so the
# less aggressive level is the one that ships.
#
# NOT applied to the two control builds. `.remaps-only` exists so the hash gate
# can read PATHS out of the wasm — it is the only build in which a broken remap
# is observable — and an optimiser between the compiler and that check could
# move or drop the very strings it reads, turning a real gate into a silent
# one. `.unhardened` is left alone for the same reason: a control is only a
# control if nothing else happened to it.
# `CONTRACT_BUILD_NO_WASM_OPT` produces the same compile with the optimiser
# skipped. It exists for the differential gate: once -Os is what ships, "does
# wasm-opt change behaviour" can only be asked against a reference built the
# same way in every OTHER respect, and rebuilding one with different RUSTFLAGS
# would be comparing two things at once. The output is still written to
# `build/<dir>.wasm`, so a caller that wants both must move the first aside —
# `wasmopt-check.sh` does exactly that.
if [ -n "${CONTRACT_BUILD_NO_WASM_OPT:-}" ]; then
  echo "contract-build: wasm-opt SKIPPED ($dir) — reference build, not a contract" >&2
elif [ "$out" = "$dir" ]; then
  command -v wasm-opt >/dev/null ||
    { echo "contract-build: wasm-opt is required (binaryen $WANT_BINARYEN)" >&2; exit 1; }
  # PINNED, and asserted. wasm-opt's output is reproducible for a given
  # binaryen but NOT across versions, so an unpinned tool would silently move
  # every contract hash — a network epoch — on somebody's next brew upgrade.
  got=$(wasm-opt --version | awk '{print $NF}')
  [ "$got" = "$WANT_BINARYEN" ] ||
    { echo "contract-build: binaryen $got, expected $WANT_BINARYEN." >&2
      echo "contract-build: a different binaryen produces different bytes, and these bytes" >&2
      echo "contract-build: are a network key. Install binaryen $WANT_BINARYEN or change" >&2
      echo "contract-build: WANT_BINARYEN here, which is an EPOCH." >&2
      exit 1; }
  before=$(wc -c < "$wasm" | tr -d ' ')
  # wasm-opt FAILS SILENTLY on a feature flag it does not accept: no error, no
  # exit status, and NO OUTPUT FILE. `-o` to a temporary and a check that the
  # file exists is what makes that a failure instead of a build that quietly
  # ships the previous artefact.
  wasm-opt -Os "${WASM_OPT_FLAGS[@]}" "$wasm" -o "$wasm.opt"
  [ -s "$wasm.opt" ] ||
    { echo "contract-build: wasm-opt produced no output for $dir." >&2
      echo "contract-build: it exits 0 and writes nothing when a feature flag is wrong." >&2
      rm -f "$wasm.opt"; exit 1; }
  after=$(wc -c < "$wasm.opt" | tr -d ' ')
  [ "$after" -lt "$before" ] ||
    { echo "contract-build: wasm-opt did not shrink $dir ($before -> $after bytes)." >&2
      echo "contract-build: that is not an optimisation; something is wrong with the flags." >&2
      rm -f "$wasm.opt"; exit 1; }
  mv "$wasm.opt" "$wasm"
  echo "build/$out.wasm wasm-opt -Os: $before -> $after bytes ($(( (before - after) * 1000 / before ))‰ smaller), binaryen $got" >&2
fi

echo "build/$out.wasm $(wc -c < "$wasm" | tr -d ' ') bytes sha256=$(shasum -a 256 "$wasm" | cut -c1-16)"
