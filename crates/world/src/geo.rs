//! IP geolocation (D16): no-key HTTPS, two services tried in order
//! (ipapi.co then ipwho.is), cached to a file, manual override first, and a
//! Greenwich, London fallback.  No request carries any personal data — each is a
//! plain GET to a fixed URL.

use serde::Deserialize;
use std::fs;
use std::path::Path;

use crate::http::HttpClient;

/// Final fallback location when offline with no cache (D16).
pub const DEFAULT_LATLON: (f64, f64) = (51.48, 0.0);
const DEFAULT_NAME: &str = "Greenwich, London";

/// Services tried in order.
const SERVICES: [&str; 2] = ["https://ipapi.co/json", "https://ipwho.is/"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeoSource {
    /// A manual override from settings won.
    Override,
    /// A fresh response from one of the live services.
    Ip,
    /// Re-read from the last good cached response.
    Cache,
    /// Greenwich, London fallback.
    Fallback,
}

#[derive(Debug, Clone)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
    pub name: String,
    pub source: GeoSource,
}

impl Location {
    /// Build a manual override (settings app calls this).
    pub fn override_loc(lat: f64, lon: f64, name: &str) -> Self {
        Self {
            lat,
            lon,
            name: name.to_string(),
            source: GeoSource::Override,
        }
    }

    pub fn default_place() -> Self {
        Self {
            lat: DEFAULT_LATLON.0,
            lon: DEFAULT_LATLON.1,
            name: DEFAULT_NAME.to_string(),
            source: GeoSource::Fallback,
        }
    }
}

#[derive(Debug, Deserialize)]
struct IpBody {
    latitude: Option<f64>,
    longitude: Option<f64>,
    city: Option<String>,
    region: Option<String>,
    country: Option<String>,
}

fn name_of(b: &IpBody) -> String {
    let mut parts = Vec::new();
    if let Some(c) = &b.city {
        if !c.is_empty() {
            parts.push(c.clone());
        }
    }
    if let Some(r) = &b.region {
        if !r.is_empty() {
            parts.push(r.clone());
        }
    }
    if let Some(ct) = &b.country {
        if !ct.is_empty() {
            parts.push(ct.clone());
        }
    }
    if parts.is_empty() {
        "unknown".to_string()
    } else {
        parts.join(", ")
    }
}

/// Parse an ipapi.co / ipwho.is body into (lat, lon, name).
pub fn parse(body: &str) -> Result<(f64, f64, String), String> {
    let b: IpBody = serde_json::from_str(body).map_err(|e| format!("geo json: {e}"))?;
    let lat = b.latitude.ok_or_else(|| "geo: missing latitude".to_string())?;
    let lon = b.longitude.ok_or_else(|| "geo: missing longitude".to_string())?;
    Ok((lat, lon, name_of(&b)))
}

fn read_cache(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn write_cache(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, body);
}

/// Resolve the viewer's location.
/// * A manual `override_loc` (settings) always wins and is returned as-is
///   (with its `source` forced to [`GeoSource::Override`]).
/// * Else try `ipapi.co` then `ipwho.is`; a good response is cached.
/// * Else read the last good cached response.
/// * Else the [`Location::default_place`] fallback.
pub fn locate(client: &dyn HttpClient, override_loc: Option<&Location>, cache_path: &Path) -> Location {
    if let Some(o) = override_loc {
        let mut l = o.clone();
        l.source = GeoSource::Override;
        return l;
    }
    for &url in &SERVICES {
        if let Ok(body) = client.get(url) {
            if let Ok((lat, lon, name)) = parse(&body) {
                write_cache(cache_path, &body);
                return Location {
                    lat,
                    lon,
                    name,
                    source: GeoSource::Ip,
                };
            }
        }
    }
    if let Some(body) = read_cache(cache_path) {
        if let Ok((lat, lon, name)) = parse(&body) {
            return Location {
                lat,
                lon,
                name,
                source: GeoSource::Cache,
            };
        }
    }
    Location::default_place()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::tests::MapClient;

    const IPAPI: &str = "https://ipapi.co/json";
    const IPWHO: &str = "https://ipwho.is/";
    const IPAPI_BODY: &str = r#"{"ip":"1.2.3.4","city":"Greenwich","region":"England","country":"United Kingdom","latitude":51.4826,"longitude":-0.0077}"#;
    const IPWHO_BODY: &str = r#"{"ip":"5.6.7.8","success":true,"city":"Brooklyn","region":"New York","country":"United States","latitude":40.6782,"longitude":-73.9442}"#;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir()
            .join("citylive-world-geo")
            .join(format!("{}-{}-{}", tag, std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parse_geo() {
        let (lat, lon, name) = parse(IPAPI_BODY).unwrap();
        assert!((lat - 51.4826).abs() < 1e-6);
        assert!((lon + 0.0077).abs() < 1e-6);
        assert_eq!(name, "Greenwich, England, United Kingdom");
    }

    #[test]
    fn locate_success_uses_first_service() {
        let d = tmpdir("ok");
        let cache = d.join("geo.json");
        let client = MapClient::new().ok(IPAPI, IPAPI_BODY);
        let l = locate(&client, None, &cache);
        assert_eq!(l.source, GeoSource::Ip);
        assert!((l.lat - 51.4826).abs() < 1e-6);
        assert_eq!(client.called_urls(), vec![IPAPI.to_string()]);
        // cached for next time
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), IPAPI_BODY);
        let _ = d;
    }

    #[test]
    fn locate_failover_to_second_service() {
        let d = tmpdir("failover");
        let cache = d.join("geo.json");
        let client = MapClient::new()
            .with_log(IPAPI, Err("first service down"))
            .with_log(IPWHO, Ok(IPWHO_BODY));
        let l = locate(&client, None, &cache);
        assert_eq!(l.source, GeoSource::Ip);
        assert!((l.lat - 40.6782).abs() < 1e-6, "should come from ipwho.is: {l:?}");
        assert_eq!(
            client.called_urls(),
            vec![IPAPI.to_string(), IPWHO.to_string()]
        );
        let _ = d;
    }

    #[test]
    fn locate_offline_uses_cache() {
        let d = tmpdir("cache");
        let cache = d.join("geo.json");
        std::fs::write(&cache, IPWHO_BODY).unwrap();
        let client = MapClient::new(); // all network Err
        let l = locate(&client, None, &cache);
        assert_eq!(l.source, GeoSource::Cache, "{l:?}");
        assert!((l.lat - 40.6782).abs() < 1e-6);
        let _ = d;
    }

    #[test]
    fn locate_offline_no_cache_falls_back_to_default() {
        let d = tmpdir("no-cache");
        let client = MapClient::new();
        let l = locate(&client, None, &d.join("geo.json"));
        assert_eq!(l.source, GeoSource::Fallback, "{l:?}");
        assert!((l.lat - DEFAULT_LATLON.0).abs() < 1e-9);
        assert!((l.lon - DEFAULT_LATLON.1).abs() < 1e-9);
        let _ = d;
    }

    #[test]
    fn override_takes_precedence() {
        let d = tmpdir("override");
        let client = MapClient::new().ok(IPAPI, IPAPI_BODY); // should NEVER be used
        let ov = Location::override_loc(-33.0, 151.0, "Sydney");
        let l = locate(&client, Some(&ov), &d.join("geo.json"));
        assert_eq!(l.source, GeoSource::Override);
        assert!((l.lat + 33.0).abs() < 1e-9);
        assert!(client.called_urls().is_empty(), "override must not hit the network");
        let _ = d;
    }
}
