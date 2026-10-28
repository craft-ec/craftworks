#!/usr/bin/env bash
# PUBLISH a built tree to ONE private test node: publish-to.sh <tree> [port, default 17691]
# Carries the identity builds the site before knows (PREVIOUS_SITE) and sends only what that site does not hold
# (PUBLISHED_MANIFEST: a PUT of a piece the node held already went unanswered — 10-28, fx: 971 of 1516, every try).
# A publish that fails stops here: a probe run on the build before tests nothing.
set -uo pipefail
TREE=${1:?the tree to publish}; PORT=${2:-17691}
SITE=${SITE:-EckhKsQWCG6hSJqhRvxXqBEkFMBCm41YHTKnKRmMznpJ}
here="$(cd "$(dirname "$0")" && pwd)"
OUT=$(mktemp -d)
bash "$here/../previous-site.sh" "http://127.0.0.1:$PORT/v1/contract/web/$SITE" "$OUT/previous" > /dev/null || { echo "PUBLISH: the site before could not be read on $PORT"; exit 1; }
(cd "$TREE" && PUBLISHED_MANIFEST="$OUT/previous/manifest.json" PREVIOUS_SITE="$OUT/previous" CRAFTWORKS_TOKEN=off lockf -t 600 /tmp/craftworks-realnet.lock \
  ./target/debug/publish-craftworks "ws://127.0.0.1:$PORT/v1/contract/command?encodingProtocol=native" . > "$OUT/pub.log" 2>&1)
rc=$?
echo "PUBLISH=$rc $(grep -E '^site craftworks' "$OUT/pub.log")"
[ "$rc" = 0 ] || { tail -2 "$OUT/pub.log"; exit 1; }
rm -rf "$OUT"
