#!/bin/sh
# Build and install the org.citylive.frames QML module for plasmashell.
#
#   plasma/frames/build-install.sh
#
# User-local: ~/.local/lib/qt6/qml/org/citylive/frames/ (no sudo, nothing
# outside $HOME). Needs cmake, g++ and the Qt 6 base + declarative packages
# (Arch: cmake qt6-base qt6-declarative).
#
# Overridable: BUILD_DIR (default ./build), PREFIX (default ~/.local),
# QML_INSTALL_DIR (default $PREFIX/lib/qt6/qml).
set -eu

HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BUILD=${BUILD_DIR:-$HERE/build}
PREFIX=${PREFIX:-$HOME/.local}
QMLDIR=${QML_INSTALL_DIR:-$PREFIX/lib/qt6/qml}
JOBS=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)

command -v cmake >/dev/null 2>&1 \
    || { echo "cmake is missing: sudo pacman -S --needed cmake" >&2; exit 1; }
command -v g++ >/dev/null 2>&1 \
    || { echo "g++ is missing: sudo pacman -S --needed gcc" >&2; exit 1; }

echo "== configuring ($BUILD) =="
cmake -S "$HERE" -B "$BUILD" \
      -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_INSTALL_PREFIX="$PREFIX" \
      -DQML_INSTALL_DIR="$QMLDIR"

echo "== building =="
cmake --build "$BUILD" --parallel "$JOBS"

echo "== installing to $QMLDIR/org/citylive/frames =="
cmake --install "$BUILD"

DEST="$QMLDIR/org/citylive/frames"
[ -f "$DEST/qmldir" ] || { echo "install failed: no $DEST/qmldir" >&2; exit 1; }

echo "installed:"
ls -1 "$DEST"
echo
echo "plasmashell imports it with QML_IMPORT_PATH=$QMLDIR"
echo "citylive-iso --install-plasma writes that for plasmashell (and for the next login)."
