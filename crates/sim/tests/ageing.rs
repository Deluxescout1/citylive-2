//! B78 (D84): neighbourhoods that age and renew — the sim's half.
//!
//! The brief's tests: deterministic per seed; the run-down share per district is
//! never above the cap; every boarded-up building is resolved within the bound;
//! graffiti and murals never land on a home's front door or on a landmark.

use sim::ageing::{self, BOARD_DAYS, RUN_DOWN_CAP};
use sim::{District, Pace, Sim};

fn town(days: f64) -> Sim {
    let mut s = Sim::new(7, Pace::Month, 90, 90);
    s.ending.world_days = 1e9;
    s.run_to(days * 86_400.0);
    s
}

/// The run-down share of each district, as (district, share, lots).
fn run_down_shares(s: &Sim) -> Vec<(District, f32, usize)> {
    let mut by: Vec<(District, usize, usize)> = Vec::new();
    for (i, l) in s.lots.iter().enumerate() {
        if !l.finished() {
            continue;
        }
        let d = s.district_at(l.x, l.y);
        let run_down = s.wear().get(i).map(|w| w.grime >= 3).unwrap_or(false);
        match by.iter_mut().find(|(dd, _, _)| *dd == d) {
            Some(e) => {
                e.1 += 1;
                e.2 += run_down as usize;
            }
            None => by.push((d, 1, run_down as usize)),
        }
    }
    by.into_iter()
        .map(|(d, n, rd)| (d, rd as f32 / n.max(1) as f32, n))
        .collect()
}

#[test]
fn the_same_seed_ages_the_same_way() {
    let a = town(60.0);
    let b = town(60.0);
    assert_eq!(a.wear(), b.wear(), "two worlds with one seed wear alike");
    assert_eq!(a.hash(), b.hash());
    assert!(!a.wear().is_empty());
}

#[test]
fn a_two_month_old_city_is_crisper_than_a_four_month_old_one() {
    let a = town(30.0);
    let b = town(120.0);
    let mean = |s: &Sim| {
        let mut n = 0usize;
        let mut sum = 0u32;
        for (i, l) in s.lots.iter().enumerate() {
            if !l.finished() {
                continue;
            }
            n += 1;
            sum += s.wear().get(i).map(|w| w.grime as u32).unwrap_or(0);
        }
        sum as f32 / n.max(1) as f32
    };
    let (ma, mb) = (mean(&a), mean(&b));
    assert!(mb > ma + 0.25, "the town weathers with age: {ma:.2} -> {mb:.2}");
    // And not everything is derelict even at four months: the renewal is real.
    let run_down = b.wear().iter().filter(|w| w.grime >= 3).count();
    assert!(run_down < b.wear().len() / 4, "never bleak: {run_down} of {}", b.wear().len());
}

#[test]
fn the_run_down_share_never_breaks_the_district_cap() {
    for days in [35.0, 60.0, 120.0] {
        let s = town(days);
        for (d, share, n) in run_down_shares(&s) {
            if n < 8 {
                continue; // a handful of lots cannot show a share
            }
            assert!(
                share <= RUN_DOWN_CAP + 0.02,
                "day {days}: {d:?} is {:.0}% run-down over {n} buildings (cap {:.0}%)",
                share * 100.0,
                RUN_DOWN_CAP * 100.0
            );
        }
    }
}

#[test]
fn every_boarded_building_comes_down_within_the_bound() {
    let mut s = Sim::new(11, Pace::Month, 90, 90);
    s.ending.world_days = 1e9;
    // A day at a time, so an episode can be timed: no position stays boarded
    // for longer than the bound, and a lot that was standing is later reset to a
    // fresh `started` (the crew pulled it down and put a new one up).
    let mut boarded_since: std::collections::HashMap<(i32, i32), f64> = Default::default();
    let mut started: std::collections::HashMap<(i32, i32), f64> = Default::default();
    let mut seen = 0usize;
    let mut rebuilt = 0usize;
    for day in 1..=170 {
        s.run_to(day as f64 * 86_400.0);
        let mut today: std::collections::HashSet<(i32, i32)> = Default::default();
        for (i, l) in s.lots.iter().enumerate() {
            let pos = (l.x, l.y);
            let w = match s.wear().get(i) {
                Some(w) => *w,
                None => continue,
            };
            if w.boarded {
                today.insert(pos);
                let e = *boarded_since.entry(pos).or_insert(day as f64);
                assert!(
                    day as f64 - e <= BOARD_DAYS + 2.0,
                    "boarded at {pos:?} since day {e}, still boarded on day {day}"
                );
            }
            match started.get(&pos) {
                // A standing building whose clock jumped forward: rebuilt.
                Some(&old) if l.started > old + 60.0 && day > 40 => {
                    rebuilt += 1;
                    started.insert(pos, l.started);
                }
                Some(_) => {}
                None => {
                    started.insert(pos, l.started);
                }
            }
        }
        seen += today.len();
        boarded_since.retain(|k, _| today.contains(k));
    }
    assert!(seen > 0, "a grown city boards up at least one building over 170 days");
    assert!(rebuilt > 0, "and the crew pulls one down and puts a new one up");
}

#[test]
fn street_art_never_lands_on_a_home_or_a_landmark() {
    let s = town(220.0);
    let mut murals = 0;
    let mut tags = 0;
    for (i, l) in s.lots.iter().enumerate() {
        let w = match s.wear().get(i) {
            Some(w) => *w,
            None => continue,
        };
        if w.art == 0 {
            continue;
        }
        assert!(
            ageing::takes_art(l.kind),
            "art on a {:?} at {:?} — a home's door or a landmark is not a wall",
            l.kind,
            (l.x, l.y)
        );
        if w.art >= 2 {
            murals += 1;
        } else {
            tags += 1;
        }
    }
    assert!(tags > 0, "a busy old town has spray tags");
    assert!(murals > 0, "and some of those walls become murals");
}

#[test]
fn old_buildings_get_scaffolded_and_come_back_repainted() {
    let mut s = Sim::new(3, Pace::Month, 90, 90);
    s.ending.world_days = 1e9;
    let mut scaffolded = false;
    let mut repainted = 0;
    for day in 1..=200 {
        s.run_to(day as f64 * 86_400.0);
        scaffolded |= s.wear().iter().any(|w| w.scaffold);
        repainted = repainted.max(s.wear().iter().filter(|w| w.repaint > 0).count());
    }
    assert!(scaffolded, "a renovation puts scaffolding up at some point");
    assert!(repainted > 0, "and a repainted building is left behind");
}
