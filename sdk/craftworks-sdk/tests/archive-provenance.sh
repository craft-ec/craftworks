#!/usr/bin/env bash
# THE gate for how `rev` gets into the wasm.
#
# The builder fetches this repo with `git archive <rev> | tar -x`, so the tree
# it compiles has no `.git` and `build.rs` cannot ask git. The rev arrives
# instead through git's `export-subst`, which rewrites `$Format:%H$` in
# `src/build_rev.txt` — a mechanism that fails SILENTLY if `.gitattributes`
# is lost or the placeholder is edited, leaving `rev = unknown` and a consumer
# whose mismatch warning can never do anything but fire.
#
# The unit tests cannot see that: they run in a checkout, where `git rev-parse`
# answers and the placeholder is never used. This builds the way the builder
# builds and asserts the archive names its own commit.
set -euo pipefail
cd "$(dirname "$0")/.."
want_full=$(git rev-parse HEAD)
want=$(git rev-parse --short=7 HEAD)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git archive HEAD | tar -x -C "$work"

[ -d "$work/.git" ] && { echo "FAIL: the archive has a .git — this is not the builder's case"; exit 1; }
stamped=$(cat "$work/src/build_rev.txt")
[ "$stamped" = "$want_full" ] || {
  echo "FAIL: export-subst did not substitute. src/build_rev.txt holds '$stamped', wanted $want_full"
  echo "      check .gitattributes still marks src/build_rev.txt export-subst"
  exit 1
}

(cd "$work" && ./build.sh >/dev/null)
got=$(cd "$work" && node --input-type=module -e '
import { wrap } from "./js/wrap.js";
import { createRequire } from "node:module";
const sdk = wrap(createRequire(import.meta.url)("./pkg/node/craftworks_sdk.js"));
process.stdout.write(sdk.buildInfo().rev);
')
[ "$got" = "$want" ] || { echo "FAIL: a build with no .git reported rev '$got', wanted '$want'"; exit 1; }
echo "archive provenance ok: a build with no .git named its own commit ($got)"
