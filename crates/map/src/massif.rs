//! Massifs: the big landforms the generator grows (B46d).
//!
//! The renderer draws each of these as one composite sprite on a flattened
//! footprint, so the sim must not build or farm under it either. Detection reads
//! only the height field, so it gives the same answer before and after
//! [`mark_footprints`] changes the ground kinds.

use crate::TileMap;

/// The biggest foot a sprite carries, in tiles. The generator grows massifs far
/// larger; the renderer flattens the rest into a skirt.
pub const MAX_RADIUS: f32 = 6.0;

/// A massif: where it stands and how big it is (tile space, levels).
#[derive(Clone, Copy, Debug)]
pub struct Massif {
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

/// Decide the world's landforms: detect the massifs, mark each chosen footprint
/// `Kind::Rock` (nothing is built or planted under a landform) and remember the
/// sites and the unused hills on the map.
pub fn plan(map: &mut TileMap) {
    let (sites, others) = detect_all(map);
    mark_footprints(map, &sites);
    map.landforms = sites;
    map.hills = others;
}

fn mark_footprints(map: &mut TileMap, sites: &[Massif]) {
    for m in sites {
        let r = m.radius + 0.5;
        for y in ((m.cy - r).floor() as i32).max(0)..=((m.cy + r).ceil() as i32).min(map.h - 1) {
            for x in ((m.cx - r).floor() as i32).max(0)..=((m.cx + r).ceil() as i32).min(map.w - 1) {
                let (dx, dy) = (x as f32 + 0.5 - m.cx, y as f32 + 0.5 - m.cy);
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let mut t = map.at(x, y);
                if t.kind.is_water() {
                    continue;
                }
                t.kind = crate::Kind::Rock;
                t.prop = crate::Prop::None;
                map.set(x, y, t);
            }
        }
    }
}

/// How far from a landform's centre the drawing map is reshaped (the footprint
/// and its skirt): anything built inside it keeps the massif off the map.
pub fn reach(m: &Massif) -> f32 {
    m.radius * 2.4 + 4.0
}

/// A save is loaded on a map generated with landforms. Where the city already
/// stands within a landform's reach (`occupied` says so for a tile) the massif
/// is given up: its footprint gets back the ground the world had before landforms
/// (`original`, from [`crate::try_generate_unmarked`]) except on `keep` tiles
/// (the save's own overrides), and it joins the unused hills.
///
/// `whole_reach` says how far "within reach" goes. A save from before landforms
/// existed (`true`) gives the massif up when the city stands anywhere in its
/// skirt. A save written with landforms (`false`) has only ever built outside
/// the footprint, beside the skirt: the live city keeps its massif there, so the
/// reload must too, and only ground *inside the footprint* gives it up.
pub fn release_occupied(
    map: &mut TileMap,
    original: &TileMap,
    occupied: &dyn Fn(i32, i32) -> bool,
    keep: &dyn Fn(i32, i32) -> bool,
    whole_reach: bool,
) {
    let sites = std::mem::take(&mut map.landforms);
    for m in sites {
        let r = if whole_reach { reach(&m) } else { m.radius + 0.5 };
        let mut hit = false;
        let (x0, x1) = (((m.cx - r).floor() as i32).max(0), ((m.cx + r).ceil() as i32).min(map.w - 1));
        let (y0, y1) = (((m.cy - r).floor() as i32).max(0), ((m.cy + r).ceil() as i32).min(map.h - 1));
        'scan: for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 + 0.5 - m.cx, y as f32 + 0.5 - m.cy);
                if dx * dx + dy * dy <= r * r && occupied(x, y) {
                    hit = true;
                    break 'scan;
                }
            }
        }
        if !hit {
            map.landforms.push(m);
            continue;
        }
        let fr = m.radius + 0.5;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 + 0.5 - m.cx, y as f32 + 0.5 - m.cy);
                if dx * dx + dy * dy <= fr * fr && !keep(x, y) {
                    map.set(x, y, original.at(x, y));
                }
            }
        }
        map.hills.push(m);
    }
}

/// Read the generator's massifs back out of the height field: each summit gets
/// a foot radius (24 rays out to where the land is back at its plain), a base
/// level and a height. Small hills are left as terrain.
pub fn detect(map: &TileMap) -> Vec<Massif> {
    detect_all(map).0
}

/// The chosen landform sites and the other massifs (no sprite: the renderer
/// smooths those so no terraced plate is left).
///
/// Placement follows the landscape (D73): landforms stand in the back half of
/// the map (low `x + y`, the top of the screen) so they rise behind the city,
/// in a small group rather than scattered, each a different size. A temperate
/// map gets at most two.
pub fn detect_all(map: &TileMap) -> (Vec<Massif>, Vec<Massif>) {
    let (w, h) = (map.w, map.h);
    // Only the full-size worlds (the sim's 100+ tile maps) carry landforms: a
    // small test map has no room for a 6-tile foot beside its village.
    if w < 64 || h < 64 {
        return (Vec::new(), Vec::new());
    }
    let mut levels: Vec<u8> = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            levels.push(map.at(x, y).max_level());
        }
    }
    levels.sort_unstable();
    let plain = levels[levels.len() / 2] as f32;
    let peak_level = *levels.last().unwrap() as f32;
    if peak_level - plain < 5.0 {
        return (Vec::new(), Vec::new());
    }
    let mut cand: Vec<(u8, i32, i32)> = Vec::new();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let l = map.at(x, y).max_level();
            if l as f32 >= plain + 0.62 * (peak_level - plain) {
                let mut top = true;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if (dx != 0 || dy != 0) && map.at(x + dx, y + dy).max_level() > l {
                            top = false;
                        }
                    }
                }
                if top {
                    cand.push((l, x, y));
                }
            }
        }
    }
    // The backmost first (low x + y: the top of the screen, behind the city),
    // the tallest of equals.
    cand.sort_by(|a, b| {
        ((a.1 + a.2) / 6)
            .cmp(&((b.1 + b.2) / 6))
            .then(b.0.cmp(&a.0))
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
    });
    // Every massif, measured (the radius is the uncapped foot here).
    let mut meas: Vec<Massif> = Vec::new();
    for (level, sx, sy) in cand {
        if meas.len() >= 16 {
            break;
        }
        let (cx, cy) = (sx as f32 + 0.5, sy as f32 + 0.5);
        if meas.iter().any(|m| ((cx - m.cx).powi(2) + (cy - m.cy).powi(2)).sqrt() < m.radius) {
            continue;
        }
        let mut radii = Vec::with_capacity(24);
        for i in 0..24 {
            let a = i as f32 / 24.0 * std::f32::consts::TAU;
            let (dx, dy) = (a.cos(), a.sin());
            let mut d = 0.0f32;
            while d < 24.0 {
                let (px, py) = (cx + dx * d, cy + dy * d);
                if px < 0.0 || py < 0.0 || px >= w as f32 || py >= h as f32 {
                    break;
                }
                if map.at(px as i32, py as i32).max_level() as f32 <= plain + 1.5 {
                    break;
                }
                d += 0.5;
            }
            radii.push(d);
        }
        radii.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let foot = radii[radii.len() / 2];
        if foot < 2.5 {
            continue;
        }
        let r = foot.min(if map.biome == crate::Biome::Alpine { 8.5 } else if map.biome == crate::Biome::Highlands { 4.6 } else { MAX_RADIUS });
        let mut bnd = Vec::with_capacity(32);
        for i in 0..32 {
            let a = i as f32 / 32.0 * std::f32::consts::TAU;
            for k in [1.0f32, 2.0] {
                let px = (cx + a.cos() * (r + k)) as i32;
                let py = (cy + a.sin() * (r + k)) as i32;
                if px >= 0 && py >= 0 && px < w && py < h {
                    bnd.push(map.at(px, py).max_level());
                }
            }
        }
        bnd.sort_unstable();
        let ring = bnd.get(bnd.len() / 2).copied().unwrap_or(plain as u8) as f32;
        let base = ring.min(plain + 2.0);
        let peak = ((level as f32 - base).max(0.0) * 1.5).clamp(7.0, 19.0);
        let mut solid = 0;
        let ri = r.ceil() as i32;
        for dy in -ri..=ri {
            for dx in -ri..=ri {
                if (dx * dx + dy * dy) as f32 > r * r {
                    continue;
                }
                let (x, y) = (sx + dx, sy + dy);
                if x < 0 || y < 0 || x >= w || y >= h {
                    continue;
                }
                if map.at(x, y).max_level() as f32 > plain + 2.0 {
                    solid += 1;
                }
            }
        }
        if solid < 40 {
            continue;
        }
        let seed = map.seed ^ ((solid as u64) << 8) ^ ((cx as u64) << 3) ^ cy as u64;
        meas.push(Massif { cx, cy, radius: foot.min(16.0), base, peak, seed });
    }

    // B46e: an alpine valley has a chain of peaks down each wall.
    let alpine = map.biome == crate::Biome::Alpine;
    let max_sites = if alpine {
        6
    } else if map.biome == crate::Biome::Temperate {
        2
    } else {
        3
    };
    let sizes = if alpine { [1.0f32, 0.92, 0.86, 0.8, 0.76, 0.72] } else { [1.0f32, 0.78, 0.6, 0.6, 0.6, 0.6] };
    let mut sites: Vec<Massif> = Vec::new();
    let mut others: Vec<Massif> = Vec::new();
    for m in meas {
        let k = sites.len();
        if k >= max_sites {
            others.push(m);
            continue;
        }
        // A group: the later ones stand beside the first, not across the map.
        if let Some(f) = sites.first() {
            if !alpine && ((m.cx - f.cx).powi(2) + (m.cy - f.cy).powi(2)).sqrt() > 3.4 * MAX_RADIUS {
                others.push(m);
                continue;
            }
        }
        let cap = if alpine { 8.5 } else if map.biome == crate::Biome::Highlands { 4.6 } else { MAX_RADIUS };
        let mut radius = m.radius.min(cap * sizes[k]);
        // Touching, not stacked: shrink until it clears the ones already placed.
        for o in &sites {
            let d = ((m.cx - o.cx).powi(2) + (m.cy - o.cy).powi(2)).sqrt();
            radius = radius.min(d / 0.95 - o.radius);
        }
        // The village's ground (the central frame) stays free.
        while radius >= 3.0 && !clear_of_village(map, m.cx, m.cy, radius + 1.5) {
            radius -= 0.5;
        }
        if radius < 3.0 {
            others.push(m);
            continue;
        }
        sites.push(Massif { radius, ..m });
    }
    (sites, others)
}

/// Does a disc of tiles keep out of the central frame (the village's ground)?
fn clear_of_village(map: &TileMap, cx: f32, cy: f32, r: f32) -> bool {
    let ri = r.ceil() as i32;
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            if ((dx * dx + dy * dy) as f32) > r * r {
                continue;
            }
            let (x, y) = (cx as i32 + dx, cy as i32 + dy);
            if crate::terrain_gen::in_central_frame(x, y, map.w, map.h) {
                return false;
            }
        }
    }
    true
}
