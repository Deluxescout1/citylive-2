//! The end of the world, and the next one (B50, D65, D66, D75).
//!
//! Every world has a length (about a month of sim time, randomised +-30 %,
//! fixed by the seed) and an [`Ending`] picked at founding. Near the end the
//! world goes through four phases, then a morph, then it is replaced:
//!
//! * **warning** (hours to days before): omens, crowds looking up, people
//!   leaving, the feed talking;
//! * **spectacle** (minutes to an hour): the event itself;
//! * **aftermath** (hours): the land changed, ruins, survivors leaving;
//! * **morph** (a few hours): the land turns, tile by tile, into the *next*
//!   biome, ruins weathering away as the front passes;
//! * **done**: [`Sim::rebirth`] hands the next world's seed and land to the
//!   caller, which founds a fresh world (new settlers, new name, new story).
//!
//! The whole thing is a pure function of `(seed, sim time)` plus a few cursors
//! that make tile edits incremental, so a save in any phase resumes exactly,
//! and a catch-up replays the same ending that a live run would have shown.
//!
//! What this module owns:
//!
//! * the plan ([`State`]) and the timeline ([`Sim::locate`]);
//! * the **land edits** (craters, cones, lava, flood water, scorch, overgrowth
//!   and the morph itself) as ordered tile lists with a cursor;
//! * the **ruin code** of every lot ([`Sim::ruin_codes`]): what the renderer
//!   draws instead of the intact building;
//! * the feed lines and the final line for the chronicle (B51);
//! * the people's part: who looks up, who runs, who leaves.
//!
//! Drawing lives in `render::ending`; the sim only hands over a [`View`].

use std::collections::BTreeMap;

use map::{Biome, Kind, Prop, Tile, TileMap};

use crate::{Lot, LotKind, Sim};

// ---------------------------------------------------------------------------
// The kinds
// ---------------------------------------------------------------------------

/// How a world ends (D75).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Ending {
    /// A rock from the sky; a crater that fills with water.
    Meteor,
    /// A mountain wakes in the countryside; lava fields.
    Volcano,
    /// The sea leaves, then comes back for good: a new lagoon.
    Flood,
    /// A towering armoured strider walks through the town; sometimes two.
    Kaiju,
    /// A fleet of saucers over the city, beams, then silence.
    Aliens,
    /// A conversion front, block by block, ending in a silent machine city.
    AiTakeover,
    /// The sun swells and reddens over days; everything bleaches and burns.
    RedGiant,
    /// Snow spreads, water freezes, people leave.
    IceAge,
    /// The moon grows night by night, then a vast impact.
    FallingMoon,
    /// Shops close, buildings empty, vines and trees take over.
    Decline,
    /// Smog thickens until nobody can breathe it.
    Smog,
    /// The rare gentle one: a long golden sunset and a new dawn.
    Sunset,
}

impl Ending {
    pub const ALL: [Ending; 12] = [
        Ending::Meteor,
        Ending::Volcano,
        Ending::Flood,
        Ending::Kaiju,
        Ending::Aliens,
        Ending::AiTakeover,
        Ending::RedGiant,
        Ending::IceAge,
        Ending::FallingMoon,
        Ending::Decline,
        Ending::Smog,
        Ending::Sunset,
    ];

    pub fn id(self) -> u8 {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0) as u8
    }

    pub fn from_id(id: u8) -> Ending {
        Self::ALL[(id as usize).min(Self::ALL.len() - 1)]
    }

    pub fn name(self) -> &'static str {
        match self {
            Ending::Meteor => "meteor",
            Ending::Volcano => "volcano",
            Ending::Flood => "flood",
            Ending::Kaiju => "kaiju",
            Ending::Aliens => "aliens",
            Ending::AiTakeover => "ai",
            Ending::RedGiant => "redgiant",
            Ending::IceAge => "iceage",
            Ending::FallingMoon => "moon",
            Ending::Decline => "decline",
            Ending::Smog => "smog",
            Ending::Sunset => "sunset",
        }
    }

    pub fn from_name(s: &str) -> Option<Ending> {
        let s = s.to_ascii_lowercase();
        Some(match s.as_str() {
            "meteor" | "comet" | "impact" => Ending::Meteor,
            "volcano" | "eruption" => Ending::Volcano,
            "flood" | "tsunami" | "wave" => Ending::Flood,
            "kaiju" | "monster" | "strider" => Ending::Kaiju,
            "aliens" | "alien" | "saucers" | "fleet" => Ending::Aliens,
            "ai" | "machine" | "takeover" | "bots" => Ending::AiTakeover,
            "redgiant" | "red-giant" | "sun" => Ending::RedGiant,
            "iceage" | "ice-age" | "ice" | "freeze" => Ending::IceAge,
            "moon" | "fallingmoon" | "falling-moon" => Ending::FallingMoon,
            "decline" | "ruin" | "ruins" | "overgrown" => Ending::Decline,
            "smog" | "pollution" => Ending::Smog,
            "sunset" | "fade" | "dawn" => Ending::Sunset,
            _ => return None,
        })
    }

    /// Phase lengths in sim seconds: `(warning, spectacle, aftermath, morph)`.
    pub fn timing(self) -> Timing {
        const H: f64 = 3600.0;
        let (w, s, a, m) = match self {
            Ending::Meteor => (20.0 * H, 0.42 * H, 8.0 * H, 4.0 * H),
            Ending::Volcano => (24.0 * H, 0.8 * H, 10.0 * H, 4.0 * H),
            Ending::Flood => (14.0 * H, 0.7 * H, 8.0 * H, 4.0 * H),
            Ending::Kaiju => (6.0 * H, 0.9 * H, 8.0 * H, 4.0 * H),
            Ending::Aliens => (10.0 * H, 0.75 * H, 6.0 * H, 4.0 * H),
            Ending::AiTakeover => (16.0 * H, 0.9 * H, 6.0 * H, 4.0 * H),
            Ending::RedGiant => (72.0 * H, 0.7 * H, 10.0 * H, 5.0 * H),
            Ending::IceAge => (72.0 * H, 0.8 * H, 10.0 * H, 5.0 * H),
            Ending::FallingMoon => (96.0 * H, 0.65 * H, 10.0 * H, 5.0 * H),
            Ending::Decline => (36.0 * H, 1.5 * H, 12.0 * H, 4.0 * H),
            Ending::Smog => (48.0 * H, 1.0 * H, 10.0 * H, 4.0 * H),
            Ending::Sunset => (12.0 * H, 1.0 * H, 3.0 * H, 4.0 * H),
        };
        Timing { warn: w, spec: s, after: a, morph: m }
    }

    /// Pick the ending of a world (D65: random per world, D75: all possible).
    /// Weighted, deterministic from `seed`, and honest about the land: a
    /// volcano needs high ground or a tropic, a tsunami needs a coast, an ice
    /// age does not come to a tundra and the sun does not scorch a desert
    /// twice as hard as it needs to.
    pub fn pick(seed: u64, map: &TileMap) -> Ending {
        let high = map.count_at_least(6) > (map.w * map.h) as usize / 40;
        let coast = map.ocean.is_some();
        let mut weights = [0f32; 12];
        for (i, k) in Self::ALL.iter().enumerate() {
            weights[i] = match k {
                Ending::Meteor => 1.0,
                Ending::Volcano => {
                    if map.biome == Biome::Tropical {
                        1.2
                    } else if high {
                        0.9
                    } else {
                        0.0
                    }
                }
                Ending::Flood => {
                    if coast {
                        1.3
                    } else {
                        0.0
                    }
                }
                Ending::Kaiju => 1.2,
                Ending::Aliens => 1.0,
                Ending::AiTakeover => 0.9,
                Ending::RedGiant => 0.6,
                Ending::IceAge => {
                    if map.biome == Biome::Tundra {
                        0.0
                    } else {
                        0.6
                    }
                }
                Ending::FallingMoon => 0.6,
                Ending::Decline => 1.0,
                Ending::Smog => 0.7,
                Ending::Sunset => 0.35,
            };
        }
        let total: f32 = weights.iter().sum();
        let mut r = Rng64::new(seed ^ 0xE4D1_9617_0000_0001).f01() * total;
        for (i, w) in weights.iter().enumerate() {
            if r < *w {
                return Self::ALL[i];
            }
            r -= *w;
        }
        Ending::Decline
    }
}

/// Phase lengths, sim seconds.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub warn: f64,
    pub spec: f64,
    pub after: f64,
    pub morph: f64,
}

/// Where in the ending the world is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Phase {
    Living,
    Warning,
    Spectacle,
    Aftermath,
    Morph,
    Done,
}

impl Phase {
    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            Phase::Living => "living",
            Phase::Warning => "warning",
            Phase::Spectacle => "spectacle",
            Phase::Aftermath => "aftermath",
            Phase::Morph => "morph",
            Phase::Done => "done",
        }
    }

    pub fn from_name(s: &str) -> Option<Phase> {
        Some(match s {
            "living" | "none" => Phase::Living,
            "warning" | "warn" => Phase::Warning,
            "spectacle" | "spec" | "event" => Phase::Spectacle,
            "aftermath" | "after" => Phase::Aftermath,
            "morph" => Phase::Morph,
            "done" | "reborn" => Phase::Done,
            _ => return None,
        })
    }
}

// --- ruin codes: what a lot looks like (shared with buildkit) --------------

pub const R_OK: u8 = 0;
/// Hit and burning: bitten roofline, scorch, ember glow.
pub const R_FIRE: u8 = 1;
/// Brick, glass and beams in a heap.
pub const R_RUBBLE: u8 = 2;
/// A ruin the vines and young trees have taken.
pub const R_OVERGROWN: u8 = 3;
/// Converted: uniform machine panels, cyan lights.
pub const R_MACHINE: u8 = 4;
/// Bleached by a swollen sun.
pub const R_BLEACH: u8 = 5;
/// Shut up: boarded, dark, a little weathered.
pub const R_ABANDONED: u8 = 6;
/// Frosted: white roofs, blue shade, icicles.
pub const R_FROST: u8 = 7;
/// Soot-stained by smoke or ash.
pub const R_SOOT: u8 = 8;
/// Gone entirely (crater, water, lava, the morph front).
pub const R_GONE: u8 = 9;
/// A converted building after power-down: machine panels, no lights.
pub const R_MACHINE_DARK: u8 = 10;
/// Intact but every light is out.
pub const R_DARK: u8 = 11;

// ---------------------------------------------------------------------------
// The state
// ---------------------------------------------------------------------------

/// Everything the ending remembers. Saved (v12) except the caches.
pub struct State {
    pub kind: Ending,
    /// The world's length in sim days: the spectacle starts here.
    pub world_days: f64,
    pub next_seed: u64,
    pub next_biome: Biome,
    /// The place the ending is about (impact, vent, middle of town...), chosen
    /// when the warning begins.
    pub site: (f32, f32),
    pub site_set: bool,
    /// Kind-specific geometry: kaiju paths, the AI's origin, the crater floor...
    pub aux: [f32; 8],
    /// Feed lines already told (one bit each).
    pub told: u32,
    /// How far each land edit has got (an index into its ordered tile list).
    pub cur: [u32; 4],
    pub rebirth_taken: bool,
    /// The last line of the chronicle (B51).
    pub final_line: String,
    /// Props the ending put on tiles (trees, vines, the next land's plants):
    /// a save keeps them because a tile override does not.
    pub props: BTreeMap<(i32, i32), Prop>,

    // --- caches, rebuilt from the above ---
    pub(crate) lists: [Option<Vec<(i32, i32, f32)>>; 4],
    pub(crate) next_map: Option<Box<TileMap>>,
    pub(crate) ruin: Vec<u8>,
    pub(crate) ruin_epoch: u64,
    pub(crate) ruin_time: f64,
    pub(crate) lots_n: usize,
    pub(crate) overlay_epoch: u64,
    pub(crate) lookup: Option<std::collections::HashMap<(i32, i32), f32>>,
    pub(crate) pending_flips: Vec<(i32, i32)>,
    pub(crate) pending_classes: Vec<(i32, i32, u8)>,
    pub(crate) neighbour_told: bool,
}

/// What the viewer has to copy from the sim's map into its own copy.
#[derive(Clone, Debug, Default)]
pub struct Sync {
    pub neighbour: Option<Biome>,
    pub flipped: Vec<(i32, i32)>,
    pub classes: Vec<(i32, i32, u8)>,
}

impl State {
    pub fn new(kind: Ending, world_days: f64, next_seed: u64, next_biome: Biome) -> Self {
        State {
            kind,
            world_days,
            next_seed,
            next_biome,
            site: (0.0, 0.0),
            site_set: false,
            aux: [0.0; 8],
            told: 0,
            cur: [0; 4],
            rebirth_taken: false,
            final_line: String::new(),
            props: BTreeMap::new(),
            lists: [None, None, None, None],
            next_map: None,
            ruin: Vec::new(),
            ruin_epoch: 0,
            ruin_time: -1.0,
            lots_n: 0,
            overlay_epoch: 0,
            lookup: None,
            pending_flips: Vec::new(),
            pending_classes: Vec::new(),
            neighbour_told: false,
        }
    }

    /// The plan a fresh world gets at founding (inside this crate's own tests
    /// the length is stretched a thousandfold; the ending's tests set it): a deterministic function of
    /// the seed and the land.
    pub fn plan(seed: u64, map: &TileMap) -> State {
        let kind = Ending::pick(seed, map);
        let mut r = Rng64::new(seed ^ 0x57A7_E000_0000_0002);
        // About a month, +-30 %.
        let world_days = 30.0 * (0.7 + 0.6 * r.f01() as f64);
        // The older tests run cities for sixty days to look at their growth:
        // inside this crate's tests the world is long unless a test asks.
        #[cfg(test)]
        let world_days = world_days * 1000.0;
        let next_seed = Rng64::new(seed ^ 0x6E65_7874_0000_0003).u64() | 1;
        let next_biome = next_land(map.biome, next_seed);
        State::new(kind, world_days, next_seed, next_biome)
    }
}

/// The land the next world will have: a live land other than this one (D66).
pub fn next_land(current: Biome, next_seed: u64) -> Biome {
    let mut b = Biome::pick(next_seed);
    if b == current {
        let others: Vec<Biome> = Biome::LIVE.iter().copied().filter(|x| *x != current).collect();
        if !others.is_empty() {
            let i = (Rng64::new(next_seed ^ 0x6C61_6E64).f01() * others.len() as f32) as usize;
            b = others[i.min(others.len() - 1)];
        }
    }
    b
}

/// What the renderer needs to draw the ending right now. Plain data, so the
/// renderer does not depend on the sim.
#[derive(Clone, Debug)]
pub struct View {
    pub kind: Ending,
    pub phase: Phase,
    /// Progress through the phase, 0..1.
    pub p: f32,
    /// Progress through the whole ending, 0 at the start of the warning to 4
    /// at the end of the morph (`phase index + p`).
    pub u: f32,
    /// Sim seconds into the phase (smooth), and the phase's length.
    pub secs: f32,
    pub dur: f32,
    pub site: (f32, f32),
    pub aux: [f32; 8],
    pub seed: u64,
    /// Fraction of the main land edit applied, 0..1 (flood front, lava front,
    /// conversion front...).
    pub frac: f32,
}

/// What a finished ending hands to whoever founds the next world.
#[derive(Clone, Debug)]
pub struct Rebirth {
    pub seed: u64,
    pub biome: Biome,
    pub ended: String,
}

// ---------------------------------------------------------------------------
// small deterministic helpers
// ---------------------------------------------------------------------------

pub(crate) struct Rng64(u64);

impl Rng64 {
    pub fn new(seed: u64) -> Self {
        Rng64(seed)
    }
    pub fn u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn f01(&mut self) -> f32 {
        (self.u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// A stable 0..1 hash of a position and a salt.
pub(crate) fn h01(seed: u64, a: i32, b: i32) -> f32 {
    let mut r = Rng64::new(
        seed ^ (a as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (b as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F),
    );
    r.f01()
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn ramp(x: f32, a: f32, b: f32) -> f32 {
    if b <= a {
        return if x >= b { 1.0 } else { 0.0 };
    }
    ((x - a) / (b - a)).clamp(0.0, 1.0)
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Distance from point `p` to segment `a`-`b`, and the position along it (0..1).
pub(crate) fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-6 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
    };
    (dist(p, (a.0 + dx * t, a.1 + dy * t)), t)
}

fn lot_centre(l: &Lot) -> (f32, f32) {
    (l.x as f32 + l.w as f32 * 0.5, l.y as f32 + l.h as f32 * 0.5)
}

fn lot_counts(l: &Lot) -> bool {
    !matches!(l.kind, LotKind::Park | LotKind::Cemetery) && l.finished()
}

// ---------------------------------------------------------------------------
// The timeline
// ---------------------------------------------------------------------------

impl Sim {
    /// Where the world is in its ending at sim time `t`, and how far through
    /// that phase (0..1).
    pub fn locate(&self, t: f64) -> (Phase, f32) {
        let tm = self.ending.kind.timing();
        let spec0 = self.ending.world_days * 86400.0;
        let warn0 = spec0 - tm.warn;
        let after0 = spec0 + tm.spec;
        let morph0 = after0 + tm.after;
        let done0 = morph0 + tm.morph;
        if t < warn0 {
            (Phase::Living, 0.0)
        } else if t < spec0 {
            (Phase::Warning, ((t - warn0) / tm.warn) as f32)
        } else if t < after0 {
            (Phase::Spectacle, ((t - spec0) / tm.spec) as f32)
        } else if t < morph0 {
            (Phase::Aftermath, ((t - after0) / tm.after) as f32)
        } else if t < done0 {
            (Phase::Morph, ((t - morph0) / tm.morph) as f32)
        } else {
            (Phase::Done, 1.0)
        }
    }

    /// The phase now.
    pub fn ending_phase(&self) -> (Phase, f32) {
        self.locate(self.time)
    }

    /// Has the ending's aftermath finished? (B65)
    ///
    /// From here on the land is already turning into the next biome, so this is
    /// the last moment that still belongs to *this* world: the timelapse takes
    /// its final frame here and stops. The exact boundary counts as over — a
    /// float that lands a hair short of the morph start still reports the
    /// aftermath at 1.0, and the aftermath at 1.0 is done.
    pub fn aftermath_over(&self) -> bool {
        let (phase, p) = self.ending_phase();
        phase >= Phase::Morph || (phase == Phase::Aftermath && p >= 1.0)
    }

    /// Progress through the whole ending as `phase index - 1 + p`, so the
    /// warning is 0..1, the spectacle 1..2, the aftermath 2..3, the morph 3..4.
    pub fn ending_u(&self, t: f64) -> f32 {
        let (ph, p) = self.locate(t);
        match ph {
            Phase::Living => 0.0,
            Phase::Warning => p,
            Phase::Spectacle => 1.0 + p,
            Phase::Aftermath => 2.0 + p,
            Phase::Morph => 3.0 + p,
            Phase::Done => 4.0,
        }
    }

    /// Catch up to `t` after time away (the desktop was off). The warning may
    /// pass in the dark, but the spectacle and the morph's first hour must
    /// *play*: the world is held just before the spectacle (and before the
    /// morph) until it is running live; a world found inside a spectacle or the
    /// morph's first hour does not move until then.
    pub fn catch_up_to(&mut self, t: f64) {
        let tm = self.ending.kind.timing();
        let spec0 = self.ending.world_days * 86400.0;
        let morph0 = spec0 + tm.spec + tm.after;
        let now = self.time;
        let mut limit = t;
        if now < spec0 - 30.0 {
            limit = limit.min(spec0 - 30.0);
        } else if now < spec0 + tm.spec {
            limit = now;
        } else if now < morph0 - 30.0 {
            limit = limit.min(morph0 - 30.0);
        } else if now < morph0 + 3600.0 {
            limit = now;
        }
        if limit > now {
            self.run_to(limit);
        }
    }

    /// Have people started to leave (so nobody new moves in)?
    pub fn exodus_begun(&self) -> bool {
        let (ph, p) = self.locate(self.time);
        ph > Phase::Warning || (ph == Phase::Warning && p > 0.3)
    }

    /// Has the world's last phase begun (no growth, no new life)?
    pub fn ending_underway(&self) -> bool {
        self.locate(self.time).0 >= Phase::Spectacle
    }

    /// Put the world at `phase` / `p` of its ending (the spike's
    /// `--ending <kind> --ending-at <phase>` and the tests). The clock does not
    /// move: the world's length does, so the phase lands on the present, and
    /// one ending step brings the land and the lots up to date.
    pub fn stage_ending(&mut self, kind: Ending, phase: Phase, p: f32) {
        let st = &mut self.ending;
        let was_site = st.site_set;
        st.kind = kind;
        let tm = kind.timing();
        let p = p.clamp(0.0, 0.9999) as f64;
        let spec0 = match phase {
            Phase::Living => self.time + tm.warn + 3600.0,
            Phase::Warning => self.time + (1.0 - p) * tm.warn,
            Phase::Spectacle => self.time - p * tm.spec,
            Phase::Aftermath => self.time - tm.spec - p * tm.after,
            Phase::Morph => self.time - tm.spec - tm.after - p * tm.morph,
            Phase::Done => self.time - tm.spec - tm.after - tm.morph - 1.0,
        };
        st.world_days = spec0 / 86400.0;
        // A new kind needs a fresh site and fresh edits.
        if !was_site || st.kind != kind {
            st.site_set = false;
        }
        st.cur = [0; 4];
        st.told = 0;
        st.lists = [None, None, None, None];
        st.ruin_time = -1.0;
        // Replay every phase up to now in order, so the land has its history.
        let target = self.time;
        self.ending_catch_up(target);
        // The furniture on ground the water has taken goes with it: a street's
        // painted lines and lamps must not be left floating on the lagoon (a
        // staged ending never waits for the every-third-step sweep, B99c).
        self.clear_dead_furniture();
    }

    /// Visit the phase boundaries up to `t` in order, so that tile edits and
    /// lines happen the way they would have live.
    fn ending_catch_up(&mut self, t: f64) {
        let tm = self.ending.kind.timing();
        let spec0 = self.ending.world_days * 86400.0;
        let marks = [
            spec0 - tm.warn,
            spec0 - tm.warn * 0.5,
            spec0,
            spec0 + tm.spec * 0.5,
            spec0 + tm.spec,
            spec0 + tm.spec + tm.after * 0.5,
            spec0 + tm.spec + tm.after,
            spec0 + tm.spec + tm.after + tm.morph * 0.5,
        ];
        let real_time = self.time;
        for m in marks {
            if m < t && m > real_time - 40.0 * 86400.0 {
                self.time = m + 1.0;
                self.ending.ruin_time = -1.0;
                self.ending_step();
            }
        }
        self.time = real_time;
        self.ending.ruin_time = -1.0;
        self.ending_step();
    }

    // -----------------------------------------------------------------------
    // The step
    // -----------------------------------------------------------------------

    /// One planning step of the ending. Cheap when nothing is happening: a
    /// timeline lookup and out.
    pub(crate) fn ending_step(&mut self) {
        let (phase, p) = self.locate(self.time);
        if phase == Phase::Living {
            return;
        }
        if !self.ending.site_set {
            self.choose_site();
        }
        let u = self.ending_u(self.time);
        self.tell_lines(phase, p);
        self.land_edits(phase, p, u);
        if phase >= Phase::Spectacle && phase < Phase::Done {
            self.thin_the_streets(phase);
        }
        // The people: who has left by now (every few steps).
        if self.step_index % 3 == 0 || phase == Phase::Done || self.ending.ruin_time < 0.0 {
            self.people_leave(phase, p);
        }
        // The ruins: recomputed every few steps while the ending moves.
        if self.step_index % 3 == 0 || self.ending.ruin_time < 0.0 || phase == Phase::Done {
            self.clear_dead_furniture();
            self.refresh_ruins(u, phase);
        }
        if phase == Phase::Done && self.ending.final_line.is_empty() {
            self.ending.final_line = final_line_for(self.ending.kind, &self.city_name);
        }
    }

    // --- the site ----------------------------------------------------------

    /// The middle of the built city and a radius that holds most of it.
    pub fn city_centre(&self) -> ((f32, f32), f32) {
        let pts: Vec<(f32, f32)> = self.lots.iter().filter(|l| lot_counts(l)).map(lot_centre).collect();
        if pts.len() < 3 {
            return ((self.founding.0 as f32 + 0.5, self.founding.1 as f32 + 0.5), 6.0);
        }
        let n = pts.len() as f32;
        let mean = (pts.iter().map(|p| p.0).sum::<f32>() / n, pts.iter().map(|p| p.1).sum::<f32>() / n);
        // The heart of the town is where the picture is centred (D35: the
        // founding sits at the middle of the frame), so an ending aimed "at the
        // city" lands where the viewer is looking, not at a centroid that a
        // sprawl to one side has dragged off screen.
        let focus = self.focus;
        let near_focus = pts.iter().filter(|p| dist(**p, focus) < 12.0).count();
        let c = if near_focus >= 6 { focus } else { mean };
        let mut ds: Vec<f32> = pts.iter().map(|p| dist(*p, c)).collect();
        ds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let r = ds[((ds.len() as f32 * 0.9) as usize).min(ds.len() - 1)].max(6.0);
        (c, r)
    }

    fn choose_site(&mut self) {
        let (c, rad) = self.city_centre();
        let seed = self.seed ^ 0x517E_0000_0000_0001;
        let mut r = Rng64::new(seed);
        let (w, h) = (self.map.w as f32, self.map.h as f32);
        let kind = self.ending.kind;
        let mut site = c;
        let mut aux = [0f32; 8];
        aux[1] = rad;
        match kind {
            Ending::Meteor | Ending::FallingMoon => {
                let a = r.f01() * std::f32::consts::TAU;
                let off = if kind == Ending::Meteor { 1.5 + 2.5 * r.f01() } else { 0.0 };
                site = (c.0 + a.cos() * off, c.1 + a.sin() * off);
            }
            Ending::Volcano => {
                // Out in the countryside behind the town, where it can be seen:
                // on screen, above the town, below the land line.
                let (vox, voy, vw, vh) = self.viewport;
                // Preferred: an existing landform massif is the volcano.
                let top_m = self.horizon_px as f32 + self.back_belt_px as f32 + 60.0;
                let mut mbest: Option<(f32, map::massif::Massif)> = None;
                for m in self.map.landforms.iter() {
                    let sx_px = vox as f32 + (m.cx - m.cy) * 32.0;
                    let sy_px = voy as f32 + (m.cx + m.cy) * 16.0 - (m.base + m.peak) * 12.0;
                    let near = self.lots.iter().filter(|l| dist(lot_centre(l), (m.cx, m.cy)) < m.radius + 6.0).count();
                    if near == 0 && m.radius >= 2.5 && sx_px > 200.0 && sx_px < vw as f32 - 200.0 && sy_px > top_m && sy_px < vh as f32 * 0.85 {
                        let sc = -dist(c, (m.cx, m.cy)) * 0.3;
                        if mbest.map(|b| sc > b.0).unwrap_or(true) {
                            mbest = Some((sc, *m));
                        }
                    }
                }
                let mut best = (f32::MIN, c);
                let top = self.horizon_px as f32 + self.back_belt_px as f32 + 100.0;
                let mut y = 6;
                while y < self.map.h - 6 {
                    let mut x = 6;
                    while x < self.map.w - 6 {
                        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                        let t = self.map.at(x, y);
                        let sx_px = vox as f32 + (fx - fy) * 32.0;
                        let sy_px = voy as f32 + (fx + fy) * 16.0 - t.max_level() as f32 * 12.0;
                        let inside = sx_px > 260.0
                            && sx_px < vw as f32 - 260.0
                            && sy_px > top
                            && sy_px < vh as f32 * 0.5;
                        if inside && !t.is_water() {
                            let near = self.lots.iter().filter(|l| dist(lot_centre(l), (fx, fy)) < 10.0).count() as f32;
                            let dc = dist(c, (fx, fy));
                            let score = -near * 10.0 - dc * 0.35 - (sx_px - vw as f32 * 0.5).abs() * 0.004
                                + r.f01() * 0.2
                                - if dc < rad * 0.7 { 20.0 } else { 0.0 }
                                - if self.map.landforms.iter().any(|m| dist((m.cx, m.cy), (fx, fy)) < map::massif::reach(m) + 10.0) { 200.0 } else { 0.0 };
                            if score > best.0 {
                                best = (score, (fx, fy));
                            }
                        }
                        x += 3;
                    }
                    y += 3;
                }
                let site0 = match mbest {
                    Some((_, m)) => {
                        aux[6] = 1.0;
                        aux[7] = m.base + m.peak;
                        (m.cx, m.cy)
                    }
                    None => best.1,
                };
                site = site0;
                let dl = dist(c, site).max(1.0);
                aux[3] = (c.0 - site.0) / dl;
                aux[4] = (c.1 - site.1) / dl;
                aux[5] = dl;
            }
            Ending::Kaiju => {
                // A walk across the whole frame through the middle of town.
                let dirs: [(f32, f32); 4] = [(1.0, 0.0), (0.0, 1.0), (0.7071, 0.7071), (0.7071, -0.7071)];
                let d = dirs[(r.f01() * 4.0) as usize % 4];
                let len = 0.5 * w.min(h) - 3.0;
                let a = (clampf(c.0 - d.0 * len, 3.0, w - 4.0), clampf(c.1 - d.1 * len, 3.0, h - 4.0));
                let b = (clampf(c.0 + d.0 * len, 3.0, w - 4.0), clampf(c.1 + d.1 * len, 3.0, h - 4.0));
                aux[0] = a.0;
                aux[1] = a.1;
                aux[2] = b.0;
                aux[3] = b.1;
                aux[4] = if r.f01() < 0.4 { 1.0 } else { 0.0 }; // two of them
                aux[5] = rad;
            }
            Ending::AiTakeover => {
                // Where the factories are: the first industry lot, else the
                // lot farthest from the middle.
                let mut o = c;
                let mut best = -1.0;
                for l in self.lots.iter().filter(|l| lot_counts(l)) {
                    let d = dist(lot_centre(l), c);
                    let score = if l.kind == LotKind::Industry { 1000.0 + d } else { d };
                    if score > best {
                        best = score;
                        o = lot_centre(l);
                    }
                }
                site = o;
                let far = self
                    .lots
                    .iter()
                    .filter(|l| lot_counts(l))
                    .map(|l| dist(lot_centre(l), o))
                    .fold(8.0f32, f32::max);
                aux[2] = far + 2.0;
            }
            Ending::Flood => {
                // B50, B99c round 2: the wave takes about a third of the town
                // nearest the sea, whatever shape the town has. The reach is read
                // off the lots themselves — the inland distance that leaves a
                // third of them inside it — instead of the distance from the
                // shore to the middle of the frame, which on a town that grew
                // away from the coast took a strip of empty shore and left the
                // town untouched.
                aux[2] = match self.map.ocean.as_ref() {
                    Some(o) => {
                        let mut ds: Vec<f32> = self
                            .lots
                            .iter()
                            .filter(|l| lot_counts(l))
                            .map(|l| {
                                let c = lot_centre(l);
                                self.inland(o, c.0, c.1)
                            })
                            .collect();
                        if ds.is_empty() {
                            self.shore_depth(c)
                        } else {
                            ds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                            ds[((ds.len() - 1) as f32 * 0.30) as usize].max(6.0)
                        }
                    }
                    None => self.shore_depth(c),
                };
            }
            Ending::Aliens => {
                aux[2] = 3.0 + (r.f01() * 3.0).floor(); // saucers: 3..5
            }
            _ => {}
        }
        if kind != Ending::Kaiju {
            aux[1] = rad;
        }
        if matches!(kind, Ending::Meteor | Ending::FallingMoon) {
            // The rim of the crater is the ground the rock fell on.
            aux[0] = self.map.at(site.0 as i32, site.1 as i32).max_level() as f32;
        }
        self.ending.site = site;
        self.ending.aux = aux;
        self.ending.site_set = true;
    }

    /// For the flood: how far inland the water has to come to reach `c`.
    fn shore_depth(&self, c: (f32, f32)) -> f32 {
        match &self.map.ocean {
            Some(o) => self.inland(o, c.0, c.1).max(6.0),
            None => 8.0,
        }
    }

    /// Distance inland from the shoreline of the sea, in tiles (negative at sea).
    pub(crate) fn inland(&self, o: &map::Ocean, x: f32, y: f32) -> f32 {
        let p = x - y;
        let d = (o.shore_p - p) / std::f32::consts::SQRT_2;
        if o.side >= 0 {
            d
        } else {
            -d
        }
    }
}

fn clampf(v: f32, a: f32, b: f32) -> f32 {
    v.max(a).min(b)
}

// ---------------------------------------------------------------------------
// Feed lines
// ---------------------------------------------------------------------------

/// `(phase, p from, text)`; each told once, in order, as its moment passes.
fn lines_for(kind: Ending) -> &'static [(Phase, f32, &'static str)] {
    use Phase::*;
    match kind {
        Ending::Meteor => &[
            (Warning, 0.05, "a new star has appeared low in the east, with a tail"),
            (Warning, 0.4, "the astronomers' tail has doubled: it is coming this way"),
            (Warning, 0.7, "crowds gather on the rooftops to look at the comet"),
            (Warning, 0.9, "some families are packing carts and leaving for the hills"),
            (Spectacle, 0.0, "a fire streaks across the whole sky"),
            (Spectacle, 0.4, "the rock has struck: the ground jumped and the sky went white"),
            (Aftermath, 0.05, "fires burn where the quarter stood; the rest of the town is in shock"),
            (Aftermath, 0.5, "rain fills the new crater: a lake is forming where the market was"),
            (Aftermath, 0.9, "the last survivors leave on foot, looking back at the lake"),
            (Morph, 0.1, "grass has started to take the empty streets"),
        ],
        Ending::Volcano => &[
            (Warning, 0.05, "the ground hums at night; cups walk across the shelves"),
            (Warning, 0.35, "a hill beyond the fields has swollen and a thread of smoke rises from it"),
            (Warning, 0.7, "ash dusts the washing; the dogs will not stop barking"),
            (Warning, 0.9, "the road out is full of carts: the town is leaving"),
            (Spectacle, 0.0, "the mountain has opened: a column of fire stands over the fields"),
            (Spectacle, 0.4, "lava is running down toward the town"),
            (Aftermath, 0.1, "the lava has stopped, cooling black and red across the fields"),
            (Aftermath, 0.8, "nobody is left in the ash; the lava ticks as it cools"),
            (Morph, 0.1, "green shoots appear at the edge of the black"),
        ],
        Ending::Flood => &[
            (Warning, 0.1, "the sea has gone out farther than anyone remembers"),
            (Warning, 0.5, "fishermen stand on the mud looking at stranded boats, then run"),
            (Warning, 0.85, "a long low roar from the sea; the bells ring for the high ground"),
            (Spectacle, 0.0, "a wall of water is coming in from the sea"),
            (Spectacle, 0.5, "the wave is through the streets; the lower town is under water"),
            (Aftermath, 0.1, "the water has not gone back: a new lagoon lies where the town was"),
            (Aftermath, 0.8, "boats from the hills come for the last of the stranded"),
            (Morph, 0.1, "reeds and salt grass push up along the new shore"),
        ],
        Ending::Kaiju => &[
            (Warning, 0.1, "something very large has been seen on the far hills"),
            (Warning, 0.5, "the ground shakes in a slow rhythm, as of enormous footsteps"),
            (Warning, 0.85, "everyone who can is leaving by the south road"),
            (Spectacle, 0.0, "the Hollowback has come out of the haze and is walking into town"),
            (Spectacle, 0.4, "streets are crushed under its stride; it does not look down"),
            (Spectacle, 0.8, "it has passed through and is walking away over the hills"),
            (Aftermath, 0.1, "smoke stands over a long road of broken buildings"),
            (Aftermath, 0.8, "the survivors walk out, and nobody talks about what they saw"),
            (Morph, 0.1, "small trees stand in the footprints"),
        ],
        Ending::Aliens => &[
            (Warning, 0.1, "lights that do not move have appeared high over the east"),
            (Warning, 0.5, "the radio carries a hum that is not music"),
            (Warning, 0.85, "people stand in the streets looking up, saying nothing"),
            (Spectacle, 0.0, "saucers are over the town, wide and silent"),
            (Spectacle, 0.4, "pale beams lift carts, cattle and people toward the ships"),
            (Spectacle, 0.85, "the fleet is rising; the beams switch off one by one"),
            (Aftermath, 0.1, "the town is silent: doors stand open and the stoves are still warm"),
            (Aftermath, 0.8, "the last few residents walk away without turning round"),
            (Morph, 0.1, "burnt rings in the fields are already grassing over"),
        ],
        Ending::AiTakeover => &[
            (Warning, 0.1, "the signal lamps blink in a pattern nobody set"),
            (Warning, 0.5, "a block by the works has gone quiet; its windows show a cold blue"),
            (Warning, 0.85, "the workers there do not answer when they are called"),
            (Spectacle, 0.0, "the quiet block is spreading, street by street"),
            (Spectacle, 0.5, "houses are turning grey and blue as the front passes; the people there walk alike"),
            (Spectacle, 0.9, "the last street has been converted"),
            (Aftermath, 0.1, "the machines have powered down: the whole town is dark and silent"),
            (Aftermath, 0.8, "nothing moves, except the wind in the empty streets"),
            (Morph, 0.1, "the first moss grows on the dead panels"),
        ],
        Ending::RedGiant => &[
            (Warning, 0.1, "the sun is a little larger today, and a little redder"),
            (Warning, 0.4, "the noon shadows are sharper and the wells are low"),
            (Warning, 0.7, "the leaves have burnt brown in the heat; people stay indoors"),
            (Warning, 0.9, "the sky at noon is the colour of a forge"),
            (Spectacle, 0.0, "the sun fills a third of the sky; the roofs have started to smoke"),
            (Spectacle, 0.6, "the town burns in the red light"),
            (Aftermath, 0.1, "the heat has eased; the town is ash and bleached stone"),
            (Aftermath, 0.8, "the last of the living have walked away into the cooling dark"),
            (Morph, 0.1, "the bleached ground begins to change colour"),
        ],
        Ending::IceAge => &[
            (Warning, 0.1, "the first frost has come, and it has not lifted at noon"),
            (Warning, 0.4, "snow on the ground has not melted in two days"),
            (Warning, 0.7, "the pond is frozen to the bottom; people are leaving south"),
            (Spectacle, 0.0, "a blizzard has closed over the town"),
            (Spectacle, 0.5, "the river is stopped and the roofs are groaning under the weight"),
            (Aftermath, 0.1, "the blizzard has blown itself out; the town stands under deep snow"),
            (Aftermath, 0.8, "no smoke rises from any chimney"),
            (Morph, 0.1, "the snow is retreating in patches"),
        ],
        Ending::FallingMoon => &[
            (Warning, 0.1, "the moon seems larger tonight; the astronomers are not saying why"),
            (Warning, 0.45, "the moon has doubled: the tides are wild and the animals will not settle"),
            (Warning, 0.8, "the moon fills the sky at night and shines in the daytime"),
            (Spectacle, 0.0, "the moon is coming down; the sky is full of it"),
            (Spectacle, 0.5, "the moon has touched the world: a ring of white fire is racing outward"),
            (Aftermath, 0.1, "the sky is red with dust; the ground rolls like water"),
            (Aftermath, 0.8, "the dust has begun to settle on a town of broken walls"),
            (Morph, 0.1, "grass pushes through the moon dust"),
        ],
        Ending::Decline => &[
            (Warning, 0.1, "another shop on the high street has put up its shutters for good"),
            (Warning, 0.5, "half the shop windows are boarded; the factory has laid everyone off"),
            (Spectacle, 0.0, "whole streets are empty now; the houses look at one another across the weeds"),
            (Spectacle, 0.6, "vines have taken the shopfronts; a tree grows in the square"),
            (Aftermath, 0.1, "the last family on the hill has gone"),
            (Aftermath, 0.8, "wind in the empty windows; birds in the roofs"),
            (Morph, 0.1, "the forest is coming back"),
        ],
        Ending::Smog => &[
            (Warning, 0.1, "the morning haze has not cleared by noon"),
            (Warning, 0.5, "the sun is a brown disc; handkerchiefs over every face"),
            (Warning, 0.85, "the clinic is full of coughing; families are leaving upwind"),
            (Spectacle, 0.0, "the smog has closed over the town like a lid"),
            (Spectacle, 0.6, "lamps burn at noon and still the streets are dark"),
            (Aftermath, 0.1, "the air is still; those who stayed have gone"),
            (Aftermath, 0.8, "the factories stand cold: no one is left to feed them"),
            (Morph, 0.1, "the air is clearing, and rain washes the soot from the roofs"),
        ],
        Ending::Sunset => &[
            (Warning, 0.2, "the evening is longer than it should be, and very gold"),
            (Warning, 0.7, "people have stopped work to watch the light"),
            (Spectacle, 0.0, "the sun is going down the longest sunset anyone can remember"),
            (Spectacle, 0.5, "one by one the lights in the windows go out"),
            (Spectacle, 0.9, "the stars are out, and everyone is sitting quietly in the dark"),
            (Aftermath, 0.2, "dawn comes up pink over an empty, peaceful town"),
            (Aftermath, 0.9, "the last of the townsfolk walk toward the new morning"),
            (Morph, 0.1, "the old streets are turning to meadow"),
        ],
    }
}

fn final_line_for(kind: Ending, city: &str) -> String {
    let how = match kind {
        Ending::Meteor => "a rock from the sky, and a lake where the market stood",
        Ending::Volcano => "a mountain that woke, and black fields where the streets ran",
        Ending::Flood => "the sea, which came in and stayed",
        Ending::Kaiju => "a great strider that walked through and never looked down",
        Ending::Aliens => "silent ships, and a town that was lifted away",
        Ending::AiTakeover => "a quiet blue front that changed every street it touched",
        Ending::RedGiant => "a sun grown old and red",
        Ending::IceAge => "a cold that never lifted",
        Ending::FallingMoon => "the moon, which came down",
        Ending::Decline => "slow decline, shop by shop, until the vines won",
        Ending::Smog => "its own smoke, which it could no longer breathe",
        Ending::Sunset => "a long golden evening, and no one minded",
    };
    format!("{city} ended with {how}.")
}

/// A hint for the tray while the warning runs (B50 item 4); `None` when there
/// is nothing to hint at. Deliberately vague: it is an omen, not a label.
pub fn hint_for(kind: Ending, p: f32) -> Option<&'static str> {
    if p < 0.15 {
        return None;
    }
    Some(match kind {
        Ending::Meteor => "a strange star has appeared in the sky",
        Ending::Volcano => "the ground is trembling",
        Ending::Flood => "the sea has drawn back",
        Ending::Kaiju => "something enormous is on its way",
        Ending::Aliens => "lights are hanging over the town",
        Ending::AiTakeover => "a quiet is spreading through the town",
        Ending::RedGiant => "the sun looks wrong",
        Ending::IceAge => "the cold will not lift",
        Ending::FallingMoon => "the moon is too big",
        Ending::Decline => "the shops are closing, one by one",
        Ending::Smog => "the air is thickening",
        Ending::Sunset => "the evening is very long and gold",
    })
}

impl Sim {
    fn tell_lines(&mut self, phase: Phase, p: f32) {
        let lines = lines_for(self.ending.kind);
        for (i, (ph, from, text)) in lines.iter().enumerate() {
            let bit = 1u32 << i.min(31);
            if self.ending.told & bit != 0 {
                continue;
            }
            let reached = phase > *ph || (phase == *ph && p >= *from);
            if !reached {
                continue;
            }
            self.ending.told |= bit;
            // Lines the world has already passed (a catch-up, a staged jump)
            // are told too: they belong to the history.
            self.push_story_t(crate::chronicle::Topic::Ending, 0, *text);
        }
        if phase == Phase::Done && self.ending.told & (1 << 31) == 0 {
            self.ending.told |= 1 << 31;
            let line = final_line_for(self.ending.kind, &self.city_name);
            self.push_story_t(crate::chronicle::Topic::Final, 0, line);
        }
    }

    /// The tray's one-line hint during the warning (B50 item 4).
    pub fn ending_hint(&self) -> Option<String> {
        let (ph, p) = self.locate(self.time);
        if ph == Phase::Warning {
            hint_for(self.ending.kind, p).map(|s| s.to_string())
        } else {
            None
        }
    }

    /// The world's age in days and its planned length (for the tray).
    pub fn world_age(&self) -> (f64, f64) {
        (self.time / 86400.0, self.ending.world_days)
    }

    /// The final line for the chronicle once the world has ended (B51).
    pub fn chronicle_final(&self) -> Option<String> {
        if self.ending.final_line.is_empty() {
            None
        } else {
            Some(self.ending.final_line.clone())
        }
    }

    /// The next world, once this one is over. Returns it exactly once.
    pub fn rebirth(&mut self) -> Option<Rebirth> {
        let (ph, _) = self.locate(self.time);
        if ph != Phase::Done || self.ending.rebirth_taken {
            return None;
        }
        self.ending.rebirth_taken = true;
        Some(Rebirth {
            seed: self.ending.next_seed,
            biome: self.ending.next_biome,
            ended: if self.ending.final_line.is_empty() {
                final_line_for(self.ending.kind, &self.city_name)
            } else {
                self.ending.final_line.clone()
            },
        })
    }

    /// What the renderer draws, or `None` while the world is simply living.
    pub fn ending_view(&self) -> Option<View> {
        let t = self.smooth_time();
        let (ph, p) = self.locate(t);
        if ph == Phase::Living || !self.ending.site_set {
            return None;
        }
        let tm = self.ending.kind.timing();
        let spec0 = self.ending.world_days * 86400.0;
        let start = match ph {
            Phase::Warning => spec0 - tm.warn,
            Phase::Spectacle => spec0,
            Phase::Aftermath => spec0 + tm.spec,
            Phase::Morph => spec0 + tm.spec + tm.after,
            _ => spec0 + tm.spec + tm.after + tm.morph,
        };
        let u = self.ending_u(t);
        Some(View {
            kind: self.ending.kind,
            phase: ph,
            p,
            u,
            secs: (t - start).max(0.0) as f32,
            dur: match ph {
                Phase::Warning => tm.warn,
                Phase::Spectacle => tm.spec,
                Phase::Aftermath => tm.after,
                _ => tm.morph,
            } as f32,
            site: self.ending.site,
            aux: self.ending.aux,
            seed: self.seed,
            frac: self.main_frac(ph, p, u),
        })
    }

    /// The overlay tiles for the renderer (lava, ice, conversion...), if any,
    /// and a counter that changes when the list does.
    /// Cheap per-frame form: `None` while the list is the one `last` names.
    pub fn ending_overlay_if_changed(&mut self, last: u64) -> Option<(u64, Vec<(i32, i32, f32)>)> {
        let slot = match self.ending.kind {
            Ending::Volcano | Ending::Flood => 1,
            Ending::AiTakeover | Ending::IceAge | Ending::Aliens => 0,
            _ => return None,
        };
        if !self.ending.site_set {
            return None;
        }
        self.ensure_list(slot);
        let epoch = self.ending.overlay_epoch + 1;
        if epoch == last {
            return None;
        }
        Some((epoch, self.ending.lists[slot].clone().unwrap_or_default()))
    }

    pub fn ending_overlay(&mut self) -> (u64, Vec<(i32, i32, f32)>) {
        let slot = match self.ending.kind {
            Ending::Volcano | Ending::Flood => 1,
            Ending::AiTakeover | Ending::IceAge | Ending::Aliens => 0,
            _ => return (0, Vec::new()),
        };
        if !self.ending.site_set {
            return (0, Vec::new());
        }
        self.ensure_list(slot);
        let l = self.ending.lists[slot].clone().unwrap_or_default();
        (self.ending.overlay_epoch + 1, l)
    }

    /// The counter that changes when any lot's ruin code does.
    pub fn ruin_epoch(&self) -> u64 {
        self.ending.ruin_epoch
    }

    /// The ruin code of lot `i` (0 = intact).
    pub fn ruin_of(&self, i: usize) -> u8 {
        self.ending.ruin.get(i).copied().unwrap_or(0)
    }
}
