//! B70: smooth open ground.
//!
//! The terrain is a grid of integer levels, so open ground reads as flat plates
//! joined by one-tile ramps with hard iso-aligned edges. This module computes a
//! *drawing-only* layer on the map (`TileMap::soft`):
//!
//! * a vertical offset (in pixels, at most half a level) for every corner of
//!   open natural ground, found by relaxing the height field toward its
//!   neighbours, so a plate edge becomes a gentle slope spread over several
//!   tiles. Anything built, stepped, wet, rocky or under a lot keeps its
//!   integer height (roads, kerbs, lots and shores stay exactly where the sim
//!   put them), and a vertex shared with such a tile is pinned;
//! * a light value per corner, from the normals of the smoothed surface, that
//!   the tile painter interpolates across the tile and dithers. No tile edge
//!   is visible in the shading because neighbouring tiles share the corner.
//!
//! The sim never reads it; it is recomputed from the map and the lots.

use map::{Kind, MAX_LEVEL, Tile, TileMap};

/// The most a corner may move from its integer level, in pixels (half a level).
const MAX_PX: f32 = 6.0;
/// Relaxation sweeps: how far a plate edge spreads (about this many tiles).
const SWEEPS: usize = 12;
/// How much steeper the shading sees the slopes than the geometry is.
pub const RELIEF_GAIN: f32 = 6.0;

/// Corner `i` of tile `(x, y)` sits on vertex `(x + CX[i], y + CY[i])`.
const CX: [i32; 4] = [0, 1, 1, 0];
const CY: [i32; 4] = [0, 0, 1, 1];

/// Open natural ground that may be smoothed.
#[inline]
pub fn soft_kind(k: Kind) -> bool {
    matches!(
        k,
        Kind::Grass
            | Kind::Meadow
            | Kind::Upland
            | Kind::DryGrass
            | Kind::Sand
            | Kind::Scree
            | Kind::Rock
    )
}

#[inline]
fn eligible(t: &Tile) -> bool {
    // The raw spread, not `Tile::relief` (which calls every rock tile steep).
    soft_kind(t.kind) && t.max_level() - t.min_level() < 3
}

/// Recompute `map.soft` from the tiles and the lots (`occupied(x, y)` is true
/// under a building or a lot). Returns the tiles whose look changed.
pub fn compute(map: &mut TileMap, occupied: &dyn Fn(i32, i32) -> bool) -> Vec<(i32, i32)> {
    let (w, h) = (map.w, map.h);
    let n = (w * h) as usize;
    let vw = w + 1;
    let vh = h + 1;
    let vi = |vx: i32, vy: i32| (vy * vw + vx) as usize;

    let elig: Vec<bool> = (0..n)
        .map(|i| {
            let (x, y) = (i as i32 % w, i as i32 / w);
            eligible(&map.tiles[i]) && !occupied(x, y)
        })
        .collect();

    // Integer height and pin state of every vertex.
    let mut base = vec![0.0f32; (vw * vh) as usize];
    let mut pinned = vec![true; (vw * vh) as usize];
    for vy in 0..vh {
        for vx in 0..vw {
            let mut sum = 0.0;
            let mut cnt = 0;
            let mut lo = u8::MAX;
            let mut hi = 0u8;
            let mut all = true;
            for i in 0..4 {
                let (tx, ty) = (vx - CX[i], vy - CY[i]);
                if tx < 0 || ty < 0 || tx >= w || ty >= h {
                    all = false;
                    continue;
                }
                let k = (ty * w + tx) as usize;
                let c = map.tiles[k].corner[i];
                sum += c as f32;
                cnt += 1;
                lo = lo.min(c);
                hi = hi.max(c);
                if !elig[k] {
                    all = false;
                }
            }
            let v = vi(vx, vy);
            base[v] = if cnt > 0 { sum / cnt as f32 } else { 0.0 };
            pinned[v] = !(all && cnt == 4 && lo == hi);
        }
    }

    // Relax the free vertices toward the mean of their four neighbours.
    let mut s = base.clone();
    let mut next = base.clone();
    let lim = MAX_PX / crate::iso::HEIGHT_UNIT as f32;
    for _ in 0..SWEEPS {
        for vy in 1..vh - 1 {
            for vx in 1..vw - 1 {
                let v = vi(vx, vy);
                if pinned[v] {
                    continue;
                }
                let m = (s[vi(vx - 1, vy)] + s[vi(vx + 1, vy)] + s[vi(vx, vy - 1)] + s[vi(vx, vy + 1)])
                    * 0.25;
                next[v] = m.clamp(base[v] - lim, base[v] + lim);
            }
        }
        std::mem::swap(&mut s, &mut next);
    }

    // Per-corner offsets.
    let mut dz = vec![[0i8; 4]; n];
    for y in 0..h {
        for x in 0..w {
            let k = (y * w + x) as usize;
            if !elig[k] {
                continue;
            }
            for i in 0..4 {
                let v = vi(x + CX[i], y + CY[i]);
                if pinned[v] {
                    continue;
                }
                let px = ((s[v] - base[v]) * crate::iso::HEIGHT_UNIT as f32).round();
                dz[k][i] = px.clamp(-MAX_PX, MAX_PX) as i8;
            }
        }
    }

    // Face light of every eligible tile from its smoothed corners.
    let l = crate::terrain::light_vector();
    let unit = crate::iso::HEIGHT_UNIT as f32;
    let mut face = vec![f32::NAN; n];
    for k in 0..n {
        if !elig[k] {
            continue;
        }
        let t = &map.tiles[k];
        let z = |i: usize| t.corner[i] as f32 + dz[k][i] as f32 / unit;
        let gx = ((z(1) + z(2)) - (z(0) + z(3))) * 0.5;
        let gy = ((z(3) + z(2)) - (z(0) + z(1))) * 0.5;
        // One level is half a tile of height (see `terrain::triangle_light`).
        let nx = -0.5 * gx * RELIEF_GAIN;
        let ny = -0.5 * gy * RELIEF_GAIN;
        let len = (nx * nx + ny * ny + 1.0).sqrt();
        face[k] = (nx * l[0] + ny * l[1] + l[2]) / len;
    }

    // The patch noise is a function of the seed: compute it once per map.
    if map.soft.jit.len() != n {
        let jit: Vec<f32> = (0..n)
            .map(|k| {
                let (x, y) = (k as i32 % w, k as i32 / w);
                (map.patch_noise(x, y) - 0.5) * 0.16 + (map.tile_hash(x, y) - 0.5) * 0.05
            })
            .collect();
        map.soft.jit = jit;
    }
    // Light per corner: the mean of the faces around the vertex, plus the same
    // height tint and patch noise the flat path used, taken per vertex so they
    // too are continuous.
    let mut lit = vec![[f32::NAN; 4]; n];
    for y in 0..h {
        for x in 0..w {
            let k = (y * w + x) as usize;
            if !elig[k] {
                continue;
            }
            for i in 0..4 {
                let (vx, vy) = (x + CX[i], y + CY[i]);
                let mut sum = 0.0;
                let mut cnt = 0.0;
                let mut jit = 0.0;
                let mut jn = 0.0;
                for j in 0..4 {
                    let (tx, ty) = (vx - CX[j], vy - CY[j]);
                    if tx < 0 || ty < 0 || tx >= w || ty >= h {
                        continue;
                    }
                    let kk = (ty * w + tx) as usize;
                    if face[kk].is_finite() {
                        sum += face[kk];
                        cnt += 1.0;
                    }
                    jit += map.soft.jit[kk];
                    jn += 1.0;
                }
                let tint = (s[vi(vx, vy)] / MAX_LEVEL as f32 - 0.5) * 0.10;
                lit[k][i] = sum / cnt + tint + if jn > 0.0 { jit / jn } else { 0.0 };
            }
        }
    }

    let mut changed = Vec::new();
    if map.soft.dz.len() == n {
        for k in 0..n {
            let same_lit = (0..4).all(|i| {
                let (a, b) = (map.soft.lit[k][i], lit[k][i]);
                (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-3
            });
            if map.soft.dz[k] != dz[k] || !same_lit {
                changed.push((k as i32 % w, k as i32 / w));
            }
        }
    } else {
        changed.extend((0..n).map(|k| (k as i32 % w, k as i32 / w)));
    }
    map.soft.dz = dz;
    map.soft.lit = lit;
    changed
}

#[cfg(test)]
mod tests;
