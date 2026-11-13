//! B69: trees sway with the real wind.
//!
//! A tree used to be painted into the static layer once and never move again.
//! Now the static pass hands every swaying tree to this layer instead: the tree
//! is drawn once, at rest, into a small scratch buffer and kept as a tight
//! palettised sprite; the static layer receives only its **depth silhouette**
//! (so people, cars and animals still walk behind it), never its colour.
//!
//! Each frame the layer works out, for every tree, how far the crown is blown
//! from the wind and the tree's own phase. The crown is the sprite sheared by
//! rows: nothing moves at the foot and the shift grows towards the tip, so the
//! tree **bends** rather than slides. The shear is a pure function of one float
//! (the lean at the tip), and it only changes at the half-pixel boundaries of
//! some row ([`Profile::thr`]); a tree is therefore re-damaged exactly when its
//! picture changes, which is what makes calm air cost nothing and a breeze cost
//! a handful of small rectangles.
//!
//! The overlay is drawn inside `Scene::composite_rect` (clipped to the damaged
//! rectangle, back to front, tested against the static layer's depth plane so a
//! house in front still hides the tree) and takes the night tint through the
//! same `night_pixel` the static layer uses.
//!
//! Shadows: the canopy's cast shadow stays where the light baked it. The crown
//! moves 1-3 px in a breeze and 4-7 px in a gale; the shadow is a soft, many-pixel
//! smudge on the ground that the sun moves by a tile an hour, so following the
//! sway would be invisible and cost a re-bake. (The soft ground-shade pool a
//! dense wood paints under its crowns also stays put.)
//!
//! Wind comes from the one source the clouds use (`Scene::set_cloud_wind`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use map::TreeSpecies;

use crate::framebuffer::{Framebuffer, Rect};
use crate::light::Lut;
use crate::palette::{Color, TRANSPARENT};

/// The window (px around the ground point) a tree is drawn in: wide enough for
/// the broadest crown, tall enough for the tallest, a little below the foot for
/// fallen leaves. The scratch is just that window, but its origin is moved to
/// the tree's place in the frame, so the position-hashed speckle on bark and
/// leaves is exactly what the static layer used to get.
const WIN_L: i32 = 64;
const WIN_R: i32 = 64;
const WIN_UP: i32 = 92;
const WIN_DOWN: i32 = 24;

/// The most a crown can be blown, px at the tip (before the profile).
const XMAX: f32 = 10.0;
/// Grid cell of the spatial index, px.
const CELL: i32 = 32;
/// Below this eased wind (km/h) the air is calm: no work at all.
const CALM_KMH: f32 = 1.5;

/// Keys at or above this are park trees, not map tiles.
pub const PARK_KEY: i32 = 1 << 24;

/// How a tree bends.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Class {
    /// A round crown on a trunk: the crown moves as a mass.
    Broad,
    /// A spire: the tip whips more than the skirts.
    Conifer,
    /// Fronds on a bare trunk: bends from the foot.
    Palm,
}

/// Which species sway, and how much (a multiplier on the tip lean). `None`
/// means rigid: the saguaro and the Joshua tree stay painted in the static
/// layer exactly as before.
fn species_params(sp: TreeSpecies) -> Option<(Class, f32)> {
    use TreeSpecies as T;
    Some(match sp {
        T::Oak => (Class::Broad, 1.0),
        T::Maple => (Class::Broad, 1.0),
        T::Birch => (Class::Broad, 1.15),
        T::Beech => (Class::Broad, 0.9),
        T::Elm => (Class::Broad, 1.0),
        T::Willow => (Class::Broad, 1.3),
        T::Poplar => (Class::Conifer, 1.0),
        T::Apple => (Class::Broad, 0.9),
        T::Cherry => (Class::Broad, 0.95),
        T::Pine => (Class::Conifer, 1.0),
        T::Spruce => (Class::Conifer, 1.05),
        T::Fir => (Class::Conifer, 1.1),
        T::Cedar => (Class::Conifer, 0.9),
        T::Palm => (Class::Palm, 1.4),
        T::Cypress => (Class::Conifer, 0.85),
        T::JapaneseMaple => (Class::Broad, 0.9),
        T::FanPalm => (Class::Palm, 1.3),
        T::Flamboyant => (Class::Broad, 1.0),
        T::Mangrove => (Class::Broad, 0.6),
        T::Acacia => (Class::Broad, 0.85),
        T::Baobab => (Class::Broad, 0.3),
        T::BaldCypress => (Class::Conifer, 0.75),
        T::Olive => (Class::Broad, 0.85),
        T::Cactus | T::Joshua => return None,
    })
}

/// Does this species sway at all?
pub fn sways(sp: TreeSpecies) -> bool {
    species_params(sp).is_some()
}

/// Per-row weights of the bend for a sprite.
struct Profile {
    /// Row weight in 0..=1 (0 at the foot, 1 at the tip).
    w: Vec<f32>,
    /// Rows from the top that ever move.
    moving_rows: usize,
}

impl Profile {
    fn build(class: Class, top: i32, rows: usize) -> Self {
        let (h0, p) = match class {
            Class::Broad => (0.34, 1.7),
            Class::Conifer => (0.10, 2.0),
            Class::Palm => (0.0, 1.8),
        };
        let height = top.max(1) as f32;
        let mut w = Vec::with_capacity(rows);
        let mut moving_rows = 0;
        for r in 0..rows {
            let hgt = (top - r as i32) as f32;
            let u = ((hgt - h0 * height) / (height * (1.0 - h0))).clamp(0.0, 1.0);
            let v = u.powf(p);
            if v > 1e-3 {
                moving_rows = r + 1;
            }
            w.push(v);
        }
        Self { w, moving_rows }
    }

    /// Row `r`'s sideways shift, px, for a lean of `q` half-pixels at the tip.
    #[inline]
    fn shift(&self, r: usize, q: i16) -> i32 {
        let wr = self.w[r];
        if q == 0 || wr <= 1e-3 {
            return 0;
        }
        let m = ((q.unsigned_abs() as f32) * 0.5 * wr + 0.5).floor() as i32;
        if q < 0 { -m } else { m }
    }

    /// The rows (first, last+1) and the widest shift among them where leans
    /// `a` and `b` differ; `None` when the two draw identically.
    fn diff(&self, a: i16, b: i16) -> Option<(usize, usize, i32)> {
        let (mut first, mut last, mut widest) = (usize::MAX, 0usize, 0i32);
        for r in 0..self.moving_rows {
            let (sa, sb) = (self.shift(r, a), self.shift(r, b));
            if sa != sb {
                first = first.min(r);
                last = r + 1;
                widest = widest.max(sa.abs()).max(sb.abs());
            }
        }
        (first != usize::MAX).then_some((first, last, widest))
    }
}

/// A leaf (or petal) let go by a gust.
struct Leaf {
    x: f32,
    y: f32,
    vx: f32,
    phase: f32,
    age: f32,
    floor: f32,
    key: u16,
    col: Color,
    last: (i32, i32),
}

/// One tree.
struct Entry {
    tile: (i32, i32),
    /// The static painter key of the tree's tile.
    key: u16,
    /// Screen top-left of the sprite, and its size.
    x0: i32,
    y0: i32,
    w: u16,
    h: u16,
    /// Palette indices (0 = empty) and the palette (entry 0 unused).
    idx: Vec<u8>,
    pal: Vec<Color>,
    prof: u32,
    /// Everything that scales the tip lean: species, size.
    gain: f32,
    /// Natural-frequency ratio and phase, fixed per tree.
    ratio: f32,
    phase: f32,
    /// Ground row, and whether this is a leafy (shedding) tree.
    foot: i32,
    broad: bool,
    /// Tile position, for the travelling gust.
    tx: f32,
    ty: f32,
    /// The pose being shown: the lean at the tip in half pixels (signed).
    q: i16,
    /// The palette as the current light grades it, and the grade it is for.
    tp: RefCell<Vec<Color>>,
    tp_gen: Cell<u32>,
    /// Could anything that is not a tree stand in front of this one? Checked
    /// once per repaint of its surroundings (`settle`); a tree in the open (the
    /// common case) is drawn without reading the depth plane at all, which is
    /// what the draw was waiting on.
    occluded: bool,
    checked: bool,
}

impl Entry {
    /// The whole footprint including the biggest swing, for the index.
    fn reach(&self) -> Rect {
        let m = XMAX as i32 + 1;
        Rect::new(self.x0 - m, self.y0, self.x0 + self.w as i32 + m, self.y0 + self.h as i32)
    }
}

/// The eased wind.
#[derive(Clone, Copy, Debug)]
struct Wind {
    dir: f32,
    kmh: f32,
    target_dir: f32,
    target_kmh: f32,
    /// Unit vector the wind blows towards, in tile space.
    ux: f32,
    uy: f32,
    /// Share of the push that lands sideways on screen (-1..1).
    fx: f32,
    /// Phase of the sway and the travel of the gust field.
    osc: f32,
    gust: f32,
}

impl Default for Wind {
    fn default() -> Self {
        let mut w = Self {
            dir: 250.0,
            kmh: 0.0,
            target_dir: 250.0,
            target_kmh: 0.0,
            ux: 1.0,
            uy: 0.0,
            fx: 0.0,
            osc: 0.0,
            gust: 0.0,
        };
        w.aim();
        w
    }
}

impl Wind {
    fn aim(&mut self) {
        let (tx, ty) = crate::light::toward(self.dir + 180.0);
        self.ux = tx;
        self.uy = ty;
        let sx = (tx - ty) * map::iso_consts::HALF_W as f32;
        let sy = (tx + ty) * map::iso_consts::HALF_H as f32;
        let n = (sx * sx + sy * sy).sqrt().max(1e-3);
        self.fx = sx / n;
    }
}

/// A cheap sine (parabola with a correction), plenty for a sway.
#[inline]
fn fsin(x: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    // Wrap to [-PI, PI] without a libm call (the inputs are small).
    let t = x * (1.0 / TAU);
    let mut x = (t - t as i64 as f32) * TAU;
    if x > PI {
        x -= TAU;
    } else if x < -PI {
        x += TAU;
    }
    let y = 1.273_239_5 * x - 0.405_284_73 * x * x.abs();
    0.225 * (y * y.abs() - y) + y
}

#[inline]
fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

#[inline]
fn unit(x: u32) -> f32 {
    (hash32(x) & 0xFFFF) as f32 / 65535.0
}

/// What the layer reports.
#[derive(Clone, Copy, Debug, Default)]
pub struct SwayStats {
    /// Trees held.
    pub trees: usize,
    /// Trees currently away from rest.
    pub moving: usize,
    /// Trees whose picture changed in the last tick.
    pub repaints_last: usize,
    /// All repaints so far, and the ticks that did any work.
    pub repaints_total: u64,
    pub ticks_busy: u64,
    /// The eased wind in use (km/h).
    pub kmh: f32,
    /// Bytes of sprite data held.
    pub bytes: usize,
    /// Most colours any one tree needed (a sprite holds up to 255), and how
    /// many trees had more than that.
    pub max_colours: usize,
    pub over_palette: usize,
    /// Leaves or petals in the air.
    pub leaves: usize,
}

/// Direct-mapped tint cache for the overlay.
struct TintCache {
    sig: [u32; 8],
    lut: Option<Box<Lut>>,
    src: Vec<u32>,
    out: Vec<u32>,
}

impl Default for TintCache {
    fn default() -> Self {
        Self {
            sig: [0; 8],
            lut: None,
            src: vec![0; 2048],
            out: vec![0; 2048],
        }
    }
}

#[derive(Default)]
pub struct TreeLayer {
    slots: Vec<Option<Entry>>,
    by_tile: HashMap<(i32, i32), usize>,
    free: Vec<usize>,
    scratch: Option<Framebuffer>,
    /// The first land row: the silhouette is tagged on the land only.
    land_top: i32,
    fw: usize,
    fh: usize,
    profiles: Vec<Profile>,
    prof_ix: HashMap<(Class, i32, usize), u32>,
    wind: Wind,
    grid: Vec<Vec<u32>>,
    gcols: i32,
    grows: i32,
    grid_dirty: bool,
    /// Rectangles the capture pass changed (a tree removed, replaced): the
    /// scene folds them into the next composite.
    damage: Vec<Rect>,
    moving: usize,
    max_colours: usize,
    over_palette: usize,
    /// Tools: hold every tree at this lean (half pixels) whatever the wind.
    force: Option<i16>,
    leaves: Vec<Leaf>,
    leaf_cols: [Color; 3],
    leaf_strength: f32,
    leaf_acc: f32,
    rng: u32,
    stats: SwayStats,
    tint: RefCell<TintCache>,
    /// Bumped whenever the light's grade changes (invalidates every `tp`).
    tint_gen: Cell<u32>,
    cand: RefCell<Vec<u32>>,
}

thread_local! {
    static SINK: RefCell<Option<TreeLayer>> = const { RefCell::new(None) };
}

/// Make `layer` the one the static pass captures trees into (this thread).
pub fn install(layer: TreeLayer) {
    SINK.with(|s| *s.borrow_mut() = Some(layer));
}

/// Take the layer back after the pass.
pub fn uninstall() -> TreeLayer {
    SINK.with(|s| s.borrow_mut().take()).unwrap_or_default()
}

/// Is a capture pass under way?
pub fn active() -> bool {
    SINK.with(|s| s.borrow().is_some())
}

/// Run `f` on the installed layer, if any.
pub fn with_sink<R>(f: impl FnOnce(&mut TreeLayer) -> R) -> Option<R> {
    SINK.with(|s| s.borrow_mut().as_mut().map(f))
}

/// B69b: a tree whose pixels are already known (a landform flank's): put them
/// in the scratch and capture. False when no layer is installed or it cannot
/// take the tree.
pub fn capture_pixels(
    fb: &mut Framebuffer,
    anchor: (i32, i32),
    px: &[(i16, i16, Color)],
    species: TreeSpecies,
) -> bool {
    if !sways(species) {
        return false;
    }
    with_sink(|l| {
        let scratch = l.scratch(anchor.0, anchor.1);
        for &(dx, dy, c) in px {
            scratch.put(anchor.0 + dx as i32, anchor.1 + dy as i32, c);
        }
        l.insert(fb, (PARK_KEY + 2_000_000 + anchor.0, anchor.1), anchor, species);
    })
    .is_some()
}

/// A tile is being drawn: whatever tree was captured for it is stale.
pub fn forget(x: i32, y: i32) {
    SINK.with(|s| {
        if let Some(l) = s.borrow_mut().as_mut() {
            l.remove(x, y);
        }
    });
}

impl TreeLayer {
    /// B61f: the bytes the sway sprites and their palettes hold (for the
    /// memory report). The scratch window is included; it is small.
    pub fn bytes(&self) -> usize {
        let mut b = 0usize;
        for e in self.slots.iter().flatten() {
            b += e.idx.capacity() + e.pal.capacity() * std::mem::size_of::<Color>();
        }
        if let Some(s) = &self.scratch {
            b += s.bytes();
        }
        for p in &self.profiles {
            b += p.w.capacity() * std::mem::size_of::<f32>();
        }
        b += self.grid.capacity() * std::mem::size_of::<Vec<u32>>();
        for g in &self.grid {
            b += g.capacity() * std::mem::size_of::<u32>();
        }
        b += self.leaves.capacity() * std::mem::size_of::<Leaf>();
        b
    }

    /// Start empty for a frame of `w` x `h` (a full rebuild).
    pub fn reset(&mut self, w: usize, h: usize, land_top: i32) {
        self.land_top = land_top;
        self.slots.clear();
        self.by_tile.clear();
        self.free.clear();
        self.fw = w;
        self.fh = h;
        self.grid_dirty = true;
        self.moving = 0;
        self.damage.clear();
        self.max_colours = 0;
        self.over_palette = 0;
    }

    /// The scratch framebuffer: a small window that takes the frame's
    /// coordinates round `(cx, cy)`, cleared for the next tree.
    pub fn scratch(&mut self, cx: i32, cy: i32) -> &mut Framebuffer {
        let s = self.scratch.get_or_insert_with(|| {
            Framebuffer::new((WIN_L + WIN_R) as usize, (WIN_UP + WIN_DOWN) as usize)
        });
        s.set_origin(cx - WIN_L, cy - WIN_UP);
        s.clear(TRANSPARENT);
        s
    }

    /// Kept for the callers of the old frame-sized scratch: nothing to free.
    pub fn release_scratch(&mut self) {}

    /// Drop every park tree (key x >= `PARK_KEY`) whose box meets `r`: the
    /// lot is about to be repainted and re-captures what is still there.
    pub fn forget_parks_in(&mut self, r: Rect) {
        let keys: Vec<(i32, i32)> = self
            .by_tile
            .keys()
            .copied()
            .filter(|k| k.0 >= PARK_KEY)
            .filter(|k| {
                self.by_tile
                    .get(k)
                    .and_then(|&i| self.slots[i].as_ref())
                    .is_some_and(|e| e.reach().intersects(&r))
            })
            .collect();
        for k in keys {
            self.remove(k.0, k.1);
        }
    }

    fn remove(&mut self, x: i32, y: i32) {
        if let Some(i) = self.by_tile.remove(&(x, y)) {
            if let Some(e) = self.slots[i].take() {
                self.damage.push(e.reach().intersect(&Rect::from_size(self.fw, self.fh)));
                if e.q != 0 {
                    self.moving -= 1;
                }
            }
            self.free.push(i);
            self.grid_dirty = true;
        }
    }

    /// Keep the tree drawn in the scratch as a sprite and stamp its silhouette
    /// into the static layer's depth plane.
    pub fn insert(
        &mut self,
        fb: &mut Framebuffer,
        tile: (i32, i32),
        anchor: (i32, i32),
        sp: TreeSpecies,
    ) {
        if let Some((class, gain)) = species_params(sp) {
            self.insert_with(fb, tile, anchor, class, gain);
        }
    }

    /// `insert` for a tree that is not a map prop (a park's): the class and
    /// gain are given, and `tile` is any key not used by a tile.
    pub fn insert_with(
        &mut self,
        fb: &mut Framebuffer,
        tile: (i32, i32),
        anchor: (i32, i32),
        class: Class,
        species_gain: f32,
    ) {
        let (cx, cy) = anchor;
        let Some(scratch) = self.scratch.as_ref() else {
            return;
        };
        let (sw, sh) = (scratch.w as i32, scratch.h as i32);
        let sp_px = scratch.px();
        // Tight box of the opaque pixels.
        let (mut bx0, mut by0, mut bx1, mut by1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for y in 0..sh {
            for x in 0..sw {
                if (sp_px[(y * sw + x) as usize] >> 24) >= 0xF0 {
                    bx0 = bx0.min(x);
                    bx1 = bx1.max(x);
                    by0 = by0.min(y);
                    by1 = by1.max(y);
                }
            }
        }
        if bx1 < bx0 {
            return;
        }
        let (w, h) = ((bx1 - bx0 + 1) as usize, (by1 - by0 + 1) as usize);
        let mut idx = vec![0u8; w * h];
        let mut pal: Vec<Color> = vec![0];
        let mut over = 0usize;
        for y in 0..h {
            for x in 0..w {
                let c = sp_px[((by0 + y as i32) * sw + bx0 + x as i32) as usize];
                if (c >> 24) < 0xF0 {
                    continue;
                }
                let c = c | 0xFF00_0000;
                let k = match pal.iter().position(|&p| p == c) {
                    Some(k) => k,
                    None if pal.len() < 255 => {
                        pal.push(c);
                        pal.len() - 1
                    }
                    None => {
                        over += 1;
                        // Out of palette: the nearest colour already held.
                        let near = |p: &Color| {
                            let d = |sh: u32| ((p >> sh) & 0xFF) as i32 - ((c >> sh) & 0xFF) as i32;
                            d(16) * d(16) + d(8) * d(8) + d(0) * d(0)
                        };
                        (1..pal.len()).min_by_key(|&i| near(&pal[i])).unwrap_or(1)
                    }
                };
                idx[y * w + x] = k as u8;
            }
        }
        self.max_colours = self.max_colours.max(pal.len() - 1 + over.min(1));
        if over > 0 {
            self.over_palette += 1;
        }
        // Where the sprite sits on screen.
        let (x0, y0) = (cx - WIN_L + bx0, cy - WIN_UP + by0);
        if !Rect::new(x0 - 12, y0, x0 + w as i32 + 12, y0 + h as i32).intersects(&Rect::from_size(self.fw, self.fh)) {
            return;
        }
        let key = fb.depth_key();
        // The silhouette goes into the depth plane, and the ground under it is
        // tagged as a prop on the land (no frame-sized side plane: the overlay
        // lets a tree through wherever the static pixel carries the tag).
        {
            let clip = fb.clip();
            let fbw = fb.w;
            let land_top = self.land_top;
            for y in 0..h {
                let sy = y0 + y as i32;
                if sy < clip.y0 || sy >= clip.y1 {
                    continue;
                }
                for x in 0..w {
                    if idx[y * w + x] == 0 {
                        continue;
                    }
                    let sx = x0 + x as i32;
                    if sx < clip.x0 || sx >= clip.x1 {
                        continue;
                    }
                    let i = sy as usize * fbw + sx as usize;
                    if let Some(d) = fb.depth_mut() {
                        d[i] = key;
                    }
                    if sy >= land_top {
                        let c = fb.px()[i];
                        if c != TRANSPARENT {
                            fb.px_mut()[i] = (c & 0x00FF_FFFF) | ((crate::framebuffer::TAG_PROP as u32) << 24);
                        }
                    }
                }
            }
        }
        let top = cy - y0;
        let pk = (class, top, h);
        let prof = match self.prof_ix.get(&pk) {
            Some(&p) => p,
            None => {
                self.profiles.push(Profile::build(class, top, h));
                let p = (self.profiles.len() - 1) as u32;
                self.prof_ix.insert(pk, p);
                p
            }
        };
        // Taller trees lean further and sway slower; small ones are stiff.
        let rise = top.max(8) as f32;
        let hscale = (rise / 40.0).powf(0.7);
        let seed = (tile.0 as u32).wrapping_mul(73856093) ^ (tile.1 as u32).wrapping_mul(19349663);
        let ratio = (0.82 + 0.36 * unit(seed ^ 0x9e37)) * (40.0 / rise).sqrt().clamp(0.7, 1.3);
        let phase = unit(seed ^ 0x51ed) * std::f32::consts::TAU;
        let entry = Entry {
            tile,
            key,
            x0,
            y0,
            w: w as u16,
            h: h as u16,
            idx,
            pal,
            prof,
            foot: cy,
            broad: class == Class::Broad,
            gain: species_gain * hscale,
            ratio,
            phase,
            tx: tile.0 as f32 + 0.5,
            ty: tile.1 as f32 + 0.5,
            q: 0,
            tp: RefCell::new(Vec::new()),
            tp_gen: Cell::new(0),
            occluded: true,
            checked: false,
        };
        self.remove(tile.0, tile.1);
        let slot = match self.free.pop() {
            Some(i) => {
                self.slots[i] = Some(entry);
                i
            }
            None => {
                self.slots.push(Some(entry));
                self.slots.len() - 1
            }
        };
        self.by_tile.insert(tile, slot);
        let reach = self.slots[slot].as_ref().unwrap().reach().intersect(&Rect::from_size(self.fw, self.fh));
        self.damage.push(reach);
        self.grid_dirty = true;
    }

    /// The static layer around these rectangles was repainted: re-check which
    /// trees might be hidden by something.
    pub fn invalidate(&mut self, r: Rect) {
        let r = r.intersect(&Rect::from_size(self.fw, self.fh));
        if r.is_empty() || self.grid.is_empty() {
            return;
        }
        for cy in r.y0 / CELL..=(r.y1 - 1) / CELL {
            for cx in r.x0 / CELL..=(r.x1 - 1) / CELL {
                if let Some(cell) = self.grid.get((cy * self.gcols + cx) as usize) {
                    for &i in cell {
                        if let Some(e) = self.slots[i as usize].as_mut() {
                            e.checked = false;
                        }
                    }
                }
            }
        }
    }

    /// Work out, for every tree not yet checked, whether anything other than a
    /// tree could stand in front of it anywhere in its swing.
    pub fn settle(&mut self, depth: &[u16], stat: &[Color]) {
        let m = XMAX as i32 + 1;
        let (fw, fh) = (self.fw as i32, self.fh as i32);
        for e in self.slots.iter_mut().flatten() {
            if e.checked {
                continue;
            }
            e.checked = true;
            let (x0, x1) = ((e.x0 - m).max(0), (e.x0 + e.w as i32 + m).min(fw));
            let (y0, y1) = (e.y0.max(0), (e.y0 + e.h as i32).min(fh));
            let mut occ = false;
            'scan: for y in y0..y1 {
                let row = y as usize * fw as usize;
                for x in x0..x1 {
                    let i = row + x as usize;
                    if depth[i] > e.key && (stat[i] >> 24) as u8 != crate::framebuffer::TAG_PROP {
                        occ = true;
                        break 'scan;
                    }
                }
            }
            e.occluded = occ;
        }
    }

    /// Trees that something can stand in front of (diagnostics).
    pub fn occluded_count(&self) -> usize {
        self.slots.iter().flatten().filter(|e| e.occluded).count()
    }

    /// Rectangles the capture pass wants repainted (drained).
    pub fn take_damage(&mut self, out: &mut Vec<Rect>) {
        out.append(&mut self.damage);
    }

    /// Rebuild the spatial index if trees came or went.
    pub fn reindex(&mut self) {
        if !self.grid_dirty {
            return;
        }
        self.grid_dirty = false;
        self.gcols = (self.fw as i32 + CELL - 1) / CELL;
        self.grows = (self.fh as i32 + CELL - 1) / CELL;
        self.grid.clear();
        self.grid.resize((self.gcols * self.grows).max(0) as usize, Vec::new());
        let bounds = Rect::from_size(self.fw, self.fh);
        for (i, s) in self.slots.iter().enumerate() {
            let Some(e) = s else { continue };
            let r = e.reach().intersect(&bounds);
            if r.is_empty() {
                continue;
            }
            for cy in r.y0 / CELL..=(r.y1 - 1) / CELL {
                for cx in r.x0 / CELL..=(r.x1 - 1) / CELL {
                    self.grid[(cy * self.gcols + cx) as usize].push(i as u32);
                }
            }
        }
        self.stats.trees = self.by_tile.len();
        self.stats.bytes = self
            .slots
            .iter()
            .flatten()
            .map(|e| e.idx.len() + e.pal.len() * 4 + std::mem::size_of::<Entry>())
            .sum::<usize>()
;
    }

    /// What the gusts shake loose: the colours of the leaves (or petals) and
    /// how heavily the season sheds (0 = nothing).
    pub fn set_leaf_look(&mut self, cols: [Color; 3], strength: f32) {
        self.leaf_cols = cols;
        self.leaf_strength = strength.clamp(0.0, 1.0);
    }

    /// Leaves in the air now.
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    fn rnd(&mut self) -> f32 {
        self.rng = hash32(self.rng.wrapping_add(0x9e37_79b9));
        (self.rng & 0xFFFF) as f32 / 65535.0
    }

    /// Spawn, move and retire the leaves; their old and new boxes go to `out`.
    fn tick_leaves(&mut self, dt: f32, out: &mut Vec<Rect>) {
        let kmh = self.wind.kmh;
        // B76: in leaf-fall a few leaves always drift down, even on a still day
        // ("a few drifting leaves near trees, more on windy days"). B69 only
        // shed above 9 km/h, so at the fair-weather default of 8 km/h a whole
        // autumn passed without one leaf falling.
        let gust = ((kmh - 9.0) / 25.0).clamp(0.0, 1.6);
        let rate = self.leaf_strength * (0.30 + gust * 4.2) * 2.4;
        if rate > 0.0 && self.leaves.len() < 120 && self.by_tile.len() > 0 {
            self.leaf_acc += rate * dt;
            while self.leaf_acc >= 1.0 {
                self.leaf_acc -= 1.0;
                let n = self.slots.len();
                let k = (self.rnd() * n as f32) as usize % n.max(1);
                let (rx, ry, rc, rp) = (self.rnd(), self.rnd(), self.rnd(), self.rnd());
                let Some(e) = self.slots.get(k).and_then(|s| s.as_ref()) else { continue };
                if !e.broad {
                    continue;
                }
                let col = self.leaf_cols[(rc * 3.0) as usize % 3];
                let x = e.x0 as f32 + e.w as f32 * (0.2 + 0.6 * rx);
                let y = e.y0 as f32 + e.h as f32 * (0.15 + 0.4 * ry);
                out.push(Rect::new(x as i32, y as i32, x as i32 + 2, y as i32 + 2));
                self.leaves.push(Leaf {
                    x,
                    y,
                    vx: 0.0,
                    phase: rp * std::f32::consts::TAU,
                    age: 0.0,
                    floor: (e.foot + 4) as f32,
                    key: e.key,
                    col,
                    last: (x as i32, y as i32),
                });
            }
        }
        let fx = self.wind.fx;
        let drift = fx * kmh * 0.5;
        let mut i = 0;
        while i < self.leaves.len() {
            let l = &mut self.leaves[i];
            l.age += dt;
            // The air carries it sideways, a flutter rocks it, it sinks.
            l.vx += (drift - l.vx) * (1.0 - (-dt * 1.5).exp());
            let flutter = fsin(l.age * 4.5 + l.phase) * 5.0;
            l.x += (l.vx + flutter) * dt;
            l.y += (9.0 + 4.0 * fsin(l.age * 3.0 + l.phase * 2.0)) * dt;
            let now = (l.x as i32, l.y as i32);
            let gone = l.y >= l.floor || l.age > 10.0;
            if now != l.last || gone {
                for p in [l.last, now] {
                    out.push(Rect::new(p.0, p.1, p.0 + 2, p.1 + 2));
                }
                l.last = now;
            }
            if gone {
                self.leaves.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// For tools: hold every tree at one lean (half pixels at the tip).
    pub fn force_lean(&mut self, q: Option<i16>) {
        self.force = q;
    }

    /// Set the wind the trees feel (the one the clouds drift with).
    pub fn set_wind(&mut self, dir_deg: f32, kmh: f32) {
        self.wind.target_dir = dir_deg;
        self.wind.target_kmh = kmh.clamp(0.0, 140.0);
    }

    pub fn stats(&self) -> SwayStats {
        SwayStats {
            moving: self.moving,
            kmh: self.wind.kmh,
            max_colours: self.max_colours,
            over_palette: self.over_palette,
            leaves: self.leaves.len(),
            ..self.stats
        }
    }

    /// The lean (half pixels at the tip, signed) the tree on `tile` is shown
    /// with, if there is one.
    pub fn lean_q(&self, tile: (i32, i32)) -> Option<i16> {
        let i = *self.by_tile.get(&tile)?;
        self.slots[i].as_ref().map(|e| e.q)
    }

    /// The screen box a tree on `tile` occupies at rest (the tests need it).
    pub fn rest_rect(&self, tile: (i32, i32)) -> Option<Rect> {
        let i = *self.by_tile.get(&tile)?;
        self.slots[i]
            .as_ref()
            .map(|e| Rect::new(e.x0, e.y0, e.x0 + e.w as i32, e.y0 + e.h as i32))
    }

    /// The top-most screen row any tree reaches, if there are any (the sky
    /// band's skyline wants the treeline).
    pub fn top_row(&self) -> Option<i32> {
        self.slots.iter().flatten().map(|e| e.y0).min()
    }

    /// The lean (px at the tip, signed) tree `e` is given by the wind now.
    fn lean(&self, e: &Entry) -> f32 {
        let w = &self.wind;
        let s = e.tx * w.ux + e.ty * w.uy - w.gust;
        // Gusts: two waves of different length, peaky.
        let g = 0.5
            + 0.5 * (0.65 * fsin(s * (std::f32::consts::TAU / 7.0) + 0.7)
                + 0.35 * fsin(s * (std::f32::consts::TAU / 3.1) + 2.1));
        let g = g * g;
        let v = w.kmh * (0.55 + 1.3 * g);
        let mean = 0.045 * v * w.fx;
        let amp = 0.03 * v;
        let ph = w.osc * e.ratio + e.phase + s * 0.35;
        let o = 0.8 * fsin(ph) + 0.2 * fsin(ph * 2.31 + 1.3);
        let flutter = if w.kmh > 25.0 {
            0.006 * (w.kmh - 25.0) * fsin(ph * 3.7 + e.phase * 3.0)
        } else {
            0.0
        };
        ((mean + amp * o + flutter) * e.gain).clamp(-XMAX, XMAX)
    }

    /// Advance the wind and the trees by `dt` seconds; every tree whose picture
    /// changed pushes its before-and-after rectangle onto `out`.
    pub fn tick(&mut self, dt: f32, out: &mut Vec<Rect>) {
        self.stats.repaints_last = 0;
        let dt = dt.clamp(0.0, 0.25);
        // Ease the wind: a front arriving is a rising breeze, not a step.
        let k = 1.0 - (-dt / 3.0).exp();
        self.wind.kmh += (self.wind.target_kmh - self.wind.kmh) * k;
        let mut d = self.wind.target_dir - self.wind.dir;
        d = (d + 180.0).rem_euclid(360.0) - 180.0;
        self.wind.dir += d * k;
        self.tick_leaves(dt, out);
        if self.wind.kmh < CALM_KMH && self.moving == 0 && self.force.is_none() {
            // Calm: nothing to compute, nothing to repaint.
            return;
        }
        self.wind.aim();
        let kmh = self.wind.kmh;
        // The phase advances faster in a stronger wind, and continuously.
        self.wind.osc =
            (self.wind.osc + dt * std::f32::consts::TAU * 0.38 * (1.0 + kmh.min(80.0) / 320.0))
                % (std::f32::consts::TAU * 64.0);
        let travel = 0.8 * (0.018 * kmh + 0.03);
        self.wind.gust = (self.wind.gust + dt * travel) % 4096.0;
        let calm = kmh < CALM_KMH;
        self.stats.ticks_busy += 1;
        let mut changed = 0usize;
        for i in 0..self.slots.len() {
            let Some(e) = self.slots[i].as_ref() else { continue };
            let x = if calm { 0.0 } else { self.lean(e) };
            let q = match self.force {
                Some(f) => f,
                None => (x.round() as i16) * 2,
            };
            if q == e.q {
                continue;
            }
            let prof = &self.profiles[e.prof as usize];
            let diff = prof.diff(e.q, q);
            let (x0, y0, w) = (e.x0, e.y0, e.w as i32);
            let e = self.slots[i].as_mut().unwrap();
            if q != 0 && e.q == 0 {
                self.moving += 1;
            } else if q == 0 && e.q != 0 {
                self.moving -= 1;
            }
            e.q = q;
            // The pose changed; the picture may not (a half pixel that no row
            // rounds across). Only the rows that moved are damaged.
            if let Some((r0, r1, m)) = diff {
                out.push(Rect::new(x0 - m, y0 + r0 as i32, x0 + w + m, y0 + r1 as i32));
                changed += 1;
            }
        }
        self.stats.repaints_last = changed;
        self.stats.repaints_total += changed as u64;
    }

    /// Draw every tree that touches `clip`, back to front.
    ///
    /// `depth` is the static layer's painter plane; `tint` and `bulbs` the
    /// scene's night grade, and `night` its per-pixel function.
    pub fn draw(
        &self,
        dst: &mut Framebuffer,
        clip: Rect,
        depth: &[u16],
        stat: &[Color],
        tint: [f32; 3],
        bulbs: &[Color],
        night: fn(Color, &Lut, &[Color]) -> Color,
    ) {
        if (self.by_tile.is_empty() && self.leaves.is_empty()) || self.grid.is_empty() {
            return;
        }
        let clip = clip.intersect(&dst.clip()).intersect(&Rect::from_size(self.fw, self.fh));
        if clip.is_empty() {
            return;
        }
        let mut cand = self.cand.borrow_mut();
        cand.clear();
        for cy in clip.y0 / CELL..=(clip.y1 - 1) / CELL {
            for cx in clip.x0 / CELL..=(clip.x1 - 1) / CELL {
                if let Some(cell) = self.grid.get((cy * self.gcols + cx) as usize) {
                    cand.extend_from_slice(cell);
                }
            }
        }
        cand.sort_unstable();
        cand.dedup();
        // Only trees whose swing reaches the clip; then back to front.
        let m = XMAX as i32 + 1;
        cand.retain(|&i| {
            self.slots[i as usize].as_ref().is_some_and(|e| {
                e.y0 < clip.y1 && e.y0 + (e.h as i32) > clip.y0 && e.x0 - m < clip.x1 && e.x0 + e.w as i32 + m > clip.x0
            })
        });
        cand.sort_unstable_by_key(|&i| (self.slots[i as usize].as_ref().map_or(0, |e| e.key), i));
        let mut tc = self.tint.borrow_mut();
        // Refresh the lut and the cache when the grade moved.
        let l = |v: f32| (v * 1000.0) as u32;
        let sig = [
            l(tint[0]),
            l(tint[1]),
            l(tint[2]),
            bulbs.len() as u32,
            bulbs.first().copied().unwrap_or(0),
            bulbs.last().copied().unwrap_or(0),
            0,
            0,
        ];
        if tc.lut.is_none() || tc.sig != sig {
            tc.sig = sig;
            tc.lut = Some(crate::light::lut(tint));
            tc.src.fill(0);
            self.tint_gen.set(self.tint_gen.get().wrapping_add(1));
        }
        let grade = self.tint_gen.get();
        let TintCache { lut, src, out, .. } = &mut *tc;
        let lut: &Lut = lut.as_ref().unwrap();
        let dw = dst.w;
        let px = dst.px_mut();
        for &ci in cand.iter() {
            let Some(e) = self.slots[ci as usize].as_ref() else { continue };
            let prof = &self.profiles[e.prof as usize];
            let w = e.w as usize;
            // The tinted palette, once per tree per grade.
            if e.tp_gen.get() != grade || e.tp.borrow().len() != e.pal.len() {
                let mut tp = e.tp.borrow_mut();
                tp.clear();
                tp.push(0);
                for &col in e.pal.iter().skip(1) {
                    let h = ((col ^ (col >> 11) ^ (col >> 19)) & 2047) as usize;
                    tp.push(if src[h] == col {
                        out[h]
                    } else {
                        let o = night(col, lut, bulbs);
                        src[h] = col;
                        out[h] = o;
                        o
                    });
                }
                e.tp_gen.set(grade);
            }
            let tp = e.tp.borrow();
            let r0 = (clip.y0 - e.y0).max(0) as usize;
            let r1 = ((clip.y1 - e.y0).max(0) as usize).min(e.h as usize);
            for r in r0..r1 {
                let y = e.y0 + r as i32;
                let xs = e.x0 + prof.shift(r, e.q);
                let c0 = (clip.x0 - xs).max(0) as usize;
                let c1 = ((clip.x1 - xs).max(0) as usize).min(w);
                if c0 >= c1 {
                    continue;
                }
                let row = &e.idx[r * w..(r + 1) * w];
                // B99d: a row that leans left of the frame (`xs` negative) used
                // to wrap `xs as usize` into a huge index and overflow the add.
                // The offsets are already relative to `xs`, so the base is only
                // signed arithmetic until the final, in-range cast.
                let base = (y as i64) * dw as i64 + xs as i64;
                if !e.occluded {
                    // Nothing can be in front: no reads of the depth plane.
                    let out = &mut px[(base + c0 as i64) as usize..(base + c1 as i64) as usize];
                    for (o, &p) in out.iter_mut().zip(&row[c0..c1]) {
                        if p != 0 {
                            *o = tp[p as usize];
                        }
                    }
                    continue;
                }
                for c in c0..c1 {
                    let p = row[c];
                    if p == 0 {
                        continue;
                    }
                    let i = (base + c as i64) as usize;
                    if depth[i] > e.key && (stat[i] >> 24) as u8 != crate::framebuffer::TAG_PROP {
                        continue;
                    }
                    px[i] = tp[p as usize];
                }
            }
        }
        // The leaves in the air, 2x2 px.
        for l in &self.leaves {
            let (x0, y0) = l.last;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (x, y) = (x0 + dx, y0 + dy);
                if x < clip.x0 || x >= clip.x1 || y < clip.y0 || y >= clip.y1 {
                    continue;
                }
                let i = y as usize * dw + x as usize;
                if depth[i] > l.key && (stat[i] >> 24) as u8 != crate::framebuffer::TAG_PROP {
                    continue;
                }
                let h = ((l.col ^ (l.col >> 11) ^ (l.col >> 19)) & 2047) as usize;
                px[i] = if src[h] == l.col {
                    out[h]
                } else {
                    let o = night(l.col, lut, bulbs);
                    src[h] = l.col;
                    out[h] = o;
                    o
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fsin_is_a_sine() {
        for k in 0..200 {
            let x = k as f32 * 0.137 - 3.0;
            assert!((fsin(x) - x.sin()).abs() < 0.06, "x={x} {} {}", fsin(x), x.sin());
        }
    }

    #[test]
    fn profile_poses_are_monotone_and_foot_is_planted() {
        let p = Profile::build(Class::Broad, 40, 48);
        assert_eq!(p.w[47], 0.0, "below the anchor nothing moves");
        assert_eq!(p.w[40], 0.0, "the foot is planted");
        assert!((p.w[0] - 1.0).abs() < 1e-5, "the tip carries the whole lean");
        assert!(p.w.windows(2).take(30).all(|w| w[0] >= w[1]), "bends, not slides");
        assert_eq!(p.shift(0, 0), 0);
        assert_eq!(p.shift(0, 4), 2, "two px of lean move the tip two px");
        assert_eq!(p.shift(0, -4), -2);
        assert_eq!(p.shift(45, 8), 0, "the foot stays planted");
        assert!(p.shift(10, 8) <= p.shift(0, 8), "the shift grows towards the tip");
        assert!(p.diff(0, 0).is_none());
        assert!(p.diff(0, 8).is_some());
    }
}
