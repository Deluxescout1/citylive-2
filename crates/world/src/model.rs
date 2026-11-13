//! Shared calendar / astronomical helpers, matching the exact JS math in
//! old CityLive `city.js`.
//!
//! Conventions:
//! * Unix milliseconds are absolute instants (`Date.valueOf()` in the JS).
//! * A [`Date`] is a *local calendar* stamp (year/month/day/hour) — the thing
//!   CityLive's `holidays`/`seasonInfo` read via `getMonth()`/`getDate()`.
//! * Julian Date follows the JS: `jd = ms / 86_400_000 + 2_440_587.5`.
//!   (`sunTimes` used `ms/86_400_000 - 0.5 + 2_440_588`, which is identical.)

/// A local calendar date with an hour component (hour needed for NYE /
/// New-Year's-Day windows that key off `getHours()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
    /// 0..=23
    pub hour: u8,
}

impl Date {
    pub const fn new(year: i32, month: u8, day: u8, hour: u8) -> Self {
        Self { year, month, day, hour }
    }

    /// 0 = Sunday .. 6 = Saturday (proleptic Gregorian, matches the JS
    /// `new Date(y, m-1, d).getDay()` which is a property of the calendar date).
    pub fn weekday(&self) -> u32 {
        weekday(self.year, self.month as i32, self.day as i32)
    }
}

/// Days from the civil epoch of `days_from_civil` (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i32, m: i32, d: i32) -> i64 {
    let y = i64::from(if m <= 2 { y - 1 } else { y });
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // 1..400
    let doy = (153 * i64::from(if m > 2 { m - 3 } else { m + 9 }) + 2) / 5
        + i64::from(d)
        - 1; // 0..365
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // 0..146096
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`] → (year, month 1..12, day 1..31).
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (y as i32, m as u32, d as u32)
}

/// 0 = Sunday .. 6 = Saturday for a proleptic Gregorian date.
pub fn weekday(y: i32, m: i32, d: i32) -> u32 {
    let days = days_from_civil(y, m, d);
    // 1970-01-01 was a Thursday (=4); rem_euclid keeps r in 0..7.
    let r = days.rem_euclid(7) as u32;
    (4 + r) % 7
}

/// Unix ms → Julian Date (the JS `JD = valueOf()/86400000 + 2440587.5`).
pub fn julian_from_ms(ms: i64) -> f64 {
    ms as f64 / 86_400_000.0 + 2_440_587.5
}

/// Julian Date → Unix ms (the JS `fromJulian` / `new Date((j+0.5-J1970)*86400000)`).
/// J1970 = 2_440_588, so `(j + 0.5 - 2_440_588) * 86400000`.
pub fn ms_from_julian(j: f64) -> i64 {
    ((j + 0.5f64 - 2_440_588.0) * 86_400_000.0).round() as i64
}

/// `Math.floor` equivalent on f64 (JS floor; used by moon phase).
pub fn floor(v: f64) -> f64 {
    v.floor()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekdays_match_known_values() {
        // A few known calendar weekdays (US calendar, TZ-independent).
        assert_eq!(weekday(2026, 1, 1), 4); // Thursday
        assert_eq!(weekday(2026, 11, 1), 0); // Sunday
        assert_eq!(weekday(2026, 4, 22), 3); // Wednesday (Earth day)
        // Nov 1 2026 is a Sunday → Thanksgiving = 4th Thursday = 26.
        let thanks = 1 + ((4 - weekday(2026, 11, 1) + 7) % 7) + 21;
        assert_eq!(thanks, 26);
    }

    #[test]
    fn julian_roundtrip() {
        let ms: i64 = 1_767_225_600_000; // arbitrary
        let j = julian_from_ms(ms);
        assert!(ms_from_julian(j).abs_diff(ms) <= 1);
    }
}
