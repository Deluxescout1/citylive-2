//! B42 regression test: a rectangle composite must reproduce exactly what the
//! full composite puts there.
//!
//! The wallpaper repaints only damaged rectangles and copies them to the
//! screen, so anything `composite_rect` draws differently from `composite`
//! shows up as a permanent artefact (the sky-trail bug was one of these). The
//! horizon fade (B42) is applied per rectangle, so it must be position-exact.

use map::{generate, world_for_viewport};
use render::{Framebuffer, Rect, Scene, SceneConfig, SceneMode};

fn scene(seed: u64, w: usize, h: usize, sky: f32) -> Scene {
    let land_h = (h as f32 * (1.0 - sky)) as i32;
    let vp = world_for_viewport(w as i32, land_h);
    let map = generate(seed, vp.w, vp.h);
    let cfg = SceneConfig {
        width: w,
        height: h,
        seed,
        mode: SceneMode::Sim,
        cars: 0,
        peds: 0,
        buildings: 0,
        rain: false,
    };
    let mut s = Scene::from_map_with_sky(map, cfg, sky);
    s.set_light([0.92, 0.86, 0.9], 0.2);
    s
}

#[test]
fn a_rect_composite_matches_the_full_frame() {
    for (w, h, sky) in [(960, 540, 0.30), (1600, 400, 0.30), (400, 300, 0.0)] {
        let s = scene(5, w, h, sky);
        let mut full = Framebuffer::new(w, h);
        s.composite(&mut full);

        let mut part = Framebuffer::new(w, h);
        s.composite(&mut part);
        // Overwrite the region with a single rect composite, exactly as the
        // wallpaper does after a repaint.
        let sky_h = s.sky_height() as i32;
        let rects = [
            Rect::new(0, 0, w as i32, h as i32),
            Rect::new(0, (sky_h - 90).max(0), 320, sky_h + 90),
            Rect::new(200, (sky_h - 30).max(0), 700, sky_h + 4),
            Rect::new(0, h as i32 - 40, w as i32, h as i32),
            // Damage rects that are entirely off-screen (an animal that left
            // the frame): these must change nothing.
            Rect::new(-186, (sky_h / 2), -176, (sky_h / 2) + 6),
            Rect::new(w as i32 + 4, 10, w as i32 + 30, 40),
            Rect::new(10, -30, 40, -2),
        ];
        for r in rects {
            s.composite_rect(&mut part, r);
        }
        let mut bad = 0usize;
        let mut first = None;
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let i = y as usize * w + x as usize;
                if full.px()[i] != part.px()[i] {
                    bad += 1;
                    if first.is_none() {
                        first = Some((x, y));
                    }
                }
            }
        }
        assert_eq!(bad, 0, "{w}x{h} sky {sky}: {bad} px differ (first {first:?})");
    }
}
