//! B90: the horizon never cuts a standing thing.
//!
//! A tower on the back row rises past the land line. It is drawn over the sky
//! and the far land with every pixel of its roof; only bare ground behind the
//! horizon is masked away.

use map::{generate, world_for_viewport};
use render::framebuffer::{Framebuffer, TAG_BUILT, TAG_PROP};
use render::palette;
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
    // A sky that is not transparent, so "the sky shows" is a real colour.
    let mut sky = Framebuffer::new(W, s.sky_height());
    sky.clear(palette::rgb(0x70, 0x98, 0xD0));
    s.set_sky(sky);
    s
}

/// A tile whose centre projects `below` px under the land line, near mid-frame.
fn tile_below_the_line(s: &Scene, below: f32) -> (i32, i32) {
    let cam = s.camera();
    let line = s.sky_height() as f32;
    let mut best = ((0, 0), f32::MAX);
    for y in 0..s.map.h {
        for x in 0..s.map.w {
            let (px, py) = cam.project(x as f32 + 0.5, y as f32 + 0.5, 0);
            let d = (py - (line + below)).abs() + (px - W as f32 * 0.5).abs() * 0.2;
            if d < best.1 {
                best = ((x, y), d);
            }
        }
    }
    best.0
}

fn check(overcast: bool) {
    let mut s = scene(5);
    s.set_tree_sway(false);
    if overcast {
        s.set_clouds(0.97, 0.0);
    }
    let sky_h = s.sky_height() as i32;
    // Base ~30 px under the land line, 14 storeys (~150 px): the roof is about
    // 120 px above the line, further than the old 72 px painted-into-the-band
    // allowance.
    let (x, y) = tile_below_the_line(&s, 30.0);
    s.set_city(vec![CityObj {
        x,
        y,
        w: 2,
        h: 2,
        level: 0,
        kind: ObjKind::Landmark(render::city::LandmarkKind::SignatureTower),
        storeys: 14,
        style: 0,
        stage: render::city::Stage::Finished,
        seed: 7,
        front: 0,
        t: 0.0,
        kit: 0,
        wear: Default::default(),
    }]);
    let mut fb = Framebuffer::new(W, H);
    s.composite(&mut fb);
    let stat = s.static_layer();
    let tint = s.tinted_layer();
    // Every standing pixel the static layer holds above the land line, and the
    // highest row it reaches.
    let (mut above, mut shown, mut top_row) = (0usize, 0usize, sky_h);
    for yy in 0..sky_h {
        for xx in 0..W as i32 {
            let i = yy as usize * W + xx as usize;
            let p = stat.px()[i];
            let a = palette::alpha(p);
            if p != palette::TRANSPARENT && (a == TAG_BUILT || a == TAG_PROP) {
                above += 1;
                top_row = top_row.min(yy);
                if (fb.px()[i] ^ tint.px()[i]) & 0x00FF_FFFF == 0 {
                    shown += 1;
                }
            }
        }
    }
    assert!(
        top_row < sky_h - 90,
        "the tower's roof was cut at row {top_row} (land line {sky_h}); it should reach well above it"
    );
    assert!(above > 400, "the tower left only {above} px above the line");
    assert!(
        shown * 100 >= above * 99,
        "{} of {above} roof px above the land line were replaced by the sky or a cloud",
        above - shown
    );
}

#[test]
fn a_tower_whose_roof_is_above_the_land_line_keeps_every_roof_pixel() {
    check(false);
}

/// A deck and the cells are far behind a tower: they are never painted over it.
#[test]
fn a_cloud_deck_is_never_painted_over_a_tower_roof() {
    check(true);
}
