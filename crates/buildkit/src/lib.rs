//! `buildkit`: detailed building catalogue and procedural generator for CityLive Iso (B33).
//!
//! Provides ≥ 40 recognisably distinct building types rendered in 64-px tile isometric
//! projection with per-pixel depth masks, day/night variants, and weathering.

pub mod color;
pub mod geom;
pub mod parts;
pub mod types;
pub mod stages;
pub mod ruin;
pub mod style;
pub mod vernacular;
pub mod catalogue;
pub mod lots;
pub mod genkit;

pub use types::{BuildingFunction, BuildingSpec, BuildingStyle, BuildingType, ConstructionStage, Era, LandmarkKind, Light, Sprite};
pub use style::BiomeStyle;
pub use lots::{CacheReport, KitCache, light_for, set_city, set_city_in, set_city_ruined, spec_for};

/// Draw a building specified by `spec` under ambient `light` into `out`.
///
/// Output sprite contains 32-bit ARGB/RGBA color pixels (`out.px`), per-pixel depth/height
/// (`out.depth`) for the engine's depth mask, and the anchor point (`out.ox`, `out.oy`) corresponding
/// to the footprint's tile origin (0, 0, 0).
pub fn draw(spec: &BuildingSpec, light: &Light, out: &mut Sprite) {
    let (fw, fh) = (
        spec.footprint.0.max(1) as f32,
        spec.footprint.1.max(1) as f32,
    );
    let h_max = catalogue::max_height(spec);

    let min_x = (-((fh * 32.0 + h_max * 0.72 + 16.0).ceil())) as i32;
    let max_x = ((fw * 32.0 + 24.0).ceil()) as i32;
    let min_y = (-((h_max + 24.0).ceil())) as i32;
    let max_y = (((fw + fh) * 16.0 + 24.0).ceil()) as i32;

    let w = (max_x - min_x + 1).max(1) as usize;
    let h = (max_y - min_y + 1).max(1) as usize;
    let ox = -min_x;
    let oy = -min_y;

    out.resize(w, h);
    out.ox = ox;
    out.oy = oy;
    out.on_surface(render::snow::NONE);

    // B46b: the land draws its own low-rise buildings (adobe, timber cabin,
    // stilt house) out of the parts system. Anything else falls through to the
    // catalogue, and takes the gentler material shift instead.
    if !vernacular::dispatch(spec, light, out, ox as f32, oy as f32) {
        // D66 item 4 used to put the land's palette wash on every building. The
        // kit (B73c) picks the land's own parts instead, so the wash only
        // reaches the hand-made art it was written for.
        if !catalogue::draw_building(spec, light, out, ox as f32, oy as f32) {
            style::apply_style(out, spec);
        }
    }
    // B76 r3: a horizontal face nobody tagged is still a surface snow lies on —
    // the hand-made catalogue's stepped crowns and roof decks fill their tops
    // directly. The depth plane says which pixels are horizontal: on a wall or
    // a slope the height falls away from the pixel above, on a top face it does
    // not. Ground (the plot's lawn and pavement) is far below that.
    tag_horizontal_tops(out);
}

/// Tag every untagged, horizontal, raised face as a flat roof surface.
fn tag_horizontal_tops(out: &mut Sprite) {
    let (w, h) = (out.w, out.h);
    if w < 3 || h < 2 {
        return;
    }
    const MIN_DEPTH: u16 = 8;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if out.surf[i] != render::snow::NONE || out.px[i] == 0 {
                continue;
            }
            let d = out.depth[i];
            if d < MIN_DEPTH {
                continue;
            }
            let above = out.depth[i - w];
            let below = out.depth[i + w];
            let left = out.depth[i - 1];
            // Horizontal: nothing above it is higher, and it does not fall away
            // to the sides or towards the viewer.
            if d >= above && d >= below && d >= left {
                out.surf[i] = render::snow::FLAT;
            }
        }
    }
}
