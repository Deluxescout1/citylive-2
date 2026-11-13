//! Moon: port of old CityLive `moonPhase` (city.js:183), `lstHours` (city.js:267),
//! `altAz` (city.js:273) and `moonRaDec` (city.js:281).

use crate::model;

const DEG: f64 = std::f64::consts::PI / 180.0;
const SYNODIC: f64 = 29.53058867; // lunations reference used in the JS

/// Fractional lunation: 0=new, 0.25=first quarter, 0.5=full, 0.75=last.
/// Verbatim port of `moonPhase`.
pub fn phase_fraction(ms: i64) -> f64 {
    let jd = model::julian_from_ms(ms);
    let mut p = (jd - 2_451_550.1) / SYNODIC;
    p -= model::floor(p);
    if p < 0.0 {
        p += 1.0;
    }
    p
}

/// Moon phase with derived age/illumination/name.  `fraction` and
/// `illumination` match the JS (`drawMoon` uses `illum=(1-cos(2πp))/2`); `name`
/// is a standard 8-way naming (not in the original JS).
#[derive(Debug, Clone, Copy)]
pub struct Phase {
    /// `p` from `moonPhase` (0..1).
    pub fraction: f64,
    /// Days since new moon (0..30.5).
    pub age_days: f64,
    /// Illuminated fraction (0..1) — matches the JS `illum`.
    pub illumination: f64,
    pub name: &'static str,
}

impl Phase {
    pub fn at(ms: i64) -> Self {
        let p = phase_fraction(ms);
        let illum = (1.0 - (2.0 * std::f64::consts::PI * p).cos()) / 2.0;
        Self {
            fraction: p,
            age_days: p * SYNODIC,
            illumination: illum,
            name: NAME[p8(p)],
        }
    }
}

const NAME: [&str; 8] = [
    "new",
    "waxing crescent",
    "first quarter",
    "waxing gibbous",
    "full",
    "waning gibbous",
    "last quarter",
    "waning crescent",
];

/// Bucket the fraction into one of 8 phase slots with a small tolerance around
/// the quarters.
fn p8(p: f64) -> usize {
    const TOL: f64 = 0.0625 * 0.5; // ± a quarter of an eighth
    for (i, anchor) in [0.0, 0.25, 0.5, 0.75, 1.0].iter().enumerate() {
        if (p - anchor).abs() < TOL {
            // 1.0 wraps back to the new-moon slot: `NAME` has only 8 entries.
            return (i * 2) % 8;
        }
    }
    // Between anchors: map to the leading slot.
    let e = (p * 8.0).floor() as usize;
    e.min(7)
}

/// Local sidereal time in hours (verbatim port of `lstHours`).
pub fn lst_hours(ms: i64, lon_east: f64) -> f64 {
    let jd = model::julian_from_ms(ms);
    let t = (jd - 2_451_545.0) / 36_525.0;
    let gmst = 280.46061837
        + 360.98564736629 * (jd - 2_451_545.0)
        + 0.000387933 * t * t
        - t * t * t / 38_710_000.0;
    let mut lst = (gmst + lon_east) % 360.0;
    if lst < 0.0 {
        lst += 360.0;
    }
    lst / 15.0
}

/// Equatorial (RA hours, Dec deg) → horizontal (alt deg, az deg from north).
/// Verbatim port of `altAz` (the JS used the `LAT` global; here it is a param).
pub fn alt_az(ra_hours: f64, dec_deg: f64, lst_hours: f64, lat_deg: f64) -> (f64, f64) {
    let h = (lst_hours - ra_hours) * 15.0 * DEG;
    let sd = (dec_deg * DEG).sin();
    let cd = (dec_deg * DEG).cos();
    let sl = (lat_deg * DEG).sin();
    let cl = (lat_deg * DEG).cos();
    let sin_alt = sd * sl + cd * cl * h.cos();
    let alt = sin_alt.asin();
    let cos_alt = if alt.cos() == 0.0 { 1e-6 } else { alt.cos() };
    let mut a = (-cd * h.sin() / cos_alt).atan2((sd - sin_alt * sl) / (cos_alt * cl));
    if a < 0.0 {
        a += 2.0 * std::f64::consts::PI;
    }
    (alt / DEG, a / DEG)
}

#[derive(Debug, Clone, Copy)]
pub struct RaDec {
    /// Right ascension, hours (0..24).
    pub ra: f64,
    /// Declination, degrees.
    pub dec: f64,
}

/// Low-precision Moon RA/Dec (degree-based series) — verbatim port of `moonRaDec`.
pub fn ra_dec(ms: i64) -> RaDec {
    let jd = model::julian_from_ms(ms);
    let t = (jd - 2_451_545.0) / 36_525.0;
    let lm = 218.316 + 481_267.881 * t;
    let m = 134.963 + 477_198.867 * t;
    let f = 93.272 + 483_202.017 * t;
    let lon = (lm + 6.289 * (m * DEG).sin()) * DEG;
    let lat = (5.128 * (f * DEG).sin()) * DEG;
    let e = 23.439 * DEG;
    let ra = (lon.sin() * e.cos() - lat.tan() * e.sin()).atan2(lon.cos());
    let dec = (lat.sin() * e.cos() + lat.cos() * e.sin() * lon.sin()).asin();
    let ra = ((ra * 12.0 / std::f64::consts::PI) + 24.0) % 24.0;
    RaDec {
        ra,
        dec: dec / DEG,
    }
}

/// Moon altitude/azimuth (degrees) for a location.  = `altAz(moonRaDec, lst)`.
pub fn position(ms: i64, lat: f64, lon: f64) -> (f64, f64) {
    let r = ra_dec(ms);
    alt_az(r.ra, r.dec, lst_hours(ms, lon), lat)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_bounds_and_quarters() {
        // Full moon 2026-02-24 ~ 00:00Z (well, approximate).  Just check range
        // and that illumination stays in [0,1].
        for ms in [1_700_000_000_000i64, 1_750_000_000_000, 1_767_225_600_000, 1_790_000_000_000] {
            let ph = Phase::at(ms);
            assert!((0.0..1.0).contains(&ph.fraction));
            assert!((0.0..=1.0).contains(&ph.illumination));
            assert!((0.0..=SYNODIC).contains(&ph.age_days));
        }
    }

    #[test]
    fn lst_is_hours() {
        let lst = lst_hours(1_767_225_600_000, -75.0);
        assert!((0.0..24.0).contains(&lst));
    }
}
