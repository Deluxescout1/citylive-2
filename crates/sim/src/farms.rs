//! B80 (D84): the countryside. Farmland round the town: patchwork fields of
//! different crops, pastures for the stock, farm tracks, and a farmstead (a
//! farmhouse and a barn round a yard) in the middle of each farm.
//!
//! The farms are laid once, when a world is founded, from the seed alone:
//! the layout is a pure function of the map, the founding spot and the seed,
//! so a given world always grows the same farms. What it writes is ordinary
//! sim state: `Kind::Field` tiles (with the crop in the tile's `aux`, see
//! `map::Crop`), `Field` rectangles, and two lots per farmstead — so a save
//! carries it with the tiles it already stores and a load replays it. The
//! town then eats into the farmland the way it always has (`grade_lot` and the
//! lane writers turn a field tile back into grass); `tidy_fields` removes the
//! slivers that leaves behind, so the edge of the town reads as fields giving
//! way to streets rather than as stray bits of crop between the houses.
//!
//! Which crops a land grows is the land's business ([`Profile`]): wheat, maize
//! and orchards in the temperate lands, rice in the tropics, vines and olives
//! round the Mediterranean, sheep on the uplands, date palms at a desert oasis.

use map::rng::Rng;
use map::{Biome, Crop, Kind, Tile};

use crate::{Field, Lot, LotKind, Sim, Stage};

/// Style id of a farmstead's house (`District::style()` runs 0..=5).
pub const FARM_STYLE: u8 = 6;

static DBG: [std::sync::atomic::AtomicUsize; 2] = [std::sync::atomic::AtomicUsize::new(0), std::sync::atomic::AtomicUsize::new(0)];

/// A rectangle of tiles.
#[derive(Clone, Copy, Debug)]
struct R {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// What a land grows, and how much of it is farmed.
struct Profile {
    /// Crops and their weights. `Pasture` is the stock's grass.
    crops: &'static [(Crop, f32)],
    /// Farms per hundred open tiles (a rough density).
    density: f32,
    /// Largest leaf of the patchwork (tiles): small plots on the islands.
    leaf_max: i32,
    /// Farms need water within this many tiles (a desert's oasis). 0 = no need.
    near_water: i32,
    /// Tiles round the founding spot kept free (the land the town grows on).
    keep_clear: i32,
}

fn profile(b: Biome) -> Option<Profile> {
    use Crop::*;
    Some(match b {
        Biome::Temperate | Biome::Lagoon => Profile {
            crops: &[(Cereal, 4.0), (Maize, 2.4), (Veg, 1.8), (Orchard, 1.6), (Pasture, 1.6)],
            density: 1.1,
            leaf_max: 8,
            near_water: 0,
            keep_clear: KEEP_CLEAR,
        },
        Biome::Mediterranean => Profile {
            crops: &[(Cereal, 2.0), (Vineyard, 3.4), (Orchard, 3.0), (Veg, 1.0), (Pasture, 1.8)],
            density: 0.55,
            leaf_max: 8,
            near_water: 0,
            keep_clear: KEEP_CLEAR,
        },
        Biome::Volcanic => Profile {
            crops: &[(Vineyard, 3.0), (Veg, 2.0), (Orchard, 1.0), (Pasture, 1.5)],
            density: 0.40,
            leaf_max: 7,
            near_water: 0,
            keep_clear: KEEP_CLEAR,
        },
        Biome::Tropical | Biome::Bamboo | Biome::Swamp => Profile {
            crops: &[(Paddy, 5.0), (Maize, 1.0), (Veg, 1.6), (Orchard, 1.6), (Pasture, 0.8)],
            density: 0.52,
            leaf_max: 8,
            near_water: 0,
            keep_clear: KEEP_CLEAR,
        },
        Biome::Savanna => Profile {
            crops: &[(Maize, 2.6), (Cereal, 1.4), (Veg, 0.8), (Pasture, 4.0)],
            density: 0.50,
            leaf_max: 9,
            near_water: 0,
            keep_clear: KEEP_CLEAR,
        },
        Biome::Steppe => Profile {
            crops: &[(Cereal, 5.0), (Maize, 0.8), (Pasture, 3.0)],
            density: 0.55,
            leaf_max: 9,
            near_water: 0,
            keep_clear: KEEP_CLEAR,
        },
        Biome::Highlands | Biome::Alpine => Profile {
            crops: &[(Pasture, 6.0), (Cereal, 1.0), (Veg, 0.7)],
            density: 0.46,
            leaf_max: 8,
            near_water: 0,
            keep_clear: KEEP_CLEAR,
        },
        // Islands: small plots and the goats.
        Biome::Islands => Profile {
            crops: &[(Veg, 2.0), (Orchard, 2.0), (Cereal, 1.0), (Pasture, 2.5)],
            density: 0.20,
            leaf_max: 5,
            near_water: 0,
            keep_clear: 20,
        },
        // A desert farms its oasis and nothing else: dates and a vegetable bed.
        Biome::Desert => Profile {
            crops: &[(Orchard, 3.0), (Veg, 2.0), (Pasture, 1.0)],
            density: 0.5,
            leaf_max: 5,
            near_water: 4,
            keep_clear: KEEP_CLEAR,
        },
        // Reindeer and nothing: the tundra grows no crop.
        Biome::Tundra => return None,
    })
}

/// How many tiles one side of a map must have before it is farmed (the tiny
/// maps the unit tests use are left as they were).
const MIN_FARMED_SIDE: i32 = 100;

/// Tiles within this many of the founding spot stay open for the first village.
const KEEP_CLEAR: i32 = 9;

/// Whether the tile may be farmed: open, flat-ish, dry, unclaimed ground.
fn open_ground(sim: &Sim, x: i32, y: i32, claim: &[u8]) -> bool {
    let m = &sim.map;
    if x < 4 || y < 4 || x >= m.w - 4 || y >= m.h - 4 {
        return false;
    }
    if claim[(y * m.w + x) as usize] != 0 {
        return false;
    }
    let t = m.at(x, y);
    // Only land the frame shows, and below the land line: a farm nobody sees is
    // wasted work.
    let z = t.min_level() as i32 * map::iso_consts::HEIGHT_UNIT;
    if !sim.in_frame(x, y, z, 24) {
        return false;
    }
    // The belt under the land line stays countryside as it is (a house there
    // would be cut by the horizon, B42f).
    if sim.behind_horizon(x, y) {
        return false;
    }
    t.relief() <= 1
        && (matches!(t.kind, Kind::Grass | Kind::Meadow | Kind::DryGrass | Kind::Upland)
            || (sim.biome == Biome::Desert && t.kind == Kind::Sand))
        && t.aux & (map::AUX_TECH | map::AUX_BRIDGE | map::AUX_HIGHWAY | map::AUX_RAMP) == 0
        && !sim.lots.iter().any(|l| l.covers(x, y))
}

fn water_within(m: &map::TileMap, x: i32, y: i32, r: i32) -> bool {
    (-r..=r).any(|dy| (-r..=r).any(|dx| m.in_bounds(x + dx, y + dy) && m.at(x + dx, y + dy).is_water()))
}

/// Every tile of `r` is open, flat and on one level (arable needs a flat bed;
/// pasture may follow the lie of the land).
fn leaf_ok(sim: &Sim, r: R, claim: &[u8], flat: bool, near_water: i32) -> bool {
    let (mut lo, mut hi) = (u8::MAX, 0u8);
    let mut trees = 0;
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            if !open_ground(sim, x, y, claim) {
                return false;
            }
            let t = sim.map.at(x, y);
            lo = lo.min(t.min_level());
            hi = hi.max(t.max_level());
            if t.prop.is_tree() {
                trees += 1;
            }
        }
    }
    // An arable bed is levelled to its lowest corner: it may only be a step
    // out from one end to the other.
    if flat && hi.saturating_sub(lo) > 1 {
        return false;
    }
    if near_water > 0 && !water_within(&sim.map, r.x + r.w / 2, r.y + r.h / 2, near_water + r.w.max(r.h) / 2) {
        return false;
    }
    // A plot is cleared out of the open meadow, not hacked out of a wood.
    trees * 5 <= r.w * r.h * 3
}

fn pick_crop(rng: &mut Rng, p: &Profile, avoid: &[Crop], r: R, near_yard: bool) -> Crop {
    // The fields touching the yard are the stock's.
    if near_yard && p.crops.iter().any(|(c, _)| *c == Crop::Pasture) && rng.chance(0.75) {
        return Crop::Pasture;
    }
    let total: f32 = p.crops.iter().map(|(_, w)| *w).sum();
    for _ in 0..6 {
        let mut roll = rng.range_f32(0.0, total);
        let mut pick = p.crops[0].0;
        for (c, w) in p.crops {
            if roll < *w {
                pick = *c;
                break;
            }
            roll -= *w;
        }
        let big_enough = match pick {
            Crop::Orchard => r.w >= 4 && r.h >= 4,
            Crop::Vineyard => r.w >= 4 && r.h >= 3,
            _ => true,
        };
        if big_enough && !avoid.contains(&pick) {
            return pick;
        }
    }
    p.crops[0].0
}

/// Split a rectangle into the leaves of a patchwork: wide fields and narrow
/// ones, never smaller than 3 tiles a side.
fn patchwork(rng: &mut Rng, r: R, leaf_max: i32, depth: u32, out: &mut Vec<R>) {
    if r.w < 3 || r.h < 3 {
        return;
    }
    let cap = leaf_max.max(4);
    let fits = r.w <= cap && r.h <= cap;
    let stop = depth >= 4 || (r.w < 6 && r.h < 6) || (fits && (depth > 0 && rng.chance(0.45)));
    if stop || (fits && rng.chance(0.7)) {
        out.push(r);
        return;
    }
    let split_w = if r.w >= 2 * 3 && r.h >= 2 * 3 {
        r.w >= r.h
    } else {
        r.w >= 6
    };
    if split_w && r.w >= 6 {
        let cut = rng.range(3, r.w - 3);
        let a = R { w: cut, ..r };
        let b = R { x: r.x + cut, w: r.w - cut, ..r };
        patchwork(rng, a, leaf_max, depth + 1, out);
        patchwork(rng, b, leaf_max, depth + 1, out);
    } else if r.h >= 6 {
        let cut = rng.range(3, r.h - 3);
        let a = R { h: cut, ..r };
        let b = R { y: r.y + cut, h: r.h - cut, ..r };
        patchwork(rng, a, leaf_max, depth + 1, out);
        patchwork(rng, b, leaf_max, depth + 1, out);
    } else {
        out.push(r);
    }
}

impl Sim {
    /// Lay the farms of a fresh world (B80).
    pub(crate) fn seed_countryside(&mut self) {
        let Some(profile) = profile(self.biome) else {
            return;
        };
        let (mw, mh) = (self.map.w, self.map.h);
        if mw.min(mh) < MIN_FARMED_SIDE {
            return;
        }
        let mut rng = Rng::new(self.seed ^ 0xFA57_0B80_C0DE_1984);
        // What is already claimed (a field, a track, a yard).
        let mut claim = vec![0u8; (mw * mh) as usize];
        // Pre-claim the first village's room.
        let (fx, fy) = self.founding;
        let kc = profile.keep_clear;
        for y in (fy - kc).max(0)..(fy + kc).min(mh) {
            for x in (fx - kc).max(0)..(fx + kc).min(mw) {
                claim[(y * mw + x) as usize] = 1;
            }
        }
        // How much open ground there is decides how many farms.
        let mut open = 0usize;
        for y in 0..mh {
            for x in 0..mw {
                if open_ground(self, x, y, &claim) {
                    open += 1;
                }
            }
        }
        let want = ((open as f32 * profile.density / 100.0).round() as usize).clamp(2, 60);
        let spacing = if profile.leaf_max <= 5 { 9 } else { 12 };
        let mut open_tiles: Vec<(i32, i32)> = Vec::with_capacity(open);
        for y in 0..mh {
            for x in 0..mw {
                if open_ground(self, x, y, &claim) {
                    open_tiles.push((x, y));
                }
            }
        }
        if open_tiles.is_empty() {
            return;
        }
        let mut centres: Vec<(i32, i32)> = Vec::new();
        let mut farms = 0usize;
        for _ in 0..want * 60 {
            if farms >= want {
                break;
            }
            let (cx, cy) = open_tiles[rng.usize(open_tiles.len())];
            if centres.iter().any(|&(x, y)| (x - cx).abs() < spacing && (y - cy).abs() < spacing) {
                continue;
            }
            if self.lay_farm(&mut rng, &profile, &mut claim, cx, cy) {
                centres.push((cx, cy));
                farms += 1;
            }
        }
        if std::env::var("B80_DEBUG").is_ok() {
            let mut n = [0usize; 8];
            for y in 0..mh {
                for x in 0..mw {
                    let t = self.map.at(x, y);
                    if t.kind == Kind::Field {
                        n[map::crop_of(&t) as usize] += 1;
                    }
                }
            }
            eprintln!("field tiles by crop {n:?}");
            eprintln!("yard fails {} few fields {}", DBG[0].load(std::sync::atomic::Ordering::Relaxed), DBG[1].load(std::sync::atomic::Ordering::Relaxed));
            eprintln!("countryside: open {open}, want {want}, laid {farms}, fields {}", self.fields.len());
        }
    }

    /// One farm around `(cx, cy)`: the yard in the middle, the patchwork
    /// round it. Returns whether it fitted.
    fn lay_farm(&mut self, rng: &mut Rng, p: &Profile, claim: &mut [u8], cx: i32, cy: i32) -> bool {
        let mw = self.map.w;
        // The yard: 6x5 or 5x6 tiles, a house at one end and a barn at the other.
        let along_x = rng.chance(0.5);
        let (yw, yh) = if along_x { (6, 5) } else { (5, 6) };
        let yard = R { x: cx - yw / 2, y: cy - yh / 2, w: yw, h: yh };
        if !leaf_ok(self, yard, claim, false, p.near_water) {
            DBG[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        // Strips round the yard: left/right/top/bottom, each 4..9 tiles deep.
        let depth = |rng: &mut Rng| rng.range(4, p.leaf_max.max(5) + 1);
        let (dl, dr, dt, db) = (depth(rng), depth(rng), depth(rng), depth(rng));
        let left = R { x: yard.x - dl, y: yard.y - dt, w: dl, h: yard.h + dt + db };
        let right = R { x: yard.x + yard.w, y: yard.y - dt, w: dr, h: yard.h + dt + db };
        let top = R { x: yard.x, y: yard.y - dt, w: yard.w, h: dt };
        let bottom = R { x: yard.x, y: yard.y + yard.h, w: yard.w, h: db };
        // The exit track runs out through one strip, along its middle.
        let exit = rng.range(0, 4);
        let mut strips: Vec<R> = Vec::new();
        let mut tracks: Vec<R> = Vec::new();
        for (i, s) in [left, right, top, bottom].into_iter().enumerate() {
            if i as i32 == exit {
                // Left/right strips are walked along x, top/bottom along y.
                if i < 2 {
                    let mid = s.y + s.h / 2;
                    // The lane leaves from the yard's own row (the yard's middle).
                    let row = yard.y + yard.h / 2;
                    let _ = mid;
                    tracks.push(R { x: s.x, y: row, w: s.w, h: 1 });
                    strips.push(R { h: row - s.y, ..s });
                    strips.push(R { y: row + 1, h: s.y + s.h - row - 1, ..s });
                } else {
                    let col = yard.x + yard.w / 2;
                    tracks.push(R { x: col, y: s.y, w: 1, h: s.h });
                    strips.push(R { w: col - s.x, ..s });
                    strips.push(R { x: col + 1, w: s.x + s.w - col - 1, ..s });
                }
            } else {
                strips.push(s);
            }
        }
        let mut leaves: Vec<R> = Vec::new();
        for s in strips {
            patchwork(rng, s, p.leaf_max, 0, &mut leaves);
        }
        // The yard is a farm only if something round it can be farmed.
        let mut placed: Vec<(R, Crop)> = Vec::new();
        let mut grid_crop: std::collections::HashMap<(i32, i32), Crop> = std::collections::HashMap::new();
        let mut pastures = 0;
        for leaf in &leaves {
            let r = *leaf;
            let near_yard = r.x < yard.x + yard.w + 1
                && r.x + r.w > yard.x - 1
                && r.y < yard.y + yard.h + 1
                && r.y + r.h > yard.y - 1;
            // Neighbours already placed: do not repeat their crop.
            let mut avoid: Vec<Crop> = Vec::new();
            for y in r.y - 1..=r.y + r.h {
                for x in r.x - 1..=r.x + r.w {
                    if let Some(c) = grid_crop.get(&(x, y)) {
                        if !avoid.contains(c) {
                            avoid.push(*c);
                        }
                    }
                }
            }
            let mut crop = pick_crop(rng, p, &avoid, r, near_yard && pastures < 2);
            if crop == Crop::Pasture && pastures >= 2 {
                crop = pick_crop(rng, p, &[avoid.as_slice(), &[Crop::Pasture]].concat(), r, false);
            }
            if crop == Crop::Pasture {
                pastures += 1;
            }
            let flat = crop != Crop::Pasture;
            let mut r = r;
            if !leaf_ok(self, r, claim, flat, 0) {
                // Trim a stray row off the bad side before giving the leaf up.
                let mut ok = false;
                for (dx, dy, dw, dh) in [(0, 0, -1, 0), (0, 0, 0, -1), (1, 0, -1, 0), (0, 1, 0, -1)] {
                    let t = R { x: r.x + dx, y: r.y + dy, w: r.w + dw, h: r.h + dh };
                    if t.w >= 3 && t.h >= 3 && leaf_ok(self, t, claim, flat, 0) {
                        r = t;
                        ok = true;
                        break;
                    }
                }
                if !ok {
                    continue;
                }
            }
            for y in r.y..r.y + r.h {
                for x in r.x..r.x + r.w {
                    grid_crop.insert((x, y), crop);
                    claim[(y * mw + x) as usize] = 2;
                }
            }
            placed.push((r, crop));
        }
        // A farm needs at least two fields round the yard.
        if placed.len() < 2 {
            DBG[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        // Commit: the fields...
        for (r, crop) in &placed {
            self.plough_rect(*r, *crop);
            if crop.is_arable() || *crop == Crop::Pasture {
                self.fields.push(Field { x: r.x, y: r.y, w: r.w, h: r.h });
            }
        }
        // ...the track...
        for t in &tracks {
            for y in t.y..t.y + t.h {
                for x in t.x..t.x + t.w {
                    let tile = self.map.at(x, y);
                    if matches!(tile.kind, Kind::Grass | Kind::Meadow | Kind::DryGrass | Kind::Upland)
                        && tile.relief() <= 1
                        && claim[(y * mw + x) as usize] <= 1
                    {
                        self.lay_field_tile(x, y, Crop::Track, tile.min_level());
                        claim[(y * mw + x) as usize] = 3;
                    }
                }
            }
        }
        // ...and the yard: packed earth, a farmhouse and a barn, on one level.
        let mut level = u8::MAX;
        for y in yard.y..yard.y + yard.h {
            for x in yard.x..yard.x + yard.w {
                level = level.min(self.map.at(x, y).min_level());
            }
        }
        for y in yard.y..yard.y + yard.h {
            for x in yard.x..yard.x + yard.w {
                self.lay_field_tile(x, y, Crop::Track, level);
                claim[(y * mw + x) as usize] = 3;
            }
        }
        let (house, barn, front_house, front_barn) = if along_x {
            // House at the -x end, barn at the +x end; both face the yard's middle.
            (
                R { x: yard.x, y: yard.y + rng.range(0, 3), w: 2, h: 2 },
                R { x: yard.x + yard.w - 2, y: yard.y + rng.range(0, 3), w: 2, h: 2 },
                0u8,
                2u8,
            )
        } else {
            (
                R { x: yard.x + rng.range(0, 3), y: yard.y, w: 2, h: 2 },
                R { x: yard.x + rng.range(0, 3), y: yard.y + yard.h - 2, w: 2, h: 2 },
                1u8,
                3u8,
            )
        };
        let age_days = rng.range(40, 260) as f64;
        for (r, kind, front, storeys, style) in [
            (house, LotKind::Farmhouse, front_house, 2u8, FARM_STYLE),
            (barn, LotKind::Barn, front_barn, 1u8, FARM_STYLE),
        ] {
            self.lots.push(Lot {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
                level,
                kind,
                storeys,
                style,
                seed: rng.next_u32(),
                front,
                stage: Stage::Finished,
                stage_end: 0.0,
                started: -age_days * 86_400.0,
            });
        }
        true
    }

    /// Plough a rectangle for `crop`.
    fn plough_rect(&mut self, r: R, crop: Crop) {
        let mut bed = u8::MAX;
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                bed = bed.min(self.map.at(x, y).min_level());
            }
        }
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let level = bed;
                if crop == Crop::Pasture {
                    // Pasture follows the land: keep the corners, change the cover.
                    let mut t = self.map.at(x, y);
                    t.kind = Kind::Field;
                    t.prop = map::Prop::None;
                    map::set_crop(&mut t.aux, crop);
                    self.write(x, y, t);
                } else {
                    self.lay_field_tile(x, y, crop, level);
                }
            }
        }
    }

    fn lay_field_tile(&mut self, x: i32, y: i32, crop: Crop, level: u8) {
        let mut t: Tile = self.map.at(x, y);
        t.kind = Kind::Field;
        t.prop = map::Prop::None;
        t.corner = [level; 4];
        map::set_crop(&mut t.aux, crop);
        self.write(x, y, t);
    }

    /// B80: remove what the town's growth leaves of a farm: a field the streets
    /// cut to a sliver (a strip under three tiles wide, or a scrap under eight
    /// tiles) goes back to grass, so the edge of the town reads as fields giving
    /// way to streets. Looks only at the field tiles beside a tile that was
    /// just taken, so it costs nothing while the town is not growing.
    pub(crate) fn tidy_fields(&mut self, taken: &[(i32, i32)]) {
        let mut done: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
        for &(ax, ay) in taken {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let start = (ax + dx, ay + dy);
                if !self.map.in_bounds(start.0, start.1) || done.contains(&start) {
                    continue;
                }
                let t = self.map.at(start.0, start.1);
                if t.kind != Kind::Field {
                    continue;
                }
                let crop = map::crop_of(&t);
                if crop == Crop::Track {
                    // A track with no field left beside it leads nowhere.
                    let alone = ![(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(ox, oy)| {
                        self.map.at(start.0 + ox, start.1 + oy).kind == Kind::Field
                    });
                    if alone {
                        self.release_field_tile(start.0, start.1);
                    }
                    continue;
                }
                // The connected patch of this crop (capped: a big field is never a sliver).
                let mut comp = vec![start];
                let mut seen = std::collections::HashSet::new();
                seen.insert(start);
                let mut i = 0;
                let (mut x0, mut y0, mut x1, mut y1) = (start.0, start.1, start.0, start.1);
                while i < comp.len() && comp.len() < 160 {
                    let (cx, cy) = comp[i];
                    i += 1;
                    for (ox, oy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let n = (cx + ox, cy + oy);
                        if !self.map.in_bounds(n.0, n.1) || seen.contains(&n) {
                            continue;
                        }
                        let nt = self.map.at(n.0, n.1);
                        if nt.kind == Kind::Field && map::crop_of(&nt) == crop {
                            seen.insert(n);
                            comp.push(n);
                            x0 = x0.min(n.0);
                            y0 = y0.min(n.1);
                            x1 = x1.max(n.0);
                            y1 = y1.max(n.1);
                        }
                    }
                }
                for c in &comp {
                    done.insert(*c);
                }
                if comp.len() >= 160 {
                    continue;
                }
                let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
                if bw.min(bh) < 3 || comp.len() < 8 {
                    for (x, y) in comp {
                        self.release_field_tile(x, y);
                    }
                }
            }
        }
    }

    /// Hand one tile of farmland back to the grass.
    fn release_field_tile(&mut self, x: i32, y: i32) {
        let mut t = self.map.at(x, y);
        t.kind = Kind::Grass;
        t.aux &= !map::AUX_CROP_MASK;
        self.write(x, y, t);
        self.drop_field(x, y);
    }
}
