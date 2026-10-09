#!/usr/bin/env bash
# The LIVE network's health bags (today and yesterday, UTC), read through B — under the ONE shared lock, one tunnel
# killed by PID (as publish.sh). Prints health-watch's report.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
HOST=${REALNET_HOST:-root@46.224.172.252}; T=${REALNET_TUNNEL:-17619}; LOCK=${REALNET_LOCK:-/tmp/craftworks-realnet.lock}
if ! mkdir "$LOCK" 2>/dev/null; then
  hpid=$(sed -n 's/^pid=//p' "$LOCK/owner" 2>/dev/null)
  if [ -n "$hpid" ] && kill -0 "$hpid" 2>/dev/null; then echo "BUSY another real-network run holds the lock: $(tr '\n' ' ' <"$LOCK/owner")"; exit 3; fi
  rm -rf "$LOCK"; mkdir "$LOCK"
fi
printf 'pid=%s\nwhat=health check\nsince=%s\n' "$$" "$(date -u +%FT%TZ)" > "$LOCK/owner"
tunnel=""; trap '[ -n "$tunnel" ] && kill "$tunnel" 2>/dev/null; rm -rf "$LOCK"' EXIT
ssh -N -o BatchMode=yes -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 -L "127.0.0.1:$T:127.0.0.1:7509" "$HOST" &
tunnel=$!
for _ in $(seq 1 30); do lsof -nP -iTCP:"$T" -sTCP:LISTEN >/dev/null 2>&1 && break; sleep 1; done
"$here/target/release/health-watch" "ws://127.0.0.1:$T/v1/contract/command?encodingProtocol=native" "$here/../../contracts/bag.wasm" "$@"
