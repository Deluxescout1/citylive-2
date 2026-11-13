//! B69: trees sway with the wind.
//!
//! Calm air is still and free; a breeze moves the crowns and not the trunks; the
//! stronger the wind the further they lean; neighbours are out of step; and a
//! tree only ever repaints the rows that moved, never more than the frame.

use map::{Kind, Prop, Tile, TileMap, TreeSpecies, world_for_viewport};
use render::framebuffer::{Framebuffer, Rect};
use render::light::LightState;
use render::{Scene, SceneConfig, SceneMode};

const W: usize = 520;
const H: usize = 320;
const DT: f32 = 1.0 / 30.0;

/// A grass field planted with a grid of oaks (every third tile) and a pair of
/// pines, lit by a plain midday.
fn grove(species: TreeSpecies) -> (Scene, Vec<(i32, i32)>) {
    let vp = world_for_viewport(W as i32, H as i32);
    let mut map = TileMap::fill(vp.w, vp.h, Tile::flat(2), 5);
    let mut trees = Vec::new();
    for y in 0..map.h {
        for x in 0..map.w {
            let t = map.at(x, y);
            let plant = x % 3 == 1 && y % 3 == 1 && x > 3 && y > 3 && x < map.w - 3 && y < map.h - 3;
            let prop = if plant {
                trees.push((x, y));
                Prop::Tree { species, size: 1, variant: (x * 7 + y * 13) as u8 }
            } else {
                Prop::None
            };
            map.set(x, y, Tile { kind: Kind::Grass, prop, ..t });
        }
    }
    let mut scene = Scene::from_map_with_sky(
        map,
        SceneConfig {
            width: W,
            height: H,
            seed: 5,
            mode: SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        },
        0.28,
    );
    let mut ls = LightState::default();
    ls.ambient = [1.0; 3];
    scene.set_light_state(ls);
    while scene.shadow_tick(1.0) {}
    scene.flush_dirty();
    (scene, trees)
}

fn frame(scene: &mut Scene) -> Framebuffer {
    let mut fb = Framebuffer::new(W, H);
    scene.mark_full();
    scene.composite(&mut fb);
    fb
}

fn settle(scene: &mut Scene, kmh: f32, dir: f32) {
    scene.set_cloud_wind(dir, kmh);
    for _ in 0..450 {
        scene.tree_tick(DT);
        let _ = scene.take_pending();
    }
}

fn differing(a: &Framebuffer, b: &Framebuffer) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for y in 0..H {
        for x in 0..W {
            if (a.px()[y * W + x] & 0xFFFFFF) != (b.px()[y * W + x] & 0xFFFFFF) {
                out.push((x as i32, y as i32));
            }
        }
    }
    out
}

#[test]
fn trees_are_taken_out_of_the_static_layer() {
    let (scene, trees) = grove(TreeSpecies::Oak);
    let st = scene.tree_stats();
    assert!(st.trees > 10, "the grove should hold many swaying trees ({})", st.trees);
    assert!(trees.len() >= st.trees);
    // A saguaro does not sway: it stays a static prop.
    let (scene, _) = grove(TreeSpecies::Cactus);
    assert_eq!(scene.tree_stats().trees, 0, "cacti are rigid and stay in the static layer");
}

#[test]
fn calm_air_is_still_and_costs_nothing() {
    let (mut scene, _) = grove(TreeSpecies::Oak);
    scene.set_cloud_wind(250.0, 0.0);
    let first = frame(&mut scene);
    let _ = scene.take_pending();
    for _ in 0..300 {
        scene.tree_tick(DT);
    }
    assert_eq!(scene.pending_len(), 0, "calm air damaged nothing");
    let st = scene.tree_stats();
    assert_eq!(st.repaints_total, 0);
    assert_eq!(st.moving, 0);
    let later = frame(&mut scene);
    assert!(differing(&first, &later).is_empty(), "the picture did not change");
    // Light air (3 km/h) is nearly still: under a pixel of lean.
    scene.set_cloud_wind(250.0, 3.0);
    for _ in 0..600 {
        scene.tree_tick(DT);
        let _ = scene.take_pending();
    }
    assert!(scene.tree_stats().moving <= scene.tree_stats().trees / 4, "light air barely moves a crown");
}

#[test]
fn a_breeze_moves_the_crown_and_leaves_the_trunk_planted() {
    let (mut scene, trees) = grove(TreeSpecies::Oak);
    let calm = frame(&mut scene);
    settle(&mut scene, 40.0, 250.0);
    // Find an instant where plenty of trees are away from rest.
    let mut windy = calm.clone();
    for _ in 0..200 {
        scene.tree_tick(DT);
        let _ = scene.take_pending();
        if trees.iter().filter(|&&(x, y)| scene.tree_lean(x, y).unwrap_or(0).abs() >= 4).count() * 2 > trees.len() {
            break;
        }
    }
    scene.mark_full();
    scene.composite(&mut windy);
    let diff = differing(&calm, &windy);
    assert!(diff.len() > 200, "the crowns moved ({} px)", diff.len());
    // Nothing within the trunk's reach changed: every tree's lowest rows are
    // the same as at rest.
    let mut checked = 0;
    for &(tx, ty) in &trees {
        let Some(r) = scene.tree_rect(tx, ty) else { continue };
        // The tree's own foot is its bottom edge; the lowest 6 rows are trunk
        // and ground.
        let foot = Rect::new(r.x0 - 1, r.y1 - 6, r.x1 + 1, r.y1);
        // Count differences that cannot belong to a *neighbour's* crown: only
        // the central columns of the trunk.
        let cx = (r.x0 + r.x1) / 2;
        for y in foot.y0..foot.y1 {
            for x in (cx - 1)..=(cx + 1) {
                if x >= 0 && y >= 0 && (x as usize) < W && (y as usize) < H {
                    let i = y as usize * W + x as usize;
                    // A neighbour's crown in front can cover it either way; the
                    // trunk itself must not shift: compare against calm only
                    // where the calm pixel is trunk-brown (not leaf green).
                    let c = calm.px()[i];
                    let (cr, cg) = (render::palette::r(c) as i32, render::palette::g(c) as i32);
                    if cr > cg {
                        checked += 1;
                        assert_eq!(c & 0xFFFFFF, windy.px()[i] & 0xFFFFFF, "trunk moved at {x},{y}");
                    }
                }
            }
        }
    }
    assert!(checked > 10, "no trunk pixels were checked ({checked})");
}

#[test]
fn stronger_wind_leans_further() {
    let mean_abs = |kmh: f32| -> f64 {
        let (mut scene, trees) = grove(TreeSpecies::Oak);
        settle(&mut scene, kmh, 250.0);
        let (mut sum, mut n) = (0.0f64, 0usize);
        for k in 0..600 {
            scene.tree_tick(DT);
            let _ = scene.take_pending();
            if k % 15 == 0 {
                for &(x, y) in &trees {
                    if let Some(q) = scene.tree_lean(x, y) {
                        sum += (q as f64).abs() * 0.5;
                        n += 1;
                    }
                }
            }
        }
        sum / n as f64
    };
    let (light, breeze, gale) = (mean_abs(5.0), mean_abs(20.0), mean_abs(70.0));
    println!("mean tip lean: 5 km/h {light:.2} px, 20 km/h {breeze:.2} px, 70 km/h {gale:.2} px");
    assert!(light < 0.5, "light air is nearly imperceptible ({light})");
    assert!(breeze > 0.6 && breeze < 2.5, "a breeze leans a pixel or two ({breeze})");
    assert!(gale > breeze * 1.8, "a gale leans much further ({gale} vs {breeze})");
    assert!(gale < 8.0);
}

#[test]
fn the_lean_follows_the_wind_direction() {
    let mean = |dir: f32| -> f64 {
        let (mut scene, trees) = grove(TreeSpecies::Oak);
        settle(&mut scene, 30.0, dir);
        let (mut sum, mut n) = (0.0f64, 0usize);
        for k in 0..900 {
            scene.tree_tick(DT);
            let _ = scene.take_pending();
            if k % 10 == 0 {
                for &(x, y) in &trees {
                    sum += scene.tree_lean(x, y).unwrap_or(0) as f64;
                    n += 1;
                }
            }
        }
        sum / n as f64
    };
    // From the north-west (315) the wind blows towards the south-east; from the
    // opposite quarter the other way. The mean lean flips sign.
    let (a, b) = (mean(315.0), mean(135.0));
    println!("mean lean (half px): from 315 {a:.2}, from 135 {b:.2}");
    assert!(a * b < 0.0, "opposite winds lean opposite ways ({a} vs {b})");
    assert!(a.abs() > 0.4 && b.abs() > 0.4);
}

#[test]
fn neighbouring_trees_are_out_of_step() {
    let (mut scene, trees) = grove(TreeSpecies::Oak);
    settle(&mut scene, 25.0, 250.0);
    let (mut differ, mut pairs) = (0usize, 0usize);
    for k in 0..300 {
        scene.tree_tick(DT);
        let _ = scene.take_pending();
        if k % 10 != 0 {
            continue;
        }
        for &(x, y) in &trees {
            if let (Some(a), Some(b)) = (scene.tree_lean(x, y), scene.tree_lean(x + 3, y)) {
                pairs += 1;
                if a != b {
                    differ += 1;
                }
            }
        }
    }
    assert!(pairs > 100);
    assert!(
        differ * 10 > pairs * 4,
        "neighbours moved in lockstep: {differ} of {pairs} pairs differed"
    );
}

#[test]
fn repaints_are_bounded_and_incremental_equals_full() {
    let (mut scene, trees) = grove(TreeSpecies::Pine);
    let mut fb = frame(&mut scene);
    settle(&mut scene, 70.0, 250.0);
    scene.mark_full();
    scene.composite(&mut fb);
    let mut worst = 0usize;
    let mut check = Framebuffer::new(W, H);
    for k in 0..240 {
        scene.tree_tick(DT);
        let before = scene.tree_stats().repaints_last;
        worst = worst.max(before);
        match scene.take_pending() {
            None => scene.composite(&mut fb),
            Some(rects) => {
                for r in &rects {
                    scene.composite_rect(&mut fb, *r);
                }
            }
        }
        if k % 20 == 0 {
            scene.mark_full();
            scene.composite(&mut check);
            let bad = differing(&fb, &check);
            assert!(bad.is_empty(), "frame {k}: damage-limited composite differs from the full one in {} px", bad.len());
            let _ = scene.take_pending();
            fb.copy_from(&check);
        }
    }
    assert!(worst <= trees.len(), "a tree repaints at most once a frame ({worst} of {})", trees.len());
    let st = scene.tree_stats();
    println!("worst repaints in one frame {worst} of {} trees", st.trees);
}

#[test]
fn a_building_in_front_still_hides_a_swaying_tree() {
    // The tree's depth silhouette is in the static layer: something nearer
    // keeps its pixels whatever the crown does. Checked through the plane.
    let (scene, trees) = grove(TreeSpecies::Oak);
    let depth = scene.static_layer().depth().expect("depth plane");
    let (tx, ty) = trees
        .iter()
        .copied()
        .find(|&(x, y)| scene.tree_rect(x, y).is_some())
        .expect("a captured tree");
    let r = scene.tree_rect(tx, ty).expect("tree");
    let mid = ((r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2);
    let key = depth[mid.1 as usize * W + mid.0 as usize];
    assert!(key > 0, "the crown's silhouette stands in the depth plane ({key})");
}

#[test]
fn felling_a_tree_removes_it() {
    let (mut scene, trees) = grove(TreeSpecies::Oak);
    let (tx, ty) = trees
        .iter()
        .copied()
        .find(|&(x, y)| scene.tree_lean(x, y).is_some())
        .expect("a captured tree");
    let n0 = scene.tree_stats().trees;
    let t = scene.map.at(tx, ty);
    scene.set_tile(tx, ty, Tile { prop: Prop::None, ..t });
    scene.flush_dirty();
    assert!(scene.tree_lean(tx, ty).is_none(), "the felled tree left the layer");
    assert_eq!(scene.tree_stats().trees, n0 - 1);
}

#[test]
fn autumn_gusts_shake_leaves_loose_and_calm_air_does_not() {
    let (mut scene, _) = grove(TreeSpecies::Oak);
    let mut season = render::terrain::SeasonLook::default();
    season.autumn = 0.8;
    scene.set_season(season);
    scene.set_cloud_wind(250.0, 2.0);
    for _ in 0..600 {
        scene.tree_tick(DT);
        let _ = scene.take_pending();
    }
    // B76: in leaf-fall a few leaves drift down even in still air; a gust is
    // what shakes them loose in numbers.
    let calm = scene.tree_stats().leaves;
    assert!(calm <= 8, "still air sheds a few leaves, not a storm ({calm})");
    scene.set_cloud_wind(250.0, 35.0);
    let mut most = 0;
    for _ in 0..600 {
        scene.tree_tick(DT);
        let _ = scene.take_pending();
        most = most.max(scene.tree_stats().leaves);
    }
    assert!(
        most > calm.max(3) && most <= 120,
        "a gusty autumn day has leaves in the air ({most}, calm {calm})"
    );
}
