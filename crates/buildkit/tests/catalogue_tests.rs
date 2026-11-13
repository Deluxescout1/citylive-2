//! Integration tests for the buildkit catalogue.

use std::collections::HashSet;
use buildkit::{BuildingType, Light, Sprite, draw};

#[test]
fn test_all_types_render_4_variants_day_and_night() {
    assert!(BuildingType::ALL.len() >= 46, "catalogue must have >= 46 types");

    for &bt in BuildingType::ALL {
        for variant in 0..4 {
            let spec = bt.default_spec(42, variant);

            // Test Day render
            let light_day = Light::day();
            let mut sprite_day = Sprite::new(1, 1);
            draw(&spec, &light_day, &mut sprite_day);
            assert!(sprite_day.w > 0, "width must be > 0 for {:?} v{}", bt, variant);
            assert!(sprite_day.h > 0, "height must be > 0 for {:?} v{}", bt, variant);
            let has_non_transparent_day = sprite_day.px.iter().any(|&c| c != 0);
            assert!(has_non_transparent_day, "{:?} v{} day has non-transparent pixels", bt, variant);

            // Test Night render
            let light_night = Light::night();
            let mut sprite_night = Sprite::new(1, 1);
            draw(&spec, &light_night, &mut sprite_night);
            assert_eq!(sprite_night.w, sprite_day.w);
            assert_eq!(sprite_night.h, sprite_day.h);
            let has_non_transparent_night = sprite_night.px.iter().any(|&c| c != 0);
            assert!(has_non_transparent_night, "{:?} v{} night has non-transparent pixels", bt, variant);

            // Test depth mask has valid depth values
            let has_depth = sprite_day.depth.iter().any(|&d| d > 0);
            assert!(has_depth, "{:?} v{} has non-zero depth values", bt, variant);
        }
    }
}

#[test]
fn test_grounded_and_closed_silhouette() {
    let light = Light::day();

    for &bt in BuildingType::ALL {
        for variant in 0..4 {
            let spec = bt.default_spec(42, variant);
            let mut sprite = Sprite::new(1, 1);
            draw(&spec, &light, &mut sprite);

            // 1. Grounded test: sprite must contain solid opaque pixels at ground level (depth <= 6)
            let grounded = sprite.depth.iter().any(|&d| d > 0 && d <= 6);
            assert!(grounded, "{:?} v{} must be grounded (have wall/base pixels near ground level)", bt, variant);

            // 2. Closed silhouette test: verify no 1-pixel subpixel sampling dropout holes inside main wall columns
            let is_open_structure = bt == BuildingType::TowerSlenderPinnacle
                || bt == BuildingType::LandmarkSignatureTower
                || bt == BuildingType::TowerWhiteLatticeCrown
                || bt == BuildingType::LandmarkFerrisWheel
                || bt == BuildingType::LandmarkBridgePylon
                || bt == BuildingType::LandmarkContainerPort;

            if !is_open_structure {
                let x_start = sprite.ox - (spec.footprint.1 as i32 * 16);
                let x_end = sprite.ox + (spec.footprint.0 as i32 * 16);
                for x in x_start.max(0)..x_end.min(sprite.w as i32) {
                    let mut in_shape = false;
                    let mut hole_len = 0;
                    for y in 0..sprite.h {
                        let is_opaque = (sprite.px[y * sprite.w + x as usize] & 0xFF00_0000) != 0;
                        if is_opaque {
                            if hole_len == 1 && in_shape {
                                panic!("{:?} v{} has a 1-pixel dropout hole at x={}, y={}", bt, variant, x, y);
                            }
                            in_shape = true;
                            hole_len = 0;
                        } else if in_shape {
                            hole_len += 1;
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn test_construction_stages() {
    let light = Light::day();
    for stage in [
        buildkit::ConstructionStage::Excavation,
        buildkit::ConstructionStage::Foundation,
        buildkit::ConstructionStage::Framing,
        buildkit::ConstructionStage::Cladding,
    ] {
        let mut spec = BuildingType::TowerGlassCurtainWall.default_spec(42, 0);
        spec.stage = stage;
        let mut sprite = Sprite::new(1, 1);
        draw(&spec, &light, &mut sprite);
        assert!(sprite.w > 0);
        assert!(sprite.h > 0);
        let has_opaque = sprite.px.iter().any(|&c| c != 0);
        assert!(has_opaque, "{:?} stage must have opaque pixels", stage);
    }
}

#[test]
fn test_row_variety_yields_at_least_6_distinct_sprites() {
    let light = Light::day();
    let mut sprites: Vec<Vec<u32>> = Vec::new();

    for lot_idx in 0..12 {
        let seed = (lot_idx as u64 % 6) * 0x9E37_79B9;
        let variant = (lot_idx % 4) as u8;
        let spec = buildkit::BuildingSpec {
            building_type: None, // uses resolved_type()
            function: buildkit::BuildingFunction::Apartment,
            style: buildkit::BuildingStyle::BrickMainStreet,
            footprint: (2, 2),
            storeys: 5,
            era: buildkit::Era::City,
            stage: buildkit::ConstructionStage::Finished,
            age: ((lot_idx % 3) as f32) * 20.0,
            seed,
            variant,
            landmark_kind: None,
            biome: buildkit::BiomeStyle::Temperate,
        };
        let mut sprite = Sprite::new(1, 1);
        draw(&spec, &light, &mut sprite);

        if !sprites.iter().any(|existing| existing == &sprite.px) {
            sprites.push(sprite.px);
        }
    }

    assert!(
        sprites.len() >= 6,
        "A 12-lot row of the same function and style must yield >= 6 visually distinct sprites, got {}",
        sprites.len()
    );
}

#[test]
fn test_no_labels_or_text_in_sprites() {
    let light = Light::day();
    let label_colors = [
        render::palette::rgb(0x8a, 0x98, 0xaa),
        render::palette::rgb(0xa0, 0xae, 0xc0),
        render::palette::rgb(0xe2, 0xe8, 0xf0),
    ];
    for &bt in BuildingType::ALL {
        for variant in 0..4 {
            let spec = bt.default_spec(42, variant);
            let mut sprite = Sprite::new(1, 1);
            draw(&spec, &light, &mut sprite);
            for &c in &sprite.px {
                for &lc in &label_colors {
                    assert_ne!(c, lc, "{:?} v{} sprite contains label color 0x{:08x}", bt, variant, lc);
                }
            }
        }
    }
}

#[test]
fn test_determinism() {
    let spec = BuildingType::TowerArtDecoSpire.default_spec(12345, 0);
    let light = Light::night();

    let mut s1 = Sprite::new(1, 1);
    let mut s2 = Sprite::new(1, 1);

    draw(&spec, &light, &mut s1);
    draw(&spec, &light, &mut s2);

    assert_eq!(s1.w, s2.w);
    assert_eq!(s1.h, s2.h);
    assert_eq!(s1.ox, s2.ox);
    assert_eq!(s1.oy, s2.oy);
    assert_eq!(s1.px, s2.px);
    assert_eq!(s1.depth, s2.depth);
}

#[test]
fn test_cacheability_hash_eq() {
    let mut set = HashSet::new();

    let spec1 = BuildingType::NewEnglandCape.default_spec(999, 0);
    let mut spec2 = spec1.clone();
    spec2.variant = 1;

    set.insert((spec1.clone(), Light::DAY_BUCKET));
    set.insert((spec2.clone(), Light::DAY_BUCKET));
    set.insert((spec1.clone(), Light::NIGHT_BUCKET));

    assert_eq!(set.len(), 3);
    assert!(set.contains(&(spec1, Light::DAY_BUCKET)));
}
