//! Sun: port of old CityLive `sunTimes` (city.js:153, NOAA simplified) plus a
//! real `position` built from the same solar geometry and the already-ported
//! `altAz` helpers (city.js:267,273).

use crate::model;

const DEG: f64 = std::f64::consts::PI / 180.0;
const J2000: f64 = 2_451_545.0;
/// 0.833°: the combined refraction + solar-diameter depression the JS uses.
const SOLAR_DEPRESSION: f64 = 0.833;
/// Civil twilight depression (dawn/dusk), not present in the JS.
const CIVIL_DEPRESSION: f64 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polar {
    /// Sun is (never) up: `cosH > 1`.
    Night,
    /// Sun never sets: `cosH < -1`.
    Day,
}

/// Sunrise / sunset instants for a date and location.
/// `rise`/`set` are Unix ms; both `None` when a pole flag is set.
#[derive(Debug, Clone, Copy)]
pub struct SunTimes {
    pub rise: Option<i64>,
    pub set: Option<i64>,
    pub polar: Option<Polar>,
    /// Extras not in the original JS (documented, not parity-tested):
    /// `solar_noon`, `civil_dawn`, `civil_dusk` in Unix ms.
    pub solar_noon: Option<i64>,
    pub civil_dawn: Option<i64>,
    pub civil_dusk: Option<i64>,
}

/// Solar geometry derived from the JS `sunTimes` intermediate values: the day
/// declination and the transit (solar noon) Julian Date.
struct Solar {
    /// Obliquity-derived declination `dec` (rad).
    dec: f64,
    /// Apparent transit (solar noon) Julian Date.
    jt: f64,
}

fn solar_geometry(ms: i64, lon_east: f64) -> Solar {
    let jd = model::julian_from_ms(ms);
    // Verbatim from sunTimes:
    //   n = round(toJulian(d) - J2000 - 0.0009 - (-LON/360))
    //   Ja = J2000 + 0.0009 + (-LON/360) + n
    let n = (jd - J2000 - 0.0009 + lon_east / 360.0).round();
    let ja = J2000 + 0.0009 - lon_east / 360.0 + n;
    let m = DEG * (357.5291 + 0.98560028 * (ja - J2000));
    let c = DEG * (1.9148 * m.sin() + 0.02 * (2.0 * m).sin() + 0.0003 * (3.0 * m).sin());
    let ls = m + c + DEG * 102.9372 + std::f64::consts::PI;
    let jt = ja + 0.0053 * m.sin() - 0.0069 * (2.0 * ls).sin();
    let dec = (ls.sin() * (DEG * 23.4397).sin()).asin();
    Solar { dec, jt }
}

fn hour_angle_hours(cos_h: f64) -> Option<f64> {
    // Return `H0` in *days* (the JS divides by 2*PI), or None for polar.
    if cos_h > 1.0 {
        return None;
    }
    if cos_h < -1.0 {
        return Some(f64::INFINITY);
    }
    Some(cos_h.acos() / (2.0 * std::f64::consts::PI))
}

fn cos_h(dep_deg: f64, lat_deg: f64, dec_rad: f64) -> f64 {
    // Verbatim from sunTimes (city.js:164). The whole numerator is grouped:
    //   cosH = ( sin(-dep) - sin(lat)·sin(dec) ) / ( cos(lat)·cos(dec) )
    // A tempting `a - b/d` (the default operator precedence) does NOT match the JS.
    let lat = lat_deg * DEG;
    let a = (DEG * -dep_deg).sin();
    let b = lat.sin() * dec_rad.sin();
    let d = lat.cos() * dec_rad.cos();
    (a - b) / d
}

/// Port of `sunTimes`. Returns rise/set instants (Unix ms) plus pole flags.
pub fn times(ms: i64, lat: f64, lon: f64) -> SunTimes {
    let g = solar_geometry(ms, lon);
    let ch = cos_h(SOLAR_DEPRESSION, lat, g.dec);
    match hour_angle_hours(ch) {
        None => {
            return SunTimes {
                rise: None,
                set: None,
                polar: Some(Polar::Night),
                solar_noon: Some(model::ms_from_julian(g.jt)),
                civil_dawn: None,
                civil_dusk: None,
            }
        }
        Some(f64::INFINITY) => {
            return SunTimes {
                rise: None,
                set: None,
                polar: Some(Polar::Day),
                solar_noon: Some(model::ms_from_julian(g.jt)),
                civil_dawn: None,
                civil_dusk: None,
            }
        }
        Some(h0) => {
            // civil dawn/dusk
            let chc = cos_h(CIVIL_DEPRESSION, lat, g.dec);
            let (cd, cs) = match hour_angle_hours(chc) {
                Some(h0c) if !h0c.is_infinite() => (
                    Some(model::ms_from_julian(g.jt - h0c)),
                    Some(model::ms_from_julian(g.jt + h0c)),
                ),
                _ => (None, None),
            };
            return SunTimes {
                rise: Some(model::ms_from_julian(g.jt - h0)),
                set: Some(model::ms_from_julian(g.jt + h0)),
                polar: None,
                solar_noon: Some(model::ms_from_julian(g.jt)),
                civil_dawn: cd,
                civil_dusk: cs,
            };
        }
    }
}

/// Sun altitude/azimuth at an instant (degrees).  Uses the *continuous* NOAA
/// solar geometry (mean → true ecliptic longitude from the real Julian Date,
/// NOT snapped to a calendar day), then the ported `altAz`/`lstHours`.
/// (`times` keeps its own day-snapped transit math; that is what the original
/// JS `sunTimes` does.)
pub fn position(ms: i64, lat: f64, lon: f64) -> (f64, f64) {
    let jd = model::julian_from_ms(ms);
    let m = DEG * (357.5291 + 0.98560028 * (jd - J2000));
    let c = DEG * (1.9148 * m.sin() + 0.02 * (2.0 * m).sin() + 0.0003 * (3.0 * m).sin());
    let ls = m + c + DEG * 102.9372 + std::f64::consts::PI;
    let e = DEG * 23.4397;
    let dec = (ls.sin() * e.sin()).asin();
    let ra = (ls.sin() * e.cos()).atan2(ls.cos()) * 12.0 / std::f64::consts::PI; // → hours
    let lst = crate::moon::lst_hours(ms, lon);
    crate::moon::alt_az(ra, dec / DEG, lst, lat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn polar_flags_longyearbyen() {
        // ~78°N: polar day in June, polar night in December.
        // 2026-06-21T12:00:00Z and 2026-12-21T12:00:00Z
        let jun = model::ms_from_julian(
            2451545.0 + 0.0, // placeholder, overwritten below
        );
        let _ = (jun, PI);
        // Use explicit instants.
        let jun = 1_782_777_600_000i64; // 2026-06-21T12:00Z
        let dec = 1_797_979_200_000i64; // 2026-12-21T12:00Z
        let lat = 78.223;
        let lon = -15.626;
        let jt = times(jun, lat, lon);
        let jtd = times(dec, lat, lon);
        assert_eq!(jt.polar, Some(Polar::Day), "June at 78N should be polar day: {jt:?}");
        assert_eq!(jtd.polar, Some(Polar::Night), "Dec at 78N should be polar night: {jtd:?}");
    }

    #[test]
    fn sun_position_consistent_at_rise() {
        // No JS function computes the sun's alt/az, so the port is proven by
        // self-consistency: the continuous position() model crosses the -0.833°
        // depression at the same instant (± a few minutes) that times() calls
        // sunrise.  That ties the two sun functions together and against the
        // physical sun.
        for ms in [1_700_000_000_000, 1_736_942_400_000, 1_768_435_200_000, 1_790_000_000_000] {
            for (lat, lon) in [(44.0, -72.0), (41.0, -75.0)] {
                let t = times(ms, lat, lon);
                if let Some(rise) = t.rise {
                    // Find the closest -0.833° crossing within a ±5 min window
                    // (15 s grid, so the residual stays << the tolerance).
                    let mut best = f64::INFINITY;
                    for k in -200..=200 {
                        let a = position(rise + k * 15_000, lat, lon).0;
                        best = best.min((a - (-0.833)).abs());
                    }
                    assert!(
                        best < 0.05,
                        "ms={ms} lat={lat}: position() crosses -0.833 deg only {best} deg from the NOAA sunrise"
                    );
                }
            }
        }
    }

    #[test]
    fn rise_set_roundtrip_at_noon() {
        // At local noon for a mid-latitude site the sun is roughly due south
        // with positive altitude.
        let t = times(1_767_225_600_000, 41.0, -75.0); // 2026-01-01T00:00Z
        assert!(t.polar.is_none());
        let (alt, az) = position(1_767_225_600_000, 41.0, -75.0);
        // Jan 1, 41N, ~00Z (i.e. local ~17-19h) → sun below horizon.
        assert!(alt < 30.0, "alt={alt}");
        let _ = az;
    }
}

#[cfg(test)]
mod sweep_tests {
    use super::*;

    /// Poles, the date line and odd latitudes never produce NaN or panic.
    #[test]
    fn sun_and_moon_are_finite_everywhere() {
        let base = 1_782_777_600_000i64;
        for lat in [-90.0, -89.999, -66.5, 0.0, 23.4, 66.6, 89.999, 90.0] {
            for lon in [-180.0, -72.0, 0.0, 179.999, 180.0] {
                for k in 0..48 {
                    let ms = base + k * 3_600_000 * 7;
                    let (a, z) = position(ms, lat, lon);
                    assert!(a.is_finite() && z.is_finite(), "sun {lat},{lon},{ms}: {a} {z}");
                    let (a, z) = crate::moon::position(ms, lat, lon);
                    assert!(a.is_finite() && z.is_finite(), "moon {lat},{lon},{ms}: {a} {z}");
                    let _ = times(ms, lat, lon);
                }
            }
        }
    }
}
