//! B67: weather changes are transitions, never steps.
//!
//! The wallpaper is told what the weather is (a snapshot from the forecast
//! service) a few times an hour. Before B67 every channel followed the snapshot
//! at once: a front that Open-Meteo reports between two hours became a cloudy sky
//! turning sunny in a single frame. Each channel now follows its target through a
//! critically damped spring: it starts slowly, picks up, and settles slowly — no
//! overshoot, no pop, and an exact answer to "how long": a channel with time
//! constant `tau` has covered 95 % of a step after about `4.7 * tau`.

use crate::Intensity;
use render::terrain::WeatherLook;

/// `CITYLIVE_EASE_SPEED=10` plays every transition ten times faster, so a
/// twelve-minute front can be filmed in about a minute. A tool's aid: it is read
/// once, and the wallpaper never sets it.
fn speed() -> f32 {
    static S: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *S.get_or_init(|| {
        std::env::var("CITYLIVE_EASE_SPEED")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| *v > 0.0 && *v <= 100.0)
            .unwrap_or(1.0)
    })
}

/// One eased channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ease {
    pub x: f32,
    v: f32,
}

impl Ease {
    pub const fn at(x: f32) -> Self {
        Self { x, v: 0.0 }
    }

    /// Move towards `target` for `dt` seconds with time constant `tau` (seconds).
    /// Sub-stepped so a 1 s tick is as accurate as a 33 ms one.
    pub fn step(&mut self, target: f32, dt: f32, tau: f32) {
        let w = 1.0 / tau.max(0.05);
        let mut left = (dt * speed()).clamp(0.0, 30.0);
        while left > 1e-6 {
            let h = left.min(0.1 * tau.max(0.05)).min(0.25);
            let a = w * w * (target - self.x) - 2.0 * w * self.v;
            self.v += a * h;
            self.x += self.v * h;
            left -= h;
        }
        // Park on the target once it is inside a hair (so "settled" is exact and
        // the caller's change gates go quiet).
        if (target - self.x).abs() < 1e-4 && self.v.abs() < 1e-5 {
            self.x = target;
            self.v = 0.0;
        }
    }
}

/// The time constants (seconds), up and down, of every channel. A critically
/// damped channel has covered 95 % of a step after `4.7 * tau`.
#[derive(Clone, Copy, Debug)]
pub struct Taus {
    /// Cloud cover and everything derived from it (the dimming, the deck): 5 to
    /// 15 minutes.
    pub cover: f32,
    /// Rain: quick to start, slower to stop.
    pub rain_up: f32,
    pub rain_down: f32,
    pub snow_up: f32,
    pub snow_down: f32,
    pub fog_up: f32,
    pub fog_down: f32,
    pub storm_up: f32,
    pub storm_down: f32,
    pub dust: f32,
}

pub const TAUS: Taus = Taus {
    cover: 150.0,
    rain_up: 25.0,
    rain_down: 60.0,
    snow_up: 40.0,
    snow_down: 90.0,
    fog_up: 90.0,
    fog_down: 150.0,
    storm_up: 30.0,
    storm_down: 60.0,
    dust: 50.0,
};

fn pick(target: f32, now: f32, up: f32, down: f32) -> f32 {
    if target >= now { up } else { down }
}

/// The eased version of an [`Intensity`], for the overlay.
#[derive(Clone, Copy, Debug, Default)]
pub struct IntensityEase {
    rain: Ease,
    snow: Ease,
    fog: Ease,
    dim: Ease,
    shadow: Ease,
    dust: Ease,
    storm: Ease,
    primed: bool,
}

impl IntensityEase {
    /// Follow `raw` for `dt` seconds and return what the overlay should show.
    pub fn step(&mut self, raw: &Intensity, dt: f32) -> Intensity {
        let t = TAUS;
        if !self.primed {
            // The first look is the truth: a wallpaper that starts in the rain
            // starts raining.
            self.primed = true;
            self.rain = Ease::at(raw.rain);
            self.snow = Ease::at(raw.snow);
            self.fog = Ease::at(raw.fog);
            self.dim = Ease::at(raw.dim);
            self.shadow = Ease::at(raw.shadow);
            self.dust = Ease::at(raw.dust);
            self.storm = Ease::at(if raw.thunder { 1.0 } else { 0.0 });
        }
        self.rain.step(raw.rain, dt, pick(raw.rain, self.rain.x, t.rain_up, t.rain_down));
        self.snow.step(raw.snow, dt, pick(raw.snow, self.snow.x, t.snow_up, t.snow_down));
        self.fog.step(raw.fog, dt, pick(raw.fog, self.fog.x, t.fog_up, t.fog_down));
        self.dim.step(raw.dim, dt, t.cover);
        self.shadow.step(raw.shadow, dt, t.cover);
        self.dust.step(raw.dust, dt, t.dust);
        let st = if raw.thunder { 1.0 } else { 0.0 };
        self.storm.step(st, dt, pick(st, self.storm.x, t.storm_up, t.storm_down));
        Intensity {
            rain: self.rain.x,
            snow: self.snow.x,
            fog: self.fog.x,
            dim: self.dim.x,
            shadow: self.shadow.x,
            // Thunder is a flag for the lightning scheduler: it is on while the
            // storm has really built, and off once it has really faded.
            thunder: self.storm.x > 0.5,
            dust: self.dust.x,
            wind_x: raw.wind_x,
        }
    }

    /// Jump to `raw` (tests, and a `--weather` preset the user just asked for).
    pub fn snap(&mut self) {
        self.primed = false;
    }
}

/// The eased version of the book's [`WeatherLook`] (what the ground and the sky
/// read), at the same time constants.
#[derive(Clone, Copy, Debug, Default)]
pub struct LookEase {
    cloud: Ease,
    rain: Ease,
    snow: Ease,
    fog: Ease,
    storm: Ease,
    dust: Ease,
    wind: Ease,
    primed: bool,
}

impl LookEase {
    pub fn step(&mut self, raw: &WeatherLook, dt: f32) -> WeatherLook {
        let t = TAUS;
        if !self.primed {
            self.primed = true;
            self.cloud = Ease::at(raw.cloud);
            self.rain = Ease::at(raw.rain);
            self.snow = Ease::at(raw.snow);
            self.fog = Ease::at(raw.fog);
            self.storm = Ease::at(raw.storm);
            self.dust = Ease::at(raw.dust);
            self.wind = Ease::at(raw.wind_kmh);
        }
        self.cloud.step(raw.cloud, dt, t.cover);
        self.rain.step(raw.rain, dt, pick(raw.rain, self.rain.x, t.rain_up, t.rain_down));
        self.snow.step(raw.snow, dt, pick(raw.snow, self.snow.x, t.snow_up, t.snow_down));
        self.fog.step(raw.fog, dt, pick(raw.fog, self.fog.x, t.fog_up, t.fog_down));
        self.storm.step(raw.storm, dt, pick(raw.storm, self.storm.x, t.storm_up, t.storm_down));
        self.dust.step(raw.dust, dt, t.dust);
        self.wind.step(raw.wind_kmh, dt, 120.0);
        WeatherLook {
            cloud: self.cloud.x,
            rain: self.rain.x,
            snow: self.snow.x,
            fog: self.fog.x,
            storm: self.storm.x,
            wind_kmh: self.wind.x,
            dust: self.dust.x,
            ..*raw
        }
    }

    pub fn snap(&mut self) {
        self.primed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A step in the target is followed over minutes, never in one tick.
    #[test]
    fn cover_follows_a_step_over_minutes_and_never_pops() {
        let mut e = Ease::at(0.9);
        let mut prev = 0.9f32;
        let mut biggest = 0.0f32;
        let mut t95 = None;
        for s in 0..1800 {
            e.step(0.1, 1.0, TAUS.cover);
            biggest = biggest.max((prev - e.x).abs());
            prev = e.x;
            if t95.is_none() && (e.x - 0.1).abs() < 0.04 {
                t95 = Some(s + 1);
            }
        }
        // 95 % of a 0.8 step is 0.04 from the target: 4.7 tau = 705 s.
        let t95 = t95.expect("never arrived");
        assert!((600..820).contains(&t95), "95 % after {t95} s");
        assert!(biggest < 0.004, "one second moved the cover {biggest}");
        assert!((e.x - 0.1).abs() < 1e-3);
    }

    /// No overshoot: an eased cover never goes past its target.
    #[test]
    fn it_never_overshoots() {
        for (from, to) in [(0.0f32, 1.0f32), (1.0, 0.0), (0.3, 0.35)] {
            let mut e = Ease::at(from);
            for _ in 0..4000 {
                e.step(to, 1.0, TAUS.cover);
                let (lo, hi) = (from.min(to) - 1e-3, from.max(to) + 1e-3);
                assert!(e.x >= lo && e.x <= hi, "{from}->{to}: {}", e.x);
            }
        }
    }

    /// The same answer at 1 Hz and at 30 Hz.
    #[test]
    fn the_tick_rate_does_not_matter() {
        let (mut a, mut b) = (Ease::at(0.0), Ease::at(0.0));
        for _ in 0..300 {
            a.step(1.0, 1.0, TAUS.cover);
        }
        for _ in 0..9000 {
            b.step(1.0, 1.0 / 30.0, TAUS.cover);
        }
        assert!((a.x - b.x).abs() < 0.01, "{} vs {}", a.x, b.x);
    }

    /// Rain starts faster than it stops.
    #[test]
    fn rain_starts_quickly_and_stops_slowly() {
        let mut on = Ease::at(0.0);
        let mut off = Ease::at(1.0);
        let (mut t_on, mut t_off) = (0, 0);
        for s in 1..3000 {
            on.step(1.0, 1.0, pick(1.0, on.x, TAUS.rain_up, TAUS.rain_down));
            off.step(0.0, 1.0, pick(0.0, off.x, TAUS.rain_up, TAUS.rain_down));
            if t_on == 0 && on.x > 0.9 {
                t_on = s;
            }
            if t_off == 0 && off.x < 0.1 {
                t_off = s;
            }
        }
        assert!(t_on > 30 && t_on < t_off, "on {t_on} s, off {t_off} s");
    }
}
