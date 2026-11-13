use std::env;
use std::path::PathBuf;
use std::process;

use present_gnome::{run, WallpaperGnomeConfig};
use render::SceneMode;

const USAGE: &str = "\
citylive-iso-gnome - CityLive Iso live wallpaper on GNOME Wayland

Usage: citylive-iso-gnome [OPTIONS]

Options:
  --scene <KIND>     sim   = live growing settlement/village (default)
                     empty = generated land only (D20)
                     test  = land + roads, buildings, cars and people
  --seed <N>         world and scene seed (default 7)
  --pace <PACE>      sim pace preset: day, week, month (default month)
  --speed <X>        multiplier for simulation clock rate (default 1.0)
  --sim-days <D>     fast-forward initial founding to D sim days (default 0.0)
  --save <PATH>      save file location (default: $XDG_DATA_HOME/citylive-iso/city.sav)
  --fps <N>          target frame rate cap (default 30, 0 = uncapped)
  --seconds <N>      exit cleanly after N seconds (default 0 = run until stopped)
  --rain             enable rain overlay
  --stats            print frame times, CPU% and RSS every 10 s and on exit
  -h, --help         print this help message
";

fn parse_args() -> Result<WallpaperGnomeConfig, String> {
    let mut cfg = WallpaperGnomeConfig::default();
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
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
            "--seed" => {
                let val = args.next().ok_or("missing value for --seed")?;
                cfg.seed = val.parse().map_err(|e| format!("invalid --seed '{val}': {e}"))?;
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
            "--rain" => {
                cfg.rain = true;
            }
            "--stats" => {
                cfg.stats = true;
            }
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
            other => return Err(format!("unknown argument '{other}'\n\n{USAGE}")),
        }
    }

    Ok(cfg)
}

fn main() {
    let cfg = match parse_args() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Error: {err}");
            process::exit(1);
        }
    };

    if let Err(err) = run(cfg) {
        eprintln!("Error running citylive-iso-gnome: {err}");
        process::exit(1);
    }
}
