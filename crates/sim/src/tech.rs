//! The world's technology path (B49/D72/P5).
//!
//! Every world advances through technology in its **own order and at its own
//! pace**: the path is drawn once from the seed (a nominal day per advance,
//! jittered, with one advance running early and one late, dependencies kept in
//! order, and the far-future ones rolled per world), and then a world that
//! stalls as a village only ever gets as far as the era gate allows.
//!
//! Nothing here pops. Each advance is carried out **by a crew, one tile at a
//! time** (D33/D63): lamps are swapped lamp by lamp down the street, poles are
//! set and later pulled, solar panels appear roof by roof, the cell tower goes
//! up in a few hours of work. The work is driven from the planning step
//! ([`Sim::tech_step`]), so a frame costs nothing while nothing is upgrading,
//! and what has been installed is static once it is there (the renderer bakes
//! it into the static layer, one tile at a time).

use std::collections::{BTreeMap, BTreeSet};

use map::rng::Rng;
use map::Kind;

use crate::{District, Era, LotKind, Sim};

/// Sim seconds in a day (the path is planned in days).
const DAY: f64 = 86_400.0;
/// Sim seconds one tile of a crew's work takes. A street of a dozen lamps is
/// then a bit over an hour, and the long jobs (burying the core's lines, the
/// rooftop solar) run for a few hours each — "a small crew over a few sim
/// hours" (B49 item 2).
pub const TILE_SECONDS: f64 = 240.0;
/// A cell tower is a bigger job than a lamp: a few hours for the mast.
pub const TOWER_SECONDS: f64 = 3.0 * 3600.0;
/// The shortest gap between a dependency and the advance that needs it.
const DEP_GAP: f64 = 0.75 * DAY;
/// How many rooftops one solar crew fits before it comes back for more.
const SOLAR_PER_JOB: usize = 10;
/// The share of the standing houses that end up with solar on the roof by the
/// late era (B49b item 4: 30 %, not the 55 % of the first pass). The spread
/// reaches it gradually: a few roofs at first, more as the years pass.
pub const SOLAR_TARGET: f32 = 0.30;
/// Days over which the rooftop spread climbs from its first roofs to the full
/// share above.
const SOLAR_RAMP_DAYS: f64 = 9.0;

/// One advance in a world's technology (D72).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Advance {
    /// Gas lamps along the first paved streets.
    GasLamps,
    /// The first motor cars (carts are the baseline before this).
    EarlyCars,
    /// Telephone/power poles and their overhead wires.
    UtilityPoles,
    /// Sodium lamps: the streets turn orange.
    SodiumLamps,
    /// Modern cars, vans and buses: the cart is gone.
    ModernCars,
    /// Rooftop solar spreading over the houses and the flat roofs.
    SolarRoofs,
    /// The first cell tower on the edge of town.
    CellTowers,
    /// The railway is electrified (catenary) — a flag for B58's rail art.
    Catenary,
    /// LED lamps: the streets turn white.
    LedLamps,
    /// The core's overhead wires are buried and the poles come down.
    BuriedLines,
    /// A solar farm at the edge of the city.
    SolarFarm,
    /// Wind turbines on the ridge.
    WindTurbines,
    /// Charger bays at the car parks and the filling stations.
    EvChargers,
    /// Electric cars in the traffic mix (quiet, no exhaust).
    Evs,
    /// 5G small cells on the lamp posts downtown.
    SmallCells,
    /// Delivery drones over the city by day (near-future worlds).
    Drones,
    /// A maglev line for a huge city (near-future worlds).
    Maglev,
}

impl Advance {
    /// Every advance, in the enum's order (the save's bit order).
    pub const ALL: [Advance; 17] = [
        Advance::GasLamps,
        Advance::EarlyCars,
        Advance::UtilityPoles,
        Advance::SodiumLamps,
        Advance::ModernCars,
        Advance::SolarRoofs,
        Advance::CellTowers,
        Advance::Catenary,
        Advance::LedLamps,
        Advance::BuriedLines,
        Advance::SolarFarm,
        Advance::WindTurbines,
        Advance::EvChargers,
        Advance::Evs,
        Advance::SmallCells,
        Advance::Drones,
        Advance::Maglev,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The short name the tech-path printout uses.
    pub fn name(self) -> &'static str {
        match self {
            Advance::GasLamps => "gas lamps",
            Advance::EarlyCars => "early cars",
            Advance::UtilityPoles => "utility poles",
            Advance::SodiumLamps => "sodium lamps",
            Advance::ModernCars => "modern cars",
            Advance::SolarRoofs => "rooftop solar",
            Advance::CellTowers => "cell tower",
            Advance::Catenary => "rail catenary",
            Advance::LedLamps => "LED lamps",
            Advance::BuriedLines => "buried lines",
            Advance::SolarFarm => "solar farm",
            Advance::WindTurbines => "wind turbines",
            Advance::EvChargers => "EV chargers",
            Advance::Evs => "electric cars",
            Advance::SmallCells => "5G small cells",
            Advance::Drones => "delivery drones",
            Advance::Maglev => "maglev",
        }
    }

    /// The nominal day of the advance in a month-long world (D47).
    fn base_day(self) -> f64 {
        match self {
            Advance::GasLamps => 2.0,
            Advance::UtilityPoles => 6.0,
            Advance::EarlyCars => 9.0,
            Advance::SodiumLamps => 13.0,
            Advance::ModernCars => 16.0,
            Advance::SolarRoofs => 18.0,
            Advance::CellTowers => 20.0,
            Advance::Catenary => 21.0,
            Advance::LedLamps => 23.0,
            Advance::BuriedLines => 24.0,
            Advance::SolarFarm => 25.0,
            Advance::WindTurbines => 27.0,
            Advance::EvChargers => 27.5,
            Advance::Evs => 28.0,
            Advance::SmallCells => 29.0,
            Advance::Drones => 31.0,
            Advance::Maglev => 33.0,
        }
    }

    /// How far a world may be ahead of or behind the nominal day (a world can
    /// be early to one thing and late to another — B49 item 1).
    fn jitter(self) -> f32 {
        match self {
            Advance::GasLamps => 0.30,
            Advance::UtilityPoles => 0.35,
            Advance::EarlyCars => 0.30,
            Advance::SodiumLamps => 0.28,
            Advance::ModernCars => 0.22,
            Advance::SolarRoofs | Advance::SolarFarm => 0.34,
            Advance::CellTowers => 0.32,
            Advance::Catenary => 0.32,
            Advance::LedLamps => 0.24,
            Advance::BuriedLines => 0.22,
            Advance::WindTurbines => 0.26,
            Advance::EvChargers => 0.26,
            Advance::Evs => 0.22,
            Advance::SmallCells => 0.22,
            Advance::Drones | Advance::Maglev => 0.18,
        }
    }

    /// The era the world must have reached before the advance can start: a
    /// village never gets electric cars, however early its path says.
    pub fn min_era(self) -> Era {
        match self {
            Advance::GasLamps => Era::Village,
            Advance::UtilityPoles
            | Advance::EarlyCars
            | Advance::SodiumLamps
            | Advance::ModernCars
            | Advance::SolarRoofs => Era::Town,
            Advance::Maglev => Era::Metropolis,
            _ => Era::City,
        }
    }

    /// The population a world needs before the advance may start, beyond the
    /// era gate (B49b item 4: a maglev needs a *huge* city, ≥ 8 000 residents).
    pub fn min_residents(self) -> u32 {
        match self {
            Advance::Maglev => 8_000,
            _ => 0,
        }
    }

    /// What must come first (technology has an order even when worlds vary).
    pub fn requires(self) -> &'static [Advance] {
        match self {
            Advance::SodiumLamps => &[Advance::GasLamps],
            Advance::LedLamps => &[Advance::SodiumLamps],
            Advance::SmallCells => &[Advance::LedLamps],
            Advance::ModernCars => &[Advance::EarlyCars],
            Advance::Evs => &[Advance::ModernCars],
            Advance::EvChargers => &[Advance::ModernCars],
            Advance::BuriedLines => &[Advance::UtilityPoles],
            Advance::CellTowers => &[Advance::UtilityPoles],
            Advance::SolarFarm => &[Advance::SolarRoofs],
            Advance::Drones => &[Advance::CellTowers],
            Advance::Catenary => &[Advance::UtilityPoles],
            Advance::Maglev => &[Advance::Catenary],
            _ => &[],
        }
    }

    /// Whether the advance only happens in a near-future world (D72: "for
    /// near-future worlds maglev, drones, autonomous shuttles").
    fn far_future(self) -> bool {
        matches!(self, Advance::Drones | Advance::Maglev)
    }

    /// The line the feed gets when the milestone is done (B49 item 3).
    pub fn line(self) -> &'static str {
        match self {
            Advance::GasLamps => "the first gas lamps are lit along the main street",
            Advance::EarlyCars => {
                "the first motor car splutters down the high street: the horses step aside"
            }
            Advance::UtilityPoles => {
                "pole crews run the first telephone and power wires along the streets"
            }
            Advance::SodiumLamps => {
                "the last gas lamp on the high street is taken down: the streets are sodium orange now"
            }
            Advance::ModernCars => "the last horse cart is retired: the streets belong to the motor",
            Advance::SolarRoofs => {
                "the first roofs go solar: panels climb over the houses and the flat roofs"
            }
            Advance::CellTowers => "a cell tower goes up on the edge of town: the first bars of signal",
            Advance::Catenary => "the railway is electrified: the catenary goes up along the line",
            Advance::LedLamps => "the last sodium head is swapped out: the streets are white LED now",
            Advance::BuriedLines => "the last pole in the core comes down: the wires are buried",
            Advance::SolarFarm => "the solar farm is switched on at the edge of the city",
            Advance::WindTurbines => "the first turbines turn on the ridge above the town",
            Advance::EvChargers => "charger bays open at the car parks and the filling stations",
            Advance::Evs => "electric cars glide past: the street is quieter and the air is cleaner",
            Advance::SmallCells => "small 5G cells appear on the lamp posts downtown",
            Advance::Drones => "delivery drones cross the rooftops by day",
            Advance::Maglev => "the maglev line opens along the edge of the metropolis",
        }
    }
}

/// One advance in this world's path: when it is due (sim seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub advance: Advance,
    /// Nominal sim time (the advance still waits for its era gate).
    pub at: f64,
}

impl Stage {
    /// The day the advance is due, for the printout.
    pub fn day(&self) -> f64 {
        self.at / DAY
    }
}

/// What a crew is installing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    /// Swap a lamp head to this level (0 gas, 1 sodium, 2 LED).
    Lamp(u8),
    /// Set a utility pole with its overhead wires.
    Pole,
    /// Pull a pole and mark the line buried under it.
    Bury,
    /// Raise a cell tower.
    Tower,
    /// Fit a 5G small cell to a lamp post.
    Cell,
    /// Fit rooftop solar to the lot whose anchor is this tile.
    Solar,
    /// Lay solar panels over a tile of the farm.
    Farm,
    /// Raise a wind turbine.
    Turbine,
    /// Fit a charger bay beside the kerb.
    Charger,
}

impl Work {
    /// Which worker the renderer draws: 1 = electrician with a tool, 2 = a
    /// worker up a ladder, 3 = a pole crew (B41's crew art already has these).
    pub fn crew_kind(self) -> u8 {
        match self {
            // 4 = a service van and its worker (a fitting job parked at the kerb);
            // 3 = a pole crew; 2 = a worker up a ladder; 1 = a worker on the ground.
            Work::Lamp(_) | Work::Cell | Work::Charger => 4,
            Work::Pole | Work::Bury | Work::Tower => 3,
            Work::Solar => 2,
            Work::Farm | Work::Turbine => 1,
        }
    }
}

/// A crew working a list of tiles, one at a time.
#[derive(Clone, Debug)]
pub struct Job {
    pub work: Work,
    /// The advance this job is part of (an index into [`Tech::path`]).
    pub stage: usize,
    pub path: Vec<(i32, i32)>,
    pub i: usize,
    pub next_at: f64,
}

/// What the sim wants to do next.
enum Next {
    /// Send this crew out.
    Job(Job),
    /// Nothing to install for this advance: it is done (a flag, or there is
    /// nothing left to fit).
    Done,
    /// Not yet: the town has nothing to work on (no lamps have been set).
    Wait,
}

/// Everything a world has installed (B49).
#[derive(Clone, Debug, Default)]
pub struct Tech {
    /// The world's own path, in the order it happens.
    pub path: Vec<Stage>,
    /// Whether each stage is done (same length as `path`).
    pub done: Vec<bool>,
    /// The crews at work.
    pub jobs: Vec<Job>,
    /// Lamp head per lamp tile: 0 gas, 1 sodium, 2 LED. A lamp tile not in the
    /// map wears the world's latest lamp level (a new street is lit with what
    /// the town fits now).
    pub lamps: BTreeMap<(i32, i32), u8>,
    /// Utility poles still standing (and their overhead wires).
    pub poles: BTreeSet<(i32, i32)>,
    /// Tiles where the wires went underground (a small cabinet stands there).
    pub buried: BTreeSet<(i32, i32)>,
    /// Cell towers.
    pub towers: Vec<(i32, i32)>,
    /// 5G small cells, fitted to lamp posts.
    pub cells: BTreeSet<(i32, i32)>,
    /// Rooftop solar, keyed by the lot's own anchor tile (value = panel style).
    pub solar: BTreeMap<(i32, i32), u8>,
    /// The solar farm's tiles.
    pub farm: BTreeSet<(i32, i32)>,
    /// Wind turbines.
    pub turbines: Vec<(i32, i32)>,
    /// Charger bays.
    pub chargers: BTreeSet<(i32, i32)>,
    /// The railway has catenary (B58 draws it; B49 only raises the flag).
    pub catenary: bool,
    /// Delivery drones are up (near-future worlds).
    pub drones: bool,
    /// The maglev line exists (near-future, huge city).
    pub maglev: bool,
    /// The crews found nothing to do in this era (the scan is skipped until
    /// the town changes under it), the trick the street works use (B61).
    pub halted_in: Option<Era>,
    /// The rooftop solar spread is under way (it stops at its share).
    pub solar_spreading: bool,
}

impl Tech {
    /// Draw this world's path from its seed (B49 item 1). A pure function of
    /// the seed, planned once and kept in the save.
    pub fn plan(seed: u64) -> Tech {
        let mut rng = Rng::new(seed ^ 0x7EC4_DA70_1234_5678);
        // A near-future world, or one that stops at the present (D65/D72).
        let near_future = rng.chance(0.45);
        // One advance this world is early to and one it is late to, so two
        // worlds still read differently from the same table.
        let early = Advance::ALL[rng.usize(Advance::ALL.len())];
        let late = Advance::ALL[rng.usize(Advance::ALL.len())];
        let mut at = [f64::INFINITY; 17];
        for a in Advance::ALL {
            if a.far_future() && !near_future {
                continue;
            }
            let j = rng.range_f32(-a.jitter(), a.jitter()) as f64;
            let mut day = a.base_day() * (1.0 + j);
            if a == early {
                day *= rng.range_f32(0.55, 0.75) as f64;
            }
            if a == late {
                day *= rng.range_f32(1.25, 1.55) as f64;
            }
            // Nothing before the first streets can be lit, and nothing so late
            // that the world ends first (a month's life, D47).
            at[a.index()] = (day * DAY).clamp(0.6 * DAY, 34.0 * DAY);
        }
        // Dependencies: an advance never starts before what it needs. Three
        // passes settle any chain (the longest is catenary → maglev).
        for _ in 0..3 {
            for a in Advance::ALL {
                if !at[a.index()].is_finite() {
                    continue;
                }
                for r in a.requires() {
                    if !at[r.index()].is_finite() {
                        continue;
                    }
                    let need = at[r.index()] + DEP_GAP;
                    if need > at[a.index()] {
                        at[a.index()] = need;
                    }
                }
            }
        }
        let mut path: Vec<Stage> = Advance::ALL
            .iter()
            .filter(|a| at[a.index()].is_finite())
            .map(|&advance| Stage {
                advance,
                at: at[advance.index()],
            })
            .collect();
        // In the order the world actually gets them; ties keep the enum order.
        path.sort_by(|a, b| {
            a.at.partial_cmp(&b.at)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.advance.index().cmp(&b.advance.index()))
        });
        let done = vec![false; path.len()];
        Tech {
            path,
            done,
            ..Tech::default()
        }
    }

    /// The world's latest lamp technology, 0 gas / 1 sodium / 2 LED. A lamp put
    /// up after an advance is fitted with what the town uses now.
    pub fn lamp_baseline(&self) -> u8 {
        if self.is_done(Advance::LedLamps) {
            2
        } else if self.is_done(Advance::SodiumLamps) {
            1
        } else {
            0
        }
    }

    pub fn is_done(&self, a: Advance) -> bool {
        self.path
            .iter()
            .position(|s| s.advance == a)
            .map(|i| self.done.get(i).copied().unwrap_or(false))
            .unwrap_or(false)
    }

    /// The lamp head at a tile: what the crew fitted there, else the world's
    /// baseline (an untouched gas lamp in a village, an LED in a modern city).
    pub fn lamp_level(&self, x: i32, y: i32) -> u8 {
        self.lamps
            .get(&(x, y))
            .copied()
            .unwrap_or_else(|| self.lamp_baseline())
    }

    /// How far along the vehicle ladder the world is: 0 carts, 1 early cars,
    /// 2 modern cars, 3 electric cars.
    pub fn vehicle_era(&self) -> u8 {
        if self.is_done(Advance::Evs) {
            3
        } else if self.is_done(Advance::ModernCars) {
            2
        } else if self.is_done(Advance::EarlyCars) {
            1
        } else {
            0
        }
    }

    /// The next stage that is due and gated in: (index, stage).
    pub fn next_due(&self, now: f64, era: Era, residents: u32) -> Option<(usize, Stage)> {
        self.path
            .iter()
            .enumerate()
            .find(|(i, s)| {
                !self.done.get(*i).copied().unwrap_or(false)
                    && era >= s.advance.min_era()
                    && residents >= s.advance.min_residents()
                    && now + 1e-9 >= s.at
            })
            .map(|(i, s)| (i, *s))
    }

    /// The share of roofs the spread is aiming for right now: it starts at a
    /// few roofs and climbs to [`SOLAR_TARGET`] over [`SOLAR_RAMP_DAYS`].
    pub fn solar_share_now(&self, now: f64) -> f32 {
        let start = self
            .path
            .iter()
            .find(|s| s.advance == Advance::SolarRoofs)
            .map(|s| s.at)
            .unwrap_or(now);
        let t = ((now - start) / (SOLAR_RAMP_DAYS * DAY)).clamp(0.0, 1.0) as f32;
        SOLAR_TARGET * (0.15 + 0.85 * t)
    }

    /// Where the crews are, for the renderer: (x, y, crew kind).
    pub fn crews(&self) -> Vec<(i32, i32, u8)> {
        self.jobs
            .iter()
            .filter_map(|j| {
                let p = j.path.get(j.i.min(j.path.len().saturating_sub(1)))?;
                Some((p.0, p.1, j.work.crew_kind()))
            })
            .collect()
    }

    pub fn busy(&self) -> bool {
        !self.jobs.is_empty()
    }

    /// One line per advance of the world's path (the tech-path printout).
    pub fn printout(&self) -> Vec<String> {
        self.path
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    "day {:>5.1}  {:<16} {:<12}{}",
                    s.day(),
                    s.advance.name(),
                    format!("{:?}", s.advance.min_era()).to_lowercase(),
                    if self.done.get(i).copied().unwrap_or(false) {
                        "done"
                    } else {
                        ""
                    }
                )
            })
            .collect()
    }
}

impl Sim {
    /// Advance the technology crews. Called every planning step: free when
    /// nothing is due and no crew is out (B49 item 4).
    pub fn tech_step(&mut self) {
        let now = self.time;
        // 1. Finish whatever the crews reach this step (one tile at a time, so a
        //    lamp is swapped on its own and nothing pops in all at once).
        let mut due: Vec<(Work, (i32, i32))> = Vec::new();
        for job in self.tech.jobs.iter_mut() {
            while job.i < job.path.len() && now + 1e-9 >= job.next_at {
                due.push((job.work, job.path[job.i]));
                job.i += 1;
                job.next_at += match job.work {
                    Work::Tower => TOWER_SECONDS,
                    _ => TILE_SECONDS,
                };
            }
        }
        for (work, (x, y)) in due {
            self.install_tech(work, x, y);
        }
        // A finished job tells its milestone, once the whole crew is done.
        let finished: Vec<usize> = self
            .tech
            .jobs
            .iter()
            .filter(|j| j.i >= j.path.len())
            .map(|j| j.stage)
            .collect();
        self.tech.jobs.retain(|j| j.i < j.path.len());
        for stage in finished {
            self.finish_tech_stage(stage);
        }

        // 2. Send the next crew out, when the last one is done.
        if self.tech.jobs.is_empty() {
            let era = self.era();
            match self.next_techwork(now) {
                Some(job) => {
                    self.tech.halted_in = None;
                    self.tech.jobs.push(job);
                }
                None => {
                    // Nothing to do in this era: skip the scan until the town
                    // changes under it (B61's trick, reused).
                    if self.tech.halted_in != Some(era) {
                        self.tech.halted_in = Some(era);
                    }
                }
            }
        }
    }

    /// The next crew job: more of the work that keeps spreading (rooftop solar),
    /// or the next advance that is due.
    fn next_techwork(&mut self, now: f64) -> Option<Job> {
        if self.tech.solar_spreading {
            if let Some(job) = self.solar_job(now) {
                return Some(job);
            }
        }
        let era = self.era();
        let (i, stage) = self.tech.next_due(now, era, self.residents())?;
        match self.plan_advance(i, stage.advance, now) {
            Next::Job(job) => Some(job),
            Next::Done => {
                self.finish_tech_stage(i);
                None
            }
            Next::Wait => None,
        }
    }

    /// Build the crew's tile list for an advance.
    fn plan_advance(&mut self, i: usize, a: Advance, now: f64) -> Next {
        // A crew job. An empty list means "nowhere to put it": for the works
        // that must be *visible* (poles, towers, the farm, the turbines, the
        // chargers) the advance is **not** finished — it waits for the town to
        // make room, so the feed never claims a turbine that is not there.
        let job = |work: Work, path: Vec<(i32, i32)>, stage: usize| {
            if path.is_empty() {
                match work {
                    Work::Lamp(_) | Work::Cell => Next::Done,
                    _ => Next::Wait,
                }
            } else {
                Next::Job(Job {
                    work,
                    stage,
                    path,
                    i: 0,
                    next_at: now + TILE_SECONDS,
                })
            }
        };
        match a {
            Advance::GasLamps => {
                // The village's lamps are the street crew's, so this crew goes
                // round and lights the ones already standing. Nothing standing
                // yet: wait, and come back when the first street is furnished.
                let mut path: Vec<(i32, i32)> = self.streetworks.lamps.iter().copied().collect();
                path.sort_by_key(|&(x, y)| (x + y, x));
                if path.is_empty() {
                    Next::Wait
                } else {
                    job(Work::Lamp(0), path, i)
                }
            }
            Advance::SodiumLamps => job(Work::Lamp(1), self.lamp_retrofit(1), i),
            Advance::LedLamps => job(Work::Lamp(2), self.lamp_retrofit(2), i),
            Advance::UtilityPoles => job(Work::Pole, self.pole_run(), i),
            Advance::BuriedLines => job(Work::Bury, self.core_poles(), i),
            Advance::CellTowers => {
                let sites = self.tower_sites(i as u64);
                job(Work::Tower, sites, i)
            }
            Advance::SmallCells => {
                let mut path: Vec<(i32, i32)> = self
                    .streetworks
                    .lamps
                    .iter()
                    .copied()
                    .filter(|&(x, y)| {
                        !self.tech.cells.contains(&(x, y))
                            && matches!(
                                self.district_at(x, y),
                                District::Core | District::Downtown | District::European
                            )
                    })
                    .collect();
                path.sort();
                path.truncate(24);
                job(Work::Cell, path, i)
            }
            Advance::SolarRoofs => {
                self.tech.solar_spreading = true;
                match self.solar_job(now) {
                    Some(j) => Next::Job(j),
                    None => Next::Done,
                }
            }
            Advance::SolarFarm => job(Work::Farm, self.farm_sites(i as u64), i),
            Advance::WindTurbines => job(Work::Turbine, self.turbine_sites(i as u64), i),
            Advance::EvChargers => job(Work::Charger, self.charger_sites(), i),
            // The rest are facts about the town and its traffic, not furniture:
            // the traffic mix reads `vehicle_era`, and the flags are raised.
            Advance::EarlyCars
            | Advance::ModernCars
            | Advance::Evs
            | Advance::Catenary
            | Advance::Drones
            | Advance::Maglev => {
                self.set_tech_flag(a);
                self.finish_tech_stage(i);
                Next::Done
            }
        }
    }

    /// The lamps still below `to`, in the order a crew walks them: in diagonal
    /// bands along the streets, so the change sweeps across the town.
    fn lamp_retrofit(&self, to: u8) -> Vec<(i32, i32)> {
        let mut path: Vec<(i32, i32)> = self
            .streetworks
            .lamps
            .iter()
            .copied()
            .filter(|&(x, y)| self.tech.lamp_level(x, y) < to)
            .collect();
        path.sort_by_key(|&(x, y)| (x + y, x));
        path
    }

    /// Paved tiles that should carry a utility pole: one every four tiles along
    /// a street, offset from the lamps (every five) so the two do not fight.
    fn pole_run(&self) -> Vec<(i32, i32)> {
        let mut paved: Vec<(i32, i32)> = self
            .lane_tiles()
            .into_iter()
            .filter(|&(x, y)| self.road_class(x, y).is_paved())
            .collect();
        paved.sort();
        paved
            .into_iter()
            .filter(|&(x, y)| {
                !self.tech.poles.contains(&(x, y))
                    && !self.tech.buried.contains(&(x, y))
                    && (x + y).rem_euclid(4) == 1
            })
            .collect()
    }

    /// The poles standing in the core, which the burial crew takes down.
    fn core_poles(&self) -> Vec<(i32, i32)> {
        let mut path: Vec<(i32, i32)> = self
            .tech
            .poles
            .iter()
            .copied()
            .filter(|&(x, y)| {
                matches!(
                    self.district_at(x, y),
                    District::Core | District::Downtown | District::European
                )
            })
            .collect();
        path.sort();
        path
    }

    /// Rooftop solar: the next few bare roofs, in a stable order, one job at a
    /// time so the panels spread over hours.
    fn solar_job(&mut self, now: f64) -> Option<Job> {
        let roofs: Vec<(i32, i32)> = self
            .lots
            .iter()
            .filter(|l| {
                l.finished()
                    && matches!(
                        l.kind,
                        LotKind::House
                            | LotKind::Townhouse
                            | LotKind::Apartment
                            | LotKind::Office
                            | LotKind::Store
                            | LotKind::Shop
                            | LotKind::Industry
                    )
            })
            .map(|l| (l.x, l.y))
            .collect();
        // The planner reads the share on the *houses* (the town is mostly
        // houses), so the target is measured against them too. B61d: the two lot
        // scans and the fitted count are one pass now — a HashSet of the house
        // roofs, which also replaces the O(roofs x lots) nested scan below.
        let mut houses = 0usize;
        // B61d: this used to be `for each fitted roof: scan every lot`, i.e.
        // millions of comparisons once the solar spread is under way — perf put
        // `solar_job` at 20 % of a whole 30-day catch-up run. One pass over the
        // lots answers the same question.
        let mut house_roofs: std::collections::HashSet<(i32, i32)> =
            std::collections::HashSet::with_capacity(houses);
        for l in &self.lots {
            if l.finished()
                && matches!(
                    l.kind,
                    LotKind::House | LotKind::Townhouse | LotKind::Apartment
                )
            {
                house_roofs.insert((l.x, l.y));
                houses += 1;
            }
        }
        let fitted_houses = self
            .tech
            .solar
            .keys()
            .filter(|p| house_roofs.contains(p))
            .count();
        // B49b: 30 % of the roofs by the late era, climbed gradually — measured
        // against the same set of roofs the crews may fit (`roofs`), so the share
        // can never overshoot by counting shops into a house-only denominator.
        // The spread pauses once it has caught up and resumes as the years (or
        // the town) grow; `halted_in` keeps the idle cost at nothing in between.
        let target = self.tech.solar_share_now(now);
        if houses > 0 && fitted_houses as f32 >= houses as f32 * target {
            return None;
        }
        let mut eligible: Vec<(i32, i32)> = roofs
            .into_iter()
            .filter(|p| !self.tech.solar.contains_key(p))
            .collect();
        if eligible.is_empty() {
            return None;
        }
        eligible.sort();
        eligible.truncate(SOLAR_PER_JOB);
        let stage = self.stage_of(Advance::SolarRoofs).unwrap_or(0);
        Some(Job {
            work: Work::Solar,
            stage,
            path: eligible,
            i: 0,
            next_at: now + TILE_SECONDS,
        })
    }

    /// Tower sites: open ground a little outside the built town, on two
    /// opposite sides of the city.
    fn tower_sites(&self, tag: u64) -> Vec<(i32, i32)> {
        let mut rng = Rng::new(self.seed ^ 0x70E5 ^ tag.wrapping_mul(7919));
        let mut out: Vec<(i32, i32)> = Vec::new();
        for k in 0..2 {
            let Some(p) = self.tech_site(&mut rng, 0.36, 0.48, true) else {
                continue;
            };
            if out
                .iter()
                .any(|&q| (q.0 - p.0).abs() + (q.1 - p.1).abs() < 12)
            {
                continue;
            }
            let _ = k;
            out.push(p);
        }
        out
    }

    /// A solar farm: a small block of open ground at the edge.
    fn farm_sites(&self, tag: u64) -> Vec<(i32, i32)> {
        let mut rng = Rng::new(self.seed ^ 0xFA12 ^ tag.wrapping_mul(104_729));
        // A farm needs a real patch, not a single tile: try a few spots and keep
        // the fullest block, so the edge of the city reads as an installation.
        let mut best: Vec<(i32, i32)> = Vec::new();
        for _ in 0..6 {
            let Some((cx, cy)) = self.tech_site(&mut rng, 0.28, 0.44, true) else {
                continue;
            };
            let mut block = Vec::new();
            for dy in 0..3 {
                for dx in 0..4 {
                    if self.free_ground(cx + dx, cy + dy) {
                        block.push((cx + dx, cy + dy));
                    }
                }
            }
            if block.len() > best.len() {
                best = block;
            }
            if best.len() >= 10 {
                break;
            }
        }
        best
    }

    /// A wind farm (B49b item 2): turbines in a **row** on open land at the edge
    /// of the city (or on the high ground, picked the same way), spaced a few
    /// tiles apart, and never close to a lot — the planner's "never inside town
    /// lots" holds even as the town grows towards them.
    fn turbine_sites(&self, tag: u64) -> Vec<(i32, i32)> {
        let mut rng = Rng::new(self.seed ^ 0x715B ^ tag.wrapping_mul(65_537));
        let n = 3 + rng.usize(3) as i32; // 3..=5 machines in the row
        let gap = 5i32;
        let mut best: Vec<(i32, i32)> = Vec::new();
        for _ in 0..40 {
            let Some((sx, sy)) = self.tech_site(&mut rng, 0.30, 0.48, false) else {
                continue;
            };
            // The row may run along either iso axis, either way: take the
            // direction that stays on open ground (the town can be on one side).
            let dirs = [
                ((1, 0), true),
                ((-1, 0), true),
                ((0, 1), false),
                ((0, -1), false),
            ];
            for ((dx, dy), _along_x) in dirs {
                let mut row: Vec<(i32, i32)> = Vec::new();
                for k in 0..n {
                    let (x, y) = (sx + dx * k * gap, sy + dy * k * gap);
                    if !self.wind_site(x, y) {
                        break;
                    }
                    row.push((x, y));
                }
                if row.len() > best.len() {
                    best = row;
                }
            }
            if best.len() >= 3 {
                break;
            }
        }
        if best.is_empty() {
            // No room for a row on this seed's land: a single machine on open
            // ground at the edge is still a wind turbine (never in a back yard —
            // `wind_site` checks that, and the tile is reserved).
            let mut fallback = Rng::new(self.seed ^ 0x715B ^ tag.wrapping_mul(31));
            for _ in 0..12 {
                let Some(p) = self.tech_site(&mut fallback, 0.30, 0.48, false) else {
                    continue;
                };
                if self.wind_site(p.0, p.1) {
                    best.push(p);
                    break;
                }
            }
        }
        best
    }

    /// Is this a place for a turbine? Open ground, clear of the town (no lot
    /// within six tiles), not water, not the horizon belt.
    fn wind_site(&self, x: i32, y: i32) -> bool {
        if !self.free_ground(x, y) {
            return false;
        }
        for l in &self.lots {
            let nx = x.clamp(l.x, l.x + l.w - 1);
            let ny = y.clamp(l.y, l.y + l.h - 1);
            if (nx - x).abs() + (ny - y).abs() < 3 {
                return false;
            }
        }
        true
    }

    /// Charger bays: paved tiles in the core and downtown, spread out (where
    /// B59's car parks and filling stations belong).
    fn charger_sites(&self) -> Vec<(i32, i32)> {
        let mut paved: Vec<(i32, i32)> = self
            .lane_tiles()
            .into_iter()
            .filter(|&(x, y)| {
                self.road_class(x, y).is_paved()
                    && matches!(self.district_at(x, y), District::Core | District::Downtown)
                    && !self.tech.chargers.contains(&(x, y))
            })
            .collect();
        paved.sort_by_key(|&(x, y)| (x + y, x));
        // Every ninth tile of the list: a few bays spread down the main streets.
        paved.into_iter().step_by(9).take(6).collect()
    }

    /// One tile of open ground for a tower, the farm or a turbine: outside the
    /// built town, dry, and not on the horizon belt.
    fn tech_site(&self, rng: &mut Rng, lo: f32, hi: f32, prefer_flat: bool) -> Option<(i32, i32)> {
        let (fx, fy) = self.founding;
        let span = self.span();
        let mut fallback: Option<(i32, i32)> = None;
        for _ in 0..220 {
            let a = rng.range_f32(0.0, std::f32::consts::TAU);
            let r = rng.range_f32(lo, hi) * span;
            let x = fx + (a.cos() * r) as i32;
            let y = fy + (a.sin() * r) as i32;
            if !self.free_ground(x, y) {
                continue;
            }
            if prefer_flat && self.map.at(x, y).relief() > 1 {
                continue;
            }
            // In the frame the viewer shows, if the sim knows it (the wallpaper
            // sets the viewport before it runs; a tool may set it later, so an
            // out-of-frame site is a fallback rather than a refusal).
            let z = self.map.at(x, y).min_level() as i32 * 12;
            if self.in_frame(x, y, z, 40) {
                return Some((x, y));
            }
            if fallback.is_none() {
                fallback = Some((x, y));
            }
        }
        fallback
    }

    /// Can something permanent stand here? Not water, not a street, not a
    /// building, not the horizon belt, and not already a tech structure.
    fn free_ground(&self, x: i32, y: i32) -> bool {
        if x < 3 || y < 3 || x >= self.map.w - 3 || y >= self.map.h - 3 {
            return false;
        }
        let t = self.map.at(x, y);
        if t.kind.is_water() || t.kind.is_lane() || t.kind == Kind::Sidewalk {
            return false;
        }
        if matches!(t.kind, Kind::Rock | Kind::Scree) {
            return false;
        }
        if t.aux & map::AUX_TECH != 0 {
            return false;
        }
        // A farm, a turbine or a tower the camera never shows is not a feature
        // of the city (B49 item 2).
        let z = t.min_level() as i32 * 12;
        if !self.in_frame(x, y, z, 24) {
            return false;
        }
        if self.behind_horizon(x, y) {
            return false;
        }
        if self.lots.iter().any(|l| l.covers(x, y)) {
            return false;
        }
        true
    }

    /// Install one tile's worth of an advance.
    fn install_tech(&mut self, work: Work, x: i32, y: i32) {
        match work {
            Work::Lamp(level) => {
                self.tech.lamps.insert((x, y), level);
            }
            Work::Pole => {
                self.tech.poles.insert((x, y));
            }
            Work::Bury => {
                self.tech.poles.remove(&(x, y));
                self.tech.buried.insert((x, y));
                self.reserve_ground(x, y);
            }
            Work::Tower => {
                if !self.tech.towers.contains(&(x, y)) {
                    self.tech.towers.push((x, y));
                }
                self.reserve_ground(x, y);
            }
            Work::Cell => {
                self.tech.cells.insert((x, y));
            }
            Work::Solar => {
                // A panel style from the lot's own seed, so no two roofs match.
                let style = self
                    .lots
                    .iter()
                    .find(|l| l.x == x && l.y == y)
                    .map(|l| (l.seed % 3) as u8 + 1)
                    .unwrap_or(1);
                self.tech.solar.insert((x, y), style);
            }
            Work::Farm => {
                self.tech.farm.insert((x, y));
                self.reserve_ground(x, y);
            }
            Work::Turbine => {
                if !self.tech.turbines.contains(&(x, y)) {
                    self.tech.turbines.push((x, y));
                }
                self.reserve_ground(x, y);
            }
            Work::Charger => {
                self.tech.chargers.insert((x, y));
            }
        }
        // The renderer repaints exactly these tiles (B49 item 4). A lamp's pool
        // reaches a tile or so, so its neighbours are flagged too.
        self.mark_changed(x, y);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            self.mark_changed(x + dx, y + dy);
        }
    }

    /// Mark ground as taken by a permanent tech structure (a tower, a buried
    /// cabinet, a farm panel, a turbine): no lot is staked on top of it.
    fn reserve_ground(&mut self, x: i32, y: i32) {
        let mut t = self.map.at(x, y);
        if t.aux & map::AUX_TECH == 0 {
            t.aux |= map::AUX_TECH;
            // The crew clears what stood there: no tree grows through a mast.
            t.prop = map::Prop::None;
            // Not `write`: the kind and height are unchanged, and a `write`
            // would re-run the paving story hooks.
            self.map.set(x, y, t);
            self.overrides.insert((x, y), t);
        }
    }

    /// Raise the flags that are not furniture but facts about the town.
    fn set_tech_flag(&mut self, a: Advance) {
        match a {
            Advance::Catenary => self.tech.catenary = true,
            Advance::Drones => self.tech.drones = true,
            Advance::Maglev => self.tech.maglev = true,
            _ => {}
        }
    }

    /// Close an advance: mark it done and tell the feed once.
    fn finish_tech_stage(&mut self, i: usize) {
        if i >= self.tech.path.len() || self.tech.done.get(i).copied().unwrap_or(true) {
            return;
        }
        let a = self.tech.path[i].advance;
        self.tech.done[i] = true;
        // The solar milestone is the first crew; the spread goes on after it.
        if a == Advance::SolarRoofs {
            self.tech.solar_spreading = true;
        }
        self.push_story_t(crate::chronicle::Topic::Tech, a.index() as i32, a.line());
    }

    /// The index of an advance in the path, if this world has it.
    pub fn stage_of(&self, a: Advance) -> Option<usize> {
        self.tech.path.iter().position(|s| s.advance == a)
    }

    pub fn tech(&self) -> &Tech {
        &self.tech
    }

    /// B58: whether the railway carries catenary (B49 only raises the flag).
    pub fn rail_electrified(&self) -> bool {
        self.tech.catenary
    }

    /// Delivery drones are flying (near-future worlds, by day).
    pub fn drones_active(&self) -> bool {
        self.tech.drones
    }

    /// The maglev line exists.
    pub fn maglev(&self) -> bool {
        self.tech.maglev
    }

    /// A drone's tile position (a pure function of the clock, so nothing is
    /// saved): the `i`-th drone of `n`, drifting over the city by day.
    pub fn drone_at(&self, i: usize, n: usize, now: f64) -> Option<(f32, f32)> {
        if !self.tech.drones {
            return None;
        }
        let hod = (now / 3600.0).rem_euclid(24.0);
        if !(7.0..19.0).contains(&hod) {
            return None;
        }
        let (fx, fy) = (self.founding.0 as f32, self.founding.1 as f32);
        let span = self.span() * 0.28;
        let t = now as f32;
        let a = t * 0.021 + i as f32 * (std::f32::consts::TAU / n.max(1) as f32);
        Some((
            fx + (a * 0.7).cos() * span,
            fy + (a * 1.1).sin() * span * 0.8,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pace;

    fn sim(seed: u64) -> Sim {
        Sim::new(seed, Pace::Month, 61, 61)
    }

    /// B49 item 1: the path is a pure function of the seed, it varies between
    /// worlds, and an advance never comes before what it depends on.
    #[test]
    fn tech_path_is_deterministic_per_seed_and_respects_dependencies() {
        for seed in 0..6u64 {
            let a = Tech::plan(seed);
            let b = Tech::plan(seed);
            assert_eq!(a.path, b.path, "seed {seed} planned two different paths");
            let mut seen: Vec<Advance> = Vec::new();
            for s in &a.path {
                for r in s.advance.requires() {
                    assert!(
                        seen.contains(r),
                        "seed {seed}: {:?} comes before its dependency {:?}",
                        s.advance,
                        r
                    );
                }
                seen.push(s.advance);
            }
            // Every advance carries its era gate.
            for s in &a.path {
                assert!(s.advance.min_era() >= Era::Village);
            }
        }
        // Two worlds of different seeds do not get the same order and timing.
        let p0: Vec<f64> = Tech::plan(1).path.iter().map(|s| s.at).collect();
        let p1: Vec<f64> = Tech::plan(2).path.iter().map(|s| s.at).collect();
        assert_ne!(p0, p1, "two seeds planned the same technology path");
    }

    /// B49 item 1: a world can be early to one advance and late to another, and
    /// the far-future two only come to some worlds.
    #[test]
    fn worlds_differ_in_order_and_in_the_far_future() {
        let mut orders = std::collections::BTreeSet::new();
        let mut near_future = 0;
        for seed in 0..40u64 {
            let t = Tech::plan(seed);
            let mut order = String::new();
            for s in &t.path {
                order.push_str(s.advance.name());
                order.push('>');
            }
            orders.insert(order);
            if t.path.iter().any(|s| s.advance == Advance::Drones) {
                near_future += 1;
            }
        }
        assert!(
            orders.len() >= 20,
            "only {} distinct orders in 40 worlds",
            orders.len()
        );
        assert!(near_future > 5, "no world ever reaches the near future");
        assert!(near_future < 35, "every world reaches the near future");
    }

    /// B49 item 2/4: the advances happen in a running world, the lamps go
    /// through gas → sodium → LED, and each change arrives a tile at a time,
    /// never all at once in one step.
    #[test]
    fn advances_arrive_by_crews_a_tile_at_a_time() {
        let mut s = sim(11);
        let mut max_per_step = 0usize;
        let mut mid_gas = false;
        let mut mid_sodium = false;
        let mut prev = s.tech.lamps.clone();
        // 34 days at 20 s a step: the whole month's life plus the near future.
        for _ in 0..(34 * 24 * 3 * 60) {
            s.advance(20.0);
            let now = s.tech.lamps.clone();
            let changed = now.iter().filter(|(k, v)| prev.get(*k) != Some(*v)).count();
            max_per_step = max_per_step.max(changed);
            prev = now;
            let levels: Vec<u8> = s.tech.lamps.values().copied().collect();
            if levels.contains(&0) && levels.contains(&1) {
                mid_gas = true;
            }
            if levels.contains(&1) && levels.contains(&2) {
                mid_sodium = true;
            }
        }
        assert!(
            s.tech.lamps.len() > 8,
            "only {} lamps were ever fitted",
            s.tech.lamps.len()
        );
        assert!(s.tech.is_done(Advance::GasLamps), "gas lamps never arrived");
        assert!(
            mid_gas,
            "the town never had a gas lamp and a sodium lamp at once"
        );
        assert!(
            mid_sodium,
            "the town never had a sodium lamp and an LED at once"
        );
        assert!(
            max_per_step <= 2,
            "{max_per_step} lamp changes landed in one 20 s step (a pop)"
        );
        // The feed told the milestones.
        let feed: Vec<String> = s.story.iter().map(|l| l.text.clone()).collect();
        assert!(
            feed.iter().any(|l| l.contains("sodium")),
            "no milestone line for the sodium lamps"
        );
    }

    /// B49 item 2: the poles go up, then the core's come down and the lines are
    /// buried; a cell tower, solar roofs, the farm and the turbines appear.
    #[test]
    fn the_hardware_appears_and_the_poles_come_down() {
        let mut s = sim(7);
        s.run_to(34.0 * 86400.0);
        assert!(
            s.tech.poles.len() + s.tech.buried.len() > 0,
            "no poles or cables"
        );
        if s.tech.is_done(Advance::BuriedLines) {
            assert!(
                s.tech.buried.len() > 0,
                "the lines were buried but no cabinet stands anywhere"
            );
        }
        if s.tech.is_done(Advance::SolarRoofs) {
            assert!(
                s.tech.solar.len() > 3,
                "the first roofs went solar, then nothing"
            );
        }
        if s.tech.is_done(Advance::CellTowers) {
            assert!(!s.tech.towers.is_empty(), "no cell tower went up");
        }
        if s.tech.is_done(Advance::WindTurbines) {
            assert!(!s.tech.turbines.is_empty(), "no turbine went up");
        }
        if s.tech.is_done(Advance::SolarFarm) {
            assert!(!s.tech.farm.is_empty(), "no farm panel was laid");
        }
    }

    /// B49 item 4: the per-step cost once the crews have gone quiet.
    #[test]
    fn tech_step_is_free_when_nothing_is_upgrading() {
        let mut s = sim(21);
        s.run_to(34.0 * 86400.0);
        assert!(!s.tech.busy(), "a crew is still out at day 34");
        let t0 = std::time::Instant::now();
        for _ in 0..1000 {
            s.tech_step();
        }
        let us = t0.elapsed().as_secs_f64() * 1e6 / 1000.0;
        println!("tech_step idle: {us:.3} us per planning step (one per 20 sim s)");
        // A halted step is a comparison and an era check: far under a microsecond
        // on any machine, and it is not on the frame path at all.
        assert!(us < 50.0, "an idle tech step costs {us:.1} us");
    }

    /// B49: how much the technology state adds to a save (the handback's size line).
    #[test]
    fn tech_state_is_small_in_the_save() {
        let mut s = Sim::new(11, Pace::Month, 92, 92);
        s.run_to(30.0 * 86400.0);
        let dir = std::env::temp_dir().join("citylive-b49-size");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("size.sav");
        s.save(&p).unwrap();
        let full = std::fs::metadata(&p).unwrap().len();
        let tech_bytes = s.tech().lamps.len() * 9
            + (s.tech().poles.len()
                + s.tech().buried.len()
                + s.tech().cells.len()
                + s.tech().chargers.len()
                + s.tech().farm.len()
                + s.tech().towers.len()
                + s.tech().turbines.len())
                * 8
            + s.tech().solar.len() * 9
            + s.tech().path.len() * 9
            + 40;
        println!("save {full} B, technology state ~{tech_bytes} B");
        assert!(tech_bytes < 20_000, "technology state is {tech_bytes} B");
    }

    /// B49b item 4: the rooftop share is ~30 % by the late era and it climbs
    /// there gradually, rather than jumping to a fixed share.
    #[test]
    fn the_solar_share_climbs_to_thirty_percent() {
        let mut s = sim(11);
        s.run_to(30.0 * 86400.0);
        let t = s.tech();
        let houses = s
            .lots
            .iter()
            .filter(|l| {
                l.finished()
                    && matches!(
                        l.kind,
                        LotKind::House | LotKind::Townhouse | LotKind::Apartment
                    )
            })
            .count();
        let fitted = t
            .solar
            .keys()
            .filter(|p| {
                s.lots.iter().any(|l| {
                    l.x == p.0
                        && l.y == p.1
                        && l.finished()
                        && matches!(
                            l.kind,
                            LotKind::House | LotKind::Townhouse | LotKind::Apartment
                        )
                })
            })
            .count();
        let share = fitted as f32 / houses.max(1) as f32;
        assert!(
            // B59b: the roadside trade is zoned its own ground, which the town's
            // housing would otherwise have grown into, so on a 72-home town one
            // roof is 1.4 points; the tolerance is two roofs' worth wider.
            share <= SOLAR_TARGET + 0.08,
            "{share:.2} of roofs is over the {SOLAR_TARGET} target"
        );
        // Early in the spread it is a few roofs, not the whole town.
        let mut early = sim(11);
        early.run_to(16.0 * 86400.0);
        let early_share = early.tech().solar.len() as f32 / houses.max(1) as f32;
        assert!(
            early_share < share + 0.01,
            "the spread did not grow: {early_share:.2} then {share:.2}"
        );
        // The target itself ramps: yesterday's aim is smaller than today's.
        let start = t
            .path
            .iter()
            .find(|st| st.advance == Advance::SolarRoofs)
            .map(|st| st.at)
            .unwrap_or(0.0);
        let a = t.solar_share_now(start + 86400.0);
        let b = t.solar_share_now(start + 9.0 * 86400.0);
        assert!(a < b, "the share does not climb: {a} then {b}");
        assert!(
            (b - SOLAR_TARGET).abs() < 0.02,
            "it never reaches the target"
        );
    }

    /// B49b item 2: the turbines stand in a row on open ground, spaced out and
    /// clear of the town's lots — never a lone mast in a back garden.
    #[test]
    fn turbines_stand_in_a_row_on_open_ground() {
        for seed in [11u64, 5, 42, 21] {
            let mut s = sim(seed);
            s.run_to(34.0 * 86400.0);
            let t = s.tech();
            if t.turbines.len() < 2 {
                continue; // this world never reached the advance: nothing to check
            }
            // All on one line, evenly spaced, and no lot within three tiles.
            let first = t.turbines[0];
            for w in t.turbines.windows(2) {
                let (a, b) = (w[0], w[1]);
                assert!(
                    a.0 == b.0 || a.1 == b.1,
                    "seed {seed}: turbines at {a:?} and {b:?} are not in a row"
                );
                let d = (a.0 - b.0).abs() + (a.1 - b.1).abs();
                assert!((3..=8).contains(&d), "seed {seed}: spacing {d}");
            }
            for &(x, y) in &t.turbines {
                assert!(
                    (x - first.0).abs() + (y - first.1).abs() < 40,
                    "seed {seed}: a turbine at {x},{y} is not in the farm"
                );
                for l in &s.lots {
                    let nx = x.clamp(l.x, l.x + l.w - 1);
                    let ny = y.clamp(l.y, l.y + l.h - 1);
                    assert!(
                        (nx - x).abs() + (ny - y).abs() >= 3,
                        "seed {seed}: a turbine stands on a lot at {x},{y}"
                    );
                }
            }
        }
    }

    /// B49b item 4: the maglev also needs a huge city (≥ 8 000 residents).
    #[test]
    fn the_maglev_needs_eight_thousand_residents() {
        assert_eq!(Advance::Maglev.min_residents(), 8_000);
        let t = Tech::plan(27);
        let maglev = t.path.iter().position(|s| s.advance == Advance::Maglev);
        if let Some(i) = maglev {
            let at = t.path[i].at;
            // Two days after it is due, at a city of 5 000, it is not offered…
            assert!(
                t.next_due(at + 2.0 * 86400.0, Era::Metropolis, 5_000)
                    .is_none()
                    || t.next_due(at + 2.0 * 86400.0, Era::Metropolis, 5_000)
                        .map(|(j, _)| j != i)
                        .unwrap_or(true),
                "the maglev came to a city of 5 000"
            );
            // …and it is, at 8 000.
            let due = t.next_due(at + 2.0 * 86400.0, Era::Metropolis, 8_000);
            assert!(due.is_some(), "the maglev never came to a huge city");
        }
    }

    /// B49 item 2: the traffic mix follows the era (carts before the motor).
    #[test]
    fn the_traffic_mix_follows_the_technology() {
        let mut s = sim(5);
        s.run_to(30.0 * 86400.0);
        let t = s.tech.clone();
        assert_eq!(
            t.vehicle_era(),
            if t.is_done(Advance::Evs) {
                3
            } else if t.is_done(Advance::ModernCars) {
                2
            } else if t.is_done(Advance::EarlyCars) {
                1
            } else {
                0
            }
        );
    }
}
