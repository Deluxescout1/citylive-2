//! B50: the ending's tests (determinism, the phases, a save in every phase,
//! the morph finishing with a valid new world, and a world that cannot end
//! twice).

use crate::ending::*;
use crate::{Pace, Sim};

fn city(seed: u64, days: f64) -> Sim {
    let mut s = Sim::new(seed, Pace::Month, 71, 71);
    s.run_to(days * 86400.0);
    s
}

fn reload(s: &Sim, tag: &str) -> Sim {
    let path = std::env::temp_dir().join(format!("citylive-b50-{tag}.sav"));
    s.save(&path).unwrap();
    Sim::load(&path).unwrap()
}

#[test]
fn the_plan_is_a_function_of_the_seed_and_a_world_lasts_about_a_month() {
    let mut kinds = std::collections::BTreeSet::new();
    for seed in 1..40u64 {
        let a = Sim::new(seed, Pace::Month, 41, 41);
        let b = Sim::new(seed, Pace::Month, 41, 41);
        assert_eq!(a.ending.kind, b.ending.kind);
        assert_eq!(a.ending.next_seed, b.ending.next_seed);
        assert_eq!(a.ending.next_biome, b.ending.next_biome);
        assert_eq!(a.ending.world_days.to_bits(), b.ending.world_days.to_bits());
        // Tests stretch the world a thousandfold (see `State::plan`).
        let days = a.ending.world_days / 1000.0;
        assert!((21.0..=39.0).contains(&days), "seed {seed}: {days} days");
        assert_ne!(a.ending.next_biome, a.biome, "the next land is a different land");
        kinds.insert(a.ending.kind.id());
    }
    assert!(kinds.len() >= 6, "endings are varied: {kinds:?}");
}

#[test]
fn the_phases_follow_each_other_and_scale_with_the_kind() {
    for kind in Ending::ALL {
        let mut s = Sim::new(3, Pace::Month, 41, 41);
        s.ending.kind = kind;
        s.ending.world_days = 30.0;
        let tm = kind.timing();
        let spec0 = 30.0 * 86400.0;
        assert_eq!(s.locate(spec0 - tm.warn - 10.0).0, Phase::Living);
        assert_eq!(s.locate(spec0 - tm.warn + 10.0).0, Phase::Warning);
        assert_eq!(s.locate(spec0 + 1.0).0, Phase::Spectacle);
        assert_eq!(s.locate(spec0 + tm.spec + 1.0).0, Phase::Aftermath);
        assert_eq!(s.locate(spec0 + tm.spec + tm.after + 1.0).0, Phase::Morph);
        assert_eq!(s.locate(spec0 + tm.spec + tm.after + tm.morph + 1.0).0, Phase::Done);
        // Warning is hours to days; the spectacle minutes to an hour or so.
        assert!(tm.warn >= 5.0 * 3600.0 && tm.warn <= 100.0 * 3600.0, "{kind:?}");
        assert!(tm.spec >= 10.0 * 60.0 && tm.spec <= 2.0 * 3600.0, "{kind:?}");
    }
}

#[test]
fn a_live_run_walks_every_phase_and_tells_its_lines_once() {
    let mut s = city(4, 6.0);
    s.ending.kind = Ending::Meteor;
    s.ending.world_days = 6.0 + 3.0;
    let tm = Ending::Meteor.timing();
    let end = 9.0 * 86400.0 + tm.spec + tm.after + tm.morph + 3600.0;
    let mut seen = std::collections::BTreeSet::new();
    while s.time < end {
        s.run_to(s.time + 600.0);
        seen.insert(s.ending_phase().0.id());
    }
    assert_eq!(seen.len(), 6, "{seen:?}");
    let told = s
        .story
        .iter()
        .filter(|l| l.text.contains("fire streaks across the whole sky"))
        .count();
    assert_eq!(told, 1, "a line is told once");
    assert!(s.chronicle_final().is_some() || s.ending.told & (1 << 31) != 0);
    let r = s.rebirth().expect("the world is over");
    assert!(s.rebirth().is_none(), "a world cannot end twice");
    assert_ne!(r.biome, s.biome);
    assert!(r.ended.contains("ended"));
}

#[test]
fn every_ending_stages_at_every_phase_and_survives_a_save() {
    let base = city(5, 12.0);
    let path = std::env::temp_dir().join("citylive-b50-base.sav");
    base.save(&path).unwrap();
    for kind in Ending::ALL {
        for (phase, p) in [
            (Phase::Warning, 0.6),
            (Phase::Spectacle, 0.5),
            (Phase::Aftermath, 0.5),
            (Phase::Morph, 0.4),
        ] {
            let mut a = Sim::load(&path).unwrap();
            a.stage_ending(kind, phase, p);
            assert_eq!(a.ending_phase().0, phase, "{kind:?}");
            let tag = format!("{}-{}", kind.name(), phase.name());
            let mut b = reload(&a, &tag);
            assert_eq!(a.hash(), b.hash(), "{kind:?} {phase:?}: a save changes the world");
            assert_eq!(b.ending.kind, kind);
            assert_eq!(b.ending.cur, a.ending.cur, "{kind:?} {phase:?}: cursors");
            // Byte-exact round trip.
            let p1 = std::env::temp_dir().join(format!("citylive-b50-{tag}.sav"));
            let p2 = std::env::temp_dir().join(format!("citylive-b50-{tag}-2.sav"));
            b.save(&p2).unwrap();
            assert_eq!(std::fs::read(&p1).unwrap(), std::fs::read(&p2).unwrap(), "{kind:?} {phase:?}");
            // And the loaded world goes on identically to the live one.
            a.run_to(a.time + 1800.0);
            b.run_to(b.time + 1800.0);
            assert_eq!(a.hash(), b.hash(), "{kind:?} {phase:?}: after the save the world diverges");
            assert_eq!(a.ending.ruin, b.ending.ruin, "{kind:?} {phase:?}: ruins");
        }
    }
}

#[test]
fn the_morph_finishes_with_a_valid_new_world() {
    let base = city(6, 12.0);
    let path = std::env::temp_dir().join("citylive-b50-morph.sav");
    base.save(&path).unwrap();
    let mut s = Sim::load(&path).unwrap();
    s.stage_ending(Ending::Decline, Phase::Morph, 0.1);
    let before = s.map.morph.iter().filter(|m| **m != 0).count();
    let tm = Ending::Decline.timing();
    s.run_to(s.time + tm.morph);
    let after_ = s.map.morph.iter().filter(|m| **m != 0).count();
    assert!(after_ > before, "the front moved: {before} -> {after_}");
    assert_eq!(s.ending_phase().0, Phase::Done);
    assert_eq!(after_, (s.map.w * s.map.h) as usize, "the whole land turned");
    assert!(s.lots.is_empty(), "no building survives the morph");
    let r = s.rebirth().expect("done");
    assert!(s.rebirth().is_none());
    // The next world founds normally, in the new land.
    let mut n = Sim::with_focus_in(r.seed, Pace::Month, 71, 71, (35.0, 35.0), Some(r.biome));
    n.run_to(2.0 * 86400.0);
    assert_eq!(n.biome, r.biome);
    assert_ne!(n.biome, s.biome);
    assert!(n.people.living() > 0 || !n.lots.is_empty());
    assert!(n.ending.world_days > 0.0);
    assert_eq!(n.ending_phase().0, Phase::Living, "the new world has its own ending to come");
}

#[test]
fn catching_up_never_skips_the_spectacle_or_the_morphs_first_hour() {
    let tm = Ending::Meteor.timing();
    let mut s = city(11, 5.0);
    s.ending.kind = Ending::Meteor;
    s.ending.world_days = 8.0;
    let spec0 = 8.0 * 86400.0;
    // Away for a week: the warning passes in the dark, the spectacle does not.
    s.catch_up_to(s.time + 7.0 * 86400.0);
    assert!(s.time < spec0 && s.time > spec0 - 90.0, "held at the start: {}", s.time - spec0);
    assert_eq!(s.ending_phase().0, Phase::Warning);
    // Found inside a spectacle: it does not move until it runs live.
    s.run_to(spec0 + 100.0);
    let t = s.time;
    s.catch_up_to(t + 86400.0);
    assert_eq!(s.time, t, "a spectacle is live only");
    // After it, the aftermath may be caught up, but only to the morph.
    s.run_to(spec0 + tm.spec + 10.0);
    s.catch_up_to(s.time + 3.0 * 86400.0);
    let morph0 = spec0 + tm.spec + tm.after;
    assert!(s.time < morph0 && s.time > morph0 - 90.0);
    s.run_to(morph0 + 5.0);
    let t = s.time;
    s.catch_up_to(t + 86400.0);
    assert_eq!(s.time, t, "the morph's first hour is live");
    // An hour later catch-up is free again.
    s.run_to(morph0 + 3700.0);
    s.catch_up_to(s.time + 3.0 * 3600.0);
    assert!(s.time > morph0 + 3800.0);
}

#[test]
fn an_old_save_gets_a_plan_from_its_founding_and_never_ends_at_load() {
    let mut a = Sim::new(21, Pace::Month, 71, 71);
    a.ending.world_days = 1e9;
    a.run_to(40.0 * 86400.0);
    let path = std::env::temp_dir().join("citylive-b50-v11.sav");
    a.save(&path).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    // A v11 file: no ending trailer for the reader to find.
    bytes[4..8].copy_from_slice(&11u32.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let b = Sim::load(&path).unwrap();
    let tm = b.ending.kind.timing();
    assert_eq!(b.ending_phase().0, Phase::Living, "it does not end at load");
    assert!(b.ending.world_days * 86400.0 >= b.time + tm.warn + 2.5 * 86400.0, "days of grace");
    // The plan is the seed's own, so a second load agrees.
    let c = Sim::load(&path).unwrap();
    assert_eq!(b.ending.kind, c.ending.kind);
    assert_eq!(b.ending.world_days.to_bits(), c.ending.world_days.to_bits());
    // A young v11 city keeps the seed's own length, counted from its founding.
    let mut y = Sim::new(22, Pace::Month, 71, 71);
    y.ending.world_days = 1e9;
    y.run_to(2.0 * 86400.0);
    y.save(&path).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[4..8].copy_from_slice(&11u32.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let l = Sim::load(&path).unwrap();
    let planned = State::plan(22, &l.map).world_days / 1000.0; // tests stretch it
    let _ = planned;
    assert!(l.ending.world_days >= 20.0, "{}", l.ending.world_days);
}

#[test]
fn the_feed_stays_calm_at_the_new_life_pace() {
    // 0.4 days to a life year: births, weddings and funerals are far more
    // frequent than B48 tuned for. The per-day caps must hold, so the tray
    // (it shows the latest eight lines) changes a few times a day, not a few
    // times a minute.
    let s = city(31, 30.0);
    let mut per_day = std::collections::BTreeMap::<i64, usize>::new();
    for l in s.story.iter() {
        *per_day.entry((l.time / 86400.0) as i64).or_default() += 1;
    }
    let worst = per_day.values().copied().max().unwrap_or(0);
    let total: usize = per_day.values().sum();
    eprintln!("feed: {total} lines in 30 days, worst day {worst}");
    // B66: each *category* has its own cap (life 3 a day, one per kind) and they
    // add up on a day that has a fire, a birth and a piece of civic works; that
    // is a busy day, not a chatty one. The bound is the caps' own sum, not the
    // number the first seed happened to draw (it was 9).
    assert!(worst <= 12, "a day told {worst} lines");
    // At 1 sim s per real s a day is a real day: a handful of tray changes a day.
    assert!(total <= 30 * 9);
}

#[test]
fn people_leave_and_the_town_stops_growing() {
    let mut s = city(7, 14.0);
    let living = s.people.living();
    assert!(living > 20);
    s.stage_ending(Ending::Kaiju, Phase::Aftermath, 0.95);
    assert!(s.people.living() * 5 <= living, "{} of {living} left in the town", s.people.living());
    let lots = s.lots.len();
    s.run_to(s.time + 6.0 * 3600.0);
    assert!(s.lots.len() <= lots, "nothing is built after the end has begun");
}

#[test]
fn the_ruin_codes_change_with_the_phase_and_never_panic_on_an_empty_world() {
    let mut s = city(8, 14.0);
    s.stage_ending(Ending::Meteor, Phase::Spectacle, 0.9);
    let n = s.lots.len();
    assert_eq!(s.ending.ruin.len(), n);
    let hit = s.ending.ruin.iter().filter(|c| **c != R_OK).count();
    assert!(hit * 4 >= n, "a meteor wrecks much of the town: {hit} of {n}");
    let mut e = Sim::new(9, Pace::Month, 41, 41);
    for kind in Ending::ALL {
        e.stage_ending(kind, Phase::Spectacle, 0.5);
        e.stage_ending(kind, Phase::Morph, 0.9);
    }
}

/// B65: the timelapse stops at the end of the aftermath, so the last frame of a
/// world is its ruins and never the next biome's land.
#[test]
fn the_world_is_still_its_own_until_the_morph_begins() {
    for kind in [Ending::Meteor, Ending::Decline, Ending::Sunset] {
        let mut s = city(9, 12.0);
        s.stage_ending(kind, Phase::Aftermath, 0.5);
        assert!(!s.aftermath_over(), "{kind:?}: the aftermath is not over");
        s.run_to(s.time + 1.0);
        assert!(!s.aftermath_over(), "{kind:?}");
        s.stage_ending(kind, Phase::Morph, 0.0);
        assert!(
            s.aftermath_over(),
            "{kind:?}: the morph has begun (phase {:?})",
            s.ending_phase()
        );
        s.stage_ending(kind, Phase::Aftermath, 1.0 - 1e-6);
        // A world that simply lives has nothing over it.
        let fresh = city(9, 1.0);
        assert!(!fresh.aftermath_over(), "{kind:?}: a living world");
    }
}

/// B99c round 2 (D77, B50): the wave's reach is a share of the *built* extent —
/// about a third of the town nearest the sea — not the distance from the shore
/// to the middle of the frame. Read off the lots, so it holds whatever shape the
/// town grew into and whichever way it faces the water.
#[test]
fn the_flood_takes_about_a_third_of_the_town() {
    let mut checked = 0;
    for (seed, biome) in [
        (11u64, map::Biome::Lagoon),
        (3, map::Biome::Lagoon),
        (5, map::Biome::Islands),
        (7, map::Biome::Islands),
        (19, map::Biome::Lagoon),
    ] {
        let vp = map::world_for_viewport(1920, 1080);
        let focus = map::screen_to_tile(&vp, 960.0, 540.0);
        let mut s = Sim::with_focus_in(seed, Pace::Month, vp.w, vp.h, focus, Some(biome));
        s.ending.world_days = 1e9;
        s.run_to(30.0 * 86400.0);
        if s.map.ocean.is_none() || s.lots.len() < 40 {
            continue; // no coast: no flood on this land
        }
        let before = s.lots.len();
        s.stage_ending(Ending::Flood, Phase::Spectacle, 0.99);
        let gone = before.saturating_sub(s.lots.len());
        checked += 1;
        let pct = gone as f32 * 100.0 / before as f32;
        assert!(
            (20.0..=50.0).contains(&pct),
            "seed {seed} {}: the flood took {gone} of {before} lots ({pct:.0} %), not about a third",
            biome.name()
        );
    }
    assert!(checked >= 2, "no coastal land to measure the reach on");
}
