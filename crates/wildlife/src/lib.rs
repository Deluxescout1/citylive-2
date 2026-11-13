//! CityLive Iso wildlife layer (B22, D44).
//!
//! A cheap, deterministic animal layer the renderer draws on top of the scene
//! (like actors): the map decides where animals live.
//!
//! * **birds** — small flocks crossing the frame (light boids: banding
//!   direction + weak cohesion toward a per-flock centroid);
//! * **gulls** — circling their home water by day, a few perched on roofs
//!   (or a tree until the village has one);
//! * **ducks / swans** — paddling on lake, river and pond water;
//! * **fish** — an occasional jump drawn as a 3-frame splash on the surface;
//! * **deer** — grazing meadow / forest edges, darting away when a road or
//!   building comes within reach;
//! * **rabbits** — hopping the grass;
//! * **cows / sheep / horses** — inside their pasture (`Crop::Pasture` fields) only,
//!   and in the barn at night and through the winter; **hens** in the farmyard;
//!   **tractors and combines** on arable fields in their season;
//! * **owls / bats** — night only; land animals thin out at night.
//!
//! Design rules (same ground rules as the rest of the repo):
//! * fixed pools allocated once in [`Wildlife::new`] — `update` and `draw`
//!   never allocate (checked by `tests/no_alloc.rs`);
//! * deterministic in `(seed, dt-sequence, time-of-day-sequence, map,
//!   obstacles)`, all randomness from `map::rng::Rng` (checked by
//!   `tests/determinism.rs`);
//! * `draw`'s `depth` is the static layer's per-pixel painter-key plane, so
//!   an animal hides behind a building or a hill exactly like an actor does;
//!   flying animals live above the terrain, so buildings pass under them;
//! * `dirty_rects()` lists where the last `draw` touched the frame, so the
//!   wallpaper only repaints where animals moved.
//!
//! `update` takes the *current* map (the village keeps growing into the same
//! `TileMap`); surface height is re-sampled per animal per step, so a
//! re-graded road still lands under their feet. `draw` takes the same
//! [`Viewport`] the scene was rasterised with (the default camera — for a
//! sky-band scene the caller passes that camera's view instead).

pub mod sprites;

use map::rng::Rng;
use map::{Kind, TileMap, Viewport};
use render::actors::actor_key;
use render::city::{CityObj, Stage};
use render::framebuffer::{Framebuffer, Rect, Sprite};
use render::iso::{Camera, HEIGHT_UNIT};

pub use crate::sprites::Sheets;

use std::f32::consts::TAU;

/// Total animals in the fixed pool (the brief's "~60 animals").
pub const POOL: usize = 84;

/// How many dirty rectangles `draw` may buffer (each active animal marks at
/// most one swept rect, and an animal that just went inactive marks the box it
/// left behind; the buffer is 2× the pool for fence-sitting).
pub const DIRTY_MAX: usize = POOL * 2;

/// A species group and how many pool slots it owns. Order defines the pool
/// partition.
const GROUPS: [(Species, usize); 15] = [
    (Species::Bird, 12),
    (Species::Gull, 6),
    (Species::Duck, 8),
    (Species::Swan, 4),
    (Species::Fish, 4),
    (Species::Deer, 8),
    (Species::Rabbit, 8),
    (Species::Cow, 8),
    (Species::Sheep, 8),
    (Species::Horse, 3),
    (Species::Owl, 2),
    (Species::Bat, 4),
    (Species::Chicken, 6),
    (Species::Tractor, 2),
    (Species::Combine, 1),
];

const fn group_fill(mut out: [Species; POOL], g: usize, i: usize) -> [Species; POOL] {
    if g == GROUPS.len() {
        return out;
    }
    let mut k = 0usize;
    while k < GROUPS[g].1 {
        out[i + k] = GROUPS[g].0;
        k += 1;
    }
    group_fill(out, g + 1, i + k)
}

const SPECIES: [Species; POOL] = group_fill([Species::Bird; POOL], 0, 0);

/// What lives in one pool slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Species {
    Bird,
    Gull,
    Duck,
    Swan,
    Fish,
    Deer,
    Rabbit,
    Cow,
    Sheep,
    Horse,
    Owl,
    Bat,
    /// B80: hens in the farmyard.
    Chicken,
    /// B80: a tractor ploughing and sowing.
    Tractor,
    /// B80: a combine at the harvest.
    Combine,
}

impl Species {
    const NAMES: [&'static str; 15] = [
        "bird", "gull", "duck", "swan", "fish", "deer", "rabbit", "cow", "sheep", "horse", "owl", "bat",
        "chicken", "tractor", "combine",
    ];

    /// Index into `Wildlife::counts()`.
    pub const fn id(self) -> u8 {
        match self {
            Species::Bird => 0,
            Species::Gull => 1,
            Species::Duck => 2,
            Species::Swan => 3,
            Species::Fish => 4,
            Species::Deer => 5,
            Species::Rabbit => 6,
            Species::Cow => 7,
            Species::Sheep => 8,
            Species::Horse => 9,
            Species::Owl => 10,
            Species::Bat => 11,
            Species::Chicken => 12,
            Species::Tractor => 13,
            Species::Combine => 14,
        }
    }

    /// The species with `id()` index `i` (`counts()` order).
    pub const fn by_id(i: usize) -> Option<Self> {
        match i {
            0 => Some(Self::Bird),
            1 => Some(Self::Gull),
            2 => Some(Self::Duck),
            3 => Some(Self::Swan),
            4 => Some(Self::Fish),
            5 => Some(Self::Deer),
            6 => Some(Self::Rabbit),
            7 => Some(Self::Cow),
            8 => Some(Self::Sheep),
            9 => Some(Self::Horse),
            10 => Some(Self::Owl),
            11 => Some(Self::Bat),
            12 => Some(Self::Chicken),
            13 => Some(Self::Tractor),
            14 => Some(Self::Combine),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        Self::NAMES[self.id() as usize]
    }

    /// Whether it lives on the water, on land, or in the air (tests/report).
    pub const fn in_water(self) -> bool {
        matches!(self, Species::Duck | Species::Swan | Species::Fish)
    }

    /// The farm stock: must stay in its pasture (hens in the yard).
    pub const fn farm(self) -> bool {
        matches!(self, Species::Cow | Species::Sheep | Species::Horse | Species::Chicken)
    }

    /// A farm machine: stays on its own arable field.
    pub const fn machine(self) -> bool {
        matches!(self, Species::Tractor | Species::Combine)
    }

    /// The day animals that graze grass and flee roads (deer / rabbits).
    pub const fn grazer(self) -> bool {
        matches!(self, Species::Deer | Species::Rabbit)
    }
}

impl std::fmt::Display for Species {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// 0..=1 daylight amount: 0 at night, 1 from 6:00 to 20:00, ramps over dawn
/// and dusk. Drives the day/night masks (D44: fewer animals at night, owls
/// and bats out at night).
fn day_factor(tod: f32) -> f32 {
    let t = tod.rem_euclid(24.0);
    if t < 4.5 {
        0.0
    } else if t < 6.0 {
        (t - 4.5) / 1.5
    } else if t < 20.0 {
        1.0
    } else if t < 21.5 {
        1.0 - (t - 20.0) / 1.5
    } else {
        0.0
    }
}

/// Is tile `(x, y)` inside any building's footprint (all stages)?
fn in_obstacle(obstacles: &[CityObj], x: i32, y: i32) -> bool {
    obstacles
        .iter()
        .any(|o| x >= o.x && x < o.x + o.w && y >= o.y && y < o.y + o.h)
}

/// A legal standing place for species `s` at continuous position `(fx, fy)`.
/// Water paddlers stay on water; farm stock never leaves the field; grazers
/// stay off worked ground and never enter a building footprint. Steep or
/// cliff tiles are refused for everyone.
fn legal(map: &TileMap, obstacles: &[CityObj], s: Species, fx: f32, fy: f32, field_locked: bool) -> bool {
    let (x, y) = (fx as i32, fy as i32);
    if !map.in_bounds(x, y) {
        return false;
    }
    let t = map.at(x, y);
    if t.relief() > 1 {
        return false;
    }
    let _ = field_locked;
    if s.farm() || s.machine() {
        let crop = (t.kind == Kind::Field).then(|| map::crop_of(&t));
        if s == Species::Chicken {
            return crop == Some(map::Crop::Track) && !in_obstacle(obstacles, x, y);
        }
        if s.machine() {
            return crop.is_some_and(|c| c.is_arable());
        }
        // B80: stock lives in the pasture. A land without pasture fields (the
        // savanna, the highlands) lets the herd graze the open grass.
        if crop == Some(map::Crop::Pasture) {
            return true;
        }
        if matches!(map.biome, map::Biome::Savanna | map::Biome::Highlands) {
            return matches!(t.kind, Kind::Grass | Kind::Meadow | Kind::DryGrass | Kind::Upland)
                && !in_obstacle(obstacles, x, y);
        }
        return false;
    }
    match s {
        Species::Duck | Species::Swan => t.is_water(),
        Species::Deer | Species::Rabbit | Species::Cow | Species::Sheep | Species::Horse => {
            matches!(t.kind, Kind::Grass | Kind::Meadow | Kind::Upland | Kind::DryGrass)
                && !t.kind.is_worked()
                && !in_obstacle(obstacles, x, y)
        }
        _ => false,
    }
}

/// A lane (or sidewalk) anywhere in the square around the animal, or any
/// building within `radius` tiles — the thing that makes them run off (D44:
/// animals keep away from busy streets and buildings).
fn scan_threat(map: &TileMap, obstacles: &[CityObj], a: &Animal, radius: i32) -> Option<(f32, f32)> {
    let (cx, cy) = (a.x as i32, a.y as i32);
    let mut nearest: Option<(i32, i32, i32)> = None; // (dist, x, y)
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let (nx, ny) = (cx + dx, cy + dy);
            if !map.in_bounds(nx, ny) {
                continue;
            }
            let t = map.at(nx, ny);
            if t.kind.is_lane() || t.kind == Kind::Sidewalk {
                let d = (dx * dx + dy * dy) as i32;
                if nearest.map(|(nd, _, _)| d < nd).unwrap_or(true) {
                    nearest = Some((d, nx, ny));
                }
            }
        }
    }
    if let Some((_, nx, ny)) = nearest {
        return Some((nx as f32 + 0.5, ny as f32 + 0.5));
    }
    for o in obstacles {
        let (ox, oy) = (o.x + o.w / 2, o.y + o.h / 2);
        if (ox - cx).abs() <= radius && (oy - cy).abs() <= radius {
            return Some((ox as f32, oy as f32));
        }
    }
    None
}

/// One pooled animal. Fields are fixed-size; flying species reuse the same
/// slots for altitude / flocking.
struct Animal {
    species: Species,
    /// Index inside its own species group (stable: the night population is
    /// the same animals every night).
    gid: u8,
    /// Continuous tile position.
    x: f32,
    y: f32,
    /// Home tile, the wandering anchor.
    hx: i32,
    hy: i32,
    /// Heading.
    dx: f32,
    dy: f32,
    /// Base walking speed, tiles/s.
    speed: f32,
    /// Seconds until the next heading change.
    turn_in: f32,
    /// Circle / bob phase (gull circle angle, up-down for birds).
    bob: f32,
    /// Screen-px altitude above terrain (flying species).
    alt: f32,
    /// Surface height in screen px, re-sampled in `update`.
    z: i32,
    /// Current animation frame and its timer.
    frame: u8,
    frame_t: f32,
    /// Live this update (mask on).
    active: bool,
    /// Flock index for birds (3 flocks).
    flock: u8,
    /// Perched (gulls / owls), plus the perch screen point.
    perched: bool,
    perch_x: f32,
    perch_y: f32,
    perch_z: i32,
    /// Seconds until the perch / flight state flips.
    perch_t: f32,
    /// Seconds left of a flee burst, and the threat it was aimed at.
    flee_t: f32,
    threat_x: f32,
    threat_y: f32,
    /// Seconds until the next coarse threat rescan (1 Hz, staggered).
    threat_t: f32,
    /// Deer only: stags (even gids) carry antlers, does don't (B22b).
    stag: bool,
    /// Fish: countdown to the next jump, and the splash age once jumping.
    jump_t: f32,
    jump_e: f32,
    /// Screen rect the sprite occupied on the last `draw`.
    prev_rect: Rect,
    rng: Rng,
}

impl Animal {
    fn blank(s: Species, gid: u8) -> Self {
        Animal {
            species: s,
            gid,
            x: 0.5,
            y: 0.5,
            hx: 0,
            hy: 0,
            dx: 1.0,
            dy: 0.0,
            speed: 0.2,
            turn_in: 1.0,
            bob: 0.0,
            alt: 0.0,
            z: 0,
            frame: 0,
            frame_t: 0.0,
            active: false,
            flock: 0,
            perched: false,
            perch_x: 0.0,
            perch_y: 0.0,
            perch_z: 0,
            perch_t: 0.0,
            flee_t: 0.0,
            threat_x: 0.0,
            threat_y: 0.0,
            threat_t: 0.0,
            stag: false,
            jump_t: 0.0,
            jump_e: 0.0,
            prev_rect: Rect::EMPTY,
            // Placeholder stream; spawn() replaces it with a forked one.
            rng: Rng::new(0),
        }
    }
}

/// The wildlife layer: every animal, in fixed pools, built once.
pub struct Wildlife {
    animals: [Animal; POOL],
    sheets: Sheets,
    /// Habitat harvest from `new`: candidate tiles per habitat (capped).
    trees: Vec<(i32, i32, i32)>, // (x, y, ground level)
    // Dirty-rect ring for the last `draw`.
    dirties: [Rect; DIRTY_MAX],
    dirty_n: usize,
    /// The boxes the last `draw` actually painted (not the swept union): the
    /// trail test needs to know where an animal *is*, not where it has been.
    boxes: [Rect; POOL],
    box_n: usize,
    /// Latched in `update`; what `is_active` reports.
    any: bool,
    /// B60/D74: the light's shadow direction, so a deer on the grass throws
    /// one the same way a house does.
    shadow: Option<render::light::ActorShadow>,
    /// The screen row of the land line (the horizon). A ground animal whose
    /// feet are above it stands on land the horizon belt hides, and would be
    /// drawn on the sky (the owner: "moose in the sky"); it is not drawn.
    land_y: i32,
    /// B80: arable field tiles (where the machines work), harvested in `new`.
    arable: Vec<(i32, i32)>,
    /// B80: where the crop year stands (`SeasonLook::field_stage`) and whether
    /// it is winter: the machines and the stock follow them.
    farm_stage: f32,
    winter: bool,
}

impl Wildlife {
    /// Build the layer for this world. One-off (allocates the pools, the
    /// habitat lists and the sprite sheets); `update` and `draw` never
    /// allocate afterwards.
    ///
    /// The map only needs `new` for the habitat harvest — the animal
    /// behaviour always follows the map passed to `update`.
    pub fn new(seed: u64, map: &TileMap) -> Self {
        let mut rng = Rng::new(seed ^ 0x5157_11FE);
        let mut water: Vec<(i32, i32)> = Vec::with_capacity(400);
        let mut fields: Vec<(i32, i32)> = Vec::with_capacity(160);
        let mut tracks: Vec<(i32, i32)> = Vec::with_capacity(64);
        let mut arable: Vec<(i32, i32)> = Vec::with_capacity(256);
        let mut meadow: Vec<(i32, i32)> = Vec::with_capacity(160);
        let mut grass: Vec<(i32, i32)> = Vec::with_capacity(320);
        let mut trees: Vec<(i32, i32, i32)> = Vec::with_capacity(24);
        for y in 0..map.h {
            for x in 0..map.w {
                let t = map.at(x, y);
                if t.is_water() {
                    water.push((x, y));
                    continue;
                }
                if t.kind == Kind::Field {
                    match map::crop_of(&t) {
                        map::Crop::Pasture => fields.push((x, y)),
                        map::Crop::Track => tracks.push((x, y)),
                        c if c.is_arable() => arable.push((x, y)),
                        _ => {}
                    }
                    continue;
                }
                if t.kind.is_worked() || t.relief() > 1 {
                    continue;
                }
                let on_grass = matches!(
                    t.kind,
                    Kind::Grass | Kind::Meadow | Kind::Upland | Kind::DryGrass
                );
                if !on_grass {
                    continue;
                }
                if grass.len() < 320 {
                    grass.push((x, y));
                }
                if t.prop.is_tree() && trees.len() < 24 {
                    trees.push((x, y, t.min_level() as i32));
                }
                // Forest-edge meadow: grass within one tile of a tree, where
                // deer browse.
                if matches!(t.kind, Kind::Grass | Kind::Meadow)
                    && meadow.len() < 160
                    && near_tree(map, x, y)
                {
                    meadow.push((x, y));
                }
            }
        }

        let mut animals = std::array::from_fn(|_i| Animal::blank(Species::Bird, 0));
        let mut gid = [0u8; 15];
        let mut slot = 0usize;
        for &(sp, n) in GROUPS.iter() {
            for _ in 0..n {
                animals[slot] = Self::spawn(
                    &mut rng,
                    sp,
                    gid[sp.id() as usize],
                    map,
                    &water,
                    &fields,
                    &meadow,
                    &grass,
                    &trees,
                    &tracks,
                    &arable,
                );
                gid[sp.id() as usize] += 1;
                slot += 1;
            }
        }

        Wildlife {
            animals,
            sheets: Sheets::build_for(map.biome),
            trees,
            dirties: [Rect::EMPTY; DIRTY_MAX],
            dirty_n: 0,
            boxes: [Rect::EMPTY; POOL],
            box_n: 0,
            any: false,
            shadow: None,
            land_y: i32::MIN,
            arable,
            farm_stage: 0.5,
            winter: false,
        }
    }

    /// Place one slot in its habitat (or mark the slot hollow when the world
    /// has none of it).
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        rng: &mut Rng,
        s: Species,
        gid: u8,
        map: &TileMap,
        water: &[(i32, i32)],
        fields: &[(i32, i32)],
        meadow: &[(i32, i32)],
        grass: &[(i32, i32)],
        trees: &[(i32, i32, i32)],
        tracks: &[(i32, i32)],
        arable: &[(i32, i32)],
    ) -> Animal {
        let mut a = Animal::blank(s, gid);
        a.rng = rng.fork(0x4E1D2A00 + gid as u64);
        let rand_ang = |rng: &mut Rng| -> (f32, f32) {
            let ang = rng.f32() * TAU;
            (ang.cos(), ang.sin())
        };
        match s {
            Species::Bird => {
                a.flock = (gid / 4) as u8;
                a.x = -3.0;
                a.y = a.rng.range(6, map.h.saturating_sub(6).max(6)) as f32;
                a.hx = a.x as i32;
                a.hy = a.y as i32;
                a.speed = a.rng.range_f32(1.1, 1.8);
                a.dx = 0.7071068;
                a.dy = -0.7071068;
                // 64-px world (B32): flight heights double with the tile, so
                // the flocks clear the doubled building heights.
                a.alt = a.rng.range_f32(32.0, 84.0);
                a.bob = a.rng.f32() * TAU;
                a.threat_t = (a.rng.f32()).clamp(0.01, 1.0);
            }
            Species::Gull => {
                if water.is_empty() {
                    a.gid = 255;
                    return a;
                }
                let (wx, wy) = water[a.rng.usize(water.len())];
                a.hx = wx;
                a.hy = wy;
                a.x = wx as f32 + 0.5;
                a.y = wy as f32 + 0.5;
                if gid >= 4 {
                    // Percher: perches from its first active update (needs
                    // the obstacle list), 25–60 s per perch.
                    a.perch_t = a.rng.range_f32(25.0, 60.0);
                } else {
                    a.bob = a.rng.f32() * TAU;
                    a.speed = a.rng.range_f32(0.4, 0.9);
                    a.alt = a.rng.range_f32(16.0, 40.0);
                }
            }
            Species::Duck | Species::Swan => {
                if water.is_empty() {
                    a.gid = 255;
                    return a;
                }
                let (wx, wy) = water[a.rng.usize(water.len())];
                a.hx = wx;
                a.hy = wy;
                a.x = wx as f32 + 0.5;
                a.y = wy as f32 + 0.5;
                let (dx, dy) = rand_ang(&mut a.rng);
                a.dx = dx;
                a.dy = dy;
                a.speed = a.rng.range_f32(0.14, 0.3);
                a.turn_in = a.rng.range_f32(2.0, 6.0);
                a.z = map.ground_px(a.x, a.y);
            }
            Species::Fish => {
                if water.is_empty() {
                    a.gid = 255;
                    return a;
                }
                let (wx, wy) = water[a.rng.usize(water.len())];
                a.hx = wx;
                a.hy = wy;
                a.x = wx as f32 + 0.5;
                a.y = wy as f32 + 0.5;
                a.jump_t = a.rng.range_f32(3.0, 14.0);
                a.z = map.ground_px(a.x, a.y);
            }
            Species::Deer => {
                let list = if meadow.is_empty() { grass } else { meadow };
                if list.is_empty() {
                    a.gid = 255;
                    return a;
                }
                let (mx, my) = list[a.rng.usize(list.len())];
                a.hx = mx;
                a.hy = my;
                a.x = mx as f32 + 0.5;
                a.y = my as f32 + 0.5;
                let (dx, dy) = rand_ang(&mut a.rng);
                a.dx = dx;
                a.dy = dy;
                a.speed = 0.22;
                a.turn_in = a.rng.range_f32(2.0, 6.0);
                a.threat_t = (gid as f32 * 0.13).rem_euclid(1.0).max(0.05);
                a.stag = gid % 2 == 0;
                a.z = map.ground_px(a.x, a.y);
            }
            Species::Rabbit => {
                if grass.is_empty() {
                    a.gid = 255;
                    return a;
                }
                let (gx, gy) = grass[a.rng.usize(grass.len())];
                a.hx = gx;
                a.hy = gy;
                a.x = gx as f32 + 0.5;
                a.y = gy as f32 + 0.5;
                let (dx, dy) = rand_ang(&mut a.rng);
                a.dx = dx;
                a.dy = dy;
                a.speed = 0.34;
                a.turn_in = a.rng.range_f32(1.0, 4.0);
                a.threat_t = (gid as f32 * 0.17).rem_euclid(1.0).max(0.05);
                a.z = map.ground_px(a.x, a.y);
            }
            Species::Cow | Species::Sheep | Species::Horse => {
                // B46e: on the savanna the herds graze the open plain, not fields.
                let herd = matches!(map.biome, map::Biome::Savanna | map::Biome::Highlands);
                let pool = if herd && fields.is_empty() { grass } else { fields };
                if pool.is_empty() {
                    a.gid = 255;
                    return a;
                }
                let (fx, fy) = pool[a.rng.usize(pool.len())];
                a.hx = fx;
                a.hy = fy;
                a.x = fx as f32 + 0.5;
                a.y = fy as f32 + 0.5;
                let (dx, dy) = rand_ang(&mut a.rng);
                a.dx = dx;
                a.dy = dy;
                a.speed = a.rng.range_f32(0.08, 0.16);
                a.turn_in = a.rng.range_f32(3.0, 8.0);
                a.threat_t = (gid as f32 * 0.21).rem_euclid(1.0).max(0.05);
                a.z = map.ground_px(a.x, a.y);
            }
            Species::Chicken => {
                if tracks.is_empty() {
                    a.gid = 255;
                    return a;
                }
                let (fx, fy) = tracks[a.rng.usize(tracks.len())];
                a.hx = fx;
                a.hy = fy;
                a.x = fx as f32 + 0.5;
                a.y = fy as f32 + 0.5;
                let (dx, dy) = rand_ang(&mut a.rng);
                a.dx = dx;
                a.dy = dy;
                a.speed = a.rng.range_f32(0.06, 0.14);
                a.turn_in = a.rng.range_f32(1.0, 3.0);
                a.z = map.ground_px(a.x, a.y);
            }
            Species::Tractor | Species::Combine => {
                if arable.is_empty() {
                    a.gid = 255;
                    return a;
                }
                let (fx, fy) = arable[a.rng.usize(arable.len())];
                a.hx = fx;
                a.hy = fy;
                a.x = fx as f32 + 0.5;
                a.y = fy as f32 + 0.5;
                // Along x or along y, by the slot: the rows of one field run one way.
                let along_x = a.gid % 2 == 0;
                a.dx = if along_x { 1.0 } else { 0.0 };
                a.dy = if along_x { 0.0 } else { 1.0 };
                a.speed = if a.species == Species::Combine { 0.30 } else { 0.42 };
                a.turn_in = a.rng.range_f32(60.0, 160.0);
                a.bob = 1.0;
                a.z = map.ground_px(a.x, a.y);
            }
            Species::Owl => {
                // Perches on a tree once night falls; hollow if the world has
                // no trees.
                if trees.is_empty() {
                    a.gid = 255;
                    return a;
                }
                let (tx, ty, lv) = trees[a.rng.usize(trees.len())];
                a.hx = tx;
                a.hy = ty;
                a.perch_x = tx as f32 + 0.5;
                a.perch_y = ty as f32 + 0.5;
                // 64-px trees (B32) stand 23–44 px tall: mid-canopy.
                a.perch_z = lv * HEIGHT_UNIT + 30;
                a.perched = true;
            }
            Species::Bat => {
                let list = if meadow.is_empty() { grass } else { meadow };
                if list.is_empty() {
                    a.gid = 255;
                    return a;
                }
                let (bx, by) = list[a.rng.usize(list.len())];
                a.hx = bx;
                a.hy = by;
                a.bob = a.rng.f32() * TAU;
                a.speed = a.rng.range_f32(0.8, 1.4);
                a.alt = a.rng.range_f32(16.0, 40.0);
                a.z = map.ground_px(a.x, a.y);
            }
        }
        a
    }
}

/// Grass within one tile of any tree (forest edge).
fn near_tree(map: &TileMap, x: i32, y: i32) -> bool {
    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            if map.in_bounds(x + dx, y + dy) && map.at(x + dx, y + dy).prop.is_tree() {
                return true;
            }
        }
    }
    false
}

impl Wildlife {
    /// Advance the layer by `dt` seconds. `map` is the world as it stands
    /// *now* (the sim keeps growing the village into it), `obstacles` is the
    /// current building list, `time_of_day` is the clock hour (0..24). No
    /// allocation.
    pub fn update(&mut self, dt: f32, map: &TileMap, obstacles: &[CityObj], time_of_day: f32) {
        let dt = dt.clamp(0.0, 0.1);
        let day = day_factor(time_of_day);
        let w = map.w as f32;
        let h = map.h as f32;
        let trees = &self.trees;
        let (self_stage, self_winter) = (self.farm_stage, self.winter);
        let arable = &self.arable;

        // Flock centroids from the previous step (birds steer toward these).
        let mut acc = [[0.0f32, 0.0f32, 0.0f32]; 3];
        for a in self.animals.iter() {
            if a.species == Species::Bird && a.active {
                let f = a.flock as usize;
                acc[f][0] += a.x;
                acc[f][1] += a.y;
                acc[f][2] += 1.0;
            }
        }
        let mut fc = [[w / 2.0, h / 2.0]; 3];
        for f in 0..3 {
            if acc[f][2] > 0.0 {
                fc[f] = [acc[f][0] / acc[f][2], acc[f][1] / acc[f][2]];
            }
        }

        let mut any = false;
        for a in self.animals.iter_mut() {
            // Fish timers run regardless of the mask: the jump must be armed
            // *before* the mask lets it on screen.
            if a.species == Species::Fish && a.gid != 255 {
                fish_timer(a, dt);
            }
            let live = mask(a, day, self_stage, self_winter);
            if live {
                if !a.active {
                    enter(a, trees, obstacles);
                }
                a.active = true;
                a.z = map.ground_px(a.x, a.y);
                step_anim(a, dt);
                step_behavior(a, dt, map, obstacles, &fc, w, h, trees, arable);
                any = true;
            } else {
                a.active = false;
                a.flee_t = 0.0;
                // Keep `prev_rect`: `draw` reports it once so the caller can
                // restore the pixels the animal left behind (B42 — the sky
                // trail bug).
            }
        }
        self.any = any;
    }

    /// Blit one animal and record its swept rect for `dirty_rects`.
    fn draw_one(&mut self, i: usize, fb: &mut Framebuffer, cam: &Camera, depth: Option<&[u16]>) {
        // Read the state into locals first so the sprite lookup and the
        // `prev_rect` write never fight a live borrow.
        let (sp, stag, perched, frame) = {
            let a = &self.animals[i];
            (a.species, a.stag, a.perched, a.frame)
        };
        // A walker on the ground (not a flyer, not perched) whose feet project
        // above the land line is hidden: report what it painted last and stop.
        {
            let a = &self.animals[i];
            let flyer = matches!(sp, Species::Bird | Species::Gull | Species::Owl | Species::Bat) || a.alt >= 1.0;
            if !flyer && !perched {
                let (_, fy) = cam.project(a.x, a.y, a.z.max(0));
                if (fy as i32) < self.land_y {
                    let pr = self.animals[i].prev_rect;
                    if !pr.is_empty() && self.dirty_n < DIRTY_MAX {
                        self.dirties[self.dirty_n] = pr.expanded(1);
                        self.dirty_n += 1;
                    }
                    self.animals[i].prev_rect = Rect::EMPTY;
                    return;
                }
            }
        }
        let mut shadow_r = Rect::EMPTY;
        let (px, py) = if perched {
            let (cx, cy, cz) = {
                let a = &self.animals[i];
                (a.perch_x, a.perch_y, a.perch_z)
            };
            cam.project(cx, cy, cz)
        } else {
            let (ax, ay, az, aalt) = {
                let a = &self.animals[i];
                (a.x, a.y, a.z, a.alt)
            };
            // B60: an animal on the ground throws the same shadow everything
            // else does (one on a wing, or perched in a tree, throws none).
            if let Some(sh) = self.shadow {
                if aalt < 1.0 {
                    let (tx, ty) = (ax.floor() as i32, ay.floor() as i32);
                    shadow_r = render::light::draw_actor_shadow(
                        fb,
                        cam,
                        depth.unwrap_or(&[]),
                        actor_key(tx, ty),
                        ax,
                        ay,
                        az.max(0),
                        0.06,
                        0.06,
                        9.0,
                        &sh,
                        depth.is_some(),
                    );
                }
            }
            cam.project(ax, ay, ((az as f32 + aalt).max(0.0)).round() as i32)
        };
        let spr = Self::sprite_of(&self.sheets, sp, stag, perched, frame as usize);
        let bx = px as i32 - spr.w as i32 / 2;
        let by = py as i32 - spr.h as i32 + 1;
        let cur = Rect::new(bx, by, bx + spr.w as i32, by + spr.h as i32);
        // The shadow belongs to the animal's pixels too (it trailed otherwise).
        let cur = if shadow_r.is_empty() { cur } else { cur.union(&shadow_r) };
        let swept = if self.animals[i].prev_rect.is_empty() {
            cur
        } else {
            self.animals[i].prev_rect.union(&cur)
        };
        if self.dirty_n < DIRTY_MAX {
            self.dirties[self.dirty_n] = swept.expanded(1);
            self.dirty_n += 1;
        }
        if self.box_n < POOL {
            self.boxes[self.box_n] = cur;
            self.box_n += 1;
        }
        let (tx, ty) = if perched {
            let a = &self.animals[i];
            (a.perch_x as i32, a.perch_y as i32)
        } else {
            let a = &self.animals[i];
            (a.x as i32, a.y as i32)
        };
        let key = actor_key(tx, ty);
        match depth {
            Some(d) => fb.blit_sprite_depth(spr, bx, by, key, d),
            None => fb.blit_sprite(spr, bx, by),
        }
        self.animals[i].prev_rect = cur;
    }

    /// Draw the layer on top of a finished scene frame. `vp` must be the
    /// camera the scene was rasterised with; `depth` is the static layer's
    /// painter plane (`None` = no occlusion test). No allocation.
    pub fn draw(&mut self, fb: &mut Framebuffer, vp: &Viewport, depth: Option<&[u16]>) {
        let cam = Camera::for_viewport(vp);
        self.dirty_n = 0;
        self.box_n = 0;

        // B42: an animal that went inactive still owns the pixels it painted
        // last time. Report that box once so the caller can re-composite the
        // ground (or the sky) there — this is what stops a bird leaving a
        // dotted trail across the sky.
        for a in self.animals.iter_mut() {
            if a.active || a.prev_rect.is_empty() {
                continue;
            }
            if self.dirty_n < DIRTY_MAX {
                self.dirties[self.dirty_n] = a.prev_rect.expanded(1);
                self.dirty_n += 1;
            }
            a.prev_rect = Rect::EMPTY;
        }

        // Painter order: back (small x+y) to front. Insertion sort over at
        // most POOL stack entries — no allocation.
        let mut idx = [0usize; POOL];
        let mut n = 0usize;
        for i in 0..POOL {
            if self.animals[i].active {
                idx[n] = i;
                n += 1;
            }
        }
        for k in 1..n {
            let v = idx[k];
            let dv = (self.animals[v].x as i32) + (self.animals[v].y as i32);
            let mut j = k;
            while j > 0 {
                let u = idx[j - 1];
                if (self.animals[u].x as i32) + (self.animals[u].y as i32) <= dv {
                    break;
                }
                idx[j] = idx[j - 1];
                j -= 1;
            }
            idx[j] = v;
        }

        for &i in &idx[..n] {
            self.draw_one(i, fb, &cam, depth);
        }
    }

    /// Hand the layer the world's light (B60). Cheap: it stores one small
    /// struct, and every animal's shadow is drawn from it per frame.
    /// Tell the layer where the land line is on screen (see `land_y`).
    pub fn set_land_line(&mut self, y: i32) {
        self.land_y = y;
    }

    /// B80: tell the layer where the crop year stands (`field_stage`, 0..1) and
    /// whether it is winter, so the machines and the stock keep the season.
    pub fn set_farm(&mut self, stage: f32, winter: bool) {
        self.farm_stage = stage;
        self.winter = winter;
    }

    pub fn set_light(&mut self, ls: &render::light::LightState) {
        self.shadow = ls.actor();
    }

    /// Whether any animal is out right now (from the last `update`) — so the
    /// wallpaper knows the wildlife pass is worth ticking.
    pub fn is_active(&self) -> bool {
        self.any
    }

    /// Where the last `draw` touched, so the wallpaper only repaints those
    /// rectangles. Each rect is the sweep (old ∪ new box) of one animal; the
    /// caller must re-composite them from the scene *before* the next `draw`.
    /// Valid until the next `draw`.
    pub fn dirty_rects(&self) -> &[Rect] {
        &self.dirties[..self.dirty_n]
    }

    /// The boxes the last `draw` painted (one per live animal). Used by the
    /// trail test; unlike [`Self::dirty_rects`] these are the current
    /// positions, not the swept area.
    pub fn drawn_rects(&self) -> &[Rect] {
        &self.boxes[..self.box_n]
    }

    /// How many live animals per species (`Species::id()` order).
    pub fn counts(&self) -> [usize; 15] {
        let mut c = [0usize; 15];
        for a in &self.animals {
            if a.active {
                c[a.species.id() as usize] += 1;
            }
        }
        c
    }

    pub fn active_count(&self) -> usize {
        self.animals.iter().filter(|a| a.active).count()
    }

    /// Which species owns pool slot `i` (reports only).
    pub fn slot_species(i: usize) -> Species {
        SPECIES[i]
    }

    /// Active animals in tile space (tools/tests only — allocates).
    pub fn tile_positions(&self) -> Vec<(Species, f32, f32)> {
        self.animals
            .iter()
            .filter(|a| a.active)
            .map(|a| (a.species, a.x, a.y))
            .collect()
    }

    /// Active animals at their screen positions (tools only — allocates).
    pub fn positions(&self, vp: &Viewport) -> Vec<(Species, i32, i32)> {
        let cam = Camera::for_viewport(vp);
        self.animals
            .iter()
            .filter(|a| a.active)
            .map(|a| {
                let p = if a.perched {
                    cam.project(a.perch_x, a.perch_y, a.perch_z)
                } else {
                    cam.project(a.x, a.y, (a.z + a.alt as i32).max(0))
                };
                (a.species, p.0 as i32, p.1 as i32)
            })
            .collect()
    }

    /// A rolling hash of every slot's state (determinism tests: two runs of
    /// one seed must land on the same value).
    pub fn state_fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcb12f6a3;
        for a in &self.animals {
            h ^= a.x.to_bits() as u64 + 0x9e3779b97f4a7c15;
            h = h.wrapping_mul(0xff51afd7ed558ccd);
            h ^= a.y.to_bits() as u64 + 0xc2b2ae3d27d4eb4f;
            h = h.wrapping_mul(0xff51afd7ed558ccd);
            h ^= (a.frame as u64) | (a.active as u64) << 8 | (a.gid as u64) << 16;
        }
        h
    }

    /// The sprite for (species, stag, perched, frame) — perched gulls always
    /// show their perched frame, flight gulls alternate glide/beat, stags and
    /// does use their own half of the deer sheet, fish run their 3-frame
    /// splash.
    fn sprite_of(sheets: &Sheets, sp: Species, stag: bool, perched: bool, frame: usize) -> &Sprite {
        let s = sheets;
        match sp {
            Species::Bird => &s.bird[frame % 3],
            Species::Gull => &s.gull[if perched { 0 } else { 1 + frame % 2 }],
            Species::Duck => &s.duck[frame % 2],
            Species::Swan => &s.swan[frame % 2],
            Species::Fish => &s.fish[frame % 3],
            Species::Deer => &s.deer[if stag { 0 } else { 2 } + frame % 2],
            Species::Rabbit => &s.rabbit[frame % 3],
            Species::Cow => &s.cow[frame % 2],
            Species::Sheep => &s.sheep[frame % 2],
            Species::Horse => &s.horse[frame % 2],
            Species::Owl => &s.owl[frame % 2],
            Species::Bat => &s.bat[frame % 3],
            Species::Chicken => &s.chicken[frame % 2],
            Species::Tractor => &s.tractor[(if stag { 2 } else { 0 }) + frame % 2],
            Species::Combine => &s.combine[(if stag { 2 } else { 0 }) + frame % 2],
        }
    }
}

// --- free behaviour helpers (keep `update`'s borrow of `animals` clean) -----

/// Time-of-day mask per slot — a fixed rule of (species, gid), so the night
/// population is the same named animals every night (D44).
fn mask(a: &Animal, day: f32, stage: f32, winter: bool) -> bool {
    if a.gid == 255 {
        return false;
    }
    match a.species {
        Species::Bird | Species::Gull => day > 0.05,
        Species::Duck => day > 0.3 || a.gid % 2 == 0,
        Species::Swan => day > 0.3 || (a.gid as u32) < 2,
        Species::Fish => day > 0.25 && a.jump_e > 0.0,
        Species::Deer => day > 0.45 || a.gid % 3 == 0,
        Species::Rabbit => day > 0.45 || a.gid % 4 == 0,
        // B80: the stock is in the barn at night, and through the winter all but
        // a third of it (the hardy ones are out on a fine day).
        Species::Cow | Species::Sheep | Species::Horse => {
            day > 0.3 && (!winter || a.gid % 3 == 0)
        }
        Species::Chicken => day > 0.3 && !(winter && a.gid % 2 == 0),
        // The machines work in daylight: ploughing and sowing in the spring and
        // after the harvest, the combine when the crop is ripe.
        Species::Tractor => day > 0.95 && !winter && (stage < 0.22 || stage > 0.96) && (a.gid == 0 || stage < 0.14 || stage > 0.97),
        Species::Combine => day > 0.95 && (0.80..0.95).contains(&stage),
        Species::Owl => day < 0.4,
        Species::Bat => day < 0.5,
    }
}

/// A slot just went live: park it on its home tile (birds keep their
/// crossing position; perchers pick their perch first).
fn enter(a: &mut Animal, trees: &[(i32, i32, i32)], obstacles: &[CityObj]) {
    // `prev_rect` is deliberately left alone: if the animal was drawn before it
    // last went inactive, `draw` still has to report (and the caller restore)
    // that box. Drawing the new position unions with it, so nothing is missed.
    match a.species {
        Species::Bird => {}
        // B80: a machine resumes where it stopped, on its own rows.
        Species::Tractor | Species::Combine => {}
        Species::Gull if a.gid >= 4 => {
            pick_perch(trees, a, obstacles);
            a.perched = true;
        }
        Species::Owl => {
            // Spawn already put the owl on its tree; just make sure it is
            // drawn as perched from this moment.
            a.perched = true;
        }
        _ => {
            a.x = a.hx as f32 + 0.5 + a.rng.range_f32(-0.3, 0.3);
            a.y = a.hy as f32 + 0.5 + a.rng.range_f32(-0.3, 0.3);
            let ang = a.rng.f32() * TAU;
            a.dx = ang.cos();
            a.dy = ang.sin();
        }
    }
}

/// Pick a perch: a finished roof if the village has one, else a tree, else
/// the home surface.
fn pick_perch(trees: &[(i32, i32, i32)], a: &mut Animal, obstacles: &[CityObj]) {
    let n: usize = obstacles.iter().filter(|o| o.stage == Stage::Finished).count();
    if n > 0 {
        let pick = a.rng.usize(n);
        for (i, o) in obstacles.iter().enumerate() {
            if o.stage == Stage::Finished && i == pick {
                let base = o.level as i32 * HEIGHT_UNIT;
                // Roof top ≈ wall height + ridge. The renderer's exact
                // wall-height table is private to `render`, so mirror it
                // (B32: 8 px/storey, 10 px for 8+ storeys, min 12; the
                // ridge rise is clamped 6–14, so take ~10). A perched
                // gull forgives a pixel.
                let per = if o.storeys >= 8 { 10 } else { 8 };
                let wall = (o.storeys as i32 * per + 4).max(12);
                a.perch_x = o.x as f32 + 0.3 + a.rng.f32() * 0.4;
                a.perch_y = o.y as f32 + 0.3 + a.rng.f32() * 0.4;
                a.perch_z = base + wall + 10;
                return;
            }
        }
    }
    if !trees.is_empty() {
        let (x, y, lv) = trees[a.rng.usize(trees.len())];
        a.perch_x = x as f32 + 0.5;
        a.perch_y = y as f32 + 0.5;
        a.perch_z = lv * HEIGHT_UNIT + 30;
        return;
    }
    a.perch_x = a.hx as f32 + 0.5;
    a.perch_y = a.hy as f32 + 0.5;
    a.perch_z = a.z + 12;
}

/// The animation loop per species (B22b: the wing beat is three frames —
/// up, mid, down — so the flap reads at 64-px scale).
fn step_anim(a: &mut Animal, dt: f32) {
    let (period, frames) = match a.species {
        Species::Bird => (0.5, 3),
        Species::Gull => (0.7, 2),
        Species::Duck => (1.0, 2),
        Species::Swan => (1.3, 2),
        Species::Fish => (0.1, 3),
        Species::Deer => (2.4, 2),
        Species::Rabbit => (0.45, 3),
        Species::Cow => (1.7, 2),
        Species::Sheep => (1.9, 2),
        Species::Horse => (1.4, 2),
        Species::Owl => (2.2, 2),
        Species::Bat => (0.3, 3),
        Species::Chicken => (0.5, 2),
        Species::Tractor => (0.35, 2),
        Species::Combine => (0.35, 2),
    };
    a.frame_t += dt;
    if a.frame_t >= period {
        a.frame_t = a.frame_t % period;
        a.frame = (a.frame + 1) % frames as u8;
    }
}

#[allow(clippy::too_many_arguments)]
fn step_behavior(
    a: &mut Animal,
    dt: f32,
    map: &TileMap,
    obstacles: &[CityObj],
    fc: &[[f32; 2]; 3],
    w: f32,
    h: f32,
    trees: &[(i32, i32, i32)],
    arable: &[(i32, i32)],
) {
    match a.species {
        Species::Chicken => {
            flee_step(a, dt, map, obstacles, 0, 0.0, 1.0, 1.6)
        }
        Species::Tractor | Species::Combine => machine_step(a, dt, map, arable),
        Species::Bird => bird_step(a, dt, w, h, fc),
        Species::Gull => {
            if a.gid >= 4 {
                gull_perch_step(a, dt, obstacles, trees)
            } else {
                gull_circle_step(a, dt, map)
            }
        }
        Species::Duck | Species::Swan => wander_step(a, dt, map, obstacles, 1.6),
        Species::Fish => {} // the timer did all the work
        Species::Deer => flee_step(a, dt, map, obstacles, 4, 2.5, 8.0, 1.4),
        Species::Rabbit => flee_step(a, dt, map, obstacles, 2, 1.2, 5.0, 2.0),
        // B80: the stock does not shy from the farm's own buildings.
        Species::Cow | Species::Sheep | Species::Horse => {
            flee_step(a, dt, map, obstacles, 1, 1.5, 6.0, 2.2)
        }
        Species::Owl => {} // perched; the frame loop blinks it
        Species::Bat => bat_step(a, dt),
    }
}

/// One step of the fish timer: a jump is a 0.3 s / 3-frame splash at home,
/// then a random wait of 5–18 s. Runs even while the mask is off, so a jump
/// is armed the moment day breaks.
fn fish_timer(a: &mut Animal, dt: f32) {
    if a.jump_e > 0.0 {
        a.jump_e -= dt;
        if a.jump_e <= 0.0 {
            a.jump_e = 0.0;
            a.frame = 0;
            a.frame_t = 0.0;
            a.jump_t = a.rng.range_f32(5.0, 18.0);
        }
        return;
    }
    a.jump_t -= dt;
    if a.jump_t <= 0.0 {
        a.jump_e = 0.3;
        a.frame = 0;
        a.frame_t = 0.0;
    }
}

/// Flock crossing: a banding direction across the frame, weak cohesion
/// toward the flock centroid, and a soft up-down bob. No tile legality —
/// birds go over everything (the brief's one exception).
fn bird_step(a: &mut Animal, dt: f32, w: f32, h: f32, fc: &[[f32; 2]; 3]) {
    let [cx, cy] = fc[a.flock as usize];
    a.dx += (cx - a.x) * 0.06 * dt;
    a.dy += (cy - a.y) * 0.06 * dt;
    // Blend back toward the banding direction (1,-1)/√2 (up-right on screen).
    a.dx += (0.7071068 - a.dx) * 0.25 * dt;
    a.dy += (-0.7071068 - a.dy) * 0.25 * dt;
    let m = (a.dx * a.dx + a.dy * a.dy).sqrt().max(0.001);
    a.dx /= m;
    a.dy /= m;
    a.bob = (a.bob + dt * 3.0) % TAU;
    a.x += a.dx * a.speed * dt;
    a.y += a.dy * a.speed * dt;
    // Re-enter from the left edge beyond the other side of the map.
    if a.x > w + 3.0 || a.y < -4.0 || a.y > h + 3.0 {
        a.x = -3.0;
        a.y = a.rng.range(4, (h - 4.0) as i32).max(4) as f32;
    }
}

/// Circling gulls: a lazy orbit around their home water, altitude held.
fn gull_circle_step(a: &mut Animal, dt: f32, map: &TileMap) {
    a.bob = (a.bob + a.speed * dt) % TAU;
    let r = 0.9;
    let (nx, ny) = (a.hx as f32 + 0.5 + a.bob.cos() * r, a.hy as f32 + 0.5 + a.bob.sin() * r);
    if legal(map, &[], Species::Duck, nx, ny, false) {
        a.x = nx;
        a.y = ny;
    }
}

/// Perching gulls: perch 25–60 s, take off, circle ~6–14 s, re-perch.
fn gull_perch_step(a: &mut Animal, dt: f32, obstacles: &[CityObj], trees: &[(i32, i32, i32)]) {
    a.perch_t -= dt;
    if a.perched {
        if a.perch_t <= 0.0 {
            a.perched = false;
            a.perch_t = a.rng.range_f32(6.0, 14.0);
        }
    } else if a.perch_t <= 0.0 {
        pick_perch(trees, a, obstacles);
        a.perched = true;
        a.perch_t = a.rng.range_f32(25.0, 55.0);
    }
}

/// Bats: a slow loop around their roost, well below the birds.
fn bat_step(a: &mut Animal, dt: f32) {
    a.bob = (a.bob + a.speed * dt) % (2.0 * TAU);
    a.x = a.hx as f32 + 0.5 + a.bob.cos() * 4.0;
    a.y = a.hy as f32 + 0.5 + (a.bob * 0.5).sin() * 2.5;
}

/// A machine working a field: it drives along the rows, and at the end of a row
/// turns round and steps over to the next one. After a couple of minutes (or
/// when it runs out of field) it moves to another field. `bob` holds which way
/// the next row lies (+1 or -1).
fn machine_step(a: &mut Animal, dt: f32, map: &TileMap, arable: &[(i32, i32)]) {
    a.turn_in -= dt;
    let (nx, ny) = (a.x + a.dx * a.speed * dt, a.y + a.dy * a.speed * dt);
    let ok = |x: f32, y: f32| legal(map, &[], a.species, x, y, true);
    let mut relocate = a.turn_in <= 0.0;
    if !relocate {
        if ok(nx, ny) {
            a.x = nx;
            a.y = ny;
        } else {
            // End of the row: turn round, and over to the next row.
            a.dx = -a.dx;
            a.dy = -a.dy;
            let side = if a.bob < 0.0 { -0.5 } else { 0.5 };
            let (sx, sy) = if a.dx != 0.0 { (0.0, side) } else { (side, 0.0) };
            if ok(a.x + sx, a.y + sy) {
                a.x += sx;
                a.y += sy;
            } else if ok(a.x - sx, a.y - sy) {
                // The other edge of the field: work back the other way.
                a.bob = -a.bob;
                a.x -= sx;
                a.y -= sy;
            } else {
                relocate = true;
            }
        }
    }
    if relocate && !arable.is_empty() {
        let (fx, fy) = arable[a.rng.usize(arable.len())];
        a.x = fx as f32 + 0.5;
        a.y = fy as f32 + 0.5;
        a.hx = fx;
        a.hy = fy;
        let along_x = a.rng.chance(0.5);
        a.dx = if along_x { 1.0 } else { 0.0 };
        a.dy = if along_x { 0.0 } else { 1.0 };
        a.bob = if a.rng.chance(0.5) { 1.0 } else { -1.0 };
        a.turn_in = a.rng.range_f32(90.0, 200.0);
    }
    // Which way it faces on screen: +x and -y run right, -x and +y run left.
    a.stag = a.dx - a.dy < 0.0;
}

/// Random-wander with a home patch: turn now and then, and when the animal
/// strays past `home_r` tiles it steers straight home.
fn wander_step(a: &mut Animal, dt: f32, map: &TileMap, obstacles: &[CityObj], home_r: f32) {
    a.turn_in -= dt;
    if a.turn_in <= 0.0 {
        a.turn_in = a.rng.range_f32(1.5, 5.0);
        let ang = a.rng.f32() * TAU;
        a.dx = ang.cos();
        a.dy = ang.sin();
    }
    let (ddx, ddy) = (a.hx as f32 + 0.5 - a.x, a.hy as f32 + 0.5 - a.y);
    let d = (ddx * ddx + ddy * ddy).sqrt();
    if d > home_r {
        let m = d.max(0.001);
        a.dx = ddx / m;
        a.dy = ddy / m;
    }
    move_step(a, dt, map, obstacles, a.speed, false);
}

/// Grazers and farm stock: a coarse threat scan about once a second (lane or
/// building within `radius` tiles); a hit sets a short flee burst away from
/// the offender, otherwise the animal wanders its home patch.
fn flee_step(
    a: &mut Animal,
    dt: f32,
    map: &TileMap,
    obstacles: &[CityObj],
    radius: i32,
    flee_dur: f32,
    flee_mult: f32,
    home_r: f32,
) {
    if a.flee_t > 0.0 {
        a.flee_t -= dt;
        move_step(a, dt, map, obstacles, (a.speed * flee_mult).max(1.6), true);
        return;
    }
    a.threat_t -= dt;
    if a.threat_t <= 0.0 {
        a.threat_t = 1.0;
        if let Some((tx, ty)) = scan_threat(map, obstacles, a, radius) {
            a.threat_x = tx;
            a.threat_y = ty;
            a.flee_t = flee_dur;
            let dx = a.x - tx;
            let dy = a.y - ty;
            let m = (dx * dx + dy * dy).sqrt().max(0.001);
            a.dx = dx / m;
            a.dy = dy / m;
        }
    }
    wander_step(a, dt, map, obstacles, home_r);
}

/// Move and legality-check one frame. An illegal next tile rejects the move
/// and flips the heading (a paddler cannot step onto land, a grazer cannot
/// step onto a road or into a building).
fn move_step(a: &mut Animal, dt: f32, map: &TileMap, obstacles: &[CityObj], sp: f32, field_locked: bool) {
    let (nx, ny) = (a.x + a.dx * sp * dt, a.y + a.dy * sp * dt);
    let ok = legal(map, obstacles, a.species, nx, ny, field_locked);
    if ok {
        a.x = nx;
        a.y = ny;
    } else {
        a.dx = -a.dx;
        a.dy = -a.dy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bat's loop phase must stay small: kept growing for weeks in an f32 it
    /// stops resolving a 10 ms step and the bat freezes on its roost.
    #[test]
    fn a_bat_keeps_flying_after_weeks_of_nights() {
        let mut a = Animal::blank(Species::Bat, 0);
        a.speed = 0.8;
        a.bob = 600_000.0 % (2.0 * TAU);
        assert!(a.bob < 2.0 * TAU);
        let start = a.bob;
        for _ in 0..1000 {
            bat_step(&mut a, 0.01);
        }
        let moved = (a.bob - start).rem_euclid(2.0 * TAU);
        assert!((moved - 8.0).abs() < 0.05, "moved {moved} rad in 10 s at 0.8 rad/s");
        // And a bob that did grow large (an old save of the layer, a long run
        // before this fix) is brought back by the first step.
        a.bob = 1_000_000.0;
        bat_step(&mut a, 0.01);
        assert!(a.bob < 2.0 * TAU);
    }
    use map::Prop;
    use map::world_for_viewport;
    use map::Tile;
    use render::city::{ObjKind, Stage as BuildStage};

    /// A small world shaped for the tests: a lake left-of-centre, a paved
    /// road splitting the map vertically, a 4×4 field patch bottom-right and
    /// a handful of trees (so forest-edge meadow and perches exist).
    fn syn_map() -> TileMap {
        let vp = world_for_viewport(960, 540);
        let mut m = TileMap::fill(vp.w, vp.h, Tile::flat(4), 7);
        // Lake.
        for y in vp.h / 3..(2 * vp.h) / 3 {
            for x in vp.w / 6..vp.w / 3 {
                m.set(x, y, Tile::new(1, Kind::Water));
            }
        }
        // Field patch.
        for y in (2 * vp.h) / 3..(2 * vp.h) / 3 + 4 {
            for x in (5 * vp.w) / 6..(5 * vp.w) / 6 + 4 {
                let mut t = Tile::new(4, Kind::Field);
                map::set_crop(&mut t.aux, map::Crop::Pasture);
                m.set(x, y, t);
            }
        }
        // A farmyard beside it (hens), and an arable field for the machines.
        for y in (2 * vp.h) / 3 + 5..(2 * vp.h) / 3 + 8 {
            for x in (5 * vp.w) / 6..(5 * vp.w) / 6 + 4 {
                let mut t = Tile::new(4, Kind::Field);
                map::set_crop(&mut t.aux, map::Crop::Track);
                m.set(x, y, t);
            }
        }
        for y in vp.h / 8..vp.h / 8 + 6 {
            for x in (4 * vp.w) / 5..(4 * vp.w) / 5 + 8 {
                let mut t = Tile::new(4, Kind::Field);
                map::set_crop(&mut t.aux, map::Crop::Cereal);
                m.set(x, y, t);
            }
        }
        // Road down the middle.
        for y in 0..vp.h {
            for x in vp.w / 2..vp.w / 2 + 1 {
                m.set(x, y, Tile::new(4, Kind::Paved));
            }
        }
        // Trees (all on plain grass).
        for &(x, y) in &[(3i32, 3), (4, 3), (3, 4), (5, 5), (vp.w - 8, 3)] {
            let mut t = m.at(x, y);
            t.prop = Prop::Tree {
                species: map::TreeSpecies::Oak,
                size: 1,
                variant: 0,
            };
            t.kind = Kind::Grass;
            m.set(x, y, t);
        }
        m
    }

    fn obj(x: i32, y: i32) -> CityObj {
        CityObj {
            x,
            y,
            w: 1,
            h: 1,
            level: 4,
            kind: ObjKind::House,
            storeys: 1,
            style: 0,
            stage: BuildStage::Finished,
            seed: 1,
            front: 0,
            t: 0.0,
            kit: 0, wear: Default::default(),
        }
    }

    fn run(wl: &mut Wildlife, m: &TileMap, obs: &[CityObj], tod: f32, secs: f32) {
        let dt = 1.0 / 30.0;
        for _ in 0..(secs * 30.0) as usize {
            wl.update(dt, m, obs, tod);
        }
    }

    #[test]
    fn water_species_stay_on_water() {
        let m = syn_map();
        let obs = vec![obj(40, 40)];
        let mut wl = Wildlife::new(1, &m);
        run(&mut wl, &m, &obs, 12.0, 30.0);
        for (sp, x, y) in wl.tile_positions() {
            if sp.in_water() {
                let (tx, ty) = (x as i32, y as i32);
                assert!(m.in_bounds(tx, ty), "{sp} out of bounds at {x},{y}");
                assert!(m.at(tx, ty).is_water(), "{sp} on dry land at {x},{y}");
            }
        }
    }

    #[test]
    fn land_species_stay_off_roads_and_buildings() {
        let m = syn_map();
        let obs = vec![obj(40, 40)];
        let mut wl = Wildlife::new(1, &m);
        run(&mut wl, &m, &obs, 12.0, 30.0);
        for (sp, x, y) in wl.tile_positions() {
            if sp.grazer() || sp.farm() {
                let (tx, ty) = (x as i32, y as i32);
                assert!(m.in_bounds(tx, ty), "{sp} out of bounds at {x},{y}");
                let t = m.at(tx, ty);
                assert!(
                    !t.kind.is_lane() && t.kind != Kind::Sidewalk,
                    "{sp} standing on a road at {tx},{ty}"
                );
                assert!(!in_obstacle(&obs, tx, ty), "{sp} inside a building at {tx},{ty}");
            }
        }
    }

    #[test]
    fn farm_stock_stays_in_fields() {
        let m = syn_map();
        let mut wl = Wildlife::new(1, &m);
        run(&mut wl, &m, &[], 12.0, 30.0);
        let c = wl.counts();
        let farm = c[Species::Cow.id() as usize]
            + c[Species::Sheep.id() as usize]
            + c[Species::Horse.id() as usize];
        assert!(farm > 0, "farm stock should be out at noon (field exists)");
        for (sp, x, y) in wl.tile_positions() {
            if sp.farm() {
                let (tx, ty) = (x as i32, y as i32);
                assert!(
                    m.in_bounds(tx, ty) && m.at(tx, ty).kind == Kind::Field,
                    "{sp} out of the field at {tx},{ty}"
                );
                // B80: stock stays in its *pasture*, hens in the yard.
                let crop = map::crop_of(&m.at(tx, ty));
                let want = if sp == Species::Chicken { map::Crop::Track } else { map::Crop::Pasture };
                assert_eq!(crop, want, "{sp} left its own ground at {tx},{ty}");
            }
        }
    }

    #[test]
    fn stock_is_in_the_barn_at_night_and_in_winter() {
        let m = syn_map();
        let stock = |c: &[usize; 15]| {
            c[Species::Cow.id() as usize] + c[Species::Sheep.id() as usize] + c[Species::Horse.id() as usize]
        };
        let mut noon = Wildlife::new(1, &m);
        run(&mut noon, &m, &[], 12.0, 10.0);
        assert!(stock(&noon.counts()) > 0);
        let mut night = Wildlife::new(1, &m);
        run(&mut night, &m, &[], 23.0, 10.0);
        assert_eq!(stock(&night.counts()), 0, "the stock is in at night");
        let mut winter = Wildlife::new(1, &m);
        winter.set_farm(0.02, true);
        run(&mut winter, &m, &[], 12.0, 10.0);
        assert!(stock(&winter.counts()) < stock(&noon.counts()), "most of the stock is in through the winter");
    }

    #[test]
    fn machines_work_their_field_in_season() {
        let m = syn_map();
        let machines = |c: &[usize; 15]| (c[Species::Tractor.id() as usize], c[Species::Combine.id() as usize]);
        // Ploughing time: tractors out, no combine.
        let mut spring = Wildlife::new(1, &m);
        spring.set_farm(0.10, false);
        run(&mut spring, &m, &[], 10.0, 30.0);
        let (t, c) = machines(&spring.counts());
        assert!(t > 0 && c == 0, "spring: tractors {t}, combines {c}");
        // Harvest: the combine is out, the tractors are not.
        let mut harvest = Wildlife::new(1, &m);
        harvest.set_farm(0.86, false);
        run(&mut harvest, &m, &[], 14.0, 30.0);
        let (t, c) = machines(&harvest.counts());
        assert!(c > 0 && t == 0, "harvest: tractors {t}, combines {c}");
        // Midsummer: neither.
        let mut summer = Wildlife::new(1, &m);
        summer.set_farm(0.55, false);
        run(&mut summer, &m, &[], 12.0, 30.0);
        assert_eq!(machines(&summer.counts()), (0, 0));
        // They never leave their field.
        for (sp, x, y) in harvest.tile_positions().into_iter().chain(spring.tile_positions()) {
            if sp.machine() {
                let t = m.at(x as i32, y as i32);
                assert!(t.kind == Kind::Field && map::crop_of(&t).is_arable(), "{sp} off its field");
            }
        }
    }

    #[test]
    fn night_swaps_the_population() {
        let m = syn_map();
        let obs: Vec<CityObj> = Vec::new();
        let mut day = Wildlife::new(3, &m);
        run(&mut day, &m, &obs, 12.0, 5.0);
        let mut night = Wildlife::new(3, &m);
        run(&mut night, &m, &obs, 23.0, 5.0);
        let dc = day.counts();
        let nc = night.counts();
        assert_eq!(dc[Species::Bird.id() as usize], 12, "flocks at noon");
        assert_eq!(nc[Species::Bird.id() as usize], 0, "no flocks at night");
        assert_eq!(dc[Species::Owl.id() as usize], 0);
        assert_eq!(nc[Species::Owl.id() as usize], 2, "owls at night (trees exist)");
        assert_eq!(dc[Species::Bat.id() as usize], 0);
        assert_eq!(nc[Species::Bat.id() as usize], 4, "bats at night");
        assert_eq!(dc[Species::Deer.id() as usize], 8, "all deer at noon");
        assert_eq!(nc[Species::Deer.id() as usize], 3, "fewer deer at night");
        assert!(day.is_active() && night.is_active());
    }

    #[test]
    fn same_seed_same_run() {
        let m = syn_map();
        let obs = vec![obj(40, 40)];
        let mut a = Wildlife::new(9, &m);
        let mut b = Wildlife::new(9, &m);
        let mut c = Wildlife::new(10, &m);
        for _ in 0..900 {
            a.update(1.0 / 30.0, &m, &obs, 12.0);
            b.update(1.0 / 30.0, &m, &obs, 12.0);
            c.update(1.0 / 30.0, &m, &obs, 12.0);
        }
        assert_eq!(a.state_fingerprint(), b.state_fingerprint(), "same seed must agree");
        assert_eq!(a.tile_positions(), b.tile_positions());
        assert_ne!(a.state_fingerprint(), c.state_fingerprint(), "different seeds must differ");
    }

    #[test]
    fn draw_marks_dirty_rects_and_paints() {
        let m = syn_map();
        let vp = world_for_viewport(960, 540);
        let obs: Vec<CityObj> = Vec::new();
        let mut wl = Wildlife::new(1, &m);
        run(&mut wl, &m, &obs, 12.0, 5.0);
        let mut fb = Framebuffer::new(960, 540);
        let bg = render::palette::rgb(0x64, 0x78, 0x8c);
        fb.clear(bg);
        wl.draw(&mut fb, &vp, None);
        assert!(!wl.dirty_rects().is_empty(), "draw must report dirty rects");
        let painted = fb.px().iter().filter(|&&p| p != bg).count();
        assert!(painted > 10, "animals painted only {painted} px");
    }
}
