#!/usr/bin/env bash
# Reproducible build of tail.wasm, with the one shared flag set (../contract-build.sh).
set -euo pipefail
exec "$(dirname "$0")/../contract-build.sh" tail craftec-tail-contract "$@"
