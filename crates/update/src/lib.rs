//! Citylive 2 auto-update (B97, D92/D93).
//!
//! * [`Updater::run`] asks GitHub Releases for the latest release (no token,
//!   ETag aware, at most one automatic check an hour, backing off on errors),
//!   and when it is newer downloads the file for this OS, verifies its SHA-256
//!   against the release's `SHA256SUMS`, and stages it.
//! * [`apply::apply_at_start`] runs at the next start of the program: Linux
//!   unpacks the tarball and runs its `install.sh --update`, Windows runs the
//!   staged installer silently (`/S /UPDATE`).
//!
//! Everything but the HTTPS transport is pure and tested with fixtures.

pub mod apply;
pub mod release;
pub mod sha;
pub mod state;
pub mod transport;
pub mod version;

pub use release::{parse_release, parse_sums, Platform, Release};
pub use state::{State, Staged};
pub use transport::{Reply, Transport};
pub use version::Version;

use std::path::PathBuf;

/// The public repository's latest-release endpoint (D92: public, so no token).
pub const API_URL: &str = "https://api.github.com/repos/Deluxescout1/citylive-2/releases/latest";
/// The version of this build (one version for the whole product).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Automatic checks are at least this far apart.
pub const MIN_INTERVAL_SECS: i64 = 3600;
/// The program re-checks this often while running.
pub const POLL_SECS: u64 = 6 * 3600;
/// "Check now" may not be pressed more often than this.
pub const FORCE_INTERVAL_SECS: i64 = 15;
const MAX_ASSET: u64 = 256 << 20;

/// What one pass did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Too soon since the last check.
    Throttled,
    /// This OS has no update files (not Windows or Linux x86_64).
    Unsupported,
    /// Running the newest release (or newer).
    UpToDate { latest: String },
    /// A newer release is downloaded, verified and waiting for the next start.
    Staged { version: String },
    /// Something went wrong; tried again later.
    Failed(String),
}

impl Outcome {
    /// A line for the log and the settings window.
    pub fn describe(&self) -> String {
        match self {
            Outcome::Throttled => "checked recently".into(),
            Outcome::Unsupported => "updates are not available on this system".into(),
            Outcome::UpToDate { latest } => format!("up to date (latest release {latest})"),
            Outcome::Staged { version } => {
                format!("version {version} is downloaded and installs the next time Citylive 2 starts")
            }
            Outcome::Failed(e) => format!("update check failed: {e}"),
        }
    }
}

pub struct Updater<'a> {
    pub transport: &'a dyn Transport,
    pub dir: PathBuf,
    pub platform: Option<Platform>,
    pub current: Version,
    pub api_url: String,
}

fn backoff(fails: u32) -> i64 {
    (900i64 << fails.min(5)).clamp(MIN_INTERVAL_SECS, 6 * 3600)
}

impl<'a> Updater<'a> {
    pub fn new(transport: &'a dyn Transport) -> Self {
        Self {
            transport,
            dir: state::default_dir(),
            platform: Platform::current(),
            current: Version::parse(VERSION).expect("workspace version is semver"),
            api_url: API_URL.to_string(),
        }
    }

    /// One check, and the download when something newer exists. `force` is the
    /// "Check now" button: it skips the hourly limit (not the 15 s one).
    pub fn run(&self, now: i64, force: bool) -> Outcome {
        let Some(platform) = self.platform else { return Outcome::Unsupported };
        let mut st = state::load(&self.dir);
        let due = if force { st.last_check + FORCE_INTERVAL_SECS } else { st.next_check };
        if now < due {
            return Outcome::Throttled;
        }
        let out = self.pass(platform, now, &mut st);
        match &out {
            Outcome::Failed(e) => {
                st.fails += 1;
                st.last_error = Some(e.clone());
                st.next_check = now + backoff(st.fails);
            }
            _ => {
                st.fails = 0;
                st.last_error = None;
                st.next_check = now + MIN_INTERVAL_SECS;
            }
        }
        st.last_check = now;
        state::save(&self.dir, &st);
        out
    }

    fn pass(&self, platform: Platform, _now: i64, st: &mut State) -> Outcome {
        // Only ask "has it changed?" when there is nothing left to do for the
        // release we already know: a 304 carries no asset list.
        let known = st.latest.as_deref().and_then(Version::parse);
        let settled = match (&known, &st.staged) {
            (Some(k), _) if *k <= self.current => true,
            (Some(k), Some(s)) => s.version == k.to_string() && self.staged_ok(s),
            _ => false,
        };
        let mut headers = vec![("Accept", "application/vnd.github+json")];
        let etag = st.etag.clone().filter(|_| settled);
        if let Some(e) = &etag {
            headers.push(("If-None-Match", e.as_str()));
        }
        let reply = match self.transport.get(&self.api_url, &headers, 4 << 20) {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(e),
        };
        let release = match reply.status {
            304 => {
                let Some(k) = known else { return Outcome::Failed("304 without a known release".into()) };
                if k > self.current && st.staged.as_ref().map(|s| s.version != k.to_string()).unwrap_or(true) {
                    return Outcome::Failed("304 but nothing staged".into());
                }
                return self.after(&k, None, platform, st);
            }
            // No release published yet.
            404 => return Outcome::UpToDate { latest: self.current.to_string() },
            200 => match parse_release(&String::from_utf8_lossy(&reply.body)) {
                Some(r) => r,
                // A draft, a pre-release or an odd tag: nothing to offer.
                None => return Outcome::UpToDate { latest: self.current.to_string() },
            },
            s => return Outcome::Failed(format!("GitHub answered {s}")),
        };
        st.etag = reply.etag;
        st.latest = Some(release.version.to_string());
        st.notes = Some(release.notes.clone());
        let v = release.version.clone();
        self.after(&v, Some(&release), platform, st)
    }

    fn after(&self, latest: &Version, release: Option<&Release>, platform: Platform, st: &mut State) -> Outcome {
        let latest_s = latest.to_string();
        if *latest <= self.current || st.bad.contains(&latest_s) {
            return Outcome::UpToDate { latest: latest_s };
        }
        if let Some(s) = &st.staged {
            if s.version == latest_s && self.staged_ok(s) {
                return Outcome::Staged { version: latest_s };
            }
        }
        let Some(release) = release else { return Outcome::Failed("no release data to download from".into()) };
        match self.download(release, platform) {
            Ok(staged) => {
                self.clear_old(Some(&staged.path));
                st.staged = Some(staged);
                Outcome::Staged { version: latest_s }
            }
            Err(e) => Outcome::Failed(e),
        }
    }

    fn staged_ok(&self, s: &Staged) -> bool {
        sha::sha256_file(std::path::Path::new(&s.path)).map(|h| h == s.sha256).unwrap_or(false)
    }

    fn clear_old(&self, keep: Option<&str>) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let ours = name.starts_with("Citylive2-Setup-") || name.starts_with("citylive-2-");
            if ours && Some(p.to_string_lossy().as_ref()) != keep {
                let _ = std::fs::remove_file(&p);
            }
        }
    }

    fn download(&self, release: &Release, platform: Platform) -> Result<Staged, String> {
        let name = platform.asset_name(&release.version);
        let asset = release.asset(&name).ok_or_else(|| format!("release has no {name}"))?;
        let sums_asset = release.asset("SHA256SUMS").ok_or("release has no SHA256SUMS")?;
        let sums = transport::get_following(self.transport, &sums_asset.browser_download_url, &[], 1 << 20)?;
        if sums.status != 200 {
            return Err(format!("SHA256SUMS: HTTP {}", sums.status));
        }
        let sums = parse_sums(&String::from_utf8_lossy(&sums.body));
        let want = sums.get(&name).ok_or_else(|| format!("SHA256SUMS has no entry for {name}"))?.clone();
        let file = transport::get_following(self.transport, &asset.browser_download_url, &[], MAX_ASSET)?;
        if file.status != 200 {
            return Err(format!("{name}: HTTP {}", file.status));
        }
        let got = sha::sha256_hex(&file.body);
        if got != want {
            return Err(format!("{name}: checksum mismatch (got {got}, expected {want}); download discarded"));
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.dir.join(&name);
        let part = self.dir.join(format!("{name}.part"));
        std::fs::write(&part, &file.body).map_err(|e| e.to_string())?;
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
        Ok(Staged { version: release.version.to_string(), path: path.to_string_lossy().to_string(), sha256: got })
    }
}

/// One check with the real HTTPS client and the default folder.
#[cfg(feature = "net")]
pub fn run_default(force: bool) -> Outcome {
    let t = transport::HttpsTransport::new();
    Updater::new(&t).run(locate::now_secs(), force)
}

#[cfg(test)]
mod tests;
