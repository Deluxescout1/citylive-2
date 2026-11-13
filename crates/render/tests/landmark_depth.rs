//! B65: a walker never draws over a tall landmark it is standing beside.
//!
//! A plot paints its whole sprite with one painter key, and a figure standing
//! on an *open* lot (a park, a site, a civic square) is lifted to that key so
//! the lawn cannot hide it (B43). The lift used to be to the plot's *building*
//! key, which is also the key of every wall the plot carries — so a walker on
//! the square under the clock tower was painted over its shaft. The lift is to
//! the plot's **ground** key now: over the lawn, behind the walls.

use map::{generate, world_for_viewport};
use render::actors::{actor_key, static_key};
use render::framebuffer::Framebuffer;
use render::{CityObj, ObjKind, Scene, SceneConfig, SceneMode};

const W: usize = 900;
const H: usize = 400;
const SKY: f32 = 0.30;

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

fn lot(x: i32, y: i32, w: i32, h: i32, kind: ObjKind, storeys: u8) -> CityObj {
    CityObj {
        x,
        y,
        w,
        h,
        level: 0,
        kind,
        storeys,
        style: 0,
        stage: render::city::Stage::Finished,
        seed: 7,
        front: 0,
        t: 0.0,
        kit: 0, wear: Default::default(),
    }
}

/// The key `people::draw` gives a figure standing on `(x, y)` with `open` set.
fn open_figure_key(s: &Scene, x: i32, y: i32) -> u16 {
    actor_key(x, y).max(s.city_key_at(x, y))
}

/// A tile near the middle of the frame, so a lot placed there is on screen.
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

#[test]
fn a_figure_on_a_tall_lots_square_stays_behind_its_walls() {
    let mut s = scene(5);
    let (cx, cy) = centred_tile(&s);
    let t = lot(
        cx,
        cy,
        2,
        2,
        ObjKind::Landmark(render::city::LandmarkKind::SignatureTower),
        12,
    );
    s.set_city(vec![t.clone()]);
    let diag = t.x + t.w - 1 + t.y + t.h - 1;
    let wall = static_key(diag, true);
    let ground = static_key(diag, false);
    assert!(ground < wall);
    for (tx, ty) in [(t.x, t.y), (t.x + 1, t.y), (t.x, t.y + 1), (t.x + 1, t.y + 1)] {
        assert_eq!(
            s.city_key_at(tx, ty),
            ground,
            "a figure on {tx},{ty} is lifted to the plot's ground key, not its walls"
        );
        assert!(
            open_figure_key(&s, tx, ty) < wall,
            "a figure on {tx},{ty} must stay behind the tower (key {wall})"
        );
    }
    // The tower's own pixels carry the wall key, so the test above is about the
    // real wall and not a figure-free corner.
    let depth = s
        .static_layer()
        .depth()
        .expect("the static layer carries a depth plane");
    let cam = s.camera();
    let (sx, base) = (
        cam.project((t.x + t.w) as f32, (t.y + t.h) as f32, 0).0,
        cam.project(t.x as f32, (t.y + t.h) as f32, 0).1,
    );
    let mut shaft = 0;
    let mut seen: std::collections::BTreeMap<u16, usize> = Default::default();
    for dy in 8..60 {
        let (x, y) = (sx.round() as i32, base as i32 - dy);
        if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
            continue;
        }
        let key = depth[y as usize * W + x as usize];
        *seen.entry(key).or_default() += 1;
        if key == wall {
            shaft += 1;
        }
    }
    assert!(shaft > 8, "only {shaft} wall pixels found on the shaft ({seen:?}, wall {wall})");
}

#[test]
fn a_figure_still_draws_over_the_lawn_it_stands_on() {
    let mut s = scene(5);
    let (cx, cy) = centred_tile(&s);
    let p = lot(cx, cy, 3, 3, ObjKind::Park, 0);
    s.set_city(vec![p.clone()]);
    let diag = p.x + p.w - 1 + p.y + p.h - 1;
    for (tx, ty) in [(p.x, p.y), (p.x + 2, p.y), (p.x, p.y + 2), (p.x + 2, p.y + 2)] {
        let key = open_figure_key(&s, tx, ty);
        assert!(
            key >= s.city_key_at(tx, ty),
            "the lawn on {tx},{ty} must not hide the figure standing on it"
        );
        // A park has no walls: the plot is drawn at its building key, so a
        // figure there is lifted all the way (nothing to hide behind).
        assert_eq!(s.city_key_at(tx, ty), static_key(diag, true));
    }
    // A site under construction is open framework, not a wall: builders on the
    // slab (B43) must still be lifted over the frame.
    let mut site = lot(cx, cy, 2, 2, ObjKind::House, 1);
    site.stage = render::city::Stage::Framing;
    s.set_city(vec![site]);
    let d = cx + 1 + cy + 1;
    assert_eq!(s.city_key_at(cx, cy), static_key(d, true), "a site lifts to its frame");
    // A tile with no lot on it lifts nothing.
    let far = (p.x + 12, p.y + 12);
    assert_eq!(s.city_key_at(far.0, far.1), 0);
}
