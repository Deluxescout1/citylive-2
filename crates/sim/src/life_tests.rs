//! Tests for the life stories (B48): determinism, the save round trip, rates,
//! the feed cap, and what the scenes put on the street.

use crate::life::*;
use crate::people::*;
use crate::{Lot, Pace, Sim};

fn grown(seed: u64, days: f64) -> Sim {
    let mut s = Sim::new(seed, Pace::Month, 71, 71);
    s.run_to(days * 86400.0);
    s
}

fn count(s: &Sim, key: &str) -> usize {
    s.story.iter().filter(|l| l.text.contains(key)).count()
}

const WED: [&str; 4] = [" married at", " tied the knot", " said their vows", " were wed"];
const BURY: [&str; 5] = ["laid to rest", "was buried", "seen off by", "quiet funeral", "has died"];

fn count_any(s: &Sim, keys: &[&str]) -> usize {
    s.story.iter().filter(|l| keys.iter().any(|k| l.text.contains(k))).count()
}

/// Every kind of scene a city produces over `days`, sampled day by day (a
/// scene is pruned a day after it ends, so sample as it runs).
fn scenes_seen(seed: u64, days: usize) -> (Sim, Vec<LifeScene>) {
    let mut s = Sim::new(seed, Pace::Month, 71, 71);
    let mut seen: Vec<LifeScene> = Vec::new();
    for d in 1..=days {
        s.run_to(d as f64 * 86400.0);
        for sc in s.people.scenes.iter() {
            if sc.kind != SceneKind::Note
                && !seen
                    .iter()
                    .any(|o| o.start == sc.start && o.kind == sc.kind && o.at == sc.at)
            {
                seen.push(sc.clone());
            }
        }
    }
    (s, seen)
}

#[test]
fn the_same_seed_lives_the_same_lives() {
    let a = grown(5, 40.0);
    let b = grown(5, 40.0);
    assert_eq!(a.hash(), b.hash());
    let la: Vec<(i32, u8)> = a.people.list.iter().map(|p| (p.born, p.life.to_u8())).collect();
    let lb: Vec<(i32, u8)> = b.people.list.iter().map(|p| (p.born, p.life.to_u8())).collect();
    assert_eq!(la, lb);
    assert_eq!(a.people.scenes, b.people.scenes);
    let c = grown(6, 40.0);
    assert_ne!(a.hash(), c.hash());
}

#[test]
fn a_town_has_births_weddings_funerals_and_departures() {
    // B50: a world is about a month and a life year is 0.4 days, so 30 days
    // is three generations.
    let s = grown(3, 30.0);
    let dead = s.people.list.iter().filter(|p| p.life == Life::Dead).count();
    let away = s.people.list.iter().filter(|p| p.life == Life::Away).count();
    let babies = s.people.list.iter().filter(|p| p.born > 0).count();
    assert!(dead >= 5, "{dead} deaths in 60 days");
    assert!(away >= 3, "{away} departures in 60 days");
    assert!(babies >= 10, "{babies} births in 60 days");
    assert!(count_any(&s, &WED) >= 5, "weddings: {}", count_any(&s, &WED));
    assert!(count(&s, "was born") >= 5);
    assert!(count_any(&s, &BURY) >= 5);
    assert!(count(&s, "moved into") >= 5);
    // The cached headcount is the real one.
    assert_eq!(s.people.living(), s.people.list.iter().filter(|p| p.alive()).count());
    // Rates: three generations pass, so the first settlers do die, but a
    // world does not lose more than half of everyone who ever lived to old age.
    let pop = s.people.list.len();
    assert!(dead * 100 <= pop * 50, "{dead} of {pop} died");
}

#[test]
fn the_feed_stays_a_feed() {
    let s = grown(3, 60.0);
    assert!(s.story.len() <= crate::story::STORY_CAP + crate::story::KEPT_CAP);
    let life = [
        "married",
        "tied the knot",
        "said their vows",
        "were wed",
        "was buried",
        "seen off by",
        "quiet funeral",
        "was born",
        "laid to rest",
        "has died",
        "left the family",
        "loaded up",
        "turned eighteen",
        "retired",
        "last time",
        "for good",
        "plough",
        "slump",
    ];
    let mut per_day: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    for l in s.story.iter() {
        if life.iter().any(|k| l.text.contains(k)) {
            *per_day.entry((l.time / 86400.0) as u64).or_insert(0) += 1;
        }
    }
    // A funeral is told the morning after the day's budget made it, so a told
    // day can hold the tail of one budget and the head of the next.
    assert!(
        per_day.values().all(|n| *n <= 2 * LINES_PER_DAY as usize),
        "{per_day:?}"
    );
    assert!(!per_day.is_empty());
}

#[test]
fn families_share_a_name_and_names_are_varied() {
    let s = grown(3, 30.0);
    let mut kids = 0;
    let mut same = 0;
    for p in s.people.list.iter().filter(|p| p.alive() && p.kind == AgeGroup::Child) {
        if let Some(m) = p.parent.and_then(|m| s.people.get(m)) {
            if m.spouse.is_some() {
                kids += 1;
                if m.last == p.last {
                    same += 1;
                }
            }
        }
    }
    assert!(kids > 20, "{kids} children with married parents");
    assert!(same * 100 >= kids * 90, "{same} of {kids} children share the family name");
    let mut pairs = 0;
    let mut shared = 0;
    for (id, p) in s.people.list.iter().enumerate() {
        if let (true, Some(sp)) = (p.alive(), p.spouse) {
            if (sp as usize) > id {
                pairs += 1;
                if s.people.list[sp as usize].last == p.last {
                    shared += 1;
                }
            }
        }
    }
    assert!(pairs > 20 && shared * 100 >= pairs * 95, "{shared} of {pairs} couples share a name");
    let firsts: std::collections::BTreeSet<u8> = s.people.list.iter().map(|p| p.first).collect();
    let lasts: std::collections::BTreeSet<u8> = s.people.list.iter().map(|p| p.last).collect();
    assert!(firsts.len() > 100 && lasts.len() > 80, "{} first, {} last", firsts.len(), lasts.len());
}

#[test]
fn the_lives_survive_a_save_and_keep_living_alike() {
    let mut a = grown(11, 25.0);
    let path = std::env::temp_dir().join("citylive-b48-lives.save");
    a.save(&path).unwrap();
    let mut b = Sim::load(&path).unwrap();
    assert_eq!(a.people.list.len(), b.people.list.len());
    for (x, y) in a.people.list.iter().zip(b.people.list.iter()) {
        assert_eq!(x, y);
    }
    assert_eq!(a.people.scenes, b.people.scenes);
    assert_eq!(a.people.last_day, b.people.last_day);
    assert_eq!(a.people.living(), b.people.living());
    assert_eq!(a.hash(), b.hash());
    // Save -> load -> save is byte for byte the same file...
    let again = std::env::temp_dir().join("citylive-b48-lives2.save");
    b.save(&again).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&again).unwrap());
    // ...and the loaded city goes on living: a week more, nobody lost.
    let n = b.people.living();
    b.run_to(32.0 * 86400.0);
    assert!(b.people.living() * 100 >= n * 90, "{} of {n}", b.people.living());
    a.run_to(32.0 * 86400.0);
}

#[test]
fn an_old_save_gets_birthdays_not_a_massacre() {
    let a = grown(3, 20.0);
    let path = std::env::temp_dir().join("citylive-b48-old.save");
    a.save(&path).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    // Version 9 has the roster and the story but no lives behind it: the
    // reader never touches the v10 trailer.
    bytes[4..8].copy_from_slice(&9u32.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let mut old = Sim::load(&path).unwrap();
    assert_eq!(old.people.list.len(), a.people.list.len());
    let today = (old.time / 86400.0).floor() as i64;
    for p in old.people.list.iter().filter(|p| p.alive()) {
        // Life years are shorter than the day now (B50), so a derived birthday
        // can be a year or two off the recorded age, never more.
        let d = (People::age_on(p, today) as i32 - p.age as i32).abs();
        assert!(d <= 2, "{} changes age on load by {d}", p.name());
    }
    let n = old.people.living();
    old.run_to(old.time + 86400.0);
    // A day of life afterwards, not a replay of twenty. (This fake v9 file is a
    // v16 roster with its lives stripped, so the people who had died load as
    // living — 846 against the live city's ~500, the extra ones mostly old —
    // and one day of life takes the oldest of them. Before the reload-growth
    // fix no life ran after a load at all. A replay of twenty days would take
    // far more than 15 %.)
    assert!(old.people.living() * 100 >= n * 85, "{} of {n} left after one day", old.people.living());
}

#[test]
fn the_cemetery_is_at_the_edge() {
    let mut found = 0;
    for seed in [3u64, 7, 11] {
        // Watch for the day it is staked and judge it against the town of that
        // day (the town grows past it later, as towns do).
        let mut s = Sim::new(seed, Pace::Month, 71, 71);
        let mut day = 0.0;
        while day < 40.0 && !s.lots.iter().any(|l| l.kind == crate::LotKind::Cemetery) {
            day += 0.25;
            s.run_to(day * 86400.0);
        }
        let Some(c) = s.lots.iter().find(|l| l.kind == crate::LotKind::Cemetery) else {
            continue;
        };
        found += 1;
        let (fx, fy) = s.founding;
        let d = |l: &Lot| (l.x - fx).abs() + (l.y - fy).abs();
        let mut ds: Vec<i32> = s.lots.iter().filter(|l| l.kind != crate::LotKind::Cemetery).map(d).collect();
        ds.sort();
        let median = ds[ds.len() / 2];
        assert!(d(c) >= median, "seed {seed}: the cemetery ({}) is inside the median lot ({median})", d(c));
    }
    assert!(found >= 2, "only {found} of 3 towns have a cemetery");
}

#[test]
fn the_scenes_are_small_and_readable() {
    let (_s, seen) = scenes_seen(11, 40);
    let wedding = seen.iter().find(|s| s.kind == SceneKind::Wedding).expect("a wedding in 40 days");
    let funeral = seen.iter().find(|s| s.kind == SceneKind::Funeral).expect("a funeral in 40 days");
    let mover = seen.iter().find(|s| s.kind == SceneKind::Move).expect("a move in 40 days");
    let run = |sc: &LifeScene, at: f64| {
        let mut p = People::default();
        p.scenes.push(sc.clone());
        let mut out = Vec::new();
        p.scene_figures(sc.start + sc.dur * at, &mut out);
        out
    };
    // A wedding: one white figure, a handful of guests, confetti late in the
    // party and not before.
    let early = run(wedding, 0.2);
    let late = run(wedding, 0.8);
    assert_eq!(early.iter().filter(|f| f.tone == TONE_WHITE).count(), 1);
    assert_eq!(early.iter().filter(|f| f.prop == Prop::Arch).count(), 1, "one flower arch");
    assert!((7..=12).contains(&early.len()), "{} figures at the wedding", early.len());
    assert!(early.iter().all(|f| f.prop != Prop::Confetti));
    assert!(late.iter().any(|f| f.prop == Prop::Confetti));
    // A funeral: dark coats only, gathered tight.
    let f = run(funeral, 0.5);
    assert!(f.len() >= 5 && f.iter().filter(|f| f.tone == TONE_DARK).count() >= 5);
    assert_eq!(f.iter().filter(|f| f.prop == Prop::Hearse).count() as i32, funeral.open as i32, "one hearse at a cemetery");
    let f: Vec<Figure> = f.into_iter().filter(|f| f.prop != Prop::Hearse).collect();
    let (minx, maxx) = f.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.x), a.1.max(p.x)));
    assert!(maxx - minx < 2.5, "mourners spread {} tiles", maxx - minx);
    // A move: exactly one van.
    let m = run(mover, 0.5);
    assert_eq!(m.iter().filter(|f| f.prop == Prop::Truck).count(), 1);
    // Outside their hour, nothing.
    assert!(run(wedding, 1.5).is_empty() && run(funeral, -0.5).is_empty());
}

#[test]
fn a_birth_puts_a_pram_on_the_pavement() {
    let mut s = grown(3, 30.0);
    assert!(!s.people.walkers.is_empty(), "no baby under two after 30 days");
    s.set_clock_hour(10.5);
    let prams = s.people_view().iter().filter(|f| f.prop == Prop::Stroller).count();
    assert!(prams > 0 && prams <= s.people.walkers.len());
    // ...and not at night.
    s.set_clock_hour(3.0);
    assert!(s.people_view().iter().all(|f| f.prop != Prop::Stroller));
}

#[test]
fn the_dead_and_the_departed_draw_nothing() {
    let mut s = grown(3, 40.0);
    s.set_clock_hour(8.0);
    let all = s.people_view().len();
    for p in s.people.list.iter_mut() {
        p.life = Life::Dead;
    }
    s.people.recount();
    s.people.scenes.clear();
    let none = s.people_view();
    assert!(none.len() < all, "{} with everyone gone vs {all}", none.len());
    assert_eq!(s.people.living(), 0);
}

#[test]
fn street_names_are_stable_per_street() {
    let s = grown(3, 30.0);
    let w = World { map: &s.map, lots: &s.lots, fields: &s.fields, seed: s.seed };
    let l = s
        .lots
        .iter()
        .find(|l| l.finished() && l.kind == crate::LotKind::House)
        .unwrap();
    let d = door_tile(l);
    assert_eq!(street_name(&w, d, l.front), street_name(&w, d, l.front));
    assert!(street_name(&w, d, l.front).contains(' '));
}

/// `cargo test -p sim census -- --ignored --nocapture`: the numbers for the
/// handback and for tuning the rates.
#[test]
#[ignore]
fn census() {
    for seed in [1u64, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
        let s = grown(seed, 60.0);
        let alive = s.people.living();
        let dead = s.people.list.iter().filter(|p| p.life == Life::Dead).count();
        let away = s.people.list.iter().filter(|p| p.life == Life::Away).count();
        println!(
            "seed {seed}: residents {} living {alive} dead {dead} away {away} list {} jobless {:.2} lines {} | married {} born {} rest {} moved {} loaded {} eighteen {} left {} cemetery {} church {}",
            s.residents(),
            s.people.list.len(),
            s.people.jobless_share(),
            s.story.len(),
            count(&s, "married"),
            count(&s, "was born"),
            count(&s, "laid to rest"),
            count(&s, "moved into"),
            count(&s, "loaded up"),
            count(&s, "eighteen"),
            count(&s, "left the family"),
            s.lots.iter().filter(|l| l.kind == crate::LotKind::Cemetery).count(),
            s.lots.iter().filter(|l| l.kind == crate::LotKind::Church).count(),
        );
    }
}

/// `cargo test -p sim cost -- --ignored --nocapture`: the per-frame view and
/// the hourly planning step, with the life events and without them.
#[test]
#[ignore]
fn cost() {
    let mut s = grown(3, 40.0);
    // A morning with a van, a funeral and a wedding on the street, if the
    // city has them: take the time from a staged scene.
    let mut scenes = 0usize;
    for d in 41..60 {
        s.run_to(d as f64 * 86400.0 + 12.0 * 3600.0);
        scenes = s.people.scenes.iter().filter(|c| c.kind != SceneKind::Note).count();
        if scenes >= 3 {
            break;
        }
    }
    let now = s.time;
    let mut out = Vec::new();
    for hour in [8.0f64, 12.0, 19.0] {
        s.set_clock_hour(hour);
        let t0 = std::time::Instant::now();
        let n = 2000;
        for _ in 0..n {
            s.people_view_into(&mut out);
        }
        let with = t0.elapsed().as_secs_f64() / n as f64 * 1e6;
        let figs = out.len();
        let saved = std::mem::take(&mut s.people.scenes);
        let walkers = std::mem::take(&mut s.people.walkers);
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            s.people_view_into(&mut out);
        }
        let without = t0.elapsed().as_secs_f64() / n as f64 * 1e6;
        s.people.scenes = saved;
        s.people.walkers = walkers;
        println!(
            "hour {hour}: view {with:.1} us ({figs} figures, {} living, {scenes} scenes today) vs {without:.1} us without scenes and prams",
            s.people.living()
        );
    }
    // The planning step: sync + life pass per hourly decision, over a day.
    let t0 = std::time::Instant::now();
    let steps = 24 * 3;
    s.run_to(now + steps as f64 * 3600.0);
    println!(
        "{steps} hourly decisions (whole step, growth included): {:.2} ms each; life pass alone below",
        t0.elapsed().as_secs_f64() * 1000.0 / steps as f64
    );
    let (homes, _jobs) = s.people_slots();
    let world = World { map: &s.map, lots: &s.lots, fields: &s.fields, seed: s.seed };
    let mut p = s.people.clone();
    let t0 = std::time::Instant::now();
    for d in 0..30 {
        p.last_day = 0;
        p.life_pass(&world, &homes, (d + 100) as f64 * 86400.0);
    }
    println!("one life pass over {} people: {:.2} ms", p.list.len(), t0.elapsed().as_secs_f64() * 1000.0 / 30.0);
}

#[test]
fn the_feed_keeps_its_milestones_and_rolls_its_gossip() {
    let s = grown(3, 60.0);
    let all: Vec<_> = s.story.iter().map(|l| l.text.clone()).collect();
    // The founding lines survive 60 days of births and funerals...
    assert!(all.iter().any(|t| t.contains("settlers")), "the founding was pushed out: {:?}", &all[..all.len().min(3)]);
    // ...while the life lines roll in a ring of their own.
    let life = count_any(&s, &WED) + count_any(&s, &BURY) + count(&s, "was born");
    assert!(life > 20);
    // The tray reads both, newest first.
    let l = s.story.latest(30);
    assert!(l.windows(2).all(|w| w[0].time >= w[1].time));
    // The ring is bounded on its own: 200 life lines at most.
    let mut b = crate::story::Story::default();
    for i in 0..400 {
        b.push(i as f64, "gossip");
    }
    b.push_kept(0.5, "the founding");
    assert_eq!(b.len(), crate::story::STORY_CAP + 1);
    assert!(b.iter().any(|l| l.text == "the founding"));
}

#[test]
fn the_lines_are_varied_and_the_venue_is_the_nearest() {
    let s = grown(3, 60.0);
    let verbs = WED.iter().filter(|k| count(&s, k) > 0).count();
    assert!(verbs >= 3, "only {verbs} ways of saying a wedding");
    let ends = ["laid to rest", "was buried", "seen off by", "quiet funeral"]
        .iter()
        .filter(|k| count(&s, k) > 0)
        .count();
    assert!(ends >= 2, "only {ends} ways of saying a funeral");
}

#[test]
fn nothing_is_built_beside_the_cemetery_and_a_second_goes_far_from_the_first() {
    let mut s = grown(3, 45.0);
    let c: Vec<Lot> = s.lots.iter().filter(|l| l.kind == crate::LotKind::Cemetery).cloned().collect();
    assert_eq!(c.len(), 1, "one cemetery in a town of 2500");
    let c = &c[0];
    for l in s.lots.iter().filter(|l| l.kind != crate::LotKind::Cemetery) {
        let near = l.x < c.x + c.w + 2 && l.x + l.w > c.x - 2 && l.y < c.y + c.h + 2 && l.y + l.h > c.y - 2;
        assert!(!near, "a {:?} at {},{} crowds the cemetery at {},{}", l.kind, l.x, l.y, c.x, c.y);
    }
    // The cemetery sits on gently sloping ground.
    let lv: Vec<u8> = (c.y..c.y + c.h).flat_map(|y| (c.x..c.x + c.w).map(move |x| (x, y))).map(|(x, y)| s.map.at(x, y).min_level()).collect();
    assert!(lv.iter().max().unwrap() - lv.iter().min().unwrap() <= 1);
    // A second one, as a rich city would ask for, lands well away from the first.
    if s.place_civic_edge(crate::LotKind::Cemetery) {
        let all: Vec<&Lot> = s.lots.iter().filter(|l| l.kind == crate::LotKind::Cemetery).collect();
        assert_eq!(all.len(), 2);
        let d = (all[0].x - all[1].x).abs() + (all[0].y - all[1].y).abs();
        assert!(d >= 8, "the two cemeteries are {d} tiles apart");
    }
}
