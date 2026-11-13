//! B74 round 5: draw-time cleaning and planting, in the renderer.
//!
//! The sim's map is left untouched (rounds 1-4 showed that any prop state the
//! sim writes breaks save round-trips and growth). Everything that used to be a
//! sim-side pass now happens here, where the frame is drawn, from the map, the
//! lots and a seed:
//!
//! * a **claims bitmap** that says, per tile, what the ground is claimed by (a
//!   road, a rail bed, a pavement, a plaza, a wall, a lot, the airfield);
//! * **draw-time cleaning**: a stored terrain prop whose footprint/crown would
//!   touch a road, a rail bed, a paving, a plaza or a wall is skipped (a small
//!   species may stand a pixel off a wall, so it is kept in the margin);
//! * **derived trees**: the garden trees (one or two small species per house
//!   lot, on its own margin) and the street trees (every three verge tiles,
//!   one species per street) — computed from the map + lots + seed, never
//!   stored, so a save round-trip cannot see them.
//!
//! The render crate cannot depend on the sim, so the shared rules (`is_small`,
//! `GARDEN`, `STREET_PITCH`) live in `map::treeline`.

use map::{Kind, Prop, TileMap, treeline};
use crate::city::{CityObj, ObjKind, Stage};
use crate::rail::Fixtures;

/// Claim bits per tile (a tile may wear several).
pub const C_ROAD: u8 = 1 << 0; // a lane
pub const C_RAIL: u8 = 1 << 1; // rail bed, tram, quay
pub const C_PAVING: u8 = 1 << 2; // a pavement (Sidewalk)
pub const C_PLAZA: u8 = 1 << 3; // a paved civic square
pub const C_WALL: u8 = 1 << 4; // a building's wall (the lot inset)
pub const C_LOT: u8 = 1 << 5; // any building lot's tile (yard + walls)
pub const C_AIRFIELD: u8 = 1 << 6; // runway / apron / taxiway
pub const C_WATER: u8 = 1 << 7; // water (from the map)

/// Ground a prop may not stand on (the brief's road, rail bed, paving, plaza,
/// plus the airfield, which is the same "nothing grows on the runway" rule).
pub const C_HARD: u8 = C_ROAD | C_RAIL | C_PAVING | C_PLAZA | C_AIRFIELD;

/// How far the walls are inset from the lot edge, in tiles (matches the sim's
/// `SETBACK` and the renderer's `INSET`).
const INSET: f32 = 0.13;

/// A lot with walls on it (as opposed to a green, a graveyard, a car park or a
/// plaza).
fn has_walls(kind: ObjKind) -> bool {
    !matches!(
        kind,
        ObjKind::Park
            | ObjKind::Cemetery
            | ObjKind::CarPark
            | ObjKind::Landmark(crate::city::LandmarkKind::Plaza)
    )
}

/// Does a house lot get a garden tree? The small-town homes the brief means.
fn is_garden_lot(kind: ObjKind) -> bool {
    matches!(kind, ObjKind::House | ObjKind::Townhouse | ObjKind::LogCabin)
}

/// Build the claim bitmap from the map, the lots and the fixtures.
pub fn build_claims(map: &TileMap, city: &[CityObj], fixtures: &Fixtures) -> Vec<u8> {
    let (w, h) = (map.w as usize, map.h as usize);
    let n = w * h;
    let mut cells = vec![0u8; n];
    let mut set = |cells: &mut Vec<u8>, x: i32, y: i32, bit: u8| {
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            cells[y as usize * w + x as usize] |= bit;
        }
    };

    for y in 0..map.h {
        for x in 0..map.w {
            let t = map.at(x, y);
            let mut c = 0u8;
            if t.kind.is_lane() {
                c |= C_ROAD;
            }
            if t.kind == Kind::Sidewalk {
                c |= C_PAVING;
            }
            if t.is_water() {
                c |= C_WATER;
            }
            cells[(y * map.w + x) as usize] = c;
        }
    }

    for o in city {
        let plaza = matches!(o.kind, ObjKind::Landmark(crate::city::LandmarkKind::Plaza));
        let walled = has_walls(o.kind);
        // The wall box: the lot pulled in by INSET on every side, decided by the
        // tile's centre (the same test the sim's audit uses).
        let (x0, y0, x1, y1) = (o.x as f32, o.y as f32, (o.x + o.w) as f32, (o.y + o.h) as f32);
        let mx = (o.w as f32 * INSET).max(0.25);
        let my = (o.h as f32 * INSET).max(0.25);
        let (wx0, wx1) = (x0 + mx, x1 - mx);
        let (wy0, wy1) = (y0 + my, y1 - my);
        for ty in o.y..o.y + o.h {
            for tx in o.x..o.x + o.w {
                if plaza {
                    set(&mut cells, tx, ty, C_PLAZA | C_LOT);
                    continue;
                }
                if walled {
                    set(&mut cells, tx, ty, C_LOT);
                    let (cx, cy) = (tx as f32 + 0.5, ty as f32 + 0.5);
                    if cx > wx0 && cx < wx1 && cy > wy0 && cy < wy1 {
                        set(&mut cells, tx, ty, C_WALL);
                    }
                }
            }
        }
    }

    // The rail bed and its ground: the line, the tram, the second track, the
    // sidings, the platforms, the yard and the station's own ground.
    for &(x, y) in &fixtures.track {
        set(&mut cells, x, y, C_RAIL);
    }
    for &(x, y) in &fixtures.tram_line {
        set(&mut cells, x, y, C_RAIL);
    }
    for &(x, y) in &fixtures.transit.twin {
        set(&mut cells, x, y, C_RAIL);
    }
    for &(x, y) in &fixtures.transit.sidings {
        set(&mut cells, x, y, C_RAIL);
    }
    for &(x, y, _) in &fixtures.transit.platforms {
        set(&mut cells, x, y, C_RAIL);
    }
    if let Some((x, y)) = fixtures.station {
        set(&mut cells, x, y, C_RAIL);
    }
    if let Some((x, y, _)) = fixtures.station_platform {
        set(&mut cells, x, y, C_RAIL);
    }
    for r in [fixtures.yard, fixtures.transit.station_build, fixtures.transit.depot]
        .into_iter()
        .flatten()
    {
        let (rx, ry, rw, rh) = r;
        for ty in ry..ry + rh {
            for tx in rx..rx + rw {
                set(&mut cells, tx, ty, C_RAIL);
            }
        }
    }
    // The airport's pavement and the harbour's quay are built ground too.
    if let Some(ap) = &fixtures.transit.airport {
        for &(x, y) in &ap.paved {
            set(&mut cells, x, y, C_AIRFIELD);
        }
        for &(x, y) in &ap.runway {
            set(&mut cells, x, y, C_AIRFIELD);
        }
        for &(x, y) in &ap.taxiway {
            set(&mut cells, x, y, C_AIRFIELD);
        }
    }
    if let Some(h) = &fixtures.transit.harbour {
        for &(x, y) in &h.quay {
            set(&mut cells, x, y, C_RAIL);
        }
    }
    cells
}

/// Is this tile open ground a derived tree could stand on? Not a street, a
/// pavement, a lot, the rail bed, the airfield, water, a field or a steep bank.
fn plantable(map: &TileMap, claims: &[u8], x: i32, y: i32) -> bool {
    if !map.in_bounds(x, y) {
        return false;
    }
    let idx = (y * map.w + x) as usize;
    if claims.get(idx).is_some_and(|&c| c & (C_HARD | C_LOT | C_WATER) != 0) {
        return false;
    }
    let t = map.at(x, y);
    if t.kind.is_lane()
        || t.kind == Kind::Sidewalk
        || t.is_water()
        || t.kind.is_worked()
        || t.relief() > 2
        || t.prop != Prop::None
    {
        return false;
    }
    true
}

/// Is a walled lot's tile within `r` of this tile? (A street tree keeps out of a
/// yard and out of the drive to a door.)
fn lot_within(city: &[CityObj], x: i32, y: i32, r: i32) -> bool {
    city.iter().any(|o| {
        has_walls(o.kind)
            && x >= o.x - r
            && x < o.x + o.w + r
            && y >= o.y - r
            && y < o.y + o.h + r
    })
}

/// A tiny deterministic hash for the garden tree's side/along pick (the
/// renderer has no sim Rng, so it derives the two numbers itself).
fn hash64(a: u64, b: u64) -> u64 {
    let mut x = a ^ (b.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// The derived garden and street trees, one entry per tile that carries one
/// (`None` elsewhere). Deterministic from the map, the lots and the lot seeds,
/// so the same frame draws the same trees every time and nothing is stored in
/// the sim.
pub fn derive_trees(map: &TileMap, city: &[CityObj], claims: &[u8]) -> Vec<Option<Prop>> {
    let n = (map.w * map.h) as usize;
    let mut out: Vec<Option<Prop>> = vec![None; n];
    let mut put = |out: &mut Vec<Option<Prop>>, x: i32, y: i32, prop: Prop| {
        if x >= 0 && y >= 0 && x < map.w && y < map.h {
            out[(y * map.w + x) as usize] = Some(prop);
        }
    };

    // Garden trees: one or two small species in a finished house's own margin,
    // front or back, never on the walls and never on a street, and never in the
    // gap two houses share (round 4: order-independent, so a live run and a
    // reload plant the same trees).
    for (i, l) in city
        .iter()
        .enumerate()
        .filter(|(_, o)| is_garden_lot(o.kind) && o.stage == Stage::Finished)
    {
        let want = 1 + (l.seed % 2) as i32;
        let mut placed = 0;
        for k in 0..12i32 {
            if placed >= want {
                break;
            }
            let salt = (l.seed as u64).wrapping_mul(0x9E37_79B9) ^ (k as u64) << 7;
            let r = hash64(salt, l.seed as u64);
            let side = (r % 4) as usize;
            let along = ((r >> 2) % (l.w.max(l.h)) as u64) as i32;
            let (x, y) = match side {
                0 => (l.x + along, l.y - 1),
                1 => (l.x + along, l.y + l.h),
                2 => (l.x - 1, l.y + along),
                _ => (l.x + l.w, l.y + along),
            };
            if !plantable(map, claims, x, y) {
                continue;
            }
            // Not the gap two houses share: no *other* walled lot's ring may
            // claim this tile.
            if city.iter().enumerate().any(|(j, o)| {
                j != i
                    && has_walls(o.kind)
                    && x >= o.x - 1
                    && x < o.x + o.w + 1
                    && y >= o.y - 1
                    && y < o.y + o.h + 1
            }) {
                continue;
            }
            let species = treeline::GARDEN[(l.seed as usize + k as usize) % treeline::GARDEN.len()];
            let variant = (l.seed.wrapping_add(k as u32 * 37) % 8) as u8;
            put(
                &mut out,
                x,
                y,
                Prop::Tree {
                    species,
                    size: 0,
                    variant,
                },
            );
            placed += 1;
        }
    }

    // Street trees: a row in the verge beside the town's streets, one species
    // to a street, every `STREET_PITCH` tiles, gaps at junctions and drives.
    for y in 0..map.h {
        for x in 0..map.w {
            if !plantable(map, claims, x, y) {
                continue;
            }
            // Exactly one street beside it: a corner or a junction gets no tree.
            let lanes = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .filter(|(dx, dy)| {
                    let (nx, ny) = (x + dx, y + dy);
                    map.in_bounds(nx, ny) && map.at(nx, ny).kind.is_lane()
                })
                .count();
            if lanes != 1 {
                continue;
            }
            // The verge is a step or more from any wall: a tile within one of a
            // lot is a garden, not a verge.
            if lot_within(city, x, y, 1) {
                continue;
            }
            let along = if map.in_bounds(x + 1, y) && map.at(x + 1, y).kind.is_lane()
                || map.in_bounds(x - 1, y) && map.at(x - 1, y).kind.is_lane()
            {
                x
            } else {
                y
            };
            if along.rem_euclid(treeline::STREET_PITCH) != 0 {
                continue;
            }
            let (line, axis) = if map.in_bounds(x + 1, y) && map.at(x + 1, y).kind.is_lane()
                || map.in_bounds(x - 1, y) && map.at(x - 1, y).kind.is_lane()
            {
                (y, 0i32)
            } else {
                (x, 1i32)
            };
            let pick = (map.tile_hash(line, axis) * treeline::GARDEN.len() as f32) as usize;
            let species = treeline::GARDEN[pick.min(treeline::GARDEN.len() - 1)];
            let variant = (map.tile_hash(x, y) * 8.0) as u8;
            put(
                &mut out,
                x,
                y,
                Prop::Tree {
                    species,
                    size: 0,
                    variant,
                },
            );
        }
    }
    out
}

/// Draw-time cleaning: should a *stored* terrain prop be skipped on this tile?
/// A prop on hard ground (road, rail, paving, plaza, airfield) or under a wall
/// is skipped; a big tree within a tile of a wall is skipped (its crown would
/// reach the wall), but a small species may stand a pixel off the wall.
pub fn skip_prop(map: &TileMap, claims: &[u8], x: i32, y: i32, prop: Prop) -> bool {
    let idx = (y * map.w + x) as usize;
    let c = claims.get(idx).copied().unwrap_or(0);
    if c & C_HARD != 0 || c & C_WALL != 0 {
        return true;
    }
    if !prop.is_tree() {
        return false;
    }
    // A big tree beside a wall leans into it; a small one may stand a pixel off.
    if let Prop::Tree { species, .. } = prop {
        if treeline::is_small_species(species) {
            return false;
        }
    }
    for dy in -1..=1i32 {
        for dx in -1..=1i32 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nx = x + dx;
            let ny = y + dy;
            if nx < 0 || ny < 0 || nx >= map.w || ny >= map.h {
                continue;
            }
            let nc = claims.get((ny * map.w + nx) as usize).copied().unwrap_or(0);
            if nc & C_WALL != 0 {
                return true;
            }
        }
    }
    false
}
