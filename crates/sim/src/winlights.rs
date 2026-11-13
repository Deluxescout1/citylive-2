//! B72 (D78): who is behind each window.
//!
//! Every building's windows are grouped into 64 slots; a slot is a light — a
//! room, a floor, a cleaner's corridor. This module answers, for one hour of
//! one day, which slots of which lot are lit, from the people the sim already
//! has: their home door, their job door, their age, their trade, and the stable
//! per-person phase that staggers every trade's hours.
//!
//! The rules are the real world's, not a curve: a home is lit while someone who
//! lives there is in and awake (and each person keeps their own bedtime, so the
//! lights go out one at a time); an office while its staff are at their desks,
//! with a cleaner's floor and a couple of late workers after that; a shop while
//! it is open; a works while a shift is on; a civic building by its caretaker.
//! Nobody home, nothing lit — an empty building is dark.
//!
//! All of it is a pure function of the sim's state and the hour: the same save
//! at the same hour gives the same windows.

use crate::people::{schedule_at, Act, AgeGroup};
use crate::{Lot, LotKind, Sim};
use std::collections::HashMap;

/// Light slots per building: one bit each in the plan's `u64`.
pub const SLOTS: u32 = 64;

/// The hour's own flags, so the plan is one function of (state, clock).
#[derive(Clone, Copy)]
pub struct At {
    pub hour: f64,
    pub weekend: bool,
}

impl At {
    pub fn new(hour: f64, weekend: bool) -> At {
        At {
            hour: hour.rem_euclid(24.0),
            weekend,
        }
    }
}

/// One slot's clock: the hour with its own quarter of an hour of offset. Two windows of
/// the same building never switch in the same minute, which is what keeps a
/// facade from flipping in one frame when a shift ends or a household goes to
/// bed.
fn slot_hour(at: At, lot_seed: u32, j: u32) -> f64 {
    let jitter = (hash(lot_seed as u64 ^ 0x5A5, j as u64) - 0.5) * 0.5;
    (at.hour + jitter as f64).rem_euclid(24.0)
}

/// Stable 0..1 for (lot, slot): the same window is the same room for ever.
fn hash(a: u64, b: u64) -> f32 {
    let mut x = a ^ b.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
    x ^= x >> 13;
    (x & 0xFFFF) as f32 / 65535.0
}

/// The per-person phase the people layer itself uses (B46c): the same stagger,
/// so a household's lights agree with the walkers on its pavement.
fn phase_of(id: usize) -> f32 {
    ((id as u64).wrapping_mul(2_654_435_761) >> 9) as f32 % 100.0 / 100.0
}

/// When this person goes to bed and gets up: their own, from their phase. The
/// walker's `schedule_at` turns everyone in at half ten; a town's windows do
/// not, and that is the whole point of D78.
fn bed_hours(kind: AgeGroup, phase: f32) -> (f64, f64) {
    match kind {
        AgeGroup::Child => (19.1 + 1.9 * phase as f64, 6.3 + 1.1 * phase as f64),
        AgeGroup::Elder => (20.6 + 1.8 * phase as f64, 5.2 + 1.1 * phase as f64),
        AgeGroup::Adult => (21.7 + 3.2 * phase as f64, 5.5 + 1.6 * phase as f64),
    }
}

/// In bed at `hour`? (A bedtime past midnight wraps.)
fn in_bed(kind: AgeGroup, phase: f32, hour: f64) -> bool {
    let (bed, wake) = bed_hours(kind, phase);
    if bed < 24.0 {
        if hour >= bed {
            return true;
        }
    } else if hour >= bed - 24.0 && hour < wake {
        // 24.x is only reachable when the phase pushes past midnight.
        return hour >= bed - 24.0;
    }
    hour < wake
}

/// Is this person in the house at `at`? The walkers' own `Act` is about being
/// *visible on the street* — an adult's evening is "Leisure" for three hours
/// whether they are in the park or on the sofa — so a home's light uses the same
/// schedule with a person's own homecoming and habits on top: the town fills from
/// about a quarter to six, each house at its own time, and by nine nearly
/// everyone is in.
fn resident_home(kind: AgeGroup, job: crate::people::JobKind, at: At, phase: f32, id: usize) -> bool {
    let h = at.hour;
    let act = schedule_at(kind, job, h, at.weekend, phase);
    let out = match act {
        // In bed, in the house, or in for lunch.
        Act::Asleep | Act::Home | Act::Lunch => false,
        // On the way, at work, in class.
        Act::Commute { .. } | Act::Work | Act::School => true,
        // Out in the afternoon, home from this person's own hour — which is the
        // staggered "windows come on one by one" the brief asks for. Noon and
        // evening are the same rule, so no hour of the clock flips a whole
        // street at once.
        Act::Leisure => {
            // Home between a quarter past five and a quarter to seven: by
            // half past seven the street is in. (B72 round 2: the old window ran
            // to half past eight, so a fifth of the town stood dark at 19:30.)
            let home_at = 17.15 + 1.55 * phase as f64;
            // A few go out again after dinner — the pub, the cinema, a late
            // shift — and their rooms go dark while the rest of the house stays
            // lit. That is the last of the evening's switching.
            if h >= 19.9 && h < 19.9 + 1.9 * phase as f64 && hash(id as u64, 7) < 0.14 {
                return false;
            }
            if h >= 17.0 {
                h < home_at
            } else if h < 9.0 {
                true
            } else {
                // Working hours on a day off: some are out, some are in, and it
                // is the same people all day.
                hash(id as u64, 0x1D) < 0.55
            }
        }
    };
    !out
}

/// Would this resident's room be lit: are they in the house, and up?
fn resident_lit(kind: AgeGroup, job: crate::people::JobKind, at: At, phase: f32, id: usize) -> bool {
    if !resident_home(kind, job, at, phase, id) {
        return false;
    }
    !in_bed(kind, phase, at.hour)
}

/// One household's contribution: the rooms of a home.
///
/// Slot `j` is the room of resident `j % n`, so a household's rooms follow the
/// people who live in them: the children's rooms go dark at eight, the adults'
/// later, and one in five rooms is not in use at all. A house nobody is up in is
/// dark; a house with someone up is never *all* lit.
fn home_slots(
    residents: &[(AgeGroup, crate::people::JobKind, usize)],
    at: At,
    lot_seed: u32,
    storeys: u8,
) -> u64 {
    let n = residents.len();
    if n == 0 {
        return 0;
    }
    let mut bits = 0u64;
    let mut any = false;
    // A block of flats lights floor by floor, like the offices do: a 20-storey
    // apartment is not a checkerboard of single rooms.
    let floors = storeys.max(1) as u32;
    let per_floor = (SLOTS / floors).max(1);
    for j in 0..SLOTS {
        let who = (j as usize) % n;
        if storeys >= 4 && hash(lot_seed as u64 ^ 0xF11, (j / per_floor) as u64) < 0.25 {
            continue;
        }
        let (kind, job, id) = residents[who];
        let phase = phase_of(id);
        let slot = At {
            hour: slot_hour(at, lot_seed, j),
            weekend: at.weekend,
        };
        if !resident_lit(kind, job, slot, phase, id) {
            continue;
        }
        // A person lights the room they are in, not every window they own: two
        // rooms in three stay dark, so a home shows **one to three** lit rooms
        // (B72 round 2) instead of every pane it has.
        if hash(lot_seed as u64 ^ 0x51, j as u64) < 0.66 {
            continue;
        }
        bits |= 1 << j;
        any = true;
    }
    // The bathroom at three in the morning: one slot, in one house in eight —
    // the little "someone is up" the eye catches at night. (Every house at once
    // would read as the whole street being awake.)
    if n >= 2 && (2.2..3.6).contains(&at.hour) && hash(lot_seed as u64, 0x3A3) < 0.13 {
        let j = (hash(lot_seed as u64, 0xB47) * SLOTS as f32) as u32 % SLOTS;
        bits |= 1 << j;
        any = true;
    }
    // A home with somebody up is never completely dark: one room is always in
    // use (the kitchen, the hall).
    if !any {
        for j in 0..SLOTS {
            let who = (j as usize) % n;
            let (kind, job, id) = residents[who];
            let slot = At {
                hour: slot_hour(at, lot_seed, j),
                weekend: at.weekend,
            };
            if resident_lit(kind, job, slot, phase_of(id), id) {
                bits |= 1 << j;
                any = true;
                break;
            }
        }
    }
    if any {
        // The street-facing room: the first window of the first face is the one
        // a small house shows, so a home with somebody up always glows. (A
        // two-pane cottage would otherwise be dark half the evening.)
        bits |= 1;
        bits
    } else {
        0
    }
}

/// An office or a tower: staff at their desks by day, a cleaner's floor and a
/// couple of late workers after.
///
/// Offices let whole floors, so a floor answers first — in use or not — and the
/// rooms inside it are only part lit. That is what a tower looks like at night:
/// blocks of light, not a checkerboard of single panes.
fn office_slots(
    staff: &[(crate::people::JobKind, usize)],
    at: At,
    lot_seed: u32,
    storeys: u8,
) -> u64 {
    let n = staff.len();
    let floors = storeys.max(1) as u32;
    let per_floor = (SLOTS / floors).max(1);
    let mut bits = 0u64;
    for j in 0..SLOTS {
        let floor_of = j / per_floor;
        let at = At {
            hour: slot_hour(at, lot_seed, j),
            weekend: at.weekend,
        };
        // The floor's own state, before any person. Lower floors are let, the
        // crown is emptier (B72 round 2, the supervisor's taste): a tower reads
        // brighter at the base of the shaft.
        let rel = floor_of as f32 / floors.max(1) as f32;
        let let_p = 0.80 - 0.45 * rel;
        let floor_let = hash(lot_seed as u64 ^ 0xF10, floor_of as u64) < let_p;
        // The cleaners come in after the last desk has gone — at their own hour
        // per floor, so a tower's corridors light one floor at a time.
        let h24 = if at.hour < 12.0 { at.hour + 24.0 } else { at.hour };
        let clean_start = 22.2 + 1.6 * hash(lot_seed as u64 ^ 0xABC, floor_of as u64) as f64;
        let cleaner_floor = h24 >= clean_start && h24 < clean_start + 4.2;
        // One floor is being cleaned tonight, not the whole building.
        let clean_this = (hash(lot_seed as u64 ^ 0x77, 3) * floors as f32) as u32 == floor_of;
        if n == 0 {
            // A building nobody works in still has a caretaker's corridor.
            if cleaner_floor && clean_this && hash(lot_seed as u64 ^ 0x79, j as u64) < 0.42 {
                bits |= 1 << j;
            }
            continue;
        }
        let pick = (hash(lot_seed as u64 ^ 0x0F, j as u64) * n as f32) as usize % n;
        let (_job, id) = staff[pick];
        let phase = phase_of(id);
        // This worker's own day at the desk: in by their own minute, out by
        // their own — so an office empties over an hour and a half, not at one
        // stroke of the clock.
        let arrive = 7.85 + 1.2 * phase as f64;
        let leave = 16.5 + 2.6 * phase as f64;
        let mut lit = floor_let
            && !at.weekend
            && at.hour >= arrive
            && at.hour < leave
            && hash(lot_seed as u64 ^ 0x33, j as u64) < 0.62;
        // A late worker: a few desks stay on into the evening.
        if !lit
            && floor_let
            && at.hour >= 16.5
            && at.hour < 17.6 + 2.9 * phase as f64
            && hash(lot_seed as u64 ^ 0x31, j as u64) < 0.18
        {
            lit = true;
        }
        // The cleaner's floor: a corridor, some of the rooms on it.
        if !lit && cleaner_floor && clean_this && hash(lot_seed as u64 ^ 0x79, j as u64) < 0.42 {
            lit = true;
        }
        // Downtown never goes completely black: a lobby, a night porter, a
        // floor left on. A couple of slots out of the tower's 64.
        if !lit && (at.hour >= 19.5 || at.hour < 5.5) && hash(lot_seed as u64 ^ 0x9A, j as u64) < 0.03 {
            lit = true;
        }
        if lit {
            bits |= 1 << j;
        }
    }
    bits
}

/// A shop: open by its own hours (staggered by the keeper's phase), a few lights
/// while it is being cleaned up, a security light all night.
fn shop_slots(staff: &[(crate::people::JobKind, usize)], at: At, lot_seed: u32) -> u64 {
    let phase = staff
        .first()
        .map(|&(_, id)| phase_of(id))
        .unwrap_or(hash(lot_seed as u64, 3));
    let open = 7.6 + 2.0 * phase as f64;
    let close = 17.0 + 4.2 * phase as f64;
    let mut bits = 0u64;
    for j in 0..SLOTS {
        let h = slot_hour(at, lot_seed, j);
        let trading = h >= open && h < close;
        let tidying = h >= close && h < close + 0.9;
        let lit = if trading {
            // The window displays, not every pane.
            hash(lot_seed as u64 ^ 0x11, j as u64) < 0.72
        } else if tidying {
            hash(lot_seed as u64 ^ 0x12, j as u64) < 0.35
        } else {
            // One small light on inside, all night.
            j == 0 && hash(lot_seed as u64 ^ 0x13, 0) < 0.55
        };
        if lit {
            bits |= 1 << j;
        }
    }
    bits
}

/// A works: the shift that is on, which changes three times a day.
fn works_slots(at: At, lot_seed: u32, day: u64) -> u64 {
    let shift = ((day + (hash(lot_seed as u64, 9) * 3.0) as u64) % 3) as u8;
    let mut bits = 0u64;
    for j in 0..SLOTS {
        let at = At {
            hour: slot_hour(at, lot_seed, j),
            weekend: at.weekend,
        };
        let on = match shift {
            0 => at.hour >= 6.0 && at.hour < 14.0,
            1 => at.hour >= 14.0 && at.hour < 22.0,
            _ => at.hour >= 22.0 || at.hour < 6.0,
        };
        let lit = if on {
            // A factory works with its lights on, but not in every room: the
            // floor, the office, the canteen.
            hash(lot_seed as u64 ^ 0x21, j as u64) < 0.34
        } else {
            // Between shifts: the plant's own lamps.
            hash(lot_seed as u64 ^ 0x22, j as u64) < 0.10
        };
        if lit {
            bits |= 1 << j;
        }
    }
    bits
}

/// A civic building: a caretaker's round and the evening's business.
fn civic_slots(at: At, lot_seed: u32) -> u64 {
    let mut bits = 0u64;
    for j in 0..SLOTS {
        let at = At {
            hour: slot_hour(at, lot_seed, j),
            weekend: at.weekend,
        };
        let lit = if at.hour >= 8.5 && at.hour < 17.0 {
            hash(lot_seed as u64 ^ 0x41, j as u64) < 0.45
        } else if at.hour >= 18.0 && at.hour < 22.5 {
            hash(lot_seed as u64 ^ 0x42, j as u64) < 0.30
        } else {
            // A caretaker's round: one corridor, two floors.
            j < 2 && hash(lot_seed as u64 ^ 0x43, j as u64) < 0.6
        };
        if lit {
            bits |= 1 << j;
        }
    }
    bits
}

/// The plan: `out[i]` is lot `i`'s lit slots at `at`.
pub fn plan(sim: &Sim, at: At) -> Vec<u64> {
    let lots: &[Lot] = &sim.lots;
    let mut out = vec![0u64; lots.len()];
    // B77: the town's one bakery keeps its own hours.
    let bakery = sim.bakery_index();
    // A door is a tile just outside a lot, and two neighbouring lots can share
    // one — so a door is a *group* of lots, and the people who use it are dealt
    // out among them: by bed for a home, by turns for a workplace. (Reading only
    // the first lot per door put nine residents in a two-bedroom house.)
    let mut home_door: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    let mut work_door: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, l) in lots.iter().enumerate() {
        let door = crate::people::door_tile(l);
        match l.kind {
            LotKind::House
            | LotKind::Townhouse
            | LotKind::Apartment
            | LotKind::LogCabin
            | LotKind::Motel => home_door.entry(door).or_default().push(i),
            _ => {}
        }
        match l.kind {
            LotKind::Office
            | LotKind::Tower
            | LotKind::Industry
            | LotKind::Shop
            | LotKind::Store
            | LotKind::Landmark(_) => work_door.entry(door).or_default().push(i),
            _ => {}
        }
    }
    let mut residents: Vec<Vec<(AgeGroup, crate::people::JobKind, usize)>> = vec![Vec::new(); lots.len()];
    let mut staff: Vec<Vec<(crate::people::JobKind, usize)>> = vec![Vec::new(); lots.len()];
    // How many people of each door each lot has taken, for the dealing out.
    let mut home_fill: HashMap<(i32, i32), usize> = HashMap::new();
    let mut work_turn: HashMap<(i32, i32), usize> = HashMap::new();
    for (id, p) in sim.people.list.iter().enumerate() {
        if !p.alive() {
            continue;
        }
        if let Some(h) = p.home {
            if let Some(group) = home_door.get(&h) {
                // Fill the lots of this door in order, each to its own beds.
                let taken = home_fill.entry(h).or_insert(0);
                let mut seat = group.len() - 1;
                let mut acc = 0usize;
                for &li in group {
                    let beds = lots[li].occupants_now().max(1) as usize;
                    if *taken < acc + beds {
                        seat = group.iter().position(|&x| x == li).unwrap_or(seat);
                        break;
                    }
                    acc += beds;
                }
                *taken += 1;
                residents[group[seat]].push((p.kind, p.job_kind, id));
            }
        }
        if let Some(j) = p.job {
            if let Some(group) = work_door.get(&j) {
                let turn = work_turn.entry(j).or_insert(0);
                let li = group[*turn % group.len()];
                *turn += 1;
                staff[li].push((p.job_kind, id));
            }
        }
    }
    let day = sim.time as u64 / 86400;
    for (i, l) in lots.iter().enumerate() {
        if !l.finished() {
            continue;
        }
        // A building whose staff the people layer never filled (a tower that
        // went up last week, a shop nobody works in yet) is still a *building*:
        // it keeps its trade's hours with a few people's worth of rooms, from its
        // own seed. (Otherwise half a downtown stands black all night.)
        if staff[i].is_empty()
            && matches!(
                l.kind,
                LotKind::Office | LotKind::Tower | LotKind::Shop | LotKind::Store
            )
        {
            let n = 4u32;
            for k in 0..n {
                let id = 0x4000_0000usize ^ (l.seed as usize).wrapping_mul(31) ^ (k as usize).wrapping_mul(0x9E37);
                staff[i].push((crate::people::JobKind::Office, id));
            }
        }
        out[i] = match l.kind {
            LotKind::House
            | LotKind::Townhouse
            | LotKind::Apartment
            | LotKind::LogCabin
            | LotKind::Motel => home_slots(&residents[i], at, l.seed, l.storeys),
            LotKind::Office | LotKind::Tower => office_slots(&staff[i], at, l.seed, l.storeys),
            LotKind::Shop | LotKind::Store => shop_slots(&staff[i], at, l.seed),
            LotKind::Industry | LotKind::Barn => works_slots(at, l.seed, day),
            LotKind::Church
            | LotKind::Farmhouse
            | LotKind::Hall
            | LotKind::Landmark(_)
            | LotKind::GasStation
            | LotKind::DriveThrough
            | LotKind::CarWash
            | LotKind::Dealership => civic_slots(at, l.seed),
            // Nothing behind these windows: parks, plots, car parks.
            _ => 0,
        };
        // B77/D84: the bakery is up before the town is — one window lit on the
        // shop front and one above it from half four, so the small hours have a
        // place that is awake.
        if bakery == Some(i) && (4.3..7.6).contains(&at.hour) {
            out[i] |= 0b11;
        }
    }
    out
}

impl Sim {
    /// B72: which windows of every lot are lit at `hour` — the plan the renderer
    /// paints. Indexed by lot.
    pub fn window_plan(&self, hour: f64, weekend: bool) -> Vec<u64> {
        plan(self, At::new(hour, weekend))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bedtime_is_a_persons_own() {
        // Two people, two phases: their rooms do not go dark in the same minute.
        let a = bed_hours(AgeGroup::Adult, 0.05);
        let b = bed_hours(AgeGroup::Adult, 0.95);
        assert!(a.0 < b.0, "{:?} {:?}", a, b);
        assert!(!in_bed(AgeGroup::Adult, 0.05, 21.5)); // bed at ~21:52
        assert!(in_bed(AgeGroup::Adult, 0.95, 1.0)); // bed at ~00:44
    }

    #[test]
    fn an_empty_house_has_no_lights() {
        let bits = home_slots(&[], At { hour: 21.0, weekend: false }, 7, 2);
        assert_eq!(bits, 0);
    }
}
