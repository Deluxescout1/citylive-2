//! The GitHub Releases answer: which release is newest, which file is ours,
//! and the `SHA256SUMS` list.

use serde::Deserialize;
use std::collections::HashMap;

use crate::version::Version;

/// The files a release carries for each platform (the packaging scripts and the
/// release workflow use the same names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    WindowsX64,
    LinuxX64,
}

impl Platform {
    /// The platform this program was built for (`None`: no updates offered).
    pub fn current() -> Option<Platform> {
        if cfg!(all(windows, target_arch = "x86_64")) {
            Some(Platform::WindowsX64)
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Some(Platform::LinuxX64)
        } else {
            None
        }
    }

    /// The release file for version `v` (no leading `v`).
    pub fn asset_name(self, v: &Version) -> String {
        match self {
            Platform::WindowsX64 => format!("Citylive2-Setup-{v}.exe"),
            Platform::LinuxX64 => format!("citylive-2-{v}-linux-x86_64.tar.gz"),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// A usable release: a proper version, not a draft or pre-release.
#[derive(Clone, Debug)]
pub struct Release {
    pub version: Version,
    pub notes: String,
    pub assets: Vec<Asset>,
}

/// Parse the `releases/latest` JSON. Drafts, pre-releases and tags that are
/// not versions give `None`.
pub fn parse_release(json: &str) -> Option<Release> {
    let r: RawRelease = serde_json::from_str(json).ok()?;
    if r.draft || r.prerelease {
        return None;
    }
    let version = Version::parse(&r.tag_name)?;
    if version.is_prerelease() {
        return None;
    }
    Some(Release { version, notes: r.body.unwrap_or_default(), assets: r.assets })
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// `SHA256SUMS`: lines of `<64 hex>  <name>` (also `*name`, the binary mark).
pub fn parse_sums(text: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((h, name)) = line.split_once(char::is_whitespace) else { continue };
        let name = name.trim().trim_start_matches('*');
        if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) && !name.is_empty() {
            m.insert(name.to_string(), h.to_ascii_lowercase());
        }
    }
    m
}
