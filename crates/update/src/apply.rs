//! Installing a staged update at the next start of the program.
//!
//! * Linux: the tarball is unpacked next to the download and its `install.sh
//!   --update --no-restart` swaps the files (each one copied beside the old
//!   and renamed over it, so a running binary is replaced atomically); then
//!   the program starts the new binary in its place ([`Applied::Relaunch`]).
//! * Windows: the staged installer runs silently and per-user
//!   (`/S /UPDATE`); it closes this program, installs and starts the new one
//!   ([`Applied::Exit`]).
//!
//! A version is written to `bad` *before* it is tried, so a failing update is
//! never retried in a loop; the next release gets a fresh chance.

use std::path::{Path, PathBuf};

use crate::state::{self, Staged};
use crate::version::Version;

#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    /// Nothing staged, or not an installed copy, or the update failed.
    Nothing,
    /// Linux: the new binary is installed here; start it with the same arguments.
    Relaunch(PathBuf),
    /// Windows: the installer took over; leave now.
    Exit,
}

/// The staged update that should be installed now: newer than `current`,
/// not marked bad, its file still there and still matching its SHA-256.
pub fn pending(dir: &Path, current: &Version) -> Option<Staged> {
    let st = state::load(dir);
    let s = st.staged?;
    let v = Version::parse(&s.version)?;
    if v <= *current || st.bad.contains(&s.version) {
        return None;
    }
    let h = crate::sha::sha256_file(Path::new(&s.path)).ok()?;
    (h == s.sha256).then_some(s)
}

/// Remember that `version` was tried (and drop the staged entry).
pub fn mark_tried(dir: &Path, version: &str) {
    let mut st = state::load(dir);
    if !st.bad.iter().any(|b| b == version) {
        st.bad.push(version.to_string());
    }
    st.staged = None;
    state::save(dir, &st);
}

/// Is this program the installed copy (not a build in a source tree)? Only the
/// installed copy updates itself.
pub fn is_installed_copy() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let Some(dir) = exe.parent() else { return false };
    #[cfg(windows)]
    {
        dir.join("Uninstall.exe").exists()
    }
    #[cfg(not(windows))]
    {
        let root = install_root();
        exe.canonicalize().map(|e| e.starts_with(&root)).unwrap_or(false) || dir.starts_with(&root)
    }
}

/// `$XDG_DATA_HOME/citylive-2` (default `~/.local/share/citylive-2`).
#[cfg(not(windows))]
pub fn install_root() -> PathBuf {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    if let Some(d) = var("XDG_DATA_HOME") {
        return PathBuf::from(d).join("citylive-2");
    }
    PathBuf::from(var("HOME").unwrap_or_default()).join(".local/share/citylive-2")
}

/// Install a staged update if there is one. Call before anything else starts.
/// `enabled` is the "Check for updates automatically" setting.
pub fn apply_at_start(enabled: bool) -> Applied {
    if !enabled || !is_installed_copy() {
        return Applied::Nothing;
    }
    let dir = state::default_dir();
    let Ok(current) = Version::parse(crate::VERSION).ok_or(()) else { return Applied::Nothing };
    let Some(staged) = pending(&dir, &current) else { return Applied::Nothing };
    println!("update: installing {} (from {})", staged.version, current);
    mark_tried(&dir, &staged.version);
    apply(&staged, &dir)
}

#[cfg(not(windows))]
fn apply(staged: &Staged, dir: &Path) -> Applied {
    use std::process::Command;
    let unpack = dir.join("unpack");
    let _ = std::fs::remove_dir_all(&unpack);
    if std::fs::create_dir_all(&unpack).is_err() {
        return Applied::Nothing;
    }
    let ok = Command::new("tar")
        .arg("-xzf")
        .arg(&staged.path)
        .arg("-C")
        .arg(&unpack)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("update: could not unpack {}", staged.path);
        return Applied::Nothing;
    }
    let top = std::fs::read_dir(&unpack)
        .ok()
        .and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.join("install.sh").is_file()));
    let Some(top) = top else {
        eprintln!("update: the tarball has no install.sh");
        let _ = std::fs::remove_dir_all(&unpack);
        return Applied::Nothing;
    };
    let ran = Command::new("sh")
        .arg(top.join("install.sh"))
        .args(["--update", "--no-restart"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = std::fs::remove_dir_all(&unpack);
    if !ran {
        eprintln!("update: install.sh failed; staying on the current version");
        return Applied::Nothing;
    }
    let _ = std::fs::remove_file(&staged.path);
    let new_exe = install_root().join("bin/citylive-iso");
    if new_exe.is_file() {
        Applied::Relaunch(new_exe)
    } else {
        Applied::Nothing
    }
}

#[cfg(windows)]
fn apply(staged: &Staged, _dir: &Path) -> Applied {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    match std::process::Command::new(&staged.path)
        .args(["/S", "/UPDATE"])
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
    {
        Ok(_) => Applied::Exit,
        Err(e) => {
            eprintln!("update: could not start the installer: {e}");
            Applied::Nothing
        }
    }
}

/// Start the freshly installed binary in place of this process (Linux).
#[cfg(unix)]
pub fn relaunch(exe: &Path) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(exe).args(std::env::args_os().skip(1)).exec()
}
