//! What the updater remembers between runs, in `<cache>/update/state.json`:
//! the ETag, when to ask next, the newest version seen, the staged download
//! and versions whose installation failed. The settings window reads the same
//! file to show the latest version.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staged {
    pub version: String,
    /// The downloaded installer or tarball.
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub etag: Option<String>,
    /// Seconds since the epoch of the last attempt that reached the network.
    pub last_check: i64,
    /// No automatic attempt before this time (an hour, longer after errors).
    pub next_check: i64,
    /// Failed attempts in a row.
    pub fails: u32,
    /// The newest release seen, as `x.y.z`.
    pub latest: Option<String>,
    pub notes: Option<String>,
    pub staged: Option<Staged>,
    /// Versions that were tried and must not be tried again.
    pub bad: Vec<String>,
    pub last_error: Option<String>,
}

/// `<cache>/update`.
pub fn default_dir() -> PathBuf {
    locate::cache_dir().join("update")
}

fn file(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

pub fn load(dir: &Path) -> State {
    std::fs::read_to_string(file(dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Atomic write: a reader (the settings window) never sees half a file.
pub fn save(dir: &Path, s: &State) {
    let _ = std::fs::create_dir_all(dir);
    let tmp = dir.join("state.json.tmp");
    if let Ok(text) = serde_json::to_string_pretty(s) {
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, file(dir));
        }
    }
}
