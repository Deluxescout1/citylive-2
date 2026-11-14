//! CityLive Iso render core (M2).
//!
//! A CPU software rasteriser for a low-res isometric pixel-art world: height
//! aware 2:1 diamond tiles with ramps and water banks, procedural trees,
//! buildings, a cached static layer with per-pixel painter keys (depth) and
//! neighbour-indexed dirty-rectangle repaints, plus a dynamic layer of cars,
//! people and rain composited on top with per-pixel occlusion.
//!
//! Design rules that matter and should survive into `render` proper:
//! * no allocation in the per-frame path (buffers are owned by the caller),
//! * one clip rectangle instead of per-layer overdraw logic,
//! * everything deterministic from a seed,
//! * colours only from [`palette`], so night/season palettes can swap them.

pub mod actors;
pub mod audit;
pub mod bands;
pub mod boats;
pub mod biome;
pub mod farmland;
pub mod building;
pub mod city;
pub mod clouds;
pub mod ending;
pub mod framebuffer;
pub mod holiday;
pub mod horizon;
pub mod iso;
pub mod landform;
pub mod light;
pub mod nightlife;
pub mod palette;
pub mod people;
pub mod planting;
pub mod rain;
pub mod rigs;
pub mod scale;
pub mod street;
pub mod sway;
pub mod scene;
pub mod seasons;
pub mod spectacle;
pub mod shadow;
pub mod shore;
pub mod jetty;
pub mod snow;
pub mod softground;
pub mod civic;
pub mod tech;
pub mod tex;
pub mod terrain;
pub mod rail;
pub mod traffic;
pub mod windows;
pub mod transit;
pub mod testscene;
pub mod weather;
pub mod wear;

pub use framebuffer::{Framebuffer, Rect, Sprite};
pub use map::TileMap;
pub use palette::Color;
pub use city::{CityObj, ObjKind, Stage as BuildStage, Wear};
pub use biome::{Ground, GroundTex, Pair, RoofStyle, SeasonStyle};
pub use horizon::Horizon;
pub use scene::{Probe, Scene, SceneConfig, SceneMode, SceneStats, report_pass_times};

/// Turn the per-pass composite clock on when `CITYLIVE_PASS_TIME` is set.
pub fn pass_time_enable_from_env() -> bool {
    scene::pass_time::enable_from_env()
}
pub use weather::Ambient;
