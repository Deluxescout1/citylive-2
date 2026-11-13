//! B80: the countryside. Farms are laid when a world is founded; they never sit
//! on water, a road or a building; every farmstead has a farmhouse beside its
//! barn; and the town eats the farmland nearest it while the outer farms stay.

use map::{Kind, world_for_viewport};
use sim::{LotKind, Pace, Sim};

const FB_W: i32 = 7040;
const FB_H: i32 = 1440;
const SKY: f32 = 0.30;

/// A world at (about) half the real frame, founded the way the viewer does it.
fn world(seed: u64, biome: Option<map::Biome>) -> Sim {
    let vp = world_for_viewport(FB_W, FB_H);
    let focus = map::screen_to_tile(&vp, FB_W as f32 * 0.5, FB_H as f32 * 0.5);
    let mut s = Sim::with_focus_in(seed, Pace::Month, vp.w, vp.h, focus, biome);
    let cam = render_cam(&vp);
    s.set_viewport(cam.0, cam.1 + (SKY * FB_H as f32) as i32, FB_W, FB_H);
    s.set_horizon((SKY * FB_H as f32) as i32, (160.0 * FB_H as f32 / 1440.0) as i32);
    s
}

fn render_cam(vp: &map::Viewport) -> (i32, i32) {
    vp.origin
}

fn field_tiles(s: &Sim) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for y in 0..s.map.h {
        for x in 0..s.map.w {
            if s.map.at(x, y).kind == Kind::Field {
                v.push((x, y));
            }
        }
    }
    v
}

#[test]
fn a_farm_lot_has_a_farmhouse_beside_its_barn() {
    for seed in [5u64, 9, 13] {
        let s = world(seed, Some(map::Biome::Temperate));
        let barns: Vec<_> = s.lots.iter().filter(|l| l.kind == LotKind::Barn).collect();
        assert!(!barns.is_empty(), "seed {seed}: the countryside has no farm");
        for b in &barns {
            let house = s
                .lots
                .iter()
                .any(|h| h.kind == LotKind::Farmhouse && (h.x - b.x).abs() <= 6 && (h.y - b.y).abs() <= 6);
            assert!(house, "seed {seed}: the barn at {},{} has no farmhouse", b.x, b.y);
        }
        // Houses and barns come in pairs.
        let houses = s.lots.iter().filter(|l| l.kind == LotKind::Farmhouse).count();
        assert_eq!(houses, barns.len(), "seed {seed}");
        // And the farmland has crops: more than one kind.
        let mut kinds = std::collections::BTreeSet::new();
        for (x, y) in field_tiles(&s) {
            kinds.insert(map::crop_of(&s.map.at(x, y)) as u8);
        }
        assert!(kinds.len() >= 4, "seed {seed}: a patchwork wants several crops, got {kinds:?}");
    }
}

#[test]
fn fields_never_sit_on_water_roads_or_buildings() {
    for seed in [5u64, 9, 13, 21] {
        let mut s = world(seed, Some(map::Biome::Temperate));
        // At the start, and after the town has grown into the land.
        for days in [0.0f64, 12.0] {
            s.run_to(days * 86400.0);
            for (x, y) in field_tiles(&s) {
                let t = s.map.at(x, y);
                assert!(!t.kind.is_water() && !t.kind.is_lane(), "seed {seed}: field on water or road");
                assert_eq!(t.aux & (map::AUX_BRIDGE | map::AUX_HIGHWAY | map::AUX_RAMP | map::AUX_TECH), 0);
                assert!(
                    !s.lots.iter().any(|l| l.kind != LotKind::Barn && l.kind != LotKind::Farmhouse && l.covers(x, y)),
                    "seed {seed} day {days}: field under a building at {x},{y}"
                );
            }
            // No road or water tile carries a crop code.
            for y in 0..s.map.h {
                for x in 0..s.map.w {
                    let t = s.map.at(x, y);
                    if t.kind != Kind::Field {
                        assert_eq!(t.aux & map::AUX_CROP_MASK, 0, "seed {seed}: crop bits on {:?}", t.kind);
                    }
                }
            }
        }
    }
}

#[test]
fn the_town_takes_the_near_farms_and_the_far_ones_stay() {
    let mut s = world(5, Some(map::Biome::Temperate));
    let (fx, fy) = s.founding;
    let near = |s: &Sim, r: i32| {
        field_tiles(s)
            .into_iter()
            .filter(|&(x, y)| (x - fx).abs() <= r && (y - fy).abs() <= r)
            .count()
    };
    let far = |s: &Sim, r: i32| {
        field_tiles(s)
            .into_iter()
            .filter(|&(x, y)| (x - fx).abs() > r || (y - fy).abs() > r)
            .count()
    };
    let (n0, f0) = (near(&s, 24), far(&s, 24));
    s.run_to(20.0 * 86400.0);
    let (n1, f1) = (near(&s, 24), far(&s, 24));
    assert!(f0 > 0, "no outer farms to start with");
    assert!(n1 < n0 || n0 == 0, "the town did not take any of the near farmland: {n0} -> {n1}");
    assert!(f1 * 3 >= f0 * 2, "the outer farms did not stay: {f0} -> {f1}");
    // What is left near the town is whole fields, not slivers: every field tile
    // has a like neighbour.
    for (x, y) in field_tiles(&s) {
        let t = s.map.at(x, y);
        let c = map::crop_of(&t);
        let like = [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .filter(|(dx, dy)| {
                let n = s.map.at(x + dx, y + dy);
                n.kind == Kind::Field && (map::crop_of(&n) == c || c == map::Crop::Track)
            })
            .count();
        assert!(like >= 1, "a lone field tile at {x},{y} ({})", c.name());
    }
}

#[test]
fn every_land_but_the_tundra_farms_in_its_own_way() {
    use map::{Biome, Crop};
    for (biome, expect) in [
        (Biome::Temperate, Crop::Cereal),
        (Biome::Mediterranean, Crop::Vineyard),
        (Biome::Tropical, Crop::Paddy),
        (Biome::Highlands, Crop::Pasture),
        (Biome::Islands, Crop::Veg),
    ] {
        // A handful of seeds: the land has to farm *somewhere* in one of them.
        let mut found = false;
        for seed in [3u64, 7, 11, 17] {
            let s = world(seed, Some(biome));
            if field_tiles(&s).iter().any(|&(x, y)| map::crop_of(&s.map.at(x, y)) == expect) {
                found = true;
                break;
            }
        }
        assert!(found, "{biome:?} grows no {}", expect.name());
    }
    let s = world(3, Some(Biome::Tundra));
    assert!(field_tiles(&s).is_empty(), "the tundra grows no crops");
}

#[test]
fn a_saved_countryside_loads_back_the_same() {
    let mut s = world(5, Some(map::Biome::Temperate));
    s.run_to(3.0 * 86400.0);
    let dir = std::env::temp_dir().join(format!("b80-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("farm.save");
    s.save(&path).unwrap();
    let t = Sim::load(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(field_tiles(&s), field_tiles(&t));
    for (x, y) in field_tiles(&s) {
        assert_eq!(map::crop_of(&s.map.at(x, y)), map::crop_of(&t.map.at(x, y)));
    }
    let farm_lots = |s: &Sim| s.lots.iter().filter(|l| matches!(l.kind, LotKind::Barn | LotKind::Farmhouse)).count();
    assert_eq!(farm_lots(&s), farm_lots(&t));
}
