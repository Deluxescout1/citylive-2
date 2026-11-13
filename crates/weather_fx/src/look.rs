//! The weather *book*: what the ground and the overlays are told (B27).
//!
//! `world::Weather` is a snapshot of the last Open-Meteo fetch (or of a
//! `--weather` override). A city, though, has **memory**: the roads are still
//! wet twenty minutes after it stops raining, and snow lies until it melts. This
//! module keeps that small amount of state and translates it into the two things
//! the renderer wants:
//!
//! * [`render::terrain::WeatherLook`] — cloud, wet roads, snow cover (read by the
//!   terrain and the buildings when they are rasterised);
//! * [`Fx`] — the overlay particles (rain streaks, flakes, fog, lightning).
//!
//! Everything is a pure function of the snapshots it has been fed, so replaying
//! the same weather gives the same city.

use render::terrain::WeatherLook;
use world::Weather;

use crate::Fx;

/// How fast the roads dry out (1 = bone dry), and how fast they soak.
const DRY_PER_SEC: f32 = 1.0 / 300.0;
const SOAK_PER_SEC: f32 = 1.0 / 120.0;
/// B76: snow settles over hours and melts over days. It used to reach full
/// cover in fifteen minutes of snow and be gone in half an hour, which read as
/// a switch, not a winter. Six hours to a full cover, two days to melt.
const SETTLE_PER_SEC: f32 = 1.0 / 21_600.0;
const MELT_PER_SEC: f32 = 1.0 / 172_800.0;
/// B76: a dry spell is 3 days of no rain to start showing and 4 more to be a
/// drought (the grass in the parks goes patchy yellow).
const DRY_START_SEC: f32 = 3.0 * 86_400.0;
const DRY_FULL_SEC: f32 = 4.0 * 86_400.0;

/// Weather with a memory.
#[derive(Debug, Clone, Copy)]
pub struct WeatherBook {
    /// Roads wetness, 0..1.
    pub wet: f32,
    /// Snow lying on the ground and roofs, 0..1.
    pub snow_cover: f32,
    /// Seconds since the last rain (B76).
    dry_secs: f64,
    /// Frost on the grass, 0..1, set from the hour and the temperature by the
    /// caller (B76).
    frost: f32,
    /// The last look handed to the renderer.
    pub look: WeatherLook,
    /// B67: the look follows the forecast through a spring (see [`crate::ease`]).
    ease: crate::ease::LookEase,
}

impl Default for WeatherBook {
    fn default() -> Self {
        Self {
            wet: 0.0,
            snow_cover: 0.0,
            dry_secs: 0.0,
            frost: 0.0,
            look: WeatherLook::default(),
            ease: crate::ease::LookEase::default(),
        }
    }
}

impl WeatherBook {
    /// B76: the frost this dawn should show (0..1). The caller knows the hour
    /// and the temperature; the book only carries it into the look.
    pub fn set_frost(&mut self, frost: f32) {
        self.frost = frost.clamp(0.0, 1.0);
    }

    /// Advance the book by `dt` seconds of a weather snapshot and return the look.
    pub fn update(&mut self, dt: f32, wx: &Weather) -> WeatherLook {
        let dt = dt.clamp(0.0, 5.0);
        let f = wx.flags();
        let raining = f.rain || f.drizzle || f.thunder;
        if raining {
            self.wet = (self.wet + dt * SOAK_PER_SEC * 4.0).min(1.0);
            self.dry_secs = 0.0;
        } else {
            self.wet = (self.wet - dt * DRY_PER_SEC).max(0.0);
            self.dry_secs += dt as f64;
        }
        // A reported snow depth wins; otherwise it settles while it snows.
        // B76: 20 cm is a full cover, so a 6 cm forecast starts as a thin layer
        // and thickens while it keeps snowing.
        let depth = wx.snow.unwrap_or(0.0);
        let target_from_depth = (depth / 0.20).clamp(0.0, 1.0);
        if f.snow {
            self.snow_cover = (self.snow_cover + dt * SETTLE_PER_SEC).min(1.0);
        } else {
            self.snow_cover = (self.snow_cover - dt * MELT_PER_SEC).max(0.0);
        }
        self.snow_cover = self.snow_cover.max(target_from_depth);
        if wx.temp > 4.0 {
            // Above 4 °C it is not lying for long — but a thaw still takes a
            // day or two (B76), not an afternoon.
            self.snow_cover = (self.snow_cover - dt * MELT_PER_SEC * 0.5).max(0.0);
        }
        let drought = (((self.dry_secs - DRY_START_SEC as f64) / DRY_FULL_SEC as f64) as f32).clamp(0.0, 1.0);

        let i = crate::Intensity::from_weather(wx);
        let raw = WeatherLook {
            cloud: (wx.cloud / 100.0).clamp(0.0, 1.0),
            rain: i.rain,
            snow: i.snow,
            fog: i.fog,
            storm: if i.thunder { 1.0 } else { 0.0 },
            wind_kmh: wx.wind,
            wind_dir: wx.wind_dir.unwrap_or(270.0),
            wet: self.wet,
            snow_cover: self.snow_cover,
            dust: i.dust,
            drought,
            frost: self.frost,
        };
        self.look = self.ease.step(&raw, dt);
        self.look
    }

    /// Jump straight to the next snapshot instead of easing to it (a preset the
    /// user just chose, tests).
    pub fn snap(&mut self) {
        self.ease.snap();
    }
}

/// A fixed weather for `--weather clear|rain|snow|fog|storm` (and for tests):
/// a plausible snapshot with the right WMO code, temperature and visibility.
pub fn preset(name: &str) -> Option<Weather> {
    let mut w = Weather {
        temp: 12.0,
        feels: 12.0,
        precip: 0.0,
        cloud: 20.0,
        wind: 8.0,
        gust: 14.0,
        wind_dir: Some(250.0),
        visibility: Some(20_000.0),
        snow: None,
        code: 0,
        proj: Default::default(),
    };
    match name {
        "clear" => {}
        "rain" => {
            w.code = 63; // moderate rain
            w.precip = 0.8;
            w.cloud = 95.0;
            w.temp = 11.0;
            w.feels = 10.0;
            w.visibility = Some(8_000.0);
            w.wind = 18.0;
            w.gust = 30.0;
        }
        // B61b: the light end of the rain scale, the case the wallpaper must
        // afford all day (WMO 51 = light drizzle, 0.2 mm/h).
        "drizzle" => {
            w.code = 51;
            w.precip = 0.22;
            w.cloud = 80.0;
            w.temp = 13.0;
            w.feels = 12.0;
            w.visibility = Some(12_000.0);
            w.wind = 10.0;
            w.gust = 16.0;
        }
        "snow" => {
            w.code = 73; // moderate snow
            w.precip = 0.7;
            w.cloud = 98.0;
            w.temp = -2.0;
            w.feels = -6.0;
            w.visibility = Some(4_000.0);
            w.wind = 12.0;
            w.gust = 20.0;
            w.snow = Some(0.06);
        }
        "fog" => {
            w.code = 45; // fog
            w.cloud = 90.0;
            w.temp = 6.0;
            w.feels = 5.0;
            w.visibility = Some(400.0);
            w.wind = 4.0;
        }
        "storm" => {
            w.code = 95; // thunderstorm
            w.precip = 2.5;
            w.cloud = 100.0;
            w.temp = 18.0;
            w.feels = 20.0;
            w.visibility = Some(5_000.0);
            w.wind = 38.0;
            w.gust = 65.0;
        }
        // D66/D68: a sandstorm in its own right (a desert world's answer to a
        // snowstorm). WMO 33 is a severe sandstorm.
        "sandstorm" => {
            w.code = 33;
            w.precip = 2.0;
            w.cloud = 85.0;
            w.temp = 96.0;
            w.feels = 99.0;
            w.visibility = Some(2_500.0);
            w.wind = 46.0;
            w.gust = 74.0;
            w.wind_dir = Some(200.0);
        }
        "partly" => {
            w.code = 2; // partly cloudy
            w.cloud = 55.0;
            w.temp = 14.0;
            w.feels = 14.0;
            w.visibility = Some(22_000.0);
        }
        "overcast" => {
            w.code = 3; // overcast
            w.cloud = 100.0;
            w.temp = 9.0;
            w.feels = 8.0;
            w.visibility = Some(14_000.0);
        }
        _ => return None,
    }
    Some(w)
}

/// Build the overlay for a framebuffer size.
pub fn overlay(seed: u64, w: usize, h: usize) -> Fx {
    Fx::new(seed, w, h)
}

/// D68: real weather, in the land's own idiom.
///
/// the owner's rule is that the biome wins: a snowstorm is a sandstorm on a
/// desert world, heavy rain in the jungle and a blizzard on the tundra. The real
/// time of day, cloud and wind still drive everything — only *what falls* is
/// translated, plus how hard.
pub fn for_biome(biome: map::Biome, wx: &Weather) -> Weather {
    let f = wx.flags();
    let mut w = *wx;
    let raining = f.rain || f.drizzle || f.thunder || f.snow;
    match biome {
        // A desert has no rain: what falls is dust. Snow is a cold, clear day
        // with dust blowing off the dunes.
        map::Biome::Desert => {
            if f.snow || f.rain || f.thunder || f.drizzle {
                w.code = if f.thunder || wx.precip > 1.2 { 33 } else { 31 };
                w.precip = (wx.precip.max(0.6) * 1.2).min(3.0);
                w.cloud = wx.cloud.max(70.0);
                w.wind = wx.wind.max(28.0);
                w.gust = wx.gust.max(48.0);
                w.visibility = Some(wx.visibility.unwrap_or(9_000.0).min(2_500.0));
                w.snow = None;
                // The land is hot regardless of what the forecast says.
                w.temp = w.temp.max(72.0);
            } else if f.fog {
                // Morning mist over the dunes: a dust haze, not a wet fog.
                w.code = 30;
                w.precip = 0.3;
                w.visibility = Some(900.0);
            } else {
                w.snow = None;
                w.cloud *= 0.85;
            }
        }
        // A tundra keeps its snow, harder: rain becomes snow below freezing, and a
        // blizzard is just what a windy snowfall is here.
        map::Biome::Tundra => {
            if f.rain || f.drizzle || f.thunder {
                w.code = if f.thunder { 75 } else { 73 };
                w.precip = wx.precip.max(0.5);
                w.snow = Some(0.04 + wx.precip * 0.02);
                w.temp = w.temp.min(26.0);
            }
            if f.snow || raining {
                w.wind = wx.wind.max(16.0);
                w.visibility = Some(wx.visibility.unwrap_or(8_000.0).min(2_500.0));
            }
            w.cloud = wx.cloud.max(45.0);
        }
        // Jungle rain is a downpour; snow never falls, it is just a cold rain.
        map::Biome::Tropical => {
            if f.snow {
                w.code = 80; // rain showers
                w.snow = None;
                w.precip = wx.precip.max(0.8);
            }
            if f.rain || f.drizzle || f.thunder || wx.code >= 80 {
                w.precip = (wx.precip * 1.8).max(0.6).min(4.0);
                w.cloud = wx.cloud.max(85.0);
                w.temp = w.temp.max(74.0);
                // A wet-season sky drops the light a stop.
                w.visibility = Some(wx.visibility.unwrap_or(12_000.0).min(6_000.0));
            } else if f.fog {
                // Jungle mist rather than a sea fog: warm and close.
                w.code = 45;
                w.visibility = Some(1_200.0);
            }
            w.snow = None;
        }
        // B46e: a valley holds its weather. Snow falls harder and lies, a cold
        // rain is a flurry, and a fog is the valley mist that sits till noon.
        map::Biome::Alpine => {
            if f.rain || f.drizzle {
                if wx.temp < 46.0 {
                    w.code = 71;
                    w.snow = Some(0.03 + wx.precip * 0.02);
                }
            }
            if f.snow {
                w.precip = (wx.precip * 1.25).min(3.0);
                w.wind = wx.wind.max(10.0);
            }
            if f.fog {
                w.visibility = Some(wx.visibility.unwrap_or(1_500.0).min(1_400.0));
            }
        }
        // B46e: a wet season and a dry one. Rain is a downpour, snow never
        // falls, a fog is the dust of the dry season rather than a wet mist.
        map::Biome::Savanna | map::Biome::Steppe => {
            if f.snow {
                w.code = 61;
                w.snow = None;
                w.precip = wx.precip.max(0.5);
            }
            if f.rain || f.drizzle || f.thunder {
                w.precip = (wx.precip * 1.5).clamp(0.6, 3.5);
                w.cloud = wx.cloud.max(80.0);
                w.temp = w.temp.max(66.0);
            } else if f.fog {
                w.code = 30;
                w.precip = 0.25;
                w.visibility = Some(2_200.0);
            } else {
                w.snow = None;
                w.cloud *= 0.8;
            }
        }
        // B46e: the wet land: mist at every dawn and dusk, drizzle rather than
        // a clean rain, snow a stranger.
        map::Biome::Swamp => {
            if f.snow {
                w.code = 51;
                w.snow = None;
                w.precip = wx.precip.max(0.4);
            }
            if f.fog {
                w.visibility = Some(wx.visibility.unwrap_or(900.0).min(700.0));
            } else if wx.cloud > 40.0 || f.drizzle {
                w.visibility = Some(wx.visibility.unwrap_or(9_000.0).min(3_500.0));
            }
            w.temp = w.temp.max(58.0);
        }
        // B46e: the open sea: always a breeze, short showers, no snow.
        map::Biome::Lagoon | map::Biome::Islands => {
            if f.snow {
                w.code = 80;
                w.snow = None;
                w.precip = wx.precip.max(0.5);
            }
            w.wind = wx.wind.max(7.0);
            if f.rain || f.drizzle {
                w.precip = (wx.precip * 0.8).max(0.3);
            }
            if f.fog {
                w.visibility = Some(wx.visibility.unwrap_or(1_500.0).min(1_200.0));
            }
        }
        // B46e: moor weather: drizzle and mist more than rain.
        map::Biome::Highlands => {
            if f.rain && !f.thunder && wx.precip < 2.0 {
                w.code = 53;
                w.precip = (wx.precip * 0.8).max(0.3);
            }
            if f.rain || f.drizzle {
                w.cloud = wx.cloud.max(75.0);
                w.visibility = Some(wx.visibility.unwrap_or(7_000.0).min(5_500.0));
            }
            if f.fog {
                w.visibility = Some(wx.visibility.unwrap_or(900.0).min(600.0));
            }
        }
        _ => {}
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dry-spell clock is long (a week) and ticks in frames: kept in f32 it
    /// stops resolving a 33 ms step past six days and runs twice too fast.
    #[test]
    fn the_dry_spell_clock_keeps_real_time_late_in_a_drought() {
        let mut b = WeatherBook::default();
        b.dry_secs = 540_000.0;
        let clear = preset("clear").unwrap();
        for _ in 0..3000 {
            b.update(0.033, &clear);
        }
        let ran = b.dry_secs - 540_000.0;
        assert!((ran - 99.0).abs() < 0.5, "99 s of dry weather counted as {ran} s");
    }

    #[test]
    fn rain_wets_the_roads_and_they_dry_again() {
        let mut b = WeatherBook::default();
        let rain = preset("rain").unwrap();
        let clear = preset("clear").unwrap();
        for _ in 0..120 {
            b.update(1.0, &rain);
        }
        assert!(b.wet > 0.9, "roads did not soak: {}", b.wet);
        assert!(b.look.rain > 0.3);
        for _ in 0..200 {
            b.update(1.0, &clear);
        }
        assert!(b.wet < 0.5, "roads did not dry: {}", b.wet);
        // B67: the rain itself dies away over minutes, it does not stop.
        assert!(b.look.rain > 0.0 && b.look.rain < 0.3, "rain {}", b.look.rain);
        for _ in 0..600 {
            b.update(1.0, &clear);
        }
        assert!(b.look.rain < 0.01);
    }

    /// B67: no weather change moves the look by more than a sliver in one tick,
    /// whatever the forecast does between two ticks.
    #[test]
    fn a_front_is_a_transition_not_a_step() {
        let clear = preset("clear").unwrap();
        let mut over = preset("rain").unwrap();
        over.cloud = 100.0;
        let mut b = WeatherBook::default();
        for _ in 0..30 {
            b.update(1.0, &clear);
        }
        let mut prev = b.look;
        let mut worst = (0.0f32, 0.0f32, 0.0f32);
        for _ in 0..1500 {
            let l = b.update(1.0, &over);
            worst.0 = worst.0.max((l.cloud - prev.cloud).abs());
            worst.1 = worst.1.max((l.rain - prev.rain).abs());
            worst.2 = worst.2.max((l.fog - prev.fog).abs());
            prev = l;
        }
        assert!(worst.0 < 0.006, "cover moved {} in one second", worst.0);
        assert!(worst.1 < 0.03, "rain moved {} in one second", worst.1);
        assert!(worst.2 < 0.03, "fog moved {} in one second", worst.2);
        assert!(prev.cloud > 0.95, "cover never arrived: {}", prev.cloud);
        // And back to sunny: also over minutes.
        let mut steps = 0;
        let mut last = prev.cloud;
        while b.look.cloud > clear.cloud / 100.0 + 0.05 && steps < 4000 {
            let l = b.update(1.0, &clear);
            assert!((l.cloud - last).abs() < 0.006);
            last = l.cloud;
            steps += 1;
        }
        assert!((400..1400).contains(&steps), "clearing took {steps} s");
    }

    #[test]
    fn snow_lies_and_melts() {
        let mut b = WeatherBook::default();
        let snow = preset("snow").unwrap();
        let clear = preset("clear").unwrap();
        // B76: hours to settle, not minutes.
        for _ in 0..5_040 {
            b.update(5.0, &snow); // 7 h
        }
        assert!(b.snow_cover > 0.5, "no snow on the ground: {}", b.snow_cover);
        // A day of thaw leaves most of it: it melts over days.
        for _ in 0..17_280 {
            b.update(5.0, &clear); // 24 h
        }
        assert!(
            (0.15..0.95).contains(&b.snow_cover),
            "a day of thaw should leave snow: {}",
            b.snow_cover
        );
        for _ in 0..50_000 {
            b.update(5.0, &clear);
        }
        assert!(b.snow_cover < 0.2, "snow never melted: {}", b.snow_cover);
    }

    /// B76: a long dry spell is remembered, real rain wipes it.
    #[test]
    fn a_dry_spell_is_remembered() {
        let clear = preset("clear").unwrap();
        let rain = preset("rain").unwrap();
        let mut b = WeatherBook::default();
        for _ in 0..7 * 86_400 / 5 {
            b.update(5.0, &clear);
        }
        assert!(b.look.drought > 0.95, "a week of sun is a drought: {}", b.look.drought);
        for _ in 0..120 {
            b.update(5.0, &rain); // 10 minutes of rain
        }
        assert!(b.look.drought < 0.05, "rain should end the dry spell: {}", b.look.drought);
    }

    /// The marginal cost of the overlay at 3520x720 (B27: "CPU with rain at
    /// 3520x720"). The scene, the sim and the present are measured elsewhere;
    /// this is what rain adds on top.
    ///
    /// `cargo test --release -p weather_fx -- --ignored --nocapture rain_cost`
    #[test]
    #[ignore = "timing"]
    fn rain_overlay_cost() {
        use std::time::Instant;
        let (w, h) = (3520usize, 720usize);
        let mut fb = render::Framebuffer::new(w, h);
        for name in ["clear", "rain", "snow", "storm"] {
            let wx = preset(name).unwrap();
            let mut fx = overlay(7, w, h);
            fx.update(0.016, &wx);
            let n = 120;
            let t0 = Instant::now();
            for _ in 0..n {
                fx.update(0.016, &wx);
                if fx.is_active() {
                    fx.draw(&mut fb);
                }
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
            println!(
                "{name}: active {} -> {ms:.3} ms per frame at {w}x{h} ({:.1} % of a core at 6 fps)",
                fx.is_active(),
                ms * 6.0 / 10.0
            );
        }
    }

    #[test]
    fn clear_weather_is_free() {
        let mut b = WeatherBook::default();
        let clear = preset("clear").unwrap();
        b.update(1.0, &clear);
        assert!(b.look.is_clear(), "{:?}", b.look);
    }
}
