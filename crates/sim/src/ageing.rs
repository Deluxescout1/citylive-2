//! B78 (D84): neighbourhoods that age and renew.
//!
//! the owner chose "some grit, then renewal": a two-year-old city must not look like a
//! two-month-old one, and it must never be bleak for long. Whole-district decline
//! stays an ending (P6), so this module is only ever *building by building*.
//!
//! Everything here is a **pure function of the lot's own age and the town's luck**
//! — its seed, the land's wetness, its district and the era — recomputed on a
//! coarse tick. That makes it deterministic per seed, idempotent across a
//! save/load, and correct after a bulk catch-up (a wallpaper that was off for a
//! week lands in exactly the state it would have been in live). The one thing
//! that is *not* a pure function is the pull-down of a boarded building, which
//! mutates the lot (a new seed, a new storey count, a fresh site): the rebuild
//! resets the lot's own `started`, so it cannot happen again for a long while.
//!
//! The rules, in the order a building lives them:
//!
//! * **Grime** — years since the last repaint. Streaked, dulled walls; moss and
//!   lichen on the roof in a wet land. New buildings are crisp.
//! * **Renovation** — every six-to-nine months an old building gets scaffolding
//!   and a crew for a few days, then comes back repainted (the paint scheme and,
//!   now and then, an extension come out of the sprite seed).
//! * **Street art** — spray tags on the blank walls of the busy old districts,
//!   and a painted mural on some of those walls years later.
//! * **Shops turn over** — a shop shuts for a fortnight with a TO LET board, then
//!   reopens under a new fascia.
//! * **Boarded and rebuilt** — rarely an old building is boarded up; within a
//!   bound it is pulled down and a new building goes up on the lot.
//! * **Never bleak** — caps per district on how much is run-down at once, and
//!   every run-down building is scheduled for renewal, not left.

use crate::{District, Lot, LotKind, Sim};

/// Days a building stays crisp after it is finished (or repainted).
pub const GRIME_DAYS: [f64; 3] = [20.0, 70.0, 160.0];
/// How long a renovation takes, start to finish: the scaffold is up for this
/// long, then the paint is new.
pub const RENO_DAYS: f64 = 5.0;
/// The first renovation of a building, at this age (days, plus a per-lot jitter).
pub const FIRST_RENO: f64 = 100.0;
/// The gap between renovations (days, plus a per-lot jitter).
pub const RENO_GAP: f64 = 150.0;
/// A boarded building is resolved (pulled down and rebuilt) within this many
/// days. The brief's bound; the test asserts it.
pub const BOARD_DAYS: f64 = 18.0;
/// A shop stays shut (shutters down, TO LET) for this long before it reopens.
pub const SHUT_DAYS: f64 = 14.0;
/// The most of a district that may be run-down at once (share of its finished
/// buildings). The rest are weathered but not derelict: "never bleak".
pub const RUN_DOWN_CAP: f32 = 0.18;
/// The most of a district that may be boarded up at once: one in this many.
pub const BOARDED_ONE_IN: usize = 25;

/// What one building wears, and what the street did to it. The renderer's copy
/// is `render::city::Wear`; `endfx::kinds` carries it across.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wear {
    pub grime: u8,
    pub wet: bool,
    pub art: u8,
    pub art_seed: u32,
    pub boarded: bool,
    pub shut: bool,
    pub scaffold: bool,
    pub repaint: u8,
    pub fascia: u8,
}

impl Wear {
    /// Nothing on show: a crisp, plain building.
    pub fn is_plain(&self) -> bool {
        *self == Wear::default()
    }
}

/// One lot's plan for this tick: what it wears, and whether the crew should be
/// pulling it down (a boarded building whose time is up).
#[derive(Clone, Copy, Debug, Default)]
pub struct Planned {
    pub wear: Wear,
    pub rebuild: bool,
}

#[inline]
fn hash(x: i32, y: i32, salt: u32) -> u32 {
    let mut h = (x as u32)
        .wrapping_mul(0x8da6_bb43)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ salt.wrapping_mul(0x27d4_eb2f);
    h ^= h >> 15;
    h = h.wrapping_mul(0xca5a_8265);
    h ^= h >> 13;
    h
}

/// Does this land keep its roofs damp? Moss and lichen grow where the air is wet.
pub fn wet_land(b: map::Biome) -> bool {
    matches!(
        b,
        map::Biome::Tropical
            | map::Biome::Swamp
            | map::Biome::Lagoon
            | map::Biome::Islands
            | map::Biome::Bamboo
            | map::Biome::Mediterranean
    )
}

/// Can this kind of building wear street art? Never a home's front door, never a
/// church, a hall, a park or a landmark (the brief's rule, and D59's: the civic
/// and the hand-made art stay out of it). Shops, warehouses and the other low
/// commercial walls: a mural goes where a wall reaches the pavement and can be
/// seen, not on the fortieth floor of a tower nobody can see past.
pub fn takes_art(k: LotKind) -> bool {
    // Only kinds the building kit actually draws: the car places are engine art
    // and never reach the wear pass, so art there would be a rule with no wall.
    matches!(
        k,
        LotKind::Shop | LotKind::Store | LotKind::Industry | LotKind::Office
    )
}

/// Does this lot have walls a crew can work on? (A park, a plaza or a car park
/// is open ground, however old it is.)
pub fn has_walls(k: LotKind) -> bool {
    can_board(k) || matches!(k, LotKind::Tower)
}

/// Can this kind of building be boarded up and rebuilt? The same guard, plus no
/// landmarks and no car places (they are engine art, not buildings).
pub fn can_board(k: LotKind) -> bool {
    matches!(
        k,
        LotKind::House
            | LotKind::LogCabin
            | LotKind::Townhouse
            | LotKind::Apartment
            | LotKind::Shop
            | LotKind::Store
            | LotKind::Office
            | LotKind::Industry
    )
}

/// How much a district looks after itself: a smart estate repaints sooner, and a
/// district on hard times a little later.
fn district_care(d: District) -> f64 {
    match d {
        District::HighEnd => 0.75,
        District::Core | District::European => 1.0,
        District::Downtown => 0.9,
        District::Suburb => 1.1,
        District::Industrial => 1.2,
    }
}

/// One lot's wear at `now`, before the district caps are applied. `mood` is the
/// town's fortunes: a bust leaves more streets shabby and puts the painters off;
/// a boom keeps the paint fresh ("driven by ... the town's fortunes").
pub fn plan(l: &Lot, now: f64, wet: bool, district: District, mood: f32) -> Planned {
    if !l.finished() || !l.started.is_finite() {
        return Planned::default();
    }
    let age = ((now - l.started) / 86_400.0).max(0.0);
    let h = hash(l.x, l.y, l.seed);
    // Hard times put the painters off; a boom brings them round sooner.
    let care = district_care(district) * (1.0 - 0.30 * mood.clamp(-1.0, 1.0) as f64);

    // --- renovations: a scaffold every six-to-nine months -------------------
    let first = (FIRST_RENO + (h % 61) as f64) * care;
    let gap = (RENO_GAP + ((h >> 6) % 111) as f64) * care;
    let renos = if age < first {
        0u8
    } else {
        (((age - first) / gap).floor() as u8).saturating_add(1).min(3)
    };
    let since = if renos == 0 {
        age
    } else {
        age - (first + (renos as f64 - 1.0) * gap)
    };
    // A wall low enough for scaffolding to be seen from the street gets the
    // full job (poles, boards, a crew); a tower is repainted from the inside,
    // which shows as the new design its seed asks for.
    // A wall the street can see: commercial and mid-rise, never a tower's
    // fortieth floor and never a home's front door.
    let low = l.storeys <= 8;
    let mid = l.storeys <= 8;
    let scaffold = renos > 0 && since < RENO_DAYS && mid && has_walls(l.kind);
    // The paint is the one the last *completed* renovation gave it.
    let repaint = if scaffold { renos.saturating_sub(1) } else { renos };
    let since_reno = if renos == 0 {
        age
    } else if scaffold {
        0.0
    } else {
        since - RENO_DAYS
    };

    // --- grime -------------------------------------------------------------
    let mut grime: u8 = match since_reno {
        d if d < GRIME_DAYS[0] => 0,
        d if d < GRIME_DAYS[1] => 1,
        d if d < GRIME_DAYS[2] => 2,
        _ => 3,
    };
    // A smart district keeps its paint; a hard-used one weathers a step sooner.
    if district == District::HighEnd {
        grime = grime.saturating_sub(1);
    }
    // Not every wall on a street ages at the same rate, and hard times leave a
    // few more of the older ones shabby — a step, not a district of ruins.
    if grime > 0 && h % 7 == 0 {
        grime = (grime + 1).min(3);
    }
    if mood < -0.4 && grime >= 2 && h % 3 == 0 {
        grime = 3;
    }

    // --- street art --------------------------------------------------------
    // Busy, or industrial: a warehouse wall is where the tags actually go.
    let busy = matches!(
        district,
        District::Core | District::Downtown | District::European | District::Industrial
    );
    // Only a wall that faces the street the camera sees: in this isometric view
    // the +x and +y faces are the ones in front, and a mural on a face the next
    // building hides is a mural nobody ever sees.
    let faces_us = l.front == 0 || l.front == 1;
    let (art, art_seed) = if takes_art(l.kind) && low && busy && faces_us && !scaffold {
        let tagged = age > 80.0 && (h >> 9) % 2 == 0;
        if tagged && age > 200.0 && (h >> 11) % 3 == 0 {
            // A tag that later got painted over properly: a mural.
            (2u8, h ^ 0x5bd1_e995)
        } else if tagged {
            (1, h ^ 0x1b87_3593)
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    // --- boarded up, then pulled down --------------------------------------
    let b0 = 60.0 + ((h >> 17) % 51) as f64;
    let bd = 12.0 + ((h >> 21) % 7) as f64;
    let doomed = can_board(l.kind) && !scaffold && (h >> 13) % 5 == 0;
    let boarded = doomed && mid && since >= b0 && since < b0 + bd;
    // The crew comes as soon as the boards have been up their time. The rebuild
    // resets `started`, so this cannot fire twice.
    let rebuild = doomed && mid && since >= b0 + bd;

    // --- a shop shuts and reopens as something else -------------------------
    let (shut, fascia) = if matches!(l.kind, LotKind::Shop | LotKind::Store) {
        let c0 = 60.0 + ((h >> 3) % 91) as f64;
        let cycle = 150.0 + ((h >> 7) % 110) as f64;
        let k = if age < c0 {
            0u8
        } else {
            (((age - c0) / cycle).floor() as u8).saturating_add(1).min(3)
        };
        let since_c = if k == 0 {
            age
        } else {
            age - (c0 + (k as f64 - 1.0) * cycle)
        };
        let shut = k > 0 && since_c < SHUT_DAYS;
        let fascia = if shut { k.saturating_sub(1) } else { k };
        (shut, fascia)
    } else {
        (false, 0)
    };

    Planned {
        wear: Wear {
            grime,
            wet,
            art,
            art_seed,
            boarded,
            shut,
            scaffold,
            repaint,
            fascia,
        },
        rebuild,
    }
}

impl Sim {
    /// B78: recompute every lot's wear, keep the districts honest, and send the
    /// crews to the buildings whose time is up. Runs at the decision cadence (an
    /// hour of sim time), which is 60 ticks a game day: fine enough that a
    /// scaffold appears the hour it should, cheap enough to ignore.
    ///
    /// Returns the lots whose *visible* wear changed (the caller marks the tiles
    /// dirty, so the renderer repaints those buildings and nothing else).
    pub fn ageing_step(&mut self) {
        let n = self.lots.len();
        if n == 0 {
            if !self.wear.is_empty() {
                self.wear.clear();
                self.wear_keys.clear();
            }
            return;
        }
        // The districts are what make a high-end street and an industrial one
        // behave differently; a loaded city has not decided anything yet, so
        // make sure they exist before asking which district a lot is in.
        let _ = self.district_seeds();
        let wet = wet_land(self.map.biome);
        let now = self.time;
        let mut plans: Vec<Planned> = Vec::with_capacity(n);
        let mood = self.civics.mood;
        for l in &self.lots {
            let d = self.district_at(l.x, l.y);
            plans.push(plan(l, now, wet, d, mood));
        }

        // --- never bleak: the caps, per district ---------------------------
        // Districts are few, so a small fixed table (index by `District as u8`
        // order in `DISTRICTS`) costs nothing.
        let mut by_district: Vec<Vec<usize>> = vec![Vec::new(); 6];
        for (i, l) in self.lots.iter().enumerate() {
            if l.finished() {
                let d = self.district_at(l.x, l.y);
                by_district[district_index(d)].push(i);
            }
        }
        for list in by_district.iter() {
            if list.is_empty() {
                continue;
            }
            // Run-down: keep the oldest (lowest hash) up to the cap; the rest are
            // weathered, not derelict.
            let mut run_down: Vec<usize> = list
                .iter()
                .copied()
                .filter(|&i| plans[i].wear.grime >= 3)
                .collect();
            let cap = ((list.len() as f32 * RUN_DOWN_CAP).ceil() as usize).max(1);
            if run_down.len() > cap {
                run_down.sort_by_key(|&i| {
                    let l = &self.lots[i];
                    (hash(l.x, l.y, l.seed ^ 0x9e37_79b9), i)
                });
                for &i in &run_down[cap..] {
                    plans[i].wear.grime = 2;
                }
            }
            // Boarded: one in 25 at most, and at least one is allowed so a small
            // district can show a single boarded house.
            let bcap = (list.len() / BOARDED_ONE_IN).max(1);
            let mut boarded: Vec<usize> = list
                .iter()
                .copied()
                .filter(|&i| plans[i].wear.boarded && !plans[i].rebuild)
                .collect();
            if boarded.len() > bcap {
                // Keep the ones that have been boarded longest: a building due
                // for its rebuild is never held back (the bound comes first).
                boarded.sort_by_key(|&i| {
                    let l = &self.lots[i];
                    (hash(l.x, l.y, l.seed ^ 0x51ed_2701), i)
                });
                for &i in &boarded[bcap..] {
                    plans[i].wear.boarded = false;
                }
            }
        }

        // --- the pull-downs ------------------------------------------------
        let mut rebuilds: Vec<usize> = Vec::new();
        for (i, p) in plans.iter().enumerate() {
            if p.rebuild {
                rebuilds.push(i);
            }
        }
        if !rebuilds.is_empty() {
            let era = self.era();
            for i in rebuilds {
                // A boarded building's crew takes it down and puts a new one up:
                // a fresh seed (a new design), a little taller if the era has
                // moved on, and a site the site-crews work like any other.
                let l = &mut self.lots[i];
                let h = hash(l.x, l.y, l.seed ^ 0x0bad_f00d);
                l.seed = l
                    .seed
                    .wrapping_mul(1_664_525)
                    .wrapping_add(1_013_904_223)
                    ^ (h & 0x7fff_ffff);
                let lift = match era {
                    crate::Era::City | crate::Era::Metropolis => (h >> 3) % 3,
                    crate::Era::Town => (h >> 3) % 2,
                    _ => 0,
                };
                l.storeys = (l.storeys + lift as u8).max(1);
                l.stage = crate::Stage::Staked;
                l.stage_end = now + crate::HOUSE_SECONDS / 6.0;
                l.started = now;
                plans[i] = Planned::default();
            }
            self.events.push(crate::Event {
                time: now,
                text: "a boarded building comes down, and a new one goes up".to_string(),
            });
        }

        // --- publish, and tell the caller what moved ------------------------
        let mut changed = Vec::new();
        if self.wear.len() == n {
            for (i, p) in plans.iter().enumerate() {
                if self.wear[i] != p.wear {
                    changed.push(i);
                }
            }
        } else {
            changed.extend(0..n);
        }
        self.wear = plans.into_iter().map(|p| p.wear).collect();
        self.wear_keys = self.lots.iter().map(|l| (l.x, l.y, l.seed)).collect();
        for i in changed {
            let (x, y, w, h) = {
                let l = &self.lots[i];
                (l.x, l.y, l.w, l.h)
            };
            for ty in y..y + h {
                for tx in x..x + w {
                    self.changes.insert((tx, ty));
                }
            }
        }
    }

    /// The wear of every lot, in lot order (empty until the first ageing tick).
    /// Between two ticks the lot list may have changed (a lot cleared for a
    /// street shifts every later lot down): use [`Self::lot_wear`] to read one.
    pub fn wear(&self) -> &[Wear] {
        &self.wear
    }

    /// The wear worked out for lot `i`, or `None` when the tick has not seen
    /// that lot yet (a new or rebuilt one: it is plain new) or the list has
    /// shifted since the tick and this slot now holds a different lot.
    pub fn lot_wear(&self, i: usize) -> Option<&Wear> {
        let l = self.lots.get(i)?;
        if self.wear_keys.get(i) == Some(&(l.x, l.y, l.seed)) {
            self.wear.get(i)
        } else {
            None
        }
    }
}

fn district_index(d: District) -> usize {
    match d {
        District::Core => 0,
        District::European => 1,
        District::Suburb => 2,
        District::Downtown => 3,
        District::HighEnd => 4,
        District::Industrial => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(kind: LotKind, started: f64, x: i32, y: i32, seed: u32) -> Lot {
        Lot {
            x,
            y,
            w: 2,
            h: 2,
            level: 2,
            kind,
            storeys: 2,
            style: 0,
            seed,
            front: 1,
            stage: crate::Stage::Finished,
            stage_end: started,
            started,
        }
    }

    #[test]
    fn a_new_building_is_crisp_and_an_old_one_is_not() {
        let fresh = plan(&lot(LotKind::House, 1_000.0, 10, 10, 7), 1_000.0 + 86400.0, false, District::Core, 0.0);
        assert_eq!(fresh.wear.grime, 0);
        assert!(!fresh.wear.scaffold && fresh.wear.art == 0 && !fresh.wear.boarded);

        let old = plan(&lot(LotKind::House, 0.0, 10, 10, 7), 180.0 * 86400.0, false, District::Core, 0.0);
        assert!(old.wear.grime >= 2, "a six-month-old house is weathered: {:?}", old.wear);
    }

    #[test]
    fn art_never_lands_on_a_home_or_a_landmark() {
        for seed in 0..400u32 {
            let now = 400.0 * 86400.0;
            for kind in [LotKind::House, LotKind::Church, LotKind::Hall, LotKind::Park] {
                let p = plan(&lot(kind, 0.0, 20, 30, seed), now, false, District::Downtown, 0.0);
                assert_eq!(p.wear.art, 0, "{kind:?} must not wear street art");
            }
            for k in crate::landmarks::LandmarkKind::ALL {
                let p = plan(&lot(LotKind::Landmark(k), 0.0, 20, 30, seed), now, false, District::Downtown, 0.0);
                assert_eq!(p.wear.art, 0);
                assert!(!p.wear.boarded);
            }
        }
    }

    #[test]
    fn a_shop_shuts_and_reopens_under_a_new_fascia() {
        // Find a seed that shuts within a few hundred days, then check the state
        // either side of it: closed with the boards down, then open again with a
        // higher fascia.
        let mut seen_shut = false;
        let mut reopened = false;
        for seed in 0..200u32 {
            let l = lot(LotKind::Shop, 0.0, 12, 14, seed);
            let mut was_shut = false;
            for day in 0..400 {
                let p = plan(&l, day as f64 * 86400.0, false, District::Core, 0.0);
                if p.wear.shut {
                    was_shut = true;
                    seen_shut = true;
                } else if was_shut && p.wear.fascia > 0 {
                    reopened = true;
                }
            }
            if seen_shut && reopened {
                break;
            }
        }
        assert!(seen_shut, "some shop shuts its shutters");
        assert!(reopened, "and opens again under a new fascia");
    }

    #[test]
    fn a_boarded_building_is_told_to_rebuild_after_the_bound() {
        let mut boarding: Option<(f64, u32)> = None;
        let mut resolved = false;
        for seed in 0..300u32 {
            let l = lot(LotKind::House, 0.0, 30, 31, seed);
            boarding = None;
            for day in 0..400 {
                let p = plan(&l, day as f64 * 86400.0, false, District::Core, 0.0);
                if p.wear.boarded && boarding.is_none() {
                    boarding = Some((day as f64, seed));
                }
                if p.rebuild && boarding.is_some() {
                    let (start, _) = boarding.unwrap();
                    assert!(
                        day as f64 - start <= BOARD_DAYS + 1.0,
                        "boarded on day {start}, rebuilt on day {day}"
                    );
                    resolved = true;
                    break;
                }
            }
            if resolved {
                break;
            }
        }
        assert!(resolved, "a boarded building is always resolved");
    }

    #[test]
    fn a_wet_land_grows_moss() {
        let l = lot(LotKind::House, 0.0, 10, 10, 3);
        let dry = plan(&l, 200.0 * 86400.0, false, District::Core, 0.0);
        let wet = plan(&l, 200.0 * 86400.0, true, District::Core, 0.0);
        assert!(!dry.wear.wet && wet.wear.wet);
        assert!(wet_land(map::Biome::Tropical) && !wet_land(map::Biome::Desert));
    }

    #[test]
    fn a_lot_removed_between_ticks_does_not_lend_its_wear_to_the_next() {
        let mut s = crate::Sim::new(5, crate::Pace::Month, 107, 107);
        s.run_to(12.0 * 86400.0);
        s.ageing_step();
        assert!(s.lots.len() > 5);
        let n = s.lots.len();
        assert!((0..n).all(|i| s.lot_wear(i).is_some()), "every lot has its wear after a tick");
        // A lot cleared for a street: every later lot moves down one place.
        s.lots.remove(1);
        for i in 1..s.lots.len() {
            assert!(s.lot_wear(i).is_none(), "slot {i} now holds another lot: no borrowed wear");
        }
        assert!(s.lot_wear(0).is_some(), "the lots before the gap keep theirs");
        s.ageing_step();
        assert!((0..s.lots.len()).all(|i| s.lot_wear(i).is_some()), "the next tick catches up");
    }
}
