//! Per-biome building styles (D66 item 4).
//!
//! The brief is explicit: a per-biome *style hint* on the buildkit specs, kept
//! to palette / roof / material variation of the existing catalogue — no second
//! catalogue. So a decorator walks the finished sprite and shifts it into the
//! land's vernacular:
//!
//! * **desert** — adobe: walls pulled towards sand and ochre, saturation down,
//!   the roof band clay-red and flat-looking (the pitch stays, the material is
//!   unmistakably sun-baked).
//! * **tundra** — timber and stone, cooled and desaturated, with the roof band
//!   lifted towards snow so every roof carries its winter load.
//! * **tropical** — bright coral plaster under palm thatch: walls saturated and
//!   warmed, the roof band a green-brown thatch.
//!
//! Everything is deterministic and only touches non-emissive pixels: lit
//! windows, neon and the outline keep their colour, so night still reads as
//! night. It runs once per sprite build (not per frame).

use crate::types::{BuildingSpec, Sprite};
use render::palette::{self, Color, rgb};

/// A land's building vernacular (D66).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum BiomeStyle {
    #[default]
    Temperate,
    Desert,
    Tundra,
    Tropical,
    /// B46e: chalets with deep eaves and balconies.
    Alpine,
    /// B46e: mud-brick and thatch, then tin.
    Savanna,
    /// B46e: weathered stilt houses on the wet ground.
    Swamp,
    /// B46e: bright painted beach houses (islands and lagoons).
    Coastal,
    /// B46e: slate-roofed stone cottages.
    Highlands,
}

impl BiomeStyle {
    pub fn of(b: map::Biome) -> BiomeStyle {
        match b {
            map::Biome::Desert => BiomeStyle::Desert,
            map::Biome::Tundra => BiomeStyle::Tundra,
            map::Biome::Tropical => BiomeStyle::Tropical,
            map::Biome::Alpine => BiomeStyle::Alpine,
            map::Biome::Savanna | map::Biome::Steppe => BiomeStyle::Savanna,
            map::Biome::Swamp => BiomeStyle::Swamp,
            map::Biome::Lagoon | map::Biome::Islands => BiomeStyle::Coastal,
            map::Biome::Highlands => BiomeStyle::Highlands,
            _ => BiomeStyle::Temperate,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BiomeStyle::Temperate => "temperate",
            BiomeStyle::Desert => "desert",
            BiomeStyle::Tundra => "tundra",
            BiomeStyle::Tropical => "tropical",
            BiomeStyle::Alpine => "alpine",
            BiomeStyle::Savanna => "savanna",
            BiomeStyle::Swamp => "swamp",
            BiomeStyle::Coastal => "coastal",
            BiomeStyle::Highlands => "highlands",
        }
    }

    /// Walls: the colour shift applied to everything below the roof band.
    fn wall(self, c: Color) -> Color {
        match self {
            BiomeStyle::Temperate => c,
            BiomeStyle::Desert => {
                // Sun-baked earth: warm and a little bleached. Gentle — this pass
                // only reaches the buildings the land does not draw itself (a
                // tower or a hospital), and a heavy wash read as fog over the
                // walls in review.
                palette::mix(c, rgb(0xe0, 0xc0, 0x92), 0.34)
            }
            BiomeStyle::Tundra => {
                // Cold timber and stone: cooled a little, kept dark enough to read
                // against the snow.
                palette::mix(c, rgb(0x9c, 0xae, 0xc2), 0.20)
            }
            BiomeStyle::Tropical => {
                // Coral plaster: a warm shift, never muddy.
                palette::shade(palette::mix(c, rgb(0xf2, 0xc0, 0x9a), 0.26), 1.03)
            }
            BiomeStyle::Alpine => palette::mix(c, rgb(0xe6, 0xdc, 0xc8), 0.18),
            BiomeStyle::Savanna => palette::mix(c, rgb(0xd8, 0xa8, 0x78), 0.30),
            BiomeStyle::Swamp => palette::mix(c, rgb(0x8a, 0x9a, 0x86), 0.26),
            BiomeStyle::Coastal => palette::mix(c, rgb(0xf0, 0xe8, 0xdc), 0.22),
            BiomeStyle::Highlands => palette::mix(c, rgb(0x9a, 0x9a, 0x94), 0.30),
        }
    }

    /// Roofs: the top band of the sprite (a roof is what a building wears on
    /// top of its walls in an isometric view).
    fn roof(self, c: Color) -> Color {
        match self {
            BiomeStyle::Temperate => c,
            BiomeStyle::Desert => {
                // Flat clay: a dark catalogue roof has to move most of the way or
                // the land keeps black roofs, but only the roof band is touched.
                palette::shade(
                    palette::mix(palette::mix(c, rgb(0xc0, 0x82, 0x5c), 0.72), rgb(0xd2, 0x9c, 0x72), 0.25),
                    1.12,
                )
            }
            BiomeStyle::Tundra => {
                // Snow on the pitch: the roof band goes most of the way white,
                // keeping a hint of the roof's own colour in the shade.
                palette::mix(c, rgb(0xf2, 0xf7, 0xfc), 0.62)
            }
            BiomeStyle::Tropical => {
                // Palm thatch, with a tile-red under it.
                let thatch = palette::mix(c, rgb(0x8a, 0x6a, 0x3c), 0.55);
                palette::mix(thatch, rgb(0xb4, 0x8a, 0x50), 0.20)
            }
            BiomeStyle::Alpine => palette::mix(c, rgb(0x6a, 0x48, 0x34), 0.45),
            BiomeStyle::Savanna => palette::mix(c, rgb(0x9a, 0x7a, 0x52), 0.45),
            BiomeStyle::Swamp => palette::mix(c, rgb(0x8a, 0x6a, 0x58), 0.40),
            BiomeStyle::Coastal => palette::mix(c, rgb(0xc8, 0x60, 0x4a), 0.40),
            BiomeStyle::Highlands => palette::mix(c, rgb(0x4a, 0x50, 0x58), 0.60),
        }
    }
}

/// Pixels this bright and this neutral are structural steel (the catalogue
/// paints it near-white); a biome tint must not turn a steel frame to snow.
fn is_steel(c: Color) -> bool {
    let (r, g, b) = (palette::r(c), palette::g(c), palette::b(c));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    // Bright, near-neutral and *cool*: white clapboard is warm (b < r) and must
    // take the land's wall colour, or a desert village stays New England white.
    min > 190 && max - min < 30 && b >= r
}

/// Is this pixel an outline / a dark detail? Those stay as they are.
fn is_ink(c: Color) -> bool {
    let (r, g, b) = (palette::r(c), palette::g(c), palette::b(c));
    let max = r.max(g).max(b);
    // The catalogue's outline (0x2e2b2a) and its shadow: near-black and neutral.
    // A shaded slate roof is dark too (0x1c2836) but *cool*, and it must take
    // the land's roof material — the first pass skipped it and a desert village
    // kept its navy roofs.
    max < 48 && max - r.min(g).min(b) < 24
}

/// Apply a land's style to one finished sprite. `spec` supplies the style and
/// the sprite's own height, which is what decides where the roof band sits.
pub fn apply_style(sprite: &mut Sprite, spec: &BuildingSpec) {
    let style = spec.biome;
    if style == BiomeStyle::Temperate {
        return;
    }
    // The roof band: the top third of the art (the sprite already carries its
    // transparent margin, so this is measured from the first opaque pixel).
    let mut top = sprite.px.iter().position(|c| (c >> 24) != 0).unwrap_or(0);
    let mut bottom = sprite.px.iter().rposition(|c| (c >> 24) != 0).unwrap_or(0);
    if bottom < top {
        std::mem::swap(&mut top, &mut bottom);
    }
    let h = (bottom - top + 1).max(1) as f32;
    let roof_h = match style {
        // A flat roof is a thin band; a steep one takes more of the height.
        BiomeStyle::Desert => 0.20,
        BiomeStyle::Tundra => 0.30,
        BiomeStyle::Tropical => 0.26,
        BiomeStyle::Alpine | BiomeStyle::Highlands => 0.28,
        BiomeStyle::Savanna | BiomeStyle::Swamp | BiomeStyle::Coastal => 0.24,
        BiomeStyle::Temperate => 0.0,
    };
    let roof_end = top + (h * roof_h) as usize;
    for i in 0..sprite.px.len() {
        let c = sprite.px[i];
        let a = (c >> 24) & 0xFF;
        // Transparent, and emissive (0xFE: a lit window or a neon sign).
        if a == 0 || a == 0xFE {
            continue;
        }
        if is_ink(c) {
            continue;
        }
        let in_roof = i >= top && i <= roof_end;
        let mut out = if in_roof && !is_steel(c) {
            style.roof(c)
        } else {
            style.wall(c)
        };
        // Keep the outline/edge shading of the original: a pixel darker than
        // its neighbours stays relatively darker.
        if is_steel(c) {
            out = c;
        }
        sprite.px[i] = (c & 0xFF00_0000) | (out & 0x00FF_FFFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        BuildingFunction, BuildingSpec, BuildingStyle, ConstructionStage, Era,
    };

    fn spec(style: BiomeStyle) -> BuildingSpec {
        BuildingSpec {
            // The wash is for the hand-made art (B73c: the kit picks the land's
            // own parts instead), so ask for the cape by name.
            building_type: Some(crate::BuildingType::NewEnglandCape),
            function: BuildingFunction::House,
            landmark_kind: None,
            style: BuildingStyle::NewEngland,
            footprint: (1, 1),
            storeys: 2,
            era: Era::Village,
            stage: ConstructionStage::Finished,
            age: 0.0,
            seed: 7,
            variant: 0,
            biome: style,
        }
    }

    fn built(style: BiomeStyle) -> Sprite {
        let mut s = Sprite::new(1, 1);
        crate::draw(&spec(style), &crate::Light::day(), &mut s);
        s
    }

    fn mean(px: &[u32], f: impl Fn(root::Color) -> u32) -> f32 {
        let n = px.iter().filter(|c| (**c >> 24) != 0).count().max(1);
        px.iter()
            .filter(|c| (**c >> 24) != 0)
            .map(|c| f(*c) as f32)
            .sum::<f32>()
            / n as f32
    }

    // A tiny alias so the tests read; the sprite pixels are `render::palette::Color`.
    mod root {
        pub use render::palette::Color;
    }

    #[test]
    fn every_land_shifts_the_sprite() {
        let t = built(BiomeStyle::Temperate);
        for style in [BiomeStyle::Desert, BiomeStyle::Tundra, BiomeStyle::Tropical] {
            let s = built(style);
            assert_eq!(s.px.len(), t.px.len(), "{} changed the sprite size", style.name());
            let (mut same, mut opaque) = (0, 0);
            for (a, b) in s.px.iter().zip(t.px.iter()) {
                if (*b >> 24) == 0 {
                    continue;
                }
                opaque += 1;
                if a == b {
                    same += 1;
                }
            }
            assert!(
                same * 2 < opaque,
                "{} left the sprite untouched ({same}/{opaque})",
                style.name()
            );
        }
    }

    #[test]
    fn desert_is_warmer_and_tundra_roofs_are_lighter() {
        let t = built(BiomeStyle::Temperate);
        let d = built(BiomeStyle::Desert);
        let n = built(BiomeStyle::Tundra);
        // Wall warmth: red minus blue rises towards the sand.
        let warm = |s: &Sprite| {
            mean(&s.px, |c| palette::r(c) as u32) - mean(&s.px, |c| palette::b(c) as u32)
        };
        assert!(
            warm(&d) > warm(&t) + 6.0,
            "the adobe is not warmer ({} vs {})",
            warm(&d),
            warm(&t)
        );
        // Snow on the pitch: the top band is much brighter than the temperate one.
        let top_lum = |s: &Sprite| {
            let top = s.px.iter().position(|c| (c >> 24) != 0).unwrap_or(0);
            let band = &s.px[top..(top + 40).min(s.px.len())];
            mean(band, |c| {
                (palette::r(c) as u32 * 3 + palette::g(c) as u32 * 6 + palette::b(c) as u32) / 10
            })
        };
        assert!(
            top_lum(&n) > top_lum(&t) + 12.0,
            "the tundra roofs carry no snow ({} vs {})",
            top_lum(&n),
            top_lum(&t)
        );
    }

    /// The top quarter of the art, by row (a sprite has a wide transparent
    /// margin, so an index window is not a band).
    fn top_band(s: &Sprite) -> Vec<u32> {
        let rows: Vec<usize> = s
            .px
            .chunks(s.w.max(1))
            .enumerate()
            .filter(|(_, r)| r.iter().any(|c| (c >> 24) != 0))
            .map(|(i, _)| i)
            .collect();
        let (Some(first), Some(last)) = (rows.first().copied(), rows.last().copied()) else {
            return Vec::new();
        };
        let end = (first + (last - first) / 4 + 1).min(s.h);
        let mut out = Vec::new();
        for y in first..end {
            let row = &s.px[y * s.w..(y + 1) * s.w];
            out.extend(row.iter().copied().filter(|c| (c >> 24) != 0));
        }
        out
    }

    #[test]
    fn a_desert_cape_wears_a_clay_roof() {
        use crate::BuildingType;
        let mut t = BuildingType::NewEnglandCape.default_spec(7, 0);
        t.biome = BiomeStyle::Desert;
        let mut s = Sprite::new(1, 1);
        crate::draw(&t, &crate::Light::day(), &mut s);
        let band = top_band(&s);
        assert!(band.len() > 10, "no opaque pixels in the roof band");
        let (r, b) = band.iter().fold((0u32, 0u32), |(r, b), c| {
            (r + palette::r(*c) as u32, b + palette::b(*c) as u32)
        });
        let n = band.len() as u32;
        assert!(
            r > b + 20 * n,
            "the cape's roof is not clay: mean r {} vs b {}",
            r / n,
            b / n
        );
    }

    #[test]
    fn lit_windows_survive_the_style() {
        let s = built(BiomeStyle::Desert);
        // No pixel may have been given an emissive alpha it did not have.
        assert!(
            s.px.iter().all(|c| (c >> 24) != 0xFE),
            "the day sprite invented an emissive pixel"
        );
    }

    /// B46b: the three lands no longer shift the catalogue's house — they draw
    /// their own (adobe / cabin / stilt), so a desert cape must not be the
    /// temperate cape at all: a different silhouette, not a tint.
    #[test]
    fn the_land_draws_its_own_house() {
        use crate::BuildingType;
        let mut t = BuildingType::NewEnglandCape.default_spec(7, 0);
        let temperate = {
            let mut s = Sprite::new(1, 1);
            crate::draw(&t, &crate::Light::day(), &mut s);
            s
        };
        for (style, name) in [
            (BiomeStyle::Desert, "adobe"),
            (BiomeStyle::Tundra, "cabin"),
            (BiomeStyle::Tropical, "stilt"),
        ] {
            t.biome = style;
            let mut s = Sprite::new(1, 1);
            crate::draw(&t, &crate::Light::day(), &mut s);
            // Different art, so the opaque pixel count is different: a palette
            // swap would keep it identical.
            let count = |sp: &Sprite| sp.px.iter().filter(|c| (**c >> 24) != 0).count();
            assert!(
                count(&s) != count(&temperate),
                "{name}: the {style:?} house is still the catalogue's cape"
            );
            // ...and it is opaque: no washed-out walls (the review's
            // "semi-transparent-looking" houses).
            let washed = s
                .px
                .iter()
                .filter(|c| (**c >> 24) != 0)
                .filter(|c| {
                    let (r, g, b) = (palette::r(**c), palette::g(**c), palette::b(**c));
                    let max = r.max(g).max(b);
                    let min = r.min(g).min(b);
                    // A wall pixel must have some contrast of its own.
                    max - min > 6
                })
                .count();
            assert!(washed > 200, "{name}: the walls are flat washes ({washed})");
        }
    }
}
