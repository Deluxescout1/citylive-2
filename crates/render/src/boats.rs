//! The boat kit (B91): seventeen kinds of boat, each modelled once as a small
//! voxel hull-and-rig, then ray-cast into the four isometric headings, by day
//! and by night, in eight variants (paint, trim, sail, wear).
//!
//! Like the car kit (B75) the sprites are built once, lazily per kind, and
//! shared by every scene (`kit`). A model is a few thousand coloured voxels:
//! hull, deck, cabins, sails; thin things (masts, stays, oars, antennas) are 3-D
//! lines rasterised into the same picture, depth-tested against the voxels.
//!
//! Local axes: `l` runs stern (0) to bow (nl-1), `w` across (+ is starboard,
//! the right-hand side looking forward), `z` up from the waterline.
//! Heading 0 is +x on the tile grid, 1 is +y, 2 is -x, 3 is -y.

use std::sync::OnceLock;

use crate::framebuffer::{Framebuffer, Sprite};
use crate::palette::{self, rgb, shade, Color};

/// Every kind in the kit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoatKind {
    Launch,
    Rowboat,
    Dinghy,
    Sailboat,
    Sloop,
    MotorYacht,
    Fishing,
    Lobster,
    Speedboat,
    Kayak,
    Pontoon,
    Tug,
    Barge,
    Ferry,
    CarFerry,
    Coaster,
    Container,
}

pub const KINDS: usize = 17;
pub const VARIANTS: usize = 8;

/// The small-craft index list the sim uses in `Mover::set` (low five bits).
pub const SMALL: [BoatKind; 13] = [
    BoatKind::Launch,
    BoatKind::Rowboat,
    BoatKind::Dinghy,
    BoatKind::Sailboat,
    BoatKind::Sloop,
    BoatKind::MotorYacht,
    BoatKind::Fishing,
    BoatKind::Lobster,
    BoatKind::Speedboat,
    BoatKind::Kayak,
    BoatKind::Pontoon,
    BoatKind::Tug,
    BoatKind::Barge,
];

impl BoatKind {
    pub const ALL: [BoatKind; KINDS] = [
        BoatKind::Launch,
        BoatKind::Rowboat,
        BoatKind::Dinghy,
        BoatKind::Sailboat,
        BoatKind::Sloop,
        BoatKind::MotorYacht,
        BoatKind::Fishing,
        BoatKind::Lobster,
        BoatKind::Speedboat,
        BoatKind::Kayak,
        BoatKind::Pontoon,
        BoatKind::Tug,
        BoatKind::Barge,
        BoatKind::Ferry,
        BoatKind::CarFerry,
        BoatKind::Coaster,
        BoatKind::Container,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn name(self) -> &'static str {
        match self {
            BoatKind::Launch => "water taxi",
            BoatKind::Rowboat => "rowing boat",
            BoatKind::Dinghy => "dinghy",
            BoatKind::Sailboat => "sailboat",
            BoatKind::Sloop => "sloop yacht",
            BoatKind::MotorYacht => "motor yacht",
            BoatKind::Fishing => "fishing boat",
            BoatKind::Lobster => "lobster boat",
            BoatKind::Speedboat => "speedboat",
            BoatKind::Kayak => "kayak",
            BoatKind::Pontoon => "houseboat",
            BoatKind::Tug => "tug",
            BoatKind::Barge => "barge",
            BoatKind::Ferry => "passenger ferry",
            BoatKind::CarFerry => "car ferry",
            BoatKind::Coaster => "coaster",
            BoatKind::Container => "container ship",
        }
    }

    /// From a mover's `set` byte (sim::rail::Mover::set): the low five bits
    /// pick the kind, the top three the variant. `class` is what the sim says
    /// the mover is, so an old save with `set == 0` still gets a sensible hull.
    pub fn decode(class: Class, set: u8) -> (BoatKind, u8) {
        let k = (set & 31) as usize;
        let v = set >> 5;
        let kind = match class {
            Class::Boat => SMALL[k % SMALL.len()],
            Class::Ferry => {
                if k % 2 == 0 {
                    BoatKind::Ferry
                } else {
                    BoatKind::CarFerry
                }
            }
            Class::Ship => {
                if k % 2 == 0 {
                    BoatKind::Container
                } else {
                    BoatKind::Coaster
                }
            }
        };
        (kind, v)
    }

    /// Nominal length in pixels along the heading.
    pub fn len(self) -> f32 {
        self.dims().0 as f32
    }

    /// (length, half-width of the grid, height of the grid).
    fn dims(self) -> (i32, i32, i32) {
        match self {
            BoatKind::Launch => (14, 4, 12),
            BoatKind::Rowboat => (13, 6, 7),
            BoatKind::Dinghy => (12, 4, 8),
            BoatKind::Sailboat => (14, 5, 22),
            BoatKind::Sloop => (22, 6, 32),
            BoatKind::MotorYacht => (26, 6, 18),
            BoatKind::Fishing => (20, 5, 20),
            BoatKind::Lobster => (16, 4, 14),
            BoatKind::Speedboat => (14, 4, 8),
            BoatKind::Kayak => (16, 4, 7),
            BoatKind::Pontoon => (20, 6, 14),
            BoatKind::Tug => (18, 5, 20),
            BoatKind::Barge => (34, 5, 10),
            BoatKind::Ferry => (34, 6, 20),
            BoatKind::CarFerry => (36, 7, 18),
            BoatKind::Coaster => (44, 6, 22),
            BoatKind::Container => (54, 8, 28),
        }
    }

    /// Half the beam at the bow, for the bow wave.
    fn bow_beam(self) -> f32 {
        match self {
            BoatKind::Kayak | BoatKind::Rowboat => 0.6,
            BoatKind::Barge | BoatKind::Pontoon | BoatKind::CarFerry => 3.0,
            BoatKind::Ferry | BoatKind::Coaster | BoatKind::Container => 1.6,
            _ => 1.0,
        }
    }

    /// How hard the wake churns: a drifting kayak or a houseboat barely.
    fn wake_power(self) -> f32 {
        match self {
            BoatKind::Kayak | BoatKind::Rowboat => 0.5,
            BoatKind::Pontoon => 0.35,
            BoatKind::Sailboat | BoatKind::Sloop | BoatKind::Dinghy => 0.7,
            BoatKind::Speedboat => 1.4,
            BoatKind::Tug | BoatKind::Ferry | BoatKind::CarFerry | BoatKind::Coaster | BoatKind::Container => 1.2,
            _ => 1.0,
        }
    }

    /// How many wake segments trail the hull, and how far apart (pixels).
    fn wake(self) -> (i32, f32) {
        let n = (5.0 + self.len() * 0.2).min(11.0) as i32;
        (n, 1.5 * self.wake_power().max(0.6))
    }

    /// Screen extent a boat of this kind can touch around its anchor, wake
    /// included: (half-width, up, down). The scene's damage rectangle.
    pub fn extent(self) -> (i32, i32, i32) {
        let (l, hw, h) = self.dims();
        let (n, step) = self.wake();
        let reach = l as f32 * 0.5 + n as f32 * step + self.wake_power() * n as f32 * 0.5 + 3.0;
        let across = hw as f32 + 4.0;
        // The long axis runs 2:1 on screen: 1 px across for every px along, and
        // half a px down; plus the hull's own beam.
        let (a, b, c) = (reach.ceil() as i32 + 2, h + (l + hw) / 4 + 6, (reach * 0.5 + across * 0.5).ceil() as i32 + 6);
        // And whatever the sprites themselves (hull, rig, lamps, contact ring)
        // reach, plus a pixel to bob in and the night glow.
        let (ka, kb, kc) = kit(self).ext;
        (a.max(ka + 3), b.max(kb + 3), c.max(kc + 3))
    }
}

/// What the sim calls a mover (mirrors `rail::Kind` for the three water ones).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Boat,
    Ferry,
    Ship,
}

// ----------------------------------------------------------------- voxels

const M_PLAIN: u8 = 0;
const M_GLASS: u8 = 1;
const M_LAMP: u8 = 2;
const M_PORT: u8 = 3;
const M_STBD: u8 = 4;
const M_WET: u8 = 5;
const M_SAIL: u8 = 6;
const M_HULL: u8 = 7;

struct Vox {
    nl: i32,
    nw: i32,
    nz: i32,
    ow: i32,
    col: Vec<u32>,
    mat: Vec<u8>,
    lines: Vec<Line>,
}

#[derive(Clone, Copy)]
struct Line {
    a: (f32, f32, f32),
    b: (f32, f32, f32),
    c: Color,
    m: u8,
}

impl Vox {
    fn new(nl: i32, half_w: i32, nz: i32) -> Self {
        let nw = half_w * 2;
        Self {
            nl,
            nw,
            nz,
            ow: half_w,
            col: vec![0; (nl * nw * nz) as usize],
            mat: vec![0; (nl * nw * nz) as usize],
            lines: Vec::new(),
        }
    }

    fn idx(&self, l: i32, w: i32, z: i32) -> Option<usize> {
        let wi = w + self.ow;
        if l < 0 || l >= self.nl || wi < 0 || wi >= self.nw || z < 0 || z >= self.nz {
            None
        } else {
            Some(((z * self.nw + wi) * self.nl + l) as usize)
        }
    }

    fn put(&mut self, l: i32, w: i32, z: i32, c: Color, m: u8) {
        if let Some(i) = self.idx(l, w, z) {
            self.col[i] = c;
            self.mat[i] = m;
        }
    }

    fn at(&self, l: i32, w: i32, z: i32) -> (Color, u8) {
        match self.idx(l, w, z) {
            Some(i) => (self.col[i], self.mat[i]),
            None => (0, 0),
        }
    }

    fn solid(&self, l: i32, w: i32, z: i32) -> bool {
        self.at(l, w, z).0 != 0
    }

    #[allow(clippy::too_many_arguments)]
    fn cube(&mut self, l0: i32, l1: i32, w0: i32, w1: i32, z0: i32, z1: i32, c: Color) {
        for z in z0..z1 {
            for w in w0..w1 {
                for l in l0..l1 {
                    self.put(l, w, z, c, M_PLAIN);
                }
            }
        }
    }

    fn line(&mut self, a: (f32, f32, f32), b: (f32, f32, f32), c: Color) {
        self.lines.push(Line { a, b, c, m: M_PLAIN });
    }

    /// A walled box with a window band and a roof layer.
    #[allow(clippy::too_many_arguments)]
    fn cabin(&mut self, l0: i32, l1: i32, w0: i32, w1: i32, z0: i32, z1: i32, wall: Color, roof: Color, glass: Color) {
        let zlo = (z1 - 3).max(z0 + 1);
        for z in z0..z1 {
            for w in w0..w1 {
                for l in l0..l1 {
                    let shell = l == l0 || l == l1 - 1 || w == w0 || w == w1 - 1;
                    let corner = (l == l0 || l == l1 - 1) && (w == w0 || w == w1 - 1);
                    let win = shell
                        && !corner
                        && z >= zlo
                        && z <= z1 - 2
                        && ((l - l0) % 3 != 2 || l == l0 || l == l1 - 1)
                        && ((w - w0) % 3 != 2 || w == w0 || w == w1 - 1);
                    if z == z1 - 1 {
                        self.put(l, w, z, if shell { shade(roof, 1.08) } else { roof }, M_PLAIN);
                    } else if win && (z1 - z0) >= 3 {
                        self.put(l, w, z, glass, M_GLASS);
                    } else {
                        self.put(l, w, z, wall, M_PLAIN);
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------------------- the hull

struct Hull {
    o: i32,
    len: i32,
    beam: f32,
    depth: i32,
    sheer: f32,
    bow_pt: f32,
    bow_pow: f32,
    stern: f32,
    flare: f32,
    open: bool,
    hull: Color,
    trim: Color,
    deck: Color,
    boot: Color,
    plank: bool,
}

impl Hull {
    fn halfwidth(&self, t: f32) -> f32 {
        let bow = if t > self.bow_pt {
            1.0 - ((t - self.bow_pt) / (1.0 - self.bow_pt)).powf(self.bow_pow)
        } else {
            1.0
        };
        let stern = if t < 0.3 {
            self.stern + (1.0 - self.stern) * (t / 0.3).powf(0.7)
        } else {
            1.0
        };
        (self.beam * 0.5 * bow.min(stern)).max(0.0)
    }

    fn deck_h(&self, t: f32) -> i32 {
        let rise = ((t - 0.35).max(0.0) / 0.65).powi(2) * self.sheer + (0.2 - t).max(0.0) * self.sheer * 0.5;
        self.depth + rise.round() as i32
    }

    fn fill(&self, v: &mut Vox) {
        let nlf = self.len as f32;
        for li in 0..self.len {
            let l = self.o + li;
            let t = (li as f32 + 0.5) / nlf;
            let hw = self.halfwidth(t);
            let dh = self.deck_h(t);
            if hw < 0.35 && li < self.len - 1 {
                continue;
            }
            for z in 0..dh {
                let zf = z as f32 / self.depth.max(1) as f32;
                let hz = (hw * (1.0 - self.flare * (1.0 - zf).max(0.0))).max(0.0);
                let hz = if li >= self.len - 2 { hz.max(0.55) } else { hz };
                for w in -8..8 {
                    let wc = (w as f32 + 0.5).abs();
                    if wc > hz + 0.01 {
                        continue;
                    }
                    let shell = wc > hz - 1.0 || li == 0 || li >= self.len - 2 || hz < 1.6;
                    let top = z == dh - 1;
                    let (c, m) = if z == 0 {
                        (shade(self.boot, 0.62), M_WET)
                    } else if top && shell {
                        (self.trim, M_PLAIN)
                    } else if shell {
                        let mut c = if z == 1 { self.boot } else { self.hull };
                        if self.plank && z % 3 == 2 {
                            c = shade(c, 0.9);
                        }
                        (c, M_HULL)
                    } else if self.open {
                        if z == 1 {
                            (shade(self.deck, 0.95), M_PLAIN)
                        } else {
                            continue;
                        }
                    } else if top {
                        let c = if (li + (w + 8)) % 4 == 0 && self.plank { shade(self.deck, 0.93) } else { self.deck };
                        (c, M_PLAIN)
                    } else {
                        (shade(self.hull, 0.8), M_HULL)
                    };
                    v.put(l, w, z, c, m);
                }
            }
        }
    }
}

fn hash(a: i32, b: i32, c: i32, s: u32) -> u32 {
    let mut h = (a as u32).wrapping_mul(0x9E37_79B1)
        ^ (b as u32).wrapping_mul(0x85EB_CA77)
        ^ (c as u32).wrapping_mul(0xC2B2_AE3D)
        ^ s.wrapping_mul(0x27D4_EB2F);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

/// Weathering: faded paint, rust streaks and the green line at the water.
fn weather(v: &mut Vox, level: u8, seed: u32) {
    if level == 0 {
        return;
    }
    let rust = rgb(0x9a, 0x4c, 0x2c);
    for z in 0..v.nz {
        for w in -v.ow..v.ow {
            for l in 0..v.nl {
                let (c, m) = v.at(l, w, z);
                if m != M_HULL && m != M_WET {
                    continue;
                }
                let h = hash(l, w, z, seed);
                if m == M_HULL {
                    if level >= 1 && h % 9 == 0 {
                        v.put(l, w, z, shade(c, 0.86), M_HULL);
                    }
                    if level >= 2 {
                        let streak = hash(l, w, 0, seed ^ 7) % 5 == 0;
                        if (z <= 2 && h % 4 == 0) || (streak && z <= 4 && h % 3 != 0) {
                            v.put(l, w, z, palette::mix(c, rust, 0.8), M_HULL);
                        }
                    }
                } else if level >= 1 && h % 3 == 0 {
                    v.put(l, w, z, palette::mix(c, rgb(0x2e, 0x5a, 0x3e), 0.5), M_WET);
                }
            }
        }
    }
}

// ----------------------------------------------------------------- looks

/// The paint of one variant.
struct Look {
    hull: Color,
    trim: Color,
    deck: Color,
    cabin: Color,
    roof: Color,
    sail: Color,
    boot: Color,
    wear: u8,
    seed: u32,
}

fn glass() -> Color {
    rgb(0x4a, 0x70, 0x88)
}

fn white() -> Color {
    rgb(0xea, 0xe6, 0xda)
}

fn wood() -> Color {
    rgb(0x9a, 0x6a, 0x3a)
}

fn pick<T: Copy>(a: &[T], i: usize) -> T {
    a[i % a.len()]
}

fn look(kind: BoatKind, variant: usize) -> Look {
    let paints = [
        rgb(0xe6, 0xdc, 0xc0),
        rgb(0xb4, 0x44, 0x3a),
        rgb(0x3c, 0x66, 0x8e),
        rgb(0x3e, 0x7c, 0x78),
        rgb(0x4c, 0x7c, 0x4a),
        rgb(0xd6, 0xa8, 0x3a),
        rgb(0x30, 0x40, 0x62),
        rgb(0xec, 0xe8, 0xdc),
    ];
    let sails = [
        rgb(0xf0, 0xec, 0xe0),
        rgb(0xe8, 0xdc, 0xc0),
        rgb(0xc4, 0x4a, 0x3a),
        rgb(0xe2, 0x9a, 0x3e),
        rgb(0x4a, 0x7a, 0xa8),
        rgb(0xe8, 0xcc, 0x58),
        rgb(0xa8, 0x58, 0x3c),
        rgb(0xf4, 0xf0, 0xe8),
    ];
    let trims = [
        rgb(0xc8, 0x3c, 0x30),
        rgb(0x2e, 0x4a, 0x7a),
        rgb(0x5a, 0x3a, 0x22),
        rgb(0xf0, 0xe8, 0xd0),
        rgb(0x2e, 0x5a, 0x40),
        rgb(0xd8, 0xa0, 0x30),
    ];
    let v = variant % VARIANTS;
    let seed = (kind.index() as u32) * 31 + v as u32 * 7 + 3;
    let wear = if v >= 6 {
        2
    } else if v >= 4 {
        1
    } else {
        0
    };
    let hull;
    let mut trim = pick(&trims, v * 3 + 1);
    let sail = pick(&sails, v + kind.index());
    let mut boot = rgb(0x2c, 0x30, 0x38);
    let mut deck = rgb(0xb8, 0x96, 0x62);
    let mut cabin = white();
    let mut roof = rgb(0xdc, 0xd6, 0xc6);
    match kind {
        BoatKind::Rowboat => {
            hull = if v % 3 == 0 { rgb(0x8a, 0x5a, 0x32) } else { pick(&paints, v + 1) };
            trim = rgb(0xec, 0xe0, 0xc4);
            deck = rgb(0x7a, 0x58, 0x38);
            boot = shade(hull, 0.55);
        }
        BoatKind::Dinghy => {
            hull = pick(&[white(), rgb(0x3c, 0x66, 0x8e), rgb(0xd6, 0xa8, 0x3a), rgb(0xb4, 0x44, 0x3a)], v);
            trim = rgb(0xf0, 0xe8, 0xd4);
            deck = rgb(0xa8, 0x88, 0x58);
            boot = shade(hull, 0.55);
        }
        BoatKind::Sailboat | BoatKind::Sloop => {
            hull = pick(
                &[white(), rgb(0x30, 0x40, 0x62), rgb(0xe6, 0xdc, 0xc0), rgb(0x3c, 0x66, 0x8e), white(), rgb(0x4c, 0x7c, 0x4a)],
                v,
            );
            boot = pick(&[rgb(0xb4, 0x44, 0x3a), rgb(0x2c, 0x30, 0x38), rgb(0x30, 0x40, 0x62)], v);
            cabin = if v % 2 == 0 { white() } else { rgb(0xe6, 0xdc, 0xc0) };
        }
        BoatKind::MotorYacht => {
            hull = pick(&[white(), white(), rgb(0x30, 0x40, 0x62), rgb(0xe6, 0xdc, 0xc0)], v);
            boot = pick(&[rgb(0x30, 0x40, 0x62), rgb(0x2c, 0x30, 0x38), rgb(0xb4, 0x44, 0x3a)], v);
            deck = rgb(0xc0, 0x9c, 0x68);
        }
        BoatKind::Fishing => {
            hull = pick(
                &[rgb(0x3c, 0x66, 0x8e), rgb(0xb4, 0x44, 0x3a), rgb(0x4c, 0x7c, 0x4a), rgb(0xe6, 0xdc, 0xc0), rgb(0x30, 0x40, 0x62), rgb(0xd6, 0xa8, 0x3a)],
                v,
            );
            boot = rgb(0x24, 0x28, 0x2e);
            cabin = pick(&[white(), rgb(0xe6, 0xdc, 0xc0), rgb(0xdc, 0xc8, 0x70)], v);
            deck = rgb(0x8a, 0x72, 0x52);
        }
        BoatKind::Lobster => {
            hull = pick(&[white(), rgb(0xe6, 0xdc, 0xc0), rgb(0x3c, 0x66, 0x8e), white()], v);
            boot = pick(&[rgb(0xb4, 0x44, 0x3a), rgb(0x2c, 0x30, 0x38), rgb(0x3e, 0x7c, 0x78)], v);
            cabin = pick(&[rgb(0xe6, 0xdc, 0xc0), white(), rgb(0x6a, 0xa0, 0xb0)], v);
            deck = rgb(0x8a, 0x72, 0x52);
        }
        BoatKind::Launch => {
            hull = pick(&[rgb(0xd6, 0xa8, 0x3a), white(), rgb(0x3c, 0x66, 0x8e), rgb(0xb4, 0x44, 0x3a)], v);
            roof = shade(hull, 1.05);
            boot = shade(hull, 0.5);
        }
        BoatKind::Speedboat => {
            hull = pick(
                &[rgb(0xc8, 0x3c, 0x30), white(), rgb(0x2e, 0x5a, 0x9a), rgb(0xe8, 0xb8, 0x30), rgb(0x30, 0x34, 0x3c)],
                v,
            );
            trim = pick(&[white(), rgb(0x2c, 0x30, 0x38), rgb(0xe8, 0xb8, 0x30)], v);
            boot = rgb(0x24, 0x28, 0x2e);
        }
        BoatKind::Kayak => {
            hull = pick(
                &[rgb(0xe0, 0x7a, 0x2e), rgb(0xe8, 0xcc, 0x40), rgb(0xc8, 0x3c, 0x30), rgb(0x3e, 0x9a, 0x6a), rgb(0x3c, 0x7a, 0xc0)],
                v,
            );
            trim = shade(hull, 0.7);
        }
        BoatKind::Pontoon => {
            hull = pick(&[rgb(0x7a, 0x84, 0x88), rgb(0x8a, 0x72, 0x52), rgb(0x5a, 0x6a, 0x70)], v);
            cabin = pick(&[rgb(0xe6, 0xdc, 0xc0), rgb(0x8a, 0xb0, 0xa8), rgb(0xd0, 0x8a, 0x6a), rgb(0xe8, 0xcc, 0x70)], v);
            roof = pick(&[rgb(0x9a, 0x44, 0x3a), rgb(0x3c, 0x66, 0x8e), rgb(0x5a, 0x6a, 0x50), rgb(0x6a, 0x5a, 0x4a)], v + 1);
            boot = shade(hull, 0.55);
            deck = rgb(0xa8, 0x86, 0x58);
        }
        BoatKind::Tug => {
            hull = pick(&[rgb(0xb4, 0x44, 0x3a), rgb(0x30, 0x40, 0x62), rgb(0x2c, 0x30, 0x38), rgb(0x3e, 0x7c, 0x78)], v);
            boot = rgb(0x1c, 0x20, 0x24);
            cabin = pick(&[white(), rgb(0xe6, 0xdc, 0xc0)], v);
            deck = rgb(0x6a, 0x6e, 0x72);
        }
        BoatKind::Barge => {
            hull = pick(
                &[rgb(0x4a, 0x50, 0x58), rgb(0x6a, 0x3a, 0x2c), rgb(0x30, 0x40, 0x62), rgb(0x4c, 0x6a, 0x4a), rgb(0x2c, 0x30, 0x38)],
                v,
            );
            boot = rgb(0x1c, 0x20, 0x24);
            deck = rgb(0x6a, 0x6e, 0x72);
            cabin = pick(&[white(), rgb(0xe6, 0xdc, 0xc0), rgb(0xc8, 0xa8, 0x70)], v);
        }
        BoatKind::Ferry | BoatKind::CarFerry => {
            hull = pick(&[white(), white(), rgb(0xe6, 0xdc, 0xc0), white()], v);
            trim = pick(&[rgb(0x2e, 0x5a, 0x8a), rgb(0xb4, 0x44, 0x3a), rgb(0x2e, 0x5a, 0x40), rgb(0x30, 0x40, 0x62)], v);
            boot = pick(&[rgb(0x2e, 0x5a, 0x8a), rgb(0xb4, 0x44, 0x3a), rgb(0x2e, 0x5a, 0x40), rgb(0x30, 0x40, 0x62)], v);
            cabin = white();
            roof = rgb(0xd8, 0xd4, 0xc8);
            deck = rgb(0x8a, 0x8e, 0x92);
        }
        BoatKind::Coaster => {
            hull = pick(&[rgb(0x30, 0x40, 0x62), rgb(0x6a, 0x3a, 0x2c), rgb(0x2e, 0x5a, 0x40), rgb(0x4a, 0x50, 0x58), rgb(0x8a, 0x2e, 0x2c)], v);
            boot = rgb(0x8a, 0x2e, 0x2c);
            deck = rgb(0x6a, 0x6e, 0x72);
            cabin = white();
        }
        BoatKind::Container => {
            hull = pick(&[rgb(0x2c, 0x30, 0x38), rgb(0x30, 0x40, 0x62), rgb(0x2e, 0x5a, 0x40), rgb(0x6a, 0x2e, 0x2a)], v);
            boot = rgb(0xa8, 0x3a, 0x30);
            deck = rgb(0x6a, 0x6e, 0x72);
            cabin = white();
        }
    }
    Look { hull, trim, deck, cabin, roof, sail, boot, wear, seed }
}

// ----------------------------------------------------------------- the models

const RED_LAMP: Color = 0xFF_E8_40_30;
const GREEN_LAMP: Color = 0xFF_40_E0_60;

fn red_lamp() -> Color {
    rgb(0xe8, 0x40, 0x30)
}

fn green_lamp() -> Color {
    rgb(0x40, 0xe0, 0x60)
}

/// Build one kind's voxel model in a variant.
fn model(kind: BoatKind, variant: usize) -> Vox {
    let lk = look(kind, variant);
    let (len, hw, hz) = kind.dims();
    let o = 2;
    let mut v = Vox::new(len + 2 * o, hw, hz);
    let nl = len + 2 * o;
    let var = variant % VARIANTS;
    let dark = rgb(0x2c, 0x2e, 0x34);
    let rope = rgb(0xc8, 0xb4, 0x84);
    let skin = rgb(0xe0, 0xb0, 0x88);
    let _ = (RED_LAMP, GREEN_LAMP);
    let mut h = Hull {
        o,
        len,
        beam: 5.0,
        depth: 3,
        sheer: 1.0,
        bow_pt: 0.55,
        bow_pow: 1.4,
        stern: 0.8,
        flare: 0.2,
        open: false,
        hull: lk.hull,
        trim: lk.trim,
        deck: lk.deck,
        boot: lk.boot,
        plank: false,
    };
    match kind {
        BoatKind::Rowboat => {
            h.beam = 5.4;
            h.depth = 4;
            h.sheer = 1.6;
            h.stern = 0.6;
            h.bow_pt = 0.5;
            h.bow_pow = 1.15;
            h.open = true;
            h.plank = true;
            h.fill(&mut v);
            for l in [o + 4, o + 8] {
                for w in -2..2 {
                    v.put(l, w, 3, rgb(0xe8, 0xd0, 0x98), M_PLAIN);
                }
            }
            let shirt = pick(&[rgb(0xb4, 0x44, 0x3a), rgb(0x3c, 0x66, 0x8e), rgb(0xe8, 0xdc, 0xc0), rgb(0x4c, 0x7c, 0x4a)], var);
            v.put(o + 6, 0, 2, shirt, M_PLAIN);
            v.put(o + 6, 0, 3, shirt, M_PLAIN);
            v.put(o + 6, -1, 3, shade(shirt, 0.85), M_PLAIN);
            v.put(o + 6, 0, 4, skin, M_PLAIN);
            v.put(o + 5, 0, 4, rgb(0x5a, 0x3a, 0x22), M_PLAIN);
            let oc = rgb(0xe8, 0xd0, 0xa0);
            let oarl = o as f32 + 6.5;
            v.line((oarl, 1.8, 3.2), (oarl - 2.5, 7.5, 0.5), oc);
            v.line((oarl, -1.8, 3.2), (oarl - 2.5, -7.5, 0.5), oc);
            v.put(o + 4, 7, 0, rgb(0xf4, 0xe8, 0xc0), M_PLAIN);
        }
        BoatKind::Dinghy => {
            h.beam = 5.2;
            h.depth = 3;
            h.sheer = 0.8;
            h.stern = 0.9;
            h.bow_pt = 0.5;
            h.bow_pow = 1.1;
            h.open = true;
            h.fill(&mut v);
            for l in [o + 5, o + 8] {
                for w in -1..=0 {
                    v.put(l, w, 2, shade(wood(), 1.05), M_PLAIN);
                }
            }
            v.cube(o - 1, o + 1, -1, 1, 2, 5, dark);
            v.put(o - 1, 0, 5, rgb(0xb4, 0x44, 0x3a), M_PLAIN);
            v.line((o as f32, 0.0, 3.0), (o as f32 + 2.5, 0.0, 3.6), dark);
            if var % 2 == 0 {
                v.line((o as f32 + 4.0, -1.5, 3.4), (o as f32 + 10.0, -1.5, 3.4), shade(lk.sail, 0.9));
            }
            let oc = shade(wood(), 1.2);
            v.line((o as f32 + 3.0, 1.6, 2.6), (o as f32 + 9.0, 1.2, 2.6), oc);
            let shirt = pick(&[rgb(0xe2, 0x9a, 0x3e), rgb(0x3c, 0x66, 0x8e), rgb(0xe8, 0xdc, 0xc0)], var);
            v.put(o + 4, 0, 2, shirt, M_PLAIN);
            v.put(o + 4, 0, 3, shirt, M_PLAIN);
            v.put(o + 4, 0, 4, skin, M_PLAIN);
        }
        BoatKind::Sailboat => {
            h.beam = 5.2;
            h.depth = 3;
            h.sheer = 1.0;
            h.stern = 0.7;
            h.bow_pt = 0.5;
            h.bow_pow = 1.3;
            h.fill(&mut v);
            for l in o + 2..o + 6 {
                for w in -1..1 {
                    v.put(l, w, 2, shade(lk.deck, 0.7), M_PLAIN);
                }
            }
            v.cabin(o + 6, o + 10, -1, 1, 3, 5, lk.cabin, shade(lk.cabin, 0.96), glass());
            rig(&mut v, o as f32 + 7.0, 17.0, 3.0, (o + len) as f32 - 1.0, o as f32 + 0.5, lk.sail, 0.09, var, false);
        }
        BoatKind::Sloop => {
            h.beam = 7.0;
            h.depth = 4;
            h.sheer = 1.5;
            h.stern = 0.62;
            h.bow_pt = 0.5;
            h.bow_pow = 1.35;
            h.fill(&mut v);
            for l in o + 2..o + 7 {
                for w in -2..2 {
                    v.put(l, w, 3, shade(lk.deck, 0.72), M_PLAIN);
                }
            }
            v.cabin(o + 8, o + 16, -2, 2, 4, 7, lk.cabin, shade(lk.cabin, 0.98), glass());
            for l in 0..nl {
                for w in -v.ow..v.ow {
                    let (_, m) = v.at(l, w, 3);
                    if m == M_HULL {
                        v.put(l, w, 3, lk.trim, M_HULL);
                    }
                }
            }
            rig(&mut v, o as f32 + 11.0, 27.0, 4.5, (o + len) as f32 - 1.5, o as f32 + 0.5, lk.sail, 0.08, var, true);
        }
        BoatKind::MotorYacht => {
            h.beam = 8.0;
            h.depth = 4;
            h.sheer = 1.5;
            h.stern = 0.78;
            h.bow_pt = 0.55;
            h.bow_pow = 1.3;
            h.fill(&mut v);
            v.cabin(o + 8, o + 21, -3, 3, 4, 8, lk.cabin, shade(lk.cabin, 0.97), glass());
            v.cabin(o + 11, o + 18, -2, 2, 8, 11, lk.cabin, rgb(0xe8, 0xe4, 0xd8), glass());
            for l in o + 2..o + 6 {
                v.put(l, -1, 4, rgb(0xe8, 0xe0, 0xc8), M_PLAIN);
                v.put(l, 0, 4, rgb(0xe8, 0xe0, 0xc8), M_PLAIN);
            }
            for l in 0..nl {
                for w in -v.ow..v.ow {
                    let (_, m) = v.at(l, w, 3);
                    if m == M_HULL {
                        v.put(l, w, 3, lk.trim, M_HULL);
                    }
                }
            }
            let ml = (o + 15) as f32;
            v.line((ml, 0.0, 11.0), (ml, 0.0, 17.0), shade(white(), 0.9));
            v.put(o + 15, -1, 14, dark, M_PLAIN);
            v.put(o + 15, 0, 14, dark, M_PLAIN);
            v.put(o + 15, 0, 17, white(), M_LAMP);
            v.line((ml, 0.0, 17.0), (ml - 3.0, 0.0, 9.0), rgb(0xa0, 0xa4, 0xa8));
            v.put(o + len - 3, 0, 5, rgb(0xb8, 0xbc, 0xc0), M_PLAIN);
        }
        BoatKind::Fishing => {
            h.beam = 7.0;
            h.depth = 4;
            h.sheer = 2.0;
            h.stern = 0.8;
            h.bow_pt = 0.58;
            h.bow_pow = 1.25;
            h.plank = true;
            h.fill(&mut v);
            let wl = o + 11 + (var as i32 % 2);
            v.cabin(wl, wl + 6, -2, 2, 5, 10, lk.cabin, shade(lk.cabin, 0.9), glass());
            v.cube(wl - 2, wl - 1, 0, 1, 5, 12, dark);
            v.put(wl - 2, 0, 12, dark, M_PLAIN);
            let ml = (o + 8) as f32;
            v.line((ml, 0.0, 5.0), (ml, 0.0, 17.0), rgb(0x7a, 0x62, 0x44));
            v.put(o + 8, 0, 17, white(), M_LAMP);
            v.line((ml, 0.0, 15.0), (ml - 5.0, 3.5, 7.0), rope);
            v.line((ml, 0.0, 14.0), (ml + 5.0, 0.0, 10.5), shade(rope, 0.8));
            let net = pick(&[rgb(0x5a, 0x7a, 0x5a), rgb(0x6a, 0x8a, 0x8a), rgb(0x8a, 0x7a, 0x4a)], var);
            for l in o + 2..o + 7 {
                for w in -2..2 {
                    let hgt = if (l + w) % 2 == 0 { 2 } else { 1 };
                    for z in 0..hgt {
                        v.put(l, w, 5 + z, if (l * 3 + w) % 4 == 0 { shade(net, 0.7) } else { net }, M_PLAIN);
                    }
                }
            }
            for (l, w) in [(o + 3, -2), (o + 6, 1), (o + 4, 1)] {
                v.put(l, w, 7, rgb(0xe8, 0x6a, 0x2a), M_PLAIN);
            }
            v.cube(o + 9, o + 11, 1, 3, 5, 7, wood());
            v.put(wl + 1, 0, 10, white(), M_LAMP);
            v.put(wl + 5, -2, 8, red_lamp(), M_PORT);
            v.put(wl + 5, 1, 8, green_lamp(), M_STBD);
        }
        BoatKind::Lobster => {
            h.beam = 5.6;
            h.depth = 3;
            h.sheer = 2.0;
            h.stern = 0.78;
            h.bow_pt = 0.5;
            h.bow_pow = 1.2;
            h.plank = true;
            h.fill(&mut v);
            v.cabin(o + 8, o + 12, -2, 2, 4, 8, lk.cabin, shade(lk.cabin, 0.92), glass());
            let trap = rgb(0x6a, 0x58, 0x3c);
            for (l, w) in [(o + 2, -1), (o + 2, 0), (o + 4, -1), (o + 4, 0)] {
                for z in 4..6 {
                    v.put(l, w, z, if z == 4 { shade(trap, 0.8) } else { trap }, M_PLAIN);
                }
            }
            v.put(o + 3, 0, 6, rgb(0xd8, 0x40, 0x30), M_PLAIN);
            v.put(o + 3, -1, 4, rgb(0xe8, 0xb8, 0x30), M_PLAIN);
            v.put(o + 9, -1, 8, rgb(0xd8, 0x40, 0x30), M_PLAIN);
            v.put(o + 10, 1, 8, rgb(0xe8, 0xb8, 0x30), M_PLAIN);
            v.line((o as f32 + 9.5, 0.0, 8.5), (o as f32 + 9.5, 0.0, 13.0), dark);
            v.put(o + 9, 0, 13, white(), M_LAMP);
        }
        BoatKind::Launch => {
            h.beam = 5.4;
            h.depth = 3;
            h.sheer = 1.0;
            h.stern = 0.82;
            h.bow_pt = 0.55;
            h.bow_pow = 1.3;
            h.fill(&mut v);
            let roof = lk.roof;
            for (pl, pw) in [(o + 4, -2), (o + 4, 1), (o + 9, -2), (o + 9, 1)] {
                for z in 3..7 {
                    v.put(pl, pw, z, rgb(0xe8, 0xe4, 0xd8), M_PLAIN);
                }
            }
            for l in o + 3..o + 11 {
                for w in -2..2 {
                    v.put(l, w, 7, if l == o + 3 || l == o + 10 { shade(roof, 1.1) } else { roof }, M_PLAIN);
                }
            }
            for l in o + 5..o + 9 {
                v.put(l, -1, 3, shade(wood(), 0.9), M_PLAIN);
                v.put(l, 0, 3, shade(wood(), 0.9), M_PLAIN);
            }
            v.cabin(o + 10, o + 12, -1, 1, 3, 5, white(), white(), glass());
            v.put(o + 6, 0, 8, rgb(0xf4, 0xf0, 0xd8), M_LAMP);
            v.put(o + 7, 0, 8, rgb(0xf4, 0xf0, 0xd8), M_PLAIN);
            v.line((o as f32 + 1.0, 0.0, 3.0), (o as f32 + 1.0, 0.0, 8.0), rgb(0xc8, 0xc8, 0xc8));
            v.put(o + 1, 0, 8, rgb(0xc8, 0x3c, 0x30), M_PLAIN);
            v.put(o, 0, 8, rgb(0xc8, 0x3c, 0x30), M_PLAIN);
        }
        BoatKind::Speedboat => {
            h.beam = 4.8;
            h.depth = 2;
            h.sheer = 1.0;
            h.stern = 0.92;
            h.bow_pt = 0.42;
            h.bow_pow = 1.15;
            h.flare = 0.12;
            h.fill(&mut v);
            for l in o + 3..o + 8 {
                for w in -1..1 {
                    v.put(l, w, 1, shade(lk.deck, 0.55), M_PLAIN);
                }
            }
            for w in -2..2 {
                v.put(o + 8, w, 2, shade(glass(), 1.1), M_GLASS);
                v.put(o + 8, w, 3, shade(glass(), 1.2), M_GLASS);
            }
            v.cube(o + 6, o + 8, -1, 1, 2, 3, rgb(0xe8, 0xe0, 0xc8));
            v.cube(o, o + 3, -1, 1, 2, 4, shade(lk.hull, 0.8));
            for l in o + 9..o + len - 1 {
                v.put(l, -1, 2, lk.trim, M_PLAIN);
                v.put(l, 0, 2, lk.trim, M_PLAIN);
            }
            v.put(o + 5, 0, 2, rgb(0x2e, 0x5a, 0x9a), M_PLAIN);
            v.put(o + 5, 0, 3, skin, M_PLAIN);
            v.put(o + 5, 0, 4, rgb(0xc8, 0x3c, 0x30), M_PLAIN);
            v.line((o as f32 + 1.5, 0.0, 4.0), (o as f32 + 1.5, 0.0, 8.0), dark);
            v.put(o + 1, 0, 8, white(), M_LAMP);
        }
        BoatKind::Kayak => {
            h.beam = 3.4;
            h.depth = 2;
            h.sheer = 1.2;
            h.stern = 0.25;
            h.bow_pt = 0.3;
            h.bow_pow = 0.9;
            h.flare = 0.0;
            h.fill(&mut v);
            let ring = shade(lk.hull, 0.55);
            for w in -1..1 {
                v.put(o + 7, w, 2, ring, M_PLAIN);
                v.put(o + 9, w, 2, ring, M_PLAIN);
            }
            v.put(o + 8, -1, 2, ring, M_PLAIN);
            v.put(o + 8, 0, 2, ring, M_PLAIN);
            let shirt = pick(&[rgb(0xe8, 0xdc, 0xc0), rgb(0xb4, 0x44, 0x3a), rgb(0x3c, 0x66, 0x8e), rgb(0x4c, 0x7c, 0x4a)], var);
            v.put(o + 8, 0, 3, shirt, M_PLAIN);
            v.put(o + 8, -1, 3, shirt, M_PLAIN);
            v.put(o + 8, 0, 4, skin, M_PLAIN);
            v.put(o + 8, 0, 5, rgb(0x5a, 0x3a, 0x22), M_PLAIN);
            let pc = rgb(0x3a, 0x3e, 0x48);
            let blade = rgb(0xf0, 0xe8, 0xd0);
            let pl = o as f32 + 8.5;
            let tilt = if var % 2 == 0 { 1.0 } else { -1.0 };
            v.line((pl - 1.0, -6.0 * tilt, 0.8), (pl + 1.0, 6.0 * tilt, 3.4), pc);
            v.put(o + 7, (-6.0 * tilt) as i32, 0, blade, M_PLAIN);
            v.put(o + 8, (6.0 * tilt) as i32, 3, blade, M_PLAIN);
        }
        BoatKind::Pontoon => {
            h.beam = 10.0;
            h.depth = 2;
            h.sheer = 0.0;
            h.stern = 1.0;
            h.bow_pt = 0.92;
            h.bow_pow = 2.0;
            h.flare = 0.0;
            h.plank = true;
            h.fill(&mut v);
            if var % 2 == 0 {
                let wall = lk.cabin;
                v.cabin(o + 4, o + 16, -3, 3, 2, 7, wall, wall, glass());
                for k in 0..4 {
                    for l in o + 3..o + 17 {
                        for w in -4 + k..4 - k {
                            let z = 7 + k;
                            let edge = w == -4 + k || w == 3 - k;
                            v.put(l, w, z, if edge { shade(lk.roof, 1.12) } else { lk.roof }, M_PLAIN);
                        }
                    }
                }
                v.cube(o + 12, o + 14, 1, 3, 11, 13, rgb(0x7a, 0x4a, 0x3a));
                for w in -3..3 {
                    v.put(o + len - 2, w, 2, shade(wood(), 1.2), M_PLAIN);
                }
                v.put(o + len - 3, 2, 2, rgb(0x4c, 0x8a, 0x4c), M_PLAIN);
                v.put(o + len - 3, 2, 3, rgb(0xd8, 0x50, 0x50), M_PLAIN);
            } else {
                for z in 2..6 {
                    for (l, w) in [(o + 3, -3), (o + 3, 2), (o + 15, -3), (o + 15, 2), (o + 9, -3), (o + 9, 2)] {
                        v.put(l, w, z, rgb(0xe0, 0xdc, 0xd0), M_PLAIN);
                    }
                }
                for l in o + 2..o + 17 {
                    for w in -4..4 {
                        v.put(l, w, 6, if l % 3 == 0 { shade(lk.roof, 1.1) } else { lk.roof }, M_PLAIN);
                    }
                }
                for l in o + 4..o + 14 {
                    v.put(l, -2, 2, shade(wood(), 1.1), M_PLAIN);
                    v.put(l, 1, 2, shade(wood(), 1.1), M_PLAIN);
                }
            }
        }
        BoatKind::Tug => {
            h.beam = 8.0;
            h.depth = 4;
            h.sheer = 2.0;
            h.stern = 0.85;
            h.bow_pt = 0.6;
            h.bow_pow = 1.3;
            h.fill(&mut v);
            v.cabin(o + 3, o + 10, -2, 2, 5, 10, lk.cabin, shade(lk.cabin, 0.9), glass());
            v.cabin(o + 5, o + 8, -1, 1, 10, 12, lk.cabin, white(), glass());
            v.cube(o + 11, o + 13, -1, 1, 5, 13, lk.hull);
            v.cube(o + 11, o + 13, -1, 1, 11, 13, dark);
            v.put(o + 6, 0, 12, white(), M_LAMP);
            for l in o + len - 6..o + len - 1 {
                for w in -v.ow..v.ow {
                    let (_, m) = v.at(l, w, 4);
                    let (_, mu) = v.at(l, w, 3);
                    if m == M_PLAIN && mu == M_HULL {
                        v.put(l, w, 4, rgb(0x1c, 0x1c, 0x20), M_PLAIN);
                    }
                }
            }
            v.cube(o + 1, o + 3, -1, 1, 4, 6, dark);
            v.put(o + 1, 0, 6, rgb(0xc8, 0xc0, 0xa0), M_PLAIN);
        }
        BoatKind::Barge => {
            h.beam = 9.0;
            h.depth = 3;
            h.sheer = 0.5;
            h.stern = 0.9;
            h.bow_pt = 0.86;
            h.bow_pow = 1.6;
            h.flare = 0.0;
            h.plank = true;
            h.open = true;
            h.fill(&mut v);
            v.cabin(o + 1, o + 6, -2, 2, 3, 7, lk.cabin, shade(lk.roof, 0.9), glass());
            let cargo = pick(&[rgb(0x2c, 0x2e, 0x34), rgb(0xd0, 0xb0, 0x6c), rgb(0x7a, 0x6a, 0x54), rgb(0x8a, 0x8a, 0x84)], var);
            let kind_c = var % 3;
            for l in o + 8..o + len - 3 {
                let tl = (l - (o + 8)) as f32 / (len - 11) as f32;
                let bulge = ((tl * std::f32::consts::PI).sin() * 3.0).round() as i32;
                for w in -3..3 {
                    if kind_c == 2 {
                        let cc = pick(
                            &[rgb(0xb4, 0x44, 0x3a), rgb(0x3c, 0x66, 0x8e), rgb(0xe2, 0x9a, 0x3e), rgb(0x4c, 0x7c, 0x4a), rgb(0x8a, 0x8a, 0x84)],
                            (l as usize / 4) + (w + 3) as usize,
                        );
                        let hgt = if (l / 4 + w) % 3 == 0 { 3 } else { 5 };
                        if ((l - (o + 8)) % 4) < 3 {
                            for z in 2..2 + hgt {
                                v.put(l, w, z, if z == 1 + hgt { shade(cc, 1.1) } else { cc }, M_PLAIN);
                            }
                        }
                    } else {
                        let top = 2 + (bulge as f32 * (1.0 - (w as f32 + 0.5).abs() / 4.0)).round() as i32;
                        for z in 1..top.max(2) {
                            let c = if (l * 5 + w * 3 + z) % 4 == 0 { shade(cargo, 0.82) } else { cargo };
                            v.put(l, w, z, if z == top - 1 { shade(c, 1.12) } else { c }, M_PLAIN);
                        }
                    }
                }
            }
            v.put(o + len - 2, -3, 3, dark, M_PLAIN);
            v.put(o + len - 2, 2, 3, dark, M_PLAIN);
            v.put(o + 7, 0, 3, rope, M_PLAIN);
            v.put(o + 3, 0, 7, white(), M_LAMP);
        }
        BoatKind::Ferry => {
            h.beam = 11.0;
            h.depth = 4;
            h.sheer = 1.5;
            h.stern = 0.78;
            h.bow_pt = 0.7;
            h.bow_pow = 1.4;
            h.flare = 0.12;
            h.fill(&mut v);
            v.cabin(o + 4, o + 28, -4, 4, 5, 9, lk.cabin, rgb(0xd8, 0xd4, 0xc8), glass());
            v.cabin(o + 8, o + 24, -3, 3, 9, 12, lk.cabin, white(), glass());
            v.cabin(o + 17, o + 22, -2, 2, 12, 15, lk.cabin, white(), glass());
            for l in o + 4..o + 28 {
                for w in -4..4 {
                    let (_, m) = v.at(l, w, 5);
                    if (l == o + 4 || l == o + 27 || w == -4 || w == 3) && m == M_PLAIN {
                        v.put(l, w, 5, lk.trim, M_PLAIN);
                    }
                }
            }
            v.cube(o + 11, o + 14, -1, 1, 12, 17, lk.trim);
            v.cube(o + 11, o + 14, -1, 1, 15, 17, dark);
            for l in [o + 10, o + 12, o + 14] {
                v.put(l, -4, 9, rgb(0xe2, 0x6a, 0x2a), M_PLAIN);
                v.put(l, 3, 9, rgb(0xe2, 0x6a, 0x2a), M_PLAIN);
            }
            v.put(o + 19, 0, 15, white(), M_LAMP);
            v.put(o + 17, -2, 12, red_lamp(), M_PORT);
            v.put(o + 17, 1, 12, green_lamp(), M_STBD);
            v.line((o as f32 + 19.5, 0.0, 15.0), (o as f32 + 19.5, 0.0, 19.0), dark);
        }
        BoatKind::CarFerry => {
            h.beam = 12.0;
            h.depth = 4;
            h.sheer = 1.0;
            h.stern = 0.92;
            h.bow_pt = 0.82;
            h.bow_pow = 1.5;
            h.flare = 0.08;
            h.open = true;
            h.fill(&mut v);
            let cc = [
                rgb(0xb4, 0x44, 0x3a),
                rgb(0x3c, 0x66, 0x8e),
                rgb(0xe8, 0xdc, 0xc0),
                rgb(0x4c, 0x7c, 0x4a),
                rgb(0xd6, 0xa8, 0x3a),
                rgb(0x5a, 0x5e, 0x64),
            ];
            for k in 0..5 {
                let l = o + 10 + k * 5;
                for w in [-4, 1] {
                    let c = pick(&cc, (k as usize) * 2 + (w + 4) as usize + var);
                    if (k + w + var as i32).rem_euclid(5) == 0 {
                        continue;
                    }
                    v.cube(l, l + 4, w, w + 3, 2, 4, c);
                    v.cube(l + 1, l + 3, w, w + 3, 4, 5, shade(c, 0.85));
                }
            }
            v.cabin(o + 2, o + 8, -4, 4, 4, 11, lk.cabin, rgb(0xd8, 0xd4, 0xc8), glass());
            v.cabin(o + 3, o + 7, -3, 3, 11, 14, lk.cabin, white(), glass());
            v.cube(o + 4, o + 6, 3, 5, 11, 16, lk.trim);
            v.cube(o + 4, o + 6, 3, 5, 14, 16, dark);
            for w in -3..3 {
                v.put(o + len - 2, w, 4, shade(lk.trim, 0.9), M_PLAIN);
            }
            v.put(o + 5, 0, 14, white(), M_LAMP);
            v.put(o + 3, -3, 11, red_lamp(), M_PORT);
            v.put(o + 3, 2, 11, green_lamp(), M_STBD);
            v.line((o as f32 + 5.5, 0.0, 14.0), (o as f32 + 5.5, 0.0, 18.0), dark);
        }
        BoatKind::Coaster => {
            h.beam = 11.0;
            h.depth = 6;
            h.sheer = 1.5;
            h.stern = 0.8;
            h.bow_pt = 0.75;
            h.bow_pow = 1.5;
            h.flare = 0.1;
            h.fill(&mut v);
            v.cabin(o + 2, o + 10, -4, 4, 6, 13, lk.cabin, rgb(0xd8, 0xd4, 0xc8), glass());
            v.cabin(o + 3, o + 8, -3, 3, 13, 16, lk.cabin, white(), glass());
            v.cube(o + 11, o + 13, -1, 1, 6, 14, lk.boot);
            v.cube(o + 11, o + 13, -1, 1, 12, 14, dark);
            let hatch = pick(&[rgb(0x6a, 0x8a, 0x6a), rgb(0x8a, 0x5a, 0x4a), rgb(0x6a, 0x78, 0x8a), rgb(0x8a, 0x84, 0x70)], var);
            v.cube(o + 15, o + 33, -3, 3, 6, 8, hatch);
            for l in (o + 16..o + 33).step_by(4) {
                v.cube(l, l + 1, -3, 3, 8, 9, shade(hatch, 0.8));
            }
            for l in [o + 20, o + 28] {
                v.cube(l, l + 1, 3, 4, 6, 14, rgb(0xc8, 0x9a, 0x32));
                v.line((l as f32 + 0.5, 3.5, 14.0), (l as f32 + 0.5, -2.0, 9.0), rgb(0xe8, 0xba, 0x42));
            }
            v.put(o + 5, 0, 16, white(), M_LAMP);
            v.put(o + 3, -3, 13, red_lamp(), M_PORT);
            v.put(o + 3, 2, 13, green_lamp(), M_STBD);
            v.line((o as f32 + 5.5, 0.0, 16.0), (o as f32 + 5.5, 0.0, 21.0), dark);
            v.cube(o + len - 4, o + len - 2, -2, 2, 6, 8, shade(lk.deck, 0.9));
        }
        BoatKind::Container => {
            h.beam = 14.0;
            h.depth = 6;
            h.sheer = 1.5;
            h.stern = 0.85;
            h.bow_pt = 0.82;
            h.bow_pow = 1.6;
            h.flare = 0.1;
            h.fill(&mut v);
            v.cabin(o + 2, o + 9, -5, 5, 6, 14, lk.cabin, rgb(0xd8, 0xd4, 0xc8), glass());
            v.cabin(o + 3, o + 8, -4, 4, 14, 18, lk.cabin, white(), glass());
            v.cube(o + 9, o + 12, -1, 1, 6, 15, lk.boot);
            v.cube(o + 9, o + 12, -1, 1, 13, 15, dark);
            let pal = [
                rgb(0xb4, 0x44, 0x3a),
                rgb(0x3c, 0x66, 0x8e),
                rgb(0xe2, 0x9a, 0x3e),
                rgb(0x4c, 0x7c, 0x4a),
                rgb(0x8a, 0x8a, 0x84),
                rgb(0xe6, 0xdc, 0xc0),
                rgb(0x6a, 0x44, 0x7a),
                rgb(0x3e, 0x7c, 0x78),
            ];
            let mut l = o + 13;
            let mut bay = 0;
            while l + 4 <= o + len - 6 {
                for row in -3..3 {
                    let hs = hash(bay, row, 1, lk.seed) % 3;
                    let stack = 2 + hs as i32 + if row.abs() < 2 { 1 } else { 0 };
                    for k in 0..stack {
                        let cidx = hash(bay, row, k + 11, lk.seed) as usize % pal.len();
                        let cc = pal[cidx];
                        for ll in l..l + 4 {
                            for ww in [row * 2, row * 2 + 1] {
                                let z0 = 7 + k * 2;
                                v.put(ll, ww, z0, shade(cc, 0.95), M_PLAIN);
                                v.put(ll, ww, z0 + 1, if k == stack - 1 { shade(cc, 1.1) } else { cc }, M_PLAIN);
                            }
                        }
                    }
                }
                l += 5;
                bay += 1;
            }
            v.put(o + 5, 0, 18, white(), M_LAMP);
            v.put(o + 3, -4, 14, red_lamp(), M_PORT);
            v.put(o + 3, 3, 14, green_lamp(), M_STBD);
            v.line((o as f32 + 5.5, 0.0, 18.0), (o as f32 + 5.5, 0.0, 23.0), dark);
        }
    }
    weather(&mut v, lk.wear, lk.seed);
    v
}

/// Mast, boom, main and head sails, stays, with a lean to leeward. `big` adds
/// a larger jib.
#[allow(clippy::too_many_arguments)]
fn rig(v: &mut Vox, mast_l: f32, top: f32, foot: f32, bow_l: f32, stern_l: f32, sail: Color, lean: f32, var: usize, big: bool) {
    let mast_c = rgb(0xd8, 0xd4, 0xc8);
    let rigc = rgb(0xa8, 0xaa, 0xac);
    let lw = |z: f32| lean * (z - foot);
    v.line((mast_l, 0.0, foot - 1.0), (mast_l, lw(top), top), mast_c);
    let boom_end = stern_l + 1.0;
    v.line((mast_l - 0.5, 0.0, foot + 0.5), (boom_end, 0.3, foot + 0.8), mast_c);
    v.line((mast_l, lw(top), top), (bow_l, 0.0, foot), rigc);
    v.line((mast_l, lw(top), top), (stern_l, 0.0, foot), rigc);
    let zt = top - 1.0;
    let sw = |z: f32| (lw(z) + 0.3).round() as i32;
    let mut z = foot + 1.0;
    while z < zt {
        let t = (z - foot - 1.0) / (zt - foot - 1.0);
        let reach = (mast_l - 1.5 - boom_end) * (1.0 - t * 0.95);
        let mut l = mast_l - 1.0;
        while l >= mast_l - 1.0 - reach {
            let c = if ((l * 0.7) as i32 + z as i32) % 5 == 0 { shade(sail, 0.9) } else { sail };
            v.put(l.round() as i32, sw(z), z as i32, c, M_SAIL);
            l -= 1.0;
        }
        z += 1.0;
    }
    let jt = if big { top - 3.0 } else { top - 4.0 };
    let mut z = foot + 0.5;
    while z < jt {
        let t = (z - foot) / (jt - foot);
        let reach = (bow_l - mast_l - 2.5) * (1.0 - t) * 0.92;
        let mut l = mast_l + 1.5;
        while l <= mast_l + 1.5 + reach {
            let c = if big && ((z as i32 + l as i32) % 6 == 0) { shade(sail, 0.9) } else { sail };
            v.put(l.round() as i32, sw(z) + 1, z as i32, shade(c, 0.97), M_SAIL);
            l += 1.0;
        }
        z += 1.0;
    }
    let pc = pick(&[rgb(0xc8, 0x3c, 0x30), rgb(0x2e, 0x5a, 0x8a), rgb(0xe8, 0xb8, 0x30), rgb(0x30, 0x34, 0x3c)], var);
    v.put((mast_l - 1.0).round() as i32, lw(top).round() as i32, top as i32, pc, M_PLAIN);
    v.put((mast_l - 2.0).round() as i32, lw(top).round() as i32, top as i32, pc, M_PLAIN);
    v.put(mast_l.round() as i32, lw(top).round() as i32, (top as i32) + 1, white(), M_LAMP);
    v.put((mast_l + 1.0) as i32, -2, foot as i32, red_lamp(), M_PORT);
    v.put((mast_l + 1.0) as i32, 1, foot as i32, green_lamp(), M_STBD);
}

// ----------------------------------------------------------------- ray-casting

/// One heading's picture, before it becomes a sprite.
struct Raster {
    w: usize,
    h: usize,
    ax: i32,
    ay: i32,
    px: Vec<u32>,
    mat: Vec<u8>,
    /// Water pixels just outside the hull at the waterline (relative to the
    /// anchor): the dark contact the hull sits in.
    contact: Vec<(i16, i16)>,
}

fn to_local(dir: usize, x: f32, y: f32) -> (f32, f32) {
    match dir {
        0 => (x, y),
        1 => (y, -x),
        2 => (-x, -y),
        _ => (-y, x),
    }
}

fn to_world(dir: usize, l: f32, w: f32) -> (f32, f32) {
    match dir {
        0 => (l, w),
        1 => (-w, l),
        2 => (-l, -w),
        _ => (w, -l),
    }
}

fn raycast(v: &Vox, dir: usize) -> Raster {
    let (hl, hw) = (v.nl as f32 * 0.5, v.nw as f32 * 0.5);
    let mut minx = f32::MAX;
    let mut maxx = f32::MIN;
    let mut miny = f32::MAX;
    let mut maxy = f32::MIN;
    for &l in &[-hl, hl] {
        for &w in &[-hw, hw] {
            for &z in &[0.0, v.nz as f32] {
                let (x, y) = to_world(dir, l, w);
                let sx = x - y;
                let sy = (x + y) * 0.5 - z;
                minx = minx.min(sx);
                maxx = maxx.max(sx);
                miny = miny.min(sy);
                maxy = maxy.max(sy);
            }
        }
    }
    let margin = 2;
    let ax = (-minx).ceil() as i32 + margin;
    let ay = (-miny).ceil() as i32 + margin;
    let w = (maxx - minx).ceil() as usize + 2 * margin as usize + 1;
    let h = (maxy - miny).ceil() as usize + 2 * margin as usize + 1;
    let mut px = vec![0u32; w * h];
    let mut mat = vec![0u8; w * h];
    let mut depth = vec![f32::MIN; w * h];
    let nlh = v.nl as f32 * 0.5;
    for py in 0..h {
        for pxx in 0..w {
            let sx = pxx as f32 - ax as f32 + 0.5;
            let sy = py as f32 - ay as f32 + 0.5;
            let mut t = 2.0 * sy + 2.0 * v.nz as f32 - 0.01;
            let t_lo = 2.0 * sy;
            while t >= t_lo {
                let x = (t + sx) * 0.5;
                let y = (t - sx) * 0.5;
                let z = t * 0.5 - sy;
                let here = t;
                t -= 1.0;
                let (lc, wc) = to_local(dir, x, y);
                let li = (lc + nlh).floor() as i32;
                let wi = wc.floor() as i32;
                let zi = z.floor() as i32;
                let Some(i) = v.idx(li, wi, zi) else { continue };
                let c = v.col[i];
                if c == 0 {
                    continue;
                }
                // Which visible face did the ray come through?
                let (cl, cw) = (li as f32 + 0.5 - nlh, wi as f32 + 0.5);
                let (cx, cy) = to_world(dir, cl, cw);
                let dx = cx + 0.5 - x;
                let dy = cy + 0.5 - y;
                let dz = zi as f32 + 1.0 - z;
                let m = v.mat[i];
                let (ft, fy, fx) = if m == M_SAIL { (1.0, 0.96, 0.8) } else { (1.07, 0.9, 0.66) };
                let f = if dz <= dx && dz <= dy {
                    ft
                } else if dy <= dx {
                    fy
                } else {
                    fx
                };
                let out = if matches!(m, M_LAMP | M_PORT | M_STBD) { c } else { shade(c, f) };
                let o = py * w + pxx;
                px[o] = out;
                mat[o] = m;
                depth[o] = here;
                break;
            }
        }
    }
    // Thin lines: 3-D segments drawn as one-pixel strokes, depth-tested.
    for ln in &v.lines {
        let dist = ((ln.b.0 - ln.a.0).powi(2) + (ln.b.1 - ln.a.1).powi(2) + (ln.b.2 - ln.a.2).powi(2)).sqrt();
        let n = (dist * 3.0).ceil().max(1.0) as i32;
        for s in 0..=n {
            let u = s as f32 / n as f32;
            let l = ln.a.0 + (ln.b.0 - ln.a.0) * u - nlh;
            let wv = ln.a.1 + (ln.b.1 - ln.a.1) * u;
            let z = ln.a.2 + (ln.b.2 - ln.a.2) * u;
            let (x, y) = to_world(dir, l, wv);
            let sx = x - y;
            let sy = (x + y) * 0.5 - z;
            let pxx = (sx + ax as f32).floor() as i32;
            let py = (sy + ay as f32).floor() as i32;
            if pxx < 0 || py < 0 || pxx >= w as i32 || py >= h as i32 {
                continue;
            }
            let o = py as usize * w + pxx as usize;
            let t = x + y;
            if t + 0.75 >= depth[o] {
                px[o] = ln.c;
                mat[o] = ln.m;
                depth[o] = t;
            }
        }
    }
    // Outline: a dark ring in a shade of the neighbouring colour.
    let snapshot = px.clone();
    for py in 0..h as i32 {
        for pxx in 0..w as i32 {
            let o = py as usize * w + pxx as usize;
            if snapshot[o] != 0 {
                continue;
            }
            let mut best: Option<u32> = None;
            for (dx, dy) in [(0, 1), (1, 0), (-1, 0), (0, -1)] {
                let (nx, ny) = (pxx + dx, py + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let n = snapshot[ny as usize * w + nx as usize];
                if n != 0 {
                    best = Some(n);
                    break;
                }
            }
            if let Some(n) = best {
                px[o] = palette::mix(shade(n, 0.42), rgb(0x1c, 0x22, 0x30), 0.45);
            }
        }
    }
    // Contact ring: water cells one step out from the hull's waterline.
    let mut contact: Vec<(i16, i16)> = Vec::new();
    for li in 0..v.nl {
        for wi in -v.ow..v.ow {
            if v.solid(li, wi, 0) {
                continue;
            }
            let mut near = false;
            for (dl, dw) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1), (1, -1), (-1, 1)] {
                if v.at(li + dl, wi + dw, 0).1 == M_WET {
                    near = true;
                }
            }
            if !near {
                continue;
            }
            let (x, y) = to_world(dir, li as f32 + 0.5 - nlh, wi as f32 + 0.5);
            let cx = (x - y).floor() as i16;
            let cy = ((x + y) * 0.5).floor() as i16;
            for c in [(cx, cy), (cx + 1, cy)] {
                if !contact.contains(&c) {
                    contact.push(c);
                }
            }
        }
    }
    Raster { w, h, ax, ay, px, mat, contact }
}

// ----------------------------------------------------------------- the kit

/// One variant's pictures: day and night, four headings.
pub struct VariantSprites {
    pub day: [Sprite; 4],
    pub night: [Sprite; 4],
}

/// One kind in all its variants.
pub struct KindKit {
    pub kind: BoatKind,
    pub variants: Vec<VariantSprites>,
    /// Per heading: the water ring that darkens round the hull.
    pub contact: [Vec<(i16, i16)>; 4],
    /// Per variant and heading: lamps visible in the picture (dx, dy, kind).
    pub lamps: Vec<[Vec<(i16, i16, u8)>; 4]>,
    /// The most any sprite, contact ring or lamp of this kind reaches from the
    /// anchor: (half-width, up, down). The damage rect is never smaller.
    pub ext: (i32, i32, i32),
}

impl KindKit {
    pub fn bytes(&self) -> usize {
        self.variants
            .iter()
            .map(|v| v.day.iter().chain(v.night.iter()).map(|s| s.bytes()).sum::<usize>())
            .sum::<usize>()
            + self.contact.iter().map(|c| c.capacity() * 4).sum::<usize>()
    }
}

/// The drawn box of a raster: (x0, y0, x1, y1), end exclusive.
fn bbox(r: &Raster) -> (usize, usize, usize, usize) {
    let (mut x0, mut y0, mut x1, mut y1) = (r.w, r.h, 0, 0);
    for y in 0..r.h {
        for x in 0..r.w {
            if r.px[y * r.w + x] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 == 0 {
        (0, 0, 1, 1)
    } else {
        (x0, y0, x1, y1)
    }
}

/// Crop a raster to its drawn box and recolour each pixel with `f`.
fn sprite_of(r: &Raster, f: impl Fn(u32, u8) -> u32) -> Sprite {
    let (x0, y0, x1, y1) = bbox(r);
    let mut s = Sprite::new(x1 - x0, y1 - y0);
    s.set_anchor(r.ax - x0 as i32, r.ay - y0 as i32);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = y * r.w + x;
            let c = r.px[i];
            if c != 0 {
                s.px[(y - y0) * s.w + (x - x0)] = f(c, r.mat[i]);
            }
        }
    }
    s
}

fn night_of(r: &Raster) -> Sprite {
    sprite_of(r, |c, m| match m {
        M_GLASS => rgb(0xff, 0xd8, 0x84),
        M_LAMP => rgb(0xff, 0xfb, 0xe0),
        M_PORT => rgb(0xff, 0x44, 0x34),
        M_STBD => rgb(0x44, 0xff, 0x70),
        _ => {
            let (rr, gg, bb) = (palette::r(c) as f32, palette::g(c) as f32, palette::b(c) as f32);
            rgb((rr * 0.52) as u8, (gg * 0.58) as u8, (bb * 0.76) as u8)
        }
    })
}

fn day_of(r: &Raster) -> Sprite {
    sprite_of(r, |c, m| match m {
        M_LAMP => shade(c, 0.95),
        M_PORT => rgb(0xa0, 0x40, 0x38),
        M_STBD => rgb(0x38, 0x8a, 0x50),
        _ => c,
    })
}

fn build_kind(kind: BoatKind) -> KindKit {
    let mut variants = Vec::with_capacity(VARIANTS);
    let mut lamps = Vec::new();
    let mut contact: [Vec<(i16, i16)>; 4] = Default::default();
    for var in 0..VARIANTS {
        let v = model(kind, var);
        let rs: Vec<Raster> = (0..4).map(|d| raycast(&v, d)).collect();
        if var == 0 {
            for d in 0..4 {
                contact[d] = rs[d].contact.clone();
            }
        }
        let day: [Sprite; 4] = std::array::from_fn(|d| day_of(&rs[d]));
        let night: [Sprite; 4] = std::array::from_fn(|d| night_of(&rs[d]));
        let lp: [Vec<(i16, i16, u8)>; 4] = std::array::from_fn(|d| {
            let r = &rs[d];
            let mut out = Vec::new();
            for (i, &m) in r.mat.iter().enumerate() {
                if matches!(m, M_LAMP | M_PORT | M_STBD) && r.px[i] != 0 {
                    out.push(((i % r.w) as i16 - r.ax as i16, (i / r.w) as i16 - r.ay as i16, m));
                }
            }
            out
        });
        lamps.push(lp);
        variants.push(VariantSprites { day, night });
    }
    let (mut hw, mut up, mut down) = (0i32, 0i32, 0i32);
    for v in &variants {
        for s in v.day.iter().chain(v.night.iter()) {
            hw = hw.max(s.ax).max(s.w as i32 - s.ax);
            up = up.max(s.ay);
            down = down.max(s.h as i32 - s.ay);
        }
    }
    for c in contact.iter().flatten() {
        hw = hw.max(c.0.abs() as i32 + 1);
        up = up.max(-(c.1 as i32) + 1);
        down = down.max(c.1 as i32 + 1);
    }
    KindKit { kind, variants, contact, lamps, ext: (hw, up, down) }
}

static KITS: [OnceLock<KindKit>; KINDS] = [const { OnceLock::new() }; KINDS];

/// The kit for one kind, built the first time anything asks for it and shared
/// by every scene after that (the car kit's rule: build once).
pub fn kit(kind: BoatKind) -> &'static KindKit {
    KITS[kind.index()].get_or_init(|| build_kind(kind))
}

/// Bytes of boat sprites built so far (for the memory report).
pub fn bytes() -> usize {
    KITS.iter().filter_map(|k| k.get()).map(|k| k.bytes()).sum()
}

// ----------------------------------------------------------------- drawing

/// How a boat is placed this frame.
pub struct Pose<'a> {
    pub kind: BoatKind,
    pub variant: u8,
    pub dir: (f32, f32),
    /// Screen position of the boat's centre on the waterline.
    pub sx: i32,
    pub sy: i32,
    pub night: bool,
    /// Seconds, for the shimmer of the foam.
    pub age: f32,
    pub moving: bool,
    /// Per-pixel painter test against the static layer: (key, depth plane).
    pub depth: Option<(u16, &'a [u16])>,
    /// How far each screen row has faded into the distance (0 near, 1 gone):
    /// the boat takes it towards the water colour it is sitting on.
    pub haze: Option<&'a [f32]>,
    /// The colour the haze fades the boat towards: the static layer's own
    /// water under it (0: read the frame, which is only right when the whole
    /// frame is being drawn, not a damaged part of it).
    pub haze_to: Color,
}

/// The tile heading's index, 0..4 (same as `traffic::dir_index`).
pub fn heading(dir: (f32, f32)) -> usize {
    crate::traffic::dir_index(dir)
}

/// How much of the haze a row takes (0 when there is none).
fn haze_at(haze: Option<&[f32]>, y: i32) -> f32 {
    match haze {
        Some(rows) if y >= 0 => rows.get(y as usize).copied().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// A blended pixel whose coverage fades with the distance haze.
fn pb(fb: &mut Framebuffer, haze: Option<&[f32]>, x: i32, y: i32, c: Color, cov: f32) {
    let a = haze_at(haze, y);
    fb.put_blended(x, y, c, cov * (1.0 - a));
}

/// The hull, drawn the way the ground under it is: each pixel mixed towards the
/// air colour by the row's haze amount, so a far boat fades into the horizon
/// band instead of floating on it.
pub(crate) fn blit_hazed(fb: &mut Framebuffer, s: &Sprite, x0: i32, y0: i32, depth: Option<(u16, &[u16])>, rows: &[f32], bg: Color) {
    let clip = fb.clip().intersect(&crate::framebuffer::Rect::from_size(fb.w, fb.h));
    let w = fb.w;
    for sy in 0..s.h as i32 {
        let y = y0 + sy;
        if y < clip.y0 || y >= clip.y1 {
            continue;
        }
        let amount = rows.get(y as usize).copied().unwrap_or(0.0);
        for sx in 0..s.w as i32 {
            let x = x0 + sx;
            if x < clip.x0 || x >= clip.x1 {
                continue;
            }
            let c = s.px[sy as usize * s.w + sx as usize];
            if c == 0 {
                continue;
            }
            let i = y as usize * w + x as usize;
            if let Some((key, plane)) = depth {
                if plane[i] > key {
                    continue;
                }
            }
            let out = if amount > 0.0 { palette::mix(c, bg, amount) } else { c };
            fb.px_mut()[i] = out;
        }
    }
}

/// Draw the boat: contact shadow on the water, bow wave and wake, the hull,
/// the lamps.
pub fn draw(fb: &mut Framebuffer, p: &Pose) {
    let k = kit(p.kind);
    let d = heading(p.dir);
    let var = &k.variants[p.variant as usize % VARIANTS];
    let s = if p.night { &var.night[d] } else { &var.day[d] };
    let (sx, sy) = (p.sx, p.sy);
    // A boat is on the water, and the water fades into the distance haze: so do
    // its wake, its contact ring and its hull.
    let water = if p.night { rgb(0x06, 0x12, 0x22) } else { rgb(0x10, 0x34, 0x44) };
    for &(dx, dy) in &k.contact[d] {
        pb(fb, p.haze, sx + dx as i32, sy + dy as i32, water, if p.night { 0.5 } else { 0.4 });
    }
    let (hx, hy) = (p.dir.0, p.dir.1);
    let (hxn, hyn) = {
        let n = (hx * hx + hy * hy).sqrt().max(1e-3);
        (hx / n, hy / n)
    };
    let foam = if p.night { rgb(0x9a, 0xb4, 0xc8) } else { rgb(0xe8, 0xf2, 0xf6) };
    let proj = |wx: f32, wy: f32| -> (i32, i32) { (sx + (wx - wy).round() as i32, sy + ((wx + wy) * 0.5).round() as i32) };
    let half = p.kind.len() * 0.5;
    let (px_, py_) = (-hyn, hxn);
    let power = p.kind.wake_power();
    let shimmer = |i: i32| 0.5 + 0.5 * ((p.age * 3.0 + i as f32 * 1.7).sin());
    if p.moving {
        let bb = p.kind.bow_beam();
        for side in [-1.0f32, 1.0] {
            for i in 0..4 {
                let along = half - 0.5 - i as f32 * 1.6;
                let across = (bb + 0.7 + i as f32 * 0.55) * side;
                let (x, y) = proj(hxn * along + px_ * across, hyn * along + py_ * across);
                pb(fb, p.haze, x, y, foam, (0.85 - i as f32 * 0.15) * (0.8 + 0.2 * shimmer(i)));
                pb(fb, p.haze, x + 1, y, foam, (0.5 - i as f32 * 0.12).max(0.0));
            }
        }
        let (n, step) = p.kind.wake();
        for i in 1..=n {
            let along = -half - i as f32 * step;
            let across = 0.8 + i as f32 * 0.5 * power;
            let cov = (0.5 - i as f32 / (n as f32 + 2.0) * 0.45) * (0.6 + 0.4 * shimmer(i + 3));
            for side in [-1.0f32, 1.0] {
                let (x, y) = proj(hxn * along + px_ * across * side, hyn * along + py_ * across * side);
                // B92: a darker trough under each crest, so the wake still
                // reads on the pale turquoise shallows of the new water.
                pb(fb, p.haze, x, y + 1, water, cov * 0.55);
                pb(fb, p.haze, x + 1, y + 1, water, cov * 0.30);
                pb(fb, p.haze, x, y, foam, (cov * 1.5 * power.min(1.2)).min(0.8));
                pb(fb, p.haze, x + 1, y, foam, cov * 0.7);
            }
            if i <= n / 2 {
                let (x, y) = proj(hxn * (-half - i as f32 * 1.1), hyn * (-half - i as f32 * 1.1));
                pb(fb, p.haze, x, y, foam, cov * 0.8);
            }
        }
    }
    let x0 = sx - s.ax;
    let y0 = sy - s.ay;
    let bg_under = if p.haze_to != 0 {
        p.haze_to
    } else {
        fb.px()[sy.clamp(0, fb.h as i32 - 1) as usize * fb.w + sx.clamp(0, fb.w as i32 - 1) as usize]
    };
    match (p.haze, p.depth) {
        (None, Some((key, depth))) => fb.blit_sprite_depth(s, x0, y0, key, depth),
        (None, None) => fb.blit_sprite(s, x0, y0),
        (Some(rows), d) => {
            let (bx, by) = (sx.clamp(0, fb.w as i32 - 1) as usize, sy.clamp(0, fb.h as i32 - 1) as usize);
            let bg = bg_under;
            let _ = (bx, by);
            blit_hazed(fb, s, x0, y0, d, rows, bg)
        }
    }
    if p.night {
        for &(dx, dy, m) in &k.lamps[p.variant as usize % VARIANTS][d] {
            let c = match m {
                M_PORT => rgb(0xff, 0x40, 0x30),
                M_STBD => rgb(0x40, 0xff, 0x70),
                _ => rgb(0xff, 0xf4, 0xc8),
            };
            for (ox, oy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                pb(fb, p.haze, sx + dx as i32 + ox, sy + dy as i32 + oy, c, 0.22);
            }
        }
        if matches!(
            p.kind,
            BoatKind::Ferry
                | BoatKind::CarFerry
                | BoatKind::Pontoon
                | BoatKind::Fishing
                | BoatKind::MotorYacht
                | BoatKind::Sloop
                | BoatKind::Launch
                | BoatKind::Coaster
                | BoatKind::Container
        ) {
            let w = (p.kind.len() * 0.35) as i32;
            for i in -w..=w {
                pb(fb, p.haze, sx + i, sy + (p.kind.len() * 0.15) as i32 + 2, rgb(0xff, 0xd8, 0x84), 0.07);
            }
        }
    }
}

/// A small bob for a boat on the water: 0 or -1 px, slow. Big hulls do not.
pub fn bob(age: f32, phase: f32, kind: BoatKind) -> i32 {
    if matches!(
        kind,
        BoatKind::Container | BoatKind::Coaster | BoatKind::Barge | BoatKind::Ferry | BoatKind::CarFerry | BoatKind::Pontoon
    ) {
        return 0;
    }
    if (age * 0.9 + phase).sin() > 0.55 {
        -1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(s: &Sprite) -> usize {
        s.px.iter().filter(|c| **c != 0).count()
    }

    fn distinct(s: &Sprite) -> usize {
        let mut seen: Vec<u32> = Vec::new();
        for &c in &s.px {
            if c != 0 && !seen.contains(&c) {
                seen.push(c);
            }
        }
        seen.len()
    }

    #[test]
    fn every_kind_has_every_direction() {
        for kind in BoatKind::ALL {
            let k = kit(kind);
            assert_eq!(k.variants.len(), VARIANTS);
            for (vi, v) in k.variants.iter().enumerate() {
                for d in 0..4 {
                    for (name, s) in [("day", &v.day[d]), ("night", &v.night[d])] {
                        assert!(opaque(s) > 25, "{kind:?} v{vi} d{d} {name}: only {} pixels", opaque(s));
                        assert!(distinct(s) >= 5, "{kind:?} v{vi} d{d} {name}: flat, {} colours", distinct(s));
                    }
                }
            }
        }
    }

    #[test]
    fn headings_look_different() {
        for kind in BoatKind::ALL {
            let k = kit(kind);
            let v = &k.variants[0];
            assert_ne!(v.day[0].px, v.day[1].px, "{kind:?}");
            assert_ne!(v.day[0].px, v.day[2].px, "{kind:?}");
        }
    }

    #[test]
    fn variants_differ() {
        for kind in BoatKind::ALL {
            let k = kit(kind);
            let mut same = 0;
            for i in 1..VARIANTS {
                if k.variants[0].day[0].px == k.variants[i].day[0].px {
                    same += 1;
                }
            }
            assert!(same <= 1, "{kind:?}: {same} variants identical to variant 0");
        }
    }

    #[test]
    fn the_kit_is_built_once() {
        let a = kit(BoatKind::Tug) as *const KindKit;
        let b = kit(BoatKind::Tug) as *const KindKit;
        assert_eq!(a, b);
        let before = bytes();
        let _ = kit(BoatKind::Tug);
        assert_eq!(before, bytes());
    }

    #[test]
    fn the_kit_costs_a_few_megabytes() {
        for kind in BoatKind::ALL {
            let _ = kit(kind);
        }
        let mb = bytes() as f32 / 1.0e6;
        assert!(mb < 5.0, "the boat kit holds {mb:.1} MB");
    }

    #[test]
    fn the_damage_extent_covers_every_sprite() {
        for kind in BoatKind::ALL {
            let (hw, up, down) = kind.extent();
            let k = kit(kind);
            for v in &k.variants {
                for s in v.day.iter() {
                    assert!(s.ax <= hw && s.w as i32 - s.ax <= hw, "{kind:?}: width {} anchor {} vs {hw}", s.w, s.ax);
                    assert!(s.ay <= up, "{kind:?}: {} above the anchor vs {up}", s.ay);
                    assert!(s.h as i32 - s.ay <= down, "{kind:?}: {} below vs {down}", s.h as i32 - s.ay);
                }
            }
        }
    }

    #[test]
    fn a_far_boat_fades_at_least_as_much_as_the_water_beside_it() {
        let (w, h) = (120usize, 90usize);
        let air = palette::rgb(0xc8, 0xc4, 0xa8);
        let water = palette::rgb(0x3a, 0x86, 0x9a);
        for amount in [0.35f32, 0.7, 1.0] {
            let rows = vec![amount; h];
            let mut fb = Framebuffer::new(w, h);
            fb.clear(palette::mix(water, air, amount));
            let beside = fb.px()[10];
            let pose = Pose {
                kind: BoatKind::Ferry,
                variant: 0,
                dir: (1.0, 0.0),
                sx: 60,
                sy: 50,
                night: false,
                age: 0.0,
                moving: true,
                depth: None,
                haze: Some(&rows),
                haze_to: 0,
            };
            draw(&mut fb, &pose);
            // Distance from the water beside the boat: no drawn pixel may stand
            // out from it more than the unhazed sprite did, scaled by what the
            // haze leaves (the boat is blended at least as much as the water).
            let dist = |c: u32| {
                let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
                d(palette::r(c), palette::r(beside)) + d(palette::g(c), palette::g(beside)) + d(palette::b(c), palette::b(beside))
            };
            let sp = &kit(BoatKind::Ferry).variants[0].day[0];
            let mut worst = dist(rgb(0xe8, 0xf2, 0xf6));
            for &c in &sp.px {
                if c != 0 {
                    worst = worst.max(dist(c));
                }
            }
            let allowed = (worst as f32 * (1.0 - amount)).ceil() as i32 + 3;
            for (i, &c) in fb.px().iter().enumerate() {
                assert!(dist(c) <= allowed, "amount {amount}: pixel {i} is {} from the water, allowed {allowed}", dist(c));
            }
            if amount >= 1.0 {
                assert!(fb.px().iter().all(|&c| dist(c) == 0), "a boat in full haze must vanish");
            }
        }
    }

    #[test]
    fn decode_round_trips() {
        for (i, k) in SMALL.iter().enumerate() {
            let (kind, v) = BoatKind::decode(Class::Boat, (i as u8) | (5 << 5));
            assert_eq!(kind, *k);
            assert_eq!(v, 5);
        }
    }
}
