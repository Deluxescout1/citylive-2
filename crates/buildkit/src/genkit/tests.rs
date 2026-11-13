//! B73b's tests: the generator is deterministic, a big city gets many towers,
//! touching towers never share the triple, every tower stays inside its lot and
//! its height, and no crown goes back to the flat cyan band B99d removed.

use super::*;
use crate::types::{ConstructionStage, Light};

fn spec(seed: u64, style: BuildingStyle, footprint: (u8, u8), storeys: u8) -> BuildingSpec {
    BuildingSpec {
        building_type: None,
        function: BuildingFunction::Tower,
        landmark_kind: None,
        style,
        footprint,
        storeys,
        era: Era::Metropolis,
        stage: ConstructionStage::Finished,
        age: (seed % 3) as f32 * 20.0,
        seed,
        variant: 0,
        biome: BiomeStyle::Temperate,
    }
}

/// The districts a grown city puts towers in, cycled so the pick sees them all.
fn district(i: usize) -> BuildingStyle {
    const ALL: [BuildingStyle; 4] = [
        BuildingStyle::ModernGlass,
        BuildingStyle::HighEnd,
        BuildingStyle::BrickMainStreet,
        BuildingStyle::Industrial,
    ];
    ALL[i % ALL.len()]
}

#[test]
fn the_same_lot_always_gives_the_same_tower() {
    for seed in [7u64, 0x51ee_7, 0x9E37_79B9_7F4A_7C15, 42] {
        for style in [BuildingStyle::ModernGlass, BuildingStyle::BrickMainStreet] {
            let s = spec(seed, style, (2, 2), 24);
            let a = design_of(&s).expect("generated");
            let b = design_of(&s).expect("generated");
            assert_eq!(format!("{a:?}"), format!("{b:?}"), "seed {seed} is not deterministic");
        }
    }
}

#[test]
fn a_thousand_seeds_make_many_towers() {
    let mut keys = std::collections::HashSet::new();
    let mut whole = std::collections::HashSet::new();
    for i in 0..1000u64 {
        let storeys = 10 + (i % 25) as u8;
        let fp = [(1, 1), (1, 2), (2, 2)][(i as usize / 3) % 3];
        let s = spec(0x9E37_79B9u64.wrapping_mul(i + 1), district(i as usize), fp, storeys);
        let d = design_of(&s).expect("generated");
        keys.insert(d.key());
        whole.insert(format!("{:?}{:?}{:?}{:?}{:?}", d.key(), d.top, d.base, d.massing, d.storeys));
    }
    assert!(
        whole.len() >= 500,
        "1000 seeds gave only {} distinct towers ({} part triples)",
        whole.len(),
        keys.len()
    );
}

#[test]
fn touching_towers_never_share_the_triple() {
    // A terrace of towers in one street: each is picked, then nudged against
    // the ones already placed that touch it (the rule `lots::attach` applies).
    let mut placed: Vec<(i32, i32, (u8, u8), DesignKey)> = Vec::new();
    for i in 0..40i32 {
        let x = i * 2;
        let mut s = spec(0x1234_5678u64.wrapping_mul(i as u64 + 1), BuildingStyle::ModernGlass, (2, 2), 22);
        let clash = |k: DesignKey| {
            placed.iter().any(|&(nx, ny, (nw, nh), nk)| {
                let gap_x = (x - (nx + nw as i32)).max(nx - (x + 2));
                let gap_y = 0i32.max(-(0));
                gap_x <= 1 && gap_y <= 1 && nk == k
            })
        };
        if let Some(seed) = nudge_seed(&s, clash) {
            s.seed = seed;
        }
        let k = key_of(&s).expect("generated");
        placed.push((x, 0, (2, 2), k));
    }
    for (i, a) in placed.iter().enumerate() {
        for b in &placed[i + 1..] {
            let gap = (a.0 - (b.0 + 2)).max(b.0 - (a.0 + 2));
            if gap <= 1 {
                assert_ne!(a.3, b.3, "touching towers at {} and {} share {:?}", a.0, b.0, a.3);
            }
        }
    }
}

#[test]
fn every_tower_draws_inside_its_lot_and_its_height() {
    for i in 0..120u64 {
        let fp = [(1, 1), (1, 2), (2, 2)][(i as usize) % 3];
        let storeys = 10 + (i % 25) as u8;
        let s = spec(0xABCD_1234u64.wrapping_mul(i + 1), district(i as usize), fp, storeys);
        let d = design_of(&s).expect("generated");
        let mut sprite = crate::Sprite::new(1, 1);
        crate::draw(&s, &Light::day(), &mut sprite);
        let (min_x, max_x) = (-d.bd * 32.0, d.bw * 32.0);
        let (min_y, max_y) = (-(d.z_crown as f32), (d.bw + d.bd) * 16.0);
        let mut top_z = 0u16;
        for y in 0..sprite.h {
            for x in 0..sprite.w {
                let c = sprite.px[y * sprite.w + x];
                if (c >> 24) < 0x80 {
                    continue;
                }
                // The sprite's pixels are screen coordinates; measure from the
                // lot's own ground corner (the anchor at `ox`, `oy`).
                let (fx, fy) = ((x as i32 - sprite.ox) as f32, (y as i32 - sprite.oy) as f32);
                assert!(
                    fx >= min_x - 2.0 && fx <= max_x + 2.0 && fy >= min_y - 4.0 && fy <= max_y + 2.0,
                    "design {i} {d:?} drew at ({x},{y}) = ({fx},{fy}) outside x {min_x}..{max_x} y {min_y}..{max_y}"
                );
                top_z = top_z.max(sprite.depth[y * sprite.w + x]);
            }
        }
        assert!(
            top_z as i32 <= d.z_crown,
            "design {i} draws {} px high, its crown is {}",
            top_z,
            d.z_crown
        );
    }
}

#[test]
fn no_flat_cyan_band_on_a_generated_tower() {
    // B99d's rule, for the crowns the generator makes: a tower's top is a warm
    // lamp at night or metal, never a band of saturated cyan.
    for i in 0..40u64 {
        let s = spec(0x77u64.wrapping_mul(i + 1), district(i as usize), (2, 2), 26);
        for light in [
            Light::day(),
            crate::light_for([1.0, 1.0, 1.0], 0.60, 16.5),
            Light::night(),
        ] {
            let mut sprite = crate::Sprite::new(1, 1);
            crate::draw(&s, &light, &mut sprite);
            for &c in &sprite.px {
                if (c >> 24) < 0x80 {
                    continue;
                }
                let (r, g, b) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
                assert!(
                    !(b > 0xD0 && g > 0xC0 && r < 0x60),
                    "design {i} paints a cyan pixel #{:06x} at bucket {}",
                    c & 0xFF_FFFF,
                    light.bucket
                );
            }
        }
    }
}

#[test]
fn no_generated_tower_has_a_floating_crown() {
    // H32's bug class: a crown that reads as a slab hanging in the air. The
    // rule is `lots::tests::no_sprite_has_a_floating_roof`'s: a shape 6+ px
    // wide, 4+ rows tall, with a 4+ row gap under it.
    for i in 0..300u64 {
        let fp = [(1, 1), (1, 2), (2, 2)][(i as usize) % 3];
        let storeys = 10 + (i % 25) as u8;
        let s = spec(0x5EEDu64.wrapping_mul(i + 1), district(i as usize), fp, storeys);
        let d = design_of(&s).expect("generated");
        let mut sprite = crate::Sprite::new(1, 1);
        crate::draw(&s, &Light::day(), &mut sprite);
        for x in 0..sprite.w {
            let mut runs: Vec<(usize, usize)> = Vec::new();
            let mut start: Option<usize> = None;
            for y in 0..sprite.h {
                let on = sprite.px[y * sprite.w + x] != 0;
                match (on, start) {
                    (true, None) => start = Some(y),
                    (false, Some(a)) => {
                        runs.push((a, y - 1));
                        start = None;
                    }
                    _ => {}
                }
            }
            if let Some(a) = start {
                runs.push((a, sprite.h - 1));
            }
            for w in runs.windows(2) {
                let (a0, a1) = w[0];
                let (b0, _) = w[1];
                let upper = a1 - a0 + 1;
                let gap = b0 - a1 - 1;
                let mut width = 1;
                let mut xx = x;
                while xx > 0 && sprite.px[a0 * sprite.w + xx - 1] != 0 {
                    xx -= 1;
                    width += 1;
                }
                let mut xx = x;
                while xx + 1 < sprite.w && sprite.px[a0 * sprite.w + xx + 1] != 0 {
                    xx += 1;
                    width += 1;
                }
                if width >= 6 && upper >= 4 && gap >= 4 && std::env::var("B73B_ASCII").is_ok() {
                    println!("design {i} {d:?} sprite {}x{} ox {} oy {}", sprite.w, sprite.h, sprite.ox, sprite.oy);
                    for y in (a1.saturating_sub(6))..(b0 + 6).min(sprite.h) {
                        let mut line = String::new();
                        for xx in x.saturating_sub(25)..(x + 25).min(sprite.w) {
                            line.push(if sprite.px[y * sprite.w + xx] != 0 { '#' } else { '.' });
                        }
                        println!("{y:>4} {line}");
                    }
                }
                assert!(
                    !(width >= 6 && upper >= 4 && gap >= 4),
                    "design {i} ({:?}/{:?}/{:?} top {:?} base {:?} massing {:?}): column {x} has a {width}-wide, {upper}-row slab {gap} rows above the body",
                    d.body, d.facade, d.material, d.top, d.base, d.massing
                );
            }
        }
    }
}


#[test]
fn the_skyline_keeps_the_old_height_range() {
    // Round 2 item 1: the tallest towers of a grown city are as tall as the old
    // catalogue's (its towers ran 120-265 px; a generated tower's height is its
    // crown height), and the profile is varied: short, mid and tall all exist.
    let mut heights = Vec::new();
    for i in 0..240u64 {
        let storeys = 10 + (i % 25) as u8; // the sim's own 10-34 range
        let s = spec(0xC0FFEEu64.wrapping_mul(i + 1), BuildingStyle::ModernGlass, (2, 2), storeys);
        heights.push(design_of(&s).expect("generated").z_crown);
    }
    heights.sort_unstable();
    let min = heights[0];
    let median = heights[heights.len() / 2];
    let max = heights[heights.len() - 1];
    assert!(max >= 250, "the tallest generated tower is {max} px, the old art reached 265");
    assert!(min <= 190, "the shortest generated tower is {min} px: no mid-height profile");
    assert!(median < max - 20, "heights are a wall ({min}/{median}/{max})");
}

#[test]
fn crowns_carry_their_own_colours_and_most_are_dark() {
    // Round 2 item 3: a tower may light its crown in its own colour; most towers
    // are windows only.
    let mut lit = 0;
    let mut pals = std::collections::HashSet::new();
    let mut led = 0;
    let n = 200;
    for i in 0..n {
        let s = spec(0xBEEFu64.wrapping_mul(i as u64 + 1), BuildingStyle::ModernGlass, (2, 2), 26);
        let d = design_of(&s).expect("generated");
        if d.crown_lights {
            lit += 1;
            pals.insert(d.crown_pal);
        }
        led += d.led_edge as usize;
    }
    assert!(lit * 3 < n, "{lit} of {n} towers light their crowns: most should be windows only");
    assert!(lit > n / 20, "only {lit} of {n} towers light a crown: some should");
    assert!(pals.len() >= 3, "crown palettes are not varied: {pals:?}");
    assert!(led > 0 && led < n / 4, "{led} LED edges of {n}");
}

// ---------------------------------------------------------------------------
// B73c: every family, its variety and its determinism
// ---------------------------------------------------------------------------

use crate::BuildingType;

/// The families the brief names, each with the civic program it must keep.
pub(crate) fn b73c_families() -> Vec<(crate::BuildingType, (u8, u8), u8)> {
    use crate::BuildingType as T;
    vec![
        (T::HouseGenerated, (1, 1), 2),
        (T::HouseGenerated, (2, 1), 2),
        (T::TownhouseGenerated, (1, 2), 3),
        (T::ShopGenerated, (2, 1), 3),
        (T::ApartmentGenerated, (2, 2), 5),
        (T::OfficeGenerated, (2, 2), 5),
        (T::IndustrialGenerated, (2, 2), 2),
        (T::CivicSchool, (3, 2), 2),
        (T::CivicFireStation, (2, 1), 2),
        (T::CivicLibrary, (2, 2), 2),
        (T::CivicTownHall, (2, 2), 3),
        (T::CivicHospital, (3, 2), 4),
        (T::CivicChurch, (2, 2), 3),
    ]
}

/// A spec of one family at lot `i`: a different seed, street theme, district,
/// height and era, the way 100 lots of a type in a grown city differ.
pub(crate) fn b73c_spec(bt: BuildingType, footprint: (u8, u8), storeys: u8, i: u64) -> BuildingSpec {
    let seed = 0x9E37_79B9u64.wrapping_mul(i + 1) ^ 0x5DEE_CE66u64;
    let mut s = bt.default_spec(seed, ((i * 37 + 11) % 251) as u8);
    s.footprint = footprint;
    s.storeys = storeys + (i % 3) as u8;
    s.style = [
        BuildingStyle::NewEngland,
        BuildingStyle::BrickMainStreet,
        BuildingStyle::European,
        BuildingStyle::Suburban,
        BuildingStyle::ModernGlass,
        BuildingStyle::HighEnd,
        BuildingStyle::Industrial,
    ][(i as usize) % 7];
    s.biome = [
        BiomeStyle::Temperate,
        BiomeStyle::Temperate,
        BiomeStyle::Desert,
        BiomeStyle::Alpine,
        BiomeStyle::Tropical,
        BiomeStyle::Tundra,
        BiomeStyle::Coastal,
        BiomeStyle::Highlands,
        BiomeStyle::Savanna,
        BiomeStyle::Swamp,
    ][(i as usize) % 10];
    s
}

fn drawn(spec: &BuildingSpec, light: &Light) -> crate::Sprite {
    let mut sp = crate::Sprite::new(1, 1);
    crate::draw(spec, light, &mut sp);
    sp
}

/// How much two sprites differ, as a share of the pixels either of them draws.
/// A pixel counts as different when its colour is more than 24 away in total
/// channel distance, or when one draws and the other does not.
fn mismatch(a: &crate::Sprite, b: &crate::Sprite, ox: i32, oy: i32) -> f32 {
    let (wa, ha) = (a.w as i32, a.h as i32);
    let (wb, hb) = (b.w as i32, b.h as i32);
    let x0 = -ox;
    let y0 = -oy;
    let x1 = (wa.max(wb - ox)).max(x0 + 1);
    let y1 = (ha.max(hb - oy)).max(y0 + 1);
    let (mut diff, mut union) = (0usize, 0usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let pa = if x >= 0 && y >= 0 && x < wa && y < ha {
                a.px[(y * wa + x) as usize]
            } else {
                0
            };
            let (bx, by) = (x + ox, y + oy);
            let pb = if bx >= 0 && by >= 0 && bx < wb && by < hb {
                b.px[(by * wb + bx) as usize]
            } else {
                0
            };
            let da = (pa >> 24) >= 0x80;
            let db = (pb >> 24) >= 0x80;
            if !da && !db {
                continue;
            }
            union += 1;
            if da != db {
                diff += 1;
                continue;
            }
            let d = ((pa >> 16) & 0xFF) as i32 - ((pb >> 16) & 0xFF) as i32;
            let e = ((pa >> 8) & 0xFF) as i32 - ((pb >> 8) & 0xFF) as i32;
            let f = (pa & 0xFF) as i32 - (pb & 0xFF) as i32;
            if d.abs() + e.abs() + f.abs() > 24 {
                diff += 1;
            }
        }
    }
    diff as f32 / union.max(1) as f32
}

/// the owner's target (D82): at least 100 visibly different variants of every type.
/// "Visibly different" = two of them side by side are clearly not the same
/// building: at 1× at least 6 % of the pixels either draws differ (>72 pixels
/// on a 40×30 house — more than a door or a window, so a recolour of one detail
/// does not count).
#[test]
fn every_family_makes_a_hundred_visibly_different_buildings() {
    let light = Light::day();
    for (bt, fp, storeys) in b73c_families() {
        let sprites: Vec<crate::Sprite> = (0..100)
            .map(|i| drawn(&b73c_spec(bt, fp, storeys, i), &light))
            .collect();
        let (ox, oy) = (sprites[0].ox, sprites[0].oy);
        let mut closest = (1.0f32, 0usize, 0usize);
        for i in 0..sprites.len() {
            for j in (i + 1)..sprites.len() {
                let m = mismatch(&sprites[i], &sprites[j], ox, oy);
                if m < closest.0 {
                    closest = (m, i, j);
                }
            }
        }
        assert!(
            closest.0 >= 0.06,
            "{bt:?}: designs {} and {} are the same building ({:.2}% of pixels differ)",
            closest.1,
            closest.2,
            closest.0 * 100.0
        );
    }
}

/// The brief's second number: ≥ 200 combinations over 1000 seeds, per family.
#[test]
fn a_thousand_seeds_make_two_hundred_designs_per_family() {
    for (bt, fp, storeys) in b73c_families() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..1000u64 {
            let s = b73c_spec(bt, fp, storeys, i);
            if let Some(d) = design_of(&s) {
                seen.insert(format!(
                    "{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
                    d.plan, d.roof, d.roof_mat, d.finish, d.paint, d.facade, d.material, d.porch, d.top
                ));
            }
        }
        assert!(
            seen.len() >= 200,
            "{bt:?}: 1000 seeds gave only {} design combinations",
            seen.len()
        );
    }
}

/// The same lot always gives the same building, for every family.
#[test]
fn the_same_lot_always_gives_the_same_building() {
    for (bt, fp, storeys) in b73c_families() {
        for i in [0u64, 7, 41] {
            let s = b73c_spec(bt, fp, storeys, i);
            let a = design_of(&s).expect("generated");
            let b = design_of(&s).expect("generated");
            assert_eq!(format!("{a:?}"), format!("{b:?}"), "{bt:?} seed {i} is not deterministic");
        }
    }
}

/// Neighbours are never the same building: the rule `lots::attach` applies over
/// a row of touching lots, for every family.
#[test]
fn touching_neighbours_never_share_a_design() {
    for (bt, fp, storeys) in b73c_families() {
        let mut placed: Vec<(i32, i32, (u8, u8), DesignKey)> = Vec::new();
        let (w, h) = (fp.0 as i32, fp.1 as i32);
        for i in 0..40i32 {
            let x = i * (w + 1);
            let mut s = b73c_spec(bt, fp, storeys, i as u64);
            let clash = |k: DesignKey| {
                placed.iter().any(|&(nx, ny, (nw, nh), nk)| {
                    let gap_x = (x - (nx + nw as i32)).max(nx - (x + w));
                    let gap_y = (0 - (ny + nh as i32)).max(ny - (0 + h));
                    gap_x <= 1 && gap_y <= 1 && nk == k
                })
            };
            if let Some(seed) = nudge_seed(&s, clash) {
                s.seed = seed;
            }
            let d = design_of(&s).expect("generated");
            let k = d.key();
            let clash_after = placed.iter().any(|&(nx, ny, (nw, nh), nk)| {
                let gap_x = (x - (nx + nw as i32)).max(nx - (x + w));
                let gap_y = (0 - (ny + nh as i32)).max(ny - (0 + h));
                gap_x <= 1 && gap_y <= 1 && nk == k
            });
            assert!(!clash_after, "{bt:?}: house {i} is identical to its neighbour");
            placed.push((x, 0, (w as u8, h as u8), k));
        }
    }
}

/// Every building draws inside its lot: the drawn pixels never reach past the
/// footprint's projected ground diamond (the B74 overlap audit's rule).
#[test]
fn every_family_draws_inside_its_lot() {
    let light = Light::day();
    for (bt, fp, storeys) in b73c_families() {
        for i in 0..24u64 {
            let s = b73c_spec(bt, fp, storeys, i);
            let sp = drawn(&s, &light);
            let (fw, fh) = (fp.0 as i32, fp.1 as i32);
            // The lot's ground diamond: from the anchor (north corner) the
            // diamond spans fw*32 px right-of-anchor on the x edge and fh*32 on
            // the y edge, and (fw+fh)*16 px down. Allow the roof's eave and the
            // foundation a couple of pixels of lip.
            let mut top = i32::MAX;
            let mut bottom = i32::MIN;
            let mut left = i32::MAX;
            let mut right = i32::MIN;
            for y in 0..sp.h {
                for x in 0..sp.w {
                    if (sp.px[y * sp.w + x] >> 24) >= 0x80 {
                        top = top.min(y as i32);
                        bottom = bottom.max(y as i32);
                        left = left.min(x as i32);
                        right = right.max(x as i32);
                    }
                }
            }
            let (ox, oy) = (sp.ox, sp.oy);
            assert!(left >= ox - (fh * 32) - 8, "{bt:?} {i}: draws {} px left of its lot", ox - left);
            assert!(right <= ox + fw * 32 + 8, "{bt:?} {i}: draws {} px right of its lot", right - ox);
            assert!(bottom <= oy + (fw + fh) * 16 + 10, "{bt:?} {i}: draws below its lot");
            assert!(top >= oy - 160, "{bt:?} {i}: draws above its box");
        }
    }
}
