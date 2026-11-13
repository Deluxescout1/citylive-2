//! CityLive Iso sky layer (M5, brief B12).
//!
//! Pure, cheap functions that turn (unix ms, lat, lon) into how the sky and the
//! scene light should look, for the renderer to use later:
//!
//! * [`state`] — sun/moon alt·az (from the `world` crate) + a day-phase class;
//! * [`palette`] — zenith/horizon colours, ambient tint, light strength,
//!   window glow (`window_glow`, D25) and star alpha, smoothly blended across
//!   twilight (a *bright clear day*, a *warm sunset*, a *moody blue night* —
//!   D4), with no hard jumps between consecutive minutes;
//! * [`draw`] — vertical gradient + deterministic star field + sun/moon discs
//!   into a [`render::Framebuffer`].
//!
//! No state, no allocation on the hot path, deterministic in `(seed)`.
//! The renderer owns `Framebuffer`s; this crate only fills one given.

pub mod backdrop;
pub mod live;
pub mod clouds;
pub mod draw;
pub mod palette;

// Re-exported so call sites read `sky::state`, `sky::palette`, `sky::draw`,
// `sky::Palette` without reaching into submodules.
pub use draw::draw;

/// Epoch milliseconds for "today at `HH:MM`" or a fixed `date`, interpreted as
/// **local** time at `lon` (round(lon/15) hours from UTC; DST ignored).
pub fn clock_ms(time: Option<&str>, date: Option<&str>, lon: f64) -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    clock_ms_at(now, time, date, lon)
}

/// [`clock_ms`] for a given "now" (epoch ms), so the day rules can be tested.
pub fn clock_ms_at(now: i64, time: Option<&str>, date: Option<&str>, lon: f64) -> i64 {
    let mut secs = now / 1000;
    if let Some(d) = date {
        let parts: Vec<i64> = d.split('-').filter_map(|p| p.parse().ok()).collect();
        if parts.len() == 3 {
            let days = days_from_civil(parts[0], parts[1] as u32, parts[2] as u32);
            secs = days * 86400 + secs.rem_euclid(86400);
        }
    }
    if let Some(t) = time {
        let parts: Vec<i64> = t.split(':').filter_map(|p| p.parse().ok()).collect();
        if parts.len() >= 2 {
            let tz = (lon / 15.0).round() as i64;
            // The day is the *local* day: "today at 12:00" at the test site at 01:00
            // UTC is yesterday's date there (B93: it used the UTC date).
            let day = (secs + tz * 3600).div_euclid(86400);
            secs = day * 86400 + parts[0] * 3600 + parts[1] * 60 - tz * 3600;
        }
    }
    secs * 1000
}

/// The *local solar hour* (0..24) at a clock instant, for a longitude: the
/// renderer needs the hour itself, not just how dark it is (B58b item 7 — who
/// has their lights on is a question about the clock).
pub fn local_hour(ms: i64, lon: f64) -> f32 {
    let local = ms as f64 / 1000.0 + lon / 15.0 * 3600.0;
    (local.rem_euclid(86400.0) / 3600.0) as f32
}

/// Howard Hinnant's days-from-civil (no chrono dependency).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
pub use palette::palette;
pub use palette::Palette;
pub use render::palette::Color;

use world::{moon, sun};

/// Coarse classification of the sky for the hour: what a person calls it.
/// Derived purely from the sun's altitude and the sign of its vertical
/// velocity, so it always agrees with the smooth palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Sun ≤ −18° and no twilight: full night.
    Night,
    /// Sun climbing from below the horizon up through twilight.
    Dawn,
    /// Sun up; `golden = true` only in the brief low-sun band.
    Day,
    /// Sun descending through twilight.
    Dusk,
}

impl Phase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Night => "night",
            Self::Dawn => "dawn",
            Self::Day => "day",
            Self::Dusk => "dusk",
        }
    }
}

/// A snapshot of the sky at one instant. All angles are degrees (altitude
/// from the horizon, azimuth from true north, east-positive).
#[derive(Debug, Clone, Copy)]
pub struct SkyState {
    pub ms: i64,
    pub lat: f64,
    pub lon: f64,
    /// Sun altitude (deg).
    pub sun_alt: f64,
    /// Sun azimuth (deg from north, east-positive).
    pub sun_az: f64,
    /// Moon altitude (deg).
    pub moon_alt: f64,
    /// Moon azimuth (deg from north, east-positive).
    pub moon_az: f64,
    /// Moon phase fraction 0 = new … 0.5 = full … 1 (matches `moon::phase_fraction`).
    pub moon_phase: f64,
    /// Illuminated fraction of the moon, 0..=1 (matches the JS `illum`).
    pub moon_illum: f64,
    /// Vertical sun velocity over ±1 min (deg), ≥ 0 when the sun is climbing.
    pub sun_rate: f64,
    /// Coarse phase class (see [`Phase`]).
    pub phase: Phase,
    /// Cloud cover 0..1 from the weather (B27): more puffs, a greyer band.
    pub cloud: f32,
}

/// Classify the phase from an altitude and whether the sun is climbing.
/// Pure (no I/O, no astronomy) so it is trivially unit-testable.
fn classify(alt: f64, climbing: bool) -> Phase {
    if alt > -18.0 {
        // In or above civil twilight.
        if alt > 0.0 {
            Phase::Day
        } else if climbing {
            Phase::Dawn
        } else {
            Phase::Dusk
        }
    } else {
        Phase::Night
    }
}

/// Compute the sky state at `ms` for the given location (lat/lon deg).
/// Cheap (a handful of trigs on top of `world::sun` / `world::moon`);
/// measured `state` + `palette` together in the H12 measurements.
pub fn state(ms: i64, lat: f64, lon: f64) -> SkyState {
    let (sun_alt, sun_az) = sun::position(ms, lat, lon);
    // Vertical velocity over a 2-min window, deg/min.
    let (up, _) = sun::position(ms + 60_000, lat, lon);
    let (dn, _) = sun::position(ms - 60_000, lat, lon);
    let sun_rate = (up - dn) / 2.0;
    let (moon_alt, moon_az) = moon::position(ms, lat, lon);
    let ph = moon::Phase::at(ms);

    let phase = classify(sun_alt, sun_rate >= 0.0);

    SkyState {
        ms,
        lat,
        lon,
        sun_alt,
        sun_az,
        moon_alt,
        moon_az,
        moon_phase: ph.fraction,
        moon_illum: ph.illumination,
        sun_rate,
        phase,
        // A fair-weather default: the wallpaper replaces it with the real cover.
        cloud: 0.15,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE_LAT: f64 = 41.0;
    const SITE_LON: f64 = -75.0;
    /// 21 June 2026, 00:00 local (test site, EDT = UTC-4)
    const JUN21: i64 = 1_782_014_400_000;
    /// 21 December 2026, 00:00 local (test site, EST = UTC-5)
    const DEC21: i64 = 1_797_829_200_000;
    const HOUR: i64 = 3_600_000;
    const MIN: i64 = 60_000;

    #[test]
    fn a_clock_time_without_a_date_is_on_the_local_day() {
        // 2026-11-13 01:00 UTC is still 12 November (20:00) in the test site.
        let lon = SITE_LON;
        let now = days_from_civil(2026, 11, 13) * 86_400_000 + HOUR;
        let ms = clock_ms_at(now, Some("12:00"), None, lon);
        // Local noon on 12 November, EST (lon -72 rounds to UTC-5): 17:00 UTC.
        let want = days_from_civil(2026, 11, 12) * 86_400_000 + 17 * HOUR;
        assert_eq!(ms, want, "off by {} h", (ms - want) as f64 / HOUR as f64);
        // And late in the local evening of the 12th, 12:00 is still the 12th.
        let now = days_from_civil(2026, 11, 13) * 86_400_000 + 4 * HOUR;
        assert_eq!(clock_ms_at(now, Some("12:00"), None, lon), want);
        // From 05:00 UTC the local date is the 13th.
        let now = days_from_civil(2026, 11, 13) * 86_400_000 + 6 * HOUR;
        assert_eq!(clock_ms_at(now, Some("12:00"), None, lon), want + 24 * HOUR);
    }

    #[test]
    fn determinism_same_inputs_same_state() {
        let a = state(JUN21 + 12 * HOUR, SITE_LAT, SITE_LON);
        let b = state(JUN21 + 12 * HOUR, SITE_LAT, SITE_LON);
        assert_eq!((a.sun_alt, a.sun_az), (b.sun_alt, b.sun_az));
        assert_eq!((a.moon_alt, a.moon_az), (b.moon_alt, b.moon_az));
        assert_eq!(a.moon_phase, b.moon_phase);
        assert_eq!(a.phase, b.phase);
    }

    #[test]
    fn noon_in_june_is_day_midnight_is_night() {
        let noon = state(JUN21 + 12 * HOUR, SITE_LAT, SITE_LON);
        assert_eq!(noon.phase, Phase::Day, "sun_alt={}", noon.sun_alt);
        assert!(noon.sun_alt > 30.0, "sun_alt={}", noon.sun_alt);

        let midnight = state(JUN21, SITE_LAT, SITE_LON);
        assert_eq!(midnight.phase, Phase::Night, "sun_alt={}", midnight.sun_alt);
        assert!(
            midnight.sun_alt < -18.0,
            "sun_alt={}",
            midnight.sun_alt
        );
    }

    #[test]
    fn midnight_in_december_is_night() {
        let midnight = state(DEC21, SITE_LAT, SITE_LON);
        assert_eq!(midnight.phase, Phase::Night);
    }

    #[test]
    fn polar_day_78n_june_never_reaches_night() {
        // 78.2°N, 21 June. The sun never sets; we must never classify Night.
        let lat = 78.223;
        let lon = -15.626;
        for minute in 0..(24 * 60) {
            let s = state(JUN21 + minute * MIN, lat, lon);
            assert_ne!(
                s.phase,
                Phase::Night,
                "minute={minute} sun_alt={}",
                s.sun_alt
            );
        }
    }

    #[test]
    fn polar_night_78n_december_never_reaches_day() {
        let lat = 78.223;
        let lon = -15.626;
        for minute in 0..(24 * 60) {
            let s = state(DEC21 + minute * MIN, lat, lon);
            assert_ne!(
                s.phase,
                Phase::Day,
                "minute={minute} sun_alt={}",
                s.sun_alt
            );
        }
    }

    #[test]
    fn sunrise_is_dawn_sunset_is_dusk() {
        // 21 June at the test site: find the first Dawn and Dusk minutes of the day.
        let mut dawn: Option<u32> = None;
        let mut dusk: Option<u32> = None;
        for minute in 0..(24 * 60) {
            let s = state(JUN21 + minute as i64 * MIN, SITE_LAT, SITE_LON);
            if s.phase == Phase::Dawn && dawn.is_none() {
                dawn = Some(minute);
            }
            if s.phase == Phase::Dusk && dusk.is_none() {
                dusk = Some(minute);
            }
        }
        assert!(dawn.is_some(), "no dawn minute found");
        assert!(dusk.is_some(), "no dusk minute found");
        assert!(
            dawn.unwrap() < dusk.unwrap(),
            "dawn={:?} dusk={:?}",
            dawn,
            dusk
        );
        assert!(dawn.unwrap() < 6 * 60, "dawn={:?}", dawn);
        assert!(dusk.unwrap() > 18 * 60, "dusk={:?}", dusk);
    }
}
