//! Spike palette: day colours and their night counterparts.
//!
//! Surfaces go through `Mode::surf` (day: identity; night: darkened and
//! blue-shifted). Emissives (lit windows, neon, lamps) skip it.

use render::palette::{self, Color, rgb};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Day,
    Night,
}

impl Mode {
    /// A lit surface colour (sun/multiplied). Night cools and darkens.
    pub fn surf(&self, c: Color) -> Color {
        match self {
            Mode::Day => c,
            Mode::Night => {
                let d = palette::shade(c, 0.42);
                palette::mix(d, rgb(0x1c, 0x28, 0x46), 0.38)
            }
        }
    }

    /// Sky gradient top/horizon.
    pub fn sky(&self) -> (Color, Color) {
        match self {
            Mode::Day => (rgb(0x6c, 0xb0, 0xe2), rgb(0xce, 0xe8, 0xf4)),
            Mode::Night => (rgb(0x0c, 0x12, 0x2e), rgb(0x2c, 0x2a, 0x52)),
        }
    }
}

// --- deterministic per-position hash for texture speckle -------------------
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

/// A lit window (evening warm) or cool TV-ish window (D25).
pub const WIN_WARM: Color = rgb(0xff, 0xd2, 0x7e);
pub const WIN_COOL: Color = rgb(0xc8, 0xe6, 0xff);
pub const WIN_OFF: Color = rgb(0x2c, 0x38, 0x4e);

/// Per-cell jitter: stable per cs×cs pixel cell (textures read as material,
/// not salt noise). amp is the +/- brightness range.
#[inline]
pub fn jcell(x: i32, y: i32, cs: i32, amp: f32) -> f32 {
    1.0 + (hash01(x / cs.max(1), y / cs.max(1)) - 0.5) * 2.0 * amp
}

/// Sparse blotch mask: true for a few cells (material weathering).
#[inline]
pub fn blotch(x: i32, y: i32, cs: i32, p_mod: u32) -> bool {
    hash2(x / cs.max(1), y / cs.max(1)) % p_mod == 0
}

/// Neon + lamp emissives.
pub const NEON_PINK: Color = rgb(0xff, 0x3e, 0xc8);
pub const NEON_CYAN: Color = rgb(0x26, 0xe0, 0xff);
pub const NEON_ORANGE: Color = rgb(0xff, 0x9a, 0x3c);
pub const LAMP: Color = rgb(0xff, 0xd2, 0x8a);
/// Aircraft-warning beacon.
pub const BEACON: Color = rgb(0xff, 0x3a, 0x30);

// --- daylight surface colours (bright saturated, D4) ------------------------
pub mod c {
    use render::palette::{Color, rgb};

    // ground
    pub const GRASS_A: Color = rgb(0x67, 0xa6, 0x44);
    pub const GRASS_B: Color = rgb(0x5c, 0x9a, 0x3c);
    pub const GARDEN_SOIL: Color = rgb(0x74, 0x5a, 0x3e);
    pub const ASPHALT: Color = rgb(0x55, 0x5a, 0x63);
    pub const ASPHALT_DARK: Color = rgb(0x44, 0x48, 0x50);
    pub const LANE_LINE: Color = rgb(0xd8, 0xd2, 0xa8);
    pub const CENTER_LINE: Color = rgb(0xe8, 0xc8, 0x4c);
    pub const CROSSWALK: Color = rgb(0xe2, 0xe2, 0xdc);
    pub const CURB: Color = rgb(0xc2, 0xc1, 0xb6);
    pub const SIDEWALK: Color = rgb(0xbb, 0xba, 0xb0);
    pub const SIDEWALK_LINE: Color = rgb(0xa4, 0xa3, 0x99);
    pub const MANHOLE: Color = rgb(0x40, 0x44, 0x4c);
    pub const DRIVEWAY: Color = rgb(0x9b, 0x9a, 0x92);
    pub const PAVER_A: Color = rgb(0xc4, 0xbc, 0xb0);
    pub const PAVER_B: Color = rgb(0xb4, 0xaa, 0x9e);

    // clapboard house
    pub const CLAP: Color = rgb(0xee, 0xe6, 0xd2);
    pub const CLAP_SHADE: Color = rgb(0xd8, 0xcc, 0xb2);
    pub const TRIM: Color = rgb(0xf6, 0xf4, 0xea);
    pub const SHUTTER: Color = rgb(0x3e, 0x5a, 0x4a);
    pub const ROOF_SHINGLE: Color = rgb(0x6e, 0x5d, 0x54);
    pub const ROOF_SHINGLE_DARK: Color = rgb(0x5c, 0x4e, 0x46);

    // brick shop
    pub const BRICK: Color = rgb(0x95, 0x52, 0x40);
    pub const BRICK_MORTAR: Color = rgb(0x6e, 0x3c, 0x30);
    pub const BRICK_LIGHT: Color = rgb(0xa6, 0x60, 0x4c);
    pub const AWNING_A: Color = rgb(0xc8, 0x46, 0x46);
    pub const AWNING_B: Color = rgb(0xef, 0xe6, 0xd6);
    pub const SHOP_SIGN: Color = rgb(0x24, 0x2a, 0x34);

    // stucco townhouse
    pub const STUCCO: Color = rgb(0xe4, 0xd2, 0xae);
    pub const STUCCO_SHADE: Color = rgb(0xd2, 0xbe, 0x98);
    pub const TILE_ROOF: Color = rgb(0xb4, 0x54, 0x3a);
    pub const TILE_ROOF_DARK: Color = rgb(0x96, 0x44, 0x30);
    pub const RAIL: Color = rgb(0x28, 0x26, 0x24);

    // glass office / tower
    pub const GLASS_A: Color = rgb(0x6a, 0x96, 0xb8);
    pub const GLASS_B: Color = rgb(0x42, 0x6a, 0x8a);
    pub const GLASS_HI: Color = rgb(0xb8, 0xde, 0xf4);
    pub const GLASS_TOWER_A: Color = rgb(0x4a, 0x84, 0xae);
    pub const GLASS_TOWER_B: Color = rgb(0x28, 0x4c, 0x6e);
    pub const MULLION: Color = rgb(0x20, 0x28, 0x34);
    pub const STEEL: Color = rgb(0x6c, 0x76, 0x80);
    pub const CONCRETE: Color = rgb(0xb4, 0xae, 0xa2);
    pub const ROOF_UNIT: Color = rgb(0x8c, 0x94, 0x9c);
    pub const PARAPET: Color = rgb(0x3a, 0x44, 0x4e);

    // props
    pub const LAMP_POST: Color = rgb(0x28, 0x30, 0x34);
    pub const FENCE: Color = rgb(0xee, 0xee, 0xe4);
    pub const MAILBOX: Color = rgb(0x2e, 0x4a, 0x7c);
    pub const HEDGE: Color = rgb(0x3c, 0x7a, 0x34);
    pub const TRUNK_DARK: Color = rgb(0x5e, 0x40, 0x2a);
    pub const CAR_GLASS: Color = rgb(0x3c, 0x4a, 0x60);
    pub const DOOR: Color = rgb(0x48, 0x34, 0x2c);
    pub const PORCH_BOARDS: Color = rgb(0xb8, 0x9a, 0x72);
    pub const FLOWER_A: Color = rgb(0xe8, 0x5a, 0x78);
    pub const FLOWER_B: Color = rgb(0xf0, 0xd8, 0x5a);
    pub const FLOWER_C: Color = rgb(0xe8, 0xee, 0xf2);
    pub const BENCH: Color = rgb(0x6a, 0x4a, 0x2e);
    pub const HYDRANT: Color = rgb(0xdc, 0x34, 0x28);
}
