#!/bin/sh
# Build the Windows release files (B97) into dist/:
#
#   Citylive2-Setup-<ver>.exe              the NSIS per-user installer
#   Citylive2-<ver>-windows-x64.zip        portable: the two programs, LICENSE and a note
#
#   bin/build-installer.sh              # MSVC build (bin/build-windows.sh), then package
#   bin/build-installer.sh --skip-build # package dist/windows as it is
#
# Needs makensis (NSIS) and zip. The version is the workspace version in Cargo.toml.
set -eu
cd "$(dirname "$0")/.."
ROOT=$PWD
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$VER" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }
command -v makensis >/dev/null 2>&1 || { echo "makensis (NSIS) is missing" >&2; exit 1; }

if [ "${1:-}" != "--skip-build" ]; then
    sh bin/build-windows.sh msvc
fi
SRC="$ROOT/dist/windows"
for f in citylive.exe citylive-settings.exe; do
    [ -f "$SRC/$f" ] || { echo "missing $SRC/$f (run bin/build-windows.sh first)" >&2; exit 1; }
done

mkdir -p dist
makensis -V2 -DVERSION="$VER" -DSRCDIR="$SRC" -DICON="$ROOT/assets/windows/citylive.ico" \
    -DLICENSE="$ROOT/LICENSE" -DOUTFILE="$ROOT/dist/Citylive2-Setup-$VER.exe" packaging/windows/installer.nsi
echo "wrote dist/Citylive2-Setup-$VER.exe ($(du -h "dist/Citylive2-Setup-$VER.exe" | cut -f1))"

# Portable zip
P=$(mktemp -d "${TMPDIR:-/tmp}/citylive-2-zip.XXXXXX")
trap 'rm -rf "$P"' EXIT INT TERM
mkdir -p "$P/Citylive 2"
cp "$SRC/citylive.exe" "$SRC/citylive-settings.exe" "$P/Citylive 2/"
cp LICENSE "$P/Citylive 2/LICENSE.txt"
cat > "$P/Citylive 2/README.txt" <<EOF
Citylive 2 $VER (portable)

Run citylive.exe to start the wallpaper (it lives in the tray). citylive-settings.exe opens the settings.
This portable copy does not update itself and does not add Start menu entries; use the installer
(Citylive2-Setup-$VER.exe) for those.

Windows SmartScreen may say "Windows protected your PC": click More info, then Run anyway.
https://github.com/Deluxescout1/citylive-2
EOF
( cd "$P" && rm -f "$ROOT/dist/Citylive2-$VER-windows-x64.zip" && zip -qr "$ROOT/dist/Citylive2-$VER-windows-x64.zip" "Citylive 2" )
echo "wrote dist/Citylive2-$VER-windows-x64.zip ($(du -h "dist/Citylive2-$VER-windows-x64.zip" | cut -f1))"
