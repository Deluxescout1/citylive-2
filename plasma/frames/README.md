# org.citylive.frames — the shared-memory wallpaper item (B63)

A ~400-line C++17 / Qt 6 QML module with one type:

```qml
import org.citylive.frames

CityFrame {
    path: "/run/user/1000/citylive-iso/shm/DP-3.frame"
    active: true
    // status: CityFrame.NoFile | CityFrame.Live | CityFrame.Stale
}
```

It mmaps a frame file the wallpaper writes (see
`crates/present/src/shm.rs` for the layout and the seqlock protocol), copies the
newest published frame into a texture on the render thread and lets the GPU
scale it with nearest-neighbour filtering. `NoFile` means nothing is mapped,
`Live` means frames are arriving, `Stale` means mapped but quiet for a few
seconds (an idle city writes nothing on purpose — D71).

Only QtCore/QtGui/QtQuick are used: no private headers, no QML module
dependencies, nothing that needs KDE framework headers.

## Build and install (on the desktop, no sudo)

```sh
plasma/frames/build-install.sh
```

That configures a Release build in `plasma/frames/build/`, compiles it and
installs to `~/.local/lib/qt6/qml/org/citylive/frames/`:

| file | what |
|---|---|
| `qmldir` | the module index (`module org.citylive.frames`, plugin name) |
| `cityliveframes.qmltypes` | type information (tooling; not needed at run time) |
| `libcityliveframesplugin.so` | the plugin (the type is registered from it) |

Requirements: `cmake`, `g++`, `qt6-base`, `qt6-declarative` (Arch: `sudo pacman
-S --needed cmake qt6-base qt6-declarative`). Override the install location with
`QML_INSTALL_DIR`, the build dir with `BUILD_DIR`.

## How plasmashell finds it

`~/.local/lib/qt6/qml` is not on Qt's built-in import path, so plasmashell needs

```sh
QML_IMPORT_PATH="$HOME/.local/lib/qt6/qml"
```

`citylive-iso --install-plasma` writes `~/.config/plasma-workspace/env/citylive-iso.sh`
(which startplasma sources at login) and sets the variable in the user systemd
manager before it restarts plasmashell, so nothing has to be done by hand.

The wallpaper QML does not depend on this module: `main.qml` loads
`CityFrame.qml` in a `Loader` and switches to `FrameFiles.qml` (the PPM
transport) when the import fails, so a machine without the build keeps working.

## Checking it by hand

```sh
# 30 frames written by the wallpaper, no plasmashell:
citylive-iso --export-shm /tmp/shm-probe --stats --seconds 10
ls -l /tmp/shm-probe/            # DP-3.frame and friends
# in a Plasma session, watch the journal for the fallback warning:
journalctl --user -u plasma-plasmashell -f | grep -i citylive
```

If you see `CityLive: org.citylive.frames is not installed`, the module is not
on plasmashell's import path (or the plugin failed to load —
`QT_DEBUG_PLUGINS=1 plasmashell --replace` prints why).
