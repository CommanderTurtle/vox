#!/bin/bash
# Package vox for local distribution on the current platform.
# Usage: bash scripts/package.sh
#
# For cross-platform builds, use the GitHub Actions release workflow
# (.github/workflows/release.yml) which builds Windows/macOS/Linux in parallel.
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)"/\1/')"
OS_TAG="$(uname -s)"
case "$OS_TAG" in
    MINGW*|MSYS*|CYGWIN*) EXT=".exe"; ARCHIVE="zip"; ARTIFACT="dist/vox-v${VERSION}-windows-x86_64" ;;
    Darwin)               EXT="";    ARCHIVE="tar.gz"; ARTIFACT="dist/vox-v${VERSION}-macos-$(uname -m)" ;;
    Linux)                EXT="";    ARCHIVE="tar.gz"; ARTIFACT="dist/vox-v${VERSION}-linux-$(uname -m)" ;;
    *) echo "Unknown OS: $OS_TAG"; exit 1 ;;
esac

echo "=== Packaging vox v$VERSION ($OS_TAG) ==="
cargo build --release

BIN="target/release/vox${EXT}"
if [ ! -f "$BIN" ]; then
    echo "❌ Release binary not found: $BIN"; exit 1
fi

STAGE="dist/stage"
rm -rf "$STAGE"
mkdir -p "$STAGE/config-example"
cp "$BIN" "$STAGE/"
cp README.md "$STAGE/" 2>/dev/null || true
cp LICENSE "$STAGE/" 2>/dev/null || true
cp src/config/defaults.toml "$STAGE/config-example/config.toml"

mkdir -p dist
rm -f "${ARTIFACT}.${ARCHIVE}"
if [ "$ARCHIVE" = "zip" ]; then
    # Windows: prefer 7z, fall back to PowerShell Compress-Archive (absolute paths).
    if command -v 7z &>/dev/null; then
        7z a "${ARTIFACT}.zip" "$STAGE"/*
    else
        STAGE_ABS="$(cd "$STAGE" && pwd -W 2>/dev/null || pwd)"
        OUT_ABS="$(pwd -W 2>/dev/null || pwd)/${ARTIFACT}.zip"
        powershell -NoProfile -c "Compress-Archive -Path '${STAGE_ABS}/*' -DestinationPath '${OUT_ABS}' -Force"
    fi
else
    tar czf "${ARTIFACT}.tar.gz" -C "$STAGE" .
fi
rm -rf "$STAGE"

echo "✅ Created ${ARTIFACT}.${ARCHIVE}"
