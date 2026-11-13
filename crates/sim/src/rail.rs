//! Rail, trams, boats and planes (B31, M7 rest).
//!
//! All of it is **placeholders with real mechanics**: a rail line planned over
//! the terrain (graded, bridged, connected), a station and a yard, passenger and
//! freight trains on a timetable, trams on the main avenue from the city era,
//! boats on the nearest water, a marina on a coast and a container port once the
//! city era arrives — plus planes crossing the sky band.
//!
//! Everything is data the renderer draws; nothing here knows about pixels. As
//! with traffic, the sim is a pure function of `(seed, time)`: planning happens
//! once, at the era that unlocks it, and the movers are derived from the clock.

use map::{Kind, WaterKind};

use crate::{transit, District, Era, Sim};

/// A moving thing on a fixed route (a train, a tram, a boat, a plane).
#[derive(Clone, Debug)]
pub struct Mover {
    pub kind: MoverKind,
    /// The tiles (or sky positions for a plane) it travels through.
    pub route: Vec<(i32, i32)>,
    /// Distance along the route, in tiles.
    pub travelled: f32,
    /// Tiles per second.
    pub speed: f32,
    /// Runs back along the route when it reaches the end (trams, boats, planes).
    pub shuttle: bool,
    /// Heading back? Only meaningful with `shuttle`.
    pub back: bool,
    /// How many trailing units follow (train cars, tram cars).
    pub cars: u8,
    /// Height above the ground, in pixels (planes).
    pub lift: i32,
    /// Seconds still standing at a stop (B31b: trams call at tram stops, trains
    /// at the station). While this is positive the mover does not advance.
    pub dwell: f32,
    /// Seconds before this mover may call at a stop again, so it does not get
    /// stuck on one.
    pub cooldown: f32,
    /// Which set this is (B58): 0 = diesel passenger, 1 = electric passenger,
    /// 2 = freight with mixed wagons. Trams, buses and boats leave it 0.
    pub set: u8,
    /// Which bus line (index into `rail.bus_lines`), so a bus calls at its own
    /// stops and not at another line's.
    pub line: u8,
    /// Aircraft: 0 = taxi out and take off, 1 = land and taxi in.
    pub mode: u8,
    /// A continuous phase for a mover that cycles in place (a crane's trolley,
    /// 0..1 out and back). Everything that travels along a route leaves it 0.
    pub phase: f32,
}

impl Mover {
    pub fn route_len(&self) -> f32 {
        self.route
            .windows(2)
            .map(|w| ((w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()) as f32)
            .sum::<f32>()
            .max(0.1)
    }

    /// Position and heading at `travelled` (or the way back).
    pub fn pose(&self) -> ((f32, f32), (f32, f32)) {
        self.pose_at(self.travelled)
    }

    /// Position and heading at another distance along the route.
    pub fn pose_at(&self, travelled: f32) -> ((f32, f32), (f32, f32)) {
        self.pose_core(travelled, false)
    }

    /// `exact` gives a zero for a zero step (`f32::signum(0.0)` is 1, which
    /// turns a straight run along y into a diagonal heading).
    fn pose_core(&self, travelled: f32, exact: bool) -> ((f32, f32), (f32, f32)) {
        let sg = |v: f32| if exact && v == 0.0 { 0.0 } else { v.signum() };
        let len = self.route_len();
        let mut d = travelled.rem_euclid(len * if self.shuttle { 2.0 } else { 1.0 });
        let reversed = self.shuttle && d > len;
        if reversed {
            d = len * 2.0 - d;
        }
        let mut acc = 0.0f32;
        for w in self.route.windows(2) {
            let (ax, ay) = (w[0].0 as f32, w[0].1 as f32);
            let (bx, by) = (w[1].0 as f32, w[1].1 as f32);
            let seg = (bx - ax).abs() + (by - ay).abs();
            if d <= acc + seg || seg <= f32::EPSILON {
                let t = ((d - acc) / seg.max(f32::EPSILON)).clamp(0.0, 1.0);
                let mut dir = (sg(bx - ax), sg(by - ay));
                if reversed {
                    dir = (-dir.0, -dir.1);
                }
                return ((ax + (bx - ax) * t + 0.5, ay + (by - ay) * t + 0.5), dir);
            }
            acc += seg;
        }
        let last = *self.route.last().unwrap_or(&(0, 0));
        (
            (last.0 as f32 + 0.5, last.1 as f32 + 0.5),
            (if reversed { -1.0 } else { 1.0 }, 0.0),
        )
    }

    /// B79 (D91): a boat's pose with its lane: `cooldown` carries the sideways
    /// offset in tiles (boats never use the cooldown for anything else), applied
    /// across the heading, smoothed round corners.
    pub fn boat_pose(&self) -> ((f32, f32), (f32, f32)) {
        if !matches!(self.kind, MoverKind::Boat | MoverKind::Ferry | MoverKind::Ship) {
            return self.pose();
        }
        self.boat_pose_at(self.travelled, self.cooldown)
    }

    /// The same for another distance along the route and another lane offset
    /// (what a boat would do if it moved: the collision check asks first).
    pub fn boat_pose_at(&self, travelled: f32, off: f32) -> ((f32, f32), (f32, f32)) {
        let ((x, y), dir) = self.pose_core(travelled, true);
        if off == 0.0 {
            return ((x, y), dir);
        }
        let perp = |d: (f32, f32)| {
            let n = (d.0 * d.0 + d.1 * d.1).sqrt().max(1e-3);
            (-d.1 / n, d.0 / n)
        };
        let (a, b) = (perp(self.pose_core(travelled - 0.6, true).1), perp(self.pose_core(travelled + 0.6, true).1));
        let (mut px, mut py) = (a.0 + b.0, a.1 + b.1);
        let n = (px * px + py * py).sqrt();
        if n < 0.3 {
            (px, py) = perp(dir);
        } else {
            px /= n;
            py /= n;
        }
        ((x + px * off, y + py * off), dir)
    }

    /// A trailing car's position: `back` tiles behind along the same route.
    pub fn car_pose(&self, back: f32) -> (f32, f32) {
        let mut m = self.clone();
        m.travelled = (self.travelled - back).max(0.0);
        m.pose().0
    }

    pub fn tile(&self) -> (i32, i32) {
        let (p, _) = self.pose();
        (p.0.floor() as i32, p.1.floor() as i32)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoverKind {
    /// A locomotive (the trailing units are `cars` wagons).
    Train,
    Tram,
    Boat,
    /// A container ship at the port quay (B31b).
    Ship,
    /// A plane crossing the sky band.
    Plane,
    /// A bus on its line (B58): calls at its stops and waits.
    Bus,
    /// A harbour ferry, shuttling between the piers.
    Ferry,
    /// An aircraft on the airport's runway and taxiway: taxi, take-off, landing.
    Aircraft,
    /// A container crane's trolley: `travelled` is its cycle, 0..1 out and back.
    Crane,
}

/// A rectangle on the map (the port, the marina, the yard).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Zone {
    pub fn centre(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Everything about rail and water traffic.
#[derive(Default)]
pub struct Rail {
    /// The main line, in order from one end to the other.
    pub line: Vec<(i32, i32)>,
    /// The second track of a double-track railway (city era): a parallel line on
    /// the side away from the platform, so trains pass each other at the
    /// station (B58).
    pub twin: Vec<(i32, i32)>,
    /// The tram line (the main avenue), from the city era.
    pub tram_line: Vec<(i32, i32)>,
    /// Tram stops along it (B31b).
    pub tram_stops: Vec<(i32, i32)>,
    /// Which side of the street each tram stop's shelter stands on (0..3, as the
    /// platform's side flag; B58).
    pub tram_stop_sides: Vec<u8>,
    pub movers: Vec<Mover>,
    /// The track tile where trains stop (on the line).
    pub station: Option<(i32, i32)>,
    /// The platform beside it: (x, y, side) where side 0/1 = +x/-x and 2/3 =
    /// +y/-y from the station tile (B31b: the platform runs parallel to the
    /// track instead of sitting on it).
    pub station_platform: Option<(i32, i32, u8)>,
    /// The station the city built: a halt, a town station or a terminal, with
    /// its building, its platforms and the canopy over them (B58).
    pub station_build: Option<StationBuild>,
    /// Freight-yard sidings (B58): short tracks inside the yard.
    pub sidings: Vec<(i32, i32)>,
    /// Wagons standing in the yard: (tile, wagon kind 0..3).
    pub parked_wagons: Vec<((i32, i32), u8)>,
    /// Overhead catenary (B58): exposed for the era system (B49) to drive.
    pub electrified: bool,
    /// Bus lines, their stops and the depot (B58).
    pub bus_lines: Vec<transit::BusLine>,
    pub depot: Option<Zone>,
    /// The subway (B58), from the metropolis era.
    pub subway: Option<transit::Subway>,
    /// Sim time the metro era began: the anchor of the subway works, and the
    /// only transit time that has to survive a save.
    pub metro_at: Option<f64>,
    /// When each search last ran and found nothing: a planner that fails must
    /// not scan the whole map on every frame (a big world makes that a stall).
    pub bus_tried: f64,
    pub crossings_tried: f64,
    pub airport_tried: f64,
    pub subway_tried: f64,
    pub harbour_tried: f64,
    /// B61d: the container port's waterfront search. It scans the visible band
    /// for a free quay, and on a map with no site that fits the frame it found
    /// nothing — every frame, forever (the wallpaper calls `plan_rail` from the
    /// frame path), at 8 % of a core. Same retry rule as the rest (once the
    /// plan is in, ask again once an hour).
    pub port_tried: f64,
    /// The airfield (B58).
    pub airport: Option<transit::Airport>,
    /// The harbour (B58).
    pub harbour: Option<transit::Harbour>,
    pub yard: Option<Zone>,
    pub marina: Option<Zone>,
    pub port: Option<Zone>,
    /// Containers stacked in the port (for the renderer): (x, y, colour pick).
    pub containers: Vec<(i32, i32, u8)>,
    /// Planned already, so the planner runs once.
    pub planned: bool,
    /// Next arrival times (sim seconds).
    pub next_train: f64,
    pub next_boat: f64,
    pub next_plane: f64,
    pub next_ship: f64,
    pub next_bus: f64,
    pub next_ferry: f64,
    pub next_aircraft: f64,
    /// Tick counter: transport steps taken. The rail's own dice, so a spawn is
    /// not the same draw every step (B31b: trams never spawned because the RNG
    /// was keyed on the city's planning step).
    pub tick: u64,
    /// The transport clock: seconds of timetable run so far. Not the city clock
    /// (a warm-up runs many steps without moving sim time).
    pub clock: f64,
    /// Counters for the report.
    pub trains: u32,
    pub boats_seen: u32,
    /// B79: islands, jetties, sea routes and the shore (derived, not saved).
    pub isles: crate::isles::Isles,
}

/// How big the station the city has built is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StationKind {
    /// A halt: a platform, a shelter and a name board (town era).
    Halt,
    /// A town station: a building with a booking hall, canopy, two platforms
    /// (city era, single or double track).
    Town,
    /// A terminal: a bigger building, a longer platform, a forecourt.
    Terminal,
}

impl StationKind {
    pub fn index(self) -> u8 {
        match self {
            StationKind::Halt => 0,
            StationKind::Town => 1,
            StationKind::Terminal => 2,
        }
    }

    pub fn from_index(i: u8) -> Self {
        match i {
            0 => StationKind::Halt,
            1 => StationKind::Town,
            _ => StationKind::Terminal,
        }
    }

    pub fn from_era(e: Era) -> Self {
        if e >= Era::Metropolis {
            StationKind::Terminal
        } else if e >= Era::City {
            StationKind::Town
        } else {
            StationKind::Halt
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            StationKind::Halt => "halt",
            StationKind::Town => "station",
            StationKind::Terminal => "terminal",
        }
    }

    /// How many tiles of platform a train calls at (its length in coaches).
    pub fn platform_len(self) -> u8 {
        match self {
            StationKind::Halt => 2,
            StationKind::Town => 4,
            StationKind::Terminal => 6,
        }
    }
}

/// The station building, its platforms and the canopy over them (B58).
#[derive(Clone, PartialEq, Debug)]
pub struct StationBuild {
    pub kind: StationKind,
    /// The booking hall: a walled building beside the platforms.
    pub building: Zone,
    /// Every platform tile and the side it faces the track from.
    pub platforms: Vec<(i32, i32, u8)>,
    /// Platform tiles under a canopy.
    pub canopy: Vec<(i32, i32)>,
    /// The forecourt (paving between the building and the street).
    pub forecourt: Vec<(i32, i32)>,
}


impl Sim {
    /// Plan the rail line, the station, the yard, the tram line, the marina and
    /// the port — once, at the era that unlocks them.
    pub fn plan_rail(&mut self) {
        let era = self.era();
        if era < Era::Town {
            return;
        }
        if !self.rail.planned {
            self.rail.planned = true;
            // The anchor is where the station will end up; the line is planned
            // through it, then the platform goes *beside* the track (B31b).
            let anchor = self.pick_station();
            let line = self.plan_line(anchor);
            self.rail.line = self.trim_line_to_view(line, anchor);
            self.clear_rail_corridor();
            self.place_station();
            // A yard by the industry and, on a coast, a marina in the high-end
            // quarter (D38/D55).
            self.rail.yard = self.zone_in(District::Industrial, 4);
            if self.map.ocean.is_some() {
                self.rail.marina = self.marina_zone(4);
            }
            self.rail.next_train = self.rail.clock + 30.0;
            self.rail.next_boat = self.rail.clock + 90.0;
            self.rail.next_plane = self.rail.clock + 150.0;
            self.rail.next_ship = self.rail.clock + 40.0;
            // B58: the station the town builds first (a halt), and the sidings
            // in the yard.
            self.plan_station_build(StationKind::from_era(era));
            self.plan_sidings();
            self.events.push(crate::Event {
                time: self.time,
                text: "the railway is surveyed".into(),
            });
        }
        // The city era adds the tram line and the container port (B31b: these
        // used to be planned only if the first plan happened in the city era).
        if era >= Era::City {
            if self.rail.tram_line.is_empty() {
                self.rail.tram_line = self.plan_tram();
                self.rail.tram_stops = self.plan_tram_stops();
                self.rail.tram_stop_sides = self.plan_tram_stop_sides();
            }
            // B58: double track and a station building once the city is big.
            if self.rail.twin.is_empty() {
                let twin = self.plan_twin();
                self.rail.twin = twin
                    .into_iter()
                    .filter(|&(x, y)| !self.near_land_line(x, y, 48))
                    .collect();
                if !self.rail.twin.is_empty() {
                    self.clear_rail_corridor();
                }
            }
            if self.rail.port.is_none()
                && (self.rail.port_tried == 0.0 || self.time - self.rail.port_tried > 3600.0)
            {
                self.rail.port_tried = self.time.max(1.0);
                self.rail.port = self.waterfront_zone(District::Industrial, 6, true);
                if let Some(p) = self.rail.port {
                    self.clear_port_plots(p);
                    self.stack_containers(p);
                }
            }
        }
        // The station grows with the eras (halt → town station → terminal).
        let want = StationKind::from_era(era);
        if self
            .rail
            .station_build
            .as_ref()
            .map(|s| s.kind)
            .is_some_and(|k| k != want)
        {
            self.plan_station_build(want);
            self.events.push(crate::Event {
                time: self.time,
                text: format!("the station is rebuilt as a {}", want.name()),
            });
        }
        // The town's crossings over the line, then the rest of the network.
        self.plan_crossings();
        self.plan_transit();
    }

    /// A station site: on a lane near the main street, close to downtown.
    fn pick_station(&self) -> Option<(i32, i32)> {
        let (fx, fy) = self.founding;
        let mut best: Option<((i32, i32), i32)> = None;
        for y in (fy - 12).max(2)..=(fy + 12).min(self.map.h - 3) {
            for x in (fx - 12).max(2)..=(fx + 12).min(self.map.w - 3) {
                let t = self.map.at(x, y);
                if t.is_water() || t.kind.is_lane() || t.kind.is_worked() || t.relief() > 1 {
                    continue;
                }
                if self.lots.iter().any(|l| l.covers(x, y)) {
                    continue;
                }
                // Next to a street, and in or near downtown.
                let near_road = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| self.map.at(x + dx, y + dy).kind.is_lane());
                if !near_road {
                    continue;
                }
                // On the edge of the old town, not in its middle: a station in
                // the densest block is invisible from this camera (B31).
                let d = (x - fx).abs() + (y - fy).abs();
                if !(8..=16).contains(&d) {
                    continue;
                }
                if best.map(|(_, bd)| d < bd).unwrap_or(true) {
                    best = Some(((x, y), d));
                }
            }
        }
        best.map(|(p, _)| p)
    }

    /// The rail's dice: `rng` keyed on the transport tick, so a timetable draw
    /// changes from step to step (B31b).
    fn rail_rng(&self, tag: u64) -> map::rng::Rng {
        map::rng::Rng::new(
            self.seed
                ^ self.rail.tick.wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ tag.wrapping_mul(0xA24B_AED4_963E_E407),
        )
    }

    /// B99/D73: the railway ends where the picture does. A line that ran on to
    /// the map edge went into the sky band, where the horizon cut its ballast
    /// flat over the haze. Keep the run of tiles around the station that stays
    /// below the land line (with a margin for the slab); the track then fades
    /// into the haze at its ends like the streets do.
    fn trim_line_to_view(&self, line: Vec<(i32, i32)>, keep: Option<(i32, i32)>) -> Vec<(i32, i32)> {
        const MARGIN: i32 = 48;
        if line.is_empty() || !line.iter().any(|&(x, y)| self.near_land_line(x, y, MARGIN)) {
            return line;
        }
        let at = keep.unwrap_or(line[line.len() / 2]);
        let Some(si) = (0..line.len()).min_by_key(|&i| (line[i].0 - at.0).abs() + (line[i].1 - at.1).abs()) else {
            return line;
        };
        let mut lo = si;
        while lo > 0 && !self.near_land_line(line[lo - 1].0, line[lo - 1].1, MARGIN) {
            lo -= 1;
        }
        let mut hi = si;
        while hi + 1 < line.len() && !self.near_land_line(line[hi + 1].0, line[hi + 1].1, MARGIN) {
            hi += 1;
        }
        if hi - lo < 8 {
            return line;
        }
        line[lo..=hi].to_vec()
    }

    /// The line: **through** the city — in from one edge, past the station, out
    /// to the industrial ground — graded like a street (a rail is a civil work
    /// too, D34).
    fn plan_line(&mut self, anchor: Option<(i32, i32)>) -> Vec<(i32, i32)> {
        let Some(station) = anchor else {
            return Vec::new();
        };
        // The industrial ground gives the direction: freight goes there, and the
        // passenger line runs on past the station to the opposite edge.
        let target = self
            .district_seed(District::Industrial)
            .unwrap_or((station.0, (station.1 - 30).max(3)));
        let blocked = self.lot_tiles_for_rail();
        let noise = self.rail_rng(0x7A11_0001).next_u64();
        // Far target: beyond the industrial seed, pushed out to the map edge.
        let (dx, dy) = (target.0 - station.0, target.1 - station.1);
        let far = {
            let scale = 1.6;
            (
                (station.0 as f32 + dx as f32 * scale).clamp(3.0, (self.map.w - 4) as f32) as i32,
                (station.1 as f32 + dy as f32 * scale).clamp(3.0, (self.map.h - 4) as f32) as i32,
            )
        };
        // Behind the station: the opposite side of town.
        let back = (
            (station.0 - dx / 2).clamp(3, self.map.w - 4),
            (station.1 - dy / 2).clamp(3, self.map.h - 4),
        );
        let a = crate::grid::route_rail(&self.map, back, station, noise, &blocked);
        let b = crate::grid::route_rail(&self.map, station, far, noise ^ 0x9E37, &blocked);
        if let (Some(mut a), Some(b)) = (a, b) {
            if a.len() > 3 && b.len() > 3 {
                a.extend(b.into_iter().skip(1));
                return a;
            }
        }
        // One leg is better than none.
        if let Some(path) = crate::grid::route_rail(&self.map, station, far, noise, &blocked) {
            if path.len() > 6 {
                return path;
            }
        }
        // Fall back to a plain line toward the map edge.
        let mut line = Vec::new();
        let (dx, dy) = ((target.0 - station.0).signum(), (target.1 - station.1).signum());
        let (mut x, mut y) = station;
        for _ in 0..40 {
            if x < 3 || y < 3 || x > self.map.w - 4 || y > self.map.h - 4 {
                break;
            }
            line.push((x, y));
            if x != target.0 {
                x += dx;
            } else if y != target.1 {
                y += dy;
            } else {
                break;
            }
        }
        line
    }

    /// A railway takes its corridor: plots the alignment crosses are demolished
    /// (a graded cutting through the block, D33/D34) and the line is drawn on
    /// open ground instead of under a roof.
    fn clear_rail_corridor(&mut self) {
        if self.rail.line.is_empty() {
            return;
        }
        let mut near: std::collections::BTreeSet<(i32, i32)> = self
            .rail
            .line
            .iter()
            .flat_map(|&(x, y)| [(x, y), (x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)])
            .collect();
        // The second track and the yard's sidings take their ground too (B58:
        // the first render had trees standing in the ballast).
        for &(x, y) in self.rail.twin.iter().chain(self.rail.sidings.iter()) {
            near.insert((x, y));
        }
        let before = self.lots.len();
        self.lots.retain(|l| {
            // B52b: the civic square and the hall are never cleared for the
            // line (the line already routes round every lot; this is the
            // one-tile margin).
            // B52b: the civic square and the hall are never cleared for the
            // line (the line already routes round every lot; this is the
            // one-tile margin).
            // B77: and neither is anything the town cannot build again — the
            // school, the church, the cathedral, the library, the station, the
            // cemetery. At the one-tile margin a grown city used to lose its
            // whole civic quarter to the railway: no school, no church, and a
            // school run with nothing to run to.
            if matches!(
                l.kind,
                crate::LotKind::Hall
                    | crate::LotKind::Church
                    | crate::LotKind::Cemetery
                    | crate::LotKind::Landmark(_)
            ) {
                return true;
            }
            !(l.y..l.y + l.h).any(|ty| (l.x..l.x + l.w).any(|tx| near.contains(&(tx, ty))))
        });
        // And the ground it takes: trees and bushes out of the corridor.
        let corridor: Vec<(i32, i32)> = near.iter().copied().collect();
        self.clear_props(&corridor);
        let cleared = before - self.lots.len();
        if cleared > 0 {
            self.events.push(crate::Event {
                time: self.time,
                text: format!("the railway takes its corridor: {cleared} plots cleared"),
            });
        }
    }

    /// Is this tile the railway's own ground? The corridor, the second track,
    /// the yard's sidings, the platforms, the canopy and the station hall. Streets
    /// and plots keep off it (B58d: a town paved the platform strip and the
    /// station had nowhere to stand).
    pub fn on_rail_ground(&self, x: i32, y: i32) -> bool {
        let p = (x, y);
        if self.rail.line.contains(&p)
            || self.rail.twin.contains(&p)
            || self.rail.sidings.contains(&p)
        {
            return true;
        }
        if let Some(sb) = &self.rail.station_build {
            if sb.platforms.iter().any(|q| (q.0, q.1) == p)
                || sb.canopy.contains(&p)
                || sb.forecourt.contains(&p)
            {
                return true;
            }
            let b = sb.building;
            if x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h {
                return true;
            }
        }
        false
    }

    /// The railway takes its ground: no tree, bush or boulder stands in the
    /// corridor or on the station's paving (B58: a tree through a platform was
    /// the first render's most obvious fault).
    pub(crate) fn clear_props(&mut self, tiles: &[(i32, i32)]) {
        for &(x, y) in tiles {
            if !self.map.in_bounds(x, y) {
                continue;
            }
            let t = self.map.at(x, y);
            if t.prop != map::Prop::None {
                let mut nt = t;
                nt.prop = map::Prop::None;
                self.write(x, y, nt);
            }
        }
    }

    /// Lots are walls for a rail route (a line does not run through a house).
    fn lot_tiles_for_rail(&self) -> std::collections::BTreeSet<(i32, i32)> {
        self.lot_tiles()
    }

    /// Put the station on the line nearest downtown, with the platform on the
    /// free tile beside the track (B31b item 4: parallel to it, not under it).
    ///
    /// Two things matter: it should be downtown-adjacent, and it must be *seen*
    /// — a station behind a tower is a station nobody knows about, and the
    /// camera looks from +x/+y, so the tiles in front (larger x and y) are what
    /// hides it.
    fn place_station(&mut self) {
        if self.rail.line.len() < 6 {
            return;
        }
        let (fx, fy) = self.founding;
        let mut best: Option<(Option<(i32, i32, u8)>, (i32, i32), i32)> = None;
        for &(tx, ty) in self.rail.line.iter().skip(1) {
            let along_x = self
                .rail
                .line
                .iter()
                .any(|&p| p == (tx + 1, ty) || p == (tx - 1, ty));
            let sides: [(i32, i32); 2] = if along_x {
                [(0, 1), (0, -1)]
            } else {
                [(1, 0), (-1, 0)]
            };
            for (dx, dy) in sides {
                let (px, py) = (tx + dx, ty + dy);
                if !self.map.in_bounds(px, py) {
                    continue;
                }
                let t = self.map.at(px, py);
                if t.is_water() || t.kind.is_lane() || t.relief() > 1 {
                    continue;
                }
                if self.lots.iter().any(|l| l.covers(px, py)) {
                    continue;
                }
                // What stands between the camera and the platform (B31b: it has
                // to be visible). The camera looks from +x/+y, so a lot in that
                // quadrant hides us if it is tall enough: a storey is ~4 px and
                // a tile of depth is 8 px, so a building covers the tiles up to
                // `storeys / 2` steps behind it.
                let mut front = 0;
                for l in &self.lots {
                    let fx = (l.x - px).max(0);
                    let fy = (l.y - py).max(0);
                    let steps = fx + fy;
                    if steps == 0 {
                        continue; // covers us — excluded above
                    }
                    if (l.y + l.h <= py) || (l.x + l.w <= px) {
                        continue; // behind the platform
                    }
                    if steps <= 1 + l.storeys as i32 / 2 {
                        front += 1;
                    }
                }
                let d = (tx - fx).abs() + (ty - fy).abs();
                // Visibility first, then closeness: a hidden station is worthless.
                let score = front * 40 + d;
                let side = if px > tx {
                    0
                } else if px < tx {
                    1
                } else if py > ty {
                    2
                } else {
                    3
                };
                if best.map(|(_, _, b)| score < b).unwrap_or(true) {
                    best = Some((Some((px, py, side)), (tx, ty), score));
                }
            }
        }
        if let Some((platform, station, _)) = best {
            self.rail.station = Some(station);
            self.rail.station_platform = platform;
        }
    }

    /// Tram stops: every sixth tile of the avenue, away from the ends (B31b).
    fn plan_tram_stops(&self) -> Vec<(i32, i32)> {
        let line = &self.rail.tram_line;
        if line.len() < 8 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut i = 2;
        while i < line.len() - 1 {
            out.push(line[i]);
            i += 6;
        }
        out
    }

    /// The pavement side of each tram stop (B58: the shelter goes on the kerb,
    /// not in the road).
    fn plan_tram_stop_sides(&self) -> Vec<u8> {
        let line = &self.rail.tram_line;
        self.rail
            .tram_stops
            .iter()
            .map(|&(x, y)| {
                let along_x = line.contains(&(x + 1, y)) || line.contains(&(x - 1, y));
                if along_x {
                    if self.map.at(x, y + 1).kind == Kind::Sidewalk {
                        2
                    } else if self.map.at(x, y - 1).kind == Kind::Sidewalk {
                        3
                    } else {
                        2
                    }
                } else if self.map.at(x + 1, y).kind == Kind::Sidewalk {
                    0
                } else if self.map.at(x - 1, y).kind == Kind::Sidewalk {
                    1
                } else {
                    0
                }
            })
            .collect()
    }

    /// The station a city of this era builds: its platforms (one tile either
    /// side of the track once there is a second one), the canopy over them and
    /// the booking hall beside them. The station takes its ground, like the
    /// corridor it stands on (B58/D73).
    fn plan_station_build(&mut self, kind: StationKind) {
        let Some(station) = self.rail.station else {
            return;
        };
        let Some((_, _, side)) = self.rail.station_platform else {
            return;
        };
        let Some(si) = self.rail.line.iter().position(|&p| p == station) else {
            return;
        };
        // The line's own direction at the station (its neighbours may be
        // streets, which the adjacent-tile test below cannot tell from track).
        let (dx, dy) = if si + 1 < self.rail.line.len() {
            let n = self.rail.line[si + 1];
            ((n.0 - station.0).signum(), (n.1 - station.1).signum())
        } else if si > 0 {
            let p = self.rail.line[si - 1];
            ((station.0 - p.0).signum(), (station.1 - p.1).signum())
        } else {
            (1, 0)
        };
        let along_x = dx.abs() >= dy.abs();
        let len = kind.platform_len() as i32;
        let half = len / 2;
        let vec_of = |s: u8| -> (i32, i32) {
            match s {
                0 => (1, 0),
                1 => (-1, 0),
                2 => (0, 1),
                _ => (0, -1),
            }
        };
        // The strip of platform tiles on one side, taken along the line's own
        // indices so a bend or an end does not cut the platform short.
        let strip = |sim: &Sim, s: u8| -> Vec<(i32, i32)> {
            let (sx, sy) = vec_of(s);
            let mut out = Vec::new();
            for k in -half..=half {
                let idx = si as i32 + k;
                if idx < 0 || idx >= sim.rail.line.len() as i32 {
                    continue;
                }
                let (tx, ty) = sim.rail.line[idx as usize];
                let (ux, uy) = (tx + sx, ty + sy);
                if !sim.map.in_bounds(ux, uy) {
                    continue;
                }
                let t = sim.map.at(ux, uy);
                if t.is_water() || t.kind.is_lane() {
                    continue;
                }
                out.push((ux, uy));
            }
            out
        };
        let opposite = match side {
            0 => 1,
            1 => 0,
            2 => 3,
            _ => 2,
        };
        let a = strip(self, side);
        let b = strip(self, opposite);
        // The side with room wins; a station with a platform in the road is not
        // a station (B58/D73).
        let (use_side, chosen) = if b.len() > a.len() { (opposite, b) } else { (side, a) };
        if chosen.is_empty() {
            return;
        }
        let (sx, sy) = vec_of(use_side);
        // Keep the sim's platform tile saying which side the platform is on.
        let (px, py) = (station.0 + sx, station.1 + sy);
        if self.map.in_bounds(px, py) {
            self.rail.station_platform = Some((px, py, use_side));
        }
        let mut platforms: Vec<(i32, i32, u8)> = Vec::new();
        let mut canopy: Vec<(i32, i32)> = Vec::new();
        let mut behind: Vec<(i32, i32)> = Vec::new();
        for &(ux, uy) in &chosen {
            platforms.push((ux, uy, use_side));
            if kind != StationKind::Halt {
                canopy.push((ux, uy));
            }
            behind.push((ux + sx, uy + sy));
            // The far platform, across the twin track, once the line doubles.
            if !self.rail.twin.is_empty() {
                let (vx, vy) = (ux - sx * 3, uy - sy * 3);
                if self.map.in_bounds(vx, vy) && !self.map.at(vx, vy).is_water() {
                    let face = match use_side {
                        0 => 1,
                        1 => 0,
                        2 => 3,
                        _ => 2,
                    };
                    if !platforms.iter().any(|p| (p.0, p.1) == (vx, vy)) {
                        platforms.push((vx, vy, face));
                        if kind != StationKind::Halt {
                            canopy.push((vx, vy));
                        }
                    }
                }
            }
        }
        // The booking hall: two tiles deep, beyond the platform row, at one
        // *end* of the platform strip. That is where a real station hall stands
        // — and where the platform's canopy cannot hide it (B58).
        let want_w = match kind {
            StationKind::Halt => 1,
            StationKind::Town => 3,
            StationKind::Terminal => 4,
        };
        let depth = 2i32;
        let (bw, bh) = if along_x { (want_w, depth) } else { (depth, want_w) };
        let along: Vec<i32> = if along_x {
            platforms.iter().map(|p| p.0).collect()
        } else {
            platforms.iter().map(|p| p.1).collect()
        };
        let (lo, hi) = (
            along.iter().min().copied().unwrap_or(station.0),
            along.iter().max().copied().unwrap_or(station.0),
        );
        let (ax, ay) = if along_x { (1, 0) } else { (0, 1) };
        let mut best: Option<(Zone, i32)> = None;
        for w in [want_w, 3, 2, 1] {
            let (bw, bh) = if along_x { (w, depth) } else { (depth, w) };
            for side_end in [-1i32, 1] {
              for back in 1..=6i32 {
                let anchor = if side_end < 0 { lo - back } else { hi + back };
                let (bx, by) = if along_x {
                    let x = if side_end < 0 { anchor - w + 1 } else { anchor };
                    (x, station.1 + sy * 2 - if sy < 0 { depth - 1 } else { 0 })
                } else {
                    let y = if side_end < 0 { anchor - w + 1 } else { anchor };
                    (station.0 + sx * 2 - if sx < 0 { depth - 1 } else { 0 }, y)
                };
                let _ = (ax, ay);
                if bx < 2 || by < 2 || bx + bw > self.map.w - 2 || by + bh > self.map.h - 2 {
                    continue;
                }
                let mut ok = true;
                'rect: for ty in by..by + bh {
                    for tx in bx..bx + bw {
                        let t = self.map.at(tx, ty);
                        if t.is_water() || t.kind.is_lane() || self.rail.line.contains(&(tx, ty))
                        {
                            ok = false;
                            break 'rect;
                        }
                        // Never on the platforms or the track between them.
                        if platforms.iter().any(|p| p.0 == tx && p.1 == ty) {
                            ok = false;
                            break 'rect;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                // Prefer the wider hall, then the end nearer the station.
                let dist = (bx - station.0).abs() + (by - station.1).abs();
                let score = (want_w - w).abs() * 6 + dist;
                if best.as_ref().map(|(_, b)| score < *b).unwrap_or(true) {
                    best = Some((
                        Zone {
                            x: bx,
                            y: by,
                            w: bw,
                            h: bh,
                        },
                        score,
                    ));
                }
              }
            }
        }
        let Some((building, _)) = best else {
            return;
        };

        // The hall takes its ground: plots under it are cleared (a station
        // forecourt replaces a block, as it does in any real town).
        let before = self.lots.len();
        self.lots.retain(|l| {
            !(l.y..l.y + l.h).any(|ty| {
                (l.x..l.x + l.w).any(|tx| {
                    tx >= building.x
                        && tx < building.x + building.w
                        && ty >= building.y
                        && ty < building.y + building.h
                })
            })
        });
        let cleared = before - self.lots.len();
        if cleared > 0 {
            self.events.push(crate::Event {
                time: self.time,
                text: format!("the station takes its forecourt: {cleared} plots cleared"),
            });
        }
        // The forecourt paving between the hall and the platform.
        let mut forecourt = Vec::new();
        if along_x {
            let fy = if sy > 0 {
                building.y - 1
            } else {
                building.y + building.h
            };
            for k in 0..building.w {
                if self.map.in_bounds(building.x + k, fy) {
                    forecourt.push((building.x + k, fy));
                }
            }
        } else {
            let fx = if sx > 0 {
                building.x - 1
            } else {
                building.x + building.w
            };
            for k in 0..building.h {
                if self.map.in_bounds(fx, building.y + k) {
                    forecourt.push((fx, building.y + k));
                }
            }
        }
        // The station's own ground: platforms, forecourt and hall keep no
        // trees on them (B58).
        let mut ground: Vec<(i32, i32)> = platforms.iter().map(|p| (p.0, p.1)).collect();
        ground.extend(forecourt.iter().copied());
        for ty in building.y..building.y + building.h {
            for tx in building.x..building.x + building.w {
                ground.push((tx, ty));
            }
        }
        self.clear_props(&ground);
        self.rail.station_build = Some(StationBuild {
            kind,
            building,
            platforms,
            canopy,
            forecourt,
        });
    }

    /// The second track: a parallel line on the side away from the platform,
    /// through the station stretch (where the two platforms face each other).
    /// Where the alignment turns a corner the parallel track would jump, so it
    /// stops there rather than kinking across the first track (B58).
    fn plan_twin(&self) -> Vec<(i32, i32)> {
        let Some((_, _, side)) = self.rail.station_platform else {
            return Vec::new();
        };
        let side_vec = match side {
            0 => (1i32, 0i32),
            1 => (-1, 0),
            2 => (0, 1),
            _ => (0, -1),
        };
        let line = &self.rail.line;
        let station = self.rail.station.unwrap_or(line[0]);
        let Some(si) = line.iter().position(|&p| p == station) else {
            return Vec::new();
        };
        let mut out: Vec<(i32, i32)> = Vec::new();
        let lo = si.saturating_sub(14);
        let hi = (si + 14).min(line.len().saturating_sub(1));
        for i in lo..=hi {
            let (x, y) = line[i];
            // The local direction of the line (its neighbours, not a guess).
            let prev = if i > 0 { Some(line[i - 1]) } else { None };
            let next = line.get(i + 1).copied();
            let (dx, dy) = match (prev, next) {
                (Some(p), Some(n)) => ((n.0 - p.0).signum(), (n.1 - p.1).signum()),
                (_, Some(n)) => ((n.0 - x).signum(), (n.1 - y).signum()),
                (Some(p), None) => ((x - p.0).signum(), (y - p.1).signum()),
                _ => (1, 0),
            };
            // Perpendicular to the direction, on the side the platform is *not*
            // on. A line running along x is offset in y, and the other way
            // round — the platform's own axis has nothing to do with it.
            let (px, py) = if dx.abs() >= dy.abs() {
                (0, if side_vec.1 != 0 { -side_vec.1 } else { 1 })
            } else {
                (if side_vec.0 != 0 { -side_vec.0 } else { 1 }, 0)
            };
            let (tx, ty) = (x + px, y + py);
            if !self.map.in_bounds(tx, ty) || self.map.at(tx, ty).is_water() {
                continue;
            }
            // A bend can fold the offset back onto the line: a second track on
            // top of the first is not a second track (B58).
            if line.contains(&(tx, ty)) {
                continue;
            }
            if let Some(&last) = out.last() {
                // B66: a gap of even one tile is a jump — the check used to
                // allow two, so a skipped tile (water, or a fold onto the
                // line) left the double track with a hole in it.
                if (last.0 - tx).abs() + (last.1 - ty).abs() > 1 {
                    // A jump: the parallel track ends here (the station stretch
                    // is the part that matters, and a kink across the first
                    // track would not be a track at all).
                    if i > si {
                        break;
                    }
                    out.clear();
                }
            }
            out.push((tx, ty));
        }
        out.dedup();
        out
    }

    /// The freight yard's sidings: three short parallel tracks that branch off
    /// the line into the industrial ground, and a couple of wagons standing on
    /// them (B58).
    fn plan_sidings(&mut self) {
        if self.rail.line.len() < 12 {
            return;
        }
        let Some(seed) = self.district_seed(District::Industrial) else {
            return;
        };
        let line = self.rail.line.clone();
        let station = self.rail.station.unwrap_or(seed);
        // The line tile closest to the industry, at least eight tiles from the
        // station so the yard is not the platform.
        let mut best: Option<(usize, i32)> = None;
        for (i, &(x, y)) in line.iter().enumerate() {
            let d = (x - seed.0).abs() + (y - seed.1).abs();
            let ds = (x - station.0).abs() + (y - station.1).abs();
            if ds < 8 {
                continue;
            }
            if best.map(|(_, b)| d < b).unwrap_or(true) {
                best = Some((i, d));
            }
        }
        let Some((i, _)) = best else { return };
        let (x, y) = line[i];
        // The line direction there, and the side the yard lies on.
        let (dx, dy) = if line.contains(&(x + 1, y)) {
            (1i32, 0i32)
        } else if line.contains(&(x, y + 1)) {
            (0, 1)
        } else {
            (1, 0)
        };
        let perp = if dx != 0 {
            if seed.1 >= y {
                (0, 1)
            } else {
                (0, -1)
            }
        } else if seed.0 >= x {
            (1, 0)
        } else {
            (-1, 0)
        };
        let mut sidings = Vec::new();
        let mut parked = Vec::new();
        for k in 1..=3i32 {
            let mut track = Vec::new();
            for s in 0..5i32 {
                let (tx, ty) = (x + dx * s + perp.0 * k, y + dy * s + perp.1 * k);
                if !self.map.in_bounds(tx, ty) || self.map.at(tx, ty).is_water() {
                    break;
                }
                track.push((tx, ty));
            }
            if track.len() >= 3 {
                // A wagon or two standing on each siding.
                if let Some(&w) = track.get(2) {
                    parked.push((w, (k as u8) % 3));
                }
                sidings.extend(track);
            }
        }
        if sidings.is_empty() {
            return;
        }
        // The yard is the box around its sidings.
        let xs: Vec<i32> = sidings.iter().map(|p| p.0).collect();
        let ys: Vec<i32> = sidings.iter().map(|p| p.1).collect();
        let x0 = xs.iter().min().copied().unwrap_or(x);
        let x1 = xs.iter().max().copied().unwrap_or(x);
        let y0 = ys.iter().min().copied().unwrap_or(y);
        let y1 = ys.iter().max().copied().unwrap_or(y);
        self.rail.sidings = sidings;
        self.rail.parked_wagons = parked;
        self.rail.yard = Some(Zone {
            x: (x0 - 1).max(1),
            y: (y0 - 1).max(1),
            w: (x1 - x0 + 3).max(3),
            h: (y1 - y0 + 3).max(3),
        });
    }

    /// The tram line: along the main avenue through the founding, from the
    /// oldest street to the downtown edge.
    fn plan_tram(&mut self) -> Vec<(i32, i32)> {
        let (fx, fy) = self.founding;
        let mut line: Vec<(i32, i32)> = Vec::new();
        // The main street is the avenue through the founding (B26b): walk it
        // both ways while it is paved.
        for dir in [-1i32, 1] {
            let mut x = fx;
            for _ in 0..24 {
                let t = self.map.at(x, fy);
                if !t.kind.is_lane() {
                    break;
                }
                line.push((x, fy));
                x += dir;
            }
        }
        line.sort_by_key(|&(x, _)| x);
        line.dedup();
        // Only the stretch inside the picture (B58): the avenue through the
        // founding is what the camera shows, and a stop behind the frame is a
        // stop nobody sees.
        line.retain(|&(x, y)| self.in_view(x, y, 26));
        if line.len() < 8 {
            // No long avenue: run along the longest straight lane row in view.
            line.clear();
            for dy in -10..=10 {
                let y = fy + dy;
                let mut run: Vec<(i32, i32)> = Vec::new();
                for x in (fx - 20).max(2)..=(fx + 20).min(self.map.w - 3) {
                    if self.map.at(x, y).kind.is_lane() {
                        run.push((x, y));
                    } else if !run.is_empty() {
                        break;
                    }
                }
                run.retain(|&(x, y)| self.in_view(x, y, 26));
                if run.len() > line.len() {
                    line = run;
                }
            }
        }
        line
    }

    fn district_seed(&self, d: District) -> Option<(i32, i32)> {
        self.districts
            .as_ref()?
            .iter()
            .find(|(_, dd)| *dd == d)
            .map(|(p, _)| *p)
    }

    /// A free rectangle in a district, for the yard.
    pub(crate) fn zone_in(&self, d: District, size: i32) -> Option<Zone> {
        let (fx, fy) = self.founding;
        for r in 2..24i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let (x, y) = (fx + dx * 1, fy + dy * 1);
                    if x < 3 || y < 3 || x + size > self.map.w - 3 || y + size > self.map.h - 3 {
                        continue;
                    }
                    if self.district_at(x, y) != d {
                        continue;
                    }
                    let mut ok = true;
                    for ty in y..y + size {
                        for tx in x..x + size {
                            let t = self.map.at(tx, ty);
                            if t.is_water() || t.kind.is_lane() || t.relief() > 1 {
                                ok = false;
                            }
                            if self.lots.iter().any(|l| l.covers(tx, ty)) {
                                ok = false;
                            }
                        }
                    }
                    if ok {
                        return Some(Zone {
                            x,
                            y,
                            w: size,
                            h: size,
                        });
                    }
                }
            }
        }
        None
    }

    /// A waterfront site: a free rectangle on land that meets water **in view**
    /// (B31b item 3). The marina and the port used to be sited inside a named
    /// district, which on a coastal map put them at the map corner where the
    /// sea is — outside the camera's diamond. Now the search runs over the
    /// visible band of the frame and scores by distance to the town, the kind of
    /// water and whether the water is on the camera side (so the ships and
    /// boats are drawn in front of the quay, not behind it).
    pub(crate) fn waterfront_zone(&self, d: District, size: i32, allow_river: bool) -> Option<Zone> {
        let (fx, fy) = self.founding;
        let mut best: Option<(Zone, i32)> = None;
        for y in 2..=(self.map.h - size - 2).max(3) {
            for x in 2..=(self.map.w - size - 2).max(3) {
                // Inside the picture, clear of the edge (B31b item 3): a quay
                // in the corner of the frame is not a waterfront anyone sees.
                if !self.in_frame(
                    x + size / 2,
                    y + size / 2,
                    self.map.at(x + size / 2, y + size / 2).min_level() as i32 * 6,
                    56,
                ) {
                    continue;
                }
                // The footprint has to be dry, flat and empty.
                let mut ok = true;
                'footprint: for ty in y..y + size {
                    for tx in x..x + size {
                        let t = self.map.at(tx, ty);
                        if t.is_water() || t.kind.is_lane() || t.relief() > 1 {
                            ok = false;
                            break 'footprint;
                        }
                        // A quay takes its land: the port clears the plots in its
                        // footprint (like the railway corridor). A marina is a
                        // pontoon and does not, so it must find open ground.
                        if !allow_river && self.lots.iter().any(|l| l.covers(tx, ty)) {
                            ok = false;
                            break 'footprint;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                // Below the land line, or its cranes stand in the sky (B58d).
                let mut shape = Vec::new();
                for ty in y..y + size {
                    for tx in x..x + size {
                        shape.push((tx, ty));
                    }
                }
                if !self.clear_of_belt(&shape, 24) {
                    continue;
                }
                // Which water does it face? The sea is best for a marina; a
                // river is enough for the container port (the brief allows it).
                let mut water = 0;
                let mut in_front = 0;
                let mut touching = false;
                for dy in -2..=size + 1 {
                    for dx in -2..=size + 1 {
                        let (wx, wy) = (x + dx, y + dy);
                        if !self.map.in_bounds(wx, wy) || !self.map.at(wx, wy).is_water() {
                            continue;
                        }
                        let k = self.map.water_kind(wx, wy);
                        let value = match k {
                            map::WaterKind::Sea => 3,
                            map::WaterKind::River => {
                                if allow_river {
                                    2
                                } else {
                                    0
                                }
                            }
                            _ => 1,
                        };
                        water = water.max(value);
                        // The quay has to reach the waterline — a 4-neighbour
                        // of the footprint, not a diagonal — otherwise the
                        // cranes stand on a strip of grass (B31b).
                        let along = (0..size).contains(&dx) || (0..size).contains(&dy);
                        if along && (dx == -1 || dy == -1 || dx == size || dy == size) {
                            touching = true;
                        }
                        // The camera looks from +x/+y: water there is in front.
                        if dx >= size / 2 && dy >= size / 2 {
                            in_front = 1;
                        }
                    }
                }
                if water == 0 || !touching {
                    continue;
                }
                let dist = (x - fx).abs() + (y - fy).abs();
                let in_district = self.district_at(x + size / 2, y + size / 2) == d;
                // Near the camera (large x + y) is *in front of* the city: a
                // quay behind the skyline is a quay nobody sees.
                let near = (x + y) / 2;
                let score = dist * 2 - water * 8 - in_front * 6 - near
                    + if in_district { 0 } else { 20 };
                if best.map(|(_, b)| score < b).unwrap_or(true) {
                    best = Some((
                        Zone {
                            x,
                            y,
                            w: size,
                            h: size,
                        },
                        score,
                    ));
                }
            }
        }
        best.map(|(z, _)| z)
    }

    /// The quay takes its land: plots inside the port footprint (and the one
    /// tile road along its edge) are cleared (B31b).
    pub(crate) fn clear_port_plots(&mut self, z: Zone) {
        let before = self.lots.len();
        self.lots.retain(|l| {
            !(l.y..l.y + l.h).any(|ty| (l.x..l.x + l.w).any(|tx| z.contains(tx, ty)))
        });
        let cleared = before - self.lots.len();
        if cleared > 0 {
            self.events.push(crate::Event {
                time: self.time,
                text: format!("the port takes its quay: {cleared} plots cleared"),
            });
        }
    }

    /// A marina: a pontoon **on the water** beside the shore, in view (B31b).
    /// Putting it on land was the reason it kept falling outside the frame: the
    /// only open, plot-free strips on this coast are the beach and the sea.
    pub(crate) fn marina_zone(&self, size: i32) -> Option<Zone> {
        let (fx, fy) = self.founding;
        let port = self.rail.port;
        let mut best: Option<(Zone, i32)> = None;
        for y in 6..=(self.map.h - size - 6).max(7) {
            for x in 6..=(self.map.w - size - 6).max(7) {
                if !self.in_frame(
                    x + size / 2,
                    y + size / 2,
                    self.map.at(x + size / 2, y + size / 2).min_level() as i32 * 6,
                    56,
                ) {
                    continue;
                }
                let mut all_water = true;
                let mut sea = 0;
                for ty in y..y + size {
                    for tx in x..x + size {
                        let t = self.map.at(tx, ty);
                        if !t.is_water() {
                            all_water = false;
                            break;
                        }
                        if self.map.water_kind(tx, ty) == map::WaterKind::Sea {
                            sea += 1;
                        }
                    }
                    if !all_water {
                        break;
                    }
                }
                if !all_water {
                    continue;
                }
                // B66: the pontoon has to **touch** the shore — a deck you can
                // step onto, not a raft out in the water. The old rule ("land
                // within two tiles") let a 4x4 pontoon float two tiles off a
                // beach, which drew as a brown grid of deck tiles in the middle
                // of the sea on the islands and lagoon frames. A face-adjacent
                // land tile (not a diagonal) is what a gangway needs.
                let land_at = |tx: i32, ty: i32| {
                    self.map.in_bounds(tx, ty) && !self.map.at(tx, ty).is_water()
                };
                let mut ashore = false;
                for k in 0..size {
                    if land_at(x - 1, y + k)
                        || land_at(x + size, y + k)
                        || land_at(x + k, y - 1)
                        || land_at(x + k, y + size)
                    {
                        ashore = true;
                        break;
                    }
                }
                if !ashore {
                    continue;
                }
                // The pontoon must be below the land line too (B58d).
                let mut shape = Vec::new();
                for ty in y..y + size {
                    for tx in x..x + size {
                        shape.push((tx, ty));
                    }
                }
                if !self.clear_of_belt(&shape, 24) {
                    continue;
                }
                let dist = (x - fx).abs() + (y - fy).abs();
                // The marina belongs to the high-end quarter and away from the
                // container port: two waterfronts, not one.
                let mut score = dist * 2 - (sea * 20) / (size * size);
                if self.district_at(x + size / 2, y + size / 2) != District::HighEnd {
                    score += 40;
                }
                if let Some(p) = port {
                    let (px, py) = p.centre();
                    if (px - x).abs() + (py - y).abs() < 12 {
                        score += 70;
                    }
                }
                if best.map(|(_, b)| score < b).unwrap_or(true) {
                    best = Some((
                        Zone {
                            x,
                            y,
                            w: size,
                            h: size,
                        },
                        score,
                    ));
                }
            }
        }
        best.map(|(z, _)| z)
    }

    /// Stack containers in the port zone (the renderer draws them as boxes).
    pub(crate) fn stack_containers(&mut self, z: Zone) {
        let mut rng = self.rng(0xC047);
        for i in 0..(z.w * z.h / 3).max(1) {
            let x = z.x + (i % z.w);
            let y = z.y + (i / z.w) % z.h;
            self.rail.containers.push((x, y, rng.usize(5) as u8));
        }
    }

    /// Advance the trains, trams, boats and planes by `dt` seconds, and put new
    /// ones on the map on their timetable.
    pub fn transport_step(&mut self, dt: f64) {
        self.plan_rail();
        self.rail.tick = self.rail.tick.wrapping_add(1);
        self.rail.clock += dt;
        let d = dt as f32;
        // B79 (D91): how much room each boat has ahead, and its lane.
        for m in self.rail.movers.iter_mut() {
            let is_boat = matches!(m.kind, MoverKind::Boat | MoverKind::Ferry | MoverKind::Ship);
            // Every boat moves in `isles_step`, where the room around it is
            // known.
            if is_boat {
                continue;
            }
            if m.dwell > 0.0 {
                m.dwell -= d;
                continue;
            }
            m.travelled += m.speed * d;
            if m.cooldown > 0.0 {
                m.cooldown -= d;
            }
        }
        // Trams call at tram stops, trains at the station and buses at their
        // own line's stops (B31b/B58): a pause, then a cooldown so the mover
        // cannot get stuck on the stop.
        let stops = self.rail.tram_stops.clone();
        let station = self.rail.station;
        let bus_stops: Vec<Vec<(i32, i32)>> = self
            .rail
            .bus_lines
            .iter()
            .map(|l| l.stops.clone())
            .collect();
        for m in &mut self.rail.movers {
            if m.dwell > 0.0 || m.cooldown > 0.0 {
                continue;
            }
            let at = match m.kind {
                MoverKind::Tram => stops.contains(&m.tile()),
                MoverKind::Train => station == Some(m.tile()) && m.travelled > 2.0,
                MoverKind::Bus => bus_stops
                    .get(m.line as usize)
                    .is_some_and(|s| s.contains(&m.tile())),
                _ => false,
            };
            if at {
                m.dwell = match m.kind {
                    MoverKind::Train => 8.0,
                    MoverKind::Bus => 6.0,
                    MoverKind::Tram => 5.0,
                    _ => 4.0,
                };
                m.cooldown = if m.kind == MoverKind::Bus { 25.0 } else { 40.0 };
            }
        }
        // Retire trains that reached the end of the line (a new one follows),
        // and buses and aircraft that finished their run.
        self.rail.movers.retain(|m| match m.kind {
            MoverKind::Boat | MoverKind::Ferry | MoverKind::Ship if m.mode & 0x40 != 0 && m.dwell <= 0.0 => false,
            MoverKind::Train | MoverKind::Bus | MoverKind::Aircraft => {
                m.travelled < m.route_len()
            }
            _ => true,
        });
        if self.rail.line.len() > 8 && self.rail.clock >= self.rail.next_train {
            let by_day = (self.rail_rng(0x7A11_0002).range_f32(70.0, 150.0)) as f64;
            // More trains by day than at night.
            let hod = (self.time / 3600.0).rem_euclid(24.0);
            let day = (6.0..19.0).contains(&hod);
            self.rail.next_train = self.rail.clock + by_day * if day { 1.0 } else { 2.4 };
            let freight = self.rail_rng(0x7A11_0003).chance(0.45);
            let route = self.rail.line.clone();
            // B58: the train has to fit the platform — a halt takes two coaches,
            // a terminal six — and the passenger sets are diesel until the line
            // is electrified, electric after (a visible era change, B49 drives
            // the flag).
            let platform = self
                .rail
                .station_build
                .as_ref()
                .map(|s| s.kind.platform_len())
                .unwrap_or(3) as u8;
            let (set, cars, speed) = if freight {
                (
                    2u8,
                    (6 + self.rail_rng(0x7A11_0004).usize(4)) as u8,
                    2.9f32,
                )
            } else if self.rail.electrified {
                (
                    1,
                    platform + 1,
                    if self.era() >= Era::Metropolis { 4.6 } else { 4.2 },
                )
            } else {
                (0, platform + 1, 3.6)
            };
            self.rail.movers.push(Mover {
                kind: MoverKind::Train,
                route,
                travelled: 0.0,
                speed,
                shuttle: false,
                back: false,
                cars,
                lift: 0,
                dwell: 0.0,
                cooldown: 0.0,
                set,
                line: 0,
                mode: 0,
                phase: 0.0,
            });
            self.rail.trains += 1;
        }
        // Trams: two or three on the avenue, shuttling.
        if self.rail.tram_line.len() > 8 && self.era() >= Era::City {
            let trams = self
                .rail
                .movers
                .iter()
                .filter(|m| m.kind == MoverKind::Tram)
                .count();
            // A night service, not a fleet (B58: the city idles after dark).
            let hod = (self.time / 3600.0).rem_euclid(24.0);
            let want = if (6.0..23.0).contains(&hod) { 3 } else { 1 };
            if trams < want && self.rail_rng(0x7A11_0006).chance(0.02) {
                self.rail.movers.push(Mover {
                    kind: MoverKind::Tram,
                    route: self.rail.tram_line.clone(),
                    travelled: 0.0,
                    speed: 1.9,
                    shuttle: true,
                    back: false,
                    cars: 1,
                    lift: 0,
                    dwell: 0.0,
                    cooldown: 0.0,
                    set: 0,
                    line: 0,
                    mode: 0,
                    phase: 0.0,
                });
            }
        }
        // Boats: on the water nearest the city.
        let legacy_boats = self.rail.movers.iter().filter(|m| m.line == 0 && m.kind == MoverKind::Boat).count();
        if self.rail.clock >= self.rail.next_boat && legacy_boats < 3 {
            self.rail.next_boat = self.rail.clock + self.rail_rng(0x7A11_0007).range_f32(150.0, 420.0) as f64;
            if let Some((route, water)) = self.water_route() {
                let (set, speed, tow) = self.pick_boat(water, route.len());
                self.push_boat(Mover {
                    kind: MoverKind::Boat,
                    route,
                    travelled: 0.0,
                    speed,
                    shuttle: true,
                    back: false,
                    cars: if tow { 1 } else { 0 },
                    lift: 0,
                    dwell: 0.0,
                    cooldown: 0.0,
                    set,
                    line: 0,
                    mode: 0,
                    phase: 0.0,
                });
                self.rail.boats_seen += 1;
            }
        }
        // A container ship at the port: in from the open water, alongside for a
        // while, out again (B31b).
        if self.rail.port.is_some() && self.rail.clock >= self.rail.next_ship && !self.rail.movers.iter().any(|m| m.line == 0 && m.kind == MoverKind::Ship) {
            self.rail.next_ship = self.rail.clock + self.rail_rng(0x7A11_000C).range_f32(240.0, 600.0) as f64;
            if let Some(route) = self.ship_route() {
                self.push_boat(Mover {
                    kind: MoverKind::Ship,
                    route,
                    travelled: 0.0,
                    speed: 0.5,
                    shuttle: true,
                    back: false,
                    cars: 0,
                    lift: 0,
                    dwell: 0.0,
                    cooldown: 0.0,
                    set: self.pick_ship(),
                    line: 0,
                    mode: 0,
                    phase: 0.0,
                });
            }
        }
        // Planes: across the sky band now and then. When the city has an
        // airfield they climb out of it (B58: the band's traffic connects to
        // the runway instead of crossing the frame from nowhere).
        if self.rail.clock >= self.rail.next_plane {
            self.rail.next_plane = self.rail.clock + self.rail_rng(0x7A11_0009).range_f32(300.0, 900.0) as f64;
            // Across the *visible* diamond, in front of the camera — a plane on
            // the far side of the map is a plane nobody sees (B31b).
            let (fx, fy) = self.founding;
            let jitter = self.rail_rng(0x7A11_000A).usize(9) as i32 - 4;
            let (mut sx, mut sy) = ((fx - 26).max(2), (fy - 3 + jitter).clamp(3, self.map.h - 4));
            if let Some(ap) = &self.rail.airport {
                if let Some(&(rx, ry)) = ap.runway.last() {
                    sx = rx;
                    sy = ry;
                }
            }
            let (ex, ey) = ((fx + 26).min(self.map.w - 3), (sy + 4).min(self.map.h - 4));
            let start = (sx, sy);
            let end = (ex, ey);
            let lift = 60 + self.rail_rng(0x7A11_000B).usize(60) as i32;
            self.rail.movers.push(Mover {
                kind: MoverKind::Plane,
                route: vec![start, end],
                travelled: 0.0,
                speed: 8.0,
                shuttle: false,
                back: false,
                cars: 0,
                lift,
                dwell: 0.0,
                cooldown: 0.0,
                set: 0,
                line: 0,
                mode: 0,
                phase: 0.0,
            });
        }
        // B58: buses, ferries, aircraft and the cranes' trolleys.
        self.transit_step(dt);
        self.plan_cranes();
        self.isles_step(dt);
        self.rail.movers.retain(|m| {
            m.kind != MoverKind::Plane || m.travelled < m.route_len()
        });
    }

    /// The container ship's lane: **along the quay**, two tiles out, so the ship
    /// sits beside the cranes instead of wandering off to the edge of the map
    /// (B31b).
    fn ship_route(&self) -> Option<Vec<(i32, i32)>> {
        let port = self.rail.port?;
        // Every water tile two out from a quay edge tile.
        let mut edge: Vec<(i32, i32)> = Vec::new();
        for ty in port.y..port.y + port.h {
            for tx in port.x..port.x + port.w {
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (wx, wy) = (tx + dx * 2, ty + dy * 2);
                    if self.map.in_bounds(wx, wy) && self.map.at(wx, wy).is_water() {
                        edge.push((wx, wy));
                    }
                }
            }
        }
        edge.sort_unstable();
        edge.dedup();
        if edge.len() < 4 {
            return None;
        }
        // Run along whichever axis the edge is longer in.
        let (x0, x1) = (edge[0].0, edge[edge.len() - 1].0);
        let span_x = x1 - x0;
        let span_y = edge.iter().map(|p| p.1).max()? - edge.iter().map(|p| p.1).min()?;
        let mut route = edge;
        if span_x >= span_y {
            route.sort_by_key(|p| (p.0, p.1));
        } else {
            route.sort_by_key(|p| (p.1, p.0));
        }
        // Keep only a connected chain: drop tiles that jump.
        let mut out: Vec<(i32, i32)> = Vec::new();
        for p in route {
            match out.last() {
                Some(&q) if (p.0 - q.0).abs() + (p.1 - q.1).abs() <= 2 => out.push(p),
                None => out.push(p),
                _ => {}
            }
        }
        if out.len() < 4 {
            return None;
        }
        Some(out)
    }

    /// A short route over water **in the frame** near the city: a lake reach, a
    /// river stretch or the sea shelf (B31b: the old walk needed a 4-tile run of
    /// one water kind starting 8–16 tiles out, which most maps do not have, so
    /// no boat ever appeared).
    fn water_route(&self) -> Option<(Vec<(i32, i32)>, map::WaterKind)> {
        let (fx, fy) = self.founding;
        let mut best: Option<(Vec<(i32, i32)>, i32, map::WaterKind)> = None;
        for y in 2..self.map.h - 3 {
            for x in 2..self.map.w - 3 {
                if !self.map.at(x, y).is_water() {
                    continue;
                }
                // Open water in the picture, near enough to the town to read as
                // the city's water.
                let d = (x - fx).abs() + (y - fy).abs();
                if d > 34 {
                    continue;
                }
                if !self.in_frame(x, y, self.map.at(x, y).min_level() as i32 * 6, 40) {
                    continue;
                }
                let kind = self.map.water_kind(x, y);
                // The longest straight run through here, in both axes.
                let mut runs: Vec<Vec<(i32, i32)>> = Vec::new();
                for (dx, dy) in [(1, 0), (0, 1)] {
                    let mut run = vec![(x, y)];
                    for sgn in [1i32, -1] {
                        let mut cur = (x, y);
                        for _ in 0..20 {
                            let n = (cur.0 + dx * sgn, cur.1 + dy * sgn);
                            if !self.map.in_bounds(n.0, n.1) {
                                break;
                            }
                            if !self.map.at(n.0, n.1).is_water()
                                || self.map.water_kind(n.0, n.1) != kind
                            {
                                break;
                            }
                            run.push(n);
                            cur = n;
                        }
                    }
                    run.sort_unstable();
                    run.dedup();
                    runs.push(run);
                }
                runs.sort_by_key(|r| std::cmp::Reverse(r.len()));
                let run = runs.remove(0);
                // B79: not through a jetty's berths.
                if run.len() < 4 || run.iter().any(|&(rx, ry)| self.near_jetty(rx, ry, 2)) {
                    continue;
                }
                // Longest wins, then nearest, then the open kind over a pond.
                let open = match kind {
                    map::WaterKind::Sea => 2,
                    map::WaterKind::River => 1,
                    _ => 0,
                };
                let score = run.len() as i32 * 4 + open * 6 - d;
                if best.as_ref().map(|(_, b, _)| score > *b).unwrap_or(true) {
                    best = Some((run, score, kind));
                }
            }
        }
        best.map(|(r, _, k)| (r, k))
    }

    /// B91: which boat sails this water. The low five bits of the returned
    /// `set` are the boat kit's small-craft index (`render::boats::SMALL`), the
    /// top three the variant (paint, trim, sail, wear). Returns (set, speed,
    /// tows a barge).
    ///
    /// Indices: 0 water taxi, 1 rowing boat, 2 dinghy, 3 sailboat, 4 sloop,
    /// 5 motor yacht, 6 fishing boat, 7 lobster boat, 8 speedboat, 9 kayak,
    /// 10 houseboat, 11 tug, 12 barge.
    fn pick_boat(&self, water: map::WaterKind, route_len: usize) -> (u8, f32, bool) {
        let mut rng = self.rail_rng(0x7A11_0008);
        let era = self.era();
        let harbour = self.rail.harbour.is_some() || self.rail.marina.is_some();
        // (kind index, weight): a lake or pond is for rowing and dinghies, a
        // river for barges and tugs, the sea for fishing boats and yachts.
        let mut table: Vec<(u8, u32)> = match water {
            map::WaterKind::Pond | map::WaterKind::Lake => {
                vec![(1, 5), (2, 4), (9, 4), (3, 3), (6, 1), (0, 1), (10, 1)]
            }
            map::WaterKind::River => vec![(12, 4), (11, 3), (0, 2), (1, 3), (9, 2), (6, 2), (2, 1), (5, 1)],
            _ => vec![(3, 4), (6, 4), (4, 2), (7, 2), (2, 1), (0, 1), (5, 1), (8, 1)],
        };
        if harbour {
            for e in table.iter_mut() {
                if matches!(e.0, 6 | 7 | 4 | 5 | 0) {
                    e.1 += 2;
                }
            }
        }
        // Early settlements: oars, sails, nets and plain barges only.
        if era < Era::Town {
            table.retain(|e| matches!(e.0, 1 | 2 | 3 | 6 | 9 | 12 | 10));
        } else if era < Era::City {
            table.retain(|e| !matches!(e.0, 8 | 5));
        }
        if table.is_empty() {
            table.push((1, 1));
        }
        let total: u32 = table.iter().map(|e| e.1).sum();
        let mut roll = rng.usize(total as usize) as u32;
        let mut kind = table[0].0;
        for e in &table {
            if roll < e.1 {
                kind = e.0;
                break;
            }
            roll -= e.1;
        }
        let variant = rng.usize(8) as u8;
        let speed = match kind {
            1 | 9 => 0.55,
            2 | 3 | 4 => 0.8,
            10 => 0.4,
            11 | 12 => 0.7,
            8 => 1.7,
            6 | 7 => 0.9,
            _ => 1.1,
        };
        // A tug on a route long enough to tow a barge behind it.
        let tow = kind == 11 && route_len >= 9;
        (kind | (variant << 5), speed, tow)
    }

    /// B91: which ship comes to the port: a container ship at a big port in a
    /// big city, otherwise a coaster. Variant in the top three bits.
    pub(crate) fn ferry_set(&self) -> u8 {
        let mut rng = self.rail_rng(0x7A11_000E);
        // A passenger ferry or a car ferry (the kit decodes even/odd).
        let kind: u8 = if rng.chance(0.5) { 0 } else { 1 };
        kind | ((rng.usize(8) as u8) << 5)
    }

    fn pick_ship(&self) -> u8 {
        let mut rng = self.rail_rng(0x7A11_000D);
        let big = self.era() >= Era::Metropolis || self.rail.port.map(|p| p.w * p.h >= 40).unwrap_or(false);
        let kind: u8 = if big && rng.chance(0.65) { 0 } else { 1 };
        kind | ((rng.usize(8) as u8) << 5)
    }

    /// The movers to draw (trains, trams, boats, planes).
    pub fn transport(&self) -> &[Mover] {
        &self.rail.movers
    }

    /// Is this tile part of the rail line (the renderer bakes the track)?
    pub fn rail_tiles(&self) -> &[(i32, i32)] {
        &self.rail.line
    }

    /// Is the tile a river (a barge may only use the river, a boat the lake/sea)?
    pub fn river_tile(&self, x: i32, y: i32) -> bool {
        self.map.at(x, y).is_water() && self.map.water_kind(x, y) == WaterKind::River
    }

    /// A level crossing is where the rail line meets a street: the renderer draws
    /// the sleepers, the sim only reports the tiles.
    pub fn level_crossings(&self) -> Vec<(i32, i32)> {
        self.rail
            .line
            .iter()
            .copied()
            .filter(|&(x, y)| self.map.at(x, y).kind == Kind::Paved || self.map.at(x, y).kind == Kind::Road)
            .collect()
    }

    /// The gantry cranes at the container port: one trolley per crane, cycling
    /// out over the ship and back while the city is big enough to work it
    /// (B58). Their mast is baked; the trolley and its container are movers, so
    /// they animate on a dirty rect like everything else.
    fn plan_cranes(&mut self) {
        if self.rail.port.is_none() || self.era() < Era::City {
            return;
        }
        if self
            .rail
            .movers
            .iter()
            .any(|m| m.kind == MoverKind::Crane)
        {
            return;
        }
        let Some(p) = self.rail.port else { return };
        let mut quay = 0;
        for ty in p.y..p.y + p.h {
            for tx in p.x..p.x + p.w {
                let water = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .find(|(dx, dy)| self.map.at(tx + dx, ty + dy).is_water())
                    .copied();
                let Some((wx, wy)) = water else { continue };
                quay += 1;
                if quay % 3 != 1 {
                    continue;
                }
                // The route is the quay tile and the water it reaches over, so
                // the pose's direction is the boom's.
                self.rail.movers.push(Mover {
                    kind: MoverKind::Crane,
                    route: vec![(tx, ty), (tx + wx, ty + wy)],
                    travelled: 0.0,
                    speed: 0.0,
                    shuttle: false,
                    back: false,
                    cars: 0,
                    lift: 0,
                    dwell: 0.0,
                    cooldown: 0.0,
                    set: 0,
                    line: 0,
                    mode: 0,
                    phase: (quay as f32 * 0.29).fract(),
                });
            }
        }
    }

    /// The people the transport puts on the street: waiting on the platform,
    /// under the tram and bus shelters, and going in and out of the subway at
    /// rush hour (B58). Deterministic from the tile, the hour and the seed.
    pub fn waiting_figures(&self, hour: f64, out: &mut Vec<crate::people::Figure>) {
        use crate::people::{Figure, Pose, Prop};
        // Nobody waits for a train at three in the morning.
        if !(6.0..22.0).contains(&hour) {
            return;
        }
        let rush = (7.2..9.4).contains(&hour) || (16.4..18.6).contains(&hour);
        let per_tile = if rush { 2 } else { 1 };
        let mut add = |x: f32, y: f32, k: u32, dir: u8| {
            let r = |s: u32| tile_hash(x.floor() as i32, y.floor() as i32, k * 7 + s, self.seed);
            out.push(Figure {
                x: x + 0.15 + r(1) * 0.6,
                y: y + 0.15 + r(2) * 0.6,
                pose: if r(3) < 0.25 { Pose::Chat } else { Pose::Stand },
                tone: (r(4) * 7.9) as u8,
                child: r(5) < 0.12,
                prop: if r(6) < 0.10 { Prop::Dog } else { Prop::None },
                dir,
                open: false,
                lift: 0,
            });
        };
        // The platform: people wait beside the train, facing the track.
        if let Some(sb) = &self.rail.station_build {
            for &(x, y, side) in &sb.platforms {
                let face = match side {
                    0 => 2u8,
                    1 => 0,
                    2 => 3,
                    _ => 1,
                };
                let n = if sb.kind == StationKind::Halt {
                    per_tile.min(1)
                } else {
                    per_tile
                };
                for k in 0..n {
                    add(x as f32, y as f32, k as u32 + 11, face);
                }
            }
        }
        // Tram stops and bus stops: one or two under the shelter.
        for (i, &(x, y)) in self.rail.tram_stops.iter().enumerate() {
            for k in 0..per_tile.min(2) {
                add(x as f32, y as f32, 31 + i as u32 * 3 + k, 0);
            }
        }
        let stops = self.bus_stops();
        for (i, &(x, y, _)) in stops.iter().enumerate() {
            for k in 0..per_tile.min(2) {
                add(x as f32, y as f32, 71 + i as u32 * 3 + k, 0);
            }
        }
        // The subway: only once the entrance is open (stage 4), and at rush
        // hour the crowd on the stairs is what makes the station read.
        if let Some(s) = &self.rail.subway {
            if let Some((stage, _)) = self.subway_works() {
                if stage >= 4 {
                    for (i, st) in s.stations.iter().enumerate() {
                        let (ex, ey, _) = st.entrance;
                        let n = if rush { 3 } else { 1 };
                        for k in 0..n {
                            add(ex as f32, ey as f32, 131 + i as u32 * 5 + k, 0);
                        }
                    }
                }
            }
        }
    }
}

/// A cheap deterministic hash in `0..1` from a tile, a salt and the seed.
fn tile_hash(x: i32, y: i32, k: u32, seed: u64) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (k as u64).wrapping_mul(0x1656_67B1_9E37_79F9)
        ^ seed;
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h % 10_000) as f32 / 10_000.0
}
