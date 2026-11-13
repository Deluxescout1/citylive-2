//! B42b: what the horizon may and may not touch.
//!
//! The haze is a *distance* effect on bare ground only. Anything standing on it
//! — a wall, a roof, a lane, a tree — keeps its colour however far away it is,
//! and the map's ground above the land line is replaced by the painted distance
//! rather than melted into it.

use map::{generate, world_for_viewport};
use render::framebuffer::{TAG_BUILT, TAG_PROP};
use render::palette;
use render::{Framebuffer, Rect, Scene, SceneConfig, SceneMode};

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
    // A neutral light: any difference from the static layer is the haze.
    s.set_light([1.0, 1.0, 1.0], 0.0);
    // The haze takes its colour from the sky, so the sky has to exist.
    let sky = Framebuffer::new(W, s.sky_height());
    s.set_sky(sky);
    s
}

#[test]
fn the_haze_never_touches_a_solid_feature() {
    let s = scene(5);
    let sky_h = s.sky_height();
    let stat = s.static_layer();
    let tint = s.tinted_layer();
    let (mut prop, mut built, mut ground) = (0usize, 0usize, 0usize);
    for y in sky_h..(sky_h + 12).min(H) {
        for x in 0..W {
            let i = y * W + x;
            let src = stat.px()[i];
            if src == palette::TRANSPARENT {
                continue;
            }
            let a = palette::alpha(src);
            let plain = palette::with_alpha(src, 0xFF);
            if a == TAG_PROP {
                prop += 1;
                assert_eq!(tint.px()[i], plain, "a prop was hazed at {x},{y}");
            } else if a == TAG_BUILT {
                built += 1;
                assert_eq!(tint.px()[i], plain, "a built surface was hazed at {x},{y}");
            } else {
                ground += 1;
            }
        }
    }
    assert!(
        ground > 100,
        "the test sampled almost no bare ground ({ground})"
    );
    println!("haze band: {prop} prop px, {built} built px, {ground} ground px");
}

#[test]
fn bare_ground_is_replaced_by_the_sky_above_the_land_line() {
    let mut s = scene(5);
    // B69: a swaying tree is drawn over the band on purpose; this is about
    // the ground.
    s.set_tree_sway(false);
    let mut fb = Framebuffer::new(W, H);
    s.composite(&mut fb);
    let sky_h = s.sky_height() as i32;
    let sky = s.sky();
    let stat = s.static_layer();
    let mut checked = 0usize;
    for y in (sky_h - 6)..sky_h {
        for x in 0..W as i32 {
            let i = y as usize * W + x as usize;
            // Only where the map drew bare ground: anything that stands up
            // (a prop, a building) is kept whole (B90).
            let a = palette::alpha(stat.px()[i]);
            if stat.px()[i] != palette::TRANSPARENT && a != TAG_PROP && a != TAG_BUILT {
                assert_eq!(
                    fb.px()[i],
                    sky.px()[i],
                    "map content left visible above the horizon at {x},{y}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 50, "nothing to check above the line ({checked})");
}

#[test]
fn a_rect_composite_still_matches_through_the_band() {
    let s = scene(7);
    let mut full = Framebuffer::new(W, H);
    s.composite(&mut full);
    let mut part = Framebuffer::new(W, H);
    s.composite(&mut part);
    let sky_h = s.sky_height() as i32;
    for r in [
        Rect::new(0, (sky_h - 80).max(0), 200, sky_h + 40),
        Rect::new(300, 0, 500, sky_h),
        Rect::new(-40, (sky_h / 2), -10, (sky_h / 2) + 9),
    ] {
        s.composite_rect(&mut part, r);
    }
    let mut bad = 0;
    for i in 0..W * H {
        if full.px()[i] != part.px()[i] {
            bad += 1;
        }
    }
    assert_eq!(bad, 0, "{bad} px differ between full and rect composites");
}
