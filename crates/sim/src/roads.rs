//! Road sanity and hierarchy (B41/D63): what class a street tile is, and the
//! clean-up pass that keeps the network honest — junctions as proper T or
//! 4-way crossings, no diagonal near-misses, no slivers, no dead-end stubs that
//! serve nothing.
//!
//! The class is *derived*, never stored: a lane's class follows its surface, the
//! grid's avenue lines and how much street runs through it. That way a street
//! that gets paved, extended or joined changes class on its own.

use map::Kind;

use crate::{Era, Sim};

/// What a street tile is, for painting, furniture and the plan overlay (D63).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RoadClass {
    /// Not a street (grass, water, a worn footpath only).
    None,
    /// A worn footpath — people, not vehicles.
    Path,
    /// A dirt or gravel lane (the settlement's cart track).
    Track,
    /// A paved street inside a block.
    Local,
    /// A paved street that carries traffic between blocks.
    Collector,
    /// A two-lane avenue: the grid's main lines.
    Avenue,
}

impl RoadClass {
    pub fn name(self) -> &'static str {
        match self {
            RoadClass::None => "none",
            RoadClass::Path => "path",
            RoadClass::Track => "track",
            RoadClass::Local => "local",
            RoadClass::Collector => "collector",
            RoadClass::Avenue => "avenue",
        }
    }

    /// Is this a street a vehicle drives on?
    pub fn is_vehicle(self) -> bool {
        matches!(
            self,
            RoadClass::Track | RoadClass::Local | RoadClass::Collector | RoadClass::Avenue
        )
    }

    /// Is this a paved street (paint, lamps, signs belong here)?
    pub fn is_paved(self) -> bool {
        matches!(
            self,
            RoadClass::Local | RoadClass::Collector | RoadClass::Avenue
        )
    }
}

impl Sim {
    /// How many lane neighbours this tile has (4-neighbourhood).
    pub fn lane_degree(&self, x: i32, y: i32) -> u8 {
        let mut n = 0;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            if self.map.at(x + dx, y + dy).kind.is_lane() {
                n += 1;
            }
        }
        n
    }

    /// Length of the straight run of lane through this tile, along its own axis.
    pub fn run_length(&self, x: i32, y: i32) -> i32 {
        self.run_length_capped(x, y, i32::MAX)
    }

    /// `run_length`, but stop counting once the run is `cap` long (B61). The
    /// road class only asks "is it at least 8, at least 12", so walking a
    /// 200-tile avenue to the map edge for every tile of every street pass was
    /// pure cost — it was 25-29 % of a 30-day run in `perf`.
    pub fn run_length_capped(&self, x: i32, y: i32, cap: i32) -> i32 {
        if !self.map.at(x, y).kind.is_lane() {
            return 0;
        }
        let cap = cap.max(1);
        let along_x = self.map.at(x - 1, y).kind.is_lane() || self.map.at(x + 1, y).kind.is_lane();
        let (dx, dy) = if along_x { (1, 0) } else { (0, 1) };
        let mut n = 1;
        for sgn in [1, -1] {
            let mut k = 1;
            // `map.at` clamps to the edge, so a lane that reaches the border
            // would otherwise count forever: stop at the map's edge.
            while n < cap
                && self.map.in_bounds(x + dx * k * sgn, y + dy * k * sgn)
                && self.map.at(x + dx * k * sgn, y + dy * k * sgn).kind.is_lane()
            {
                n += 1;
                k += 1;
            }
        }
        n
    }

    /// Is this tile on one of the grid's avenue lines (B26b: every third line)?
    pub fn on_avenue_line(&self, x: i32, y: i32) -> bool {
        let (ox, oy, pitch) = (self.blocks().ox, self.blocks().oy, crate::blocks::PITCH);
        let on_x_line = (x - ox).rem_euclid(pitch) == 0 && self.blocks.avenue((x - ox) / pitch);
        let on_y_line = (y - oy).rem_euclid(pitch) == 0 && self.blocks.avenue((y - oy) / pitch);
        on_x_line || on_y_line
    }

    /// The class of a tile (B41/D63).
    pub fn road_class(&self, x: i32, y: i32) -> RoadClass {
        let t = self.map.at(x, y);
        if !t.kind.is_lane() {
            if t.kind.is_water() || t.kind.is_worked() {
                return RoadClass::None;
            }
            return if map::WearStage::of(self.map.wear(x, y)) >= map::WearStage::Footpath {
                RoadClass::Path
            } else {
                RoadClass::None
            };
        }
        if matches!(t.kind, Kind::Dirt | Kind::Gravel) {
            return RoadClass::Track;
        }
        // Paved. The grid's own avenue lines are the avenues; a junction or a
        // long run is a collector; the rest are the short fill streets.
        // B61: the two thresholds below are 12 and 8, so the walk stops at 13.
        let run = self.run_length_capped(x, y, 13);
        if self.on_avenue_line(x, y) {
            return RoadClass::Avenue;
        }
        // A grid street that runs past one block, or a junction on a long run,
        // carries traffic between blocks: a collector. The rest are the short
        // fill streets inside a block.
        if run >= 12 || (self.lane_degree(x, y) >= 3 && run >= 8) {
            return RoadClass::Collector;
        }
        RoadClass::Local
    }

    /// The class of every lane tile, for the overlay and the furniture pass.
    pub fn road_classes(&self) -> Vec<((i32, i32), RoadClass)> {
        let mut out = Vec::new();
        for y in 0..self.map.h {
            for x in 0..self.map.w {
                let c = self.road_class(x, y);
                if c != RoadClass::None {
                    out.push(((x, y), c));
                }
            }
        }
        out
    }

    /// Tidy the network (B41/D63 item 1). Three rules, each cheap and local:
    ///
    /// 1. **Snap near-misses** — two lane ends a single tile apart are joined,
    ///    so the network has no diagonal-only touch (D32: one component).
    /// 2. **Fill slivers** — a one-tile hole with lanes on three or four sides
    ///    becomes road: a junction, not a pinch point.
    /// 3. **Drop dead ends** — a lane end that reaches nothing (no lot within
    ///    two tiles, shorter than three tiles of run) goes back to grass.
    ///
    /// Returns how many tiles changed.
    pub fn tidy_roads(&mut self) -> u32 {
        let mut changed = 0;
        changed += self.snap_near_misses();
        changed += self.fill_slivers();
        // B83: the pass stops at the metropolis as it always did — running it in
        // every era cost 16 % of a 60-day run for one or two stubs, and the
        // renderer now turns any lane end without a drive in a circle, which is
        // what the eye needs.
        if self.era() < Era::Metropolis {
            changed += self.drop_dead_ends();
            // Dropping a dead end can open a two-sided pinch into a three-sided
            // sliver, so fill once more: the invariant is about the state the
            // pass leaves behind (B42).
            changed += self.fill_slivers();
        }
        changed
    }

    /// A diagonal-only touch — two lanes meeting at a corner with neither
    /// shared orthogonal tile laid — is closed by laying one of the two gap
    /// tiles (B41 rule 1).
    fn snap_near_misses(&mut self) -> u32 {
        let mut laid = 0;
        for y in 1..self.map.h - 1 {
            for x in 1..self.map.w - 1 {
                if !self.map.at(x, y).kind.is_lane() {
                    continue;
                }
                for (dx, dy) in [(1, 1), (1, -1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if !self.map.at(nx, ny).kind.is_lane() {
                        continue;
                    }
                    // Joined already through an orthogonal lane?
                    if self.map.at(x + dx, y).kind.is_lane()
                        || self.map.at(x, y + dy).kind.is_lane()
                    {
                        continue;
                    }
                    // Lay the gap tile that is cheaper to build (flat, free).
                    for (gx, gy) in [(x + dx, y), (x, y + dy)] {
                        if self.can_lay(gx, gy) {
                            self.lay_one(gx, gy);
                            laid += 1;
                            break;
                        }
                    }
                }
            }
        }
        laid
    }

    /// A one-tile hole inside the street network becomes road (B41 rule 2).
    fn fill_slivers(&mut self) -> u32 {
        let mut filled = 0;
        for y in 1..self.map.h - 1 {
            for x in 1..self.map.w - 1 {
                let t = self.map.at(x, y);
                if t.kind.is_lane() || t.is_water() || t.kind.is_worked() {
                    continue;
                }
                let lanes = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .filter(|(dx, dy)| self.map.at(x + dx, y + dy).kind.is_lane())
                    .count();
                if lanes >= 3 && self.can_lay(x, y) {
                    self.lay_one(x, y);
                    filled += 1;
                }
            }
        }
        filled
    }

    /// A lane end that serves nothing goes back to grass (B41 rule 3).
    fn drop_dead_ends(&mut self) -> u32 {
        let mut dropped = 0;
        let mut tips: Vec<(i32, i32)> = Vec::new();
        for y in 1..self.map.h - 1 {
            for x in 1..self.map.w - 1 {
                if self.map.at(x, y).kind.is_lane()
                    && self.lane_degree(x, y) <= 1
                    && self.run_length(x, y) <= 3
                {
                    tips.push((x, y));
                }
            }
        }
        for (x, y) in tips {
            // Keep a tip that serves a lot, or that is a bridge end.
            let used = self.lots.iter().any(|l| {
                (x - l.x).abs() <= l.w + 2 && (y - l.y).abs() <= l.h + 2
            });
            if used || self.map.at(x, y).kind == Kind::Bridge {
                continue;
            }
            let mut t = self.map.at(x, y);
            t.kind = Kind::Grass;
            self.write(x, y, t);
            self.remark_around(x, y);
            self.map.wear[(y * self.map.w + x) as usize] = 0;
            dropped += 1;
        }
        dropped
    }

    /// Can a new lane tile be laid here? Not on water, a lot, a building or a
    /// cliff.
    fn can_lay(&self, x: i32, y: i32) -> bool {
        if !self.map.in_bounds(x, y) {
            return false;
        }
        let t = self.map.at(x, y);
        if t.kind.is_water() || t.kind.is_worked() || t.kind.is_lane() {
            return false;
        }
        if self.lots.iter().any(|l| l.covers(x, y)) {
            return false;
        }
        t.relief() <= 1
    }

    /// Lay one dirt lane tile, graded to its neighbours (D34).
    fn lay_one(&mut self, x: i32, y: i32) {
        // Not across the airfield's ground (B99c: closing a gap in the network
        // was the one lane writer that did not know the airfield was there, and
        // a street through the runway's grass is exactly what D73 forbids).
        if self.on_airfield(x, y) {
            return;
        }
        let mut t = self.map.at(x, y);
        t.kind = Kind::Dirt;
        self.write(x, y, t);
        self.remark_around(x, y);
        self.events.push(crate::Event {
            time: self.time,
            text: format!("a gap in the street is closed at {x},{y}"),
        });
    }
}

#[cfg(test)]
mod edge_tests {
    use crate::{Pace, Sim};
    use map::{Kind, Tile};

    /// A lane that runs to the map border must have a finite run length (the
    /// clamping `map.at` made this loop forever).
    #[test]
    fn run_length_terminates_on_a_border_lane() {
        let mut s = Sim::new(1, Pace::Month, 41, 41);
        let w = s.map.w;
        for x in 0..w {
            let mut t = s.map.at(x, 0);
            t.kind = Kind::Road;
            s.map.set(x, 0, t);
        }
        let _: Tile = s.map.at(0, 0);
        assert_eq!(s.run_length(5, 0), w);
    }

    /// B83: a lane that ends in open ground with nothing to serve is a stub, and
    /// the tidy pass takes it back to grass (in every era).
    #[test]
    fn a_stub_that_serves_nothing_is_dropped() {
        let mut s = Sim::new(7, Pace::Month, 41, 41);
        // A two-tile spur off a long lane, in the middle of a field of grass.
        for x in 5..=25 {
            let mut t = s.map.at(x, 20);
            t.kind = Kind::Road;
            s.map.set(x, 20, t);
        }
        for y in 21..=22 {
            let mut t = s.map.at(10, y);
            t.kind = Kind::Road;
            s.map.set(10, y, t);
        }
        s.tidy_roads();
        assert!(!s.map.at(10, 22).kind.is_lane(), "the stub's tip is still a lane");
    }

    /// B83: paint belongs to paved streets only — never a dirt or gravel track.
    #[test]
    fn paint_never_lands_on_a_track() {
        use crate::roads::RoadClass;
        let mut s = Sim::new(7, Pace::Month, 41, 41);
        s.run_to(12.0 * 86400.0);
        for &(x, y) in s.streetworks().paint.iter() {
            assert!(
                s.road_class(x, y).is_paved(),
                "paint on a {} at {x},{y}",
                s.road_class(x, y).name()
            );
            let _ = RoadClass::Track;
        }
    }

    /// B83: while the tidy pass runs (a town, not a metropolis), no lane end in
    /// open ground is left without something to serve: what survives is a drive
    /// or a bridge. A metropolis keeps long approaches, and the renderer turns
    /// those ends in a circle.
    #[test]
    fn no_lane_end_is_left_serving_nothing() {
        let mut s = Sim::new(7, Pace::Month, 61, 61);
        s.postpone_ending();
        s.run_to(20.0 * 86400.0);
        s.tidy_roads();
        let (w, h) = (s.map.w, s.map.h);
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                if !s.map.at(x, y).kind.is_lane() || s.lane_degree(x, y) > 1 {
                    continue;
                }
                if s.map.at(x, y).kind == Kind::Bridge {
                    continue;
                }
                let serves = s.lots.iter().any(|l| {
                    (x - l.x).abs() <= l.w + 2 && (y - l.y).abs() <= l.h + 2
                });
                assert!(serves, "lane end {x},{y} ends in the grass");
            }
        }
    }
}
