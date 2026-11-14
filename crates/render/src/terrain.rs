//! Terrain drawing: corner-height tiles as two shaded triangles, cliff faces,
//! water, and the vegetation standing on top.
//!
//! Shading is the whole point of B3/Q1: a 2:1 iso projection cannot show a
//! one-level slope geometrically (it cancels), so slopes are read from the
//! shade of each triangle's normal against a light from the upper left.
//!
//! B32/D54: every surface carries texture at 64-px tile detail — grass tufts,
//! soil grain and pebbles, rock strata, beach ripples, paving joints, asphalt
//! with tyre tracks — and the details (kerbs, markings, manholes, drains,
//! street lamps, crop rows, fences) are drawn at the new scale. The texture
//! lives in [`crate::tex`]: a fill with a per-pixel closure, addressed in tile
//! space so joints and rows line up with the grid.

use map::{Dir, Kind, Prop, Tile, TileMap, TreeKind, TreeSpecies};

use crate::biome::{Ground, GroundTex, Pair};
use crate::framebuffer::Framebuffer;
use crate::iso::{Camera, Pt, level_px};
use crate::palette::{self, Color, pal};
use crate::softground::soft_kind;
use crate::tex;

/// Art detail multiplier (2 at 64-px tiles): hand-tuned pixel sizes from the
/// 32-px art are multiplied by this.
const S: i32 = map::iso_consts::DETAIL;

/// B83: the carriageway's own asphalt, a shade darker and cooler than the
/// palette's road grey so a kerb and a pavement read against it.
const CARRIAGEWAY: Color = palette::rgb(0x50, 0x55, 0x5c);
/// The pavement's paving stones (between the asphalt and the old `SIDEWALK`,
/// which is bright enough to read as a painted line along a whole street).
const PAVING: Color = palette::rgb(0xa3, 0xa1, 0x99);
/// The lit top of a kerb stone, and the shadow it throws on the asphalt.
const KERB_TOP: Color = palette::rgb(0xc6, 0xc4, 0xb8);
const KERB_SHADOW: Color = palette::rgb(0x39, 0x3d, 0x43);

/// Light direction in tile space (x, y in tiles, z in half-levels), pointing
/// *toward* the light: from the upper left of the screen.
const LIGHT: [f32; 3] = [-0.82, -0.27, 0.58];

/// The unit vector toward the light (B70: the smooth-ground shading uses the
/// same direction as the per-triangle path).
pub fn light_vector() -> [f32; 3] {
    normalize(LIGHT)
}

/// Corner positions in tile space.
const CORNER_POS: [(f32, f32); 4] = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];

/// The two triangles of a tile top, as corner index triples.
#[inline]
pub fn triangles(diagonal: u8) -> [(usize, usize, usize); 2] {
    if diagonal == 0 {
        [(0, 1, 2), (0, 2, 3)]
    } else {
        [(0, 1, 3), (1, 2, 3)]
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / n, v[1] / n, v[2] / n]
}

/// Light value of one triangle: `n · L` in roughly `0.1 ..= 0.85`.
fn triangle_light(c: [u8; 4], tri: (usize, usize, usize)) -> f32 {
    // z in half-levels: one level is a tile step's worth of height.
    let z = |i: usize| c[i] as f32 * 0.5;
    let (i0, i1, i2) = tri;
    let p0 = [CORNER_POS[i0].0, CORNER_POS[i0].1, z(i0)];
    let p1 = [CORNER_POS[i1].0, CORNER_POS[i1].1, z(i1)];
    let p2 = [CORNER_POS[i2].0, CORNER_POS[i2].1, z(i2)];
    let u = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let v = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
    let mut n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    if n[2] < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    let n = normalize(n);
    let l = normalize(LIGHT);
    n[0] * l[0] + n[1] * l[1] + n[2] * l[2]
}

/// Turn a light value into a ramp index, with a faint height tint and per-tile
/// jitter so large faces do not look like flat paint.
fn shade_index(light: f32, level: u8, jitter: f32) -> usize {
    let tint = (level as f32 / map::MAX_LEVEL as f32 - 0.5) * 0.10;
    palette::ramp_index(light + tint + jitter)
}

// ---------------------------------------------------------------------------
// surface textures (D54)
// ---------------------------------------------------------------------------

/// A small per-tile offset for repeating textures, from the screen pixel (which
/// is unique inside a tile only to within the tile — so it is taken from a
/// coarse 64-px cell, i.e. one tile at 1x).
#[inline]
fn phase_shift(px: i32, py: i32, salt: u32) -> f32 {
    (tex::hash2(px >> 6, py >> 6, salt) - 0.5) * 0.55
}

/// Fine per-pixel grain: two steps either side of the base, deterministic.
#[inline]
fn grain(c: Color, px: i32, py: i32, salt: u32) -> Color {
    let h = tex::hash2(px, py, salt);
    if h < 0.13 {
        palette::shade(c, 0.94)
    } else if h > 0.90 {
        palette::shade(c, 1.05)
    } else {
        c
    }
}

/// Grass: 4-px tall tufts plus fine grain, so a meadow has direction and depth
/// instead of flat paint (the H25 spike's grass blades, at 1×).
#[inline]
fn grass_px(base: Color, px: i32, py: i32) -> Color {
    let tuft = tex::hash2(px, py / 4, 0x51);
    let mut c = base;
    if tuft < 0.20 {
        c = palette::shade(c, 1.07); // a sunlit blade
    } else if tuft > 0.88 {
        c = palette::shade(c, 0.94); // the shade between tufts
    }
    let g = tex::hash2(px, py, 0x52);
    if g > 0.97 {
        c = palette::shade(c, 1.13);
    } else if g < 0.03 {
        c = palette::shade(c, 0.90);
    }
    c
}

/// Snow: a cold sparkle over blue shadow, drifts running with the wind (D66).
#[inline]
fn snow_px(base: Color, px: i32, py: i32) -> Color {
    let drift = tex::hash2(px / 3, py / 6, 0x71);
    let mut c = base;
    if drift > 0.72 {
        c = palette::shade(c, 1.02);
    } else if drift < 0.20 {
        c = palette::shade(c, 0.97);
    }
    let spark = tex::hash2(px, py, 0x72);
    if spark > 0.975 {
        c = palette::rgb(0xff, 0xff, 0xff);
    } else if spark < 0.05 {
        c = palette::shade(c, 0.94);
    }
    c
}

/// Jungle ground: broad wet blades, darker in the shade.
#[inline]
fn lush_px(base: Color, px: i32, py: i32) -> Color {
    let blade = tex::hash2(px, py / 3, 0x73);
    let mut c = base;
    if blade < 0.30 {
        c = palette::shade(c, 1.10);
    } else if blade > 0.80 {
        c = palette::shade(c, 0.90);
    }
    let g = tex::hash2(px, py, 0x74);
    if g > 0.96 {
        c = palette::shade(c, 1.14);
    }
    c
}

/// Bare soil: grain plus a scatter of pebbles and darker clods.
#[inline]
fn soil_px(base: Color, px: i32, py: i32) -> Color {
    let p = tex::hash2(px / 2, py / 2, 0x77);
    let mut c = grain(base, px, py, 0x73);
    if p > 0.94 {
        c = palette::shade(c, 1.16); // pebble
    } else if p < 0.06 {
        c = palette::shade(c, 0.86); // clod
    }
    c
}

/// Gravel: heavier, coarser speckle than soil.
#[inline]
fn gravel_px(base: Color, px: i32, py: i32) -> Color {
    let p = tex::hash2(px / 2, py, 0x79);
    let c = if p > 0.72 {
        palette::shade(base, 1.10)
    } else if p < 0.28 {
        palette::shade(base, 0.90)
    } else {
        base
    };
    grain(c, px, py, 0x7a)
}

/// One octave of bilinear value noise on a `(sx, sy)`-pixel lattice, 0..1. Cheap
/// enough for a per-pixel texture (the 2-octave `fbm` was a third of a rebuild).
#[inline]
fn vnoise(px: i32, py: i32, sx: f32, sy: f32, salt: u32) -> f32 {
    let (x, y) = (px as f32 / sx, py as f32 / sy);
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let h = |i: i32, j: i32| tex::hash2(i, j, salt);
    let top = h(xi, yi) * (1.0 - fx) + h(xi + 1, yi) * fx;
    let bot = h(xi, yi + 1) * (1.0 - fx) + h(xi + 1, yi + 1) * fx;
    top * (1.0 - fy) + bot * fy
}

/// Rock: strata bands (lines of constant screen row, so a cliff face and a flat
/// top share one bedding) with a wobble, a dark seam between bands, mottled
/// patches and sparse grit.
///
/// B70: the mottling is noise in screen space, not a hash on a 6x4 cell grid.
/// The cell grid read as a wall of bricks across a whole plateau; blotches of
/// two tones with ragged outlines read as weathered stone.
#[inline]
fn rock_px(base: Color, px: i32, py: i32, u: f32, v: f32, seam: Color) -> Color {
    let s = py as f32 / 6.0 + ((u - v) * 4.0).sin() * 0.30;
    let band = s.floor();
    let frac = s - band;
    let mut c = if (band as i32) & 1 == 0 {
        palette::shade(base, 1.03)
    } else {
        palette::shade(base, 0.96)
    };
    if frac < 0.10 {
        c = palette::mix(c, seam, 0.30); // the seam between strata
    }
    // Mottling: wide flat blotches (stretched along the bedding).
    let m = vnoise(px, py, 14.0, 6.0, 0x79);
    if m > 0.62 {
        c = palette::shade(c, 1.06);
    } else if m < 0.36 {
        c = palette::shade(c, 0.94);
    }
    // A few dark cracks (short horizontal runs) and a scatter of grit.
    let crack = tex::hash2(px / 11, py / 5, 0x7b);
    if crack > 0.95 && tex::hash2(px, py, 0x7d) > 0.35 {
        c = palette::mix(c, seam, 0.45);
    }
    let grit = tex::hash2(px, py, 0x7c);
    if grit > 0.93 {
        c = palette::shade(c, 1.10);
    } else if grit < 0.04 {
        c = palette::shade(c, 0.90);
    }
    c
}

/// Beach sand: ripple bands curving across the tile, plus grain.
///
/// B46c: the ripples are laid out in **screen** space, not tile space. A slope
/// that runs along the view diagonal projects each of its triangles to a
/// vertical sliver, and a tile-space texture in that sliver collapses into a
/// vertical smear of stretched colour — that is the "column of smeared colour"
/// that survived three rounds of the biome review. Sampling from the pixel puts
/// the ripples on the frame, where no projection can stretch them.
#[inline]
fn sand_px(base: Color, px: i32, py: i32, _u: f32, _v: f32) -> Color {
    // B89: soft windblown mottling, not the fine diagonal stripes that read as
    // scan lines: two scales of noise lift and dim broad patches a few percent,
    // with the odd pale grain.
    let broad = vnoise(px, py, 46.0, 24.0, 0x8A1);
    let fine = vnoise(px, py, 11.0, 6.0, 0x8A2);
    let m = broad * 0.7 + fine * 0.3;
    let c = if m > 0.58 {
        palette::shade(base, 1.04)
    } else if m < 0.40 {
        palette::shade(base, 0.955)
    } else {
        base
    };
    grain(c, px, py, 0x8b)
}

/// Sidewalk paving: slabs in both tile directions with joints and per-slab tint.
#[inline]
fn paving_px(base: Color, px: i32, py: i32, u: f32, v: f32) -> Color {
    let (nu, nv) = (6.0, 6.0);
    let ju = (u * nu).fract();
    let jv = (v * nv).fract();
    let su = (u * nu).floor() as i32;
    let sv = (v * nv).floor() as i32;
    let slab = tex::hash2(su, sv, 0x91);
    let c = palette::jitter(base, slab, 0.045);
    if ju < 0.09 || jv < 0.09 {
        palette::shade(c, 0.84) // the joint
    } else if ju > 0.93 || jv > 0.93 {
        palette::shade(c, 1.05) // the chamfer
    } else {
        grain(c, px, py, 0x93)
    }
}

/// Asphalt: grain plus the two tyre tracks a used carriageway always shows.
#[inline]
fn asphalt_px(base: Color, px: i32, py: i32, across: f32, tracks: bool) -> Color {
    let mut c = grain(base, px, py, 0x97);
    if tracks {
        // Two darker bands where wheels run; a faint centre sheen between them.
        let d = (across - 0.30).abs().min((across - 0.70).abs());
        if d < 0.075 {
            c = palette::shade(c, 0.93);
        } else if (across - 0.5).abs() < 0.05 {
            c = palette::shade(c, 1.04);
        }
    }
    // Tar patches and cracks: rare, and they read as a real road.
    let p = tex::hash2(px / 3, py / 3, 0x98);
    if p > 0.975 {
        c = palette::shade(c, 0.88);
    }
    c
}

/// B89: what a water tile's pixels are painted from. The sea is banded by
/// *depth* (the distance to the nearest land, one smooth field across tile
/// boundaries) instead of being one flat blue: thin turquoise over the sand,
/// two shallow steps, open water, and deep pockets far out, the steps dithered
/// into each other in a Bayer pattern so they read as hand-placed pixel bands.
#[derive(Clone, Copy)]
struct WaterBake {
    /// Distance to land, in tiles, at the tile's corners (0,0) (1,0) (1,1) (0,1).
    depth: [f32; 4],
    /// Thin shallows, shallows, mid shallows, open water, deep water.
    band: [Color; 5],
    /// How fast the water deepens off the shore (lakes shelve faster).
    scale: f32,
    /// The sea (and its lagoons) rolls with long swells; a pond hardly does.
    swell: f32,
    /// B92: the smooth coast field, where land is within reach of this tile.
    coast: Option<Coast>,
    /// A pond or lake: still water that carries lily pads.
    still: bool,
}

/// Distance in tiles from each corner of tile `(x, y)` to the nearest land
/// tile within four tiles (9 when there is none).
fn water_depth(map: &TileMap, x: i32, y: i32) -> [f32; 4] {
    const R: i32 = 4;
    const N: usize = 9;
    let mut land = [[false; N]; N];
    let mut any = false;
    for j in 0..N {
        for i in 0..N {
            let t = map.at(x - R + i as i32, y - R + j as i32);
            let l = !(t.is_water() || t.kind == Kind::Bridge);
            land[j][i] = l;
            any |= l;
        }
    }
    if !any {
        return [9.0; 4];
    }
    let mut out = [9.0f32; 4];
    for (k, (cx, cy)) in [(0i32, 0i32), (1, 0), (1, 1), (0, 1)].into_iter().enumerate() {
        let (qx, qy) = (R + cx, R + cy);
        let mut best = 81.0f32;
        for j in 0..N {
            let dy = ((j as i32 - qy).max(qy - (j as i32 + 1))).max(0) as f32;
            for i in 0..N {
                if !land[j][i] {
                    continue;
                }
                let dx = ((i as i32 - qx).max(qx - (i as i32 + 1))).max(0) as f32;
                let d2 = dx * dx + dy * dy;
                if d2 < best {
                    best = d2;
                }
            }
        }
        out[k] = best.sqrt();
    }
    out
}

/// B92: the smoothed coast. The tile map says land or water per tile, so every
/// shore drawn from it is a staircase of tile edges. A pixel instead asks this
/// field, which is the land mask blurred over a couple of tiles and read
/// bilinearly between tile centres; its 0.5 contour is a smooth curve through
/// the staircase (and stays on the tile edge where the coast is straight).
#[derive(Clone, Copy)]
pub(crate) struct Coast {
    /// The field at the centres of the 3x3 tiles around this one: `f[j][i]` is
    /// tile `(x - 1 + i, y - 1 + j)`.
    f: [[f32; 3]; 3],
    /// A water tile with built or steep land (or a bridge) near it: no beach is
    /// laid into it, the coast keeps its straight edge there.
    hard: bool,
    /// A bridge deck is within a tile: land beside it is left alone.
    bridge: bool,
}

/// How much of the field is the tile itself (the rest is the blur). More blur
/// rounds the coast further but eats one-tile islets.
const COAST_SELF: f32 = 0.66;

/// Land that the coast may reshape: open soft ground at (or one level above)
/// the water.
fn soft_low(t: &Tile, wl: u8) -> bool {
    matches!(
        t.kind,
        Kind::Grass | Kind::Meadow | Kind::Upland | Kind::DryGrass | Kind::Sand
    ) && t.max_level() <= wl.saturating_add(1)
}

/// The coast field around tile `(x, y)`; `None` when the nine tiles are all
/// land or all water. Also the level of the water seen.
fn coast_of(map: &TileMap, x: i32, y: i32) -> Option<(Coast, u8)> {
    const K: [f32; 7] = [1.0, 6.0, 15.0, 20.0, 15.0, 6.0, 1.0];
    // Cheap first: most tiles have no land/water mix in their 3x3.
    let first = map.at(x - 1, y - 1).is_water() || map.at(x - 1, y - 1).kind == Kind::Bridge;
    let mut mixed = false;
    'scan: for j in -1..=1 {
        for i in -1..=1 {
            let t = map.at(x + i, y + j);
            if (t.is_water() || t.kind == Kind::Bridge) != first {
                mixed = true;
                break 'scan;
            }
        }
    }
    if !mixed {
        return None;
    }
    let mut land = [[0.0f32; 9]; 9];
    let (mut any_land, mut any_water) = (false, false);
    let mut wl: Option<u8> = None;
    let mut bridge = false;
    for j in 0..9 {
        for i in 0..9 {
            let t = map.at(x - 4 + i as i32, y - 4 + j as i32);
            let l = !(t.is_water() || t.kind == Kind::Bridge);
            land[j][i] = if l { 1.0 } else { 0.0 };
            if (3..6).contains(&i) && (3..6).contains(&j) {
                if l {
                    any_land = true;
                } else {
                    any_water = true;
                }
                if t.kind == Kind::Bridge {
                    bridge = true;
                }
                if t.is_water() && wl.is_none() {
                    wl = Some(t.corner[0]);
                }
            }
        }
    }
    if !(any_land && any_water) {
        return None;
    }
    let wl = wl.unwrap_or(1);
    let hard = bridge;
    // A binomial blur over seven tiles ([1 6 15 20 15 6 1] / 64 each way).
    let mut f = [[0.0f32; 3]; 3];
    for b in 0..3 {
        for a in 0..3 {
            let mut g = 0.0;
            for dj in 0..7 {
                for di in 0..7 {
                    g += K[di] * K[dj] * land[b + dj][a + di];
                }
            }
            g /= 4096.0;
            f[b][a] = COAST_SELF * land[b + 3][a + 3] + (1.0 - COAST_SELF) * g;
        }
    }
    Some((Coast { f, hard, bridge }, wl))
}

/// Signed distance to the coast in tile units at tile position `(u, v)`
/// (positive on the land side), warped by a low-frequency noise so no two
/// stretches of coast are alike.
#[inline]
fn coast_s(c: &Coast, u: f32, v: f32, px: i32, py: i32) -> f32 {
    let w = (vnoise(px, py, 44.0, 26.0, 0xC0A5) - 0.5) * 0.30
        + (vnoise(px, py, 13.0, 8.0, 0xC0A6) - 0.5) * 0.10;
    coast_raw(c, u, v) + w
}

/// [`coast_s`] without the warp: the plain smooth curve (for the animation,
/// which only needs a line near the coast).
#[inline]
fn coast_raw(c: &Coast, u: f32, v: f32) -> f32 {
    let (i0, tu) = if u < 0.5 { (0usize, u + 0.5) } else { (1usize, u - 0.5) };
    let (j0, tv) = if v < 0.5 { (0usize, v + 0.5) } else { (1usize, v - 0.5) };
    let top = c.f[j0][i0] * (1.0 - tu) + c.f[j0][i0 + 1] * tu;
    let bot = c.f[j0 + 1][i0] * (1.0 - tu) + c.f[j0 + 1][i0 + 1] * tu;
    let f = top * (1.0 - tv) + bot * tv;
    (f - 0.5) * (1.0 / 0.75)
}

/// The colours and depth of one water tile; `base` is the tile's flat colour
/// (class colour, sky tint, night). `None` for ice, which has its own look.
fn water_bake(map: &TileMap, x: i32, y: i32, base: Color, wl: &WaterLook) -> Option<WaterBake> {
    let kind = map.water_kind(x, y);
    let pair = Pair::of(map);
    let g = pair.at(map, x, y);
    let still = matches!(kind, map::WaterKind::Lake | map::WaterKind::Pond);
    if still && g.ice.is_some() {
        return None;
    }
    let sea = matches!(kind, map::WaterKind::Sea | map::WaterKind::Lagoon);
    let n = wl.night_q as f32 / 16.0;
    // The land's shallows colour, taken through the same night as the sea.
    let shallow = palette::shade(palette::mix(g.shallow, wl.tint, 0.58 * n), 1.0 - 0.40 * n);
    let deep = if sea {
        // A touch of ultramarine so the far water is a *blue*, not a darker cyan.
        let blue = palette::shade(palette::rgb(0x1a, 0x52, 0xa6), 1.0 - 0.32 * n);
        palette::mix(palette::shade(base, 0.975), blue, 0.10)
    } else {
        palette::shade(palette::mix(base, pair.deep(map, x, y), 0.55), 0.84)
    };
    let (k0, k1, k2) = if sea { (0.90, 0.68, 0.30) } else { (0.62, 0.42, 0.18) };
    let mut band = [
        palette::shade(palette::mix(base, shallow, k0), 1.04),
        palette::mix(base, shallow, k1),
        palette::mix(base, shallow, k2),
        base,
        deep,
    ];
    // Overcast: the water goes grey and a little darker; a rough sea is darker too.
    let grey = (0.42 * wl.cloud).clamp(0.0, 0.5);
    let dark = 1.0 - 0.03 * wl.cloud - 0.10 * (wl.chop - 0.55).max(0.0);
    if grey > 0.01 || dark < 0.999 {
        for c in band.iter_mut() {
            let l = palette::luma(*c);
            *c = palette::shade(palette::mix(*c, palette::rgb(l, l, l), grey), dark);
        }
    }
    let (scale, swell) = match kind {
        map::WaterKind::Sea => (1.0, 1.0),
        map::WaterKind::Lagoon => (0.75, 0.7),
        map::WaterKind::Lake => (1.7, 0.55),
        map::WaterKind::Pond => (2.2, 0.35),
        _ => (2.0, 0.4),
    };
    Some(WaterBake {
        depth: water_depth(map, x, y),
        band,
        scale,
        swell,
        coast: coast_of(map, x, y).map(|c| c.0).filter(|c| !c.hard),
        still,
    })
}

/// One water pixel (B89): depth band by dithered thresholds on the smooth
/// depth field, then a soft swell lighter and darker over it. `u, v` is the
/// position in the tile, `px, py` the screen pixel.
#[inline]
fn water_px(wb: &WaterBake, px: i32, py: i32, u: f32, v: f32) -> Color {
    let d = &wb.depth;
    let top = d[0] * (1.0 - u) + d[1] * u;
    let bot = d[3] * (1.0 - u) + d[2] * u;
    let mut d0 = top * (1.0 - v) + bot * v;
    // B92: near the coast the depth is measured from the smooth coast, not
    // from the land tiles' edges, so the bands follow the curve too.
    if let Some(c) = &wb.coast {
        let dn = (-coast_s(c, u, v, px, py)).max(0.0);
        let m = ((d0 - 0.9) / 0.6).clamp(0.0, 1.0);
        d0 = dn * (1.0 - m) + d0 * m;
    }
    let mut z = d0 * wb.scale;
    if d0 < 5.0 {
        // The coast wanders: no band follows the tile diamonds.
        z += (vnoise(px, py, 26.0, 16.0, 0xD3AB) - 0.5) * 0.6;
    }
    // Far from any land: broad patches of open and deep water, two octaves so
    // their outlines wander instead of tracing one smooth blob.
    let patch = vnoise(px, py, 300.0, 150.0, 0xD4C1) * 0.7 + vnoise(px, py, 110.0, 55.0, 0xD4C2) * 0.3;
    z = z.min(2.4 + 1.6 * patch);
    let far = ((d0 - 1.5) / 2.5).clamp(0.0, 1.0);
    let bay = bayer4(px, py) - 0.5;
    const T: [f32; 4] = [0.42, 1.3, 2.4, 3.7];
    let mut i = 0usize;
    for (n, t) in T.into_iter().enumerate() {
        // The far steps dither over a wider band, so a deep patch fades in.
        let w = if n >= 2 { 0.22 + 0.55 * far } else { 0.22 };
        if z > t + bay * w {
            i += 1;
        }
    }
    let c = wb.band[i];
    // B92: a pond or lake has a darker, muddier rim where the bank shelves, and
    // lily pads on its edge water (small green ellipses, the odd pink bloom).
    let mut c = c;
    let mut padded = false;
    if wb.swell < 0.6 {
        if d0 < 0.15 {
            c = palette::mix(palette::shade(c, 0.84), palette::rgb(0x3a, 0x5a, 0x40), 0.18);
        }
        if wb.still && d0 > 0.14 && d0 < 1.1 {
            let (cx, cy) = (px.div_euclid(12), py.div_euclid(7));
            if tex::hash2(cx, cy, 0x1A1) < 0.16 {
                let ox = cx * 12 + 3 + (tex::hash2(cx, cy, 0x1A2) * 6.0) as i32;
                let oy = cy * 7 + 2 + (tex::hash2(cx, cy, 0x1A3) * 3.0) as i32;
                let (dx, dy) = ((px - ox) as f32, (py - oy) as f32);
                if dx * dx / 20.0 + dy * dy / 5.0 < 1.0 {
                    let bloom = tex::hash2(cx, cy, 0x1A4) > 0.82 && dx.abs() < 1.5 && dy.abs() < 1.0;
                    c = if bloom {
                        palette::rgb(0xf0, 0xa8, 0xc4)
                    } else if dx > 0.0 && dy.abs() < 0.8 {
                        palette::shade(c, 0.9) // the notch in the pad
                    } else {
                        palette::mix(palette::rgb(0x44, 0x92, 0x3c), c, 0.12 + 0.18 * (bay + 0.5))
                    };
                    padded = true;
                }
            }
        }
    }
    let _ = padded;
    // A faint broad swell: lens-shaped lighter and darker areas, dithered.
    let a = px as f32 / 64.0 + py as f32 / 32.0;
    let b = py as f32 / 32.0 - px as f32 / 64.0;
    let w1 = vnoise(px, py, 170.0, 85.0, 0xD5);
    let w2 = vnoise(px, py, 120.0, 60.0, 0xD6);
    let sw = (std::f32::consts::TAU * (0.5 * a + 2.4 * w1)).sin()
        * (std::f32::consts::TAU * (0.16 * b + 1.6 * w2)).sin();
    let mut c = c;
    if sw > 0.45 + 0.4 * bay {
        c = palette::shade(c, 1.0 + 0.03 * wb.swell);
    } else if sw < -0.55 + 0.4 * bay {
        c = palette::shade(c, 1.0 - 0.03 * wb.swell);
    }
    let mut gust = 0.0f32;
    // B92: the open sea rolls in long diagonal swell bands (darker troughs,
    // paler backs) and is combed by wind into horizontal cat's-paw streaks, so
    // a wide stretch of sea far from any land is never one flat blue. The
    // gusty patches (`gust`) are covered in wave marks, the calm ones bare.
    if wb.swell > 0.6 && far > 0.0 {
        let ra = (px as f32 * 0.45 + py as f32 * 0.9) as i32;
        let rb = (py as f32 * 0.45 - px as f32 * 0.9) as i32;
        let band = vnoise(ra, rb, 80.0, 300.0, 0xE1);
        let tb = (band - 0.5) * (0.5 + 0.5 * far);
        if tb > 0.10 + 0.20 * bay {
            c = palette::shade(c, 1.035);
        } else if tb < -0.10 + 0.20 * bay {
            c = palette::shade(c, 0.955);
        }
        gust = far * vnoise(px, py, 300.0, 120.0, 0xE5);
    }
    // Wave marks: short flat dashes laid in staggered rows (a brick pattern
    // whose bricks mostly stay empty, with their own length, height and
    // brightness), a lighter line over a darker one under it.
    let row = py.div_euclid(12);
    let off = (tex::hash2(row, 0, 0xD7) * 46.0) as i32;
    let cx = (px + off).div_euclid(46);
    if tex::hash2(cx, row, 0xD8) < 0.30 * wb.swell + 0.40 * (gust - 0.52).max(0.0) * 3.0 {
        let w = 6 + (tex::hash2(cx, row, 0xD9) * 10.0) as i32;
        let x0 = cx * 46 - off + (tex::hash2(cx, row, 0xDA) * (46 - w) as f32) as i32;
        let y0 = row * 12 + 1 + (tex::hash2(cx, row, 0xDB) * 8.0) as i32;
        let (lx, ly) = (px - x0, py - y0);
        if lx >= 0 && lx < w {
            let tone = tex::hash2(cx, row, 0xDC);
            if ly == 0 || (ly == 1 && lx >= 2 && lx < w - 2) {
                c = palette::shade(c, 1.07 + 0.06 * tone);
            } else if ly == 2 && lx >= 4 && lx < w - 1 {
                c = palette::shade(c, 0.93);
            }
        }
    }
    c
}

/// The pre-B89 water pixel: slow swell bands in tile space (ice-free water
/// without a bake; only ice-less callers outside the tile painter use it).
#[inline]
fn water_px_flat(base: Color, px: i32, py: i32, u: f32, v: f32) -> Color {
    let s = (u + v) * 5.0 + ((u - v) * 4.0).sin() * 0.5;
    let frac = s - s.floor();
    let c = if frac < 0.35 {
        palette::shade(base, 1.05)
    } else if frac > 0.85 {
        palette::shade(base, 0.96)
    } else {
        base
    };
    // A few static glints; the animation adds the moving ones.
    if tex::hash2(px, py, 0xa1) > 0.985 {
        palette::mix(c, pal::RAPIDS, 0.45)
    } else {
        c
    }
}

/// Base colour of a tile top triangle, before texture.
fn top_colour_raw(
    map: &TileMap,
    x: i32,
    y: i32,
    t: &Tile,
    tri: (usize, usize, usize),
    water: &WaterLook,
    g: &Pair,
) -> Color {
    // Two scales of noise: 3x3-tile patches plus per-tile grain, so a flat
    // meadow still has texture.
    let jitter = (map.patch_noise(x, y) - 0.5) * 0.16 + (map.tile_hash(x, y) - 0.5) * 0.05;
    match t.kind {
        Kind::Water => water_colour(map.water_kind(x, y), water, map, x, y, map.tile_hash(x, y)),
        // B83: dark asphalt, and only a little per-tile tone (a strong per-tile
        // jitter drew the tile grid).
        Kind::Road | Kind::Paved => palette::jitter(CARRIAGEWAY, map.tile_hash(x, y), 0.025),
        Kind::Sidewalk => palette::jitter(pal::SIDEWALK[0], map.tile_hash(x, y), 0.05),
        Kind::Dirt => palette::jitter(pal::DIRT[3], map.tile_hash(x, y), 0.08),
        Kind::Gravel => palette::jitter(pal::GRAVEL, map.tile_hash(x, y), 0.07),
        Kind::Field => palette::jitter(pal::DIRT[5], map.tile_hash(x, y), 0.05),
        Kind::Bridge => pal::BRIDGE_DECK,
        kind => {
            let light = triangle_light(t.corner, tri);
            let level = t.max_level();
            // D66: the land's own ramp, mixed with the neighbour's at the edge.
            g.ground(map, x, y, kind, shade_index(light, level, jitter))
        }
    }
}

/// Which per-pixel edge treatments a tile needs (B58b): computed once per tile,
/// so a meadow in the middle of a field pays nothing for the shoreline and scree
/// passes. The first cut ran both on every pixel of the frame and tripled the
/// static rebuild.
#[derive(Clone, Copy, Default)]
struct Edges {
    /// This tile meets water: a shore band (either side of the boundary).
    shore: bool,
    /// This tile meets a different natural ground (B70: any two, not only hard
    /// against soft).
    material: bool,
    /// The tile's colour jitter, worked out once for the edge bands (it was
    /// recomputed, with a 2-octave noise, for every pixel of an edge tile).
    jitter: f32,
    /// For the ground border: the other natural ground across each edge
    /// (-x, +x, -y, +y), if it differs from this tile's.
    other: [Option<Kind>; 4],
    /// B89: a water tile's depth bands (None for land and for ice). B92: a soft
    /// shore tile carries the bands of the water beside it, for the part of it
    /// the smooth coast turns back into water.
    wb: Option<WaterBake>,
    /// B92: the smooth coast field, for water tiles and soft low land tiles
    /// within a tile of the other.
    coast: Option<Coast>,
}

impl Edges {
    fn of(map: &TileMap, x: i32, y: i32, t: &Tile) -> Edges {
        let water = t.is_water();
        let mut e = Edges::default();
        if let Some((c, wl)) = coast_of(map, x, y) {
            if water || (soft_low(t, wl) && !c.bridge) {
                e.shore = true;
                e.coast = Some(c);
            }
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let n = map.at(x + dx, y + dy);
            if n.is_water() || n.kind == t.kind {
                continue;
            }
            // B70: any two different natural grounds dither at their border.
            if blends(t.kind) && blends(n.kind) {
                e.material = true;
                let slot = match (dx, dy) {
                    (-1, 0) => 0,
                    (1, 0) => 1,
                    (0, -1) => 2,
                    _ => 3,
                };
                e.other[slot] = Some(n.kind);
            }
        }
        if e.shore || e.material {
            e.jitter = (map.patch_noise(x, y) - 0.5) * 0.16 + (map.tile_hash(x, y) - 0.5) * 0.05;
        }
        e
    }
}

/// Distance, in tile units, from a tile pixel to the nearest edge whose
/// neighbour satisfies `wanted_water` — the raw material for an organic shore
/// (B58b item 2). `None` when no edge of this tile faces such a neighbour.
fn edge_dist(
    map: &TileMap,
    x: i32,
    y: i32,
    u: f32,
    v: f32,
    wanted_water: bool,
) -> Option<f32> {
    let mut best = f32::MAX;
    for (dx, dy, d) in [
        (-1, 0, u),
        (1, 0, 1.0 - u),
        (0, -1, v),
        (0, 1, 1.0 - v),
    ] {
        if map.at(x + dx, y + dy).is_water() == wanted_water {
            best = best.min(d);
        }
    }
    if best == f32::MAX {
        None
    } else {
        Some(best)
    }
}

/// A shared, global warp field for every tile boundary (shore sand, scree
/// aprons). It is a function of the screen pixel, so the two tiles either side
/// of a boundary sample the *same* value and their edges line up exactly
/// instead of each stepping along its own diamond.
#[inline]
fn warp_noise(px: i32, py: i32, salt: u32) -> f32 {
    // Value noise at two scales, bilinearly smoothed. `fbm` here cost a
    // third of the static rebuild: this runs per *pixel* along every shore.
    // Two octaves: a broad wander over several tiles (what stops a shore
    // following the tile grid) and a fine ripple along it.
    let sample = |sx: f32, sy: f32| {
        let (xi, yi) = (sx.floor() as i32, sy.floor() as i32);
        let (fx, fy) = (sx - xi as f32, sy - yi as f32);
        let s = |i: i32, j: i32| tex::hash2(i, j, salt);
        let top = s(xi, yi) * (1.0 - fx) + s(xi + 1, yi) * fx;
        let bot = s(xi, yi + 1) * (1.0 - fx) + s(xi + 1, yi + 1) * fx;
        top * (1.0 - fy) + bot * fy
    };
    let broad = sample(px as f32 / 34.0, py as f32 / 22.0);
    let fine = sample(px as f32 / 9.0, py as f32 / 6.0);
    (broad * 0.65 + fine * 0.35).clamp(0.0, 1.0)
}

/// The shore band of a water tile, or of the land tile beside it (B58b item 2).
///
/// A real shore is a continuous sand/pebble strip that wanders across the tile
/// boundary; drawn per tile it came out as two staircases with a hard edge
/// between them. Both sides now measure the distance to the *shared* boundary
/// and warp it with [`warp_noise`], so the band straddles the edge and both
/// tiles agree on where it is. `None` means "not shore: draw the tile itself".
#[allow(clippy::too_many_arguments)]
fn shore_px(
    map: &TileMap,
    x: i32,
    y: i32,
    t: &Tile,
    u: f32,
    v: f32,
    px: i32,
    py: i32,
    sand: Color,
    bank: Color,
    reed: Color,
    wb: Option<&WaterBake>,
    coast: &Coast,
) -> Option<Color> {
    let water = t.is_water();
    // B92: one signed distance to the smooth coast, shared by the water tile
    // and the land tile either side of it (positive on the land side). The
    // pixel's own tile no longer decides which side it is on: the coast cuts
    // across tiles, so a land tile can hold water and a water tile a beach.
    let mut s = coast_s(coast, u, v, px, py);
    if water && coast.hard {
        // Built or steep land (or a bridge) beside this water: the coast keeps
        // the tile's straight edge, no beach is laid into the water.
        let d = edge_dist(map, x, y, u, v, false)?;
        s = -(d + (warp_noise(px, py, 0x5EED) - 0.5) * 0.30);
        if s >= 0.0 {
            return None;
        }
    }
    if s < 0.0 {
        let Some(wb) = wb else {
            return None;
        };
        let rw = -s;
        // Thin water over the sand, a 2-3 px foam line with a soft, broken
        // outer fringe (the lapping surf is drawn over it by the overlay).
        let foam_c = palette::mix(wb.band[0], palette::rgb(0xf6, 0xfc, 0xff), 0.46 + 0.42 * wb.swell);
        let wob = (vnoise(px, py, 15.0, 9.0, 0xF0A1) - 0.5) * 0.05;
        let inner = 0.035 + wob * 0.5;
        let edge = 0.115 + wob;
        let h = tex::hash2(px / 2, py / 2, 0xF0A2);
        if rw < inner {
            return Some(palette::mix(sand, wb.band[0], 0.62));
        }
        if rw < edge {
            let f = (rw - inner) / (edge - inner);
            return Some(if h < 0.97 - 0.30 * f { foam_c } else { wb.band[0] });
        }
        if rw < edge + 0.09 {
            let f = (rw - edge) / 0.09;
            if h < 0.50 * (1.0 - f) * (1.0 - f) {
                return Some(palette::mix(wb.band[0], foam_c, 0.65));
            }
        }
        return if water { None } else { Some(water_px(wb, px, py, u, v)) };
    }
    // The land side: wet sand right at the water, dry sand behind it.
    let r = s;
    let (wet, dry) = (0.060, 0.135);
    if r < wet {
        // A darker, wet strip where the two meet, so the edge still reads as an
        // edge without being a drawn line (B99d: it takes the water's colour for
        // most of its own and breaks up, so no step is a ruled edge).
        let h = tex::hash2(px / 2, py / 3, 0x5EEF);
        if h < 0.18 && !water {
            return None;
        }
        return Some(palette::mix(bank, sand, 0.34 + 0.10 * h));
    }
    if r < dry {
        let f = (r - wet) / (dry - wet);
        return Some(palette::mix(palette::mix(sand, bank, 0.30), sand, f));
    }
    // The outer edge: dithered, with tufts of reed/grass where the ground is
    // soft, so nothing ends in a ruled line.
    let outer = dry + 0.09;
    if r < outer {
        let f = (r - dry) / (outer - dry);
        let h = tex::hash2(px / 2, py / 3, 0x5EEE);
        // Reeds stand in the inner half of the fringe, on still water only.
        if h > 0.62 && f < 0.55 && !matches!(map.water_kind(x, y), map::WaterKind::Sea) {
            return Some(palette::mix(sand, reed, 0.50));
        }
        return if h < f * 0.85 && !water { None } else { Some(sand) };
    }
    if water {
        Some(sand)
    } else {
        None
    }
}

/// The natural ground kinds whose borders dither into each other (B70).
#[inline]
fn blends(k: Kind) -> bool {
    matches!(
        k,
        Kind::Grass
            | Kind::Meadow
            | Kind::Upland
            | Kind::DryGrass
            | Kind::Sand
            | Kind::Scree
            | Kind::Rock
            | Kind::Gravel
    )
}

/// A soft material edge (B58b item 6, widened by B70): two different grounds
/// (scree against sand, meadow against dry grass, a rock hillside against
/// anything) never meet in a stair-stepped tile boundary: both sides dither into
/// each other over a band a third of a tile each way, warped by the same field
/// the shores use, so a patch of one ground runs into the next in a ragged drift
/// instead of being cut out along the tile diamonds. `colour` gives the ramp
/// colour of another kind at this pixel's shade.
#[allow(clippy::too_many_arguments)]
fn material_edge_px(
    nb: &[Option<Kind>; 4],
    u: f32,
    v: f32,
    px: i32,
    py: i32,
    colour: &dyn Fn(Kind) -> Color,
) -> Option<Color> {
    // The nearest edge whose neighbour is another ground.
    let mut best = f32::MAX;
    let mut other = Kind::Grass;
    for (k, d) in nb.iter().zip([u, 1.0 - u, v, 1.0 - v]) {
        if let Some(k) = *k {
            if d < best {
                best = d;
                other = k;
            }
        }
    }
    if best > 0.55 {
        return None;
    }
    // One octave of value noise (a ground border runs for tiles; the 2-px dither
    // supplies the fine ragged edge), a quarter the cost of `warp_noise`.
    let w = (vnoise(px, py, 26.0, 16.0, 0x5C12) - 0.5) * 0.60;
    let r = best + w;
    if r > 0.40 {
        return None;
    }
    // The other ground wins more and more towards the boundary, in a dither.
    let f = (r / 0.40).clamp(0.0, 1.0);
    let h = tex::hash2(px / 2, py / 2, 0x5C13);
    if h < 0.85 - f * 0.80 {
        Some(colour(other))
    } else {
        None
    }
}

/// Texture for one tile-top pixel: `base` is the shaded material colour, `u, v`
/// the position inside the tile (0..1), `px, py` the screen pixel.
#[allow(clippy::too_many_arguments)]
fn top_texture(
    map: &TileMap,
    x: i32,
    y: i32,
    t: &Tile,
    base: Color,
    px: i32,
    py: i32,
    u: f32,
    v: f32,
    field: Option<&crate::farmland::FieldTile>,
    look: &Ground,
    light: f32,
    edges: Edges,
) -> Color {
    let tex = look.tex;
    let pair = Pair::of(map);
    // D66/B99d: the *land* blends at the edge, the season does not. A
    // neighbour's tongue inside a snow world is under the same snow — without
    // this a tundra with a sand neighbour carried a bare, desert-coloured patch
    // in the middle of the snow.
    let neighbour_snow =
        matches!(pair.host.tex, GroundTex::Snow) && !matches!(tex, GroundTex::Snow);
    let snow_c = palette::rgb(0xF2, 0xF7, 0xFB);

    // --- B58b item 2: one organic shoreline, shared by both tiles ---------
    if edges.shore && edges.coast.is_some() {
        let jitter = edges.jitter;
        let idx = shade_index(light, t.max_level(), jitter).min(6);
        let mut sand = pair.ground(map, x, y, Kind::Sand, idx.min(5));
        if let Some(ice) = look.ice {
            // A frozen land's beach is snow-covered gravel, not dune sand.
            sand = palette::mix(sand, ice[0], 0.60);
        } else if matches!(tex, GroundTex::Snow) || neighbour_snow {
            sand = palette::mix(sand, snow_c, 0.55);
        }
        if let Some(c) = shore_px(
            map,
            x,
            y,
            t,
            u,
            v,
            px,
            py,
            sand,
            pair.bank(map, x, y),
            look.bush[0],
            edges.wb.as_ref(),
            edges.coast.as_ref().unwrap(),
        ) {
            return grain(c, px, py, 0x5EE0);
        }
    }
    // --- B58b item 6: scree and gravel end in a dithered apron ------------
    if edges.material {
        let jitter = edges.jitter;
        let idx = shade_index(light, t.max_level(), jitter);
        let snow = matches!(tex, GroundTex::Snow);
        let colour = |k: Kind| {
            let c = pair.ground(map, x, y, k, idx.min(5));
            // A snow world's borders are snow on both sides of the edge (see
            // `neighbour_snow`): the apron never shows bare sand.
            if (snow || neighbour_snow)
                && matches!(
                    k,
                    Kind::Grass | Kind::Meadow | Kind::Upland | Kind::DryGrass | Kind::Sand
                )
            {
                palette::mix(snow_c, c, 0.30)
            } else {
                c
            }
        };
        if let Some(c) = material_edge_px(&edges.other, u, v, px, py, &colour) {
            return grain(c, px, py, 0x5C14);
        }
    }

    // A wall: bedding planes, banded by how far the tile steps.
    if t.relief() >= 3 && !t.kind.is_rocky() {
        // Bands on the *screen row*, not on the tile coordinate: on a steep
        // quad v runs down the slope, which drew vertical stripes where the
        // review asked for horizontal bedding planes.
        let pitch = (9.0 - t.relief() as f32 * 0.8).clamp(3.0, 9.0);
        let s = py as f32 / pitch + (u * 4.0).sin() * 0.18;
        let band = s.floor();
        let frac = s - band;
        let seam = if t.kind == Kind::Sand { palette::shade(base, 0.82) } else { look.earth[1] };
        let mut c = palette::shade(base, 1.0 + ((band as i32) & 1) as f32 * 0.06);
        if frac < 0.18 {
            c = palette::mix(c, seam, 0.45);
        }
        return grain(c, px, py, 0x8c);
    }
    // A snow world's neighbour blend is snow-covered whatever its material is
    // (see `neighbour_snow`): only the ground's warmth and its species show
    // through. Paved and wet surfaces keep their own art.
    if neighbour_snow
        && !matches!(t.kind, Kind::Water | Kind::Road | Kind::Paved | Kind::Sidewalk)
    {
        return snow_px(palette::mix(snow_c, base, 0.12), px, py);
    }
    match t.kind {
        // D66: the land's ground texture, not the material's name — a desert
        // "dry grass" tile is sand with a little scrub on it, and a tundra
        // meadow is snow.
        // B46e: an alpine land's high ground is snow whatever its grass is.
        Kind::Upland if look.biome == map::Biome::Alpine => snow_px(base, px, py),
        Kind::Grass | Kind::Meadow | Kind::Upland | Kind::DryGrass if field.is_some() => {
            // B80: pasture, orchard and vineyard ground.
            let f = field.unwrap();
            let g = match tex {
                GroundTex::Snow => snow_px(base, px, py),
                GroundTex::Lush => lush_px(base, px, py),
                GroundTex::Sand => sand_px(base, px, py, u, v),
                _ => grass_px(base, px, py),
            };
            crate::farmland::field_px(f, g, px, py, x as f32 + u, y as f32 + v)
        }
        Kind::Grass | Kind::Meadow | Kind::Upland | Kind::DryGrass => match tex {
            GroundTex::Snow => snow_px(base, px, py),
            GroundTex::Lush => lush_px(base, px, py),
            GroundTex::Sand => sand_px(base, px, py, u, v),
            _ => grass_px(base, px, py),
        },
        Kind::Rock | Kind::Scree => {
            let c = rock_px(base, px, py, u, v, look.earth[1]);
            // D66/B46b: a snow land's bare rock keeps snow in its hollows, so a
            // crag reads as rock in snow and not as a grey plate.
            if let Some(ice) = look.ice {
                // Snow in the hollows, at *patch* scale: a per-2-px speckle read
                // as television static on a whole mountainside (B46c).
                // Two scales: a broad drift and a ragged edge, so the snow lies
                // in banks rather than in a dither.
                let broad = map::rng::fbm(px as f32 / 16.0, py as f32 / 10.0, 0x3F, 2);
                let edge = tex::hash2(px / 4, py / 4, 0x3E);
                if broad > 0.56 {
                    let t = ((broad - 0.56) / 0.44).min(1.0);
                    let jitter = (edge - 0.5) * 0.14;
                    palette::mix(c, ice[0], (0.10 + 0.36 * t + jitter).clamp(0.0, 0.8))
                } else {
                    c
                }
            } else {
                c
            }
        }
        Kind::Sand => sand_px(base, px, py, u, v),
        Kind::Dirt => soil_px(base, px, py),
        Kind::Gravel => gravel_px(base, px, py),
        Kind::Sidewalk => paving_px(base, px, py, u, v),
        Kind::Road | Kind::Paved => {
            // Tyre tracks only on a through lane, never across a junction.
            let along_x = map.at(x - 1, y).kind.is_lane() && map.at(x + 1, y).kind.is_lane();
            let along_y = map.at(x, y - 1).kind.is_lane() && map.at(x, y + 1).kind.is_lane();
            let across = if along_y && !along_x { u } else { v };
            asphalt_px(base, px, py, across, along_x != along_y)
        }
        Kind::Field => match field {
            Some(f) => crate::farmland::field_px(f, base, px, py, x as f32 + u, y as f32 + v),
            None => base,
        },
        Kind::Water if edges.wb.is_some() => water_px(edges.wb.as_ref().unwrap(), px, py, u, v),
        Kind::Water => {
            let mut c = water_px_flat(base, px, py, u, v);
            // B58b item 2: the shallows are a smooth band along the warped
            // shore — water lit from the bed — instead of a per-tile tone that
            // stepped at the tile grid.
            if edges.shore {
                let d = edge_dist(map, x, y, u, v, false).unwrap_or(1.0);
                let w = (warp_noise(px, py, 0x5EED) - 0.5) * 0.42;
                let r = d + w;
                if r < 0.60 {
                    let f = 1.0 - (r / 0.60).clamp(0.0, 1.0);
                    let k = if look.biome == map::Biome::Tropical { 0.34 } else { 0.20 };
                    let shallow = pair.at(map, x, y).shallow;
                    c = palette::mix(c, shallow, k * f * f);
                }
            }
            c
        }
        // Bridge deck: timber planks across it, grain along them.
        _ => {
            let plank = (u * 7.0).floor();
            let frac = (u * 7.0).fract();
            let c = palette::jitter(base, tex::hash2(plank as i32, 0, 0xb1), 0.10);
            if frac < 0.12 {
                palette::shade(c, 0.80)
            } else {
                grain(c, px, py, 0xb2)
            }
        }
    }
}

/// How water is lit and animated (B21 items 3-4).
#[derive(Clone, Copy, Debug)]
pub struct WaterLook {
    /// Sky colour at the horizon by day, moonlight/lamps at night: water picks
    /// it up instead of staying a fixed cyan sea (D44, review of H20).
    pub tint: Color,
    /// Animation phase in "old frames" (6 per second, wrapping every
    /// `WATER_PHASE`), advanced continuously (D71): a fractional phase means
    /// the ripples scroll smoothly at the display rate instead of stepping
    /// through six frames a second.
    pub phase: f32,
    /// Off = no animation at all (`--water-anim off`).
    pub anim: bool,
    /// How lit the scene is (0..1): ripples and foam fade out at night instead
    /// of glowing white.
    pub brightness: f32,
    /// B64: how far the water body is into its night look, 0..=16 (quantised so
    /// the baked tile colour only re-rasterises a few times per dusk).
    pub night_q: u8,
    /// B89: overcast, 0..1 (greys the sea and calms its glitter). Baked.
    pub cloud: f32,
    /// B89: how rough the sea is, 0 = glass .. 1 = storm (wind and thunder).
    pub chop: f32,
    /// B89: rain on the water, 0..1 (dimples).
    pub rain: f32,
    /// B89: screen column of the sun (or the moon at night) and how strongly it
    /// glitters on the water, 0..1 (quantised by the caller so a slow sun does
    /// not restart every tile's strokes).
    pub glint_x: f32,
    pub glint: f32,
    /// Colour of that glitter: warm white by day, orange at the horizon, pale
    /// blue for the moon.
    pub glint_c: Color,
    /// B89: how close the sun is to the horizon, 0..1 (1 at sunrise/sunset).
    /// The warm evening light multiplies a cyan sea down to slate grey, so at
    /// dusk the sea starts from a deeper, more violet blue.
    pub dusk: f32,
}

impl WaterLook {
    /// 0..=16 night steps for a light level: 0 by day, 16 in full night.
    pub fn night_steps(brightness: f32) -> u8 {
        (((0.75 - brightness) / 0.55).clamp(0.0, 1.0) * 16.0).round() as u8
    }
}

impl Default for WaterLook {
    fn default() -> Self {
        Self {
            tint: pal::WATER[1],
            phase: 0.0,
            anim: false,
            brightness: 1.0,
            night_q: 0,
            cloud: 0.0,
            chop: 0.35,
            rain: 0.0,
            glint_x: 0.0,
            glint: 0.0,
            glint_c: palette::rgb(0xff, 0xf6, 0xdc),
            dusk: 0.0,
        }
    }
}

/// The season as the *renderer* needs it (B28). `render` does not depend on
/// `world`, so the caller (present/spike) fills this from `world::SeasonLook`.
#[derive(Clone, Copy, Debug)]
pub struct SeasonLook {
    /// How far into the autumn colour change the leaves are (0..1). Spring has
    /// bare branches without any of this (B42).
    pub autumn: f32,
    /// 0 = fully in leaf, 1 = bare branches.
    pub bareness: f32,
    /// Blossom on the deciduous trees, 0..1 (spring).
    pub blossom: f32,
    /// Fresh new leaf, 0..1: spring's pale first green (B76).
    pub fresh: f32,
    /// Fallen leaves under the trees, 0..1.
    pub litter: f32,
    /// Grass gone dry and brown, 0..1.
    pub grass_dry: f32,
    /// A field's stage: ploughed → shoots → crop → stubble.
    pub field_stage: f32,
    /// The green a deciduous canopy has, and the colour it turns to.
    pub green: [[u8; 3]; 3],
    pub turn: [[u8; 3]; 3],
}

impl Default for SeasonLook {
    fn default() -> Self {
        Self {
            autumn: 0.0,
            bareness: 0.0,
            blossom: 0.0,
            fresh: 0.0,
            litter: 0.0,
            grass_dry: 0.0,
            field_stage: 0.5,
            green: [[0x5a, 0xb4, 0x46], [0x3f, 0x94, 0x3c], [0x27, 0x69, 0x30]],
            turn: [[0xd8, 0xb0, 0x3a], [0xc8, 0x6e, 0x2c], [0x9a, 0x3a, 0x2c]],
        }
    }
}

impl SeasonLook {
    /// The turn for one tree, from its own hash: a wood turns a tree at a time.
    pub fn turn_for(&self, seed: f32) -> f32 {
        (self.autumn + (seed - 0.5) * 0.30).clamp(0.0, 1.0)
    }

    /// How bare the branches are, jittered per tree so a wood does not drop its
    /// leaves on one day.
    pub fn bare_for(&self, seed: f32) -> f32 {
        (self.bareness + (seed - 0.5) * 0.20).clamp(0.0, 1.0)
    }

    pub fn buckets(&self) -> (i32, i32, i32, i32, i32) {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 24.0).round() as i32;
        (
            q(self.bareness),
            q(self.blossom),
            q(self.grass_dry),
            q(self.field_stage),
            q(self.fresh),
        )
    }
}

/// B76 r2: which street tiles a plough or gritter has cleared, and how far.
///
/// A plough leaves a dark lane behind it and pushes the snow to the kerb; the
/// mask is a level per tile (0 = untouched, 1 = one pass, 2 = cleared down to
/// the asphalt), so the road visibly darkens as the service works along it.
#[derive(Clone, Debug, Default)]
pub struct Cleared(pub std::collections::HashMap<(i32, i32), u8>);

impl Cleared {
    /// 0..1 clearing at a tile.
    pub fn at(&self, x: i32, y: i32) -> f32 {
        (self.0.get(&(x, y)).copied().unwrap_or(0) as f32 / 2.0).min(1.0)
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The weather as the *renderer* needs it (B27): one small struct that both the
/// sky and the ground read, so a rainy frame is consistent everywhere.
#[derive(Clone, Copy, Debug)]
pub struct WeatherLook {
    /// Cloud cover, 0..1: the sky's cloud count and the light's dimming.
    pub cloud: f32,
    /// Rain intensity 0..1 (streaks, splashes).
    pub rain: f32,
    /// Snow intensity 0..1 (flakes).
    pub snow: f32,
    /// Fog / mist 0..1 (washes the bottom of the frame).
    pub fog: f32,
    /// Thunder 0..1 (lightning flashes).
    pub storm: f32,
    /// Wind, km/h and degrees (rain slant, cloud drift).
    pub wind_kmh: f32,
    pub wind_dir: f32,
    /// Roads are wet 0..1: darker asphalt with a few sky reflections.
    pub wet: f32,
    /// Snow lying on the ground and the roofs, 0..1.
    pub snow_cover: f32,
    /// A dust storm (D66/D68), 0..1: the ambient light goes warm and dim, which
    /// is what a wall of sand does to a frame. It costs nothing per frame —
    /// unlike a full-frame veil, which measured 23 ms at 7040x1440.
    pub dust: f32,
    /// B76: how long it has been dry, 0..1 (0 = raining recently, 1 = a long
    /// dry spell). Parks and grass go patchy yellow when this is high.
    pub drought: f32,
    /// B76: frost on the grass, 0..1 (a cold, clear dawn).
    pub frost: f32,
}

impl WeatherLook {
    /// The hour's ambient tint with this weather folded in (B27/D66): overcast
    /// and fog dim the light, lying snow lifts it, and a sandstorm's dust veil
    /// warms and dims it. This is *where* the veil lives — a per-frame
    /// full-frame wash measured 23 ms at 7040x1440, a light change is free.
    pub fn tint(&self, tint: [f32; 3]) -> [f32; 3] {
        // B76: lying snow is a reflector. A snowy overcast noon was 34 % darker
        // than a clear one, which made a white world read as a grey fog bank.
        let dim = (1.0 - 0.34 * self.cloud.clamp(0.0, 1.0) - 0.14 * self.fog.clamp(0.0, 1.0)
            + 0.46 * self.snow_cover.clamp(0.0, 1.0))
            .clamp(0.45, 1.15);
        let d = self.dust.clamp(0.0, 1.0);
        // Sand scatters the blue out of the light: red survives, blue does not.
        let warm = [1.0 - 0.18 * d, 1.0 - 0.26 * d, 1.0 - 0.44 * d];
        // A storm takes another stop off the light and cools it (B57b item 2:
        // "the land dimmer and cooler"), and it takes the blue *up*: a wet sky
        // between you and the sun is a filter, not a shade.
        let s = self.storm.clamp(0.0, 1.0);
        let cool = [1.0 - 0.06 * s, 1.0 - 0.02 * s, 1.0 + 0.05 * s];
        let dim = dim * (1.0 - 0.09 * d) * (1.0 - 0.17 * s);
        [
            tint[0] * dim * warm[0] * cool[0],
            tint[1] * dim * warm[1] * cool[1],
            tint[2] * dim * warm[2] * cool[2],
        ]
    }
}

impl Default for WeatherLook {
    fn default() -> Self {
        Self {
            cloud: 0.30,
            rain: 0.0,
            snow: 0.0,
            fog: 0.0,
            storm: 0.0,
            wind_kmh: 8.0,
            wind_dir: 250.0,
            wet: 0.0,
            snow_cover: 0.0,
            dust: 0.0,
            drought: 0.0,
            frost: 0.0,
        }
    }
}

impl WeatherLook {
    /// Clear weather costs nothing: no overlay pass, no extra dirty work.
    pub fn is_clear(&self) -> bool {
        self.rain < 0.02 && self.snow < 0.02 && self.fog < 0.02 && self.storm < 0.02
    }

    /// A coarse bucket per visual knob, so the wallpaper only re-rasterises the
    /// ground when the look actually changes (not on every 1 % wobble).
    pub fn buckets(&self) -> (i32, i32, i32, i32) {
        // B76: lying snow, a dry spell and frost are baked into the static
        // ground, so they are quantised coarsely — four steps of snow over its
        // six-hour build-up is one rebuild an hour and a half, which is the
        // brief's ceiling. Wet roads are *not* in the bucket: they never
        // re-rasterised before B76 either, and the tint carries the mood.
        let q4 = |v: f32| (v.clamp(0.0, 1.0) * 4.0).round() as i32;
        let q = |v: f32| (v.clamp(0.0, 1.0) * 10.0).round() as i32;
        (q4(self.snow_cover), q(self.cloud), q4(self.drought), q4(self.frost))
    }
}

/// One water animation cycle, in "old frames" (B21 set six of them at 6 fps,
/// so one cycle was one second). D71 keeps that speed and splits it finely:
/// the phase is fractional, so a 30 fps frame advances it by 0.2 and the
/// ripples glide instead of stepping.
pub const WATER_PHASE: f32 = 120.0;

/// Phase advance per second (one cycle per second, as before).
pub const WATER_PHASE_HZ: f32 = 6.0;

/// The season on the ground: winter grass straws off, autumn leaves lie under
/// the trees (B28). Water, lanes and buildings are untouched.
#[inline]
fn seasonise(c: Color, t: &Tile, s: &SeasonLook, style: crate::biome::SeasonStyle) -> Color {
    let mut out = c;
    if s.grass_dry > 0.01
        && matches!(
            t.kind,
            map::Kind::Grass | map::Kind::Meadow | map::Kind::Upland | map::Kind::DryGrass
        )
    {
        // B76 r2: winter grass is *not* dead everywhere. Only the countryside
        // fields go straw (they are a crop — see `field_colour`); a temperate
        // lawn or park goes a muted, darker green, a boreal one browns harder,
        // and an arid or tropical lawn barely changes at all. Round 1 dried the
        // whole town to straw, which read as a desert.
        use crate::biome::SeasonStyle as St;
        let (straw, desat, target) = match style {
            St::Temperate => (0.30, 0.16, palette::rgb(0x7e, 0x8a, 0x52)),
            St::Arid => (0.12, 0.04, palette::rgb(0xa8, 0x9c, 0x6a)),
            St::Boreal => (0.55, 0.28, palette::rgb(0x92, 0x84, 0x58)),
            St::Tropical => (0.10, 0.05, palette::rgb(0x74, 0x92, 0x4a)),
        };
        let dry = s.grass_dry.clamp(0.0, 1.0);
        // A verge (the strip beside a road) dulls a little more than a lawn.
        let verge = matches!(t.kind, map::Kind::DryGrass);
        let k = if verge { 1.35 } else { 1.0 };
        out = palette::mix(out, target, (straw * k * dry).clamp(0.0, 0.9));
        let l = (0.30 * palette::r(out) as f32
            + 0.59 * palette::g(out) as f32
            + 0.11 * palette::b(out) as f32) as u8;
        out = palette::mix(out, palette::rgb(l, l, l), (desat * dry).clamp(0.0, 0.5));
    }
    out
}

/// B76 r2: fallen leaves gather in *windrows* along the kerb, not in speckle.
///
/// The wind and the traffic push them into the gutter, so a street tree's
/// leaves end up as a continuous drift a few pixels deep along the kerb of a
/// lane, deeper in a corner where two kerbs meet, thinning as the season runs
/// out. On a lawn they lie in drifts under the crowns instead of a uniform
/// scatter. All of it is baked into the static layer: no per-frame cost.
fn draw_litter(
    fb: &mut Framebuffer,
    map: &TileMap,
    x: i32,
    y: i32,
    p: &[(f32, f32); 4],
    season: &SeasonLook,
) {
    let t = map.at(x, y);
    let soft = matches!(
        t.kind,
        map::Kind::Sidewalk | map::Kind::Paved | map::Kind::Grass | map::Kind::Meadow | map::Kind::Dirt
    );
    if !soft {
        return;
    }
    let litter = season.litter.clamp(0.0, 1.0);
    if litter < 0.08 {
        return;
    }
    let lane = |dx: i32, dy: i32| {
        map.get(x + dx, y + dy)
            .is_some_and(|n| n.kind.is_lane())
    };
    let (posx, posy) = (lane(1, 0), lane(0, 1));
    // A windrow needs a kerb: this tile must touch a lane (or be a pavement
    // beside one).
    if !posx && !posy && !lane(-1, 0) && !lane(0, -1) {
        // On open lawn: a drift under the nearest crowns, in clumps.
        if !matches!(t.kind, map::Kind::Grass | map::Kind::Meadow) || litter < 0.35 {
            return;
        }
        // Leaves lie where they fell: under a crown. A lawn in the open keeps
        // none, which is also what stops a whole field reading as speckle.
        let near_tree = [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]
            .iter()
            .any(|(dx, dy)| map.at(x + dx, y + dy).prop.is_tree());
        if !near_tree {
            return;
        }
        let h = map.tile_hash(x, y);
        // A drift is a few clumps of leaves, not one leaf per pixel.
        let clumps = 3;
        let per = (((litter * 10.0) / clumps as f32).round() as i32).max(1);
        for k in 0..clumps {
            let a = (h * 53.0 + k as f32 * 2.399).fract();
            let b = (h * 91.0 + k as f32 * 1.618).fract();
            // Bilinear *inside the diamond*: `p[0]` is the tile's top corner,
            // `p[1]` its right and `p[3]` its left, so `u` runs down-right and
            // `v` down-left. (B76 r3: this used `p[2].x - p[0].x` for both axes,
            // and the top and bottom corners of a diamond share an x — so every
            // mark in the tile landed on one vertical line, and a whole autumn
            // forest read as columns of orange dashes.)
            // Bilinear *inside the diamond*: `p[0]` is the tile's top corner,
            // `p[1]` its right and `p[3]` its left, so `u` runs down-right and
            // `v` down-left. (B76 r3: this used `p[2].x - p[0].x` for both axes,
            // and a tile diamond's top and bottom corners share an x — so every
            // mark in the tile landed on one vertical line, and a whole autumn
            // forest read as columns of orange dashes.)
            // One clump per strip of the diamond, so a tile's drift spreads
            // across it instead of huddling in one corner.
            let u = (k as f32 + 0.2 + 0.6 * ((a * 7.0).fract())) / clumps as f32;
            let v = 0.12 + 0.76 * ((b * 7.0).fract());
            let cx = p[0].0 + (p[1].0 - p[0].0) * u + (p[3].0 - p[0].0) * v;
            let cy = p[0].1 + (p[1].1 - p[0].1) * u + (p[3].1 - p[0].1) * v;
            for i in 0..per {
                let ang = a * 21.0 + i as f32 * 2.1;
                let dx = (ang.cos() * 4.0).round() as i32;
                let dy = (ang.sin() * 2.0).round() as i32;
                let c = season.turn[((k + i) as usize) % 3];
                fb.put(cx as i32 + dx, cy as i32 + dy, palette::rgb(c[0], c[1], c[2]));
            }
        }
        return;
    }
    // The kerb edges this tile owns, in screen space (the two faces the camera
    // sees), and how deep the drift is (a corner gathers deeper).
    let corner = (posx && posy) || (posx && lane(0, -1)) || (posy && lane(-1, 0));
    let depth = if corner { 5.0 } else { 3.0 } * (0.5 + 0.5 * litter);
    let mut edges: [(f32, f32, f32, f32); 2] = [(0.0, 0.0, 0.0, 0.0); 2];
    let mut n_edges = 0;
    if posx {
        edges[n_edges] = (p[1].0, p[1].1, p[2].0, p[2].1);
        n_edges += 1;
    }
    if posy {
        edges[n_edges] = (p[3].0, p[3].1, p[2].0, p[2].1);
        n_edges += 1;
    }
    // Also draw the gutter of a lane tile's *neighbour's* kerb on the far side.
    if lane(-1, 0) && n_edges < 2 {
        edges[n_edges] = (p[0].0, p[0].1, p[3].0, p[3].1);
        n_edges += 1;
    }
    if lane(0, -1) && n_edges < 2 {
        edges[n_edges] = (p[0].0, p[0].1, p[1].0, p[1].1);
        n_edges += 1;
    }
    let salt = ((x as u32) << 8) ^ (y as u32) ^ 0x1eaf;
    for (ax, ay, bx, by) in edges[..n_edges].iter().copied() {
        let dx = bx - ax;
        let dy = by - ay;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        // Inward normal (the face looks at the camera; its normal points into
        // the tile).
        let (nx, ny) = (-dy / len, -dx / len).into();
        let (nx, ny): (f32, f32) = if (nx, ny) == (0.0, 0.0) { (0.0, 1.0) } else { (nx, ny) };
        let px_per = len as i32;
        for k in 0..px_per {
            let f = k as f32 / px_per as f32;
            let px = ax + dx * f;
            let py = ay + dy * f;
            // The windrow: dense at the kerb, thinning into the pavement. A
            // gap wherever the drift has thinned (`litter`).
            let across = ((litter * depth).round() as i32).max(1);
            for d in 0..across {
                let h = tex::hash2(px as i32, (py as i32) + d * 2, salt);
                if h > 0.25 + 0.7 * litter {
                    continue;
                }
                let c = season.turn[((h * 7.0) as usize) % 3];
                let shade = 0.85 + 0.3 * h;
                fb.put(
                    (px + nx * d as f32) as i32,
                    (py + ny * d as f32) as i32,
                    palette::shade(palette::rgb(c[0], c[1], c[2]), shade),
                );
            }
            // The outermost line is continuous at full litter: that is what
            // makes it a windrow and not a sprinkle.
            if litter > 0.45 {
                let c = season.turn[(k as usize) % 3];
                fb.put(px as i32, py as i32, palette::rgb(c[0], c[1], c[2]));
            }
        }
    }
}

/// B76: the weather's own marks on soft ground — a long dry spell bleaches the
/// grass in patches, and a cold clear dawn rimes it white.
#[inline]
fn weather_ground(c: Color, t: &Tile, wx: &WeatherLook, x: i32, y: i32, map: &TileMap) -> Color {
    let soft = matches!(
        t.kind,
        map::Kind::Grass | map::Kind::Meadow | map::Kind::Upland | map::Kind::DryGrass
    );
    if !soft {
        return c;
    }
    let mut out = c;
    if wx.drought > 0.05 {
        // Patchy, not a uniform tan: a dry lawn dies in patches round its edges.
        let n = map.patch_noise(x, y);
        let patch = ((n - 0.34) * 2.6).clamp(0.0, 1.0);
        out = palette::mix(out, palette::rgb(0xbe, 0xae, 0x62), 0.55 * wx.drought * patch);
    }
    if wx.frost > 0.05 {
        out = palette::mix(out, palette::rgb(0xe4, 0xee, 0xf4), 0.62 * wx.frost);
    }
    out
}

/// Rain darkens the asphalt, snow whitens what it can lie on. One function so
/// the rule is the same for every tile (B27 item 2).
#[inline]
fn weatherise(c: Color, t: &Tile, wx: &WeatherLook) -> Color {
    let mut out = c;
    if wx.wet > 0.01 && t.kind.is_lane() {
        // Wet asphalt: darker, and a touch cooler.
        out = palette::mix(out, palette::rgb(0x38, 0x3c, 0x44), 0.55 * wx.wet);
    }
    if wx.snow_cover > 0.01 {
        if t.kind.is_water() || t.kind == map::Kind::Bridge {
            return out;
        }
        let lay = if t.kind.is_lane() {
            // Ploughed-or-driven roads keep less of it.
            0.35 * wx.snow_cover
        } else if t.kind.is_rocky() {
            0.55 * wx.snow_cover
        } else {
            0.88 * wx.snow_cover
        };
        out = palette::mix(out, palette::rgb(0xf2, 0xf7, 0xfb), lay.min(0.94));
    }
    out
}

/// Water colour for one tile: the class base mixed towards the sky/moon.
#[inline]
fn water_colour(
    kind: map::WaterKind,
    look: &WaterLook,
    map: &TileMap,
    x: i32,
    y: i32,
    h: f32,
) -> Color {
    let pair = Pair::of(map);
    let g = pair.at(map, x, y);
    // B46b: a still lake in a land that freezes is ice, not open water.
    if let Some(ice) = g.ice {
        if matches!(kind, map::WaterKind::Lake | map::WaterKind::Pond) {
            let scuff = (h * 7.0).fract();
            return palette::mix(ice[0], ice[1], 0.10 + 0.18 * scuff);
        }
    }
    // B58b item 2: depth is one smooth field across the whole body of water. It
    // used to be picked per tile from a hash, which made a lake a checkerboard
    // of three blues at exactly the tile grid — the giveaway that the water was
    // tiles at all.
    let t = map.patch_noise(x, y).clamp(0.0, 1.0);
    let base = match kind {
        map::WaterKind::Sea => {
            let deep = pair.deep(map, x, y);
            // B46e: inside a lagoon's reef the sea is a pale turquoise shallow.
            match map.ocean.as_ref() {
                Some(o) if o.reef > 0.0 => {
                    let d = o.side as f32 * ((x - y) as f32 - o.shore_p);
                    let k = ((o.reef + 1.5 - d) / 3.0).clamp(0.0, 1.0);
                    palette::mix(deep, pair.host.shallow, 0.78 * k)
                }
                _ => deep,
            }
        }
        map::WaterKind::Lagoon => {
            // Shallow, and over land the flood took: the land's own shallows
            // with the still water, paler than the open sea beside it.
            let a = pair.host.shallow;
            let b = pair.water(map, x, y, 2);
            palette::mix(a, b, 0.35 + 0.30 * t)
        }
        _ => {
            let a = pair.water(map, x, y, 0);
            let b = pair.water(map, x, y, 2);
            palette::mix(a, b, t * 0.75)
        }
    };
    // The shallows are per *pixel* in `top_texture` (a smooth turquoise band
    // along the warped shore); the flat tile colour stays the open-water one.
    let _ = (x, y);
    let k = match kind {
        map::WaterKind::Sea => 0.34,
        map::WaterKind::Lagoon => 0.30,
        map::WaterKind::Lake => 0.26,
        map::WaterKind::Pond => 0.24,
        _ => 0.20,
    };
    let _ = (x, y);
    let base = if look.dusk > 0.0 && matches!(kind, map::WaterKind::Sea | map::WaterKind::Lagoon) {
        palette::mix(base, palette::rgb(0x3a, 0x5c, 0xf0), 0.72 * look.dusk * (1.0 - look.cloud))
    } else {
        base
    };
    let day = palette::mix(base, look.tint, k);
    // B64: at night the water is dark and carries the sky's colour, not the
    // saturated day blue (the ambient light alone left it glowing cyan).
    // B99d: but it stays *water*: at 0.58 of the day's brightness a night sea
    // was a near-black void with a few white specks. The sea keeps a fifth more
    // light and takes more of the sky's own colour, so it reads as water under
    // a night sky (the moon's path on it is the water overlay's business).
    let n = look.night_q as f32 / 16.0;
    if n <= 0.0 {
        return day;
    }
    let sky = palette::mix(day, look.tint, 0.66 * n);
    // B99c round 2: a shallow lagoon under a storm sky still carries light.
    if kind == map::WaterKind::Lagoon {
        let sky = palette::mix(day, look.tint, 0.42 * n);
        return palette::shade(sky, 1.0 - 0.40 * n);
    }
    // The night sea keeps a moonlit blue so it is never a black plate.
    palette::mix(palette::shade(sky, 1.0 - 0.28 * n), palette::rgb(0x34, 0x58, 0xa0), 0.42 * n)
}

/// What the water overlay paints with (B87): a framebuffer, or a recorder that
/// keeps the strokes instead, so a tick can tell which tiles' ripples moved.
pub trait WaterInk {
    fn line(&mut self, a: (f32, f32), b: (f32, f32), c: Color);
    /// A line of alpha-blended pixels (the glints fade over the water).
    fn line_cov(&mut self, a: (f32, f32), b: (f32, f32), c: Color, cov: f32);
    fn put_blended(&mut self, x: i32, y: i32, c: Color, cov: f32);
}

impl WaterInk for Framebuffer {
    #[inline]
    fn line(&mut self, a: (f32, f32), b: (f32, f32), c: Color) {
        Framebuffer::line(self, a, b, c);
    }
    #[inline]
    fn line_cov(&mut self, a: (f32, f32), b: (f32, f32), c: Color, cov: f32) {
        if cov >= 1.0 {
            Framebuffer::line(self, a, b, c);
        } else {
            Framebuffer::line_blended(self, a, b, c, cov);
        }
    }
    #[inline]
    fn put_blended(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        Framebuffer::put_blended(self, x, y, c, cov);
    }
}

/// One stroke of a tile's ripples as the framebuffer would draw it: the
/// rounded end points of a line (a blended dot has both ends the same), its
/// colour and its coverage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub c: Color,
    pub cov: f32,
    /// A blended dot (`put_blended`) rather than a line.
    pub dot: bool,
}

impl Stroke {
    /// The pixels the stroke can touch (a Bresenham line stays inside its end points).
    /// Paint it, exactly as `water_overlay` would have.
    #[inline]
    pub fn paint(&self, fb: &mut Framebuffer) {
        if self.dot {
            fb.put_blended(self.x0, self.y0, self.c, self.cov);
        } else if self.cov < 1.0 {
            fb.line_blended(
                (self.x0 as f32, self.y0 as f32),
                (self.x1 as f32, self.y1 as f32),
                self.c,
                self.cov,
            );
        } else {
            fb.line(
                (self.x0 as f32, self.y0 as f32),
                (self.x1 as f32, self.y1 as f32),
                self.c,
            );
        }
    }

    pub fn bounds(&self) -> crate::framebuffer::Rect {
        crate::framebuffer::Rect::new(
            self.x0.min(self.x1),
            self.y0.min(self.y1),
            self.x0.max(self.x1) + 1,
            self.y0.max(self.y1) + 1,
        )
    }
}

/// The most strokes one tile draws (B89: crests, glitter, surf, a dimple).
const MAX_STROKES: usize = 14;

/// A tile's strokes.
#[derive(Clone, Copy, Debug, Default)]
pub struct TilePrims {
    n: u8,
    s: [Option<Stroke>; MAX_STROKES],
}

/// B101: two tiles' strokes are the same when their used strokes are; the
/// derived comparison also walked every empty slot of the 14.
impl PartialEq for TilePrims {
    fn eq(&self, o: &Self) -> bool {
        self.n == o.n && self.s[..self.n as usize] == o.s[..o.n as usize]
    }
}

impl TilePrims {
    /// Forget the strokes (the slots are overwritten as new ones arrive).
    #[inline]
    pub fn clear(&mut self) {
        self.n = 0;
    }

    fn push(&mut self, st: Stroke) {
        if (self.n as usize) < MAX_STROKES {
            self.s[self.n as usize] = Some(st);
            self.n += 1;
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &Stroke> {
        self.s[..self.n as usize].iter().flatten()
    }
    pub fn contains(&self, p: &Stroke) -> bool {
        self.iter().any(|q| q == p)
    }
}

impl WaterInk for TilePrims {
    fn line(&mut self, a: (f32, f32), b: (f32, f32), c: Color) {
        self.push(Stroke {
            x0: a.0.round() as i32,
            y0: a.1.round() as i32,
            x1: b.0.round() as i32,
            y1: b.1.round() as i32,
            c,
            cov: 1.0,
            dot: false,
        });
    }
    fn line_cov(&mut self, a: (f32, f32), b: (f32, f32), c: Color, cov: f32) {
        self.push(Stroke {
            x0: a.0.round() as i32,
            y0: a.1.round() as i32,
            x1: b.0.round() as i32,
            y1: b.1.round() as i32,
            c,
            cov,
            dot: false,
        });
    }
    fn put_blended(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        self.push(Stroke { x0: x, y0: y, x1: x, y1: y, c, cov, dot: true });
    }
}

/// A hashed 0..1 value for one tile, slot and salt.
#[inline]
fn th(x: i32, y: i32, slot: u32, salt: u32) -> f32 {
    tex::hash2(x.wrapping_mul(7).wrapping_add(slot as i32 * 1013), y.wrapping_mul(5).wrapping_add(slot as i32 * 17), salt)
}

/// Where a repeating life of `period` old frames stands at phase `f`, shifted
/// by `off` (0..1): the position in it (0..1) and the cycle number, wrapped
/// to the loop so a stroke re-rolled each cycle still closes the loop
/// (every period divides [`WATER_PHASE`]).
#[inline]
fn life(f: f32, period: f32, off: f32) -> (f32, i32) {
    let t = f / period + off;
    let n = t.floor();
    let cycles = ((WATER_PHASE / period).round() as i32).max(1);
    (t - n, (n as i32).rem_euclid(cycles))
}

/// A glint's brightness over `phi` of its life, in three steps (a stroke that
/// fades smoothly would change every tick and cost a repaint each time).
/// 0 outside the `on` part of the life.
#[inline]
fn glint_level(phi: f32, on: f32) -> f32 {
    if phi >= on {
        return 0.0;
    }
    let w = phi / on;
    let tri = 1.0 - (2.0 * w - 1.0).abs();
    (tri * 3.0).ceil() / 3.0
}

/// The strokes of open water (B89): long irregular crests that fade in and
/// out where they stand, the sun's (or moon's) glitter, a surf line lapping at
/// every coast, and rain dimples. Each stroke has its own period and its own
/// place per cycle, so nothing repeats on a grid.
#[allow(clippy::too_many_arguments)]
fn open_water<F: WaterInk>(
    fb: &mut F,
    map: &TileMap,
    look: &WaterLook,
    kind: map::WaterKind,
    x: i32,
    y: i32,
    p: &[Pt; 4],
    hi: Color,
    k: f32,
    f: f32,
    mid: (f32, f32),
    rapids: Color,
    coast: Option<Coast>,
) {
    let s = S as f32;
    // A point of the tile diamond at (u, v).
    let pt = |u: f32, v: f32| -> (f32, f32) {
        (
            p[0].0 * (1.0 - u) * (1.0 - v) + p[1].0 * u * (1.0 - v) + p[2].0 * u * v + p[3].0 * (1.0 - u) * v,
            p[0].1 * (1.0 - u) * (1.0 - v) + p[1].1 * u * (1.0 - v) + p[2].1 * u * v + p[3].1 * (1.0 - u) * v,
        )
    };
    let chop = look.chop.clamp(0.0, 1.0);
    // How lively this kind of water is.
    let c = match kind {
        map::WaterKind::Sea => 0.40 + 0.60 * chop,
        map::WaterKind::Lagoon => 0.30 + 0.30 * chop,
        map::WaterKind::Lake => 0.16 + 0.34 * chop,
        _ => 0.10 + 0.20 * chop,
    };
    let veil = 1.0 - 0.40 * look.cloud;
    let amp = (0.50 + 0.50 * k) * veil;
    // Glitter: how much of the sun's (or moon's) path this tile is in.
    let dxs = (mid.0 - look.glint_x).abs();
    let sigma = 150.0 + (mid.1 - 260.0).max(0.0) * 0.32;
    let u0 = (1.0 - (dxs / (2.4 * sigma)).powi(2)).max(0.0);
    let path = look.glint * u0 * u0;
    // B92: at night the lit quay and the lamps along it lie in the harbour
    // water as short broken amber streaks.
    if k < 0.40 && matches!(kind, map::WaterKind::Sea | map::WaterKind::Lagoon) {
        for (side, (dx, dy)) in [(-1, 0), (1, 0), (0, -1), (0, 1)].into_iter().enumerate() {
            let n = map.at(x + dx, y + dy);
            if !matches!(n.kind, Kind::Road | Kind::Paved | Kind::Sidewalk | Kind::Bridge) {
                continue;
            }
            let sl = side as u32;
            for j in 0..2u32 {
                if th(x, y, sl * 2 + j, 0xF1) > 0.80 {
                    continue;
                }
                let period = [10.0, 12.0, 15.0][(th(x, y, sl * 2 + j, 0xF2) * 3.0) as usize % 3];
                let (phi, _) = life(f, period, th(x, y, sl * 2 + j, 0xF3));
                let q = glint_level(phi, 0.8);
                if q <= 0.0 {
                    continue;
                }
                let a = 0.2 + 0.6 * th(x, y, sl * 2 + j, 0xF4);
                let o = 0.10 + 0.18 * th(x, y, sl * 2 + j, 0xF5);
                let (u, v) = match side {
                    0 => (o, a),
                    1 => (1.0 - o, a),
                    2 => (a, o),
                    _ => (a, 1.0 - o),
                };
                let (cx, cy) = pt(u, v);
                let w = 2.0 + 3.0 * th(x, y, sl * 2 + j, 0xF6);
                let amber = palette::rgb(0xff, 0xc8, 0x70);
                for r in 0..3 {
                    let ry = cy + r as f32 * 2.0;
                    let ww = (w - r as f32 * 0.7).max(1.0);
                    fb.line_cov((cx - ww, ry), (cx + ww, ry), amber, (0.42 - 0.11 * r as f32) * q);
                }
            }
        }
    }
    // Deep night: only the moon's path glitters; the rest of the sea is still.
    if k < 0.2 && path < 0.05 {
        return;
    }
    // --- crests ---------------------------------------------------------
    const PERIODS: [f32; 6] = [12.0, 15.0, 20.0, 24.0, 30.0, 40.0];
    for slot in 0..3u32 {
        if th(x, y, slot, 0xA0) > c * (0.62 - 0.14 * slot as f32) + path * 0.9 {
            continue;
        }
        let period = PERIODS[(th(x, y, slot, 0xA1) * 6.0) as usize % 6];
        let (phi, n) = life(f, period, th(x, y, slot, 0xA2));
        let q = glint_level(phi, 0.60);
        if q <= 0.0 {
            continue;
        }
        let hh = |salt: u32| tex::hash2(x.wrapping_mul(13).wrapping_add(slot as i32 * 101), y.wrapping_mul(7).wrapping_add(n * 29), salt);
        let (ua, va) = (0.14 + 0.72 * hh(0xB1), 0.14 + 0.72 * hh(0xB2));
        // Mostly flat dashes like the baked wave marks, some along the tile axes.
        let dk = hh(0xB3);
        let (dx, dy) = if dk < 0.15 {
            (1.0, 0.5)
        } else if dk < 0.30 {
            (1.0, -0.5)
        } else {
            (1.0, 0.0)
        };
        let mut len = (1.8 + hh(0xB4) * (2.6 + 3.4 * c)) * s;
        let (mut col, mut cov) = (hi, (0.82 * q * amp * (1.0 + 0.6 * path)).min(1.0));
        // Whitecaps in a gale: long and white.
        if (hh(0xB5) < (chop - 0.68).max(0.0) * 2.6 + 0.012 + (chop - 0.30).max(0.0) * 0.25) && matches!(kind, map::WaterKind::Sea) {
            col = palette::mix(rapids, hi, 0.25);
            cov = 0.92 * q;
            len *= 1.5;
        } else if path > 0.05 {
            col = palette::mix(col, look.glint_c, (path * 1.4).min(0.8));
        }
        let (cx, cy) = pt(ua, va);
        let drift = phi * 1.5 * s;
        let (cx, cy) = (cx + dx * drift, cy + dy * drift * 0.5);
        fb.line_cov(
            (cx - dx * len * 0.5, cy - dy * len * 0.5),
            (cx + dx * len * 0.5, cy + dy * len * 0.5),
            col,
            cov,
        );
        // The trough under it: a shorter, darker line one pixel below.
        if cov < 0.9 {
            fb.line_cov(
                (cx - dx * len * 0.3, cy - dy * len * 0.3 + s),
                (cx + dx * len * 0.35, cy + dy * len * 0.35 + s),
                palette::rgb(0x0c, 0x3a, 0x6a),
                0.20 * q * veil,
            );
        }
    }
    // --- glitter: a shimmering streak under the sun, a few sparkles anywhere --
    // B92: not confetti. Inside the path the glints are long horizontal dashes
    // (longest on the path's axis, shorter at its edges), each with a faint
    // longer line under it, twinkling through three steps; outside it only the
    // odd short sparkle.
    let sparkle = path * 1.0 + 0.06 * k * veil * c;
    if sparkle > 0.01 {
        const GP: [f32; 5] = [4.0, 5.0, 6.0, 8.0, 10.0];
        let axis = u0 * u0;
        let slots = if path > 0.6 { 3u32 } else { 2 };
        for slot in 0..slots {
            if th(x, y, slot, 0xC0) > sparkle {
                continue;
            }
            let period = GP[(th(x, y, slot, 0xC1) * 5.0) as usize % 5];
            let (phi, n) = life(f, period, th(x, y, slot, 0xC2));
            let q = glint_level(phi, 0.55);
            if q <= 0.0 {
                continue;
            }
            let hh = |salt: u32| tex::hash2(x.wrapping_mul(17).wrapping_add(slot as i32 * 59), y.wrapping_mul(11).wrapping_add(n * 31), salt);
            let (cx, cy) = pt(0.10 + 0.80 * hh(0xC3), 0.10 + 0.80 * hh(0xC4));
            let col = if path > 0.05 { look.glint_c } else { hi };
            let long = (3.0 + 15.0 * path * (0.3 + 0.7 * axis) * (0.4 + 0.9 * hh(0xC5))).round();
            let w = (s * 1.5 + long - 1.0).max(2.0);
            let (x0, x1) = (cx - w * 0.5, cx + w * 0.5);
            let vis = veil.max(0.5);
            fb.line_cov((x0, cy), (x1, cy), col, (0.60 + 0.40 * q) * vis);
            if path > 0.3 && q > 0.6 {
                fb.line_cov((x0 + w * 0.25, cy - 1.0), (x1 - w * 0.25, cy - 1.0), col, 0.55 * q * vis);
            }
        }
    }
    // --- surf: a foam line crawling in along the smooth coast -------------
    if k > 0.2 && matches!(kind, map::WaterKind::Sea | map::WaterKind::Lagoon) {
        if let Some(coast) = coast {
            if !coast.hard {
                let sc = |u: f32, v: f32| coast_raw(&coast, u, v);
                for slot in 0..2u32 {
                    let period = [20.0, 24.0, 30.0][(th(x, y, slot, 0xD0) * 3.0) as usize % 3];
                    let (phi, cyc) = life(f, period, th(x, y, slot, 0xD1));
                    // In from the open water to the wash line, fading in then out.
                    let q = glint_level(phi, 0.85);
                    if q <= 0.0 {
                        continue;
                    }
                    let hh = |salt: u32| {
                        tex::hash2(
                            x.wrapping_mul(19).wrapping_add(slot as i32 * 71),
                            y.wrapping_mul(23).wrapping_add(cyc * 37),
                            salt,
                        )
                    };
                    let (mut u, mut v) = (0.1 + 0.8 * hh(0xD2), 0.1 + 0.8 * hh(0xD3));
                    // The wash line this far from the coast: a pass of Newton
                    // steps puts the point on the contour s = -o.
                    let o = 0.15 + 0.30 * (1.0 - (phi * 8.0).floor() / 8.0);
                    let e = 0.04;
                    for _ in 0..2 {
                        let s0 = sc(u, v);
                        let gu = (sc(u + e, v) - sc(u - e, v)) / (2.0 * e);
                        let gv = (sc(u, v + e) - sc(u, v - e)) / (2.0 * e);
                        let g2 = (gu * gu + gv * gv).max(0.05);
                        u -= (s0 + o) * gu / g2;
                        v -= (s0 + o) * gv / g2;
                    }
                    let (gu, gv) = {
                        let gu = sc(u + e, v) - sc(u - e, v);
                        let gv = sc(u, v + e) - sc(u, v - e);
                        let l = (gu * gu + gv * gv).sqrt().max(1e-4);
                        (gu / l, gv / l)
                    };
                    if !(0.02..0.98).contains(&u) || !(0.02..0.98).contains(&v) {
                        continue;
                    }
                    let half = 0.10 + 0.12 * hh(0xD4);
                    let (p0, p1) = (pt(u + gv * half, v - gu * half), pt(u - gv * half, v + gu * half));
                    let col = palette::mix(rapids, hi, 0.30);
                    fb.line_cov(p0, p1, col, 0.80 * q * (0.55 + 0.45 * k));
                }
            }
        }
    }
    // --- rain dimples -----------------------------------------------------
    if look.rain > 0.04 {
        let (phi, n) = life(f, 6.0, th(x, y, 0, 0xE0));
        if th(x, y, 0, 0xE1) < look.rain * 0.9 && phi < 0.5 {
            let (cx, cy) = pt(0.15 + 0.7 * tex::hash2(x, y.wrapping_add(n * 3), 0xE2), 0.15 + 0.7 * tex::hash2(x.wrapping_add(n * 5), y, 0xE3));
            let r = if phi < 0.25 { 1.0 } else { 2.0 } * s * 0.5;
            fb.line_cov((cx - r, cy), (cx + r, cy), hi, 0.55 * veil);
        }
    }
}

/// The flowing surface on top of the static water, drawn per visible water tile
/// (B21 item 3): rivers carry a scrolling highlight along the downhill
/// direction, lakes shimmer, the sea has wave lines rolling in, the beach gets
/// a breaking foam line.
pub fn water_overlay<F: WaterInk>(
    fb: &mut F,
    cam: &Camera,
    map: &TileMap,
    look: &WaterLook,
    x: i32,
    y: i32,
) {
    water_overlay_c(fb, cam, map, look, x, y, None, None)
}

/// The smooth coast around tile `(x, y)` as the surf wants it (none where the
/// coast keeps a straight edge). The scene keeps these per water tile, so a
/// tick does not blur the land mask of every tile again.
pub(crate) fn coast_for(map: &TileMap, x: i32, y: i32) -> Option<Coast> {
    coast_of(map, x, y).map(|c| c.0).filter(|c| !c.hard)
}

/// [`water_overlay`] with the tile's coast already worked out (`Some(None)` =
/// no coast near this tile).
pub(crate) fn water_overlay_c<F: WaterInk>(
    fb: &mut F,
    cam: &Camera,
    map: &TileMap,
    look: &WaterLook,
    x: i32,
    y: i32,
    pre: Option<Option<Coast>>,
    ground: Option<&'static Ground>,
) {
    if !look.anim {
        return;
    }
    let kind = map.water_kind(x, y);
    if kind == map::WaterKind::None {
        return;
    }
    let t = map.at(x, y);
    let level = t.corner[0] as f32;
    let p = cam.corners(x, y, t.corner.map(|c| c as f32));
    let f = look.phase.rem_euclid(WATER_PHASE);
    // Highlights fade with the light: at night only a faint moon glint is left.
    let k = look.brightness.clamp(0.0, 1.0);
    // D66: the land's own highlight (a desert rapid is bleached white, a polar
    // one is pale ice blue).
    let pair = Pair::of(map);
    // B101: the land that owns the tile never changes between ticks, and
    // finding it walks a noise field; the scene keeps it per water tile.
    let owner = match ground {
        Some(g) => g,
        None => pair.at(map, x, y),
    };
    let rapids = owner.rapids;
    let hi = palette::mix(look.tint, rapids, 0.22 + 0.45 * k);
    // B64: at night the ripple crests are a faint glint, not cyan streaks.
    let dim = 0.30 + 0.70 * ((k - 0.2) / 0.6).clamp(0.0, 1.0);
    let hi = palette::shade(hi, dim);
    let h = map.tile_hash(x, y);
    // The middle of the tile, in screen space.
    let (mx, my) = ((p[0].0 + p[2].0) * 0.5, (p[0].1 + p[2].1) * 0.5);
    // B46b: a frozen lake cracks instead of rippling.
    if let Some(ice) = owner.ice {
        if matches!(kind, map::WaterKind::Lake | map::WaterKind::Pond) {
            let hsh = map.tile_hash(x, y);
            if hsh > 0.45 {
                let a = (hsh * 9.0).fract() * std::f32::consts::TAU;
                let len = 7.0 + 9.0 * (1.0 - hsh);
                let cx = mx + (hsh - 0.5) * 14.0 * S as f32;
                let cy = my + ((hsh * 3.0).fract() - 0.5) * 6.0 * S as f32;
                fb.line(
                    (cx, cy),
                    (cx + a.cos() * len, cy + a.sin() * len * 0.5),
                    ice[1],
                );
                // A pale scuff either side of the crack, so it reads as ice.
                fb.put_blended(
                    (cx + a.cos() * len * 0.5) as i32,
                    (cy + a.sin() * len * 0.25 - 1.0) as i32,
                    palette::shade(ice[0], 1.05),
                    0.5,
                );
            }
            return;
        }
    }
    match kind {
        map::WaterKind::River => {
            // Downhill direction from the neighbouring water levels, in tile
            // space, then to the screen (2:1 iso).
            let mut best = (0.0f32, 0.0f32);
            let mut lowest = level;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = map.at(x + dx, y + dy);
                if n.is_water() && (n.corner[0] as f32) < lowest {
                    lowest = n.corner[0] as f32;
                    best = (dx as f32, dy as f32);
                }
            }
            if best == (0.0, 0.0) {
                best = (1.0, 0.0);
            }
            // B92: flow lines that follow the river's own direction: long,
            // thin streaks sliding downstream across the tile at their own
            // speeds and lanes, fading in and out at the ends, each with a
            // darker line under it.
            let pt = |u: f32, v: f32| -> (f32, f32) {
                (
                    p[0].0 * (1.0 - u) * (1.0 - v) + p[1].0 * u * (1.0 - v) + p[2].0 * u * v + p[3].0 * (1.0 - u) * v,
                    p[0].1 * (1.0 - u) * (1.0 - v) + p[1].1 * u * (1.0 - v) + p[2].1 * u * v + p[3].1 * (1.0 - u) * v,
                )
            };
            let (fx, fy) = best;
            let (qx, qy) = (-fy, fx);
            let bright = palette::mix(hi, rapids, 0.35);
            let dark = palette::shade(pair.water(map, x, y, 1), 0.70 * dim);
            let veil = 1.0 - 0.40 * look.cloud;
            for i in 0..4u32 {
                let period = [12.0, 15.0, 20.0, 24.0][(th(x, y, i, 0x61) * 4.0) as usize % 4];
                let (phi, _) = life(f, period, th(x, y, i, 0x62));
                let lane = (th(x, y, i, 0x63) - 0.5) * 0.62;
                let t = (phi - 0.5) * 1.5;
                let half = 0.10 + 0.10 * th(x, y, i, 0x64);
                let fade = (std::f32::consts::PI * phi).sin().max(0.0);
                let c = |a: f32| pt(0.5 + fx * a + qx * lane, 0.5 + fy * a + qy * lane);
                let (p0, p1) = (c(t - half), c(t + half));
                fb.line_cov((p0.0, p0.1 + 1.0), (p1.0, p1.1 + 1.0), dark, 0.30 * fade * veil);
                fb.line_cov(p0, p1, bright, 0.80 * fade * veil * (0.5 + 0.5 * k));
            }
            // A waterfall: white water down the face where the surface steps.
            if map.is_rapid(x, y) && (h * 5.0 + f) as i32 % 2 == 0 {
                fb.line(
                    (mx - 4.0 * S as f32, my - 2.0 * S as f32),
                    (mx + 4.0 * S as f32, my + 2.0 * S as f32),
                    rapids,
                );
            }
        }
        map::WaterKind::Sea
        | map::WaterKind::Lagoon
        | map::WaterKind::Lake
        | map::WaterKind::Pond => {
            let coast = match pre {
                Some(c) => c,
                None => coast_for(map, x, y),
            };
            open_water(fb, map, look, kind, x, y, &p, hi, k, f, (mx, my), rapids, coast)
        }
        _ => {}
    }
}


/// A 4×4 ordered Bayer threshold in `[0, 1)`. Two adjacent ramp steps resolved
/// through it read as a smooth gradient instead of a band.
#[inline]
fn bayer4(px: i32, py: i32) -> f32 {
    const B: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    let i = (py.rem_euclid(4) * 4 + px.rem_euclid(4)) as usize;
    (B[i] as f32 + 0.5) / 16.0
}

/// The look that owns the tile at `(x, y)` (the neighbour where its tongue of
/// land is strongest). Used where the ground's *cliff* ramp is needed.
#[inline]
fn look_for_tile(map: &TileMap, x: i32, y: i32) -> &'static Ground {
    let host = crate::biome::look(map.biome);
    match map.neighbour {
        Some(n) if map.biome_at(x, y) == n => crate::biome::look(n),
        _ => host,
    }
}

/// The two ramp steps a soft-ground triangle sits between, already through the
/// weather and the season, plus how far between them it is (`0` = the lower).
///
/// Water, lanes, paving and field soil are painted flat (their texture carries
/// the detail), so they come back as the same colour twice.
#[allow(clippy::too_many_arguments)]
fn top_shade(
    map: &TileMap,
    x: i32,
    y: i32,
    t: &Tile,
    tri: (usize, usize, usize),
    water: &WaterLook,
    g: &Pair,
    wx: &WeatherLook,
    season: &SeasonLook,
) -> (Color, Color, f32) {
    match t.kind {
        Kind::Water
        | Kind::Road
        | Kind::Paved
        | Kind::Sidewalk
        | Kind::Dirt
        | Kind::Gravel
        | Kind::Field
        | Kind::Bridge => {
            let c = top_colour_raw(map, x, y, t, tri, water, g);
            // Order matters (B76): the season and the weather's own marks on
            // the grass first, the snow last — a strawed winter lawn under snow
            // is white, not tan.
            let c = weatherise(
                weather_ground(seasonise(c, t, season, g.at(map, x, y).season), t, wx, x, y, map),
                t,
                wx,
            );
            (c, c, 0.0)
        }
        kind => {
            let light = triangle_light(t.corner, tri);
            let level = t.max_level();
            let jitter = (map.patch_noise(x, y) - 0.5) * 0.16 + (map.tile_hash(x, y) - 0.5) * 0.05;
            let tint = (level as f32 / map::MAX_LEVEL as f32 - 0.5) * 0.10;
            // The same index the flat path used, but kept fractional so the last
            // 40 % of a step is dithered rather than rounded away.
            let idx = 3.0 + (light + tint + jitter - 0.54) * 4.6;
            let i = idx.floor().clamp(0.0, 6.0) as usize;
            let frac = (idx - idx.floor()).clamp(0.0, 1.0);
            // A tile that steps several levels across itself has no horizontal
            // surface to speak of: its "top" is the wall of a cut, so it takes
            // the land's cliff ramp instead of the ground's (B46c). A stretched
            // sand texture is what read as a vertical smeared column.
            let (lo, hi) = if t.relief() >= 3 && !kind.is_rocky() && kind != Kind::Sand {
                let ramp = &look_for_tile(map, x, y).earth;
                (ramp[2], ramp[3])
            } else {
                (
                    g.ground(map, x, y, kind, i),
                    g.ground(map, x, y, kind, (i + 1).min(6)),
                )
            };
            (
                weatherise(
                    weather_ground(seasonise(lo, t, season, g.host.season), t, wx, x, y, map),
                    t,
                    wx,
                ),
                weatherise(
                    weather_ground(seasonise(hi, t, season, g.host.season), t, wx, x, y, map),
                    t,
                    wx,
                ),
                frac,
            )
        }
    }
}

/// The riser of a step *up* toward the viewer: the strip between a higher
/// neighbour's surface and ours. It is a cut in the ground, so a deep one is
/// drawn as a rock face with strata and a lit lip; a one-level bank keeps the
/// ground's own material. (It used to be one flat washed-ramp quad, which is
/// what put vertical smears down every hillside in the B46 review.)
#[allow(clippy::too_many_arguments)]
fn draw_riser(
    fb: &mut Framebuffer,
    a: Pt,
    b: Pt,
    c: Pt,
    d: Pt,
    step: i32,
    t: &Tile,
    x: i32,
    y: i32,
    look: &Ground,
    g: &Pair,
    map: &TileMap,
    dir: Dir,
) {
    let salt = ((x as u32) << 8) ^ (y as u32) ^ 0x2f;
    let (base, light) = if t.is_water() {
        (look.water_bank, look.rapids)
    } else if t.kind == Kind::Sand {
        let ramp = look.ramp(Kind::Sand);
        (ramp[2], ramp[4])
    } else if step >= 2 {
        let rock = t.kind.is_rocky();
        let ramp = if rock { &look.cliff_rock } else { &look.earth };
        (ramp[2], ramp[5])
    } else {
        let ramp = g.ground(map, x, y, t.kind, 1);
        (ramp, palette::shade(ramp, 1.22))
    };
    // The same two-faced rule the tile tops use: the +x face turns away from
    // the light, the +y face catches it. Without this every tile of a long cliff
    // shades itself and the wall reads as a patchwork.
    let face = if dir == Dir::PosX { 0.80 } else { 1.0 };
    let bands = 1.6 + step as f32 * 0.55;
    tex::quad_px(fb, a, b, c, d, |px, py, u, v| {
        let s = py as f32 / (bands * 2.2) + (u * 5.0).sin() * 0.16;
        let band = s.floor();
        let frac = s - band;
        let mut col = palette::shade(base, face * (1.0 + ((band as i32) & 1) as f32 * 0.07));
        if frac < 0.15 {
            col = palette::shade(col, 0.84);
        }
        // A dark line just under the rim, so the lip reads as an edge.
        if v < 0.06 {
            col = palette::shade(col, 0.72);
        } else if step >= 2 && v < 0.14 {
            col = palette::mix(col, light, 0.22);
        }
        grain(col, px, py, salt)
    });
    // The lit lip along the top of the cut and the shadow at its foot.
    fb.line(a, b, palette::shade(light, 0.98));
    if step >= 2 {
        fb.line(c, d, palette::shade(base, 0.62 * face));
    }
}

/// Draw one tile: faces, two shaded triangles, water/road detail, then props.
#[allow(clippy::too_many_arguments)]
pub fn draw_tile(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    land_y: i32,
    glow: f32,
    water: &WaterLook,
    wx: &WeatherLook,
    season: &SeasonLook,
    holiday: &crate::holiday::HolidayLook,
    cleared: &Cleared,
    g: &Pair,
    plots: &[u16],
    claims: &[u8],
    derived: &[Option<Prop>],
) {
    let t0 = map.at(x, y);
    // B80: a pasture, an orchard and a vineyard are grass with a fence and
    // things standing on it: the ordinary grass pipeline paints them.
    let mut t = t0;
    if t0.kind == Kind::Field && crate::farmland::grassy(map::crop_of(&t0)) {
        t.kind = Kind::Grass;
    }
    let t = t;
    // B76 r2: a cleared lane keeps much less of its snow (and its kerb keeps a
    // heap of what was pushed aside).
    let clear_here = cleared.at(x, y);
    let wx_local;
    let wx = if clear_here > 0.01 && t.kind.is_lane() {
        wx_local = WeatherLook {
            snow_cover: wx.snow_cover * (1.0 - 0.92 * clear_here),
            ..*wx
        };
        &wx_local
    } else {
        wx
    };
    // D66: the land that owns this tile (the neighbour, where its tongue of land
    // is strongest) for faces, rock and plants; `g` handles the blended ground.
    let look = g.at(map, x, y);
    // B42b: a lane, pavement or bridge deck is something the city *built*: it
    // keeps its colour into the distance instead of dissolving into the haze.
    // Bare ground (grass, meadow, field, sand, rock) takes the distance haze.
    fb.solid = if t.kind.is_work_surface() {
        crate::framebuffer::TAG_BUILT
    } else {
        0
    };
    // B70: open ground corners may sit up to half a level off their integer
    // height (smooth slopes); `soft` is zero everywhere else.
    let dz = map.soft_dz(x, y);
    let lv = |c: &[u8; 4], d: [i8; 4]| -> [f32; 4] {
        [0, 1, 2, 3].map(|i| c[i] as f32 + d[i] as f32 / crate::iso::HEIGHT_UNIT as f32)
    };
    let p = cam.corners(x, y, lv(&t.corner, dz));
    // --- the two visible faces (x = x+1 and y = y+1) ---------------------
    for dir in [Dir::PosX, Dir::PosY] {
        let (dx, dy) = dir.delta();
        let nb = map.at(x + dx, y + dy);
        let (mine, theirs) = match dir {
            Dir::PosX => ([1usize, 2usize], [0usize, 3usize]),
            _ => ([3usize, 2usize], [0usize, 1usize]),
        };
        // The neighbour's edge of this tile, at the neighbour's own level.
        let nl = lv(&nb.corner, map.soft_dz(x + dx, y + dy));
        let (a, b) = if dir == Dir::PosX {
            (
                cam.project((x + 1) as f32, y as f32, level_px(nl[theirs[0]])),
                cam.project((x + 1) as f32, (y + 1) as f32, level_px(nl[theirs[1]])),
            )
        } else {
            (
                cam.project(x as f32, (y + 1) as f32, level_px(nl[theirs[0]])),
                cam.project((x + 1) as f32, (y + 1) as f32, level_px(nl[theirs[1]])),
            )
        };
        if nb.corner[theirs[0]] >= t.corner[mine[0]] && nb.corner[theirs[1]] >= t.corner[mine[1]] {
            // The neighbour is higher. Its riser faces away from the camera, but
            // the strip between its surface and ours is on screen (a step *up*
            // toward the viewer): leaving it unpainted shows the sky through a
            // hillside in a 1-px diagonal band, so it is drawn as a shaded riser
            // in our own material — which is what such a step looks like.
            let taller = nb.corner[theirs[0]] > t.corner[mine[0]]
                || nb.corner[theirs[1]] > t.corner[mine[1]];
            if taller {
                // How far up the neighbour stands above us: a one-level step is
                // a cut bank, a deeper one is a cliff face and takes strata.
                let step = (nb.corner[theirs[0]].max(nb.corner[theirs[1]]) as i32)
                    - (t.corner[mine[0]].min(t.corner[mine[1]]) as i32);
                draw_riser(
                    fb,
                    a,
                    b,
                    p[mine[0]],
                    p[mine[1]],
                    step,
                    &t,
                    x,
                    y,
                    look,
                    g,
                    map,
                    dir,
                );
            }
            continue;
        }
        let drop = (t.corner[mine[0]].max(t.corner[mine[1]]) as i32)
            - (nb.corner[theirs[0]].min(nb.corner[theirs[1]]) as i32);
        let edge = if dir == Dir::PosX { 0 } else { 1 };
        let works = map::edge_works(t.aux, edge).max(map::edge_works(nb.aux, edge ^ 2));
        draw_face(fb, &p, mine, a, b, drop, &t, works, x, y, look, dir);
    }

    // --- the top: two triangles, each textured in tile space --------------
    let farm = (t0.kind == Kind::Field).then(|| {
        crate::farmland::FieldTile::of(map, x, y, look, season.field_stage, wx.snow_cover)
    });
    let field = farm.as_ref();
    let mut edges = Edges::of(map, x, y, &t);
    if t.is_water() {
        // B89: depth bands, from the tile's flat colour (weather and all).
        let (flat, _, _) = top_shade(map, x, y, &t, triangles(t.diagonal())[0], water, g, wx, season);
        edges.wb = water_bake(map, x, y, flat, water);
    } else if edges.shore {
        // B92: a soft shore tile paints the water the smooth coast turns it
        // back into with the bands of a water tile beside it.
        let mut found = None;
        'find: for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)] {
            let n = map.at(x + dx, y + dy);
            if n.is_water() {
                found = Some((x + dx, y + dy, n));
                break 'find;
            }
        }
        match found {
            Some((nx, ny, nt)) => {
                let (flat, _, _) =
                    top_shade(map, nx, ny, &nt, triangles(nt.diagonal())[0], water, g, wx, season);
                match water_bake(map, nx, ny, flat, water) {
                    Some(mut wb) => {
                        wb.coast = edges.coast;
                        wb.depth = [0.0; 4];
                        edges.wb = Some(wb);
                    }
                    None => {
                        edges.shore = false;
                        edges.coast = None;
                    }
                }
            }
            None => {
                edges.shore = false;
                edges.coast = None;
            }
        }
    }
    // B70: open ground takes its light from the smoothed surface, per pixel,
    // so no tile edge shows in the shading; the colour is a dithered pair of
    // ramp steps (the palette stays the land's own small ramp).
    let soft_lit = if soft_kind(t.kind) { map.soft_lit(x, y) } else { None };
    if let Some(lit) = soft_lit {
        // The ramp steps are worked out on first use: a tile touches two or three.
        let mut ramp: [Option<Color>; 7] = [None; 7];
        let mut step = |i: usize| -> Color {
            *ramp[i].get_or_insert_with(|| {
                weatherise(
                    weather_ground(
                        seasonise(g.ground(map, x, y, t.kind, i), &t, season, look.season),
                        &t, wx, x, y, map,
                    ),
                    &t, wx,
                )
            })
        };
        // What the edge bands (shore, ground borders) take as "light": the smooth
        // value without the tint and patch noise they add back themselves.
        let own = if edges.shore || edges.material {
            let tint = (t.max_level() as f32 / map::MAX_LEVEL as f32 - 0.5) * 0.10;
            let jit = (map.patch_noise(x, y) - 0.5) * 0.16 + (map.tile_hash(x, y) - 0.5) * 0.05;
            tint + jit
        } else {
            0.0
        };
        // l(u, v) = l0 + du*u + dv*v + duv*u*v; a tile whose corners agree (most of
        // an open plain) is one constant, so only the dither runs per pixel.
        let (l0, du, dv) = (lit[0], lit[1] - lit[0], lit[3] - lit[0]);
        let duv = lit[2] - lit[1] - lit[3] + lit[0];
        let flat = du.abs() + dv.abs() + duv.abs() < 0.012;
        let l_flat = (lit[0] + lit[1] + lit[2] + lit[3]) * 0.25;
        let idx_flat = 3.0 + (l_flat - 0.54) * 4.6;
        let i_flat = idx_flat.floor().clamp(0.0, 6.0) as usize;
        let frac_flat = (idx_flat - idx_flat.floor()).clamp(0.0, 1.0);
        let (lo_flat, hi_flat) = if flat {
            (step(i_flat), step((i_flat + 1).min(6)))
        } else {
            (Color::default(), Color::default())
        };
        for tri in triangles(t.diagonal()) {
            if flat {
                let light = l_flat - own;
                tex::diamond_tri_px(fb, &p, tri, |px, py, u, v| {
                    let base = if i_flat < 6 && bayer4(px, py) < frac_flat { hi_flat } else { lo_flat };
                    top_texture(map, x, y, &t, base, px, py, u, v, field, look, light, edges)
                });
                continue;
            }
            tex::diamond_tri_px(fb, &p, tri, |px, py, u, v| {
                let l = l0 + du * u + dv * v + duv * u * v;
                let idx = 3.0 + (l - 0.54) * 4.6;
                let i = idx.floor().clamp(0.0, 6.0) as usize;
                let frac = (idx - idx.floor()).clamp(0.0, 1.0);
                let base = if i < 6 && bayer4(px, py) < frac { step(i + 1) } else { step(i) };
                top_texture(map, x, y, &t, base, px, py, u, v, field, look, l - own, edges)
            });
        }
    } else {
    for tri in triangles(t.diagonal()) {
        // D66/B46b: soft ground is shaded with a *dithered* pair of ramp steps
        // instead of one quantised colour, so a hillside has a smooth gradient
        // rather than seven flat bands (the "flat-coloured top" in review).
        let (lo, hi, frac) = top_shade(map, x, y, &t, tri, water, g, wx, season);
        // B58b: the shore and scree-apron bands take their own ramp step from
        // this triangle's light, so a beach on a shaded slope stays shaded.
        let light = triangle_light(t.corner, tri);
        if frac <= 0.0 || lo == hi {
            tex::diamond_tri_px(fb, &p, tri, |px, py, u, v| {
                top_texture(map, x, y, &t, lo, px, py, u, v, field, look, light, edges)
            });
        } else {
            tex::diamond_tri_px(fb, &p, tri, |px, py, u, v| {
                let base = if bayer4(px, py) < frac { hi } else { lo };
                top_texture(map, x, y, &t, base, px, py, u, v, field, look, light, edges)
            });
        }
    }

    }

    // --- rock facets: cracks and facets so a peak reads as sculpted stone --
    if t.kind.is_rocky() {
        draw_rock_facets(map, x, y, &p, fb, look);
    }

    // --- desire paths (D45) -----------------------------------------------
    if !t.kind.is_worked() && !t.is_water() {
        draw_wear(map, x, y, &p, fb, look);
    }

    // --- details ----------------------------------------------------------
    match t.kind {
        Kind::Water => {
            if map.is_rapid(x, y) {
                draw_rapids(fb, map, x, y, &p, look);
            }
        }
        Kind::Road | Kind::Paved => {
            // B83: the street's own anatomy — kerb, pavement or verge, and one
            // paved area at a junction. No outline on the tile boundary: that
            // line drew the grid, not the street.
            draw_street_edges(fb, map, x, y, &p, g, wx, season, plots);
            // B83: a street that ends in open ground turns in a circle rather
            // than stopping dead. One that ends at a lot's front ends in a drive
            // (the kerb opens there, see `draw_street_edges`).
            if crate::street::lane_degree(map, x, y) == 1 && !plot_within(plots, map, x, y, 3) {
                draw_turnaround(fb, cam, map, x, y);
            }
            draw_manhole(fb, cam, map, x, y);
            draw_drain(fb, cam, map, x, y);
        }
        Kind::Dirt | Kind::Gravel => {
            // B83: an unpaved lane is two worn ruts with a grassy crown, not a
            // painted street.
            draw_track_ruts(fb, map, x, y, &p, look);
        }
        Kind::Sidewalk => draw_kerb_edge(fb, map, x, y, &p),
        _ => {}
    }
    if let Some(f) = &farm {
        crate::farmland::draw_edges(fb, map, x, y, &p, f, look, season.bareness, wx.snow_cover);
        crate::farmland::draw_stand(fb, map, x, y, &p, f, look);
    }

    // --- B76: autumn litter along the kerb and on the pavement --------------
    if season.litter > 0.10 {
        draw_litter(fb, map, x, y, &p, season);
    }

    // --- street lamp at night (B16 item 2) --------------------------------
    if glow > 0.45 && matches!(t.kind, Kind::Paved) {
        draw_lamp(fb, map, x, y, &p, glow);
    }

    // --- prop -------------------------------------------------------------
    // A prop stands up out of the ground: never dissolved by the horizon haze,
    // and it may rise past the horizon (a back-row canopy).
    fb.solid = crate::framebuffer::TAG_PROP;
    draw_prop(fb, cam, map, x, y, land_y, season, wx.snow_cover, look, claims, derived);
    // B36: a lit garland over the street trees on a decorated day.
    // Only street trees: a tree with a road or paved tile beside it (not every tree in the woods, B42).
    let street_tree = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| {
        map.get(x as i32 + dx, y as i32 + dy)
            .is_some_and(|n| matches!(n.kind, Kind::Road | Kind::Paved))
    });
    // the owner 2026-11-10: tree garlands read as scratches everywhere at 1x; off until redrawn.
    if false && holiday.is_decorated() && street_tree && matches!(t.prop, Prop::Tree { .. }) {
        let (cx, cy) = prop_anchor(cam, map, x, y);
        let h = map.tile_hash(x, y);
        let r = 6.0 + 4.0 * h;
        crate::holiday::garland_line(
            fb,
            (cx as f32 - r, cy as f32 - 14.0 - 8.0 * h),
            (cx as f32 + r, cy as f32 - 18.0 - 8.0 * h),
            holiday,
            (x as u32) << 8 ^ y as u32,
        );
    }
}

/// A vertical face between this tile's edge and the neighbour's, drawn in earth
/// or rock strata (never as darker grass).
#[allow(clippy::too_many_arguments)]
fn draw_face(
    fb: &mut Framebuffer,
    p: &[Pt; 4],
    mine: [usize; 2],
    a: Pt,
    b: Pt,
    drop: i32,
    t: &Tile,
    works: u8,
    x: i32,
    y: i32,
    look: &Ground,
    dir: Dir,
) {
    let top_a = p[mine[0]];
    let top_b = p[mine[1]];
    // A retaining wall is coursed stone; an embankment is graded grass.
    if works >= 2 {
        let base = look.wall_stone[2];
        tex::quad_px(fb, top_a, top_b, a, b, |px, py, u, v| {
            // Coursed stone: 5 courses with staggered joints.
            let course = (v * 5.0).floor();
            let cf = (v * 5.0).fract();
            let stagger = if course as i32 % 2 == 0 { 0.0 } else { 0.25 };
            let joint = ((u + stagger) * 4.0).fract();
            let mut c = palette::jitter(base, tex::hash2(course as i32, (u * 4.0) as i32, 0xc1), 0.10);
            if cf < 0.14 || joint < 0.06 {
                c = palette::shade(c, 0.80);
            }
            grain(c, px, py, 0xc2)
        });
        fb.line(top_a, top_b, look.wall_stone[0]);
        return;
    }
    if works == 1 {
        let base = palette::shade(look.grass[2], 0.95);
        tex::quad_px(fb, top_a, top_b, a, b, |px, py, _u, _v| grass_px(base, px, py));
        return;
    }
    let (base, light) = if t.is_water() {
        (look.water_deep, look.rapids)
    } else if t.kind == Kind::Sand {
        // B89: a beach that drops to the water is a sand bank, not a brown
        // earth cliff (the brown staircase along every island coast).
        let ramp = look.ramp(Kind::Sand);
        (ramp[2], ramp[4])
    } else if drop >= 2 {
        let rock = t.kind == Kind::Rock || t.kind == Kind::Scree;
        let ramp = if rock { &look.cliff_rock } else { &look.earth };
        (ramp[2], ramp[5])
    } else {
        // A one-level step: shade the material's own side.
        let ramp = look.ramp(t.kind);
        (ramp[1], ramp[3])
    };
    // A whole face is lit or shadowed by the way it turns (above, B46b): one
    // multiplier for the whole wall, not one per tile.
    let face = if dir == Dir::PosX { 0.80 } else { 1.0 };
    let base = palette::shade(base, face);
    // Strata: horizontal bands down the face, with a wobble and grain, so a
    // cut reads as layered ground rather than a flat slab. The bands scale with
    // the height of the cut — a 9-level butte wall wants a dozen courses, not
    // four (B46c).
    let bands = 2.0 + drop as f32 * 1.15;
    // The foot of a tall face is a talus: rubble that has come off the wall.
    let talus = if drop >= 4 { look.scree[3] } else { base };
    let salt = ((x as u32) << 8) ^ (y as u32);
    tex::quad_px(fb, top_a, top_b, a, b, |px, py, u, v| {
        let s = py as f32 / (bands * 2.2) + (u * 7.0).sin() * 0.18;
        let band = s.floor();
        let frac = s - band;
        let mut c = palette::shade(base, 1.0 + ((band as i32) & 1) as f32 * 0.06);
        if frac < 0.16 {
            c = palette::shade(c, 0.84);
        }
        // The last rows are the scree apron at the foot of the wall.
        if drop >= 4 && v > 0.86 {
            let t = ((v - 0.86) / 0.14).clamp(0.0, 1.0);
            c = palette::mix(c, talus, 0.35 + 0.45 * t);
        }
        let c = grain(c, px, py, salt);
        if drop >= 2 && v < 0.06 {
            palette::mix(c, light, 0.35)
        } else {
            c
        }
    });
    if drop >= 3 {
        // Bright lip where the face meets the surface.
        fb.line(top_a, top_b, light);
    }
}

/// Rock facets: short cracks so a peak reads as sculpted stone, plus a facet
/// edge along the tile diagonal.
fn draw_rock_facets(
    map: &TileMap,
    x: i32,
    y: i32,
    p: &[Pt; 4],
    fb: &mut Framebuffer,
    look: &Ground,
) {
    let h = map.tile_hash(x, y);
    // A crack: 5-11 px, at a pseudo-random angle inside the diamond. The tile
    // diagonals are vertical on screen, so a random direction beats following
    // them.
    if h > 0.30 {
        let cx = p[0].0 + (p[1].0 - p[0].0) * (0.25 + 0.5 * h);
        let cy = p[0].1 + (p[3].1 - p[0].1) * (0.30 + 0.4 * h);
        let ang = (h * 12.0).fract() * std::f32::consts::TAU;
        let len = (4.0 + 7.0 * (1.0 - h)) * S as f32 * 0.5;
        // B58b item 3: cracks stay inside their own tile.
        let (ex, ey) = (cx + ang.cos() * len, cy + ang.sin() * len * 0.5);
        if in_tile_quad(p, cx, cy) && in_tile_quad(p, ex, ey) {
            fb.line((cx, cy), (ex, ey), palette::shade(look.rock[0], 0.62));
            // A lighter highlight on the sunlit side of the crack.
            fb.line(
                (cx, cy - 1.0),
                (cx + ang.cos() * len * 0.8, cy + ang.sin() * len * 0.4 - 1.0),
                palette::shade(look.rock[5], 1.02),
            );
        }
    }
    // A second, shorter facet line so the surface is broken up.
    if h > 0.62 {
        let cx = p[0].0 + (p[1].0 - p[0].0) * (0.55 + 0.3 * (h * 3.0).fract());
        let cy = p[0].1 + (p[3].1 - p[0].1) * (0.55 + 0.3 * (h * 7.0).fract());
        fb.line(
            (cx - 3.0 * S as f32, cy + 1.0),
            (cx + 2.0 * S as f32, cy - 1.0),
            palette::shade(palette::shade(look.rock[0], 0.62), 1.35),
        );
    }
}

/// Darker line inside a water tile where it meets land.
fn draw_water_edges(
    fb: &mut Framebuffer,
    map: &TileMap,
    x: i32,
    y: i32,
    p: &[Pt; 4],
    look: &Ground,
) {
    let sides: [(usize, usize, i32, i32, f32); 4] = [
        (0, 1, 0, -1, 1.0),
        (1, 2, 1, 0, -1.0),
        (2, 3, 0, 1, -1.0),
        (3, 0, -1, 0, 1.0),
    ];
    for (a, b, dx, dy, inset) in sides {
        if map.at(x + dx, y + dy).is_water() {
            continue;
        }
        fb.line(
            (p[a].0, p[a].1 + inset),
            (p[b].0, p[b].1 + inset),
            look.water_bank,
        );
        // A pale shallow-water band just inside the bank.
        fb.line(
            (p[a].0, p[a].1 + inset * 2.0 * S as f32),
            (p[b].0, p[b].1 + inset * 2.0 * S as f32),
            palette::mix(look.water_bank, look.rapids, 0.22),
        );
    }
}

/// White water where the river steps down a level: a foam line along the edge
/// that drops, plus a few flecks below it.
fn draw_rapids(
    fb: &mut Framebuffer,
    map: &TileMap,
    x: i32,
    y: i32,
    p: &[Pt; 4],
    look: &Ground,
) {
    let h = map.tile_hash(x, y);
    // The edge facing the lower water carries the foam.
    for dir in [Dir::PosX, Dir::PosY, Dir::NegX, Dir::NegY] {
        let (dx, dy) = dir.delta();
        let n = map.at(x + dx, y + dy);
        if !n.is_water() || n.corner[0] >= map.at(x, y).corner[0] {
            continue;
        }
        let (a, b) = match dir {
            Dir::PosX => (p[1], p[2]),
            Dir::PosY => (p[3], p[2]),
            Dir::NegX => (p[0], p[3]),
            _ => (p[0], p[1]),
        };
        fb.line(a, b, look.rapids);
        fb.line((a.0, a.1 - 1.0), (b.0, b.1 - 1.0), look.rapids);
        for i in 1..5 {
            let f = i as f32 / 5.0;
            let px = a.0 + (b.0 - a.0) * f;
            let py = a.1 + (b.1 - a.1) * f + 2.0 + h * 4.0;
            fb.put(px as i32, py as i32, look.rapids);
        }
    }
}

/// Wear from villagers walking (D45): trampled grass, then a thin footpath
/// along the direction of travel, then a cart track. Drawn on top of the tile
/// so it reads as the ground itself wearing through.
fn draw_wear(map: &TileMap, x: i32, y: i32, p: &[Pt; 4], fb: &mut Framebuffer, look: &Ground) {
    let stage = map::WearStage::of(map.wear(x, y));
    if stage == map::WearStage::None {
        return;
    }
    // The tile's centre on screen, and the travel direction: the axis whose
    // neighbours are more worn (a path runs along the tiles people use).
    let cx = (p[0].0 + p[1].0 + p[2].0 + p[3].0) * 0.25;
    let cy = (p[0].1 + p[1].1 + p[2].1 + p[3].1) * 0.25;
    let along_x = map.wear(x + 1, y) as i32 + map.wear(x - 1, y) as i32;
    let along_y = map.wear(x, y + 1) as i32 + map.wear(x, y - 1) as i32;
    let _ = (p[1], p[3]);
    match stage {
        map::WearStage::Trampled => {
            // Flattened, paler grass: a ragged patch in the middle.
            let c = look.trampled;
            let r = 3 * S;
            for dy in -r..=r {
                for dx in -(r + S)..=(r + S) {
                    if dx * dx + dy * dy * 2 <= (r * r) + S * S {
                        if tex::hash2(cx as i32 + dx, cy as i32 + dy, 0xd1) > 0.22 {
                            fb.put(cx as i32 + dx, cy as i32 + dy, c);
                        }
                    }
                }
            }
        }
        map::WearStage::Footpath | map::WearStage::CartTrack => {
            // A trodden line, not a painted plank (B32b): the path is a *stain*
            // in the grass — a dithered band that fades at its edges and breaks
            // up at its ends, in a mix of the ground's own colour. Drawn as
            // solid parallel bars it read as loose planks lying on the lawn.
            let (w, col, strength) = if stage == map::WearStage::CartTrack {
                (3 * S, look.cart_track, 0.72)
            } else {
                // B40: a trodden path has to read at 1× — one 2-px strand read
                // as a scratch across the meadow.
                (3 * S, look.footpath, 0.72)
            };
            let ground = palette::ramp(map.at(x, y).kind)[3];
            // Direction from the worn neighbours (a diagonal path in tile space
            // is a straight line on screen), falling back to the axis sums.
            let here = map.wear(x, y);
            let mut dx = 0.0f32;
            let mut dy = 0.0f32;
            for (ox, oy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                // u8 widened before the +20: a heavily worn neighbour (≥ 236)
                // overflowed in debug builds (and wrapped in release, which
                // wrongly excluded the very trodden paths from the direction).
                if map.wear(x + ox, y + oy) as u16 + 20 >= here as u16 {
                    dx += ox as f32;
                    dy += oy as f32;
                }
            }
            if dx.abs() + dy.abs() < 0.5 {
                if along_x >= along_y {
                    dx = 1.0;
                } else {
                    dy = 1.0;
                }
            }
            let l = (dx * dx + dy * dy).sqrt().max(1.0);
            let (ux, uy) = ((dx - dy) / l, (dx + dy) / l * 0.5);
            let perp = eye_perp(ux, uy);
            let half = 9.0 * S as f32;
            let salt = ((x as u32) << 8) ^ (y as u32) ^ 0xd4;
            for k in 0..w {
                // The band's *width* comes from the strand spacing (B40): at
                // 0.75 px six strands still drew a 3-px scratch.
                let o = (k as f32 - (w as f32 - 1.0) * 0.5) * 1.5;
                let edge = k == 0 || k == w - 1;
                let c = palette::mix(ground, col, if edge { strength * 0.45 } else { strength });
                // Walk the strand in half-pixel steps and dither it in, so the
                // band has a soft edge and a few gaps where grass survives.
                let steps = (half * 2.0) as i32;
                for i in 0..=steps {
                    let t = i as f32 / steps as f32;
                    let along = (t * 2.0 - 1.0) * half;
                    // Taper the ends: a path thins where it leaves the tile.
                    let taper = (1.0 - (along.abs() / half).powi(3) * 0.8).max(0.15);
                    let px = cx + ux * along + perp.0 * o;
                    let py = cy + uy * along + perp.1 * o;
                    let h = tex::hash2(px as i32, py as i32, salt);
                    if h < taper * if edge { 0.5 } else { 0.95 } && in_tile_quad(p, px, py) {
                        fb.put(px as i32, py as i32, c);
                    }
                }
            }
        }
        map::WearStage::None => {}
    }
}

/// Is a screen point inside a tile's own top face? (B58b item 3: ground detail
/// belongs to its tile — a worn strand or a crack that spills over the edge can
/// end up as a stray slanted streak in the sky at the horizon, which is exactly
/// what the audit saw. The static layer's clip is the land line; this one is the
/// tile.)
#[inline]
fn in_tile_quad(p: &[Pt; 4], x: f32, y: f32) -> bool {
    for i in 0..4 {
        let a = p[i];
        let b = p[(i + 1) % 4];
        // Corner order is clockwise on screen, so a point inside is to the same
        // side of every edge.
        if (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0) > 1.0 {
            return false;
        }
    }
    true
}

/// A unit vector across a screen direction, for the width of a path line.
fn eye_perp(dx: f32, dy: f32) -> (f32, f32) {
    let l = (dx * dx + dy * dy).sqrt().max(1.0);
    (-dy / l, dx / l)
}

/// Which of the four tile edges has a lane across it (0 = the 0→1 edge, the
/// `-y` side of the diamond, and so on clockwise).
#[inline]
fn edge_has_lane(map: &TileMap, x: i32, y: i32, edge: usize) -> bool {
    let (dx, dy) = match edge {
        0 => (0, -1),
        1 => (1, 0),
        2 => (0, 1),
        _ => (-1, 0),
    };
    map.at(x + dx, y + dy).kind.is_lane()
}

/// Is this tile a junction (three or four streets meet here)?
pub fn junction_arms(map: &TileMap, x: i32, y: i32) -> u8 {
    (0..4)
        .filter(|&e| edge_has_lane(map, x, y, e))
        .count() as u8
}

/// B83: is the street at this tile built up? A plot within two tiles makes it a
/// town street (kerb and pavement); open ground makes it a lane with verges.
fn built_up(plots: &[u16], map: &TileMap, x: i32, y: i32) -> bool {
    let w = map.w as i32;
    let mut n = 0;
    // A 7x7 window: a 5x5 flips between pavement and verge tile by tile along a
    // street that runs past a gap between plots, which reads as a broken edge.
    for dy in -3..=3 {
        for dx in -3..=3 {
            let (px, py) = (x + dx, y + dy);
            if px < 0 || py < 0 || px >= map.w as i32 || py >= map.h as i32 {
                continue;
            }
            let i = (py * w + px) as usize;
            if plots.get(i).is_some_and(|&k| k != 0) {
                n += 1;
                if n >= 9 {
                    return true;
                }
            }
        }
    }
    false
}

/// Is there a plot tile within `r` of this one (a drive into a lot)?
fn plot_within(plots: &[u16], map: &TileMap, x: i32, y: i32, r: i32) -> bool {
    let w = map.w as i32;
    for dy in -r..=r {
        for dx in -r..=r {
            let (px, py) = (x + dx, y + dy);
            if px < 0 || py < 0 || px >= map.w as i32 || py >= map.h as i32 {
                continue;
            }
            if plots.get((py * w + px) as usize).is_some_and(|&k| k != 0) {
                return true;
            }
        }
    }
    false
}

/// Kerb and pavement on a paved street (B83, replacing B7's `draw_kerbed_sides`).
///
/// * **No outline on the tile boundary.** The old kerb lines sat *on* the tile
///   edge, half of them landing on the neighbour: that drew the grid as a field
///   of diamonds in white felt-tip, which is what the owner's phone shot showed
///   (`draw_paint`'s edge lines did the same inside the tile).
/// * **A kerb that reads as a kerb**: a lit stone edge and the shadow it throws
///   on the asphalt just inside it.
/// * **Pavement in town, verge in the suburbs**: paving stones (with joints)
///   where plots stand within two tiles, the street's own ground otherwise.
/// * **A junction is one paved area**: the kerb never crosses a street mouth,
///   and each corner between two mouths is a rounded pavement wedge, so a
///   crossroads is a crossroads and not four strips laid over each other.
///
/// Everything is decided per pixel from the tile-space coordinate `(u, v)` that
/// [`tex::diamond_tri_px`] hands out, so a band follows the diamond exactly and
/// never leaks onto the tile behind (a screen-space quad did: it tore into a
/// sawtooth along every street).
fn draw_street_edges(
    fb: &mut Framebuffer,
    map: &TileMap,
    x: i32,
    y: i32,
    p: &[Pt; 4],
    g: &Pair,
    wx: &WeatherLook,
    season: &SeasonLook,
    plots: &[u16],
) {
    let t = map.at(x, y);
    let arms = junction_arms(map, x, y);
    let town = built_up(plots, map, x, y);
    // Is there a street across each edge? (0 = v=0, 1 = u=1, 2 = v=1, 3 = u=0)
    let mut closed = [true; 4];
    for (e, c) in closed.iter_mut().enumerate() {
        *c = !edge_has_lane(map, x, y, e);
    }
    // A street that ends at a lot's front lets the kerb open: a drive.
    if arms <= 1 && !town && plot_within(plots, map, x, y, 3) {
        for (e, c) in closed.iter_mut().enumerate() {
            if edge_has_lane(map, x, y, e) {
                continue;
            }
            let (dx, dy) = match e {
                0 => (0, -1),
                1 => (1, 0),
                2 => (0, 1),
                _ => (-1, 0),
            };
            if plot_within(plots, map, x + dx, y + dy, 2) {
                *c = false;
            }
        }
    }
    // A junction corner between two mouths gets a rounded pavement wedge.
    let mut wedge = [false; 4];
    if arms >= 3 {
        for (c, w) in wedge.iter_mut().enumerate() {
            let e_in = (c + 3) % 4;
            *w = !closed[e_in] && !closed[c];
        }
    }
    // Sizes in tile units: one tile is 32 px across the screen vertically.
    const T: f32 = 1.0 / 32.0;
    let band = (if town { 8.0 } else { 5.0 }) * T;
    let kerb = 1.2 * T;
    let wedge_r = 5.5 * T;
    // The verge is the land the street runs through, a little worn at the kerb.
    let verge_base = seasonise(
        weatherise(g.ground(map, x, y, Kind::Grass, 3), &Tile { kind: Kind::Grass, ..t }, wx),
        &Tile { kind: Kind::Grass, ..t },
        season,
        g.at(map, x, y).season,
    );
    let verge_worn = palette::mix(verge_base, pal::DIRT[3], 0.45);
    let asphalt = CARRIAGEWAY;
    let salt = ((x as u32) << 8) ^ (y as u32);
    let diagonal = t.diagonal();
    for tri in triangles(diagonal) {
        tex::diamond_tri_px(fb, p, tri, |px, py, u, v| {
            // Distance to the nearest open edge, in tile units.
            let (du0, du1, du2, du3) = (v, 1.0 - u, 1.0 - v, u);
            let mut d = f32::MAX;
            let mut which = 0usize;
            for (e, dist) in [du0, du1, du2, du3].into_iter().enumerate() {
                if closed[e] && dist < d {
                    d = dist;
                    which = e;
                }
            }
            // The wedge of a junction corner, measured round from the corner.
            let mut wd = f32::MAX;
            for (c, on) in wedge.iter().enumerate() {
                if !*on {
                    continue;
                }
                let (cu, cv) = match c {
                    0 => (u, v),
                    1 => (1.0 - u, v),
                    2 => (1.0 - u, 1.0 - v),
                    _ => (u, 1.0 - v),
                };
                wd = wd.min((cu * cu + cv * cv).sqrt());
            }
            let paved_corner = wd < wedge_r;
            let inside = d < band || paved_corner;
            if !inside && !(d < band + kerb) {
                return palette::TRANSPARENT;
            }
            // The kerb: lit stone at the carriageway boundary, its shadow on the
            // asphalt inside it.
            let edge_d = if paved_corner { wd - wedge_r } else { d - band };
            if edge_d < 0.0 && edge_d > -kerb {
                return if town || paved_corner { KERB_TOP } else { palette::shade(PAVING, 0.86) };
            }
            if edge_d >= 0.0 && edge_d < kerb {
                return if town { KERB_SHADOW } else { palette::shade(asphalt, 0.80) };
            }
            if !inside {
                return palette::TRANSPARENT;
            }
            // Paving or verge. Joints run along the edge, so a pavement reads as
            // flags and a verge as grass. A junction corner is always paving:
            // it is the kerbed corner of the block, and a green sliver between
            // two roads read as a lawn poking into the carriageway.
            if town || paved_corner {
                let along = match which {
                    0 | 2 => u * 5.5,
                    1 | 3 => v * 5.5,
                    _ => u * 5.5,
                };
                let slab = along.floor() as i32;
                let c = palette::jitter(PAVING, tex::hash2(slab, (x + y) & 7, 0xc9), 0.05);
                let c = grain(c, px, py, 0xca);
                if along.fract() < 0.10 {
                    palette::shade(c, 0.87)
                } else {
                    c
                }
            } else {
                // A verge: grass, with a worn lip at the kerb.
                let near = edge_d < 0.0 && edge_d > -3.0 * T;
                let base = if near { verge_worn } else { verge_base };
                let c = grain(palette::jitter(base, tex::hash2(px / 2, py / 2, salt), 0.07), px, py, salt ^ 3);
                if tex::hash2(px, py, salt ^ 5) > 0.90 {
                    palette::shade(c, 1.06)
                } else {
                    c
                }
            }
        });
    }
}

/// B83: the turning circle at the end of a cul-de-sac.
fn draw_turnaround(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let z = level_px(map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5));
    let (cx, cy) = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
    let (rx, ry) = (19.0f32, 9.5f32);
    // The apron: asphalt with the kerb ring round it.
    tex::ellipse(fb, cx, cy + 1.0, rx + 2.0, ry + 1.2, KERB_SHADOW);
    tex::ellipse(fb, cx, cy, rx + 1.2, ry + 0.8, palette::shade(PAVING, 1.05));
    tex::ellipse_px(fb, cx, cy, rx, ry, |px, py, _t| {
        grain(CARRIAGEWAY, px, py, 0x3c)
    });
}

/// The kerb on a standalone sidewalk tile: a lit outer edge and a shadow.
fn draw_kerb_edge(fb: &mut Framebuffer, map: &TileMap, x: i32, y: i32, p: &[Pt; 4]) {
    let edges: [(usize, usize, i32, i32); 4] = [
        (0, 1, 0, -1),
        (1, 2, 1, 0),
        (2, 3, 0, 1),
        (3, 0, -1, 0),
    ];
    for (a, b, dx, dy) in edges {
        if map.at(x + dx, y + dy).kind.is_lane() {
            fb.line(p[a], p[b], palette::shade(pal::SIDEWALK[1], 0.78));
        }
    }
}

/// B83: an unpaved lane is two worn wheel ruts with a grassy crown between
/// them, a soft ragged edge against the verge, and no paint at all.
fn draw_track_ruts(fb: &mut Framebuffer, map: &TileMap, x: i32, y: i32, p: &[Pt; 4], look: &Ground) {
    let along_x = map.at(x - 1, y).kind.is_lane() || map.at(x + 1, y).kind.is_lane();
    let along_y = map.at(x, y - 1).kind.is_lane() || map.at(x, y + 1).kind.is_lane();
    if !(along_x ^ along_y) {
        return;
    }
    let dirt = look.dirt[3];
    let rut = palette::shade(dirt, 0.72);
    let crown = palette::mix(look.grass[2], dirt, 0.30);
    let salt = ((x as u32) << 8) ^ (y as u32) ^ 0x5b;
    let t = map.at(x, y);
    for tri in triangles(t.diagonal()) {
        tex::diamond_tri_px(fb, p, tri, |px, py, u, v| {
            let (along, across) = if along_x { (u, v) } else { (v, u) };
            let a = (across - 0.5).abs();
            // A ragged outer edge: the lane fades into the verge instead of
            // ending on a line.
            let wobble = (tex::hash2(px / 3, py / 3, salt ^ 7) - 0.5) * 0.10;
            if a > 0.34 + wobble {
                return palette::TRANSPARENT;
            }
            // The crown between the ruts keeps its grass, patchily.
            if a < 0.055 {
                if tex::hash2(px / 2, py / 2, salt ^ 9) > 0.45 {
                    return crown;
                }
            }
            let in_rut = (across - 0.30).abs() < 0.055 || (across - 0.70).abs() < 0.055;
            let mut c = if in_rut {
                rut
            } else {
                // Between and outside the ruts the dirt is thin enough for the
                // odd tuft to come through.
                if tex::hash2(px, py, salt ^ 11) > 0.86 {
                    crown
                } else {
                    dirt
                }
            };
            if tex::hash2(px / 2, py, salt ^ 13) > 0.93 {
                c = palette::shade(c, 0.88);
            }
            let _ = along;
            grain(c, px, py, salt ^ 15)
        });
    }
}

/// A kerbside lamp: a post, an arm over the road and a warm pool of light. Only
/// some paved tiles carry one, and only at night.
fn draw_lamp(fb: &mut Framebuffer, map: &TileMap, x: i32, y: i32, p: &[Pt; 4], glow: f32) {
    let h = map.tile_hash(x, y);
    if h < 0.45 {
        return;
    }
    // Post at the pavement side.
    let along_x = map.at(x - 1, y).kind.is_lane() && map.at(x + 1, y).kind.is_lane();
    let (px, py) = if along_x {
        (p[2].0, p[2].1)
    } else {
        (p[3].0, p[3].1)
    };
    let (px, py) = (px as i32, py as i32);
    let post = 8 * S;
    // A 2-px post with a lit edge.
    fb.fill_rect(px, py - post, S, post, pal::OUTLINE);
    fb.fill_rect(px + 1, py - post, 1, post, palette::shade(pal::OUTLINE, 2.2));
    // An arm reaching over the carriageway.
    let dir = if along_x { -1 } else { -1 };
    let arm = 3 * S;
    fb.line(
        (px as f32 + 1.0, py as f32 - post as f32),
        (px as f32 + 1.0 + (arm * dir) as f32, py as f32 - post as f32),
        palette::shade(pal::OUTLINE, 2.2),
    );
    let hx = px + 1 + arm * dir;
    // Lamp head.
    fb.fill_rect(hx - S / 2, py - post - 1, 2 * S, S, pal::LAMP_GLOW);
    fb.put_blended(hx, py - post - 2, pal::LAMP_GLOW, 0.8);
    // Warm pool on the road, fading with distance from the head.
    let strength = ((glow - 0.45) / 0.55).clamp(0.0, 1.0);
    // B36b: a real pool of light on the road, not a smudge.
    let rad = 9 * S;
    for dy in 0..(5 * S) {
        for dx in -rad..=rad {
            let d = ((dx * dx) as f32 + (dy * 3) as f32).sqrt();
            let a = (1.0 - d / (rad as f32 + 2.0)).clamp(0.0, 1.0) * 0.80 * strength;
            if a > 0.02 {
                fb.put_blended(hx + dx, py - 1 + dy, pal::LAMP_GLOW, a);
            }
        }
    }
}

/// Centre-line dashes on straight road tiles.
fn draw_road_marking(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    let along_x = map.at(x - 1, y).kind == Kind::Road && map.at(x + 1, y).kind == Kind::Road;
    let along_y = map.at(x, y - 1).kind == Kind::Road && map.at(x, y + 1).kind == Kind::Road;
    if !along_x && !along_y {
        return;
    }
    if (x + y) % 2 != 0 {
        return;
    }
    let z = level_px(map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5));
    // Two 2-px dashes per tile, so the line is continuous at 64-px tiles.
    for (t0, t1) in [(0.18f32, 0.36f32), (0.62, 0.80)] {
        for k in 0..2 {
            let o = k as f32 * 0.032;
            if along_x && !along_y {
                let a = cam.project(x as f32 + t0, y as f32 + 0.5 + o, z);
                let b = cam.project(x as f32 + t1, y as f32 + 0.5 + o, z);
                fb.line(a, b, pal::ROAD_LINE);
            } else if along_y && !along_x {
                let a = cam.project(x as f32 + 0.5 + o, y as f32 + t0, z);
                let b = cam.project(x as f32 + 0.5 + o, y as f32 + t1, z);
                fb.line(a, b, pal::ROAD_LINE);
            }
        }
    }
}

/// A manhole cover on some straight asphalt tiles (D54): a dark disc with a
/// rim, sitting in the carriageway.
fn draw_manhole(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    if map.at(x, y).kind != Kind::Road {
        return;
    }
    let h = map.tile_hash(x, y);
    if h < 0.55 {
        return;
    }
    let along_x = map.at(x - 1, y).kind.is_lane() && map.at(x + 1, y).kind.is_lane();
    let along_y = map.at(x, y - 1).kind.is_lane() && map.at(x, y + 1).kind.is_lane();
    if along_x == along_y {
        return;
    }
    // Put it in the near half of the carriageway so a kerb does not cover it.
    let (fx, fy) = if along_x {
        (x as f32 + 0.35 + 0.3 * h, y as f32 + 0.72)
    } else {
        (x as f32 + 0.72, y as f32 + 0.35 + 0.3 * h)
    };
    let z = level_px(map.surface_level_at(fx, fy));
    let (sx, sy) = cam.project(fx, fy, z);
    // An iso circle is a 2:1 ellipse.
    tex::ellipse(fb, sx, sy, 3.0 * S as f32, 1.5 * S as f32, palette::shade(pal::ROAD[1], 0.62));
    tex::ellipse(fb, sx, sy - 0.5, 2.0 * S as f32, 1.0 * S as f32, palette::shade(pal::ROAD[1], 0.78));
    // Two slots and a highlight on the rim.
    fb.put(sx as i32, sy as i32, palette::shade(pal::ROAD[1], 0.55));
    fb.put(sx as i32 + 1, sy as i32, palette::shade(pal::ROAD[1], 0.55));
    fb.line((sx - 2.0, sy - 1.5), (sx + 2.0, sy - 1.5), palette::shade(pal::ROAD[1], 1.25));
}

/// A gutter drain beside the kerb on some road tiles (D54).
fn draw_drain(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, x: i32, y: i32) {
    if map.at(x, y).kind != Kind::Road {
        return;
    }
    let h = map.tile_hash(x, y);
    if h > 0.30 {
        return;
    }
    let along_x = map.at(x - 1, y).kind.is_lane() && map.at(x + 1, y).kind.is_lane();
    let along_y = map.at(x, y - 1).kind.is_lane() && map.at(x, y + 1).kind.is_lane();
    if along_x == along_y {
        return;
    }
    let (fx, fy) = if along_x {
        (x as f32 + 0.5, y as f32 + 0.93)
    } else {
        (x as f32 + 0.93, y as f32 + 0.5)
    };
    let z = level_px(map.surface_level_at(fx, fy));
    let (sx, sy) = cam.project(fx, fy, z);
    // A grate: a dark rectangle with bars along the kerb.
    fb.fill_rect(sx as i32 - 3, sy as i32 - 1, 6, 3, palette::shade(pal::OUTLINE, 1.3));
    for k in 0..3 {
        fb.put(sx as i32 - 2 + k * 2, sy as i32 - 1, palette::shade(pal::OUTLINE, 0.8));
        fb.put(sx as i32 - 2 + k * 2, sy as i32, palette::shade(pal::OUTLINE, 0.8));
    }
    fb.put(sx as i32 - 3, sy as i32 + 1, palette::shade(pal::SIDEWALK[1], 1.05));
}

/// Screen anchor and surface height of a prop standing on a tile.
fn prop_anchor(cam: &Camera, map: &TileMap, x: i32, y: i32) -> (i32, i32) {
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
    let mut z = level_px(map.surface_level_at(fx, fy));
    // B70: the smoothed ground under the prop (centre of the tile: the mean).
    let dz = map.soft_dz(x, y);
    z += (dz.iter().map(|&d| d as i32).sum::<i32>() as f32 / 4.0).round() as i32;
    let (sx, sy) = cam.project(fx, fy, z);
    (sx as i32, sy as i32)
}

fn draw_prop(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &TileMap,
    x: i32,
    y: i32,
    land_y: i32,
    season: &SeasonLook,
    snow: f32,
    look: &Ground,
    claims: &[u8],
    derived: &[Option<Prop>],
) {
    let t = map.at(x, y);
    // B69: whatever tree the sway layer held for this tile is stale now.
    crate::sway::forget(x, y);
    let (cx, cy) = prop_anchor(cam, map, x, y);
    // A prop standing behind the horizon is out of the world: skip it whole, so
    // it can neither float in the sky band nor be cut by the land line (B42b).
    if cy < land_y - 6 {
        return;
    }
    // The prop this tile draws: the stored one, else the derived garden/street
    // tree the planting pass put here (B74: the sim's map is left alone, so a
    // garden tree never reaches a save).
    let stored = t.prop;
    let orchard = crate::farmland::orchard_prop(map, x, y, look.biome);
    let prop = if let (Prop::None, Some(o)) = (stored, orchard) {
        o
    } else if stored != Prop::None {
        stored
    } else {
        derived
            .get((y * map.w + x) as usize)
            .copied()
            .flatten()
            .unwrap_or(Prop::None)
    };
    if prop == Prop::None {
        return;
    }
    // Draw-time cleaning (B74): a prop whose footprint or crown would touch a
    // road, a rail bed, a paving, a plaza or a wall is never drawn. A small
    // species may stand a pixel off a wall, so it is kept in the margin.
    if crate::planting::skip_prop(map, claims, x, y, prop) {
        return;
    }
    // In a closed wood the trunks are shaded out and run short: that is what
    // makes a wood read as a mass of foliage rather than a field of poles (B42b).
    let dense = matches!(prop, Prop::Tree { .. })
        && [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]
            .iter()
            .filter(|(dx, dy)| map.at(x + dx, y + dy).prop.is_tree())
            .count()
            >= 4;
    match prop {
        Prop::None => {}
        Prop::Tree {
            species,
            size,
            variant,
        } => {
            let jitter = map.tile_hash(x, y);
            // B69: a tree that sways is not painted into the static layer. It is
            // drawn once, at rest, into the sway layer's sprite store, and only
            // its depth silhouette lands here (see `sway`).
            if crate::sway::sways(species) && crate::sway::active() {
                capture_sway_tree(
                    fb, x, y, cx, cy, species, size, variant, jitter, season, snow, dense, look,
                );
                return;
            }
            draw_tree_dispatch(fb, cx, cy, species, size, variant, jitter, season, snow, dense, look);
        }
        Prop::Bush(v) => draw_bush(fb, cx, cy, v, map.tile_hash(x, y), snow, look),
        Prop::Boulder(v) => draw_boulder(fb, cx, cy, v, map.tile_hash(x, y), look),
        Prop::Meadow(v) => draw_meadow(fb, cx, cy, v, map.tile_hash(x, y), look),
        Prop::TallGrass(v) => draw_tall_grass(fb, cx, cy, v, map.tile_hash(x, y), look),
        Prop::Reeds(v) => draw_reeds(fb, cx, cy, v, map.tile_hash(x, y), look),
        Prop::Log(v) => draw_log(fb, cx, cy, v, map.tile_hash(x, y), look),
        Prop::Stone(v) => draw_stones(fb, cx, cy, v, map.tile_hash(x, y), look),
    }
}

/// The species dispatch for one tree standing at `(cx, cy)`.
#[allow(clippy::too_many_arguments)]
fn draw_tree_dispatch(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    species: TreeSpecies,
    size: u8,
    variant: u8,
    jitter: f32,
    season: &SeasonLook,
    snow: f32,
    dense: bool,
    look: &Ground,
) {
    if species == TreeSpecies::Cactus {
        draw_cactus(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Joshua {
        draw_joshua(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::FanPalm {
        draw_fan_palm(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Flamboyant {
        draw_flowering(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Mangrove {
        draw_mangrove(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Acacia {
        draw_acacia(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Baobab {
        draw_baobab(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::BaldCypress {
        draw_bald_cypress(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Olive {
        draw_olive(fb, cx, cy, size, variant, jitter, look);
    } else if species.is_park_accent() {
        draw_park_tree(fb, cx, cy, size, species, variant, jitter, season, look);
    } else if species.kind() == TreeKind::Conifer {
        draw_conifer(fb, cx, cy, size, variant, jitter, season, species, snow, dense, look);
    } else {
        draw_broadleaf_season(fb, cx, cy, size, variant, jitter, season, species, snow, dense, look);
    }
}

/// B69: the soft shade a closed wood lays on the floor under a crown. It is
/// ground, so it stays in the static layer when the tree itself moves out.
fn sway_ground_shade(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    species: TreeSpecies,
    size: u8,
    jitter: f32,
    season: &SeasonLook,
    dense: bool,
    look: &Ground,
) {
    if !dense
        || matches!(
            species,
            TreeSpecies::Cactus
                | TreeSpecies::Joshua
                | TreeSpecies::FanPalm
                | TreeSpecies::Flamboyant
                | TreeSpecies::Mangrove
                | TreeSpecies::Acacia
                | TreeSpecies::Baobab
                | TreeSpecies::BaldCypress
                | TreeSpecies::Olive
        )
        || species.is_park_accent()
    {
        return;
    }
    if species.kind() == TreeKind::Conifer {
        let w0 = match species {
            TreeSpecies::Pine => 22.0,
            TreeSpecies::Spruce => 18.0,
            TreeSpecies::Fir => 14.0,
            TreeSpecies::Cedar => 26.0,
            _ => 18.0,
        };
        let boost = if size > 0 { 1.28 } else { 1.0 };
        let w = w0 * boost * (0.9 + 0.2 * jitter);
        canopy_shade(fb, cx, cy, (w * 0.55) as i32, (w * 0.24) as i32, look);
    } else if !broadleaf_bare(species, jitter, season) {
        let crown = crown_for(species, size > 0, jitter);
        canopy_shade(fb, cx, cy, (crown.w * 0.62) as i32, (crown.w * 0.28) as i32, look);
    }
}

/// Does `draw_broadleaf_season` draw this tree as bare winter branches?
fn broadleaf_bare(sp: TreeSpecies, jitter: f32, season: &SeasonLook) -> bool {
    let blossom_tree = matches!(sp, TreeSpecies::Cherry | TreeSpecies::Apple);
    let blossom = if blossom_tree {
        season.blossom * (0.55 + 0.45 * jitter)
    } else {
        season.blossom * 0.12
    };
    season.bare_for(jitter) > 0.72 && blossom < 0.2
}

/// B69: draw one swaying tree into the sway layer's sprite store, put its
/// ground shade and its depth silhouette into the static layer.
#[allow(clippy::too_many_arguments)]
fn capture_sway_tree(
    fb: &mut Framebuffer,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    species: TreeSpecies,
    size: u8,
    variant: u8,
    jitter: f32,
    season: &SeasonLook,
    snow: f32,
    dense: bool,
    look: &Ground,
) {
    sway_ground_shade(fb, cx, cy, species, size, jitter, season, dense, look);
    crate::sway::with_sink(|layer| {
        let scratch = layer.scratch(cx, cy);
        draw_tree_dispatch(scratch, cx, cy, species, size, variant, jitter, season, snow, dense, look);
        layer.insert(fb, (x, y), (cx, cy), species);
    });
}

/// Shape and colour of one species' crown (D64). `variant` jitters these per
/// tree, so two oaks side by side are never identical.
#[derive(Clone, Copy)]
struct Crown {
    /// Canopy width and height at size 0 (px), and the trunk.
    w: f32,
    h: f32,
    trunk: i32,
    trunk_w: i32,
    /// 0 = a round crown, 1 = a willow's hanging curtain.
    droop: f32,
    /// Where the crown sits on the trunk (0 = on top, 1 = half-way down).
    low: f32,
    /// Autumn colour bias: (r, g, b) the season's turn is pulled towards.
    autumn: [u8; 3],
    /// How strongly `autumn` applies.
    autumn_bias: f32,
    /// How many clumps the crown is built from, and how far they spread
    /// (as a fraction of the crown radius): 1 = a single oval, 3 = a lobed oak.
    lobes: u8,
    spread: f32,
}

fn crown_for(sp: TreeSpecies, big: bool, jitter: f32) -> Crown {
    let (w, h, trunk, trunk_w) = match sp {
        TreeSpecies::Oak => (26.0, 24.0, 12, 3),
        TreeSpecies::Maple => (24.0, 22.0, 11, 3),
        TreeSpecies::Birch => (18.0, 28.0, 14, 2),
        TreeSpecies::Beech => (23.0, 25.0, 12, 3),
        TreeSpecies::Elm => (22.0, 30.0, 15, 3),
        TreeSpecies::Willow => (25.0, 20.0, 11, 3),
        TreeSpecies::Poplar => (16.0, 33.0, 15, 3),
        TreeSpecies::Apple => (16.0, 14.0, 8, 2),
        TreeSpecies::Cherry => (18.0, 16.0, 9, 2),
        _ => (22.0, 24.0, 12, 3),
    };
    let k = if big { 1.55 } else { 1.0 } * (0.90 + 0.20 * jitter);
    let autumn = match sp {
        TreeSpecies::Maple => [0xd2, 0x45, 0x2c],     // scarlet
        TreeSpecies::Oak => [0xb2, 0x7a, 0x2e],       // bronze
        TreeSpecies::Birch => [0xe8, 0xd0, 0x4a],     // yellow
        TreeSpecies::Beech => [0xc4, 0x84, 0x3a],     // copper
        TreeSpecies::Elm => [0xd8, 0xb0, 0x3c],
        TreeSpecies::Willow => [0xd8, 0xd0, 0x62],    // yellow-green
        TreeSpecies::Poplar => [0xe2, 0xbe, 0x46],
        TreeSpecies::Apple => [0xd8, 0x8a, 0x3a],
        TreeSpecies::Cherry => [0xd8, 0x5a, 0x3a],
        _ => [0xb8, 0x8a, 0x3a],
    };
    let autumn_bias = match sp {
        TreeSpecies::Maple | TreeSpecies::Cherry => 0.75,
        TreeSpecies::Oak | TreeSpecies::Beech => 0.45,
        TreeSpecies::Birch | TreeSpecies::Poplar | TreeSpecies::Willow | TreeSpecies::Elm => 0.6,
        TreeSpecies::Apple => 0.5,
        _ => 0.4,
    };
    let (lobes, spread) = match sp {
        TreeSpecies::Oak => (3, 0.55),
        TreeSpecies::Maple => (3, 0.50),
        TreeSpecies::Beech => (3, 0.42),
        TreeSpecies::Elm => (2, 0.45),
        TreeSpecies::Birch => (2, 0.30),
        TreeSpecies::Poplar => (1, 0.0),
        TreeSpecies::Willow => (3, 0.60),
        TreeSpecies::Apple => (3, 0.35),
        TreeSpecies::Cherry => (3, 0.38),
        _ => (2, 0.35),
    };
    Crown {
        w: w * k,
        h: h * k,
        trunk: (trunk as f32 * (if big { 1.5 } else { 1.0 })) as i32,
        trunk_w,
        droop: if sp == TreeSpecies::Willow { 1.0 } else { 0.0 },
        low: match sp {
            TreeSpecies::Willow => 0.45,
            TreeSpecies::Apple => 0.30,
            TreeSpecies::Cherry => 0.28,
            _ => 0.10,
        },
        autumn,
        autumn_bias,
        lobes,
        spread,
    }
}

/// Canopy tones for one deciduous tree: the season's greens turned towards its
/// autumn colour by `turn`, and optionally washed with blossom.
fn canopy_tones(season: &SeasonLook, turn: f32, blossom: f32) -> [Color; 3] {
    let mut out = [[0u8; 3]; 3];
    // B76: spring's first leaf is a pale yellow-green, not summer's dark green.
    let fresh = season.fresh.clamp(0.0, 1.0);
    let new_leaf = [[0xa2, 0xd0, 0x58], [0x84, 0xbc, 0x4a], [0x60, 0x9c, 0x40]];
    for k in 0..3 {
        let g = season.green[k];
        let t = season.turn[k];
        let mut c = [
            (g[0] as f32 + (t[0] as f32 - g[0] as f32) * turn) as u8,
            (g[1] as f32 + (t[1] as f32 - g[1] as f32) * turn) as u8,
            (g[2] as f32 + (t[2] as f32 - g[2] as f32) * turn) as u8,
        ];
        if fresh > 0.01 {
            let w = fresh * if k == 0 { 0.70 } else { 0.55 };
            c = [
                (c[0] as f32 + (new_leaf[k][0] as f32 - c[0] as f32) * w) as u8,
                (c[1] as f32 + (new_leaf[k][1] as f32 - c[1] as f32) * w) as u8,
                (c[2] as f32 + (new_leaf[k][2] as f32 - c[2] as f32) * w) as u8,
            ];
        }
        if blossom > 0.05 {
            let pink = [[0xf6, 0xd2, 0xe0], [0xf2, 0xb8, 0xcc], [0xd8, 0x96, 0xb0]][k];
            c = [
                (c[0] as f32 + (pink[0] as f32 - c[0] as f32) * blossom) as u8,
                (c[1] as f32 + (pink[1] as f32 - c[1] as f32) * blossom) as u8,
                (c[2] as f32 + (pink[2] as f32 - c[2] as f32) * blossom) as u8,
            ];
        }
        out[k] = c;
    }
    [
        palette::rgb(out[0][0], out[0][1], out[0][2]),
        palette::rgb(out[1][0], out[1][1], out[1][2]),
        palette::rgb(out[2][0], out[2][1], out[2][2]),
    ]
}

/// A deciduous tree with the season on it (B28, D54, D64): the crown shape,
/// trunk and autumn colour come from the species, and `variant` + the tile hash
/// vary size, lean and hue so no two neighbours look stamped. Green in summer,
/// blossom in spring, turning per species in autumn, bare branches in winter.
fn draw_broadleaf_season(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    season: &SeasonLook,
    sp: TreeSpecies,
    snow: f32,
    dense: bool,
    look: &Ground,
) {
    let turn = season.turn_for(jitter);
    // The blossom trees are the orchard ones; B76: a good share of the other
    // broadleaves flower too (plum, magnolia, hawthorn), so a spring street is
    // visibly in flower instead of one rare orchard row.
    let blossom_tree = matches!(sp, TreeSpecies::Cherry | TreeSpecies::Apple);
    let blossom = if blossom_tree {
        season.blossom * (0.85 + 0.15 * jitter)
    } else if (jitter * 7.0).fract() > 0.70 {
        // A distinct flowering tree (30 % of the broadleaves, B76 r2), not a
        // wash on every crown: the rest stay
        // green, so the street reads as spring green with blossom, not as grey.
        season.blossom * 0.80
    } else {
        0.0
    };
    if season.bare_for(jitter) > 0.72 && blossom < 0.2 {
        draw_bare_tree(fb, cx, cy, size, variant, jitter, snow, look);
        // A few fallen leaves at the foot.
        if season.litter > 0.05 {
            let n = (season.litter * 9.0) as i32;
            for i in 0..n {
                let a = jitter * 21.0 + i as f32 * 1.9;
                let dx = ((a.cos() * 10.0) as i32).clamp(-10, 10);
                let dy = ((a.sin() * 5.0) as i32 + 1).clamp(-3, 5);
                let c = season.turn[(i as usize) % 3];
                fb.put(cx + dx, cy + dy, palette::rgb(c[0], c[1], c[2]));
            }
        }
        return;
    }
    let crown = crown_for(sp, size > 0, jitter);
    // Per-tree hue jitter: a canopy two steps off its neighbour's.
    let hue = (jitter * 5.0).fract() - 0.5;
    // Inside a closed wood the floor is shaded by the canopy above: without it
    // the crowns read as balloons floating over bright grass (B42b).
    if dense {
        canopy_shade(fb, cx, cy, (crown.w * 0.62) as i32, (crown.w * 0.28) as i32, look);
    }
    let (light, mid, dark) = {
        let tone = canopy_tones(season, turn, blossom);
        let (mut l, mut m, mut d) = (tone[0], tone[1], tone[2]);
        let a = (0.6 + 0.5 * hue.clamp(-0.5, 0.5)).clamp(0.75, 1.15);
        l = palette::shade(l, a);
        m = palette::shade(m, a * 0.99);
        d = palette::shade(d, a * 0.97);
        // The species' own autumn colour, so a maple goes scarlet and a birch
        // yellow even when the season's turn is shared.
        if turn > 0.18 {
            let t = (turn * crown.autumn_bias).min(1.0);
            let au = palette::rgb(crown.autumn[0], crown.autumn[1], crown.autumn[2]);
            l = palette::mix(l, palette::shade(au, 1.1), t * 0.75);
            m = palette::mix(m, au, t * 0.8);
            d = palette::mix(d, palette::shade(au, 0.82), t * 0.8);
        }
        (l, m, d)
    };
    let trunk_col = if sp == TreeSpecies::Birch {
        // Birch bark: pale with dark marks.
        palette::rgb(0xd8, 0xd4, 0xc8)
    } else {
        look.trunk
    };
    // Per-tree lean: the crown sits a pixel or two off the trunk's foot.
    let lean = ((jitter * 17.0).fract() - 0.5) * 3.0;
    let canopy_w = crown.w;
    let canopy_h = crown.h;
    // A tree inside a closed wood shows only a short trunk.
    let trunk_h = (((crown.trunk as f32) * if dense { 0.55 } else { 1.0 }) as i32).max(3);
    let trunk_w = crown.trunk_w;
    // Trunk with bark texture and a root flare.
    fb.fill_rect(cx - trunk_w / 2, cy - trunk_h, trunk_w, trunk_h + 1, trunk_col);
    fb.fill_rect(cx - trunk_w / 2 - 1, cy - 1, 1, 1, palette::shade(trunk_col, 0.85));
    if sp == TreeSpecies::Birch {
        for i in 0..(trunk_h / 3).max(1) {
            let yy = cy - 2 - i * 3;
            fb.put(cx - trunk_w / 2, yy, palette::shade(trunk_col, 0.62));
            fb.put(cx, yy - 1, palette::shade(trunk_col, 0.7));
        }
    } else {
        fb.fill_rect(cx + trunk_w / 2 - 1, cy - trunk_h, 1, trunk_h, palette::shade(trunk_col, 0.78));
    }
    // The canopy: an ellipse of leaf clumps, lit from the upper left.
    let ccx = cx as f32 + lean;
    let ccy = (cy - trunk_h) as f32 - canopy_h * (0.5 - crown.low);
    let rx = canopy_w * 0.5;
    let ry = canopy_h * 0.5;
    let salt = (jitter * 4096.0) as u32;
    // The crown is one to three overlapping clumps, so a species has its own
    // silhouette (a lobed oak, a slim poplar) instead of one egg for all.
    let clumps = crown.lobes.max(1) as usize;
    let mut clump = |ox: f32, oy: f32, sx: f32, sy: f32| {
        let cxx = ccx + ox;
        let cyy = ccy + oy;
        let rrx = rx * sx;
        let rry = ry * sy;
        tex::ellipse_px(fb, cxx, cyy, rrx, rry, |px, py, d| {
        // Leaf clumps: a low-frequency patch hash picks light or shadow, and a
        // fine hash dithers inside the patch, so the canopy has structure
        // without a lattice pattern.
        let patch = tex::hash2(px / 3, py / 3, salt);
        let fine_px = tex::hash2(px, py, salt ^ 0x51);
        // Light from the upper left: a soft radial term plus the diagonal, both
        // dithered by the clump hash so the canopy has no hard banding.
        let lit = ((px as f32 - cxx) / rrx + (py as f32 - cyy) / rry) * 0.6
            + (1.0 - d) * 0.5
            + (patch - 0.5) * 0.5;
        let mut c = if lit < -0.15 {
            light
        } else if lit > 0.55 {
            dark
        } else {
            mid
        };
        if patch > 0.66 {
            c = palette::shade(c, 1.07); // a clump catching the sun
        } else if patch < 0.30 {
            c = palette::shade(c, 0.90); // a shaded gap between clumps
        }
        if fine_px > 0.74 {
            c = palette::shade(c, 1.05);
        } else if fine_px < 0.24 {
            c = palette::shade(c, 0.96);
        }
        if d > 0.90 {
            c = palette::shade(c, 0.92); // the canopy's rim darkens
        }
        c
        });
    };
    for i in 0..clumps {
        let a = jitter * 9.0 + i as f32 * (std::f32::consts::TAU / clumps as f32);
        let (ox, oy, s) = if clumps == 1 {
            (0.0, 0.0, 1.0)
        } else {
            (
                a.cos() * rx * crown.spread,
                a.sin() * ry * crown.spread * 0.6,
                0.68 + 0.12 * ((a * 1.7).sin() * 0.5 + 0.5),
            )
        };
        clump(ox, oy, s, s);
    }
    // B76: snow lying on the crown. It sits on the top of the clumps and thins
    // towards the underside, so a snowed tree keeps its dark shape below.
    if snow > 0.08 {
        snow_cap(fb, ccx, ccy, rx, ry, snow, salt);
    }
    // A few leaf tips breaking the silhouette, so the canopy is not a smooth
    // blob at 1x.
    for i in 0..6 {
        let a = jitter * 31.0 + i as f32 * 1.37;
        let (dx, dy) = (a.cos(), a.sin() * 0.6);
        let px = ccx + dx * (rx + 1.5);
        let py = ccy + dy * (ry + 1.5);
        fb.put(px as i32, py as i32, if i % 2 == 0 { light } else { mid });
    }
    // A willow's curtain: strands hanging off the crown, thinning at the tips.
    if crown.droop > 0.0 {
        let n = (5.0 + jitter * 4.0) as i32;
        for i in 0..n {
            let f = i as f32 / n as f32 - 0.5;
            let x = ccx + f * canopy_w * 1.05;
            if (x - ccx).abs() > rx * 1.1 {
                continue;
            }
            let len = (6.0 + 9.0 * (1.0 - f.abs() * 1.6).max(0.0) + jitter * 4.0) as i32;
            let base = (ccy + ry * 0.55) as i32;
            for k in 0..len {
                let c = if k > len * 2 / 3 { dark } else { mid };
                fb.put(x as i32 + ((k as f32 * 0.25).sin() * 0.6) as i32, base + k, c);
            }
        }
    }
    // Autumn leaves under a tree that is still holding some.
    if season.litter > 0.05 && turn > 0.35 {
        let n = (season.litter * 6.0) as i32;
        for i in 0..n {
            let a = jitter * 17.0 + i as f32 * 2.3;
            let dx = ((a.cos() * 8.0) as i32).clamp(-8, 8);
            let dy = ((a.sin() * 3.0) as i32 + 3).clamp(0, 6);
            let c = season.turn[(i as usize) % 3];
            fb.put(cx + dx, cy + dy, palette::rgb(c[0], c[1], c[2]));
        }
    }
    // B76: petals on the ground under a tree in flower — petals, not leaves,
    // so the spring pavement reads pink.
    if blossom > 0.15 {
        let n = (blossom * 8.0) as i32;
        for i in 0..n {
            let a = jitter * 23.0 + i as f32 * 2.7;
            let dx = ((a.cos() * 9.0) as i32).clamp(-9, 9);
            let dy = ((a.sin() * 3.5) as i32 + 3).clamp(0, 6);
            let c = [[0xf6, 0xd2, 0xe0], [0xfa, 0xe4, 0xec], [0xf2, 0xb8, 0xcc]]
                [(i as usize) % 3];
            fb.put(cx + dx, cy + dy, palette::rgb(c[0], c[1], c[2]));
        }
    }
    let _ = variant;
}

/// B76: a load of snow on a deciduous crown. Patchy and blue-white, lit on the
/// top-left, so a snowed tree reads as snowed and not as a white blob.
fn snow_cap(fb: &mut Framebuffer, cx: f32, cy: f32, rx: f32, ry: f32, snow: f32, salt: u32) {
    let cap = palette::rgb(0xec, 0xf3, 0xfa);
    let shade = palette::rgb(0xc8, 0xd6, 0xe4);
    let x0 = (cx - rx) as i32;
    let x1 = (cx + rx) as i32;
    let y0 = (cy - ry) as i32;
    for py in y0..=cy as i32 {
        for px in x0..=x1 {
            let nx = (px as f32 - cx) / rx.max(1.0);
            let ny = (py as f32 - cy) / ry.max(1.0);
            if nx * nx + ny * ny > 1.0 {
                continue;
            }
            let upper = (-ny).clamp(0.0, 1.0);
            let coverage = snow * (0.22 + 0.78 * upper);
            let h = tex::hash2(px, py, salt ^ 0x7A15);
            if h > 0.45 + 0.45 * (1.0 - coverage) {
                continue;
            }
            // The lower rim keeps a colder blue; the top catches the light.
            let c = if upper > 0.55 { cap } else { shade };
            fb.put_blended(px, py, c, (0.45 + 0.5 * coverage).min(0.95));
        }
    }
}

/// A soft patch of shade on the ground, for the floor of a closed wood.
fn canopy_shade(fb: &mut Framebuffer, cx: i32, cy: i32, rx: i32, ry: i32, look: &Ground) {
    let shade = palette::mix(look.grass[2], palette::rgb(0x14, 0x22, 0x12), 0.55);
    for dy in -ry..=ry {
        let t = dy as f32 / ry.max(1) as f32;
        let half = (rx as f32 * (1.0 - t * t).max(0.0).sqrt()) as i32;
        for dx in -half..=half {
            let edge = 1.0 - (dx.abs() as f32 / half.max(1) as f32).powi(2);
            fb.put_blended(cx + dx, cy - 2 + dy, shade, 0.22 + 0.20 * edge);
        }
    }
}

/// A bare deciduous tree: the trunk, a fan of branches and fine twigs (B28),
/// at 2x the old detail so a winter wood is not a field of sticks.
fn draw_bare_tree(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    snow: f32,
    look: &Ground,
) {
    let (trunk, spread, twigs) = if size > 0 { (18, 11, 4) } else { (14, 8, 3) };
    let trunk_col = if variant % 3 == 1 {
        palette::rgb(0xd0, 0xcc, 0xc0)
    } else {
        look.trunk
    };
    fb.fill_rect(cx - 1, cy - trunk, 2, trunk + 1, trunk_col);
    let top = cy - trunk;
    let skew = (variant % 3) as i32 - 1;
    let branches: [(i32, i32); 5] = [
        (-spread, 7),
        (spread, 9),
        (-spread / 2, 12),
        (spread / 2, 13),
        (-2 + skew, 15),
    ];
    let rime = snow > 0.15;
    for (i, (dx, dy)) in branches.into_iter().enumerate() {
        if i == 4 && jitter < 0.5 {
            continue;
        }
        let ex = cx + dx;
        let ey = top - dy;
        fb.line((cx as f32, top as f32), (ex as f32, ey as f32), trunk_col);
        if rime {
            // B76: snow caught along the upper side of a winter branch.
            let n = (snow * 5.0) as i32;
            for k in 0..n {
                let f = (k as f32 + 0.5) / n.max(1) as f32;
                let x = cx as f32 + (ex - cx) as f32 * f;
                let y = top as f32 + (ey - top) as f32 * f - 1.0;
                fb.put_blended(x as i32, y as i32, palette::rgb(0xe8, 0xf0, 0xf8), 0.55 + 0.4 * snow);
            }
        }
        // Twigs at the tip: two short forks.
        for k in 0..2 {
            let a = (i as f32 * 1.7 + k as f32) * 0.9;
            let tl = 3.0 + jitter * 2.0;
            fb.line(
                (ex as f32, ey as f32),
                (ex as f32 + a.cos() * tl * dx.signum() as f32, ey as f32 - a.sin().abs() * tl),
                palette::shade(trunk_col, 0.9),
            );
        }
        let _ = twigs;
    }
}

/// Conifer: a trunk and a stack of tiers. Four species (D64): a pine holds its
/// tiers high on a bare trunk, a spruce is a full cone, a fir is narrow and
/// dense, a cedar is wide, dark and ragged. Needles keep their green all
/// winter; only a faint browning in deep cold.
fn draw_conifer(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    season: &SeasonLook,
    sp: TreeSpecies,
    snow: f32,
    dense: bool,
    look: &Ground,
) {
    let big = size > 0;
    // Species shape: (width, height, trunk, tiers, tone shift).
    let (w0, h0, trunk, tiers, tone) = match sp {
        // A pine shows more trunk and fewer, flatter tiers; a fir is the
        // narrowest and densest; a cedar is wide and dark.
        TreeSpecies::Pine => (22.0, 34.0, 15, 3, 1),
        TreeSpecies::Spruce => (18.0, 36.0, 6, 5, 0),
        TreeSpecies::Fir => (14.0, 44.0, 5, 6, 2),
        TreeSpecies::Cedar => (26.0, 32.0, 9, 4, 3),
        _ => (18.0, 34.0, 6, 4, 0),
    };
    let boost = if big { 1.28 } else { 1.0 };
    let w = w0 * boost * (0.9 + 0.2 * jitter);
    let hgt = h0 * boost * (0.92 + 0.16 * jitter);
    if dense {
        canopy_shade(fb, cx, cy, (w * 0.55) as i32, (w * 0.24) as i32, look);
    }
    // Half the conifers of a snow land are snow-laden (B46b), chosen per tree;
    // B76: and every conifer holds a load while real snow is lying.
    let snow_ly = snow.clamp(0.0, 1.0);
    let snowed = (look.biome == map::Biome::Tundra && (jitter * 7.0).fract() > 0.35)
        || snow_ly > 0.12;
    let mut trunk = (trunk as f32 * if big { 1.3 } else { 1.0 }) as i32;
    if dense {
        trunk = (((trunk as f32) * 0.55) as i32).max(3);
    }
    // Conifers keep their needles in winter: only a faint browning.
    let brown = season.bareness * 0.18;
    let tones = [
        palette::mix(look.needles[0], palette::rgb(0x62, 0x74, 0x3c), brown),
        palette::mix(look.needles[1], palette::rgb(0x4a, 0x58, 0x2e), brown),
        palette::mix(look.needles[2], palette::rgb(0x2c, 0x38, 0x20), brown),
    ];
    // The cedar is the darkest of the four; the pine the greyest.
    let tint = |c: palette::Color| -> palette::Color {
        match tone {
            1 => palette::mix(c, palette::rgb(0x6c, 0x7a, 0x58), 0.30),
            2 => palette::shade(c, 0.90),
            3 => palette::shade(c, 0.80),
            _ => c,
        }
    };
    let tones = [tint(tones[0]), tint(tones[1]), tint(tones[2])];
    fb.fill_rect(cx - 1, cy - trunk, 2, trunk + 1, look.trunk);
    let top = cy - trunk - hgt as i32;
    let hh = hgt as i32;
    let lean = ((jitter * 11.0).fract() - 0.5) * 2.0;
    for dy in 0..hh {
        let ty = dy as f32 / (hh - 1).max(1) as f32;
        // Sawtooth: each tier widens, then the next starts narrow again. A pine
        // holds its branches in a couple of clumps at the top instead.
        let tier = (ty * tiers as f32).fract();
        let (half, tier) = (
            w * 0.5 * (0.28 + 0.72 * ty) * (0.42 + 0.58 * tier),
            tier,
        );
        let off = (lean * ty) as i32;
        let x0 = cx - half as i32 + off;
        let x1 = cx + half as i32 + off;
        let yy = top + dy;
        let tone_by_depth = if ((dy as f32) / (3.0 * S as f32)).fract() < 0.34 {
            tones[0]
        } else {
            tones[1]
        };
        fb.fill_rect(x0, yy, (x1 - x0 + 1).max(1), 1, tone_by_depth);
        // Shaded right side, plus needle speckle on the lit left side.
        let dark_from = cx + (half * 0.22) as i32 + off;
        if x1 > dark_from {
            fb.fill_rect(dark_from, yy, (x1 - dark_from).max(1), 1, tones[2]);
        }
        for x in x0..dark_from {
            if tex::hash2(x, yy, 0xe1) > 0.80 {
                fb.put(x, yy, tones[0]);
            }
        }
        // The tier's outer tip: a single needle point.
        if tier < 0.12 {
            fb.put(x1, yy, tones[0]);
        }
        // B46b: in a boreal land half the spruces carry their snow. The load
        // sits on the top of each tier, so the tree keeps its dark underside and
        // still reads as a spruce.
        if snowed && tier < 0.12 + 0.34 * snow_ly.max(if look.biome == map::Biome::Tundra { 0.5 } else { 0.0 }) {
            let cap = palette::rgb(0xe4, 0xee, 0xf8);
            let edge = palette::shade(cap, 0.86);
            for x in x0..=x1 {
                if tex::hash2(x, yy, 0x5C) > 0.35 {
                    fb.put(x, yy, cap);
                }
            }
            fb.put(x0, yy, edge);
            fb.put(x1, yy, edge);
        }
        let _ = variant;
    }
}


/// A Joshua tree (D66/B46b): a rough, leaning yucca trunk with two or three
/// spiky arm heads. It is the desert's silhouette tree — almost bare, so it
/// reads against the sand at 1x.
fn draw_joshua(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    look: &Ground,
) {
    let big = size > 0;
    let h = (if big { 26 } else { 18 }) + (jitter * 8.0) as i32;
    // The trunk leans, and its rough fibre catches the light on one side only.
    let lean = ((variant as i32 % 3) - 1) * 2;
    let bark = palette::mix(look.trunk, palette::shade(look.leaf[1], 0.9), 0.25);
    let lit = palette::shade(bark, 1.25);
    let dark = palette::shade(bark, 0.72);
    for k in 0..h {
        let t = k as f32 / h as f32;
        let x = cx + (lean as f32 * t * t) as i32;
        fb.put(x, cy - k, lit);
        fb.put(x + 1, cy - k, bark);
        if k % 4 == 0 {
            fb.put(x - 1, cy - k, dark); // a bark shelf
        }
    }
    // Arm heads: a rosette of spiky fronds at the top and at one or two elbows.
    let heads = 2 + (variant % 2) as i32;
    let leaves = look.leaf[1];
    for a in 0..heads {
        let top = h - a * (h / 3) - 2;
        let x = cx + (lean as f32 * (top as f32 / h as f32).powi(2)) as i32 + if a == 0 { 0 } else { (a % 2) * 2 - 1 };
        let y = cy - top;
        // A ring of 8-10 spikes, each 3-5 px, drooping at the tips.
        let n = 8 + (variant % 3) as i32;
        for i in 0..n {
            let ang = std::f32::consts::PI * 0.15 + (i as f32 / n as f32) * std::f32::consts::PI * 0.7;
            let len = (3.0 + 2.5 * ((i as f32 * 2.3).sin() * 0.5 + 0.5)) as i32;
            let (dx, dy) = (ang.cos(), -ang.sin());
            for k in 1..=len {
                fb.put(
                    x + (dx * k as f32) as i32,
                    y + (dy * k as f32) as i32 + (k * k) / 8,
                    if k == len { palette::shade(leaves, 1.2) } else { leaves },
                );
            }
        }
    }
    // A little dead grass at the foot.
    for dx in -2..=2 {
        if tex::hash2(cx + dx, cy, 0xC7) > 0.5 {
            fb.put(cx + dx, cy, palette::shade(look.blade, 0.85));
        }
    }
}

/// A fan palm (B46b): short and stout with palmate fans, next to the coconut
/// palm's long feathered fronds in the same jungle.
fn draw_fan_palm(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    look: &Ground,
) {
    let big = size > 0;
    let h = (if big { 20 } else { 13 }) + (jitter * 5.0) as i32;
    let trunk = palette::mix(look.trunk, palette::shade(look.leaf[2], 0.8), 0.3);
    for k in 0..h {
        let x = cx + ((variant as i32 % 3) - 1) * (k / 6);
        fb.put(x, cy - k, palette::shade(trunk, 1.15));
        fb.put(x + 1, cy - k, palette::shade(trunk, 0.85));
    }
    // Fans: broad wedges of ribs, drooping round the crown.
    let top = (cx, cy - h);
    let fans = 5 + (variant % 3) as i32;
    for f in 0..fans {
        let a = std::f32::consts::PI * (0.12 + 0.76 * f as f32 / fans as f32);
        let len = (7 + (jitter * 3.0) as i32) as f32;
        for i in 0..4 {
            let spread = (i as f32 - 1.5) * 0.14;
            let (dx, dy) = ((a + spread).cos(), -(a + spread).sin());
            for k in 1..=len as i32 {
                let t = k as f32 / len;
                fb.put(
                    top.0 + (dx * k as f32) as i32,
                    top.1 + (dy * k as f32) as i32 + (t * t * 3.0) as i32,
                    palette::shade(look.leaf[0], 0.95 + 0.1 * (i == 1) as u8 as f32),
                );
            }
        }
        // The fan's webbing: a darker line down its middle.
        for k in 1..=len as i32 {
            let (dx, dy) = (a.cos(), -a.sin());
            fb.put(
                top.0 + (dx * k as f32) as i32,
                top.1 + (dy * k as f32) as i32 + ((k as f32 / len).powi(2) * 3.0) as i32,
                look.leaf[2],
            );
        }
    }
}

/// A flowering tropical tree (B46b): a broad crown gone over almost entirely to
/// blossom — the flamboyant / jacaranda that shows a whole street a colour.
fn draw_flowering(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    look: &Ground,
) {
    let big = size > 0;
    let trunk = (if big { 14 } else { 10 }) + (jitter * 3.0) as i32;
    let w = (if big { 22.0 } else { 16.0 }) * (0.9 + 0.2 * jitter);
    let h = w * 0.72;
    fb.fill_rect(cx - 1, cy - trunk, 3, trunk + 1, look.trunk);
    // The blossom colour follows the land's flower palette, so a jungle has a
    // red street and a savanna a yellow one.
    let hot = look.flowers[(variant as usize) % look.flowers.len()];
    let deep = palette::shade(hot, 0.74);
    let pale = palette::shade(hot, 1.18);
    let (ccx, ccy) = (cx as f32 + ((jitter * 5.0).fract() - 0.5) * 2.0, (cy - trunk) as f32 - h * 0.45);
    tex::ellipse_px(fb, ccx, ccy, w * 0.5, h * 0.5, |px, py, d| {
        let lit = (px as f32 - ccx) / (w * 0.5) + (py as f32 - ccy) / (h * 0.5);
        let patch = tex::hash2(px / 2, py / 2, 0xB7);
        let mut c = if lit < -0.35 {
            pale
        } else if d > 0.62 {
            deep
        } else {
            hot
        };
        if patch > 0.70 {
            c = palette::shade(c, 1.08);
        } else if patch < 0.24 {
            c = palette::shade(c, 0.92);
        }
        // A few leaves left in the crown, so it is a tree and not a balloon.
        if patch < 0.12 {
            c = palette::mix(c, look.leaf[1], 0.6);
        }
        c
    });
    // Fallen blossom on the ground.
    if big {
        for i in 0..5 {
            let a = jitter * 17.0 + i as f32 * 1.7;
            fb.put(cx + (a.cos() * 7.0) as i32, cy + (a.sin() * 3.0) as i32, hot);
        }
    }
}

/// A mangrove (B46b): a dark clump of leaves on stilt roots, standing at the
/// water's edge where the jungle meets the shallows.
fn draw_mangrove(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    look: &Ground,
) {
    let big = size > 0;
    let h = (if big { 16 } else { 11 }) + (jitter * 4.0) as i32;
    let wood = palette::shade(look.trunk, 0.8);
    // The stilt roots: three legs out of the water.
    for k in -2..=2 {
        let foot = cx + k * 2;
        let top = (cx, cy - h / 2);
        fb.line(
            (top.0 as f32, top.1 as f32),
            (foot as f32, (cy + 1) as f32),
            palette::shade(wood, 0.9 + 0.1 * (k & 1) as f32),
        );
    }
    // The crown: a low, wide, dark-green mass with a lit top.
    let w = (if big { 18.0 } else { 13.0 }) * (0.9 + 0.2 * jitter);
    let hh = w * 0.55;
    let cc = (cx as f32, (cy - h) as f32 + 1.0);
    tex::ellipse_px(fb, cc.0, cc.1, w * 0.5, hh * 0.5, |px, py, d| {
        let lit = (px as f32 - cc.0) / (w * 0.5) - (py as f32 - cc.1) / (hh * 0.5);
        let patch = tex::hash2(px / 2, py, 0xB9);
        if lit < -0.5 {
            palette::shade(look.leaf[0], 1.02)
        } else if d > 0.6 || lit > 0.4 {
            look.leaf[2]
        } else {
            palette::shade(look.leaf[1], 0.96 + 0.08 * patch)
        }
    });
}


/// B46e: a flat-topped acacia. A thin, leaning trunk that forks high, and a
/// wide umbrella of two stacked, open layers: the silhouette of the plain.
fn draw_acacia(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    look: &Ground,
) {
    let big = size > 0;
    let h = (if big { 24 } else { 17 }) + (jitter * 5.0) as i32;
    let lean = ((variant % 5) as i32 - 2) as f32 * 0.5;
    let wood = palette::shade(look.trunk, 0.85);
    let top = (cx as f32 + lean * 2.0, (cy - h) as f32);
    fb.line((cx as f32, cy as f32), (top.0, top.1 + 3.0), wood);
    fb.line((cx as f32 + 1.0, cy as f32), (top.0 + 1.0, top.1 + 3.0), palette::shade(wood, 0.8));
    // The fork: two limbs out under the canopy.
    let w = (if big { 38.0 } else { 28.0 }) * (0.9 + 0.2 * jitter);
    fb.line((top.0, top.1 + 3.0), (top.0 - w * 0.30, top.1 + 1.0), wood);
    fb.line((top.0, top.1 + 3.0), (top.0 + w * 0.30, top.1 + 1.0), wood);
    // Lower layer: wide and flat; upper layer: shorter, lit.
    for (k, (ww, hh, dy)) in [(1.0f32, 0.17f32, 1.0f32), (0.66, 0.15, -2.0)].iter().enumerate() {
        let (rx, ry) = (w * 0.5 * ww, (w * hh).max(2.0));
        let cc = (top.0 + (variant as f32 % 3.0 - 1.0) * 1.5 * k as f32, top.1 + dy);
        tex::ellipse_px(fb, cc.0, cc.1, rx, ry, |px, py, d| {
            // Gaps: an acacia's umbrella is open, the sky shows through it.
            let gap = tex::hash2(px / 2, py, 0xAC + k as u32);
            if d > 0.55 && gap > 0.80 {
                return 0;
            }
            let up = (cc.1 - py as f32) / ry;
            if up > 0.25 {
                palette::shade(look.leaf[0], 1.04)
            } else if up < -0.35 || d > 0.82 {
                look.leaf[2]
            } else {
                palette::shade(look.leaf[1], 0.94 + 0.1 * gap)
            }
        });
    }
}

/// B46e: a baobab. A fat, bottle-shaped grey trunk and a small, bare crown of
/// stubby limbs held up like roots: it reads at a glance from any other tree.
fn draw_baobab(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    look: &Ground,
) {
    let big = size > 0;
    let h = (if big { 26 } else { 19 }) + (jitter * 4.0) as i32;
    let half0 = if big { 6 } else { 5 };
    let body = palette::shade(look.trunk, 1.05);
    for k in 0..h {
        let t = k as f32 / h as f32;
        // Swollen low, a little waisted, wide again under the crown.
        let half = ((half0 as f32) * (1.0 - 0.34 * t.powf(0.8)) + 0.8 * (t * 9.0).sin().abs() * (1.0 - t)) as i32;
        let y = cy - k;
        for dx in -half..=half {
            let c = if dx < -half / 3 {
                palette::shade(body, 1.18)
            } else if dx > half / 2 {
                palette::shade(body, 0.68)
            } else {
                palette::shade(body, 0.96 + 0.08 * tex::hash2(dx, y / 3, 0xBA))
            };
            fb.put(cx + dx, y, c);
        }
    }
    // The crown: a handful of short limbs ending in small leaf tufts.
    let top = (cx as f32, (cy - h) as f32);
    let limbs = 5 + (variant % 3) as i32;
    for i in 0..limbs {
        let a = std::f32::consts::PI * (0.10 + 0.80 * i as f32 / (limbs - 1) as f32);
        let len = 7.0 + 5.0 * tex::hash2(i, variant as i32, 0xBB) + if big { 3.0 } else { 0.0 };
        let end = (top.0 + a.cos() * len * 1.2, top.1 - a.sin() * len * 0.8);
        fb.line((top.0, top.1 + 1.0), end, palette::shade(body, 0.82));
        for (ox, oy) in [(0, 0), (-2, 1), (2, 1), (0, -2), (-1, 2), (1, 2)] {
            let c = if oy < 0 { look.leaf[0] } else if ox == 0 { look.leaf[1] } else { look.leaf[2] };
            fb.put(end.0 as i32 + ox, end.1 as i32 + oy, c);
        }
    }
}

/// B46e: a bald cypress: a flared, fluted trunk standing in the water, a ragged
/// feathery crown, grey moss hanging from its limbs and a ring of knees.
fn draw_bald_cypress(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    look: &Ground,
) {
    let big = size > 0;
    let trunk_h = (if big { 14 } else { 10 }) + (jitter * 3.0) as i32;
    let wood = palette::shade(look.trunk, 0.9);
    // Knees round the foot.
    for (dx, hgt) in [(-6, 3), (-4, 2), (5, 3), (7, 2), (3, 1)] {
        if (dx + variant as i32) % 3 != 0 {
            fb.put(cx + dx, cy, palette::shade(wood, 0.75));
            for k in 1..=hgt {
                fb.put(cx + dx, cy - k, if k == hgt { palette::shade(wood, 1.15) } else { wood });
            }
        }
    }
    for k in 0..trunk_h {
        let half = if k < 4 { 3 - k / 2 } else { 1 };
        for dx in -half..=half {
            let c = if dx < 0 { palette::shade(wood, 1.15) } else if dx > 0 { palette::shade(wood, 0.72) } else { wood };
            fb.put(cx + dx, cy - k, c);
        }
    }
    // The crown: ragged tiers, wider than a spruce, soft at the edge.
    let hgt = (if big { 26.0 } else { 20.0 }) * (0.92 + 0.16 * jitter);
    let w = (if big { 24.0 } else { 18.0 }) * (0.9 + 0.2 * jitter);
    let top = cy - trunk_h + 3 - hgt as i32;
    for dy in 0..hgt as i32 {
        let ty = dy as f32 / hgt;
        let tier = (ty * 4.0).fract();
        let half = w * 0.5 * (0.22 + 0.78 * ty) * (0.55 + 0.45 * tier);
        let y = top + dy;
        let x0 = cx - half as i32;
        let x1 = cx + half as i32;
        for x in x0..=x1 {
            if tex::hash2(x, y, 0xBC) > 0.90 {
                continue;
            }
            let c = if x < cx - (half * 0.2) as i32 {
                look.leaf[0]
            } else if x > cx + (half * 0.25) as i32 {
                look.leaf[2]
            } else {
                look.leaf[1]
            };
            fb.put(x, y, c);
        }
        // Hanging moss under the tier's edge.
        if tier < 0.14 && dy > 5 {
            let moss = palette::mix(look.leaf[0], palette::rgb(0xc4, 0xcc, 0xb4), 0.55);
            for side in [-1, 1] {
                let mx = cx + side * (half as i32);
                let len = 3 + (tex::hash2(mx, y, 0xBD) * 5.0) as i32;
                for k in 0..len {
                    fb.put(mx, y + k, if k == len - 1 { palette::shade(moss, 0.8) } else { moss });
                }
            }
        }
    }
}

/// B46e: an olive: a short, knotted, forked trunk and a rounded, silvery-grey
/// crown of narrow leaves.
fn draw_olive(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    look: &Ground,
) {
    let big = size > 0;
    let h = (if big { 12 } else { 9 }) + (jitter * 3.0) as i32;
    let wood = palette::shade(look.trunk, 0.9);
    let top = (cx as f32 + ((variant % 3) as f32 - 1.0), (cy - h) as f32);
    fb.line((cx as f32, cy as f32), (top.0 - 1.0, top.1), wood);
    fb.line((cx as f32 + 1.0, cy as f32), (top.0 + 2.0, top.1 + 1.0), palette::shade(wood, 0.78));
    let w = (if big { 24.0 } else { 18.0 }) * (0.9 + 0.2 * jitter);
    let hh = w * 0.62;
    let cc = (top.0, top.1 - hh * 0.28);
    tex::ellipse_px(fb, cc.0, cc.1, w * 0.5, hh * 0.5, |px, py, d| {
        let lit = (px as f32 - cc.0) / (w * 0.5) - (py as f32 - cc.1) / (hh * 0.5);
        let n = tex::hash2(px, py, 0xBE);
        if d > 0.78 && n > 0.72 {
            return 0;
        }
        if lit < -0.45 {
            palette::shade(look.leaf[0], 1.04)
        } else if lit > 0.35 || d > 0.82 {
            look.leaf[2]
        } else if n > 0.8 {
            look.leaf[0]
        } else {
            look.leaf[1]
        }
    });
}

/// A saguaro (D66): a ribbed green column with one or two arms, lit from the
/// upper left. It has no crown, so it reads as a tree in a desert the way a
/// palm reads as one on a beach — a silhouette, not a canopy.
fn draw_cactus(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    variant: u8,
    jitter: f32,
    look: &Ground,
) {
    let big = size > 0;
    let h = (if big { 30 } else { 20 }) + (jitter * 8.0) as i32;
    // A saguaro is chunky: a ribbed column three pixels wide at 1x, four at the
    // big size, so it reads at wallpaper distance instead of as a green stick.
    let w = if big { 5 } else { 4 } + (variant % 2) as i32;
    let body = palette::shade(look.leaf[1], 1.02);
    let lit = palette::shade(body, 1.24);
    let dark = palette::shade(body, 0.72);
    let rib = |c: Color, k: i32| palette::shade(c, 0.93 + 0.07 * ((k & 1) as f32));
    // The column, ribbed.
    for k in 0..h {
        let x0 = cx - w / 2;
        for dx in 0..w {
            let c = if dx == 0 {
                lit
            } else if dx == w - 1 {
                dark
            } else {
                rib(body, k + dx)
            };
            fb.put(x0 + dx, cy - k, c);
        }
    }
    // The crown: a rounded, slightly lighter top with its own rib.
    fb.fill_rect(cx - w / 2 + 1, cy - h - 1, (w - 1).max(1), 1, lit);
    fb.fill_rect(cx - w / 2, cy - h, w, 1, rib(body, h));
    fb.put(cx, cy - h - 2, palette::shade(lit, 1.05));
    // Arms: one or two elbows out and up, two pixels thick.
    let arms = (variant % 3) as i32;
    for a in 0..arms {
        let start = h / 2 + a * (h / 5);
        let side = if a % 2 == 0 { 1 } else { -1 };
        let reach = 3 + (h / 7);
        let up = 5 + h / 5;
        let bx = if side > 0 { cx + w / 2 - 1 } else { cx - w / 2 };
        // The elbow along the arm's underside, then the vertical with a lit
        // left edge and a dark right one.
        for k in 1..=reach {
            fb.put(bx + side * k, cy - start, dark);
            fb.put(bx + side * k, cy - start - 1, body);
        }
        let ax = bx + side * reach;
        for k in 1..=up {
            let c = if k == up { lit } else { rib(body, k) };
            fb.put(ax, cy - start - k, c);
            fb.put(ax - side, cy - start - k, dark);
            if k % 4 == 0 {
                fb.put(ax + side, cy - start - k, palette::shade(dark, 0.92));
            }
        }
        fb.put(ax, cy - start - up - 1, lit);
        fb.put(ax - side / 2, cy - start - up - 1, palette::shade(lit, 1.06));
    }
    // Spines: a sparse lighter speckle down the ribs.
    for k in (0..h).step_by(2) {
        if tex::hash2(cx, cy - k, 0xC5) > 0.80 {
            fb.put(cx + w / 2 - 1, cy - k, palette::shade(lit, 1.1));
        }
    }
    // A small shadow at the foot so it stands on the sand.
    for dx in -w..=w {
        fb.put_blended(cx + dx, cy, palette::rgb(0x3a, 0x2a, 0x1e), 0.28);
    }
}

/// The park accents (D64): a palm, a cypress and a Japanese maple, which only
/// the city plants. They are evergreen, so they do not follow the season; the
/// maple is the red-leafed cultivar, red in spring and summer too.
fn draw_park_tree(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    size: u8,
    sp: TreeSpecies,
    variant: u8,
    jitter: f32,
    _season: &SeasonLook,
    look: &Ground,
) {
    let big = size > 0;
    match sp {
        TreeSpecies::Palm => {
            let h = (if big { 26 } else { 18 }) + (jitter * 6.0) as i32;
            let bend = ((variant % 3) as i32 - 1) * 2;
            for k in 0..h {
                let t = k as f32 / h as f32;
                let x = cx + (bend as f32 * t * t) as i32;
                fb.put(x, cy - k, palette::rgb(0x8a, 0x72, 0x4e));
                if k % 4 == 0 {
                    fb.put(x + 1, cy - k, palette::shade(palette::rgb(0x8a, 0x72, 0x4e), 0.82));
                }
            }
            let top_x = cx + bend;
            let top_y = cy - h;
            // Fronds: six arcs fanning out and drooping at the tips.
            let leaf = look.leaf[1];
            let leaf_lit = palette::shade(leaf, 1.18);
            for i in 0..6 {
                let a = std::f32::consts::PI + (i as f32 + 0.5) / 6.0 * std::f32::consts::PI;
                let len = (if big { 11.0 } else { 8.0 }) * (0.8 + 0.4 * jitter);
                let mut px = top_x as f32;
                let mut py = top_y as f32;
                for k in 0..len as i32 {
                    let t = k as f32 / len;
                    px += a.cos() * 1.2;
                    py += a.sin() * 1.2 + t * t * 1.4;
                    fb.put(px as i32, py as i32, if k % 3 == 0 { leaf_lit } else { leaf });
                }
            }
            fb.put(top_x, top_y - 1, leaf_lit);
        }
        TreeSpecies::Cypress => {
            let h = (if big { 44 } else { 30 }) + (jitter * 8.0) as i32;
            let wmax = if big { 9.0 } else { 6.0 };
            let col = palette::shade(look.needles[2], 0.92);
            for k in 0..h {
                let t = k as f32 / h as f32;
                // A tall teardrop: widest a third of the way up.
                let half = (wmax * (0.25 + 0.75 * (t * 1.6).min(1.0)) * (1.0 - (t * t).powi(3))) as i32;
                let yy = cy - k;
                for dx in -half..=half {
                    let edge = (dx.abs() as f32 / half.max(1) as f32).min(1.0);
                    let c = palette::shade(col, 1.0 - 0.35 * edge);
                    fb.put(cx + dx, yy, c);
                }
                if half == 0 {
                    fb.put(cx, yy, col);
                }
            }
        }
        _ => {
            // Japanese maple: a small, wide, deep-red crown on a short trunk.
            let trunk = if big { 12 } else { 9 };
            fb.fill_rect(cx - 1, cy - trunk, 2, trunk + 1, palette::rgb(0x5a, 0x40, 0x38));
            let rx = if big { 13.0 } else { 10.0 };
            let ry = if big { 10.0 } else { 8.0 };
            let ccx = cx as f32 + ((jitter * 9.0).fract() - 0.5) * 2.0;
            let ccy = (cy - trunk) as f32 - ry * 0.55;
            let red = palette::rgb(0xa8, 0x38, 0x38);
            let dark = palette::rgb(0x76, 0x24, 0x28);
            let lit = palette::rgb(0xd0, 0x5a, 0x44);
            tex::ellipse_px(fb, ccx, ccy, rx, ry, |px, py, d| {
                let patch = tex::hash2(px / 2, py / 2, 0x9A1);
                let mut c = if (px as f32 - ccx) / rx + (py as f32 - ccy) / ry < -0.1 {
                    lit
                } else if d > 0.55 {
                    dark
                } else {
                    red
                };
                if patch > 0.72 {
                    c = palette::shade(c, 1.08);
                } else if patch < 0.26 {
                    c = palette::shade(c, 0.9);
                }
                c
            });
        }
    }
}

/// One park accent tree, for the city's parks (B26c draws its own trees; D64
/// adds the exotics). `pick` chooses the species so a park is not all palms.
pub fn draw_park_accent(fb: &mut Framebuffer, cx: i32, cy: i32, pick: u8, variant: u8) {
    let sp = match pick % 3 {
        0 => TreeSpecies::Palm,
        1 => TreeSpecies::Cypress,
        _ => TreeSpecies::JapaneseMaple,
    };
    let season = SeasonLook::default();
    let look = crate::biome::look(map::Biome::Temperate);
    draw_park_tree(fb, cx, cy, 1, sp, variant, (variant as f32 * 0.31).fract(), &season, look);
}

fn draw_bush(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    v: u8,
    jitter: f32,
    snow: f32,
    look: &Ground,
) {
    let w = (5 + (v % 2) as i32) * S;
    let h = (3 + (jitter * 2.0) as i32) * S;
    // A clump of leaves: two tones with a hashed lattice.
    tex::ellipse_px(fb, cx as f32, (cy - h / 2) as f32, (w / 2) as f32, (h / 2) as f32, |px, py, _d| {
        if tex::hash2(px, py, 0xe5) > 0.62 {
            look.bush[0]
        } else {
            look.bush[1]
        }
    });
    fb.put(cx, cy - h, palette::shade(look.bush[0], 1.15));
    // B76 r2: a shrub under snow carries a cap, like a tree — the top of the
    // clump goes white and the underside keeps its green.
    if snow > 0.05 {
        let cap = palette::rgb(0xec, 0xf3, 0xfa);
        let shade = palette::rgb(0xcf, 0xdd, 0xea);
        let r = w / 2;
        for dy in 0..=(h / 2 + 1) {
            let t = dy as f32 / (h as f32 * 0.5 + 1.0);
            let half = (r as f32 * (1.0 - t * t).max(0.0).sqrt()) as i32;
            for dx in -half..=half {
                let hsh = tex::hash2(cx + dx, cy - h + dy, 0x5c0a);
                if hsh > 0.30 + 0.55 * (1.0 - snow) {
                    continue;
                }
                let c = if t < 0.35 { cap } else { shade };
                fb.put_blended(cx + dx, cy - h + dy, c, 0.55 + 0.4 * snow);
            }
        }
    }
}

fn draw_boulder(fb: &mut Framebuffer, cx: i32, cy: i32, v: u8, jitter: f32, look: &Ground) {
    let w = (5 + (v % 3) as i32) * S;
    let h = 3 * S;
    // A faceted rock: lit top-left, shadowed right, a crack.
    tex::ellipse_px(fb, cx as f32, (cy - h / 2) as f32, (w / 2) as f32, (h / 2) as f32, |px, py, d| {
        let lit = (px - cx) as f32 / (w as f32 * 0.5) + (py - cy) as f32 / (h as f32 * 0.5);
        let mut c = if lit < -0.2 { look.boulder[0] } else { look.boulder[1] };
        if d > 0.88 {
            c = palette::shade(c, 0.9);
        }
        c
    });
    fb.line(
        (cx as f32 - w as f32 * 0.2, cy as f32 - h as f32 * 0.6),
        (cx as f32 + w as f32 * 0.1, cy as f32 - 1.0),
        palette::shade(look.boulder[1], 0.8),
    );
    let _ = jitter;
}

/// Tall grass: a clump of blades, a few catching the light (D64).
fn draw_tall_grass(fb: &mut Framebuffer, cx: i32, cy: i32, v: u8, jitter: f32, look: &Ground) {
    // Three clumps spread over the tile: one prop per tile is one clump in the
    // model, but a meadow should not read as bare ground with tufts on a grid.
    let offsets = [(0, 0), (-11, -3), (10, 2)];
    for (ci, (ox, oy)) in offsets.iter().enumerate() {
        let n = 3 + (v as usize % 3) + ci;
        for i in 0..n {
            let a = jitter * 41.0 + i as f32 * 2.1 + ci as f32;
            let dx = ((a.cos() * 7.0) as i32).clamp(-7, 7);
            let len = 3 + ((a.sin() * 0.5 + 0.5) * 5.0) as i32 + (v % 2) as i32;
            let lean = if i % 2 == 0 { 1 } else { -1 };
            // Darker than the ground, so a clump reads as grass and not as
            // texture (B42).
            let base = palette::shade(look.grass[1], 0.82);
            for k in 0..len {
                let c = if k + 1 == len {
                    palette::shade(base, 1.55)
                } else {
                    base
                };
                fb.put(cx + ox + dx + (k / 3) * lean, cy + oy - k, c);
            }
        }
        if jitter > 0.45 + 0.15 * ci as f32 {
            // A seed head on the taller clumps.
            let dx = ((jitter * 23.0 + ci as f32 * 3.7).cos() * 6.0) as i32;
            fb.put(cx + ox + dx, cy + oy - 6, palette::rgb(0xc8, 0xb4, 0x6a));
        }
    }
}

/// Reeds and cattails on the water's edge (D64): tall thin stalks with a brown
/// seed head on some of them, standing in the shallows' colour.
fn draw_reeds(fb: &mut Framebuffer, cx: i32, cy: i32, v: u8, jitter: f32, look: &Ground) {
    let n = 4 + (v % 3) as i32;
    // Reeds keep their own green, tinted by the land (a desert oasis reed is
    // olive, a jungle one is vivid).
    let stem = palette::mix(palette::rgb(0x6e, 0x84, 0x3e), look.blade, 0.35);
    for i in 0..n {
        let a = jitter * 29.0 + i as f32 * 2.4;
        let dx = ((a.cos() * 7.0) as i32).clamp(-7, 7);
        let h = 6 + ((a.sin() * 0.5 + 0.5) * 8.0) as i32;
        let lean = if i % 3 == 0 { 1 } else { 0 };
        for k in 0..h {
            fb.put(cx + dx + (k / 4) * lean, cy - k, if k > h - 3 { palette::shade(stem, 1.12) } else { stem });
        }
        if i % 2 == 0 {
            // The cattail.
            fb.fill_rect(cx + dx + (h / 4) * lean, cy - h - 2, 1, 2, palette::rgb(0x6a, 0x44, 0x2a));
        }
    }
}

/// A fallen log lying across the grass, with an end-grain ring (D64).
fn draw_log(fb: &mut Framebuffer, cx: i32, cy: i32, v: u8, jitter: f32, look: &Ground) {
    let len = 10 + (v % 2) as i32 * 4 + (jitter * 3.0) as i32;
    let wood = palette::mix(look.trunk, palette::rgb(0x6a, 0x52, 0x3c), 0.5);
    let bark = palette::shade(wood, 0.78);
    let moss = palette::mix(look.bush[1], palette::rgb(0x4e, 0x72, 0x36), 0.5);
    let (dx, dy) = if v % 2 == 0 { (1, 0) } else { (1, 1) };
    for k in 0..len {
        let x = cx - len / 2 + k * dx;
        let y = cy - k * dy;
        fb.put(x, y, wood);
        fb.put(x, y + 1, bark);
        // Moss along the top of the trunk.
        if (k + v as i32) % 4 != 0 {
            fb.put(x, y - 1, moss);
        }
    }
    // The cut end: a pale ring.
    let ex = cx - len / 2;
    let ey = cy;
    fb.put(ex, ey, palette::rgb(0xb0, 0x96, 0x6e));
    fb.put(ex, ey + 1, palette::shade(wood, 0.9));
}

/// A scatter of small stones (D64).
fn draw_stones(fb: &mut Framebuffer, cx: i32, cy: i32, v: u8, jitter: f32, look: &Ground) {
    let n = 2 + (v % 3) as i32;
    for i in 0..n {
        let a = jitter * 33.0 + i as f32 * 2.9;
        let dx = ((a.cos() * 8.0) as i32).clamp(-8, 8);
        let dy = ((a.sin() * 4.0) as i32).clamp(-4, 4);
        let light = look.boulder[0];
        let shade = look.boulder[1];
        fb.put(cx + dx, cy + dy, light);
        fb.put(cx + dx + 1, cy + dy, shade);
        if i % 2 == 0 {
            fb.put(cx + dx, cy + dy + 1, shade);
        }
    }
}

fn draw_meadow(fb: &mut Framebuffer, cx: i32, cy: i32, v: u8, jitter: f32, look: &Ground) {
    // Flower clumps scattered inside the tile's diamond, plus a few blades.
    let n = (5 + (v % 4) as i32) * S;
    for i in 0..n {
        let a = jitter * 37.0 + i as f32 * 2.7;
        let r = 0.55 + 0.35 * ((a * 1.7).sin() * 0.5 + 0.5);
        let dx = ((a.cos() * 12.0 * r) as i32).clamp(-12, 12);
        let dy = ((a.sin() * 6.0 * r) as i32).clamp(-6, 6);
        let c = look.flowers[(i as usize + v as usize) % look.flowers.len()];
        fb.put(cx + dx, cy + dy, c);
        if i % 2 == 0 {
            fb.put(cx + dx, cy + dy - 1, palette::shade(c, 1.1));
            fb.put(cx + dx, cy + dy + 1, look.grass[1]);
        }
    }
}

/// Draw one tree of `species` standing at `(cx, cy)` (its ground point), for
/// the species sheets and the demos. Same art as the terrain uses.
pub fn draw_tree_sprite(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    species: TreeSpecies,
    size: u8,
    variant: u8,
    jitter: f32,
    season: &SeasonLook,
) {
    let look = crate::biome::look(map::Biome::Temperate);
    draw_tree_sprite_in(fb, cx, cy, species, size, variant, jitter, season, look);
}

/// The same tree, in a chosen land's colours (D66: the species sheets and the
/// deliverable shots need a desert oak to be an olive, not a meadow green).
pub fn draw_tree_sprite_in(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    species: TreeSpecies,
    size: u8,
    variant: u8,
    jitter: f32,
    season: &SeasonLook,
    look: &Ground,
) {
    if species == TreeSpecies::Cactus {
        draw_cactus(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Joshua {
        draw_joshua(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::FanPalm {
        draw_fan_palm(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Flamboyant {
        draw_flowering(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Mangrove {
        draw_mangrove(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Acacia {
        draw_acacia(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Baobab {
        draw_baobab(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::BaldCypress {
        draw_bald_cypress(fb, cx, cy, size, variant, jitter, look);
    } else if species == TreeSpecies::Olive {
        draw_olive(fb, cx, cy, size, variant, jitter, look);
    } else if species.is_park_accent() {
        draw_park_tree(fb, cx, cy, size, species, variant, jitter, season, look);
    } else if species.kind() == TreeKind::Conifer {
        draw_conifer(fb, cx, cy, size, variant, jitter, season, species, 0.0, false, look);
    } else {
        draw_broadleaf_season(fb, cx, cy, size, variant, jitter, season, species, 0.0, false, look);
    }
}

/// Draw one prop sprite of any kind at `(cx, cy)`, its ground point: the same
/// art the static layer paints, for the sheets and for the B74 pixel audit.
pub fn draw_prop_sprite(
    fb: &mut Framebuffer,
    cx: i32,
    cy: i32,
    prop: Prop,
    jitter: f32,
    season: &SeasonLook,
    look: &Ground,
) {
    match prop {
        Prop::None => {}
        Prop::Tree {
            species,
            size,
            variant,
        } => draw_tree_sprite_in(fb, cx, cy, species, size, variant, jitter, season, look),
        Prop::Bush(v) => draw_bush(fb, cx, cy, v, jitter, 0.0, look),
        Prop::Boulder(v) => draw_boulder(fb, cx, cy, v, jitter, look),
        Prop::Meadow(v) => draw_meadow(fb, cx, cy, v, jitter, look),
        Prop::TallGrass(v) => draw_tall_grass(fb, cx, cy, v, jitter, look),
        Prop::Reeds(v) => draw_reeds(fb, cx, cy, v, jitter, look),
        Prop::Log(v) => draw_log(fb, cx, cy, v, jitter, look),
        Prop::Stone(v) => draw_stones(fb, cx, cy, v, jitter, look),
    }
}

/// How far the pixels a sprite actually painted reach either side of its anchor
/// column: `(left, right)` in pixels. B74 measures a crown by drawing it, so the
/// check cannot drift from the art.
pub fn prop_sprite_extent(fb: &Framebuffer, anchor_x: i32) -> (i32, i32) {
    let (mut l, mut r) = (0i32, 0i32);
    let px = fb.px();
    for y in 0..fb.h {
        for x in 0..fb.w {
            let c = px[y * fb.w + x];
            // Transparent is 0; anything with colour in it was painted.
            if c == 0 || c >> 24 == 0 {
                continue;
            }
            let d = x as i32 - anchor_x;
            l = l.max(-d);
            r = r.max(d);
        }
    }
    (l.max(0), r.max(0))
}

/// How far a prop can stick up above its tile, in pixels (item bounds need it).
pub fn prop_rise(t: &Tile) -> i32 {
    match t.prop {
        Prop::Tree { size, species, .. } => match (species.kind(), size) {
            (TreeKind::Deciduous, 0) => 50,
            (TreeKind::Deciduous, _) => 68,
            // A park palm or cypress stands taller than a wood tree (D64).
            (TreeKind::Conifer, 0) => 52,
            (TreeKind::Conifer, _) => 68,
            // A saguaro or a Joshua tree: a column of arms, not a canopy (D66).
            (TreeKind::Succulent, 0) => 34,
            (TreeKind::Succulent, _) => 52,
        },
        Prop::Bush(_) => 12,
        Prop::Boulder(_) => 12,
        Prop::Meadow(_) => 8,
        Prop::TallGrass(_) => 12,
        Prop::Reeds(_) => 18,
        Prop::Log(_) => 10,
        Prop::Stone(_) => 6,
        Prop::None => 0,
    }
}

#[cfg(test)]
mod litter_tests {
    use super::*;
    const TRANSPARENT: crate::palette::Color = 0;

    /// B76 r3: the lawn litter is a drift **inside** the tile, not a column.
    ///
    /// It used to take its x range from `p[2].x - p[0].x`, and a tile diamond's
    /// top and bottom corners share an x — so every mark in the tile landed on
    /// one vertical line and a whole autumn forest read as columns of dashes.
    #[test]
    fn lawn_litter_spreads_inside_its_tile() {
        let vp = map::world_for_viewport(400, 300);
        let mut m = map::TileMap::fill(vp.w, vp.h, map::Tile::flat(2), 5);
        // A tree beside the tile under test, so its lawn keeps a drift.
        let (tx, ty) = (vp.w / 2, vp.h / 2);
        let t = m.at(tx, ty);
        m.set(tx, ty, map::Tile { kind: map::Kind::Grass, ..t });
        let n = m.at(tx + 1, ty);
        m.set(
            tx + 1,
            ty,
            map::Tile {
                kind: map::Kind::Grass,
                prop: map::Prop::Tree { species: map::TreeSpecies::Oak, size: 1, variant: 3 },
                ..n
            },
        );
        let season = SeasonLook { litter: 1.0, ..SeasonLook::default() };
        // Put the tile in the middle of the frame.
        let cam = crate::iso::Camera::new(200 - (tx - ty) * 32, 150 - (tx + ty) * 16);
        let p = cam.corners(tx, ty, [0.0; 4]);
        let mut fb = Framebuffer::new(400, 300);
        fb.clear(TRANSPARENT);
        draw_litter(&mut fb, &m, tx, ty, &p, &season);

        // The buffer started empty and only this tile's litter was drawn, so
        // every mark is a leaf.
        let mut xs: Vec<i32> = Vec::new();
        let mut ys: Vec<i32> = Vec::new();
        for y in 0..300 {
            for x in 0..400 {
                if fb.px()[y * 400 + x] != TRANSPARENT {
                    xs.push(x as i32);
                    ys.push(y as i32);
                }
            }
        }
        let _ = &season;
        assert!(xs.len() >= 6, "the tile kept no litter at all ({} marks)", xs.len());
        let spread_x = xs.iter().max().unwrap() - xs.iter().min().unwrap();
        let spread_y = ys.iter().max().unwrap() - ys.iter().min().unwrap();
        assert!(
            spread_x >= 20,
            "one tile's litter spans {spread_x} px across (a column, not a drift)"
        );
        assert!(spread_y >= 6, "one tile's litter is flat: {spread_y} px tall");
    }

    /// A lawn with no crown near it keeps no leaves: litter falls where trees are.
    #[test]
    fn open_lawn_keeps_no_leaves() {
        let vp = map::world_for_viewport(400, 300);
        let mut m = map::TileMap::fill(vp.w, vp.h, map::Tile::flat(2), 5);
        let (tx, ty) = (vp.w / 2, vp.h / 2);
        let season = SeasonLook { litter: 1.0, ..SeasonLook::default() };
        let cam = crate::iso::Camera::new(200 - (tx - ty) * 32, 150 - (tx + ty) * 16);
        let p = cam.corners(tx, ty, [0.0; 4]);
        let mut fb = Framebuffer::new(400, 300);
        fb.clear(TRANSPARENT);
        draw_litter(&mut fb, &m, tx, ty, &p, &season);
        let marks = (0..400 * 300)
            .filter(|&i| fb.px()[i] != TRANSPARENT)
            .count();
        assert_eq!(marks, 0, "a lawn with no tree near it kept {marks} leaf marks");
        // The map is untouched: this is only about what gets drawn.
        let _ = m.at(tx, ty);
    }
}

#[cfg(test)]
mod street_tests {
    use super::*;

    fn flat_map(w: i32, h: i32) -> TileMap {
        let mut m = map::generate_with(5, w, h, Some(map::Biome::Temperate)).expect("map");
        for y in 0..m.h {
            for x in 0..m.w {
                m.with_tile_mut(x, y, |t| {
                    t.kind = Kind::Grass;
                    t.corner = [4; 4];
                    t.prop = map::Prop::None;
                });
            }
        }
        m
    }

    /// B83: a street is a town street (pavement) only where the town stands.
    #[test]
    fn pavement_follows_the_plots() {
        let m = flat_map(21, 21);
        let mut plots = vec![0u16; (m.w * m.h) as usize];
        assert!(!built_up(&plots, &m, 10, 10));
        // A row of plots two tiles away, as a street with houses on it has.
        for x in 8..=12 {
            plots[(9 * m.w as i32 + x) as usize] = 7;
            plots[(11 * m.w as i32 + x) as usize] = 7;
        }
        assert!(built_up(&plots, &m, 10, 10));
        assert!(!plot_within(&plots, &m, 10, 15, 2));
        assert!(plot_within(&plots, &m, 10, 11, 2));
    }

    /// B83: a lane end with no drive draws a turning circle, so the street does
    /// not stop dead in the grass.
    #[test]
    fn a_cul_de_sac_gets_a_turning_circle() {
        let m = flat_map(21, 21);
        let cam = Camera::new(320, -180);
        let mut fb = Framebuffer::new(640, 280);
        draw_turnaround(&mut fb, &cam, &m, 10, 10);
        let n = fb.px().iter().filter(|&&p| p != palette::TRANSPARENT).count();
        assert!(n > 200, "the turning circle drew {n} pixels");
    }
}

#[cfg(test)]
mod water_depth_tests {
    use super::*;

    fn bake(depth: f32) -> WaterBake {
        let base = palette::rgb(0x1f, 0x9d, 0xc2);
        let shallow = palette::rgb(0x7f, 0xd2, 0xe2);
        WaterBake {
            depth: [depth; 4],
            band: [
                palette::mix(base, shallow, 0.9),
                palette::mix(base, shallow, 0.68),
                palette::mix(base, shallow, 0.3),
                base,
                palette::shade(base, 0.86),
            ],
            scale: 1.0,
            swell: 1.0,
            coast: None,
            still: false,
        }
    }

    #[test]
    fn shallow_water_near_sand_is_lighter_than_open_water() {
        let mean = |d: f32| {
            let wb = bake(d);
            let mut sum = 0u64;
            for py in 0..64 {
                for px in 0..128 {
                    sum += palette::luma(water_px(&wb, px, py, 0.5, 0.5)) as u64;
                }
            }
            sum as f32 / (64.0 * 128.0)
        };
        let (near, open) = (mean(0.8), mean(9.0));
        assert!(near > open + 8.0, "shallows {near} vs open {open}");
    }
}

#[cfg(test)]
mod coast_tests {
    use super::*;

    fn world(land: impl Fn(i32, i32) -> bool) -> map::TileMap {
        let mut m = map::TileMap::fill(60, 60, map::Tile::flat(1), 5);
        for y in 0..60 {
            for x in 0..60 {
                let t = m.at(x, y);
                let kind = if land(x, y) { Kind::Sand } else { Kind::Water };
                m.set(x, y, map::Tile { kind, ..t });
            }
        }
        m
    }

    /// The signed coast distance at a world position in tile units.
    fn s_at(m: &map::TileMap, fx: f32, fy: f32) -> f32 {
        let (x, y) = (fx.floor() as i32, fy.floor() as i32);
        match coast_of(m, x, y) {
            Some((c, _)) => coast_raw(&c, fx - x as f32, fy - y as f32),
            None => {
                if m.at(x, y).is_water() {
                    -1.0
                } else {
                    1.0
                }
            }
        }
    }

    /// A coast that runs along a tile diagonal is a staircase of tile edges in
    /// the map: every step is half a tile off the straight line. The smooth
    /// coast stays within 0.3 tile of that line, so no stretch of the
    /// drawn shore follows the 2:1 tile stair.
    #[test]
    fn a_diagonal_coast_is_a_line_not_a_staircase() {
        let m = world(|x, y| x >= y);
        let mut worst = 0.0f32;
        for k in 15..45 {
            let c = 2.0 * k as f32 + 0.5;
            // Walk across the coast on the line x + y = c.
            let (mut lo, mut hi) = (k as f32 - 1.6, k as f32 + 1.6);
            // Land is x >= y: the large x side.
            assert!(s_at(&m, hi, c - hi) > 0.0 && s_at(&m, lo, c - lo) < 0.0);
            for _ in 0..24 {
                let mid = 0.5 * (lo + hi);
                if s_at(&m, mid, c - mid) > 0.0 {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let cross = 0.5 * (lo + hi);
            worst = worst.max((cross - c * 0.5).abs());
        }
        assert!(worst < 0.3, "the coast strays {worst:.2} tile from the straight diagonal");
    }

    /// A one-tile islet keeps some land in the middle of its tile.
    #[test]
    fn a_one_tile_islet_keeps_its_beach() {
        let m = world(|x, y| x == 30 && y == 30);
        assert!(s_at(&m, 30.5, 30.5) > 0.1);
        assert!(s_at(&m, 28.5, 30.5) < 0.0);
    }

    /// Land and water far from the coast have no coast field at all.
    #[test]
    fn open_sea_has_no_coast() {
        let m = world(|x, _| x >= 45);
        assert!(coast_of(&m, 10, 30).is_none());
        assert!(coast_of(&m, 50, 30).is_none());
        assert!(coast_of(&m, 44, 30).is_some());
    }
}
