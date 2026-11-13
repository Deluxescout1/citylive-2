//! Plasma wallpaper plugin (D49, B24 item 1).
//!
//! **Why a plugin and not a layer surface.** `Layer::Background` puts our surface
//! *below* Plasma's desktop containment window, and that window is opaque: on
//! stock Plasma 6 a transparent wallpaper (or `org.kde.color` with alpha 0) draws
//! black and hides us — measured in the VM, both variants, see the handback. The
//! desktop icons and widgets live *inside* that window, so nothing can be behind
//! them from outside. The only place behind the icons is *inside the containment*,
//! which is where a wallpaper plugin draws.
//!
//! So `org.citylive.iso` is a wallpaper plugin that paints our frames: the
//! wallpaper process keeps rendering exactly as before (same scene, same sim,
//! same slicing) and writes each output's frame as a PPM (Qt's cheapest lossless
//! format) plus a sequence number; the plugin shows the newest one. Frames are
//! only written when the picture actually changed, so an idle desktop still
//! writes nothing (our idle rules are unaffected).
//!
//! Everything here is user-level: `~/.local/share/plasma/wallpapers/org.citylive.iso`
//! and one key per desktop containment in `plasma-org.kde.plasma.desktop-appletsrc`,
//! which is backed up byte-for-byte and restored exactly on uninstall.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Where the frames go (`--export-frames` default): **tmpfs**, not the SSD.
///
/// 1.5 MB per head per frame at 3 fps is ~13 MB/s of writes on three screens;
/// the editor of this project asked for that to stay in RAM (B27 item 1).
pub fn frames_dir() -> PathBuf {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(d) if !d.is_empty() => PathBuf::from(d).join("citylive-iso/frames"),
        _ => PathBuf::from("/tmp/citylive-iso/frames"),
    }
}

/// Where the shared-memory frames go (`--export-shm` default, B63): one
/// `<head>.frame` per output, mmapped by both sides.
pub fn shm_dir() -> PathBuf {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(d) if !d.is_empty() => PathBuf::from(d).join("citylive-iso/shm"),
        _ => PathBuf::from("/tmp/citylive-iso/shm"),
    }
}

/// Where the native QML module is installed by `plasma/frames/build-install.sh`
/// (B63). plasmashell needs this on its import path.
pub fn qml_import_dir() -> PathBuf {
    home().join(".local/lib/qt6/qml")
}

/// The env file startplasma sources, so the module is found after every login.
fn env_file() -> PathBuf {
    home().join(".config/plasma-workspace/env/citylive-iso.sh")
}

/// Is this directory on a RAM-backed filesystem? Only `tmpfs`/`ramfs` count: the
/// frames are written several times a second and must not wear an SSD.
pub fn is_tmpfs(dir: &Path) -> bool {
    // Walk up to the nearest existing ancestor: the dir may not exist yet.
    let mut p = dir.to_path_buf();
    while !p.exists() {
        match p.parent() {
            Some(parent) => p = parent.to_path_buf(),
            None => return false,
        }
    }
    let out = std::process::Command::new("stat")
        .args(["-f", "-c", "%T"])
        .arg(&p)
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let fs = String::from_utf8_lossy(&o.stdout).trim().to_string();
            fs == "tmpfs" || fs == "ramfs"
        }
        // No `stat`: fall back to the path heuristic, which is true on every
        // normal Linux session (`XDG_RUNTIME_DIR` is a tmpfs).
        _ => {
            let s = p.display().to_string();
            s.starts_with("/run/") || s.starts_with("/dev/shm") || s.starts_with("/tmp/")
        }
    }
}

/// Where the config backup goes.
fn backup_path() -> PathBuf {
    home().join(".local/share/citylive-iso/plasma-appletsrc.bak")
}

fn package_dir() -> PathBuf {
    home().join(".local/share/plasma/wallpapers/org.citylive.iso")
}

fn appletsrc() -> PathBuf {
    home().join(".config/plasma-org.kde.plasma.desktop-appletsrc")
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
}

const METADATA: &str = r##"{
    "KPackageStructure": "Plasma/Wallpaper",
    "KPlugin": {
        "Authors": [{"Name": "CityLive Iso"}],
        "Description": "Shows the CityLive Iso live city behind the desktop icons and widgets.",
        "Id": "org.citylive.iso",
        "License": "GPL-2.0-or-later",
        "Name": "CityLive Iso",
        "ServiceTypes": ["Plasma/Wallpaper"],
        "Version": "1.0"
    },
    "X-Plasma-API": "declarativeimports",
    "X-Plasma-MainScript": "ui/main.qml"
}
"##;

/// `main.qml`: the wallpaper item, which is nothing but a `Loader`.
///
/// B63: the picture comes from the native `org.citylive.frames` item (one
/// mmapped, double-buffered frame per head) when that module is installed, and
/// from the B42e PPM files when it is not. `Loader.Error` is exactly the
/// "import failed / module missing" case, so `--install-plasma` keeps working on
/// a machine that never built the C++ module.
const MAIN_QML: &str = r##"import QtQuick
import org.kde.plasma.plasmoid

// CityLive Iso (D49): the wallpaper process renders the city; this item shows
// it inside the desktop containment, so Plasma's icons and widgets stay on top.
// B63: the fast path is a shared-memory frame per head (org.citylive.frames).
// Without that module the PPM components of B42e are used instead.
WallpaperItem {
    id: root

    Loader {
        id: view
        anchors.fill: parent
        source: "CityFrame.qml"
        onStatusChanged: {
            if (view.status !== Loader.Error)
                return;
            if (String(view.source).indexOf("FrameFiles") >= 0)
                return; // the fallback itself failed: nothing left to try
            console.warn("CityLive: org.citylive.frames is not installed ("
                         + view.errorString + "), falling back to PPM frames");
            view.source = "FrameFiles.qml";
        }
    }
}
"##;

/// `CityFrame.qml`: the native shared-memory item (B63). `__SHM__` is replaced
/// with the absolute shm directory. The file name matches what the writer
/// creates (`plasma::sanitise` on the output name, then `.frame`).
const CITY_FRAME_QML: &str = r##"import QtQuick
import org.citylive.frames

// B63: one mmapped frame per head, double buffered and published with a
// seqlock by the wallpaper. The GPU scales it; nothing is read or written as a
// file while it runs, so no frame can be lost to a race (the PPM strips could).
Item {
    id: root

    readonly property string head: (Screen.name ? Screen.name : "")
                                    .replace(/[^A-Za-z0-9_-]/g, "_")
    readonly property string framePath: "__SHM__/" + root.head + ".frame"

    CityFrame {
        id: frame
        anchors.fill: parent
        path: root.framePath
        active: true
    }

    Rectangle {
        // Before the first frame (wallpaper not running yet) show the sky the
        // city will appear in, not a black hole.
        anchors.fill: parent
        color: "#0b1020"
        visible: frame.status === CityFrame.NoFile
        z: -1
    }
}
"##;

/// `FrameFiles.qml`: the B42e file transport, kept as the fallback.
///
/// `__FRAMES__` is replaced with the absolute frames directory. The wallpaper
/// item draws nothing itself: it shows the PPMs the wallpaper process wrote for
/// this screen, and picks files by name (Qt refuses to let QML read file
/// contents without QML_XHR_ALLOW_FILE_READ).
///
/// The picture is a **base frame** (`f-<seq>.ppm`, the whole head) with
/// **strips** (`s<k>-<rows>-<seq>.ppm`, 128-row bands) drawn on top. A frame in
/// which only a bird moved writes one or two strips — a few hundred KB instead
/// of the whole 6–11 MB PPM. The server drops every strip when it writes a new
/// base (the base already contains those pixels), so a strip is always newer
/// than the base underneath it and no strip can go stale.
const FRAME_FILES_QML: &str = r##"import QtQuick
import Qt.labs.folderlistmodel

// CityLive Iso (D49), the file transport (B42e): the wallpaper process writes
// base frames and strips as PPMs per output; this item shows the newest base
// plus the strips that came after it.
Item {
    id: root

    readonly property string dir: "file://__FRAMES__/" + (Screen.name ? Screen.name : "") + "/"
    // Screen pixels per internal pixel: 1 normally, 2 on a 4K screen (D61).
    readonly property real zoom: (frame.sourceSize.height > 0 && root.height > 0)
                                 ? root.height / frame.sourceSize.height : 1

    // URL of the newest base frame the model knows, or "" if there is none.
    function newest() {
        let best = -1;
        let name = "";
        for (let i = 0; i < files.count; i++) {
            const n = files.get(i, "fileName");
            const m = /^f-([0-9]+)\.ppm$/.exec(String(n));
            if (m && parseInt(m[1]) > best) {
                best = parseInt(m[1]);
                name = n;
            }
        }
        return name === "" ? "" : root.dir + name;
    }

    // s<k>-<rows>-<seq>.ppm -> [k, rows, seq], or null for anything else.
    function stripParts(name) {
        const m = /^s([0-9]+)-([0-9]+)-([0-9]+)\.ppm$/.exec(String(name));
        return m ? [parseInt(m[1]), parseInt(m[2]), parseInt(m[3])] : null;
    }

    FolderListModel {
        id: files
        folder: root.dir
        nameFilters: ["f-*.ppm"]
        showDirs: false
    }

    FolderListModel {
        id: strips
        folder: root.dir
        nameFilters: ["s*.ppm"]
        showDirs: false
    }

    Image {
        id: frame
        anchors.fill: parent
        cache: false
        smooth: false
        fillMode: Image.Stretch
        source: root.newest()
        z: 0
    }

    // One image per strip, placed by the index and row count in its name.
    // Rounded edges tile the screen exactly at any zoom, with no seam and no
    // overlap. A strip is always newer than the base under it (the server
    // drops every strip when it writes a new base), so nothing can go stale.
    Repeater {
        model: strips
        Image {
            readonly property var parts: root.stripParts(model.fileName)
            readonly property real yTop: parts ? Math.round(parts[0] * 128 * root.zoom) : 0
            readonly property real yBottom: parts
                ? Math.round((parts[0] * 128 + parts[1]) * root.zoom) : 0
            x: 0
            y: yTop
            width: root.width
            height: Math.max(1, yBottom - yTop)
            source: parts ? root.dir + model.fileName : ""
            cache: false
            smooth: false
            fillMode: Image.Stretch
            visible: status === Image.Ready
            z: 1
        }
    }

    Timer {
        // plasmashell often starts before the wallpaper has created the frames
        // directory, and FolderListModel does not pick up a directory that
        // appears after it was created (measured in the dev VM: the desktop
        // stayed on the fallback colour until plasmashell was restarted). While
        // no base frame is showing, re-arm the models every 2 s.
        interval: 2000
        running: true
        repeat: true
        onTriggered: {
            const want = root.newest();
            if (want !== "" && want !== frame.source) {
                frame.source = want;
                return;
            }
            if (frame.status === Image.Ready)
                return;
            // Nudge the models with the trailing slash flipped: a different
            // string for the same directory, so FolderListModel re-lists it.
            const other = root.dir.endsWith("/")
                  ? root.dir.slice(0, root.dir.length - 1) : root.dir;
            files.folder = other;
            files.folder = root.dir;
            strips.folder = other;
            strips.folder = root.dir;
        }
    }

    Rectangle {
        // Before the first frame (wallpaper not running yet) show the sky the
        // city will appear in, not a black hole.
        anchors.fill: parent
        color: "#0b1020"
        visible: frame.status !== Image.Ready
        z: -1
    }
}
"##;

/// Install the plugin and point every desktop containment at it.
/// Returns the lines to print (the caller prints them, including the warning
/// that plasmashell is restarted).
pub fn install() -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    let pkg = package_dir();
    fs::create_dir_all(pkg.join("contents/ui"))?;
    fs::write(pkg.join("metadata.json"), METADATA)?;
    let frames = frames_dir();
    fs::create_dir_all(&frames)?;
    let shm = shm_dir();
    fs::create_dir_all(&shm)?;
    // B63: main.qml is only a Loader now. The two components it chooses between
    // ship next to it, so the PPM path keeps working without the native module.
    let ui = pkg.join("contents/ui");
    fs::write(ui.join("main.qml"), MAIN_QML)?;
    fs::write(
        ui.join("CityFrame.qml"),
        CITY_FRAME_QML.replace("__SHM__", &shm.display().to_string()),
    )?;
    fs::write(
        ui.join("FrameFiles.qml"),
        FRAME_FILES_QML.replace("__FRAMES__", &frames.display().to_string()),
    )?;
    out.push(format!(
        "installed the Plasma wallpaper plugin in {}",
        pkg.display()
    ));
    out.push(format!("frames directory {}", frames.display()));
    out.push(format!("shared-memory frames {}", shm.display()));

    // B63: plasmashell has to find org.citylive.frames. The env file is sourced
    // by startplasma at every login; restart_plasmashell also sets it in the
    // user manager, so it applies to the restart we do now.
    let qml = qml_import_dir();
    let env = env_file();
    let script = format!(
        "# written by citylive-iso --install-plasma (B63): where plasmashell finds\n\
         # the org.citylive.frames QML module. Remove with --uninstall-plasma.\n\
         QML_IMPORT_PATH=\"{}:${{QML_IMPORT_PATH}}\"\n\
         export QML_IMPORT_PATH\n",
        qml.display()
    );
    fs::create_dir_all(env.parent().unwrap())?;
    fs::write(&env, script)?;
    if qml.join("org/citylive/frames").exists() {
        out.push(format!("QML module found in {}", qml.display()));
    } else {
        out.push(format!(
            "QML module NOT found in {}: the wallpaper will use the PPM frames until \
             plasma/frames/build-install.sh has been run (it needs Qt 6 headers)",
            qml.display()
        ));
    }

    let cfg = appletsrc();
    if !cfg.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} not found: is this a Plasma session?", cfg.display()),
        ));
    }
    // Back the whole file up once, byte for byte, so uninstall can restore it
    // *exactly* (comments, ordering, everything).
    let bak = backup_path();
    if !bak.exists() {
        fs::create_dir_all(bak.parent().unwrap())?;
        fs::copy(&cfg, &bak)?;
        out.push(format!("saved the previous wallpaper config to {}", bak.display()));
    } else {
        out.push(format!("kept the existing backup {}", bak.display()));
    }

    let text = fs::read_to_string(&cfg)?;
    let mut section = String::new();
    let mut changed = 0;
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with('[') {
            section = line.to_string();
        }
        // Only the containment's own key — never a panel's or an applet's.
        if line.starts_with("wallpaperplugin=")
            && section.starts_with("[Containments]")
            && !section.contains("[Applets]")
            && line != "wallpaperplugin=org.citylive.iso"
        {
            lines.push("wallpaperplugin=org.citylive.iso".to_string());
            changed += 1;
        } else {
            lines.push(line.to_string());
        }
    }
    let mut body = lines.join("\n");
    body.push('\n');
    fs::write(&cfg, body)?;
    out.push(format!(
        "switched {changed} desktop containment(s) to the CityLive Iso wallpaper"
    ));
    Ok(out)
}

/// Restore the exact previous config and remove the plugin.
pub fn uninstall() -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    let cfg = appletsrc();
    let bak = backup_path();
    if bak.exists() {
        fs::copy(&bak, &cfg)?;
        fs::remove_file(&bak)?;
        out.push(format!(
            "restored {} byte for byte from the backup",
            cfg.display()
        ));
    } else {
        out.push(format!(
            "no backup found ({}): leaving the config alone",
            bak.display()
        ));
    }
    let pkg = package_dir();
    if pkg.exists() {
        fs::remove_dir_all(&pkg)?;
        out.push(format!("removed the plugin from {}", pkg.display()));
    }
    // B63: stop advertising the native QML module to plasmashell. The module
    // itself stays installed in ~/.local/lib/qt6/qml (it is a build artefact,
    // not something this command put there).
    let env = env_file();
    if env.exists() {
        fs::remove_file(&env)?;
        out.push(format!("removed {}", env.display()));
    }
    Ok(out)
}

/// Is the plugin installed *and* selected by a desktop containment?
pub fn plugin_active() -> bool {
    let installed = package_dir().join("metadata.json").exists();
    let selected = fs::read_to_string(appletsrc())
        .map(|s| s.contains("wallpaperplugin=org.citylive.iso"))
        .unwrap_or(false);
    installed && selected
}

/// Restart plasmashell so it picks the new wallpaper up. Deliberately loud.
pub fn restart_plasmashell(out: &mut Vec<String>) {
    out.push("restarting plasmashell so the change takes effect (its windows disappear for a moment)".to_string());
    // B63: the org.citylive.frames module lives outside Qt's built-in import
    // paths, so plasmashell needs QML_IMPORT_PATH in its environment. The env
    // file covers the next login; `systemctl --user set-environment` covers the
    // restart we are about to do (the service inherits the manager's env).
    let import_path = match std::env::var("QML_IMPORT_PATH") {
        Ok(v) if !v.is_empty() => {
            format!("{}:{v}", qml_import_dir().display())
        }
        _ => qml_import_dir().display().to_string(),
    };
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "set-environment", &format!("QML_IMPORT_PATH={import_path}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    // systemd owns plasmashell on a normal Plasma session, so restart *that*
    // unit: starting a second `plasmashell --replace` by hand leaves two
    // instances fighting over the config, which is how a panel gets lost.
    let unit = std::process::Command::new("systemctl")
        .args(["--user", "restart", "plasma-plasmashell.service"])
        .status();
    if matches!(unit, Ok(s) if s.success()) {
        out.push("plasmashell restarted".to_string());
        return;
    }
    let _ = std::process::Command::new("kquitapp6").arg("plasmashell").status();
    std::thread::sleep(std::time::Duration::from_millis(600));
    match std::process::Command::new("setsid")
        .arg("plasmashell")
        .arg("--replace")
        .env("QML_IMPORT_PATH", &import_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => out.push("plasmashell restarted".to_string()),
        Err(e) => out.push(format!(
            "could not restart plasmashell ({e}); log out and in, or run `plasmashell --replace &`"
        )),
    }
}

/// Write one PPM (P6) frame plus its sequence number, and drop the previous
/// frame file for that output.
///
/// B42d: **atomically**. Plasma's plugin watches the directory and decodes the
/// newest file the instant it appears, so a 6–11 MB file written in place is
/// regularly decoded half-finished. The frame is written to a hidden `.part`
/// file, flushed to the filesystem, and only then renamed over the real name —
/// the rename is what makes the frame visible, and a reader either sees the
/// whole old file or the whole new one.
///
/// B42e: a full frame is the **base** picture; every strip is dropped with it,
/// because the base already contains their pixels.
pub fn write_frame(dir: &Path, name: &str, seq: u64, px_w: u32, px_h: u32, px: &[u32]) -> io::Result<()> {
    let mut scratch = Vec::new();
    write_frame_scratch(dir, name, seq, px_w, px_h, px, &mut scratch)
}

/// The same, reusing `scratch` for the encoded bytes: at 3 fps on three screens
/// the old version allocated 28 MB a frame.
pub fn write_frame_scratch(
    dir: &Path,
    name: &str,
    seq: u64,
    px_w: u32,
    px_h: u32,
    px: &[u32],
    scratch: &mut Vec<u8>,
) -> io::Result<()> {
    let out_dir = head_dir(dir, name)?;
    let file = base_name(seq);
    write_ppm_atomic(&out_dir, &file, px_w, px_h, px, scratch)?;
    // A new base carries the whole picture: no strip may outlive it.
    drop_others(&out_dir, |n| n == file);
    Ok(())
}

/// One strip (B42e): `s<k>-<rows>-<seq>.ppm`, the full width of the head and
/// `rows` rows starting at row `k * STRIP_ROWS`. Only the previous file of the
/// **same** strip is dropped; the base and the other strips stay.
#[allow(clippy::too_many_arguments)]
pub fn write_strip_scratch(
    dir: &Path,
    name: &str,
    k: usize,
    rows: usize,
    seq: u64,
    px_w: u32,
    px: &[u32],
    scratch: &mut Vec<u8>,
) -> io::Result<()> {
    let out_dir = head_dir(dir, name)?;
    let file = strip_name(k, rows, seq);
    write_ppm_atomic(&out_dir, &file, px_w, rows as u32, px, scratch)?;
    let old = strip_prefix(k);
    drop_others(&out_dir, |n| !n.starts_with(&old) || n == file);
    Ok(())
}

/// Rows in one strip. 128 divides 1440 into 11 full strips plus a 32-row tail
/// and 1080 into 8 plus a 56-row tail; a bird is ~20 rows, so a strip is the
/// smallest useful unit without a per-strip index file.
pub const STRIP_ROWS: usize = 128;

/// The base frame's file name.
pub fn base_name(seq: u64) -> String {
    format!("f-{seq}.ppm")
}

/// One strip's file name: index, row count, sequence. The row count is in the
/// name so the QML can place a short tail strip exactly.
pub fn strip_name(k: usize, rows: usize, seq: u64) -> String {
    format!("s{k}-{rows}-{seq}.ppm")
}

/// `s<k>-` — what the old file of strip `k` starts with.
fn strip_prefix(k: usize) -> String {
    format!("s{k}-")
}

fn head_dir(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let out_dir = dir.join(sanitise(name));
    fs::create_dir_all(&out_dir)?;
    Ok(out_dir)
}

/// Encode `px` as a P6 PPM and make it visible under `file` in one rename.
fn write_ppm_atomic(
    out_dir: &Path,
    file: &str,
    px_w: u32,
    px_h: u32,
    px: &[u32],
    scratch: &mut Vec<u8>,
) -> io::Result<()> {
    let path = out_dir.join(file);
    let part = out_dir.join(format!(".{file}.part"));
    let pixels = px_w as usize * px_h as usize;
    scratch.clear();
    scratch.extend_from_slice(format!("P6\n{px_w} {px_h}\n255\n").as_bytes());
    let header = scratch.len();
    scratch.resize(header + pixels * 3, 0);
    // Channel-split in one pass over three-byte groups: the old per-pixel
    // indexing cost ~30 ms for a 7040x1440 union (B42d).
    for (dst, &p) in scratch[header..].chunks_exact_mut(3).zip(px.iter()) {
        dst[0] = (p >> 16) as u8;
        dst[1] = (p >> 8) as u8;
        dst[2] = p as u8;
    }
    fs::write(&part, &scratch[..])?;
    fs::rename(&part, &path)
}

/// Remove every file in a head's directory whose name `keep` rejects.
fn drop_others<F: Fn(&str) -> bool>(out_dir: &Path, keep: F) {
    if let Ok(rd) = fs::read_dir(out_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !keep(&name) {
                let _ = fs::remove_file(e.path());
            }

        }
    }
}

/// Output names are used in file names: keep them boring. The native QML does
/// the same (`CityFrame.qml`'s `head` property), so both sides agree on
/// `<name>.frame`.
pub fn sanitise(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_qml_agrees_with_the_file_names() {
        let qml = FRAME_FILES_QML.replace("__FRAMES__", "/tmp/frames");
        assert!(!qml.contains("__FRAMES__"), "the frames path is substituted");
        assert!(qml.contains("file:///tmp/frames/"), "the base URL keeps frame:// semantics");
        assert!(qml.contains("\"f-*.ppm\""), "the base model watches f-*");
        assert!(qml.contains("\"s*.ppm\""), "the strip model watches s*");
        // The strip pattern must match what strip_name writes.
        assert!(qml.contains("s([0-9]+)-([0-9]+)-([0-9]+)"), "strip pattern");
        assert_eq!(strip_name(12, 56, 99), "s12-56-99.ppm");
    }

    /// B63: main.qml only chooses between the native item and the PPM fallback,
    /// and the native item's file name matches what `sanitise` + `.frame` make.
    #[test]
    fn the_main_qml_prefers_the_native_item_and_keeps_the_fallback() {
        assert!(MAIN_QML.contains("source: \"CityFrame.qml\""), "{}", MAIN_QML);
        assert!(MAIN_QML.contains("Loader.Error"), "the missing module is caught");
        assert!(
            MAIN_QML.contains("view.source = \"FrameFiles.qml\""),
            "the PPM path is the fallback"
        );
        // The fallback must not loop on its own error.
        assert!(MAIN_QML.contains("indexOf(\"FrameFiles\")"));

        let native = CITY_FRAME_QML.replace("__SHM__", "/run/user/1000/citylive-iso/shm");
        assert!(!native.contains("__SHM__"));
        assert!(native.contains("import org.citylive.frames"));
        assert!(native.contains("CityFrame.NoFile"));
        assert!(native.contains("\"/run/user/1000/citylive-iso/shm/\" + root.head + \".frame\""));
        // Same name rule as plasma::sanitise: only [A-Za-z0-9_-] survives.
        assert!(native.contains(".replace(/[^A-Za-z0-9_-]/g, \"_\")"));
        assert_eq!(sanitise("HDMI/A-1"), "HDMI_A-1");
    }

    #[test]
    fn strip_names_parse_back() {
        // The QML reads s<k>-<rows>-<seq>.ppm with this pattern; keep them in step.
        let re = ["s", "", "-", "", "-", "", ".ppm"];
        let name = strip_name(3, 128, 42);
        assert_eq!(name, "s3-128-42.ppm");
        let parts: Vec<&str> = name.trim_end_matches(".ppm").split('-').collect();
        assert_eq!(parts.len(), 3, "{name}");
        assert_eq!(parts[0], format!("{}{}", re[0], 3));
        assert_eq!(parts[1].parse::<usize>().unwrap(), 128);
        assert_eq!(parts[2].parse::<usize>().unwrap(), 42);
        assert!(base_name(42) != name);
    }

    /// B42e: a base plus the strips written after it must reproduce the frame
    /// exactly — the property the QML relies on when it stacks them.
    #[test]
    fn base_and_strips_reassemble_the_frame() {
        let dir = std::env::temp_dir().join("citylive-iso-ppm-strips");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (16usize, 300usize); // 128 + 128 + 44: two strips and a tail
        let mut scratch = Vec::new();
        let base: Vec<u32> = (0..w * h).map(|i| 0x0010_0000 + i as u32).collect();
        write_frame_scratch(&dir, "DP-3", 1, w as u32, h as u32, &base, &mut scratch).unwrap();

        // Change rows 150..160 and 280..290 (the second and the last strip).
        let mut now = base.clone();
        for y in 150..160 {
            for x in 0..w {
                now[y * w + x] = 0x00ff_ff00;
            }
        }
        for y in 280..290 {
            for x in 0..w {
                now[y * w + x] = 0x0000_00ff;
            }
        }
        for k in [1usize, 2] {
            let row0 = (k * STRIP_ROWS).min(h);
            let rows = (h - row0).min(STRIP_ROWS);
            let mut rows_px = Vec::new();
            for y in row0..row0 + rows {
                rows_px.extend_from_slice(&now[y * w..(y + 1) * w]);
            }
            write_strip_scratch(&dir, "DP-3", k, rows, 2, w as u32, &rows_px, &mut scratch).unwrap();
        }
        // The base is still there, both strips are, and nothing else is.
        let names: Vec<String> = fs::read_dir(dir.join("DP-3"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");

        // Reassemble the way the QML does: base, then strips by index.
        let mut shown = base.clone();
        let mut strips: Vec<(usize, usize, usize)> = names
            .iter()
            .filter_map(|n| {
                let p: Vec<&str> = n.trim_end_matches(".ppm").split('-').collect();
                (p.len() == 3 && p[0].starts_with('s')).then(|| {
                    (
                        p[0][1..].parse::<usize>().unwrap(),
                        p[1].parse::<usize>().unwrap(),
                        p[2].parse::<usize>().unwrap(),
                    )
                })
            })
            .collect();
        strips.sort_unstable();
        for (k, rows, seq) in strips {
            let fname = strip_name(k, rows, seq as u64);
            let raw = fs::read(dir.join("DP-3").join(&fname)).unwrap();
            let body = &raw[format!("P6\n{w} {rows}\n255\n").len()..];
            let row0 = k * STRIP_ROWS;
            for y in 0..rows {
                for x in 0..w {
                    let o = (y * w + x) * 3;
                    shown[(row0 + y) * w + x] =
                        ((body[o] as u32) << 16) | ((body[o + 1] as u32) << 8) | body[o + 2] as u32;
                }
            }
        }
        assert_eq!(shown, now, "base + strips do not reproduce the frame");

        // A new full frame drops every strip (nothing can go stale).
        write_frame_scratch(&dir, "DP-3", 3, w as u32, h as u32, &now, &mut scratch).unwrap();
        let names: Vec<String> = fs::read_dir(dir.join("DP-3"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["f-3.ppm".to_string()], "{names:?}");
    }

    #[test]
    fn names_are_sanitised() {
        assert_eq!(sanitise("DP-3"), "DP-3");
        assert_eq!(sanitise("HDMI/A-1"), "HDMI_A-1");
        assert_eq!(sanitise("eDP 1"), "eDP_1");
    }

    #[test]
    fn a_frame_is_a_valid_ppm() {
        let dir = std::env::temp_dir().join("citylive-iso-ppm-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let px = vec![0x00ff_0000u32, 0x0000_ff00];
        write_frame(&dir, "Virtual-1", 3, 2, 1, &px).unwrap();
        let raw = std::fs::read(dir.join("Virtual-1/f-3.ppm")).unwrap();
        assert_eq!(&raw[..11], b"P6\n2 1\n255\n");
        assert_eq!(&raw[11..], &[255, 0, 0, 0, 255, 0]);
        // A newer frame removes the old one.
        write_frame(&dir, "Virtual-1", 4, 1, 1, &px[..1]).unwrap();
        assert!(!dir.join("Virtual-1/f-3.ppm").exists());
        assert!(dir.join("Virtual-1/f-4.ppm").exists());
    }

    /// B42d: no partial file is ever visible under the name the plugin reads.
    #[test]
    fn a_frame_appears_whole() {
        let dir = std::env::temp_dir().join("citylive-iso-ppm-atomic");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let px: Vec<u32> = (0..64 * 32).map(|i| 0x0010_2030 + i as u32).collect();
        let mut scratch = Vec::new();
        write_frame_scratch(&dir, "DP-3", 1, 64, 32, &px, &mut scratch).unwrap();
        let raw = std::fs::read(dir.join("DP-3/f-1.ppm")).unwrap();
        // Header + every pixel, in one piece.
        // "P6\n64 32\n255\n" is 13 bytes, then three bytes a pixel.
        assert_eq!(raw.len(), 13 + px.len() * 3);
        // Nothing but the finished frame is left behind (no .part, no temp).
        let names: Vec<String> = std::fs::read_dir(dir.join("DP-3"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["f-1.ppm".to_string()], "{names:?}");
        // The scratch buffer is reused, not grown without bound.
        let cap = scratch.capacity();
        write_frame_scratch(&dir, "DP-3", 2, 64, 32, &px, &mut scratch).unwrap();
        assert_eq!(scratch.capacity(), cap);
    }
}
