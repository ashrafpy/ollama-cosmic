#!/usr/bin/env bash
set -euo pipefail

APP_ID="dev.ashraf.CosmicAppletOllama"
BIN_NAME="cosmic-applet-ollama"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if command -v cargo >/dev/null 2>&1; then
    CARGO_BIN="cargo"
elif [ -x "$HOME/.cargo/bin/cargo" ]; then
    CARGO_BIN="$HOME/.cargo/bin/cargo"
else
    echo "Error: cargo not found. Install Rust via https://rustup.rs or your system package manager." >&2
    exit 1
fi

echo "==> Building $BIN_NAME (release)..."
"$CARGO_BIN" build --release

BIN_DIR="${HOME}/.local/bin"
APPS_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"

mkdir -p "$BIN_DIR" "$APPS_DIR" "$ICON_DIR"

echo "==> Installing binary to $BIN_DIR/$BIN_NAME"
install -Dm755 "target/release/$BIN_NAME" "$BIN_DIR/$BIN_NAME"

echo "==> Installing symbolic icons to $ICON_DIR/"
install -Dm644 "res/icons/${APP_ID}-symbolic.svg" "$ICON_DIR/${APP_ID}-symbolic.svg"
install -Dm644 "res/icons/${APP_ID}-active-symbolic.svg" "$ICON_DIR/${APP_ID}-active-symbolic.svg"

echo "==> Installing desktop entry to $APPS_DIR/${APP_ID}.desktop"
sed "s|^Exec=.*|Exec=${BIN_DIR}/${BIN_NAME}|" "res/${APP_ID}.desktop" > "$APPS_DIR/${APP_ID}.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

if pgrep -f "$BIN_NAME" >/dev/null 2>&1; then
    echo "==> Restarting running $BIN_NAME instance..."
    pkill -f "$BIN_NAME" || true
fi

echo ""
echo "Installed successfully!"
echo "To add to your COSMIC Panel:"
echo "  1. Open COSMIC Settings -> Desktop -> Panel -> Configure Panel Applets"
echo "  2. Click '+ Add Applet' and add 'Ollama Model Status'"
