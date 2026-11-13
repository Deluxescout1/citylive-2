//! Location services (B96, D92/D93).
//!
//! With `location.auto` on, the city's place is found in this order: the OS
//! location service (GeoClue on Linux, `Windows.Devices.Geolocation` on
//! Windows), then one free no-key IP lookup (a chain of services), then the
//! last known place from the cache, then a built-in default. The answer is
//! cached with a timestamp. With `auto` off the place is the one the user set.
//!
//! Privacy: only coordinates and a place name are stored. The one IP lookup is
//! the only thing sent anywhere for detection (a plain GET with no personal
//! data), and `auto = false` means nothing is ever detected.
//!
//! Nothing here is called from the render loop: the callers run [`resolve`] on
//! a background thread. Everything is behind small traits, so the tests use
//! mocks and never touch the network.

mod ip;
mod os;
mod search;

pub use ip::{parse_service, IpService, SERVICES};
pub use os::system_locator;
pub use search::{parse_search, search, search_url, Place};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use world::HttpClient;

/// Built-in default when nothing else works (Greenwich, London; D16).
pub const DEFAULT_LAT: f64 = 51.48;
pub const DEFAULT_LON: f64 = 0.0;
pub const DEFAULT_NAME: &str = "Greenwich, London";

/// A detection is refreshed once a day.
pub const MAX_AGE_SECS: i64 = 24 * 3600;
/// After a failed detection (offline, no service) try again after this long,
/// which also covers "the network came back".
pub const RETRY_SECS: u64 = 10 * 60;
/// How long the OS location service gets before the IP lookup takes over.
pub const OS_TIMEOUT: Duration = Duration::from_secs(8);

/// Where a location came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The OS location service.
    Os,
    /// An IP lookup.
    Ip,
    /// The last known detection, re-read from the cache.
    Cache,
    /// The built-in default (or the config file's own values).
    Default,
    /// Set by the user.
    Manual,
}

/// A place with its provenance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resolved {
    pub lat: f64,
    pub lon: f64,
    pub name: String,
    pub source: Source,
    /// Which service: "GeoClue", "Windows Location", "ipapi.co", ...
    pub detail: String,
    /// Unix seconds when it was detected (0 = never).
    pub at: i64,
}

impl Resolved {
    pub fn default_place() -> Self {
        Self {
            lat: DEFAULT_LAT,
            lon: DEFAULT_LON,
            name: DEFAULT_NAME.to_string(),
            source: Source::Default,
            detail: String::new(),
            at: 0,
        }
    }

    /// The line the settings app shows: "detected via GeoClue", ...
    pub fn describe(&self) -> String {
        match self.source {
            Source::Os => format!("detected via {}", self.detail),
            Source::Ip => format!("detected via IP lookup ({})", self.detail),
            Source::Cache => "last known location (could not detect now)".to_string(),
            Source::Default => "built-in default (not detected yet)".to_string(),
            Source::Manual => "set by you".to_string(),
        }
    }
}

/// One fix from the OS: latitude, longitude and an optional place name.
pub type OsFix = (f64, f64, Option<String>);

/// An OS location service.
pub trait OsLocator {
    /// Short name for the source line ("GeoClue", "Windows Location").
    fn name(&self) -> &'static str;
    /// One fix, giving up after `timeout`. Must never prompt in a loop.
    fn locate(&self, timeout: Duration) -> Result<OsFix, String>;
}

/// Is a coordinate pair something a map can have?
pub fn valid(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0)
}

/// "52.37N, 4.90E", for a fix that came without a name.
pub fn coord_name(lat: f64, lon: f64) -> String {
    format!(
        "{:.2}\u{b0}{}, {:.2}\u{b0}{}",
        lat.abs(),
        if lat >= 0.0 { 'N' } else { 'S' },
        lon.abs(),
        if lon >= 0.0 { 'E' } else { 'W' }
    )
}

/// The cache folder: `$XDG_CACHE_HOME/citylive-iso`, `%LOCALAPPDATA%\citylive-iso`
/// on Windows, else `~/.cache/citylive-iso`.
pub fn cache_dir() -> PathBuf {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    if let Some(d) = var("XDG_CACHE_HOME") {
        return PathBuf::from(d).join("citylive-iso");
    }
    #[cfg(target_os = "windows")]
    if let Some(d) = var("LOCALAPPDATA") {
        return PathBuf::from(d).join("citylive-iso");
    }
    if let Some(h) = var("HOME") {
        return PathBuf::from(h).join(".cache").join("citylive-iso");
    }
    std::env::temp_dir().join("citylive-iso")
}

/// `location.json` in [`cache_dir`].
pub fn cache_path() -> PathBuf {
    cache_dir().join("location.json")
}

pub fn load_cache(path: &Path) -> Option<Resolved> {
    let r: Resolved = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    valid(r.lat, r.lon).then_some(r)
}

pub fn save_cache(path: &Path, r: &Resolved) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(r) {
        let _ = std::fs::write(path, text);
    }
}

/// Run the detection chain: OS, then each IP service, then the cache, then the
/// default. A fresh detection is written to the cache. Blocking: run it on a
/// background thread.
pub fn resolve(os: Option<&dyn OsLocator>, http: Option<&dyn HttpClient>, cache: &Path, now: i64) -> Resolved {
    if let Some(os) = os {
        if let Ok((lat, lon, name)) = os.locate(OS_TIMEOUT) {
            if valid(lat, lon) {
                let r = Resolved {
                    lat,
                    lon,
                    name: name
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| coord_name(lat, lon)),
                    source: Source::Os,
                    detail: os.name().to_string(),
                    at: now,
                };
                save_cache(cache, &r);
                return r;
            }
        }
    }
    if let Some(http) = http {
        for svc in SERVICES.iter() {
            let Ok(body) = http.get(svc.url) else { continue };
            let Ok((lat, lon, name)) = parse_service(svc, &body) else { continue };
            if !valid(lat, lon) {
                continue;
            }
            let r = Resolved { lat, lon, name, source: Source::Ip, detail: svc.name.to_string(), at: now };
            save_cache(cache, &r);
            return r;
        }
    }
    if let Some(mut r) = load_cache(cache) {
        // Keep the original timestamp so the refresh logic still sees its age.
        r.source = Source::Cache;
        return r;
    }
    Resolved::default_place()
}

/// The place to use right now, without any network or OS call: the user's own
/// if `auto` is off, else the cached detection, else the config file's values.
pub fn effective(auto: bool, lat: f64, lon: f64, name: &str, cache: &Path) -> Resolved {
    if !auto {
        return Resolved {
            lat,
            lon,
            name: name.to_string(),
            source: Source::Manual,
            detail: String::new(),
            at: 0,
        };
    }
    if let Some(r) = load_cache(cache) {
        return r;
    }
    let (lat, lon, name) = if valid(lat, lon) {
        (lat, lon, name.to_string())
    } else {
        (DEFAULT_LAT, DEFAULT_LON, DEFAULT_NAME.to_string())
    };
    Resolved { lat, lon, name, source: Source::Default, detail: String::new(), at: 0 }
}

/// Should the detection run? Never when auto is off; always when nothing is
/// cached; otherwise once the cached detection is a day old.
pub fn refresh_due(auto: bool, cached: Option<&Resolved>, now: i64) -> bool {
    if !auto {
        return false;
    }
    match cached {
        None => true,
        Some(r) => {
            let age = now - r.at;
            !(0..MAX_AGE_SECS).contains(&age)
        }
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
