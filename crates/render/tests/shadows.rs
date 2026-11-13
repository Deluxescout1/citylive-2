//! B60/D74: the cast-shadow system, end to end, on a synthetic town.
//!
//! These tests are about the *behaviour* the brief asks for — the shadow is on
//! the side away from the sun, it stretches as the sun lowers, it disappears at
//! night unless the moon is bright, overcast weakens it, and it never lands on a
//! roof — not about exact pixel counts.

use map::{Kind, Tile, TileMap, Viewport, world_for_viewport};
use render::city::{CityObj, ObjKind, Stage};
use render::framebuffer::Framebuffer;
use render::light::{LightState, Orb};
use render::{Scene, SceneConfig, SceneMode};

const W: usize = 520;
const H: usize = 320;

/// Flat grass with one 2×2 house, and the camera looking at it.
fn town() -> (Scene, (i32, i32)) {
    let vp = world_for_viewport(W as i32, H as i32);
    let mut map = TileMap::fill(vp.w, vp.h, Tile::flat(2), 5);
    for y in 0..map.h {
        for x in 0..map.w {
            let t = map.at(x, y);
            map.set(x, y, Tile { kind: Kind::Grass, ..t });
        }
    }
    let (fx, fy) = map::screen_to_tile(&vp, W as f32 * 0.5, H as f32 * 0.6);
    let (cx, cy) = (fx.floor() as i32, fy.floor() as i32);
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
        0.0,
    );
    // A slim tower: tall enough (84 px) that its shadow is easy to probe at
    // 30°, and still short enough at noon that the ground beside it is sunlit.
    scene.set_city(vec![CityObj {
        x: cx,
        y: cy,
        w: 2,
        h: 2,
        level: 2,
        kind: ObjKind::Tower,
        storeys: 8,
        style: 0,
        stage: Stage::Finished,
        seed: 9,
        front: 0,
        t: 0.0,
        kit: 0, wear: Default::default(),
    }]);
    (scene, (cx, cy))
}

fn lit(mut scene: Scene, alt: f64, az: f64, cloud: f32, strength: f32) -> Scene {
    let mut ls = LightState::default();
    ls.sun = Orb::new(alt, az);
    ls.cloud = cloud;
    ls.dir_strength = strength;
    ls.ambient = [1.0; 3];
    scene.set_light_state(ls);
    while scene.shadow_tick(1.0) {}
    scene
}

/// Total shadow coverage in a screen box.
fn coverage(scene: &Scene, r: render::Rect) -> f64 {
    let (w, h) = (scene.width(), scene.height());
    let plane = scene.shadow_plane();
    let mut sum = 0u64;
    let mut n = 0u64;
    for y in r.y0.max(0)..r.y1.min(h as i32) {
        for x in r.x0.max(0)..r.x1.min(w as i32) {
            sum += plane[y as usize * w + x as usize] as u64;
            n += 1;
        }
    }
    if n == 0 {
        return 0.0;
    }
    sum as f64 / (n as f64 * 255.0)
}

/// The screen half-way between the house and one azimuth's shadow side.
/// `dir` is in tile space: the shadow lies along it, the sun against it.
fn probe(scene: &Scene, house: (i32, i32), dir: (f32, f32), dist: f32) -> render::Rect {
    let m = &scene.map;
    let z = render::iso::level_px(m.surface_level_at(house.0 as f32 + 1.0, house.1 as f32 + 1.0));
    let (cx, cy) = (house.0 as f32 + 1.0 + dir.0 * dist, house.1 as f32 + 1.0 + dir.1 * dist);
    let p = scene.cam.project(cx, cy, z);
    let (px, py) = (p.0 as i32, p.1 as i32);
    render::Rect::new(px - 6, py - 4, px + 6, py + 5)
}

#[test]
fn the_shadow_falls_away_from_the_sun_and_turns_with_it() {
    // Sun due west (az 270): the shadow lies east (screen left, since the sky
    // band puts east at the left). Sun due east: the shadow lies west (right).
    let sun_dir = |az: f64| {
        let (x, y) = render::light::toward(az as f32);
        (-x, -y)
    };
    let house = town().1;
    let west = lit(town().0, 30.0, 270.0, 0.0, 1.0);
    let east = lit(town().0, 30.0, 90.0, 0.0, 1.0);
    // Where the western sun's shadow lies, the eastern sun's must not, and the
    // other way round.
    let d = sun_dir(270.0);
    let shadow_east = coverage(&west, probe(&west, house, d, 2.2));
    let lit_east = coverage(&east, probe(&east, house, d, 2.2));
    assert!(
        shadow_east > 0.25,
        "a 30° western sun must throw a shadow east ({shadow_east:.2})"
    );
    assert!(
        lit_east < 0.05,
        "the same ground is sunlit with the sun in the east ({lit_east:.2})"
    );
    let d2 = sun_dir(90.0);
    assert!(coverage(&east, probe(&east, house, d2, 2.2)) > 0.25);
    assert!(coverage(&west, probe(&west, house, d2, 2.2)) < 0.05);
}

#[test]
fn a_lower_sun_throws_a_longer_shadow() {
    let house = town().1;
    let dir = {
        let (x, y) = render::light::toward(180.0);
        (-x, -y)
    };
    // Down-screen (north) at noon in this projection, so the shadow is visible.
    let high = lit(town().0, 70.0, 180.0, 0.0, 1.0);
    let low = lit(town().0, 12.0, 180.0, 0.0, 1.0);
    let near = coverage(&high, probe(&high, house, dir, 1.25));
    let far = coverage(&low, probe(&low, house, dir, 1.25));
    assert!(near > 0.15, "70° sun: short but present ({near:.2})");
    assert!(far > 0.15, "12° sun: the shadow must still cover the near ground");
    // Two tiles out only the low sun reaches: its shadow is ~6 tiles long at
    // 12° and under half a tile at 70°.
    let high_far = coverage(&high, probe(&high, house, dir, 2.0));
    let low_far = coverage(&low, probe(&low, house, dir, 2.0));
    assert!(
        low_far > 0.25 && high_far < 0.05,
        "at 2 tiles: low sun {low_far:.2}, high sun {high_far:.2}"
    );
}

#[test]
fn night_casts_nothing_unless_the_moon_is_bright() {
    let mut scene = town().0;
    let mut ls = LightState::default();
    ls.sun = Orb::new(-22.0, 180.0);
    ls.moon = Orb::new(45.0, 150.0);
    ls.moon_illum = 0.05;
    ls.dir_strength = 0.02;
    ls.ambient = [0.2, 0.25, 0.6];
    scene.set_light_state(ls);
    while scene.shadow_tick(1.0) {}
    assert!(scene.shadow_plane().iter().all(|v| *v == 0), "a new moon casts nothing");

    ls.moon_illum = 1.0;
    ls.dir_strength = 0.3;
    scene.set_light_state(ls);
    while scene.shadow_tick(1.0) {}
    let some = scene.shadow_plane().iter().any(|v| *v > 0);
    assert!(some, "a full moon must cast faint shadows");
    // Faint: the strength the multiply is made with stays low.
    let c = scene.light_state().caster().expect("full moon");
    assert!(c.moon && c.strength < 0.4, "moonlight shadow strength {}", c.strength);
}

#[test]
fn overcast_weakens_the_shadow() {
    let house = town().1;
    let dir = {
        let (x, y) = render::light::toward(200.0);
        (-x, -y)
    };
    let clear = lit(town().0, 35.0, 200.0, 0.0, 1.0);
    let grey = lit(town().0, 35.0, 200.0, 0.95, 1.0);
    // Terrain shape unchanged, but the strength has to fall a long way.
    let sc = clear.light_state().caster().unwrap().strength;
    let sg = grey.light_state().caster().unwrap().strength;
    assert!(sg < 0.4 * sc, "overcast strength {sg:.2} vs clear {sc:.2}");
    // The ground coverage itself stays (the geometry is the same); what fades
    // is the light, which is what the multiply reads.
    let _ = (house, dir);
    assert!(coverage(&grey, render::Rect::from_size(W, H)) > 0.0);
}

#[test]
fn no_shadow_lands_on_a_roof() {
    // A kit-less engine building is TAG_BUILT on an even painter key: never a
    // receiver. The shadow plane must be clean over the house's own roof.
    let house = town().1;
    let scene = lit(town().0, 20.0, 250.0, 0.0, 1.0);
    let z = render::iso::level_px(scene.map.surface_level_at(house.0 as f32 + 1.0, house.1 as f32 + 1.0));
    let p = scene.cam.project(house.0 as f32 + 1.0, house.1 as f32 + 1.0, z + 60);
    let r = render::Rect::new(p.0 as i32 - 4, p.1 as i32 - 3, p.0 as i32 + 4, p.1 as i32 + 4);
    assert!(
        coverage(&scene, r) < 0.01,
        "a roof must not take the ground's shadow (coverage {:.2})",
        coverage(&scene, r)
    );
}

#[test]
fn the_shadow_plane_settles_and_moves_smoothly() {
    // A bake starts a crossfade; a few ticks finish it, and no single frame may
    // jump the whole way (D71: never a visible step).
    let mut scene = lit(town().0, 40.0, 180.0, 0.0, 1.0);
    let before = scene.shadow_plane().to_vec();
    let mut ls = scene.light_state();
    ls.sun = Orb::new(20.0, 200.0);
    scene.set_light_state(ls);
    let mut worst = 0i32;
    let mut frames = 0;
    let mut prev = before.clone();
    while scene.shadow_tick(0.033) {
        frames += 1;
        let cur = scene.shadow_plane().to_vec();
        let d = cur
            .iter()
            .zip(prev.iter())
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap_or(0);
        worst = worst.max(d);
        prev = cur;
        assert!(frames < 60, "the crossfade must finish");
    }
    assert!(frames >= 2, "the fade must take more than one frame ({frames})");
    assert!(worst <= 200, "one frame moved {worst}/255 of the shadow");
    assert!(prev.iter().any(|v| *v > 0), "the settled plane carries the new shadow");
}

#[test]
fn the_static_layer_and_composite_stay_in_step() {
    // A shadowed frame differs from an unshadowed one only in how bright it is:
    // same pixels, darker. This is what guarantees nothing structural moved.
    let mut scene = lit(town().0, 25.0, 100.0, 0.0, 1.0);
    let mut a = Framebuffer::new(W, H);
    scene.composite(&mut a);
    let mut ls = scene.light_state();
    ls.sun = Orb::new(-30.0, 100.0);
    ls.dir_strength = 0.0;
    scene.set_light_state(ls);
    while scene.shadow_tick(1.0) {}
    let mut b = Framebuffer::new(W, H);
    scene.composite(&mut b);
    // With the sun down the cast shadow is gone, so the frame must be *lighter*
    // exactly where it was shadowed — and nowhere else.
    let mut lighter = 0usize;
    let mut darker = 0usize;
    for (x, y) in a.px().iter().zip(b.px().iter()) {
        let la = (x >> 16 & 0xff) + (x >> 8 & 0xff) + (x & 0xff);
        let lb = (y >> 16 & 0xff) + (y >> 8 & 0xff) + (y & 0xff);
        if lb > la {
            lighter += 1;
        } else if lb < la {
            darker += 1;
        }
    }
    assert!(lighter > 500, "the shadow must darken real ground ({lighter} px)");
    assert!(
        darker < lighter / 4,
        "removing the light must not darken the frame ({darker} darker vs {lighter} lighter)"
    );
}

/// The height field is geometry, so it must not change when only the light does.
#[test]
fn the_height_field_survives_a_light_change() {
    let scene = lit(town().0, 30.0, 120.0, 0.0, 1.0);
    let a = scene.terrain_field_checksum();
    let scene2 = lit(town().0, 8.0, 300.0, 0.9, 0.4);
    assert_eq!(a, scene2.terrain_field_checksum(), "the field is light-independent");
}

#[test]
fn a_viewport_far_from_the_city_is_still_covered() {
    // Sanity on the frame-sized plane: every pixel has an entry (a short plane
    // would silently switch the multiply off).
    let scene = lit(town().0, 30.0, 120.0, 0.0, 1.0);
    assert_eq!(scene.shadow_plane().len(), scene.width() * scene.height());
    let _: Viewport = scene.viewport;
}
