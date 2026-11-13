//! Place search with Open-Meteo's free geocoding API (no key).

use serde_json::Value;
use world::HttpClient;

/// One search result.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// Percent-encode a query for a URL (UTF-8 bytes, unreserved kept).
fn encode(q: &str) -> String {
    let mut out = String::new();
    for b in q.trim().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn search_url(query: &str) -> String {
    format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=8&language=en&format=json",
        encode(query)
    )
}

/// Parse the geocoding answer into places ("Name, Region, Country").
pub fn parse_search(body: &str) -> Result<Vec<Place>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("search json: {e}"))?;
    // No match at all comes back as `{}` with no `results`.
    let Some(results) = v.get("results").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for r in results {
        let (Some(lat), Some(lon)) =
            (r.get("latitude").and_then(Value::as_f64), r.get("longitude").and_then(Value::as_f64))
        else {
            continue;
        };
        if !crate::valid(lat, lon) {
            continue;
        }
        let mut parts: Vec<String> = Vec::new();
        for key in ["name", "admin1", "country"] {
            if let Some(t) = r.get(key).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) {
                if !parts.iter().any(|p| p == t) {
                    parts.push(t.to_string());
                }
            }
        }
        if parts.is_empty() {
            continue;
        }
        out.push(Place { name: parts.join(", "), lat, lon });
    }
    Ok(out)
}

/// Search for a place by name. A query under two characters sends nothing.
pub fn search(http: &dyn HttpClient, query: &str) -> Result<Vec<Place>, String> {
    if query.trim().chars().count() < 2 {
        return Ok(Vec::new());
    }
    let body = http.get(&search_url(query)).map_err(|e| e.to_string())?;
    parse_search(&body)
}
