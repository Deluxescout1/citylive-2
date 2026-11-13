//! No network: a fixture transport answers from a table and records requests.

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

type Calls = RefCell<Vec<(String, Vec<(String, String)>)>>;

struct Fixture {
    routes: HashMap<String, Vec<Reply>>,
    calls: Calls,
}

impl Fixture {
    fn new() -> Self {
        Fixture { routes: HashMap::new(), calls: RefCell::new(Vec::new()) }
    }
    fn route(mut self, url: &str, status: u16, body: &[u8]) -> Self {
        self.routes
            .entry(url.to_string())
            .or_default()
            .push(Reply { status, etag: None, location: None, body: body.to_vec() });
        self
    }
    fn route_etag(mut self, url: &str, status: u16, body: &[u8], etag: &str) -> Self {
        self.routes.entry(url.to_string()).or_default().push(Reply {
            status,
            etag: Some(etag.into()),
            location: None,
            body: body.to_vec(),
        });
        self
    }
    fn redirect(mut self, url: &str, to: &str) -> Self {
        self.routes.entry(url.to_string()).or_default().push(Reply {
            status: 302,
            etag: None,
            location: Some(to.into()),
            body: vec![],
        });
        self
    }
    fn urls(&self) -> Vec<String> {
        self.calls.borrow().iter().map(|c| c.0.clone()).collect()
    }
    fn header_sent(&self, i: usize, name: &str) -> Option<String> {
        self.calls.borrow()[i].1.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }
}

impl Transport for Fixture {
    fn get(&self, url: &str, headers: &[(&str, &str)], _max: u64) -> Result<Reply, String> {
        self.calls
            .borrow_mut()
            .push((url.to_string(), headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()));
        // The last answer registered for a URL repeats; earlier ones are used once.
        let list = self.routes.get(url).ok_or_else(|| format!("no route for {url}"))?;
        let n = self.calls.borrow().iter().filter(|c| c.0 == url).count();
        let r = &list[(n - 1).min(list.len() - 1)];
        Ok(Reply { status: r.status, etag: r.etag.clone(), location: r.location.clone(), body: r.body.clone() })
    }
}

const PAYLOAD: &[u8] = b"pretend installer bytes";

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("citylive-update-{tag}-{}-{}", std::process::id(), locate::now_secs()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn release_json(tag: &str, plat: Platform, extra: &str) -> String {
    let v = Version::parse(tag).unwrap();
    let name = plat.asset_name(&v);
    format!(
        r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"body":"Fixes things.",{extra}
        "assets":[
          {{"name":"{name}","browser_download_url":"https://github.com/Deluxescout1/citylive-2/releases/download/{tag}/{name}","size":23}},
          {{"name":"SHA256SUMS","browser_download_url":"https://github.com/Deluxescout1/citylive-2/releases/download/{tag}/SHA256SUMS","size":100}}
        ]}}"#
    )
}

fn sums(plat: Platform, tag: &str, hash: &str) -> String {
    let v = Version::parse(tag).unwrap();
    format!("{hash}  {}\n{}  other-file.zip\n", plat.asset_name(&v), "0".repeat(64))
}

/// A fixture holding a release `tag` for `plat` with a correct (or wrong) checksum.
fn world_with(plat: Platform, tag: &str, good: bool) -> Fixture {
    let name = plat.asset_name(&Version::parse(tag).unwrap());
    let base = format!("https://github.com/Deluxescout1/citylive-2/releases/download/{tag}");
    let hash = if good { sha::sha256_hex(PAYLOAD) } else { "f".repeat(64) };
    Fixture::new()
        .route_etag(API_URL, 200, release_json(tag, plat, "").as_bytes(), "\"etag-1\"")
        .route(&format!("{base}/SHA256SUMS"), 200, sums(plat, tag, &hash).as_bytes())
        // Downloads redirect to a storage host, like the real thing.
        .redirect(&format!("{base}/{name}"), "https://objects.githubusercontent.com/blob/abc")
        .route("https://objects.githubusercontent.com/blob/abc", 200, PAYLOAD)
}

fn updater<'a>(t: &'a dyn Transport, dir: &Path, plat: Platform, current: &str) -> Updater<'a> {
    Updater {
        transport: t,
        dir: dir.to_path_buf(),
        platform: Some(plat),
        current: Version::parse(current).unwrap(),
        api_url: API_URL.to_string(),
    }
}

use std::path::Path;

#[test]
fn versions_parse_and_order() {
    let v = |s: &str| Version::parse(s).unwrap();
    assert_eq!(v("v0.2.0").to_string(), "0.2.0");
    assert!(v("0.2.1") > v("0.2.0"));
    assert!(v("0.10.0") > v("0.9.9"));
    assert!(v("1.0.0") > v("0.99.99"));
    assert!(v("1.0.0-rc.1") < v("1.0.0"));
    assert!(v("1.0.0-rc.2") > v("1.0.0-rc.1"));
    assert!(v("1.0.0+build5") == v("1.0.0"));
    for bad in ["", "1", "1.2", "1.2.3.4", "x.y.z", "1.2.3-", "latest"] {
        assert!(Version::parse(bad).is_none(), "{bad}");
    }
    assert!(Version::parse(VERSION).is_some(), "the workspace version is semver");
}

#[test]
fn release_json_and_sums() {
    let r = parse_release(&release_json("v0.3.0", Platform::LinuxX64, "")).unwrap();
    assert_eq!(r.version.to_string(), "0.3.0");
    assert_eq!(r.assets.len(), 2);
    assert!(r.asset("SHA256SUMS").is_some());
    let draft = release_json("v0.3.0", Platform::LinuxX64, "").replace("\"draft\":false", "\"draft\":true");
    assert!(parse_release(&draft).is_none());
    let pre = release_json("v0.3.0", Platform::LinuxX64, "").replace("\"prerelease\":false", "\"prerelease\":true");
    assert!(parse_release(&pre).is_none());
    assert!(parse_release(&release_json("v0.3.0-rc.1", Platform::LinuxX64, "")).is_none());
    assert!(parse_release("not json").is_none());
    assert!(parse_release(r#"{"tag_name":"nightly"}"#).is_none());

    let m = parse_sums(&format!("{}  a.exe\n{} *b.tar.gz\nbogus line\nabc  short\n", "A".repeat(64), "b".repeat(64)));
    assert_eq!(m.len(), 2);
    assert_eq!(m["a.exe"], "a".repeat(64), "hex is lower-cased");
    assert_eq!(m["b.tar.gz"], "b".repeat(64));
}

#[test]
fn asset_names_match_the_packaging() {
    let v = Version::parse("0.2.0").unwrap();
    assert_eq!(Platform::WindowsX64.asset_name(&v), "Citylive2-Setup-0.2.0.exe");
    assert_eq!(Platform::LinuxX64.asset_name(&v), "citylive-2-0.2.0-linux-x86_64.tar.gz");
}

#[test]
fn sha256_known_vectors() {
    assert_eq!(sha::sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(sha::sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    let d = tmpdir("sha");
    let f = d.join("x");
    std::fs::write(&f, b"abc").unwrap();
    assert_eq!(sha::sha256_file(&f).unwrap(), sha::sha256_hex(b"abc"));
}

#[test]
fn a_newer_release_is_downloaded_verified_and_staged() {
    for plat in [Platform::WindowsX64, Platform::LinuxX64] {
        let d = tmpdir("stage");
        let t = world_with(plat, "v0.3.0", true);
        let out = updater(&t, &d, plat, "0.2.0").run(10_000, false);
        assert_eq!(out, Outcome::Staged { version: "0.3.0".into() });
        let st = state::load(&d);
        let staged = st.staged.expect("staged");
        assert_eq!(std::fs::read(&staged.path).unwrap(), PAYLOAD);
        assert_eq!(staged.sha256, sha::sha256_hex(PAYLOAD));
        assert_eq!(st.latest.as_deref(), Some("0.3.0"));
        assert_eq!(st.etag.as_deref(), Some("\"etag-1\""));
        assert!(!d.join(format!("{}.part", plat.asset_name(&Version::parse("0.3.0").unwrap()))).exists());
        // The API asked for the GitHub JSON and sent no token.
        assert_eq!(t.header_sent(0, "Accept").as_deref(), Some("application/vnd.github+json"));
        assert!(t.calls.borrow()[0].1.iter().all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));
        // And the installer is what pending() hands the next start.
        let p = apply::pending(&d, &Version::parse("0.2.0").unwrap()).expect("pending");
        assert_eq!(p.version, "0.3.0");
        assert!(apply::pending(&d, &Version::parse("0.3.0").unwrap()).is_none(), "already running it");
    }
}

#[test]
fn a_wrong_checksum_is_refused_and_nothing_is_staged() {
    let d = tmpdir("badsum");
    let plat = Platform::LinuxX64;
    let t = world_with(plat, "v0.3.0", false);
    let out = updater(&t, &d, plat, "0.2.0").run(10_000, false);
    assert!(matches!(&out, Outcome::Failed(e) if e.contains("checksum mismatch")), "{out:?}");
    let st = state::load(&d);
    assert!(st.staged.is_none());
    assert_eq!(st.fails, 1);
    let leftovers: Vec<_> = std::fs::read_dir(&d)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("citylive-2-"))
        .collect();
    assert!(leftovers.is_empty(), "no unverified file stays on disk");
}

#[test]
fn a_missing_sums_file_or_asset_fails() {
    let d = tmpdir("nosums");
    let plat = Platform::LinuxX64;
    let json = release_json("v0.3.0", plat, "").replace("SHA256SUMS", "SOMETHING-ELSE");
    let t = Fixture::new().route(API_URL, 200, json.as_bytes());
    let out = updater(&t, &d, plat, "0.2.0").run(10_000, false);
    assert!(matches!(&out, Outcome::Failed(e) if e.contains("SHA256SUMS")), "{out:?}");

    let d = tmpdir("noasset");
    let json = release_json("v0.3.0", Platform::WindowsX64, "");
    let t = Fixture::new().route(API_URL, 200, json.as_bytes());
    let out = updater(&t, &d, plat, "0.2.0").run(10_000, false);
    assert!(matches!(&out, Outcome::Failed(e) if e.contains("no citylive-2-0.3.0")), "{out:?}");
}

#[test]
fn same_or_older_release_does_nothing() {
    let plat = Platform::WindowsX64;
    for current in ["0.3.0", "0.4.0"] {
        let d = tmpdir("same");
        let t = world_with(plat, "v0.3.0", true);
        let out = updater(&t, &d, plat, current).run(10_000, false);
        assert_eq!(out, Outcome::UpToDate { latest: "0.3.0".into() });
        assert_eq!(t.urls(), vec![API_URL.to_string()], "no download for an old release");
        assert!(state::load(&d).staged.is_none());
    }
}

#[test]
fn checks_are_at_most_hourly_and_the_etag_is_honoured() {
    let plat = Platform::LinuxX64;
    let d = tmpdir("etag");
    let t = Fixture::new()
        .route_etag(API_URL, 200, release_json("v0.2.0", plat, "").as_bytes(), "\"e1\"")
        .route(API_URL, 304, b"");
    let u = updater(&t, &d, plat, "0.2.0");
    assert_eq!(u.run(1_000, false), Outcome::UpToDate { latest: "0.2.0".into() });
    assert_eq!(u.run(1_000 + 3_599, false), Outcome::Throttled, "less than an hour");
    assert_eq!(t.urls().len(), 1);
    // After the hour: conditional request, 304 answer, still up to date.
    assert_eq!(u.run(1_000 + 3_600, false), Outcome::UpToDate { latest: "0.2.0".into() });
    assert_eq!(t.header_sent(1, "If-None-Match").as_deref(), Some("\"e1\""));
    // "Check now" skips the hour but not the 15 s guard.
    assert_eq!(u.run(1_000 + 3_600 + 5, true), Outcome::Throttled);
    assert_eq!(u.run(1_000 + 3_600 + 20, true), Outcome::UpToDate { latest: "0.2.0".into() });
}

#[test]
fn a_304_never_hides_an_update_that_was_not_downloaded() {
    // The release is newer, the first download failed: the next check must
    // not send the ETag (a 304 has no asset list) and must retry the download.
    let plat = Platform::LinuxX64;
    let d = tmpdir("retry");
    let bad = world_with(plat, "v0.3.0", false);
    let out = updater(&bad, &d, plat, "0.2.0").run(1_000, false);
    assert!(matches!(out, Outcome::Failed(_)));
    let good = world_with(plat, "v0.3.0", true);
    let out = updater(&good, &d, plat, "0.2.0").run(1_000 + 6 * 3_600, false);
    assert_eq!(out, Outcome::Staged { version: "0.3.0".into() });
    assert!(good.header_sent(0, "If-None-Match").is_none());
}

#[test]
fn a_staged_update_is_not_downloaded_twice() {
    let plat = Platform::LinuxX64;
    let d = tmpdir("twice");
    let t = world_with(plat, "v0.3.0", true);
    let u = updater(&t, &d, plat, "0.2.0");
    assert_eq!(u.run(1_000, false), Outcome::Staged { version: "0.3.0".into() });
    let n = t.urls().len();
    assert_eq!(u.run(1_000 + 3_600, false), Outcome::Staged { version: "0.3.0".into() });
    assert_eq!(t.urls().len(), n + 1, "only the API call is repeated");
}

#[test]
fn errors_back_off() {
    let plat = Platform::LinuxX64;
    let d = tmpdir("backoff");
    let t = Fixture::new().route(API_URL, 500, b"oops");
    let u = updater(&t, &d, plat, "0.2.0");
    assert!(matches!(u.run(1_000, false), Outcome::Failed(_)));
    let s1 = state::load(&d);
    assert_eq!(s1.fails, 1);
    assert!(s1.next_check - 1_000 >= MIN_INTERVAL_SECS);
    assert_eq!(u.run(1_000 + 60, false), Outcome::Throttled);
    assert!(matches!(u.run(s1.next_check, false), Outcome::Failed(_)));
    let s2 = state::load(&d);
    assert_eq!(s2.fails, 2);
    assert!(s2.next_check - s1.next_check >= s1.next_check - 1_000, "no faster than before");
    assert!(s2.last_error.unwrap().contains("500"));
    // The limit: never longer than six hours.
    assert_eq!(backoff(40), 6 * 3600);
    // A network failure counts too.
    let d = tmpdir("backoff2");
    let t = Fixture::new();
    assert!(matches!(updater(&t, &d, plat, "0.2.0").run(1_000, false), Outcome::Failed(_)));
}

#[test]
fn no_release_yet_is_quiet() {
    let d = tmpdir("404");
    let t = Fixture::new().route(API_URL, 404, b"{}");
    let out = updater(&t, &d, Platform::LinuxX64, "0.2.0").run(1_000, false);
    assert_eq!(out, Outcome::UpToDate { latest: "0.2.0".into() });
    assert_eq!(state::load(&d).fails, 0);
}

#[test]
fn unsupported_systems_get_no_updates() {
    let d = tmpdir("unsupported");
    let t = Fixture::new();
    let mut u = updater(&t, &d, Platform::LinuxX64, "0.2.0");
    u.platform = None;
    assert_eq!(u.run(1_000, false), Outcome::Unsupported);
    assert!(t.urls().is_empty());
}

#[test]
fn redirects_stay_on_github() {
    assert!(transport::redirect_allowed("https://objects.githubusercontent.com/x"));
    assert!(transport::redirect_allowed("https://github.com/a/b"));
    assert!(transport::redirect_allowed("https://api.github.com/x"));
    assert!(!transport::redirect_allowed("http://github.com/x"), "https only");
    assert!(!transport::redirect_allowed("https://github.com.evil.example/x"));
    assert!(!transport::redirect_allowed("https://evilgithub.com/x"));
    assert!(!transport::redirect_allowed("https://example.com/x"));

    let t = Fixture::new().redirect("https://github.com/a", "https://example.com/payload");
    assert!(transport::get_following(&t, "https://github.com/a", &[], 10).is_err());
    let loops = Fixture::new().redirect("https://github.com/a", "https://github.com/a");
    assert!(transport::get_following(&loops, "https://github.com/a", &[], 10).unwrap_err().contains("too many"));
}

#[test]
fn a_tried_version_is_not_tried_again() {
    let plat = Platform::LinuxX64;
    let d = tmpdir("bad");
    let t = world_with(plat, "v0.3.0", true);
    let u = updater(&t, &d, plat, "0.2.0");
    assert_eq!(u.run(1_000, false), Outcome::Staged { version: "0.3.0".into() });
    let cur = Version::parse("0.2.0").unwrap();
    assert!(apply::pending(&d, &cur).is_some());
    apply::mark_tried(&d, "0.3.0");
    assert!(apply::pending(&d, &cur).is_none(), "not retried after a failed attempt");
    // And the checker does not download it again either.
    let t2 = world_with(plat, "v0.3.0", true);
    let out = updater(&t2, &d, plat, "0.2.0").run(1_000 + 7_200, false);
    assert_eq!(out, Outcome::UpToDate { latest: "0.3.0".into() });
    assert!(!t2.urls().iter().any(|u| u.contains("objects.")), "no download");
}

#[test]
fn a_corrupted_staged_file_is_not_applied() {
    let plat = Platform::WindowsX64;
    let d = tmpdir("corrupt");
    let t = world_with(plat, "v0.3.0", true);
    assert!(matches!(updater(&t, &d, plat, "0.2.0").run(1_000, false), Outcome::Staged { .. }));
    let staged = state::load(&d).staged.unwrap();
    std::fs::write(&staged.path, b"tampered").unwrap();
    assert!(apply::pending(&d, &Version::parse("0.2.0").unwrap()).is_none());
}

#[test]
fn a_development_build_does_not_update_itself() {
    // The test binary lives in a target directory, not an install.
    assert!(!apply::is_installed_copy());
    assert_eq!(apply::apply_at_start(true), apply::Applied::Nothing);
}

#[test]
fn state_survives_garbage() {
    let d = tmpdir("garbage");
    std::fs::write(d.join("state.json"), "{{{{").unwrap();
    assert_eq!(state::load(&d), State::default());
    std::fs::write(d.join("state.json"), r#"{"latest":"1.2.3","future_field":1}"#).unwrap();
    assert_eq!(state::load(&d).latest.as_deref(), Some("1.2.3"));
}
