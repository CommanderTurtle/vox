#!/bin/bash
# Package vox for distribution
# Usage: bash scripts/package.sh
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION="$(cargo metadata --format-version=1 --no-deps | python3 -c "import sys,json;print(json.load(sys.stdin)['packages'][0]['version'])")"
echo "=== Packaging vox v$VERSION ==="

cargo build --release

BIN="target/release/vox"
if [ ! -f "$BIN" ]; then
    BIN="target/release/vox.exe"
fi

PKG_DIR="dist/vox-v$VERSION"
mkdir -p "$PKG_DIR"

cp "$BIN" "$PKG_DIR/"
cp README.md "$PKG_DIR/"
cp LICENSE 2>/dev/null || true

case "$(uname -s)" in
    Darwin)
        # Create .dmg
        if command -v create-dmg &>/dev/null; then
            create-dmg --volname "vox v$VERSION" "dist/vox-v$VERSION.dmg" "$PKG_DIR"
            echo "✅ Created dist/vox-v$VERSION.dmg"
        else
            echo "⚠️  create-dmg not found, skipping .dmg"
            echo "   brew install create-dmg"
        fi
        ;;
    Linux)
        # Create .AppImage stub (requires appimagetool)
        if command -v appimagetool &>/dev/null; then
            mkdir -p "$PKG_DIR/usr/bin" "$PKG_DIR/usr/share/applications"
            cp "$BIN" "$PKG_DIR/usr/bin/"
            cat > "$PKG_DIR/usr/share/applications/vox.desktop" <<DESKTOP
[Desktop Entry]
Name=vox
Comment=Voice I/O companion for CLI AI agents
Exec=vox
Terminal=false
Type=Application
Categories=Utility;
DESKTOP
            echo "✅ dist/vox-v$VERSION prepared (use appimagetool to create .AppImage)"
        else
            echo "✅ Release binary at $PKG_DIR/vox"
        fi
        ;;
    *)
        echo "✅ Release binary at $PKG_DIR/vox.exe"
        ;;
esac

echo "=== Done ==="
