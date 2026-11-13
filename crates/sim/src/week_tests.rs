//! Tests for the weekly rhythm (B77/D84): bin day, the Saturday market, the
//! school run and its holidays, and a Sunday morning that is quieter than a
//! Tuesday.

use crate::people::Pose;
use crate::{Pace, Sim};

/// A grown town on a big enough world to have a school, a church and a square.
fn grown(seed: u64) -> Sim {
    let mut s = Sim::new(seed, Pace::Month, 121, 121);
    s.run_to(30.0 * 86400.0);
    s
}

/// A grown town with a school in it (the test's own fixture: a seed whose
/// school survived the build).
fn with_school() -> Sim {
    for seed in [7u64, 3, 5, 11, 13, 2] {
        let s = grown(seed);
        if s.school_lot().is_some() {
            return s;
        }
    }
    panic!("no seed built a school: the school run has nothing to run to");
}

fn school_children(s: &Sim, hour: f64) -> usize {
    let mut v = Vec::new();
    s.school_run_into(hour, s.time, &mut v);
    v.iter().filter(|f| f.child).count()
}

fn yard_children(s: &Sim, hour: f64) -> usize {
    let mut v = Vec::new();
    s.playground_into(hour, s.time, &mut v);
    v.iter().filter(|f| f.child).count()
}

fn clock(day: f64, hour: f64) -> f64 {
    day * 86400.0 + hour * 3600.0
}

#[test]
fn bins_go_out_the_night_before_and_on_the_bin_day_only() {
    let mut s = grown(7);
    // Walk a whole week, one 07:00 (and one 20:00 the evening before) at a time.
    for d in 0..7 {
        let day = 21.0 + d as f64;
        s.time = clock(day, 7.0);
        let dow = s.week_day();
        let n = s.weekly_figure_list(7.0, s.time).iter().filter(|f| f.prop == crate::people::Prop::Bin).count();
        if dow == crate::week::SUNDAY || dow == crate::week::SATURDAY {
            assert_eq!(n, 0, "bins out on a {} morning: no collection at the weekend", crate::week::day_name(dow));
        } else {
            assert!(n > 0, "no bins out on a {} morning, which is a bin day", crate::week::day_name(dow));
        }
        // The evening before: tomorrow's district wheels its bins out at seven.
        s.time = clock(day, 20.0);
        let tomorrow = (dow + 1) % 7;
        let n = s.weekly_figure_list(20.0, s.time).iter().filter(|f| f.prop == crate::people::Prop::Bin).count();
        if tomorrow == crate::week::SUNDAY || tomorrow == crate::week::SATURDAY {
            assert_eq!(n, 0, "bins out on the eve of a {}", crate::week::day_name(tomorrow));
        } else {
            assert!(n > 0, "no bins out the evening before the {} collection", crate::week::day_name(tomorrow));
        }
    }
}

#[test]
fn each_district_has_its_own_day() {
    // A street's bins go out once a week, and the districts do not all share a
    // day: the sets of bins standing on a Monday and on a Tuesday are different
    // streets, not the same ones twice.
    let mut s = grown(7);
    let mut by_day: Vec<std::collections::BTreeSet<(i32, i32)>> = Vec::new();
    for d in 0..7 {
        // Days 21..27 are every weekday once (a Thursday founding).
        s.time = clock(21.0 + d as f64, 9.0);
        let mut bins = std::collections::BTreeSet::new();
        let mut v = Vec::new();
        s.bins_out_into(9.0, &mut v);
        for f in v {
            bins.insert((f.x.floor() as i32, f.y.floor() as i32));
        }
        by_day.push(bins);
    }
    // Mon..Fri each have bins out; the weekend has none.
    for (d, bins) in by_day.iter().enumerate() {
        // `Sim::weekday` is (sim day + 3) mod 7, 0 = Sunday.
        let name = crate::week::day_name(((21 + d + 3) % 7) as i32);
        if name == "Saturday" || name == "Sunday" {
            assert!(bins.is_empty(), "{name} has bins out");
        } else {
            assert!(!bins.is_empty(), "{name} has no bins out");
        }
    }
    // Monday's bins are not Tuesday's bins (two rounds, two streets).
    let mon = &by_day[4]; // day 25 = Monday
    let tue = &by_day[5]; // day 26 = Tuesday
    let shared = mon.intersection(tue).count();
    assert!(
        shared * 20 < mon.len().max(1),
        "the Monday and Tuesday rounds are the same street ({shared} shared bins)"
    );
}

#[test]
fn the_market_is_a_saturday() {
    let mut s = grown(7);
    assert!(s.market_place().is_some(), "no square to hold a market on");
    for d in 0..7 {
        let day = 21.0 + d as f64;
        for hour in [8.0, 11.0, 15.0] {
            s.time = clock(day, hour);
            let figs = s.weekly_figure_list(hour, s.time);
            let stalls = figs.iter().filter(|f| f.pose == Pose::Object && f.prop == crate::people::Prop::Stall).count();
            if s.week_day() == crate::week::SATURDAY {
                assert!(stalls > 0, "no stalls at {hour}:00 on a Saturday");
                assert!(figs.len() > 20, "the market has no crowd at {hour}:00");
            } else {
                assert_eq!(stalls, 0, "a market on a {}", crate::week::day_name(s.week_day()));
            }
        }
    }
    // And it is packed up by the evening.
    s.time = clock(27.0, 19.0);
    assert_eq!(s.market_stalls(19.0), 0, "stalls still standing at seven in the evening");
}

#[test]
fn no_school_run_at_the_weekend_or_in_the_holidays() {
    let mut s = with_school();
    // A term-time weekday: the children walk in, and the playground fills.
    s.set_calendar_date(2026, 10, 13); // a Tuesday
    s.time = clock(30.0, 8.0);
    let wk = s.week_day();
    assert!(wk > 0 && wk < 6, "13 October 2026 is not a weekday here");
    let run = school_children(&s, 8.0);
    assert!(s.school_in_session());
    assert!(run > 10, "no school run on a term-time Tuesday: {run} children out");

    // The weekend of the same week: nothing.
    for day in [31.0, 32.0] {
        s.time = clock(day, 8.0);
        let n = school_children(&s, 8.0);
        assert_eq!(n, 0, "a school run on a {} morning", crate::week::day_name(s.week_day()));
        assert!(!s.school_in_session());
    }

    // A Tuesday in the summer holidays: nothing either.
    s.set_calendar_date(2026, 8, 11);
    s.time = clock(30.0, 8.0);
    assert!(!s.school_in_session(), "August is a school holiday");
    let n = school_children(&s, 8.0);
    assert_eq!(n, 0, "a school run in the summer holidays");

    // And the break: a full yard in term time, an empty one in August.
    s.set_calendar_date(2026, 10, 13);
    s.time = clock(30.0, 10.1);
    let yard = yard_children(&s, 10.1);
    assert!(yard > 8, "an empty playground at break: {yard}");
    s.set_calendar_date(2026, 8, 11);
    let yard = yard_children(&s, 10.1);
    assert_eq!(yard, 0, "children in the yard in the summer holidays");
}

#[test]
fn a_sunday_morning_is_quieter_than_a_tuesday() {
    let mut a = grown(7);
    let mut b = grown(7);
    // Tuesday 07:30 (day 27 is a Tuesday for a Thursday founding) and Sunday
    // 07:30 (day 25), on the same city.
    a.time = clock(27.0, 7.5);
    b.time = clock(25.0, 7.5);
    assert_eq!(a.week_day(), crate::week::TUESDAY);
    let tue = a.weekly_figure_list(7.5, a.time).len();
    let sun = b.weekly_figure_list(7.5, b.time).len();
    assert!(
        sun * 3 < tue,
        "Sunday morning is not the quiet one: {sun} figures against {tue} on a Tuesday"
    );
    // And the roads: fewer cars on a Sunday morning than at a Tuesday rush.
    let tue_cars = crate::traffic::Traffic::busyness(a.time);
    let sun_cars = crate::traffic::Traffic::busyness(b.time);
    assert!(
        sun_cars < tue_cars * 0.5,
        "Sunday roads are not quiet: {sun_cars} against {tue_cars}"
    );
}

#[test]
fn the_easter_break_reaches_back_into_march() {
    // Easter Sunday 2024 is 31 March: the fortnight round it starts on the 22nd.
    assert!(crate::week::school_holiday(2024, 3, 28), "the week before Easter 2024");
    assert!(crate::week::school_holiday(2024, 4, 5), "the week after Easter 2024");
    assert!(!crate::week::school_holiday(2024, 3, 11), "term time in mid March");
    // Easter Sunday 2025 is 20 April: March is plain term time.
    assert!(!crate::week::school_holiday(2025, 3, 28));
    assert!(crate::week::school_holiday(2025, 4, 15));
}

#[test]
fn a_school_with_no_children_left_draws_no_run_and_does_not_panic() {
    // The exodus of an ending (or a slump) empties the roster of children while
    // the school still stands on a weekday morning in term.
    let mut s = with_school();
    for p in s.people.list.iter_mut() {
        if p.kind == crate::people::AgeGroup::Child {
            p.life = crate::people::Life::Away;
        }
    }
    s.people.recount();
    s.time = clock(26.0, 8.0); // a Monday
    assert!(s.school_in_session());
    let mut v = Vec::new();
    s.school_run_into(8.0, s.time, &mut v);
    assert_eq!(v.iter().filter(|f| f.child).count(), 0, "no children left to run");
}
