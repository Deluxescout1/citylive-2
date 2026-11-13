//! The tile grid and the queries the renderer and sim need.

use crate::MAX_LEVEL;
use crate::biome::Biome;
use crate::types::{Dir, Edge, Kind, Prop, SlopeKind, Tile, WaterLayout};

/// A river: the centre-line it follows from one map edge to another, plus the
/// water surface level at each point. Levels never increase along the path.
#[derive(Clone, Debug)]
pub struct River {
    pub path: Vec<(i32, i32)>,
    pub surface: Vec<u8>,
    /// Nominal water width in tiles (2-4).
    pub width: f32,
    pub entry: (i32, i32),
    pub exit: (i32, i32),
    pub entry_edge: Edge,
    pub exit_edge: Edge,
}

/// The sea along one side of the screen: one flat surface, a beach shelf and
/// dunes. `side` is -1 for the screen's left edge, +1 for the right; the water
/// lies beyond `shore_p` on that side, measured in `p = x - y`.
#[derive(Clone, Debug)]
pub struct Ocean {
    pub side: i8,
    pub shore_p: f32,
    pub level: u8,
    pub tiles: usize,
    /// B46e: a lagoon's barrier reef stands this far (in `p`) off the shore;
    /// 0 for every other sea. The water inside it is the lagoon.
    pub reef: f32,
}

/// How worn a tile is (D45): the thresholds are the tunables the brief asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum WearStage {
    /// Fresh grass.
    None,
    /// Walked over: flattened, lighter grass.
    Trampled,
    /// A thin worn dirt line along the direction of travel.
    Footpath,
    /// Enough traffic for the sim to lay a dirt lane (D34 graded).
    CartTrack,
}

impl WearStage {
    pub const TRAMPLED_AT: u8 = 48;
    pub const FOOTPATH_AT: u8 = 110;
    pub const CART_TRACK_AT: u8 = 200;

    #[inline]
    pub fn of(wear: u8) -> WearStage {
        if wear >= Self::CART_TRACK_AT {
            WearStage::CartTrack
        } else if wear >= Self::FOOTPATH_AT {
            WearStage::Footpath
        } else if wear >= Self::TRAMPLED_AT {
            WearStage::Trampled
        } else {
            WearStage::None
        }
    }
}

/// What a water tile belongs to (B21): the renderer animates each class
/// differently and the sim only bridges rivers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WaterKind {
    #[default]
    None,
    River,
    Lake,
    Pond,
    Sea,
    /// Shallow water standing over land a flood took (B50, B99c round 2). It is
    /// the sea's own water once it has come inland: pale, still, and carrying
    /// light at dusk and at night that the deep sea does not, so the whole
    /// lagoon — the sea included — reads as one body of water.
    Lagoon,
}

impl WaterKind {
    #[inline]
    pub fn from_code(c: u8) -> Self {
        match c {
            1 => WaterKind::River,
            2 => WaterKind::Lake,
            3 => WaterKind::Pond,
            4 => WaterKind::Sea,
            5 => WaterKind::Lagoon,
            _ => WaterKind::None,
        }
    }

    #[inline]
    pub fn code(self) -> u8 {
        match self {
            WaterKind::None => 0,
            WaterKind::River => 1,
            WaterKind::Lake => 2,
            WaterKind::Pond => 3,
            WaterKind::Sea => 4,
            WaterKind::Lagoon => 5,
        }
    }
}

/// A lake in a low basin: one flat surface for the whole body.
#[derive(Clone, Debug)]
pub struct Lake {
    pub centre: (i32, i32),
    pub level: u8,
    pub tiles: usize,
}

/// The world. Row-major, `w * h` tiles.
#[derive(Clone, Debug)]
pub struct TileMap {
    pub w: i32,
    pub h: i32,
    pub tiles: Vec<Tile>,
    pub seed: u64,
    pub river: Option<River>,
    pub lake: Option<Lake>,
    /// Small ponds (WaterLayout::Ponds).
    pub ponds: Vec<Lake>,
    /// The sea along one map side (WaterLayout::Coast / Delta).
    pub ocean: Option<Ocean>,
    /// Which water layout the seed asked for (D44).
    pub layout: WaterLayout,
    /// Per-tile water class (0 none, 1 river, 2 lake, 3 pond, 4 sea), so the
    /// renderer and the sim do not have to re-derive it from geometry.
    pub water_class: Vec<u8>,
    /// Per-tile wear from villagers walking (D45): 0 fresh ground, 255 a heavily
    /// used track. The renderer turns it into trampled grass, a footpath or a
    /// cart track; the sim turns a cart track into a dirt lane.
    pub wear: Vec<u8>,
    /// Which land this world is (D66).
    pub biome: Biome,
    /// The land that blends in at one map edge, if a world was given one.
    pub neighbour: Option<Biome>,
    /// The map edge the neighbour comes in from.
    pub blend_edge: Edge,
    /// B50: the morph into the next land. Empty when no morph is running;
    /// otherwise one byte per tile, 1 where the tile has already turned into
    /// the `neighbour` land. It replaces the edge tongue while it runs.
    pub morph: Vec<u8>,
    /// B46d: the landforms (sprites) this world carries, decided when the world is
    /// generated or a save is loaded (a save whose city already stands where a
    /// massif would be keeps that massif as plain terrain).
    pub landforms: Vec<crate::massif::Massif>,
    /// B46d: massifs that got no sprite; the renderer smooths them.
    pub hills: Vec<crate::massif::Massif>,
    /// B70: the drawing layer that smooths open ground. Empty (everything flat
    /// and unlit) unless the renderer fills it in; the simulation never reads it.
    pub soft: SoftGround,
}

/// B70: per-tile-corner data for smooth open ground, kept beside the tiles so
/// the draw code can read it without a second parameter. All vectors are
/// either empty (feature off) or one entry per tile.
#[derive(Clone, Debug, Default)]
pub struct SoftGround {
    /// Vertical offset of each tile corner from its integer level, in pixels
    /// (corner order as `Tile::corner`). Zero on anything built, stepped or wet.
    pub dz: Vec<[i8; 4]>,
    /// The light index value at each corner (see `render::softground`); a tile
    /// is "soft" when its first entry is not NaN.
    pub lit: Vec<[f32; 4]>,
    /// The per-tile colour jitter (a function of the seed alone), cached.
    pub jit: Vec<f32>,
}

impl SoftGround {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dz.is_empty()
    }
}

impl TileMap {
    /// B70: the pixel offsets of a tile's corners (zero when the layer is off).
    #[inline]
    pub fn soft_dz(&self, x: i32, y: i32) -> [i8; 4] {
        if self.soft.dz.is_empty() || !self.in_bounds(x, y) {
            return [0; 4];
        }
        self.soft.dz[self.index(x, y)]
    }

    /// B70: the smooth light values at a tile's corners, `None` for a tile that
    /// is not smoothed.
    #[inline]
    pub fn soft_lit(&self, x: i32, y: i32) -> Option<[f32; 4]> {
        if self.soft.lit.is_empty() || !self.in_bounds(x, y) {
            return None;
        }
        let l = self.soft.lit[self.index(x, y)];
        if l[0].is_nan() { None } else { Some(l) }
    }
}

impl TileMap {
    pub fn fill(w: i32, h: i32, tile: Tile, seed: u64) -> Self {
        Self {
            w,
            h,
            tiles: vec![tile; (w * h) as usize],
            seed,
            river: None,
            lake: None,
            ponds: Vec::new(),
            ocean: None,
            layout: WaterLayout::RiverOnly,
            water_class: vec![0; (w * h) as usize],
            wear: vec![0; (w * h) as usize],
            biome: Biome::Temperate,
            neighbour: None,
            blend_edge: Edge::Left,
            morph: Vec::new(),
            landforms: Vec::new(),
            hills: Vec::new(),
            soft: SoftGround::default(),
        }
    }

    /// Which land this world is (D66).
    #[inline]
    pub fn biome(&self) -> Biome {
        self.biome
    }

    /// How much of the neighbouring land is at `(x, y)` (0..1). Zero everywhere
    /// when the world has no neighbour.
    #[inline]
    pub fn blend_at(&self, x: i32, y: i32) -> f32 {
        match self.neighbour {
            None => 0.0,
            Some(_) if !self.morph.is_empty() => {
                if self.in_bounds(x, y) {
                    self.morph[(y * self.w + x) as usize] as f32
                } else {
                    0.0
                }
            }
            Some(_) => crate::biome::blend_weight(self.seed, self.blend_edge, self.w, self.h, x, y),
        }
    }

    /// The land that owns tile `(x, y)` for species and materials: the host, or
    /// the neighbour where the blend tongue is strong (a palm at the jungle end
    /// of a desert world, say).
    #[inline]
    pub fn biome_at(&self, x: i32, y: i32) -> Biome {
        match self.neighbour {
            Some(n) if crate::biome::has_neighbour(self.blend_at(x, y), x, y, self.seed) => n,
            _ => self.biome,
        }
    }

    /// B50: start (or continue) a morph into `next`; every tile still belongs
    /// to the old land until [`Self::morph_tile`] turns it.
    pub fn begin_morph(&mut self, next: Biome) {
        self.neighbour = Some(next);
        // The old land's massif sprites go with it (the next land brings its own).
        self.landforms.clear();
        self.hills.clear();
        if self.morph.len() != (self.w * self.h) as usize {
            self.morph = vec![0; (self.w * self.h) as usize];
        }
    }

    /// B50: mark tile `(x, y)` as turned into the next land.
    pub fn morph_tile(&mut self, x: i32, y: i32) {
        if self.in_bounds(x, y) && !self.morph.is_empty() {
            let i = (y * self.w + x) as usize;
            self.morph[i] = 1;
        }
    }

    /// B50: has tile `(x, y)` already turned into the next land?
    #[inline]
    pub fn morphed(&self, x: i32, y: i32) -> bool {
        !self.morph.is_empty() && self.in_bounds(x, y) && self.morph[(y * self.w + x) as usize] != 0
    }

    /// B50: the morph is over: the next land is the land.
    pub fn end_morph(&mut self) {
        if let Some(n) = self.neighbour {
            self.biome = n;
        }
        self.neighbour = None;
        self.morph = Vec::new();
    }

    #[inline]
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.w && y < self.h
    }

    #[inline]
    fn index(&self, x: i32, y: i32) -> usize {
        (y * self.w + x) as usize
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<Tile> {
        if self.in_bounds(x, y) {
            Some(self.tiles[self.index(x, y)])
        } else {
            None
        }
    }

    /// Tile at `(x, y)` with coordinates clamped to the map. The renderer uses
    /// this for neighbour lookups: at the world edge it sees the edge tile.
    #[inline]
    pub fn at(&self, x: i32, y: i32) -> Tile {
        let cx = x.clamp(0, self.w - 1);
        let cy = y.clamp(0, self.h - 1);
        self.tiles[self.index(cx, cy)]
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, tile: Tile) {
        if self.in_bounds(x, y) {
            let i = self.index(x, y);
            self.tiles[i] = tile;
        }
    }

    #[inline]
    pub fn with_tile_mut(&mut self, x: i32, y: i32, f: impl FnOnce(&mut Tile)) {
        if self.in_bounds(x, y) {
            let i = self.index(x, y);
            f(&mut self.tiles[i]);
        }
    }

    /// The four corner levels of a tile, in the order
    /// `[(x,y), (x+1,y), (x+1,y+1), (x,y+1)]`.
    #[inline]
    pub fn corner_heights(&self, x: i32, y: i32) -> [u8; 4] {
        self.at(x, y).corner
    }

    /// Walk wear at a tile (D45).
    #[inline]
    pub fn wear(&self, x: i32, y: i32) -> u8 {
        if !self.in_bounds(x, y) {
            return 0;
        }
        self.wear[self.index(x, y)]
    }

    /// Add wear, saturating. Returns true when the tile's *stage* changed.
    pub fn add_wear(&mut self, x: i32, y: i32, amount: u16) -> bool {
        if !self.in_bounds(x, y) {
            return false;
        }
        let i = self.index(x, y);
        let before = WearStage::of(self.wear[i]);
        self.wear[i] = (self.wear[i] as u16).saturating_add(amount).min(255) as u8;
        WearStage::of(self.wear[i]) != before
    }

    /// Wear back-off: every worn tile loses a little, so unused paths fade.
    pub fn decay_wear(&mut self, amount: u8) {
        for w in &mut self.wear {
            *w = w.saturating_sub(amount);
        }
    }

    /// What this tile's water belongs to (B21).
    #[inline]
    pub fn water_kind(&self, x: i32, y: i32) -> WaterKind {
        if !self.in_bounds(x, y) {
            return WaterKind::None;
        }
        WaterKind::from_code(self.water_class[self.index(x, y)])
    }

    /// Set the water class grid (generation only).
    pub fn set_water_classes(&mut self, class: Vec<u8>) {
        debug_assert_eq!(class.len(), self.tiles.len());
        self.water_class = class;
    }

    #[inline]
    pub fn is_flat(&self, x: i32, y: i32) -> bool {
        self.at(x, y).is_flat()
    }

    #[inline]
    pub fn slope_kind(&self, x: i32, y: i32) -> SlopeKind {
        self.at(x, y).slope_kind()
    }

    /// The two corner levels of tile `(x, y)` on the side facing `dir`.
    #[inline]
    pub fn edge_levels(&self, x: i32, y: i32, dir: Dir) -> [u8; 2] {
        let c = self.at(x, y).corner;
        let [a, b] = dir.corners();
        [c[a], c[b]]
    }

    /// The two height pairs that meet along the edge between tile `(x, y)` and
    /// its neighbour in `dir`, as `(mine, theirs)`, in matching world order.
    pub fn facing_pairs(&self, x: i32, y: i32, dir: Dir) -> [(u8, u8); 2] {
        let (dx, dy) = dir.delta();
        let mine = self.edge_levels(x, y, dir);
        let theirs = self.edge_levels(x + dx, y + dy, dir.opposite());
        // Corners are [(x,y), (x+1,y), (x+1,y+1), (x,y+1)]. Across an x edge both
        // tiles list the shared vertices in the same (y) order; across a y edge
        // the neighbour lists them the other way round.
        match dir {
            Dir::PosX | Dir::NegX => [(mine[0], theirs[0]), (mine[1], theirs[1])],
            Dir::PosY | Dir::NegY => [(mine[0], theirs[1]), (mine[1], theirs[0])],
        }
    }

    /// Height step across one edge (0 = the two tiles meet exactly).
    pub fn edge_step(&self, x: i32, y: i32, dir: Dir) -> i32 {
        self.facing_pairs(x, y, dir)
            .iter()
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap_or(0)
    }

    /// True where this tile meets a neighbour at least two levels away: the
    /// explicit cliff edges of B3/Q1.
    pub fn is_cliff(&self, x: i32, y: i32) -> bool {
        Dir::ALL.iter().any(|d| self.edge_step(x, y, *d) >= 2)
    }

    /// Land the sim may build on: not water, and flat or a single straight climb
    /// (so a road can follow it).
    pub fn buildable(&self, x: i32, y: i32) -> bool {
        if !self.in_bounds(x, y) {
            return false;
        }
        let t = self.at(x, y);
        if t.is_water() {
            return false;
        }
        matches!(t.slope_kind(), SlopeKind::Flat | SlopeKind::Edge(_))
    }

    /// Flat and buildable: where a building can stand.
    pub fn flat_buildable(&self, x: i32, y: i32) -> bool {
        let t = self.at(x, y);
        !t.is_water() && t.is_flat() && t.kind != crate::Kind::Rock
    }

    /// Interpolated surface level at a continuous tile position, following the
    /// same triangulation the renderer draws.
    pub fn surface_level_at(&self, fx: f32, fy: f32) -> f32 {
        let tx = fx.floor();
        let ty = fy.floor();
        let u = fx - tx;
        let v = fy - ty;
        let t = self.at(tx as i32, ty as i32);
        let c = t.corner.map(|h| h as f32);
        if t.diagonal() == 0 {
            if u >= v {
                c[0] + (c[1] - c[0]) * u + (c[2] - c[1]) * v
            } else {
                c[0] + (c[3] - c[0]) * v + (c[2] - c[3]) * u
            }
        } else if u + v <= 1.0 {
            c[0] + (c[1] - c[0]) * u + (c[3] - c[0]) * v
        } else {
            c[2] + (c[3] - c[2]) * (1.0 - u) + (c[1] - c[2]) * (1.0 - v)
        }
    }

    /// The height something that *moves* stands at (B67).
    ///
    /// [`Self::surface_level_at`] is exact, and where two flat tiles differ by a
    /// level (a road that climbs a hill in steps, 91 edges in one city) it jumps
    /// a whole level — 12 px — in a single frame: a car, a bus or a walker
    /// crossing the seam popped up the hill. The ground is still stepped, but
    /// the thing standing on it now eases across the seam over about a third of a
    /// tile either side: at the edge itself it stands at the mean of the two
    /// sides, and away from every edge it is exactly the surface. Continuous
    /// across the seam, identical away from it, and nothing for water.
    pub fn ground_level_at(&self, fx: f32, fy: f32) -> f32 {
        const R: f32 = 0.34;
        let (tx, ty) = (fx.floor(), fy.floor());
        let (u, v) = (fx - tx, fy - ty);
        let t = self.at(tx as i32, ty as i32);
        let h0 = self.surface_level_at(fx, fy);
        if t.is_water() || (u >= R && u <= 1.0 - R && v >= R && v <= 1.0 - R) {
            return h0;
        }
        const E: f32 = 1e-3;
        let mut d = 0.0;
        // For each edge we are close to: how far the surface on the other side
        // of it, at this point of the edge, is from ours, weighted half at the
        // edge and nothing at distance R.
        let mut edge = |dist: f32, own: (f32, f32), other: (f32, f32)| {
            if dist < R {
                let w = 0.5 * (1.0 - dist / R);
                d += w * (self.surface_level_at(other.0, other.1) - self.surface_level_at(own.0, own.1));
            }
        };
        edge(u, (tx + E, fy), (tx - E, fy));
        edge(1.0 - u, (tx + 1.0 - E, fy), (tx + 1.0 + E, fy));
        edge(v, (fx, ty + E), (fx, ty - E));
        edge(1.0 - v, (fx, ty + 1.0 - E), (fx, ty + 1.0 + E));
        h0 + d
    }

    /// [`Self::ground_level_at`] in screen pixels, rounded: the one answer every
    /// layer uses for the height of a moving thing, so the sprite, its shadow and
    /// its damage rectangle agree.
    #[inline]
    pub fn ground_px(&self, fx: f32, fy: f32) -> i32 {
        (self.ground_level_at(fx, fy) * crate::iso_consts::HEIGHT_UNIT as f32).round() as i32
    }

    pub fn count_trees(&self) -> usize {
        self.tiles.iter().filter(|t| t.prop.is_tree()).count()
    }

    pub fn count_kind(&self, kind: Kind) -> usize {
        self.tiles.iter().filter(|t| t.kind == kind).count()
    }

    pub fn count_props(&self) -> usize {
        self.tiles.iter().filter(|t| t.prop.is_something()).count()
    }

    /// How many tiles sit at each level, by the tile's highest corner.
    pub fn height_stats(&self) -> [usize; (MAX_LEVEL + 1) as usize] {
        let mut out = [0usize; (MAX_LEVEL + 1) as usize];
        for t in &self.tiles {
            out[(t.max_level() as usize).min(MAX_LEVEL as usize)] += 1;
        }
        out
    }

    /// Tiles whose highest corner reaches `min` or above.
    pub fn count_at_least(&self, min: u8) -> usize {
        self.tiles.iter().filter(|t| t.max_level() >= min).count()
    }

    /// Tiles with a neighbour two or more levels away.
    pub fn cliff_tiles(&self) -> usize {
        let mut n = 0;
        for y in 0..self.h {
            for x in 0..self.w {
                if self.is_cliff(x, y) {
                    n += 1;
                }
            }
        }
        n
    }

    /// Arbitrary but stable per-tile hash in `[0, 1)`, for texture detail.
    #[inline]
    pub fn tile_hash(&self, x: i32, y: i32) -> f32 {
        crate::rng::hash2(x, y, self.seed ^ 0x5EED_1234)
    }

    /// Smooth large-scale noise (about a 6-tile wavelength) for colour
    /// variation, so flat ground is not flat paint.
    #[inline]
    pub fn patch_noise(&self, x: i32, y: i32) -> f32 {
        crate::rng::fbm(x as f32 / 6.0, y as f32 / 6.0, self.seed ^ 0xBEEF, 2)
    }

    #[inline]
    pub fn has_prop(&self, x: i32, y: i32) -> bool {
        !matches!(self.at(x, y).prop, Prop::None)
    }

    /// `true` when this tile is a rapid: water whose surface steps down.
    pub fn is_rapid(&self, x: i32, y: i32) -> bool {
        let t = self.at(x, y);
        if !t.is_water() {
            return false;
        }
        let l = t.corner[0];
        Dir::ALL.iter().any(|d| {
            let (dx, dy) = d.delta();
            let n = self.at(x + dx, y + dy);
            n.is_water() && n.corner[0] != l
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamped_access_returns_edge_tiles() {
        let m = TileMap::fill(4, 3, Tile::flat(2), 1);
        assert_eq!(m.at(-5, -5), Tile::flat(2));
        assert_eq!(m.at(99, 99), Tile::flat(2));
        assert!(m.get(4, 0).is_none());
    }

    #[test]
    fn surface_follows_the_triangles() {
        let mut t = Tile::flat(2);
        t.corner = [2, 4, 4, 2]; // rises toward +x
        let m = TileMap::fill(4, 4, t, 1);
        // Along the u axis the surface climbs from 2 to 4.
        assert!((m.surface_level_at(0.5, 0.5) - 3.0).abs() < 1e-4);
        assert!((m.surface_level_at(0.0, 0.5) - 2.0).abs() < 1e-4);
        assert!((m.surface_level_at(0.999, 0.5) - 4.0).abs() < 5e-3);
    }

    #[test]
    fn flat_tile_is_flat_everywhere() {
        let m = TileMap::fill(3, 3, Tile::flat(5), 1);
        for (u, v) in [(0.0, 0.0), (0.5, 0.5), (0.99, 0.01), (0.2, 0.8)] {
            assert_eq!(m.surface_level_at(1.0 + u, 1.0 + v), 5.0);
        }
    }

    #[test]
    fn a_continuous_slope_is_no_cliff_across_any_edge() {
        // One surface z = 2 + x + y (in levels), cut into tiles: no step anywhere.
        let mut m = TileMap::fill(4, 4, Tile::flat(2), 1);
        for y in 0..4 {
            for x in 0..4 {
                let z = |cx: i32, cy: i32| (2 + cx + cy) as u8;
                m.with_tile_mut(x, y, |t| t.corner = [z(x, y), z(x + 1, y), z(x + 1, y + 1), z(x, y + 1)]);
            }
        }
        for d in Dir::ALL {
            assert_eq!(m.edge_step(1, 1, d), 0, "{d:?}");
        }
        assert!(!m.is_cliff(1, 1));
    }

    #[test]
    fn cliffs_are_detected() {
        let mut m = TileMap::fill(4, 4, Tile::flat(3), 1);
        // Raise tile (1,1) by 3 levels without touching its neighbours.
        m.with_tile_mut(1, 1, |t| t.corner = [6, 6, 6, 6]);
        assert!(m.is_cliff(1, 1));
        assert!(m.is_cliff(2, 1)); // the neighbour sees the drop too
        assert!(!m.is_cliff(3, 3));
    }
}
