//! Measurements at the owner's monitor union (PLAN P1/P3): the 32-px path runs a
//! 3520×720 internal buffer upscaled ×2 to 7040×1440; the 64-px path runs
//! 7040×1440 at 1×. Reports static-layer composite time, layer RSS, idle
//! CPU and a full-screen redraw (sunset tint).

use std::time::Instant;

use render::Framebuffer;
use render::palette;

use crate::col::Mode;
use crate::procstat;
use crate::scene::Scene;

struct OneRun {
    label: &'static str,
    w: usize,
    h: usize,
    tw: i32,
}

/// One full measurement pass; returns a markdown table row source.
pub fn run() {
    println!("| path | internal fb | static composite | layer RSS | idle CPU | sunset tint |");
    println!("|---|---|---|---|---|---|");
    for run in [
        OneRun { label: "32-px tiles @2x", w: 3520, h: 720, tw: 32 },
        OneRun { label: "64-px tiles @1x", w: 7040, h: 1440, tw: 64 },
    ] {
        measure(run);
    }
}

fn measure(run: OneRun) {
    let (w, h, tw) = (run.w, run.h, run.tw);

    // ---- layer stack: sky + static(with depth) + dynamic ----
    let rss0 = procstat::rss_kb().unwrap_or(0);
    let mut sky = Framebuffer::new(w, h);
    let mut static_fb = Framebuffer::with_depth(w, h);
    let mut dynamic = Framebuffer::new(w, h);
    // touch every page
    sky.px_mut().fill(0xFF000000);
    dynamic.px_mut().fill(0x00000000);

    // ---- static composite time (fresh rasterization) ----
    let scene = Scene::new(tw, w, h, Mode::Day, 0xC17C_2025);
    let mut times = Vec::new();
    for i in 0..6 {
        let t = Instant::now();
        let crops = scene.render(&mut static_fb);
        let el = t.elapsed().as_secs_f64() * 1000.0;
        if i > 0 {
            times.push(el);
        }
        let _ = crops.len();
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let composite_med = times[times.len() / 2];
    let composite_min = times[0];

    // RSS delta after a full render (all pages of the layers are hot)
    let rss1 = procstat::rss_kb().unwrap_or(0);
    let layer_rss_mb = (rss1 as i64 - rss0 as i64) as f64 / 1024.0;
    let theory_mb =
        (w * h * 4) as f64 * 3.0 / 1_048_576.0 + (w * h * 2) as f64 / 1_048_576.0;

    // ---- idle CPU: 30 fps change-check loop, nothing dirty ----
    let idle_secs = 5.0;
    let t0 = Instant::now();
    let cpu0 = procstat::process_cpu_ticks().unwrap_or(0);
    let mut ticks = 0u64;
    while t0.elapsed().as_secs_f64() < idle_secs {
        // the wallpaper's steady-state per-frame work when nothing changed:
        // scan the dirty list (empty) and hash-check the frame before present.
        ticks = ticks.wrapping_add(static_fb.px()[0] as u64);
        std::thread::sleep(std::time::Duration::from_millis(33));
    }
    let wall = t0.elapsed().as_secs_f64();
    let cpu1 = procstat::process_cpu_ticks().unwrap_or(0);
    let idle_pct = (cpu1 - cpu0) as f64 / procstat::USER_HZ / wall * 100.0;
    let _ = ticks;

    // ---- sunset tint: full-screen re-light pass ----
    let mut tint_times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        for p in static_fb.px_mut().iter_mut() {
            // same shape as sky/light.rs set_light: multiply toward the tint
            *p = palette::mix(palette::shade(*p, 0.92), palette::rgb(0xe0, 0x8a, 0x4c), 0.28);
        }
        tint_times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    tint_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let tint_med = tint_times[tint_times.len() / 2];

    println!(
        "| {} | {}×{} | {:.1} ms med ({} ms min) | layers {:.0} MB (theory {:.0} MB) | {:.3}% | {:.1} ms |",
        run.label,
        w,
        h,
        composite_med,
        format!("{composite_min:.0}"),
        layer_rss_mb,
        theory_mb,
        idle_pct,
        tint_med
    );
}

/// Timed composite of the 1920×1080 deliverables too (matches the PNGs).
pub fn run_1080() {
    println!();
    println!("Deliverable frames (1920×1080 on-screen):");
    println!("| path | internal | composite |");
    println!("|---|---|---|");
    for (label, tw, w, h) in [
        ("32-px @2x", 32i32, 960usize, 540usize),
        ("64-px @1x", 64, 1920, 1080),
    ] {
        let mut fb = Framebuffer::new(w, h);
        let scene = Scene::new(tw, w, h, Mode::Day, 0xC17C_2025);
        let mut times = Vec::new();
        for i in 0..6 {
            let t = Instant::now();
            scene.render(&mut fb);
            if i > 0 {
                times.push(t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("| {} | {}×{} | {:.1} ms |", label, w, h, times[times.len() / 2]);
    }
}
