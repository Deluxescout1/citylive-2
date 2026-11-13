//! B76 round 3: snow knows the surface it lies on.
//!
//! The builder stamps every pixel with what it is (a flat deck, a slope of a
//! given pitch, a ridge, a sill, a coping, the top or the side of a chimney),
//! and the renderer paints the load by that. These tests keep the stamping
//! honest for the shapes a town actually has.

use buildkit::genkit::{design_of, Family, RoofForm};
use buildkit::{BuildingFunction, BuildingSpec, BuildingStyle, ConstructionStage, Era, Light};
use render::snow;

fn light() -> Light {
    buildkit::light_for([1.0, 1.0, 1.0], 0.0, 12.25)
}

fn spec(f: Family, seed: u64, storeys: u8, style: BuildingStyle) -> BuildingSpec {
    BuildingSpec {
        building_type: None,
        function: f.function(),
        landmark_kind: None,
        style,
        footprint: (2, 2),
        storeys,
        era: Era::Town,
        stage: ConstructionStage::Finished,
        age: 8.0,
        seed: 0x9E37_79B9u64.wrapping_mul(seed + 1) ^ 0x5DEE_CE66,
        variant: ((seed * 37 + 11) % 251) as u8,
        biome: buildkit::BiomeStyle::Temperate,
    }
}

fn draw(f: Family, seed: u64, storeys: u8, style: BuildingStyle) -> buildkit::Sprite {
    let mut s = buildkit::Sprite::new(1, 1);
    buildkit::draw(&spec(f, seed, storeys, style), &light(), &mut s);
    s
}

fn count(s: &buildkit::Sprite) -> [usize; 16] {
    let mut n = [0usize; 16];
    for (&px, &t) in s.px.iter().zip(s.surf.iter()) {
        if px != 0 {
            n[t as usize] += 1;
        }
    }
    n
}

#[test]
fn every_family_has_a_surface_snow_can_lie_on() {
    for (f, storeys, style) in [
        (Family::House, 2u8, BuildingStyle::Suburban),
        (Family::Townhouse, 3, BuildingStyle::European),
        (Family::Shop, 3, BuildingStyle::BrickMainStreet),
        (Family::Apartment, 5, BuildingStyle::BrickMainStreet),
        (Family::Office, 6, BuildingStyle::ModernGlass),
        (Family::Industrial, 2, BuildingStyle::Industrial),
        (Family::Tower, 26, BuildingStyle::ModernGlass),
    ] {
        for seed in 0..6u64 {
            let s = draw(f, seed, storeys, style);
            let n = count(&s);
            let snowed = n[snow::FLAT as usize]
                + n[snow::SLOPE_GENTLE as usize]
                + n[snow::SLOPE_MEDIUM as usize]
                + n[snow::SLOPE_STEEP as usize]
                + n[snow::EQUIP_TOP as usize];
            assert!(
                snowed > 20,
                "{f:?} seed {seed} has almost no surface snow can lie on: {n:?}"
            );
        }
    }
}

#[test]
fn a_flat_house_has_a_flat_deck_and_a_chimney_that_stands_out_of_it() {
    // Find a flat-roofed house: its deck is flat, its stack is not.
    for seed in 0..40u64 {
        let sp = spec(Family::House, seed, 2, BuildingStyle::Suburban);
        let Some(d) = design_of(&sp) else { continue };
        if !matches!(d.roof, RoofForm::Flat) || d.chimney == 0 {
            continue;
        }
        let s = draw(Family::House, seed, 2, BuildingStyle::Suburban);
        let n = count(&s);
        assert!(n[snow::FLAT as usize] > 60, "a flat roof should be a deck: {n:?}");
        assert!(
            n[snow::EQUIP_SIDE as usize] > 0,
            "the stack should be a vertical face: {n:?}"
        );
        assert!(n[snow::EQUIP_TOP as usize] > 0, "the stack should have a cap: {n:?}");
        return;
    }
    panic!("no flat-roofed house with a chimney in 40 seeds");
}

#[test]
fn a_steep_roof_and_a_shallow_one_are_told_apart() {
    // Across the house range the pitch varies, so both buckets must appear.
    let mut seen = [false; 3];
    for seed in 0..60u64 {
        let s = draw(Family::House, seed, 2, BuildingStyle::NewEngland);
        let n = count(&s);
        for (i, tag) in [snow::SLOPE_GENTLE, snow::SLOPE_MEDIUM, snow::SLOPE_STEEP]
            .into_iter()
            .enumerate()
        {
            if n[tag as usize] > 30 {
                seen[i] = true;
            }
        }
    }
    assert!(seen[0], "no shallow slope was ever tagged");
    assert!(seen[1] || seen[2], "no steeper slope was ever tagged");
}

#[test]
fn surfaces_are_only_placed_on_pixels_that_were_drawn() {
    for (f, storeys, style) in [
        (Family::House, 2u8, BuildingStyle::Suburban),
        (Family::Apartment, 5, BuildingStyle::BrickMainStreet),
        (Family::Tower, 24, BuildingStyle::ModernGlass),
    ] {
        let s = draw(f, 3, storeys, style);
        for i in 0..s.px.len() {
            if s.surf[i] != snow::NONE {
                assert_ne!(s.px[i], 0, "{f:?}: a surface tag on an empty pixel ({i})");
            }
        }
    }
}

#[test]
fn the_hand_made_catalogue_gets_the_same_rule() {
    // A signature tower is drawn by the catalogue's own art (no kit): its top
    // faces are found from the depth plane, so it is not left bare.
    let mut sp = spec(Family::Tower, 2, 26, BuildingStyle::ModernGlass);
    sp.building_type = Some(buildkit::BuildingType::TowerArtDecoSpire);
    let mut s = buildkit::Sprite::new(1, 1);
    buildkit::draw(&sp, &light(), &mut s);
    let n = count(&s);
    assert!(
        n[snow::FLAT as usize] + n[snow::EQUIP_TOP as usize] > 20,
        "the catalogue's own tower has no surface snow can lie on: {n:?}"
    );
}

#[test]
fn the_surface_plane_costs_an_eighth_of_the_pixels() {
    let s = draw(Family::Apartment, 1, 5, BuildingStyle::BrickMainStreet);
    let (_, _, _, _) = (s.w, s.h, 0, 0);
    let packed = snow::pack(&s.surf);
    assert_eq!(packed.len(), (s.surf.len() + 1) / 2);
    // Two pixels a byte: half a byte per sprite pixel, versus four for the
    // colour, so the plane costs an eighth of what the sprite's colours do.
    assert!(packed.len() * 2 >= s.surf.len() - 1);
}

/// The functions the sheet and the tests both want, kept explicit.
#[allow(dead_code)]
fn _types(_: BuildingFunction) {}
