#!/bin/sh
# Cross-compile the Windows programs on Linux (B95).
#
#   bin/build-windows.sh            # MSVC-ABI release build (cargo-xwin) -> dist/windows/
#   bin/build-windows.sh gnu        # mingw build (fallback)             -> dist/windows-gnu/
#   bin/build-windows.sh check      # cargo check only (gnu, fast)
#
# MSVC build needs: rustup target x86_64-pc-windows-msvc and `cargo install cargo-xwin`
# (it downloads the Windows CRT/SDK into .xwin-cache on first use). The gnu build needs
# mingw-w64 (x86_64-w64-mingw32-gcc and -windres). Both embed the icon, manifest and
# version info from assets/windows (windres), link the CRT statically and are not packed.
# Output: citylive.exe (wallpaper) and citylive-settings.exe (settings window).
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
MODE="${1:-msvc}"
SCOPE="systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 -p CPUQuota=400%"

case "$MODE" in
check)
    $SCOPE env PATH="$PATH" CARGO_TARGET_DIR=target-win cargo check --target x86_64-pc-windows-gnu -p wallpaper -p settings
    exit 0 ;;
gnu)
    TARGET=x86_64-pc-windows-gnu; TD=target-win; OUT=dist/windows-gnu
    $SCOPE env PATH="$PATH" CARGO_TARGET_DIR=$TD cargo build --release --target $TARGET -p wallpaper -p settings ;;
msvc)
    TARGET=x86_64-pc-windows-msvc; TD=${CARGO_TARGET_DIR:-target-msvc}; OUT=dist/windows
    $SCOPE env PATH="$PATH" CARGO_TARGET_DIR=$TD XWIN_CACHE_DIR="$PWD/.xwin-cache" \
        RUSTFLAGS="-C target-feature=+crt-static" \
        cargo xwin build --release --target $TARGET -p wallpaper -p settings ;;
*) echo "usage: $0 [msvc|gnu|check]"; exit 2 ;;
esac

mkdir -p "$OUT"
cp "$TD/$TARGET/release/citylive-iso.exe" "$OUT/citylive.exe"
cp "$TD/$TARGET/release/citylive-iso-settings.exe" "$OUT/citylive-settings.exe"
echo; echo "built ($MODE):"; ls -l "$OUT"/*.exe
for f in "$OUT"/*.exe; do
    echo "  $f imports:"; x86_64-w64-mingw32-objdump -p "$f" | awk '/DLL Name/ {print "    " $3}' | sort -u
done
