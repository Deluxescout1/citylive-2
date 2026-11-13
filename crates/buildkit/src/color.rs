//! Colors, material palettes, day/night lighting, and weathering for buildkit.

use render::palette::{self, Color, rgb};
use crate::types::Light;

pub const TRANSPARENT: Color = 0;

pub const FACE_L: f32 = 1.00;
pub const FACE_R: f32 = 0.78;
pub const FACE_TOP: f32 = 1.08;
pub const FACE_REAR: f32 = 0.65;

// --- Emissive colors (bypass surface darkness at night) ----------------------
pub const WIN_WARM: Color = rgb(0xff, 0xd2, 0x7e); // warm evening tungsten
pub const WIN_COOL: Color = rgb(0xc8, 0xe6, 0xff); // cool TV / modern LED
pub const WIN_OFF_DAY: Color = rgb(0x3a, 0x48, 0x5e); // daytime dark glass
pub const WIN_OFF_NIGHT: Color = rgb(0x1a, 0x22, 0x32); // night unlit glass
pub const NEON_PINK: Color = rgb(0xff, 0x3e, 0xc8);
pub const NEON_CYAN: Color = rgb(0x26, 0xe0, 0xff);
pub const NEON_AMBER: Color = rgb(0xff, 0x9a, 0x3c);
pub const NEON_GREEN: Color = rgb(0x36, 0xe6, 0x78);
pub const LAMP_AMBER: Color = rgb(0xff, 0xd2, 0x8a);
pub const BEACON_RED: Color = rgb(0xff, 0x32, 0x28);
pub const GOLD_SIGN: Color = rgb(0xe8, 0xc8, 0x4c);

// --- Material palettes -------------------------------------------------------
pub mod pal {
    use render::palette::{Color, rgb};

    // New England Clapboard
    pub const CLAP_WHITE: Color = rgb(0xf2, 0xee, 0xe2);
    pub const CLAP_BLUE_GREY: Color = rgb(0xa4, 0xb2, 0xc0);
    pub const CLAP_SAGE: Color = rgb(0x9a, 0xb0, 0x98);
    pub const CLAP_BARN_RED: Color = rgb(0x9c, 0x3a, 0x32);
    pub const CLAP_CREAM: Color = rgb(0xee, 0xe4, 0xca);
    pub const CLAP_PALE_YELLOW: Color = rgb(0xea, 0xda, 0xa6);

    // Brick
    pub const BRICK_RED: Color = rgb(0x98, 0x4e, 0x3c);
    pub const BRICK_DARK_CLINKER: Color = rgb(0x72, 0x38, 0x30);
    pub const BRICK_ORANGE: Color = rgb(0xb4, 0x5a, 0x42);
    pub const BRICK_BUFF: Color = rgb(0xc6, 0x9a, 0x72);
    pub const BRICK_MORTAR: Color = rgb(0x6e, 0x40, 0x34);
    pub const BRICK_LIGHT_MORTAR: Color = rgb(0xd4, 0xca, 0xbc);

    // Stucco / Plaster
    pub const STUCCO_OCHRE: Color = rgb(0xe4, 0xc8, 0x98);
    pub const STUCCO_TERRACOTTA: Color = rgb(0xdc, 0x8c, 0x74);
    pub const STUCCO_SAND: Color = rgb(0xee, 0xde, 0xc2);
    pub const STUCCO_ROSE: Color = rgb(0xe8, 0xbe, 0xb4);
    pub const STUCCO_WHITE: Color = rgb(0xf4, 0xf0, 0xe6);

    // Stone & Concrete
    pub const STONE_GRANITE: Color = rgb(0xc8, 0xc6, 0xc0);
    pub const STONE_LIMESTONE: Color = rgb(0xdc, 0xd6, 0xc4);
    pub const STONE_BROWNSTONE: Color = rgb(0x82, 0x54, 0x46);
    pub const STONE_DARK_SLATE: Color = rgb(0x56, 0x5a, 0x62);
    pub const CONCRETE_GREY: Color = rgb(0xa8, 0xa8, 0xa4);
    pub const CONCRETE_LIGHT: Color = rgb(0xd0, 0xce, 0xc8);

    // Roofs
    pub const ROOF_ASPHALT_DARK: Color = rgb(0x54, 0x4e, 0x4c);
    pub const ROOF_SLATE_GREY: Color = rgb(0x4a, 0x54, 0x60);
    pub const ROOF_CEDAR_SHAKE: Color = rgb(0x74, 0x60, 0x50);
    pub const ROOF_TERRACOTTA: Color = rgb(0xba, 0x56, 0x38);
    pub const ROOF_COPPER_NEW: Color = rgb(0xb8, 0x72, 0x48);
    pub const ROOF_COPPER_PATINA: Color = rgb(0x62, 0xa8, 0x90);
    pub const ROOF_ZINC: Color = rgb(0x82, 0x8e, 0x98);
    pub const ROOF_GRAVEL: Color = rgb(0x94, 0x92, 0x8e);

    // Trim & Details
    pub const TRIM_WHITE: Color = rgb(0xf8, 0xf6, 0xf0);
    pub const TRIM_CREAM: Color = rgb(0xf0, 0xe8, 0xd4);
    pub const SHUTTER_GREEN: Color = rgb(0x36, 0x58, 0x44);
    pub const SHUTTER_BLACK: Color = rgb(0x28, 0x28, 0x28);
    pub const SHUTTER_BURGUNDY: Color = rgb(0x64, 0x28, 0x2a);
    pub const WROUGHT_IRON: Color = rgb(0x28, 0x26, 0x26);
    pub const FLOWER_RED: Color = rgb(0xdc, 0x32, 0x38);
    pub const FLOWER_PINK: Color = rgb(0xee, 0x5a, 0x8c);
    pub const FOLIAGE_GREEN: Color = rgb(0x3e, 0x7a, 0x34);

    // Glass & Modern Facades
    pub const GLASS_BLUE_SKY: Color = rgb(0x64, 0x98, 0xc4);
    pub const GLASS_REFLECTIVE: Color = rgb(0x84, 0xb4, 0xd8);
    pub const GLASS_DARK: Color = rgb(0x32, 0x44, 0x56);
    pub const GLASS_BRONZE: Color = rgb(0x6e, 0x5e, 0x52);
    pub const GLASS_TEAL: Color = rgb(0x4c, 0x9e, 0x9c);
    pub const METAL_STEEL_DARK: Color = rgb(0x38, 0x3c, 0x44);
    pub const METAL_CHAMPAGNE: Color = rgb(0xd4, 0xc8, 0xb2);
    pub const METAL_WHITE: Color = rgb(0xee, 0xf0, 0xf4);

    // Ground / Surroundings
    pub const GRASS_LUSH: Color = rgb(0x64, 0xa2, 0x42);
    pub const SOIL_GARDEN: Color = rgb(0x6a, 0x52, 0x3c);
    pub const CONCRETE_DRIVE: Color = rgb(0xb8, 0xb6, 0xae);
    pub const ASPHALT: Color = rgb(0x4e, 0x52, 0x5a);
    pub const CURB: Color = rgb(0xcc, 0xc8, 0xc0);
    pub const SHADOW_GROUND: Color = rgb(0x2a, 0x36, 0x2a);
}

// --- Deterministic hash & noise functions -------------------------------------
#[inline]
pub fn hash2(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6b343) ^ (y as u32).wrapping_mul(0xd8163841);
    h ^= h >> 13;
    h = h.wrapping_mul(0xca5a8265);
    h ^ (h >> 16)
}

#[inline]
pub fn hash01(x: i32, y: i32) -> f32 {
    (hash2(x, y) & 0xffff) as f32 / 65536.0
}

#[inline]
pub fn hash_seed(seed: u64, salt: u32) -> u32 {
    let mut h = seed.wrapping_add(salt as u64);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ceb9fe1a85ec53);
    h ^= h >> 33;
    h as u32
}

#[inline]
pub fn jcell(x: i32, y: i32, cs: i32, amp: f32) -> f32 {
    let cs = cs.max(1);
    1.0 + (hash01(x / cs, y / cs) - 0.5) * 2.0 * amp
}

/// Seed-driven brick color palette (red, dark clinker, buff/yellow, salmon/orange, brownstone, white, sage).
pub fn seed_brick_color(seed: u64) -> Color {
    match (seed % 7) as u32 {
        0 => pal::BRICK_RED,
        1 => pal::BRICK_DARK_CLINKER,
        2 => pal::BRICK_BUFF,
        3 => pal::BRICK_ORANGE,
        4 => pal::STONE_BROWNSTONE,
        5 => pal::CLAP_WHITE,
        _ => pal::CLAP_SAGE,
    }
}

/// Seed-driven glass tint palette (sky blue, reflective mirror, teal, bronze, dark charcoal).
pub fn seed_glass_color(seed: u64) -> Color {
    match (seed % 5) as u32 {
        0 => pal::GLASS_BLUE_SKY,
        1 => pal::GLASS_REFLECTIVE,
        2 => pal::GLASS_TEAL,
        3 => pal::GLASS_BRONZE,
        _ => pal::GLASS_DARK,
    }
}

// --- Lighting & Surface Shading ----------------------------------------------

#[inline]
pub fn shade(c: Color, factor: f32) -> Color {
    palette::shade(c, factor)
}

#[inline]
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    palette::mix(a, b, t)
}

/// Transform a surface color according to light conditions (day tint vs night dimming & blue cooling).
///
/// B99d: the evening is continuous. `light.night` (0 = day, 1 = full night)
/// blends between the hour's own light and the night's: a wall keeps its
/// material and texture all evening, and the change happens over the buckets
/// instead of in one step at a boundary — which is what used to put a flat,
/// warm-brown box where a glass tower had been.
pub fn surf(c: Color, light: &Light, side_shading: f32) -> Color {
    let base = shade(c, side_shading);
    let night = light.night.clamp(0.0, 1.0);
    let night_blue = rgb(0x18, 0x22, 0x3e);
    // Day: the material under this hour's light. Night: the material under the
    // blue night sky (the frame's light pass then dims the whole layer).
    let cooled = if night > 0.0 {
        mix(base, night_blue, 0.36 * night)
    } else {
        base
    };
    let mut r = (palette::r(cooled) as f32 * light.tint[0]).clamp(0.0, 255.0);
    let mut g = (palette::g(cooled) as f32 * light.tint[1]).clamp(0.0, 255.0);
    let mut b = (palette::b(cooled) as f32 * light.tint[2]).clamp(0.0, 255.0);
    // D74/B99d: a low sun still catches what faces up into it — roofs, decks,
    // cornices — while the walls and the shadowed ground stay cool. The peak is
    // the twilight itself (`4n(1-n)`), so noon (n = 0) and full night (n = 1)
    // are untouched and nothing jumps between buckets.
    let golden = 4.0 * night * (1.0 - night);
    if golden > 0.01 && side_shading > 1.04 {
        let k = golden * 0.30;
        r = (r * (1.0 + 0.22 * k)).min(255.0);
        g = (g * (1.0 + 0.02 * k)).min(255.0);
        b = (b * (1.0 - 0.20 * k)).min(255.0);
    }
    palette::with_alpha(
        rgb(r as u8, g as u8, b as u8),
        palette::alpha(cooled),
    )
}

/// Architectural accent light: the warm white a tower's crown lamps and accent
/// strips are lit with, brought up as the evening goes on (B99d).
///
/// It is a *lamp*, not a material: emitted, so the frame's light pass leaves it
/// alone, and warm, so a tower's top reads as a lit crown rather than a strip of
/// neon in a colour the rest of the city never uses. At `night` 0 it is the
/// metal's own highlight, at 1 the full warm white.
pub fn crown_lit(light: &Light) -> Color {
    let n = light.night.clamp(0.0, 1.0);
    let level = 0.45 + 0.55 * n;
    let warm = rgb(
        (0xff as f32 * level).min(255.0) as u8,
        (0xf0 as f32 * level).min(255.0) as u8,
        (0xd2 as f32 * level).min(255.0) as u8,
    );
    palette::with_alpha(warm, 0xFE)
}

/// The metal a lit crown is built from: dark, so the accents stand out, and
/// never a colour of its own at night.
pub fn crown_metal(light: &Light, face: f32) -> Color {
    surf(pal::METAL_STEEL_DARK, light, face)
}

/// Weathering effect applied to surface color based on building age.
pub fn weather(c: Color, x: i32, y: i32, age: f32) -> Color {
    if age <= 0.5 {
        return c;
    }
    let age_factor = (age / 50.0).clamp(0.0, 1.0);
    // Subtle blotches and patina
    let blotch = (hash2(x / 4, y / 4) % 17 == 0) as i32;
    let grime = (hash2(x / 8, y / 6) % 19 == 0) as i32;
    let mut factor = 1.0;
    if blotch != 0 {
        factor -= 0.08 * age_factor;
    }
    if grime != 0 {
        factor -= 0.12 * age_factor;
    }
    let jitter = (hash01(x / 2, y / 2) - 0.5) * 0.06 * age_factor;
    factor += jitter;
    shade(c, factor.clamp(0.70, 1.15))
}
