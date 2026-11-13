//! Personal dates the city celebrates (B53 item 2, D68 "personal touches").
//!
//! Dates are entered in the settings app and stored only in `config.toml`:
//! a name, a day and month, an optional year (a one-off date) and a style.
//! Nothing here reads any other source.  A date with no year recurs every year.

use serde::{Deserialize, Serialize};

/// How the city dresses for a special date.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateStyle {
    #[default]
    Balloons,
    Banner,
    Fireworks,
    Flowers,
}

impl DateStyle {
    /// Every style, in the order the settings app offers them.
    pub const ALL: [DateStyle; 4] = [
        DateStyle::Balloons,
        DateStyle::Banner,
        DateStyle::Fireworks,
        DateStyle::Flowers,
    ];

    pub fn name(self) -> &'static str {
        match self {
            DateStyle::Balloons => "balloons",
            DateStyle::Banner => "banner",
            DateStyle::Fireworks => "fireworks",
            DateStyle::Flowers => "flowers",
        }
    }
}

/// One named date from the settings file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecialDate {
    /// Shown in the feed and the tray, e.g. "Anniversary".
    pub name: String,
    /// 1..=12.
    pub month: u8,
    /// 1..=31, valid for the month (29 Feb only in leap years).
    pub day: u8,
    /// `None` = every year; `Some(y)` = that one day only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default)]
    pub style: DateStyle,
}

/// Leap year (Gregorian).
pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Days in a month, leap-aware. `None` for a month outside 1..=12.
pub fn days_in_month(y: i32, m: u8) -> Option<u8> {
    Some(match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        _ => return None,
    })
}

impl SpecialDate {
    /// Is the entry usable?  A bad date is skipped, never a crash.
    pub fn is_valid(&self) -> bool {
        let year = self.year.unwrap_or(2000);
        !self.name.trim().is_empty()
            && days_in_month(year, self.month).is_some_and(|max| self.day >= 1 && self.day <= max)
    }

    /// Does this entry fall on the local calendar date `y-m-d`?
    pub fn matches(&self, y: i32, m: u8, d: u8) -> bool {
        if !self.is_valid() || m != self.month || d != self.day {
            return false;
        }
        match self.year {
            None => true,
            Some(year) => year == y,
        }
    }
}

/// The first valid entry on the given local date, in file order.
pub fn on_date(dates: &[SpecialDate], y: i32, m: u8, d: u8) -> Option<&SpecialDate> {
    dates.iter().find(|s| s.matches(y, m, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_year(month: u8, day: u8, style: DateStyle) -> SpecialDate {
        SpecialDate {
            name: "Test day".into(),
            month,
            day,
            year: None,
            style,
        }
    }

    #[test]
    fn recurring_date_matches_every_year() {
        let s = every_year(6, 14, DateStyle::Balloons);
        assert!(s.matches(2026, 6, 14));
        assert!(s.matches(2031, 6, 14));
        assert!(!s.matches(2026, 6, 15));
        assert!(!s.matches(2026, 7, 14));
    }

    #[test]
    fn one_off_date_matches_only_its_year() {
        let s = SpecialDate {
            year: Some(2027),
            ..every_year(3, 9, DateStyle::Banner)
        };
        assert!(s.matches(2027, 3, 9));
        assert!(!s.matches(2026, 3, 9));
    }

    #[test]
    fn leap_day_rules() {
        assert!(is_leap(2028) && !is_leap(2100) && is_leap(2000));
        let leap = every_year(2, 29, DateStyle::Flowers);
        assert!(leap.is_valid(), "29 Feb recurs; valid because 2000 is leap");
        assert!(leap.matches(2028, 2, 29));
        assert!(!leap.matches(2027, 2, 28));
        let bad = SpecialDate {
            year: Some(2027),
            ..every_year(2, 29, DateStyle::Flowers)
        };
        assert!(!bad.is_valid(), "29 Feb 2027 does not exist");
    }

    #[test]
    fn invalid_entries_are_skipped() {
        assert!(!every_year(13, 1, DateStyle::Balloons).is_valid());
        assert!(!every_year(4, 31, DateStyle::Balloons).is_valid());
        assert!(!every_year(0, 5, DateStyle::Balloons).is_valid());
        let nameless = SpecialDate {
            name: "  ".into(),
            ..every_year(5, 5, DateStyle::Balloons)
        };
        assert!(!nameless.matches(2026, 5, 5));
    }

    #[test]
    fn first_match_in_file_order_wins() {
        let list = vec![
            every_year(7, 4, DateStyle::Fireworks),
            every_year(7, 4, DateStyle::Banner),
            every_year(8, 1, DateStyle::Flowers),
        ];
        assert_eq!(on_date(&list, 2026, 7, 4).unwrap().style, DateStyle::Fireworks);
        assert_eq!(on_date(&list, 2026, 8, 1).unwrap().style, DateStyle::Flowers);
        assert!(on_date(&list, 2026, 9, 9).is_none());
    }

    #[test]
    fn style_names_are_lowercase() {
        assert_eq!(DateStyle::Flowers.name(), "flowers");
        let v: DateStyle = serde_json::from_str("\"fireworks\"").unwrap();
        assert_eq!(v, DateStyle::Fireworks);
    }
}
