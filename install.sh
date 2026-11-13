#!/bin/sh
# Citylive 2 installer for Linux (x86_64). Per-user: nothing here needs root.
#
#   curl -fsSL https://raw.githubusercontent.com/Deluxescout1/citylive-2/main/install.sh | sh
#
# or, from an unpacked release tarball:  sh install.sh
#
# What it does
#   1. (download mode) fetches the latest release tarball and checks it against the
#      release's SHA256SUMS;
#   2. installs the programs to ~/.local/share/citylive-2 (and links to them from
#      ~/.local/bin);
#   3. writes a systemd *user* service, enables it for graphical sessions and starts it;
#   4. adds a "Citylive 2 Settings" entry to the application menu;
#   5. on KDE Plasma, offers to build the Plasma wallpaper module (needs cmake, g++
#      and the Qt 6 development packages) and to make Citylive 2 the wallpaper.
#
# Options
#   --version X.Y.Z    install that release instead of the latest (download mode)
#   --yes              answer yes to every question (Plasma module and wallpaper)
#   --no-plasma        do not touch Plasma
#   --install-plasma   only (re)build the Plasma module and set the wallpaper
#   --presenter NAME   plasma | wayland | gnome | x11 (default: chosen from your session)
#   --no-service       do not write or start the systemd service
#   --no-restart       install the files but do not (re)start the service
#   --update           used by the self-updater: no questions, no output noise
#   -h, --help         this text
#
# Config and saved worlds are never touched. Uninstall with uninstall.sh.
set -eu

REPO="Deluxescout1/citylive-2"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}/citylive-2"
BIN="$HOME/.local/bin"
UNITDIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
APPDIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
UNIT=citylive-2.service

WANT_VERSION=""
YES=0
NO_PLASMA=0
ONLY_PLASMA=0
PRESENTER=""
NO_SERVICE=0
NO_RESTART=0
UPDATE=0

say() { [ "$UPDATE" = 1 ] || printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

usage() { sed -n '2,/^set -eu/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || die "--version needs a value"; WANT_VERSION="$2"; shift ;;
        --yes|-y) YES=1 ;;
        --no-plasma) NO_PLASMA=1 ;;
        --install-plasma) ONLY_PLASMA=1 ;;
        --presenter) [ $# -ge 2 ] || die "--presenter needs a value"; PRESENTER="$2"; shift ;;
        --no-service) NO_SERVICE=1 ;;
        --no-restart) NO_RESTART=1 ;;
        --update) UPDATE=1; YES=1; NO_PLASMA=0 ;;
        --local) ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
    shift
done

[ "$(uname -s)" = Linux ] || die "this installer is for Linux; on Windows use the Citylive2-Setup .exe"
[ "$(uname -m)" = x86_64 ] || die "only x86_64 builds are published (this machine is $(uname -m))"
[ -n "${HOME:-}" ] || die "HOME is not set"

# ---------------------------------------------------------------- download mode
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd || echo "")
if [ -z "$HERE" ] || [ ! -x "$HERE/bin/citylive-iso" ]; then
    # Started through a pipe (curl | sh) or from a source checkout: fetch a release.
    if have curl; then
        fetch() { curl -fsSL "$1"; }
        fetch_to() { curl -fsSL -o "$2" "$1"; }
    elif have wget; then
        fetch() { wget -qO- "$1"; }
        fetch_to() { wget -qO "$2" "$1"; }
    else
        die "need curl or wget to download Citylive 2"
    fi
    have tar || die "need tar"
    have sha256sum || die "need sha256sum (coreutils)"
    if [ -z "$WANT_VERSION" ]; then
        say "Looking for the latest release..."
        WANT_VERSION=$(fetch "https://api.github.com/repos/$REPO/releases/latest" \
            | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' | head -n 1) \
            || die "could not reach GitHub"
        [ -n "$WANT_VERSION" ] || die "no release found (has one been published yet?)"
    fi
    WANT_VERSION=${WANT_VERSION#v}
    NAME="citylive-2-$WANT_VERSION-linux-x86_64"
    BASE="https://github.com/$REPO/releases/download/v$WANT_VERSION"
    TMP=$(mktemp -d "${TMPDIR:-/tmp}/citylive-2.XXXXXX")
    trap 'rm -rf "$TMP"' EXIT INT TERM
    say "Downloading Citylive 2 $WANT_VERSION..."
    fetch_to "$BASE/$NAME.tar.gz" "$TMP/$NAME.tar.gz" || die "download failed: $BASE/$NAME.tar.gz"
    fetch_to "$BASE/SHA256SUMS" "$TMP/SHA256SUMS" || die "download failed: $BASE/SHA256SUMS"
    WANT=$(awk -v n="$NAME.tar.gz" '{f=$2; sub(/^\*/, "", f)} f==n {print $1}' "$TMP/SHA256SUMS" | head -n 1)
    [ -n "$WANT" ] || die "SHA256SUMS has no entry for $NAME.tar.gz"
    GOT=$(sha256sum "$TMP/$NAME.tar.gz" | awk '{print $1}')
    [ "$WANT" = "$GOT" ] || die "checksum mismatch for $NAME.tar.gz (expected $WANT, got $GOT)"
    say "Checksum ok."
    tar -xzf "$TMP/$NAME.tar.gz" -C "$TMP"
    [ -x "$TMP/$NAME/bin/citylive-iso" ] || die "the tarball has an unexpected layout"
    # Hand over to the installer that ships in the tarball (same version as the files).
    set -- --local
    [ "$YES" = 1 ] && set -- "$@" --yes
    [ "$NO_PLASMA" = 1 ] && set -- "$@" --no-plasma
    [ "$ONLY_PLASMA" = 1 ] && set -- "$@" --install-plasma
    [ -n "$PRESENTER" ] && set -- "$@" --presenter "$PRESENTER"
    [ "$NO_SERVICE" = 1 ] && set -- "$@" --no-service
    [ "$NO_RESTART" = 1 ] && set -- "$@" --no-restart
    # Not exec: the trap must still remove the temporary folder afterwards.
    sh "$TMP/$NAME/install.sh" "$@"
    exit $?
fi

# ------------------------------------------------------------------ local mode
VERSION=$(cat "$HERE/VERSION" 2>/dev/null || echo "?")

ask() { # ask "question" -> 0 for yes
    [ "$YES" = 1 ] && return 0
    if [ -r /dev/tty ]; then
        printf '%s [Y/n] ' "$1" > /dev/tty
        read -r ans < /dev/tty || ans=""
    elif [ -t 0 ]; then
        printf '%s [Y/n] ' "$1"
        read -r ans || ans=""
    else
        return 1
    fi
    case "$ans" in n|N|no|NO) return 1 ;; *) return 0 ;; esac
}

is_kde() { case ":${XDG_CURRENT_DESKTOP:-}:${DESKTOP_SESSION:-}:" in *KDE*|*plasma*|*Plasma*) return 0 ;; esac; return 1; }
is_gnome() { case ":${XDG_CURRENT_DESKTOP:-}:" in *GNOME*|*Unity*|*ubuntu*|*Ubuntu*) return 0 ;; esac; return 1; }

# put SRC DEST MODE: copy beside the destination, then rename over it, so a running
# program is replaced atomically and a half-copied file is never seen.
put() {
    mkdir -p "$(dirname "$2")"
    cp "$1" "$2.new.$$"
    chmod "$3" "$2.new.$$"
    mv -f "$2.new.$$" "$2"
}

pick_presenter() {
    [ -n "$PRESENTER" ] && { echo "$PRESENTER"; return; }
    if is_kde && [ -f "$HERE/plasma/frames/build-install.sh" ]; then echo plasma
    elif is_gnome && [ -x "$HERE/bin/citylive-iso-gnome" ]; then echo gnome
    elif [ -z "${WAYLAND_DISPLAY:-}" ] && [ -n "${DISPLAY:-}" ] && [ -x "$HERE/bin/citylive-iso-x11" ]; then echo x11
    else echo wayland
    fi
}

write_unit() { # write_unit PRESENTER
    case "$1" in
        plasma) EXEC="$DATA/bin/citylive-iso --export-shm %t/citylive-iso/shm" ;;
        wayland) EXEC="$DATA/bin/citylive-iso" ;;
        gnome) EXEC="$DATA/bin/citylive-iso-gnome" ;;
        x11) EXEC="$DATA/bin/citylive-iso-x11" ;;
        *) die "unknown presenter '$1' (plasma, wayland, gnome or x11)" ;;
    esac
    mkdir -p "$UNITDIR"
    cat > "$UNITDIR/$UNIT.new.$$" <<EOF
[Unit]
Description=Citylive 2 live wallpaper
Documentation=https://github.com/$REPO
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=$EXEC
Restart=on-failure
RestartSec=5
# A wallpaper must never crowd the desktop.
Nice=10
MemoryHigh=1G

[Install]
WantedBy=graphical-session.target
EOF
    mv -f "$UNITDIR/$UNIT.new.$$" "$UNITDIR/$UNIT"
    echo "$1" > "$DATA/presenter"
}

plasma_module() {
    # Build and install org.citylive.frames, then let the program select the wallpaper plugin.
    have cmake && have g++ || {
        warn "the Plasma module needs cmake and g++ plus the Qt 6 development packages:"
        warn "  Debian/Ubuntu: sudo apt install cmake g++ qt6-base-dev qt6-base-private-dev qt6-declarative-dev"
        warn "  Fedora:        sudo dnf install cmake gcc-c++ qt6-qtbase-devel qt6-qtbase-private-devel qt6-qtdeclarative-devel"
        warn "  Arch:          sudo pacman -S --needed cmake gcc qt6-base qt6-declarative"
        warn "then run:  sh $DATA/install.sh --install-plasma"
        return 1
    }
    say "Building the Plasma module (a minute or two)..."
    BUILD_DIR="${TMPDIR:-/tmp}/citylive-2-frames-build.$$" \
        sh "$HERE/plasma/frames/build-install.sh" > "${TMPDIR:-/tmp}/citylive-2-plasma-build.log" 2>&1 \
        || { warn "the Plasma module did not build; the log is ${TMPDIR:-/tmp}/citylive-2-plasma-build.log"; rm -rf "${TMPDIR:-/tmp}/citylive-2-frames-build.$$"; return 1; }
    rm -rf "${TMPDIR:-/tmp}/citylive-2-frames-build.$$"
    cat "$HERE/plasma/frames/cityframe.cpp" "$HERE/plasma/frames/cityframe.h" "$HERE/plasma/frames/CMakeLists.txt" \
        | sha256sum | cut -c1-16 > "$DATA/plasma-frames.sum"
    return 0
}

plasma_wallpaper() {
    if ask "Make Citylive 2 your Plasma wallpaper now? (the desktop shell restarts once; undo with: citylive-iso --uninstall-plasma)"; then
        "$DATA/bin/citylive-iso" --install-plasma || { warn "could not select the wallpaper plugin"; return 1; }
        touch "$DATA/plasma-wallpaper"
    else
        say "Skipped. Later: $DATA/bin/citylive-iso --install-plasma"
    fi
}

# --install-plasma on its own (also the follow-up after installing Qt packages)
if [ "$ONLY_PLASMA" = 1 ]; then
    [ -x "$DATA/bin/citylive-iso" ] || die "install Citylive 2 first"
    HERE="$DATA"
    plasma_module && plasma_wallpaper
    [ "$NO_SERVICE" = 1 ] || { write_unit plasma; have systemctl && systemctl --user daemon-reload && systemctl --user restart "$UNIT" || true; }
    exit 0
fi

say "Installing Citylive 2 $VERSION to $DATA"
mkdir -p "$DATA/bin" "$BIN"
for f in "$HERE"/bin/*; do
    [ -f "$f" ] && put "$f" "$DATA/bin/$(basename "$f")" 755
done
for f in VERSION LICENSE README.txt install.sh uninstall.sh; do
    [ -f "$HERE/$f" ] && put "$HERE/$f" "$DATA/$f" 644
done
chmod 755 "$DATA/install.sh" "$DATA/uninstall.sh" 2>/dev/null || true
for d in plasma gnome-extension share; do
    if [ -d "$HERE/$d" ]; then
        rm -rf "$DATA/$d.new.$$"
        cp -R "$HERE/$d" "$DATA/$d.new.$$"
        rm -rf "$DATA/$d"
        mv "$DATA/$d.new.$$" "$DATA/$d"
    fi
done
ln -sf "$DATA/bin/citylive-iso" "$BIN/citylive-2"
ln -sf "$DATA/bin/citylive-iso-settings" "$BIN/citylive-2-settings"

# Application menu entry for the settings window.
mkdir -p "$APPDIR"
ICON=preferences-desktop-wallpaper
[ -f "$DATA/share/citylive-2.png" ] && ICON="$DATA/share/citylive-2.png"
cat > "$APPDIR/citylive-2-settings.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Citylive 2 Settings
Comment=Settings for the Citylive 2 live wallpaper
Exec=$DATA/bin/citylive-iso-settings
Icon=$ICON
Terminal=false
Categories=Settings;DesktopSettings;
EOF
have update-desktop-database && update-desktop-database "$APPDIR" 2>/dev/null || true

if [ "$UPDATE" = 1 ]; then
    # The self-updater: refresh the Plasma module only if its source changed.
    if [ -f "$DATA/plasma-frames.sum" ]; then
        NEW=$(cat "$HERE/plasma/frames/cityframe.cpp" "$HERE/plasma/frames/cityframe.h" "$HERE/plasma/frames/CMakeLists.txt" | sha256sum | cut -c1-16)
        [ "$NEW" = "$(cat "$DATA/plasma-frames.sum")" ] || plasma_module || true
    fi
    if [ "$NO_RESTART" = 0 ] && have systemctl && systemctl --user is-active --quiet "$UNIT" 2>/dev/null; then
        systemctl --user restart "$UNIT" || true
    fi
    exit 0
fi

P=$(pick_presenter)
if [ "$P" = plasma ] && [ "$NO_PLASMA" = 1 ]; then P=wayland; fi
if [ "$P" = plasma ]; then
    if ask "This is KDE Plasma. Build the Plasma wallpaper module now? (needs cmake, g++, Qt 6 dev packages)"; then
        if plasma_module; then
            plasma_wallpaper || true
        else
            P=wayland
        fi
    else
        P=wayland
        say "Skipped the Plasma module. Later: sh $DATA/install.sh --install-plasma"
    fi
fi
if [ "$P" = gnome ] && [ -d "$HERE/gnome-extension/citylive-iso@citylive.org" ]; then
    EXT="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/citylive-iso@citylive.org"
    mkdir -p "$(dirname "$EXT")"
    rm -rf "$EXT"
    cp -R "$HERE/gnome-extension/citylive-iso@citylive.org" "$EXT"
    have gnome-extensions && gnome-extensions enable citylive-iso@citylive.org 2>/dev/null || \
        say "Enable the GNOME extension 'CityLive Iso Wallpaper' after logging out and in again."
fi

if [ "$NO_SERVICE" = 1 ]; then
    say "Service skipped (--no-service). Run it with: $DATA/bin/citylive-iso"
elif have systemctl; then
    write_unit "$P"
    systemctl --user daemon-reload
    systemctl --user enable "$UNIT" >/dev/null 2>&1 || warn "could not enable the service"
    if [ "$NO_RESTART" = 0 ]; then
        systemctl --user restart "$UNIT" 2>/dev/null \
            || warn "the service is enabled but did not start (no graphical session?). It starts at your next login."
    fi
else
    warn "no systemctl: start it yourself with $DATA/bin/citylive-iso"
fi

case ":$PATH:" in *":$BIN:"*) ;; *) say "Note: $BIN is not in your PATH." ;; esac
say ""
say "Citylive 2 $VERSION is installed ($P presenter)."
say "  settings:  citylive-2-settings   (or the application menu)"
say "  status:    systemctl --user status $UNIT"
say "  logs:      journalctl --user -u $UNIT"
say "  remove:    sh $DATA/uninstall.sh"
