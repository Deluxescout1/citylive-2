//! Car culture (B59 / D72): the places cars need, the bays they park in, and
//! purposeful trips between homes, work and those places.
//!
//! The places are ordinary [`Lot`]s of their own kinds, so they get the town's
//! frontage, grading and construction stages for free; what makes them car
//! places is the **bays** they offer. `bays_of` is the only definition of that
//! geometry in the sim, and it is derived from the lot rectangle and the side
//! its street is on, so a save carries nothing extra.
//!
//! Cars parked in a bay *are* extra state, but like `traffic` they are not
//! saved: they are re-derived by replaying the clock (the same warm-up the
//! wallpaper uses), which keeps the save file about the city and not its cars.
//!
//! Cheap by construction: placement is one decision at a time, a car's dwell is
//! a single comparison, and everything a frame draws comes from short vectors.

use crate::traffic::VehicleKind;
use crate::{Era, Lot, LotKind, Sim, HOUSE_SECONDS};

/// `Tile::aux` bit marking a highway lane (B59), defined in the map crate so
/// the renderer can paint it without depending on the sim.
pub use map::AUX_HIGHWAY;

/// Residents a city needs before a highway is cut past it (B59b).
pub const HIGHWAY_RESIDENTS: u32 = 1500;

/// How much frontage one bay takes, in tiles. A tile is about twelve metres
/// and a bay is about four, so three cars park along each tile's edge (B59).
pub const BAY_PITCH: f32 = map::carplaces::BAY_PITCH;

/// The kinds of place that exist because cars do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaceKind {
    /// Surface car park with marked bays.
    CarPark,
    /// Multi-storey car park downtown.
    CarParkDeck,
    GasStation,
    /// EV charger hub: B49's `EvChargers` flag, later eras.
    ChargingHub,
    DriveThrough,
    CarWash,
    Dealership,
    Motel,
}

impl PlaceKind {
    pub const ALL: [PlaceKind; 8] = [
        PlaceKind::CarPark,
        PlaceKind::CarParkDeck,
        PlaceKind::GasStation,
        PlaceKind::ChargingHub,
        PlaceKind::DriveThrough,
        PlaceKind::CarWash,
        PlaceKind::Dealership,
        PlaceKind::Motel,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PlaceKind::CarPark => "car park",
            PlaceKind::CarParkDeck => "multi-storey car park",
            PlaceKind::GasStation => "filling station",
            PlaceKind::ChargingHub => "charging hub",
            PlaceKind::DriveThrough => "drive-through",
            PlaceKind::CarWash => "car wash",
            PlaceKind::Dealership => "dealership",
            PlaceKind::Motel => "motel",
        }
    }

    pub fn lot_kind(self) -> LotKind {
        match self {
            PlaceKind::CarPark => LotKind::CarPark,
            PlaceKind::CarParkDeck => LotKind::CarParkDeck,
            PlaceKind::GasStation => LotKind::GasStation,
            PlaceKind::ChargingHub => LotKind::ChargingHub,
            PlaceKind::DriveThrough => LotKind::DriveThrough,
            PlaceKind::CarWash => LotKind::CarWash,
            PlaceKind::Dealership => LotKind::Dealership,
            PlaceKind::Motel => LotKind::Motel,
        }
    }

    pub fn of_lot(k: LotKind) -> Option<PlaceKind> {
        Some(match k {
            LotKind::CarPark => PlaceKind::CarPark,
            LotKind::CarParkDeck => PlaceKind::CarParkDeck,
            LotKind::GasStation => PlaceKind::GasStation,
            LotKind::ChargingHub => PlaceKind::ChargingHub,
            LotKind::DriveThrough => PlaceKind::DriveThrough,
            LotKind::CarWash => PlaceKind::CarWash,
            LotKind::Dealership => PlaceKind::Dealership,
            LotKind::Motel => PlaceKind::Motel,
            _ => return None,
        })
    }

    /// Plot size a new one is staked on.
    pub fn size(self) -> (i32, i32) {
        match self {
            // Small plots: a car place must never take the land a street of
            // houses would have stood on (the town's era follows its housing).
            // The size the trade needs to read as itself: a forecourt with a
            // canopy and a queue lane, not a widened cottage plot. The trade
            // buys up its neighbours to reach it (see `convert_to_car_place`).
            PlaceKind::CarPark => (4, 4),
            PlaceKind::CarParkDeck => (4, 4),
            PlaceKind::GasStation => (3, 3),
            PlaceKind::ChargingHub => (3, 3),
            PlaceKind::DriveThrough => (3, 3),
            PlaceKind::CarWash => (3, 2),
            PlaceKind::Dealership => (4, 3),
            PlaceKind::Motel => (5, 2),
        }
    }

    /// The earliest era one may appear in.
    pub fn min_era(self) -> Era {
        match self {
            PlaceKind::Motel | PlaceKind::Dealership => Era::City,
            _ => Era::Town,
        }
    }

    /// How many the city wants at its widest (a cap, not a target: most seeds
    /// never reach it).
    pub fn max_count(self) -> usize {
        match self {
            PlaceKind::CarPark => 3,
            PlaceKind::CarParkDeck => 1,
            PlaceKind::GasStation => 3,
            PlaceKind::ChargingHub => 1,
            PlaceKind::DriveThrough => 2,
            PlaceKind::CarWash => 1,
            PlaceKind::Dealership => 1,
            PlaceKind::Motel => 1,
        }
    }

    /// What kind of car culture this place is: gas stations and their later
    /// charging hubs are the same site with a different flag.
    /// The plot shape the shared layout (`map::carplaces`) knows this by.
    pub fn shape(self) -> map::carplaces::Shape {
        use map::carplaces::Shape;
        match self {
            PlaceKind::CarPark => Shape::Park,
            PlaceKind::CarParkDeck => Shape::Deck,
            PlaceKind::GasStation => Shape::Gas,
            PlaceKind::ChargingHub => Shape::Charge,
            PlaceKind::DriveThrough => Shape::Drive,
            PlaceKind::CarWash => Shape::Wash,
            PlaceKind::Dealership => Shape::Dealer,
            PlaceKind::Motel => Shape::Motel,
        }
    }

    pub fn is_fuel(self) -> bool {
        matches!(self, PlaceKind::GasStation | PlaceKind::ChargingHub)
    }
}

/// One marked bay: the tile-space point a car parks on, which way it faces, and
/// how high the deck is (pixels, 0 for a flat surface).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bay {
    pub x: f32,
    pub y: f32,
    /// Facing of a parked car: axis aligned.
    pub dir: (f32, f32),
    /// Deck height in screen pixels (multi-storey car park).
    pub lift: f32,
}

impl Bay {
    pub fn tile(&self) -> (i32, i32) {
        (self.x.floor() as i32, self.y.floor() as i32)
    }
}

/// The bays a lot offers, derived from its rectangle and the side its street is
/// on. Empty for every kind that is not a car place. The geometry itself is
/// `map::carplaces` (shared with the renderer, which paints the lines round
/// exactly these bays).
pub fn bays_of(l: &Lot) -> Vec<Bay> {
    let Some(kind) = PlaceKind::of_lot(l.kind) else {
        return Vec::new();
    };
    let f = map::carplaces::Frame::new(l.x, l.y, l.w, l.h, l.front);
    let (al, dl) = f.size();
    map::carplaces::bays_for(kind.shape(), al, dl, l.front >= 2)
        .into_iter()
        .map(|b| {
            let (x, y) = f.at(b.t, b.d);
            Bay {
                x: x.clamp(l.x as f32 + 0.1, (l.x + l.w) as f32 - 0.1),
                y: y.clamp(l.y as f32 + 0.1, (l.y + l.h) as f32 - 0.1),
                dir: f.dir(b.nose),
                lift: if b.level > 0 || kind == PlaceKind::CarParkDeck {
                    b.level as f32 * map::carplaces::DECK_PX + 2.0
                } else {
                    0.0
                },
            }
        })
        .collect()
}

/// Bays a lot offers. Deck bays count: the upper floors really do hold cars.
pub fn parking_capacity(l: &Lot) -> usize {
    if !l.finished() {
        return 0;
    }
    bays_of(l).len()
}

/// Where a car will stand, and for how long: carried by the vehicle on its way
/// there, then handed to [`Cars::parked`] when it arrives.
#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub bay: Bay,
    /// Sim time the owner leaves.
    pub until: f64,
    /// Where it goes when it leaves.
    pub home: (i32, i32),
    pub work: Option<(i32, i32)>,
    /// The place it is parked at (lot centre), for occupancy counts.
    pub at: (i32, i32),
}

/// A car sitting in a bay until its owner comes back.
#[derive(Clone, Copy, Debug)]
pub struct Parked {
    pub slot: Slot,
    pub kind: VehicleKind,
    pub variant: u8,
    /// B75: the body it drove in with (it keeps it while it is parked).
    pub style: u8,
}

/// The car layer: what is parked, the highway, and the counters a test reads.
#[derive(Clone, Default, Debug)]
pub struct Cars {
    /// Cars in bays right now. Rebuilt from the clock on load, never saved.
    pub parked: Vec<Parked>,
    /// Highway lane tiles (also flagged in the map with [`AUX_HIGHWAY`]).
    pub highway: Vec<(i32, i32)>,
    /// The interchange legs, for the renderer and the tests.
    pub ramps: Vec<(i32, i32)>,
    /// Set once the highway has been cut.
    pub planned_highway: bool,
    /// Purposeful trips sent to a car place.
    pub trips: u32,
    /// Trips that found every bay of their place taken.
    pub no_bay: u32,
    /// Cars that ended their stay and drove off.
    pub departures: u32,
    /// Trips to a place that could not be routed.
    pub no_route: u32,
    /// Sim time the next place may be staked. A car place must not eat every
    /// chance the town gets to build something else (a landmark needs its turn).
    pub next_at: f64,
}

/// One car to draw, whether it is moving, parked or queued. The renderer only
/// needs this; it never sees the sim.
/// B75: a parked car's body, from its colour pick. Bays are full of ordinary
/// cars; the body follows the variant so a row of them is not one model.
pub(crate) fn parked_style(variant: u8) -> u8 {
    use crate::traffic::style;
    const BODIES: [u8; 6] = [
        style::SALOON,
        style::HATCHBACK,
        style::ESTATE,
        style::SUV,
        style::CITY_CAR,
        style::MINIVAN,
    ];
    BODIES[variant as usize % BODIES.len()]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarView {
    pub x: f32,
    pub y: f32,
    pub dir: (f32, f32),
    pub kind: VehicleKind,
    pub variant: u8,
    /// B75: the body style (the renderer's table).
    pub style: u8,
    /// Deck height in pixels (multi-storey car park).
    pub lift: f32,
    /// Brake lights: a car queuing at a window or held in a jam.
    pub braking: bool,
}

/// Why a car is out: the destination class the hour asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    Work,
    Shop,
    Fuel,
    Food,
    Wash,
    Motel,
}

impl Sim {
    /// Every car place in the city, by lot.
    pub fn car_places(&self) -> Vec<&Lot> {
        self.lots
            .iter()
            .filter(|l| PlaceKind::of_lot(l.kind).is_some())
            .collect()
    }

    pub fn place_count(&self, k: PlaceKind) -> usize {
        self.lots.iter().filter(|l| l.kind == k.lot_kind()).count()
    }

    /// Total bays of the finished places — the city's parking capacity.
    pub fn parking_capacity(&self) -> usize {
        self.car_places().iter().map(|l| parking_capacity(l)).sum()
    }

    /// Is this bay of this place taken? `parked` is short (a city has a few
    /// hundred cars), so a linear scan is the cheapest honest answer.
    fn bay_taken(&self, b: &Bay) -> bool {
        let same = |q: &Bay| (q.x - b.x).abs() < 0.05 && (q.y - b.y).abs() < 0.05 && q.lift == b.lift;
        self.cars.parked.iter().any(|p| same(&p.slot.bay))
            // A bay is taken from the moment a car sets out for it, or two cars
            // would drive to the same spot (B59: capacity, not parking spaces
            // that appear twice).
            || self
                .traffic
                .vehicles
                .iter()
                .any(|v| v.slot.as_ref().is_some_and(|sl| same(&sl.bay)))
    }

    /// Choose a free bay at a place, starting from a seeded offset so the same
    /// seed fills a lot from the same end.
    pub fn free_bay(&self, place: &Lot, seed: u64) -> Option<Bay> {
        let bays = bays_of(place);
        if bays.is_empty() {
            return None;
        }
        let start = (seed % bays.len() as u64) as usize;
        for k in 0..bays.len() {
            let b = bays[(start + k) % bays.len()];
            if !self.bay_taken(&b) {
                return Some(b);
            }
        }
        None
    }

    /// How long a car stays, by purpose, in seconds.
    pub fn dwell(purpose: Purpose, seed: u64) -> f64 {
        let jitter = (seed % 17) as f64 / 17.0; // 0..1
        match purpose {
            Purpose::Work => 7.0 * 3600.0 + jitter * 2.0 * 3600.0,
            Purpose::Shop => 25.0 * 60.0 + jitter * 50.0 * 60.0,
            Purpose::Fuel => 5.0 * 60.0 + jitter * 4.0 * 60.0,
            Purpose::Food => 4.0 * 60.0 + jitter * 3.0 * 60.0,
            Purpose::Wash => 8.0 * 60.0 + jitter * 6.0 * 60.0,
            Purpose::Motel => 8.0 * 3600.0 + jitter * 4.0 * 3600.0,
        }
    }

    /// The purpose a trip out has at this hour: the people schedule, read for
    /// cars. Weekday mornings go to work, lunch to the drive-through, the
    /// evening back home with a stop at the pumps, and the night to a motel.
    pub fn purpose_at(&self, hod: f64, weekday: bool) -> Purpose {
        if !weekday {
            return match hod {
                0.0..=9.5 => Purpose::Shop,
                9.5..=11.5 => Purpose::Wash,
                11.5..=13.5 => Purpose::Food,
                13.5..=17.0 => Purpose::Shop,
                17.0..=20.0 => Purpose::Food,
                20.0..=22.5 => Purpose::Motel,
                _ => Purpose::Fuel,
            };
        }
        match hod {
            0.0..=5.5 => Purpose::Fuel,
            5.5..=9.5 => Purpose::Work,
            9.5..=11.5 => Purpose::Shop,
            11.5..=13.5 => Purpose::Food,
            13.5..=16.5 => Purpose::Shop,
            16.5..=19.5 => Purpose::Fuel,
            19.5..=22.5 => Purpose::Food,
            _ => Purpose::Motel,
        }
    }

    /// The place a purpose wants, newest fit first: a charging hub for an EV
    /// once the town has them, else a filling station.
    pub fn place_for(&self, purpose: Purpose, seed: u64) -> Option<(i32, i32)> {
        let ev = self.tech.vehicle_era() >= 3;
        let pick = |kinds: &[PlaceKind]| -> Option<(i32, i32)> {
            let mut v: Vec<&Lot> = self
                .lots
                .iter()
                .filter(|l| l.finished() && kinds.iter().any(|k| l.kind == k.lot_kind()))
                .filter(|l| l.stage == crate::Stage::Finished)
                .collect();
            if v.is_empty() {
                return None;
            }
            v.sort_by_key(|l| (l.x, l.y));
            let i = (seed % v.len() as u64) as usize;
            Some((v[i].x + v[i].w / 2, v[i].y + v[i].h / 2))
        };
        match purpose {
            Purpose::Work => pick(&[PlaceKind::CarPark, PlaceKind::CarParkDeck]),
            Purpose::Shop => pick(&[PlaceKind::CarPark]),
            Purpose::Fuel if ev => {
                pick(&[PlaceKind::ChargingHub]).or_else(|| pick(&[PlaceKind::GasStation]))
            }
            Purpose::Fuel => pick(&[PlaceKind::GasStation, PlaceKind::ChargingHub]),
            Purpose::Food => pick(&[PlaceKind::DriveThrough]),
            Purpose::Wash => pick(&[PlaceKind::CarWash]),
            Purpose::Motel => pick(&[PlaceKind::Motel]),
        }
    }

    /// The lot at a centre point, if it is a car place (the trip's destination
    /// was stored as a tile).
    pub fn place_at(&self, p: (i32, i32)) -> Option<&Lot> {
        self.lots.iter().find(|l| {
            p.0 >= l.x && p.0 < l.x + l.w && p.1 >= l.y && p.1 < l.y + l.h
                && PlaceKind::of_lot(l.kind).is_some()
        })
    }

    /// One car step: owners come back for their cars and drive off. Called from
    /// `traffic_step`, so a warm-up replays it exactly.
    pub fn car_step(&mut self, dt: f64) {
        let _ = dt;
        if self.cars.parked.is_empty() {
            return;
        }
        // At most one departure a step (10 a second): a car leaving is a route.
        let now = self.time;
        if let Some(i) = self.cars.parked.iter().position(|p| p.slot.until <= now) {
            let p = self.cars.parked.remove(i);
            self.cars.departures += 1;
            // Back to work in the morning, home in the evening.
            let hod = (now / 3600.0).rem_euclid(24.0);
            let to = if hod >= 6.0 && hod < 16.0 {
                p.slot.work.unwrap_or(p.slot.home)
            } else {
                p.slot.home
            };
            let from = p.slot.bay.tile();
            let mut rng = self.rng(0xCA20 + self.cars.departures as u64);
            if let Some(mut route) = self.car_route(from, to, rng.next_u64()) {
                route.insert(0, from);
                self.traffic.vehicles.push(crate::traffic::Vehicle {
                    kind: p.kind,
                    route,
                    travelled: 0.0,
                    from,
                    to,
                    pace: rng.range_f32(0.92, 1.08),
                    blink: 0.0,
                    started: now,
                    slot: None,
                    variant: p.variant,
                    style: p.style,
                    signal: 0,
                    braking: false,
                    held: 0.0,
                    stopped: false,
                    hw: (0.0, 0.0),
                    vel: 0.0,
                });
            }
        }
    }

    /// Route from a bay tile to another lot: the street route, with the bay
    /// prepended so the car drives out of its spot instead of appearing on the
    /// kerb.
    pub(crate) fn car_route(
        &mut self,
        from: (i32, i32),
        to: (i32, i32),
        seed: u64,
    ) -> Option<Vec<(i32, i32)>> {
        let Some(lane) = self.nearest_lane(from) else {
            self.cars.no_route += 1;
            return None;
        };
        let Some(mut route) = self.route_between(lane, to, seed) else {
            self.cars.no_route += 1;
            return None;
        };
        if route[0] != lane {
            route.insert(0, lane);
        }
        Some(route)
    }

    /// Cars parked right now at one place (its centre).
    pub fn occupancy(&self, place_centre: (i32, i32)) -> usize {
        self.cars
            .parked
            .iter()
            .filter(|p| p.slot.at == place_centre)
            .count()
    }

    /// A random stream for what a lot looks like (which cars stand in its bays):
    /// fixed by the world's seed and the tag, **not** by the planning step. The
    /// step-keyed `rng` re-rolled every parked car's body, and whether a motel's
    /// guest or a flat's neighbour was "away", every twenty sim seconds.
    fn lot_rng(&self, tag: u64) -> map::rng::Rng {
        map::rng::Rng::new(self.seed ^ tag.wrapping_mul(0xA24B_AED4_963E_E407) ^ 0x7A4E_D001)
    }

    /// Everything a frame draws that is a car and not a moving vehicle: the
    /// parked ones, the cars in a drive-through queue, the dealership's stock,
    /// the motel's guests at night and the homes' driveway cars.
    pub fn car_views(&self) -> Vec<CarView> {
        let mut out: Vec<CarView> = Vec::with_capacity(self.cars.parked.len() + 24);
        for p in &self.cars.parked {
            // The multi-storey: the roof deck's cars are sprites; the cars on
            // the covered decks are painted in its openings by the renderer
            // (a sprite would stand through the deck above it).
            if p.slot.bay.lift > 0.0 && p.slot.bay.lift < map::carplaces::DECK_PX * 2.0 {
                continue;
            }
            out.push(CarView {
                x: p.slot.bay.x,
                y: p.slot.bay.y,
                dir: p.slot.bay.dir,
                kind: p.kind,
                variant: p.variant,
                style: parked_style(p.variant),
                lift: p.slot.bay.lift,
                braking: false,
            });
        }
        let hod = (self.time / 3600.0).rem_euclid(24.0);
        for l in &self.lots {
            if !l.finished() {
                continue;
            }
            let Some(kind) = PlaceKind::of_lot(l.kind) else {
                if l.kind == LotKind::House {
                    if let Some(c) = self.driveway_car(l, hod) {
                        out.push(c);
                    }
                }
                // B74 note: a block of flats parks its residents in its own
                // spaces — the kerb row or the undercroft — so a full block at
                // night reads as a full car park and an empty one at eight.
                if crate::parking::needs_parking(l) {
                    let bays = self.parking_bays(l);
                    let fill = crate::parking::fill(hod);
                    let taken = (bays.len() as f32 * fill).round() as usize;
                    let mut rng = self.lot_rng(0xA9A7 ^ (l.x as u64) << 9 ^ l.y as u64);
                    // Fill from the street end in, so a half-empty block has a
                    // clear run of bays rather than a dapple.
                    for (i, b) in bays.iter().enumerate() {
                        if i >= taken {
                            break;
                        }
                        if rng.chance(0.06) {
                            continue; // one resident away at any hour
                        }
                        let variant = rng.usize(8) as u8;
                        out.push(CarView {
                            x: b.x,
                            y: b.y,
                            dir: b.dir,
                            kind: if rng.chance(0.12) {
                                VehicleKind::Van
                            } else {
                                VehicleKind::Car
                            },
                            variant,
                            style: parked_style(variant),
                            lift: b.lift,
                            braking: false,
                        });
                    }
                }
                continue;
            };
            let mut rng = self.lot_rng(0xC0FFEE ^ (l.x as u64) << 8 ^ l.y as u64);
            match kind {
                PlaceKind::DriveThrough => {
                    // The queue is the hour's demand: long at lunch and dinner,
                    // one car at other times.
                    let busy = self.busyness(self.time);
                    let rush = matches!(hod as i32, 11..=13) || matches!(hod as i32, 18..=20);
                    let n = if rush { 5 + (busy * 4.0) as usize } else if busy > 0.4 { 3 } else { 1 };
                    let bays = bays_of(l);
                    for (i, b) in bays.iter().take(n.min(bays.len())).enumerate() {
                        out.push(CarView {
                            x: b.x,
                            y: b.y,
                            dir: b.dir,
                            kind: if i == 0 { VehicleKind::Van } else { VehicleKind::Car },
                            variant: rng.usize(8) as u8,
                            style: parked_style(rng.usize(8) as u8),
                            lift: 0.0,
                            braking: i + 1 < n,
                        });
                    }
                }
                PlaceKind::Dealership => {
                    // The stock: every display bay full, all day.
                    for b in bays_of(l) {
                        out.push(CarView {
                            x: b.x,
                            y: b.y,
                            dir: b.dir,
                            kind: if rng.chance(0.2) { VehicleKind::Van } else { VehicleKind::Car },
                            variant: rng.usize(8) as u8,
                            style: parked_style(rng.usize(8) as u8),
                            lift: 0.0,
                            braking: false,
                        });
                    }
                }
                PlaceKind::Motel => {
                    // Guests' cars outside the doors from dusk, gone by noon.
                    let night = !(9.0..17.0).contains(&hod);
                    if night {
                        for b in bays_of(l) {
                            if rng.chance(0.75) {
                                out.push(CarView {
                                    x: b.x,
                                    y: b.y,
                                    dir: b.dir,
                                    kind: VehicleKind::Car,
                                    variant: rng.usize(8) as u8,
                                    style: parked_style(rng.usize(8) as u8),
                                    lift: 0.0,
                                    braking: false,
                                });
                            }
                        }
                    }
                }
                PlaceKind::CarWash => {
                    // In and out: a car in the tunnel and one waiting, through
                    // the day (the wash is a daytime errand).
                    if (8.0..19.0).contains(&hod) {
                        let bays = bays_of(l);
                        for (i, b) in bays.iter().take(2).enumerate() {
                            out.push(CarView {
                                x: b.x,
                                y: b.y,
                                dir: b.dir,
                                kind: VehicleKind::Car,
                                variant: rng.usize(8) as u8,
                                style: parked_style(rng.usize(8) as u8),
                                lift: 0.0,
                                braking: i == 1,
                            });
                        }
                    }
                }
                // A forecourt is never quite empty: a car at the pumps through
                // the day, an EV on the chargers after dusk.
                PlaceKind::GasStation => {
                    let busy = (7.0..21.0).contains(&hod);
                    let bays = bays_of(l);
                    let pumps: Vec<&Bay> = if bays.len() >= 4 {
                        vec![&bays[0], &bays[bays.len() - 1]]
                    } else {
                        bays.iter().collect()
                    };
                    for (i, b) in pumps.into_iter().take(if busy { 2 } else { 1 }).enumerate() {
                        out.push(CarView {
                            x: b.x,
                            y: b.y,
                            dir: b.dir,
                            kind: VehicleKind::Car,
                            variant: ((l.seed as usize + i) % 8) as u8,
                            style: parked_style(((l.seed as usize + i) % 8) as u8),
                            lift: 0.0,
                            braking: false,
                        });
                    }
                }
                PlaceKind::ChargingHub => {
                    // A car or two on the chargers through the day; the sim's
                    // parked EVs add to these.
                    for b in bays_of(l).into_iter().take(2) {
                        if rng.chance(0.6) {
                            out.push(CarView {
                                x: b.x,
                                y: b.y,
                                dir: b.dir,
                                kind: VehicleKind::Ev,
                                variant: rng.usize(8) as u8,
                                style: parked_style(rng.usize(8) as u8),
                                lift: 0.0,
                                braking: false,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// A house's car in its driveway, or `None` when it is out at work: a
    /// driveway car exists only from the motor-car era, and a household in five
    /// has none (D72: not every house is a garage).
    pub fn driveway_car(&self, l: &Lot, hod: f64) -> Option<CarView> {
        if self.tech.vehicle_era() < 1 || l.seed % 5 == 0 {
            return None;
        }
        // Away on a weekday from eight to six, back at night.
        let away = crate::Sim::weekday_of(self.time) && hod >= 8.0 && hod < 18.0;
        if away && l.seed % 3 != 0 {
            return None;
        }
        // The pad's kerb side, half a tile out from the house.
        let (dx, dy) = match l.front {
            0 => (1.0f32, 0.0f32),
            1 => (0.0, 1.0),
            2 => (-1.0, 0.0),
            _ => (0.0, -1.0),
        };
        let (cx, cy) = (
            l.x as f32 + l.w as f32 * 0.5 + dx * (l.w as f32 * 0.5 + 0.2),
            l.y as f32 + l.h as f32 * 0.5 + dy * (l.h as f32 * 0.5 + 0.2),
        );
        Some(CarView {
            x: cx,
            y: cy,
            dir: (-dx, -dy),
            kind: if self.tech.vehicle_era() >= 3 && l.seed % 4 == 0 {
                VehicleKind::Ev
            } else {
                VehicleKind::Car
            },
            variant: (l.seed % 8) as u8,
            style: parked_style((l.seed % 8) as u8),
            lift: 0.0,
            braking: false,
        })
    }

    /// How busy the roads are at a time (the traffic model's own curve), used
    /// by the car places to size their queues.
    pub fn busyness(&self, time: f64) -> f32 {
        crate::traffic::Traffic::busyness(time)
    }

    /// Is a tile a highway lane?
    pub fn is_highway(&self, x: i32, y: i32) -> bool {
        self.map.in_bounds(x, y) && self.map.at(x, y).aux & AUX_HIGHWAY != 0
    }
}

// ---------------------------------------------------------------- placement

impl Sim {
    /// One car-culture place per call, if the era and the city want one.
    /// Called from `try_town_extras`, so it competes with the other extras and
    /// never spends a whole decision on a second place.
    pub(crate) fn try_car_places(&mut self) -> bool {
        if std::env::var("SIM_NO_CARS").is_ok() {
            return false;
        }
        let era = self.era();
        let ve = self.tech.vehicle_era();
        let n_lots = self.lots.len();
        // Cars need motors: carts do not queue at a filling station.
        if ve < 1 {
            return false;
        }
        // The town's other buildings come first; car places arrive a few hours
        // apart, so a landmark or a park is never starved by them.
        if self.time < self.cars.next_at || n_lots < 40 {
            return false;
        }
        // A town of fifty houses has one gas station, a city four.
        let want = |k: PlaceKind| -> usize {
            let cap = k.max_count();
            match k {
                PlaceKind::CarPark => (n_lots / 120).clamp(1, cap),
                PlaceKind::GasStation => (n_lots / 150).clamp(1, cap),
                PlaceKind::Motel | PlaceKind::Dealership => {
                    if era >= Era::City { 1 } else { 0 }
                }
                _ => 1.min(cap),
            }
        };
        for k in PlaceKind::ALL {
            if era < k.min_era() || self.place_count(k) >= want(k) {
                continue;
            }
            // B49: the latest era turns some filling stations into charging
            // hubs rather than opening a site next door.
            // Chargers arrive with the electric cars (B49's flag), or as soon
            // as the crews have fitted bays beside the kerb.
            if k == PlaceKind::ChargingHub
                && ve < 3
                && !self.tech.is_done(crate::tech::Advance::EvChargers)
            {
                continue;
            }
            // And once the chargers are in, one station is converted rather than
            // a second forecourt being built (B59 item 2).
            if k == PlaceKind::ChargingHub && self.place_count(PlaceKind::GasStation) == 0 {
                continue;
            }
            if self.place_car_place(k) {
                self.cars.next_at = self.time + 5.0 * 3600.0;
                return true;
            }
        }
        false
    }

    /// Turn an existing lot into a car place (B59).
    ///
    /// The land is already spoken for: a forecourt or a dealership replaces a
    /// cottage or a shop on the fringe of town, so the town's housing — and
    /// therefore its era — is not starved of plots by the trade that serves its
    /// cars. The plot is re-staked, so the trade going in is a visible job.
    pub(crate) fn place_car_place(&mut self, k: PlaceKind) -> bool {
        // B59b (D-answer 1): the trade is *zoned* a roadside plot on the edge
        // of the built-up ground, the way the block planner reserves a park,
        // instead of buying a cottage. The conversion stays as the fallback
        // for a town with no clear roadside left.
        if self.zone_roadside_plot(k) {
            return true;
        }
        self.convert_to_car_place(k)
    }

    /// Zone a roadside trade plot: a clear rectangle of the kind's size (either
    /// way round) whose kerb side looks onto a lane of the arterial network,
    /// outside the cells the planner has opened for housing, never next door to
    /// another car place. Searched outward from the founding so the trade is on
    /// the edge of town, deterministic in the seed.
    fn zone_roadside_plot(&mut self, k: PlaceKind) -> bool {
        let (fx, fy) = self.founding;
        let (kw, kh) = k.size();
        let places: Vec<(i32, i32)> = self
            .lots
            .iter()
            .filter(|l| PlaceKind::of_lot(l.kind).is_some())
            .map(|l| (l.x + l.w / 2, l.y + l.h / 2))
            .collect();
        // Every lane tile on the planned grid, nearest the edge of town first
        // (but not the town square: five tiles clear of the founding).
        let mut lanes: Vec<(i32, i32, i32)> = Vec::new();
        for y in 0..self.map.h {
            for x in 0..self.map.w {
                if !self.map.at(x, y).kind.is_lane() {
                    continue;
                }
                let d = (x - fx).abs() + (y - fy).abs();
                if d < 7 || d > 34 {
                    continue;
                }
                lanes.push((x, y, d));
            }
        }
        // Prefer the middle distances (the fringe of a town), tie-break by the
        // tile so the walk is stable.
        lanes.sort_by_key(|&(x, y, d)| ((d - 15).abs(), y, x));
        let mut tried = 0;
        for (lx, ly, _) in lanes {
            // The four sides of a lane tile: the plot starts one tile out (a
            // sidewalk may stand between), in either orientation.
            for (sx, sy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                for &(w, h) in &[(kw, kh), (kh, kw)] {
                    for gap in [1i32, 0] {
                        // The rectangle whose near edge is `gap` tiles from the lane.
                        let (x, y) = match (sx, sy) {
                            (1, 0) => (lx + 1 + gap, ly - h / 2),
                            (-1, 0) => (lx - gap - w, ly - h / 2),
                            (0, 1) => (lx - w / 2, ly + 1 + gap),
                            _ => (lx - w / 2, ly - gap - h),
                        };
                        // The kerb must be the long side for the long kinds
                        // (a motel's rooms face its street).
                        if (sx != 0 && h < w && k == PlaceKind::Motel)
                            || (sy != 0 && w < h && k == PlaceKind::Motel)
                        {
                            continue;
                        }
                        let (cx, cy) = (x + w / 2, y + h / 2);
                        if places.iter().any(|&(px, py)| (px - cx).abs() + (py - cy).abs() < 8) {
                            continue;
                        }
                        // Outside the opened cells: that ground is the housing's.
                        if (y..y + h).any(|yy| (x..x + w).any(|xx| self.in_open_cell(xx, yy))) {
                            continue;
                        }
                        tried += 1;
                        if tried > 4000 {
                            return false;
                        }
                        if self.place_planned(
                            crate::blocks::PlanLot { x, y, w, h, role: crate::blocks::Role::Mid },
                            k.lot_kind(),
                        ) {
                            let dname = self.district_at(x, y).name();
                            self.events.push(crate::Event {
                                time: self.time,
                                text: format!("a {} zoned by the road in the {dname} district", k.name()),
                            });
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Unused for now, kept for the day plots are zoned for the roadside trade:
    /// it is the shape a proper 4x4 forecourt needs (see the handback).
    #[allow(dead_code)]
    fn place_car_plot(&mut self, k: PlaceKind) -> bool {
        let (w, h) = k.size();
        let (fx, fy) = self.founding;
        for ring in 8..26i32 {
            for step in 0..(ring * 8).max(1) {
                let (dx, dy) = ring_step(step, ring);
                let (x, y) = (fx + dx, fy + dy);
                if self.place_planned(
                    crate::blocks::PlanLot {
                        x,
                        y,
                        w,
                        h,
                        role: crate::blocks::Role::Mid,
                    },
                    k.lot_kind(),
                ) {
                    let dname = self.district_at(x, y).name();
                    self.events.push(crate::Event {
                        time: self.time,
                        text: format!("a {} staked in the {dname} district", k.name()),
                    });
                    return true;
                }
            }
        }
        false
    }

    /// Turn an existing fringe lot into a car place, when the town has no free
    /// plot left (B59).
    fn convert_to_car_place(&mut self, k: PlaceKind) -> bool {
        let (fx, fy) = self.founding;
        let places: Vec<(i32, i32)> = self
            .lots
            .iter()
            .filter(|l| PlaceKind::of_lot(l.kind).is_some())
            .map(|l| (l.x + l.w / 2, l.y + l.h / 2))
            .collect();
        // Candidates: a finished cottage or shop out on the fringe (the
        // roadside trade is not on the town square), largest plot first, and
        // never next door to another car place.
        let mut cand: Vec<(usize, i32)> = Vec::new();
        for (i, l) in self.lots.iter().enumerate() {
            if !l.finished() {
                continue;
            }
            let d = self.district_at(l.x, l.y);
            // The fallback buys a shop, never somebody's home: the town's
            // housing decides its era, and zoning (above) is how the trade
            // normally gets its ground.
            let kind_ok = matches!(l.kind, LotKind::Shop | LotKind::Store);
            if !kind_ok || !matches!(d, crate::District::Industrial | crate::District::Suburb) {
                continue;
            }
            let dist = (l.x - fx).abs() + (l.y - fy).abs();
            if dist < 5 {
                continue;
            }
            // A forecourt with no road is a forecourt nobody drives to (D73).
            let mut road = false;
            for ty in l.y - 3..=l.y + l.h + 2 {
                for tx in l.x - 3..=l.x + l.w + 2 {
                    if self.map.at(tx, ty).kind.is_lane() {
                        road = true;
                    }
                }
            }
            if !road {
                continue;
            }
            if places.iter().any(|&(px, py)| (px - l.x).abs() + (py - l.y).abs() < 6) {
                continue;
            }
            // Prefer the biggest plot (a forecourt needs room) and the fringe
            // (where the traffic is), with a stable tie-break.
            cand.push((i, l.w * l.h * 10 + dist / 3 + (l.seed % 7) as i32));
        }
        cand.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let Some(&(i, _)) = cand.first() else {
            return false;
        };
        let dname = self.district_at(self.lots[i].x, self.lots[i].y).name();
        // Take the chosen plot out of the town's books, then let the trade buy
        // up a neighbour or two: a forecourt needs the size of a forecourt, and
        // it takes no land the town was not already using (B59).
        let base = self.lots.remove(i);
        self.retire_roof_fittings(base.x, base.y, base.w, base.h);
        let mut x0 = base.x;
        let mut y0 = base.y;
        let mut x1 = base.x + base.w;
        let mut y1 = base.y + base.h;
        let (want_w, want_h) = k.size();
        let mut absorbed = 0;
        while (x1 - x0 < want_w || y1 - y0 < want_h) && absorbed < 2 {
            let mut best: Option<(usize, i32)> = None;
            let mut best_area = 0;
            for (j, l) in self.lots.iter().enumerate() {
                // Only a shop or a works is bought up with the plot: a cottage
                // is somebody's home, and the town's housing is what decides its
                // era (B59). A shop taken with it is the trade buying a business.
                if !l.finished()
                    || !matches!(l.kind, LotKind::Shop | LotKind::Store)
                    || self.district_at(l.x, l.y) != self.district_at(x0, y0)
                {
                    continue;
                }
                // Beside it (a yard at most between them) and overlapping the
                // other axis, so the union is still one rectangle of ground.
                let gap_x = (l.x - x1).abs() <= 1 || (l.x + l.w - x0).abs() <= 1;
                let gap_y = (l.y - y1).abs() <= 1 || (l.y + l.h - y0).abs() <= 1;
                let beside = gap_x && l.y < y1 && l.y + l.h > y0;
                let above = gap_y && l.x < x1 && l.x + l.w > x0;
                if !beside && !above {
                    continue;
                }
                // The merged pad must be clear ground: no lane, no water, no
                // other building inside it.
                let (nx0, ny0) = (x0.min(l.x), y0.min(l.y));
                let (nx1, ny1) = (x1.max(l.x + l.w), y1.max(l.y + l.h));
                let mut clear = true;
                for ty in ny0..ny1 {
                    for tx in nx0..nx1 {
                        let t = self.map.at(tx, ty);
                        if t.kind.is_water() || t.kind.is_lane() {
                            clear = false;
                        }
                        if self.lots.iter().enumerate().any(|(k, o)| k != j && o.covers(tx, ty)) {
                            clear = false;
                        }
                    }
                }
                if !clear {
                    continue;
                }
                let area = l.w * l.h;
                if area > best_area {
                    best_area = area;
                    best = Some((j, area));
                }
            }
            let Some((j, _)) = best else { break };
            let l = self.lots.remove(j);
            // Its roof array goes with it: a panel is fitted to a building, and
            // this building is not there any more (B49).
            self.retire_roof_fittings(l.x, l.y, l.w, l.h);
            x0 = x0.min(l.x);
            y0 = y0.min(l.y);
            x1 = x1.max(l.x + l.w);
            y1 = y1.max(l.y + l.h);
            absorbed += 1;
        }
        // The plot is graded flat at its street's level, and the street side is
        // whichever edge of the merged rectangle looks onto a lane.
        let level = base.level;
        self.grade_lot(x0, y0, x1 - x0, y1 - y0, level);
        let mut front = base.front;
        let mut nearest = i32::MAX;
        for (code, (px, py)) in [
            (0u8, (x1, (y0 + y1) / 2)),
            (2, (x0 - 1, (y0 + y1) / 2)),
            (1, ((x0 + x1) / 2, y1)),
            (3, ((x0 + x1) / 2, y0 - 1)),
        ] {
            if self.map.in_bounds(px, py) && self.map.at(px, py).kind.is_lane() {
                let d = (px - x0).abs() + (py - y0).abs();
                if d < nearest {
                    nearest = d;
                    front = code;
                }
            }
        }
        self.lots.push(Lot {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
            level,
            kind: k.lot_kind(),
            storeys: k.lot_kind().base_storeys().max(1),
            style: self.district_at(x0, y0).style(),
            seed: base.seed ^ ((x0 as u32) << 16) ^ (y0 as u32),
            front,
            // The trade goes in as a job: staked, graded, built, open.
            stage: crate::Stage::Staked,
            stage_end: self.time + HOUSE_SECONDS / 3.0,
            started: self.time,
        });
        self.events.push(crate::Event {
            time: self.time,
            text: format!("a {} staked in the {dname} district", k.name()),
        });
        true
    }
}

impl Sim {
    /// Forget the technology fitted to a building that is being taken down:
    /// the panels on its roof go with it (B49/B59).
    fn retire_roof_fittings(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.tech
            .solar
            .retain(|p, _| !(p.0 >= x && p.0 < x + w && p.1 >= y && p.1 < y + h));
    }
}

/// The `n`-th step round a square ring of radius `r` (a closed walk), used to
/// search outward from the founding without a growing allocation.
fn ring_step(n: i32, r: i32) -> (i32, i32) {
    let side = (r * 2).max(1);
    let per = side * 4;
    let mut s = n.rem_euclid(per);
    // Top edge, right edge, bottom edge, left edge.
    if s < side {
        let d = (-r + s, -r);
        s -= side;
        let _ = s;
        return d;
    }
    s -= side;
    if s < side {
        return (r, -r + s);
    }
    s -= side;
    if s < side {
        return (r - s, r);
    }
    s -= side;
    (-r, r - s)
}

// ------------------------------------------------------------------ highway

impl Sim {
    /// Once the world is a metropolis, cut a highway along its edge and a
    /// diamond interchange to the nearest avenue (B59 item 4).
    ///
    /// The corridor is chosen on the least-built edge: a two-lane straight run
    /// of asphalt with narrow verges, so it reads as a road that passes the city
    /// rather than one of its streets.
    pub(crate) fn plan_highway(&mut self) -> bool {
        self.plan_highway_at(HIGHWAY_RESIDENTS)
    }

    /// The highway, once the city era is here *and* the city has `min`
    /// residents (B59b design answer: 1 500, not the metropolis' 4 000).
    pub(crate) fn plan_highway_at(&mut self, min: u32) -> bool {
        if self.cars.planned_highway || self.era() < Era::City || self.residents() < min {
            return false;
        }
        let (w, h) = (self.map.w, self.map.h);
        // Four candidate lines, one per edge, a few tiles in from the frame.
        let inset = 4;
        let mut best: Option<((i32, i32), (i32, i32), i32)> = None; // (start, dir, score)
        for side in 0..4 {
            let (sx, sy, dx, dy, len) = match side {
                0 => (inset, inset, 0, 1, h - inset * 2),
                1 => (w - inset - 1, inset, 0, 1, h - inset * 2),
                2 => (inset, inset, 1, 0, w - inset * 2),
                _ => (inset, h - inset - 1, 1, 0, w - inset * 2),
            };
            let mut score = 0i32;
            let mut run = 0i32;
            for i in 0..len {
                let (x, y) = (sx + dx * i, sy + dy * i);
                let t = self.map.at(x, y);
                let free = !t.kind.is_water()
                    && !self.on_airfield(x, y)
                    && t.is_flat()
                    && t.relief() <= 1;
                let built = self.lots.iter().any(|l| l.covers(x, y));
                if free && !built {
                    run += 1;
                    score = score.max(run);
                } else {
                    run = 0;
                }
            }
            if score > 12 && best.map(|b| b.2 < score).unwrap_or(true) {
                best = Some(((sx, sy), (dx, dy), score));
            }
        }
        let Some(((sx, sy), (dx, dy), _)) = best else {
            return false;
        };
        // Play the line out as two lanes, stopping at anything in the way. The
        // old tiles are kept so a highway that ends up joined to nothing is
        // put back: the city's roads must stay one network (B26).
        let (mut x, mut y) = (sx, sy);
        let mut tiles = Vec::new();
        let mut old: Vec<((i32, i32), map::Tile)> = Vec::new();
        let mut rng = self.rng(0x11_1111);
        let _ = rng.next_u32();
        for _ in 0..400 {
            if !self.map.in_bounds(x, y) {
                break;
            }
            let mut ok = true;
            for lane in 0..2 {
                let (px, py) = (x + if dx != 0 { 0 } else { lane }, y + if dy != 0 { 0 } else { lane });
                let t = self.map.at(px, py);
                // B99c: not across the airfield's ground. The highway is laid
                // along a frame edge, which is exactly where an airfield goes,
                // and a motorway down the runway is the one lane the airfield
                // never saw coming (D73).
                if t.kind.is_water()
                    || t.relief() > 2
                    || self.on_airfield(px, py)
                    || self.lots.iter().any(|l| l.covers(px, py))
                {
                    ok = false;
                }
            }
            if !ok {
                if tiles.len() < 12 {
                    // Nothing yet: slide the start along and try again.
                    x += dx;
                    y += dy;
                    continue;
                }
                break;
            }
            for lane in 0..2 {
                let (px, py) = (x + if dx != 0 { 0 } else { lane }, y + if dy != 0 { 0 } else { lane });
                let level = self.map.at(px, py).min_level();
                let mut t = self.map.at(px, py);
                old.push(((px, py), t));
                t.kind = map::Kind::Road;
                t.corner = [level; 4];
                t.prop = map::Prop::None;
                t.aux |= AUX_HIGHWAY;
                self.write(px, py, t);
                self.remark_around(px, py);
                tiles.push((px, py));
            }
            x += dx;
            y += dy;
        }
        if tiles.len() < 24 {
            for (p, t) in old {
                self.write(p.0, p.1, t);
            }
            return false;
        }
        self.cars.highway = tiles;
        self.cars.planned_highway = true;
        // The interchange: two legs from the highway to the nearest avenue,
        // one going each way along it — a diamond, which is what a city's edge
        // gets when a highway passes it.
        // Inward is perpendicular to the highway, toward the middle of the map
        // (the highway runs along an edge).
        let (px, py) = (-dy, dx);
        let (cx, cy) = (w / 2, h / 2);
        let mid_tile = self.cars.highway[self.cars.highway.len() / 2];
        let to = ((cx - mid_tile.0).signum(), (cy - mid_tile.1).signum());
        let into = if px * to.0 + py * to.1 >= 0 {
            (px, py)
        } else {
            (-px, -py)
        };
        let mut laid = 0;
        let mut tried = Vec::new();
        let mid = self.cars.highway.len() / 2;
        for k in 0..self.cars.highway.len() {
            // Walk out from the middle along the highway in both directions.
            let off = (k as i32 / 2) * if k % 2 == 0 { 1 } else { -1 };
            let i = mid as i32 + off;
            if i < 0 || i as usize >= self.cars.highway.len() {
                continue;
            }
            let tile = self.cars.highway[i as usize];
            if laid == 0 {
                if self.cut_ramp(tile, into) {
                    laid = 1;
                    tried.push(tile);
                }
            } else if tried.iter().all(|&(x, y)| (x - tile.0).abs() + (y - tile.1).abs() > 8) {
                if self.cut_ramp(tile, into) {
                    laid = 2;
                    break;
                }
            }
        }
        self.cars.ramps = self.ramps_laid();
        if laid == 0 || self.cars.ramps.is_empty() {
            // Joined to nothing: take it up again and try another day. A
            // highway that touches no street is a road to nowhere (D73).
            for (p, t) in old {
                self.write(p.0, p.1, t);
            }
            self.cars.highway.clear();
            self.cars.ramps.clear();
            self.cars.planned_highway = false;
            return false;
        }
        self.events.push(crate::Event {
            time: self.time,
            text: "the highway opens on the edge of the city, with an interchange".into(),
        });
        self.push_story("the highway opens on the edge of the city, with an interchange");
        true
    }

    /// The ramp tiles laid so far (derived from the map, so nothing extra is
    /// saved): lanes beside the highway that are not highway themselves.
    fn ramps_laid(&self) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        for &(hx, hy) in &self.cars.highway {
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (x, y) = (hx + dx, hy + dy);
                if self.map.in_bounds(x, y)
                    && self.map.at(x, y).kind.is_lane()
                    && self.map.at(x, y).aux & AUX_HIGHWAY == 0
                {
                    v.push((x, y));
                }
            }
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Cut one leg of the interchange: straight inward from a highway tile
    /// until it meets a street (or 14 tiles of open ground). Refuses water,
    /// plots and steep ground, so a ramp never cuts a building in half.
    fn cut_ramp(&mut self, from: (i32, i32), into: (i32, i32)) -> bool {
        let (mut x, mut y) = from;
        let mut laid = Vec::new();
        for _ in 0..14 {
            x += into.0;
            y += into.1;
            if !self.map.in_bounds(x, y) {
                return false;
            }
            let t = self.map.at(x, y);
            if t.kind.is_lane() {
                self.cars.ramps.extend(laid);
                return true;
            }
            // B99c: a ramp that runs across the airfield lays a street down the
            // runway (the interchange is cut from the frame edge, which is where
            // the airfield sits).
            if t.kind.is_water()
                || t.relief() > 2
                || self.on_airfield(x, y)
                || self.lots.iter().any(|l| l.covers(x, y))
            {
                return false;
            }
            let level = t.min_level();
            let mut nt = t;
            nt.kind = map::Kind::Road;
            nt.aux |= map::AUX_RAMP;
            nt.corner = [level; 4];
            nt.prop = map::Prop::None;
            self.write(x, y, nt);
            self.remark_around(x, y);
            laid.push((x, y));
        }
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parked_cars_do_not_change_from_one_planning_step_to_the_next() {
        let mut s = crate::Sim::new(5, crate::Pace::Month, 107, 107);
        s.run_to(40.0 * 86400.0);
        // Pin an evening hour (the flats' bays are full) and compare two steps.
        s.set_clock_hour(21.0);
        let a = s.car_views();
        assert!(a.len() > 20, "only {} cars standing: the test needs a parked city", a.len());
        s.step_index += 1;
        let b = s.car_views();
        assert_eq!(a, b, "the parked cars changed with the planning step");
    }

    use super::*;

    fn lot(kind: LotKind, x: i32, y: i32, w: i32, h: i32, front: u8) -> Lot {
        Lot {
            x,
            y,
            w,
            h,
            level: 2,
            kind,
            storeys: 1,
            style: 0,
            seed: 7,
            front,
            stage: crate::Stage::Finished,
            stage_end: 0.0,
            started: 0.0,
        }
    }

    #[test]
    fn bays_sit_inside_their_plot() {
        for k in PlaceKind::ALL {
            let (w, h) = k.size();
            for front in 0..4u8 {
                let l = lot(k.lot_kind(), 10, 10, w, h, front);
                let bays = bays_of(&l);
                assert!(!bays.is_empty(), "{k:?} front {front} has no bays");
                for b in &bays {
                    assert!(
                        b.x >= 10.0 && b.x <= 10.0 + w as f32 && b.y >= 10.0 && b.y <= 10.0 + h as f32,
                        "{k:?} front {front}: bay {} at {},{} outside the plot",
                        b.lift,
                        b.x,
                        b.y
                    );
                }
            }
        }
    }

    #[test]
    fn the_car_parks_hold_more_cars_than_a_house() {
        let flat = lot(LotKind::CarPark, 10, 10, 4, 4, 1);
        // Three cars to a tile's frontage and two double banks: a 4x4 car park
        // is a real car park, not four lonely cars in a field.
        assert_eq!(parking_capacity(&flat), 40, "a 4x4 surface car park holds four banks of ten");
        let deck = lot(LotKind::CarParkDeck, 10, 10, 4, 4, 1);
        assert!(
            parking_capacity(&deck) >= 2 * parking_capacity(&flat),
            "three decks of the same footprint, less the stair core"
        );
        assert!(bays_of(&deck).iter().any(|b| b.lift > 0.0));
        let house = lot(LotKind::House, 10, 10, 2, 2, 1);
        assert_eq!(parking_capacity(&house), 0);
    }

    #[test]
    fn only_finished_places_offer_parking() {
        let mut l = lot(LotKind::CarPark, 10, 10, 4, 4, 1);
        l.stage = crate::Stage::Framing;
        assert_eq!(parking_capacity(&l), 0);
    }

    #[test]
    fn every_place_kind_round_trips_through_its_code() {
        for k in PlaceKind::ALL {
            assert_eq!(PlaceKind::of_lot(k.lot_kind()), Some(k));
            let code = crate::save::lot_kind_code(k.lot_kind());
            assert_eq!(crate::save::lot_kind_from_code(code).unwrap(), k.lot_kind());
        }
    }

    fn world(seed: u64, days: f64) -> Sim {
        let vp = map::world_for_viewport(2347, 480);
        // Pinned to the temperate land: B46e's second land roll moves some
        // seeds to the new lands, and these tests measure a temperate town.
        let mut s = crate::Sim::with_focus_in(
            seed,
            crate::Pace::Month,
            vp.w,
            vp.h,
            (vp.w as f32 * 0.5, vp.h as f32 * 0.5),
            Some(map::Biome::Temperate),
        );
        s.run_to(days * 86400.0);
        s
    }

    /// Every car place has a road, stands on the right ground, and keeps its
    /// distance from the next one (B59 item 1's placement rules).
    #[test]
    fn car_places_sit_by_a_road_on_the_fringe() {
        let s = world(7, 25.0);
        let places = s.car_places();
        assert!(places.len() >= 6, "only {} car places in a city", places.len());
        for l in &places {
            let mut road = false;
            for ty in l.y - 3..=l.y + l.h + 2 {
                for tx in l.x - 3..=l.x + l.w + 2 {
                    if s.map.at(tx, ty).kind.is_lane() {
                        road = true;
                    }
                }
            }
            assert!(road, "{:?} at {},{} has no road", l.kind, l.x, l.y);
            let d = s.district_at(l.x, l.y);
            assert!(
                !matches!(d, crate::District::Core | crate::District::Downtown),
                "{:?} stands in the {:?} district, not the edge of town",
                l.kind,
                d
            );
            for o in &places {
                if std::ptr::eq(o, l) {
                    continue;
                }
                let gap = (o.x - l.x).abs() + (o.y - l.y).abs();
                assert!(gap >= 4, "{:?} and {:?} are {gap} tiles apart", l.kind, o.kind);
            }
        }
    }

    /// Cars need motors: a village of carts has no filling station.
    #[test]
    fn no_car_places_before_the_motor_car() {
        let s = world(7, 2.0);
        assert_eq!(s.tech.vehicle_era(), 0);
        assert!(
            s.car_places().is_empty(),
            "a cart village staked {:?}",
            s.car_places().iter().map(|l| l.kind).collect::<Vec<_>>()
        );
    }

    /// Capacity is real: the bays a car park offers are distinct places, the
    /// deck holds more than the same ground as a surface park, and a lot under
    /// construction offers nothing.
    #[test]
    fn parking_capacity_is_honest() {
        let s = world(7, 25.0);
        let mut total = 0;
        for l in s.car_places() {
            let bays = bays_of(l);
            let mut seen = std::collections::BTreeSet::new();
            for b in &bays {
                let key = (
                    (b.x * 10.0) as i32,
                    (b.y * 10.0) as i32,
                    (b.lift * 10.0) as i32,
                );
                assert!(seen.insert(key), "{:?} offers one bay twice at {:?}", l.kind, b);
            }
            total += parking_capacity(l);
            if l.kind == LotKind::CarParkDeck {
                let surface = lot(LotKind::CarPark, l.x, l.y, l.w, l.h, l.front);
                assert!(
                    parking_capacity(l) >= bays_of(&surface).len() * 2,
                    "the deck holds no more than the ground it stands on"
                );
            }
        }
        assert_eq!(total, s.parking_capacity());
        assert!(total >= 24, "a city with {total} bays has nowhere to park");
    }

    /// The trip schedule: car parks fill in the working day and empty at night,
    /// and the cars that park are standing in bays that exist.
    #[test]
    fn the_working_day_fills_the_bays_and_the_night_empties_them() {
        let mut s = world(7, 25.0);
        // Morning: the cars are out and parked at work/shops.
        s.set_clock_hour(8.5);
        s.warm_traffic(900.0);
        let morning = s.cars.parked.len();
        assert!(morning > 0, "no car parked at 08:30");
        for p in &s.cars.parked {
            let place = s
                .car_places()
                .into_iter()
                .find(|l| (l.x + l.w / 2, l.y + l.h / 2) == p.slot.at)
                .expect("a parked car is not at a car place");
            assert!(
                bays_of(place)
                    .iter()
                    .any(|b| (b.x - p.slot.bay.x).abs() < 0.05 && (b.y - p.slot.bay.y).abs() < 0.05),
                "parked car is not in a bay of {:?}",
                place.kind
            );
        }
        // Night: the owners have come back for them.
        s.set_clock_hour(19.5);
        for _ in 0..1200 {
            s.traffic_step(0.1);
        }
        assert!(s.cars.departures > 0, "no car ever left its bay");
        assert!(
            s.cars.parked.len() < morning,
            "the bays did not empty: {} in the evening, {morning} in the morning",
            s.cars.parked.len()
        );
    }

    /// The highway is cut at the city era, its tiles are flagged, and a route
    /// can use it — which is what makes traffic flow faster on it.
    #[test]
    fn the_highway_is_a_road_with_ramps() {
        let mut s = world(7, 25.0);
        // The real gate is the city era plus 1 500 residents, which a world
        // this size does not reach in 25 days; cut it here at the city era.
        assert!(!s.cars.planned_highway || s.residents() >= HIGHWAY_RESIDENTS);
        s.cars.planned_highway = false;
        s.cars.highway.clear();
        assert!(s.plan_highway_at(0), "no highway at the city era");
        assert!(s.cars.highway.len() >= 40, "highway of {} tiles", s.cars.highway.len());
        for &(x, y) in &s.cars.highway {
            assert!(s.map.at(x, y).kind.is_lane(), "highway tile {x},{y} is not a lane");
            assert_ne!(
                s.map.at(x, y).aux & AUX_HIGHWAY,
                0,
                "highway tile {x},{y} is not flagged"
            );
            assert!(s.is_highway(x, y));
        }
        // Some trip drives along it.
        s.set_clock_hour(8.0);
        s.warm_traffic(600.0);
        let on_it = s
            .traffic
            .vehicles
            .iter()
            .filter(|v| v.hw.1 > v.hw.0)
            .count();
        assert!(on_it > 0, "no trip uses the highway");
    }

    #[test]
    fn placement_is_a_ring_walk() {
        // No duplicates in one ring, and every step is on the ring.
        for r in 1..4 {
            let mut seen = std::collections::BTreeSet::new();
            for n in 0..(r * 8) {
                let (dx, dy) = ring_step(n, r);
                assert!(dx.abs() == r || dy.abs() == r, "off the ring at {dx},{dy}");
                seen.insert((dx, dy));
            }
            assert_eq!(seen.len() as i32, r * 8);
        }
    }
}
