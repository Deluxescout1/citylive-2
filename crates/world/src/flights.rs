//! Live flights: data layer for the old CityLive live-aircraft overlay
//! (city.js:2670-2728, "LIVE FLIGHTS"), on the free no-key ADS-B feed
//! (airplanes.live).  The API pre-computes `dir` (bearing viewer→aircraft)
//! and `dst` (distance, nautical miles), so each record carries a local
//! East/North offset from the viewer (metres) alongside position, altitude,
//! heading and speed.
//!
//! Same discipline as the old `fetchFlights`: shared-bucket refresh (90 s,
//! see [`crate::feeds`]), never throws — any failure leaves the last list
//! (or nothing) so the overlay degrades silently, no personal data.  The
//! per-aircraft motion smoothing / dead-reckoning the old renderer did
//! between the sparse snapshots (`flightSmooth` / `drawRealFlights`)
//! belongs to a renderer; this module is data only.

use serde::Deserialize;
use std::fs;
use std::path::Path;

use crate::http::HttpClient;

/// Default search radius, nautical miles (old `FL_RADIUS = 45`, ~83 km of
/// local air; the API itself caps around 250 nm).
pub const DEFAULT_RADIUS_NM: u8 = 45;
/// Metres in a foot (the feed reports altitude in feet).
pub const FEET_TO_M: f64 = 0.3048;
/// Cap the list at the N nearest (old `realFlights` cap).
pub const MAX_AIRCRAFT: usize = 20;

/// A single aircraft near the viewer, from one ADS-B snapshot.
#[derive(Debug, Clone)]
pub struct Flight {
    /// Callsign, e.g. `DAL1234` (upper-cased; `FLIGHT` when unknown).
    pub callsign: String,
    /// ICAO 24-bit address in hex (stable between snapshots; may be empty).
    pub hex: String,
    /// Type string from the feed (e.g. `A320N`).
    pub plane: String,
    /// ICAO category: `A1` light, `A3` airliner, `A5` heavy, `A7` rotor…
    pub category: String,
    /// Geodetic position, degrees.
    pub lat: f64,
    pub lon: f64,
    /// Local East/North offset from the viewer, metres (flat vector, as the
    /// old code built it from `dir`/`dst`).
    pub east_m: f64,
    pub north_m: f64,
    /// Barometric altitude, metres (grounded aircraft are dropped).
    pub altitude_m: f32,
    /// Track (heading), degrees — falls back to the bearing to the aircraft
    /// when the feed omits it.
    pub heading: f32,
    /// Ground speed, knots.
    pub speed_kt: f32,
    /// Barometric vertical rate, ft/min.
    pub baro_rate: f32,
    /// Bearing from the viewer to the aircraft, degrees.
    pub bearing: f32,
    /// Distance from the viewer, metres.
    pub distance_m: f64,
}

/// The airplanes.live point query (old `fetchFlights` URL): aircraft within
/// `radius_nm` nautical miles of (lat, lon).  No key, no personal data.
pub fn url_for(lat: f64, lon: f64, radius_nm: u8) -> String {
    format!("https://api.airplanes.live/v2/point/{lat:.4}/{lon:.4}/{radius_nm}")
}

// ---- airplanes.live response shapes ----

#[derive(Debug, Deserialize)]
struct AdsbResponse {
    ac: Option<Vec<Aircraft>>,
}

#[derive(Debug, Deserialize)]
struct Aircraft {
    flight: Option<String>,
    r: Option<String>,
    t: Option<String>,
    category: Option<String>,
    hex: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    alt_baro: Option<serde_json::Value>,
    gs: Option<f32>,
    track: Option<f32>,
    dir: Option<f32>,
    dst: Option<f64>,
    baro_rate: Option<f32>,
}

/// Parse a recorded airplanes.live body (the old `fetchFlights` parse
/// block).  Grounded aircraft (alt `"ground"`/missing) and records missing
/// bearing/distance/position are dropped, and the result is the
/// [`MAX_AIRCRAFT`] nearest, sorted nearest-first.
pub fn parse(body: &str) -> Result<Vec<Flight>, String> {
    let v: AdsbResponse =
        serde_json::from_str(body).map_err(|e| format!("airplanes.live json: {e}"))?;
    let mut out = Vec::new();
    for a in v.ac.as_deref().unwrap_or(&[]) {
        // Parked / taxiing aircraft are out (old code skip rule).
        let alt = match &a.alt_baro {
            Some(serde_json::Value::Number(n)) => n.as_f64(),
            Some(serde_json::Value::String(s)) if s != "ground" => s.parse::<f64>().ok(),
            _ => None,
        };
        let (alt, dir, dst, lat) = match (alt, a.dir, a.dst, a.lat) {
            (Some(alt), Some(dir), Some(dst), Some(lat)) => (alt, dir, dst, lat),
            _ => continue,
        };
        let dst_m = dst * 1852.0; // nm → m
        let azr = (dir as f64) * std::f64::consts::PI / 180.0; // bearing → radians
        let callsign = a
            .flight
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| a.r.as_deref().map(str::trim).filter(|s| !s.is_empty()))
            .map(|s| s.to_uppercase())
            .unwrap_or_else(|| "FLIGHT".to_string());
        out.push(Flight {
            callsign,
            hex: a.hex.clone().unwrap_or_default(),
            plane: a.t.clone().unwrap_or_default(),
            category: a.category.clone().unwrap_or_default(),
            lat,
            lon: a.lon.unwrap_or(0.0),
            east_m: dst_m * azr.sin(),
            north_m: dst_m * azr.cos(),
            // B93: the feed's `alt_baro` is in **feet**; the field (and the sky's
            // elevation and contrail maths) are metres. It was passed through
            // unconverted, so every aircraft sat 3.3x too high.
            altitude_m: (alt * FEET_TO_M) as f32,
            heading: a.track.unwrap_or(dir),
            speed_kt: a.gs.unwrap_or(0.0),
            baro_rate: a.baro_rate.unwrap_or(0.0),
            bearing: dir,
            distance_m: dst_m,
        });
    }
    out.sort_by(|p, q| {
        p.distance_m
            .partial_cmp(&q.distance_m)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(MAX_AIRCRAFT);
    Ok(out)
}

/// Fetch the current snapshot.  Tries the network, then the cache file,
/// then nothing (the overlay simply draws no aircraft — old behaviour).
/// On a successful fetch the raw body is written to `cache_path`.  The
/// refresh runs on a ≥90 s shared bucket ([`crate::feeds::FLIGHTS_REFRESH_MS`],
/// above the brief's 30 s floor).
pub fn fetch(
    client: &dyn HttpClient,
    lat: f64,
    lon: f64,
    radius_nm: u8,
    cache_path: &Path,
) -> Vec<Flight> {
    let url = url_for(lat, lon, radius_nm);
    match client.get(&url) {
        Ok(body) => {
            if let Ok(flights) = parse(&body) {
                write_cache(cache_path, &body);
                return flights;
            }
            // malformed → fall through to cache
        }
        Err(_) => {}
    }
    if let Some(body) = read_cache(cache_path) {
        if let Ok(flights) = parse(&body) {
            return flights;
        }
    }
    Vec::new()
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

    #[test]
    fn url_matches_old_feed() {
        assert_eq!(
            url_for(41.0, -75.0, DEFAULT_RADIUS_NM),
            "https://api.airplanes.live/v2/point/41.0000/-75.0000/45"
        );
    }

    #[test]
    fn parse_keeps_airborne_drops_grounded_and_incomplete() {
        let flights = parse(&fixture("airplanes-live.json")).unwrap();
        // 4 records: one grounded (alt "ground"), one missing dir/dst/alt.
        assert_eq!(flights.len(), 2);
        // Nearest first.
        assert_eq!(flights[0].callsign, "DAL1234");
        assert_eq!(flights[0].hex, "a1b2c3");
        assert_eq!(flights[0].category, "A3A");
        // The feed's 340.2 is feet (B93): 103.7 m.
        assert!((flights[0].altitude_m - 340.2 * 0.3048).abs() < 0.01);
        assert!((flights[0].heading - 90.0).abs() < 0.01);
        assert!((flights[0].speed_kt - 152.4).abs() < 0.01);
        assert!((flights[0].distance_m - 0.42 * 1852.0).abs() < 0.5);
        // Local East/North vector from the old math (`dir` is an f32 from the
        // feed, so allow a hair of rounding).
        let azr = 45.2_f64.to_radians();
        let dst_m = 0.42 * 1852.0;
        assert!((flights[0].east_m - dst_m * azr.sin()).abs() < 1e-3);
        assert!((flights[0].north_m - dst_m * azr.cos()).abs() < 1e-3);
        assert_eq!(flights[1].callsign, "AAL777");
        assert!((flights[1].distance_m - 2.1 * 1852.0).abs() < 0.5);
    }

    /// B93: a cruising airliner at 35,000 ft is 10,668 m up, not 35,000.
    #[test]
    fn altitude_is_converted_from_feet() {
        let body = r#"{"ac":[{"hex":"abc","flight":"UAL1","lat":1.0,"lon":2.0,"alt_baro":35000,
            "dir":0.0,"dst":10.0,"track":10.0,"gs":450.0}]}"#;
        let f = parse(body).unwrap();
        assert!((f[0].altitude_m - 10_668.0).abs() < 0.5, "got {}", f[0].altitude_m);
    }

    #[test]
    fn parse_callsign_falls_back_to_registration() {
        let body = r#"{
            "ac": [
                { "r": "n123ua", "lat": 1.0, "lon": 2.0, "alt_baro": 100.0,
                  "dir": 0.0, "dst": 1.0, "track": 10.0, "gs": 50.0 }
            ]
        }"#;
        let flights = parse(body).unwrap();
        assert_eq!(flights.len(), 1);
        assert_eq!(flights[0].callsign, "N123UA", "no flight # → upper-cased registration");
        assert_eq!(flights[0].hex, "");
    }

    #[test]
    fn fetch_success_writes_cache() {
        let d = tmpdir("flights-ok");
        let cache = d.join("flights.json");
        let body = fixture("airplanes-live.json");
        let url = url_for(41.0, -75.0, DEFAULT_RADIUS_NM);
        let client = crate::http::tests::MapClient::new().ok(&url, &body);
        let flights = fetch(&client, 41.0, -75.0, DEFAULT_RADIUS_NM, &cache);
        assert_eq!(flights.len(), 2);
        let cached = std::fs::read_to_string(&cache).unwrap();
        assert_eq!(cached, body, "cache holds the raw body");
        let _ = d;
    }

    #[test]
    fn fetch_offline_uses_cache() {
        let d = tmpdir("flights-cache");
        let cache = d.join("flights.json");
        std::fs::write(&cache, fixture("airplanes-live.json")).unwrap();
        let client = crate::http::tests::MapClient::new(); // nothing served
        let flights = fetch(&client, 51.5, 0.0, DEFAULT_RADIUS_NM, &cache);
        assert_eq!(flights.len(), 2, "serves the cached snapshot: {flights:?}");
        assert_eq!(flights[0].callsign, "DAL1234");
        let _ = d;
    }

    #[test]
    fn fetch_offline_no_cache_is_empty() {
        let d = tmpdir("flights-none");
        let cache = d.join("flights.json"); // absent
        let client = crate::http::tests::MapClient::new();
        let flights = fetch(&client, 51.5, 0.0, DEFAULT_RADIUS_NM, &cache);
        assert!(flights.is_empty(), "offline + no cache → no aircraft");
        let _ = d;
    }
}
