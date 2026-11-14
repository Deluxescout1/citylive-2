//! B100/D95: a tree is painted over a building that stands behind it, and
//! under a building that stands in front of it. Trees are tile props painted
//! with their tile's diagonal; a lot's pieces are painted at their own, so the
//! two interleave correctly.
//!
//! This file holds one test on purpose: it switches the draw-time tree cleaning
//! off (a big tree beside a wall is normally skipped), and that is process-wide.

use map::{Prop, TreeSpecies, generate, world_for_viewport};
use render::framebuffer::Framebuffer;
use render::scene::KitSprite;
use render::{CityObj, ObjKind, Scene, SceneConfig, SceneMode};

const W: usize = 1000;
const H: usize = 500;
const SKY: f32 = 0.30;
const HOUSE_RGB: u32 = 0x00c03030;

fn scene(seed: u64) -> Scene {
    let vp = world_for_viewport(W as i32, (H as f32 * (1.0 - SKY)) as i32);
    let map = generate(seed, vp.w, vp.h);
    let cfg = SceneConfig {
        width: W,
        height: H,
        seed,
        mode: SceneMode::Sim,
        cars: 0,
        peds: 0,
        buildings: 0,
        rain: false,
    };
    let mut s = Scene::from_map_with_sky(map, cfg, SKY);
    s.set_light([1.0, 1.0, 1.0], 0.0);
    s.set_sky(Framebuffer::new(W, s.sky_height()));
    s
}

/// A solid 1x1 house, 60 px tall, drawn inside its ground diamond and above.
fn house() -> KitSprite {
    let (ox, oy, w, h) = (32, 60, 64usize, 92usize);
    let mut px = vec![0u32; w * h];
    for j in 0..h {
        for i in 0..w {
            let (u, v) = (i as i32 - ox, j as i32 - oy);
            let edge = if u <= 0 { 16 + (u + 32) / 2 } else { 32 - u / 2 };
            if (-32..32).contains(&u) && v <= edge - 2 {
                px[j * w + i] = 0xFF00_0000 | HOUSE_RGB;
            }
        }
    }
    KitSprite {
        w,
        h,
        ox,
        oy,
        px: std::sync::Arc::new(px),
        top: 60,
        foot: [0.0, 0.0, 1.0, 1.0],
        roof: None,
        panes: Vec::new(),
        surf: Vec::new(),
    }
}

fn lot(x: i32, y: i32, level: u8) -> CityObj {
    CityObj {
        x,
        y,
        w: 1,
        h: 1,
        level,
        kind: ObjKind::House,
        storeys: 2,
        style: 0,
        stage: render::city::Stage::Finished,
        seed: 7,
        front: 0,
        t: 0.0,
        kit: 1,
        wear: Default::default(),
    }
}

fn centred_tile(s: &Scene) -> (i32, i32) {
    let cam = s.camera();
    let mut best = ((0i32, 0i32), f32::MAX);
    for y in 0..s.map.h {
        for x in 0..s.map.w {
            let (px, py) = cam.project(x as f32, y as f32, 0);
            let d = (px - W as f32 * 0.5).hypot(py - H as f32 * 0.55);
            if d < best.1 {
                best = ((x, y), d);
            }
        }
    }
    best.0
}

fn rgb_at(s: &Scene, x: i32, y: i32) -> u32 {
    let l = s.static_layer();
    l.px()[y as usize * l.w + x as usize] & 0x00FF_FFFF
}

#[test]
fn trees_and_buildings_are_painted_in_order() {
    // SAFETY: the only test in this binary, so nothing else reads the environment.
    unsafe { std::env::set_var("CITYLIVE_NO_DRAWN_TREES", "1") };
    let tree = Prop::Tree { species: TreeSpecies::Birch, size: 0, variant: 1 };
    let mut s = scene(5);
    // Painted into the static layer (a swaying tree is a separate sprite).
    s.set_tree_sway(false);
    let (cx, cy) = centred_tile(&s);
    let level = s.map.at(cx, cy).corner[0];
    s.set_city_with_sprites(vec![house()], vec![lot(cx, cy, level)]);
    let cam = *s.camera();
    let (ax, ay) = cam.project(cx as f32, cy as f32, level as i32 * render::iso::HEIGHT_UNIT);
    let (ax, ay) = (ax as i32, ay as i32);

    // Where the house is, with no trees near it: the baseline.
    let region = |s: &Scene| -> Vec<u32> {
        let mut v = Vec::new();
        for dy in 0..30 {
            for dx in -12..=12 {
                v.push(rgb_at(s, ax + dx, ay + dy));
            }
        }
        v
    };
    let before_front = region(&s);
    let before_back = rgb_at(&s, ax, ay - 28);
    assert_eq!(before_back, HOUSE_RGB, "the house is where the test expects it");

    // A tree on the tile in front of the house (nearer): it covers the corner.
    let mut t = s.map.at(cx + 1, cy + 1);
    t.prop = tree;
    s.set_tile(cx + 1, cy + 1, t);
    s.rebuild_static();
    let in_front = region(&s);
    let changed = in_front.iter().zip(&before_front).filter(|(a, b)| a != b).count();
    assert!(changed > 20, "a tree in front must cover the house corner ({changed} px changed)");

    // Remove it; put one on the tile behind (farther): the house covers it.
    let mut t = s.map.at(cx + 1, cy + 1);
    t.prop = Prop::None;
    s.set_tile(cx + 1, cy + 1, t);
    let mut t = s.map.at(cx - 1, cy - 1);
    t.prop = tree;
    s.set_tile(cx - 1, cy - 1, t);
    s.rebuild_static();
    let behind = rgb_at(&s, ax, ay - 28);
    assert_eq!(behind, HOUSE_RGB, "a tree behind the house must be covered by it");
}
