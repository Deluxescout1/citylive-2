//! Building catalogue dispatch.

pub mod houses;
pub mod shops;
pub mod townhouses;
pub mod suburban;
pub mod apartments;
pub mod offices;
pub mod towers;
pub mod industrial;
pub mod civic;
pub mod landmarks;

use crate::types::{BuildingSpec, BuildingType, Light, Sprite};

pub fn max_height(spec: &BuildingSpec) -> f32 {
    match spec.resolved_type() {
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
        | BuildingType::LandmarkSignatureTower
        | BuildingType::LandmarkBridgePylon => 260.0,

        // B73b round 2: generated towers are taller than the hand-made ones now
        // (the lot's storeys mapped onto the old art's visual range), so their
        // sprite box is sized for the tallest crown the generator can make.
        BuildingType::TowerGenerated => 300.0,

        // B73c: the generated families — a house, a terrace, a shop, a block,
        // a works, a civic building — all one generator now. Their boxes hold
        // the tallest design each family can make (`genkit::pick`'s own cap).
        BuildingType::HouseGenerated => 62.0,
        BuildingType::TownhouseGenerated => 96.0,
        BuildingType::ShopGenerated => 88.0,
        BuildingType::ApartmentGenerated => 112.0,
        BuildingType::OfficeGenerated => 150.0,
        BuildingType::IndustrialGenerated => 104.0,
        BuildingType::CivicGenerated => 120.0,
        // The civic types keep their program but are generated too, so their box
        // covers the tallest generated civic (a hospital block with its wings).
        BuildingType::CivicSchool | BuildingType::CivicLibrary | BuildingType::CivicFireStation => {
            120.0
        }
        BuildingType::CivicChurch => 140.0,
        BuildingType::CivicTownHall | BuildingType::CivicHospital => 132.0,

        BuildingType::OfficeCorporatePlaza
        | BuildingType::OfficeModernGlass
        | BuildingType::OfficeArtDeco
        | BuildingType::OfficeTealCurtain
        | BuildingType::OfficeBronzePavilion
        | BuildingType::OfficeBrutalistConcrete
        | BuildingType::OfficePostmodernClock => 140.0,

        BuildingType::ApartmentModernMidrise
        | BuildingType::ApartmentCornerBalcony
        | BuildingType::ApartmentArtDecoMasonry
        | BuildingType::ApartmentTerraceGarden
        | BuildingType::CivicHospital
        | BuildingType::LandmarkCathedral => 110.0,

        BuildingType::OfficeClassicStone
        | BuildingType::ApartmentCourtyard
        | BuildingType::ApartmentBrickWalkup
        | BuildingType::ApartmentBrownstoneTrio
        | BuildingType::ApartmentBayWindowTenement
        | BuildingType::CivicTownHall
        | BuildingType::LandmarkTownHall
        | BuildingType::LandmarkLighthouse
        | BuildingType::LandmarkFerrisWheel
        | BuildingType::IndustrialSilos
        | BuildingType::IndustrialBrickFactory => 95.0,

        BuildingType::CivicSchool
        | BuildingType::CivicLibrary
        | BuildingType::CivicFireStation
        | BuildingType::LandmarkStadium
        | BuildingType::LandmarkTrainStation
        | BuildingType::LandmarkMuseum
        | BuildingType::LandmarkContainerPort
        | BuildingType::EuropeanHaussmann
        | BuildingType::EuropeanCanalHouse
        | BuildingType::BrickCornerStore4Storey => 85.0,

        BuildingType::BrickShop2Storey
        | BuildingType::BrickBakeryCafe3Storey
        | BuildingType::BrickBookstoreArcade
        | BuildingType::EuropeanStuccoTownhouse
        | BuildingType::EuropeanOldTownHalfTimbered
        | BuildingType::IndustrialWarehouse
        | BuildingType::IndustrialPowerSubstation => 70.0,

        BuildingType::NewEnglandCape
        | BuildingType::NewEnglandColonial
        | BuildingType::NewEnglandSaltbox
        | BuildingType::NewEnglandFarmhouse
        | BuildingType::SuburbanRanch
        | BuildingType::SuburbanSplitLevel
        | BuildingType::SuburbanCraftsman
        | BuildingType::SuburbanContemporary => 65.0,
    }
}

/// Draw a building. Returns `true` when the kit generator drew it (B73c), so the
/// caller knows not to apply the old per-biome palette wash on top of art that
/// already wears the land's own traditions.
pub fn draw_building(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) -> bool {
    // If building is in an active construction stage, render the construction site
    if spec.stage != crate::types::ConstructionStage::Finished {
        let h = max_height(spec);
        crate::stages::draw_construction_stage(spec, light, sprite, ox, oy, h);
        return false;
    }

    // B73c: a lot whose spec is one of the generated families is drawn by the
    // kit, whatever family it is. Hand-made types (the signature towers, the
    // landmarks) fall through to their own art.
    if let Some(d) = crate::genkit::design_of(spec) {
        crate::genkit::draw(spec, &d, light, sprite, ox, oy);
        return true;
    }

    match spec.resolved_type() {
        // New England Houses
        BuildingType::NewEnglandCape => houses::draw_cape(spec, light, sprite, ox, oy),
        BuildingType::NewEnglandColonial => houses::draw_colonial(spec, light, sprite, ox, oy),
        BuildingType::NewEnglandSaltbox => houses::draw_saltbox(spec, light, sprite, ox, oy),
        BuildingType::NewEnglandFarmhouse => houses::draw_farmhouse(spec, light, sprite, ox, oy),

        // Brick Main-Street Shops
        BuildingType::BrickShop2Storey => shops::draw_shop_2storey(spec, light, sprite, ox, oy),
        BuildingType::BrickBakeryCafe3Storey => shops::draw_cafe_3storey(spec, light, sprite, ox, oy),
        BuildingType::BrickCornerStore4Storey => shops::draw_corner_store_4storey(spec, light, sprite, ox, oy),
        BuildingType::BrickBookstoreArcade => shops::draw_bookstore_arcade(spec, light, sprite, ox, oy),

        // European Townhouses
        BuildingType::EuropeanHaussmann => townhouses::draw_haussmann(spec, light, sprite, ox, oy),
        BuildingType::EuropeanStuccoTownhouse => townhouses::draw_mediterranean_stucco(spec, light, sprite, ox, oy),
        BuildingType::EuropeanCanalHouse => townhouses::draw_canal_house(spec, light, sprite, ox, oy),
        BuildingType::EuropeanOldTownHalfTimbered => townhouses::draw_half_timbered(spec, light, sprite, ox, oy),

        // Suburban Houses
        BuildingType::SuburbanRanch => suburban::draw_ranch(spec, light, sprite, ox, oy),
        BuildingType::SuburbanSplitLevel => suburban::draw_split_level(spec, light, sprite, ox, oy),
        BuildingType::SuburbanCraftsman => suburban::draw_craftsman(spec, light, sprite, ox, oy),
        BuildingType::SuburbanContemporary => suburban::draw_contemporary(spec, light, sprite, ox, oy),

        // Apartments (8)
        BuildingType::ApartmentBrickWalkup => apartments::draw_brick_walkup(spec, light, sprite, ox, oy),
        BuildingType::ApartmentBrownstoneTrio => apartments::draw_brownstone(spec, light, sprite, ox, oy),
        BuildingType::ApartmentModernMidrise => apartments::draw_modern_midrise(spec, light, sprite, ox, oy),
        BuildingType::ApartmentCourtyard => apartments::draw_courtyard(spec, light, sprite, ox, oy),
        BuildingType::ApartmentBayWindowTenement => apartments::draw_bay_window_tenement(spec, light, sprite, ox, oy),
        BuildingType::ApartmentCornerBalcony => apartments::draw_corner_balcony(spec, light, sprite, ox, oy),
        BuildingType::ApartmentArtDecoMasonry => apartments::draw_art_deco_masonry(spec, light, sprite, ox, oy),
        BuildingType::ApartmentTerraceGarden => apartments::draw_terrace_garden(spec, light, sprite, ox, oy),

        // Offices (8)
        BuildingType::OfficeClassicStone => offices::draw_classic_stone(spec, light, sprite, ox, oy),
        BuildingType::OfficeArtDeco => offices::draw_art_deco(spec, light, sprite, ox, oy),
        BuildingType::OfficeModernGlass => offices::draw_modern_glass(spec, light, sprite, ox, oy),
        BuildingType::OfficeCorporatePlaza => offices::draw_corporate_plaza(spec, light, sprite, ox, oy),
        BuildingType::OfficeTealCurtain => offices::draw_teal_curtain(spec, light, sprite, ox, oy),
        BuildingType::OfficeBronzePavilion => offices::draw_bronze_pavilion(spec, light, sprite, ox, oy),
        BuildingType::OfficeBrutalistConcrete => offices::draw_brutalist_concrete(spec, light, sprite, ox, oy),
        BuildingType::OfficePostmodernClock => offices::draw_postmodern_clock(spec, light, sprite, ox, oy),

        // Towers (16)
        BuildingType::TowerArtDecoSpire => towers::draw_art_deco_spire(spec, light, sprite, ox, oy),
        BuildingType::TowerGlassCurtainWall => towers::draw_glass_curtain_wall(spec, light, sprite, ox, oy),
        BuildingType::TowerTieredTerraces => towers::draw_tiered_terraces(spec, light, sprite, ox, oy),
        BuildingType::TowerCrossBraced => towers::draw_cross_braced(spec, light, sprite, ox, oy),
        BuildingType::TowerSlantedGlass => towers::draw_slanted_glass(spec, light, sprite, ox, oy),
        BuildingType::TowerCylindricalCrown => towers::draw_cylindrical_crown(spec, light, sprite, ox, oy),
        BuildingType::TowerReflectiveBands => towers::draw_reflective_bands(spec, light, sprite, ox, oy),
        BuildingType::TowerSlenderPinnacle => towers::draw_slender_pinnacle(spec, light, sprite, ox, oy),
        BuildingType::TowerTealObelisk => towers::draw_teal_obelisk(spec, light, sprite, ox, oy),
        BuildingType::TowerDarkBronzeCorporate => towers::draw_dark_bronze_corporate(spec, light, sprite, ox, oy),
        BuildingType::TowerWhiteLatticeCrown => towers::draw_white_lattice_crown(spec, light, sprite, ox, oy),
        BuildingType::TowerSilverMirrorBlade => towers::draw_silver_mirror_blade(spec, light, sprite, ox, oy),
        BuildingType::TowerBlackGraniteSpire => towers::draw_black_granite_spire(spec, light, sprite, ox, oy),
        BuildingType::TowerCurvedGlassSail => towers::draw_curved_glass_sail(spec, light, sprite, ox, oy),
        BuildingType::TowerDiagridHex => towers::draw_diagrid_hex(spec, light, sprite, ox, oy),
        BuildingType::TowerHelipadHeadquarters => towers::draw_helipad_headquarters(spec, light, sprite, ox, oy),
        BuildingType::TowerGenerated => {
            if let Some(d) = crate::genkit::design_of(spec) {
                crate::genkit::draw(spec, &d, light, sprite, ox, oy);
            }
        }
        // B73c's generated families were drawn above; a spec cannot reach here
        // with one of them (kept for exhaustiveness).
        BuildingType::HouseGenerated
        | BuildingType::TownhouseGenerated
        | BuildingType::ShopGenerated
        | BuildingType::ApartmentGenerated
        | BuildingType::OfficeGenerated
        | BuildingType::IndustrialGenerated
        | BuildingType::CivicGenerated => {}

        // Industrial
        BuildingType::IndustrialBrickFactory => industrial::draw_brick_factory(spec, light, sprite, ox, oy),
        BuildingType::IndustrialWarehouse => industrial::draw_warehouse(spec, light, sprite, ox, oy),
        BuildingType::IndustrialSilos => industrial::draw_silos(spec, light, sprite, ox, oy),
        BuildingType::IndustrialPowerSubstation => industrial::draw_power_substation(spec, light, sprite, ox, oy),

        // Civic
        BuildingType::CivicSchool => civic::draw_school(spec, light, sprite, ox, oy),
        BuildingType::CivicTownHall => civic::draw_town_hall(spec, light, sprite, ox, oy),
        BuildingType::CivicLibrary => civic::draw_library(spec, light, sprite, ox, oy),
        BuildingType::CivicFireStation => civic::draw_fire_station(spec, light, sprite, ox, oy),
        BuildingType::CivicHospital => civic::draw_hospital(spec, light, sprite, ox, oy),
        BuildingType::CivicChurch => civic::draw_school(spec, light, sprite, ox, oy),

        // Landmarks (10)
        BuildingType::LandmarkStadium => landmarks::draw_stadium(spec, light, sprite, ox, oy),
        BuildingType::LandmarkTrainStation => landmarks::draw_train_station(spec, light, sprite, ox, oy),
        BuildingType::LandmarkMuseum => landmarks::draw_museum(spec, light, sprite, ox, oy),
        BuildingType::LandmarkTownHall => landmarks::draw_town_hall(spec, light, sprite, ox, oy),
        BuildingType::LandmarkCathedral => landmarks::draw_cathedral(spec, light, sprite, ox, oy),
        BuildingType::LandmarkSignatureTower => landmarks::draw_signature_tower(spec, light, sprite, ox, oy),
        BuildingType::LandmarkLighthouse => landmarks::draw_lighthouse(spec, light, sprite, ox, oy),
        BuildingType::LandmarkContainerPort => landmarks::draw_container_port(spec, light, sprite, ox, oy),
        BuildingType::LandmarkFerrisWheel => landmarks::draw_ferris_wheel(spec, light, sprite, ox, oy),
        BuildingType::LandmarkBridgePylon => landmarks::draw_bridge_pylon(spec, light, sprite, ox, oy),
    }
    false
}
