//! B73/D80: nightlife — a night character for each part of town, and a sign
//! generator that gives every lot its own.
//!
//! The wallpaper used to put the same handful of 1–2 px blade signs on every
//! shop (`buildkit::color::NEON_*`). This module replaces that with **descriptors**:
//! a [`Plan`] is generated once per city change from the renderer's own objects
//! (`CityObj::style` is the sim's district id, `seed` the lot's), and every frame
//! draws the visible spots on top of the static layer from ~32 bytes apiece — no
//! baked sprite per sign state, no bitmap per animation frame.
//!
//! Characters (D80, inspiration only, original art):
//! * [`Character::Neon`] — the cyberpunk quarter (Kabukicho, Dotonbori, Mong Kok,
//!   Myeongdong): stacked blades at several heights, mixed saturated colours.
//! * [`Character::Square`] — the bright square (Times Square, Piccadilly,
//!   Shibuya): huge billboards and screens, marquees with chaser bulbs.
//! * [`Character::HighStreet`] — the old high street (London, Amsterdam, Prague):
//!   warm hanging signs, painted fascias, string lights, a cinema marquee.
//! * [`Character::Business`] — crown lighting in each tower's own colours, LED
//!   edge lines, lit lobbies, aviation lights, invented company light boxes.
//! * [`Character::Suburb`] / [`Character::Village`] — a corner shop, a diner, a
//!   petrol canopy, a motel sign; a village stays dark and cosy.
//!
//! Everything is invented: names are letter-like glyph shapes ([`GLYPHS`]) with
//! no alphabet, so no real word or brand can be spelled (D75).

use crate::bands::Bands;
use crate::city::{CityObj, LandmarkKind, ObjKind, Stage};
use crate::framebuffer::{Framebuffer, Rect};
use crate::iso::{Camera, HEIGHT_UNIT};
use crate::palette::{self, rgb, Color};
use crate::scene::KitSprite;
use map::TileMap;

/// How often the animated panels are redrawn, in Hz (D71: the wallpaper caps at
/// 30 fps; a chaser does not need every frame, and this is what keeps the damage
/// small).
pub const ANIM_HZ: f32 = 6.0;

/// The animation's slots are dealt out to this many lanes in rotation, by
/// character: each sign still updates a few times a second, but only a slice of
/// the city's animated panels is damaged on any one slot (B73r2: the frame's
/// damage budget, measured — see the handback).
pub const ANIM_LANES: i64 = 2;

/// How high a steam puff rises before it fades out, in screen pixels.
pub const STEAM_RISE: i32 = 22;

/// A screen's brightness by the residents' clock (brief item 6): full until
/// 01:00, dimming across the next hour, dark from 02:00 until it opens again.
pub fn screen_dim(hour: f32) -> f32 {
    let h = hour.rem_euclid(24.0);
    if h < 1.0 {
        1.0
    } else if h >= 2.0 {
        0.0
    } else {
        1.0 - (h - 1.0)
    }
}

// --- characters -------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Character {
    Village = 0,
    Suburb = 1,
    HighStreet = 2,
    Business = 3,
    Neon = 4,
    Square = 5,
}

impl Character {
    pub const ALL: [Character; 6] = [
        Character::Village,
        Character::Suburb,
        Character::HighStreet,
        Character::Business,
        Character::Neon,
        Character::Square,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: u8) -> Self {
        Self::ALL[(i as usize).min(Self::ALL.len() - 1)]
    }

    pub fn name(self) -> &'static str {
        match self {
            Character::Village => "village",
            Character::Suburb => "suburb",
            Character::HighStreet => "high street",
            Character::Business => "business",
            Character::Neon => "neon quarter",
            Character::Square => "bright square",
        }
    }

    /// The lit palette: colour pairs are picked from here, per sign.
    pub fn palette(self) -> &'static [(u8, u8, u8)] {
        match self {
            // Magenta, cyan, lime, red, amber, violet, blue, warm white.
            Character::Neon => &[
                (0xff, 0x3e, 0xc8),
                (0x2e, 0xe6, 0xff),
                (0xa6, 0xff, 0x3c),
                (0xff, 0x44, 0x38),
                (0xff, 0xb0, 0x3c),
                (0x8c, 0x5c, 0xff),
                (0x4a, 0x86, 0xff),
                (0xff, 0xe8, 0xc0),
            ],
            // Large, bright, few: screen white, red, magenta, amber, cyan, purple.
            Character::Square => &[
                (0xe8, 0xf4, 0xff),
                (0xff, 0x3a, 0x46),
                (0xff, 0x58, 0xc8),
                (0xff, 0xcc, 0x44),
                (0x40, 0xd8, 0xff),
                (0xa0, 0x70, 0xff),
                (0x50, 0xdc, 0x80),
                (0xff, 0x8a, 0x30),
            ],
            // Warm whites and ambers, one cinema red.
            Character::HighStreet => &[
                (0xff, 0xdc, 0xae),
                (0xf0, 0xb0, 0x60),
                (0xff, 0xee, 0xcc),
                (0xe8, 0xc8, 0x4c),
                (0xd8, 0x60, 0x40),
                (0xc8, 0xe0, 0x9c),
                (0xf8, 0x9c, 0x58),
                (0xd8, 0xe8, 0xb0),
            ],
            // Each tower's own: warm white, gold, red, green, blue.
            Character::Business => &[
                (0xff, 0xe8, 0xc8),
                (0xe8, 0xc8, 0x4c),
                (0xd8, 0x40, 0x38),
                (0x48, 0xb0, 0x70),
                (0x58, 0x90, 0xe0),
                (0xc8, 0xd8, 0xe8),
                (0x9c, 0x6c, 0xf0),
                (0x50, 0xc8, 0xc0),
            ],
            // Little neon: warm, practical colours.
            Character::Suburb => &[
                (0xff, 0xc0, 0x70),
                (0xe0, 0x50, 0x40),
                (0x70, 0xa0, 0xe0),
                (0x60, 0xc0, 0x80),
                (0xe8, 0xe8, 0xe0),
            ],
            Character::Village => &[
                (0xff, 0xc8, 0x80),
                (0xff, 0xe0, 0xb0),
                (0xc8, 0x50, 0x40),
                (0xe0, 0xd0, 0xa0),
            ],
        }
    }
}

// --- forms ------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    /// A vertical blade projecting over the street.
    Blade = 0,
    /// A small box on the wall.
    WallBox = 1,
    /// A band along the shopfront, following the facade.
    Fascia = 2,
    /// A letter frame standing on the roof.
    Rooftop = 3,
    /// A large flat billboard.
    Billboard = 4,
    /// A screen that changes its ad.
    Screen = 5,
    /// A marquee with chaser bulbs.
    Marquee = 6,
    /// A small sign hanging from an iron bracket.
    Bracket = 7,
    /// A cloth banner across the street.
    Banner = 8,
    /// A lit shopfront pane.
    Window = 9,
    /// A crown light band on a tower's roof edge.
    Crown = 10,
    /// String lights strung across the street (a lantern string, a bar string).
    String = 11,
}

impl Form {
    pub const ALL: [Form; 12] = [
        Form::Blade,
        Form::WallBox,
        Form::Fascia,
        Form::Rooftop,
        Form::Billboard,
        Form::Screen,
        Form::Marquee,
        Form::Bracket,
        Form::Banner,
        Form::Window,
        Form::Crown,
        Form::String,
    ];

    pub fn from_index(i: u8) -> Self {
        Self::ALL[(i as usize).min(Self::ALL.len() - 1)]
    }

    pub fn name(self) -> &'static str {
        match self {
            Form::Blade => "blade",
            Form::WallBox => "wall box",
            Form::Fascia => "fascia",
            Form::Rooftop => "rooftop frame",
            Form::Billboard => "billboard",
            Form::Screen => "screen",
            Form::Marquee => "marquee",
            Form::Bracket => "hanging",
            Form::Banner => "banner",
            Form::Window => "window",
            Form::Crown => "crown",
            Form::String => "string lights",
        }
    }
}

// --- flags ------------------------------------------------------------------

pub const CHASER: u8 = 1 << 0;
pub const SCREEN: u8 = 1 << 1;
pub const CYCLE: u8 = 1 << 2;
pub const FLICKER: u8 = 1 << 3;
pub const SPILL: u8 = 1 << 4;
pub const ALL_NIGHT: u8 = 1 << 5;
/// One half of a screen wrapped round a corner (both halves share glyph/palette,
/// so they show the same ad).
pub const WRAP: u8 = 1 << 6;
/// A screen whose ad scrolls sideways instead of cutting to another one.
pub const SCROLL: u8 = 1 << 7;

/// Does this flag set animate (and therefore need frame damage)?
pub fn animated(flags: u8) -> bool {
    flags & (CHASER | SCREEN | CYCLE | FLICKER) != 0
}

// --- land -------------------------------------------------------------------

/// The land's tint on the palettes (D66/D80). Pick per land:
/// * Desert — lanterns and warm string lights: everything pulled to amber.
/// * Tropical — bar strings and tiki torches: warm, with more banners.
/// * Tundra / Alpine / Highlands — warm windows on snow, a cool cast on the rest.
/// * Swamp / Lagoon / Islands — warm, slightly green.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Land {
    Temperate,
    Desert,
    Tropical,
    Tundra,
    Green,
}

impl Land {
    pub fn of(b: map::Biome) -> Self {
        use map::Biome::*;
        match b {
            Desert | Savanna | Steppe => Land::Desert,
            Tropical | Lagoon | Islands | Bamboo => Land::Tropical,
            Tundra | Alpine | Highlands => Land::Tundra,
            Swamp | Volcanic => Land::Green,
            _ => Land::Temperate,
        }
    }

    fn tint(self, c: Color) -> Color {
        let (r, g, b) = (
            palette::r(c) as f32 / 255.0,
            palette::g(c) as f32 / 255.0,
            palette::b(c) as f32 / 255.0,
        );
        let (r, g, b) = match self {
            Land::Temperate => (r, g, b),
            Land::Desert => (
                r * 0.9 + 0.22,
                g * 0.85 + 0.10,
                b * 0.62,
            ),
            Land::Tropical => (r * 0.95 + 0.08, g * 0.92 + 0.05, b * 0.85),
            Land::Tundra => (r * 0.86, g * 0.93, b * 1.0),
            Land::Green => (r * 0.94, g * 0.98, b * 0.88),
        };
        rgb(
            (r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (b.clamp(0.0, 1.0) * 255.0).round() as u8,
        )
    }
}

// --- letters ----------------------------------------------------------------

/// Invented glyphs: 3x5 marks that read as lettering at 1× and spell nothing.
/// There is deliberately no alphabet: [`rune`] maps every glyph to a private-use
/// code point, so no real word or brand can be composed (D75).
pub const GLYPHS: [u16; 18] = [
    0b111_101_101_101_111, // ring
    0b110_010_010_010_111, // stem with a foot
    0b111_001_101_100_111, // top bar
    0b111_001_111_001_111, // double bar
    0b101_101_111_001_001, // fork
    0b111_100_110_001_111, // hook
    0b110_101_101_101_110, // arch
    0b111_001_010_100_111, // zig
    0b101_101_101_101_111, // grid
    0b010_101_111_001_010, // arrow
    0b111_100_111_100_111, // ladder
    0b101_001_010_100_101, // cross
    0b111_010_010_010_111, // pillar
    0b100_110_101_101_101, // flag
    0b001_010_111_010_001, // star
    0b111_000_111_000_111, // stripes
    0b101_111_101_101_101, // crown
    0b111_101_111_101_101, // box
];

/// The private-use rune a glyph stands for: every one is outside ASCII, so a
/// sign can never spell a word from any alphabet (used by the banned-word test).
pub fn rune(g: u16) -> char {
    core::char::from_u32(0xE000 + (g as u32 & 0x7FFF)).unwrap_or('\u{E000}')
}

// --- town & plan ------------------------------------------------------------

/// What the sim knows about the town (D80/D57). The nightlife is gated on
/// **people and era**, like the tower cap — not on how many lots happen to be
/// in the frame: a big town's suburbs are not a big town.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TownStats {
    /// Residents and workers, `sim::Sim::population`.
    pub population: u32,
    /// `sim::Era` as an index: 0 settlement, 1 village, 2 town, 3 city, 4 metropolis.
    pub era: u8,
}

impl TownStats {
    /// The era index for a population, when the caller has no sim (tools, tests).
    fn estimated(population: u32) -> u8 {
        match population {
            0..=39 => 0,
            40..=139 => 1,
            140..=1199 => 2,
            1200..=3999 => 3,
            _ => 4,
        }
    }
}

/// Residents before the town earns its first night-life landmark and its bright
/// square (D80: "only appear once a town is big enough"). Tunable — and they were
/// tuned up after the day-25 world (6 200 people) came back with a neon quarter
/// on its suburban streets: a city of six thousand is not a Mong Kok.
pub const NEON_POP: u32 = 9_000;
pub const SQUARE_POP: u32 = 12_000;
/// Residents before a second neon block (a Mong Kok, then another one).
pub const HUGE_POP: u32 = 24_000;
/// Residents under which every character is `Village` (D80: a village stays cosy).
pub const VILLAGE_POP: u32 = 250;

/// What the city is, as far as the nightlife cares.
#[derive(Clone, Copy, Debug, Default)]
pub struct Town {
    pub lots: usize,
    pub max_storeys: u8,
    /// From the sim (`TownStats`), or estimated when there is no sim.
    pub population: u32,
    pub era: u8,
    /// Big enough for a neon quarter and a bright square (D80: they appear as
    /// the town grows).
    pub big: bool,
    /// Big enough for a second neon block.
    pub huge: bool,
}

impl Town {
    /// A village: one small sign on the store is all it wants.
    pub fn village(&self) -> bool {
        self.population < VILLAGE_POP && self.era <= 1
    }
}


/// One sign, ~40 bytes, drawn per frame (never baked per state).
#[derive(Clone, Copy, Debug)]
pub struct Spot {
    /// Wall anchor in world tiles.
    pub wx: f32,
    pub wy: f32,
    /// Screen-space height of the sign's bottom edge (world z, px).
    pub z: f32,
    pub w: u8,
    pub h: u8,
    pub form: u8,
    pub character: u8,
    pub pal: u8,
    /// 0 = the +x wall, 1 = the +y wall (which way the panel slants).
    pub face: u8,
    /// Opening hours in the residents' clock (close > 24 wraps past midnight).
    pub open: f32,
    pub close: f32,
    pub flags: u8,
    pub glyph: u32,
    /// The lot it stands on (the neighbour test compares different lots).
    pub lot: u32,
    /// The painter key of the wall it hangs on, so a sign behind a nearer
    /// building is not drawn through it.
    pub key: u16,
}

impl Spot {
    pub fn form(&self) -> Form {
        Form::from_index(self.form)
    }

    pub fn character(&self) -> Character {
        Character::from_index(self.character)
    }

    /// Is the sign switched on at `hour`? (D78: the residents' clock.)
    pub fn open_at(&self, hour: f32) -> bool {
        if self.flags & ALL_NIGHT != 0 {
            return true;
        }
        let h = if hour < self.open { hour + 24.0 } else { hour };
        h >= self.open && h < self.close
    }

    /// How bright the sign is: 0 by day, up as the evening comes on (B99d's
    /// fade), and 0 outside its opening hours. A **screen** dims after 01:00 and
    /// is dark by 02:00 (D80, brief item 6).
    pub fn lit_at(&self, night: f32, hour: f32) -> f32 {
        let dusk = ((night - 0.22) / 0.55).clamp(0.0, 1.0);
        if !self.open_at(hour) {
            return 0.0;
        }
        if self.flags & SCREEN != 0 {
            return dusk * screen_dim(hour);
        }
        dusk
    }

    /// A stable identity for the design count and the neighbour test.
    pub fn design_key(&self) -> (u8, u8, u8, u8, u8, u16) {
        (self.form, self.character, self.pal, self.w, self.h, (self.glyph & 0xFFFF) as u16)
    }
}

/// An aviation light on the tallest towers.
#[derive(Clone, Copy, Debug)]
pub struct Beacon {
    pub wx: f32,
    pub wy: f32,
    pub z: f32,
    pub phase: f32,
}

/// A steam vent in the neon quarter (D80: "steam from vents", cheap particles;
/// the crowd belongs to B75 — this is the hook it will hang on).
#[derive(Clone, Copy, Debug)]
pub struct Vent {
    pub wx: f32,
    pub wy: f32,
    /// Screen height of the vent mouth above ground, in px.
    pub z: f32,
    /// Its own pulse, 0..1.
    pub phase: f32,
    pub strength: f32,
    /// Which of the neon palette's colours lights its steam.
    pub tint: u8,
}

/// The bright square's light: one pool over the whole junction, "like day"
/// (D80). Drawn before every sign's own pool, so the square's wash is the base.
#[derive(Clone, Copy, Debug)]
pub struct SquarePool {
    pub wx: f32,
    pub wy: f32,
    /// The junction's own height above sea level, in px: the city does not sit
    /// at z = 0, and a pool there would land two tiles down the street.
    pub z: f32,
    /// Radius in screen pixels (the junction is ~4 tiles across).
    pub radius: f32,
    pub strength: f32,
}

/// Everything the nightlife needs, generated from the city once per change.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub spots: Vec<Spot>,
    pub beacons: Vec<Beacon>,
    pub vents: Vec<Vent>,
    pub pool: Option<SquarePool>,
    pub town: Town,
    /// The neon quarter(s) in tiles (one block, a second in a very large town).
    pub anchors: [Option<(f32, f32)>; 2],
    /// The bright square, in tiles: a real junction or the civic plaza.
    pub square: Option<(f32, f32)>,
    /// True when the square came from a plaza lot (rather than a junction).
    pub square_is_plaza: bool,
    pub chars: [u32; 6],
    pub forms: [u32; 12],
    /// Screen anchor per spot, and the box it paints (for damage and clipping).
    px: Vec<(i32, i32)>,
    /// The pavement pixel under each sign (for the coloured light).
    spill_px: Vec<(i32, i32)>,
    beacon_px: Vec<(i32, i32)>,
    vent_px: Vec<(i32, i32)>,
    pool_px: Option<(i32, i32)>,
    /// The painter key of the junction's own ground, so the square's pool lands
    /// on the roads and the facades around it and not through a nearer block.
    pool_key: u16,
    rects: Vec<Rect>,
    bands: Bands,
    height: i32,
    /// How many distinct signed designs the plan holds.
    pub designs: usize,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.spots.is_empty()
    }

    pub fn spot_count(&self) -> usize {
        self.spots.len()
    }

    /// The screen box of spot `i` (valid after [`Plan::project`]).
    pub fn rect(&self, i: usize) -> Rect {
        self.rects.get(i).copied().unwrap_or(Rect::EMPTY)
    }

    pub fn screen(&self, i: usize) -> (i32, i32) {
        self.px.get(i).copied().unwrap_or((0, 0))
    }

    /// The sprites have no room above the horizon: project every spot for the
    /// camera in force. Called when the plan is built and whenever the camera
    /// moves (the wallpaper resizes), never per frame.
    pub fn project(&mut self, cam: &Camera, height: i32) {
        self.px.clear();
        self.spill_px.clear();
        self.beacon_px.clear();
        self.vent_px.clear();
        self.rects.clear();
        self.height = height;
        self.px.reserve(self.spots.len());
        self.rects.reserve(self.spots.len());
        for s in &self.spots {
            let p = cam.project(s.wx, s.wy, s.z.round() as i32);
            let (x, y) = (p.0.round() as i32, p.1.round() as i32);
            // A banner reaches sideways; the box is generous by one arm/leg.
            let w = s.w as i32 + 2;
            let h = s.h as i32 + 2;
            self.px.push((x, y));
            // The pavement is just outside the wall the sign hangs on.
            let (gx, gy) = if s.face == 0 {
                (s.wx + 1.2, s.wy)
            } else {
                (s.wx, s.wy + 1.2)
            };
            let g = cam.project(gx, gy, 0);
            self.spill_px.push((g.0.round() as i32, g.1.round() as i32));
            self.rects.push(Rect::new(x - 1, y - h, x + w, y + 2));
        }
        self.beacon_px.reserve(self.beacons.len());
        for b in &self.beacons {
            let p = cam.project(b.wx, b.wy, b.z.round() as i32);
            self.beacon_px.push((p.0.round() as i32, p.1.round() as i32));
        }
        self.vent_px.reserve(self.vents.len());
        for v in &self.vents {
            let p = cam.project(v.wx, v.wy, v.z.round() as i32);
            self.vent_px.push((p.0.round() as i32, p.1.round() as i32));
        }
        self.pool_px = None;
        if let Some(p) = self.pool {
            let q = cam.project(p.wx, p.wy, p.z.round() as i32);
            self.pool_px = Some((q.0.round() as i32, q.1.round() as i32));
            self.pool_key = crate::actors::static_key((p.wx.floor() + p.wy.floor()) as i32, false);
        }
        self.bands
            .rebuild(height, self.rects.iter().enumerate().map(|(i, r)| (r.y0, i as u32)));
    }

    /// Visible spots in painter order for `clip`: the band index keeps a damaged
    /// rectangle's cost proportional to the signs on its rows.
    pub fn for_each_visible(&self, clip: Rect, mut f: impl FnMut(usize)) {
        self.bands.for_each(clip, 0, |i| {
            let i = i as usize;
            if self.rects[i].intersects(&clip) {
                f(i);
            }
        });
    }

    /// Damaged boxes of the animated panels right now: the caller merges them.
    /// `slot` is the animation quantum, so this runs a few times a second.
    pub fn animated_rects(&self, clip: Rect, slot: i64, out: &mut Vec<Rect>) {
        let lane = slot.rem_euclid(ANIM_LANES);
        // The slot this lane last saw: a screen compares against its own last
        // picture, not the slot in between, or its ad never turns over.
        let prev = slot - ANIM_LANES;
        for (i, s) in self.spots.iter().enumerate() {
            if (s.character as i64).rem_euclid(ANIM_LANES) != lane {
                continue;
            }
            if !animated(s.flags) || !animates_this_slot(s, slot, prev) {
                continue;
            }
            let r = self.rect(i);
            if r.intersects(&clip) {
                out.push(r);
            }
        }
    }

    /// The boxes the steam's puffs sweep: they move every frame, so the caller
    /// damages them every frame (D71: the steam is never slowed, only scoped).
    pub fn steam_rects(&self, clip: Rect, out: &mut Vec<Rect>) {
        if self.vents.is_empty() {
            return;
        }
        for &(x, y) in &self.vent_px {
            // A puff rises ~22 px and drifts a couple sideways.
            let r = Rect::new(x - 4, y - STEAM_RISE - 2, x + 5, y + 2);
            if r.intersects(&clip) {
                out.push(r);
            }
        }
    }

    /// Is there anything that changes every frame in `clip` (steam)?
    pub fn steam_live(&self) -> bool {
        !self.vents.is_empty()
    }
}

/// Does this sign's picture change on this animation slot? A chaser steps every
/// slot; a screen only when its ad turns over; a slow cycle every ~4 s; a
/// flicker only in its rare dark moment. This is what keeps a night city's
/// damage at a handful of small boxes a frame.
fn animates_this_slot(s: &Spot, slot: i64, prev: i64) -> bool {
    if !animated(s.flags) {
        return false;
    }
    // A slot is 1/ANIM_HZ s, and `prev` is the slot before this one on this
    // sign's own lane.
    if s.flags & CHASER != 0 {
        return true;
    }
    // A scrolling ad moves a pixel every slot.
    if s.flags & SCROLL != 0 {
        return true;
    }
    if s.flags & SCREEN != 0 {
        return ad_index(s, slot) != ad_index(s, prev);
    }
    if s.flags & CYCLE != 0 {
        return (slot / 4) != (prev / 4);
    }
    if s.flags & FLICKER != 0 {
        return flickering(s, slot) != flickering(s, prev);
    }
    false
}

/// The ad a screen shows in `slot`: one every 8–15 s (D80), from its seed.
fn ad_index(s: &Spot, slot: i64) -> i64 {
    let period = 8.0 + (s.glyph % 8) as f32; // 8–15 s
    let t = slot as f32 / ANIM_HZ;
    (t / period) as i64
}

fn flickering(s: &Spot, slot: i64) -> bool {
    // A rare bad tube: ~0.5 s every 7–13 s, on its own phase.
    let every = 7.0 + (s.glyph >> 8) as f32 % 6.0;
    let t = slot as f32 / ANIM_HZ + (s.glyph & 0xFF) as f32 * 0.13;
    (t % every) < 0.28
}

// --- generation -------------------------------------------------------------

/// Kinds that can carry a sign. A park, a cemetery or a cabin has nothing to
/// advertise.
fn signable(k: ObjKind) -> bool {
    matches!(
        k,
        ObjKind::Store
            | ObjKind::Shop
            | ObjKind::Townhouse
            | ObjKind::Apartment
            | ObjKind::Office
            | ObjKind::Tower
            | ObjKind::Industry
            | ObjKind::Hall
            | ObjKind::GasStation
            | ObjKind::DriveThrough
            | ObjKind::CarWash
            | ObjKind::Dealership
            | ObjKind::Motel
            | ObjKind::Landmark(LandmarkKind::Station)
            | ObjKind::Landmark(LandmarkKind::SignatureTower)
            | ObjKind::Landmark(LandmarkKind::Museum)
    )
}

/// Shops and ground-floor commerce: the signs a high street is made of.
fn commercial(k: ObjKind) -> bool {
    matches!(
        k,
        ObjKind::Store
            | ObjKind::Shop
            | ObjKind::Dealership
            | ObjKind::DriveThrough
            | ObjKind::CarWash
            | ObjKind::Motel
            | ObjKind::GasStation
    )
}

/// Somebody's home. A residential lot advertises nothing: no neon blade up its
/// wall, no billboard on its roof — at most a small fascia on a mixed-use main
/// street (B73r2 note: a plain townhouse wearing magenta neon read as a bug).
fn residential(k: ObjKind) -> bool {
    matches!(k, ObjKind::House | ObjKind::Townhouse | ObjKind::Apartment)
}

fn hash(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// A stable 0..1 from a lot seed and a salt.
fn h01(seed: u32, salt: u64) -> f32 {
    (hash(seed as u64 ^ salt.wrapping_mul(0x9E37_79B9)) & 0xFFFF) as f32 / 65535.0
}

fn hi(seed: u32, salt: u64, n: u32) -> u32 {
    (hash(seed as u64 ^ salt.wrapping_mul(0x9E37_79B9)) % n.max(1) as u64) as u32
}

/// The neon quarter's radius and the bright square's, in tiles (D80: the
/// quarter is wall-to-wall in its core and thins at its edge; the square wraps
/// the facades around it).
pub const NEON_R: f32 = 12.0;
pub const SQUARE_R: f32 = 7.0;
/// The quarter's dense core (every facade, four to six signs), in tiles.
const NEON_CORE: f32 = 4.5;
/// The square's core (wrapped corners, four panels a facade), in tiles.
const SQUARE_CORE: f32 = 3.5;

/// How strongly a lot belongs to an anchored character: 1 in the core, falling
/// to ~0.25 at the rim, 0 outside.
fn falloff(d: f32, r: f32, core: f32, floor: f32) -> f32 {
    if d <= core {
        1.0
    } else if d >= r {
        0.0
    } else {
        let t = (d - core) / (r - core);
        (1.0 - t * (1.0 - floor)).max(floor)
    }
}

/// The character of a lot and, for an anchored one, its weight in that block
/// (D80: dense core, thinning edge).
fn character_of(o: &CityObj, plan: &Plan, big: bool) -> (Character, f32) {
    if plan.town.village() {
        return (Character::Village, 1.0);
    }
    if big {
        let (cx, cy) = (o.x as f32 + o.w as f32 * 0.5, o.y as f32 + o.h as f32 * 0.5);
        // A home is never part of the *quarter's* light show, and never wears a
        // neon blade (B73r2 note). The one exception is the corner facing the
        // bright square: those are shopfronts with flats over them in any real
        // town, and the brief wants their facades wrapped.
        if signable(o.kind) && (!residential(o.kind) || plan.square.is_some_and(|(ax, ay)| (cx - ax).abs().max((cy - ay).abs()) <= SQUARE_CORE)) {
            // The square wins inside its own radius: it is the brightest place in
            // town, so its facades are wrapped even where the quarter's thinned
            // rim reaches (D80).
            if let Some((ax, ay)) = plan.square {
                let d = (cx - ax).abs().max((cy - ay).abs());
                if d <= SQUARE_R {
                    return (Character::Square, falloff(d, SQUARE_R, SQUARE_CORE, 0.30));
                }
            }
            for a in plan.anchors.iter().flatten() {
                let d = (cx - a.0).abs().max((cy - a.1).abs());
                if d <= NEON_R {
                    return (Character::Neon, falloff(d, NEON_R, NEON_CORE, 0.28));
                }
            }
        }
    }
    match o.style {
        3 => (Character::Business, 1.0),
        0 | 1 => (Character::HighStreet, 1.0),
        _ => (Character::Suburb, 1.0),
    }
}

/// Build the plan for a city. `kit` is the sprite list the objects index into
/// (`KitSprite::foot` is the real wall footprint, `top` the real height), `land`
/// tints the palettes, `cam`/`frame` place the spots on screen.
pub fn build(
    objects: &[CityObj],
    kit: &[KitSprite],
    cam: &Camera,
    land: Land,
    frame_w: i32,
    frame_h: i32,
) -> Plan {
    build_in(objects, kit, cam, land, frame_w, frame_h, TownStats::default(), None)
}

/// As [`build`], with the sim's own town facts (D80 gating: population and era,
/// like the tower cap) and the map (a bright square sits on a real junction or
/// the civic plaza).
pub fn build_in(
    objects: &[CityObj],
    kit: &[KitSprite],
    cam: &Camera,
    land: Land,
    frame_w: i32,
    frame_h: i32,
    stats: TownStats,
    map: Option<&TileMap>,
) -> Plan {
    let mut plan = Plan::default();
    let mut max_storeys = 0u8;
    for o in objects {
        max_storeys = max_storeys.max(o.storeys);
    }
    // Explicit from the sim when it is there; otherwise an estimate from the
    // city itself, so tools and tests still read. The *gate* is population and
    // era either way — never lot count or height (brief item 1).
    let population = if stats.population > 0 {
        stats.population
    } else {
        objects.len() as u32 * 12
    };
    let era = if stats.population > 0 {
        stats.era
    } else {
        TownStats::estimated(population)
    };
    plan.town = Town {
        lots: objects.len(),
        max_storeys,
        population,
        era,
        big: population >= NEON_POP && era >= 2,
        huge: population >= HUGE_POP,
    };
    if objects.is_empty() {
        return plan;
    }
    if plan.town.big {
        let (neon, square, plaza) = anchors(objects, plan.town.huge, cam, frame_w, frame_h, map);
        plan.anchors = neon;
        plan.square = square;
        plan.square_is_plaza = plaza;
        if let Some((sx, sy)) = square {
            if population >= SQUARE_POP {
                // The junction itself, lit like day (D80's "whole junction").
                let z = map
                    .map(|m| m.surface_level_at(sx, sy) * HEIGHT_UNIT as f32)
                    .unwrap_or(0.0);
                plan.pool = Some(SquarePool {
                    wx: sx,
                    wy: sy,
                    z,
                    radius: if plaza { 46.0 } else { 38.0 },
                    strength: 1.0,
                });
            }
        }
    }
    if plan.square.is_none() {
        plan.pool = None;
    }

    let big = plan.town.big;
    // Per character, the last sign put on a lot, so two neighbours never share
    // a form *and* a colour (the brief's rule, not a hope).
    let mut last: [Option<(i32, i32, u8, u8)>; 6] = [None; 6];

    for o in objects {
        if o.stage != Stage::Finished || !signable(o.kind) {
            continue;
        }
        let (ch, weight) = character_of(o, &plan, big);
        let lot_id = (o.x as u32 & 0xFFFF) | ((o.y as u32 & 0xFFFF) << 16);
        let (fx0, fy0, fx1, fy1, top) = walls(o, kit);
        if fx1 - fx0 < 0.4 || fy1 - fy0 < 0.4 {
            continue;
        }
        let base = o.level as i32 * HEIGHT_UNIT;
        let walls_len = ((fx1 - fx0) + (fy1 - fy0)) * 0.5;
        let mut n = wanted(o, ch, weight);
        if n == 0 {
            continue;
        }
        // A balcony-wide facade cannot hold four signs: cap by its length.
        let cap = match ch {
            Character::Neon => 12,
            Character::Square => 5,
            _ => 2,
        };
        n = n.min(cap.max(1));
        for k in 0..n {
            // Width/height per form: a blade is tall and thin, a billboard wide.
            let (form, w, h, mut z, flags) = pick(o, ch, k, n, top, walls_len, salt_for(k), land);
            let mut h = h;
            if !matches!(form, Form::Crown | Form::Rooftop) {
                // A stack climbs the whole facade: evenly spaced from the shop
                // fronts to just under the roof, whatever the building's height.
                if matches!(ch, Character::Neon | Character::Square) {
                    let room = (top - 10.0).max(8.0);
                    let f = (k as f32 + 0.7) / (n as f32 + 0.7);
                    z = 8.0 + room * f + h01(o.seed, 61 + k as u64) * 3.0;
                }
                // A panel never hangs past its own roof: a three-floor blade on a
                // one-storey shop would float over the skyline.
                h = h.min((top - 4.0).max(6.0) as u8).max(2);
                // `z` is where the sign's top edge sits: hang it on the wall, a
                // little above the ground, and keep it below the roofline.
                z = z.max(h as f32 + 2.0);
                if z > top - 1.5 {
                    z = top - 1.5;
                }
                if z - (h as f32) < 1.0 {
                    continue;
                }
            }
            // Face: the street side if it is one of the two the camera sees. In
            // a shelf district the signs alternate faces, so a stack climbs both
            // corners of the building and the block reads from either side.
            let face = if matches!(ch, Character::Neon | Character::Square) {
                (k % 2) as u8
            } else {
                match o.front {
                0 => 0u8,
                1 => 1u8,
                2 => {
                    if h01(o.seed, 11 + k as u64) < 0.6 {
                        1
                    } else {
                        0
                    }
                }
                3 => {
                    if h01(o.seed, 11 + k as u64) < 0.6 {
                        0
                    } else {
                        1
                    }
                }
                _ => 0,
                }
            };
            // Keep the panel on its facade: a wall is `face_len` tiles long (32 px
            // a tile across the screen), so a wide board near the corner would
            // hang off it. The anchor stays central and the width is clamped. A
            // wrapped screen (the square's corner) is allowed to run wider.
            let face_len = if face == 0 { fy1 - fy0 } else { fx1 - fx0 };
            let face_px = (face_len * 32.0).max(6.0);
            let share = if ch == Character::Square { 0.90 } else { 0.66 };
            let w = (w as f32).min(face_px * share).max(4.0) as u8;
            // Signs spread along the facade, not bunched in its middle: a dense
            // quarter's wall is covered from corner to corner.
            let fpos = if matches!(ch, Character::Neon | Character::Square) {
                0.12 + h01(o.seed, 41 + k as u64) * 0.76
            } else {
                0.30 + h01(o.seed, 41 + k as u64) * 0.40
            };
            let (wx, wy) = if face == 0 {
                (fx1, fy0 + (fy1 - fy0) * fpos)
            } else {
                (fx0 + (fx1 - fx0) * fpos, fy1)
            };
            // Colour: the character's palette, but a neighbour gets another.
            let np = ch.palette().len();
            let mut pal = hi(o.seed, 71 + k as u64, np as u32) as u8;
            if let Some((lx, ly, lf, lp)) = last[ch.index()] {
                let near = (lx - o.x).abs() <= 2 && (ly - o.y).abs() <= 2;
                if near && lp == pal && lf == form as u8 {
                    pal = ((pal as usize + 1) % np) as u8;
                }
            }
            last[ch.index()] = Some((o.x, o.y, form as u8, pal));
            let (open, close) = hours(o, ch, form, k);
            let mut flags = flags;
            if open == 0.0 && close >= 24.0 {
                flags |= ALL_NIGHT;
            }
            // Chaser bulbs: the square's marquees and a cinema each on the old
            // high streets. Every marquee chasing would repaint half the frame a
            // second (D71's budget is a few small boxes a frame, not a street).
            // Chaser bulbs: the bright square's marquees (the theatre fronts
            // are there). Every marquee in the city chasing would repaint half
            // the frame a second, which the frame-damage budget cannot carry;
            // the rest keep their bulbs painted at one phase.
            if form == Form::Marquee && ch == Character::Square {
                flags |= CHASER;
            }
            if form == Form::Screen {
                flags |= SCREEN;
                // A third of the square's screens scroll their ad instead of
                // cutting to another (brief item 6).
                if ch == Character::Square && h01(o.seed, 131 + k as u64) < 0.34 {
                    flags |= SCROLL;
                }
            }
            if form == Form::Crown {
                flags |= CYCLE;
            }
            if ch != Character::Village && h01(o.seed, 91 + k as u64) < 0.012 {
                flags |= FLICKER;
            }
            // Every lit sign throws its colour on the pavement around it (brief
            // item 7: not just the big boards).
            flags |= SPILL;
            let glyph = hash(o.seed as u64 ^ (k as u64) << 17 ^ 0xB73) as u32;
            let mut spot = Spot {
                wx,
                wy,
                z: base as f32 + z,
                w,
                h,
                form: form as u8,
                character: ch as u8,
                pal,
                face,
                open,
                close,
                flags,
                glyph,
                lot: lot_id,
                key: crate::actors::static_key(o.diagonal(), true),
            };
            // The square's corners are *wrapped*: a second panel meets the first
            // at the corner with the same ad, so the facade turns the screen
            // round the block (brief item 3).
            let wrap = ch == Character::Square
                && weight >= 0.8
                && k == 0
                && matches!(form, Form::Screen | Form::Billboard)
                && (fx1 - fx0) >= 1.6
                && (fy1 - fy0) >= 1.6;
            if wrap {
                spot.flags |= WRAP;
            }
            plan.chars[ch.index()] += 1;
            plan.forms[form as usize] += 1;
            plan.spots.push(spot);
            if wrap {
                // The other wall at the same corner, a high fpos so the two meet.
                let fp = 0.88;
                let (wx2, wy2) = if face == 0 {
                    (fx0 + (fx1 - fx0) * fp, fy1)
                } else {
                    (fx1, fy0 + (fy1 - fy0) * fp)
                };
                plan.forms[form as usize] += 1;
                plan.spots.push(Spot {
                    wx: wx2,
                    wy: wy2,
                    face: 1 - face,
                    flags: spot.flags | WRAP,
                    ..spot
                });
            }
            // Aviation lights: the tallest towers get one, at the very top.
            if matches!(o.kind, ObjKind::Tower) && top > 58.0 && k == 0 {
                plan.beacons.push(Beacon {
                    wx: (fx0 + fx1) * 0.5,
                    wy: (fy0 + fy1) * 0.5,
                    z: base as f32 + top + 1.0,
                    phase: h01(o.seed, 7),
                });
            }
        }
        // D80: steam from vents in the neon quarter — a handful of the densest
        // blocks breathe, which is what makes a Mong Kok street read as one.
        if ch == Character::Neon
            && weight >= 0.98
            && plan.vents.len() < 4
            && h01(o.seed, 151) < 0.55
        {
            let mid = 0.35 + h01(o.seed, 157) * 0.3;
            let (vx, vy) = if o.front == 1 {
                (fx0 + (fx1 - fx0) * mid, fy1 + 0.55)
            } else {
                (fx1 + 0.55, fy0 + (fy1 - fy0) * mid)
            };
            plan.vents.push(Vent {
                wx: vx,
                wy: vy,
                z: base as f32 + 2.0,
                phase: h01(o.seed, 163),
                strength: 0.65 + h01(o.seed, 167) * 0.5,
                tint: hi(o.seed, 173, 8) as u8,
            });
        }
    }

    // Painter order: far corner first, and the land tint on the palette index is
    // applied while drawing (the index is land-independent).
    plan.spots.sort_by(|a, b| {
        (a.wx + a.wy)
            .total_cmp(&(b.wx + b.wy))
            .then(a.z.total_cmp(&b.z))
            .then(a.form.cmp(&b.form))
    });
    resolve_neighbours(&mut plan.spots);
    let mut keys: Vec<(u8, u8, u8, u8, u8, u16)> = plan.spots.iter().map(|s| s.design_key()).collect();
    keys.sort_unstable();
    keys.dedup();
    plan.designs = keys.len();
    plan.project(cam, frame_h);
    plan
}

/// The brief's rule, enforced after generation: two signs on neighbouring lots
/// never share a form *and* a colour. A dense neon quarter can exhaust one
/// character's palette on a block, so the fallback is another form (with its own
/// size), never "almost" a match.
fn resolve_neighbours(spots: &mut [Spot]) {
    use std::collections::HashMap;
    // Cell grid: 2-tile cells, so a sign only looks at nine cells.
    let cell = |s: &Spot| ((s.wx.floor() as i32) >> 1, (s.wy.floor() as i32) >> 1);
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for i in 0..spots.len() {
        let (cx, cy) = cell(&spots[i]);
        // Candidates already placed nearby.
        let mut clash = Vec::new();
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(v) = grid.get(&(cx + dx, cy + dy)) {
                    for &j in v {
                        let o = &spots[j as usize];
                        if o.lot == spots[i].lot {
                            continue;
                        }
                        if (o.wx - spots[i].wx).abs() <= 1.3 && (o.wy - spots[i].wy).abs() <= 1.3 {
                            clash.push((o.form, o.pal, o.character));
                        }
                    }
                }
            }
        }
        if !clash.is_empty() {
            let ch = spots[i].character();
            let np = ch.palette().len() as u8;
            let mut guard = 0;
            'resolve: while clash
                .iter()
                .any(|&(f, p, c)| f == spots[i].form && p == spots[i].pal && c == spots[i].character)
            {
                // First another colour of the same form ...
                for _ in 0..np {
                    spots[i].pal = (spots[i].pal + 1) % np;
                    if !clash
                        .iter()
                        .any(|&(f, p, c)| f == spots[i].form && p == spots[i].pal && c == spots[i].character)
                    {
                        break 'resolve;
                    }
                }
                // ... then the next form, sized for it, and round again.
                let next = next_form(ch, spots[i].form());
                spots[i].form = next as u8;
                let (w, h) = form_size(next, spots[i].glyph);
                spots[i].w = w;
                spots[i].h = h;
                guard += 1;
                if guard > 40 {
                    break;
                }
            }
        }
        grid.entry((cx, cy)).or_default().push(i as u32);
    }
}

/// The forms a character can fall back to, in order.
fn next_form(ch: Character, f: Form) -> Form {
    use Form::*;
    let ring: &[Form] = match ch {
        Character::Neon => &[Blade, WallBox, Fascia, Bracket, Banner, Billboard],
        Character::Square => &[Billboard, Screen, Marquee, WallBox],
        Character::HighStreet => &[Bracket, Fascia, Window, Banner, WallBox],
        Character::Business => &[WallBox, Window, Rooftop, Crown],
        Character::Suburb => &[Fascia, WallBox, Window, Banner],
        Character::Village => &[WallBox, Fascia, Window],
    };
    let i = ring.iter().position(|&x| x == f).unwrap_or(0);
    ring[(i + 1) % ring.len()]
}

/// A representative size for a form, deterministic from the sign's glyph seed.
fn form_size(f: Form, glyph: u32) -> (u8, u8) {
    let r = (glyph % 7) as u8;
    match f {
        Form::Blade => (2 + r % 2, 8 + r),
        Form::WallBox => (5 + r % 4, 3),
        Form::Fascia => (7 + r % 5, 3),
        Form::Rooftop => (6 + r % 4, 3),
        Form::Billboard => (10 + r % 4, 5),
        Form::Screen => (8 + r % 4, 5),
        Form::Marquee => (6 + r % 3, 3),
        Form::Bracket => (4, 3),
        Form::Banner => (8 + r % 4, 2),
        Form::Window => (3, 2),
        Form::Crown => (8 + r % 4, 1),
        Form::String => (13 + r % 4, 2),
    }
}

fn salt_for(k: usize) -> u64 {
    101 + k as u64 * 37
}

/// The wall footprint in tiles and the top of the building, in px above ground.
fn walls(o: &CityObj, kit: &[KitSprite]) -> (f32, f32, f32, f32, f32) {
    if o.kit != 0 {
        if let Some(k) = kit.get(o.kit as usize - 1) {
            let f = k.foot;
            if f[2] - f[0] > 0.2 && f[3] - f[1] > 0.2 {
                let top = if k.top > 0 { k.top as f32 } else { o.wall_height() as f32 };
                return (
                    o.x as f32 + f[0],
                    o.y as f32 + f[1],
                    o.x as f32 + f[2],
                    o.y as f32 + f[3],
                    top.max(6.0),
                );
            }
        }
    }
    let i = 0.13;
    (
        o.x as f32 + i,
        o.y as f32 + i,
        (o.x + o.w) as f32 - i,
        (o.y + o.h) as f32 - i,
        o.wall_height() as f32,
    )
}

/// How many signs a lot wants, before the facade cap. `w` is the lot's weight
/// in its anchored block (1 in the core, ~0.3 at the rim), so the neon quarter
/// is wall-to-wall where it matters and thins towards its edge (brief item 2).
fn wanted(o: &CityObj, ch: Character, w: f32) -> usize {
    let h = h01(o.seed, 3);
    // A home never wears a *neon* sign (B73r2 note: magenta blades on plain
    // townhouses in a small town read as a bug). On an old main street a flat
    // over a shop keeps its one warm fascia — that is a shopfront, not a light
    // show; on a residential street it keeps nothing at all.
    if residential(o.kind) {
        return match ch {
            Character::HighStreet => 1,
            Character::Village if h < 0.08 => 1,
            _ => 0,
        };
    }
    match ch {
        // D80: the quarter is wall-to-wall signage — every building in it wears
        // signs, whatever its use (a tower's base is a row of shops).
        Character::Neon => {
            if w >= 0.85 {
                // Wall to wall: a Mong Kok block is a stack of signs with a
                // building behind it. The quarter is commerce only now, so the
                // buildings that *do* stand in it carry the whole wall.
                8 + (h * 3.99) as usize
            } else if w >= 0.6 {
                5 + (h < 0.55) as usize
            } else if h < 0.62 {
                2
            } else {
                (h < 0.88) as usize
            }
        }
        // The square wraps its facades: three or four big panels next to it,
        // two or three on the blocks behind.
        Character::Square => {
            if w >= 0.85 {
                4
            } else if w >= 0.55 {
                3
            } else if h < 0.62 {
                2
            } else {
                1
            }
        }
        Character::HighStreet => {
            if commercial(o.kind) || matches!(o.kind, ObjKind::Landmark(LandmarkKind::Museum)) {
                1 + (h < 0.45) as usize
            } else {
                0
            }
        }
        Character::Business => match o.kind {
            ObjKind::Tower | ObjKind::Landmark(LandmarkKind::SignatureTower) => 2,
            ObjKind::Office => 1 + (h < 0.5) as usize,
            _ => 0,
        },
        Character::Suburb => match o.kind {
            ObjKind::Motel | ObjKind::GasStation | ObjKind::Dealership => 1,
            ObjKind::DriveThrough | ObjKind::CarWash => (h < 0.7) as usize,
            k if commercial(k) => (h < 0.34) as usize,
            _ => 0,
        },
        Character::Village => match o.kind {
            ObjKind::Store | ObjKind::Shop => 1,
            _ => (h < 0.08) as usize,
        },
    }
}

/// The form, its size, the height on the facade, and its flags.
#[allow(clippy::too_many_arguments)]
fn pick(
    o: &CityObj,
    ch: Character,
    k: usize,
    n: usize,
    top: f32,
    walls_len: f32,
    salt: u64,
    land: Land,
) -> (Form, u8, u8, f32, u8) {
    let r = h01(o.seed, salt);
    let tall = matches!(o.kind, ObjKind::Tower | ObjKind::Landmark(LandmarkKind::SignatureTower));
    let wide = (walls_len * 20.0).clamp(4.0, 16.0);
    match ch {
        Character::Neon => {
            // A stack up the facade: blades and boxes at 5, 11, 17 px. Bolder
            // than B73 round 1 (brief: "sizes readable at 1x"): blades are three
            // or four floors tall, boxes a shopfront wide.
            let floor = if o.storeys >= 8 { 10.0 } else { 8.0 };
            let z = 10.0 + k as f32 * 7.5 + r * 3.0;
            if tall && k == 0 {
                (Form::Screen, 20 + (r * 12.0) as u8, 12, (top * 0.35).min(56.0), SCREEN)
            } else if o.storeys <= 3 && k + 1 == n && n >= 2 {
                // A light frame standing on a low block's roof: the sky clutter
                // that makes a Mong Kok street read as one.
                (Form::Rooftop, 13 + (r * 7.99) as u8, 6, top + 3.0, 0)
            } else if r < 0.50 {
                let floors = if o.storeys >= 3 { 3.0 + r * 2.0 } else { 2.0 };
                (
                    Form::Blade,
                    4 + (r * 3.99) as u8,
                    (floor * floors).clamp(16.0, 56.0) as u8,
                    z,
                    0,
                )
            } else if r < 0.68 {
                (Form::WallBox, 10 + (r * 9.0) as u8, 6, z, 0)
            } else if r < 0.83 {
                (Form::Fascia, 14 + (r * 9.99) as u8, 4, 7.0, 0)
            } else {
                (Form::Banner, 14 + (r * 8.99) as u8, 4, 8.0 + r * 5.0, 0)
            }
        }
        Character::Square => {
            // D80: the facades *wrapped* in billboards and screens — the largest
            // panels in the city, stacked up a tall facade.
            let z = match k {
                0 => 15.0 + r * 6.0,
                1 => (top * 0.42).clamp(20.0, 64.0),
                _ => 13.0 + r * 4.0,
            };
            if o.storeys <= 4 && k + 1 == n && n >= 2 {
                (Form::Rooftop, 18 + (r * 10.99) as u8, 8, top + 4.0, 0)
            } else if r < 0.42 {
                (Form::Screen, 24 + (r * 2.0) as u8 * 10, 12 + (r > 0.2) as u8 * 4, z, SCREEN)
            } else if r < 0.62 {
                (Form::Marquee, 16 + (r * 9.99) as u8, 6, 9.0, CHASER)
            } else {
                (Form::Billboard, 22 + (r * 2.0) as u8 * 8, 12 + (r * 5.99) as u8, z, 0)
            }
        }
        Character::HighStreet => {
            if residential(o.kind) {
                // The one sign a home may wear: a modest painted fascia.
                (Form::Fascia, 8 + (r * 3.99) as u8, 3, 7.0, 0)
            } else if o.kind == ObjKind::Landmark(LandmarkKind::Museum) || r < 0.05 {
                (Form::Marquee, 11, 4, 8.0, CHASER)
            } else if r < 0.13 {
                (Form::String, 14 + (r * 40.0) as u8, 2, 15.0 + r * 4.0, 0)
            } else if r < 0.44 {
                (Form::Bracket, 5, 5, 9.0 + r * 8.0, 0)
            } else if r < 0.62 {
                (Form::Fascia, (wide * 0.7) as u8 + 8, 3, 7.0, 0)
            } else if r < 0.82 {
                (Form::Window, 5, 4, 6.0, 0)
            } else {
                (Form::Banner, 10 + (r * 5.99) as u8, 3, 11.0 + r * 5.0, 0)
            }
        }
        Character::Business => {
            // B73b round 2: most towers are windows only after dark. A third
            // carry the animated crown band in their own colour, one in ten of
            // those with the slow cycle, and a sixth a rooftop frame; the rest
            // keep a lit base and their own windows.
            if tall && k == 0 && r < 0.34 {
                let flags = if r < 0.034 { CYCLE } else { 0 };
                (Form::Crown, (walls_len * 19.0).clamp(5.0, 15.0) as u8, 1, top - 0.5, flags)
            } else if tall && k == 0 && r < 0.5 {
                (Form::Rooftop, 10 + (r * 4.99) as u8, 4, top + 1.0 + 4.0, 0)
            } else if r < 0.5 {
                (Form::WallBox, 8 + (r * 3.99) as u8, 4, 9.0 + r * 4.0, 0)
            } else {
                (Form::Window, 5, 4, 6.0, 0)
            }
        }
        Character::Suburb => match o.kind {
            ObjKind::Motel => (Form::Blade, 3, 14, 17.0, SPILL),
            ObjKind::GasStation | ObjKind::Dealership => (Form::Banner, 12, 3, 11.0, 0),
            ObjKind::DriveThrough => (Form::WallBox, 9, 4, 9.0, 0),
            ObjKind::CarWash => (Form::Fascia, 9, 3, 7.0, 0),
            _ => {
                if r < 0.4 {
                    (Form::Fascia, 9 + (r * 3.99) as u8, 3, 7.0, 0)
                } else if r < 0.75 {
                    (Form::Window, 5, 4, 6.0, 0)
                } else if matches!(land, Land::Desert | Land::Tropical) {
                    (Form::String, 12, 2, 14.0, 0)
                } else {
                    (Form::WallBox, 8, 4, 9.0, 0)
                }
            }
        },
        Character::Village => {
            if r < 0.5 {
                (Form::WallBox, 6 + (r * 2.99) as u8, 4, 8.0, 0)
            } else {
                (Form::Fascia, 7 + (r * 3.99) as u8, 3, 7.0, 0)
            }
        }
    }
}

/// Opening hours in the residents' clock (D80): shops and offices close in the
/// evening, bars and the night quarter run late, a diner, a petrol station and a
/// motel never close.
fn hours(o: &CityObj, ch: Character, form: Form, k: usize) -> (f32, f32) {
    let r = h01(o.seed, 201 + k as u64);
    match (ch, form) {
        (_, Form::Crown) => (17.0, 29.0),
        (Character::Neon, Form::Screen) => (0.0, 26.0),
        (Character::Square, Form::Screen) | (Character::Square, Form::Billboard) => (0.0, 26.0),
        (Character::Neon, _) => (16.5 + r * 2.0, 25.0 + r * 3.0),
        (Character::Square, Form::Marquee) => (18.0, 25.5),
        (Character::Square, _) => (7.0, 26.0),
        (Character::HighStreet, Form::Marquee) => (12.0, 25.0),
        (Character::HighStreet, _) => (9.0, 21.0 + r * 1.5),
        (Character::Business, Form::Window) => (8.0, 19.0),
        (Character::Business, _) => (9.0, 18.0),
        (_, _) => match o.kind {
            ObjKind::Motel | ObjKind::GasStation => (0.0, 24.0),
            ObjKind::Store | ObjKind::Shop if r < 0.12 => (0.0, 24.0), // an all-night diner
            _ => (8.5, 20.5 + r * 2.0),
        },
    }
}

/// The nightlife's landmarks in a big town (D80): the densest commercial block
/// (the neon quarter), a second one in a very large town, and one bright square
/// a stroll away. The square sits on a **real junction or the civic plaza**
/// (brief item 3), and its surrounding facades get wrapped. Scored by how much
/// commerce stands within a few tiles, with the downtown and the old core
/// preferred — the idea, not a copy of any one city.
fn anchors(
    objects: &[CityObj],
    huge: bool,
    cam: &Camera,
    frame_w: i32,
    frame_h: i32,
    map: Option<&TileMap>,
) -> ([Option<(f32, f32)>; 2], Option<(f32, f32)>, bool) {
    // Only the blocks the frame actually shows: the light show belongs where
    // the viewer is looking, and the map is larger than the frame.
    let width = frame_w;
    let mut lots: Vec<(f32, f32, f32)> = Vec::new(); // x, y, weight
    let mut plaza: Option<(f32, f32, f32)> = None; // centre + screen distance
    for o in objects {
        if o.stage != Stage::Finished {
            continue;
        }
        let (cx, cy) = (o.x as f32 + o.w as f32 * 0.5, o.y as f32 + o.h as f32 * 0.5);
        let p = cam.project(cx, cy, 0);
        let in_frame = p.0 >= 0.0 && p.0 <= width as f32 && p.1 >= 0.0 && p.1 <= frame_h as f32;
        // The civic square is open ground: it is the square's home if it is in
        // the picture (brief item 3).
        if o.kind == ObjKind::Landmark(LandmarkKind::Plaza) && in_frame {
            let d = ((p.0 - width as f32 * 0.5).abs() + (p.1 - frame_h as f32 * 0.5).abs()).abs();
            if plaza.map(|(_, _, bd)| d < bd).unwrap_or(true) {
                plaza = Some((cx, cy - 0.0, d));
            }
        }
        // The anchors stand on *commerce*: a block of homes is not a neon
        // quarter, however many of them there are (B73r2 note).
        if !signable(o.kind) || residential(o.kind) || !in_frame {
            continue;
        }
        let weight = match o.style {
            3 => 1.6,
            0 => 1.35,
            1 => 1.2,
            _ => 1.0,
        } * if commercial(o.kind) { 1.0 } else { 0.7 };
        lots.push((cx, cy, weight));
    }
    if lots.len() < 12 {
        return ([None; 2], None, false);
    }
    // Score on a 2-tile stride: cheap, and the peak of a block is stable.
    let mut scored: Vec<(f32, f32, f32, i64)> = Vec::new();
    for (i, &(x, y, _)) in lots.iter().enumerate() {
        if i % 2 == 1 {
            continue;
        }
        // Dense *and contiguous*: a block where the buildings touch is a street
        // wall (Mong Kok); the same number of shops spread over a suburb is not.
        let mut score = 0.0;
        for &(a, b, wt) in &lots {
            let d = (a - x).abs().max((b - y).abs());
            if d <= 5.0 {
                let near = if d <= 2.0 { 1.7 } else { 1.0 };
                score += wt * near * (1.0 - d / 7.0).max(0.25);
            }
        }
        scored.push((score, x, y, hash(x as i64 as u64 * 31 + y as i64 as u64) as i64));
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.3.cmp(&b.3)));
    let Some(&(best, bx, by, _)) = scored.first() else {
        return ([None; 2], None, false);
    };
    if best < 7.0 {
        return ([None; 2], None, false);
    }
    let neon1 = (bx, by);
    // A second neon block only in a very large town, away from the first.
    let mut neon2 = None;
    if huge {
        for &(score, x, y, _) in &scored {
            if score < 8.0 {
                break;
            }
            if (x - bx).abs().max((y - by).abs()) >= 14.0 {
                neon2 = Some((x, y));
                break;
            }
        }
    }
    // The bright square: the civic plaza if the town has one in frame with
    // facades around it, else the busiest junction — a crossing of real lanes —
    // at least 8 tiles off the neon quarter. That is what makes the "square" a
    // square (brief item 3).
    let commerce = |x: f32, y: f32, r: f32| -> f32 {
        lots.iter()
            .filter(|(a, b, _)| (a - x).abs().max((b - y).abs()) <= r)
            .map(|&(_, _, w)| w)
            .sum()
    };
    if let Some((px, py, _)) = plaza {
        if (px - bx).abs().max((py - by).abs()) >= 8.0 && commerce(px, py, SQUARE_R) >= 5.0 {
            return ([Some(neon1), neon2], Some((px, py)), true);
        }
    }
    // The road crossings near the commerce, strongest first, and one the camera
    // can actually *see*: a junction hidden behind the block in front of it
    // cannot be the city's bright square.
    let boxes = occluders(objects, cam);
    let visible = |x: f32, y: f32| seen(cam, map, &boxes, x, y);
    let plaza_ok = plaza.filter(|&(px, py, _)| {
        (px - bx).abs().max((py - by).abs()) >= 8.0 && commerce(px, py, SQUARE_R) >= 5.0
    });
    let mut square = None;
    let mut is_plaza = false;
    if let Some((px, py, _)) = plaza_ok {
        if visible(px, py) {
            return ([Some(neon1), neon2], Some((px, py)), true);
        }
    }
    {
        let mut cands = junctions(map, &lots, neon1, cam, frame_w, frame_h);
        cands.sort_by(|a, b| b.0.total_cmp(&a.0));
        for &(_, x, y) in cands.iter().take(24) {
            if commerce(x, y, SQUARE_R) >= 5.0 && visible(x, y) {
                square = Some((x, y));
                break;
            }
        }
    }
    if square.is_none() {
        // Nothing the camera can see through (a very young road net, or a block
        // with no street wide enough at this angle): the civc plaza, then the
        // strongest cluster, still beats no square at all.
        if let Some((px, py, _)) = plaza_ok {
            square = Some((px, py));
            is_plaza = true;
        }
    }
    if square.is_none() {
        for &(score, x, y, _) in &scored {
            if score < 6.0 {
                break;
            }
            if (x - bx).abs().max((y - by).abs()) >= 8.0 {
                square = Some((x, y));
                break;
            }
        }
    }
    ([Some(neon1), neon2], square, is_plaza)
}

/// The screen boxes of the standing buildings, with their painter diagonal: a
/// ground point is hidden by a building whose diagonal is nearer and whose
/// sprite box covers the point (B73r2: only a junction the viewer can see can be
/// the city's bright square).
fn occluders(objects: &[CityObj], cam: &Camera) -> Vec<(i32, Rect)> {
    let mut out = Vec::with_capacity(objects.len());
    for o in objects {
        if o.stage != Stage::Finished {
            continue;
        }
        // Open ground has no sprite: a park or the civic square hides nothing.
        if matches!(o.kind, ObjKind::Park | ObjKind::Landmark(LandmarkKind::Plaza)) {
            continue;
        }
        let base = o.level as i32 * HEIGHT_UNIT;
        let wall = o.wall_height();
        // A tile is 32 px wide: the near corner (+x, +y) is the right edge, the
        // far corner the left one.
        let left = cam.project(o.x as f32, (o.y + o.h) as f32, base).0;
        let right = cam.project((o.x + o.w) as f32, o.y as f32, base).0;
        let bottom = cam.project((o.x + o.w) as f32, (o.y + o.h) as f32, base).1;
        let top = cam.project(o.x as f32, o.y as f32, base + wall).1;
        out.push((
            o.diagonal(),
            Rect::new(
                left.round() as i32,
                top.round() as i32,
                right.round() as i32 + 1,
                bottom.round() as i32 + 1,
            ),
        ));
    }
    out
}

/// Is the ground at `(x, y)` in view from this camera? A point on the ground
/// plane is hidden only by a *nearer* building standing in front of it.
fn seen(cam: &Camera, map: Option<&TileMap>, boxes: &[(i32, Rect)], x: f32, y: f32) -> bool {
    let z = map
        .map(|m| (m.surface_level_at(x, y) * HEIGHT_UNIT as f32).round() as i32)
        .unwrap_or(0);
    let p = cam.project(x, y, z);
    let d = (p.1 - cam.oy as f32) / (map::iso_consts::HALF_H as f32);
    let (px, py) = (p.0.round() as i32, p.1.round() as i32);
    !boxes.iter().any(|&(bd, r)| {
        bd as f32 > d + 0.5 && px >= r.x0 && px < r.x1 && py >= r.y0 && py < r.y1
    })
}

/// Real junctions near the busiest blocks: lane tiles with three or four lane
/// neighbours, scored by how much commerce stands within six tiles and how
/// central they are in the frame, at least eight tiles from the neon quarter.
fn junctions(
    map: Option<&TileMap>,
    lots: &[(f32, f32, f32)],
    neon: (f32, f32),
    cam: &Camera,
    frame_w: i32,
    frame_h: i32,
) -> Vec<(f32, f32, f32)> {
    let Some(map) = map else { return Vec::new() };
    // Only the tiles the frame shows: the map is wider than the picture.
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(x, y, _) in lots {
        x0 = x0.min(x.floor() as i32 - 7);
        y0 = y0.min(y.floor() as i32 - 7);
        x1 = x1.max(x.ceil() as i32 + 7);
        y1 = y1.max(y.ceil() as i32 + 7);
    }
    if x0 > x1 || y0 > y1 {
        return Vec::new();
    }
    let x0 = x0.max(1);
    let y0 = y0.max(1);
    let x1 = x1.min(map.w - 2);
    let y1 = y1.min(map.h - 2);
    let mut out = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            if !map.at(x, y).kind.is_lane() {
                continue;
            }
            let mut arms = 0;
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                if map.at(x + dx, y + dy).kind.is_lane() {
                    arms += 1;
                }
            }
            if arms < 3 {
                continue;
            }
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            if (cx - neon.0).abs().max((cy - neon.1).abs()) < 8.0 {
                continue;
            }
            let p = cam.project(cx, cy, 0);
            if p.0 < 0.0 || p.0 > frame_w as f32 || p.1 < 0.0 || p.1 > frame_h as f32 {
                continue;
            }
            let mut score = 0.0;
            for &(a, b, wt) in lots {
                let d = (a - cx).abs().max((b - cy).abs());
                if d <= 6.0 {
                    score += wt * (1.0 - d / 8.0);
                }
            }
            // A four-way crossing beats a T, and the frame's middle a corner.
            score += arms as f32 * 0.6;
            score -= ((p.0 - frame_w as f32 * 0.5).abs() / frame_w as f32) * 2.0;
            out.push((score, cx, cy));
        }
    }
    out
}

// --- drawing ----------------------------------------------------------------

/// The light and clock a frame is drawn under.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// 0 = full day, 1 = full night (the scene's window glow).
    pub night: f32,
    /// The residents' clock, 0..24.
    pub hour: f32,
    /// Animation time in seconds.
    pub t: f32,
    /// Road wetness 0..1 (B42f: reflections on a wet night).
    pub wet: f32,
    pub land: Land,
}

fn lit_colour(s: &Spot, land: Land) -> Color {
    let p = s.character().palette();
    let (r, g, b) = p[(s.pal as usize) % p.len()];
    land.tint(rgb(r, g, b))
}



/// Draw every visible sign and beacon in `clip`. `plan` must have been
/// projected for the camera in force.
pub fn draw(plan: &Plan, fb: &mut Framebuffer, clip: Rect, look: &Look, depth: Option<&[u16]>) {
    if plan.spots.is_empty() && plan.vents.is_empty() && plan.pool.is_none() {
        return;
    }
    let b = fb.bounds();
    let (w, h) = (b.x1, b.y1);
    // The bright square first: the junction is lit like day, so its wash is the
    // base every panel around it sits in (D80, brief item 3).
    draw_pool(plan, fb, clip, look, depth, w, h);
    let mut drawn = 0usize;
    let mut blocked = 0usize;
    plan.for_each_visible(clip, |i| {
        let s = &plan.spots[i];
        let (x, y) = plan.screen(i);
        if let Some(d) = depth {
            if x >= 0 && y >= 0 && x < w && y < h && d[(y * w + x) as usize] > s.key {
                blocked += 1;
                return;
            }
        }
        drawn += 1;
        let ground = plan.spill_px.get(i).copied().unwrap_or((x, y));
        let mut pen = Pen { fb, depth, w, h, key: s.key, tol: 12 };
        draw_spot(&mut pen, s, x, y, ground, look, clip);
    });
    draw_steam(plan, fb, clip, look, w, h);
    if std::env::var("CITYLIVE_SIGN_DEBUG").is_ok() {
        let mut keys: Vec<(u8, u8, u8, u8, u8, u16)> = Vec::new();
        for (i, s) in plan.spots.iter().enumerate() {
            let (x, y) = plan.screen(i);
            if x < clip.x0 || x >= clip.x1 || y < clip.y0 || y >= clip.y1 {
                continue;
            }
            keys.push(s.design_key());
        }
        keys.sort_unstable();
        keys.dedup();
        eprintln!(
            "sign draw: {} spots, {} drawn ({} animated), {} blocked at the anchor, {} distinct designs on screen, {} vents, square {:?}, clip {:?}",
            plan.spots.len(),
            drawn,
            plan.spots.iter().enumerate().filter(|(i, s)| animated(s.flags) && plan.rect(*i).intersects(&clip)).count(),
            blocked,
            keys.len(),
            plan.vents.len(),
            plan.square,
            clip
        );
    }
    // Aviation lights: blinking red on the tallest, above everything else.
    for (i, b) in plan.beacons.iter().enumerate() {
        let Some(&(bx, by)) = plan.beacon_px.get(i) else { continue };
        if bx < clip.x0 - 2 || bx >= clip.x1 + 2 || by < clip.y0 - 2 || by >= clip.y1 + 2 {
            continue;
        }
        let blink = ((look.t * 0.85 + b.phase * 6.283).sin() * 0.5 + 0.5).powf(3.0);
        if look.night < 0.25 {
            continue;
        }
        if blink > 0.35 {
            fb.put(bx, by, rgb(0xff, 0x3a, 0x30));
            fb.put_blended(bx, by - 1, rgb(0xff, 0x3a, 0x30), 0.35);
            fb.put_blended(bx - 1, by, rgb(0xff, 0x3a, 0x30), 0.25);
        } else {
            fb.put(bx, by, rgb(0x5a, 0x1a, 0x18));
        }
    }
}

/// The bright square's pool: one dithered additive gradient over the junction,
/// big enough to wash the roads and the facades around it. Drawn under the
/// panels, and only at night.
fn draw_pool(
    plan: &Plan,
    fb: &mut Framebuffer,
    clip: Rect,
    look: &Look,
    depth: Option<&[u16]>,
    w: i32,
    h: i32,
) {
    let Some(p) = plan.pool else { return };
    let Some((cx, cy)) = plan.pool_px else { return };
    let lit = ((look.night - 0.22) / 0.55).clamp(0.0, 1.0);
    if lit <= 0.02 {
        return;
    }
    let r = p.radius.max(6.0) * 1.9;
    if cx as f32 + r < clip.x0 as f32
        || cx as f32 - r >= clip.x1 as f32
        || cy as f32 + r < clip.y0 as f32
        || cy as f32 - r >= clip.y1 as f32
    {
        return;
    }
    // A warm white core: the junction under its screens reads as day (D80).
    let mut pen = Pen { fb, depth, w, h, key: plan.pool_key, tol: 40 };
    // An ellipse on the ground plane (a tile is twice as wide as it is tall),
    // so the pool lies on the junction instead of hovering over it. The core
    // *lifts* the surfaces towards daylight (a source-over blend, which reads as
    // illumination) and the rim adds the screens' warmth on top of it.
    let litc = rgb(0xbc, 0xa8, 0x86);
    let warm = rgb(0xff, 0xd2, 0x9c);
    let rx = r as i32;
    let ry = (r * 0.5) as i32;
    for j in -ry..=ry {
        for i in -rx..=rx {
            let u = i as f32 / rx as f32;
            let v = j as f32 / ry.max(1) as f32;
            let d2 = u * u + v * v;
            if d2 >= 1.0 {
                continue;
            }
            let f = (1.0 - d2).max(0.0);
            // The junction is lit *like day*: a smooth wash (a stipple this
            // large reads as fog, not light), with a 2x2 dither of a quarter
            // step to stop the gradient banding on flat asphalt.
            let d2x = DITHER2[(j & 1) as usize][(i & 1) as usize];
            let lift = (0.62 * p.strength * lit * f.powf(0.75) + d2x).min(1.0);
            // Walls only below the junction's own line: those are the facades
            // facing it, at street level; above it is roof, and a lit roof would
            // read as fog hanging over the block.
            let wall = if j >= 0 { 0.34 } else { 0.0 };
            pen.lift_split(cx + i, cy + j, litc, lift, wall);
            let add = (0.20 * p.strength * lit * f.powf(1.6) + d2x * 0.5).min(1.0);
            pen.add_ground(cx + i, cy + j, warm, add);
        }
    }
}

/// The neon quarter's steam: a few soft puffs rising from the vents, drifting
/// with their own phase (D80; the crowd is B75's).
fn draw_steam(plan: &Plan, fb: &mut Framebuffer, clip: Rect, look: &Look, w: i32, h: i32) {
    if plan.vents.is_empty() || std::env::var("CITYLIVE_NO_STEAM").is_ok() {
        return;
    }
    let vis = ((look.night - 0.18) / 0.5).clamp(0.0, 1.0);
    if vis <= 0.02 {
        return;
    }
    for (i, v) in plan.vents.iter().enumerate() {
        let Some(&(x, y)) = plan.vent_px.get(i) else { continue };
        let r = Rect::new(x - 4, y - STEAM_RISE - 2, x + 5, y + 2);
        if !r.intersects(&clip) {
            continue;
        }
        let base = Character::Neon.palette()[(v.tint as usize) % Character::Neon.palette().len()];
        let tint = rgb(base.0, base.1, base.2);
        // Lit steam: the neon around it, pulled most of the way to grey-white.
        let c = palette::mix(tint, rgb(0xd8, 0xd4, 0xcc), 0.68);
        // Only the mouth of the vent needs a paint, and the depth plane has no
        // say in it (steam is translucent, and the vent is on the street).
        let mut pen = Pen { fb, depth: None, w, h, key: 0, tol: 12 };
        for k in 0..4 {
            let age = (look.t * 0.32 + v.phase + k as f32 * 0.25).rem_euclid(1.0);
            let yy = y - (age * STEAM_RISE as f32) as i32;
            let drift = ((age * 4.0 + k as f32 * 1.7).sin() * 1.6) as i32;
            let alpha = (1.0 - age) * 0.26 * vis * v.strength;
            let rad = 1.0 + age * 1.8;
            let ri = rad.ceil() as i32;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    let d2 = (dx * dx + dy * dy) as f32 / (rad * rad);
                    if d2 >= 1.0 {
                        continue;
                    }
                    let cov = alpha * (1.0 - d2);
                    dither_add(&mut pen, x + dx + drift, yy + dy, c, cov);
                }
            }
        }
    }
}

/// A 2x2 ordered dither, +-1/8: enough to break a gradient's banding without
/// turning a pool of light into a stipple.
const DITHER2: [[f32; 2]; 2] = [[0.0, 0.06], [0.09, 0.03]];

/// 4x4 Bayer matrix, /16: the ordered dither that keeps a soft light pool from
/// banding on a flat pavement.
const BAYER4: [[u8; 4]; 4] = [
    [0, 8, 2, 10],
    [12, 4, 14, 6],
    [3, 11, 1, 9],
    [15, 7, 13, 5],
];

/// Add a light pixel through the ordered dither: coverage quantised to three
/// steps, so the pool is a stipple of light rather than a flat wash.
#[inline]
fn dither_q(x: i32, y: i32, cov: f32) -> f32 {
    if cov <= 0.0 {
        return 0.0;
    }
    let t = BAYER4[(y & 3) as usize][(x & 3) as usize] as f32 / 16.0;
    let level = (cov * 4.0 + t).floor();
    if level < 1.0 {
        0.0
    } else {
        level.min(4.0) / 4.0
    }
}

#[inline]
fn dither_add(fb: &mut Pen, x: i32, y: i32, c: Color, cov: f32) {
    let q = dither_q(x, y, cov);
    if q > 0.0 {
        fb.add(x, y, c, q);
    }
}

/// The orderd dither, split by surface: ground takes the light, the walls take a
/// share of it (a pavement pool brightens a facade it touches, a facade does not
/// turn into a light box).
#[inline]
fn dither_spill(fb: &mut Pen, x: i32, y: i32, tint: Color, c: Color, cov: f32, wall: f32) {
    let q = dither_q(x, y, cov);
    if q <= 0.0 {
        return;
    }
    // The surface takes the colour (a blend), with the light itself added on top.
    fb.lift(x, y, tint, q * 0.75);
    fb.add(x, y, c, q * 0.55);
    let _ = wall;
}

/// Forms that are a glowing panel by night (letters dark on light) rather than
/// a tube or a painted board (light on dark).
fn is_panel(f: Form) -> bool {
    matches!(
        f,
        Form::WallBox | Form::Fascia | Form::Window | Form::Billboard | Form::Marquee | Form::Rooftop | Form::Screen
    )
}

/// The colours one sign is painted with at this hour.
struct Paint {
    body: Color,
    edge: Color,
    ink: Color,
    /// The lit colour itself (spill, bulbs, borders).
    glow: Color,
    lit: f32,
}

fn paint(s: &Spot, look: &Look) -> Paint {
    let land = look.land;
    let c = lit_colour(s, land);
    let mut lit = s.lit_at(look.night, look.hour);
    if s.flags & FLICKER != 0 && flickering(s, (look.t * ANIM_HZ) as i64) {
        lit *= 0.35;
    }
    // By day a sign is a painted board: the colour, dusted down and a little
    // grey, with dark lettering. By night the panel glows and the letters go dark.
    let grey = palette::rgb(0x8c, 0x86, 0x7c);
    let painted = palette::mix(palette::shade(c, 0.70), grey, 0.28);
    let painted_dark = palette::mix(palette::shade(c, 0.30), grey, 0.34);
    let f = s.form();
    let (body, edge, ink) = if is_panel(f) {
        (
            palette::mix(painted, palette::mix(c, rgb(0xff, 0xff, 0xff), 0.06), lit),
            palette::mix(painted_dark, palette::shade(c, 0.92), lit),
            palette::mix(painted_dark, palette::rgb(0x22, 0x18, 0x14), lit * 0.85),
        )
    } else {
        (
            // Tubes, cloth and iron: a dark board, the tube the bright thing.
            palette::mix(painted_dark, palette::shade(c, 0.34), lit),
            palette::mix(palette::shade(c, 0.34), palette::shade(c, 0.72), lit),
            palette::mix(palette::shade(c, 0.92), palette::mix(c, rgb(0xff, 0xff, 0xff), 0.15), lit),
        )
    };
    Paint { body, edge, ink, glow: c, lit }
}

/// A pen that knows the static layer's painter keys, so a sign is drawn only
/// where it stands in front of what the world already put there (a board halfway
/// up a facade may be behind the roof in front of it, and only its top shows).
pub struct Pen<'a> {
    fb: &'a mut Framebuffer,
    depth: Option<&'a [u16]>,
    w: i32,
    h: i32,
    key: u16,
    /// How many painter keys (two a tile) of the neighbourhood may take light:
    /// a sign's small pool vs the square's whole junction.
    tol: u16,
}

impl<'a> Pen<'a> {
    #[inline]
    fn blocked(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return true;
        }
        match self.depth {
            Some(d) => d[(y * self.w + x) as usize] > self.key,
            None => false,
        }
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, c: Color) {
        if !self.blocked(x, y) {
            self.fb.put(x, y, c);
        }
    }

    #[inline]
    fn put_blended(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        if !self.blocked(x, y) {
            self.fb.put_blended(x, y, c, cov);
        }
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, c);
            }
        }
    }

    fn fill_blended(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color, cov: f32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put_blended(xx, yy, c, cov);
            }
        }
    }

    /// Additive light on whatever surface is at the pixel: the pavement pool a
    /// sign throws in front of it, and the walls across the street it washes. The
    /// neighbourhood is three tiles either side (twelve painter keys), so light
    /// lands on the street canyon but never leaks through a block onto the sky or
    /// a row of buildings behind.
    #[inline]
    fn add(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        if cov <= 0.0 {
            return;
        }
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        if let Some(d) = self.depth {
            let dd = d[(y * self.w + x) as usize];
            if dd < self.key.saturating_sub(self.tol) || dd > self.key.saturating_add(self.tol) {
                return;
            }
        }
        self.fb.add_blended(x, y, c, cov);
    }

    /// Lift the neighbourhood towards a lit colour, the ground fully and the
    /// walls at `wall` of it (one pass: the square's daylight wash).
    #[inline]
    fn lift_split(&mut self, x: i32, y: i32, c: Color, cov: f32, wall: f32) {
        if cov <= 0.0 || x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let mut cov = cov;
        if let Some(d) = self.depth {
            let dd = d[(y * self.w + x) as usize];
            if dd < self.key.saturating_sub(self.tol) || dd > self.key.saturating_add(self.tol) {
                return;
            }
            if dd % 2 == 0 {
                cov *= wall;
            }
        }
        if cov > 0.002 {
            self.fb.put_blended(x, y, c, cov);
        }
    }

    /// Additive light only where the surface is **ground** (a tile: an odd
    /// painter key), within the neighbourhood: the pool a sign throws on the
    /// pavement, and the square's wash on the roads.
    #[inline]
    fn add_ground(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        if cov <= 0.0 || x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        if let Some(d) = self.depth {
            let dd = d[(y * self.w + x) as usize];
            if dd < self.key.saturating_sub(self.tol) || dd > self.key.saturating_add(self.tol) {
                return;
            }
            if dd % 2 == 0 {
                return;
            }
        }
        self.fb.add_blended(x, y, c, cov);
    }


    /// Lift a surface towards a lit colour (a source-over blend): what daylight
    /// does to asphalt under a bright square. Ground and walls both take it, in
    /// the same neighbourhood the glow reaches.
    #[inline]
    fn lift(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        if cov <= 0.0 || x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        if let Some(d) = self.depth {
            let dd = d[(y * self.w + x) as usize];
            if dd < self.key.saturating_sub(self.tol) || dd > self.key.saturating_add(self.tol) {
                return;
            }
        }
        self.fb.put_blended(x, y, c, cov);
    }

    /// Blend light into the wall the sign hangs on (`depth == key`): the facade's
    /// own wash, which must never smear into the sky behind the roof.
    #[inline]
    fn lift_on_wall(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        if cov <= 0.0 {
            return;
        }
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        if let Some(d) = self.depth {
            if d[(y * self.w + x) as usize] != self.key {
                return;
            }
        }
        self.fb.put_blended(x, y, c, cov);
    }
}

/// One sign, in screen space. `x, y` is the wall anchor (bottom edge, centre).
fn draw_spot(
    fb: &mut Pen,
    s: &Spot,
    x: i32,
    y: i32,
    ground: (i32, i32),
    look: &Look,
    clip: Rect,
) {
    let p = paint(s, look);
    let (w, h) = (s.w as i32, s.h as i32);
    let lit = p.lit;

    // The soft coloured pool first, so the panel and its letters sit in it
    // (D80, brief item 1): a neon street is washed in its own signs' colour.
    if lit > 0.05 {
        glow(fb, s, x, y, ground, &p, look, clip);
    }

    match s.form() {
        Form::Blade => {
            // An arm out from the wall, then the panel hanging from it.
            let px = x + 1;
            fb.put(px, y - h - 1, p.edge);
            fb.put(px + 1, y - h - 1, p.edge);
            panel(fb, px, y - h, w, h, p.body, p.edge);
            glyph_down(fb, s, px + 1, y - h + 1, h - 2, p.ink, lit);
        }
        Form::WallBox => {
            let (px, py, slope) = (x - w / 2, y - h, slope_of(s));
            slant_fill(fb, px, py, w, h, slope, p.edge);
            if w > 2 && h > 2 {
                slant_fill(fb, px + 1, py + 1, w - 2, h - 2, slope, p.body);
            }
            glyph_slanted(fb, s, px + 1, py + 1, w - 2, h - 2, slope, p.ink);
        }
        Form::Fascia => {
            // A band along the shopfront, following the facade's 2:1 slope.
            let slope = slope_of(s);
            for i in 0..w {
                let yy = y - 3 + (i - w / 2) * slope / 2;
                let px = x - w / 2 + i;
                if lit > 0.3 {
                    fb.put(px, yy, palette::mix(p.body, p.glow, 0.5 + 0.5 * lit));
                }
                fb.put(px, yy + 1, p.body);
                fb.put(px, yy + 2, p.edge);
            }
            glyph_slope(fb, s, x - w / 2 + 1, y - 2, w - 2, slope, p.ink, lit);
        }
        Form::Rooftop => {
            // Legs on the roof, the light box above it.
            let (px, py) = (x - w / 2, y - h);
            fb.put(px + 1, y, p.edge);
            fb.put(px + w - 2, y, p.edge);
            panel(fb, px, py, w, h, p.body, p.edge);
            glyph_row(fb, s, px + 1, py + 1, w - 2, h - 2, p.ink, lit);
        }
        Form::Billboard => {
            let (px, py, slope) = (x - w / 2, y - h, slope_of(s));
            slant_fill(fb, px, py, w, h, slope, p.edge);
            // Two painted halves, so the panel reads as a poster.
            let split = (w / 2).clamp(2, w - 2);
            let other = palette::mix(p.glow, palette::shade(p.glow, 0.42), 0.55);
            let a = if lit > 0.0 { p.glow } else { p.body };
            let b = if lit > 0.0 { other } else { palette::shade(p.body, 1.15) };
            slant_fill(fb, px + 1, py + 1, split - 1, h - 2, slope, a);
            slant_fill(fb, px + split, py + 1, w - split - 1, h - 2, slope, b);
            glyph_slanted(fb, s, px + 1, py + 1, split - 1, h - 2, slope, p.ink);
            glyph_slanted(fb, s, px + split, py + 1, w - split - 1, h - 2, slope, p.ink);
        }
        Form::Screen => {
            let (px, py, slope) = (x - w / 2, y - h, slope_of(s));
            slant_fill(fb, px, py, w, h, slope, p.edge);
            // The ad: colour fields and a band. Two thirds cut to a new one every
            // 8–15 s; a third *scroll* their ad sideways, one pixel a slot, which
            // is the finest step the pixel grid has and reads as a crawl
            // (brief item 6).
            let slot = (look.t * ANIM_HZ) as i64;
            let scroll = s.flags & SCROLL != 0;
            let ad = if scroll { slot / 90 } else { ad_index(s, slot) };
            let shift = if scroll { slot.rem_euclid(w.max(1) as i64) as i32 } else { 0 };
            let pal = s.character().palette();
            let np = pal.len();
            let pick = |k: usize| {
                let (r, g, b) = pal[(s.pal as usize + ad as usize * 2 + k) % np];
                look.land.tint(rgb(r, g, b))
            };
            let ca = if lit > 0.0 { pick(0) } else { p.body };
            let cb = if lit > 0.0 { pick(3) } else { palette::shade(p.body, 1.2) };
            let bands = 2 + ad.rem_euclid(2) as i32;
            for yy in 0..h - 2 {
                if scroll {
                    // A scrolling band: the two colours cross the panel, one
                    // pixel a slot.
                    for xx in 0..w - 2 {
                        let t = (xx + shift).rem_euclid((w - 2).max(1));
                        let cc = if t < (w - 2) / 2 { ca } else { cb };
                        let yy2 = py + 1 + yy + (xx - (w - 2) / 2) * slope / 2;
                        fb.put(px + 1 + xx, yy2, cc);
                    }
                } else {
                    let band = (yy * bands) / (h - 2).max(1);
                    let cc = if band % 2 == 0 { ca } else { cb };
                    slant_fill(fb, px + 1, py + 1 + yy, w - 2, 1, slope, cc);
                }
            }
            glyph_slanted(fb, s, px + 1, py + 1, w - 2, h - 2, slope, p.ink);
        }
        Form::Marquee => {
            let (px, py) = (x - w / 2, y - h);
            fb.fill_rect(px, py, w, h, p.edge);
            fb.fill_rect(px + 1, py + 1, w - 2, h - 2, p.body);
            glyph_row(fb, s, px + 1, py + 1, w - 2, h - 2, p.ink, lit);
            // Chaser bulbs around the border, one step per slot.
            if lit > 0.12 {
                let step = (look.t * ANIM_HZ) as i64;
                let bulb = palette::mix(p.glow, rgb(0xff, 0xff, 0xf0), 0.55);
                for i in 0..w {
                    let on = ((i as i64 + step) % 3) == 0;
                    let cc = if on { bulb } else { palette::shade(p.glow, 0.45) };
                    fb.put(px + i, py, cc);
                    fb.put(px + i, py + h - 1, cc);
                }
                for j in 0..h {
                    let on = ((j as i64 + step + 1) % 3) == 0;
                    let cc = if on { bulb } else { palette::shade(p.glow, 0.45) };
                    fb.put(px, py + j, cc);
                    fb.put(px + w - 1, py + j, cc);
                }
            }

        }
        Form::Bracket => {
            // An iron arm from the wall, a small sign hanging under it.
            let px = x + 1;
            fb.fill_rect(px, y - h - 3, 3, 1, palette::rgb(0x28, 0x26, 0x26));
            panel(fb, px, y - h - 2, w, h, p.body, p.edge);
            glyph_row(fb, s, px + 1, y - h - 1, w - 2, h - 2, p.ink, lit);
        }
        Form::Banner => {
            // A cloth across the street: two poles and a slack strip.
            let (px, py) = (x - w / 2, y - h);
            fb.fill_rect(px, py - 1, 1, h + 3, p.edge);
            fb.fill_rect(px + w - 1, py - 1, 1, h + 3, p.edge);
            for i in 0..w {
                let sag = ((i - w / 2) as f32 * 0.16).abs() as i32;
                fb.put(px + i, py + sag, p.body);
                fb.put(px + i, py + sag + 1, p.edge);
            }
            for i in 0..w - 2 {
                let sag = ((i as i32 + 1 - w / 2) as f32 * 0.16).abs() as i32;
                fb.put(px + 1 + i, py + sag, p.ink);
            }
        }
        Form::Window => {
            // A lit shopfront pane: dark frame, glass, a warm sill.
            let (px, py) = (x - w / 2, y - h);
            fb.fill_rect(px, py, w, h, p.edge);
            fb.fill_rect(px + 1, py + 1, w - 2, h - 2, p.body);
            if w > 4 {
                fb.put(px + w / 2, py + 1, p.edge);
                fb.put(px + w / 2, py + h - 2, p.edge);
            }
            glyph_row(fb, s, px + 1, py + 1, w - 2, h - 2, p.ink, lit * 0.85);
        }
        Form::String => {
            // A line of warm bulbs strung between two poles, sagging over the
            // street (an old high street; a desert lantern string, a tropical
            // bar string).
            let (px, py) = (x - w / 2, y);
            fb.fill_rect(px, py - 4, 1, 4, p.edge);
            fb.fill_rect(px + w - 1, py - 4, 1, 4, p.edge);
            for i in 0..w {
                let t = (i - w / 2) as f32 / w as f32;
                let sag = (t * t * 6.0) as i32;
                let yy = py - 4 + sag;
                // A thin wire, and a bulb every third pixel.
                fb.put(px + i, yy, palette::mix(p.edge, p.ink, 0.4));
                if i % 3 == 1 && lit > 0.1 {
                    let warm = palette::mix(p.glow, rgb(0xff, 0xe6, 0xb0), 0.35);
                    fb.put(px + i, yy + 1, warm);
                    fb.put_blended(px + i, yy + 2, warm, 0.22 * lit);
                }
            }
        }
        Form::Crown => {
            // A crown band along the roof edge, in the tower's own colour, with a
            // slow colour cycle on a few (D80: an occasional slow change).
            let (px, py) = (x - w / 2, y - h);
            let slow = if s.flags & CYCLE != 0 {
                let pal = s.character().palette();
                let step = ((look.t * 0.12) as i64 % pal.len() as i64) as usize;
                let (r, g, b) = pal[(s.pal as usize + step) % pal.len()];
                Some(look.land.tint(rgb(r, g, b)))
            } else {
                None
            };
            let cc = slow.unwrap_or(p.glow);
            let band = palette::mix(palette::shade(cc, 0.30), cc, lit.min(1.0));
            let hot = palette::mix(band, rgb(0xff, 0xff, 0xff), 0.25);
            for i in 0..w {
                fb.put(px + i, py, band);
                if lit > 0.2 {
                    // Two rows read as a crown at 1x; every other pixel is a hot
                    // bulb, so the band is a light, not a painted stripe.
                    fb.put(px + i, py - 1, if i % 2 == 0 { hot } else { band });
                    if i % 2 == 0 {
                        fb.put_blended(px + i, py + 1, cc, 0.30 * lit);
                    }
                }
            }
        }
    }
}

/// How much light a form throws: the big bright things (screens, billboards,
/// blades, marquees) wash the street; a window pane barely does.
fn glow_reach(s: &Spot) -> f32 {
    match s.form() {
        Form::Screen | Form::Billboard => 1.0,
        Form::Blade | Form::Marquee | Form::Fascia | Form::Rooftop => 0.82,
        Form::Crown => 0.62,
        Form::WallBox | Form::Bracket | Form::Banner | Form::String => 0.5,
        Form::Window => 0.34,
    }
}

/// A lit sign's coloured light on the pavement in front of it, the wall around
/// it, and (on a wet night) a smeared reflection down the road (B42f). Additive
/// and dithered, with a small radius, so a neon street is washed in colour but
/// no single pool reads as a flat disc.
fn glow(
    fb: &mut Pen,
    s: &Spot,
    x: i32,
    y: i32,
    ground: (i32, i32),
    p: &Paint,
    look: &Look,
    clip: Rect,
) {
    let a = p.lit * glow_reach(s);
    if a <= 0.05 {
        return;
    }
    let c = p.glow;
    let w = s.w as i32;
    let hgt = s.h as i32;
    // How far a form throws: the big bright panels light the street, a window
    // pane only its own sill.
    let (pool_r, wash, core) = match s.form() {
        Form::Screen | Form::Billboard | Form::Marquee => (14, 6, 0.72),
        Form::Blade | Form::Fascia | Form::Rooftop => (12, 7, 0.66),
        Form::WallBox | Form::Bracket | Form::Banner | Form::Crown => (8, 4, 0.52),
        Form::Window | Form::String => (5, 3, 0.34),
    };
    let pool_col = palette::shade(c, 0.66);
    let face_col = palette::mix(palette::shade(c, 0.82), c, 0.30);
    // --- the pavement pool, in front of the wall the sign hangs on -----------
    // (A damaged rectangle only pays for the part of the glow it can see: this
    // is what keeps a night city's per-frame cost at a few small boxes.)
    let r = (pool_r + w / 8).clamp(pool_r - 2, pool_r + 4);
    let pool_box = Rect::new(
        ground.0 - r - 2,
        ground.1 - 1,
        ground.0 + r + 2,
        ground.1 + r + 20,
    );
    if clip.intersects(&pool_box) {
    for j in 0..=r {
        let t = j as f32 / r as f32;
        let half = ((1.0 - t * t * 0.55) * r as f32) as i32;
        let cov = core * a * (1.0 - t).powf(1.3);
        if cov < 0.03 {
            continue;
        }
        for i in -half..=half {
            let edge = 1.0 - (i.abs() as f32 / (half.max(1) as f32 + 0.6)) * 0.45;
            dither_spill(fb, ground.0 + i, ground.1 + j, pool_col, c, cov * edge, 0.4);
        }
    }
    }
    // --- the wall's own wash, all round the panel ----------------------------
    // A facade lit by its sign: the wall's own texture *takes the colour* (a
    // blend) with a hot additive core, falling off over a few pixels, so a stack
    // of blades reads as a wall of light at 1x.
    let cy = y - hgt / 2;
    let wash_box = Rect::new(
        x - w / 2 - wash - 1,
        cy - hgt / 2 - wash - 1,
        x + w / 2 + wash + 2,
        cy + hgt / 2 + wash + 2,
    );
    if clip.intersects(&wash_box) {
    for dy in -(hgt / 2 + wash)..=(hgt / 2 + wash) {
        let t = (dy as f32 / (hgt as f32 / 2.0 + wash as f32)).abs();
        if t > 1.0 {
            continue;
        }
        for dx in -(w / 2 + wash)..=(w / 2 + wash) {
            // The panel itself is drawn over this: no light under it.
            if dx.abs() < w / 2 - 1 && dy.abs() < hgt / 2 - 1 {
                continue;
            }
            let u = (dx as f32 / (w as f32 / 2.0 + wash as f32)).abs();
            let f = (1.0 - t * t) * (1.0 - u * u);
            if f <= 0.10 {
                continue;
            }
            let q = dither_q(x + dx, cy + dy, 0.72 * a * f);
            if q <= 0.0 {
                continue;
            }
            fb.lift_on_wall(x + dx, cy + dy, face_col, q);
        }
    }
    }
    // --- a wet road holds the sign: a vertical smear below it ----------------
    if look.wet > 0.18 && clip.intersects(&pool_box) {
        let len = (4.0 + look.wet * 11.0) as i32;
        for j in 0..len {
            let cov = 0.30 * look.wet * a * (1.0 - j as f32 / len as f32);
            let half = (w / 6).clamp(0, 4);
            for i in -half..=half {
                let edge = 1.0 - (i.abs() as f32 / (half as f32 + 1.0)) * 0.5;
                fb.add(ground.0 + i, ground.1 + 3 + j, c, cov * edge);
            }
        }
    }
}


/// A bordered panel: 1 px edge, the body inside.
fn panel(fb: &mut Pen, x: i32, y: i32, w: i32, h: i32, body: Color, edge: Color) {
    if w <= 0 || h <= 0 {
        return;
    }
    fb.fill_rect(x, y, w, h, edge);
    if w > 2 && h > 2 {
        fb.fill_rect(x + 1, y + 1, w - 2, h - 2, body);
    }
}

/// Which way a wall-mounted panel slants: the +x wall recedes to the right and
/// down, the +y wall to the right and up (2:1 iso).
fn slope_of(s: &Spot) -> i32 {
    if s.face == 0 {
        1
    } else {
        -1
    }
}

/// Fill a box that follows a wall's 2:1 slope: each column drops (or rises) by
/// half a pixel per pixel of its width, so the panel sits on the facade instead
/// of floating in front of it.
fn slant_fill(fb: &mut Pen, x: i32, y: i32, w: i32, h: i32, slope: i32, c: Color) {
    if w <= 0 || h <= 0 {
        return;
    }
    for i in 0..w {
        let dy = (i - w / 2) * slope / 2;
        fb.fill_rect(x + i, y + dy, 1, h, c);
    }
}

/// Glyphs on a slanted panel: 3x5 cells when there is room, a lettering line
/// otherwise, every pixel shifted with its column.
fn glyph_slanted(
    fb: &mut Pen,
    s: &Spot,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    slope: i32,
    ink: Color,
) {
    if w < 2 || h < 1 {
        return;
    }
    let dy = |col: i32| (col - w / 2) * slope / 2;
    if h >= 6 && w >= 4 {
        let cells = (w / 4).max(1);
        for i in 0..cells {
            let g = GLYPHS[(s.glyph as usize >> ((i * 3) % 16)) % GLYPHS.len()];
            let bx = x + i * 4;
            for r in 0..5 {
                for c in 0..3 {
                    if g & (1 << (14 - (r * 3 + c))) != 0 && bx + c < x + w {
                        fb.put(bx + c, y + r + dy(bx + c - x), ink);
                    }
                }
            }
        }
        return;
    }
    let yy = y + (h - 1).max(0) / 2;
    let mut i = 0;
    let mut bit = 0;
    while i < w {
        let m = (s.glyph >> (bit % 24)) as i32 & 0x3;
        let len = 1 + (m & 1);
        for k in 0..len.min(w - i) {
            let xx = x + i + k;
            fb.put(xx, yy + dy(xx - x), ink);
        }
        i += len + 1;
        bit += 2;
    }
}

/// A row of invented glyphs inside a `w` x `h` box: 3x5 cells when there is room,
/// a dotted lettering line when there is not (at 1× both read as text).
fn glyph_row(fb: &mut Pen, s: &Spot, x: i32, y: i32, w: i32, h: i32, ink: Color, lit: f32) {
    if w < 2 || h < 1 {
        return;
    }
    if h >= 5 && w >= 4 {
        let cells = (w / 4).max(1);
        for i in 0..cells {
            let g = GLYPHS[(s.glyph as usize >> ((i * 3) % 16)) % GLYPHS.len()];
            let bx = x + i * 4;
            for r in 0..5 {
                for c in 0..3 {
                    if g & (1 << (14 - (r * 3 + c))) != 0 && bx + c < x + w {
                        fb.put(bx + c, y + r, ink);
                    }
                }
            }
        }
        return;
    }
    // A single lettering line: marks with gaps, never a plain bar.
    let yy = y + h / 2;
    let mut i = 0;
    let mut bit = 0;
    while i < w {
        let m = (s.glyph >> (bit % 24)) as i32 & 0x3;
        let len = 1 + (m & 1);
        for k in 0..len.min(w - i) {
            fb.put(x + i + k, yy, ink);
        }
        i += len + 1;
        bit += 2;
    }
    let _ = lit;
}

/// Glyphs down a narrow blade: 2 px tube marks, so the strip reads as lettering
/// standing out from the board.
fn glyph_down(fb: &mut Pen, s: &Spot, x: i32, y: i32, h: i32, ink: Color, lit: f32) {
    if h < 3 {
        return;
    }
    let w = 2;
    for j in 0..(h / 4).max(1) {
        let m = ((s.glyph >> (j * 2)) as i32) & 0x3;
        let yy = y + j * 4;
        match m {
            0 => fb.fill_rect(x, yy, w, 2, ink),
            1 => fb.fill_rect(x, yy, 1, 2, ink),
            2 => fb.fill_rect(x + 1, yy, 1, 2, ink),
            _ => {
                fb.fill_rect(x, yy, w, 1, ink);
                fb.put(x, yy + 2, ink);
            }
        }
    }
    if lit > 0.4 {
        // A faint tube glow either side of the board.
        fb.put_blended(x - 1, y + h / 2, ink, 0.20);
        fb.put_blended(x + w, y + h / 2, ink, 0.20);
    }
}

/// Glyphs along a sloped fascia band.
fn glyph_slope(fb: &mut Pen, s: &Spot, x: i32, y: i32, w: i32, slope: i32, ink: Color, lit: f32) {
    if w < 2 {
        return;
    }
    let mut i = 0;
    let mut bit = 0;
    while i < w {
        let m = (s.glyph >> (bit % 24)) as i32 & 0x3;
        let len = 1 + (m & 1);
        for k in 0..len.min(w - i) {
            let xx = x + i + k;
            fb.put(xx, y + (xx - x - w / 2) * slope / 2, ink);
        }
        i += len + 1;
        bit += 2;
    }
    let _ = lit;
}

/// Coloured light on the pavement under a bright sign, and on a wet night a
/// vertical reflection smeared down the road (B42f).
#[allow(dead_code)]
fn spill(fb: &mut Pen, c: Color, x: i32, y: i32, wet: f32) {
    fb.fill_blended(x - 6, y, 12, 1, c, 0.42);
    fb.fill_blended(x - 5, y + 1, 10, 1, c, 0.26);
    fb.fill_blended(x - 4, y + 2, 8, 1, c, 0.15);
    fb.fill_blended(x - 2, y + 3, 4, 1, c, 0.08);
    if wet > 0.18 {
        let len = (2.0 + wet * 7.0) as i32;
        for j in 0..len {
            let cov = 0.18 * wet * (1.0 - j as f32 / len as f32);
            fb.put_blended(x - 1, y + 3 + j, c, cov);
            fb.put_blended(x, y + 3 + j, c, cov * 1.25);
            fb.put_blended(x + 1, y + 3 + j, c, cov * 0.7);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iso::Camera;

    fn lot(x: i32, y: i32, kind: ObjKind, style: u8, seed: u32) -> CityObj {
        CityObj {
            x,
            y,
            w: 2,
            h: 2,
            level: 2,
            kind,
            storeys: 6,
            style,
            stage: Stage::Finished,
            seed,
            front: 1,
            t: 0.0,
            kit: 0, wear: Default::default(),
        }
    }

    fn big_city() -> Vec<CityObj> {
        let mut v = Vec::new();
        for y in 0i32..22 {
            for x in 0i32..22 {
                let style = if (x - 10).abs() + (y - 10).abs() < 5 { 3 } else { 0 };
                v.push(lot(x, y, ObjKind::Shop, style, (x * 97 + y * 31) as u32));
            }
        }
        // The town-size gate reads storeys too.
        for o in &mut v {
            o.storeys = 12;
        }
        v
    }

    fn cam() -> Camera {
        Camera::new(400, 300)
    }

    /// The whole test city inside a big frame: the anchors only consider blocks
    /// the viewer can see, and `cam()` shows an eighth of it.
    fn wide_cam() -> Camera {
        Camera::new(500, 100)
    }

    #[test]
    fn no_two_neighbouring_signs_are_the_same() {
        let objs = big_city();
        let plan = build(&objs, &[], &cam(), Land::Temperate, 800, 400);
        assert!(plan.spots.len() > 40, "the city should have signs");
        // Every sign is 2x2 lots apart at most: any two within 2 tiles of each
        // other must differ in form or palette.
        for a in &plan.spots {
            for b in &plan.spots {
                if a.lot == b.lot {
                    continue;
                }
                if (a.wx - b.wx).abs() <= 1.3 && (a.wy - b.wy).abs() <= 1.3 {
                    assert!(
                        a.form != b.form || a.pal != b.pal || a.character != b.character,
                        "two neighbours share form {} and palette {}",
                        a.form,
                        a.pal
                    );
                }
            }
        }
    }

    #[test]
    fn a_village_has_no_neon_and_a_big_town_does() {
        let mut small: Vec<CityObj> = (0..30)
            .map(|i| lot(i * 3, 4, ObjKind::Shop, 0, i as u32))
            .collect();
        for o in &mut small {
            o.storeys = 2;
        }
        let plan = build(&small, &[], &cam(), Land::Temperate, 800, 400);
        assert!(plan.chars[Character::Neon.index()] == 0);
        assert!(plan.chars[Character::Square.index()] == 0);

        // A big town means people: 15 000 of them, not 484 lots (B73r2 note).
        let city = TownStats { population: 15_000, era: 4 };
        let plan = build_in(&big_city(), &[], &cam(), Land::Temperate, 800, 400, city, None);
        assert!(plan.chars[Character::Neon.index()] > 0, "a big town gets a neon quarter");
    }

    #[test]
    fn a_shop_is_dark_at_three_and_the_all_night_diner_is_not() {
        let shop = Spot {
            wx: 0.0,
            wy: 0.0,
            z: 10.0,
            w: 4,
            h: 2,
            form: Form::WallBox as u8,
            character: Character::HighStreet as u8,
            pal: 0,
            open: 9.0,
            close: 21.0,
            flags: 0,
            glyph: 1,
            lot: 1,
            face: 0,
            key: 0,
        };
        assert!(shop.open_at(20.0));
        assert!(!shop.open_at(3.0));
        assert_eq!(shop.lit_at(1.0, 3.0), 0.0, "a closed shop is off");

        let diner = Spot {
            open: 0.0,
            close: 24.0,
            flags: ALL_NIGHT,
            ..shop
        };
        assert!(diner.open_at(3.0));
        assert!(diner.lit_at(1.0, 3.0) > 0.9, "the diner stays lit");
    }

    #[test]
    fn a_sign_is_painted_by_day_and_lit_at_night() {
        let s = Spot {
            wx: 0.0,
            wy: 0.0,
            z: 0.0,
            w: 5,
            h: 3,
            form: Form::WallBox as u8,
            character: Character::Neon as u8,
            pal: 1,
            open: 17.0,
            close: 26.0,
            flags: 0,
            glyph: 0x1234,
            lot: 2,
            face: 0,
            key: 0,
        };
        let look_day = Look { night: 0.0, hour: 12.0, t: 0.0, wet: 0.0, land: Land::Temperate };
        let look_night = Look { night: 1.0, hour: 20.0, t: 0.0, wet: 0.0, land: Land::Temperate };
        let mut a = Framebuffer::new(24, 24);
        let mut b = Framebuffer::new(24, 24);
        let bounds = a.bounds();
        let bounds_b = b.bounds();
        let spot = s;
        let plan_a = Plan {
            spots: vec![spot],
            ..Default::default()
        };
        let mut plan_a = plan_a;
        plan_a.project(&Camera::new(12, 18), 24);
        let mut plan_b = plan_a.clone();
        draw(&plan_a, &mut a, bounds, &look_day, None);
        draw(&mut plan_b, &mut b, bounds_b, &look_night, None);
        let bright = |fb: &Framebuffer| {
            fb.px().iter().filter(|&&p| p != 0).map(|&p| {
                palette::r(p) as u32 + palette::g(p) as u32 + palette::b(p) as u32
            }).max().unwrap_or(0)
        };
        assert!(bright(&a) > 0, "the panel exists by day");
        assert!(bright(&b) > bright(&a) + 60, "and it is much brighter at night");
    }

    #[test]
    fn glyphs_are_invented_and_never_ascii() {
        // No glyph maps into ASCII, so no real word or brand can be spelled.
        for g in GLYPHS {
            assert!(rune(g) as u32 >= 0xE000, "a glyph must not be an ASCII letter");
        }
        // And they are more than a handful of shapes.
        let mut seen = GLYPHS.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert!(seen.len() >= 12, "enough distinct glyph shapes: {}", seen.len());
    }

    /// D75: the glyphs are invented. A sign's runes never fall in ASCII, so no
    /// real word — least of all a brand from this list — can be spelled.
    #[test]
    fn no_sign_spells_a_banned_word() {
        const BANNED: [&str; 16] = [
            "coca", "cola", "pepsi", "nike", "sony", "apple", "google", "ford", "toyota",
            "mcdonald", "starbucks", "times square", "kabukicho", "dotonbori", "mongkok",
            "hollywood",
        ];
        let plan = build(&big_city(), &[], &cam(), Land::Temperate, 800, 400);
        for s in &plan.spots {
            let glyphs = 2 + (s.glyph % 4);
            let mut text = String::new();
            for i in 0..glyphs {
                let g = GLYPHS[(s.glyph as usize >> (i * 2)) % GLYPHS.len()];
                text.push(rune(g));
            }
            assert!(!text.is_ascii(), "a sign decoded to ASCII: {text:?}");
            for word in BANNED {
                assert!(
                    !text.to_lowercase().contains(word),
                    "a sign spells {word:?}"
                );
            }
        }
    }

    /// D80/brief item 1: the quarter and the square are gated on the sim's own
    /// town (population and era), not on how many lots a frame happens to hold.
    #[test]
    fn a_quarter_is_gated_on_people_not_lots() {
        let objs = big_city();
        let small = TownStats { population: 300, era: 1 };
        let plan = build_in(&objs, &[], &wide_cam(), Land::Temperate, 1200, 900, small, None);
        assert_eq!(plan.chars[Character::Neon.index()], 0, "a village gets no neon quarter");
        assert_eq!(plan.chars[Character::Square.index()], 0, "and no bright square");
        assert!(plan.pool.is_none());

        let big = TownStats { population: 9_000, era: 4 };
        let plan = build_in(&objs, &[], &wide_cam(), Land::Temperate, 1200, 900, big, None);
        assert!(plan.chars[Character::Neon.index()] > 0, "the same lots, a real city");
        assert!(plan.chars[Character::Square.index()] > 0);
    }

    /// B73r2 note: the day-25 world (6 200 people) came back with a neon quarter
    /// on its streets. A city that size is not a Mong Kok.
    #[test]
    fn a_small_city_has_no_neon_quarter() {
        let objs = big_city();
        let day25 = TownStats { population: 6_200, era: 3 };
        let plan = build_in(&objs, &[], &wide_cam(), Land::Temperate, 1200, 900, day25, None);
        assert_eq!(plan.chars[Character::Neon.index()], 0, "no neon quarter in a small city");
        assert_eq!(plan.chars[Character::Square.index()], 0, "and no bright square");
        assert!(plan.pool.is_none());
        assert!(plan.vents.is_empty(), "and no steam");

        // A metropolis wears them.
        let big = TownStats { population: 15_400, era: 4 };
        let plan = build_in(&objs, &[], &wide_cam(), Land::Temperate, 1200, 900, big, None);
        assert!(plan.chars[Character::Neon.index()] > 0);
        assert!(plan.chars[Character::Square.index()] > 0);
    }

    /// B73r2 note: a residential lot advertises nothing — never a neon blade, and
    /// never a screen or billboard, however big the city is.
    #[test]
    fn a_home_never_wears_neon() {
        // A dense, tall district made *entirely* of homes, in a metropolis: the
        // quarter's anchor would sit right on top of it.
        let mut objs = Vec::new();
        for y in 0i32..20 {
            for x in 0i32..20 {
                let kind = if (x + y) % 2 == 0 { ObjKind::Townhouse } else { ObjKind::Apartment };
                let mut o = lot(x, y, kind, 0, (x * 131 + y * 47) as u32);
                o.storeys = 6;
                objs.push(o);
            }
        }
        let stats = TownStats { population: 20_000, era: 4 };
        let plan = build_in(&objs, &[], &wide_cam(), Land::Temperate, 1200, 900, stats, None);
        for s in &plan.spots {
            assert!(
                !matches!(s.character(), Character::Neon | Character::Square),
                "a home at lot {} wears {}",
                s.lot,
                s.character().name()
            );
            assert!(
                !matches!(s.form(), Form::Blade | Form::Screen | Form::Billboard | Form::Marquee),
                "a home wears a {}",
                s.form().name()
            );
        }
    }

    /// Brief item 3: the bright square sits on the civic plaza when the town has
    /// one in view.
    #[test]
    fn the_square_sits_on_the_civic_plaza() {
        let mut objs = big_city();
        // A plaza in the middle of the south-east quadrant.
        objs.push(CityObj {
            x: 17,
            y: 1,
            w: 5,
            h: 5,
            kind: ObjKind::Landmark(LandmarkKind::Plaza),
            style: 0,
            storeys: 0,
            ..lot(17, 1, ObjKind::Landmark(LandmarkKind::Plaza), 0, 77)
        });
        let stats = TownStats { population: 9_000, era: 4 };
        let plan = build_in(&objs, &[], &wide_cam(), Land::Temperate, 1200, 900, stats, None);
        let (px, py) = plan.square.expect("a square");
        assert!(plan.square_is_plaza, "it should be the plaza");
        assert!((px - 19.5).abs() < 1.6 && (py - 3.5).abs() < 1.6, "on the plaza: {px},{py}");
    }

    /// Brief item 6: a screen dims after 01:00 and is dark from 02:00.
    #[test]
    fn screens_dim_after_one_and_go_dark_at_two() {
        assert_eq!(screen_dim(0.5), 1.0);
        assert_eq!(screen_dim(1.0), 1.0);
        assert!((screen_dim(1.5) - 0.5).abs() < 0.01);
        assert_eq!(screen_dim(2.0), 0.0);
        assert_eq!(screen_dim(3.0), 0.0);
        let mut s = Spot {
            wx: 0.0,
            wy: 0.0,
            z: 10.0,
            w: 20,
            h: 10,
            form: Form::Screen as u8,
            character: Character::Square as u8,
            pal: 0,
            open: 0.0,
            close: 26.0,
            flags: SCREEN | ALL_NIGHT,
            glyph: 7,
            lot: 3,
            face: 0,
            key: 0,
        };
        assert!(s.lit_at(1.0, 0.5) > 0.9, "a screen is on at half past midnight");
        assert!(s.lit_at(1.0, 1.5) < 0.6, "it is dimming at 01:30");
        assert_eq!(s.lit_at(1.0, 2.5), 0.0, "and dark at 02:30");
        // Any other form keeps its own hours.
        s.form = Form::Blade as u8;
        s.flags = ALL_NIGHT;
        assert!(s.lit_at(1.0, 2.5) > 0.9);
    }

    /// Brief item 6: a third of the square's screens scroll rather than cut.
    #[test]
    fn a_third_of_the_square_screens_scroll() {
        let stats = TownStats { population: 9_000, era: 4 };
        let plan = build_in(&big_city(), &[], &wide_cam(), Land::Temperate, 1200, 900, stats, None);
        let screens: Vec<&Spot> = plan
            .spots
            .iter()
            .filter(|s| s.form() == Form::Screen && s.character() == Character::Square)
            .collect();
        assert!(!screens.is_empty(), "the square has screens");
        let scroll = screens.iter().filter(|s| s.flags & SCROLL != 0).count();
        let share = scroll as f32 / screens.len() as f32;
        assert!((0.1..0.6).contains(&share), "scroll share {share}");
    }

    /// Brief item 7: every sign reflects on a wet road, not just the billboards.
    #[test]
    fn every_sign_reflects_on_a_wet_night() {
        let stats = TownStats { population: 9_000, era: 4 };
        let plan = build_in(&big_city(), &[], &cam(), Land::Temperate, 800, 400, stats, None);
        assert!(!plan.spots.is_empty());
        assert!(plan.spots.iter().all(|s| s.flags & SPILL != 0));
    }

    /// Brief item 1: a lit sign throws light on what is around it — the pavement
    /// in front and the wall it hangs on.
    #[test]
    fn a_lit_sign_lights_the_wall_and_pavement_around_it() {
        let s = Spot {
            wx: 5.0,
            wy: 4.0,
            z: 24.0,
            w: 6,
            h: 20,
            form: Form::Blade as u8,
            character: Character::Neon as u8,
            pal: 0,
            open: 16.5,
            close: 27.0,
            flags: SPILL,
            glyph: 5,
            lot: 1,
            face: 0,
            key: 40,
        };
        let build = |night: f32| {
            let mut plan = Plan {
                spots: vec![s],
                ..Default::default()
            };
            plan.project(&Camera::new(20, -60), 200);
            let mut fb = Framebuffer::new(200, 200);
            let look = Look {
                night,
                hour: 20.0,
                t: 0.0,
                wet: 0.0,
                land: Land::Temperate,
            };
            let clip = fb.bounds();
            draw(&plan, &mut fb, clip, &look, None);
            (fb, plan.screen(0), plan.spill_px[0])
        };
        let (day, (x, y), ground) = build(0.0);
        let (night, _, _) = build(1.0);
        let lum = |fb: &Framebuffer, x: i32, y: i32| {
            let p = fb.px()[fb.index(x, y)];
            palette::r(p) as u32 + palette::g(p) as u32 + palette::b(p) as u32
        };
        // The pavement just below the sign's wall anchor.
        let day_ground: u32 = (-3..=3).map(|i| lum(&day, ground.0 + i, ground.1 + 2)).sum();
        let lit_ground: u32 = (-3..=3).map(|i| lum(&night, ground.0 + i, ground.1 + 2)).sum();
        assert!(
            lit_ground > day_ground + 40,
            "the pavement takes the sign's light: {day_ground} -> {lit_ground}"
        );
        // And the wall beside the panel.
        let day_wall = lum(&day, x + 6, y - 10);
        let lit_wall = lum(&night, x + 6, y - 10);
        assert!(lit_wall > day_wall, "the wall takes it too: {day_wall} -> {lit_wall}");
    }

    /// D80: the steam belongs to the neon quarter and nowhere else.
    #[test]
    fn steam_rises_only_in_the_quarter() {
        let stats = TownStats { population: 9_000, era: 4 };
        let plan = build_in(&big_city(), &[], &wide_cam(), Land::Temperate, 1200, 900, stats, None);
        assert!(plan.vents.len() <= 4, "a handful of vents: {}", plan.vents.len());
        assert!(!plan.vents.is_empty(), "the quarter breathes");
        // Each vent stands within the quarter's own radius of an anchor.
        for v in &plan.vents {
            let near = plan
                .anchors
                .iter()
                .flatten()
                .any(|a| (v.wx - a.0).abs().max((v.wy - a.1).abs()) <= NEON_R);
            assert!(near, "a vent outside the quarter at {:?}", (v.wx, v.wy));
        }
    }

    #[test]
    fn a_town_with_neighbours_reports_many_designs() {
        let plan = build(&big_city(), &[], &cam(), Land::Temperate, 800, 400);
        assert!(plan.designs > 20, "distinct designs {} too few", plan.designs);
    }
}

