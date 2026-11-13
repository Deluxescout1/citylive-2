//! CityLive Iso growth simulation (M3, v1: settlement → village).
//!
//! The sim is a **pure function of `(seed, sim_time)`**: every random choice
//! comes from a stream seeded by `(seed, step_index)`, every decision runs in a
//! fixed order on fixed-size planning steps, and no iteration order depends on
//! a hash map. That is what makes catch-up (replaying a week of missed time in
//! bulk) produce exactly the same city as running live.
//!
//! Time model: `sim_time` is seconds. A [`Pace`] multiplies real time (a month
//! to a full city is 1×, a week 4×, a day 30×). Growth decisions happen once
//! per [`DECISION_PERIOD`] of sim time; construction is a set of timed events.

pub mod civics;
pub mod audit;
pub mod parking;
pub mod streetlife;
pub mod week;
pub mod ending;
mod ending_fx;
pub mod grid;
pub mod landmarks;
pub mod life;
#[cfg(test)]
mod life_tests;
#[cfg(test)]
mod ending_tests;
#[cfg(test)]
mod week_tests;
#[cfg(test)]
mod chronicle_tests;
pub mod people;
pub mod winlights;
pub mod roads;
pub mod settlers;
pub mod streetworks;
pub mod story;
pub mod chronicle;
pub mod timelapse;
pub mod save;
pub mod ageing;
pub mod rail;
pub mod isles;
pub mod carculture;
pub mod tech;
pub mod traffic;
pub mod transit;
mod blocks;
mod farms;
pub mod reach;
mod forever;

use blocks::Blocks;
use traffic::Traffic;

use std::collections::{BTreeMap, BTreeSet};

use map::rng::Rng;
use map::{Kind, Tile, TileMap, generate};

/// Sim seconds per planning step.
pub const PLANNING_STEP: f64 = 20.0;
/// Sim seconds between growth decisions (one per sim hour).
pub const DECISION_PERIOD: f64 = 3600.0;

/// B66: how far a home may stand from the nearest street, in tiles, when its
/// only frontage is a worn path. This is `nearest_lane`'s own radius (9): the
/// router looks no further than that, so a house beyond it is a house no trip
/// can leave (islands seed 3 had ten villagers nobody could route, on sandbar
/// houses twenty tiles out).
pub const PLOT_LANE_REACH: i32 = 9;
/// Sim seconds a house takes, staked to finished (D37: 10-30 min at month pace).
pub const HOUSE_SECONDS: f64 = 1200.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pace {
    Day,
    Week,
    Month,
}

impl Pace {
    /// Sim seconds per real second.
    pub fn scale(self) -> f64 {
        match self {
            Pace::Day => 30.0,
            Pace::Week => 4.0,
            Pace::Month => 1.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Pace::Day => "day",
            Pace::Week => "week",
            Pace::Month => "month",
        }
    }

    pub fn from_name(s: &str) -> Option<Pace> {
        match s {
            "day" => Some(Pace::Day),
            "week" => Some(Pace::Week),
            "month" => Some(Pace::Month),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Era {
    Settlement,
    Village,
    Town,
    City,
    Metropolis,
}

impl Era {
    pub fn name(self) -> &'static str {
        match self {
            Era::Settlement => "settlement",
            Era::Village => "village",
            Era::Town => "town",
            Era::City => "city",
            Era::Metropolis => "metropolis",
        }
    }

    /// The era as a number (0 settlement … 4 metropolis) for callers that keep no
    /// dependency on this enum (the renderer's nightlife gate, B73r2).
    pub fn index(self) -> u8 {
        match self {
            Era::Settlement => 0,
            Era::Village => 1,
            Era::Town => 2,
            Era::City => 3,
            Era::Metropolis => 4,
        }
    }

    /// Population threshold (people) that starts the era — tuneable; the day
    /// targets in the design note are the check (D47).
    pub fn from_population(pop: u32) -> Era {
        if pop >= 4_000 {
            Era::Metropolis
        } else if pop >= 1_200 {
            Era::City
        } else if pop >= 140 {
            Era::Town
        } else if pop >= 40 {
            Era::Village
        } else {
            Era::Settlement
        }
    }
}

/// The style districts (D46): soft regions the city grows in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum District {
    Core,
    European,
    Suburb,
    Downtown,
    HighEnd,
    Industrial,
}

impl District {
    pub fn name(self) -> &'static str {
        match self {
            District::Core => "core",
            District::European => "european",
            District::Suburb => "suburb",
            District::Downtown => "downtown",
            District::HighEnd => "high-end",
            District::Industrial => "industrial",
        }
    }

    /// Style id handed to the renderer (B27's kit will read the same value).
    pub fn style(self) -> u8 {
        self as u8
    }

    /// How much this district wants density (added to the land value when
    /// choosing what to upgrade).
    pub fn density_bonus(self) -> f32 {
        match self {
            District::Downtown => 40.0,
            District::Core | District::European => 18.0,
            District::Suburb => 4.0,
            District::HighEnd => 0.0,
            District::Industrial => 2.0,
        }
    }
}

fn next_name(k: LotKind) -> &'static str {
    match k {
        LotKind::House => "house",
        LotKind::Church => "church",
        LotKind::Store => "store",
        LotKind::Shop => "shop",
        LotKind::Barn => "barn",
        LotKind::Farmhouse => "farmhouse",
        LotKind::Townhouse => "townhouse",
        LotKind::Apartment => "apartment",
        LotKind::Office => "office",
        LotKind::Tower => "tower",
        LotKind::Industry => "works",
        LotKind::Park => "park",
        LotKind::Cemetery => "cemetery",
        LotKind::Hall => "town hall",
        LotKind::Landmark(k) => k.name(),
        LotKind::LogCabin => "log cabin",
        LotKind::CarPark => "car park",
        LotKind::CarParkDeck => "car park",
        LotKind::GasStation => "filling station",
        LotKind::ChargingHub => "charging hub",
        LotKind::DriveThrough => "drive-through",
        LotKind::CarWash => "car wash",
        LotKind::Dealership => "dealership",
        LotKind::Motel => "motel",
    }
}

/// What stands on a plot (D6/D46/D54). Every building is **data**: the kind
/// carries its footprint, storeys and the district gives it a style, so the art
/// kit can be replaced without touching the sim.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LotKind {
    House,
    Church,
    Store,
    Shop,
    Barn,
    /// B80: the farmer's house at a farmstead. It houses nobody for the eras (the
    /// town's population counts homes in the town), and it is never staked by
    /// growth: only the countryside seeding places one beside its barn.
    Farmhouse,
    /// Two-three storey row of joined houses (the densification of a cottage).
    Townhouse,
    /// 4-8 storey apartments.
    Apartment,
    /// Low-rise offices on the main street.
    Office,
    /// Downtown tower: the tallest thing in the city.
    Tower,
    /// Mill/works by the river or the edge.
    Industry,
    /// A reserved green: no building, keeps the land open.
    Park,
    /// The town hall on its square (D24, B26b): the civic heart of the town.
    Hall,
    /// A landmark (B35): placed once per city, on a big multi-tile plot.
    Landmark(landmarks::LandmarkKind),
    /// The graveyard (B48): a small walled lawn with stones, at the edge of
    /// town, where the funerals are held.
    Cemetery,
    /// The first settlers' log cabin (B39/D62): built from felled timber.
    LogCabin,
    /// A surface car park with marked bays (B59).
    CarPark,
    /// A multi-storey car park downtown (B59).
    CarParkDeck,
    /// A filling station: canopy, pumps, kiosk (B59).
    GasStation,
    /// An EV charging hub, once the world has electric cars (B49/B59).
    ChargingHub,
    /// A burger drive-through with its queue lane (B59).
    DriveThrough,
    /// A car wash: in dirty, out shiny (B59).
    CarWash,
    /// A dealership: rows of cars and bunting (B59).
    Dealership,
    /// A roadside motel: rooms and the cars outside them (B59).
    Motel,
}

impl LotKind {
    /// Kinds that stand on the street as a continuous wall (B26b item 4);
    /// they do not need the village's yard-between-houses rule (D41).
    pub fn urban(self) -> bool {
        matches!(
            self,
            LotKind::Townhouse
                | LotKind::Apartment
                | LotKind::Office
                | LotKind::Tower
                | LotKind::Shop
                | LotKind::Store
                | LotKind::Industry
                | LotKind::Hall
                | LotKind::Park
                | LotKind::Cemetery
                | LotKind::Landmark(_)
                | LotKind::LogCabin
                | LotKind::CarPark
                | LotKind::CarParkDeck
                | LotKind::GasStation
                | LotKind::ChargingHub
                | LotKind::DriveThrough
                | LotKind::CarWash
                | LotKind::Dealership
                | LotKind::Motel
        )
    }

    pub fn is_civic(self) -> bool {
        matches!(
            self,
            LotKind::Church
                | LotKind::Store
                | LotKind::Shop
                | LotKind::Hall
                | LotKind::Cemetery
                | LotKind::Landmark(_)
        )
    }

    /// Storeys this kind stands at, varied by `seed` in `storeys_for`.
    pub fn base_storeys(self) -> u8 {
        match self {
            LotKind::House => 2,
            LotKind::Barn => 1,
            LotKind::Farmhouse => 2,
            LotKind::Church => 3,
            LotKind::Store | LotKind::Shop => 2,
            LotKind::Townhouse => 3,
            LotKind::Apartment => 6,
            LotKind::Office => 4,
            LotKind::Tower => 12,
            LotKind::Industry => 2,
            LotKind::Park | LotKind::Cemetery => 0,
            LotKind::Hall => 4,
            LotKind::Landmark(k) => k.storeys(),
            LotKind::LogCabin => 1,
            // A car park has no building; the deck is the structure itself.
            LotKind::CarPark => 0,
            LotKind::CarParkDeck => 3,
            LotKind::GasStation | LotKind::DriveThrough | LotKind::CarWash => 1,
            // A kiosk block under the canopy: one storey reads best.
            LotKind::ChargingHub => 0,
            LotKind::Dealership | LotKind::Motel => 1,
        }
    }

    /// How many people the finished building holds.
    pub fn occupants(self) -> u32 {
        match self {
            LotKind::House => 4,
            LotKind::Barn => 1,
            LotKind::Farmhouse => 0,
            LotKind::Church => 0,
            LotKind::Store | LotKind::Shop => 4,
            LotKind::Townhouse => 10,
            LotKind::Apartment => 44,
            LotKind::Office => 30,
            LotKind::Tower => 160,
            LotKind::Industry => 20,
            LotKind::Park | LotKind::Cemetery => 0,
            LotKind::Hall => 0,
            LotKind::Landmark(_) => 0,
            LotKind::LogCabin => 4,
            // The people who work there, not residents: small numbers.
            LotKind::CarPark | LotKind::CarParkDeck => 0,
            LotKind::GasStation => 6,
            LotKind::ChargingHub => 2,
            LotKind::DriveThrough => 9,
            LotKind::CarWash => 3,
            LotKind::Dealership => 7,
            LotKind::Motel => 12,
        }
    }
}

/// Construction stages, in order. Each is visible (D33).
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Stage {
    Staked,
    Graded,
    Foundation,
    Framing,
    Scaffold,
    Roofed,
    Finished,
}

impl Stage {
    pub const ALL: [Stage; 7] = [
        Stage::Staked,
        Stage::Graded,
        Stage::Foundation,
        Stage::Framing,
        Stage::Scaffold,
        Stage::Roofed,
        Stage::Finished,
    ];

    pub fn next(self) -> Option<Stage> {
        match self {
            Stage::Staked => Some(Stage::Graded),
            Stage::Graded => Some(Stage::Foundation),
            Stage::Foundation => Some(Stage::Framing),
            Stage::Framing => Some(Stage::Scaffold),
            Stage::Scaffold => Some(Stage::Roofed),
            Stage::Roofed => Some(Stage::Finished),
            Stage::Finished => None,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// A stable small code for a lot kind (hashing and the save format).
pub(crate) fn lot_kind_code(k: LotKind) -> u8 {
    match k {
        LotKind::House => 0,
        LotKind::Church => 1,
        LotKind::Store => 2,
        LotKind::Shop => 3,
        LotKind::Barn => 4,
        LotKind::Farmhouse => 50,
        LotKind::Townhouse => 5,
        LotKind::Apartment => 6,
        LotKind::Office => 7,
        LotKind::Tower => 8,
        LotKind::Industry => 9,
        LotKind::Park => 10,
        LotKind::Hall => 11,
        LotKind::Landmark(k) => 12 + k as u8,
        LotKind::LogCabin => 26,
        LotKind::Cemetery => 27,
        LotKind::CarPark => 29,
        LotKind::CarParkDeck => 30,
        LotKind::GasStation => 31,
        LotKind::ChargingHub => 32,
        LotKind::DriveThrough => 33,
        LotKind::CarWash => 34,
        LotKind::Dealership => 35,
        LotKind::Motel => 36,
    }
}

/// A plot: the graded pad and the building rising on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Lot {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub level: u8,
    pub kind: LotKind,
    /// Storeys above ground (1 for a barn, 12+ for a tower).
    pub storeys: u8,
    /// District style id (see `District::style`).
    pub style: u8,
    /// Sorted colour/style choice, so the renderer is data-driven (M4 replaces it).
    pub seed: u32,
    /// Which side the road is on: 0 = +x, 1 = +y, 2 = -x, 3 = -y.
    pub front: u8,
    pub stage: Stage,
    /// Sim time at which the current stage completes.
    pub stage_end: f64,
    pub started: f64,
}

impl Lot {
    /// People in this building, following its height (B26d): the old fixed
    /// table meant a downtown that doubled in height added nobody.
    pub fn occupants_now(&self) -> u32 {
        let per_storey = match self.kind {
            LotKind::House => return 4,
            LotKind::Barn => return 1,
            LotKind::Farmhouse => return 0,
            LotKind::Church
            | LotKind::Park
            | LotKind::Cemetery
            | LotKind::Hall
            | LotKind::Landmark(_)
            | LotKind::CarPark
            | LotKind::CarParkDeck
            | LotKind::ChargingHub => return 0,
            LotKind::LogCabin => return 4,
            LotKind::GasStation => 2,
            LotKind::DriveThrough => 3,
            LotKind::CarWash => 1,
            LotKind::Dealership => 2,
            LotKind::Motel => 4,
            LotKind::Townhouse | LotKind::Shop | LotKind::Store => 2,
            LotKind::Apartment => 3,
            LotKind::Office => 4,
            LotKind::Tower => 5,
            LotKind::Industry => 6,
        };
        per_storey * self.storeys.max(1) as u32
    }

    pub fn finished(&self) -> bool {
        self.stage == Stage::Finished
    }

    pub fn covers(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// A ploughed field on the outskirts.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Something the UI can report ("lane laid", "church started", ...).
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub time: f64,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub decisions: u64,
    pub lanes: u32,
    pub lane_tiles: u32,
    pub upgraded: u32,
    pub bridges: u32,
    /// Footpaths the planner has turned into lanes (D45).
    pub paths_paved: u32,
    /// Development opportunities tried / taken (B26).
    pub dev_tried: u32,
    pub dev_built: u32,
    pub farms: u32,
    pub last_decision_us: u64,
    pub max_decision_us: u64,
    /// Grid cells whose streets are laid (B26b).
    pub cells: u32,
}

/// The one-shot story lines of the city's life (B48a/D65): each fires exactly
/// once, and a save keeps the ring itself, so the flags only guard the live run.
/// A loaded city re-derives them from its own state ([`Sim::refresh_firsts`]).
#[derive(Clone, Copy, Debug, Default)]
struct Firsts {
    /// The party has walked in to the founding spot.
    arrived: bool,
    /// The first log cabin has been staked.
    first_cabin: bool,
    /// The first cart track is laid.
    first_track: bool,
    /// The camp has been struck (the naming, D68).
    named: bool,
    /// The first street is paved (asphalt, not the cart track).
    first_paved: bool,
    /// The first of each building kind is finished (D65: what the city
    /// *first* gets): shop, office, tower.
    first_shop: bool,
    first_office: bool,
    first_tower: bool,
    /// Which street works have had their first tile installed (B41/D63),
    /// one bit per [`streetworks::JobKind`], in its `index()` order.
    sw_done: u8,
    /// Which landmarks have finished, one bit per [`landmarks::LandmarkKind`],
    /// in its `ALL` order — the feed tells each landmark once.
    landmarks_done: u32,
}

pub struct Sim {
    pub seed: u64,
    /// Sim seconds since the founding.
    pub time: f64,
    pub pace: Pace,
    /// The first settlers (B39): the walk-in, the camp, the felling.
    pub settlers: settlers::Settlers,
    /// Street furniture installed by crews (B41/D63).
    pub streetworks: streetworks::Streetworks,
    /// The world's technology path and what its crews have installed
    /// (B49/D72/P5).
    pub tech: tech::Tech,
    /// Length of the last planning step, in sim seconds (B40).
    pub last_dt: f64,
    pub map: TileMap,
    pub lots: Vec<Lot>,
    pub fields: Vec<Field>,
    /// B80: tiles of farmland the town just took; `tidy_fields` looks beside them.
    field_touch: Vec<(i32, i32)>,
    /// B80: a fresh world's farms are laid at the first step, once the frame is known.
    countryside_pending: bool,
    pub events: Vec<Event>,
    pub stats: Stats,
    /// B78 (D84): what the years and the street have done to each lot, in lot
    /// order. Derived (never saved): recomputed by `ageing_step`, so a save/load
    /// or a long catch-up lands in exactly the same state.
    pub wear: Vec<ageing::Wear>,
    /// Which lot each entry of `wear` was worked out for (x, y, seed), so a lot
    /// removed or rebuilt since the last tick cannot lend its wear to the lot
    /// that took its place in the list ([`Sim::lot_wear`]).
    wear_keys: Vec<(i32, i32, u32)>,
    /// Tiles the sim has rewritten (what a save has to carry).
    overrides: BTreeMap<(i32, i32), Tile>,
    acc: f64,
    step_index: u64,
    next_decision: f64,
    changes: BTreeSet<(i32, i32)>,
    /// Lane tiles waiting to be upgraded, with the sim time each one is due:
    /// the paving spreads along the street instead of switching at once.
    upgrade_queue: Vec<(i32, i32)>,
    upgrade_due: Vec<((i32, i32), f64)>,
    /// A road has already been run toward the river (so the search runs once).
    aimed_at_river: bool,
    /// Style districts, seeded on first use (B26 item 3).
    districts: Option<Vec<((i32, i32), District)>>,
    /// Land value per tile, refreshed at each decision (B26 item 1).
    land: Vec<f32>,
    /// Villager layer (D45): the routes people actually walk, refreshed hourly,
    /// and the tiles that have been walked on.
    routes: Vec<Vec<(i32, i32)>>,
    worn: BTreeSet<(i32, i32)>,
    /// Rotates which cached route gets the next trip's wear.
    trip_cursor: usize,
    /// The planned street grid (B26b items 1/3).
    blocks: Blocks,
    /// Vehicles on the streets (B29).
    pub traffic: Traffic,
    /// Cars parked in bays, the highway and its interchange (B59).
    pub cars: carculture::Cars,
    /// Rail, trams, boats and planes (B31).
    pub rail: rail::Rail,
    /// Residents and their daily life (B43/D65).
    pub people: people::People,
    /// The world's land (D66): temperate for now, set by the generation
    /// when the land roll arrives (B46 on lane/biome).
    pub biome: map::Biome,
    /// The city's name (D68), chosen by the founders when the camp is struck;
    /// the settings app can override it for display.
    pub city_name: String,
    /// The story feed (D65): a bounded ring the tray reads newest first.
    pub story: story::Story,
    /// B51: the world's written history (typed entries, bounded, saved v16).
    pub chronicle: chronicle::Chronicle,
    /// One-shot story lines (the naming, the first cabin, the first street)
    /// and the population-milestone index, so each line fires exactly once.
    firsts: Firsts,
    pop_milestone: usize,
    /// Sim time of the last family-story line (families are throttled).
    family_story_at: f64,
    /// The last weather extreme noted in the story (0 = none) and its time,
    /// so a storm is announced once, not every tick (B48a).
    wx_kind: u8,
    wx_at: f64,
    /// Which holidays the feed has told already (one bit per [`Self::HOLIDAYS`]).
    holidays_told: u8,
    /// Why `try_lot` refused (diagnostics).
    lot_reason: [u32; 8],
    /// B66: the map's land regions, to keep plots off land the streets cannot
    /// reach. Derived from the map, not saved.
    reach: reach::Reach,
    /// Tile-space point that sits at the middle of the framebuffer (D35).
    pub focus: (f32, f32),
    pub founding: (i32, i32),
    /// The camera: screen origin of tile (0,0,0) and the framebuffer size in
    /// pixels (B31b). Not saved: it belongs to the viewer, not to the city.
    /// The sim only uses it to keep waterfronts inside the frame.
    pub viewport: (i32, i32, i32, i32),
    /// B42f: the land line (the sky band) in framebuffer pixels, and the belt
    /// below it that must stay countryside. Anything built closer than that to
    /// the far edge is cut flat by the horizon (the owner, B42f), so the sim does
    /// not build there: the tiles stay forest and fields that fade into the
    /// haze. Both are view state, not city state: not saved.
    horizon_px: i32,
    back_belt_px: i32,
    /// B50: how and when this world ends, and how far the ending has got.
    pub ending: ending::State,
    /// B52: the economy, the government, the pollution and the incidents.
    pub civics: civics::Civics,
    /// B77/D84: the real calendar date the city is living in, for the school
    /// terms (`week::school_holiday`). Not city state — the viewer's clock —
    /// so it is not saved.
    pub calendar: Option<(i32, u8, u8)>,
}

impl Sim {
    /// Tell the sim which frame the viewer is showing (B31b): the camera origin
    /// and the framebuffer size, exactly as `map::world_for_viewport` returns
    /// them. Used to keep the marina and the port inside the picture.
    pub fn set_viewport(&mut self, ox: i32, oy: i32, fb_w: i32, fb_h: i32) {
        if self.viewport == (ox, oy, fb_w, fb_h) {
            return;
        }
        self.viewport = (ox, oy, fb_w, fb_h);
        // The marina and the port are the two things sited *by* the frame: if
        // the viewer's camera is not the guessed one, put them where they can
        // actually be seen (B31b item 3).
        self.refresh_waterfront();
    }

    /// Tell the sim where the land line is (B42f): the sky band in framebuffer
    /// pixels and how deep the countryside belt below it must be. `sky_px = 0`
    /// switches the belt off (a frame with no sky band, and every test that
    /// does not model one).
    pub fn set_horizon(&mut self, sky_px: i32, belt_px: i32) {
        self.horizon_px = sky_px.max(0);
        self.back_belt_px = if sky_px > 0 { belt_px.max(0) } else { 0 };
        // B80: the frame and the land line are known: lay the farms.
        self.maybe_seed_countryside();
    }

    /// A test knob (B80): found this world without its farms, to compare a world
    /// with and without them.
    pub fn skip_countryside(&mut self) {
        self.countryside_pending = false;
    }

    /// Lay a fresh world's farms once (B80). Called when the frame is known
    /// (`set_horizon`), or at the first step if nobody told the sim.
    fn maybe_seed_countryside(&mut self) {
        if self.countryside_pending {
            self.countryside_pending = false;
            if self.time == 0.0 {
                self.seed_countryside();
            }
        }
    }

    /// The belt the horizon leaves alone, in framebuffer pixels (0 = none).
    pub fn back_belt(&self) -> (i32, i32) {
        (self.horizon_px, self.back_belt_px)
    }

    /// B42f: is this tile so near the back of the frame that anything built on
    /// it would be cut by the horizon? The test is the tile's own screen row:
    /// the iso projection sends small `x + y` to the top of the frame, so the
    /// belt is a diagonal band in tile space, exactly where the map meets the
    /// sky band on screen.
    pub fn behind_horizon(&self, x: i32, y: i32) -> bool {
        if self.back_belt_px <= 0 {
            return false;
        }
        let (_, oy, _, _) = self.viewport;
        let sy = oy + (x + y) * map::iso_consts::HALF_H;
        sy < self.horizon_px + self.back_belt_px
    }

    /// B99: is this tile within `margin` pixels of the land line (or above it)?
    /// Rail, platforms and the like: a slab drawn there is cut flat by the sky.
    pub fn near_land_line(&self, x: i32, y: i32, margin: i32) -> bool {
        if self.horizon_px <= 0 {
            return false;
        }
        let (_, oy, _, _) = self.viewport;
        oy + (x + y) * map::iso_consts::HALF_H < self.horizon_px + margin
    }

    /// B99: how many storeys can stand on a plot whose back corner is `(x, y)`
    /// before its roof crosses the land line (the sky band would cut it flat).
    /// A storey is 10 px in a tall block (see `render::city`), plus the roof
    /// slab. `u8::MAX` when no sky band is declared.
    pub fn headroom_storeys(&self, x: i32, y: i32) -> u8 {
        if self.horizon_px <= 0 {
            return u8::MAX;
        }
        let (_, oy, _, _) = self.viewport;
        let sy = oy + (x + y) * map::iso_consts::HALF_H;
        let room = sy - self.horizon_px - 12;
        (room / 10).clamp(1, 250) as u8
    }

    /// Is every one of these tiles **below the land line**, with `margin` pixels
    /// to spare? The land line is the sky height the viewer hands to
    /// [`Self::set_horizon`]; `margin` is the height of the tallest thing that
    /// will stand on the tile (a crane, a shed roof, a hangar).
    ///
    /// This is the line that matters for transport (B58d): the harbour's quay,
    /// its pier deck and the marina pontoon were drawn *above* it, over the sky
    /// band, and cut by the horizon haze. The city's own belt (`back_belt_px`,
    /// below the line) is a wider margin it keeps for tall buildings; water,
    /// low quays and piers do not need it, and on these coasts it would leave no
    /// harbour at all.
    pub fn clear_of_belt(&self, tiles: &[(i32, i32)], margin: i32) -> bool {
        if self.horizon_px <= 0 {
            return true;
        }
        let top = self.horizon_px + margin;
        let (_, oy, _, _) = self.viewport;
        tiles.iter().all(|&(x, y)| {
            let sy = oy + (x + y) * map::iso_consts::HALF_H;
            sy >= top
        })
    }

    /// Re-place the marina and the container port in the frame the viewer
    /// actually shows.
    pub fn refresh_waterfront(&mut self) {
        if !self.rail.planned {
            return;
        }
        self.rail.containers.clear();
        self.rail.port = None;
        // B61d: a freshly declared frame re-opens the search immediately.
        self.rail.port_tried = 0.0;
        self.rail.marina = None;
        if self.map.ocean.is_some() {
            self.rail.marina = self.marina_zone(4);
        }
        if self.era() >= Era::City {
            self.rail.port = self.waterfront_zone(District::Industrial, 6, true);
            if let Some(p) = self.rail.port {
                self.clear_port_plots(p);
                self.stack_containers(p);
            }
        }
    }

    /// Has the viewer told the sim its frame? A test that never sets one must
    /// not have its transport trimmed to a frame it did not choose (B58).
    pub fn frame_known(&self) -> bool {
        self.viewport != (480, -240, 960, 540)
    }

    /// In the picture — or no picture has been declared, in which case
    /// everything counts as in it.
    pub fn in_view(&self, x: i32, y: i32, margin: i32) -> bool {
        !self.frame_known() || self.in_frame(x, y, self.map.at(x, y).min_level() as i32 * 6, margin)
    }

    /// Would a point at tile `(x, y)` (ground height `z` px) be on screen? The
    /// margins keep a waterfront clear of the frame's edge.
    pub fn in_frame(&self, x: i32, y: i32, z: i32, margin: i32) -> bool {
        let (ox, oy, w, h) = self.viewport;
        let sx = ox + (x - y) * map::iso_consts::HALF_W;
        let sy = oy + (x + y) * map::iso_consts::HALF_H - z;
        sx >= margin && sx <= w - margin && sy >= margin && sy <= h - margin
    }

    /// Fresh world at `(seed)` with nothing built yet, plus the surveyor's
    /// stakes for the first lane.
    /// A fresh world whose first settlement starts at `focus` (tile space) —
    /// the founding monitor's centre, not the union centre (B17 item 4).
    pub fn with_focus(seed: u64, pace: Pace, w: i32, h: i32, focus: (f32, f32)) -> Self {
        Self::with_focus_in(seed, pace, w, h, focus, None)
    }

    /// A fresh world in a chosen land (D66): `biome` overrides the roll
    /// (`--biome desert` for a spike or a wallpaper test). `None` rolls the land
    /// from the seed.
    pub fn with_focus_in(
        seed: u64,
        pace: Pace,
        w: i32,
        h: i32,
        focus: (f32, f32),
        biome: Option<map::Biome>,
    ) -> Self {
        // B61: a frame too small to hold a world is clamped up to the smallest
        // one that can be generated, instead of aborting in the generator.
        let (w, h) = (w.max(map::MIN_SIDE), h.max(map::MIN_SIDE));
        let map = map::generate_with(seed, w, h, biome).expect("a map at its minimum size");
        // The land may have been overridden (`--biome`): the name and the
        // founders' naming follow the *actual* land the world holds.
        let (biome, city_name) = if let Some(b) = biome {
            (b, story::city_name(seed, b))
        } else {
            (map.biome, story::city_name(seed, map.biome))
        };
        let mut sim = Sim::new(seed, pace, w, h);
        sim.map = map;
        sim.overrides.clear();
        sim.changes.clear();
        sim.biome = biome;
        sim.city_name = city_name;
        sim.ending = ending::State::plan(seed, &sim.map);
        sim.focus = focus;
        sim.found();
        // B80 (D84): the farms the town will grow into (laid at the first step,
        // when the frame they must fill is known).
        sim.countryside_pending = true;
        sim
    }

    pub fn new(seed: u64, pace: Pace, w: i32, h: i32) -> Self {
        let map = generate(seed, w, h);
        let biome = map.biome;
        let city_name = story::city_name(seed, biome);
        let ending = ending::State::plan(seed, &map);
        let mut sim = Sim {
            seed,
            time: 0.0,
            pace,
            settlers: settlers::Settlers::default(),
            streetworks: streetworks::Streetworks::default(),
            tech: tech::Tech::plan(seed),
            last_dt: PLANNING_STEP,
            map,
            lots: Vec::new(),
            fields: Vec::new(),
            field_touch: Vec::new(),
            countryside_pending: false,
            events: Vec::new(),
            stats: Stats::default(),
            overrides: BTreeMap::new(),
            acc: 0.0,
            step_index: 0,
            next_decision: DECISION_PERIOD,
            changes: BTreeSet::new(),
            wear: Vec::new(),
            wear_keys: Vec::new(),
            upgrade_queue: Vec::new(),
            upgrade_due: Vec::new(),
            districts: None,
            land: Vec::new(),
            routes: Vec::new(),
            worn: BTreeSet::new(),
            trip_cursor: 0,
            blocks: Blocks::default(),
            traffic: Traffic::default(),
            cars: carculture::Cars::default(),
            rail: rail::Rail::default(),
            people: people::People::default(),
            biome,
            city_name,
            story: story::Story::default(),
            chronicle: chronicle::Chronicle::default(),
            firsts: Firsts::default(),
            pop_milestone: 0,
            family_story_at: 0.0,
            wx_kind: 0,
            wx_at: 0.0,
            holidays_told: 0,
            lot_reason: [0; 8],
            reach: reach::Reach::default(),
            aimed_at_river: false,
            focus: (w as f32 * 0.5, h as f32 * 0.5),
            founding: (w / 2, h / 2),
            // A 960x540 frame's camera; the viewer overrides it with its own.
            viewport: (480, -240, 960, 540),
            horizon_px: 0,
            back_belt_px: 0,
            ending,
            civics: civics::Civics::default(),
            calendar: None,
        };
        sim.found();
        sim
    }

    /// Tell the sim where the middle of the screen is in tile space, so the
    /// settlement starts there (D35) rather than at the middle of the map.
    /// Re-run the founding (used by the focus test, and when the camera moves).
    pub fn found_again(&mut self) {
        self.lots.clear();
        self.fields.clear();
        self.found();
    }

    pub fn set_focus(&mut self, fx: f32, fy: f32) {
        self.focus = (fx, fy);
    }

    /// Move the sim clock to `hour` of the current day (a test knob: the
    /// wallpaper starts a saved city at whatever time it was saved, and a rush
    /// hour is what makes the traffic measurable).
    pub fn set_clock_hour(&mut self, hour: f64) {
        let day = (self.time / 86400.0).floor();
        self.time = day * 86400.0 + hour.clamp(0.0, 23.99) * 3600.0;
        // A pinned clock is exactly that instant: the smooth clock (D71) must
        // not carry a leftover fraction from the last planning step, or a test
        // that pins 17:30 would draw 17:30:09.
        self.acc = 0.0;
        self.next_decision = self.time + 60.0;
    }

    pub fn era(&self) -> Era {
        // Eras come from the number of **residents**, and the calendar is the
        // floor under that (D47: village ≈ a week, town ≈ two, city ≈ a month).
        // The floor matters: a village boxed in by hills must still become a
        // town, or the planned town never starts (B26b).
        let by_pop = Era::from_population(self.residents());
        let d = self.days();
        let by_time = if d >= 20.0 {
            Era::City
        } else if d >= 7.0 {
            Era::Town
        } else if d >= 2.0 {
            Era::Village
        } else {
            Era::Settlement
        };
        by_pop.max(by_time)
    }

    /// Half the smaller side of the world, in tiles: the scale every district
    /// radius is measured against (B26c: a wide frame must have a wide city).
    pub fn span(&self) -> f32 {
        self.map.w.min(self.map.h) as f32 * 0.5
    }

    /// People who live in the city: finished homes only (an office block is
    /// jobs, not residents — B26b: counting workplaces as population made the
    /// eras jump a whole step in a day).
    pub fn residents(&self) -> u32 {
        self.lots
            .iter()
            .filter(|l| l.finished())
            .map(|l| match l.kind {
                LotKind::House => 4,
                LotKind::Townhouse => 2 * l.storeys.max(1) as u32,
                LotKind::Apartment => 3 * l.storeys.max(1) as u32,
                _ => 0,
            })
            .sum()
    }

    /// People living and working in the city: finished buildings only. A
    /// building's headcount follows its **height** (B26d): a 50-storey tower
    /// holds more than a 30-storey one, so a densifying city keeps growing.
    pub fn population(&self) -> u32 {
        self.lots
            .iter()
            .filter(|l| l.finished())
            .map(|l| l.occupants_now())
            .sum()
    }

    /// Change the growth pace of a running city (B37: the settings file).
    pub fn set_pace(&mut self, pace: Pace) {
        self.pace = pace;
    }

    /// B72: is the city in its weekend? (The people's own days follow this.)
    pub fn is_weekend(&self) -> bool {
        let wd = ((self.time / 86400.0) + 3.0).rem_euclid(7.0);
        wd < 1.0 || wd >= 6.0
    }

    /// Sim days elapsed.
    pub fn days(&self) -> f64 {
        self.time / 86400.0
    }

    /// B84: what this world's state holds, biggest first, so the wallpaper's
    /// memory report can name the sim's share instead of leaving it in the
    /// remainder. Byte counts of the vectors' capacity, like the scene's table.
    pub fn mem_report(&self) -> Vec<(&'static str, usize)> {
        fn v<T>(x: &Vec<T>) -> usize {
            x.capacity() * std::mem::size_of::<T>()
        }
        fn nested<T>(x: &Vec<Vec<T>>) -> usize {
            x.capacity() * std::mem::size_of::<Vec<T>>() + x.iter().map(v).sum::<usize>()
        }
        fn map_of<K, V>(x: &std::collections::BTreeMap<K, V>) -> usize {
            x.len() * (std::mem::size_of::<K>() + std::mem::size_of::<V>() + 40)
        }
        fn set_of<K>(x: &std::collections::BTreeSet<K>) -> usize {
            x.len() * (std::mem::size_of::<K>() + 40)
        }
        let mut out: Vec<(&'static str, usize)> = vec![
            (
                "sim: map + planning",
                v(&self.map.tiles)
                    + v(&self.land)
                    + v(&self.lots)
                    + v(&self.fields)
                    + v(&self.events)
                    + self.overrides.len() * (8 + std::mem::size_of::<map::Tile>() + 40)
                    + self.changes.len() * 40
                    + v(&self.upgrade_queue)
                    + v(&self.upgrade_due)
                    + self.districts.as_ref().map_or(0, |d| v(d))
                    + v(&self.blocks.plan)
                    + set_of(&self.blocks.open)
                    + set_of(&self.blocks.skip)
                    + set_of(&self.blocks.reclaimed),
            ),
            (
                "sim: routes + wear",
                nested(&self.routes) + set_of(&self.worn) + v(&self.map.water_class) + v(&self.map.wear),
            ),
            (
                "sim: people",
                self.people.bytes(),
            ),
            (
                "sim: traffic + cars",
                v(&self.traffic.vehicles)
                    + self.traffic.vehicles.iter().map(|x| v(&x.route)).sum::<usize>()
                    + v(&self.cars.parked)
                    + v(&self.cars.highway)
                    + v(&self.cars.ramps),
            ),
            (
                "sim: rail",
                v(&self.rail.line)
                    + v(&self.rail.twin)
                    + v(&self.rail.tram_line)
                    + v(&self.rail.tram_stops)
                    + v(&self.rail.movers)
                    + v(&self.rail.sidings),
            ),
            (
                "sim: streetworks + tech",
                v(&self.streetworks.signs)
                    + v(&self.streetworks.jobs)
                    + set_of(&self.streetworks.paint)
                    + set_of(&self.streetworks.lamps)
                    + set_of(&self.streetworks.lights)
                    + set_of(&self.streetworks.painted_done)
                    + set_of(&self.streetworks.lamps_done)
                    + set_of(&self.streetworks.signs_done)
                    + set_of(&self.streetworks.lights_done)
                    + v(&self.tech.path)
                    + v(&self.tech.done)
                    + v(&self.tech.jobs)
                    + map_of(&self.tech.lamps)
                    + set_of(&self.tech.poles)
                    + set_of(&self.tech.buried)
                    + set_of(&self.tech.cells)
                    + v(&self.tech.towers),
            ),
            (
                "sim: settlers + story",
                v(&self.settlers.party)
                    + v(&self.settlers.piles)
                    + v(&self.settlers.falling)
                    + v(&self.chronicle.entries)
                    + self.story.bytes(),
            ),
        ];
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }

    /// B61d: put the ending out of reach (developer benchmark fixture). A world
    /// normally ends at 21-39 sim days (D62), so a "day 60 city" cannot exist
    /// inside one life; this lets a bench grow one on a single land without the
    /// ending's warning phase rewriting the map under it.
    pub fn postpone_ending(&mut self) {
        self.ending.world_days = 1000.0;
    }

    /// Deterministic RNG for this step, mixed with a purpose tag so different
    /// decisions in the same step do not correlate.
    fn rng(&self, tag: u64) -> Rng {
        Rng::new(
            self.seed
                ^ self.step_index.wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ tag.wrapping_mul(0xA24B_AED4_963E_E407),
        )
    }

    // --- time ---------------------------------------------------------

    /// Advance by real seconds (scaled by the pace), in fixed planning steps.
    pub fn advance(&mut self, real_seconds: f64) {
        self.maybe_seed_countryside();
        // A NaN/inf/negative interval (a bad `speed` in the config, a clock
        // step) would poison `acc`: NaN stalls the sim for good, inf never
        // leaves the loop below.
        if !real_seconds.is_finite() || real_seconds <= 0.0 {
            return;
        }
        self.acc += real_seconds * self.pace.scale();
        loop {
            let step = self.next_step();
            if self.acc < step {
                break;
            }
            self.acc -= step;
            self.step(step);
        }
    }

    /// The sim clock as the renderer sees it: the last planning step plus the
    /// time accumulated since (B40 — settler legs interpolate off this, so a
    /// walker moves every frame instead of jumping a planning step at a time).
    pub fn smooth_time(&self) -> f64 {
        self.time + self.acc
    }

    /// Catch up to `t` sim seconds. Identical to running live to `t`.
    /// Catch up to `t` sim seconds. Defined as one long `advance`, so a long
    /// catch-up and many small advances take exactly the same steps (B40: the
    /// settlement tick changes size mid-run, and two steppers that disagree
    /// about where the change lands drift apart).
    pub fn run_to(&mut self, t: f64) {
        if t > self.time {
            self.advance((t - self.time) / self.pace.scale());
        }
    }

    /// The length of the last planning step (B40: the settlement ticks finer).
    pub fn last_step(&self) -> f64 {
        self.last_dt
    }

    /// The planning tick right now (B40): fine while the settlement is on
    /// screen — a party walking in, a chopper swinging and a tree going over all
    /// have to be *watchable*, and a 20 s tick made walkers jump five tiles and a
    /// 3 s fall invisible — and the ordinary 20 s step after that.
    fn next_step(&self) -> f64 {
        if self.settlers.is_active() {
            settlers::SETTLE_STEP
        } else {
            PLANNING_STEP
        }
    }

    fn step(&mut self, dt: f64) {
        self.last_dt = dt;
        self.time += dt;
        self.step_index += 1;
        self.walk_step();
        self.settlers_step();
        self.settlers_story();
        self.update_construction();
        // B78: the years, once an hour (the decision cadence). Cheap: a pass over
        // the lots and a compare against what they wore last hour.
        if self.time + 1e-9 >= self.next_decision {
            self.ageing_step();
        }
        self.streetworks_step();
        self.tech_step();
        self.civics_step();
        self.apply_upgrades();
        self.ending_step();
        if self.time + 1e-9 >= self.next_decision {
            self.next_decision += DECISION_PERIOD;
            let t0 = std::time::Instant::now();
            self.decide();
            let us = t0.elapsed().as_micros() as u64;
            self.stats.decisions += 1;
            self.stats.last_decision_us = us;
            self.stats.max_decision_us = self.stats.max_decision_us.max(us);
        }
    }

    // --- the growth engine (B26) ------------------------------------------

    /// The style districts, as soft regions grown from a few seed points (D46).
    /// A tile belongs to the nearest seed; the core sits on the founding.
    ///
    /// B26b: the seeds sit at real city distances (the old core in the middle,
    /// downtown and the European quarter just outside it, suburbs, estates and
    /// industry at the edge) instead of all inside the village, and each
    /// district has a radial band so "downtown in the middle, suburbs to the
    /// edges" holds (D52).
    fn district_seeds(&mut self) -> Vec<((i32, i32), District)> {
        if let Some(r) = &self.districts {
            return r.clone();
        }
        let (fx, fy) = (self.founding.0, self.founding.1);
        let span = (self.map.w.min(self.map.h) as f32) * 0.5;
        let mut rng = self.rng(0xD157);
        let mut seeds = vec![((fx, fy), District::Core)];
        // (district, distance from the founding as a fraction of half the map,
        //  angle spread so no two districts sit on top of each other)
        let plan = [
            // (district, distance from the founding / half the map, angle)
            (District::European, 0.26f32, 0.6f32),
            (District::Downtown, 0.24, 2.6),
            (District::Suburb, 0.66, 3.9),
            (District::HighEnd, 0.78, 5.1),
            (District::Industrial, 0.70, 5.9),
        ];
        for (d, rf, af) in plan {
            let a = af + rng.range_f32(-0.3, 0.3);
            let r = span * rf * rng.range_f32(0.9, 1.1);
            let x = (fx as f32 + a.cos() * r).clamp(4.0, self.map.w as f32 - 5.0) as i32;
            let y = (fy as f32 + a.sin() * r).clamp(4.0, self.map.h as f32 - 5.0) as i32;
            seeds.push(((x, y), d));
        }
        self.districts = Some(seeds.clone());
        seeds
    }

    /// Which district a tile belongs to: the nearest seed whose band the tile
    /// falls in (D52's ring structure), else the nearest seed at all.
    pub fn district_at(&self, x: i32, y: i32) -> District {
        let seeds = match &self.districts {
            Some(s) => s,
            None => return District::Core,
        };
        let (fx, fy) = (self.founding.0 as f32, self.founding.1 as f32);
        let r = ((x as f32 - fx).powi(2) + (y as f32 - fy).powi(2)).sqrt();
        // B26c item 2: the bands are **fractions of the world**, not tile
        // counts, so a wide layout gets a downtown and suburbs at its own scale
        // instead of one small town in the middle of a suburb sea.
        let span = self.span();
        let band = |d: District| match d {
            District::Core => (0.0, 0.40 * span),
            District::European => (0.14 * span, 0.74 * span),
            District::Downtown => (0.14 * span, 0.37 * span),
            District::Suburb => (0.34 * span, f32::MAX),
            District::HighEnd => (0.50 * span, f32::MAX),
            District::Industrial => (0.42 * span, f32::MAX),
        };
        let mut best = District::Core;
        let mut bd = i32::MAX;
        for &((sx, sy), d) in seeds {
            let (lo, hi) = band(d);
            if r < lo || r > hi {
                continue;
            }
            let dist = (sx - x).abs() + (sy - y).abs();
            if dist < bd {
                bd = dist;
                best = d;
            }
        }
        if bd == i32::MAX {
            // Outside every band (only just outside the core): nearest seed.
            for &((sx, sy), d) in seeds {
                let dist = (sx - x).abs() + (sy - y).abs();
                if dist < bd {
                    bd = dist;
                    best = d;
                }
            }
        }
        best
    }

    /// The land-value field, in whole-map units, refreshed at each decision:
    /// proximity to the centre, street access, a view of water, and the
    /// industrial nuisance pulling value down.
    fn update_land_value(&mut self) {
        let _ = self.district_seeds(); // seeded on first use (needs the founding)
        let (w, h) = (self.map.w, self.map.h);
        if self.land.len() != (w * h) as usize {
            self.land = vec![0.0; (w * h) as usize];
        }
        let (fx, fy) = (self.founding.0 as f32, self.founding.1 as f32);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let t = self.map.at(x, y);
                if t.is_water() {
                    self.land[i] = 0.0;
                    continue;
                }
                let d = ((x as f32 - fx).powi(2) + (y as f32 - fy).powi(2)).sqrt();
                let mut v = 60.0 / (1.0 + d * 0.22);
                // Street access.
                if [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| self.map.at(x + dx, y + dy).kind.is_lane())
                {
                    v += 14.0;
                }
                // A view of water or a rise.
                let near_water = (-2..=2).any(|dx| {
                    (-2..=2).any(|dy| self.map.at(x + dx, y + dy).is_water())
                });
                if near_water {
                    v += 9.0;
                }
                if t.max_level() >= 8 {
                    v += 4.0;
                }
                // Nuisance: the industrial quarter is cheap land.
                if self.district_at(x, y) == District::Industrial {
                    v -= 12.0;
                }
                self.land[i] = v.max(0.5);
            }
        }
    }

    /// Land value at a tile (0 when not computed yet).
    pub fn land_value(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x >= self.map.w || y >= self.map.h {
            return 0.0;
        }
        self.land
            .get((y * self.map.w + x) as usize)
            .copied()
            .unwrap_or(0.0)
    }

    /// How dense the town is getting: 0 = village, 1 = town, 2 = city-scale
    /// density. A town that has used its land starts going up before the
    /// population formally makes it a city (D6/D39: densify as you grow).
    pub fn density_level(&self) -> u8 {
        // D47's dates are the development schedule: village until ~day 8, town
        // to ~day 20, then the city goes up. The era's own threshold can push
        // it ahead of the clock but never behind it.
        let by_time = if self.days() >= 20.0 {
            2
        } else if self.days() >= 7.5 {
            1
        } else {
            0
        };
        let by_era = match self.era() {
            Era::Settlement | Era::Village => 0,
            Era::Town => 1,
            Era::City | Era::Metropolis => 2,
        };
        by_time.max(by_era)
    }

    /// What the era, the density and the district want built next.
    fn demanded_kind(&self, d: District) -> LotKind {
        let lvl = self.density_level();
        match d {
            District::Downtown => {
                if lvl >= 2 {
                    LotKind::Tower
                } else {
                    LotKind::Office
                }
            }
            District::Core | District::European => match lvl {
                0 => LotKind::House,
                1 => LotKind::Townhouse,
                _ => LotKind::Apartment,
            },
            District::Suburb => {
                if lvl >= 2 {
                    LotKind::Townhouse
                } else {
                    LotKind::House
                }
            }
            District::HighEnd => LotKind::House,
            District::Industrial => LotKind::Industry,
        }
    }

    // --- villagers and desire paths (D45) -------------------------------

    /// How busy the village is at this hour, 0..1: mornings and evenings are
    /// the rush, the small hours are quiet, Sunday morning belongs to church.
    fn activity(&self) -> f64 {
        let hod = (self.time / 3600.0).rem_euclid(24.0);
        let weekday = ((self.time / 86400.0) + 3.0).rem_euclid(7.0) < 1.0; // Sunday
        let base: f64 = if (7.0..=9.0).contains(&hod) {
            1.0
        } else if (12.0..=13.0).contains(&hod) {
            0.7
        } else if (16.0..=19.0).contains(&hod) {
            0.9
        } else if (9.0..=16.0).contains(&hod) {
            0.45
        } else if hod < 6.0 || hod > 21.0 {
            0.08
        } else {
            0.3
        };
        let church: f64 = if weekday && (9.0..=11.0).contains(&hod) { 0.5 } else { 0.0 };
        (base + church).min(1.4)
    }

    /// Rebuild the walking routes from the current city (called from `decide`,
    /// so it costs nothing per step). Routes are the trips people make: house to
    /// store/church/shop, farm to field, and a walk to the water.
    fn plan_trips(&mut self) {
        self.routes.clear();
        if self.lots.len() < 2 {
            return;
        }
        let mut rng = self.rng(0x7A11);
        // A trip starts at a house door and ends at another door, so lots are
        // not obstacles (their ground is ordinary grass to a pedestrian).
        let blocked = BTreeSet::new();
        let houses: Vec<(i32, i32)> = self
            .lots
            .iter()
            .map(|l| (l.x + l.w / 2, l.y + l.h / 2))
            .collect();
        // Destinations: the civic and commercial lots first (they are the trips
        // that repeat every day), then a few neighbours.
        let mut dests: Vec<(i32, i32)> = self
            .lots
            .iter()
            .filter(|l| matches!(l.kind, LotKind::Church | LotKind::Store | LotKind::Shop))
            .map(|l| (l.x + l.w / 2, l.y + l.h / 2))
            .collect();
        if let Some(f) = self.fields.first() {
            dests.push((f.x + f.w / 2, f.y + f.h / 2));
        }
        // The water: a walk to the river or the lake.
        if let Some(r) = &self.map.river {
            if let Some(&p) = r.path.get(r.path.len() / 2) {
                dests.push(p);
            }
        }
        if let Some(l) = &self.map.lake {
            dests.push(l.centre);
        }
        const MAX_ROUTES: usize = 10;
        for i in 0..MAX_ROUTES {
            let home = houses[rng.usize(houses.len())];
            let to = if !dests.is_empty() && i % 2 == 0 {
                dests[rng.usize(dests.len())]
            } else {
                houses[rng.usize(houses.len())]
            };
            if home == to {
                continue;
            }
            if let Some(path) = grid::route_walk(&self.map, home, to, rng.next_u64(), &blocked) {
                self.routes.push(path);
            }
        }
    }

    /// One step of foot traffic: a little wear along the route currently in use,
    /// and a slow fade on everything else. No walker is simulated individually
    /// during catch-up — the trips are the cached routes scaled by the hour, so
    /// a hundred days of catch-up cost the same per step as one minute.
    fn walk_step(&mut self) {
        if self.routes.is_empty() {
            return;
        }
        // One trip per route per sim minute: a third of the planning steps.
        if self.step_index % 3 != 0 {
            return;
        }
        let act = self.activity() as f32;
        if act < 0.05 {
            return;
        }
        let per_step = act;
        let r = self.trip_cursor % self.routes.len();
        self.trip_cursor = self.trip_cursor.wrapping_add(1);
        let amount = (2.0 * per_step).round().max(1.0) as u16;
        let path = self.routes[r].clone();
        let mut changed = false;
        for &(x, y) in &path {
            let t = self.map.at(x, y);
            if t.is_water() || t.kind.is_lane() || t.kind.is_worked() {
                continue; // already a street or a building
            }
            let stage = map::WearStage::of(self.map.wear(x, y));
            if self.map.add_wear(x, y, amount) {
                changed = true;
                self.changes.insert((x, y));
            }
            // Footpaths alongside a street are what the planner paves later.
            if stage >= map::WearStage::Footpath {
                self.worn.insert((x, y));
            }
        }
        let _ = changed;
        // Back-off: unused ground fades, a little every sim hour.
        if self.step_index % 180 == 0 {
            let worn: Vec<(i32, i32)> = self.worn.iter().copied().collect();
            for (x, y) in worn {
                if self.map.wear(x, y) > 0 {
                    let now = self.map.wear(x, y).saturating_sub(3);
                    self.map.wear[(y * self.map.w + x) as usize] = now;
                    if map::WearStage::of(now) < map::WearStage::Footpath {
                        self.worn.remove(&(x, y));
                    }
                    self.changes.insert((x, y));
                }
            }
        }
    }

    /// A busy footpath becomes a lane (D45 item 3): lay a dirt lane along the
    /// worn tiles nearest the network, graded per D34.
    /// Is there a paved lane beside this plot (B39: the block grid's frontage)?
    pub(crate) fn has_lane_near(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        for ty in y - 1..=y + h {
            for tx in x - 1..=x + w {
                if self.map.at(tx, ty).kind.is_lane() {
                    return true;
                }
            }
        }
        false
    }

    /// Tiles worn into a footpath or a cart track (B39: the first streets).
    pub fn path_tiles(&self) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        for y in 0..self.map.h {
            for x in 0..self.map.w {
                if matches!(
                    map::WearStage::of(self.map.wear(x, y)),
                    map::WearStage::Footpath | map::WearStage::CartTrack
                ) {
                    v.push((x, y));
                }
            }
        }
        v
    }

    fn pave_worn_path(&mut self) -> bool {
        if self.worn.is_empty() {
            return false;
        }
        let lane: BTreeSet<(i32, i32)> = self.lane_tiles().into_iter().collect();
        // Try every worn tile near the network, not just the first: a one-tile
        // spur must not stop the planner (D45).
        // A lane beside a plot would make the planner clear the plot (D41), so a
        // path that hugs a house is left as a path.
        // B26b: only the plot itself is off limits — a lot may front a street,
        // so a footpath one tile away from a house is exactly the path a village
        // paves into its lane (D45).
        let lots = self.lot_tiles();
        let candidates: Vec<(i32, i32)> = self
            .worn
            .iter()
            .copied()
            .filter(|(x, y)| {
                // Never pave over a house or up a cliff: that is what the
                // graded-road invariants (D34) and the yard rule (D41) forbid.
                // Nor across the airfield's ground (B99c: a footpath through the
                // runway's grass became a street, because only `lay_lane` knew
                // the airfield was there).
                !lots.contains(&(*x, *y))
                    && !self.on_airfield(*x, *y)
                    && self.map.at(*x, *y).relief() <= 1
            })
            .filter(|(x, y)| {
                (-2..=2)
                    .any(|dx| (-2..=2).any(|dy| lane.contains(&(x + dx, y + dy))))
            })
            .collect();
        let mut path: Vec<(i32, i32)> = Vec::new();
        for start in candidates {
            let mut p = vec![start];
            let mut cur = start;
            for _ in 0..12 {
                let mut next = None;
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let n = (cur.0 + dx, cur.1 + dy);
                    if p.contains(&n) {
                        continue;
                    }
                    if self.map.wear(n.0, n.1) >= map::WearStage::FOOTPATH_AT
                        && !self.map.at(n.0, n.1).kind.is_lane()
                        && !lots.contains(&n)
                        && !self.on_airfield(n.0, n.1)
                        && self.map.at(n.0, n.1).relief() <= 1
                    {
                        next = Some(n);
                        break;
                    }
                }
                match next {
                    Some(n) => {
                        p.push(n);
                        cur = n;
                    }
                    None => break,
                }
            }
            if p.len() >= 3 {
                path = p;
                break;
            }
        }
        if path.len() < 3 {
            return false;
        }
        // Join the path to the street: a stub would split the network (D32).
        let first = path[0];
        let mut best: Option<((i32, i32), i32)> = None;
        for &l in lane.iter() {
            let d = (l.0 - first.0).abs() + (l.1 - first.1).abs();
            if best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((l, d));
            }
        }
        if let Some((target, d)) = best {
            if d > 1 {
                let mut cur = first;
                let mut link = Vec::new();
                while cur != target && link.len() < 8 {
                    let dx = (target.0 - cur.0).signum();
                    let dy = (target.1 - cur.1).signum();
                    cur = if (target.0 - cur.0).abs() >= (target.1 - cur.1).abs() {
                        (cur.0 + dx, cur.1)
                    } else {
                        (cur.0, cur.1 + dy)
                    };
                    link.push(cur);
                }
                // The connector to the street must not cut through a house
                // either (B40: `the_village_has_space_between_houses`).
                if link.iter().any(|t| lots.contains(t)) {
                    return false;
                }
                link.reverse();
                link.extend(path.iter().copied());
                path = link;
            }
        }
        // Only pave where the graded lane will sit within one level of the
        // ground beside it (D34) — otherwise the street would cut a step.
        {
            let mut level = self.map.at(path[0].0, path[0].1).min_level() as i32;
            let mut ok = true;
            for &(x, y) in &path {
                let natural = self.map.at(x, y).min_level() as i32;
                level += (natural - level).signum();
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let nb = self.map.at(x + dx, y + dy).min_level() as i32;
                    if (level - nb).abs() > 1 {
                        ok = false;
                    }
                }
            }
            if !ok {
                return false;
            }
        }
        self.lay_lane(&path, Kind::Dirt);
        for &(x, y) in &path {
            self.worn.remove(&(x, y));
            self.map.wear[(y * self.map.w + x) as usize] = 0;
            self.changes.insert((x, y));
        }
        self.stats.paths_paved += 1;
        self.events.push(Event {
            time: self.time,
            text: format!("footpath paved: {} tiles", path.len()),
        });
        true
    }

    /// Villagers to draw: positions along the cached routes, phased by `phase`.
    /// Cheap and deterministic — no walker state is stored.
    pub fn walkers(&self, phase: f32) -> Vec<(f32, f32, u8)> {
        let act = self.activity() as f32;
        let n = (self.routes.len() as f32 * act * 1.6) as usize;
        let mut out = Vec::with_capacity(n);
        for (i, route) in self.routes.iter().take(n).enumerate() {
            if route.len() < 2 {
                continue;
            }
            let t = ((phase * 0.06) + i as f32 * 0.37).fract();
            let f = t * (route.len() - 1) as f32;
            let a = route[f as usize];
            let b = route[(f as usize + 1).min(route.len() - 1)];
            let frac = f.fract();
            let x = a.0 as f32 + (b.0 - a.0) as f32 * frac;
            let y = a.1 as f32 + (b.1 - a.1) as f32 * frac;
            out.push((x, y, ((phase * 4.0) as u8) % 2));
        }
        out
    }

    // --- residents (B43/D65) -------------------------------------------

    /// The beds and the jobs of the finished city, as the people layer wants
    /// them: homes in lot order with their bed counts, workplaces with their
    /// door and kind, and one pseudo-job per field for the farm hands.
    fn people_slots(&self) -> (Vec<people::HomeSlot>, Vec<people::JobSlot>) {
        use people::JobKind;
        let mut homes = Vec::new();
        let mut jobs = Vec::new();
        for l in self.lots.iter().filter(|l| l.finished()) {
            let beds = match l.kind {
                LotKind::House | LotKind::LogCabin => 4,
                LotKind::Townhouse => 2 * l.storeys.max(1) as u32,
                LotKind::Apartment => 3 * l.storeys.max(1) as u32,
                _ => 0,
            };
            if beds > 0 {
                homes.push(people::HomeSlot {
                    x: l.x,
                    y: l.y,
                    door: people::door_tile(l),
                    capacity: beds,
                });
            }
            let kind = match l.kind {
                LotKind::Office | LotKind::Tower => JobKind::Office,
                LotKind::Industry => JobKind::Factory,
                LotKind::Shop | LotKind::Store => JobKind::Shop,
                LotKind::Landmark(crate::landmarks::LandmarkKind::School) => JobKind::School,
                LotKind::Landmark(_) => JobKind::Office,
                _ => continue,
            };
            jobs.push(people::JobSlot {
                x: l.x,
                y: l.y,
                door: people::door_tile(l),
                kind,
            });
        }
        // B80: a few farm jobs, not one per field of a countryside of farms.
        for f in self.fields.iter().filter(|f| map::crop_of(&self.map.at(f.x, f.y)).is_arable()).take(6) {
            jobs.push(people::JobSlot {
                x: f.x,
                y: f.y,
                door: (f.x, f.y),
                kind: JobKind::Farm,
            });
        }
        (homes, jobs)
    }

    /// Keep the resident list in step with the city (D65). Runs on the hourly
    /// growth decision: people are only ever added, and every one takes a bed
    /// and (usually) a job, so the population follows the growth numbers the
    /// rest of the sim already uses.
    pub fn sync_people(&mut self) {
        let (homes, jobs) = self.people_slots();
        let target = self.residents();
        self.people.econ_slump = self.in_slump();
        let world = people::World {
            map: &self.map,
            lots: &self.lots,
            fields: &self.fields,
            seed: self.seed,
        };
        let day = (self.time / 86400.0).floor() as i64;
        let arrivals = self.people.sync(&world, target, &homes, &jobs, day);
        // B48: a day's worth of lives (births, weddings, funerals, moves), the
        // trucks for the households that arrived, and the lines whose scene
        // has begun. All planning-step work; nothing here runs per frame.
        self.people.life_pass(&world, &homes, self.time);
        let tell = !arrivals.is_empty() && self.time - self.family_story_at >= 6.0 * 3600.0;
        self.people.moves_in(&world, &arrivals, self.time, tell);
        if tell {
            self.family_story_at = self.time;
        }
        for line in self.people.due_lines(self.time) {
            self.push_life(line);
        }
    }

    /// The materials at every construction site (B43): a plank stack, a brick
    /// pallet or a heap of sand beside each plot being built.
    pub fn build_sites(&self) -> Vec<(f32, f32, u8)> {
        self.lots
            .iter()
            .filter(|l| !l.finished() && l.stage >= Stage::Foundation)
            .map(|l| {
                let (dx, dy) = people::door_tile(l);
                let kind = (l.seed % 3) as u8;
                // On the frontage *outside* the lot (a pile inside it would be
                // hidden by the building's own painter key), shifted along the
                // kerb so it does not stand on the doorway itself.
                let along = match l.front % 4 {
                    0 | 2 => (0.0, 0.9),
                    _ => (0.9, 0.0),
                };
                (
                    dx as f32 + 0.5 + along.0,
                    dy as f32 + 0.5 + along.1,
                    kind,
                )
            })
            .collect()
    }

    /// Everyone doing something right now: the renderer's input (D65). A pure
    /// function of the sim clock and the city, so two calls at the same instant
    /// give the same figures and a test can pin an hour.
    ///
    /// D71: the clock is the **smooth** one (the last planning step plus the
    /// time accumulated since), so a commuter's position advances with real
    /// time and the renderer draws them gliding frame by frame instead of
    /// jumping a planning step at a time.
    pub fn people_view(&self) -> Vec<people::Figure> {
        let mut out = Vec::new();
        self.people_view_into(&mut out);
        out
    }

    /// The same, into a caller-owned vector (D71: the wallpaper calls this 30
    /// times a second, and it must not allocate each time).
    pub fn people_view_into(&self, out: &mut Vec<people::Figure>) {
        out.clear();
        let t = self.smooth_time();
        let hour = (t / 3600.0).rem_euclid(24.0);
        let world = people::World {
            map: &self.map,
            lots: &self.lots,
            fields: &self.fields,
            seed: self.seed,
        };
        self.people.view_into(&world, hour, t, self.weekday(), out);
        // B77/D84: the week's own actors — bins at the kerb on bin day, the
        // Saturday market, the school run, the Sunday morning, the small hours.
        self.weekly_figures(hour, t, out);
        // B58: the passengers the transport puts on the street — the platform,
        // the shelters, the subway entrances.
        self.waiting_figures(hour, out);
        // B50: in the last days the people react to what is coming.
        self.ending_adjust_figures(out);
    }

    /// Day of the week, 0 = Sunday, matching `activity()`'s church rule.
    pub fn weekday(&self) -> f64 {
        ((self.time / 86400.0) + 3.0).floor().rem_euclid(7.0)
    }

    /// The residents (name, age, home, job) for a story feed or a hover card.
    pub fn residents_list(&self) -> &[people::Person] {
        &self.people.list
    }

    // --- the story feed (B48a/D65) --------------------------------------

    /// The population milestones (D65): residents at which the city says it
    /// has crossed a line. Tunable; the day targets are the check.
    pub const MILESTONES: [u32; 10] = [
        50, 100, 250, 500, 1000, 2500, 5000, 10000, 15000, 20000,
    ];

    /// One line for the feed, at the current sim time (D65). Bounded by the
    /// ring.
    pub fn push_story(&mut self, text: impl Into<String>) {
        self.push_story_t(chronicle::Topic::Note, 0, text);
    }

    /// A rolling life line (birth, wedding, funeral, move) for the ring.
    pub fn push_life(&mut self, text: impl Into<String>) {
        self.story.push(self.time, text);
    }

    /// The latest feed lines, newest first, capped at `n` (the tray shows 8).
    #[allow(dead_code)]
    pub fn story_latest(&self, n: usize) -> Vec<story::Line> {
        self.story.latest(n)
    }

    /// A weather extreme, translated to the world's own idiom (D68): the sim
    /// notes an extreme once, on the change in — a storm is announced when it
    /// comes, not on every tick. The caller (the wallpaper) passes the
    /// snapshot's `temp` (°F) and whether it storms or snows.
    pub fn note_weather(&mut self, temp_f: f32, storm: bool, snowing: bool) {
        let kind: u8 = if storm {
            1
        } else if snowing {
            2
        } else if temp_f <= 20.0 {
            3
        } else if temp_f >= 95.0 {
            4
        } else {
            // The sky is ordinary again: the town may announce the next
            // extreme afresh.
            self.wx_kind = 0;
            return;
        };
        if kind == self.wx_kind {
            return; // already out: told once
        }
        if self.wx_kind != 0 && self.time - self.wx_at < 6.0 * 3600.0 {
            return; // do not flap between two extremes on one grey day
        }
        let line = match kind {
            1 => "a storm rolls over the town: the lights flicker and the crews hold on",
            2 => "the first snow settles on the roofs and the streets are white by morning",
            3 => "a cold snap grips the town: the fires are lit and the walks are short",
            _ => "a heat wave closes in: the streets empty out and the fountains run at full",
        };
        self.wx_kind = kind;
        self.wx_at = self.time;
        self.push_story_t(chronicle::Topic::Weather, kind as i32, line);
    }

    /// The holidays the feed dresses the town for (B48a), in the stable order
    /// the wallpaper maps `render::holiday::Holiday` onto.
    pub const HOLIDAYS: [&str; 8] = [
        "christmas",
        "halloween",
        "the fourth of july",
        "new year's eve",
        "valentine's day",
        "st. patrick's day",
        "thanksgiving",
        "easter",
    ];

    /// B75: is the sky wet? The people carry umbrellas while it rains or snows.
    /// The weather is the wallpaper's (the sim never fetches it), so this is
    /// told to the sim rather than saved in it.
    pub fn set_wet(&mut self, wet: bool) {
        self.people.wet = wet;
    }

    /// B76: how much snow is lying (0..1). Above a fifth the winter service
    /// comes out: ploughs and gritters, as trucks, on the main roads.
    pub fn set_snow(&mut self, cover: f32) {
        self.traffic.snow = cover.clamp(0.0, 1.0);
    }

    /// Tell the feed a holiday has come (B48a: "holidays"): each one is told
    /// once per world. `i` is an index into [`Self::HOLIDAYS`].
    pub fn note_holiday(&mut self, i: usize) {
        if i >= Self::HOLIDAYS.len() {
            return;
        }
        let bit = 1u8 << i;
        if self.holidays_told & bit != 0 {
            return;
        }
        self.holidays_told |= bit;
        let line = match Self::HOLIDAYS[i] {
            "christmas" => "christmas: the garlands go up on the porches and the town smells of pine",
            "halloween" => "halloween: the children go door to door and the porches are full of pumpkins",
            "the fourth of july" => "the fourth of july: the flags come out and the fireworks start over the river",
            "new year's eve" => "new year's eve: the town counts down to midnight in the square",
            "valentine's day" => "valentine's day: the corner flower stand is doing very well",
            "st. patrick's day" => "st. patrick's day: the river is dyed green and the town wears it too",
            "thanksgiving" => "thanksgiving: the porches are dressed in the last leaves of the season",
            _ => "easter: the eggs are dyed and the first crocus comes up through the cold",
        };
        self.push_story_t(chronicle::Topic::Holiday, i as i32, line);
    }

    /// The name the city answers to (D68): the founders' name, or the
    /// override the user typed in the settings (an empty string is "no
    /// override").
    pub fn display_name(&self, override_name: &str) -> String {
        let o = override_name.trim();
        if o.is_empty() {
            self.city_name.clone()
        } else {
            o.to_string()
        }
    }

    /// The tray's picture of the city (B48a item 3): the name as the title,
    /// the headcount and the era as one line, and the latest feed lines newest
    /// first. "Just watch" (D65) hides the feed.
    pub fn tray_status(&self, override_name: &str, just_watch: bool) -> story::TrayStatus {
        let city = self.display_name(override_name);
        // B50: the world's age, and during a warning a hint of what is coming.
        let (age, len) = self.world_age();
        let mut line = format!(
            "{} people · {} · day {} of about {}",
            self.population(),
            self.era().name(),
            age.floor() as i64 + 1,
            len.round() as i64
        );
        if let Some(h) = self.ending_hint() {
            line.push_str(" · ");
            line.push_str(&h);
        }
        // B52: the mayor and the economy's mood.
        let civic = if self.civics.mayor.is_some() || self.civics.mood.abs() > 0.2 {
            self.civic_headline()
        } else {
            String::new()
        };
        let feed = if just_watch {
            Vec::new()
        } else {
            self.story.latest(8).into_iter().map(|l| story::tray_line(&l)).collect()
        };
        story::TrayStatus {
            city,
            line,
            feed,
            gameday: String::new(),
            civic,
        }
    }

    /// B53: a city-wide feed line from the wallpaper's own world feeds (the
    /// Bills gameday, a personal date). The caller decides when it is due.
    pub fn note_city_line(&mut self, text: &str) {
        self.push_story(text.to_string());
    }

    /// The settlement's one-shot lines (B48a): the walk-in, the first cabin,
    /// the first road, the camp struck and the naming (D68). Each fires exactly
    /// once; a loaded city re-derives these from its state.
    fn settlers_story(&mut self) {
        // Copy the settlement's state out first: the story's push borrows
        // the city mutably, and a held `&self.settlers` would refuse it.
        let (arrived, cabins, road_laid, struck) = (
            self.settlers.arrived,
            self.settlers.cabins,
            self.settlers.road_laid,
            self.settlers.struck,
        );
        if !self.firsts.arrived && arrived {
            self.firsts.arrived = true;
            let party = self.settlers.party.len() as i32;
            self.push_story_t(chronicle::Topic::Founding, party, "the settlers have walked in from the hills and cut a clearing");
        }
        if !self.firsts.first_cabin && cabins > 0 {
            self.firsts.first_cabin = true;
            self.push_story_t(chronicle::Topic::First, 0, "the first log cabin is staked: someone will sleep under a roof tonight");
        }
        if !self.firsts.first_track && road_laid {
            self.firsts.first_track = true;
            self.push_story_t(
                chronicle::Topic::First,
                0,
                "the path between the camp and the cabins is worn into the first road",
            );
        }
        if !self.firsts.named && struck {
            self.firsts.named = true;
            // D68: the city names itself when the camp goes — the name fits the land.
            self.push_story_t(chronicle::Topic::Naming, 0, format!(
                "the camp is struck and the founders name the settlement {}",
                self.city_name
            ));
        }
    }

    /// Re-derive the one-shot story flags from the city's own state, so a
    /// loaded world never re-tells a line its feed already has (B48a: a save
    /// keeps the ring, and the flags guard only the live run).
    pub(crate) fn refresh_firsts(&mut self) {
        self.firsts.arrived = self.settlers.arrived;
        self.firsts.first_cabin = self.settlers.cabins > 0;
        self.firsts.first_track = self.settlers.road_laid;
        self.firsts.named = self.settlers.struck;
        self.firsts.first_paved = self
            .map
            .tiles
            .iter()
            .any(|t| t.kind == Kind::Paved);
        // B88: only a *finished* building has been told (the live run tells a
        // first when it finishes); counting staked ones dropped the line for a
        // shop that was still going up when the city was saved.
        let kinds: Vec<LotKind> = self.lots.iter().filter(|l| l.finished()).map(|l| l.kind).collect();
        self.firsts.first_shop = kinds
            .iter()
            .any(|k| matches!(k, LotKind::Shop | LotKind::Store));
        self.firsts.first_office = kinds.iter().any(|k| matches!(k, LotKind::Office));
        self.firsts.first_tower = kinds.iter().any(|k| matches!(k, LotKind::Tower));
        self.firsts.sw_done = u8::from(!self.streetworks.paint.is_empty())
            | (u8::from(!self.streetworks.signs.is_empty()) << 1)
            | (u8::from(!self.streetworks.lamps.is_empty()) << 2)
            | (u8::from(!self.streetworks.lights.is_empty()) << 3);
        let mut mask = 0u32;
        for k in landmarks::LandmarkKind::ALL {
            if self.lots.iter().any(|l| l.finished() && l.kind == LotKind::Landmark(k)) {
                mask |= 1u32 << k.index();
            }
        }
        self.firsts.landmarks_done = mask;
        // The next milestone is the first one the city has not crossed yet.
        let pop = self.residents();
        self.pop_milestone = Self::MILESTONES
            .iter()
            .take_while(|&&m| pop >= m)
            .count();
    }

    /// Stage transitions for every lot under construction, and the first time
    /// each kind finishes (D65: what the city *first* gets, B48a).
    fn update_construction(&mut self) {
        for i in 0..self.lots.len() {
            if self.lots[i].finished() || self.time + 1e-9 < self.lots[i].stage_end {
                continue;
            }
            let next = self.lots[i].stage.next();
            if let Some(n) = next {
                self.lots[i].stage = n;
                self.lots[i].stage_end = self.time + HOUSE_SECONDS / 6.0;
                let (x, y, w, h) = (
                    self.lots[i].x,
                    self.lots[i].y,
                    self.lots[i].w,
                    self.lots[i].h,
                );
                for ty in y..y + h {
                    for tx in x..x + w {
                        self.changes.insert((tx, ty));
                    }
                }
                if n == Stage::Finished {
                    self.first_finishes(i);
                    // B49b: a finished building is new work for the technology
                    // crews (a bare roof for the solar spread, a new kerb for the
                    // poles), so the scan is armed again.
                    self.resume_streetworks();
                }
            }
        }
    }

    /// The first of each kind the feed tells about, as it comes up (B48a:
    /// "first shop/school/office/skyscraper, new landmark").
    fn first_finishes(&mut self, i: usize) {
        match self.lots[i].kind {
            LotKind::Shop | LotKind::Store if !self.firsts.first_shop => {
                self.firsts.first_shop = true;
                self.push_story_t(chronicle::Topic::First, 0, "the first shop opens its door on the main street");
            }
            LotKind::Office if !self.firsts.first_office => {
                self.firsts.first_office = true;
                self.push_story_t(chronicle::Topic::First, 0, "the first office opens: people now work where they live");
            }
            LotKind::Tower if !self.firsts.first_tower => {
                self.firsts.first_tower = true;
                self.push_story_t(
                    chronicle::Topic::First,
                    0,
                    "the first tower tops out over the town — the tallest thing people will ever build here",
                );
            }
            LotKind::Landmark(k) => self.landmark_finished(k),
            _ => {}
        }
    }

    /// A landmark is finished (B48a: "new landmark"): the feed tells each
    /// landmark once, with a line or two for the ones the town notices.
    fn landmark_finished(&mut self, k: landmarks::LandmarkKind) {
        let bit = 1u32 << k.index();
        if self.firsts.landmarks_done & bit != 0 {
            return;
        }
        self.firsts.landmarks_done |= bit;
        let line: String = match k {
            landmarks::LandmarkKind::School => {
                "the school opens: the first lessons are on Monday".into()
            }
            landmarks::LandmarkKind::Station => {
                "the rail station is finished — the trains can start running".into()
            }
            landmarks::LandmarkKind::SignatureTower => {
                "the signature tower tops out over downtown".into()
            }
            landmarks::LandmarkKind::Cathedral => {
                "the cathedral's bell tolls for the first time over the old square".into()
            }
            landmarks::LandmarkKind::Plaza => {
                "the civic square is laid: the town has its heart".into()
            }
            _ => format!("the {} is finished and open", k.name()),
        };
        self.push_story_t(chronicle::Topic::Landmark, k.index() as i32, line);
    }

    // --- growth -------------------------------------------------------

    /// Stakes for the first lane plus a couple of lots, at the most buildable
    /// flat spot nearest the middle of the frame (D35).
    fn found(&mut self) {
        let (w, h) = (self.map.w, self.map.h);
        // Stay within a few tiles of the middle of the frame (D35); widen the
        // search only if that pocket is unbuildable.
        let mut best = None;
        for radius in [4.0f32, 8.0, 16.0] {
            let mut best_score = f32::MIN;
            for y in 4..h - 4 {
                for x in 4..w - 4 {
                    if !self.map.flat_buildable(x, y) || self.map.at(x, y).max_level() > 5 {
                        continue;
                    }
                    let dx = x as f32 - self.focus.0;
                    let dy = y as f32 - self.focus.1;
                    let dist2 = dx * dx + dy * dy;
                    if dist2 > radius * radius {
                        continue;
                    }
                    let score = -dist2 * 1.5
                        + self.roominess(x, y) as f32 * 0.3
                        + (5 - self.map.at(x, y).max_level() as i32) as f32 * 0.5;
                    if score > best_score {
                        best_score = score;
                        best = Some((x, y));
                    }
                }
            }
            if best.is_some() {
                break;
            }
        }
        let (x, y) = best.unwrap_or((w / 2, h / 2));
        self.founding = (x, y);
        // B39/D62: no stakes and no lane. A party walks in from the map edge,
        // camps, and the first paths are the ones they wear walking.
        self.spawn_party();
    }

    /// How much flat, free land surrounds a tile (0..12).
    fn roominess(&self, x: i32, y: i32) -> i32 {
        let mut n = 0;
        for dy in -2..=2 {
            for dx in -2..=2 {
                let t = self.map.at(x + dx, y + dy);
                if t.kind == Kind::Grass || t.kind == Kind::Meadow {
                    n += 1;
                }
            }
        }
        n
    }

    fn decide(&mut self) {
        // B39: while the party is still walking in there is no city to grow.
        if !self.settlers.arrived {
            return;
        }
        // B50: once the end has begun there is nothing more to build.
        if self.ending_underway() {
            return;
        }
        // B80: what the last decision took of the farmland leaves slivers.
        if !self.field_touch.is_empty() {
            let taken = std::mem::take(&mut self.field_touch);
            self.tidy_fields(&taken);
        }
        // D41 invariant, enforced every decision: no plot may sit on or beside
        // a street. (Streets can appear next to an existing lot via any path.)
        // B26b: dense lots in a block may touch their neighbours, but never a
        // street, so the rule stays exactly as it was.
        //
        // B61: this used to build a `BTreeSet` of every lane tile (a whole-map
        // scan plus a 1300-node tree) and then do a tree lookup for every tile
        // of every lot, every decision. The test is only "is this tile a lane",
        // which the map answers directly, so the scan and the lookups are gone.
        let before = self.lots.len();
        self.lots.retain(|l| {
            // B26b: the town's street wall stands on the pavement, as a
            // town's does; only the village's detached kinds keep a yard on
            // every side (D41).
            // B26b: a plot must never stand **on** a street; a plan that
            // fronts one is what makes a street wall, and a house one tile
            // from a lane is a front garden, not a fault.
            !(l.y..l.y + l.h)
                .any(|ty| (l.x..l.x + l.w).any(|tx| self.map.at(tx, ty).kind.is_lane()))
        });
        let dropped = before - self.lots.len();
        self.stats.dev_tried += dropped as u32;
        if dropped > 0 {
            self.resume_streetworks();
        }
        // B88: houses and no street at all: lay streets again rather than
        // stall (a loaded city whose lanes were lost).
        if self.revive_streets() {
            return;
        }
        let days = self.days();
        let era = self.era();
        // D65: the people the city holds follow its finished homes and jobs —
        // and the feed says so: a milestone when the city crosses one, and a
        // line now and then about a family moving in (B48a).
        // B50: once the exodus has begun nobody new moves in.
        if !self.exodus_begun() {
            self.sync_people();
        }
        self.chron_era_step();
        let pop = self.residents();
        while self.pop_milestone < Self::MILESTONES.len() && pop >= Self::MILESTONES[self.pop_milestone] {
            let m = Self::MILESTONES[self.pop_milestone];
            let line = match m {
                50 => "fifty people live here: it is a settlement of record",
                100 => "a hundred people live here: the village is on the county map",
                250 => "a quarter of a thousand call this place home",
                500 => "five hundred people live here: the post office no longer has to ride",
                1000 => "the first thousand people live here: the town is a town",
                2500 => "twenty-five hundred people live here: the trains are full by evening",
                5000 => "five thousand people live here: the city is a city",
                10_000 => "ten thousand people live here: the crowds fill the avenues",
                15_000 => "fifteen thousand people live here: the signature tower is no longer enough",
                20_000 => "twenty thousand people live here",
                _ => unreachable!(),
            };
            self.push_story_t(chronicle::Topic::Population, m as i32, line);
            self.pop_milestone += 1;
        }
        // Housing demand: a handful at founding, a village by day 8 (D39).
        // D41: a village, not a terrace. Roughly 40 houses by day 8.
        // B11: a fuller village again (~60 by day 8), still detached.
        let base = (6.0 + 6.8 * days) as usize;
        let target = match era {
            Era::Settlement | Era::Village => base,
            Era::Town => base * 3,
            Era::City => base * 8,
            Era::Metropolis => base * 20,
        };
        let houses = self.lots.iter().filter(|l| l.kind == LotKind::House).count();

        // Paving is continuous background work: schedule a few tiles every
        // decision without spending the decision itself.
        self.pave_works();
        if self.land.is_empty() || (self.time / DECISION_PERIOD) as u64 % 6 == 0 {
            self.update_land_value();
        }

        // Villagers (D45): re-plan the trips every few hours, and every so often
        // turn the busiest footpath into a lane, so the street network grows out
        // of where people actually walk.
        let hour = (self.time / DECISION_PERIOD) as u64;
        if hour % 6 == 0 || self.routes.is_empty() {
            self.plan_trips();
        }

        // B48: the graveyard is reserved early, at the edge of the young town
        // on gently sloping ground beside a road, and kept clear of building
        // round it, so the town grows around it. A second one once the city
        // passes five thousand residents. Any era: a town that skipped its
        // village years still buries its dead.
        let cemeteries = self.lots.iter().filter(|l| l.kind == LotKind::Cemetery).count();
        let want = if self.residents() >= 5000 { 2 } else { 1 };
        if hour % 4 == 0
            && self.lots.len() >= 12
            && cemeteries < want
            && self.place_civic_edge(LotKind::Cemetery)
        {
            return;
        }

        if self.era() >= Era::Town {
            // B26b: the town and the city grow as planned blocks. A street is
            // only laid when the block it bounds is about to be filled, and a
            // block only opens when the one being filled is nearly full — that
            // is what keeps roads at a fifth of the built area.
            let mut built = 0;
            // One opportunity an hour in a town, two in a city: the whole town
            // must not appear in four days (D47's dates are the target).
            let n = if self.era() >= Era::City { 2 } else { 1 };
            for _ in 0..n {
                if self.fill_cell() {
                    built += 1;
                } else if self.cell_filled() && self.open_cell() {
                    built += 1;
                } else if self.cramped_land() && self.grow_along_existing() {
                    // B99c: a land of lakes, marsh and islands never finds the
                    // dry rectangle a planned block needs — its street lines run
                    // into water, and once every cell has been refused the town
                    // stopped growing altogether and stayed a village on the day
                    // a desert had a downtown (D77). Such a land builds along the
                    // lanes it already has instead, which is how a town on water
                    // is built anyway.
                    built += 1;
                } else {
                    // D6: the city never stops — one teardown-and-rebuild every
                    // other decision, from cottages to terraces to towers, and
                    // towers that keep gaining floors.
                    let mut r = self.rng(0x0D9);
                    if self.redevelop(&mut r) {
                        built += 1;
                    }
                }
            }
            // The old core keeps its organic lanes (D36) and gets its street
            // wall by filling the gaps along them.
            if self.densify_core() {
                built += 1;
            }
            if built == 0 {
                let mut r = self.rng(0x0DA);
                self.redevelop(&mut r);
            }
            if hour % 4 == 0 {
                self.try_town_extras(days);
            }
            if hour % 24 == 0 {
                self.revert_unused_streets();
                self.reclaim();
            }
            return;
        }

        // D45: a worn footpath becomes a lane — that is how a village street
        // is born, so it gets a slightly wider budget than a random branch.
        if era == Era::Village && hour % 6 == 0 && self.worn.len() > 10 && self.street_budget_ok()
        {
            self.pave_worn_path();
        }
        if era == Era::Village && hour % 4 == 0 && self.try_extras(days) {
            return;
        }

        if houses < target {
            if self.grow_along_existing() {
                return;
            }
            // Only cut a new lane while the network is still thin (B26b item 1).
            if self.street_budget_ok() && self.new_branch() {
                return;
            }
        }
    }

    /// Civic buildings, shops, farms and the bridge: at most one per call.
    fn try_extras(&mut self, days: f64) -> bool {
        // Each candidate is *attempted*; a failure must not block the rest
        // (a church that cannot find room must not starve the farms forever).
        if self.lots.len() >= 10 && !self.has_kind(LotKind::Store) && self.place_civic(LotKind::Store)
        {
            return true;
        }
        if self.lots.len() >= 20 && !self.has_kind(LotKind::Church) && self.place_civic(LotKind::Church)
        {
            return true;
        }
        if self.lots.len() >= 16 && !self.has_kind(LotKind::Shop) && self.place_civic(LotKind::Shop) {
            return true;
        }
        if days > 4.0 && self.stats.farms < 2 && self.place_farm() {
            return true;
        }
        if days > 3.5 && self.stats.bridges == 0 && self.lots.len() >= 18 && self.plan_bridge() {
            return true;
        }
        false
    }

    /// Every tile a plot occupies: lanes must not be routed through them.
    fn lot_tiles(&self) -> BTreeSet<(i32, i32)> {
        let mut s = BTreeSet::new();
        for l in &self.lots {
            for ty in l.y..l.y + l.h {
                for tx in l.x..l.x + l.w {
                    s.insert((tx, ty));
                }
            }
        }
        s
    }

    fn has_kind(&self, k: LotKind) -> bool {
        self.lots.iter().any(|l| l.kind == k)
    }

    /// Put a house on a free plot next to an existing lane.
    ///
    /// B26b item 1: a street is laid to serve lots, so it gets as many houses as
    /// its frontage holds instead of one per street. D41 still holds — houses
    /// stand two tiles back from the kerb, so a street reads as yard + road +
    /// yard — and every plot keeps a tile of open ground all round.
    fn grow_along_existing(&mut self) -> bool {
        // B39/D62: before the first cart track there are no lanes — the village
        // grows along the *paths* the settlers wore instead (D45).
        let mut lanes = self.lane_tiles();
        if lanes.is_empty() && self.era() <= Era::Village {
            lanes = self.path_tiles();
        }
        if lanes.is_empty() {
            return false;
        }
        for attempt in 0..40 {
            let mut rng = self.rng(0x100 + attempt);
            let &(rx, ry) = &lanes[rng.usize(lanes.len())];
            // A street runs along one axis here; fill that row on one side.
            let along_x = self.map.at(rx - 1, ry).kind.is_lane()
                || self.map.at(rx + 1, ry).kind.is_lane();
            let along = if along_x { (1, 0) } else { (0, 1) };
            let side = if rng.chance(0.5) {
                (-along.1, along.0)
            } else {
                (along.1, -along.0)
            };
            // On a cramped land (few blocks open) the lanes the town has are all
            // the frontage there will ever be, so they get terraces rather than
            // one cottage each (B99c, D77).
            let kind = if self.cramped_land() && self.era() >= Era::Town {
                LotKind::Townhouse
            } else {
                LotKind::House
            };
            let placed = self.fill_row(rx, ry, along, side, kind, 6);
            if placed > 0 {
                return true;
            }
        }
        false
    }

    /// Stake up to `n` lots in a row fronting the street tile at `(rx, ry)` on
    /// one side: lots step `along` the street, each one two tiles out from the
    /// kerb. Returns how many were staked.
    fn fill_row(
        &mut self,
        rx: i32,
        ry: i32,
        along: (i32, i32),
        side: (i32, i32),
        kind: LotKind,
        n: i32,
    ) -> i32 {
        let mut placed = 0;
        for dir in [1, -1] {
            for i in 0..n {
                if dir == -1 && i == 0 {
                    continue; // the tile in front of the street is shared
                }
                // A plot is two tiles wide and keeps a tile of garden on each
                // side (D41), so the next one starts three tiles along.
                let step = i * dir * 3;
                let (x, y) = (
                    rx + along.0 * step + side.0 * 2,
                    ry + along.1 * step + side.1 * 2,
                );
                let salt = (x.rem_euclid(997) * 31 + y.rem_euclid(997)) as u64;
                let mut rng = self.rng(0x1400 + salt);
                if self.try_lot(x, y, kind, &mut rng) {
                    placed += 1;
                }
            }
        }
        placed
    }

    /// A new lane branching off the network (so it is always connected, D32).
    fn new_branch(&mut self) -> bool {
        let lanes = self.lane_tiles();
        if lanes.is_empty() {
            return false;
        }
        let mut rng = self.rng(0x200);
        // Keep the organic village compact (B26b): a branch starts from street
        // near the founding, so the old core stays a core and the grid has room.
        // The old core is a core: small enough that the planned town has room
        // around it (B26b item 3), big enough for a village of detached houses.
        let reach = (8.0 + self.days() * 0.8).min(16.0) as f32;
        let mut near: Vec<(i32, i32)> = lanes
            .iter()
            .copied()
            .filter(|&(x, y)| {
                let dx = x as f32 - self.founding.0 as f32;
                let dy = y as f32 - self.founding.1 as f32;
                dx * dx + dy * dy <= reach * reach
            })
            .collect();
        if near.is_empty() {
            near = lanes.clone();
        }
        // A village street grows from the **end** of the network, outward: a
        // lane cut across the middle of the village only demolishes houses
        // (B26b: 36 plots were cleared this way in ten days).
        let ends: Vec<(i32, i32)> = near
            .iter()
            .copied()
            .filter(|&(x, y)| {
                [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .filter(|(dx, dy)| self.map.at(x + dx, y + dy).kind.is_lane())
                    .count()
                    <= 1
            })
            .collect();
        let pool = if ends.is_empty() { &near } else { &ends };
        let &(sx, sy) = &pool[rng.usize(pool.len())];
        // Head away from the village centre, with a little wander (D36).
        let (fx, fy) = (self.founding.0 as f32, self.founding.1 as f32);
        let outward = (sy as f32 - fy).atan2(sx as f32 - fx);
        let bearing = outward + rng.range_f32(-0.7, 0.7);
        let len = rng.range(5, 9);
        let tx = sx as f32 + bearing.cos() * len as f32;
        let ty = sy as f32 + bearing.sin() * len as f32;
        // Keep the whole lane inside the village's reach: the old core is a
        // core, not a spider web over the whole map (B26b).
        let (fx, fy) = (self.founding.0 as f32, self.founding.1 as f32);
        let (dx, dy) = (tx - fx, ty - fy);
        let d = (dx * dx + dy * dy).sqrt().max(0.001);
        let k = (reach / d).min(1.0);
        let target = (
            (fx + dx * k) as i32,
            (fy + dy * k) as i32,
        );
        let target = (
            target.0.clamp(2, self.map.w - 3),
            target.1.clamp(2, self.map.h - 3),
        );
        let allow_water = self.time / 86400.0 > 4.0 && self.lots.len() >= 14;
        let noise_seed = rng.next_u64();
        let blocked = self.lot_tiles();
        let Some(path) = grid::route(&self.map, (sx, sy), target, allow_water, noise_seed, &blocked) else {
            if std::env::var("SIM_DEBUG").is_ok() {
                eprintln!("route failed {sx},{sy} -> {target:?} blocked={}", blocked.len());
            }
            return false;
        };
        if path.len() < 3 {
            if std::env::var("SIM_DEBUG").is_ok() {
                eprintln!("route too short {} {sx},{sy} -> {target:?}", path.len());
            }
            return false;
        }
        // A street keeps its verges and never demolishes plots (B26b item 1):
        // if the run would cross or hug one, this branch waits instead.
        let near: BTreeSet<(i32, i32)> = path
            .iter()
            .flat_map(|&(x, y)| [(x, y), (x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)])
            .collect();
        if self
            .lots
            .iter()
            .any(|l| (l.y..l.y + l.h).any(|ty| (l.x..l.x + l.w).any(|tx| path.contains(&(tx, ty)))))
        {
            return false;
        }
        let _ = &near;
        let era = self.era();
        // B26b item 2: the surface follows the era. A village lane is gravel;
        // from the town era a new street is asphalt with pavements.
        let kind = match era {
            Era::Settlement => Kind::Dirt,
            Era::Village => Kind::Gravel,
            _ => Kind::Paved,
        };
        self.lay_lane(&path, kind);
        self.stats.lanes += 1;
        self.events.push(Event {
            time: self.time,
            text: format!("lane laid: {} tiles", path.len()),
        });
        // Fill the frontage this street just opened, on both sides: a street
        // exists because lots are about to stand on it (B26b item 1).
        let mut placed = 0;
        let along_x = path[0].1 == path[path.len() - 1].1;
        let along = if along_x { (1, 0) } else { (0, 1) };
        for &(px, py) in path.iter().step_by(3) {
            if placed >= 8 {
                break;
            }
            for side in [(-along.1, along.0), (along.1, -along.0)] {
                let mut rng = self.rng(0x201 + (px.rem_euclid(997) * 31 + py.rem_euclid(997)) as u64);
                if self.try_lot(px + side.0 * 2, py + side.1 * 2, LotKind::House, &mut rng) {
                    placed += 1;
                }
            }
        }
        true
    }

    // --- lanes and grading -------------------------------------------

    /// The block grid (read-only), for the street class and the overlay.
    pub fn blocks(&self) -> &Blocks {
        &self.blocks
    }

    pub fn lane_tiles(&self) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        for y in 0..self.map.h {
            for x in 0..self.map.w {
                if self.map.at(x, y).kind.is_lane() {
                    v.push((x, y));
                }
            }
        }
        v
    }

    /// Lay a graded lane: one level per tile at most, flat across, with
    /// retaining walls or embankments wherever it cuts into or fills against the
    /// ground beside it (D34). Every tile is recorded as a change.
    fn lay_lane(&mut self, path: &[(i32, i32)], kind: Kind) {
        // B42f: a street that would run into the countryside belt under the
        // land line is not laid at all: half a street ending at an invisible
        // wall is worse than a network that stops a tile short (D32 still
        // holds — the caller's candidate simply fails and it picks another).
        if path.iter().any(|&(x, y)| self.behind_horizon(x, y)) {
            return;
        }
        // Nor across the airfield (B58/D73): a street over a runway is absurd.
        if path.iter().any(|&(x, y)| self.on_airfield(x, y)) {
            return;
        }
        // Nor over the railway's own ground: the corridor, its platforms and its
        // station hall (B58d).
        if path.iter().any(|&(x, y)| self.on_rail_ground(x, y)) {
            return;
        }
        // A lane never leaves a building standing on it (B41b). The planner's
        // own rule is "a plot on a street is cleared"; a lane laid *after* the
        // lot was staked used to leave the two overlapping until the next
        // decision, which `the_village_has_space_between_houses` caught. Clear
        // here, so the invariant holds at every instant (and never drop the
        // lane: that split the network).
        self.lots
            .retain(|l| !path.iter().any(|&(x, y)| l.covers(x, y)));
        // A street that joins another one takes the **joined** street's level at
        // that end and ramps to the ground from there, so no junction ever steps
        // more than one (D34/B26b: a connector met a village lane six levels up).
        let in_path: BTreeSet<(i32, i32)> = path.iter().copied().collect();
        let join_at = |sim: &Sim, p: (i32, i32)| -> Option<u8> {
            let mut best: Option<u8> = None;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let q = (p.0 + dx, p.1 + dy);
                if in_path.contains(&q) {
                    continue;
                }
                let t = sim.map.at(q.0, q.1);
                if t.kind.is_lane() {
                    best = Some(best.map_or(t.min_level(), |b: u8| b.min(t.min_level())));
                }
            }
            best
        };
        if path.is_empty() {
            return;
        }
        let head = join_at(self, path[0]);
        let tail = join_at(self, path[path.len() - 1]);
        let mut written: Vec<(i32, i32, u8)> = Vec::with_capacity(path.len());
        let mut levels: Vec<u8> = Vec::with_capacity(path.len());
        if head.is_none() {
            if let Some(t) = tail {
                // Grade from the far end back to the start.
                let mut lv = t;
                for (i, &(x, y)) in path.iter().enumerate().rev() {
                    let natural = self.map.at(x, y).min_level() as i32;
                    if i != path.len() - 1 {
                        lv = (lv as i32 + (natural - lv as i32).signum()).clamp(1, map::MAX_LEVEL as i32) as u8;
                    }
                    levels.push(lv);
                }
                levels.reverse();
                for (i, &(x, y)) in path.iter().enumerate() {
                    let mut t = self.map.at(x, y);
                    t.kind = kind;
                    t.corner = [levels[i]; 4];
                    t.prop = map::Prop::None;
                    t.aux &= !(map::AUX_STUMP);
                    self.write(x, y, t);
                    self.drop_field(x, y);
                    written.push((x, y, levels[i]));
                }
                for &(x, y, _) in &written {
                    self.remark_around(x, y);
                }
                self.stats.lane_tiles += written.len() as u32;
                // B61: a new lane is new work for the street crews.
                if !written.is_empty() {
                    self.resume_streetworks();
                }
                return;
            }
        }
        let mut level = head.unwrap_or_else(|| {
            self.map
                .at(path[0].0, path[0].1)
                .min_level()
                .clamp(1, map::MAX_LEVEL)
        });
        for &(x, y) in path {
            let natural = self.map.at(x, y).min_level() as i32;
            let diff = natural - level as i32;
            level = (level as i32 + diff.signum()).clamp(1, map::MAX_LEVEL as i32) as u8;
            let mut t = self.map.at(x, y);
            t.kind = kind;
            t.corner = [level; 4];
            t.prop = map::Prop::None;
            t.aux &= !(map::AUX_STUMP);
            self.write(x, y, t);
            self.drop_field(x, y);
            written.push((x, y, level));
        }
        for &(x, y, _) in &written {
            self.remark_around(x, y);
        }
        self.stats.lane_tiles += written.len() as u32;
        // B61: a new lane is new work for the street crews.
        if !written.is_empty() {
            self.resume_streetworks();
        }
    }

    /// Re-mark a tile and its four neighbours: grading one tile changes the
    /// step its neighbours see.
    pub(crate) fn remark_around(&mut self, x: i32, y: i32) {
        let level = self.map.at(x, y).min_level();
        self.mark_edge_works(x, y, level);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if self.map.in_bounds(nx, ny) {
                let l = self.map.at(nx, ny).min_level();
                self.mark_edge_works(nx, ny, l);
            }
        }
    }

    /// Compare this tile's edges with its neighbours and flag a wall or an
    /// embankment wherever the ground steps.
    fn mark_edge_works(&mut self, x: i32, y: i32, level: u8) {
        let mut t = self.map.at(x, y);
        for (edge, d) in [(0usize, (1, 0)), (1, (0, 1)), (2, (-1, 0)), (3, (0, -1))] {
            let n = self.map.at(x + d.0, y + d.1);
            let mine = level as i32;
            let theirs = n.min_level() as i32;
            let step = (mine - theirs).abs();
            let works = if step == 0 {
                0
            } else if step == 1 {
                1 // graded embankment
            } else {
                2 // retaining wall
            };
            map::set_edge_works(&mut t.aux, edge, works);
        }
        self.write(x, y, t);
    }

    /// Flatten a lot to one level (a graded pad) and record it.
    fn grade_lot(&mut self, x: i32, y: i32, w: i32, h: i32, level: u8) {
        for ty in y..y + h {
            for tx in x..x + w {
                if !self.map.in_bounds(tx, ty) {
                    continue;
                }
                let mut t = self.map.at(tx, ty);
                // A field the city has reached is ploughed back into grass
                // (D38: farms are swallowed as the town grows).
                if t.kind == Kind::Field {
                    t.kind = Kind::Grass;
                    self.drop_field(tx, ty);
                }
                t.corner = [level; 4];
                t.prop = map::Prop::None;
                self.write(tx, ty, t);
            }
        }
        for ty in y..y + h {
            for tx in x..x + w {
                self.remark_around(tx, ty);
            }
        }
    }

    /// Forget a ploughed field that something else has been built on.
    fn drop_field(&mut self, x: i32, y: i32) {
        self.fields
            .retain(|f| !(x >= f.x && x < f.x + f.w && y >= f.y && y < f.y + f.h));
        self.field_touch.push((x, y));
    }

    /// Try to stake a plot at `(x, y)` next to the lane network.
    pub fn lot_tries(&self) -> [u32; 8] {
        self.lot_reason
    }

    /// B66: is this pad on land the street network can reach? A plot stands on
    /// the founding landmass (where the streets begin) or on a landmass that
    /// already holds a street; and, once the world has any lane, within
    /// `nearest_lane`'s own reach of one. Before the first cart track there are
    /// no streets and the settlers' worn paths are frontage enough (D62) — but
    /// only where the town itself stands.
    fn road_reach_ok(&mut self, x: i32, y: i32, w: i32, h: i32, kind: LotKind, lane_fronted: bool) -> bool {
        let founding = self.founding;
        self.reach.ensure(&self.map, founding);
        let centre = (x + w / 2, y + h / 2);
        // Civic ground is exempt from the landmass rule (a park, a church or a
        // school has no cars of its own, and on a cramped island the only free
        // ground left may be across a strait — `every_land_grows_a_city` caught
        // exactly that on `islands`).
        let civic = matches!(
            kind,
            LotKind::Park | LotKind::Cemetery | LotKind::Hall | LotKind::Landmark(_)
        );
        if !civic && !self.reach.on_network_land(centre.0, centre.1) {
            return false;
        }
        // A plot that fronts a real lane is on the network by definition; one
        // that only fronts a worn footpath is staked within reach of a street —
        // or freely while the world has no lanes at all (D62: the paths come
        // first, and the village grows along them).
        //
        // The lane reach is asked of the *homes* only: a home is where the
        // traffic starts, so a home twenty tiles from any street is ten
        // villagers nobody can route. A church, a school or a park is reached on
        // foot along the paths, and on a cramped island it may be the only land
        // left — refusing it loses the land's civic buildings altogether.
        if matches!(kind, LotKind::House | LotKind::Townhouse | LotKind::Apartment) {
            lane_fronted || !self.reach.any_lane() || self.lane_within(centre, PLOT_LANE_REACH)
        } else {
            true
        }
    }

    fn try_lot(&mut self, x: i32, y: i32, kind: LotKind, rng: &mut Rng) -> bool {
        let (w, h) = match kind {
            // B26b: a plot is a plot — 2x2 (or a narrow 2x1) with a garden, so
            // the building reads at 1x and the land is more building than road.
            LotKind::House if rng.chance(0.30) => (2, 1),
            LotKind::Church => (3, 2),
            LotKind::Store | LotKind::Shop => (2, 2),
            LotKind::Barn => (2, 2),
            LotKind::Townhouse => (2, 2),
            LotKind::Apartment => (2, 2),
            LotKind::Office => (2, 2),
            LotKind::Tower => (3, 3),
            LotKind::Industry => (3, 2),
            LotKind::Park => (2, 2),
            LotKind::Cemetery => (3, 2),
            _ => (2, 2),
        };
        if x + w > self.map.w - 2 || y + h > self.map.h - 2 || x < 2 || y < 2 {
            self.lot_reason[0] += 1;
            return false;
        }
        // B58/D73: nothing is built on the airfield's ground — a house on a
        // runway is the kind of absurdity D73 exists to stop.
        if (0..h).any(|dy| (0..w).any(|dx| self.on_airfield(x + dx, y + dy))) {
            self.lot_reason[7 % 8] += 1;
            return false;
        }
        if kind != LotKind::Cemetery && self.near_cemetery(x, y, w, h) {
            self.lot_reason[5] += 1;
            return false;
        }
        // B42f: nothing in the belt under the land line — it would be cut by
        // the horizon. (x, y) is the tile nearest the back of the frame.
        if self.behind_horizon(x, y) {
            self.lot_reason[6] += 1;
            return false;
        }
        // Free of lanes, water, other lots and steep ground, and it has to front
        // a lane: the pad is levelled to *that* road's level so a lot never sits
        // above its street (B7 item 3).
        let mut frontage: Option<u8> = None;
        let mut front_dir = 0u8;
        // B66: a real lane as frontage, rather than a worn footpath.
        let mut lane_fronted = false;
        for ty in y..y + h {
            for tx in x..x + w {
                let t = self.map.at(tx, ty);
                if t.kind.is_water() || t.kind.is_lane() || t.kind == Kind::Sidewalk {
                    self.lot_reason[1] += 1;
                    return false;
                }
                if t.relief() > 2 {
                    self.lot_reason[2] += 1;
                    return false;
                }

                if self.lots.iter().any(|l| l.covers(tx, ty)) {
                    self.lot_reason[4] += 1;
                    return false;
                }
                // B49: a cell tower, a farm panel or a turbine stands here.
                if t.aux & map::AUX_TECH != 0 {
                    self.lot_reason[7] += 1;
                    return false;
                }
                // B49b: a wind turbine keeps a clear margin — no plot is staked
                // within two tiles of one, so the town never grows into the wind
                // farm (the turbine tile itself is reserved above).
                let too_close_to_turbine = self
                    .tech
                    .turbines
                    .iter()
                    .any(|&(gx, gy)| (gx - tx).abs() <= 2 && (gy - ty).abs() <= 2);
                if too_close_to_turbine {
                    self.lot_reason[7] += 1;
                    return false;
                }
                // D41: at least one tile of open ground all round.
                let blocked = self.lots.iter().any(|l| {
                    tx >= l.x - 1 && tx < l.x + l.w + 1 && ty >= l.y - 1 && ty < l.y + l.h + 1
                });
                if blocked {
                    self.lot_reason[5] += 1;
                    return false;
                }
            }
        }
        for ty in y - 2..=y + h + 1 {
            for tx in x - 2..=x + w + 1 {
                let on_edge = tx < x || tx >= x + w || ty < y || ty >= y + h;
                if !on_edge {
                    continue;
                }
                // B39 item 4: before the first cart track, a worn footpath is
                // frontage enough — there are no roads yet.
                let lane = self.map.at(tx, ty).kind.is_lane();
                let path = matches!(
                    map::WearStage::of(self.map.wear(tx, ty)),
                    map::WearStage::Footpath | map::WearStage::CartTrack
                );
                if !lane {
                    // A worn path is frontage only where it runs level with the
                    // plot: a path down a hillside is not a street the house can
                    // stand on (B40, `lots_never_sit_above_their_road`).
                    let level_ok =
                        (self.map.at(tx, ty).min_level() as i32
                            - self.map.at(x, y).max_level() as i32)
                            .abs()
                            <= 1;
                    if !(path && level_ok && self.era() <= Era::Village) {
                        continue;
                    }
                }
                // The plot itself is never on the lane (checked above); this
                // loop only finds which side the street is on.
                let road = self.map.at(tx, ty).min_level();
                lane_fronted |= lane;
                if frontage.is_none() {
                    front_dir = if tx >= x + w {
                        0
                    } else if ty >= y + h {
                        1
                    } else if tx < x {
                        2
                    } else {
                        3
                    };
                }
                frontage = Some(frontage.map_or(road, |f: u8| f.min(road)));
            }
        }
        let Some(level) = frontage else {
            self.lot_reason[6] += 1;
            return false;
        };
        // B66: no plot on land the streets cannot reach. An islet across the
        // water, or a sandbar twenty tiles from the nearest lane, gets no lots:
        // the villagers who live there can never be routed (islands seed 3).
        if !self.road_reach_ok(x, y, w, h, kind, lane_fronted) {
            self.lot_reason[2] += 1;
            return false;
        }

        // Earthwork is what makes lots look like slabs: refuse a plot that would
        // need more than one level of cut or fill anywhere on the pad.
        let mut earthwork = 0u32;
        for ty in y..y + h {
            for tx in x..x + w {
                let natural = self.map.at(tx, ty).min_level() as i32;
                earthwork += (natural - level as i32).unsigned_abs();
            }
        }
        if earthwork > (w * h * 2) as u32 {
            self.lot_reason[7] += 1;
            return false;
        }
        self.grade_lot(x, y, w, h, level);
        self.stats.dev_built += 1;
        let storeys = (kind.base_storeys().max(1) + rng.usize(2) as u8)
            .min(self.headroom_storeys(x, y).max(kind.base_storeys().min(2)));
        let style = self.district_at(x, y).style();
        self.lots.push(Lot {
            x,
            y,
            w,
            h,
            level,
            kind,
            storeys,
            style,
            seed: rng.next_u32(),
            front: front_dir,
            stage: Stage::Staked,
            stage_end: self.time + HOUSE_SECONDS / 6.0,
            started: self.time,
        });
        let i = self.lots.len() - 1;
        self.events.push(Event {
            time: self.time,
            text: format!("{:?} staked on a {w}x{h} pad at {x},{y}", self.lots[i].kind),
        });
        true
    }

    /// Is a plot of this size too close to a cemetery? Nothing may be built
    /// within two tiles of one (B48): the graveyard is not to be squeezed
    /// between flats as the town grows round it.
    pub(crate) fn near_cemetery(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.lots.iter().any(|c| {
            c.kind == LotKind::Cemetery
                && x < c.x + c.w + 2
                && x + w > c.x - 2
                && y < c.y + c.h + 2
                && y + h > c.y - 2
        })
    }

    /// A civic plot out at the edge of the street network: the lanes farthest
    /// from the founding (and, for a second one, from the first) are tried
    /// first, on gently sloping ground beside a road with open land all round
    /// (B48), so a graveyard sits on the outskirts rather than in the middle of
    /// the high street and the town then grows around it.
    fn place_civic_edge(&mut self, kind: LotKind) -> bool {
        let mut lanes = self.lane_tiles();
        if lanes.is_empty() {
            return false;
        }
        let (fx, fy) = self.founding;
        let others: Vec<(i32, i32)> = self
            .lots
            .iter()
            .filter(|l| l.kind == kind)
            .map(|l| (l.x, l.y))
            .collect();
        let score = |&(x, y): &(i32, i32)| -> i32 {
            let d0 = (x - fx).abs() + (y - fy).abs();
            // A further one wants to be far from the first as well.
            let apart = others.iter().map(|o| (x - o.0).abs() + (y - o.1).abs()).min().unwrap_or(d0);
            // Prefer the camera side (larger x + y): a graveyard behind the
            // town's tall blocks would be hidden from the wallpaper's view.
            apart + d0 + 2 * ((x + y) - (fx + fy))
        };
        lanes.sort_by_key(|t| std::cmp::Reverse(score(t)));
        let pool = (lanes.len() / 3).max(1);
        let (w, h) = (3, 2);
        for attempt in 0..160 {
            let mut rng = self.rng(0x340 + attempt);
            let &(rx, ry) = &lanes[rng.usize(pool)];
            let d = [(1, 0), (-1, 0), (0, 1), (0, -1)][rng.usize(4)];
            let (x, y) = (rx + d.0, ry + d.1);
            if !self.map.in_bounds(x, y) || !self.map.in_bounds(x + w, y + h) {
                continue;
            }
            // Gentle slope: no more than one level of fall across the plot,
            // and the first sixty tries want a visible tilt, not a table.
            let (mut lo, mut hi) = (u8::MAX, 0u8);
            for ty in y..y + h {
                for tx in x..x + w {
                    let l = self.map.at(tx, ty).min_level();
                    lo = lo.min(l);
                    hi = hi.max(l);
                }
            }
            if hi - lo > 1 || (attempt < 60 && hi == lo) {
                continue;
            }
            // Not on the shore: the coast is for the harbour and the lighthouse.
            let wet = (y - 4..y + h + 4).any(|ty| {
                (x - 4..x + w + 4).any(|tx| self.map.in_bounds(tx, ty) && self.map.at(tx, ty).is_water())
            });
            if wet {
                continue;
            }
            // Open land all round (three tiles, the street side excepted).
            let crowded = self.lots.iter().any(|l| {
                l.x < x + w + 3 && l.x + l.w > x - 3 && l.y < y + h + 3 && l.y + l.h > y - 3
            });
            if crowded {
                continue;
            }
            if self.try_lot(x, y, kind, &mut rng) {
                return true;
            }
        }
        false
    }

    fn place_civic(&mut self, kind: LotKind) -> bool {
        let lanes = self.lane_tiles();
        for attempt in 0..64 {
            let mut rng = self.rng(0x300 + attempt);
            if lanes.is_empty() {
                return false;
            }
            let &(rx, ry) = &lanes[rng.usize(lanes.len())];
            let d = [(1, 0), (-1, 0), (0, 1), (0, -1)][rng.usize(4)];
            if self.try_lot(rx + d.0, ry + d.1, kind, &mut rng) {
                return true;
            }
        }
        false
    }

    /// A planned crossing (B7 item 7): once the near bank is busy, find the
    /// nearest water next to the network, route a lane across it and mark the
    /// tiles a bridge deck.
    fn plan_bridge(&mut self) -> bool {
        let lanes = self.lane_tiles();
        if lanes.is_empty() {
            return false;
        }
        // A water tile within reach of the network, with water on one side and
        // dry land on the other.
        // (from lane, across to, cost)
        type Crossing = ((i32, i32), (i32, i32), i32);
        let mut best: Option<Crossing> = None;
        for &(lx, ly) in &lanes {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                for step in 1..6 {
                    let (wx, wy) = (lx + dx * step, ly + dy * step);
                    // Only rivers get bridges (D44/B21): a sea or a lake is a
                    // barrier, not a stream to hop.
                    if !self.map.at(wx, wy).is_water()
                        || self.map.water_kind(wx, wy) != map::WaterKind::River
                    {
                        continue;
                    }
                    // Not right at the sea: a deck there reads as a pier across
                    // open water (B21).
                    if let Some(o) = &self.map.ocean {
                        let near_sea =
                            o.side as f32 * ((wx - wy) as f32 - o.shore_p) > -4.0;
                        if near_sea {
                            continue;
                        }
                    }
                    // Look for land on the far side within 5 tiles.
                    for far in step + 1..step + 6 {
                        let (fx, fy) = (lx + dx * far, ly + dy * far);
                        if !self.map.in_bounds(fx, fy) {
                            continue;
                        }
                        if !self.map.at(fx, fy).is_water() && self.map.at(fx, fy).relief() <= 1 {
                            let cost = step + far - step;
                            if best.as_ref().map(|(_, _, c)| cost < *c).unwrap_or(true) {
                                best = Some(((lx, ly), (fx, fy), cost));
                            }
                        }
                    }
                }
            }
        }
        // No river beside the network yet: run a deliberate road toward the
        // nearest water first (B11 answer 1), then bridge it next decision.
        if best.is_none() {
            if self.aimed_at_river {
                return false;
            }
            let lanes2 = self.lane_tiles();
            let mut nearest: Option<((i32, i32), (i32, i32), i32)> = None;
            for y in 0..self.map.h {
                for x in 0..self.map.w {
                    if !self.map.at(x, y).is_water()
                        || self.map.water_kind(x, y) != map::WaterKind::River
                    {
                        continue;
                    }
                    for &(lx, ly) in lanes2.iter().step_by(7) {
                        let d = (lx - x).abs() + (ly - y).abs();
                        if d < 3 || d > 40 {
                            continue;
                        }
                        if nearest.as_ref().map(|(_, _, c)| d < *c).unwrap_or(true) {
                            nearest = Some(((lx, ly), (x, y), d));
                        }
                    }
                }
            }
            // Only search once: if there is no river within reach the scan is
            // expensive and pointless to repeat.
            self.aimed_at_river = true;
            if let Some((from, water, _)) = nearest {
                let mut rng = self.rng(0x710);
                let noise = rng.next_u64();
                let blocked = self.lot_tiles();
                if let Some(path) = grid::route(&self.map, from, water, false, noise, &blocked) {
                    // Only pave where the graded lane will sit within one level of the
        // ground beside it (D34) — otherwise the street would cut a step.
        {
            let mut level = self.map.at(path[0].0, path[0].1).min_level() as i32;
            let mut ok = true;
            for &(x, y) in &path {
                let natural = self.map.at(x, y).min_level() as i32;
                level += (natural - level).signum();
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let nb = self.map.at(x + dx, y + dy).min_level() as i32;
                    if (level - nb).abs() > 1 {
                        ok = false;
                    }
                }
            }
            if !ok {
                return false;
            }
        }
        self.lay_lane(&path, Kind::Dirt);
                    self.remark_around(path[0].0, path[0].1);
                    self.aimed_at_river = true;
                    self.events.push(Event {
                        time: self.time,
                        text: "road aimed at the river".into(),
                    });
                    return true;
                }
            }
            return false;
        }
        let Some((from, to, _)) = best else {
            return false;
        };
        let mut rng = self.rng(0x700);
        let noise = rng.next_u64();
        let Some(path) = grid::route(&self.map, from, to, true, noise, &BTreeSet::new()) else {
            return false;
        };
        // A deck may only cross a river: if the route wanders over a sea or a
        // lake (B21) there is no bridge this time.
        if path.iter().any(|&(x, y)| {
            self.map.at(x, y).is_water()
                && self.map.water_kind(x, y) != map::WaterKind::River
        }) {
            return false;
        }
        // Grade the approach and lay the deck: one level above the water.
        let mut level = self.map.at(from.0, from.1).min_level();
        let mut crossed = 0;
        for &(x, y) in &path {
            let natural = self.map.at(x, y).min_level() as i32;
            level = (level as i32 + (natural - level as i32).signum()).clamp(1, map::MAX_LEVEL as i32) as u8;
            let mut t = self.map.at(x, y);
            if t.is_water() {
                t.kind = Kind::Bridge;
                t.aux |= map::AUX_BRIDGE;
                t.corner = [level; 4];
                crossed += 1;
            } else {
                t.kind = Kind::Dirt;
                t.corner = [level; 4];
            }
            t.prop = map::Prop::None;
            self.write(x, y, t);
            self.remark_around(x, y);
        }
        let near: BTreeSet<(i32, i32)> = path
            .iter()
            .flat_map(|&(x, y)| [(x, y), (x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)])
            .collect();
        self.lots
            .retain(|l| !(l.y..l.y + l.h).any(|ty| (l.x..l.x + l.w).any(|tx| near.contains(&(tx, ty)))));
        if crossed == 0 {
            // The route found a way round: no bridge this time.
            return false;
        }
        self.stats.bridges += 1;
        self.events.push(Event {
            time: self.time,
            text: format!("bridge planned {} tiles at {from:?}", path.len()),
        });
        true
    }

    /// A farm on the outskirts: a ploughed field plus a barn.
    fn place_farm(&mut self) -> bool {
        let mut rng = self.rng(0x400);
        for _ in 0..128 {
            // B11 answer 2: farms live in a ring outside the village.
            let ang = rng.range_f32(0.0, std::f32::consts::TAU);
            let dist = rng.range_f32(12.0, 20.0);
            let x = (self.founding.0 as f32 + ang.cos() * dist) as i32;
            let y = (self.founding.1 as f32 + ang.sin() * dist) as i32;
            if x < 3 || y < 3 || x > self.map.w - 8 || y > self.map.h - 8 {
                continue;
            }
            let mut ok = true;
            for ty in y..y + 4 {
                for tx in x..x + 4 {
                    let t = self.map.at(tx, ty);
                    if t.kind.is_water()
                        || t.kind.is_lane()
                        || t.relief() > 1
                        // B13 item 4: fields belong on gentle grass, not on a
                        // rocky bluff or a scree slope.
                        || t.kind.is_rocky()
                        || t.max_level() > 7
                        || self.lots.iter().any(|l| l.covers(tx, ty))
                    {
                        ok = false;
                    }
                }
            }
            if !ok {
                continue;
            }
            for ty in y..y + 4 {
                for tx in x..x + 4 {
                    let mut t = self.map.at(tx, ty);
                    t.kind = Kind::Field;
                    t.prop = map::Prop::None;
                    let l = t.min_level();
                    t.corner = [l; 4];
                    self.write(tx, ty, t);
                    self.remark_around(tx, ty);
                }
            }
            self.fields.push(Field { x, y, w: 4, h: 4 });
            self.stats.farms += 1;
            self.events.push(Event {
                time: self.time,
                text: format!("farm ploughed at {x},{y}"),
            });
            return true;
        }
        false
    }

    /// Apply any paving whose scheduled time has come.
    fn apply_upgrades(&mut self) {
        if self.upgrade_due.is_empty() {
            return;
        }
        let now = self.time;
        let due: Vec<(i32, i32)> = self
            .upgrade_due
            .iter()
            .filter(|(_, t)| *t <= now)
            .map(|(p, _)| *p)
            .collect();
        self.upgrade_due.retain(|(_, t)| *t > now);
        for (x, y) in due {
            let mut t = self.map.at(x, y);
            if !t.kind.is_lane() {
                continue;
            }
            t.kind = match t.kind {
                // From the town era a lane goes straight to asphalt: a town
                // street is paved, not a gravel track (B26b item 2).
                Kind::Dirt if self.era() >= Era::Town => Kind::Paved,
                Kind::Dirt => Kind::Gravel,
                _ => Kind::Paved,
            };
            self.write(x, y, t);
            self.remark_around(x, y);
            if t.kind == Kind::Paved {
                self.stats.upgraded += 1;
            }
            // B61: a street that just became asphalt is new work (paint, lamps).
            self.resume_streetworks();
        }
    }

    /// The box that holds every finished building — the built-up area the
    /// brief's paving and road-share figures are measured over.
    fn built_box(&self) -> (i32, i32, i32, i32) {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for l in &self.lots {
            if !l.finished() || l.kind == LotKind::Park {
                continue;
            }
            x0 = x0.min(l.x);
            y0 = y0.min(l.y);
            x1 = x1.max(l.x + l.w - 1);
            y1 = y1.max(l.y + l.h - 1);
        }
        if x0 > x1 {
            let (fx, fy) = self.founding;
            return (fx - 4, fy - 4, fx + 4, fy + 4);
        }
        (x0.max(1), y0.max(1), x1.min(self.map.w - 2), y1.min(self.map.h - 2))
    }

    /// B26b item 1: a dead-end lane that no lot uses within reach is taken up
    /// again. Only village surfaces are reverted (a town keeps its grid), only
    /// tiles at the **tip** of the network are touched, so the network stays one
    /// piece (D32), and the ground goes back to grass with its wear cleared.
    fn revert_unused_streets(&mut self) -> bool {
        if self.era() >= Era::Metropolis {
            return false;
        }
        let lane: BTreeSet<(i32, i32)> = self.lane_tiles().into_iter().collect();
        let nbrs = |x: i32, y: i32| {
            [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .filter(|(dx, dy)| lane.contains(&(x + dx, y + dy)))
                .count()
        };
        // A lane serves a lot up to three tiles away — that is the radius a
        // plot is staked on — so a street with a forecourt or a house three
        // tiles off it is in use, not a dead end (B59: a car wash must not end
        // up on a road that is then taken up).
        let used = |lots: &Vec<Lot>, x: i32, y: i32| {
            lots.iter().any(|l| {
                x >= l.x - 3 && x < l.x + l.w + 3 && y >= l.y - 3 && y < l.y + l.h + 3
            })
        };
        let tips: Vec<(i32, i32)> = lane
            .iter()
            .copied()
            .filter(|&(x, y)| nbrs(x, y) <= 1 && !used(&self.lots, x, y))
            .filter(|&(x, y)| {
                // Never take up a bridge deck or an asphalt street.
                matches!(self.map.at(x, y).kind, Kind::Dirt | Kind::Gravel)
            })
            .collect();
        if tips.is_empty() {
            return false;
        }
        let mut reverted = 0;
        for (sx, sy) in tips {
            let mut path = vec![(sx, sy)];
            let mut cur = (sx, sy);
            for _ in 0..12 {
                let next = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .map(|(dx, dy)| (cur.0 + dx, cur.1 + dy))
                    .find(|p| lane.contains(p) && !path.contains(p) && !used(&self.lots, p.0, p.1));
                match next {
                    Some(n) => {
                        // Only take up a chain of unused lane, never a junction.
                        if nbrs(n.0, n.1) > 2 {
                            break;
                        }
                        if !matches!(self.map.at(n.0, n.1).kind, Kind::Dirt | Kind::Gravel) {
                            break;
                        }
                        path.push(n);
                        cur = n;
                    }
                    None => break,
                }
            }
            // Leave a single-tile spur alone if it is the way to something.
            for (x, y) in path {
                let mut t = self.map.at(x, y);
                t.kind = Kind::Grass;
                self.write(x, y, t);
                self.map.wear[(y * self.map.w + x) as usize] = 0;
                self.worn.remove(&(x, y));
                reverted += 1;
            }
        }
        if reverted > 0 {
            self.events.push(Event {
                time: self.time,
                text: format!("{reverted} tiles of unused lane went back to grass"),
            });
        }
        reverted > 0
    }

    /// Street tiles and lot tiles, for the road-share budget.
    pub fn street_ratio(&self) -> (i32, i32) {
        let mut road = 0;
        for y in 0..self.map.h {
            for x in 0..self.map.w {
                let k = self.map.at(x, y).kind;
                if k.is_lane() || k == Kind::Sidewalk {
                    road += 1;
                }
            }
        }
        let lots: i32 = self.lots.iter().map(|l| l.w * l.h).sum();
        (road, lots)
    }

    /// B26b item 1: a street is only laid while the network is thinner than the
    /// budget. The measure is **road against the built-up area** — the box the
    /// city covers, which counts the gardens, parks and open ground the lanes
    /// were burying (the complaint the brief was written from: "dirt/gravel
    /// lanes cover more ground than buildings and grass"). Growth is then forced
    /// to serve lots that are about to be built.
    pub(crate) fn street_budget_ok(&self) -> bool {
        let (x0, y0, x1, y1) = self.built_box();
        let area = ((x1 - x0 + 1) * (y1 - y0 + 1)).max(1) as f32;
        let (road, _) = self.street_ratio();
        let target = if self.era() >= Era::Town { 0.26 } else { 0.22 };
        road as f32 <= target * area
    }

    /// The brief's road share: street tiles as a share of the ground the city
    /// covers (B26 was 63-81 % of the built area).
    pub fn road_share(&self) -> f32 {
        let (x0, y0, x1, y1) = self.built_box();
        let area = ((x1 - x0 + 1) * (y1 - y0 + 1)).max(1) as f32;
        let (road, _) = self.street_ratio();
        road as f32 / area
    }

    /// Paving follows the era (B26b item 2). In the village only the high
    /// street near the square is asphalted (B11); from the town era every
    /// street inside the built-up area is paved, nearest the core first, a few
    /// tiles per decision so it stays a visible job (D33).
    fn pave_works(&mut self) -> bool {
        if self.era() < Era::Town {
            return self.upgrade_some();
        }
        if self.upgrade_due.len() > 60 {
            return false; // a paving crew is already working through a backlog
        }
        let (x0, y0, x1, y1) = self.built_box();
        let (fx, fy) = (self.founding.0, self.founding.1);
        let mut cand: Vec<(i32, i32, i32)> = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let t = self.map.at(x, y);
                if !matches!(t.kind, Kind::Dirt | Kind::Gravel) {
                    continue;
                }
                cand.push(((x - fx) * (x - fx) + (y - fy) * (y - fy), x, y));
            }
        }
        if cand.is_empty() {
            return false;
        }
        cand.sort_unstable();
        let mut at = self.time;
        for &(_, x, y) in cand.iter().take(4) {
            at += 40.0; // 40 s per tile: a street takes minutes, not one frame
            self.upgrade_due.push(((x, y), at));
        }
        true
    }

    /// Schedule the next stretch of street for its upgrade (B7 item 6): the
    /// work walks along the lane, 40 s apart, so it is visible as a job.
    fn upgrade_some(&mut self) -> bool {
        if self.upgrade_queue.is_empty() {
            self.upgrade_queue = self.lane_tiles();
            if self.upgrade_queue.is_empty() {
                return false;
            }
        }
        // B11: a village is mostly dirt and gravel. Only the main street near
        // the founding square gets asphalt by day 8.
        let paved = self.map.tiles.iter().filter(|t| t.kind == Kind::Paved).count() as i32;
        let lanes = self.lane_tiles().len() as i32;
        if paved * 100 >= lanes * 22 {
            return true; // enough asphalt for this era
        }
        let mut done = 0;
        let mut at = self.time;
        while done < 3 && !self.upgrade_queue.is_empty() {
            let (x, y) = self.upgrade_queue.remove(0);
            let t = self.map.at(x, y);
            // Never pave a bridge deck (B11: the deck disappeared under asphalt).
            if !t.kind.is_lane() || t.kind == Kind::Paved || t.kind == Kind::Bridge {
                continue;
            }
            // Only lanes near the middle of the village are candidates.
            let dx = x as f32 - self.founding.0 as f32;
            let dy = y as f32 - self.founding.1 as f32;
            if dx * dx + dy * dy > 10.0 * 10.0 {
                continue;
            }
            at += 40.0; // 40 s per tile: a short street takes several minutes
            self.upgrade_due.push(((x, y), at));
            done += 1;
        }
        done > 0
    }

    // --- tile writing and the change feed ------------------------------

    pub(crate) fn write(&mut self, x: i32, y: i32, tile: Tile) {
        if !self.map.in_bounds(x, y) {
            return;
        }
        // D62/D65: the first paved street is an event, told once.
        if !self.firsts.first_paved
            && tile.kind == map::Kind::Paved
            && self.map.at(x, y).kind != map::Kind::Paved
        {
            self.firsts.first_paved = true;
            self.push_story_t(chronicle::Topic::First, 0, "the first street is paved: its asphalt is still warm at the edge");
        }
        // B66: the reach table follows the map — a tile that turns into water
        // (a flood, a crater) or a tile that becomes (or stops being) a lane
        // changes what the streets can reach.
        let old = self.map.at(x, y);
        if old.is_water() != tile.is_water() || old.kind.is_lane() != tile.kind.is_lane() {
            self.reach.invalidate();
        }
        let mut tile = tile;
        if tile.kind != map::Kind::Field {
            // B80: the crop bits belong to a field tile only.
            tile.aux &= !map::AUX_CROP_MASK;
        }
        self.map.set(x, y, tile);
        self.overrides.insert((x, y), tile);
        self.changes.insert((x, y));
    }

    /// Tiles changed since the last call (the renderer dirties only these).
    /// Record a tile as walked on (the village paves the busiest paths, B39).
    pub(crate) fn note_worn(&mut self, x: i32, y: i32) {
        if self.map.in_bounds(x, y) {
            self.worn.insert((x, y));
        }
    }

    /// Mark a tile as changed for the renderer (and the save's dirty set).
    pub(crate) fn mark_changed(&mut self, x: i32, y: i32) {
        self.changes.insert((x, y));
    }

    pub fn take_changes(&mut self) -> Vec<(i32, i32)> {
        std::mem::take(&mut self.changes).into_iter().collect()
    }

    /// Stable hash of everything that defines the city, for determinism tests.
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x1000_0000_01b3);
        };
        mix(self.seed);
        mix(self.time.to_bits());
        for (&(x, y), t) in &self.overrides {
            mix(x as u64);
            mix(y as u64);
            mix(t.corner[0] as u64 | (t.corner[1] as u64) << 8 | (t.corner[2] as u64) << 16 | (t.corner[3] as u64) << 24);
            mix(t.kind as u64 | (t.aux as u64) << 8);
        }
        mix(self.ending.kind.id() as u64 | (self.ending.world_days.to_bits() << 8));
        mix(self.ending.cur[0] as u64 | (self.ending.cur[1] as u64) << 20 | (self.ending.cur[3] as u64) << 40);
        for l in &self.lots {
            mix(l.x as u64);
            mix(l.y as u64);
            mix(l.w as u64 * 3 + l.h as u64 * 5);
            mix(lot_kind_code(l.kind) as u64 * 9 + l.stage as u64);
            mix(l.seed as u64);
            mix(l.front as u64);
        }
        for f in &self.fields {
            mix(f.x as u64);
            mix(f.y as u64);
        }
        // B43: the residents are part of what the city *is*.
        for p in &self.people.list {
            mix(p.first as u64 | (p.last as u64) << 8 | (p.age as u64) << 16);
            mix(p.job_kind.to_u8() as u64 | (p.tone as u64) << 4);
            mix(p.born as u32 as u64 | (p.life.to_u8() as u64) << 32);
            for (a, b) in [p.home, p.job].into_iter().flatten() {
                mix(a as u64 | (b as u64) << 16);
            }
        }
        // B48: the pending scenes are part of what the city is going to do.
        for sc in &self.people.scenes {
            mix(sc.start.to_bits());
            mix(sc.kind.to_u8() as u64 | (sc.told as u64) << 8);
        }
        // B48a/D68: the name and the story are part of what the city is.
        mix(self.biome.id() as u64);
        for b in self.city_name.as_bytes() {
            mix(*b as u64);
        }
        mix(self.story.hash());
        mix(self.chronicle.hash());
        // B49: the technology path and what the crews have installed. The crews
        // themselves are mid-job state, not part of what the city *is*; the
        // sets are mixed in their sorted order so a reload hashes the same.
        for s in &self.tech.path {
            mix(s.advance.index() as u64 | (s.at.to_bits() & 0xFFFF));
        }
        for (i, d) in self.tech.done.iter().enumerate() {
            mix(*d as u64 | (i as u64) << 1);
        }
        for (&(x, y), &l) in &self.tech.lamps {
            mix((x as u64) << 40 ^ (y as u64) << 16 ^ l as u64);
        }
        for set in [
            &self.tech.poles,
            &self.tech.buried,
            &self.tech.cells,
            &self.tech.chargers,
            &self.tech.farm,
        ] {
            mix(set.len() as u64);
            for &(x, y) in set {
                mix(x as u64 | (y as u64) << 20);
            }
        }
        for &(x, y) in &self.tech.towers {
            mix(x as u64 | (y as u64) << 20 | 1 << 44);
        }
        for &(x, y) in &self.tech.turbines {
            mix(x as u64 | (y as u64) << 20 | 2 << 44);
        }
        for (&(x, y), &s) in &self.tech.solar {
            mix(x as u64 | (y as u64) << 20 | (s as u64) << 45);
        }
        mix(
            self.tech.catenary as u64
                | (self.tech.drones as u64) << 1
                | (self.tech.maglev as u64) << 2,
        );
        // B52: the economy, the government and the incidents.
        let c = &self.civics;
        mix(c.mood.to_bits() as u64 | (c.vacancy.to_bits() as u64) << 32);
        mix(c.smog.to_bits() as u64 | (c.cleanups as u64) << 32);
        for co in &c.companies {
            mix(co.a as u64 | (co.b as u64) << 8 | (co.kind as u64) << 16 | (co.floors as u64) << 24 | (co.closed as u64) << 32);
            mix(co.hq.0 as u32 as u64 | (co.hq.1 as u32 as u64) << 32);
        }
        if let Some(m) = c.mayor {
            mix(m.id as u64 | (m.slate as u64) << 32 | (m.since_day as u32 as u64) << 40);
        }
        mix(c.next_election as u32 as u64 | (c.elections_held as u64) << 32 | (c.approved as u64) << 40);
        mix(c.next_fire as u32 as u64 | (c.next_blackout as u32 as u64) << 32);
        h
    }
}

#[cfg(test)]
mod tests {
    /// A non-finite or negative interval must neither hang nor stall the sim.
    #[test]
    fn advance_ignores_nan_inf_and_negative_intervals() {
        let mut a = Sim::new(3, Pace::Month, 41, 41);
        a.advance(100.0);
        let t = a.time;
        a.advance(f64::INFINITY);
        a.advance(f64::NAN);
        a.advance(-50.0);
        assert_eq!(a.time, t);
        assert!(a.acc.is_finite());
        a.advance(100.0);
        assert!(a.time > t, "the sim stalled after a bad interval");
    }

    use super::*;

    /// D45: the same seed and time must produce the same wear.
    #[test]
    fn wear_is_deterministic() {
        let mut a = sim(9);
        let mut b = sim(9);
        a.run_to(4.0 * 86400.0);
        b.run_to(4.0 * 86400.0);
        assert_eq!(a.map.wear, b.map.wear, "wear differs between identical runs");
    }

    /// D45: villagers wear footpaths, and the planner paves some of them.
    #[test]
    fn desire_paths_appear_and_become_lanes() {
        let mut a = sim(9);
        a.run_to(8.0 * 86400.0);
        let footpath = a
            .map
            .wear
            .iter()
            .filter(|w| map::WearStage::of(**w) >= map::WearStage::Footpath)
            .count();
        assert!(footpath > 10, "only {footpath} footpath tiles after 8 days");
        assert!(a.stats.paths_paved > 0, "no footpath was ever paved");
        // D32: the network is one component even with paths feeding it.
        let lanes: Vec<(i32, i32)> = a.lane_tiles().into_iter().collect();
        assert!(!lanes.is_empty());
        let mut seen = std::collections::BTreeSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(lanes[0]);
        seen.insert(lanes[0]);
        let set: BTreeSet<(i32, i32)> = lanes.iter().copied().collect();
        while let Some((x, y)) = queue.pop_front() {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = (x + dx, y + dy);
                if set.contains(&n) && seen.insert(n) {
                    queue.push_back(n);
                }
            }
        }
        assert_eq!(seen.len(), set.len(), "the street network split into pieces");
        // D34: every lane is graded to a walkable step.
        for &(x, y) in &lanes {
            assert!(a.map.at(x, y).relief() <= 1, "lane at {x},{y} is not graded");
        }
    }

    /// D45: a long catch-up and many small advances must agree, wear included.
    #[test]
    fn catch_up_matches_live_wear() {
        let mut a = sim(4);
        let mut b = sim(4);
        // B40: the settlement ticks at 2 s and the city at 20 s, and `advance`
        // carries the remainder between calls, so the two runs are compared at
        // the same *instant* (b's), not at the same nominal day.
        while b.time < 5.0 * 86400.0 {
            b.advance(600.0);
        }
        a.run_to(b.time);
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a.map.wear, b.map.wear);
    }

    fn sim(seed: u64) -> Sim {
        Sim::new(seed, Pace::Month, 71, 71)
    }

    #[test]
    fn deterministic_for_the_same_seed_and_time() {
        let mut a = sim(1);
        let mut b = sim(1);
        a.run_to(3.0 * 86400.0);
        b.run_to(3.0 * 86400.0);
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a.lots.len(), b.lots.len());
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = sim(1);
        let mut b = sim(2);
        a.run_to(2.0 * 86400.0);
        b.run_to(2.0 * 86400.0);
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn the_city_grows() {
        let mut a = sim(1);
        let early = a.lots.len();
        a.run_to(8.0 * 86400.0);
        assert!(a.lots.len() > early + 10, "only {} lots after 8 days", a.lots.len());
        assert!(a.lane_tiles().len() > 10, "the lane network did not grow");
    }

    /// B41b: the settlement ends with the settlement — tents down, stumps out,
    /// log piles used up, the party gone, and nothing settler-related drawn on
    /// a street or a lot.
    #[test]
    fn the_camp_ends_with_the_settlement() {
        for seed in 1..=5u64 {
            let mut a = sim(seed);
            a.run_to(1.0 * 86400.0);
            assert!(a.settlers.struck, "seed {seed}: the camp was never struck");
            assert!(a.camp().is_none(), "seed {seed}: tents still standing");
            assert!(a.log_piles().is_empty(), "seed {seed}: log piles left");
            assert!(
                a.settlers_at(a.smooth_time()).is_empty(),
                "seed {seed}: settlers still walking"
            );
            // And while the camp is still up, nothing of it is drawn on a
            // street or a lot.
            let mut b = sim(seed);
            b.run_to(0.02 * 86400.0);
            if let Some(c) = b.camp() {
                let (fx, fy) = b.founding;
                for (dx, dy) in c.tents.iter().copied() {
                    let t = b.map.at(fx + dx, fy + dy);
                    assert!(!t.kind.is_lane(), "seed {seed}: a tent on a street");
                    assert!(
                        !b.lots.iter().any(|l| l.covers(fx + dx, fy + dy)),
                        "seed {seed}: a tent inside a lot"
                    );
                }
            }
            for p in b.log_piles() {
                assert!(!b.map.at(p.x, p.y).kind.is_lane(), "a pile on a street");
            }
        }
    }

    /// B41b: a loaded save is a *city*, not a settlement. `Sim::load` builds on
    /// `Sim::new`, which spawns a party — a load must not walk a camp in.
    #[test]
    fn a_loaded_city_has_no_camp() {
        let dir = std::env::temp_dir().join("citylive-b41b");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("camp.sav");
        let mut a = sim(1);
        a.run_to(2.0 * 86400.0);
        a.save(&path).unwrap();
        let b = Sim::load(&path).unwrap();
        assert!(b.settlers.struck, "a loaded save grew a camp");
        assert!(b.camp().is_none());
        assert!(b.settlers_at(b.smooth_time()).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// B41/D63 rule 1: the network is one piece, on every seed and at every
    /// stage the brief names.
    #[test]
    fn road_network_is_one_piece_on_every_seed() {
        for seed in 1..=5u64 {
            for day in [8.0, 30.0, 90.0] {
                // A small map: the rules are local, and 15 long runs at 71x71
                // cost four minutes.
                let mut a = Sim::new(seed, Pace::Month, 41, 41);
                a.run_to(day * 86400.0);
                a.tidy_roads();
                assert!(
                    grid::network_connected(&a.map),
                    "seed {seed} day {day}: the lane network split"
                );
            }
        }
    }

    /// B41/D63 rule 2: a junction is a T or a 4-way, never a diagonal-only
    /// touch (two lanes meeting at a corner with no shared street tile).
    #[test]
    fn junctions_are_never_diagonal_only() {
        for seed in 1..=5u64 {
            for day in [8.0, 30.0, 90.0] {
                let mut a = Sim::new(seed, Pace::Month, 41, 41);
                a.run_to(day * 86400.0);
                a.tidy_roads();
                for y in 1..a.map.h - 1 {
                    for x in 1..a.map.w - 1 {
                        if !a.map.at(x, y).kind.is_lane() {
                            continue;
                        }
                        for (dx, dy) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
                            let (nx, ny) = (x + dx, y + dy);
                            if !a.map.at(nx, ny).kind.is_lane() {
                                continue;
                            }
                            let joined = a.map.at(x + dx, y).kind.is_lane()
                                || a.map.at(x, y + dy).kind.is_lane();
                            assert!(
                                joined,
                                "seed {seed} day {day}: diagonal-only touch at {x},{y} and {nx},{ny}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// B41/D63 rule 3: no sliver — a one-tile hole inside the street network.
    #[test]
    fn no_slivers_in_the_street_network() {
        for seed in 1..=5u64 {
            for day in [8.0, 30.0, 90.0] {
                let mut a = Sim::new(seed, Pace::Month, 41, 41);
                a.run_to(day * 86400.0);
                a.tidy_roads();
                for y in 1..a.map.h - 1 {
                    for x in 1..a.map.w - 1 {
                        let t = a.map.at(x, y);
                        if t.kind.is_lane() || t.is_water() || t.kind.is_worked() {
                            continue;
                        }
                        let n = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                            .iter()
                            .filter(|(dx, dy)| a.map.at(x + dx, y + dy).kind.is_lane())
                            .count();
                        // A hole that could be laid is a defect; one holding a
                        // lot, water or a cliff is the ground the street bends
                        // around.
                        if n >= 3 {
                            let buildable = !a.lots.iter().any(|l| l.covers(x, y))
                                && t.relief() <= 1;
                            assert!(
                                !buildable,
                                "seed {seed} day {day}: a {n}-sided sliver at {x},{y}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// B55: signs only where a real town has them.
    #[test]
    fn signs_follow_the_placement_rules() {
        let mut a = sim(1);
        a.run_to(30.0 * 86400.0);
        let paved: Vec<(i32, i32)> = a
            .lane_tiles()
            .into_iter()
            .filter(|&(x, y)| a.road_class(x, y).is_paved())
            .collect();
        let plan = a.sign_placements(&paved);
        let sw = a.streetworks();
        let blades: Vec<_> = sw.signs.iter().filter(|s| s.2 == 0).collect();
        let stops: Vec<_> = sw.signs.iter().filter(|s| s.2 == 1).collect();
        // Every sign is one the rules allow, on a paved street, none on tracks.
        for &(x, y, k) in &sw.signs {
            assert_eq!(plan.get(&(x, y)), Some(&k), "unplanned sign at {x},{y}");
            assert!(a.road_class(x, y).is_paved(), "sign off a paved street {x},{y}");
        }
        // One blade per junction: no two blades touch.
        for (i, p) in blades.iter().enumerate() {
            for q in &blades[i + 1..] {
                assert!((p.0 - q.0).abs() > 1 || (p.1 - q.1).abs() > 1, "two blades at one junction");
            }
        }
        // A stop sign sits on a minor approach, never beside a traffic-light junction.
        let rank = |c: roads::RoadClass| match c {
            roads::RoadClass::Local => 1,
            roads::RoadClass::Collector => 2,
            _ => 3,
        };
        for &&(x, y, _) in &stops {
            let c = a.road_class(x, y);
            let busier = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| {
                let (nx, ny) = (x + dx, y + dy);
                a.map.at(nx, ny).kind.is_lane()
                    && a.lane_degree(nx, ny) >= 3
                    && rank(a.road_class(nx, ny)) > rank(c).min(2)
                    || (a.lane_degree(nx, ny) >= 3 && rank(a.road_class(nx, ny)) >= rank(c))
            });
            assert!(busier, "stop sign at {x},{y} not at a junction");
            assert!(!sw.lights.contains(&(x, y)));
        }
        assert!(stops.len() <= 2 * blades.len(), "{} stops vs {} blades", stops.len(), blades.len());
    }

    #[test]
    fn legacy_signs_are_thinned() {
        let mut a = sim(1);
        a.run_to(30.0 * 86400.0);
        let before = a.streetworks().signs.len();
        // An old save: a blade on every lane tile of the town, plus on a track.
        for (x, y) in a.lane_tiles() {
            a.streetworks.signs.push((x, y, 0));
        }
        a.thin_signs();
        assert!(a.streetworks().signs.len() <= before + 8, "not thinned: {} vs {before}", a.streetworks().signs.len());
        for &(x, y, _) in &a.streetworks().signs {
            assert!(a.road_class(x, y).is_paved());
        }
    }

    /// B41/D63 rule 4: every lane has a class, avenues sit on the grid's avenue
    /// lines, and only paved streets get painted.
    #[test]
    fn road_classes_and_furniture_are_consistent() {
        let mut a = sim(1);
        a.run_to(30.0 * 86400.0);
        for (x, y) in a.lane_tiles() {
            let c = a.road_class(x, y);
            assert!(c.is_vehicle(), "lane at {x},{y} has class {c:?}");
            if c == roads::RoadClass::Avenue {
                assert!(a.on_avenue_line(x, y), "avenue off the grid at {x},{y}");
            }
        }
        // Paint only ever lands on a paved street.
        for &(x, y) in &a.streetworks().paint {
            assert!(
                a.road_class(x, y).is_paved(),
                "paint on a non-paved tile at {x},{y}"
            );
        }
        // And by day 30 the town has painted most of its streets.
        assert!(
            a.paint_progress() > 0.5,
            "only {:.0}% of the streets painted by day 30",
            a.paint_progress() * 100.0
        );
    }

    #[test]
    fn road_graph_is_one_component_after_every_step() {
        let mut a = sim(1);
        // A 10-sim-day run, one planning step at a time.
        let steps = (10.0 * 86400.0 / PLANNING_STEP) as usize;
        for i in 0..steps {
            let dt = a.next_step();
            a.step(dt);
            if i % 97 == 0 {
                assert!(
                    grid::network_connected(&a.map),
                    "the lane network split at t={:.0}s",
                    a.time
                );
            }
        }
        assert!(grid::network_connected(&a.map));
    }

    #[test]
    fn graded_roads_hold_their_invariants() {
        let mut a = sim(1);
        a.run_to(8.0 * 86400.0);
        let m = &a.map;
        for y in 0..m.h {
            for x in 0..m.w {
                let t = m.at(x, y);
                if !t.kind.is_lane() && t.kind != Kind::Sidewalk {
                    continue;
                }
                // Flat across the road.
                assert!(t.is_flat(), "cross-slope at {x},{y}");
                let level = t.min_level();
                for (edge, d) in [(0usize, (1, 0)), (1, (0, 1)), (2, (-1, 0)), (3, (0, -1))] {
                    let n = m.at(x + d.0, y + d.1);
                    if n.kind.is_lane() || n.kind == Kind::Sidewalk {
                        let step = (level as i32 - n.min_level() as i32).abs();
                        assert!(step <= 1, "step of {step} between {x},{y} and its neighbour");
                        if t.kind == Kind::Sidewalk && n.kind.is_lane() {
                            // Pavement sits at its own road's level and never
                            // steps more than one from another road beside it.
                            assert_eq!(level, n.min_level(), "pavement off its road level");
                        }
                    } else {
                        let step = (level as i32 - n.min_level() as i32).abs();
                        if step >= 1 {
                            assert!(
                                map::edge_works(t.aux, edge) > 0,
                                "no wall/embankment at {x},{y} edge {edge} (step {step})"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn houses_stand_on_graded_pads() {
        let mut a = sim(1);
        a.run_to(6.0 * 86400.0);
        assert!(!a.lots.is_empty());
        for l in &a.lots {
            for ty in l.y..l.y + l.h {
                for tx in l.x..l.x + l.w {
                    let t = a.map.at(tx, ty);
                    assert!(t.is_flat(), "lot {:?} has a slope at {tx},{ty}", l.kind);
                    assert_eq!(t.min_level(), l.level, "lot pad off its level");
                    assert!(!t.kind.is_lane(), "lot on a lane");
                }
            }
        }
    }

    #[test]
    fn construction_goes_through_every_stage() {
        let mut a = sim(1);
        a.run_to(1.0 * 86400.0);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..2000 {
            a.run_to(a.time + 60.0);
            for l in &a.lots {
                seen.insert(l.stage);
            }
            if seen.len() >= Stage::ALL.len() {
                break;
            }
        }
        assert_eq!(seen.len(), Stage::ALL.len(), "stages seen: {seen:?}");
    }

    #[test]
    fn catch_up_matches_running_live() {
        let mut live = sim(3);
        live.run_to(10.0 * 3600.0);

        let dir = std::env::temp_dir().join("citylive-iso-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("catchup.sav");

        let early = sim(3);
        early.save(&path).unwrap();
        let mut loaded = Sim::load(&path).unwrap();
        loaded.run_to(10.0 * 3600.0);

        assert_eq!(live.hash(), loaded.hash(), "catch-up diverged from a live run");
        assert_eq!(live.lots.len(), loaded.lots.len());
    }

    #[test]
    fn founding_is_at_the_middle_of_the_frame() {
        // D35: the first settlement projects within 10 % of the framebuffer
        // centre, at both internal resolutions.
        for (w, h) in [(960i32, 540i32), (853, 480)] {
            let vp = map::world_for_viewport(w, h);
            let (fx, fy) = map::screen_to_tile(&vp, w as f32 * 0.5, h as f32 * 0.5);
            for seed in 1..=5u64 {
                // B46: the strict 10 % is the temperate rule it was written for.
                // A desert or a tundra has to walk past a dune or a rock face to
                // find a flat, dry site, so the band widens to 18 % for the
                // lands that do not have a lawn in the middle of the frame.
                for biome in map::Biome::LIVE {
                    let mut s = Sim::with_focus_in(
                        seed,
                        Pace::Month,
                        vp.w,
                        vp.h,
                        (fx, fy),
                        Some(biome),
                    );
                    s.set_focus(fx, fy);
                    s.found_again();
                    let (x, y) = s.founding;
                    let sx = vp.origin.0 as f32
                        + (x as f32 + 0.5 - y as f32 - 0.5) * map::iso_consts::HALF_W as f32;
                    let sy = vp.origin.1 as f32
                        + (x as f32 + 0.5 + y as f32 + 0.5) * map::iso_consts::HALF_H as f32;
                    let dx = (sx - w as f32 * 0.5).abs() / w as f32;
                    let dy = (sy - h as f32 * 0.5).abs() / h as f32;
                    let band = if biome == map::Biome::Temperate { 0.10 } else { 0.22 };
                    assert!(
                        dx < band && dy < band,
                        "{biome:?} seed {seed} {w}x{h}: founding {dx:.2},{dy:.2} off centre"
                    );
                }
            }
        }
    }

    #[test]
    fn the_village_has_space_between_houses() {
        // D41: detached, spaced out, streets readable.
        let mut a = sim(1);
        a.run_to(8.0 * 86400.0);
        // D41 is about the village's **detached** houses; a town's street wall
        // is attached by design (B26b item 4).
        let lots: Vec<&Lot> = a
            .lots
            .iter()
            .filter(|l| l.finished() && l.kind == LotKind::House)
            .collect();
        assert!(lots.len() > 12, "only {} detached houses", lots.len());

        // 1. Footprints never share an edge and are well apart.
        for (i, x) in lots.iter().enumerate() {
            for y in lots.iter().skip(i + 1) {
                let gap_x = (x.x - (y.x + y.w)).max(y.x - (x.x + x.w));
                let gap_y = (x.y - (y.y + y.h)).max(y.y - (x.y + x.h));
                assert!(gap_x >= 1 || gap_y >= 1, "lots at {},{} and {},{} touch", x.x, x.y, y.x, y.y);
            }
        }
        // 2. At least two sides with open ground (counted against the other
        // detached houses: a town terrace may stand beside an old cottage once
        // the town has grown round the village).
        for l in &lots {
            let mut open = 0;
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (l.x + dx, l.y + dy);
                let occupied = lots.iter().any(|o| o.covers(nx, ny));
                if !occupied {
                    open += 1;
                }
            }
            assert!(open >= 2, "lot at {},{} has only {open} open sides", l.x, l.y);
        }
        // 3. No lot ever stands **on** the street (a lot may front one, B26b).
        for l in &a.lots {
            for ty in l.y..l.y + l.h {
                for tx in l.x..l.x + l.w {
                    assert!(
                        !a.map.at(tx, ty).kind.is_lane(),
                        "a lot sits right on the street at {tx},{ty}"
                    );
                }
            }
        }
        // 4. Coverage: the built footprint is a small share of the plots.
        let lot_area: f32 = lots.iter().map(|l| (l.w * l.h) as f32).sum();
        let built_area = lot_area * 0.56 * 0.56; // 1 - 2*INSET, per side
        assert!(built_area / lot_area <= 0.35, "buildings cover {:.0}% of the lots", built_area / lot_area * 100.0);
    }

    #[test]
    fn streets_are_straight_runs() {
        // B11 item 2: A* used to zig-zag tile by tile; with a turn penalty the
        // network should be long runs with rare bends.
        let mut a = sim(1);
        a.run_to(8.0 * 86400.0);
        let lanes: std::collections::BTreeSet<(i32, i32)> = a.lane_tiles().into_iter().collect();
        let mut bends = 0;
        for &(x, y) in &lanes {
            let n = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .filter(|(dx, dy)| lanes.contains(&(x + dx, y + dy)))
                .count();
            let has_x = lanes.contains(&(x + 1, y)) || lanes.contains(&(x - 1, y));
            let has_y = lanes.contains(&(x, y + 1)) || lanes.contains(&(x, y - 1));
            if n == 2 && has_x && has_y {
                bends += 1;
            }
        }
        // B26b: village branches now head outward with a little wander, so a few
        // more bends are expected; the check is still against a zig-zag.
        let frac = bends as f32 / lanes.len().max(1) as f32;
        assert!(frac < 0.25, "{:.0}% of road tiles are bends", frac * 100.0);
    }

    #[test]
    fn lots_never_sit_above_their_road() {
        let mut a = sim(1);
        a.run_to(6.0 * 86400.0);
        for l in &a.lots {
            // A landmark is deliberately placed on a rise above its street (the
            // cathedral on the hill); it is not a slab left too high.
            if matches!(l.kind, LotKind::Landmark(_)) {
                continue;
            }
            let mut road_level = None;
            for ty in l.y - 1..=l.y + l.h {
                for tx in l.x - 1..=l.x + l.w {
                    let on_edge = tx < l.x || tx >= l.x + l.w || ty < l.y || ty >= l.y + l.h;
                    if on_edge && a.map.at(tx, ty).kind.is_lane() {
                        road_level = Some(a.map.at(tx, ty).min_level());
                    }
                }
            }
            if let Some(r) = road_level {
                // The pad is levelled to the street when it is staked, but a lane
                // can be regraded later (a bridge approach, B21's coastal ramps),
                // so one level of step is allowed — a kerb, not a raised slab.
                assert!(
                    l.level as i32 - r as i32 <= 1,
                    "lot at {},{} is {} above its road",
                    l.x,
                    l.y,
                    l.level as i32 - r as i32
                );
            }
        }
    }

    /// The frames' world: the founding at the middle of a 960x540 framebuffer.
    fn frame_sim(seed: u64) -> Sim {
        // A 1080p screen at D61's 1x: the same land the old 2x engine showed.
        let vp = map::world_for_viewport(1920, 1080);
        let focus = map::screen_to_tile(&vp, 960.0, 540.0);
        // B46e: pinned to the temperate land these tests were tuned on, so a
        // seed's land cannot move under them.
        Sim::with_focus_in(seed, Pace::Month, vp.w, vp.h, focus, Some(map::Biome::Temperate))
    }

    /// B26b item 1: **roads follow need** — street tiles stay at or under 30 % of
    /// the built-up area at days 8, 16 and 30, and there is always more building
    /// land than road. (B26 was 63-81 % of the built area.)
    #[test]
    fn roads_stay_a_minority_of_the_developed_area() {
        for seed in [1u64, 7, 9] {
            let mut a = frame_sim(seed);
            for day in [8u32, 16, 30] {
                a.run_to(day as f64 * 86400.0);
                let share = a.road_share();
                let (road, lots) = a.street_ratio();
                assert!(
                    share <= 0.30,
                    "seed {seed} day {day}: {road} street tiles = {:.0} % of the built area",
                    share * 100.0
                );
                assert!(lots > road, "seed {seed} day {day}: {road} road vs {lots} lot tiles");
            }
        }
    }

    /// B26b item 2: paving keeps up — by day 16 at least 80 % of the street
    /// tiles inside the built-up area are asphalt (B26 managed 1 %).
    #[test]
    fn paving_keeps_up_with_the_town() {
        for seed in [1u64, 7, 9] {
            let mut a = frame_sim(seed);
            a.run_to(16.0 * 86400.0);
            let (x0, y0, x1, y1) = a.built_box();
            let (mut total, mut paved) = (0, 0);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let k = a.map.at(x, y).kind;
                    if k.is_lane() || k == Kind::Sidewalk {
                        total += 1;
                        if k == Kind::Paved || k == Kind::Road {
                            paved += 1;
                        }
                    }
                }
            }
            assert!(total > 40, "seed {seed}: only {total} street tiles built");
            let share = paved as f32 / total as f32;
            assert!(
                share >= 0.80,
                "seed {seed} day 16: {paved}/{total} paved = {:.0} %",
                share * 100.0
            );
        }
    }

    /// B26b item 4: no mid-rise box stands out in a field — every dense building
    /// belongs to a block with a paved street within three tiles.
    #[test]
    fn dense_buildings_stand_in_blocks() {
        let mut a = frame_sim(7);
        a.run_to(30.0 * 86400.0);
        let mut dense = 0;
        for l in a.lots.iter().filter(|l| l.kind.urban()) {
            // Parks and landmarks are sited deliberately (a green, the old
            // square, the rail line, a coastal point), not by the block grid
            // (B36).
            // B59: car places are sited deliberately too (a roadside trade on
            // the edge of town, not a plot in a block).
            if matches!(
                l.kind,
                LotKind::Park
                    | LotKind::Landmark(_)
                    | LotKind::LogCabin
                    | LotKind::CarPark
                    | LotKind::CarParkDeck
                    | LotKind::GasStation
                    | LotKind::ChargingHub
                    | LotKind::DriveThrough
                    | LotKind::CarWash
                    | LotKind::Dealership
                    | LotKind::Motel
            ) {
                continue;
            }
            // Inside an opened block (or a street within three tiles for the
            // old core's fill), never alone in a field.
            let mut ok = a.in_open_cell(l.x, l.y);
            for ty in l.y - 3..=l.y + l.h + 2 {
                for tx in l.x - 3..=l.x + l.w + 2 {
                    if a.map.at(tx, ty).kind.is_lane() {
                        ok = true;
                    }
                }
            }
            assert!(ok, "{:?} at {},{} stands outside any block", l.kind, l.x, l.y);
            dense += 1;
        }
        assert!(dense > 50, "only {dense} urban buildings");
    }

    /// B31b item 5: the railway survives a save/load — same line, same station,
    /// same port, and no replanning drift (the line must not move because the
    /// plots around it changed).
    #[test]
    fn the_railway_survives_a_save() {
        let mut a = frame_sim(7);
        a.run_to(60.0 * 86400.0);
        a.plan_rail();
        assert!(a.rail.line.len() > 10, "no line planned: {}", a.rail.line.len());
        let line = a.rail.line.clone();
        let station = a.rail.station;
        let port = a.rail.port;

        let dir = std::env::temp_dir().join("citylive-iso-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rail.sav");
        a.save(&path).unwrap();
        let b = Sim::load(&path).unwrap();

        assert_eq!(b.rail.line, line, "the line moved on reload");
        assert_eq!(b.rail.station_platform, a.rail.station_platform);
        assert_eq!(b.rail.tram_stops, a.rail.tram_stops);
        assert_eq!(b.rail.station, station);
        assert_eq!(b.rail.port, port);
        assert!(b.rail.planned, "the reloaded city would replan the railway");
        // A reloaded city must not replan: running the transport step keeps the
        // same alignment.
        let mut c = b;
        c.transport_step(0.2);
        assert_eq!(c.rail.line, line);
    }

    /// B31b item 4: the platform runs **beside** the track (not on it) and the
    /// trams have stops on their line.
    #[test]
    fn the_platform_is_beside_the_track_and_trams_have_stops() {
        let mut s = frame_sim(7);
        s.run_to(60.0 * 86400.0);
        s.plan_rail();
        let station = s.rail.station.expect("no station");
        assert!(s.rail.line.contains(&station), "the station is not on the line");
        let Some((px, py, side)) = s.rail.station_platform else {
            panic!("no platform");
        };
        assert_ne!((px, py), station, "the platform is on the track");
        assert!(
            (px - station.0).abs() + (py - station.1).abs() == 1,
            "the platform is not beside the station"
        );
        // The side flag agrees with the offset.
        let want = match (px - station.0, py - station.1) {
            (1, 0) => 0,
            (-1, 0) => 1,
            (0, 1) => 2,
            _ => 3,
        };
        assert_eq!(side, want, "the platform side flag is wrong");

        assert!(!s.rail.tram_stops.is_empty(), "no tram stops");
        for stop in &s.rail.tram_stops {
            assert!(
                s.rail.tram_line.contains(stop),
                "tram stop {stop:?} is not on the tram line"
            );
        }
    }

    /// B31b item 4: a tram actually stands at a tram stop for a few seconds.
    #[test]
    fn a_tram_stops_at_its_stops() {
        let mut s = frame_sim(7);
        s.run_to(60.0 * 86400.0);
        s.plan_rail();
        assert!(s.rail.tram_line.len() > 8, "no tram line");
        // Run until a tram exists.
        let mut found = false;
        for _ in 0..4000 {
            s.transport_step(0.2);
            if s.transport().iter().any(|m| m.kind == crate::rail::MoverKind::Tram) {
                found = true;
                break;
            }
        }
        assert!(found, "no tram was ever dispatched");
        // Run a while and check the tram spent time standing on a stop.
        let mut dwelt = 0;
        for _ in 0..600 {
            s.transport_step(0.1);
            if s.transport()
                .iter()
                .any(|m| m.kind == crate::rail::MoverKind::Tram && m.dwell > 0.0)
            {
                dwelt += 1;
            }
        }
        assert!(dwelt > 5, "the tram never stopped at a stop ({dwelt})");
    }

    /// B30 item 3: at least 98 % of the trips the city generates reach the
    /// street network, on a day-30 town and a day-60 city.
    #[test]
    fn nearly_every_trip_routes() {
        for day in [30u32, 60] {
            let mut a = frame_sim(7);
            a.run_to(day as f64 * 86400.0);
            assert!(a.lots.len() > 100, "day {day}: only {} lots", a.lots.len());
            let mut ok = 0;
            let mut tried = 0;
            for _ in 0..200 {
                a.traffic.vehicles.clear();
                tried += 1;
                if a.spawn_trip() {
                    ok += 1;
                }
            }
            let rate = ok as f32 / tried as f32;
            assert!(
                rate >= 0.98,
                "day {day}: only {:.1} % of trips routed ({ok}/{tried}, why {:?})",
                rate * 100.0,
                a.traffic.why
            );
        }
    }

    /// D66: the land is saved, so a reload is the same land even when the
    /// seed's roll would pick another today.
    #[test]
    fn the_biome_survives_a_save() {
        let dir = std::env::temp_dir().join("citylive-b46-biome");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("desert.sav");
        let mut a = Sim::with_focus_in(9, Pace::Month, 41, 41, (20.0, 20.0), Some(map::Biome::Desert));
        assert_eq!(a.map.biome, map::Biome::Desert);
        assert!(matches!(
            a.map.at(20, 20).kind,
            map::Kind::Sand | map::Kind::DryGrass | map::Kind::Rock | map::Kind::Scree
        ), "a desert village stands on desert ground");
        a.run_to(86400.0);
        a.save(&path).unwrap();
        let b = Sim::load(&path).unwrap();
        assert_eq!(b.map.biome, map::Biome::Desert);
        assert_eq!(a.hash(), b.hash());
        let _ = std::fs::remove_file(&path);
    }

    /// D66: a save written before the biome existed was a temperate world and
    /// must load as one — the terrain is regenerated from the seed, so a seed
    /// that rolls a desert today would otherwise move the city's ground.
    #[test]
    fn an_old_save_loads_as_temperate() {
        let dir = std::env::temp_dir().join("citylive-b46-old");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v7.sav");
        let mut a = sim(3);
        a.run_to(86400.0);
        a.save(&path).unwrap();
        // Rewrite the header as a v7 file: version 7, and no biome byte (it
        // sits just after the founding point, at byte 41).
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[4..8].copy_from_slice(&7u32.to_le_bytes());
        bytes.remove(41);
        std::fs::write(&path, &bytes).unwrap();
        let b = Sim::load(&path).unwrap();
        assert_eq!(b.map.biome, map::Biome::Temperate);
        assert_eq!(b.map.w, a.map.w);
        // B48a/D68: a save that predates the naming gets the name the land and
        // the seed would have given it at the founding, and no feed.
        assert_eq!(b.city_name, story::city_name(3, map::Biome::Temperate));
        assert!(b.story.is_empty(), "a save from before the feed had no feed");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_round_trip_is_identical() {
        let mut a = sim(5);
        a.run_to(2.0 * 86400.0);
        let dir = std::env::temp_dir().join("citylive-iso-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.sav");
        a.save(&path).unwrap();
        let b = Sim::load(&path).unwrap();
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a.time, b.time);
        assert_eq!(a.lots.len(), b.lots.len());
        assert!(std::fs::metadata(&path).unwrap().len() > 16);
    }

    /// B48a: the city's name and the story feed survive a save, and the
    /// reloaded city keeps telling the story where the save left it — it does
    /// not re-tell a line its feed already has (the one-shot flags are
    /// re-derived from the city on load).
    #[test]
    fn the_name_and_the_feed_survive_a_save() {
        let mut a = sim(9);
        a.run_to(10.0 * 86400.0);
        let dir = std::env::temp_dir().join("citylive-story-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("story.sav");
        a.save(&path).unwrap();
        let mut b = Sim::load(&path).unwrap();
        assert_eq!(b.city_name, a.city_name, "the city keeps its name");
        assert_eq!(b.story.len(), a.story.len(), "the feed is kept whole");
        for (i, la) in a.story.iter().enumerate() {
            let lb = b
                .story
                .iter()
                .nth(i)
                .unwrap_or_else(|| panic!("feed line {i} lost on load"));
            assert_eq!(la, lb, "feed line {i} changed on load");
        }
        assert_eq!(a.hash(), b.hash(), "the name and the feed are part of the city");
        // Two lines of the feed must be about things that already happened
        // (the settlement always tells its founding first).
        assert!(a.story.iter().next().unwrap().time > 0.0);
        // The reloaded city keeps telling the story.
        let kept = b.story.len();
        let last_time = b.story.latest(1).first().unwrap().time;
        b.run_to(14.0 * 86400.0);
        assert!(b.story.len() >= kept, "the ring only grows while it is under the cap");
        let newest_time = b.story.latest(1).first().unwrap().time;
        assert!(newest_time >= last_time, "the feed only moves forward");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[ignore = "timing test: run with --release --ignored"]
    fn a_week_of_catch_up_takes_under_two_seconds() {
        let dir = std::env::temp_dir().join("citylive-iso-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("week.sav");
        let a = sim(7);
        a.save(&path).unwrap();
        let mut b = Sim::load(&path).unwrap();
        let t0 = std::time::Instant::now();
        b.run_to(7.0 * 86400.0);
        let dt = t0.elapsed();
        println!("one week of catch-up: {:.2?}", dt);
        assert!(dt.as_secs_f64() < 2.0, "catch-up took {:.2?}", dt);
    }
}
