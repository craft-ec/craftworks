#!/usr/bin/env bash
# THE UPGRADE TEST (docs/RELEASE.md §0) on fx/fy: the LIVE tree published, two lived-in accounts made there (a space,
# a channel, a row, a DM); the CANDIDATE published over it; each account unlocks with its PIN and every read and write
# is checked, with the login's `build` moves (RETIRED: yes) and fx/fy's delegate failures.
#   OLD=<live release tree> NEW=<candidate tree> FX_LOG=<fx log dir> FY_LOG=<fy log dir> tools/upgrade-gate/run.sh
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
OLD=${OLD:?the live release tree}; NEW=${NEW:?the candidate tree}; FX_LOG=${FX_LOG:?fx log dir}; FY_LOG=${FY_LOG:?fy log dir}
OUT=${OUT:-${TMPDIR:-/tmp}/craftworks-upgrade}; mkdir -p "$OUT"
WS='ws://127.0.0.1:17691/v1/contract/command?encodingProtocol=native'
CHROME=${CHROME:-$(ls -d ~/Library/Caches/ms-playwright/chromium_headless_shell-*/*/chrome-headless-shell | tail -1)}; export CHROME
# A publish that FAILS stops the gate: a phase run on the site left from before tests nothing (10-19: a candidate too
# large to publish, and the gate passed on the old code).
pub() { (cd "$1" && CRAFTWORKS_TOKEN=off lockf -t 600 /tmp/craftworks-realnet.lock "$1/target/debug/publish-craftworks" "$WS" "$1" > "$2" 2>&1); local rc=$?; echo "publish $(basename "$1"): $rc $(grep -E '^site craftworks' "$2")"; [ "$rc" = 0 ] || { echo "GATE: publishing $(basename "$1") failed: $(tail -1 "$2")"; exit 1; }; }
fails() { for d in "$FX_LOG" "$FY_LOG"; do f=$(ls -t "$d"/freenet.*.log | grep -v error | head -1); echo "$(basename "$(dirname "$d")") failed=$(grep -c 'Delegate operation failed' "$f") park=$(grep -c PARK_WORK_BUDGET "$f") cap=$(grep -c MAX_NETWORK_CONTRACT_OPS_PER_PARK "$f")"; done; }
F=u$(date +%H%M)
pub "$OLD" "$OUT/pub-old.log"
# THE VERSION JUST PUBLISHED (its manifest's siteV): what the phase's pages must run before anything is made there.
site=$(grep -oE "OPEN: /v1/contract/web/[^/]+" "$OUT/pub-old.log" | head -1 | sed 's|.*/||')
vOf() { curl -sf -m 120 "http://127.0.0.1:17691/v1/contract/web/$site/manifest.json" | shasum -a 256 | cut -c1-12; }
export WANT_V=$(vOf); echo "old version: $WANT_V"
UP_PHASE=old perl -e 'alarm 2400; exec @ARGV' node "$here/upgrade.mjs" "${F}a" > "$OUT/old.log" 2>&1
grep -E "^[0-9:]+ ([1-3] |   dm|FAILED)" "$OUT/old.log" | cut -c1-300
# The candidate takes every identity build the site before it knows, as a release does (PREVIOUS_SITE; 10-20: v409's
# build missing from a copied history, and every v409 member's PIN read as wrong).
bash "$here/../previous-site.sh" "http://127.0.0.1:17691/v1/contract/web/$site" "$OUT/previous" > /dev/null || { echo "GATE: the site before could not be read"; exit 1; }
PREVIOUS_SITE="$OUT/previous" pub "$NEW" "$OUT/pub-new.log"
export WANT_V=$(vOf); echo "new version: $WANT_V"
grep -E "^identity: every build" "$OUT/pub-new.log"
echo "before the new phase: $(fails | tr '\n' ' ')"
UP_PHASE=new perl -e 'alarm 2400; exec @ARGV' node "$here/upgrade.mjs" "${F}b" > "$OUT/new.log" 2>&1
grep -E "^[0-9:]+ ([4-9] |   [AB] rec|FAILED|RESULT)" -A12 "$OUT/new.log" | grep -vE "^\s+\[|journal" | cut -c1-400 | head -80
echo "after the new phase: $(fails | tr '\n' ' ')"
grep -q "FAILED" "$OUT/new.log" && exit 1
# A NEW identity build: each login must retire the old one (the same build: nothing to retire).
if ! cmp -s "$OLD/packages/build/identity.wasm" "$NEW/packages/build/identity.wasm"; then
  [ "$(grep -c 'RETIRED: yes' "$OUT/new.log")" -ge 2 ] || { echo "GATE: an account did not retire the old build"; exit 1; }
fi
# Every read: the before row, the DM and the after row read on the new build.
grep -q '"aBefore":"written on the old build"' "$OUT/new.log" && grep -q '"bDm":"a direct message on the old build"' "$OUT/new.log" && grep -q "B reads after: written on the new build" "$OUT/new.log" || { echo "GATE: a read failed"; exit 1; }
# Every app DRAWS on the candidate (a read answering is not a screen: 10-25, every app frame drew nothing for a day).
perl -e 'alarm 900; exec @ARGV' node "$here/frames.mjs" "${F}c" > "$OUT/frames.log" 2>&1; cat "$OUT/frames.log"
grep -q "^FRAMES: PASS" "$OUT/frames.log" || { echo "GATE: an app did not draw"; exit 1; }
echo "GATE: PASS"
