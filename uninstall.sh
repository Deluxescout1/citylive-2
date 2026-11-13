#!/bin/sh
# Citylive 2 uninstaller for Linux. Removes the programs, the systemd user service,
# the menu entry, the links in ~/.local/bin and the Plasma module / GNOME extension.
# Your settings and saved worlds stay (so a reinstall picks the same city up) unless
# you pass --purge.
#
#   sh ~/.local/share/citylive-2/uninstall.sh [--purge]
set -eu

DATA="${XDG_DATA_HOME:-$HOME/.local/share}/citylive-2"
BIN="$HOME/.local/bin"
UNITDIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
APPDIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
UNIT=citylive-2.service
PURGE=0
[ "${1:-}" = "--purge" ] && PURGE=1
have() { command -v "$1" >/dev/null 2>&1; }

if have systemctl; then
    systemctl --user disable --now "$UNIT" 2>/dev/null || true
fi
rm -f "$UNITDIR/$UNIT"
have systemctl && systemctl --user daemon-reload 2>/dev/null || true

# Give Plasma its previous wallpaper back before the program that knows it goes away.
if [ -f "$DATA/plasma-wallpaper" ] && [ -x "$DATA/bin/citylive-iso" ]; then
    "$DATA/bin/citylive-iso" --uninstall-plasma || true
fi
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/plasma/wallpapers/org.citylive.iso" 2>/dev/null || true
rm -rf "$HOME/.local/lib/qt6/qml/org/citylive/frames" 2>/dev/null || true
rmdir "$HOME/.local/lib/qt6/qml/org/citylive" 2>/dev/null || true

rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/citylive-iso@citylive.org"
rm -f "$APPDIR/citylive-2-settings.desktop"
rm -f "$BIN/citylive-2" "$BIN/citylive-2-settings"
rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/citylive-iso/update"
rm -rf "$DATA"

if [ "$PURGE" = 1 ]; then
    rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/citylive-iso" "${XDG_DATA_HOME:-$HOME/.local/share}/citylive-iso" \
           "${XDG_CACHE_HOME:-$HOME/.cache}/citylive-iso"
    echo "Citylive 2 removed, with its settings and saved worlds."
else
    echo "Citylive 2 removed. Settings and saved worlds were kept"
    echo "(~/.config/citylive-iso, ~/.local/share/citylive-iso); add --purge to delete them too."
fi
