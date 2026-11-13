//! Aurora: the current planetary Kp index (NOAA SWPC, no key) and whether
//! there's actually anything to see at *your* latitude — port of old
//! `city.js` `fetchKp` (city.js:2648-2669) and `geomagLat`/`auroraLevel`
//! (city.js:1131-1156).  The level is 0 = nothing .. 1 = the oval is
//! essentially overhead.  Refresh is 30 minutes
//! ([`crate::feeds::AURORA_REFRESH_MS`], the old `KP_BUCKET`).

use std::fs;
use std::path::Path;

use crate::http::HttpClient;

/// The NOAA SWPC planetary K-index product (old `fetchKp` endpoint).
/// No key, no personal data.
pub fn url_for() -> &'static str {
    "https://services.swpc.noaa.gov/products/noaa-planetary-k-index.json"
}

/// Where a Kp reading came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KpSource {
    /// Fresh from the live service.
    Live,
    /// Re-read from the last good cached response.
    Cache,
    /// Offline with no cache: quiet sky (kp 0, the old `kpNow` default).
    Fallback,
}

/// A current Kp reading and the aurora intensity it implies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aurora {
    /// Planetary Kp index, 0..9 (0.0 when nothing is known).
    pub kp: f32,
    pub source: KpSource,
}

impl Default for Aurora {
    fn default() -> Self {
        Self {
            kp: 0.0,
            source: KpSource::Fallback,
        }
    }
}

impl Aurora {
    /// Port of `geomagLat` (city.js:1136): the absolute geomagnetic
    /// latitude in degrees, using the IGRF-2020 north geomagnetic pole
    /// (80.65°N, 72.68°W).  The absolute value covers the southern oval —
    /// without it, Hobart or Ushuaia (both deeper inside the australis than
    /// the test site is inside the borealis) would never see a curtain.
    pub fn geomag_lat(lat: f64, lon: f64) -> f64 {
        const DEG: f64 = std::f64::consts::PI / 180.0;
        const P_LAT: f64 = 80.65;
        const P_LON: f64 = -72.68;
        let c = (lat * DEG).sin() * (P_LAT * DEG).sin()
            + (lat * DEG).cos() * (P_LAT * DEG).cos() * ((lon - P_LON) * DEG).cos();
        (90.0 - c.clamp(-1.0, 1.0).acos() / DEG).abs()
    }

    /// Full `auroraLevel` (city.js:1149) port: the 0..1 aurora intensity for
    /// a given Kp at the geographic (lat, lon).  The equatorward edge of the
    /// oval sits at ~67° geomagnetic at Kp 0 and marches ~2.2° equatorward
    /// per Kp step; beyond ~9° past that edge, nothing shows.  So Fairbanks
    /// (~69° geomag) sees it on quiet nights, the test site needs a real storm,
    /// Houston effectively never does.
    pub fn level_for(kp: f32, lat: f64, lon: f64) -> f32 {
        if kp <= 0.0 {
            return 0.0;
        }
        let gm = Self::geomag_lat(lat, lon);
        let edge = 67.0 - 2.2 * kp as f64;
        if gm >= edge {
            return 1.0; // inside the oval
        }
        let below = edge - gm;
        if below < 9.0 {
            (1.0 - below / 9.0).max(0.0) as f32
        } else {
            0.0
        }
    }

    /// The `visible_at(lat)` helper: the level at the given geographic
    /// latitude, read on the geomagnetic pole's meridian — the most
    /// favourable longitude for that latitude (exact for a viewer on that
    /// meridian, e.g. a site at -75).
    pub fn visible_at(&self, lat: f64) -> f32 {
        Self::level_for(self.kp, lat, -72.68)
    }

    /// Whether anything is up at all (old `auroraActive`: level > 0.02).
    pub fn active_at(&self, lat: f64) -> bool {
        self.visible_at(lat) > 0.02
    }
}

/// Parse a recorded SWPC K-index body and return the latest Kp.  Two shapes
/// are accepted, latest row last in both:
///
/// * **current** (2026): an array of objects —
///   `[{"time_tag":"2026-11-03T06:00:00","Kp":2.0,"a_running":7,"station_count":6}, …]`;
/// * **legacy** (what old `city.js` `fetchKp` read, city.js:2660): an array
///   of string rows — `["2026-11-09 00:00:00","2.0", …]`, Kp in column 1.
pub fn parse(body: &str) -> Result<f32, String> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(body).map_err(|e| format!("swpc k-index json: {e}"))?;
    let last = rows.last().ok_or_else(|| "swpc k-index: table is empty".to_string())?;
    let kp = if last.is_object() {
        // current shape: {"Kp": 1.67, …}
        let cell = last
            .get("Kp")
            .ok_or_else(|| "swpc k-index: no Kp field in last row".to_string())?;
        cell
            .as_f64()
            .map(|x| x as f32)
            .or_else(|| cell.as_str().and_then(|s| s.parse::<f32>().ok()))
            .ok_or_else(|| format!("swpc k-index: Kp not a number: {cell}"))?
    } else {
        // legacy shape: ["time", "kp", …]
        let cell = last
            .get(1)
            .ok_or_else(|| "swpc k-index: no Kp column in last row".to_string())?;
        cell
            .as_str()
            .and_then(|s| s.parse::<f32>().ok())
            .or_else(|| cell.as_f64().map(|x| x as f32))
            .ok_or_else(|| format!("swpc k-index: Kp not a number: {cell}"))?
    };
    Ok(kp)
}

/// Fetch the current Kp.  Tries the service, then the cache file, then the
/// quiet-sky fallback (kp 0 → no aurora, old `kpNow` default).  The refresh
/// is 30 minutes ([`crate::feeds::AURORA_REFRESH_MS`]).
pub fn fetch(client: &dyn HttpClient, cache_path: &Path) -> Aurora {
    let url = url_for();
    match client.get(url) {
        Ok(body) => {
            if let Ok(kp) = parse(&body) {
                write_cache(cache_path, &body);
                return Aurora {
                    kp,
                    source: KpSource::Live,
                };
            }
            // malformed → fall through to cache
        }
        Err(_) => {}
    }
    if let Some(body) = read_cache(cache_path) {
        if let Ok(kp) = parse(&body) {
            return Aurora {
                kp,
                source: KpSource::Cache,
            };
        }
    }
    Aurora::default()
}

fn write_cache(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, body);
}

fn read_cache(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join("citylive-world-test").join(format!(
            "{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data")
                .join(name),
        )
        .unwrap()
    }

    const SITE: (f64, f64) = (41.0, -75.0);
    const FAIRBANKS: (f64, f64) = (64.8378, -147.7164);

    #[test]
    fn url_is_old_endpoint() {
        assert_eq!(
            url_for(),
            "https://services.swpc.noaa.gov/products/noaa-planetary-k-index.json"
        );
    }

    #[test]
    fn parse_current_live_sample() {
        // Real saved response (current SWPC shape, latest row last): Kp 2.00.
        assert_eq!(parse(&fixture("noaa-k-index.json")).unwrap(), 2.0);
    }

    #[test]
    fn parse_legacy_string_row_shape() {
        // The shape old `city.js` `fetchKp` read — string rows, Kp in column 1.
        let body = r#"[["time (UTC)","Kp"],["2026-11-09 06:00:00","4.0"]]"#;
        assert_eq!(parse(body).unwrap(), 4.0);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse(r#"{"kp":9}"#).is_err(), "an object, not an array");
        assert!(parse(r#"[["2026-11-09 06:00:00"]]"#).is_err(), "no Kp column");
        assert!(parse(r#"[["2026-11-09 06:00:00","-"]]"#).is_err(), "non-numeric Kp");
        assert!(parse(r#"[{"time_tag":"2026-11-09 06:00:00"}]"#).is_err(), "object row without Kp");
    }

    #[test]
    fn geomag_latitude_matches_old_cities() {
        // the test site ~50-52, Fairbanks ~65+, by the old in-code comments.
        let site = Aurora::geomag_lat(SITE.0, SITE.1);
        assert!((50.0..=52.5).contains(&site), "the test site: {site}");
        let fairbanks = Aurora::geomag_lat(FAIRBANKS.0, FAIRBANKS.1);
        assert!(fairbanks > 65.0, "Fairbanks: {fairbanks}");
        // The southern oval: a southern-mid-latitude city still sits tens of
        // degrees inside it (abs() over the north-pole angle, old comment).
        let hobart = Aurora::geomag_lat(-42.88, 147.33);
        assert!((47.0..=52.0).contains(&hobart), "Hobart: {hobart}");
    }

    #[test]
    fn level_obeys_the_oval_rule() {
        // Kp 0: nothing anywhere, even overhead the pole.
        assert_eq!(Aurora::level_for(0.0, 90.0, -72.68), 0.0);
        // Quiet night: Fairbanks inside the oval, the test site (50.8° geomag) not.
        assert_eq!(Aurora::level_for(3.0, FAIRBANKS.0, FAIRBANKS.1), 1.0);
        assert_eq!(
            Aurora::level_for(3.0, SITE.0, SITE.1),
            0.0,
            "the test site ~9.6° past the Kp-3 edge → nothing"
        );
        // Severe storm: the oval reaches the test site.
        let n = Aurora::level_for(5.0, SITE.0, SITE.1);
        assert!((0.35..=0.5).contains(&n), "Kp-5 the test site level: {n}");
        assert_eq!(Aurora::level_for(8.0, SITE.0, SITE.1), 1.0);
    }

    #[test]
    fn visible_at_helper() {
        let a = Aurora {
            kp: 5.0,
            source: KpSource::Live,
        };
        // On the pole's meridian: 50.8° geomag vs a 56° edge → ~0.37.
        let v = a.visible_at(SITE.0);
        assert!((0.34..=0.46).contains(&v), "Kp-5 visible_at(the test site): {v}");
        assert!(a.active_at(SITE.0));
        let quiet = Aurora {
            kp: 0.0,
            source: KpSource::Fallback,
        };
        assert_eq!(quiet.visible_at(70.0), 0.0);
        assert!(!quiet.active_at(70.0));
    }

    #[test]
    fn fetch_uses_network_then_cache_then_fallback() {
        let d = tmpdir("kp");
        let cache = d.join("kp.json");

        // Live.
        let body = fixture("noaa-k-index.json");
        let client = crate::http::tests::MapClient::new().ok(url_for(), &body);
        let a = fetch(&client, &cache);
        assert_eq!(a.kp, 2.0);
        assert_eq!(a.source, KpSource::Live);
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), body, "cache written");

        // Offline → cache.
        let client = crate::http::tests::MapClient::new();
        let a = fetch(&client, &cache);
        assert_eq!(a.kp, 2.0);
        assert_eq!(a.source, KpSource::Cache);

        // Offline, no cache → quiet-sky fallback.
        let d2 = tmpdir("kp2");
        let a = fetch(&client, &d2.join("kp.json"));
        assert_eq!(a, Aurora::default(), "fallback is kp 0");
        let _ = (d, d2);
    }
}
