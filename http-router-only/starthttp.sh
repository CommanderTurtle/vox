#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

[[ -x target/release/vox-http ]] || {
  printf 'Missing release binary. Run ./setup first.\n' >&2
  exit 1
}

export RUST_LOG="${RUST_LOG:-vox_http=info,tower_http=info}"
exec ./target/release/vox-http
