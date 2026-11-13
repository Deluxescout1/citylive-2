//! One live weather fetch, printed (B27 item 1 / `H27-live.txt`).
//!
//! ```text
//! cargo run --release -p world --features net --example live            # IP location
//! cargo run --release -p world --features net --example live -- 51.48 0.0
//! ```
//!
//! Prints the location, the request URL, the parsed snapshot and the raw body —
//! no personal data (the IP city is whatever the geolocation service says, and
//! the coordinates are the ones used).

use std::path::PathBuf;
use world::HttpClient;

#[cfg(not(feature = "net"))]
fn main() {
    eprintln!("this example needs --features net");
}

#[cfg(feature = "net")]
fn main() {
    let mut args = std::env::args().skip(1);
    let lat = args.next().and_then(|v| v.parse::<f64>().ok());
    let lon = args.next().and_then(|v| v.parse::<f64>().ok());
    let client = world::UreqClient::new();
    let cache = std::env::temp_dir().join("citylive-open-meteo-cache.json");
    let geo_cache = std::env::temp_dir().join("citylive-geo-cache.json");

    let loc = match (lat, lon) {
        (Some(a), Some(b)) => world::Location::override_loc(a, b, "given on the command line"),
        _ => world::locate(&client, None, &geo_cache),
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let url = world::weather_url(loc.lat, loc.lon);
    println!("location: {} ({}, {}) via {:?}", loc.name, loc.lat, loc.lon, loc.source);
    println!("url: {url}");
    let body = match client.get(&url) {
        Ok(b) => Some(b),
        Err(e) => {
            println!("fetch failed: {e}");
            None
        }
    };
    if let Some(b) = &body {
        println!("body: {b}");
    }
    let wx = world::weather::fetch(&client, loc.lat, loc.lon, now_ms, &cache);
    println!(
        "parsed: code {} ({}), temp {:.1} F (feels {:.1}), precip {:.2} mm, cloud {:.0} %, \
         wind {:.1} km/h dir {:?}, gust {:.1}, vis {:?} m, snow {:?} cm",
        wx.code,
        wx.word(),
        wx.temp,
        wx.feels,
        wx.precip,
        wx.cloud,
        wx.wind,
        wx.wind_dir,
        wx.gust,
        wx.visibility,
        wx.snow
    );
    let f = wx.flags();
    println!(
        "flags: rain {} snow {} fog {} thunder {} drizzle {}",
        f.rain, f.snow, f.fog, f.thunder, f.drizzle
    );
}
