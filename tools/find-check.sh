#!/usr/bin/env bash
# DELIVERY CHECK (docs/RELEASE.md §3): every piece of the LIVE site fetched through the OWNER's node — the question any
# user's lookup asks, from a node other than B — and per package, how many came back; a package with fewer than `k`
# cannot be rebuilt there, and a page that needs it runs the version before. Under the shared real-network lock.
#   tools/find-check.sh <out dir> [extra http base to compare, e.g. a fresh live node: http://127.0.0.1:17700]
set -uo pipefail
OUT=${1:?out dir}; CMP=${2:-}
HOST=${REALNET_HOST:-root@46.224.172.252}; LOCK=${REALNET_LOCK:-/tmp/craftworks-realnet.lock}
SITE=B8Y3M8Z3eDzXoxKDmJnU72ysWGzQ9xjpYWGkbkMaUiZr
here="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$OUT"
mkdir "$LOCK" 2>/dev/null || { echo "BUSY $(tr '\n' ' ' < "$LOCK/owner" 2>/dev/null)"; exit 3; }
printf 'pid=%s\nwhat=find check of the live site\nsince=%s\n' "$$" "$(date -u +%FT%TZ)" > "$LOCK/owner"
ssh -N -o BatchMode=yes -o ExitOnForwardFailure=yes -L 127.0.0.1:17619:127.0.0.1:7509 -L 127.0.0.1:17620:127.0.0.1:7510 "$HOST" & tun=$!
trap 'kill $tun 2>/dev/null; rm -rf "$LOCK"' EXIT
until lsof -nP -iTCP:17620 -sTCP:LISTEN >/dev/null 2>&1; do sleep 1; done
bash "$here/previous-site.sh" "http://127.0.0.1:17619/v1/contract/web/$SITE" "$OUT/site"
python3 - "$OUT/site" > "$OUT/pieces.txt" <<'PY'
import json, glob, sys
for f in glob.glob(sys.argv[1] + "/p/*.json"):
    e = json.load(open(f))
    for p in e["pieces"]: print(e["file"], e["k"], len(e["pieces"]), p["address"])
PY
fetch() { xargs -P 48 -L 1 bash -c 'c=$(curl -s -o /dev/null -m 300 -w "%{http_code}" "'"$1"'/v1/contract/web/$3/piece"); echo "$0 $1 $2 $3 $c"' < "$OUT/pieces.txt"; }
fetch http://127.0.0.1:17620 > "$OUT/owner.txt"
[ -n "$CMP" ] && fetch "$CMP" > "$OUT/compare.txt"
python3 - "$OUT" <<'PY'
import sys, os, collections
def report(name, path):
    by = collections.defaultdict(lambda: [0, 0, 0])
    for l in open(path):
        f, k, n, a, c = l.split()
        b = by[f]; b[0] = int(k); b[1] = int(n); b[2] += c == "200"
    bad = sorted((f, k, n, got) for f, (k, n, got) in by.items() if got < k)
    print(f"{name}: packages {len(by)}; pieces fetched {sum(v[2] for v in by.values())} of {sum(v[1] for v in by.values())}; UNREBUILDABLE {len(bad)}")
    for f, k, n, got in bad: print(f"  {f}: {got} of {n} (needs {k})")
    return len(bad)
bad = report("owner node", sys.argv[1] + "/owner.txt")
if os.path.exists(sys.argv[1] + "/compare.txt"): report("compare node", sys.argv[1] + "/compare.txt")
sys.exit(1 if bad else 0)
PY
