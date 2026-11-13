//! X11 desktop wallpaper presentation backend for CityLive Iso.
//!
//! Spans all monitors on an X11 session (GNOME on Xorg, XFCE, Cinnamon, MATE, Plasma X11),
//! using pure-Rust x11rb (RandR, MIT-SHM, Shape extension click-through, and root pixmap mode).

pub mod procstat;
pub mod slice;

use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use bytemuck;
use render::{Color, Framebuffer, Scene, SceneConfig, SceneMode};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::shm::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, AtomEnum, ConnectionExt as _, PropMode};
use x11rb::wrapper::ConnectionExt as _;

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

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DESKTOP,
        _NET_WM_STATE,
        _NET_WM_STATE_STICKY,
        _NET_WM_STATE_BELOW,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_DESKTOP,
        _XROOTPMAP_ID,
        ESETROOT_PMAP_ID,
        WM_PROTOCOLS,
        WM_DELETE_WINDOW,
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

pub fn set_city_with_kit(scene: &mut render::Scene, s: &sim::Sim) {
    let hour = (s.time / 3600.0).rem_euclid(24.0) as f32;
    let night = !(6.5..19.5).contains(&hour);
    let light = buildkit::light_for([1.0, 1.0, 1.0], if night { 1.0 } else { 0.0 }, hour);
    let objs = endfx::kinds::objects(s);
    let _ = buildkit::set_city(scene, objs, &light);
}

#[derive(Clone, Debug)]
pub struct WallpaperX11Config {
    pub seed: u64,
    pub mode: SceneMode,
    pub root_mode: bool,
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

impl Default for WallpaperX11Config {
    fn default() -> Self {
        Self {
            seed: 7,
            mode: SceneMode::Sim,
            root_mode: false,
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
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnionGeometry {
    pub min_x: i32,
    pub min_y: i32,
    pub union_w: u32,
    pub union_h: u32,
    pub scale: u32,
    pub fb_w: usize,
    pub fb_h: usize,
    pub monitors: Vec<Monitor>,
}

pub fn detect_monitors<C: Connection>(
    conn: &C,
    root: xproto::Window,
    screen: &xproto::Screen,
) -> Result<UnionGeometry, Box<dyn std::error::Error>> {
    let mut monitors = Vec::new();

    let randr_supported = conn
        .extension_information(randr::X11_EXTENSION_NAME)?
        .is_some();
    if randr_supported {
        if let Ok(ver) = conn.randr_query_version(1, 5)?.reply() {
            if (ver.major_version > 1) || (ver.major_version == 1 && ver.minor_version >= 5) {
                if let Ok(reply) = conn.randr_get_monitors(root, true)?.reply() {
                    for m in reply.monitors {
                        let name = conn
                            .get_atom_name(m.name)?
                            .reply()
                            .map(|r| String::from_utf8_lossy(&r.name).to_string())
                            .unwrap_or_else(|_| "monitor".to_string());
                        monitors.push(Monitor {
                            name,
                            x: m.x as i32,
                            y: m.y as i32,
                            width: m.width as u32,
                            height: m.height as u32,
                            primary: m.primary,
                        });
                    }
                }
            }
        }

        if monitors.is_empty() {
            if let Ok(res) = conn.randr_get_screen_resources_current(root)?.reply() {
                for (i, &crtc) in res.crtcs.iter().enumerate() {
                    if let Ok(c) = conn.randr_get_crtc_info(crtc, res.config_timestamp)?.reply() {
                        if c.mode != 0 && c.width > 0 && c.height > 0 {
                            monitors.push(Monitor {
                                name: format!("CRTC-{i}"),
                                x: c.x as i32,
                                y: c.y as i32,
                                width: c.width as u32,
                                height: c.height as u32,
                                primary: i == 0,
                            });
                        }
                    }
                }
            }
        }
    }

    if monitors.is_empty() {
        monitors.push(Monitor {
            name: "Default".to_string(),
            x: 0,
            y: 0,
            width: screen.width_in_pixels as u32,
            height: screen.height_in_pixels as u32,
            primary: true,
        });
    }

    let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
    let max_x = monitors
        .iter()
        .map(|m| m.x + m.width as i32)
        .max()
        .unwrap_or(screen.width_in_pixels as i32);
    let max_y = monitors
        .iter()
        .map(|m| m.y + m.height as i32)
        .max()
        .unwrap_or(screen.height_in_pixels as i32);
    let union_w = (max_x - min_x).max(1) as u32;
    let union_h = (max_y - min_y).max(1) as u32;
    let tallest = monitors
        .iter()
        .map(|m| m.height as i32)
        .max()
        .unwrap_or(screen.height_in_pixels as i32);

    let (scale, _, _) = map::pick_scale(union_w as i32, tallest);
    let fb_w = (union_w as usize + scale as usize - 1) / scale as usize;
    let fb_h = (union_h as usize + scale as usize - 1) / scale as usize;

    Ok(UnionGeometry {
        min_x,
        min_y,
        union_w,
        union_h,
        scale,
        fb_w,
        fb_h,
        monitors,
    })
}

pub struct ShmBuffer {
    pub seg: shm::Seg,
    pub shmid: i32,
    pub ptr: *mut u8,
    pub size: usize,
}

impl ShmBuffer {
    pub fn new<C: Connection>(conn: &C, size: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let seg = conn.generate_id()?;
        unsafe {
            let shmid = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o777);
            if shmid < 0 {
                return Err("libc::shmget failed".into());
            }
            let ptr = libc::shmat(shmid, std::ptr::null(), 0);
            if ptr == (libc::MAP_FAILED as *mut libc::c_void) || ptr.is_null() {
                libc::shmctl(shmid, libc::IPC_RMID, std::ptr::null_mut());
                return Err("libc::shmat failed".into());
            }
            // Mark for deletion upon detach
            libc::shmctl(shmid, libc::IPC_RMID, std::ptr::null_mut());

            conn.shm_attach(seg, shmid as u32, false)?;

            Ok(Self {
                seg,
                shmid,
                ptr: ptr as *mut u8,
                size,
            })
        }
    }
}

impl Drop for ShmBuffer {
    fn drop(&mut self) {
        unsafe {
            libc::shmdt(self.ptr as *const libc::c_void);
        }
    }
}

pub struct Presenter {
    pub shm_available: bool,
    pub shm_buffers: Option<[ShmBuffer; 2]>,
    pub active_shm_idx: usize,
    pub plain_buffer: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub gc: xproto::Gcontext,
    pub depth: u8,
}

impl Presenter {
    pub fn new<C: Connection>(
        conn: &C,
        width: u32,
        height: u32,
        gc: xproto::Gcontext,
        depth: u8,
    ) -> Self {
        let shm_supported = conn
            .extension_information(shm::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_some();
        let bytes_needed = (width as usize) * (height as usize) * 4;

        let mut shm_buffers = None;
        let mut shm_available = false;

        if shm_supported {
            match (
                ShmBuffer::new(conn, bytes_needed),
                ShmBuffer::new(conn, bytes_needed),
            ) {
                (Ok(b0), Ok(b1)) => {
                    shm_buffers = Some([b0, b1]);
                    shm_available = true;
                    log::info!("MIT-SHM shared memory initialized ({} bytes x 2)", bytes_needed);
                }
                (Ok(b0), Err(_)) => {
                    let _ = conn.shm_detach(b0.seg);
                }
                _ => {}
            }
        }

        Self {
            shm_available,
            shm_buffers,
            active_shm_idx: 0,
            plain_buffer: if shm_available {
                Vec::new()
            } else {
                vec![0u32; (width as usize) * (height as usize)]
            },
            width,
            height,
            gc,
            depth,
        }
    }

    pub fn get_draw_slice(&mut self) -> &mut [u32] {
        if self.shm_available {
            let buf = &mut self.shm_buffers.as_mut().unwrap()[self.active_shm_idx];
            unsafe {
                std::slice::from_raw_parts_mut(
                    buf.ptr as *mut u32,
                    (self.width as usize) * (self.height as usize),
                )
            }
        } else {
            &mut self.plain_buffer
        }
    }

    pub fn present<C: Connection>(
        &mut self,
        conn: &C,
        drawable: xproto::Drawable,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.shm_available {
            let seg = self.shm_buffers.as_ref().unwrap()[self.active_shm_idx].seg;
            conn.shm_put_image(
                drawable,
                self.gc,
                self.width as u16,
                self.height as u16,
                0,
                0,
                self.width as u16,
                self.height as u16,
                0,
                0,
                self.depth,
                2, // ZPixmap
                false,
                seg,
                0,
            )?;
            self.active_shm_idx ^= 1;
        } else {
            let max_req = conn.maximum_request_bytes();
            let row_bytes = self.width as usize * 4;
            let rows_per_chunk = (max_req / row_bytes.max(1)).clamp(1, 64);

            let mut y = 0;
            while y < self.height {
                let h = (self.height - y).min(rows_per_chunk as u32);
                let start = (y as usize) * (self.width as usize);
                let end = start + (h as usize) * (self.width as usize);
                let slice = &self.plain_buffer[start..end];
                let byte_slice: &[u8] = bytemuck::cast_slice(slice);

                conn.put_image(
                    xproto::ImageFormat::Z_PIXMAP,
                    drawable,
                    self.gc,
                    self.width as u16,
                    h as u16,
                    0,
                    y as i16,
                    0,
                    self.depth,
                    byte_slice,
                )?;
                y += h;
            }
        }
        Ok(())
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        if let Some(bufs) = self.shm_buffers.take() {
            for b in bufs {
                drop(b);
            }
        }
    }
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

pub fn run(config: WallpaperX11Config) -> Result<RunStats, Box<dyn std::error::Error>> {
    setup_signal_handlers();

    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let depth = screen.root_depth;

    let atoms = Atoms::new(&conn)?.reply()?;

    // Select RandR notifications if RandR exists
    let randr_supported = conn
        .extension_information(randr::X11_EXTENSION_NAME)?
        .is_some();
    if randr_supported {
        let _ = conn.randr_select_input(
            root,
            randr::NotifyMask::SCREEN_CHANGE
                | randr::NotifyMask::CRTC_CHANGE
                | randr::NotifyMask::OUTPUT_CHANGE,
        );
    }

    let mut geom = detect_monitors(&conn, root, screen)?;
    println!(
        "X11 union: {}x{} at ({},{}), scale {}x -> internal FB {}x{} across {} monitor(s)",
        geom.union_w,
        geom.union_h,
        geom.min_x,
        geom.min_y,
        geom.scale,
        geom.fb_w,
        geom.fb_h,
        geom.monitors.len()
    );
    for m in &geom.monitors {
        println!(
            "  monitor '{}': pos ({},{}), size {}x{}, primary={}",
            m.name, m.x, m.y, m.width, m.height, m.primary
        );
    }

    let gc = conn.generate_id()?;
    conn.create_gc(
        gc,
        root,
        &xproto::CreateGCAux::new()
            .graphics_exposures(0),
    )?;

    // Window or Root Pixmap
    let mut window = x11rb::NONE;
    let mut root_pixmap = x11rb::NONE;
    let mut orig_root_pixmap: Option<xproto::Pixmap> = None;

    if config.root_mode {
        // Query existing _XROOTPMAP_ID to restore on exit
        if let Ok(reply) = conn.get_property(
            false,
            root,
            atoms._XROOTPMAP_ID,
            xproto::AtomEnum::PIXMAP,
            0,
            1,
        )?.reply() {
            if reply.value.len() >= 4 {
                orig_root_pixmap = Some(u32::from_ne_bytes(reply.value[0..4].try_into().unwrap()));
            }
        }

        root_pixmap = conn.generate_id()?;
        conn.create_pixmap(
            depth,
            root_pixmap,
            root,
            geom.union_w as u16,
            geom.union_h as u16,
        )?;
        println!("X11 running in --root mode (root pixmap {})", root_pixmap);
    } else {
        window = conn.generate_id()?;
        let win_aux = xproto::CreateWindowAux::new()
            .event_mask(xproto::EventMask::EXPOSURE | xproto::EventMask::STRUCTURE_NOTIFY)
            .background_pixel(screen.black_pixel);

        conn.create_window(
            depth,
            window,
            root,
            geom.min_x as i16,
            geom.min_y as i16,
            geom.union_w as u16,
            geom.union_h as u16,
            0,
            xproto::WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &win_aux,
        )?;

        // EWMH Desktop properties
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_WINDOW_TYPE,
            AtomEnum::ATOM,
            &[atoms._NET_WM_WINDOW_TYPE_DESKTOP],
        )?;

        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_STATE,
            AtomEnum::ATOM,
            &[
                atoms._NET_WM_STATE_STICKY,
                atoms._NET_WM_STATE_BELOW,
                atoms._NET_WM_STATE_SKIP_TASKBAR,
                atoms._NET_WM_STATE_SKIP_PAGER,
            ],
        )?;

        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_DESKTOP,
            AtomEnum::CARDINAL,
            &[0xFFFF_FFFF],
        )?;

        // Set input focus false (WM_HINTS)
        let wm_hints = [1u32, 0, 0, 0, 0, 0, 0, 0, 0];
        conn.change_property32(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_HINTS,
            AtomEnum::WM_HINTS,
            &wm_hints,
        )?;

        // Input shape pass-through
        let shape_supported = conn
            .extension_information(shape::X11_EXTENSION_NAME)?
            .is_some();
        if shape_supported {
            let _ = conn.shape_rectangles(
                shape::SO::SET,
                shape::SK::INPUT,
                xproto::ClipOrdering::UNSORTED,
                window,
                0,
                0,
                &[],
            );
        }

        conn.map_window(window)?;
        conn.configure_window(
            window,
            &xproto::ConfigureWindowAux::new().stack_mode(xproto::StackMode::BELOW),
        )?;
        println!(
            "X11 running in desktop window mode (window ID {}, {}x{} at {},{})",
            window, geom.union_w, geom.union_h, geom.min_x, geom.min_y
        );
    }
    conn.flush()?;

    let mut presenter = Presenter::new(&conn, geom.union_w, geom.union_h, gc, depth);

    // Simulation / Scene setup
    let mut sim: Option<sim::Sim> = None;
    let mut scene: Scene;

    let vp = map::world_for_viewport(geom.fb_w as i32, geom.fb_h as i32);
    let (fx, fy) = map::screen_to_tile(&vp, geom.fb_w as f32 * 0.5, geom.fb_h as f32 * 0.5);
    let layout_path = layout_save_path(&config.save_path, geom.fb_w, geom.fb_h);

    match config.mode {
        SceneMode::Sim => {
            let s = if layout_path.exists() {
                match sim::Sim::load(&layout_path) {
                    Ok(mut loaded) => {
                        if loaded.map.w == vp.w && loaded.map.h == vp.h {
                            println!("Loaded layout-matched city save from {}", layout_path.display());
                            if let Ok(meta) = std::fs::metadata(&layout_path) {
                                if let Ok(mtime) = meta.modified() {
                                    let elapsed_real = SystemTime::now()
                                        .duration_since(mtime)
                                        .unwrap_or(Duration::ZERO)
                                        .as_secs_f64();
                                    if elapsed_real > 0.0 {
                                        let catch_up = elapsed_real * loaded.pace.scale();
                                        println!(
                                            "Catching up {:.1} sim-days of offline time ({:.1} real seconds)",
                                            catch_up / 86400.0,
                                            elapsed_real
                                        );
                                        loaded.run_to(loaded.time + catch_up);
                                    }
                                }
                            }
                            if config.sim_days > 0.0 && loaded.days() < config.sim_days {
                                loaded.run_to(config.sim_days * 86400.0);
                            }
                            loaded.save(&config.save_path).ok();
                            loaded
                        } else {
                            println!(
                                "Layout save map {}x{} differs from viewport {}x{}. Starting fresh.",
                                loaded.map.w, loaded.map.h, vp.w, vp.h
                            );
                            let mut new_sim = sim::Sim::new(config.seed, config.pace, vp.w, vp.h);
                            new_sim.set_focus(fx, fy);
                            new_sim.found_again();
                            if config.sim_days > 0.0 {
                                new_sim.run_to(config.sim_days * 86400.0);
                            }
                            new_sim.save(&layout_path).ok();
                            new_sim.save(&config.save_path).ok();
                            new_sim
                        }
                    }
                    Err(_) => {
                        let mut new_sim = sim::Sim::new(config.seed, config.pace, vp.w, vp.h);
                        new_sim.set_focus(fx, fy);
                        new_sim.found_again();
                        if config.sim_days > 0.0 {
                            new_sim.run_to(config.sim_days * 86400.0);
                        }
                        new_sim.save(&layout_path).ok();
                        new_sim.save(&config.save_path).ok();
                        new_sim
                    }
                }
            } else if config.save_path.exists() {
                match sim::Sim::load(&config.save_path) {
                    Ok(mut loaded) => {
                        if loaded.map.w == vp.w && loaded.map.h == vp.h {
                            println!("Loaded base city save matching {}x{}", geom.fb_w, geom.fb_h);
                            if let Ok(meta) = std::fs::metadata(&config.save_path) {
                                if let Ok(mtime) = meta.modified() {
                                    let elapsed_real = SystemTime::now()
                                        .duration_since(mtime)
                                        .unwrap_or(Duration::ZERO)
                                        .as_secs_f64();
                                    if elapsed_real > 0.0 {
                                        let catch_up = elapsed_real * loaded.pace.scale();
                                        loaded.run_to(loaded.time + catch_up);
                                    }
                                }
                            }
                            if config.sim_days > 0.0 && loaded.days() < config.sim_days {
                                loaded.run_to(config.sim_days * 86400.0);
                            }
                            loaded.save(&layout_path).ok();
                            loaded
                        } else {
                            let mut new_sim = sim::Sim::new(config.seed, config.pace, vp.w, vp.h);
                            new_sim.set_focus(fx, fy);
                            new_sim.found_again();
                            if config.sim_days > 0.0 {
                                new_sim.run_to(config.sim_days * 86400.0);
                            }
                            new_sim.save(&layout_path).ok();
                            new_sim.save(&config.save_path).ok();
                            new_sim
                        }
                    }
                    Err(_) => {
                        let mut new_sim = sim::Sim::new(config.seed, config.pace, vp.w, vp.h);
                        new_sim.set_focus(fx, fy);
                        new_sim.found_again();
                        if config.sim_days > 0.0 {
                            new_sim.run_to(config.sim_days * 86400.0);
                        }
                        new_sim.save(&layout_path).ok();
                        new_sim.save(&config.save_path).ok();
                        new_sim
                    }
                }
            } else {
                let mut new_sim = sim::Sim::new(config.seed, config.pace, vp.w, vp.h);
                new_sim.set_focus(fx, fy);
                new_sim.found_again();
                if config.sim_days > 0.0 {
                    new_sim.run_to(config.sim_days * 86400.0);
                }
                new_sim.save(&layout_path).ok();
                new_sim.save(&config.save_path).ok();
                new_sim
            };

            let mut sc = Scene::from_map(
                s.map.clone(),
                SceneConfig {
                    width: geom.fb_w,
                    height: geom.fb_h,
                    seed: s.seed,
                    mode: SceneMode::Sim,
                    cars: 0,
                    peds: 0,
                    buildings: 0,
                    rain: config.rain,
                },
            );
            set_city_with_kit(&mut sc, &s);
            sc.flush_dirty();

            sim = Some(s);
            scene = sc;
        }
        SceneMode::Empty => {
            let mut sc = Scene::new(SceneConfig {
                width: geom.fb_w,
                height: geom.fb_h,
                seed: config.seed,
                mode: SceneMode::Empty,
                cars: 0,
                peds: 0,
                buildings: 0,
                rain: config.rain,
            });
            sc.rebuild_static();
            scene = sc;
        }
        SceneMode::Test => {
            let mut sc = Scene::new(SceneConfig {
                width: geom.fb_w,
                height: geom.fb_h,
                seed: config.seed,
                mode: SceneMode::Test,
                cars: config.cars,
                peds: config.peds,
                buildings: config.buildings,
                rain: config.rain,
            });
            sc.rebuild_static();
            scene = sc;
        }
    }

    let mut current_fb = Framebuffer::new(geom.fb_w, geom.fb_h);
    let mut prev_fb_px: Vec<Color> = Vec::new();
    let mut needs_composite = true;

    let frame_interval = if config.fps == 0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(1.0 / config.fps as f64)
    };

    let started = Instant::now();
    let mut next_frame_time = Instant::now();
    let mut last_sim_step = Instant::now();
    let mut last_save_time = Instant::now();
    let mut stats_collector = StatsCollector::new(config.stats);

    println!(
        "X11 wallpaper starting: scene={:?} seed={} pace={} speed={}x mode={} fps_cap={} duration={:.1}s",
        config.mode,
        config.seed,
        config.pace.name(),
        config.speed,
        if config.root_mode { "root" } else { "window" },
        config.fps,
        config.seconds
    );

    let raw_fd = conn.stream().as_raw_fd();

    loop {
        if TERMINATING.load(Ordering::Relaxed) {
            break;
        }
        if config.seconds > 0.0 && started.elapsed().as_secs_f64() >= config.seconds {
            break;
        }

        // Process incoming X11 events
        while let Some(event) = conn.poll_for_event()? {
            match event {
                x11rb::protocol::Event::RandrScreenChangeNotify(_)
                | x11rb::protocol::Event::RandrNotify(_) => {
                    log::info!("RandR change detected, re-evaluating monitor layout...");
                    if let Ok(new_geom) = detect_monitors(&conn, root, screen) {
                        if new_geom.union_w != geom.union_w || new_geom.union_h != geom.union_h {
                            println!(
                                "RandR layout update: {}x{} (was {}x{})",
                                new_geom.union_w, new_geom.union_h, geom.union_w, geom.union_h
                            );
                            if let Some(s) = &sim {
                                let old_path = layout_save_path(&config.save_path, geom.fb_w, geom.fb_h);
                                let _ = s.save(&old_path);
                            }
                            geom = new_geom;

                            // Recreate window/pixmap and presenter
                            if config.root_mode {
                                let _ = conn.free_pixmap(root_pixmap);
                                root_pixmap = conn.generate_id()?;
                                conn.create_pixmap(
                                    depth,
                                    root_pixmap,
                                    root,
                                    geom.union_w as u16,
                                    geom.union_h as u16,
                                )?;
                            } else {
                                conn.configure_window(
                                    window,
                                    &xproto::ConfigureWindowAux::new()
                                        .x(geom.min_x)
                                        .y(geom.min_y)
                                        .width(geom.union_w)
                                        .height(geom.union_h),
                                )?;
                            }
                            presenter = Presenter::new(
                                &conn,
                                geom.union_w,
                                geom.union_h,
                                gc,
                                depth,
                            );

                            let new_vp = map::world_for_viewport(geom.fb_w as i32, geom.fb_h as i32);
                            let (nfx, nfy) = map::screen_to_tile(&new_vp, geom.fb_w as f32 * 0.5, geom.fb_h as f32 * 0.5);
                            let mut new_sim = sim::Sim::new(config.seed, config.pace, new_vp.w, new_vp.h);
                            new_sim.set_focus(nfx, nfy);
                            new_sim.found_again();
                            let mut sc = Scene::from_map(
                                new_sim.map.clone(),
                                SceneConfig {
                                    width: geom.fb_w,
                                    height: geom.fb_h,
                                    seed: new_sim.seed,
                                    mode: SceneMode::Sim,
                                    cars: 0,
                                    peds: 0,
                                    buildings: 0,
                                    rain: config.rain,
                                },
                            );
                            set_city_with_kit(&mut sc, &new_sim);
                            sc.flush_dirty();

                            sim = Some(new_sim);
                            scene = sc;
                            current_fb = Framebuffer::new(geom.fb_w, geom.fb_h);
                            prev_fb_px.clear();
                            needs_composite = true;
                        }
                    }
                }
                x11rb::protocol::Event::Expose(_) => {
                    needs_composite = true;
                }
                _ => {}
            }
        }

        // Periodic 5-minute save
        if last_save_time.elapsed() >= Duration::from_secs(300) {
            if let Some(s) = &sim {
                let lp = layout_save_path(&config.save_path, geom.fb_w, geom.fb_h);
                let _ = s.save(&config.save_path);
                let _ = s.save(&lp);
                log::info!("Periodic save written to {}", config.save_path.display());
            }
            last_save_time = Instant::now();
        }

        // Simulation clock advance
        let now = Instant::now();
        let real_dt = now.duration_since(last_sim_step).as_secs_f64();
        last_sim_step = now;

        let t_sim0 = Instant::now();
        if let Some(s) = &mut sim {
            let sim_dt = real_dt * config.speed;
            s.advance(sim_dt);
            let changes = s.take_changes();
            if !changes.is_empty() {
                for (tx, ty) in changes {
                    scene.set_tile(tx, ty, s.map.at(tx, ty));
                }
                set_city_with_kit(&mut scene, s);
                needs_composite = true;
            }
        }

        if scene.rain_on() || !scene.life.is_empty() {
            scene.step(real_dt as f32);
            needs_composite = true;
        }

        if scene.has_dirty() {
            scene.flush_dirty();
            needs_composite = true;
        }
        let t_sim1 = Instant::now();

        // Render and present only when something changed and frame cap allows
        if needs_composite && now >= next_frame_time {
            stats_collector.record_tick();

            let t_comp0 = Instant::now();
            scene.composite(&mut current_fb);
            let t_comp1 = Instant::now();

            let fb_px = current_fb.px();
            let changed = prev_fb_px.len() != fb_px.len() || prev_fb_px != fb_px;
            if changed {
                if prev_fb_px.len() != fb_px.len() {
                    prev_fb_px.resize(fb_px.len(), 0);
                }
                prev_fb_px.copy_from_slice(fb_px);
            }

            let t_pres0 = Instant::now();
            {
                let dst_win = presenter.get_draw_slice();
                for m in &geom.monitors {
                    slice::slice_monitor_into_window(
                        current_fb.px(),
                        geom.fb_w,
                        geom.fb_h,
                        geom.scale,
                        (m.x - geom.min_x) as usize,
                        (m.y - geom.min_y) as usize,
                        m.width as usize,
                        m.height as usize,
                        geom.union_w as usize,
                        dst_win,
                    );
                }
            }

            let active_drawable = if config.root_mode { root_pixmap } else { window };
            presenter.present(&conn, active_drawable)?;

            if config.root_mode {
                conn.change_window_attributes(
                    root,
                    &xproto::ChangeWindowAttributesAux::new().background_pixmap(root_pixmap),
                )?;
                conn.clear_area(false, root, 0, 0, geom.union_w as u16, geom.union_h as u16)?;
                conn.change_property32(
                    PropMode::REPLACE,
                    root,
                    atoms._XROOTPMAP_ID,
                    AtomEnum::PIXMAP,
                    &[root_pixmap],
                )?;
                conn.change_property32(
                    PropMode::REPLACE,
                    root,
                    atoms.ESETROOT_PMAP_ID,
                    AtomEnum::PIXMAP,
                    &[root_pixmap],
                )?;
            }
            conn.flush()?;
            let t_pres1 = Instant::now();

            stats_collector.record_commit(
                t_sim1 - t_sim0,
                t_comp1 - t_comp0,
                t_pres1 - t_pres0,
            );

            needs_composite = false;

            if config.fps > 0 {
                next_frame_time = now + frame_interval;
            } else {
                next_frame_time = now;
            }
        }

        // Sleep timeout calculation for idle suppression
        let timeout_ms = if needs_composite {
            let rem = next_frame_time.saturating_duration_since(Instant::now());
            rem.as_millis().clamp(1, 100) as i32
        } else if config.mode == SceneMode::Sim && sim.is_some() {
            let s = sim.as_ref().unwrap();
            let step_offset = s.time % sim::PLANNING_STEP;
            let remaining_step = (sim::PLANNING_STEP - step_offset).max(0.1);
            let real_secs = remaining_step / (s.pace.scale() * config.speed.max(0.001));
            (real_secs.clamp(0.016, 0.5) * 1000.0) as i32
        } else if config.mode == SceneMode::Empty {
            500
        } else {
            let rem = next_frame_time.saturating_duration_since(Instant::now());
            rem.as_millis().clamp(1, 100) as i32
        };

        // Poll X11 connection fd with timeout
        unsafe {
            let mut pfd = libc::pollfd {
                fd: raw_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            libc::poll(&mut pfd, 1, timeout_ms);
        }

        stats_collector.maybe_report();
    }

    // Save on exit
    if let Some(s) = &sim {
        let lp = layout_save_path(&config.save_path, geom.fb_w, geom.fb_h);
        let _ = s.save(&config.save_path);
        let _ = s.save(&lp);
        println!(
            "Saved city state on exit to {} (and {})",
            config.save_path.display(),
            lp.display()
        );
    }

    // Clean exit restore for root mode
    if config.root_mode {
        if let Some(orig) = orig_root_pixmap {
            let _ = conn.change_window_attributes(
                root,
                &xproto::ChangeWindowAttributesAux::new().background_pixmap(orig),
            );
            let _ = conn.clear_area(false, root, 0, 0, geom.union_w as u16, geom.union_h as u16);
            let _ = conn.change_property32(
                PropMode::REPLACE,
                root,
                atoms._XROOTPMAP_ID,
                AtomEnum::PIXMAP,
                &[orig],
            );
            let _ = conn.change_property32(
                PropMode::REPLACE,
                root,
                atoms.ESETROOT_PMAP_ID,
                AtomEnum::PIXMAP,
                &[orig],
            );
        } else {
            let _ = conn.delete_property(root, atoms._XROOTPMAP_ID);
            let _ = conn.delete_property(root, atoms.ESETROOT_PMAP_ID);
        }
        let _ = conn.free_pixmap(root_pixmap);
        let _ = conn.flush();
    } else if window != x11rb::NONE {
        let _ = conn.destroy_window(window);
        let _ = conn.flush();
    }

    let stats = stats_collector.finish();
    if config.stats {
        println!(
            "RESULT mode=x11 scene={:?} seed={} pace={} speed={}x mode={} fps_cap={} rain={} frames={} (committed={}) seconds={:.2} fps_avg={:.2} cpu_pct_one_core={:.2} cpu_seconds={:.2} rss_peak_kb={} sim_ms={:.3} composite_ms={:.3} present_ms={:.3}",
            config.mode,
            config.seed,
            config.pace.name(),
            config.speed,
            if config.root_mode { "root" } else { "window" },
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
