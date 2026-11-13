//! Civics, economy and pollution (B52, D67/D75): the part of a city that is
//! made of decisions rather than bricks.
//!
//! * **Economy.** One index (`mood`, -1 bust .. +1 boom) driven by the city:
//!   a business cycle, jobs against residents, shocks, smog. It replaces B48's
//!   placeholder cosine. Named companies hold the office towers: they expand in
//!   booms, close in busts (their offices go dark), and new firms move in during
//!   the recovery. A share of shops/offices is vacant in proportion to the bust.
//! * **Government.** A mayor (a named resident) with terms; an election every
//!   ten days or so with a campaign, a rally in the square and an election
//!   night; a ballot measure that decides a civic project, which then goes up
//!   through the ordinary construction stages.
//! * **Pollution.** Industry and traffic build smog; the council answers with
//!   scrubbers, trees and electric buses.
//! * **Incidents.** A building fire and a district blackout, each with a warning,
//!   the event, and recovery crews.
//!
//! Everything is decided on the **planning step** (once a sim day, plus a time
//! check per step); nothing here runs per frame. What the renderer needs is a
//! set of cheap read-only queries at the bottom of this file.

use map::rng::{hash2, Rng};

use crate::chronicle::Topic;
use crate::landmarks::LandmarkKind;
use crate::people::{self, AgeGroup};
use crate::{Era, LotKind, Sim};

const DAY: f64 = 86_400.0;
const HOUR: f64 = 3600.0;

/// Below this mood the town is in a slump: the works lay hands off.
pub const SLUMP_BELOW: f32 = -0.45;
/// Above this the town is booming.
pub const BOOM_ABOVE: f32 = 0.5;

/// The cleanups the council can order (bit flags in `Civics::cleanups`).
pub const CLEAN_SCRUBBERS: u8 = 1;
pub const CLEAN_TREES: u8 = 2;
pub const CLEAN_BUSES: u8 = 4;

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// A named firm. It owns one office or tower (`hq`, the lot's anchor tile).
#[derive(Clone, Debug, PartialEq)]
pub struct Company {
    /// Indices into [`NAME_A`] and the table `kind` picks.
    pub a: u8,
    pub b: u8,
    /// 0 old-fashioned firm, 1 modern firm, 2 bank.
    pub kind: u8,
    pub hq: (i32, i32),
    /// 0..1: how well the firm is doing.
    pub health: f32,
    /// Extra floors the firm has taken (0..=3): drawn as lit floors.
    pub floors: u8,
    pub closed: bool,
    /// Sim day the firm closed (so a new tenant waits a few days).
    pub closed_day: i32,
}

const NAME_A: [&str; 24] = [
    "Halden", "Brightwater", "Orrin", "Tallow", "Quillon", "Marrow", "Lowell", "Verity", "Ashgrove",
    "Pennant", "Corvin", "Eastmere", "Thistle", "Ravenel", "Dunmore", "Kestrel", "Sable", "Wexley",
    "Alder", "Penrose", "Calder", "Fenwick", "Garrow", "Maple Cross",
];
const NAME_OLD: [&str; 8] = [
    "& Daughters", "Milling", "Provisions", "Freight", "Foundry", "Chandlers", "Timber", "Cartage",
];
const NAME_MODERN: [&str; 8] = [
    "Group", "Logistics", "Systems", "Labs", "Capital", "Media", "Energy", "Networks",
];
const NAME_BANK: [&str; 4] = ["Savings Bank", "Trust", "Mutual Bank", "Credit Union"];

impl Company {
    pub fn name(&self) -> String {
        let tail = match self.kind {
            2 => NAME_BANK[self.b as usize % NAME_BANK.len()],
            1 => NAME_MODERN[self.b as usize % NAME_MODERN.len()],
            _ => NAME_OLD[self.b as usize % NAME_OLD.len()],
        };
        format!("{} {}", NAME_A[self.a as usize % NAME_A.len()], tail)
    }

    pub fn is_bank(&self) -> bool {
        self.kind == 2
    }
}

/// The mayor, and the slate they ran on (0 Lantern, 1 Compass).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mayor {
    /// Index in the people list.
    pub id: u32,
    pub slate: u8,
    pub since_day: i32,
}

pub const SLATES: [&str; 2] = ["Lantern", "Compass"];

/// What a ballot measure can build. Each maps to a landmark, or a park.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Project {
    Park,
    Stadium,
    Museum,
    FerrisWheel,
    Zoo,
}

impl Project {
    pub const ALL: [Project; 5] = [
        Project::Park,
        Project::Stadium,
        Project::Museum,
        Project::FerrisWheel,
        Project::Zoo,
    ];

    pub fn id(self) -> u8 {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0) as u8
    }

    pub fn from_id(i: u8) -> Project {
        Self::ALL[(i as usize) % Self::ALL.len()]
    }

    pub fn phrase(self) -> &'static str {
        match self {
            Project::Park => "a new city park",
            Project::Stadium => "a new stadium",
            Project::Museum => "a city museum",
            Project::FerrisWheel => "a riverside wheel",
            Project::Zoo => "a city zoo",
        }
    }

    pub fn landmark(self) -> Option<LandmarkKind> {
        match self {
            Project::Park => None,
            Project::Stadium => Some(LandmarkKind::Stadium),
            Project::Museum => Some(LandmarkKind::Museum),
            Project::FerrisWheel => Some(LandmarkKind::FerrisWheel),
            Project::Zoo => Some(LandmarkKind::Zoo),
        }
    }
}

/// An election: two candidates, a day, and the measure on the ballot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Election {
    /// The sim day of the vote.
    pub day: i32,
    /// People-list ids and their slates.
    pub cand: [(u32, u8); 2],
    pub measure: Option<Project>,
    /// Bits: 1 campaign told, 2 rally told, 4 result told.
    pub told: u8,
}

/// A building fire: where, and when the first smoke showed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fire {
    pub x: i32,
    pub y: i32,
    pub t0: f64,
    /// Engines sent (so traffic dispatches them once).
    pub dispatched: bool,
    /// Bits: 1 smoke told, 2 flames told, 4 out told.
    pub told: u8,
}

/// A district blackout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blackout {
    pub cx: i32,
    pub cy: i32,
    pub r: i32,
    /// When the lights go.
    pub t0: f64,
    pub dur: f64,
    /// Bits: 1 warning told, 2 out told, 4 restored told.
    pub told: u8,
}

/// Everything B52 adds to the save (one block, `save.rs` v14).
#[derive(Clone, Debug, PartialEq)]
pub struct Civics {
    // economy
    pub mood: f32,
    pub vacancy: f32,
    pub shock: f32,
    pub shock_left: f32,
    /// 0 steady, 1 boom, 2 bust (the feed tells each episode once).
    pub phase: u8,
    pub companies: Vec<Company>,
    // pollution
    pub smog: f32,
    pub cleanups: u8,
    pub cleanup_day: i32,
    // government
    pub mayor: Option<Mayor>,
    pub election: Option<Election>,
    pub next_election: i32,
    pub elections_held: u8,
    /// Landmarks the voters approved (bit per `LandmarkKind::index`).
    pub approved: u32,
    /// Landmarks the voters turned down, until a few elections have passed.
    pub rejected: u32,
    pub parks_owed: u8,
    // incidents
    pub fire: Option<Fire>,
    pub blackout: Option<Blackout>,
    pub next_fire: i32,
    pub next_blackout: i32,
    /// The last sim day the daily update ran.
    pub day: i32,
    /// Counters for the handback and tests.
    pub fires_seen: u8,
    pub blackouts_seen: u8,
}

impl Default for Civics {
    fn default() -> Self {
        Civics {
            mood: 0.1,
            vacancy: 0.0,
            shock: 0.0,
            shock_left: 0.0,
            phase: 0,
            companies: Vec::new(),
            smog: 0.0,
            cleanups: 0,
            cleanup_day: -99,
            mayor: None,
            election: None,
            next_election: 0,
            elections_held: 0,
            approved: 0,
            rejected: 0,
            parks_owed: 0,
            fire: None,
            blackout: None,
            next_fire: 0,
            next_blackout: 0,
            day: -1,
            fires_seen: 0,
            blackouts_seen: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Time of an election
// ---------------------------------------------------------------------------

/// Where in the election cycle the clock is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// No election in sight.
    None,
    /// Posters are up.
    Campaign,
    /// The rally in the square, 0..1 through its two hours.
    Rally(f32),
    /// Polling day (a queue at the hall).
    Voting,
    /// Election night: the crowd in the square, 0..1 through the night.
    Night(f32),
}

const CAMPAIGN_DAYS: f64 = 4.0;
const RALLY_FROM: f64 = 17.0;
const RALLY_TO: f64 = 19.5;
const NIGHT_FROM: f64 = 20.0;
const NIGHT_TO: f64 = 23.5;
const RESULT_AT: f64 = 21.5;

// ---------------------------------------------------------------------------
// The economy
// ---------------------------------------------------------------------------

impl Sim {
    /// The economy's mood, -1 (bust) to +1 (boom).
    pub fn economy_mood(&self) -> f32 {
        self.civics.mood
    }

    /// The mood in a few words, for the tray.
    pub fn mood_word(&self) -> &'static str {
        let m = self.civics.mood;
        if m > BOOM_ABOVE {
            "boom times"
        } else if m > 0.2 {
            "good times"
        } else if m > -0.2 {
            "steady"
        } else if m > SLUMP_BELOW {
            "hard times"
        } else {
            "slump"
        }
    }

    /// Is the town in a slump today? (The works lay hands off.)
    pub fn in_slump(&self) -> bool {
        self.civics.mood < SLUMP_BELOW
    }

    /// The mayor's name, if the town has one.
    pub fn mayor_name(&self) -> Option<String> {
        let m = self.civics.mayor?;
        self.people.list.get(m.id as usize).map(|p| p.name())
    }

    /// The tray's headline: the mayor and the economy's mood in a few words.
    pub fn civic_headline(&self) -> String {
        match self.mayor_name() {
            Some(n) => format!("Mayor {n} · {}", self.mood_word()),
            None => format!("no mayor yet · {}", self.mood_word()),
        }
    }

    /// The smog over the town, 0 clean .. 1 thick.
    pub fn smog(&self) -> f32 {
        self.civics.smog
    }

    /// Is this lot standing empty? A firm's HQ is empty when the firm has
    /// closed; every other shop, store, office, tower and works is empty when
    /// its rank falls under the town's vacancy, so the bust empties the same
    /// buildings in the same order every time and a recovery refills them in
    /// reverse.
    pub fn lot_vacant(&self, l: &crate::Lot) -> bool {
        if !l.finished() {
            return false;
        }
        let scale = match l.kind {
            LotKind::Shop | LotKind::Store => 1.0,
            LotKind::Office => 0.8,
            LotKind::Industry => 0.55,
            LotKind::Tower => 0.45,
            _ => return false,
        };
        if matches!(l.kind, LotKind::Office | LotKind::Tower) {
            if let Some(c) = self.civics.companies.iter().find(|c| c.hq == (l.x, l.y)) {
                return c.closed;
            }
        }
        let rank = hash2(l.x, l.y, self.seed ^ 0xB52_7AC);
        rank < self.civics.vacancy * scale
    }

    /// The company that holds a lot, if any.
    pub fn company_at(&self, x: i32, y: i32) -> Option<&Company> {
        self.civics.companies.iter().find(|c| c.hq == (x, y))
    }

    /// Run the day's civic update if a new sim day has begun, and check the
    /// clock-driven events (elections, fires, blackouts). Called every planning
    /// step: a comparison and a few branches when nothing is due.
    pub fn civics_step(&mut self) {
        if self.lots.len() < 12 || self.ending_underway() {
            return;
        }
        let day = (self.time / DAY).floor() as i32;
        if day > self.civics.day {
            self.civics.day = day;
            self.civics_day(day);
        }
        self.election_step(day);
        self.incident_step(day);
    }

    /// A civic feed line, told once. The tray shows a few lines a day (B48a),
    /// so a day that already has plenty drops the soft lines (a firm taking a
    /// floor) and keeps the ones that matter (a result, a fire, a blackout).
    fn civic_line(&mut self, text: impl Into<String>) {
        let day = (self.time / DAY).floor() as i64;
        let today = self.story.iter().filter(|l| (l.time / DAY).floor() as i64 == day).count();
        let text: String = text.into();
        let critical = text.contains("Measure") || text.contains("ballot") || text.contains("elected") || text.contains("unseats") || text.contains("fire") || text.contains("blackout") || text.contains("smoke");
        let topic = civic_topic(&text);
        if today >= if critical { 8 } else { 5 } {
            // The feed has had enough today; the chronicle still keeps the
            // events that matter to a history (B51).
            if !matches!(topic, Topic::Civic | Topic::Company | Topic::Measure) {
                self.chron(topic, 0, &text);
            }
            return;
        }
        self.push_story_t(topic, 0, text);
    }

    fn civics_day(&mut self, day: i32) {
        self.economy_day(day);
        self.pollution_day(day);
        self.companies_day(day);
        self.ending_pressure();
    }

    /// B52b: the city's own fortunes move its ending. A world that is to end
    /// in Decline is brought nearer by every day of slump and eased back by good
    /// times; one that is to end in Smog by every day of thick air (and eased by
    /// the council's cleanups). `world_days` is what the save already keeps, so
    /// this is deterministic and saved; it never pulls the end closer than the
    /// warning plus two days from now, and never past 45 days.
    fn ending_pressure(&mut self) {
        use crate::ending::Ending;
        let (hot, calm) = match self.ending.kind {
            Ending::Decline => (self.civics.mood < SLUMP_BELOW, self.civics.mood > 0.3),
            Ending::Smog => (self.civics.smog > 0.6, self.civics.smog < 0.25),
            _ => return,
        };
        let floor = (self.time + self.ending.kind.timing().warn + 2.0 * DAY) / DAY;
        let w = &mut self.ending.world_days;
        if hot {
            *w = (*w - 0.35).max(floor.min(*w));
        } else if calm {
            *w = (*w + 0.1).min(45.0f64.max(*w));
        }
    }

    /// The business cycle, jobs, shocks and smog fold into one number.
    fn economy_day(&mut self, day: i32) {
        let residents = self.residents() as f32;
        // A hamlet has no business cycle to speak of.
        let maturity = ((residents - 60.0) / 340.0).clamp(0.0, 1.0);
        let mut rng = Rng::new(self.seed ^ 0xEC0_0000 ^ (day as u64).wrapping_mul(0x9E37_79B9));
        let c = &mut self.civics;
        if c.shock_left > 0.0 {
            c.shock_left -= 1.0;
            if c.shock_left <= 0.0 {
                c.shock = 0.0;
            }
        } else if maturity > 0.3 && rng.chance(0.07) {
            c.shock = if rng.chance(0.5) { 0.45 } else { -0.45 };
            c.shock_left = rng.range(3, 6) as f32;
        }
        let period = 12.0 + (self.seed % 5) as f64;
        let phase = (self.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f64 / (1u64 << 24) as f64;
        let cycle = 0.62 * ((day as f64 / period + phase) * std::f64::consts::TAU).cos() as f32;
        let jobless = self.people.jobless_share();
        let labour = ((0.22 - jobless) * 1.5).clamp(-0.4, 0.4);
        let c = &self.civics;
        let target = (cycle + labour + c.shock - 0.3 * c.smog) * maturity;
        let c = &mut self.civics;
        c.mood += (target.clamp(-1.0, 1.0) - c.mood) * 0.4;
        // Vacancy follows the bust slowly and recovers a little faster.
        let vac_target = ((-c.mood - 0.15) / 0.85).clamp(0.0, 1.0) * 0.6;
        let step = if vac_target > c.vacancy { 0.05 } else { 0.04 };
        c.vacancy += (vac_target - c.vacancy).clamp(-step, step);
        // The feed tells each episode once.
        let (phase, mood) = (c.phase, c.mood);
        if phase != 1 && mood > BOOM_ABOVE {
            c.phase = 1;
            self.civic_line(format!("business is booming in {}: new orders, full offices", self.city_name));
        } else if phase != 2 && mood < SLUMP_BELOW {
            c.phase = 2;
            self.civic_line("hard times: orders dry up, and shops begin to close");
        } else if phase == 2 && mood > 0.0 {
            c.phase = 0;
            self.civic_line("the slump is over: the empty shopfronts begin to fill again");
        } else if phase == 1 && mood < 0.25 {
            c.phase = 0;
        }
    }

    fn pollution_day(&mut self, day: i32) {
        let industry = self
            .lots
            .iter()
            .filter(|l| l.finished() && l.kind == LotKind::Industry)
            .count() as f32;
        let residents = self.residents() as f32;
        let ev = self.tech.vehicle_era() >= 3;
        let c = &self.civics;
        let mut ind = 0.05 * industry;
        if c.cleanups & CLEAN_SCRUBBERS != 0 {
            ind *= 0.4;
        }
        let mut traffic = 0.24 * (residents / 900.0).min(1.5);
        if ev {
            traffic *= 0.55;
        }
        if c.cleanups & CLEAN_BUSES != 0 {
            traffic *= 0.8;
        }
        let mut target = ind + traffic;
        if c.cleanups & CLEAN_TREES != 0 {
            target -= 0.15;
        }
        let target = target.clamp(0.0, 1.0);
        let c = &mut self.civics;
        c.smog += (target - c.smog).clamp(-0.1, 0.12);
        c.smog = c.smog.clamp(0.0, 1.0);
        // The council acts when the air is bad and a measure is still open.
        if c.smog > 0.5 && day - c.cleanup_day >= 3 {
            let ev_done = self.tech.is_done(crate::tech::Advance::Evs);
            let c = &mut self.civics;
            let (bit, line) = if c.cleanups & CLEAN_SCRUBBERS == 0 && industry > 0.0 {
                (CLEAN_SCRUBBERS, "the council orders scrubbers fitted to the works' chimneys")
            } else if c.cleanups & CLEAN_TREES == 0 {
                (CLEAN_TREES, "the council plants trees along the avenues to clear the air")
            } else if c.cleanups & CLEAN_BUSES == 0 && ev_done {
                (CLEAN_BUSES, "the first electric buses take over the town's routes")
            } else {
                (0, "")
            };
            if bit != 0 {
                c.cleanups |= bit;
                c.cleanup_day = day;
                self.civic_line(line);
            }
        }
    }

    /// Firms are founded as the offices go up, rise and fall with the mood,
    /// and are replaced in the recovery.
    fn companies_day(&mut self, day: i32) {
        let era = self.era();
        if era < Era::Town {
            return;
        }
        let offices: Vec<(i32, i32, LotKind)> = self
            .lots
            .iter()
            .filter(|l| l.finished() && matches!(l.kind, LotKind::Office | LotKind::Tower))
            .map(|l| (l.x, l.y, l.kind))
            .collect();
        let mut rng = Rng::new(self.seed ^ 0xC0_FFEE ^ (day as u64).wrapping_mul(0xA24B_AED4));
        // Retire records of buildings that have gone.
        self.civics.companies.retain(|c| offices.iter().any(|o| (o.0, o.1) == c.hq));
        let want = (offices.len() / 2).min(9);
        let have = self.civics.companies.len();
        if have < want && rng.chance(0.6) {
            let free: Vec<&(i32, i32, LotKind)> = offices
                .iter()
                .filter(|o| !self.civics.companies.iter().any(|c| c.hq == (o.0, o.1)))
                .collect();
            if !free.is_empty() {
                let pick = free
                    .iter()
                    .min_by(|a, b| {
                        hash2(a.0, a.1, self.seed ^ 0xF1_4)
                            .partial_cmp(&hash2(b.0, b.1, self.seed ^ 0xF1_4))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .copied()
                    .copied();
                if let Some((x, y, k)) = pick {
                    let banks = self.civics.companies.iter().filter(|c| c.is_bank()).count();
                    let want_bank = banks == 0 && have >= 1 || banks == 1 && have >= 6;
                    let c = self.new_company((x, y), want_bank && k == LotKind::Office, &mut rng, day);
                    let name = c.name();
                    let bank = c.is_bank();
                    self.civics.companies.push(c);
                    self.civic_line(if bank {
                        format!("the {name} opens its doors downtown")
                    } else {
                        format!("{name} opens new offices")
                    });
                }
            }
        }
        // Fortunes.
        let mood = self.civics.mood;
        let mut lines: Vec<String> = Vec::new();
        for i in 0..self.civics.companies.len() {
            let mut noise = rng.f32();
            let c = &mut self.civics.companies[i];
            if c.closed {
                // A new tenant moves in once the town has recovered.
                if mood > 0.05 && day - c.closed_day >= 3 && noise < 0.4 {
                    let old = c.name();
                    let hq = c.hq;
                    let mut fresh = Company {
                        a: 0,
                        b: 0,
                        kind: 0,
                        hq,
                        health: 0.55,
                        floors: 0,
                        closed: false,
                        closed_day: 0,
                    };
                    fresh.a = rng.usize(NAME_A.len()) as u8;
                    fresh.b = rng.usize(8) as u8;
                    fresh.kind = if self.era() >= Era::City && rng.chance(0.7) { 1 } else { 0 };
                    let nm = fresh.name();
                    self.civics.companies[i] = fresh;
                    lines.push(format!("{nm} takes over the empty offices of {old}"));
                }
                continue;
            }
            noise -= 0.5;
            c.health = (c.health + mood * 0.07 + noise * 0.07 - 0.004).clamp(0.0, 1.0);
            if c.health > 0.78 && c.floors < 3 && mood > 0.3 && rng.chance(0.35) {
                c.floors += 1;
                lines.push(format!("{} takes another floor: business is good", c.name()));
            } else if c.health < 0.45 && c.floors > 0 {
                c.floors -= 1;
            }
            if c.health < 0.07 && mood < -0.3 {
                c.closed = true;
                c.closed_day = day;
                c.floors = 0;
                lines.push(if c.is_bank() {
                    format!("the {} has failed; its doors are shut", c.name())
                } else {
                    format!("{} closes its offices", c.name())
                });
            }
        }
        for l in lines.into_iter().take(2) {
            self.civic_line(l);
        }
    }

    fn new_company(&self, hq: (i32, i32), bank: bool, rng: &mut Rng, day: i32) -> Company {
        let modern = self.era() >= Era::City;
        let kind = if bank { 2 } else if modern && rng.chance(0.6) { 1 } else { 0 };
        let used: Vec<u8> = self.civics.companies.iter().map(|c| c.a).collect();
        let mut a = rng.usize(NAME_A.len()) as u8;
        for _ in 0..8 {
            if !used.contains(&a) {
                break;
            }
            a = rng.usize(NAME_A.len()) as u8;
        }
        let _ = day;
        Company {
            a,
            b: rng.usize(8) as u8,
            kind,
            hq,
            health: 0.6,
            floors: 0,
            closed: false,
            closed_day: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Government
    // -----------------------------------------------------------------------

    /// Is there a finished town hall to hold an election in?
    fn has_hall(&self) -> bool {
        self.lots
            .iter()
            .any(|l| l.finished() && l.kind == LotKind::Hall)
    }

    /// Time of day in hours.
    fn hour_of(&self) -> f64 {
        (self.time / HOUR).rem_euclid(24.0)
    }

    /// Where the election cycle is right now.
    pub fn election_phase(&self) -> Phase {
        let Some(e) = self.civics.election else {
            return Phase::None;
        };
        let t = self.time / DAY - e.day as f64;
        let h = self.hour_of();
        if t < -CAMPAIGN_DAYS || t >= 1.25 {
            return Phase::None;
        }
        // The day before: the rally.
        if (-1.0..0.0).contains(&t) && (RALLY_FROM..RALLY_TO).contains(&h) {
            return Phase::Rally(((h - RALLY_FROM) / (RALLY_TO - RALLY_FROM)) as f32);
        }
        if (0.0..1.0).contains(&t) {
            if h >= NIGHT_FROM && h < NIGHT_TO {
                return Phase::Night(((h - NIGHT_FROM) / (NIGHT_TO - NIGHT_FROM)) as f32);
            }
            if (7.0..19.0).contains(&h) {
                return Phase::Voting;
            }
        }
        // Posters stay up until the morning after.
        if t < 1.0 + 6.0 / 24.0 {
            return Phase::Campaign;
        }
        Phase::None
    }

    /// Is the square dressed for the rally? From the rally's start the evening
    /// before to the end of election night.
    pub fn election_dress(&self) -> bool {
        let Some(e) = self.civics.election else { return false };
        let t = self.time / DAY - e.day as f64;
        let from = -1.0 + RALLY_FROM / 24.0;
        let to = NIGHT_TO / 24.0;
        t >= from && t < to
    }

    fn pick_candidates(&self, day: i32, incumbent: Option<Mayor>) -> Option<[(u32, u8); 2]> {
        let mut adults: Vec<u32> = self
            .people
            .list
            .iter()
            .enumerate()
            .filter(|(_, p)| p.alive() && p.kind == AgeGroup::Adult && p.age >= 26 && p.age <= 72)
            .map(|(i, _)| i as u32)
            .collect();
        if adults.len() < 6 {
            return None;
        }
        let mut rng = Rng::new(self.seed ^ 0xE1EC_7000 ^ (day as u64).wrapping_mul(0x9E37_79B1));
        let first = match incumbent {
            Some(m) if self.people.list.get(m.id as usize).is_some_and(|p| p.alive()) => (m.id, m.slate),
            _ => {
                let i = adults.swap_remove(rng.usize(adults.len()));
                (i, rng.usize(2) as u8)
            }
        };
        adults.retain(|&i| i != first.0 && self.people.list[i as usize].surname() != self.people.list[first.0 as usize].surname());
        if adults.is_empty() {
            return None;
        }
        let second = (adults[rng.usize(adults.len())], 1 - first.1);
        Some([first, second])
    }

    /// Choose a measure for the ballot: something the town does not have and
    /// the voters have not just refused.
    fn pick_measure(&self, day: i32) -> Option<Project> {
        let mut options: Vec<Project> = Vec::new();
        let pop = self.population();
        for p in Project::ALL {
            let ok = match p.landmark() {
                None => self.lots.iter().filter(|l| l.kind == LotKind::Park).count() < 8 && pop >= 400,
                Some(k) => {
                    !self.has_landmark(k)
                        && self.civics.approved & (1 << k.index()) == 0
                        && self.civics.rejected & (1 << k.index()) == 0
                        && self.era() >= Era::Town
                        && pop >= 700
                }
            };
            if ok {
                options.push(p);
            }
        }
        if options.is_empty() {
            return None;
        }
        let mut rng = Rng::new(self.seed ^ 0xBA11_07 ^ (day as u64).wrapping_mul(0x85EB_CA6B));
        Some(options[rng.usize(options.len())])
    }

    fn election_step(&mut self, day: i32) {
        let hour = self.hour_of();
        // Schedule the first election once there is a hall and a town.
        if self.civics.election.is_none() {
            if self.civics.next_election == 0 {
                if (self.has_hall() && self.residents() >= 160) || (day >= 9 && self.residents() >= 240) {
                    self.civics.next_election = day + 6;
                }
                return;
            }
            // Campaign opens four days ahead.
            if day >= self.civics.next_election - CAMPAIGN_DAYS as i32 && day <= self.civics.next_election {
                let inc = self.civics.mayor;
                if let Some(cand) = self.pick_candidates(self.civics.next_election, inc) {
                    let measure = self.pick_measure(self.civics.next_election);
                    self.civics.election = Some(Election {
                        day: self.civics.next_election,
                        cand,
                        measure,
                        told: 0,
                    });
                } else {
                    self.civics.next_election = day + 2;
                }
            }
            return;
        }
        let Some(mut e) = self.civics.election else { return };
        let t = self.time / DAY - e.day as f64;
        // Counted: the posters come down the morning after.
        if e.told & 4 != 0 {
            if t >= 1.3 {
                self.civics.election = None;
            }
            return;
        }
        // 1. The campaign opens: posters go up.
        if e.told & 1 == 0 && t >= -CAMPAIGN_DAYS {
            e.told |= 1;
            let (a, b) = (self.person_name(e.cand[0].0), self.person_name(e.cand[1].0));
            let inc = self.civics.mayor.is_some();
            let line = if inc {
                format!(
                    "campaign posters go up: Mayor {a} ({}) is challenged by {b} ({})",
                    SLATES[e.cand[0].1 as usize], SLATES[e.cand[1].1 as usize]
                )
            } else {
                format!(
                    "the first mayoral campaign begins: {a} ({}) against {b} ({})",
                    SLATES[e.cand[0].1 as usize], SLATES[e.cand[1].1 as usize]
                )
            };
            self.civic_line(line);
            if let Some(m) = e.measure {
                self.civic_line(format!("the ballot also asks the voters: {}?", m.phrase()));
            }
        }
        // 2. The rally.
        if e.told & 2 == 0 && t >= -1.0 && hour >= RALLY_FROM && t < 0.0 {
            e.told |= 2;
            self.civic_line("both campaigns rally in the square tonight");
        }
        // 3. Election night: the result.
        if e.told & 4 == 0 && t >= 0.0 && hour >= RESULT_AT {
            e.told |= 4;
            self.civics.election = Some(e);
            self.count_votes(e);
            // `count_votes` closes the election.
            return;
        }
        self.civics.election = Some(e);
    }

    fn person_name(&self, id: u32) -> String {
        self.people
            .list
            .get(id as usize)
            .map(|p| p.name())
            .unwrap_or_else(|| "a resident".into())
    }

    fn count_votes(&mut self, e: Election) {
        let mut rng = Rng::new(self.seed ^ 0x707E_5000 ^ (e.day as u64).wrapping_mul(0xC2B2_AE35));
        let inc = self.civics.mayor;
        let mood = self.civics.mood;
        // The sitting mayor is helped by good times and hurt by bad ones.
        let p_first = if inc.is_some() { (0.5 + mood * 0.35).clamp(0.15, 0.85) } else { 0.5 };
        let first_wins = rng.f32() < p_first;
        let (w, l) = if first_wins { (e.cand[0], e.cand[1]) } else { (e.cand[1], e.cand[0]) };
        let share = 51 + rng.range(0, 14);
        let (wn, ln) = (self.person_name(w.0), self.person_name(l.0));
        let line = match inc {
            Some(m) if m.id == w.0 => format!("{wn} is re-elected mayor with {share}% of the vote"),
            Some(m) => {
                let old = self.person_name(m.id);
                format!("{wn} ({}) unseats Mayor {old}, winning {share}% of the vote", SLATES[w.1 as usize])
            }
            None => format!("{wn} ({}) is elected the first mayor, with {share}% against {ln}", SLATES[w.1 as usize]),
        };
        self.civic_line(line);
        self.civics.mayor = Some(Mayor { id: w.0, slate: w.1, since_day: e.day });
        self.civics.elections_held = self.civics.elections_held.saturating_add(1);
        // The measure.
        if let Some(m) = e.measure {
            let yes = (50.0 + mood * 9.0 + rng.range_f32(-10.0, 12.0)).clamp(25.0, 80.0) as i32;
            let passes = yes > 50;
            let letter = (b'A' + self.civics.elections_held.saturating_sub(1) % 8) as char;
            let mut line = format!("Measure {letter} ({}): {yes} to {} — ", m.phrase(), 100 - yes);
            if passes {
                match m.landmark() {
                    Some(k) => self.civics.approved |= 1 << k.index(),
                    None => self.civics.parks_owed = self.civics.parks_owed.saturating_add(1),
                }
                line.push_str("passed; work will begin");
            } else {
                if let Some(k) = m.landmark() {
                    self.civics.rejected |= 1 << k.index();
                }
                line.push_str("rejected");
            }
            self.civic_line(line);
        }
        // The rejected list is forgotten after a few elections.
        if self.civics.elections_held % 3 == 0 {
            self.civics.rejected = 0;
        }
        let gap = 10 + (hash2(e.day, 3, self.seed) * 4.0) as i32;
        self.civics.next_election = e.day + gap;
        // The campaign posters stay up until the morning: keep the day for the
        // phase function by leaving a closed record.
        self.civics.election = Some(Election { told: 7, ..e });
        // …and the schedule resumes when the posters have come down.
    }

    /// Called from the extras pass: build what the voters approved.
    pub(crate) fn try_civic_projects(&mut self) -> bool {
        if self.civics.parks_owed == 0 || !self.has_landmark(LandmarkKind::School) {
            return false;
        }
        for d in [
            crate::District::Core,
            crate::District::Downtown,
            crate::District::European,
            crate::District::Suburb,
            crate::District::HighEnd,
        ] {
            if let Some(p) = self.free_spot_in(d, 6) {
                if self.place_planned(
                    crate::blocks::PlanLot { x: p.0, y: p.1, w: 6, h: 6, role: crate::blocks::Role::Green },
                    LotKind::Park,
                ) {
                    self.civics.parks_owed -= 1;
                    self.civic_line("ground is broken on the new city park");
                    return true;
                }
            }
        }
        false
    }

    /// Did the voters approve this landmark? (It is then wanted even before its
    /// ordinary population gate.)
    pub fn landmark_approved(&self, k: LandmarkKind) -> bool {
        self.civics.approved & (1 << k.index()) != 0
    }

    // -----------------------------------------------------------------------
    // Incidents: fires and blackouts
    // -----------------------------------------------------------------------

    fn incident_step(&mut self, day: i32) {
        let now = self.time;
        // Schedule the next ones once the town is worth burning.
        if self.residents() >= 220 {
            if self.civics.next_fire == 0 {
                self.civics.next_fire = day + 5 + (hash2(day, 1, self.seed) * 5.0) as i32;
            }
            if self.civics.next_blackout == 0 {
                self.civics.next_blackout = day + 7 + (hash2(day, 2, self.seed) * 6.0) as i32;
            }
        }
        // A fire.
        if let Some(mut f) = self.civics.fire {
            let dt = now - f.t0;
            let at = self.street_of_lot(f.x, f.y);
            let mut lines: Vec<String> = Vec::new();
            if f.told & 1 == 0 {
                f.told |= 1;
                lines.push(format!("smoke is rising over {at}"));
            }
            if dt >= 1500.0 && f.told & 2 == 0 {
                f.told |= 2;
                lines.push(format!("fire crews fight a blaze on {at}"));
            }
            if dt >= FIRE_OUT && f.told & 4 == 0 {
                f.told |= 4;
                lines.push("the fire is out; repair crews move in behind the engines".into());
            }
            if dt >= FIRE_DONE {
                self.civics.fire = None;
                lines.push("the fire-damaged building has been repaired".into());
            } else {
                self.civics.fire = Some(f);
            }
            for l in lines {
                self.civic_line(l);
            }
        } else if self.civics.next_fire != 0
            && day >= self.civics.next_fire
            && !self.settlers.is_active()
            && self.civics.election.map_or(true, |e| e.day != day)
        {
            let hour = 9.0 + hash2(day, 5, self.seed) as f64 * 11.0;
            let t0 = day as f64 * DAY + hour * HOUR;
            if now >= t0 {
                if let Some((x, y)) = self.fire_target(day) {
                    self.civics.fire = Some(Fire { x, y, t0: now, dispatched: false, told: 0 });
                    self.civics.fires_seen = self.civics.fires_seen.saturating_add(1);
                }
                self.civics.next_fire = day + 8 + (hash2(day, 7, self.seed) * 8.0) as i32;
            }
        }
        // A blackout.
        if let Some(mut b) = self.civics.blackout {
            let mut lines: Vec<&str> = Vec::new();
            if b.told & 1 == 0 {
                b.told |= 1;
                lines.push("demand is straining the grid: lights flicker across town");
            }
            if now >= b.t0 && b.told & 2 == 0 {
                b.told |= 2;
                lines.push("a blackout: a whole district goes dark; repair crews are on their way");
            }
            if now >= b.t0 + b.dur {
                self.civics.blackout = None;
                lines.push("the lights come back on; the crews head home");
            } else {
                self.civics.blackout = Some(b);
            }
            for l in lines {
                self.civic_line(l);
            }
        } else if self.civics.next_blackout != 0
            && day >= self.civics.next_blackout
            && self.civics.fire.is_none()
            && self.civics.election.map_or(true, |e| e.day != day)
        {
            // Warn an hour before: the lights go in the early evening.
            let go = day as f64 * DAY + (17.5 + hash2(day, 9, self.seed) as f64 * 2.0) * HOUR;
            if now >= go - HOUR {
                if let Some((cx, cy)) = self.blackout_centre(day) {
                    let r = 6 + (hash2(day, 4, self.seed) * 4.0) as i32;
                    let dur = (3.0 + hash2(day, 6, self.seed) as f64 * 2.0) * HOUR;
                    self.civics.blackout = Some(Blackout { cx, cy, r, t0: go.max(now + 600.0), dur, told: 0 });
                    self.civics.blackouts_seen = self.civics.blackouts_seen.saturating_add(1);
                }
                self.civics.next_blackout = day + 10 + (hash2(day, 8, self.seed) * 8.0) as i32;
            }
        }
    }

    fn street_of_lot(&self, x: i32, y: i32) -> String {
        match self.lots.iter().find(|l| l.x == x && l.y == y) {
            Some(l) => {
                let w = people::World { map: &self.map, lots: &self.lots, fields: &self.fields, seed: self.seed };
                crate::life::street_name(&w, people::door_tile(l), l.front)
            }
            None => "the town".into(),
        }
    }

    /// A building to burn: ordinary, lived-in, near enough to be seen.
    fn fire_target(&self, day: i32) -> Option<(i32, i32)> {
        let (fx, fy) = self.founding;
        let mut best: Option<(f32, (i32, i32))> = None;
        for l in &self.lots {
            if !l.finished()
                || !matches!(
                    l.kind,
                    LotKind::House | LotKind::Shop | LotKind::Store | LotKind::Townhouse | LotKind::Industry | LotKind::Apartment | LotKind::Barn
                )
            {
                continue;
            }
            let d = ((l.x - fx).abs() + (l.y - fy).abs()) as f32;
            if d > 22.0 || l.storeys > 4 {
                continue;
            }
            // Not hidden behind a taller neighbour on the near side.
            if self.lots.iter().any(|o| {
                !std::ptr::eq(o, l)
                    && o.x + o.w > l.x
                    && o.y + o.h > l.y
                    && o.x - l.x <= 5
                    && o.y - l.y <= 5
                    && (o.x >= l.x || o.y >= l.y)
                    && o.storeys > l.storeys
                    && !matches!(o.kind, LotKind::Park | LotKind::Cemetery)
            }) {
                continue;
            }
            let mut score = hash2(l.x, l.y, self.seed ^ (day as u64).wrapping_mul(0x51ED));
            // A fire is worth having where the wallpaper can show it.
            let (ox, oy, vw, vh) = self.viewport;
            let (sx, sy) = (ox + (l.x - l.y) * 32, oy + (l.x + l.y) * 16);
            if sx > 160 && sx < vw - 160 && sy > 220 && sy < vh - 110 {
                score += 1.0;
            }
            if best.map_or(true, |(b, _)| score > b) {
                best = Some((score, (l.x, l.y)));
            }
        }
        best.map(|(_, p)| p)
    }

    fn blackout_centre(&self, day: i32) -> Option<(i32, i32)> {
        let (fx, fy) = self.founding;
        let mut best: Option<(f32, (i32, i32))> = None;
        for l in &self.lots {
            if !l.finished() || matches!(l.kind, LotKind::Park | LotKind::Cemetery) {
                continue;
            }
            let d = (l.x - fx).abs() + (l.y - fy).abs();
            if d > 18 {
                continue;
            }
            let s = hash2(l.x, l.y, self.seed ^ 0xB1AC_0u64 ^ day as u64);
            if best.map_or(true, |(b, _)| s > b) {
                best = Some((s, (l.x + l.w / 2, l.y + l.h / 2)));
            }
        }
        best.map(|(_, p)| p)
    }

    /// Fire engines due to be sent: the traffic step takes this once.
    pub fn take_fire_dispatch(&mut self) -> Option<(i32, i32)> {
        let now = self.time;
        let f = self.civics.fire.as_mut()?;
        if !f.dispatched && now - f.t0 >= 600.0 {
            f.dispatched = true;
            let (x, y) = (f.x, f.y);
            let l = self.lots.iter().find(|l| l.x == x && l.y == y)?;
            return Some(people::door_tile(l));
        }
        None
    }

    // -----------------------------------------------------------------------
    // What the renderer reads
    // -----------------------------------------------------------------------

    /// The fire's stage at the current instant.
    pub fn fire_stage(&self) -> Option<(Fire, FireStage)> {
        let f = self.civics.fire?;
        let dt = self.time - f.t0;
        let st = if dt < 1500.0 {
            FireStage::Smoke(dt as f32 / 1500.0)
        } else if dt < FIRE_OUT {
            FireStage::Burning(((dt - 1500.0) / (FIRE_OUT - 1500.0)) as f32)
        } else {
            FireStage::Aftermath(((dt - FIRE_OUT) / (FIRE_DONE - FIRE_OUT)) as f32)
        };
        Some((f, st))
    }

    /// The dark zone right now: `(cx, cy, radius_in_tiles)`, shrinking as the
    /// crews restore district by district.
    pub fn blackout_zone(&self) -> Option<(i32, i32, f32)> {
        let b = self.civics.blackout?;
        let u = (self.time - b.t0) / b.dur;
        if u < 0.0 {
            return None;
        }
        let r = if u < 0.55 { 1.0 } else { 1.0 - (u - 0.55) / 0.45 };
        (r > 0.02).then_some((b.cx, b.cy, b.r as f32 * r as f32))
    }

    /// Is the grid flickering (the hour before the lights go)?
    pub fn grid_straining(&self) -> bool {
        self.civics
            .blackout
            .map_or(false, |b| self.time < b.t0 && self.time >= b.t0 - HOUR)
    }

    /// A repair crew's truck stands at the edge of the dark zone while the
    /// power is out: `(x, y)` in tiles.
    pub fn blackout_crew(&self) -> Option<(f32, f32)> {
        let b = self.civics.blackout?;
        let u = (self.time - b.t0) / b.dur;
        if !(0.1..0.97).contains(&u) {
            return None;
        }
        // On the nearest street tile to the centre.
        let c = (b.cx, b.cy);
        self.civic_lane_near(c).map(|(x, y)| (x as f32 + 0.5, y as f32 + 0.5))
    }

    fn civic_lane_near(&self, c: (i32, i32)) -> Option<(i32, i32)> {
        for ring in 0..9i32 {
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let (x, y) = (c.0 + dx, c.1 + dy);
                    if self.map.in_bounds(x, y) && self.map.at(x, y).kind.is_lane() {
                        return Some((x, y));
                    }
                }
            }
        }
        None
    }

    /// A staging tool for the handback's shots (like `stage_ending`): put the
    /// civic state into a named scenario at `day` and `hour`, whatever the
    /// seed would have done. `what` is one of `campaign`, `rally`, `vote`,
    /// `night`, `fire`, `fire-smoke`, `fire-after`, `blackout`, `bust`, `boom`,
    /// `smog`, `clean`. Not used by the running game.
    pub fn stage_civic(&mut self, what: &str, day: i64, hour: f64) {
        self.time = day as f64 * DAY + hour.clamp(0.0, 23.99) * HOUR;
        self.civics.day = day as i32;
        let d = day as i32;
        let cand = self.pick_candidates(d + 1, self.civics.mayor);
        let election = |this: &Sim, day: i32| {
            cand.map(|c| Election { day, cand: c, measure: this.pick_measure(day), told: 3 })
        };
        match what {
            "campaign" => self.civics.election = election(self, d + 2),
            "rally" => self.civics.election = election(self, d + 1),
            "vote" | "night" => self.civics.election = election(self, d),
            "fire" | "fire-smoke" | "fire-after" => {
                let ago = match what {
                    "fire-smoke" => 600.0,
                    "fire-after" => 5.0 * HOUR,
                    _ => 5400.0,
                };
                if let Some((x, y)) = self.fire_target(d) {
                    self.civics.fire = Some(Fire { x, y, t0: self.time - ago, dispatched: true, told: 7 });
                }
            }
            "blackout" => {
                if let Some((cx, cy)) = self.blackout_centre(d) {
                    let dur = 4.0 * HOUR;
                    self.civics.blackout = Some(Blackout { cx, cy, r: 10, t0: self.time - 0.3 * dur, dur, told: 7 });
                }
            }
            "bust" => {
                self.civics.mood = -0.85;
                self.civics.vacancy = 0.5;
                let n = self.civics.companies.len();
                for (i, c) in self.civics.companies.iter_mut().enumerate() {
                    c.closed = i % 2 == 0 || n < 3;
                    c.floors = 0;
                }
            }
            "boom" => {
                self.civics.mood = 0.85;
                self.civics.vacancy = 0.0;
                for c in self.civics.companies.iter_mut() {
                    c.closed = false;
                    c.floors = 3;
                }
            }
            "smog" => {
                self.civics.smog = 0.9;
                self.civics.cleanups = 0;
            }
            "clean" => {
                self.civics.smog = 0.05;
                self.civics.cleanups = CLEAN_SCRUBBERS | CLEAN_TREES;
            }
            _ => {}
        }
    }

    /// The code a lot is drawn with for civic reasons: 0 intact, otherwise a
    /// ruin code of `buildkit::ruin` (12 vacant, 13 scorched, 8 acid-stained,
    /// 11 lights out). The ending's own codes take precedence in the caller.
    pub fn civic_ruin_of(&self, i: usize) -> u8 {
        let Some(l) = self.lots.get(i) else { return 0 };
        if !l.finished() {
            return 0;
        }
        if let Some((f, st)) = self.fire_stage() {
            if (l.x, l.y) == (f.x, f.y) {
                return match st {
                    FireStage::Aftermath(_) => CODE_SCORCHED,
                    FireStage::Burning(u) if u > 0.5 => CODE_SCORCHED,
                    _ => 0,
                };
            }
        }
        if let Some((cx, cy, r)) = self.blackout_zone() {
            let (mx, my) = (l.x as f32 + l.w as f32 * 0.5 - cx as f32, l.y as f32 + l.h as f32 * 0.5 - cy as f32);
            if (mx * mx + my * my).sqrt() <= r {
                return 11;
            }
        }
        if self.lot_vacant(l) {
            return CODE_VACANT;
        }
        // Acid rain marks on the old stone when the air is bad.
        if self.civics.smog > 0.42
            && matches!(l.kind, LotKind::Church | LotKind::Hall | LotKind::Landmark(_))
            && !matches!(l.kind, LotKind::Landmark(LandmarkKind::Plaza | LandmarkKind::Zoo | LandmarkKind::Castle))
            && hash2(l.x, l.y, self.seed ^ 0xAC1D) < (self.civics.smog - 0.3) * 1.6
        {
            return 8;
        }
        0
    }

    /// A number that changes whenever the baked picture of the city has to be
    /// redrawn for a civic reason (vacancy shifting, a fire changing stage, the
    /// blackout's edge moving, posters going up).
    pub fn civic_epoch(&self) -> u64 {
        let c = &self.civics;
        let mut h = (c.vacancy * 24.0) as u64;
        h = h.wrapping_mul(31).wrapping_add((c.smog * 12.0) as u64);
        h = h.wrapping_mul(31).wrapping_add(c.companies.iter().filter(|x| x.closed).count() as u64);
        if let Some((_, st)) = self.fire_stage() {
            h = h.wrapping_mul(31).wrapping_add(match st {
                FireStage::Smoke(_) => 1,
                FireStage::Burning(u) => 2 + (u > 0.5) as u64,
                FireStage::Aftermath(_) => 4,
            });
        }
        if let Some((_, _, r)) = self.blackout_zone() {
            h = h.wrapping_mul(31).wrapping_add(1 + (r * 2.0) as u64);
        }
        h
    }
}

/// Ruin codes `buildkit::ruin` draws for the civic states.
pub const CODE_VACANT: u8 = 12;
pub const CODE_SCORCHED: u8 = 13;

/// When the fire is out (seconds after the first smoke) and when the repairs end.
pub const FIRE_OUT: f64 = 3.0 * HOUR;
pub const FIRE_DONE: f64 = 9.0 * HOUR;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FireStage {
    /// First smoke, 0..1 through the warning.
    Smoke(f32),
    /// Flames, 0..1.
    Burning(f32),
    /// Repair crews, 0..1.
    Aftermath(f32),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pace;

    fn world(seed: u64) -> Sim {
        let vp = map::world_for_viewport(1920, 1080);
        let mut s = Sim::with_focus_in(
            seed,
            Pace::Month,
            vp.w,
            vp.h,
            (vp.w as f32 * 0.5, vp.h as f32 * 0.5),
            Some(map::Biome::Temperate),
        );
        s.found_again();
        s
    }

    #[test]
    fn names_are_original_and_distinct_by_kind() {
        let c = Company { a: 0, b: 0, kind: 2, hq: (0, 0), health: 0.5, floors: 0, closed: false, closed_day: 0 };
        assert!(c.name().ends_with("Savings Bank"));
        assert!(c.is_bank());
    }

    #[test]
    fn a_slump_brings_a_decline_ending_nearer() {
        let mut s = world(1);
        s.run_to(12.0 * 86400.0);
        s.ending.kind = crate::ending::Ending::Decline;
        s.ending.world_days = 40.0;
        s.civics.mood = -0.9;
        for _ in 0..10 {
            s.ending_pressure();
        }
        assert!(s.ending.world_days < 38.0, "{}", s.ending.world_days);
        s.civics.mood = 0.6;
        let w = s.ending.world_days;
        s.ending_pressure();
        assert!(s.ending.world_days > w);
    }

    #[test]
    fn projects_round_trip_ids() {
        for p in Project::ALL {
            assert_eq!(Project::from_id(p.id()), p);
        }
    }

    /// The handback's timeline: what a world's civic life does over 40 days.
    #[test]
    fn a_world_has_elections_a_cycle_companies_and_incidents() {
        let mut elections = 0;
        let mut fires = 0;
        let mut blackouts = 0;
        let mut mayors = 0;
        let mut bust_seen = 0;
        for seed in [1u64, 2, 3, 4] {
            let mut s = world(seed);
            let mut moods = String::new();
            let mut lo = 1.0f32;
            let mut hi = -1.0f32;
            let mut max_vac = 0.0f32;
            for day in 1..=40 {
                s.run_to(day as f64 * 86400.0 + 12.0 * 3600.0);
                lo = lo.min(s.civics.mood);
                hi = hi.max(s.civics.mood);
                max_vac = max_vac.max(s.civics.vacancy);
                if day % 4 == 0 {
                    moods.push_str(&format!("{:+.2} ", s.civics.mood));
                }
            }
            eprintln!(
                "seed {seed}: pop {} mood [{lo:+.2},{hi:+.2}] vac<={max_vac:.2} smog {:.2} cos {} mayor {:?} elections {} fires {} blackouts {} approved {:#x}\n  moods {moods}",
                s.population(),
                s.civics.smog,
                s.civics.companies.len(),
                s.mayor_name(),
                s.civics.elections_held,
                s.civics.fires_seen,
                s.civics.blackouts_seen,
                s.civics.approved,
            );
            for l in s.story.iter().filter(|l| {
                let t = l.text.as_str();
                t.contains("mayor") || t.contains("Measure") || t.contains("fire") || t.contains("blackout") || t.contains("booming") || t.contains("hard times") || t.contains("opens") || t.contains("closes") || t.contains("council") || t.contains("smoke")
            }) {
                eprintln!("    day {:.1}: {}", l.time / 86400.0, l.text);
            }
            elections += s.civics.elections_held as i32;
            fires += s.civics.fires_seen as i32;
            blackouts += s.civics.blackouts_seen as i32;
            mayors += s.civics.mayor.is_some() as i32;
            bust_seen += (lo < SLUMP_BELOW) as i32;
        }
        assert!(elections >= 4, "elections {elections}");
        assert!(mayors >= 3, "mayors {mayors}");
        assert!(fires >= 2, "fires {fires}");
        assert!(blackouts >= 2, "blackouts {blackouts}");
        assert!(bust_seen >= 1, "no seed ever had a slump");
    }

    /// What the voters approve gets built, and the civic centre gathers round
    /// the square.
    #[test]
    fn approved_projects_are_built_and_the_centre_gathers() {
        let mut built = 0;
        let mut centre = 0;
        for seed in [1u64, 2, 3] {
            let mut s = world(seed);
            s.run_to(42.0 * 86400.0);
            for k in LandmarkKind::ALL {
                if s.landmark_approved(k) && s.has_landmark(k) {
                    built += 1;
                }
            }
            // Courthouse, police and post office near the square.
            let plaza = s.lots.iter().find(|l| l.kind == LotKind::Landmark(LandmarkKind::Plaza));
            for l in &s.lots {
                if matches!(
                    l.kind,
                    LotKind::Landmark(LandmarkKind::Courthouse | LandmarkKind::PoliceStation | LandmarkKind::PostOffice)
                ) {
                    centre += 1;
                    if let Some(p) = plaza {
                        let d = (l.x - p.x).abs() + (l.y - p.y).abs();
                        eprintln!("seed {seed}: {:?} {d} tiles from the square", l.kind);
                    }
                }
            }
        }
        assert!(built >= 1, "no approved landmark was built");
        assert!(centre >= 3, "only {centre} civic-centre buildings in 3 worlds");
    }

    /// The civics block survives a save, and the hash agrees after a reload.
    #[test]
    fn the_civics_block_round_trips() {
        let mut s = world(2);
        s.run_to(24.0 * 86400.0 + 20.0 * 3600.0);
        assert!(s.civics.mayor.is_some() && !s.civics.companies.is_empty());
        let dir = std::env::temp_dir().join("citylive-b52-save");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("civics.sav");
        s.save(&p).unwrap();
        let l = Sim::load(&p).unwrap();
        assert_eq!(s.civics, l.civics, "the civics block reads back as written");
        assert_eq!(s.hash(), l.hash());
        assert_eq!(s.mayor_name(), l.mayor_name());
        // The tray's headline names the mayor and the mood.
        let head = s.tray_status("", false).civic;
        assert!(head.starts_with("Mayor ") && head.contains('·'), "{head}");
        assert_eq!(head, l.tray_status("", false).civic);
    }
}

/// B51: which chronicle topic a civic feed line belongs to.
fn civic_topic(text: &str) -> Topic {
    if text.contains("is booming") {
        Topic::Boom
    } else if text.starts_with("hard times") {
        Topic::Bust
    } else if text.starts_with("the slump is over") {
        Topic::Recovery
    } else if text.contains("is elected") || text.contains("re-elected") || text.contains("unseats") {
        Topic::Election
    } else if text.starts_with("Measure ") {
        Topic::Measure
    } else if text.contains("fight a blaze") {
        Topic::Fire
    } else if text.starts_with("a blackout") {
        Topic::Blackout
    } else if text.contains("opens") || text.contains("closes") || text.contains("failed") {
        Topic::Company
    } else {
        Topic::Civic
    }
}
