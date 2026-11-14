//! B100/D95: the nearer building always covers the farther one, whatever the
//! lots' shapes. One painter key per lot (the far corner's) is wrong for a long
//! lot beside a short one; a lot is painted in screen-column slices, each at
//! the diagonal of the nearest ground it stands on.

use map::{generate, world_for_viewport};
use render::framebuffer::Framebuffer;
use render::scene::KitSprite;
use render::{CityObj, ObjKind, Scene, SceneConfig, SceneMode};

const W: usize = 1400;
const H: usize = 600;
const SKY: f32 = 0.30;

fn scene(seed: u64) -> Scene {
    let vp = world_for_viewport(W as i32, (H as f32 * (1.0 - SKY)) as i32);
    let mut map = generate(seed, vp.w, vp.h);
    // Flat grass at sea level: the fake sprites below are drawn at level 0, and a
    // hill or a lake under them would cover them for reasons that are not the
    // painter's order.
    for y in 0..map.h {
        for x in 0..map.w {
            map.set(x, y, map::Tile::flat(0));
        }
    }
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

/// A solid sprite of an `lw` x `lh` lot, `height` px tall, drawn only inside
/// the lot's ground diamond and above it (nothing below the front edges, so a
/// tile in front of it never overlaps it).
fn fake_sprite(lw: i32, lh: i32, height: i32, rgb: u32) -> KitSprite {
    let (ox, oy) = (lh * 32, height);
    let w = ((lw + lh) * 32) as usize;
    let h = (height + (lw + lh) * 16) as usize;
    let (ul, ub, ur) = (-lh * 32, (lw - lh) * 32, lw * 32);
    let (vl, vb) = (lh * 16, (lw + lh) * 16);
    let mut px = vec![0u32; w * h];
    for j in 0..h {
        for i in 0..w {
            let (u, v) = (i as i32 - ox, j as i32 - oy);
            if u < ul || u >= ur {
                continue;
            }
            let edge = if u <= ub { vl + (u - ul) / 2 } else { vb - (u - ub) / 2 };
            if v <= edge - 2 {
                px[j * w + i] = 0xFF00_0000 | rgb;
            }
        }
    }
    KitSprite {
        w,
        h,
        ox,
        oy,
        px: std::sync::Arc::new(px),
        top: height as u16,
        foot: [0.0, 0.0, lw as f32, lh as f32],
        roof: None,
        panes: Vec::new(),
        surf: Vec::new(),
    }
}

fn lot(x: i32, y: i32, w: i32, h: i32, kind: ObjKind, kit: u16) -> CityObj {
    CityObj {
        x,
        y,
        w,
        h,
        level: 0,
        kind,
        storeys: 3,
        style: 0,
        stage: render::city::Stage::Finished,
        seed: 7,
        front: 0,
        t: 0.0,
        kit,
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

const A_RGB: u32 = 0x00c03030;
const B_RGB: u32 = 0x003050c0;

fn colour_at(s: &Scene, x: i32, y: i32) -> u32 {
    let l = s.static_layer();
    l.px()[y as usize * l.w + x as usize] & 0x00FF_FFFF
}

/// A tall 1x1 house stands in front (+x) of a long 1x8 lot whose far corner is
/// *nearer* on the old single key. Where both draw, the house must win.
#[test]
fn a_short_lot_in_front_of_a_long_one_covers_it() {
    let mut s = scene(5);
    let (cx, cy) = centred_tile(&s);
    let (ax, ay) = (cx, cy - 4);
    let long = lot(ax, ay, 1, 8, ObjKind::House, 1);
    let short = lot(ax + 1, ay, 1, 1, ObjKind::House, 2);
    s.set_city_with_sprites(vec![fake_sprite(1, 8, 60, A_RGB), fake_sprite(1, 1, 60, B_RGB)], vec![long, short]);
    let cam = s.camera();
    // A point above the short house's anchor, left of centre, where the long
    // lot's sprite also reaches.
    let (bx, by) = cam.project((ax + 1) as f32, ay as f32, 0);
    let (px, py) = (bx as i32 - 16, by as i32 - 20);
    assert_eq!(colour_at(&s, px, py), B_RGB, "the house in front must cover the long lot at ({px},{py})");
}

/// A person walking beside the middle of a long lot is nearer than that part of
/// the lot's wall. One key for the whole lot (its far corner's) hid the person
/// behind it; each column of the lot has its own key now.
#[test]
fn a_person_beside_a_long_lot_is_not_hidden_by_its_wall() {
    use render::actors::actor_key;
    let mut s = scene(5);
    let (cx, cy) = centred_tile(&s);
    let (ax, ay) = (cx, cy - 4);
    s.set_city_with_sprites(vec![fake_sprite(1, 8, 70, A_RGB)], vec![lot(ax, ay, 1, 8, ObjKind::House, 1)]);
    // The person stands on the tile beside the lot's middle (+x of its cell 3).
    let (tx, ty) = (ax + 1, ay + 3);
    let (px, py) = s.camera().project(tx as f32 + 0.5, ty as f32 + 0.5, 0);
    let depth = s.static_layer().depth().expect("the static layer carries a depth plane");
    let lw = s.static_layer().w;
    let mut tested = 0;
    for head in [14, 18, 22, 26] {
        let (x, y) = (px as i32, py as i32 - head);
        if colour_at(&s, x, y) != A_RGB {
            continue;
        }
        tested += 1;
        let wall = depth[y as usize * lw + x as usize];
        assert!(
            wall < actor_key(tx, ty),
            "a person at ({tx},{ty}) must draw over the wall (wall key {wall}, person {})",
            actor_key(tx, ty)
        );
    }
    assert!(tested > 0, "the sample points are on the long lot's wall");
}

/// A flat park behind a tall building never paints over it: the building's
/// pixels are the same with the park there and without it.
#[test]
fn a_flat_park_does_not_cover_a_building_in_front_of_it() {
    let mut s = scene(5);
    let (cx, cy) = centred_tile(&s);
    let tower = lot(cx + 2, cy - 4, 2, 2, ObjKind::Tower, 1);
    // Without the park: whatever the terrain does to the tower is the baseline.
    s.set_city_with_sprites(vec![fake_sprite(2, 2, 90, A_RGB)], vec![tower.clone()]);
    let base: Vec<u32> = s.static_layer().px().iter().map(|c| c & 0x00FF_FFFF).collect();
    // In front along +x of the park, beside its far rows.
    let park = lot(cx - 4, cy - 4, 6, 6, ObjKind::Park, 0);
    s.set_city_with_sprites(vec![fake_sprite(2, 2, 90, A_RGB)], vec![park, tower.clone()]);
    let cam = s.camera();
    let k = &s.kit_sprites()[0];
    let origin = cam.project(tower.x as f32, tower.y as f32, 0);
    let (sx, sy) = (origin.0 as i32 - k.ox, origin.1 as i32 - k.oy);
    let (mut wrong, mut seen) = (0, 0);
    let l = s.static_layer();
    for j in 0..k.h {
        for i in 0..k.w {
            if k.px[j * k.w + i] == 0 {
                continue;
            }
            let (x, y) = (sx + i as i32, sy + j as i32);
            if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
                continue;
            }
            seen += 1;
            let at = y as usize * l.w + x as usize;
            if l.px()[at] & 0x00FF_FFFF != base[at] {
                wrong += 1;
            }
        }
    }
    assert!(seen > 1000, "the tower is on screen ({seen} px)");
    assert_eq!(wrong, 0, "{wrong} of {seen} tower pixels were painted over by the park");
}
