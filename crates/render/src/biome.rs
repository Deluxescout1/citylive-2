//! Per-biome look tables (D66): one static table per land, read by the ground,
//! the vegetation, the water and the buildings.
//!
//! The brief's rule is "data-driven: one table per biome", so the tables are
//! `static` values in this module and every consumer looks them up through
//! [`look`]. Nothing here branches on the biome at draw time beyond picking the
//! table, and the temperate table is a byte-for-byte copy of the palette the
//! engine shipped before D66 — so an existing temperate world renders exactly
//! as it did.
//!
//! Blending (D66: "neighbouring biomes blending in at the edges") is
//! [`Pair`]: the host look plus the neighbour's, mixed per tile by
//! `map::TileMap::blend_at`. The ground ramps mix per tile; a plant takes its
//! land wholesale, since half a saguaro is not a plant.

use map::{Biome, Kind, TileMap};
use crate::horizon::Horizon;
use crate::palette::{Color, mix, pal, rgb};

/// Which ground texture a land's soft surfaces carry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GroundTex {
    /// Tufted grass (the temperate look).
    Grass,
    /// Dry sand: ripples and grit.
    Sand,
    /// Snow: a cold sparkle with blue shadow.
    Snow,
    /// Jungle: broad blades, wet and dark green.
    Lush,
}

/// How the real calendar season expresses itself in a land (D68).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeasonStyle {
    /// Four seasons: green summer, autumn turn, bare winter, blossom.
    Temperate,
    /// Arid: the ground barely changes, the trees never bare, "winter" is a
    /// cooler, clearer light.
    Arid,
    /// Boreal: a long bare winter, a short green summer.
    Boreal,
    /// Tropical: ever green, a wet and a dry season rather than four.
    Tropical,
}

/// The look of one land.
#[derive(Clone, Copy)]
pub struct Ground {
    pub biome: Biome,
    pub name: &'static str,

    // --- ground ramps, darkest to lightest (7 steps) ----------------------
    pub grass: [Color; 7],
    pub meadow: [Color; 7],
    pub upland: [Color; 7],
    pub dry_grass: [Color; 7],
    pub scree: [Color; 7],
    pub rock: [Color; 7],
    pub sand: [Color; 7],
    /// Cliff / step faces: earth strata.
    pub earth: [Color; 7],
    /// Cliff faces on high ground.
    pub cliff_rock: [Color; 7],
    /// Retaining wall courses.
    pub wall_stone: [Color; 3],

    // --- water ------------------------------------------------------------
    pub water: [Color; 3],
    pub water_deep: Color,
    pub water_bank: Color,
    pub rapids: Color,

    // --- vegetation -------------------------------------------------------
    pub leaf: [Color; 3],
    pub needles: [Color; 3],
    pub trunk: Color,
    pub bush: [Color; 2],
    pub boulder: [Color; 2],
    pub flowers: [Color; 3],
    /// The green a soft surface takes at its lightest (grass blades, meadow
    /// ground under a flower).
    pub blade: Color,

    // --- soft-ground detail ----------------------------------------------
    pub trampled: Color,
    pub footpath: Color,
    pub cart_track: Color,
    /// Ploughed soil for fields and the bedrock under a farm.
    pub dirt: [Color; 7],

    // --- behaviour --------------------------------------------------------
    pub tex: GroundTex,
    pub season: SeasonStyle,
    /// The horizon treatment this land reads best against (D64).
    pub horizon: Horizon,
    /// Wall colours for a building of this land (buildkit's style hint). The
    /// first is the common wall, the second its trim, the third the roof.
    pub walls: [Color; 3],
    /// Roof material for this land: flat parapet (desert), steep snowy pitch
    /// (tundra), thatch / bright tile (tropical).
    pub roof: RoofStyle,
    /// Standing water freezes here (D66/B46b): the ice surface and its crack
    /// colour. `None` = open water all year.
    pub ice: Option<[Color; 2]>,
    /// The shallow band at a shore: bright turquoise in the tropics (B46b),
    /// a little paler everywhere else.
    pub shallow: Color,
}

/// How a land roofs its buildings (D66: "palette/roof/material variation").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoofStyle {
    /// The catalogue's own pitched roofs.
    AsBuilt,
    /// Flat, parapeted, adobe: the desert.
    Flat,
    /// Steep and snow-laden: the tundra.
    SteepSnow,
    /// Palm thatch early, bright tile later: the tropics.
    Thatch,
}

// ---------------------------------------------------------------------------
// the tables
// ---------------------------------------------------------------------------

/// The land the engine shipped with (D64): byte-for-byte the old palette.
pub static TEMPERATE: Ground = Ground {
    biome: Biome::Temperate,
    name: "temperate",
    grass: pal::GRASS,
    meadow: pal::MEADOW,
    upland: pal::UPLAND,
    dry_grass: pal::DRY_GRASS,
    scree: pal::SCREE,
    rock: pal::ROCK,
    sand: pal::SAND,
    earth: pal::EARTH,
    cliff_rock: pal::CLIFF_ROCK,
    wall_stone: pal::WALL_STONE,
    water: pal::WATER,
    water_deep: pal::WATER_DEEP,
    water_bank: pal::WATER_BANK,
    rapids: pal::RAPIDS,
    leaf: pal::LEAF,
    needles: pal::NEEDLES,
    trunk: pal::TRUNK,
    bush: pal::BUSH,
    boulder: pal::BOULDER,
    flowers: pal::FLOWERS,
    blade: pal::GRASS[1],
    trampled: pal::TRAMPLED,
    footpath: pal::FOOTPATH,
    cart_track: pal::CART_TRACK,
    dirt: pal::DIRT,
    tex: GroundTex::Grass,
    season: SeasonStyle::Temperate,
    horizon: Horizon::Forest,
    walls: [
        rgb(0xc6, 0xbe, 0xb2),
        rgb(0xe0, 0xd6, 0xc0),
        rgb(0x78, 0x4c, 0x42),
    ],
    roof: RoofStyle::AsBuilt,
    ice: None,
    shallow: rgb(0x7f, 0xd2, 0xe2),
};

/// D66: the dune sea. Pale gold sand, ochre scrub, red sandstone mesas, a
/// bleached white-gold horizon (the old CityLive's DUNE SEA, `city.js:3689`).
pub static DESERT: Ground = Ground {
    biome: Biome::Desert,
    name: "desert",
    // A desert has no lawn: even the "grass" kind is sun-bleached sand grass.
    grass: [
        rgb(0xb2, 0x94, 0x5c),
        rgb(0xbd, 0x9f, 0x66),
        rgb(0xc7, 0xa9, 0x70),
        rgb(0xd1, 0xb4, 0x7b),
        rgb(0xda, 0xbe, 0x87),
        rgb(0xe3, 0xc9, 0x94),
        rgb(0xeb, 0xd4, 0xa2),
    ],
    meadow: [
        rgb(0xa2, 0x88, 0x4e),
        rgb(0xae, 0x93, 0x57),
        rgb(0xba, 0x9f, 0x61),
        rgb(0xc5, 0xaa, 0x6c),
        rgb(0xd0, 0xb5, 0x77),
        rgb(0xda, 0xc0, 0x83),
        rgb(0xe3, 0xcb, 0x90),
    ],
    upland: [
        rgb(0x8a, 0x74, 0x44),
        rgb(0x96, 0x7f, 0x4c),
        rgb(0xa2, 0x8a, 0x55),
        rgb(0xad, 0x95, 0x5e),
        rgb(0xb8, 0xa0, 0x68),
        rgb(0xc3, 0xab, 0x73),
        rgb(0xcd, 0xb6, 0x7f),
    ],
    dry_grass: [
        rgb(0x9c, 0x86, 0x52),
        rgb(0xa7, 0x91, 0x5b),
        rgb(0xb1, 0x9c, 0x64),
        rgb(0xbb, 0xa6, 0x6d),
        rgb(0xc4, 0xb0, 0x77),
        rgb(0xcd, 0xba, 0x81),
        rgb(0xd5, 0xc3, 0x8c),
    ],
    // Red sandstone rubble.
    scree: [
        rgb(0x6d, 0x43, 0x33),
        rgb(0x7c, 0x4d, 0x3a),
        rgb(0x8b, 0x58, 0x42),
        rgb(0x99, 0x63, 0x4b),
        rgb(0xa7, 0x6e, 0x55),
        rgb(0xb5, 0x7a, 0x60),
        rgb(0xc2, 0x87, 0x6c),
    ],
    rock: [
        rgb(0x7c, 0x3f, 0x2c),
        rgb(0x8c, 0x49, 0x33),
        rgb(0x9c, 0x54, 0x3a),
        rgb(0xac, 0x60, 0x43),
        rgb(0xbb, 0x6c, 0x4d),
        rgb(0xc8, 0x79, 0x58),
        rgb(0xd4, 0x88, 0x64),
    ],
    sand: [
        rgb(0xba, 0xa2, 0x74),
        rgb(0xc4, 0xad, 0x7e),
        rgb(0xcd, 0xb7, 0x89),
        rgb(0xd5, 0xc1, 0x96),
        rgb(0xdd, 0xca, 0xa2),
        rgb(0xe4, 0xd3, 0xaf),
        rgb(0xea, 0xdb, 0xba),
    ],
    // Mesa strata: deep oxide bands, not brown mud.
    earth: [
        rgb(0x4a, 0x24, 0x18),
        rgb(0x5c, 0x2f, 0x1f),
        rgb(0x6e, 0x3b, 0x27),
        rgb(0x80, 0x47, 0x2f),
        rgb(0x92, 0x54, 0x38),
        rgb(0xa3, 0x62, 0x43),
        rgb(0xb3, 0x70, 0x4e),
    ],
    cliff_rock: [
        rgb(0x5a, 0x2e, 0x22),
        rgb(0x69, 0x38, 0x29),
        rgb(0x78, 0x43, 0x31),
        rgb(0x87, 0x4e, 0x3a),
        rgb(0x95, 0x59, 0x43),
        rgb(0xa3, 0x64, 0x4c),
        rgb(0xb0, 0x70, 0x56),
    ],
    wall_stone: [rgb(0xb0, 0x8c, 0x62), rgb(0xc2, 0x9e, 0x72), rgb(0x8e, 0x6c, 0x48)],
    // A desert river is a green ribbon: keep the water honest, let the banks
    // do the work.
    water: [rgb(0x3a, 0xa8, 0xb4), rgb(0x40, 0xb4, 0xbe), rgb(0x4a, 0xc1, 0xc9)],
    water_deep: rgb(0x2a, 0x86, 0x9a),
    water_bank: rgb(0x1e, 0x6c, 0x76),
    rapids: rgb(0xea, 0xf6, 0xf2),
    // Cacti and scrub: olive and sage, never a lush lawn green.
    leaf: [
        rgb(0x8a, 0x9c, 0x4c),
        rgb(0x6a, 0x7e, 0x3a),
        rgb(0x4e, 0x60, 0x2c),
    ],
    needles: [
        rgb(0x74, 0x8c, 0x42),
        rgb(0x58, 0x6e, 0x34),
        rgb(0x40, 0x52, 0x28),
    ],
    trunk: rgb(0x8a, 0x6c, 0x48),
    bush: [rgb(0x8c, 0x94, 0x56), rgb(0x66, 0x70, 0x3e)],
    boulder: [rgb(0xa8, 0x86, 0x68), rgb(0x7e, 0x5e, 0x46)],
    flowers: [
        rgb(0xe8, 0xc0, 0x4a),
        rgb(0xd8, 0x73, 0x4a),
        rgb(0xf0, 0xe6, 0xc8),
    ],
    blade: rgb(0xc2, 0xa8, 0x6e),
    trampled: rgb(0xd8, 0xc4, 0x94),
    footpath: rgb(0xb0, 0x94, 0x66),
    cart_track: rgb(0xbc, 0xa0, 0x74),
    dirt: [
        rgb(0x7a, 0x5c, 0x3e),
        rgb(0x86, 0x66, 0x45),
        rgb(0x92, 0x70, 0x4c),
        rgb(0x9e, 0x7b, 0x54),
        rgb(0xa9, 0x85, 0x5c),
        rgb(0xb3, 0x8f, 0x66),
        rgb(0xbc, 0x99, 0x71),
    ],
    tex: GroundTex::Sand,
    season: SeasonStyle::Arid,
    horizon: Horizon::Dunes,
    walls: [
        rgb(0xe4, 0xc6, 0x9a), // adobe
        rgb(0xf2, 0xe8, 0xd2), // whitewash
        rgb(0xc6, 0x8c, 0x64), // clay roof
    ],
    roof: RoofStyle::Flat,
    // The oasis pool stays liquid; only a cold snap would skin it.
    ice: None,
    shallow: rgb(0x6e, 0xb4, 0xae),
};

/// D66: snow and tundra. White ground, blue shadow, a nearly black polar sea and
/// a desaturated sky (the old CityLive's PACK ICE, `city.js:3555`).
pub static TUNDRA: Ground = Ground {
    biome: Biome::Tundra,
    name: "tundra",
    grass: [
        rgb(0xb9, 0xc8, 0xd8),
        rgb(0xc4, 0xd2, 0xe0),
        rgb(0xcf, 0xdb, 0xe7),
        rgb(0xd9, 0xe3, 0xed),
        rgb(0xe3, 0xeb, 0xf3),
        rgb(0xec, 0xf2, 0xf8),
        rgb(0xf5, 0xfa, 0xfd),
    ],
    meadow: [
        rgb(0xb2, 0xc2, 0xd4),
        rgb(0xbe, 0xcd, 0xdd),
        rgb(0xc9, 0xd6, 0xe4),
        rgb(0xd4, 0xdf, 0xea),
        rgb(0xdf, 0xe8, 0xf1),
        rgb(0xe9, 0xf0, 0xf6),
        rgb(0xf2, 0xf8, 0xfc),
    ],
    upland: [
        rgb(0xa2, 0xb2, 0xc6),
        rgb(0xae, 0xbd, 0xd0),
        rgb(0xba, 0xc8, 0xd8),
        rgb(0xc5, 0xd2, 0xe1),
        rgb(0xd0, 0xdc, 0xe9),
        rgb(0xdb, 0xe5, 0xf0),
        rgb(0xe6, 0xee, 0xf6),
    ],
    // Frozen stubble poking through.
    dry_grass: [
        rgb(0x8e, 0x8a, 0x7a),
        rgb(0x9a, 0x96, 0x85),
        rgb(0xa5, 0xa1, 0x90),
        rgb(0xaf, 0xac, 0x9c),
        rgb(0xba, 0xb7, 0xa8),
        rgb(0xc4, 0xc2, 0xb4),
        rgb(0xcf, 0xcd, 0xc0),
    ],
    // Bare rock in a snow land is lit by the snow around it: pale blue-grey,
    // not near-black (the first B46b frame at noon read as a grey pancake).
    scree: [
        rgb(0x5e, 0x66, 0x74),
        rgb(0x6a, 0x72, 0x80),
        rgb(0x76, 0x7e, 0x8c),
        rgb(0x82, 0x8a, 0x98),
        rgb(0x8e, 0x96, 0xa4),
        rgb(0x9a, 0xa2, 0xb0),
        rgb(0xa6, 0xae, 0xbc),
    ],
    rock: [
        rgb(0x62, 0x68, 0x76),
        rgb(0x6e, 0x74, 0x82),
        rgb(0x7a, 0x80, 0x8e),
        rgb(0x86, 0x8c, 0x9a),
        rgb(0x92, 0x98, 0xa6),
        rgb(0x9e, 0xa4, 0xb1),
        rgb(0xaa, 0xb0, 0xbc),
    ],
    // Frozen shoreline: shelf ice, pale and blue in the crevices.
    sand: [
        rgb(0xb2, 0xc6, 0xd8),
        rgb(0xbe, 0xd1, 0xe0),
        rgb(0xc9, 0xdb, 0xe8),
        rgb(0xd4, 0xe3, 0xee),
        rgb(0xdf, 0xeb, 0xf3),
        rgb(0xea, 0xf3, 0xf8),
        rgb(0xf4, 0xfa, 0xfd),
    ],
    earth: [
        rgb(0x36, 0x34, 0x34),
        rgb(0x44, 0x41, 0x40),
        rgb(0x51, 0x4e, 0x4c),
        rgb(0x5e, 0x5b, 0x58),
        rgb(0x6b, 0x68, 0x64),
        rgb(0x78, 0x75, 0x70),
        rgb(0x85, 0x82, 0x7c),
    ],
    cliff_rock: [
        rgb(0x3d, 0x41, 0x49),
        rgb(0x49, 0x4d, 0x55),
        rgb(0x55, 0x59, 0x61),
        rgb(0x61, 0x65, 0x6d),
        rgb(0x6d, 0x71, 0x79),
        rgb(0x79, 0x7d, 0x85),
        rgb(0x85, 0x89, 0x91),
    ],
    wall_stone: [rgb(0x9a, 0xa2, 0xac), rgb(0xac, 0xb4, 0xbe), rgb(0x7a, 0x82, 0x8c)],
    // A polar sea is nearly black; the ice around it then reads as ice.
    water: [rgb(0x24, 0x44, 0x5c), rgb(0x2a, 0x4e, 0x68), rgb(0x32, 0x58, 0x74)],
    water_deep: rgb(0x12, 0x26, 0x38),
    water_bank: rgb(0x0e, 0x1e, 0x2e),
    rapids: rgb(0xdc, 0xe8, 0xf2),
    leaf: [
        rgb(0x8e, 0xa4, 0x9a),
        rgb(0x6e, 0x86, 0x7c),
        rgb(0x52, 0x68, 0x60),
    ],
    needles: [
        rgb(0x36, 0x5a, 0x48),
        rgb(0x26, 0x46, 0x38),
        rgb(0x1a, 0x34, 0x2a),
    ],
    trunk: rgb(0x6a, 0x58, 0x48),
    bush: [rgb(0x6e, 0x7c, 0x6c), rgb(0x50, 0x5e, 0x52)],
    boulder: [rgb(0x9a, 0xa0, 0xa8), rgb(0x76, 0x7c, 0x84)],
    flowers: [
        rgb(0xe8, 0xc8, 0xd8),
        rgb(0xd8, 0xe0, 0xe8),
        rgb(0xf6, 0xf8, 0xfa),
    ],
    blade: rgb(0xc8, 0xd6, 0xe4),
    trampled: rgb(0xc4, 0xd2, 0xe0),
    footpath: rgb(0x9c, 0xa4, 0xac),
    cart_track: rgb(0xa8, 0xb0, 0xb8),
    dirt: [
        rgb(0x4e, 0x4a, 0x46),
        rgb(0x5c, 0x58, 0x52),
        rgb(0x6a, 0x66, 0x5f),
        rgb(0x78, 0x74, 0x6c),
        rgb(0x85, 0x81, 0x78),
        rgb(0x92, 0x8e, 0x84),
        rgb(0x9e, 0x9a, 0x90),
    ],
    tex: GroundTex::Snow,
    season: SeasonStyle::Boreal,
    horizon: Horizon::SnowRidges,
    walls: [
        rgb(0x8c, 0x6a, 0x4c), // larch / timber
        rgb(0xdc, 0xe4, 0xea), // snow-laden trim
        rgb(0x8e, 0x96, 0xa0), // slate
    ],
    roof: RoofStyle::SteepSnow,
    // A frozen lake: pale wind-scoured ice, and the crack lines across it.
    ice: Some([rgb(0xd2, 0xe2, 0xee), rgb(0x8e, 0xa8, 0xbc)]),
    shallow: rgb(0xb8, 0xd8, 0xe8),
};

/// D66: tropical jungle and beach. Saturated jungle green, white-coral sand and
/// a bright turquoise sea (the old CityLive's CORAL COAST, `city.js:3502`).
pub static TROPICAL: Ground = Ground {
    biome: Biome::Tropical,
    name: "tropical",
    grass: [
        rgb(0x2e, 0x6e, 0x28),
        rgb(0x37, 0x7d, 0x2d),
        rgb(0x40, 0x8c, 0x33),
        rgb(0x49, 0x9b, 0x39),
        rgb(0x53, 0xaa, 0x40),
        rgb(0x5d, 0xb8, 0x47),
        rgb(0x69, 0xc6, 0x4f),
    ],
    meadow: [
        rgb(0x38, 0x7c, 0x2c),
        rgb(0x42, 0x8b, 0x32),
        rgb(0x4c, 0x9a, 0x38),
        rgb(0x57, 0xa9, 0x3f),
        rgb(0x62, 0xb7, 0x46),
        rgb(0x6d, 0xc4, 0x4d),
        rgb(0x79, 0xd1, 0x56),
    ],
    upland: [
        rgb(0x26, 0x5e, 0x26),
        rgb(0x2e, 0x6c, 0x2b),
        rgb(0x37, 0x7a, 0x31),
        rgb(0x40, 0x88, 0x37),
        rgb(0x49, 0x96, 0x3d),
        rgb(0x52, 0xa3, 0x44),
        rgb(0x5c, 0xb0, 0x4b),
    ],
    dry_grass: [
        rgb(0x76, 0x86, 0x34),
        rgb(0x81, 0x91, 0x3b),
        rgb(0x8c, 0x9c, 0x42),
        rgb(0x96, 0xa6, 0x49),
        rgb(0xa1, 0xb1, 0x51),
        rgb(0xab, 0xbb, 0x59),
        rgb(0xb5, 0xc5, 0x62),
    ],
    scree: [
        rgb(0x4c, 0x4a, 0x40),
        rgb(0x59, 0x57, 0x4c),
        rgb(0x66, 0x64, 0x58),
        rgb(0x73, 0x71, 0x64),
        rgb(0x80, 0x7e, 0x70),
        rgb(0x8d, 0x8b, 0x7c),
        rgb(0x99, 0x97, 0x88),
    ],
    rock: [
        rgb(0x58, 0x54, 0x4c),
        rgb(0x65, 0x61, 0x58),
        rgb(0x72, 0x6e, 0x64),
        rgb(0x7f, 0x7b, 0x70),
        rgb(0x8c, 0x88, 0x7c),
        rgb(0x99, 0x95, 0x88),
        rgb(0xa6, 0xa2, 0x94),
    ],
    // Coral sand: brighter and pinker than a temperate beach.
    sand: [
        rgb(0xd8, 0xc6, 0x9c),
        rgb(0xe2, 0xd1, 0xa8),
        rgb(0xea, 0xdb, 0xb6),
        rgb(0xf0, 0xe4, 0xc4),
        rgb(0xf5, 0xec, 0xd2),
        rgb(0xf8, 0xf1, 0xde),
        rgb(0xfb, 0xf6, 0xea),
    ],
    earth: [
        rgb(0x3e, 0x2c, 0x1e),
        rgb(0x4b, 0x36, 0x25),
        rgb(0x58, 0x40, 0x2c),
        rgb(0x65, 0x4a, 0x34),
        rgb(0x72, 0x54, 0x3c),
        rgb(0x7f, 0x5e, 0x44),
        rgb(0x8b, 0x68, 0x4c),
    ],
    cliff_rock: [
        rgb(0x3c, 0x38, 0x32),
        rgb(0x49, 0x45, 0x3d),
        rgb(0x56, 0x52, 0x48),
        rgb(0x63, 0x5f, 0x53),
        rgb(0x70, 0x6c, 0x5e),
        rgb(0x7d, 0x79, 0x6a),
        rgb(0x8a, 0x86, 0x76),
    ],
    wall_stone: [rgb(0x9e, 0x9a, 0x8c), rgb(0xb0, 0xac, 0x9c), rgb(0x82, 0x7e, 0x72)],
    // Bright, shallow, coral-turquoise.
    water: [rgb(0x1e, 0xb8, 0xc4), rgb(0x26, 0xc8, 0xd0), rgb(0x32, 0xd6, 0xda)],
    water_deep: rgb(0x0f, 0x76, 0x9e),
    water_bank: rgb(0x0c, 0x5e, 0x82),
    rapids: rgb(0xe8, 0xff, 0xfa),
    leaf: [
        rgb(0x86, 0xd0, 0x52),
        rgb(0x4f, 0xa8, 0x3e),
        rgb(0x2c, 0x76, 0x2e),
    ],
    needles: [
        rgb(0x5c, 0xbe, 0x5c),
        rgb(0x3e, 0x94, 0x44),
        rgb(0x26, 0x6a, 0x30),
    ],
    trunk: rgb(0x8c, 0x6a, 0x48),
    bush: [rgb(0x5a, 0xb0, 0x44), rgb(0x3c, 0x84, 0x36)],
    boulder: [rgb(0x8e, 0x8a, 0x7e), rgb(0x6a, 0x66, 0x5c)],
    flowers: [
        rgb(0xf2, 0x6a, 0x8d),
        rgb(0xff, 0xd1, 0x66),
        rgb(0xf0, 0x8a, 0x5a),
    ],
    blade: rgb(0x4a, 0xa8, 0x3c),
    trampled: rgb(0x8a, 0xb0, 0x56),
    footpath: rgb(0xb0, 0x9a, 0x6e),
    cart_track: rgb(0xc0, 0xa8, 0x7c),
    dirt: [
        rgb(0x6a, 0x4e, 0x34),
        rgb(0x76, 0x58, 0x3c),
        rgb(0x82, 0x62, 0x44),
        rgb(0x8e, 0x6d, 0x4c),
        rgb(0x99, 0x77, 0x55),
        rgb(0xa4, 0x82, 0x5e),
        rgb(0xae, 0x8c, 0x68),
    ],
    tex: GroundTex::Lush,
    season: SeasonStyle::Tropical,
    horizon: Horizon::JungleCanopy,
    walls: [
        rgb(0xf2, 0xc8, 0xa8), // coral plaster
        rgb(0xf6, 0xf0, 0xe4), // whitewash
        rgb(0x8a, 0x6a, 0x3c), // palm thatch
    ],
    roof: RoofStyle::Thatch,
    ice: None,
    // The shallows over coral sand: the postcard turquoise.
    shallow: rgb(0x4e, 0xe0, 0xdc),
};


/// A 7-step ramp, darkest to lightest, through a middle colour (B46e): the new
/// lands' ground ramps are built from three anchors instead of 21 literals.
const fn ramp3(lo: Color, mid: Color, hi: Color) -> [Color; 7] {
    const fn lerp(a: Color, b: Color, n: u32, d: u32) -> Color {
        const fn ch(a: Color, b: Color, sh: u32, n: u32, d: u32) -> u8 {
            let x = ((a >> sh) & 0xff) as i32;
            let y = ((b >> sh) & 0xff) as i32;
            (x + (y - x) * n as i32 / d as i32) as u8
        }
        rgb(ch(a, b, 16, n, d), ch(a, b, 8, n, d), ch(a, b, 0, n, d))
    }
    [
        lo,
        lerp(lo, mid, 1, 3),
        lerp(lo, mid, 2, 3),
        mid,
        lerp(mid, hi, 1, 3),
        lerp(mid, hi, 2, 3),
        hi,
    ]
}

/// B46e: alpine. A green valley floor, pasture on the slopes, snow above the
/// treeline (the `upland` ramp *is* the snow here), granite faces and a
/// glacial-teal river.
pub static ALPINE: Ground = Ground {
    biome: Biome::Alpine,
    name: "alpine",
    grass: ramp3(rgb(0x3c, 0x76, 0x3a), rgb(0x5e, 0x9c, 0x46), rgb(0x8c, 0xca, 0x64)),
    meadow: ramp3(rgb(0x52, 0x82, 0x3c), rgb(0x78, 0xa8, 0x4a), rgb(0xa6, 0xd0, 0x6c)),
    upland: ramp3(rgb(0xb0, 0xc4, 0xdc), rgb(0xd8, 0xe6, 0xf3), rgb(0xfa, 0xfd, 0xff)),
    dry_grass: ramp3(rgb(0x82, 0x78, 0x46), rgb(0xa4, 0x98, 0x58), rgb(0xc4, 0xb8, 0x72)),
    scree: ramp3(rgb(0x52, 0x56, 0x62), rgb(0x80, 0x82, 0x8c), rgb(0xae, 0xb0, 0xb8)),
    rock: ramp3(rgb(0x58, 0x5c, 0x68), rgb(0x86, 0x88, 0x92), rgb(0xb6, 0xb8, 0xc0)),
    sand: ramp3(rgb(0x92, 0x8e, 0x84), rgb(0xb8, 0xb4, 0xa8), rgb(0xd8, 0xd4, 0xc8)),
    earth: ramp3(rgb(0x3a, 0x32, 0x2c), rgb(0x62, 0x54, 0x48), rgb(0x8a, 0x7a, 0x6a)),
    cliff_rock: ramp3(rgb(0x44, 0x48, 0x54), rgb(0x6c, 0x70, 0x7c), rgb(0x98, 0x9c, 0xa6)),
    wall_stone: [rgb(0x9c, 0x9c, 0xa0), rgb(0xb4, 0xb2, 0xb0), rgb(0x7c, 0x7c, 0x82)],
    water: [rgb(0x3c, 0xb4, 0xc4), rgb(0x48, 0xc2, 0xce), rgb(0x56, 0xce, 0xd8)],
    water_deep: rgb(0x2a, 0x8e, 0xa6),
    water_bank: rgb(0x1e, 0x70, 0x86),
    rapids: rgb(0xf0, 0xf8, 0xfc),
    leaf: [rgb(0x78, 0xb4, 0x4c), rgb(0x4e, 0x8c, 0x3e), rgb(0x2e, 0x5e, 0x2e)],
    needles: [rgb(0x2e, 0x6a, 0x4a), rgb(0x20, 0x50, 0x3a), rgb(0x14, 0x3a, 0x2a)],
    trunk: rgb(0x6a, 0x4a, 0x34),
    bush: [rgb(0x4a, 0x80, 0x3c), rgb(0x32, 0x5e, 0x2e)],
    boulder: [rgb(0x96, 0x98, 0xa0), rgb(0x70, 0x72, 0x7c)],
    flowers: [rgb(0xf4, 0xdc, 0x58), rgb(0xf6, 0xf6, 0xfa), rgb(0x6c, 0x84, 0xee)],
    blade: rgb(0x80, 0xc0, 0x5c),
    trampled: rgb(0xa6, 0xc0, 0x78),
    footpath: rgb(0xb4, 0xa8, 0x8a),
    cart_track: rgb(0xc0, 0xb2, 0x90),
    dirt: ramp3(rgb(0x5a, 0x46, 0x34), rgb(0x84, 0x6a, 0x4c), rgb(0xa8, 0x90, 0x6c)),
    tex: GroundTex::Grass,
    season: SeasonStyle::Temperate,
    horizon: Horizon::Alpine,
    walls: [
        rgb(0xe6, 0xdc, 0xc8), // limewash
        rgb(0xf4, 0xee, 0xe0), // trim
        rgb(0x6a, 0x48, 0x34), // dark timber shingle
    ],
    roof: RoofStyle::AsBuilt,
    ice: None,
    shallow: rgb(0x6c, 0xd4, 0xd2),
};


/// B46e: the savanna. Gold and straw grass under a high pale sky, red earth
/// showing through, laterite red rock, a slow brown-green river.
pub static SAVANNA: Ground = Ground {
    biome: Biome::Savanna,
    name: "savanna",
    grass: ramp3(rgb(0x8c, 0x86, 0x3e), rgb(0xb6, 0xaa, 0x54), rgb(0xda, 0xca, 0x74)),
    meadow: ramp3(rgb(0x9c, 0x8c, 0x42), rgb(0xc4, 0xb2, 0x5a), rgb(0xe6, 0xd2, 0x7c)),
    upland: ramp3(rgb(0x8a, 0x6c, 0x3c), rgb(0xb8, 0x92, 0x58), rgb(0xdc, 0xb8, 0x78)),
    dry_grass: ramp3(rgb(0xb0, 0x9c, 0x5c), rgb(0xd0, 0xbc, 0x76), rgb(0xea, 0xdc, 0x9a)),
    scree: ramp3(rgb(0x6a, 0x40, 0x30), rgb(0x96, 0x62, 0x48), rgb(0xc0, 0x88, 0x68)),
    rock: ramp3(rgb(0x70, 0x3c, 0x2c), rgb(0xa0, 0x5c, 0x40), rgb(0xcc, 0x84, 0x60)),
    sand: ramp3(rgb(0x9a, 0x54, 0x34), rgb(0xc0, 0x76, 0x4c), rgb(0xdc, 0x9c, 0x6c)),
    earth: ramp3(rgb(0x4a, 0x28, 0x1c), rgb(0x84, 0x4a, 0x30), rgb(0xb0, 0x70, 0x4c)),
    cliff_rock: ramp3(rgb(0x5a, 0x34, 0x28), rgb(0x8a, 0x54, 0x3a), rgb(0xb4, 0x78, 0x58)),
    wall_stone: [rgb(0xb8, 0x80, 0x58), rgb(0xcc, 0x94, 0x68), rgb(0x94, 0x64, 0x44)],
    water: [rgb(0x5a, 0x9a, 0x8a), rgb(0x66, 0xa6, 0x94), rgb(0x74, 0xb2, 0xa0)],
    water_deep: rgb(0x3e, 0x78, 0x70),
    water_bank: rgb(0x6a, 0x50, 0x38),
    rapids: rgb(0xe8, 0xf0, 0xe8),
    leaf: [rgb(0xa6, 0xb0, 0x52), rgb(0x7e, 0x8e, 0x3e), rgb(0x52, 0x62, 0x2c)],
    needles: [rgb(0x7a, 0x8a, 0x44), rgb(0x5a, 0x6a, 0x34), rgb(0x40, 0x50, 0x28)],
    trunk: rgb(0x86, 0x76, 0x62),
    bush: [rgb(0xa8, 0x9c, 0x50), rgb(0x76, 0x6c, 0x38)],
    boulder: [rgb(0xb0, 0x84, 0x68), rgb(0x84, 0x5c, 0x44)],
    flowers: [rgb(0xf0, 0xc8, 0x4a), rgb(0xf4, 0xf0, 0xe0), rgb(0xe0, 0x6a, 0x4a)],
    blade: rgb(0xca, 0xb4, 0x5c),
    trampled: rgb(0xdc, 0xc8, 0x90),
    footpath: rgb(0xb8, 0x8c, 0x5c),
    cart_track: rgb(0xc8, 0x9c, 0x68),
    dirt: ramp3(rgb(0x7a, 0x4a, 0x30), rgb(0xa0, 0x64, 0x40), rgb(0xc4, 0x8a, 0x5c)),
    tex: GroundTex::Grass,
    season: SeasonStyle::Arid,
    horizon: Horizon::Savanna,
    walls: [
        rgb(0xc4, 0x80, 0x58), // red earth
        rgb(0xf0, 0xe4, 0xc8), // limewash
        rgb(0xb8, 0x9a, 0x50), // golden thatch
    ],
    roof: RoofStyle::Thatch,
    ice: None,
    shallow: rgb(0x8a, 0xb8, 0x98),
};


/// B46e: the wetland. Dark green ground, reed-green hummocks, peat and mud,
/// tea-dark water.
pub static SWAMP: Ground = Ground {
    biome: Biome::Swamp,
    name: "swamp",
    grass: ramp3(rgb(0x2e, 0x4a, 0x2a), rgb(0x4a, 0x6c, 0x3a), rgb(0x70, 0x92, 0x52)),
    meadow: ramp3(rgb(0x4a, 0x6a, 0x36), rgb(0x6e, 0x8a, 0x44), rgb(0x98, 0xb0, 0x62)),
    upland: ramp3(rgb(0x38, 0x52, 0x30), rgb(0x58, 0x74, 0x40), rgb(0x7c, 0x98, 0x56)),
    dry_grass: ramp3(rgb(0x6a, 0x5a, 0x34), rgb(0x92, 0x80, 0x4c), rgb(0xb8, 0xa4, 0x6c)),
    scree: ramp3(rgb(0x4a, 0x4a, 0x44), rgb(0x70, 0x6e, 0x62), rgb(0x98, 0x94, 0x84)),
    rock: ramp3(rgb(0x50, 0x52, 0x4a), rgb(0x78, 0x78, 0x6c), rgb(0xa0, 0xa0, 0x90)),
    sand: ramp3(rgb(0x4a, 0x3c, 0x2a), rgb(0x6c, 0x58, 0x3c), rgb(0x8c, 0x76, 0x52)),
    earth: ramp3(rgb(0x30, 0x26, 0x1c), rgb(0x52, 0x40, 0x2e), rgb(0x74, 0x5e, 0x44)),
    cliff_rock: ramp3(rgb(0x40, 0x42, 0x3c), rgb(0x64, 0x66, 0x5a), rgb(0x8a, 0x8a, 0x7c)),
    wall_stone: [rgb(0x86, 0x84, 0x74), rgb(0x9c, 0x9a, 0x88), rgb(0x66, 0x64, 0x58)],
    water: [rgb(0x4a, 0x6c, 0x48), rgb(0x52, 0x76, 0x4e), rgb(0x5c, 0x82, 0x58)],
    water_deep: rgb(0x2c, 0x46, 0x30),
    water_bank: rgb(0x3a, 0x30, 0x20),
    rapids: rgb(0xcf, 0xd8, 0xc8),
    leaf: [rgb(0x7a, 0x96, 0x48), rgb(0x56, 0x76, 0x38), rgb(0x38, 0x52, 0x2a)],
    needles: [rgb(0x4a, 0x6a, 0x3c), rgb(0x34, 0x52, 0x30), rgb(0x24, 0x3c, 0x24)],
    trunk: rgb(0x6a, 0x5a, 0x48),
    bush: [rgb(0x56, 0x78, 0x42), rgb(0x3a, 0x58, 0x30)],
    boulder: [rgb(0x86, 0x88, 0x78), rgb(0x62, 0x64, 0x58)],
    flowers: [rgb(0xf0, 0xe0, 0x6a), rgb(0xf4, 0xf0, 0xf0), rgb(0xd8, 0x7a, 0xc8)],
    blade: rgb(0x6e, 0x92, 0x4a),
    trampled: rgb(0x8a, 0xa0, 0x62),
    footpath: rgb(0x7a, 0x6a, 0x4a),
    cart_track: rgb(0x8a, 0x78, 0x54),
    dirt: ramp3(rgb(0x3a, 0x2e, 0x22), rgb(0x5c, 0x48, 0x34), rgb(0x7e, 0x66, 0x4c)),
    tex: GroundTex::Lush,
    season: SeasonStyle::Tropical,
    horizon: Horizon::Swamp,
    walls: [
        rgb(0x86, 0x78, 0x5e), // weathered board
        rgb(0xd8, 0xd2, 0xc0), // trim
        rgb(0x8a, 0x6a, 0x4a), // rusted tin
    ],
    roof: RoofStyle::AsBuilt,
    ice: None,
    shallow: rgb(0x6a, 0x8a, 0x62),
};

/// The sea lands' shared ground: bright grass, wide pale beaches, turquoise.
const ISLES: Ground = Ground {
    biome: Biome::Islands,
    name: "islands",
    grass: ramp3(rgb(0x3e, 0x86, 0x34), rgb(0x60, 0xae, 0x48), rgb(0x8c, 0xd2, 0x62)),
    meadow: ramp3(rgb(0x4a, 0x92, 0x3a), rgb(0x6e, 0xba, 0x50), rgb(0x9a, 0xdc, 0x6a)),
    upland: ramp3(rgb(0x30, 0x72, 0x2e), rgb(0x4a, 0x92, 0x3c), rgb(0x70, 0xb4, 0x50)),
    dry_grass: ramp3(rgb(0xa8, 0x98, 0x58), rgb(0xcc, 0xbc, 0x74), rgb(0xe8, 0xda, 0x98)),
    scree: ramp3(rgb(0x6a, 0x64, 0x5e), rgb(0x96, 0x8e, 0x84), rgb(0xc0, 0xb8, 0xaa)),
    rock: ramp3(rgb(0x6e, 0x66, 0x5e), rgb(0x9a, 0x90, 0x84), rgb(0xc4, 0xba, 0xaa)),
    sand: ramp3(rgb(0xc8, 0xb8, 0x92), rgb(0xe6, 0xda, 0xb8), rgb(0xfa, 0xf2, 0xd8)),
    earth: ramp3(rgb(0x5a, 0x46, 0x34), rgb(0x84, 0x6a, 0x4c), rgb(0xaa, 0x90, 0x6c)),
    cliff_rock: ramp3(rgb(0x58, 0x52, 0x4e), rgb(0x86, 0x7e, 0x74), rgb(0xb0, 0xa6, 0x98)),
    wall_stone: [rgb(0xc8, 0xbc, 0xa0), rgb(0xdc, 0xd0, 0xb4), rgb(0xa4, 0x98, 0x7c)],
    water: [rgb(0x38, 0xcc, 0xd0), rgb(0x44, 0xd8, 0xd8), rgb(0x52, 0xe2, 0xe0)],
    water_deep: rgb(0x1a, 0x8a, 0xc0),
    water_bank: rgb(0x2a, 0x9a, 0xa8),
    rapids: rgb(0xf4, 0xfc, 0xfc),
    leaf: [rgb(0x78, 0xd0, 0x58), rgb(0x4a, 0xa8, 0x44), rgb(0x2c, 0x70, 0x36)],
    needles: [rgb(0x3a, 0x84, 0x4a), rgb(0x28, 0x68, 0x3e), rgb(0x1a, 0x4c, 0x30)],
    trunk: rgb(0x8a, 0x6c, 0x48),
    bush: [rgb(0x4a, 0x9a, 0x40), rgb(0x32, 0x72, 0x34)],
    boulder: [rgb(0xa8, 0xa0, 0x94), rgb(0x7c, 0x76, 0x6c)],
    flowers: [rgb(0xf2, 0x6a, 0x8d), rgb(0xff, 0xd1, 0x66), rgb(0xf4, 0xf0, 0xe8)],
    blade: rgb(0x58, 0xb0, 0x44),
    trampled: rgb(0x9a, 0xc0, 0x62),
    footpath: rgb(0xd0, 0xc0, 0x9a),
    cart_track: rgb(0xd8, 0xc8, 0xa2),
    dirt: ramp3(rgb(0x6a, 0x50, 0x38), rgb(0x92, 0x72, 0x50), rgb(0xb8, 0x96, 0x70)),
    tex: GroundTex::Lush,
    season: SeasonStyle::Tropical,
    horizon: Horizon::Coast,
    walls: [
        rgb(0xf6, 0xf0, 0xe4), // white
        rgb(0xfa, 0xf8, 0xf0), // trim
        rgb(0xc8, 0x54, 0x40), // red tile
    ],
    roof: RoofStyle::Thatch,
    ice: None,
    shallow: rgb(0x70, 0xf2, 0xe4),
};

/// B46e: an archipelago.
pub static ISLANDS: Ground = Ground { biome: Biome::Islands, name: "islands", ..ISLES };

/// B46e: a lagoon behind a barrier reef: the same land, brighter shallows and a
/// deeper blue beyond the reef.
pub static LAGOON: Ground = Ground {
    biome: Biome::Lagoon,
    name: "lagoon",
    water: [rgb(0x44, 0xe0, 0xd8), rgb(0x50, 0xea, 0xe0), rgb(0x60, 0xf2, 0xe8)],
    water_deep: rgb(0x14, 0x7a, 0xb8),
    water_bank: rgb(0x22, 0xa8, 0xb4),
    shallow: rgb(0x80, 0xf8, 0xe8),
    grass: ramp3(rgb(0x3a, 0x84, 0x3a), rgb(0x5c, 0xac, 0x4e), rgb(0x88, 0xd0, 0x68)),
    leaf: [rgb(0x74, 0xcc, 0x5c), rgb(0x48, 0xa4, 0x48), rgb(0x2a, 0x6c, 0x38)],
    ..ISLES
};

/// B46e: the moors. Pasture green in the glens, rough moor, purple heather on
/// the tops, grey granite, a peat-brown loch.
pub static HIGHLANDS: Ground = Ground {
    biome: Biome::Highlands,
    name: "highlands",
    grass: ramp3(rgb(0x44, 0x6e, 0x3a), rgb(0x62, 0x90, 0x4a), rgb(0x88, 0xb4, 0x62)),
    meadow: ramp3(rgb(0x5a, 0x70, 0x3e), rgb(0x7a, 0x8c, 0x4c), rgb(0xa0, 0xac, 0x68)),
    // B99d: the heather tops were (0x52,0x44,0x4e)..(0x9c,0x8a,0x96) — a dark
    // purple-brown that, over a whole hillside, read as a stain of dirt on the
    // land rather than as heather. Lighter and pinker, so the moor reads as
    // vegetation (the heather clumps and the tufts carry the texture).
    upland: ramp3(rgb(0x6a, 0x5a, 0x62), rgb(0x92, 0x7c, 0x88), rgb(0xb8, 0xa4, 0xac)),
    dry_grass: ramp3(rgb(0x7a, 0x5a, 0x36), rgb(0xa0, 0x78, 0x44), rgb(0xc4, 0x9a, 0x5c)),
    scree: ramp3(rgb(0x56, 0x58, 0x5c), rgb(0x80, 0x80, 0x82), rgb(0xaa, 0xaa, 0xa8)),
    rock: ramp3(rgb(0x5a, 0x5c, 0x60), rgb(0x88, 0x88, 0x88), rgb(0xb4, 0xb2, 0xac)),
    sand: ramp3(rgb(0x8a, 0x84, 0x76), rgb(0xb0, 0xaa, 0x9a), rgb(0xd0, 0xca, 0xb8)),
    earth: ramp3(rgb(0x32, 0x28, 0x20), rgb(0x58, 0x44, 0x34), rgb(0x7e, 0x66, 0x50)),
    cliff_rock: ramp3(rgb(0x44, 0x46, 0x4c), rgb(0x70, 0x70, 0x74), rgb(0x9c, 0x9a, 0x98)),
    wall_stone: [rgb(0x94, 0x94, 0x92), rgb(0xaa, 0xaa, 0xa6), rgb(0x74, 0x74, 0x72)],
    water: [rgb(0x3a, 0x68, 0x7a), rgb(0x44, 0x74, 0x86), rgb(0x50, 0x80, 0x92)],
    water_deep: rgb(0x24, 0x44, 0x58),
    water_bank: rgb(0x34, 0x2c, 0x24),
    rapids: rgb(0xe8, 0xf0, 0xf2),
    leaf: [rgb(0x6e, 0x96, 0x48), rgb(0x4a, 0x74, 0x3a), rgb(0x2e, 0x50, 0x2c)],
    needles: [rgb(0x3a, 0x62, 0x4a), rgb(0x28, 0x4a, 0x3a), rgb(0x1a, 0x34, 0x2a)],
    trunk: rgb(0x6a, 0x56, 0x44),
    bush: [rgb(0x90, 0x62, 0x92), rgb(0x66, 0x42, 0x70)],
    boulder: [rgb(0x98, 0x98, 0x98), rgb(0x70, 0x70, 0x72)],
    flowers: [rgb(0xf4, 0xe0, 0x5a), rgb(0xf4, 0xf4, 0xf4), rgb(0xc0, 0x70, 0xd0)],
    blade: rgb(0x6a, 0x96, 0x4c),
    trampled: rgb(0x8a, 0xa4, 0x68),
    footpath: rgb(0x9a, 0x8c, 0x70),
    cart_track: rgb(0xa8, 0x98, 0x78),
    dirt: ramp3(rgb(0x4a, 0x3a, 0x2c), rgb(0x74, 0x5c, 0x44), rgb(0x9c, 0x82, 0x62)),
    tex: GroundTex::Grass,
    season: SeasonStyle::Temperate,
    horizon: Horizon::Highlands,
    walls: [
        rgb(0x8c, 0x8a, 0x84), // grey stone
        rgb(0xe8, 0xe4, 0xd8), // trim
        rgb(0x4a, 0x50, 0x5a), // slate
    ],
    roof: RoofStyle::AsBuilt,
    ice: None,
    shallow: rgb(0x6a, 0x90, 0x9c),
};

/// The table for a land. A stub falls back to temperate until it is built.
pub fn look(b: Biome) -> &'static Ground {
    match b {
        Biome::Desert => &DESERT,
        Biome::Tundra => &TUNDRA,
        Biome::Tropical => &TROPICAL,
        Biome::Alpine => &ALPINE,
        Biome::Savanna => &SAVANNA,
        Biome::Swamp => &SWAMP,
        Biome::Islands => &ISLANDS,
        Biome::Lagoon => &LAGOON,
        Biome::Highlands => &HIGHLANDS,
        _ => &TEMPERATE,
    }
}

/// The four live lands' tables, for the deliverable shots and the tests.
pub const LIVE: [&Ground; 10] = [
    &TEMPERATE, &DESERT, &TUNDRA, &TROPICAL, &ALPINE, &SAVANNA, &SWAMP, &LAGOON, &ISLANDS, &HIGHLANDS,
];

impl Ground {
    /// The 7-step ramp for a material in this land.
    #[inline]
    pub fn ramp(&self, kind: Kind) -> &[Color; 7] {
        match kind {
            Kind::Grass | Kind::Road | Kind::Sidewalk => &self.grass,
            Kind::Meadow => &self.meadow,
            Kind::Upland => &self.upland,
            Kind::DryGrass => &self.dry_grass,
            Kind::Scree => &self.scree,
            Kind::Rock => &self.rock,
            Kind::Sand => &self.sand,
            Kind::Dirt | Kind::Gravel | Kind::Paved | Kind::Field | Kind::Bridge => &self.dirt,
            Kind::Water => &self.scree,
        }
    }

    /// The ramp index a shaded surface uses, unchanged by the land.
    #[inline]
    pub fn ground(&self, kind: Kind, index: usize) -> Color {
        self.ramp(kind)[index.min(6)]
    }
}

/// The host look plus the neighbour blending in at one edge (D66).
#[derive(Clone, Copy)]
pub struct Pair {
    pub host: &'static Ground,
    pub near: Option<&'static Ground>,
}

impl Default for Pair {
    fn default() -> Self {
        Self {
            host: &TEMPERATE,
            near: None,
        }
    }
}

impl Pair {
    /// The pair for a map: its land plus whatever blends in.
    pub fn of(map: &TileMap) -> Pair {
        Pair {
            host: look(map.biome),
            near: map.neighbour.map(look),
        }
    }

    /// How much of the neighbour sits on this tile.
    #[inline]
    pub fn weight(&self, map: &TileMap, x: i32, y: i32) -> f32 {
        if self.near.is_none() {
            0.0
        } else {
            map.blend_at(x, y)
        }
    }

    /// Ground colour for one material step at a tile, mixed with the neighbour.
    #[inline]
    pub fn ground(&self, map: &TileMap, x: i32, y: i32, kind: Kind, index: usize) -> Color {
        let host = self.host.ramp(kind)[index.min(6)];
        match self.near {
            None => host,
            Some(n) => {
                let w = map.blend_at(x, y);
                if w <= 0.0 {
                    host
                } else {
                    mix(host, n.ramp(kind)[index.min(6)], w)
                }
            }
        }
    }

    /// The look that owns a tile: the neighbour where its tongue of land is
    /// strong, so a palm grows on sand and a spruce on snow.
    #[inline]
    pub fn at(&self, map: &TileMap, x: i32, y: i32) -> &'static Ground {
        match self.near {
            Some(n) if map.biome_at(x, y) == n.biome => n,
            _ => self.host,
        }
    }

    /// The deep-water colour (the sea) at a tile.
    #[inline]
    pub fn deep(&self, map: &TileMap, x: i32, y: i32) -> Color {
        match self.near {
            None => self.host.water_deep,
            Some(n) => {
                let w = map.blend_at(x, y);
                if w <= 0.0 {
                    self.host.water_deep
                } else {
                    mix(self.host.water_deep, n.water_deep, w)
                }
            }
        }
    }

    /// The bank colour (shallow edge, river bed) at a tile.
    #[inline]
    pub fn bank(&self, map: &TileMap, x: i32, y: i32) -> Color {
        match self.near {
            None => self.host.water_bank,
            Some(n) => {
                let w = map.blend_at(x, y);
                if w <= 0.0 {
                    self.host.water_bank
                } else {
                    mix(self.host.water_bank, n.water_bank, w)
                }
            }
        }
    }

    /// Water colour for a tile class, mixed with the neighbour at the edges.
    #[inline]
    pub fn water(&self, map: &TileMap, x: i32, y: i32, j: usize) -> Color {
        let host = self.host.water[j % 3];
        match self.near {
            None => host,
            Some(n) => {
                let w = map.blend_at(x, y);
                if w <= 0.0 {
                    host
                } else {
                    mix(host, n.water[j % 3], w)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// season expression (D68)
// ---------------------------------------------------------------------------

/// Express the real calendar season in a land's own idiom (D68).
///
/// The date still drives it: a tundra in January is deep snow and bare
/// conifers, a desert in July is bleached and dry, a jungle's "autumn" is the
/// wet season. What changes is the amount and the colour, not the calendar.
pub fn season_for(b: Biome, s: crate::terrain::SeasonLook) -> crate::terrain::SeasonLook {
    let g = look(b);
    let mut out = s;
    // The land's own canopy palette, so a desert oak is olive and a jungle oak
    // is vivid, whatever the temperate table says.
    out.green = [
        [crate::palette::r(g.leaf[0]), crate::palette::g(g.leaf[0]), crate::palette::b(g.leaf[0])],
        [crate::palette::r(g.leaf[1]), crate::palette::g(g.leaf[1]), crate::palette::b(g.leaf[1])],
        [crate::palette::r(g.leaf[2]), crate::palette::g(g.leaf[2]), crate::palette::b(g.leaf[2])],
    ];
    match g.season {
        SeasonStyle::Temperate => {}
        SeasonStyle::Arid => {
            // No autumn turn, no bare wood, no blossom: a desert does not shed
            // its leaves. The ground is always dry; the dry season is drier.
            out.autumn = 0.0;
            out.bareness = 0.0;
            out.litter = 0.0;
            out.blossom *= 0.35;
            out.grass_dry = (0.55 + 0.45 * s.grass_dry).clamp(0.0, 1.0);
            out.turn = [
                [0xa8, 0x8a, 0x4a],
                [0x8a, 0x70, 0x3c],
                [0x6e, 0x58, 0x30],
            ];
        }
        SeasonStyle::Boreal => {
            // Long winter: bare wood from late autumn, snow already in the
            // ground, a short and pale green summer.
            out.autumn = (s.autumn * 1.15).min(1.0);
            out.bareness = (s.bareness * 1.25).min(1.0);
            out.grass_dry = s.grass_dry * 0.5;
            out.blossom *= 0.5;
            out.litter *= 0.6;
            out.turn = [
                [0xb8, 0x9a, 0x52],
                [0x96, 0x7a, 0x44],
                [0x6e, 0x5c, 0x3a],
            ];
        }
        SeasonStyle::Tropical => {
            // Ever green. "Autumn" is the wet season: darker, heavier canopy,
            // more litter, but never bare and never a turn.
            out.autumn = 0.0;
            out.bareness = 0.0;
            out.blossom = (s.blossom * 0.8 + s.grass_dry * 0.25).min(1.0);
            out.litter = s.litter * 0.5;
            out.grass_dry = s.grass_dry * 0.25;
            out.turn = [
                [0x9a, 0xc4, 0x4a],
                [0x76, 0x9c, 0x3a],
                [0x54, 0x76, 0x30],
            ];
        }
    }
    out
}

/// Wash a sky framebuffer with a sandstorm's dust (D66): heavier at the horizon,
/// where the sand is, than overhead. Run when the sky is re-baked (a light
/// change), never per frame.
pub fn dust_sky(fb: &mut crate::framebuffer::Framebuffer, amount: f32) {
    let a = amount.clamp(0.0, 1.0);
    if a <= 0.01 {
        return;
    }
    let dust = rgb(0xcf, 0xae, 0x80);
    let (w, h) = (fb.w, fb.h.max(1));
    let px = fb.px_mut();
    for y in 0..h {
        // 0 at the top of the band, 1 at the bottom.
        let t = y as f32 / h as f32;
        let cov = a * (0.10 + 0.55 * t * t);
        if cov <= 0.004 {
            continue;
        }
        let base = y * w;
        for i in base..base + w {
            px[i] = mix(px[i], dust, cov);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use map::{generate_with, Biome};

    #[test]
    fn temperate_is_the_old_palette_exactly() {
        // A temperate world must render exactly as it did before D66.
        let g = look(Biome::Temperate);
        assert_eq!(g.grass, pal::GRASS);
        assert_eq!(g.meadow, pal::MEADOW);
        assert_eq!(g.sand, pal::SAND);
        assert_eq!(g.earth, pal::EARTH);
        assert_eq!(g.water, pal::WATER);
        assert_eq!(g.leaf, pal::LEAF);
        assert_eq!(g.dirt, pal::DIRT);
        assert_eq!(g.ramp(Kind::Grass), &pal::GRASS);
        assert_eq!(g.ramp(Kind::Sand), &pal::SAND);
    }

    /// Every land must be visibly its own: no two tables may share a ground
    /// ramp, and each has to be lighter or darker in the direction its land is.
    #[test]
    fn lands_are_distinct() {
        for (i, a) in LIVE.iter().enumerate() {
            for b in LIVE.iter().skip(i + 1) {
                assert_ne!(a.grass, b.grass, "{} and {} share a ground", a.name, b.name);
                assert_ne!(a.water, b.water, "{} and {} share water", a.name, b.name);
                assert_ne!(a.leaf, b.leaf, "{} and {} share a canopy", a.name, b.name);
            }
        }
        let lum = |c: Color| (crate::palette::r(c) as i32 * 3
            + crate::palette::g(c) as i32 * 6
            + crate::palette::b(c) as i32) as i32;
        // Snow ground is much lighter than grass; desert sand is warmer than snow.
        assert!(lum(TUNDRA.grass[3]) > lum(TEMPERATE.grass[3]) + 240);
        assert!(lum(DESERT.sand[3]) > lum(TEMPERATE.grass[3]) + 150);
        // ...and a desert mesa is red, not grey: R well above B.
        let r = DESERT.rock[3];
        assert!(
            crate::palette::r(r) > crate::palette::b(r) + 60,
            "the mesa is not red"
        );
    }

    #[test]
    fn a_biome_map_blends_only_at_its_edge() {
        // Force a desert with a temperate neighbour and check the pair follows
        // the map's blend.
        let map = generate_with(4, 60, 60, Some(Biome::Desert)).expect("60x60 world");
        assert_eq!(map.biome, Biome::Desert);
        let pair = Pair::of(&map);
        let mid = pair.ground(&map, 30, 30, Kind::Sand, 3);
        assert_eq!(mid, DESERT.sand[3]);
        if let Some(_) = map.neighbour {
            // Somewhere in the band the colour is neither pure host nor pure
            // neighbour.
            let mut blended = 0;
            for y in 0..map.h {
                for x in 0..map.w {
                    let w = map.blend_at(x, y);
                    if w > 0.2 && w < 0.8 {
                        blended += 1;
                        let c = pair.ground(&map, x, y, Kind::Sand, 3);
                        assert_ne!(c, DESERT.sand[3]);
                    }
                }
            }
            assert!(blended > 20, "no blended tiles ({blended})");
        }
    }

    #[test]
    fn seasons_follow_the_land() {
        let s = crate::terrain::SeasonLook {
            autumn: 0.8,
            bareness: 0.7,
            blossom: 0.5,
            litter: 0.6,
            grass_dry: 0.4,
            field_stage: 0.5,
            ..Default::default()
        };
        let d = season_for(Biome::Desert, s);
        assert_eq!((d.autumn, d.bareness, d.litter), (0.0, 0.0, 0.0), "a desert does not bare");
        assert!(d.grass_dry > 0.5);
        let t = season_for(Biome::Tundra, s);
        assert!(t.bareness > s.bareness, "a tundra winters harder");
        let j = season_for(Biome::Tropical, s);
        assert_eq!((j.autumn, j.bareness), (0.0, 0.0), "a jungle never bares");
        // B76: a desert and a jungle never turn, so no autumn leaves fall and
        // none gather under them; a temperate wood does.
        assert_eq!(d.autumn, 0.0);
        assert_eq!(d.litter, 0.0);
        assert_eq!(j.autumn, 0.0);
        // A jungle has litter (its wet-season drop) but nothing turns: the
        // litter is halved, never an autumn fall.
        assert_eq!(j.litter, s.litter * 0.5);
        assert!(t.autumn > 0.0, "a boreal land turns early");
        // The canopy palette follows the land.
        assert_eq!(d.green[1], [0x6a, 0x7e, 0x3a]);
        assert_eq!(j.green[1], [0x4f, 0xa8, 0x3e]);
    }
}
