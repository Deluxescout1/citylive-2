//! `--window`: winit window + softbuffer presentation, nearest-neighbour
//! integer upscale on the CPU, sleep-based frame pacing (never busy-waits).

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use render::scale::upscale_to_size_in;
use render::{Framebuffer, Scene, SceneConfig};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::args::Args;
use crate::procstat;

struct Totals {
    frames: u64,
    sim: Duration,
    comp: Duration,
    present: Duration,
}

struct App<'a> {
    args: &'a Args,
    scene: Scene,
    /// Live growth (B9 item 8): the sim drives the window frame by frame.
    live: Option<sim::Sim>,
    frame: Framebuffer,
    up: Vec<u32>,
    window: Option<Arc<Window>>,
    // Declared after `window` so the surface is dropped before the window.
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    _context: Option<softbuffer::Context<Arc<Window>>>,
    cities: u32,
    next_frame: Instant,
    started: Instant,
    last_report: Instant,
    since_report: Totals,
    total: Totals,
    dt: f32,
    frame_dt: Duration,
    cpu_ticks_start: u64,
}

impl<'a> App<'a> {
    fn new(args: &'a Args) -> Self {
        let cfg = SceneConfig {
            width: args.w,
            height: args.h,
            seed: args.seed,
            mode: args.scene,
            cars: render::actors::DEFAULT_CARS,
            peds: render::actors::DEFAULT_PEDS,
            buildings: 150,
            rain: args.rain,
        };
        let (mut scene, live) = if args.sim {
            let vp = map::world_for_viewport(args.w as i32, args.h as i32);
            let pace = sim::Pace::from_name(&args.pace).unwrap_or(sim::Pace::Month);
            let mut sim = sim::Sim::with_focus_in(
                args.seed,
                pace,
                vp.w,
                vp.h,
                (vp.w as f32 * 0.5, vp.h as f32 * 0.5),
                args.biome,
            );
            sim.set_viewport(vp.origin.0, vp.origin.1, args.w as i32, args.h as i32);
            let (fx, fy) = map::screen_to_tile(&vp, args.w as f32 * 0.5, args.h as f32 * 0.5);
            sim.set_focus(fx, fy);
            sim.run_to(args.sim_days * 86400.0);
            let mut scene = Scene::from_map(sim.map.clone(), cfg);
            scene.set_town_stats(sim.population(), sim.era().index());
            scene.set_city(endfx::kinds::objects(&sim));
            (scene, Some(sim))
        } else {
            (Scene::new(cfg), None)
        };
        scene.depth_mode = args.depth;
        println!(
            "window  {}x{} internal -> {}x{} present (cpu nearest x{})  fps {}  rain {}  scene {:?}  seed {}",
            args.w,
            args.h,
            args.w * args.scale,
            args.h * args.scale,
            args.scale,
            args.fps,
            if args.rain { "on" } else { "off" },
            args.scene,
            args.seed
        );
        println!(
            "  static layer rebuilt in {:.1} ms, {} buildings, {} cars, {} people",
            scene.stats.full_rebuild_ms,
            scene.buildings.len(),
            scene.life.cars.len(),
            scene.life.peds.len()
        );
        println!("  rss at start {} kB", procstat::rss_kb().unwrap_or(0));

        let frame_dt = if args.fps == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(1.0 / args.fps as f64)
        };
        let now = Instant::now();
        Self {
            args,
            frame: Framebuffer::new(args.w, args.h),
            up: Vec::new(),
            scene,
            live,
            window: None,
            surface: None,
            _context: None,
            cities: 0,
            next_frame: now,
            started: now,
            last_report: now,
            since_report: Totals {
                frames: 0,
                sim: Duration::ZERO,
                comp: Duration::ZERO,
                present: Duration::ZERO,
            },
            total: Totals {
                frames: 0,
                sim: Duration::ZERO,
                comp: Duration::ZERO,
                present: Duration::ZERO,
            },
            dt: if args.fps == 0 {
                1.0 / 30.0
            } else {
                1.0 / args.fps as f32
            },
            frame_dt,
            cpu_ticks_start: procstat::process_cpu_ticks().unwrap_or(0),
        }
    }

    fn report(&self, tag: &str, t: &Totals, span: f64, elapsed: f64) {
        let n = t.frames.max(1) as f64;
        println!(
            "[{:>5.1}s] {:<4} frames={} fps={:.1}  sim={:.3} ms  composite={:.3} ms  upscale+present={:.3} ms",
            elapsed,
            tag,
            t.frames,
            t.frames as f64 / span.max(1e-9),
            t.sim.as_secs_f64() * 1000.0 / n,
            t.comp.as_secs_f64() * 1000.0 / n,
            t.present.as_secs_f64() * 1000.0 / n
        );
    }

    fn render(&mut self, el: &ActiveEventLoop) {
        let (Some(window), Some(surface)) = (self.window.as_ref(), self.surface.as_mut()) else {
            return;
        };

        let t0 = Instant::now();
        if let Some(s) = &mut self.live {
            // Real seconds in, sim seconds out (pace × speed), then hand the
            // renderer only the tiles that moved.
            let real = self.frame_dt.as_secs_f64().max(1.0 / 240.0);
            s.advance(real * self.args.speed.max(0.1));
            let changed = s.take_changes();
            for (x, y) in changed {
                let t = s.map.at(x, y);
                self.scene.set_tile(x, y, t);
            }
            self.cities += 1;
            if self.cities % 15 == 0 {
                let objs = endfx::kinds::objects(s);
                self.scene.set_town_stats(s.population(), s.era().index());
                self.scene.set_city(objs);
            }
        }
        self.scene.step(self.dt);
        self.scene.sign_tick(self.dt);
        if self.scene.has_dirty() {
            self.scene.flush_dirty();
        }
        let t1 = Instant::now();
        self.scene.composite(&mut self.frame);
        let t2 = Instant::now();

        let size = window.inner_size();
        let (dw, dh) = (size.width as usize, size.height as usize);
        let presented = if dw > 0 && dh > 0 {
            // B61: the destination buffer is resized once, then every frame
            // expands into it in place (this used to reallocate and rebuild the
            // column map per frame).
            let n = dw * dh;
            if self.up.len() != n {
                self.up.resize(n, 0);
            }
            upscale_to_size_in(self.frame.px(), self.frame.w, self.frame.h, &mut self.up, dw, dh);
            if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
            {
                surface.resize(w, h).ok();
            }
            match surface.buffer_mut() {
                Ok(mut buf) => {
                    let n = buf.len().min(self.up.len());
                    buf[..n].copy_from_slice(&self.up[..n]);
                    buf.present().is_ok()
                }
                Err(_) => false,
            }
        } else {
            false
        };
        let t3 = Instant::now();

        if presented {
            self.since_report.frames += 1;
            self.since_report.sim += t1 - t0;
            self.since_report.comp += t2 - t1;
            self.since_report.present += t3 - t2;
            self.total.frames += 1;
            self.total.sim += t1 - t0;
            self.total.comp += t2 - t1;
            self.total.present += t3 - t2;
        }

        // Every 5 s, print the average breakdown for that interval.
        if self.last_report.elapsed().as_secs_f64() >= 5.0 {
            let span = self.last_report.elapsed().as_secs_f64();
            let elapsed = self.started.elapsed().as_secs_f64();
            self.report("last", &self.since_report, span, elapsed);
            self.last_report = Instant::now();
            self.since_report = Totals {
                frames: 0,
                sim: Duration::ZERO,
                comp: Duration::ZERO,
                present: Duration::ZERO,
            };
        }

        if self.args.seconds > 0.0 && self.started.elapsed().as_secs_f64() >= self.args.seconds {
            self.finish(el);
        }
    }

    fn finish(&mut self, el: &ActiveEventLoop) {
        let elapsed = self.started.elapsed().as_secs_f64();
        if self.total.frames > 0 {
            self.report("avg", &self.total, elapsed, elapsed);
        }
        let ticks = procstat::process_cpu_ticks()
            .unwrap_or(self.cpu_ticks_start)
            .saturating_sub(self.cpu_ticks_start);
        let cpu_s = ticks as f64 / procstat::USER_HZ;
        let cpu_pct = cpu_s / elapsed.max(1e-9) * 100.0;
        let n = self.total.frames.max(1) as f64;
        println!(
            "RESULT mode=window {}x{} fps_cap={} rain={} frames={} seconds={:.2} fps_avg={:.2} cpu_pct_one_core={:.2} cpu_seconds={:.2} rss_peak_kb={} sim_ms={:.3} composite_ms={:.3} present_ms={:.3}",
            self.args.w,
            self.args.h,
            self.args.fps,
            if self.args.rain { "on" } else { "off" },
            self.total.frames,
            elapsed,
            self.total.frames as f64 / elapsed.max(1e-9),
            cpu_pct,
            cpu_s,
            procstat::rss_peak_kb().unwrap_or(0),
            self.total.sim.as_secs_f64() * 1000.0 / n,
            self.total.comp.as_secs_f64() * 1000.0 / n,
            self.total.present.as_secs_f64() * 1000.0 / n,
        );
        el.exit();
    }

    fn tick(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if now < self.next_frame {
            el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
            return;
        }
        self.render(el);
        if self.frame_dt.is_zero() {
            self.next_frame = Instant::now();
            el.set_control_flow(ControlFlow::Poll);
        } else {
            self.next_frame += self.frame_dt;
            // If we fell behind (or the window was blocked), resync instead of
            // trying to catch up.
            let now = Instant::now();
            if self.next_frame <= now {
                self.next_frame = now + self.frame_dt;
            }
            el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
        }
    }
}

impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("CityLive Iso - M0 spike")
            .with_inner_size(PhysicalSize::new(
                (self.args.w * self.args.scale) as u32,
                (self.args.h * self.args.scale) as u32,
            ))
            .with_resizable(false);
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("failed to create window: {e}");
                el.exit();
                return;
            }
        };
        let context = match softbuffer::Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("softbuffer context: {e}");
                el.exit();
                return;
            }
        };
        let surface = match softbuffer::Surface::new(&context, window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("softbuffer surface: {e}");
                el.exit();
                return;
            }
        };
        self.window = Some(window);
        self._context = Some(context);
        self.surface = Some(surface);
        let now = Instant::now();
        self.started = now;
        self.last_report = now;
        self.next_frame = now;
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::KeyboardInput { ref event, .. }
                if event.state == ElementState::Pressed =>
            {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => el.exit(),
                    PhysicalKey::Code(KeyCode::KeyR) => {
                        self.scene.toggle_rain();
                        println!("  rain {}", if self.scene.rain_on() { "on" } else { "off" });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.tick(el);
    }

    fn exiting(&mut self, el: &ActiveEventLoop) {
        if self.args.seconds <= 0.0 {
            self.finish(el);
        }
    }
}

pub fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    let mut app = App::new(args);
    event_loop.run_app(&mut app)?;
    Ok(())
}
