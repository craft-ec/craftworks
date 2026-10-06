#!/usr/bin/env bash
# FreenetTube: the page plus the SAME files the Craftworks app ships (no copies kept in the repo: copied at build).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); app=$(cd "$here/../.." && pwd); out="$here/www"
rm -rf "$out"; mkdir -p "$out"
cp "$here/index.html" "$here/video.json" "$out/"
for p in node files video-player player-chrome; do cp "$app/packages/$p.js" "$out/"; done
cp "$app/packages/build/craftworks_core.js" "$app/packages/build/craftworks_core_bg.wasm" "$app/packages/build/identity.wasm" "$out/"
cp "$app/contracts/sealed.wasm" "$app/contracts/piece.wasm" "$out/"
du -sh "$out" | cut -f1
