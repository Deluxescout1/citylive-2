//! `--bench`: time every per-frame and per-update cost, with no window.

use std::time::{Duration, Instant};

use render::actors::DepthMode;
use render::scale::upscale_into;
use render::{Framebuffer, Scene, SceneConfig};

use crate::args::Args;

fn pct(d: &[Duration], p: f64) -> f64 {
    if d.is_empty() {
        return 0.0;
    }
    let mut v: Vec<f64> = d.iter().map(|x| x.as_secs_f64() * 1000.0).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = (((v.len() - 1) as f64) * p).round() as usize;
    v[idx.min(v.len() - 1)]
}

fn mean(d: &[Duration]) -> f64 {
    if d.is_empty() {
        return 0.0;
    }
    d.iter().map(|x| x.as_secs_f64() * 1000.0).sum::<f64>() / d.len() as f64
}

fn max(d: &[Duration]) -> f64 {
    d.iter()
        .map(|x| x.as_secs_f64() * 1000.0)
        .fold(0.0, f64::max)
}

fn fingerprint(px: &[u32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &p in px {
        h ^= p as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn row(name: &str, d: &[Duration], note: &str) {
    println!(
        "  {:<26} mean {:>7.3} ms  p95 {:>7.3} ms  max {:>7.3} ms   {}",
        name,
        mean(d),
        pct(d, 0.95),
        max(d),
        note
    );
}

pub fn run(a: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = SceneConfig {
        width: a.w,
        height: a.h,
        seed: a.seed,
        mode: a.scene,
        cars: render::actors::DEFAULT_CARS,
        peds: render::actors::DEFAULT_PEDS,
        buildings: 150,
        rain: a.rain,
    };

    let t_build = Instant::now();
    let mut scene = Scene::new(cfg);
    scene.depth_mode = a.depth;
    let build_ms = t_build.elapsed().as_secs_f64() * 1000.0;

    // Layer memory at this framebuffer (D14/D61/B34): the buffers the renderer
    // actually keeps — no tinted copy, and the sky only as tall as its band.
    let lb = render::scene::layer_bytes(a.w, a.h, 0.30);
    let lb = render::scene::LayerBytes {
        static_b: lb.static_b + a.w * a.h * 4,
        ..lb
    };
    let mb = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
    println!(
        "  layers {}x{} (sky band {} px): static+tinted {:.1} (colour+depth+tinted) + sky {:.1} + frame {:.1} = {:.1} MB",
        a.w,
        a.h,
        render::scene::sky_pixels(a.h, 0.30),
        mb(lb.static_b),
        mb(lb.sky_b),
        mb(lb.frame_b),
        mb(lb.total())
    );

    // Full static rebuild (repeat, so a single cold pass does not dominate).
    let mut rebuild: Vec<Duration> = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        scene.rebuild_static();
        rebuild.push(t.elapsed());
    }

    // A light change no longer re-bakes a copy of the static layer (B34: the
    // tint is applied while compositing), so what it costs is the composite it
    // schedules: measure that, plus the set_light call itself.
    let mut retint: Vec<Duration> = Vec::new();
    for i in 0..6 {
        let f = 1.0 + (i as f32) * 0.02;
        let t = Instant::now();
        scene.set_light([f, f * 0.99, f * 0.98], 0.0);
        let mut fbm = Framebuffer::new(a.w, a.h);
        scene.composite(&mut fbm);
        retint.push(t.elapsed());
    }
    row("light change + full frame", &retint, "per-minute palette cadence");

    let mut frame = Framebuffer::new(a.w, a.h);
    let dt = 1.0 / 30.0f32;
    for _ in 0..30 {
        scene.step(dt);
        scene.composite(&mut frame);
    }

    // --- per-frame work
    let mut comp: Vec<Duration> = Vec::with_capacity(a.frames);
    let mut step: Vec<Duration> = Vec::with_capacity(a.frames);
    for _ in 0..a.frames {
        let t = Instant::now();
        scene.step(dt);
        step.push(t.elapsed());
        let t = Instant::now();
        scene.composite(&mut frame);
        comp.push(t.elapsed());
    }

    // --- actors on their own, per depth-test mode
    let mut act = [
        (DepthMode::PerPixel, Vec::with_capacity(a.frames)),
        (DepthMode::Anchor, Vec::with_capacity(a.frames)),
        (DepthMode::Off, Vec::with_capacity(a.frames)),
    ];
    for (mode, samples) in act.iter_mut() {
        scene.depth_mode = *mode;
        for _ in 0..a.frames {
            let t = Instant::now();
            scene.draw_actors(&mut frame);
            samples.push(t.elapsed());
        }
    }
    scene.depth_mode = DepthMode::PerPixel;

    // --- dirty flushes
    let mut flush1: Vec<Duration> = Vec::new();
    let mut flush50: Vec<Duration> = Vec::new();
    let mut rng = map::rng::Rng::new(0xF1_05);
    let mut visited1 = 0usize;
    let mut draws1 = 0usize;
    let mut visited50 = 0usize;
    let mut draws50 = 0usize;
    let (w, h) = (scene.viewport.w, scene.viewport.h);
    let visible_tile = |scene: &Scene, rng: &mut map::rng::Rng| -> (i32, i32) {
        for _ in 0..200 {
            let tx = rng.range(2, w - 3);
            let ty = rng.range(2, h - 3);
            let (sx, sy) = scene.cam.project(tx as f32 + 0.5, ty as f32 + 0.5, 0);
            if sx >= 0.0 && sx < a.w as f32 && sy >= 0.0 && sy < a.h as f32 {
                return (tx, ty);
            }
        }
        (w / 2, h / 2)
    };
    for _ in 0..20 {
        let (tx, ty) = visible_tile(&scene, &mut rng);
        scene.dirty_tile(tx, ty);
        let t = Instant::now();
        scene.flush_dirty();
        flush1.push(t.elapsed());
        visited1 = scene.stats.last_flush_visited;
        draws1 = scene.stats.last_flush_draws;
    }
    for _ in 0..20 {
        for _ in 0..50 {
            let (tx, ty) = visible_tile(&scene, &mut rng);
            scene.dirty_tile(tx, ty);
        }
        let t = Instant::now();
        scene.flush_dirty();
        flush50.push(t.elapsed());
        visited50 = scene.stats.last_flush_visited;
        draws50 = scene.stats.last_flush_draws;
    }

    // --- upscales
    let mut up2 = Vec::new();
    let mut up3 = Vec::new();
    let mut up4 = Vec::new();
    let (mut t2, mut t3, mut t4) = (
        Vec::with_capacity(a.frames),
        Vec::with_capacity(a.frames),
        Vec::with_capacity(a.frames),
    );
    for _ in 0..a.frames {
        let t = Instant::now();
        upscale_into(frame.px(), a.w, a.h, &mut up2, 2);
        t2.push(t.elapsed());
        let t = Instant::now();
        upscale_into(frame.px(), a.w, a.h, &mut up3, 3);
        t3.push(t.elapsed());
        let t = Instant::now();
        upscale_into(frame.px(), a.w, a.h, &mut up4, 4);
        t4.push(t.elapsed());
    }

    println!(
        "bench  {}x{}  seed {}  scene {:?}  rain {}  frames {}  world {}x{}",
        a.w,
        a.h,
        a.seed,
        a.scene,
        if a.rain { "on" } else { "off" },
        a.frames,
        scene.viewport.w,
        scene.viewport.h
    );
    println!(
        "  scene build {:.1} ms, peak rss {} kB, frame hash {:#x}, {} buildings, {} cars, {} people, {} trees, {} water tiles",
        build_ms,
        crate::procstat::rss_peak_kb().unwrap_or(0),
        fingerprint(frame.px()),
        scene.buildings.len(),
        scene.life.cars.len(),
        scene.life.peds.len(),
        scene.map.count_trees(),
        scene.map.count_kind(map::Kind::Water)
    );
    row(
        "full static rebuild",
        &rebuild,
        "terrain + buildings + depth plane",
    );
    row("sim step", &step, "cars + people + weather");
    row("composite", &comp, "sky copy + static blit + actors + rain");
    row("actors: depth per pixel", &act[0].1, "P4 default");
    row("actors: depth per anchor", &act[1].1, "one probe per actor");
    row(
        "actors: no depth test",
        &act[2].1,
        "H0 behaviour, for comparison",
    );
    row(
        "flush 1 tile",
        &flush1,
        &format!("{draws1} items drawn of {visited1} visited"),
    );
    row(
        "flush 50 tiles",
        &flush50,
        &format!("{draws50} items drawn of {visited50} visited"),
    );
    row(&format!("upscale x2 -> {}x{}", a.w * 2, a.h * 2), &t2, "");
    row(&format!("upscale x3 -> {}x{}", a.w * 3, a.h * 3), &t3, "");
    row(&format!("upscale x4 -> {}x{}", a.w * 4, a.h * 4), &t4, "");
    let _ = (up2.len(), up3.len(), up4.len());
    Ok(())
}
