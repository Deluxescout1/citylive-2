//! B62: the wallpaper's side of the spectacle layer.
//!
//! Every animation tick reads what the calendar and the sky schedule
//! (`world::spectacle`) and hands the scene plain data: which events are on,
//! which balloons are aloft, whether a rainbow hangs, which side-monitor moment
//! is playing. A day with nothing on costs one cheap check a second.

use std::time::{Duration, Instant};

use render::scene::{BalloonNow, SideKind, SideNow};
use render::spectacle::Events;
use world::spectacle as ws;

use crate::WallpaperState;

/// The side heads as framebuffer rectangles: every output but the widest (the
/// main desk). On a single monitor there are none.
fn side_zones(state: &WallpaperState) -> Vec<render::Rect> {
    let Some(layout) = state.current_union else { return Vec::new() };
    if state.heads.len() < 2 {
        return Vec::new();
    }
    let main = state
        .heads
        .iter()
        .enumerate()
        .max_by_key(|(_, h)| h.logical_size.0)
        .map(|(i, _)| i);
    let scale = layout.scale.max(1) as i32;
    let mut zones: Vec<render::Rect> = state
        .heads
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != main)
        .map(|(_, h)| {
            let x0 = (h.logical_pos.0 - layout.min_x) / scale;
            let x1 = x0 + h.logical_size.0 as i32 / scale;
            render::Rect::new(x0, 0, x1, 100_000)
        })
        .collect();
    zones.sort_by_key(|r| r.x0);
    zones
}

impl WallpaperState {
    /// One spectacle step (called every animation tick; cheap when quiet).
    pub fn spectacle_step(&mut self) {
        let Some(sk) = self.spec_sky else { return };
        let now = Instant::now();
        let (seed, day, hour) = (self.config.seed, sk.day, sk.hour);
        let mut ev = ws::active(seed, day, hour);
        // B61d: `CITYLIVE_SPEC=festival,parade,show,market` forces events on for
        // a measurement run, the way the spike's tooling does. A developer hook,
        // read once at startup, so a bench can measure a festival afternoon or a
        // fireworks show without hunting the calendar for the right day.
        if let Some(forced) = self.spec_forced.as_ref() {
            let mid = self.spec_forced_t;
            let on = |n: &str| forced.iter().any(|f| f == n);
            if on("festival") {
                ev.festival = Some(mid);
            }
            if on("market") {
                ev.night_market = Some(mid);
            }
            if on("parade") {
                ev.parade = Some(mid);
            }
            if on("show") {
                ev.fireworks = Some(mid * 1100.0);
            }
        }
        let live = ev.any()
            || self.scene.spec_live()
            || ws::balloon_morning(&sk.now, hour)
            || ws::rainbow(&sk.now) > 0.0;
        // A quiet day: look once a second, not every frame.
        if !live && now < self.spec_next {
            return;
        }
        self.spec_next = now + Duration::from_millis(if live { 0 } else { 1000 });
        let zones = side_zones(self);
        self.scene.set_events(
            Events {
                festival: ev.festival,
                parade: ev.parade,
                fireworks: ev.fireworks,
                night_market: ev.night_market,
            },
            hour * 3600.0,
        );
        // Feed lines: once, when an event begins.
        let mut told = |kind: ws::EventKind, on: bool, state: &mut WallpaperState| {
            if !on || state.spec_told.contains(&(kind as u8, day)) {
                return;
            }
            state.spec_told.push((kind as u8, day));
            state.spec_told.retain(|(_, d)| *d >= day - 1);
            if let Some(sim) = state.sim.as_mut() {
                sim.note_city_line(&ws::feed_line(kind, seed, day));
            }
        };
        told(ws::EventKind::StreetFestival, ev.festival.is_some(), self);
        told(ws::EventKind::Parade, ev.parade.is_some(), self);
        told(ws::EventKind::Fireworks, ev.fireworks.is_some(), self);
        told(ws::EventKind::NightMarket, ev.night_market.is_some(), self);

        // Balloons on a calm clear morning, drifting with the real wind.
        if self.spec_sunrise.0 != day {
            let ms = (day * 86400 + 43200) * 1000;
            let tz = (self.config.lon / 15.0).round() as f32;
            let rise = world::sun::times(ms, self.config.lat, self.config.lon)
                .rise
                .map(|r| ((r / 1000).rem_euclid(86400) as f32 / 3600.0 + tz).rem_euclid(24.0))
                .unwrap_or(6.0);
            self.spec_sunrise = (day, rise);
        }
        let balloon_day = ws::balloon_morning(&sk.now, hour);
        let mut balloons: Vec<BalloonNow> = Vec::new();
        if balloon_day {
            for (i, f) in ws::balloon_flights(seed, day, self.spec_sunrise.1).iter().enumerate() {
                if let Some(p) = f.progress(hour) {
                    balloons.push(BalloonNow {
                        progress: p,
                        scheme: f.scheme,
                        size: f.size,
                        spot: f.spot,
                        seed: (seed as u32) ^ (i as u32 * 7919) ^ day as u32,
                        land_in: None,
                    });
                }
            }
        }
        // Side-monitor moments: gentle, rare, never in rain.
        let mut side: Vec<SideNow> = Vec::new();
        if !sk.now.precip && !sk.now.storm {
            for (i, e) in ws::side_events(seed, day, zones.len().min(255) as u8).iter().enumerate() {
                let Some(p) = e.progress(hour) else { continue };
                let kind = match e.kind {
                    ws::SideKind::FoxCrossing => SideKind::Fox,
                    ws::SideKind::KiteFlying if sk.now.wind_kmh > 5.0 => SideKind::Kite,
                    ws::SideKind::KiteFlying => continue,
                    ws::SideKind::StreetMusician => SideKind::Musician,
                    ws::SideKind::BalloonLanding if balloon_day => SideKind::Balloon,
                    ws::SideKind::BalloonLanding => continue,
                };
                side.push(SideNow { kind, progress: p, zone: e.head as usize, spot: e.spot, seed: (seed as u32) ^ (i as u32) ^ day as u32, seconds: e.seconds });
            }
        }
        for s in &side {
            if s.kind == SideKind::Balloon {
                balloons.push(BalloonNow { progress: s.progress, scheme: 3, size: 1.0, spot: s.spot, seed: s.seed ^ 0x77, land_in: Some(s.zone) });
            }
        }
        // The rainbow's centre: opposite the sun (the camera looks south, east is
        // screen-left).
        let anti = (sk.now.sun_az + 180.0).rem_euclid(360.0);
        let cx = (0.5 + (anti - 180.0) / 190.0).clamp(0.2, 0.8);
        self.scene.set_sky_spectacle(
            &balloons,
            sk.now.wind_from,
            sk.now.wind_kmh,
            ws::rainbow(&sk.now),
            cx,
            ws::spectacular_storm(seed, day),
        );
        self.scene.set_side(&zones, &side);
    }
}
