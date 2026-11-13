//! B42 regression test: the sky-trail bug.
//!
//! The wallpaper draws the wildlife layer straight into the frame buffer at
//! ~6 fps and repaints only the rectangles it reports. If the caller does not
//! restore the pixels the *previous* pass painted, a flying bird is never
//! erased and leaves a dotted trail across the sky (the owner's D64 screenshots).
//!
//! This test does exactly what the wallpaper does — composite the scene once,
//! then restore the reported rects from the cached scene before every draw —
//! and asserts the only pixels that differ from the animal-free scene are
//! inside the sprite boxes of the animals that are out *now*.

use map::{generate, world_for_viewport};
use render::framebuffer::Framebuffer;
use render::{Scene, SceneConfig, SceneMode};
use wildlife::Wildlife;

const W: usize = 960;
const H: usize = 540;
const SKY: f32 = 0.30;

fn scene_for(seed: u64) -> Scene {
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
    let mut scene = Scene::from_map_with_sky(map, cfg, SKY);
    scene.set_light([1.0, 1.0, 1.0], 0.0);
    scene
}

#[test]
fn no_wildlife_pixels_survive_outside_the_current_sprites() {
    let seed = 3;
    let mut scene = scene_for(seed);

    // The animal-free frame every tick is compared against.
    let mut clean = Framebuffer::new(W, H);
    scene.composite(&mut clean);

    let mut fb = Framebuffer::new(W, H);
    scene.composite(&mut fb);

    let mut wl = Wildlife::new(seed ^ 0x77, &scene.map);
    let vp = map::Viewport {
        origin: (scene.cam.ox, scene.cam.oy),
        w: W as i32,
        h: H as i32,
    };

    let mut stale: Vec<render::Rect> = Vec::new();
    let mut painted_any = false;
    for tick in 0..90 {
        // What the wallpaper does first: restore last tick's rectangles from
        // the cached scene (sky band included).
        for r in stale.drain(..) {
            scene.composite_rect(&mut fb, r);
        }
        // Six frames of a second at the wallpaper's wildlife rate, crossing
        // dusk so the day/night mask flips and animals leave the frame.
        let hour = 17.0 + tick as f32 * 0.06;
        for _ in 0..5 {
            wl.update(1.0 / 30.0, &scene.map, &[], hour);
        }
        wl.draw(&mut fb, &vp, Some(scene.depth()));
        stale.extend_from_slice(wl.dirty_rects());

        // Any pixel the animals changed must be inside a sprite drawn *now*.
        let boxes = wl.drawn_rects();
        let mut stray = 0usize;
        let mut first: Option<(i32, i32, u32, u32)> = None;
        for y in 0..H as i32 {
            for x in 0..W as i32 {
                let i = y as usize * W + x as usize;
                let (a, b) = (fb.px()[i], clean.px()[i]);
                if a == b {
                    continue;
                }
                painted_any = true;
                let inside = boxes
                    .iter()
                    .any(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1);
                if !inside {
                    stray += 1;
                    if first.is_none() {
                        first = Some((x, y, a, b));
                    }
                }
            }
        }
        assert_eq!(
            stray,
            0,
            "tick {tick}: {stray} wildlife pixels left outside the current sprites (first at {:?})",
            first
        );
    }
    assert!(painted_any, "the test never painted an animal — it proves nothing");
}

/// The naive path the wallpaper used before B42 (draw, never restore) must
/// fail the same check — so the assertion above is known to catch the bug.
#[test]
fn the_bug_is_detectable_without_restore() {
    let seed = 3;
    let mut scene = scene_for(seed);
    let mut clean = Framebuffer::new(W, H);
    scene.composite(&mut clean);
    let mut fb = Framebuffer::new(W, H);
    scene.composite(&mut fb);

    let mut wl = Wildlife::new(seed ^ 0x77, &scene.map);
    let vp = map::Viewport {
        origin: (scene.cam.ox, scene.cam.oy),
        w: W as i32,
        h: H as i32,
    };
    let mut stray_total = 0usize;
    for tick in 0..40 {
        let hour = 8.0 + tick as f32 * 0.05;
        for _ in 0..5 {
            wl.update(1.0 / 30.0, &scene.map, &[], hour);
        }
        wl.draw(&mut fb, &vp, Some(scene.depth()));
        let boxes = wl.drawn_rects();
        for y in 0..H as i32 {
            for x in 0..W as i32 {
                let i = y as usize * W + x as usize;
                if fb.px()[i] != clean.px()[i]
                    && !boxes.iter().any(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1)
                {
                    stray_total += 1;
                }
            }
        }
    }
    assert!(
        stray_total > 0,
        "without the restore the trail must show up; the test harness is wrong"
    );
}
