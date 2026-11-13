//! Tile value types: corner heights, materials, props.
//!
//! Terrain is a **corner-height field** (B3/Q1): a tile's shape comes from the
//! levels of its four corners, on a vertex grid with levels `0..=MAX_LEVEL`.
//! Each tile top is two triangles; shading per triangle is what makes slopes
//! read in a 2:1 iso projection.

/// Highest terrain level. One level is `HEIGHT_UNIT` (8) screen pixels, so the
/// full range is 96 px of relief.
pub const MAX_LEVEL: u8 = 18;

/// What a tile's top surface is made of. Cliff *faces* use their own materials
/// (see `Kind::Earth`/`Kind::Rock`), independent of the top.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum Kind {
    /// Lush green, low ground.
    #[default]
    Grass,
    /// Mid-height meadow: slightly paler, warmer grass.
    Meadow,
    /// Upland grass: darker, cooler green on the high ground.
    Upland,
    /// Drier, ochre grass: small patches on sunny mid slopes only.
    DryGrass,
    /// Loose rocky ground: steep upper slopes.
    Scree,
    /// Bare rock: peaks and cliff tops.
    Rock,
    /// Beach / river bank.
    Sand,
    Road,
    Sidewalk,
    Water,
    /// Theatre 0: unsealed lane (the settlement era).
    Dirt,
    /// Theatre 0: gravel lane, the step between dirt and asphalt.
    Gravel,
    /// Theatre 1: asphalt.
    Paved,
    /// Planked deck over water.
    Bridge,
    /// Ploughed field (farms on the outskirts).
    Field,
}

impl Kind {
    #[inline]
    pub fn is_water(self) -> bool {
        matches!(self, Kind::Water)
    }

    /// Anything the city has laid down: lane, pavement, bridge deck, field.
    #[inline]
    pub fn is_worked(self) -> bool {
        matches!(
            self,
            Kind::Road | Kind::Sidewalk | Kind::Dirt | Kind::Gravel | Kind::Paved | Kind::Bridge | Kind::Field
        )
    }

    /// A built surface: a lane, a pavement or a bridge deck. The renderer keeps
    /// these opaque into the distance instead of hazing them (B42b).
    #[inline]
    pub fn is_work_surface(self) -> bool {
        matches!(
            self,
            Kind::Road | Kind::Sidewalk | Kind::Dirt | Kind::Gravel | Kind::Paved | Kind::Bridge
        )
    }

    /// Surfaces a vehicle or a person can use.
    #[inline]
    pub fn is_lane(self) -> bool {
        matches!(self, Kind::Dirt | Kind::Gravel | Kind::Paved | Kind::Road | Kind::Bridge)
    }

    /// Surfaces an actor may walk on.
    #[inline]
    pub fn is_walkable(self) -> bool {
        !self.is_water()
    }

    /// Materials for the vertical faces of a cliff / step.
    #[inline]
    pub fn is_rocky(self) -> bool {
        matches!(self, Kind::Scree | Kind::Rock)
    }
}

/// Which way a slope or a cliff edge faces, in tile space. `PosX` is down-right
/// on screen, `PosY` down-left.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Dir {
    PosX,
    NegX,
    PosY,
    NegY,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::PosX, Dir::NegX, Dir::PosY, Dir::NegY];

    #[inline]
    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::PosX => (1, 0),
            Dir::NegX => (-1, 0),
            Dir::PosY => (0, 1),
            Dir::NegY => (0, -1),
        }
    }

    #[inline]
    pub fn opposite(self) -> Dir {
        match self {
            Dir::PosX => Dir::NegX,
            Dir::NegX => Dir::PosX,
            Dir::PosY => Dir::NegY,
            Dir::NegY => Dir::PosY,
        }
    }

    /// The two corner indices of a tile that lie on the edge facing `self`
    /// (corners are `[(x,y), (x+1,y), (x+1,y+1), (x,y+1)]`).
    #[inline]
    pub fn corners(self) -> [usize; 2] {
        match self {
            Dir::PosX => [1, 2], // the x+1 edge
            Dir::NegX => [0, 3],
            Dir::PosY => [2, 3], // the y+1 edge
            Dir::NegY => [0, 1],
        }
    }
}

/// One of the four map edges. The river enters and leaves through two of these.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];

    #[inline]
    pub fn contains(self, x: i32, y: i32, w: i32, h: i32) -> bool {
        match self {
            Edge::Left => x <= 0,
            Edge::Right => x >= w - 1,
            Edge::Top => y <= 0,
            Edge::Bottom => y >= h - 1,
        }
    }
}

/// Which water a world got (D44): picked from the seed, ~30 % coastal, ~40 % with
/// a lake, the rest river only; every world has at least one river.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum WaterLayout {
    /// One river crossing the map, nothing else.
    RiverOnly,
    /// River plus a lake in a low basin (the historical default).
    #[default]
    RiverLake,
    /// Sea along one map side: beach, dunes, rocks; the river runs into it.
    Coast,
    /// Coast plus a river delta: two or three mouths reaching the sea.
    Delta,
    /// Two to four small ponds scattered over the low ground.
    Ponds,
}

impl WaterLayout {
    /// Name for logs and the seed table.
    pub fn name(self) -> &'static str {
        match self {
            WaterLayout::RiverOnly => "river",
            WaterLayout::RiverLake => "lake",
            WaterLayout::Coast => "coast",
            WaterLayout::Delta => "delta",
            WaterLayout::Ponds => "ponds",
        }
    }

    /// True when the layout has a sea along one map edge.
    pub fn is_coastal(self) -> bool {
        matches!(self, WaterLayout::Coast | WaterLayout::Delta)
    }

    /// Relative weight out of 20 (D44 asks ~30 % coast, ~40 % lake).
    /// coast 6 + delta 1 = 7/20 = 35 % coastal, lake 8/20 = 40 %, ponds 2/20,
    /// river only 3/20.
    pub fn weight(self) -> u32 {
        match self {
            WaterLayout::RiverOnly => 3,
            WaterLayout::RiverLake => 8,
            WaterLayout::Coast => 6,
            WaterLayout::Delta => 1,
            WaterLayout::Ponds => 2,
        }
    }

    /// Every layout, for the generator's weighted pick and the tests.
    pub const ALL: [WaterLayout; 5] = [
        WaterLayout::RiverOnly,
        WaterLayout::RiverLake,
        WaterLayout::Coast,
        WaterLayout::Delta,
        WaterLayout::Ponds,
    ];
}

/// Tree species: broadleaf low down, conifers up high (Q5), and the desert's
/// ever-green succulent (D66).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum TreeKind {
    Deciduous,
    Conifer,
    /// A cactus: no crown, no leaves, no winter.
    Succulent,
}

/// The species a tree is (D64): the temperate mix the world is planted with,
/// plus the park accents that only get planted in a city park.
///
/// `variant` (0..=255) still varies each individual within its species, so no
/// two neighbours look stamped.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum TreeSpecies {
    // Broadleaf: the wild wood and the street tree.
    Oak,
    Maple,
    Birch,
    Beech,
    Elm,
    Willow,
    Poplar,
    // Orchard and blossom.
    Apple,
    Cherry,
    // Conifer: the hillside and the treeline.
    Pine,
    Spruce,
    Fir,
    Cedar,
    // Park accents (D64: city parks only).
    Palm,
    Cypress,
    JapaneseMaple,
    // Desert (D66): the saguaro, which is a tree in the botanical sense only,
    // and the Joshua tree, a yucca that reads as a bare, spiky silhouette.
    Cactus,
    Joshua,
    // Tropics (B46b): a second palm (the fan palm), a flowering tree and the
    // mangrove that stands in the shallows.
    FanPalm,
    Flamboyant,
    Mangrove,
    // B46e: the savanna's flat-topped acacia and the baobab, the swamp's
    // bald cypress and the Mediterranean olive.
    Acacia,
    Baobab,
    BaldCypress,
    Olive,
}

impl TreeSpecies {
    /// Every species, in `id()` order.
    pub const ALL: [TreeSpecies; 25] = [
        TreeSpecies::Oak,
        TreeSpecies::Maple,
        TreeSpecies::Birch,
        TreeSpecies::Beech,
        TreeSpecies::Elm,
        TreeSpecies::Willow,
        TreeSpecies::Poplar,
        TreeSpecies::Apple,
        TreeSpecies::Cherry,
        TreeSpecies::Pine,
        TreeSpecies::Spruce,
        TreeSpecies::Fir,
        TreeSpecies::Cedar,
        TreeSpecies::Palm,
        TreeSpecies::Cypress,
        TreeSpecies::JapaneseMaple,
        TreeSpecies::Cactus,
        TreeSpecies::Joshua,
        TreeSpecies::FanPalm,
        TreeSpecies::Flamboyant,
        TreeSpecies::Mangrove,
        TreeSpecies::Acacia,
        TreeSpecies::Baobab,
        TreeSpecies::BaldCypress,
        TreeSpecies::Olive,
    ];

    /// The broadleaf / conifer split: conifers keep their needles all winter.
    pub fn kind(self) -> TreeKind {
        match self {
            TreeSpecies::Pine | TreeSpecies::Spruce | TreeSpecies::Fir | TreeSpecies::Cedar => {
                TreeKind::Conifer
            }
            TreeSpecies::Cactus | TreeSpecies::Joshua | TreeSpecies::Acacia | TreeSpecies::Baobab => {
                TreeKind::Succulent
            }
            TreeSpecies::FanPalm => TreeKind::Conifer, // evergreen, no turn
            _ => TreeKind::Deciduous,
        }
    }

    /// A park accent: only the city plants these (a palm in a wild wood would
    /// look like a mistake).
    pub fn is_park_accent(self) -> bool {
        matches!(
            self,
            TreeSpecies::Palm | TreeSpecies::Cypress | TreeSpecies::JapaneseMaple
        )
    }

    /// A palm: drawn by the palm routine, never by the wood routine.
    pub fn is_palm(self) -> bool {
        matches!(self, TreeSpecies::Palm | TreeSpecies::FanPalm)
    }

    pub fn id(self) -> u8 {
        match self {
            TreeSpecies::Oak => 0,
            TreeSpecies::Maple => 1,
            TreeSpecies::Birch => 2,
            TreeSpecies::Beech => 3,
            TreeSpecies::Elm => 4,
            TreeSpecies::Willow => 5,
            TreeSpecies::Poplar => 6,
            TreeSpecies::Apple => 7,
            TreeSpecies::Cherry => 8,
            TreeSpecies::Pine => 9,
            TreeSpecies::Spruce => 10,
            TreeSpecies::Fir => 11,
            TreeSpecies::Cedar => 12,
            TreeSpecies::Palm => 13,
            TreeSpecies::Cypress => 14,
            TreeSpecies::JapaneseMaple => 15,
            TreeSpecies::Cactus => 16,
            TreeSpecies::Joshua => 17,
            TreeSpecies::FanPalm => 18,
            TreeSpecies::Flamboyant => 19,
            TreeSpecies::Mangrove => 20,
            TreeSpecies::Acacia => 21,
            TreeSpecies::Baobab => 22,
            TreeSpecies::BaldCypress => 23,
            TreeSpecies::Olive => 24,
        }
    }

    pub fn from_id(id: u8) -> TreeSpecies {
        TreeSpecies::ALL[(id as usize) % TreeSpecies::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            TreeSpecies::Oak => "oak",
            TreeSpecies::Maple => "maple",
            TreeSpecies::Birch => "birch",
            TreeSpecies::Beech => "beech",
            TreeSpecies::Elm => "elm",
            TreeSpecies::Willow => "willow",
            TreeSpecies::Poplar => "poplar",
            TreeSpecies::Apple => "apple",
            TreeSpecies::Cherry => "cherry",
            TreeSpecies::Pine => "pine",
            TreeSpecies::Spruce => "spruce",
            TreeSpecies::Fir => "fir",
            TreeSpecies::Cedar => "cedar",
            TreeSpecies::Palm => "palm",
            TreeSpecies::Cypress => "cypress",
            TreeSpecies::JapaneseMaple => "japanese maple",
            TreeSpecies::Cactus => "cactus",
            TreeSpecies::Joshua => "joshua tree",
            TreeSpecies::FanPalm => "fan palm",
            TreeSpecies::Flamboyant => "flamboyant",
            TreeSpecies::Mangrove => "mangrove",
            TreeSpecies::Acacia => "acacia",
            TreeSpecies::Baobab => "baobab",
            TreeSpecies::BaldCypress => "bald cypress",
            TreeSpecies::Olive => "olive",
        }
    }
}

/// What stands on a tile. One prop per tile; canopies may overhang neighbours.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum Prop {
    #[default]
    None,
    Tree {
        species: TreeSpecies,
        /// 0 = small, 1 = large.
        size: u8,
        /// Per-individual variation (crown shape, hue, lean).
        variant: u8,
    },
    Bush(u8),
    Boulder(u8),
    /// Wildflowers / meadow patch (drawn as a few coloured pixels).
    Meadow(u8),
    /// Tall grass: a clump of blades, in the meadows and along verges.
    TallGrass(u8),
    /// Reeds and cattails on the water's edge.
    Reeds(u8),
    /// A fallen log, mossy on top.
    Log(u8),
    /// A scatter of small stones.
    Stone(u8),
}

impl Prop {
    #[inline]
    pub fn is_tree(self) -> bool {
        matches!(self, Prop::Tree { .. })
    }

    #[inline]
    pub fn is_something(self) -> bool {
        !matches!(self, Prop::None)
    }
}

/// Shape of a tile top, derived from its four corner levels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlopeKind {
    /// All four corners equal.
    Flat,
    /// Two adjacent corners high, two low: a straight climb. Rises toward `Dir`.
    Edge(Dir),
    /// One corner differs from the other three (index of the odd corner).
    Corner(u8),
    /// Two opposite corners high, two low.
    Saddle,
    /// More than one level of difference across a single tile.
    Steep,
}

// --- `Tile::aux` bit layout (owned by the map so sim and render agree) -------
/// Two bits per edge, in the order `[+x, +y, -x, -y]`: 0 = none,
/// 1 = graded embankment, 2 = retaining wall.
pub const AUX_EDGE_BITS: [u32; 4] = [0, 2, 4, 6];
/// Bridge deck.
pub const AUX_BRIDGE: u16 = 1 << 8;
/// A cleared tree left a stump.
pub const AUX_STUMP: u16 = 1 << 9;
/// B49: ground taken by a permanent technology structure (a cell tower, a
/// buried cable cabinet, a solar-farm panel, a wind turbine). No lot is staked
/// on a tile wearing this bit.
pub const AUX_TECH: u16 = 1 << 10;
/// B59: a lane of the highway. Traffic flows faster on it, and the street
/// layer paints it as a road that passes the city rather than one more street.
pub const AUX_HIGHWAY: u16 = 1 << 11;
/// B59b: a tile of an interchange ramp (a leg from the highway to the street):
/// the street layer paints it with its own edge lines, guardrail and signs.
pub const AUX_RAMP: u16 = 1 << 12;

/// B80: a `Kind::Field` tile keeps what is grown on it in the top three bits of
/// `aux` (bits 13..=15), so the sim writes it and a save carries it with the
/// tile (no format change: `aux` is already saved).
pub const AUX_CROP_SHIFT: u32 = 13;
pub const AUX_CROP_MASK: u16 = 0b111 << AUX_CROP_SHIFT;

/// What a field grows (B80, D84). `Pasture` is grass inside a fence: the
/// livestock live there. `Track` is the farm lane between fields (two ruts,
/// no crop). Which crops a land grows is decided by the sim from the biome.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum Crop {
    /// Wheat, barley: the golden field of the late summer.
    #[default]
    Cereal = 0,
    /// Maize: tall stalks in wide rows, harvested late.
    Maize = 1,
    /// Vegetables: short rows of cabbage and greens, in beds.
    Veg = 2,
    /// Fruit trees in rows on grass (apple, olive, date palm by land).
    Orchard = 3,
    /// Vines on trellis wires.
    Vineyard = 4,
    /// Fenced grass for the stock.
    Pasture = 5,
    /// Rice: a flooded bed with green shoots.
    Paddy = 6,
    /// The farm track and the yard.
    Track = 7,
}

impl Crop {
    pub const ALL: [Crop; 8] = [
        Crop::Cereal,
        Crop::Maize,
        Crop::Veg,
        Crop::Orchard,
        Crop::Vineyard,
        Crop::Pasture,
        Crop::Paddy,
        Crop::Track,
    ];

    pub fn from_bits(b: u16) -> Crop {
        Crop::ALL[(b & 7) as usize]
    }

    pub fn name(self) -> &'static str {
        match self {
            Crop::Cereal => "cereal",
            Crop::Maize => "maize",
            Crop::Veg => "vegetables",
            Crop::Orchard => "orchard",
            Crop::Vineyard => "vineyard",
            Crop::Pasture => "pasture",
            Crop::Paddy => "paddy",
            Crop::Track => "track",
        }
    }

    /// A crop that is ploughed, sown and harvested (follows the field stage).
    pub fn is_arable(self) -> bool {
        matches!(self, Crop::Cereal | Crop::Maize | Crop::Veg | Crop::Paddy)
    }
}

/// The crop a tile grows (the tile must be `Kind::Field`; anything else reads
/// as the default).
#[inline]
pub fn crop_of(t: &Tile) -> Crop {
    if t.kind == Kind::Field {
        Crop::from_bits(t.aux >> AUX_CROP_SHIFT)
    } else {
        Crop::Cereal
    }
}

#[inline]
pub fn set_crop(aux: &mut u16, c: Crop) {
    *aux = (*aux & !AUX_CROP_MASK) | ((c as u16) << AUX_CROP_SHIFT);
}

/// Which civil work, if any, stands on one edge of a tile.
#[inline]
pub fn edge_works(aux: u16, edge: usize) -> u8 {
    ((aux >> AUX_EDGE_BITS[edge]) & 0b11) as u8
}

#[inline]
pub fn set_edge_works(aux: &mut u16, edge: usize, value: u8) {
    let mask = 0b11u16 << AUX_EDGE_BITS[edge];
    *aux = (*aux & !mask) | ((value as u16 & 0b11) << AUX_EDGE_BITS[edge]);
}

/// One tile: four corner levels plus what is on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tile {
    /// Levels at `[(x,y), (x+1,y), (x+1,y+1), (x,y+1)]`.
    pub corner: [u8; 4],
    pub kind: Kind,
    pub prop: Prop,
    /// Free for the sim (lot id, construction stage, snow depth...).
    pub aux: u16,
}

impl Tile {
    pub const fn new(level: u8, kind: Kind) -> Self {
        Self {
            corner: [level; 4],
            kind,
            prop: Prop::None,
            aux: 0,
        }
    }

    pub const fn flat(level: u8) -> Self {
        Self::new(level, Kind::Grass)
    }

    #[inline]
    pub fn is_water(&self) -> bool {
        self.kind.is_water()
    }

    #[inline]
    pub fn min_level(&self) -> u8 {
        *self.corner.iter().min().unwrap()
    }

    #[inline]
    pub fn max_level(&self) -> u8 {
        *self.corner.iter().max().unwrap()
    }

    #[inline]
    pub fn is_flat(&self) -> bool {
        let c = self.corner;
        c[0] == c[1] && c[1] == c[2] && c[2] == c[3]
    }

    /// How much the surface climbs across this tile.
    #[inline]
    pub fn relief(&self) -> u8 {
        let r = self.max_level() - self.min_level();
        // B46d: bare rock is never graded flat ground, even on a plateau: the
        // top of a landform is a sprite's cap, not a building site.
        if self.kind == Kind::Rock {
            r.max(3)
        } else {
            r
        }
    }

    /// Which diagonal the two top triangles are split along:
    /// `0` joins corners 0-2, `1` joins corners 1-3.
    ///
    /// The split follows the *shorter* 3D diagonal, which is the one that does
    /// not fold the surface for a saddle.
    #[inline]
    pub fn diagonal(&self) -> u8 {
        let c = self.corner;
        let d02 = (c[0] as i32 - c[2] as i32).abs();
        let d13 = (c[1] as i32 - c[3] as i32).abs();
        if d02 <= d13 { 0 } else { 1 }
    }

    pub fn slope_kind(&self) -> SlopeKind {
        let c = self.corner;
        let lo = self.min_level();
        let hi = self.max_level();
        if hi == lo {
            return SlopeKind::Flat;
        }
        if hi - lo > 1 {
            return SlopeKind::Steep;
        }
        // Exactly one level of difference.
        let high: [bool; 4] = [c[0] == hi, c[1] == hi, c[2] == hi, c[3] == hi];
        let n = high.iter().filter(|h| **h).count();
        match n {
            1 => SlopeKind::Corner(high.iter().position(|h| *h).unwrap() as u8),
            3 => SlopeKind::Corner(high.iter().position(|h| !*h).unwrap() as u8),
            _ => {
                // Two high: adjacent gives an edge slope, diagonal a saddle.
                if high[0] && high[1] {
                    SlopeKind::Edge(Dir::NegY) // the y edge is high
                } else if high[2] && high[3] {
                    SlopeKind::Edge(Dir::PosY)
                } else if high[0] && high[3] {
                    SlopeKind::Edge(Dir::NegX)
                } else if high[1] && high[2] {
                    SlopeKind::Edge(Dir::PosX)
                } else {
                    SlopeKind::Saddle
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_are_consistent() {
        for d in Dir::ALL {
            assert_eq!(d.opposite().opposite(), d);
            let (dx, dy) = d.delta();
            let (ox, oy) = d.opposite().delta();
            assert_eq!((dx + ox, dy + oy), (0, 0));
        }
    }

    #[test]
    fn slope_kinds() {
        let flat = Tile {
            corner: [3, 3, 3, 3],
            ..Tile::flat(3)
        };
        assert_eq!(flat.slope_kind(), SlopeKind::Flat);

        // Rises toward +x: corners 1,2 high.
        let e = Tile {
            corner: [2, 3, 3, 2],
            ..Tile::flat(2)
        };
        assert_eq!(e.slope_kind(), SlopeKind::Edge(Dir::PosX));
        assert_eq!(e.diagonal(), 0); // equal corners 0-2

        // One high corner (index 2).
        let c = Tile {
            corner: [2, 2, 3, 2],
            ..Tile::flat(2)
        };
        assert_eq!(c.slope_kind(), SlopeKind::Corner(2));

        // Diagonal high pair: saddle.
        let s = Tile {
            corner: [3, 2, 3, 2],
            ..Tile::flat(2)
        };
        assert_eq!(s.slope_kind(), SlopeKind::Saddle);

        // Two levels across one tile: steep.
        let st = Tile {
            corner: [2, 4, 4, 2],
            ..Tile::flat(2)
        };
        assert_eq!(st.slope_kind(), SlopeKind::Steep);
    }

    #[test]
    fn diagonal_picks_the_short_one() {
        // 0-2 equal, 1-3 differ -> split 0-2.
        assert_eq!(
            Tile {
                corner: [2, 2, 2, 3],
                ..Tile::flat(2)
            }
            .diagonal(),
            0
        );
        // corner 0 high: the 1-3 diagonal is the flat one -> split 1-3.
        assert_eq!(
            Tile {
                corner: [3, 2, 2, 2],
                ..Tile::flat(2)
            }
            .diagonal(),
            1
        );
    }
}

/// What a park is (B58b item 4): parks used to be one design repeated — a lime
/// square with a round pond — six times a city. A lot's `style` now carries one
/// of these, so a district's greens read as a lawn, a playground, a pitch, a
/// paved square, allotments, or the big pond park.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParkKind {
    /// Mown lawn, cross paths, trees and a fountain: the classic square.
    Lawn,
    /// Swings, a slide and a sand pit, fenced.
    Playground,
    /// A marked sports pitch with goals.
    Sports,
    /// Paved hard standing with benches and planters.
    Plaza,
    /// Allotment beds, a shed and a path.
    Garden,
    /// The big park with a pond.
    Pond,
}

impl ParkKind {
    /// The index a lot's `style` stores.
    pub fn index(self) -> u8 {
        match self {
            ParkKind::Lawn => 0,
            ParkKind::Playground => 1,
            ParkKind::Sports => 2,
            ParkKind::Plaza => 3,
            ParkKind::Garden => 4,
            ParkKind::Pond => 5,
        }
    }

    /// Read a park's kind back off a lot. Out-of-range values (an old save, or a
    /// district style on a lot that is not a park) fall back to the lawn.
    pub fn of_style(s: u8) -> ParkKind {
        match s {
            1 => ParkKind::Playground,
            2 => ParkKind::Sports,
            3 => ParkKind::Plaza,
            4 => ParkKind::Garden,
            5 => ParkKind::Pond,
            _ => ParkKind::Lawn,
        }
    }

    /// Every kind, for the "never two alike next door" walk and the tests.
    pub const ALL: [ParkKind; 6] = [
        ParkKind::Lawn,
        ParkKind::Playground,
        ParkKind::Sports,
        ParkKind::Plaza,
        ParkKind::Garden,
        ParkKind::Pond,
    ];
}

/// The kind a park of this size and hash would like to be, before the
/// neighbourhood settles it: a pond in at most one in four parks, and only where
/// there is room for a body of water.
pub fn park_kind_for(w: i32, h: i32, hash: f32) -> ParkKind {
    let hash = hash.clamp(0.0, 0.999_99);
    if hash < 0.25 && w.min(h) >= 5 {
        return ParkKind::Pond;
    }
    // The other three quarters of the range pick one of the five dry kinds.
    let t = ((hash - 0.25) / 0.75).clamp(0.0, 0.999_99);
    let i = (t * 5.0) as usize;
    ParkKind::ALL[i.min(4)]
}

/// A park's pond, in tile units relative to the lot's top-left corner: centre
/// and radius (B26c's park art, shared so the people layer can keep out of the
/// water). Only the pond park gets one, and only with room for it; every other
/// kind is dry ground.
pub fn park_pond(w: i32, h: i32, kind: ParkKind) -> Option<(f32, f32, f32)> {
    if kind != ParkKind::Pond {
        return None;
    }
    let (wf, hf) = (w as f32, h as f32);
    if wf.min(hf) < 5.5 {
        return None;
    }
    let r = (wf.min(hf) * 0.28).clamp(0.7, 2.0);
    Some((wf * 0.5, hf * 0.5, r))
}

/// B64: any open water a park paints that is not a water tile: the pond of the
/// big pond park, the fountain basin of a lawn square, and the basin of the
/// civic square (`civic`, the plaza landmark, which paints its own fountain
/// whatever its style). Centre and radius in tile units from the lot's corner,
/// radius including the basin's stone rim.
pub fn park_water(w: i32, h: i32, civic: bool, kind: ParkKind) -> Option<(f32, f32, f32)> {
    let (wf, hf) = (w as f32, h as f32);
    if civic {
        let r = (wf.min(hf) * 0.18).clamp(0.5, 1.2) + 0.10;
        return Some((wf * 0.5, hf * 0.5, r));
    }
    match kind {
        ParkKind::Pond => park_pond(w, h, kind),
        // The lawn square's drinking fountain is a small basin in the middle.
        ParkKind::Lawn => Some((wf * 0.5, hf * 0.5, 0.35)),
        _ => None,
    }
}

/// Is the tile-space point in a park's water (pond or fountain basin), with a
/// margin so nobody stands on the rim?
pub fn in_park_water(lot_w: i32, lot_h: i32, civic: bool, kind: ParkKind, dx: f32, dy: f32) -> bool {
    match park_water(lot_w, lot_h, civic, kind) {
        Some((cx, cy, r)) => {
            let (ux, uy) = ((dx - cx) / (r + 0.25), (dy - cy) / (r + 0.25));
            ux * ux + uy * uy <= 1.0
        }
        None => false,
    }
}

/// Is the tile-space point inside a park's pond (with a margin for its bank)?
pub fn in_park_pond(lot_w: i32, lot_h: i32, kind: ParkKind, dx: f32, dy: f32) -> bool {
    match park_pond(lot_w, lot_h, kind) {
        Some((cx, cy, r)) => {
            let (ux, uy) = ((dx - cx) / (r + 0.25), (dy - cy) / (r + 0.25));
            ux * ux + uy * uy <= 1.0
        }
        None => false,
    }
}

#[cfg(test)]
mod park_water_tests {
    use super::*;

    /// B64: the lawn square's fountain and the civic square's basin are water
    /// people must stay out of; a pitch is dry.
    #[test]
    fn fountain_basins_are_water() {
        assert!(in_park_water(8, 8, false, ParkKind::Lawn, 4.0, 4.0));
        assert!(!in_park_water(8, 8, false, ParkKind::Lawn, 1.0, 1.0));
        assert!(in_park_water(8, 8, true, ParkKind::Lawn, 4.0, 4.5));
        assert!(!in_park_water(8, 8, false, ParkKind::Sports, 4.0, 4.0));
        assert!(in_park_water(8, 8, false, ParkKind::Pond, 4.0, 4.0));
    }
}

#[cfg(test)]
mod park_kind_tests {
    use super::*;

    /// B58b item 4: a park is one of five kinds, and a pond in at most one in
    /// four — the audit counted four to six identical round ponds per city.
    #[test]
    fn a_pond_is_at_most_one_park_in_four() {
        let mut ponds = 0;
        let mut total = 0;
        for i in 0..2000 {
            let h = (i as f32 + 0.5) / 2000.0;
            let k = park_kind_for(6, 6, h);
            total += 1;
            if k == ParkKind::Pond {
                ponds += 1;
            }
        }
        let share = ponds as f32 / total as f32;
        assert!(
            (0.2..=0.30).contains(&share),
            "pond share {share:.2} is not ~1 in 4"
        );
    }

    /// A small square has no room for water, whatever the hash says.
    #[test]
    fn a_small_square_never_gets_a_pond() {
        for i in 0..500 {
            let h = (i as f32 + 0.5) / 500.0;
            assert_ne!(park_kind_for(3, 3, h), ParkKind::Pond);
        }
    }

    /// Every kind is reachable, or the city's greens would still repeat.
    #[test]
    fn every_kind_appears() {
        let mut seen = [false; 6];
        for i in 0..600 {
            let h = (i as f32 + 0.5) / 600.0;
            seen[park_kind_for(6, 6, h).index() as usize] = true;
        }
        assert!(seen.iter().all(|s| *s), "a park kind never appears: {seen:?}");
    }

    /// A pond exists on the *pond* park only, so the people layer and the art
    /// cannot disagree about where the water is.
    #[test]
    fn only_the_pond_park_has_water() {
        for k in ParkKind::ALL {
            let pond = park_pond(6, 6, k);
            assert_eq!(pond.is_some(), k == ParkKind::Pond, "{k:?}");
            assert_eq!(in_park_pond(6, 6, k, 3.0, 3.0), k == ParkKind::Pond, "{k:?}");
        }
        // A 3x3 pond park has no room for water, and neither art nor feet.
        assert!(park_pond(3, 3, ParkKind::Pond).is_none());
        assert!(!in_park_pond(3, 3, ParkKind::Pond, 1.5, 1.5));
    }
}
