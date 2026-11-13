//! Dynamic life: cars on the road grid, pedestrians on pavements.
//!
//! Positions are continuous tile coordinates. Cars follow the road tiles and
//! pick a turn at each tile boundary (no pathfinding, no traffic rules): the
//! point is believable motion at a known cost, not a traffic sim. See the
//! handback for what M7 will have to add.
//!
//! Occlusion uses the static layer's per-pixel painter keys (P4): an actor is
//! hidden wherever something drawn later (nearer the viewer) already wrote.

use map::rng::Rng;
use map::{Kind, TileMap};

use crate::framebuffer::{Framebuffer, Sprite};
use crate::iso::{Camera, HEIGHT_UNIT};
use crate::palette::{self, pal};

pub const CAR_W: usize = 22;
pub const CAR_H: usize = 18;
pub const PED_W: usize = 3;
pub const PED_H: usize = 8;

/// How actors are tested against the static depth plane.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DepthMode {
    /// Per pixel: the best-looking option, a fixed cost per sprite pixel.
    #[default]
    PerPixel,
    /// One test per actor at its feet; cheaper, but whole sprites pop.
    Anchor,
    /// No test at all (H0 behaviour, kept for the comparison numbers).
    Off,
}

#[inline]
pub fn is_road(map: &TileMap, x: i32, y: i32) -> bool {
    map.in_bounds(x, y) && map.at(x, y).kind == Kind::Road
}

#[inline]
fn is_walkable(map: &TileMap, x: i32, y: i32) -> bool {
    map.in_bounds(x, y) && map.at(x, y).kind.is_walkable()
}

/// The test scene's cars are the same isometric boxes the traffic layer uses
/// (B32), in eight body colours, all facing +x.
pub fn car_sprites(_seed: u64) -> Vec<Sprite> {
    (0..8).map(|v| crate::traffic::sprite_for_actor(v as u8)).collect()
}

/// A villager at 64-px detail: 3 px wide, 8 tall, with a head, a coat, arms and
/// two legs, in two walk frames (B32). Eight colour variants.
pub fn ped_sprites(seed: u64) -> Vec<Sprite> {
    let mut rng = Rng::new(seed ^ 0x9ED0_0000);
    let mut out = Vec::new();
    for i in 0..8 {
        let shirt = palette::jitter(*rng.pick(&pal::PEOPLE_SHIRT), rng.f32(), 0.08);
        let hair = palette::rgb(0x3a, 0x2c, 0x22);
        let mut s = Sprite::new(PED_W, PED_H);
        // Head and hair.
        s.set(1, 0, hair);
        s.set(1, 1, pal::PEOPLE_HEAD);
        s.set(0, 1, hair);
        // Coat, with a lit edge on the sunlit side.
        s.set(1, 2, palette::shade(shirt, 1.10));
        s.set(0, 2, shirt);
        s.set(1, 3, shirt);
        s.set(2, 3, palette::shade(shirt, 0.85));
        s.set(1, 4, palette::shade(shirt, 0.92));
        // Legs: the walk frame alternates which leg is forward.
        if i % 2 == 0 {
            s.set(1, 5, pal::PEOPLE_LEGS);
            s.set(0, 6, pal::PEOPLE_LEGS);
            s.set(1, 7, pal::PEOPLE_LEGS);
        } else {
            s.set(1, 5, pal::PEOPLE_LEGS);
            s.set(2, 6, pal::PEOPLE_LEGS);
            s.set(1, 7, pal::PEOPLE_LEGS);
        }
        out.push(s);
    }
    out
}

#[derive(Clone, Debug)]
pub struct Car {
    pub tx: i32,
    pub ty: i32,
    /// Travel direction: one of (1,0), (-1,0), (0,1), (0,-1).
    pub dir: (i8, i8),
    /// Progress across the current tile, 0..1.
    pub t: f32,
    /// Tiles per second.
    pub speed: f32,
    pub sprite: u8,
    pub rng: Rng,
}

impl Car {
    pub fn spawn(map: &TileMap, road: &[(i32, i32)], rng: &mut Rng) -> Option<Self> {
        if road.is_empty() {
            return None;
        }
        for _ in 0..8 {
            let &(tx, ty) = rng.pick(road);
            let dirs = [(1i8, 0i8), (-1, 0), (0, 1), (0, -1)];
            let d = dirs[rng.usize(4)];
            if is_road(map, tx + d.0 as i32, ty + d.1 as i32) {
                return Some(Self {
                    tx,
                    ty,
                    dir: d,
                    t: rng.f32(),
                    speed: rng.range_f32(1.1, 2.4),
                    sprite: rng.usize(8) as u8,
                    rng: rng.fork(0xCA5),
                });
            }
        }
        None
    }

    pub fn update(&mut self, dt: f32, map: &TileMap) {
        self.t += self.speed * dt;
        let mut guard = 0;
        while self.t >= 1.0 && guard < 8 {
            guard += 1;
            self.t -= 1.0;
            let nx = self.tx + self.dir.0 as i32;
            let ny = self.ty + self.dir.1 as i32;
            if !is_road(map, nx, ny) {
                self.dir = (-self.dir.0, -self.dir.1);
                self.t = 0.0;
                break;
            }
            self.tx = nx;
            self.ty = ny;
            self.choose_dir(map);
        }
    }

    fn choose_dir(&mut self, map: &TileMap) {
        let straight = self.dir;
        let left = (self.dir.1, -self.dir.0);
        let right = (-self.dir.1, self.dir.0);
        let ok = |d: (i8, i8)| is_road(map, self.tx + d.0 as i32, self.ty + d.1 as i32);

        if ok(straight) && self.rng.chance(0.64) {
            return;
        }
        let mut options = [(0i8, 0i8); 3];
        let mut n = 0;
        for d in [straight, left, right] {
            if ok(d) {
                options[n] = d;
                n += 1;
            }
        }
        if n == 0 {
            self.dir = (-self.dir.0, -self.dir.1);
        } else {
            self.dir = options[self.rng.usize(n)];
        }
    }

    /// Continuous position in tile space, with a lane offset to the right.
    pub fn position(&self) -> (f32, f32) {
        let lane = 0.16
            * if self.dir.0 < 0 || self.dir.1 < 0 {
                1.0
            } else {
                -1.0
            };
        let px = -self.dir.1 as f32 * lane;
        let py = self.dir.0 as f32 * lane;
        (
            self.tx as f32 + 0.5 + self.dir.0 as f32 * self.t + px,
            self.ty as f32 + 0.5 + self.dir.1 as f32 * self.t + py,
        )
    }
}

#[derive(Clone, Debug)]
pub struct Ped {
    pub x: f32,
    pub y: f32,
    pub dir: (f32, f32),
    /// Tiles per second.
    pub speed: f32,
    /// Seconds until the next direction change.
    pub turn_in: f32,
    pub sprite: u8,
    pub rng: Rng,
    /// Some people just stand about.
    pub still: bool,
}

impl Ped {
    pub fn spawn(map: &TileMap, walkable: &[(i32, i32)], rng: &mut Rng) -> Option<Self> {
        if walkable.is_empty() {
            return None;
        }
        for _ in 0..8 {
            let &(tx, ty) = rng.pick(walkable);
            if !is_walkable(map, tx, ty) {
                continue;
            }
            let a = rng.f32() * std::f32::consts::TAU;
            return Some(Self {
                x: tx as f32 + rng.f32(),
                y: ty as f32 + rng.f32(),
                dir: (a.cos(), a.sin()),
                speed: rng.range_f32(0.18, 0.55),
                turn_in: rng.range_f32(0.5, 4.0),
                sprite: rng.usize(8) as u8,
                rng: rng.fork(0x9ED),
                still: rng.chance(0.25),
            });
        }
        None
    }

    pub fn update(&mut self, dt: f32, map: &TileMap) {
        self.turn_in -= dt;
        if self.turn_in <= 0.0 {
            let a = self.rng.f32() * std::f32::consts::TAU;
            self.dir = (a.cos(), a.sin());
            self.turn_in = self.rng.range_f32(0.6, 5.0);
            self.still = self.rng.chance(0.18);
        }
        if self.still {
            return;
        }
        let nx = self.x + self.dir.0 * self.speed * dt;
        let ny = self.y + self.dir.1 * self.speed * dt;
        if is_walkable(map, nx.floor() as i32, ny.floor() as i32) {
            self.x = nx;
            self.y = ny;
        } else {
            self.dir = (-self.dir.0, -self.dir.1);
        }
    }
}

pub const DEFAULT_CARS: usize = 300;
pub const DEFAULT_PEDS: usize = 400;

pub struct Life {
    pub cars: Vec<Car>,
    pub peds: Vec<Ped>,
    pub car_sprites: Vec<Sprite>,
    pub ped_sprites: Vec<Sprite>,
    /// Untinted originals, so re-tinting is idempotent.
    base_cars: Vec<Sprite>,
    base_peds: Vec<Sprite>,
}

impl Life {
    pub fn new(map: &TileMap, seed: u64, cars: usize, peds: usize) -> Self {
        let mut rng = Rng::new(seed ^ 0x11FE_0000);
        let road: Vec<(i32, i32)> = (0..map.h)
            .flat_map(|y| (0..map.w).map(move |x| (x, y)))
            .filter(|&(x, y)| is_road(map, x, y))
            .collect();
        let walkable: Vec<(i32, i32)> = (0..map.h)
            .flat_map(|y| (0..map.w).map(move |x| (x, y)))
            .filter(|&(x, y)| map.at(x, y).kind == Kind::Sidewalk)
            .collect();

        let mut car_v = Vec::with_capacity(cars);
        for _ in 0..cars {
            if let Some(c) = Car::spawn(map, &road, &mut rng) {
                car_v.push(c);
            }
        }
        let mut ped_v = Vec::with_capacity(peds);
        for _ in 0..peds {
            if road.is_empty() {
                break;
            }
            if let Some(p) = Ped::spawn(map, &walkable, &mut rng) {
                ped_v.push(p);
            }
        }
        let cs = car_sprites(seed);
        let ps = ped_sprites(seed);
        Self {
            cars: car_v,
            peds: ped_v,
            car_sprites: cs.clone(),
            ped_sprites: ps.clone(),
            base_cars: cs,
            base_peds: ps,
        }
    }

    /// Re-tint the sprite caches for a new ambient light (B13). Sixteen tiny
    /// sprites, so this costs microseconds and only runs on a palette change.
    pub fn retint(&mut self, tint: [f32; 3]) {
        let lut = crate::light::lut(tint);
        for (dst, src) in self.car_sprites.iter_mut().zip(self.base_cars.iter()) {
            dst.px.copy_from_slice(&src.px);
            crate::light::tint_pixels(&mut dst.px, &lut);
        }
        for (dst, src) in self.ped_sprites.iter_mut().zip(self.base_peds.iter()) {
            dst.px.copy_from_slice(&src.px);
            crate::light::tint_pixels(&mut dst.px, &lut);
        }
    }

    /// A life layer with no actors (empty land has nothing moving yet).
    pub fn empty(seed: u64) -> Self {
        let cs = car_sprites(seed);
        let ps = ped_sprites(seed);
        Self {
            cars: Vec::new(),
            peds: Vec::new(),
            car_sprites: cs.clone(),
            ped_sprites: ps.clone(),
            base_cars: cs,
            base_peds: ps,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cars.is_empty() && self.peds.is_empty()
    }

    pub fn update(&mut self, dt: f32, map: &TileMap) {
        for c in &mut self.cars {
            c.update(dt, map);
        }
        for p in &mut self.peds {
            p.update(dt, map);
        }
    }

    /// Draw every actor, testing against the static depth plane.
    pub fn draw(
        &self,
        fb: &mut Framebuffer,
        cam: &Camera,
        map: &TileMap,
        depth: &[u16],
        mode: DepthMode,
    ) {
        for c in &self.cars {
            let (x, y) = c.position();
            let z = map.ground_px(x, y);
            let (sx, sy) = cam.project(x, y, z);
            let s = &self.car_sprites[c.sprite as usize % self.car_sprites.len()];
            let (sx, sy) = (sx as i32 - s.ax, sy as i32 - s.ay);
            let key = actor_key(c.tx, c.ty);
            match mode {
                DepthMode::PerPixel => fb.blit_sprite_depth(s, sx, sy, key, depth),
                DepthMode::Anchor => {
                    fb.blit_sprite_anchor(s, sx, sy, key, depth);
                }
                DepthMode::Off => fb.blit_sprite(s, sx, sy),
            }
        }
        for p in &self.peds {
            let z = map.ground_px(p.x, p.y);
            let (sx, sy) = cam.project(p.x, p.y, z);
            let s = &self.ped_sprites[p.sprite as usize % self.ped_sprites.len()];
            let (sx, sy) = (sx as i32 - s.ax, sy as i32 - s.ay);
            let key = actor_key(p.x.floor() as i32, p.y.floor() as i32);
            match mode {
                DepthMode::PerPixel => fb.blit_sprite_depth(s, sx, sy, key, depth),
                DepthMode::Anchor => {
                    fb.blit_sprite_anchor(s, sx, sy, key, depth);
                }
                DepthMode::Off => fb.blit_sprite(s, sx, sy),
            }
        }
    }
}

/// Painter keys are `2 * diagonal + 1` for the ground and `2 * diagonal + 2` for
/// a building on that diagonal (buildings paint after the ground they stand on).
/// Zero therefore means "nothing drawn here", which the coverage test and the
/// depth plane rely on.
#[inline]
pub fn actor_key(tx: i32, ty: i32) -> u16 {
    (((tx + ty).max(0)) * 2) as u16 + 1
}

/// Painter key for a static item on diagonal `d`.
#[inline]
pub fn static_key(d: i32, is_building: bool) -> u16 {
    ((d.max(0) * 2) as u16) + 1 + is_building as u16
}

/// Bucket index (painter diagonal) of a key.
#[inline]
pub fn key_diagonal(key: u16) -> i32 {
    ((key.max(1) - 1) / 2) as i32
}

#[cfg(test)]
mod empty_spawn_tests {
    use super::*;
    use map::Tile;

    #[test]
    fn spawning_with_no_road_or_walkable_tiles_is_none_not_a_panic() {
        let map = TileMap::fill(4, 4, Tile::flat(2), 1);
        let mut rng = Rng::new(1);
        assert!(Car::spawn(&map, &[], &mut rng).is_none());
        assert!(Ped::spawn(&map, &[], &mut rng).is_none());
    }
}
