//! The no-key HTTPS IP lookup services, tried in order. Each is a plain GET to
//! a fixed URL; nothing about the user goes in the request.

use serde_json::Value;

/// One service: where to ask and what its answer's field names are.
pub struct IpService {
    pub name: &'static str,
    pub url: &'static str,
    city: &'static str,
    region: &'static str,
    country: &'static str,
}

/// Tried in this order; the first good answer wins.
pub const SERVICES: [IpService; 4] = [
    IpService { name: "ipapi.co", url: "https://ipapi.co/json", city: "city", region: "region", country: "country_name" },
    IpService { name: "ipwho.is", url: "https://ipwho.is/", city: "city", region: "region", country: "country" },
    IpService {
        name: "geojs.io",
        url: "https://get.geojs.io/v1/ip/geo.json",
        city: "city",
        region: "region",
        country: "country",
    },
    IpService {
        name: "reallyfreegeoip.org",
        url: "https://reallyfreegeoip.org/json/",
        city: "city",
        region: "region_name",
        country: "country_name",
    },
];

/// A JSON number, or a number inside a string (geojs.io sends strings).
fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    let s = v.get(key)?.as_str()?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Parse one service's answer into (lat, lon, "City, Region, Country").
pub fn parse_service(svc: &IpService, body: &str) -> Result<(f64, f64, String), String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("{}: bad json: {e}", svc.name))?;
    // ipwho.is says `"success": false` with a message; ipapi.co says `"error": true`.
    if v.get("success").and_then(Value::as_bool) == Some(false)
        || v.get("error").and_then(Value::as_bool) == Some(true)
    {
        return Err(format!("{}: the service refused", svc.name));
    }
    let lat = num(v.get("latitude")).ok_or_else(|| format!("{}: no latitude", svc.name))?;
    let lon = num(v.get("longitude")).ok_or_else(|| format!("{}: no longitude", svc.name))?;
    let mut parts: Vec<String> = Vec::new();
    for key in [svc.city, svc.region, svc.country] {
        if let Some(t) = text(&v, key) {
            if !parts.contains(&t) {
                parts.push(t);
            }
        }
    }
    let name = if parts.is_empty() { crate::coord_name(lat, lon) } else { parts.join(", ") };
    Ok((lat, lon, name))
}
