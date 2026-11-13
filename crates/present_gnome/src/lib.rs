//! GNOME Wayland desktop wallpaper presentation backend for CityLive Iso.
//!
//! Renders an xdg-toplevel window via smithay-client-toolkit (wl_shm) with app-id
//! `org.citylive.Iso` and title `citylive-iso-wallpaper`, sized to the monitor union,
//! designed to be reparented into GNOME Shell's background group by the companion extension.

pub mod procstat;
pub mod slice;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use bytemuck;
use render::{Color, Framebuffer, Scene, SceneConfig, SceneMode};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_registry, delegate_shm, delegate_xdg_shell,
    delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::calloop::EventLoop,
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
    },
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};

pub static TERMINATING: AtomicBool = AtomicBool::new(false);

extern "C" fn sig_handler(_: libc::c_int) {
    TERMINATING.store(true, Ordering::Relaxed);
}

pub fn setup_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sig_handler as *const () as usize;
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
    }
}

pub fn default_save_path() -> PathBuf {
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        if !data_home.trim().is_empty() {
            return PathBuf::from(data_home).join("citylive-iso").join("city.sav");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("citylive-iso")
            .join("city.sav");
    }
    PathBuf::from("city.sav")
}

pub fn layout_save_path(base: &Path, fb_w: usize, fb_h: usize) -> PathBuf {
    let filename = format!(
        "{}.{}x{}",
        base.file_name().unwrap_or_default().to_string_lossy(),
        fb_w,
        fb_h
    );
    base.with_file_name(filename)
}

#[derive(Clone, Debug)]
pub struct WallpaperGnomeConfig {
    pub seed: u64,
    pub mode: SceneMode,
    pub pace: sim::Pace,
    pub speed: f64,
    pub sim_days: f64,
    pub save_path: PathBuf,
    pub fps: u32,
    pub rain: bool,
    pub cars: usize,
    pub peds: usize,
    pub buildings: usize,
    pub seconds: f64,
    pub stats: bool,
}

impl Default for WallpaperGnomeConfig {
    fn default() -> Self {
        Self {
            seed: 7,
            mode: SceneMode::Sim,
            pace: sim::Pace::Month,
            speed: 1.0,
            sim_days: 0.0,
            save_path: default_save_path(),
            fps: 30,
            rain: false,
            cars: render::actors::DEFAULT_CARS,
            peds: render::actors::DEFAULT_PEDS,
            buildings: 150,
            seconds: 0.0,
            stats: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RunStats {
    pub total_ticks: u64,
    pub committed_frames: u64,
    pub elapsed_secs: f64,
    pub avg_fps: f64,
    pub cpu_pct_one_core: f64,
    pub cpu_seconds: f64,
    pub rss_peak_kb: u64,
    pub avg_sim_ms: f64,
    pub avg_comp_ms: f64,
    pub avg_present_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub wl_output: wl_output::WlOutput,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnionLayout {
    pub min_x: i32,
    pub min_y: i32,
    pub union_w: u32,
    pub union_h: u32,
    pub scale: u32,
    pub fb_w: usize,
    pub fb_h: usize,
}

struct FrameTotals {
    frames: u64,
    sim: Duration,
    comp: Duration,
    present: Duration,
}

struct StatsCollector {
    enabled: bool,
    started: Instant,
    last_report: Instant,
    last_report_ticks: u64,
    cpu_ticks_start: u64,
    last_cpu_ticks: u64,
    since_report: FrameTotals,
    total: FrameTotals,
    total_ticks: u64,
}

impl StatsCollector {
    fn new(enabled: bool) -> Self {
        let now = Instant::now();
        let cpu0 = procstat::process_cpu_ticks().unwrap_or(0);
        Self {
            enabled,
            started: now,
            last_report: now,
            last_report_ticks: 0,
            cpu_ticks_start: cpu0,
            last_cpu_ticks: cpu0,
            since_report: FrameTotals {
                frames: 0,
                sim: Duration::ZERO,
                comp: Duration::ZERO,
                present: Duration::ZERO,
            },
            total: FrameTotals {
                frames: 0,
                sim: Duration::ZERO,
                comp: Duration::ZERO,
                present: Duration::ZERO,
            },
            total_ticks: 0,
        }
    }

    fn record_tick(&mut self) {
        self.total_ticks += 1;
    }

    fn record_commit(&mut self, sim: Duration, comp: Duration, present: Duration) {
        self.since_report.frames += 1;
        self.since_report.sim += sim;
        self.since_report.comp += comp;
        self.since_report.present += present;

        self.total.frames += 1;
        self.total.sim += sim;
        self.total.comp += comp;
        self.total.present += present;
    }

    fn maybe_report(&mut self) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        let span = now.duration_since(self.last_report).as_secs_f64();
        if span >= 10.0 {
            let elapsed = now.duration_since(self.started).as_secs_f64();
            let n = self.since_report.frames.max(1) as f64;
            let current_cpu = procstat::process_cpu_ticks().unwrap_or(self.last_cpu_ticks);
            let ticks_delta = current_cpu.saturating_sub(self.last_cpu_ticks);
            let cpu_pct = (ticks_delta as f64 / procstat::USER_HZ) / span.max(1e-9) * 100.0;
            self.last_cpu_ticks = current_cpu;

            let rss = procstat::rss_kb().unwrap_or(0);

            println!(
                "[{:>5.1}s] last frames={} (committed={}) fps={:.1} sim={:.3} ms comp={:.3} ms present={:.3} ms cpu={:.1}% rss={} kB",
                elapsed,
                self.total_ticks - self.last_report_ticks,
                self.since_report.frames,
                self.since_report.frames as f64 / span.max(1e-9),
                self.since_report.sim.as_secs_f64() * 1000.0 / n,
                self.since_report.comp.as_secs_f64() * 1000.0 / n,
                self.since_report.present.as_secs_f64() * 1000.0 / n,
                cpu_pct,
                rss
            );

            self.last_report = now;
            self.last_report_ticks = self.total_ticks;
            self.since_report = FrameTotals {
                frames: 0,
                sim: Duration::ZERO,
                comp: Duration::ZERO,
                present: Duration::ZERO,
            };
        }
    }

    fn finish(&self) -> RunStats {
        let elapsed = self.started.elapsed().as_secs_f64();
        let cpu_now = procstat::process_cpu_ticks().unwrap_or(self.cpu_ticks_start);
        let total_ticks = cpu_now.saturating_sub(self.cpu_ticks_start);
        let cpu_s = total_ticks as f64 / procstat::USER_HZ;
        let cpu_pct = cpu_s / elapsed.max(1e-9) * 100.0;
        let n = self.total.frames.max(1) as f64;

        RunStats {
            total_ticks: self.total_ticks,
            committed_frames: self.total.frames,
            elapsed_secs: elapsed,
            avg_fps: self.total.frames as f64 / elapsed.max(1e-9),
            cpu_pct_one_core: cpu_pct,
            cpu_seconds: cpu_s,
            rss_peak_kb: procstat::rss_peak_kb().unwrap_or(0),
            avg_sim_ms: self.total.sim.as_secs_f64() * 1000.0 / n,
            avg_comp_ms: self.total.comp.as_secs_f64() * 1000.0 / n,
            avg_present_ms: self.total.present.as_secs_f64() * 1000.0 / n,
        }
    }
}

pub struct HeadWindow {
    pub output_name: String,
    pub wl_output: wl_output::WlOutput,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: i32,
    pub window: Window,
    pub buffers: [Buffer; 2],
    pub buf_idx: usize,
    pub configured: bool,
    pub frame_pending: bool,
    pub frame_requested_at: Option<Instant>,
}

pub struct WallpaperGnomeState {
    pub config: WallpaperGnomeConfig,
    pub registry_state: RegistryState,
    pub output_state: OutputState,
    pub compositor_state: CompositorState,
    pub xdg_shell: XdgShell,
    pub xdg_output_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    pub shm: Shm,
    pub slot_pool: SlotPool,
    pub monitors: Vec<Monitor>,
    pub head_windows: Vec<HeadWindow>,
    pub current_union: Option<UnionLayout>,
    pub sim: Option<sim::Sim>,
    pub scene: Scene,
    pub current_fb: Framebuffer,
    pub prev_px: Vec<Color>,
    pub frame_seq: u64,
    pub needs_composite: bool,
    pub initial_frames: u32,
    pub last_sim_step: Instant,
    pub last_save_time: Instant,
    pub qh: QueueHandle<Self>,
    stats_collector: StatsCollector,
}

impl WallpaperGnomeState {
    pub fn new(
        config: WallpaperGnomeConfig,
        registry_state: RegistryState,
        output_state: OutputState,
        compositor_state: CompositorState,
        xdg_shell: XdgShell,
        xdg_output_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
        shm: Shm,
        qh: QueueHandle<Self>,
    ) -> Self {
        let slot_pool =
            SlotPool::new(3840 * 1080 * 4 * 2, &shm).expect("Failed to create SHM slot pool");

        let stats_enabled = config.stats;
        let scene_cfg = SceneConfig {
            width: 960,
            height: 540,
            seed: config.seed,
            mode: config.mode,
            cars: config.cars,
            peds: config.peds,
            buildings: config.buildings,
            rain: config.rain,
        };
        let scene = Scene::new(scene_cfg);
        let current_fb = Framebuffer::new(960, 540);
        let now = Instant::now();

        Self {
            config,
            registry_state,
            output_state,
            compositor_state,
            xdg_shell,
            xdg_output_manager,
            shm,
            slot_pool,
            monitors: Vec::new(),
            head_windows: Vec::new(),
            current_union: None,
            sim: None,
            scene,
            current_fb,
            prev_px: Vec::new(),
            frame_seq: 1,
            needs_composite: true,
            initial_frames: 10,
            last_sim_step: now,
            last_save_time: now,
            qh,
            stats_collector: StatsCollector::new(stats_enabled),
        }
    }

    pub fn register_output(&mut self, output: wl_output::WlOutput) {
        if self.monitors.iter().any(|m| m.wl_output == output) {
            return;
        }

        if let Some(mgr) = &self.xdg_output_manager {
            mgr.get_xdg_output(&output, &self.qh, output.clone());
        }

        let name = self
            .output_state
            .info(&output)
            .and_then(|i| i.name)
            .unwrap_or_else(|| "unknown".to_string());
        let scale_factor = self
            .output_state
            .info(&output)
            .map(|i| i.scale_factor.max(1))
            .unwrap_or(1);
        let logical_pos = self
            .output_state
            .info(&output)
            .and_then(|i| i.logical_position)
            .or_else(|| self.output_state.info(&output).map(|i| i.location))
            .unwrap_or((0, 0));
        let logical_size = self
            .output_state
            .info(&output)
            .and_then(|i| i.logical_size)
            .map(|s| (s.0 as u32, s.1 as u32))
            .unwrap_or((1920, 1080));

        self.monitors.push(Monitor {
            wl_output: output,
            name,
            x: logical_pos.0,
            y: logical_pos.1,
            width: logical_size.0,
            height: logical_size.1,
            scale_factor,
        });
    }

    pub fn check_geometry_change(&mut self) {
        if self.monitors.is_empty() {
            return;
        }

        self.monitors.sort_by_key(|m| (m.x, m.y));

        let min_x = self.monitors.iter().map(|m| m.x).min().unwrap_or(0);
        let min_y = self.monitors.iter().map(|m| m.y).min().unwrap_or(0);
        let max_x = self
            .monitors
            .iter()
            .map(|m| m.x + m.width as i32)
            .max()
            .unwrap_or(1920);
        let max_y = self
            .monitors
            .iter()
            .map(|m| m.y + m.height as i32)
            .max()
            .unwrap_or(1080);
        let tallest_h = self
            .monitors
            .iter()
            .map(|m| m.height as i32)
            .max()
            .unwrap_or(1080);

        let union_w = (max_x - min_x).max(1) as u32;
        let union_h = (max_y - min_y).max(1) as u32;

        let (scale, _, _) = map::pick_scale(union_w as i32, tallest_h);
        let fb_w = (union_w as usize + scale as usize - 1) / scale as usize;
        let fb_h = (union_h as usize + scale as usize - 1) / scale as usize;

        let new_layout = UnionLayout {
            min_x,
            min_y,
            union_w,
            union_h,
            scale,
            fb_w,
            fb_h,
        };

        if self.current_union != Some(new_layout) {
            if let (Some(old_sim), Some(old_layout)) = (&self.sim, self.current_union) {
                let old_path =
                    layout_save_path(&self.config.save_path, old_layout.fb_w, old_layout.fb_h);
                let _ = old_sim.save(&old_path);
            }

            println!(
                "GNOME union: {}x{} at ({},{}), scale {}x -> internal FB {}x{} across {} monitor(s)",
                union_w,
                union_h,
                min_x,
                min_y,
                scale,
                fb_w,
                fb_h,
                self.monitors.len()
            );
            for (idx, m) in self.monitors.iter().enumerate() {
                println!(
                    "  monitor [{}] '{}': pos ({},{}), size {}x{}, scale {}",
                    idx, m.name, m.x, m.y, m.width, m.height, m.scale_factor
                );
            }

            self.current_union = Some(new_layout);
            self.setup_scene_and_sim(fb_w, fb_h);

            // Recreate per-monitor xdg-toplevel windows
            self.head_windows.clear();

            for (idx, m) in self.monitors.iter().enumerate() {
                let surface = self.compositor_state.create_surface(&self.qh);
                let window = self
                    .xdg_shell
                    .create_window(surface, WindowDecorations::None, &self.qh);
                window.set_app_id("org.citylive.Iso");
                let title = format!(
                    "citylive-iso-wallpaper:{}:{}:{}:{}:{}x{}",
                    idx, m.name, m.x, m.y, m.width, m.height
                );
                window.set_title(&title);
                window.set_min_size(Some((m.width, m.height)));
                window.set_max_size(Some((m.width, m.height)));
                window.commit();

                let stride = (m.width as i32 * m.scale_factor) * 4;
                let buf_w = m.width as i32 * m.scale_factor;
                let buf_h = m.height as i32 * m.scale_factor;

                let b0 = self
                    .slot_pool
                    .create_buffer(buf_w, buf_h, stride, wl_shm::Format::Argb8888)
                    .ok()
                    .map(|(b, _)| b);
                let b1 = self
                    .slot_pool
                    .create_buffer(buf_w, buf_h, stride, wl_shm::Format::Argb8888)
                    .ok()
                    .map(|(b, _)| b);

                if let (Some(b0), Some(b1)) = (b0, b1) {
                    println!(
                        "Created xdg-toplevel window [{}] for monitor '{}': title='{}' size={}x{}",
                        idx, m.name, title, m.width, m.height
                    );
                    self.head_windows.push(HeadWindow {
                        output_name: m.name.clone(),
                        wl_output: m.wl_output.clone(),
                        x: m.x,
                        y: m.y,
                        width: m.width,
                        height: m.height,
                        scale_factor: m.scale_factor,
                        window,
                        buffers: [b0, b1],
                        buf_idx: 0,
                        configured: false,
                        frame_pending: false,
                        frame_requested_at: None,
                    });
                } else {
                    eprintln!("Failed to allocate double buffers for monitor '{}'", m.name);
                }
            }
        }
    }

    fn setup_scene_and_sim(&mut self, fb_w: usize, fb_h: usize) {
        let vp = map::world_for_viewport(fb_w as i32, fb_h as i32);
        let (fx, fy) = map::screen_to_tile(&vp, fb_w as f32 * 0.5, fb_h as f32 * 0.5);
        let layout_path = layout_save_path(&self.config.save_path, fb_w, fb_h);

        match self.config.mode {
            SceneMode::Sim => {
                let s = if layout_path.exists() {
                    match sim::Sim::load(&layout_path) {
                        Ok(mut loaded_sim) => {
                            if loaded_sim.map.w == vp.w && loaded_sim.map.h == vp.h {
                                println!(
                                    "Loaded layout-matched city save from {}",
                                    layout_path.display()
                                );
                                if let Ok(meta) = std::fs::metadata(&layout_path) {
                                    if let Ok(mtime) = meta.modified() {
                                        let elapsed_real = SystemTime::now()
                                            .duration_since(mtime)
                                            .unwrap_or(Duration::ZERO)
                                            .as_secs_f64();
                                        if elapsed_real > 0.0 {
                                            let catch_up_sim =
                                                elapsed_real * loaded_sim.pace.scale();
                                            println!(
                                                "Catching up {:.1} sim-days of offline time ({:.1} real seconds)",
                                                catch_up_sim / 86400.0,
                                                elapsed_real
                                            );
                                            loaded_sim.run_to(loaded_sim.time + catch_up_sim);
                                        }
                                    }
                                }
                                if self.config.sim_days > 0.0
                                    && loaded_sim.days() < self.config.sim_days
                                {
                                    loaded_sim.run_to(self.config.sim_days * 86400.0);
                                }
                                loaded_sim.save(&self.config.save_path).ok();
                                loaded_sim
                            } else {
                                println!(
                                    "Layout save map {}x{} differs from viewport {}x{}. Starting fresh for this layout.",
                                    loaded_sim.map.w, loaded_sim.map.h, vp.w, vp.h
                                );
                                let mut new_sim =
                                    sim::Sim::new(self.config.seed, self.config.pace, vp.w, vp.h);
                                new_sim.set_focus(fx, fy);
                                new_sim.found_again();
                                if self.config.sim_days > 0.0 {
                                    new_sim.run_to(self.config.sim_days * 86400.0);
                                }
                                new_sim.save(&layout_path).ok();
                                new_sim.save(&self.config.save_path).ok();
                                new_sim
                            }
                        }
                        Err(_) => {
                            let mut new_sim =
                                sim::Sim::new(self.config.seed, self.config.pace, vp.w, vp.h);
                            new_sim.set_focus(fx, fy);
                            new_sim.found_again();
                            if self.config.sim_days > 0.0 {
                                new_sim.run_to(self.config.sim_days * 86400.0);
                            }
                            new_sim.save(&layout_path).ok();
                            new_sim.save(&self.config.save_path).ok();
                            new_sim
                        }
                    }
                } else if self.config.save_path.exists() {
                    match sim::Sim::load(&self.config.save_path) {
                        Ok(mut loaded_sim) => {
                            if loaded_sim.map.w == vp.w && loaded_sim.map.h == vp.h {
                                println!(
                                    "Loaded base city save matching {}x{} from {}",
                                    fb_w,
                                    fb_h,
                                    self.config.save_path.display()
                                );
                                if let Ok(meta) = std::fs::metadata(&self.config.save_path) {
                                    if let Ok(mtime) = meta.modified() {
                                        let elapsed_real = SystemTime::now()
                                            .duration_since(mtime)
                                            .unwrap_or(Duration::ZERO)
                                            .as_secs_f64();
                                        if elapsed_real > 0.0 {
                                            let catch_up_sim =
                                                elapsed_real * loaded_sim.pace.scale();
                                            loaded_sim.run_to(loaded_sim.time + catch_up_sim);
                                        }
                                    }
                                }
                                if self.config.sim_days > 0.0
                                    && loaded_sim.days() < self.config.sim_days
                                {
                                    loaded_sim.run_to(self.config.sim_days * 86400.0);
                                }
                                loaded_sim.save(&layout_path).ok();
                                loaded_sim
                            } else {
                                let mut new_sim =
                                    sim::Sim::new(self.config.seed, self.config.pace, vp.w, vp.h);
                                new_sim.set_focus(fx, fy);
                                new_sim.found_again();
                                if self.config.sim_days > 0.0 {
                                    new_sim.run_to(self.config.sim_days * 86400.0);
                                }
                                new_sim.save(&layout_path).ok();
                                new_sim.save(&self.config.save_path).ok();
                                new_sim
                            }
                        }
                        Err(_) => {
                            let mut new_sim =
                                sim::Sim::new(self.config.seed, self.config.pace, vp.w, vp.h);
                            new_sim.set_focus(fx, fy);
                            new_sim.found_again();
                            if self.config.sim_days > 0.0 {
                                new_sim.run_to(self.config.sim_days * 86400.0);
                            }
                            new_sim.save(&layout_path).ok();
                            new_sim.save(&self.config.save_path).ok();
                            new_sim
                        }
                    }
                } else {
                    println!(
                        "No existing save found. Starting fresh world for layout {}x{} (map {}x{})",
                        fb_w, fb_h, vp.w, vp.h
                    );
                    let mut new_sim =
                        sim::Sim::new(self.config.seed, self.config.pace, vp.w, vp.h);
                    new_sim.set_focus(fx, fy);
                    new_sim.found_again();
                    if self.config.sim_days > 0.0 {
                        new_sim.run_to(self.config.sim_days * 86400.0);
                    }
                    new_sim.save(&layout_path).ok();
                    new_sim.save(&self.config.save_path).ok();
                    new_sim
                };

                let mut scene = Scene::from_map(
                    s.map.clone(),
                    SceneConfig {
                        width: fb_w,
                        height: fb_h,
                        seed: s.seed,
                        mode: SceneMode::Sim,
                        cars: 0,
                        peds: 0,
                        buildings: 0,
                        rain: self.config.rain,
                    },
                );
                scene.set_city(endfx::kinds::objects(&s));
                scene.flush_dirty();

                self.sim = Some(s);
                self.scene = scene;
            }
            SceneMode::Empty => {
                let mut scene = Scene::new(SceneConfig {
                    width: fb_w,
                    height: fb_h,
                    seed: self.config.seed,
                    mode: SceneMode::Empty,
                    cars: 0,
                    peds: 0,
                    buildings: 0,
                    rain: self.config.rain,
                });
                scene.rebuild_static();
                self.sim = None;
                self.scene = scene;
            }
            SceneMode::Test => {
                let mut scene = Scene::new(SceneConfig {
                    width: fb_w,
                    height: fb_h,
                    seed: self.config.seed,
                    mode: SceneMode::Test,
                    cars: self.config.cars,
                    peds: self.config.peds,
                    buildings: self.config.buildings,
                    rain: self.config.rain,
                });
                scene.rebuild_static();
                self.sim = None;
                self.scene = scene;
            }
        }

        self.current_fb = Framebuffer::new(fb_w, fb_h);
        self.prev_px.clear();
        self.frame_seq += 1;
        self.needs_composite = true;
        self.initial_frames = 10;
    }

    pub fn render_and_present(&mut self) {
        let Some(layout) = self.current_union else {
            return;
        };
        if self.head_windows.is_empty() {
            return;
        }

        let now = Instant::now();
        for head in &mut self.head_windows {
            if head.frame_pending {
                if let Some(t) = head.frame_requested_at {
                    if now.duration_since(t) >= Duration::from_millis(100) {
                        head.frame_pending = false;
                        head.frame_requested_at = None;
                        head.buffers[head.buf_idx].deactivate().ok();
                    }
                }
            }
        }

        let all_configured = self.head_windows.iter().all(|h| h.configured);
        let any_pending = self.head_windows.iter().any(|h| h.frame_pending);

        if !all_configured || any_pending {
            return;
        }

        self.stats_collector.record_tick();

        let t0 = Instant::now();
        if self.scene.has_dirty() {
            self.scene.flush_dirty();
        }
        let t1 = Instant::now();

        self.scene.composite(&mut self.current_fb);
        let t2 = Instant::now();
        self.needs_composite = false;

        let fb_px = self.current_fb.px();
        let changed = self.prev_px.len() != fb_px.len() || self.prev_px != fb_px;
        if changed {
            self.frame_seq += 1;
            if self.prev_px.len() != fb_px.len() {
                self.prev_px.resize(fb_px.len(), 0);
            }
            self.prev_px.copy_from_slice(fb_px);
        }

        let t_pres0 = Instant::now();
        for head in &mut self.head_windows {
            let idx = head.buf_idx;
            let (buf, canvas) = if let Some(c) = head.buffers[idx].canvas(&mut self.slot_pool) {
                head.buf_idx = idx ^ 1;
                (&head.buffers[idx], c)
            } else {
                head.buffers[idx].deactivate().ok();
                if let Some(c) = head.buffers[idx].canvas(&mut self.slot_pool) {
                    head.buf_idx = idx ^ 1;
                    (&head.buffers[idx], c)
                } else if let Some(c) = head.buffers[idx ^ 1].canvas(&mut self.slot_pool) {
                    head.buf_idx = idx;
                    (&head.buffers[idx ^ 1], c)
                } else {
                    continue;
                }
            };

            let dst_px: &mut [u32] = bytemuck::cast_slice_mut(canvas);

            slice::slice_into_output(
                self.current_fb.px(),
                layout.fb_w,
                layout.fb_h,
                layout.scale,
                layout.min_x,
                layout.min_y,
                head.x,
                head.y,
                head.width,
                head.height,
                head.scale_factor,
                dst_px,
            );

            match buf.attach_to(head.window.wl_surface()) {
                Ok(()) => {
                    head.window.wl_surface().damage_buffer(
                        0,
                        0,
                        (head.width as i32) * head.scale_factor,
                        (head.height as i32) * head.scale_factor,
                    );
                    head.window
                        .wl_surface()
                        .frame(&self.qh, head.window.wl_surface().clone());
                    head.frame_pending = true;
                    head.frame_requested_at = Some(Instant::now());
                    head.window.commit();
                }
                Err(e) => {
                    eprintln!("GNOME buffer attach failed for {}: {:?}", head.output_name, e);
                }
            }
        }
        let t_pres1 = Instant::now();

        self.stats_collector.record_commit(
            t1 - t0,
            t2 - t1,
            t_pres1 - t_pres0,
        );
    }
}

impl WindowHandler for WallpaperGnomeState {
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _window: &Window) {
        TERMINATING.store(true, Ordering::Relaxed);
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        window: &Window,
        _configure: WindowConfigure,
        _serial: u32,
    ) {
        for head in &mut self.head_windows {
            if &head.window == window {
                head.configured = true;
                break;
            }
        }
    }
}

impl CompositorHandler for WallpaperGnomeState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        for head in &mut self.head_windows {
            if head.window.wl_surface() == surface {
                head.frame_pending = false;
                head.frame_requested_at = None;
                head.buffers[head.buf_idx].deactivate().ok();
                break;
            }
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for WallpaperGnomeState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.register_output(output);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            if let Some(m) = self.monitors.iter_mut().find(|m| m.wl_output == output) {
                if let Some(name) = info.name {
                    m.name = name;
                }
                if let Some(pos) = info.logical_position {
                    m.x = pos.0;
                    m.y = pos.1;
                } else {
                    m.x = info.location.0;
                    m.y = info.location.1;
                }
                if let Some(size) = info.logical_size {
                    if size.0 > 0 && size.1 > 0 {
                        m.width = size.0 as u32;
                        m.height = size.1 as u32;
                    }
                }
                m.scale_factor = info.scale_factor.max(1);
            }
        }
        if self.xdg_output_manager.is_none() {
            self.check_geometry_change();
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.monitors.retain(|m| m.wl_output != output);
        self.head_windows.retain(|h| h.wl_output != output);
        self.check_geometry_change();
    }
}

impl ShmHandler for WallpaperGnomeState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl Dispatch<zxdg_output_manager_v1::ZxdgOutputManagerV1, ()> for WallpaperGnomeState {
    fn event(
        _state: &mut Self,
        _proxy: &zxdg_output_manager_v1::ZxdgOutputManagerV1,
        _event: zxdg_output_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, wl_output::WlOutput> for WallpaperGnomeState {
    fn event(
        state: &mut Self,
        _proxy: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        output: &wl_output::WlOutput,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let Some(m) = state.monitors.iter_mut().find(|m| &m.wl_output == output) {
            match event {
                zxdg_output_v1::Event::LogicalPosition { x, y } => {
                    m.x = x;
                    m.y = y;
                }
                zxdg_output_v1::Event::LogicalSize { width, height } => {
                    if width > 0 && height > 0 {
                        m.width = width as u32;
                        m.height = height as u32;
                    }
                }
                zxdg_output_v1::Event::Name { name } => {
                    m.name = name;
                }
                zxdg_output_v1::Event::Done => {
                    state.check_geometry_change();
                }
                _ => {}
            }
        }
    }
}

delegate_compositor!(WallpaperGnomeState);
delegate_output!(WallpaperGnomeState);
delegate_shm!(WallpaperGnomeState);
delegate_xdg_shell!(WallpaperGnomeState);
delegate_xdg_window!(WallpaperGnomeState);
delegate_registry!(WallpaperGnomeState);

impl ProvidesRegistryState for WallpaperGnomeState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

pub fn run(config: WallpaperGnomeConfig) -> Result<RunStats, Box<dyn std::error::Error>> {
    setup_signal_handlers();

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let mut event_loop: EventLoop<WallpaperGnomeState> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue).insert(loop_handle)?;

    let compositor_state =
        CompositorState::bind(&globals, &qh).map_err(|e| format!("Compositor bind error: {e}"))?;
    let xdg_shell =
        XdgShell::bind(&globals, &qh).map_err(|e| format!("XdgShell bind error: {e}"))?;
    let xdg_output_manager = globals
        .bind::<zxdg_output_manager_v1::ZxdgOutputManagerV1, _, _>(&qh, 1..=3, ())
        .ok();
    let shm = Shm::bind(&globals, &qh).map_err(|e| format!("Shm bind error: {e}"))?;
    let output_state = OutputState::new(&globals, &qh);
    let registry_state = RegistryState::new(&globals);

    let initial_outputs: Vec<wl_output::WlOutput> = output_state.outputs().collect();

    let mut state = WallpaperGnomeState::new(
        config.clone(),
        registry_state,
        output_state,
        compositor_state,
        xdg_shell,
        xdg_output_manager,
        shm,
        qh,
    );

    for output in initial_outputs {
        state.register_output(output);
    }

    event_loop
        .dispatch(Some(Duration::from_millis(50)), &mut state)
        .ok();
    state.check_geometry_change();

    let frame_interval = if config.fps == 0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(1.0 / config.fps as f64)
    };

    let started = Instant::now();
    let mut next_frame_time = Instant::now();

    println!(
        "GNOME wallpaper starting: scene={:?} seed={} pace={} speed={}x fps_cap={} duration={:.1}s",
        config.mode,
        config.seed,
        config.pace.name(),
        config.speed,
        config.fps,
        config.seconds
    );

    loop {
        if TERMINATING.load(Ordering::Relaxed) {
            break;
        }
        if config.seconds > 0.0 && started.elapsed().as_secs_f64() >= config.seconds {
            break;
        }

        // Periodic 5-minute save
        if state.last_save_time.elapsed() >= Duration::from_secs(300) {
            if let Some(sim) = &state.sim {
                let _ = sim.save(&config.save_path);
                if let Some(layout) = state.current_union {
                    let lp = layout_save_path(&config.save_path, layout.fb_w, layout.fb_h);
                    let _ = sim.save(&lp);
                }
                log::info!("Saved city state to {}", config.save_path.display());
            }
            state.last_save_time = Instant::now();
        }

        // Simulation clock advance
        let now = Instant::now();
        let real_dt = now.duration_since(state.last_sim_step).as_secs_f64();
        state.last_sim_step = now;

        if let Some(sim) = &mut state.sim {
            let sim_dt = real_dt * config.speed;
            sim.advance(sim_dt);
            let changes = sim.take_changes();
            if !changes.is_empty() {
                for (tx, ty) in changes {
                    state.scene.set_tile(tx, ty, sim.map.at(tx, ty));
                }
                state.scene.set_city(endfx::kinds::objects(sim));
                state.needs_composite = true;
            }
        }

        if state.scene.rain_on() || !state.scene.life.is_empty() {
            state.scene.step(real_dt as f32);
            state.needs_composite = true;
        }

        if state.scene.has_dirty() {
            state.needs_composite = true;
        }

        if state.initial_frames > 0 {
            state.initial_frames -= 1;
            state.needs_composite = true;
        }

        let should_render = state.needs_composite && now >= next_frame_time;

        if should_render {
            state.render_and_present();
            conn.flush().ok();

            if config.fps > 0 {
                next_frame_time = now + frame_interval;
            } else {
                next_frame_time = now;
            }
        }

        // Idle-cost sleep
        let timeout = if state.needs_composite {
            next_frame_time.saturating_duration_since(Instant::now())
        } else if config.mode == SceneMode::Sim && state.sim.is_some() {
            let sim = state.sim.as_ref().unwrap();
            let step_offset = sim.time % sim::PLANNING_STEP;
            let remaining_step = (sim::PLANNING_STEP - step_offset).max(0.1);
            let real_secs = remaining_step / (sim.pace.scale() * config.speed.max(0.001));
            Duration::from_secs_f64(real_secs.clamp(0.016, 0.5))
        } else if config.mode == SceneMode::Empty {
            Duration::from_millis(500)
        } else {
            next_frame_time.saturating_duration_since(Instant::now())
        };

        if let Err(e) = event_loop.dispatch(Some(timeout), &mut state) {
            eprintln!("event loop error: {:?}", e);
            break;
        }

        state.stats_collector.maybe_report();
    }

    // Save on exit
    if let Some(sim) = &state.sim {
        let _ = sim.save(&config.save_path);
        if let Some(layout) = state.current_union {
            let lp = layout_save_path(&config.save_path, layout.fb_w, layout.fb_h);
            let _ = sim.save(&lp);
            println!(
                "Saved city state on exit to {} (and {})",
                config.save_path.display(),
                lp.display()
            );
        } else {
            println!("Saved city state on exit to {}", config.save_path.display());
        }
    }

    let stats = state.stats_collector.finish();
    if config.stats {
        println!(
            "RESULT mode=gnome scene={:?} seed={} pace={} speed={}x fps_cap={} rain={} frames={} (committed={}) seconds={:.2} fps_avg={:.2} cpu_pct_one_core={:.2} cpu_seconds={:.2} rss_peak_kb={} sim_ms={:.3} composite_ms={:.3} present_ms={:.3}",
            config.mode,
            config.seed,
            config.pace.name(),
            config.speed,
            config.fps,
            config.rain,
            stats.total_ticks,
            stats.committed_frames,
            stats.elapsed_secs,
            stats.avg_fps,
            stats.cpu_pct_one_core,
            stats.cpu_seconds,
            stats.rss_peak_kb,
            stats.avg_sim_ms,
            stats.avg_comp_ms,
            stats.avg_present_ms
        );
    }

    Ok(stats)
}
