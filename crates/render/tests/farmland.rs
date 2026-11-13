//! B80: farmland follows the seasons, per crop.

use map::{Crop, Kind, Tile, TileMap};
use render::farmland::{FieldTile, field_px};
use render::palette;

/// A flat map with one field of `crop` in the middle.
fn field_map(crop: Crop) -> TileMap {
    let mut m = map::generate(3, 40, 40);
    for y in 0..40 {
        for x in 0..40 {
            m.set(x, y, Tile::new(3, Kind::Grass));
        }
    }
    for y in 8..16 {
        for x in 8..16 {
            let mut t = Tile::new(3, Kind::Field);
            map::set_crop(&mut t.aux, crop);
            m.set(x, y, t);
        }
    }
    m
}

/// The average colour of a field tile at one stage and snow cover.
fn average(crop: Crop, stage: f32, snow: f32) -> (f32, f32, f32) {
    let m = field_map(crop);
    let look = render::biome::look(map::Biome::Temperate);
    let ft = FieldTile::of(&m, 11, 11, look, stage, snow);
    let base = palette::rgb(0x7a, 0x55, 0x37);
    let (mut r, mut g, mut b, mut n) = (0.0, 0.0, 0.0, 0.0);
    for py in 0..64 {
        for px in 0..64 {
            let (u, v) = (px as f32 / 64.0, py as f32 / 64.0);
            let c = field_px(&ft, base, 600 + px, 300 + py, 11.0 + u, 11.0 + v);
            r += palette::r(c) as f32;
            g += palette::g(c) as f32;
            b += palette::b(c) as f32;
            n += 1.0;
        }
    }
    (r / n, g / n, b / n)
}

/// The stage of the crop year on a real date.
fn stage_on(month: u8, day: u8) -> f32 {
    world::SeasonLook::at(&world::Date::new(2026, month, day, 12), 41.5).field_stage
}

#[test]
fn a_cereal_field_changes_with_the_season() {
    let spring = average(Crop::Cereal, stage_on(3, 20), 0.0); // ploughed
    let june = average(Crop::Cereal, stage_on(6, 5), 0.0); // green
    let august = average(Crop::Cereal, stage_on(8, 12), 0.0); // gold
    let october = average(Crop::Cereal, stage_on(10, 3), 0.0); // stubble
    let winter = average(Crop::Cereal, stage_on(1, 15), 0.9); // snow
    let green_of = |c: (f32, f32, f32)| c.1 - 0.5 * (c.0 + c.2);
    // Ploughed: brown (red over green over blue).
    assert!(spring.0 > spring.1 && spring.1 > spring.2, "ploughed earth is brown: {spring:?}");
    // The crop is greener in June than the bare earth was.
    assert!(green_of(june) > green_of(spring) + 4.0, "green in June: {june:?} vs {spring:?}");
    // The ripe crop is golden: strong red and green, little blue.
    assert!(august.0 > 170.0 && august.1 > 130.0 && august.2 < 120.0, "gold in August: {august:?}");
    // Snow in the winter: bright.
    assert!(winter.0 > 200.0 && winter.1 > 200.0 && winter.2 > 200.0, "snow in January: {winter:?}");
    // And every pair of dates differs.
    let all = [spring, june, august, october, winter];
    for i in 0..all.len() {
        for j in i + 1..all.len() {
            let d = (all[i].0 - all[j].0).abs() + (all[i].1 - all[j].1).abs() + (all[i].2 - all[j].2).abs();
            assert!(d > 6.0, "dates {i} and {j} look alike: {:?} {:?}", all[i], all[j]);
        }
    }
}

#[test]
fn every_arable_crop_follows_the_year_and_has_its_own_look() {
    for crop in [Crop::Cereal, Crop::Maize, Crop::Veg] {
        let bare = average(crop, 0.04, 0.0);
        let grown = average(crop, 0.55, 0.0);
        let d = (bare.0 - grown.0).abs() + (bare.1 - grown.1).abs() + (bare.2 - grown.2).abs();
        assert!(d > 8.0, "{} does not change between ploughing and growth: {bare:?} {grown:?}", crop.name());
    }
    // Different crops are different pictures.
    let a = average(Crop::Cereal, 0.55, 0.0);
    let b = average(Crop::Veg, 0.55, 0.0);
    let c = average(Crop::Paddy, 0.55, 0.0);
    assert!((a.1 - b.1).abs() + (a.0 - b.0).abs() > 4.0, "{a:?} {b:?}");
    assert!((a.2 - c.2).abs() + (b.2 - c.2).abs() > 8.0, "rice stands in water: {c:?}");
}
