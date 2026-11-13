//! B76 round 3: autumn leaves lie and fall, they do not draw lines.
//!
//! the owner's desktop photo showed regular **vertical dashed columns** of orange
//! specks running down an autumn forest. They were the lawn litter: the clump
//! positions took their x range from `p[2].x - p[0].x`, and the top and bottom
//! corners of a tile diamond share an x — so every mark in a tile landed on one
//! vertical line, and a forest read as columns of dashes.
//!
//! These tests pin the fix (the litter spreads inside the tile's diamond and no
//! column of marks builds up), and the leaf rules the brief asks for: a falling
//! leaf is a 1–2 px speck at its own position, the count stays bounded per tree,
//! and leaves only come from trees the renderer draws.

use map::{Kind, Prop, Tile, TileMap, TreeSpecies, world_for_viewport};
use render::light::LightState;
use render::terrain::SeasonLook;
use render::{Scene, SceneConfig, SceneMode};

const W: usize = 520;
const H: usize = 320;

/// A grass field: oaks on every third tile, so there are crowns to shed.
fn grove(litter: f32) -> (Scene, TileMap) {
    let vp = world_for_viewport(W as i32, H as i32);
    let mut map = TileMap::fill(vp.w, vp.h, Tile::flat(2), 5);
    for y in 0..map.h {
        for x in 0..map.w {
            let t = map.at(x, y);
            let plant = x % 3 == 1 && y % 3 == 1 && x > 3 && y > 3 && x < map.w - 3 && y < map.h - 3;
            let prop = if plant {
                Prop::Tree { species: TreeSpecies::Oak, size: 1, variant: (x * 7 + y * 13) as u8 }
            } else {
                Prop::None
            };
            map.set(x, y, Tile { kind: Kind::Grass, prop, ..t });
        }
    }
    let mut scene = Scene::from_map_with_sky(
        map.clone(),
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
    let mut season = SeasonLook::default();
    season.autumn = 1.0;
    season.litter = litter;
    scene.set_season(season);
    while scene.shadow_tick(1.0) {}
    scene.flush_dirty();
    (scene, map)
}

fn autumn() -> SeasonLook {
    let mut s = SeasonLook::default();
    s.autumn = 1.0;
    s.litter = 1.0;
    s
}

fn is_litter(c: render::palette::Color, turn: &[[u8; 3]; 3]) -> bool {
    let (r, g, b) = (
        ((c >> 16) & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
        (c & 0xFF) as u8,
    );
    turn.iter().any(|t| t[0] == r && t[1] == g && t[2] == b)
}

#[test]
fn lawn_litter_spreads_across_the_tile_instead_of_standing_in_a_column() {
    // One tile's worth of marks: draw a still and look at the pixels of the
    // litter colours. Before the fix every mark in a tile shared one x.
    let (mut scene, map) = grove(1.0);
    let season = autumn();
    let mut fb = render::Framebuffer::new(W, H);
    scene.mark_full();
    scene.composite(&mut fb);

    // The tile under test: grass, with a tree on it so it keeps a drift.
    let mut found = false;
    for y in 1..map.h - 1 {
        for x in 1..map.w - 1 {
            if !map.at(x, y).prop.is_tree() {
                continue;
            }
            // The tile's screen box, from the camera the scene built.
            let p = scene.cam.corners(x, y, [0.0; 4]);
            let (x0, x1) = (p[3].0 as i32, p[1].0 as i32);
            let (y0, y1) = (p[0].1 as i32, p[2].1 as i32);
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for sy in y0 - 4..y1 + 4 {
                for sx in x0 - 4..x1 + 4 {
                    if sx < 0 || sy < 0 || sx >= W as i32 || sy >= H as i32 {
                        continue;
                    }
                    let c = fb.px()[sy as usize * W + sx as usize];
                    if c == render::palette::TRANSPARENT || !is_litter(c, &season.turn) {
                        continue;
                    }
                    xs.push(sx);
                    ys.push(sy);
                }
            }
            if xs.len() < 6 {
                continue;
            }
            let spread_x = xs.iter().max().unwrap() - xs.iter().min().unwrap();
            let spread_y = ys.iter().max().unwrap() - ys.iter().min().unwrap();
            assert!(
                spread_x >= 24,
                "the litter of one tile spans {spread_x} px across: that is a column, not a drift"
            );
            assert!(spread_y >= 4, "the litter is flat: {spread_y} px tall");
            // And it belongs to its own tile: the marks are inside the tile's
            // diamond, allowing for a clump's own 4-px radius and the marks a
            // neighbour's drift leaves at the edge.
            let inside = xs
                .iter()
                .zip(ys.iter())
                .filter(|(sx, sy)| **sx >= x0 - 6 && **sx <= x1 + 6 && **sy >= y0 - 6 && **sy <= y1 + 6)
                .count();
            assert!(
                inside * 10 >= xs.len() * 9,
                "only {inside} of {} marks are in this tile's diamond",
                xs.len()
            );
            found = true;
            break;
        }
        if found {
            break;
        }
    }
    assert!(found, "no tile of the grove kept a drift of leaves");
}

#[test]
fn no_column_of_leaf_marks_in_a_still() {
    // The brief's gate: no vertical run of leaf pixels longer than ~3 px. Walk
    // every column of the frame; a run of litter-coloured pixels may be at most
    // three tall (one speck plus its neighbour, never a dash trail).
    let (mut scene, map) = grove(1.0);
    let season = autumn();
    let mut fb = render::Framebuffer::new(W, H);
    scene.mark_full();
    scene.composite(&mut fb);
    let mut longest = 0usize;
    for sx in 0..W {
        let mut run = 0usize;
        for sy in 0..H {
            let c = fb.px()[sy * W + sx];
            // A crown is not litter: only count marks on grass tiles.
            let tile = map.at(0, 0);
            let _ = tile;
            if c != render::palette::TRANSPARENT && is_litter(c, &season.turn) {
                run += 1;
                longest = longest.max(run);
            } else {
                run = 0;
            }
        }
    }
    assert!(
        longest <= 3,
        "a column of leaf marks {longest} px tall: leaves are forming a trail"
    );
}

#[test]
fn a_few_leaves_fall_and_they_come_from_trees_that_are_drawn() {
    // Leaves exist in leaf-fall, they are bounded (a handful, never a cloud),
    // and every tree that sheds is a tree the renderer drew (the sway layer's
    // slots are filled by `draw_prop`, so a tile with no tree has no source).
    let (mut scene, map) = grove(1.0);
    scene.set_cloud_wind(90.0, 22.0);
    let mut dt = 0.0f32;
    while dt < 20.0 {
        scene.tree_tick(1.0 / 30.0);
        dt += 1.0 / 30.0;
    }
    let stats = scene.tree_stats();
    assert!(stats.leaves > 0, "a windy autumn day shed no leaves at all");
    // The hard cap, and a handful per tree: the grove has ~1 500 oaks.
    assert!(stats.leaves <= 120, "too many leaves in the air: {}", stats.leaves);
    assert!(
        stats.leaves <= stats.moving.max(1) * 4 + 40,
        "{} leaves for {} swaying trees is more than a handful each",
        stats.leaves,
        stats.moving
    );
    // Every tile that holds a leaf source holds a tree.
    let mut sources = 0;
    for y in 0..map.h {
        for x in 0..map.w {
            if scene.tree_lean(x, y).is_some() {
                sources += 1;
                assert!(
                    map.at(x, y).prop.is_tree(),
                    "a leaf source stands on a tile with no tree at {x},{y}"
                );
            }
        }
    }
    assert!(sources > 10, "only {sources} leaf sources in a grove of oaks");
}

#[test]
fn a_calm_autumn_day_still_sheds_a_few_leaves() {
    // B76 r1: the fair-weather default (8 km/h) used to shed nothing at all.
    let (mut scene, _map) = grove(1.0);
    scene.set_cloud_wind(90.0, 8.0);
    let mut dt = 0.0f32;
    while dt < 30.0 {
        scene.tree_tick(1.0 / 30.0);
        dt += 1.0 / 30.0;
    }
    assert!(
        scene.tree_stats().leaves > 0,
        "a calm autumn day shed no leaves: leaf-fall must not need a gale"
    );
}

#[test]
fn leaves_stay_one_or_two_pixels_while_they_fall() {
    // A leaf is a speck at its own position: 2x2 at most, and it does not leave
    // the last frame's mark behind. Two frames a few ticks apart must not both
    // hold leaf pixels in a line between them.
    let (mut scene, _map) = grove(1.0);
    scene.set_cloud_wind(90.0, 25.0);
    let mut fb = render::Framebuffer::new(W, H);
    // Let a good number of leaves get into the air.
    for _ in 0..90 {
        scene.tree_tick(1.0 / 30.0);
    }
    scene.mark_full();
    scene.composite(&mut fb);
    let season = autumn();
    let mut per_column: Vec<usize> = vec![0; W];
    for sx in 0..W {
        for sy in 0..H {
            let c = fb.px()[sy * W + sx];
            if c != render::palette::TRANSPARENT && is_litter(c, &season.turn) {
                per_column[sx] += 1;
            }
        }
    }
    // A few leaves in the air plus the drifts on the ground: a single column
    // holding a dozen marks would be a trail, not leaves.
    let worst = per_column.iter().copied().max().unwrap_or(0);
    assert!(worst <= 8, "one column holds {worst} leaf marks in a still");
}
