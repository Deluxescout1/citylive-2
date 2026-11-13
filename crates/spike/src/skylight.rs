//! Fixed-clock sky for the spike: build the sky layer and the ambient light
//! from a `--time`/`--date` override (D21: real time, real sun/moon).

use render::Framebuffer;
use sky::{palette::palette, state};

/// Greenwich, London — D16's offline fallback until the settings app exists.
pub const LAT: f64 = 51.48;
pub const LON: f64 = 0.0;

/// Epoch milliseconds for "today at HH:MM", or a fixed `--date`.
pub fn clock_ms(time: Option<&str>, date: Option<&str>) -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let mut secs = now / 1000;
    if let Some(d) = date {
        // YYYY-MM-DD -> days since epoch (civil-from-days, no chrono).
        let parts: Vec<i64> = d.split('-').filter_map(|p| p.parse().ok()).collect();
        if parts.len() == 3 {
            let days = days_from_civil(parts[0], parts[1] as u32, parts[2] as u32);
            secs = days * 86400 + secs.rem_euclid(86400);
        }
    }
    if let Some(t) = time {
        let parts: Vec<i64> = t.split(':').filter_map(|p| p.parse().ok()).collect();
        if parts.len() >= 2 {
            let day = secs.div_euclid(86400);
            // `--time` is *local* time at the location: convert to UTC by
            // subtracting the timezone offset. the test site is at lon -75, so
            // round(lon/15) = -5 h (standard time; DST is ignored on purpose).
            let tz = (LON / 15.0).round() as i64;
            secs = day * 86400 + parts[0] * 3600 + parts[1] * 60 - tz * 3600;
        }
    }
    secs * 1000
}

/// Howard Hinnant's civil-from-days.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Re-bake the sky with a weather look: more cloud, and an overcast dimming of
/// the ambient light (B27 item 2). The sun/moon stay where they are.
pub fn apply_weather(scene: &mut render::Scene, st: &sky::SkyState, look: render::terrain::WeatherLook) {
    let mut st = *st;
    st.cloud = look.cloud;
    let pal = palette(&st);
    if scene.sky_height() == 0 {
        return;
    }
    let mut fb = Framebuffer::new(scene.width(), scene.sky_height());
    scene.set_horizon_air(sky::backdrop::air_colour(&pal));
    // A one-shot render: a throwaway night cache, a small town's glow.
    let mut night = sky::draw::NightSky::default();
    sky::draw::draw_with_treatment(
        &st,
        &pal,
        &mut fb,
        scene.seed(),
        scene.sky_height() as i32,
        0,
        scene.horizon(),
        &mut night,
        0.25,
    );
    // Overcast dims the light, fog dims more, lying snow lifts it, and a
    // sandstorm warms and dims it (D66) — one rule, in `WeatherLook::tint`.
    let tint = look.tint(pal.ambient_tint);
    scene.set_light(tint, pal.window_glow);
    // B60: the sun and moon still cast, from where they are.
    let mut ls = scene.light_state();
    ls.cloud = look.cloud;
    scene.set_light_state(ls);
    // B57: the weather's own clouds, in this hour's light.
    sky::clouds::update(scene, &st, &pal, look.storm, look.wind_dir, look.wind_kmh);
    // D66: a sandstorm's air is the land's, not the hour's alone.
    render::biome::dust_sky(&mut fb, look.dust);
    // D66: a sandstorm's air is the land's, not the hour's alone.
    render::biome::dust_sky(&mut fb, look.dust);
    scene.set_sky(fb);
    scene.set_water_brightness(
(0.2126 * tint[0] + 0.7152 * tint[1] + 0.0722 * tint[2]).clamp(0.0, 1.0),

    );
    scene.set_water_tint(render::palette::mix(pal.horizon, pal.zenith, 0.25));
}

/// B28: the season for a render's `--date` (default: today), at the configured
/// latitude, handed to the scene so the trees, grass and fields are that day.
pub fn apply_season(scene: &mut render::Scene, date: Option<&str>, lat: f64) {
    // (lat comes from the caller: `--lat` when given, the default place otherwise.)
    let (y, m, d) = match date.and_then(parse_date) {
        Some(v) => v,
        None => today_ymd(),
    };
    let sl = world::SeasonLook::at(&world::Date::new(y, m, d, 12), lat);
    let look = render::terrain::SeasonLook {
        autumn: sl.autumn,
        bareness: sl.bareness,
        blossom: sl.blossom,
        fresh: sl.fresh,
        litter: sl.litter,
        grass_dry: sl.grass_dry,
        field_stage: sl.field_stage,
        green: sl.green,
        turn: sl.turn,
    };
    // D68: the land's own expression of the real date.
    let look = render::biome::season_for(scene.biome(), look);
    scene.set_season(look);
}

/// B76: frost on a cold, clear dawn. The still and the wallpaper share the
/// rule: a winter dawn (04:00-09:00) under a clear sky rimes the grass.
pub fn apply_frost(scene: &mut render::Scene, time: Option<&str>, date: Option<&str>) {
    let (y, m, d) = date.and_then(parse_date).unwrap_or_else(today_ymd);
    let _ = y;
    let hour = time
        .and_then(|t| t.split(':').next().and_then(|h| h.parse::<f32>().ok()))
        .unwrap_or(12.0);
    let winter = matches!(m, 12 | 1 | 2);
    let cold_dawn = winter && (4.0..9.0).contains(&hour) && scene.weather().is_clear();
    let frost = if cold_dawn {
        (1.0 - (hour - 4.0) / 6.0).clamp(0.25, 1.0)
    } else {
        0.0
    };
    if (scene.weather().frost - frost).abs() > 0.02 {
        let mut w = scene.weather();
        w.frost = frost;
        scene.set_weather(w);
    }
    let _ = (m, d);
}

fn parse_date(s: &str) -> Option<(i32, u8, u8)> {
    let parts: Vec<i64> = s.split('-').filter_map(|p| p.parse().ok()).collect();
    if parts.len() == 3 {
        Some((parts[0] as i32, parts[1] as u8, parts[2] as u8))
    } else {
        None
    }
}

/// Today's local date from the clock (no chrono).
fn today_ymd() -> (i32, u8, u8) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    // Civil-from-days (Howard Hinnant), the inverse of `days_from_civil`.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m as u8, d as u8)
}

/// The holiday the city dresses for, and whether it is a market day (B36).
/// Public holidays only (D60); the decorations come from the ported HOLIDECOR.
pub fn holiday_look(
    time: Option<&str>,
    date: Option<&str>,
) -> (render::holiday::HolidayLook, bool) {
    use world::calendar;
    let (y, m, d) = date.and_then(parse_date).unwrap_or_else(today_ymd);
    let hour = time
        .and_then(|t| t.split(':').next().and_then(|h| h.parse::<u8>().ok()))
        .unwrap_or(12);
    let day = world::model::Date::new(y, m, d, hour);
    // B76 r2: the dressing follows the light season (1 Dec - 6 Jan).
    let in_season = calendar::holiday_lights(m, d) > 0.0;
    let decorated = calendar::decorated(&day).or(if in_season {
        Some(calendar::Holiday::Xmas)
    } else {
        None
    });
    let look = match decorated {
        Some(h) => match calendar::decor(h) {
            Some(dc) => render::holiday::HolidayLook::new(
                match h {
                    calendar::Holiday::Xmas => render::holiday::Holiday::Xmas,
                    calendar::Holiday::Halloween => render::holiday::Holiday::Halloween,
                    calendar::Holiday::July4 => render::holiday::Holiday::July4,
                    calendar::Holiday::Nye => render::holiday::Holiday::Nye,
                    calendar::Holiday::Valentine => render::holiday::Holiday::Valentine,
                    calendar::Holiday::StPatricks => render::holiday::Holiday::StPatricks,
                    calendar::Holiday::Thanks => render::holiday::Holiday::Thanks,
                    calendar::Holiday::Easter => render::holiday::Holiday::Easter,
                    _ => render::holiday::Holiday::None,
                },
                dc.garland,
                dc.tint,
            ),
            None => render::holiday::HolidayLook::default(),
        },
        None => render::holiday::HolidayLook::default(),
    };
    // A market day is a weekend in daylight (B36).
    let wd = day.weekday();
    let market = (wd == 0 || wd == 6) && (9..18).contains(&hour);
    (look, market)
}

/// `CITYLIVE_SIDE=x0:x1,x0:x1`: the framebuffer columns of the side heads, for
/// stills of the side-monitor moments (a 7040-wide frame has 5120:7040).
pub fn side_zones() -> Vec<render::Rect> {
    std::env::var("CITYLIVE_SIDE")
        .map(|s| {
            s.split(',')
                .filter_map(|z| {
                    let (a, b) = z.split_once(':')?;
                    Some(render::Rect::new(a.parse().ok()?, 0, b.parse().ok()?, 100_000))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// B99: the window glow the sky has at this time and date (what the building
/// sprites' day/night bucket follows).
pub fn glow_at(time: Option<&str>, date: Option<&str>) -> f32 {
    let ms = clock_ms(time, date);
    palette(&state(ms, LAT, LON)).window_glow
}

/// Draw the sky layer and hand the scene its ambient light.
pub fn apply(
    scene: &mut render::Scene,
    time: Option<&str>,
    date: Option<&str>,
    water_anim: bool,
) -> sky::SkyState {
    let ms = clock_ms(time, date);
    let st = state(ms, LAT, LON);
    let pal = palette(&st);
    // A scene with no sky band (sky_height 0) still needs the light, but no sky.
    if scene.sky_height() > 0 {
        let mut fb = Framebuffer::new(scene.width(), scene.sky_height());
        // D43: the sky ends at the land's top edge, so the sun sets behind the
        // distant ridges rather than over the city.
        scene.set_horizon_air(sky::backdrop::air_colour(&pal));
        // A one-shot render: a throwaway night cache, a small town's glow.
        let mut night = sky::draw::NightSky::default();
        sky::draw::draw_with_treatment(
            &st,
            &pal,
            &mut fb,
            scene.seed(),
            scene.sky_height() as i32,
            0,
            scene.horizon(),
            &mut night,
            0.25,
        );
        scene.set_sky(fb);
    }
    // B60/D74: the whole light model, so the spike's PNG shows what the
    // wallpaper shows — shadows that turn with the real sun.
    let mut ls = render::light::LightState::from_sky(
        st.sun_alt,
        st.sun_az,
        st.moon_alt,
        st.moon_az,
        st.moon_illum,
    );
    ls.ambient = pal.ambient_tint;
    ls.glow = pal.window_glow;
    ls.cloud = st.cloud;
    ls.dir_strength = pal.light_dir_strength;
    // B99: which windows are lit follows the clock (B58b item 7); the spike left
    // the default (noon), so its night stills lit the wrong share of windows.
    ls.hour = sky::local_hour(ms, LON);
    scene.set_light_state(ls);
    // B57/D72: clouds — the cover, the wind and this hour's ink. Drawn per frame
    // by the scene, so a still PNG shows the same weather the wallpaper does.
    let wx = scene.weather();
    sky::clouds::update(scene, &st, &pal, wx.storm, wx.wind_dir, wx.wind_kmh);
    // B36: the holiday dressing, the weekend market and the holiday parade.
    let (mut hol, market) = holiday_look(time, date);
    // B53 tooling: `CITYLIVE_LOOK=bills|billswin|banner|balloons|fireworks|flowers`
    // forces a gameday or personal-date look for the handback shots.
    if let Ok(name) = std::env::var("CITYLIVE_LOOK") {
        use render::holiday::{Holiday, HolidayLook};
        let (k, garland): (Holiday, &[[u8; 3]]) = match name.as_str() {
            "bills" => (Holiday::Bills, &[[0x1f, 0x3c, 0x88], [0xc8, 0x1e, 0x2e]]),
            "billswin" => (Holiday::BillsWin, &[[0x1f, 0x3c, 0x88], [0xc8, 0x1e, 0x2e]]),
            "banner" => (Holiday::Banner, &[[0xf0, 0xc0, 0x30], [0x30, 0x90, 0xe0]]),
            "balloons" => (
                Holiday::Balloons,
                &[[0xe0, 0x30, 0x60], [0x30, 0x90, 0xe0], [0xf0, 0xc0, 0x30]],
            ),
            "fireworks" => (Holiday::Fireworks, &[[0xff, 0xd0, 0x60], [0xff, 0x60, 0x90]]),
            "flowers" => (
                Holiday::Flowers,
                &[[0xff, 0x8a, 0xb0], [0xf5, 0xd0, 0x40], [0xe0, 0x60, 0xa0]],
            ),
            _ => (Holiday::None, &[]),
        };
        if k != Holiday::None {
            hol = HolidayLook::new(k, garland, 0.0);
        }
    }
    // B76: the seasonal lights are the date's, not a single holiday's.
    {
        let (y, m, d) = date.and_then(parse_date).unwrap_or_else(today_ymd);
        let _ = y;
        hol.lights = world::calendar::holiday_lights(m, d);
    }
    scene.set_holiday(hol);
    scene.set_market(market);
    // A parade runs on a decorated day between 11:00 and 13:00.
    let hour_f = time
        .and_then(|t| {
            let mut p = t.split(':');
            let h: f32 = p.next()?.parse().ok()?;
            let m: f32 = p.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
            Some(h + m / 60.0)
        })
        .unwrap_or(12.0);
    let parade = if hol.holiday != render::holiday::Holiday::None
        && (11.0..13.0).contains(&hour_f)
    {
        Some(((hour_f - 11.0) / 2.0).clamp(0.0, 0.999))
    } else {
        None
    };
    scene.set_parade(parade);
    // The fireworks display runs off the clock, so a still is reproducible.
    if hol.holiday.is_night_fireworks() {
        let secs = hour_f * 3600.0;
        scene.animate_fireworks(secs);
    }
    // B62: the spectacle (festival, parade, shows, balloons, rainbow, side moments).
    let zones = side_zones();
    crate::spec::apply(scene, &st, date, hour_f, &zones, LAT, LON, 0.0);
    // B21 item 4: water takes the sky's colour (and goes dark at night).
    let t = pal.ambient_tint;
    scene.set_water_brightness(
        (0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]).clamp(0.0, 1.0),
    );
    scene.set_water_tint(render::palette::mix(pal.horizon, pal.zenith, 0.25));
    // Stills still show the flowing surface: the overlay is drawn at frame 0
    // unless `--water-anim off` (B21 item 3).
    scene.set_water_anim(water_anim);
    st
}
