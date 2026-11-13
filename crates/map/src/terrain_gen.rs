//! Seeded terrain generation: rolling low ground, real hills, one river, maybe a
//! lake, a few bluffs.
//!
//! Pipeline (all deterministic from the seed):
//! 1. elevation on the **vertex grid** = gentle fbm + 1-2 hill masses placed
//!    toward a screen edge (the frame's diamond, expressed in `p = x - y`,
//!    `q = x + y` coordinates), with ridged detail for spurs;
//! 2. quantise to levels 0..=MAX_LEVEL and clamp neighbouring *vertices* to one
//!    level apart (smooth hillsides);
//! 3. trace one river through the low ground from edge to edge and carve it;
//! 4. maybe a lake in a low basin;
//! 5. per-tile corners from the vertex grid, then the exceptions: water is flat,
//!    soft banks slope into the water, bluffs stay high (that is where the
//!    cliff faces come from);
//! 6. materials from level + slope + water adjacency;
//! 7. vegetation: broadleaf woods low down, conifers higher, bushes, boulders,
//!    meadow patches.

use std::f32::consts::TAU;

use crate::MAX_LEVEL;
use crate::biome::Biome;
use crate::rng::{Rng, fbm, hash2};
use crate::tiles::{Lake, Ocean, River, TileMap, WaterKind};
use crate::types::{Dir, Edge, Kind, Prop, Tile, TreeSpecies, WaterLayout};

const EDGES: [Edge; 4] = Edge::ALL;

/// The smallest side a world can be generated at. Below this the terrain
/// passes (rivers, lakes, the hill guard) have no room to work and the tile
/// grid no longer holds the founding diamond.
pub const MIN_SIDE: i32 = 16;

/// Why a world could not be generated (B61 item 4: a too-small frame is a
/// caller error, not a panic — a 7x5 framebuffer used to abort the process).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    /// The requested world is smaller than [`MIN_SIDE`] on a side.
    TooSmall { w: i32, h: i32 },
    /// A side was not positive (a zero or negative frame size).
    BadSize { w: i32, h: i32 },
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapError::TooSmall { w, h } => write!(
                f,
                "map {w}x{h} is too small: a world needs at least {MIN_SIDE} tiles a side"
            ),
            MapError::BadSize { w, h } => write!(f, "map size {w}x{h} is not a positive size"),
        }
    }
}

impl std::error::Error for MapError {}

/// Generate the world (empty land: D20 — no roads, no buildings). Every caller
/// of this one passes a real viewport; a bad size is still a panic here, so
/// the fallible entry point stays [`try_generate_with`].
pub fn generate(seed: u64, w: i32, h: i32) -> TileMap {
    try_generate_with(seed, w, h, None).expect("world size")
}

/// Generate the world for a chosen land (D66), or say why it cannot be done.
/// `biome` overrides the land the seed would roll (`--biome desert` for a spike
/// or a wallpaper test, or a loaded save); a stub land falls back to temperate.
pub fn try_generate_with(seed: u64, w: i32, h: i32, biome: Option<Biome>) -> Result<TileMap, MapError> {
    if w <= 0 || h <= 0 {
        return Err(MapError::BadSize { w, h });
    }
    if w < MIN_SIDE || h < MIN_SIDE {
        return Err(MapError::TooSmall { w, h });
    }
    Ok(generate_checked(seed, w, h, biome, true))
}

/// The world as it was before landforms: no footprint is marked and no massif is
/// planned. A loaded save uses it to give a massif back its own ground when a
/// city already stands there.
pub fn try_generate_unmarked(seed: u64, w: i32, h: i32, biome: Option<Biome>) -> Result<TileMap, MapError> {
    if w < MIN_SIDE || h < MIN_SIDE {
        return Err(MapError::TooSmall { w, h });
    }
    Ok(generate_checked(seed, w, h, biome, false))
}

/// `generate_with` as callers have always used it, except that a world too small
/// to generate is an `Err` instead of an assert (B61 item 4).
pub fn generate_with(
    seed: u64,
    w: i32,
    h: i32,
    biome: Option<Biome>,
) -> Result<TileMap, MapError> {
    try_generate_with(seed, w, h, biome)
}

/// The generator proper; `try_generate_with` has already checked the size.
fn generate_checked(seed: u64, w: i32, h: i32, biome: Option<Biome>, mark_landforms: bool) -> TileMap {
    let biome = match biome {
        Some(b) if b.is_live() => b,
        _ => Biome::pick(seed),
    };
    let neighbour = biome.neighbour(seed);
    let blend_edge = Biome::blend_edge(seed);
    let mut rng = Rng::new(seed ^ 0x4261_7365_0000_0001);
    let vw = w + 1;
    let vh = h + 1;

    // --- 1. elevation on the vertex grid ---------------------------------
    let (mut verts, hills) = elevation(&mut rng, seed, w, h, biome);

    // --- 2. smoothing -----------------------------------------------------
    clamp_vertices(&mut verts, vw, vh, 2);
    despeckle(&mut verts, vw, vh, 2);

    // Every seed has to read as hilly (D29): if the frame's diamond has too
    // little high ground, lift the hill masses and settle again.
    let mut guard = 0;
    while (visible_high_fraction(&verts, w, h, vw) < 0.17
        || max_visible_level(&verts, w, h, vw) < 14)
        && guard < 8
    {
        guard += 1;
        for &(cx, cy, radius, _, _) in &hills {
            let r = radius.ceil() as i32 + 1;
            for y in (cy as i32 - r).max(0)..=(cy as i32 + r).min(h) {
                for x in (cx as i32 - r).max(0)..=(cx as i32 + r).min(w) {
                    let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                    if d <= radius {
                        let j = idx(vw, x, y);
                        verts[j] = (verts[j] + 1.0).min(MAX_LEVEL as f32);
                    }
                }
            }
        }
        clamp_vertices(&mut verts, vw, vh, 2);
        despeckle(&mut verts, vw, vh, 1);
    }

    // B13 item 5: outside real mountain ground (level >= 10) the surface must
    // never step more than one level edge to edge.
    clamp_low_ground(&mut verts, vw, vh, 6);

    // D66: the land's own shape. A desert is dunes and flat-topped mesas, a
    // tundra is higher and sharper, a jungle is low and rolling.
    shape_for_biome(&mut verts, vw, vh, biome, seed);
    clamp_low_ground(&mut verts, vw, vh, 6);

    // --- 3. which water the seed asked for (D44) ---------------------------
    let mut layout = pick_layout(&mut rng);
    // B46e: an island land is a sea land.
    let watery = matches!(biome, Biome::Islands | Biome::Lagoon);
    if watery && !layout.is_coastal() && w.min(h) >= 48 {
        layout = WaterLayout::Coast;
    }
    let mut surface_of = vec![0u8; (w * h) as usize];
    let mut class_of = vec![0u8; (w * h) as usize];

    // --- 4. the sea (coastal layouts), before the river so the river can run
    //        into it ------------------------------------------------------
    let mut beach_of = vec![false; (w * h) as usize];
    let mut ocean: Option<Ocean> = if layout.is_coastal() {
        Some(carve_ocean(
            &mut rng,
            &mut verts,
            w,
            h,
            vw,
            &mut surface_of,
            &mut class_of,
            &mut beach_of,
            if watery && w.min(h) >= 48 { SEA_WIDE } else { SEA_DEFAULT },
        ))
    } else {
        None
    };
    // B46e: the land's own water, cut after the sea so the river can still
    // reach it. Islands: a second sea on the far side, straits through the
    // land and islets out at sea; lagoon: a sand barrier standing off the
    // shore; swamp: channels and pools through the low ground.
    let mut reef = 0.0f32;
    match (biome, ocean.as_ref()) {
        (Biome::Islands, Some(o)) => carve_archipelago(
            &mut rng, &mut verts, w, h, vw, &mut surface_of, &mut class_of, &mut beach_of, o, seed,
        ),
        (Biome::Lagoon, Some(o)) => {
            reef = carve_barrier(&mut rng, &mut verts, w, h, vw, &mut surface_of, &mut class_of, o, seed)
        }
        (Biome::Swamp, _) => carve_wetland(&mut verts, w, h, vw, &mut surface_of, &mut class_of, seed),
        _ => {}
    }
    if let Some(o) = ocean.as_mut() {
        o.reef = reef;
    }

    // --- 5. river ---------------------------------------------------------
    let river = carve_river(
        &mut rng,
        &mut verts,
        w,
        h,
        vw,
        &mut surface_of,
        &mut class_of,
        ocean.as_ref(),
    );

    // --- 6. delta mouths (WaterLayout::Delta) ------------------------------
    if layout == WaterLayout::Delta {
        if let Some(o) = &ocean {
            carve_delta(
                &mut rng,
                &mut verts,
                w,
                h,
                vw,
                &mut surface_of,
                &mut class_of,
                &river,
                o,
            );
        }
    }

    // --- 7. lake / ponds --------------------------------------------------
    let lake = match layout {
        WaterLayout::RiverLake => maybe_lake(
            &mut rng,
            &mut verts,
            w,
            h,
            vw,
            &mut surface_of,
            &mut class_of,
            true,
        ),
        _ => maybe_lake(
            &mut rng,
            &mut verts,
            w,
            h,
            vw,
            &mut surface_of,
            &mut class_of,
            false,
        ),
    };
    let ponds = if layout == WaterLayout::Ponds {
        carve_ponds(&mut rng, &mut verts, w, h, vw, &mut surface_of, &mut class_of)
    } else {
        Vec::new()
    };

    // --- 5. per-tile corners ---------------------------------------------
    let mut tiles: Vec<Tile> = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            tiles.push(Tile {
                corner: [
                    verts[idx(vw, x, y)].round() as u8,
                    verts[idx(vw, x + 1, y)].round() as u8,
                    verts[idx(vw, x + 1, y + 1)].round() as u8,
                    verts[idx(vw, x, y + 1)].round() as u8,
                ],
                kind: Kind::Grass,
                prop: Prop::None,
                aux: 0,
            });
        }
    }
    let mut map = TileMap {
        w,
        h,
        tiles,
        seed,
        river: Some(river),
        lake,
        ponds,
        ocean,
        layout,
        water_class: class_of,
        wear: vec![0; (w * h) as usize],
        biome,
        neighbour,
        blend_edge,
        morph: Vec::new(),
        landforms: Vec::new(),
        hills: Vec::new(),
        soft: Default::default(),
    };

    shape_water(&mut map, &surface_of);
    // D43: the world's back edge (low `q = x + y`, the top of the screen) is
    // kept low so its silhouette hugs the horizon instead of filling the sky
    // band, which is where the distant ridges and the sun live.
    level_back_edge(&mut map);
    assign_materials(&mut map, rng.next_u64(), &beach_of);
    plant(&mut rng, &mut map, seed);
    // B46d: the big landforms are sprites; nothing is built or planted under one.
    if mark_landforms {
        crate::massif::plan(&mut map);
    }
    map
}

/// The land's own landform (D66). Applied to the vertex field before the water
/// is cut, so a river still follows the ground it always did.
fn shape_for_biome(verts: &mut [f32], vw: i32, vh: i32, biome: Biome, seed: u64) {
    match biome {
        // Dunes and buttes. B46c: the massif is a *cone cut into three strata*
        // — benches at 9, 13 and 17 with a cliff between each — so a hill becomes
        // a butte with two tall rock walls and a small summit cap, instead of one
        // thin wall around a huge flat plane. The rim wanders by a level or two
        // (noise on the cut), and the foot keeps a steeper skirt, which is where
        // the scree and the rubble gather.
        Biome::Desert => {
            let mean = 6.0f32;
            for y in 0..vh {
                for x in 0..vw {
                    let i = idx(vw, x, y);
                    let h = verts[i];
                    // The founding frame is the village's ground: no butte, no
                    // skirt, no strata in it. Without this a massif could take
                    // the middle and the surveyor would walk a third of a screen
                    // to find a flat, dry site (D35 is that the settlement starts
                    // at the monitor's centre).
                    // The dune basin the village stands in: a shallow, flat
                    // hollow at the middle of the *visible* frame (the frame's
                    // centre is a few tiles in from the map's, and the founding
                    // search wants flat ground at level five or less).
                    let fcx = (vw as f32 - 7.2) * 0.5;
                    let fcy = (vh as f32 - 7.2) * 0.5;
                    let dfx = x as f32 - fcx;
                    let dfy = y as f32 - fcy;
                    if dfx * dfx + dfy * dfy < 9.0 * 9.0 {
                        verts[i] = 4.0;
                        continue;
                    }
                    // A radius round the middle of the map, not the material
                    // rule's diamond: the founding point sits at the middle of
                    // the *visible* frame, which for a square map is a few tiles
                    // off the map's own centre, and the butte must not swallow
                    // it.
                    let dxc = x as f32 - vw as f32 * 0.5;
                    let dyc = y as f32 - vh as f32 * 0.5;
                    if dxc * dxc + dyc * dyc < (0.22 * vw as f32).powi(2) {
                        continue;
                    }
                    if h >= 9.0 {
                        // Two tiers, not a lattice of small steps: a bench at
                        // 10 round the massif and a cap at 16 on its summit,
                        // with a tall cliff between. The rim wanders by a level
                        // or two (noise on the cut), so it is not a circle.
                        let n = (fbm(x as f32 / 4.5, y as f32 / 4.5, seed ^ 0xB077E5, 2) - 0.5)
                            * 2.6;
                        let cut = h + n;
                        verts[i] = if cut >= 14.0 {
                            16.0
                        } else if cut >= 9.0 {
                            10.0
                        } else {
                            6.0
                        };
                    } else if h > mean {
                        // Only the *foot of the massif* steepens, so a talus of
                        // scree sits against the wall. Scaling the whole plain
                        // turned a flat 4-level area into a 4/5 staircase and
                        // the surveyor could find no flat ground near the centre.
                        let t = ((h - mean) / (9.0 - mean)).clamp(0.0, 1.0);
                        verts[i] = h + (h - mean) * 0.30 * t;
                    } else {
                        // The dune plain sits a level below the hills, so the
                        // buttes stand proud of it — and so the village's ground
                        // is low enough to build on (the founding search wants
                        // level five or less).
                        verts[i] = (h - 1.0).max(1.0);
                    }
                }
            }
        }
        // A tundra is a high, bare land: lift the middle band, keep the peaks.
        Biome::Tundra => {
            for v in verts.iter_mut() {
                if *v < 12.0 {
                    *v = (*v * 1.12 + 1.0).min(MAX_LEVEL as f32);
                }
            }
            let _ = (vw, vh);
        }
        // B46e: a valley. The floor (the village's ground) runs up the screen
        // between two foothill benches (pasture and wood) that carry a chain
        // of separate peaks along each side: cones the landform pass then
        // turns into faceted snow-capped mountains (B46d). Never a plateau,
        // which would draw as one grey wall.
        Biome::Alpine => {
            let w = (vw - 1) as f32;
            let floor_half = 0.17 * w;
            let sway_of = |q: f32| (fbm(q / 17.0, 3.1, seed ^ 0xA1_9E_01, 2) - 0.5) * 0.10 * w;
            // The peaks: five a side, spaced along the valley.
            let mut peaks: Vec<(f32, f32, f32, f32)> = Vec::new();
            let mut r = Rng::new(seed ^ 0xA1_9E_77);
            for side in [-1.0f32, 1.0] {
                for k in 0..5 {
                    let q = (0.50 + 0.17 * k as f32) * w + r.range_f32(-0.02, 0.02) * w;
                    let p = side * (0.36 + r.range_f32(-0.02, 0.05)) * w + sway_of(q);
                    let (cx, cy) = ((q + p) * 0.5, (q - p) * 0.5);
                    if cx < 6.0 || cy < 6.0 || cx > w - 6.0 || cy > (vh - 1) as f32 - 6.0 {
                        continue;
                    }
                    peaks.push((cx, cy, r.range_f32(8.5, 10.0), r.range_f32(12.5, 13.5)));
                }
            }
            for y in 0..vh {
                for x in 0..vw {
                    let i = idx(vw, x, y);
                    let q = (x + y) as f32;
                    let p = (x - y) as f32 - sway_of(q);
                    let d = (p.abs() - floor_half).max(0.0);
                    // The bench: a long slope of wooded foothills with gentle
                    // spurs, up to level 8 where it meets the peaks.
                    let spur = fbm(x as f32 / 9.0, y as f32 / 9.0, seed ^ 0xA1_9E_02, 2);
                    let bench = 3.0 + (d * 0.22).min(7.0) + (spur - 0.5) * 2.4 * (d / (0.1 * w)).min(1.0);
                    let mut e = if d > 0.0 { bench } else { verts[i].min(3.5) };
                    for &(cx, cy, pr, ph) in &peaks {
                        let dd = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() / pr;
                        if dd < 1.0 {
                            let t = 1.0 - dd;
                            // A steep, slightly concave flank.
                            e = e.max(bench.max(5.0) + ph * t.powf(0.85));
                        }
                    }
                    verts[i] = e.min(MAX_LEVEL as f32);
                }
            }
        }
        // B46e: the plain. Long, low swells; the hills are kopjes that stand up
        // out of it, never a range.
        Biome::Savanna => {
            for v in verts.iter_mut() {
                if *v < 9.0 {
                    *v = *v * 0.55 + 0.6;
                } else {
                    *v = 5.5 + (*v - 9.0) * 0.8 + 3.0 * 0.55;
                }
            }
            let _ = (vw, vh);
        }
        // B46e: a wetland is as flat as land gets: a level or two over the water.
        Biome::Swamp => {
            for v in verts.iter_mut() {
                *v = if *v >= 5.0 { 3.0 } else { 2.0 };
            }
            let _ = (vw, vh);
        }
        // B46e: islands and lagoons: low, sandy, with a few green rises.
        Biome::Islands | Biome::Lagoon => {
            for v in verts.iter_mut() {
                *v = if *v < 8.0 { *v * 0.55 + 0.5 } else { 4.9 + (*v - 8.0) * 0.45 };
            }
            let _ = (vw, vh);
        }
        // B46e: moorland: broad, rounded hills, never a crag; the tors are the
        // landforms.
        Biome::Highlands => {
            let w = (vw - 1) as f32;
            let h = (vh - 1) as f32;
            for y in 0..vh {
                for x in 0..vw {
                    let i = idx(vw, x, y);
                    if verts[i] >= 9.0 {
                        verts[i] = 9.0 + (verts[i] - 9.0) * 0.6;
                    }
                    // The moor rolls: broad swells that rise away from the
                    // village's ground and never make a crag.
                    let dp = ((x - y) as f32).abs() / (0.5 * w);
                    let dq = (((x + y) as f32) - 0.5 * (w + h)).abs() / (0.5 * h);
                    let away = ((dp * dp + dq * dq).sqrt() - 0.30).clamp(0.0, 0.45) / 0.45;
                    let away = away * away * (3.0 - 2.0 * away);
                    let roll = fbm(x as f32 / 13.0, y as f32 / 13.0, seed ^ 0x40_0F_01, 3);
                    verts[i] = verts[i].max(2.0 + away * (1.5 + 6.5 * roll));
                }
            }
        }
        // A jungle is low and rolling, with the peaks left in the distance.
        Biome::Tropical => {
            for v in verts.iter_mut() {
                if *v < 10.0 {
                    *v *= 0.82;
                }
            }
            let _ = (vw, vh);
        }
        _ => {}
    }
}

#[inline]
fn idx(w: i32, x: i32, y: i32) -> usize {
    (y * w + x) as usize
}

/// Elevation on the vertex grid: low rolling ground plus 1-2 hill masses that
/// sit toward an edge of the *frame*.
type Hill = (f32, f32, f32, f32, f32);

fn elevation(rng: &mut Rng, seed: u64, w: i32, h: i32, biome: Biome) -> (Vec<f32>, Vec<Hill>) {
    let vw = w + 1;
    let vh = h + 1;
    let base_seed = rng.next_u64();
    let ridge_seed = rng.next_u64();

    // Hill masses, placed in screen space: p = x - y (horizontal), q = x + y
    // (vertical). Scene-diagonal units: the frame's diamond reaches about
    // ±w/2 in p and c_q ± h/2 in q.
    let n_zones = if rng.chance(0.55) { 1 } else { 2 };
    // Frame-space bounds: the visible diamond is roughly |p| <= 0.45w and
    // q in [0.45w, 1.45w]. Mountain zones go in a band just inside one of its
    // sides, so the mass is on screen and the middle of the frame stays low.
    let p_max = 0.44 * w as f32;
    let q0 = 0.46 * w as f32;
    let q1 = 1.44 * w as f32;
    let frame_r = p_max.min(0.5 * (q1 - q0));
    // Zone = (cx, cy, radius, angle, base height, peaks: (px, py, r, height))
    struct Zone {
        cx: f32,
        cy: f32,
        radius: f32,
        angle: f32,
        base: f32,
        peaks: Vec<(f32, f32, f32, f32)>,
    }
    let mut zones: Vec<Zone> = Vec::new();
    for i in 0..n_zones {
        let theta = rng.range_f32(0.0, TAU) + i as f32 * 2.2;
        let p = (0.55 * frame_r + rng.range_f32(-0.15, 0.15) * frame_r) * theta.cos();
        let q =
            0.5 * (q0 + q1) + (0.55 * frame_r + rng.range_f32(-0.15, 0.15) * frame_r) * theta.sin();
        let cx = ((q + p) * 0.5).clamp(5.0, (w - 6) as f32);
        let cy = ((q - p) * 0.5).clamp(5.0, (h - 6) as f32);
        let radius = rng.range_f32(0.34, 0.46) * w as f32;
        let base = rng.range_f32(4.5, 7.0);
        // 1-3 distinct summits inside the mass: separate cones, so ridges and
        // saddles form where they meet instead of one smooth dome.
        let n_peaks = rng.range(1, 3);
        let mut peaks = Vec::new();
        for _ in 0..n_peaks {
            let a = rng.range_f32(0.0, TAU);
            let dist = rng.range_f32(0.0, 0.42) * radius;
            let px = cx + dist * a.cos();
            let py = cy + dist * a.sin();
            let r = radius * rng.range_f32(0.55, 0.85);
            let h = rng.range_f32(14.0, 18.0);
            peaks.push((px, py, r, h));
        }
        zones.push(Zone {
            cx,
            cy,
            radius,
            angle: rng.range_f32(0.0, TAU),
            base,
            peaks,
        });
    }
    let hills: Vec<Hill> = zones
        .iter()
        .map(|z| (z.cx, z.cy, z.radius, z.base, z.angle))
        .collect();

    let mut v = vec![0.0f32; (vw * vh) as usize];
    for y in 0..vh {
        for x in 0..vw {
            let fx = x as f32;
            let fy = y as f32;
            // Gentle rolling base.
            let base = fbm(fx / 13.0, fy / 13.0, base_seed, 3);
            let mut e = 1.0 + 2.6 * base;

            for z in &zones {
                // Elliptical, rotated falloff for the massif itself (foothills).
                let dx = fx - z.cx;
                let dy = fy - z.cy;
                let (sns, cs) = (z.angle.sin(), z.angle.cos());
                let rx = dx * cs + dy * sns;
                let ry = (-dx * sns + dy * cs) * 1.4;
                let d = (rx * rx + ry * ry).sqrt() / z.radius;
                if d < 1.0 {
                    let t = 1.0 - d;
                    e += z.base * t.powf(1.25);
                }
                for &(px, py, pr, ph) in &z.peaks {
                    let ddx = fx - px;
                    let ddy = fy - py;
                    let dp = (ddx * ddx + ddy * ddy).sqrt() / pr;
                    if dp >= 1.0 {
                        continue;
                    }
                    let t = 1.0 - dp;
                    // Ridged noise carves spurs and gullies into the flanks.
                    let rn = fbm(fx / 9.5, fy / 9.5, ridge_seed, 3);
                    let ridge = 1.0 - (2.0 * rn - 1.0).abs();
                    // D66: the land's own summit profile. A desert butte rises
                    // steeply at the rim and then runs flat across the top (a
                    // butte is a cliff, not a dome); a tundra is a shade sharper
                    // than a temperate hill; a jungle is a low, wide dome.
                    let prof = match biome {
                        // A butte: flat plain, a steep flank, a small cap. The
                        // height has to arrive within a few tiles for the strata
                        // cut above to become a *wall* instead of a wide bench.
                        Biome::Desert => {
                            let u = ((t - 0.62) / 0.17).clamp(0.0, 1.0);
                            u * u * (3.0 - 2.0 * u)
                        }
                        Biome::Tundra => t.powf(0.70),
                        Biome::Tropical => t.powf(1.25),
                        _ => t.powf(0.9),
                    };
                    let cone = 1.0 + ph * prof * (0.86 + 0.22 * ridge);
                    // max(), not sum(): keeps summits distinct.
                    e = e.max(cone);
                }
            }

            // Keep the middle of the frame low enough to build a city on.
            let p = (x as f32) - (y as f32);
            let q = (x as f32) + (y as f32);
            let dp = p / (w as f32 * 0.5);
            let dq = (q - 0.5 * (w + h) as f32) / (h as f32 * 0.5);
            let centre = 1.0 - ((dp * dp + dq * dq) * 2.6).min(1.0);
            if centre > 0.0 {
                let cap = 3.5;
                e = e + (e.min(cap) - e) * centre;
            }
            // D31: mountains belong at the edges. Fade the central cap in over a
            // wide margin (B11: a hard cap left a mesa with cliff walls).
            let p_norm = p.abs() / (0.20 * w as f32);
            let q_norm = ((q - 0.5 * (w + h) as f32) / (0.10 * (w + h) as f32)).abs();
            let d = p_norm.max(q_norm);
            // 1 well inside the central box, 0 at 2.2x its half-size.
            let w_centre = (1.0 - ((d - 1.0) / 1.2).clamp(0.0, 1.0)).powf(2.0);
            if w_centre > 0.0 {
                let capped = e.min(4.0);
                e = e + (capped - e) * w_centre;
            }
            v[idx(vw, x, y)] = e;
        }
    }
    let _ = seed;
    (v, hills)
}

/// Quantise and pull every vertex to within one level of its neighbours.
fn clamp_vertices(v: &mut [f32], vw: i32, vh: i32, passes: u32) {
    for e in v.iter_mut() {
        *e = e.round().clamp(1.0, MAX_LEVEL as f32);
    }
    for _ in 0..passes {
        let snapshot = v.to_vec();
        for y in 0..vh {
            for x in 0..vw {
                let i = idx(vw, x, y);
                let mut lo = MAX_LEVEL as f32;
                let mut hi = 0.0f32;
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let j = idx(vw, (x + dx).clamp(0, vw - 1), (y + dy).clamp(0, vh - 1));
                    lo = lo.min(snapshot[j]);
                    hi = hi.max(snapshot[j]);
                }
                if snapshot[i] > hi + 1.0 {
                    v[i] = hi + 1.0;
                } else if snapshot[i] < lo - 1.0 {
                    v[i] = lo - 1.0;
                }
            }
        }
    }
}

/// Is this tile inside the central 40 % of the frame (the village's ground)?
/// `p = x - y` is the horizontal and `q = x + y` the vertical frame coordinate.
pub fn in_central_frame(x: i32, y: i32, w: i32, h: i32) -> bool {
    let p = (x - y) as f32;
    let q = (x + y) as f32;
    let qc = 0.5 * (w + h) as f32;
    p.abs() <= 0.20 * w as f32 && (q - qc).abs() <= 0.20 * (w + h) as f32 * 0.5
}

/// Highest level anywhere in the frame's visible diamond.
fn max_visible_level(verts: &[f32], w: i32, h: i32, vw: i32) -> u8 {
    let p_max = 0.455 * w as f32;
    let q0 = 0.455 * w as f32;
    let q1 = 1.485 * w as f32;
    let mut best = 0u8;
    for y in 0..h {
        for x in 0..w {
            let p = (x - y) as f32;
            let q = (x + y) as f32;
            if p.abs() > p_max || q < q0 || q > q1 {
                continue;
            }
            let m = verts[idx(vw, x, y)]
                .max(verts[idx(vw, x + 1, y)])
                .max(verts[idx(vw, x + 1, y + 1)])
                .max(verts[idx(vw, x, y + 1)]);
            best = best.max(m as u8);
        }
    }
    best
}

/// Share of the frame's visible diamond whose tiles reach level 6 or more.
fn visible_high_fraction(verts: &[f32], w: i32, h: i32, vw: i32) -> f32 {
    let p_max = 0.455 * w as f32;
    let q0 = 0.455 * w as f32;
    let q1 = 1.485 * w as f32;
    let (mut seen, mut high) = (0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            let p = (x - y) as f32;
            let q = (x + y) as f32;
            if p.abs() > p_max || q < q0 || q > q1 {
                continue;
            }
            seen += 1;
            let m = verts[idx(vw, x, y)]
                .max(verts[idx(vw, x + 1, y)])
                .max(verts[idx(vw, x + 1, y + 1)])
                .max(verts[idx(vw, x, y + 1)]);
            if m >= 6.0 {
                high += 1;
            }
        }
    }
    if seen == 0 {
        0.0
    } else {
        high as f32 / seen as f32
    }
}

/// Clamp every vertex below `MOUNTAIN` to within one level of its neighbours,
/// leaving genuine mountain ground alone (B13 item 5).
fn clamp_low_ground(v: &mut [f32], vw: i32, vh: i32, passes: u32) {
    const MOUNTAIN: f32 = 9.5;
    for _ in 0..passes {
        let snapshot = v.to_vec();
        for y in 0..vh {
            for x in 0..vw {
                let i = idx(vw, x, y);
                if snapshot[i] >= MOUNTAIN {
                    continue;
                }
                let mut lo = MAX_LEVEL as f32;
                let mut hi = 0.0f32;
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let j = idx(vw, (x + dx).clamp(0, vw - 1), (y + dy).clamp(0, vh - 1));
                    lo = lo.min(snapshot[j]);
                    hi = hi.max(snapshot[j]);
                }
                if snapshot[i] > hi + 1.0 {
                    v[i] = hi + 1.0;
                } else if snapshot[i] < lo - 1.0 {
                    v[i] = lo - 1.0;
                }
            }
        }
    }
}

/// Remove isolated bumps and pits: a vertex that is a strict local extreme of
/// all four neighbours becomes the median. Single-vertex spikes read as holes in
/// an iso projection, not as terrain.
fn despeckle(v: &mut [f32], vw: i32, vh: i32, passes: u32) {
    for _ in 0..passes {
        let snapshot = v.to_vec();
        for y in 1..vh - 1 {
            for x in 1..vw - 1 {
                let i = idx(vw, x, y);
                let n = [
                    snapshot[idx(vw, x - 1, y)],
                    snapshot[idx(vw, x + 1, y)],
                    snapshot[idx(vw, x, y - 1)],
                    snapshot[idx(vw, x, y + 1)],
                ];
                let v0 = snapshot[i];
                let above = n.iter().filter(|q| **q < v0).count();
                let below = n.iter().filter(|q| **q > v0).count();
                if above == 4 || below == 4 {
                    let mut sorted = n;
                    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    v[i] = (sorted[1] + sorted[2]) * 0.5;
                }
            }
        }
    }
}

/// Weighted pick of the world's water layout (D44).
fn pick_layout(rng: &mut Rng) -> WaterLayout {
    let total: u32 = WaterLayout::ALL.iter().map(|l| l.weight()).sum();
    let mut r = rng.usize(total as usize) as u32;
    for l in WaterLayout::ALL {
        let w = l.weight();
        if r < w {
            return l;
        }
        r -= w;
    }
    WaterLayout::RiverLake
}

/// Mark water tiles for a channel: every tile whose centre is within `halfw` of
/// the path gets the surface level, and the vertices under it are pulled down to
/// the waterline so the banks slope in.
fn carve_channel(
    verts: &mut [f32],
    w: i32,
    h: i32,
    vw: i32,
    surface_of: &mut [u8],
    class_of: &mut [u8],
    path: &[(i32, i32)],
    halfw: &[f32],
    levels: &[u8],
    class: WaterKind,
) {
    for y in 0..h {
        for x in 0..w {
            let cx = x as f32 + 0.5;
            let cy = y as f32 + 0.5;
            let mut best = f32::MAX;
            let mut best_i = 0usize;
            for (i, &(px, py)) in path.iter().enumerate() {
                let d = (cx - px as f32).hypot(cy - py as f32);
                if d < best {
                    best = d;
                    best_i = i;
                }
            }
            if best <= halfw[best_i.min(halfw.len() - 1)] {
                let i = (y * w + x) as usize;
                surface_of[i] = levels[best_i.min(levels.len() - 1)].max(1);
                if class_of[i] == 0 {
                    class_of[i] = class.code();
                }
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            if surface_of[(y * w + x) as usize] == 0 {
                continue;
            }
            for (vx, vy) in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)] {
                let j = idx(vw, vx, vy);
                let lvl = surface_of[(y * w + x) as usize] as f32;
                if verts[j] > lvl {
                    verts[j] = lvl;
                }
            }
        }
    }
}

/// How wide the sea is allowed to be (B46e): the default is the historical
/// quarter of the map; an island land takes much more.
#[derive(Clone, Copy)]
struct SeaShape {
    c_lo: f32,
    c_hi: f32,
    max_frac: f32,
}

const SEA_DEFAULT: SeaShape = SeaShape { c_lo: 0.62, c_hi: 0.80, max_frac: 0.25 };
/// The shore stands just outside the village's ground (the central frame is
/// `|p| <= 0.2 w`, about 0.45 of the visible half width).
const SEA_WIDE: SeaShape = SeaShape { c_lo: 0.50, c_hi: 0.54, max_frac: 0.60 };

/// The sea: a noisy shore along the screen's left or right side, one flat
/// surface, a beach shelf and a line of dunes/rocks just behind it (D44, B21
/// item 2).
///
/// The camera is a fixed 45° view, so the *screen* sides are lines of constant
/// `p = x - y`, not the tile grid's edges (the grid's corners are off-screen).
/// The shore therefore sits at `|p| = ~0.6 * (half the visible width)`, which
/// leaves the village's ground at `p = 0` dry.
fn carve_ocean(
    rng: &mut Rng,
    verts: &mut [f32],
    w: i32,
    h: i32,
    vw: i32,
    surface_of: &mut [u8],
    class_of: &mut [u8],
    beach_of: &mut [bool],
    shape: SeaShape,
) -> Ocean {
    let side: i8 = if rng.chance(0.5) { -1 } else { 1 };
    let level: u8 = 1 + rng.usize(2) as u8;
    // Half the visible width in p: the framebuffer covers about `w` tiles
    // across, so the visible p range is roughly ±(w / 2 - 3).
    let p_vis = (w.min(h) as f32 / 2.0 - 3.0).max(6.0);
    let wob_seed = rng.next_u64();
    // B27 item 3: the sea may cover at most a quarter of the visible land. The
    // shore starts where B21 put it and is pushed out until the count agrees —
    // an analytic guess does not survive the wobble and the map's aspect.
    let base = rng.range_f32(shape.c_lo, shape.c_hi);
    let mut c = base;
    let (mut shore_p, mut tiles);
    loop {
        let sp = side as f32 * p_vis * c;
        // Wobble the shoreline along the screen's vertical (q = x + y).
        let at = |x: i32, y: i32| -> f32 {
            let wob = (fbm((x + y) as f32 / 11.0, 0.5, wob_seed, 2) - 0.5) * 6.0;
            sp - side as f32 * wob
        };
        let mut n = 0usize;
        for y in 0..h {
            for x in 0..w {
                let p = (x - y) as f32;
                if side as f32 * (p - at(x, y)) > 0.0 {
                    n += 1;
                }
            }
        }
        shore_p = sp;
        tiles = n;
        let frac = n as f32 / (w * h) as f32;
        if frac <= shape.max_frac || c >= 0.95 {
            break;
        }
        c = (c + 0.05).min(0.95);
    }
    let shore_at = |x: i32, y: i32| -> f32 {
        let wob = (fbm((x + y) as f32 / 11.0, 0.5, wob_seed, 2) - 0.5) * 6.0;
        shore_p - side as f32 * wob
    };
    for y in 0..h {
        for x in 0..w {
            let p = (x - y) as f32;
            if side as f32 * (p - shore_at(x, y)) > 0.0 {
                let i = (y * w + x) as usize;
                surface_of[i] = level;
                class_of[i] = WaterKind::Sea.code();
            }
        }
    }
    // Pull the sea bed and the beach shelf down: a gentle step into the water.
    for y in 0..h {
        for x in 0..w {
            let p = (x - y) as f32;
            let d = side as f32 * (p - shore_at(x, y));
            if d > -3.5 && d <= 0.0 {
                beach_of[(y * w + x) as usize] = true;
            }
            for (vx, vy) in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)] {
                let j = idx(vw, vx, vy);
                if d > 0.0 {
                    if verts[j] > level as f32 {
                        verts[j] = level as f32;
                    }
                } else if d > -4.0 && verts[j] > level as f32 + 1.0 {
                    // A wide beach shelf: the first few tiles inland are only a
                    // step above the water, so they read as sand.
                    verts[j] = level as f32 + 1.0;
                } else if d <= -4.0 {
                    // A gentle coastal plain: the land climbs away from the
                    // beach instead of standing as a wall at the waterline.
                    let cap = level as f32 + 1.0 + (-d - 4.0) * 0.70;
                    if verts[j] > cap {
                        verts[j] = cap;
                    }
                }
            }
        }
    }
    // Dunes and a few rock outcrops just inland of the beach.
    for _ in 0..(w + h) / 5 {
        let q = rng.range(0, w + h) as i32;
        let depth = rng.range(2, 4) as f32;
        let p = shore_p - side as f32 * depth;
        let x = ((q + p as i32) / 2).clamp(0, w - 1);
        let y = ((q - p as i32) / 2).clamp(0, h - 1);
        let rock = rng.chance(0.15);
        let r = if rock { 1 } else { 2 };
        for dy in -r..=r {
            for dx in -r..=r {
                let (tx, ty) = (x + dx, y + dy);
                if tx < 0 || ty < 0 || tx >= w || ty >= h {
                    continue;
                }
                if surface_of[(ty * w + tx) as usize] > 0 {
                    continue;
                }
                let lift = if rock { 2.0 } else { 1.0 };
                // A dune is a bump on the coastal slope, never a new hill: cap
                // it a couple of levels above the ramp. Otherwise 28 overlapping
                // bumps pile up into a wall at the waterline.
                let d = side as f32 * ((tx - ty) as f32 - shore_at(tx, ty));
                let cap = level as f32 + 1.0 + (-d - 4.0).max(0.0) * 0.55 + 2.0;
                for (vx, vy) in [(tx, ty), (tx + 1, ty), (tx + 1, ty + 1), (tx, ty + 1)] {
                    let j = idx(vw, vx, vy);
                    verts[j] = (verts[j] + lift).min(cap).min(MAX_LEVEL as f32);
                }
            }
        }
    }
    Ocean {
        side,
        shore_p,
        level,
        tiles,
        reef: 0.0,
    }
}


/// Is this tile inside the village's ground, with a margin?
fn village_ground(x: i32, y: i32, w: i32, h: i32, margin: f32) -> bool {
    let p = (x - y) as f32;
    let q = (x + y) as f32;
    let qc = 0.5 * (w + h) as f32;
    p.abs() <= 0.20 * w as f32 + margin && (q - qc).abs() <= 0.20 * (w + h) as f32 * 0.5 + margin
}

/// Turn one tile to sea at `level`, pulling the vertices under it down.
fn sea_tile(
    x: i32,
    y: i32,
    level: u8,
    w: i32,
    vw: i32,
    verts: &mut [f32],
    surface_of: &mut [u8],
    class_of: &mut [u8],
) {
    let i = (y * w + x) as usize;
    surface_of[i] = level;
    class_of[i] = WaterKind::Sea.code();
    for (vx, vy) in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)] {
        let j = idx(vw, vx, vy);
        verts[j] = verts[j].min(level as f32);
    }
}

/// B46e: an archipelago. The land is the village's island in the middle and
/// the sea reaches in from both sides; straits cut the rest into islands and
/// islets stand out of the sea.
fn carve_archipelago(
    rng: &mut Rng,
    verts: &mut [f32],
    w: i32,
    h: i32,
    vw: i32,
    surface_of: &mut [u8],
    class_of: &mut [u8],
    beach_of: &mut [bool],
    o: &Ocean,
    seed: u64,
) {
    let level = o.level;
    // The second sea, on the far side from the first.
    let far = -(o.side as f32);
    let far_shore = far * o.shore_p.abs();
    let wob = rng.next_u64();
    for y in 0..h {
        for x in 0..w {
            let p = (x - y) as f32;
            let at = far_shore - far * (fbm((x + y) as f32 / 11.0, 0.5, wob, 2) - 0.5) * 6.0;
            let i = (y * w + x) as usize;
            if far * (p - at) > 0.0 && surface_of[i] == 0 {
                sea_tile(x, y, level, w, vw, verts, surface_of, class_of);
            } else if surface_of[i] == 0 && far * (p - at) > -3.5 {
                beach_of[i] = true;
                for (vx, vy) in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)] {
                    let j = idx(vw, vx, vy);
                    verts[j] = verts[j].min(level as f32 + 1.0);
                }
            }
        }
    }
    // Straits: contour lines of a slow noise, kept clear of the village.
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if surface_of[i] != 0 || village_ground(x, y, w, h, 3.0) {
                continue;
            }
            let n = fbm(x as f32 / 11.0, y as f32 / 11.0, seed ^ 0x15_1A_D5, 3);
            if (2.0 * n - 1.0).abs() < 0.11 {
                sea_tile(x, y, level, w, vw, verts, surface_of, class_of);
            }
        }
    }
    // Islets standing out of the sea: a blob of low sand and palms.
    for _ in 0..(w + h) / 14 {
        let cx = rng.range(4, w - 5);
        let cy = rng.range(4, h - 5);
        let r = rng.range(1, 3) as f32 + 0.5;
        if village_ground(cx, cy, w, h, 3.0) {
            continue;
        }
        for dy in -3..=3 {
            for dx in -3..=3 {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= w || y >= h {
                    continue;
                }
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d > r * (1.0 + 0.3 * (hash2(x, y, 0x151E7) - 0.5)) {
                    continue;
                }
                let i = (y * w + x) as usize;
                if surface_of[i] == 0 {
                    continue;
                }
                surface_of[i] = 0;
                class_of[i] = 0;
                beach_of[i] = true;
                for (vx, vy) in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)] {
                    verts[idx(vw, vx, vy)] = level as f32 + if d < r * 0.5 { 2.0 } else { 1.0 };
                }
            }
        }
    }
}

/// B46e: a lagoon. A sand barrier, broken by gaps, stands off the shore; the
/// water behind it is the lagoon, the water beyond the open sea.
fn carve_barrier(
    rng: &mut Rng,
    verts: &mut [f32],
    w: i32,
    h: i32,
    vw: i32,
    surface_of: &mut [u8],
    class_of: &mut [u8],
    o: &Ocean,
    seed: u64,
) -> f32 {
    let level = o.level;
    let off = 7.5 + rng.range_f32(0.0, 2.0);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if surface_of[i] == 0 || class_of[i] != WaterKind::Sea.code() {
                continue;
            }
            let p = (x - y) as f32;
            let d = o.side as f32 * (p - o.shore_p);
            // The barrier wanders along the shore.
            let wander = (fbm((x + y) as f32 / 9.0, 2.5, seed ^ 0xBA_44_1E, 2) - 0.5) * 4.0;
            let band = (d - off - wander).abs();
            if band > 1.35 {
                continue;
            }
            // Gaps: a lagoon has passes through its reef.
            let gap = fbm((x + y) as f32 / 6.0, 7.5, seed ^ 0xBA_44_1F, 2);
            if gap < 0.40 {
                continue;
            }
            surface_of[i] = 0;
            class_of[i] = 0;
            let top = level as f32 + if band < 0.6 { 2.0 } else { 1.0 };
            for (vx, vy) in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)] {
                let j = idx(vw, vx, vy);
                verts[j] = verts[j].max(top);
            }
        }
    }
    off
}

/// B46e: a wetland. Meandering channels (contour lines of a slow noise) and
/// open pools through ground that is only a level or two above the water, with
/// the village's ground kept dry.
fn carve_wetland(
    verts: &mut [f32],
    w: i32,
    h: i32,
    vw: i32,
    surface_of: &mut [u8],
    class_of: &mut [u8],
    seed: u64,
) {
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if surface_of[i] != 0 || village_ground(x, y, w, h, 1.0) {
                continue;
            }
            let n = fbm(x as f32 / 10.0, y as f32 / 10.0, seed ^ 0x5A_4D_01, 3);
            let pool = fbm(x as f32 / 7.0, y as f32 / 7.0, seed ^ 0x5A_4D_02, 2);
            if (2.0 * n - 1.0).abs() < 0.050 || pool > 0.73 {
                surface_of[i] = 2;
                class_of[i] = WaterKind::Pond.code();
                for (vx, vy) in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)] {
                    let j = idx(vw, vx, vy);
                    verts[j] = verts[j].min(2.0);
                }
            }
        }
    }
}

/// Two extra mouths fanning out from the last stretch of the river, so a delta
/// reads as a river splitting into the sea (D44).
#[allow(clippy::too_many_arguments)]
fn carve_delta(
    rng: &mut Rng,
    verts: &mut [f32],
    w: i32,
    h: i32,
    vw: i32,
    surface_of: &mut [u8],
    class_of: &mut [u8],
    river: &River,
    ocean: &Ocean,
) {
    let path = &river.path;
    if path.len() < 8 {
        return;
    }
    let start = path[path.len() / 2];
    let level = *river.surface.last().unwrap_or(&1).max(&1);
    let side = ocean.side as f32;
    let (q0, _) = (start.0 + start.1, 0);
    for k in [-1.0f32, 1.0] {
        // Each mouth aims at its own point along the shore, so the branches fan
        // out instead of running parallel.
        let target_q = q0 as f32 + k * rng.range_f32(6.0, 11.0);
        let mut cur = start;
        let mut branch = vec![cur];
        let mut guard = 0;
        while surface_of[(cur.1 * w + cur.0) as usize] == 0 && guard < 2 * (w + h) {
            guard += 1;
            let p_here = (cur.0 - cur.1) as f32;
            // Only walk if this is not already the sea.
            if side * (p_here - ocean.shore_p) > 0.0 {
                break;
            }
            let mut best = None;
            let mut best_score = f32::MIN;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let nx = cur.0 + dx;
                let ny = cur.1 + dy;
                if nx < 1 || ny < 1 || nx >= w - 1 || ny >= h - 1 {
                    continue;
                }
                let p_new = (nx - ny) as f32;
                let q_new = (nx + ny) as f32;
                let score = side * (p_new - p_here) - 0.35 * (q_new - target_q).abs();
                if score > best_score {
                    best_score = score;
                    best = Some((nx, ny));
                }
            }
            let Some(nxt) = best else { break };
            if nxt == cur {
                break;
            }
            cur = nxt;
            branch.push(cur);
        }
        // A little jitter in the width so the mouths are not identical.
        let wob = rng.range_f32(0.0, 1.0);
        // Narrow braids (H21 review): about one tile wide, not a lagoon.
        let halfw: Vec<f32> = (0..branch.len())
            .map(|i| 0.55 + 0.18 * ((i as f32 * 0.3 + wob).sin().abs()))
            .collect();
        let levels = vec![level; branch.len()];
        carve_channel(
            verts,
            w,
            h,
            vw,
            surface_of,
            class_of,
            &branch,
            &halfw,
            &levels,
            WaterKind::River,
        );
    }
}

/// Two to four small round ponds in low ground away from the centre.
fn carve_ponds(
    rng: &mut Rng,
    verts: &mut [f32],
    w: i32,
    h: i32,
    vw: i32,
    surface_of: &mut [u8],
    class_of: &mut [u8],
) -> Vec<Lake> {
    let want = 2 + rng.usize(3);
    let mut out = Vec::new();
    for _ in 0..want {
        let radius = rng.range(2, 4);
        let mut placed = false;
        for _try in 0..48 {
            let cx = rng.range(radius + 3, w - radius - 4);
            let cy = rng.range(radius + 3, h - radius - 4);
            let mut clash = false;
            for dy in -radius - 2..=radius + 2 {
                for dx in -radius - 2..=radius + 2 {
                    let x = (cx + dx).clamp(0, w - 1);
                    let y = (cy + dy).clamp(0, h - 1);
                    if surface_of[(y * w + x) as usize] > 0 {
                        clash = true;
                    }
                }
            }
            if clash || in_central_frame(cx, cy, w, h) {
                continue;
            }
            let mut level = MAX_LEVEL;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx * dx + dy * dy > radius * radius {
                        continue;
                    }
                    let x = (cx + dx).clamp(0, w);
                    let y = (cy + dy).clamp(0, h);
                    level = level.min(verts[idx(vw, x, y)] as u8);
                }
            }
            let level = level.clamp(1, 4);
            let mut tiles = 0usize;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let d = ((dx * dx + dy * dy) as f32).sqrt();
                    let wob = 1.0 + 0.25 * (hash2(cx + dx, cy + dy, 0x9D0B_5000) - 0.5);
                    if d > radius as f32 * wob {
                        continue;
                    }
                    let x = cx + dx;
                    let y = cy + dy;
                    if x < 0 || y < 0 || x >= w || y >= h {
                        continue;
                    }
                    surface_of[(y * w + x) as usize] = level;
                    class_of[(y * w + x) as usize] = WaterKind::Pond.code();
                    for (vx, vy) in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)] {
                        let j = idx(vw, vx, vy);
                        verts[j] = verts[j].min(level as f32);
                    }
                    tiles += 1;
                }
            }
            if tiles > 3 {
                out.push(Lake {
                    centre: (cx, cy),
                    level,
                    tiles,
                });
                placed = true;
            }
            break;
        }
        let _ = placed;
    }
    out
}

fn edge_point(e: Edge, along: f32, w: i32, h: i32) -> (i32, i32) {
    match e {
        Edge::Left => (0, ((along * h as f32) as i32).clamp(0, h - 1)),
        Edge::Right => (w - 1, ((along * h as f32) as i32).clamp(0, h - 1)),
        Edge::Top => (((along * w as f32) as i32).clamp(0, w - 1), 0),
        Edge::Bottom => (((along * w as f32) as i32).clamp(0, w - 1), h - 1),
    }
}

/// Walk from one edge to another, preferring low ground (the river finds the
/// valley instead of cutting through a hill).
fn trace_river(
    rng: &mut Rng,
    verts: &[f32],
    vw: i32,
    w: i32,
    h: i32,
    from: Edge,
    to: Edge,
    goal: Option<(i32, i32)>,
    start_hint: Option<(i32, i32)>,
) -> Vec<(i32, i32)> {
    let lowest_on = |e: Edge| -> (i32, i32) {
        let mut best = edge_point(e, 0.5, w, h);
        let mut best_h = f32::MAX;
        for k in 0..=12 {
            let p = edge_point(e, 0.28 + 0.44 * k as f32 / 12.0, w, h);
            let hh = verts[idx(vw, p.0, p.1)];
            if hh < best_h {
                best_h = hh;
                best = p;
            }
        }
        best
    };
    let start = start_hint.unwrap_or_else(|| lowest_on(from));
    let target = goal.unwrap_or_else(|| lowest_on(to));
    let reached = |p: (i32, i32)| -> bool {
        match goal {
            Some(g) => (p.0 - g.0).abs() + (p.1 - g.1).abs() <= 1,
            None => to.contains(p.0, p.1, w, h),
        }
    };

    let mut path = vec![start];
    let mut pos = start;
    let mut step = 0i32;
    let phase = rng.range_f32(0.0, TAU);
    let bend = rng.range_f32(0.45, 0.85);
    let max_steps = 4 * (w + h);

    while !reached(pos) && step < max_steps {
        step += 1;
        // Direction: toward the exit, with a meander, plus a pull downhill.
        let tx = target.0 as f32 - pos.0 as f32;
        let ty = target.1 as f32 - pos.1 as f32;
        let len = (tx * tx + ty * ty).sqrt().max(1.0);
        let ang = (step as f32 * 0.20 + phase).sin() * bend;
        let (s, c) = (ang.sin(), ang.cos());
        let ux = (tx / len) * c - (ty / len) * s;
        let uy = (tx / len) * s + (ty / len) * c;

        // Score the four neighbours: prefer downhill, then the intended heading.
        let mut best = None;
        let mut best_score = f32::MIN;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nx = pos.0 + dx;
            let ny = pos.1 + dy;
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                continue;
            }
            let hh = verts[idx(vw, nx, ny)];
            let here = verts[idx(vw, pos.0, pos.1)];
            let score = (here - hh) * 1.4 + ux * dx as f32 + uy * dy as f32;
            if score > best_score {
                best_score = score;
                best = Some((nx, ny));
            }
        }
        let Some(np) = best else { break };
        if np == pos {
            break;
        }
        pos = np;
        path.push(pos);
    }
    // Guarantee we reach the exit edge (or the shore).
    let mut guard = 0;
    let exit = lowest_on(to); // the lowest point of the exit edge, not a corner
    while !reached(pos) && guard < max_steps {
        guard += 1;
        // Head for the exit *point* rather than straight out of the map: a river
        // that leaves through a far corner is off-screen (H21 review).
        let dx = (exit.0 - pos.0).signum();
        let dy = (exit.1 - pos.1).signum();
        let step = if dx.abs() + dy.abs() == 0 {
            (0, 0)
        } else if (exit.0 - pos.0).abs() >= (exit.1 - pos.1).abs() {
            (dx, 0)
        } else {
            (0, dy)
        };
        if step == (0, 0) {
            break;
        }
        pos = ((pos.0 + step.0).clamp(0, w - 1), (pos.1 + step.1).clamp(0, h - 1));
        path.push(pos);
    }
    path
}

/// Trace and carve the river. Returns the record plus, per tile, the water
/// surface level (0 = dry land).
fn carve_river(
    rng: &mut Rng,
    verts: &mut [f32],
    w: i32,
    h: i32,
    vw: i32,
    surface_of: &mut [u8],
    class_of: &mut [u8],
    ocean: Option<&Ocean>,
) -> River {
    // A coastal map: the river enters from the most inland point of a land edge
    // (a corner of the entry edge can be under the sea) and leaves through the
    // side that carries the sea, so its mouth lands in the water (D44).
    // Elsewhere both edges are free.
    let (from, to, goal) = match ocean {
        Some(o) => {
            // Enter across the map and stop just inland of the shore, so the
            // river and its delta are on land instead of swallowed by the sea.
            let to = if o.side < 0 { Edge::Left } else { Edge::Right };
            let from = if rng.chance(0.5) { Edge::Top } else { Edge::Bottom };
            (from, to, None)
        }
        None => {
            let from = EDGES[rng.usize(4)];
            let mut to = EDGES[rng.usize(4)];
            if to == from {
                to = EDGES[(EDGES.iter().position(|e| *e == from).unwrap() + 2) % 4];
            }
            (from, to, None)
        }
    };
    let start_hint = ocean.map(|o| {
        // The most inland point of the entry edge: a corner of it can be under
        // the sea, and a river cannot start there.
        let mut best = i32::MIN;
        let mut bp = edge_point(from, 0.5, w, h);
        for k in 0..=20 {
            let p = edge_point(from, 0.05 + 0.9 * k as f32 / 20.0, w, h);
            let m = -(o.side as i32) * ((p.0 - p.1) - o.shore_p as i32);
            if m > best {
                best = m;
                bp = p;
            }
        }
        bp
    });
    let mut path = trace_river(rng, verts, vw, w, h, from, to, goal, start_hint);
    // On a coast: cut the river at the point closest to the shore, then walk it
    // the last few tiles onto the beach, so its mouth is on the sand rather than
    // the river running out of the map (D44).
    if let Some(o) = ocean {
        let dist = |x: i32, y: i32| o.side as f32 * ((x - y) as f32 - o.shore_p);
        let best = path
            .iter()
            .enumerate()
            .min_by(|a, b| {
                dist(a.1.0, a.1.1)
                    .abs()
                    .partial_cmp(&dist(b.1.0, b.1.1).abs())
                    .unwrap()
            })
            .map(|(i, _)| i)
            .unwrap_or(path.len() - 1);
        path.truncate((best + 1).max(3).min(path.len()));
        let sea_at = |x: i32, y: i32| -> bool {
            x >= 0
                && y >= 0
                && x < w
                && y < h
                && class_of[(y * w + x) as usize] == WaterKind::Sea.code()
        };
        let dx = if o.side > 0 { 1 } else { -1 };
        let mut cur = *path.last().unwrap();
        // The shore can be far from the river's last bend (B27 pushed the sea
        // out to a quarter of the land), so the walk that puts the mouth on the
        // beach is generous.
        for k in 0..80 {
            let touching = sea_at(cur.0, cur.1)
                || Dir::ALL.iter().any(|d| {
                    let (ex, ey) = d.delta();
                    sea_at(cur.0 + ex, cur.1 + ey)
                });
            if touching {
                break;
            }
            let step = if k % 2 == 0 { (dx, 0) } else { (0, -dx) };
            let nx = cur.0 + step.0;
            let ny = cur.1 + step.1;
            // The mouth may run right up to the map edge (a river that reaches
            // the sea *is* at the edge on a coastal map).
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                break;
            }
            cur = (nx, ny);
            path.push(cur);
        }
        // Corner case: the walk hit the map edge before the water. Cut straight
        // to the nearest sea tile so the mouth still reaches the sea.
        let touching = sea_at(cur.0, cur.1)
            || Dir::ALL.iter().any(|d| {
                let (ex, ey) = d.delta();
                sea_at(cur.0 + ex, cur.1 + ey)
            });
        if !touching {
            let mut best = None;
            let mut bd = i32::MAX;
            for y in 1..h - 1 {
                for x in 1..w - 1 {
                    if !sea_at(x, y) {
                        continue;
                    }
                    let d = (x - cur.0).abs() + (y - cur.1).abs();
                    if d < bd {
                        bd = d;
                        best = Some((x, y));
                    }
                }
            }
            if let Some((tx, ty)) = best {
                while cur != (tx, ty) && path.len() < (4 * (w + h)) as usize {
                    let dx = (tx - cur.0).signum();
                    let dy = (ty - cur.1).signum();
                    cur = if (tx - cur.0).abs() >= (ty - cur.1).abs() {
                        (cur.0 + dx, cur.1)
                    } else {
                        (cur.0, cur.1 + dy)
                    };
                    path.push(cur);
                }
            }
        }
    }

    // The water sits at (or just above) the lowest ground along the path, and
    // may drop one level partway with a rapid.
    let min_h = path
        .iter()
        .map(|&(x, y)| verts[idx(vw, x, y)])
        .fold(f32::MAX, f32::min);
    let base = (min_h.round() as i32).clamp(1, 3) as u8;
    let drop_at = if base >= 2 && rng.chance(0.5) {
        Some((path.len() as f32 * rng.range_f32(0.45, 0.8)) as usize)
    } else {
        None
    };
    let surface: Vec<u8> = (0..path.len())
        .map(|i| {
            if drop_at.is_some_and(|d| i >= d) {
                base - 1
            } else {
                base
            }
        })
        .collect();

    let base_half = rng.range_f32(0.85, 1.45);
    let widen = rng.range_f32(0.0, TAU);
    let halfw: Vec<f32> = (0..path.len())
        .map(|i| base_half * (1.0 + 0.18 * (i as f32 * 0.16 + widen).sin()))
        .collect();

    // Mark water tiles and pull the vertex field down along the channel so land
    // slopes into it (a tile is water when its centre is in the channel).
    carve_channel(
        verts,
        w,
        h,
        vw,
        surface_of,
        class_of,
        &path,
        &halfw,
        &surface,
        WaterKind::River,
    );

    River {
        entry: path[0],
        exit: *path.last().unwrap(),
        entry_edge: from,
        exit_edge: to,
        width: base_half * 2.0,
        path,
        surface,
    }
}

fn maybe_lake(
    rng: &mut Rng,
    verts: &mut [f32],
    w: i32,
    h: i32,
    vw: i32,
    surface_of: &mut [u8],
    class_of: &mut [u8],
    force: bool,
) -> Option<Lake> {
    // A RiverLake world always gets its lake; the other layouts may still get
    // one on a coin flip (variety without breaking the D44 mix).
    if !force && !rng.chance(0.25) {
        return None;
    }
    let radius = rng.range(4, 7);
    let mut best: Option<((i32, i32), f32)> = None;
    for _ in 0..64 {
        let cx = rng.range(radius + 2, w - radius - 3);
        let cy = rng.range(radius + 2, h - radius - 3);
        // Never in the village's founding pocket (D35/D44).
        if in_central_frame(cx, cy, w, h) {
            continue;
        }
        let mut near_water = false;
        for dy in -radius - 1..=radius + 1 {
            for dx in -radius - 1..=radius + 1 {
                let vx = (cx + dx).clamp(0, w);
                let vy = (cy + dy).clamp(0, h);
                if surface_of[(vy.min(h - 1) * w + vx.min(w - 1)) as usize] > 0 {
                    near_water = true;
                }
            }
        }
        if near_water {
            continue;
        }
        let hh = verts[idx(vw, cx, cy)];
        if best.map(|(_, bh)| hh < bh).unwrap_or(true) {
            best = Some(((cx, cy), hh));
        }
    }
    let ((cx, cy), _) = best?;

    let mut level = MAX_LEVEL;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= radius * radius {
                level =
                    level.min(verts[idx(vw, (cx + dx).clamp(0, w), (cy + dy).clamp(0, h))] as u8);
            }
        }
    }
    let level = level.clamp(1, 4);

    let mut tiles = 0usize;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            let wobble = 1.0 + 0.25 * (hash2(cx + dx, cy + dy, 0x1A4B_E000) - 0.5);
            if d > radius as f32 * wobble {
                continue;
            }
            let x = cx + dx;
            let y = cy + dy;
            if x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            surface_of[(y * w + x) as usize] = level;
            class_of[(y * w + x) as usize] = WaterKind::Lake.code();
            for (vx, vy) in [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)] {
                let j = idx(vw, vx, vy);
                verts[j] = verts[j].min(level as f32);
            }
            tiles += 1;
        }
    }
    Some(Lake {
        centre: (cx, cy),
        level,
        tiles,
    })
}

/// Water tiles are flat at their surface; low banks slope into the water; banks
/// three or more levels above it stay up and become cliff faces.
fn shape_water(map: &mut TileMap, surface_of: &[u8]) {
    for y in 0..map.h {
        for x in 0..map.w {
            let lvl = surface_of[(y * map.w + x) as usize];
            if lvl == 0 {
                continue;
            }
            map.with_tile_mut(x, y, |t| {
                t.kind = Kind::Water;
                t.corner = [lvl; 4];
                t.prop = Prop::None;
            });
        }
    }

    for y in 0..map.h {
        for x in 0..map.w {
            let t = map.at(x, y);
            if t.is_water() {
                continue;
            }
            let mut water_lvl = None;
            let mut sides: [bool; 4] = [false; 4];
            for (i, d) in [Dir::NegY, Dir::PosX, Dir::PosY, Dir::NegX]
                .iter()
                .enumerate()
            {
                let (dx, dy) = d.delta();
                let n = map.at(x + dx, y + dy);
                if n.is_water() {
                    water_lvl = Some(n.corner[0]);
                    sides[i] = true;
                }
            }
            let Some(wl) = water_lvl else { continue };
            if t.max_level() >= wl + 3 {
                continue; // bluff: the drop stays and becomes a cliff face
            }
            // Soft bank: the corners on the water side come down to the surface.
            map.with_tile_mut(x, y, |t| {
                let mut c = t.corner;
                for (i, on_water) in sides.iter().enumerate() {
                    if !on_water {
                        continue;
                    }
                    for corner in match i {
                        0 => [0, 1], // -y edge
                        1 => [1, 2], // +x edge
                        2 => [2, 3], // +y edge
                        _ => [3, 0], // -x edge
                    } {
                        c[corner] = c[corner].min(wl);
                    }
                }
                t.corner = c;
            });
        }
    }
}

/// Ramp the back rows of the world down to low ground (D43).
fn level_back_edge(map: &mut TileMap) {
    let q_min = 2; // the top corner of the world is tile (0,0) at q = 0
    let q_max = map.w + map.h;
    let flat_to = q_min as f32 + 0.10 * (q_max - q_min) as f32;
    let ramp_to = q_min as f32 + 0.20 * (q_max - q_min) as f32;
    for y in 0..map.h {
        for x in 0..map.w {
            let q = (x + y) as f32;
            if q > ramp_to {
                continue;
            }
            let t = ((q - flat_to) / (ramp_to - flat_to)).clamp(0.0, 1.0);
            // 3 levels at the very back, blending up to whatever is there.
            let cap = 3.0 + t * (MAX_LEVEL as f32 - 3.0);
            let mut tile = map.at(x, y);
            if tile.is_water() {
                continue;
            }
            let h = tile.max_level() as f32;
            if h > cap {
                let nv = cap.round() as u8;
                tile.corner = [nv; 4];
                map.set(x, y, tile);
            }
        }
    }
}

fn assign_materials(map: &mut TileMap, dry_seed: u64, beach_of: &[bool]) {
    for y in 0..map.h {
        for x in 0..map.w {
            let t = map.at(x, y);
            if t.is_water() {
                continue;
            }
            let near_water = Dir::ALL.iter().any(|d| {
                let (dx, dy) = d.delta();
                map.at(x + dx, y + dy).is_water()
            });
            // The beach: anything within a step of the sea is sand even when the
            // ground behind it is steep (a rocky coast would otherwise paint the
            // whole shoreline as scree).
            let sea_level = map.ocean.as_ref().map(|o| o.level).unwrap_or(0);
            let near_sea = map.ocean.is_some()
                && Dir::ALL.iter().any(|d| {
                    let (dx, dy) = d.delta();
                    map.water_kind(x + dx, y + dy) == crate::tiles::WaterKind::Sea
                });
            let hi = t.max_level();
            let relief = t.relief();
            // D30: grass covers almost everything. Rock only where the ground
            // is genuinely steep (cliff faces, summit caps), scree as the
            // transition, ochre grass only as small patches on sunny midslopes.
            let patch = fbm(x as f32 / 5.5, y as f32 / 5.5, dry_seed, 2);
            let central = in_central_frame(x, y, map.w, map.h);
            let beach = beach_of[(y * map.w + x) as usize];
            let biome = map.biome_at(x, y);
            let sand = beach || (near_sea && hi <= sea_level + 3);
            let kind = match biome {
                // D66: the dune sea. Low ground is bare sand, the mid slopes are
                // dry ochre scrub, and the high ground is red rock — the mesa.
                Biome::Desert => {
                    if sand || (near_water && hi <= 3) {
                        Kind::Sand
                    } else if !central && (relief >= 3 || hi >= 15) {
                        // The cap of a butte is bare rock; the bench below it is
                        // scree, which is the talus at the foot of the wall.
                        Kind::Rock
                    } else if !central && hi >= 9 {
                        Kind::Scree
                    } else if hi >= 9 {
                        Kind::DryGrass
                    } else if patch > 0.70 && hash2(x, y, dry_seed) < 0.30 {
                        // Sparse dry scrub islands on the sand, not a carpet.
                        Kind::DryGrass
                    } else if patch < 0.24 && hash2(x, y, dry_seed ^ 0x5EED) < 0.55 {
                        // Rocky flats: a pavement of flat stone out in the sand
                        // sea, which is what breaks a dune field into places
                        // (B46b item 4).
                        Kind::Scree
                    } else {
                        Kind::Sand
                    }
                }
                // D66: snow and bare rock. The palette makes grass snowfield and
                // sand the ice at a lake edge; the materials stay temperate so
                // fields, roads and the sim behave as they always did.
                Biome::Tundra => {
                    if sand || (near_water && hi <= 3) {
                        Kind::Sand
                    } else if !central && (relief >= 3 || hi >= 17) {
                        // B58b item 6: only a genuinely steep face or a summit cap
                        // is bare rock; a snowfield's high ground is snow, so the
                        // tundra reads as snow with crags instead of one grey mass.
                        Kind::Rock
                    } else if !central && hi >= 12 {
                        Kind::Scree
                    } else if hi >= 9 {
                        Kind::Upland
                    } else if hi >= 5 {
                        Kind::Meadow
                    } else {
                        Kind::Grass
                    }
                }
                // D66: jungle and beach. Wide pale sand at the water, lush green
                // everywhere else, rock only where the ground is genuinely steep.
                Biome::Tropical => {
                    if sand || (near_water && hi <= 4) {
                        Kind::Sand
                    } else if !central && (relief >= 3 || hi >= 16) {
                        Kind::Rock
                    } else if !central && hi >= 13 {
                        Kind::Scree
                    } else if hi >= 9 {
                        Kind::Upland
                    } else if hi >= 5 {
                        Kind::Meadow
                    } else {
                        Kind::Grass
                    }
                }
                // B46e: alpine. The valley floor is green, the slopes pasture,
                // the high ground snow (Upland wears the snow ramp here), rock
                // only on the steep faces.
                // B46e: savanna. Gold grass, straw and bare red earth in
                // patches, tawny rises, red laterite where it is steep.
                Biome::Savanna => {
                    if sand || (near_water && hi <= 3) {
                        Kind::Sand
                    } else if !central && (relief >= 3 || hi >= 15) {
                        Kind::Rock
                    } else if !central && (hi >= 12 || (hi >= 9 && relief >= 2)) {
                        Kind::Scree
                    } else if hi >= 9 {
                        Kind::Upland
                    } else if patch > 0.66 && hash2(x, y, dry_seed ^ 0x5A5A) < 0.60 {
                        // Bare red earth: the dust of the dry season.
                        Kind::Sand
                    } else if patch < 0.34 && hash2(x, y, dry_seed ^ 0x5A5B) < 0.55 {
                        Kind::DryGrass
                    } else if hi >= 5 {
                        Kind::Meadow
                    } else {
                        Kind::Grass
                    }
                }
                // B46e: swamp: dark green ground, reed-green hummocks, peat
                // patches and the odd mud flat; the water's edge is drawn as
                // mud by the shore pass, so the banks stay planted (reeds).
                Biome::Swamp => {
                    if sand {
                        Kind::Sand
                    } else if patch > 0.72 && hash2(x, y, dry_seed ^ 0x5A5C) < 0.45 {
                        Kind::Sand
                    } else if patch < 0.30 && hash2(x, y, dry_seed ^ 0x5A5D) < 0.55 {
                        Kind::DryGrass
                    } else if hi >= 4 || patch > 0.58 {
                        Kind::Meadow
                    } else {
                        Kind::Grass
                    }
                }
                // B46e: islands and lagoons: wide pale beaches, bright grass.
                Biome::Islands | Biome::Lagoon => {
                    if sand || (near_water && hi <= 4) {
                        Kind::Sand
                    } else if !central && (relief >= 3 || hi >= 15) {
                        Kind::Rock
                    } else if hi >= 7 {
                        Kind::Upland
                    } else if hi >= 4 {
                        Kind::Meadow
                    } else {
                        Kind::Grass
                    }
                }
                // B46e: highlands: pasture in the glens, rough moor above,
                // heather (the upland ramp) on the tops, granite where steep.
                Biome::Highlands => {
                    if sand || (near_water && hi <= 3) {
                        Kind::Sand
                    } else if !central && (relief >= 4 || hi >= 16) {
                        Kind::Rock
                    } else if !central && relief >= 3 && hi >= 10 {
                        Kind::Scree
                    } else if hi >= 9 {
                        Kind::Upland
                    } else if hi >= 5 {
                        Kind::Meadow
                    } else {
                        Kind::Grass
                    }
                }
                Biome::Alpine => {
                    if sand || (near_water && hi <= 3) {
                        Kind::Sand
                    } else if !central && (relief >= 4 || hi >= 17) {
                        Kind::Rock
                    } else if !central && (hi >= 15 || relief >= 3) {
                        Kind::Scree
                    } else if hi >= 15 {
                        Kind::Upland
                    } else if hi >= 6 {
                        Kind::Meadow
                    } else {
                        Kind::Grass
                    }
                }
                _ => {
                    if sand || (near_water && hi <= 3) {
                        Kind::Sand
                    } else if !central && (relief >= 3 || hi >= 16) {
                        Kind::Rock
                    } else if !central && (hi >= 14 || (hi >= 12 && relief >= 2)) {
                        Kind::Scree
                    } else if hi >= 9 {
                        Kind::Upland
                    } else if hi >= 5 {
                        Kind::Meadow
                    } else {
                        Kind::Grass
                    }
                }
            };
            // Ochre patches: small, scattered, only on mid slopes that are not
            // steep, and never more than a tenth of the hill.
            let kind = if kind == Kind::Meadow && patch > 0.66 && hash2(x, y, dry_seed) < 0.35 {
                Kind::DryGrass
            } else {
                kind
            };
            map.with_tile_mut(x, y, |t| t.kind = kind);
        }
    }

    demote_isolated_specks(map);

    // B58b item 6: scree is *talus* — the apron of broken rock at the foot of a
    // cliff, a tile or two wide — not a field of its own. Every scree tile more
    // than two steps from real rock goes back to the land's own soft ground, so a
    // tundra hillside is snow with grey aprons under the crags instead of a
    // quarter of the frame in gravel. A desert keeps a scatter of its flagged
    // stone pavements (they are not talus at all).
    let mut ring1: Vec<bool> = vec![false; (map.w * map.h) as usize];
    for y in 0..map.h {
        for x in 0..map.w {
            if map.at(x, y).kind != Kind::Scree {
                continue;
            }
            let near_rock = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1), (1, -1), (-1, 1)]
                .iter()
                .any(|(dx, dy)| map.at(x + dx, y + dy).kind == Kind::Rock);
            ring1[(y * map.w + x) as usize] = near_rock;
        }
    }
    let mut demote: Vec<(i32, i32)> = Vec::new();
    for y in 0..map.h {
        for x in 0..map.w {
            if map.at(x, y).kind != Kind::Scree {
                continue;
            }
            let i = (y * map.w + x) as usize;
            if ring1[i] {
                continue;
            }
            // The second ring: a ragged edge, and only sometimes.
            let second = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(dx, dy)| {
                    let (nx, ny) = (x + dx, y + dy);
                    (nx >= 0 && ny >= 0 && nx < map.w && ny < map.h)
                        && ring1[(ny * map.w + nx) as usize]
                });
            let desert = map.biome_at(x, y) == Biome::Desert;
            let keep = if desert {
                hash2(x, y, dry_seed ^ 0x5C2A) < 0.30
            } else if second {
                hash2(x, y, dry_seed ^ 0x5C2B) < 0.55
            } else {
                false
            };
            if !keep {
                demote.push((x, y));
            }
        }
    }
    for (x, y) in demote {
        let hi = map.at(x, y).max_level();
        let biome = map.biome_at(x, y);
        let k = match biome {
            Biome::Desert => {
                if hi >= 7 {
                    Kind::DryGrass
                } else {
                    Kind::Sand
                }
            }
            Biome::Tundra | Biome::Tropical => {
                if hi >= 9 {
                    Kind::Upland
                } else {
                    Kind::Meadow
                }
            }
            Biome::Alpine => {
                if hi >= 15 {
                    Kind::Upland
                } else if hi >= 6 {
                    Kind::Meadow
                } else {
                    Kind::Grass
                }
            }
            _ => {
                if hi >= 9 {
                    Kind::Upland
                } else {
                    Kind::Meadow
                }
            }
        };
        map.with_tile_mut(x, y, |t| t.kind = k);
    }
    // A scree tile that was the only grey neighbour of a rock leaves it alone in
    // the grass: the same speck rule runs again after the talus thinning.
    demote_isolated_specks(map);
}

/// Anything grey with no grey neighbour is a speck, not a cliff: demote it to
/// the land's own soft ground.
fn demote_isolated_specks(map: &mut TileMap) {
    for _ in 0..2 {
        let mut demote: Vec<(i32, i32)> = Vec::new();
        for y in 0..map.h {
            for x in 0..map.w {
                let t = map.at(x, y);
                if !t.kind.is_rocky() {
                    continue;
                }
                let touching = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| map.at(x + dx, y + dy).kind.is_rocky());
                if !touching {
                    demote.push((x, y));
                }
            }
        }
        if demote.is_empty() {
            break;
        }
        for (x, y) in demote {
            let hi = map.at(x, y).max_level();
            let k = match map.biome_at(x, y) {
                // A stray red boulder on sand is still desert, not meadow.
                Biome::Desert => {
                    if hi >= 7 {
                        Kind::DryGrass
                    } else {
                        Kind::Sand
                    }
                }
                _ => {
                    if hi >= 9 {
                        Kind::Upland
                    } else {
                        Kind::Meadow
                    }
                }
            };
            map.with_tile_mut(x, y, |t| t.kind = k);
        }
    }
}

fn plant(rng: &mut Rng, map: &mut TileMap, seed: u64) {
    let wood_seed = rng.next_u64();
    let bush_seed = rng.next_u64();
    let flat_seed = rng.next_u64();
    let river_seed = rng.next_u64();

    for y in 0..map.h {
        for x in 0..map.w {
            let t = map.at(x, y);
            // Nothing grows on the beach (B21)... except a palm, on a sea
            // land's beach (B46e).
            let sea_land = matches!(map.biome_at(x, y), Biome::Islands | Biome::Lagoon);
            if t.kind == Kind::Sand && sea_land && !t.is_water() {
                let p = if t.max_level() >= 2 { 0.30 } else { 0.06 };
                if hash2(x, y, seed ^ 0x9A1B) < p {
                    map.with_tile_mut(x, y, |t| {
                        t.prop = Prop::Tree {
                            species: if hash2(x, y, seed ^ 0x9A1C) > 0.3 { TreeSpecies::Palm } else { TreeSpecies::FanPalm },
                            size: (hash2(x, y, seed ^ 0x51) > 0.5) as u8,
                            variant: (hash2(x, y, seed ^ 0x52) * 8.0) as u8,
                        }
                    });
                }
                continue;
            }
            if t.is_water()
                || t.kind == Kind::Road
                || t.kind == Kind::Sidewalk
                || t.kind == Kind::Sand
            {
                continue;
            }
            let hi = t.max_level();
            let relief = t.relief();
            let h = hash2(x, y, seed ^ 0x7EE5);
            let biome = map.biome_at(x, y);

            // Woods: two scales of noise. The broad field makes masses and
            // clearings; the fine one breaks the straight edge of a wood.
            let wood = fbm(x as f32 / 6.5, y as f32 / 6.5, wood_seed, 3);
            let fine = fbm(x as f32 / 2.5, y as f32 / 2.5, river_seed, 2);

            // Forest cover by height (D30, D64): dense on the foothills and mid
            // slopes, thinning to conifers at the treeline, nothing above 14.
            // D64 asked for *much* denser wild land: real woods with clearings,
            // groves, and lone trees standing in the meadows. D66: how much grows
            // and how high the treeline sits is the land's, not the engine's.
            let treeline = match biome {
                Biome::Desert => 15,
                Biome::Tundra => 11,
                Biome::Tropical => 16,
                Biome::Alpine => 13,
                Biome::Savanna => 16,
                Biome::Highlands => 12,
                _ => 14,
            };
            let cover = if hi >= treeline {
                0.0
            } else if hi >= treeline - 2 {
                0.35 // treeline: sparse conifers
            } else if hi >= 5 {
                1.0 // hillside forest
            } else {
                0.93 // lowland wood
            };
            // D66: the land's own density. A dune sea is nearly bare, a jungle is
            // a wall of green, a tundra is a thin spruce wood on the rock.
            let land = match biome {
                Biome::Desert => 0.16,
                Biome::Tundra => 0.78,
                Biome::Tropical => 1.35,
                Biome::Savanna => 0.22,
                Biome::Swamp => 1.15,
                Biome::Islands | Biome::Lagoon => 0.80,
                Biome::Highlands => 0.30,
                _ => 1.0,
            };
            let cover = cover * land;
            // Masses: the wood field is taken to a hard edge with a smoothstep,
            // so a wood has a dense core and an open margin instead of a
            // uniform scatter of trees (D64: real forests with clearings).
            let m = ((wood - 0.36) / 0.12).clamp(0.0, 1.0);
            let mass = m * m * (3.0 - 2.0 * m);
            let in_wood = mass > 0.0;
            let mut p_tree = if in_wood {
                // Glades: the fine noise opens holes inside a wood.
                let glade = if fine < 0.26 { 0.55 } else { 1.0 };
                cover * mass * glade
            } else if hi < 5 && fine > 0.60 {
                0.28 // hedgerows and small copses on the plain
            } else if hi < 5 {
                // a lone tree in a meadow; on the plain they stand about
                if biome == Biome::Savanna { 0.07 } else { 0.03 }
            } else {
                0.0
            };
            if t.kind == Kind::Rock {
                p_tree = 0.0;
            } else if t.kind.is_rocky() {
                // Scree carries a scatter of conifers: a bare grey hillside the
                // size of a district reads as a hole in the world (B42).
                // B46c: in a jungle the green climbs the lower slopes, so a
                // tropical range is rock *in* vegetation, not bare grey.
                p_tree *= if biome == Biome::Tropical {
                    if hi >= 14 { 0.30 } else { 0.62 }
                } else if hi >= 11 {
                    0.35
                } else {
                    0.12
                };
            } else if relief >= 2 {
                p_tree *= 0.35;
            }

            let prop = if h < p_tree {
                let species = pick_species_in(map, biome, seed, x, y, hi, fine);
                Prop::Tree {
                    species,
                    size: if hash2(x, y, seed ^ 0x51) > 0.42 {
                        1
                    } else {
                        0
                    },
                    // Eight individuals per species before the shape repeats.
                    variant: (hash2(x, y, seed ^ 0x52) * 8.0) as u8,
                }
            } else if t.kind.is_rocky() && hash2(x, y, bush_seed) < 0.10 {
                Prop::Boulder((hash2(x, y, seed ^ 0x53) * 2.0) as u8)
            } else if t.kind.is_rocky() && hash2(x, y, bush_seed ^ 0x31) < 0.22 {
                Prop::Stone((hash2(x, y, seed ^ 0x5A) * 4.0) as u8)
            } else if !t.kind.is_rocky() && hash2(x, y, bush_seed) < bush_p(biome) {
                // Scrub: the desert's answer to a hedgerow, and the tundra's
                // dwarf willow.
                Prop::Bush((hash2(x, y, seed ^ 0x54) * 2.0) as u8)
            } else if near_field(map, x, y) && hash2(x, y, bush_seed ^ 0x32) < 0.42 {
                // Hedgerow: a bush along the edge of a field, so the farmland
                // reads as enclosed (D64).
                Prop::Bush((hash2(x, y, seed ^ 0x54) * 2.0) as u8)
            } else if biome == Biome::Highlands && near_field(map, x, y) && hash2(x, y, bush_seed ^ 0x32) < 0.80 {
                // A dry-stone wall along the field edge, as a run of stones.
                Prop::Boulder((hash2(x, y, seed ^ 0x53) * 2.0) as u8)
            } else if biome == Biome::Swamp
                && water_within(map, x, y, 2)
                && hash2(x, y, bush_seed ^ 0x33) < 0.60
            {
                // Reed beds along every channel and pool.
                Prop::Reeds((hash2(x, y, seed ^ 0x5B) * 3.0) as u8)
            } else if shore_edge(map, x, y) && hash2(x, y, bush_seed ^ 0x33) < 0.55 {
                Prop::Reeds((hash2(x, y, seed ^ 0x5B) * 3.0) as u8)
            } else if t.prop.is_tree() && hash2(x, y, bush_seed ^ 0x34) < 0.012 {
                // A fallen log among the trees.
                Prop::Log((hash2(x, y, seed ^ 0x5C) * 2.0) as u8)
            } else if t.kind.is_rocky() && hash2(x, y, bush_seed ^ 0x35) < 0.05 {
                Prop::Log((hash2(x, y, seed ^ 0x5D) * 2.0) as u8)
            } else if biome == Biome::Savanna && hash2(x, y, bush_seed ^ 0x38) < 0.04 {
                // A termite mound, a stone in the grass.
                Prop::Stone((hash2(x, y, seed ^ 0x5A) * 4.0) as u8)
            } else if biome == Biome::Savanna
                && !t.kind.is_rocky()
                && hi <= 8
                && fbm(x as f32 / 4.0, y as f32 / 4.0, flat_seed ^ 0x9E, 2) > 0.30
                && hash2(x, y, seed ^ 0x5E) < 0.34
            {
                // Tall dry grass: the savanna is mostly this.
                Prop::TallGrass((hash2(x, y, seed ^ 0x5F) * 3.0) as u8)
            } else if biome == Biome::Desert && hash2(x, y, bush_seed ^ 0x36) < 0.16 {
                // Dry scrub and loose sand: no meadow, no wildflowers, but the
                // dune face gets stones and a scatter of dry blades.
                if hash2(x, y, seed ^ 0x60) < 0.45 {
                    Prop::Stone((hash2(x, y, seed ^ 0x5A) * 4.0) as u8)
                } else {
                    Prop::TallGrass((hash2(x, y, seed ^ 0x5F) * 3.0) as u8)
                }
            } else if biome == Biome::Tundra && hash2(x, y, bush_seed ^ 0x37) < 0.14 {
                // Snowfield: bare rock, a stone poking through.
                Prop::Stone((hash2(x, y, seed ^ 0x5A) * 4.0) as u8)
            } else if biome != Biome::Desert
                && biome != Biome::Tundra
                && !t.kind.is_rocky()
                && hi <= 8
                && fbm(x as f32 / 4.0, y as f32 / 4.0, flat_seed ^ 0x9E, 2) > 0.46
                && hash2(x, y, seed ^ 0x5E) < 0.22
            {
                // Tall grass: patches in the meadows and along the verges.
                Prop::TallGrass((hash2(x, y, seed ^ 0x5F) * 3.0) as u8)
            } else if biome != Biome::Desert
                && biome != Biome::Tundra
                && t.is_flat()
                && hi <= 4
                && fbm(x as f32 / 5.0, y as f32 / 5.0, flat_seed, 2) > 0.54
                && hash2(x, y, seed ^ 0x55) < 0.30
            {
                // Wildflowers: the meadow's colour, in the land's own bloom.
                Prop::Meadow((hash2(x, y, seed ^ 0x56) * 3.0) as u8)
            } else {
                Prop::None
            };
            map.with_tile_mut(x, y, |t| t.prop = prop);
        }
    }
}

/// Is this tile on the edge of a ploughed field? (A hedgerow goes there.)
fn near_field(map: &TileMap, x: i32, y: i32) -> bool {
    if map.at(x, y).kind == Kind::Field {
        return false;
    }
    Dir::ALL.iter().any(|d| {
        let (dx, dy) = d.delta();
        map.in_bounds(x + dx, y + dy) && map.at(x + dx, y + dy).kind == Kind::Field
    })
}

/// Is there water within `r` tiles?
fn water_within(map: &TileMap, x: i32, y: i32, r: i32) -> bool {
    (-r..=r).any(|dy| {
        (-r..=r).any(|dx| map.in_bounds(x + dx, y + dy) && map.at(x + dx, y + dy).is_water())
    })
}

/// Is this tile the water's edge? (Reeds and cattails grow there.)
fn shore_edge(map: &TileMap, x: i32, y: i32) -> bool {
    let t = map.at(x, y);
    if t.is_water() || t.kind == Kind::Road || t.kind == Kind::Sidewalk {
        return false;
    }
    Dir::ALL.iter().any(|d| {
        let (dx, dy) = d.delta();
        map.in_bounds(x + dx, y + dy) && map.at(x + dx, y + dy).is_water()
    })
}

/// Bush / scrub density for a land (the temperate rate is 0.09).
fn bush_p(biome: Biome) -> f32 {
    match biome {
        Biome::Desert => 0.16,
        Biome::Tundra => 0.05,
        Biome::Tropical => 0.14,
        Biome::Savanna => 0.11,
        // Heather: the moor is mostly this.
        Biome::Highlands => 0.30,
        Biome::Swamp => 0.12,
        _ => 0.09,
    }
}

/// Which species grows on this tile (D64), for the land it is in (D66).
fn pick_species_in(
    map: &TileMap,
    biome: Biome,
    seed: u64,
    x: i32,
    y: i32,
    hi: u8,
    fine: f32,
) -> TreeSpecies {
    let r = hash2(x, y, seed ^ 0x57);
    let near_water = (-2..=2).any(|dy| {
        (-2..=2).any(|dx| {
            let nx = x + dx;
            let ny = y + dy;
            map.in_bounds(nx, ny) && map.at(nx, ny).is_water()
        })
    });
    match biome {
        // D66: cacti and scrub on the sand, palms only at an oasis.
        Biome::Desert => {
            if near_water {
                // The oasis: a stand of palms and a little shade.
                if r > 0.35 {
                    return TreeSpecies::Palm;
                }
                return TreeSpecies::Cypress;
            }
            // Cacti, Joshua trees and the odd cypress: the desert's three
            // silhouettes, none of them a leafy crown.
            if r < 0.52 {
                TreeSpecies::Cactus
            } else if r < 0.78 {
                TreeSpecies::Joshua
            } else if r < 0.90 {
                TreeSpecies::Cypress
            } else {
                TreeSpecies::Palm
            }
        }
        // D66: spruce and fir, thinning to bare rock; no blossom, no orchard.
        Biome::Tundra => {
            if near_water && r > 0.55 {
                return TreeSpecies::Willow;
            }
            match (r * 100.0) as u32 % 5 {
                0 | 1 => TreeSpecies::Spruce,
                2 => TreeSpecies::Fir,
                3 => TreeSpecies::Pine,
                _ => TreeSpecies::Cedar,
            }
        }
        // B46e: swamp. Bald cypress standing in the water, mangrove and willow
        // at the edge, a few live oaks on the hummocks.
        Biome::Swamp => {
            if near_water {
                return if r < 0.55 {
                    TreeSpecies::BaldCypress
                } else if r < 0.80 {
                    TreeSpecies::Mangrove
                } else {
                    TreeSpecies::Willow
                };
            }
            if r < 0.50 {
                TreeSpecies::BaldCypress
            } else if r < 0.72 {
                TreeSpecies::Willow
            } else {
                TreeSpecies::Oak
            }
        }
        // B46e: islands and lagoons: coconut palms, a fan palm, a flamboyant,
        // and broadleaves inland.
        Biome::Islands | Biome::Lagoon => {
            if near_water && r > 0.50 {
                return if r > 0.80 { TreeSpecies::Mangrove } else { TreeSpecies::Palm };
            }
            if r > 0.88 {
                TreeSpecies::Flamboyant
            } else if r > 0.74 {
                TreeSpecies::FanPalm
            } else if r > 0.50 {
                TreeSpecies::Palm
            } else {
                match ((fine * 3.0) as u32 + (r * 3.0) as u32) % 3 {
                    0 => TreeSpecies::Beech,
                    1 => TreeSpecies::Oak,
                    _ => TreeSpecies::Elm,
                }
            }
        }
        // B46e: highlands. Scots pine and birch in the glens, nothing on the top.
        Biome::Highlands => {
            if near_water && r > 0.6 {
                return TreeSpecies::Willow;
            }
            match (r * 100.0) as u32 % 5 {
                0 | 1 => TreeSpecies::Pine,
                2 | 3 => TreeSpecies::Birch,
                _ => TreeSpecies::Spruce,
            }
        }
        // B46e: savanna. Acacias, an occasional baobab, a doum palm at the
        // water; nothing leafy and round.
        Biome::Savanna => {
            if near_water && r > 0.70 {
                return TreeSpecies::Palm;
            }
            if r < 0.11 {
                TreeSpecies::Baobab
            } else {
                TreeSpecies::Acacia
            }
        }
        // B46e: alpine. Spruce and fir on the slopes, larch-like cedar and pine
        // up to the treeline, a few birch and willows down on the valley floor.
        Biome::Alpine => {
            if near_water && r > 0.62 {
                return TreeSpecies::Willow;
            }
            if hi <= 4 && r > 0.80 {
                return TreeSpecies::Birch;
            }
            match (r * 100.0) as u32 % 6 {
                0 | 1 | 2 => TreeSpecies::Spruce,
                3 => TreeSpecies::Fir,
                4 => TreeSpecies::Pine,
                _ => TreeSpecies::Cedar,
            }
        }
        // D66: palms on the sand, dense broadleaf jungle behind, cypress in the
        // wet ground.
        Biome::Tropical => {
            // The shore is mangroves and palms; the interior is a jungle of
            // broadleaves with two palms and a flowering tree mixed in.
            if near_water && r > 0.58 {
                return TreeSpecies::Mangrove;
            }
            if near_water && r > 0.30 {
                return TreeSpecies::Palm;
            }
            if r > 0.90 {
                TreeSpecies::Palm
            } else if r > 0.83 {
                TreeSpecies::FanPalm
            } else if r > 0.76 {
                TreeSpecies::Flamboyant
            } else {
                match ((fine * 4.0) as u32 + (r * 3.0) as u32) % 4 {
                    0 => TreeSpecies::Beech,
                    1 => TreeSpecies::Elm,
                    2 => TreeSpecies::Oak,
                    _ => TreeSpecies::Poplar,
                }
            }
        }
        _ => pick_species(map, seed, x, y, hi, fine),
    }
}

/// Which species grows on this tile (D64).
///
/// Height decides the conifer share (pines and spruces take the hillside, firs
/// and cedars the treeline), water within a tile or two gets a willow, flat
/// lowland near the frame picks up orchard rows of apple and cherry, and the
/// rest is the temperate broadleaf mix. Every branch is a pure function of the
/// coordinates, so a seed replays exactly.
fn pick_species(map: &TileMap, seed: u64, x: i32, y: i32, hi: u8, fine: f32) -> TreeSpecies {
    let r = hash2(x, y, seed ^ 0x57);
    // Water bank: willows lean over the river and the pond.
    let near_water = (-2..=2).any(|dy| {
        (-2..=2).any(|dx| {
            let nx = x + dx;
            let ny = y + dy;
            map.in_bounds(nx, ny) && map.at(nx, ny).is_water()
        })
    });
    if near_water && r > 0.45 {
        return TreeSpecies::Willow;
    }
    // Orchard rows on flat low ground: a grid, so the rows read as planted.
    if hi <= 4 {
        let orchard = fbm(x as f32 / 9.0, y as f32 / 9.0, seed ^ 0x0AC4, 2);
        if orchard > 0.62 && x.rem_euclid(3) == 0 && y.rem_euclid(3) == 0 {
            return if hash2(x / 3, y / 3, seed ^ 0x0AC5) > 0.5 {
                TreeSpecies::Apple
            } else {
                TreeSpecies::Cherry
            };
        }
    }
    let conifer = if hi >= 11 {
        r < 0.85
    } else if hi >= 7 {
        r < 0.55
    } else if hi >= 5 {
        r < 0.30
    } else {
        r < 0.10
    };
    if conifer {
        // Higher up: fir and cedar on the treeline, pine and spruce below.
        return match (r * 100.0) as u32 % 4 {
            0 => TreeSpecies::Pine,
            1 => TreeSpecies::Spruce,
            2 => TreeSpecies::Fir,
            _ => {
                if hi >= 9 {
                    TreeSpecies::Cedar
                } else {
                    TreeSpecies::Pine
                }
            }
        };
    }
    // Broadleaf mix; the groves are seeded so a wood has one dominant species.
    let grove = fbm(x as f32 / 14.0, y as f32 / 14.0, seed ^ 0x6207, 2);
    let pick = ((fine * 5.0) as u32 + (grove * 3.0) as u32) % 6;
    match pick {
        0 => TreeSpecies::Oak,
        1 => TreeSpecies::Maple,
        2 => TreeSpecies::Birch,
        3 => TreeSpecies::Beech,
        4 => TreeSpecies::Elm,
        _ => TreeSpecies::Poplar,
    }
}

#[cfg(test)]

mod tests {
    use super::*;

    /// B61 item 4: a world smaller than the generator can make is an error, not
    /// an assert that takes the process down (the spike used to panic on
    /// `--sim --w 40 --h 40`, where the map is sized from the framebuffer).
    #[test]
    fn a_map_too_small_is_an_error_not_a_panic() {
        for (w, h) in [(15, 40), (40, 15), (7, 5), (0, 0), (-3, 20)] {
            let e = try_generate_with(7, w, h, None);
            assert!(e.is_err(), "{w}x{h} should be refused");
        }
        assert!(matches!(
            try_generate_with(7, 15, 40, None),
            Err(MapError::TooSmall { .. })
        ));
        assert!(matches!(
            try_generate_with(7, 0, 0, None),
            Err(MapError::BadSize { .. })
        ));
        assert!(try_generate_with(7, MIN_SIDE, MIN_SIDE, None).is_ok());
        assert_eq!(
            try_generate_with(7, 15, 40, None).unwrap_err().to_string(),
            "map 15x40 is too small: a world needs at least 16 tiles a side"
        );
        // Whatever the frame size, a viewport is a world the generator accepts.
        for (fw, fh) in [(7, 5), (40, 40), (1, 1), (960, 540), (7040, 1440)] {
            let vp = crate::world_for_viewport(fw, fh);
            assert!(vp.w >= MIN_SIDE && vp.h >= MIN_SIDE, "{fw}x{fh} -> {:?}", vp);
            assert!(try_generate_with(7, vp.w, vp.h, None).is_ok());
        }
    }

    /// B27 item 3: a coastal map's sea may cover at most a quarter of the land
    /// (H24-icons.png was half ocean).
    #[test]
    fn sea_covers_at_most_a_quarter_of_the_land() {
        let mut coastal = 0;
        let mut worst: f32 = 0.0;
        for seed in 1..=60u64 {
            let m = generate_with(seed, 71, 71, Some(Biome::Temperate)).unwrap();
            if m.ocean.is_none() {
                continue;
            }
            coastal += 1;
            let mut sea = 0;
            for y in 0..m.h {
                for x in 0..m.w {
                    if m.water_kind(x, y) == crate::tiles::WaterKind::Sea {
                        sea += 1;
                    }
                }
            }
            let frac = sea as f32 / (m.w * m.h) as f32;
            worst = worst.max(frac);
            assert!(frac <= 0.26, "seed {seed}: sea is {:.0} % of the map", frac * 100.0);
        }
        assert!(coastal > 8, "only {coastal} coastal maps in 60 seeds");
        println!("worst coastal sea: {:.1} %", worst * 100.0);
    }

    fn make(seed: u64) -> TileMap {
        generate_with(seed, 66, 66, Some(Biome::Temperate)).unwrap()
    }

    #[test]
    fn deterministic_per_seed() {
        let a = make(7);
        let b = make(7);
        assert_eq!(a.tiles, b.tiles);
    }

    #[test]
    fn different_seeds_differ() {
        let a = make(7);
        let b = make(8);
        assert!(
            a.tiles
                .iter()
                .zip(b.tiles.iter())
                .any(|(x, y)| x.corner != y.corner)
        );
    }

    #[test]
    fn levels_are_in_range() {
        for seed in [1u64, 7, 42, 1234] {
            let m = make(seed);
            assert!(m.tiles.iter().all(|t| t.max_level() <= MAX_LEVEL));
        }
    }

    #[test]
    fn vertex_constraint_holds_except_at_cliffs() {
        for seed in [1u64, 7, 42, 99, 1234] {
            let m = make(seed);
            let mut ok = 0;
            let mut cliff = 0;
            for y in 0..m.h - 1 {
                for x in 0..m.w - 1 {
                    // Compare the two tiles sharing the edge y+1 .. and x+1.
                    for d in [Dir::PosX, Dir::PosY] {
                        let (dx, dy) = d.delta();
                        let a = m.edge_levels(x, y, d);
                        let b = m.edge_levels(x + dx, y + dy, d.opposite());
                        let diff = (a[0] as i32 - b[1] as i32)
                            .abs()
                            .max((a[1] as i32 - b[0] as i32).abs());
                        if diff >= 2 {
                            cliff += 1;
                        } else {
                            ok += 1;
                            assert!(diff <= 1, "seed {seed}: vertex step {diff} at {x},{y}");
                        }
                    }
                }
            }
            assert!(
                ok > cliff * 4,
                "seed {seed}: too many cliff edges ({cliff})"
            );
        }
    }

    /// D44/B21 item 2: over 100 seeds the mix is within ±10 points of the
    /// target, every map has a river that reaches water or the map edge, the sea
    /// never takes the founding frame, and coastal rivers end in the sea.
    #[test]
    fn water_layout_mix_and_connectivity() {
        let n = 100u64;
        let mut counts = std::collections::HashMap::new();
        for seed in 1..=n {
            let m = generate_with(seed, 71, 71, Some(Biome::Temperate)).unwrap();
            *counts.entry(m.layout).or_insert(0usize) += 1;

            let r = m.river.as_ref().expect("every world has a river");
            // The river reaches the sea (or a lake/pond) or leaves the map.
            let reaches = r.exit_edge.contains(r.exit.0, r.exit.1, m.w, m.h)
                || mouth_at_sea(&m)
                || m.lake
                    .as_ref()
                    .is_some_and(|l| water_path_exists(&m, r.entry, l.centre))
                || m.ponds
                    .iter()
                    .any(|p| water_path_exists(&m, r.entry, p.centre));
            assert!(reaches, "seed {seed}: river reaches no water body or edge");

            if m.layout.is_coastal() {
                let o = m.ocean.as_ref().expect("coastal map without a sea");
                // D44: on a coast the river should reach the sea; the brief's
                // hard rule is "a lake/sea or the map edge", which is looser.
                let mouth_ok = mouth_at_sea(&m)
                    || r.exit_edge.contains(r.exit.0, r.exit.1, m.w, m.h);
                assert!(mouth_ok, "seed {seed}: river reaches neither the sea nor an edge");
                let _ = o;
            } else {
                assert!(m.ocean.is_none(), "seed {seed}: stray sea");
            }

            // The village always has land: plenty of flat buildable ground near
            // the frame's centre, whatever water the layout picked (D35/D44).
            let (cx, cy) = (m.w / 2, m.h / 2);
            let mut buildable = 0;
            for dy in -8..=8 {
                for dx in -8..=8 {
                    if dx * dx + dy * dy <= 64 && m.flat_buildable(cx + dx, cy + dy) {
                        buildable += 1;
                    }
                }
            }
            assert!(
                buildable >= 40,
                "seed {seed} ({}): only {buildable} buildable tiles near the centre",
                m.layout.name()
            );
            // ... and the sea stays a coastal strip: under 40 % of the map, so
            // it cannot reach the founding spot at the centre.
            if let Some(o) = &m.ocean {
                let frac = o.tiles as f32 / (m.w * m.h) as f32;
                assert!(frac < 0.40, "seed {seed}: sea covers {:.0} % of the map", frac * 100.0);
            }
            if m.layout.is_coastal() {
                let o = m.ocean.as_ref().unwrap();
                assert!(o.tiles > 60, "seed {seed}: sea is a puddle ({} tiles)", o.tiles);
                // A visible beach between the water and the grass.
                let mut sand = 0;
                for y in 0..m.h {
                    for x in 0..m.w {
                        if m.at(x, y).kind != Kind::Sand {
                            continue;
                        }
                        if Dir::ALL.iter().any(|d| {
                            let (dx, dy) = d.delta();
                            m.at(x + dx, y + dy).is_water()
                        }) {
                            sand += 1;
                        }
                    }
                }
                assert!(sand > 20, "seed {seed}: no beach on the sea ({sand} tiles)");
            }
        }
        let pct = |l: WaterLayout| counts.get(&l).copied().unwrap_or(0) as f32 / n as f32;
        for l in WaterLayout::ALL {
            println!("{:>6}: {:>3} / {n} = {:>3.0} %", l.name(), counts[&l], pct(l) * 100.0);
        }
        // Targets: 30 % coast (coast + delta), 40 % lake, within ±10 points.
        let coastal = (pct(WaterLayout::Coast) + pct(WaterLayout::Delta)) * 100.0;
        let lake = pct(WaterLayout::RiverLake) * 100.0;
        assert!((20.0..=40.0).contains(&coastal), "coastal {coastal:.0} %");
        assert!((30.0..=50.0).contains(&lake), "lake {lake:.0} %");
        for l in WaterLayout::ALL {
            assert!(counts.contains_key(&l), "layout {} never happened", l.name());
        }
    }

    #[test]
    fn river_connects_two_edges() {
        for seed in [1u64, 7, 42, 99, 1234] {
            let m = make(seed);
            let r = m.river.as_ref().unwrap();
            assert!(
                r.entry_edge.contains(r.entry.0, r.entry.1, m.w, m.h),
                "seed {seed}: entry off its edge"
            );
            assert!(
                r.exit_edge.contains(r.exit.0, r.exit.1, m.w, m.h) || mouth_at_sea(&m),
                "seed {seed}: exit off its edge and not at the sea"
            );
            if !m.layout.is_coastal() {
                assert_ne!(r.entry_edge, r.exit_edge, "seed {seed}: same edge");
            }
            assert!(
                water_path_exists(&m, r.entry, r.exit),
                "seed {seed}: river broken"
            );
        }
    }

    /// True when the river's last tile sits against the sea (a coastal mouth).
    fn mouth_at_sea(m: &TileMap) -> bool {
        let Some(r) = &m.river else { return false };
        use crate::tiles::WaterKind::Sea;
        m.water_kind(r.exit.0, r.exit.1) == Sea
            || Dir::ALL.iter().any(|d| {
                let (dx, dy) = d.delta();
                m.water_kind(r.exit.0 + dx, r.exit.1 + dy) == Sea
            })
    }

    fn water_path_exists(m: &TileMap, from: (i32, i32), to: (i32, i32)) -> bool {
        let mut seen = vec![false; (m.w * m.h) as usize];
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(from);
        seen[(from.1 * m.w + from.0) as usize] = true;
        while let Some((x, y)) = queue.pop_front() {
            if (x, y) == to {
                return true;
            }
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x + dx, y + dy);
                if !m.in_bounds(nx, ny) {
                    continue;
                }
                let i = (ny * m.w + nx) as usize;
                if seen[i] || !m.at(nx, ny).is_water() {
                    continue;
                }
                seen[i] = true;
                queue.push_back((nx, ny));
            }
        }
        false
    }

    #[test]
    fn river_never_flows_uphill() {
        for seed in 1u64..=16 {
            let m = make(seed);
            let r = m.river.as_ref().unwrap();
            for i in 1..r.surface.len() {
                assert!(r.surface[i] <= r.surface[i - 1], "seed {seed}: river rose");
            }
            for (i, &(x, y)) in r.path.iter().enumerate() {
                let t = m.at(x, y);
                if !t.is_water() {
                    continue; // a bank tile of a wide river is fine
                }
                assert!(t.corner[0] <= r.surface[i.min(r.surface.len() - 1)] + 1);
            }
        }
    }

    #[test]
    fn every_seed_has_a_visible_mountain() {
        // D31: at least one visible tile reaches level 14 (the treeline is 13).
        for seed in [1u64, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
            let m = make(seed);
            let mut best = 0;
            for y in 0..m.h {
                for x in 0..m.w {
                    let p = x - y;
                    let q = x + y;
                    if p.abs() <= 30 && (30..=98).contains(&q) {
                        best = best.max(m.at(x, y).max_level());
                    }
                }
            }
            assert!(
                best >= 14,
                "seed {seed}: highest visible tile is level {best}"
            );
        }
    }

    /// B58b item 6: scree is talus — the apron at the foot of a cliff — so the
    /// tundra's high ground cannot turn into a field of gravel.
    #[test]
    fn scree_is_talus_not_a_field() {
        for seed in [1u64, 2, 3, 7, 42] {
            let m = make(seed);
            let mut scree = 0;
            for y in 2..m.h - 2 {
                for x in 2..m.w - 2 {
                    if m.at(x, y).kind != Kind::Scree {
                        continue;
                    }
                    scree += 1;
                    // Within two steps of real rock, or a desert's flagged
                    // pavement (which is not talus at all).
                    let mut near_rock = false;
                    for dy in -2..=2i32 {
                        for dx in -2..=2i32 {
                            if m.at(x + dx, y + dy).kind == Kind::Rock {
                                near_rock = true;
                            }
                        }
                    }
                    let desert = m.biome_at(x, y) == Biome::Desert;
                    assert!(
                        near_rock || desert,
                        "seed {seed}: scree at {x},{y} is {:.0} tiles from any rock",
                        2.0
                    );
                }
            }
            let share = scree as f64 / (m.w * m.h) as f64;
            assert!(
                share < 0.06,
                "seed {seed}: scree covers {:.1} % of the map",
                share * 100.0
            );
        }
    }

    #[test]
    fn no_rock_in_the_middle_of_the_frame() {
        // D31/B9: mountains live at the frame's edges; the village's ground
        // (central 40 %) stays green.
        for seed in [1u64, 2, 3, 4, 5] {
            let m = make(seed);
            for y in 0..m.h {
                for x in 0..m.w {
                    if !in_central_frame(x, y, m.w, m.h) {
                        continue;
                    }
                    let k = m.at(x, y).kind;
                    assert!(
                        !k.is_rocky(),
                        "seed {seed}: {k:?} in the frame centre at {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn materials_have_no_isolated_patches() {
        // A grey tile surrounded by green reads as a hole in the terrain: every
        // rock/scree tile must touch another one.
        for seed in [1u64, 2, 3, 4, 7, 42] {
            let m = make(seed);
            for y in 1..m.h - 1 {
                for x in 1..m.w - 1 {
                    let k = m.at(x, y).kind;
                    if !k.is_rocky() {
                        continue;
                    }
                    let touching = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|(dx, dy)| m.at(x + dx, y + dy).kind.is_rocky());
                    assert!(touching, "seed {seed}: isolated {k:?} at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn seeds_have_hills_and_buildable_centre() {
        for seed in [1u64, 2, 3, 4, 7, 42, 99] {
            let m = make(seed);
            // The frame's visible diamond, in p/q form.
            let mut visible = 0;
            let mut high = 0;
            for y in 0..m.h {
                for x in 0..m.w {
                    let p = x - y;
                    let q = x + y;
                    if p.abs() <= 30 && (30..=98).contains(&q) {
                        visible += 1;
                        if m.at(x, y).max_level() >= 6 {
                            high += 1;
                        }
                    }
                }
            }
            let frac = high as f32 / visible as f32;
            assert!(
                frac >= 0.15,
                "seed {seed}: only {:.1}% of visible tiles are high",
                frac * 100.0
            );

            // Central 40% of the frame must be flat-buildable land.
            let mut central = 0;
            let mut buildable = 0;
            for y in 0..m.h {
                for x in 0..m.w {
                    let p = x - y;
                    let q = x + y;
                    if p.abs() <= 12 && (50..=76).contains(&q) {
                        central += 1;
                        if m.flat_buildable(x, y) {
                            buildable += 1;
                        }
                    }
                }
            }
            let cf = buildable as f32 / central.max(1) as f32;
            assert!(
                cf >= 0.30,
                "seed {seed}: only {:.1}% central flat-buildable",
                cf * 100.0
            );
        }
    }
}
