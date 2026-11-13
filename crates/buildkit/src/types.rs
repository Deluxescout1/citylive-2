//! Core types for the CityLive Iso building catalogue (B33/B33d).

use std::hash::{Hash, Hasher};

/// Building function / zone purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildingFunction {
    House,
    Townhouse,
    Apartment,
    Shop,
    Office,
    Tower,
    Industrial,
    Civic,
    Landmark,
}

/// Architectural district style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildingStyle {
    NewEngland,
    BrickMainStreet,
    European,
    Suburban,
    ModernGlass,
    HighEnd,
    Industrial,
}

/// Historical era of settlement / growth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Era {
    Settlement,
    Village,
    Town,
    City,
    Metropolis,
}

/// Construction stage for buildings and skyscrapers (D53 tower cranes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ConstructionStage {
    Excavation, // Ground pit with shoring and excavator
    Foundation, // Poured concrete slab and rebar grid
    Framing,    // Steel skeleton / concrete frame rising with yellow tower crane
    Cladding,   // Lower floors clad in glass/stone, upper frame with crane
    #[default]
    Finished,   // Completed building
}

/// Generic landmark kind (D59).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LandmarkKind {
    Stadium,
    TrainStation,
    Museum,
    TownHall,
    Cathedral,
    SignatureTower,
    Lighthouse,
    ContainerPort,
    FerrisWheel,
    BridgePylon,
}

/// Distinct recognized building archetypes across all eras and districts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuildingType {
    // New England Houses (4)
    NewEnglandCape,
    NewEnglandColonial,
    NewEnglandSaltbox,
    NewEnglandFarmhouse,

    // Brick Main-Street Shops (4)
    BrickShop2Storey,
    BrickBakeryCafe3Storey,
    BrickCornerStore4Storey,
    BrickBookstoreArcade,

    // European Townhouses (4)
    EuropeanHaussmann,
    EuropeanStuccoTownhouse,
    EuropeanCanalHouse,
    EuropeanOldTownHalfTimbered,

    // Suburban Houses (4)
    SuburbanRanch,
    SuburbanSplitLevel,
    SuburbanCraftsman,
    SuburbanContemporary,

    // Apartments (8 Visibly Distinct Designs)
    ApartmentBrickWalkup,
    ApartmentBrownstoneTrio,
    ApartmentModernMidrise,
    ApartmentCourtyard,
    ApartmentBayWindowTenement,
    ApartmentCornerBalcony,
    ApartmentArtDecoMasonry,
    ApartmentTerraceGarden,

    // Offices (8 Visibly Distinct Designs)
    OfficeClassicStone,
    OfficeArtDeco,
    OfficeModernGlass,
    OfficeCorporatePlaza,
    OfficeTealCurtain,
    OfficeBronzePavilion,
    OfficeBrutalistConcrete,
    OfficePostmodernClock,

    // Towers (16 Slender Varied Designs)
    TowerArtDecoSpire,
    TowerGlassCurtainWall,
    TowerTieredTerraces,
    TowerCrossBraced,
    TowerSlantedGlass,
    TowerCylindricalCrown,
    TowerReflectiveBands,
    TowerSlenderPinnacle,
    TowerTealObelisk,
    TowerDarkBronzeCorporate,
    TowerWhiteLatticeCrown,
    TowerSilverMirrorBlade,
    TowerBlackGraniteSpire,
    TowerCurvedGlassSail,
    TowerDiagridHex,
    TowerHelipadHeadquarters,
    /// B73b: a tower from the generator (podium, shaft, setback, facade, crown
    /// and material picked from the lot's seed). Everything that is not one of
    /// the 16 hand-made signature designs is this.
    TowerGenerated,

    // B73c: every other family from the generator. The hand-made designs above
    // stay in the catalogue as the rare named ones; a lot in a grown city is
    // these.
    HouseGenerated,
    TownhouseGenerated,
    ShopGenerated,
    ApartmentGenerated,
    OfficeGenerated,
    IndustrialGenerated,
    CivicGenerated,

    // Industrial (4)
    IndustrialBrickFactory,
    IndustrialWarehouse,
    IndustrialSilos,
    IndustrialPowerSubstation,

    // Civic (5)
    CivicSchool,
    CivicTownHall,
    CivicLibrary,
    CivicFireStation,
    CivicHospital,
    /// B73c: the village church (nave, steeple, arched windows) — the civic
    /// program it must keep.
    CivicChurch,

    // Generic Landmarks (10)
    LandmarkStadium,
    LandmarkTrainStation,
    LandmarkMuseum,
    LandmarkTownHall,
    LandmarkCathedral,
    LandmarkSignatureTower,
    LandmarkLighthouse,
    LandmarkContainerPort,
    LandmarkFerrisWheel,
    LandmarkBridgePylon,
}

impl BuildingType {
    pub const ALL: &'static [BuildingType] = &[
        BuildingType::NewEnglandCape,
        BuildingType::NewEnglandColonial,
        BuildingType::NewEnglandSaltbox,
        BuildingType::NewEnglandFarmhouse,
        BuildingType::BrickShop2Storey,
        BuildingType::BrickBakeryCafe3Storey,
        BuildingType::BrickCornerStore4Storey,
        BuildingType::BrickBookstoreArcade,
        BuildingType::EuropeanHaussmann,
        BuildingType::EuropeanStuccoTownhouse,
        BuildingType::EuropeanCanalHouse,
        BuildingType::EuropeanOldTownHalfTimbered,
        BuildingType::SuburbanRanch,
        BuildingType::SuburbanSplitLevel,
        BuildingType::SuburbanCraftsman,
        BuildingType::SuburbanContemporary,
        // 8 Apartments
        BuildingType::ApartmentBrickWalkup,
        BuildingType::ApartmentBrownstoneTrio,
        BuildingType::ApartmentModernMidrise,
        BuildingType::ApartmentCourtyard,
        BuildingType::ApartmentBayWindowTenement,
        BuildingType::ApartmentCornerBalcony,
        BuildingType::ApartmentArtDecoMasonry,
        BuildingType::ApartmentTerraceGarden,
        // 8 Offices
        BuildingType::OfficeClassicStone,
        BuildingType::OfficeArtDeco,
        BuildingType::OfficeModernGlass,
        BuildingType::OfficeCorporatePlaza,
        BuildingType::OfficeTealCurtain,
        BuildingType::OfficeBronzePavilion,
        BuildingType::OfficeBrutalistConcrete,
        BuildingType::OfficePostmodernClock,
        // 16 Towers
        BuildingType::TowerArtDecoSpire,
        BuildingType::TowerGlassCurtainWall,
        BuildingType::TowerTieredTerraces,
        BuildingType::TowerCrossBraced,
        BuildingType::TowerSlantedGlass,
        BuildingType::TowerCylindricalCrown,
        BuildingType::TowerReflectiveBands,
        BuildingType::TowerSlenderPinnacle,
        BuildingType::TowerTealObelisk,
        BuildingType::TowerDarkBronzeCorporate,
        BuildingType::TowerWhiteLatticeCrown,
        BuildingType::TowerSilverMirrorBlade,
        BuildingType::TowerBlackGraniteSpire,
        BuildingType::TowerCurvedGlassSail,
        BuildingType::TowerDiagridHex,
        BuildingType::TowerHelipadHeadquarters,
        BuildingType::TowerGenerated,
        // B73c: the generated families
        BuildingType::HouseGenerated,
        BuildingType::TownhouseGenerated,
        BuildingType::ShopGenerated,
        BuildingType::ApartmentGenerated,
        BuildingType::OfficeGenerated,
        BuildingType::IndustrialGenerated,
        BuildingType::CivicGenerated,
        // Industrial
        BuildingType::IndustrialBrickFactory,
        BuildingType::IndustrialWarehouse,
        BuildingType::IndustrialSilos,
        BuildingType::IndustrialPowerSubstation,
        // Civic
        BuildingType::CivicSchool,
        BuildingType::CivicTownHall,
        BuildingType::CivicLibrary,
        BuildingType::CivicFireStation,
        BuildingType::CivicHospital,
        BuildingType::CivicChurch,
        // 10 Landmarks
        BuildingType::LandmarkStadium,
        BuildingType::LandmarkTrainStation,
        BuildingType::LandmarkMuseum,
        BuildingType::LandmarkTownHall,
        BuildingType::LandmarkCathedral,
        BuildingType::LandmarkSignatureTower,
        BuildingType::LandmarkLighthouse,
        BuildingType::LandmarkContainerPort,
        BuildingType::LandmarkFerrisWheel,
        BuildingType::LandmarkBridgePylon,
    ];

    pub const APARTMENTS_8: &'static [BuildingType] = &[
        BuildingType::ApartmentBrickWalkup,
        BuildingType::ApartmentBrownstoneTrio,
        BuildingType::ApartmentModernMidrise,
        BuildingType::ApartmentCourtyard,
        BuildingType::ApartmentBayWindowTenement,
        BuildingType::ApartmentCornerBalcony,
        BuildingType::ApartmentArtDecoMasonry,
        BuildingType::ApartmentTerraceGarden,
    ];

    pub const OFFICES_8: &'static [BuildingType] = &[
        BuildingType::OfficeClassicStone,
        BuildingType::OfficeArtDeco,
        BuildingType::OfficeModernGlass,
        BuildingType::OfficeCorporatePlaza,
        BuildingType::OfficeTealCurtain,
        BuildingType::OfficeBronzePavilion,
        BuildingType::OfficeBrutalistConcrete,
        BuildingType::OfficePostmodernClock,
    ];

    pub const TOWERS_16: &'static [BuildingType] = &[
        BuildingType::TowerArtDecoSpire,
        BuildingType::TowerGlassCurtainWall,
        BuildingType::TowerTieredTerraces,
        BuildingType::TowerCrossBraced,
        BuildingType::TowerSlantedGlass,
        BuildingType::TowerCylindricalCrown,
        BuildingType::TowerReflectiveBands,
        BuildingType::TowerSlenderPinnacle,
        BuildingType::TowerTealObelisk,
        BuildingType::TowerDarkBronzeCorporate,
        BuildingType::TowerWhiteLatticeCrown,
        BuildingType::TowerSilverMirrorBlade,
        BuildingType::TowerBlackGraniteSpire,
        BuildingType::TowerCurvedGlassSail,
        BuildingType::TowerDiagridHex,
        BuildingType::TowerHelipadHeadquarters,
    ];

    pub const LANDMARKS_10: &'static [BuildingType] = &[
        BuildingType::LandmarkStadium,
        BuildingType::LandmarkTrainStation,
        BuildingType::LandmarkMuseum,
        BuildingType::LandmarkTownHall,
        BuildingType::LandmarkCathedral,
        BuildingType::LandmarkSignatureTower,
        BuildingType::LandmarkLighthouse,
        BuildingType::LandmarkContainerPort,
        BuildingType::LandmarkFerrisWheel,
        BuildingType::LandmarkBridgePylon,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            BuildingType::NewEnglandCape => "New England Cape",
            BuildingType::NewEnglandColonial => "New England Colonial",
            BuildingType::NewEnglandSaltbox => "New England Saltbox",
            BuildingType::NewEnglandFarmhouse => "New England Farmhouse",
            BuildingType::BrickShop2Storey => "Brick Shop 2-Storey",
            BuildingType::BrickBakeryCafe3Storey => "Brick Cafe 3-Storey",
            BuildingType::BrickCornerStore4Storey => "Brick Corner 4-Storey",
            BuildingType::BrickBookstoreArcade => "Brick Bookstore Arcade",
            BuildingType::EuropeanHaussmann => "European Haussmann",
            BuildingType::EuropeanStuccoTownhouse => "European Stucco Townhouse",
            BuildingType::EuropeanCanalHouse => "European Canal House",
            BuildingType::EuropeanOldTownHalfTimbered => "Old Town Half-Timbered",
            BuildingType::SuburbanRanch => "Suburban Ranch House",
            BuildingType::SuburbanSplitLevel => "Suburban Split-Level",
            BuildingType::SuburbanCraftsman => "Suburban Craftsman",
            BuildingType::SuburbanContemporary => "Suburban Contemporary",
            BuildingType::ApartmentBrickWalkup => "Brick Walkup Apartment",
            BuildingType::ApartmentBrownstoneTrio => "Brownstone Townhouse",
            BuildingType::ApartmentModernMidrise => "Modern Mid-Rise Apartment",
            BuildingType::ApartmentCourtyard => "Courtyard Apartment",
            BuildingType::ApartmentBayWindowTenement => "Tenement Bay Windows",
            BuildingType::ApartmentCornerBalcony => "Corner Balcony Apartment",
            BuildingType::ApartmentArtDecoMasonry => "Art Deco Masonry Apartment",
            BuildingType::ApartmentTerraceGarden => "Terraced Garden Apartment",
            BuildingType::OfficeClassicStone => "Classic Stone Bank/Office",
            BuildingType::OfficeArtDeco => "Art Deco Commercial",
            BuildingType::OfficeModernGlass => "Modern Glass Office",
            BuildingType::OfficeCorporatePlaza => "Corporate Plaza Office",
            BuildingType::OfficeTealCurtain => "Teal Curtain Office",
            BuildingType::OfficeBronzePavilion => "Bronze Pavilion Office",
            BuildingType::OfficeBrutalistConcrete => "Brutalist Concrete Office",
            BuildingType::OfficePostmodernClock => "Post-Modern Clock Office",
            BuildingType::TowerArtDecoSpire => "Art Deco Spire Tower",
            BuildingType::TowerGlassCurtainWall => "Glass Curtain Skyscraper",
            BuildingType::TowerTieredTerraces => "Tiered Terrace Skyscraper",
            BuildingType::TowerCrossBraced => "Cross-Braced Mega Tower",
            BuildingType::TowerSlantedGlass => "Slanted Glass Tower",
            BuildingType::TowerCylindricalCrown => "Cylindrical Crown Tower",
            BuildingType::TowerReflectiveBands => "Reflective Band Tower",
            BuildingType::TowerSlenderPinnacle => "Slender Pinnacle Tower",
            BuildingType::TowerTealObelisk => "Teal Obelisk Tower",
            BuildingType::TowerDarkBronzeCorporate => "Dark Bronze Corporate Tower",
            BuildingType::TowerWhiteLatticeCrown => "White Lattice Crown Tower",
            BuildingType::TowerSilverMirrorBlade => "Silver Mirror Blade Tower",
            BuildingType::TowerBlackGraniteSpire => "Black Granite Spire",
            BuildingType::TowerCurvedGlassSail => "Curved Glass Sail Tower",
            BuildingType::TowerDiagridHex => "Hex Diagrid Tower",
            BuildingType::TowerHelipadHeadquarters => "Helipad Headquarters Tower",
            BuildingType::TowerGenerated => "Generated Tower",
            BuildingType::HouseGenerated => "Generated House",
            BuildingType::TownhouseGenerated => "Generated Townhouse",
            BuildingType::ShopGenerated => "Generated Shop",
            BuildingType::ApartmentGenerated => "Generated Apartment",
            BuildingType::OfficeGenerated => "Generated Office",
            BuildingType::IndustrialGenerated => "Generated Works",
            BuildingType::CivicGenerated => "Generated Civic",
            BuildingType::IndustrialBrickFactory => "Industrial Brick Factory",
            BuildingType::IndustrialWarehouse => "Logistics Warehouse",
            BuildingType::IndustrialSilos => "Industrial Silos",
            BuildingType::IndustrialPowerSubstation => "Power Substation",
            BuildingType::CivicSchool => "Civic School",
            BuildingType::CivicTownHall => "Civic Town Hall",
            BuildingType::CivicLibrary => "Civic Library",
            BuildingType::CivicFireStation => "Civic Fire Station",
            BuildingType::CivicHospital => "Civic Hospital",
            BuildingType::CivicChurch => "Civic Church",
            BuildingType::LandmarkStadium => "Landmark Stadium",
            BuildingType::LandmarkTrainStation => "Landmark Train Station",
            BuildingType::LandmarkMuseum => "Landmark Museum",
            BuildingType::LandmarkTownHall => "Landmark Town Hall",
            BuildingType::LandmarkCathedral => "Landmark Cathedral",
            BuildingType::LandmarkSignatureTower => "Signature Observation Tower",
            BuildingType::LandmarkLighthouse => "Landmark Coastal Lighthouse",
            BuildingType::LandmarkContainerPort => "Container Port Gantry",
            BuildingType::LandmarkFerrisWheel => "Landmark Ferris Wheel",
            BuildingType::LandmarkBridgePylon => "Cable-Stay Bridge Pylon",
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            BuildingType::NewEnglandCape
            | BuildingType::NewEnglandColonial
            | BuildingType::NewEnglandSaltbox
            | BuildingType::NewEnglandFarmhouse => "New England Houses",

            BuildingType::BrickShop2Storey
            | BuildingType::BrickBakeryCafe3Storey
            | BuildingType::BrickCornerStore4Storey
            | BuildingType::BrickBookstoreArcade => "Brick Main-Street Shops",

            BuildingType::EuropeanHaussmann
            | BuildingType::EuropeanStuccoTownhouse
            | BuildingType::EuropeanCanalHouse
            | BuildingType::EuropeanOldTownHalfTimbered => "European Townhouses",

            BuildingType::SuburbanRanch
            | BuildingType::SuburbanSplitLevel
            | BuildingType::SuburbanCraftsman
            | BuildingType::SuburbanContemporary => "Suburban Houses",

            BuildingType::ApartmentBrickWalkup
            | BuildingType::ApartmentBrownstoneTrio
            | BuildingType::ApartmentModernMidrise
            | BuildingType::ApartmentCourtyard
            | BuildingType::ApartmentBayWindowTenement
            | BuildingType::ApartmentCornerBalcony
            | BuildingType::ApartmentArtDecoMasonry
            | BuildingType::ApartmentTerraceGarden => "Apartments",

            BuildingType::OfficeClassicStone
            | BuildingType::OfficeArtDeco
            | BuildingType::OfficeModernGlass
            | BuildingType::OfficeCorporatePlaza
            | BuildingType::OfficeTealCurtain
            | BuildingType::OfficeBronzePavilion
            | BuildingType::OfficeBrutalistConcrete
            | BuildingType::OfficePostmodernClock => "Offices",

            BuildingType::HouseGenerated => "Generated Houses",
            BuildingType::TownhouseGenerated => "Generated Townhouses",
            BuildingType::ShopGenerated => "Generated Shops",
            BuildingType::ApartmentGenerated => "Generated Apartments",
            BuildingType::OfficeGenerated => "Generated Offices",
            BuildingType::IndustrialGenerated => "Generated Industrial",
            BuildingType::CivicGenerated => "Generated Civic",

            BuildingType::TowerArtDecoSpire
            | BuildingType::TowerGlassCurtainWall
            | BuildingType::TowerTieredTerraces
            | BuildingType::TowerCrossBraced
            | BuildingType::TowerSlantedGlass
            | BuildingType::TowerCylindricalCrown
            | BuildingType::TowerReflectiveBands
            | BuildingType::TowerSlenderPinnacle
            | BuildingType::TowerTealObelisk
            | BuildingType::TowerDarkBronzeCorporate
            | BuildingType::TowerWhiteLatticeCrown
            | BuildingType::TowerSilverMirrorBlade
            | BuildingType::TowerBlackGraniteSpire
            | BuildingType::TowerCurvedGlassSail
            | BuildingType::TowerDiagridHex
            | BuildingType::TowerHelipadHeadquarters
            | BuildingType::TowerGenerated => "Towers",

            BuildingType::IndustrialBrickFactory
            | BuildingType::IndustrialWarehouse
            | BuildingType::IndustrialSilos
            | BuildingType::IndustrialPowerSubstation => "Industrial",

            BuildingType::CivicSchool
            | BuildingType::CivicTownHall
            | BuildingType::CivicLibrary
            | BuildingType::CivicFireStation
            | BuildingType::CivicHospital
            | BuildingType::CivicChurch => "Civic",

            BuildingType::LandmarkStadium
            | BuildingType::LandmarkTrainStation
            | BuildingType::LandmarkMuseum
            | BuildingType::LandmarkTownHall
            | BuildingType::LandmarkCathedral
            | BuildingType::LandmarkSignatureTower
            | BuildingType::LandmarkLighthouse
            | BuildingType::LandmarkContainerPort
            | BuildingType::LandmarkFerrisWheel
            | BuildingType::LandmarkBridgePylon => "Landmarks",
        }
    }

    pub fn default_spec(&self, seed: u64, variant: u8) -> BuildingSpec {
        let (function, style, footprint, storeys, era, l_kind) = match self {
            BuildingType::NewEnglandCape => (BuildingFunction::House, BuildingStyle::NewEngland, (1, 1), 2, Era::Settlement, None),
            BuildingType::NewEnglandColonial => (BuildingFunction::House, BuildingStyle::NewEngland, (2, 1), 2, Era::Village, None),
            BuildingType::NewEnglandSaltbox => (BuildingFunction::House, BuildingStyle::NewEngland, (1, 1), 2, Era::Settlement, None),
            BuildingType::NewEnglandFarmhouse => (BuildingFunction::House, BuildingStyle::NewEngland, (2, 2), 2, Era::Village, None),

            BuildingType::BrickShop2Storey => (BuildingFunction::Shop, BuildingStyle::BrickMainStreet, (1, 1), 2, Era::Town, None),
            BuildingType::BrickBakeryCafe3Storey => (BuildingFunction::Shop, BuildingStyle::BrickMainStreet, (1, 1), 3, Era::Town, None),
            BuildingType::BrickCornerStore4Storey => (BuildingFunction::Shop, BuildingStyle::BrickMainStreet, (2, 1), 4, Era::Town, None),
            BuildingType::BrickBookstoreArcade => (BuildingFunction::Shop, BuildingStyle::BrickMainStreet, (1, 1), 3, Era::Town, None),

            BuildingType::EuropeanHaussmann => (BuildingFunction::Townhouse, BuildingStyle::European, (1, 1), 4, Era::Town, None),
            BuildingType::EuropeanStuccoTownhouse => (BuildingFunction::Townhouse, BuildingStyle::European, (1, 1), 3, Era::Town, None),
            BuildingType::EuropeanCanalHouse => (BuildingFunction::Townhouse, BuildingStyle::European, (1, 1), 4, Era::Town, None),
            BuildingType::EuropeanOldTownHalfTimbered => (BuildingFunction::Townhouse, BuildingStyle::European, (1, 1), 3, Era::Village, None),

            BuildingType::SuburbanRanch => (BuildingFunction::House, BuildingStyle::Suburban, (2, 2), 1, Era::City, None),
            BuildingType::SuburbanSplitLevel => (BuildingFunction::House, BuildingStyle::Suburban, (2, 1), 2, Era::City, None),
            BuildingType::SuburbanCraftsman => (BuildingFunction::House, BuildingStyle::Suburban, (2, 2), 2, Era::Town, None),
            BuildingType::SuburbanContemporary => (BuildingFunction::House, BuildingStyle::Suburban, (2, 2), 2, Era::City, None),

            BuildingType::ApartmentBrickWalkup => (BuildingFunction::Apartment, BuildingStyle::BrickMainStreet, (1, 1), 4, Era::City, None),
            BuildingType::ApartmentBrownstoneTrio => (BuildingFunction::Apartment, BuildingStyle::BrickMainStreet, (1, 1), 4, Era::Town, None),
            BuildingType::ApartmentModernMidrise => (BuildingFunction::Apartment, BuildingStyle::ModernGlass, (2, 2), 6, Era::City, None),
            BuildingType::ApartmentCourtyard => (BuildingFunction::Apartment, BuildingStyle::BrickMainStreet, (2, 2), 5, Era::City, None),
            BuildingType::ApartmentBayWindowTenement => (BuildingFunction::Apartment, BuildingStyle::BrickMainStreet, (1, 1), 5, Era::City, None),
            BuildingType::ApartmentCornerBalcony => (BuildingFunction::Apartment, BuildingStyle::ModernGlass, (2, 2), 6, Era::City, None),
            BuildingType::ApartmentArtDecoMasonry => (BuildingFunction::Apartment, BuildingStyle::HighEnd, (2, 2), 7, Era::City, None),
            BuildingType::ApartmentTerraceGarden => (BuildingFunction::Apartment, BuildingStyle::HighEnd, (2, 2), 6, Era::City, None),

            BuildingType::OfficeClassicStone => (BuildingFunction::Office, BuildingStyle::HighEnd, (2, 2), 4, Era::Town, None),
            BuildingType::OfficeArtDeco => (BuildingFunction::Office, BuildingStyle::HighEnd, (2, 2), 7, Era::City, None),
            BuildingType::OfficeModernGlass => (BuildingFunction::Office, BuildingStyle::ModernGlass, (2, 2), 8, Era::City, None),
            BuildingType::OfficeCorporatePlaza => (BuildingFunction::Office, BuildingStyle::ModernGlass, (2, 2), 10, Era::Metropolis, None),
            BuildingType::OfficeTealCurtain => (BuildingFunction::Office, BuildingStyle::ModernGlass, (2, 2), 8, Era::City, None),
            BuildingType::OfficeBronzePavilion => (BuildingFunction::Office, BuildingStyle::HighEnd, (2, 2), 7, Era::City, None),
            BuildingType::OfficeBrutalistConcrete => (BuildingFunction::Office, BuildingStyle::Industrial, (2, 2), 8, Era::City, None),
            BuildingType::OfficePostmodernClock => (BuildingFunction::Office, BuildingStyle::HighEnd, (2, 2), 9, Era::Metropolis, None),

            // Towers with slender proportions (1×1, 1×2, 2×2)
            BuildingType::TowerArtDecoSpire => (BuildingFunction::Tower, BuildingStyle::HighEnd, (1, 2), 45, Era::Metropolis, None),
            BuildingType::TowerGlassCurtainWall => (BuildingFunction::Tower, BuildingStyle::ModernGlass, (1, 1), 38, Era::Metropolis, None),
            BuildingType::TowerTieredTerraces => (BuildingFunction::Tower, BuildingStyle::HighEnd, (2, 2), 32, Era::Metropolis, None),
            BuildingType::TowerCrossBraced => (BuildingFunction::Tower, BuildingStyle::ModernGlass, (1, 2), 50, Era::Metropolis, None),
            BuildingType::TowerSlantedGlass => (BuildingFunction::Tower, BuildingStyle::ModernGlass, (1, 1), 30, Era::Metropolis, None),
            BuildingType::TowerCylindricalCrown => (BuildingFunction::Tower, BuildingStyle::ModernGlass, (1, 1), 34, Era::Metropolis, None),
            BuildingType::TowerReflectiveBands => (BuildingFunction::Tower, BuildingStyle::HighEnd, (1, 2), 30, Era::Metropolis, None),
            BuildingType::TowerSlenderPinnacle => (BuildingFunction::Tower, BuildingStyle::HighEnd, (1, 1), 56, Era::Metropolis, None),
            BuildingType::TowerTealObelisk => (BuildingFunction::Tower, BuildingStyle::ModernGlass, (1, 1), 42, Era::Metropolis, None),
            BuildingType::TowerDarkBronzeCorporate => (BuildingFunction::Tower, BuildingStyle::ModernGlass, (1, 1), 36, Era::Metropolis, None),
            BuildingType::TowerWhiteLatticeCrown => (BuildingFunction::Tower, BuildingStyle::HighEnd, (2, 2), 50, Era::Metropolis, None),
            BuildingType::TowerSilverMirrorBlade => (BuildingFunction::Tower, BuildingStyle::ModernGlass, (1, 2), 44, Era::Metropolis, None),
            BuildingType::TowerBlackGraniteSpire => (BuildingFunction::Tower, BuildingStyle::HighEnd, (1, 1), 40, Era::Metropolis, None),
            BuildingType::TowerCurvedGlassSail => (BuildingFunction::Tower, BuildingStyle::ModernGlass, (1, 1), 35, Era::Metropolis, None),
            BuildingType::TowerDiagridHex => (BuildingFunction::Tower, BuildingStyle::ModernGlass, (1, 1), 36, Era::Metropolis, None),
            BuildingType::TowerHelipadHeadquarters => (BuildingFunction::Tower, BuildingStyle::HighEnd, (1, 2), 32, Era::Metropolis, None),
            BuildingType::TowerGenerated => (BuildingFunction::Tower, BuildingStyle::ModernGlass, (1, 1), 24, Era::Metropolis, None),

            // B73c: the generated families (a lot in a grown city is one of
            // these; the hand-made types above stay as named/rare designs).
            BuildingType::HouseGenerated => (BuildingFunction::House, BuildingStyle::NewEngland, (1, 1), 2, Era::Village, None),
            BuildingType::TownhouseGenerated => (BuildingFunction::Townhouse, BuildingStyle::European, (1, 2), 3, Era::Town, None),
            BuildingType::ShopGenerated => (BuildingFunction::Shop, BuildingStyle::BrickMainStreet, (2, 1), 3, Era::Town, None),
            BuildingType::ApartmentGenerated => (BuildingFunction::Apartment, BuildingStyle::BrickMainStreet, (2, 2), 5, Era::City, None),
            BuildingType::OfficeGenerated => (BuildingFunction::Office, BuildingStyle::ModernGlass, (2, 2), 5, Era::City, None),
            BuildingType::IndustrialGenerated => (BuildingFunction::Industrial, BuildingStyle::Industrial, (2, 2), 2, Era::Town, None),
            BuildingType::CivicGenerated => (BuildingFunction::Civic, BuildingStyle::HighEnd, (2, 2), 2, Era::Town, None),

            BuildingType::IndustrialBrickFactory => (BuildingFunction::Industrial, BuildingStyle::Industrial, (2, 2), 2, Era::Town, None),
            BuildingType::IndustrialWarehouse => (BuildingFunction::Industrial, BuildingStyle::Industrial, (3, 2), 2, Era::City, None),
            BuildingType::IndustrialSilos => (BuildingFunction::Industrial, BuildingStyle::Industrial, (2, 2), 4, Era::Town, None),
            BuildingType::IndustrialPowerSubstation => (BuildingFunction::Industrial, BuildingStyle::Industrial, (2, 2), 1, Era::City, None),

            BuildingType::CivicSchool => (BuildingFunction::Civic, BuildingStyle::BrickMainStreet, (3, 2), 2, Era::Town, None),
            BuildingType::CivicTownHall => (BuildingFunction::Civic, BuildingStyle::HighEnd, (2, 2), 3, Era::Town, None),
            BuildingType::CivicLibrary => (BuildingFunction::Civic, BuildingStyle::HighEnd, (2, 2), 2, Era::Town, None),
            BuildingType::CivicFireStation => (BuildingFunction::Civic, BuildingStyle::BrickMainStreet, (2, 1), 2, Era::Town, None),
            BuildingType::CivicHospital => (BuildingFunction::Civic, BuildingStyle::ModernGlass, (3, 2), 5, Era::City, None),
            BuildingType::CivicChurch => (BuildingFunction::Civic, BuildingStyle::NewEngland, (2, 2), 3, Era::Settlement, None),

            // Landmarks (10)
            BuildingType::LandmarkStadium => (BuildingFunction::Landmark, BuildingStyle::HighEnd, (3, 3), 4, Era::City, Some(LandmarkKind::Stadium)),
            BuildingType::LandmarkTrainStation => (BuildingFunction::Landmark, BuildingStyle::HighEnd, (3, 2), 3, Era::City, Some(LandmarkKind::TrainStation)),
            BuildingType::LandmarkMuseum => (BuildingFunction::Landmark, BuildingStyle::HighEnd, (3, 2), 3, Era::City, Some(LandmarkKind::Museum)),
            BuildingType::LandmarkTownHall => (BuildingFunction::Landmark, BuildingStyle::HighEnd, (2, 2), 3, Era::Town, Some(LandmarkKind::TownHall)),
            BuildingType::LandmarkCathedral => (BuildingFunction::Landmark, BuildingStyle::HighEnd, (3, 2), 4, Era::Village, Some(LandmarkKind::Cathedral)),
            BuildingType::LandmarkSignatureTower => (BuildingFunction::Landmark, BuildingStyle::ModernGlass, (2, 2), 60, Era::Metropolis, Some(LandmarkKind::SignatureTower)),
            BuildingType::LandmarkLighthouse => (BuildingFunction::Landmark, BuildingStyle::NewEngland, (2, 2), 6, Era::Settlement, Some(LandmarkKind::Lighthouse)),
            BuildingType::LandmarkContainerPort => (BuildingFunction::Landmark, BuildingStyle::Industrial, (3, 2), 4, Era::City, Some(LandmarkKind::ContainerPort)),
            BuildingType::LandmarkFerrisWheel => (BuildingFunction::Landmark, BuildingStyle::HighEnd, (2, 2), 5, Era::City, Some(LandmarkKind::FerrisWheel)),
            BuildingType::LandmarkBridgePylon => (BuildingFunction::Landmark, BuildingStyle::ModernGlass, (2, 2), 8, Era::City, Some(LandmarkKind::BridgePylon)),
        };

        BuildingSpec {
            building_type: Some(*self),
            function,
            landmark_kind: l_kind,
            style,
            footprint,
            storeys,
            era,
            stage: ConstructionStage::Finished,
            age: 0.0,
            seed,
            variant,
            biome: crate::style::BiomeStyle::Temperate,
        }
    }
}

/// Full specification for procedural building generation.
#[derive(Clone, Debug)]
pub struct BuildingSpec {
    /// Optional explicit archetype (if None, inferred from function, style, storeys, footprint)
    pub building_type: Option<BuildingType>,
    pub function: BuildingFunction,
    pub landmark_kind: Option<LandmarkKind>,
    pub style: BuildingStyle,
    pub footprint: (u8, u8),
    pub storeys: u8,
    pub era: Era,
    /// Construction stage (Excavation, Foundation, Framing with tower crane, Cladding, Finished)
    pub stage: ConstructionStage,
    /// Age in years (0.0 .. 100.0) for weathering / aging.
    pub age: f32,
    pub seed: u64,
    pub variant: u8,
    /// The land's building vernacular (D66 item 4): a palette / roof / material
    /// shift applied to the catalogue's own art, never a second catalogue.
    pub biome: crate::style::BiomeStyle,
}

impl PartialEq for BuildingSpec {
    fn eq(&self, other: &Self) -> bool {
        self.building_type == other.building_type
            && self.function == other.function
            && self.landmark_kind == other.landmark_kind
            && self.style == other.style
            && self.footprint == other.footprint
            && self.storeys == other.storeys
            && self.era == other.era
            && self.stage == other.stage
            && (self.age * 10.0).round() as i32 == (other.age * 10.0).round() as i32
            && self.seed == other.seed
            && self.variant == other.variant
            && self.biome == other.biome
    }
}

impl Eq for BuildingSpec {}

impl Hash for BuildingSpec {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.building_type.hash(state);
        self.function.hash(state);
        self.landmark_kind.hash(state);
        self.style.hash(state);
        self.footprint.hash(state);
        self.storeys.hash(state);
        self.era.hash(state);
        self.stage.hash(state);
        ((self.age * 10.0).round() as i32).hash(state);
        self.seed.hash(state);
        self.variant.hash(state);
        self.biome.hash(state);
    }
}

impl BuildingSpec {
    pub fn resolved_type(&self) -> BuildingType {
        if let Some(bt) = self.building_type {
            return bt;
        }
        if let Some(lk) = self.landmark_kind {
            return match lk {
                LandmarkKind::Stadium => BuildingType::LandmarkStadium,
                LandmarkKind::TrainStation => BuildingType::LandmarkTrainStation,
                LandmarkKind::Museum => BuildingType::LandmarkMuseum,
                LandmarkKind::TownHall => BuildingType::LandmarkTownHall,
                LandmarkKind::Cathedral => BuildingType::LandmarkCathedral,
                LandmarkKind::SignatureTower => BuildingType::LandmarkSignatureTower,
                LandmarkKind::Lighthouse => BuildingType::LandmarkLighthouse,
                LandmarkKind::ContainerPort => BuildingType::LandmarkContainerPort,
                LandmarkKind::FerrisWheel => BuildingType::LandmarkFerrisWheel,
                LandmarkKind::BridgePylon => BuildingType::LandmarkBridgePylon,
            };
        }
        match (self.function, self.style) {
            // B73c: every family comes from the kit now. The hand-made
            // archetypes above are only drawn when a lot asks for one by name
            // (the catalogue's tests, and the rare signature art).
            (BuildingFunction::House, _) => BuildingType::HouseGenerated,
            (BuildingFunction::Townhouse, _) => BuildingType::TownhouseGenerated,
            (BuildingFunction::Shop, _) => BuildingType::ShopGenerated,
            (BuildingFunction::Apartment, _) => BuildingType::ApartmentGenerated,
            (BuildingFunction::Office, _) => BuildingType::OfficeGenerated,
            (BuildingFunction::Tower, _) => BuildingType::TowerGenerated,
            (BuildingFunction::Industrial, _) => BuildingType::IndustrialGenerated,
            (BuildingFunction::Civic, _) => BuildingType::CivicGenerated,
            (BuildingFunction::Landmark, _) => match self.variant % 10 {
                0 => BuildingType::LandmarkStadium,
                1 => BuildingType::LandmarkTrainStation,
                2 => BuildingType::LandmarkMuseum,
                3 => BuildingType::LandmarkTownHall,
                4 => BuildingType::LandmarkCathedral,
                5 => BuildingType::LandmarkSignatureTower,
                6 => BuildingType::LandmarkLighthouse,
                7 => BuildingType::LandmarkContainerPort,
                8 => BuildingType::LandmarkFerrisWheel,
                _ => BuildingType::LandmarkBridgePylon,
            },
        }
    }

    /// Convenience constructor for Landmarks.
    pub fn landmark(kind: LandmarkKind, seed: u64, variant: u8) -> Self {
        let bt = match kind {
            LandmarkKind::Stadium => BuildingType::LandmarkStadium,
            LandmarkKind::TrainStation => BuildingType::LandmarkTrainStation,
            LandmarkKind::Museum => BuildingType::LandmarkMuseum,
            LandmarkKind::TownHall => BuildingType::LandmarkTownHall,
            LandmarkKind::Cathedral => BuildingType::LandmarkCathedral,
            LandmarkKind::SignatureTower => BuildingType::LandmarkSignatureTower,
            LandmarkKind::Lighthouse => BuildingType::LandmarkLighthouse,
            LandmarkKind::ContainerPort => BuildingType::LandmarkContainerPort,
            LandmarkKind::FerrisWheel => BuildingType::LandmarkFerrisWheel,
            LandmarkKind::BridgePylon => BuildingType::LandmarkBridgePylon,
        };
        let mut spec = bt.default_spec(seed, variant);
        spec.landmark_kind = Some(kind);
        spec
    }
}

/// Ambient light condition for rendering sprites and night window illumination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    /// Discrete bucket for cache lookup (e.g. 0 = Day, 1 = Sunset, 2 = Night).
    pub bucket: u8,
    /// Whether night rules apply (emissive windows, neon signs, beacons).
    pub is_night: bool,
    /// How far into the evening the scene is, 0 (full day) .. 1 (full night).
    ///
    /// B99d: the materials used to flip on `is_night` alone, so the glass of a
    /// tower went from reflecting the sky to a flat dark box between two frames
    /// while the light around it kept fading. Everything that changes with the
    /// evening now reads this instead: the glass keeps reflecting (darkening as
    /// it goes), the number of lit panes grows from nothing to the night share,
    /// and `surf` dims by it. It is quantised by [`bucket`] like everything else
    /// that is baked, so the frame-to-frame fade is still the ambient tint's job.
    pub night: f32,
    /// Color multiplier tint [r, g, b].
    pub tint: [f32; 3],
    /// Time of day in hours (0.0 .. 24.0).
    pub hour: f32,
}

impl Light {
    pub const DAY_BUCKET: u8 = 0;
    pub const SUNSET_BUCKET: u8 = 1;
    pub const NIGHT_BUCKET: u8 = 2;

    pub fn day() -> Self {
        Self {
            bucket: Self::DAY_BUCKET,
            is_night: false,
            night: 0.0,
            tint: [1.0, 1.0, 1.0],
            hour: 12.0,
        }
    }

    pub fn sunset() -> Self {
        Self {
            bucket: Self::SUNSET_BUCKET,
            is_night: false,
            night: 0.45,
            tint: [1.05, 0.78, 0.62],
            hour: 18.5,
        }
    }

    pub fn night() -> Self {
        Self {
            bucket: Self::NIGHT_BUCKET,
            is_night: true,
            night: 1.0,
            tint: [0.38, 0.42, 0.58],
            hour: 22.0,
        }
    }

    #[inline]
    pub fn bucket(&self) -> u8 {
        self.bucket
    }

    #[inline]
    pub fn is_night(&self) -> bool {
        self.is_night
    }
}

/// B72: one switchable window pane of a kit sprite: the glass pixels it owns,
/// in the sprite's own coordinates, and whether it burns warm or cool.
#[derive(Clone, Debug, Default)]
pub struct WinPane {
    pub px: Vec<(i16, i16)>,
    pub warm: bool,
}

/// A rendered sprite: pixels, the depth/height plane, the roof it drew and the
/// window panes the renderer may light.
#[derive(Clone, Debug, Default)]
pub struct Sprite {
    pub w: usize,
    pub h: usize,
    /// Anchor X offset in pixels relative to top-left (corresponds to tile origin x=0, y=0).
    pub ox: i32,
    /// Anchor Y offset in pixels relative to top-left (corresponds to tile origin x=0, y=0).
    pub oy: i32,
    /// 32-bit ARGB/RGBA color pixels (0xAARRGGBB), row-major.
    pub px: Vec<u32>,
    /// Per-pixel depth or height in pixels above ground for the engine's depth mask.
    pub depth: Vec<u16>,
    /// B71: the pitched roof the last roof part drew (eave, ridge, form).
    pub roof: Option<render::scene::RoofInfo>,
    /// B72 (D78): the window panes the sim can switch. They are painted **dark**
    /// here; the renderer lights the ones whose building has someone in.
    pub win: Vec<WinPane>,
    /// B76 r3: the surface each pixel belongs to (`render::snow`'s tags), so
    /// snow can lie on a roof plane instead of on the top of the sprite.
    pub surf: Vec<u8>,
    /// The tag stamped onto every pixel written from now on.
    pub stamp: u8,
}

impl Sprite {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            ox: 0,
            oy: 0,
            px: vec![0; w * h],
            depth: vec![0; w * h],
            roof: None,
            win: Vec::new(),
            surf: vec![0; w * h],
            stamp: render::snow::NONE,
        }
    }

    /// Paint the pixels written from here on as this surface.
    #[inline]
    pub fn on_surface(&mut self, tag: u8) {
        self.stamp = tag;
    }

    pub fn resize(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        self.px.resize(w * h, 0);
        self.px.fill(0);
        self.depth.resize(w * h, 0);
        self.depth.fill(0);
        self.surf.resize(w * h, 0);
        self.surf.fill(0);
        self.stamp = render::snow::NONE;
        self.roof = None;
        self.win.clear();
    }

    #[inline]
    pub fn set_pixel(&mut self, x: i32, y: i32, color: u32, depth: u16) {
        if x >= 0 && (x as usize) < self.w && y >= 0 && (y as usize) < self.h {
            let idx = y as usize * self.w + x as usize;
            self.px[idx] = color;
            self.depth[idx] = depth;
            if self.stamp != render::snow::NONE {
                self.surf[idx] = self.stamp;
            } else if color != 0 {
                // A plain write clears whatever surface was tagged here before
                // (a wall painted over an earlier roof pixel).
                self.surf[idx] = render::snow::NONE;
            }
        }
    }

    #[inline]
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: u32, depth: u16, alpha: f32) {
        if x >= 0 && (x as usize) < self.w && y >= 0 && (y as usize) < self.h && alpha > 0.0 {
            let idx = y as usize * self.w + x as usize;
            let dst = self.px[idx];
            self.px[idx] = render::palette::blend(dst, color, alpha);
            if depth > self.depth[idx] {
                self.depth[idx] = depth;
            }
            if self.stamp != render::snow::NONE {
                self.surf[idx] = self.stamp;
            }
        }
    }

    #[inline]
    pub fn get_pixel(&self, x: usize, y: usize) -> u32 {
        if x < self.w && y < self.h {
            self.px[y * self.w + x]
        } else {
            0
        }
    }

    #[inline]
    pub fn get_depth(&self, x: usize, y: usize) -> u16 {
        if x < self.w && y < self.h {
            self.depth[y * self.w + x]
        } else {
            0
        }
    }

    /// Convert to render crate's Sprite type for engine integration.
    pub fn to_render_sprite(&self) -> render::framebuffer::Sprite {
        let mut s = render::framebuffer::Sprite::new(self.w, self.h);
        s.px.copy_from_slice(&self.px);
        s
    }
}
