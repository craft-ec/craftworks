#!/usr/bin/env bash
# PUBLISH the Craftworks app on freenet THROUGH B (the always-on test node), with B's signer. The same rules as every
# real-network run: the ONE shared lock (one session on B at a time), ONE SSH tunnel on this machine's loopback killed
# by PID, nothing on B but the client API's requests.
#
#   ./publish.sh      build, then publish every package (again: a node may have evicted one) and the two sites
set -euo pipefail
root="$(cd "$(dirname "$0")" && pwd)"
HOST=${REALNET_HOST:-root@46.224.172.252}
T=${REALNET_TUNNEL:-17619}   # not 17609: the owner's standing demo tunnel
BR=${REALNET_B_REMOTE:-7509}
LOCK=${REALNET_LOCK:-/tmp/craftworks-realnet.lock}
cd "$root"
./build.sh
cargo build -q -p craftworks-publish
tool="${CARGO_TARGET_DIR:-$root/target}/debug/publish-craftworks"

if ! mkdir "$LOCK" 2>/dev/null; then
  hpid=$(sed -n 's/^pid=//p' "$LOCK/owner" 2>/dev/null)
  if [ -n "$hpid" ] && kill -0 "$hpid" 2>/dev/null; then echo "REFUSED  another real-network run holds $LOCK: $(tr '\n' ' ' <"$LOCK/owner")"; exit 3; fi
  echo "the lock was left by a run that is gone; taking it"; rm -rf "$LOCK"; mkdir "$LOCK"
fi
printf 'pid=%s\ncwd=%s\nwhat=publish craftworks\nsince=%s\n' "$$" "$root" "$(date -u +%FT%TZ)" > "$LOCK/owner"
tunnel=""
cleanup() { [ -n "$tunnel" ] && kill "$tunnel" 2>/dev/null; rm -rf "$LOCK"; }
trap cleanup EXIT
if lsof -nP -iTCP:"$T" -sTCP:LISTEN >/dev/null 2>&1; then echo "REFUSED  something already listens on $T"; exit 3; fi
ssh -N -o BatchMode=yes -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 -L "127.0.0.1:$T:127.0.0.1:$BR" "$HOST" &
tunnel=$!
for _ in $(seq 1 30); do lsof -nP -iTCP:"$T" -sTCP:LISTEN >/dev/null 2>&1 && break; sleep 1; done
echo "tunnel pid $tunnel: 127.0.0.1:$T -> B 127.0.0.1:$BR"
# EVERY package is sent, each time: a node that evicted one (B keeps a limited number of contracts) has it again, and
# a package already there is the same immutable state (nothing changes). Skipping what the live manifest names left
# pages waiting forever on a package B had dropped.
out=$("$tool" "ws://127.0.0.1:$T/v1/contract/command?encodingProtocol=native" "$root" | tee /dev/stderr)
sed -n 's/^SITE //p' <<<"$out" > .published-site
