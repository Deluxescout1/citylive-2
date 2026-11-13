//! B73c: the picker for every family but towers.
//!
//! A low-rise building is a combination of parts, chosen from the lot's own seed
//! within what the district, the era *and the land* allow — the same shape as
//! [`super::pick_tower`], with the land's building traditions as an extra gate
//! (brief §3):
//!
//! * **plan** — how the mass sits on the lot (a box, an L, a split level, a
//!   terrace frontage, a courtyard, a floor on stilts);
//! * **walls** — a cladding (`Finish`), painted from that finish's own palette;
//! * **roof** — a form and a covering, pitched and eaved by the land's weather;
//! * **features** — porch, dormers, a bay, a chimney, a garage, the garden and
//!   the lot's edge.
//!
//! Mid-rise and commercial families reuse the tower kit's facade systems and
//! material palettes ([`super::facades`], [`super::pick_material`]); their
//! low-rise extras (a shop front, a works, a civic program) are their own.
//!
//! Nothing here allocates per pick on the hot path: every table is static and
//! the weighted picks walk a slice.

use render::palette::{rgb, Color};

use super::{
    bases, facades, material_weight, materials, tops, wears, wpick, Base, Body, CivicKind, Ctx,
    Design, Era, Facade, Family, Finish, Massing, Material, Plan, RoofForm, RoofMat, Top,
};
use crate::color;
use crate::style::BiomeStyle;
use crate::types::BuildingStyle;

/// The roof colours a design can wear. Asphalt, cedar, slate, terracotta, tin
/// (two), charcoal and zinc — the eight a real street is made of.
pub const ROOF_TINTS: [Color; 8] = [
    rgb(0x54, 0x4e, 0x4c), // asphalt / dark shingle
    rgb(0x74, 0x60, 0x50), // cedar shake
    rgb(0x4a, 0x54, 0x60), // slate blue-grey
    rgb(0xba, 0x56, 0x38), // terracotta tile
    rgb(0x9c, 0x46, 0x3c), // painted tin, red
    rgb(0x46, 0x6a, 0x56), // painted tin, green
    rgb(0x3a, 0x3e, 0x44), // charcoal metal
    rgb(0x8e, 0x96, 0x9c), // zinc / weathered tin
];

/// The wall colours of each cladding, in the order a land likes them (the
/// street theme and the land both bias this list).
///
/// `const` tables: a pick must not allocate, and a wall colour is data.
const OPAQUE: Color = 0xFF00_0000;
const PAINT_CLAPBOARD: [Color; 8] = [
    OPAQUE | 0xF2_EE_E2, // white
    OPAQUE | 0xEE_E4_CA, // cream
    OPAQUE | 0x9A_B0_98, // sage
    OPAQUE | 0xA4_B2_C0, // blue-grey
    OPAQUE | 0xEA_DA_A6, // pale yellow
    OPAQUE | 0x9C_3A_32, // barn red
    OPAQUE | 0x8C_B4_D0, // sky blue
    OPAQUE | 0xD8_C6_D8, // lilac
];
const PAINT_SHINGLE: [Color; 6] = [
    OPAQUE | 0x8A_70_50, // cedar
    OPAQUE | 0x9A_94_8A, // weathered grey
    OPAQUE | 0x7A_5A_42, // brown
    OPAQUE | 0x5E_4E_42, // dark stain
    OPAQUE | 0xA8_9A_82, // pale tan
    OPAQUE | 0x6C_8A_74, // green stain
];
const PAINT_BRICK: [Color; 6] = [
    OPAQUE | 0x98_4E_3C, // red
    OPAQUE | 0xB4_5A_42, // orange
    OPAQUE | 0xC6_9A_72, // buff
    OPAQUE | 0x72_38_30, // dark clinker
    OPAQUE | 0x82_54_46, // brownstone
    OPAQUE | 0xB0_A2_9A, // whitewashed
];
const PAINT_STONE: [Color; 6] = [
    OPAQUE | 0xDC_D6_C4, // limestone
    OPAQUE | 0xC8_C6_C0, // granite
    OPAQUE | 0xD8_C2_A0, // sandstone
    OPAQUE | 0x8A_90_98, // slate
    OPAQUE | 0xE6_E2_D8, // whitewash
    OPAQUE | 0xC0_A8_88, // ochre stone
];
const PAINT_STUCCO: [Color; 8] = [
    OPAQUE | 0xF4_F0_E6, // white
    OPAQUE | 0xEE_DE_C2, // sand
    OPAQUE | 0xE4_C8_98, // ochre
    OPAQUE | 0xDC_8C_74, // terracotta
    OPAQUE | 0xE8_BE_B4, // rose
    OPAQUE | 0xC8_D8_E0, // pale blue
    OPAQUE | 0xD8_E8_D0, // mint
    OPAQUE | 0xF2_D0_6A, // sunflower
];
const PAINT_TIMBER: [Color; 4] = [
    OPAQUE | 0xF0_E8_D4, // cream plaster
    OPAQUE | 0xE0_D4_B8, // straw plaster
    OPAQUE | 0xF8_F4_EA, // white plaster
    OPAQUE | 0xD8_C0_98, // ochre plaster
];
const PAINT_LOG: [Color; 4] = [
    OPAQUE | 0x8C_68_4A, // 
    OPAQUE | 0x7A_58_40, // 
    OPAQUE | 0x6E_5A_48, // 
    OPAQUE | 0xA0_80_58, // 
];
const PAINT_ADOBE: [Color; 5] = [
    OPAQUE | 0xE0_C8_A0, // 
    OPAQUE | 0xD8_B0_88, // 
    OPAQUE | 0xC8_A0_70, // 
    OPAQUE | 0xE8_D8_B8, // 
    OPAQUE | 0xD0_9C_7C, // 
];
const PAINT_CORRUGATED: [Color; 5] = [
    OPAQUE | 0x6A_7A_68, // dull green
    OPAQUE | 0x8A_5A_44, // rust
    OPAQUE | 0x90_94_98, // galvanised
    OPAQUE | 0x5A_6A_80, // shed blue
    OPAQUE | 0x9A_4A_40, // shed red
];
const PAINT_PANEL: [Color; 7] = [
    OPAQUE | 0xEE_F0_F4, // white
    OPAQUE | 0x3E_44_4C, // charcoal
    OPAQUE | 0x4A_7A_78, // teal
    OPAQUE | 0xB5_64_3C, // oxidised orange
    OPAQUE | 0x8A_90_98, // grey
    OPAQUE | 0xC8_BC_A8, // sand
    OPAQUE | 0x4A_6A_9A, // blue
];
const PAINT_CONCRETE: [Color; 4] = [
    OPAQUE | 0xA8_A8_A4, // 
    OPAQUE | 0x90_90_90, // 
    OPAQUE | 0xB8_B4_AC, // 
    OPAQUE | 0x7A_7C_80, // 
];
const PAINT_GLASS: [Color; 4] = [
    OPAQUE | 0x74_A0_C8, // 
    OPAQUE | 0x6C_A8_9C, // 
    OPAQUE | 0x74_80_8C, // 
    OPAQUE | 0x86_A0_B0, // 
];

pub fn paints(f: Finish) -> &'static [Color] {
    use Finish::*;
    match f {
        Clapboard => &PAINT_CLAPBOARD,
        Shingle => &PAINT_SHINGLE,
        Brick => &PAINT_BRICK,
        Stone => &PAINT_STONE,
        Stucco => &PAINT_STUCCO,
        Timber => &PAINT_TIMBER,
        Log => &PAINT_LOG,
        Adobe => &PAINT_ADOBE,
        Corrugated => &PAINT_CORRUGATED,
        Panel => &PAINT_PANEL,
        Concrete => &PAINT_CONCRETE,
        Glass => &PAINT_GLASS,
    }
}
/// A land's building tradition (brief §3).
struct Trad {
    /// Roof forms the land builds, with weights.
    roofs: &'static [(RoofForm, u32)],
    /// What it covers them with.
    mats: &'static [(RoofMat, u32)],
    /// What it builds walls in.
    finishes: &'static [(Finish, u32)],
    /// Pitch as a percentage of the roof's short side.
    pitch: (u8, u8),
    /// Eave overhang, in hundredths of a tile.
    eaves: u8,
    /// Front feature weights (0 none, 1 stoop, 2 porch, 3 veranda).
    porch: &'static [(u8, u32)],
    /// Lot edge (0 none, 1 wall, 2 hedge, 3 picket).
    fence: &'static [(u8, u32)],
    /// Garden kinds (0 lawn, 1 flowers, 2 vegetables, 3 gravel, 4 pool, 5 paved).
    garden: &'static [(u8, u32)],
    /// How often a house is raised on posts.
    stilts: u32,
    /// How often the plan is a courtyard.
    courtyard: u32,
}

fn trad(b: BiomeStyle) -> Trad {
    use Finish::*;
    use RoofForm::*;
    use RoofMat::*;
    match b {
        BiomeStyle::Temperate => Trad {
            roofs: &[(Gable, 5), (GableCross, 2), (Hip, 3), (Mansard, 1), (Saltbox, 1), (HalfHip, 1), (Flat, 1)],
            mats: &[(RoofMat::Shingle, 5), (Slate, 3), (Tile, 1), (Metal, 1)],
            finishes: &[(Clapboard, 4), (Brick, 3), (Stucco, 3), (Finish::Shingle, 2), (Stone, 2), (Timber, 1), (Panel, 1)],
            pitch: (45, 72),
            eaves: 9,
            porch: &[(0, 2), (1, 1), (2, 3), (3, 2)],
            fence: &[(0, 2), (2, 3), (3, 2), (1, 1)],
            garden: &[(0, 5), (1, 2), (2, 1), (3, 1), (5, 1)],
            stilts: 0,
            courtyard: 2,
        },
        BiomeStyle::Desert => Trad {
            roofs: &[(Flat, 5), (Hip, 3), (Pyramid, 1), (Shed, 1), (Gable, 1)],
            mats: &[(Membrane, 4), (Tile, 3), (Metal, 1)],
            finishes: &[(Adobe, 4), (Stucco, 4), (Stone, 1), (Brick, 1), (Concrete, 1)],
            pitch: (18, 34),
            eaves: 2,
            porch: &[(2, 2), (0, 3), (1, 1), (3, 1)],
            fence: &[(1, 3), (0, 3), (3, 1)],
            garden: &[(3, 4), (5, 2), (4, 1), (0, 1)],
            stilts: 0,
            courtyard: 35,
        },
        BiomeStyle::Tundra => Trad {
            roofs: &[(Gable, 6), (GableCross, 1), (Hip, 1), (Shed, 1), (Saltbox, 1)],
            mats: &[(Metal, 4), (Slate, 2), (RoofMat::Shingle, 2)],
            finishes: &[(Log, 3), (Timber, 3), (Corrugated, 2), (Stone, 2), (Panel, 2)],
            pitch: (78, 108),
            eaves: 12,
            porch: &[(2, 3), (3, 2), (0, 1), (1, 1)],
            fence: &[(0, 4), (3, 1)],
            garden: &[(0, 3), (3, 2), (2, 1)],
            stilts: 0,
            courtyard: 0,
        },
        BiomeStyle::Tropical => Trad {
            roofs: &[(Hip, 4), (Gable, 3), (Shed, 2), (HalfHip, 1), (Flat, 1)],
            mats: &[(Metal, 4), (Thatch, 3), (Tile, 1)],
            finishes: &[(Clapboard, 4), (Stucco, 2), (Timber, 2), (Corrugated, 2), (Panel, 1)],
            pitch: (34, 56),
            eaves: 13,
            porch: &[(3, 4), (2, 2), (0, 1), (1, 1)],
            fence: &[(0, 3), (2, 2), (3, 1)],
            garden: &[(1, 4), (0, 3), (2, 1), (3, 1)],
            stilts: 22,
            courtyard: 2,
        },
        BiomeStyle::Alpine => Trad {
            roofs: &[(Gable, 6), (GableCross, 2), (Hip, 1), (Saltbox, 1), (Shed, 1)],
            mats: &[(RoofMat::Shingle, 4), (Metal, 3), (Slate, 2)],
            finishes: &[(Timber, 4), (Stone, 3), (Stucco, 2), (Log, 2), (Panel, 1)],
            pitch: (62, 88),
            eaves: 15,
            porch: &[(3, 4), (2, 3), (0, 1), (1, 1)],
            fence: &[(2, 3), (0, 2), (1, 1)],
            garden: &[(1, 3), (0, 3), (2, 2)],
            stilts: 0,
            courtyard: 0,
        },
        BiomeStyle::Savanna => Trad {
            roofs: &[(Gable, 3), (Flat, 3), (Shed, 2), (Hip, 2), (HalfHip, 1)],
            mats: &[(Metal, 4), (Thatch, 2), (Membrane, 2), (Tile, 1)],
            finishes: &[(Adobe, 3), (Corrugated, 3), (Stucco, 2), (Brick, 1), (Timber, 1)],
            pitch: (26, 46),
            eaves: 6,
            porch: &[(2, 3), (3, 2), (0, 2), (1, 1)],
            fence: &[(1, 3), (0, 3), (3, 1)],
            garden: &[(3, 3), (2, 2), (0, 2), (1, 1)],
            stilts: 4,
            courtyard: 25,
        },
        BiomeStyle::Swamp => Trad {
            roofs: &[(Gable, 4), (Shed, 2), (Hip, 2), (HalfHip, 1), (Saltbox, 1)],
            mats: &[(Metal, 3), (RoofMat::Shingle, 3), (Thatch, 2)],
            finishes: &[(Timber, 4), (Clapboard, 3), (Corrugated, 2), (Log, 1)],
            pitch: (50, 72),
            eaves: 11,
            porch: &[(3, 4), (2, 2), (0, 1), (1, 1)],
            fence: &[(0, 3), (2, 2), (3, 1)],
            garden: &[(1, 3), (0, 2), (2, 2)],
            stilts: 45,
            courtyard: 1,
        },
        BiomeStyle::Coastal => Trad {
            roofs: &[(Gable, 3), (Hip, 3), (HalfHip, 2), (Mansard, 1), (Flat, 1)],
            mats: &[(Tile, 4), (Metal, 3), (Slate, 1)],
            finishes: &[(Clapboard, 4), (Stucco, 3), (Stone, 2), (Panel, 1)],
            pitch: (36, 58),
            eaves: 9,
            porch: &[(2, 3), (3, 2), (0, 2), (1, 1)],
            fence: &[(1, 2), (2, 2), (0, 2), (3, 1)],
            garden: &[(1, 4), (0, 3), (2, 1)],
            stilts: 6,
            courtyard: 6,
        },
        BiomeStyle::Highlands => Trad {
            roofs: &[(Gable, 4), (Hip, 2), (Saltbox, 2), (HalfHip, 1), (Shed, 1)],
            mats: &[(Slate, 5), (RoofMat::Shingle, 3), (Metal, 2)],
            finishes: &[(Stone, 4), (Stucco, 3), (Clapboard, 2), (Brick, 1), (Timber, 1)],
            pitch: (55, 82),
            eaves: 8,
            porch: &[(0, 3), (1, 1), (2, 2), (3, 1)],
            fence: &[(1, 3), (2, 2), (0, 2)],
            garden: &[(0, 4), (1, 2), (2, 2), (3, 1)],
            stilts: 0,
            courtyard: 2,
        },
    }
}

fn plans(f: Family, t: &Trad) -> Vec<(Plan, u32)> {
    let base: &[(Plan, u32)] = match f {
        Family::House => &[
            (Plan::Box, 4),
            (Plan::Narrow, 1),
            (Plan::L, 3),
            (Plan::T, 2),
            (Plan::Cross, 1),
            (Plan::Split, 3),
            (Plan::Wing, 2),
            (Plan::Courtyard, 1),
            (Plan::Stilts, 1),
        ],
        Family::Townhouse => &[(Plan::Narrow, 5), (Plan::Box, 3), (Plan::L, 1), (Plan::Arcade, 1)],
        Family::Shop => &[(Plan::Box, 4), (Plan::Narrow, 2), (Plan::Arcade, 2), (Plan::L, 1), (Plan::Narrow, 1)],
        Family::Apartment => &[(Plan::Box, 4), (Plan::L, 3), (Plan::T, 2), (Plan::Cross, 1), (Plan::Narrow, 1)],
        Family::Office => &[(Plan::Box, 4), (Plan::L, 2), (Plan::Narrow, 1), (Plan::Wing, 1)],
        Family::Industrial => &[(Plan::Wing, 3), (Plan::Box, 4), (Plan::L, 2), (Plan::T, 1)],
        Family::Civic => &[(Plan::Wing, 3), (Plan::Box, 3), (Plan::L, 2), (Plan::T, 1)],
        Family::Tower => &[(Plan::Box, 1)],
    };
    base.iter()
    .map(|&(p, w)| {
        // The land's own habit: raised floors in the wet lands, courtyards in
        // the dry ones (both are the plan, not a decoration).
        let w = match p {
            Plan::Stilts => w * t.stilts.max(1) / 6,
            Plan::Courtyard => w * t.courtyard.max(1) / 8,
            _ => w,
        };
        (p, w.max(1))
    })
    .collect()
}

/// Roof forms a family may wear. A modern office is a flat box; a terrace in the
/// old core can still be a mansard.
fn roofs(f: Family, style: BuildingStyle, era: Era, t: &Trad) -> Vec<(RoofForm, u32)> {
    use RoofForm::*;
    let old = matches!(
        style,
        BuildingStyle::BrickMainStreet | BuildingStyle::European | BuildingStyle::HighEnd
    );
    let modern = matches!(style, BuildingStyle::ModernGlass)
        && matches!(era, Era::City | Era::Metropolis);
    let list: Vec<(RoofForm, u32)> = match f {
        Family::House => t.roofs.to_vec(),
        Family::Townhouse => {
            if old {
                vec![(Mansard, 3), (Gable, 3), (Flat, 3), (HalfHip, 1), (Saltbox, 1), (Hip, 1)]
            } else {
                vec![(Flat, 4), (Gable, 3), (Hip, 1), (Mansard, 1)]
            }
        }
        Family::Shop => {
            if modern {
                vec![(Flat, 6), (Shed, 1)]
            } else if old {
                vec![(Flat, 4), (Mansard, 3), (Gable, 2), (Hip, 1)]
            } else {
                vec![(Flat, 3), (Gable, 3), (Hip, 1)]
            }
        }
        Family::Apartment => {
            if modern {
                vec![(Flat, 6), (Shed, 1)]
            } else {
                vec![(Flat, 5), (Mansard, 2), (Gable, 2), (Hip, 1)]
            }
        }
        Family::Office => vec![(Flat, 7), (Shed, 1)],
        Family::Industrial => vec![(Shed, 3), (Flat, 4), (Gable, 2)],
        Family::Civic => vec![(Flat, 3), (Hip, 3), (Gable, 3), (Mansard, 1), (Pyramid, 1)],
        Family::Tower => vec![(Flat, 1)],
    };
    // A land with heavy snow keeps a steep pitch whatever the district says, so
    // a mid-rise's flat roof is rare there (the land's own roof table wins).
    if t.pitch.0 >= 78 && f != Family::Industrial {
        let mut out = list.clone();
        out.extend(t.roofs.iter().copied().filter(|(r, _)| *r != Flat));
        return out;
    }
    list
}

fn roof_mats(f: Family, t: &Trad) -> &'static [(RoofMat, u32)] {
    match f {
        Family::Industrial => &[(RoofMat::Metal, 5), (RoofMat::Membrane, 3), (RoofMat::Tile, 1)],
        Family::Office => &[(RoofMat::Membrane, 7), (RoofMat::Metal, 3)],
        _ => t.mats,
    }
}

fn finishes(f: Family, style: BuildingStyle, t: &Trad) -> Vec<(Finish, u32)> {
    use Finish::*;
    let mut out: Vec<(Finish, u32)> = t.finishes.to_vec();
    match f {
        // A block in the modern district is a panel or glass box; the old core
        // is brick and stone; the works are concrete, steel and brick.
        Family::Apartment | Family::Office | Family::Shop if style == BuildingStyle::ModernGlass => {
            out.extend([(Panel, 4), (Glass, 3), (Concrete, 1)]);
        }
        Family::Apartment | Family::Office | Family::Shop if style == BuildingStyle::BrickMainStreet => {
            out.extend([(Brick, 4), (Stone, 2), (Stucco, 1)]);
        }
        Family::Apartment | Family::Office | Family::Shop if style == BuildingStyle::HighEnd => {
            out.extend([(Stone, 4), (Stucco, 2), (Panel, 2)]);
        }
        Family::Industrial => {
            out.extend([(Brick, 4), (Concrete, 4), (Corrugated, 4), (Panel, 2)]);
        }
        Family::Civic => {
            out.extend([(Brick, 3), (Stone, 3), (Stucco, 2), (Panel, 2), (Concrete, 1)]);
        }
        _ => {}
    }
    out
}

/// The podium a commercial family stands on (the tower kit's own bases).
fn base_for(f: Family, style: BuildingStyle, ctx: &Ctx) -> Base {
    let table = bases(style, ctx.storeys);
    match f {
        Family::Shop => Base::ShopFront,
        Family::Apartment => {
            if style == BuildingStyle::HighEnd {
                Base::Colonnade
            } else {
                wpick(ctx.seed, 0x61, &[(Base::None, 3), (Base::ShopFront, 1), (Base::StonePlinth, 1)])
            }
        }
        Family::Office => wpick(ctx.seed, 0x62, table),
        Family::Civic => wpick(ctx.seed, 0x63, &[(Base::None, 2), (Base::Colonnade, 3), (Base::StonePlinth, 1), (Base::RaisedPlaza, 1)]),
        Family::Industrial => wpick(ctx.seed, 0x64, &[(Base::None, 3), (Base::StonePlinth, 1)]),
        _ => Base::None,
    }
}

/// The tallest crown each family may draw (its `max_height` in the catalogue).
fn crown_cap(f: Family, civic: CivicKind) -> i32 {
    match (f, civic) {
        (Family::House, _) => 56,
        (Family::Townhouse, _) => 90,
        (Family::Shop, _) => 82,
        (Family::Apartment, _) => 106,
        (Family::Office, _) => 144,
        (Family::Industrial, _) => 98,
        (Family::Civic, CivicKind::Hospital) => 124,
        (Family::Civic, _) => 112,
        (Family::Tower, _) => 296,
    }
}

/// Pick a low-rise or mid-rise design for a context.
pub(crate) fn building(ctx: &Ctx) -> Design {
    let seed = ctx.seed;
    let f = ctx.family;
    let t = trad(ctx.biome);
    let style = ctx.style;
    let era = ctx.era;

    let mut plan = wpick(seed, 0x11, &plans(f, &t));
    // A civic program has its own massing: a nave and a tower, a hall with a
    // drill tower, a portico'd block. The plan follows the program.
    if f == Family::Civic {
        plan = match ctx.civic {
            CivicKind::Church | CivicKind::FireStation | CivicKind::Library | CivicKind::TownHall => Plan::Box,
            CivicKind::School => {
                if matches!(plan, Plan::T | Plan::Cross | Plan::Courtyard | Plan::Stilts) {
                    Plan::Wing
                } else {
                    plan
                }
            }
            CivicKind::Hospital => Plan::Wing,
            CivicKind::None => plan,
        };
    }
    let roof_list = roofs(f, style, era, &t);
    // The street keeps a roof family (1 in 3 lots takes the street's form);
    // the lot decides the rest.
    let street_roof = (ctx.theme / 3) as usize % roof_list.len().max(1);
    let roof = if color::hash_seed(seed, 0x21) % 100 < 34 {
        roof_list[street_roof].0
    } else {
        wpick(seed, 0x22, &roof_list)
    };
    let roof_mat = wpick(seed, 0x23, roof_mats(f, &t));
    let roof_tint = (color::hash_seed(seed, 0x24) as usize % ROOF_TINTS.len()) as u8;

    let fin_list = finishes(f, style, &t);
    let finish = wpick(seed, 0x25, &fin_list);
    // Paint: the street's own colour about 45 % of the time, so a row reads as
    // one street and still no two houses are the same building.
    let pal = paints(finish);
    let n = pal.len().max(1);
    let theme_paint = ((ctx.theme as u32 >> 1) as usize) % n;
    let paint = if color::hash_seed(seed, 0x26) % 100 < 45 {
        theme_paint as u8
    } else {
        (color::hash_seed(seed, 0x27) as usize % n) as u8
    };

    // Extras. A house has a porch, a chimney, a garden; a block has none of
    // those but carries its own (a shop front, a works' sawtooth).
    let house_like = matches!(f, Family::House | Family::Townhouse);
    let porch = if house_like {
        wpick(seed, 0x31, t.porch)
    } else {
        0
    };
    let dormers = if house_like && !matches!(roof, RoofForm::Flat) {
        wpick(seed, 0x32, &[(0u8, 5), (1, 3), (2, 1)])
    } else if matches!(roof, RoofForm::Mansard) {
        1
    } else {
        0
    };
    let bay = if matches!(f, Family::House | Family::Townhouse | Family::Shop) {
        (color::hash_seed(seed, 0x33) % 100 < 40) as u8
    } else {
        0
    };
    let chimney = match f {
        Family::House => wpick(seed, 0x34, &[(0u8, 3), (1, 4), (2, 1)]),
        Family::Townhouse => wpick(seed, 0x35, &[(0u8, 4), (1, 3)]),
        Family::Industrial => 2,
        Family::Civic => {
            if ctx.civic == CivicKind::Church {
                1
            } else {
                0
            }
        }
        _ => 0,
    };
    let garage = if f == Family::House {
        wpick(seed, 0x36, &[(0u8, 4), (1, 3), (2, 2)])
    } else {
        0
    };
    let garden = if matches!(f, Family::House | Family::Townhouse) {
        wpick(seed, 0x37, t.garden)
    } else if f == Family::Civic {
        wpick(seed, 0x38, &[(0u8, 4), (5, 2), (1, 2)])
    } else {
        5
    };
    let fence = if house_like {
        wpick(seed, 0x39, t.fence)
    } else {
        0
    };

    // Proportions. A house stands back from the pavement (a front garden); a
    // block fills its lot; a works spreads across it.
    let fw = ctx.footprint.0.max(1) as f32;
    let fd = ctx.footprint.1.max(1) as f32;
    let (fx, fy) = match f {
        Family::House => (0.60 + 0.24 * color::hash01((seed as i32) ^ 0x11, 0x41), 0.55 + 0.24 * color::hash01((seed as i32) ^ 0x12, 0x42)),
        Family::Townhouse => (0.62 + 0.26 * color::hash01(seed as i32, 0x43), 0.62 + 0.24 * color::hash01(seed as i32, 0x44)),
        Family::Shop | Family::Apartment | Family::Office => (0.82 + 0.12 * color::hash01(seed as i32, 0x45), 0.78 + 0.14 * color::hash01(seed as i32, 0x46)),
        Family::Industrial => (0.62 + 0.10 * color::hash01(seed as i32, 0x47), 0.78 + 0.12 * color::hash01(seed as i32, 0x48)),
        Family::Civic => (0.86 + 0.10 * color::hash01(seed as i32, 0x47), 0.80 + 0.12 * color::hash01(seed as i32, 0x48)),
        Family::Tower => (0.94, 0.94),
    };
    let mut w = fw * fx;
    let mut d = fd * fy;
    if matches!(plan, Plan::Narrow) {
        w *= 0.62;
    }
    if f == Family::Civic && (ctx.civic == CivicKind::Church || ctx.civic == CivicKind::FireStation) {
        // The tower takes a third of the frontage in `volumes`; keep the nave
        // tall enough to be a nave.
        w = (w * 0.96).min(fw * 0.96);
    }
    if matches!(plan, Plan::Courtyard) {
        d *= 0.9;
    }
    w = w.max(0.45).min(fw * 0.98);
    d = d.max(0.45).min(fd * 0.98);

    let floor = match f {
        Family::House => 12,
        Family::Townhouse => 11,
        Family::Shop => 11,
        Family::Apartment => 10,
        Family::Office => 10,
        Family::Industrial => 13,
        Family::Civic => 11,
        Family::Tower => 4,
    };
    let mut drawn = match f {
        Family::House => {
            // A cottage is one storey, a two-storey house two, and the sim's
            // storey count is the lot's rank: it decides which.
            if matches!(plan, Plan::Split) {
                2
            } else if matches!(plan, Plan::Wing) || ctx.storeys >= 3 {
                2
            } else {
                1 + (color::hash_seed(seed, 0x51) % 2) as i32
            }
        }
        Family::Townhouse | Family::Shop => (ctx.storeys as i32).clamp(2, 4),
        Family::Apartment => (ctx.storeys as i32).clamp(3, 7),
        Family::Office => (ctx.storeys as i32).clamp(2, 7),
        Family::Industrial => (ctx.storeys as i32).clamp(1, 2),
        Family::Civic => match ctx.civic {
            CivicKind::Hospital => (ctx.storeys as i32).clamp(3, 5),
            CivicKind::School | CivicKind::TownHall => 2,
            CivicKind::Church => 3,
            _ => (ctx.storeys as i32).clamp(1, 3),
        },
        Family::Tower => 0,
    };

    // A block four storeys or more is a flat-roofed building: a pitched roof
    // over a mid-rise slab is not a thing a city has. The old core may still
    // wear a shallow shed roof.
    let roof = if matches!(f, Family::Apartment | Family::Shop | Family::Townhouse | Family::Office)
        && drawn >= 4
        && !matches!(roof, RoofForm::Flat | RoofForm::Shed)
    {
        RoofForm::Flat
    } else {
        roof
    };

    // The base band's height: a shop front or a colonnade is a storey of its own.
    let base_floors = match base_for(f, style, ctx) {
        Base::None => 0,
        Base::StonePlinth | Base::RaisedPlaza => 0,
        Base::ShopFront | Base::Colonnade | Base::GlassLobby | Base::MallScreen => 1,
        Base::Parking => 1,
    };
    let z_podium = base_floors * floor;

    // Height budget: the family's crown cap is the sprite box, so a tall lot in
    // a small box gives way (a drawn storey fewer) rather than being clipped.
    let mut z_top;
    let mut rise;
    let mut z_crown;
    // The land's pitch range, and this roof's pitch inside it.
    let pct = t.pitch.0 as f32
        + (t.pitch.1 - t.pitch.0) as f32 * color::hash01((seed as i32) ^ 0x61, 0x62);
    loop {
        z_top = z_podium + drawn * floor;
        let flat = matches!(roof, RoofForm::Flat);
        // A roof's rise is a proportion of its *wall*, and of the plan for a
        // cottage. A block spans a whole lot, so a span-based rise would put a
        // 50-pixel pyramid on a mid-rise: the block's roofs stay low.
        let span = w.min(d) * 32.0;
        let pitched = (span * pct / 100.0) as i32;
        rise = if flat {
            4
        } else if matches!(roof, RoofForm::Mansard) {
            (pitched * 7 / 10).clamp(7, 20)
        } else if matches!(f, Family::House) {
            pitched.clamp(5, 26)
        } else if matches!(f, Family::Townhouse | Family::Shop) {
            pitched.clamp(5, 16)
        } else {
            pitched.clamp(4, 11)
        };
        z_crown = z_top + rise;
        if z_crown <= crown_cap(f, ctx.civic) || drawn <= 1 {
            break;
        }
        drawn -= 1;
    }

    // Mid-rise and commercial walls: the tower kit's facade systems, so a block
    // reads as part of the same city.
    let facade = if matches!(finish, Finish::Glass) {
        Facade::Curtain
    } else if matches!(f, Family::House | Family::Townhouse) {
        Facade::BrickTrim
    } else {
        wpick(seed, 0x71, facades(style))
    };
    let material = match finish {
        // The wall palette of a low-rise is its own; the material palette is
        // what the facade painter and the trim use (glass for a glass box).
        Finish::Glass => Material::GlassBlue,
        Finish::Brick => Material::RedBrick,
        Finish::Stone => Material::Limestone,
        Finish::Concrete => Material::Concrete,
        Finish::Panel => Material::WhitePanel,
        Finish::Corrugated | Finish::Log | Finish::Timber => Material::SilverSteel,
        Finish::Adobe | Finish::Stucco => Material::Sandstone,
        Finish::Clapboard | Finish::Shingle => Material::WhitePanel,
    };
    let material = if matches!(f, Family::House | Family::Townhouse) {
        material
    } else {
        pick_material_weighted(seed, style, era, facade, material)
    };
    let top = if matches!(roof, RoofForm::Flat) && !matches!(f, Family::House) {
        pick_roof_kit(seed, style, f, material)
    } else {
        Top::FlatPlant
    };
    Design {
        family: f,
        base: base_for(f, style, ctx),
        body: body_of(f),
        massing: Massing::None,
        facade,
        top,
        material,
        storeys: ctx.storeys,
        w,
        d,
        bw: fw,
        bd: fd,
        floor,
        z_podium,
        // A house stands back from the street for its front garden; a block
        // fills its lot. The draw uses these to place the volumes.
        z_top,
        z_crown,
        seed,
        age: ctx.age,
        crown_lights: false,
        crown_pal: 0,
        led_edge: false,
        civic: ctx.civic,
        plan,
        roof,
        roof_mat,
        roof_tint,
        finish,
        paint,
        porch,
        dormers,
        bay,
        chimney,
        garage,
        garden,
        fence,
        eaves: t.eaves,
        pitch: pct.round().clamp(0.0, 255.0) as u8,
        wall: pal[paint as usize % n] as usize,
    }
}

fn body_of(f: Family) -> Body {
    match f {
        Family::House | Family::Townhouse => Body::Square,
        Family::Shop | Family::Apartment | Family::Office => Body::Slab,
        Family::Industrial => Body::Square,
        Family::Civic => Body::Square,
        Family::Tower => Body::Square,
    }
}

/// A material that wears the picked facade, keeping the district's own bias.
fn pick_material_weighted(
    seed: u64,
    style: BuildingStyle,
    era: Era,
    facade: Facade,
    fallback: Material,
) -> Material {
    let all: Vec<(Material, u32)> = materials(style, era)
        .iter()
        .copied()
        .filter(|m| wears(facade, *m))
        .map(|m| (m, material_weight(style, m)))
        .collect();
    if all.len() <= 1 {
        return fallback;
    }
    let m = wpick(seed, 0x81, &all);
    // Keep a nod to the cladding where it is compatible.
    if wears(facade, fallback) && color::hash_seed(seed, 0x82) % 100 < 35 {
        fallback
    } else {
        m
    }
}

/// The rooftop kit on a flat roof (the tower kit's own tops, filtered to the
/// domestic kinds a low block would carry).
fn pick_roof_kit(seed: u64, style: BuildingStyle, f: Family, m: Material) -> Top {
    let table = tops(style, m, Body::Square, 6);
    let ok: Vec<(Top, u32)> = table
        .into_iter()
        .filter(|(t, _)| {
            matches!(
                t,
                Top::FlatPlant | Top::WaterTank | Top::RoofGarden | Top::AntennaCluster | Top::CoolingTowers | Top::Helipad | Top::LightBox
            ) && match f {
                Family::Industrial => !matches!(t, Top::Helipad | Top::LightBox | Top::RoofGarden),
                Family::Civic => !matches!(t, Top::Helipad | Top::LightBox),
                _ => !matches!(t, Top::CoolingTowers | Top::Helipad),
            }
        })
        .collect();
    if ok.is_empty() {
        Top::FlatPlant
    } else {
        wpick(seed, 0x91, &ok)
    }
}
