//! Named colours. Draw code never hard-codes RGB: night and season palettes
//! (M5/M6) will swap these tables wholesale.

pub type Color = u32;

pub const TRANSPARENT: Color = 0;
pub const OPAQUE: u32 = 0xFF00_0000;

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    OPAQUE | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

pub const fn r(c: Color) -> u8 {
    (c >> 16) as u8
}
pub const fn g(c: Color) -> u8 {
    (c >> 8) as u8
}
pub const fn b(c: Color) -> u8 {
    c as u8
}
pub const fn alpha(c: Color) -> u8 {
    (c >> 24) as u8
}

pub const fn with_alpha(c: Color, a: u8) -> Color {
    (c & 0x00FF_FFFF) | ((a as u32) << 24)
}

fn clamp_u8(v: f32) -> u8 {
    if v <= 0.0 {
        0
    } else if v >= 255.0 {
        255
    } else {
        v as u8
    }
}

/// Multiply the RGB channels by `f` (keeps alpha). Used for face shading.
/// Perceived brightness of a colour, 0..255 (B49b: telling a roof deck from the
/// rooftop plant on it, and a lit lamp head from the shade).
pub fn luma(c: Color) -> u8 {
    ((r(c) as u32 * 77 + g(c) as u32 * 151 + b(c) as u32 * 28) >> 8) as u8
}

pub fn shade(c: Color, f: f32) -> Color {
    with_alpha(
        rgb(
            clamp_u8(r(c) as f32 * f),
            clamp_u8(g(c) as f32 * f),
            clamp_u8(b(c) as f32 * f),
        ),
        alpha(c),
    )
}

/// Linear mix, `t = 0` gives `ca`, `t = 1` gives `cb`.
pub fn mix(ca: Color, cb: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| clamp_u8(x as f32 + (y as f32 - x as f32) * t);
    with_alpha(
        rgb(f(r(ca), r(cb)), f(g(ca), g(cb)), f(b(ca), b(cb))),
        alpha(ca),
    )
}

/// Source-over blend of `src` onto `dst` with coverage `alpha` in 0..=1.
pub fn blend(dst: Color, src: Color, cov: f32) -> Color {
    if cov <= 0.0 {
        return dst;
    }
    if cov >= 1.0 {
        return with_alpha(src, alpha(dst).max(alpha(src)));
    }
    let f = |x: u8, y: u8| clamp_u8(x as f32 + (y as f32 - x as f32) * cov);
    with_alpha(
        rgb(f(r(dst), r(src)), f(g(dst), g(src)), f(b(dst), b(src))),
        alpha(dst),
    )
}

/// Slight per-object colour jitter, deterministic from `h` (0..1).
pub fn jitter(c: Color, h: f32, amount: f32) -> Color {
    let d = (h - 0.5) * 2.0 * amount;
    with_alpha(
        rgb(
            clamp_u8(r(c) as f32 * (1.0 + d)),
            clamp_u8(g(c) as f32 * (1.0 + d)),
            clamp_u8(b(c) as f32 * (1.0 + d)),
        ),
        alpha(c),
    )
}

/// Palette tables. Bright saturated day (D4, ref images 1-2).
pub mod pal {
    use super::{Color, rgb};

    // --- sky (own layer, P5) ---
    pub const SKY_TOP: Color = rgb(108, 176, 226);
    pub const SKY_HORIZON: Color = rgb(206, 232, 244);

    // --- terrain ---------------------------------------------------------
    // Each material is a 7-step ramp, darkest to lightest. Slope shading picks
    // a step; a faint height tint nudges it. 5+ shades per material is what
    // makes hills read in an iso projection (B3/Q1).
    pub const GRASS: [Color; 7] = [
        rgb(0x46, 0x79, 0x2e),
        rgb(0x51, 0x86, 0x34),
        rgb(0x5c, 0x93, 0x3b),
        rgb(0x68, 0xa0, 0x43),
        rgb(0x74, 0xae, 0x4c),
        rgb(0x80, 0xbc, 0x55),
        rgb(0x8d, 0xcb, 0x5e),
    ];
    /// Mid-height meadow: slightly paler, warmer than the lush lowland.
    pub const MEADOW: [Color; 7] = [
        rgb(0x4e, 0x83, 0x33),
        rgb(0x5a, 0x91, 0x39),
        rgb(0x66, 0x9e, 0x40),
        rgb(0x73, 0xab, 0x48),
        rgb(0x80, 0xb8, 0x51),
        rgb(0x8c, 0xc5, 0x5a),
        rgb(0x99, 0xd1, 0x64),
    ];
    /// Upland grass: darker and cooler where the forest thins out.
    pub const UPLAND: [Color; 7] = [
        rgb(0x3a, 0x66, 0x2e),
        rgb(0x45, 0x73, 0x34),
        rgb(0x50, 0x80, 0x3a),
        rgb(0x5b, 0x8c, 0x41),
        rgb(0x66, 0x98, 0x48),
        rgb(0x71, 0xa4, 0x4f),
        rgb(0x7c, 0xb0, 0x57),
    ];
    pub const DRY_GRASS: [Color; 7] = [
        rgb(0x8a, 0x7a, 0x3e),
        rgb(0x94, 0x83, 0x43),
        rgb(0x9e, 0x8d, 0x48),
        rgb(0xa8, 0x96, 0x4e),
        rgb(0xb2, 0xa0, 0x55),
        rgb(0xbb, 0xaa, 0x5c),
        rgb(0xc4, 0xb3, 0x63),
    ];
    pub const SCREE: [Color; 7] = [
        rgb(0x50, 0x4a, 0x42),
        rgb(0x5e, 0x57, 0x4d),
        rgb(0x6c, 0x64, 0x59),
        rgb(0x7a, 0x72, 0x66),
        rgb(0x88, 0x80, 0x72),
        rgb(0x96, 0x8d, 0x7e),
        rgb(0xa3, 0x99, 0x89),
    ];
    pub const ROCK: [Color; 7] = [
        rgb(0x5a, 0x55, 0x4f),
        rgb(0x68, 0x62, 0x5b),
        rgb(0x76, 0x6f, 0x67),
        rgb(0x84, 0x7c, 0x73),
        rgb(0x92, 0x89, 0x7f),
        rgb(0x9f, 0x96, 0x8b),
        rgb(0xac, 0xa2, 0x96),
    ];
    /// Crack line between the two facets of a rock tile.
    pub const CREVICE: Color = rgb(0x3f, 0x3b, 0x37);
    pub const SAND: [Color; 7] = [
        rgb(0xbf, 0xad, 0x7d),
        rgb(0xcb, 0xb9, 0x8b),
        rgb(0xd5, 0xc4, 0x99),
        rgb(0xdd, 0xce, 0xa6),
        rgb(0xe5, 0xd7, 0xb4),
        rgb(0xec, 0xe1, 0xc2),
        rgb(0xf1, 0xe8, 0xcf),
    ];
    /// Cliff / step faces: brown strata, never darker grass.
    pub const EARTH: [Color; 7] = [
        rgb(0x3c, 0x2b, 0x20),
        rgb(0x4a, 0x35, 0x28),
        rgb(0x58, 0x3f, 0x30),
        rgb(0x66, 0x4a, 0x38),
        rgb(0x74, 0x55, 0x41),
        rgb(0x82, 0x60, 0x4a),
        rgb(0x8f, 0x6b, 0x53),
    ];
    /// Rock face for cliffs on high ground.
    pub const CLIFF_ROCK: [Color; 7] = [
        rgb(0x39, 0x37, 0x34),
        rgb(0x46, 0x43, 0x3f),
        rgb(0x53, 0x4f, 0x4a),
        rgb(0x60, 0x5c, 0x56),
        rgb(0x6d, 0x68, 0x62),
        rgb(0x7a, 0x75, 0x6e),
        rgb(0x87, 0x81, 0x79),
    ];
    /// Bright top line where a face meets the surface.
    pub const CLIFF_HIGHLIGHT: Color = rgb(0xa8, 0x92, 0x74);

    pub const WATER: [Color; 3] = [
        rgb(0x2f, 0xbf, 0xdd),
        rgb(0x35, 0xc9, 0xe5),
        rgb(0x3e, 0xd3, 0xec),
    ];
    pub const WATER_DEEP: Color = rgb(0x1f, 0x9d, 0xc2);
    pub const WATER_BANK: Color = rgb(0x17, 0x80, 0x9f);
    /// B26 placeholder massing colours, one pair per district style.
    pub const STYLE_STUCCO: Color = rgb(0xd9, 0xcf, 0xb4);
    pub const STYLE_TILE: Color = rgb(0x9c, 0x51, 0x3c);
    pub const STYLE_SIDING: Color = rgb(0xc8, 0xc3, 0xb2);
    pub const STYLE_SHINGLE: Color = rgb(0x5a, 0x5a, 0x62);
    pub const STYLE_CONCRETE: Color = rgb(0xb4, 0xae, 0xa2);
    pub const STYLE_BRONZE: Color = rgb(0x8d, 0x74, 0x5c);
    pub const STYLE_GLASS: Color = rgb(0x7f, 0x9d, 0xb4);
    pub const STYLE_STEEL: Color = rgb(0x6b, 0x74, 0x7d);
    pub const STYLE_VILLA: Color = rgb(0xe4, 0xdc, 0xc8);
    pub const STYLE_BRICK: Color = rgb(0x8a, 0x4a, 0x3a);
    pub const GLASS: Color = rgb(0x35, 0x46, 0x55);
    /// A villager's coat and legs (D45).
    pub const WALKER: Color = rgb(0x3b, 0x33, 0x2f);
    /// Ground trodden flat (D45): a paler, drier grass.
    pub const TRAMPLED: Color = rgb(0x9a, 0xa8, 0x62);
    /// A worn footpath line through the grass (D45).
    pub const FOOTPATH: Color = rgb(0x8d, 0x78, 0x52);
    /// A cart track: bare earth with a hint of wheel ruts (D45).
    pub const CART_TRACK: Color = rgb(0xa0, 0x83, 0x59);
    /// White water on a rapid tile.
    pub const RAPIDS: Color = rgb(0xe8, 0xf7, 0xff);
    /// Street-lamp head and its pool of light.
    pub const LAMP_GLOW: Color = rgb(0xff, 0xd2, 0x8a);
    /// Faint halo around a lit window.
    pub const WINDOW_HALO: Color = rgb(0x8a, 0x6a, 0x38);

    pub const ROAD: [Color; 2] = [rgb(0x5c, 0x62, 0x6b), rgb(0x63, 0x69, 0x72)];
    /// Unsealed lane / field soil.
    pub const DIRT: [Color; 7] = [
        rgb(0x6b, 0x55, 0x3d),
        rgb(0x77, 0x5f, 0x45),
        rgb(0x83, 0x69, 0x4c),
        rgb(0x8f, 0x74, 0x54),
        rgb(0x9a, 0x7e, 0x5c),
        rgb(0xa4, 0x89, 0x66),
        rgb(0xad, 0x93, 0x71),
    ];
    pub const GRAVEL: Color = rgb(0x9a, 0x93, 0x86);
    pub const BRIDGE_DECK: Color = rgb(0x7a, 0x5c, 0x3e);
    pub const BRIDGE_RAIL: Color = rgb(0x5a, 0x42, 0x2c);
    pub const ROAD_LINE: Color = rgb(0xd8, 0xd2, 0xb0);
    // --- construction ------------------------------------------------------
    pub const CONCRETE: Color = rgb(0xa8, 0xa5, 0x9c);
    /// Silhouette outline for buildings (D41).
    pub const OUTLINE: Color = rgb(0x2e, 0x2b, 0x2a);
    pub const TIMBER: Color = rgb(0x9a, 0x74, 0x48);
    pub const SCAFFOLD: Color = rgb(0xc8, 0xa0, 0x5c);
    pub const ROPE: Color = rgb(0xe0, 0xd8, 0xb8);
    pub const BRICK: Color = rgb(0x9a, 0x5e, 0x48);
    // --- building sites (B43c) -----------------------------------------------
    /// Orange safety fence: mesh, shaded mesh, steel post.
    pub const FENCE: Color = rgb(0xf0, 0x82, 0x22);
    pub const FENCE_D: Color = rgb(0xb4, 0x58, 0x1a);
    pub const FENCE_POST: Color = rgb(0x5a, 0x5e, 0x68);
    /// Survey stake (pale wood) and its marker tape.
    pub const STAKE: Color = rgb(0xd2, 0xb4, 0x7c);
    pub const STAKE_TAPE: Color = rgb(0xff, 0x5a, 0x2c);
    /// Structural steel and rebar.
    pub const STEEL: Color = rgb(0x7c, 0x84, 0x92);
    pub const STEEL_D: Color = rgb(0x55, 0x5b, 0x68);
    pub const REBAR: Color = rgb(0x93, 0x52, 0x2e);
    /// Plywood sheathing.
    pub const SHEATHING: Color = rgb(0xd2, 0xb2, 0x7a);
    /// Scaffold plank and safety netting.
    pub const PLANK: Color = rgb(0xb8, 0x8a, 0x52);
    pub const NETTING: Color = rgb(0x3e, 0x9c, 0x5a);
    /// Plant: tower-crane yellow (light, dark), mobile-crane red, tyre black.
    pub const CRANE: Color = rgb(0xec, 0xb8, 0x1c);
    pub const CRANE_D: Color = rgb(0xa0, 0x76, 0x12);
    pub const CRANE_RED: Color = rgb(0xc8, 0x3c, 0x2e);
    pub const TRACK: Color = rgb(0x2c, 0x2c, 0x30);
    pub const COUNTERWEIGHT: Color = rgb(0x8c, 0x8a, 0x86);
    /// Site cabin, portable toilet, skip, mixer drum.
    pub const CABIN: Color = rgb(0xdc, 0xdc, 0xd4);
    pub const TOILET: Color = rgb(0x2e, 0x7a, 0xb8);
    pub const SKIP: Color = rgb(0x2c, 0x62, 0xa8);
    pub const SKIP_STRIPE: Color = rgb(0xf2, 0xc8, 0x20);
    pub const DRUM: Color = rgb(0xdc, 0xd8, 0xcc);
    pub const DRUM_STRIPE: Color = rgb(0xe0, 0x6a, 0x20);
    /// Aviation light on the crane head.
    pub const CRANE_LIGHT: Color = rgb(0xff, 0x30, 0x28);
    /// Hi-vis vest and hard hat.
    pub const HIVIS: Color = rgb(0xe4, 0xe0, 0x2a);
    pub const HIVIS_O: Color = rgb(0xf4, 0x8a, 0x1c);
    pub const HARDHAT: Color = rgb(0xf6, 0xd2, 0x26);
    pub const HARDHAT_W: Color = rgb(0xf0, 0xf0, 0xe8);
    /// Stone retaining wall.
    pub const WALL_STONE: [Color; 3] = [rgb(0x8a, 0x86, 0x7c), rgb(0x9c, 0x98, 0x8d), rgb(0x74, 0x70, 0x68)];
    pub const SIDEWALK: [Color; 2] = [rgb(0xb9, 0xb8, 0xae), rgb(0xb1, 0xb0, 0xa6)];

    // --- face shading ---
    pub const FACE_RIGHT: f32 = 0.72;
    pub const FACE_LEFT: f32 = 0.88;

    // --- vegetation -------------------------------------------------------
    /// Broadleaf canopy: light, mid, dark.
    pub const LEAF: [Color; 3] = [
        rgb(0x6d, 0xc4, 0x52),
        rgb(0x46, 0x9c, 0x42),
        rgb(0x2b, 0x6d, 0x33),
    ];
    /// Conifer needles: light, mid, dark.
    pub const NEEDLES: [Color; 3] = [
        rgb(0x3f, 0x7d, 0x4a),
        rgb(0x2c, 0x64, 0x3e),
        rgb(0x1d, 0x4a, 0x30),
    ];
    pub const TRUNK: Color = rgb(0x7b, 0x55, 0x37);
    pub const BUSH: [Color; 2] = [rgb(0x4d, 0x8f, 0x3f), rgb(0x35, 0x6b, 0x31)];
    pub const BOULDER: [Color; 2] = [rgb(0x8f, 0x8a, 0x83), rgb(0x6a, 0x65, 0x60)];
    pub const FLOWERS: [Color; 3] = [
        rgb(0xf2, 0xe0, 0x6a),
        rgb(0xf0, 0xf0, 0xf4),
        rgb(0xc9, 0x8f, 0xd8),
    ];

    // --- buildings (M4 will replace these with part-based generation) ---
    pub const WALLS: [Color; 8] = [
        rgb(0xba, 0x8e, 0x6e), // brick
        rgb(0xa6, 0x74, 0x58), // dark brick
        rgb(0xc6, 0xbe, 0xb2), // concrete
        rgb(0xe0, 0xd6, 0xc0), // cream
        rgb(0x96, 0xa8, 0xb0), // steel blue
        rgb(0xc4, 0xa8, 0xc4), // lilac
        rgb(0x78, 0x8e, 0x9c), // slate
        rgb(0xd2, 0xb0, 0x8c), // sand
    ];
    pub const ROOFS: [Color; 5] = [
        rgb(0x78, 0x4c, 0x42),
        rgb(0x5c, 0x60, 0x68),
        rgb(0x40, 0x4a, 0x5a),
        rgb(0x88, 0x60, 0x4c),
        rgb(0x54, 0x58, 0x50),
    ];
    pub const BUILDING_SHADOW: Color = rgb(0x38, 0x3c, 0x40);

    pub const WINDOW_DARK: Color = rgb(0x36, 0x42, 0x5c);
    pub const WINDOW_LIT: Color = rgb(0xff, 0xd6, 0x8a);
    pub const WINDOW_LIT_COOL: Color = rgb(0xbe, 0xe2, 0xff);
    pub const DOOR: Color = rgb(0x48, 0x34, 0x2c);

    // --- life ---
    /// B29 traffic placeholders.
    pub const TAXI: Color = rgb(0xf2, 0xc4, 0x2e);
    /// A cart's timber bed, and the sacks or crates on it (B49).
    pub const CART: Color = rgb(0x7a, 0x5a, 0x3a);
    pub const CART_LOAD: [Color; 3] = [rgb(0xa8, 0x8c, 0x52), rgb(0x8e, 0x70, 0x4a), rgb(0xc0, 0xb2, 0x8c)];
    pub const HORSE: Color = rgb(0x4c, 0x38, 0x2a);
    pub const HORSE_DARK: Color = rgb(0x2e, 0x22, 0x1a);
    /// Electric cars: pale, cool bodies, so a quiet car reads differently (B49).
    pub const EV_BODY: [Color; 8] = [
        rgb(0xd8, 0xe4, 0xe8),
        rgb(0x58, 0x98, 0xa8),
        rgb(0xe4, 0xe8, 0xd0),
        rgb(0x3e, 0x68, 0xa6),
        rgb(0xc6, 0x52, 0x4a),
        rgb(0x84, 0x8a, 0x94),
        rgb(0x7a, 0xb8, 0x8a),
        rgb(0xe8, 0xea, 0xec),
    ];
    pub const VAN: Color = rgb(0xdc, 0xdc, 0xd6);
    pub const BUS: Color = rgb(0xc2, 0x4a, 0x3a);
    pub const TRUCK: Color = rgb(0x5a, 0x66, 0x74);
    pub const EMERGENCY: Color = rgb(0xe6, 0xe8, 0xec);
    /// B75: a school bus is the one vehicle allowed to be that yellow.
    pub const SCHOOL_BUS: Color = rgb(0xe8, 0xb4, 0x2a);
    pub const EMERGENCY_RED: Color = rgb(0xff, 0x3a, 0x2a);
    pub const EMERGENCY_BLUE: Color = rgb(0x3a, 0x8c, 0xff);
    pub const HEADLIGHT: Color = rgb(0xff, 0xf2, 0xc8);
    pub const TAILLIGHT: Color = rgb(0xd0, 0x2a, 0x22);
    pub const BUS_WINDOW: Color = rgb(0x9a, 0xc8, 0xe0);
    pub const CAR_BODY: [Color; 6] = [
        rgb(0xd6, 0x52, 0x4a),
        rgb(0xec, 0xc4, 0x54),
        rgb(0x60, 0xa4, 0xd6),
        rgb(0xee, 0xee, 0xe8),
        rgb(0x78, 0xbe, 0x82),
        rgb(0x86, 0x8c, 0x9a),
    ];
    /// B75: the colours a street actually carries, in the order they turn up.
    /// White, silver, grey and black are most of the traffic; dark blue and red
    /// are common; a bright car is a treat. The sim picks an index from this
    /// table by weight, so a random street looks like a car park and not a
    /// sweetshop (D83).
    pub const CAR_PAINT: [Color; 14] = [
        rgb(0xe8, 0xe8, 0xe4), // white
        rgb(0xb4, 0xb8, 0xbe), // silver
        rgb(0x86, 0x8a, 0x90), // grey
        rgb(0x2a, 0x2c, 0x32), // black
        rgb(0x2e, 0x46, 0x72), // dark blue
        rgb(0xb0, 0x3a, 0x34), // red
        rgb(0xd6, 0xd6, 0xd0), // off white
        rgb(0x9a, 0x9e, 0xa4), // light grey
        rgb(0x3c, 0x3e, 0x44), // charcoal
        rgb(0xc2, 0xb4, 0x96), // beige
        rgb(0x3e, 0x6a, 0x74), // teal (uncommon)
        rgb(0x6a, 0x8a, 0x5a), // olive (uncommon)
        rgb(0xe0, 0xa8, 0x3a), // taxi yellow / bright (rare)
        rgb(0xa8, 0x5a, 0x9a), // magenta (rare)
    ];
    /// B75: van, lorry and coach liveries — work vehicles are white or a firm's
    /// colour, never a pearl finish.
    pub const FLEET_PAINT: [Color; 8] = [
        rgb(0xe4, 0xe4, 0xde),
        rgb(0xcc, 0xcc, 0xc6),
        rgb(0x4a, 0x5a, 0x70),
        rgb(0xb0, 0x38, 0x30),
        rgb(0x3c, 0x62, 0x96),
        rgb(0xe0, 0xc0, 0x4a),
        rgb(0x58, 0x6a, 0x52),
        rgb(0x8a, 0x8e, 0x94),
    ];
    pub const CAR_GLASS: Color = rgb(0x3c, 0x4a, 0x60);
    pub const CAR_WHEEL: Color = rgb(0x24, 0x24, 0x28);

    // --- car culture (B59/D72): the places cars need ------------------------
    /// A forecourt canopy: pale steel deck with a bright fascia band.
    pub const CANOPY_TOP: Color = rgb(0xd8, 0xdc, 0xde);
    pub const CANOPY_FASCIA: Color = rgb(0xe8, 0xc8, 0x50);
    /// Pump heads, and the board a forecourt announces itself with.
    pub const PUMP_HEAD: Color = rgb(0xd0, 0x3c, 0x34);
    pub const STYLE_RED_SIGN: Color = rgb(0xd2, 0x3a, 0x32);
    pub const STYLE_BLUE_SIGN: Color = rgb(0x3a, 0x6c, 0xc0);
    /// EV chargers: a pale post and its green "ready" LED.
    pub const CHARGER: Color = rgb(0xdc, 0xdc, 0xd4);
    pub const LED_GREEN: Color = rgb(0x62, 0xe0, 0x84);
    pub const LED_AMBER: Color = rgb(0xf0, 0xb0, 0x3c);
    /// Solar panels over a charging canopy.
    pub const SOLAR_TOP: Color = rgb(0x2e, 0x3e, 0x58);
    /// A burger drive-through: warm ochre walls, a red roof.
    pub const FASTFOOD_WALL: Color = rgb(0xe0, 0xc8, 0x9a);
    pub const FASTFOOD_ROOF: Color = rgb(0xc0, 0x42, 0x38);
    /// A car wash: blue shell, pale brushes.
    pub const WASH_WALL: Color = rgb(0x6a, 0x9c, 0xc4);
    pub const WASH_ROOF: Color = rgb(0x3c, 0x6c, 0x9c);
    pub const WASH_BRUSH: Color = rgb(0xd8, 0xe4, 0xea);
    /// Bunting over a dealership's lot.
    pub const BUNTING_A: Color = rgb(0xe8, 0x5a, 0x4a);
    pub const BUNTING_B: Color = rgb(0xf0, 0xd0, 0x58);
    pub const BUNTING_C: Color = rgb(0x58, 0x9c, 0xe0);
    /// A roadside motel: pale render, a dark roof, a red door, a neon sign.
    pub const MOTEL_WALL: Color = rgb(0xe8, 0xe0, 0xd0);
    pub const MOTEL_ROOF: Color = rgb(0x50, 0x54, 0x5c);
    pub const MOTEL_DOOR: Color = rgb(0x88, 0x3a, 0x30);
    pub const MOTEL_NEON: Color = rgb(0xff, 0x5a, 0x88);

    pub const PEOPLE_SHIRT: [Color; 6] = [
        rgb(0xe2, 0x6c, 0x60),
        rgb(0x78, 0x96, 0xdc),
        rgb(0xf0, 0xd8, 0x78),
        rgb(0x8c, 0xc8, 0x96),
        rgb(0xdc, 0xdc, 0xe2),
        rgb(0xbe, 0x82, 0xc8),
    ];
    pub const PEOPLE_HEAD: Color = rgb(0xe8, 0xc4, 0xa0);
    pub const PEOPLE_LEGS: Color = rgb(0x3a, 0x3a, 0x42);

    pub const RAIN: Color = rgb(0xc4, 0xdc, 0xf0);

    /// Grass tone for a height level (1..=6).
    #[inline]
    pub fn grass(level: u8) -> Color {
        GRASS[(level.max(1) as usize - 1).min(GRASS.len() - 1)]
    }
}

/// The 7-step ramp for a terrain material.
pub fn ramp(kind: map::Kind) -> &'static [Color; 7] {
    use map::Kind;
    match kind {
        Kind::Grass | Kind::Road | Kind::Sidewalk => &pal::GRASS,
        Kind::Meadow => &pal::MEADOW,
        Kind::Upland => &pal::UPLAND,
        Kind::DryGrass => &pal::DRY_GRASS,
        Kind::Scree => &pal::SCREE,
        Kind::Rock => &pal::ROCK,
        Kind::Sand => &pal::SAND,
        Kind::Dirt | Kind::Gravel | Kind::Paved | Kind::Field | Kind::Bridge => &pal::DIRT,
        Kind::Water => &pal::SCREE, // water is painted from pal::WATER, see terrain.rs
    }
}

/// Map a light value (`n · L`, about `0.1..=0.85`) to a ramp index
/// (0 = darkest, 6 = lightest), centred on a flat surface.
#[inline]
pub fn ramp_index(light: f32) -> usize {
    (3.0 + (light - 0.54) * 4.6).round().clamp(0.0, 6.0) as usize
}

/// The clear pixel: `TRANSPARENT` behind a function name, for callers that
/// compare against it by path.
pub const fn transparent() -> Color {
    TRANSPARENT
}
