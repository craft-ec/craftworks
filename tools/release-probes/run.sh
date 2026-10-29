#!/usr/bin/env bash
# THE RELEASE PROBES on the private nodes fx/fy/fz (17691/17692/17693): every state that writes, on the candidate
# (decided 10-29: tested on the nodes, not an offline harness). Run with the candidate published to fx and fy.
#   MLS_OLD=<built v446 tree> NODE_LOGS="fx=<dir> fy=<dir> fz=<dir>" tools/release-probes/run.sh [probe …]
# Every run keeps each page's WHOLE trace and the nodes' logs for its window in <OUT>/<probe>-trace/ — read, never re-run.
# k86-mls publishes MLS_OLD then this tree to fx, and the candidate again after it. Every probe page waits until it runs
# the build its node serves. Before a run: the nodes propagate (a publish via fx reached fy and fz in 34 s after a
# restart; nodes up for days had stopped — 10-28).
# A probe with no RESULT line FAILED (a crash, a stall, a probe gone): the verdict is per declared probe, never a tally.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; tree="$(cd "$here/../.." && pwd)"
CHROME=${CHROME:-$(ls -d ~/Library/Caches/ms-playwright/chromium_headless_shell-*/*/chrome-headless-shell | tail -1)}; export CHROME
OUT=${OUT:-${TMPDIR:-/tmp}/craftworks-probes}; mkdir -p "$OUT"
ALL="k86-reg k85-two k85-node k87-rot k87-resume k87-three remove-control-bk k88-join-kick k90-kick-closed k86-mls"
PROBES=${*:-$ALL}
pass=0; fail=0
for p in $PROBES; do
  [ -f "$here/$p.mjs" ] || { echo "$p: NO SUCH PROBE"; fail=$((fail + 1)); continue; }
  for port in 17691 17692 17693; do perl -e 'alarm 120; exec @ARGV' node "$here/logout.mjs" "$port" > /dev/null 2>&1; done
  t0=$(date +%s); since=$(date -u +%FT%T)
  if [ "$p" = k86-mls ]; then
    [ -n "${MLS_OLD:-}" ] || { echo "$p: FAIL (MLS_OLD — a built v446 tree — not given)"; fail=$((fail + 1)); continue; }
    OLD="$MLS_OLD" NEW="$tree" perl -e 'alarm 5400; exec @ARGV' node "$here/$p.mjs" "r$RANDOM" > "$OUT/$p.log" 2>&1
  else
    perl -e 'alarm 3600; exec @ARGV' node "$here/$p.mjs" "r$RANDOM" > "$OUT/$p.log" 2>&1
  fi
  # (Beside its page traces, <OUT>/<probe>-trace/: the nodes' own log lines for its window — NODE_LOGS set.)
  [ -n "${NODE_LOGS:-}" ] && bash "$here/node-logs.sh" "$since" "$OUT/$p-trace" > /dev/null
  r=$(grep -E "RESULT" "$OUT/$p.log" | tail -1)
  echo "$p $(($(date +%s) - t0))s ${r:-RESULT MISSING}" | cut -c1-300
  case "$r" in *"RESULT PASS"*) pass=$((pass + 1)) ;; *) fail=$((fail + 1)) ;; esac
  # (k86-mls left fx on the candidate it published; fy follows by the network.)
done
echo "PROBES: $pass passed, $fail failed, of $(echo $PROBES | wc -w | tr -d ' ') (logs in $OUT)"
[ "$fail" = 0 ]
