//! The real ISS: its live sub-satellite point (open-notify, no key) and
//! where it looks from *you* — port of old `city.js` "THE REAL ISS"
//! (city.js:2811-2865).  The old code asked over plain http (CORS-open); the
//! same host speaks https, which is what [`crate::http::UreqClient`]
//! supports, so the endpoint is the https form.  Refresh is ~25 s
//! ([`crate::feeds::ISS_REFRESH_MS`]).  When the feed is unreachable,
//! [`fetch`] returns `None` and a renderer's scripted pass stands in — the
//! old behaviour.

use serde::Deserialize;
use std::fs;
use std::path::Path;

use crate::http::HttpClient;

const EARTH_RADIUS_KM: f64 = 6371.0;
const ISS_ALT_KM: f64 = 420.0; // the old `issLookAngle` orbital altitude

/// open-notify's current ISS position (old `fetchISS` endpoint, https form).
pub fn url_for() -> &'static str {
    "https://api.open-notify.org/iss-now.json"
}

/// The maintained no-key alternative (`wheretheiss.at`, the ISS's NORAD id):
/// the same sub-satellite point in a different shape. The old endpoint has been
/// refused by its host for years (H44 measured connection-refused from two
/// networks, and it is still dead in 2026), so this is tried second: the old
/// URL stays first, so nothing that works today changes behaviour.
pub fn alt_url_for() -> &'static str {
    "https://api.wheretheiss.at/v1/satellites/25544"
}

/// The ISS's sub-satellite point at an instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Iss {
    /// Sub-satellite latitude, degrees.
    pub lat: f64,
    /// Sub-satellite longitude, degrees.
    pub lon: f64,
    /// Unix ms this read is as of (the server `timestamp` when present,
    /// else the fetch time).
    pub ts: i64,
}

/// Look angle from a viewer to the point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Altitude above the horizon, degrees (< 0 = below the horizon).
    pub alt: f64,
    /// Azimuth, degrees (0 = N, clockwise through E).
    pub az: f64,
}

// ---- open-notify response shapes ----

#[derive(Debug, Deserialize)]
struct IssNow {
    #[serde(default)]
    timestamp: Option<i64>,
    iss_position: Option<IssPosition>,
}

#[derive(Debug, Deserialize)]
struct IssPosition {
    latitude: Option<String>,
    longitude: Option<String>,
}

/// Parse a recorded open-notify `iss-now.json` body.  `now_ms` (the fetch
/// time) is used as `ts` when the body carries no `timestamp`.
pub fn parse(body: &str, now_ms: i64) -> Result<Iss, String> {
    let v: IssNow =
        serde_json::from_str(body).map_err(|e| format!("open-notify json: {e}"))?;
    let pos = v
        .iss_position
        .as_ref()
        .ok_or_else(|| "open-notify: missing iss_position".to_string())?;
    let lat = pos
        .latitude
        .as_deref()
        .map(str::trim)
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or_else(|| "open-notify: bad latitude".to_string())?;
    let lon = pos
        .longitude
        .as_deref()
        .map(str::trim)
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or_else(|| "open-notify: bad longitude".to_string())?;
    let ts = v.timestamp.map(|t| t * 1000).unwrap_or(now_ms);
    Ok(Iss { lat, lon, ts })
}

impl Iss {
    /// Port of `issLookAngle` (city.js:2847): the topocentric look angle to
    /// the sub-point from a viewer on a spherical Earth, ISS altitude 420 km.
    /// `alt > 0` ⇔ it is above the *viewer's* horizon right now.
    pub fn look(&self, v_lat: f64, v_lon: f64) -> Look {
        let d2r = std::f64::consts::PI / 180.0;
        let r2d = 180.0 / std::f64::consts::PI;
        let rho = EARTH_RADIUS_KM / (EARTH_RADIUS_KM + ISS_ALT_KM);
        let lo = v_lat * d2r;
        let ls = self.lat * d2r;
        let dl = (self.lon - v_lon) * d2r;
        let cg = (lo.sin() * ls.sin() + lo.cos() * ls.cos() * dl.cos()).clamp(-1.0, 1.0);
        let sg = (1.0 - cg * cg).sqrt().max(1e-6);
        let alt = (cg - rho).atan2(sg) * r2d;
        let az = (dl.sin() * ls.cos())
            .atan2(lo.cos() * ls.sin() - lo.sin() * ls.cos() * dl.cos())
            * r2d;
        Look {
            alt,
            az: ((az % 360.0) + 360.0) % 360.0,
        }
    }

    /// The brief's `overhead_pass`: true while the ISS is above 10° altitude
    /// from the viewer — "it's REALLY up there" (old comment).
    pub fn overhead_pass(&self, v_lat: f64, v_lon: f64) -> bool {
        self.look(v_lat, v_lon).alt > 10.0
    }

    /// Dead-reckon between two reads (old `issAltAzNow`, city.js:2858): the
    /// ground rate from `prev` to `cur`, projected forward clamped to
    /// -8..+45 s, so the 25 s fetch cycle yields a smooth trail instead of
    /// hops.
    pub fn dead_reckon(prev: &Iss, cur: &Iss, now_ms: i64) -> Iss {
        let span = ((cur.ts - prev.ts) / 1000).max(1) as f64;
        let mut dlo = cur.lon - prev.lon;
        if dlo > 180.0 {
            dlo -= 360.0;
        }
        if dlo < -180.0 {
            dlo += 360.0;
        }
        let vlat = (cur.lat - prev.lat) / span;
        let vlon = dlo / span;
        let dt = ((now_ms - cur.ts) / 1000).clamp(-8, 45) as f64;
        Iss {
            lat: cur.lat + vlat * dt,
            lon: cur.lon + vlon * dt,
            ts: now_ms,
        }
    }
}

/// Parse a `wheretheiss.at` body (the alternative endpoint above). Only the
/// position and the time are taken: the eclipse is worked out from the sun at
/// the sub-point, which is the same physics and does not trust a flag.
pub fn parse_alt(body: &str, now_ms: i64) -> Result<Iss, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("wheretheiss json: {e}"))?;
    let num = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_f64())
            .ok_or_else(|| format!("wheretheiss: no {k}"))
    };
    let ts = v
        .get("timestamp")
        .and_then(|t| t.as_i64())
        .map(|t| t * 1000)
        .unwrap_or(now_ms);
    Ok(Iss {
        lat: num("latitude")?,
        lon: num("longitude")?,
        ts,
    })
}

/// Fetch the current sub-satellite point.  Tries the service, then the
/// cache file, then `None` (a scripted pass stands in — old behaviour).
/// The refresh runs on the ~25 s bucket
/// ([`crate::feeds::ISS_REFRESH_MS`]).
pub fn fetch(client: &dyn HttpClient, now_ms: i64, cache_path: &Path) -> Option<Iss> {
    let url = url_for();
    match client.get(url) {
        Ok(body) => {
            if let Ok(iss) = parse(&body, now_ms) {
                write_cache(cache_path, &body);
                return Some(iss);
            }
            // malformed → fall through to cache
        }
        Err(_) => {}
    }
    // The old host is dead; ask the maintained one before falling back. (A
    // stale cache is a *last* resort: a live fix beats yesterday's.)
    if let Ok(body) = client.get(alt_url_for()) {
        if let Ok(iss) = parse_alt(&body, now_ms) {
            return Some(iss);
        }
    }
    read_cache(cache_path).and_then(|body| parse(&body, now_ms).ok())
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
    const NOW_MS: i64 = 1_762_694_400_000; // matches the fixture timestamp

    #[test]
    fn url_is_https_open_notify() {
        assert_eq!(url_for(), "https://api.open-notify.org/iss-now.json");
    }

    #[test]
    fn parse_reads_sub_point_and_timestamp() {
        let iss = parse(&fixture("iss-now.json"), NOW_MS).unwrap();
        assert!((iss.lat - 41.0).abs() < 1e-9);
        assert!((iss.lon - -65.0).abs() < 1e-9);
        assert_eq!(iss.ts, NOW_MS, "server timestamp wins");
    }

    #[test]
    fn parse_falls_back_to_fetch_time() {
        let body = r#"{"message":"success","iss_position":{"latitude":"10.5","longitude":"-20.5"}}"#;
        let iss = parse(body, 42_000).unwrap();
        assert_eq!(iss.ts, 42_000);
    }

    #[test]
    fn look_angle_nearby_point() {
        // Sub-point 10° of longitude east of the viewer (fixture): a high
        // arc — ~22° altitude, overhead pass true.
        let iss = parse(&fixture("iss-now.json"), NOW_MS).unwrap();
        let look = iss.look(SITE.0, SITE.1);
        assert!((21.5..=23.0).contains(&look.alt), "alt {look:?}");
        // A point 10° east on the same latitude sits NNE-E, not dead east.
        assert!((85.0..=88.0).contains(&look.az), "az {look:?}");
        assert!(iss.overhead_pass(SITE.0, SITE.1));
    }

    #[test]
    fn look_angle_far_point_is_below_horizon() {
        // A sub-point 60° away (same latitude): below the horizon.
        let iss = Iss {
            lat: SITE.0,
            lon: SITE.1 - 60.0,
            ts: NOW_MS,
        };
        let look = iss.look(SITE.0, SITE.1);
        assert!(look.alt < 0.0, "alt {look:?}");
        assert!(!iss.overhead_pass(SITE.0, SITE.1));
    }

    #[test]
    fn look_at_the_nadir_point_is_near_zenith() {
        let iss = Iss {
            lat: SITE.0,
            lon: SITE.1,
            ts: NOW_MS,
        };
        let look = iss.look(SITE.0, SITE.1);
        assert!(look.alt > 89.0, "alt {look:?}");
    }

    #[test]
    fn dead_reckon_extrapolates_with_wrapped_longitude() {
        // Two reads 25 s apart (lat +0.25°, lon +0.2° with a dateline wrap),
        // so the ground rate is 0.01°/s N and 0.008°/s E; the old code
        // clamps the projection to -8..+45 s.
        let prev = Iss {
            lat: 41.0,
            lon: 179.9,
            ts: NOW_MS - 25_000,
        };
        let cur = Iss {
            lat: 41.25,
            lon: -179.9, // +0.2° east, wrapped across the dateline
            ts: NOW_MS,
        };
        let pos = Iss::dead_reckon(&prev, &cur, NOW_MS + 45_000); // max clamp
        assert!((pos.lat - 41.7).abs() < 1e-9, "lat {pos:?}");
        assert!((pos.lon - -179.54).abs() < 1e-9, "lon {pos:?}");
        let back = Iss::dead_reckon(&prev, &cur, NOW_MS - 8_000); // min clamp
        assert!((back.lat - 41.17).abs() < 1e-9, "lat {back:?}");
        assert!((back.lon - -179.964).abs() < 1e-9, "lon {back:?}");
    }

    #[test]
    fn fetch_uses_network_then_cache_then_none() {
        let d = tmpdir("iss");
        let cache = d.join("iss.json");

        let body = fixture("iss-now.json");
        let client = crate::http::tests::MapClient::new().ok(url_for(), &body);
        let iss = fetch(&client, NOW_MS, &cache);
        assert_eq!(iss.unwrap().lat, 41.0);
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), body, "cache written");

        // Offline → cache.
        let client = crate::http::tests::MapClient::new();
        let iss = fetch(&client, NOW_MS, &cache);
        assert_eq!(iss.unwrap().lon, -65.0);

        // Offline, no cache → None (scripted pass stands in).
        let d2 = tmpdir("iss2");
        assert!(fetch(&client, NOW_MS, &d2.join("iss.json")).is_none());
        let _ = (d, d2);
    }

    #[test]
    fn the_alternative_shape_parses() {
        let body = include_str!("../tests/data/iss-wheretheiss.json");
        let iss = parse_alt(body, 1).expect("wheretheiss body");
        // A real capture: the position moves, so only the ranges are fixed.
        assert!(iss.lat.abs() <= 52.0, "lat {}", iss.lat);
        assert!(iss.lon.abs() <= 180.0, "lon {}", iss.lon);
        // The body's own timestamp is seconds; the reading is in ms.
        assert!(iss.ts > 1_700_000_000_000, "ts {}", iss.ts);
    }
}
