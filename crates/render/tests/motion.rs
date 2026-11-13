//! B67: smooth motion. What these tests lock in:
//!
//! * a cloud drifting across the sky is repainted by strips, and the frame that
//!   comes out of those strips is *the same frame* a full composite would make
//!   (no smear, no stale edge, nothing that "distorts" as it moves);
//! * a constant-speed drift steps in the even cadence pixel art allows (the
//!   sprite translation test is in `render::clouds`);
//! * a thing standing on a stepped hill does not pop a level when it crosses the
//!   seam.

use map::{Kind, Tile, TileMap, world_for_viewport};
use render::framebuffer::{Framebuffer, Rect};
use render::light::{LightState, Orb};
use render::{Probe, Scene, SceneConfig, SceneMode};

const W: usize = 640;
const H: usize = 320;

fn town() -> Scene {
    let vp = world_for_viewport(W as i32, H as i32);
    let mut map = TileMap::fill(vp.w, vp.h, Tile::flat(2), 5);
    for y in 0..map.h {
        for x in 0..map.w {
            let t = map.at(x, y);
            map.set(x, y, Tile { kind: Kind::Grass, ..t });
        }
    }
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
        0.30,
    );
    let mut ls = LightState::default();
    ls.sun = Orb::new(40.0, 200.0);
    ls.dir_strength = 1.0;
    ls.ambient = [1.0; 3];
    scene.set_light_state(ls);
    while scene.shadow_tick(1.0) {}
    scene
}

/// Apply the scene's pending damage to `fb` the way the wallpaper does.
fn apply(scene: &mut Scene, fb: &mut Framebuffer) {
    match scene.take_pending() {
        None => scene.composite(fb),
        Some(rects) => {
            for r in rects {
                scene.composite_rect(fb, r);
            }
        }
    }
}

fn probes(scene: &Scene, class: &str) -> Vec<Probe> {
    let mut v = Vec::new();
    scene.motion_probe(&mut v);
    v.retain(|p| p.class == class);
    v
}

/// Strip repainting must reproduce the full frame, frame after frame, at several
/// covers and winds: this is the difference test for "the clouds distort as they
/// move". Any pixel that differs is a cloud edge left behind or painted twice.
#[test]
fn drifting_clouds_repaint_to_exactly_the_full_frame() {
    for (cover, kmh) in [(0.25f32, 6.0f32), (0.6, 18.0), (0.9, 30.0), (0.45, 55.0)] {
        let mut scene = town();
        scene.set_cloud_wind(255.0, kmh);
        scene.set_clouds(cover, 0.0);
        let mut fb = Framebuffer::new(W, H);
        scene.composite(&mut fb);
        let _ = scene.take_pending();
        let mut worst = 0usize;
        for frame in 0..180 {
            scene.cloud_tick(1.0 / 30.0);
            apply(&mut scene, &mut fb);
            if frame % 6 == 5 {
                let mut full = Framebuffer::new(W, H);
                scene.composite(&mut full);
                let bad = fb.px().iter().zip(full.px()).filter(|(a, b)| a != b).count();
                worst = worst.max(bad);
            }
        }
        assert_eq!(
            worst, 0,
            "cover {cover}, {kmh} km/h: strips left {worst} px different from a full composite"
        );
    }
}

/// Constant-speed drift steps in the cadence pixel art allows: at 0.4 px/frame
/// the integer position goes 0-1-0-1-1 (never two steps in one frame, never a
/// run of zeros longer than the speed implies, never backward).
#[test]
fn a_constant_drift_steps_evenly() {
    let mut scene = town();
    scene.set_cloud_wind(270.0, 30.0);
    scene.set_clouds(0.5, 0.0);
    let n = probes(&scene, "cloud").len();
    let mut pos: Vec<Vec<(i32, f32)>> = vec![Vec::new(); n];
    for _ in 0..240 {
        scene.cloud_tick(1.0 / 30.0);
        for (i, p) in probes(&scene, "cloud").iter().enumerate() {
            pos[i].push((p.x, p.fx));
        }
    }
    let mut checked = 0;
    for track in &pos {
        let v = (track.last().unwrap().1 - track[0].1) / (track.len() - 1) as f32;
        // Skip a cloud that wrapped round the frame.
        if v.abs() < 0.05 || track.windows(2).any(|w| (w[1].1 - w[0].1).abs() > 8.0) {
            continue;
        }
        let steps: Vec<i32> = track.windows(2).map(|w| w[1].0 - w[0].0).collect();
        let sign = if v > 0.0 { 1 } else { -1 };
        assert!(steps.iter().all(|s| s * sign >= 0), "a cloud stepped backwards: {steps:?}");
        let ideal = (1.0 / v.abs()).ceil() as usize - 1;
        let mut run = 0usize;
        let mut worst = 0usize;
        for s in &steps {
            if *s == 0 {
                run += 1;
                worst = worst.max(run);
            } else {
                run = 0;
            }
            assert!(s.abs() <= v.abs().ceil() as i32, "a cloud stepped {s} px at {v:.2} px/frame");
        }
        assert!(worst <= ideal + 1, "a cloud stood still {worst} frames at {v:.2} px/frame (ideal {ideal})");
        checked += 1;
    }
    assert!(checked > 0, "no drifting cloud to check");
}

/// Two flat terraces a level apart: a thing walking across the seam climbs 12 px
/// over a fraction of a tile instead of popping.
#[test]
fn ground_height_is_continuous_across_a_terrace_seam() {
    let mut map = TileMap::fill(8, 8, Tile::flat(2), 1);
    for y in 0..8 {
        for x in 4..8 {
            map.set(x, y, Tile::flat(3));
        }
    }
    let mut prev = map.ground_px(2.0, 3.5);
    let mut biggest = 0;
    let mut x = 2.0f32;
    while x < 6.0 {
        let z = map.ground_px(x, 3.5);
        biggest = biggest.max((z - prev).abs());
        prev = z;
        x += 0.01;
    }
    assert!(biggest <= 1, "the height jumped {biggest} px in 0.01 of a tile");
    // Away from the seam it is exact.
    assert_eq!(map.ground_px(2.5, 3.5), 24);
    assert_eq!(map.ground_px(5.5, 3.5), 36);
    // The exact surface does pop (this is what the old code drew).
    let pop = ((map.surface_level_at(4.01, 3.5) - map.surface_level_at(3.99, 3.5)) * 12.0).round() as i32;
    assert_eq!(pop, 12);
    let _ = Rect::EMPTY;
}

/// The yardstick: repainting an arbitrary rectangle reproduces the full frame.
#[test]
fn repainting_a_rect_is_exact_without_clouds() {
    let mut scene = town();
    scene.set_clouds(0.25, 0.0);
    let mut full = Framebuffer::new(W, H);
    scene.composite(&mut full);
    let mut fb = Framebuffer::new(W, H);
    scene.composite(&mut fb);
    for r in [Rect::new(10, 20, 200, 100), Rect::new(0, 90, 300, 140), Rect::new(100, 0, 400, 320)] {
        scene.composite_rect(&mut fb, r);
    }
    let bad = fb.px().iter().zip(full.px()).filter(|(a, b)| a != b).count();
    assert_eq!(bad, 0);
}


/// What a drifting sky costs at the desktop's own scale (7040x1440 at 1x is a
/// 2347x480 frame): damage per frame and milliseconds per frame, at two covers.
/// `cargo test --release -p render --test motion -- --ignored --nocapture desktop_scale`
#[test]
#[ignore = "timing"]
fn cloud_cost_at_desktop_scale() {
    const DW: usize = 2347;
    const DH: usize = 480;
    for cover in [0.30f32, 0.80] {
        let vp = world_for_viewport(DW as i32, DH as i32);
        let map = TileMap::fill(vp.w, vp.h, Tile::flat(2), 5);
        let mut scene = Scene::from_map_with_sky(
            map,
            SceneConfig { width: DW, height: DH, seed: 5, mode: SceneMode::Sim, cars: 0, peds: 0, buildings: 0, rain: false },
            0.30,
        );
        let mut ls = LightState::default();
        ls.sun = Orb::new(40.0, 200.0);
        ls.dir_strength = 1.0;
        ls.ambient = [1.0; 3];
        scene.set_light_state(ls);
        while scene.shadow_tick(1.0) {}
        scene.set_cloud_wind(255.0, 18.0);
        scene.set_clouds(cover, 0.0);
        let mut fb = Framebuffer::new(DW, DH);
        scene.composite(&mut fb);
        let _ = scene.take_pending();
        let n = 300;
        let (mut tick, mut comp, mut area, mut fulls) = (0.0f64, 0.0f64, 0i64, 0);
        for _ in 0..n {
            let t = std::time::Instant::now();
            scene.cloud_tick(1.0 / 30.0);
            tick += t.elapsed().as_secs_f64();
            let t = std::time::Instant::now();
            match scene.take_pending() {
                None => {
                    fulls += 1;
                    scene.composite(&mut fb);
                    area += (DW * DH) as i64;
                }
                Some(rects) => {
                    for r in &rects {
                        area += r.area();
                        scene.composite_rect(&mut fb, *r);
                    }
                }
            }
            comp += t.elapsed().as_secs_f64();
        }
        let ms = |d: f64| d * 1000.0 / n as f64;
        println!(
            "clouds cover {:.0} % at {DW}x{DH}, 18 km/h: tick {:.3} ms + composite {:.3} ms a frame, {:.2} % of the frame damaged, \
             {fulls} full frames of {n}, {:.2} % of a core at 30 fps",
            cover * 100.0,
            ms(tick),
            ms(comp),
            area as f64 / n as f64 * 100.0 / (DW * DH) as f64,
            (ms(tick) + ms(comp)) * 30.0 / 10.0,
        );
    }
}

/// B67: a change of cover is a transition. Sweep the cover from overcast to clear
/// and back in the small steps an eased weather book hands the scene (0.004 is
/// what the book moves in about 3 s) and check that no step changes the picture
/// by more than a sliver: the number of clouds, their size, the deck and the
/// shadows all follow the cover continuously.
#[test]
fn no_single_step_of_cover_changes_the_picture_much() {
    for (from, to) in [(0.92f32, 0.08f32), (0.08, 0.92), (0.55, 0.30)] {
        let mut scene = town();
        scene.set_cloud_wind(255.0, 10.0);
        scene.set_clouds(from, 0.0);
        let mut prev = Framebuffer::new(W, H);
        scene.composite(&mut prev);
        let steps = ((to - from).abs() / 0.004).ceil() as usize;
        let (mut worst_mean, mut worst_changed) = (0.0f64, 0.0f64);
        let mut total_mean = 0.0f64;
        for i in 1..=steps {
            let c = from + (to - from) * i as f32 / steps as f32;
            scene.set_clouds(c, 0.0);
            // One tick so the damage machinery and the lives settle.
            scene.cloud_tick(1.0 / 30.0);
            let mut now = Framebuffer::new(W, H);
            scene.composite(&mut now);
            let n = (W * H) as f64;
            let mut sum = 0u64;
            let mut changed = 0usize;
            for (a, b) in prev.px().iter().zip(now.px()) {
                if a != b {
                    let big = [
                        (render::palette::r(*a), render::palette::r(*b)),
                        (render::palette::g(*a), render::palette::g(*b)),
                        (render::palette::b(*a), render::palette::b(*b)),
                    ]
                    .iter()
                    .any(|(x, y)| (*x as i32 - *y as i32).abs() > 24);
                    // A smooth fade nudges a great many pixels by a level; a pop
                    // moves some of them by a lot. Count the latter.
                    if big {
                        changed += 1;
                    }
                    for (x, y) in [
                        (render::palette::r(*a), render::palette::r(*b)),
                        (render::palette::g(*a), render::palette::g(*b)),
                        (render::palette::b(*a), render::palette::b(*b)),
                    ] {
                        sum += (x as i32 - y as i32).unsigned_abs() as u64;
                    }
                }
            }
            let mean = sum as f64 / (3.0 * n);
            worst_mean = worst_mean.max(mean);
            worst_changed = worst_changed.max(changed as f64 / n);
            total_mean += mean;
            prev = now;
        }
        println!(
            "cover {from} -> {to} in {steps} steps: worst step mean |dRGB| {worst_mean:.3} of 255, worst {:.2} % of pixels moved by more than 24/255, whole sweep {total_mean:.1}",
            worst_changed * 100.0
        );
        assert!(worst_mean < 1.2, "a step of cover changed the frame by {worst_mean:.2}/255 on average");
        assert!(worst_changed < 0.02, "a step of cover moved {:.1} % of the pixels by more than 24/255", worst_changed * 100.0);
    }
}

/// B67: a wash too faint to draw (the first minute of a slow fade-in) must not
/// ask for a full frame on every call: `set_ambient` took the empty row table for
/// a stale one. At 7040x1440 that was a 40 ms composite every frame while a front
/// was arriving.
#[test]
fn a_faint_wash_does_not_ask_for_a_full_frame_every_frame() {
    let mut scene = town();
    let frame = Rect::from_size(W, H);
    let faint = render::Ambient::new(0.008, 0.0, 0.0, 0.0, 0.0);
    scene.set_ambient(&faint, frame);
    let mut asked = 0;
    for _ in 0..30 {
        if scene.set_ambient(&faint, frame) {
            asked += 1;
        }
    }
    assert_eq!(asked, 0, "{asked} of 30 identical faint washes asked for a recomposite");
    // A real step still does.
    let stronger = render::Ambient::new(0.30, 0.0, 0.0, 0.0, 0.0);
    assert!(scene.set_ambient(&stronger, frame));
}
