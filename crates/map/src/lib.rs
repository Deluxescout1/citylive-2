//! Tile-level world data for CityLive Iso.
//!
//! No rendering, no I/O: the growth sim (M3) and the renderer both depend on
//! this crate, so it must stay cheap to clone, deterministic from a seed, and
//! free of graphics concepts.
//!
//! Coordinate conventions (shared with the renderer):
//! * tile `(x, y)` covers `x..x+1, y..y+1` in tile space;
//! * on screen `+x` goes down-right and `+y` goes down-left (2:1 iso);
//! * `height` is an integer level, 0 (lowest, water) ..= [`MAX_LEVEL`];
//! * a tile with `slope: Some(dir)` is a 1-level ramp rising toward `dir`.

pub mod biome;
pub mod carplaces;
pub mod endgeo;
pub mod iso_consts;
pub mod massif;
pub mod rng;
pub mod tiles;
pub mod treeline;
pub mod types;
pub mod world;

mod terrain_gen;

pub use terrain_gen::{generate, generate_with, try_generate_unmarked, try_generate_with, MapError, MIN_SIDE};
pub use tiles::{Lake, Ocean, River, SoftGround, TileMap, WaterKind, WearStage};
pub use biome::Biome;
pub use types::{
    AUX_BRIDGE, AUX_CROP_MASK, AUX_CROP_SHIFT, AUX_HIGHWAY, AUX_RAMP, AUX_STUMP, AUX_TECH, Crop, Dir, Edge, Kind,
    ParkKind, Prop, SlopeKind, Tile, TreeKind, crop_of, set_crop,
    TreeSpecies, WaterLayout,
    edge_works, in_park_pond, in_park_water, park_kind_for, park_pond, park_water, set_edge_works,
};
pub use world::{Viewport, pick_scale, screen_to_tile, world_for_viewport};

pub use types::MAX_LEVEL;
