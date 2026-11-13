//! Calendar: port of old CityLive `easterDate` (city.js:1528), `holidays`
//! (city.js:1532) and `seasonInfo` (city.js:1580).

use crate::model::Date;

/// Every holiday CityLive celebrates. **Public holidays only** (D60): the old
/// game's personal days — the creator's birthday on Feb 17 and "Kathy" on
/// Dec 18 — are deliberately gone, so the city celebrates the calendar, not a
/// family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Holiday {
    Xmas,
    Halloween,
    July4,
    Nye,
    Valentine,
    StPatricks,
    Thanks,
    Easter,
    NewYearDay,
    EarthDay,
    MayDay,
    Lantern,
    Kite,
    Founders,
    Harvest,
}

impl Holiday {
    /// Stable key, identical to the JS flag/object names.
    pub fn as_str(self) -> &'static str {
        match self {
            Holiday::Xmas => "xmas",
            Holiday::Halloween => "halloween",
            Holiday::July4 => "july4",
            Holiday::Nye => "nye",
            Holiday::Valentine => "valentine",
            Holiday::StPatricks => "stpatrick",
            Holiday::Thanks => "thanks",
            Holiday::Easter => "easter",
            Holiday::NewYearDay => "newyearday",
            Holiday::EarthDay => "earthday",
            Holiday::MayDay => "mayday",
            Holiday::Lantern => "lantern",
            Holiday::Kite => "kite",
            Holiday::Founders => "founders",
            Holiday::Harvest => "harvest",
        }
    }
}

/// B76: the share of the town with holiday lights up on this date, 0..1.
///
/// Residents start the first weekend of December and the town fills up over
/// three weeks; the lights come down in the first week of January. Outside that
/// window nothing is lit (the public-holiday rules of `flag` are untouched).
pub fn holiday_lights(month: u8, day: u8) -> f32 {
    if month == 12 {
        // Dec 1: a few early ones; Dec 22: the whole street.
        (day as f32 / 21.0).clamp(0.04, 1.0)
    } else if month == 1 && day <= 6 {
        // Down over the first six days of the new year.
        (1.0 - (day as f32 - 1.0) / 5.0).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The order the *buildings* dress in (first active wins), verbatim from the
/// JS `order` array.
const DECOR_ORDER: [Holiday; 8] = [
    Holiday::Xmas,
    Holiday::Halloween,
    Holiday::July4,
    Holiday::Nye,
    Holiday::Valentine,
    Holiday::StPatricks,
    Holiday::Thanks,
    Holiday::Easter,
];

/// Canonical emission order (matches the JS object-literal key order).
const CANON: [Holiday; 15] = [
    Holiday::Xmas,
    Holiday::Halloween,
    Holiday::July4,
    Holiday::Nye,
    Holiday::Valentine,
    Holiday::StPatricks,
    Holiday::Thanks,
    Holiday::Easter,
    Holiday::NewYearDay,
    Holiday::EarthDay,
    Holiday::MayDay,
    Holiday::Lantern,
    Holiday::Kite,
    Holiday::Founders,
    Holiday::Harvest,
];

/// Easter Sunday for a Gregorian year (Anonymous Gregorian algorithm).
/// Verbatim port of `easterDate`.
pub fn easter_date(y: i32) -> (u8, u8) {
    // JS `|0` is integer truncation toward zero; every operand here is
    // non-negative, so this equals the JS results exactly.
    let y = i64::from(y);
    let a = y % 19;
    let b = y / 100; // (y/100)|0
    let c = y % 100;
    let dd = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g2 = (b - f + 1) / 3;
    let h = (19 * a + b - dd - g2 + 15) % 30;
    let i2 = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i2 - h - k) % 7;
    let mth = (a + 11 * h + 22 * l) / 451;
    let mo = (h + l - 7 * mth + 114) / 31;
    let day = (h + l - 7 * mth + 114) % 31 + 1;
    (mo as u8, day as u8)
}

fn flag(h: Holiday, d: &Date) -> bool {
    let y = d.year;
    let m = d.month as i32;
    let day = d.day as i32;
    let hr = d.hour as i32;
    match h {
        Holiday::Xmas => m == 12 && day >= 12 && day <= 26,
        Holiday::Halloween => m == 10 && day >= 24,
        Holiday::July4 => m == 7 && (day == 3 || day == 4 || day == 5),
        Holiday::Nye => (m == 12 && day == 31 && hr >= 18) || (m == 1 && day == 1 && hr < 2),
        Holiday::Valentine => m == 2 && day == 14,
        Holiday::StPatricks => m == 3 && day == 17,
        Holiday::Thanks => {
            let nov1 = Date::new(y, 11, 1, 0).weekday() as i32;
            let thanks = 1 + ((4 - nov1 + 7) % 7) + 21;
            m == 11 && day == thanks
        }
        Holiday::Easter => {
            // The Sunday and the Saturday before it, by day number: when Easter
            // is 1 April the Saturday is 31 March, which a "same month, day - 1"
            // test misses (B93).
            let (em, ed) = easter_date(y);
            let easter = crate::model::days_from_civil(y, i32::from(em), i32::from(ed));
            let today = crate::model::days_from_civil(y, m, day);
            today == easter || today == easter - 1
        }
        Holiday::NewYearDay => m == 1 && day == 1 && hr >= 2,
        Holiday::EarthDay => m == 4 && day == 22,
        Holiday::MayDay => m == 5 && day == 1,
        Holiday::Lantern => m == 8 && day == 15,
        Holiday::Kite => m == 4 && day >= 18 && day <= 20,
        Holiday::Founders => m == 6 && day == 1,
        Holiday::Harvest => m == 9 && day >= 20 && day <= 22,
    }
}

/// All active holidays / personal days for a local date, in canonical order.
pub fn holidays(date: &Date) -> Vec<Holiday> {
    CANON.iter().filter(|h| flag(**h, date)).copied().collect()
}

/// Which holiday the buildings are currently dressed for (first match in
/// `DECOR_ORDER`), or `None`.
pub fn decorated(date: &Date) -> Option<Holiday> {
    let set: std::collections::HashSet<_> = holidays(date).into_iter().collect();
    DECOR_ORDER.iter().find(|h| set.contains(h)).copied()
}

/// Per-holiday decoration data (garland palette / prop / window tint), ported
/// from the JS `HOLIDECOR`.  Data only — the renderer reads it.
pub struct Decor {
    pub garland: &'static [[u8; 3]],
    pub prop: &'static str,
    pub tint: f32,
}

/// The `HOLIDECOR` table: one row per decorated holiday.
pub fn decor(h: Holiday) -> Option<Decor> {
    use Holiday::*;
    Some(match h {
        Xmas => Decor {
            garland: &[[255, 68, 68], [68, 255, 102], [255, 224, 138], [68, 136, 255]],
            prop: "wreath",
            tint: 0.0,
        },
        Halloween => Decor {
            garland: &[[255, 138, 26], [160, 74, 208], [255, 210, 122]],
            prop: "pumpkin",
            tint: 0.0,
        },
        July4 => Decor {
            garland: &[[255, 68, 68], [255, 255, 255], [68, 102, 255]],
            prop: "flag",
            tint: 0.45,
        },
        Nye => Decor {
            garland: &[
                [255, 224, 138],
                [223, 230, 246],
                [255, 122, 208],
                [122, 255, 215],
            ],
            prop: "balloon",
            tint: 0.45,
        },
        Valentine => Decor {
            garland: &[[255, 90, 160], [255, 158, 196], [255, 59, 107]],
            prop: "heart",
            tint: 0.45,
        },
        StPatricks => Decor {
            garland: &[[47, 191, 90], [143, 224, 160], [255, 224, 138]],
            prop: "clover",
            tint: 0.5,
        },
        Thanks => Decor {
            garland: &[[217, 130, 43], [181, 101, 29], [224, 176, 64], [138, 59, 30]],
            prop: "leaf",
            tint: 0.5,
        },
        Easter => Decor {
            garland: &[[255, 158, 196], [166, 227, 255], [200, 245, 160], [255, 224, 138]],
            prop: "egg",
            tint: 0.5,
        },
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Season {
    pub name: &'static str,
    pub bare: bool,
    pub blossom: bool,
    /// Canopy palette `[rgb; 3]`, verbatim from the JS.
    pub canopy: [[u8; 3]; 3],
}

/// Season by (local) month — verbatim port of `seasonInfo`.  The original JS
/// ignores `lat` (it is month-based, Northern-Hemisphere); `lat` is accepted
/// for API symmetry and left unused to keep the port faithful.
pub fn season(date: &Date, _lat: f64) -> Season {
    let m = date.month as i32;
    if m == 12 || m <= 2 {
        Season {
            name: "winter",
            bare: true,
            blossom: false,
            canopy: [[86, 94, 86], [104, 110, 100], [74, 84, 78]],
        }
    } else if m <= 5 {
        Season {
            name: "spring",
            bare: false,
            blossom: true,
            canopy: [[112, 172, 92], [150, 200, 120], [124, 182, 104]],
        }
    } else if m <= 8 {
        Season {
            name: "summer",
            bare: false,
            blossom: false,
            canopy: [[52, 128, 58], [70, 148, 74], [42, 110, 52]],
        }
    } else {
        Season {
            name: "autumn",
            bare: false,
            blossom: false,
            canopy: [[206, 120, 42], [212, 86, 44], [188, 152, 52]],
        }
    }
}

#[cfg(test)]
mod tests {
    /// B93: Easter Saturday is dressed even when it falls in the month before
    /// Easter (Easter 2029 is 1 April, so its Saturday is 31 March).
    #[test]
    fn easter_saturday_across_a_month_boundary() {
        assert_eq!(easter_date(2029), (4, 1));
        let sat = Date::new(2029, 3, 31, 12);
        assert!(holidays(&sat).contains(&Holiday::Easter));
        assert!(holidays(&Date::new(2029, 4, 1, 12)).contains(&Holiday::Easter));
        assert!(!holidays(&Date::new(2029, 3, 30, 12)).contains(&Holiday::Easter));
        assert!(!holidays(&Date::new(2029, 4, 2, 12)).contains(&Holiday::Easter));
    }

    use super::*;

    #[test]
    fn easter_known_years() {
        // Known Easter Sundays.
        assert_eq!(easter_date(2024), (3, 31));
        assert_eq!(easter_date(2025), (4, 20));
        assert_eq!(easter_date(2026), (4, 5));
        assert_eq!(easter_date(2027), (3, 28));
    }

    #[test]
    fn thanks_day_2026() {
        // 4th Thursday of Nov 2026 = 26.
        assert!(holidays(&Date::new(2026, 11, 26, 12)).contains(&Holiday::Thanks));
        assert!(!holidays(&Date::new(2026, 11, 25, 12)).contains(&Holiday::Thanks));
    }

    #[test]
    fn xmas_window_and_nye_split() {
        assert!(holidays(&Date::new(2026, 12, 12, 12)).contains(&Holiday::Xmas));
        assert!(!holidays(&Date::new(2026, 12, 11, 12)).contains(&Holiday::Xmas));
        assert!(holidays(&Date::new(2026, 12, 26, 12)).contains(&Holiday::Xmas));
        assert!(!holidays(&Date::new(2026, 12, 27, 12)).contains(&Holiday::Xmas));
        // NYE: Dec 31 evening only
        assert!(holidays(&Date::new(2026, 12, 31, 18)).contains(&Holiday::Nye));
        assert!(!holidays(&Date::new(2026, 12, 31, 17)).contains(&Holiday::Nye));
        // Jan 1 split: before 02:00 NYE, from 02:00 NewYearDay
        assert!(holidays(&Date::new(2027, 1, 1, 1)).contains(&Holiday::Nye));
        assert!(holidays(&Date::new(2027, 1, 1, 2)).contains(&Holiday::NewYearDay));
        assert!(!holidays(&Date::new(2027, 1, 1, 2)).contains(&Holiday::Nye));
    }

    /// D60: public holidays only. The old personal days (Feb 17, Dec 18) must
    /// never light the city up again.
    #[test]
    fn no_personal_days() {
        // Feb 17 used to be the creator's birthday: now it is an ordinary day.
        assert!(holidays(&Date::new(2026, 2, 17, 12)).is_empty());
        // Dec 18 used to be "Kathy": now it is only the Christmas season.
        assert_eq!(
            holidays(&Date::new(2026, 12, 18, 12)),
            vec![Holiday::Xmas],
            "Dec 18 must be Christmas and nothing else"
        );
        // The public holidays round them are untouched.
        assert!(holidays(&Date::new(2026, 2, 14, 12)).contains(&Holiday::Valentine));
        assert!(holidays(&Date::new(2026, 12, 25, 12)).contains(&Holiday::Xmas));
    }

    #[test]
    fn seasons_by_month() {
        assert_eq!(season(&Date::new(2026, 1, 15, 12), 0.0).name, "winter");
        assert_eq!(season(&Date::new(2026, 3, 1, 12), 0.0).name, "spring");
        assert_eq!(season(&Date::new(2026, 6, 15, 12), 0.0).name, "summer");
        assert_eq!(season(&Date::new(2026, 11, 1, 12), 0.0).name, "autumn");
        assert_eq!(season(&Date::new(2026, 12, 1, 12), 0.0).name, "winter");
    }

    /// B76: the town's lights are up from the first days of December to the
    /// first week of January, and nowhere else.
    #[test]
    fn holiday_lights_keep_their_window() {
        assert_eq!(holiday_lights(11, 30), 0.0, "no lights before December");
        assert_eq!(holiday_lights(10, 31), 0.0);
        let d1 = holiday_lights(12, 1);
        let d8 = holiday_lights(12, 8);
        let d15 = holiday_lights(12, 15);
        let d22 = holiday_lights(12, 22);
        assert!(d1 > 0.0 && d1 < d8 && d8 < d15 && d15 < d22, "they go up by degrees: {d1} {d8} {d15} {d22}");
        assert!((d22 - 1.0).abs() < 0.01, "the whole town is lit by Dec 22: {d22}");
        assert_eq!(holiday_lights(12, 31), 1.0);
        assert!(holiday_lights(1, 2) < 1.0 && holiday_lights(1, 2) > 0.0, "still up on Jan 2");
        assert_eq!(holiday_lights(1, 6), 0.0, "down by Twelfth Night");
        assert_eq!(holiday_lights(1, 7), 0.0);
        assert_eq!(holiday_lights(7, 1), 0.0, "midsummer is not Christmas");
    }

    #[test]
    fn decorated_prefers_xmas() {
        // Late December: both xmas and (on Dec 18) bday are active, but xmas wins.
        assert_eq!(decorated(&Date::new(2026, 12, 18, 12)), Some(Holiday::Xmas));
        assert_eq!(decorated(&Date::new(2026, 1, 15, 12)), None);
    }
}
