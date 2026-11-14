//! B100/D95: a building sprite stands inside its own lot. Nothing it draws at
//! ground level (walls, a porch, paving) may reach past the lot's diamond, or
//! it shares ground with the neighbour and the painter's order cannot be right.
//!
//! The sweep is over what the sim can place: every lot kind, size, style and a
//! few seeds, through the same `spec_for` + `draw` the wallpaper uses.

use buildkit::{Light, Sprite, draw, ground_overhang, spec_for};
use render::city::{LandmarkKind, ObjKind, Stage};
use render::CityObj;

/// Pixels past the lot diamond tolerated at ground level.
const TOLERANCE: i32 = 1;

fn obj(kind: ObjKind, w: i32, h: i32, storeys: u8, style: u8, seed: u32) -> CityObj {
    CityObj {
        x: 10,
        y: 10,
        w,
        h,
        level: 0,
        kind,
        storeys,
        style,
        stage: Stage::Finished,
        seed,
        front: (seed % 4) as u8,
        t: 0.0,
        kit: 0,
        wear: Default::default(),
    }
}

#[test]
fn every_building_the_sim_places_fits_its_lot() {
    let light = Light::day();
    // (kind, the sizes the sim places it at). Landmarks have one size each
    // (`sim::landmarks::LandmarkKind::footprint`); a farmstead's house is 2x2.
    let free = [(1, 1), (2, 1), (1, 2), (2, 2), (3, 2), (3, 3), (4, 3)];
    let mut cases: Vec<(ObjKind, Vec<(i32, i32)>)> = [
        ObjKind::House,
        ObjKind::Church,
        ObjKind::Store,
        ObjKind::Shop,
        ObjKind::Townhouse,
        ObjKind::Apartment,
        ObjKind::Office,
        ObjKind::Tower,
        ObjKind::Industry,
        ObjKind::Hall,
    ]
    .into_iter()
    .map(|k| (k, free.to_vec()))
    .collect();
    for (k, size) in [
        (LandmarkKind::Cathedral, (3, 4)),
        (LandmarkKind::Station, (5, 3)),
        (LandmarkKind::School, (4, 3)),
        (LandmarkKind::Library, (3, 3)),
        (LandmarkKind::FireStation, (3, 3)),
        (LandmarkKind::Hospital, (5, 4)),
        (LandmarkKind::Museum, (4, 3)),
        (LandmarkKind::Stadium, (6, 5)),
        (LandmarkKind::SignatureTower, (3, 3)),
        (LandmarkKind::Lighthouse, (2, 2)),
        (LandmarkKind::Port, (5, 4)),
        (LandmarkKind::FerrisWheel, (3, 3)),
    ] {
        cases.push((ObjKind::Landmark(k), vec![size]));
    }
    let mut bad: std::collections::BTreeMap<String, (i32, usize)> = Default::default();
    let mut drawn = 0;
    for (kind, sizes) in &cases {
        let kind = *kind;
        for &(w, h) in sizes {
            for style in [0u8, 1, 2, 3, 4, 5, 6] {
                // Style 6 on a house is the farmstead's farmhouse: 2x2 only.
                if kind == ObjKind::House && style == 6 && (w, h) != (2, 2) {
                    continue;
                }
                for seed in [1u32, 2, 3] {
                    let storeys = match kind {
                        ObjKind::Tower => 20,
                        ObjKind::Office | ObjKind::Apartment => 6,
                        _ => 2,
                    };
                    let o = obj(kind, w, h, storeys, style, seed.wrapping_mul(0x9E37_79B1));
                    let Some(spec) = spec_for(&o) else { continue };
                    let mut s = Sprite::new(1, 1);
                    draw(&spec, &light, &mut s);
                    drawn += 1;
                    let (lw, lh) = (spec.footprint.0.max(1) as i32, spec.footprint.1.max(1) as i32);
                    let over = ground_overhang(&s, lw, lh);
                    if over > TOLERANCE {
                        let key = format!("{kind:?} {lw}x{lh} {:?}/{:?}", spec.function, spec.building_type);
                        let e = bad.entry(key).or_insert((0, 0));
                        e.0 = e.0.max(over);
                        e.1 += 1;
                    }
                }
            }
        }
    }
    assert!(drawn > 500, "the sweep drew {drawn} sprites");
    let list: Vec<String> = bad.iter().map(|(k, (o, n))| format!("{k}: up to {o} px ({n} sprites)")).collect();
    assert!(list.is_empty(), "sprites past their lot ({drawn} drawn):\n{}", list.join("\n"));
}
