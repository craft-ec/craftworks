#!/usr/bin/env bash
# THE NODES' OWN LOGS for a probe run's window, beside its page traces: every line of fx/fy/fz's freenet logs (errors
# and all) stamped at or after <since> (UTC, ISO: 2026-10-29T07:10:00), into <dir>/<node>.log.
#   NODE_LOGS="fx=<log dir> fy=<log dir> fz=<log dir>" tools/release-probes/node-logs.sh <since> <dir>
set -uo pipefail
SINCE=${1:?since, UTC ISO}; DIR=${2:?out dir}; mkdir -p "$DIR"
for pair in ${NODE_LOGS:?NODE_LOGS: name=log-dir pairs}; do
  name=${pair%%=*}; d=${pair#*=}
  # (Each hour's file from the window's start: the names carry the hour.)
  for f in $(ls "$d"/freenet.*.log 2>/dev/null | sort); do
    h=$(basename "$f" | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9]{2}')
    [ -n "$h" ] && [[ "${h:0:10}T${h:11:2}" < "${SINCE:0:13}" ]] && continue
    awk -v s="$SINCE" 'substr($0,1,19) >= s' "$f"
  done | sort -s -k1,1 > "$DIR/$name.log"
  echo "$name: $(wc -l < "$DIR/$name.log") line(s)"
done
