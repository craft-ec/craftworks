#!/usr/bin/env bash
# THE VERSION BEFORE, for a release: the site's manifest.json and every package entry (p/<sha16>.json) as the node
# serves them now, into <dir> — handed to the publisher as PREVIOUS_SITE=<dir>, so the release carries it as
# `previous` and a page that cannot fetch the new packages yet (minutes after a release) runs it instead.
#   tools/previous-site.sh <site http base: http://127.0.0.1:<port>/v1/contract/web/<site>> <dir>
set -euo pipefail
base=${1%/}; dir=$2
rm -rf "$dir"; mkdir -p "$dir/p"
curl -sf -m 120 -o "$dir/manifest.json" "$base/manifest.json"
python3 - "$dir/manifest.json" > "$dir/shas.txt" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
print("\n".join(sorted({v["sha256"][:16] for v in m["packages"].values()})))
PY
while read -r s; do curl -sf -m 120 -o "$dir/p/$s.json" "$base/p/$s.json"; done < "$dir/shas.txt"
echo "previous-site: $(wc -l < "$dir/shas.txt" | tr -d ' ') entries of $(python3 -c "import json;print(len(json.load(open('$dir/manifest.json'))['packages']))" ) packages in $dir"
