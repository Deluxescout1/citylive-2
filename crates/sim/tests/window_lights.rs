//! B72 (D78): who is behind each window — the sim's half.
//!
//! The plan must follow the residents: homes fill through the evening and empty
//! after midnight, offices empty after work, a few lights stay on all night
//! (a caretaker, a cleaner's floor, the bathroom at three), an empty building is
//! dark, and the same save at the same hour gives the same windows.

use sim::{Pace, Sim, winlights};

/// Lit windows, not lit buildings: the town's total.
fn lit(s: &Sim, h: f64) -> u32 {
    s.window_plan(h, false).iter().map(|b| b.count_ones()).sum()
}

fn town() -> Sim {
    let mut s = Sim::new(7, Pace::Month, 60, 60);
    s.ending.world_days = 1e9;
    s.run_to(25.0 * 86400.0);
    s
}

#[test]
fn the_town_has_residents() {
    let s = town();
    assert!(s.people.list.len() > 50, "a day-25 town has people");
    assert!(s.lots.iter().filter(|l| l.finished()).count() > 20, "and buildings");
}

#[test]
fn an_occupied_town_lights_up_in_the_evening() {
    let s = town();
    let count = |h: f64| lit(&s, h);
    let noon = count(12.0);
    let evening = count(20.5);
    let three = count(3.0);
    assert!(evening > noon, "more lights at night than at noon: {evening} vs {noon}");
    assert!(evening > 20, "an occupied town has lit windows at 20:30: {evening}");
    assert!(three < evening / 2, "most of them are out at three: {three} vs {evening}");
    assert!(three > 0, "but not every one: a caretaker, a bathroom");
}

#[test]
fn the_evening_fills_one_house_at_a_time() {
    let s = town();
    let steps: Vec<u32> = [17.5, 18.5, 19.5, 20.5, 21.5]
        .iter()
        .map(|&h| lit(&s, h))
        .collect();
    // Somewhere in the evening the count rises and then falls: houses fill and
    // then empty, rather than the whole town switching at one hour.
    let rises = steps.windows(2).any(|w| w[1] > w[0]);
    let falls = steps.windows(2).any(|w| w[1] < w[0]);
    assert!(rises, "the lights come on through the evening: {steps:?}");
    assert!(falls, "and go off again: {steps:?}");
}

#[test]
fn an_empty_building_has_no_lights() {
    let s = town();
    let plan = s.window_plan(20.5, false);
    // A lot nobody lives in — a car park, a park, a plinth — is dark.
    for (i, l) in s.lots.iter().enumerate() {
        if matches!(l.kind, sim::LotKind::CarPark | sim::LotKind::Park) {
            assert_eq!(plan[i], 0, "no windows behind a {:?}", l.kind);
        }
    }
}

#[test]
fn the_same_save_gives_the_same_windows() {
    let a = town();
    let b = town();
    assert_eq!(a.window_plan(20.5, false), b.window_plan(20.5, false));
    assert_eq!(a.window_plan(3.0, false), b.window_plan(3.0, false));
}

#[test]
fn a_household_never_lights_every_window_at_once() {
    // Not "all on": the rooms of one household do not all burn at once.
    let s = town();
    let plan = s.window_plan(21.0, false);
    let full = plan.iter().filter(|&&b| b == u64::MAX).count();
    assert_eq!(full, 0, "no building is fully lit");
    assert!(plan.iter().any(|&b| b.count_ones() > 2), "some are quite lit");
}

#[test]
fn the_bedtime_is_each_persons_own() {
    // The lights go out one at a time after nine: the town's count falls in more
    // than one step.
    let s = town();
    let steps: Vec<u32> = [21.0, 22.0, 23.0, 0.0, 1.0, 2.0]
        .iter()
        .map(|&h| lit(&s, h))
        .collect();
    let falls = steps.windows(2).filter(|w| w[1] < w[0]).count();
    assert!(falls >= 2, "bedtime is staggered: {steps:?}");
}

#[test]
fn a_predawn_wake_up_is_visible() {
    // A few lights come on before dawn (an early shift, a baker).
    let s = town();
    let five = lit(&s, 5.0);
    let six = lit(&s, 6.2);
    assert!(six >= five, "the town wakes: {five} at 5:00, {six} at 6:12");
    assert!(winlights::SLOTS == 64);
}
