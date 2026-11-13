//! `citylive-iso-settings` — the settings window (B37/M10).
//!
//! A small egui app over the same `config.toml` the wallpaper watches: every
//! option the file has, with a **live preview** — saving writes the file, and
//! the running wallpaper applies the change within two seconds (zoom/sky rebuild
//! the scene, pace/speed/fps/holidays/weather take effect immediately, and the
//! city is kept because saves are keyed by world size).
//!
//! About: version, the config and save paths, and a button that opens the save
//! folder. Ambient sound (D26) is a disabled placeholder.

// The Windows build has no console: output goes to
// %LOCALAPPDATA%\CityLive\citylive-settings.log.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;

use eframe::egui;
use present::config::{self, Config, Density, LayerMode, SkyAmount, WeatherMode, Zoom};
use world::{DateStyle, SpecialDate};

/// B38: if the first attempt cannot get a GL surface (a VM on llvmpipe, a
/// remote session), re-exec ourselves with Mesa's software rasteriser, so the
/// window opens anywhere.
fn relaunch_software() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    std::process::Command::new(exe)
        .env("CITYLIVE_SOFTWARE", "1")
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .env("GALLIUM_DRIVER", "llvmpipe")
        .status()
        .is_ok()
}

fn main() -> eframe::Result<()> {
    #[cfg(windows)]
    {
        present::win::log::init("citylive-settings", false);
        present::win::log::banner("CityLive settings");
    }
    let res = run();
    #[cfg(windows)]
    {
        if let Err(e) = &res {
            eprintln!("settings: the window failed: {e}");
        }
        present::win::log::finish();
    }
    res
}

fn run() -> eframe::Result<()> {
    let path = config::config_path();
    let (cfg, warn) = Config::load_or_create(&path);
    if let Some(w) = &warn {
        eprintln!("settings: {w}");
    }
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 640.0])
            .with_min_inner_size([460.0, 480.0])
            .with_title("CityLive Iso — Settings"),
        ..Default::default()
    };
    // Windows: Direct3D (wgpu, with the software WARP adapter as a last resort)
    // first, because a VM or an old machine often has no OpenGL 2+ driver and
    // the stock Windows OpenGL is 1.1. `CITYLIVE_RENDERER=glow` forces OpenGL.
    #[cfg(windows)]
    {
        let r = std::env::var("CITYLIVE_RENDERER").unwrap_or_default();
        options.renderer = if r == "glow" {
            eframe::Renderer::Glow
        } else {
            eframe::Renderer::Wgpu
        };
        eprintln!("settings: renderer {:?}", options.renderer);
    }
    let res = eframe::run_native(
        "CityLive Iso — Settings",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(SettingsApp {
                path: path.clone(),
                cfg,
                warn,
                dirty: false,
                saved_at: None,
                loc: LocationUi::default(),
                upd: UpdateUi::new(),
                #[cfg(windows)]
                others: Vec::new(),
                #[cfg(windows)]
                others_at: None,
                #[cfg(windows)]
                replace_note: None,
            }))
        }),
    );
    #[cfg(windows)]
    if res.is_err() && std::env::var("CITYLIVE_RENDERER").is_err() {
        eprintln!("settings: Direct3D renderer failed, retrying with OpenGL");
        if let Ok(exe) = std::env::current_exe() {
            if std::process::Command::new(exe)
                .env("CITYLIVE_RENDERER", "glow")
                .status()
                .is_ok()
            {
                return Ok(());
            }
        }
    }
    #[cfg(not(windows))]
    if res.is_err() && std::env::var("CITYLIVE_SOFTWARE").is_err() {
        eprintln!("settings: no GL surface, retrying with Mesa software rendering");
        if relaunch_software() {
            return Ok(());
        }
    }
    res
}

struct SettingsApp {
    path: PathBuf,
    cfg: Config,
    warn: Option<String>,
    dirty: bool,
    saved_at: Option<std::time::Instant>,
    loc: LocationUi,
    upd: UpdateUi,
    #[cfg(windows)]
    others: Vec<present::win::procs::Other>,
    #[cfg(windows)]
    others_at: Option<std::time::Instant>,
    #[cfg(windows)]
    replace_note: Option<String>,
}

/// B96: what the location section remembers between frames. Network work
/// (place search, "Detect now") runs on a thread and comes back on `rx`, so
/// the window never blocks.
#[derive(Default)]
struct LocationUi {
    query: String,
    results: Vec<locate::Place>,
    status: String,
    busy: bool,
    rx: Option<std::sync::mpsc::Receiver<LocMsg>>,
}

enum LocMsg {
    Found(Result<Vec<locate::Place>, String>),
    Detected(locate::Resolved),
}

impl LocationUi {
    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        match rx.try_recv() {
            Ok(LocMsg::Found(Ok(r))) => {
                self.status = if r.is_empty() { "No place with that name.".into() } else { String::new() };
                self.results = r;
                self.busy = false;
            }
            Ok(LocMsg::Found(Err(e))) => {
                self.status = format!("Search failed: {e}");
                self.busy = false;
            }
            Ok(LocMsg::Detected(r)) => {
                self.status = format!("{}: {}", r.describe(), r.name);
                self.busy = false;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
            Err(_) => self.busy = false,
        }
        if !self.busy {
            self.rx = None;
        }
    }

    fn search(&mut self) {
        let q = self.query.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.busy = true;
        self.status = "Searching...".into();
        std::thread::spawn(move || {
            let client = world::UreqClient::new();
            let _ = tx.send(LocMsg::Found(locate::search(&client, &q)));
        });
    }

    fn detect(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.busy = true;
        self.status = "Detecting...".into();
        std::thread::spawn(move || {
            let client = world::UreqClient::new();
            let os = locate::system_locator();
            let r = locate::resolve(
                os.as_deref().map(|o| o as &dyn locate::OsLocator),
                Some(&client),
                &locate::cache_path(),
                locate::now_secs(),
            );
            let _ = tx.send(LocMsg::Detected(r));
        });
    }
}

/// B97: the updates section. "Check now" runs on a thread; the answer comes
/// back on `rx`. The latest version and any downloaded update are read from the
/// updater's state file, which the wallpaper writes too.
struct UpdateUi {
    st: update::State,
    status: String,
    busy: bool,
    rx: Option<std::sync::mpsc::Receiver<update::Outcome>>,
}

impl UpdateUi {
    fn new() -> Self {
        Self {
            st: update::state::load(&update::state::default_dir()),
            status: String::new(),
            busy: false,
            rx: None,
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        match rx.try_recv() {
            Ok(out) => {
                self.status = out.describe();
                self.busy = false;
                self.rx = None;
                self.st = update::state::load(&update::state::default_dir());
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
            Err(_) => {
                self.busy = false;
                self.rx = None;
            }
        }
    }

    fn check_now(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.busy = true;
        self.status = "Checking...".into();
        std::thread::spawn(move || {
            let _ = tx.send(update::run_default(true));
        });
    }
}

impl SettingsApp {
    /// B97: automatic updates from GitHub Releases.
    fn updates_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.upd.poll(ctx);
        if ui
            .checkbox(&mut self.cfg.auto_update, "Check for updates automatically")
            .on_hover_text(
                "Looks at the project's GitHub releases at start and every six hours, downloads a \
                 newer version in the background and installs it the next time Citylive 2 starts.",
            )
            .changed()
        {
            self.dirty = true;
        }
        let latest = self.upd.st.latest.clone().unwrap_or_else(|| "not checked yet".into());
        ui.label(format!("Installed: {}    Latest release: {}", update::VERSION, latest));
        if let Some(s) = &self.upd.st.staged {
            ui.label(format!("Version {} is downloaded and installs the next time Citylive 2 starts.", s.version));
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.upd.busy, egui::Button::new("Check now")).clicked() {
                self.upd.check_now();
            }
            if !self.upd.status.is_empty() {
                ui.label(egui::RichText::new(self.upd.status.clone()).weak());
            } else if let Some(e) = &self.upd.st.last_error {
                ui.label(egui::RichText::new(format!("Last check failed: {e}")).weak());
            }
        });
    }

    fn save(&mut self) {
        match self.cfg.save(&self.path) {
            Ok(()) => {
                self.dirty = false;
                self.saved_at = Some(std::time::Instant::now());
                self.warn = None;
            }
            Err(e) => self.warn = Some(format!("cannot write {}: {e}", self.path.display())),
        }
    }
}

impl SettingsApp {
    /// B96: where the city is. Detected (OS location service, then an IP
    /// lookup), found by name, or typed in; the line at the top says which.
    fn location_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.loc.poll(ctx);
        let l = &self.cfg.location;
        let here = locate::effective(l.auto, l.lat, l.lon, &l.name, &locate::cache_path());
        ui.label(egui::RichText::new("Location").strong());
        ui.label(format!("{} ({:.3}, {:.3})", here.name, here.lat, here.lon));
        ui.label(egui::RichText::new(format!("Source: {}", here.describe())).weak());
        if ui
            .checkbox(&mut self.cfg.location.auto, "Detect my location automatically")
            .on_hover_text(
                "Uses the operating system's location service when it has one, then a free \
                 IP lookup (one request, nothing about you). Turn it off to never detect.",
            )
            .changed()
        {
            self.dirty = true;
        }
        if self.cfg.location.auto {
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.loc.busy, egui::Button::new("Detect now")).clicked() {
                    self.loc.detect();
                }
                ui.label(egui::RichText::new("Detection runs at start and once a day.").weak());
            });
        }
        ui.horizontal(|ui| {
            ui.label("Find a place");
            let r = ui.add(egui::TextEdit::singleline(&mut self.loc.query).hint_text("a city, e.g. Sydney").desired_width(180.0));
            let go = ui.add_enabled(!self.loc.busy, egui::Button::new("Search")).clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go && !self.loc.busy {
                self.loc.search();
            }
        });
        let mut picked = None;
        for p in &self.loc.results {
            if ui.button(format!("{}  ({:.2}, {:.2})", p.name, p.lat, p.lon)).clicked() {
                picked = Some(p.clone());
            }
        }
        if let Some(p) = picked {
            // Choosing a place sets it by hand: detection is off from here on.
            self.cfg.location.auto = false;
            self.cfg.location.lat = p.lat;
            self.cfg.location.lon = p.lon;
            self.cfg.location.name = p.name;
            self.loc.results.clear();
            self.loc.status = String::new();
            self.dirty = true;
        }
        if !self.loc.status.is_empty() {
            ui.label(egui::RichText::new(self.loc.status.clone()).weak());
        }
        ui.horizontal(|ui| {
            let mut changed = false;
            ui.label("Lat");
            changed |= ui
                .add(egui::DragValue::new(&mut self.cfg.location.lat).speed(0.01).range(-90.0..=90.0))
                .changed();
            ui.label("Lon");
            changed |= ui
                .add(egui::DragValue::new(&mut self.cfg.location.lon).speed(0.01).range(-180.0..=180.0))
                .changed();
            if changed {
                // Typing a coordinate is setting the place yourself.
                self.cfg.location.auto = false;
                self.dirty = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Name");
            if ui.text_edit_singleline(&mut self.cfg.location.name).changed() {
                self.dirty = true;
            }
        });
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Live preview: save shortly after a change, so the wallpaper follows.
        if self.dirty && self.saved_at.map(|t| t.elapsed().as_millis() > 350).unwrap_or(true) {
            self.save();
        }
        egui::TopBottomPanel::top("title").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("CityLive Iso");
                ui.label(egui::RichText::new("settings").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.dirty {
                        ui.label(egui::RichText::new("saving…").weak());
                    } else if self.saved_at.is_some() {
                        ui.label(egui::RichText::new("applied").weak().color(egui::Color32::from_rgb(120, 200, 140)));
                    }
                });
            });
            if let Some(w) = &self.warn {
                ui.colored_label(egui::Color32::from_rgb(230, 120, 100), w);
            }
            ui.separator();
        });

        egui::CentralPanel::default().show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);

            ui.group(|ui| {
                ui.label(egui::RichText::new("Growth").strong());
                ui.horizontal(|ui| {
                    ui.label("Pace");
                    for (name, label) in [("day", "A day"), ("week", "A week"), ("month", "A month")] {
                        if ui
                            .selectable_label(self.cfg.pace == name, label)
                            .clicked()
                        {
                            self.cfg.pace = name.to_string();
                            self.dirty = true;
                        }
                    }
                });
                ui.add(
                    egui::Slider::new(&mut self.cfg.speed, 0.25..=8.0)
                        .text("speed")
                        .logarithmic(true),
                );
                if ui.button("Reset to defaults").clicked() {
                    self.cfg = Config::default();
                    self.dirty = true;
                }
            });

            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Picture").strong());
                ui.horizontal(|ui| {
                    ui.label("Zoom");
                    for (z, label) in [
                        (Zoom::Auto, "Auto"),
                        (Zoom::One, "1×"),
                        (Zoom::Two, "2×"),
                        (Zoom::Three, "3×"),
                        (Zoom::Four, "4×"),
                    ] {
                        if ui.selectable_label(self.cfg.zoom == z, label).clicked() {
                            self.cfg.zoom = z;
                            self.dirty = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Sky");
                    for (s, label) in [
                        (SkyAmount::Small, "Small"),
                        (SkyAmount::Medium, "Medium"),
                        (SkyAmount::Large, "Large"),
                    ] {
                        if ui.selectable_label(self.cfg.sky == s, label).clicked() {
                            self.cfg.sky = s;
                            self.dirty = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Layer");
                    for (l, label) in [
                        (LayerMode::Bottom, "Behind windows"),
                        (LayerMode::Background, "Behind icons"),
                        (LayerMode::PlasmaPlugin, "Plasma plugin"),
                    ] {
                        if ui.selectable_label(self.cfg.layer == l, label).clicked() {
                            self.cfg.layer = l;
                            self.dirty = true;
                        }
                    }
                });
            });

            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Life").strong());
                ui.horizontal(|ui| {
                    ui.label("Density");
                    for (d, label) in [(Density::Calm, "Calm"), (Density::Lively, "Lively")] {
                        if ui.selectable_label(self.cfg.density == d, label).clicked() {
                            self.cfg.density = d;
                            self.dirty = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.cfg.holidays, "Public holidays")
                        .on_hover_text("Christmas lights, New Year and 4th of July fireworks")
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.cfg.bills_gameday, "Bills gameday")
                        .on_hover_text(
                            "Flags and banners on gameday, tailgaters at the stadium, the live score \
                             in the tray, fireworks after a win. Uses the live sports feed.",
                        )
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.cfg.quiet_when_away, "Quieter while I am away")
                        .on_hover_text(
                            "While I am away the city grows at a calmer pace (down to 0.6x after 30 \
                             minutes). Uses the desktop's idle hint only.",
                        )
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    let mut sound = self.cfg.sound;
                    ui.add_enabled(false, egui::Checkbox::new(&mut sound, "Ambient sound"));
                    ui.label(egui::RichText::new("(later)").weak());
                });
            });

            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Sky life").strong());
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.cfg.aurora, "Aurora")
                        .on_hover_text(
                            "The northern lights when the real Kp index puts them over this \
                             latitude: a green curtain, red tops in a big storm, never by day \
                             and dimmed by cloud and moonlight.",
                        )
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.cfg.iss, "ISS passes")
                        .on_hover_text(
                            "The real space station, a bright steady point, on a pass where the \
                             sun is down here and still lighting it up there. Uses the live ISS \
                             feed.",
                        )
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.cfg.flights, "Live flights")
                        .on_hover_text(
                            "Aircraft near the city from the ADS-B feed: pixel planes at their \
                             real bearing and height, nav lights at night, contrails when it is \
                             cold up there. A sparse schedule stands in when the feed is empty.",
                        )
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
            });

            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("City").strong());
                ui.horizontal(|ui| {
                    ui.label("Name");
                    if ui.text_edit_singleline(&mut self.cfg.city_name).changed() {
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.cfg.just_watch, "Just watch")
                        .on_hover_text("Keep the story feed out of the tray (D65). The name stays.")
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
                ui.label(
                    egui::RichText::new("An empty name is the one the founders chose (D68).")
                        .weak(),
                );
            });

            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Weather").strong());
                ui.horizontal(|ui| {
                    for (w, label) in [
                        (WeatherMode::Live, "Live"),
                        (WeatherMode::Off, "Off"),
                        (WeatherMode::Fixed(self.cfg.weather_fixed.clone()), "Fixed"),
                    ] {
                        let selected = std::mem::discriminant(&self.cfg.weather)
                            == std::mem::discriminant(&w);
                        if ui.selectable_label(selected, label).clicked() {
                            self.cfg.weather = w;
                            self.dirty = true;
                        }
                    }
                });
                if matches!(self.cfg.weather, WeatherMode::Fixed(_)) {
                    ui.horizontal(|ui| {
                        ui.label("Look");
                        for k in ["clear", "partly", "rain", "snow", "fog", "storm", "overcast"] {
                            if ui
                                .selectable_label(self.cfg.weather_fixed == k, k)
                                .clicked()
                            {
                                self.cfg.weather_fixed = k.to_string();
                                self.cfg.weather = WeatherMode::Fixed(k.to_string());
                                self.dirty = true;
                            }
                        }
                    });
                }
            });

            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Place and screens").strong());
                self.location_ui(ui, ctx);
                ui.collapsing("Outputs to include / ignore", |ui| {
                    ui.label("One name per line (e.g. Virtual-Sunshine).");
                    let mut inc = self.cfg.include_outputs.join("\n");
                    if ui
                        .add(egui::TextEdit::multiline(&mut inc).desired_rows(2))
                        .changed()
                    {
                        self.cfg.include_outputs = inc
                            .lines()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        self.dirty = true;
                    }
                    let mut ign = self.cfg.ignore_outputs.join("\n");
                    if ui
                        .add(egui::TextEdit::multiline(&mut ign).desired_rows(2))
                        .changed()
                    {
                        self.cfg.ignore_outputs = ign
                            .lines()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("FPS cap");
                    if ui
                        .add(egui::DragValue::new(&mut self.cfg.fps).range(1..=60))
                        .changed()
                    {
                        self.dirty = true;
                    }
                    ui.label("Seed for the next new city");
                    if ui
                        .add(egui::DragValue::new(&mut self.cfg.seed).speed(1.0))
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
            });

            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Dates").strong());
                ui.label(
                    egui::RichText::new(
                        "Days the city dresses up for: a birthday, an anniversary, anything. \
                         Only the dates you enter here are used.",
                    )
                    .weak(),
                );
                let mut touched = false;
                let mut remove: Option<usize> = None;
                for (i, sd) in self.cfg.dates.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.text_edit_singleline(&mut sd.name).changed() {
                            touched = true;
                        }
                        if ui
                            .add(egui::DragValue::new(&mut sd.month).range(1..=12).prefix("month "))
                            .changed()
                        {
                            touched = true;
                        }
                        if ui
                            .add(egui::DragValue::new(&mut sd.day).range(1..=31).prefix("day "))
                            .changed()
                        {
                            touched = true;
                        }
                        let mut every = sd.year.is_none();
                        if ui.checkbox(&mut every, "every year").changed() {
                            sd.year = if every { None } else { Some(2026) };
                            touched = true;
                        }
                        if let Some(y) = sd.year.as_mut() {
                            if ui
                                .add(egui::DragValue::new(y).range(1900..=2200))
                                .changed()
                            {
                                touched = true;
                            }
                        }
                        egui::ComboBox::from_id_salt(("date-style", i))
                            .selected_text(sd.style.name())
                            .show_ui(ui, |ui| {
                                for st in DateStyle::ALL {
                                    if ui.selectable_label(sd.style == st, st.name()).clicked() {
                                        sd.style = st;
                                        touched = true;
                                    }
                                }
                            });
                        if ui.button("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                    if !sd.is_valid() {
                        ui.colored_label(
                            egui::Color32::from_rgb(230, 120, 100),
                            "This date does not exist, so it is skipped.",
                        );
                    }
                }
                if let Some(i) = remove {
                    self.cfg.dates.remove(i);
                    touched = true;
                }
                if touched {
                    self.dirty = true;
                }
                if ui.button("Add a date").clicked() {
                    self.cfg.dates.push(SpecialDate {
                        name: "New date".to_string(),
                        month: 1,
                        day: 1,
                        year: None,
                        style: DateStyle::Balloons,
                    });
                    self.dirty = true;
                }
            });

            ui.add_space(10.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("About").strong());
                ui.label(format!(
                    "Citylive 2 {} — a living pixel city for your desktop",
                    env!("CARGO_PKG_VERSION")
                ));
                self.updates_ui(ui, ctx);
                ui.label(egui::RichText::new(format!("Settings: {}", self.path.display())).weak());
                ui.label(
                    egui::RichText::new(format!(
                        "City saves: {}",
                        present::default_save_path().display()
                    ))
                    .weak(),
                );
                #[cfg(windows)]
                {
                    // Other wallpaper programs holding the desktop: told, and the
                    // old CityLive can be replaced, only on this click.
                    if self.others_at.map(|t| t.elapsed().as_secs() > 5).unwrap_or(true) {
                        self.others = present::win::procs::scan_other_wallpapers();
                        self.others_at = Some(std::time::Instant::now());
                    }
                    let has_old = self
                        .others
                        .iter()
                        .any(|o| o.kind == present::wincore::OtherApp::OldCityLive);
                    for o in &self.others {
                        ui.label(
                            egui::RichText::new(format!("{} is running and may cover or hide this wallpaper.", o.label))
                                .color(egui::Color32::from_rgb(230, 180, 90)),
                        );
                    }
                    if has_old {
                        if ui.button("Replace the old CityLive (stop it and turn off its start-up)").clicked() {
                            let lines = present::win::procs::replace_old_citylive();
                            for l in &lines {
                                eprintln!("settings: replace old CityLive: {l}");
                            }
                            self.replace_note = Some(lines.join("; "));
                            self.others_at = None;
                        }
                    }
                    if let Some(n) = &self.replace_note {
                        ui.label(egui::RichText::new(n).weak());
                    }
                    let mut on = present::win::autostart::is_enabled();
                    if ui.checkbox(&mut on, "Start CityLive when I sign in to Windows").changed() {
                        if let Err(e) = present::win::autostart::set(on) {
                            self.warn = Some(format!("start with Windows: {e}"));
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Open save folder").clicked() {
                        let dir = present::default_save_path()
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| PathBuf::from("."));
                        present::plat::open_path(&dir);
                    }
                    if ui.button("Save now").clicked() {
                        self.save();
                    }
                });
            });
            ui.add_space(8.0);
        });
        });
    }
}
