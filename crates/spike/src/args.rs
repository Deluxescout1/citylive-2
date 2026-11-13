//! Command line parsing. No external crate: the spike's flags are a fixed set.

use std::fmt;

use render::SceneMode;
use render::actors::DepthMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Png,
    Window,
    Bench,
}

pub struct Args {
    pub mode: Mode,
    pub scene: SceneMode,
    pub depth: DepthMode,
    /// Drive the growth sim instead of a static scene.
    pub sim: bool,
    /// Render at this sim time (days).
    pub sim_days: f64,
    pub pace: String,
    /// Timelapse: write a PNG every `every_min` sim-minutes to this directory.
    pub timelapse: Option<String>,
    /// Day loop (B20): write a PNG every `every_min` *clock* minutes from
    /// `start_hour` through 24 h, with the sky at that time of day.
    pub dayloop: Option<String>,
    pub start_hour: f64,
    pub every_min: f64,
    /// Render seven lots, one per construction stage, into this PNG.
    pub stage_strip: Option<String>,
    /// Plan view: districts, the street grid and the built cells (B26b item 6).
    pub districts: Option<String>,
    /// B41: tint every lane by its road class on the perspective frame.
    pub road_class: bool,
    /// B41b: draw the wildlife layer over the frame.
    pub wildlife: bool,
    /// Extra multiplier on the pace for the live window.
    pub speed: f64,
    /// Fixed clock for the renderer: "HH:MM" and "YYYY-MM-DD" (D21 override).
    pub time: Option<String>,
    /// B41: the light, without moving the sim clock (so a frame can be taken at
    /// an exact sim moment and still be daylight).
    pub light: Option<String>,
    pub date: Option<String>,
    /// Founding point override, in framebuffer pixels (B17: largest monitor).
    pub focus: Option<(f32, f32)>,
    /// Sky band as a fraction of the frame height (D43).
    pub sky: f32,
    /// Animate the water surface (B21); `--water-anim off` freezes it.
    pub water_anim: bool,
    /// `--weather clear|rain|snow|fog|storm|overcast` (B27): a fixed sky, for
    /// stills and for the tests.
    pub weather: Option<String>,
    /// B29: force an emergency call-out (for the still) and/or write a traffic
    /// animation loop into this directory.
    pub force_emergency: bool,
    /// B31: run the timetable until a plane is crossing the sky.
    pub plane: bool,
    /// B31b: run the timetable until this mover is on the map
    /// (`train|boat|ship|plane`).
    pub want: Option<String>,
    pub traffic_loop: Option<String>,
    /// Draw the villagers (D45) — on by default.
    pub walkers: bool,
    /// `--water-loop DIR`: N frames of the water animation (B21's GIF).
    pub water_loop: Option<String>,
    /// `--walker-loop DIR`: N frames of villagers walking (D45).
    pub walker_loop: Option<String>,
    /// Time one water animation tick and print the cost per second at 8 fps.
    pub water_bench: bool,
    /// B61c: time the cloud drift's swept-strip damage and composite at a few
    /// covers (`--clouds-bench`, covers 30 % and 80 %).
    pub clouds_bench: bool,
    /// B46d: `--landform-sheet DIR` writes the landform sheet (1x and 4x strips).
    pub landform_sheet: Option<String>,
    /// B61b: render one weather kind twice — the old whole-frame overlay draw and
    /// the scene's ambient + particles — into `<DIR>/B61b-<kind>-compare.png`
    /// (a 4x crop pair), and print whether the two frames are identical.
    pub wx_compare: Option<String>,
    pub path: String,
    pub w: usize,
    pub h: usize,
    /// Window upscale / bench upscale factor.
    pub scale: usize,
    pub fps: u32,
    pub rain: bool,
    pub seconds: f64,
    pub frames: usize,
    /// Sim steps run before the second PNG.
    pub steps: usize,
    pub seed: u64,
    /// D66: force the world's land (`--biome desert`); `None` rolls it from the
    /// seed. The spike's way to render a specific land for a still.
    pub biome: Option<map::Biome>,
    /// B49: print this world's technology path (and what its crews have
    /// installed by `--sim-days`), then carry on with the chosen mode.
    pub tech_print: bool,
    /// B74: print the overlap audit (tiles and pixels) for the city at
    /// `--sim-days`, then exit without writing a frame.
    pub overlap_report: bool,
    /// B78: print what the city wears (grime, art, boards, shutters, scaffold,
    /// repaints) and where the interesting lots are on screen, then exit.
    pub wear_report: bool,
    /// B50: put the world at a point of its ending (`--ending meteor
    /// --ending-at spectacle:0.5`). Without `--ending` the world never ends in
    /// a spike run, whatever `--sim-days` says.
    pub ending: Option<sim::ending::Ending>,
    pub ending_at: (sim::ending::Phase, f32),
    /// B50: write this many frames of the ending, `--ending-step` sim seconds
    /// apart, to `--ending-loop DIR` (and time them).
    pub ending_loop: Option<String>,
    pub ending_step: f64,
    /// B50: also write the morph's next-world check (found the next land).
    pub ending_world: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            mode: Mode::Png,
            scene: SceneMode::Test,
            depth: DepthMode::PerPixel,
            sim: false,
            sim_days: 0.0,
            pace: "month".to_string(),
            timelapse: None,
            dayloop: None,
            start_hour: 5.0,
            every_min: 30.0,
            stage_strip: None,
            districts: None,
            road_class: false,
            wildlife: false,
            speed: 1.0,
            time: None,
            light: None,
            date: None,
            focus: None,
            sky: 0.30,
            water_anim: true,
            weather: None,
            force_emergency: false,
            plane: false,
            want: None,
            traffic_loop: None,
            walkers: true,
            water_loop: None,
            walker_loop: None,
            water_bench: false,
            clouds_bench: false,
            landform_sheet: None,
            wx_compare: None,
            path: "frame.png".to_string(),
            w: 960,
            h: 540,
            scale: 2,
            fps: 30,
            rain: false,
            seconds: 0.0,
            frames: 600,
            steps: 100,
            seed: 7,
            biome: None,
            tech_print: false,
            overlap_report: false,
            wear_report: false,
            ending: None,
            ending_at: (sim::ending::Phase::Spectacle, 0.5),
            ending_loop: None,
            ending_step: 1.0 / 30.0,
            ending_world: false,
        }
    }
}

pub const USAGE: &str = "\
citylive-iso-spike - CityLive Iso renderer spike (M2)

  --png PATH           render one frame (and one after --steps sim steps) to PATH
  --scene KIND         empty = generated land only (D20); test = land + roads,
                       buildings and actors (default test)
  --window             open a window and present the framebuffer (nearest upscale)
  --bench              render as fast as possible, no window, print timings

  --w N                internal framebuffer width  (default 960)
  --h N                internal framebuffer height (default 540)
  --scale N            integer upscale for window/bench (default 2)
  --fps N              frame cap for --window (default 30, 0 = uncapped)
  --seconds S          --window: exit after S seconds (default 0 = run until closed)
  --frames N           --bench: frames to time (default 600)
  --steps N            --png: sim steps before the second image (default 100)
  --depth MODE         actor occlusion: perpixel (default), anchor or off
  --time HH:MM         fixed time of day (sky + light), default: now
  --date YYYY-MM-DD    fixed date, default: today
  --sim                drive the growth simulation (D20/D33)
  --sim-days D         render the city at D sim days (default 0)
  --pace P             growth pace: day | week | month (default month)
  --districts PATH     plan view: district map, street grid and built cells
  --timelapse DIR      write a PNG every --every S sim-minutes into DIR
  --every M            timelapse interval in sim minutes (default 30)
  --dayloop DIR        write a PNG every --every clock minutes for 24 h, the
                       sky at that time of day (start hour via --start)
  --start H            dayloop start hour, 0..24 (default 5)
  --speed X            live window: multiply the pace by X (default 1)
  --stage-strip PATH   render one lot at each of the 7 stages side by side
  --road-class         tint the streets by class (B41 overlay)
  --wildlife           draw the wildlife layer (B41b)
  --light HH:MM        light only: set the sun without moving the sim clock
  --rain               enable the rain overlay
  --water-anim on|off  animate rivers, lakes and the sea (default on)
  --force-emergency    put an emergency vehicle on the road for the still
  --plane              run the timetable until a plane crosses the sky (B31)
  --want KIND          run the timetable until this mover exists
                       (train | boat | ship | plane) and print where it is
  --traffic-loop DIR   write --frames PNGs of the traffic moving (B29)
  --weather KIND       clear | rain | snow | fog | storm | overcast: a fixed
                       sky for stills (B27)
  --walkers on|off     draw the villagers on their paths (default on)
  --walker-loop DIR    write --frames PNGs of the villagers walking
  --water-loop DIR     write --frames PNGs of the water animation into DIR
  --water-bench        time one water-animation tick (compose + upscale + copy)
  --seed N             scene seed (default 7)
  --ending KIND        B50: stage the world's ending: meteor | volcano | flood |
                       kaiju | aliens | ai | redgiant | iceage | moon | decline |
                       smog | sunset (needs --sim)
  --ending-at PHASE[:P] where in it: warning | spectacle | aftermath | morph
                       (P = 0..1 through the phase, default 0.5)
  --ending-loop DIR    write --frames frames of the ending, --ending-step sim
                       seconds apart, to DIR, and time them
  --biome NAME         force the world's land: temperate | desert | tundra |
                       tropical (D66; default: rolled from the seed)
  --tech-print         print the world's technology path (B49/D72) and what the
                       crews installed by --sim-days, then continue
  --overlap-report     B74: audit the city at --sim-days for overlaps (tiles and
                       pixels) and print the counts, then exit
  --wear-report        B78: print what the city wears and where the interesting
                       lots are on screen, then exit
";

#[derive(Debug)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

fn need(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, ParseError> {
    it.next()
        .ok_or_else(|| ParseError(format!("{flag} needs a value")))
}

fn num<T: std::str::FromStr>(v: String, flag: &str) -> Result<T, ParseError> {
    v.parse::<T>()
        .map_err(|_| ParseError(format!("{flag}: not a number: {v}")))
}

pub fn parse_from<I: Iterator<Item = String>>(it: I) -> Result<Args, ParseError> {
    let mut a = Args::default();
    let mut mode: Option<Mode> = None;
    let mut it = it;

    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--png" => {
                a.path = need(&mut it, "--png")?;
                mode = Some(Mode::Png);
            }
            "--scene" => {
                let v = need(&mut it, "--scene")?;
                a.scene = match v.as_str() {
                    "empty" => SceneMode::Empty,
                    "test" => SceneMode::Test,
                    other => return Err(ParseError(format!("--scene: unknown kind: {other}"))),
                };
            }
            "--depth" => {
                let v = need(&mut it, "--depth")?;
                a.depth = match v.as_str() {
                    "perpixel" | "per-pixel" | "pixel" => DepthMode::PerPixel,
                    "anchor" => DepthMode::Anchor,
                    "off" | "none" => DepthMode::Off,
                    other => return Err(ParseError(format!("--depth: unknown mode: {other}"))),
                };
            }
            "--window" => mode = Some(Mode::Window),
            "--bench" => mode = Some(Mode::Bench),
            "--w" => a.w = num(need(&mut it, "--w")?, "--w")?,
            "--h" => a.h = num(need(&mut it, "--h")?, "--h")?,
            "--scale" => a.scale = num::<usize>(need(&mut it, "--scale")?, "--scale")?.max(1),
            "--fps" => a.fps = num(need(&mut it, "--fps")?, "--fps")?,
            "--seconds" => a.seconds = num(need(&mut it, "--seconds")?, "--seconds")?,
            "--frames" => a.frames = num(need(&mut it, "--frames")?, "--frames")?,
            "--steps" => a.steps = num(need(&mut it, "--steps")?, "--steps")?,
            "--seed" => a.seed = num(need(&mut it, "--seed")?, "--seed")?,
            "--tech-print" => a.tech_print = true,
            "--overlap-report" => a.overlap_report = true,
            "--wear-report" => a.wear_report = true,
            "--ending" => {
                let v = need(&mut it, "--ending")?;
                a.ending = Some(sim::ending::Ending::from_name(&v).ok_or_else(|| {
                    ParseError(format!("--ending: unknown ending '{v}'"))
                })?);
            }
            "--ending-at" => {
                let v = need(&mut it, "--ending-at")?;
                let (name, p) = match v.split_once(':') {
                    Some((n, p)) => (n.to_string(), p.parse::<f32>().map_err(|_| ParseError(format!("--ending-at: bad progress '{p}'")))?),
                    None => (v.clone(), 0.5),
                };
                let ph = sim::ending::Phase::from_name(&name)
                    .ok_or_else(|| ParseError(format!("--ending-at: unknown phase '{name}'")))?;
                a.ending_at = (ph, p);
            }
            "--ending-loop" => {
                a.ending_loop = Some(need(&mut it, "--ending-loop")?);
            }
            "--ending-step" => a.ending_step = num(need(&mut it, "--ending-step")?, "--ending-step")?,
            "--biome" => {
                let v = need(&mut it, "--biome")?;
                a.biome = Some(map::Biome::from_name(&v).ok_or_else(|| {
                    ParseError(format!("--biome: unknown land '{v}'"))
                })?);
            }
            "--stage-strip" => {
                a.stage_strip = Some(need(&mut it, "--stage-strip")?);
                mode = Some(Mode::Png);
            }
            "--speed" => a.speed = num(need(&mut it, "--speed")?, "--speed")?,
            "--time" => a.time = Some(need(&mut it, "--time")?),
            "--light" => a.light = Some(need(&mut it, "--light")?),
            "--date" => a.date = Some(need(&mut it, "--date")?),
            "--sky" => a.sky = num(need(&mut it, "--sky")?, "--sky")?,
            "--focus" => {
                let v = need(&mut it, "--focus")?;
                let parts: Vec<f32> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                if parts.len() != 2 {
                    return Err(ParseError("--focus needs X,Y".into()));
                }
                a.focus = Some((parts[0], parts[1]));
            }
            "--sim" => a.sim = true,
            "--sim-days" => a.sim_days = num(need(&mut it, "--sim-days")?, "--sim-days")?,
            "--pace" => a.pace = need(&mut it, "--pace")?,
            "--road-class" => a.road_class = true,
            "--wildlife" => a.wildlife = true,
            "--districts" => {
                a.districts = Some(need(&mut it, "--districts")?);
                mode = Some(Mode::Png);
            }
            "--timelapse" => a.timelapse = Some(need(&mut it, "--timelapse")?),
            "--dayloop" => a.dayloop = Some(need(&mut it, "--dayloop")?),
            "--start" => a.start_hour = num(need(&mut it, "--start")?, "--start")?,
            "--every" => a.every_min = num(need(&mut it, "--every")?, "--every")?,
            "--rain" => a.rain = true,
            "--water-loop" => a.water_loop = Some(need(&mut it, "--water-loop")?),
            "--walker-loop" => a.walker_loop = Some(need(&mut it, "--walker-loop")?),
            "--walkers" => {
                let v = need(&mut it, "--walkers")?;
                a.walkers = !matches!(v.to_lowercase().as_str(), "off" | "0" | "false" | "no");
            }
            "--water-bench" => a.water_bench = true,
            "--clouds-bench" => a.clouds_bench = true,
            "--landform-sheet" => a.landform_sheet = Some(need(&mut it, "--landform-sheet")?),
            "--wx-compare" => {
                a.wx_compare = Some(need(&mut it, "--wx-compare")?);
                mode = Some(Mode::Png);
            }
            "--force-emergency" => a.force_emergency = true,
            "--plane" => a.plane = true,
            "--want" => a.want = Some(need(&mut it, "--want")?),
            "--traffic-loop" => {
                a.traffic_loop = Some(need(&mut it, "--traffic-loop")?);
                mode = Some(Mode::Png);
            }
            "--weather" => {
                let v = need(&mut it, "--weather")?;
                if weather_fx::look::preset(&v.to_lowercase()).is_none() {
                    return Err(ParseError(format!(
                        "--weather: unknown kind '{v}' (clear|partly|overcast|drizzle|rain|snow|fog|storm|sandstorm)"
                    )));
                }
                a.weather = Some(v.to_lowercase());
            }
            "--water-anim" => {
                let v = need(&mut it, "--water-anim")?;
                a.water_anim = match v.to_lowercase().as_str() {
                    "on" | "1" | "true" => true,
                    "off" | "0" | "false" => false,
                    other => return Err(ParseError(format!("--water-anim expects on|off, got '{other}'"))),
                };
            }
            "-h" | "--help" => return Err(ParseError("help".into())),
            other => return Err(ParseError(format!("unknown argument: {other}"))),
        }
    }

    // A timelapse or a day loop is a PNG sequence, so it implies png mode.
    let mode = if (a.timelapse.is_some()
        || a.dayloop.is_some()
        || a.water_loop.is_some()
        || a.walker_loop.is_some()
        || a.wx_compare.is_some()
        || a.clouds_bench
        || a.landform_sheet.is_some()
        || a.water_bench
        || a.overlap_report)
        && mode.is_none()
    {
        Some(Mode::Png)
    } else {
        mode
    };
    a.mode =
        mode.ok_or_else(|| ParseError("pick a mode: --png PATH, --window or --bench".into()))?;
    if a.w < 32 || a.h < 32 {
        return Err(ParseError("--w/--h must be at least 32".into()));
    }
    Ok(a)
}

pub fn parse() -> Result<Args, ParseError> {
    parse_from(std::env::args().skip(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Result<Args, ParseError> {
        parse_from(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn png_mode() {
        let a = p(&["--png", "out.png", "--w", "320", "--h", "180", "--rain"]).unwrap();
        assert_eq!(a.mode, Mode::Png);
        assert_eq!(a.path, "out.png");
        assert_eq!((a.w, a.h), (320, 180));
        assert!(a.rain);
    }

    #[test]
    fn window_mode() {
        let a = p(&[
            "--window",
            "--fps",
            "10",
            "--seconds",
            "2.5",
            "--scale",
            "3",
        ])
        .unwrap();
        assert_eq!(a.mode, Mode::Window);
        assert_eq!(a.fps, 10);
        assert_eq!(a.seconds, 2.5);
        assert_eq!(a.scale, 3);
    }

    #[test]
    fn bench_mode() {
        let a = p(&["--bench", "--frames", "50", "--seed", "11"]).unwrap();
        assert_eq!(a.mode, Mode::Bench);
        assert_eq!(a.frames, 50);
        assert_eq!(a.seed, 11);
    }

    #[test]
    fn missing_mode_is_an_error() {
        assert!(p(&["--rain"]).is_err());
    }

    #[test]
    fn biome_override() {
        let a = p(&["--png", "x.png", "--biome", "desert"]).unwrap();
        assert_eq!(a.biome, Some(map::Biome::Desert));
        assert_eq!(a.biome, p(&["--png", "x.png", "--biome", "snow"]).unwrap().biome
            .map(|b| if b == map::Biome::Tundra { map::Biome::Desert } else { b }));
        assert!(p(&["--png", "x.png", "--biome", "lava"]).is_err());
    }

    #[test]
    fn missing_value_is_an_error() {
        assert!(p(&["--png"]).is_err());
    }
}
