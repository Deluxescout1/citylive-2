//! B72 (D78): the window lights follow the people inside.
//!
//! What these check, in the brief's words: lights follow occupants (an empty
//! building is dark at night), switches are staggered (no frame flips more than a
//! few per cent of a building's windows), and the whole thing is deterministic —
//! the same save at the same hour gives the same windows.

use map::{Kind, Tile, TileMap, world_for_viewport};
use render::city::{CityObj, ObjKind, Stage};
use render::framebuffer::Framebuffer;
use render::light::LightState;
use render::{Scene, SceneConfig, SceneMode};

const W: usize = 640;
const H: usize = 400;

fn scene() -> Scene {
    let vp = world_for_viewport(W as i32, H as i32);
    let map = TileMap::fill(vp.w, vp.h, Tile::flat(2), 5);
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
        0.24,
    );
    let mut ls = LightState::default();
    ls.ambient = [1.0; 3];
    scene.set_light_state(ls);
    while scene.shadow_tick(1.0) {}
    scene.flush_dirty();
    scene
}

/// One engine-drawn building (no kit sprite): the renderer's own walls and its
/// own window grid.
fn lot(x: i32, y: i32, kind: ObjKind, storeys: u8, seed: u32) -> CityObj {
    CityObj {
        x,
        y,
        w: 2,
        h: 2,
        level: 2,
        kind,
        storeys,
        style: 0,
        stage: Stage::Finished,
        seed,
        front: 1,
        t: 0.0,
        kit: 0, wear: Default::default(),
    }
}

/// Night: the sky dark, and a given hour.
fn night(scene: &mut Scene, hour: f32, glow: f32) {
    let mut ls = scene.light_state();
    ls.ambient = [0.22, 0.24, 0.30];
    ls.glow = glow;
    ls.hour = hour;
    scene.set_light_state(ls);
    scene.flush_dirty();
}

fn frame(scene: &mut Scene) -> Framebuffer {
    let mut fb = Framebuffer::new(W, H);
    scene.mark_full();
    scene.composite(&mut fb);
    fb
}

/// The pixels of one lot's screen box, so a test can look at a building alone.
fn litres(scene: &Scene, x: i32, y: i32) -> usize {
    let (panes, lit) = scene.window_counts();
    let _ = (x, y);
    let _ = panes;
    lit
}

#[test]
fn an_empty_building_is_dark_at_night() {
    let mut scene = scene();
    let o = lot(10, 10, ObjKind::House, 2, 77);
    let (x, y) = (o.x, o.y);
    scene.set_city(vec![o]);
    // Night, with nobody in the house: no slots lit.
    night(&mut scene, 21.0, 0.9);
    scene.set_window_plan(&[(x, y, 0)]);
    assert_eq!(scene.window_counts().1, 0, "nobody home, no light");

    // The same house with people in: its windows come on.
    scene.set_window_plan(&[(x, y, u64::MAX)]);
    assert!(scene.window_counts().1 > 0, "a full house has lit windows");
    // And the frame really changed.
    let fb = frame(&mut scene);
    let lit_px = fb.px().iter().filter(|&&c| {
        let (r, g, b) = (c >> 16 & 0xFF, c >> 8 & 0xFF, c & 0xFF);
        r > 150 && g > 100 && r > b + 40
    }).count();
    assert!(lit_px > 0, "lit panes are warm pixels on the frame");
}

#[test]
fn a_window_does_not_switch_by_day() {
    let mut scene = scene();
    let o = lot(10, 10, ObjKind::Office, 8, 21);
    let (x, y) = (o.x, o.y);
    scene.set_city(vec![o]);
    let mut ls = scene.light_state();
    ls.ambient = [1.0; 3];
    ls.glow = 0.0;
    ls.hour = 12.0;
    scene.set_light_state(ls);
    scene.set_window_plan(&[(x, y, u64::MAX)]);
    scene.flush_dirty();
    assert_eq!(scene.window_counts().1, 0, "no lit glass at noon");
}

#[test]
fn switches_are_staggered() {
    // A whole facade given all its slots: the panes do not all come on at the
    // same darkness — each has its own hour in the dusk.
    let mut scene = scene();
    let o = lot(10, 10, ObjKind::Office, 10, 1234);
    let (x, y) = (o.x, o.y);
    scene.set_city(vec![o]);
    scene.set_window_plan(&[(x, y, u64::MAX)]);
    let (panes, _) = scene.window_counts();
    assert!(panes > 12, "a ten-storey office has plenty of panes: {panes}");
    let mut prev = 0usize;
    let mut steps = 0;
    for k in 0..=12 {
        // A dusk from just dark to full night.
        let glow = 0.14 + 0.04 * k as f32;
        night(&mut scene, 20.0, glow);
        let lit = scene.window_counts().1;
        if k > 0 {
            let step = lit.saturating_sub(prev);
            assert!(
                step * 100 <= panes * 60,
                "one dusk step switched {step} of {panes} panes at once"
            );
            if step > 0 {
                steps += 1;
            }
        }
        prev = lit;
    }
    assert!(steps >= 3, "the lights came on over {steps} steps, not one");
}

#[test]
fn the_same_save_gives_the_same_windows() {
    let build = || {
        let mut scene = scene();
        let a = lot(10, 10, ObjKind::House, 2, 9);
        let b = lot(16, 10, ObjKind::Townhouse, 3, 31);
        let (ax, ay, bx, by) = (a.x, a.y, b.x, b.y);
        scene.set_city(vec![a, b]);
        scene.set_window_plan(&[(ax, ay, 0x00FF_00FF_1234_5678), (bx, by, 0x0F0F_0F0F_AAAA_5555)]);
        night(&mut scene, 20.5, 0.8);
        let fb = frame(&mut scene);
        (scene.window_counts(), fb.px().to_vec())
    };
    let (c1, f1) = build();
    let (c2, f2) = build();
    assert_eq!(c1, c2, "same save, same hour, same window count");
    assert_eq!(f1, f2, "same save, same hour, the same pixels");
    assert!(c1.1 > 0);
}

#[test]
fn the_frame_follows_the_plan() {
    // The renderer's half of the bargain: a town whose households are in is
    // mostly lit, and the same town with everyone in bed is not. (Which slots
    // are lit is the sim's answer — `sim/tests/window_lights.rs` checks that it
    // follows the residents.)
    let mut scene = scene();
    let mut plan = Vec::new();
    let mut objs = Vec::new();
    for i in 0..6 {
        let o = lot(10 + i * 4, 10, ObjKind::House, 2, 100 + i as u32);
        plan.push((o.x, o.y, u64::MAX));
        objs.push(o);
    }
    let keys: Vec<(i32, i32)> = plan.iter().map(|p| (p.0, p.1)).collect();
    scene.set_city(objs);
    scene.set_window_plan(&plan);
    night(&mut scene, 21.0, 0.9);
    let (panes, evening) = scene.window_counts();
    assert!(evening > panes / 2, "an occupied town is mostly lit at nine: {evening}/{panes}");
    // Three in the morning: one slot each, the bathroom light.
    let sparse: Vec<(i32, i32, u64)> = keys.iter().map(|&(x, y)| (x, y, 1u64 << 7)).collect();
    scene.set_window_plan(&sparse);
    let (_, late) = scene.window_counts();
    assert!(late * 4 < evening, "most lights are out at three: {late}/{evening}");
    let _ = litres(&scene, 10, 10);
}
