//! Buses, the subway, the airport and the harbour (B58, D72).
//!
//! Transport is planned from the city the same way its districts are: from the
//! population, the street network and the ground, never at random. Each mode
//! grows through the eras:
//!
//! * **Buses** from the town era: two loop lines along the longest streets,
//!   stops with shelters every few tiles, a depot on the edge, and nothing
//!   running between 23:00 and 05:30.
//! * **The subway** from the metropolis era: a cut-and-cover trench dug along a
//!   downtown street over [`SUBWAY_BUILD_SECS`] of sim time — hoardings, then an
//!   open cut with crews and spoil, then the deck goes back on, then the
//!   entrances and their signs. Trains run below and are not drawn; people go in
//!   and out of the entrances at rush hour.
//! * **The airport** from the town era: a grass strip with a windsock and a
//!   hangar, paved and given a terminal in the city era, an apron, tower and
//!   beacon at the metropolis era. Placed on flat, dry land at the city's edge,
//!   connected to the street by its own access drive, sized to the city.
//! * **The harbour**: piers with a ferry route and moored fishing boats, beside
//!   the marina (the container port is the industrial waterfront, in `rail`).
//!
//! Everything here is planning + timetable; the renderer owns every pixel.

use map::{Kind, WaterKind};

use crate::rail::{Mover, Zone};
use crate::{District, Era, Sim};

/// How long a subway takes to build, in sim seconds (about 19 hours of real
/// time at the month pace — long enough to watch, short enough to see).
pub const SUBWAY_BUILD_SECS: f64 = 0.8 * 86400.0;
/// A bus calls at its stops this often, in tiles.
pub const BUS_STOP_GAP: i32 = 7;
/// The runway is two rows of pavement wide: one row reads as a line in the
/// grass at 1x, two read as a runway (B58c).
pub const AIRPORT_WIDTH: i32 = 2;
/// How far the airfield reaches from the runway's far row: the taxiway, the
/// apron, the terminal and the hangar (in rows).
pub const AIRPORT_DEPTH: i32 = 6;
/// Days into the world's life when the city is a metropolis for transport
/// (see [`Sim::metro_ready`]: the subway arrives late in the city era).
pub const METRO_DAYS: f64 = 28.0;

/// How big the airport is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum AirportStage {
    /// A grass strip, a windsock and a hangar (town era).
    Strip,
    /// Paved runway, terminal and apron (city era).
    Regional,
    /// International: a second apron stand, tower with a beacon, longer runway.
    International,
}

impl AirportStage {
    pub fn index(self) -> u8 {
        match self {
            AirportStage::Strip => 0,
            AirportStage::Regional => 1,
            AirportStage::International => 2,
        }
    }

    pub fn from_index(i: u8) -> Self {
        match i {
            0 => AirportStage::Strip,
            1 => AirportStage::Regional,
            _ => AirportStage::International,
        }
    }

    pub fn from_era(e: Era) -> Self {
        if e >= Era::Metropolis {
            AirportStage::International
        } else if e >= Era::City {
            AirportStage::Regional
        } else {
            AirportStage::Strip
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AirportStage::Strip => "grass strip",
            AirportStage::Regional => "regional airport",
            AirportStage::International => "international airport",
        }
    }
}

/// The airfield: everything the renderer draws and the timetable uses.
#[derive(Clone, PartialEq, Debug)]
pub struct Airport {
    pub stage: AirportStage,
    /// The runway's centre line in take-off order (first tile = threshold).
    /// Aircraft run on this line; the pavement is [`Self::width`] rows wide.
    pub runway: Vec<(i32, i32)>,
    /// How many rows of pavement the runway is (B58c: two, so it reads as a
    /// runway at 1x instead of a line in the grass).
    pub width: i32,
    /// Parallel taxiway from the apron to the threshold.
    pub taxiway: Vec<(i32, i32)>,
    pub terminal: Zone,
    pub apron: Zone,
    pub hangar: Option<Zone>,
    /// The tower and which way the runway lies from it (0..3, as the station's
    /// side flags).
    pub tower: Option<(i32, i32, u8)>,
    /// Where an aircraft stands before it leaves.
    pub gate: (i32, i32),
    /// The windsock, at the threshold.
    pub windsock: (i32, i32),
    /// The drive from the terminal to the street it joins.
    pub access: Vec<(i32, i32)>,
    /// Stands on the apron for parked aircraft.
    pub parked: Vec<(i32, i32)>,
    /// Sim time the paving finished (the strip is grass before it).
    pub paved_at: f64,
}

impl Airport {
    /// Every paved runway tile (all rows), for grading, clearing and drawing.
    pub fn runway_tiles(&self) -> Vec<(i32, i32)> {
        let mut out = Vec::with_capacity(self.runway.len() * self.width.max(1) as usize);
        let (dx, dy) = self.dir();
        let perp = if dx != 0 { (0, 1) } else { (1, 0) };
        for r in 0..self.width.max(1) {
            for &(x, y) in &self.runway {
                out.push((x + perp.0 * r, y + perp.1 * r));
            }
        }
        out
    }

    /// The runway's direction in tile steps.
    pub fn dir(&self) -> (i32, i32) {
        if self.runway.len() < 2 {
            return (1, 0);
        }
        let a = self.runway[0];
        let b = self.runway[1];
        (b.0 - a.0, b.1 - a.1)
    }

    /// The grass shoulders either side of the pavement: kept clear so nothing
    /// (a house, a tree) crowds the strip.
    pub fn shoulders(&self) -> Vec<(i32, i32)> {
        let (dx, dy) = self.dir();
        let perp = if dx != 0 { (0, 1) } else { (1, 0) };
        let w = self.width.max(1);
        let mut out = Vec::new();
        for &(x, y) in &self.runway {
            out.push((x - perp.0, y - perp.1));
            out.push((x + perp.0 * w, y + perp.1 * w));
        }
        out
    }

    /// The airfield's ground *plus* a mown margin around it (B58c): without the
    /// margin the city builds right up to the fence and the runway reads as a
    /// street between towers.
    pub fn fence(&self) -> Vec<(i32, i32)> {
        let mut core = self.runway_tiles();
        core.extend(self.shoulders());
        core.extend(self.taxiway.iter().copied());
        core.extend(self.ground_tiles());
        let mut out: std::collections::BTreeSet<(i32, i32)> = core.iter().copied().collect();
        for &(x, y) in &core {
            for dx in -2..=2i32 {
                for dy in -2..=2i32 {
                    if dx.abs() == 2 || dy.abs() == 2 {
                        out.insert((x + dx, y + dy));
                    }
                }
            }
        }
        out.into_iter().collect()
    }

    /// Is this tile part of the airfield's operational ground? Streets and lots
    /// keep off it (B58/D73): the runway and its shoulders, the taxiway, the
    /// apron, the terminal and the hangar.
    pub fn covers(&self, x: i32, y: i32) -> bool {
        self.runway_tiles().contains(&(x, y))
            || self.shoulders().contains(&(x, y))
            || self.taxiway.contains(&(x, y))
            || (x >= self.terminal.x
                && x < self.terminal.x + self.terminal.w
                && y >= self.terminal.y
                && y < self.terminal.y + self.terminal.h)
            || (x >= self.apron.x
                && x < self.apron.x + self.apron.w
                && y >= self.apron.y
                && y < self.apron.y + self.apron.h)
            || self.hangar.is_some_and(|h| {
                x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h
            })
    }

    /// The paved ground around the runway (apron, terminal, hangar): graded flat
    /// and kept clear like the strip itself.
    pub fn ground_tiles(&self) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for z in [Some(self.apron), Some(self.terminal), self.hangar]
            .into_iter()
            .flatten()
        {
            for ty in z.y..z.y + z.h {
                for tx in z.x..z.x + z.w {
                    out.push((tx, ty));
                }
            }
        }
        out
    }

    pub fn centre(&self) -> (i32, i32) {
        let n = self.runway.len().max(1);
        let (x, y) = self.runway[n / 2];
        (x, y)
    }

    pub fn label(&self) -> &'static str {
        self.stage.name()
    }
}

/// A subway station: the platform below (not drawn), its entrance on the
/// pavement and the grates that let the tunnels breathe.
#[derive(Clone, PartialEq, Debug)]
pub struct SubwayStation {
    /// The platform tile under the street.
    pub at: (i32, i32),
    /// The entrance tile and the side of the street it stands on (0/1 = +x/-x,
    /// 2/3 = +y/-y from the platform tile).
    pub entrance: (i32, i32, u8),
}

/// The subway: the line under the street and the works that built it.
#[derive(Clone, PartialEq, Debug)]
pub struct Subway {
    /// Sim time the first hoarding went up.
    pub started: f64,
    /// The street the cut runs along.
    pub line: Vec<(i32, i32)>,
    pub stations: Vec<SubwayStation>,
    /// Ventilation grates along the pavement.
    pub grates: Vec<(i32, i32)>,
}

/// The stage of the works at a given sim time: 0 hoardings, 1 open cut, 2 deck
/// going back on, 3 entrances and signs, 4 open. `progress` is 0..1 within the
/// whole build (the renderer uses it to move the crews).
pub fn subway_stage(started: f64, time: f64) -> (u8, f32) {
    let f = ((time - started) / SUBWAY_BUILD_SECS).clamp(0.0, 1.0) as f32;
    let stage = if f < 0.22 {
        0
    } else if f < 0.58 {
        1
    } else if f < 0.80 {
        2
    } else if f < 1.0 {
        3
    } else {
        4
    };
    (stage, f)
}

/// The harbour: piers into the water, a ferry route and moored fishing boats.
#[derive(Clone, PartialEq, Debug)]
pub struct Harbour {
    /// The quay: a straight run of shore tiles, in order along the shore. The
    /// ground under it is graded to one level, so the edge is straight (B58c:
    /// a quay of ragged diamond teeth is not a quay).
    pub quay: Vec<(i32, i32)>,
    /// The direction the water lies from the quay (one tile step).
    pub water_dir: (i32, i32),
    /// The piers: straight runs out from the quay into the water.
    pub piers: Vec<Vec<(i32, i32)>>,
    /// The ferry's route (water tiles, both ways).
    pub ferry: Vec<(i32, i32)>,
    /// Where the boats tie up (water tiles beside the pier).
    pub moored: Vec<(i32, i32)>,
    /// The fish market shed on the quay [x, y, w, h].
    pub shed: (i32, i32, i32, i32),
    /// Small cranes on the quay.
    pub cranes: Vec<(i32, i32)>,
}

impl Harbour {
    /// Every tile the harbour occupies (quay, piers, shed): graded flat and
    /// kept clear of lots and trees.
    pub fn ground(&self) -> Vec<(i32, i32)> {
        let mut out = self.quay.clone();
        for p in &self.piers {
            out.extend(p.iter().copied());
        }
        for ty in self.shed.1..self.shed.1 + self.shed.3 {
            for tx in self.shed.0..self.shed.0 + self.shed.2 {
                out.push((tx, ty));
            }
        }
        out
    }

    /// The water side of the quay (where the wall and the bollards go).
    pub fn water_tiles(&self) -> Vec<(i32, i32)> {
        let (dx, dy) = self.water_dir;
        self.quay.iter().map(|&(x, y)| (x + dx, y + dy)).collect()
    }
}

/// One bus line: the loop it drives and the stops along it.
#[derive(Clone, PartialEq, Debug)]
pub struct BusLine {
    /// Out and back (a closed loop, so a bus never teleports home).
    pub route: Vec<(i32, i32)>,
    /// Stop tiles, on the route.
    pub stops: Vec<(i32, i32)>,
    /// The pavement side of each stop (0..3, as the platform's side flag).
    pub sides: Vec<u8>,
}

// ------------------------------------------------------------------ planning

impl Sim {
    /// Plan whatever transport the era has unlocked and is not planned yet.
    /// Called every transport step; every branch is an `Option` test, so the
    /// steady state costs nothing.
    pub(crate) fn plan_transit(&mut self) {
        let era = self.era();
        if era < Era::Town {
            return;
        }
        let retry = |tried: f64, now: f64| tried == 0.0 || now - tried > 3600.0;
        if self.rail.bus_lines.is_empty() && retry(self.rail.bus_tried, self.time) {
            self.rail.bus_tried = self.time.max(1.0);
            self.rail.bus_lines = self.plan_bus_lines();
            if !self.rail.bus_lines.is_empty() {
                self.rail.depot = self
                    .zone_in(District::Suburb, 4)
                    .or_else(|| self.zone_in(District::Industrial, 4));
                self.events.push(crate::Event {
                    time: self.time,
                    text: "the first bus line opens: a shelter and a timetable board".into(),
                });
            }
        }
        if self.rail.airport.is_none() && retry(self.rail.airport_tried, self.time) {
            self.rail.airport_tried = self.time.max(1.0);
            self.rail.airport = self.plan_airport_unused();
        }
        let want = self.airport_stage();
        let upgrade = self
            .rail
            .airport
            .as_ref()
            .map(|ap| ap.stage)
            .filter(|was| *was != want);
        if let Some(was) = upgrade {
            // An upgrade keeps the site: the runway grows where it already is,
            // and the terminal arrives beside it.
            self.grow_airport(want);
            self.events.push(crate::Event {
                time: self.time,
                text: format!("the airfield is upgraded: {} to {}", was.name(), want.name()),
            });
        }
        if self.rail.harbour.is_none() && era >= Era::City && retry(self.rail.harbour_tried, self.time) {
            self.rail.harbour_tried = self.time.max(1.0);
            self.rail.harbour = self.plan_harbour();
        }
        // The subway belongs to the metro era. Its anchor is the sim time the
        // era began (saved, v10), so a reload does not restart the works.
        if self.metro_ready() && self.rail.metro_at.is_none() {
            self.rail.metro_at = Some(self.time);
        }
        if self.rail.subway.is_none() && retry(self.rail.subway_tried, self.time) {
            self.rail.subway_tried = self.time.max(1.0);
            if let Some(start) = self.rail.metro_at {
                self.rail.subway = self.plan_subway(start);
                if self.rail.subway.is_some() {
                    self.events.push(crate::Event {
                        time: self.time,
                        text: "the subway is approved: hoardings go up along the avenue".into(),
                    });
                }
            }
        }
        if !self.rail.electrified && self.metro_ready() {
            // B49 drives this flag in the era system; the default is that a
            // metropolis electrifies its line (catenary, electric sets).
            self.rail.electrified = true;
        }
    }

    /// Is this tile airfield ground (streets stay off it)? Cheap enough to call
    /// per candidate tile: the airfield is one option and a handful of lists.
    pub(crate) fn on_airfield(&self, x: i32, y: i32) -> bool {
        self.rail.airport.as_ref().is_some_and(|a| a.covers(x, y))
    }

    /// A city big enough for the metro era: the metropolis era itself, or a
    /// city of the city era that has matured past a month (D47).
    ///
    /// The metropolis era's own gate is 4 000 residents, which the growth engine
    /// does not reach on the 60×60 world a 7040×1440 desktop union makes
    /// (it tops out around 600 there) — so the subway would never be built
    /// anywhere. The belt-and-braces gate is the city era at [`METRO_DAYS`]:
    /// the same point in the world's life, expressed in the clock instead of a
    /// population the frame cannot hold. Open question for the planner: whether
    /// the era thresholds or the frame's land budget should move instead.
    pub fn metro_ready(&self) -> bool {
        self.era() >= Era::Metropolis || (self.era() >= Era::City && self.days() >= METRO_DAYS)
    }

    /// How big the airfield should be now.
    pub fn airport_stage(&self) -> AirportStage {
        if self.era() >= Era::Metropolis || (self.era() >= Era::City && self.days() >= 45.0) {
            AirportStage::International
        } else if self.era() >= Era::City {
            AirportStage::Regional
        } else {
            AirportStage::Strip
        }
    }

    /// The bus lines: the two longest straight street runs near the town, as
    /// loops, with stops and a shelter on the pavement side.
    fn plan_bus_lines(&mut self) -> Vec<BusLine> {
        let (fx, fy) = self.founding;
        let mut runs: Vec<Vec<(i32, i32)>> = Vec::new();
        // Rows of lanes.
        for y in 2..self.map.h - 2 {
            let mut run: Vec<(i32, i32)> = Vec::new();
            for x in 2..self.map.w - 2 {
                if self.map.at(x, y).kind.is_lane() {
                    run.push((x, y));
                } else {
                    if run.len() >= 10 {
                        runs.push(std::mem::take(&mut run));
                    } else {
                        run.clear();
                    }
                }
            }
            if run.len() >= 10 {
                runs.push(run);
            }
        }
        // Columns of lanes.
        for x in 2..self.map.w - 2 {
            let mut run: Vec<(i32, i32)> = Vec::new();
            for y in 2..self.map.h - 2 {
                if self.map.at(x, y).kind.is_lane() {
                    run.push((x, y));
                } else {
                    if run.len() >= 10 {
                        runs.push(std::mem::take(&mut run));
                    } else {
                        run.clear();
                    }
                }
            }
            if run.len() >= 10 {
                runs.push(run);
            }
        }
        // Only streets inside the picture: a bus line behind the camera's frame
        // is a route nobody ever sees (B58, D72).
        runs.retain(|r| {
            let mid = r[r.len() / 2];
            self.in_view(mid.0, mid.1, 28)
        });
        // Longest first, then nearest the founding; no two lines on one street.
        runs.sort_by_key(|r| {
            let mid = r[r.len() / 2];
            let d = (mid.0 - fx).abs() + (mid.1 - fy).abs();
            (std::cmp::Reverse(r.len() as i32), d)
        });
        let mut lines: Vec<BusLine> = Vec::new();
        for run in runs {
            if lines.len() >= 3 {
                break;
            }
            let (a, b) = (run[0], run[run.len() - 1]);
            let along_x = a.1 == b.1;
            // A line already served by a street within two tiles is a duplicate.
            if lines.iter().any(|l| {
                let mid = l.route[l.route.len() / 4];
                (mid.0 - a.0).abs() + (mid.1 - a.1).abs() < 6
                    && (mid.0 - b.0).abs() + (mid.1 - b.1).abs() < 6
            }) {
                continue;
            }
            // Out and back, so the bus drives the loop instead of vanishing at
            // the end of the road.
            let mut route = run.clone();
            let mut back = run.clone();
            back.reverse();
            route.extend(back.into_iter().skip(1));
            // Stops every BUS_STOP_GAP tiles, away from the ends.
            let mut stops = Vec::new();
            let mut sides = Vec::new();
            let mut i = BUS_STOP_GAP / 2;
            while i < run.len() as i32 - 2 {
                let (x, y) = run[i as usize];
                let (side, sx, sy) = if along_x {
                    if self.map.at(x, y + 1).kind == Kind::Sidewalk {
                        (2u8, x, y + 1)
                    } else if self.map.at(x, y - 1).kind == Kind::Sidewalk {
                        (3u8, x, y - 1)
                    } else {
                        (2u8, x, y + 1)
                    }
                } else if self.map.at(x + 1, y).kind == Kind::Sidewalk {
                    (0u8, x + 1, y)
                } else if self.map.at(x - 1, y).kind == Kind::Sidewalk {
                    (1u8, x - 1, y)
                } else {
                    (0u8, x + 1, y)
                };
                if !self.map.in_bounds(sx, sy) || self.map.at(sx, sy).is_water() {
                    i += BUS_STOP_GAP;
                    continue;
                }
                // The shelter must be *seen*: the camera looks from +x/+y, so a
                // tall block just in front of the pavement hides it (B58c item
                // 3: the bus-stop crop showed a tower and no shelter). If that
                // side is blocked, try the other pavement; if both are, the
                // street gets no stop here.
                let blocked = |sim: &Sim, px: i32, py: i32| -> bool {
                    sim.lots.iter().any(|l| {
                        l.storeys >= 3
                            && l.x + l.w > px
                            && l.x <= px + 2
                            && l.y + l.h > py
                            && l.y <= py + 2
                    })
                };
                let other = if along_x {
                    if side == 2 { 3 } else { 2 }
                } else if side == 0 {
                    1
                } else {
                    0
                };
                let (ox, oy) = if along_x {
                    (x, if other == 2 { y + 1 } else { y - 1 })
                } else if other == 0 {
                    (x + 1, y)
                } else {
                    (x - 1, y)
                };
                let chosen = if !blocked(self, sx, sy) {
                    Some((sx, sy, side))
                } else if self.map.in_bounds(ox, oy)
                    && !self.map.at(ox, oy).is_water()
                    && !blocked(self, ox, oy)
                {
                    Some((ox, oy, other))
                } else {
                    None
                };
                if let Some((_, _, side)) = chosen {
                    stops.push((x, y));
                    sides.push(side);
                }
                i += BUS_STOP_GAP;
            }
            if stops.len() >= 2 {
                lines.push(BusLine {
                    route,
                    stops,
                    sides,
                });
            }
        }
        lines
    }

    /// Plan the airfield on flat, dry land at the edge, away from the core, and
    /// size the runway to the city (D73: no runway through hills or downtown).
    fn plan_airport_unused(&mut self) -> Option<Airport> {
        let dbg = std::env::var("B58_DEBUG").is_ok();
        let mut why = [0usize; 8];
        let span = self.span();
        let (fx, fy) = self.founding;
        let longest = (span * 0.72).clamp(10.0, 24.0) as i32;
        let width = AIRPORT_WIDTH;
        // The longest runway the land will take, then shorter ones: a town gets
        // a grass strip, and a strip that fits is better than no airfield.
        for len in (longest - 8..=longest).rev().step_by(2) {
            let mut best: Option<(Airport, i32)> = None;
            for axis in [0u8, 1] {
              for flip in [1i32, -1] {
                // Tile (i, across) of the airfield, from its threshold corner.
                // The apron and terminal can lie on either side of the runway:
                // a town on one side would otherwise veto every site (B58c).
                let at = |i: i32, across: i32| -> (i32, i32) {
                    if axis == 0 {
                        (i, flip * across)
                    } else {
                        (flip * across, i)
                    }
                };
                // The footprint is `len` long and (width + depth) deep: the
                // loop has to leave room for both, whichever way it points.
                let (ew, eh) = if axis == 0 {
                    (len, width + AIRPORT_DEPTH)
                } else {
                    (width + AIRPORT_DEPTH, len)
                };
                // A mirrored layout reaches back towards the top/left of the
                // map, so its search window starts further in.
                let pad = if flip < 0 { width + AIRPORT_DEPTH } else { 0 };
                let (ew, eh) = if axis == 0 { (ew, eh + pad) } else { (ew + pad, eh) };
                for y in 4..self.map.h - 4 - eh {
                    for x in 4..self.map.w - 4 - ew {
                        // (x, y) walks the map as a corner; `at` turns it into
                        // the airfield's own (i, across) frame.
                        let corner = |i: i32, across: i32| -> (i32, i32) {
                            let (a, b) = at(i, across);
                            (x + a, y + b)
                        };
                        let (cx, cy) = corner(len / 2, width / 2);
                        let d = (cx - fx).abs() + (cy - fy).abs();
                        // At the city's edge, but inside the picture.
                        if (d as f32) < span * 0.55 || (d as f32) > span * 2.4 {
                            why[0] += 1;
                            continue;
                        }
                        if matches!(self.district_at(cx, cy), District::Downtown | District::Core) {
                            why[1] += 1;
                            continue;
                        }
                        if self.behind_horizon(cx, cy) {
                            why[2] += 1;
                            continue;
                        }
                        // And the whole footprint clear of the belt (B58d).
                        let mut shape: Vec<(i32, i32)> = Vec::new();
                        for across in -1..=width {
                            shape.push(corner(0, across));
                            shape.push(corner(len - 1, across));
                        }
                        if !self.clear_of_belt(&shape, 8) {
                            why[2] += 1;
                            continue;
                        }
                        // Inside the picture (a generous margin, so the strip and
                        // its apron are in frame together).
                        if !self.in_view(cx, cy, 64) {
                            why[3] += 1;
                            continue;
                        }
                        // Clear of the railway too: an airfield across the line
                        // is a level crossing with aeroplanes (B58).
                        let near_rail = self.rail.line.iter().any(|&(lx, ly)| {
                            (lx - cx).abs() <= 10 && (ly - cy).abs() <= 12
                        });
                        if near_rail {
                            why[4] += 1;
                            continue;
                        }
                        // The whole airfield on flat, dry, open ground: the strip
                        // with its shoulders, the taxiway, the apron, the terminal
                        // and the hangar (D73: no runway through hills, water or
                        // town).
                        let mut flat = 0;
                        let mut worst = 0u8;
                        let mut ok = true;
                        'area: for across in -1..width + AIRPORT_DEPTH {
                            for i in 0..len {
                                let (tx, ty) = corner(i, across);
                                if !self.map.in_bounds(tx, ty) {
                                    ok = false;
                                    break 'area;
                                }
                                let t = self.map.at(tx, ty);
                                // B99c: nor on the highway (it is laid along the
                                // same frame edge the airfield wants, and the
                                // runway never gets to run down a motorway).
                                if t.is_water()
                                    || t.relief() > 3
                                    || t.kind.is_lane()
                                    || t.aux & crate::carculture::AUX_HIGHWAY != 0
                                {
                                    ok = false;
                                    break 'area;
                                }
                                worst = worst.max(t.relief());
                                flat += t.relief() as i32;
                            }
                        }
                        if !ok {
                            why[5] += 1;
                            continue;
                        }
                        // Open country: the airfield must not be carved out of a
                        // town (B58c: the first site at the real frame size had
                        // skyscrapers down both sides of the runway). Nothing
                        // built on the footprint, and few plots around it.
                        let mut on_it = false;
                        'lots: for across in -1..=width {
                            for i in 0..len {
                                let (tx, ty) = corner(i, across);
                                if self.lots.iter().any(|l| l.covers(tx, ty)) {
                                    on_it = true;
                                    break 'lots;
                                }
                            }
                        }
                        if on_it {
                            why[5] += 1;
                            continue;
                        }
                        let mut near = 0;
                        for l in &self.lots {
                            let (lx, ly) = (l.x + l.w / 2, l.y + l.h / 2);
                            if (lx - cx).abs() <= 14 && (ly - cy).abs() <= 14 {
                                near += 1;
                            }
                        }
                        if near > 24 {
                            why[1] += 1;
                            continue;
                        }
                        // The terminal site has to reach a street, or the
                        // airport is unreachable (D73).
                        let (ax, ay) = corner(2, width + AIRPORT_DEPTH - 1);
                        let Some(link) = self.nearest_roadish((ax, ay), 8) else {
                            why[6] += 1;
                            continue;
                        };
                        let Some(access) = crate::grid::route(
                            &self.map,
                            (ax, ay),
                            link,
                            false,
                            self.seed ^ 0xA17,
                            &std::collections::BTreeSet::new(),
                        ) else {
                            why[7] += 1;
                            continue;
                        };
                        // A dead-flat site wins (the strip is graded, but a site
                        // already flat needs no embankment), then the far edge,
                        // then a short drive.
                        let score =
                            flat * 4 + worst as i32 * 12 - (d as f32 * 2.0) as i32
                                + access.len() as i32 * 2;
                        if best.as_ref().map(|(_, b)| score < *b).unwrap_or(true) {
                            let ap = self.airport_at(axis, flip, x, y, len, access);
                            best = Some((ap, score));
                        }
                    }
                }
            }
            if dbg {
                eprintln!("airfield len {len}: rejects dist {} core {} horizon {} view {} rail {} ground {} link {} route {}",
                    why[0], why[1], why[2], why[3], why[4], why[5], why[6], why[7]);
            }
              }
            if let Some((ap, _)) = best {
                self.clear_airport_plots(&ap);
                // The strip and its shoulders are graded flat (D34/D73), with
                // the trees off them: a runway through a wood is not a runway.
                let mut ground = ap.runway_tiles();
                ground.extend(ap.shoulders());
                ground.extend(ap.taxiway.iter().copied());
                ground.extend(ap.ground_tiles());
                // The margin is cleared of plots and trees too, and mown rather
                // than built on (B58c: the runway was a street between towers).
                let fence = ap.fence();
                let before = self.lots.len();
                self.lots.retain(|l| {
                    !(l.y..l.y + l.h).any(|ty| (l.x..l.x + l.w).any(|tx| fence.contains(&(tx, ty))))
                });
                let cleared = before - self.lots.len();
                if cleared > 0 {
                    self.events.push(crate::Event {
                        time: self.time,
                        text: format!("the airfield fence takes its margin: {cleared} plots cleared"),
                    });
                }
                self.clear_props(&fence);
                self.grade_ground(&ground);
                // Trees come out of the fence line as well: a canopy leaning
                // over the apron reads as a tree growing out of concrete.
                let mut cleared = ground.clone();
                for &(x, y) in &ground {
                    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1), (1, -1), (-1, 1)] {
                        if self.map.in_bounds(x + dx, y + dy) {
                            cleared.push((x + dx, y + dy));
                        }
                    }
                }
                self.clear_props(&cleared);
                self.events.push(crate::Event {
                    time: self.time,
                    text: format!("{} cleared at the edge of town", ap.label()),
                });
                return Some(ap);
            }
        }
        None
    }

    /// Level a set of tiles to one height: the civil work that makes a strip a
    /// strip instead of a staircase (D34). The ground either side keeps its own
    /// level — that step is the runway's embankment.
    fn grade_ground(&mut self, tiles: &[(i32, i32)]) {
        if tiles.is_empty() {
            return;
        }
        let mut levels: Vec<u8> = tiles
            .iter()
            .map(|&(x, y)| self.map.at(x, y).min_level())
            .collect();
        levels.sort_unstable();
        let level = levels[levels.len() / 2];
        for &(x, y) in tiles {
            let t = self.map.at(x, y);
            if t.corner == [level; 4] {
                continue;
            }
            let mut nt = t;
            nt.corner = [level; 4];
            if nt.kind.is_water() {
                continue;
            }
            if nt.kind != map::Kind::Grass && !nt.kind.is_work_surface() {
                nt.kind = map::Kind::Grass;
            }
            self.write(x, y, nt);
        }
    }

    /// The airfield geometry for a chosen strip: two rows of runway, grass
    /// shoulders, a taxiway, an apron with stands, a glazed terminal, a hangar,
    /// a tower, the windsock and the access drive (B58c).
    fn airport_at(&mut self, axis: u8, flip: i32, x: i32, y: i32, len: i32, access: Vec<(i32, i32)>) -> Airport {
        let w = AIRPORT_WIDTH;
        let at = |i: i32, across: i32| -> (i32, i32) {
            if axis == 0 {
                (x + i, y + flip * across)
            } else {
                (x + flip * across, y + i)
            }
        };
        let mut runway = Vec::with_capacity(len as usize);
        for i in 0..len {
            // The centre row of the pavement: the line aircraft follow.
            runway.push(at(i, (w - 1) / 2));
        }
        let mut taxiway = Vec::with_capacity(len as usize);
        for i in 0..len {
            taxiway.push(at(i, w + 1));
        }
        // The apron: two rows deep in front of the terminal, six tiles long at
        // the threshold end.
        let (ax, ay) = at(1, w + 2);
        let apron = Zone {
            x: ax.min(at(6, w + 3).0),
            y: ay.min(at(6, w + 3).1),
            w: if axis == 0 { 6 } else { 2 },
            h: if axis == 0 { 2 } else { 6 },
        };
        let (tx, ty) = at(1, w + 4);
        let terminal = Zone {
            x: tx.min(at(4, w + 5).0),
            y: ty.min(at(4, w + 5).1),
            w: if axis == 0 { 4 } else { 2 },
            h: if axis == 0 { 2 } else { 4 },
        };
        let (hx, hy) = at(6, w + 2);
        let hangar = Some(Zone {
            x: hx.min(at(8, w + 4).0),
            y: hy.min(at(8, w + 4).1),
            w: if axis == 0 { 3 } else { 3 },
            h: if axis == 0 { 3 } else { 3 },
        });
        // The tower stands beside the terminal's far end, on its own ground (a
        // tower inside the hall is a chimney, B58c).
        let (twx, twy) = at(5, w + 4);
        let tower = Some((
            twx,
            twy,
            if axis == 0 { 3u8 } else { 1u8 },
        ));
        let gate = at(2, w + 2);
        let windsock = at(1, -1);
        let parked = vec![gate, at(4, w + 2)];
        Airport {
            stage: AirportStage::Strip,
            runway,
            width: w,
            taxiway,
            terminal,
            apron,
            hangar,
            tower,
            gate,
            windsock,
            access,
            parked,
            paved_at: f64::INFINITY,
        }
    }

    /// Grow the airfield in place: a longer runway (at its far end, so the
    /// threshold, the terminal and the drive stay put), graded and cleared, and
    /// the stage's buildings. The apron and terminal are part of the footprint
    /// from the start, so nothing has to move when the era arrives.
    fn grow_airport(&mut self, want: AirportStage) {
        let Some(mut ap) = self.rail.airport.take() else {
            return;
        };
        if want <= ap.stage {
            ap.stage = want;
            self.rail.airport = Some(ap);
            return;
        }
        if want >= AirportStage::Regional && ap.paved_at.is_infinite() {
            ap.paved_at = self.time;
        }
        let extra = match want {
            AirportStage::Regional => 3,
            AirportStage::International => 7,
            AirportStage::Strip => 0,
        };
        let (dx, dy) = ap.dir();
        let w = ap.width.max(1);
        let perp = if dx != 0 { (0, 1) } else { (1, 0) };
        let mut grown: Vec<(i32, i32)> = Vec::new();
        for _ in 0..extra {
            let (lx, ly) = *ap.runway.last().unwrap();
            let (nx, ny) = (lx + dx, ly + dy);
            // The whole new row has to be dry, free of streets and flat enough
            // to grade; otherwise the runway stays as long as the land allows.
            let mut ok = true;
            for r in 0..w {
                let (tx, ty) = (nx + perp.0 * r, ny + perp.1 * r);
                if !self.map.in_bounds(tx, ty) {
                    ok = false;
                    break;
                }
                let t = self.map.at(tx, ty);
                if t.is_water() || t.kind.is_lane() {
                    ok = false;
                    break;
                }
            }
            if !ok {
                break;
            }
            ap.runway.push((nx, ny));
            if let Some((mx, my)) = ap.taxiway.last().copied() {
                let (tx, ty) = (mx + dx, my + dy);
                if self.map.in_bounds(tx, ty) && !self.map.at(tx, ty).is_water() {
                    ap.taxiway.push((tx, ty));
                    // The extended taxiway takes its ground too: a tree on it
                    // is the same fault as a house on the runway (B58c).
                    grown.push((tx, ty));
                }
            }
            for r in 0..w {
                grown.push((nx + perp.0 * r, ny + perp.1 * r));
                grown.push((nx + perp.0 * r - perp.0, ny + perp.1 * r - perp.1));
                grown.push((nx + perp.0 * r + perp.0, ny + perp.1 * r + perp.1));
            }
        }
        ap.stage = want;
        // The extension takes its ground: plots and trees off it, and graded.
        if !grown.is_empty() {
            let before = self.lots.len();
            self.lots.retain(|l| {
                !(l.y..l.y + l.h)
                    .any(|ty| (l.x..l.x + l.w).any(|tx| grown.contains(&(tx, ty))))
            });
            let cleared = before - self.lots.len();
            if cleared > 0 {
                self.events.push(crate::Event {
                    time: self.time,
                    text: format!("the runway is extended: {cleared} plots cleared"),
                });
            }
            self.grade_ground(&grown);
            self.clear_props(&grown);
        }
        self.rail.airport = Some(ap);
    }

    /// The airfield takes its ground: the plots inside its footprint are
    /// cleared (like the railway corridor and the port quay), so no house ever
    /// stands on a runway.
    fn clear_airport_plots(&mut self, ap: &Airport) {
        let mut taken = ap.runway_tiles();
        taken.extend(ap.shoulders());
        taken.extend(ap.taxiway.iter().copied());
        taken.extend(ap.ground_tiles());
        let before = self.lots.len();
        self.lots
            .retain(|l| !(l.y..l.y + l.h).any(|ty| (l.x..l.x + l.w).any(|tx| taken.contains(&(tx, ty)))));
        let cleared = before - self.lots.len();
        if cleared > 0 {
            self.events.push(crate::Event {
                time: self.time,
                text: format!("the airfield takes its ground: {cleared} plots cleared"),
            });
        }
    }

    /// The nearest lane, or failing that a street tile, within `r`.
    fn nearest_roadish(&self, p: (i32, i32), r: i32) -> Option<(i32, i32)> {
        for ring in 1..=r {
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let (x, y) = (p.0 + dx, p.1 + dy);
                    if self.map.in_bounds(x, y) {
                        let k = self.map.at(x, y).kind;
                        if k.is_lane() || k == Kind::Sidewalk {
                            return Some((x, y));
                        }
                    }
                }
            }
        }
        None
    }

    /// The subway: the longest downtown street, entrances on the pavement,
    /// grates along it. The works (not the line) are what the eras show.
    fn plan_subway(&mut self, started: f64) -> Option<Subway> {
        let (fx, fy) = self.founding;
        let span = self.span();
        let mut best: Option<(Vec<(i32, i32)>, i32)> = None;
        for axis in [0u8, 1] {
            let outer = if axis == 0 { self.map.h } else { self.map.w };
            let inner = if axis == 0 { self.map.w } else { self.map.h };
            for row in 2..outer - 2 {
                let mut run: Vec<(i32, i32)> = Vec::new();
                for i in 2..inner - 2 {
                    let (x, y) = if axis == 0 { (i, row) } else { (row, i) };
                    let t = self.map.at(x, y);
                    if t.kind.is_lane() {
                        run.push((x, y));
                    } else {
                        if run.len() >= 12 {
                            let mid = run[run.len() / 2];
                            // Downtown or the old core, not a suburban lane.
                            let d = (mid.0 - fx).abs() + (mid.1 - fy).abs();
                            if (d as f32) < span * 0.5 && self.in_view(mid.0, mid.1, 30) {
                                let score = run.len() as i32 * 4 - d;
                                if best.as_ref().map(|(_, b)| score > *b).unwrap_or(true) {
                                    best = Some((run.clone(), score));
                                }
                            }
                        }
                        run.clear();
                    }
                }
                if run.len() >= 12 {
                    let mid = run[run.len() / 2];
                    let d = (mid.0 - fx).abs() + (mid.1 - fy).abs();
                    if (d as f32) < span * 0.5 && self.in_view(mid.0, mid.1, 30) {
                        let score = run.len() as i32 * 4 - d;
                        if best.as_ref().map(|(_, b)| score > *b).unwrap_or(true) {
                            best = Some((run, score));
                        }
                    }
                }
            }
        }
        let (line, _) = best?;
        // Stations every sixteen tiles, starting six in.
        let mut stations = Vec::new();
        let mut grates = Vec::new();
        let mut i = 6;
        while i < line.len() - 4 {
            let at = line[i as usize];
            let along_x = at.1 == line[0].1;
            let sides: [((i32, i32), u8); 4] = if along_x {
                [((at.0, at.1 + 1), 2), ((at.0, at.1 - 1), 3), ((at.0 + 1, at.1), 0), ((at.0 - 1, at.1), 1)]
            } else {
                [((at.0 + 1, at.1), 0), ((at.0 - 1, at.1), 1), ((at.0, at.1 + 1), 2), ((at.0, at.1 - 1), 3)]
            };
            for ((ex, ey), side) in sides {
                if !self.map.in_bounds(ex, ey) {
                    continue;
                }
                let t = self.map.at(ex, ey);
                if t.is_water() || t.kind.is_lane() || t.relief() > 1 {
                    continue;
                }
                stations.push(SubwayStation {
                    at,
                    entrance: (ex, ey, side),
                });
                break;
            }
            i += 16;
        }
        // Grates every fifth tile, on the pavement beside the cut.
        for (k, &(x, y)) in line.iter().enumerate() {
            if k % 5 != 2 {
                continue;
            }
            let along_x = y == line[0].1;
            let (gx, gy) = if along_x {
                if self.map.at(x, y + 1).kind == Kind::Sidewalk {
                    (x, y + 1)
                } else {
                    (x, y - 1)
                }
            } else if self.map.at(x + 1, y).kind == Kind::Sidewalk {
                (x + 1, y)
            } else {
                (x - 1, y)
            };
            if self.map.in_bounds(gx, gy) && !self.map.at(gx, gy).is_water() {
                grates.push((gx, gy));
            }
        }
        if stations.is_empty() {
            return None;
        }
        Some(Subway {
            started,
            line,
            stations,
            grates,
        })
    }

    /// The harbour: a pier out into the water near the city, a ferry route and
    /// two moored fishing boats. Only on a coast or a big lake (D73: no port in
    /// a puddle).
    fn plan_harbour(&mut self) -> Option<Harbour> {
        let (fx, fy) = self.founding;
        let mut best: Option<(Harbour, i32)> = None;
        // A quay needs a *straight* run of shore: the same side wet for six
        // tiles or more, with land behind it (B58c).
        for axis in [0u8, 1] {
            for side in [1i32, -1] {
                let (dx, dy) = if axis == 0 { (0, side) } else { (side, 0) };
                let (ax, ay) = if axis == 0 { (1, 0) } else { (0, 1) };
                for y in 4..self.map.h - 4 {
                    for x in 4..self.map.w - 4 {
                        // Measure the run of straight shore from here.
                        let mut run: Vec<(i32, i32)> = Vec::new();
                        let mut kind = WaterKind::None;
                        for k in 0..10 {
                            let (tx, ty) = (x + ax * k, y + ay * k);
                            if !self.map.in_bounds(tx, ty) {
                                break;
                            }
                            let t = self.map.at(tx, ty);
                            let (wx, wy) = (tx + dx, ty + dy);
                            let (bx, by) = (tx - dx, ty - dy);
                            if t.is_water()
                                || !self.map.in_bounds(wx, wy)
                                || !self.map.at(wx, wy).is_water()
                                || !self.map.in_bounds(bx, by)
                                || self.map.at(bx, by).is_water()
                            {
                                break;
                            }
                            kind = self.map.water_kind(wx, wy);
                            run.push((tx, ty));
                        }
                        if run.len() < 5 {
                            continue;
                        }
                        // A coast, a big lake or a navigable river: on these maps
                        // the sea is often outside the frame, and a river quay is
                        // a real quay (B58d: no straight *sea* shore survived the
                        // land-line rule on any of the five seeds).
                        if !matches!(kind, WaterKind::Sea | WaterKind::Lake | WaterKind::River) {
                            continue;
                        }
                        let mid = run[run.len() / 2];
                        if !self.in_view(mid.0, mid.1, 60) {
                            continue;
                        }
                        // The *whole* harbour below the land line, with room to
                        // spare: a quay or a pier deck in the belt is cut by the
                        // horizon haze (B58d).
                        let mut shape: Vec<(i32, i32)> = run.clone();
                        for k in [1usize, 4] {
                            if k < run.len() {
                                let (bx, by) = run[k];
                                for j in 1..=3 {
                                    shape.push((bx + dx * j, by + dy * j));
                                }
                            }
                        }
                        if !self.clear_of_belt(&shape, 24) {
                            continue;
                        }
                        // Keep clear of the marina: its pontoons drawn through
                        // this quay was the first render's fault (B58c). It is a
                        // penalty, not a veto — on a small coast the only
                        // straight shore may be next to the marina.
                        let near_marina = self.rail.marina.is_some_and(|m| {
                            let (mx, my) = m.centre();
                            (mx - mid.0).abs() + (my - mid.1).abs() < 9
                        });
                        // Keep clear of the container port: two waterfronts.
                        if let Some(p) = self.rail.port {
                            let (px, py) = p.centre();
                            if (px - mid.0).abs() + (py - mid.1).abs() < 10 {
                                continue;
                            }
                        }
                        // Water for the ferry: a run along the shore two tiles
                        // out from the quay.
                        let mut ferry = Vec::new();
                        for k in 0..run.len() as i32 + 6 {
                            let (tx, ty) = (x + ax * k + dx * 2, y + ay * k + dy * 2);
                            if self.map.in_bounds(tx, ty) && self.map.at(tx, ty).is_water() {
                                ferry.push((tx, ty));
                            } else if !ferry.is_empty() {
                                break;
                            }
                        }
                        if ferry.len() < 6 {
                            continue;
                        }
                        let d = (mid.0 - fx).abs() + (mid.1 - fy).abs();
                        let open = match kind {
                            WaterKind::Sea => 10,
                            WaterKind::Lake => 6,
                            _ => 3,
                        };
                        let score = run.len() as i32 * 30
                            + open * 20
                            + ferry.len() as i32 * 4
                            - d * 2
                            - if near_marina { 60 } else { 0 };
                        if !best.as_ref().map(|(_, b)| score > *b).unwrap_or(true) {
                            continue;
                        }
                        // The piers: two straight fingers out into the water.
                        let mut piers: Vec<Vec<(i32, i32)>> = Vec::new();
                        for k in [1usize, 4] {
                            if k >= run.len() {
                                continue;
                            }
                            let (bx, by) = run[k];
                            let mut pier = Vec::new();
                            for j in 1..=3 {
                                let (px, py) = (bx + dx * j, by + dy * j);
                                if self.map.in_bounds(px, py) && self.map.at(px, py).is_water() {
                                    pier.push((px, py));
                                } else {
                                    break;
                                }
                            }
                            if pier.len() >= 2 {
                                piers.push(pier);
                            }
                        }
                        // Boats tie up alongside the piers.
                        let mut moored = Vec::new();
                        for pier in &piers {
                            if let Some(&(px, py)) = pier.last() {
                                for (mx, my) in [(px + ax, py + ay), (px - ax, py - ay)] {
                                    if self.map.in_bounds(mx, my) && self.map.at(mx, my).is_water() {
                                        moored.push((mx, my));
                                    }
                                }
                            }
                        }
                        // The fish market shed sits on the land behind the quay.
                        let (sx, sy) = run[run.len() / 2];
                        let (hx, hy) = (sx - dx, sy - dy);
                        let shed = if axis == 0 {
                            (hx - 1, hy, 3, 1)
                        } else {
                            (hx, hy - 1, 1, 3)
                        };
                        let shed_ok = (shed.1..shed.1 + shed.3).all(|ty| {
                            (shed.0..shed.0 + shed.2).all(|tx| {
                                self.map.in_bounds(tx, ty)
                                    && !self.map.at(tx, ty).is_water()
                                    && self.map.at(tx, ty).relief() <= 2
                            })
                        });
                        let cranes = vec![run[0], run[run.len() - 1]];
                        best = Some((
                            Harbour {
                                quay: run,
                                water_dir: (dx, dy),
                                piers,
                                ferry,
                                moored,
                                shed: if shed_ok { shed } else { (0, 0, 0, 0) },
                                cranes,
                            },
                            score,
                        ));
                    }
                }
            }
        }
        let (h, _) = best?;
        // The harbour takes its ground: graded flat along the quay (that is what
        // straightens a ragged shore), with its plots and trees out of the way.
        let ground = h.ground();
        self.grade_ground(&ground);
        self.clear_props(&ground);
        let before = self.lots.len();
        self.lots.retain(|l| {
            !(l.y..l.y + l.h).any(|ty| (l.x..l.x + l.w).any(|tx| ground.contains(&(tx, ty))))
        });
        let cleared = before - self.lots.len();
        self.events.push(crate::Event {
            time: self.time,
            text: if cleared > 0 {
                format!("the harbour is dredged: a straight quay, {cleared} plots cleared")
            } else {
                "the harbour is dredged: a straight quay and its piers".to_string()
            },
        });
        Some(h)
    }

    /// Where the town crosses the railway (B58/D73): a street on each side of
    /// the line, joined across it. The line keeps its own corridor, so without
    /// this there is no crossing anywhere — which the first run showed on every
    /// seed (zero crossings), and then the barriers, the lights and the waiting
    /// cars have nothing to do.
    pub(crate) fn plan_crossings(&mut self) {
        if self.rail.line.len() < 6 || self.era() < Era::City {
            return;
        }
        // Two crossings are enough, and the search is an A* across the town: it
        // runs at most once an hour, not once a frame (B58: the wallpaper calls
        // this every frame).
        if self.level_crossings().len() >= 2
            || (self.rail.crossings_tried != 0.0 && self.time - self.rail.crossings_tried < 3600.0)
        {
            return;
        }
        self.rail.crossings_tried = self.time.max(1.0);
        let mut made = 0usize;
        let line = self.rail.line.clone();
        let (fx, fy) = self.founding;
        // Two places along the line, near the town: the ends of the platform
        // stretch, where the streets are.
        let mut anchors: Vec<(i32, i32)> = Vec::new();
        for frac in [0.25f32, 0.40, 0.55, 0.70] {
            let i = ((line.len() as f32 - 1.0) * frac) as usize;
            anchors.push(line[i]);
        }
        // And the line tile nearest the founding, as a fallback.
        if let Some(&p) = line
            .iter()
            .min_by_key(|&&(x, y)| (x - fx).abs() + (y - fy).abs())
        {
            anchors.push(p);
        }
        for &(rx, ry) in &anchors {
            if made >= 2 {
                break;
            }
            // The line's local direction, and the way across it.
            let dir = if line.contains(&(rx + 1, ry)) {
                (1i32, 0i32)
            } else if line.contains(&(rx, ry + 1)) {
                (0, 1)
            } else {
                (1, 0)
            };
            let perp = if dir.0 != 0 { (0i32, 1i32) } else { (1, 0) };
            // The nearest street on one side of the line (walking out from the
            // corridor), up to sixteen tiles: the town's streets are a block or
            // two back from the rails.
            let lane_near = |sim: &Sim, sign: i32| -> Option<(i32, i32)> {
                for k in 2..=16i32 {
                    let (x, y) = (rx + perp.0 * sign * k, ry + perp.1 * sign * k);
                    if !sim.map.in_bounds(x, y) {
                        return None;
                    }
                    let t = sim.map.at(x, y);
                    if t.is_water() {
                        return None;
                    }
                    if t.kind.is_lane() {
                        return Some((x, y));
                    }
                }
                None
            };
            let (Some(a), Some(b)) = (lane_near(self, -1), lane_near(self, 1)) else {
                continue;
            };
            let Some(path) = crate::grid::route(
                &self.map,
                a,
                b,
                false,
                self.seed ^ 0xC205,
                &std::collections::BTreeSet::new(),
            ) else {
                continue;
            };
            if path.len() < 3 || !path.iter().any(|p| line.contains(p)) {
                continue;
            }
            // The new street: a crossing where it meets the rails.
            for &(x, y) in &path {
                let t = self.map.at(x, y);
                if t.is_water() || t.kind == Kind::Paved {
                    continue;
                }
                let mut nt = t;
                nt.kind = if line.contains(&(x, y)) {
                    Kind::Paved
                } else if t.kind.is_lane() {
                    continue;
                } else {
                    Kind::Road
                };
                self.write(x, y, nt);
            }
            made += 1;
            self.events.push(crate::Event {
                time: self.time,
                text: "the town crosses the railway: barriers and lights at the crossing".into(),
            });
        }
    }
}

// ----------------------------------------------------------------- movers

impl Sim {
    /// Spawn the buses, ferries and aircraft the timetable calls for, and keep
    /// the flying ones at the right height above the runway.
    pub(crate) fn transit_step(&mut self, dt: f64) {
        let _ = dt;
        // Aircraft altitude follows their position along the runway (take-off
        // climbs, landing descends to the touchdown point), so a plane is only
        // ever drawn at the height it would really be at.
        let runway_len = self
            .rail
            .airport
            .as_ref()
            .map(|a| a.runway.len() as f32)
            .unwrap_or(1.0)
            .max(1.0);
        for m in &mut self.rail.movers {
            if m.kind != crate::rail::MoverKind::Aircraft {
                continue;
            }
            let f = (m.travelled / runway_len).clamp(0.0, 1.0);
            m.lift = match m.mode {
                // Take-off: hold the ground until the rotation point, then climb.
                0 => {
                    if f < 0.45 {
                        0
                    } else {
                        let c = (f - 0.45) / 0.55;
                        (c * c * 150.0) as i32
                    }
                }
                // Landing: down the glide path to the touchdown, then the ground.
                _ => {
                    if f < 0.42 {
                        (150.0 * (1.0 - f / 0.42)) as i32
                    } else {
                        0
                    }
                }
            };
        }
        // Cranes: a trolley cycling out over the ship and back, forever.
        for m in &mut self.rail.movers {
            if m.kind == crate::rail::MoverKind::Crane {
                m.travelled = m.travelled.rem_euclid(1.0);
            }
        }

        let era = self.era();
        let clock = self.rail.clock;
        let hod = (self.time / 3600.0).rem_euclid(24.0);
        let night = !(5.5..23.0).contains(&hod);

        // Buses: two per line by day, one after dark, none in the small hours.
        if !self.rail.bus_lines.is_empty() && self.rail.clock >= self.rail.next_bus {
            self.rail.next_bus = clock + 40.0;
            let lines = self.rail.bus_lines.clone();
            for (i, line) in lines.iter().enumerate() {
                let running = self
                    .rail
                    .movers
                    .iter()
                    .filter(|m| m.kind == crate::rail::MoverKind::Bus && m.line as usize == i)
                    .count();
                let want = if hod < 5.5 || hod > 23.0 { 0 } else { 2 };
                if running < want {
                    self.rail.movers.push(Mover {
                        kind: crate::rail::MoverKind::Bus,
                        route: line.route.clone(),
                        travelled: (self.seed % 17) as f32,
                        speed: 1.25,
                        shuttle: false,
                        back: false,
                        cars: 0,
                        lift: 0,
                        dwell: 0.0,
                        cooldown: 0.0,
                        set: 0,
                        line: i as u8,
                        mode: 0,
                        phase: 0.0,
                    });
                }
            }
        }

        // Ferries: a crossing every few minutes while people are about.
        if let Some(h) = &self.rail.harbour {
            let ferries = self.rail.movers.iter().filter(|m| m.line == 0 && m.kind == crate::rail::MoverKind::Ferry).count();
            if clock >= self.rail.next_ferry && ferries < 2 {
                self.rail.next_ferry = clock + if night { 900.0 } else { 260.0 };
                let ferry = h.ferry.clone();
                self.push_boat(Mover {
                    kind: crate::rail::MoverKind::Ferry,
                    route: ferry,
                    travelled: 0.0,
                    speed: 0.8,
                    shuttle: true,
                    back: false,
                    cars: 0,
                    lift: 0,
                    dwell: 0.0,
                    cooldown: 0.0,
                    set: self.ferry_set(),
                    line: 0,
                    mode: 0,
                    phase: 0.0,
                });
            }
        }

        // Aircraft: a departure then an arrival, in daylight, from the city era.
        if era >= Era::City && self.rail.airport.is_some() && clock >= self.rail.next_aircraft {
            self.rail.next_aircraft = clock + if night { 420.0 } else { 150.0 };
            self.spawn_aircraft();
        }
    }

    fn spawn_aircraft(&mut self) {
        let Some(ap) = self.rail.airport.clone() else {
            return;
        };
        if ap.runway.len() < 4 {
            return;
        }
        let take_off = self.rng(0xA1).chance(0.5);
        let runway_fwd = ap.runway.clone();
        let mut runway_rev = ap.runway.clone();
        runway_rev.reverse();
        let mut route = if take_off {
            let mut r = vec![ap.gate];
            r.extend(ap.taxiway.iter().copied());
            r.extend(runway_fwd.iter().copied());
            r
        } else {
            let mut r = runway_rev;
            let mut taxi = ap.taxiway.clone();
            taxi.reverse();
            r.extend(taxi);
            r.push(ap.gate);
            r
        };
        // The taxiway may overlap the runway at the threshold: dedupe the seam.
        route.dedup();
        if route.len() < 6 {
            return;
        }
        self.rail.movers.push(Mover {
            kind: crate::rail::MoverKind::Aircraft,
            route,
            travelled: 0.0,
            speed: 4.2,
            shuttle: false,
            back: false,
            cars: 0,
            lift: if take_off { 0 } else { 150 },
            dwell: 0.0,
            cooldown: 0.0,
            set: if ap.stage == AirportStage::Strip { 1 } else { 0 },
            line: 0,
            mode: if take_off { 0 } else { 1 },
            phase: 0.0,
        });
    }

    /// The barriers: every level crossing with the phase its arms are at (0 =
    /// open, 1 = down). A crossing closes as the train's tail approaches and
    /// stays down until the tail has cleared it.
    pub fn crossings_live(&self) -> Vec<((i32, i32), f32)> {
        let line = &self.rail.line;
        if line.len() < 3 {
            return Vec::new();
        }
        // Arc position of every line tile, so a crossing can be compared with a
        // train's distance along the same line.
        let mut arc: std::collections::BTreeMap<(i32, i32), f32> =
            std::collections::BTreeMap::new();
        let mut acc = 0.0;
        for (i, &p) in line.iter().enumerate() {
            arc.insert(p, acc);
            if let Some(w) = line.get(i + 1) {
                acc += (w.0 - p.0).abs() as f32 + (w.1 - p.1).abs() as f32;
            }
        }
        let trains: Vec<(f32, f32)> = self
            .rail
            .movers
            .iter()
            .filter(|m| m.kind == crate::rail::MoverKind::Train)
            .map(|m| (m.travelled, m.travelled - m.cars as f32))
            .collect();
        let mut out = Vec::new();
        for &(x, y) in line {
            let k = self.map.at(x, y).kind;
            if !(k == Kind::Paved || k == Kind::Road) {
                continue;
            }
            let Some(&cp) = arc.get(&(x, y)) else {
                continue;
            };
            let mut phase = 0.0f32;
            for &(head, tail) in &trains {
                if head < cp {
                    // Approaching: the barrier falls over the last eight tiles.
                    let d = cp - head;
                    if d < 8.0 {
                        phase = phase.max(1.0 - d / 8.0);
                    }
                } else if cp > tail {
                    phase = 1.0;
                }
            }
            out.push(((x, y), phase));
        }
        out
    }

    /// The crossings whose barriers are down (traffic waits for these).
    pub fn shut_crossings(&self) -> Vec<(i32, i32)> {
        self.crossings_live()
            .into_iter()
            .filter(|(_, p)| *p > 0.45)
            .map(|(t, _)| t)
            .collect()
    }

    /// Is this crossing shut? Traffic waits for it (D73: cars wait at the
    /// barriers, they do not drive through a passing train).
    pub fn crossing_shut(&self, x: i32, y: i32) -> bool {
        self.shut_crossings().contains(&(x, y))
    }

    /// The bus stops (tile + the pavement side the shelter is on).
    pub fn bus_stops(&self) -> Vec<(i32, i32, u8)> {
        let mut out = Vec::new();
        for line in &self.rail.bus_lines {
            for (k, &(x, y)) in line.stops.iter().enumerate() {
                out.push((x, y, line.sides.get(k).copied().unwrap_or(0)));
            }
        }
        out
    }

    /// The subway's works at this instant, for the renderer: the stage, the
    /// crew positions along the cut and whether the entrances are open.
    pub fn subway_works(&self) -> Option<(u8, f32)> {
        let s = self.rail.subway.as_ref()?;
        Some(subway_stage(s.started, self.time))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pace;

    /// A city grown the way the wallpaper grows one: the transport is planned
    /// at the town era and the city grows around it after.
    fn city(seed: u64, days: f64) -> Sim {
        let vp = map::world_for_viewport(1920, 1080);
        let focus = map::screen_to_tile(&vp, 960.0, 540.0);
        let mut s = Sim::with_focus(seed, Pace::Month, vp.w, vp.h, focus);
        s.set_viewport(vp.origin.0, vp.origin.1, 1920, 1080);
        s.run_to(8.0 * 86400.0);
        s.plan_rail();
        s.run_to(days * 86400.0);
        s.plan_rail();
        s
    }

    /// The planner's survey (run with `--nocapture` when tuning the thresholds).
    #[test]
    fn survey_what_gets_planned() {
        for seed in [1u64, 3, 7, 9, 11] {
            for days in [10.0f64, 30.0, 60.0] {
                let s = city(seed, days);
                println!(
                    "seed {seed} day {days}: era {:?} pop {} line {} twin {} build {:?} buses {} \
                     stops {} airport {:?} harbour {} subway {:?} crossings {}",
                    s.era(),
                    s.residents(),
                    s.rail.line.len(),
                    s.rail.twin.len(),
                    s.rail.station_build.as_ref().map(|b| (b.kind, b.building, b.platforms.len())),
                    s.rail.bus_lines.len(),
                    s.bus_stops().len(),
                    s.rail.airport.as_ref().map(|a| (a.stage, a.runway.len())),
                    s.rail.harbour.as_ref().map(|h| h.piers.len()).unwrap_or(0),
                    s.rail.subway.as_ref().map(|w| w.line.len()),
                    s.level_crossings().len(),
                );
            }
        }
    }

    /// D73: an airfield is flat, dry land at the city's edge — never a runway
    /// through a hill, the water or downtown — and it is joined to the street.
    #[test]
    fn the_airfield_is_flat_dry_and_off_the_core() {
        let mut found = 0;
        for seed in [1u64, 3, 7, 9, 11] {
            for days in [10.0f64, 45.0] {
                let s = city(seed, days);
                let Some(ap) = &s.rail.airport else { continue };
                found += 1;
                assert!(ap.runway.len() >= 8, "seed {seed}: {} tiles of runway", ap.runway.len());
                let (fx, fy) = s.founding;
                let centre = ap.centre();
                let d = (centre.0 - fx).abs() + (centre.1 - fy).abs();
                assert!(
                    d as f32 >= s.span() * 0.4,
                    "seed {seed}: the airfield is {d} tiles from the founding — in the town"
                );
                assert!(
                    !matches!(s.district_at(centre.0, centre.1), District::Downtown | District::Core),
                    "seed {seed}: the runway runs through downtown"
                );
                for &(x, y) in &ap.runway {
                    let t = s.map.at(x, y);
                    assert!(!t.is_water(), "seed {seed}: runway tile {x},{y} is water");
                    // The strip is graded flat; a neighbour levelled later can
                    // tilt a shoulder corner, so allow one level of tolerance.
                    assert!(t.relief() <= 3, "seed {seed}: runway tile {x},{y} is a cliff");
                    assert!(!t.kind.is_lane(), "seed {seed}: the runway is a street at {x},{y}");
                }
                for &(x, y) in &ap.taxiway {
                    assert!(!s.map.at(x, y).is_water(), "seed {seed}: taxiway in water at {x},{y}");
                }
                // The terminal reaches the street network (D73: connected).
                assert!(!ap.access.is_empty(), "seed {seed}: the airfield has no access drive");
                let last = *ap.access.last().unwrap();
                let t = s.map.at(last.0, last.1);
                assert!(
                    t.kind.is_lane() || t.kind == map::Kind::Sidewalk,
                    "seed {seed}: the access drive ends at {:?}, not a street",
                    t.kind
                );
                // And nothing of it is cut by the horizon.
                assert!(!s.behind_horizon(centre.0, centre.1), "seed {seed}: airfield behind the land line");
                // Nothing else is built on it: no plot, no street, no tree
                // (B58c item 1: houses must not stand in the runway).
                let mut footprint = ap.runway_tiles();
                footprint.extend(ap.shoulders());
                footprint.extend(ap.taxiway.iter().copied());
                footprint.extend(ap.ground_tiles());
                for &(x, y) in &footprint {
                    for l in &s.lots {
                        assert!(
                            !l.covers(x, y),
                            "seed {seed}: a plot ({:?}) stands on the airfield at {x},{y}",
                            l.kind
                        );
                    }
                    assert!(
                        s.map.at(x, y).prop == map::Prop::None,
                        "seed {seed}: a prop stands on the airfield at {x},{y}"
                    );
                }
            }
        }
        assert!(found > 0, "no seed ever grew an airfield");
    }

    /// D72: the airfield grows with the eras — a grass strip, then paving and a
    /// terminal, then the international apron and tower. One world, run through
    /// its life: the strip is planned once and only ever grows (comparing three
    /// separate worlds compared three different airfields).
    #[test]
    fn the_airfield_grows_with_the_eras() {
        let mut seen = Vec::new();
        for seed in [1u64, 7, 11] {
            let vp = map::world_for_viewport(1920, 1080);
            let focus = map::screen_to_tile(&vp, 960.0, 540.0);
            let mut s = Sim::with_focus(seed, Pace::Month, vp.w, vp.h, focus);
            s.set_viewport(vp.origin.0, vp.origin.1, 1920, 1080);
            s.run_to(8.0 * 86400.0);
            s.plan_rail();
            let mut last = 0;
            for days in [10.0f64, 30.0, 60.0] {
                s.run_to(days * 86400.0);
                s.plan_rail();
                if let Some(ap) = &s.rail.airport {
                    seen.push((seed, days, ap.stage, ap.runway.len()));
                    assert!(
                        ap.runway.len() >= last,
                        "seed {seed}: the runway shrank from {last} to {}",
                        ap.runway.len()
                    );
                    last = ap.runway.len();
                }
            }
        }
        assert!(seen.iter().any(|(_, _, st, _)| *st == AirportStage::Strip), "no grass strip: {seen:?}");
        assert!(seen.iter().any(|(_, _, st, _)| *st >= AirportStage::Regional), "never regional: {seen:?}");
        assert!(seen.iter().any(|(_, _, st, _)| *st == AirportStage::International), "never international: {seen:?}");
    }

    /// B58: buses run a loop of their own line, call at their own stops, and
    /// are out in the day and in the shed at three in the morning.
    #[test]
    fn buses_run_their_lines_and_stop_at_their_stops() {
        let mut s = city(3, 30.0);
        assert!(!s.rail.bus_lines.is_empty(), "a city with no bus line");
        for line in &s.rail.bus_lines {
            assert!(line.route.len() > 12, "a bus route of {} tiles", line.route.len());
            for stop in &line.stops {
                assert!(line.route.contains(stop), "a stop off its own route: {stop:?}");
                assert!(s.map.at(stop.0, stop.1).kind.is_lane(), "a stop in a field: {stop:?}");
            }
        }
        // Daytime service: a bus is dispatched and calls at a stop.
        s.set_clock_hour(8.0);
        let mut buses = 0;
        let mut dwelt = 0;
        for _ in 0..3000 {
            s.transport_step(0.25);
            let live: Vec<_> = s
                .transport()
                .iter()
                .filter(|m| m.kind == crate::rail::MoverKind::Bus)
                .collect();
            buses = buses.max(live.len());
            if live.iter().any(|m| m.dwell > 0.0) {
                dwelt += 1;
            }
        }
        assert!(buses >= 1, "no bus ever ran");
        assert!(dwelt > 5, "no bus ever stopped at a stop ({dwelt})");
        // Night: the depot, not the street.
        s.set_clock_hour(2.0);
        for _ in 0..400 {
            s.transport_step(0.25);
        }
        for _ in 0..2000 {
            s.transport_step(0.25);
            let live = s
                .transport()
                .iter()
                .filter(|m| m.kind == crate::rail::MoverKind::Bus)
                .count();
            if live == 0 {
                return;
            }
        }
        panic!("buses still running at 02:00");
    }

    /// B58: the subway is built visibly (hoardings → open cut → deck →
    /// entrances → open), its entrances are on the pavement, and people use
    /// them at rush hour only once they are open.
    #[test]
    fn the_subway_is_built_visibly_then_opens() {
        let mut s = city(1, 30.0);
        let Some(sub) = s.rail.subway.clone() else {
            panic!("a 30-day city of this size has no subway");
        };
        // The cut runs along a street, the entrances stand beside it.
        for &(x, y) in &sub.line {
            assert!(s.map.at(x, y).kind.is_lane(), "the cut is not in a street at {x},{y}");
        }
        assert!(!sub.stations.is_empty(), "no subway station");
        for st in &sub.stations {
            let (ex, ey, _) = st.entrance;
            assert!(s.map.at(ex, ey).kind != map::Kind::Water, "an entrance in the water");
            assert!(!sub.line.contains(&(ex, ey)), "the entrance is in the cut");
            assert!(
                (ex - st.at.0).abs() + (ey - st.at.1).abs() <= 2,
                "an entrance {:?} is not beside its platform {:?}",
                st.entrance,
                st.at
            );
        }
        // The stages arrive in order, and the whole build is finite.
        let started = sub.started;
        let mut stages = Vec::new();
        for frac in [0.1f64, 0.3, 0.65, 0.85, 0.95, 1.1] {
            let t = started + SUBWAY_BUILD_SECS * frac;
            s.run_to(t);
            let (stage, _) = s.subway_works().unwrap();
            stages.push(stage);
            if frac < 1.0 {
                assert!(s.rail.bus_lines.iter().all(|l| !l.stops.is_empty()));
            }
        }
        assert_eq!(stages, vec![0, 1, 2, 3, 3, 4], "the works do not run in order");
        // Nobody goes in before the entrance is open; at 08:00 after, they do.
        // The entrance crowd is the subway's own rule, tested directly: the sim
        // adds figures at the stairs only once the works are open (the city's
        // ordinary walkers are a different population and would drown it out).
        let entrances: Vec<(i32, i32)> = sub
            .stations
            .iter()
            .map(|st| (st.entrance.0, st.entrance.1))
            .collect();
        let at_entrances = |figs: &[crate::people::Figure]| -> usize {
            figs.iter()
                .filter(|f| {
                    entrances
                        .iter()
                        .any(|&(ex, ey)| (f.x - ex as f32).abs() < 1.2 && (f.y - ey as f32).abs() < 1.2)
                })
                .count()
        };
        s.run_to(started + SUBWAY_BUILD_SECS * 0.5);
        s.set_clock_hour(8.0);
        let (stage, _) = s.subway_works().unwrap();
        assert!(stage < 4);
        let mut figs = Vec::new();
        s.waiting_figures(8.0, &mut figs);
        assert_eq!(at_entrances(&figs), 0, "people on the stairs of a subway that is not open");
        // A day after the works finished (pinning 08:00 rewinds to *that day's*
        // eight o'clock, so the day has to be past the end of the build).
        s.run_to(started + SUBWAY_BUILD_SECS * 2.0);
        s.set_clock_hour(8.0);
        assert_eq!(s.subway_works().unwrap().0, 4, "the works are not finished");
        let mut figs = Vec::new();
        s.waiting_figures(8.0, &mut figs);
        let after = at_entrances(&figs);
        assert!(after > 0, "the open subway has nobody on its stairs");
    }

    /// B58: a train fits its platform, and a crossing is shut while it passes —
    /// with the traffic holding at the barrier.
    #[test]
    fn trains_fit_the_platform_and_crossings_shut() {
        let mut s = city(1, 45.0);
        let platform = s
            .rail
            .station_build
            .as_ref()
            .map(|b| b.kind.platform_len())
            .expect("no station");
        let mut pax = 0;
        let mut freight = 0;
        for _ in 0..6000 {
            s.transport_step(0.5);
            for m in s.transport() {
                if m.kind != crate::rail::MoverKind::Train {
                    continue;
                }
                if m.set == 2 {
                    freight += 1;
                    assert!(m.cars >= 5, "a freight of {} wagons", m.cars);
                } else {
                    pax += 1;
                    assert_eq!(
                        m.cars,
                        platform + 1,
                        "a passenger set of {} cars at a {platform}-tile platform",
                        m.cars
                    );
                }
            }
            if pax > 0 && freight > 0 {
                break;
            }
        }
        assert!(pax > 0, "no passenger train ever ran");
        assert!(freight > 0, "no freight train ever ran");
        // A crossing shuts as a train passes, and traffic waits.
        let crossings = s.level_crossings();
        if crossings.is_empty() {
            return; // a line that never meets a street has none (see the survey)
        }
        let mut shut = false;
        for _ in 0..4000 {
            s.transport_step(0.5);
            if s.shut_crossings().iter().any(|c| crossings.contains(c)) {
                shut = true;
                break;
            }
        }
        assert!(shut, "a barrier never came down");
        // And a car whose nose is at a shut crossing does not move: a synthetic
        // trip that ends on the crossing tile makes the rule exact (D73: cars
        // wait at the barrier).
        let shut = s.shut_crossings();
        let (cx, cy) = *shut.first().expect("a barrier is down");
        let approach = [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .map(|(dx, dy)| (cx + dx, cy + dy))
            .find(|&(x, y)| s.map.at(x, y).kind.is_lane())
            .expect("the crossing has no road onto it");
        let before = s.traffic.vehicles.len();
        s.traffic.vehicles.push(crate::traffic::Vehicle {
            kind: crate::traffic::VehicleKind::Car,
            route: vec![approach, (cx, cy)],
            travelled: 0.0,
            from: approach,
            to: (cx, cy),
            pace: 1.0,
            blink: 0.0,
            started: 0.0,
            slot: None,
            variant: 0,
            style: crate::traffic::style::SALOON,
            signal: 0,
            braking: false,
            held: 0.0,
            stopped: false,
            hw: (0.0, 0.0),
            vel: 0.0,
        });
        let idx = before;
        s.traffic_step(1.0);
        let held = s
            .traffic
            .vehicles
            .get(idx)
            .map(|v| v.travelled)
            .unwrap_or(0.0);
        assert!(held < 0.01, "a car drove over a shut crossing ({held})");
    }

    /// D73: the station stands beside its track, with a platform reachable from
    /// the rails and a hall that is not on either.
    #[test]
    fn the_station_stands_beside_its_track() {
        let mut built = 0;
        for seed in [1u64, 3, 7, 9, 11] {
            let s = city(seed, 45.0);
            let Some(sb) = &s.rail.station_build else { continue };
            built += 1;
            let station = s.rail.station.expect("a build with no station tile");
            assert!(s.rail.line.contains(&station), "the station is not on the line");
            assert!(!sb.platforms.is_empty(), "seed {seed}: no platforms");
            for &(px, py, _) in &sb.platforms {
                // A platform is beside the track: one tile out for the near
                // platform, three for the one across the second track.
                let near = s.rail.line.iter().any(|&(lx, ly)| {
                    let d = (lx - px).abs() + (ly - py).abs();
                    d == 1 || d == 3
                });
                assert!(near, "seed {seed}: platform {px},{py} is not beside the track");
            }
            // The hall: off the track, off the platforms, on dry ground.
            let (hx, hy, hw, hh) = (
                sb.building.x,
                sb.building.y,
                sb.building.w,
                sb.building.h,
            );
            for ty in hy..hy + hh {
                for tx in hx..hx + hw {
                    assert!(!s.rail.line.contains(&(tx, ty)), "seed {seed}: the hall is on the track");
                    assert!(!sb.platforms.iter().any(|p| p.0 == tx && p.1 == ty), "seed {seed}: the hall is on a platform");
                    assert!(!s.map.at(tx, ty).is_water(), "seed {seed}: the hall stands in water");
                }
            }
        }
        assert!(built >= 3, "only {built} seeds had a station building");
    }

    /// B91: every boat, ferry and ship the sim spawns sails on water tiles and
    /// carries a kind the kit knows (a few islands, lake and river seeds).
    #[test]
    fn boats_never_sail_on_land() {
        let mut seen = 0;
        for seed in [5u64, 9, 13, 16] {
            let mut s = city(seed, 45.0);
            for _ in 0..12 {
                s.rail.next_boat = s.rail.clock;
                s.rail.next_ship = s.rail.clock;
                for _ in 0..80 {
                    s.transport_step(0.25);
                }
            }
            for m in s.transport() {
                if !matches!(m.kind, crate::rail::MoverKind::Boat | crate::rail::MoverKind::Ship | crate::rail::MoverKind::Ferry) {
                    continue;
                }
                seen += 1;
                assert!(m.route.len() >= 2, "seed {seed}: a {:?} route of {}", m.kind, m.route.len());
                for &(x, y) in &m.route {
                    assert!(s.map.at(x, y).is_water(), "seed {seed}: a {:?} on land at {x},{y}", m.kind);
                }
                assert!((m.set & 31) < 13, "seed {seed}: kind index {}", m.set & 31);
            }
        }
        assert!(seen > 0, "no boats spawned on any seed");
    }

    /// B58: the harbour's piers and its ferry route are on water, and the boats
    /// are moored beside the pier.
    #[test]
    fn the_harbour_is_on_the_water() {
        let mut found = 0;
        for seed in [1u64, 7] {
            let s = city(seed, 45.0);
            let Some(h) = &s.rail.harbour else { continue };
            found += 1;
            // The quay is a straight run of land with water in front of it
            // (B58c): every tile of it on dry ground, every water neighbour wet.
            assert!(h.quay.len() >= 5, "seed {seed}: a quay of {} tiles", h.quay.len());
            for pair in h.quay.windows(2) {
                let d = (pair[0].0 - pair[1].0).abs() + (pair[0].1 - pair[1].1).abs();
                assert_eq!(d, 1, "seed {seed}: the quay jumps {:?} -> {:?}", pair[0], pair[1]);
            }
            for &(x, y) in &h.quay {
                assert!(!s.map.at(x, y).is_water(), "seed {seed}: the quay is in the water");
                let (wx, wy) = (x + h.water_dir.0, y + h.water_dir.1);
                assert!(s.map.at(wx, wy).is_water(), "seed {seed}: the quay faces land at {x},{y}");
            }
            for pier in &h.piers {
                assert!(!pier.is_empty());
                for pair in pier.windows(2) {
                    let d = (pair[0].0 - pair[1].0).abs() + (pair[0].1 - pair[1].1).abs();
                    assert_eq!(d, 1, "seed {seed}: a pier jumps");
                }
                for &(x, y) in pier {
                    assert!(s.map.at(x, y).is_water(), "seed {seed}: a pier on land at {x},{y}");
                }
            }
            assert!(h.ferry.len() >= 6, "seed {seed}: a ferry route of {} tiles", h.ferry.len());
            for &(x, y) in &h.ferry {
                assert!(s.map.at(x, y).is_water(), "seed {seed}: the ferry runs on land at {x},{y}");
            }
            for &(x, y) in &h.moored {
                assert!(s.map.at(x, y).is_water(), "seed {seed}: a fishing boat in a field");
            }
        }
        assert!(found > 0, "no coastal city got a harbour");
    }

    /// B58d: nothing transport sites stands above the land line. With the frame
    /// and the land line the viewer declares, the harbour, its piers, the marina
    /// pontoon and the container port all have to be below it — the review found
    /// the quay, a pier deck and the pontoon drawn up over the sky band, cut by
    /// the horizon haze.
    #[test]
    fn nothing_transport_builds_stands_above_the_land_line() {
        let mut checked = 0;
        for seed in [1u64, 3, 7, 11] {
            let vp = map::world_for_viewport(1920, 1080);
            let focus = map::screen_to_tile(&vp, 960.0, 540.0);
            let mut s = Sim::with_focus(seed, Pace::Month, vp.w, vp.h, focus);
            // The wallpaper's own numbers for a 1080-row frame at 1×.
            s.set_viewport(vp.origin.0, vp.origin.1, 1920, 1080);
            s.set_horizon(324, 120);
            s.run_to(8.0 * 86400.0);
            s.plan_rail();
            s.run_to(45.0 * 86400.0);
            s.plan_rail();
            let mut ground: Vec<(i32, i32)> = Vec::new();
            if let Some(h) = &s.rail.harbour {
                ground.extend(h.quay.iter().copied());
                ground.extend(h.cranes.iter().copied());
                for p in &h.piers {
                    ground.extend(p.iter().copied());
                }
                let (sx, sy, sw, sh) = h.shed;
                for ty in sy..sy + sh {
                    for tx in sx..sx + sw {
                        ground.push((tx, ty));
                    }
                }
                ground.extend(h.moored.iter().copied());
            }
            if let Some(m) = s.rail.marina {
                for ty in m.y..m.y + m.h {
                    for tx in m.x..m.x + m.w {
                        ground.push((tx, ty));
                    }
                }
            }
            if let Some(p) = s.rail.port {
                for ty in p.y..p.y + p.h {
                    for tx in p.x..p.x + p.w {
                        ground.push((tx, ty));
                    }
                }
            }
            if ground.is_empty() {
                continue;
            }
            checked += 1;
            for &(x, y) in &ground {
                let (_, oy, _, _) = s.viewport;
                let row = oy + (x + y) * map::iso_consts::HALF_H;
                assert!(
                    row >= s.horizon_px,
                    "seed {seed}: a harbour tile at {x},{y} is on row {row}, above the land line ({})",
                    s.horizon_px
                );
            }
        }
        assert!(checked > 0, "no seed built a harbour to check");
    }

    /// The same seed and the same days plan the same network: the transport is
    /// part of the deterministic world, not a roll of the dice at runtime.
    #[test]
    fn transport_is_deterministic() {
        let a = city(3, 30.0);
        let b = city(3, 30.0);
        assert_eq!(a.rail.line, b.rail.line);
        assert_eq!(a.rail.twin, b.rail.twin);
        assert_eq!(a.rail.station_build, b.rail.station_build);
        assert_eq!(a.rail.bus_lines, b.rail.bus_lines);
        assert_eq!(a.rail.sidings, b.rail.sidings);
        assert_eq!(a.rail.airport, b.rail.airport);
        assert_eq!(a.rail.harbour, b.rail.harbour);
        assert_eq!(a.rail.subway, b.rail.subway);
        assert_eq!(a.rail.containers, b.rail.containers);
    }

    /// B58: the second track is a *parallel* line, apart from the first and no
    /// further than a couple of tiles from it, with no jumps.
    #[test]
    fn the_second_track_runs_alongside_the_first() {
        let s = city(3, 45.0);
        assert!(!s.rail.twin.is_empty(), "a city-era line with no second track");
        for &(x, y) in &s.rail.twin {
            assert!(!s.rail.line.contains(&(x, y)), "the twin sits on the line at {x},{y}");
            let d = s
                .rail
                .line
                .iter()
                .map(|&(lx, ly)| (lx - x).abs() + (ly - y).abs())
                .min()
                .unwrap_or(99);
            assert!(d <= 2, "a second track {d} tiles from the first at {x},{y}");
        }
        // The double track serves the station stretch (that is where two
        // platforms face each other), so it is a stretch and not the whole line.
        assert!(
            s.rail.twin.len() >= 6,
            "only {} tiles of double track",
            s.rail.twin.len()
        );
        // And it is contiguous: a parallel track that jumps is not a track.
        for pair in s.rail.twin.windows(2) {
            let d = (pair[0].0 - pair[1].0).abs() + (pair[0].1 - pair[1].1).abs();
            assert_eq!(d, 1, "the second track jumps {:?} -> {:?}", pair[0], pair[1]);
        }
    }
}
