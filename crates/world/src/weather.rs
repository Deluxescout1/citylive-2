//! Weather: port of old CityLive `fetchWeather` (city.js:1438) and its code
//! helpers `wfx`/`wxWord` (city.js:1483,1490).  Networking is behind
//! [`crate::http::HttpClient`]; parsing uses a cached copy of the last good
//! response and a seasonal [`Weather::fallback`] when offline.
//!
//! Units match the original request: `temperature_unit=fahrenheit`, wind in
//! km/h, precipitation in mm.

use serde::Deserialize;
use std::fs;
use std::path::Path;

use crate::http::HttpClient;
use crate::model::Date;

/// A parsed Weather snapshot.
#[derive(Debug, Clone, Copy)]
pub struct Weather {
    /// Air temperature, °F.
    pub temp: f32,
    /// "Feels like", °F.
    pub feels: f32,
    /// Current precipitation rate, mm (the 15-min nowcast override applies).
    pub precip: f32,
    /// Cloud cover, 0..100 %.
    pub cloud: f32,
    /// Wind speed, km/h.
    pub wind: f32,
    /// Gusts, km/h.
    pub gust: f32,
    /// Wind direction, 0..360 (0 = N), from the current block's
    /// `wind_direction_10m`.
    pub wind_dir: Option<f32>,
    /// Visibility, metres (from `visibility`).
    pub visibility: Option<f32>,
    /// Snow depth / accumulation reported by Open-Meteo, if present.
    pub snow: Option<f32>,
    /// WMO weather code (0..99).  Type (rain/snow/fog/thunder) derives from it.
    pub code: u16,
    /// ~12 h projection for the HUD forecast line.
    pub proj: Projection,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Projection {
    pub hi: Option<f32>,
    pub lo: Option<f32>,
    /// Peak precipitation probability, 0..100.
    pub pp: f32,
    /// Condition roughly 3 h out (a WMO code).
    pub code: Option<u16>,
}

impl Default for Weather {
    fn default() -> Self {
        // Plausible defaults: mild, a bit of cloud, no precip.
        Self {
            temp: 55.0,
            feels: 54.0,
            precip: 0.0,
            cloud: 30.0,
            wind: 6.0,
            gust: 10.0,
            wind_dir: None,
            visibility: None,
            snow: None,
            code: 2,
            proj: Projection::default(),
        }
    }
}

/// Port of `wfx` (city.js:1483): boolean precipitation-type flags from the
/// WMO code + cloud cover.
pub struct Flags {
    pub fog: bool,
    pub drizzle: bool,
    pub rain: bool,
    pub snow: bool,
    pub thunder: bool,
    pub cloudy: bool,
    /// WMO codes 30-35: a sandstorm or a dust storm (D66: the desert's idiom
    /// for what is rain everywhere else).
    pub dust: bool,
}

impl Weather {
    pub fn flags(&self) -> Flags {
        let c = self.code;
        Flags {
            fog: c == 45 || c == 48,
            drizzle: (51..=57).contains(&c),
            rain: (61..=67).contains(&c) || (80..=82).contains(&c),
            snow: (71..=77).contains(&c) || c == 85 || c == 86,
            thunder: c >= 95,
            cloudy: self.cloud > 60.0,
            dust: (30..=35).contains(&c),
        }
    }

    /// Port of `wxWord` (city.js:1490): a short weather word from the WMO code
    /// (+ cloud % for the clear/cloudy split).
    pub fn word(&self) -> &'static str {
        let c = self.code;
        if c >= 95 {
            "STORMS"
        } else if (71..=77).contains(&c) || c == 85 || c == 86 {
            "SNOW"
        } else if (61..=67).contains(&c) || (80..=82).contains(&c) {
            "RAIN"
        } else if (51..=57).contains(&c) {
            "DRIZZLE"
        } else if c == 45 || c == 48 {
            "FOG"
        } else if self.cloud >= 85.0 || c == 3 {
            "OVERCAST"
        } else if self.cloud >= 45.0 || c == 2 {
            "CLOUDY"
        } else if c == 1 {
            "P.CLOUDY"
        } else {
            "CLEAR"
        }
    }
}

/// The Open-Meteo request CityLive builds (city.js:1479), plus the three extra
/// `current` fields the `world` brief asks for (wind direction, visibility,
/// snow depth).  No API key is sent.
pub fn url_for(lat: f64, lon: f64) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=weather_code,cloud_cover,wind_speed_10m,wind_direction_10m,wind_gusts_10m,temperature_2m,apparent_temperature,precipitation,visibility,snowfall&minutely_15=precipitation,weather_code&hourly=weather_code,precipitation_probability,temperature_2m&forecast_days=2&temperature_unit=fahrenheit&timezone=auto"
    )
}

// ---- Open-Meteo response shapes ----

#[derive(Debug, Deserialize)]
struct OmResponse {
    /// Seconds the response's local timestamps are ahead of UTC (`timezone=auto`
    /// returns local times with no offset in the string, B93).
    utc_offset_seconds: Option<i64>,
    current: Option<OmCurrent>,
    minutely_15: Option<OmBlock>,
    hourly: Option<OmBlock>,
}

#[derive(Debug, Deserialize)]
struct OmCurrent {
    weather_code: Option<u16>,
    cloud_cover: Option<f32>,
    wind_speed_10m: Option<f32>,
    wind_direction_10m: Option<f32>,
    wind_gusts_10m: Option<f32>,
    temperature_2m: Option<f32>,
    apparent_temperature: Option<f32>,
    precipitation: Option<f32>,
    visibility: Option<f32>,
    snowfall: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct OmBlock {
    time: Vec<String>,
    weather_code: Option<Vec<Option<u16>>>,
    precipitation: Option<Vec<Option<f32>>>,
    precipitation_probability: Option<Vec<Option<f32>>>,
    temperature_2m: Option<Vec<Option<f32>>>,
}

/// Parse a recorded Open-Meteo body into a `Weather`, applying the same
/// nowcast override (minutely_15) and 12 h projection scan the JS did.
/// `now_ms` is the wall clock used for those look-ups.
pub fn parse(body: &str, now_ms: i64) -> Result<Weather, String> {
    let v: OmResponse =
        serde_json::from_str(body).map_err(|e| format!("open-meteo json: {e}"))?;

    let cur = v
        .current
        .as_ref()
        .ok_or_else(|| "open-meteo: missing current".to_string())?;
    // B93: `timezone=auto` makes every `time` local to the place, with no offset
    // in the string. Read as UTC they were `utc_offset` hours out, so the
    // nowcast bucket was the forecast several hours ahead (5 h in the test site) and
    // the 12 h projection window was shifted the same way.
    let off_ms = v.utc_offset_seconds.unwrap_or(0) * 1000;
    let mut w = Weather {
        temp: cur.temperature_2m.unwrap_or(55.0),
        feels: cur.apparent_temperature.unwrap_or(54.0),
        precip: cur.precipitation.unwrap_or(0.0),
        cloud: cur.cloud_cover.unwrap_or(30.0),
        wind: cur.wind_speed_10m.unwrap_or(6.0),
        gust: cur.wind_gusts_10m.unwrap_or(10.0),
        wind_dir: cur.wind_direction_10m,
        visibility: cur.visibility,
        snow: cur.snowfall,
        code: cur.weather_code.unwrap_or(0),
        proj: Projection::default(),
    };

    // NOWCAST: the 15-min bucket for "right now" (city.js:1450).
    if let Some(m15) = &v.minutely_15 {
        if let Some(t0ms) = m15.time.first().and_then(|t| parse_iso_in(t, off_ms)) {
            let idx = ((now_ms - t0ms) / 900_000) as usize;
            if now_ms >= t0ms && idx < m15.time.len() {
                if let Some(p) = m15.precipitation.as_ref().and_then(|a| a.get(idx).copied().flatten()) {
                    w.precip = p;
                }
                if let Some(c) = m15.weather_code.as_ref().and_then(|a| a.get(idx).copied().flatten()) {
                    w.code = c;
                }
            }
        }
    }

    // PROJECTED: scan the next ~12 h (city.js:1461).
    if let Some(hh) = &v.hourly {
        let mut hi: i32 = -900;
        let mut lo: i32 = 900;
        let mut pp = 0.0f32;
        let base_code = Some(w.code);
        let mut soon_code: Option<u16> = None;
        for (k, t) in hh.time.iter().enumerate() {
            let tms = match parse_iso_in(t, off_ms) {
                Some(x) => x,
                None => continue,
            };
            let dh = (tms - now_ms) as f64 / 3_600_000.0;
            if dh < -1.0 {
                continue;
            }
            if dh > 12.0 {
                break;
            }
            if let Some(tv) = hh.temperature_2m.as_ref().and_then(|a| a.get(k).copied().flatten()) {
                let iv = tv as i32;
                if iv > hi { hi = iv; }
                if iv < lo { lo = iv; }
            }
            if let Some(ppv) = hh
                .precipitation_probability
                .as_ref()
                .and_then(|a| a.get(k).copied().flatten())
            {
                if ppv > pp {
                    pp = ppv;
                }
            }
            if (2.5..3.5).contains(&dh) {
                soon_code = hh.weather_code.as_ref().and_then(|a| a.get(k).copied().flatten());
            }
        }
        let code = soon_code.or(base_code);
        w.proj = Projection {
            hi: if hi > -900 { Some(hi as f32) } else { None },
            lo: if lo < 900 { Some(lo as f32) } else { None },
            pp,
            code,
        };
    }

    Ok(w)
}

/// [`parse_iso`] for a response whose bare timestamps are local time `off_ms`
/// ahead of UTC (B93). A string with its own offset or a `Z` is already exact.
fn parse_iso_in(s: &str, off_ms: i64) -> Option<i64> {
    let ms = parse_iso(s)?;
    let b = s.as_bytes();
    let explicit = s.ends_with('Z') || (b.len() >= 21 && (b[19] == b'+' || b[19] == b'-'));
    Some(if explicit { ms } else { ms - off_ms })
}

/// `Date.parse` for the ISO-8601 strings Open-Meteo returns.  Handles both the
/// local `YYYY-MM-DDTHH:MM` (→ treated as UTC) and the `+HH:MM` offset the
/// JS's `Date.parse` interpreted.  We interpret an explicit offset correctly;
/// a bare timestamp is read as UTC.  Returns Unix ms.
fn parse_iso(s: &str) -> Option<i64> {
    // We only need to compare relative ordering / buckets, so a consistent
    // interpretation is what matters.  Support: YYYY-MM-DDTHH:MM[:SS][+HH:MM]
    let bytes = s.as_bytes();
    if bytes.len() < 16 {
        return None;
    }
    fn num(slice: &[u8]) -> Option<i64> {
        let t = std::str::from_utf8(slice).ok()?;
        t.parse().ok()
    }
    let y = num(&bytes[0..4])?;
    let mo = num(&bytes[5..7])?;
    let d = num(&bytes[8..10])?;
    let h = num(&bytes[11..13])?;
    let mi = num(&bytes[14..16])?;
    // days from civil → unix seconds (UTC), then add h/mi.
    let secs = crate::model::days_from_civil(y as i32, mo as i32, d as i32) * 86_400
        + h * 3600
        + mi * 60;
    let mut ms = secs * 1000;
    // Optional offset +HH:MM / -HH:MM
    if bytes.len() >= 21 && (bytes[19] == b'+' || bytes[19] == b'-') {
        let sign: i64 = if bytes[19] == b'-' { -1 } else { 1 };
        let oh = num(&bytes[20..22])?;
        let omi = if bytes.len() >= 25 {
            num(&bytes[23..25])?
        } else {
            0
        };
        ms -= sign * (oh * 3600 + omi * 60) * 1000;
    }
    Some(ms)
}

/// Fetch current weather.  Tries the network, then the cache file, then a
/// seasonal fallback.  On a successful fetch the raw body is written to
/// `cache_path` (parents created as needed).
pub fn fetch(
    client: &dyn HttpClient,
    lat: f64,
    lon: f64,
    now_ms: i64,
    cache_path: &Path,
) -> Weather {
    let url = url_for(lat, lon);
    match client.get(&url) {
        Ok(body) => {
            if let Some(w) = parse(&body, now_ms).ok() {
                write_cache(cache_path, &body);
                return w;
            }
            // malformed → fall through to cache
        }
        Err(_) => {}
    }
    if let Some(body) = read_cache(cache_path) {
        if let Ok(w) = parse(&body, now_ms) {
            return w;
        }
    }
    let d = model_local_date(lat, now_ms);
    Weather::fallback(&d, lat)
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

/// Rough local calendar stamp from a UTC instant for the location (used only
/// to pick a season for the fallback).  Good enough for a fallback.
fn model_local_date(lat: f64, now_ms: i64) -> Date {
    let _ = lat;
    let secs = (now_ms / 1000) as i64;
    let days = secs.div_euclid(86_400) + 10_957; // ~1970-01-01
    let rem = secs.rem_euclid(86_400);
    let hour = (rem / 3600) as u8;
    let (y, m, d) = crate::model::civil_from_days(days);
    Date::new(y, m as u8, d as u8, hour)
}

impl Weather {
    /// Plausible seasonal defaults when offline and no cached response exists
    /// (city.js used to keep its last values; we synthesise by month + hemisphere).
    /// `lat` is used only to flip the seasons for the southern hemisphere.
    pub fn fallback(date: &Date, lat: f64) -> Weather {
        // Northern-hemisphere seasonal mean temperatures (°F) at the equator-ish;
        // the southern hemisphere shifts by 6 months.
        let months = [
            [33.0, 58.0, 15.0],   // winter:   temp, feels, cloud
            [58.0, 56.0, 45.0],   // spring
            [76.0, 74.0, 30.0],   // summer
            [55.0, 52.0, 55.0],   // autumn
        ];
        let mut idx = if date.month <= 2 || date.month == 12 {
            0
        } else if date.month <= 5 {
            1
        } else if date.month <= 8 {
            2
        } else {
            3
        };
        if lat < 0.0 {
            idx = (idx + 2) % 4;
        }
        let [temp, feels, cloud] = months[idx];
        let code = if date.month <= 2 || date.month == 12 { 3 } else { 2 };
        Weather {
            temp,
            feels,
            precip: 0.0,
            cloud,
            wind: 8.0,
            gust: 14.0,
            wind_dir: None,
            visibility: None,
            snow: None,
            code: code as u16,
            proj: Projection::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    /// A short or empty `minutely_15` block from the network must not panic.
    /// B93: with `timezone=auto` the times are local and `utc_offset_seconds`
    /// says how far from UTC. At 16:15 in the test site (EST, UTC-5) the nowcast is
    /// the 16:15 bucket (65), not the 21:15 one (85) that reading the local
    /// times as UTC picked.
    #[test]
    fn local_times_use_the_response_utc_offset() {
        let n = 192usize; // two days of 15-minute buckets
        let times: Vec<String> = (0..n)
            .map(|i| format!("2026-11-13T{:02}:{:02}", (i / 4) % 24, (i % 4) * 15))
            .collect();
        // Two days: the second half would need the 14th; keep it simple and
        // only use the first 96 buckets, which is the day that matters.
        let times: Vec<String> = times[..96].to_vec();
        let precip: Vec<String> = (0..96).map(|i| format!("{}.0", i)).collect();
        let body = format!(
            r#"{{"utc_offset_seconds":-18000,"current":{{"temperature_2m":50.0,"weather_code":1}},"minutely_15":{{"time":{},"precipitation":[{}]}}}}"#,
            serde_json::to_string(&times).unwrap(),
            precip.join(",")
        );
        // 16:15 local = 21:15 UTC.
        let day = crate::model::days_from_civil(2026, 11, 13) * 86_400_000;
        let now = day + (21 * 3600 + 15 * 60) * 1000;
        let w = parse(&body, now).unwrap();
        assert!(
            (w.precip - 65.0).abs() < 0.001,
            "the nowcast should be the 16:15 local bucket (65), got {}",
            w.precip
        );
    }

    #[test]
    fn short_minutely_blocks_do_not_panic() {
        let cur = r#""current":{"temperature_2m":50.0,"weather_code":1}"#;
        for m15 in [
            r#"{"time":[]}"#,
            r#"{"time":["2026-07-04T12:00","2026-07-04T12:15"],"precipitation":[]}"#,
            r#"{"time":["2026-07-04T12:00","2026-07-04T12:15"],"weather_code":[1]}"#,
        ] {
            let body = format!(r#"{{{cur},"minutely_15":{m15}}}"#);
            // 12:20 UTC: bucket 1 of 2.
            let now = 1_783_167_600_000 + 20 * 60_000;
            assert!(parse(&body, now).is_ok(), "{m15}");
        }
    }

    use super::*;
    use crate::http::tests::MapClient;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join("citylive-world-test")
            .join(format!("{}-{}-{}", tag, std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name),
        )
        .unwrap()
    }

    /// The "now" instants aligned with the first minutely_15 bucket of each
    /// fixture (so the nowcast override hits bucket 0, as in the JS).
    const NOW_CLEAR: i64 = 1_768_435_200_000; // 2026-01-15T00:00Z
    const NOW_RAIN: i64 = 1_779_235_200_000; // 2026-05-20T00:00Z
    const NOW_SNOW: i64 = 1_767_312_000_000; // 2026-01-02T00:00Z

    #[test]
    fn parse_clear() {
        let w = parse(&fixture("open-meteo-clear.json"), NOW_CLEAR).unwrap();
        assert_eq!(w.code, 0);
        assert!((w.temp - 64.0).abs() < 0.01);
        assert!((w.cloud - 3.0).abs() < 0.01);
        assert_eq!(w.word(), "CLEAR");
        assert!(w.visibility.is_some());
        assert!(!w.flags().fog && !w.flags().rain && !w.flags().snow && !w.flags().thunder);
        assert_eq!(w.proj.hi, Some(74.0));
        assert_eq!(w.proj.lo, Some(58.0));
        assert!((w.proj.pp - 0.0).abs() < 0.001);
    }

    #[test]
    fn parse_rain_with_nowcast_override() {
        let w = parse(&fixture("open-meteo-rain.json"), NOW_RAIN).unwrap();
        // current code 61 is overridden by the 15-min nowcast to code 63.
        assert_eq!(w.code, 63, "nowcast override expected");
        assert!((w.precip - 1.4).abs() < 0.001, "nowcast precip override");
        assert_eq!(w.word(), "RAIN");
        assert!(w.flags().rain);
        assert!((w.wind - 16.0).abs() < 0.01);
        assert_eq!(w.wind_dir, Some(200.0));
        assert_eq!(w.proj.hi, Some(56.0));
        assert_eq!(w.proj.lo, Some(50.0));
        assert!((w.proj.pp - 85.0).abs() < 0.001);
        assert_eq!(w.proj.code, Some(65));
    }

    #[test]
    fn parse_snow() {
        let w = parse(&fixture("open-meteo-snow.json"), NOW_SNOW).unwrap();
        assert_eq!(w.code, 75);
        assert!((w.precip - 0.6).abs() < 0.001);
        assert_eq!(w.word(), "SNOW");
        assert!(w.flags().snow);
        assert_eq!(w.snow, Some(1.8));
        assert!(w.visibility.unwrap() < 5000.0, "low visibility in snow");
        assert_eq!(w.proj.hi, Some(33.0));
        assert_eq!(w.proj.lo, Some(26.0));
        assert!((w.proj.pp - 95.0).abs() < 0.001);
    }

    #[test]
    fn fetch_success_writes_cache() {
        let d = tmpdir("wx-ok");
        let cache = d.join("wx.json");
        let body = fixture("open-meteo-clear.json");
        let url = url_for(41.0, -75.0);
        let client = MapClient::new().ok(&url, &body);
        let w = fetch(&client, 41.0, -75.0, NOW_CLEAR, &cache);
        assert_eq!(w.code, 0);
        assert!((w.temp - 64.0).abs() < 0.01);
        // cache was written with the raw body
        let cached = std::fs::read_to_string(&cache).unwrap();
        assert_eq!(cached, body);
        let _ = d;
    }

    #[test]
    fn fetch_offline_uses_cache() {
        let d = tmpdir("wx-cache");
        let cache = d.join("wx.json");
        let body = fixture("open-meteo-rain.json");
        std::fs::write(&cache, &body).unwrap();
        let client = MapClient::new(); // nothing served → all network calls Err
        let w = fetch(&client, 41.0, -72.0, NOW_RAIN, &cache);
        assert_eq!(w.code, 63, "should serve the cached rain sample: {w:?}");
        assert!((w.precip - 1.4).abs() < 0.001);
        let _ = d;
    }

    #[test]
    fn fetch_offline_no_cache_falls_back() {
        let d = tmpdir("wx-fallback");
        let cache = d.join("wx.json"); // absent
        let client = MapClient::new();
        let w = fetch(&client, 41.0, -72.0, NOW_CLEAR, &cache);
        // No network + no cache → seasonal fallback: no visibility, no precip.
        assert!(w.visibility.is_none(), "fallback has no visibility: {w:?}");
        assert_eq!(w.precip, 0.0);
        assert!(matches!(w.code, 0 | 2 | 3), "fallback code is clear-ish: {w:?}");
        let _ = d;
    }
}
