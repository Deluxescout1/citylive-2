//! Cast shadows from the world's geometry (D74 / B60).
//!
//! A shadow map from a **height field**, not a screen-space march, because the
//! height field is *geometry only*: it is built when the terrain or the city
//! changes and then reused for every light change, while only the mask is
//! re-marched. A screen-space march would need a per-pixel height plane written
//! by every drawing function (terrain, city, holiday, street, kit blits), and it
//! self-shadows at every silhouette. The iso projection is linear, so the light's
//! ground direction is a single 2D vector and a horizon march in the field is
//! both exact and cheap.
//!
//! Three steps, each once per light change (~a minute):
//! 1. [`HeightField`] — per sample: the ground's height and the height of
//!    whatever stands on it (buildings, kit sprites, trees, hills).
//! 2. [`bake_mask`] — march every sample toward the sun and record how much of
//!    the sun an occluder blocks; a ±1.2° band around the sun's own disc gives
//!    a penumbra that widens with the occluder's distance, in one pass.
//! 3. [`bake_screen`] — project the mask onto the frame, but only onto pixels
//!    that are *ground* (bare ground, lawns, roads, pavements); a roof or a wall
//!    is lit by the same sun that lights the street, and sampling a ground field
//!    for it would put its neighbour's shadow on the wrong face.

use crate::building::Building;
use crate::city::CityObj;
use crate::framebuffer::{Framebuffer, TAG_BUILT, TAG_PROP};
use crate::iso::Camera;
use crate::light::{Caster, LightState};
use crate::palette::{self, TRANSPARENT};
use map::iso_consts::{HEIGHT_UNIT, TILE_W};
use map::TileMap;

/// Shadow samples per tile edge. 8 gives one sample per 8 screen px, which is
/// the finest the 64-px pixel art can show without the mask costing more than
/// it is worth.
pub const SAMPLES_PER_TILE: i32 = 8;

/// A height in pixels, as the field stores it (B61f: whole pixels in an `i16`).
#[inline]
fn px(h: f32) -> i16 {
    h.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

/// The ground and what stands on it, in tile space.
pub struct HeightField {
    pub n: i32,
    /// Samples across and down.
    pub w: usize,
    pub h: usize,
    /// Ground height in pixels per sample. Heights are whole pixels in an
    /// `i16`: a sample is 8 screen px wide, so a fraction of a pixel cannot be
    /// seen in the mask, and three `f32` planes cost 20 MB at the owner's union
    /// where three `i16` planes cost 10 (B61f).
    pub ground: Vec<i16>,
    /// Height in pixels of the tallest thing standing on that sample.
    pub occl: Vec<i16>,
    /// B68: where the occluder at that sample *starts*. Equal to `ground` for
    /// anything solid to the floor (walls, hills); higher for a tree's canopy,
    /// so a trunk and a canopy cast their own shapes instead of one column.
    pub lo: Vec<i16>,
    /// B68: 0 = fully opaque, 1 = half-opaque (a trunk, thinner than a sample).
    pub half: Vec<u8>,
    /// Tallest occluder anywhere (caps the march).
    pub max_occl: f32,
    /// B71: pitched roofs with the builder's own geometry (small: one per kit
    /// house), and a tile -> roof lookup (index + 1, 0 = none).
    pub roofs: Vec<RoofRect>,
    pub roof_grid: Vec<u16>,
    tiles_w: usize,
}

/// How far outside its rectangle a hit still counts as that roof (the field's
/// roof and the sprite's differ by a few pixels at the edges).
const ROOF_MARGIN: f32 = 0.45;

/// One pitched roof in the field (B71): its wall rectangle in tiles, the eave
/// and ridge in absolute pixels, and its form (see `scene::RoofInfo`).
#[derive(Clone, Copy, Debug)]
pub struct RoofRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub eave: f32,
    pub ridge: f32,
    pub form: u8,
}

impl RoofRect {
    fn half(&self) -> (f32, f32, f32, f32) {
        ((self.x0 + self.x1) * 0.5, (self.y0 + self.y1) * 0.5, (self.x1 - self.x0) * 0.5, (self.y1 - self.y0) * 0.5)
    }

    /// The roof's surface height (absolute px) at a point inside the rectangle.
    pub fn top_at(&self, x: f32, y: f32) -> f32 {
        let (cx, cy, hx, hy) = self.half();
        let rise = (self.ridge - self.eave).max(0.0);
        let t = match self.form {
            1 => 1.0 - ((y - cy).abs() / hy.max(0.05)),
            3 => (hx - (x - cx).abs()).min(hy - (y - cy).abs()).max(0.0) / 0.22,
            _ => (hx - (x - cx).abs()).min(hy - (y - cy).abs()).max(0.0) / hx.min(hy).max(0.05),
        };
        self.eave + rise * t.clamp(0.0, 1.0)
    }

    /// Which slope a roof point belongs to: 0..=3.
    pub fn facet_at(&self, x: f32, y: f32) -> usize {
        let (cx, cy, hx, hy) = self.half();
        if self.form == 1 {
            return (y >= cy) as usize;
        }
        let (u, v) = ((x - cx) / hx.max(0.05), (y - cy) / hy.max(0.05));
        if u.abs() >= v.abs() {
            if u > 0.0 { 0 } else { 1 }
        } else if v > 0.0 {
            2
        } else {
            3
        }
    }
}

impl HeightField {
    pub fn new(map: &TileMap) -> Self {
        let n = SAMPLES_PER_TILE;
        let w = (map.w * n).max(1) as usize;
        let h = (map.h * n).max(1) as usize;
        let mut f = Self {
            n,
            w,
            h,
            ground: vec![0; w * h],
            occl: vec![0; w * h],
            lo: vec![0; w * h],
            half: vec![0; w * h],
            max_occl: 0.0,
            roofs: Vec::new(),
            roof_grid: vec![0; (map.w.max(1) * map.h.max(1)) as usize],
            tiles_w: map.w.max(1) as usize,
        };
        f.rebuild_ground(map);
        f
    }

    /// Terrain only.
    pub fn rebuild_ground(&mut self, map: &TileMap) {
        let n = self.n as f32;
        for j in 0..self.h {
            let y = (j as f32 + 0.5) / n;
            for i in 0..self.w {
                let x = (i as f32 + 0.5) / n;
                let h = px(map.surface_level_at(x, y) * HEIGHT_UNIT as f32);
                let k = j * self.w + i;
                self.ground[k] = h;
                self.occl[k] = h;
                self.lo[k] = h;
                self.half[k] = 0;
            }
        }
        self.max_occl = self
            .ground
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
            .max(0) as f32;
    }

    /// Terrain, buildings, kit sprites and trees (the whole geometry).
    pub fn rebuild(&mut self, map: &TileMap, buildings: &[Building], city: &[CityObj], kit: &[crate::scene::KitSprite]) {
        self.rebuild_ground(map);
        self.roofs.clear();
        self.roof_grid.iter_mut().for_each(|g| *g = 0);
        for b in buildings {
            let top = b.base_z as f32 + b.height as f32;
            self.stamp_rect(b.tx as f32, b.ty as f32, (b.tx + b.fw) as f32, (b.ty + b.fh) as f32, top);
        }
        for o in city {
            // B59b: a car place casts from what stands on it (canopy, kiosk,
            // showroom, deck), not from its whole plot.
            if o.kind.is_car_place() {
                for (x0, y0, x1, y1, top) in crate::city::place_casters(o) {
                    self.stamp_rect(x0, y0, x1, y1, o.level as f32 * HEIGHT_UNIT as f32 + top);
                }
                continue;
            }
            let Some(shape) = object_shape(o, kit) else {
                continue;
            };
            let base = o.level as f32 * HEIGHT_UNIT as f32;
            let (x0, y0, x1, y1) = shape.rect;
            if let Some(r) = shape.roof {
                let rr = RoofRect { x0, y0, x1, y1, eave: base + r.eave, ridge: base + r.ridge, form: r.form };
                self.stamp_roof(&rr);
                self.add_roof(rr);
            } else if shape.eave < shape.top - 3.0 {
                self.stamp_hip(x0, y0, x1, y1, base, shape.eave, shape.top);
                // A pitched kit house whose roof the builder did not describe: its
                // field shape is a guess, so it receives no cast shadow (form 0).
                if shape.guessed {
                    self.add_roof(RoofRect { x0, y0, x1, y1, eave: base + shape.eave, ridge: base + shape.top, form: 0 });
                }
            } else {
                self.stamp_rect(x0, y0, x1, y1, base + shape.top);
            }
        }
        // Trees, bushes, boulders: everything `draw_prop` stands up, each with
        // its own silhouette (B68): a canopy is a ball on a trunk, a conifer a
        // cone, a bush a dome.
        for y in 0..map.h {
            for x in 0..map.w {
                let t = map.at(x, y);
                self.stamp_prop(&t, x as f32 + 0.5, y as f32 + 0.5);
            }
        }
    }

    /// A pitched roof with the builder's own eave and ridge (B71).
    fn stamp_roof(&mut self, r: &RoofRect) {
        let n = self.n as f32;
        let i0 = ((r.x0 * n) as i32 - 1).max(0);
        let i1 = ((r.x1 * n) as i32 + 1).min(self.w as i32 - 1);
        let j0 = ((r.y0 * n) as i32 - 1).max(0);
        let j1 = ((r.y1 * n) as i32 + 1).min(self.h as i32 - 1);
        for j in j0..=j1 {
            let y = (j as f32 + 0.5) / n;
            if y < r.y0 || y > r.y1 {
                continue;
            }
            for i in i0..=i1 {
                let x = (i as f32 + 0.5) / n;
                if x < r.x0 || x > r.x1 {
                    continue;
                }
                let top = r.top_at(x, y);
                let k = self.idx(i, j);
                if top > self.occl[k] as f32 {
                    let g = self.ground[k] as f32;
                    self.put_cell(k, g, top, false);
                }
            }
        }
    }

    fn add_roof(&mut self, r: RoofRect) {
        if self.roofs.len() >= u16::MAX as usize - 1 {
            return;
        }
        self.roofs.push(r);
        let id = self.roofs.len() as u16;
        let th = self.roof_grid.len() / self.tiles_w;
        let m = ROOF_MARGIN;
        for ty in ((r.y0 - m).floor() as i32).max(0)..=(((r.y1 + m).floor() as i32).min(th as i32 - 1)) {
            for tx in ((r.x0 - m).floor() as i32).max(0)..=(((r.x1 + m).floor() as i32).min(self.tiles_w as i32 - 1)) {
                self.roof_grid[ty as usize * self.tiles_w + tx as usize] = id;
            }
        }
    }

    /// The pitched roof at a tile position, if the position is on it (or on its
    /// wall, within a hair).
    pub fn roof_at(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (tx, ty) = (x as usize, y as usize);
        if tx >= self.tiles_w || ty * self.tiles_w >= self.roof_grid.len() {
            return None;
        }
        let id = self.roof_grid[ty * self.tiles_w + tx] as usize;
        if id == 0 {
            return None;
        }
        let r = &self.roofs[id - 1];
        let m = ROOF_MARGIN;
        (x >= r.x0 - m && x <= r.x1 + m && y >= r.y0 - m && y <= r.y1 + m).then_some(id - 1)
    }

    /// One tile's prop as a shape in the field.
    fn stamp_prop(&mut self, t: &map::Tile, cx: f32, cy: f32) {
        use map::{Prop, TreeKind};
        let rise = crate::terrain::prop_rise(t) as f32;
        match t.prop {
            Prop::Tree { species, .. } => match species.kind() {
                TreeKind::Deciduous => {
                    // Trunk to the crown, then a ball: bottom at 0.30 of the
                    // height, widest at 0.62.
                    let r = 0.40;
                    let (zc, zr) = (rise * 0.64, rise * 0.34);
                    self.stamp_shape(cx - 0.08, cy - 0.08, cx + 0.08, cy + 0.08, true, &|_, _| Some((0.0, rise * 0.34)));
                    self.stamp_shape(cx - r, cy - r, cx + r, cy + r, false, &|x, y| {
                        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() / r;
                        if d >= 1.0 {
                            return None;
                        }
                        let h = zr * (1.0f32 - d * d).sqrt();
                        Some((zc - h, zc + h))
                    });
                }
                TreeKind::Conifer => {
                    let r = 0.30;
                    self.stamp_shape(cx - 0.08, cy - 0.08, cx + 0.08, cy + 0.08, true, &|_, _| Some((0.0, rise * 0.2)));
                    self.stamp_shape(cx - r, cy - r, cx + r, cy + r, false, &|x, y| {
                        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() / r;
                        if d >= 1.0 {
                            return None;
                        }
                        Some((rise * 0.14, rise * (0.14 + 0.86 * (1.0 - d))))
                    });
                }
                TreeKind::Succulent => {
                    self.stamp_shape(cx - 0.09, cy - 0.09, cx + 0.09, cy + 0.09, true, &|_, _| Some((0.0, rise)));
                }
            },
            Prop::Bush(_) | Prop::Boulder(_) | Prop::Stone(_) => {
                let r = if matches!(t.prop, Prop::Stone(_)) { 0.18 } else { 0.30 };
                self.stamp_shape(cx - r, cy - r, cx + r, cy + r, false, &|x, y| {
                    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() / r;
                    if d >= 1.0 {
                        return None;
                    }
                    Some((0.0, rise * (1.0 - d * d).sqrt()))
                });
            }
            // Meadow flowers, tall grass, reeds and logs lie too low and too
            // soft to throw a shadow a viewer can read.
            _ => {}
        }
    }

    /// Put an occluder spanning `lo..top` (above the ground) on one sample,
    /// merging with whatever already stands there.
    #[inline]
    fn put_cell(&mut self, k: usize, lo: f32, top: f32, half: bool) {
        let g = self.ground[k] as f32;
        let empty = self.occl[k] as f32 <= g + 0.5;
        if empty {
            self.lo[k] = px(lo.max(g));
            self.occl[k] = px(top);
            self.half[k] = half as u8;
        } else {
            self.lo[k] = self.lo[k].min(px(lo.max(g)));
            self.occl[k] = self.occl[k].max(px(top));
            self.half[k] = (self.half[k] != 0 && half) as u8;
        }
        self.max_occl = self.max_occl.max(top);
    }

    /// A shape given by a height profile `f(x, y) -> Option<(lo, top)>` (both
    /// above the ground at that sample), clipped to a rectangle.
    pub fn stamp_shape(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, half: bool, f: &dyn Fn(f32, f32) -> Option<(f32, f32)>) {
        let n = self.n as f32;
        let i0 = ((x0 * n) as i32 - 1).max(0);
        let i1 = ((x1 * n) as i32 + 1).min(self.w as i32 - 1);
        let j0 = ((y0 * n) as i32 - 1).max(0);
        let j1 = ((y1 * n) as i32 + 1).min(self.h as i32 - 1);
        for j in j0..=j1 {
            let y = (j as f32 + 0.5) / n;
            for i in i0..=i1 {
                let x = (i as f32 + 0.5) / n;
                if x < x0 || x > x1 || y < y0 || y > y1 {
                    continue;
                }
                if let Some((lo, top)) = f(x, y) {
                    let k = self.idx(i, j);
                    let g = self.ground[k] as f32;
                    self.put_cell(k, g + lo, g + top, half);
                }
            }
        }
    }

    /// A hip roof on a box: walls to `eave`, then a roof rising to `ridge`
    /// along the long axis (a ridge line) over the rectangle. `base` is the
    /// plot's level in pixels.
    pub fn stamp_hip(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, base: f32, eave: f32, ridge: f32) {
        let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let (hx, hy) = ((x1 - x0) * 0.5, (y1 - y0) * 0.5);
        let short = hx.min(hy).max(0.05);
        let rise = (ridge - eave).max(0.0);
        let n = self.n as f32;
        let i0 = ((x0 * n) as i32 - 1).max(0);
        let i1 = ((x1 * n) as i32 + 1).min(self.w as i32 - 1);
        let j0 = ((y0 * n) as i32 - 1).max(0);
        let j1 = ((y1 * n) as i32 + 1).min(self.h as i32 - 1);
        for j in j0..=j1 {
            let y = (j as f32 + 0.5) / n;
            if y < y0 || y > y1 {
                continue;
            }
            for i in i0..=i1 {
                let x = (i as f32 + 0.5) / n;
                if x < x0 || x > x1 {
                    continue;
                }
                let edge = (hx - (x - cx).abs()).min(hy - (y - cy).abs()).max(0.0);
                let top = base + eave + rise * (edge / short).min(1.0);
                let k = self.idx(i, j);
                if top > self.occl[k] as f32 {
                    let g = self.ground[k] as f32;
                    self.put_cell(k, g, top, false);
                }
            }
        }
    }

    #[inline]
    fn idx(&self, i: i32, j: i32) -> usize {
        j as usize * self.w + i as usize
    }

    /// The ground's height at a continuous tile position, bilinear.
    #[inline]
    pub fn ground_at(&self, x: f32, y: f32) -> f32 {
        self.sample_bilinear(&self.ground, x, y)
    }

    #[inline]
    fn sample_bilinear(&self, plane: &[i16], x: f32, y: f32) -> f32 {
        let n = self.n as f32;
        let fx = (x * n - 0.5).clamp(0.0, self.w as f32 - 1.001);
        let fy = (y * n - 0.5).clamp(0.0, self.h as f32 - 1.001);
        let i = fx as i32;
        let j = fy as i32;
        let tx = fx - i as f32;
        let ty = fy - j as f32;
        let (a, b, c, d) = (
            plane[self.idx(i, j)] as f32,
            plane[self.idx(i + 1, j)] as f32,
            plane[self.idx(i, j + 1)] as f32,
            plane[self.idx(i + 1, j + 1)] as f32,
        );
        let ab = a + (b - a) * tx;
        let cd = c + (d - c) * tx;
        ab + (cd - ab) * ty
    }

    /// Raise the occluders of every sample whose centre is inside the rectangle.
    /// Raise the occluder plane over a rectangle to `f(x, y)` wherever that is
    /// above it (a landform's body, B46d).
    pub fn stamp_fn(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, f: &dyn Fn(f32, f32) -> f32) {
        let n = self.n as f32;
        let i0 = ((x0 * n) as i32).max(0);
        let i1 = ((x1 * n) as i32).min(self.w as i32 - 1);
        let j0 = ((y0 * n) as i32).max(0);
        let j1 = ((y1 * n) as i32).min(self.h as i32 - 1);
        for j in j0..=j1 {
            let y = (j as f32 + 0.5) / n;
            for i in i0..=i1 {
                let x = (i as f32 + 0.5) / n;
                let k = self.idx(i, j);
                let g = self.ground[k] as f32;
                let top = g + f(x, y);
                if top > self.occl[k] as f32 {
                    self.put_cell(k, g, top, false);
                }
            }
        }
    }

    pub fn stamp_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, top: f32) {
        let n = self.n as f32;
        let i0 = ((x0 * n) as i32 - 1).max(0);
        let i1 = ((x1 * n) as i32 + 1).min(self.w as i32 - 1);
        let j0 = ((y0 * n) as i32 - 1).max(0);
        let j1 = ((y1 * n) as i32 + 1).min(self.h as i32 - 1);
        for j in j0..=j1 {
            let y = (j as f32 + 0.5) / n;
            if y < y0 || y > y1 {
                continue;
            }
            for i in i0..=i1 {
                let x = (i as f32 + 0.5) / n;
                if x < x0 || x > x1 {
                    continue;
                }
                let k = self.idx(i, j);
                if top > self.occl[k] as f32 {
                    self.put_cell(k, self.ground[k] as f32, top, false);
                }
            }
        }
        self.max_occl = self.max_occl.max(top);
    }

    pub fn stamp_disc(&mut self, cx: f32, cy: f32, r: f32, top: f32) {
        let n = self.n as f32;
        let i0 = (((cx - r) * n) as i32).max(0);
        let i1 = (((cx + r) * n) as i32 + 1).min(self.w as i32 - 1);
        let j0 = (((cy - r) * n) as i32).max(0);
        let j1 = (((cy + r) * n) as i32 + 1).min(self.h as i32 - 1);
        let r2 = r * r;
        for j in j0..=j1 {
            let y = (j as f32 + 0.5) / n;
            for i in i0..=i1 {
                let x = (i as f32 + 0.5) / n;
                let (dx, dy) = (x - cx, y - cy);
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let k = self.idx(i, j);
                if top > self.occl[k] as f32 {
                    self.put_cell(k, self.ground[k] as f32, top, false);
                }
            }
        }
        self.max_occl = self.max_occl.max(top);
    }

    /// The height of whatever stands on a continuous tile position (0 height
    /// above the ground where nothing does).
    #[inline]
    pub fn height_above_ground(&self, x: f32, y: f32) -> f32 {
        let o = self.sample_bilinear(&self.occl, x, y);
        (o - self.ground_at(x, y)).max(0.0)
    }
}

/// What a city object casts from: its wall rectangle (tiles), the height of
/// its walls up to the eave and of its highest roof point (pixels above the
/// plot's base).
struct Shape {
    rect: (f32, f32, f32, f32),
    eave: f32,
    top: f32,
    /// The builder's own roof, when the sprite carries one (B71).
    roof: Option<crate::scene::RoofInfo>,
    /// A kit house's pitched roof drawn from the 0.58 guess (B68).
    guessed: bool,
}

/// The shape a city object casts from, or `None` when it is open ground (a park,
/// a plaza) or not built up yet.
fn object_shape(o: &CityObj, kit: &[crate::scene::KitSprite]) -> Option<Shape> {
    use crate::city::{LandmarkKind, ObjKind, Stage};
    if matches!(o.kind, ObjKind::Park | ObjKind::Cemetery) || matches!(o.kind, ObjKind::Landmark(LandmarkKind::Plaza | LandmarkKind::Zoo)) {
        return None;
    }
    let plot = (o.x as f32, o.y as f32, (o.x + o.w) as f32, (o.y + o.h) as f32);
    // The walls stand inside the lot (setback + yard) and the sprite paints a
    // paved apron round them; the shadow has to start at the wall.
    let inset = if o.kind.is_massing() { 0.2 } else { 0.4 };
    let engine_rect = (plot.0 + inset, plot.1 + inset, plot.2 - inset, plot.3 - inset);
    let h = match o.stage {
        Stage::Staked | Stage::Graded => return None,
        Stage::Foundation => 4.0,
        Stage::Framing => o.wall_height() as f32 * 0.55,
        Stage::Scaffold => o.wall_height() as f32 * 0.8,
        Stage::Roofed | Stage::Finished => {
            if let Some(k) = (o.kit != 0).then(|| kit.get(o.kit as usize - 1)).flatten() {
                // B68: a kit sprite fills only the top corner of its plot. Its
                // walls are where its own silhouette says (`foot`), and its
                // height is the builder's depth plane, not the anchor's offset
                // (which also holds the footprint's screen depth). A pitched
                // roof is a hip from the eave to the ridge; a flat one a box.
                if let Some(r) = k.roof.filter(|r| r.eave + 3.0 < r.ridge) {
                    // B71: the real roof: its own rectangle, eave and ridge.
                    let m = 0.05;
                    let rect = (plot.0 + r.x0 + m, plot.1 + r.y0 + m, plot.0 + r.x1 - m, plot.1 + r.y1 - m);
                    return Some(Shape { rect, eave: r.eave, top: r.ridge, roof: Some(r), guessed: false });
                }
                let top = (k.top as f32).max(k.oy as f32);
                let pitched = matches!(
                    o.kind,
                    ObjKind::House | ObjKind::Townhouse | ObjKind::Barn | ObjKind::LogCabin | ObjKind::Church | ObjKind::Hall
                );
                let f = k.foot;
                let m = 0.05;
                let rect = (plot.0 + f[0] + m, plot.1 + f[1] + m, plot.0 + f[2] - m, plot.1 + f[3] - m);
                return Some(Shape {
                    rect,
                    eave: if pitched { top * 0.58 } else { top * 0.94 },
                    top: if pitched { top } else { top * 0.94 },
                    roof: None,
                    guessed: pitched,
                });
            }
            o.wall_height() as f32
        }
    };
    Some(Shape { rect: engine_rect, eave: h, top: h, roof: None, guessed: false })
}

/// How far a shadow march may run, in tiles (beyond this the light is flat on
/// the ground and the pixel art cannot show it).
const MAX_SHADOW_TILES: f32 = 42.0;

/// How much of the sun's disc the penumbra spans, in degrees. The real sun is
/// 0.53° (a 2-px penumbra on a 200-px shadow); a little more reads as a soft
/// edge at pixel scale without going mushy.
pub const PENUMBRA_DEG: f32 = 2.0;

/// March every sample toward the light and return the shadow coverage per
/// sample (0 = full sun, 255 = full shadow).
pub fn bake_mask(field: &HeightField, light: &LightState, out: &mut Vec<u8>) -> Option<Caster> {
    let caster = light.caster()?;
    out.clear();
    out.resize(field.w * field.h, 0);
    bake_mask_rows(field, &caster, out, 0, field.h);
    Some(caster)
}

/// Rows `j0..j1` of [`bake_mask`] into an already sized mask (B67: the wallpaper
/// bakes a few rows per frame so no frame freezes; B68's threading still splits
/// each slice).
pub fn bake_mask_rows(field: &HeightField, caster: &Caster, out: &mut [u8], j0: usize, j1: usize) {
    let j1 = j1.min(field.h);
    if j1 <= j0 {
        return;
    }
    let alt = caster.alt.max(0.6);
    let tan = alt.to_radians().tan();
    let hi = (alt + PENUMBRA_DEG).min(89.0).to_radians().tan();
    let lo = (alt - PENUMBRA_DEG).max(0.3).to_radians().tan();
    let band = (hi - lo).max(1e-4);
    let n = field.n as f32;
    // March *toward* the light, one sample per step (the field's own pitch): a
    // sample is in shadow when something stands between it and the sun.
    let step = (TILE_W as f32) / n; // screen px between samples
    let (dx, dy) = (caster.toward.0, caster.toward.1);
    let max_steps = (MAX_SHADOW_TILES * n) as i32;
    const BINS: u32 = 32;
    const FULL: u32 = u32::MAX;
    // The rows are independent, so a low sun's long marches (the slowest bake,
    // ~70 ms single-threaded) are split over a few threads: the same CPU, spent
    // in a quarter of the wall time, so the frame that triggers a re-bake does
    // not hitch (B68).
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4);
    let nrows = j1 - j0;
    let rows_per = nrows.div_ceil(threads);
    std::thread::scope(|sc| {
        for (ci, chunk) in out[j0 * field.w..j1 * field.w].chunks_mut(rows_per * field.w).enumerate() {
            let (lo, hi, band, tan, step, dx, dy) = (lo, hi, band, tan, step, dx, dy);
            sc.spawn(move || {
    let jb = j0 + ci * rows_per;
    for (jr, row) in chunk.chunks_mut(field.w).enumerate() {
        let j = jb + jr;
        for i in 0..field.w {
            let k = j * field.w + i;
            let h0 = field.ground[k] as f32;
            // A sample under something solid (a wall, a hill) is not the ground
            // we shade: skip it and save the march. Under a canopy the ground
            // is real and is marched like any other.
            if field.occl[k] as f32 > h0 + 2.0 && field.lo[k] as f32 <= h0 + 2.0 {
                row[i] = 0;
                continue;
            }
            // The band between the sun's lower and upper limb is split into
            // BINS slices; every occluder blocks the slices its tangent range
            // covers (a wall from the ground up, a canopy only between its
            // underside and its crown). The shadow is the share blocked.
            let mut blocked = 0u32;
            let mut px = i as f32 + 0.5;
            let mut py = j as f32 + 0.5;
            let mut dist = 0.0f32;
            for _ in 0..max_steps {
                px += dx;
                py += dy;
                dist += step;
                let (ii, jj) = (px as i32, py as i32);
                if ii < 0 || jj < 0 || ii >= field.w as i32 || jj >= field.h as i32 {
                    break;
                }
                let c = jj as usize * field.w + ii as usize;
                let o = field.occl[c] as f32;
                if o > h0 {
                    let t_hi = (o - h0) / dist;
                    if t_hi > lo {
                        let floor = field.lo[c] as f32;
                        let t_lo = if floor > h0 + 0.5 { (floor - h0) / dist } else { 0.0 };
                        let b0 = (((t_lo - lo) / band * BINS as f32).round().max(0.0) as u32).min(BINS);
                        let b1 = (((t_hi - lo) / band * BINS as f32).round().max(0.0) as u32).min(BINS);
                        if b1 > b0 {
                            let mut m = (((1u64 << b1) - (1u64 << b0)) & 0xFFFF_FFFF) as u32;
                            if field.half[c] != 0 {
                                m &= 0x5555_5555;
                            }
                            blocked |= m;
                            if blocked == FULL {
                                break; // fully shadowed: no need to march further
                            }
                        }
                    }
                }
                // Nothing left in the field can be tall enough to matter.
                if dist * tan > field.max_occl - h0 + 1.0 {
                    break;
                }
            }
            row[i] = ((blocked.count_ones() * 255 + BINS / 2) / BINS) as u8;
        }
    }
            });
        }
    });
}


/// B71: the shadow *ceiling* of every column near something solid. A receiver
/// that is not on the ground (a roof, a wall) cannot read the ground mask, which
/// says only whether the floor is lit. For a point at height `z` in column `p`
/// the question is whether any occluder along the sun's line is taller than the
/// sun's ray at that distance: `z < occl(c) - dist * tan(alt)`. The maximum of the
/// right-hand side over the march is the column's ceiling; anything below it is
/// in shadow. Two ceilings (the sun's upper and lower limb) give the penumbra: a
/// point under the lower one is fully shadowed, over the upper one fully lit,
/// between them it fades. 2 x i16 per sample (0.9 MB at 60 x 60 tiles).
pub struct Ceil {
    /// Below this height the whole disc is blocked (upper limb).
    pub full: Vec<i16>,
    /// Above this height nothing is blocked (lower limb).
    pub none: Vec<i16>,
    /// Tallest thing within one 4 x 4 block of samples and its 8 neighbours: lets
    /// the view-ray walk leap through the empty air above the city.
    pub bmax: Vec<i16>,
    /// B71: one shade per slope and wall of every pitched roof (0..=3 slopes,
    /// 4 the +x wall, 5 the +y wall). A slope or wall is in a neighbour's shadow
    /// or it is not: no dashes.
    pub facets: Vec<[u8; 6]>,
}

/// Samples per side of a `bmax` block.
const BLOCK: usize = 4;

impl Ceil {
    pub fn new(field: &HeightField) -> Self {
        let (bw, bh) = (field.w.div_ceil(BLOCK), field.h.div_ceil(BLOCK));
        let mut own = vec![i16::MIN; bw * bh];
        for j in 0..field.h {
            for i in 0..field.w {
                let b = (j / BLOCK) * bw + i / BLOCK;
                own[b] = own[b].max((field.occl[j * field.w + i] as f32) as i16);
            }
        }
        let mut bmax = own.clone();
        for bj in 0..bh {
            for bi in 0..bw {
                let mut m = i16::MIN;
                for dj in bj.saturating_sub(1)..(bj + 2).min(bh) {
                    for di in bi.saturating_sub(1)..(bi + 2).min(bw) {
                        m = m.max(own[dj * bw + di]);
                    }
                }
                bmax[bj * bw + bi] = m;
            }
        }
        Self {
            full: vec![i16::MIN; field.w * field.h],
            none: vec![i16::MIN; field.w * field.h],
            bmax,
            facets: vec![[0; 6]; field.roofs.len()],
        }
    }
}

/// True when the sample is a solid thing worth casting on a roof or wall: a
/// building, a hill, a boulder. A canopy (with air under it) is left to the
/// ground mask, which handles its gap.
#[inline]
fn solid_cell(field: &HeightField, c: usize) -> bool {
    (field.occl[c] as f32) > (field.ground[c] as f32) + 2.0 && (field.lo[c] as f32) <= (field.ground[c] as f32) + 2.0 && field.half[c] == 0
}

/// Rows `j0..j1` of the ceilings. Only columns with something solid within one
/// sample are marched (a roof or the air in front of a wall): everywhere else
/// the ceiling stays at `i16::MIN` and costs nothing.
pub fn bake_ceil_rows(field: &HeightField, caster: &Caster, ceil: &mut Ceil, j0: usize, j1: usize) {
    let j1 = j1.min(field.h);
    let alt = caster.alt.max(0.6);
    let t_hi = (alt + PENUMBRA_DEG).min(89.0).to_radians().tan();
    let t_lo = (alt - PENUMBRA_DEG).max(0.3).to_radians().tan();
    let n = field.n as f32;
    let step = (TILE_W as f32) / n;
    let (dx, dy) = (caster.toward.0, caster.toward.1);
    let max_steps = (MAX_SHADOW_TILES * n) as i32;
    let (w, h) = (field.w as i32, field.h as i32);
    for j in j0..j1 {
        for i in 0..field.w {
            let k = j * field.w + i;
            // Near anything solid?
            let mut near = false;
            'n: for dj in -1..=1i32 {
                for di in -1..=1i32 {
                    let (a, b) = (i as i32 + di, j as i32 + dj);
                    if a >= 0 && b >= 0 && a < w && b < h && solid_cell(field, b as usize * field.w + a as usize) {
                        near = true;
                        break 'n;
                    }
                }
            }
            if !near {
                continue;
            }
            let g = (field.ground[k] as f32);
            let (mut full, mut none) = (g, g);
            let mut px = i as f32 + 0.5;
            let mut py = j as f32 + 0.5;
            let mut dist = 0.0f32;
            for _ in 0..max_steps {
                px += dx;
                py += dy;
                dist += step;
                let (ii, jj) = (px as i32, py as i32);
                if ii < 0 || jj < 0 || ii >= w || jj >= h {
                    break;
                }
                let c = jj as usize * field.w + ii as usize;
                if (field.occl[c] as f32) > g && solid_cell(field, c) {
                    let o = (field.occl[c] as f32);
                    full = full.max(o - dist * t_hi);
                    none = none.max(o - dist * t_lo);
                }
                if field.max_occl - dist * t_lo <= none {
                    break;
                }
            }
            ceil.full[k] = full.round().clamp(-30000.0, 30000.0) as i16;
            ceil.none[k] = none.round().clamp(-30000.0, 30000.0) as i16;
        }
    }
}

/// The whole ceiling in one call (tests and tools).
pub fn bake_ceil(field: &HeightField, caster: &Caster) -> Ceil {
    let mut c = Ceil::new(field);
    bake_ceil_rows(field, caster, &mut c, 0, field.h);
    bake_facets(field, caster, &mut c, 0, field.roofs.len());
    c
}

/// Shade of each slope and sun-facing wall of roofs `r0..r1`: the mean of the
/// shadow over a grid of points on that surface, snapped so that a slope that is
/// mostly lit is lit and one that is mostly shaded is shaded (B71).
pub fn bake_facets(field: &HeightField, caster: &Caster, ceil: &mut Ceil, r0: usize, r1: usize) {
    let n = field.n as f32;
    let (dx, dy) = caster.toward;
    const G: usize = 7;
    let snap = |sum: f32, cnt: f32| -> u8 {
        if cnt < 1.0 {
            return 0;
        }
        let m = sum / cnt / 255.0;
        (((m - 0.15) / 0.7).clamp(0.0, 1.0) * 255.0) as u8
    };
    for ri in r0..r1.min(field.roofs.len()) {
        let r = field.roofs[ri];
        if r.form == 0 {
            ceil.facets[ri] = [0; 6];
            continue;
        }
        let (mut sum, mut cnt) = ([0f32; 4], [0f32; 4]);
        for a in 0..G {
            for b in 0..G {
                let x = r.x0 + (r.x1 - r.x0) * (a as f32 + 0.5) / G as f32;
                let y = r.y0 + (r.y1 - r.y0) * (b as f32 + 0.5) / G as f32;
                let f = r.facet_at(x, y);
                let z = r.top_at(x, y) + 0.5;
                sum[f] += ceil_shadow(field, ceil, x * n, y * n, z) as f32;
                cnt[f] += 1.0;
            }
        }
        let mut out = [0u8; 6];
        for f in 0..4 {
            out[f] = snap(sum[f], cnt[f]);
        }
        // A hip roof shows only its +x and +y slopes to the camera; the back two
        // (1, 3) are seen only as slivers along the ridge where the field and
        // the sprite disagree by a pixel, so they take their neighbours' shade.
        if r.form != 1 {
            out[1] = out[0];
            out[3] = out[2];
        }
        // Walls: only the two faces that look at the camera, and only if they
        // look at the sun too (the others are in their own shade).
        for (k, (nx, ny)) in [(1.0f32, 0.0f32), (0.0, 1.0)].into_iter().enumerate() {
            if dx * nx + dy * ny <= 0.12 {
                continue;
            }
            let (mut s, mut c) = (0.0f32, 0.0f32);
            let gnd = (field.ground[((r.y0 * n) as usize).min(field.h - 1) * field.w + ((r.x0 * n) as usize).min(field.w - 1)] as f32);
            for a in 0..5 {
                for h in 1..=3 {
                    let t = (a as f32 + 0.5) / 5.0;
                    let z = gnd + (r.eave - gnd) * h as f32 * 0.25;
                    let (px, py) = if k == 0 { (r.x1 * n + 1.0, (r.y0 + (r.y1 - r.y0) * t) * n) } else { ((r.x0 + (r.x1 - r.x0) * t) * n, r.y1 * n + 1.0) };
                    s += ceil_shadow(field, ceil, px, py, z) as f32;
                    c += 1.0;
                }
            }
            out[4 + k] = snap(s, c);
        }
        ceil.facets[ri] = out;
    }
}

/// Shadow (0..=255) of the point at sample position `(px, py)` and height `z`.
fn ceil_shadow(field: &HeightField, ceil: &Ceil, px: f32, py: f32, z: f32) -> u8 {
    let fx = (px - 0.5).clamp(0.0, field.w as f32 - 1.001);
    let fy = (py - 0.5).clamp(0.0, field.h as f32 - 1.001);
    let (i0, j0) = (fx as usize, fy as usize);
    let (ax, ay) = (fx - i0 as f32, fy - j0 as f32);
    let bil = |p: &[i16]| {
        let at = |i: usize, j: usize| p[j * field.w + i] as f32;
        let ab = at(i0, j0) + (at(i0 + 1, j0) - at(i0, j0)) * ax;
        let cd = at(i0, j0 + 1) + (at(i0 + 1, j0 + 1) - at(i0, j0 + 1)) * ax;
        ab + (cd - ab) * ay
    };
    let (full, none) = (bil(&ceil.full), bil(&ceil.none));
    if z >= none {
        0
    } else if z <= full || none - full < 0.5 {
        255
    } else {
        (255.0 * (none - z) / (none - full)).clamp(0.0, 255.0) as u8
    }
}

/// Where the camera sees a building pixel in the height field: the first solid
/// surface met by walking the view ray down from above the tallest thing.
/// Returns the sample-space position, the height, and the wall normal (None for
/// a roof). `None` when the ray reaches the ground without meeting a solid, i.e.
/// the pixel is something the field does not model.
fn view_hit(field: &HeightField, ceil: &Ceil, cam: &Camera, sx: f32, sy: f32) -> Option<(f32, f32, f32, Option<(i32, i32)>)> {
    const DZ: f32 = 2.0;
    let n = field.n as f32;
    let u = (sx - cam.ox as f32) / map::iso_consts::HALF_W as f32;
    let hh = map::iso_consts::HALF_H as f32;
    let mut z = field.max_occl + 1.0;
    let mut prev: Option<(f32, i32, i32, f32, f32)> = None;
    while z > -20.0 {
        let v = (sy - cam.oy as f32 + z) / hh;
        let (tx, ty) = ((u + v) * 0.5, (v - u) * 0.5);
        let (i, j) = ((tx * n).floor() as i32, (ty * n).floor() as i32);
        if i < 0 || j < 0 {
            return None;
        }
        if i < field.w as i32 && j < field.h as i32 {
            let c = j as usize * field.w + i as usize;
            // Far above everything near: leap (at most 2 samples of travel).
            let bw = field.w.div_ceil(BLOCK);
            let top = ceil.bmax[(j as usize / BLOCK) * bw + i as usize / BLOCK] as f32;
            if z - top >= 3.0 {
                z -= (z - top).min(8.0);
                // Still above everything: remember where we landed, so the
                // first solid cell met next is classified from this state (a
                // roof if it is the same cell, a wall if the cell changed).
                let v2 = (sy - cam.oy as f32 + z) / hh;
                let (i2, j2) = (((u + v2) * 0.5 * n).floor() as i32, ((v2 - u) * 0.5 * n).floor() as i32);
                prev = Some((z, i2, j2, (u + v2) * 0.5 * n, (v2 - u) * 0.5 * n));
                z -= DZ;
                continue;
            }
            if solid_cell(field, c) && (field.occl[c] as f32) >= z {
                let top = (field.occl[c] as f32);
                return Some(match prev {
                    Some((zp, ip, jp, pxf, pyf)) if (ip != i || jp != j) && zp <= top => {
                        // Entered through a side: which one is open.
                        let open = |a: i32, b: i32| -> f32 {
                            if a < 0 || b < 0 || a >= field.w as i32 || b >= field.h as i32 {
                                return -1e9;
                            }
                            field.occl[b as usize * field.w + a as usize] as f32
                        };
                        // The ray was beyond this cell's +x face by `dxp` and beyond its
                        // +y face by `dyp`; it enters the cell through the face it
                        // crosses last (the larger distance). A ray that runs along the
                        // grid diagonal ties: always the +x face, so no flicker.
                        let (dxp, dyp) = (pxf - (i + 1) as f32, pyf - (j + 1) as f32);
                        let _ = (ip, jp);
                        // Which faces have open air in front of them (the ray steps are
                        // half a sample, so it can hop over a cell along the wall).
                        let (open_x, open_y) = (open(i + 1, j) <= top - 7.0, open(i, j + 1) <= top - 7.0);
                        let nrm = if open_x != open_y {
                            if open_x { (1, 0) } else { (0, 1) }
                        } else if dxp >= 0.0 && (dyp < 0.0 || dxp >= dyp - 1e-4) {
                            (1, 0)
                        } else {
                            (0, 1)
                        };
                        // A step of a few pixels is the staircase of a pitched roof
                        // sampled at 8 per tile, not a wall: it stays a roof.
                        if top - open(i + nrm.0, j + nrm.1) < 7.0 {
                            (tx * n, ty * n, top, None)
                        } else {
                            // The exact height where the ray crosses that face (a step of
                            // 1 px of z moves the point 1/4 sample), not the 2 px step it was
                            // found on: a shadow edge then runs smoothly down the wall.
                            let dist = if nrm.0 == 1 { dxp.max(if nrm.1 == 1 { dyp } else { dxp }) } else { dyp };
                            let zx = (zp - dist / 0.25).clamp(z, zp);
                            (tx * n, ty * n, zx, Some(nrm))
                        }
                    }
                    _ => (tx * n, ty * n, top, None),
                });
            }
            if z <= (field.ground[c] as f32) {
                return None;
            }
            prev = Some((z, i, j, tx * n, ty * n));
        } else {
            prev = None;
        }
        z -= DZ;
    }
    None
}

/// Is the roof around a sample position flat (the 3 x 3 samples within a pixel)?
fn flat_top(field: &HeightField, px: f32, py: f32) -> bool {
    let (ci, cj) = (px.floor() as i32, py.floor() as i32);
    let at = |i: i32, j: i32| -> Option<i16> {
        (i >= 0 && j >= 0 && i < field.w as i32 && j < field.h as i32).then(|| field.occl[j as usize * field.w + i as usize])
    };
    let Some(c) = at(ci, cj) else { return false };
    for dj in -1..=1 {
        for di in -1..=1 {
            if let Some(v) = at(ci + di, cj + dj) {
                // A neighbour that is lower is the roof's edge, not a slope.
                if v > c || (1..=8).contains(&(c - v)) {
                    return false;
                }
            }
        }
    }
    true
}

/// Bilinear roof height at a sample position, when the four samples around it
/// are one gently sloping surface.
fn smooth_top(field: &HeightField, px: f32, py: f32) -> Option<f32> {
    let fx = (px - 0.5).clamp(0.0, field.w as f32 - 1.001);
    let fy = (py - 0.5).clamp(0.0, field.h as f32 - 1.001);
    let (i0, j0) = (fx as usize, fy as usize);
    let (ax, ay) = (fx - i0 as f32, fy - j0 as f32);
    let at = |i: usize, j: usize| (field.occl[j * field.w + i] as f32);
    let (a, b, c, d) = (at(i0, j0), at(i0 + 1, j0), at(i0, j0 + 1), at(i0 + 1, j0 + 1));
    let (lo, hi) = (a.min(b).min(c).min(d), a.max(b).max(c).max(d));
    if hi - lo > 8.0 {
        return None;
    }
    let ab = a + (b - a) * ax;
    let cd = c + (d - c) * ax;
    Some(ab + (cd - ab) * ay)
}

/// B71: the shadow on a building pixel (a roof or a wall), or 0.
fn receiver_shadow(field: &HeightField, ceil: &Ceil, cam: &Camera, caster: &Caster, sx: f32, sy: f32) -> u8 {
    let Some((px, py, z, normal)) = view_hit(field, ceil, cam, sx, sy) else {
        return 0;
    };
    let (dx, dy) = caster.toward;
    // A kit house with its real roof: each slope and wall is one shade.
    if !field.roofs.is_empty() {
        let n = field.n as f32;
        if let Some(r) = field.roof_at(px / n, py / n).filter(|&r| z <= field.roofs[r].ridge + 8.0) {
            let f = ceil.facets[r];
            // The eave strip and above (overhang, gable ends) reads as roof, so a
            // slope and its fringe are one shade.
            let normal = if z >= field.roofs[r].eave - 2.0 { None } else { normal };
            return match normal {
                None => {
                    // Settle the point on the roof plane itself (the ray walk steps 2 px), so a
                    // hip line comes out as a straight edge, not a staircase.
                    let rr = &field.roofs[r];
                    let u = (sx - cam.ox as f32) / map::iso_consts::HALF_W as f32;
                    let (mut tx, mut ty, mut zz) = (px / n, py / n, z);
                    for _ in 0..3 {
                        zz = rr.top_at(tx.clamp(rr.x0, rr.x1), ty.clamp(rr.y0, rr.y1));
                        let v = (sy - cam.oy as f32 + zz) / map::iso_consts::HALF_H as f32;
                        tx = (u + v) * 0.5;
                        ty = (v - u) * 0.5;
                    }
                    let _ = zz;
                    f[rr.facet_at(tx, ty)]
                }
                Some((1, 0)) => f[4],
                Some((0, 1)) => f[5],
                Some(_) => f[4].max(f[5]),
            };
        }
    }
    match normal {
        Some((1, 1)) if (field.occl[(py.floor() as usize).min(field.h - 1) * field.w + (px.floor() as usize).min(field.w - 1)] as f32 - field.ground[(py.floor() as usize).min(field.h - 1) * field.w + (px.floor() as usize).min(field.w - 1)] as f32) < 64.0 => 0,
        Some((1, 1)) => {
            let mut best = 0;
            for (nx, ny) in [(1, 0), (0, 1)] {
                if dx * nx as f32 + dy * ny as f32 > 0.12 {
                    let (qx, qy) = if nx != 0 { (px.floor() + 1.5, py) } else { (px, py.floor() + 1.5) };
                    best = best.max(ceil_shadow(field, ceil, qx, qy, z));
                }
            }
            best
        }
        Some(_) if (field.occl[(py.floor() as usize).min(field.h - 1) * field.w + (px.floor() as usize).min(field.w - 1)] as f32
            - field.ground[(py.floor() as usize).min(field.h - 1) * field.w + (px.floor() as usize).min(field.w - 1)] as f32)
            < 64.0 =>
        {
            // A low wall that is not a kit house's: its sprite's own shading only.
            0
        }
        Some((nx, ny)) => {
            // A wall facing away from the sun is already in its own shade.
            if dx * nx as f32 + dy * ny as f32 <= 0.12 {
                return 0;
            }
            let (qx, qy) = if nx != 0 { (px.floor() + 1.5, py) } else { (px, py.floor() + 1.5) };
            ceil_shadow(field, ceil, qx, qy, z)
        }
        None => {
            // Only a flat roof takes a shadow here. A pitched one is either a kit
            // house (handled above, a slope at a time) or something whose
            // geometry the field only approximates: it gets none rather than
            // dashes.
            let (ci, cj) = ((px.floor() as usize).min(field.w - 1), (py.floor() as usize).min(field.h - 1));
            let rise = field.occl[cj * field.w + ci] as f32 - field.ground[cj * field.w + ci] as f32;
            if rise < 56.0 || !flat_top(field, px, py) {
                return 0;
            }
            ceil_shadow(field, ceil, px, py, z + 0.5)
        }
    }
}

/// Project the sample mask onto the frame: only ground pixels take it.
///
/// The receiver test uses the static layer's own tags: bare ground and lawns are
/// alpha `0xFF`, lanes and pavements are [`TAG_BUILT`] on an *odd* painter key
/// (a tile item), while walls, roofs and kit sprites are `TAG_BUILT` on an even
/// key and canopies are [`TAG_PROP`]. Emissive pixels (alpha `0xFE`) never take
/// shadow either — they are the things making their own light.
pub fn bake_screen(
    static_layer: &Framebuffer,
    cam: &Camera,
    field: &HeightField,
    mask: &[u8],
    caster: &Caster,
    ceil: &Ceil,
    out: &mut Vec<u8>,
    // B84: the first row with a receiver (the static layer's land line); the
    // rows above it are sky and stay 0.
    y0: usize,
) {
    out.clear();
    out.resize(static_layer.w * static_layer.h, 0);
    bake_screen_rows(static_layer, cam, field, mask, caster, ceil, out, y0, static_layer.h);
}

/// Rows `y0..y1` of [`bake_screen`] into an already sized plane (B67).
pub fn bake_screen_rows(
    static_layer: &Framebuffer,
    cam: &Camera,
    field: &HeightField,
    mask: &[u8],
    caster: &Caster,
    ceil: &Ceil,
    out: &mut [u8],
    y0: usize,
    y1: usize,
) {
    let w = static_layer.w;
    let h = static_layer.h.min(y1);
    if h <= y0 {
        return;
    }
    let depth = static_layer.depth().unwrap_or(&[]);
    let px = static_layer.px();
    let n = field.n as f32;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4);
    let rows_per = (h - y0).div_ceil(threads).max(1);
    std::thread::scope(|sc| {
        for (ci, chunk) in out[y0 * w..h * w].chunks_mut(rows_per * w).enumerate() {
            sc.spawn(move || {
    let mut bld = vec![false; w];
    for (yr, orow) in chunk.chunks_mut(w).enumerate() {
        let y = y0 + ci * rows_per + yr;
        let sy = y as f32 + 0.5;
        let row = y * w;
        let mut last_b = usize::MAX; // x of the last building pixel read
        bld.iter_mut().for_each(|b| *b = false);
        for x in 0..w {
            let i = row + x;
            let c = px[i];
            if c == TRANSPARENT {
                continue;
            }
            let a = palette::alpha(c);
            if a == 0xFE {
                continue;
            }
            let key = if depth.is_empty() { 1 } else { depth[i] };
            // B71: a wall or a roof (TAG_BUILT on an even key) takes the shadow of
            // what stands between it and the sun, read from the ceilings.
            if a == TAG_BUILT && key & 1 == 0 {
                // The view-ray walk is the dear part and a shadow is no finer than a
                // field sample (8 px): read it every 4th pixel of a run.
                if x % 4 != 0 && last_b == x.wrapping_sub(1) {
                    orow[x] = orow[x - 1];
                    // (0.83 not 0.5: every 4th pixel column makes the view ray run exactly
                    // along the field's grid corners and a wall face flickers.)
                } else {
                    orow[x] = receiver_shadow(field, ceil, cam, caster, x as f32 + 0.83, sy);
                }
                last_b = x;
                bld[x] = true;
                continue;
            }
            let receiver = a == 0xFF || (a == TAG_BUILT && key & 1 == 1);
            if !receiver || a == TAG_PROP {
                continue;
            }
            // Screen → tile space (the inverse of `Camera::project`). `z` only
            // moves a pixel along one diagonal, so two fixed-point steps settle
            // a sloped tile exactly.
            let sx = x as f32 + 0.5;
            let u = (sx - cam.ox as f32) / map::iso_consts::HALF_W as f32;
            let mut z = 0.0f32;
            let mut tx = 0.0;
            let mut ty = 0.0;
            for _ in 0..2 {
                let v = (sy - cam.oy as f32 + z) / map::iso_consts::HALF_H as f32;
                tx = (u + v) * 0.5;
                ty = (v - u) * 0.5;
                z = field.ground_at(tx, ty);
            }
            // Bilinear sample of the mask at that tile position.
            let fx = (tx * n - 0.5).clamp(0.0, field.w as f32 - 1.001);
            let fy = (ty * n - 0.5).clamp(0.0, field.h as f32 - 1.001);
            let (i0, j0) = (fx as usize, fy as usize);
            let (ax, ay) = (fx - i0 as f32, fy - j0 as f32);
            let at = |i: usize, j: usize| mask[j * field.w + i] as f32;
            let ab = at(i0, j0) + (at(i0 + 1, j0) - at(i0, j0)) * ax;
            let cd = at(i0, j0 + 1) + (at(i0 + 1, j0 + 1) - at(i0, j0 + 1)) * ax;
            orow[x] = (ab + (cd - ab) * ay).clamp(0.0, 255.0) as u8;
        }
        clean_runs(orow, &bld);
    }
            });
        }
    });
}

/// B71: the shadow read off a roof or wall is a few pixels noisier than the
/// ground's (a pitched roof is a staircase in the field, a tower's corner flips
/// between two faces). Along each run of building pixels, drop specks narrower
/// than 5 px (a minimum over 5 taps) and soften what is left (a 5-tap mean).
fn clean_runs(row: &mut [u8], bld: &[bool]) {
    let w = row.len();
    let mut x = 0;
    let mut tmp = [0u8; 8192];
    while x < w {
        if !bld[x] {
            x += 1;
            continue;
        }
        let s = x;
        while x < w && bld[x] {
            x += 1;
        }
        let e = x;
        if e - s < 3 || e - s > tmp.len() {
            continue;
        }
        let at = |i: isize| row[i.clamp(s as isize, e as isize - 1) as usize] as u32;
        for i in s..e {
            let mut m = 255;
            for d in -2..=2 {
                m = m.min(at(i as isize + d));
            }
            tmp[i - s] = m as u8;
        }
        let at2 = |i: isize| tmp[(i.clamp(s as isize, e as isize - 1) as usize) - s] as u32;
        for i in s..e {
            let mut sum = 0;
            for d in -2..=2 {
                sum += at2(i as isize + d);
            }
            row[i] = (sum / 5) as u8;
        }
    }
}

/// Shadows of things thinner than a field sample (B68): street poles, signs and
/// traffic lights. Each is a one-pixel line from its foot along the light,
/// `height / (tan alt)` long, written into the frame's shadow plane (only onto
/// ground pixels) and fading with its length like a penumbra. `posts` are
/// `(foot x, foot y, height)` in frame pixels.
pub fn stamp_posts(
    plane: &mut [u8],
    static_layer: &Framebuffer,
    caster: &Caster,
    posts: &[(f32, f32, f32)],
) {
    let (w, h) = (static_layer.w as i32, static_layer.h as i32);
    let depth = static_layer.depth().unwrap_or(&[]);
    let px = static_layer.px();
    let tan = caster.alt.max(2.0).to_radians().tan();
    for &(fx, fy, ht) in posts {
        let len = ht / (tan * TILE_W as f32); // tiles
        let (tx, ty) = (-caster.toward.0 * len, -caster.toward.1 * len);
        let (du, dv) = ((tx - ty) * 32.0, (tx + ty) * 16.0);
        let n = (du.abs().max(dv.abs())).ceil().max(1.0) as i32;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let x = (fx + du * t).round() as i32;
            let y = (fy + dv * t).round() as i32;
            if x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            let k = (y * w + x) as usize;
            let c = px[k];
            if c == TRANSPARENT {
                continue;
            }
            let a = palette::alpha(c);
            let key = if depth.is_empty() { 1 } else { depth[k] };
            if !(a == 0xFF || (a == TAG_BUILT && key & 1 == 1)) {
                continue;
            }
            let v = (225.0 * (1.0 - 0.55 * t)) as u8;
            plane[k] = plane[k].max(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light::{LightState, Orb};

    /// A flat field with one box caster 57 px tall, sampled the way a city is.
    fn field_with_box() -> HeightField {
        let map = TileMap::fill(20, 20, map::Tile::flat(2), 1);
        let mut f = HeightField::new(&map);
        let g = 2.0 * HEIGHT_UNIT as f32;
        f.stamp_rect(8.0, 8.0, 10.0, 10.0, g + 57.0);
        f
    }

    fn mask_at(f: &HeightField, mask: &[u8], x: f32, y: f32) -> u8 {
        let i = (x * f.n as f32) as usize;
        let j = (y * f.n as f32) as usize;
        mask[j * f.w + i]
    }

    #[test]
    fn a_box_shadows_the_ground_for_its_full_length() {
        let f = field_with_box();
        let mut ls = LightState::default();
        ls.sun = Orb::new(28.4, 90.0); // due east, like a July morning
        ls.dir_strength = 1.0;
        let mut mask = Vec::new();
        let caster = bake_mask(&f, &ls, &mut mask).expect("the sun is up");
        // The shadow lies west of the box: 57 px at 28.4° is 57/0.54 = 105 px
        // (1.64 tiles) beyond the box's west corner (10, 8).
        let (dx, dy) = (-caster.toward.0, -caster.toward.1);
        let mut last_shadowed = 0.0f32;
        let mut d = 0.1f32;
        while d < 3.0 {
            if mask_at(&f, &mask, 10.0 + dx * d, 8.0 + dy * d) > 128 {
                last_shadowed = d;
            }
            d += 0.05;
        }
        assert!(
            (1.3..2.0).contains(&last_shadowed),
            "the shadow reaches {last_shadowed:.2} tiles (expected ~1.64)"
        );
        // And nothing on the sun side of the box.
        assert_eq!(mask_at(&f, &mask, 10.0 - dx * 0.8, 8.0 - dy * 0.8), 0);
    }

    /// The mask must be a gradient at its tip, not a step: that is the penumbra,
    /// and it has to widen with the distance the shadow reaches.
    #[test]
    fn the_edge_is_soft() {
        let f = field_with_box();
        let mut ls = LightState::default();
        ls.sun = Orb::new(20.0, 90.0);
        ls.dir_strength = 1.0;
        let mut mask = Vec::new();
        let c = bake_mask(&f, &ls, &mut mask).expect("the sun is up");
        let (dx, dy) = (-c.toward.0, -c.toward.1);
        let vals: Vec<u8> = (0..60)
            .map(|k| mask_at(&f, &mask, 10.0 + dx * (1.5 + k as f32 * 0.03), 8.0 + dy * (1.5 + k as f32 * 0.03)))
            .collect();
        let mid = vals.iter().filter(|v| **v > 10 && **v < 245).count();
        assert!(mid > 1, "no partial coverage at the shadow tip: {vals:?}");
        assert!(vals[0] > 200, "the shadow's body must be solid: {:?}", vals[0]);
        assert!(*vals.last().unwrap() < 40, "it must end: {:?}", vals.last());
    }

    #[test]
    fn a_taller_caster_reaches_further() {
        let map = TileMap::fill(20, 20, map::Tile::flat(2), 1);
        let mut ls = LightState::default();
        ls.sun = Orb::new(25.0, 200.0);
        ls.dir_strength = 1.0;
        let mut short = HeightField::new(&map);
        short.stamp_rect(8.0, 8.0, 10.0, 10.0, 24.0 + 20.0);
        let mut tall = HeightField::new(&map);
        tall.stamp_rect(8.0, 8.0, 10.0, 10.0, 24.0 + 120.0);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let c = bake_mask(&short, &ls, &mut a).unwrap();
        bake_mask(&tall, &ls, &mut b);
        let (dx, dy) = (-c.toward.0, -c.toward.1);
        let at = |m: &[u8], f: &HeightField, d: f32| mask_at(f, m, 9.0 + dx * d, 9.0 + dy * d);
        assert!(at(&a, &short, 2.0) == 0, "20 px must not reach 2 tiles");
        assert!(at(&b, &tall, 2.0) > 200, "120 px must reach 2 tiles");
    }

    /// A tall tower at (8..10, 8..10) and two low boxes: one in the tower's
    /// shadow, one on the sun's side.
    fn tower_and_boxes() -> (HeightField, Caster, Ceil, Camera, (f32, f32), (f32, f32)) {
        let map = TileMap::fill(24, 24, map::Tile::flat(2), 1);
        let mut f = HeightField::new(&map);
        let g = 2.0 * HEIGHT_UNIT as f32;
        f.stamp_rect(8.0, 8.0, 10.0, 10.0, g + 200.0);
        let mut ls = LightState::default();
        ls.sun = Orb::new(30.0, 200.0);
        ls.dir_strength = 1.0;
        let mut mask = Vec::new();
        let c = bake_mask(&f, &ls, &mut mask).expect("the sun is up");
        let (tx, ty) = (c.toward.0, c.toward.1);
        // 3 tiles from the tower's centre, away from the sun / toward it.
        let shaded = (9.0 - tx * 3.0, 9.0 - ty * 3.0);
        let sunny = (9.0 + tx * 3.5, 9.0 + ty * 3.5);
        for (cx, cy) in [shaded, sunny] {
            f.stamp_rect(cx - 0.5, cy - 0.5, cx + 0.5, cy + 0.5, g + 70.0);
        }
        let ceil = bake_ceil(&f, &c);
        (f, c, ceil, Camera::new(0, 0), shaded, sunny)
    }

    #[test]
    fn a_roof_in_a_towers_shadow_is_shaded_and_one_in_the_sun_is_not() {
        let (f, c, ceil, cam, shaded, sunny) = tower_and_boxes();
        let g = 2 * HEIGHT_UNIT;
        let roof = |p: (f32, f32)| {
            let (sx, sy) = cam.project(p.0, p.1, g + 70);
            receiver_shadow(&f, &ceil, &cam, &c, sx, sy)
        };
        assert!(roof(shaded) > 200, "roof in the tower's shadow: {}", roof(shaded));
        assert_eq!(roof(sunny), 0, "roof in the sun");
    }

    #[test]
    fn a_lone_box_does_not_shade_its_own_roof() {
        let (f, c, ceil, cam, _, sunny) = tower_and_boxes();
        let g = 2 * HEIGHT_UNIT;
        let (sx, sy) = cam.project(sunny.0 + 0.3, sunny.1 - 0.2, g + 70);
        assert_eq!(receiver_shadow(&f, &ceil, &cam, &c, sx, sy), 0);
    }

    #[test]
    fn a_towers_own_walls_are_not_given_a_cast_shadow() {
        let (f, c, ceil, cam, _, _) = tower_and_boxes();
        let g = 2 * HEIGHT_UNIT;
        for k in 0..16 {
            let (sx, sy) = cam.project(8.2 + k as f32 * 0.1, 10.0, g + 60);
            let v = receiver_shadow(&f, &ceil, &cam, &c, sx, sy);
            assert!(v < 20, "tower wall pixel {k} shaded {v}");
        }
    }
}
