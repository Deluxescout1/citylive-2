//! B80 (D84): farmland. What a `Kind::Field` tile looks like for each crop and
//! each stage of the crop year, the fences, hedges and walls between fields,
//! the farm track, and the things that stand in a field (maize, vines, the
//! orchard's trees).
//!
//! The sim writes the crop into the tile (`map::crop_of`); the stage of the
//! year comes from `SeasonLook::field_stage` (0 ploughed, 0.12-0.4 shoots,
//! 0.4-0.7 green turning gold, 0.7-0.92 ripe, 0.92-1 stubble). Everything is
//! painted into the static layer: rows in tile space so they run on across
//! tile joints, a per-field tone so neighbours differ, and no per-frame cost.

use map::{Crop, Kind, Prop, TileMap, TreeSpecies};

use crate::biome::Ground;
use crate::framebuffer::Framebuffer;
use crate::iso::Pt;
use crate::palette::{self, Color};
use crate::tex;

const S: i32 = map::iso_consts::DETAIL;

/// Grass-covered crops are drawn by the ordinary grass pipeline.
#[inline]
pub fn grassy(c: Crop) -> bool {
    matches!(c, Crop::Pasture | Crop::Orchard | Crop::Vineyard)
}

/// Everything one field tile needs, worked out once per tile.
#[derive(Clone, Copy)]
pub struct FieldTile {
    pub crop: Crop,
    /// Rows run along x (true) or along y.
    pub along_x: bool,
    /// Per-field 0..1: tone and which variety.
    pub tone: f32,
    pub stage: f32,
    pub snow: f32,
    pub soil: [Color; 3],
    /// A flooded or tropical land: staged per field instead of per season.
    pub staged: bool,
    /// This tile's neighbours along its own track (for ruts): bit0 -x, 1 +x, 2 -y, 3 +y.
    pub link: u8,
}

fn same(map: &TileMap, x: i32, y: i32, crop: Crop) -> bool {
    map.in_bounds(x, y) && {
        let t = map.at(x, y);
        t.kind == Kind::Field && map::crop_of(&t) == crop
    }
}

/// The run of like tiles through `(x, y)` along one axis, as `(back, forward)`.
fn run(map: &TileMap, x: i32, y: i32, crop: Crop, dx: i32, dy: i32) -> (i32, i32) {
    let (mut b, mut f) = (0, 0);
    while b < 14 && same(map, x - dx * (b + 1), y - dy * (b + 1), crop) {
        b += 1;
    }
    while f < 14 && same(map, x + dx * (f + 1), y + dy * (f + 1), crop) {
        f += 1;
    }
    (b, f)
}

impl FieldTile {
    pub fn of(
        map: &TileMap,
        x: i32,
        y: i32,
        look: &Ground,
        stage: f32,
        snow: f32,
    ) -> FieldTile {
        let t = map.at(x, y);
        let crop = map::crop_of(&t);
        let (bx, fx) = run(map, x, y, crop, 1, 0);
        let (by, fy) = run(map, x, y, crop, 0, 1);
        let (lenx, leny) = (bx + fx + 1, by + fy + 1);
        let along_x = if lenx == leny { map::rng::hash2(x - bx, y - by, 0xA1) > 0.5 } else { lenx > leny };
        // The field's own anchor: the corner its run starts at.
        let (ax, ay) = (x - bx, y - by);
        let tone = map::rng::hash2(ax, ay, 0xF1E1D);
        let link = (same(map, x - 1, y, crop) as u8)
            | (same(map, x + 1, y, crop) as u8) << 1
            | (same(map, x, y - 1, crop) as u8) << 2
            | (same(map, x, y + 1, crop) as u8) << 3;
        let staged = matches!(
            look.biome,
            map::Biome::Tropical | map::Biome::Bamboo | map::Biome::Swamp | map::Biome::Lagoon
        ) || crop == Crop::Paddy;
        // A tropical field is not on the temperate year: each field sits at its
        // own point of a growing cycle.
        let stage = if staged { (0.14 + tone * 0.84).clamp(0.0, 1.0) } else { stage };
        FieldTile {
            crop,
            along_x,
            tone,
            stage,
            snow,
            soil: [look.dirt[2], look.dirt[3], look.dirt[4]],
            staged,
            link,
        }
    }
}

#[inline]
fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn grain(c: Color, px: i32, py: i32, salt: u32) -> Color {
    let h = tex::hash2(px, py, salt);
    if h < 0.10 {
        palette::shade(c, 0.93)
    } else if h > 0.92 {
        palette::shade(c, 1.06)
    } else {
        c
    }
}

/// Cover of the ground by the crop (0 = bare soil, 1 = full).
fn cover(stage: f32) -> f32 {
    smooth(0.10, 0.40, stage)
}

/// 0 = green, 1 = gold: how ripe a cereal is.
fn ripe(stage: f32, late: f32) -> f32 {
    smooth(0.40 + late, 0.72 + late, stage)
}

/// 1 once the stubble stage is reached.
fn stubble(stage: f32) -> f32 {
    smooth(0.90, 0.95, stage)
}

fn palette_green(tone: f32) -> [Color; 3] {
    let k = 0.92 + tone * 0.16;
    [
        palette::shade(palette::rgb(0x86, 0xbf, 0x4c), k),
        palette::shade(palette::rgb(0x5f, 0x9d, 0x3c), k),
        palette::shade(palette::rgb(0x3f, 0x78, 0x32), k),
    ]
}

fn palette_gold(tone: f32) -> [Color; 3] {
    // Wheat runs from pale straw to deep amber from one field to the next.
    let warm = tone;
    [
        palette::mix(palette::rgb(0xee, 0xd0, 0x72), palette::rgb(0xf0, 0xbc, 0x56), warm),
        palette::mix(palette::rgb(0xd6, 0xae, 0x4a), palette::rgb(0xd0, 0x92, 0x36), warm),
        palette::mix(palette::rgb(0xa9, 0x84, 0x34), palette::rgb(0xa0, 0x6a, 0x2a), warm),
    ]
}

const STUBBLE: [Color; 3] = [
    palette::rgb(0xd4, 0xbb, 0x7c),
    palette::rgb(0xbd, 0xa0, 0x62),
    palette::rgb(0x8f, 0x76, 0x46),
];

/// The colour of one pixel of a field. `gx, gy` are tile-space coordinates of
/// the pixel (tile + the position inside it); rows run along one axis.
pub fn field_px(f: &FieldTile, base: Color, px: i32, py: i32, gx: f32, gy: f32) -> Color {
    let c = match f.crop {
        Crop::Cereal => cereal_px(f, px, py, gx, gy),
        Crop::Maize => maize_px(f, px, py, gx, gy),
        Crop::Veg => veg_px(f, px, py, gx, gy),
        Crop::Paddy => paddy_px(f, px, py, gx, gy),
        Crop::Track => track_px(f, base, px, py, gx, gy),
        Crop::Vineyard => vineyard_px(f, base, px, py, gx, gy),
        Crop::Pasture => pasture_px(f, base, px, py, gx, gy),
        Crop::Orchard => orchard_px(f, base, px, py, gx, gy),
    };
    snowy(f, c, px, py)
}

fn snowy(f: &FieldTile, c: Color, px: i32, py: i32) -> Color {
    if f.snow < 0.02 {
        return c;
    }
    // Snow lies in the furrows deeper than on the crests: the rows stay
    // faintly legible under it.
    let white = palette::rgb(0xf0, 0xf5, 0xfa);
    let k = (0.86 * f.snow).min(0.94);
    let lum = (palette::r(c) as f32 + palette::g(c) as f32 + palette::b(c) as f32) / 765.0;
    let shade = 1.0 - (0.5 - lum).max(0.0) * 0.22;
    let w = palette::shade(white, shade);
    let w = if tex::hash2(px, py, 0x5A0) > 0.93 { palette::shade(w, 0.96) } else { w };
    palette::mix(c, w, k)
}

fn row_coords(f: &FieldTile, gx: f32, gy: f32, rows: f32) -> (f32, f32) {
    // r: which row and where in it; a: distance along the row, in tiles.
    let (r, a) = if f.along_x { (gy, gx) } else { (gx, gy) };
    (r * rows, a)
}

fn cereal_px(f: &FieldTile, px: i32, py: i32, gx: f32, gy: f32) -> Color {
    let (rr, a) = row_coords(f, gx, gy, 7.0);
    let rp = rr.fract();
    let s = f.stage;
    let soil = f.soil;
    let cov = cover(s);
    let gold = ripe(s, 0.0);
    let stub = stubble(s);
    let wave = tex::hash2((gx * 3.0) as i32, (gy * 3.0) as i32, 0xC3);
    // Soil: a dark furrow, a lit crest.
    let mut c = if rp < 0.34 {
        soil[0]
    } else if rp < 0.80 {
        soil[2]
    } else {
        soil[1]
    };
    c = grain(c, px, py, 0x31);
    if s < 0.12 {
        // Harrowed tilth: fine clods along the rows.
        if tex::hash2(px, py / 2, 0x32) > 0.80 {
            c = palette::shade(c, 1.10);
        }
        return c;
    }
    // The crop: plants along the crest, thickening with the cover.
    let plant_here = (0.34..0.84).contains(&rp);
    let dens = if plant_here { cov * 1.05 } else { cov * 0.45 };
    let n = tex::hash2(px, py, 0x33);
    if n < dens {
        let g = palette_green(f.tone);
        let go = palette_gold(f.tone);
        let leaf = if rp > 0.55 { 0 } else { 1 };
        let green = g[leaf];
        let golden = go[(leaf + (n > dens * 0.7) as usize).min(2)];
        let mut crop = palette::mix(green, golden, gold);
        if wave > 0.82 {
            crop = palette::shade(crop, 1.07);
        } else if wave < 0.18 {
            crop = palette::shade(crop, 0.94);
        }
        if stub > 0.0 {
            let st = STUBBLE[(rp > 0.55) as usize];
            crop = palette::mix(crop, st, stub);
        }
        c = crop;
    } else if stub > 0.5 {
        c = palette::mix(c, STUBBLE[2], 0.40 * stub);
    }
    let _ = a;
    c
}

fn maize_px(f: &FieldTile, px: i32, py: i32, gx: f32, gy: f32) -> Color {
    let (rr, _a) = row_coords(f, gx, gy, 4.0);
    let rp = rr.fract();
    let s = f.stage;
    let soil = f.soil;
    let cov = cover(s);
    let gold = ripe(s, 0.10);
    let stub = stubble(s);
    let mut c = if rp < 0.30 { soil[0] } else if rp < 0.82 { soil[2] } else { soil[1] };
    c = grain(c, px, py, 0x41);
    if s < 0.12 {
        return c;
    }
    // Wide rows with leaf ground cover; the stalks themselves stand up (see
    // `draw_stand`). Between the rows the weeds and the leaf shade.
    let in_row = (0.28..0.86).contains(&rp);
    let n = tex::hash2(px, py, 0x42);
    if (in_row && n < cov * 1.1) || n < cov * 0.35 {
        let g = [palette::rgb(0x4e, 0x8f, 0x34), palette::rgb(0x3a, 0x7a, 0x2c), palette::rgb(0x2b, 0x62, 0x28)];
        let go = [palette::rgb(0xc5, 0xa8, 0x4c), palette::rgb(0xa8, 0x8a, 0x3c), palette::rgb(0x84, 0x6c, 0x2e)];
        let i = ((n * 7.0) as usize + (rp * 3.0) as usize) % 3;
        let mut crop = palette::mix(g[i], go[i], gold);
        if stub > 0.0 {
            crop = palette::mix(crop, STUBBLE[i], stub);
        }
        c = crop;
    }
    c
}

fn veg_px(f: &FieldTile, px: i32, py: i32, gx: f32, gy: f32) -> Color {
    let (rr, a) = row_coords(f, gx, gy, 5.0);
    let row = rr.floor() as i32;
    let rp = rr.fract();
    let s = f.stage;
    let soil = f.soil;
    let cov = smooth(0.12, 0.34, s) * (1.0 - smooth(0.86, 0.93, s));
    let mut c = if rp < 0.28 { soil[0] } else { soil[1] };
    c = grain(c, px, py, 0x51);
    // A bed: rows of plants set a regular step apart.
    let pitch = 0.17;
    let ai = (a / pitch).floor() as i32;
    let af = (a / pitch).fract();
    let which = (map::rng::hash2(row, (f.tone * 977.0) as i32, 0xB3) * 5.0) as u32;
    let skip = tex::hash2(ai, row, 0x52) > 0.93;
    if cov > 0.05 && !skip && (0.28..0.98).contains(&rp) && (0.12..0.88).contains(&af) {
        let jitter = tex::hash2(px, py, 0x53);
        if jitter < cov {
            let (lo, hi) = match which {
                0 => (palette::rgb(0x7a, 0xa6, 0x6a), palette::rgb(0xa6, 0xcc, 0x8a)), // cabbage
                1 => (palette::rgb(0x6c, 0xb0, 0x3e), palette::rgb(0x9a, 0xd8, 0x58)), // lettuce
                2 => (palette::rgb(0x4c, 0x88, 0x34), palette::rgb(0x76, 0xb2, 0x46)), // carrots, feathery
                3 => (palette::rgb(0x58, 0x92, 0x3a), palette::rgb(0xc8, 0x3c, 0x30)), // tomatoes
                _ => (palette::rgb(0x6e, 0x3a, 0x5c), palette::rgb(0x9a, 0x52, 0x84)), // beet
            };
            c = if rp < 0.55 { lo } else { palette::mix(lo, hi, 0.5 + 0.5 * jitter) };
            if which == 3 && jitter < cov * 0.18 {
                c = hi;
            }
        }
    }
    c
}

fn paddy_px(f: &FieldTile, px: i32, py: i32, gx: f32, gy: f32) -> Color {
    let s = f.stage;
    let cov = smooth(0.10, 0.30, s) * (1.0 - smooth(0.88, 0.97, s));
    let gold = ripe(s, 0.05);
    // The flooded bed: green-blue water with the sky in it, brighter streaks.
    let sheen = tex::hash2(px / 3, py, 0x61);
    let drained = smooth(0.90, 0.97, s);
    let water = palette::mix(
        palette::rgb(0x4c, 0x8e, 0x8c),
        palette::rgb(0x7c, 0xbc, 0xb4),
        if sheen > 0.82 { 1.0 } else { sheen * 0.25 },
    );
    let mud = palette::rgb(0x6d, 0x58, 0x3c);
    let mut c = palette::mix(water, mud, drained);
    // Rice hills in a regular grid (four to a tile each way): a tuft of leaves.
    let cx = (gx * 4.0).floor() as i32;
    let cy = (gy * 4.0).floor() as i32;
    let fx = (gx * 4.0).fract();
    let fy = (gy * 4.0).fract();
    let present = tex::hash2(cx, cy, 0x62) < 0.96;
    if cov > 0.04 && present {
        // A tuft is a small star: tall in the middle, leaves leaning out.
        let dx = (fx - 0.5).abs();
        let dy = (fy - 0.55).abs();
        let reach = 0.16 + 0.34 * cov;
        if dx < reach && dy < reach * 0.9 && dx + dy * 1.3 < reach * 1.25 {
            let n = tex::hash2(px, py, 0x63);
            let green = palette::mix(palette::rgb(0x4a, 0x9e, 0x38), palette::rgb(0x92, 0xd0, 0x50), n);
            let golden = palette::mix(palette::rgb(0xc8, 0xb0, 0x4c), palette::rgb(0xe8, 0xd0, 0x6a), n);
            c = palette::mix(green, golden, gold);
        }
    }
    c
}

fn pasture_px(f: &FieldTile, base: Color, px: i32, py: i32, gx: f32, gy: f32) -> Color {
    // Grazed turf: patches of lusher and of thinner grass, clover and a few
    // flowers, at hand-placed scale (blocks of a third of a tile).
    let block = tex::hash2(px.div_euclid(7), py.div_euclid(3), 0xA5 ^ (f.tone * 255.0) as u32);
    let _ = (gx, gy);
    let mut c = base;
    if block > 0.80 {
        c = palette::shade(c, 0.91);
    } else if block < 0.14 {
        c = palette::mix(c, palette::rgb(0xb6, 0xb6, 0x68), 0.30);
    }
    let h = tex::hash2(px, py, 0xA6);
    if h > 0.993 {
        c = if tex::hash2(px / 2, py, 0xA7) > 0.5 { palette::rgb(0xf4, 0xf0, 0xd8) } else { palette::rgb(0xf0, 0xd8, 0x52) };
    } else if h > 0.975 {
        c = palette::shade(c, 1.12);
    }
    c
}

fn orchard_px(_f: &FieldTile, base: Color, px: i32, py: i32, gx: f32, gy: f32) -> Color {
    // Mown alleys between the tree rows: stripes a little lighter than the
    // grass under the trees.
    let k = ((gx + gy) * 1.0).fract();
    let _ = (px, py);
    if (0.35..0.65).contains(&k) { palette::shade(base, 1.05) } else { base }
}

fn vineyard_px(f: &FieldTile, base: Color, px: i32, py: i32, gx: f32, gy: f32) -> Color {
    let (rr, _a) = row_coords(f, gx, gy, 3.0);
    let rp = rr.fract();
    // A strip of bare earth under each vine row.
    if (0.38..0.62).contains(&rp) {
        let soil = grain(f.soil[1], px, py, 0x71);
        let c = if rp < 0.42 { palette::shade(soil, 0.9) } else { soil };
        return c;
    }
    // Between the rows the grass is mown short and a touch paler.
    let _ = f;
    if (0.15..0.30).contains(&rp) {
        palette::shade(base, 0.96)
    } else {
        base
    }
}

fn track_px(f: &FieldTile, base: Color, px: i32, py: i32, gx: f32, gy: f32) -> Color {
    let _ = base;
    let soil = f.soil;
    let n = (f.link & 1) + (f.link >> 1 & 1) + (f.link >> 2 & 1) + (f.link >> 3 & 1);
    let packed = grain(soil[1], px, py, 0x81);
    if n >= 3 {
        // The yard: beaten earth with scraps of straw and the odd pebble.
        let h = tex::hash2(px, py, 0x82);
        return if h > 0.965 {
            palette::rgb(0xc8, 0xb0, 0x62)
        } else if h < 0.04 {
            palette::shade(packed, 0.86)
        } else {
            packed
        };
    }
    // A lane: two ruts with a grassy crown, along whichever way it runs.
    let along_x = (f.link & 3).count_ones() >= (f.link >> 2).count_ones();
    let across = if along_x { gy.fract() } else { gx.fract() };
    let d = (across - 0.28).abs().min((across - 0.72).abs());
    if d < 0.085 {
        palette::shade(soil[0], 0.95)
    } else if (across - 0.5).abs() < 0.14 {
        // The crown: grass tufts over earth.
        if tex::hash2(px, py, 0x83) > 0.45 {
            palette::rgb(0x72, 0x96, 0x46)
        } else {
            palette::shade(soil[1], 1.04)
        }
    } else {
        packed
    }
}

// ---------------------------------------------------------------------------
// Standing things
// ---------------------------------------------------------------------------

/// The orchard's tree for a tile, when it carries one: on a staggered lattice
/// (every other tile, rows running across the screen).
pub fn orchard_prop(map: &TileMap, x: i32, y: i32, biome: map::Biome) -> Option<Prop> {
    let t = map.at(x, y);
    if t.kind != Kind::Field || map::crop_of(&t) != Crop::Orchard {
        return None;
    }
    if (x + y) % 2 != 0 {
        return None;
    }
    // Not at the very edge of the plot: a row of trees along the fence is a hedge.
    let h = map::rng::hash2(x, y, 0x0C4A);
    let species = match biome {
        map::Biome::Mediterranean | map::Biome::Volcanic => TreeSpecies::Olive,
        map::Biome::Desert => TreeSpecies::Palm,
        map::Biome::Tropical | map::Biome::Bamboo | map::Biome::Swamp | map::Biome::Islands | map::Biome::Lagoon => {
            if h > 0.5 { TreeSpecies::FanPalm } else { TreeSpecies::Flamboyant }
        }
        _ => {
            // Apple orchards mostly, a cherry block here and there (per field).
            let blk = map::rng::hash2(x / 4, y / 4, 0x0C4B);
            if blk > 0.7 { TreeSpecies::Cherry } else { TreeSpecies::Apple }
        }
    };
    Some(Prop::Tree { species, size: 0, variant: (h * 8.0) as u8 })
}

/// Position inside a tile (0..1 each way) as a screen point, flat approximation.
#[inline]
fn at(p: &[Pt; 4], tx: f32, ty: f32) -> (f32, f32) {
    (
        p[0].0 + (p[1].0 - p[0].0) * tx + (p[3].0 - p[0].0) * ty,
        p[0].1 + (p[1].1 - p[0].1) * tx + (p[3].1 - p[0].1) * ty,
    )
}

/// Maize stalks and vine rows, drawn as small upright things on the ground.
pub fn draw_stand(fb: &mut Framebuffer, map: &TileMap, x: i32, y: i32, p: &[Pt; 4], f: &FieldTile, look: &Ground) {
    match f.crop {
        Crop::Cereal if !f.staged && f.stage > 0.93 && f.stage < 1.5 => {
            // After the harvest: round bales of straw lying on the stubble.
            let h = tex::hash2(x, y, 0xBA1E);
            if h < 0.30 {
                let (tx, ty) = (0.25 + 0.5 * tex::hash2(x, y, 0xBA1F), 0.25 + 0.5 * tex::hash2(x, y, 0xBA20));
                let (sx, sy) = at(p, tx, ty);
                let (sx, sy) = (sx as i32, sy as i32);
                let straw = palette::rgb(0xd8, 0xbc, 0x6a);
                let dark = palette::rgb(0xa8, 0x8a, 0x48);
                // A cylinder on its side: a body, a lit top, a round dark end.
                fb.fill_rect(sx - 5, sy - 5, 11, 6, straw);
                fb.fill_rect(sx - 5, sy - 5, 11, 1, palette::shade(straw, 1.15));
                fb.fill_rect(sx - 5, sy, 11, 1, dark);
                for k in 0..3 {
                    fb.put(sx - 3 + k * 3, sy - 3, dark);
                }
                tex::ellipse_px(fb, (sx + 5) as f32, (sy - 2) as f32, 2.0, 3.0, |_, _, _| palette::rgb(0xc2, 0xa2, 0x54));
                fb.put(sx + 5, sy - 2, dark);
            }
        }
        Crop::Maize => {
            let cov = cover(f.stage);
            if cov < 0.30 || f.stage > 0.93 {
                // After the harvest only stumps: nothing stands.
                return;
            }
            let rows = 4;
            let h = (2.0 + 7.0 * smooth(0.12, 0.55, f.stage)) as i32;
            let gold = ripe(f.stage, 0.10);
            let leaf = palette::mix(palette::rgb(0x3f, 0x82, 0x2e), palette::rgb(0xbb, 0x9c, 0x44), gold);
            let leaf_hi = palette::mix(palette::rgb(0x62, 0xa4, 0x42), palette::rgb(0xd8, 0xbc, 0x5c), gold);
            let leaf_lo = palette::shade(leaf, 0.72);
            for r in 0..rows {
                let rc = (r as f32 + 0.58) / rows as f32;
                for k in 0..5 {
                    let ac = (k as f32 + 0.5) / 5.0;
                    let (tx, ty) = if f.along_x { (ac, rc) } else { (rc, ac) };
                    let jit = tex::hash2(x * 8 + r * 3 + k, y * 8 + k, 0x91);
                    if jit < 0.12 {
                        continue;
                    }
                    let (sx, sy) = at(p, tx, ty);
                    let (sx, sy) = (sx as i32, sy as i32 + 1);
                    let hh = h + (jit * 2.0) as i32;
                    // A stalk: dark core, lit left edge, a leaf blade out each side.
                    fb.fill_rect(sx, sy - hh, 2, hh, leaf);
                    fb.fill_rect(sx, sy - hh, 1, hh, leaf_hi);
                    fb.put(sx + 2, sy - hh + 1, leaf_lo);
                    fb.put(sx - 1, sy - hh / 2, leaf_hi);
                    fb.put(sx + 2, sy - hh / 2 - 1, leaf_lo);
                    if gold > 0.6 && hh > 5 {
                        // The cob.
                        fb.put(sx + 2, sy - hh / 2 + 1, palette::rgb(0xe8, 0xc8, 0x50));
                    }
                }
            }
        }
        Crop::Vineyard => {
            // Posts and a wire at the end of each row segment; a band of leaf on the wire.
            let leaf_amt = (1.0 - smooth(0.0, 0.14, f.stage)) * 0.0
                + smooth(0.14, 0.40, f.stage) * (1.0 - smooth(0.93, 0.99, f.stage));
            let autumn = smooth(0.78, 0.95, f.stage);
            let leaf = palette::mix(palette::rgb(0x4e, 0x92, 0x36), palette::rgb(0xc2, 0x7a, 0x2c), autumn);
            let leaf_hi = palette::mix(palette::rgb(0x72, 0xb4, 0x48), palette::rgb(0xe0, 0xa8, 0x40), autumn);
            let leaf_lo = palette::shade(leaf, 0.70);
            let rows = 3;
            let post = palette::shade(look.trunk, 0.9);
            for r in 0..rows {
                let rc = (r as f32 + 0.5) / rows as f32;
                let a0 = if f.along_x { at(p, 0.0, rc) } else { at(p, rc, 0.0) };
                let a1 = if f.along_x { at(p, 1.0, rc) } else { at(p, rc, 1.0) };
                // The vine band: a thick, bumpy line a little above the soil.
                let steps = 14;
                for i in 0..=steps {
                    let t = i as f32 / steps as f32;
                    let (sx, sy) = (a0.0 + (a1.0 - a0.0) * t, a0.1 + (a1.1 - a0.1) * t);
                    let (sx, sy) = (sx as i32, sy as i32);
                    if leaf_amt > 0.15 {
                        let bump = (tex::hash2(sx, sy, 0x92) * 2.0) as i32;
                        fb.fill_rect(sx - 1, sy - 5 - bump, 3, 5 + bump, leaf);
                        fb.put(sx - 1, sy - 5 - bump, leaf_hi);
                        fb.put(sx + 1, sy - 1, leaf_lo);
                        if tex::hash2(sx, sy, 0x93) > 0.86 && autumn < 0.3 {
                            // A bunch of grapes.
                            fb.put(sx, sy - 1, palette::rgb(0x5a, 0x2c, 0x6e));
                        }
                    } else if i % 3 == 0 {
                        // Bare cane on the wire.
                        fb.put(sx, sy - 4, palette::shade(look.trunk, 1.1));
                        fb.put(sx, sy - 3, post);
                    }
                }
                // Posts at both ends and one in the middle.
                for t in [0.0f32, 0.5, 1.0] {
                    let (sx, sy) = (a0.0 + (a1.0 - a0.0) * t, a0.1 + (a1.1 - a0.1) * t);
                    fb.fill_rect(sx as i32, sy as i32 - 6, 1, 7, post);
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Edges: hedges, fences and walls
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    None,
    Hedge,
    Fence,
    Wall,
    /// A low strip of grass and a line of bushes between two crops.
    Balk,
    /// The bund round a paddy.
    Bund,
}

fn edge_kind(map: &TileMap, x: i32, y: i32, nx: i32, ny: i32, crop: Crop, biome: map::Biome) -> Edge {
    if !map.in_bounds(nx, ny) {
        return Edge::None;
    }
    let n = map.at(nx, ny);
    if n.is_water() {
        return Edge::None;
    }
    let ncrop = if n.kind == Kind::Field { Some(map::crop_of(&n)) } else { None };
    if ncrop == Some(crop) {
        return Edge::None;
    }
    // A track is open to its own fields: no hedge between a field and the lane
    // that serves it, except on the side of the road proper.
    if crop == Crop::Track || ncrop == Some(Crop::Track) {
        return Edge::None;
    }
    // Each shared boundary is drawn once: by the tile with the lower crop code.
    if let Some(nc) = ncrop {
        if (crop as u8) > (nc as u8) {
            return Edge::None;
        }
    }
    let h = map::rng::hash2(x.min(nx) * 2 + (nx - x).abs(), y.min(ny) * 2 + (ny - y).abs(), 0xED6E);
    if crop == Crop::Paddy {
        return Edge::Bund;
    }
    let upland = matches!(biome, map::Biome::Highlands | map::Biome::Alpine);
    if ncrop.is_some() {
        return if crop == Crop::Pasture || ncrop == Some(Crop::Pasture) {
            if upland { Edge::Wall } else { Edge::Fence }
        } else {
            Edge::Balk
        };
    }
    // Field against open ground or a street.
    if crop == Crop::Pasture {
        return if upland || h > 0.7 { Edge::Wall } else { Edge::Fence };
    }
    if upland {
        Edge::Wall
    } else if h < 0.62 {
        Edge::Hedge
    } else if h < 0.86 {
        Edge::Fence
    } else {
        Edge::Wall
    }
}

/// Hedges, fences and walls on the sides of this field tile that meet open
/// ground, a street or another crop.
pub fn draw_edges(fb: &mut Framebuffer, map: &TileMap, x: i32, y: i32, p: &[Pt; 4], f: &FieldTile, look: &Ground, season_bare: f32, snow: f32) {
    for (a, b, dx, dy) in [(0usize, 1usize, 0i32, -1i32), (1, 2, 1, 0), (2, 3, 0, 1), (3, 0, -1, 0)] {
        let kind = edge_kind(map, x, y, x + dx, y + dy, f.crop, look.biome);
        if kind == Edge::None {
            continue;
        }
        let (pa, pb) = (p[a], p[b]);
        match kind {
            Edge::Hedge => hedge(fb, pa, pb, x, y, a, look, season_bare, snow),
            Edge::Fence => fence(fb, pa, pb, look, snow),
            Edge::Wall => wall(fb, pa, pb, x, y, a, look, snow),
            Edge::Balk => balk(fb, pa, pb, x, y, a, look, season_bare, snow),
            Edge::Bund => bund(fb, pa, pb, f, snow),
            Edge::None => {}
        }
    }
}

fn hedge(fb: &mut Framebuffer, pa: Pt, pb: Pt, x: i32, y: i32, side: usize, look: &Ground, bare: f32, snow: f32) {
    let len = ((pb.0 - pa.0).powi(2) + (pb.1 - pa.1).powi(2)).sqrt();
    let n = (len / (4.0 * S as f32)).round().max(2.0) as i32;
    // A hedge in winter is bare twigs with a dark core, not green.
    let leaf = palette::mix(look.bush[0], palette::rgb(0x6a, 0x5a, 0x3c), bare * 0.55);
    let leaf2 = palette::mix(look.bush[1], palette::rgb(0x4e, 0x42, 0x2c), bare * 0.55);
    for k in 0..=n {
        let t = k as f32 / n as f32;
        let h = tex::hash2(x * 5 + side as i32, y * 5 + k, 0xE0);
        if h < 0.07 {
            continue; // a gap
        }
        let cx = (pa.0 + (pb.0 - pa.0) * t) as i32;
        let cy = (pa.1 + (pb.1 - pa.1) * t) as i32;
        let hh = (6.0 + h * 5.0) as i32;
        let w = (5.0 + h * 3.0) as i32;
        tex::ellipse_px(fb, cx as f32, (cy - hh / 2) as f32, w as f32 / 2.0 + 1.0, hh as f32 / 2.0, |px, py, _d| {
            let n = tex::hash2(px, py, 0xE1);
            if n > 0.66 { palette::shade(leaf, 1.22) } else if n > 0.30 { leaf } else { leaf2 }
        });
        fb.put(cx - 1, cy - hh + 1, palette::shade(leaf, 1.32));
        if snow > 0.05 {
            for dx in -w / 2..=w / 2 {
                fb.put(cx + dx, cy - hh, palette::rgb(0xec, 0xf3, 0xfa));
            }
        }
        // A tree standing in the hedge now and then.
        if h > 0.93 {
            let trunk = look.trunk;
            fb.fill_rect(cx, cy - hh - 4, 2, hh + 4, trunk);
            tex::ellipse_px(fb, cx as f32 + 1.0, (cy - hh - 7) as f32, 6.0, 5.0, |px, py, _d| {
                let n = tex::hash2(px, py, 0xE2);
                if n > 0.6 { palette::shade(look.leaf[0], 1.1) } else if n > 0.25 { look.leaf[1] } else { look.leaf[2] }
            });
        }
    }
}

fn fence(fb: &mut Framebuffer, pa: Pt, pb: Pt, look: &Ground, snow: f32) {
    let post_h = 4 * S;
    let wood = look.trunk;
    let len = ((pb.0 - pa.0).powi(2) + (pb.1 - pa.1).powi(2)).sqrt();
    if len < 4.0 {
        return;
    }
    let rail = palette::shade(wood, 1.20);
    let rail_s = palette::shade(wood, 0.82);
    for (frac, col) in [(0.95f32, rail), (0.50, rail_s)] {
        let lift = post_h as f32 * frac;
        fb.line((pa.0, pa.1 - lift), (pb.0, pb.1 - lift), col);
    }
    let posts = (len / (5.0 * S as f32)).round().max(2.0) as i32;
    for k in 0..=posts {
        let t = k as f32 / posts as f32;
        let px = (pa.0 + (pb.0 - pa.0) * t).round() as i32;
        let py = (pa.1 + (pb.1 - pa.1) * t).round() as i32;
        fb.fill_rect(px, py - post_h, S.max(1), post_h + 1, wood);
        fb.fill_rect(px, py - post_h, 1, post_h, palette::shade(wood, 1.15));
        fb.put(px, py + 1, palette::shade(wood, 0.8));
        if snow > 0.05 {
            fb.put(px, py - post_h - 1, palette::rgb(0xec, 0xf3, 0xfa));
        }
    }
}

fn wall(fb: &mut Framebuffer, pa: Pt, pb: Pt, x: i32, y: i32, side: usize, look: &Ground, snow: f32) {
    // A dry-stone wall: a low band of stones, each a few pixels, mortar-less.
    let stone = [
        palette::rgb(0xa4, 0x9c, 0x8c),
        palette::rgb(0x8c, 0x85, 0x78),
        palette::rgb(0xb8, 0xb0, 0x9e),
    ];
    let _ = look;
    let len = ((pb.0 - pa.0).powi(2) + (pb.1 - pa.1).powi(2)).sqrt();
    let n = (len * 1.0) as i32;
    let hgt = 4 + (S);
    for k in 0..=n {
        let t = k as f32 / n.max(1) as f32;
        let cx = (pa.0 + (pb.0 - pa.0) * t).round() as i32;
        let cy = (pa.1 + (pb.1 - pa.1) * t).round() as i32;
        for r in 0..hgt {
            let course = r / 2;
            let off = (course * 3 + x + y * 2 + side as i32) as i32;
            let col_stone = ((cx + off) / 3) as i32;
            let h = tex::hash2(col_stone, course + x * 7 + y * 13, 0xE7);
            let mut c = stone[(h * 3.0) as usize % 3];
            if (cx + off) % 3 == 0 {
                c = palette::shade(c, 0.78); // the joint
            }
            if r == hgt - 1 {
                c = palette::shade(c, 1.12); // the capstones catch the light
            }
            if r == 0 {
                c = palette::shade(c, 0.80);
            }
            fb.put(cx, cy - r, c);
        }
        if snow > 0.05 {
            fb.put(cx, cy - hgt, palette::rgb(0xec, 0xf3, 0xfa));
        }
    }
}

fn balk(fb: &mut Framebuffer, pa: Pt, pb: Pt, x: i32, y: i32, side: usize, look: &Ground, bare: f32, snow: f32) {
    // A strip of rough grass and, now and then, a shrub between two crops.
    let len = ((pb.0 - pa.0).powi(2) + (pb.1 - pa.1).powi(2)).sqrt();
    let n = len as i32;
    let g = palette::mix(look.blade, palette::rgb(0x8a, 0x8a, 0x52), bare * 0.5);
    for k in 0..=n {
        let t = k as f32 / n.max(1) as f32;
        let cx = (pa.0 + (pb.0 - pa.0) * t).round() as i32;
        let cy = (pa.1 + (pb.1 - pa.1) * t).round() as i32;
        let h = tex::hash2(cx, cy, 0xE9);
        let up = if h > 0.5 { 2 } else { 1 };
        for r in 0..up {
            fb.put(cx, cy - r, if r == 1 { palette::shade(g, 1.12) } else { palette::shade(g, 0.86) });
        }
        if snow > 0.05 {
            fb.put(cx, cy - up, palette::rgb(0xec, 0xf3, 0xfa));
        }
    }
    let sh = tex::hash2(x * 3 + side as i32, y * 3, 0xEA);
    if sh > 0.55 {
        let t = (sh - 0.55) / 0.45;
        let cx = (pa.0 + (pb.0 - pa.0) * t) as i32;
        let cy = (pa.1 + (pb.1 - pa.1) * t) as i32;
        tex::ellipse_px(fb, cx as f32, (cy - 3) as f32, 3.5, 3.0, |px, py, _d| {
            if tex::hash2(px, py, 0xEB) > 0.6 { look.bush[0] } else { look.bush[1] }
        });
    }
}

fn bund(fb: &mut Framebuffer, pa: Pt, pb: Pt, f: &FieldTile, snow: f32) {
    let _ = f;
    // The raised earth bank round a paddy, grass on top.
    let len = ((pb.0 - pa.0).powi(2) + (pb.1 - pa.1).powi(2)).sqrt();
    let n = len as i32;
    for k in 0..=n {
        let t = k as f32 / n.max(1) as f32;
        let cx = (pa.0 + (pb.0 - pa.0) * t).round() as i32;
        let cy = (pa.1 + (pb.1 - pa.1) * t).round() as i32;
        let h = tex::hash2(cx, cy, 0xEC);
        fb.put(cx, cy, palette::rgb(0x8a, 0x6c, 0x44));
        fb.put(cx, cy - 1, if h > 0.4 { palette::rgb(0x84, 0xa8, 0x4c) } else { palette::rgb(0x6c, 0x8e, 0x3c) });
        if snow > 0.05 {
            fb.put(cx, cy - 2, palette::rgb(0xec, 0xf3, 0xfa));
        }
    }
}
