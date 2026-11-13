//! B79 (D84, D90): water life. Islands and their jetties, the sea routes between
//! them, the boats that sail them, and the life on the shore (beaches, anglers,
//! gulls).
//!
//! Nothing here is saved. The islands, the jetties, the routes and the beach
//! spots are derived from the map and the lots (like the harbour on an older
//! save), and the boats' positions are a function of the transport clock and the
//! time of day, so a load replans the same picture.
//!
//! * An **island** is a patch of dry land (4-connected, bridges count as land)
//!   with at least [`MIN_AREA`] tiles in the frame. The island the city grows on
//!   is the **home island**; the others are outlying islands, which the sim does
//!   not build on (the streets cannot reach them, B66). Those get a **landing**:
//!   the village's boats go there, and it is where the beaches and the fishing
//!   are.
//! * A **jetty** grows with its island: a plank jetty, a pier, then a harbour
//!   with a shed ([`Jetty::level`]). It reaches out from a free shore tile, a
//!   straight deck on water tiles in the +x or +y direction (towards the camera).
//! * **Routes** are tile paths on water only: ferries and water taxis between
//!   jetties, fishing boats to a ground out at sea, pleasure boats to a cove,
//!   cargo to the home harbour. Planned one route per transport step, so a plan
//!   never costs a frame.
//! * The **fleet** is a set of movers that follow their routes, wait at the
//!   jetties and sail by the activity of the day: busy on summer weekends,
//!   moored at night and in a storm; fishing boats leave at dawn and come home
//!   in the afternoon.

use crate::rail::{Mover, MoverKind};
use crate::{LotKind, Sim};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

/// Smallest island that is worth a landing, in tiles.
pub const MIN_AREA: u32 = 40;
/// Longest deck a jetty can have.
pub const MAX_DECK: i32 = 5;
/// The most routes the fleet carries (the mover `line` byte is `route + 1`).
const MAX_ROUTES: usize = 160;
/// The most fleet boats at once.
const MAX_FLEET: usize = 90;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Jetty {
    pub isle: usize,
    /// The land region's id (stable across replans).
    pub isle_id: u32,
    /// The land tile the deck starts from.
    pub shore: (i32, i32),
    /// The way it points: `(1, 0)` or `(0, 1)`.
    pub dir: (i32, i32),
    /// The deck at full length, from the shore outwards (water tiles).
    pub deck: Vec<(i32, i32)>,
    /// 1 jetty, 2 pier, 3 harbour.
    pub level: u8,
    /// Is this the home island's jetty?
    pub home: bool,
}

impl Jetty {
    /// How many deck tiles are built at this level.
    pub fn visible_len(&self) -> usize {
        (2 + self.level as usize).min(self.deck.len())
    }

    pub fn perp(&self) -> (i32, i32) {
        (self.dir.1, self.dir.0)
    }

    /// Water tiles a boat can lie in, tip first: the sides of the full-length
    /// deck, then the water just beyond the tip. Always the same list for a
    /// jetty, so a boat's berth never moves when the jetty grows.
    pub fn berths(&self) -> Vec<(i32, i32)> {
        let p = self.perp();
        let mut out = Vec::new();
        for i in (1..self.deck.len()).rev() {
            let (x, y) = self.deck[i];
            out.push((x + p.0, y + p.1));
            out.push((x - p.0, y - p.1));
        }
        if let Some(&(x, y)) = self.deck.last() {
            out.push((x + self.dir.0, y + self.dir.1));
            out.push((x + self.dir.0 + p.0, y + self.dir.1 + p.1));
            out.push((x + self.dir.0 - p.0, y + self.dir.1 - p.1));
            out.push((x + self.dir.0 * 2, y + self.dir.1 * 2));
            out.push((x + self.dir.0 * 2 + p.0, y + self.dir.1 * 2 + p.1));
            out.push((x + self.dir.0 * 2 - p.0, y + self.dir.1 * 2 - p.1));
        }
        out
    }
}

#[derive(Clone, Debug, Default)]
pub struct Isle {
    pub id: u32,
    pub area: u32,
    pub lots: u32,
    pub centre: (i32, i32),
    pub home: bool,
    pub jetty: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Service {
    /// A ferry between two jetties.
    Ferry = 1,
    /// A water taxi between two jetties.
    Taxi = 2,
    /// A pleasure boat: out to a cove and back.
    Pleasure = 3,
    /// A fishing boat: out to the ground at dawn, home in the afternoon.
    Fishing = 4,
    /// Cargo: from the open sea to the home harbour.
    Cargo = 5,
    /// A harbour boat answering a crash.
    Rescue = 6,
}

impl Service {
    fn from_code(c: u8) -> Option<Self> {
        match c & 0x0f {
            1 => Some(Service::Ferry),
            2 => Some(Service::Taxi),
            3 => Some(Service::Pleasure),
            4 => Some(Service::Fishing),
            5 => Some(Service::Cargo),
            6 => Some(Service::Rescue),
            6 => Some(Service::Rescue),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SeaRoute {
    pub service: Service,
    pub jetty: usize,
    pub other: Option<usize>,
    pub slot: usize,
    pub slot2: usize,
    pub salt: u32,
    /// Corner points, water tiles only, starting at the jetty's berth.
    pub path: Vec<(i32, i32)>,
}

#[derive(Clone, Copy, Debug)]
struct Job {
    service: Service,
    jetty: usize,
    other: Option<usize>,
    slot: usize,
    slot2: usize,
    salt: u32,
}

/// What stands on a beach or in the water by it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShoreKind {
    Umbrella = 0,
    Towel = 1,
    Sunbather = 2,
    Swimmer = 3,
    Angler = 4,
    /// A gull in the air (`lift` is its height).
    Gull = 5,
    /// A gull perched on a post.
    GullPerched = 6,
    /// A child with a ball on the sand.
    Child = 7,
    /// Flotsam on the water where two boats bumped.
    Debris = 9,
    /// A marker buoy (a flag by day, an amber light at night) at a crash.
    Hazard = 10,
    /// A standing bather (waist deep).
    Wader = 8,
}

#[derive(Clone, Copy, Debug)]
pub struct Spot {
    pub x: f32,
    pub y: f32,
    pub kind: ShoreKind,
    pub var: u8,
    /// Shows up when the beach mood is above this.
    pub thr: f32,
    /// Which way it faces.
    pub dir: (i8, i8),
}

#[derive(Clone, Copy, Debug)]
pub struct ShoreItem {
    pub x: f32,
    pub y: f32,
    pub kind: ShoreKind,
    pub var: u8,
    pub dir: (i8, i8),
    /// 0..1 animation phase.
    pub phase: f32,
    /// Height above the water in px (gulls).
    pub lift: i32,
}

/// A boat lying at a berth.
#[derive(Clone, Copy, Debug)]
pub struct Moored {
    pub x: f32,
    pub y: f32,
    pub dir: (f32, f32),
    pub set: u8,
}

/// The mood of the sea and the beach: told to the sim by the wallpaper (the sim
/// never fetches the weather), not saved.
#[derive(Clone, Copy, Debug)]
pub struct Mood {
    /// 0 freezing .. 1 hot. Swimmers show above 0.55.
    pub warm: f32,
    /// 0 calm .. 1 storm.
    pub storm: f32,
    /// 0 winter .. 1 high summer, from the calendar (not the air).
    pub season: f32,
}

impl Default for Mood {
    fn default() -> Self {
        Self { warm: 0.6, storm: 0.0, season: 1.0 }
    }
}

#[derive(Default, Clone)]
pub struct Isles {
    pub isles: Vec<Isle>,
    pub jetties: Vec<Jetty>,
    pub routes: Vec<SeaRoute>,
    spots: Vec<Spot>,
    jobs: Vec<Job>,
    /// Clock of the last full plan.
    pub planned_at: f64,
    /// Per tile: 255 land, 0 water touching land, 1 land within two, 2 open.
    clear: Vec<u8>,
    pub mood: Mood,
    /// Per mover: how much room it has ahead this step (1 clear .. 0 stop).
    pub factor: Vec<f32>,
    /// Counters for the checker: boat-boat overlaps and boats off the water
    /// seen by [`Sim::boat_audit`], crashes that happened.
    pub crashes: u32,
    pub last_crash: f64,
    /// Per mover: seconds it has been held with no room to move.
    stuck: Vec<f32>,
    /// Lane targets the avoidance chose: (mover, offset, ease rate).
    steer: Vec<(usize, f32, f32)>,
    /// Boats put back in harbour after being held too long (the checker's
    /// count of avoidance that failed without a crash).
    pub given_up: u32,
}

fn hash(a: u64) -> u64 {
    let mut z = a.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(a: u64) -> f32 {
    (hash(a) >> 40) as f32 / (1u64 << 24) as f32
}

const DIRS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

impl Sim {
    /// Tell the sim how the weather feels (the wallpaper does, every few
    /// seconds): `temp_f` is the air temperature, `storm` 0..1 a thunderstorm
    /// or a gale.
    pub fn set_sea_mood(&mut self, temp_f: f32, storm: f32) {
        let season = self.rail.isles.mood.season;
        self.rail.isles.mood = Mood {
            warm: ((temp_f - 50.0) / 36.0).clamp(0.0, 1.0),
            storm: storm.clamp(0.0, 1.0),
            season,
        };
    }

    /// The calendar's season for the beach, 0 deep winter .. 1 midsummer: a
    /// warm day in January does not fill the sand.
    pub fn set_sea_season(&mut self, season: f32) {
        self.rail.isles.mood.season = season.clamp(0.0, 1.0);
    }

    /// How busy the water is, 0..1: the hour, the week, the season, the storm.
    pub fn sea_activity(&self) -> f32 {
        let hod = (self.time / 3600.0).rem_euclid(24.0) as f32;
        let day = smooth(5.5, 9.0, hod) * (1.0 - smooth(18.0, 21.5, hod));
        let m = self.rail.isles.mood;
        let weekend = if self.is_weekend() { 1.0 } else { 0.0 };
        let base = ((0.22 + 0.5 * m.warm + 0.28 * weekend) * (0.55 + 0.45 * m.season)).min(1.0);
        (0.04 + day * base * (1.0 - m.storm * 0.97)).clamp(0.0, 1.0)
    }

    /// How many of the beach's people are out, 0..1 (see [`Spot::thr`]).
    pub fn beach_mood(&self) -> f32 {
        let hod = (self.time / 3600.0).rem_euclid(24.0) as f32;
        let day = smooth(8.5, 11.0, hod) * (1.0 - smooth(18.5, 20.5, hod));
        let m = self.rail.isles.mood;
        let weekend = if self.is_weekend() { 0.12 } else { 0.0 };
        let cap = 0.25 + 0.75 * m.season;
        ((m.warm + weekend).min(cap) * day * (1.0 - m.storm)).clamp(0.0, 1.0)
    }

    /// Is this tile within `r` tiles of a jetty's deck or berths?
    pub(crate) fn near_jetty(&self, x: i32, y: i32, r: i32) -> bool {
        self.rail.isles.jetties.iter().any(|j| {
            j.deck.iter().chain(j.berths().iter()).any(|&(dx, dy)| (dx - x).abs() <= r && (dy - y).abs() <= r)
        })
    }

    fn isles_ground_ok(&self, x: i32, y: i32) -> bool {
        self.map.in_bounds(x, y)
            && self.map.at(x, y).is_water()
            && !self.near_land_line(x, y, 70)
            && self.in_frame(x, y, 0, 8)
    }

    /// One transport step's worth of island work: a (re)plan when due, one
    /// route job, the fleet.
    pub(crate) fn isles_step(&mut self, dt: f64) {
        if self.lots.is_empty() || self.map.w < 8 || std::env::var("ISLES_OFF").is_ok() {
            self.fleet_step(dt as f32);
            return;
        }
        let clock = self.rail.clock;
        let due = self.rail.isles.planned_at == 0.0 || clock - self.rail.isles.planned_at >= 1800.0;
        if due {
            self.plan_isles();
        }
        if let Some(job) = self.rail.isles.jobs.pop() {
            self.run_job(job);
        }
        self.fleet_maintain();
        self.fleet_step(dt as f32);
    }

    // ------------------------------------------------------------- islands

    fn plan_isles(&mut self) {
        let clock = self.rail.clock;
        let (w, h) = (self.map.w, self.map.h);
        let n = (w * h) as usize;
        // Land regions: 4-connected dry ground, bridges included.
        let mut comp = vec![0u32; n];
        let mut regions: Vec<(u32, u32, i64, i64, u32)> = Vec::new(); // id, area, sx, sy, in-frame
        let mut stack: Vec<(i32, i32)> = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if comp[i] != 0 || self.map.at(x, y).is_water() {
                    continue;
                }
                let id = i as u32 + 1;
                let (mut area, mut sx, mut sy, mut vis) = (0u32, 0i64, 0i64, 0u32);
                comp[i] = id;
                stack.push((x, y));
                while let Some((cx, cy)) = stack.pop() {
                    area += 1;
                    sx += cx as i64;
                    sy += cy as i64;
                    if self.in_frame(cx, cy, 0, 24) && !self.near_land_line(cx, cy, 40) {
                        vis += 1;
                    }
                    for (dx, dy) in DIRS {
                        let (nx, ny) = (cx + dx, cy + dy);
                        if !self.map.in_bounds(nx, ny) {
                            continue;
                        }
                        let ni = (ny * w + nx) as usize;
                        if comp[ni] == 0 && !self.map.at(nx, ny).is_water() {
                            comp[ni] = id;
                            stack.push((nx, ny));
                        }
                    }
                }
                regions.push((id, area, sx, sy, vis));
            }
        }
        // Lots per region; the home island is the one with the most.
        let mut lots_in: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut lot_c: std::collections::HashMap<u32, (i64, i64, i64)> = std::collections::HashMap::new();
        for l in &self.lots {
            if matches!(l.kind, LotKind::Park | LotKind::Cemetery) {
                continue;
            }
            let id = comp[(l.y.clamp(0, h - 1) * w + l.x.clamp(0, w - 1)) as usize];
            if id != 0 {
                *lots_in.entry(id).or_default() += 1;
                let e = lot_c.entry(id).or_default();
                e.0 += l.x as i64;
                e.1 += l.y as i64;
                e.2 += 1;
            }
        }
        let home_id = lots_in.iter().max_by_key(|(id, c)| (**c, Reverse(**id))).map(|(id, _)| *id);
        let era_ok = self.era() >= crate::Era::Village;
        let mut isles: Vec<Isle> = Vec::new();
        for &(id, area, sx, sy, vis) in &regions {
            if area < MIN_AREA || vis < 10 {
                continue;
            }
            let lots = lots_in.get(&id).copied().unwrap_or(0);
            let home = Some(id) == home_id;
            if !home && !era_ok {
                continue;
            }
            let centre = match lot_c.get(&id) {
                Some(&(x, y, c)) if c > 0 => ((x / c) as i32, (y / c) as i32),
                _ => ((sx / area as i64) as i32, (sy / area as i64) as i32),
            };
            isles.push(Isle { id, area, lots, centre, home, jetty: None });
        }
        // Keep the jetties already planned (matched by region id).
        let old_isles = std::mem::take(&mut self.rail.isles.isles);
        let old_jetties = std::mem::take(&mut self.rail.isles.jetties);
        let mut jetties: Vec<Jetty> = Vec::new();
        self.rail.isles.clear = self.clearance_map();
        let occupied: HashSet<(i32, i32)> = self
            .lots
            .iter()
            .flat_map(|l| (l.x - 1..=l.x + l.w).flat_map(move |x| (l.y - 1..=l.y + l.h).map(move |y| (x, y))))
            .collect();
        let harbour_tiles: HashSet<(i32, i32)> = self.rail.harbour.as_ref().map(|hb| hb.ground().into_iter().chain(hb.water_tiles()).chain(hb.moored.iter().copied()).chain(hb.ferry.iter().flat_map(|&(x, y)| (-2..=2).flat_map(move |dx| (-2..=2).map(move |dy| (x + dx, y + dy))))).collect()).unwrap_or_default();
        let marina = self.rail.marina;
        for (k, isle) in isles.iter_mut().enumerate() {
            let level: u8 = if isle.lots >= 18 { 3 } else if isle.lots >= 6 { 2 } else { 1 };
            let kept: Vec<&Jetty> = old_jetties.iter().filter(|j| j.isle_id == isle.id).collect();
            let found: Vec<Jetty> = if !kept.is_empty() {
                kept.into_iter().cloned().collect()
            } else {
                self.find_jetties(k, isle, &comp, &occupied, &harbour_tiles, marina)
            };
            for (n, j) in found.into_iter().enumerate() {
                if n == 0 {
                    isle.jetty = Some(jetties.len());
                }
                // The first jetty is the island's own; the extra ones on a big
                // island are a step behind it.
                let lvl = if n == 0 { level } else { level.saturating_sub(1).max(1) };
                jetties.push(Jetty { isle: k, level: lvl, home: isle.home, ..j });
            }
        }
        let _ = old_isles;
        let had = old_jetties.len();
        self.rail.isles.isles = isles;
        self.rail.isles.jetties = jetties;
        self.rail.isles.planned_at = clock;
        self.plan_spots(&comp, &occupied);
        // Route jobs for whatever is new.
        if self.rail.isles.jetties.len() != had || self.rail.isles.routes.is_empty() {
            self.plan_jobs();
        }
    }

    fn clearance_map(&self) -> Vec<u8> {
        let (w, h) = (self.map.w, self.map.h);
        let mut out = vec![255u8; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                if !self.map.at(x, y).is_water() {
                    continue;
                }
                let mut c = 2u8;
                'r: for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        let (nx, ny) = (x + dx, y + dy);
                        if !self.map.in_bounds(nx, ny) {
                            continue;
                        }
                        if !self.map.at(nx, ny).is_water() {
                            if dx.abs() <= 1 && dy.abs() <= 1 {
                                c = 0;
                                break 'r;
                            }
                            c = 1;
                        }
                    }
                }
                out[(y * w + x) as usize] = c;
            }
        }
        out
    }

    /// The jetties an island gets: the best free shore, and on a big island
    /// (a long coast) two more far from it. Straight decks of three to five
    /// water tiles in the +x or +y direction, open water beyond the tip.
    fn find_jetties(
        &self,
        k: usize,
        isle: &Isle,
        comp: &[u32],
        occupied: &HashSet<(i32, i32)>,
        harbour: &HashSet<(i32, i32)>,
        marina: Option<crate::rail::Zone>,
    ) -> Vec<Jetty> {
        let w = self.map.w;
        let in_marina = |x: i32, y: i32| marina.is_some_and(|z| z.contains(x, y));
        let mut cands: Vec<(i64, Jetty)> = Vec::new();
        let (cx, cy) = isle.centre;
        for y in 2..self.map.h - 2 {
            for x in 2..w - 2 {
                if comp[(y * w + x) as usize] != isle.id {
                    continue;
                }
                let t = self.map.at(x, y);
                if t.kind.is_worked() || t.relief() > 1 || occupied.contains(&(x, y)) {
                    continue;
                }
                for dir in [(1, 0), (0, 1)] {
                    let p = (dir.1, dir.0);
                    let mut deck = Vec::new();
                    for s in 1..=MAX_DECK {
                        let (dx, dy) = (x + dir.0 * s, y + dir.1 * s);
                        let ok = self.isles_ground_ok(dx, dy)
                            && self.map.at(dx + p.0, dy + p.1).is_water()
                            && self.map.at(dx - p.0, dy - p.1).is_water()
                            && !harbour.contains(&(dx, dy))
                            && !in_marina(dx, dy);
                        if !ok {
                            break;
                        }
                        deck.push((dx, dy));
                    }
                    if deck.len() < 3 {
                        continue;
                    }
                    // Open water beyond the tip for the boats.
                    let tip = *deck.last().unwrap();
                    if !(1..=3).all(|s| self.isles_ground_ok(tip.0 + dir.0 * s, tip.1 + dir.1 * s)) {
                        continue;
                    }
                    // The open sea in front, not a cove: clear of land four
                    // tiles out beyond the tip.
                    if !(1..=4).all(|s| {
                        let (ox, oy) = (tip.0 + dir.0 * s, tip.1 + dir.1 * s);
                        self.map.at(ox + p.0, oy + p.1).is_water() && self.map.at(ox - p.0, oy - p.1).is_water()
                    }) {
                        continue;
                    }
                    let d = ((x - cx).abs() + (y - cy).abs()) as i64;
                    let score = d * 4 - deck.len() as i64 * 3 + (hash((x * 131 + y) as u64 ^ isle.id as u64) % 7) as i64;
                    cands.push((score, Jetty { isle: k, isle_id: isle.id, shore: (x, y), dir, deck, level: 1, home: isle.home }));
                }
            }
        }
        cands.sort_by_key(|(s, j)| (*s, j.shore));
        let want = if isle.area > 1500 { 4 } else { 1 };
        let mut picked: Vec<Jetty> = Vec::new();
        if let Some((_, first)) = cands.first() {
            picked.push(first.clone());
        }
        // Then the candidate farthest from those already picked, so the
        // jetties spread round the whole coast and the whole sea has boats.
        while picked.len() < want {
            let best = cands
                .iter()
                .take(4000)
                .map(|(_, j)| (picked.iter().map(|o| (o.shore.0 - j.shore.0).abs() + (o.shore.1 - j.shore.1).abs()).min().unwrap_or(0), j))
                .filter(|(d, _)| *d >= 18)
                .max_by_key(|(d, j)| (*d, Reverse(j.shore)));
            match best {
                Some((_, j)) => picked.push(j.clone()),
                None => break,
            }
        }
        picked
    }

    // -------------------------------------------------------------- routes

    fn plan_jobs(&mut self) {
        let isles = &self.rail.isles;
        let nj = isles.jetties.len();
        if nj == 0 {
            return;
        }
        let mut jobs: Vec<Job> = Vec::new();
        let hub = isles
            .jetties
            .iter()
            .enumerate()
            .max_by_key(|(i, j)| (j.home, isles.isles[j.isle].lots, Reverse(*i)))
            .map(|(i, _)| i)
            .unwrap_or(0);
        let mut slots = vec![0usize; nj];
        let exists = |isles: &Isles, svc: Service, a: usize, b: Option<usize>| {
            isles.routes.iter().any(|r| r.service == svc && r.jetty == a && r.other == b)
                || isles.jobs.iter().any(|r| r.service == svc && r.jetty == a && r.other == b)
        };
        // Count slots already taken by existing routes, so new ones queue up.
        for r in &isles.routes {
            slots[r.jetty] = slots[r.jetty].max(r.slot + 1);
            if let Some(o) = r.other {
                slots[o] = slots[o].max(r.slot2 + 1);
            }
        }
        // Links: every outlying jetty to the nearest jetty of the home island,
        // as a ferry and a taxi on one path; and each to its nearest
        // neighbour among the outlying ones.
        let dist = |a: usize, b: usize| {
            let (p, q) = (isles.jetties[a].shore, isles.jetties[b].shore);
            (p.0 - q.0).abs() + (p.1 - q.1).abs()
        };
        for j in 0..nj {
            if isles.jetties[j].home {
                continue;
            }
            let Some(hubj) = (0..nj).filter(|&o| isles.jetties[o].home).min_by_key(|&o| dist(j, o)) else { continue };
            for svc in [Service::Ferry, Service::Taxi] {
                if !exists(isles, svc, j, Some(hubj)) {
                    jobs.push(Job { service: svc, jetty: j, other: Some(hubj), slot: 0, slot2: 0, salt: j as u32 });
                }
            }
        }
        for j in 0..nj {
            if isles.jetties[j].home {
                continue;
            }
            let near = (0..nj).filter(|&o| o != j && !isles.jetties[o].home && isles.jetties[o].isle != isles.jetties[j].isle).min_by_key(|&o| dist(j, o));
            if let Some(o) = near {
                if j < o && dist(j, o) < 60 && !exists(isles, Service::Ferry, j, Some(o)) && !exists(isles, Service::Ferry, o, Some(j)) {
                    jobs.push(Job { service: Service::Ferry, jetty: j, other: Some(o), slot: 0, slot2: 0, salt: (j * 7 + o) as u32 });
                }
            }
        }
        // Fishing, pleasure and cargo.
        for j in 0..nj {
            let big = isles.jetties[j].level >= 2;
            let have = |svc: Service, salt: u32| {
                isles.routes.iter().any(|r| r.service == svc && r.jetty == j && r.salt == salt)
                    || isles.jobs.iter().any(|r| r.service == svc && r.jetty == j && r.salt == salt)
            };
            let fish_salt = j as u32 * 3 + 1;
            if !have(Service::Fishing, fish_salt) {
                jobs.push(Job { service: Service::Fishing, jetty: j, other: None, slot: 0, slot2: 0, salt: fish_salt });
            }
            if !have(Service::Fishing, fish_salt + 1) {
                jobs.push(Job { service: Service::Fishing, jetty: j, other: None, slot: 0, slot2: 0, salt: fish_salt + 1 });
            }
            for p in 0..(if big { 5 } else { 4 }) {
                let salt = j as u32 * 5 + p;
                if !have(Service::Pleasure, salt) {
                    jobs.push(Job { service: Service::Pleasure, jetty: j, other: None, slot: 0, slot2: 0, salt });
                }
            }
        }
        if isles.jetties[hub].level >= 2 && !exists(isles, Service::Cargo, hub, None) {
            jobs.push(Job { service: Service::Cargo, jetty: hub, other: None, slot: 0, slot2: 0, salt: 77 });
        }
        for jb in &mut jobs {
            jb.slot = slots[jb.jetty];
            slots[jb.jetty] += 1;
            if let Some(o) = jb.other {
                jb.slot2 = slots[o];
                slots[o] += 1;
            }
        }
        // Popped from the back: ferries first.
        jobs.reverse();
        self.rail.isles.jobs.extend(jobs);
    }

    /// Open water beyond the tip, for a ship or a harbour launch to start from.
    fn anchorage(&self, j: usize, k: i32) -> Option<((i32, i32), (i32, i32))> {
        let jet = &self.rail.isles.jetties[j];
        let tip = *jet.deck.last()?;
        let t = (tip.0 + jet.dir.0 * (3 + k), tip.1 + jet.dir.1 * (3 + k));
        self.isles_ground_ok(t.0, t.1).then_some((t, jet.dir))
    }

    /// A berth and the way out of it: away from the deck, so a boat leaving
    /// does not pass through the boats lying beside it.
    fn berth_of(&self, j: usize, slot: usize) -> Option<((i32, i32), (i32, i32))> {
        let jet = &self.rail.isles.jetties[j];
        let ok: Vec<(i32, i32)> = jet.berths().into_iter().filter(|&(x, y)| self.isles_ground_ok(x, y)).collect();
        // One boat to a berth: a jetty with no room left gets no more routes.
        let used: std::collections::HashSet<(i32, i32)> = self
            .rail
            .isles
            .routes
            .iter()
            .flat_map(|r| [r.path.first().copied(), r.path.last().copied()])
            .flatten()
            .collect();
        let _ = slot;
        let b = *ok.iter().find(|t| !used.contains(t))?;
        let p = jet.perp();
        let out = if jet.deck.contains(&(b.0 - p.0, b.1 - p.1)) {
            p
        } else if jet.deck.contains(&(b.0 + p.0, b.1 + p.1)) {
            (-p.0, -p.1)
        } else {
            jet.dir
        };
        Some((b, out))
    }

    fn run_job(&mut self, job: Job) {
        if self.rail.isles.routes.len() >= MAX_ROUTES || job.jetty >= self.rail.isles.jetties.len() {
            return;
        }
        let first = if job.service == Service::Cargo { self.anchorage(job.jetty, 1) } else { self.berth_of(job.jetty, job.slot) };
        // Pleasure boats beyond the first lie at a mooring in the bay, not at a
        // berth (there are only so many berths).
        let first = if job.service == Service::Pleasure && job.salt % 5 >= 1 {
            let tip = self.rail.isles.jetties[job.jetty].deck.last().copied();
            let dir = self.rail.isles.jetties[job.jetty].dir;
            tip.and_then(|t| {
                let taken: Vec<(i32, i32)> = self.rail.isles.routes.iter().flat_map(|r| [r.path.first().copied(), r.path.last().copied()]).flatten().collect();
                let mut c: Vec<(i32, i32)> = Vec::new();
                for dy in -7i32..=7 {
                    for dx in -7i32..=7 {
                        let (x, y) = (t.0 + dx, t.1 + dy);
                        let d = dx.abs() + dy.abs();
                        if (3..=7).contains(&d)
                            && self.isles_ground_ok(x, y)
                            && self.rail.isles.clear[(y * self.map.w + x) as usize] >= 2
                            && !self.near_jetty(x, y, 1)
                            && taken.iter().all(|q| (q.0 - x).abs() + (q.1 - y).abs() >= 3)
                        {
                            c.push((x, y));
                        }
                    }
                }
                c.sort_by_key(|&(x, y)| (hash((x * 733 + y) as u64 ^ job.salt as u64 * 977), x, y));
                c.first().map(|&m| (m, dir))
            })
        } else {
            first
        };
        let Some((a, start_dir)) = first else { return };
        let salt = job.salt as u64 * 977 + job.jetty as u64;
        let path = match job.service {
            Service::Ferry | Service::Taxi => {
                let Some(o) = job.other else { return };
                if o >= self.rail.isles.jetties.len() {
                    return;
                }
                // The far end: a free berth there.
                let Some((b, _)) = self.berth_of(o, job.slot2) else { return };
                self.sea_path(a, b, start_dir)
            }
            Service::Pleasure => {
                let Some(dest) = self.pick_water(a, 5, 18, 1, salt) else { return };
                if unit(salt) < 0.55 {
                    self.tack_path(a, dest, start_dir)
                } else {
                    self.sea_path(a, dest, start_dir)
                }
            }
            Service::Fishing => {
                let Some(dest) = self.pick_water(a, 12, 36, 2, salt) else { if std::env::var("ISLES_DEBUG").is_ok() { let (mut nw, mut nf, mut nc, mut nl) = (0, 0, 0, 0); for dy in -22i32..=22 { for dx in -22i32..=22 { let (x, y) = (a.0 + dx, a.1 + dy); if !self.map.in_bounds(x, y) || dx.abs() + dy.abs() < 11 || dx.abs() + dy.abs() > 22 { continue; } if self.map.at(x, y).is_water() { nw += 1; if self.in_frame(x, y, 0, 8) { nf += 1; if self.rail.isles.clear[(y * self.map.w + x) as usize] >= 3 { nc += 1; if !self.near_land_line(x, y, 110) { nl += 1; } } } } } } eprintln!("no fishing ground from {a:?}: water {nw} frame {nf} clear {nc} below line {nl} map {}x{}", self.map.w, self.map.h); } return };
                self.sea_path(a, dest, start_dir)
            }
            Service::Cargo => {
                let Some(dest) = self.pick_water(a, 16, 34, 2, salt) else { return };
                self.sea_path(a, dest, start_dir)
            }
            Service::Rescue => return,
            Service::Rescue => return,
        };
        if std::env::var("ISLES_DEBUG").is_ok() {
            eprintln!("job {:?} jetty {} from {:?} -> {}", job.service, job.jetty, a, path.as_ref().map(|p| p.len() as i32).unwrap_or(-1));
        }
        let Some(path) = path else { return };
        if path.len() < 2 {
            return;
        }
        self.rail.isles.routes.push(SeaRoute {
            service: job.service,
            jetty: job.jetty,
            other: job.other,
            slot: job.slot,
            slot2: job.slot2,
            salt: job.salt,
            path,
        });
    }

    /// A water tile `rmin..=rmax` tiles from `from` in the frame, with
    /// `clearance` (0..2) and (for variety) chosen by `salt`.
    fn pick_water(&self, from: (i32, i32), rmin: i32, rmax: i32, clearance: u8, salt: u64) -> Option<(i32, i32)> {
        let w = self.map.w;
        let mut cands: Vec<(i32, i32)> = Vec::new();
        for dy in -rmax..=rmax {
            for dx in -rmax..=rmax {
                let d = dx.abs() + dy.abs();
                if d < rmin || d > rmax {
                    continue;
                }
                let (x, y) = (from.0 + dx, from.1 + dy);
                if !self.map.in_bounds(x, y) || !self.isles_ground_ok(x, y) {
                    continue;
                }
                if self.rail.isles.clear[(y * w + x) as usize] < clearance {
                    continue;
                }
                // Well below the land line, not hazed away.
                if self.near_land_line(x, y, 110) {
                    continue;
                }
                cands.push((x, y));
            }
        }
        if cands.is_empty() {
            return None;
        }
        cands.sort_by_key(|&(x, y)| (hash((x * 7919 + y) as u64 ^ salt), x, y));
        Some(cands[0])
    }

    /// The cheapest sea path with few turns, water only, off the coast:
    /// A* over (tile, heading). Returns the corner points.
    fn sea_path(&self, a: (i32, i32), b: (i32, i32), start_dir: (i32, i32)) -> Option<Vec<(i32, i32)>> {
        let w = self.map.w;
        let (bx0, by0) = ((a.0.min(b.0) - 30).max(1), (a.1.min(b.1) - 30).max(1));
        let (bx1, by1) = ((a.0.max(b.0) + 30).min(w - 2), (a.1.max(b.1) + 30).min(self.map.h - 2));
        let (bw, bh) = (bx1 - bx0 + 1, by1 - by0 + 1);
        let idx = |x: i32, y: i32, d: usize| (((y - by0) * bw + (x - bx0)) as usize) * 4 + d;
        let n = (bw * bh) as usize * 4;
        let mut g = vec![i32::MAX; n];
        let mut from = vec![u32::MAX; n];
        let d0 = DIRS.iter().position(|&d| d == start_dir).unwrap_or(0);
        let hfn = |x: i32, y: i32| (x - b.0).abs() + (y - b.1).abs();
        let mut heap: BinaryHeap<Reverse<(i32, i32, usize)>> = BinaryHeap::new();
        let s = idx(a.0, a.1, d0);
        g[s] = 0;
        heap.push(Reverse((hfn(a.0, a.1), 0, s)));
        let near_end = |x: i32, y: i32| (x - a.0).abs() + (y - a.1).abs() <= 5 || (x - b.0).abs() + (y - b.1).abs() <= 5;
        let mut goal = None;
        let mut pops = 0;
        let deck_block: HashSet<(i32, i32)> = self
            .rail
            .isles
            .jetties
            .iter()
            .flat_map(|j| j.deck.iter().copied().chain(j.berths()))
            .filter(|&t| t != a && t != b)
            .collect();
        let deck_near: Vec<(i32, i32)> = self.rail.isles.jetties.iter().flat_map(|j| j.deck.iter().copied().chain(j.berths())).collect();
        while let Some(Reverse((_, gc, st))) = heap.pop() {
            pops += 1;
            if pops > 200_000 {
                break;
            }
            if gc > g[st] {
                continue;
            }
            let d = st % 4;
            let cell = st / 4;
            let (x, y) = (bx0 + (cell as i32 % bw), by0 + (cell as i32 / bw));
            if (x, y) == b {
                goal = Some(st);
                break;
            }
            for nd in [d, (d + 1) % 4, (d + 3) % 4] {
                let (nx, ny) = (x + DIRS[nd].0, y + DIRS[nd].1);
                if nx < bx0 || nx > bx1 || ny < by0 || ny > by1 {
                    continue;
                }
                if !self.isles_ground_ok(nx, ny) && (nx, ny) != b && (nx, ny) != a {
                    continue;
                }
                if deck_block.contains(&(nx, ny)) {
                    continue;
                }
                // Two tiles off the decks and berths, except at the ends.
                if !(((nx - a.0).abs() + (ny - a.1).abs() <= 3) || ((nx - b.0).abs() + (ny - b.1).abs() <= 3))
                    && deck_near.iter().any(|&(dx, dy)| (dx - nx).abs() <= 1 && (dy - ny).abs() <= 1)
                {
                    continue;
                }
                let c = self.rail.isles.clear[(ny * w + nx) as usize];
                let pen = match c {
                    0 if near_end(nx, ny) => 6,
                    0 => continue,
                    1 => 2,
                    _ => 0,
                };
                let cost = gc + 1 + pen + if nd != d { 5 } else { 0 };
                let ns = idx(nx, ny, nd);
                if cost < g[ns] {
                    g[ns] = cost;
                    from[ns] = st as u32;
                    heap.push(Reverse((cost + hfn(nx, ny), cost, ns)));
                }
            }
        }
        let mut st = goal?;
        let mut tiles: Vec<(i32, i32)> = Vec::new();
        loop {
            let cell = st / 4;
            tiles.push((bx0 + (cell as i32 % bw), by0 + (cell as i32 / bw)));
            if from[st] == u32::MAX {
                break;
            }
            st = from[st] as usize;
        }
        tiles.reverse();
        Some(corners(&tiles))
    }

    /// A zig-zag of short legs towards `b`, the way a sailing boat tacks.
    /// Falls back to the plain path when a leg would touch land.
    fn tack_path(&self, a: (i32, i32), b: (i32, i32), start_dir: (i32, i32)) -> Option<Vec<(i32, i32)>> {
        let w = self.map.w;
        let ok = |x: i32, y: i32| self.isles_ground_ok(x, y) && self.rail.isles.clear[(y * w + x) as usize] >= 1;
        let mut tiles = vec![a];
        let mut cur = a;
        let mut axis = if start_dir.0 != 0 { 0 } else { 1 };
        let mut leg = 0;
        let mut guard = 0;
        while cur != b && guard < 200 {
            guard += 1;
            let sgn = |axis: usize, cur: (i32, i32)| if axis == 0 { (b.0 - cur.0).signum() } else { (b.1 - cur.1).signum() };
            if sgn(axis, cur) == 0 {
                axis = 1 - axis;
                if sgn(axis, cur) == 0 {
                    break;
                }
            }
            let want = sgn(axis, cur);
            let steps = 3 + (leg % 2);
            for _ in 0..steps {
                let n = if axis == 0 { (cur.0 + want, cur.1) } else { (cur.0, cur.1 + want) };
                if (axis == 0 && cur.0 == b.0) || (axis == 1 && cur.1 == b.1) {
                    break;
                }
                if !ok(n.0, n.1) && (n.0 - a.0).abs() + (n.1 - a.1).abs() > 3 && n != b {
                    return self.sea_path(a, b, start_dir);
                }
                cur = n;
                tiles.push(cur);
            }
            axis = 1 - axis;
            leg += 1;
        }
        if cur != b {
            return self.sea_path(a, b, start_dir);
        }
        Some(corners(&tiles))
    }

    // ---------------------------------------------------------------- shore

    /// The beach: umbrellas, towels, sunbathers and swimmers on sand with
    /// water beside it.
    fn plan_spots(&mut self, comp: &[u32], occupied: &HashSet<(i32, i32)>) {
        let w = self.map.w;
        let mut spots: Vec<Spot> = Vec::new();
        let mut taken: Vec<(i32, i32)> = Vec::new();
        let mut cands: Vec<(u64, i32, i32, (i32, i32))> = Vec::new();
        for y in 3..self.map.h - 3 {
            for x in 3..w - 3 {
                let t = self.map.at(x, y);
                if t.kind != map::Kind::Sand || t.relief() > 1 || occupied.contains(&(x, y)) {
                    continue;
                }
                if !self.in_frame(x, y, 0, 40) || self.near_land_line(x, y, 90) {
                    continue;
                }
                if comp[(y * w + x) as usize] == 0 {
                    continue;
                }
                // The water lies in front of the camera: +x or +y.
                for dir in [(1, 0), (0, 1)] {
                    let (wx, wy) = (x + dir.0, y + dir.1);
                    let (w2x, w2y) = (x + dir.0 * 2, y + dir.1 * 2);
                    if !(self.map.at(wx, wy).is_water() && self.map.at(w2x, w2y).is_water()) {
                        continue;
                    }
                    if matches!(self.map.water_kind(wx, wy), map::WaterKind::Pond | map::WaterKind::River) {
                        continue;
                    }
                    // Land behind it, so the cluster has room.
                    if self.map.at(x - dir.0, y - dir.1).is_water() {
                        continue;
                    }
                    cands.push((hash((x * 977 + y * 31) as u64 ^ self.seed), x, y, dir));
                    break;
                }
            }
        }
        cands.sort();
        let mut sites = 0;
        for &(h, x, y, dir) in &cands {
            if sites >= 26 {
                break;
            }
            if taken.iter().any(|&(tx, ty)| (tx - x).abs() + (ty - y).abs() < 4) {
                continue;
            }
            // The jetty is not a beach.
            if self.rail.isles.jetties.iter().any(|j| (j.shore.0 - x).abs() + (j.shore.1 - y).abs() < 4) {
                continue;
            }
            taken.push((x, y));
            sites += 1;
            let p = (dir.1 as f32, dir.0 as f32);
            let d = (dir.0 as f32, dir.1 as f32);
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let ddir = (dir.0 as i8, dir.1 as i8);
            let v = (h & 7) as u8;
            let pu = (unit(h ^ 1) - 0.5) * 0.3;
            let at = |along: f32, across: f32| (fx + d.0 * along + p.0 * across, fy + d.1 * along + p.1 * across);
            let (ux, uy) = at(-0.08, pu);
            spots.push(Spot { x: ux, y: uy, kind: ShoreKind::Umbrella, var: v, thr: 0.34 + 0.2 * unit(h ^ 2), dir: ddir });
            let (tx, ty) = at(0.14, pu + 0.2);
            spots.push(Spot { x: tx, y: ty, kind: ShoreKind::Towel, var: v, thr: 0.34 + 0.2 * unit(h ^ 3), dir: ddir });
            let (sx, sy) = at(0.1, pu - 0.14);
            spots.push(Spot { x: sx, y: sy, kind: ShoreKind::Sunbather, var: v, thr: 0.4 + 0.2 * unit(h ^ 4), dir: ddir });
            if unit(h ^ 5) < 0.6 {
                let (cx, cy) = at(-0.2, -0.38);
                spots.push(Spot { x: cx, y: cy, kind: ShoreKind::Child, var: v, thr: 0.5 + 0.2 * unit(h ^ 6), dir: ddir });
            }
            // Swimmers a little way out, in the water in front.
            let nsw = 1 + (h >> 8) as usize % 3;
            for s in 0..nsw {
                let out = 1.55 + 0.5 * s as f32 + unit(h ^ (10 + s as u64)) * 0.4;
                let side = (unit(h ^ (20 + s as u64)) - 0.5) * 1.1;
                let (sx, sy) = at(out, side);
                if self.map.at(sx.floor() as i32, sy.floor() as i32).is_water() {
                    let kind = if s == 0 && unit(h ^ 31) < 0.5 { ShoreKind::Wader } else { ShoreKind::Swimmer };
                    spots.push(Spot { x: sx, y: sy, kind, var: ((h >> (4 * s)) & 7) as u8, thr: 0.6 + 0.3 * unit(h ^ (40 + s as u64)), dir: ddir });
                }
            }
        }
        self.rail.isles.spots = spots;
    }

    /// Everything on the shore this frame: beach people by the mood, anglers on
    /// the piers, gulls over the jetties and the fishing boats.
    pub fn shore_items(&self, out: &mut Vec<ShoreItem>) {
        let t = self.time as f32;
        let mood = self.beach_mood();
        for (i, s) in self.rail.isles.spots.iter().enumerate() {
            if mood <= s.thr {
                continue;
            }
            let ph = unit(i as u64 * 17 + self.seed);
            let (mut x, mut y) = (s.x, s.y);
            if matches!(s.kind, ShoreKind::Swimmer) {
                x += (t * 0.35 + ph * 6.3).sin() * 0.16;
                y += (t * 0.29 + ph * 5.1).cos() * 0.16;
            }
            if matches!(s.kind, ShoreKind::Child) {
                x += (t * 0.8 + ph * 6.3).sin() * 0.1;
                y += (t * 0.6 + ph * 3.1).cos() * 0.1;
            }
            out.push(ShoreItem { x, y, kind: s.kind, var: s.var, dir: s.dir, phase: (t * 0.6 + ph).fract(), lift: 0 });
        }
        let hod = (self.time / 3600.0).rem_euclid(24.0) as f32;
        let storm = self.rail.isles.mood.storm;
        // The scene of a crash while it is cleared: flotsam round the boats and
        // a marker buoy that blinks at night.
        for (k, m) in self.rail.movers.iter().enumerate() {
            if is_boat(m.kind) && m.mode & 0x40 != 0 && m.dwell > 0.0 && m.line != 254 {
                let (p, _) = m.boat_pose();
                let h = hash(k as u64 * 77 + m.set as u64);
                out.push(ShoreItem { x: p.0 + 0.45 * ((h & 3) as f32 - 1.5) / 1.5, y: p.1 + 0.35 * (((h >> 2) & 3) as f32 - 1.5) / 1.5, kind: ShoreKind::Debris, var: (h >> 5) as u8 & 7, dir: (1, 0), phase: 0.0, lift: 0 });
                out.push(ShoreItem { x: p.0 - 0.6, y: p.1 + 0.2, kind: ShoreKind::Hazard, var: 0, dir: (1, 0), phase: (t * 1.2).fract(), lift: 0 });
            }
        }
        // Anglers: at the end of each pier by day, calm weather.
        if (5.5..21.0).contains(&hod) && storm < 0.5 {
            for (ji, j) in self.rail.isles.jetties.iter().enumerate() {
                let vis = j.visible_len();
                if vis < 3 {
                    continue;
                }
                let n = if j.level >= 2 { 2 } else { 1 };
                for a in 0..n {
                    let day = self.days().floor() as u64;
                    if unit(ji as u64 * 5 + a as u64 + day * 131 + self.seed) > 0.35 + 0.55 * (0.4 + 0.6 * self.rail.isles.mood.warm) {
                        continue;
                    }
                    let (tx, ty) = j.deck[vis - 1 - a as usize];
                    let p = j.perp();
                    let side = if a == 0 { 0.2 } else { -0.2 };
                    out.push(ShoreItem {
                        x: tx as f32 + 0.5 + p.0 as f32 * side + j.dir.0 as f32 * 0.12,
                        y: ty as f32 + 0.5 + p.1 as f32 * side + j.dir.1 as f32 * 0.12,
                        kind: ShoreKind::Angler,
                        var: (ji * 3 + a) as u8,
                        dir: (j.dir.0 as i8, j.dir.1 as i8),
                        phase: (t * 0.1 + a as f32 * 0.37).fract(),
                        lift: 0,
                    });
                }
            }
        }
        // Gulls: over the fishing boats that are away from the pier, over the
        // jetties, a pair on the pier posts.
        if (5.0..20.5).contains(&hod) && storm < 0.8 {
            for m in &self.rail.movers {
                if m.line == 0 || m.kind != MoverKind::Boat || Service::from_code(m.mode) != Some(Service::Fishing) {
                    continue;
                }
                let ((bx, by), _) = m.pose();
                let len = m.route_len();
                let d = m.travelled.rem_euclid(len * 2.0);
                let away = d.min(len * 2.0 - d);
                if away < 2.5 {
                    continue;
                }
                let n = 2 + (m.set as usize >> 5) % 3;
                for k in 0..n {
                    let ph = t * (0.5 + 0.12 * k as f32) + k as f32 * 2.1 + unit(m.set as u64 + k as u64) * 6.0;
                    let r = 0.9 + 0.35 * k as f32;
                    out.push(ShoreItem {
                        x: bx + ph.cos() * r,
                        y: by + ph.sin() * r * 0.8,
                        kind: ShoreKind::Gull,
                        var: k as u8,
                        dir: if (ph + 1.57).cos() >= 0.0 { (1, 0) } else { (-1, 0) },
                        phase: (t * 2.4 + k as f32 * 0.31).fract(),
                        lift: 22 + 7 * k as i32,
                    });
                }
            }
            for (ji, j) in self.rail.isles.jetties.iter().enumerate() {
                let vis = j.visible_len();
                let tip = j.deck[vis - 1];
                for k in 0..2u64 {
                    if unit(ji as u64 * 9 + k + self.seed + (t / 90.0) as u64) > 0.7 {
                        continue;
                    }
                    let p = j.perp();
                    let side = if k == 0 { 0.4 } else { -0.4 };
                    out.push(ShoreItem {
                        x: tip.0 as f32 + 0.5 + p.0 as f32 * side,
                        y: tip.1 as f32 + 0.5 + p.1 as f32 * side,
                        kind: ShoreKind::GullPerched,
                        var: k as u8,
                        dir: (j.dir.0 as i8, j.dir.1 as i8),
                        phase: 0.0,
                        lift: 6,
                    });
                }
                let ph = t * 0.45 + ji as f32 * 1.7;
                out.push(ShoreItem {
                    x: tip.0 as f32 + 0.5 + ph.cos() * 1.8,
                    y: tip.1 as f32 + 0.5 + ph.sin() * 1.3,
                    kind: ShoreKind::Gull,
                    var: ji as u8,
                    dir: if (ph + 1.57).cos() >= 0.0 { (1, 0) } else { (-1, 0) },
                    phase: (t * 2.2 + ji as f32 * 0.4).fract(),
                    lift: 30 + (ji as i32 % 3) * 6,
                });
            }
        }
    }

    /// Boats lying at the jetties' berths: more of them on a weekend or a warm
    /// day. The first berths are the fleet's own.
    pub fn moored_boats(&self, out: &mut Vec<Moored>) {
        let m = self.rail.isles.mood;
        let weekend = if self.is_weekend() { 0.35 } else { 0.0 };
        // Visitors stay overnight, so the hour does not enter: the season and
        // the week do.
        let fill = (0.10 + 0.5 * m.warm + weekend).clamp(0.0, 1.0) * (1.0 - m.storm * 0.3);
        let era = self.era();
        for (ji, j) in self.rail.isles.jetties.iter().enumerate() {
            let used: HashSet<(i32, i32)> = self.rail.isles.routes.iter().flat_map(|r| [r.path.first().copied(), r.path.last().copied()]).flatten().collect();
            let berths: Vec<(i32, i32)> = j.berths().into_iter().filter(|&(x, y)| self.isles_ground_ok(x, y)).collect();
            let vis = j.visible_len();
            let built: HashSet<(i32, i32)> = j.deck[..vis].iter().copied().collect();
            let p = j.perp();
            for (bi, &(x, y)) in berths.iter().enumerate() {
                if used.contains(&(x, y)) {
                    continue;
                }
                // Only berths beside a built deck tile.
                if !(built.contains(&(x + p.0, y + p.1)) || built.contains(&(x - p.0, y - p.1))) {
                    continue;
                }
                if unit(ji as u64 * 101 + bi as u64 * 13 + self.seed) >= fill {
                    continue;
                }
                let r = hash(ji as u64 * 7 + bi as u64 + self.seed);
                let table: &[u8] = if era >= crate::Era::City { &[3, 2, 4, 5, 8, 6, 1, 3] } else { &[3, 2, 1, 6, 9, 3] };
                let kind = table[(r >> 8) as usize % table.len()];
                let var = ((r >> 20) & 7) as u8;
                let sign = if (r >> 30) & 1 == 0 { 1.0 } else { -1.0 };
                // Lie a little off the deck, not against its edge.
                let (ox, oy) = berth_outward(j, (x, y));
                out.push(Moored {
                    x: x as f32 + 0.5 + ox * 0.3,
                    y: y as f32 + 0.5 + oy * 0.3,
                    dir: (j.dir.0 as f32 * sign, j.dir.1 as f32 * sign),
                    set: kind | (var << 5),
                });
            }
        }
    }

    // ---------------------------------------------------------------- fleet

    fn small_kind(&self, r: u64) -> (u8, f32) {
        let era = self.era();
        let mut table: Vec<(u8, u32)> = vec![(3, 5), (2, 3), (1, 3), (9, 2), (4, 2)];
        if era >= crate::Era::Town {
            table.extend([(0, 1), (7, 1)]);
        }
        if era >= crate::Era::City {
            table.extend([(8, 2), (5, 2)]);
        }
        let total: u32 = table.iter().map(|e| e.1).sum();
        let mut roll = (r % total as u64) as u32;
        let mut kind = table[0].0;
        for e in &table {
            if roll < e.1 {
                kind = e.0;
                break;
            }
            roll -= e.1;
        }
        let speed = match kind {
            1 | 9 => 0.55,
            2..=4 => 0.75,
            8 => 1.5,
            _ => 0.95,
        };
        (kind, speed)
    }

    fn fleet_maintain(&mut self) {
        // Cheap: only every few steps, and only when there are routes.
        if self.rail.tick % 8 != 3 || self.rail.isles.routes.is_empty() {
            return;
        }
        let era = self.era();
        let routes = self.rail.isles.routes.clone();
        let mut fleet_now = self.rail.movers.iter().filter(|m| m.line > 0 && is_boat(m.kind)).count();
        for (ri, r) in routes.iter().enumerate() {
            let line = (ri + 1).min(255) as u8;
            let ja = &self.rail.isles.jetties[r.jetty];
            let lots_a = self.rail.isles.isles[ja.isle].lots;
            let lots_b = r.other.map(|o| self.rail.isles.isles[self.rail.isles.jetties[o].isle].lots).unwrap_or(0);
            let lvl_a = ja.level;
            let big_other = r.other.map(|o| self.rail.isles.jetties[o].level >= 2).unwrap_or(false);
            let home_lots = lots_a.max(lots_b);
            let want = match r.service {
                Service::Ferry => 1 + (home_lots >= 22) as usize,
                Service::Taxi => (home_lots >= 3) as usize,
                Service::Pleasure => 1,
                Service::Fishing => {
                    let _ = lvl_a;
                    1
                }
                Service::Cargo => 1,
                Service::Rescue => 0,
            };
            let have = self
                .rail
                .movers
                .iter()
                .filter(|m| m.line == line && Service::from_code(m.mode) == Some(r.service))
                .count();
            for k in have..want {
                if fleet_now >= MAX_FLEET {
                    return;
                }
                let seed = hash(self.seed ^ (ri as u64 * 31 + k as u64 * 7 + r.service as u64));
                let len = path_len(&r.path).max(1.0);
                let thr = 0.04 + 0.86 * unit(seed ^ 0x55);
                let var = ((seed >> 12) & 7) as u8;
                let (kind, set, speed, cars) = match r.service {
                    Service::Ferry => {
                        let kk: u8 = if lvl_a >= 2 && big_other && seed & 1 == 1 { 1 } else { 0 };
                        (MoverKind::Ferry, kk | (var << 5), 0.8f32, 0u8)
                    }
                    Service::Taxi => (MoverKind::Boat, var << 5, 1.25, 0),
                    Service::Pleasure => {
                        let (kind, speed) = self.small_kind(seed >> 8);
                        (MoverKind::Boat, kind | (var << 5), speed, 0)
                    }
                    Service::Fishing => (MoverKind::Boat, (if seed & 2 == 0 { 6u8 } else { 7u8 }) | (var << 5), 0.9, 0),
                    Service::Rescue => (MoverKind::Boat, 0u8, 1.8, 0),
                    Service::Cargo => {
                        if k == 0 {
                            let big = era >= crate::Era::Metropolis || home_lots >= 40;
                            (MoverKind::Ship, (if big && seed & 4 == 0 { 0u8 } else { 1u8 }) | (var << 5), 0.5, 0)
                        } else {
                            (MoverKind::Boat, 11u8 | (var << 5), 0.7, 1)
                        }
                    }
                };
                // Spread the first boats along the route so the water is not
                // empty at the start; later ones start at the berth.
                // A first position with room: not on another boat, a berth or a
                // deck. Fishing boats are placed by the hour, so they start home.
                let r_new = boat_radius(kind, set);
                let travelled;
                {
                    let (recs, obs) = self.boat_records();
                    let mut found = None;
                    for t in 0..8u64 {
                        let cand = if r.service == Service::Fishing {
                            0.0
                        } else if self.rail.clock >= 900.0 || t == 7 {
                            if k == 1 { len } else { 0.0 }
                        } else {
                            len * 2.0 * unit(seed ^ (0x99 + t * 31))
                        };
                        let probe = Mover {
                            kind,
                            route: r.path.clone(),
                            travelled: cand,
                            speed,
                            shuttle: true,
                            back: false,
                            cars,
                            lift: 0,
                            dwell: 0.0,
                            cooldown: 0.0,
                            set,
                            line,
                            mode: 0,
                            phase: 0.0,
                        };
                        let (p, _) = probe.boat_pose();
                        let free = recs.iter().all(|o| ((o.pos.0 - p.0).powi(2) + (o.pos.1 - p.1).powi(2)).sqrt() > o.r + r_new + 0.5)
                            && obs.iter().all(|o| ((o.0 - p.0).powi(2) + (o.1 - p.1).powi(2)).sqrt() > o.2 + r_new + 0.5);
                        if free {
                            found = Some(cand);
                            break;
                        }
                    }
                    // No room at the berth yet: try again next time round.
                    match found {
                        Some(c) => travelled = c,
                        None => continue,
                    }
                }
                self.rail.movers.push(Mover {
                    kind,
                    route: r.path.clone(),
                    travelled,
                    speed,
                    shuttle: true,
                    back: false,
                    cars,
                    lift: 0,
                    dwell: 0.0,
                    cooldown: 0.0,
                    set,
                    line,
                    mode: r.service as u8,
                    phase: thr,
                });
                fleet_now += 1;
            }
        }
    }

    /// Move the boats: wait at the jetties, sail by the day's activity. Every
    /// move is checked against where every other boat, deck and visitor is
    /// *now* (D91): a move that would put a hull inside another is not made, so
    /// two boats can never overlap by moving, and nothing is ever moved
    /// sideways or jumped to make room.
    fn fleet_step(&mut self, dt: f32) {
        if self.rail.movers.is_empty() {
            return;
        }
        let act = self.sea_activity();
        let storm = self.rail.isles.mood.storm;
        let hod = (self.time / 3600.0).rem_euclid(24.0) as f32;
        let factors = self.boat_factors(dt);
        let steer = std::mem::take(&mut self.rail.isles.steer);
        let (recs, obs) = self.boat_records();
        let n = self.rail.movers.len();
        let mut live: Vec<Option<(f32, f32, f32)>> = vec![None; n];
        for r in &recs {
            live[r.i] = Some((r.pos.0, r.pos.1, r.r));
        }
        let map = &self.map;
        // Is this spot free for boat `mi` of radius `r`? Water, and clear of
        // every other hull and every deck by three quarters of their radii.
        let free = |live: &Vec<Option<(f32, f32, f32)>>, mi: usize, p: (f32, f32), r: f32| -> bool {
            let (tx, ty) = (p.0.floor() as i32, p.1.floor() as i32);
            if !map.in_bounds(tx, ty) || !map.at(tx, ty).is_water() {
                return false;
            }
            for (j, o) in live.iter().enumerate() {
                if j == mi {
                    continue;
                }
                if let Some(o) = o {
                    if ((o.0 - p.0).powi(2) + (o.1 - p.1).powi(2)).sqrt() < 0.75 * (r + o.2) {
                        return false;
                    }
                }
            }
            obs.iter().all(|o| ((o.0 - p.0).powi(2) + (o.1 - p.1).powi(2)).sqrt() >= 0.75 * (r + o.2))
        };
        for mi in 0..n {
            let m = &mut self.rail.movers[mi];
            if !is_boat(m.kind) {
                continue;
            }
            let r = boat_radius(m.kind, m.set);
            let factor = factors.get(mi).copied().unwrap_or(1.0);
            let len = m.route_len();
            let off0 = m.cooldown;
            // The lane the avoidance wants, eased.
            let off1 = match steer.iter().find(|s| s.0 == mi) {
                Some(&(_, t, rate)) => off0 + (t - off0).clamp(-rate * dt, rate * dt),
                None => off0,
            };
            // What this boat would like to do: a new distance along the route.
            let mut want_t = m.travelled;
            let mut tie_up = false;
            let svc = Service::from_code(m.mode);
            if m.mode & 0x40 != 0 {
                // Crashed: stalled until it is cleared.
                m.lift = 1;
                if m.dwell > 0.0 {
                    m.dwell -= dt;
                }
                continue;
            }
            if m.line == 0 {
                // The older boats: a route and a pause, no timetable.
                if m.dwell > 0.0 {
                    m.dwell -= dt;
                } else {
                    want_t = m.travelled + m.speed * dt * factor;
                }
            } else if let Some(svc) = svc {
                if svc == Service::Fishing {
                    // Out at dawn, home in the afternoon, each boat a little off
                    // the others; a storm keeps them in. It sails towards where
                    // the hour says it should be, never jumps there.
                    let off = (m.phase - 0.5) * 1.1;
                    let out_at = 5.2 + off.max(-0.5);
                    let (t0, t1, t2, t3) = (out_at, out_at + 1.5, 14.2 + off, 15.8 + off);
                    let cyc = len * 2.0;
                    let cur = m.travelled.rem_euclid(cyc);
                    let tgt = if storm > 0.5 || hod < t0 || hod >= t3 {
                        if cur > len { cyc } else { 0.0 }
                    } else if hod < t1 {
                        len * (hod - t0) / (t1 - t0)
                    } else if hod < t2 {
                        len
                    } else {
                        len + len * (hod - t2) / (t3 - t2)
                    };
                    let delta = (tgt - cur).clamp(0.0, m.speed * dt * factor);
                    m.travelled = cur;
                    want_t = cur + delta;
                } else if m.mode & 0x80 != 0 {
                    // Tied up at an end of the route.
                    m.lift = 1;
                    if m.dwell > 0.0 {
                        m.dwell -= dt;
                    } else {
                        let at_home = (m.travelled / len).round() as i64 % 2 == 0;
                        let port_both = matches!(svc, Service::Ferry | Service::Taxi);
                        if svc != Service::Rescue && act <= m.phase && (at_home || port_both) {
                            // stays
                        } else {
                            m.mode &= 0x7f;
                            m.lift = 0;
                            want_t = m.travelled + m.speed * dt * factor;
                        }
                    }
                } else {
                    want_t = m.travelled + m.speed * dt * factor;
                }
                if svc != Service::Fishing && m.mode & 0x80 == 0 {
                    let (kb, ka) = ((m.travelled / len).floor(), (want_t / len).floor());
                    if ka != kb {
                        // Reached an end: tie up there, if it is free.
                        want_t = ka * len;
                        tie_up = true;
                    }
                }
            }
            // Try the move, then the move without the lane change, then the lane
            // change alone, then nothing. The first that leaves every hull clear
            // is made.
            let mut done = false;
            for (t, o) in [(want_t, off1), (want_t, off0), (m.travelled, off1)] {
                if t == m.travelled && o == off0 {
                    break;
                }
                let p = m.boat_pose_at(t, o).0;
                if free(&live, mi, p, r) {
                    if (t - m.travelled).abs() > 1e-6 && tie_up && t == want_t {
                        // Tied up.
                        if let Some(svc) = svc {
                            m.mode |= 0x80;
                            let far = ((t / len).round() as i64) % 2 != 0;
                            m.dwell = match svc {
                                Service::Ferry => 14.0,
                                Service::Taxi => 6.0,
                                Service::Pleasure => if far { 25.0 } else { 2.0 },
                                Service::Cargo => if far { 20.0 } else { 40.0 },
                                Service::Fishing => 0.0,
                                Service::Rescue => if far { 40.0 } else { 0.0 },
                            };
                            if svc == Service::Rescue && !far {
                                m.mode |= 0x40;
                            }
                            m.lift = 1;
                        }
                    }
                    m.travelled = t;
                    m.cooldown = o;
                    live[mi] = Some((p.0, p.1, r));
                    done = true;
                    break;
                }
            }
            if !done && tie_up {
                // The berth is taken: wait outside it.
                m.lift = 0;
            }
            // `travelled` never grows without limit: whole round trips are dropped
            // (the parity of the leg, and so the pose, are unchanged).
            if m.travelled > len * 8.0 {
                m.travelled -= len * 4.0;
            }
            if svc == Some(Service::Fishing) {
                let cyc = len * 2.0;
                let cur = m.travelled.rem_euclid(cyc);
                m.lift = if cur < 1e-3 || (cur - len).abs() < 1e-3 || (cur - cyc).abs() < 1e-3 { 1 } else { 0 };
            }
        }
        // The old general boats retire after a few round trips: the water does
        // not fill up for ever.
        self.rail.movers.retain(|m| {
            !(m.line == 0 && is_boat(m.kind) && m.travelled > m.route_len() * 5.0)
        });
    }

}


// ------------------------------------------------------------------ D91

/// A boat's footprint radius in tiles.
fn boat_radius(kind: MoverKind, set: u8) -> f32 {
    match kind {
        MoverKind::Ship => 1.1,
        MoverKind::Ferry => 0.7,
        _ => match set & 31 {
            11 => 0.65,
            12 => 0.8,
            10 => 0.6,
            5..=7 => 0.55,
            3 | 4 => 0.5,
            8 | 0 | 2 => 0.42,
            _ => 0.36,
        },
    }
}

/// Who gives way: the smaller boat. Sail over power is fine.
fn boat_priority(kind: MoverKind, set: u8, fishing: bool) -> i32 {
    if fishing {
        return 9;
    }
    match kind {
        MoverKind::Ship => 6,
        MoverKind::Ferry => 5,
        _ => match set & 31 {
            11 | 12 => 4,
            5..=7 | 10 => 3,
            0 | 8 | 3 | 4 => 2,
            _ => 1,
        },
    }
}

struct BoatRec {
    i: usize,
    pos: (f32, f32),
    dir: (f32, f32),
    r: f32,
    prio: i32,
    still: bool,
    speed: f32,
}

impl Sim {
    /// Every boat's footprint circle this instant, plus the fixed obstacles
    /// (jetty decks, the harbour's piers and moored boats, visitors at berths).
    fn boat_records(&self) -> (Vec<BoatRec>, Vec<(f32, f32, f32)>) {
        let mut recs = Vec::new();
        for (i, m) in self.rail.movers.iter().enumerate() {
            if !is_boat(m.kind) {
                continue;
            }
            let (pos, dir) = m.boat_pose();
            let n = (dir.0 * dir.0 + dir.1 * dir.1).sqrt().max(1e-3);
            let fishing = m.line > 0 && Service::from_code(m.mode) == Some(Service::Fishing);
            recs.push(BoatRec {
                i,
                pos,
                dir: (dir.0 / n, dir.1 / n),
                r: boat_radius(m.kind, m.set),
                prio: boat_priority(m.kind, m.set, fishing),
                still: m.lift == 1 || m.dwell > 0.0 || m.mode & 0xc0 != 0,
                speed: m.speed,
            });
        }
        let mut obs: Vec<(f32, f32, f32)> = Vec::new();
        for j in &self.rail.isles.jetties {
            for &(x, y) in &j.deck {
                obs.push((x as f32 + 0.5, y as f32 + 0.5, 0.55));
            }
        }
        let mut moored = Vec::new();
        self.moored_boats(&mut moored);
        for m in moored {
            obs.push((m.x, m.y, 0.42));
        }
        (recs, obs)
    }

    /// How much room each boat has ahead (1 clear, 0 stopped), and steering:
    /// a blocked boat eases to the side that is free, back to its own lane when
    /// that is clear. Crashes happen when it fails, rarely, more in a storm.
    pub(crate) fn boat_factors(&mut self, dt: f32) -> Vec<f32> {
        let n = self.rail.movers.len();
        let mut fac = vec![1.0f32; n];
        if std::env::var("ISLES_NOCOLLIDE").is_ok() || !self.rail.movers.iter().any(|m| is_boat(m.kind)) {
            self.rail.isles.factor = fac.clone();
            return fac;
        }
        let (recs, obs) = self.boat_records();
        let storm = self.rail.isles.mood.storm;
        let w = self.map.w;
        let mut crashes: Vec<(usize, usize)> = Vec::new();
        let mut steer: Vec<(usize, f32, f32)> = Vec::new();
        let mut reverse: Vec<usize> = Vec::new();
        let mut give_up: Vec<usize> = Vec::new();
        let mut stuck = std::mem::take(&mut self.rail.isles.stuck);
        stuck.resize(n, 0.0);
        {
            let clear_ok = |x: f32, y: f32| -> bool {
                let (tx, ty) = (x.floor() as i32, y.floor() as i32);
                if !self.map.in_bounds(tx, ty) || !self.map.at(tx, ty).is_water() {
                    return false;
                }
                self.rail.isles.clear.get((ty * w + tx) as usize).map(|&c| c >= 1).unwrap_or(true)
            };
            let free_at = |a: &BoatRec, p: (f32, f32)| -> bool {
                recs.iter().all(|b| b.i == a.i || ((b.pos.0 - p.0).powi(2) + (b.pos.1 - p.1).powi(2)).sqrt() > a.r + b.r + 0.15)
                    && obs.iter().all(|o| ((o.0 - p.0).powi(2) + (o.1 - p.1).powi(2)).sqrt() > a.r + o.2 + 0.1)
            };
            for a in &recs {
                let m = &self.rail.movers[a.i];
                if m.mode & 0x40 != 0 {
                    fac[a.i] = 0.0;
                    continue;
                }
                let gap = 0.35 + a.speed * 1.1;
                let mut f = 1.0f32;
                let mut blocked = false;
                let mut hard = false;
                let mut close: Option<usize> = None;
                let mut consider = |dist: f32, ahead: f32, rs: f32, yields: bool, other: Option<usize>, other_pos: (f32, f32)| {
                    // Where this boat will be shortly: a crossing boat counts too.
                    let fut = (a.pos.0 + a.dir.0 * gap * 0.7, a.pos.1 + a.dir.1 * gap * 0.7);
                    let df = ((other_pos.0 - fut.0).powi(2) + (other_pos.1 - fut.1).powi(2)).sqrt();
                    let crossing = df < rs && df < dist;
                    let dist = if crossing { dist.min(df + 0.3) } else { dist };
                    let ahead = if crossing { ahead.max(0.5) } else { ahead };
                    if dist >= rs + gap || (ahead <= 0.25 && dist >= rs) {
                        return;
                    }
                    let fr = ((dist - rs) / gap).clamp(0.0, 1.0);
                    if fr < 0.85 {
                        blocked = true;
                    }
                    if yields {
                        f = f.min(fr);
                    } else if dist < rs + 0.3 && ahead > 0.25 && dist >= 0.25 {
                        // The boat with right of way still stops short of a
                        // boat that has not moved aside.
                        f = f.min(fr);
                    }
                    if fr < 0.3 && yields && other.is_some() {
                        close = other;
                    }
                };
                for b in &recs {
                    if b.i == a.i {
                        continue;
                    }
                    let d = (b.pos.0 - a.pos.0, b.pos.1 - a.pos.1);
                    let dist = (d.0 * d.0 + d.1 * d.1).sqrt().max(1e-3);
                    let ahead = if dist < 0.25 { 1.0 } else { (a.dir.0 * d.0 + a.dir.1 * d.1) / dist };
                    let yields = b.still || b.prio > a.prio || (b.prio == a.prio && b.i < a.i);
                    if dist < 0.5 * (a.r + b.r) && b.i < a.i {
                        hard = true;
                    }
                    consider(dist, ahead, a.r + b.r, yields, Some(b.i), b.pos);
                }
                for o in &obs {
                    let d = (o.0 - a.pos.0, o.1 - a.pos.1);
                    let dist = (d.0 * d.0 + d.1 * d.1).sqrt().max(1e-3);
                    let ahead = (a.dir.0 * d.0 + a.dir.1 * d.1) / dist;
                    consider(dist, ahead, a.r + o.2, true, None, (o.0, o.1));
                }
                fac[a.i] = f;
                if (f < 0.05 || hard) && !a.still {
                    stuck[a.i] += dt;
                    if stuck[a.i] > 40.0 {
                        give_up.push(a.i);
                    }
                } else {
                    stuck[a.i] = 0.0;
                }
                if f < 0.05 && std::env::var("FAC_DEBUG").is_ok() && self.rail.tick % 400 == 0 {
                    let mut why = String::new();
                    for b in &recs {
                        if b.i != a.i && ((b.pos.0 - a.pos.0).powi(2) + (b.pos.1 - a.pos.1).powi(2)).sqrt() < a.r + b.r + gap {
                            why += &format!(" boat{} at {:.1},{:.1} r{:.2} still{} prio{};", b.i, b.pos.0, b.pos.1, b.r, b.still, b.prio);
                        }
                    }
                    for o in &obs {
                        if ((o.0 - a.pos.0).powi(2) + (o.1 - a.pos.1).powi(2)).sqrt() < a.r + o.2 + gap {
                            why += &format!(" obs {:.1},{:.1};", o.0, o.1);
                        }
                    }
                    eprintln!("STUCK mover{} {:?} line {} at {:.1},{:.1} dir {:.1},{:.1} prio {} still {}:{}", a.i, m.kind, m.line, a.pos.0, a.pos.1, a.dir.0, a.dir.1, a.prio, a.still, why);
                }
                // Nose to nose in a channel, neither can pass: the smaller boat
                // gives up and turns back.
                if f < 0.05 && blocked && !a.still {
                    let roll = unit(self.rail.tick.wrapping_mul(2654435761) ^ (a.i as u64 * 977) ^ self.seed);
                    if roll < 0.25 * dt {
                        reverse.push(a.i);
                    }
                }
                // Steering: a blocked boat eases to starboard if that is free,
                // else to port; a free boat goes back to its lane when the
                // lane is clear.
                let perp = (-a.dir.1, a.dir.0);
                let lane = {
                    let len = m.route_len();
                    let d = m.travelled.rem_euclid(len * 2.0);
                    let d = d.min(len * 2.0 - d);
                    let edge = d.min(len - d).max(0.0);
                    lane_of(m) * (edge / 3.0).clamp(0.0, 1.0)
                };
                let off = m.cooldown;
                let try_off = |t: f32| -> bool {
                    let p = (a.pos.0 + perp.0 * (t - off), a.pos.1 + perp.1 * (t - off));
                    clear_ok(p.0, p.1) && free_at(a, p)
                };
                let mut target = off;
                let mut rate = 0.8f32;
                let here_ok = clear_ok(a.pos.0, a.pos.1);
                if a.still {
                    // Tied up: back to the berth's own spot.
                    target = 0.0;
                } else if !here_ok {
                    // The lane has run into a shore: straight back to the route.
                    target = 0.0;
                    rate = 2.5;
                } else if blocked {
                    for t in [off + 0.9, off - 0.9] {
                        if t.abs() <= 1.2 && try_off(t) {
                            target = t;
                            break;
                        }
                    }
                } else if (off - lane).abs() > 0.01 && try_off(lane) {
                    target = lane;
                }
                if target != off {
                    steer.push((a.i, target, rate));
                }
                // A crash: too close and not stopping. Rare; more in a storm.
                if let Some(o) = close {
                    if self.rail.clock - self.rail.isles.last_crash > 7200.0 || (storm > 0.5 && self.rail.clock - self.rail.isles.last_crash > 300.0) {
                        let rate = (0.00004 + 0.03 * storm) * dt;
                        let roll = unit(self.rail.tick.wrapping_mul(7919) ^ (a.i as u64 * 131 + o as u64) ^ self.seed);
                        if roll < rate {
                            crashes.push((a.i, o));
                        }
                    }
                }
            }
        }
        self.rail.isles.steer = steer;
        for i in give_up {
            let m = &mut self.rail.movers[i];
            if m.mode & 0x40 == 0 {
                m.mode |= 0x40;
                m.dwell = 0.0;
                self.rail.isles.given_up += 1;
            }
            if let Some(t) = stuck.get_mut(i) {
                *t = 0.0;
            }
        }
        self.rail.isles.stuck = stuck;
        let pos_before: std::collections::HashMap<usize, (f32, f32)> = reverse.iter().map(|&i| (i, self.rail.movers[i].boat_pose().0)).collect();
        for i in reverse {
            let m = &mut self.rail.movers[i];
            let len = m.route_len();
            m.travelled = (2.0 * len - m.travelled.rem_euclid(2.0 * len)).rem_euclid(2.0 * len);
            // Turning round flips which side is starboard: keep the hull where
            // it was (whichever sign of lane puts it there).
            let before = pos_before[&i];
            let p_a = m.boat_pose_at(m.travelled, m.cooldown).0;
            let p_b = m.boat_pose_at(m.travelled, -m.cooldown).0;
            let d = |p: (f32, f32)| (p.0 - before.0).powi(2) + (p.1 - before.1).powi(2);
            if d(p_b) < d(p_a) {
                m.cooldown = -m.cooldown;
            }
            m.mode &= !0x80;
        }
        for (i, j) in crashes {
            self.boat_crash(i, j);
        }
        self.rail.isles.factor = fac.clone();
        fac
    }

    /// Two boats hit: both stall, a story line, a harbour boat comes out, then
    /// they are cleared.
    fn boat_crash(&mut self, i: usize, j: usize) {
        if self.rail.movers[i].mode & 0x40 != 0 || self.rail.movers[j].mode & 0x40 != 0 {
            return;
        }
        let (pos, _) = self.rail.movers[i].boat_pose();
        let what = |sim: &Sim, k: usize| -> &'static str {
            let m = &sim.rail.movers[k];
            match m.kind {
                MoverKind::Ship => "a coaster",
                MoverKind::Ferry => "a ferry",
                _ => match m.set & 31 {
                    11 => "a tug",
                    12 => "a barge",
                    6 | 7 => "a fishing boat",
                    3 | 4 => "a sailboat",
                    1 | 9 => "a rowing boat",
                    _ => "a small boat",
                },
            }
        };
        let line = if self.rail.isles.mood.storm > 0.5 {
            format!("in the storm {} and {} collide on the water: a harbour boat goes out", what(self, i), what(self, j))
        } else {
            format!("{} and {} bump on the water: nobody hurt, a harbour boat goes out", what(self, i), what(self, j))
        };
        for k in [i, j] {
            let m = &mut self.rail.movers[k];
            m.mode |= 0x40;
            m.dwell = 150.0;
            m.lift = 1;
        }
        self.rail.isles.crashes += 1;
        self.rail.isles.last_crash = self.rail.clock;
        self.push_story(line);
        // The harbour boat, from the nearest jetty.
        let target = (pos.0.floor() as i32, pos.1.floor() as i32);
        let nearest = (0..self.rail.isles.jetties.len()).min_by_key(|&k| {
            let s = self.rail.isles.jetties[k].shore;
            (s.0 - target.0).abs() + (s.1 - target.1).abs()
        });
        if let Some(jt) = nearest {
            if self.rail.isles.clear.is_empty() {
                return;
            }
            let Some((a, d)) = self.anchorage(jt, 0) else { return };
            if let Some(path) = self.sea_path(a, target, d) {
                if path.len() >= 2 {
                    self.push_boat(Mover {
                        kind: MoverKind::Boat,
                        route: path,
                        travelled: 0.0,
                        speed: 1.8,
                        shuttle: true,
                        back: false,
                        cars: 0,
                        lift: 0,
                        dwell: 0.0,
                        cooldown: 0.0,
                        set: 0,
                        line: 254,
                        mode: Service::Rescue as u8,
                        phase: 0.0,
                    });
                }
            }
        }
    }

    /// The checker (D91): boat pairs overlapping and boats off the water right
    /// now. Crashed boats do not count (their overlap is the accident).
    pub fn boat_audit(&self) -> (u32, u32) {
        let (recs, obs) = self.boat_records();
        let (mut pairs, mut land) = (0u32, 0u32);
        for (k, a) in recs.iter().enumerate() {
            let m = &self.rail.movers[a.i];
            let (tx, ty) = (a.pos.0.floor() as i32, a.pos.1.floor() as i32);
            if !self.map.in_bounds(tx, ty) || !self.map.at(tx, ty).is_water() {
                if std::env::var("AUDIT_DEBUG").is_ok() {
                    eprintln!("land: {:?} line {} set {} lift {} off {:.2} at {:.2},{:.2} travelled {:.1} len {:.1}", m.kind, m.line, m.set & 31, m.lift, m.cooldown, a.pos.0, a.pos.1, m.travelled, m.route_len());
                }
                land += 1;
            }
            for b in &recs[k + 1..] {
                let d = ((a.pos.0 - b.pos.0).powi(2) + (a.pos.1 - b.pos.1).powi(2)).sqrt();
                if d < (a.r + b.r) * 0.74 {
                    if std::env::var("AUDIT_DEBUG").is_ok() {
                        let (ma, mb) = (&self.rail.movers[a.i], &self.rail.movers[b.i]);
                        eprintln!("pair d {d:.2} rs {:.2}: {:?} line {} set {} mode {} lift {} dwell {:.0} off {:.2} at {:.1},{:.1} | {:?} line {} set {} mode {} lift {} dwell {:.0} off {:.2} at {:.1},{:.1}", a.r + b.r, ma.kind, ma.line, ma.set & 31, ma.mode, ma.lift, ma.dwell, ma.cooldown, a.pos.0, a.pos.1, mb.kind, mb.line, mb.set & 31, mb.mode, mb.lift, mb.dwell, mb.cooldown, b.pos.0, b.pos.1);
                    }
                    pairs += 1;
                }
            }
            // Docking: a boat within a tile and a half of its route's end is
            // alongside the deck on purpose.
            let docking = {
                let len = m.route_len();
                let d = m.travelled.rem_euclid(len * 2.0);
                let d = d.min(len * 2.0 - d);
                d.min(len - d) < 3.5
            };
            for o in &obs {
                let d = ((a.pos.0 - o.0).powi(2) + (a.pos.1 - o.1).powi(2)).sqrt();
                if d < (a.r + o.2) * 0.74 {
                    if std::env::var("AUDIT_DEBUG").is_ok() {
                        let ma = &self.rail.movers[a.i];
                        eprintln!("obs d {d:.2} rs {:.2}: {:?} line {} set {} mode {} off {:.2} at {:.1},{:.1} obs {:.1},{:.1}", a.r + o.2, ma.kind, ma.line, ma.set & 31, ma.mode, ma.cooldown, a.pos.0, a.pos.1, o.0, o.1);
                    }
                    pairs += 1;
                }
            }
        }
        (pairs, land)
    }

    /// Put a new boat on the water, but only where it has room: a new boat
    /// never appears on top of another, a deck or a visitor. False when there
    /// was no room (the caller simply tries again later).
    pub(crate) fn push_boat(&mut self, m: Mover) -> bool {
        let (recs, obs) = self.boat_records();
        let r = boat_radius(m.kind, m.set);
        let p = m.boat_pose().0;
        let (tx, ty) = (p.0.floor() as i32, p.1.floor() as i32);
        let ok = self.map.in_bounds(tx, ty)
            && self.map.at(tx, ty).is_water()
            && recs.iter().all(|o| ((o.pos.0 - p.0).powi(2) + (o.pos.1 - p.1).powi(2)).sqrt() >= o.r + r + 0.2)
            && obs.iter().all(|o| ((o.0 - p.0).powi(2) + (o.1 - p.1).powi(2)).sqrt() >= o.2 + r + 0.1);
        if ok {
            self.rail.movers.push(m);
        }
        ok
    }

    /// For a still picture (the shot tools): make the two boats nearest each
    /// other collide now.
    pub fn stage_crash(&mut self) -> bool {
        let (recs, _) = self.boat_records();
        let mut best: Option<(f32, usize, usize)> = None;
        for (k, a) in recs.iter().enumerate() {
            for b in &recs[k + 1..] {
                let d = ((a.pos.0 - b.pos.0).powi(2) + (a.pos.1 - b.pos.1).powi(2)).sqrt();
                if self.rail.movers[a.i].line != 254 && self.rail.movers[b.i].line != 254 && best.map(|x| d < x.0).unwrap_or(true) {
                    best = Some((d, a.i, b.i));
                }
            }
        }
        match best {
            Some((_, i, j)) => {
                self.boat_crash(i, j);
                true
            }
            None => false,
        }
    }

    /// Indices of overlapping boat pairs right now (the checker's detail).
    pub fn boat_overlaps(&self) -> Vec<(usize, usize)> {
        let (recs, _) = self.boat_records();
        let mut out = Vec::new();
        for (k, a) in recs.iter().enumerate() {
            for b in &recs[k + 1..] {
                if self.rail.movers[a.i].mode & 0x40 != 0 || self.rail.movers[b.i].mode & 0x40 != 0 {
                    continue;
                }
                let d = ((a.pos.0 - b.pos.0).powi(2) + (a.pos.1 - b.pos.1).powi(2)).sqrt();
                if d < (a.r + b.r) * 0.6 {
                    out.push((a.i, b.i));
                }
            }
        }
        out
    }

    /// How far a tied-up boat is drawn from its berth tile, away from the deck.
    pub fn rest_shift(&self, m: &Mover) -> (f32, f32) {
        if m.line == 0 || m.lift != 1 {
            return (0.0, 0.0);
        }
        let pos = m.boat_pose().0;
        let t = (pos.0.floor() as i32, pos.1.floor() as i32);
        for j in &self.rail.isles.jetties {
            if j.berths().contains(&t) {
                let (ox, oy) = berth_outward(j, t);
                return (ox * 0.3, oy * 0.3);
            }
        }
        (0.0, 0.0)
    }

    /// The heading a boat tied up at a jetty lies in: along its deck.
    pub fn rest_heading(&self, m: &Mover) -> Option<(f32, f32)> {
        if m.line == 0 || m.lift != 1 || m.line as usize > self.rail.isles.routes.len() {
            return None;
        }
        let r = &self.rail.isles.routes[m.line as usize - 1];
        let len = m.route_len();
        let home = (m.travelled / len).round() as i64 % 2 == 0;
        let j = if home { Some(r.jetty) } else { r.other }?;
        let d = self.rail.isles.jetties.get(j)?.dir;
        let sign = if hash(m.set as u64 + m.line as u64) & 1 == 0 { 1.0 } else { -1.0 };
        Some((d.0 as f32 * sign, d.1 as f32 * sign))
    }
}

/// A boat's own lane across its route, in tiles: boats on one route do not
/// share a line.
fn lane_of(m: &Mover) -> f32 {
    let h = hash(m.set as u64 * 31 + m.line as u64 * 7 + 3) % 5;
    (h as f32 - 2.0) * 0.3
}

/// Which way is away from the deck at a berth tile (a unit step across the
/// deck), or nothing for a berth beyond the tip.
fn berth_outward(j: &Jetty, b: (i32, i32)) -> (f32, f32) {
    let p = j.perp();
    if j.deck.contains(&(b.0 - p.0, b.1 - p.1)) {
        (p.0 as f32, p.1 as f32)
    } else if j.deck.contains(&(b.0 + p.0, b.1 + p.1)) {
        (-p.0 as f32, -p.1 as f32)
    } else {
        (0.0, 0.0)
    }
}

fn is_boat(k: MoverKind) -> bool {
    matches!(k, MoverKind::Boat | MoverKind::Ferry | MoverKind::Ship)
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn path_len(p: &[(i32, i32)]) -> f32 {
    p.windows(2).map(|w| ((w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()) as f32).sum()
}

/// Reduce a tile-by-tile path to its corner points.
fn corners(tiles: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    for (i, &t) in tiles.iter().enumerate() {
        if i == 0 || i + 1 == tiles.len() {
            out.push(t);
            continue;
        }
        let (p, n) = (tiles[i - 1], tiles[i + 1]);
        if (t.0 - p.0, t.1 - p.1) != (n.0 - t.0, n.1 - t.1) {
            out.push(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pace;

    fn islands(seed: u64) -> Sim {
        let (w, h) = (7040, 1440);
        let vp = map::world_for_viewport(w, h);
        let focus = map::screen_to_tile(&vp, w as f32 * 0.5, h as f32 * 0.5);
        let mut s = Sim::with_focus_in(seed, Pace::Month, vp.w, vp.h, focus, Some(map::Biome::Islands));
        let sky = (0.30 * h as f32) as i32;
        s.set_viewport(vp.origin.0, vp.origin.1 + sky, w, h);
        s.set_horizon(sky, 160);
        s.ending.world_days = 1e9;
        s.run_to(8.0 * 86400.0);
        s.plan_rail();
        s.run_to(30.0 * 86400.0);
        s.plan_rail();
        s
    }

    fn lake_world(seed: u64) -> Sim {
        let (w, h) = (7040, 1440);
        let vp = map::world_for_viewport(w, h);
        let focus = map::screen_to_tile(&vp, w as f32 * 0.5, h as f32 * 0.5);
        let mut s = Sim::with_focus_in(seed, Pace::Month, vp.w, vp.h, focus, Some(map::Biome::Temperate));
        let sky = (0.30 * h as f32) as i32;
        s.set_viewport(vp.origin.0, vp.origin.1 + sky, w, h);
        s.set_horizon(sky, 160);
        s.ending.world_days = 1e9;
        s.run_to(8.0 * 86400.0);
        s.plan_rail();
        s.run_to(30.0 * 86400.0);
        s.plan_rail();
        s
    }

    /// The water never fills up: over 30 sim days the number of boats, ferries
    /// and ships stays bounded and `travelled` stays small.
    #[test]
    fn the_number_of_boats_stays_bounded_over_a_month() {
        for seed in [5u64, 7] {
            let mut s = islands(seed);
            s.set_sea_mood(75.0, 0.0);
            let (mut worst, mut worst_t) = (0usize, 0.0f32);
            for k in 0..(30 * 86400 / 10) {
                s.time += 10.0;
                s.transport_step(10.0);
                if k % 500 == 0 {
                    let n = s.rail.movers.iter().filter(|m| is_boat(m.kind)).count();
                    worst = worst.max(n);
                    for m in s.rail.movers.iter().filter(|m| is_boat(m.kind)) {
                        worst_t = worst_t.max(m.travelled / m.route_len().max(1.0));
                    }
                }
            }
            println!("seed {seed}: most boats at once {worst}, longest travelled {worst_t:.1} route lengths");
            assert!(worst <= 60, "seed {seed}: {worst} boats at once");
            assert!(worst_t < 20.0, "seed {seed}: travelled grew to {worst_t} route lengths");
        }
    }

    /// D91: over several islands seeds and sim hours no boat is ever off the
    /// water, boats do not sit on each other for long, and crashes stay a
    /// rarity. Prints the numbers the handback quotes.
    #[test]
    fn boats_keep_apart_and_stay_on_the_water() {
        let (mut samples_all, mut overlap_all, mut crashes, mut boats_seen) = (0u32, 0u32, 0u32, 0usize);
        for (seed, lake) in [(7u64, false), (3, false), (1, false), (9, false), (16, true)] {
            let mut s = if lake { lake_world(seed) } else { islands(seed) };
            s.set_sea_mood(84.0, 0.0);
            // A Saturday: day number d with (d + 3) mod 7 = 6 (see `is_weekend`).
            let day = (s.time / 86400.0).floor();
            let sat = (0..7).map(|k| day + k as f64).find(|d| ((d + 3.0) % 7.0) as i32 == 6).unwrap();
            s.time = sat * 86400.0 + 6.0 * 3600.0;
            for _ in 0..240 {
                s.transport_step(0.5);
            }
            let (mut overlap, mut samples) = (0u32, 0u32);
            // 06:00 to 18:00: dawn, the weekend peak and the evening.
            for k in 0..(12 * 3600 * 2) {
                s.time += 0.5;
                s.transport_step(0.5);
                if k % 4 == 0 {
                    let (pairs, land) = s.boat_audit();
                    assert_eq!(land, 0, "seed {seed}: a boat off the water");
                    samples += 1;
                    if pairs > 0 {
                        overlap += 1;
                    }
                }
            }
            boats_seen += s.rail.movers.iter().filter(|m| is_boat(m.kind)).count();
            for r in &s.rail.isles.routes {
                for &(x, y) in &r.path {
                    assert!(s.map.at(x, y).is_water(), "seed {seed}: a route corner on land at {x},{y}");
                }
            }
            println!(
                "seed {seed}{}: jetties {} routes {} boats {} overlap samples {overlap}/{samples} crashes {}",
                if lake { " (lake)" } else { "" },
                s.rail.isles.jetties.len(),
                s.rail.isles.routes.len(),
                s.rail.movers.iter().filter(|m| is_boat(m.kind)).count(),
                s.rail.isles.crashes
            );
            samples_all += samples;
            overlap_all += overlap;
            crashes += s.rail.isles.crashes;
            assert_eq!(overlap, 0, "seed {seed}: boats overlapped in {overlap} of {samples} samples");
        }
        let _ = (samples_all, overlap_all);
        assert!(boats_seen >= 20, "only {boats_seen} boats over four islands worlds");
        assert!(crashes <= 10, "{crashes} crashes in sixty sim hours is not rare");
    }
}
