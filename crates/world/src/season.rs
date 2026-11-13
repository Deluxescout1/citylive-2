//! Seasons as the *renderer* needs them (B28): a continuous, latitude-aware
//! description of the year, not just a month name.
//!
//! `calendar::season()` stays the verbatim CityLive port (name/bare/blossom and
//! a 3-colour canopy) because the JS parity test pins it. This module adds what
//! a pixel city wants on top of it:
//!
//! * **southern hemisphere**: the month wheel is shifted half a year below the
//!   equator, so December is summer in Sydney;
//! * **gradual transitions**: everything is a function of the *day of the year*
//!   through a smooth curve, so trees turn over weeks instead of on the 1st;
//! * **per-tree variety**: [`SeasonLook::turn`] takes the tree's own hash, so a
//!   wood turns yellow-orange-red over a month, a few trees at a time, instead of
//!   all on one day (the brief's "gradually per tree (seeded)");
//! * **fields**: a 0..1 stage through ploughed → green shoots → golden crop →
//!   stubble, twice a year in the growing season;
//! * **light**: a warmth offset the sky adds to its ambient tint (summer warm,
//!   winter cold).

use crate::model::Date;

/// Everything the renderer reads about the season in one struct.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeasonLook {
    /// "winter" | "spring" | "summer" | "autumn" (as the parity port names them).
    pub name: &'static str,
    /// Position in the year, 0..1 where 0 = 1 January (northern hemisphere).
    pub year_phase: f32,
    /// How far the deciduous trees have turned: 0 = green, 1 = bare.
    pub bareness: f32,
    /// How far into the autumn colour change the leaves are, 0..1. Separate
    /// from `bareness`, which is also high in early spring when there are
    /// simply no leaves yet (B42).
    pub autumn: f32,
    /// Blossom on the deciduous trees, 0..1 (spring only).
    pub blossom: f32,
    /// Fresh new leaf, 0..1: the first green of spring, before the canopy
    /// hardens into summer's deep green (B76).
    pub fresh: f32,
    /// Leaves on the ground, 0..1 (early/mid autumn).
    pub litter: f32,
    /// Grass going dry and brown, 0..1 (late autumn to early spring).
    pub grass_dry: f32,
    /// Where a field is in its cycle, 0..1: ploughed → shoots → crop → stubble.
    pub field_stage: f32,
    /// Extra warmth added to the light: + summer, − winter.
    pub light_warmth: f32,
    /// The deciduous canopy palette to mix towards as the leaves turn, and the
    /// spring/summer green. `[light, mid, dark]` each.
    pub green: [[u8; 3]; 3],
    pub turn: [[u8; 3]; 3],
}

impl Default for SeasonLook {
    fn default() -> Self {
        Self::at(&Date::new(2026, 7, 1, 12), 41.5)
    }
}

impl SeasonLook {
    /// The season for a date at a latitude (positive north, negative south).
    pub fn at(date: &Date, lat: f64) -> Self {
        // `p` runs 0..1 from the start of (local) spring: 0 = 1 March north /
        // 1 September south, 0.25 = summer, 0.5 = autumn, 0.75 = winter.
        let p = year_phase(date, lat);
        let name = match p {
            x if x < 0.25 => "spring",
            x if x < 0.50 => "summer",
            x if x < 0.75 => "autumn",
            _ => "winter",
        };
        let _ = date;
        // Trees leaf out over the first third of spring and drop through the
        // second half of autumn, so a wood changes over weeks, not on the 1st.
        let bareness = if p < 0.30 {
            1.0 - smooth01(p / 0.30)
        } else if p < 0.55 {
            0.0
        } else if p < 0.82 {
            smooth01((p - 0.55) / 0.27)
        } else {
            1.0
        };
        let autumn = if (0.52..0.86).contains(&p) {
            smooth01((p - 0.52) / 0.22)
        } else {
            0.0
        };
        let blossom = if p < 0.28 {
            // B76: blossom is a fortnight, not a season. A third of March to
            // mid-April: it comes on over a week, peaks, and is gone.
            (smooth01((p - 0.085) / 0.030) * (1.0 - smooth01((p - 0.125) / 0.035)))
                .clamp(0.0, 1.0)
        } else {
            0.0
        };
        // Fresh, pale new leaf: spring only, strongest as the buds burst and
        // gone by the time the canopy is full (B76).
        let fresh = if p < 0.34 {
            (1.0 - smooth01(p / 0.30)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let litter = if (0.55..0.88).contains(&p) {
            (smooth01((p - 0.56) / 0.14) * (1.0 - smooth01((p - 0.80) / 0.08) * 0.6))
                .clamp(0.0, 1.0)
        } else {
            0.0
        };
        // Grass straws off as autumn ends and greens again in early spring.
        // B76: by mid-January it has to *look* dormant — the old curve only
        // reached half-brown on 15 January.
        let grass_dry = if p > 0.80 {
            smooth01((p - 0.80) / 0.09)
        } else if p < 0.12 {
            1.0 - smooth01(p / 0.12)
        } else {
            0.0
        };
        // Fields (B80): a continuous crop year, 0..1, so the renderer can show
        // every step of it. 0.00-0.12 ploughed and harrowed, 0.12-0.40 shoots
        // coming up, 0.40-0.70 green turning gold, 0.70-0.92 ripe (the harvest
        // is 0.80-0.95), 0.92-1.0 stubble. After the stubble is turned (autumn
        // ploughing) the field is bare earth again through the winter.
        let field_stage = field_stage_at(p);
        let light_warmth = if (0.22..0.52).contains(&p) {
            0.10
        } else if p > 0.78 || p < 0.08 {
            -0.10
        } else {
            0.0
        };
        Self {
            name,
            year_phase: p,
            bareness,
            autumn,
            blossom,
            fresh,
            litter,
            grass_dry,
            field_stage,
            light_warmth,
            green: [[0x5a, 0xb4, 0x46], [0x3f, 0x94, 0x3c], [0x27, 0x69, 0x30]],
            turn: [[0xd8, 0xb0, 0x3a], [0xc8, 0x6e, 0x2c], [0x9a, 0x3a, 0x2c]],
        }
    }

    /// The turn for one tree: each tree is offset in the season by its own hash,
    /// so a wood changes colour a tree at a time (B28).
    pub fn turn_for(&self, seed: f32) -> f32 {
        let jitter = (seed - 0.5) * 0.34;
        (self.bareness + jitter).clamp(0.0, 1.0)
    }

    /// Coarse buckets for the renderer: a season only re-rasterises the city when
    /// one of these changes (a few times a day at most).
    pub fn buckets(&self) -> (i32, i32, i32, i32) {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 24.0).round() as i32;
        (q(self.bareness), q(self.blossom), q(self.grass_dry), q(self.field_stage))
    }
}

/// 0..1 position in the (hemisphere-aware) year: 0 = northern spring start.
fn year_phase(date: &Date, lat: f64) -> f32 {
    let doy = day_of_year(date) as f32;
    let shifted = if lat < 0.0 { (doy + 182.5) % 365.25 } else { doy };
    ((shifted - 59.0).rem_euclid(365.25)) / 365.25
}

/// Smoothstep on 0..1 (clamped).
fn smooth01(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A date from epoch milliseconds (UTC — good enough for a season, which moves
/// by a day at a time).
pub fn date_from_ms(ms: i64) -> Date {
    let days = ms.div_euclid(86_400_000);
    let secs = (ms.rem_euclid(86_400_000) / 1000) as u32;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    Date::new(
        year as i32,
        m as u8,
        d as u8,
        (secs / 3600) as u8,
    )
}

/// Parse `YYYY-MM-DD` (the `--date` override).
pub fn parse_date(s: &str) -> Option<Date> {
    let parts: Vec<i64> = s.split('-').filter_map(|p| p.parse().ok()).collect();
    if parts.len() == 3 {
        Some(Date::new(parts[0] as i32, parts[1] as u8, parts[2] as u8, 12))
    } else {
        None
    }
}

/// Day of the year, 1 January = 1.
pub fn day_of_year(d: &Date) -> u32 {
    const CUM: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = (d.year % 4 == 0 && d.year % 100 != 0) || d.year % 400 == 0;
    let mut day = CUM[(d.month.clamp(1, 12) - 1) as usize] + d.day as u32;
    if leap && d.month > 2 {
        day += 1;
    }
    day
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day, 12)
    }

    #[test]
    fn the_northern_year_goes_the_right_way_round() {
        let jan = SeasonLook::at(&d(2026, 1, 15), 41.5);
        let apr = SeasonLook::at(&d(2026, 4, 15), 41.5);
        let jul = SeasonLook::at(&d(2026, 7, 15), 41.5);
        let oct = SeasonLook::at(&d(2026, 10, 15), 41.5);
        assert!(jan.bareness > 0.9, "January should be bare: {}", jan.bareness);
        assert!(apr.blossom > 0.3, "April should blossom: {}", apr.blossom);
        assert!(jul.bareness < 0.05, "July should be in leaf: {}", jul.bareness);
        assert!(oct.bareness > 0.1 && oct.litter > 0.2, "October is autumn");
        assert!(jan.grass_dry > 0.2, "winter grass is dry");
        assert_eq!(jul.grass_dry, 0.0);
    }

    /// B28: the southern hemisphere is half a year out of phase.
    #[test]
    fn the_southern_hemisphere_is_flipped() {
        let jul_s = SeasonLook::at(&d(2026, 7, 15), -33.9); // Sydney
        let jul_n = SeasonLook::at(&d(2026, 7, 15), 41.5);
        assert!(jul_s.bareness > 0.5, "July is winter in Sydney: {}", jul_s.bareness);
        assert!(jul_n.bareness < 0.05);
        // ... and January is their summer.
        let jan_s = SeasonLook::at(&d(2026, 1, 15), -33.9);
        assert!(jan_s.bareness < 0.1, "January is summer in Sydney");
    }

    #[test]
    fn trees_turn_a_few_at_a_time() {
        let oct = SeasonLook::at(&d(2026, 10, 5), 41.5);
        let a = oct.turn_for(0.1);
        let b = oct.turn_for(0.9);
        assert!((a - b).abs() > 0.1, "trees should differ: {a} vs {b}");
    }

    #[test]
    fn fields_move_through_the_year() {
        let spring = SeasonLook::at(&d(2026, 4, 1), 41.5);
        let summer = SeasonLook::at(&d(2026, 8, 1), 41.5);
        let autumn = SeasonLook::at(&d(2026, 10, 1), 41.5);
        assert!(spring.field_stage < 0.3);
        assert!(summer.field_stage > 0.6);
        assert!(autumn.field_stage > 0.9);
    }

    #[test]
    fn buckets_only_change_a_few_times_a_day() {
        let mut last = SeasonLook::at(&d(2026, 10, 1), 41.5).buckets();
        let mut changes = 0;
        for hour in 0..24 * 30 {
            let date = Date::new(2026, 10, 1 + (hour / 24) as u8, (hour % 24) as u8);
            let b = SeasonLook::at(&date, 41.5).buckets();
            if b != last {
                changes += 1;
                last = b;
            }
        }
        assert!(changes < 60, "a month of buckets changed {changes} times");
    }
}

/// The crop year at year phase `p` (0 = start of spring), see `SeasonLook::field_stage`.
pub fn field_stage_at(p: f32) -> f32 {
    const PTS: [(f32, f32); 9] = [
        (0.00, 0.03),
        (0.09, 0.10),
        (0.14, 0.20),
        (0.22, 0.40),
        (0.32, 0.64),
        (0.40, 0.80),
        (0.48, 0.92),
        (0.54, 0.96),
        (0.62, 1.00),
    ];
    if p >= 0.64 {
        // The stubble is ploughed in: bare earth until the spring.
        return 0.02;
    }
    if p >= 0.62 {
        return 1.0;
    }
    for w in PTS.windows(2) {
        let ((p0, s0), (p1, s1)) = (w[0], w[1]);
        if p <= p1 {
            let t = ((p - p0) / (p1 - p0)).clamp(0.0, 1.0);
            return s0 + (s1 - s0) * t;
        }
    }
    1.0
}
