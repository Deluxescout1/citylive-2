//! B42d: a short output beside tall ones must show the matching crop of the
//! union frame — not stripes.
//!
//! the owner's desk is 2560×1440 + 2560×1440 + 1920×1080 (one 7040×1440 union). The
//! whole HDMI monitor showed vertical bands of one flat colour each, which is
//! the signature of `Scene::fill_far_floor` (the sky band's bottom row laid
//! under every land row) showing through a static layer that has no ground
//! there.
//!
//! These tests build the frame the way the wallpaper does — the sim's map for
//! the *full* framebuffer, the scene's own camera for the land band — and then
//! slice it per output.

use present::slice::slice_into_output;
use render::{Framebuffer, Scene, SceneConfig, SceneMode};

fn triple() -> [(String, i32, i32, u32, u32); 3] {
    [
        ("DP-3".to_string(), 0, 0, 2560, 1440),
        ("DP-4".to_string(), 2560, 0, 2560, 1440),
        ("HDMI-A-3".to_string(), 5120, 0, 1920, 1080),
    ]
}

/// The live path: `present::WallpaperState::setup_scene_and_sim` builds the sim
/// for `world_for_viewport(fb_w, fb_h)` and the scene deferred for the same
/// size, then attaches the city (which rasterises the static layer) and
/// composites once.
fn live_frame(seed: u64, days: f64, fb_w: usize, fb_h: usize) -> (Framebuffer, Scene) {
    let vp = map::world_for_viewport(fb_w as i32, fb_h as i32);
    let (fx, fy) = map::screen_to_tile(&vp, fb_w as f32 * 0.5, fb_h as f32 * 0.5);
    let mut sim = sim::Sim::with_focus(seed, sim::Pace::Month, vp.w, vp.h, (fx, fy));
    sim.found_again();
    sim.run_to(days * 86400.0);

    let mut scene = Scene::from_map_deferred(
        sim.map.clone(),
        SceneConfig {
            width: fb_w,
            height: fb_h,
            seed,
            mode: SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        },
        0.30,
    );
    let cam = (scene.cam.ox, scene.cam.oy);
    sim.set_viewport(cam.0, cam.1, fb_w as i32, fb_h as i32);

    let mut kit = buildkit::KitCache::new();
    let mut objs = endfx::kinds::objects(&sim);
    kit.attach(&mut objs, &buildkit::light_for([1.0, 1.0, 1.0], 0.0, 12.0));
    let sprites = kit.sprites_from(0);
    scene.set_city_with_sprites(sprites, objs);

    let mut fb = Framebuffer::new(fb_w, fb_h);
    scene.composite(&mut fb);
    (fb, scene)
}

#[test]
fn the_union_frame_has_no_far_floor_under_the_land() {
    let (w, h) = (7040usize, 1440usize);
    let (fb, scene) = live_frame(2184655232852813596, 0.3, w, h);
    let sky_h = scene.sky_height();
    println!(
        "union {w}x{h}: scene viewport {:?} map {}x{} sky_h {sky_h}",
        scene.viewport,
        scene.map.w,
        scene.map.h
    );

    // Every land pixel must come from the map, not from `fill_far_floor`: the
    // far floor is only eleven rows of the sky band's bottom row repeated, so
    // a patch of it reads as columns of one flat colour.
    for (name, x0, _y0, hw, hh) in triple() {
        let x1 = (x0 as usize + hw as usize).min(w);
        let mut stripes = 0usize;
        for x in x0 as usize..x1 {
            let mut same = true;
            let first = fb.px()[(sky_h + 8) * w + x];
            for y in (sky_h + 8)..(hh as usize).min(h) {
                if fb.px()[y * w + x] != first {
                    same = false;
                    break;
                }
            }
            if same {
                stripes += 1;
            }
        }
        assert_eq!(
            stripes, 0,
            "head '{name}': {stripes} column(s) are one flat colour from the horizon down"
        );
    }
}

/// The brief's regression: what a 1080-tall output shows equals the matching
/// crop of the union frame — through **both** present paths (the layer-shell
/// slice and the Plasma plugin's PPM frame).
#[test]
fn a_short_output_shows_the_matching_crop_of_the_union() {
    let (w, h) = (7040usize, 1440usize);
    let (fb, _scene) = live_frame(2184655232852813596, 0.3, w, h);

    for (name, x0, y0, hw, hh) in triple() {
        let mut dst = vec![0u32; (hw * hh) as usize];
        slice_into_output(
            fb.px(),
            w,
            h,
            1,
            0,
            0,
            x0,
            y0,
            hw,
            hh,
            1,
            &mut dst,
        );
        // The plugin's frame: the same pixels, cropped from the union.
        let crop = present::slice::head_crop(w, h, 1, (0, 0), (x0, y0), (hw, hh))
            .unwrap_or_else(|| panic!("head '{name}' is outside the union"));
        assert_eq!(
            (crop.x, crop.y, crop.w, crop.h),
            (x0 as usize, y0 as usize, hw as usize, hh as usize),
            "head '{name}': the plugin crop is not the head's rectangle"
        );
        let mut frame = Vec::new();
        present::slice::copy_head_frame(fb.px(), w, &crop, &mut frame);
        assert_eq!(
            frame.as_slice(),
            dst.as_slice(),
            "head '{name}': the plugin frame differs from the layer-shell slice"
        );
        let mut bad = 0;
        for row in 0..hh as usize {
            for col in 0..hw as usize {
                let want = fb.px()[(y0 as usize + row) * w + x0 as usize + col];
                if frame[row * hw as usize + col] != want {
                    bad += 1;
                }
            }
        }
        assert_eq!(bad, 0, "head '{name}': {bad} pixel(s) differ from the union crop");
    }
}

/// B42d: the export only rewrites the heads whose slice changed. A change on
/// DP-3 must not touch the HDMI output's file.
#[test]
fn only_the_head_that_changed_is_exported() {
    let (w, h) = (7040usize, 1440usize);
    let x0 = 500; // a small change well inside DP-3
    let damage = render::Rect::new(x0, 200, x0 + 40, 240);
    let heads = triple();
    let mut written: Vec<&str> = Vec::new();
    for (name, hx, hy, hw, hh) in heads.iter() {
        let crop = present::slice::head_crop(w, h, 1, (0, 0), (*hx, *hy), (*hw, *hh)).unwrap();
        if present::slice::rect_hits_head(&damage, &crop) {
            written.push(name);
        }
    }
    assert_eq!(written, vec!["DP-3"], "{written:?}");
    // A change that straddles the DP-4/HDMI seam must write both.
    let seam = render::Rect::new(5000, 0, 5300, 1440);
    let mut written: Vec<&str> = Vec::new();
    for (name, hx, hy, hw, hh) in heads.iter() {
        let crop = present::slice::head_crop(w, h, 1, (0, 0), (*hx, *hy), (*hw, *hh)).unwrap();
        if present::slice::rect_hits_head(&seam, &crop) {
            written.push(name);
        }
    }
    assert_eq!(written, vec!["DP-4", "HDMI-A-3"], "{written:?}");
}

/// B42e: the 128-row strips tile a head exactly, and a damage rectangle only
/// touches the strips it actually overlaps — that is what keeps the QML's
/// base + strips composite equal to the frame.
#[test]
fn strips_tile_a_head_and_cover_only_what_changed() {
    use present::slice;
    use present::plasma::STRIP_ROWS;

    for (_name, x0, y0, hw, hh) in triple() {
        let crop = slice::head_crop(7040, 1440, 1, (0, 0), (x0, y0), (hw, hh)).unwrap();
        let n = slice::strip_count(crop.h, STRIP_ROWS);
        let mut next = 0;
        for k in 0..n {
            let (row0, rows) = slice::strip_rows(k, crop.h, STRIP_ROWS);
            assert_eq!(row0, next, "strip {k} starts where the last one ended");
            assert!(rows > 0 && rows <= STRIP_ROWS, "strip {k} has {rows} rows");
            next += rows;
        }
        assert_eq!(next, crop.h, "the strips cover the whole head");

        // A 20-row bird band inside strip 2 touches exactly strip 2.
        let (row0, _) = slice::strip_rows(2, crop.h, STRIP_ROWS);
        let damaged = slice::strips_for_range((row0 + 3) as i32, (row0 + 23) as i32, crop.h, STRIP_ROWS);
        assert_eq!(damaged, vec![2], "a bird inside one strip writes one strip");
        // A band across the strip 2/3 boundary touches both.
        let (row3, _) = slice::strip_rows(3, crop.h, STRIP_ROWS);
        let damaged = slice::strips_for_range((row3 - 5) as i32, (row3 + 5) as i32, crop.h, STRIP_ROWS);
        assert_eq!(damaged, vec![2, 3]);
        // A rectangle entirely below the head touches nothing.
        assert!(slice::strips_for_range(crop.h as i32 + 5, crop.h as i32 + 40, crop.h, STRIP_ROWS).is_empty());
        // The last row of the head is in the last strip.
        assert_eq!(
            slice::strips_for_range(crop.h as i32 - 1, crop.h as i32, crop.h, STRIP_ROWS),
            vec![n - 1]
        );
    }
}
