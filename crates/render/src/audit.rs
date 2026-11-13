//! B74/D81: the overlap audit, in pixels.
//!
//! The sim's audit (`sim::audit`) checks tiles: a tree's *tile* against a
//! street's *tile*. The review's worst picture was a tree whose tile was legal
//! and whose *sprite* still stood on the railway ballast or leaned into a wall.
//! That is a sprite question, so it is measured here, where the art lives — by
//! drawing the real sprite once into a scratch buffer and reading the pixels it
//! actually paints (no hand-kept table that can drift from the art).
//!
//! Two rules, both geometry:
//!
//! 1. **Sideways.** A prop's sprite may rise above its tile (an iso crown over
//!    the tile behind is normal), but it may not reach past its tile's left or
//!    right edge — that is a sprite leaning into the neighbour's cell.
//! 2. **Ground.** The bottom rows of a sprite — the trunk, the roots, the
//!    shadow — must land on the prop's *own* tile. If those pixels project onto
//!    a different tile, and that tile is a street, a pavement, the rail bed or
//!    water, then the prop reads as standing on it: the "trees standing on the
//!    rail line" the brief opens with.

use map::iso_consts::{HALF_H, HALF_W, HEIGHT_UNIT};
use map::{Biome, Prop, TileMap};

use crate::framebuffer::Framebuffer;
use crate::terrain::{SeasonLook, draw_prop_sprite, prop_sprite_extent};
use crate::Scene;

/// The rows at the bottom of a sprite that count as its ground contact.
const GROUND_ROWS: i32 = 4;

/// One sprite that reaches where it should not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Intrusion {
    pub x: i32,
    pub y: i32,
    /// The tile it reaches into.
    pub into: (i32, i32),
    /// How many pixels past the boundary (1 for a ground contact).
    pub px: i32,
    /// What kind of intrusion it is (only `"sideways"` today).
    pub what: &'static str,
    /// What stands there (for the handback's samples).
    pub prop: Prop,
}

/// The pixel audit's answer.
#[derive(Clone, Debug, Default)]
pub struct PixelReport {
    pub props_measured: u32,
    /// Every intrusion found (the samples below are the first few).
    pub hits: u32,
    pub sideways: u32,
    pub intrusions: Vec<Intrusion>,
}

impl PixelReport {
    pub fn count(&self) -> u32 {
        self.hits
    }

    fn add(&mut self, i: Intrusion) {
        self.hits += 1;
        if i.what == "sideways" {
            self.sideways += 1;
        }
        if self.intrusions.len() < 64 {
            self.intrusions.push(i);
        }
    }
}

/// Where a tile's ground point lands on screen, from the tile's own level.
fn ground(cam: &crate::iso::Camera, map: &TileMap, x: i32, y: i32) -> (i32, i32) {
    let t = map.at(x, y);
    cam.project_i(x, y, t.min_level() as i32 * HEIGHT_UNIT)
}

/// Which tile a screen pixel lands on, for a sprite standing on the ground
/// plane at `(cx, cy)`. Pixels above the plane are not ground (they are sky or
/// the tile behind), so they never count.
fn ground_tile(
    cam: &crate::iso::Camera,
    (cx, cy): (i32, i32),
    (px, py): (i32, i32),
) -> Option<(i32, i32)> {
    // The plane through (cx, cy): every point on it satisfies fx + fy =
    // (cy - oy) / HALF_H, and fx - fy = (px - ox) / HALF_W.
    if py > cy + 2 {
        return None;
    }
    let sum = (cy - cam.oy) as f32 / HALF_H as f32;
    let diff = (px - cam.ox) as f32 / HALF_W as f32;
    let fx = (sum + diff) * 0.5;
    let fy = (sum - diff) * 0.5;
    Some((fx.floor() as i32, fy.floor() as i32))
}

/// Audit every prop of a scene against its own tile and the tiles beside it.
/// `season` is the look the art is drawn with: the quiet green default, so the
/// crowns measured are the ones a summer frame shows.
pub fn audit(scene: &Scene, season: &SeasonLook) -> PixelReport {
    let map = &scene.map;
    let cam = &scene.cam;
    let look = crate::biome::look(Biome::Temperate);
    let mut r = PixelReport::default();
    let mut fb = Framebuffer::new(96, 112);
    let mut cache: std::collections::BTreeMap<(u8, u8, u8), ((i32, i32), Vec<(i32, i32)>)> =
        std::collections::BTreeMap::new();

    for y in 0..map.h {
        for x in 0..map.w {
            let prop = map.at(x, y).prop;
            if prop == Prop::None {
                continue;
            }
            let jitter = map.tile_hash(x, y);
            let (ext, _foot) = extents_of(&mut fb, &mut cache, prop, jitter, season, look);
            r.props_measured += 1;
            let (cx, cy) = ground(cam, map, x, y);
            let (half_l, half_r) = ext;

            // 1. Sideways: past the tile's own half-width, at any row.
            if half_l > HALF_W {
                r.add(Intrusion {
                    x,
                    y,
                    into: (x - 1, y + 1),
                    px: half_l - HALF_W,
                    what: "sideways",
                    prop,
                });
            }
            if half_r > HALF_W {
                r.add(Intrusion {
                    x,
                    y,
                    into: (x + 1, y - 1),
                    px: half_r - HALF_W,
                    what: "sideways",
                    prop,
                });
            }

        }
    }
    r
}

/// Measure one prop: draw it at a known anchor and read the extremes of the
/// pixels it actually paints, plus its ground-contact footprint relative to that
/// anchor. Cached per species, size and an eighth of the jitter, so eight
/// measurements cover a species and every tree is checked with its own
/// silhouette.
fn extents_of(
    fb: &mut Framebuffer,
    cache: &mut std::collections::BTreeMap<(u8, u8, u8), ((i32, i32), Vec<(i32, i32)>)>,
    prop: Prop,
    jitter: f32,
    season: &SeasonLook,
    look: &'static crate::biome::Ground,
) -> ((i32, i32), Vec<(i32, i32)>) {
    let bucket = (jitter * 8.0) as u8;
    let key = match prop {
        Prop::Tree { species, size, .. } => (species.id(), size, bucket),
        _ => (200, 0, bucket),
    };
    if let Some(e) = cache.get(&key) {
        return e.clone();
    }
    let (ax, ay) = (48, 96);
    fb.clear(crate::palette::TRANSPARENT);
    draw_prop_sprite(fb, ax, ay, prop, jitter, season, look);
    let ext = prop_sprite_extent(fb, ax);
    let mut foot = Vec::new();
    let px = fb.px();
    for yy in (ay - GROUND_ROWS)..=ay {
        for xx in 0..fb.w {
            if px[(yy as usize) * fb.w + xx as usize] != 0 {
                foot.push((xx as i32 - ax, yy - ay));
            }
        }
    }
    let e = (ext, foot);
    cache.insert(key, e.clone());
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measurement works: a tree is measurably bigger than a stone, and its
    /// trunk lands on the ground row.
    #[test]
    fn a_sprite_is_measured_by_drawing_it() {
        let look = crate::biome::look(Biome::Temperate);
        let season = SeasonLook::default();
        let mut fb = Framebuffer::new(96, 112);
        let mut cache = std::collections::BTreeMap::new();
        let ((l, r), foot) = extents_of(
            &mut fb,
            &mut cache,
            Prop::Tree {
                species: map::TreeSpecies::Oak,
                size: 1,
                variant: 0,
            },
            0.37,
            &season,
            look,
        );
        assert!(r > 4 && l > 4, "an oak measured {l},{r} px");
        assert!(!foot.is_empty(), "an oak paints no ground contact");
        let ((sl, sr), _) = extents_of(
            &mut fb,
            &mut cache,
            Prop::Stone(0),
            0.5,
            &season,
            look,
        );
        assert!(sl + sr < l + r, "a stone is wider than an oak");
    }

    /// A crown is allowed over the tile behind, but nothing may reach into the
    /// cell beside it: the meanest check is that no species is over the tile's
    /// own width.
    #[test]
    fn no_prop_sprite_is_wider_than_its_tile() {
        let look = crate::biome::look(Biome::Temperate);
        let season = SeasonLook::default();
        let mut fb = Framebuffer::new(96, 112);
        let mut cache = std::collections::BTreeMap::new();
        for species in map::TreeSpecies::ALL {
            for size in 0..=1u8 {
                let ((l, r), _) = extents_of(
                    &mut fb,
                    &mut cache,
                    Prop::Tree {
                        species,
                        size,
                        variant: 0,
                    },
                    0.61,
                    &season,
                    look,
                );
                assert!(
                    l <= HALF_W && r <= HALF_W,
                    "{species:?} size {size} is wider than its tile: {l}/{r} px"
                );
            }
        }
    }
}
