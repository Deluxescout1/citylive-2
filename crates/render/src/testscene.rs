//! The M2 test scene: roads, buildings and actors placed onto generated
//! terrain, so the renderer can be looked at and measured.
//!
//! This is deliberately separate from terrain generation: the real world stays
//! empty land (D20) and the growth sim (M3) will lay its own roads and lots.
//! Roads only go on buildable tiles, buildings only on flat land.

use map::rng::Rng;
use map::{Kind, TileMap};

use crate::actors::{DEFAULT_CARS, DEFAULT_PEDS, Life};
use crate::building::Building;
use crate::iso::{Camera, HEIGHT_UNIT};
use crate::palette;
use crate::palette::pal;

/// Tiles between road centre-lines.
pub const BLOCK: i32 = 9;

pub struct TestScene {
    pub buildings: Vec<Building>,
    pub life: Life,
    pub road_tiles: usize,
}

/// Lay a road grid and its pavement ring on every buildable tile.
pub fn apply_roads(map: &mut TileMap) -> usize {
    let mut roads = 0;
    for y in 0..map.h {
        for x in 0..map.w {
            // Roads stay on flat, lowish ground: a grid climbing a hillside
            // reads as nonsense and this is only a test scene.
            if !map.flat_buildable(x, y) || map.at(x, y).max_level() > 6 {
                continue;
            }
            let is_road = x % BLOCK == 0 || y % BLOCK == 0;
            let is_walk = !is_road
                && (x % BLOCK == 1
                    || x % BLOCK == BLOCK - 1
                    || y % BLOCK == 1
                    || y % BLOCK == BLOCK - 1);
            if is_road {
                map.with_tile_mut(x, y, |t| {
                    t.kind = Kind::Road;
                    t.prop = map::Prop::None;
                });
                roads += 1;
            } else if is_walk && map.at(x, y).kind == Kind::Grass {
                map.with_tile_mut(x, y, |t| {
                    t.kind = Kind::Sidewalk;
                    t.prop = map::Prop::None;
                });
            }
        }
    }
    roads
}

/// Scatter buildings on flat, free land.
pub fn build(
    map: &mut TileMap,
    cam: &Camera,
    seed: u64,
    count: usize,
    cars: usize,
    peds: usize,
) -> TestScene {
    let roads = apply_roads(map);
    let buildings = place_buildings(map, cam, seed, count);
    let life = Life::new(map, seed, cars, peds);
    TestScene {
        buildings,
        life,
        road_tiles: roads,
    }
}

/// Buildings on flat grass with a shared height level and a free footprint.
pub fn place_buildings(map: &mut TileMap, cam: &Camera, seed: u64, count: usize) -> Vec<Building> {
    let mut rng = Rng::new(seed ^ 0x0B01_1D00);
    let mut taken = vec![false; (map.w * map.h) as usize];
    let mut out: Vec<Building> = Vec::with_capacity(count);

    let mut attempts = 0usize;
    let max_attempts = count * 300;
    while out.len() < count && attempts < max_attempts {
        attempts += 1;
        let fw = if rng.chance(0.78) { 1 } else { 2 };
        let fh = if rng.chance(0.78) { 1 } else { 2 };
        let tx = rng.range(2, map.w - 3 - fw);
        let ty = rng.range(2, map.h - 3 - fh);

        let base = map.at(tx, ty);
        if base.is_water() || !map.flat_buildable(tx, ty) || base.max_level() > 8 {
            continue;
        }
        let mut ok = true;
        for dy in 0..fh {
            for dx in 0..fw {
                let t = map.at(tx + dx, ty + dy);
                if t.is_water()
                    || !t.is_flat()
                    || t.max_level() != base.max_level()
                    || taken[((ty + dy) * map.w + tx + dx) as usize]
                {
                    ok = false;
                }
            }
        }
        if !ok {
            continue;
        }
        for dy in 0..fh {
            for dx in 0..fw {
                taken[((ty + dy) * map.w + tx + dx) as usize] = true;
                // The lot is cleared before it is built on.
                map.with_tile_mut(tx + dx, ty + dy, |t| t.prop = map::Prop::None);
            }
        }

        // Mostly low-rise, a few towers.
        let roll = rng.f32();
        let height = if roll < 0.62 {
            rng.range(8, 30)
        } else if roll < 0.90 {
            rng.range(30, 70)
        } else {
            rng.range(70, 120)
        };

        let wall = palette::jitter(*rng.pick(&pal::WALLS), rng.f32(), 0.08);
        let roof = palette::jitter(*rng.pick(&pal::ROOFS), rng.f32(), 0.06);
        out.push(Building::new(
            cam,
            tx,
            ty,
            fw,
            fh,
            base.min_level() as i32 * HEIGHT_UNIT,
            height,
            wall,
            roof,
            &mut rng,
        ));
    }
    out.sort_by_key(|b| (b.tx + b.ty, b.tx));
    out
}

/// Convenience: full defaults for the spike.
pub fn build_default(map: &mut TileMap, cam: &Camera, seed: u64) -> TestScene {
    build(map, cam, seed, 150, DEFAULT_CARS, DEFAULT_PEDS)
}
