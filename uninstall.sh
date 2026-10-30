#!/usr/bin/env bash
set -euo pipefail

APP_ID="dev.ashraf.CosmicAppletOllama"
BIN_NAME="cosmic-applet-ollama"

BIN_DIR="${HOME}/.local/bin"
APPS_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"
DATA_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/${BIN_NAME}"

if pgrep -f "${BIN_DIR}/${BIN_NAME}" >/dev/null 2>&1; then
    echo "==> Stopping running $BIN_NAME instance..."
    pkill -f "${BIN_DIR}/${BIN_NAME}" || true
fi

rm -f "${BIN_DIR}/${BIN_NAME}"
rm -f "${APPS_DIR}/${APP_ID}.desktop"
rm -f "${ICON_DIR}/${APP_ID}-symbolic.svg"

if [ "${1:-}" = "--purge" ]; then
    rm -rf "$DATA_DIR"
    echo "==> Removed local token usage history ($DATA_DIR)"
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "Uninstalled $BIN_NAME from ~/.local."
if [ "${1:-}" != "--purge" ] && [ -d "$DATA_DIR" ]; then
    echo "Note: Token usage history was kept in $DATA_DIR (run './uninstall.sh --purge' to remove it)."
fi
