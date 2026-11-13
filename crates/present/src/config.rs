//! The user's settings file (B37/M10): `$XDG_CONFIG_HOME/citylive-iso/config.toml`.
//!
//! Every option has a documented default; the file is written once with those
//! defaults and comments, and from then on it is the user's. The wallpaper
//! watches its modification time and applies changes live: the cheap ones
//! (fps, speed, traffic, holidays, weather, pause) take effect immediately, the
//! expensive ones (scale, sky amount, layer) rebuild the scene, and the pace is
//! set on the running sim. Nothing here ever drops the city: the world size is
//! what decides whether a save matches, and a scale change keeps the same tiles
//! when the union is the same (B34).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// How zoomed in the wallpaper is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Zoom {
    /// D61: `max(1, round(tallest / 1440))` — 1× on a 1440p monitor.
    #[default]
    Auto,
    #[serde(rename = "1")]
    One,
    #[serde(rename = "2")]
    Two,
    #[serde(rename = "3")]
    Three,
    #[serde(rename = "4")]
    Four,
}

impl Zoom {
    pub fn scale(self) -> Option<u32> {
        match self {
            Zoom::Auto => None,
            Zoom::One => Some(1),
            Zoom::Two => Some(2),
            Zoom::Three => Some(3),
            Zoom::Four => Some(4),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Zoom::Auto => "auto",
            Zoom::One => "1",
            Zoom::Two => "2",
            Zoom::Three => "3",
            Zoom::Four => "4",
        }
    }
}

/// How much of the frame is sky (D43/D53).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkyAmount {
    Small,
    #[default]
    Medium,
    Large,
}

impl SkyAmount {
    pub fn frac(self) -> f32 {
        match self {
            SkyAmount::Small => 0.20,
            SkyAmount::Medium => 0.30,
            SkyAmount::Large => 0.40,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SkyAmount::Small => "small",
            SkyAmount::Medium => "medium",
            SkyAmount::Large => "large",
        }
    }
}

/// Where the weather comes from (D21/D27).
///
/// B93: the file documents `weather = "fixed"` with the look in `weather_fixed`,
/// but a derived `Deserialize` only accepts the table form `{ fixed = "rain" }`
/// the settings app writes, so the documented spelling made the whole file fail
/// to parse and every setting fall back to its default. Both spellings are read;
/// a bare `"fixed"` is `Fixed("")`, and [`Config::weather_arg`] takes the look
/// from `weather_fixed` for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WeatherMode {
    /// Open-Meteo every 15 minutes.
    #[default]
    Live,
    /// No weather at all (clear, still air).
    Off,
    /// A fixed look, e.g. `fixed = "rain"`.
    Fixed(String),
}

impl<'de> Deserialize<'de> for WeatherMode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Word(String),
            Table { fixed: String },
        }
        match Repr::deserialize(d)? {
            Repr::Word(w) => match w.as_str() {
                "live" => Ok(WeatherMode::Live),
                "off" => Ok(WeatherMode::Off),
                "fixed" => Ok(WeatherMode::Fixed(String::new())),
                other => Err(serde::de::Error::custom(format!(
                    "unknown weather mode `{other}` (expected live, off or fixed)"
                ))),
            },
            Repr::Table { fixed } => Ok(WeatherMode::Fixed(fixed)),
        }
    }
}

/// How busy the streets and pavements are (D53: lively but calm).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Calm,
    #[default]
    Lively,
}

impl Density {
    /// Multiplier for the number of vehicles and villagers.
    pub fn factor(self) -> f32 {
        match self {
            Density::Calm => 0.55,
            Density::Lively => 1.0,
        }
    }
}

/// Which layer the wallpaper sits on (D49).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LayerMode {
    #[default]
    Bottom,
    Background,
    PlasmaPlugin,
}

/// Where the city is (D16, B96): detected (OS service, then IP lookup, see the
/// `locate` crate), or an explicit place the user set (`auto = false`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Location {
    /// `true` = detect the place; `lat`/`lon`/`name` are the fallback until a
    /// detection exists. `false` = exactly this place, nothing is detected.
    pub auto: bool,
    pub lat: f64,
    pub lon: f64,
    pub name: String,
}

impl Default for Location {
    fn default() -> Self {
        Self::default_place()
    }
}

impl Location {
    pub fn default_place() -> Self {
        Self {
            auto: true,
            lat: 51.48,
            lon: 0.0,
            name: "Greenwich, London".to_string(),
        }
    }
}

/// The whole settings file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Growth pace: `day`, `week` or `month` (D5/D9/D47).
    pub pace: String,
    /// Sim speed multiplier (1 = real time at the chosen pace).
    pub speed: f32,
    /// Zoom: `auto` or 1..4 (D61).
    pub zoom: Zoom,
    /// Sky amount: `small` (0.20), `medium` (0.30) or `large` (0.40) (D43).
    pub sky: SkyAmount,
    /// Outputs to force in (names, repeatable in the file as a list).
    pub include_outputs: Vec<String>,
    /// Outputs to leave out.
    pub ignore_outputs: Vec<String>,
    /// Where the city is (D16).
    pub location: Location,
    /// `live`, `off` or `fixed` (with `weather_fixed`).
    pub weather: WeatherMode,
    /// The fixed look when `weather = "fixed"`.
    pub weather_fixed: String,
    /// Public holidays on or off (D11/D60).
    pub holidays: bool,
    /// `calm` or `lively` (D53).
    pub density: Density,
    /// Frame cap for the wallpaper (D14: 30 is plenty).
    pub fps: u32,
    /// `bottom`, `background` or `plasma-plugin` (D49).
    pub layer: LayerMode,
    /// Seed for the next *new* world (0 = pick a fresh random one).
    pub seed: u64,
    /// D26: ambient sound. Reserved, always off for now.
    pub sound: bool,
    /// D68: the name shown in the tray and the chronicle. The founders' name
    /// is used when the field is empty (the name the city chose itself).
    pub city_name: String,
    /// D65: "just watch" — the story feed stays out of the tray; the city keeps
    /// its name.
    pub just_watch: bool,
    /// B53 / D68: the Buffalo Bills gameday (flags, tailgating, live score in
    /// the tray, win fireworks, feed lines). Needs the live feed.
    pub bills_gameday: bool,
    /// B53 item 3 (D68): while the session is idle (logind's idle hint) the
    /// city is quieter: a calmer pace, fewer people and cars. Never faster.
    /// Off by default; gentle (down to 0.6x).
    pub quiet_when_away: bool,
    /// B53 item 2: personal dates (birthday, anniversary, custom). Written as
    /// `[[dates]]` tables; nothing else in the app reads dates.
    pub dates: Vec<world::SpecialDate>,
    /// B47 (D67): the aurora when the real Kp index puts one over this
    /// latitude.
    pub aurora: bool,
    /// B47 (D67): the real ISS crossing the sky on a visible pass.
    pub iss: bool,
    /// B47 (D67): live aircraft from the ADS-B feed (a sparse schedule stands in
    /// when the feed has nothing).
    pub flights: bool,
    /// B97: look for a newer Citylive 2 release on GitHub (at start and every
    /// six hours), download it in the background and install it the next time
    /// the program starts.
    pub auto_update: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pace: "month".to_string(),
            speed: 1.0,
            zoom: Zoom::Auto,
            sky: SkyAmount::Medium,
            include_outputs: Vec::new(),
            ignore_outputs: Vec::new(),
            location: Location::default_place(),
            weather: WeatherMode::Live,
            weather_fixed: "clear".to_string(),
            holidays: true,
            density: Density::Lively,
            fps: 30,
            layer: LayerMode::Bottom,
            seed: 0,
            sound: false,
            city_name: String::new(),
            just_watch: false,
            bills_gameday: false,
            quiet_when_away: false,
            dates: Vec::new(),
            aurora: true,
            iss: true,
            flights: true,
            auto_update: true,
        }
    }
}

/// The documented default file, written on first run.
pub const DEFAULT_TOML: &str = r#"# CityLive Iso settings
# This file is watched: save it and the wallpaper applies the change live.
# Delete it to get these defaults back.

# Growth pace: "day", "week" or "month" (a month to a full city by default).
pace = "month"
# Sim speed multiplier (1.0 = real time at that pace).
speed = 1.0
# Zoom: "auto" (1x on a 1440p screen, D61) or 1, 2, 3, 4.
zoom = "auto"
# Sky amount: "small" (20 %), "medium" (30 %) or "large" (40 %).
sky = "medium"

# Outputs: names to force in or leave out, e.g. ["Virtual-Sunshine"].
include_outputs = []
ignore_outputs = []

# Weather: "live" (Open-Meteo), "off", or "fixed" with weather_fixed below.
weather = "live"
# Used when weather = "fixed": clear, partly, rain, snow, fog, storm, overcast.
weather_fixed = "clear"

# Public holidays (Christmas lights, fireworks on New Year's Eve and the 4th).
holidays = true

# How busy the streets are: "calm" or "lively".
density = "lively"

# Frame cap for the wallpaper.
fps = 30

# Layer: "bottom", "background" or "plasma-plugin" (D49).
layer = "bottom"

# Seed for the NEXT new world (0 = a fresh random one each time).
seed = 0

# Ambient sound (D26) is reserved: always off for now.
sound = false

# The city's name (D68): what the tray and the chronicle call it.
# Empty = the name the founders chose when the camp was struck.
city_name = ""
# "Just watch" (D65): keep the story feed out of the tray. The name stays.
just_watch = false

# Buffalo Bills gameday (B53): flags, tailgating, the live score in the tray and
# fireworks after a win. Uses the live sports feed.
bills_gameday = false

# Quieter while away (B53, D68): while the desktop is idle the city runs a
# calmer pace (down to 0.6x after 30 minutes away). Off by default.
quiet_when_away = false

# The living sky (B47, D67), each one switchable:
# the aurora when the real Kp index puts one over this latitude,
aurora = true
# the real ISS on a visible pass,
iss = true
# and live aircraft from the ADS-B feed (a sparse schedule stands in when the
# feed is empty or offline).
flights = true

# Updates (B97): look for a newer Citylive 2 release on GitHub at start and every
# six hours, download it in the background (checked against the release's
# SHA256SUMS) and install it the next time the program starts. Only the
# release list and the download are requested; nothing about you is sent.
auto_update = true

# Where the city is (D16). Used for the sky, weather, sun, moon and seasons.
[location]
# true: find the place by the operating system's location service, then a free
# IP lookup, and keep the last answer (lat/lon/name below are the fallback until
# the first one). false: use exactly the place below and never detect.
auto = true
lat = 51.48
lon = 0.0
name = "Greenwich, London"

# Personal dates (B53), added in the settings app or by hand. Each one:
#   [[dates]]
#   name = "Anniversary"
#   month = 6
#   day = 14
#   style = "balloons"     # "balloons", "banner", "fireworks" or "flowers"
#   year = 2027            # optional: leave out to repeat every year
"#;

/// The config path: `$XDG_CONFIG_HOME/citylive-iso/config.toml`.
#[cfg(windows)]
pub fn config_path() -> PathBuf {
    crate::plat::config_dir().join("config.toml")
}

#[cfg(not(windows))]
pub fn config_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir).join("citylive-iso").join("config.toml");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("citylive-iso")
            .join("config.toml");
    }
    PathBuf::from("config.toml")
}

impl Config {
    /// Load the file, writing the documented defaults if it does not exist.
    /// A broken file is reported and the defaults are used — the wallpaper must
    /// never refuse to start because of a typo.
    pub fn load_or_create(path: &Path) -> (Self, Option<String>) {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, DEFAULT_TOML);
            return (Config::default(), None);
        }
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(mut c) => {
                    // The documented `weather = "fixed"` names its look in
                    // `weather_fixed` (B93): keep the two in step.
                    if matches!(&c.weather, WeatherMode::Fixed(k) if k.is_empty()) {
                        c.weather = WeatherMode::Fixed(c.weather_fixed.clone());
                    }
                    (c, None)
                }
                Err(e) => (
                    Config::default(),
                    Some(format!("{}: {e} — using defaults", path.display())),
                ),
            },
            Err(e) => (
                Config::default(),
                Some(format!("{}: {e} — using defaults", path.display())),
            ),
        }
    }

    /// Write the config back (the settings app does this).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // B93: write beside the file and rename over it. The wallpaper polls this
        // file every two seconds; `fs::write` truncates first, and a read in that
        // window sees an empty file, which parses to *all defaults* (every key is
        // optional) and was applied (a zoom, sky or layout flip-flop rebuilt the
        // scene twice).
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config.toml".to_string());
        let tmp = path.with_file_name(format!(".{name}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }

    /// Modification time, for the live-reload watcher.
    pub fn mtime(path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
    }

    /// The pace as the sim understands it.
    pub fn pace(&self) -> sim::Pace {
        sim::Pace::from_name(&self.pace).unwrap_or(sim::Pace::Month)
    }

    /// The scale override, if the user pinned a zoom.
    pub fn scale_override(&self) -> Option<u32> {
        self.zoom.scale()
    }

    /// The weather the wallpaper should ask for (`None` = live).
    pub fn weather_arg(&self) -> Option<String> {
        match &self.weather {
            WeatherMode::Live => None,
            WeatherMode::Off => Some("clear".to_string()),
            WeatherMode::Fixed(k) if k.is_empty() => Some(self.weather_fixed.clone()),
            WeatherMode::Fixed(k) => Some(k.clone()),
        }
    }

    /// The layer mode as `present` wants it.
    pub fn layer(&self) -> present_layer::Layer {
        match self.layer {
            LayerMode::Bottom => present_layer::Layer::Bottom,
            LayerMode::Background => present_layer::Layer::Background,
            LayerMode::PlasmaPlugin => present_layer::Layer::Background,
        }
    }
}

/// A tiny shim so this module does not have to depend on the layer-shell type:
/// the caller maps these onto `present::Layer`.
pub mod present_layer {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Layer {
        Background,
        Bottom,
        Top,
        Overlay,
    }
}

/// What a settings change has to do (B38). Pure, so every key can be tested
/// without a compositor: the cheap knobs are copied into the running config, and
/// `structural` says whether the layout (and therefore the scene) must be
/// rebuilt. `pace` is applied to the sim, `seed` only matters for a new world.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsPlan {
    pub fps: u32,
    pub speed: f64,
    pub holidays: bool,
    pub weather: Option<String>,
    pub structural: bool,
    pub pace: sim::Pace,
    pub lat: f64,
    pub lon: f64,
    /// Keys that only take effect on the next new world.
    pub new_world_only: bool,
    /// B53: the personal dates or the gameday switch changed (the day's look
    /// is recomputed at once).
    pub calendar_changed: bool,
    /// B53 item 3: the idle pacing switch as the file now has it.
    pub quiet_when_away: bool,
}

impl Default for SettingsPlan {
    fn default() -> Self {
        Self {
            fps: 30,
            speed: 1.0,
            holidays: true,
            weather: None,
            structural: false,
            pace: sim::Pace::Month,
            lat: 51.48,
            lon: 0.0,
            new_world_only: false,
            calendar_changed: false,
            quiet_when_away: false,
        }
    }
}

/// Work out what changed between two settings files.
pub fn plan(old: &Config, new: &Config) -> SettingsPlan {
    SettingsPlan {
        fps: new.fps.max(1),
        speed: new.speed as f64,
        holidays: new.holidays,
        weather: new.weather_arg(),
        // Zoom, sky, layer, density and the output lists all change the scene's
        // size or contents.
        structural: new.zoom != old.zoom
            || new.sky != old.sky
            || new.layer != old.layer
            || new.density != old.density
            || new.include_outputs != old.include_outputs
            || new.ignore_outputs != old.ignore_outputs,
        pace: new.pace(),
        lat: new.location.lat,
        lon: new.location.lon,
        new_world_only: new.seed != old.seed && new.seed != 0,
        calendar_changed: new.dates != old.dates || new.bills_gameday != old.bills_gameday,
        quiet_when_away: new.quiet_when_away,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults round-trip through TOML unchanged (B37 item 5).
    #[test]
    fn defaults_round_trip() {
        let c = Config::default();
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(c, back);
    }

    /// B96: the location block round-trips, for a place set by hand in the
    /// southern hemisphere as for the detected default.
    #[test]
    fn location_round_trips() {
        let mut c = Config::default();
        c.location = Location { auto: false, lat: -33.8688, lon: 151.2093, name: "Sydney, Australia".into() };
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back.location, c.location);
        assert!(Config::default().location.auto, "detection is on by default");
        // A file written by hand with only the place keeps working.
        let old: Config = toml::from_str("[location]\nlat = 1.0\nlon = 2.0\nname = \"x\"\n").unwrap();
        assert_eq!((old.location.lat, old.location.lon), (1.0, 2.0));
    }

    /// The documented default file parses to exactly the defaults.
    #[test]
    fn documented_defaults_parse() {
        let c: Config = toml::from_str(DEFAULT_TOML).unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.pace(), sim::Pace::Month);
        assert_eq!(c.scale_override(), None);
        assert!((c.sky.frac() - 0.30).abs() < 1e-6);
    }

    /// A file with only a few keys keeps the defaults for the rest.
    #[test]
    fn partial_file_keeps_defaults() {
        let c: Config = toml::from_str("pace = \"week\"\nfps = 12\n").unwrap();
        assert_eq!(c.pace, "week");
        assert_eq!(c.fps, 12);
        assert_eq!(c.sky, SkyAmount::Medium);
        assert!(c.holidays);
    }

    /// A broken file falls back to the defaults instead of failing to start.
    #[test]
    fn broken_file_falls_back() {
        let dir = std::env::temp_dir().join(format!("citylive-b37-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, "pace = \n").unwrap();
        let (c, warn) = Config::load_or_create(&p);
        assert_eq!(c, Config::default());
        assert!(warn.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// B38 item 2: every key reaches the running config. One test per key.
    #[test]
    fn every_key_reaches_the_running_config() {
        let base = Config::default();

        let mut c = base.clone();
        c.fps = 12;
        let p = plan(&base, &c);
        assert_eq!(p.fps, 12, "fps");
        assert!(!p.structural);

        let mut c = base.clone();
        c.speed = 3.5;
        assert_eq!(plan(&base, &c).speed, 3.5, "speed");

        let mut c = base.clone();
        c.holidays = false;
        assert!(!plan(&base, &c).holidays, "holidays");

        let mut c = base.clone();
        c.pace = "week".into();
        assert_eq!(plan(&base, &c).pace, sim::Pace::Week, "pace");

        let mut c = base.clone();
        c.weather = WeatherMode::Fixed("snow".into());
        assert_eq!(plan(&base, &c).weather.as_deref(), Some("snow"), "weather");
        c.weather = WeatherMode::Off;
        assert_eq!(plan(&base, &c).weather.as_deref(), Some("clear"), "weather off");

        let mut c = base.clone();
        c.zoom = Zoom::Two;
        assert!(plan(&base, &c).structural, "zoom rebuilds");

        let mut c = base.clone();
        c.sky = SkyAmount::Large;
        assert!(plan(&base, &c).structural, "sky rebuilds");

        let mut c = base.clone();
        c.layer = LayerMode::Background;
        assert!(plan(&base, &c).structural, "layer rebuilds");

        let mut c = base.clone();
        c.density = Density::Calm;
        assert!(plan(&base, &c).structural, "density rebuilds");

        let mut c = base.clone();
        c.ignore_outputs = vec!["Virtual-1".into()];
        assert!(plan(&base, &c).structural, "outputs rebuild");

        let mut c = base.clone();
        c.location.lat = 10.0;
        c.location.lon = 20.0;
        let p = plan(&base, &c);
        assert_eq!((p.lat, p.lon), (10.0, 20.0), "location");

        let mut c = base.clone();
        c.seed = 4242;
        assert!(plan(&base, &c).new_world_only, "seed is for the next world");

        // The documented default file parses to the defaults, so a file written
        // by the settings app round-trips through the watcher unchanged.
        let written: Config = toml::from_str(DEFAULT_TOML).unwrap();
        assert!(!plan(&base, &written).structural);

        // B53: the gameday switch, idle pacing and personal dates.
        let mut c = base.clone();
        c.bills_gameday = !base.bills_gameday;
        assert!(plan(&base, &c).calendar_changed, "gameday switch");
        let mut c = base.clone();
        c.dates.push(world::SpecialDate {
            name: "Test day".into(),
            month: 5,
            day: 5,
            year: None,
            style: world::DateStyle::Banner,
        });
        assert!(plan(&base, &c).calendar_changed, "dates");
        assert!(!plan(&base, &base).calendar_changed);
        let mut c = base.clone();
        c.quiet_when_away = true;
        assert!(plan(&base, &c).quiet_when_away, "idle pacing");
    }

    /// Regression (B53): every top-level key in the documented file must be a
    /// top-level key. Before, the keys after `[location]` were read as fields of
    /// that table and silently ignored.
    #[test]
    fn keys_in_the_documented_file_are_not_swallowed_by_a_table() {
        let text = DEFAULT_TOML.replace("fps = 30", "fps = 12");
        let c: Config = toml::from_str(&text).unwrap();
        assert_eq!(c.fps, 12, "a fps edit in the file must reach the config");
        let text = DEFAULT_TOML.replace("bills_gameday = true", "bills_gameday = false");
        let c: Config = toml::from_str(&text).unwrap();
        assert!(!c.bills_gameday);
    }

    /// B53 item 2: dates come from `[[dates]]` tables, with the style and an
    /// optional year.
    #[test]
    fn dates_parse_from_the_file() {
        let text = format!(
            "{DEFAULT_TOML}\n[[dates]]\nname = \"Test day\"\nmonth = 5\nday = 5\nstyle = \"fireworks\"\n\n\
             [[dates]]\nname = \"One-off\"\nmonth = 9\nday = 9\nyear = 2027\n"
        );
        let c: Config = toml::from_str(&text).unwrap();
        assert_eq!(c.dates.len(), 2);
        assert_eq!(c.dates[0].style, world::DateStyle::Fireworks);
        assert_eq!(c.dates[1].year, Some(2027));
        assert_eq!(c.dates[0].style.name(), "fireworks");
        // A round trip through the settings app's save keeps them.
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back.dates, c.dates);
    }

    /// A missing file is created with the documented defaults.
    #[test]
    fn missing_file_is_created() {
        let dir = std::env::temp_dir().join(format!("citylive-b37c-{}", std::process::id()));
        let p = dir.join("citylive-iso").join("config.toml");
        let (c, warn) = Config::load_or_create(&p);
        assert_eq!(c, Config::default());
        assert!(warn.is_none());
        assert!(p.exists());
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("pace = \"month\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// B93: the documented `weather = "fixed"` spelling parses, and picks its
    /// look from `weather_fixed`; before, the whole file was rejected.
    #[test]
    fn documented_fixed_weather_parses() {
        let c: Config = toml::from_str("weather = \"fixed\"\nweather_fixed = \"rain\"\nfps = 12\n")
            .expect("the documented spelling must parse");
        assert_eq!(c.fps, 12);
        assert_eq!(c.weather_arg().as_deref(), Some("rain"));
        // The settings app's own form still round-trips.
        let mut d = Config::default();
        d.weather = WeatherMode::Fixed("snow".into());
        let text = toml::to_string_pretty(&d).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.weather, WeatherMode::Fixed("snow".into()));
        let live: Config = toml::from_str("weather = \"live\"").unwrap();
        assert_eq!(live.weather, WeatherMode::Live);
        assert!(toml::from_str::<Config>("weather = \"hail\"").is_err());
    }

    #[test]
    fn load_resolves_documented_fixed_weather() {
        let dir = std::env::temp_dir().join(format!("citylive-b93-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, "weather = \"fixed\"\nweather_fixed = \"fog\"\n").unwrap();
        let (c, warn) = Config::load_or_create(&p);
        assert!(warn.is_none());
        assert_eq!(c.weather, WeatherMode::Fixed("fog".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// B93: saving replaces the file in one rename, so the wallpaper's two-second
    /// poll can never read a half-written (or empty) settings file.
    #[test]
    fn save_replaces_the_file_atomically() {
        use std::os::unix::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!("citylive-b93-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        let mut c = Config::default();
        c.save(&p).unwrap();
        let before = std::fs::metadata(&p).unwrap().ino();
        c.fps = 12;
        c.save(&p).unwrap();
        let after = std::fs::metadata(&p).unwrap().ino();
        assert_ne!(before, after, "an in-place write is visible half-done");
        let (back, warn) = Config::load_or_create(&p);
        assert!(warn.is_none());
        assert_eq!(back.fps, 12);
        let strays: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(strays.len(), 1, "the temporary file must be gone");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
