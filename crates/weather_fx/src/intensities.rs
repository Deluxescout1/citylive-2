//! Mapping from a [`world::Weather`] snapshot to per-effect intensities (all
//! weights in 0..1).  Pure and allocation-free; called once per `update`.
//!
//! Rain and snow intensity come from the precipitation rate (mm per the
//! Open-Meteo 15-min nowcast bucket: ~1.5 mm is a heavy shower), fog from the
//! fog flag or the reported visibility, dimming and cloud shadows from cloud
//! cover, and the wind slant from wind speed + direction.

use world::Weather;

/// Screen-space horizontal px per km/h of wind (the slant).  At 30 km/h that
/// is ~120 px/s drift against a 420-760 px/s rain fall — clearly slanted,
/// still gentle, matching the old CityLive `wfx` look.
pub const WIND_SLOPE: f32 = 4.0;

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// The per-effect weights for one frame.  `Flash`'s envelope is not part of
/// this (it is scheduled, not weather-derived).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Intensity {
    /// Rain density 0..1 (1 = downpour; the pool is full).
    pub rain: f32,
    /// Snow density 0..1.
    pub snow: f32,
    /// Mist strength 0..1 (drives the bottom-of-frame wash).
    pub fog: f32,
    /// Overcast dimming: 0 below 40 % cloud, 1.0 at 100 % cloud.
    pub dim: f32,
    /// Cloud-shadow blobs: 0 below 50 % cloud, 1.0 at 100 %.
    pub shadow: f32,
    /// WMO code ≥ 95: lightning strikes are scheduled.
    pub thunder: bool,
    /// Sandstorm density 0..1 (D66/D68: the desert's idiom for rain).
    pub dust: f32,
    /// Horizontal drift the particles should pick up, px/s, screen space
    /// (negative = left).  The camera faces south (H12 mapping): east is
    /// screen-left, so a wind *from the west* (dir 270°) pushes particles to
    /// the left, exactly like the legacy overlay.
    pub wind_x: f32,
}

impl Intensity {
    /// Derive the current weights from a live weather snapshot.
    pub fn from_weather(x: &Weather) -> Self {
        let f = x.flags();

        let mut rain = 0.0f32;
        if f.rain {
            // ~1.5 mm per 15-min bucket reads as a downpour.
            rain = clamp01(x.precip / 1.5);
        }
        if f.drizzle {
            // Drizzle never denser than ~half a pool, and always a little.
            rain = rain.max(clamp01(0.2 + 0.35 * x.precip)).min(0.55);
        }
        if f.thunder {
            // Storm codes (95-99) don't set the rain flag themselves, but a
            // thunderstorm is at least a brisk downpour.
            rain = rain.max(0.6).max(clamp01(x.precip / 1.2).min(1.0));
        }

        let snow = if f.snow {
            clamp01(x.precip / 0.8)
        } else {
            0.0
        };

        // A dust storm (WMO 30-35) blows like a downpour made of sand.
        let dust = if f.dust {
            clamp01(0.35 + x.precip / 1.6)
        } else {
            0.0
        };

        // Fog flag wins (it *is* fog); otherwise low visibility mists the
        // scene (heavy rain, dust, coastal air): 15 km → 0, 2 km → 1. B76: a
        // snowfall already veils the frame with its own flakes and light, so it
        // gets a quarter of that wash — at four kilometres of visibility snow
        // used to be drawn under an 85 % fog bank, which hid the snow itself.
        let mut fog = if f.fog {
            1.0
        } else {
            x.visibility.map_or(0.0, |v| clamp01((15_000.0 - v) / 13_000.0))
        };
        if f.snow && !f.fog {
            fog *= 0.25;
        }

        let dim = clamp01((x.cloud - 40.0) / 60.0);
        let shadow = clamp01((x.cloud - 50.0) / 50.0);

        // wind_dir is "from" (0 = N, 90 = E, ...).  Blowing-toward = dir + 180,
        // its east component is sin(to) = -sin(dir); screen-left is east, so
        // screen vx = +sin(dir) · wind · slope.  Missing direction defaults to
        // a west wind (270°), i.e. the legacy leftward slant.
        let dir = x.wind_dir.unwrap_or(270.0);
        let wind_x = dir.to_radians().sin() * x.wind * WIND_SLOPE;

        Self {
            rain,
            snow,
            fog,
            dim,
            shadow,
            thunder: f.thunder,
            dust,
            wind_x,
        }
    }

    /// Any *weather-derived* effect to draw (excludes the scheduled flash).
    pub fn any(&self) -> bool {
        self.rain > 0.01
            || self.snow > 0.01
            || self.fog > 0.01
            || self.dust > 0.01
            || self.dim > 0.004
            || self.shadow > 0.004
    }
}
