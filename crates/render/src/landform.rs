//! B46d (attempt 4): landforms as hand-built pixel-art objects.
//!
//! A mesa, butte, mountain or cone is a *low-poly model*: a few dozen flat
//! facets, each a straight-edged triangle with one material. Every facet gets
//! exactly one colour from a 7-step hand-picked ramp, chosen by how it faces the
//! caster of the world's light (D74/B60) -- lit, mid or shadow. There is no
//! per-pixel shading: a face is a flat colour, and the only detail inside it is
//! deliberate (strata lines on a cliff, a few cracks, scattered boulders and
//! trees). The silhouette is therefore made of straight iso-ish edges, like the
//! buildings, and the crisp facet seams are the "line work".
//!
//! Rasterisation is a z-buffered triangle fill in the engine's own projection
//! (`sx = (x-y)*HALF_W`, `sy = (x+y)*HALF_H - z`), so a sprite is painter-correct
//! by construction and cheap to re-bake when the light moves (a few hundred
//! triangles).

use crate::biome;
use crate::framebuffer::Sprite;
use crate::light::Caster;
use crate::palette::{self, Color, TRANSPARENT};
use map::iso_consts::{HALF_H, HALF_W};
use map::Biome;

// ---------------------------------------------------------------------------
// public surface
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    Mesa,
    Butte,
    Mountain,
    Cone,
}

impl Form {
    /// The three landforms each land grows.
    pub fn for_biome(b: Biome) -> [Form; 3] {
        match b {
            Biome::Desert => [Form::Mesa, Form::Butte, Form::Cone],
            Biome::Tundra => [Form::Mountain, Form::Cone, Form::Butte],
            Biome::Tropical => [Form::Cone, Form::Mountain, Form::Mesa],
            Biome::Alpine => [Form::Mountain, Form::Mountain, Form::Mountain],
            Biome::Savanna => [Form::Butte, Form::Butte, Form::Mesa],
            Biome::Highlands => [Form::Mountain, Form::Mountain, Form::Mountain],
            _ => [Form::Mountain, Form::Mesa, Form::Butte],
        }
    }
}

/// Where the light comes from, as a unit vector (tile-space x, y, up).
#[derive(Clone, Copy, Debug)]
pub struct Sun {
    pub dir: [f32; 3],
}

impl Sun {
    /// From the engine's caster; `None` = a flat daylight look from the front-left.
    pub fn from_caster(c: Option<Caster>) -> Self {
        match c {
            Some(c) => {
                let a = c.alt.clamp(8.0, 85.0).to_radians();
                Self {
                    dir: [c.toward.0 * a.cos(), c.toward.1 * a.cos(), a.sin()],
                }
            }
            None => Self::default_day(),
        }
    }
    pub fn default_day() -> Self {
        let a = 48f32.to_radians();
        let (tx, ty) = (0.30f32, 0.954f32);
        Self {
            dir: [tx * a.cos(), ty * a.cos(), a.sin()],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub form: Form,
    pub biome: Biome,
    pub seed: u64,
    /// Footprint radius in tiles (the foot, scree apron included).
    pub radius: f32,
    /// Summit height in px above the foot.
    pub height: f32,
}

/// A baked landform: the sprite, with its anchor on the footprint centre at the
/// foot's level.
pub struct Baked {
    /// B69b: the trees that sway, kept out of the sprite.
    pub trees: Vec<SwayTree>,
    pub sprite: Sprite,
    /// Per pixel: x + y (local tile space) of the surface drawn there; NaN = empty.
    pub depth: Vec<f32>,
}

/// B69b: one tree of a flank that sways: where it stands in the sprite, how far
/// along the painter's diagonals (`depth`, local x + y) and the pixels that
/// survived the nearer facets, relative to its foot.
#[derive(Clone)]
pub struct SwayTree {
    pub species: map::TreeSpecies,
    pub bx: i32,
    pub by: i32,
    pub depth: f32,
    pub px: Vec<(i16, i16, crate::palette::Color)>,
}

/// The model: facets and tree stamps, independent of the light.
pub struct Model {
    mesh: Mesh,
    veg: Vec<Veg>,
    st: Style,
    seed: u64,
    biome: Biome,
}

/// Bake a landform for a light.
pub fn bake(spec: &Spec, sun: &Sun) -> Baked {
    let m = model(spec);
    render_model(&m, sun)
}

/// Build the model.
pub fn model(spec: &Spec) -> Model {
    let mut m = Mesh::default();
    let mut rng = Rng::new(spec.seed ^ 0x1a2f_0b46);
    let mut veg: Vec<Veg> = Vec::new();
    let st = Style::of(spec.biome, spec.form);
    match spec.form {
        Form::Mesa => build_mesa(&mut m, &mut veg, &mut rng, spec, &st, false),
        Form::Butte => build_mesa(&mut m, &mut veg, &mut rng, spec, &st, true),
        Form::Mountain => build_mountain(&mut m, &mut veg, &mut rng, spec, &st),
        Form::Cone => build_cone(&mut m, &mut veg, &mut rng, spec, &st),
    }
    scatter_boulders(&mut m, &mut rng, spec, &st);
    if matches!(spec.form, Form::Mountain | Form::Cone) {
        plant_slopes(&m, &mut veg, &mut rng, spec, &st);
    }
    // Plant every tree on the actual surface (at ground level beyond the mesh,
    // where the real tiles show through).
    let reach = spec.radius * 0.8;
    let veg: Vec<Veg> = veg
        .into_iter()
        .filter_map(|mut v| {
            match ground_zn(&m, v.x, v.y) {
                Some((z, _)) => v.z = z,
                None if (v.x * v.x + v.y * v.y).sqrt() > reach => v.z = 0.0,
                None => return None,
            }
            Some(v)
        })
        .collect();
    Model { mesh: m, veg, st, seed: spec.seed, biome: spec.biome }
}

/// Rasterise a model for a light.
pub fn render_model(m: &Model, sun: &Sun) -> Baked {
    render(&m.mesh, &m.veg, &m.st, m.biome, sun, m.seed)
}

// ---------------------------------------------------------------------------
// materials
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mat {
    /// Steep rock face: banded.
    Cliff,
    /// Gently sloping rock (ledges, caps, ridge shoulders).
    Rock,
    /// Loose rubble at the foot.
    Scree,
    /// Vegetated ground.
    Veg,
    Snow,
    /// The land's own ground colour: the foot of a mountain or cone.
    Ground,
    /// The land's ground colour, drawn: the lowest ring of facets, so a mountain's
    /// foot has no edge of its own (the real tiles and the trees carry it on).
    Fringe,
    /// The cap's own ground (grass, sand, ash).
    Top,
    /// Dark rock (basalt, wet).
    Dark,
}

/// Per-land style: which of the land's ramps the landform is built from.
struct Style {
    cliff: [Color; 7],
    rock: [Color; 7],
    scree: [Color; 7],
    top: [Color; 7],
    veg: [Color; 7],
    dark: [Color; 7],
    snow: [Color; 7],
    ground: [Color; 7],
    /// Crown colours (dark, mid, light) for the broadleaf stamps.
    leaf: [Color; 3],
    /// How thick the stamped trees run (1 = the usual).
    dense: f32,
    /// The ramp step the foot takes (the ground beside it, as drawn).
    fringe: i32,
    /// 0 = no snow; else z fraction of the summit where it starts.
    snowline: f32,
    /// 0 = none; else z fraction of the summit below which it grows.
    treeline: f32,
    /// Strata amplitude: 0 = none (granite), 1 = strong (sandstone).
    strata: f32,
    /// Tree style: 0 none, 1 pine, 2 round broadleaf, 3 palm-ish round, 4 scrub.
    tree: u8,
    /// The cap's vegetation: stamp trees on the plateau.
    cap_veg: bool,
}

const SNOW: [Color; 7] = [
    palette::rgb(0x7b, 0x8f, 0xb0),
    palette::rgb(0x93, 0xa7, 0xc4),
    palette::rgb(0xad, 0xc0, 0xd8),
    palette::rgb(0xc4, 0xd4, 0xe6),
    palette::rgb(0xd8, 0xe4, 0xf0),
    palette::rgb(0xe9, 0xf1, 0xf8),
    palette::rgb(0xf7, 0xfb, 0xff),
];
const GRANITE: [Color; 7] = [
    palette::rgb(0x3a, 0x3f, 0x4b),
    palette::rgb(0x4a, 0x50, 0x5e),
    palette::rgb(0x5c, 0x63, 0x72),
    palette::rgb(0x70, 0x78, 0x87),
    palette::rgb(0x86, 0x8e, 0x9c),
    palette::rgb(0x9d, 0xa5, 0xb1),
    palette::rgb(0xb4, 0xbb, 0xc5),
];
const BASALT: [Color; 7] = [
    palette::rgb(0x25, 0x24, 0x2c),
    palette::rgb(0x31, 0x30, 0x39),
    palette::rgb(0x3f, 0x3e, 0x49),
    palette::rgb(0x4f, 0x4e, 0x5a),
    palette::rgb(0x61, 0x60, 0x6c),
    palette::rgb(0x75, 0x74, 0x80),
    palette::rgb(0x8a, 0x89, 0x95),
];
const JUNGLE: [Color; 7] = [
    palette::rgb(0x16, 0x30, 0x1a),
    palette::rgb(0x1e, 0x40, 0x20),
    palette::rgb(0x27, 0x52, 0x26),
    palette::rgb(0x31, 0x64, 0x2c),
    palette::rgb(0x3d, 0x76, 0x33),
    palette::rgb(0x4b, 0x8a, 0x3b),
    palette::rgb(0x5c, 0x9e, 0x47),
];
const PINE: [Color; 7] = [
    palette::rgb(0x17, 0x30, 0x2a),
    palette::rgb(0x1e, 0x3d, 0x33),
    palette::rgb(0x27, 0x4b, 0x3b),
    palette::rgb(0x31, 0x5a, 0x43),
    palette::rgb(0x3d, 0x6b, 0x4a),
    palette::rgb(0x4b, 0x7c, 0x52),
    palette::rgb(0x5c, 0x8e, 0x5b),
];

impl Style {
    fn of(b: Biome, f: Form) -> Self {
        let mut st = Self::base(b, f);
        if matches!(f, Form::Mountain | Form::Cone) {
            st.strata = 0.0;
        }
        st
    }
    fn base(b: Biome, f: Form) -> Self {
        let g = biome::look(b);
        match b {
            Biome::Desert => Style {
                cliff: g.earth,
                rock: g.cliff_rock,
                scree: g.scree,
                top: g.rock,
                veg: g.dry_grass,
                dark: g.earth,
                snow: SNOW,
                ground: g.sand,
                leaf: g.leaf,
                dense: 1.0,
                fringe: 2,
                snowline: 0.0,
                treeline: 0.0,
                strata: 1.0,
                tree: 4,
                cap_veg: false,
            },
            Biome::Tundra => Style {
                cliff: GRANITE,
                rock: GRANITE,
                scree: g.scree,
                top: SNOW,
                veg: g.upland,
                dark: if f == Form::Cone { BASALT } else { GRANITE },
                snow: SNOW,
                ground: SNOW,
                leaf: g.leaf,
                dense: 1.0,
                fringe: 5,
                snowline: if f == Form::Butte { 0.55 } else if f == Form::Cone { 0.34 } else { 0.42 },
                treeline: 0.14,
                strata: 0.35,
                tree: 1,
                cap_veg: false,
            },
            Biome::Tropical => Style {
                cliff: [
                    palette::rgb(0x3c, 0x38, 0x36),
                    palette::rgb(0x4b, 0x46, 0x42),
                    palette::rgb(0x5d, 0x57, 0x50),
                    palette::rgb(0x71, 0x69, 0x5e),
                    palette::rgb(0x86, 0x7d, 0x6e),
                    palette::rgb(0x9b, 0x91, 0x7f),
                    palette::rgb(0xb0, 0xa5, 0x90),
                ],
                rock: g.rock,
                scree: g.scree,
                top: g.grass,
                veg: JUNGLE,
                dark: BASALT,
                snow: SNOW,
                ground: g.grass,
                leaf: [palette::rgb(0x3f,0x86,0x3a), palette::rgb(0x5a,0xa8,0x44), palette::rgb(0x86,0xc8,0x5a)],
                dense: 1.7,
                fringe: 3,
                snowline: 0.0,
                treeline: if f == Form::Cone { 0.85 } else { 0.9 },
                strata: 0.6,
                tree: 3,
                cap_veg: true,
            },
            // B46e: a kopje: a flat-topped red-rock stack standing out of the
            // golden grass, a scrub-covered cap.
            Biome::Savanna => Style {
                cliff: g.earth,
                rock: g.cliff_rock,
                scree: g.scree,
                top: g.dry_grass,
                veg: g.dry_grass,
                dark: g.earth,
                snow: SNOW,
                ground: g.grass,
                leaf: g.leaf,
                dense: 0.7,
                fringe: 2,
                snowline: 0.0,
                treeline: 0.0,
                strata: 1.0,
                tree: 4,
                cap_veg: true,
            },
            // B46e: a tor: a small granite stack on a heather hilltop.
            Biome::Highlands => Style {
                cliff: GRANITE,
                rock: GRANITE,
                scree: g.scree,
                top: GRANITE,
                veg: g.upland,
                dark: GRANITE,
                snow: SNOW,
                ground: g.meadow,
                leaf: g.leaf,
                dense: 0.6,
                fringe: 3,
                snowline: 0.0,
                treeline: 0.5,
                strata: 0.0,
                tree: 0,
                cap_veg: false,
            },
            // B46e: an alpine peak: granite flanks, a forest foot, and the snow
            // coming down well below the summit.
            Biome::Alpine => Style {
                cliff: GRANITE,
                rock: GRANITE,
                scree: g.scree,
                top: SNOW,
                veg: PINE,
                dark: GRANITE,
                snow: SNOW,
                ground: g.meadow,
                leaf: g.leaf,
                dense: 1.2,
                fringe: 3,
                snowline: 0.44,
                treeline: 0.26,
                strata: 0.0,
                tree: 1,
                cap_veg: false,
            },
            _ => Style {
                cliff: GRANITE,
                rock: GRANITE,
                scree: g.scree,
                top: g.dry_grass,
                veg: PINE,
                dark: GRANITE,
                snow: SNOW,
                ground: g.grass,
                leaf: g.leaf,
                dense: 1.0,
                fringe: 3,
                snowline: if f == Form::Mountain { 0.74 } else { 0.0 },
                treeline: 0.2,
                strata: 0.5,
                tree: 2,
                cap_veg: f != Form::Mountain,
            },
        }
    }
    fn ramp(&self, m: Mat) -> &[Color; 7] {
        match m {
            Mat::Cliff => &self.cliff,
            Mat::Rock => &self.rock,
            Mat::Scree => &self.scree,
            Mat::Veg => &self.veg,
            Mat::Snow => &self.snow,
            Mat::Ground | Mat::Fringe => &self.ground,
            Mat::Top => &self.top,
            Mat::Dark => &self.dark,
        }
    }
}

// ---------------------------------------------------------------------------
// tiny RNG
// ---------------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn new(s: u64) -> Self {
        Self(s.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn r(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
}

fn hash(x: i32, y: i32, s: u64) -> u32 {
    let mut h = (x as u32 as u64).wrapping_mul(0x9E37_79B1) ^ (y as u32 as u64).wrapping_mul(0x85EB_CA77) ^ s;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    (h ^ (h >> 15)) as u32
}

// ---------------------------------------------------------------------------
// mesh
// ---------------------------------------------------------------------------

type V3 = [f32; 3];

#[derive(Clone, Copy)]
struct Tri {
    p: [V3; 3],
    mat: Mat,
    /// Ramp offset applied after lighting (a boulder is a touch lighter).
    bias: i8,
}

#[derive(Default)]
struct Mesh {
    tris: Vec<Tri>,
}

impl Mesh {
    fn tri(&mut self, a: V3, b: V3, c: V3, mat: Mat) {
        self.tris.push(Tri { p: [a, b, c], mat, bias: 0 });
    }
    fn quad(&mut self, a: V3, b: V3, c: V3, d: V3, mat: Mat) {
        // a-b-c-d around; split on the shorter diagonal in z so the facets stay
        // convex-looking.
        if (a[2] - c[2]).abs() <= (b[2] - d[2]).abs() {
            self.tri(a, b, c, mat);
            self.tri(a, c, d, mat);
        } else {
            self.tri(a, b, d, mat);
            self.tri(b, c, d, mat);
        }
    }
}

/// One level of midpoint subdivision with displaced midpoints (shared edges move
/// identically, so the mesh stays watertight). Keeps the facets flat and
/// straight-edged but doubles how many of them there are: that is what turns a
/// 10-sided cone into a mountainside. Edges lying on the ground stay on it.
fn subdivide(m: &mut Mesh, amp: f32, seed: u64, only: &dyn Fn(Mat) -> bool) {
    fn key(a: V3, b: V3) -> (i32, i32, i32, i32, i32, i32) {
        let q = |v: f32| (v * 64.0).round() as i32;
        let (a, b) = if (q(a[0]), q(a[1]), q(a[2])) <= (q(b[0]), q(b[1]), q(b[2])) { (a, b) } else { (b, a) };
        (q(a[0]), q(a[1]), q(a[2]), q(b[0]), q(b[1]), q(b[2]))
    }
    // An edge may only move if every facet on it is being subdivided; otherwise
    // the neighbour that is not would be left with a crack along it.
    let mut shared: std::collections::HashMap<_, (u32, u32)> = std::collections::HashMap::new();
    for t in &m.tris {
        for e in 0..3 {
            let c = shared.entry(key(t.p[e], t.p[(e + 1) % 3])).or_insert((0, 0));
            c.0 += 1;
            if only(t.mat) {
                c.1 += 1;
            }
        }
    }
    let mid = |a: V3, b: V3| -> V3 {
        let k = key(a, b);
        let free = shared.get(&k).map_or(true, |c| c.0 == c.1);
        let h = hash(k.0 ^ k.3.wrapping_mul(31), k.1 ^ k.4.wrapping_mul(17) ^ k.2, seed ^ k.5 as u64);
        let r = (h & 0xffff) as f32 / 65535.0 * 2.0 - 1.0;
        let r2 = ((h >> 16) & 0xffff) as f32 / 65535.0 * 2.0 - 1.0;
        let len = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt() * 36.0 + (a[2] - b[2]).abs() * 0.5;
        let on_ground = (a[2] < 0.5 && b[2] < 0.5) || !free;
        let mut p = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5, (a[2] + b[2]) * 0.5];
        if !on_ground {
            p[2] += r * amp * len * 0.12;
            p[0] += r2 * amp * 0.05;
            p[1] += r * amp * 0.04;
        }
        p
    };
    let old = std::mem::take(&mut m.tris);
    for t in old {
        if !only(t.mat) {
            m.tris.push(t);
            continue;
        }
        let [a, b, c] = t.p;
        let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
        for p in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
            m.tris.push(Tri { p, mat: t.mat, bias: t.bias });
        }
    }
}

/// After subdivision, decide snow / forest per facet from its own height, so the
/// snowline is a ragged run of straight facet edges instead of a ring.
fn assign_mountain(m: &mut Mesh, s: &Spec, st: &Style, seed: u64) {
    let h = s.height;
    for t in &mut m.tris {
        if !matches!(t.mat, Mat::Rock | Mat::Cliff | Mat::Snow | Mat::Veg) {
            continue;
        }
        let n = normal(t);
        let cz = (t.p[0][2] + t.p[1][2] + t.p[2][2]) / 3.0;
        let cx = (t.p[0][0] + t.p[1][0] + t.p[2][0]) / 3.0;
        let cy = (t.p[0][1] + t.p[1][1] + t.p[2][1]) / 3.0;
        let j = (hash((cx * 50.0) as i32, (cy * 50.0) as i32, seed) % 1000) as f32 / 1000.0 - 0.5;
        let zf = cz / h + j * 0.10;
        t.mat = if st.snowline > 0.0 && zf > st.snowline && n[2] > 0.28 {
            Mat::Snow
        } else if st.treeline > 0.0 && zf < st.treeline && n[2] > 0.4 {
            Mat::Veg
        } else if n[2] < 0.55 {
            Mat::Cliff
        } else {
            Mat::Rock
        };
    }
}

/// Tree stamp request.
struct Veg {
    x: f32,
    y: f32,
    z: f32,
    species: map::TreeSpecies,
    /// 0 small, 1 large (the engine's own two sizes).
    sz: u8,
    variant: u8,
    jitter: f32,
}

/// One of the land's own trees, for a spot on the landform.
fn mk_veg(st: &Style, rng: &mut Rng, x: f32, y: f32, big: bool) -> Veg {
    use map::TreeSpecies as T;
    let r = rng.f();
    let species = match st.tree {
        1 => if r < 0.55 { T::Spruce } else if r < 0.85 { T::Fir } else { T::Pine },
        2 => if r < 0.45 { T::Spruce } else if r < 0.75 { T::Pine } else { T::Fir },
        3 => if r < 0.55 { T::Beech } else if r < 0.85 { T::FanPalm } else { T::Flamboyant },
        _ => if r < 0.7 { T::Joshua } else { T::Cactus },
    };
    Veg { x, y, z: 0.0, species, sz: big as u8, variant: (rng.next() & 0xff) as u8, jitter: rng.f() }
}

/// A rock box (a fallen block, a boulder): a lit top and two sides plus the
/// faces behind, as a convex low-poly prism.
fn boulder(m: &mut Mesh, c: [f32; 3], rx: f32, ry: f32, h: f32, rot: f32, mat: Mat, bias: i8) {
    // a squashed octagon pillar with a smaller top: 8 sides read as a rounded
    // block at the size of a few pixels.
    let n = 6;
    let (s, co) = rot.sin_cos();
    let mut bot = Vec::new();
    let mut top = Vec::new();
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let (ax, ay) = (a.cos() * rx, a.sin() * ry);
        let (bx, by) = (ax * co - ay * s, ax * s + ay * co);
        bot.push([c[0] + bx, c[1] + by, c[2]]);
        top.push([c[0] + bx * 0.72, c[1] + by * 0.72, c[2] + h]);
    }
    let t0 = m.tris.len();
    for i in 0..n {
        let j = (i + 1) % n;
        m.quad(bot[i], bot[j], top[j], top[i], mat);
    }
    let ctr = [c[0], c[1], c[2] + h];
    for i in 0..n {
        let j = (i + 1) % n;
        m.tri(top[i], top[j], ctr, mat);
    }
    for t in &mut m.tris[t0..] {
        t.bias = bias;
    }
}

fn polar(r: f32, a: f32) -> (f32, f32) {
    (r * a.cos(), r * a.sin())
}

// ---------------------------------------------------------------------------
// the forms
// ---------------------------------------------------------------------------

/// A stack of rings sharing one set of angles; joins consecutive rings with a
/// quad strip. Each ring = (z, radius scale, jitter, material of the strip *below*).
struct RingSpec {
    z: f32,
    r: f32,
    j: f32,
    mat: Mat,
}

fn ring_strips(
    m: &mut Mesh,
    rng: &mut Rng,
    angles: &[f32],
    base_r: &[f32],
    squash: (f32, f32),
    rings: &[RingSpec],
) -> Vec<Vec<V3>> {
    let n = angles.len();
    let mut out: Vec<Vec<V3>> = Vec::new();
    for rs in rings {
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            let rr = base_r[i] * rs.r * (1.0 + rng.r(-rs.j, rs.j));
            let (x, y) = polar(rr, angles[i]);
            v.push([x * squash.0, y * squash.1, rs.z + rng.r(-rs.j, rs.j) * 4.0]);
        }
        out.push(v);
    }
    for k in 0..rings.len() - 1 {
        for i in 0..n {
            let j = (i + 1) % n;
            let (a, b) = (&out[k], &out[k + 1]);
            m.quad(a[i], a[j], b[j], b[i], rings[k + 1].mat);
        }
    }
    out
}

/// Angles with jitter, base radius with a couple of bays (notches).
fn outline(rng: &mut Rng, n: usize, notches: usize, notch_depth: f32) -> (Vec<f32>, Vec<f32>) {
    let phase = rng.r(0.0, 6.28);
    let mut ang: Vec<f32> = (0..n)
        .map(|i| phase + (i as f32 + rng.r(-0.22, 0.22)) / n as f32 * std::f32::consts::TAU)
        .collect();
    ang.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut r: Vec<f32> = (0..n).map(|_| rng.r(0.84, 1.0)).collect();
    for _ in 0..notches {
        let i = (rng.next() % n as u64) as usize;
        r[i] *= 1.0 - notch_depth;
    }
    (ang, r)
}

fn build_mesa(m: &mut Mesh, veg: &mut Vec<Veg>, rng: &mut Rng, s: &Spec, st: &Style, butte: bool) {
    let h = s.height;
    let n = if butte { 9 } else { 12 };
    let (ang, base) = outline(rng, n, if butte { 2 } else { 3 }, 0.28);
    // The foot of the cliff sits at ~56% of the radius (the apron is the rest).
    let rc = s.radius * if butte { 0.40 } else { 0.50 };
    let squash = if butte { (0.82, 1.05) } else { (1.0, 0.92) };
    let bench = !butte && rng.f() < 0.7;
    let wall = Mat::Cliff;
    let cap = Mat::Top;
    let mut rings = vec![
        // cap rim
        RingSpec { z: h, r: rc * 0.86, j: 0.02, mat: cap },
    ];
    if bench {
        rings.push(RingSpec { z: h * 0.66, r: rc * 0.90, j: 0.03, mat: wall });
        rings.push(RingSpec { z: h * 0.52, r: rc * 0.96, j: 0.03, mat: wall });
        rings.push(RingSpec { z: h * 0.48, r: rc * 1.14, j: 0.02, mat: Mat::Rock });
        rings.push(RingSpec { z: h * 0.30, r: rc * 1.16, j: 0.03, mat: wall });
        rings.push(RingSpec { z: h * 0.14, r: rc * 1.20, j: 0.03, mat: wall });
    } else {
        rings.push(RingSpec { z: h * 0.72, r: rc * 0.92, j: 0.04, mat: wall });
        rings.push(RingSpec { z: h * 0.44, r: rc * 1.0, j: 0.04, mat: wall });
        rings.push(RingSpec { z: h * 0.18, r: rc * 1.05, j: 0.03, mat: wall });
    }
    let foot_z = rings.last().unwrap().z;
    let foot_r = rings.last().unwrap().r;
    // talus apron: scree slopes down to the ground
    let outer = s.radius * 0.92;
    rings.push(RingSpec { z: foot_z * 0.45, r: foot_r + (outer - foot_r) * 0.42, j: 0.06, mat: Mat::Scree });
    rings.push(RingSpec { z: 0.0, r: outer, j: 0.05, mat: Mat::Scree });
    let out = ring_strips(m, rng, &ang, &base, squash, &rings);
    subdivide(m, 0.8, s_seed(rng), &|mat| mat == Mat::Scree);
    // Cap: fan from the centre, with a gentle rise so the top has 2 shades.
    let top = &out[0];
    let ctr = [0.0, 0.0, h + 2.5];
    for i in 0..n {
        let j = (i + 1) % n;
        m.tri(top[i], top[j], ctr, Mat::Top);
    }
    // Fallen blocks at the cliff foot, big ones.
    let blocks = if butte { 3 } else { 4 };
    for _ in 0..blocks {
        let i = (rng.next() % n as u64) as usize;
        let a = ang[i] + rng.r(-0.2, 0.2);
        let rr = foot_r * rng.r(1.08, 1.5) * base[i];
        let (x, y) = polar(rr, a);
        let z = foot_z * rng.r(0.15, 0.35);
        let sz = rng.r(0.16, 0.30);
        boulder(m, [x * squash.0, y * squash.1, z], sz, sz * 0.8, sz * 26.0, rng.r(0.0, 3.0), Mat::Cliff, 0);
    }
    // a few outcrops on the cap, so a plateau is not a plate
    for _ in 0..if butte { 2 } else { 5 } {
        let a = rng.r(0.0, 6.28);
        let rr = rng.r(0.0, rc * 0.62);
        let (x, y) = polar(rr, a);
        let sz = rng.r(0.09, 0.2);
        boulder(m, [x * squash.0, y * squash.1, h + 2.0], sz, sz * 0.8, sz * 22.0, rng.r(0.0, 3.0), Mat::Rock, 0);
    }
    // vegetation on the cap
    if st.cap_veg {
        let cnt = if butte { 5 } else { 12 };
        for _ in 0..cnt {
            let a = rng.r(0.0, 6.28);
            let rr = rng.r(0.0, rc * 0.62);
            let (x, y) = polar(rr, a);
            veg.push(mk_veg(st, rng, x * squash.0, y * squash.1, false));
        }
    } else if st.tree == 4 {
        for _ in 0..4 {
            let a = rng.r(0.0, 6.28);
            let rr = rng.r(0.0, rc * 0.6);
            let (x, y) = polar(rr, a);
            veg.push(mk_veg(st, rng, x, y, false));
        }
    }
    // low-slope vegetation at the foot
    foot_veg(veg, rng, s, st, rc * 1.2, 0.35);
}

fn foot_veg(veg: &mut Vec<Veg>, rng: &mut Rng, s: &Spec, st: &Style, r0: f32, z: f32) {
    if st.tree == 0 {
        return;
    }
    let cnt = match st.tree {
        4 => 5,
        _ => 16,
    };
    for _ in 0..cnt {
        let a = rng.r(0.0, 6.28);
        let rr = rng.r(r0 * 1.05, s.radius * 0.95);
        let (x, y) = polar(rr, a);
        let _ = z;
        veg.push(mk_veg(st, rng, x, y, true));
    }
}

fn build_mountain(m: &mut Mesh, veg: &mut Vec<Veg>, rng: &mut Rng, s: &Spec, st: &Style) {
    let h = s.height;
    let n = 10; // 5 ridges, 5 valleys
    let phase = rng.r(0.0, 6.28);
    let ring_ang = |rng: &mut Rng, wander: f32| -> Vec<f32> {
        (0..n)
            .map(|i| phase + (i as f32 + rng.r(-wander, wander)) / n as f32 * std::f32::consts::TAU)
            .collect()
    };
    let peak = [rng.r(-0.12, 0.12) * s.radius, rng.r(-0.12, 0.12) * s.radius, h];
    // (z fraction, radius fraction of the apron, ridge amount)
    let specs = [(0.80, 0.15, 0.5), (0.58, 0.27, 0.8), (0.39, 0.34, 0.9), (0.23, 0.50, 0.8), (0.045, 0.80, 0.4), (0.0, 0.92, 0.0)];
    let mut rings: Vec<Vec<V3>> = Vec::new();
    for (k, (zf, rf, ridge)) in specs.iter().enumerate() {
        let ang = ring_ang(rng, 0.16 * (k as f32).min(2.0));
        let mut v = Vec::new();
        for i in 0..n {
            let is_ridge = i % 2 == 0;
            let rr = s.radius * rf * if is_ridge { 1.0 + 0.18 * ridge } else { 1.0 - 0.26 * ridge };
            let rr = rr * rng.r(0.93, 1.07);
            let zz = h * zf * if is_ridge { 1.0 + 0.05 * ridge } else { 1.0 - 0.18 * ridge };
            let (x, y) = polar(rr, ang[i]);
            v.push([x, y * 0.94, zz * rng.r(0.96, 1.04)]);
        }
        rings.push(v);
    }
    let strip_mat = |k: usize, zmid: f32| -> Mat {
        let zf = zmid / h;
        if st.snowline > 0.0 && zf > st.snowline {
            Mat::Snow
        } else if k >= 4 {
            Mat::Ground
        } else if k == 3 {
            Mat::Fringe
        } else if st.treeline > 0.0 && zf < st.treeline && k >= 1 {
            Mat::Veg
        } else if k >= 3 {
            Mat::Scree
        } else {
            Mat::Rock
        }
    };
    // summit fan
    for i in 0..n {
        let j = (i + 1) % n;
        let zm = (peak[2] + rings[0][i][2] + rings[0][j][2]) / 3.0;
        let mat = strip_mat(0, zm);
        m.tri(peak, rings[0][i], rings[0][j], mat);
    }
    for k in 0..rings.len() - 1 {
        for i in 0..n {
            let j = (i + 1) % n;
            let (a, b) = (&rings[k], &rings[k + 1]);
            let zm = (a[i][2] + a[j][2] + b[i][2] + b[j][2]) / 4.0;
            // snow follows the ridges: valley facets in shade stay rock
            let mut mat = strip_mat(k, zm);
            if mat == Mat::Snow && i % 2 == 1 && zm / h < st.snowline + 0.12 {
                mat = Mat::Rock;
            }
            if mat == Mat::Rock && k >= 1 && k <= 3 && i % 2 == 1 {
                mat = Mat::Cliff;
            }
            m.quad(a[i], a[j], b[j], b[i], mat);
        }
    }
    subdivide(m, 1.0, s_seed(rng), &|mat| mat != Mat::Ground);
    assign_mountain(m, s, st, s_seed(rng));
}

fn build_cone(m: &mut Mesh, veg: &mut Vec<Veg>, rng: &mut Rng, s: &Spec, st: &Style) {
    let h = s.height;
    let n = 14;
    let phase = rng.r(0.0, 6.28);
    let specs = [(0.0, 0.97), (0.03, 0.92), (0.11, 0.80), (0.30, 0.64), (0.64, 0.42), (1.0, 0.26)];
    let mut rings: Vec<Vec<V3>> = Vec::new();
    for (k, (zf, rf)) in specs.iter().enumerate() {
        let mut v = Vec::new();
        for i in 0..n {
            let a = phase + (i as f32 + rng.r(-0.12, 0.12)) / n as f32 * std::f32::consts::TAU;
            let amp = [0.0, 0.02, 0.05, 0.09, 0.11, 0.10][k.min(5)];
            let gully = if i % 2 == 0 { 1.0 + amp } else { 1.0 - amp };
            let rr = s.radius * rf * gully * rng.r(0.96, 1.04);
            let (x, y) = polar(rr, a);
            v.push([x, y * 0.95, h * zf * rng.r(0.985, 1.015)]);
        }
        rings.push(v);
    }
    // crater: the rim ring is the last; an inner ring a little lower, then floor
    let rim = rings.last().unwrap().clone();
    let inner: Vec<V3> = rim
        .iter()
        .map(|p| [p[0] * 0.62, p[1] * 0.62, h * rng_z(rng)])
        .collect();
    let floor = [0.0, 0.0, h * 0.80];
    let snowline = st.snowline;
    let mat_for = |zf: f32, k: usize| -> Mat {
        if k <= 1 {
            Mat::Ground
        } else if k == 2 {
            Mat::Fringe
        } else if snowline > 0.0 && zf > snowline && k >= 3 {
            Mat::Snow
        } else if st.treeline > 0.0 && zf < st.treeline {
            Mat::Veg
        } else {
            Mat::Dark
        }
    };
    for k in 0..rings.len() - 1 {
        for i in 0..n {
            let j = (i + 1) % n;
            let (a, b) = (&rings[k], &rings[k + 1]);
            let zf = (a[i][2] + b[i][2]) * 0.5 / h;
            let mut mat = mat_for(zf, k + 1);
            if mat == Mat::Dark && i % 2 == 0 {
                mat = Mat::Rock;
            }
            m.quad(a[i], a[j], b[j], b[i], mat);
        }
    }
    subdivide(m, 0.8, s_seed(rng), &|mat| mat != Mat::Ground);
    for i in 0..n {
        let j = (i + 1) % n;
        // inside wall of the crater
        m.quad(rim[i], rim[j], inner[j], inner[i], Mat::Dark);
        m.tri(inner[i], inner[j], floor, Mat::Scree);
    }
}

/// Trees on a mountain or cone: thick at the foot, thinning up to the treeline,
/// only where the slope allows, with a scatter across the transition onto the
/// real ground so the edge of the landform is not a line.
fn plant_slopes(m: &Mesh, veg: &mut Vec<Veg>, rng: &mut Rng, s: &Spec, st: &Style) {
    if st.tree == 0 {
        return;
    }
    let tl = if st.treeline > 0.0 { st.treeline } else { 0.34 };
    let mut taken: Vec<(f32, f32)> = Vec::new();
    for _ in 0..(120.0 * st.dense) as usize {
        let a = rng.r(0.0, 6.28);
        let rr = s.radius * rng.r(0.22, 1.08);
        let (x, y) = polar(rr, a);
        let (z, nz) = match ground_zn(m, x, y) {
            Some(v) => v,
            None if rr > s.radius * 0.8 => (0.0, 1.0),
            None => continue,
        };
        let zf = z / s.height;
        if zf >= tl || nz < 0.5 {
            continue;
        }
        let mut p = (1.0 - zf / tl).powf(0.8);
        if rr > s.radius * 0.95 {
            p *= 0.35;
        }
        if rng.f() > p {
            continue;
        }
        // not on top of another tree
        if taken.iter().any(|&(tx, ty)| (tx - x).powi(2) + (ty - y).powi(2) < 0.22) {
            continue;
        }
        taken.push((x, y));
        veg.push(mk_veg(st, rng, x, y, zf < tl * 0.45));
    }
}

fn s_seed(rng: &mut Rng) -> u64 {
    rng.next()
}

fn rng_z(rng: &mut Rng) -> f32 {
    rng.r(0.84, 0.9)
}

/// Loose boulders scattered across the apron, in the scree colour.
fn scatter_boulders(m: &mut Mesh, rng: &mut Rng, s: &Spec, st: &Style) {
    let _ = st;
    let n = if matches!(s.form, Form::Mesa | Form::Butte) { 16 } else { 9 };
    // A scatter of stones across the transition onto the real ground.
    for _ in 0..10 {
        let a = rng.r(0.0, 6.28);
        let rr = s.radius * rng.r(0.85, 1.12);
        let (x, y) = polar(rr, a);
        let sz = rng.r(0.04, 0.09);
        boulder(m, [x, y, 0.0], sz, sz * 0.85, sz * 18.0, rng.r(0.0, 3.0), Mat::Scree, 1);
    }
    for k in 0..n {
        let a = rng.r(0.0, 6.28);
        let rr = s.radius * rng.r(0.5, 0.9);
        let (x, y) = polar(rr, a);
        // sit it on the talus, not in it
        let Some(z) = ground_z(m, x, y) else { continue };
        let big = k % 5 == 0;
        let sz = if big { rng.r(0.13, 0.2) } else { rng.r(0.05, 0.11) };
        boulder(m, [x, y, z - 0.5], sz, sz * 0.85, sz * 20.0, rng.r(0.0, 3.0), Mat::Scree, 1);
    }
}

// ---------------------------------------------------------------------------
// rasteriser
// ---------------------------------------------------------------------------

/// Height of the highest upward-facing facet over a plan-view point.
fn ground_z(m: &Mesh, x: f32, y: f32) -> Option<f32> {
    ground_zn(m, x, y).map(|v| v.0)
}

/// As [`ground_z`], with the facet's upward normal component.
fn ground_zn(m: &Mesh, x: f32, y: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for t in &m.tris {
        let (a, b, c) = (t.p[0], t.p[1], t.p[2]);
        let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if d.abs() < 1e-7 {
            continue;
        }
        let l0 = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / d;
        let l1 = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / d;
        let l2 = 1.0 - l0 - l1;
        if l0 < 0.0 || l1 < 0.0 || l2 < 0.0 {
            continue;
        }
        let nz = normal(t)[2];
        if nz < 0.35 {
            continue;
        }
        let z = l0 * a[2] + l1 * b[2] + l2 * c[2];
        if best.map_or(true, |b| z > b.0) {
            best = Some((z, nz));
        }
    }
    best
}

const K_HORIZ: f32 = 36.0; // px of height that one tile of run is "worth" for normals

fn proj(p: V3) -> (f32, f32) {
    ((p[0] - p[1]) * HALF_W as f32, (p[0] + p[1]) * HALF_H as f32 - p[2])
}

fn normal(t: &Tri) -> V3 {
    let a = [t.p[0][0] * K_HORIZ, t.p[0][1] * K_HORIZ, t.p[0][2]];
    let b = [t.p[1][0] * K_HORIZ, t.p[1][1] * K_HORIZ, t.p[1][2]];
    let c = [t.p[2][0] * K_HORIZ, t.p[2][1] * K_HORIZ, t.p[2][2]];
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let mut n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
    n = [n[0] / l, n[1] / l, n[2] / l];
    // face outward: away from the axis, or up
    let cx = (t.p[0][0] + t.p[1][0] + t.p[2][0]) / 3.0;
    let cy = (t.p[0][1] + t.p[1][1] + t.p[2][1]) / 3.0;
    let cz = (t.p[0][2] + t.p[1][2] + t.p[2][2]) / 3.0;
    let out = [cx * K_HORIZ, cy * K_HORIZ, cz * 0.4 + 6.0];
    if n[0] * out[0] + n[1] * out[1] + n[2] * out[2] < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    n
}

#[derive(Clone, Copy, Default)]
struct Cell {
    depth: f32,
    z: f32,
    tri: u32, // 0 = empty, else index+1
}

fn render(m: &Mesh, veg: &[Veg], st: &Style, biome: Biome, sun: &Sun, seed: u64) -> Baked {
    // bounds
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for t in &m.tris {
        for p in &t.p {
            let (sx, sy) = proj(*p);
            x0 = x0.min(sx);
            x1 = x1.max(sx);
            y0 = y0.min(sy);
            y1 = y1.max(sy);
        }
    }
    for v in veg {
        let (sx, sy) = proj([v.x, v.y, v.z]);
        x0 = x0.min(sx - 40.0);
        x1 = x1.max(sx + 40.0);
        y0 = y0.min(sy - 84.0);
        y1 = y1.max(sy + 6.0);
    }
    let pad = 3;
    let ox = x0.floor() as i32 - pad;
    let oy = y0.floor() as i32 - pad;
    let w = (x1.ceil() as i32 + pad - ox + 1) as usize;
    let h = (y1.ceil() as i32 + pad - oy + 1) as usize;
    let mut buf = vec![Cell::default(); w * h];
    let normals: Vec<V3> = m.tris.iter().map(normal).collect();

    // z-buffered fill
    for (ti, t) in m.tris.iter().enumerate() {
        // The foot is the ground itself: the real tiles show through.
        if t.mat == Mat::Ground {
            continue;
        }
        let q: Vec<(f32, f32)> = t.p.iter().map(|p| {
            let (sx, sy) = proj(*p);
            (sx - ox as f32, sy - oy as f32)
        }).collect();
        let minx = q.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
        let maxx = q.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().min(w as f32 - 1.0) as i32;
        let miny = q.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
        let maxy = q.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().min(h as f32 - 1.0) as i32;
        let d = (q[1].1 - q[2].1) * (q[0].0 - q[2].0) + (q[2].0 - q[1].0) * (q[0].1 - q[2].1);
        if d.abs() < 1e-6 {
            continue;
        }
        for py in miny..=maxy {
            for px in minx..=maxx {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let l0 = ((q[1].1 - q[2].1) * (fx - q[2].0) + (q[2].0 - q[1].0) * (fy - q[2].1)) / d;
                let l1 = ((q[2].1 - q[0].1) * (fx - q[2].0) + (q[0].0 - q[2].0) * (fy - q[2].1)) / d;
                let l2 = 1.0 - l0 - l1;
                let e = -0.002;
                if l0 < e || l1 < e || l2 < e {
                    continue;
                }
                let wx = l0 * t.p[0][0] + l1 * t.p[1][0] + l2 * t.p[2][0];
                let wy = l0 * t.p[0][1] + l1 * t.p[1][1] + l2 * t.p[2][1];
                let wz = l0 * t.p[0][2] + l1 * t.p[1][2] + l2 * t.p[2][2];
                let depth = wx + wy;
                let c = &mut buf[py as usize * w + px as usize];
                if c.tri == 0 || depth > c.depth {
                    *c = Cell { depth, z: wz, tri: ti as u32 + 1 };
                }
            }
        }
    }

    // colour: one ramp step per facet, from the light
    let tri_idx: Vec<i32> = m
        .tris
        .iter()
        .zip(&normals)
        .map(|(t, n)| {
            let d = n[0] * sun.dir[0] + n[1] * sun.dir[1] + n[2] * sun.dir[2];
            // A flat surface takes the ground's own ramp step (3); a face turned
            // to the light is brighter, one turned away is darker.
            let flat = sun.dir[2];
            let base = (3.0 + (d.clamp(-0.4, 1.0) - flat) * 6.5).round() as i32;
            let b = match t.mat {
                Mat::Snow => 1,
                _ => 0,
            };
            let v = base + b + t.bias as i32;
            // the foot is the ground itself: never lighter than the ground beside it
            let v = if matches!(t.mat, Mat::Ground | Mat::Fringe) { v.clamp(st.fringe - 1, st.fringe) } else { v };
            v.clamp(0, 6)
        })
        .collect();

    let mut sp = Sprite::new(w, h);
    sp.set_anchor(-ox, -oy);
    let mut steep = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let c = buf[y * w + x];
            if c.tri == 0 {
                continue;
            }
            let ti = c.tri as usize - 1;
            let t = &m.tris[ti];
            let n = normals[ti];
            let ramp = st.ramp(t.mat);
            let mut idx = tri_idx[ti];
            let is_steep = n[2] < 0.5;
            steep[y * w + x] = is_steep;
            if is_steep && matches!(t.mat, Mat::Cliff | Mat::Rock | Mat::Dark) && st.strata > 0.0 {
                // horizontal bedding: bands in world z, one ramp step apart
                let band_h = 7.0;
                let zb = c.z / band_h;
                let bi = zb.floor() as i32;
                let off = match hash(bi, 7, seed) % 5 {
                    0 => -1,
                    1 => 1,
                    _ => 0,
                };
                let off = if st.strata < 0.6 && hash(bi, 3, seed) % 3 != 0 { 0 } else { off };
                idx += off;
                // a 1-px seam under every band on the strongly-banded rock
                if st.strata >= 0.5 && (zb - bi as f32) < 1.0 / band_h {
                    idx -= 1;
                }
            }
            // sparse stone speckle: a 2-px fleck a step lighter or darker
            if matches!(t.mat, Mat::Rock | Mat::Cliff | Mat::Dark | Mat::Scree | Mat::Top | Mat::Veg) {
                let hh = hash((x / 2) as i32, y as i32, seed ^ 0x57EC);
                let rate = if t.mat == Mat::Top { 70 } else { 38 };
                if hh % rate == 0 {
                    idx += if hh & 0x100 != 0 { 1 } else { -1 };
                }
            }
            sp.px[y * w + x] = ramp[idx.clamp(0, 6) as usize];
        }
    }

    // crack marks on steep rock: short vertical dark strokes, sparse
    for y in 1..h - 6 {
        for x in 1..w - 1 {
            let c = buf[y * w + x];
            if c.tri == 0 || !steep[y * w + x] {
                continue;
            }
            let t = &m.tris[c.tri as usize - 1];
            if !matches!(t.mat, Mat::Cliff | Mat::Dark) {
                continue;
            }
            if hash(x as i32, y as i32, seed ^ 0xC4AC) % 520 == 0 {
                let len = 3 + (hash(x as i32, y as i32, seed) % 4) as usize;
                for k in 0..len {
                    let cc = buf[(y + k) * w + x];
                    if cc.tri == c.tri {
                        let ti = c.tri as usize - 1;
                        let i2 = (tri_idx[ti] - 2).clamp(0, 6) as usize;
                        sp.px[(y + k) * w + x] = st.ramp(t.mat)[i2];
                    }
                }
            }
        }
    }

    // lip: the lit edge where a top meets a wall, and a dark seam under it
    let snapshot = sp.px.clone();
    for y in 1..h - 1 {
        for x in 0..w {
            let c = buf[y * w + x];
            if c.tri == 0 {
                continue;
            }
            let t = &m.tris[c.tri as usize - 1];
            let below = buf[(y + 1) * w + x];
            if below.tri != 0
                && !steep[y * w + x]
                && steep[(y + 1) * w + x]
                && matches!(t.mat, Mat::Top | Mat::Snow | Mat::Rock)
            {
                // lip highlight on the top surface's last row
                let ramp = st.ramp(t.mat);
                sp.px[y * w + x] = ramp[(tri_idx[c.tri as usize - 1] + 1).clamp(0, 6) as usize];
            }
        }
    }
    let _ = snapshot;

    // outline: a darker pixel on the silhouette (not on the ground-level apron)
    let alpha_of = |x: i32, y: i32, buf: &Vec<Cell>| -> bool {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return false;
        }
        buf[y as usize * w + x as usize].tri != 0
    };
    let mut outl: Vec<(usize, usize, Color)> = Vec::new();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if !alpha_of(x, y, &buf) {
                continue;
            }
            let c = buf[y as usize * w + x as usize];
            let t = &m.tris[c.tri as usize - 1];
            if matches!(t.mat, Mat::Ground | Mat::Fringe | Mat::Veg) || (c.z < 4.0 && t.mat == Mat::Scree) {
                continue;
            }
            let edge = !alpha_of(x - 1, y, &buf) || !alpha_of(x + 1, y, &buf) || !alpha_of(x, y - 1, &buf) || !alpha_of(x, y + 1, &buf);
            if edge {
                let ramp = st.ramp(t.mat);
                outl.push((x as usize, y as usize, ramp[0]));
            }
        }
    }
    for (x, y, c) in outl {
        sp.px[y * w + x] = c;
    }

    // vegetation, nearest last, depth-tested against the facets
    let mut vs: Vec<&Veg> = veg.iter().collect();
    vs.sort_by(|a, b| (a.x + a.y).partial_cmp(&(b.x + b.y)).unwrap());
    let mut dep: Vec<f32> = buf.iter().map(|c| if c.tri != 0 { c.depth } else { f32::NAN }).collect();
    let look = biome::look(biome);
    let mut scratch = crate::framebuffer::Framebuffer::new(120, 140);
    let mut trees = Vec::new();
    for v in vs {
        stamp_tree(&mut sp, &mut dep, &buf, w, h, ox, oy, v, look, &mut scratch, &mut trees);
    }

    let _ = TRANSPARENT;
    Baked { sprite: sp, depth: dep, trees }
}

/// Draw one of the engine's own trees into the sprite where it stands, hiding
/// the pixels behind a nearer facet.
#[allow(clippy::too_many_arguments)]
fn stamp_tree(
    sp: &mut Sprite,
    dep: &mut [f32],
    buf: &[Cell],
    w: usize,
    h: usize,
    ox: i32,
    oy: i32,
    v: &Veg,
    look: &biome::Ground,
    scratch: &mut crate::framebuffer::Framebuffer,
    trees: &mut Vec<SwayTree>,
) {
    let (sx, sy) = proj([v.x, v.y, v.z]);
    let (bx, by) = (sx.round() as i32 - ox, sy.round() as i32 - oy);
    let depth = v.x + v.y;
    let at = |x: i32, y: i32| -> Option<Cell> {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            None
        } else {
            Some(buf[y as usize * w + x as usize])
        }
    };
    // Is the ground here our surface (or bare ground beyond the mesh)?
    match at(bx, by) {
        Some(c) if c.tri != 0 && (c.depth - depth).abs() < 1.6 => {}
        Some(c) if c.tri == 0 && v.z < 1.0 => {}
        _ => return,
    }
    // The engine draws a tree at its ground point on a framebuffer.
    let (sw, shh) = (scratch.w as i32, scratch.h as i32);
    scratch.clear(TRANSPARENT);
    let (cx, cy) = (sw / 2, shh - 12);
    let season = crate::terrain::SeasonLook::default();
    crate::terrain::draw_tree_sprite_in(scratch, cx, cy, v.species, v.sz, v.variant, v.jitter, &season, look);
    // B69b: a tree that sways is kept apart (its surviving pixels), and drawn
    // by the sway layer; the depth stamp stays, so the painter order is as before.
    let sways = crate::sway::sways(v.species);
    let mut kept = Vec::new();
    for yy in 0..shh {
        for xx in 0..sw {
            let c = scratch.px()[(yy * sw + xx) as usize];
            if c == TRANSPARENT || (c >> 24) < 0xF0 {
                continue;
            }
            let (px, py) = (bx + xx - cx, by + yy - cy);
            if let Some(cell) = at(px, py) {
                if cell.tri == 0 || cell.depth <= depth + 1.6 {
                    if sways {
                        kept.push(((xx - cx) as i16, (yy - cy) as i16, c | 0xFF00_0000));
                    } else {
                        sp.set(px as usize, py as usize, c | 0xFF00_0000);
                    }
                    dep[py as usize * w + px as usize] = depth;
                }
            }
        }
    }
    if sways && !kept.is_empty() {
        trees.push(SwayTree { species: v.species, bx, by, depth, px: kept });
    }
}

// ---------------------------------------------------------------------------
// sites: where a landform stands in a generated world
// ---------------------------------------------------------------------------

use crate::framebuffer::Rect;
use crate::iso::Camera;
use map::iso_consts::HEIGHT_UNIT;
use map::TileMap;

/// The biggest foot a sprite carries, in tiles. The generator grows massifs far
/// larger; `flatten` hides the rest.
pub use map::massif::MAX_RADIUS;

/// Where a landform stands and how big it is (tile space, levels).
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub form: Form,
    pub biome: Biome,
    pub cx: f32,
    pub cy: f32,
    /// Foot radius in tiles.
    pub radius: f32,
    /// Ground level the foot sits on.
    pub base: f32,
    /// Levels above `base` at the summit.
    pub peak: f32,
    pub seed: u64,
}

impl Site {
    pub fn spec(&self) -> Spec {
        let px = (self.peak * HEIGHT_UNIT as f32 * 1.3).clamp(90.0, 230.0);
        Spec {
            form: self.form,
            biome: self.biome,
            seed: self.seed,
            radius: self.radius,
            height: px.min(self.radius * match self.form { Form::Mesa => 19.0, Form::Butte => 25.0, _ => 32.0 }),
        }
    }
}

/// A baked site.
pub struct Landform {
    pub site: Site,
    pub sprite: Sprite,
    /// B69b: the swaying trees of the flank and the painter diagonal of each.
    trees: Vec<(i32, SwayTree)>,
    model: Model,
    /// Per sprite pixel: the painter diagonal (floor of x + y) it belongs to, or
    /// `i16::MIN` for empty. Fixed by the geometry, so independent of the light.
    diag: Vec<i16>,
    pub d_min: i32,
    pub d_max: i32,
    /// Height of the body above its foot, per shadow sample (8 a tile), over
    /// `[gx0, gx0 + gw) x [gy0, gy0 + gh)` samples.
    grid: Vec<f32>,
    gx0: i32,
    gy0: i32,
    gw: i32,
    gh: i32,
}

const GRID_N: i32 = 8;

impl Landform {
    /// B61f: the bytes this landform's sprite and its own planes hold (for the
    /// memory report).
    pub fn bytes(&self) -> usize {
        self.sprite.bytes() + self.diag.capacity() * 2 + self.grid.capacity() * 4
    }

    pub fn bake(site: Site, sun: &Sun) -> Self {
        let model = model(&site.spec());
        let baked = render_model(&model, sun);
        let sprite = baked.sprite;
        let base_d = site.cx + site.cy;
        let trees: Vec<(i32, SwayTree)> =
            baked.trees.into_iter().map(|t| ((base_d + t.depth).floor() as i32, t)).collect();
        let diag: Vec<i16> = baked
            .depth
            .iter()
            .map(|d| if d.is_nan() { i16::MIN } else { (base_d + d).floor() as i16 })
            .collect();
        let d_min = diag.iter().copied().filter(|&d| d != i16::MIN).min().unwrap_or(0) as i32;
        let d_max = diag.iter().copied().max().unwrap_or(0) as i32;
        // The body as a height field, for the cast shadows.
        let r = site.radius + 1.0;
        let gx0 = ((site.cx - r) * GRID_N as f32).floor() as i32;
        let gy0 = ((site.cy - r) * GRID_N as f32).floor() as i32;
        let gw = (2.0 * r * GRID_N as f32).ceil() as i32 + 2;
        let gh = gw;
        let mut grid = vec![0.0f32; (gw * gh) as usize];
        for t in &model.mesh.tris {
            let p: Vec<(f32, f32, f32)> = t
                .p
                .iter()
                .map(|v| ((v[0] + site.cx) * GRID_N as f32 - gx0 as f32 - 0.5, (v[1] + site.cy) * GRID_N as f32 - gy0 as f32 - 0.5, v[2]))
                .collect();
            let x0 = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
            let x1 = p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil().min(gw as f32 - 1.0) as i32;
            let y0 = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
            let y1 = p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil().min(gh as f32 - 1.0) as i32;
            let d = (p[1].1 - p[2].1) * (p[0].0 - p[2].0) + (p[2].0 - p[1].0) * (p[0].1 - p[2].1);
            if d.abs() < 1e-6 {
                continue;
            }
            for j in y0..=y1 {
                for i in x0..=x1 {
                    let (fx, fy) = (i as f32, j as f32);
                    let l0 = ((p[1].1 - p[2].1) * (fx - p[2].0) + (p[2].0 - p[1].0) * (fy - p[2].1)) / d;
                    let l1 = ((p[2].1 - p[0].1) * (fx - p[2].0) + (p[0].0 - p[2].0) * (fy - p[2].1)) / d;
                    let l2 = 1.0 - l0 - l1;
                    if l0 < -0.02 || l1 < -0.02 || l2 < -0.02 {
                        continue;
                    }
                    let z = l0 * p[0].2 + l1 * p[1].2 + l2 * p[2].2;
                    let k = (j * gw + i) as usize;
                    if z > grid[k] {
                        grid[k] = z;
                    }
                }
            }
        }
        Self { site, sprite, trees, model, diag, d_min, d_max, grid, gx0, gy0, gw, gh }
    }

    /// Re-render the sprite for a new light (the model is kept).
    pub fn relight(&mut self, sun: &Sun) {
        // The trees do not depend on the light: only the sprite is re-made.
        self.sprite = render_model(&self.model, sun).sprite;
    }

    /// Paint the pixels that belong to painter diagonal `d` (a landform is drawn
    /// slice by slice, so a building in front of one flank still stands in front).
    pub fn blit_slice(&self, fb: &mut crate::framebuffer::Framebuffer, cam: &Camera, d: i32) {
        let (ox, oy) = self.origin(cam);
        let s = &self.sprite;
        // B69b: the flank's trees that stand on this diagonal go to the sway
        // layer; with no layer they are painted here, as they were in the sprite.
        for (td, t) in &self.trees {
            if *td != d {
                continue;
            }
            let anchor = (ox + t.bx, oy + t.by);
            if !crate::sway::capture_pixels(fb, anchor, &t.px, t.species) {
                for &(dx, dy, c) in &t.px {
                    fb.put(anchor.0 + dx as i32, anchor.1 + dy as i32, c);
                }
            }
        }
        let r = Rect::new(ox, oy, ox + s.w as i32, oy + s.h as i32).intersect(&fb.clip());
        if r.is_empty() {
            return;
        }
        let key = fb.depth_key();
        for y in r.y0..r.y1 {
            let row = (y - oy) as usize * s.w;
            for x in r.x0..r.x1 {
                let i = row + (x - ox) as usize;
                if self.diag[i] as i32 == d && s.px[i] != TRANSPARENT {
                    fb.put(x, y, s.px[i]);
                    let _ = key;
                }
            }
        }
    }

    /// Height of the body above the ground at a tile-space point, in px (0
    /// outside it): what the cast shadows see.
    pub fn top_at(&self, x: f32, y: f32) -> f32 {
        let i = (x * GRID_N as f32 - 0.5).round() as i32 - self.gx0;
        let j = (y * GRID_N as f32 - 0.5).round() as i32 - self.gy0;
        if i < 0 || j < 0 || i >= self.gw || j >= self.gh {
            return 0.0;
        }
        self.grid[(j * self.gw + i) as usize]
    }

    /// Screen position the sprite is blitted at.
    pub fn origin(&self, cam: &Camera) -> (i32, i32) {
        let z = (self.site.base * HEIGHT_UNIT as f32).round() as i32;
        let (sx, sy) = cam.project(self.site.cx, self.site.cy, z);
        (sx.round() as i32 - self.sprite.ax, sy.round() as i32 - self.sprite.ay)
    }
    pub fn bounds(&self, cam: &Camera) -> Rect {
        let (x, y) = self.origin(cam);
        Rect::new(x, y, x + self.sprite.w as i32, y + self.sprite.h as i32)
    }

    /// Is tile `(x, y)` under the sprite?
    pub fn covers(&self, x: i32, y: i32) -> bool {
        let dx = x as f32 + 0.5 - self.site.cx;
        let dy = y as f32 + 0.5 - self.site.cy;
        dx * dx + dy * dy <= (self.site.radius + 0.5).powi(2)
    }
}

/// Read the generator's massifs (see `map::massif`) and give each a form: the
/// sites on one map take different forms (a rotation from a seed-chosen start).
pub fn place(map: &TileMap) -> (Vec<Site>, Vec<(f32, f32, f32)>) {
    let forms = Form::for_biome(map.biome);
    let (sites, others) = (map.landforms.clone(), map.hills.clone());
    let start = sites.first().map_or(0, |m| (Rng::new(m.seed).next() % 3) as usize);
    let sites = sites
        .into_iter()
        .enumerate()
        .map(|(i, m)| Site {
            form: forms[(start + i) % 3],
            biome: map.biome,
            cx: m.cx,
            cy: m.cy,
            radius: m.radius,
            base: m.base,
            peak: m.peak,
            seed: m.seed,
        })
        .collect();
    (sites, others.iter().map(|m| (m.cx, m.cy, m.radius)).collect())
}

/// The sites alone.
pub fn detect(map: &TileMap) -> Vec<Site> {
    place(map).0
}

/// Flatten the footprint of every landform to its foot level, and let the
/// ground outside it rise only gently instead of walling the sprite in. This is
/// the *drawing* map: the sim and the shadow field keep the real heights. The
/// work is on the shared corner grid so neighbouring tiles never crack apart.
/// Hills that got no sprite are smoothed so no massif stays a terraced plate.
pub fn flatten(map: &mut TileMap, sites: &[Site], hills: &[(f32, f32, f32)]) {
    let (w, h) = (map.w, map.h);
    if sites.is_empty() && hills.is_empty() {
        return;
    }
    let vw = w + 1;
    let mut v = vec![0u8; (vw * (h + 1)) as usize];
    for y in 0..h {
        for x in 0..w {
            let t = &map.tiles[(y * w + x) as usize];
            v[(y * vw + x) as usize] = t.corner[0];
            v[(y * vw + x + 1) as usize] = t.corner[1];
            v[((y + 1) * vw + x + 1) as usize] = t.corner[2];
            v[((y + 1) * vw + x) as usize] = t.corner[3];
        }
    }
    // A massif that got no sprite is smoothed: its corner grid is blurred over its
    // own footprint, so the terrace steps of the tile grid become a slope.
    let mut smooth = v.clone();
    for &(hx, hy, hr) in hills {
        let r = hr * 1.35;
        let x0 = ((hx - r).floor() as i32).max(1);
        let x1 = ((hx + r).ceil() as i32).min(w - 1);
        let y0 = ((hy - r).floor() as i32).max(1);
        let y1 = ((hy + r).ceil() as i32).min(h - 1);
        for _ in 0..3 {
            for vy in y0..=y1 {
                for vx in x0..=x1 {
                    let i = (vy * vw + vx) as usize;
                    let up = vw as usize;
                    let sum = v[i - up - 1] as u32
                        + v[i - up] as u32 * 2
                        + v[i - up + 1] as u32
                        + v[i - 1] as u32 * 2
                        + v[i] as u32 * 4
                        + v[i + 1] as u32 * 2
                        + v[i + up - 1] as u32
                        + v[i + up] as u32 * 2
                        + v[i + up + 1] as u32;
                    smooth[i] = (sum / 16) as u8;
                }
            }
            std::mem::swap(&mut v, &mut smooth);
        }
    }
    for s in sites {
        let r = s.radius;
        let reach = (r * 2.4) as i32 + 4;
        let x0 = ((s.cx - r - reach as f32).floor() as i32).max(0);
        let x1 = ((s.cx + r + reach as f32).ceil() as i32).min(w);
        let y0 = ((s.cy - r - reach as f32).floor() as i32).max(0);
        let y1 = ((s.cy + r + reach as f32).ceil() as i32).min(h);
        for vy in y0..=y1 {
            for vx in x0..=x1 {
                let (dx, dy) = (vx as f32 - s.cx, vy as f32 - s.cy);
                let d = (dx * dx + dy * dy).sqrt();
                let i = (vy * vw + vx) as usize;
                if d <= r + 0.5 {
                    v[i] = s.base.round().clamp(0.0, 255.0) as u8;
                } else {
                    let cap = s.base + (d - r - 2.0).max(0.0) * 0.3;
                    if v[i] as f32 > cap {
                        v[i] = cap.round().max(s.base).clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
    // The massif's own rock and scree tiles would show round the sprite as a
    // tile-stepped red or grey collar: the foot is painted by the sprite's
    // talus, so the ground round it is the land's plain.
    let plains: Vec<map::Kind> = sites
        .iter()
        .map(|s| {
            let mut count: Vec<(map::Kind, u32)> = Vec::new();
            let (r0, r1) = (s.radius + 5.0, s.radius + 10.0);
            for y in 0..h {
                for x in 0..w {
                    let d = ((x as f32 + 0.5 - s.cx).powi(2) + (y as f32 + 0.5 - s.cy).powi(2)).sqrt();
                    if d < r0 || d > r1 {
                        continue;
                    }
                    let k = map.tiles[(y * w + x) as usize].kind;
                    if matches!(k, map::Kind::Rock | map::Kind::Scree) || k.is_water() {
                        continue;
                    }
                    match count.iter_mut().find(|(c, _)| *c == k) {
                        Some(e) => e.1 += 1,
                        None => count.push((k, 1)),
                    }
                }
            }
            count.sort_by(|a, b| b.1.cmp(&a.1));
            count.first().map(|c| c.0).unwrap_or(map::Kind::Grass)
        })
        .collect();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let covered = sites.iter().any(|s| {
                let (dx, dy) = (x as f32 + 0.5 - s.cx, y as f32 + 0.5 - s.cy);
                dx * dx + dy * dy <= (s.radius + 0.5).powi(2)
            });
            let t = &mut map.tiles[i];
            for (si, s) in sites.iter().enumerate() {
                let d = ((x as f32 + 0.5 - s.cx).powi(2) + (y as f32 + 0.5 - s.cy).powi(2)).sqrt();
                if d <= s.radius + 4.5 && matches!(t.kind, map::Kind::Rock | map::Kind::Scree) {
                    t.kind = plains[si];
                }
            }
            t.corner = [
                v[(y * vw + x) as usize],
                v[(y * vw + x + 1) as usize],
                v[((y + 1) * vw + x + 1) as usize],
                v[((y + 1) * vw + x) as usize],
            ];
            if covered {
                t.prop = map::Prop::None;
                t.aux = 0;
                map.wear[i] = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests;
