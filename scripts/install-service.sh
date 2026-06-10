#!/bin/bash
# Install vox as autostart service on Linux / macOS
# Usage: bash scripts/install-service.sh
set -euo pipefail

VOX_BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/vox"

if [ ! -f "$VOX_BIN" ]; then
    echo "Building vox release first..."
    (cd "$(dirname "$0")/.." && cargo build --release)
fi

echo "Installing vox from: $VOX_BIN"

case "$(uname -s)" in
    Darwin)
        PLIST="$HOME/Library/LaunchAgents/com.vox.vox.plist"
        mkdir -p "$HOME/Library/LaunchAgents"
        cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.vox.vox</string>
    <key>ProgramArguments</key>
    <array>
        <string>$VOX_BIN</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>
EOF
        launchctl load "$PLIST"
        echo "✅ Installed LaunchAgent at $PLIST"
        ;;
    Linux)
        UNIT="$HOME/.config/systemd/user/vox.service"
        mkdir -p "$HOME/.config/systemd/user"
        cat > "$UNIT" <<EOF
[Unit]
Description=vox - Voice I/O companion for CLI AI agents

[Service]
ExecStart=$VOX_BIN
Restart=on-failure

[Install]
WantedBy=default.target
EOF
        systemctl --user daemon-reload
        systemctl --user enable vox.service
        systemctl --user start vox.service
        echo "✅ Installed systemd user service at $UNIT"
        ;;
    *)
        echo "Unsupported platform: $(uname -s)"
        exit 1
        ;;
esac

echo "✅ vox autostart installed!"
