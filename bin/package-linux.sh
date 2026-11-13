#!/bin/sh
# Build the Linux release files (B97) into dist/:
#
#   citylive-2-<ver>-linux-x86_64.tar.gz   the per-user install (install.sh) and what the self-updater downloads
#   citylive-2_<ver>_amd64.deb             system-wide package (no auto-enable; see the post-install note)
#
#   bin/package-linux.sh              # build (release) and package
#   bin/package-linux.sh --skip-build # package target/release as it is
#
# The version is the workspace version in Cargo.toml. Heavy build: runs in an 8 GB scope.
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$VER" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }
TD="${CARGO_TARGET_DIR:-target}"
OUT=dist
NAME="citylive-2-$VER-linux-x86_64"

if [ "${1:-}" != "--skip-build" ]; then
    systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 -p CPUQuota=400% \
        env PATH="$PATH" cargo build --release -p wallpaper -p settings -p present_x11 -p present_gnome
fi
for b in citylive-iso citylive-iso-settings citylive-iso-x11 citylive-iso-gnome; do
    [ -x "$TD/release/$b" ] || { echo "missing $TD/release/$b (build first)" >&2; exit 1; }
done

STAGE=$(mktemp -d "${TMPDIR:-/tmp}/citylive-2-package.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT INT TERM
T="$STAGE/$NAME"
mkdir -p "$T/bin" "$T/share" "$T/plasma/frames" "$T/gnome-extension" "$OUT"
for b in citylive-iso citylive-iso-settings citylive-iso-x11 citylive-iso-gnome; do
    install -m 755 "$TD/release/$b" "$T/bin/$b"
done
cp plasma/frames/CMakeLists.txt plasma/frames/cityframe.cpp plasma/frames/cityframe.h plasma/frames/build-install.sh plasma/frames/README.md "$T/plasma/frames/"
cp -R "gnome-extension/citylive-iso@citylive.org" "$T/gnome-extension/"
cp install.sh uninstall.sh LICENSE "$T/"
cp packaging/linux/README.txt "$T/README.txt"
chmod 755 "$T/install.sh" "$T/uninstall.sh"
echo "$VER" > "$T/VERSION"
# Menu icon from the Windows icon (its largest frame), when Pillow is around.
python3 - "$T/share/citylive-2.png" <<'PY' 2>/dev/null || echo "note: no menu icon (python3 with Pillow not available)"
import sys
from PIL import Image
im = Image.open("assets/windows/citylive.ico")
sizes = sorted(im.ico.sizes()) if hasattr(im, "ico") else [im.size]
im.size = sizes[-1]
im.load()
im.convert("RGBA").resize((256, 256)).save(sys.argv[1])
PY

# Normal permissions whatever the umask (a shared machine may use 007).
normalize() { find "$1" -type d -exec chmod 755 {} +; find "$1" -type f ! -perm -u+x -exec chmod 644 {} +; find "$1" -type f -perm -u+x -exec chmod 755 {} +; }
normalize "$T"

# Reproducible-ish tarball: sorted names, fixed owner, fixed mtime from the version.
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime='2026-01-01 00:00:00' \
    -C "$STAGE" -czf "$OUT/$NAME.tar.gz" "$NAME"
echo "wrote $OUT/$NAME.tar.gz ($(du -h "$OUT/$NAME.tar.gz" | cut -f1))"

# ------------------------------------------------------------------ .deb
D="$STAGE/deb"
mkdir -p "$D/DEBIAN" "$D/usr/lib/citylive-2/bin" "$D/usr/bin" "$D/usr/share/applications" \
         "$D/usr/lib/systemd/user" "$D/usr/share/doc/citylive-2" "$D/usr/share/citylive-2"
for b in citylive-iso citylive-iso-settings citylive-iso-x11 citylive-iso-gnome; do
    install -m 755 "$T/bin/$b" "$D/usr/lib/citylive-2/bin/$b"
done
ln -s ../lib/citylive-2/bin/citylive-iso "$D/usr/bin/citylive-2"
ln -s ../lib/citylive-2/bin/citylive-iso-settings "$D/usr/bin/citylive-2-settings"
cp -R "$T/plasma" "$T/gnome-extension" "$D/usr/share/citylive-2/"
[ -f "$T/share/citylive-2.png" ] && install -D -m 644 "$T/share/citylive-2.png" "$D/usr/share/icons/hicolor/256x256/apps/citylive-2.png"
install -m 644 LICENSE "$D/usr/share/doc/citylive-2/copyright"
install -m 644 CHANGELOG.md "$D/usr/share/doc/citylive-2/changelog"
gzip -9n "$D/usr/share/doc/citylive-2/changelog"
cat > "$D/usr/share/applications/citylive-2-settings.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Citylive 2 Settings
Comment=Settings for the Citylive 2 live wallpaper
Exec=/usr/lib/citylive-2/bin/citylive-iso-settings
Icon=citylive-2
Terminal=false
Categories=Settings;DesktopSettings;
EOF
cat > "$D/usr/lib/systemd/user/citylive-2.service" <<'EOF'
[Unit]
Description=Citylive 2 live wallpaper
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=/usr/lib/citylive-2/bin/citylive-iso
Restart=on-failure
RestartSec=5
Nice=10
MemoryHigh=1G

[Install]
WantedBy=graphical-session.target
EOF
cat > "$D/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
    echo "Citylive 2 is installed. To run it as your wallpaper:"
    echo "    systemctl --user enable --now citylive-2.service"
    echo "KDE Plasma: see /usr/share/citylive-2/plasma/frames/README.md for the Plasma module."
fi
exit 0
EOF
chmod 755 "$D/DEBIAN/postinst"
normalize "$D"
SIZE=$(du -sk "$D/usr" | cut -f1)
cat > "$D/DEBIAN/control" <<EOF
Package: citylive-2
Version: $VER
Section: x11
Priority: optional
Architecture: amd64
Installed-Size: $SIZE
Maintainer: Deluxescout1 <Deluxescout1@users.noreply.github.com>
Depends: libc6 (>= 2.31), libgcc-s1
Recommends: libwayland-client0, libxkbcommon0, libgl1
Homepage: https://github.com/Deluxescout1/citylive-2
Description: living isometric pixel-art city as a desktop wallpaper
 Citylive 2 grows a small isometric pixel-art city on your desktop, with the
 real sun, moon, weather and seasons for your location. This package installs
 the programs system-wide; enable the user service to start it:
 systemctl --user enable --now citylive-2.service
 Installed this way it is updated by your package manager, not by the
 built-in updater (which only updates the per-user install).
EOF
dpkg-deb --root-owner-group --build "$D" "$OUT/citylive-2_${VER}_amd64.deb" >/dev/null
echo "wrote $OUT/citylive-2_${VER}_amd64.deb ($(du -h "$OUT/citylive-2_${VER}_amd64.deb" | cut -f1))"
