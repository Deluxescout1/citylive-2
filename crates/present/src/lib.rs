//! Wayland layer-shell presentation backend for CityLive Iso.
//!
//! Spans all monitors on KDE Plasma 6 Wayland desktop background, using
//! smithay-client-toolkit (layer-shell + wl_shm) and nearest-neighbour integer
//! scaling of the internal union framebuffer.

pub mod procstat;
pub mod sampler;
pub mod load;
#[cfg(target_os = "linux")]
pub mod plasma;
#[cfg(target_os = "linux")]
pub mod shm;
pub mod slice;
pub mod plat;
pub mod wincore;
#[cfg(windows)]
pub mod win;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use render::{Framebuffer, Rect, Scene, SceneConfig, SceneMode};
#[cfg(target_os = "linux")]
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::calloop::EventLoop,
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
    },
};
#[cfg(target_os = "linux")]
pub use smithay_client_toolkit::shell::wlr_layer::Layer;
#[cfg(target_os = "linux")]
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, Dispatch, QueueHandle,
};
#[cfg(target_os = "linux")]
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};

/// Where a wallpaper surface sits in the stack. Only the layer-shell backend
/// has a real choice; elsewhere the type exists so the config and the command
/// line stay the same on every platform.
#[cfg(not(target_os = "linux"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Background,
    Bottom,
}

pub static TERMINATING: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "linux")]
extern "C" fn sig_handler(_: libc::c_int) {
    TERMINATING.store(true, Ordering::Relaxed);
}

/// B67: with `CITYLIVE_MOTION_TRACE` set, report any stage that held the loop
/// for more than 5 ms (`STALL <stage> <ms>` on stderr), so a hitch can be traced
/// to the work that caused it.
pub fn stall(stage: &str, since: Instant) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var("CITYLIVE_MOTION_TRACE").is_ok()) {
        let ms = since.elapsed().as_secs_f64() * 1000.0;
        if ms > 5.0 {
            // B61f: the run's own clock beside it, so a period is visible
            // without cross-reading the 10 s windows.
            let t = T0.get_or_init(Instant::now);
            eprintln!("STALL {stage} {ms:.1} (+{:.1}s)", t.elapsed().as_secs_f64());
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn setup_signal_handlers() {
    plat::setup_signal_handlers();
}

#[cfg(target_os = "linux")]
pub fn setup_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sig_handler as *const () as usize;
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
    }
}

/// B42f: how deep the countryside belt under the land line is, in framebuffer
/// pixels (≈160 on screen at 1×, and the framebuffer is the 1× art, so the same
/// number holds at every D61 scale). Buildings, roads and street furniture stop
/// short of it; the tiles stay forest and fields that fade into the haze.
pub const HORIZON_BELT_PX: i32 = 160;

/// Tell a sim the frame the scene will show: the camera and the land line. A
/// loaded city must have this *before* it catches up on the time it was off, or
/// the hours it grows through are built with no belt under the horizon and no
/// headroom under the sky band (the camera and the horizon belong to the viewer,
/// are not saved, and used to be declared only after the catch-up). The numbers
/// are the ones `Scene::from_map_deferred` derives for the same frame.
pub fn declare_frame(sim: &mut sim::Sim, fb_w: usize, fb_h: usize, sky_frac: f32) {
    let sky_h = render::scene::sky_pixels(fb_h, sky_frac);
    let land_h = fb_h.saturating_sub(sky_h).max(64);
    let vp = map::world_for_viewport(fb_w as i32, land_h as i32);
    sim.set_viewport(vp.origin.0, vp.origin.1 + sky_h as i32, fb_w as i32, fb_h as i32);
    sim.set_horizon(sky_h as i32, HORIZON_BELT_PX);
}

/// Helper to convert sim residents into render figures (B43/D65).
pub fn figures(s: &sim::Sim) -> Vec<render::people::Figure> {
    s.people_view()
        .into_iter()
        .map(|f| render::people::Figure {
            x: f.x,
            y: f.y,
            pose: f.pose.to_u8(),
            tone: f.tone,
            child: f.child,
            prop: f.prop.to_u8(),
            dir: f.dir,
            open: f.open,
            lift: f.lift,
            frame: 0,
        })
        .collect()
}

/// B59: the cars a sim is showing parked (bays, driveways, a drive-through
/// queue, a dealership's stock), as traffic bodies the scene draws.
pub fn parked_bodies(s: &sim::Sim) -> Vec<render::traffic::Body> {
    use sim::traffic::VehicleKind;
    s.car_views()
        .into_iter()
        .map(|c| render::traffic::Body {
            x: c.x,
            y: c.y,
            dir: c.dir,
            kind: match c.kind {
                VehicleKind::Cart => render::traffic::Kind::Cart,
                VehicleKind::Car => render::traffic::Kind::Car,
                VehicleKind::Van => render::traffic::Kind::Van,
                VehicleKind::Taxi => render::traffic::Kind::Taxi,
                VehicleKind::Bus => render::traffic::Kind::Bus,
                VehicleKind::Truck => render::traffic::Kind::Truck,
                VehicleKind::Ev => render::traffic::Kind::Ev,
                VehicleKind::Emergency => render::traffic::Kind::Emergency,
                VehicleKind::Bike => render::traffic::Kind::Bike,
            },
            variant: c.variant,
            style: render::traffic::Style::from_u8(c.style),
            // A parked car keeps its lights off except the brake lights of a
            // queue — the light model does the rest (D74).
            lights: false,
            blink: 0.0,
            lift: c.lift,
            brake: c.braking,
            signal: 0,
        })
        .collect()
}

/// B85: the rain's landing profile — for each column, the frame y where a drop
/// first hits an opaque surface (the topmost terrain, roof or treetop). Built
/// from the static layer's depth plane (0 = sky/clear). Columns with nothing
/// opaque fall back to the frame height.
fn ground_profile(depth: &[u16], w: usize, h: usize) -> Vec<i32> {
    let mut g = vec![h as i32; w];
    if depth.len() != w * h {
        return g;
    }
    let mut remaining = w;
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            if g[x] == h as i32 && depth[row + x] != 0 {
                g[x] = y as i32;
                remaining -= 1;
            }
        }
        if remaining == 0 {
            break;
        }
    }
    g
}

/// B85 r2: the water surface per column, from the scene's water-tile screen
/// positions. `-1` = no water; otherwise the frame y of the water surface, so a
/// drop landing near it makes a ripple. Each water tile is a ~64 px diamond, so
/// a tile at screen x marks a band of columns around itself.
fn water_profile(water_px: &[(i32, i32, i32, i32)], w: usize) -> Vec<i32> {
    let mut wat = vec![-1i32; w];
    for &(_, _, sx, sy) in water_px {
        let (x0, x1) = ((sx - 34).max(0), (sx + 34).min(w as i32 - 1));
        for x in x0..=x1 {
            if wat[x as usize] < 0 {
                wat[x as usize] = sy;
            }
        }
    }
    wat
}

/// The buildkit light, quantised to its buckets. B99: day, dusk and night come
/// from the sky's own window glow (the sun's altitude), not from fixed clock
/// hours: a 6:30-19:30 day put dark, unlit buildings in a winter evening (the
/// sun is down at 16:30) and lit windows in a summer afternoon. `glow` is the
/// current palette's `window_glow`; without one the old clock rule is the
/// fallback (first build, tools).
pub fn kit_light(s: &sim::Sim, glow: Option<f32>) -> buildkit::Light {
    let hour = (s.time / 3600.0).rem_euclid(24.0) as f32;
    let glow = glow.unwrap_or(if (6.5..19.5).contains(&hour) { 0.0 } else { 1.0 });
    buildkit::light_for([1.0, 1.0, 1.0], glow, hour)
}

/// Set the city with its buildkit building sprites in one rebuild (B32b/B34).
/// Used when the world is first built.
pub fn set_city_with_kit(scene: &mut render::Scene, s: &sim::Sim) {
    let mut cache = buildkit::KitCache::with_style(buildkit::BiomeStyle::of(s.map.biome));
    let mut objs = endfx::kinds::objects(s);
    cache.attach(&mut objs, &kit_light(s, None));
    // B73r2: the nightlife's gate is the sim's own town (D80).
    scene.set_town_stats(s.population(), s.era().index());
    scene.set_city_with_sprites(cache.into_sprites(), objs);
}

/// B50: a finished world is archived, never deleted: its save (with the final
/// feed and the chronicle's last line inside) goes to
/// `<save dir>/worlds/<NNN>-<city-name>.sav`. B51's chronicle and the world
/// gallery read these.
pub fn archive_world(sim: &sim::Sim, save_path: &Path) -> std::io::Result<PathBuf> {
    let dir = save_path.parent().map(|p| p.to_path_buf()).unwrap_or_default().join("worlds");
    std::fs::create_dir_all(&dir)?;
    let n = std::fs::read_dir(&dir)?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "sav"))
        .count()
        + 1;
    let name: String = sim
        .city_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let path = dir.join(format!("{n:03}-{}.sav", if name.is_empty() { "world" } else { &name }));
    sim.save(&path)?;
    Ok(path)
}

/// B51: the folder the finished worlds go to, beside the live save.
pub fn worlds_dir(save_path: &Path) -> PathBuf {
    save_path.parent().map(|p| p.to_path_buf()).unwrap_or_default().join("worlds")
}

/// B51: where the current world's chronicle is written on demand.
pub fn live_chronicle_path(save_path: &Path) -> PathBuf {
    save_path.parent().map(|p| p.to_path_buf()).unwrap_or_default().join("chronicle.md")
}

/// B51: write the world's chronicle as Markdown to `path`.
pub fn write_chronicle(sim: &sim::Sim, path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, sim.chronicle_markdown())
}

/// B51: a finished world's folder, `worlds/NNN-name/`, beside its archived
/// save `worlds/NNN-name.sav`: the chronicle, the final save, and the timelapse
/// as a GIF if the world recorded one (the raw timelapse file is removed once
/// assembled).
pub fn archive_chronicle(
    sim: &sim::Sim,
    archived_sav: &Path,
    live_save: &Path,
) -> std::io::Result<PathBuf> {
    let dir = archived_sav.with_extension("");
    std::fs::create_dir_all(&dir)?;
    write_chronicle(sim, &dir.join("chronicle.md"))?;
    // B65: the folder is the world, so it carries the world's own save too —
    // the gallery and a person browsing the archive both expect a save inside
    // it. `worlds/NNN-name.sav` stays where B50 put it: nothing that already
    // pointed at it breaks.
    if archived_sav.is_file() {
        if let Some(name) = archived_sav.file_name() {
            std::fs::copy(archived_sav, dir.join(name))?;
        }
    }
    let tl = sim::timelapse::path_for(&world_save_path(live_save, sim.map.w, sim.map.h));
    if tl.exists() {
        match sim::timelapse::assemble_gif(&tl, &dir.join("timelapse.gif")) {
            Ok(n) => println!("timelapse: {n} days assembled"),
            Err(e) => eprintln!("timelapse: cannot assemble ({e})"),
        }
        let _ = std::fs::remove_file(&tl);
    }
    Ok(dir)
}

/// B51: open a file or folder with the desktop's default viewer.
pub fn xdg_open(path: &Path) {
    plat::open_path(path);
}

#[cfg(windows)]
pub fn default_save_path() -> PathBuf {
    plat::data_dir().join("city.sav")
}

#[cfg(not(windows))]
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

pub mod config;
pub mod idle;
pub mod live;
mod spec_wire;
pub mod layout;

/// The save file for a world of `world_w x world_h` tiles (B34).
///
/// Saves used to be keyed by the *framebuffer* size, so changing the zoom/scale
/// threw the city away (the owner's restarted twice). The world size in tiles is the
/// thing that must match — a scale change keeps the same land and the same city.
pub fn world_save_path(base: &Path, world_w: i32, world_h: i32) -> PathBuf {
    let filename = format!(
        "{}.{}x{}",
        base.file_name().unwrap_or_default().to_string_lossy(),
        world_w,
        world_h
    );
    base.with_file_name(filename)
}

/// Find a pre-B34 save keyed by *framebuffer* size whose world matches
/// `world_w x world_h`, so an existing city survives the change. Returns the
/// newest match; the caller renames it to the world-keyed name.
pub fn find_layout_save(base: &Path, world_w: i32, world_h: i32) -> Option<PathBuf> {
    let dir = base.parent()?;
    let name = base.file_name()?.to_string_lossy().to_string();
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p == base {
            continue;
        }
        let fname = p.file_name()?.to_string_lossy().to_string();
        let Some(suffix) = fname.strip_prefix(&name) else {
            continue;
        };
        let Some(dims) = suffix.strip_prefix('.') else {
            continue;
        };
        let mut parts = dims.split('x');
        let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(fw), Ok(fh)) = (a.parse::<i32>(), b.parse::<i32>()) else {
            continue;
        };
        // The name only says which framebuffer wrote it; the world it holds is
        // in the file. (The pre-B34 engine used 32-px tiles, so the old
        // framebuffer size does not map to the same tile rectangle any more.)
        let _ = (fw, fh);
        let matches = match sim::Sim::load(&p) {
            Ok(loaded) => loaded.map.w == world_w && loaded.map.h == world_h,
            Err(_) => false,
        };
        if !matches {
            continue;
        }
        let mt = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().map(|(t, _)| mt > *t).unwrap_or(true) {
            best = Some((mt, p));
        }
    }
    best.map(|(_, p)| p)
}

/// Rename (or copy) a pre-B34 layout-keyed save to the world-keyed name.
pub fn migrate_layout_save(base: &Path, world_w: i32, world_h: i32) -> Option<PathBuf> {
    let target = world_save_path(base, world_w, world_h);
    if target.exists() {
        return Some(target);
    }
    let old = find_layout_save(base, world_w, world_h)?;
    if std::fs::rename(&old, &target).is_err() {
        std::fs::copy(&old, &target).ok()?;
        let _ = std::fs::remove_file(&old);
    }
    println!(
        "Migrated city save {} -> {} (world-keyed since B34)",
        old.display(),
        target.display()
    );
    Some(target)
}

/// Configuration options for the live wallpaper.
#[derive(Clone, Debug)]

pub struct WallpaperConfig {
    pub seed: u64,
    /// D66: force the world's land (`--biome desert`). `None` rolls it from the
    /// seed; a loaded save always keeps the land it was saved with.
    pub biome: Option<map::Biome>,
    /// B50: put the world at a point of its ending, for testing
    /// (`--ending meteor --ending-at spectacle:0.5`). A developer switch: it
    /// is not in the settings file or the tray.
    pub ending_test: Option<(sim::ending::Ending, sim::ending::Phase, f32)>,
    /// B61d: `--endings off` — the world never reaches its ending, so a city can
    /// be grown and measured past the ~a-month life on the same land (a bench
    /// fixture, and the only way to see a day-60 city: a world dies at 21-39
    /// days by design, D62). A developer switch like `ending_test`.
    pub endings_off: bool,
    /// Write each output's frame into this directory for the Plasma wallpaper
    /// plugin to show (D49/B24). `None` = present straight to layer surfaces.
    pub export_frames: Option<PathBuf>,
    /// B42e: export only the 128-row strips a change touches (plus a full base
    /// when the whole frame changed). `false` = the B42d behaviour, one full
    /// frame per change; kept as a switch so the two can be measured and rolled
    /// back on a desktop without a rebuild.
    pub export_strips: bool,
    /// B63: write each output's frame into one mmapped file per head (default
    /// dir `$XDG_RUNTIME_DIR/citylive-iso/shm`). `Some` wins over
    /// `export_frames`: no PPM is written then.
    pub export_shm: Option<PathBuf>,
    pub mode: SceneMode,
    pub pace: sim::Pace,
    pub speed: f64,
    pub sim_days: f64,
    pub save_path: PathBuf,
    /// Animate the water (ripples, waves) at 8 fps (B21). `--water-anim off`.
    pub water_anim: bool,
    /// `--seed` was given: keep it. Otherwise a brand-new world picks its own
    /// seed from the clock, so every first start is a different place (D44).
    pub seed_forced: bool,
    pub layer: Layer,
    pub fps: u32,
    pub rain: bool,
    /// Draw the traffic (B29). `--traffic off` keeps the streets empty.
    pub traffic: bool,
    /// Start the city's clock at this hour (a test knob, B30): a saved city
    /// otherwise starts at the time it was saved.
    pub sim_hour: Option<f64>,
    /// `--weather clear|rain|snow|fog|storm|overcast|live` (B27). `live` (the
    /// default) asks Open-Meteo every 15 minutes; a preset is fixed.
    pub weather: Option<String>,
    pub cars: usize,
    pub peds: usize,
    pub buildings: usize,
    pub seconds: f64,
    pub stats: bool,
    /// Outputs to force into the wallpaper (B17 item 3).
    pub include_outputs: Vec<String>,
    /// Outputs to drop (in addition to `Virtual*` / `HEADLESS*`).
    pub ignore_outputs: Vec<String>,
    /// `--scale N` override (D42's default is derived from the tallest output).
    pub scale_override: Option<u32>,
    /// Observation point for the sky (D16 fallback until `world::geo` lands).
    pub lat: f64,
    pub lon: f64,
    /// `--lat`/`--lon` were given: the place is fixed, nothing is detected (B96).
    pub location_forced: bool,
    /// Fixed clock overrides (D21 testing): "HH:MM" and "YYYY-MM-DD".
    pub time_override: Option<String>,
    /// Sky band as a fraction of the frame height (D43; 0.30 since D53).
    pub sky_frac: f32,
    pub date_override: Option<String>,
    /// The settings file (B37), watched for live reload.
    pub config_path: PathBuf,
    /// Public holidays on or off (D11/D60). Off also stops the fireworks.
    pub holidays: bool,
    /// The tray's action channel (B37): the wallpaper starts the tray and hands
    /// the receiver here.
    pub tray_rx: Option<std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<TrayAction>>>>,
    /// B48a: the tray's picture of the city (name, headcount + era, the latest
    /// story lines). The render loop refreshes it; the tray thread reads it
    /// when the menu opens. `None` runs without a tray.
    pub tray_display: Option<TrayDisplay>,
}

/// B48a: the tray's city display, shared between the render loop (writer) and
/// the tray thread (reader). The lock is held only for the copy-in/copy-out.
pub type TrayDisplay = std::sync::Arc<std::sync::RwLock<sim::story::TrayStatus>>;

/// B49: the technology the crews have installed, in the renderer's form.
fn tech_look(sim: &sim::Sim) -> render::tech::TechLook {
    let t = sim.tech();
    render::tech::TechLook {
        lamps: t.lamps.iter().map(|(&(x, y), &l)| (x, y, l)).collect(),
        lamp_base: t.lamp_baseline(),
        poles: t.poles.iter().copied().collect(),
        buried: t.buried.iter().copied().collect(),
        towers: t.towers.clone(),
        cells: t.cells.iter().copied().collect(),
        solar: t.solar.iter().map(|(&(x, y), &s)| (x, y, s)).collect(),
        farm: t.farm.iter().copied().collect(),
        turbines: t.turbines.clone(),
        chargers: t.chargers.iter().copied().collect(),
        drones: t.drones,
    }
}

/// B49: the delivery drones over the city, five of them, by day.
fn drone_positions(sim: &sim::Sim) -> Vec<(f32, f32)> {
    if !sim.drones_active() {
        return Vec::new();
    }
    let now = sim.smooth_time();
    (0..5)
        .filter_map(|i| sim.drone_at(i, 5, now))
        .collect()
}

/// B41: the sim's street furniture in the renderer's form.
fn street_look(sim: &sim::Sim) -> render::street::StreetLook {
    let sw = sim.streetworks();
    render::street::StreetLook {
        paint: sw
            .paint
            .iter()
            .map(|&(x, y)| (x, y, sim::streetworks::paint_style(sim, x, y)))
            .collect(),
        lamps: sw.lamps.iter().copied().collect(),
        signs: sw.signs.clone(),
        lights: sw.lights.iter().copied().collect(),
    }
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            seed: 7,
            biome: None,
            ending_test: None,
            endings_off: false,
            export_frames: None,
            export_strips: true,
            export_shm: None,
            mode: SceneMode::Sim,
            pace: sim::Pace::Month,
            speed: 1.0,
            sim_days: 0.0,
            save_path: default_save_path(),
            water_anim: true,
            seed_forced: false,
            layer: Layer::Bottom,
            fps: 30,
            rain: false,
            traffic: true,
            sim_hour: None,
            weather: None,
            cars: render::actors::DEFAULT_CARS,
            peds: render::actors::DEFAULT_PEDS,
            buildings: 150,
            seconds: 0.0,
            stats: false,
            include_outputs: Vec::new(),
            ignore_outputs: Vec::new(),
            scale_override: None,
            // Greenwich, London — D16's offline fallback until the IP lookup exists.
            lat: 51.48,
            lon: 0.0,
            location_forced: false,
            time_override: None,
            sky_frac: 0.30,
            date_override: None,
            config_path: config::config_path(),
            holidays: true,
            tray_rx: None,
            tray_display: None,
        }
    }
}

/// The local day and hour (B53): the `date_override` (`YYYY-MM-DD`) when one
/// is set, else the machine's local clock.
pub fn day_parts(date: Option<&str>) -> (i32, u8, u8, u8) {
    let parsed = date.and_then(|s| {
        let p: Vec<i64> = s.split('-').filter_map(|x| x.parse().ok()).collect();
        (p.len() == 3).then(|| (p[0] as i32, p[1] as u8, p[2] as u8))
    });
    let tm = plat::local_time();
    let (y, m, d) = parsed.unwrap_or((tm.year, tm.month, tm.day));
    (y, m, d, tm.hour)
}

/// The local zone's offset from UTC now, in ms (DST included).
fn local_utc_offset_ms() -> i64 {
    plat::local_time().utc_offset_ms
}

/// A cache file in the weather cache's folder (B53).
fn cache_file(name: &str) -> PathBuf {
    plat::cache_dir().join(name)
}

/// The Bills look: blue and red garland, a banner and flags (B53).
fn bills_look(k: render::holiday::Holiday) -> render::holiday::HolidayLook {
    render::holiday::HolidayLook::new(k, &[[0x1f, 0x3c, 0x88], [0xc8, 0x1e, 0x2e]], 0.0)
}

/// A personal date's look (B53 item 2): its style, with bright garlands.
fn personal_look(style: world::DateStyle) -> render::holiday::HolidayLook {
    use render::holiday::{Holiday, HolidayLook};
    match style {
        world::DateStyle::Balloons => HolidayLook::new(
            Holiday::Balloons,
            &[[0xe0, 0x30, 0x60], [0x30, 0x90, 0xe0], [0xf0, 0xc0, 0x30]],
            0.0,
        ),
        world::DateStyle::Banner => {
            HolidayLook::new(Holiday::Banner, &[[0xf0, 0xc0, 0x30], [0x30, 0x90, 0xe0]], 0.0)
        }
        world::DateStyle::Fireworks => {
            HolidayLook::new(Holiday::Fireworks, &[[0xff, 0xd0, 0x60], [0xff, 0x60, 0x90]], 0.0)
        }
        world::DateStyle::Flowers => HolidayLook::new(
            Holiday::Flowers,
            &[[0xff, 0x8a, 0xb0], [0xf5, 0xd0, 0x40], [0xe0, 0x60, 0xa0]],
            0.0,
        ),
    }
}

/// The holiday look for a date/time pair (B36/B37), from `world::calendar`.
pub fn holiday_look(time: Option<&str>, date: Option<&str>) -> render::holiday::HolidayLook {
    use world::calendar;
    let (y, m, d, _) = day_parts(date);
    let hour = time
        .and_then(|t| t.split(':').next().and_then(|h| h.parse::<u8>().ok()))
        .unwrap_or(12);
    let day = world::model::Date::new(y, m, d, hour);
    // B76 r2: the whole dressing (wreaths, garlands, the plaza tree) runs with
    // the light season, 1 December to 6 January, not only the public holiday's
    // own window. `calendar::flag` (the JS parity port) is untouched; this is
    // the wallpaper's dressing rule.
    let in_season = calendar::holiday_lights(m, d) > 0.0;
    let decorated = calendar::decorated(&day).or(if in_season {
        Some(calendar::Holiday::Xmas)
    } else {
        None
    });
    let Some(h) = decorated else {
        return render::holiday::HolidayLook::default();
    };
    let Some(dc) = calendar::decor(h) else {
        return render::holiday::HolidayLook::default();
    };
    let kind = match h {
        calendar::Holiday::Xmas => render::holiday::Holiday::Xmas,
        calendar::Holiday::Halloween => render::holiday::Holiday::Halloween,
        calendar::Holiday::July4 => render::holiday::Holiday::July4,
        calendar::Holiday::Nye => render::holiday::Holiday::Nye,
        calendar::Holiday::Valentine => render::holiday::Holiday::Valentine,
        calendar::Holiday::StPatricks => render::holiday::Holiday::StPatricks,
        calendar::Holiday::Thanks => render::holiday::Holiday::Thanks,
        calendar::Holiday::Easter => render::holiday::Holiday::Easter,
        _ => render::holiday::Holiday::None,
    };
    render::holiday::HolidayLook::new(kind, dc.garland, dc.tint)
}

/// B48a: the story-feed index of a calendar holiday (its slot in
/// [`sim::Sim::HOLIDAYS`]), or `None` for a day that is no holiday.
fn holiday_story_index(h: render::holiday::Holiday) -> Option<usize> {
    Some(match h {
        render::holiday::Holiday::Xmas => 0,
        render::holiday::Holiday::Halloween => 1,
        render::holiday::Holiday::July4 => 2,
        render::holiday::Holiday::Nye => 3,
        render::holiday::Holiday::Valentine => 4,
        render::holiday::Holiday::StPatricks => 5,
        render::holiday::Holiday::Thanks => 6,
        render::holiday::Holiday::Easter => 7,
        _ => return None,
    })
}

/// What the tray menu can ask for (B37).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    /// Stop asking for frames; the sim keeps running (D15).
    TogglePause,
    /// Fast-forward the sim for 60 s, visibly, then back.
    Timelapse,
    /// Show the current weather in the tooltip / a notification.
    WeatherNow,
    /// Open the settings window.
    OpenSettings,
    /// Start a new city, keeping the old save as a backup.
    NewCity,
    /// B51: write the current world's chronicle and open it.
    OpenChronicle,
    /// B51: open the folder of finished worlds.
    OpenWorlds,
    Quit,
}

/// A seed that is different every run: the clock in nanoseconds mixed with the
/// pid, run through one SplitMix64 round (D44, no external crates).
pub fn random_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos ^ ((std::process::id() as u64) << 32) ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    (x ^ (x >> 31)).max(1)
}

/// Aggregated run statistics.
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

/// B62: what the sky is doing, for the spectacle schedules.
#[derive(Clone, Copy, Debug)]
pub struct SpecSky {
    pub now: world::spectacle::SkyNow,
    /// Local hour, 0..24, and the local civil day number.
    pub hour: f32,
    pub day: i64,
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
    /// Where the village is founded, in union-logical coordinates (B17 item 4).
    pub focus: (i32, i32),
}

/// Per-output state tracking display geometry, layer surface, and SHM double buffers.
pub struct Head {
    #[cfg(target_os = "linux")]
    pub wl_output: wl_output::WlOutput,
    pub name: String,
    #[cfg(target_os = "linux")]
    pub layer: LayerSurface,
    /// Windows: this monitor's window and pixel buffer (a window-less
    /// `Surface::headless` while the self-test runs).
    #[cfg(windows)]
    pub surface: Option<win::Surface>,
    pub logical_pos: (i32, i32),
    pub logical_size: (u32, u32),
    pub scale_factor: i32,
    pub configured: bool,
    /// One shm buffer is enough (B26c item 4): the wallpaper only commits when
    /// a pixel changed and `frame_pending` already stops us drawing into a
    /// buffer the compositor still owns. Two buffers per head was 38 MB of RSS
    /// on the owner's three outputs for no gain.
    #[cfg(target_os = "linux")]
    pub buffers: Option<Vec<Buffer>>,
    pub frame_pending: bool,
    pub frame_requested_at: Option<Instant>,
    pub last_drawn_seq: u64,
}

/// Phase timings of one frame, in microseconds (D71 profiling).
#[derive(Default)]
struct Timing {
    on: bool,
    n: u32,
    flush: f64,
    comp: f64,
    /// B61c: the cloud drift tick (the shadows' swept strips).
    clouds: f64,
    /// B61b: the weather overlay (was logged as `wildlife` until B61b — the
    /// numbers in H61 §1.2 were the overlay, not the animals).
    weather: f64,
    met: f64,
    hash: f64,
    present: f64,
    export: f64,
    sim: f64,
    people: f64,
    traffic: f64,
    movers: f64,
    water: f64,
    wildlife_ms: f64,
    sim_step: f64,
    rest: f64,
    /// B101: the main thread's CPU (not wall) time per phase, ms per frame.
    cpu_loop: f64,
    cpu_dispatch: f64,
    cpu_animate: f64,
    cpu_render: f64,
    /// B61f: the same three costs on the frames that composited the whole
    /// frame (a shadow crossfade, a re-tint band, a new sky). A full frame is
    /// the one that can overrun the 33 ms budget, so it gets its own row.
    full_n: u32,
    full_comp: f64,
    full_hash: f64,
    full_export: f64,
}

impl Timing {
    fn hit(&mut self, f: impl Fn(&mut Self) -> &mut f64, d: Duration) {
        if self.on {
            let slot = f(self);
            *slot += d.as_secs_f64() * 1e3;
        }
    }

    fn report(&mut self) {
        if !self.on {
            return;
        }
        self.n += 1;
        if self.n % 60 == 0 {
            render::report_pass_times(self.n as usize);
            let n = self.n as f64;
            eprintln!(
                "timing (mean of {} frames, ms): composite {:.3} flush {:.3} clouds {:.3} weather {:.3} meteor {:.3} hash {:.3} present {:.3} export {:.3} | animate {:.3} (people {:.3} traffic {:.3} movers {:.3} water {:.3} wildlife {:.3}) sim {:.3}",
                self.n,
                self.comp / n,
                self.flush / n,
                self.clouds / n,
                self.weather / n,
                self.met / n,
                self.hash / n,
                self.present / n,
                self.export / n,
                self.sim / n,
                self.people / n,
                self.traffic / n,
                self.movers / n,
                self.water / n,
                self.wildlife_ms / n,
                self.sim_step / n
            );
            eprintln!("timing: rest (loop) {:.3} ms", self.rest / n);
            eprintln!(
                "timing: thread cpu per frame (ms): loop {:.3} = dispatch {:.3} + animate {:.3} + render {:.3}",
                self.cpu_loop / n,
                self.cpu_dispatch / n,
                self.cpu_animate / n,
                self.cpu_render / n
            );
            self.cpu_loop = 0.0;
            self.cpu_dispatch = 0.0;
            self.cpu_animate = 0.0;
            self.cpu_render = 0.0;
            if self.full_n > 0 {
                let f = self.full_n as f64;
                eprintln!(
                    "timing: full frames {} of {} (composite {:.1} hash {:.1} export {:.1} ms)",
                    self.full_n,
                    self.n,
                    self.full_comp / f,
                    self.full_hash / f,
                    self.full_export / f
                );
                self.full_n = 0;
                self.full_comp = 0.0;
                self.full_hash = 0.0;
                self.full_export = 0.0;
            }
            self.n = 0;
            self.flush = 0.0;
            self.comp = 0.0;
            self.weather = 0.0;
            self.clouds = 0.0;
            self.met = 0.0;
            self.hash = 0.0;
            self.export = 0.0;
            self.present = 0.0;
            self.sim = 0.0;
            self.people = 0.0;
            self.traffic = 0.0;
            self.movers = 0.0;
            self.water = 0.0;
            self.wildlife_ms = 0.0;
            self.sim_step = 0.0;
            self.rest = 0.0;
        }
    }
}

/// B101: this thread's CPU time so far, in ms (zero off Linux).
fn thread_cpu_ms() -> f64 {
    #[cfg(target_os = "linux")]
    {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: a plain clock read into a stack timespec.
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
        ts.tv_sec as f64 * 1e3 + ts.tv_nsec as f64 / 1e6
    }
    #[cfg(not(target_os = "linux"))]
    {
        0.0
    }
}

struct FrameTotals {
    frames: u64,
    sim: Duration,
    comp: Duration,
    present: Duration,
    /// B84 r2: damage rectangles composited, the pixels they covered, and the
    /// slowest single rectangle — the three numbers that say whether a slow
    /// frame is area or fragmentation.
    rects: u64,
    area: u64,
    comp_max: Duration,
    /// B84 r3: the geometry of the slowest rectangle, so "worst rect 22 ms" can
    /// be traced to the box that caused it.
    comp_max_rect: render::Rect,
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
    /// B42d: bytes written to the frame directory since the last report.
    since_export: u64,
    total_export: u64,
    /// B101: every committed frame's composite time (ms), for the percentiles
    /// the budget is stated in (a mean hides the frames that miss).
    win_comp: Vec<f32>,
    all_comp: Vec<f32>,
}

/// The `q`-quantile (0..1) of `v` (sorted in place), or 0.
fn quantile(v: &mut [f32], q: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f32 * q).round() as usize]
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
                rects: 0,
                area: 0,
                comp_max: Duration::ZERO,
                comp_max_rect: render::Rect::EMPTY,
            },
            total: FrameTotals {
                frames: 0,
                sim: Duration::ZERO,
                comp: Duration::ZERO,
                present: Duration::ZERO,
                rects: 0,
                area: 0,
                comp_max: Duration::ZERO,
                comp_max_rect: render::Rect::EMPTY,
            },
            total_ticks: 0,
            since_export: 0,
            total_export: 0,
            win_comp: Vec::new(),
            all_comp: Vec::new(),
        }
    }

    /// B101: the composite's p50, p99 and worst frame over the whole run.
    fn comp_percentiles(&self) -> (f32, f32, f32) {
        let mut v = self.all_comp.clone();
        (quantile(&mut v, 0.5), quantile(&mut v, 0.99), quantile(&mut v, 1.0))
    }

    /// B42d: how many bytes the plugin export actually wrote (the number that
    /// used to be 28 MB a frame on the owner's three screens).
    fn record_export(&mut self, bytes: u64) {
        self.since_export += bytes;
        self.total_export += bytes;
    }

    fn record_tick(&mut self) {
        self.total_ticks += 1;
    }

    /// B84 r2: what one frame's damage cost (rectangles, pixels, worst rectangle).
    fn record_damage(&mut self, rects: u64, area: u64, worst: Duration, worst_rect: render::Rect) {
        if worst > self.since_report.comp_max {
            self.since_report.comp_max_rect = worst_rect;
        }
        self.since_report.rects += rects;
        self.since_report.area += area;
        if worst > self.since_report.comp_max {
            self.since_report.comp_max = worst;
        }
        self.total.rects += rects;
        self.total.area += area;
    }

    fn record_commit(&mut self, sim: Duration, comp: Duration, present: Duration) {
        self.since_report.frames += 1;
        self.since_report.sim += sim;
        self.since_report.comp += comp;
        self.since_report.present += present;

        if self.enabled {
            let ms = comp.as_secs_f32() * 1e3;
            self.win_comp.push(ms);
            self.all_comp.push(ms);
        }
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

            let export = if self.total_export > 0 {
                format!(
                    " export={:.1} MB/s",
                    self.since_export as f64 / 1e6 / span.max(1e-9)
                )
            } else {
                String::new()
            };
            println!(
                "[{:>5.1}s] last frames={} (committed={}) fps={:.1} sim={:.3} ms comp={:.3} ms present={:.3} ms cpu={:.1}% rss={} kB{}",
                elapsed,
                self.total_ticks - self.last_report_ticks,
                self.since_report.frames,
                self.since_report.frames as f64 / span.max(1e-9),
                self.since_report.sim.as_secs_f64() * 1000.0 / n,
                self.since_report.comp.as_secs_f64() * 1000.0 / n,
                self.since_report.present.as_secs_f64() * 1000.0 / n,
                cpu_pct,
                rss,
                export
            );
            if !self.win_comp.is_empty() {
                let mut v = std::mem::take(&mut self.win_comp);
                println!(
                    "         composite p50 {:.2} p99 {:.2} max {:.2} ms",
                    quantile(&mut v, 0.5),
                    quantile(&mut v, 0.99),
                    quantile(&mut v, 1.0)
                );
                v.clear();
                self.win_comp = v;
            }
            if self.since_report.rects > 0 {
                let wr = self.since_report.comp_max_rect;
                println!(
                    "         damage {:.1} rects/frame, {:.0} kpx/frame, worst rect {:.2} ms ({}x{} at {},{})",
                    self.since_report.rects as f64 / n,
                    self.since_report.area as f64 / n / 1000.0,
                    self.since_report.comp_max.as_secs_f64() * 1000.0,
                    wr.width(),
                    wr.height(),
                    wr.x0,
                    wr.y0
                );
            }

            self.last_report = now;
            self.since_export = 0;
            self.last_report_ticks = self.total_ticks;
            self.since_report = FrameTotals {
                frames: 0,
                sim: Duration::ZERO,
                comp: Duration::ZERO,
                present: Duration::ZERO,
                rects: 0,
                area: 0,
                comp_max: Duration::ZERO,
                comp_max_rect: render::Rect::EMPTY,
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

pub struct WallpaperState {
    pub config: WallpaperConfig,
    #[cfg(target_os = "linux")]
    pub registry_state: RegistryState,
    #[cfg(target_os = "linux")]
    pub output_state: OutputState,
    #[cfg(target_os = "linux")]
    pub compositor_state: CompositorState,
    #[cfg(target_os = "linux")]
    pub layer_shell: LayerShell,
    #[cfg(target_os = "linux")]
    pub xdg_output_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    #[cfg(target_os = "linux")]
    pub shm: Shm,
    #[cfg(target_os = "linux")]
    pub slot_pool: SlotPool,
    pub heads: Vec<Head>,
    pub current_union: Option<UnionLayout>,
    /// When the set of outputs last changed, and the fingerprint that changed
    /// (B26c item 5): the world is only built once the set has been quiet for
    /// `OUTPUT_SETTLE`, so a half-announced layout never gets a city.
    pub layout_settle: Option<Instant>,
    pub last_specs: Vec<(String, (i32, i32), (u32, u32))>,
    pub sim: Option<sim::Sim>,
    pub scene: Scene,
    /// The building sprite cache, kept across updates (B34): rebuilding it on
    /// every sim change meant a full static rebuild per change — ten a second
    /// while a district is under construction.
    pub kit: buildkit::KitCache,
    pub current_fb: Framebuffer,
    pub traffic_shown: usize,
    /// B31b: movers shown last frame (same idle rule as the traffic).
    pub movers_shown: usize,
    /// The weather the scene is drawn with: the book (memory of wet roads and
    /// lying snow), the overlay particles, the last snapshot and its fetch time.
    pub wx_book: weather_fx::look::WeatherBook,
    pub wx_last: world::Weather,
    pub wx_fx: Option<weather_fx::Fx>,
    /// B85: the per-column landing y for the rain (from the static layer's depth
    /// plane), recomputed when the city changes and handed to the overlay.
    pub wx_ground: Vec<i32>,
    /// B85 r2: the per-column water surface y for the ring ripples.
    pub wx_water: Vec<i32>,
    pub wx_next_tick: Instant,
    pub wx_next_fetch: Instant,
    /// The fetch thread drops the newest snapshot here (B27: 15-minute cadence,
    /// never on the render thread).
    pub wx_shared: std::sync::Arc<std::sync::Mutex<Option<world::Weather>>>,
    /// B96: the location detection. The thread drops its answer in
    /// `loc_shared`; `loc_busy` keeps it to one run at a time.
    pub loc_next: Instant,
    pub loc_shared: std::sync::Arc<std::sync::Mutex<Option<locate::Resolved>>>,
    pub loc_busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// B97: the update check (a minute after start, then every six hours; never
    /// while a full-screen app hides the wallpaper). One run at a time.
    pub upd_next: Instant,
    pub upd_busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// B53: the Bills gameday. The tracker, the latest feed reading and what the
    /// city shows now. The fetch thread drops readings into `sports_shared`.
    pub game_tracker: world::GamedayTracker,
    pub game_now: world::Gameday,
    pub game_latest: Option<world::sports::BillsGame>,
    pub sports_next_fetch: Instant,
    pub sports_shared: std::sync::Arc<std::sync::Mutex<Option<Option<world::sports::BillsGame>>>>,
    /// B53: the personal date last announced in the feed, as (y, m, d).
    pub date_told: Option<(i32, u8, u8)>,
    /// B62: the sky as the spectacle schedules read it (set by `refresh_sky`),
    /// when it last rained, which event starts were told in the feed, the
    /// day's sunrise, and when the spectacle layer was last stepped while idle.
    pub spec_sky: Option<SpecSky>,
    pub spec_last_rain: Option<Instant>,
    pub spec_told: Vec<(u8, i64)>,
    pub spec_sunrise: (i64, f32),
    pub spec_next: Instant,
    /// B61d: `CITYLIVE_SPEC` / `CITYLIVE_SPEC_T` — events forced on for a bench
    /// run (empty = follow the calendar). Read once at startup.
    pub spec_forced: Option<Vec<String>>,
    pub spec_forced_t: f32,
    /// B53 item 3: the idle watch, alive while the idle pacing setting is on.
    pub idle: Option<idle::IdleWatch>,
    /// Where per-output frames are written for the Plasma plugin (D49).
    pub frame_dir: Option<PathBuf>,
    /// B63: where per-output shared-memory frames are written for the Plasma
    /// plugin. One mmapped file per head, published with a seqlock; takes the
    /// place of `frame_dir` when set (no PPM is written then).
    pub shm_dir: Option<PathBuf>,
    /// One writer per head (B63), created on the head's first frame and kept as
    /// long as its internal size stays the same.
    #[cfg(target_os = "linux")]
    pub shm_writers: Vec<(String, shm::ShmWriter)>,
    /// B42d: the scratch copy of one head's frame and the PPM bytes encoded from
    /// it — reused for every head and every frame (28 MB a frame allocated
    /// before).
    pub frame_px: Vec<u32>,
    pub frame_ppm: Vec<u8>,
    /// B61b: the rectangles the weather particles covered last frame, so the
    /// scene can erase them (they are drawn on top of the composite) before the
    /// next pass draws them again.
    pub wx_dirty: Vec<render::Rect>,
    /// Scratch for the particle rectangles of the frame being built — reused, so
    /// a weather frame allocates nothing.
    pub wx_scratch: Vec<render::Rect>,
    /// How many heads the last export wrote, and how many bytes it wrote.
    pub last_export_heads: usize,
    pub last_export_bytes: usize,
    /// Consecutive composites that produced the same pixels as the one before
    /// (B26d): after two, the animation ticks stop asking for frames until
    /// something really changes, so an idle wallpaper sleeps to the next
    /// planning step no matter what re-schedules a tick.
    pub idle_streak: u32,
    /// Hash of the last committed frame (B26c: keeping the whole frame to
    /// compare against cost 10 MB at 3520x720 for the same answer).
    pub prev_hash: u64,
    pub frame_seq: u64,
    pub needs_composite: bool,
    /// Process start, for the wall-clock drift of the clouds and the sky.
    pub started_at: Instant,
    /// When the sky was last redrawn just for the cloud drift.
    pub last_sky_drift: Instant,
    /// D71: when the animated layers were last stepped, and how many steps in
    /// a row changed no pixel (a sleeping city backs off to 4 Hz, then stops
    /// entirely and sleeps to the next planning step).
    pub last_anim: Instant,
    pub anim_quiet: u32,
    /// Per-frame scratch (D71): the sim's figure list and its render form,
    /// reused every frame so smooth animation allocates nothing.
    sim_figs: Vec<sim::people::Figure>,
    /// B77: the week's own vehicles (the bin lorry, the market's vans, the night
    /// bus…), rebuilt each frame into the same buffer.
    sched_vehicles: Vec<sim::traffic::Vehicle>,
    /// B50: what the scene has been told about the ending, the sun's place on
    /// the sky band, the next world to found once this one has ended, and the
    /// config values to put back after founding it.
    end_tr: endfx::Tracker,
    /// B52: what the civic layer has been told.
    civic_tr: endfx::civic::Tracker,
    sun_px: Option<(f32, f32)>,
    end_wx_key: u32,
    last_feed_line: String,
    rebirth: Option<sim::ending::Rebirth>,
    rebirth_restore: Option<(bool, Option<map::Biome>)>,
    figs: Vec<render::people::Figure>,
    /// Did the wildlife layer draw anything last frame? (It is the one layer
    /// whose update is worth skipping when no animal is on screen.)
    wl_live: bool,
    /// D71: is a site being built (crews move), and is the settler party live?
    building: bool,
    settlers_live: bool,
    /// B41b: the wildlife layer (built once per world), its obstacle list and
    /// the 6 fps tick.
    pub wildlife: Option<wildlife::Wildlife>,
    pub wl_objs: Vec<render::CityObj>,
    pub wl_next: Instant,
    /// B42: the pixels the last wildlife draw painted; re-composited from the
    /// scene before the next one.
    pub wl_dirty: Vec<Rect>,
    /// B45: the night-sky projection cache (star/Milky-Way/planet blits,
    /// rebuilt at most once a minute by the sky layer).
    pub night: sky::draw::NightSky,
    /// B45: the sky's current star fade (0 by day — gates the meteor layer).
    pub star_fade: f32,
    /// B45: the pixels the last meteor streak painted on the composite;
    /// restored from the scene before the next pass, like the wildlife's.
    pub met_dirty: Vec<Rect>,
    /// B47: the aurora, the ISS and live flights — the feeds, their gates and
    /// the rectangles they painted last frame.
    pub sky_live: live::LiveSky,
    /// B47: the sky state the sky was last built from (the aurora's gates read
    /// the sun, the cloud and the Moon from it), and the row the visible sky
    /// starts at (the top of the highest thing standing into the band).
    pub live_sky_state: Option<sky::SkyState>,
    pub sky_floor: i32,
    /// The water animation phase in old frames (B21 item 3, D71: fractional).
    pub water_frame: f32,
    /// B101: the load governor (thins water, clouds and far movers when frames
    /// keep missing their deadline) and the time each thinned class has not
    /// yet been stepped for.
    pub load: load::Governor,
    water_dt: f32,
    cloud_dt: f32,
    mover_dt: f32,
    pub last_sim_step: Instant,
    pub last_save_time: Instant,
    /// B37: the settings file's modification time when it was last read, the
    /// parsed settings, and when they were last polled.
    pub settings_mtime: Option<std::time::SystemTime>,
    pub settings: config::Config,
    pub settings_checked: Instant,
    /// B37: the tray's "pause animation" — the sim keeps running (D15), the
    /// frame rate drops to one still a minute.
    pub animation_paused: bool,
    /// B37: a 60-second time-lapse, `Some(ends_at)`.
    pub timelapse_until: Option<Instant>,
    /// B51: the world's day-by-day picture recorder, and the last day tried.
    pub tl_recorder: Option<sim::timelapse::Recorder>,
    pub tl_day: Option<u32>,
    /// B65: the last frame has been taken (the end of the ending's aftermath);
    /// nothing more is recorded for this world.
    pub tl_final: bool,
    /// B37: tray actions arriving from the D-Bus thread.
    pub tray_rx: Option<std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<TrayAction>>>>,
    /// B48a: the tray's city display (the name, the headcount line, the feed).
    pub tray_display: Option<TrayDisplay>,
    /// When the tray's city display was last refreshed.
    pub last_tray_refresh: Instant,
    #[cfg(target_os = "linux")]
    pub qh: QueueHandle<Self>,
    /// Windows: nothing is on screen (a full-screen app or game is in front,
    /// the session is locked or asleep): no composite, no animation, no
    /// present - zero work until it is shown again (B95).
    pub hidden: bool,
    stats_collector: StatsCollector,
    /// D71: phase timings, printed every 60 frames when `CITYLIVE_TIMING=1`.
    /// This is what found the hot path when everything started moving at the
    /// display rate; it stays because the next regression will need it too.
    timing: Timing,
    /// Sky sampling (B17 item 1): about once a minute, and only a re-bake when
    /// the tint actually moved.
    last_sky: Instant,
    /// B67: the cover and storm the sky was last drawn for.
    sky_cloud: f32,
    sky_storm: f32,
    last_tint: [f32; 3],
    last_glow: f32,
    /// B72: the hour and the population the window plan was last built for.
    win_hour: f64,
    win_people: usize,
    /// B99: the buildkit light bucket the city's sprites were last attached for.
    kit_bucket: u8,
    /// B60: the shadow crossfade's clock.
    last_shadow: Instant,
}

impl WallpaperState {
    pub fn new(
        config: WallpaperConfig,
        #[cfg(target_os = "linux")] registry_state: RegistryState,
        #[cfg(target_os = "linux")] output_state: OutputState,
        #[cfg(target_os = "linux")] compositor_state: CompositorState,
        #[cfg(target_os = "linux")] layer_shell: LayerShell,
        #[cfg(target_os = "linux")] xdg_output_manager: Option<
            zxdg_output_manager_v1::ZxdgOutputManagerV1,
        >,
        #[cfg(target_os = "linux")] shm: Shm,
        #[cfg(target_os = "linux")] qh: QueueHandle<Self>,
    ) -> Self {
        #[cfg(target_os = "linux")]
        let slot_pool =
            SlotPool::new(1920 * 1080 * 4 * 4, &shm).expect("Failed to create SHM slot pool");

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
        let frame_dir = config.export_frames.clone();
        // The tray's channel and display come with the config (B37/B48a): the
        // wallpaper starts the tray and hands them here, before the config is
        // moved into the state.
        let tray_rx = config.tray_rx.clone();
        let tray_display = config.tray_display.clone();
        let shm_dir = config.export_shm.clone();

        Self {
            config,
            #[cfg(target_os = "linux")]
            registry_state,
            #[cfg(target_os = "linux")]
            output_state,
            #[cfg(target_os = "linux")]
            compositor_state,
            #[cfg(target_os = "linux")]
            layer_shell,
            #[cfg(target_os = "linux")]
            xdg_output_manager,
            #[cfg(target_os = "linux")]
            shm,
            #[cfg(target_os = "linux")]
            slot_pool,
            heads: Vec::new(),
            current_union: None,
            layout_settle: None,
            last_specs: Vec::new(),
            sim: None,
            scene,
            current_fb,
            kit: buildkit::KitCache::new(),
            traffic_shown: 0,
            movers_shown: 0,
            wx_book: weather_fx::look::WeatherBook::default(),
            wx_last: weather_fx::look::preset("clear").unwrap_or_default(),
            wx_fx: None,
            wx_ground: Vec::new(),
            wx_water: Vec::new(),
            wx_next_tick: Instant::now(),
            wx_next_fetch: Instant::now() + Duration::from_secs(20),
            wx_shared: std::sync::Arc::new(std::sync::Mutex::new(None)),
            loc_next: Instant::now() + Duration::from_secs(3),
            loc_shared: std::sync::Arc::new(std::sync::Mutex::new(None)),
            loc_busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            upd_next: Instant::now() + Duration::from_secs(60),
            upd_busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            game_tracker: world::GamedayTracker::new(),
            spec_sky: None,
            spec_last_rain: None,
            spec_told: Vec::new(),
            spec_sunrise: (i64::MIN, 6.0),
            spec_next: Instant::now(),
            spec_forced: std::env::var("CITYLIVE_SPEC").ok().map(|v| {
                v.split(',')
                    .map(|x| x.trim().to_lowercase())
                    .filter(|x| !x.is_empty())
                    .collect()
            }),
            spec_forced_t: std::env::var("CITYLIVE_SPEC_T")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.5)
                .clamp(0.0, 1.0),
            game_now: world::Gameday::quiet_day(),
            game_latest: None,
            sports_next_fetch: Instant::now() + Duration::from_secs(5),
            sports_shared: std::sync::Arc::new(std::sync::Mutex::new(None)),
            date_told: None,
            idle: None,
            frame_dir,
            shm_dir,
            #[cfg(target_os = "linux")]
            shm_writers: Vec::new(),
            frame_px: Vec::new(),
            frame_ppm: Vec::new(),
            last_export_heads: 0,
            last_export_bytes: 0,
            idle_streak: 0,
            prev_hash: 0,
            frame_seq: 1,
            needs_composite: true,
            started_at: Instant::now(),
            last_sky_drift: Instant::now(),
            last_anim: Instant::now(),
            anim_quiet: 0,
            sim_figs: Vec::new(),
            sched_vehicles: Vec::new(),
            end_tr: endfx::Tracker::default(),
            civic_tr: endfx::civic::Tracker::default(),
            sun_px: None,
            end_wx_key: 0,
            last_feed_line: String::new(),
            rebirth: None,
            rebirth_restore: None,
            figs: Vec::new(),
            wl_live: false,
            building: false,
            settlers_live: false,
            wildlife: None,
            wl_objs: Vec::new(),
            wl_next: Instant::now(),
            wl_dirty: Vec::new(),
            wx_dirty: Vec::new(),
            wx_scratch: Vec::new(),
            night: sky::draw::NightSky::default(),
            star_fade: 0.0,
            met_dirty: Vec::new(),
            sky_live: live::LiveSky::default(),
            live_sky_state: None,
            sky_floor: 0,
            water_frame: 0.0,
            load: load::Governor::default(),
            water_dt: 0.0,
            cloud_dt: 0.0,
            mover_dt: 0.0,
            last_sim_step: now,
            last_save_time: now,
            settings_mtime: None,
            settings: config::Config::default(),
            settings_checked: now,
            animation_paused: false,
            timelapse_until: None,
            tl_recorder: None,
            tl_day: None,
            tl_final: false,
            tray_rx,
            tray_display,
            last_tray_refresh: Instant::now() - Duration::from_secs(60),
            #[cfg(target_os = "linux")]
            qh,
            hidden: false,
            stats_collector: StatsCollector::new(stats_enabled),
            timing: Timing {
                on: {
                    render::pass_time_enable_from_env();
                    std::env::var("CITYLIVE_TIMING").is_ok()
                },
                ..Timing::default()
            },
            last_sky: Instant::now() - Duration::from_secs(3600),
            sky_cloud: 0.0,
            sky_storm: 0.0,
            last_shadow: Instant::now(),
            last_tint: [1.0, 1.0, 1.0],
            last_glow: 0.0,
            win_hour: -99.0,
            win_people: usize::MAX,
            kit_bucket: u8::MAX,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn create_head_for_output(&mut self, output: wl_output::WlOutput) {
        if self.heads.iter().any(|h| h.wl_output == output) {
            return;
        }
        // B17 item 3: a streaming/headless output gets no layer surface at all.
        let known = self
            .output_state
            .info(&output)
            .and_then(|i| i.name)
            .unwrap_or_default();
        if !known.is_empty()
            && !layout::included(&known, &self.config.include_outputs, &self.config.ignore_outputs)
        {
            println!("ignoring output '{known}' (virtual/headless)");
            return;
        }

        let surface = self.compositor_state.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            self.config.layer,
            Some("citylive-iso"),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);

        if let Ok(region) = Region::new(&self.compositor_state) {
            layer.wl_surface().set_input_region(Some(region.wl_region()));
        }

        layer.commit();

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
            .unwrap_or((0, 0));

        self.heads.push(Head {
            wl_output: output,
            name,
            layer,
            logical_pos,
            logical_size,
            scale_factor,
            configured: false,
            buffers: None,
            frame_pending: false,
            frame_requested_at: None,
            last_drawn_seq: 0,
        });
    }

    /// Repaint every head from the scene (Windows: after the heads were rebuilt
    /// or the wallpaper became visible again).
    pub fn repaint_all(&mut self) {
        self.scene.mark_full();
        self.needs_composite = true;
        for h in &mut self.heads {
            h.last_drawn_seq = 0;
        }
    }

    /// How long the output set has to be quiet before a world is built for it.
    pub const OUTPUT_SETTLE: Duration = Duration::from_millis(300);

    /// Called on every output event: records the new set and, if it is still
    /// moving, leaves the world alone (B26c item 5 — the startup used to build a
    /// city.sav for a 2560x720 layout before the third head was announced).
    /// `settle_layout` does the work once the set has stopped changing.
    pub fn check_geometry_change(&mut self) {
        let specs: Vec<(String, (i32, i32), (u32, u32))> = self
            .heads
            .iter()
            .filter(|h| h.configured && h.logical_size.0 > 0 && h.logical_size.1 > 0)
            .map(|h| (h.name.clone(), h.logical_pos, h.logical_size))
            .collect();
        if specs != self.last_specs {
            self.last_specs = specs;
            self.layout_settle = Some(Instant::now() + Self::OUTPUT_SETTLE);
        }
    }

    /// Build the world once the output set has settled.
    pub fn settle_layout(&mut self) -> bool {
        let Some(at) = self.layout_settle else {
            return false;
        };
        if Instant::now() < at {
            return false;
        }
        self.layout_settle = None;
        self.apply_layout();
        true
    }

    /// How long until the set has settled (the main loop sleeps on it).
    pub fn layout_settle_in(&self) -> Option<Duration> {
        self.layout_settle
            .map(|at| at.saturating_duration_since(Instant::now()))
    }

    fn apply_layout(&mut self) {
        let configured_count = self
            .heads
            .iter()
            .filter(|h| h.configured && h.logical_size.0 > 0 && h.logical_size.1 > 0)
            .count();
        if configured_count == 0 {
            return;
        }

        // Only real monitors: the layout maths lives in `layout` so it can be
        // tested against the owner's desk without a compositor (B17 items 3-4).
        let specs: Vec<layout::OutputSpec> = self
            .heads
            .iter()
            .filter(|h| h.configured && h.logical_size.0 > 0 && h.logical_size.1 > 0)
            .map(|h| layout::OutputSpec {
                name: h.name.clone(),
                pos: h.logical_pos,
                size: h.logical_size,
            })
            .collect();
        let Some(l) = layout::layout(
            &specs,
            &self.config.include_outputs,
            &self.config.ignore_outputs,
            self.config.scale_override,
        ) else {
            return;
        };
        let (min_x, min_y, union_w, union_h, scale, fb_w, fb_h) = (
            l.min_x, l.min_y, l.union_w, l.union_h, l.scale, l.fb_w, l.fb_h,
        );

        let new_layout = UnionLayout {
            min_x,
            min_y,
            union_w,
            union_h,
            scale,
            fb_w,
            fb_h,
            focus: l.focus,
        };

        if self.current_union != Some(new_layout) {
            if let (Some(old_sim), Some(_old_layout)) = (&self.sim, self.current_union) {
                let old_path =
                    world_save_path(&self.config.save_path, old_sim.map.w, old_sim.map.h);
                let _ = old_sim.save(&old_path);
            }

            println!(
                "display union: {}x{} at ({},{}), scale {}x -> internal FB {}x{} across {} head(s)",
                union_w,
                union_h,
                min_x,
                min_y,
                scale,
                fb_w,
                fb_h,
                self.heads.len()
            );
            for h in &self.heads {
                println!(
                    "  head '{}': pos {:?}, size {:?}, scale {}",
                    h.name, h.logical_pos, h.logical_size, h.scale_factor
                );
            }
            self.current_union = Some(new_layout);
            self.setup_scene_and_sim(fb_w, fb_h);
        }

        #[cfg(target_os = "linux")]
        for head in &mut self.heads {
            if !head.configured || head.logical_size.0 == 0 || head.logical_size.1 == 0 {
                continue;
            }
            let px_w = head.logical_size.0 * head.scale_factor as u32;
            let px_h = head.logical_size.1 * head.scale_factor as u32;
            let stride = px_w as i32 * 4;

            let needs_alloc = match &head.buffers {
                Some(b) => b.is_empty() || b[0].stride() != stride || b[0].height() != px_h as i32,
                None => true,
            };

            if needs_alloc {
                head.buffers = None;
                let b0 = match self.slot_pool.create_buffer(
                    px_w as i32,
                    px_h as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                ) {
                    Ok((b, _)) => b,
                    Err(e) => {
                        eprintln!(
                            "Failed to allocate shm buffer 0 for head '{}': {:?}",
                            head.name, e
                        );
                        continue;
                    }
                };

                head.buffers = Some(vec![b0]);
                head.layer.wl_surface().set_buffer_scale(head.scale_factor);
                head.last_drawn_seq = 0;
            }
        }
    }

    /// One weather tick: refresh the snapshot (a preset is fixed, `live` comes
    /// from the fetch thread), advance the book and the overlay, and tell the
    /// scene. Returns true when the ground has to be re-rasterised.
    pub fn weather_tick(&mut self, dt: f32) -> bool {
        if let Some(name) = self.config.weather.clone() {
            if name != "live" {
                if let Some(wx) = weather_fx::look::preset(&name) {
                    self.wx_last = wx;
                }
            }
        }
        // B67 (tools): CITYLIVE_WEATHER_SEQ="clear:0,overcast:120,clear:600" plays
        // a script of presets by seconds since start, so a transition can be
        // watched and measured without waiting for a front.
        {
            static SEQ: std::sync::OnceLock<Vec<(f32, String)>> = std::sync::OnceLock::new();
            static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let seq = SEQ.get_or_init(|| {
                std::env::var("CITYLIVE_WEATHER_SEQ")
                    .ok()
                    .map(|v| {
                        v.split(',')
                            .filter_map(|p| {
                                let (n, t) = p.split_once(':')?;
                                Some((t.trim().parse::<f32>().ok()?, n.trim().to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            });
            if !seq.is_empty() {
                let t = T0.get_or_init(Instant::now).elapsed().as_secs_f32();
                if let Some((_, name)) = seq.iter().filter(|(at, _)| *at <= t).last() {
                    if let Some(wx) = weather_fx::look::preset(name) {
                        self.wx_last = wx;
                    }
                }
            }
        }
        // D68: the land wins. Real weather is translated into the world's own
        // idiom before the ground or the overlay sees it (snow -> sandstorm in
        // the desert, a downpour in the jungle, a blizzard on the tundra).
        let wx = self.wx_book_translated();
        let mut look = self.wx_book.update(dt, &wx);
        // B50: the ending has its say (the red sun's heat, smog, the ice age).
        if let Some(sim) = &self.sim {
            endfx::weather(&mut look, sim);
        }
        let changed = self.scene.set_weather(look);
        // B67: the look arrives eased, so the clouds follow it every tick (a few
        // table lookups) instead of waiting for a bucket to turn over.
        self.scene.set_clouds(look.cloud, look.storm);
        // The sky and the light follow the cover in steps too small to see: ask
        // for a sky pass each time it has moved by 0.6 % since the last one.
        let sky_due = (look.cloud - self.sky_cloud).abs() >= 0.006
            || (look.storm - self.sky_storm).abs() >= 0.02;
        // The land's weather also moves the light (dust, cloud, snow), so a
        // change re-tints the static layer with it.
        if changed || sky_due {
            self.sky_cloud = look.cloud;
            self.sky_storm = look.storm;
            let mut ls = self.scene.light_state();
            ls.ambient = look.tint(self.last_tint);
            ls.cloud = look.cloud;
            self.scene.set_light_state(ls);
            // B57: the cover drives the sky's own look (the overcast wash, the
            // greyed palette, the number of clouds), so a weather step asks for
            // a sky pass now rather than at the next minute.
            self.last_sky = Instant::now() - Duration::from_secs(120);
        }
        match &mut self.wx_fx {
            // B67: dt 0 here. The overlay is eased and stepped by the frame loop
            // (`animate`); this 1 Hz call only has to make sure it exists and
            // has seen the latest snapshot.
            Some(fx) => fx.update(0.0, &wx),
            None => {
                self.wx_fx = Some(weather_fx::look::overlay(
                    self.config.seed,
                    self.scene.width(),
                    self.scene.height(),
                ));
                if let Some(fx) = &mut self.wx_fx {
                    if !self.wx_ground.is_empty() {
                        fx.set_ground(Some(self.wx_ground.clone()));
                    }
                    if !self.wx_water.is_empty() {
                        fx.set_water(Some(self.wx_water.clone()));
                    }
                    fx.update(dt, &wx);
                }
            }
        }
        changed
    }

    /// The last weather snapshot in this world's idiom (D68).
    fn wx_book_translated(&self) -> world::Weather {
        weather_fx::look::for_biome(self.scene.biome(), &self.wx_last)
    }

    /// Is an overlay worth drawing this frame? Clear weather costs nothing.
    pub fn weather_overlay(&self) -> Option<&weather_fx::Fx> {
        self.wx_fx.as_ref().filter(|fx| fx.is_active())
    }

    /// B29: the vehicles to draw, converted from the sim (which the renderer
    /// must not depend on).
    pub fn traffic_bodies(&mut self) -> Vec<render::traffic::Body> {
        if self.sim.is_none() {
            return Vec::new();
        }
        // B77: the week's own vehicles — the bin lorry on its round, the
        // market's vans, the school bus, the night bus and the sweeper. Into a
        // buffer of its own, so a frame allocates nothing once it is warm.
        if let Some(s) = &self.sim {
            self.sched_vehicles.clear();
            s.weekly_vehicles_into(&mut self.sched_vehicles);
        }
        let mut out: Vec<render::traffic::Body> = Vec::with_capacity(
            self.sim.as_ref().map(|s| s.vehicles().len()).unwrap_or(0) + self.sched_vehicles.len(),
        );
        if let Some(s) = &self.sim {
            for v in s.vehicles() {
                out.push(self.body_of(v));
            }
        }
        for v in &self.sched_vehicles {
            out.push(self.body_of(v));
        }
        out
    }

    /// One vehicle in the renderer's form (the traffic layer's own mapping).
    fn body_of(&self, v: &sim::traffic::Vehicle) -> render::traffic::Body {
        use sim::traffic::VehicleKind;
        let ((x, y), dir) = v.pose();
        let kind = match v.kind {
            VehicleKind::Cart => render::traffic::Kind::Cart,
            VehicleKind::Car => render::traffic::Kind::Car,
            VehicleKind::Van => render::traffic::Kind::Van,
            VehicleKind::Taxi => render::traffic::Kind::Taxi,
            VehicleKind::Bus => render::traffic::Kind::Bus,
            VehicleKind::Truck => render::traffic::Kind::Truck,
            VehicleKind::Ev => render::traffic::Kind::Ev,
            VehicleKind::Emergency => render::traffic::Kind::Emergency,
            VehicleKind::Bike => render::traffic::Kind::Bike,
        };
        render::traffic::Body {
            x,
            y,
            dir: (dir.0, dir.1),
            kind,
            variant: (v.started as u64 % 8) as u8,
            style: render::traffic::Style::from_u8(v.style),
            lights: self.scene.glow() < 0.5 || v.kind == VehicleKind::Emergency,
            blink: v.blink,
            lift: 0.0,
            brake: v.braking && self.scene.glow() > 0.2,
            signal: v.signal,
        }
    }

    /// B58: the static transport fixtures (rail, station, stops, subway works,
    /// airfield, harbour), from the sim through the shared conversion.
    /// B47: everything the living-sky pass needs about this place and moment,
    /// or `None` before the first sky sample (nothing to draw yet).
    fn live_sky_view(&self, _bounds: Rect) -> Option<live::SkyView> {
        let st = self.live_sky_state?;
        let gates = live::Gates {
            aurora: self.settings.aurora,
            iss: self.settings.iss,
            flights: self.settings.flights,
        };
        if gates.none() {
            return None;
        }
        Some(live::SkyView {
            band: sky::live::Band::with_floor(
                self.current_fb.w,
                self.scene.sky_height() as i32,
                self.sky_floor,
            ),
            sky_ms: self.night.ms_sky(),
            wall_ms: wall_ms_now(),
            lat: self.config.lat,
            lon: self.config.lon,
            aurora: sky::live::AuroraSky {
                kp: self.sky_live.aurora.kp,
                level: self.sky_live.aurora.visible_at(self.config.lat),
                sun_alt: st.sun_alt,
                cloud: st.cloud,
                moon_alt: st.moon_alt,
                moon_illum: st.moon_illum,
            },
            // The weather is in °F; the contrail's lapse wants °C.
            ground_c: (self.wx_last.temp - 32.0) * 5.0 / 9.0,
            has_airfield: self.sim.as_ref().is_some_and(|s| s.rail.airport.is_some()),
            night: st.sun_alt < -2.0,
            gates,
        })
    }

    pub fn sim_fixtures(&self) -> render::rail::Fixtures {
        match &self.sim {
            Some(s) => transitview::fixtures(s),
            None => render::rail::Fixtures::default(),
        }
    }

    /// B58: everything that moves or animates — trains, trams, buses, boats,
    /// ferries, ships, aircraft, planes, crane trolleys, crossing barriers and
    /// the subway's crews.
    pub fn mover_bodies(&self) -> Vec<render::rail::Body> {
        let Some(s) = &self.sim else {
            return Vec::new();
        };
        let hod = (s.time / 3600.0).rem_euclid(24.0);
        let night = !(6.5..19.0).contains(&hod) || self.scene.glow() < 0.5;
        transitview::bodies(s, night)
    }

    /// Write each output's slice as a PPM for the Plasma wallpaper plugin
    /// (D49/B24). Only called when the picture actually changed, and at most
    /// three times a second: the file is written at the **internal** frame
    /// resolution, so the plugin's stretch is exactly our own 2x upscale
    /// (smaller files, cheaper decode, same pixels).
    ///
    /// B42d: two things changed, both measured in H42d.
    /// * **Only the heads that changed are written.** `damage` is this frame's
    ///   changed rectangles in framebuffer space; a head whose slice does not
    ///   touch one keeps the file it already had. A bird on one screen used to
    ///   rewrite all three (28 MB a frame).
    /// * **No per-frame allocation**: one scratch buffer encodes every head.
    #[cfg(target_os = "linux")]
    fn export_frames(&mut self, damage: &[render::Rect], full: bool) {
        let Some(dir) = self.frame_dir.clone() else { return };
        let Some(layout) = self.current_union else {
            return;
        };
        // D71: exported at the frame rate, like everything else that moves. The
        // strip path (B42e) already writes only the rows a change touched, so a
        // busy street costs a few hundred KB a frame, not the whole picture.
        let src = self.current_fb.px();
        let mut written = 0usize;
        let mut bytes = 0usize;
        let strips_on = self.config.export_strips;
        for head in &self.heads {
            if !head.configured || head.logical_size.0 == 0 || head.logical_size.1 == 0 {
                continue;
            }
            let Some(crop) = slice::head_crop(
                layout.fb_w,
                layout.fb_h,
                layout.scale,
                (layout.min_x, layout.min_y),
                head.logical_pos,
                head.logical_size,
            ) else {
                continue;
            };
            let touching: Vec<&render::Rect> = damage
                .iter()
                .filter(|d| slice::rect_hits_head(d, &crop))
                .collect();
            if !full && touching.is_empty() {
                continue; // this head's slice did not change: keep its files
            }
            // B42e: which strips the damage touches. A full frame, or damage
            // spread over more than ~60 % of the head, is cheaper as one base.
            let mut strips: Vec<usize> = Vec::new();
            if strips_on && !full {
                for d in &touching {
                    let row0 = d.y0.max(crop.y as i32) - crop.y as i32;
                    let row1 = d.y1.min((crop.y + crop.h) as i32) - crop.y as i32;
                    for k in slice::strips_for_range(row0, row1, crop.h, plasma::STRIP_ROWS) {
                        if !strips.contains(&k) {
                            strips.push(k);
                        }
                    }
                }
                strips.sort_unstable();
                if strips.len() * plasma::STRIP_ROWS * 10 > crop.h * 6 {
                    strips.clear(); // cheaper as a full frame
                }
            }
            if strips.is_empty() {
                slice::copy_head_frame(src, layout.fb_w, &crop, &mut self.frame_px);
                if let Err(e) = plasma::write_frame_scratch(
                    &dir,
                    &head.name,
                    self.frame_seq,
                    crop.w as u32,
                    crop.h as u32,
                    &self.frame_px,
                    &mut self.frame_ppm,
                ) {
                    eprintln!("could not write the frame for '{}': {e}", head.name);
                } else {
                    written += 1;
                    bytes += crop.w * crop.h * 3;
                }
                continue;
            }
            for k in strips {
                let (row0, rows) = slice::strip_rows(k, crop.h, plasma::STRIP_ROWS);
                if rows == 0 {
                    continue;
                }
                slice::copy_head_rows(src, layout.fb_w, &crop, row0, rows, &mut self.frame_px);
                if let Err(e) = plasma::write_strip_scratch(
                    &dir,
                    &head.name,
                    k,
                    rows,
                    self.frame_seq,
                    crop.w as u32,
                    &self.frame_px,
                    &mut self.frame_ppm,
                ) {
                    eprintln!("could not write strip {k} for '{}': {e}", head.name);
                } else {
                    bytes += crop.w * rows * 3;
                }
            }
            written += 1;
        }
        self.last_export_heads = written;
        self.last_export_bytes = bytes;
        self.stats_collector.record_export(bytes as u64);
        for head in &self.heads {
            let _ = head.scale_factor;
        }
    }

    /// B63: publish every changed head's slice into its shared-memory frame.
    ///
    /// Same rectangle as `export_frames` (the crop `slice_into_output` reads),
    /// but at the frame's **internal** resolution and as raw `u32` pixels: the
    /// screen scales it with the GPU, so there is no CPU upscale, no PPM encode
    /// and no file the reader can lose. Only the rows this frame changed are
    /// copied (the writer remembers what each buffer is missing), so an idle
    /// desktop copies nothing and a busy one copies its damage.
    #[cfg(target_os = "linux")]
    fn export_shm_frames(&mut self, damage: &[render::Rect], full: bool) {
        let Some(dir) = self.shm_dir.clone() else { return };
        let Some(layout) = self.current_union else {
            return;
        };
        let src = self.current_fb.px();
        let fps = self.config.fps;
        let seq = self.frame_seq;
        // B61b: while the weather overlay is moving, a head with no scene damage
        // still has to be published — its drops moved even if its city did not.
        // The rectangles the overlay will paint are the ones the writer saves and
        // restores (a memcpy per rectangle, not a per-pixel log).
        let mut overlay_union: Vec<render::Rect> = Vec::new();
        if let Some(fx) = self.wx_fx.as_ref() {
            if fx.is_active() {
                fx.overlay_rects_fine(&mut overlay_union, shm::STREAK_SLICE_ROWS);
            }
        }
        let overlay_active = !overlay_union.is_empty();
        let mut written = 0usize;
        let mut bytes = 0usize;
        for head in &self.heads {
            if !head.configured || head.logical_size.0 == 0 || head.logical_size.1 == 0 {
                continue;
            }
            let Some(crop) = slice::head_crop(
                layout.fb_w,
                layout.fb_h,
                layout.scale,
                (layout.min_x, layout.min_y),
                head.logical_pos,
                head.logical_size,
            ) else {
                continue;
            };
            // B61b: the part of this frame that changed, as rectangles of
            // *this* head (union frame space → head space, clipped). The writer
            // copies those rectangles, so a weather frame costs its own pixels
            // and not a whole frame's rows; the rows it names for the reader
            // still cover them.
            let mut rects: Vec<render::Rect> = Vec::new();
            if full {
                rects.push(render::Rect::new(0, 0, crop.w as i32, crop.h as i32));
            } else {
                for d in damage {
                    if !slice::rect_hits_head(d, &crop) {
                        continue;
                    }
                    let r = render::Rect::new(
                        d.x0.max(crop.x as i32) - crop.x as i32,
                        d.y0.max(crop.y as i32) - crop.y as i32,
                        d.x1.min((crop.x + crop.w) as i32) - crop.x as i32,
                        d.y1.min((crop.y + crop.h) as i32) - crop.y as i32,
                    );
                    if !r.is_empty() {
                        rects.push(r);
                    }
                }
                // B93: a head whose buffer still carries last frame's drops has
                // to be published once more even when the weather has just
                // stopped (no overlay now, no scene damage): that publish is
                // what puts the scene's pixels back. Otherwise the last drops
                // stay frozen on a quiet head until something else changes.
                let overlay_left = self
                    .shm_writers
                    .iter()
                    .any(|(n, w)| *n == head.name && w.overlay_pending());
                if rects.is_empty() && !overlay_active && !overlay_left {
                    continue; // nothing on this head changed: it keeps its frame
                }
            }
            let (w, h) = (crop.w as u32, crop.h as u32);
            let screen = head.logical_size;
            let mut slot = self.shm_writers.iter().position(|(n, _)| *n == head.name);
            if slot.is_none() {
                let path = dir.join(format!("{}.frame", plasma::sanitise(&head.name)));
                match shm::ShmWriter::open(&path, w, h, layout.scale, screen, fps) {
                    Ok(wr) => {
                        self.shm_writers.push((head.name.clone(), wr));
                        slot = Some(self.shm_writers.len() - 1);
                    }
                    Err(e) => {
                        eprintln!(
                            "could not create the shared frame for '{}': {e}",
                            head.name
                        );
                        continue;
                    }
                }
            }
            let i = match slot {
                Some(i) => i,
                None => continue,
            };
            // A head that changed size (or its scale) needs a new file: the
            // writer recreates it under a temp name and renames it into place.
            if !self.shm_writers[i].1.matches(w, h, layout.scale, screen) {
                let path = self.shm_writers[i].1.path().to_path_buf();
                match shm::ShmWriter::open(&path, w, h, layout.scale, screen, fps) {
                    Ok(wr) => self.shm_writers[i].1 = wr,
                    Err(e) => {
                        eprintln!(
                            "could not resize the shared frame for '{}': {e}",
                            head.name
                        );
                        continue;
                    }
                }
            }
            self.shm_writers[i].1.set_fps(fps);
            // B61b: the weather's moving half (cloud shadows + particles) is
            // painted into this head's buffer over the scene's pixels, with the
            // paint log the writer keeps so the next frame can take it back
            // out. A rainy frame therefore costs the drops, not a full frame.
            let fx = self.wx_fx.as_ref();
            let head_x = crop.x as i32;
            let head_y = crop.y as i32;
            let overlay = |s: &mut shm::PaintSurface| {
                if let Some(fx) = fx {
                    if fx.is_active() {
                        fx.draw_overlay(s, head_x, head_y);
                    }
                }
            };
            // The overlay's rectangles, in this head's space.
            let mut head_overlay: Vec<render::Rect> = Vec::new();
            if overlay_active {
                for d in overlay_union.iter() {
                    let r = render::Rect::new(
                        d.x0.max(crop.x as i32) - crop.x as i32,
                        d.y0.max(crop.y as i32) - crop.y as i32,
                        d.x1.min((crop.x + crop.w) as i32) - crop.x as i32,
                        d.y1.min((crop.y + crop.h) as i32) - crop.y as i32,
                    );
                    if !r.is_empty() {
                        head_overlay.push(r);
                    }
                }
            }
            bytes += self.shm_writers[i].1.publish_with(
                seq,
                src,
                layout.fb_w,
                &crop,
                &rects,
                full,
                &head_overlay,
                overlay,
            );
            written += 1;        }
        self.last_export_heads = written;
        self.last_export_bytes = bytes;
        self.stats_collector.record_export(bytes as u64);
    }

    /// The plugin's frame export: shared memory (B63) when it is configured,
    /// the PPM files of B42e otherwise.
    #[cfg(windows)]
    fn export_plugin_frame(&mut self, _damage: &[render::Rect], _full: bool) {}

    #[cfg(target_os = "linux")]
    fn export_plugin_frame(&mut self, damage: &[render::Rect], full: bool) {
        if self.shm_dir.is_some() {
            self.export_shm_frames(damage, full);
        } else {
            self.export_frames(damage, full);
        }
    }

    /// Sample the sky about once a minute and hand the scene its light. The
    /// tint is only re-baked when it actually moved, so a minute of idle frames
    /// costs a handful of trigs and nothing else.
    fn refresh_sky(&mut self, force: bool) {
        if !force && self.last_sky.elapsed() < Duration::from_secs(55) {
            return;
        }
        self.last_sky = Instant::now();
        // B28: the season (trees, grass, fields) is baked into the static layer.
        // `set_season` dedupes on coarse buckets, so this is a no-op until the
        // season has moved far enough to be worth a re-rasterise.
        {
            let date = self
                .config
                .date_override
                .as_deref()
                .and_then(world::season::parse_date)
                .unwrap_or_else(|| {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    world::season::date_from_ms(now_ms)
                });
            let sl = world::SeasonLook::at(&date, self.config.lat);
            // B77/D84: the schools need the real date for their terms (a
            // Saturday and a Tuesday are different weeks, an August morning is
            // a holiday).
            if let Some(s) = &mut self.sim {
                s.set_calendar_date(date.year, date.month, date.day);
                // B79: the beach follows the calendar, not only the air.
                s.set_sea_season((1.0 - sl.bareness).clamp(0.0, 1.0));
            }
            let look = render::terrain::SeasonLook {
                autumn: sl.autumn,
                bareness: sl.bareness,
                blossom: sl.blossom,
                fresh: sl.fresh,
                litter: sl.litter,
                grass_dry: sl.grass_dry,
                field_stage: sl.field_stage,
                green: sl.green,
                turn: sl.turn,
            };
            // D68: the real date, expressed in this world's land — a jungle in
            // November is green, a tundra is already deep in snow and bare wood.
            let look = render::biome::season_for(self.scene.biome(), look);
            // B80: the machines and the stock keep the season.
            if let Some(w) = &mut self.wildlife {
                let cold = matches!(
                    self.scene.biome(),
                    map::Biome::Temperate | map::Biome::Tundra | map::Biome::Alpine | map::Biome::Highlands | map::Biome::Steppe
                );
                w.set_farm(look.field_stage, cold && look.field_stage <= 0.06);
            }
            if self.scene.set_season(look) {
                self.needs_composite = true;
            }
        }
        let ms = sky::clock_ms(
            self.config.time_override.as_deref(),
            self.config.date_override.as_deref(),
            self.config.lon,
        );
        // B76: frost on a cold, clear dawn (04:00-09:00, near freezing, no
        // rain or snow). The book carries it into the ground's look.
        {
            let hour = sky::local_hour(ms, self.config.lon);
            let f = self.wx_last.flags();
            let wet = f.rain || f.drizzle || f.snow || f.thunder;
            let frost = if (4.0..9.0).contains(&hour) && self.wx_last.temp <= 34.0 && !wet {
                (1.0 - (hour - 4.0) / 6.0).clamp(0.3, 1.0)
            } else {
                0.0
            };
            self.wx_book.set_frost(frost);
        }
        let mut st = sky::state(ms, self.config.lat, self.config.lon);
        // B57: the *real* cloud cover drives the sky, not the fair-weather
        // default `sky::state` carries (this is what the owner saw as "there are
        // almost none anytime I look": the palette was always told 15 %).
        st.cloud = self.scene.weather().cloud.clamp(0.0, 1.0);
        let pal = sky::palette::palette(&st);
        self.sun_px = if st.sun_alt > -0.833 && self.scene.sky_height() > 0 {
            Some(sky::draw::screen_pos_at(
                st.sun_alt,
                st.sun_az,
                self.scene.width(),
                self.scene.sky_height(),
                self.scene.sky_height() as i32,
            ))
        } else {
            None
        };
        // B45: the star fade gates the per-frame meteor layer; it is set on
        // every sample, even when the tint gate below skips the redraw. Same
        // B28 rule the sky layer uses: overcast thins the stars.
        self.star_fade = pal.star_alpha * (1.0 - st.cloud.clamp(0.0, 1.0));
        // B47: the same state, for the aurora's own gates (sun, cloud, Moon),
        // and the band's skyline (one pass over the layer, once a rebuild).
        self.live_sky_state = Some(st);
        let t_skyline = Instant::now();
        self.sky_floor = sky::live::skyline(&self.scene, self.scene.sky_height() as i32);
        stall("skyline", t_skyline);
        // B27/D66: the weather folds into the light — overcast dims it, snow
        // lifts it, and a desert sandstorm warms and dims it. The bare hour tint
        // is kept, so a weather change can be re-applied without the sky pass.
        let tint = pal.ambient_tint;
        let lit = self.scene.weather().tint(tint);
        // B60/D74: one light model per instant — the sun and the moon from the
        // astronomy, the sky's ambient tint and window glow from the palette,
        // and the cloud cover that softens the shadows. Everything lit reads it.
        let wx = self.scene.weather();
        let mut ls = render::light::LightState::from_sky(
            st.sun_alt,
            st.sun_az,
            st.moon_alt,
            st.moon_az,
            st.moon_illum,
        );
        ls.ambient = lit;
        ls.glow = pal.window_glow;
        ls.cloud = wx.cloud;
        ls.dir_strength = pal.light_dir_strength;
        // B58b item 7: which windows are lit depends on the hour, not only on
        // how dark the sky is.
        ls.hour = sky::local_hour(ms, self.config.lon);
        let t_ls = Instant::now();
        self.scene.set_light_state(ls);
        stall("set_light_state", t_ls);
        // B72 (D78): the windows follow the people — recompute the plan when the
        // hour has moved a minute (or the city's population changed) and hand it
        // over: the scene repaints only the panes that actually switch.
        self.push_window_plan(ls.hour as f64);
        // B62: the same sky, as the spectacle schedules want it.
        {
            if wx.rain > 0.08 || wx.snow > 0.08 {
                self.spec_last_rain = Some(Instant::now());
            }
            let since = self.spec_last_rain.map_or(9999.0, |t| t.elapsed().as_secs_f32() / 60.0);
            let local = ms as f64 / 1000.0 + self.config.lon / 15.0 * 3600.0;
            self.spec_sky = Some(SpecSky {
                now: world::spectacle::SkyNow {
                    sun_alt: st.sun_alt as f32,
                    sun_az: st.sun_az as f32,
                    cloud: wx.cloud,
                    wind_kmh: wx.wind_kmh,
                    wind_from: wx.wind_dir,
                    precip: wx.rain > 0.05 || wx.snow > 0.05,
                    storm: wx.storm > 0.1,
                    since_rain_min: since,
                    lunar_eclipse: false,
                },
                hour: ls.hour,
                day: (local / 86400.0).floor() as i64,
            });
        }
        // B60: the animals throw theirs from the same light.
        if let Some(w) = &mut self.wildlife {
            w.set_light(&ls);
            w.set_land_line(self.scene.sky_height() as i32);
        }
        // B57/D72: the clouds — cover from the real weather (in this land's own
        // idiom), drift from the real wind, colour from this hour's light. They
        // are drawn per frame, so this pass only has to keep the *field* in step;
        // it is a no-op until the weather moves into a new band.
        self.sync_clouds(&st, &pal, &ls, wx.storm, wx.wind_dir, wx.wind_kmh);
        if !force
            && (0..3).all(|i| (tint[i] - self.last_tint[i]).abs() < 0.004)
            && (pal.window_glow - self.last_glow).abs() < 0.04
        {
            return;
        }
        self.last_tint = tint;
        self.last_glow = pal.window_glow;
        // B99: the buildings' sprites are baked for a light bucket (day, dusk,
        // night); when the sky crosses into another one they are re-attached,
        // instead of waiting for the next sim change.
        if let Some(sim) = &self.sim {
            let bucket = kit_light(sim, Some(pal.window_glow)).bucket;
            if bucket != self.kit_bucket {
                self.update_city();
            }
        }
        if self.scene.sky_height() == 0 {
            return;
        }
        let mut fb = render::Framebuffer::new(self.scene.width(), self.scene.sky_height());
        // D66: a sandstorm washes the sky the same way it washes the ground.
        let dust = self.scene.weather().dust;
        // Clouds drift with the wind: the offset is wall-clock based, so a
        // redraw never jumps back (H20 review).
        let wind = (self.started_at.elapsed().as_secs_f32() * 0.9) as i32;
        // B42: the far ground fades into the same air the backdrop hazes its
        // own distance towards, so the horizon has no seam at any hour.
        self.scene
            .set_horizon_air(sky::backdrop::air_colour(&pal));
        // B45: the city's own glow (residents → era-scale ramp, dimmed in the
        // small hours; an empty world is a dark site) washes the faint stars
        // and the Milky Way band out as the city grows.
        let light_pollution =
            sky::draw::light_pollution_for(&st, self.sim.as_ref().map(|s| s.population()));
        let t_dw = Instant::now();
        sky::draw::draw_with_treatment(
            &st,
            &pal,
            &mut fb,
            self.config.seed,
            self.scene.sky_height() as i32,
            wind,
            self.scene.horizon(),
            &mut self.night,
            light_pollution,
        );
        stall("draw_sky", t_dw);
        render::biome::dust_sky(&mut fb, dust);
        let t_ss = Instant::now();
        self.scene.set_sky(fb);
        stall("set_sky", t_ss);
        // Water picks up the sky (B21 item 4): a pale blue sea by day, a warm
        // glint at sunset, a dark moonlit blue at night.
        self.scene.set_water_brightness(
(0.2126 * tint[0] + 0.7152 * tint[1] + 0.0722 * tint[2]).clamp(0.0, 1.0),

        );
        self.scene.set_water_tint(render::palette::mix(pal.horizon, pal.zenith, 0.25));
        self.needs_composite = true;
    }

    /// B57: keep the cloud field in step with the weather (cover, wind) and the
    /// hour (ink). Cheap: the field only re-populates when the cover moves into
    /// a new band, and the ink only re-damages the cloud rectangles when it
    /// actually changed.
    fn sync_clouds(
        &mut self,
        st: &sky::SkyState,
        pal: &sky::Palette,
        ls: &render::light::LightState,
        storm: f32,
        wind_dir: f32,
        wind_kmh: f32,
    ) {
        let mut changed = self.scene.set_clouds(st.cloud, storm);
        self.scene.set_cloud_wind(wind_dir, wind_kmh);
        changed |= self.scene.set_cloud_ink(render::clouds::Ink::new(
            sky::clouds::tint_with_sun(pal, sky::backdrop::sun_fraction(st)),
            ls,
            st.cloud,
            storm,
        ));
        if changed {
            self.needs_composite = true;
        }
    }

    fn setup_scene_and_sim(&mut self, fb_w: usize, fb_h: usize) {
        // B50: the next world, founded where the last one ended: a new seed
        // and a new land, for this founding only.
        if let Some(rb) = self.rebirth.take() {
            println!(
                "rebirth: {} Founding a new world, seed {}, land {}",
                rb.ended,
                rb.seed,
                rb.biome.name()
            );
            self.rebirth_restore = Some((self.config.seed_forced, self.config.biome));
            self.config.seed = rb.seed;
            self.config.seed_forced = true;
            self.config.biome = Some(rb.biome);
        }
        let vp = map::world_for_viewport(fb_w as i32, fb_h as i32);
        // B17 item 4: found on the *founding monitor* (the largest one on a very
        // wide union), not at the middle of the whole union.
        let focus = self
            .current_union
            .map(|l| l.focus)
            .unwrap_or((fb_w as i32 / 2, fb_h as i32 / 2));
        let scale = self.current_union.map(|l| l.scale).unwrap_or(1).max(1) as f32;
        let (fx, fy) = map::screen_to_tile(&vp, focus.0 as f32 / scale, focus.1 as f32 / scale);
        // B34: keyed by the world size in tiles; migrate a pre-B34 file first.
        let _ = migrate_layout_save(&self.config.save_path, vp.w, vp.h);
        let layout_path = world_save_path(&self.config.save_path, vp.w, vp.h);

        match self.config.mode {
            SceneMode::Sim => {
                let mut s = if layout_path.exists() {
                    match sim::Sim::load(&layout_path) {
                        Ok(mut loaded_sim) => {
                            if loaded_sim.map.w == vp.w && loaded_sim.map.h == vp.h {
                                println!(
                                    "Loaded layout-matched city save from {}",
                                    layout_path.display()
                                );
                                declare_frame(&mut loaded_sim, fb_w, fb_h, self.config.sky_frac);
                                if let Ok(meta) = std::fs::metadata(&layout_path) {
                                    if let Ok(mtime) = meta.modified() {
                                        let elapsed_real = SystemTime::now()
                                            .duration_since(mtime)
                                            .unwrap_or(Duration::ZERO)
                                            .as_secs_f64();
                                        if elapsed_real > 0.0 {
                                            let catch_up_sim = elapsed_real * loaded_sim.pace.scale();
                                            println!(
                                                "Catching up {:.1} sim-days of offline time ({:.1} real seconds)",
                                                catch_up_sim / 86400.0,
                                                elapsed_real
                                            );
                                            loaded_sim.catch_up_to(loaded_sim.time + catch_up_sim);
                                        }
                                    }
                                }
                                // B61d: the `--sim-days` fast-forward is applied
                                // once, below, for every way a world is made.
                                loaded_sim.save(&self.config.save_path).ok();
                                loaded_sim
                            } else {
                                println!(
                                    "Layout save at {} has map size {}x{}, but layout requires {}x{}. Starting fresh for this layout.",
                                    layout_path.display(),
                                    loaded_sim.map.w,
                                    loaded_sim.map.h,
                                    vp.w,
                                    vp.h
                                );
                                let mut new_sim =
                                    sim::Sim::with_focus_in(
                                        self.config.seed,
                                        self.config.pace,
                                        vp.w,
                                        vp.h,
                                        (fx, fy),
                                        self.config.biome,
                                    );
                                new_sim.set_focus(fx, fy);
                                new_sim.found_again();
                                new_sim.save(&layout_path).ok();
                                new_sim.save(&self.config.save_path).ok();
                                new_sim
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "Failed to load layout save from {}: {:?}; starting fresh",
                                layout_path.display(),
                                e
                            );
                            let mut new_sim =
                                sim::Sim::with_focus_in(
                                        self.config.seed,
                                        self.config.pace,
                                        vp.w,
                                        vp.h,
                                        (fx, fy),
                                        self.config.biome,
                                    );
                            new_sim.set_focus(fx, fy);
                            new_sim.found_again();
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
                                declare_frame(&mut loaded_sim, fb_w, fb_h, self.config.sky_frac);
                                if let Ok(meta) = std::fs::metadata(&self.config.save_path) {
                                    if let Ok(mtime) = meta.modified() {
                                        let elapsed_real = SystemTime::now()
                                            .duration_since(mtime)
                                            .unwrap_or(Duration::ZERO)
                                            .as_secs_f64();
                                        if elapsed_real > 0.0 {
                                            let catch_up_sim = elapsed_real * loaded_sim.pace.scale();
                                            println!(
                                                "Catching up {:.1} sim-days of offline time ({:.1} real seconds)",
                                                catch_up_sim / 86400.0,
                                                elapsed_real
                                            );
                                            loaded_sim.catch_up_to(loaded_sim.time + catch_up_sim);
                                        }
                                    }
                                }
                                loaded_sim.save(&layout_path).ok();
                                loaded_sim
                            } else {
                                let old_archive = format!(
                                    "{}.map{}x{}",
                                    self.config.save_path.display(),
                                    loaded_sim.map.w,
                                    loaded_sim.map.h
                                );
                                println!(
                                    "Base save at {} has map size {}x{}, but layout {}x{} requires {}x{}. Archiving to {} and starting fresh for {}x{}.",
                                    self.config.save_path.display(),
                                    loaded_sim.map.w,
                                    loaded_sim.map.h,
                                    fb_w,
                                    fb_h,
                                    vp.w,
                                    vp.h,
                                    old_archive,
                                    fb_w,
                                    fb_h
                                );
                                loaded_sim.save(Path::new(&old_archive)).ok();

                                let mut new_sim =
                                    sim::Sim::with_focus_in(
                                        self.config.seed,
                                        self.config.pace,
                                        vp.w,
                                        vp.h,
                                        (fx, fy),
                                        self.config.biome,
                                    );
                                new_sim.set_focus(fx, fy);
                                new_sim.found_again();
                                new_sim.save(&layout_path).ok();
                                new_sim.save(&self.config.save_path).ok();
                                new_sim
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "Failed to load base save from {}: {:?}; starting fresh",
                                self.config.save_path.display(),
                                e
                            );
                            let mut new_sim =
                                sim::Sim::with_focus_in(
                                        self.config.seed,
                                        self.config.pace,
                                        vp.w,
                                        vp.h,
                                        (fx, fy),
                                        self.config.biome,
                                    );
                            new_sim.set_focus(fx, fy);
                            new_sim.found_again();
                            new_sim.save(&layout_path).ok();
                            new_sim.save(&self.config.save_path).ok();
                            new_sim
                        }
                    }
                } else {
                    // D44: a brand-new world gets a fresh random seed (from the
                    // clock and the pid), so every first start is a new place.
                    // `--seed` still forces one, and a save always keeps its own.
                    if !self.config.seed_forced {
                        self.config.seed = random_seed();
                    }
                    println!(
                        "No existing save found. Starting fresh world for layout {}x{} (map {}x{}), seed {}",
                        fb_w, fb_h, vp.w, vp.h, self.config.seed
                    );
                    let mut new_sim = sim::Sim::with_focus_in(
                                        self.config.seed,
                                        self.config.pace,
                                        vp.w,
                                        vp.h,
                                        (fx, fy),
                                        self.config.biome,
                                    );
                    new_sim.set_focus(fx, fy);
                    new_sim.found_again();
                    println!(
                        "world seed {} — water layout {}",
                        new_sim.seed,
                        new_sim.map.layout.name()
                    );
                    new_sim.save(&layout_path).ok();
                    new_sim.save(&self.config.save_path).ok();
                    new_sim
                };

                // B61d: the developer knobs that move a world's clock are applied
                // here, once, for every way a world can be made (a new one, a
                // loaded one, a rebirth). They used to sit inside each of the
                // seven construction arms, which meant every *rebirth* re-ran the
                // fast-forward: `--sim-days 60` founded a world, ran it past its
                // own ending (~30 days), the ending rebirthed, and the new world
                // was fast-forwarded again — an endless loop that never drew a
                // frame (measured: 10 worlds founded, 0 frames, 100 % of a core).
                if self.config.endings_off {
                    s.postpone_ending();
                }
                if self.config.sim_days > 0.0 && s.days() < self.config.sim_days {
                    println!(
                        "Fast-forwarding save from day {:.1} to day {:.1}",
                        s.days(),
                        self.config.sim_days
                    );
                    s.run_to(self.config.sim_days * 86400.0);
                    // Once per run: a world founded later starts at day 0.
                    self.config.sim_days = 0.0;
                }

                // B34: built deferred — the light, the fixtures and the city are
                // set first, so the layer is rasterised **once** instead of the
                // four times startup used to spend.
                let mut scene = Scene::from_map_deferred(
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
                    self.config.sky_frac,
                );
                // B69b: trees sway (within budget at the real 7040x1440: ~0 calm,
                // 1.4 % breeze, 3 % strong wind, 5.8 % in a rare 70 km/h storm).
                // `CITYLIVE_TREE_SWAY=0` turns it off for A/B checks.
                scene.set_tree_sway_before_build(
                    std::env::var("CITYLIVE_TREE_SWAY").map_or(true, |v| v != "0"),
                );
                // B30: a test knob to start the saved city at a given hour
                // (a rush hour is what makes the traffic measurable).
                if let Some(h) = self.config.sim_hour {
                    s.set_clock_hour(h);
                }
                // B50: `--ending KIND --ending-at PHASE`: stage it.
                if let Some((kind, ph, p)) = self.config.ending_test.take() {
                    s.stage_ending(kind, ph, p);
                    println!("ending staged: {} at {} {:.2}", kind.name(), ph.name(), p);
                }
                // B61d: `CITYLIVE_CIVIC=rally|fire|blackout|bust|boom` stages a
                // civic event for a measurement run (the spike's `stage_civic`
                // does the same for a still). Gets the clock from `--sim-hour`.
                if let Ok(what) = std::env::var("CITYLIVE_CIVIC") {
                    let hour = self.config.sim_hour.unwrap_or(13.0).clamp(0.0, 23.99);
                    s.stage_civic(&what, s.days().floor() as i64, hour);
                    println!("civic staged: {what} at day {:.0} hour {hour:.2}", s.days().floor());
                }
                // The light first (quietly: no rebuild for the window glow).
                let ms = sky::clock_ms(
                    self.config.time_override.as_deref(),
                    self.config.date_override.as_deref(),
                    self.config.lon,
                );
                let st = sky::state(ms, self.config.lat, self.config.lon);
                let pal = sky::palette::palette(&st);
                self.last_tint = pal.ambient_tint;
                self.last_glow = pal.window_glow;
                scene.set_light_quiet(pal.ambient_tint, pal.window_glow);
                let mut ls = scene.light_state();
                ls.sun = render::light::Orb::new(st.sun_alt, st.sun_az);
                ls.moon = render::light::Orb::new(st.moon_alt, st.moon_az);
                ls.moon_illum = st.moon_illum as f32;
                ls.dir_strength = pal.light_dir_strength;
                ls.cloud = self.scene.weather().cloud;
                scene.set_light_state_quiet(ls);

                // B31b: the sim sited the marina and the port against a guessed
                // camera; now that the real one exists, put them in the frame.
                let cam = (scene.cam.ox, scene.cam.oy);
                s.set_viewport(cam.0, cam.1, fb_w as i32, fb_h as i32);
                // B42f: tell the city where the land line is, so it keeps the
                // belt below it as countryside instead of building things the
                // horizon would cut flat.
                s.set_horizon(scene.sky_height() as i32, HORIZON_BELT_PX);
                // B41b: the wildlife layer is built once per world; the map is
                // only read for its habitats (`update` follows the live map).
                let mut wl = wildlife::Wildlife::new(self.config.seed ^ 0x77, &s.map);
                wl.set_land_line(self.scene.sky_height() as i32);
                self.wildlife = Some(wl);
                self.wl_next = Instant::now();
                self.sim = Some(s);
                self.scene = scene;
                // B31b: the railway, the station and the port are static, so
                // they go into the baked layer before the first frame.
                let fx = self.sim_fixtures();
                self.scene.set_fixtures_quiet(fx);
                // One rebuild for terrain + fixtures + sprites + city, with the
                // sprite cache kept in the state for every later update (B34).
                // D66: the buildings wear the land's vernacular.
                self.kit = buildkit::KitCache::with_style(match &self.sim {
                    Some(sim) => buildkit::BiomeStyle::of(sim.map.biome),
                    None => buildkit::BiomeStyle::Temperate,
                });
                let objs = match &self.sim {
                    Some(sim) => {
                        let mut objs = endfx::kinds::objects(sim);
                        self.kit.attach_ruined(&mut objs, &kit_light(sim, Some(self.last_glow)), &endfx::ruins(sim));
                        objs
                    }
                    None => Vec::new(),
                };
                let sprites = self.kit.sprites_from(0);
                // B73r2: the nightlife is gated on the sim's own town (D80).
                if let Some(sim) = &self.sim {
                    self.scene.set_town_stats(sim.population(), sim.era().index());
                }
                self.scene.set_city_with_sprites(sprites, objs);
            }
            SceneMode::Empty => {
                let scene = Scene::new(SceneConfig {
                    width: fb_w,
                    height: fb_h,
                    seed: self.config.seed,
                    mode: SceneMode::Empty,
                    cars: 0,
                    peds: 0,
                    buildings: 0,
                    rain: self.config.rain,
                });
                self.sim = None;
                self.kit = buildkit::KitCache::new();
                self.scene = scene;
            }
            SceneMode::Test => {
                let scene = Scene::new(SceneConfig {
                    width: fb_w,
                    height: fb_h,
                    seed: self.config.seed,
                    mode: SceneMode::Test,
                    cars: self.config.cars,
                    peds: self.config.peds,
                    buildings: self.config.buildings,
                    rain: self.config.rain,
                });
                self.sim = None;
                self.kit = buildkit::KitCache::new();
                self.scene = scene;
            }
        }

        if let Some(sim) = &self.sim {
            println!(
                "world founded: {} ({}), seed {}, ends in about {:.0} days",
                sim.city_name,
                sim.biome.name(),
                sim.seed,
                sim.ending.world_days
            );
        }
        if let Some((forced, biome)) = self.rebirth_restore.take() {
            self.config.seed_forced = forced;
            self.config.biome = biome;
        }
        self.end_tr = endfx::Tracker::default();
        if let Some(sim) = &self.sim {
            endfx::rebuilt(&mut self.end_tr, sim);
        }
        self.scene.set_water_anim(self.config.water_anim);
        self.current_fb = Framebuffer::new(fb_w, fb_h);
        self.prev_hash = 0;
        self.frame_seq += 1;
        self.needs_composite = true;
        for head in &mut self.heads {
            head.last_drawn_seq = 0;
        }
    }

    /// Re-attach the city after a sim change (B34): only specs the cache has not
    /// seen are rasterised, and only the tiles that changed are repainted. The
    /// old path rebuilt every sprite and the whole static layer on each change.
    pub fn update_city(&mut self) {
        let t_all = Instant::now();
        let (mut objs, light) = match &self.sim {
            Some(sim) => (endfx::kinds::objects(sim), kit_light(sim, Some(self.last_glow))),
            None => return,
        };
        // B61f: the parts of this cost a different frame each time — a new
        // building's sprite is rasterised here, once, and that is the ~200 ms
        // hitch a growing town shows every few tens of seconds.
        stall("city.objects", t_all);
        self.wl_objs = objs.clone();
        self.kit_bucket = light.bucket;
        let before = self.kit.sprite_count();
        let t = Instant::now();
        // B50: a building the ending has done something to is drawn as such.
        let ruin = self.sim.as_ref().map(endfx::ruins).unwrap_or_default();
        self.kit.attach_ruined(&mut objs, &light, &ruin);
        stall("city.sprites", t);
        if self.kit.sprite_count() > before {
            let t = Instant::now();
            let extra = self.kit.sprites_from(before);
            self.scene.push_kit_sprites(extra);
            stall("city.push", t);
        }
        let t = Instant::now();
        // B73r2: the nightlife's gate is the sim's own town (D80), set before the
        // city install so the plan is built with it.
        if let Some(sim) = &self.sim {
            self.scene.set_town_stats(sim.population(), sim.era().index());
        }
        self.scene.set_city(objs);
        if let Some(sim) = &self.sim {
            self.scene.set_street(&street_look(sim));
            // B49: the technology installed by its crews, and the drones above.
            // B49b: the technology crews have their own list — they are drawn
            // through the depth plane (the van stands on the road, not on a
            // roof), and the wind farm's rotors turn per frame.
            self.scene.set_tech(&tech_look(sim));
            self.scene.set_street_crews(sim.streetworks().crews());
            self.scene.set_tech_crews(sim.tech().crews());
            self.scene.set_turbines(sim.tech().turbines.clone());
            self.scene.set_drones(drone_positions(sim));
        }
        // B85: the drops splash where the ground and the roofs are, so re-derive
        // the landing profile from the static layer's depth plane whenever the
        // city changes; the water profile (for the ripples) comes with it.
        self.wx_ground = ground_profile(
            self.scene.depth(),
            self.scene.width(),
            self.scene.height(),
        );
        self.wx_water = water_profile(self.scene.water_px(), self.scene.width());
        if let Some(fx) = self.wx_fx.as_mut() {
            fx.set_ground(Some(self.wx_ground.clone()));
            fx.set_water(Some(self.wx_water.clone()));
        }
        stall("city.set", t);
        stall("update_city", t_all);
    }

    /// B72 (D78): tell the scene which of every building's windows are lit.
    /// Radio silence unless the hour moved a minute or the residents changed.
    fn push_window_plan(&mut self, hour: f64) {
        let Some(sim) = &self.sim else {
            return;
        };
        let people = sim.people.list.len();
        if (hour - self.win_hour).abs() < 1.0 / 60.0 && people == self.win_people {
            return;
        }
        if !(0.0..24.0).contains(&hour) {
            return;
        }
        self.win_hour = hour;
        self.win_people = people;
        let plan = endfx::kinds::window_plan(sim, hour as f32);
        self.scene.set_window_plan(&plan);
    }

    /// B37: poll the settings file and apply what can be applied live. Returns
    /// true when something changed (so the caller can note it).
    pub fn poll_settings(&mut self) -> bool {
        if self.settings_checked.elapsed() < Duration::from_secs(2) {
            return false;
        }
        self.settings_checked = Instant::now();
        let path = self.config.config_path.clone();
        let mt = config::Config::mtime(&path);
        if mt.is_none() || mt == self.settings_mtime {
            return false;
        }
        self.settings_mtime = mt;
        let (c, warn) = config::Config::load_or_create(&path);
        if let Some(w) = warn {
            eprintln!("settings: {w}");
            return false;
        }
        self.apply_settings(c);
        true
    }

    /// B96: move the city's place. The sky, the season and the weather follow it.
    fn set_place(&mut self, lat: f64, lon: f64) {
        if (self.config.lat, self.config.lon) == (lat, lon) {
            return;
        }
        self.config.lat = lat;
        self.config.lon = lon;
        self.wx_next_fetch = Instant::now();
        self.last_sky = Instant::now() - Duration::from_secs(120);
        self.needs_composite = true;
    }

    /// Apply a freshly loaded settings file.
    fn apply_settings(&mut self, c: config::Config) {
        let old = std::mem::replace(&mut self.settings, c.clone());
        let plan = config::plan(&old, &c);
        if !self.config.location_forced {
            // B96: the user's own place, or the last detection (auto).
            let l = &c.location;
            let eff = locate::effective(l.auto, plan.lat, plan.lon, &l.name, &locate::cache_path());
            self.set_place(eff.lat, eff.lon);
            if l != &old.location {
                // Detect again now (auto) and refetch the weather for the new place.
                self.loc_next = Instant::now();
            }
        }
        // Cheap, per-frame knobs (the loop reads `state.config`, so these take
        // effect on the next frame — B38).
        self.config.fps = plan.fps;
        self.config.speed = plan.speed;
        self.config.holidays = plan.holidays;
        if c.holidays != old.holidays {
            // Off = no garlands and no fireworks (the scene redraws once).
            let look = if c.holidays {
                self.current_holiday()
            } else {
                render::holiday::HolidayLook::default()
            };
            self.scene.set_holiday(look);
        }
        // B47: a living-sky switch takes effect at once; the next pass erases
        // what the layer had painted (the restore runs even with everything
        // off).
        if c.aurora != old.aurora || c.iss != old.iss || c.flights != old.flights {
            self.needs_composite = true;
            self.last_sky = Instant::now() - Duration::from_secs(120);
        }
        if c.weather != old.weather || c.weather_fixed != old.weather_fixed {
            self.config.weather = c.weather_arg();
            self.wx_next_tick = Instant::now();
        }
        if c.density != old.density {
            // Traffic and villagers are budgeted by the scene's own lists; the
            // new density takes effect on the next world build, which is
            // scheduled below with the other structural knobs.
            self.needs_composite = true;
        }
        // Structural knobs: zoom, sky, layer, density and the output lists need
        // the layout (and possibly the world) rebuilt. The city is kept: saves
        // are keyed by world size.
        if plan.structural {
            self.config.scale_override = c.scale_override();
            self.config.sky_frac = c.sky.frac();
            self.config.layer = match c.layer {
                config::LayerMode::Bottom => Layer::Bottom,
                config::LayerMode::Background | config::LayerMode::PlasmaPlugin => {
                    Layer::Background
                }
            };
            // Re-run the layout: it rebuilds the scene at the new size and
            // reloads the same city from its save.
            self.current_union = None;
            self.layout_settle = Some(Instant::now());
        }
        // The pace is a property of the running sim.
        if let Some(sim) = &mut self.sim {
            sim.set_pace(c.pace());
        }
        println!(
            "settings applied: pace {} speed {:.2} zoom {} sky {} fps {} holidays {} density {}",
            c.pace,
            c.speed,
            c.zoom.name(),
            c.sky.name(),
            c.fps,
            c.holidays,
            match c.density {
                config::Density::Calm => "calm",
                config::Density::Lively => "lively",
            }
        );
        self.needs_composite = true;
    }

    /// The day's dressing (B53), in precedence order: the Bills gameday, then a
    /// personal date from the settings file, then the public holiday (when the
    /// holidays switch is on). Off: no dressing for the public holidays only.
    fn current_holiday(&self) -> render::holiday::HolidayLook {
        use render::holiday::{Holiday, HolidayLook};
        let g = &self.game_now;
        if self.settings.bills_gameday {
            match g.phase {
                world::GamedayPhase::Pregame | world::GamedayPhase::Live => {
                    return bills_look(Holiday::Bills);
                }
                world::GamedayPhase::Won => {
                    let k = if g.fireworks { Holiday::BillsWin } else { Holiday::Bills };
                    return bills_look(k);
                }
                // A loss or a tie is a quiet evening: no dressing from the game.
                world::GamedayPhase::Lost | world::GamedayPhase::Tied => {}
                // The day after a win the city still celebrates (D68).
                world::GamedayPhase::None if g.mood == world::Mood::Celebrate => {
                    return bills_look(Holiday::Bills);
                }
                world::GamedayPhase::None => {}
            }
        }
        let (y, m, d, _) = day_parts(self.config.date_override.as_deref());
        if let Some(sd) = world::on_date(&self.settings.dates, y, m, d) {
            return personal_look(sd.style);
        }
        if self.config.holidays {
            let mut look = holiday_look(
                self.config.time_override.as_deref(),
                self.config.date_override.as_deref(),
            );
            // B76: residents put the lights up through December and take them
            // down in the first days of January (the holidays switch gates it).
            look.lights = world::calendar::holiday_lights(m, d);
            return look;
        }
        HolidayLook::default()
    }

    /// B53 (every 5 s, with the tray refresh): read the feed's latest Bills
    /// reading, step the gameday tracker, announce a final or a personal date
    /// once in the feed, and dress the city for the day.
    fn gameday_tick(&mut self) {
        let now = Instant::now();
        // The feed is fetched on a thread, on the feed's own bucket.
        #[cfg(feature = "net")]
        if self.settings.bills_gameday && now >= self.sports_next_fetch {
            self.sports_next_fetch =
                now + Duration::from_millis(world::feeds::SPORTS_REFRESH_MS as u64);
            let slot = self.sports_shared.clone();
            let cache = cache_file("espn-nfl.json");
            std::thread::spawn(move || {
                let client = world::UreqClient::new();
                let game = world::sports::fetch(&client, &cache).and_then(|sb| sb.bills_game());
                if let Ok(mut g) = slot.lock() {
                    *g = Some(game);
                }
            });
        }
        #[cfg(feature = "net")]
        if let Ok(mut g) = self.sports_shared.lock() {
            if let Some(game) = g.take() {
                self.game_latest = game;
            }
        }
        // The switch off stands the gameday down at once.
        // The NFL scoreboard lists the whole week: only a game near its
        // kickoff is today's (the owner's desktop celebrated Sunday's win on a Tuesday).
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let game = if self.settings.bills_gameday {
            self.game_latest.clone().filter(|g| g.is_today(now_ms, local_utc_offset_ms()))
        } else {
            None
        };
        let (y, m, d, h) = day_parts(self.config.date_override.as_deref());
        let day = world::model::days_from_civil(y, m as i32, d as i32);
        let upd = self.game_tracker.update(day, h, game.as_ref());
        if let Some(sim) = self.sim.as_mut() {
            if let Some(line) = &upd.feed_line {
                sim.note_city_line(line);
            }
            // A personal date gets one feed line on its day.
            if let Some(sd) = world::on_date(&self.settings.dates, y, m, d) {
                if self.date_told != Some((y, m, d)) {
                    self.date_told = Some((y, m, d));
                    sim.note_city_line(&format!("{}: the city dresses up for the day.", sd.name));
                }
            }
        }
        self.game_now = upd.gameday;
        let look = self.current_holiday();
        self.scene.set_holiday(look);
    }

    /// B53 item 3: the idle watch runs while the setting is on, and stops
    /// when it is off (checked with the tray refresh, every 5 s).
    fn sync_idle(&mut self) {
        if self.settings.quiet_when_away {
            if self.idle.is_none() {
                self.idle = Some(idle::IdleWatch::spawn());
            }
        } else {
            self.idle = None;
        }
    }

    /// Is there anything on screen that moves? (D71: this is what turns the
    /// animation cadence on. A sleeping city asks for no frames at all.)
    pub fn animation_live(&self) -> bool {
        self.scene.people_count() > 0
            || self.sky_live.live
            || self.traffic_shown > 0
            || self.movers_shown > 0
            || (self.config.water_anim && self.scene.water_live())
            || self.wl_live
            || self.building
            || self.settlers_live
            || self.wx_fx.as_ref().is_some_and(|fx| fx.is_active())
            || self.scene.clouds_live()
            || self.scene.ending_animated()
            || self.scene.ending_busy()
            || self.scene.civic_live()
    }

    /// B50: tell the scene what the world's ending looks like right now, and
    /// re-attach the city when its ruins changed. Nothing happens (and nothing
    /// is allocated) while the world simply lives.
    /// B51: once per sim day, hand the city's static layer to the timelapse
    /// recorder (boxed down 1/8, one small delta appended beside the save).
    /// Called after the frame's flush; the per-frame cost is one comparison.
    fn timelapse_tick(&mut self) {
        if self.tl_final {
            return;
        }
        let Some(sim) = self.sim.as_ref() else { return };
        let day = sim.days().floor().max(0.0) as u32;
        // B65: the story ends at the end of the ending's aftermath. From the
        // morph on, the land is already turning into the next biome, and a
        // frame of that would put the next world's coast or desert at the end
        // of this world's timelapse. So the last frame is taken *before* the
        // morph starts — forced, because the aftermath often begins and ends
        // inside a day whose frame is already in the file.
        let over = sim.aftermath_over();
        if over {
            self.tl_final = true;
            let Some(rec) = self.tl_recorder.as_mut() else {
                return;
            };
            let layer = self.scene.static_layer();
            // B84: the layer's top rows are sky — the recorder must not read
            // them (that alone made the whole plane resident once a day).
            let top = self.scene.static_row0();
            match rec.record_final_from(day, layer.px(), layer.w, layer.h, top) {
                Ok(sim::timelapse::Recorded::Added(n)) => {
                    println!("timelapse: the end, day {day} +{n} B ({} B)", rec.size())
                }
                Ok(_) => {}
                Err(e) => eprintln!("timelapse: {e}"),
            }
            return;
        }
        if self.tl_day == Some(day) {
            return;
        }
        // A settlement still being cut from the forest has nothing to show;
        // the world's story starts when the city does.
        if sim.lots.is_empty() {
            return;
        }
        self.tl_day = Some(day);
        let t0 = Instant::now();
        let path = sim::timelapse::path_for(&world_save_path(
            &self.config.save_path,
            sim.map.w,
            sim.map.h,
        ));
        let rec = self
            .tl_recorder
            .get_or_insert_with(|| sim::timelapse::Recorder::open(&path));
        let layer = self.scene.static_layer();
        let top = self.scene.static_row0();
        match rec.record_from(day, layer.px(), layer.w, layer.h, top) {
            Ok(sim::timelapse::Recorded::Added(n)) => println!(
                "timelapse: day {day} +{n} B (file {} B) in {:.1} ms",
                rec.size(),
                t0.elapsed().as_secs_f64() * 1000.0
            ),
            Ok(_) => {}
            Err(e) => eprintln!("timelapse: {e}"),
        }
    }

    fn ending_tick(&mut self) {
        let Some(sim) = self.sim.as_mut() else { return };
        if sim.ending_view().is_none() && !self.scene.ending_on() {
            return;
        }
        let objs = endfx::kinds::objects(sim);
        let (redraw, rebuild) = endfx::sync(&mut self.scene, sim, &mut self.end_tr, &objs, self.sun_px);
        if rebuild {
            self.update_city();
            if let Some(sim) = &self.sim {
                endfx::rebuilt(&mut self.end_tr, sim);
            }
        }
        if let Some(sim) = &self.sim {
            let key = endfx::weather_key(sim);
            if key != self.end_wx_key {
                self.end_wx_key = key;
                // The sky and the weather follow the ending's progress.
                self.last_sky = Instant::now() - Duration::from_secs(120);
                self.wx_next_tick = Instant::now();
            }
        }
        if redraw || rebuild {
            self.needs_composite = true;
        }
    }

    /// B52: refresh the scene's civic look when the sim's civic state or the
    /// ten-minute bucket moved, re-attach the city when a lot became vacant,
    /// burnt or dark, and mark what animates.
    fn civic_tick(&mut self) {
        static CIVIC_START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let t0 = *CIVIC_START.get_or_init(Instant::now);
        self.scene.set_civic_clock(t0.elapsed().as_secs_f32());
        let Some(sim) = self.sim.as_ref() else { return };
        if endfx::civic::due(sim, &self.civic_tr) {
            let objs = endfx::kinds::objects(sim);
            let rebuild = endfx::civic::sync(&mut self.scene, sim, &objs, &mut self.civic_tr);
            if rebuild {
                self.update_city();
            }
            self.needs_composite = true;
        } else {
            self.scene.civic_tick();
        }
    }

    /// Step the wildlife layer by `dt` and remember whether anything is out.
    fn wildlife_step(&mut self, dt: f32) {
        let (Some(wl), Some(s)) = (self.wildlife.as_mut(), self.sim.as_ref()) else {
            self.wl_live = false;
            return;
        };
        let hour = ((s.time / 3600.0).rem_euclid(24.0)) as f32;
        wl.update(dt, &s.map, &self.wl_objs, hour);
        self.wl_live = wl.is_active();
    }

    /// One animation step for every moving layer, with the same real `dt`
    /// (B54/D71). Everything is interpolated off the smooth sim clock or off
    /// the elapsed time, so at any fps the motion is continuous: no layer here
    /// has a rate of its own any more.
    ///
    /// Returns true when the step changed a composite pixel.
    fn animate(&mut self, dt: f32) -> bool {
        let before = self.scene.pending_len();
        let ta = Instant::now();
        // B101: under load some classes are stepped every n-th tick with the
        // time they missed (their motion is a function of time, so it jumps
        // instead of slowing down).
        self.load.next_tick();
        let div = self.load.divisors();
        self.water_dt += dt;
        self.cloud_dt += dt;
        self.mover_dt += dt;
        let movers_due = self.load.due(div.movers);
        let mover_dt = if movers_due { std::mem::take(&mut self.mover_dt) } else { 0.0 };

        // 0. B50: the world's ending, if it is on.
        self.ending_tick();
        // 0b. B52: the civic layer (elections, fires, blackouts, smog, street
        //     life). A comparison per frame while nothing changes.
        self.civic_tick();

        // 1. The crews at every site being built.
        if self.building && self.sim.is_some() {
            self.update_city();
        }

        // 2. The settlers: the party, the camp, the felled logs, the trees
        //    going over. Their legs interpolate off the smooth clock.
        if self.settlers_live {
            if let Some(s) = &self.sim {
                let t = s.smooth_time();
                let mut dirty = self.scene.set_settlers(s.settlers_at(t));
                dirty |= self.scene.set_falling(s.falling_at(t));
                dirty |= self.scene.set_camp(s.camp().map(|c| {
                    (
                        s.founding.0,
                        s.founding.1,
                        c.tents,
                        c.fire_ok.then_some(c.fire),
                    )
                }));
                dirty |= self.scene
                    .set_logs(s.log_piles().iter().map(|p| (p.x, p.y, p.kind)).collect());
                let _ = dirty;
            }
        }

        // 3. The residents (B43/D65): everybody doing something, at true
        //    scale, every resident who is out, gliding rather than stepping.
        let tp = Instant::now();
        if movers_due {
            let mut t = 0.0;
            if let Some(s) = &self.sim {
                t = s.smooth_time();
                self.scene.set_sites(s.build_sites());
                s.people_view_into(&mut self.sim_figs);
                if self.scene.ending_on() {
                    let d = endfx::split_drones(&mut self.sim_figs);
                    self.scene.set_ending_drones(d);
                }
            } else {
                self.sim_figs.clear();
            }
            self.figs.clear();
            for f in &self.sim_figs {
                self.figs.push(render::people::Figure {
                    x: f.x,
                    y: f.y,
                    pose: f.pose.to_u8(),
                    tone: f.tone,
                    child: f.child,
                    prop: f.prop.to_u8(),
                    dir: f.dir,
                    open: f.open,
                    lift: f.lift,
                    frame: 0,
                });
            }
            // B62/D68: fewer people while the owner is away (1.0 when the switch is off).
            let away = idle::crowd_factor(
                self.idle.is_some(),
                self.idle.as_ref().and_then(|w| w.idle_for()),
            );
            idle::thin(&mut self.figs, away);
            self.scene.set_people(&self.figs, render::people::clock_f32(t));
            if std::env::var("CITYLIVE_DEBUG").is_ok() {
                eprintln!(
                    "debug tick: people raw {} visible {}",
                    self.figs.len(),
                    self.scene.people_count()
                );
            }
        }
        let d = tp.elapsed();
        self.timing.hit(|t| &mut t.people, d);

        // 4. Water: the phase is fractional now, so ripples scroll at the
        //    display rate instead of stepping six times a second.
        let tw = Instant::now();
        if self.config.water_anim && self.load.due(div.water) {
            self.water_frame = (self.water_frame
                + std::mem::take(&mut self.water_dt) * render::terrain::WATER_PHASE_HZ)
                % render::terrain::WATER_PHASE;
            self.scene.animate_water(self.water_frame);
        }
        let d = tw.elapsed();
        self.timing.hit(|t| &mut t.water, d);

        // 4b. B57/B61c: the clouds drift at the frame rate, and a cloud only
        //     repaints the strips it swept since its last whole pixel (D71).
        //     Nothing called this from the wallpaper, so the cloud shadows stood
        //     still until something else forced a full frame.
        let tc = Instant::now();
        if self.scene.clouds_live() && self.load.due(div.clouds) {
            self.scene.cloud_tick(std::mem::take(&mut self.cloud_dt));
        }
        let d = tc.elapsed();
        self.timing.hit(|t| &mut t.clouds, d);

        // 4c. B69/D71: the trees sway with the wind the clouds drift on. Each
        //     tree whose picture changed damages its own box; in calm air this
        //     is a comparison and returns.
        self.scene.tree_tick(dt);

        // 4d. B73/D80: the nightlife's chasers, screens, crown cycles and bad
        //     tubes. A few small boxes a frame at 6 Hz, and nothing by day.
        self.scene.sign_tick(dt);

        // 5. Traffic. The vehicles are posed by the sim, stepped by real time.
        let tt = Instant::now();
        if self.config.traffic && movers_due {
            if let Some(sim) = &mut self.sim {
                sim.traffic_step(mover_dt as f64);
            }
            let mut bodies = self.traffic_bodies();
            // B62/D68: and fewer cars while he is away.
            idle::thin(
                &mut bodies,
                idle::crowd_factor(
                    self.idle.is_some(),
                    self.idle.as_ref().and_then(|w| w.idle_for()),
                ),
            );
            self.scene.set_traffic(bodies);
            // B76 r2: the winter service clears the lanes it drives, and the
            // mask goes when the snow does.
            if let Some(sim) = &self.sim {
                let tiles = sim.service_tiles();
                if !tiles.is_empty() {
                    self.scene.plough_tiles(&tiles);
                }
            }
            if self.scene.weather().snow_cover < 0.05 {
                self.scene.clear_ploughed();
            }
            // B59: and the cars standing in their bays — a car park filling
            // through the working day, the motel's guests at night.
            if let Some(sim) = &self.sim {
                let parked = parked_bodies(sim);
                self.scene.set_parked(parked);
            }
            let shown = self.scene.traffic_count();
            self.traffic_shown = shown;
        }
        let d = tt.elapsed();
        self.timing.hit(|t| &mut t.traffic, d);

        // 5b. B49: the delivery drones drift at the frame rate (they are a pure
        //     function of the sim clock, so nothing about them is saved).
        if let Some(sim) = &self.sim {
            let drones = drone_positions(sim);
            self.scene.set_drones(drones);
            // B49b: the tech crews move a tile at a time, and the turbines turn
            // (each rotor damages its own rectangle — D71, nothing cut).
            self.scene.set_tech_crews(sim.tech().crews());
            self.scene.turbine_tick();
        }

        // 6. Rail, trams, boats, ships and planes (B31b).
        let tm = Instant::now();
        if movers_due {
            if let Some(sim) = &mut self.sim {
                sim.transport_step(mover_dt as f64);
            }
            let fx = self.sim_fixtures();
            self.scene.set_fixtures(fx);
            let movers = self.mover_bodies();
            self.scene.set_movers(movers);
            let shown = self.scene.movers_shown_count();
            self.movers_shown = shown;
        }
        let d = tm.elapsed();
        self.timing.hit(|t| &mut t.movers, d);

        // 7. Wildlife that is already out.
        let tl = Instant::now();
        if self.wl_live {
            self.wildlife_step(dt);
        }
        let d = tl.elapsed();
        self.timing.hit(|t| &mut t.wildlife_ms, d);

        // 7b. B62: the spectacle (festivals, balloons, the rainbow, side moments).
        self.spectacle_step();

        // 8. Precipitation: the particles step in real time (they used to step
        //    once a second with the weather book, which made rain fall in jumps
        //    however fast it was drawn).
        let mut overlay = false;
        let wx = self.wx_book_translated();
        if let Some(fx) = &mut self.wx_fx {
            fx.update(dt, &wx);
            overlay = fx.is_active();
        }
        // 8b. B61b: the weather's *static* half (washes, cloud shadows) is the
        //     scene's: hand it over here, once per change. A wash change
        //     recomposites the frame once; a drifting cloud shadow damages the
        //     two thin boxes it swept; steady weather costs nothing per frame.
        if overlay || self.scene.ambient().is_active() {
            let frame = render::Rect::from_size(self.current_fb.w, self.current_fb.h);
            let Self { scene, wx_fx, .. } = self;
            if let Some(fx) = wx_fx.as_ref() {
                if scene.set_ambient(fx.ambient(), frame) {
                    self.needs_composite = true;
                }
            }
        }

        let d = ta.elapsed();
        self.timing.hit(|t| &mut t.sim, d);
        self.motion_trace(dt);
        // B47: an aurora, the ISS or an aircraft is a frame that is about to
        // change, every frame it is up.
        let changed = overlay || self.wl_live || self.sky_live.live || self.scene.pending_len() > before;
        if changed {
            self.anim_quiet = 0;
        } else {
            self.anim_quiet = self.anim_quiet.saturating_add(1);
        }
        changed
    }

    /// B67: with `CITYLIVE_MOTION_TRACE=<file>` set, write one line per moving
    /// thing per animation step: `frame,t_ms,dt_ms,class,idx,x,y,fx,fy` where x,y is
    /// the pixel the scene draws it at and fx,fy the same before truncation (`tools/b67-motion/analyze.py` reads it).
    /// A single env lookup per frame when off.
    fn motion_trace(&mut self, dt: f32) {
        use std::io::Write;
        static OUT: std::sync::OnceLock<Option<std::sync::Mutex<(std::io::BufWriter<std::fs::File>, u64, Instant)>>> =
            std::sync::OnceLock::new();
        let slot = OUT.get_or_init(|| {
            let path = std::env::var("CITYLIVE_MOTION_TRACE").ok()?;
            let f = std::fs::File::create(path).ok()?;
            Some(std::sync::Mutex::new((std::io::BufWriter::new(f), 0, Instant::now())))
        });
        let Some(m) = slot else { return };
        let Ok(mut g) = m.lock() else { return };
        let (w, frame, t0) = &mut *g;
        let t_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let mut probes = Vec::new();
        self.scene.motion_probe(&mut probes);
        if let Some(wl) = &self.wildlife {
            let vp = map::Viewport {
                origin: (self.scene.cam.ox, self.scene.cam.oy),
                w: self.current_fb.w as i32,
                h: self.current_fb.h as i32,
            };
            for (i, (sp, x, y)) in wl.positions(&vp).into_iter().enumerate() {
                let _ = sp;
                probes.push(render::Probe { class: "animal", idx: i as u32, x, y, fx: x as f32, fy: y as f32 });
            }
        }
        for p in probes {
            let _ = writeln!(
                w,
                "{},{:.2},{:.2},{},{},{},{},{:.3},{:.3}",
                frame, t_ms, dt * 1000.0, p.class, p.idx, p.x, p.y, p.fx, p.fy
            );
        }
        *frame += 1;
        let _ = w.flush();
    }

    /// B61f: print what is resident, biggest first, on request
    /// (`CITYLIVE_MEM=1`). Two parts: the scene's planes and sprites, and the
    /// presenter's own frame, scratch and the shared frames the Plasma reader
    /// maps. The process RSS is printed beside the sum, so the part the table
    /// cannot name (the sim's city, the allocator, the binary) shows up as a
    /// remainder instead of hiding.
    ///
    /// B84: the sim's state, the building-sprite cache and the allocator are
    /// named too, and the remainder is printed as a number, so "what is left
    /// over" can be argued with. VmHWM (the peak) comes with it.
    /// B84: how much of a buffer is actually *resident* (`mincore`), not just
    /// allocated. A plane whose top rows are never written or read costs its
    /// size on paper and nothing in RSS; this says which.
    #[cfg(target_os = "linux")]
    fn resident(bytes: &[u8]) -> usize {
        let page = 4096usize;
        let addr = bytes.as_ptr() as usize;
        let start = addr & !(page - 1);
        let end = (addr + bytes.len() + page - 1) & !(page - 1);
        let n = (end - start) / page;
        if n == 0 {
            return 0;
        }
        let mut v = vec![0u8; n];
        // SAFETY: `start..end` is the allocation this slice lives in (rounded
        // out to page bounds, so it may touch the pages the allocator's own
        // header shares — harmless for a residency reading).
        let rc = unsafe { libc::mincore(start as *mut libc::c_void, end - start, v.as_mut_ptr()) };
        if rc != 0 {
            return bytes.len();
        }
        v.iter().filter(|b| **b & 1 != 0).count() * page
    }

    #[cfg(windows)]
    pub fn print_mem_report(&self) {
        println!(
            "memory: working set {} kB, peak {} kB (frame {:.1} MB)",
            procstat::rss_kb().unwrap_or(0),
            procstat::rss_peak_kb().unwrap_or(0),
            self.current_fb.bytes() as f64 / (1024.0 * 1024.0)
        );
    }

    #[cfg(target_os = "linux")]
    pub fn print_mem_report(&self) {
        let mut lines: Vec<(String, usize)> = Vec::new();
        for (k, b) in self.scene.mem_report() {
            lines.push((k.to_string(), b));
        }
        if let Some(s) = self.sim.as_ref() {
            for (k, b) in s.mem_report() {
                lines.push((k.to_string(), b));
            }
        }
        lines.push((
            // B84: the cache and the scene share the sprite pixels (one `Arc`
            // each), so they are counted once — in the scene's line.
            "kit cache (the same pixels as the scene's kit; counted there)".into(),
            0,
        ));
        lines.push(("frame (composite target)".into(), self.current_fb.bytes()));
        lines.push((
            "shared frames (one per head)".into(),
            self.shm_writers.iter().map(|(_, w)| w.bytes()).sum(),
        ));
        lines.push((
            "head slice scratch + PPM".into(),
            self.frame_px.capacity() * 4 + self.frame_ppm.capacity(),
        ));
        lines.sort_by(|a, b| b.1.cmp(&a.1));
        let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let field = |name: &str| -> u64 {
            status
                .lines()
                .find_map(|l| l.strip_prefix(name).map(|v| v.trim().to_string()))
                .and_then(|v| v.split_whitespace().next().map(|n| n.to_string()))
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        };
        let rss = field("VmRSS:");
        let hwm = field("VmHWM:");
        println!("memory (7040x1440 union; RSS {rss} kB, peak {hwm} kB):");
        let mut total = 0usize;
        for (k, b) in &lines {
            if *b >= 256 * 1024 {
                println!("  {:7.1} MB  {k}", mb(*b));
            }
            total += b;
        }
        println!("  {:7.1} MB  named above, all of it", mb(total));
        // B84: the four biggest planes, and how much of each is resident. A
        // plane's top rows are sky (never written, never read): they cost the
        // table's size and nothing in RSS, and this is the proof.
        let as_bytes = |px: &[render::palette::Color]| -> &[u8] {
            // SAFETY: a `u32` slice is a `u8` slice of four times the length.
            unsafe {
                std::slice::from_raw_parts(px.as_ptr() as *const u8, std::mem::size_of_val(px))
            }
        };
        let scene = &self.scene;
        let planes: [(&str, usize, usize); 4] = [
            (
                "static colour",
                scene.static_bytes_colour(),
                Self::resident(as_bytes(scene.static_px())),
            ),
            (
                "tinted static",
                scene.tinted_bytes(),
                Self::resident(as_bytes(scene.tinted_px())),
            ),
            (
                "frame",
                self.current_fb.colour_bytes(),
                Self::resident(as_bytes(self.current_fb.px())),
            ),
            (
                "static depth",
                scene.static_bytes_depth(),
                Self::resident(unsafe {
                    std::slice::from_raw_parts(
                        scene.static_depth().as_ptr() as *const u8,
                        scene.static_depth().len() * 2,
                    )
                }),
            ),
        ];
        for (k, size, live) in planes {
            println!(
                "  {:7.1} MB  {k}: {:.1} MB resident ({} %)",
                mb(size),
                mb(live),
                if size > 0 { live * 100 / size } else { 0 }
            );
        }
        // B84: what the table cannot see — the allocator's own arenas, the
        // binary and its libraries, the thread stacks, and anything the two
        // tables miss. Printed as a number so it cannot hide.
        let anon = field("RssAnon:");
        let file = field("RssFile:");
        let shmem = field("RssShmem:");
        let unnamed = ((rss * 1024) as usize).saturating_sub(total);
        println!(
            "  {:7.1} MB  not named (anon {:.1} + file {:.1} + shmem {:.1} = {:.1} MB resident)",
            mb(unnamed),
            anon as f64 / 1024.0,
            file as f64 / 1024.0,
            shmem as f64 / 1024.0,
            mb((anon + file + shmem) as usize * 1024)
        );
        // SAFETY: `mallinfo2` only reports the allocator's own counters.
        let mi = unsafe { libc::mallinfo2() };
        println!(
            "  allocator: {:.1} MB in use, {:.1} MB free in the heap, {:.1} MB mapped, {} mmap chunks",
            mb(mi.uordblks),
            mb(mi.fordblks),
            mb(mi.hblkhd),
            mi.hblks
        );
    }

    pub fn render_and_present(&mut self) {
        let Some(layout) = self.current_union else {
            return;
        };
        if self.heads.is_empty() {
            return;
        }

        let now = Instant::now();
        for head in &mut self.heads {
            if head.frame_pending {
                if let Some(t) = head.frame_requested_at {
                    if now.duration_since(t) >= Duration::from_millis(100) {
                        head.frame_pending = false;
                        head.frame_requested_at = None;
                        #[cfg(target_os = "linux")]
                        if let Some(buffers) = &head.buffers {
                            if let Some(b) = buffers.first() {
                                b.deactivate().ok();
                            }
                        }
                    }
                }
            }
        }

        let any_ready = self.heads.iter().any(|h| h.configured && !h.frame_pending);
        if !any_ready {
            return;
        }

        self.stats_collector.record_tick();
        // B61e: take (and clear) the scene's "a moving layer changed" hint here,
        // once, so every path through this function agrees about it.
        let moving_changed_hint = self.scene.clear_moving_changed();

        // B67: the shadow bake (270 ms at 7040x1440) runs a few ms a frame, never
        // as one freeze.
        if self.started_at.elapsed() > Duration::from_secs(8) {
            self.scene.set_async_shadow(true);
        }
        let t_sky = Instant::now();
        self.refresh_sky(false);
        stall("refresh_sky", t_sky);
        // B60: the shadows crossfade to the new bake over a few frames, so the
        // moving sun never steps.
        if self.scene.shadow_tick(now.saturating_duration_since(self.last_shadow).as_secs_f32().min(0.25)) {
            self.needs_composite = true;
        }
        self.last_shadow = now;
        // (B57: the 30 s "redraw the sky for the cloud drift" pass is gone —
        // the clouds are drawn per frame now, so re-baking the band every 30 s
        // was a full-frame composite for nothing.)
        let t0 = Instant::now();
        if std::env::var("PP_DEBUG").is_ok() && self.scene.dirty_count() > 0 {
            eprintln!("flush: {} dirty rects", self.scene.dirty_count());
        }
        if self.scene.has_dirty() {
            self.scene.flush_dirty();
        }
        stall("flush_dirty", t0);
        let t1 = Instant::now();
        self.timing.hit(|t| &mut t.flush, t1 - t0);
        self.timelapse_tick();

        // B61b: the weather particles are drawn on top of the composite, so the
        // boxes they covered last frame have to be repainted before this one
        // draws again — the same trick the wildlife and the meteors use. In the
        // shared-buffer transport they are painted into each head's own buffer
        // instead (with a paint log that restores them), so the scene frame
        // never sees a drop and a rainy frame stays damage-limited; in the
        // layer-shell path they are pixels of this frame like any other.
        let paint_overlay_in_shm = self.shm_dir.is_some();
        let wx_overlay = self.wx_fx.as_ref().is_some_and(|fx| fx.is_active());
        if !paint_overlay_in_shm && self.wx_fx.is_some() {
            let Self {
                scene,
                wx_fx,
                wx_dirty,
                wx_scratch,
                ..
            } = self;
            for r in wx_dirty.drain(..) {
                scene.mark_rect(r);
            }
            if let Some(fx) = wx_fx.as_ref() {
                if fx.is_active() {
                    fx.overlay_rects(wx_scratch);
                    for r in wx_scratch.iter() {
                        scene.mark_rect(*r);
                    }
                }
            }
        }

        // B30: composite only what changed. `None` = the whole frame, `Some([])`
        // = nothing at all (the presenter can skip the frame entirely).
        let pending = self.scene.take_pending();
        // B61b: a weather frame is no longer a full frame. The washes live in
        // the scene (one recomposite when they change) and the particles are
        // damage (or a paint log), so `full` is only the scene's own decision
        // plus the test scene's full-frame rain.
        let full = pending.is_none() || self.scene.rain_active();
        if std::env::var("PP_DEBUG").is_ok() {
            eprintln!(
                "pp: pending {:?} weather_over {} rain {} -> full {} dirty {}",
                pending.as_ref().map(|r| r.len()),
                wx_overlay,
                self.scene.rain_active(),
                full,
                self.scene.dirty_count()
            );
        }
        let mut damage: Vec<render::Rect> = Vec::new();
        match &pending {
            _ if full => self.scene.composite(&mut self.current_fb),
            Some(rects) if !rects.is_empty() => {
                let mut worst = Duration::ZERO;
                let mut worst_rect = render::Rect::EMPTY;
                for r in rects {
                    let t = Instant::now();
                    self.scene.composite_rect(&mut self.current_fb, *r);
                    let d = t.elapsed();
                    if d > worst {
                        worst = d;
                        worst_rect = *r;
                    }
                    damage.push(*r);
                }
                let area: i64 = damage.iter().map(|r| r.area()).sum();
                self.stats_collector
                    .record_damage(damage.len() as u64, area.max(0) as u64, worst, worst_rect);
            }
            _ => {}
        }
        // B84 r2: `CITYLIVE_VERIFY_FRAME=1` proves the damage model: every 30th
        // frame the whole frame is composited into a scratch buffer and compared
        // with the incrementally built one, and any pixel that differs is a
        // rectangle the damage pass forgot. Costs a full composite, so it is a
        // test knob, not something the wallpaper runs.
        let verify = std::env::var("CITYLIVE_VERIFY_FRAME").is_ok();
        if verify {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 30 == 0 {
                let mut scratch =
                    render::Framebuffer::new(self.current_fb.w, self.current_fb.h);
                self.scene.composite(&mut scratch);
                let a = self.current_fb.px();
                let b = scratch.px();
                let w = self.current_fb.w;
                let top = self.scene.static_row0();
                let (mut diff, mut sky) = (0usize, 0usize);
                let mut first = None;
                for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
                    if x != y {
                        if i / w < top {
                            sky += 1;
                        } else {
                            diff += 1;
                            if first.is_none() {
                                first = Some((i % w, i / w));
                            }
                        }
                    }
                }
                let mut samples: Vec<(usize, usize, u32, u32)> = Vec::new();
                if diff > 0 {
                    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
                        if x != y && i / w >= top {
                            samples.push((i % w, i / w, *x, *y));
                            if samples.len() >= 6 {
                                break;
                            }
                        }
                    }
                }
                if diff > 0 {
                    // Where: a coarse map (128x64 cells), the dearest eight.
                    let mut cells: std::collections::HashMap<(usize, usize), usize> = std::collections::HashMap::new();
                    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
                        if x != y && i / w >= top {
                            *cells.entry(((i % w) / 128, (i / w) / 64)).or_default() += 1;
                        }
                    }
                    let mut v: Vec<_> = cells.into_iter().collect();
                    v.sort_by(|p, q| q.1.cmp(&p.1));
                    if let Ok(c) = std::env::var("VERIFY_CELL") {
                        let p: Vec<usize> = c.split(',').filter_map(|v| v.parse().ok()).collect();
                        if p.len() == 2 {
                            let mut shown = 0;
                            for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
                                if x != y && i / w >= top && (i % w) / 128 == p[0] && (i / w) / 64 == p[1] && shown < 12 {
                                    eprintln!("verify px ({}, {}) incremental {:08x} full {:08x}", i % w, i / w, x, y);
                                    shown += 1;
                                }
                            }
                        }
                    }
                    v.truncate(60);
                    eprintln!(
                        "verify where (x/128,y/64 -> px): {}",
                        v.iter().map(|((cx, cy), n)| format!("({cx},{cy})={n}")).collect::<Vec<_>>().join(" ")
                    );
                }
                eprintln!(
                    "verify: {diff} px differ on the land, {sky} in the sky band (first {first:?}, {} damage rects) {samples:?}",
                    damage.len()
                );
            }
        }
        // B41b: wildlife, drawn after the scene composite (animals sit on top
        // of the ground); only the rectangles it swept go to the presenter.
        // D71: the update happens in `animate` at the frame rate, so a bird
        // flies smoothly instead of hopping six times a second.
        // B42: the pixels the *previous* wildlife pass painted are restored
        // from the scene before this one draws — otherwise a flying bird is
        // never erased and leaves a dotted trail across the sky.
        if self.wildlife.is_some() && std::env::var("CITYLIVE_NO_WILDLIFE").is_err() {
            let Self {
                wildlife,
                sim,
                scene,
                current_fb,
                wl_dirty,
                ..
            } = self;
            if let (Some(wl), Some(_s)) = (wildlife.as_mut(), sim.as_ref()) {
                let fb_bounds = Rect::from_size(current_fb.w, current_fb.h);
                for r in wl_dirty.drain(..) {
                    let r = r.intersect(&fb_bounds);
                    if !r.is_empty() {
                        scene.composite_rect(current_fb, r);
                        damage.push(r);
                    }
                }
                let vp = map::Viewport {
                    origin: (scene.cam.ox, scene.cam.oy),
                    w: current_fb.w as i32,
                    h: current_fb.h as i32,
                };
                wl.draw(current_fb, &vp, Some(scene.depth()));
                for r in wl.dirty_rects() {
                    let r = r.intersect(&fb_bounds);
                    if !r.is_empty() {
                        wl_dirty.push(r);
                        damage.push(r);
                    }
                }
            }
        } else if !self.wl_dirty.is_empty() {
            // Wildlife was turned off mid-run: erase what it left behind.
            self.wl_dirty.clear();
            self.needs_composite = true;
        }
        // B45: meteors. A streak lives ~850 ms and must animate every frame,
        // but the sky band is only redrawn every ~30 s — so the streaks are
        // drawn on the composite and their previous pixels restored from the
        // scene before the next pass, exactly the wildlife's trick. The slot
        // state machine runs on real wall clock, so a `--time` override still
        // gets live streaks over its pinned sky.
        {
            let fb_bounds = Rect::from_size(self.current_fb.w, self.current_fb.h);
            for r in self.met_dirty.drain(..) {
                let r = r.intersect(&fb_bounds);
                if !r.is_empty() {
                    self.scene.composite_rect(&mut self.current_fb, r);
                    damage.push(r);
                }
            }
            // B47: the living sky — the aurora, the ISS and the aircraft. Same
            // discipline as the meteors: they move every frame over a band that
            // is re-baked once a minute, so each pass erases the last one's
            // rectangles (from the scene) before it draws. Meteors go on top.
            let view = self.live_sky_view(fb_bounds);
            if let Some(view) = view {
                let Self {
                    sky_live,
                    scene,
                    current_fb,
                    ..
                } = self;
                for r in sky_live.draw(scene, current_fb, &view) {
                    let r = r.intersect(&fb_bounds);
                    if !r.is_empty() {
                        damage.push(r);
                    }
                }
            }
            let band_h = self.scene.sky_height() as i32;
            if band_h > 0 && self.star_fade >= 0.3 {
                if let Some(r) = sky::draw::meteor_layer(
                    &mut self.current_fb,
                    self.scene.sky(),
                    &self.night,
                    wall_ms_now(),
                    band_h,
                    self.star_fade,
                ) {
                    let r = r.intersect(&fb_bounds);
                    if !r.is_empty() {
                        self.met_dirty.push(r);
                        damage.push(r);
                    }
                }
            }
        }
        let t_comp = Instant::now();
        self.timing.hit(|t| &mut t.comp, t_comp - t1);
        if full {
            // B61f: the frames that cost the whole frame get their own row.
            self.timing.full_n += 1;
            self.timing.full_comp += (t_comp - t1).as_secs_f64() * 1e3;
        }
        // B61b: nothing changed is only true when the weather is not moving
        // particles either — the drops are pixels too, even when they land in a
        // shared buffer instead of this frame.
        // B61b: in the shared-buffer transport the drops are not pixels of this
        // frame, so "nothing changed" is still a frame worth exporting.
        let nothing_changed = !full && damage.is_empty() && !(paint_overlay_in_shm && wx_overlay);
        if !paint_overlay_in_shm {
            // Layer-shell path: paint the particles here and remember what they
            // covered, so the next frame restores it (B61b).
            let Self {
                wx_fx,
                current_fb,
                wx_dirty,
                wx_scratch,
                ..
            } = self;
            if let Some(fx) = wx_fx.as_ref() {
                if fx.is_active() {
                    fx.draw_overlay(current_fb, 0, 0);
                    wx_dirty.clear();
                    wx_dirty.extend_from_slice(wx_scratch);
                }
            }
        }
        let t2 = Instant::now();
        self.timing.hit(|t| &mut t.weather, t2 - t_comp);
        if nothing_changed {
            // Nothing moved: no hash, no present, no damage.
            self.needs_composite = false;
            self.idle_streak = self.idle_streak.saturating_add(1);
            return;
        }
        self.needs_composite = false;

        let fb_px = self.current_fb.px();
        // A 64-bit FNV over the pixels: same "did anything change" answer as a
        // full copy-and-compare, without the copy living in memory.
        //
        // B42d: only the **damaged** pixels are hashed. Nothing outside a
        // damage rectangle was written this frame (that is what a damage list
        // means), so hashing it again cannot change the answer — and on the owner's
        // 7040x1440 union the old full-frame pass read 40 MB every time a bird
        // moved (measured: ~10 ms a frame, most of the export path's CPU).
        // D71: two pixels a step. Popping one pixel at a time was 0.4-0.7 ms of
        // a 3 ms frame on the owner's desk once the composite got cheap; the answer
        // is the same "did anything change", 2x cheaper.
        // B61e: when a moving layer changed this frame, some pixel changed by
        // construction and the hash (0.7-1.2 ms on a rush-hour frame, more than
        // the export it guards) has nothing left to tell us.
        let known_change = !full && moving_changed_hint;
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |w: u64| {
            hash ^= w;
            hash = hash.rotate_left(5).wrapping_mul(0x100_0000_01b3);
        };
        if known_change {
            // The scene drew a moving layer: skip the pixel pass entirely.
        } else if full || damage.is_empty() {
            for c in fb_px.chunks_exact(2) {
                mix((c[0] as u64) << 32 | c[1] as u64);
            }
            if fb_px.len() % 2 == 1 {
                mix(fb_px[fb_px.len() - 1] as u64);
            }
        } else {
            let w = self.current_fb.w;
            let bounds = render::Rect::from_size(self.current_fb.w, self.current_fb.h);
            for r in &damage {
                let r = r.intersect(&bounds);
                if r.is_empty() {
                    continue;
                }
                for y in r.y0..r.y1 {
                    let row = y as usize * w;
                    let px = &fb_px[row + r.x0 as usize..row + r.x1 as usize];
                    for c in px.chunks_exact(2) {
                        mix((c[0] as u64) << 32 | c[1] as u64);
                    }
                    if px.len() % 2 == 1 {
                        mix(px[px.len() - 1] as u64);
                    }
                }
            }
        }
        let t_hash = Instant::now();
        self.timing.hit(|t| &mut t.hash, t_hash - t2);
        if full {
            self.timing.full_hash += (t_hash - t2).as_secs_f64() * 1e3;
        }
        // B61b: with the drops in the shared buffer, a frame where only they
        // moved still has to be exported (the paint log makes that cheap).
        let changed =
            known_change || hash != self.prev_hash || (paint_overlay_in_shm && wx_overlay);
        if changed {
            self.idle_streak = 0;
            self.frame_seq += 1;
            if known_change {
                // The skipped hash left no value to compare the next frame
                // against: make the next one re-hash and re-export.
                self.prev_hash = 0;
            } else {
                self.prev_hash = hash;
            }
        } else {
            self.idle_streak = self.idle_streak.saturating_add(1);
        }

        // The Plasma-plugin path (D49) shows the frames, so there is nothing to
        // attach to a layer surface: skipping it removes the whole shm copy
        // (6 ms a frame at 3840x1080 in the VM) from the path. B63: the frames
        // may go to one shared-memory file per head instead of PPM files.
        if self.frame_dir.is_some() || self.shm_dir.is_some() {
            if changed {
                let te = Instant::now();
                self.export_plugin_frame(&damage, full);
                let d = te.elapsed();
                self.timing.hit(|t| &mut t.export, d);
                if full {
                    self.timing.full_export += d.as_secs_f64() * 1e3;
                }
            }
            for head in &mut self.heads {
                head.last_drawn_seq = self.frame_seq;
            }
            self.needs_composite = false;
            if changed {
                self.stats_collector
                    .record_commit(t1 - t0, t2 - t1, Duration::ZERO);
            }
            self.timing.report();
            return;
        }

        let mut any_presented = false;
        let t_present_start = Instant::now();

        // Windows: each monitor's window is a DIB the slice is written into
        // directly; GDI then copies only the damaged rectangles to the screen.
        #[cfg(windows)]
        for head in &mut self.heads {
            if !head.configured || head.last_drawn_seq >= self.frame_seq {
                continue;
            }
            let Some(surface) = head.surface.as_mut() else {
                head.last_drawn_seq = self.frame_seq;
                continue;
            };
            let dst_px = surface.pixels_mut();
            let head_full = full || head.last_drawn_seq == 0 || damage.is_empty();
            let mut head_damage: Vec<render::Rect> = Vec::new();
            if head_full {
                slice::slice_into_output(
                    self.current_fb.px(),
                    layout.fb_w,
                    layout.fb_h,
                    layout.scale,
                    layout.min_x,
                    layout.min_y,
                    head.logical_pos.0,
                    head.logical_pos.1,
                    head.logical_size.0,
                    head.logical_size.1,
                    head.scale_factor,
                    dst_px,
                );
            } else {
                for r in &damage {
                    let d = slice::slice_into_output_rect(
                        self.current_fb.px(),
                        layout.fb_w,
                        layout.fb_h,
                        layout.scale,
                        layout.min_x,
                        layout.min_y,
                        head.logical_pos.0,
                        head.logical_pos.1,
                        head.logical_size.0,
                        head.logical_size.1,
                        head.scale_factor,
                        dst_px,
                        *r,
                    );
                    if !d.is_empty() {
                        head_damage.push(d);
                    }
                }
            }
            if !self.hidden {
                if head_full || head_damage.is_empty() {
                    surface.present_all();
                } else {
                    surface.present_rects(&head_damage);
                }
            }
            head.last_drawn_seq = self.frame_seq;
            any_presented = true;
        }

        #[cfg(target_os = "linux")]
        for head in &mut self.heads {
            if !head.configured || head.frame_pending || head.last_drawn_seq >= self.frame_seq {
                continue;
            }

            let Some(buffers) = &head.buffers else {
                continue;
            };

            if buffers.is_empty() {
                continue;
            }
            let (buf, canvas) = if let Some(c) = buffers[0].canvas(&mut self.slot_pool) {
                (&buffers[0], c)
            } else {
                buffers[0].deactivate().ok();
                match buffers[0].canvas(&mut self.slot_pool) {
                    Some(c) => (&buffers[0], c),
                    None => continue,
                }
            };

            let dst_px: &mut [u32] = bytemuck::cast_slice_mut(canvas);

            // B30: copy only the damaged rectangles; the rest of the buffer
            // already holds the previous frame (one buffer per head, so the
            // buffer's contents are exactly frame N-1).
            // A freshly allocated buffer holds garbage: copy it whole once.
            let head_full = full || head.last_drawn_seq == 0 || damage.is_empty();
            let mut head_damage: Vec<render::Rect> = Vec::new();
            if head_full {
                slice::slice_into_output(
                    self.current_fb.px(),
                    layout.fb_w,
                    layout.fb_h,
                    layout.scale,
                    layout.min_x,
                    layout.min_y,
                    head.logical_pos.0,
                    head.logical_pos.1,
                    head.logical_size.0,
                    head.logical_size.1,
                    head.scale_factor,
                    dst_px,
                );
            } else {
                for r in &damage {
                    let d = slice::slice_into_output_rect(
                        self.current_fb.px(),
                        layout.fb_w,
                        layout.fb_h,
                        layout.scale,
                        layout.min_x,
                        layout.min_y,
                        head.logical_pos.0,
                        head.logical_pos.1,
                        head.logical_size.0,
                        head.logical_size.1,
                        head.scale_factor,
                        dst_px,
                        *r,
                    );
                    if !d.is_empty() {
                        head_damage.push(d);
                    }
                }
            }

            match buf.attach_to(head.layer.wl_surface()) {
                Ok(()) => {
                    let px_w = (head.logical_size.0 * head.scale_factor as u32) as i32;
                    let px_h = (head.logical_size.1 * head.scale_factor as u32) as i32;
                    if head_full || head_damage.is_empty() {
                        head.layer.wl_surface().damage(
                            0,
                            0,
                            head.logical_size.0 as i32,
                            head.logical_size.1 as i32,
                        );
                        head.layer.wl_surface().damage_buffer(0, 0, px_w, px_h);
                    } else {
                        for d in &head_damage {
                            head.layer.wl_surface().damage_buffer(
                                d.x0,
                                d.y0,
                                (d.x1 - d.x0).max(1),
                                (d.y1 - d.y0).max(1),
                            );
                        }
                    }
                    head.layer
                        .wl_surface()
                        .frame(&self.qh, head.layer.wl_surface().clone());
                    head.frame_pending = true;
                    head.frame_requested_at = Some(Instant::now());
                    head.layer.commit();
                    head.last_drawn_seq = self.frame_seq;
                    any_presented = true;
                }
                Err(e) => {
                    eprintln!("attach_to failed on head '{}': {:?}", head.name, e);
                }
            }
        }

        let t3 = Instant::now();
        self.timing.hit(|t| &mut t.present, t3 - t_present_start);

        // The Plasma plugin path (D49): a fresh picture goes to the frame
        // directory whether or not a layer surface took it.
        if changed {
            let te = Instant::now();
            self.export_plugin_frame(&damage, full);
            let d = te.elapsed();
            self.timing.hit(|t| &mut t.export, d);
        }

        if any_presented {
            self.stats_collector.record_commit(
                t1 - t0,
                t2 - t1,
                t3 - t_present_start,
            );
        }
        self.timing.report();
    }
}

#[cfg(target_os = "linux")]
impl CompositorHandler for WallpaperState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if let Some(head) = self.heads.iter_mut().find(|h| h.layer.wl_surface() == surface) {
            let f = new_factor.max(1);
            if head.scale_factor != f {
                log::info!("Head '{}' scale factor changed to {}", head.name, f);
                head.scale_factor = f;
                head.buffers = None;
                self.check_geometry_change();
            }
        }
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
        if let Some(head) = self.heads.iter_mut().find(|h| h.layer.wl_surface() == surface) {
            head.frame_pending = false;
            head.frame_requested_at = None;
            if let Some(buffers) = &head.buffers {
                if let Some(b) = buffers.first() {
                    b.deactivate().ok();
                }
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

#[cfg(target_os = "linux")]
impl OutputHandler for WallpaperState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        log::info!("OutputHandler: new output detected");
        self.create_head_for_output(output);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            if let Some(head) = self.heads.iter_mut().find(|h| h.wl_output == output) {
                if let Some(name) = info.name {
                    head.name = name;
                }
                if let Some(pos) = info.logical_position {
                    head.logical_pos = pos;
                } else {
                    head.logical_pos = info.location;
                }
                if let Some(size) = info.logical_size {
                    if size.0 > 0 && size.1 > 0 {
                        head.logical_size = (size.0 as u32, size.1 as u32);
                    }
                }
                head.scale_factor = info.scale_factor.max(1);
            }
        }
        self.check_geometry_change();
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        log::info!("OutputHandler: output destroyed");
        self.heads.retain(|h| h.wl_output != output);
        self.check_geometry_change();
    }
}

#[cfg(target_os = "linux")]
impl LayerShellHandler for WallpaperState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        log::info!("Layer surface closed");
        self.heads.retain(|h| h.layer != *layer);
        self.check_geometry_change();
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        if std::env::var("CITYLIVE_DEBUG").is_ok() {
            eprintln!("debug: configure {w}x{h}");
        }
        if let Some(head) = self.heads.iter_mut().find(|h| h.layer == *layer) {
            if w > 0 && h > 0 {
                head.logical_size = (w, h);
            }
            head.configured = true;
        }
        self.check_geometry_change();
    }
}

#[cfg(target_os = "linux")]
impl ShmHandler for WallpaperState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

#[cfg(target_os = "linux")]
impl Dispatch<zxdg_output_manager_v1::ZxdgOutputManagerV1, ()> for WallpaperState {
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

#[cfg(target_os = "linux")]
impl Dispatch<zxdg_output_v1::ZxdgOutputV1, wl_output::WlOutput> for WallpaperState {
    fn event(
        state: &mut Self,
        _proxy: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        output: &wl_output::WlOutput,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let Some(head) = state.heads.iter_mut().find(|h| &h.wl_output == output) {
            match event {
                zxdg_output_v1::Event::LogicalPosition { x, y } => {
                    head.logical_pos = (x, y);
                    state.check_geometry_change();
                }
                zxdg_output_v1::Event::LogicalSize { width, height } => {
                    if width > 0 && height > 0 {
                        head.logical_size = (width as u32, height as u32);
                        state.check_geometry_change();
                    }
                }
                zxdg_output_v1::Event::Name { name } => {
                    head.name = name;
                }
                zxdg_output_v1::Event::Done => {
                    state.check_geometry_change();
                }
                _ => {}
            }
        }
    }
}

#[cfg(target_os = "linux")]
delegate_compositor!(WallpaperState);
#[cfg(target_os = "linux")]
delegate_output!(WallpaperState);
#[cfg(target_os = "linux")]
delegate_shm!(WallpaperState);
#[cfg(target_os = "linux")]
delegate_layer!(WallpaperState);
#[cfg(target_os = "linux")]
delegate_registry!(WallpaperState);

#[cfg(target_os = "linux")]
impl ProvidesRegistryState for WallpaperState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

/// Real wall clock in ms (B45: the meteor slot machine keys on it, so
/// streaks animate even when the sim clock is pinned by `--time`).
pub fn wall_ms_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// What the idle sleep rule needs to know (B26c item 4). Kept as a pure
/// function so the rule is unit-tested without a compositor.
#[derive(Clone, Copy, Debug)]
pub struct IdleInputs {
    /// A composite or a frame callback is already queued.
    pub pending: bool,
    /// Sim time in seconds (only used to find the next planning step).
    pub sim_time: f64,
    /// Sim seconds per real second (pace × speed).
    pub pace_scale: f64,
    /// Time left until the next animation frame (D71: the display cadence).
    pub frame_wait: Duration,
    /// Something on screen moves, so the next animation frame matters.
    pub animating: bool,
    /// B45: a meteor streak may light within this long (0 = no night sky, or
    /// no slot due before the next planning step — sleep as usual). Waking
    /// for it is what makes showers visible on an otherwise quiet night.
    pub meteor_wait: Duration,
    /// B57: there are clouds on screen, so look again soon: they move a pixel
    /// at a time and D71 wants them smooth. Ignored by `quiet` — a still frame
    /// with clouds in it is a frame that is about to change.
    pub clouds_live: bool,
}

impl Default for IdleInputs {
    fn default() -> Self {
        Self {
            pending: false,
            sim_time: 0.0,
            pace_scale: 1.0,
            frame_wait: Duration::ZERO,
            animating: false,
            meteor_wait: Duration::ZERO,
            clouds_live: false,
        }
    }
}

/// How long the loop may sleep. Nothing pending and nothing animating means
/// sleep to the next planning step — not poll (B26c: the 500 ms poll cost ~0.9 %
/// of a core on the owner's desktop while drawing nothing).
pub fn idle_wait(i: IdleInputs) -> Duration {
    if i.pending {
        return i.frame_wait;
    }
    let step_offset = i.sim_time % sim::PLANNING_STEP;
    let remaining = (sim::PLANNING_STEP - step_offset).max(0.1);
    let step_wait = Duration::from_secs_f64((remaining / i.pace_scale.max(0.001)).clamp(0.016, 30.0));
    // B57: clouds drift at the frame rate, so a quiet scene with weather in it
    // still wakes every frame cap (the tick itself is a few microseconds; the
    // compositing only happens when a cloud actually stepped a pixel).
    let step_wait = if i.clouds_live {
        step_wait.min(i.frame_wait.max(Duration::from_millis(16)))
    } else {
        step_wait
    };
    // B45: a streak is due within `meteor_wait` — cap the sleep by it, even
    // when the scene has gone quiet (one slot check every ~20 s at night is
    // 0.05 wakes/s, well inside the budget).
    let step_wait = if i.meteor_wait > Duration::ZERO {
        step_wait.min(i.meteor_wait)
    } else {
        step_wait
    };
    if i.animating {
        // D71: everything that moves runs at the configured fps, so wake for
        // the next frame — but never sleep past the planning step.
        step_wait.min(i.frame_wait)
    } else {
        // Nothing on screen moves: sleep to the next planning step, which is
        // the next moment anything *can* move.
        step_wait
    }
}

/// Run the live wallpaper until exit (by signal or elapsed seconds).
#[cfg(target_os = "linux")]
pub fn run(config: WallpaperConfig) -> Result<RunStats, Box<dyn std::error::Error>> {
    setup_signal_handlers();

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let mut event_loop: EventLoop<WallpaperState> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue).insert(loop_handle)?;

    let compositor_state =
        CompositorState::bind(&globals, &qh).map_err(|e| format!("Compositor bind error: {e}"))?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).map_err(|e| format!("LayerShell bind error: {e}"))?;
    let xdg_output_manager = globals
        .bind::<zxdg_output_manager_v1::ZxdgOutputManagerV1, _, _>(&qh, 1..=3, ())
        .ok();
    let shm = Shm::bind(&globals, &qh).map_err(|e| format!("Shm bind error: {e}"))?;
    let output_state = OutputState::new(&globals, &qh);
    let registry_state = RegistryState::new(&globals);

    let initial_outputs: Vec<wl_output::WlOutput> = output_state.outputs().collect();

    let mut state = WallpaperState::new(
        config.clone(),
        registry_state,
        output_state,
        compositor_state,
        layer_shell,
        xdg_output_manager,
        shm,
        qh,
    );

    if std::env::var("CITYLIVE_DEBUG").is_ok() {
        eprintln!("debug: {} initial output(s)", initial_outputs.len());
    }
    for output in initial_outputs {
        state.create_head_for_output(output);
    }
    if std::env::var("CITYLIVE_DEBUG").is_ok() {
        eprintln!(
            "debug: {} head(s) after setup, configured {}",
            state.heads.len(),
            state.heads.iter().filter(|h| h.configured).count()
        );
    }

    run_loop(
        config,
        state,
        &mut |state, timeout| {
            event_loop
                .dispatch(Some(timeout), state)
                .map_err(|e| format!("{e:?}"))
        },
        &mut || {
            conn.flush().ok();
        },
    )
}

/// The platform-independent main loop: builds the world once the outputs have
/// settled, then steps the sim, the weather, the feeds and the animation, and
/// presents. `dispatch` waits up to the timeout for the platform's events
/// (Wayland events, Windows messages) and handles them; `flush` runs after a
/// frame was presented.
pub fn run_loop(
    config: WallpaperConfig,
    mut state: WallpaperState,
    dispatch: &mut dyn FnMut(&mut WallpaperState, Duration) -> Result<(), String>,
    flush: &mut dyn FnMut(),
) -> Result<RunStats, Box<dyn std::error::Error>> {
    // Let the outputs announce themselves and then let the set **settle** before
    // a world exists (B26c item 5): a city built for a half-announced layout was
    // saved as city.sav.2560x720 and then rebuilt at 3520x720, which flickered
    // on every start.
    {
        let deadline = Instant::now() + Duration::from_millis(1500);
        while state.current_union.is_none() && Instant::now() < deadline {
            let wait = state
                .layout_settle_in()
                .unwrap_or(Duration::from_millis(50));
            dispatch(&mut state, wait + Duration::from_millis(1)).ok();
            state.settle_layout();
        }
    }

    // D71: one cadence for everything that moves — the configured fps, no
    // per-layer caps. The plugin path runs the same rate: strips make a frame
    // cheap to hand over, and a wallpaper that stutters is not worth the CPU
    // it saves.
    let frame_interval = if state.config.fps == 0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(1.0 / state.config.fps as f64)
    };

    let started = Instant::now();
    let mut next_frame_time = Instant::now();
    // B84: `CITYLIVE_MEM=1` also prints the table every 10 s (a steady reading).
    let sampler_path = sampler::path_from_env();
    let mut sampler_armed = false;
    let mem_every: f64 = if std::env::var("CITYLIVE_MEM").is_ok() {
        10.0
    } else {
        0.0
    };
    let mut next_mem_report = Instant::now() + Duration::from_secs_f64(mem_every.max(1.0));

    println!(
        "wallpaper starting: scene={:?} seed={} pace={} speed={}x layer={:?} fps_cap={} duration={:.1}s",
        config.mode,
        config.seed,
        config.pace.name(),
        config.speed,
        config.layer,
        config.fps,
        config.seconds
    );

    if std::env::var("CITYLIVE_MEM").is_ok() {
        state.print_mem_report();
    }

    loop {
        let t_loop = Instant::now();
        if sampler_path.is_some() && !sampler_armed && state.started_at.elapsed() > Duration::from_secs(20) {
            sampler_armed = true;
            sampler::arm();
        }
        let c_loop = thread_cpu_ms();
        if TERMINATING.load(Ordering::Relaxed) {
            break;
        }
        if config.seconds > 0.0 && started.elapsed().as_secs_f64() >= config.seconds {
            break;
        }

        // B37: the settings file is watched; a change is applied live.
        state.poll_settings();
        // B37: tray actions.
        while let Some(act) = state
            .tray_rx
            .as_ref()
            .and_then(|rx| rx.lock().ok().and_then(|g| g.try_recv().ok()))
        {
            match act {
                TrayAction::TogglePause => {
                    state.animation_paused = !state.animation_paused;
                    println!(
                        "tray: animation {}",
                        if state.animation_paused { "paused" } else { "running" }
                    );
                    state.needs_composite = true;
                }
                TrayAction::Timelapse => {
                    state.timelapse_until = Some(Instant::now() + Duration::from_secs(60));
                    println!("tray: 60 s time-lapse");
                }
                TrayAction::WeatherNow => {
                    let w = state.wx_last.clone();
                    println!(
                        "tray: weather now: {:.0} F (feels {:.0}), cloud {:.0} %, wind {:.0} km/h, rain {:.1} mm",
                        w.temp, w.feels, w.cloud, w.wind, w.precip
                    );
                }
                TrayAction::OpenSettings => {
                    // B38: installs put the settings app next to the wallpaper,
                    // and the wallpaper is often started from a unit whose PATH
                    // does not include that folder — try the sibling first.
                    #[cfg(windows)]
                    const SETTINGS_BIN: &str = "citylive-settings.exe";
                    #[cfg(not(windows))]
                    const SETTINGS_BIN: &str = "citylive-iso-settings";
                    let sibling = std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|d| d.join(SETTINGS_BIN)));
                    let mut started = false;
                    if let Some(p) = sibling {
                        if p.exists() {
                            started = std::process::Command::new(&p).spawn().is_ok();
                        }
                    }
                    if !started {
                        match std::process::Command::new(SETTINGS_BIN).spawn() {
                            Ok(_) => started = true,
                            Err(e) => eprintln!("tray: cannot start {SETTINGS_BIN}: {e}"),
                        }
                    }
                    if started {
                        println!("tray: settings window opened");
                    }
                }
                TrayAction::NewCity => {
                    // Keep the old city as a backup, then start a new world.
                    if let Some(sim) = &state.sim {
                        let bak = state.config.save_path.with_extension(format!(
                            "bak-{}.sav",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0)
                        ));
                        let _ = sim.save(&bak);
                        println!("tray: old city backed up to {}", bak.display());
                    }
                    let _ = std::fs::remove_file(&state.config.save_path);
                    if let Some(sim) = &state.sim {
                        let _ = std::fs::remove_file(sim::timelapse::path_for(&world_save_path(
                            &state.config.save_path,
                            sim.map.w,
                            sim.map.h,
                        )));
                    }
                    state.tl_recorder = None;
                    state.tl_day = None;
                    state.sim = None;
                    state.current_union = None;
                    state.layout_settle = Some(Instant::now());
                }
                TrayAction::OpenChronicle => {
                    if let Some(sim) = &state.sim {
                        let p = live_chronicle_path(&state.config.save_path);
                        match write_chronicle(sim, &p) {
                            Ok(()) => {
                                println!("tray: chronicle written to {}", p.display());
                                xdg_open(&p);
                            }
                            Err(e) => eprintln!("tray: cannot write the chronicle ({e})"),
                        }
                    }
                }
                TrayAction::OpenWorlds => {
                    let d = worlds_dir(&state.config.save_path);
                    let _ = std::fs::create_dir_all(&d);
                    xdg_open(&d);
                }
                TrayAction::Quit => {
                    TERMINATING.store(true, Ordering::Relaxed);
                }
            }
        }
        // B37: a time-lapse runs the sim 60x for a minute, then back.
        let timelapsing = state
            .timelapse_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false);
        if !timelapsing {
            state.timelapse_until = None;
        }
        let speed_now = if timelapsing { 60.0 } else { 1.0 };

        // Wait for the output set to stop moving before building a world
        // (B26c item 5).
        state.settle_layout();

        // Periodic 5-minute save
        if state.last_save_time.elapsed() >= Duration::from_secs(300) {
            if let Some(sim) = &state.sim {
                let _ = sim.save(&config.save_path);
                if state.current_union.is_some() {
                    let lp = world_save_path(&config.save_path, sim.map.w, sim.map.h);
                    let _ = sim.save(&lp);
                }
                log::info!("Saved city state to {}", config.save_path.display());
            }
            state.last_save_time = Instant::now();
        }

        // Simulation update
        let now = Instant::now();
        let real_dt = now.duration_since(state.last_sim_step).as_secs_f64();
        state.last_sim_step = now;

        if let Some(sim) = &mut state.sim {
            let ts = Instant::now();
            // B53 item 3: a calmer pace while the session is idle (1.0 when off).
            let idle_mult = idle::quiet_factor(
                state.idle.is_some(),
                state.idle.as_ref().and_then(|w| w.idle_for()),
            );
            let sim_dt = real_dt * state.config.speed * speed_now * idle_mult;
            sim.advance(sim_dt);
            let d = ts.elapsed();
            state.timing.hit(|t| &mut t.sim_step, d);
            let changes = sim.take_changes();
            if !changes.is_empty() {
                for (tx, ty) in changes {
                    state.scene.set_tile(tx, ty, sim.map.at(tx, ty));
                }
                state.update_city();
                // No unconditional composite: `has_dirty` below decides, and a
                // change to a tile outside the frame dirties nothing.
            }
        }

        // B50: the end of the world. When the morph is over the sim hands over
        // the next world; the old save goes (the old world is over) and a new
        // one is founded with the same machinery a "new city" uses.
        if let Some(rb) = state.sim.as_mut().and_then(|s| s.rebirth()) {
            println!("world ended: {}", rb.ended);
            if let Some(sim) = &state.sim {
                match archive_world(sim, &state.config.save_path) {
                    Ok(p) => {
                        println!("world archived: {}", p.display());
                        match archive_chronicle(sim, &p, &state.config.save_path) {
                            Ok(d) => println!("chronicle written: {}", d.display()),
                            Err(e) => eprintln!("chronicle not written ({e})"),
                        }
                        // The archive is the move: the live saves are now its copies.
                        let _ = std::fs::remove_file(&state.config.save_path);
                        let _ = std::fs::remove_file(world_save_path(&state.config.save_path, sim.map.w, sim.map.h));
                    }
                    Err(e) => {
                        // Never lose a world: keep the live save and do not found
                        // the next one until the archive can be written.
                        eprintln!("world archive failed ({e}); keeping the save");
                    }
                }
            }
            state.rebirth = Some(rb);
            state.sim = None;
            state.tl_recorder = None;
            state.tl_day = None;
            state.tl_final = false;
            state.current_union = None;
            state.layout_settle = Some(Instant::now());
        }
        // B50: a slow ending (a warning that changes by the hour) still has
        // to reach the scene when nothing else is asking for frames.
        if now.duration_since(state.last_tray_refresh) >= Duration::from_secs(5) {
            state.ending_tick();
        }

        // B48a: the tray's picture of the city — the name, the headcount and
        // era line, and the latest story lines — refreshed a few times a
        // minute. This is where the wallpaper's knowledge meets the sim: a
        // weather extreme or a calendar holiday is noted here, once each, into
        // the feed (the sim only knows its own sim clock).
        if now.duration_since(state.last_tray_refresh) >= Duration::from_secs(5) {
            state.last_tray_refresh = now;
            // The holiday (and then the feed) is read before the sim is
            // borrowed: the sim's note takes the mutable borrow and the
            // holiday look reads the state.
            state.sync_idle();
            state.gameday_tick();
            let holiday = if state.config.holidays {
                holiday_story_index(
                    holiday_look(
                        state.config.time_override.as_deref(),
                        state.config.date_override.as_deref(),
                    )
                    .holiday,
                )
            } else {
                None
            };
            if let Some(sim) = state.sim.as_mut() {
                let f = state.wx_last.flags();
                sim.note_weather(state.wx_last.temp, f.thunder, f.snow);
                // B75: umbrellas in the rain (or the snow).
                sim.set_wet(f.rain || f.drizzle || f.snow || f.thunder);
                // B79: the sea's mood: the air temperature decides who swims
                // and how many boats are out; a thunderstorm or a gale clears
                // the water.
                let gale = state.wx_last.wind.max(state.wx_last.gust * 0.7);
                let storm = if f.thunder {
                    1.0
                } else if gale >= 55.0 {
                    0.8
                } else if f.rain {
                    0.3
                } else {
                    0.0
                };
                sim.set_sea_mood(state.wx_last.temp, storm);
                // B76: the winter service comes out when snow is lying.
                sim.set_snow(state.scene.weather().snow_cover);
                if let Some(i) = holiday {
                    sim.note_holiday(i);
                }
            }
            // The feed in the log, newest line when it changes (the rig's record
            // of an ending: warning lines, the last line, the new settlers).
            if let Some(sim) = state.sim.as_ref() {
                if let Some(l) = sim.story_latest(1).first() {
                    if l.text != state.last_feed_line {
                        state.last_feed_line = l.text.clone();
                        println!("feed [{} d {:.2}]: {}", sim.city_name, sim.days(), l.text);
                    }
                }
            }
            if let (Some(display), Some(sim)) = (&state.tray_display, state.sim.as_ref()) {
                let mut status =
                    sim.tray_status(&state.settings.city_name, state.settings.just_watch);
                status.gameday = state.game_now.tray_line().unwrap_or_default();
                let changed = display
                    .read()
                    .ok()
                    .map(|g| *g != status)
                    .unwrap_or(true);
                if changed {
                    if let Ok(mut w) = display.write() {
                        *w = status;
                    }
                }
            }
        }

        // Walking workers: the crews are re-posed in `animate` with everything
        // else (D71), so all this needs to know is whether a site is active.
        let building = state
            .sim
            .as_ref()
            .map(|s| {
                s.lots.iter().any(|l| {
                    matches!(
                        l.stage,
                        sim::Stage::Foundation
                            | sim::Stage::Framing
                            | sim::Stage::Scaffold
                            | sim::Stage::Roofed
                    )
                })
            })
            .unwrap_or(false);
        state.building = building;
        state.settlers_live = state
            .sim
            .as_ref()
            .map(|s| s.settlers.is_active() || !s.settlers.party.is_empty())
            .unwrap_or(false);

        if state.scene.rain_on() || !state.scene.life.is_empty() {
            state.scene.step(real_dt as f32);
            state.needs_composite = true;
        }
        if state.scene.has_dirty() {
            state.needs_composite = true;
        }

        // B34: the first second and a half goes on building the world, the sky
        // and the building sprites. Running the animation ticks during it only
        // commits frames nobody sees settle (the startup burst was 50 frames and
        // 12.9 % of a core in the first 10 s), so the animated layers wait until
        // the world is up.
        let warming = state.started_at.elapsed() < Duration::from_millis(1500);
        // B37: "pause animation" stops asking for frames; the sim keeps running
        // (D15 never pauses the city).
        // B95: nothing on screen (a full-screen app in front, a locked or
        // sleeping session) is the same as paused for the animation, and nothing
        // is composited or presented either.
        let paused = state.animation_paused || state.hidden;

        // Weather: once a second is plenty for a city sky, and a change of wet
        // roads, snow cover or cloud re-rasterises the ground once (P6).
        if now >= state.wx_next_tick {
            state.wx_next_tick = now + Duration::from_secs(1);
            let t_wx = Instant::now();
            let wx_changed = state.weather_tick(1.0);
            stall("weather_tick", t_wx);
            if wx_changed {
                state.needs_composite = true;
            }
        }
        #[cfg(feature = "net")]
        if now >= state.wx_next_fetch {
            state.wx_next_fetch = now + Duration::from_secs(15 * 60);
            if state.config.weather.as_deref().unwrap_or("live") == "live" {
                let (lat, lon) = (state.config.lat, state.config.lon);
                let slot = state.wx_shared.clone();
                std::thread::spawn(move || {
                    let client = world::UreqClient::new();
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    let cache = plat::cache_dir().join("open-meteo.json");
                    let wx = world::weather::fetch(&client, lat, lon, now_ms, &cache);
                    if let Ok(mut g) = slot.lock() {
                        *g = Some(wx);
                    }
                });
            }
        }
        #[cfg(feature = "net")]
        if let Ok(mut g) = state.wx_shared.lock() {
            if let Some(wx) = g.take() {
                state.wx_last = wx;
                state.needs_composite = true;
            }
        }

        // B96: find the place by the OS location service or an IP lookup, off
        // the render thread, at start and then daily (a failure is retried after
        // ten minutes, which also covers the network coming back).
        if now >= state.loc_next {
            state.loc_next = now + Duration::from_secs(locate::RETRY_SECS);
            let auto = state.settings.location.auto && !state.config.location_forced;
            let cache = locate::cache_path();
            let due = locate::refresh_due(auto, locate::load_cache(&cache).as_ref(), locate::now_secs());
            if due && !state.loc_busy.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let slot = state.loc_shared.clone();
                let busy = state.loc_busy.clone();
                std::thread::spawn(move || {
                    let os = locate::system_locator();
                    #[cfg(feature = "net")]
                    let client = world::UreqClient::new();
                    #[cfg(feature = "net")]
                    let http: Option<&dyn world::HttpClient> = Some(&client);
                    #[cfg(not(feature = "net"))]
                    let http: Option<&dyn world::HttpClient> = None;
                    let r = locate::resolve(
                        os.as_deref().map(|o| o as &dyn locate::OsLocator),
                        http,
                        &cache,
                        locate::now_secs(),
                    );
                    if let Ok(mut g) = slot.lock() {
                        *g = Some(r);
                    }
                    busy.store(false, std::sync::atomic::Ordering::SeqCst);
                });
            }
        }
        // B97: look for a newer release, off the render thread. Never while the
        // wallpaper is hidden behind a full-screen app or a locked session.
        #[cfg(feature = "net")]
        if now >= state.upd_next {
            if state.hidden || !state.settings.auto_update {
                state.upd_next = now + Duration::from_secs(60);
            } else {
                state.upd_next = now + Duration::from_secs(update::POLL_SECS);
                if !state.upd_busy.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    let busy = state.upd_busy.clone();
                    std::thread::spawn(move || {
                        let out = update::run_default(false);
                        if out != update::Outcome::Throttled {
                            println!("update: {}", out.describe());
                        }
                        busy.store(false, std::sync::atomic::Ordering::SeqCst);
                    });
                }
            }
        }
        let found = state.loc_shared.lock().ok().and_then(|mut g| g.take());
        if let Some(r) = found {
            if state.settings.location.auto && !state.config.location_forced {
                println!("location: {} ({}) {:.4}, {:.4}", r.name, r.describe(), r.lat, r.lon);
                state.set_place(r.lat, r.lon);
            }
        }

        // B47: the feeds. Each one fires on its own bucket (25 s for the ISS,
        // 90 s for aircraft, 30 min for Kp) on its own thread, so this is a
        // couple of `Instant` compares on the frames in between.
        {
            let gates = live::Gates {
                aurora: state.settings.aurora,
                iss: state.settings.iss,
                flights: state.settings.flights,
            };
            state
                .sky_live
                .poll(gates, state.config.lat, state.config.lon, now);
        }

        // Wildlife that is not out yet cannot ask for a frame, so it gets a
        // 1 Hz probe: the moment an animal's schedule brings it out, the layer
        // goes live and joins the frame cadence.
        if !state.wl_live && now >= state.wl_next {
            state.wl_next = now + Duration::from_secs(1);
            state.wildlife_step(1.0);
        }

        let frame_due = now >= next_frame_time;
        // B101: how late this frame starts, and when its work began, for the
        // load governor.
        let frame_late = now.saturating_duration_since(next_frame_time);
        let frame_t0 = Instant::now();
        let mut frame_worked = false;

        // D71/B54: one animation step for everything that moves, at the
        // configured frame rate, with the same real `dt` for every layer — so
        // walkers' legs, wheels, waves, rain and wildlife all run in step and
        // smoothly. `animate` returns true when a pixel changed.
        if frame_due && !warming && !paused && state.animation_live() {
            let dt = now
                .duration_since(state.last_anim)
                .as_secs_f32()
                .clamp(0.0, 0.25);
            state.last_anim = now;
            frame_worked = true;
            let t_an = Instant::now();
            let c_an = thread_cpu_ms();
            let animated = state.animate(dt);
            state.timing.cpu_animate += thread_cpu_ms() - c_an;
            if animated {
                state.needs_composite = true;
            }
            stall("animate", t_an);
            let mut cadence = frame_interval;
            if state.anim_quiet > 15 {
                // Nothing on screen has changed a pixel for half a second.
                // Keep looking, but at 4 Hz, and never sleep on a figure that
                // is about to walk into the frame.
                cadence = cadence.max(Duration::from_millis(250));
            }
            next_frame_time = if state.config.fps > 0 {
                // B67: a deadline on a fixed grid, not "a frame after whenever
                // this one happened to start". The old rule added the wake-up
                // latency to every period (33.4 ms measured for a 33.33 ms
                // frame), so the writer slid against the display's vsync and
                // every few seconds a frame was held for three refreshes and
                // the next for one. Falling behind (a long frame) restarts the
                // grid from now rather than firing a burst to catch up.
                let on_grid = next_frame_time + cadence;
                if cadence == frame_interval && on_grid > now {
                    on_grid
                } else {
                    now + cadence
                }
            } else {
                now
            };
        }

        // Check if any head needs drawing
        let any_head_stale = state
            .heads
            .iter()
            .any(|h| h.configured && !h.frame_pending && h.last_drawn_seq < state.frame_seq);

        let should_render = (state.needs_composite || any_head_stale) && frame_due && !state.hidden;

        if should_render {
            let t_rp = Instant::now();
            let c_rp = thread_cpu_ms();
            state.render_and_present();
            state.timing.cpu_render += thread_cpu_ms() - c_rp;
            stall("render_and_present", t_rp);
            flush();
        }
        if frame_worked && !state.hidden && state.config.fps > 0 && state.started_at.elapsed() > Duration::from_secs(12) {
            let busy = frame_t0.elapsed();
            if let Some((from, to)) = state.load.frame(busy, frame_late, frame_interval) {
                eprintln!(
                    "load: frames are missing their deadline ({:.1} ms of work, {:.1} ms late): level {} -> {} ({})",
                    busy.as_secs_f64() * 1e3,
                    frame_late.as_secs_f64() * 1e3,
                    from,
                    to,
                    state.load.describe()
                );
            }
        }

        // Sleep until the next thing that can change a pixel (B26c item 4):
        // a composite, a frame callback, the next sim planning step, or the
        // next animation tick — and nothing else. Sleeping on a 500 ms poll cost
        // the owner ~0.9 % of a core with nothing to draw.
        if let Some(d) = state.layout_settle_in() {
            dispatch(&mut state, d + Duration::from_millis(1)).ok();
            continue;
        }
        // A composite that changed nothing means the figures cannot be seen
        // (occluded, or creeping within one tile): stop waking for them and
        // sleep to the next planning step instead.
        let animating = state.animation_live();
        // B45: the next meteor slot can light a streak any moment — wake for
        // it (and only it) while the night sky is out, so showers show up even
        // on a quiet night. `ms_sky` carries the sim/sky clock (a pinned
        // `--time` still gets its date's shower windows).
        let meteor_wait = if state.star_fade >= 0.3 && state.scene.sky_height() > 0 {
            let due = world::astro::next_meteor_due(wall_ms_now(), state.night.ms_sky());
            Duration::from_millis(due as u64).saturating_add(Duration::from_millis(30))
        } else {
            Duration::ZERO
        };
        let timeout = if config.mode == SceneMode::Sim && state.sim.is_some() {
            let sim = state.sim.as_ref().unwrap();
            idle_wait(IdleInputs {
                pending: state.needs_composite || any_head_stale,
                sim_time: sim.time,
                pace_scale: sim.pace.scale() * state.config.speed.max(0.001),
                frame_wait: next_frame_time.saturating_duration_since(Instant::now()),
                animating,
                meteor_wait,
                clouds_live: state.scene.clouds_live() && !paused && !warming,
            })
        } else if state.needs_composite || any_head_stale {
            next_frame_time.saturating_duration_since(Instant::now())
        } else if config.mode == SceneMode::Empty {
            Duration::from_millis(500)
        } else {
            next_frame_time.saturating_duration_since(Instant::now())
        };

        // Hidden: one look a second (is it shown again yet?), nothing else.
        let timeout = if state.hidden {
            Duration::from_millis(1000)
        } else {
            timeout
        };
        let c_dp = thread_cpu_ms();
        let dispatched = dispatch(&mut state, timeout);
        state.timing.cpu_dispatch += thread_cpu_ms() - c_dp;
        if let Err(e) = dispatched {
            if TERMINATING.load(Ordering::Relaxed) {
                break;
            }
            eprintln!("event loop error: {:?}", e);
        }

        let d = t_loop.elapsed();
        state.timing.hit(|t| &mut t.rest, d);
        state.timing.cpu_loop += thread_cpu_ms() - c_loop;
        state.stats_collector.maybe_report();
        // B84: with `CITYLIVE_MEM=1`, the memory table is printed every 10 s as
        // well as at start and exit, so a reading is the *steady* footprint and
        // not the startup one (or the one taken while the process is closing).
        if mem_every > 0.0 && next_mem_report <= Instant::now() {
            state.print_mem_report();
            next_mem_report = Instant::now() + Duration::from_secs_f64(mem_every);
        }    }

    // Save on exit
    if let Some(sim) = &state.sim {
        let _ = sim.save(&config.save_path);
        if state.current_union.is_some() {
            let lp = world_save_path(&config.save_path, sim.map.w, sim.map.h);
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

    sampler::dump(&sampler_path);
    let stats = state.stats_collector.finish();
    if std::env::var("CITYLIVE_MEM").is_ok() {
        state.print_mem_report();
    }
    if config.stats {
        println!(
            "RESULT mode=wallpaper scene={:?} seed={} pace={} speed={}x layer={:?} fps_cap={} rain={} frames={} (committed={}) seconds={:.2} fps_avg={:.2} cpu_pct_one_core={:.2} cpu_seconds={:.2} rss_peak_kb={} sim_ms={:.3} composite_ms={:.3} present_ms={:.3}",
            config.mode,
            config.seed,
            config.pace.name(),
            config.speed,
            config.layer,
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
        let (p50, p99, max) = state.stats_collector.comp_percentiles();
        println!("COMPOSITE p50={p50:.2} p99={p99:.2} max={max:.2} ms over {} frames", state.stats_collector.all_comp.len());
    }

    Ok(stats)
}

#[cfg(test)]
mod idle_tests {
    use super::*;

    /// B26c item 4 / D71: with nothing on screen moving the loop sleeps to the
    /// next planning step; while anything moves it wakes for the next frame.
    #[test]
    fn idle_wakes_are_rare() {
        // Month pace, mid-step, nothing to draw: up to 20 s of sleep.
        let w = idle_wait(IdleInputs {
            sim_time: 86400.0 + 7.5,
            pace_scale: 1.0,
            ..IdleInputs::default()
        });
        assert!(w.as_secs_f64() >= 12.0, "idle sleep was only {w:?}");
        // D71: something is moving, so the loop wakes for the next frame at
        // the configured rate (30 fps) and no more.
        let w = idle_wait(IdleInputs {
            sim_time: 86400.0 + 7.5,
            pace_scale: 1.0,
            animating: true,
            frame_wait: Duration::from_millis(33),
            ..IdleInputs::default()
        });
        assert_eq!(w, Duration::from_millis(33));
        // At week pace (4x) the step is 5 s of real time: still no poll.
        let w = idle_wait(IdleInputs {
            sim_time: 0.0,
            pace_scale: 4.0,
            ..IdleInputs::default()
        });
        assert!(w.as_secs_f64() >= 4.9 && w.as_secs_f64() <= 5.01, "{w:?}");
        // A pending composite is limited by the frame cap, not by the step.
        let w = idle_wait(IdleInputs {
            pending: true,
            frame_wait: Duration::from_millis(8),
            ..IdleInputs::default()
        });
        assert_eq!(w, Duration::from_millis(8));
        // Animating never sleeps past the planning step, even with a long
        // frame wait (a paused wallpaper).
        let w = idle_wait(IdleInputs {
            sim_time: 86400.0 + 7.5,
            pace_scale: 1.0,
            animating: true,
            frame_wait: Duration::from_secs(30),
            ..IdleInputs::default()
        });
        assert!(w.as_secs_f64() >= 12.0 && w.as_secs_f64() <= 12.6, "{w:?}");
        // B45: a meteor slot due soon caps the sleep — even on a quiet night
        // with nothing else on screen, the streak must not be missed.
        let w = idle_wait(IdleInputs {
            sim_time: 86400.0 + 7.5,
            pace_scale: 1.0,
            meteor_wait: Duration::from_millis(2_500),
            ..IdleInputs::default()
        });
        assert_eq!(w, Duration::from_millis(2_500), "{w:?}");
        // A far-off slot does not shorten a long sleep.
        let w = idle_wait(IdleInputs {
            sim_time: 86400.0 + 7.5,
            pace_scale: 1.0,
            meteor_wait: Duration::from_secs(25),
            ..IdleInputs::default()
        });
        assert!(w.as_secs_f64() >= 12.0, "{w:?}");
    }

    /// The wallpaper's own loop needs a Wayland session, so the idle cost is
    /// measured here: the real `idle_wait` rule, the real per-wake work at
    /// 3520x720 on a day-30 city, for a few seconds, with the CPU and RSS of
    /// this process read from /proc.
    ///
    /// `cargo test --release -p present -- --ignored --nocapture idle_bench`
    #[test]
    #[ignore = "timing: runs for ~3 s"]
    fn idle_bench() {
        use std::time::Instant;
        let (w, h) = (3520usize, 720usize);
        let vp = map::world_for_viewport(w as i32, h as i32);
        let focus = map::screen_to_tile(&vp, w as f32 * 0.5, h as f32 * 0.5);
        let mut sim = sim::Sim::with_focus(7, sim::Pace::Month, vp.w, vp.h, focus);
        sim.run_to(30.0 * 86400.0);
        let mut scene = Scene::from_map_with_sky(
            sim.map.clone(),
            SceneConfig {
                width: w,
                height: h,
                seed: 7,
                mode: SceneMode::Sim,
                cars: 0,
                peds: 0,
                buildings: 0,
                rain: false,
            },
            0.30,
        );
        let objects: Vec<render::CityObj> = Vec::new();
        scene.set_city(objects);
        // Night: water highlights are faded, so nothing animates (the worst
        // case for the old 500 ms poll and the best case for this one).
        scene.set_water_brightness(0.02);
        let mut fb = Framebuffer::new(w, h);
        scene.composite(&mut fb);

        // (1) The wake body on its own: what one idle wake costs when nothing
        // has changed. Timed over 200 iterations, no sleep, no composites.
        scene.composite(&mut fb);
        scene.flush_dirty();
        let t_body = Instant::now();
        for _ in 0..200 {
            sim.advance(0.0005);
            let _ = sim.take_changes();
            let _ = scene.has_dirty();
        }
        let body_us = t_body.elapsed().as_secs_f64() * 1e6 / 200.0;

        // (2) The duty cycle: how often the loop actually wakes with nothing to
        // draw (the old code woke twice a second).
        let cpu0 = crate::procstat::process_cpu_ticks().unwrap_or(0);
        let t0 = Instant::now();
        let secs = 10.0f64;
        let mut wakes = 0u32;
        let mut composites = 0u32;
        let mut last_walk = Instant::now();
        while t0.elapsed().as_secs_f64() < secs {
            wakes += 1;
            sim.advance(0.0005);
            let changes = sim.take_changes();
            if !changes.is_empty() {
                for (tx, ty) in changes {
                    scene.set_tile(tx, ty, sim.map.at(tx, ty));
                }
            }
            if last_walk.elapsed() >= std::time::Duration::from_millis(166) {
                last_walk = Instant::now();
                let ws = figures(&sim);
                let changed = ws.as_slice() != scene.people_snapshot();
                let n = ws.len();
                scene.set_people(&ws, sim.smooth_time() as f32);
                if changed && n > 0 {
                    scene.composite(&mut fb);
                    composites += 1;
                }
            }
            if scene.has_dirty() {
                scene.composite(&mut fb);
                composites += 1;
            }
            let wait = idle_wait(IdleInputs {
                pending: false,
                sim_time: sim.time,
                pace_scale: 1.0,
                frame_wait: std::time::Duration::ZERO,
                animating: scene.people_count() > 0,
                meteor_wait: Duration::ZERO,
                clouds_live: scene.clouds_live(),
            });
            std::thread::sleep(wait.min(std::time::Duration::from_secs_f64(
                (secs - t0.elapsed().as_secs_f64()).max(0.0),
            )));
        }
        let cpu1 = crate::procstat::process_cpu_ticks().unwrap_or(0);
        let wall = t0.elapsed().as_secs_f64();
        let cpu = (cpu1.saturating_sub(cpu0)) as f64 / crate::procstat::USER_HZ;
        println!(
            "IDLE BENCH 3520x720: {wakes} wakes in {wall:.2} s = {:.3} wakes/s, {composites} composites, cpu {:.3} % of one core, wake body {body_us:.0} us, rss peak {} kB",
            wakes as f64 / wall,
            cpu / wall * 100.0,
            crate::procstat::rss_peak_kb().unwrap_or(0)
        );
    }
}

#[cfg(test)]
mod b50_tests {
    use super::*;

    /// B50: a finished world is archived (numbered, named), never deleted, and
    /// the archive loads back as the same finished world.
    #[test]
    fn a_finished_world_is_archived_not_deleted() {
        let dir = std::env::temp_dir().join(format!("citylive-b50-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let live = dir.join("city.sav");
        let mut s = sim::Sim::new(5, sim::Pace::Month, 41, 41);
        s.ending.world_days = 1.0;
        s.run_to(2.0 * 86400.0);
        s.stage_ending(sim::ending::Ending::Decline, sim::ending::Phase::Done, 0.5);
        let a = archive_world(&s, &live).unwrap();
        let b = archive_world(&s, &live).unwrap();
        assert!(a.exists() && b.exists() && a != b);
        assert!(a.file_name().unwrap().to_string_lossy().starts_with("001-"));
        assert!(b.file_name().unwrap().to_string_lossy().starts_with("002-"));
        let back = sim::Sim::load(&a).unwrap();
        assert_eq!(back.city_name, s.city_name);
        assert_eq!(back.ending_phase().0, sim::ending::Phase::Done);
        assert!(back.chronicle_final().is_some() || back.story_latest(1).iter().any(|l| l.text.contains("ended")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// B51: a finished world's folder holds the chronicle and, when the world
    /// recorded one, the timelapse as a GIF (the raw file is consumed).
    #[test]
    fn a_finished_world_gets_its_chronicle_and_timelapse() {
        let dir = std::env::temp_dir().join(format!("citylive-b51-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let live = dir.join("city.sav");
        let mut s = sim::Sim::new(5, sim::Pace::Month, 41, 41);
        s.ending.world_days = 1.0;
        s.run_to(2.0 * 86400.0);
        s.stage_ending(sim::ending::Ending::Decline, sim::ending::Phase::Done, 0.5);
        s.run_to(s.time + 3600.0);
        // Two days of timelapse, beside the live save.
        let tl = sim::timelapse::path_for(&world_save_path(&live, s.map.w, s.map.h));
        let mut rec = sim::timelapse::Recorder::open(&tl);
        let img = vec![0xFF40_8040u32; 128 * 64];
        rec.record(0, &img, 128, 64).unwrap();
        rec.record(1, &img, 128, 64).unwrap();
        let sav = archive_world(&s, &live).unwrap();
        let folder = archive_chronicle(&s, &sav, &live).unwrap();
        let md = std::fs::read_to_string(folder.join("chronicle.md")).unwrap();
        assert!(md.starts_with("# The Chronicle of "));
        assert!(md.contains("The End"));
        let gif = std::fs::read(folder.join("timelapse.gif")).unwrap();
        assert_eq!(&gif[..6], b"GIF89a");
        assert!(!tl.exists(), "the raw timelapse is consumed");
        assert_eq!(folder.file_name().unwrap().to_string_lossy(), sav.file_stem().unwrap().to_string_lossy());
        // B65: the folder carries the world's own save as well, and the B50
        // archive beside it is untouched.
        let inside = folder.join(sav.file_name().unwrap());
        assert!(inside.is_file(), "the folder holds a copy of the final save");
        assert!(sav.is_file(), "and worlds/NNN-name.sav is still there");
        assert_eq!(
            std::fs::read(&inside).unwrap(),
            std::fs::read(&sav).unwrap(),
            "the copy is the same save"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    /// A loaded city is told the frame before it catches up, so the numbers it is
    /// told must be the ones the scene derives for the same frame.
    #[test]
    fn declare_frame_matches_the_scene_camera_and_land_line() {
        for (w, h, frac) in [(960usize, 540usize, 0.30f32), (1920, 1080, 0.30), (2000, 700, 0.12), (800, 400, 0.0)] {
            let sky_h = render::scene::sky_pixels(h, frac);
            let vp = map::world_for_viewport(w as i32, h.saturating_sub(sky_h).max(64) as i32);
            let m = map::generate(7, vp.w, vp.h);
            let scene = Scene::from_map_deferred(
                m,
                SceneConfig {
                    width: w,
                    height: h,
                    seed: 7,
                    mode: SceneMode::Sim,
                    cars: 0,
                    peds: 0,
                    buildings: 0,
                    rain: false,
                },
                frac,
            );
            let mut s = sim::Sim::new(7, sim::Pace::Month, 41, 41);
            assert!(!s.frame_known());
            declare_frame(&mut s, w, h, frac);
            assert!(s.frame_known(), "{w}x{h}: the frame is declared");
            let belt = if scene.sky_height() > 0 { HORIZON_BELT_PX } else { 0 };
            assert_eq!(s.back_belt(), (scene.sky_height() as i32, belt), "{w}x{h}: the land line");
            for (x, y) in [(0, 0), (20, 3), (3, 20), (40, 40), (10, 25)] {
                let (sx, sy) = scene.cam.project(x as f32, y as f32, 0);
                let on = sx >= 0.0 && sx <= w as f32 && sy >= 0.0 && sy <= h as f32;
                assert_eq!(s.in_frame(x, y, 0, 0), on, "{w}x{h}: tile {x},{y} at {sx},{sy}");
            }
        }
    }
}
