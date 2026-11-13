//! B73b/B73c: the building kit.
//!
//! A building is not a hand-written archetype any more: it is a kit of parts
//! combined from the lot's seed — a **base** (podium / plinth), a **body** (plan
//! and massing), a **facade** system, a **top** (roof or crown) and a
//! **material** — chosen within what the district, the era and the land allow.
//! The 16 hand-written tower designs in [`crate::catalogue::towers`] stay as
//! rare *signature* towers; everything else comes from here.
//!
//! The module is written as a **general** kit so B73c can reuse it:
//!
//! * [`Base`], [`Body`], [`Massing`], [`Facade`], [`Top`], [`Material`] and
//!   [`Design`] are family-neutral — a house would be a base of none, a body of
//!   one storey, a pitched top and a cladding facade. Only [`Family::Tower`] is
//!   populated by this brief: B73c adds its family's row to the rule tables
//!   ([`bases`], [`bodies`], [`facades`], [`tops`], [`pick_material`]) and its
//!   arm in [`crate::genkit::tower`]'s sibling draw module. The picker, the
//!   district/era/land gates, the neighbour rule ([`design_of`] +
//!   [`nudge_seed`]) and the memory approach in [`crate::lots`] work unchanged.
//! * The pick is a pure function of a [`BuildingSpec`] through [`Ctx`]: the same
//!   lot always gives the same building, so the sprite cache can key on it and
//!   two frames of the same world never disagree.
//! * [`Design::key`] is the triple a touching neighbour may not share.

pub mod building;
pub mod paint;
pub mod pick;
pub mod roofs;
pub mod tower;

#[cfg(test)]
mod tests;

use crate::color;
use crate::style::BiomeStyle;
use crate::types::{BuildingFunction, BuildingSpec, BuildingStyle, BuildingType, Era, Light, Sprite};

// -----------------------------------------------------------------------------
// The parts
// -----------------------------------------------------------------------------

/// What family of building a design is. A tower is B73b's; every other family
/// is B73c's, and each brings its own rule rows ([`pick`]) and its own draw
/// ([`building`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Tower,
    House,
    Townhouse,
    Shop,
    Apartment,
    Office,
    Industrial,
    Civic,
}

impl Family {
    /// The function the sim uses for this family (the report groups by it).
    pub fn function(self) -> BuildingFunction {
        match self {
            Family::Tower => BuildingFunction::Tower,
            Family::House => BuildingFunction::House,
            Family::Townhouse => BuildingFunction::Townhouse,
            Family::Shop => BuildingFunction::Shop,
            Family::Apartment => BuildingFunction::Apartment,
            Family::Office => BuildingFunction::Office,
            Family::Industrial => BuildingFunction::Industrial,
            Family::Civic => BuildingFunction::Civic,
        }
    }
}

/// Which civic program a generated civic building keeps (brief §2: a school has
/// a yard, a fire station has bays and a tower — the program is not optional,
/// only the architecture is generated).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CivicKind {
    None,
    School,
    FireStation,
    Library,
    TownHall,
    Hospital,
    Church,
}

/// The plan of a low-rise volume: how the mass sits on its lot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Plan {
    /// One box.
    Box,
    /// A narrow frontage (a terrace house, a city infill).
    Narrow,
    /// A main volume with a wing.
    L,
    /// A main volume with a wing across the back (a T).
    T,
    /// A plus plan.
    Cross,
    /// A split-level: a low volume beside a tall one.
    Split,
    /// A big volume with a low hall or garage beside it.
    Wing,
    /// A walled courtyard (the desert and savanna idiom).
    Courtyard,
    /// A raised floor on posts (stilts, decks, swamps).
    Stilts,
    /// A shop with an arcade through it.
    Arcade,
}

/// The form of a roof. The numbers the picker fills (`rise`, eave) come from the
/// land's tradition, so a steep alpine pitch and a shallow desert hip are the
/// same form with different numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoofForm {
    /// A ridge along one axis, gable ends.
    Gable,
    /// A main gable with a second one across the front.
    GableCross,
    /// Four slopes to a short ridge.
    Hip,
    /// A barn gambrel: two slopes a side.
    Gambrel,
    /// A steep lower slope and a shallow top (the French block).
    Mansard,
    /// A gable with the ridge pushed to one side.
    Saltbox,
    /// One slope (a lean-to, a modern shed).
    Shed,
    /// A gable whose ends are clipped by a small hip.
    HalfHip,
    /// Four slopes to a point (a pavilion, a tower's cap).
    Pyramid,
    /// A parapet and a flat deck.
    Flat,
}

/// What a roof is covered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoofMat {
    /// Asphalt or cedar shingles, staggered courses.
    Shingle,
    /// Slate, fine courses.
    Slate,
    /// Clay or concrete tiles, ribbed.
    Tile,
    /// Standing-seam or corrugated metal.
    Metal,
    /// Straw / palm thatch.
    Thatch,
    /// A built-up flat roof.
    Membrane,
}

/// How a wall is clad. Each finish has a painter in [`building`], and its own
/// small palette ([`pick::paints`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Finish {
    /// Painted horizontal boards.
    Clapboard,
    /// Wall shingles.
    Shingle,
    Brick,
    Stone,
    /// Render / plaster.
    Stucco,
    /// Plaster between dark timber posts (fachwerk).
    Timber,
    /// Round log courses.
    Log,
    /// Sun-baked earth (adobe / mud brick).
    Adobe,
    /// Profiled metal sheet (sheds, works, favelas' better cousin).
    Corrugated,
    /// Flat metal or composite panels (modern low-rise).
    Panel,
    Concrete,
    /// Glass (a modern pavilion).
    Glass,
}


/// The bottom of a building: a podium, a plinth, a raised plaza (brief §2, ≥ 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Base {
    /// The body comes straight to the pavement.
    None,
    /// A colonnade: columns and a deep recessed lobby.
    Colonnade,
    /// Shop fronts with a fascia sign band.
    ShopFront,
    /// A two-storey glass lobby that glows warm at night.
    GlassLobby,
    /// A heavy rusticated stone plinth.
    StonePlinth,
    /// A car-park podium: open decks, cars visible.
    Parking,
    /// A retail block with a big bright screen (D80's bright square).
    MallScreen,
    /// A raised plaza with steps and planting.
    RaisedPlaza,
}

/// The plan of the body (brief §2, ≥ 12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Body {
    Square,
    /// A thin slab, broad across one axis.
    Slab,
    /// A square with its corners cut.
    Chamfered,
    /// An octagonal plan.
    Octagon,
    /// A round drum.
    Drum,
    /// An L-plan: a main volume with a wing.
    LPlan,
    /// A cross-plan: a plus-shaped mass.
    Cross,
    /// The plan narrows with height.
    Tapered,
    /// The floors rotate a little per band.
    Twisting,
    /// Twin volumes joined by a sky bridge.
    Twin,
    /// Offset stacked boxes (cantilevers).
    Stacked,
    /// A wedge: one face leans back to a sloped top.
    Wedge,
}

/// How the body steps as it rises (brief §2, ≥ 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Massing {
    None,
    /// Art-deco wedding cake: several steps, each smaller.
    WeddingCake,
    /// One big step near the top.
    OneStep,
    /// Corners step in on alternate bands.
    Staggered,
    /// A garden gap: an open planted floor between two masses.
    SkyGarden,
    /// A smooth taper over many small steps.
    Tapering,
}

/// The wall system (brief §2, ≥ 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Facade {
    /// Curtain wall glass, tinted by the design's material.
    Curtain,
    /// Punched windows in ashlar stone.
    PunchedStone,
    /// Brick with stone trim and lintels.
    BrickTrim,
    /// Horizontal ribbon windows.
    StripWindows,
    /// Vertical fins over glass.
    VerticalFins,
    /// Horizontal louvres over glass.
    Louvres,
    /// Residential balconies.
    Balconies,
    /// A diagrid of crossing steel.
    Diagrid,
    /// An exoskeleton: columns and X-bracing outside the glass.
    Exoskeleton,
    /// Brutalist concrete grid with deep reveals.
    Brutalist,
    /// A sawtooth of angled panels.
    Sawtooth,
    /// Terracotta baguette panels.
    Terracotta,
    /// A perforated metal screen over the glass.
    Perforated,
    /// A checker of glass and solid panels.
    Checker,
}

/// The top of a building: a roof, or a tower's crown (brief §2, ≥ 15).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Top {
    /// Parapet, plant boxes and a stair head.
    FlatPlant,
    /// A water tank on stilts (the old masonry towers).
    WaterTank,
    /// A stepped deco crown with a spire.
    DecoSpire,
    /// A closed pyramid cap.
    PyramidCap,
    /// A needle mast.
    NeedleMast,
    /// A rooftop helipad.
    Helipad,
    /// A glowing lantern.
    Lantern,
    /// An open steel lattice.
    OpenLattice,
    /// A sloped glass cap.
    SlopedGlass,
    /// A stepped ziggurat.
    Ziggurat,
    /// A planted roof garden.
    RoofGarden,
    /// Two cooling towers.
    CoolingTowers,
    /// A lit ring around the top.
    CrownRing,
    /// A cluster of antennae.
    AntennaCluster,
    /// A lit sign box with invented glyphs (B73's signs).
    LightBox,
}

/// The material palette of a design (walls, glass, metal).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Material {
    Limestone,
    Sandstone,
    RedBrick,
    BuffBrick,
    Brownstone,
    Concrete,
    WhitePanel,
    SilverSteel,
    Champagne,
    Bronze,
    BlackGranite,
    Copper,
    GlassBlue,
    GlassGreen,
    GlassBronze,
    GlassGrey,
}

// -----------------------------------------------------------------------------
// The design
// -----------------------------------------------------------------------------

/// What a touching neighbour may not share (brief §3). A tower keeps the
/// original shaft+facade+material triple (the rest of the fields stay at their
/// tower defaults); a house adds its plan, roof, cladding and paint, so two
/// neighbours can never be the same building.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DesignKey {
    pub body: Body,
    pub facade: Facade,
    pub material: Material,
    pub plan: Plan,
    pub roof: RoofForm,
    pub finish: Finish,
    pub paint: u8,
}

/// Everything about one generated building that the draw needs.
#[derive(Clone, Copy, Debug)]
pub struct Design {
    pub family: Family,
    pub base: Base,
    pub body: Body,
    pub massing: Massing,
    pub facade: Facade,
    pub top: Top,
    pub material: Material,
    /// The lot's own height in storeys (the sim's skyline decides it).
    pub storeys: u8,
    /// The body's plan in tiles (a tower is slender; the plan follows its
    /// height, not its lot).
    pub w: f32,
    pub d: f32,
    /// The base's plan: the lot the building stands on (the body is centred on
    /// it and stands back from its edge).
    pub bw: f32,
    pub bd: f32,
    /// Pixels per storey, varied a little per design.
    pub floor: i32,
    /// Height of the base in pixels above the pavement.
    pub z_podium: i32,
    /// Where the body ends and the top begins.
    pub z_top: i32,
    /// Total height including the top, kept under the sprite box.
    pub z_crown: i32,
    /// The lot seed, so every part can hash its own variation.
    pub seed: u64,
    /// Weathering (older districts).
    pub age: f32,
    /// Does this tower's crown carry lamps at night? Most towers are windows
    /// only (D80/B73 q9): the crown is architecture by day and dark by night
    /// unless the design says otherwise.
    pub crown_lights: bool,
    /// Which crown palette: 0 warm white, 1 gold, 2 red, 3 green, 4 blue,
    /// 5 two-tone, 6 cool silver.
    pub crown_pal: u8,
    /// A thin LED line up the tower's corner at night (modern glass only).
    pub led_edge: bool,

    // ---- B73c: the low-rise families -------------------------------------
    /// Which civic program (a school's yard, a fire station's bays).
    pub civic: CivicKind,
    /// The plan of the volume(s).
    pub plan: Plan,
    /// The roof's form.
    pub roof: RoofForm,
    /// What the roof is covered with.
    pub roof_mat: RoofMat,
    /// Which roof colour (index into [`pick::ROOF_TINTS`]).
    pub roof_tint: u8,
    /// How the walls are clad.
    pub finish: Finish,
    /// Which paint of that finish's palette.
    pub paint: u8,
    /// Front feature: 0 none, 1 stoop/steps, 2 porch, 3 veranda.
    pub porch: u8,
    /// Dormers on the near slope (0-2).
    pub dormers: u8,
    /// A projecting bay window (0-1).
    pub bay: u8,
    /// 0 no chimney, 1 a chimney stack, 2 a tall mill chimney.
    pub chimney: u8,
    /// 0 none, 1 attached garage, 2 detached garage or carport.
    pub garage: u8,
    /// What the lot's garden is: 0 lawn, 1 flowers, 2 vegetables, 3 gravel,
    /// 4 pool, 5 paved yard.
    pub garden: u8,
    /// The lot's edge: 0 none, 1 low wall, 2 hedge, 3 picket fence.
    pub fence: u8,
    /// Eave overhang in tiles (alpine and tundra are deep, desert is thin).
    pub eaves: u8,
    /// Roof pitch as a percentage of the roof's short side (35-105).
    pub pitch: u8,
    /// The wall colour the design's cladding paints (0 = the material's own).
    /// B73c: the facade painters take the material's palette, so a *painted*
    /// brick house still wears its own colour through this.
    pub wall: usize,
}

/// The crown-lamp palettes, two colours each (the second is the accent row or
/// the second face of a two-tone crown). The same families as B73's business
/// district crowns, so a baked accent and an animated band agree.
pub const CROWN_PALETTES: [[render::palette::Color; 2]; 7] = [
    [0xFFFF_E8C8, 0xFFFF_F2DC], // warm white
    [0xFFE8_C84C, 0xFFFF_DC7E], // gold
    [0xFFD8_4038, 0xFFFF_6A5A], // red
    [0xFF48_B070, 0xFF8C_E0A8], // green
    [0xFF58_90E0, 0xFF9C_C4FF], // blue
    [0xFFFF_6A5A, 0xFF58_90E0], // two-tone: red over blue
    [0xFFC8_D8E8, 0xFFFF_FFFF], // cool silver
];

impl Design {
    /// This design's crown lamp: the palette colour at the evening's level,
    /// tagged emissive. `accent` picks the second colour (two-tone crowns, the
    /// hot bulb row).
    pub fn crown_lamp(&self, light: &Light, accent: bool) -> render::palette::Color {
        let n = light.night.clamp(0.0, 1.0);
        let c = CROWN_PALETTES[(self.crown_pal as usize) % CROWN_PALETTES.len()][accent as usize];
        let level = 0.42 + 0.58 * n;
        let (r, g, b) = (
            ((c >> 16) & 0xFF) as f32,
            ((c >> 8) & 0xFF) as f32,
            (c & 0xFF) as f32,
        );
        render::palette::with_alpha(
            render::palette::rgb(
                (r * level) as u8,
                (g * level) as u8,
                (b * level) as u8,
            ),
            0xFE,
        )
    }
}

impl Design {
    pub fn key(&self) -> DesignKey {
        DesignKey {
            body: self.body,
            facade: self.facade,
            material: self.material,
            plan: self.plan,
            roof: self.roof,
            finish: self.finish,
            paint: self.paint,
        }
    }
}

/// The picker's input: everything a lot knows about itself.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    pub family: Family,
    pub civic: CivicKind,
    pub style: BuildingStyle,
    pub era: Era,
    pub biome: BiomeStyle,
    pub footprint: (u8, u8),
    pub storeys: u8,
    pub age: f32,
    pub seed: u64,
    /// The street's own character (a coarser hash than the lot's seed), so a row
    /// of houses shares a paint and a roof family but no two are the same
    /// building (brief §4).
    pub theme: u8,
}

impl Ctx {
    /// A lot's context, for any family.
    fn of(spec: &BuildingSpec) -> Self {
        let (family, civic) = family_of(spec);
        Self {
            family,
            civic,
            style: spec.style,
            era: spec.era,
            biome: spec.biome,
            footprint: spec.footprint,
            storeys: if family == Family::Tower {
                spec.storeys.max(6)
            } else {
                spec.storeys.max(1)
            },
            age: spec.age,
            seed: spec.seed,
            theme: spec.variant,
        }
    }
}

/// Which family (and civic program) a spec draws from.
pub fn family_of(spec: &BuildingSpec) -> (Family, CivicKind) {
    use BuildingType as T;
    let civic = match spec.building_type {
        Some(T::CivicSchool) => CivicKind::School,
        Some(T::CivicFireStation) => CivicKind::FireStation,
        Some(T::CivicLibrary) => CivicKind::Library,
        Some(T::CivicTownHall) => CivicKind::TownHall,
        Some(T::CivicHospital) => CivicKind::Hospital,
        Some(T::CivicChurch) => CivicKind::Church,
        _ => CivicKind::None,
    };
    let family = match spec.function {
        BuildingFunction::Tower => Family::Tower,
        BuildingFunction::House => Family::House,
        BuildingFunction::Townhouse => Family::Townhouse,
        BuildingFunction::Shop => Family::Shop,
        BuildingFunction::Apartment => Family::Apartment,
        BuildingFunction::Office => Family::Office,
        BuildingFunction::Industrial => Family::Industrial,
        BuildingFunction::Civic => Family::Civic,
        // A landmark is hand-made art, never a generated family.
        BuildingFunction::Landmark => return (Family::Tower, civic),
    };
    (family, civic)
}

// -----------------------------------------------------------------------------
// Seeded picks with weights
// -----------------------------------------------------------------------------

/// Pick from a weighted table with the seed's own hash (never the frame's).
pub(crate) fn wpick<T: Copy>(seed: u64, salt: u32, items: &[(T, u32)]) -> T {
    let total: u32 = items.iter().map(|(_, w)| *w).sum();
    if total == 0 {
        return items[0].0;
    }
    let mut r = color::hash_seed(seed, salt) % total;
    for (v, w) in items {
        if r < *w {
            return *v;
        }
        r -= *w;
    }
    items[0].0
}

pub(crate) fn bases(style: BuildingStyle, storeys: u8) -> &'static [(Base, u32)] {
    match style {
        // The business district: shop fronts, glass lobbies, a mall block.
        BuildingStyle::ModernGlass => {
            if storeys >= 14 {
                &[
                    (Base::None, 2),
                    (Base::ShopFront, 3),
                    (Base::GlassLobby, 2),
                    (Base::MallScreen, 1),
                    (Base::Parking, 1),
                    (Base::RaisedPlaza, 1),
                ]
            } else {
                &[(Base::None, 2), (Base::ShopFront, 2), (Base::GlassLobby, 1)]
            }
        }
        // The old core and the high street: stone, columns, shops.
        BuildingStyle::BrickMainStreet | BuildingStyle::European => &[
            (Base::None, 1),
            (Base::Colonnade, 2),
            (Base::StonePlinth, 2),
            (Base::ShopFront, 2),
        ],
        // High end: plazas and lobbies, never a car park.
        BuildingStyle::HighEnd => &[
            (Base::None, 2),
            (Base::RaisedPlaza, 2),
            (Base::GlassLobby, 2),
            (Base::Colonnade, 1),
        ],
        // Works: parking decks and plinths.
        BuildingStyle::Industrial => &[(Base::None, 2), (Base::Parking, 2), (Base::StonePlinth, 1)],
        BuildingStyle::NewEngland | BuildingStyle::Suburban => {
            &[(Base::None, 2), (Base::ShopFront, 1), (Base::StonePlinth, 1)]
        }
    }
}

pub(crate) fn bodies(style: BuildingStyle) -> &'static [(Body, u32)] {
    match style {
        BuildingStyle::ModernGlass => &[
            (Body::Square, 4),
            (Body::Slab, 3),
            (Body::Chamfered, 2),
            (Body::Octagon, 1),
            (Body::Drum, 1),
            (Body::Tapered, 2),
            (Body::Twisting, 1),
            (Body::Twin, 1),
            (Body::Stacked, 2),
            (Body::Wedge, 1),
            (Body::LPlan, 1),
            (Body::Cross, 1),
        ],
        BuildingStyle::HighEnd => &[
            (Body::Square, 3),
            (Body::Chamfered, 2),
            (Body::Octagon, 2),
            (Body::Twin, 1),
            (Body::Tapered, 2),
            (Body::Stacked, 1),
            (Body::Wedge, 1),
            (Body::Drum, 1),
        ],
        BuildingStyle::BrickMainStreet | BuildingStyle::European => &[
            (Body::Square, 4),
            (Body::Chamfered, 2),
            (Body::Octagon, 1),
            (Body::Slab, 1),
            (Body::Tapered, 2),
            (Body::Stacked, 1),
            (Body::Wedge, 1),
        ],
        BuildingStyle::Industrial => &[
            (Body::Square, 3),
            (Body::Slab, 2),
            (Body::Wedge, 1),
            (Body::LPlan, 2),
            (Body::Cross, 1),
            (Body::Stacked, 1),
        ],
        BuildingStyle::NewEngland | BuildingStyle::Suburban => {
            &[(Body::Square, 3), (Body::Slab, 1), (Body::Chamfered, 1), (Body::Tapered, 1)]
        }
    }
}

pub(crate) fn massings(style: BuildingStyle, era: Era, storeys: u8) -> &'static [(Massing, u32)] {
    let old = matches!(style, BuildingStyle::BrickMainStreet | BuildingStyle::European);
    match (old, era) {
        // Masonry: the wedding cake and the big step are the idiom.
        (true, _) => &[
            (Massing::None, 2),
            (Massing::WeddingCake, 3),
            (Massing::OneStep, 2),
            (Massing::Staggered, 1),
            (Massing::Tapering, 1),
        ],
        // Glass: sky gardens and tapers.
        _ if storeys >= 18 => &[
            (Massing::None, 3),
            (Massing::OneStep, 2),
            (Massing::SkyGarden, 2),
            (Massing::Tapering, 2),
            (Massing::Staggered, 1),
        ],
        _ => &[(Massing::None, 3), (Massing::OneStep, 2), (Massing::Tapering, 2)],
    }
}

pub(crate) fn facades(style: BuildingStyle) -> &'static [(Facade, u32)] {
    match style {
        BuildingStyle::ModernGlass => &[
            (Facade::Curtain, 6),
            (Facade::VerticalFins, 3),
            (Facade::Diagrid, 2),
            (Facade::Exoskeleton, 2),
            (Facade::Louvres, 2),
            (Facade::StripWindows, 2),
        ],
        BuildingStyle::HighEnd => &[
            (Facade::Curtain, 3),
            (Facade::PunchedStone, 2),
            (Facade::StripWindows, 2),
            (Facade::VerticalFins, 2),
            (Facade::Checker, 1),
            (Facade::Balconies, 1),
        ],
        BuildingStyle::BrickMainStreet | BuildingStyle::European => &[
            (Facade::PunchedStone, 3),
            (Facade::BrickTrim, 3),
            (Facade::Balconies, 2),
            (Facade::StripWindows, 1),
            (Facade::Terracotta, 1),
            (Facade::Checker, 1),
        ],
        BuildingStyle::Industrial => &[
            (Facade::Brutalist, 3),
            (Facade::Sawtooth, 2),
            (Facade::Perforated, 2),
            (Facade::PunchedStone, 1),
            (Facade::StripWindows, 1),
        ],
        BuildingStyle::NewEngland | BuildingStyle::Suburban => &[
            (Facade::BrickTrim, 2),
            (Facade::Balconies, 2),
            (Facade::PunchedStone, 1),
            (Facade::StripWindows, 1),
        ],
    }
}

/// The materials a district builds in (brief §2: palettes per era and land).
pub(crate) fn materials(style: BuildingStyle, era: Era) -> &'static [Material] {
    let modern = !matches!(era, Era::Settlement | Era::Village | Era::Town);
    match style {
        BuildingStyle::ModernGlass if modern => &[
            Material::GlassBlue,
            Material::GlassGreen,
            Material::GlassGrey,
            Material::GlassBronze,
            Material::SilverSteel,
            Material::WhitePanel,
            Material::Bronze,
            Material::BlackGranite,
            Material::Champagne,
        ],
        BuildingStyle::ModernGlass => &[
            Material::WhitePanel,
            Material::Limestone,
            Material::GlassBlue,
            Material::SilverSteel,
        ],
        BuildingStyle::HighEnd => &[
            Material::Limestone,
            Material::Champagne,
            Material::WhitePanel,
            Material::Sandstone,
            Material::BlackGranite,
            Material::Copper,
        ],
        BuildingStyle::BrickMainStreet => &[
            Material::RedBrick,
            Material::BuffBrick,
            Material::Brownstone,
            Material::Limestone,
            Material::Sandstone,
        ],
        BuildingStyle::European => &[
            Material::Sandstone,
            Material::Limestone,
            Material::BuffBrick,
            Material::RedBrick,
            Material::Copper,
        ],
        BuildingStyle::Industrial => {
            &[Material::Concrete, Material::SilverSteel, Material::BlackGranite, Material::Bronze]
        }
        BuildingStyle::NewEngland => &[Material::RedBrick, Material::BuffBrick, Material::WhitePanel],
        BuildingStyle::Suburban => &[
            Material::WhitePanel,
            Material::RedBrick,
            Material::BuffBrick,
            Material::Concrete,
        ],
    }
}

/// What a facade system can be built in. The picker chooses the facade first,
/// then a material that agrees with it, so a brick brick-trim tower and a
/// glass exoskeleton are both possible and a "brick curtain wall" is not.
pub(crate) fn wears(f: Facade, m: Material) -> bool {
    let glass = matches!(
        m,
        Material::GlassBlue | Material::GlassGreen | Material::GlassGrey | Material::GlassBronze
    );
    let metal = matches!(
        m,
        Material::WhitePanel | Material::SilverSteel | Material::Champagne | Material::Bronze
    );
    let stone = matches!(
        m,
        Material::Limestone | Material::Sandstone | Material::Brownstone | Material::BlackGranite
    );
    let masonry = matches!(m, Material::RedBrick | Material::BuffBrick);
    match f {
        Facade::Curtain => glass || metal || stone || m == Material::Concrete,
        Facade::PunchedStone => stone || masonry || m == Material::Concrete,
        Facade::BrickTrim => masonry || matches!(m, Material::Limestone | Material::Concrete),
        Facade::StripWindows => true,
        Facade::VerticalFins => glass || metal || stone,
        Facade::Louvres => glass || metal || stone,
        Facade::Balconies => masonry || metal || m == Material::Concrete,
        Facade::Diagrid => glass || metal || stone,
        Facade::Exoskeleton => glass || metal || stone,
        Facade::Brutalist => m == Material::Concrete || stone,
        Facade::Sawtooth => m == Material::Concrete || metal || masonry,
        Facade::Terracotta => masonry || m == Material::Copper || stone,
        Facade::Perforated => metal || stone,
        // A checker of glass and solid panels needs both halves: the metal,
        // stone and masonry materials supply the solid one.
        Facade::Checker => metal || stone || masonry || m == Material::Concrete,
    }
}

pub(crate) fn tops(style: BuildingStyle, m: Material, body: Body, storeys: u8) -> Vec<(Top, u32)> {
    let table: &[(Top, u32)] = match style {
        BuildingStyle::Industrial => &[
            (Top::FlatPlant, 3),
            (Top::CoolingTowers, 2),
            (Top::AntennaCluster, 1),
            (Top::WaterTank, 1),
            (Top::RoofGarden, 1),
        ],
        BuildingStyle::BrickMainStreet | BuildingStyle::European => &[
            (Top::FlatPlant, 3),
            (Top::WaterTank, 2),
            (Top::DecoSpire, 2),
            (Top::PyramidCap, 1),
            (Top::Ziggurat, 1),
            (Top::RoofGarden, 1),
            (Top::AntennaCluster, 1),
        ],
        BuildingStyle::HighEnd => &[
            (Top::FlatPlant, 2),
            (Top::DecoSpire, 2),
            (Top::Lantern, 2),
            (Top::PyramidCap, 1),
            (Top::Ziggurat, 1),
            (Top::RoofGarden, 2),
            (Top::CrownRing, 1),
            (Top::NeedleMast, 1),
            (Top::LightBox, 1),
        ],
        _ => &[
            (Top::FlatPlant, 2),
            (Top::Lantern, 2),
            (Top::SlopedGlass, 1),
            (Top::CrownRing, 1),
            (Top::AntennaCluster, 1),
            (Top::Helipad, 1),
            (Top::OpenLattice, 1),
            (Top::NeedleMast, 1),
            (Top::PyramidCap, 1),
            (Top::RoofGarden, 1),
            (Top::LightBox, 1),
        ],
    };
    let glass = matches!(
        m,
        Material::GlassBlue | Material::GlassGreen | Material::GlassGrey | Material::GlassBronze
    );
    let metal = matches!(
        m,
        Material::WhitePanel | Material::SilverSteel | Material::Champagne | Material::Bronze
    );
    let stone = matches!(
        m,
        Material::Limestone | Material::Sandstone | Material::Brownstone | Material::BlackGranite
    );
    let big = storeys >= 20;
    let _ = (glass, metal, stone);
    table
        .iter()
        .map(|&(t, w)| {
            // The height/plan compatibility rule: a helipad needs a flat top and
            // a tall enough tower; a sloped cap wants a square-ish body; a lit
            // sign box is a downtown/high-end thing.
            let ok = match t {
                Top::Helipad => big && !matches!(body, Body::Wedge | Body::Twisting),
                // The sloped-glass "sail" is a signature look: only a tall
                // modern glass tower gets one, and rarely (round 2, Q5).
                Top::SlopedGlass => glass && storeys >= 24 && !matches!(body, Body::Wedge | Body::Drum),
                Top::PyramidCap => !matches!(body, Body::Wedge | Body::Drum),
                Top::LightBox => matches!(style, BuildingStyle::ModernGlass | BuildingStyle::HighEnd),
                Top::OpenLattice | Top::CrownRing | Top::NeedleMast | Top::AntennaCluster
                | Top::DecoSpire => big,
                Top::WaterTank => stone || !glass,
                _ => true,
            };
            (t, if ok { w } else { 0 })
        })
    .filter(|(_, w)| *w > 0)
    .collect()
}

// -----------------------------------------------------------------------------
// The pick
// -----------------------------------------------------------------------------

/// Pick a design for a context: a tower from the tower kit (B73b), or a
/// low-rise building from its family's kit (B73c).
pub fn pick(ctx: &Ctx) -> Design {
    match ctx.family {
        Family::Tower => pick_tower(ctx),
        _ => pick::building(ctx),
    }
}

fn pick_tower(ctx: &Ctx) -> Design {
    let seed = ctx.seed;
    let base = wpick(seed, 11, bases(ctx.style, ctx.storeys));
    let body = wpick(seed, 21, bodies(ctx.style));
    // A twisting, twin or stacked body carries its own massing; a setback rule
    // on top of it would fight it.
    let massing = if matches!(body, Body::Twisting | Body::Twin | Body::Stacked | Body::Drum | Body::Wedge) {
        Massing::None
    } else {
        wpick(seed, 31, massings(ctx.style, ctx.era, ctx.storeys))
    };
    let facade = wpick(seed, 41, facades(ctx.style));
    let material = pick_material(seed, ctx.style, ctx.era, facade);
    // A wedge's roof is its own top: it slopes, so a crown would have to float
    // over the low end of it.
    let top = if matches!(body, Body::Wedge) {
        Top::FlatPlant
    } else {
        pick_top(seed, ctx, material, body)
    };

    // Footprint: a tower is slender — its plan follows its height, not its lot,
    // and a big lot becomes a podium or a plaza around it. Either way it never
    // grows past the lot, so every tower draws inside its footprint.
    let fw = ctx.footprint.0.max(1) as f32;
    let fd = ctx.footprint.1.max(1) as f32;
    let slender = (0.55 + ctx.storeys as f32 * 0.022).min(1.35);
    let mut w = slender.min(fw * 0.94);
    let mut d = (slender * 0.92).min(fd * 0.90);
    match body {
        Body::Slab => {
            d = (w * 1.55).min(fd * 0.95);
            w = (d * 0.62).min(fw * 0.94);
        }
        Body::Drum | Body::Octagon => {
            let side = w.min(d);
            w = side;
            d = side;
        }
        _ => {}
    }
    // The base reaches a little past the body (a pavement's width); the lot
    // itself is the plaza the tower stands on, paved when there is room (Q4).
    let bw = fw * 0.98;
    let bd = fd * 0.96;

    // A podium reads as a podium only if the body stands back from it.
    if !matches!(base, Base::None) {
        w = (w - 0.04).max(0.45);
        d = (d - 0.04).max(0.45);
    }
    // Height: the lot's storeys are the city's *rank*, not a literal pixel
    // count. The sim's towers run 10-34 storeys; the old catalogue's art ran 30
    // to 64 drawn storeys at 4 px each (120-265 px), and that is the range the
    // skyline wants back. So the rank is mapped onto 30-64 drawn storeys at the
    // old 4 px rhythm (windows stay dense), and the very top of the range gets
    // one extra flight, so a grown city has a clear high point or two.
    let rank = ((ctx.storeys as f32 - 10.0) / 24.0).clamp(0.0, 1.0);
    // The lots bunch up at the top of the sim's storey range, so the rank alone
    // makes a wall of equally tall towers. A per-lot spread of ±12 % breaks the
    // bunching into a profile — most mid-height, a few clear tall points — while
    // the very top of the range still gets its extra flight.
    let spread = 0.88 + 0.24 * color::hash01((seed as i32) ^ 0x51ED, 0x7B);
    let mut drawn = (30.0 + rank * 34.0) * spread;
    if ctx.storeys >= 26 && color::hash_seed(seed, 0x7A) % 4 == 0 {
        drawn += 6.0; // the very tall one or two
    }
    let storeys_drawn = drawn.round().max(14.0) as i32;
    let floor = 4;
    let base_floors = match base {
        Base::None => 0,
        Base::StonePlinth => 1,
        Base::ShopFront | Base::RaisedPlaza => 2,
        Base::GlassLobby | Base::Colonnade | Base::MallScreen => 3,
        Base::Parking => 4,
    };
    // A raised plaza is a low public deck, not a storey: two more pixels.
    let z_podium = base_floors * floor + if matches!(base, Base::RaisedPlaza) { 2 } else { 0 };
    let body_floors = (storeys_drawn - base_floors).max(6);
    let z_top = z_podium + body_floors * floor;
    // The crown's height is exact: every top draws inside `z_crown` (the draw
    // caps itself there), so the sprite box, the test and the shadow all agree.
    let crown_h = if matches!(body, Body::Wedge) {
        0
    } else {
        match top {
        Top::FlatPlant => 11,
        Top::WaterTank => 11,
        Top::DecoSpire => ((z_top / 8).clamp(10, 20)) + 9,
        Top::PyramidCap => ((w.max(d) * 8.0) as i32 + 6).clamp(8, 18),
        Top::NeedleMast => ((z_top / 6).clamp(10, 24)) + 4,
        Top::Helipad => 4,
        Top::Lantern => 9,
        Top::OpenLattice => 15,
        Top::SlopedGlass => ((w.max(d) * 7.0) as i32 + 6).clamp(8, 16),
        Top::Ziggurat => 15,
        Top::RoofGarden => 4,
        Top::CoolingTowers => 12,
        Top::CrownRing => 4,
        Top::AntennaCluster => 22,
        Top::LightBox => 11,
        }
    };
    // The sprite box is 300 px: the body gives way before the crown, so the
    // tallest tower still draws inside its own height.
    let z_top = z_top.min(290 - crown_h);
    let z_crown = z_top + crown_h;

    // The night look (round 2 item 3): most towers are windows only. A fifth
    // carry baked crown lamps in their own palette; the crowns that *are* lamps
    // (a lantern, a crown ring, a lit sign box) always light. A few modern glass
    // towers get a thin LED line up a corner.
    let lamp_like = matches!(top, Top::Lantern | Top::CrownRing | Top::LightBox);
    // Most towers are windows only: a lamp-shaped crown lights a little over
    // half the time, any other crown one time in eight.
    let lit_pct = if lamp_like { 55 } else { 12 };
    let crown_lights = color::hash_seed(seed, 0x91) % 100 < lit_pct;
    let crown_pal = (color::hash_seed(seed, 0x93) % 7) as u8;
    let glass_mat = matches!(
        material,
        Material::GlassBlue | Material::GlassGreen | Material::GlassGrey | Material::GlassBronze
    );
    let led_edge = glass_mat
        && matches!(
            facade,
            Facade::Curtain | Facade::VerticalFins | Facade::Diagrid | Facade::Exoskeleton | Facade::StripWindows
        )
        && color::hash_seed(seed, 0x95) % 4 == 0;

    Design {
        family: Family::Tower,
        base,
        body,
        massing,
        facade,
        top,
        material,
        storeys: ctx.storeys,
        w,
        d,
        bw,
        bd,
        floor,
        z_podium,
        z_top,
        z_crown,
        seed,
        age: ctx.age,
        crown_lights,
        crown_pal,
        led_edge,
        // B73c's fields: a tower is a tower, not a house.
        civic: CivicKind::None,
        plan: Plan::Box,
        roof: RoofForm::Flat,
        roof_mat: RoofMat::Membrane,
        roof_tint: 0,
        finish: Finish::Panel,
        paint: 0,
        porch: 0,
        dormers: 0,
        bay: 0,
        chimney: 0,
        garage: 0,
        garden: 5,
        fence: 0,
        eaves: 0,
        pitch: 0,
        wall: 0,
    }
}

/// How much a district builds in a material (D80 round 2 item 2: a modern
/// downtown reads mostly glass — blue, green-blue, silver, near-black, the odd
/// white panel — while stone, brick and terracotta live in the older core).
pub(crate) fn material_weight(style: BuildingStyle, m: Material) -> u32 {
    use Material::*;
    match style {
        BuildingStyle::ModernGlass => match m {
            GlassBlue => 9,
            GlassGreen => 7,
            GlassGrey => 6,
            SilverSteel => 3,
            BlackGranite => 4, // the near-black glass towers
            WhitePanel => 1,
            GlassBronze => 1,
            Bronze | Champagne => 1,
            _ => 1,
        },
        BuildingStyle::HighEnd => match m {
            Limestone => 4,
            Champagne => 3,
            WhitePanel => 3,
            SilverSteel => 2,
            GlassBlue => 2,
            GlassGreen | BlackGranite | Sandstone | Copper | GlassGrey => 1,
            _ => 1,
        },
        BuildingStyle::BrickMainStreet => match m {
            RedBrick => 5,
            BuffBrick => 4,
            Brownstone => 2,
            Limestone => 2,
            Sandstone => 2,
            _ => 1,
        },
        BuildingStyle::European => match m {
            Sandstone => 4,
            Limestone => 4,
            BuffBrick => 2,
            RedBrick => 2,
            Copper => 1,
            _ => 1,
        },
        BuildingStyle::Industrial => match m {
            Concrete => 5,
            SilverSteel => 2,
            BlackGranite | Bronze => 1,
            _ => 1,
        },
        BuildingStyle::NewEngland | BuildingStyle::Suburban => match m {
            RedBrick | WhitePanel => 3,
            BuffBrick | Concrete => 2,
            _ => 1,
        },
    }
}

pub(crate) fn pick_material(seed: u64, style: BuildingStyle, era: Era, facade: Facade) -> Material {
    let all = materials(style, era);
    let weighted: Vec<(Material, u32)> = all
        .iter()
        .copied()
        .filter(|m| wears(facade, *m))
        .map(|m| (m, material_weight(style, m)))
        .collect();
    if weighted.is_empty() {
        let flat: Vec<(Material, u32)> = all.iter().map(|m| (*m, 1)).collect();
        return wpick(seed, 61, &flat);
    }
    wpick(seed, 61, &weighted)
}

pub(crate) fn pick_top(seed: u64, ctx: &Ctx, m: Material, body: Body) -> Top {
    let list = tops(ctx.style, m, body, ctx.storeys);
    if list.is_empty() {
        Top::FlatPlant
    } else {
        wpick(seed, 51, &list)
    }
}

// -----------------------------------------------------------------------------
// Spec glue: a generated design, the neighbour triple, and the nudge
// -----------------------------------------------------------------------------

/// The generated design for a spec, or `None` when it is hand-made art (a
/// landmark, or one of the catalogue's rare signature towers).
pub fn design_of(spec: &BuildingSpec) -> Option<Design> {
    use BuildingType as T;
    if spec.function == BuildingFunction::Landmark {
        return None;
    }
    // An explicit hand-made archetype is signature art and draws itself;
    // `*Generated` (the catalogue's own name for "from the kit") is ours.
    let generated = match spec.building_type {
        None
        | Some(T::TowerGenerated)
        | Some(T::HouseGenerated)
        | Some(T::TownhouseGenerated)
        | Some(T::ShopGenerated)
        | Some(T::ApartmentGenerated)
        | Some(T::OfficeGenerated)
        | Some(T::IndustrialGenerated)
        | Some(T::CivicGenerated) => true,
        // The civic types keep their program and are generated (B73c §2).
        Some(T::CivicSchool)
        | Some(T::CivicFireStation)
        | Some(T::CivicLibrary)
        | Some(T::CivicTownHall)
        | Some(T::CivicHospital)
        | Some(T::CivicChurch) => true,
        Some(_) => false,
    };
    if !generated {
        return None;
    }
    Some(pick(&Ctx::of(spec)))
}

/// The triple a touching neighbour must not share.
pub fn key_of(spec: &BuildingSpec) -> Option<DesignKey> {
    design_of(spec).map(|d| d.key())
}

/// The neighbour rule: if the design at `spec.seed` clashes (the caller says
/// which of the touching neighbours already wear that key), step the seed until
/// it does not. `None` means no change is needed (or none was found in a dozen
/// tries, in which case the caller keeps the lot as it is).
pub fn nudge_seed(spec: &BuildingSpec, mut clash: impl FnMut(DesignKey) -> bool) -> Option<u64> {
    if design_of(spec).is_none() {
        return None;
    }
    let base = Ctx::of(spec);
    if !clash(pick(&base).key()) {
        return None;
    }
    let mut seed = spec.seed;
    for _ in 0..12 {
        seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut c = base;
        c.seed = seed;
        if !clash(pick(&c).key()) {
            return Some(seed);
        }
    }
    None
}

/// Draw a design into `sprite` at the sprite's anchor.
pub fn draw(spec: &BuildingSpec, d: &Design, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    match d.family {
        Family::Tower => tower::draw(spec, d, light, sprite, ox, oy),
        _ => building::draw(spec, d, light, sprite, ox, oy),
    }
}
