//! Life stories (B48/D65): the residents are people with lives.
//!
//! Once a sim day (and only then) the roster is walked once: everyone is a
//! year older at the pace of [`LIFE_YEAR_DAYS`], the old die, couples have
//! babies, singles marry, grown children leave home, the retired stop working,
//! and in hard times whole households move away. Every choice is a pure
//! function of `(seed, day)` and the roster, so two machines growing the same
//! seed grow the same lives; and nothing here runs per frame.
//!
//! What happens is told twice. Each event with a human face becomes a
//! [`LifeScene`]: a line for the story feed, told when the scene starts, and
//! for weddings, funerals and moving trucks a small gathering the people
//! layer draws while it lasts (`People::scene_figures`). Rates are per life
//! year; per-day lines are capped so the feed stays a feed, not a ticker.

use crate::people::*;
use crate::{Lot, LotKind};
use map::rng::Rng;

/// What a scene is (and so how it is drawn).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SceneKind {
    /// A line for the feed only: a birth, a retirement, a death far from
    /// the cemetery.
    Note,
    /// A wedding party on the church steps (or the park, the square).
    Wedding,
    /// Mourners in dark clothes at the cemetery.
    Funeral,
    /// A moving truck at a front door, in or out.
    Move,
}

impl SceneKind {
    pub fn to_u8(self) -> u8 {
        match self {
            SceneKind::Note => 0,
            SceneKind::Wedding => 1,
            SceneKind::Funeral => 2,
            SceneKind::Move => 3,
        }
    }

    pub fn from_u8(v: u8) -> SceneKind {
        match v {
            1 => SceneKind::Wedding,
            2 => SceneKind::Funeral,
            3 => SceneKind::Move,
            _ => SceneKind::Note,
        }
    }
}

/// One thing that happens at a place for a while.
#[derive(Clone, Debug, PartialEq)]
pub struct LifeScene {
    pub kind: SceneKind,
    /// Sim seconds at which it begins and how long it lasts.
    pub start: f64,
    pub dur: f64,
    /// The tile the gathering is built around (a door, or the middle of a
    /// cemetery). `i32::MIN` for a scene with no place (a feed note).
    pub at: (i32, i32),
    /// Which way the building (or street) at `at` faces: outward is where the
    /// party stands.
    pub front: u8,
    /// The party stands on open ground (a cemetery, a park), not a street.
    pub open: bool,
    /// Varies the guests, the truck's driver, the confetti.
    pub seed: u32,
    /// The feed line, told when the scene starts ("" = none).
    pub line: String,
    pub told: bool,
}

/// A place the city holds a ceremony: where the party stands, which way the
/// building faces, what the feed calls it, and whether it is open ground.
#[derive(Clone, Copy)]
struct Venue {
    at: (i32, i32),
    front: u8,
    name: &'static str,
    open: bool,
}

/// Where the city holds its ceremonies (B48 follow-up: every church, hall,
/// green and cemetery, so a couple marries at the nearest and the dead are
/// buried at the nearest cemetery).
struct Civic {
    /// Chapels, churches and cathedrals, then town halls (the registry) and
    /// parks and plazas: the places to marry.
    wedding: Vec<Venue>,
    /// The places to bury, by kind: cemeteries first, then churchyards.
    cemeteries: Vec<Venue>,
    churchyards: Vec<Venue>,
    /// The oldest lot's door: a village with none of the above.
    first: Option<Venue>,
}

fn civic(lots: &[Lot]) -> Civic {
    use crate::landmarks::LandmarkKind as K;
    let mut c = Civic {
        wedding: Vec::new(),
        cemeteries: Vec::new(),
        churchyards: Vec::new(),
        first: None,
    };
    for l in lots {
        if !l.finished() {
            continue;
        }
        let door = |name: &'static str| Venue { at: door_tile(l), front: l.front % 4, name, open: false };
        let mid = |name: &'static str| Venue {
            at: (l.x + l.w / 2, l.y + l.h / 2),
            front: l.front % 4,
            name,
            open: true,
        };
        match l.kind {
            LotKind::Church => {
                c.wedding.push(door("the chapel"));
                c.churchyards.push(door("the churchyard"));
            }
            LotKind::Landmark(K::Cathedral) => {
                c.wedding.push(door("the cathedral"));
                c.churchyards.push(door("the cathedral close"));
            }
            LotKind::Hall => c.wedding.push(door("the town hall")),
            LotKind::Park | LotKind::Landmark(K::Plaza) => {
                // The party stands near the street edge of the green, not in
                // the middle: the middle is a pond, a fountain or a pitch.
                let (ox, oy): (i32, i32) = match l.front % 4 {
                    0 => (1, 0),
                    1 => (0, 1),
                    2 => (-1, 0),
                    _ => (0, -1),
                };
                let extent = if ox != 0 { l.w } else { l.h };
                let k = (extent / 2 - 1).max(1);
                let mut v = mid("the park");
                v.at = (l.x + l.w / 2 + ox * k, l.y + l.h / 2 + oy * k);
                c.wedding.push(v);
            }
            LotKind::Cemetery => c.cemeteries.push(mid("the cemetery")),
            _ => {}
        }
    }
    if let Some(l) = lots.first() {
        c.first = Some(Venue { at: door_tile(l), front: l.front % 4, name: "the front step", open: false });
    }
    c
}

/// The venue nearest (by street distance) to a home.
fn nearest(list: &[Venue], home: Option<(i32, i32)>) -> Option<Venue> {
    let (hx, hy) = home?;
    list.iter()
        .min_by_key(|v| (v.at.0 - hx).abs() + (v.at.1 - hy).abs())
        .copied()
}

const STREET_A: [&str; 24] = [
    "Elm", "Oak", "Mill", "Church", "Mercer", "Orchard", "Harbor", "Bridge", "Willow", "Maple",
    "Cedar", "Rail", "Market", "Linden", "Ash", "Quarry", "Spring", "Hill", "Garden", "River",
    "Chestnut", "Fern", "Heron", "Lantern",
];
const STREET_B: [&str; 8] = ["Row", "Lane", "Street", "Road", "Way", "Court", "Walk", "Avenue"];

/// The name of the street a door opens onto. Streets carry no names in the
/// map, so this reads the lane tile the door faces and names that line: one
/// street, one name, the same on every machine for a given seed.
pub fn street_name(w: &World, door: (i32, i32), front: u8) -> String {
    let (dx, dy) = match front % 4 {
        0 => (1, 0),
        1 => (0, 1),
        2 => (-1, 0),
        _ => (0, -1),
    };
    let mut line = if dx != 0 { door.0 } else { door.1 };
    for k in 0..=4 {
        let (x, y) = (door.0 + dx * k, door.1 + dy * k);
        if w.map.in_bounds(x, y) && w.map.at(x, y).kind.is_lane() {
            line = if dx != 0 { x } else { y };
            break;
        }
    }
    let axis = (front % 2) as u64;
    let mut rng = Rng::new(w.seed ^ 0x57EE_7000 ^ (axis << 40) ^ ((line as i64 as u64) << 8));
    format!(
        "{} {}",
        STREET_A[rng.usize(STREET_A.len())],
        STREET_B[rng.usize(STREET_B.len())]
    )
}

/// Chance per life year that a person of this age dies. Nothing before sixty
/// (the town has no war or plague to speak of), then the usual climb.
fn hazard(age: u16) -> f32 {
    match age {
        0..=59 => 0.0,
        60..=64 => 0.01,
        65..=69 => 0.03,
        70..=74 => 0.06,
        75..=79 => 0.10,
        80..=84 => 0.16,
        85..=89 => 0.26,
        _ => 0.42,
    }
}

/// (B52) The slump is the economy's: `Sim::in_slump`, handed to the life pass
/// as `People::econ_slump` before each hourly sync.

/// Per life year.
const BIRTH_RATE: f32 = 0.18;
const WEDDING_RATE: f32 = 0.14;
const MOVE_OUT_RATE: f32 = 0.20;
const LEAVE_RATE: f32 = 0.01;
/// The share of working-age adults without work above which the town is in
/// hard times and households start to leave.
pub const HARD_TIMES: f32 = 0.30;
/// Most feed lines the life events may add per sim day, and per kind.
pub(crate) const LINES_PER_DAY: u8 = 3;
const LINES_PER_KIND: u8 = 1;
/// Set in a moving scene's seed when the household is leaving, not arriving.
const LEAVING: u32 = 0x8000_0000;

fn per_day(rate: f32) -> f32 {
    rate / LIFE_YEAR_DAYS as f32
}

fn dir_of(dx: f32, dy: f32) -> u8 {
    if dx.abs() >= dy.abs() {
        if dx >= 0.0 { 0 } else { 2 }
    } else if dy >= 0.0 {
        1
    } else {
        3
    }
}

impl People {
    /// Share of 18-64 year olds who are not working and are not at home with
    /// a baby: the town's idea of hard times.
    pub fn jobless_share(&self) -> f32 {
        let mut adults = 0usize;
        let mut idle = 0usize;
        for (id, p) in self.list.iter().enumerate() {
            if p.alive() && p.kind == AgeGroup::Adult {
                adults += 1;
                if p.job.is_none() && !self.walkers.contains(&(id as u32)) {
                    idle += 1;
                }
            }
        }
        if adults == 0 {
            0.0
        } else {
            idle as f32 / adults as f32
        }
    }

    pub(crate) fn age_on(p: &Person, day: i64) -> u16 {
        (((day - p.born as i64) as f64) / LIFE_YEAR_DAYS).floor().max(0.0) as u16
    }

    /// Run the life pass for every sim day since the last one (at most a week
    /// at a time: a long catch-up replays the recent past, not all of it).
    /// `now` is the sim clock in seconds. Cheap when it is not a new day.
    pub fn life_pass(&mut self, w: &World, homes: &[HomeSlot], now: f64) {
        let day = (now / 86400.0).floor() as i64;
        if self.last_day >= day {
            return;
        }
        let from = (self.last_day + 1).max(day - 6);
        let c = civic(w.lots);
        for d in from..=day {
            self.one_day(w, homes, &c, d);
        }
        self.last_day = day;
        self.scenes.retain(|s| s.start + s.dur > now - 86400.0);
    }

    /// Lines whose scene has begun, oldest first; each is told once.
    pub fn due_lines(&mut self, now: f64) -> Vec<String> {
        let mut out = Vec::new();
        self.scenes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
        for s in self.scenes.iter_mut() {
            if !s.told && s.start <= now {
                s.told = true;
                if !s.line.is_empty() {
                    out.push(s.line.clone());
                }
            }
        }
        out
    }

    /// A household moved in at `door`: put a truck at the front door at the
    /// next 9 o'clock (a few to a day, so a boom shows a row of trucks, not a
    /// jam). `tell` gives the first one a feed line.
    pub fn moves_in(&mut self, w: &World, arrivals: &[Arrival], now: f64, tell: bool) {
        let day = (now / 86400.0).floor();
        let hour = (now / 3600.0).rem_euclid(24.0);
        let mut told = false;
        for a in arrivals {
            let Some(l) = w.lots.iter().find(|l| door_tile(l) == a.door) else {
                continue;
            };
            // The earliest morning slot with room for a truck.
            let mut d = if hour < 8.5 { day } else { day + 1.0 };
            let (start, ok) = loop {
                let on_day = self
                    .scenes
                    .iter()
                    .filter(|s| {
                        s.kind == SceneKind::Move && (s.start / 86400.0).floor() == d
                    })
                    .count();
                if on_day < 4 {
                    break (d * 86400.0 + (9.0 + on_day as f64 * 0.9) * 3600.0, true);
                }
                d += 1.0;
                if d > day + 3.0 {
                    break (0.0, false);
                }
            };
            if !ok {
                continue;
            }
            let street = street_name(w, a.door, l.front);
            let line = if tell && !told {
                told = true;
                let name = LAST[a.surname as usize % LAST.len()];
                format!("the {name} family moved into a house on {street}")
            } else {
                String::new()
            };
            self.scenes.push(LifeScene {
                kind: SceneKind::Move,
                start,
                dur: 5400.0,
                at: a.door,
                front: l.front % 4,
                open: false,
                seed: ((a.door.0 as u32).wrapping_mul(31)
                    ^ (a.door.1 as u32).wrapping_mul(17)
                    ^ a.surname as u32)
                    & 0x7FFF_FFFF,
                line,
                told: false,
            });
        }
    }

    fn one_day(&mut self, w: &World, homes: &[HomeSlot], c: &Civic, d: i64) {
        let mut rng = Rng::new(w.seed ^ (d as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x11FE_B48);
        let base = d as f64 * 86400.0;
        let n = self.list.len();
        // Beds: who sleeps where, so a move or a birth finds a free one.
        let mut cap: std::collections::BTreeMap<(i32, i32), u32> = std::collections::BTreeMap::new();
        for h in homes {
            cap.insert(h.door, h.capacity);
        }
        let mut occ: std::collections::BTreeMap<(i32, i32), u32> = std::collections::BTreeMap::new();
        // A baby sleeps in the parents' room: only toddlers and up take a bed.
        for p in self.list.iter() {
            if p.alive() && p.age >= 2 {
                if let Some(h) = p.home {
                    *occ.entry(h).or_insert(0) += 1;
                }
            }
        }
        let mut budget = LINES_PER_DAY;
        let mut per_kind = [0u8; 8];
        let mut say = |kind: usize, budget: &mut u8| -> bool {
            if *budget == 0 || per_kind[kind] >= LINES_PER_KIND {
                return false;
            }
            *budget -= 1;
            per_kind[kind] += 1;
            true
        };
        let mut notes: Vec<LifeScene> = Vec::new();
        let note = |notes: &mut Vec<LifeScene>, rng: &mut Rng, line: String| {
            notes.push(LifeScene {
                kind: SceneKind::Note,
                start: base + rng.range_f32(7.0, 20.0) as f64 * 3600.0,
                dur: 0.0,
                at: (i32::MIN, i32::MIN),
                front: 0,
                open: false,
                seed: 0,
                line,
                told: false,
            });
        };

        // 1. Another day older: birthdays, school's end, retirement.
        // 0. The economy: a slump comes round every few weeks; the works let
        //    hands go, and nobody is hired until it passes.
        let was = self.slump;
        self.slump = self.econ_slump;
        let mut laid_off = 0usize;
        if self.slump && !was {
            for (id, p) in self.list.iter_mut().enumerate() {
                if p.alive()
                    && p.kind == AgeGroup::Adult
                    && matches!(p.job_kind, JobKind::Factory | JobKind::Office | JobKind::Shop)
                    && (id as u64).wrapping_mul(0x9E37_79B9) >> 5 & 3 == (d as u64 & 3)
                {
                    p.job = None;
                    p.job_kind = JobKind::None;
                    laid_off += 1;
                }
            }
        }
        if laid_off > 0 && say(7, &mut budget) {
            note(
                &mut notes,
                &mut rng,
                format!("a slump: the works and the counting houses let {laid_off} hands go"),
            );
        }
        let mut grown: Vec<u32> = Vec::new();
        let mut retired: Vec<(u32, JobKind)> = Vec::new();
        for id in 0..n {
            if !self.list[id].alive() {
                continue;
            }
            let age = Self::age_on(&self.list[id], d);
            let p = &mut self.list[id];
            if age == p.age {
                continue;
            }
            p.age = age;
            match (p.kind, age) {
                (AgeGroup::Child, 18..) => {
                    p.kind = AgeGroup::Adult;
                    // School is behind them; repair finds work on its next pass.
                    p.job = None;
                    p.job_kind = JobKind::None;
                    grown.push(id as u32);
                }
                (AgeGroup::Adult, 65..) => {
                    p.kind = AgeGroup::Elder;
                    if p.job.is_some() {
                        retired.push((id as u32, p.job_kind));
                    }
                    p.job = None;
                    p.job_kind = JobKind::None;
                }
                _ => {}
            }
        }

        // 2. Deaths of old age; funerals the next morning.
        for id in 0..n {
            let (age, alive) = (self.list[id].age, self.list[id].alive());
            if !alive {
                continue;
            }
            let h = hazard(age);
            if h <= 0.0 || rng.f32() >= per_day(h) {
                continue;
            }
            let p = self.list[id].clone();
            self.list[id].life = Life::Dead;
            self.list[id].job = None;
            self.list[id].job_kind = JobKind::None;
            if let Some(sp) = p.spouse {
                if let Some(s) = self.list.get_mut(sp as usize) {
                    s.spouse = None;
                }
                self.list[id].spouse = None;
            }
            if let Some(h) = p.home {
                if let Some(o) = occ.get_mut(&h) {
                    *o = o.saturating_sub(1);
                }
            }
            if !say(0, &mut budget) {
                continue;
            }
            let name = p.name();
            let venue = nearest(&c.cemeteries, p.home).or_else(|| nearest(&c.churchyards, p.home));
            match venue {
                Some(v) => {
                    let street = street_name(w, v.at, v.front);
                    let phrase = match rng.usize(4) {
                        0 => format!("was laid to rest at {} on {street}", v.name),
                        1 => format!("was buried at {} on {street}", v.name),
                        2 => format!("was seen off by the whole street at {} on {street}", v.name),
                        _ => format!("had a quiet funeral at {} on {street}", v.name),
                    };
                    self.scenes.push(LifeScene {
                        kind: SceneKind::Funeral,
                        start: base + 86400.0 + rng.range_f32(9.5, 11.0) as f64 * 3600.0,
                        dur: 4200.0,
                        at: v.at,
                        front: v.front,
                        open: v.open,
                        seed: rng.usize(1 << 20) as u32,
                        line: format!("{name} ({age}) {phrase}"),
                        told: false,
                    });
                }
                None => note(&mut notes, &mut rng, format!("{name} ({age}) has died; the village buries its own")),
            }
        }

        // 3. Weddings: one woman, one man, no shared surname, a church to
        //    marry in.
        {
            let mut women: Vec<usize> = Vec::new();
            let mut men: Vec<usize> = Vec::new();
            for (id, p) in self.list.iter().enumerate() {
                if p.alive() && p.spouse.is_none() && (20..=45).contains(&p.age) {
                    if p.female() {
                        women.push(id);
                    } else {
                        men.push(id);
                    }
                }
            }
            let pairs = women.len().min(men.len()) as f32;
            let expect = pairs * per_day(WEDDING_RATE);
            let mut count = expect.floor() as usize + rng.chance(expect.fract()) as usize;
            let mut tries = 0;
            while count > 0 && tries < 12 && !women.is_empty() && !men.is_empty() {
                tries += 1;
                let wi = rng.usize(women.len());
                let mi = rng.usize(men.len());
                let (f, m) = (women[wi], men[mi]);
                let (fa, ma) = (&self.list[f], &self.list[m]);
                if fa.last == ma.last || (fa.age as i32 - ma.age as i32).abs() > 12 {
                    continue;
                }
                women.swap_remove(wi);
                men.swap_remove(mi);
                count -= 1;
                // Her name is his; the pair set up house together.
                let (fname, mname) = (self.list[f].given(), self.list[m].given());
                let last = self.list[m].last;
                self.list[f].last = last;
                self.list[f].spouse = Some(m as u32);
                self.list[m].spouse = Some(f as u32);
                let (fh, mh) = (self.list[f].home, self.list[m].home);
                let new_home = find_home(&mut rng, homes, &occ, 2, true)
                    .or_else(|| mh.filter(|h| free(&cap, &occ, *h) >= 1))
                    .or_else(|| fh.filter(|h| free(&cap, &occ, *h) >= 1))
                    .or(mh)
                    .or(fh);
                if let Some(door) = new_home {
                    for id in [f, m] {
                        if let Some(old) = self.list[id].home {
                            if let Some(o) = occ.get_mut(&old) {
                                *o = o.saturating_sub(1);
                            }
                        }
                        self.list[id].home = Some(door);
                        *occ.entry(door).or_insert(0) += 1;
                    }
                }
                if !say(1, &mut budget) {
                    continue;
                }
                let surname = self.list[m].surname();
                let home = self.list[m].home.or(self.list[f].home);
                let Some(v) = nearest(&c.wedding, home).or(c.first) else { continue };
                let place = if v.open {
                    "the park".to_string()
                } else {
                    format!("{} on {}", v.name, street_name(w, v.at, v.front))
                };
                let did = match rng.usize(4) {
                    0 => "married",
                    1 => "tied the knot",
                    2 => "said their vows",
                    _ => "were wed",
                };
                self.scenes.push(LifeScene {
                    kind: SceneKind::Wedding,
                    start: base + rng.range_f32(11.5, 14.0) as f64 * 3600.0,
                    dur: 5400.0,
                    at: v.at,
                    front: v.front,
                    open: v.open,
                    seed: rng.usize(1 << 20) as u32,
                    line: format!("{fname} and {mname} {surname} {did} at {place}"),
                    told: false,
                });
            }
        }

        // 4. Births: a couple with a spare bed and a mother of an age to.
        {
            let mut births: Vec<usize> = Vec::new();
            for (id, p) in self.list.iter().enumerate() {
                if !p.alive() || !p.female() || !(20..=40).contains(&p.age) {
                    continue;
                }
                let Some(sp) = p.spouse else { continue };
                let Some(h) = p.home else { continue };
                if !self.list.get(sp as usize).map(|s| s.alive()).unwrap_or(false) {
                    continue;
                }
                if free(&cap, &occ, h) < 1 {
                    continue;
                }
                if rng.f32() < per_day(BIRTH_RATE) {
                    births.push(id);
                }
            }
            for mid in births {
                let Some(door) = self.list[mid].home else { continue };
                if free(&cap, &occ, door) < 1 || self.list.len() >= MAX_PEOPLE * 2 {
                    continue;
                }
                let girl = rng.chance(0.5);
                let mut first = 0u8;
                for _ in 0..64 {
                    let i = rng.usize(FIRST.len()) as u8;
                    if Person::female_index(i) == girl {
                        first = i;
                        break;
                    }
                }
                let mother = self.list[mid].clone();
                let father = mother.spouse.and_then(|s| self.list.get(s as usize)).cloned();
                let id = self.list.len() as u32;
                self.list.push(Person {
                    first,
                    last: mother.last,
                    age: 0,
                    kind: AgeGroup::Child,
                    home: Some(door),
                    job: None,
                    job_kind: JobKind::None,
                    spouse: None,
                    parent: Some(mid as u32),
                    tone: rng.usize(TONES) as u8,
                    jitter: rng.usize(12) as u8,
                    born: d as i32,
                    life: Life::Alive,
                });
                // The mother stays home with the baby: a pram walks the pavement.
                self.list[mid].job = None;
                self.list[mid].job_kind = JobKind::None;
                let _ = id;
                if say(2, &mut budget) {
                    let kid = if girl { "a daughter" } else { "a son" };
                    let kname = FIRST[first as usize].0;
                    let fname = father.map(|f| f.given().to_string());
                    let parents = match fname {
                        Some(f) => format!("{} and {} {}", mother.given(), f, mother.surname()),
                        None => format!("{} {}", mother.given(), mother.surname()),
                    };
                    note(&mut notes, &mut rng, format!("{kid}, {kname}, was born to {parents}"));
                }
            }
        }

        // 5. Grown children leaving home for a place of their own.
        {
            let mut out: Vec<usize> = Vec::new();
            for (id, p) in self.list.iter().enumerate() {
                if !p.alive() || p.spouse.is_some() || !(20..=30).contains(&p.age) {
                    continue;
                }
                let Some(par) = p.parent else { continue };
                let lives_with = self
                    .list
                    .get(par as usize)
                    .map(|q| q.alive() && q.home == p.home)
                    .unwrap_or(false);
                if lives_with && rng.f32() < per_day(MOVE_OUT_RATE) {
                    out.push(id);
                }
            }
            for id in out {
                // A flat in a block, or a whole empty house, whichever is free.
                let Some(door) = find_home(&mut rng, homes, &occ, 1, false)
                    .filter(|d| cap.get(d).copied().unwrap_or(0) > 4 || occ.get(d).copied().unwrap_or(0) == 0)
                else {
                    continue;
                };
                if let Some(old) = self.list[id].home {
                    if let Some(o) = occ.get_mut(&old) {
                        *o = o.saturating_sub(1);
                    }
                }
                self.list[id].home = Some(door);
                self.list[id].parent = None;
                *occ.entry(door).or_insert(0) += 1;
                if say(3, &mut budget) {
                    let p = &self.list[id];
                    let street = lot_front(w, door)
                        .map(|f| street_name(w, door, f))
                        .unwrap_or_else(|| "the high street".to_string());
                    let line = format!("{} left the family home for a place of their own on {street}", p.name());
                    note(&mut notes, &mut rng, line);
                }
            }
        }

        // 5b. A child left with no grown-up in the house is taken in by
        //     relatives elsewhere; nobody lives alone at nine.
        {
            let mut grown_up: std::collections::BTreeSet<(i32, i32)> =
                std::collections::BTreeSet::new();
            for p in self.list.iter() {
                if p.alive() && p.kind != AgeGroup::Child {
                    if let Some(h) = p.home {
                        grown_up.insert(h);
                    }
                }
            }
            for p in self.list.iter_mut() {
                if p.alive()
                    && p.kind == AgeGroup::Child
                    && p.home.map(|h| !grown_up.contains(&h)).unwrap_or(false)
                {
                    p.life = Life::Away;
                    p.job = None;
                    p.job_kind = JobKind::None;
                }
            }
        }

        // 6. Leaving: in hard times whole households pack up; in good times
        //    now and then one does anyway.
        {
            let hard = self.slump || self.jobless_share() > HARD_TIMES;
            let mut by_home: std::collections::BTreeMap<(i32, i32), Vec<usize>> =
                std::collections::BTreeMap::new();
            for (id, p) in self.list.iter().enumerate() {
                if p.alive() {
                    if let Some(h) = p.home {
                        by_home.entry(h).or_default().push(id);
                    }
                }
            }
            for (door, members) in by_home {
                let adults: Vec<usize> = members
                    .iter()
                    .copied()
                    .filter(|&i| self.list[i].kind == AgeGroup::Adult)
                    .collect();
                if adults.is_empty() {
                    continue;
                }
                let idle = adults.iter().all(|&i| self.list[i].job.is_none());
                let p = if hard && idle { per_day(LEAVE_RATE) * 8.0 } else { per_day(LEAVE_RATE) };
                if rng.f32() >= p {
                    continue;
                }
                let surname = self.list[adults[0]].surname();
                for &i in &members {
                    self.list[i].life = Life::Away;
                    self.list[i].job = None;
                    self.list[i].job_kind = JobKind::None;
                    // Their partners, if any, went with them: no stale links.
                    self.list[i].spouse = None;
                }
                occ.insert(door, 0);
                if !say(4, &mut budget) {
                    continue;
                }
                let front = lot_front(w, door).unwrap_or(1);
                let street = street_name(w, door, front);
                let why = if hard && idle { "there is no work here" } else { "for a new start elsewhere" };
                self.scenes.push(LifeScene {
                    kind: SceneKind::Move,
                    start: base + rng.range_f32(9.0, 11.0) as f64 * 3600.0,
                    dur: 5400.0,
                    at: door,
                    front,
                    open: false,
                    seed: rng.usize(1 << 20) as u32 | LEAVING,
                    line: format!("the {surname} family loaded up the truck on {street}: {why}"),
                    told: false,
                });
            }
        }

        // 7. The small notes: grown-up children and retirements.
        for id in grown {
            if say(5, &mut budget) {
                let p = &self.list[id as usize];
                let line = format!("{} turned eighteen: school is behind them", p.name());
                note(&mut notes, &mut rng, line);
            }
        }
        for (id, job_kind) in retired {
            if say(6, &mut budget) {
                let p = self.list[id as usize].clone();
                let how = match job_kind {
                    JobKind::Shop => "locked the shop door for the last time",
                    JobKind::Factory => "clocked out of the works for good",
                    JobKind::Office => "cleared the desk and retired",
                    JobKind::Farm => "hung up the plough",
                    _ => "retired",
                };
                note(&mut notes, &mut rng, format!("{} ({}) {how}", p.name(), p.age));
            }
        }
        self.scenes.extend(notes);
        self.recount();
    }

    /// The figures a scene puts on the street at `time` (sim seconds).
    pub(crate) fn scene_figures(&self, time: f64, out: &mut Vec<Figure>) {
        for s in self.scenes.iter() {
            if s.at.0 == i32::MIN || time < s.start || time >= s.start + s.dur {
                continue;
            }
            let t = ((time - s.start) / s.dur.max(1.0)) as f32;
            let (cx, cy) = (s.at.0 as f32 + 0.5, s.at.1 as f32 + 0.5);
            // Outward (where a crowd stands) and lateral (along the frontage).
            let (ox, oy) = match s.front % 4 {
                0 => (1.0f32, 0.0f32),
                1 => (0.0, 1.0),
                2 => (-1.0, 0.0),
                _ => (0.0, -1.0),
            };
            let (lx, ly) = (-oy, ox);
            let pos = |u: f32, v: f32| (cx + ox * u + lx * v, cy + oy * u + ly * v);
            let rn = |k: u32| -> f32 {
                let h = (s.seed.wrapping_add(k.wrapping_mul(0x9E37_79B1))).wrapping_mul(0x85EB_CA6B);
                ((h >> 8) & 0xFFFF) as f32 / 65535.0
            };
            let fig = |x: f32, y: f32, pose: Pose, tone: u8, prop: Prop, dir: u8| Figure {
                x,
                y,
                pose,
                tone,
                child: false,
                prop,
                dir,
                open: s.open,
                lift: 0,
            };
            match s.kind {
                SceneKind::Note => {}
                SceneKind::Wedding => {
                    // The couple on the steps, the officiant behind them, the
                    // guests in an arc facing them; confetti once they leave.
                    let toward_in = (s.front + 2) % 4;
                    let (bx, by) = pos(0.25, -0.22);
                    let (gx, gy) = pos(0.25, 0.22);
                    out.push(fig(bx, by, Pose::Stand, TONE_WHITE, Prop::None, s.front % 4));
                    out.push(fig(gx, gy, Pose::Stand, TONE_DARK, Prop::None, s.front % 4));
                    // The officiant stands between them, under a flower arch.
                    let (px, py) = pos(0.05, 0.0);
                    out.push(fig(
                        px,
                        py,
                        Pose::Stand,
                        6,
                        Prop::Arch,
                        // The arch runs along the frontage: its mirror picks the axis.
                        if s.front % 2 == 1 { 0 } else { 2 },
                    ));
                    let _ = toward_in;
                    let guests = 5 + (s.seed % 3) as usize;
                    for k in 0..guests {
                        let a = -1.15 + 2.3 * k as f32 / (guests - 1) as f32;
                        let (u, v) = (0.25 + 0.75 * a.cos(), 0.95 * a.sin());
                        let (x, y) = pos(u, v);
                        // Face the couple.
                        let dir = dir_of(
                            (cx + ox * 0.25 + 0.0) - x,
                            (cy + oy * 0.25 + 0.0) - y,
                        );
                        let throwing = t > 0.55 && k % 2 == 1;
                        out.push(fig(
                            x,
                            y,
                            if throwing { Pose::Wave } else if k % 3 == 0 { Pose::Chat } else { Pose::Stand },
                            ((s.seed as usize + k * 3) % TONES) as u8,
                            if throwing { Prop::Confetti } else { Prop::None },
                            dir,
                        ));
                    }
                    // A child with the guests, waving a ball? Not at a wedding.
                }
                SceneKind::Funeral => {
                    // Dark coats round a grave, one standing apart at its head.
                    let n = 5 + (s.seed % 3) as usize;
                    for k in 0..n {
                        let a = 0.5 + 6.28 * k as f32 / n as f32;
                        let (x, y) = (cx + a.cos() * 0.7, cy + a.sin() * 0.55);
                        out.push(fig(
                            x,
                            y,
                            Pose::Stand,
                            TONE_DARK,
                            Prop::None,
                            dir_of(cx - x, cy - y),
                        ));
                    }
                    let (px, py) = (cx, cy - 0.45);
                    out.push(fig(px, py, Pose::Stand, 6, Prop::None, 1));
                    // A hearse parked at the gate, driver at the wheel.
                    if s.open {
                        let d = if s.front % 2 == 0 { 1.5 } else { 1.0 };
                        let (hx, hy) = (cx + ox * (d + 0.8) + lx * 0.9, cy + oy * (d + 0.8) + ly * 0.9);
                        out.push(fig(hx, hy, Pose::Stand, TONE_DARK, Prop::Hearse, if s.front % 2 == 1 { 0 } else { 2 }));
                    }
                }
                SceneKind::Move => {
                    // The truck at the kerb, two movers walking boxes between it
                    // and the door, the household waiting on the step.
                    let along_x = s.front % 2 == 1;
                    // Mirror selects the truck's long axis (0/1 = along x).
                    let truck_dir: u8 = if along_x { 0 } else { 2 };
                    let (tx, ty) = pos(1.15, 0.55);
                    out.push(fig(tx, ty, Pose::Stand, 7, Prop::Truck, truck_dir));
                    let leaving = s.seed & LEAVING != 0;
                    for k in 0..2u32 {
                        let tri = {
                            let q = (time as f32 * 0.04 + k as f32 * 0.5).rem_euclid(2.0);
                            if q < 1.0 { q } else { 2.0 - q }
                        };
                        let (x, y) = pos(0.15 + 0.85 * tri, -0.25 + 0.5 * k as f32 * 0.8);
                        out.push(fig(
                            x,
                            y,
                            Pose::Carry,
                            ((s.seed + k) % 8) as u8,
                            Prop::None,
                            if (tri > 0.5) ^ leaving { s.front % 4 } else { (s.front + 2) % 4 },
                        ));
                    }
                    if !leaving {
                        let (x, y) = pos(0.05, 0.35);
                        out.push(fig(x, y, Pose::Stand, (s.seed % 8) as u8, Prop::None, s.front % 4));
                        let (x, y) = pos(0.1, 0.65);
                        out.push(Figure {
                            child: true,
                            ..fig(x, y, Pose::Play, ((s.seed >> 3) % 8) as u8, Prop::Ball, 1)
                        });
                    }
                    let _ = rn(0);
                }
            }
        }
    }
}

/// How many beds are free at `door`.
fn free(
    cap: &std::collections::BTreeMap<(i32, i32), u32>,
    occ: &std::collections::BTreeMap<(i32, i32), u32>,
    door: (i32, i32),
) -> u32 {
    cap.get(&door)
        .copied()
        .unwrap_or(0)
        .saturating_sub(occ.get(&door).copied().unwrap_or(0))
}

/// A home with at least `min_free` free beds (and, if asked, nobody in it),
/// starting the search at a random one so newcomers spread over the town.
fn find_home(
    rng: &mut Rng,
    homes: &[HomeSlot],
    occ: &std::collections::BTreeMap<(i32, i32), u32>,
    min_free: u32,
    empty: bool,
) -> Option<(i32, i32)> {
    if homes.is_empty() {
        return None;
    }
    let start = rng.usize(homes.len());
    for k in 0..homes.len() {
        let h = &homes[(start + k) % homes.len()];
        let o = occ.get(&h.door).copied().unwrap_or(0);
        if h.capacity.saturating_sub(o) >= min_free && (!empty || o == 0) {
            return Some(h.door);
        }
    }
    None
}

/// The facing of the lot whose door is at `door`, if any is in the roster of
/// lots the caller can see. (Streets are named from the facing.)
fn lot_front(_w: &World, _door: (i32, i32)) -> Option<u8> {
    _w.lots.iter().find(|l| door_tile(l) == _door).map(|l| l.front % 4)
}

