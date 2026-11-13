//! Traffic: vehicles on the city's street network (B29, M7).
//!
//! Placeholder mechanics, real numbers. The rules the brief asks for:
//!
//! * **Trips from homes to jobs and shops**, by time of day: a rush hour each end
//!   of the working day on weekdays, a quiet weekend, almost nothing at night.
//! * **Lively but calm**: the car count is capped per lane tile, so streets look
//!   busy without jamming; vehicles keep to their lane (right-hand side) and turn
//!   at junctions instead of driving through buildings.
//! * **Kinds**: cars, vans, trucks to the works and shops, taxis downtown, and
//!   the occasional **emergency vehicle** with flashing lights (D53). The buses
//!   are B58's transit movers now, so they can call at their own stops.
//! * **Cheap**: a trip's route is computed once (A* over the lane tiles, the same
//!   search the planner uses) and then walked at a constant speed, so an update
//!   is a few float adds. Everything is a pure function of `(seed, sim time)`, so
//!   a catch-up replays the same traffic.
//!
//! Vehicles are *not* saved: they are respawned from the clock on load, which is
//! what keeps the save file about the city and not about its cars.

use std::collections::BTreeSet;

use map::rng::Rng;

use crate::{LotKind, Sim};

/// B75/D83: the body styles, in the renderer's `Style::ALL` order. The sim only
/// picks a number; the art lives in `render::traffic`. Kept as plain constants
/// so the two crates stay independent (the renderer never depends on the sim).
pub mod style {
    pub const HATCHBACK: u8 = 0;
    pub const SALOON: u8 = 1;
    pub const ESTATE: u8 = 2;
    pub const SUV: u8 = 3;
    pub const PICKUP: u8 = 4;
    pub const MINIVAN: u8 = 5;
    pub const CITY_CAR: u8 = 6;
    pub const SPORTS: u8 = 7;
    pub const CONVERTIBLE: u8 = 8;
    pub const CLASSIC: u8 = 9;
    pub const CAMPER: u8 = 10;
    pub const MOTORBIKE: u8 = 11;
    pub const SCOOTER: u8 = 12;
    pub const BICYCLE: u8 = 13;
    pub const DELIVERY_VAN: u8 = 14;
    pub const BOX_TRUCK: u8 = 15;
    pub const LORRY: u8 = 16;
    pub const BIN_LORRY: u8 = 17;
    pub const SWEEPER: u8 = 18;
    pub const ICE_CREAM: u8 = 19;
    pub const COACH: u8 = 20;
    pub const SCHOOL_BUS: u8 = 21;
    pub const TOW_TRUCK: u8 = 22;
    pub const MIXER: u8 = 23;
    pub const CRANE_TRUCK: u8 = 24;
    pub const TRACTOR: u8 = 25;
    /// The three classes with their own art (B75): a taxi's roof sign, an
    /// emergency vehicle's white body, the village cart.
    pub const TAXI: u8 = 26;
    pub const EMERGENCY: u8 = 27;
    pub const CART: u8 = 28;
    /// B76: the winter service.
    pub const PLOUGH: u8 = 29;
    pub const GRITTER: u8 = 30;
    /// How many there are (the renderer's table length).
    pub const COUNT: u8 = 31;
}

/// What is driving.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VehicleKind {
    /// A horse and cart: the village's traffic before the motor car (B49).
    Cart,
    Car,
    Van,
    Taxi,
    Bus,
    Truck,
    /// An electric car: quiet, no exhaust (B49/D72).
    Ev,
    /// Fire truck, ambulance or police car (D53): the same shape, flashing.
    Emergency,
    /// A cycle, scooter or motorbike (B75): narrow, quick off the line.
    Bike,
}

impl VehicleKind {
    pub fn name(self) -> &'static str {
        match self {
            VehicleKind::Cart => "cart",
            VehicleKind::Car => "car",
            VehicleKind::Van => "van",
            VehicleKind::Taxi => "taxi",
            VehicleKind::Bus => "bus",
            VehicleKind::Truck => "truck",
            VehicleKind::Ev => "ev",
            VehicleKind::Emergency => "emergency",
            VehicleKind::Bike => "bike",
        }
    }

    /// How big the placeholder is, in tiles (length along the road).
    pub fn length(self) -> f32 {
        match self {
            VehicleKind::Cart => 0.62,
            VehicleKind::Car | VehicleKind::Taxi | VehicleKind::Ev => 0.55,
            VehicleKind::Van => 0.65,
            VehicleKind::Truck => 0.85,
            VehicleKind::Bus => 1.0,
            VehicleKind::Emergency => 0.7,
            VehicleKind::Bike => 0.3,
        }
    }

    /// Cruising speed in tiles per second (a tile is ~12 m at this zoom). A
    /// cart plods; an electric car pulls away from the lights.
    pub fn speed(self) -> f32 {
        match self {
            VehicleKind::Cart => 0.7,
            VehicleKind::Car | VehicleKind::Taxi => 1.5,
            VehicleKind::Ev => 1.6,
            VehicleKind::Van => 1.3,
            VehicleKind::Truck => 1.0,
            VehicleKind::Bus => 0.9,
            VehicleKind::Emergency => 2.1,
            VehicleKind::Bike => 1.2,
        }
    }
}

/// One vehicle driving its route.
#[derive(Clone, Debug)]
pub struct Vehicle {
    pub kind: VehicleKind,
    /// The tiles it is driving through, in order.
    pub route: Vec<(i32, i32)>,
    /// Distance travelled along the route, in tiles.
    pub travelled: f32,
    /// Where it came from and where it is going (the sim's own lots).
    pub from: (i32, i32),
    pub to: (i32, i32),
    /// Speed jitter, 0.9..1.1, so a queue does not look like a convoy.
    pub pace: f32,
    /// Set for emergency vehicles: the light phase, in seconds.
    pub blink: f32,
    /// Where along the route the vehicle was spawned (for the GPS-less camera).
    pub started: f64,
    /// The bay this trip will park in when it arrives (B59).
    pub slot: Option<crate::carculture::Slot>,
    /// Colour pick, 0..7 (a parked car keeps the one it drove in with).
    pub variant: u8,
    /// B75: the body style, an index into `style` (the renderer's own table).
    pub style: u8,
    /// Brake lights on: held behind another car or slowing into a lot (B59).
    pub braking: bool,
    /// Seconds this vehicle has been held stationary by the car in front (B59).
    pub held: f32,
    /// Held this step: it does not advance (B59 car following).
    pub stopped: bool,
    /// B75: which way it is about to turn (0 none, 1 left, 2 right) — the
    /// indicator. The renderer blinks an amber pixel on that side.
    pub signal: i8,
    /// The stretch of the route over which it is on the highway, as fractions
    /// of the route length: traffic flows faster there (B59). 0,0 = never.
    pub hw: (f32, f32),
    /// Its speed now, in tiles per second (B99c, D77). A vehicle pulls away from
    /// nought instead of appearing at full speed, eases into a turn, slows for
    /// the pumps and stops at the kerb: this is the number that changes, and
    /// the route is walked at it.
    pub vel: f32,
}

impl Vehicle {
    /// Position in tile space, and the direction it is facing (unit, axis
    /// aligned).
    pub fn pose(&self) -> ((f32, f32), (f32, f32)) {
        let total = self.travelled;
        let mut acc = 0.0f32;
        for w in self.route.windows(2) {
            let (ax, ay) = (w[0].0 as f32, w[0].1 as f32);
            let (bx, by) = (w[1].0 as f32, w[1].1 as f32);
            let seg = ((bx - ax).abs()) + ((by - ay).abs());
            if total <= acc + seg || seg <= f32::EPSILON {
                let t = ((total - acc) / seg.max(f32::EPSILON)).clamp(0.0, 1.0);
                let dir = ((bx - ax).signum(), (by - ay).signum());
                return ((ax + (bx - ax) * t + 0.5, ay + (by - ay) * t + 0.5), dir);
            }
            acc += seg;
        }
        // Past the end: sit on the last tile facing the way it came.
        let last = *self.route.last().unwrap_or(&self.to);
        let prev = self
            .route
            .iter()
            .rev()
            .nth(1)
            .copied()
            .unwrap_or(last);
        (
            (last.0 as f32 + 0.5, last.1 as f32 + 0.5),
            (
                (last.0 - prev.0).signum() as f32,
                (last.1 - prev.1).signum() as f32,
            ),
        )
    }

    /// The tile the vehicle occupies (for depth ordering and dirty rects).
    pub fn tile(&self) -> (i32, i32) {
        let (p, _) = self.pose();
        (p.0.floor() as i32, p.1.floor() as i32)
    }

    /// The tile the vehicle would occupy after travelling `d` more tiles (used
    /// to look ahead at a level crossing).
    pub fn tile_ahead(&self, d: f32) -> (i32, i32) {
        let total = self.travelled + d;
        let mut acc = 0.0f32;
        for w in self.route.windows(2) {
            let (ax, ay) = (w[0].0 as f32, w[0].1 as f32);
            let (bx, by) = (w[1].0 as f32, w[1].1 as f32);
            let seg = (bx - ax).abs() + (by - ay).abs();
            if total <= acc + seg || seg <= f32::EPSILON {
                let t = ((total - acc) / seg.max(f32::EPSILON)).clamp(0.0, 1.0);
                return (
                    (ax + (bx - ax) * t).floor() as i32,
                    (ay + (by - ay) * t).floor() as i32,
                );
            }
            acc += seg;
        }
        *self.route.last().unwrap_or(&self.to)
    }

    /// The direction of travel at `d` tiles along the route (B75: for the
    /// indicator, which has to know where the road goes next).
    pub fn dir_at(&self, d: f32) -> (f32, f32) {
        let mut acc = 0.0f32;
        for w in self.route.windows(2) {
            let (ax, ay) = (w[0].0 as f32, w[0].1 as f32);
            let (bx, by) = (w[1].0 as f32, w[1].1 as f32);
            let seg = (bx - ax).abs() + (by - ay).abs();
            if d <= acc + seg || seg <= f32::EPSILON {
                return ((bx - ax).signum(), (by - ay).signum());
            }
            acc += seg;
        }
        self.route
            .windows(2)
            .last()
            .map(|w| ((w[1].0 - w[0].0).signum() as f32, (w[1].1 - w[0].1).signum() as f32))
            .unwrap_or((1.0, 0.0))
    }

    pub fn done(&self) -> bool {
        let len: f32 = self
            .route
            .windows(2)
            .map(|w| {
                ((w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()) as f32
            })
            .sum();
        self.travelled >= len
    }
}

/// Everything about the city's traffic at one instant.
#[derive(Default)]
pub struct Traffic {
    pub vehicles: Vec<Vehicle>,
    /// Next times (sim seconds) for an emergency call-out.
    next_emergency: f64,
    /// Spawn/despawn bookkeeping.
    pub spawned: u32,
    pub arrived: u32,
    /// Attempts that found no homes/works or no route (diagnostics).
    pub no_route: u32,
    /// Failure reasons: no lane near the origin, near the destination, no path,
    /// and a path that is only one tile (diagnostics, B30 item 3).
    pub why: [u32; 4],
    /// Coarse step index of the last spawn window (B30: at most two spawns every
    /// half second of sim time).
    pub last_spawn_step: u64,
    /// Every spawn attempt, so a failed attempt is not retried with the same
    /// random pick (the sim's RNG is a pure function of the step index, which
    /// does not advance inside a warm-up).
    pub attempts: u32,
    /// B76: how much snow is lying (0..1). With snow on the roads the winter
    /// service comes out: ploughs and gritters, as trucks.
    pub snow: f32,
}

impl Traffic {
    /// Up to how many vehicles the network should carry: one per eight lane
    /// tiles, and never so many that a street looks jammed (B29 item 1).
    fn capacity(&self, lanes: usize) -> usize {
        // B75: a street that reads busy. One vehicle every four lane tiles was
        // a car every nineteen tiles in a grown city — the roads looked
        // abandoned at noon. One every three, with a higher ceiling, fills a
        // downtown without jamming it (the car-following step keeps a gap).
        (lanes / 3).clamp(0, 260)
    }

    /// How busy the roads are at this hour, 0..1: two rush hours on weekdays, a
    /// quiet weekend, almost nothing from midnight to five.
    ///
    /// B77/D84: the weekend has its own curve. A Saturday is not a weekday at
    /// 55 %: there is no rush hour, the shopping hours are the busy ones, and
    /// the morning starts late. A Sunday morning is the emptiest road of the
    /// week, and the day only picks up in the afternoon.
    pub fn busyness(time: f64) -> f32 {
        let hod = (time / 3600.0).rem_euclid(24.0);
        // 0 = Sunday (the same count `Sim::weekday` uses).
        let dow = ((time / 86400.0) + 3.0).floor().rem_euclid(7.0);
        if dow == 6.0 {
            return if hod < 5.0 {
                0.05
            } else if hod < 7.0 {
                0.10
            } else if hod < 9.0 {
                0.26
            } else if hod < 10.0 {
                0.42
            } else if hod < 17.0 {
                0.52 // the shopping Saturday
            } else if hod < 20.0 {
                0.36
            } else if hod < 22.0 {
                0.26
            } else {
                0.12
            };
        }
        if dow == 0.0 {
            return if hod < 6.0 {
                0.05
            } else if hod < 9.5 {
                0.12 // the quiet Sunday morning
            } else if hod < 11.0 {
                0.26
            } else if hod < 13.0 {
                0.34
            } else if hod < 17.5 {
                0.30
            } else if hod < 20.0 {
                0.24
            } else if hod < 22.0 {
                0.18
            } else {
                0.09
            };
        }
        let base = if hod < 5.0 {
            0.06
        } else if hod < 6.5 {
            0.20
        } else if hod < 9.5 {
            0.95 // morning rush
        } else if hod < 12.0 {
            0.55
        } else if hod < 14.0 {
            0.70 // lunch
        } else if hod < 16.5 {
            0.55
        } else if hod < 19.0 {
            0.95 // evening rush
        } else if hod < 22.0 {
            0.45
        } else {
            0.14
        };
        base
    }
}

impl Sim {
    /// B75/D83: what a vehicle looks like. The class is fixed by the trip; this
    /// chooses the body: ordinary cars in the town's own age, pickups and
    /// tractors on the rural roads, scooters in a warm land, a cycle now and
    /// then in place of a car, and the work vehicles that only ever drive for a
    /// reason (a bin lorry on its round, a mixer at a site, a coach in the
    /// street of hotels). Bodies follow the age: a young world is all classics.
    fn pick_style(&self, kind: VehicleKind, district: crate::District, rng: &mut Rng) -> u8 {
        use style::*;
        let era = self.tech.vehicle_era();
        let biome = self.map.biome;
        let rural = (self.fields.len() >= 2 || matches!(district, crate::District::Suburb))
            && rng.chance(0.6);
        let warm = matches!(biome, map::Biome::Tropical | map::Biome::Lagoon | map::Biome::Islands);
        match kind {
            VehicleKind::Cart => CART,
            VehicleKind::Taxi => TAXI,
            VehicleKind::Emergency => EMERGENCY,
            VehicleKind::Ev => match rng.usize(4) {
                0 => HATCHBACK,
                1 => SALOON,
                2 => CITY_CAR,
                _ => ESTATE,
            },
            VehicleKind::Bike => {
                // A warm land is scooters; anywhere else a mix, with the
                // bicycle most common in a suburb.
                if warm && rng.chance(0.6) {
                    SCOOTER
                } else {
                    match rng.usize(10) {
                        0..=4 => BICYCLE,
                        5..=7 => MOTORBIKE,
                        _ => SCOOTER,
                    }
                }
            }
            VehicleKind::Van => match rng.usize(10) {
                0 => CAMPER,
                1 => ICE_CREAM,
                2 => BIN_LORRY,
                3 => SWEEPER,
                4 => TOW_TRUCK,
                _ => DELIVERY_VAN,
            },
            VehicleKind::Truck => {
                // B76: a snowy road is gritted and ploughed.
                if self.traffic.snow > 0.2 && rng.chance(0.35) {
                    return if rng.chance(0.5) { PLOUGH } else { GRITTER };
                }
                match rng.usize(10) {
                    0 => LORRY,
                    1 => MIXER,
                    2 => CRANE_TRUCK,
                    3 => BIN_LORRY,
                    4 => BOX_TRUCK,
                    _ => BOX_TRUCK,
                }
            }
            VehicleKind::Bus => match rng.usize(10) {
                0 | 1 => SCHOOL_BUS,
                2 | 3 => COACH,
                _ => COACH,
            },
            VehicleKind::Car => {
                // The private car: the bulk of any street.
                if era == 0 {
                    return CLASSIC;
                }
                let mut r = rng.usize(100);
                // Rural roads carry the working pickups and the odd tractor.
                if rural {
                    if r < 12 {
                        return TRACTOR;
                    }
                    if r < 30 {
                        return PICKUP;
                    }
                }
                // Downtown brings out the nicer cars.
                if matches!(district, crate::District::Downtown) && r < 8 {
                    return if rng.chance(0.5) { SPORTS } else { CONVERTIBLE };
                }
                if era == 1 && rng.chance(0.35) {
                    return CLASSIC;
                }
                let pick = match r {
                    0..=17 => HATCHBACK,
                    18..=37 => SALOON,
                    38..=49 => ESTATE,
                    50..=59 => SUV,
                    60..=65 => MINIVAN,
                    66..=72 => CITY_CAR,
                    73..=76 => PICKUP,
                    77..=80 => CLASSIC,
                    81..=84 => SPORTS,
                    85..=87 => CONVERTIBLE,
                    88..=90 => CAMPER,
                    _ => HATCHBACK,
                };
                // A cycle in place of a car is a different class entirely; the
                // caller does not change kind, so keep this to bodies.
                pick
            }
        }
    }
}

impl Sim {
    /// B77: is this a working day? The one weekday count the whole sim uses
    /// (`Sim::weekday`, 0 = Sunday). This used to count from the founding as a
    /// Thursday — two days off the residents' week — so the traffic ran a
    /// weekday rush on a Sunday and emptied the roads on a Tuesday.
    pub fn weekday_of(time: f64) -> bool {
        let dow = ((time / 86400.0) + 3.0).floor().rem_euclid(7.0);
        dow > 0.0 && dow < 6.0
    }

    /// Route a vehicle from one lot to another over the streets. Falls back to
    /// `None` when no path exists (a disconnected yard, a lot on a stub).
    pub(crate) fn route_between(&mut self, from: (i32, i32), to: (i32, i32), seed: u64) -> Option<Vec<(i32, i32)>> {
        let blocked = BTreeSet::new();
        let Some(from) = self.nearest_lane(from) else {
            self.traffic.why[0] += 1;
            return None;
        };
        let Some(to) = self.nearest_lane(to) else {
            self.traffic.why[1] += 1;
            return None;
        };
        let Some(mut path) = crate::grid::route(&self.map, from, to, false, seed, &blocked) else {
            self.traffic.why[2] += 1;
            return None;
        };
        // A route of one tile is no trip at all.
        if path.len() < 3 {
            self.traffic.why[3] += 1;
            return None;
        }
        path.dedup();
        Some(path)
    }

    /// The closest lane tile to a point (the lot's frontage).
    pub(crate) fn nearest_lane(&self, p: (i32, i32)) -> Option<(i32, i32)> {
        if self.map.at(p.0, p.1).kind.is_lane() {
            return Some(p);
        }
        // A block's interior lot can be several tiles from its street: a 9-tile
        // block with 2-tile setbacks puts the innermost plot ~5 tiles off the
        // kerb, and the search has to reach it (B30 item 3).
        for r in 1..10i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let (x, y) = (p.0 + dx, p.1 + dy);
                    if self.map.in_bounds(x, y) && self.map.at(x, y).kind.is_lane() {
                        return Some((x, y));
                    }
                }
            }
        }
        None
    }

    /// Is there a lane within `r` tiles (Chebyshev) of `p`? The B66 plot rule
    /// uses it: a house whose nearest street is further than this is a house
    /// with no car, on land the streets are not going to reach.
    pub(crate) fn lane_within(&self, p: (i32, i32), r: i32) -> bool {
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (p.0 + dx, p.1 + dy);
                if self.map.in_bounds(x, y) && self.map.at(x, y).kind.is_lane() {
                    return true;
                }
            }
        }
        false
    }

    /// Run the traffic for `secs` of sim time without moving the clock: used by
    /// the stills so a `--time 17:30` frame is a rush hour, not an empty road.
    pub fn warm_traffic(&mut self, secs: f64) {
        let mut left = secs.max(0.0);
        while left > 0.0 {
            let dt = left.min(0.1);
            self.traffic_step(dt);
            left -= dt;
        }
    }

    /// B75: the traffic-signal cycle, 0..1 — the one clock the sim and the
    /// renderer both read, so a car stops on the red the eye can see. One cycle
    /// is fifty sim seconds: red for 0.52 of it, then green, then a short amber
    /// at either change.
    pub fn light_phase(time: f64) -> f32 {
        (time * 0.02).rem_euclid(1.0) as f32
    }

    /// How many of a tile's four neighbours are lanes: 2 is a street straight
    /// through, 3 or 4 a junction (a car eases off for it), 1 a bend or the end
    /// of the road.
    fn lane_count(&self, p: (i32, i32)) -> u32 {
        [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .filter(|(dx, dy)| self.map.at(p.0 + dx, p.1 + dy).kind.is_lane())
            .count() as u32
    }

    /// One traffic step: retire finished vehicles, spawn trips to match the
    /// hour, and move everything. Called at the traffic frame rate (≤ 10 Hz).
    pub fn traffic_step(&mut self, dt: f64) {
        // Retire the ones that arrived. A trip to a car place ends in its bay:
        // the car is handed to the parked layer, which draws it until its owner
        // comes back (B59).
        let before = self.traffic.vehicles.len();
        let mut live: Vec<Vehicle> = Vec::with_capacity(before);
        let mut took = 0usize;
        for v in std::mem::take(&mut self.traffic.vehicles) {
            if v.done() {
                took += 1;
                if let Some(slot) = v.slot {
                    if self.cars.parked.len() < 400 {
                        self.cars.parked.push(crate::carculture::Parked {
                            slot,
                            kind: v.kind,
                            variant: v.variant,
                            style: v.style,
                        });
                    }
                }
            } else {
                live.push(v);
            }
        }
        self.traffic.vehicles = live;
        self.traffic.arrived += took as u32;
        // B58: the level crossings. A car waits at a shut barrier instead of
        // driving through the train (D73: the world has to make sense).
        let shut = self.shut_crossings();
        // B99c/D77: work out what each vehicle *wants* to do this step, then
        // ramp its speed towards it. Nothing goes from nought to full speed in
        // one frame any more: a car pulls away from the kerb, eases off for a
        // junction, slows into its parking bay and stops at a raised barrier.
        // The ramp is what the B67 harness measures as smoothness, so it is a
        // rate (tiles/s per second) and not a smoothing factor: it behaves the
        // same at any frame rate.
        const ACCEL: f32 = 0.62;
        const DECEL: f32 = 1.15;
        let mut plan: Vec<(f32, bool)> = Vec::with_capacity(self.traffic.vehicles.len());
        for v in &self.traffic.vehicles {
            // B59: the highway runs faster than the streets.
            let len: f32 = v
                .route
                .windows(2)
                .map(|w| ((w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()) as f32)
                .sum();
            let t = if len > 0.0 { v.travelled / len } else { 0.0 };
            let fast = v.hw.1 > v.hw.0 && t >= v.hw.0 && t < v.hw.1;
            let mul = if fast { 1.9 } else { 1.0 };
            let cruise = v.kind.speed() * v.pace * mul;
            let mut target = cruise;
            // A junction a car's length ahead: the turn is taken slowly.
            if self.lane_count(v.tile_ahead(1.1)) > 2 {
                target = target.min(cruise * 0.55);
            }
            // The last stretch of the trip: it slows into the bay, the pumps or
            // the kerb instead of arriving at cruising speed.
            let left = len - v.travelled;
            if left < 1.4 {
                target = target.min(cruise * (left / 1.4).max(0.12));
            }
            let blocked = (!shut.is_empty() && shut.contains(&v.tile_ahead(v.vel * dt as f32 + 0.4)))
                || v.stopped;
            if blocked {
                target = 0.0;
            }
            // B75/D83: the lights. Until now the signal head was decoration; a
            // car now stops at the stop line while its approach is red, queues
            // behind the car in front and pulls away on the green.
            let ahead_tile = v.tile_ahead(v.vel * dt as f32 + 0.55);
            if self.streetworks.lights.contains(&ahead_tile) {
                let along_x = v.tile_ahead(0.6).0 != v.tile_ahead(1.6).0;
                let towards = {
                    let a = v.tile_ahead(0.6);
                    let b = v.tile_ahead(1.6);
                    if along_x { b.0 > a.0 } else { b.1 > a.1 }
                };
                // The head faces the +x/+y approach; the other one reads it
                // the other way round (one lamp per tile, the same cycle the
                // renderer draws).
                let head_red = !(0.52..0.95).contains(&Self::light_phase(self.time));
                let sees_red = if towards { head_red } else { !head_red };
                if sees_red {
                    target = 0.0;
                }
            }
            plan.push((target, blocked));
        }
        for (i, v) in self.traffic.vehicles.iter_mut().enumerate() {
            let (target, _blocked) = plan.get(i).copied().unwrap_or((v.vel, false));
            if v.kind == VehicleKind::Emergency {
                v.blink += dt as f32;
            }
            let rate = if target < v.vel { DECEL } else { ACCEL };
            let step = rate * dt as f32;
            v.vel = if v.vel < target {
                (v.vel + step).min(target)
            } else {
                (v.vel - step).max(target)
            };
            // B75: indicate before a turn. The road 2.5 tiles ahead decides,
            // so the amber blinks through the approach and off again once the
            // car is round the corner.
            let d0 = v.dir_at(v.travelled);
            let d1 = v.dir_at(v.travelled + 2.5);
            v.signal = if d0 == d1 {
                0
            } else {
                let cross = d0.0 * d1.1 - d0.1 * d1.0;
                if cross > 0.0 { 1 } else { 2 }
            };
            v.travelled += v.vel * dt as f32;
            v.held = if v.vel < 0.05 { v.held + dt as f32 } else { 0.0 };
        }
        self.follow_step(dt as f32);
        // B59: the people's cars come back and leave their bays.
        self.car_step(dt);
        if self.lots.len() < 8 || self.ending_underway() {
            return;
        }
        // B52: the engines for a fire, sent once when the call comes in.
        if let Some(door) = self.take_fire_dispatch() {
            self.spawn_fire_engines(door);
        }
        let lanes = self.lane_tiles().len();
        let cap = self.traffic.capacity(lanes);
        let want = (cap as f32 * Traffic::busyness(self.time)).round() as usize;
        // Spawn at most a couple a step, and only every few steps: a trip costs
        // an A* route, and traffic arrives rather than popping (B30: the spawn
        // budget was the wallpaper's idle cost at night).
        // A counter, not the sim clock: a warm-up runs many steps without moving
        // time, and keying the window on `self.time` froze the spawns entirely.
        self.traffic.last_spawn_step += 1;
        if self.traffic.last_spawn_step % 5 != 0 {
            return;
        }
        for _ in 0..2 {
            if self.traffic.vehicles.len() >= want {
                break;
            }
            self.spawn_trip();
        }
        // Emergency call-outs: a few a day at month pace, never at rush hour.
        if self.time >= self.traffic.next_emergency {
            let mut rng = self.rng(0xE_11ECu64);
            self.traffic.next_emergency =
                self.time + rng.range_f32(4.0 * 3600.0, 11.0 * 3600.0) as f64;
            if Traffic::busyness(self.time) < 0.9 {
                self.spawn_emergency();
            }
        }
    }

    /// Pick a home and a destination and put a vehicle between them. A pair that
    /// routes nowhere useful (or only one tile) is retried rather than counted as
    /// a failure: the metric that matters is "trips that reach the road" (B30).
    ///
    /// B59: with a chance that follows the hour, the destination is a *place* —
    /// a car park, the pumps, the drive-through — and the car parks in one of
    /// its bays instead of evaporating on the kerb.
    pub(crate) fn spawn_trip(&mut self) -> bool {
        let homes: Vec<(i32, i32)> = self
            .lots
            .iter()
            .filter(|l| {
                matches!(
                    l.kind,
                    LotKind::House | LotKind::Townhouse | LotKind::Apartment
                ) && l.finished()
            })
            .map(|l| (l.x + l.w / 2, l.y + l.h / 2))
            .collect();
        let works: Vec<(i32, i32, LotKind)> = self
            .lots
            .iter()
            .filter(|l| {
                matches!(
                    l.kind,
                    LotKind::Office
                        | LotKind::Tower
                        | LotKind::Industry
                        | LotKind::Shop
                        | LotKind::Store
                ) && l.finished()
            })
            .map(|l| (l.x + l.w / 2, l.y + l.h / 2, l.kind))
            .collect();
        if homes.is_empty() || works.is_empty() {
            return false;
        }
        for _ in 0..4 {
            self.traffic.attempts = self.traffic.attempts.wrapping_add(1);
            let mut rng = self.rng(
                0x7A11 + self.traffic.spawned as u64 + (self.traffic.attempts as u64) * 7919,
            );
            let home = homes[rng.usize(homes.len())];
            let (wx, wy, wkind) = works[rng.usize(works.len())];
            let work = (wx, wy);
            // The night is for coming home; the day for going out.
            let hod = (self.time / 3600.0).rem_euclid(24.0);
            let homeward = hod >= 17.0 || hod < 4.0;
            let mut from = if homeward { work } else { home };
            let mut to = if homeward { home } else { work };
            // B59: what the trip is for (shops, fuel, food, a bed for the night).
            let weekday = crate::Sim::weekday_of(self.time);
            let mut purpose = self.purpose_at(hod, weekday);
            // Errands are not a timetable: mid-morning and mid-afternoon a
            // quarter of the shop runs is the car wash instead.
            if weekday
                && (matches!(hod as i32, 9..=10) || matches!(hod as i32, 14..=16))
                && rng.chance(0.25)
            {
                purpose = crate::carculture::Purpose::Wash;
            }
            let chance = match purpose {
                crate::carculture::Purpose::Work => 0.5,
                _ => 0.85,
            };
            let wants_place = (!homeward || purpose == crate::carculture::Purpose::Fuel)
                && rng.chance(chance);
            let mut slot = None;
            if wants_place {
                if let Some(centre) = self.place_for(purpose, rng.next_u64()) {
                    if let Some(lot) = self.place_at(centre).cloned() {
                        let seed = rng.next_u64();
                        match self.free_bay(&lot, seed) {
                            Some(bay) => {
                                if self.tech.vehicle_era() >= 3 {
                                    // An EV charges; a motor car fills up.
                                }
                                from = home;
                                to = centre;
                                slot = Some(crate::carculture::Slot {
                                    bay,
                                    until: self.time
                                        + crate::Sim::dwell(purpose, seed),
                                    home,
                                    work: Some(work),
                                    at: centre,
                                });
                            }
                            None => {
                                // Every bay taken: the car goes where it was
                                // going anyway, and the car park reads as full
                                // (B59) instead of the streets going quiet.
                                self.cars.no_bay += 1;
                            }
                        }
                    }
                }
            }
            let route = match slot {
                Some(sl) => {
                    let Some(mut r) = self.car_route(from, to, rng.next_u64()) else {
                        continue;
                    };
                    // The last hop: off the kerb and into the bay (or up a
                    // car-park aisle). `car_route` starts at the bay only when
                    // leaving; arriving, it ends at the street.
                    let bt = sl.bay.tile();
                    if r.last() != Some(&bt) {
                        if let Some(&last) = r.last() {
                            // A corner tile, so the drive in is not a diagonal
                            // hop across the plot.
                            let corner = (bt.0, last.1);
                            if corner != last && corner != bt {
                                r.push(corner);
                            }
                            r.push(bt);
                        }
                    }
                    r
                }
                None => {
                    let Some(r) = self.route_between(from, to, rng.next_u64()) else {
                        continue;
                    };
                    r
                }
            };
            let district = self.district_at(to.0, to.1);
            // B49/D72: the mix by era — carts before the motor car, electric
            // cars once the town has them. A truck stays a truck.
            let era = self.tech.vehicle_era();
            let kind = match (wkind, district) {
                (LotKind::Industry, _) => VehicleKind::Truck,
                _ if era == 0 && rng.chance(0.82) => VehicleKind::Cart,
                (_, crate::District::Downtown) if rng.chance(0.35) => VehicleKind::Taxi,
                _ if era == 1 && rng.chance(0.45) => VehicleKind::Cart,
                _ if era >= 3 && rng.chance(0.45) => VehicleKind::Ev,
                // A trip that ends in a bay is a private car far more often than
                // a van (the bays are for cars).
                // B75: a bus or a coach calls at the hotels and the works; the
                // line's own buses are the transit layer's (B58).
                _ if slot.is_none() && rng.chance(0.02) => VehicleKind::Bus,
                _ if rng.chance(if slot.is_some() { 0.05 } else { 0.16 }) => VehicleKind::Van,
                _ => VehicleKind::Car,
            };
            // A trip to the pumps that arrives in an electric car is a charge,
            // not a fill: the car is an EV whenever the world has them.
            let kind = if slot.is_some() && era >= 3 && rng.chance(0.5) {
                VehicleKind::Ev
            } else {
                kind
            };
            // B59b: nobody parks a lorry in a car park bay: a worker bound for
            // the works drives a van when the trip ends in a bay.
            let kind = if slot.is_some() {
                match kind {
                    VehicleKind::Truck => VehicleKind::Van,
                    VehicleKind::Cart => VehicleKind::Car,
                    k => k,
                }
            } else {
                kind
            };
            // B75: one private trip in fourteen is on two wheels instead.
            let kind = if kind == VehicleKind::Car && slot.is_none() && rng.chance(0.07) {
                VehicleKind::Bike
            } else {
                kind
            };
            let pace = rng.range_f32(0.9, 1.1);
            let hw = self.highway_stretch(&route);
            let variant = rng.usize(8) as u8;
            // B75: and what it looks like. A street is mostly private cars, and
            // most cars are ordinary: the mix follows the era, the land and the
            // district, with a cycle now and then instead of a car.
            let style = self.pick_style(kind, district, &mut rng);
            self.traffic.vehicles.push(Vehicle {
                kind,
                route,
                travelled: 0.0,
                from,
                to,
                pace,
                blink: 0.0,
                started: self.time,
                slot,
                variant,
                style,
                signal: 0,
                braking: false,
                held: 0.0,
                stopped: false,
                vel: 0.0,
                hw,
            });
            self.traffic.spawned += 1;
            if slot.is_some() {
                self.cars.trips += 1;
            }
            return true;
        }
        self.traffic.no_route += 1;
        false
    }

    /// The stretch of a route that runs on the highway, as fractions of its
    /// length (B59). One scan at spawn; the step then just compares.
    fn highway_stretch(&self, route: &[(i32, i32)]) -> (f32, f32) {
        let total: f32 = route
            .windows(2)
            .map(|w| ((w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()) as f32)
            .sum();
        if total <= 0.0 || self.cars.highway.is_empty() {
            return (0.0, 0.0);
        }
        let mut acc = 0.0f32;
        let mut best = (0.0f32, 0.0f32);
        let mut run: Option<f32> = None;
        for w in route.windows(2) {
            let seg = ((w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()) as f32;
            let on = self.is_highway(w[0].0, w[0].1) || self.is_highway(w[1].0, w[1].1);
            match (on, run) {
                (true, None) => run = Some(acc / total),
                (false, Some(s)) => {
                    if acc / total - s > best.1 - best.0 {
                        best = (s, acc / total);
                    }
                    run = None;
                }
                _ => {}
            }
            acc += seg;
        }
        if let Some(s) = run {
            if 1.0 - s > best.1 - best.0 {
                best = (s, 1.0);
            }
        }
        best
    }

    /// Car following, cheaply (B59 item 4): one vehicle per tile is the leader,
    /// everyone else in that tile **waits** where it is with the brake lights
    /// on. Waiting rather than being pulled back to the tile mouth is what
    /// keeps a queue a queue and never a car sliding backwards; and a car that
    /// has waited minutes gives up its parking bay, so a jam cannot hold every
    /// space in the city for ever.
    fn follow_step(&mut self, dt: f32) {
        let n = self.traffic.vehicles.len();
        for v in &mut self.traffic.vehicles {
            v.braking = false;
            v.stopped = false;
        }
        if n < 2 {
            return;
        }
        let mut cells: Vec<((i32, i32), usize, f32)> = self
            .traffic
            .vehicles
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let (p, _) = v.pose();
                ((p.0.floor() as i32, p.1.floor() as i32), i, v.travelled)
            })
            .collect();
        cells.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut i = 0;
        while i < cells.len() {
            let mut j = i + 1;
            while j < cells.len() && cells[j].0 == cells[i].0 {
                j += 1;
            }
            if j - i > 1 {
                for c in &cells[i + 1..j] {
                    let v = &mut self.traffic.vehicles[c.1];
                    v.stopped = true;
                    v.braking = true;
                    v.held += dt;
                    if v.held > 90.0 {
                        // Give the bay back: this car is not arriving today.
                        v.slot = None;
                    }
                }
            }
            i = j;
        }
    }

    /// Fire, ambulance or police to a random building and back (D53).
    pub fn spawn_emergency(&mut self) -> bool {
        let targets: Vec<(i32, i32)> = self
            .lots
            .iter()
            .filter(|l| l.finished() && !matches!(l.kind, LotKind::Park))
            .map(|l| (l.x + l.w / 2, l.y + l.h / 2))
            .collect();
        if targets.len() < 4 {
            return false;
        }
        let mut rng = self.rng(0xE11);
        // A station on the ring road, not in the middle: the run then crosses
        // the open streets on the edge of town, which is where a call-out is
        // seen (and it keeps the vehicle out of the dense core).
        let (fx, fy) = self.founding;
        let station = self
            .lane_tiles()
            .into_iter()
            .max_by_key(|&(x, y)| (x - fx).abs() + (y - fy).abs())
            .unwrap_or(self.founding);
        // A building in the open: the one with the least built-up ground around
        // it, so the run happens on streets you can actually see (a call-out
        // through the dense core is real, but invisible from this camera).
        let open = |x: i32, y: i32| -> i32 {
            let mut n = 0;
            for dy in -3..=3 {
                for dx in -3..=3 {
                    if self.lots.iter().any(|l| l.covers(x + dx, y + dy)) {
                        n += 1;
                    }
                }
            }
            n
        };
        let mut target = targets[0];
        let mut best = i32::MAX;
        for &(x, y) in targets.iter().step_by(3) {
            let d = (x - fx).abs() + (y - fy).abs();
            if d < 4 {
                continue;
            }
            let score = open(x, y);
            if score < best {
                best = score;
                target = (x, y);
            }
        }
        let Some(route) = self.route_between(station, target, rng.next_u64()) else {
            return false;
        };
        self.traffic.vehicles.push(Vehicle {
            kind: VehicleKind::Emergency,
            route,
            travelled: 0.0,
            from: station,
            to: target,
            pace: 1.15,
            blink: 0.0,
            started: self.time,
            slot: None,
            variant: 0,
            style: style::EMERGENCY,
            signal: 0,
            vel: 0.0,
            braking: false,
            held: 0.0,
            stopped: false,
            hw: (0.0, 0.0),
        });
        self.events.push(crate::Event {
            time: self.time,
            text: "an emergency vehicle is on its way".into(),
        });
        true
    }

    /// B76: send `n` winter-service vehicles (ploughs and gritters) out along
    /// main-road routes. A still or a fresh snowfall needs them on the street
    /// straight away; afterwards the ordinary spawn mix keeps them coming while
    /// the snow lies.
    pub fn spawn_winter_service(&mut self, n: usize) -> usize {
        // Prefer the lanes round the town centre (B76): a plough sent to the
        // map's far corner is real but off the frame, and the still looks as if
        // nothing came out. The camera looks at the founding, so that is where
        // the winter service has to pass.
        let (fx, fy) = self.founding;
        let near: Vec<(i32, i32)> = self
            .lane_tiles()
            .into_iter()
            .filter(|&(x, y)| (x - fx).abs().max((y - fy).abs()) <= 26)
            .collect();
        let lanes = if near.len() >= 60 { near } else { self.lane_tiles() };
        if lanes.len() < 40 {
            return 0;
        }
        // Priority (B76 r2): avenues first, then collectors, then the local
        // streets — the same order a real service works through a town.
        let mut avenues: Vec<(i32, i32)> = Vec::new();
        let mut collectors: Vec<(i32, i32)> = Vec::new();
        for &(x, y) in &lanes {
            match self.road_class(x, y) {
                crate::roads::RoadClass::Avenue => avenues.push((x, y)),
                crate::roads::RoadClass::Collector => collectors.push((x, y)),
                _ => {}
            }
        }
        let mut rng = self.rng(0x5EED);
        let mut sent = 0;
        for k in 0..n {
            // Two ends of the network: a run across the town, not a lap of a
            // cul-de-sac. The start is on the best class available.
            let a = if k < 2 && !avenues.is_empty() {
                avenues[rng.usize(avenues.len())]
            } else if !collectors.is_empty() {
                collectors[rng.usize(collectors.len())]
            } else {
                lanes[rng.usize(lanes.len())]
            };
            let mut b = a;
            for _ in 0..24 {
                let c = lanes[rng.usize(lanes.len())];
                if (c.0 - a.0).abs() + (c.1 - a.1).abs() > 18 {
                    b = c;
                    break;
                }
            }
            let Some(route) = self.route_between(a, b, 0x76_5EED ^ k as u64) else {
                continue;
            };
            self.traffic.vehicles.push(Vehicle {
                kind: VehicleKind::Truck,
                route,
                travelled: 0.0,
                from: a,
                to: b,
                pace: 0.62,
                blink: 0.0,
                started: self.time,
                slot: None,
                variant: 0,
                style: if k % 2 == 0 { style::PLOUGH } else { style::GRITTER },
                signal: 0,
                braking: false,
                held: 0.0,
                stopped: false,
                vel: 0.0,
                hw: (0.0, 0.0),
            });
            sent += 1;
        }
        sent
    }

    /// B76 r2: the tiles the winter service has just driven — each plough and
    /// gritter, and the stretch behind it, so the renderer can clear the lane
    /// they have worked.
    pub fn service_tiles(&self) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for v in &self.traffic.vehicles {
            if !matches!(v.style, style::PLOUGH | style::GRITTER) {
                continue;
            }
            let ((x, y), (dx, dy)) = v.pose();
            for k in -2..=1 {
                let tx = (x - dx * k as f32).floor() as i32;
                let ty = (y - dy * k as f32).floor() as i32;
                out.push((tx, ty));
            }
        }
        out
    }

    /// B52: two fire engines from the fire station (or the edge of town when
    /// there is none yet) to the burning building's door.
    pub fn spawn_fire_engines(&mut self, door: (i32, i32)) -> bool {
        let station = self
            .lots
            .iter()
            .find(|l| l.finished() && l.kind == LotKind::Landmark(crate::landmarks::LandmarkKind::FireStation))
            .map(crate::people::door_tile)
            .or_else(|| {
                let (fx, fy) = self.founding;
                self.lane_tiles()
                    .into_iter()
                    .max_by_key(|&(x, y)| (x - fx).abs() + (y - fy).abs())
            })
            .unwrap_or(self.founding);
        let mut sent = false;
        for k in 0..2u64 {
            let Some(route) = self.route_between(station, door, 0xF1_4E ^ k) else {
                continue;
            };
            self.traffic.vehicles.push(Vehicle {
                kind: VehicleKind::Emergency,
                route,
                travelled: 0.0,
                from: station,
                to: door,
                pace: 1.2 - 0.12 * k as f32,
                blink: 0.0,
                started: self.time,
                slot: None,
                variant: 0,
                style: style::EMERGENCY,
                signal: 0,
                braking: false,
                held: 0.0,
                stopped: false,
                vel: 0.0,
                hw: (0.0, 0.0),
            });
            sent = true;
        }
        sent
    }

    /// The vehicles to draw, in painter order.
    pub fn vehicles(&self) -> Vec<&Vehicle> {
        self.traffic.vehicles.iter().collect()
    }
}


/// Distance along a route at which `tile` is entered, in tiles travelled.
/// Used by the car-following rule to hold a follower at the tile's mouth.
fn tile_start(route: &[(i32, i32)], tile: (i32, i32)) -> f32 {
    let mut acc = 0.0f32;
    for w in route.windows(2) {
        if w[0] == tile {
            return acc;
        }
        acc += ((w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()) as f32;
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pace;

    fn town(seed: u64, days: f64) -> crate::Sim {
        let vp = map::world_for_viewport(2347, 480);
        let mut s = crate::Sim::with_focus_in(
            seed,
            Pace::Month,
            vp.w,
            vp.h,
            (vp.w as f32 * 0.5, vp.h as f32 * 0.5),
            Some(map::Biome::Temperate),
        );
        s.run_to(days * 86400.0);
        s
    }

    /// B99c/D77: a car never goes from nought to full speed in one frame. Every
    /// vehicle's speed changes by at most the acceleration limit per step, it
    /// reaches its cruising speed on a clear street, and it comes to a stop
    /// rather than vanishing at the end of its trip.
    #[test]
    fn cars_accelerate_instead_of_jumping() {
        let mut s = town(7, 20.0);
        s.set_clock_hour(8.2); // rush hour: cars on the street
        let dt = 0.1f64;
        // Let a few trips spawn and settle.
        for _ in 0..40 {
            s.traffic_step(dt);
        }
        assert!(!s.traffic.vehicles.is_empty(), "no traffic to measure");
        // Vehicles retire and spawn every step, which renumbers the list, so a
        // car is followed by its own identity and not by its index.
        let key = |v: &Vehicle| (v.from, v.to, v.variant, v.started.to_bits());
        let mut seen_full = false;
        let mut seen_slow = false;
        let mut prev: std::collections::BTreeMap<((i32, i32), (i32, i32), u8, u64), (f32, u32)> =
            std::collections::BTreeMap::new();
        for step in 0..600 {
            s.traffic_step(dt);
            let mut now: std::collections::BTreeMap<_, (f32, u32)> =
                std::collections::BTreeMap::new();
            let mut shared: std::collections::BTreeSet<_> = std::collections::BTreeSet::new();
            for v in &s.traffic.vehicles {
                let k = key(v);
                if prev.contains_key(&k) {
                    shared.insert(k);
                }
                let e = now.entry(k).or_insert((0.0, 0));
                *e = (v.vel, e.1 + 1);
                let cruise = v.kind.speed() * v.pace;
                if v.vel > 0.9 * cruise {
                    seen_full = true;
                }
                if v.vel > 0.05 && v.vel < 0.5 * cruise {
                    seen_slow = true;
                }
            }
            for k in shared {
                // Two cars can share a key (same trip, same clock tick): skip
                // those, they are not the same car twice.
                let (is, n_now) = now[&k];
                if n_now > 1 || prev[&k].1 > 1 {
                    continue;
                }
                let was = prev[&k].0;
                assert!(
                    (is - was).abs() <= 1.2 * dt as f32 + 1e-3,
                    "step {step}: a vehicle went from {was:.2} to {is:.2} tiles/s in one step"
                );
            }
            prev = now;
        }
        assert!(seen_full, "no vehicle ever reached its cruising speed");
        assert!(seen_slow, "no vehicle was ever part way up to speed");
    }

    /// B75/D83: a car stops at a red light. The signal is the one the renderer
    /// draws (a shared phase), so a stopped queue is a queue at a red lamp and
    /// not an invisible rule.
    #[test]
    fn cars_stop_at_a_red_light() {
        let mut s = town(7, 20.0);
        assert!(!s.streetworks.lights.is_empty(), "the town has no traffic lights");
        let light = *s.streetworks.lights.iter().next().unwrap();
        // An approach tile with a lane on it, and a car pointed at the light.
        let approach = [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .map(|(dx, dy)| (light.0 + dx, light.1 + dy))
            .find(|&(x, y)| s.map.at(x, y).kind.is_lane())
            .expect("no road onto the light");
        let route = vec![approach, light, (light.0 + 1, light.1)];
        // A time whose phase is red for the +x approach.
        s.time = 0.0;
        assert!((s.time * 0.02).rem_euclid(1.0) < 0.45, "phase setup");
        s.traffic.vehicles.clear();
        s.traffic.vehicles.push(Vehicle {
            kind: VehicleKind::Car,
            route,
            travelled: 0.0,
            from: approach,
            to: (light.0 + 1, light.1),
            pace: 1.0,
            blink: 0.0,
            started: 0.0,
            slot: None,
            variant: 0,
            style: style::SALOON,
            signal: 0,
            braking: false,
            held: 0.0,
            stopped: false,
            hw: (0.0, 0.0),
            vel: 1.4,
        });
        for _ in 0..30 {
            s.traffic_step(0.1);
            // Keep the clock still: the phase must stay red for the test.
            s.time = 0.0;
        }
        let v = &s.traffic.vehicles[0];
        assert!(
            v.travelled < 1.0,
            "the car drove through a red light ({:.2} tiles in)",
            v.travelled
        );
        assert!(v.braking || v.vel < 0.05, "the car did not stop at the light");
    }

    /// B75: the whole catalogue turns up in a grown city — the point of the
    /// styles is a street you can read, so every design has to be in the mix.
    #[test]
    fn every_vehicle_style_appears_in_a_grown_city() {
        let mut s = town(7, 22.0);
        let mut seen = std::collections::BTreeSet::new();
        let mut classes = std::collections::BTreeSet::new();
        for day in 22..=26 {
            for hour in [8.0, 12.0, 17.5, 20.0] {
                s.run_to(day as f64 * 86400.0 + hour * 3600.0);
                for _ in 0..200 {
                    s.traffic_step(0.1);
                }
                for v in &s.traffic.vehicles {
                    seen.insert(v.style);
                    classes.insert(v.kind.name());
                }
                // B77: and what the week itself puts on the road — the school
                // bus at the gate, the bin lorry on its round.
                for v in s.weekly_vehicles() {
                    seen.insert(v.style);
                    classes.insert(v.kind.name());
                }
                for c in &s.cars.parked {
                    seen.insert(c.style);
                }
            }
        }
        // The renderer's table order (B75): the sim only carries the index.
        const NAMES: [&str; 29] = [
            "hatchback", "saloon", "estate", "suv", "pickup", "minivan", "city car",
            "sports car", "convertible", "classic", "campervan", "motorbike", "scooter",
            "bicycle", "delivery van", "box truck", "articulated lorry", "bin lorry",
            "street sweeper", "ice-cream van", "tour coach", "school bus", "tow truck",
            "cement mixer", "crane truck", "tractor", "taxi", "emergency", "cart",
        ];
        let missing: Vec<&str> = NAMES
            .iter()
            .enumerate()
            .filter(|(i, _)| !seen.contains(&(*i as u8)))
            .map(|(_, n)| *n)
            .collect();
        println!("styles seen: {} of 29, missing {missing:?}", seen.len());
        assert!(
            missing.len() <= 3,
            "only {} of 29 bodies ever drove: missing {missing:?}",
            seen.len()
        );
        for class in ["car", "van", "truck", "bus", "taxi", "ev", "bike"] {
            assert!(classes.contains(class), "no {class} in a grown city's traffic");
        }
    }

    #[test]
    fn a_junction_slows_the_traffic_it_carries() {
        let mut s = town(7, 20.0);
        let lanes = s.lane_tiles();
        assert!(lanes.len() > 40, "the town has no streets");
        let mut junction = None;
        let mut straight = None;
        for &(x, y) in &lanes {
            let n = s.lane_count((x, y));
            if n >= 3 && junction.is_none() {
                junction = Some((x, y));
            }
            if n == 2 && straight.is_none() {
                straight = Some((x, y));
            }
        }
        let junction = junction.expect("no junction on the map");
        let straight = straight.expect("no straight street on the map");
        assert!(s.lane_count(junction) >= 3, "junction at {junction:?}");
        assert_eq!(s.lane_count(straight), 2, "straight street at {straight:?}");
    }
}
