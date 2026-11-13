//! B74/D81: the overlap audit.
//!
//! One question, asked of a grown city: does anything stand on something else?
//! The sim already keeps a lot of local rules (a lane clears its own bed, a lot
//! keeps off a street, the railway takes its corridor), but they are enforced
//! place by place, so a rule added for one path (a bridge, an upgrade, a saved
//! city, the reclaim pass) can quietly break another. This module builds **one
//! occupancy map** out of the finished city and reports every conflict it can
//! see, so the rules can be tested against the city instead of against the code
//! that made it.
//!
//! The map is derived, never stored: [`Occupancy::build`] reads the tile map and
//! the lots, so it cannot go stale, and the audit can run on a loaded save.
//!
//! What it checks (the brief's list):
//! * building/building, building/road, building/rail, building/water;
//! * tree/road, tree/rail, tree/building (including the canopy margin a tree
//!   keeps from a wall), tree/plaza paving;
//! * park/road (a green laid over a lane), prop/anything (a bush or a boulder
//!   on a claimed tile);
//! * every lot with no road access.
//!
//! Sprite intrusion is measured in pixels, not tiles, so it lives in the
//! renderer (`render::audit`), where the art's own extents are known.

use crate::landmarks::LandmarkKind;
use crate::{Lot, LotKind, Sim};
use map::{Kind, Prop, TileMap};

/// Bits of the occupancy map. A tile may wear more than one (a park's paving
/// under a plaza is still a park edge), but a building never shares its tiles
/// with a lane.
pub const CLAIM_ROAD: u8 = 1 << 0;
pub const CLAIM_RAIL: u8 = 1 << 1;
pub const CLAIM_BUILDING: u8 = 1 << 2;
pub const CLAIM_PARK: u8 = 1 << 3;
pub const CLAIM_PLAZA: u8 = 1 << 4;
pub const CLAIM_PAVING: u8 = 1 << 5;
pub const CLAIM_WATER: u8 = 1 << 6;
/// A lot with walls on it (a building), as opposed to open ground a lot owns
/// (a car park, a graveyard, a plaza).
pub const CLAIM_WALLS: u8 = 1 << 7;

/// Every bit, for the report.
pub const ALL_CLAIMS: u8 = CLAIM_ROAD
    | CLAIM_RAIL
    | CLAIM_BUILDING
    | CLAIM_PARK
    | CLAIM_PLAZA
    | CLAIM_PAVING
    | CLAIM_WATER;

/// What a kind of lot claims: a building's footprint (walls and yard), a green,
/// a paved square.
pub fn claims_of(kind: LotKind) -> u8 {
    match kind {
        LotKind::Park => CLAIM_PARK,
        LotKind::Landmark(LandmarkKind::Plaza) => CLAIM_PLAZA,
        _ => CLAIM_BUILDING | if has_walls(kind) { CLAIM_WALLS } else { 0 },
    }
}

/// A lot with walls on it (as opposed to a green, a graveyard or a plaza).
pub fn has_walls(kind: LotKind) -> bool {
    !matches!(
        kind,
        LotKind::Park
            | LotKind::Cemetery
            | LotKind::CarPark
            | LotKind::Landmark(LandmarkKind::Plaza)
    )
}

/// The share of a lot its building stands on (the renderer's `INSET` setback
/// leaves the rest as yard). Used for the canopy margin, not for walls.
pub const SETBACK: f32 = 0.13;

/// One tile's claims, and the map they were derived from.
pub struct Occupancy {
    pub w: i32,
    pub h: i32,
    cells: Vec<u8>,
    /// The lot index that claims each building tile (`u16::MAX` = none), so an
    /// overlap can say *which* two lots are on top of each other.
    lot_at: Vec<u16>,
    /// Rail corridor tiles (the line, the second track, the sidings and the
    /// station's own ground).
    rail: Vec<bool>,
}

impl Occupancy {
    /// Build the map from a grown city: tile kinds first, then what the lots
    /// claim on top. A lot that is still being staked claims its own footprint
    /// (a construction site is just as much in the way as a finished wall).
    pub fn build(sim: &Sim) -> Self {
        let (w, h) = (sim.map.w, sim.map.h);
        let n = (w * h) as usize;
        let mut cells = vec![0u8; n];
        let mut lot_at = vec![u16::MAX; n];
        let mut rail = vec![false; n];
        let idx = |x: i32, y: i32| (y * w + x) as usize;
        for y in 0..h {
            for x in 0..w {
                let t = sim.map.at(x, y);
                let mut c = 0u8;
                if t.kind.is_lane() {
                    c |= CLAIM_ROAD;
                }
                if t.kind == Kind::Sidewalk {
                    c |= CLAIM_PAVING;
                }
                if t.kind.is_water() {
                    c |= CLAIM_WATER;
                }
                // A lane's own pavement: the tile in front of a street wall is
                // still the street's ground, and nothing stands on it.
                cells[idx(x, y)] = c;
            }
        }
        for (i, l) in sim.lots.iter().enumerate() {
            let claim = claims_of(l.kind);
            for ty in l.y..l.y + l.h {
                for tx in l.x..l.x + l.w {
                    if tx < 0 || ty < 0 || tx >= w || ty >= h {
                        continue;
                    }
                    let k = idx(tx, ty);
                    cells[k] |= claim;
                    if claim & CLAIM_BUILDING != 0 && lot_at[k] == u16::MAX {
                        lot_at[k] = i as u16;
                    }
                }
            }
        }
        for &(x, y) in sim.rail_tiles() {
            if x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            rail[idx(x, y)] = true;
            cells[idx(x, y)] |= CLAIM_RAIL;
        }
        Self {
            w,
            h,
            cells,
            lot_at,
            rail,
        }
    }

    #[inline]
    pub fn at(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0;
        }
        self.cells[(y * self.w + x) as usize]
    }

    #[inline]
    pub fn has(&self, x: i32, y: i32, bit: u8) -> bool {
        self.at(x, y) & bit != 0
    }

    #[inline]
    pub fn lot(&self, x: i32, y: i32) -> Option<u16> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return None;
        }
        let i = self.lot_at[(y * self.w + x) as usize];
        (i != u16::MAX).then_some(i)
    }

    /// Is this the railway's own ground (`sim.rail.on_rail_ground` covers the
    /// station and the yard; the map only carries the line itself)?
    #[inline]
    pub fn rail(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return false;
        }
        self.rail[(y * self.w + x) as usize]
    }

    /// Ground nothing may stand on: a lane, the rail bed, a building's
    /// footprint, a paved square, a pavement.
    #[inline]
    pub fn hard(&self, x: i32, y: i32) -> bool {
        self.at(x, y) & (CLAIM_ROAD | CLAIM_RAIL | CLAIM_BUILDING | CLAIM_PLAZA | CLAIM_PAVING) != 0
    }

    /// Ground a tree must not stand on: the same, plus anything the railway
    /// owns beyond the line itself (the station, the platforms, the yard).
    pub fn planted(&self, x: i32, y: i32) -> bool {
        self.hard(x, y) || self.rail(x, y)
    }
}

/// What went wrong, one counter per kind of overlap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlap {
    BuildingBuilding,
    BuildingRoad,
    BuildingRail,
    BuildingWater,
    TreeRoad,
    TreeRail,
    TreeBuilding,
    TreePaving,
    ParkRoad,
    PropAnything,
    NoRoadAccess,
    NoParking,
}

impl Overlap {
    pub const ALL: [Overlap; 12] = [
        Overlap::BuildingBuilding,
        Overlap::BuildingRoad,
        Overlap::BuildingRail,
        Overlap::BuildingWater,
        Overlap::TreeRoad,
        Overlap::TreeRail,
        Overlap::TreeBuilding,
        Overlap::TreePaving,
        Overlap::ParkRoad,
        Overlap::PropAnything,
        Overlap::NoRoadAccess,
        Overlap::NoParking,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Overlap::BuildingBuilding => "building/building",
            Overlap::BuildingRoad => "building/road",
            Overlap::BuildingRail => "building/rail",
            Overlap::BuildingWater => "building/water",
            Overlap::TreeRoad => "tree/road",
            Overlap::TreeRail => "tree/rail",
            Overlap::TreeBuilding => "tree/building",
            Overlap::TreePaving => "tree/paving",
            Overlap::ParkRoad => "park/road",
            Overlap::PropAnything => "prop/anything",
            Overlap::NoRoadAccess => "no road access",
            Overlap::NoParking => "apartment/no parking",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Overlap::BuildingBuilding => 0,
            Overlap::BuildingRoad => 1,
            Overlap::BuildingRail => 2,
            Overlap::BuildingWater => 3,
            Overlap::TreeRoad => 4,
            Overlap::TreeRail => 5,
            Overlap::TreeBuilding => 6,
            Overlap::TreePaving => 7,
            Overlap::ParkRoad => 8,
            Overlap::PropAnything => 9,
            Overlap::NoRoadAccess => 10,
            Overlap::NoParking => 11,
        }
    }
}

/// One conflict, with the tile it was seen on (so a test can print a place).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Finding {
    pub what: Overlap,
    pub x: i32,
    pub y: i32,
    /// The lots involved, when the finding is about lots.
    pub lots: (Option<u16>, Option<u16>),
}

/// The audit's answer: a count per kind, plus the first few places each kind
/// was seen (a test or the report needs somewhere to look).
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub counts: [u32; 12],
    pub samples: Vec<Finding>,
}

impl Report {
    pub fn total(&self) -> u32 {
        self.counts.iter().sum()
    }

    pub fn is_clean(&self) -> bool {
        self.total() == 0
    }

    pub fn count(&self, what: Overlap) -> u32 {
        self.counts[what.index()]
    }

    fn add(&mut self, what: Overlap, x: i32, y: i32, lots: (Option<u16>, Option<u16>)) {
        self.counts[what.index()] += 1;
        if self.samples.len() < 64 {
            self.samples.push(Finding { what, x, y, lots });
        }
    }

    /// Every kind with a non-zero count, as `kind: n`, for a report line.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for w in Overlap::ALL {
            let n = self.count(w);
            if n > 0 {
                if !out.is_empty() {
                    out.push_str(", ");
                }
                out.push_str(&format!("{}: {n}", w.name()));
            }
        }
        if out.is_empty() {
            out.push_str("clean");
        }
        out
    }
}

/// Audit a grown city. Cheap enough to run in a test on every land and seed:
/// one pass over the tiles, one pass over the lots, one neighbour check per
/// lot.
pub fn audit(sim: &Sim) -> Report {
    let occ = Occupancy::build(sim);
    let mut r = Report::default();
    audit_with(sim, &occ, &mut r);
    r
}

/// The same, against an occupancy map the caller already built (the spike and
/// the renderer's pixel pass share one).
pub fn audit_with(sim: &Sim, occ: &Occupancy, r: &mut Report) {
    let map = &sim.map;

    // --- lots against each other and against the ground ------------------
    building_vs_building(sim, r);
    for (i, l) in sim.lots.iter().enumerate() {
        lot_against_ground(map, l, i as u16, r);
    }

    // --- what is planted or lying about ---------------------------------
    props_vs_claims(sim, occ, r);

    // --- a block of flats parks its residents (B74 note) ----------------
    for (i, l) in sim.lots.iter().enumerate() {
        // A block still going up has no parking yet: the rule is about the
        // finished city, not a half-built shell (the brief's "every apartment
        // building has parking" is about the building, not the site).
        if crate::parking::needs_parking(l) && l.finished() && !sim.apartment_parks_itself(l) {
            r.add(Overlap::NoParking, l.x, l.y, (Some(i as u16), None));
        }
    }

    // --- every building reaches a street --------------------------------
    for (i, l) in sim.lots.iter().enumerate() {
        if !has_walls(l.kind) {
            continue;
        }
        if !lot_reaches_road(map, l) {
            r.add(
                Overlap::NoRoadAccess,
                l.x,
                l.y,
                (Some(i as u16), None),
            );
        }
    }
}

/// Two lots on the same tile. Reported once per overlapping pair, on the corner
/// of the tile they share.
fn building_vs_building(sim: &Sim, r: &mut Report) {
    let lots = &sim.lots;
    // A rect test per pair: a city has a few thousand lots, and an audit runs
    // once (a test, a save load, the report), never per frame.
    for (i, a) in lots.iter().enumerate() {
        for (j, b) in lots.iter().enumerate().skip(i + 1) {
            let x0 = a.x.max(b.x);
            let y0 = a.y.max(b.y);
            let x1 = (a.x + a.w).min(b.x + b.w);
            let y1 = (a.y + a.h).min(b.y + b.h);
            if x0 < x1 && y0 < y1 {
                r.add(
                    Overlap::BuildingBuilding,
                    x0,
                    y0,
                    (Some(i as u16), Some(j as u16)),
                );
            }
        }
    }
}

/// A lot's own tiles against the ground under it: a lane, the rail, water.
fn lot_against_ground(map: &TileMap, l: &Lot, i: u16, r: &mut Report) {
    for ty in l.y..l.y + l.h {
        for tx in l.x..l.x + l.w {
            let t = map.at(tx, ty);
            if t.kind.is_water() {
                r.add(Overlap::BuildingWater, tx, ty, (Some(i), None));
            } else if l.kind == LotKind::Park && t.kind.is_lane() {
                r.add(Overlap::ParkRoad, tx, ty, (Some(i), None));
            } else if t.kind.is_lane() {
                r.add(Overlap::BuildingRoad, tx, ty, (Some(i), None));
            }
        }
    }
}

/// Trees and props against the claims under them, with the margin a canopy
/// wants near a wall.
fn props_vs_claims(sim: &Sim, occ: &Occupancy, r: &mut Report) {
    let map = &sim.map;
    for y in 0..map.h {
        for x in 0..map.w {
            let t = map.at(x, y);
            if t.prop == Prop::None {
                continue;
            }
            let tree = t.prop.is_tree();
            let claims = occ.at(x, y);
            let on_road = claims & CLAIM_ROAD != 0;
            let on_rail = claims & CLAIM_RAIL != 0 || sim.on_rail_ground(x, y);
            let on_building = claims & CLAIM_WALLS != 0;
            let on_plaza = claims & CLAIM_PLAZA != 0;
            let on_paving = claims & CLAIM_PAVING != 0;
            if on_rail {
                r.add(
                    if tree { Overlap::TreeRail } else { Overlap::PropAnything },
                    x,
                    y,
                    (None, None),
                );
                continue;
            }
            if on_road {
                r.add(
                    if tree { Overlap::TreeRoad } else { Overlap::PropAnything },
                    x,
                    y,
                    (None, None),
                );
                continue;
            }
            if on_building {
                // A tree in a lot's yard is a garden tree, not an overlap; one
                // under the walls is. The walls are the lot inset by `SETBACK`.
                if tree && on_walls(sim, occ, x, y) {
                    r.add(Overlap::TreeBuilding, x, y, (occ.lot(x, y), None));
                    continue;
                }
                if !tree {
                    // A bush or a boulder in a yard is a garden too.
                    continue;
                }
            }
            if on_plaza || on_paving {
                r.add(
                    if tree { Overlap::TreePaving } else { Overlap::PropAnything },
                    x,
                    y,
                    (None, None),
                );
                continue;
            }
            // The canopy margin: a tree standing on the tile beside a wall has
            // its crown over the wall. This is the "tree through the corner of
            // a house" the review caught — but only for the big species: a
            // garden fruit tree is narrow enough never to reach the wall
            // (D81/Q3, round 2).
            let small = matches!(t.prop, Prop::Tree { species, .. } if map::treeline::is_small_species(species));
            if tree && !small && near_wall(occ, x, y) {
                r.add(Overlap::TreeBuilding, x, y, (None, None));
            }
        }
    }
}

/// Does this tile sit under the walls of the lot it is in (as opposed to the
/// yard the setback leaves at the lot's edge)? The tile's centre decides: the
/// walls are the lot rect pulled in by `SETBACK` on every side, which is what
/// the renderer draws.
fn on_walls(sim: &Sim, occ: &Occupancy, x: i32, y: i32) -> bool {
    let Some(i) = occ.lot(x, y) else {
        return false;
    };
    let Some(l) = sim.lots.get(i as usize) else {
        return false;
    };
    if !has_walls(l.kind) {
        return false;
    }
    let cx = x as f32 + 0.5;
    let cy = y as f32 + 0.5;
    let mx = (l.w as f32 * SETBACK).max(0.25);
    let my = (l.h as f32 * SETBACK).max(0.25);
    let x0 = l.x as f32 + mx;
    let x1 = (l.x + l.w) as f32 - mx;
    let y0 = l.y as f32 + my;
    let y1 = (l.y + l.h) as f32 - my;
    cx > x0 && cx < x1 && cy > y0 && cy < y1
}

/// Does a wall stand within one tile of `(x, y)`? A tree with a building on the
/// tile next to it has its crown over the wall: the brief's canopy margin.
fn near_wall(occ: &Occupancy, x: i32, y: i32) -> bool {
    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            if occ.has(x + dx, y + dy, CLAIM_WALLS) {
                return true;
            }
        }
    }
    false
}

/// Does a street reach this lot? A building faces the street on its `front`
/// side in the art, but every side counts: what matters is that a road — or the
/// drive or footpath that joins one — is there to reach. A pavement counts
/// because that is what the city lays to connect a plot set back from its
/// street (`crate::access`); a path is a way in, a bare lawn is not.
pub fn lot_reaches_road(map: &TileMap, l: &Lot) -> bool {
    let reachable = |x: i32, y: i32| {
        let k = map.at(x, y).kind;
        k.is_lane() || k == Kind::Sidewalk
    };
    for tx in l.x - 1..=l.x + l.w {
        for &ty in &[l.y - 1, l.y + l.h] {
            if reachable(tx, ty) {
                return true;
            }
        }
    }
    for ty in l.y - 1..=l.y + l.h {
        for &tx in &[l.x - 1, l.x + l.w] {
            if reachable(tx, ty) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pace, Sim};

    fn grown(seed: u64, days: f64) -> Sim {
        let vp = map::world_for_viewport(960, 540);
        let focus = map::screen_to_tile(&vp, 480.0, 270.0);
        let mut s = Sim::with_focus(seed, Pace::Month, vp.w, vp.h, focus);
        s.run_to(days * 86400.0);
        s
    }

    #[test]
    fn the_occupancy_map_reads_a_street() {
        let s = grown(7, 30.0);
        let occ = Occupancy::build(&s);
        let lanes = s.lane_tiles();
        assert!(!lanes.is_empty(), "no streets to audit");
        for (x, y) in lanes.iter().take(200) {
            assert!(occ.has(*x, *y, CLAIM_ROAD), "a lane at {x},{y} claims no road");
            assert!(!occ.has(*x, *y, CLAIM_BUILDING), "a lane at {x},{y} is a building");
        }
        for l in s.lots.iter().take(50) {
            assert!(occ.has(l.x, l.y, claims_of(l.kind)), "a lot claims nothing");
        }
    }

    /// The whole point: the audit runs, and it can see the faults it is for.
    /// A tree planted on a street is caught, and so is a bush on a rail bed.
    #[test]
    fn the_audit_sees_a_tree_on_a_street() {
        let mut s = grown(7, 20.0);
        let Some(&(x, y)) = s.lane_tiles().first() else {
            panic!("no lane");
        };
        let mut t = s.map.at(x, y);
        t.prop = Prop::Tree {
            species: map::TreeSpecies::Oak,
            size: 1,
            variant: 0,
        };
        s.map.set(x, y, t);
        let r = audit(&s);
        assert!(
            r.count(Overlap::TreeRoad) >= 1,
            "a tree on the street was not seen: {}",
            r.summary()
        );
        let f = r
            .samples
            .iter()
            .find(|f| f.what == Overlap::TreeRoad)
            .unwrap();
        assert_eq!((f.x, f.y), (x, y));
    }

    /// A lot with no street beside it is a lot nobody can reach.
    #[test]
    fn the_audit_sees_a_lot_with_no_road() {
        let mut s = grown(7, 20.0);
        // A lone house in the countryside: a grass patch with no lane within
        // two tiles of its 2x2 plot.
        let mut spot = None;
        'scan: for y in 4..s.map.h - 6 {
            for x in 4..s.map.w - 6 {
                let t = s.map.at(x, y);
                if t.kind != map::Kind::Grass {
                    continue;
                }
                let free = (-2..=3).all(|dy| {
                    (-2..=3).all(|dx| {
                        !s.map.at(x + dx, y + dy).kind.is_lane()
                            && s.map.at(x + dx, y + dy).kind != map::Kind::Water
                    })
                });
                if free {
                    spot = Some((x, y));
                    break 'scan;
                }
            }
        }
        let before = audit(&s).count(Overlap::NoRoadAccess);
        let (x, y) = spot.expect("a free patch of countryside");
        s.lots.push(Lot {
            x,
            y,
            w: 2,
            h: 2,
            level: s.map.at(x, y).min_level(),
            kind: LotKind::House,
            storeys: 1,
            style: 0,
            seed: 1,
            front: 0,
            stage: crate::Stage::Finished,
            stage_end: 0.0,
            started: 0.0,
        });
        let r = audit(&s);
        assert!(
            r.count(Overlap::NoRoadAccess) > before,
            "a house in the fields was not caught: {}",
            r.summary()
        );
    }
}
