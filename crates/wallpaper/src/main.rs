// The Windows build has no console: output goes to the log file
// (%LOCALAPPDATA%\CityLive\citylive.log), see `present::win::log`.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::env;
use std::path::PathBuf;

use present::{Layer, WallpaperConfig};
#[cfg(target_os = "linux")]
use present::{plasma, run};
#[cfg(windows)]
use present::win;

#[cfg(target_os = "linux")]
mod tray;
use render::SceneMode;

/// Exit after flushing the Windows log (a no-op elsewhere).
fn quit(code: i32) -> ! {
    #[cfg(windows)]
    win::log::finish();
    std::process::exit(code)
}

mod process {
    pub fn exit(code: i32) -> ! {
        super::quit(code)
    }
}

const USAGE: &str = "\
citylive-iso - CityLive Iso live wallpaper on KDE Plasma 6 Wayland

Usage: citylive-iso [OPTIONS]

Options:
  --scene <KIND>     sim   = live growing settlement/village (default)
                     empty = generated land only (D20)
                     test  = land + roads, buildings, cars and people
  --layer <LAYER>    bottom     = below normal windows, over desktop wallpaper (default)
                     background = on background layer behind desktop wallpaper
  --seed <N>         world and scene seed (default: a fresh random one per world)
  --ending <KIND>    (developer) stage a world ending: meteor | volcano | flood | kaiju | aliens | ai | redgiant | iceage | moon | decline | smog | sunset
  --ending-at <PHASE[:P]> where in it: warning | spectacle | aftermath | morph
  --endings on|off   (developer) off = this world never reaches its ending
                     (a bench fixture: a day-60 city on one land)
  --biome <NAME>     force the world's land (D66): temperate | desert | tundra |
                     tropical. Default: rolled from the world seed; a save keeps
                     the land it was saved with.
  --water-anim on|off  animate rivers, lakes and the sea (default on, 8 fps)
  --pace <PACE>      sim pace preset: day, week, month (default month)
  --speed <X>        multiplier for simulation clock rate (default 1.0)
  --sim-days <D>     fast-forward initial founding to D sim days (default 0.0)
  --save <PATH>      save file location (default: $XDG_DATA_HOME/citylive-iso/city.sav)
  --fps <N>          target frame rate cap (default 30, 0 = uncapped)
  --seconds <N>      exit cleanly after N seconds (default 0 = run until stopped)
  --rain             enable rain overlay
  --scale <N>        integer upscale override (default: max(1, round(tallest/1440)), D61)
  --include-output N force a virtual/headless output in (repeatable)
  --ignore-output N  drop an output (repeatable)
  -V, --version      print the version
  --lat <DEG>        latitude for the sky (default 51.48, D16 fallback)
  --lon <DEG>        longitude (default 0.0)
  --time HH:MM       fixed local time for the light (default: the real time)
  --date YYYY-MM-DD  fixed date (default: today)
  --sky <F>          sky band fraction at the top (0..0.4, default 0.30)
  --weather KIND     live | clear | partly | overcast | drizzle | rain | snow |
                     fog | storm | sandstorm (default: live, Open-Meteo every
                     15 minutes)
  --sim-hour H       start the saved city's clock at hour H (test knob)
  --traffic on|off   draw the traffic (default on)
  --stats            print frame times, CPU% and RSS every 10 s and on exit
  --export-frames D  write each output's frame as a PPM into D for the Plasma
                     wallpaper plugin to show (implies --layer background).
                     Defaults to $XDG_RUNTIME_DIR/citylive-iso/frames (tmpfs);
                     a directory on a real disk needs --force (see below)
  --export-shm [D]   B63: the fast transport — one mmapped, double-buffered
                     frame per output in D (default
                     $XDG_RUNTIME_DIR/citylive-iso/shm) for the native
                     org.citylive.frames QML item. Wins over --export-frames.
  --export-strips on|off
                     export only the 128-row strips a change touches on top of
                     a full base frame (default on; off = one full frame per
                     change, the B42d behaviour)
  --force            allow --export-frames/--export-shm onto a non-tmpfs directory
  --install-plasma   install the Plasma wallpaper plugin and select it for every
                     desktop containment (user level; the previous config is
                     backed up byte for byte; plasmashell is restarted)
  --uninstall-plasma restore that config exactly and remove the plugin
                     (plasmashell is restarted)
  --chronicle [SAVE] print the written history of a world and exit: the live
                     world by default, or any save (an archived world in
                     <save dir>/worlds/ too). B51
  -h, --help         print this help message

Windows only:
  --selftest [SECS]  render headless for SECS (default 15) and print timings,
                     CPU, memory and a PASS/FAIL verdict (see the log folder)
  --selftest-layout real|triple|single   monitors the self-test pretends to have
  --selftest-out DIR where the self-test picture and report go
  --diag             print the machine, monitors, DPI and the desktop's window
                     tree, then exit
  --show-pattern [SECS]  paint a test pattern on the desktop for SECS (default 10)
  --attach MODE      auto (default) | legacy | progman | toplevel: how to attach
                     to the desktop (for diagnosing)
  --autostart on|off|status   start with Windows (the per-user Run key)
  --no-pause         never pause for full-screen apps or covering windows
  --no-cover-pause   pause only for full-screen apps, not for maximised windows
";

/// The two Plasma-plugin commands, which change the config and exit.
pub enum PlasmaAction {
    None,
    Install,
    Uninstall,
    /// Windows: a diagnostic or settings command that exits without a wallpaper.
    #[cfg(windows)]
    Win(WinAction),
    /// B51: print a world's chronicle and exit (an optional save to read).
    Chronicle(Option<PathBuf>),
}

#[cfg(windows)]
pub enum WinAction {
    Selftest(win::selftest::Args),
    Diag(Option<String>),
    Pattern(f64, Option<String>),
    Autostart(String),
}

/// B37: the settings file is the base, and CLI flags override it (so a test or
/// the planner's scripts still work). A missing file is created with the
/// documented defaults.
fn apply_settings(cfg: &mut WallpaperConfig) {
    let path = present::config::config_path();
    let (c, warn) = present::config::Config::load_or_create(&path);
    if let Some(w) = warn {
        eprintln!("settings: {w}");
    }
    cfg.config_path = path;
    cfg.pace = c.pace();
    cfg.speed = c.speed as f64;
    cfg.scale_override = c.scale_override();
    cfg.sky_frac = c.sky.frac();
    if !c.include_outputs.is_empty() {
        cfg.include_outputs = c.include_outputs.clone();
    }
    if !c.ignore_outputs.is_empty() {
        cfg.ignore_outputs = c.ignore_outputs.clone();
    }
    // B96: the user's own place, or the last detection (auto); the
    // background thread refreshes it once the wallpaper runs.
    let place = locate::effective(
        c.location.auto,
        c.location.lat,
        c.location.lon,
        &c.location.name,
        &locate::cache_path(),
    );
    cfg.lat = place.lat;
    cfg.lon = place.lon;
    cfg.weather = c.weather_arg();
    cfg.fps = c.fps;
    cfg.layer = match c.layer {
        present::config::LayerMode::Bottom => Layer::Bottom,
        present::config::LayerMode::Background
        | present::config::LayerMode::PlasmaPlugin => Layer::Background,
    };
    if c.seed != 0 {
        cfg.seed = c.seed;
        cfg.seed_forced = true;
    }
    // Holidays and density are read again on every reload, so they live in the
    // config file rather than in WallpaperConfig.
    cfg.cars = (render::actors::DEFAULT_CARS as f32 * c.density.factor()) as usize;
    cfg.peds = (render::actors::DEFAULT_PEDS as f32 * c.density.factor()) as usize;
    cfg.holidays = c.holidays;
}

fn parse_args() -> Result<(WallpaperConfig, PlasmaAction), String> {
    let mut cfg = WallpaperConfig::default();
    apply_settings(&mut cfg);
    #[allow(unused_mut)]
    let mut install = false;
    #[allow(unused_mut)]
    let mut uninstall = false;
    #[allow(unused_mut, unused_variables)]
    let mut force_frames = false;
    let mut chronicle: Option<Option<PathBuf>> = None;
    #[cfg(windows)]
    let mut win_action: Option<WinAction> = None;
    #[cfg(windows)]
    let mut win_attach: Option<String> = None;
    #[cfg(windows)]
    let mut win_opts = win::Options::default();
    #[cfg(windows)]
    let mut selftest_layout: Option<String> = None;
    #[cfg(windows)]
    let mut selftest_out: Option<PathBuf> = None;
    let mut args = env::args().skip(1).peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                process::exit(0);
            }
            "-V" | "--version" => {
                println!("Citylive 2 {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "--scene" => {
                let val = args.next().ok_or("missing value for --scene")?;
                match val.to_lowercase().as_str() {
                    "sim" => cfg.mode = SceneMode::Sim,
                    "empty" => cfg.mode = SceneMode::Empty,
                    "test" => cfg.mode = SceneMode::Test,
                    other => {
                        return Err(format!("unknown scene '{other}'; use 'sim', 'empty', or 'test'"))
                    }
                }
            }
            "--layer" => {
                let val = args.next().ok_or("missing value for --layer")?;
                match val.to_lowercase().as_str() {
                    "bottom" => cfg.layer = Layer::Bottom,
                    "background" => cfg.layer = Layer::Background,
                    other => return Err(format!("unknown layer '{other}'; use 'bottom' or 'background'")),
                }
            }
            "--seed" => {
                let val = args.next().ok_or("missing value for --seed")?;
                cfg.seed = val.parse().map_err(|e| format!("invalid --seed '{val}': {e}"))?;
                cfg.seed_forced = true;
            }
            "--biome" => {
                let val = args.next().ok_or("missing value for --biome")?;
                cfg.biome = Some(
                    map::Biome::from_name(&val).ok_or_else(|| {
                        format!(
                            "invalid --biome '{val}' (temperate | desert | tundra | tropical)"
                        )
                    })?,
                );
            }
            "--ending" => {
                let val = args.next().ok_or("missing value for --ending")?;
                let kind = sim::ending::Ending::from_name(&val).ok_or_else(|| {
                    format!("invalid --ending '{val}' (meteor | volcano | flood | kaiju | aliens | ai | redgiant | iceage | moon | decline | smog | sunset)")
                })?;
                let at = cfg.ending_test.map(|e| (e.1, e.2)).unwrap_or((sim::ending::Phase::Spectacle, 0.5));
                cfg.ending_test = Some((kind, at.0, at.1));
            }
            "--ending-at" => {
                let val = args.next().ok_or("missing value for --ending-at")?;
                let (name, p) = match val.split_once(':') {
                    Some((n, p)) => (
                        n.to_string(),
                        p.parse::<f32>().map_err(|e| format!("invalid --ending-at progress '{p}': {e}"))?,
                    ),
                    None => (val.clone(), 0.5),
                };
                let ph = sim::ending::Phase::from_name(&name).ok_or_else(|| {
                    format!("invalid --ending-at '{name}' (warning | spectacle | aftermath | morph)")
                })?;
                let kind = cfg.ending_test.map(|e| e.0).unwrap_or(sim::ending::Ending::Meteor);
                cfg.ending_test = Some((kind, ph, p));
            }
            "--endings" => {
                let val = args.next().ok_or("missing value for --endings")?;
                cfg.endings_off = match val.to_lowercase().as_str() {
                    "on" | "1" | "true" => false,
                    "off" | "0" | "false" => true,
                    other => return Err(format!("--endings expects on|off, got '{other}'")),
                };
            }
            "--water-anim" => {
                let val = args.next().ok_or("missing value for --water-anim")?;
                cfg.water_anim = match val.to_lowercase().as_str() {
                    "on" | "1" | "true" => true,
                    "off" | "0" | "false" => false,
                    other => return Err(format!("--water-anim expects on|off, got '{other}'")),
                };
            }
            "--pace" => {
                let val = args.next().ok_or("missing value for --pace")?;
                cfg.pace = sim::Pace::from_name(&val)
                    .ok_or_else(|| format!("unknown pace '{val}'; use 'day', 'week', or 'month'"))?;
            }
            "--speed" => {
                let val = args.next().ok_or("missing value for --speed")?;
                cfg.speed = val.parse().map_err(|e| format!("invalid --speed '{val}': {e}"))?;
            }
            "--sim-days" => {
                let val = args.next().ok_or("missing value for --sim-days")?;
                cfg.sim_days = val.parse().map_err(|e| format!("invalid --sim-days '{val}': {e}"))?;
            }
            "--save" => {
                let val = args.next().ok_or("missing value for --save")?;
                cfg.save_path = PathBuf::from(val);
            }
            "--fps" => {
                let val = args.next().ok_or("missing value for --fps")?;
                cfg.fps = val.parse().map_err(|e| format!("invalid --fps '{val}': {e}"))?;
            }
            "--seconds" => {
                let val = args.next().ok_or("missing value for --seconds")?;
                cfg.seconds = val.parse().map_err(|e| format!("invalid --seconds '{val}': {e}"))?;
            }
            "--scale" => {
                let val = args.next().ok_or("missing value for --scale")?;
                cfg.scale_override = Some(
                    val.parse().map_err(|e| format!("invalid --scale '{val}': {e}"))?,
                );
            }
            "--include-output" => {
                let val = args.next().ok_or("missing value for --include-output")?;
                cfg.include_outputs.push(val);
            }
            "--ignore-output" => {
                let val = args.next().ok_or("missing value for --ignore-output")?;
                cfg.ignore_outputs.push(val);
            }
            "--lat" => {
                let val = args.next().ok_or("missing value for --lat")?;
                cfg.lat = val.parse().map_err(|e| format!("invalid --lat '{val}': {e}"))?;
                cfg.location_forced = true;
            }
            "--lon" => {
                let val = args.next().ok_or("missing value for --lon")?;
                cfg.lon = val.parse().map_err(|e| format!("invalid --lon '{val}': {e}"))?;
                cfg.location_forced = true;
            }
            "--time" => cfg.time_override = Some(args.next().ok_or("missing value for --time")?),
            "--date" => cfg.date_override = Some(args.next().ok_or("missing value for --date")?),
            "--sky" => {
                let val = args.next().ok_or("missing value for --sky")?;
                cfg.sky_frac = val.parse().map_err(|e| format!("invalid --sky '{val}': {e}"))?;
            }
            "--rain" => {
                cfg.rain = true;
            }
            "--stats" => {
                cfg.stats = true;
            }
            "--sim-hour" => {
                let v = args.next().ok_or("missing value for --sim-hour")?;
                cfg.sim_hour = Some(
                    v.parse::<f64>()
                        .map_err(|e| format!("invalid --sim-hour '{v}': {e}"))?,
                );
            }
            "--traffic" => {
                let v = args.next().ok_or("missing value for --traffic")?;
                cfg.traffic = match v.to_lowercase().as_str() {
                    "on" => true,
                    "off" => false,
                    other => return Err(format!("--traffic: expected on|off, got '{other}'")),
                };
            }
            "--weather" => {
                let v = args.next().ok_or("missing value for --weather")?;
                let v = v.to_lowercase();
                if v != "live" && weather_fx::look::preset(&v).is_none() {
                    return Err(format!(
                        "--weather: unknown kind '{v}' (live|clear|partly|overcast|drizzle|rain|snow|fog|storm|sandstorm)"
                    ));
                }
                cfg.weather = Some(v);
            }
            #[cfg(windows)]
            "--export-frames" | "--export-shm" | "--export-strips" | "--force" | "--install-plasma"
            | "--uninstall-plasma" => {
                return Err(format!("{arg} is a Linux (KDE Plasma) option and does nothing on Windows"));
            }
            #[cfg(target_os = "linux")]
            "--export-frames" => {
                // Optional value: bare `--export-frames` means "the default".
                let take = match args.peek() {
                    Some(v) if !v.starts_with('-') => Some(v.clone()),
                    _ => None,
                };
                let dir = match take {
                    Some(v) => {
                        args.next();
                        PathBuf::from(v)
                    }
                    None => plasma::frames_dir(),
                };
                cfg.export_frames = Some(dir);
            }
            #[cfg(target_os = "linux")]
            "--export-shm" => {
                // B63: optional value, like --export-frames.
                let take = match args.peek() {
                    Some(v) if !v.starts_with('-') => Some(v.clone()),
                    _ => None,
                };
                let dir = match take {
                    Some(v) => {
                        args.next();
                        PathBuf::from(v)
                    }
                    None => plasma::shm_dir(),
                };
                cfg.export_shm = Some(dir);
            }
            #[cfg(target_os = "linux")]
            "--export-strips" => {
                let v = args.next().ok_or("missing value for --export-strips")?;
                cfg.export_strips = match v.as_str() {
                    "on" => true,
                    "off" => false,
                    other => return Err(format!("--export-strips expects on|off, got '{other}'")),
                };
            }
            #[cfg(target_os = "linux")]
            "--force" => force_frames = true,
            "--chronicle" => {
                let take = match args.peek() {
                    Some(v) if !v.starts_with('-') => Some(PathBuf::from(v.clone())),
                    _ => None,
                };
                if take.is_some() {
                    args.next();
                }
                chronicle = Some(take);
            }
            #[cfg(target_os = "linux")]
            "--install-plasma" => install = true,
            #[cfg(target_os = "linux")]
            "--uninstall-plasma" => uninstall = true,
            "--cars" => {
                let val = args.next().ok_or("missing value for --cars")?;
                cfg.cars = val.parse().map_err(|e| format!("invalid --cars '{val}': {e}"))?;
            }
            "--peds" => {
                let val = args.next().ok_or("missing value for --peds")?;
                cfg.peds = val.parse().map_err(|e| format!("invalid --peds '{val}': {e}"))?;
            }
            "--buildings" => {
                let val = args.next().ok_or("missing value for --buildings")?;
                cfg.buildings = val
                    .parse()
                    .map_err(|e| format!("invalid --buildings '{val}': {e}"))?;
            }
            #[cfg(windows)]
            "--selftest" => {
                let mut a = win::selftest::Args::default();
                if let Some(v) = args.peek() {
                    if let Ok(secs) = v.parse::<f64>() {
                        a.secs = secs;
                        args.next();
                    }
                }
                a.attach = win_attach.clone();
                win_action = Some(WinAction::Selftest(a));
            }
            #[cfg(windows)]
            "--selftest-layout" => {
                selftest_layout = Some(args.next().ok_or("missing value for --selftest-layout")?);
            }
            #[cfg(windows)]
            "--selftest-out" => {
                selftest_out = Some(PathBuf::from(
                    args.next().ok_or("missing value for --selftest-out")?,
                ));
            }
            #[cfg(windows)]
            "--diag" => win_action = Some(WinAction::Diag(None)),
            #[cfg(windows)]
            "--show-pattern" => {
                let mut secs = 10.0;
                if let Some(v) = args.peek() {
                    if let Ok(x) = v.parse::<f64>() {
                        secs = x;
                        args.next();
                    }
                }
                win_action = Some(WinAction::Pattern(secs, None));
            }
            #[cfg(windows)]
            "--attach" => {
                win_attach = Some(args.next().ok_or("missing value for --attach")?);
            }
            #[cfg(windows)]
            "--autostart" => {
                let v = args.next().ok_or("missing value for --autostart (on|off|status)")?;
                win_action = Some(WinAction::Autostart(v));
            }
            #[cfg(windows)]
            "--no-pause" => win_opts.never_pause = true,
            #[cfg(windows)]
            "--no-cover-pause" => win_opts.pause_when_covered = false,
            other => return Err(format!("unknown argument '{other}'\n\n{USAGE}")),
        }
    }
    #[cfg(windows)]
    {
        win_opts.attach = win_attach.clone();
        win::set_options(win_opts);
        match win_action.take() {
            Some(WinAction::Selftest(mut a)) => {
                if let Some(l) = selftest_layout {
                    a.layout = l;
                }
                a.out = selftest_out;
                a.attach = win_attach;
                return Ok((cfg, PlasmaAction::Win(WinAction::Selftest(a))));
            }
            Some(WinAction::Diag(_)) => {
                return Ok((cfg, PlasmaAction::Win(WinAction::Diag(win_attach))));
            }
            Some(WinAction::Pattern(secs, _)) => {
                return Ok((cfg, PlasmaAction::Win(WinAction::Pattern(secs, win_attach))));
            }
            Some(a) => return Ok((cfg, PlasmaAction::Win(a))),
            None => {}
        }
    }

    #[cfg(target_os = "linux")]
    if let Some(dir) = &cfg.export_frames {
        if !plasma::is_tmpfs(dir) && !force_frames {
            return Err(format!(
                "{} is not on a tmpfs: the frames are rewritten several times a second (~13 MB/s on three screens) and would wear the disk.\n\
                 Use the default ($XDG_RUNTIME_DIR/citylive-iso/frames) or pass --force to accept the writes.",
                dir.display()
            ));
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(dir) = &cfg.export_shm {
        // B63: a shared-memory frame is mapped, so a file on a real disk is
        // written back by the kernel on its own schedule — the same wear.
        if !plasma::is_tmpfs(dir) && !force_frames {
            return Err(format!(
                "{} is not on a tmpfs: the shared-memory frames are rewritten at the frame rate and would wear the disk.\n\
                 Use the default ($XDG_RUNTIME_DIR/citylive-iso/shm) or pass --force to accept the writes.",
                dir.display()
            ));
        }
    }
    if let Some(p) = chronicle {
        return Ok((cfg, PlasmaAction::Chronicle(p)));
    }
    #[allow(unused_variables)]
    let action = match (install, uninstall) {
        (true, true) => return Err("--install-plasma and --uninstall-plasma are exclusive".into()),
        (true, false) => PlasmaAction::Install,
        (false, true) => PlasmaAction::Uninstall,
        (false, false) => PlasmaAction::None,
    };
    Ok((cfg, action))
}

/// B51: the save `--chronicle` reads when none is named: the live save, a
/// world-keyed live save beside it, or the newest archived world.
fn find_world_save(base: &std::path::Path) -> Option<PathBuf> {
    if base.exists() {
        return Some(base.to_path_buf());
    }
    let dir = base.parent()?;
    let name = base.file_name()?.to_string_lossy().to_string();
    let newest = |d: &std::path::Path, keep: &dyn Fn(&str) -> bool| -> Option<PathBuf> {
        std::fs::read_dir(d)
            .ok()?
            .flatten()
            .filter(|e| keep(&e.file_name().to_string_lossy()))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .max()
            .map(|(_, p)| p)
    };
    newest(dir, &|f: &str| {
        f.strip_prefix(&name)
            .and_then(|r| r.strip_prefix('.'))
            .is_some_and(|r| r.contains('x') && r.chars().all(|c| c.is_ascii_digit() || c == 'x'))
    })
    .or_else(|| newest(&dir.join("worlds"), &|f: &str| f.ends_with(".sav")))
}

fn main() {
    // B61f: keep the big buffers honest. A shadow bake's 9.7 MB plane is freed
    // when the bake is adopted; left to itself glibc raises its mmap threshold
    // after the first few 10 MB frees and that page range stays in the heap for
    // the life of the process, so RSS keeps every big buffer the wallpaper ever
    // held. Fixed at the default, a freed plane goes back to the kernel and the
    // reading is the footprint, not the high-water mark of the allocator.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 128 * 1024);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 128 * 1024);
    }
    // Windows: no console, so everything printed goes to the log file. A
    // command that is meant to be read (--selftest, --diag, --help...) also
    // attaches to the terminal it was started from.
    #[cfg(windows)]
    {
        let wants_console = env::args().skip(1).any(|a| {
            matches!(
                a.as_str(),
                "--selftest" | "--diag" | "--show-pattern" | "--help" | "-h" | "--autostart" | "--chronicle"
            )
        });
        win::log::init("citylive", wants_console);
        win::log::banner("CityLive");
    }
    let (mut cfg, action) = match parse_args() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Error: {err}");
            process::exit(1);
        }
    };

    match action {
        PlasmaAction::None => {}
        PlasmaAction::Chronicle(path) => {
            let path = path.or_else(|| find_world_save(&cfg.save_path));
            let Some(path) = path else {
                eprintln!("no world found beside {}", cfg.save_path.display());
                process::exit(1);
            };
            match sim::Sim::load(&path) {
                Ok(s) => print!("{}", s.chronicle_markdown()),
                Err(e) => {
                    eprintln!("cannot read {}: {e}", path.display());
                    process::exit(1);
                }
            }
            return;
        }
        #[cfg(windows)]
        PlasmaAction::Win(a) => {
            let code = match a {
                WinAction::Selftest(args) => win::selftest::run_selftest(cfg, args),
                WinAction::Diag(attach) => win::selftest::run_diag(attach.as_deref()),
                WinAction::Pattern(secs, attach) => win::selftest::run_pattern(secs, attach.as_deref()),
                WinAction::Autostart(v) => match v.as_str() {
                    "on" | "off" => match win::autostart::set(v == "on") {
                        Ok(()) => {
                            println!("start with Windows: {v}");
                            0
                        }
                        Err(e) => {
                            eprintln!("start with Windows: {e}");
                            1
                        }
                    },
                    _ => {
                        match win::autostart::registered() {
                            Some(c) => println!("start with Windows: on ({c})"),
                            None => println!("start with Windows: off"),
                        }
                        0
                    }
                },
            };
            quit(code);
        }
        #[cfg(not(target_os = "linux"))]
        PlasmaAction::Install | PlasmaAction::Uninstall => {
            eprintln!("the Plasma plugin is a Linux feature");
            quit(1);
        }
        #[cfg(target_os = "linux")]
        PlasmaAction::Install => {
            match plasma::install() {
                Ok(mut lines) => {
                    for l in &lines {
                        println!("{l}");
                    }
                    plasma::restart_plasmashell(&mut lines);
                    for l in lines.iter().skip(lines.len().saturating_sub(2)) {
                        println!("{l}");
                    }
                    println!(
                        "run the wallpaper with: citylive-iso --export-shm {}  (B63)\n\
                         or, without the native module: citylive-iso --export-frames {}\n\
                         uninstall with: citylive-iso --uninstall-plasma",
                        plasma::shm_dir().display(),
                        plasma::frames_dir().display()
                    );
                }
                Err(e) => {
                    eprintln!("Error installing the Plasma wallpaper plugin: {e}");
                    process::exit(1);
                }
            }
            return;
        }
        #[cfg(target_os = "linux")]
        PlasmaAction::Uninstall => {
            match plasma::uninstall() {
                Ok(mut lines) => {
                    plasma::restart_plasmashell(&mut lines);
                    for l in lines {
                        println!("{l}");
                    }
                    println!("the previous Plasma wallpaper is back");
                }
                Err(e) => {
                    eprintln!("Error uninstalling the Plasma wallpaper plugin: {e}");
                    process::exit(1);
                }
            }
            return;
        }
    }

    // B97: a downloaded update installs here, before anything starts. Only an
    // installed copy does this (never a build in a source tree), and only while
    // "check for updates automatically" is on.
    {
        let (c, _) = present::config::Config::load_or_create(&present::config::config_path());
        match update::apply::apply_at_start(c.auto_update) {
            update::apply::Applied::Nothing => {}
            update::apply::Applied::Exit => {
                println!("update: the installer has taken over; exiting");
                quit(0);
            }
            #[cfg(unix)]
            update::apply::Applied::Relaunch(exe) => {
                let e = update::apply::relaunch(&exe);
                eprintln!("update: could not start the new version ({e}); continuing with this one");
            }
            #[cfg(not(unix))]
            update::apply::Applied::Relaunch(_) => {}
        }
    }
    println!("Citylive 2 {}", env!("CARGO_PKG_VERSION"));

    if cfg.export_frames.is_some() || cfg.export_shm.is_some() {
        // The plugin draws the pixels inside the containment; a surface on the
        // Bottom layer would sit on top of the icons (D49).
        cfg.layer = Layer::Background;
    }
    if cfg.export_frames.is_some() && cfg.export_shm.is_some() {
        println!(
            "note: both --export-shm and --export-frames given; the shared-memory \
             frames win and no PPM is written"
        );
    }

    // Windows: the tray icon lives in the presenter (it owns the message loop);
    // the display is the same shared handle the render loop refreshes.
    #[cfg(windows)]
    {
        cfg.tray_display = Some(std::sync::Arc::new(std::sync::RwLock::new(
            sim::story::TrayStatus::default(),
        )));
    }

    // B37: the tray (StatusNotifierItem). No watcher on this desktop is not an
    // error — the wallpaper just runs without one.
    #[cfg(target_os = "linux")]
    {
        // B48a: the tray's picture of the city — the name, the headcount line
        // and the latest story lines — refreshed by the render loop through
        // this shared handle.
        let display: tray::Display =
            std::sync::Arc::new(std::sync::RwLock::new(sim::story::TrayStatus::default()));
        cfg.tray_display = Some(display);
        if let Some(rx) = tray::spawn("Growing a city".to_string(), cfg.tray_display.clone()) {
            cfg.tray_rx = Some(std::sync::Arc::new(std::sync::Mutex::new(rx)));
        }
    }

    #[cfg(windows)]
    let result = win::run(cfg);
    #[cfg(target_os = "linux")]
    let result = run(cfg);
    if let Err(err) = result {
        eprintln!("Error running citylive-iso wallpaper: {err}");
        process::exit(1);
    }
    #[cfg(windows)]
    {
        println!("CityLive stopped normally");
        win::log::finish();
    }
}
