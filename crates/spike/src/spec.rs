//! B62 tooling: the spectacle schedule, applied to a still.
//!
//! The spike reads the same `world::spectacle` schedules the wallpaper does and
//! hands the scene what is on at the `--date`/`--time`. For handback shots,
//! `CITYLIVE_SPEC=festival,market,parade,show,balloons,rainbow,bolt,fox,kite,
//! busker,landing` forces things on regardless of the calendar and the sky;
//! `CITYLIVE_SPEC_T=<0..1>` sets how far through the event it is (default mid).

use render::scene::{BalloonNow, SideKind, SideNow};
use render::spectacle::Events;
use render::{Rect, Scene};
use world::spectacle as ws;

fn forced() -> Vec<String> {
    std::env::var("CITYLIVE_SPEC")
        .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default()
}

fn forced_t() -> Option<f32> {
    std::env::var("CITYLIVE_SPEC_T").ok().and_then(|v| v.parse().ok())
}

fn civil_day(date: Option<&str>) -> i64 {
    if let Some(d) = date {
        let p: Vec<i64> = d.split('-').filter_map(|x| x.parse().ok()).collect();
        if p.len() == 3 {
            return world::model::days_from_civil(p[0] as i32, p[1] as i32, p[2] as i32);
        }
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    now.div_euclid(86400)
}

/// Apply the day's spectacle to the scene. `side_zones` are the framebuffer
/// rectangles of the side heads (empty on a single monitor).
pub fn apply(
    scene: &mut Scene,
    st: &sky::SkyState,
    date: Option<&str>,
    hour: f32,
    side_zones: &[Rect],
    lat: f64,
    lon: f64,
    elapsed: f32,
) {
    let force = forced();
    let on = |n: &str| force.iter().any(|f| f == n);
    let seed = scene.seed();
    let day = civil_day(date);
    let wx = scene.weather();
    let mid = forced_t();

    // The calendar's events, plus anything forced.
    let a = ws::active(seed, day, hour);
    let mut ev = Events {
        festival: a.festival,
        parade: a.parade,
        fireworks: a.fireworks,
        night_market: a.night_market,
    };
    if on("festival") {
        ev.festival = Some((mid.unwrap_or(0.5) + elapsed / 28800.0).min(0.99));
    }
    if on("market") {
        ev.night_market = Some(mid.unwrap_or(0.5));
    }
    if on("parade") {
        ev.parade = Some((mid.unwrap_or(0.55) + elapsed / 7200.0).min(0.99));
    }
    if on("show") {
        ev.fireworks = Some(mid.map_or(340.0, |t| t * 1100.0) + elapsed);
    }
    scene.set_events(ev, hour * 3600.0 + elapsed);
    if std::env::var("SIM_DEBUG").is_ok() {
        eprintln!(
            "spectacle: square {:?} avenue {} tiles, {} fields, show site {:?}",
            scene.festival_square(),
            scene.avenue_len(),
            scene.field_count(),
            scene.fireworks_site()
        );
    }

    // The sky: balloons, rainbow, storm.
    let sky = ws::SkyNow {
        sun_alt: st.sun_alt as f32,
        sun_az: st.sun_az as f32,
        cloud: st.cloud,
        wind_kmh: wx.wind_kmh,
        wind_from: wx.wind_dir,
        precip: wx.rain > 0.05 || wx.snow > 0.05,
        storm: wx.storm > 0.1,
        since_rain_min: if on("rainbow") { 12.0 } else { 9999.0 },
        lunar_eclipse: false,
    };
    let tz = (lon / 15.0).round() as f32;
    let sunrise = world::sun::times((day * 86400 + 43200) * 1000, lat, lon)
    .rise
    .map(|ms| ((ms / 1000).rem_euclid(86400) as f32 / 3600.0 + tz).rem_euclid(24.0))
    .unwrap_or(6.0);
    let mut balloons: Vec<BalloonNow> = Vec::new();
    let balloon_day = ws::balloon_morning(&sky, hour) || on("balloons");
    if balloon_day {
        let flights = ws::balloon_flights(seed, day, if on("balloons") { hour - 0.35 } else { sunrise });
        for (i, f) in flights.iter().enumerate() {
            let p = if on("balloons") {
                // Forced: spread the flights through different phases so a still
                // shows one lifting off, one cruising and one coming down.
                ([0.07, 0.45, 0.93, 0.6][i % 4] + elapsed / 2700.0).min(0.999)
            } else {
                match f.progress(hour) {
                    Some(p) => p,
                    None => continue,
                }
            };
            balloons.push(BalloonNow { progress: p, scheme: f.scheme, size: f.size, spot: f.spot, seed: (seed as u32) ^ (i as u32 * 7919) ^ 0xBA, land_in: None });
        }
    }
    let mut side: Vec<SideNow> = Vec::new();
    for (i, e) in ws::side_events(seed, day, side_zones.len().min(255) as u8).iter().enumerate() {
        let Some(p) = e.progress(hour) else { continue };
        let kind = match e.kind {
            ws::SideKind::FoxCrossing => SideKind::Fox,
            ws::SideKind::KiteFlying => SideKind::Kite,
            ws::SideKind::StreetMusician => SideKind::Musician,
            ws::SideKind::BalloonLanding => {
                if !balloon_day {
                    continue;
                }
                SideKind::Balloon
            }
        };
        side.push(SideNow { kind, progress: p, zone: e.head as usize, spot: e.spot, seed: (seed as u32) ^ i as u32, seconds: e.seconds });
    }
    for (name, kind, spot) in [("fox", SideKind::Fox, 0.2), ("kite", SideKind::Kite, 0.5), ("busker", SideKind::Musician, 0.82)] {
        if on(name) && !side_zones.is_empty() {
            side.push(SideNow { kind, progress: (mid.unwrap_or(0.5) + elapsed / 120.0).min(0.99), zone: 0, spot, seed: 77, seconds: 120.0 });
        }
    }
    if on("landing") && !side_zones.is_empty() {
        balloons.push(BalloonNow { progress: (mid.unwrap_or(0.9) + elapsed / 2700.0).min(0.999), scheme: 2, size: 1.0, spot: 0.5, seed: 4242, land_in: Some(0) });
    }
    for s in &side {
        if s.kind == SideKind::Balloon {
            balloons.push(BalloonNow { progress: s.progress, scheme: 3, size: 1.0, spot: s.spot, seed: s.seed ^ 0x77, land_in: Some(s.zone) });
        }
    }
    let rain = ws::rainbow(&sky);
    // The bow's centre: the point opposite the sun, mapped across the frame (the
    // camera looks south; east is screen-left).
    let anti = (sky.sun_az + 180.0).rem_euclid(360.0);
    let rel = ((anti - 180.0 + 180.0).rem_euclid(360.0)) - 180.0;
    let cx = (0.5 + rel / 190.0).clamp(0.2, 0.8);
    scene.set_sky_spectacle(
        &balloons,
        sky.wind_from,
        sky.wind_kmh,
        if on("rainbow") { rain.max(0.8) } else { rain },
        cx,
        ws::spectacular_storm(seed, day) || on("bolt"),
    );
    scene.set_side(side_zones, &side);
    if on("bolt") {
        // A forced strike: the weather layer is not running in a still.
        scene.force_bolt();
    }
}
