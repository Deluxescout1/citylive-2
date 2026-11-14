//! M0 tech spike for CityLive Iso: prove a CPU-rendered iso pixel framebuffer is
//! cheap enough, and print the numbers.
//!
//! Modes: `--png PATH`, `--window`, `--bench`. See `args::USAGE`.

mod args;
mod skylight;
mod spec;
mod bench;
mod orderaudit;
mod pngio;
mod procstat;
mod win;

use std::path::Path;
use std::time::Instant;

use render::{Framebuffer, Scene, SceneConfig, SceneMode};

use crate::args::{Args, Mode, USAGE};

/// B52: put the sim into the civic scenario `CITYLIVE_CIVIC` names, at the
/// frame's `--time` (a handback tool; nothing happens without the variable).
fn stage_civic_env(a: &Args, s: &mut sim::Sim, days: f64) {
    let Ok(what) = std::env::var("CITYLIVE_CIVIC") else { return };
    let hour = a
        .time
        .as_deref()
        .map(|t| {
            let p: Vec<f64> = t.split(':').filter_map(|v| v.parse().ok()).collect();
            p.first().copied().unwrap_or(13.0) + p.get(1).copied().unwrap_or(0.0) / 60.0
        })
        .unwrap_or(13.0);
    s.stage_civic(&what, days.floor() as i64, hour);
    if let Some(f) = s.civics.fire {
        eprintln!("civic fire at {},{}", f.x, f.y);
    }
    eprintln!("civic election {:?} phase {:?} dress {}", s.civics.election.map(|e| e.day), s.election_phase(), s.election_dress());
    if let Some(b) = s.civics.blackout {
        eprintln!("civic blackout at {},{} r {}", b.cx, b.cy, b.r);
    }
}

/// Build a scene driven by the growth sim at `days` sim-days.
fn make_sim_scene(a: &Args, days: f64) -> (Scene, sim::Sim) {
    let pace = sim::Pace::from_name(&a.pace).unwrap_or(sim::Pace::Month);
    let vp = map::world_for_viewport(a.w as i32, a.h as i32);
    let (cx, cy) = a.focus.unwrap_or((a.w as f32 * 0.5, a.h as f32 * 0.5));
    let (fx, fy) = map::screen_to_tile(&vp, cx, cy);
    let mut s = sim::Sim::with_focus_in(a.seed, pace, vp.w, vp.h, (fx, fy), a.biome);
    // B49: tell the sim which frame this still shows *before* it runs, so the
    // ground structures (towers, the solar farm, the turbines) are sited inside
    // the picture the camera actually takes. The scene's camera is the world
    // viewport of the land band, pushed down by the sky band.
    let sky_h = render::scene::sky_pixels(a.h, a.sky);
    let cam_vp = map::world_for_viewport(a.w as i32, a.h.saturating_sub(sky_h) as i32);
    s.set_viewport(cam_vp.origin.0, cam_vp.origin.1 + sky_h as i32, a.w as i32, a.h as i32);
    // B50: a spike run never ends the world unless it is asked to.
    s.ending.world_days = 1e9;
    // B77/D84: the schools need the real date for their terms.
    if let Some(d) = a.date.as_deref() {
        let p: Vec<i64> = d.split('-').filter_map(|x| x.parse().ok()).collect();
        if p.len() == 3 {
            s.set_calendar_date(p[0] as i32, p[1] as u8, p[2] as u8);
        }
    }
    // B99: the desktop always tells the sim where the land line is (B42f); a
    // still that skipped it showed towers and lots the real wallpaper never has.
    s.set_horizon(sky_h as i32, 160);
    // B99: endings use the same frame as every other still (camera + sky band, the
    // wallpaper's own viewport). A separate viewport without the sky offset made
    // the belt swallow the map and shrank the city to a village.
    s.run_to(days * 86400.0);
    // B52: stage a civic scenario for a handback shot (`CITYLIVE_CIVIC=fire`,
    // `rally`, `night`, `vote`, `campaign`, `blackout`, `bust`, `boom`, `smog`,
    // `clean`) at the frame's `--time`.
    stage_civic_env(a, &mut s, days);
    if let Some(kind) = a.ending {
        // Same city clock as the light (so the people are out), *then* the
        // ending: the staging must not be run past afterwards.
        if let Some(t) = a.time.as_deref() {
            let secs: Vec<i64> = t.split(':').filter_map(|p| p.parse().ok()).collect();
            if secs.len() >= 2 {
                let target = days * 86400.0 + (secs[0] * 3600 + secs[1] * 60) as f64;
                if target > s.time {
                    s.run_to(target);
                }
            }
        }
        s.stage_ending(kind, a.ending_at.0, a.ending_at.1);
        if std::env::var("SIM_DEBUG").is_ok() {
            let (sx, sy) = (s.ending.site.0 as i32, s.ending.site.1 as i32);
            for m in &s.map.landforms {
                eprintln!("landform ({:.0},{:.0}) r {:.1} base {:.1} peak {:.1}", m.cx, m.cy, m.radius, m.base, m.peak);
            }
            let (ox, oy, _, _) = s.viewport;
            eprintln!(
                "ending site {:?} aux {:?} screen ({:.0}, {:.0})",
                s.ending.site,
                s.ending.aux,
                ox as f32 + (s.ending.site.0 - s.ending.site.1) * 32.0,
                oy as f32 + (s.ending.site.0 + s.ending.site.1) * 16.0
            );
            for dx in -7..=7 {
                let t = s.map.at(sx + dx, sy);
                eprint!("{dx}:{}{:?} ", t.corner[0], t.kind);
            }
            eprintln!();
        }
    }
    let mut scene = Scene::from_map_with_sky(
        s.map.clone(),
        SceneConfig {
            width: a.w,
            height: a.h,
            seed: a.seed,
            mode: SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: a.rain,
        },
        a.sky,
    );
    if let Ok(n) = std::env::var("SPIKE_SITES") {
        // Review aid: put the N buildings nearest the middle of the frame back
        // under construction, one stage each in turn, tallest kinds included.
        let n: usize = n.parse().unwrap_or(12);
        let mut idx: Vec<usize> = (0..s.lots.len())
            .filter(|&i| s.lots[i].finished() && !matches!(s.lots[i].kind, sim::LotKind::Park | sim::LotKind::Landmark(_) | sim::LotKind::LogCabin))
            .collect();
        idx.sort_by_key(|&i| {
            let l = &s.lots[i];
            let (dx, dy) = (l.x as f32 - fx, l.y as f32 - fy);
            (dx * dx + dy * dy) as i64
        });
        // Tall kinds first (they get the tower cranes), then the nearest rest.
        let mut tall: Vec<usize> = idx.iter().copied().filter(|&i| s.lots[i].storeys >= 5).take(6).collect();
        let rest: Vec<usize> = idx.iter().copied().filter(|i| !tall.contains(i)).collect();
        tall.extend(rest);
        let idx = tall;
        for (k, &i) in idx.iter().take(n).enumerate() {
            s.lots[i].stage = sim::Stage::ALL[[3, 4, 1, 2, 5, 3, 0, 1, 2, 4, 5, 3, 1, 2][k % 14]];
            s.lots[i].stage_end = 1e15;
            let l = &s.lots[i];
            let q = scene.cam.project(l.x as f32, l.y as f32, 0);
            eprintln!("site {k}: {:?} {:?} {}x{} storeys {} front {} stage {:?} screen {:.0},{:.0}", l.kind, (l.x, l.y), l.w, l.h, l.storeys, l.front, l.stage, q.0, q.1);
        }
    }
    attach_city(&mut scene, &s, a.light.as_deref().or(a.time.as_deref()), a.date.as_deref());
    if std::env::var("KIT_DEBUG").is_ok() {
        eprintln!("static rebuild {:.1} ms", scene.stats.full_rebuild_ms);
    }
    (scene, s)
}

/// B74: the scene keeps its own copy of the map, so a tile the sim rewrote
/// after the scene was built (the railway takes its corridor, a street is laid)
/// has to be handed over or the frame draws the *old* ground. The still path
/// used to skip this, which is why a still could show trees standing on the
/// ballast of a line that had already been built through them.
fn sync_scene_map(scene: &mut Scene, s: &mut sim::Sim) {
    for (x, y) in s.take_changes() {
        if scene.map.in_bounds(x, y) {
            scene.set_tile(x, y, s.map.at(x, y));
        }
    }
}

/// Attach the city with its buildkit building sprites (B32), for the light the
/// frame is drawn under.
fn attach_city(scene: &mut Scene, s: &sim::Sim, time: Option<&str>, date: Option<&str>) {
    let time = time;
    let hour = time
        .and_then(|t| t.split(':').next().and_then(|h| h.parse::<f32>().ok()))
        .unwrap_or_else(|| (s.time / 3600.0).rem_euclid(24.0) as f32);
    // B99: day or night from the sun (the sky's window glow), like the wallpaper.
    let glow = match time {
        Some(_) => skylight::glow_at(time, date),
        None => if (6.5..19.5).contains(&hour) { 0.0 } else { 1.0 },
    };
    let light = buildkit::light_for([1.0, 1.0, 1.0], glow, hour);
    if std::env::var("LIGHT_DEBUG").is_ok() {
        eprintln!(
            "light: hour {hour} glow {glow:.3} bucket {} night {:.2} is_night {}",
            light.bucket, light.night, light.is_night
        );
    }
    let objs = endfx::kinds::objects(s);
    let objs_view = objs.clone();
    if std::env::var("SIM_DEBUG").is_ok() {
        for o in &objs {
            if let render::city::ObjKind::Landmark(k) = o.kind {
                let (sx, sy) = scene.cam.project(
                    o.x as f32,
                    o.y as f32,
                    o.level as i32 * render::iso::HEIGHT_UNIT,
                );
                eprintln!(
                    "landmark {:?} at {},{} ({}x{}) stage {:?} screen {:.0},{:.0} kit {}",
                    k, o.x, o.y, o.w, o.h, o.stage, sx, sy, o.kit
                );
            }
        }
    }
    // B39: the settlement phase — the party, the camp and the felled logs.
    let t = s.smooth_time();
    if std::env::var("SIM_DEBUG").is_ok() {
        for f in s.falling_at(t) {
            let p = scene.cam.project(f.0 as f32 + 0.5, f.1 as f32 + 0.5, 0);
            eprintln!("falling tree at {},{} frac {:.2} -> screen ({:.0},{:.0})", f.0, f.1, f.2, p.0, p.1);
        }
        for m in s.settlers_at(t) {
            let p = scene.cam.project(m.0, m.1, 0);
            eprintln!("settler k{} at ({:.1},{:.1}) -> screen ({:.0},{:.0})", m.2, m.0, m.1, p.0, p.1);
        }
    }
    scene.set_settlers(s.settlers_at(t));
    scene.set_falling(s.falling_at(t));
    scene.set_camp(
        s.camp()
            .map(|c| (s.founding.0, s.founding.1, c.tents, c.fire_ok.then_some(c.fire))),
    );
    scene.set_logs(s.log_piles().iter().map(|p| (p.x, p.y, p.kind)).collect());
    // B72 (D78): the windows follow the people inside, at this still's hour.
    scene.set_window_plan(&endfx::kinds::window_plan(s, hour));
    let style = buildkit::BiomeStyle::of(scene.biome());
    let ruin = endfx::ruins(s);
    if std::env::var("B73B_TOWERS").is_ok() {
        for o in &objs {
            if !matches!(o.kind, render::city::ObjKind::Tower) {
                continue;
            }
            let Some(sp) = buildkit::spec_for(o) else { continue };
            let d = buildkit::genkit::design_of(&sp);
            let p = scene.cam.project(o.x as f32, o.y as f32, 0);
            eprintln!(
                "tower at ({}, {}) {}x{} storeys {} screen ({:.0},{:.0}) style {:?} design {:?}",
                o.x, o.y, o.w, o.h, o.storeys, p.0, p.1, sp.style, d.map(|d| (d.body, d.facade, d.material, d.z_top, d.z_crown))
            );
        }
    }
    // B73r2: the nightlife's gate is the sim's own town (D80), not the lot count.
    scene.set_town_stats(s.population(), s.era().index());
    let report = buildkit::set_city_ruined(scene, objs, &light, style, &ruin);    // B73: a still is taken at a fixed moment of the nightlife's own animation
    // clock (a frame strip is the same city at 0, 2, 4 ... s).
    if std::env::var("CITYLIVE_SIGN_DEBUG").is_ok() {
        let p = scene.nightlife();
        eprintln!(
            "nightlife: {} spots, {} designs, town {:?}, anchors {:?} square {:?}, chars {:?}, forms {:?}",
            p.spot_count(),
            p.designs,
            p.town,
            p.anchors,
            p.square,
            p.chars,
            p.forms
        );
        let mut by: std::collections::BTreeMap<(&str, &str), usize> = Default::default();
        for sp in &p.spots {
            *by.entry((sp.character().name(), sp.form().name())).or_default() += 1;
        }
        eprintln!("  by (character, form): {by:?}");
        eprintln!("  beacons {} first {:?}", p.beacons.len(), &p.beacons.iter().take(4).map(|b| (b.wx, b.wy, b.z)).collect::<Vec<_>>());
        for c in render::nightlife::Character::ALL {
            let v: Vec<usize> = p.spots.iter().enumerate().filter(|(_, s)| s.character() == c).map(|(i, _)| i).collect();
            if v.is_empty() {
                continue;
            }
            let n = v.len() as f32;
            let mx = v.iter().map(|&i| p.spots[i].wx).sum::<f32>() / n;
            let my = v.iter().map(|&i| p.spots[i].wy).sum::<f32>() / n;
            let q = scene.cam.project(mx, my, 0);
            // And the centre of the ones the frame actually shows.
            let vis: Vec<usize> = v.iter().copied().filter(|&i| {
                let sp = p.screen(i);
                sp.0 > 100 && sp.0 < 6940 && sp.1 > 80 && sp.1 < 1400
            }).collect();
            let (vx, vy) = if vis.is_empty() {
                (0.0, 0.0)
            } else {
                (
                    vis.iter().map(|&i| p.screen(i).0 as f32).sum::<f32>() / vis.len() as f32,
                    vis.iter().map(|&i| p.screen(i).1 as f32).sum::<f32>() / vis.len() as f32,
                )
            };
            eprintln!("  {}: {} signs ({} in frame), centre ({mx:.1},{my:.1}) -> screen ({:.0},{:.0}); in frame at ({vx:.0},{vy:.0})", c.name(), v.len(), vis.len(), q.0, q.1);
        }
        for (i, sp) in p.spots.iter().enumerate().take(8) {
            eprintln!(
                "  spot {i}: {} {} at ({:.1},{:.1}) z {:.0} {}x{} screen {:?} open {:.1}-{:.1} flags {}",
                sp.character().name(),
                sp.form().name(),
                sp.wx,
                sp.wy,
                sp.z,
                sp.w,
                sp.h,
                p.screen(i),
                sp.open,
                sp.close,
                sp.flags
            );
        }
    }
    if let Ok(t) = std::env::var("CITYLIVE_SIGN_CLOCK") {
        if let Ok(v) = t.parse::<f32>() {
            scene.set_sign_clock(v);
        }
    }
    // B41: the street furniture installed so far, and the crews at work.
    scene.set_street(&street_look(s));
    // B49: the technology the crews have installed, and their own crews. The
    // handback's before/after shots switch the layer off (`CITYLIVE_NO_TECH=1`)
    // so the pair is the same instant with and without the upgrade.
    // `CITYLIVE_TECH_ONLY=<kind>` shows one upgrade alone (the handback's
    // before/after pairs), so the diff of the two frames is that upgrade; the
    // moving parts (rotors, drones, crews) follow the same switch, or a "before"
    // frame would show blades with no turbine holding them up.
    let only = std::env::var("CITYLIVE_TECH_ONLY").ok();
    let show_tech = std::env::var("CITYLIVE_NO_TECH").is_err();
    let wants = |k: &str| only.as_deref().map(|o| o == k).unwrap_or(true);
    if show_tech {
        scene.set_tech(&tech_look(s, only.as_deref()));
        if wants("turbine") {
            scene.set_turbines(s.tech().turbines.clone());
        }
        if wants("drone") {
            let ds = drone_positions(s);
            if std::env::var("SIM_DEBUG").is_ok() {
                for (dx, dy) in &ds {
                    let z = render::iso::level_px(s.map.surface_level_at(*dx, *dy));
                    let p = scene.cam.project(*dx, *dy, z + 58);
                    eprintln!("drone {dx:.1},{dy:.1} -> screen ({:.0},{:.0})", p.0, p.1);
                }
            }
            scene.set_drones(ds);
        }
    }
    scene.set_street_crews(s.streetworks().crews());
    scene.set_tech_crews(s.tech().crews());
    // B52: the civic layer (election dress, fires, blackout, street life).
    if std::env::var("CITYLIVE_NO_CIVIC").is_err() {
        let mut tr = endfx::civic::Tracker::default();
        endfx::civic::sync(scene, s, &objs_view, &mut tr);
        if std::env::var("SIM_DEBUG").is_ok() {
            let l = endfx::civic::look(s, &objs_view);
            for f in &l.dynamic.fires {
                let b = render::civic::fire_bounds(&scene.cam, f);
                eprintln!("civic fire stage {} u {:.2} bounds {:?}", f.stage, f.u, (b.x0, b.y0, b.x1, b.y1));
            }
            for p in &l.dynamic.props {
                eprintln!("civic prop {:?} at ({:.1},{:.1}) screen {:?}", p.kind, p.x, p.y, render::civic::ground(&scene.cam, &scene.map, p.x, p.y));
            }
            eprintln!("civic smog {:.2} at {:?} -> {:?}", l.dynamic.smog, l.dynamic.smog_at, render::civic::ground(&scene.cam, &scene.map, l.dynamic.smog_at.0, l.dynamic.smog_at.1));
            eprintln!("civic props {} figures {} stacks {}", l.dynamic.props.len(), l.dynamic.figures.len(), l.dynamic.stacks.len());
        }
    }
    if std::env::var("SIM_DEBUG").is_ok() {
        eprintln!(
            "streetworks: t={:.0} paint {} lamps {} signs {} lights {} jobs {}",
            s.time,
            s.streetworks().paint.len(),
            s.streetworks().lamps.len(),
            s.streetworks().signs.len(),
            s.streetworks().lights.len(),
            s.streetworks().jobs.len()
        );
        for (label, xs) in [
            ("lamp", s.streetworks().lamps.iter().copied().collect::<Vec<_>>()),
            ("light", s.streetworks().lights.iter().copied().collect::<Vec<_>>()),
        ] {
            for &(x, y) in xs.iter().take(3) {
                let p = scene.cam.project(x as f32 + 0.5, y as f32 + 0.5, 0);
                eprintln!("{label} {x},{y} -> screen ({:.0},{:.0})", p.0, p.1);
            }
        }
        for &(x, y, k) in s.streetworks().signs.iter().take(3) {
            let p = scene.cam.project(x as f32 + 0.5, y as f32 + 0.5, 0);
            eprintln!("sign k{k} {x},{y} -> screen ({:.0},{:.0})", p.0, p.1);
        }
        for (label, tiles) in [
            ("tech tower", s.tech().towers.clone()),
            ("tech turbine", s.tech().turbines.clone()),
            ("tech farm", s.tech().farm.iter().copied().collect::<Vec<_>>()),
            ("tech charger", s.tech().chargers.iter().copied().collect::<Vec<_>>()),
            ("tech cell", s.tech().cells.iter().copied().collect::<Vec<_>>()),
            ("tech pole", s.tech().poles.iter().copied().collect::<Vec<_>>()),
            ("tech buried", s.tech().buried.iter().copied().collect::<Vec<_>>()),
            ("tech solar", s.tech().solar.keys().copied().collect::<Vec<_>>()),
        ] {
            for &(x, y) in tiles.iter().take(12) {
                let p = scene.cam.project(x as f32 + 0.5, y as f32 + 0.5, 0);
                eprintln!("{label} {x},{y} -> screen ({:.0},{:.0})", p.0, p.1);
                // The same point *on the ground it stands on*: this is where the
                // structure is actually drawn (the crop uses it).
                let z = s.map.at(x, y).min_level() as i32 * render::iso::HEIGHT_UNIT;
                let q = scene.cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
                eprintln!("{label}-at {x},{y} -> screen ({:.0},{:.0})", q.0, q.1);
            }
        }
        for &(x, y, k) in &s.tech().crews() {
            let p = scene.cam.project(x as f32 + 0.5, y as f32 + 0.5, 0);
            let z = s.map.at(x, y).min_level() as i32 * render::iso::HEIGHT_UNIT;
            let q = scene.cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
            eprintln!("techcrew k{k} {x},{y} -> screen ({:.0},{:.0})", q.0, q.1);
            let _ = p;
        }
        for &(x, y, k) in &s.streetworks().crews() {
            let p = scene.cam.project(x as f32 + 0.5, y as f32 + 0.5, 0);
            eprintln!("crew k{k} at {x},{y} -> screen ({:.0},{:.0})", p.0, p.1);
        }
    }
    if std::env::var("SIM_DEBUG").is_ok() {
        eprintln!(
            "scene draws: kit {} engine {}",
            scene.stats.kit_draws, scene.stats.engine_draws
        );
    }
    if let Ok(dir) = std::env::var("KIT_BARE_DUMP") {
        // Write out every sprite whose skyline is mostly not a snow surface: the
        // roofs that will stay bare in winter (B76 r3).
        std::fs::create_dir_all(&dir).ok();
        let mut worst: Vec<(usize, usize, usize)> = Vec::new();
        for (i, k) in scene.kit_sprites().iter().enumerate() {
            let (mut bare, mut tops) = (0usize, 0usize);
            for x in 0..k.w {
                for y in 0..k.h {
                    if k.px[y * k.w + x] == 0 {
                        continue;
                    }
                    tops += 1;
                    if matches!(
                        k.surf_at(x, y),
                        0 | 8
                    ) {
                        bare += 1;
                    }
                    break;
                }
            }
            if tops > 8 && bare * 10 > tops * 3 {
                worst.push((bare * 100 / tops, i, tops));
            }
        }
        worst.sort_unstable();
        for (pct, i, _) in worst.iter().rev().take(12) {
            let k = &scene.kit_sprites()[*i];
            let (w, h) = (k.w, k.h);
            let mut rows = Vec::new();
            for y in 0..h {
                for x in 0..w * 2 {
                    let panel = x / w;
                    let sx = x % w;
                    let c = if k.px[y * w + sx] == 0 {
                        [26u8, 29, 36, 255]
                    } else if panel == 0 {
                        let c = k.px[y * w + sx];
                        [((c >> 16) & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, (c & 0xFF) as u8, 255]
                    } else {
                        match k.surf_at(sx, y) {
                            1 => [58, 122, 255, 255],
                            2 => [47, 192, 160, 255],
                            3 => [98, 140, 42, 255],
                            4 => [216, 64, 64, 255],
                            5 => [255, 140, 26, 255],
                            6 => [255, 240, 48, 255],
                            7 => [192, 96, 208, 255],
                            8 => [96, 96, 112, 255],
                            9 => [48, 144, 48, 255],
                            _ => [80, 80, 88, 255],
                        }
                    };
                    rows.extend_from_slice(&c);
                }
            }
            let path = std::path::Path::new(&dir).join(format!("bare-{i}-{pct}pct.png"));
            if let Ok(file) = std::fs::File::create(&path) {
                let bw = std::io::BufWriter::new(file);
                let mut enc = png::Encoder::new(bw, (w * 2) as u32, h as u32);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                if let Ok(mut wr) = enc.write_header() {
                    wr.write_image_data(&rows).ok();
                    wr.finish().ok();
                }
            }
            eprintln!("bare sprite {i}: {pct}% of {w}x{h}");
        }
    }
    if let Ok(v) = std::env::var("KIT_PNG") {
        // `KIT_PNG=<index>:<path>`: one cached sprite as a PNG, at 4x, with the
        // surface tags beside it (B76 r3: tracing a roof that took no snow).
        if let Some((idx, path)) = v.split_once(':') {
            let idx: usize = idx.parse().unwrap_or(0);
            if let Some(k) = scene.kit_sprites().get(idx) {
                let (w, h) = (k.w, k.h);
                let scale = 4usize;
                let mut data = Vec::with_capacity(w * scale * h * scale * 4);
                for y in 0..h * scale {
                    for x in 0..w * scale * 2 {
                        let panel = x / (w * scale);
                        let (sx, sy) = ((x % (w * scale)) / scale, y / scale);
                        let i = sy * w + sx;
                        let c = if k.px[i] == 0 {
                            (26u8, 29u8, 36u8, 255u8)
                        } else if panel == 0 {
                            let c = k.px[i];
                            (
                                ((c >> 16) & 0xFF) as u8,
                                ((c >> 8) & 0xFF) as u8,
                                (c & 0xFF) as u8,
                                255,
                            )
                        } else {
                            match k.surf_at(sx, sy) {
                                1 => (58, 122, 255, 255),
                                2 => (47, 192, 160, 255),
                                3 => (98, 140, 42, 255),
                                4 => (216, 64, 64, 255),
                                5 => (255, 140, 26, 255),
                                6 => (255, 240, 48, 255),
                                7 => (192, 96, 208, 255),
                                8 => (96, 96, 112, 255),
                                9 => (48, 144, 48, 255),
                                _ => (80, 80, 88, 255),
                            }
                        };
                        data.extend_from_slice(&[c.0, c.1, c.2, c.3]);
                    }
                }
                if let Ok(file) = std::fs::File::create(path) {
                    let bw = std::io::BufWriter::new(file);
                    let mut enc = png::Encoder::new(bw, (w * scale * 2) as u32, (h * scale) as u32);
                    enc.set_color(png::ColorType::Rgba);
                    enc.set_depth(png::BitDepth::Eight);
                    if let Ok(mut wr) = enc.write_header() {
                        wr.write_image_data(&data).ok();
                        wr.finish().ok();
                        eprintln!("wrote {path} (kit {idx}, some {w}x{h})");
                    }
                }
            }
        }
    }
    if std::env::var("KIT_DEBUG").is_ok() {
        let sprites = scene.kit_sprites();
        let empty = sprites.iter().filter(|k| k.surf.is_empty()).count();
        eprintln!("kit surf: {} sprites, {empty} with an empty surface plane", sprites.len());
        let surf_b: usize = scene.kit_sprites().iter().map(|k| k.surf.len()).sum();
        let px_b: usize = scene.kit_sprites().iter().map(|k| k.px.len() * 4).sum();
        eprintln!(
            "kit memory: {} sprites, {:.1} MB of colour + {:.1} MB of surface tags",
            scene.kit_sprites().len(),
            px_b as f64 / 1e6,
            surf_b as f64 / 1e6
        );
        let mut hist = [0usize; 16];
        for k in sprites.iter() {
            for &t in k.surf.iter() {
                hist[(t & 0x0F) as usize] += 1;
            }
        }
        eprintln!("kit surf histogram (packed bytes): {hist:?}");
        eprintln!(
            "kit: {} sprites, {} KB, {} hits / {} builds, {} reused",
            report.sprites,
            report.bytes / 1024,
            report.hits,
            report.builds,
            report.reused
        );
        eprintln!("kit: {} sprites with no snow surface", report.unsnowed);
        let mut by_type: std::collections::BTreeMap<String, (usize, usize, usize)> = Default::default();
        for (f, t, bare, tops) in &report.bare_roofs {
            let e = by_type.entry(format!("{f:?}/{t:?}")).or_default();
            e.0 += 1;
            e.1 += bare;
            e.2 += tops;
        }
        for (k, (n, bare, tops)) in by_type {
            let pct = 100.0 * bare as f32 / tops.max(1) as f32;
            eprintln!("bare roof: {k} x{n} ({pct:.0}% of the skyline untagged)");
        }
        let mut engines: std::collections::BTreeMap<String, usize> = Default::default();
        for o in scene.city_objects() {
            if o.kit == 0 {
                *engines.entry(format!("{:?}/{:?}", o.kind, o.stage)).or_default() += 1;
            }
        }
        eprintln!("engine-drawn lots: {engines:?}");
        for o in scene.city_objects() {
            if !matches!(o.stage, render::BuildStage::Finished) {
                eprintln!(
                    "unfinished lot {:?} stage {:?} at {},{} {}x{}",
                    o.kind, o.stage, o.x, o.y, o.w, o.h
                );
            }
        }
    }
}

/// B29: the vehicles the sim has on the road right now, as the renderer wants
/// them (no sim dependency in `render`).
pub fn sim_traffic(s: &sim::Sim) -> Vec<render::traffic::Body> {
    use sim::traffic::VehicleKind;
    let glow = (s.time / 3600.0).rem_euclid(24.0);
    let night = !(6.5..19.0).contains(&glow);
    let mut all: Vec<sim::traffic::Vehicle> = s.vehicles().into_iter().cloned().collect();
    // B77: the week's own vehicles ride the same layer.
    all.extend(s.weekly_vehicles());
    all.iter()
        .map(|v| {
            let ((x, y), dir) = v.pose();
            let kind = match v.kind {
                VehicleKind::Cart => render::traffic::Kind::Cart,
                VehicleKind::Car => render::traffic::Kind::Car,
                VehicleKind::Van => render::traffic::Kind::Van,
                VehicleKind::Taxi => render::traffic::Kind::Taxi,
                VehicleKind::Bus => render::traffic::Kind::Bus,
                VehicleKind::Truck => render::traffic::Kind::Truck,
                VehicleKind::Ev => render::traffic::Kind::Ev,
                VehicleKind::Emergency => render::traffic::Kind::Emergency,
                VehicleKind::Bike => render::traffic::Kind::Bike,
            };
            if std::env::var("SIM_DEBUG").is_ok() && kind == render::traffic::Kind::Emergency {
                let vp = map::world_for_viewport(960, 540);
                let sx = vp.origin.0 as f32 + (x - y) * 16.0;
                let sy = vp.origin.1 as f32 + (x + y) * 8.0;
                eprintln!(
                    "emergency at tile {x:.1},{y:.1} dir {dir:?} -> screen ~({:.0},{:.0})",
                    sx, sy
                );
            }
            render::traffic::Body {
                x,
                y,
                dir: (dir.0, dir.1),
                kind,
                variant: (v.started as u64 % 8) as u8,
                style: render::traffic::Style::from_u8(v.style),
                lights: night || v.kind == VehicleKind::Emergency,
                blink: v.blink,
                lift: 0.0,
                brake: v.braking,
                signal: v.signal,
            }
        })
        .collect()
}

/// B58: the static transport fixtures, from the sim (the shared conversion).
pub fn sim_fixtures(s: &sim::Sim) -> render::rail::Fixtures {
    transitview::fixtures(s)
}

/// B58: every mover, barrier and crew body.
pub fn sim_movers(s: &sim::Sim) -> Vec<render::rail::Body> {
    let hod = (s.time / 3600.0).rem_euclid(24.0);
    let night = !(6.5..19.0).contains(&hod);
    transitview::bodies(s, night)
}

/// B41: the sim's street furniture in the renderer's form.
/// B41: tint each lane by its class, so the hierarchy is visible on a still.
fn road_class_overlay(fb: &mut Framebuffer, s: &sim::Sim, cam: &render::iso::Camera) {
    use sim::roads::RoadClass;
    let colour = |c: RoadClass| match c {
        RoadClass::Path => render::palette::rgb(0xd0, 0xe0, 0x50),
        RoadClass::Track => render::palette::rgb(0xc0, 0x90, 0x50),
        RoadClass::Local => render::palette::rgb(0x50, 0xc8, 0xf0),
        RoadClass::Collector => render::palette::rgb(0xf8, 0x9a, 0x20),
        RoadClass::Avenue => render::palette::rgb(0xf0, 0x40, 0xb0),
        RoadClass::None => render::palette::rgb(0, 0, 0),
    };
    for y in 0..s.map.h {
        for x in 0..s.map.w {
            let c = s.road_class(x, y);
            if c == RoadClass::None {
                continue;
            }
            let col = colour(c);
            let t = s.map.at(x, y);
            let p = cam.corners(x, y, t.corner.map(|c| c as f32));
            // Snapshot the ground under the tile, then tint it: `quad_px`
            // writes through a mutable borrow of the framebuffer.
            let pts = [p[0], p[1], p[2], p[3]];
            let (x0, x1) = (
                pts.iter().map(|q| q.0).fold(f32::MAX, f32::min) as i32,
                pts.iter().map(|q| q.0).fold(f32::MIN, f32::max) as i32,
            );
            let (y0, y1) = (
                pts.iter().map(|q| q.1).fold(f32::MAX, f32::min) as i32,
                pts.iter().map(|q| q.1).fold(f32::MIN, f32::max) as i32,
            );
            for py in y0..=y1 {
                for px in x0..=x1 {
                    if px < 0 || py < 0 || px as usize >= fb.w || py as usize >= fb.h {
                        continue;
                    }
                    let cx = (pts[0].0 + pts[2].0) * 0.5;
                    let cy = (pts[0].1 + pts[2].1) * 0.5;
                    let dx = px as f32 + 0.5 - cx;
                    let dy = py as f32 + 0.5 - cy;
                    if dx.abs() / 31.0 + dy.abs() / 15.5 <= 1.0 {
                        // (the diamond's half-extents, in screen pixels)
                        let under = fb.px()[(py as usize) * fb.w + (px as usize)];
                        fb.put(px, py, render::palette::mix(under, col, 0.62));
                    }
                }
            }
        }
    }
}

/// B49: the technology the crews have installed, in the renderer's form.
/// `only` keeps a single upgrade (a spike switch for the handback's crops).
fn tech_look(sim: &sim::Sim, only: Option<&str>) -> render::tech::TechLook {
    let t = sim.tech();
    let want = |k: &str| only.map(|o| o == k).unwrap_or(true);
    let mut look = render::tech::TechLook {
        lamps: t.lamps.iter().map(|(&(x, y), &l)| (x, y, l)).collect(),
        lamp_base: t.lamp_baseline(),
        poles: t.poles.iter().copied().collect(),
        buried: t.buried.iter().copied().collect(),
        towers: t.towers.clone(),
        cells: t.cells.iter().copied().collect(),
        solar: t.solar.iter().map(|(&(x, y), &v)| (x, y, v)).collect(),
        farm: t.farm.iter().copied().collect(),
        turbines: t.turbines.clone(),
        chargers: t.chargers.iter().copied().collect(),
        drones: t.drones,
    };
    // Trim to the one upgrade asked for (wires ride with the poles).
    if only.is_some() {
        if !want("lamp") {
            // The baseline goes with them: a frame filtered to one upgrade must
            // not quietly show the town's *latest* lamp level, or every pair
            // would differ by its lamp heads as well.
            look.lamps.clear();
            look.lamp_base = 0;
        }
        if !want("pole") {
            look.poles.clear();
        }
        if !want("buried") {
            look.buried.clear();
        }
        if !want("tower") {
            look.towers.clear();
        }
        if !want("cell") {
            look.cells.clear();
        }
        if !want("solar") {
            look.solar.clear();
        }
        if !want("farm") {
            look.farm.clear();
        }
        if !want("turbine") {
            look.turbines.clear();
        }
        if !want("charger") {
            look.chargers.clear();
        }
        look.drones = look.drones && want("drone");
    }
    look
}

/// B49: the delivery drones over the city, five of them, by day.
fn drone_positions(sim: &sim::Sim) -> Vec<(f32, f32)> {
    if !sim.drones_active() {
        return Vec::new();
    }
    let now = sim.time;
    (0..5)
        .filter_map(|i| sim.drone_at(i, 5, now))
        .collect()
}

fn street_look(sim: &sim::Sim) -> render::street::StreetLook {
    let sw = sim.streetworks();
    render::street::StreetLook {
        paint: sw
            .paint
            .iter()
            .map(|&(x, y)| (x, y, sim::streetworks::paint_style(sim, x, y)))
            .collect(),
        lamps: sw.lamps.iter().copied().collect(),
        signs: sw.signs.clone(),
        lights: sw.lights.iter().copied().collect(),
    }
}

fn make_scene(a: &Args) -> Scene {
    let cfg = SceneConfig {
        width: a.w,
        height: a.h,
        seed: a.seed,
        mode: a.scene,
        cars: render::actors::DEFAULT_CARS,
        peds: render::actors::DEFAULT_PEDS,
        buildings: 150,
        rain: a.rain,
    };
    // D66: the land is generated here so `--biome` can force it; `Scene::new`
    // would roll it from the seed on its own.
    let vp = map::world_for_viewport(a.w as i32, a.h as i32);
    let map = match map::generate_with(a.seed, vp.w, vp.h, a.biome) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };
    let mut scene = Scene::from_map(map, cfg);
    scene.depth_mode = a.depth;
    scene
}

/// `--timelapse DIR`: a PNG every `--every` sim-minutes.
fn run_timelapse(a: &Args, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir)?;
    let pace = sim::Pace::from_name(&a.pace).unwrap_or(sim::Pace::Month);
    let vp = map::world_for_viewport(a.w as i32, a.h as i32);
    let mut s = sim::Sim::with_focus_in(
        a.seed,
        pace,
        vp.w,
        vp.h,
        (vp.w as f32 * 0.5, vp.h as f32 * 0.5),
        a.biome,
    );
    s.ending.world_days = 1e9;
    s.set_viewport(vp.origin.0, vp.origin.1, a.w as i32, a.h as i32);
    let (fx, fy) = map::screen_to_tile(&vp, a.w as f32 * 0.5, a.h as f32 * 0.5);
    s.set_focus(fx, fy);
    let end = a.sim_days * 86400.0;
    let step = (a.every_min.max(1.0)) * 60.0;
    let mut frame = 0usize;
    let mut next = 0.0;
    while next <= end + 1.0 {
        while s.time + 1e-9 < next {
            s.run_to(s.time + sim::PLANNING_STEP);
        }
        let mut scene = Scene::from_map_with_sky(
            s.map.clone(),
            SceneConfig {
                width: a.w,
                height: a.h,
                seed: a.seed,
                mode: SceneMode::Sim,
                cars: 0,
                peds: 0,
                buildings: 0,
                rain: false,
            },
            a.sky,
        );
        attach_city(&mut scene, &s, a.light.as_deref().or(a.time.as_deref()), a.date.as_deref());
        skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
        let mut fb = Framebuffer::new(a.w, a.h);
        scene.composite(&mut fb);
        let path = dir.join(format!("frame-{frame:04}.png"));
        pngio::write_png(&path, &fb)?;
        frame += 1;
        next += step;
    }
    println!(
        "timelapse: {frame} frames, 0..{:.1} sim days, every {:.0} min -> {}",
        a.sim_days,
        a.every_min,
        dir.display()
    );
    Ok(())
}

/// `--dayloop DIR`: one city, every `--every` clock minutes for 24 h, rendered
/// through the real sky path — H20's 24 h GIF (B20 answer 6).
fn run_dayloop(a: &Args, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir)?;
    let (mut scene, sim) = make_sim_scene(a, a.sim_days);
    let start = a.start_hour.clamp(0.0, 24.0);
    let step = a.every_min.max(1.0);
    let n = (24.0 * 60.0 / step).round() as usize;
    for i in 0..n {
        let mins = start * 60.0 + i as f64 * step;
        let hh = ((mins / 60.0) as i64).rem_euclid(24);
        let mm = (mins as i64) % 60;
        let time = format!("{hh:02}:{mm:02}");
        skylight::apply(&mut scene, Some(&time), a.date.as_deref(), a.water_anim);
        // B72: the windows follow the hour, frame by frame.
        scene.set_window_plan(&endfx::kinds::window_plan(&sim, mins as f32 % 24.0));
        let mut fb = Framebuffer::new(a.w, a.h);
        scene.composite(&mut fb);
        pngio::write_png(&dir.join(format!("day-{i:04}.png")), &fb)?;
    }
    println!(
        "dayloop: {n} frames from {start:.0} h every {step:.0} clock min -> {}",
        dir.display()
    );
    Ok(())
}

/// `--water-loop DIR`: `--frames` frames of the water animation, one PNG each
/// (B21's 3 s GIF: 24 frames at 8 fps).
fn run_waterloop(a: &Args, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir)?;
    let (mut scene, _sim) = make_sim_scene(a, a.sim_days);
    let st = skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
    // B89: `--weather` for the water frames (sea colour, glitter, chop, rain).
    if let Some(wx) = a.weather.as_deref().and_then(weather_fx::look::preset) {
        let wx = weather_fx::look::for_biome(scene.biome(), &wx);
        let mut book = weather_fx::look::WeatherBook::default();
        let mut left = 600.0f32;
        while left > 0.0 {
            let dt = left.min(5.0);
            book.update(dt, &wx);
            left -= dt;
        }
        let look = book.update(0.0, &wx);
        scene.set_weather(look);
        skylight::apply_weather(&mut scene, &st, look);
    }
    scene.set_water_anim(true);
    let mut fb = Framebuffer::new(a.w, a.h);
    for i in 0..a.frames {
        scene.animate_water(i as f32);
        scene.composite(&mut fb);
        pngio::write_png(&dir.join(format!("water-{i:04}.png")), &fb)?;
    }
    println!(
        "waterloop: {} frames of the water animation -> {}",
        a.frames,
        dir.display()
    );
    Ok(())
}

/// `--walker-loop DIR`: `--frames` frames of the villagers walking (D45).
fn run_walkerloop(a: &Args, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir)?;
    let (mut scene, mut s) = make_sim_scene(a, a.sim_days);
    // Same city clock as the light, so the villagers are out (D45).
    if let Some(t) = a.time.as_deref() {
        let secs: Vec<i64> = t.split(':').filter_map(|p| p.parse().ok()).collect();
        if secs.len() >= 2 {
            let target = a.sim_days * 86400.0 + (secs[0] * 3600 + secs[1] * 60) as f64;
            if target > s.time {
                s.run_to(target);
            }
        }
    }
    attach_city(&mut scene, &s, a.light.as_deref().or(a.time.as_deref()), a.date.as_deref());
    scene.flush_dirty();
    skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
    let mut fb = Framebuffer::new(a.w, a.h);
    for i in 0..a.frames {
        scene.set_walkers(s.walkers(i as f32 * 0.6));
        scene.composite(&mut fb);
        pngio::write_png(&dir.join(format!("walk-{i:04}.png")), &fb)?;
    }
    println!("walkerloop: {} frames -> {}", a.frames, dir.display());
    Ok(())
}

/// `--clouds-bench`: what B57's drifting cloud shadows cost a frame at a few
/// covers — the tick (its damage) plus the composite of exactly those strips
/// (B61c item 2). Real frame size, the test scene, one minute of drift.
fn bench_clouds(a: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut scene = if a.sim { make_sim_scene(a, a.sim_days).0 } else { make_scene(a) };
    scene.set_cloud_wind(255.0, 35.0);
    let mut fb = Framebuffer::new(a.w, a.h);
    let n = a.frames.max(60).min(6000);
    for cover in [0.30f32, 0.80, 0.95] {
        scene.set_clouds(cover, 0.0);
        scene.composite(&mut fb);
        let _ = scene.take_pending();
        let (mut tick, mut comp) = (0.0f64, 0.0f64);
        let mut area = 0i64;
        let mut fulls = 0usize;
        let mut rects = 0usize;
        let mut thick = 0i32;
        let mut frame_ms: Vec<f64> = Vec::new();
        let mut worst_rect = (0.0f64, 0i32, 0i32);
        let mut last_frame = (0usize, 0i64);
        for _ in 0..n {
            let tf = Instant::now();
            let mut frame_rects: Vec<(f64, i32, i32, i32)> = Vec::new();
            let t = Instant::now();
            scene.cloud_tick(1.0 / 30.0);
            tick += t.elapsed().as_secs_f64();
            let t = Instant::now();
            match scene.take_pending() {
                None => {
                    fulls += 1;
                    scene.composite(&mut fb);
                    area += (a.w * a.h) as i64;
                }
                Some(list) => {
                    let nr = list.len();
                    let ar: i64 = list.iter().map(|r| r.area()).sum();
                    last_frame = (nr, ar);
                    for r in &list {
                        rects += 1;
                        thick = thick.max(r.width().min(r.height()));
                        area += r.area();
                        let tr = Instant::now();
                        scene.composite_rect(&mut fb, *r);
                        let d = tr.elapsed().as_secs_f64() * 1000.0;
                        frame_rects.push((d, r.width(), r.height(), r.x0));
                        if d > worst_rect.0 {
                            worst_rect = (d, r.width(), r.height());
                        }
                    }
                }
            }
            comp += t.elapsed().as_secs_f64();
            let fm = tf.elapsed().as_secs_f64() * 1000.0;
            if fm > 12.0 {
                println!("  slow frame {fm:.1} ms: {} rects, {} kpx", last_frame.0, last_frame.1 / 1000);
                frame_rects.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                for q in frame_rects.iter().take(4) {
                    println!("      {:.2} ms {}x{} at x {}", q.0, q.1, q.2, q.3);
                }
            }
            frame_ms.push(fm);
        }
        frame_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("  frame ms: p50 {:.3} p99 {:.3} max {:.3}; worst rect {:.2} ms ({}x{})", frame_ms[frame_ms.len() / 2], frame_ms[frame_ms.len() * 99 / 100], frame_ms[frame_ms.len() - 1], worst_rect.0, worst_rect.1, worst_rect.2);
        for (rw, rh) in [(448, 480), (448, 240), (448, 120), (896, 480)] {
            let r = render::Rect::new(1792, 0, 1792 + rw, rh);
            let t = Instant::now();
            for _ in 0..20 {
                scene.composite_rect(&mut fb, r);
            }
            println!("  composite_rect {rw}x{rh}: {:.3} ms", t.elapsed().as_secs_f64() * 50.0);
        }
        let ms = |d: f64| d * 1000.0 / n as f64;
        let shade = scene.cloud_shadow_area() * 100.0;
        let gate = if shade > 0.001 {
            format!("  (shadows cover {shade:.1} % of the frame)")
        } else {
            "  (no shadows: at this cover the sun does not cast)".to_string()
        };
        println!(
            "clouds {:.0} % cover at {}x{}: tick {:.3} ms + composite {:.3} ms a frame = {:.2} % of a core at 30 fps; {:.2} % of the frame damaged, {:.1} rects a frame, thickest edge {thick} px, {fulls} full frames of {n}{gate}",
            cover * 100.0,
            a.w,
            a.h,
            ms(tick),
            ms(comp),
            (ms(tick) + ms(comp)) * 3.0,
            area as f64 / n as f64 * 100.0 / (a.w * a.h) as f64,
            rects as f64 / n as f64,
        );
    }
    Ok(())
}

/// `--wx-compare DIR`: render one weather kind twice — the old whole-frame
/// overlay draw (`Fx::draw` over the finished composite) and the wallpaper's
/// path (the scene paints the ambient inside its composite, then the particles)
/// — and prove they are the same picture (B61b). Writes
/// `<DIR>/B61b-<kind>-compare.png`: the old crop on the left, the new one on the
/// right, 4x.
fn run_wx_compare(a: &Args, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let kind = a.weather.clone().unwrap_or_else(|| "rain".to_string());
    let wx = weather_fx::look::preset(&kind).ok_or_else(|| format!("unknown weather '{kind}'"))?;
    let (mut scene, sim) = make_sim_scene(a, a.sim_days);
    if a.walkers {
        scene.set_walkers(sim.walkers(1.0));
    }
    skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
    let mut fx = weather_fx::look::overlay(a.seed, a.w, a.h);
    // Let the pools fill and the lightning schedule run: 4 s of weather.
    for _ in 0..120 {
        fx.update(1.0 / 30.0, &wx);
    }

    let mut frame = Framebuffer::new(a.w, a.h);
    // (a) the old path: composite, then draw the whole overlay over it.
    scene.composite(&mut frame);
    let mut old = Framebuffer::new(a.w, a.h);
    old.px_mut().copy_from_slice(frame.px());
    fx.draw(&mut old);

    // (b) the wallpaper's path: hand the scene the ambient, composite (the
    // ambient is applied inside the rectangle), then the particles.
    let full = render::Rect::from_size(a.w, a.h);
    scene.set_ambient(fx.ambient(), full);
    let mut new = Framebuffer::new(a.w, a.h);
    scene.composite(&mut new);
    fx.draw_overlay(&mut new, 0, 0);

    let diff = old
        .px()
        .iter()
        .zip(new.px().iter())
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "wx-compare {kind}: {}x{}, {} pixels differ (ambient active: {}, particles: {})",
        a.w,
        a.h,
        diff,
        fx.ambient().is_active(),
        fx.is_active()
    );

    // The crop pair: a street-level band of the frame, 4x, old | new.
    let cw = (a.w / 4).min(480);
    let ch = (cw / 4).min(a.h / 4);
    let x0 = (a.w as i32 - cw as i32) / 2;
    let y0 = (a.h as i32 - ch as i32) * 3 / 4;
    let scale = 4usize;
    let gap = 12usize;
    let pw = 2 * cw * scale + gap;
    let mut pair = Framebuffer::new(pw, ch * scale);
    for (side, src) in [(0usize, &old), (1usize, &new)] {
        let ox = side * (cw * scale + gap);
        for y in 0..ch * scale {
            for x in 0..cw * scale {
                let p = src.px()[(y0 as usize + y / scale) * a.w + (x0 as usize + x / scale)];
                pair.px_mut()[y * pw + ox + x] = p;
            }
        }
    }
    let out = Path::new(dir).join(format!("B61b-{kind}-compare.png"));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
        }
    pngio::write_png(&out, &pair)?;
    println!("  crop pair (old | new, {scale}x, {}x{}) -> {}", cw, ch, out.display());
    if diff != 0 {
        return Err(format!("{diff} pixels differ: the look changed").into());
    }
    Ok(())
}

/// `--water-bench`: what one water tick costs on the **wallpaper's** path, and
/// what the same tick costs on the legacy file path, side by side (B61 item 1).
///
/// The wallpaper publishes its frame at its internal resolution and the GPU
/// scales it (B63), so a water tick is: advance the phase, re-rasterise the
/// damaged static regions, composite the damaged rectangles, and copy the
/// damaged rows into the shared buffer — no full-frame composite, no CPU
/// upscale. `--export-frames` (B42e) still upscales on the CPU, so that path is
/// measured too, from the same damage.
fn bench_water(a: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let (mut scene, mut sim) = make_sim_scene(a, a.sim_days);
    // B79: the boats and the shore life are part of the per-frame work being
    // measured: the sim steps and the movers are handed over every frame.
    let with_movers = std::env::var("BENCH_MOVERS").is_ok();
    if with_movers {
        scene.set_fixtures(sim_fixtures(&sim));
        for _ in 0..400 {
            sim.transport_step(0.5);
        }
        scene.set_movers(sim_movers(&sim));
    }
    // D45: the villagers are part of the per-frame work being measured.
    if a.walkers {
        scene.set_walkers(sim.walkers(1.0));
    }
    skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), true);
    scene.set_water_anim(true);
    // B87: with `CITYLIVE_PASS_TIME=1`, break the composite down by pass.
    render::scene::pass_time::enable_from_env();
    let mut fb = Framebuffer::new(a.w, a.h);
    let scale = a.scale.max(1);
    let mut up = Vec::with_capacity(a.w * a.h * scale * scale);
    // The shared frame the wallpaper publishes into (B63): internal resolution.
    let mut shm = vec![0u32; a.w * a.h];
    let water_tiles = {
        scene.animate_water(1.0);
        scene.composite(&mut fb);
        let _ = scene.take_pending();
        scene.visible_water()
    };
    let n = a.frames.max(24);
    let mut t_anim = 0.0f64;
    let mut t_flush = 0.0f64;
    let mut t_comp = 0.0f64;
    let mut t_copy = 0.0f64;
    let mut t_up = 0.0f64;
    let mut area = 0i64;
    let mut rects_total = 0usize;
    let mut full_frames = 0usize;
    for i in 0..n {
        if a.walkers {
            scene.set_walkers(sim.walkers(i as f32 * 0.2));
        }
        if with_movers {
            sim.time += 0.2;
            sim.transport_step(0.2);
            scene.set_movers(sim_movers(&sim));
        }
        let t = Instant::now();
        scene.animate_water(i as f32 * 0.2);
        t_anim += t.elapsed().as_secs_f64();

        let t = Instant::now();
        if scene.has_dirty() {
            scene.flush_dirty();
        }
        t_flush += t.elapsed().as_secs_f64();

        let t = Instant::now();
        let pending = scene.take_pending();
        let mut rows: Vec<(usize, usize)> = Vec::new();
        match &pending {
            None => {
                full_frames += 1;
                scene.composite(&mut fb);
                rows.push((0, a.h));
            }
            Some(list) => {
                for r in list {
                    scene.composite_rect(&mut fb, *r);
                    let y0 = r.y0.max(0) as usize;
                    let y1 = r.y1.max(0) as usize;
                    if y1 > y0 {
                        rows.push((y0, y1));
                    }
                }
                rects_total += list.len();
                area += list.iter().map(|r| r.area()).sum::<i64>();
            }
        }
        t_comp += t.elapsed().as_secs_f64();

        // B63 path: the shared buffer only gets the damaged rows.
        let t = Instant::now();
        rows.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(rows.len());
        for (a0, b0) in rows {
            match merged.last_mut() {
                Some(last) if a0 <= last.1 => last.1 = last.1.max(b0),
                _ => merged.push((a0, b0)),
            }
        }
        for &(y0, y1) in &merged {
            let (y0, y1) = (y0.min(a.h), y1.min(a.h));
            shm[y0 * a.w..y1 * a.w].copy_from_slice(&fb.px()[y0 * a.w..y1 * a.w]);
        }
        t_copy += t.elapsed().as_secs_f64();

        // Legacy `--export-frames` path: the damaged rows are upscaled on the
        // CPU into a full-size buffer (B61: a band, not the whole frame).
        let t = Instant::now();
        up.resize(a.w * scale * a.h * scale, 0);
        for &(y0, y1) in &merged {
            render::scale::upscale_rows_into(fb.px(), a.w, a.h, &mut up, scale, y0, y1.min(a.h));
        }
        t_up += t.elapsed().as_secs_f64();
    }
    let ms = |d: f64| d * 1000.0 / n as f64;
    let (ta, tf, tc, tk, tu) = (ms(t_anim), ms(t_flush), ms(t_comp), ms(t_copy), ms(t_up));
    let shm_tick = ta + tf + tc + tk;
    let file_tick = shm_tick + tu;
    let fps = if a.fps == 0 { 30.0 } else { a.fps as f64 };
    println!(
        "water bench {}x{} ({} visible water tiles): {:.1} damaged rects/frame, {:.1} % of the frame, {} full frames of {n}",
        a.w, a.h, water_tiles,
        rects_total as f64 / n as f64,
        area as f64 * 100.0 / (n as f64) / (a.w * a.h) as f64,
        full_frames
    );
    println!(
        "  shm path   {:.3} ms/tick = animate {:.3} + flush {:.3} + composite {:.3} + row copy {:.3}",
        shm_tick, ta, tf, tc, tk
    );
    println!(
        "  file path  {:.3} ms/tick (+ band upscale x{scale} {:.3})",
        file_tick, tu
    );
    for f in [fps, 8.0] {
        println!(
            "  {} fps: shm {:.0} ms/s = {:.2} % of one core, file {:.2} %",
            f,
            shm_tick * f,
            shm_tick * f / 10.0,
            file_tick * f / 10.0
        );
    }
    render::report_pass_times(n);
    Ok(())
}

/// Plan view (B26b item 6): the districts, the planned street grid and the cells
/// that have been built, one pixel per tile at `--scale` per tile.
fn run_districts(a: &Args, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    use sim::District;
    let pace = sim::Pace::from_name(&a.pace).unwrap_or(sim::Pace::Month);
    let vp = map::world_for_viewport(a.w as i32, a.h as i32);
    let (cx, cy) = a.focus.unwrap_or((a.w as f32 * 0.5, a.h as f32 * 0.5));
    let (fx, fy) = map::screen_to_tile(&vp, cx, cy);
    let mut s = sim::Sim::with_focus_in(a.seed, pace, vp.w, vp.h, (fx, fy), a.biome);
    s.ending.world_days = 1e9;
    s.run_to(a.sim_days * 86400.0);
    let scale = a.scale.max(1) as i32;
    let (tw, th) = (s.map.w, s.map.h);
    let mut fb = Framebuffer::new((tw * scale) as usize, (th * scale) as usize);
    let rgb = |r: u8, g: u8, b: u8| render::palette::rgb(r, g, b);
    let district_colour = |d: District| match d {
        District::Core => rgb(0xc0, 0x6a, 0x4a),
        District::European => rgb(0xd8, 0xb4, 0x5a),
        District::Suburb => rgb(0x8c, 0xc0, 0x6a),
        District::Downtown => rgb(0x6a, 0x8c, 0xc8),
        District::HighEnd => rgb(0xb0, 0x84, 0xc0),
        District::Industrial => rgb(0xb0, 0x8c, 0x5a),
    };
    for y in 0..th {
        for x in 0..tw {
            let t = s.map.at(x, y);
            let base = if t.is_water() {
                rgb(0x3c, 0x64, 0x96)
            } else if t.kind.is_lane() {
                rgb(0x50, 0x50, 0x58)
            } else if s.lots.iter().any(|l| l.covers(x, y)) {
                match s.lots.iter().find(|l| l.covers(x, y)).map(|l| l.kind) {
                    Some(sim::LotKind::Park) => rgb(0x3c, 0x78, 0x3c),
                    _ => rgb(0xf0, 0xd8, 0xb0),
                }
            } else if s.blocks_on_grid(x, y) {
                rgb(0x60, 0x60, 0x68)
            } else {
                rgb(0x4a, 0x5c, 0x46)
            };
            // The district tints the land it will develop, not the built city.
            let c = if t.kind.is_lane() || t.is_water() || s.lots.iter().any(|l| l.covers(x, y)) {
                base
            } else {
                let d = district_colour(s.district_at(x, y));
                render::palette::mix(base, d, 0.62)
            };
            for sy in 0..scale {
                for sx in 0..scale {
                    let px = (y * scale + sy) as usize * fb.w + (x * scale + sx) as usize;
                    fb.px_mut()[px] = c;
                }
            }
        }
    }
    pngio::write_png(Path::new(out), &fb)?;
    println!("grid report {:?}", s.grid_report());
    println!(
        "districts: {}x{} tiles at {}x -> {} (founding {},{}; {} cells open, skips {:?})",
        tw,
        th,
        scale,
        out,
        s.founding.0,
        s.founding.1,
        s.grid_report().1,
        s.grid_skips()
    );
    Ok(())
}

fn run_png(a: &Args) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(out) = std::env::var("CITYLIVE_WEAR_SHEET") {
        return run_wear_sheet(a, &out);
    }
    if let Some(out) = &a.districts {
        return run_districts(a, out);
    }
    if let Some(out) = &a.stage_strip {
        return run_stage_strip(a, out);
    }
    if let Some(dir) = &a.timelapse {
        return run_timelapse(a, dir);
    }
    if let Some(dir) = &a.dayloop {
        return run_dayloop(a, dir);
    }
    if let Some(dir) = &a.water_loop {
        return run_waterloop(a, dir);
    }
    if let Some(dir) = &a.traffic_loop {
        return run_traffic_loop(a, dir);
    }
    if let Some(dir) = &a.walker_loop {
        return run_walkerloop(a, dir);
    }
    if a.water_bench {
        return bench_water(a);
    }
    if let Some(dir) = &a.wx_compare {
        return run_wx_compare(a, dir);
    }
    if a.clouds_bench {
        return bench_clouds(a);
    }
    if let Some(dir) = &a.landform_sheet {
        return landform_sheet(a, dir);
    }
    if std::env::var("CITYLIVE_B75_BENCH").is_ok() {
        return run_b75_bench(a);
    }
    if std::env::var("CITYLIVE_B76_TRACE").is_ok() {
        return run_b76_trace(a);
    }
    if let Ok(dir) = std::env::var("CITYLIVE_B75_STRIP") {
        return run_b75_strip(a, &dir);
    }
    if let Ok(dir) = std::env::var("CITYLIVE_VEHICLE_SHEET") {
        return vehicle_sheet(Path::new(&dir));
    }
    if a.tech_print {
        print_tech(a);
    }
    if a.overlap_report {
        return overlap_report(a);
    }
    if a.sim {
        return run_png_sim(a);
    }
    let t0 = Instant::now();
    let mut scene = make_scene(a);
    let build_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let mut frame = Framebuffer::new(a.w, a.h);

    let t = Instant::now();
    scene.composite(&mut frame);
    let first_ms = t.elapsed().as_secs_f64() * 1000.0;

    // Where the landform is on screen: the highest tile's centre, projected
    // with this scene's camera. B46c's landform crops are placed on it.
    if std::env::var("HILLS_DUMP").is_ok() {
        // The highest tile *that is on screen*: a summit just past the bottom
        // edge would put the crop on empty plain.
        let mut best = (0i32, 0i32, 0u8);
        for y in 0..scene.map.h {
            for x in 0..scene.map.w {
                let l = scene.map.at(x, y).max_level();
                if l <= best.2 {
                    continue;
                }
                let p = scene.cam.project_i(x, y, 0);
                if p.0 < 100 || p.1 < scene.sky_height() as i32 + 40 || p.1 > a.h as i32 - 120 {
                    continue;
                }
                best = (x, y, l);
            }
        }
        let p = scene
            .cam
            .project(best.0 as f32 + 0.5, best.1 as f32 + 0.5, 0);
        println!("HILL {} {} {} {}", best.0, best.1, p.0 as i32, p.1 as i32);
    }
    let path = Path::new(&a.path);
    pngio::write_png_scaled(path, &frame, a.scale)?;

    // Second image after `steps` sim steps at 30 Hz.
    let t = Instant::now();
    for _ in 0..a.steps {
        scene.step(1.0 / 30.0);
    }
    let sim_ms = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    scene.composite(&mut frame);
    let second_ms = t.elapsed().as_secs_f64() * 1000.0;

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "frame".to_string());
    let path2 = path.with_file_name(format!("{stem}-step{}.png", a.steps));
    pngio::write_png_scaled(&path2, &frame, a.scale)?;

    println!(
        "png {}x{} seed {} scene {:?} rain {} -> {} + {}",
        a.w,
        a.h,
        a.seed,
        a.scene,
        if a.rain { "on" } else { "off" },
        path.display(),
        path2.display()
    );
    println!(
        "  scene build {:.1} ms (static layer {:.1} ms), composite {:.3} ms, {} steps {:.1} ms ({:.3} ms/step), composite {:.3} ms",
        build_ms,
        scene.stats.full_rebuild_ms,
        first_ms,
        a.steps,
        sim_ms,
        sim_ms / a.steps.max(1) as f64,
        second_ms
    );
    println!(
        "  {} buildings, {} cars, {} people, {} rain drops",
        scene.buildings.len(),
        scene.life.cars.len(),
        scene.life.peds.len(),
        scene
            .rain
            .as_ref()
            .map(|_| render::rain::DEFAULT_DROPS)
            .unwrap_or(0)
    );
    Ok(())
}

/// `--stage-strip PATH`: the seven construction stages of one cottage, side by
/// side, at 4x — a standing review tool for building art.
fn run_stage_strip(a: &Args, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    use render::{BuildStage, CityObj, ObjKind};
    let vp = map::world_for_viewport(a.w as i32, a.h as i32);
    let mut map = map::generate_with(a.seed, vp.w, vp.h, a.biome)?;
    // A clean, flat meadow under the review sheet: no trees, no slopes.
    {
        let (mx, my) = ((vp.w / 2) as i32 - 24, (vp.h / 2) as i32 - 32);
        for y in my..my + 60 {
            for x in mx..mx + 66 {
                let i = (y * map.w + x) as usize;
                map.tiles[i] = map::Tile::flat(3);
            }
        }
    }
    let mut scene = Scene::from_map_with_sky(
        map,
        SceneConfig {
            width: a.w,
            height: a.h,
            seed: a.seed,
            mode: SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        },
        a.sky,
    );
    // Three rows (a cottage, a mid-rise, a tower), seven stages each, on a
    // grid of 5-tile cells near the middle of the frame (B43c review sheet).
    let cx = (vp.w / 2) as i32 - 22;
    let cy = (vp.h / 2) as i32 - 6;
    let mut lots = Vec::new();
    for (row, (kind, storeys, size)) in [
        (ObjKind::House, 2u8, 2i32),
        (ObjKind::Apartment, 6, 3),
        (ObjKind::Office, 16, 3),
    ]
    .iter()
    .enumerate()
    {
        for (i, stage) in BuildStage::ALL.iter().enumerate() {
            // Along the screen's horizontal (+x, -y), rows down the screen.
            let x = cx + i as i32 * 4 + row as i32 * 8;
            let y = cy - i as i32 * 4 + row as i32 * 8;
            let level = scene.map.at(x, y).min_level().max(2);
            lots.push(CityObj {
                x,
                y,
                w: *size,
                h: *size,
                level,
                kind: *kind,
                storeys: *storeys,
                style: 0,
                stage: *stage,
                seed: 11 + i as u32 + row as u32 * 7,
                front: 1,
                t: 0.0,
                kit: 0, wear: Default::default(),
            });
        }
    }
    scene.set_city(lots);
    scene.rebuild_static();
    skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
    let mut fb = Framebuffer::new(a.w, a.h);
    scene.composite(&mut fb);
    pngio::write_png(Path::new(out), &fb)?;
    println!("stage strip: 7 stages -> {out}");
    Ok(())
}

/// B29: `--traffic-loop DIR` writes `--frames` PNGs of the traffic moving, at
/// 10 sim-seconds a frame, so a 5 s GIF is 50 frames of 0.1 s.
/// B75 budget: composite cost per frame, the sim's own tick, CPU over two
/// B76 r2: a frame-time trace across a snow-bucket change. The wallpaper's own
/// loop: 30 fps, people and traffic re-synced, the sliced rebuild and re-tint
/// advanced every frame, and the snow depth stepped four times. Prints the
/// dearest frame, the dearest 10, and when the rebuild slices landed.
/// `CITYLIVE_B76_TRACE=1`.
fn run_b76_trace(a: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let (mut scene, mut s) = make_sim_scene(a, a.sim_days);
    s.set_viewport(scene.cam.ox, scene.cam.oy, a.w as i32, a.h as i32);
    s.warm_traffic(240.0);
    s.plan_rail();
    scene.set_fixtures(sim_fixtures(&s));
    skylight::apply_season(&mut scene, a.date.as_deref(), 51.48);
    skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
    // The wallpaper opts in to slicing 8 s after start; the trace is that steady
    // state, so turn it on at once.
    scene.set_async_shadow(true);
    let mut fb = Framebuffer::new(a.w, a.h);
    let mut figs = Vec::new();
    // Warm the first frame (lazy allocations, the first tree capture).
    scene.composite(&mut fb);
    let _ = scene.take_pending();
    // Warm-up: the startup shadow bake crossfades over ~8 frames (and each
    // fade marks the frame full) before the steady state this trace is about.
    let warm = 60usize;
    let n = 60 + 420usize;
    let mut worst: Vec<f64> = Vec::new();
    let mut when_worst = 0usize;
    let mut worst_total = 0.0f64;
    let mut cover = 0.0f32;
    let mut dbg = std::env::var("B76_TRACE_PRINT").is_ok();
    for i in 0..n {
        // Two snow steps, far enough apart that each sliced pass finishes
        // (a whole pass is ~450 ms of work, ~150 frames at 3 ms a frame).
        if i == warm {
            cover = (cover + 0.25).min(1.0);
            let mut wx = scene.weather();
            wx.snow_cover = cover;
            scene.set_weather(wx);
            if dbg {
                eprintln!("frame {i}: snow {cover:.2}");
            }
        }
        let t0 = Instant::now();
        // `B76_TRACE_NOSIM=1` freezes the sim: the frame time is then the
        // renderer's alone (a decision step in the sim is its own cost and was
        // 50-100 ms in the growth log long before B76).
        if std::env::var("B76_TRACE_NOSIM").is_err() {
            s.advance(1.0 / 30.0);
        }
        s.people_view_into(&mut figs);
        let figs_render: Vec<render::people::Figure> = figs
            .iter()
            .map(|f| render::people::Figure {
                x: f.x,
                y: f.y,
                pose: f.pose.to_u8(),
                tone: f.tone,
                child: f.child,
                prop: f.prop.to_u8(),
                dir: f.dir,
                open: f.open,
                lift: f.lift,
                frame: 0,
            })
            .collect();
        scene.set_traffic(sim_traffic(&s));
        scene.set_people(&figs_render, s.smooth_time() as f32);
        scene.tree_tick(1.0 / 30.0);
        scene.flush_dirty();
        let t_tick = Instant::now();
        let busy = scene.shadow_tick(1.0 / 30.0);
        let _ = busy;
        let tick_ms = t_tick.elapsed().as_secs_f64() * 1000.0;
        let what = match scene.take_pending() {
            None => {
                scene.composite(&mut fb);
                "full".to_string()
            }
            Some(list) => {
                let area: i64 = list.iter().map(|r| (r.x1 - r.x0).max(0) as i64 * (r.y1 - r.y0).max(0) as i64).sum();
                for r in &list {
                    scene.composite_rect(&mut fb, *r);
                }
                format!("{} rects {} Mpx", list.len(), area / 1_000_000)
            }
        };
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if i < warm {
            // The startup shadow crossfade marks frames full; not this trace.
        } else if dbg {
            eprintln!("frame {i}: {ms:.2} ms (tick {tick_ms:.2}) [{what}]");
        }
        if i >= warm {
            if ms > worst_total {
                worst_total = ms;
                when_worst = i;
            }
            worst.push(ms);
        }
    }
    let mut sorted = worst.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = sorted[sorted.len() / 2];
    let p95 = sorted[(sorted.len() * 95) / 100];
    let top: Vec<String> = sorted.iter().rev().take(5).map(|v| format!("{v:.1}")).collect();
    let st = scene.stats;
    println!(
        "B76 trace {}x{}: {} frames, median {:.2} ms, p95 {:.2} ms, max {:.2} ms (frame {when_worst}); worst 5 {} ms; rebuild slices {} dearest {:.2} ms",
        a.w, a.h, n, p50, p95, worst_total, top.join(" "),
        st.rebuild_slices, st.rebuild_worst_ms
    );
    Ok(())
}

/// minutes of the wallpaper's loop, and peak RSS. `CITYLIVE_B75_BENCH=1`.
fn run_b75_bench(a: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let (mut scene, mut s) = make_sim_scene(a, a.sim_days);
    if let Some(t) = a.time.as_deref() {
        let secs: Vec<i64> = t.split(':').filter_map(|p| p.parse().ok()).collect();
        if secs.len() >= 2 {
            let target = a.sim_days * 86400.0 + (secs[0] * 3600 + secs[1] * 60) as f64;
            if target > s.time {
                s.run_to(target);
            }
        }
    }
    s.set_viewport(scene.cam.ox, scene.cam.oy, a.w as i32, a.h as i32);
    s.warm_traffic(240.0);
    s.plan_rail();
    scene.set_fixtures(sim_fixtures(&s));
    skylight::apply_season(&mut scene, a.date.as_deref(), 51.48);
    skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
    // B76: the budget is measured with leaves in the air: `B76_WIND=20` sets the
    // breeze the brief asks for (8 is the fair-weather default).
    let wind_kmh = std::env::var("B76_WIND")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(8.0);
    scene.set_cloud_wind(250.0, wind_kmh);
    let mut fb = Framebuffer::new(a.w, a.h);
    let mut figs = Vec::new();
    // A first composite, so nothing lazy is paid inside the window.
    s.people_view_into(&mut figs);
    scene.set_people(&[], 0.0);
    scene.composite(&mut fb);
    let _ = scene.take_pending();
    let n = 120usize;
    let (mut sync, mut comp, mut best, mut full) = (0.0f64, 0.0f64, f64::MAX, 0usize);
    for i in 0..n {
        let t0 = Instant::now();
        s.advance(1.0 / 30.0);
        s.people_view_into(&mut figs);
        let traf = sim_traffic(&s);
        let figs_render: Vec<render::people::Figure> = figs
            .iter()
            .map(|f| render::people::Figure {
                x: f.x,
                y: f.y,
                pose: f.pose.to_u8(),
                tone: f.tone,
                child: f.child,
                prop: f.prop.to_u8(),
                dir: f.dir,
                open: f.open,
                lift: f.lift,
                frame: 0,
            })
            .collect();
        scene.set_traffic(traf);
        scene.set_people(&figs_render, s.smooth_time() as f32);
        // B76: the sway layer ticks every frame (the leaves are the season's
        // particle cost), exactly as the wallpaper does it.
        scene.tree_tick(1.0 / 30.0);
        sync += t0.elapsed().as_secs_f64();
        let t1 = Instant::now();
        match scene.take_pending() {
            None => {
                full += 1;
                scene.composite(&mut fb);
            }
            Some(list) => {
                for r in &list {
                    scene.composite_rect(&mut fb, *r);
                }
            }
        }
        let dt = t1.elapsed().as_secs_f64();
        comp += dt;
        best = best.min(dt);
        if i == n / 2 {
            scene.mark_full();
        }
    }
    let ms = |d: f64| d * 1000.0 / n as f64;
    println!(
        "B75 bench {}x{}: sync {:.3} ms + composite {:.3} ms a frame (best {:.3} ms); {:.2} % of a core at 30 fps; {full} full frames of {n}; wind {wind_kmh} km/h, {} leaves in the air",
        a.w, a.h, ms(sync), ms(comp), best * 1000.0, (ms(sync) + ms(comp)) * 3.0,
        scene.tree_stats().leaves
    );
    // Two minutes of the wallpaper's loop, CPU ticks and peak RSS.
    let c0 = procstat::process_cpu_ticks().unwrap_or(0);
    let w0 = Instant::now();
    let mut frames = 0u64;
    let mut next = Instant::now();
    while w0.elapsed().as_secs_f64() < 120.0 {
        s.advance(1.0 / 30.0);
        s.people_view_into(&mut figs);
        let figs_render: Vec<render::people::Figure> = figs
            .iter()
            .map(|f| render::people::Figure {
                x: f.x,
                y: f.y,
                pose: f.pose.to_u8(),
                tone: f.tone,
                child: f.child,
                prop: f.prop.to_u8(),
                dir: f.dir,
                open: f.open,
                lift: f.lift,
                frame: 0,
            })
            .collect();
        scene.set_traffic(sim_traffic(&s));
        scene.set_people(&figs_render, s.smooth_time() as f32);
        match scene.take_pending() {
            None => scene.composite(&mut fb),
            Some(list) => {
                for r in &list {
                    scene.composite_rect(&mut fb, *r);
                }
            }
        }
        frames += 1;
        // The wallpaper's own cadence: 30 fps, sleeping between frames.
        next += std::time::Duration::from_millis(1000 / 30);
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
    let secs = w0.elapsed().as_secs_f64();
    let c1 = procstat::process_cpu_ticks().unwrap_or(c0);
    let hz = 100.0;
    let cpu = (c1.saturating_sub(c0)) as f64 / hz / secs * 100.0;
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("VmHWM"))
                .and_then(|l| l.split_whitespace().nth(1).map(|v| v.parse::<u64>().ok()))
                .flatten()
        })
        .unwrap_or(0);
    println!(
        "B75 bench 2 min: {secs:.1} s, {:.1} fps, {cpu:.1} % of one core, peak RSS {:.0} MB, {} figures, {} vehicles",
        frames as f64 / secs,
        rss as f64 / 1024.0,
        scene.people_count(),
        scene.traffic_count()
    );
    Ok(())
}

/// B75: the handback's frame strips — a few seconds of the world, the way the
/// wallpaper draws them: the sim advances, the people and the traffic are
/// re-synced, and the frame is written. `CITYLIVE_B75_STRIP=DIR --frames 8`.
fn run_b75_strip(a: &Args, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir)?;
    let (mut scene, mut s) = make_sim_scene(a, a.sim_days);
    if let Some(t) = a.time.as_deref() {
        let secs: Vec<i64> = t.split(':').filter_map(|p| p.parse().ok()).collect();
        if secs.len() >= 2 {
            let target = a.sim_days * 86400.0 + (secs[0] * 3600 + secs[1] * 60) as f64;
            if target > s.time {
                s.run_to(target);
            }
        }
    }
    s.set_viewport(scene.cam.ox, scene.cam.oy, a.w as i32, a.h as i32);
    s.warm_traffic(240.0);
    s.plan_rail();
    scene.set_fixtures(sim_fixtures(&s));
    scene.flush_dirty();
    skylight::apply_season(&mut scene, a.date.as_deref(), 51.48);
    skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
    // B76: `B76_WIND=kmh` sets the wind for a leaf/petal strip (the brief's
    // "20 % wind" is 8 km/h, the fair-weather default; a gust strip is 30).
    if let Ok(v) = std::env::var("B76_WIND") {
        if let Ok(kmh) = v.parse::<f32>() {
            scene.set_cloud_wind(250.0, kmh);
        }
    }
    let mut fb = Framebuffer::new(a.w, a.h);
    let step = 0.5f64;
    for i in 0..a.frames {
        s.advance(step);
        scene.set_traffic(sim_traffic(&s));
        scene.set_movers(sim_movers(&s));
        let figs: Vec<render::people::Figure> = s
            .people_view()
            .into_iter()
            .map(|f| render::people::Figure {
                x: f.x,
                y: f.y,
                pose: f.pose.to_u8(),
                tone: f.tone,
                child: f.child,
                prop: f.prop.to_u8(),
                dir: f.dir,
                open: f.open,
                lift: f.lift,
                frame: 0,
            })
            .collect();
        scene.set_people(&figs, s.smooth_time() as f32);
        scene.composite(&mut fb);
        pngio::write_png(&dir.join(format!("frame-{i:04}.png")), &fb)?;
    }
    println!("B75 strip: {} frames, {} figures -> {}", a.frames, scene.people_count(), dir.display());
    Ok(())
}

fn run_traffic_loop(a: &Args, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir)?;
    let (mut scene, mut s) = make_sim_scene(a, a.sim_days);
    s.warm_traffic(300.0);
    s.plan_rail();
    for _ in 0..300 {
        s.transport_step(0.2);
    }
    if a.force_emergency {
        // Let it drive out of the dense core before the still, so it is on a
        // street we can see rather than behind a tower.
        s.spawn_emergency();
        for _ in 0..90 {
            s.traffic_step(0.1);
        }
    }
    scene.flush_dirty();
    skylight::apply_season(&mut scene, a.date.as_deref(), 51.48);
    skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
    let mut fb = Framebuffer::new(a.w, a.h);
    if let Some(k) = a.want.as_deref() {
        if let Some(m) = s.transport().iter().find(|m| {
            use sim::rail::MoverKind::*;
            match k {
                "train" => m.kind == Train,
                "boat" => m.kind == Boat,
                "tram" => m.kind == Tram,
                "ship" => m.kind == Ship,
                "plane" => m.kind == Plane,
                _ => false,
            }
        }) {
            let ((x, y), _) = m.pose();
            let z = if m.kind == sim::rail::MoverKind::Plane {
                m.lift
            } else {
                s.map.surface_level_at(x, y) as i32 * 6
            };
            let p = scene.cam.project(x, y, z);
            eprintln!(
                "  {k} at tile {x:.1},{y:.1} -> screen ({:.0},{:.0}) sum {:.0} founding sum {}",
                p.0,
                p.1,
                x + y,
                s.founding.0 + s.founding.1
            );
        }
    }
    scene.set_fixtures(sim_fixtures(&s));
    for i in 0..a.frames {
        s.traffic_step(0.1);
        s.transport_step(0.1);
        scene.set_traffic(sim_traffic(&s));
        scene.set_movers(sim_movers(&s));
        scene.composite(&mut fb);
        pngio::write_png(&dir.join(format!("frame-{i:04}.png")), &fb)?;
    }
    println!(
        "traffic loop: {} frames, {} vehicles -> {}",
        a.frames,
        s.vehicles().len(),
        dir.display()
    );
    Ok(())
}

/// B49: the handback's tech-path printout — the world's own order and timing,
/// and what its crews have installed by `--sim-days`.
fn print_tech(a: &Args) {
    let pace = sim::Pace::from_name(&a.pace).unwrap_or(sim::Pace::Month);
    let vp = map::world_for_viewport(a.w as i32, a.h as i32);
    let (cx, cy) = a.focus.unwrap_or((a.w as f32 * 0.5, a.h as f32 * 0.5));
    let (fx, fy) = map::screen_to_tile(&vp, cx, cy);
    let mut s = sim::Sim::with_focus_in(a.seed, pace, vp.w, vp.h, (fx, fy), a.biome);
    s.set_viewport(vp.origin.0, vp.origin.1, a.w as i32, a.h as i32);
    s.run_to(a.sim_days * 86400.0);
    // When the crews are out (for the mid-upgrade shot): step forward in
    // minute-sized bites and report the first few working moments.
    let mut found = 0;
    for _ in 0..(3 * 24 * 60) {
        s.advance(60.0);
        if !s.tech().busy() {
            continue;
        }
        println!(
            "# crew at day {:.3}: {:?}",
            s.time / 86400.0,
            s.tech().crews()
        );
        found += 1;
        if found >= 4 {
            break;
        }
    }
    let t = s.tech();
    println!("# seed {} — {} (day {:.1})", a.seed, s.city_name, a.sim_days);
    for line in t.printout() {
        println!("{line}");
    }
    println!(
        "# installed: lamps {} (gas {} sodium {} led {}), poles {} buried {} towers {} cells {} solar {} farm {} turbines {} chargers {}",
        t.lamps.len(),
        t.lamps.values().filter(|l| **l == 0).count(),
        t.lamps.values().filter(|l| **l == 1).count(),
        t.lamps.values().filter(|l| **l == 2).count(),
        t.poles.len(),
        t.buried.len(),
        t.towers.len(),
        t.cells.len(),
        t.solar.len(),
        t.farm.len(),
        t.turbines.len(),
        t.chargers.len()
    );
    // The rooftop share, for the handback: fitted houses over standing houses.
    {
        let is_house = |k: sim::LotKind| {
            matches!(
                k,
                sim::LotKind::House | sim::LotKind::Townhouse | sim::LotKind::Apartment
            )
        };
        let houses = s.lots.iter().filter(|l| l.finished() && is_house(l.kind)).count();
        let fitted = t
            .solar
            .keys()
            .filter(|p| {
                s.lots
                    .iter()
                    .any(|l| l.x == p.0 && l.y == p.1 && l.finished() && is_house(l.kind))
            })
            .count();
        println!(
            "# solar: {fitted} of {houses} houses ({:.0} %; target {:.0} %)",
            100.0 * fitted as f32 / houses.max(1) as f32,
            100.0 * t.solar_share_now(s.time)
        );
    }
    // Where the structures are, in tile space (the handback's crops use this).
    for (label, tiles) in [
        ("tower", t.towers.clone()),
        ("turbine", t.turbines.clone()),
        ("farm", t.farm.iter().copied().collect::<Vec<_>>()),
        ("charger", t.chargers.iter().copied().collect::<Vec<_>>()),
        ("cell", t.cells.iter().copied().collect::<Vec<_>>()),
        ("pole", t.poles.iter().copied().collect::<Vec<_>>()),
        ("buried", t.buried.iter().copied().collect::<Vec<_>>()),
    ] {
        if !tiles.is_empty() {
            println!(
                "# {label}: {}",
                tiles
                    .iter()
                    .take(6)
                    .map(|(x, y)| format!("{x},{y}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }
    println!(
        "# flags: catenary {} drones {} maglev {} · traffic era {} · feed lines: {}",
        t.catenary,
        t.drones,
        t.maglev,
        t.vehicle_era(),
        s.story_latest(200).len()
    );
}

/// B50: the sun's place on the sky band, for the swollen sun.
fn sun_px(scene: &Scene, st: &sky::SkyState) -> Option<(f32, f32)> {
    if st.sun_alt > -0.833 {
        Some(sky::draw::screen_pos_at(st.sun_alt, st.sun_az, scene.width(), scene.sky_height(), scene.sky_height() as i32))
    } else {
        None
    }
}

/// B75 step 1: what the frame actually contains — residents simulated, figures
/// the sim put out, figures that survived the frame cull, their poses, the
/// vehicles and the crews. The brief's before/after table.
fn b75_counts(figs: &[render::people::Figure], scene: &Scene, s: &sim::Sim) {
    let mut pose = [0usize; 8];
    for f in figs {
        pose[(f.pose as usize).min(7)] += 1;
    }
    let on = scene.people_count();
    let hour = (s.time / 3600.0).rem_euclid(24.0);
    let mut bbox = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (i, _f) in figs.iter().enumerate() {
        if let Some(&(px, py)) = scene.people_px().get(i) {
            bbox.0 = bbox.0.min(px);
            bbox.1 = bbox.1.min(py);
            bbox.2 = bbox.2.max(px);
            bbox.3 = bbox.3.max(py);
        }
    }
    println!("# B75 screen bbox {bbox:?} cam {:?} frame {}x{}", (scene.cam.ox, scene.cam.oy), scene.width(), scene.height());
    println!(
        "# B75 counts {hour:05.2}h: residents {} (living {}), figures sim {} -> drawn {on} ({} {}% on screen)",
        s.people.len(),
        s.people.living(),
        figs.len(),
        on,
        100.0 * on as f32 / figs.len().max(1) as f32
    );
    println!(
        "# B75 poses: walk {} stand {} work {} carry {} chat {} play {} wave {} sit {}",
        pose[0], pose[1], pose[2], pose[3], pose[4], pose[5], pose[6], pose[7]
    );
    let sites = s
        .lots
        .iter()
        .filter(|l| !l.finished() && l.stage >= sim::Stage::Foundation)
        .count();
    let stages: Vec<String> = s
        .lots
        .iter()
        .filter(|l| !l.finished())
        .map(|l| format!("{:?}", l.stage))
        .collect();
    println!("# B75 sites detail: {sites} {stages:?}");
    for (x, y, side) in s.bus_stops().into_iter().take(6) {
        println!("# B75 bus stop {x}x{y} side {side}");
    }
    for l in s.lots.iter().filter(|l| !l.finished()).take(8) {
        println!("# B75 site at {}x{} stage {:?} front {}", l.x, l.y, l.stage, l.front);
    }
    let workers = figs.iter().filter(|f| f.tone == 8 || f.tone == 9).count();
    // B77: the week's own cast, so a handback can say how much of a Tuesday is
    // the week and how much is the ordinary day.
    let mut bins = 0usize;
    let mut stalls = 0usize;
    let mut boxes = 0usize;
    let mut papers = 0usize;
    let hour = (s.time / 3600.0).rem_euclid(24.0);
    if std::env::var("B77_COUNTS").is_ok() {
        let mut w = Vec::new();
        s.bins_out_into(hour, &mut w);
        bins = w.len();
        let mut m = Vec::new();
        s.market_figures_into(hour, s.time, &mut m);
        stalls = m.iter().filter(|f| f.pose == sim::people::Pose::Object).count();
        let mut sc = Vec::new();
        s.school_run_into(hour, s.time, &mut sc);
        let mut pl = Vec::new();
        s.playground_into(hour, s.time, &mut pl);
        let mut su = Vec::new();
        s.sunday_life_into(hour, s.time, &mut su);
        let mut sh = Vec::new();
        s.small_hours_figures_into(hour, s.time, &mut sh);
        boxes = sh.len();
        papers = sc.len();
        println!(
            "# B77 week {} {}: bins {bins}, market stalls {stalls} ({} figures), school run {papers}, playground {}, sunday {}, small hours {}, week vehicles {}",
            sim::week::day_name(s.week_day()),
            if s.school_in_session() { "term" } else { "holiday" },
            m.len(),
            pl.len(),
            su.len(),
            boxes,
            s.weekly_vehicles().len()
        );
        {
            let mut h: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
            for l in s.lots.iter() {
                *h.entry(format!("{:?}", l.kind)).or_insert(0) += 1;
            }
            println!("# B77 kinds: {h:?}");
            println!("# B77 town: era {:?} pop {} lots {} refused {:?}", s.era(), s.population(), s.lots.len(), s.stats);
        }
        if let Ok(path) = std::env::var("B77_DUMP") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::File::create(&path) {
                let mut mark = |what: &str, x: f32, y: f32| {
                    let z = s.map.ground_px(x, y);
                    let (sx, sy) = scene.cam.project(x, y, z);
                    let _ = writeln!(f, "{what} {sx:.0} {sy:.0}");
                };
                if let Some(l) = s.market_place() {
                    mark("market", l.x as f32 + l.w as f32 * 0.5, l.y as f32 + l.h as f32 * 0.5);
                }
                if let Some(l) = s.school_lot() {
                    mark("school", l.x as f32 + l.w as f32 * 0.5, l.y as f32 + l.h as f32 * 0.5);
                }
                if let Some(i) = s.bakery_index() {
                    if let Some(l) = s.lots.get(i) {
                        mark("bakery", l.x as f32 + l.w as f32 * 0.5, l.y as f32 + l.h as f32 * 0.5);
                    }
                }
                for f in sc.iter().take(400) {
                    mark("kid", f.x, f.y);
                }
                for f in su.iter().take(400) {
                    mark("sun", f.x, f.y);
                }
                for v in s.weekly_vehicles() {
                    let ((x, y), _) = v.pose();
                    mark(&format!("weekveh_{:?}", v.kind).to_lowercase(), x, y);
                }
                let mut bins_dbg = Vec::new();
                s.bins_out_into(hour, &mut bins_dbg);
                for b in bins_dbg.iter() {
                    mark("bin", b.x, b.y);
                }
                for l in s.lots.iter().filter(|l| {
                    l.finished()
                        && (l.kind == sim::LotKind::Church
                            || matches!(l.kind, sim::LotKind::Landmark(sim::landmarks::LandmarkKind::Cathedral)))
                }) {
                    mark("church", l.x as f32 + l.w as f32 * 0.5, l.y as f32 + l.h as f32 * 0.5);
                }
            }
        }
        println!(
            "# B77 places raw: school {}/{} church {}/{} plaza {}/{}",
            s.lots.iter().filter(|l| matches!(l.kind, sim::LotKind::Landmark(sim::landmarks::LandmarkKind::School))).count(),
            s.lots.iter().filter(|l| matches!(l.kind, sim::LotKind::Landmark(sim::landmarks::LandmarkKind::School)) && l.finished()).count(),
            s.lots.iter().filter(|l| l.kind == sim::LotKind::Church).count(),
            s.lots.iter().filter(|l| l.kind == sim::LotKind::Church && l.finished()).count(),
            s.lots.iter().filter(|l| matches!(l.kind, sim::LotKind::Landmark(sim::landmarks::LandmarkKind::Plaza))).count(),
            s.lots.iter().filter(|l| matches!(l.kind, sim::LotKind::Landmark(sim::landmarks::LandmarkKind::Plaza)) && l.finished()).count());
        println!(
            "# B77 places: school {} church {} plaza {} bakery {} market {}",
            s.school_lot().is_some(),
            s.lots.iter().any(|l| l.finished() && l.kind == sim::LotKind::Church),
            s.lots.iter().any(|l| l.finished() && matches!(l.kind, sim::LotKind::Landmark(sim::landmarks::LandmarkKind::Plaza))),
            s.bakery_index().is_some(),
            s.market_place().map(|l| format!("{:?} {}x{}", l.kind, l.w, l.h)).unwrap_or_default()
        );
    }
    println!(
        "# B75 traffic: {} vehicles drawn · sites {} ({} crews) · rigs {}",
        scene.traffic_count(),
        sites,
        workers,
        scene.rig_count()
    );
    if let Ok(path) = std::env::var("B75_DUMP") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::File::create(&path) {
            for (i, _f) in figs.iter().enumerate() {
                if let Some(&(px, py)) = scene.people_px().get(i) {
                    let _ = writeln!(f, "p {px} {py}");
                }
            }
            for l in s.lots.iter().filter(|l| matches!(l.kind, sim::LotKind::Park)) {
                let (cx, cy) = (l.x as f32 + l.w as f32 * 0.5, l.y as f32 + l.h as f32 * 0.5);
                let z = s.map.ground_px(cx, cy);
                let (sx, sy) = scene.cam.project(cx, cy, z);
                let _ = writeln!(f, "k {} {}", sx as i32, sy as i32);
            }
            for v in s.vehicles() {
                let ((x, y), _) = v.pose();
                let z = s.map.ground_px(x, y);
                let (sx, sy) = scene.cam.project(x, y, z);
                let _ = writeln!(f, "v {} {} {}", sx as i32, sy as i32, v.style);
            }
        }
    }
}

/// B50: the residents, with the ending's reactions, handed to the scene.
fn ending_people(scene: &mut Scene, s: &sim::Sim) {
    let mut figs: Vec<render::people::Figure> = s
        .people_view()
        .into_iter()
        .map(|f| render::people::Figure {
            x: f.x,
            y: f.y,
            pose: f.pose.to_u8(),
            tone: f.tone,
            child: f.child,
            prop: f.prop.to_u8(),
            dir: f.dir,
            open: f.open,
            lift: f.lift,
            frame: 0,
        })
        .collect();
    let mut drones: Vec<(f32, f32)> = Vec::new();
    figs.retain(|f| {
        if f.lift == 255 {
            drones.push((f.x, f.y));
            false
        } else {
            true
        }
    });
    scene.set_ending_drones(drones);
    scene.set_people(&figs, s.smooth_time() as f32);
}

/// B50: bring the scene up to the ending as the sim has it now.
fn ending_frame(a: &Args, scene: &mut Scene, s: &mut sim::Sim, st: &sky::SkyState) {
    let objs = endfx::kinds::objects(s);
    let mut tr = endfx::Tracker::default();
    scene.set_ending_clock(Some(a.ending_at.1 * 40.0));
    endfx::sync(scene, s, &mut tr, &objs, sun_px(scene, st));
    ending_people(scene, s);
    scene.flush_dirty();
}

/// `--ending-loop DIR`: frames of the ending as it unfolds, with the cost.
fn run_ending_loop(
    a: &Args,
    scene: &mut Scene,
    s: &mut sim::Sim,
    st: &sky::SkyState,
    dir: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let mut fb = Framebuffer::new(a.w, a.h);
    let mut tr = endfx::Tracker::default();
    let style = buildkit::BiomeStyle::of(scene.biome());
    let hour = a
        .light
        .as_deref()
        .or(a.time.as_deref())
        .and_then(|t| t.split(':').next().and_then(|h| h.parse::<f32>().ok()))
        .unwrap_or(13.0);
    let light = buildkit::light_for([1.0, 1.0, 1.0], skylight::glow_at(a.light.as_deref().or(a.time.as_deref()), a.date.as_deref()), hour);
    let mut clock = a.ending_at.1 * 40.0;
    scene.composite(&mut fb);
    let _ = scene.take_pending();
    let (mut comp_ms, mut sync_ms, mut worst, mut end_ms) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut rebuilds = 0;
    let mut all_ms: Vec<f64> = Vec::new();
    let mut area_sum: i64 = 0;
    for i in 0..a.frames.max(1) {
        let t0 = Instant::now();
        s.advance(a.ending_step);
        clock += a.ending_step as f32;
        scene.set_ending_clock(Some(clock));
        let ts = Instant::now();
        let objs = endfx::kinds::objects(s);
        let (_redraw, rebuild) = endfx::sync(scene, s, &mut tr, &objs, sun_px(scene, st));
        end_ms += ts.elapsed().as_secs_f64() * 1000.0;
        if rebuild {
            let ruin = endfx::ruins(s);
            buildkit::set_city_ruined(scene, objs, &light, style, &ruin);
            endfx::rebuilt(&mut tr, s);
            rebuilds += 1;
        }
        ending_people(scene, s);
        // The ending's own weather follows its progress.
        let mut look = scene.weather();
        endfx::weather(&mut look, s);
        scene.set_weather(look);
        scene.flush_dirty();
        sync_ms += t0.elapsed().as_secs_f64() * 1000.0;
        let t1 = Instant::now();
        // The wallpaper's path: damage-limited unless the frame is all new.
        match scene.take_pending() {
            None => scene.composite(&mut fb),
            Some(rects) => {
                for r in rects {
                    area_sum += r.area();
                    scene.composite_rect(&mut fb, r);
                }
            }
        }
        let ms = t1.elapsed().as_secs_f64() * 1000.0;
        comp_ms += ms;
        worst = worst.max(ms);
        all_ms.push(ms);
        pngio::write_png_scaled(&Path::new(dir).join(format!("frame-{i:04}.png")), &fb, a.scale)?;
    }
    let n = a.frames.max(1) as f64;
    all_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = all_ms[all_ms.len() / 2];
    let p90 = all_ms[all_ms.len() * 9 / 10];
    println!("  composite median {med:.2} ms, p90 {p90:.2} ms, damaged area {:.0} px a frame ({:.1} % of the frame)", area_sum as f64 / a.frames.max(1) as f64, area_sum as f64 / a.frames.max(1) as f64 * 100.0 / (a.w * a.h) as f64);
    println!(
        "ending loop: {} frames of {:?} at {}x{}: composite {:.2} ms avg ({:.2} worst), ending sync {:.2} ms avg, whole step (sim, people, city rebuilds) {:.2} ms avg, {} city rebuilds",
        a.frames.max(1),
        a.ending.map(|e| e.name()).unwrap_or(""),
        a.w,
        a.h,
        comp_ms / n,
        worst,
        end_ms / n,
        sync_ms / n,
        rebuilds
    );
    Ok(())
}


/// `--wear-report` (B78): what the city wears at `--sim-days`, and where the
/// interesting lots are on screen — so the shots can crop straight to a
/// renovation, a wall that became a mural, a shop with its shutters down or a
/// boarded building. Prints the camera too, so a tool can recompute a crop.

/// `CITYLIVE_WEAR_SHEET=<png>` (B78): one building of each kind at each stage of
/// its life — crisp, a few years, old, run-down, boarded, scaffolded, tagged,
/// painted over, shut — so the weathering can be judged on its own, at 1x.
fn run_wear_sheet(a: &Args, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    use render::{BuildStage, CityObj, ObjKind, Wear};
    let vp = map::world_for_viewport(a.w as i32, a.h as i32);
    let mut map = map::generate_with(a.seed, vp.w, vp.h, a.biome)?;
    {
        let (mx, my) = ((vp.w / 2) as i32 - 40, (vp.h / 2) as i32 - 34);
        for y in my..my + 64 {
            for x in mx..mx + 84 {
                let i = (y * map.w + x) as usize;
                map.tiles[i] = map::Tile::flat(3);
            }
        }
    }
    let mut scene = Scene::from_map_with_sky(
        map,
        SceneConfig {
            width: a.w,
            height: a.h,
            seed: a.seed,
            mode: SceneMode::Sim,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        },
        a.sky,
    );
    let cx0 = (vp.w / 2) as i32;
    let cy0 = (vp.h / 2) as i32;
    let wear = |g: u8, art: u8| Wear { grime: g, art, art_seed: 0x51ab_c0de, ..Default::default() };
    // `B78_SHEET_ART=1|2`: only the street-art column, one building, so two
    // renders differ in nothing but the wall (the tag → mural pair).
    if let Ok(k) = std::env::var("B78_SHEET_ART") {
        let art: u8 = k.parse().unwrap_or(1);
        let mut o = CityObj {
            x: cx0, y: cy0, w: 2, h: 2,
            level: 3,
            kind: ObjKind::Shop,
            storeys: 3,
            style: 0,
            stage: BuildStage::Finished,
            seed: 40,
            front: 1,
            t: 0.0,
            kit: 0,
            wear: Wear { grime: 2, art, art_seed: 0x51ab_c0de, ..Default::default() },
        };
        o.level = scene.map.at(o.x, o.y).min_level().max(3);
        let mut objs = vec![o];
        let mut kit = buildkit::KitCache::new();
        kit.attach(&mut objs, &buildkit::light_for([1.0, 1.0, 1.0], 0.0, 12.0));
        scene.set_city_with_sprites(kit.into_sprites(), objs);
        skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
        let mut fb = Framebuffer::new(a.w, a.h);
        scene.composite(&mut fb);
        pngio::write_png(Path::new(out), &fb)?;
        println!("art sheet: art={art} -> {out}");
        return Ok(());
    }
    let states: [(&str, Wear); 9] = [
        ("new", wear(0, 0)),
        ("few years", wear(1, 0)),
        ("old", wear(2, 0)),
        ("run-down", wear(3, 0)),
        ("boarded", Wear { boarded: true, grime: 3, ..wear(3, 0) }),
        ("renovation", Wear { scaffold: true, ..wear(2, 0) }),
        ("tags", Wear { art: 1, ..wear(3, 0) }),
        ("mural", Wear { art: 2, ..wear(3, 0) }),
        ("shop shut", Wear { shut: true, grime: 2, ..wear(2, 0) }),
    ];
    let rows: [(ObjKind, u8, u8); 3] =
        [(ObjKind::House, 2, 20), (ObjKind::Shop, 4, 40), (ObjKind::Office, 6, 60)];
    let cx = (vp.w / 2) as i32 - 12;
    let cy = (vp.h / 2) as i32 + 2;
    let mut lots = Vec::new();
    let mut cells: Vec<(i32, i32)> = Vec::new();
    for (r, (kind, storeys, seed)) in rows.iter().enumerate() {
        for (i, (_, w)) in states.iter().enumerate() {
            let x = cx + i as i32 * 3 + r as i32 * 8;
            let y = cy - i as i32 * 3 + r as i32 * 8;
            let level = scene.map.at(x, y).min_level().max(3);
            let (_, sy0) = scene.cam.project(x as f32, y as f32, level as i32 * render::iso::HEIGHT_UNIT);
            let (sx0, _) = scene.cam.project(x as f32, y as f32, 0);
            cells.push((sx0 as i32, sy0 as i32));
            lots.push(CityObj {
                x,
                y,
                w: 2,
                h: 2,
                level,
                kind: *kind,
                storeys: *storeys,
                style: 0,
                stage: BuildStage::Finished,
                seed: *seed as u32,
                front: 1,
                t: 0.0,
                kit: 0,
                wear: *w,
            });
        }
    }
    let mut objs = lots;
    let mut kit = buildkit::KitCache::new();
    kit.attach(&mut objs, &buildkit::light_for([1.0, 1.0, 1.0], 0.0, 12.0));
    scene.set_city_with_sprites(kit.into_sprites(), objs);
    skylight::apply(&mut scene, a.time.as_deref(), a.date.as_deref(), a.water_anim);
    let mut fb = Framebuffer::new(a.w, a.h);
    scene.composite(&mut fb);
    pngio::write_png(Path::new(out), &fb)?;
    for (i, (name, _)) in states.iter().enumerate() {
        println!(
            "cell {i} {name:12} at ({}, {}) house and ({}, {}) office",
            cells[i].0, cells[i].1, cells[9 + i].0, cells[9 + i].1
        );
    }
    println!("wear sheet: {} states x 3 kinds -> {out}", states.len());
    Ok(())
}

fn print_wear_report(a: &Args, scene: &mut Scene, s: &sim::Sim) {
    let cam = scene.cam;
    let wear = s.wear();
    let mut grime = [0usize; 4];
    let (mut tags, mut murals, mut boarded, mut shut, mut scaffold, mut repainted, mut refascia, mut wet) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for w in wear {
        grime[(w.grime as usize).min(3)] += 1;
        tags += (w.art == 1) as usize;
        murals += (w.art >= 2) as usize;
        boarded += w.boarded as usize;
        shut += w.shut as usize;
        scaffold += w.scaffold as usize;
        repainted += (w.repaint > 0) as usize;
        refascia += (w.fascia > 0) as usize;
        wet += w.wet as usize;
    }
    println!(
        "wear day {:.0}: {} lots ({} finished) | grime 0/1/2/3 = {}/{}/{}/{} | tags {tags} murals {murals} | boarded {boarded} | shut {shut} | scaffold {scaffold} | repainted {repainted} | new fascia {refascia} | wet {wet}",
        a.sim_days,
        s.lots.len(),
        s.lots.iter().filter(|l| l.finished()).count(),
        grime[0],
        grime[1],
        grime[2],
        grime[3]
    );
    println!("cam=({}, {}) fb={}x{}", cam.ox, cam.oy, a.w, a.h);
    // The B78 budget: a full static rebuild of this city at this framebuffer.
    let t = std::time::Instant::now();
    attach_city(scene, s, a.time.as_deref(), a.date.as_deref());
    let t_attach = t.elapsed().as_secs_f64() * 1000.0;
    let t = std::time::Instant::now();
    scene.rebuild_static();
    let rebuild = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "static rebuild {rebuild:.0} ms (attach+sprites {t_attach:.0} ms) | {} kit draws, {} engine draws",
        scene.stats.kit_draws, scene.stats.engine_draws
    );
    // And what one lot's change costs: the hourly path a wear change actually
    // takes (dirty the one footprint, repaint it).
    if let Some(l) = s.lots.iter().find(|l| l.finished() && l.storeys >= 2) {
        let t = std::time::Instant::now();
        scene.dirty_lot(l.x, l.y, l.w, l.h, l.level);
        scene.flush_dirty();
        println!(
            "one changed lot: {:.2} ms flush ({} rects)",
            t.elapsed().as_secs_f64() * 1000.0,
            scene.stats.last_flush_rects
        );
    }
    // Before the district caps: what the rules alone would do, so a report can
    // tell a rule that is too rare from a cap that is holding it back.
    let (mut p_board, mut p_reno, mut p_art) = (0usize, 0usize, 0usize);
    for l in s.lots.iter() {
        let d = s.district_at(l.x, l.y);
        let p = sim::ageing::plan(l, s.time, sim::ageing::wet_land(s.map.biome), d, s.economy_mood());
        p_board += p.wear.boarded as usize;
        p_reno += p.wear.scaffold as usize;
        p_art += (p.wear.art > 0) as usize;
    }
    println!("before caps: boarded {p_board} | scaffold {p_reno} | art {p_art}");
    if std::env::var("B78_AGES").is_ok() {
        let mut ages: Vec<f64> = s.lots.iter().map(|l| (s.time - l.started) / 86_400.0).collect();
        ages.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = ages.len();
        println!(
            "ages (days): min {:.0} p10 {:.0} median {:.0} p90 {:.0} max {:.0} | storeys<=8 {} of {n}",
            ages[0], ages[n / 10], ages[n / 2], ages[n * 9 / 10], ages[n - 1],
            s.lots.iter().filter(|l| l.storeys <= 8).count()
        );
    }
    // The screen box of a lot: the tile's project point, with room for a wall.
    let boxof = |x: i32, y: i32, h: i32| {
        let (sx, sy) = cam.project(x as f32, y as f32, 0);
        (sx as i32, sy as i32 - h)
    };
    let mut printed = 0usize;
    for (i, l) in s.lots.iter().enumerate() {
        let Some(w) = wear.get(i) else { continue };
        let why = if w.scaffold {
            "RENOVATION"
        } else if w.boarded {
            "BOARDED"
        } else if w.art >= 2 {
            "MURAL"
        } else if w.shut {
            "SHOP-SHUT"
        } else if w.art == 1 {
            "TAGS"
        } else {
            continue;
        };
        if printed >= 40 {
            break;
        }
        printed += 1;
        let (sx, sy) = boxof(l.x, l.y, 0);
        let d0 = s.district_at(l.x, l.y);
        // How long this state has left, so a shot can be timed: a scaffold is
        // up for `RENO_DAYS` from the plan's own clock.
        let left = if w.scaffold {
            let mut d = 0.0;
            for k in 1..=12 {
                let t = s.time + k as f64 * 86_400.0;
                let p = sim::ageing::plan(l, t, sim::ageing::wet_land(s.map.biome), d0, s.economy_mood());
                if !p.wear.scaffold {
                    d = k as f64;
                    break;
                }
            }
            format!(" scaffold ends in {d:.0}d")
        } else {
            String::new()
        };
        println!(
            "{why:10} {:?} at ({},{}) {}x{} storeys {} seed {} grime {} repaint {} fascia {} -> screen ({sx},{sy}){left}",
            l.kind, l.x, l.y, l.w, l.h, l.storeys, l.seed, w.grime, w.repaint, w.fascia
        );
    }
}

fn run_png_sim(a: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let (mut scene, mut s) = make_sim_scene(a, a.sim_days);
    if a.wear_report {
        print_wear_report(a, &mut scene, &s);
    }
    // Run the city clock to the same time of day as `--time`, so the villagers
    // are out when the light says they should be (D45). Not during the settlers'
    // phase (B40): the camp is the first twenty minutes of day 0, and jumping
    // the clock nine hours would show a grown village instead of the camp. The
    // light still comes from `--time`, so the frame is a daytime one.
    let cabin_building = s
        .lots
        .iter()
        .any(|l| matches!(l.kind, sim::LotKind::LogCabin) && !l.finished());
    if a.light.is_some() {
        // (the light is set below; the sim clock stays where --sim-days put it)
    } else if s.settlers.is_active() || cabin_building {
        // (keep the sim clock where `--sim-days` put it)
    } else if let Some(t) = a.time.as_deref() {
        let secs: Vec<i64> = t.split(':').filter_map(|p| p.parse().ok()).collect();
        if secs.len() >= 2 {
            let target = a.sim_days * 86400.0 + (secs[0] * 3600 + secs[1] * 60) as f64;
            if target > s.time {
                s.run_to(target);
            }
        }
    }
    // B31b: the scene's camera is the real one — put the waterfronts in frame.
    s.set_viewport(scene.cam.ox, scene.cam.oy, a.w as i32, a.h as i32);
    // B29: a little sim time of traffic so the frame has cars on it.
    s.warm_traffic(240.0);
    // B31: plan the rail (which takes its corridor through the blocks) and run
    // the timetable long enough to have a train, a tram, a boat or a plane in
    // the frame. The city objects are set *after* that, so the cleared plots
    // are gone from the frame.
    s.plan_rail();
    // B74: the corridor the railway just cleared has to reach the scene, or the
    // still keeps the trees the sim has already felled.
    sync_scene_map(&mut scene, &mut s);
    // B31b: `--want train|boat|ship|plane` runs the timetable until *that* mover
    // is on the map, so a still can show one. Without it, stop at the first
    // mover of any kind.
    let want = |k: Option<&str>, m: &sim::rail::MoverKind| -> bool {
        use sim::rail::MoverKind::*;
        match k {
            Some("train") => *m == Train,
            Some("boat") => *m == Boat,
            Some("tram") => *m == Tram,
            Some("ship") => *m == Ship,
            Some("plane") => *m == Plane,
            _ => true,
        }
    };
    let want_kind = a.want.as_deref();
    for _ in 0..6000 {
        s.transport_step(0.2);
        if s.transport().iter().any(|m| want(want_kind, &m.kind)) {
            break;
        }
    }
    // Then run it on until it is properly *on* its route — a train that has just
    // appeared on the line is at the map edge, a ship that has just arrived is at
    // the end of the quay (B31b stills).
    for _ in 0..400 {
        let target = s
            .transport()
            .iter()
            .find(|m| want(want_kind, &m.kind))
            .map(|m| match m.kind {
                // A train is put just before the station: the platform, the
                // track and the train in one crop (B31b).
                sim::rail::MoverKind::Train => {
                    let idx = s
                        .rail
                        .line
                        .iter()
                        .position(|&p| Some(p) == s.rail.station)
                        .unwrap_or(m.route_len() as usize / 2);
                    (idx as f32 - 3.0).max(0.0)
                }
                _ => m.route_len() * 0.4,
            });
        let Some(target) = target else { break };
        let travelled = s
            .transport()
            .iter()
            .find(|m| want(want_kind, &m.kind))
            .map(|m| m.travelled)
            .unwrap_or(0.0);
        if travelled >= target {
            break;
        }
        s.transport_step(0.2);
    }
    if a.plane {
        for _ in 0..6000 {
            s.transport_step(0.2);
            if s.transport()
                .iter()
                .any(|m| m.kind == sim::rail::MoverKind::Plane)
            {
                break;
            }
        }
        for _ in 0..8 {
            s.transport_step(0.2);
        }
    }
    for _ in 0..12 {
        s.transport_step(0.2);
    }
    stage_civic_env(a, &mut s, a.sim_days);
    attach_city(&mut scene, &s, a.light.as_deref().or(a.time.as_deref()), a.date.as_deref());
    if std::env::var("CITYLIVE_ORDER_AUDIT").is_ok() {
        let label = format!(
            "seed {} {} day {}",
            a.seed,
            a.biome.map(|b| b.name()).unwrap_or("auto"),
            a.sim_days
        );
        let r = orderaudit::audit(&scene, &label);
        orderaudit::print(&label, &r);
        return Ok(());
    }
    // B74: a still shows the cars standing in the city's spaces (the kerb rows
    // outside the blocks of flats among them), not only the moving ones.
    scene.set_parked(endfx::kinds::parked_bodies(&s));
    if let Some(k) = a.want.as_deref() {
        if let Some(m) = s.transport().iter().find(|m| {
            use sim::rail::MoverKind::*;
            match k {
                "train" => m.kind == Train,
                "boat" => m.kind == Boat,
                "tram" => m.kind == Tram,
                "ship" => m.kind == Ship,
                "plane" => m.kind == Plane,
                _ => false,
            }
        }) {
            let ((x, y), _) = m.pose();
            let z = if m.kind == sim::rail::MoverKind::Plane {
                m.lift
            } else {
                s.map.surface_level_at(x, y) as i32 * 6
            };
            let p = scene.cam.project(x, y, z);
            eprintln!(
                "  {k} at tile {x:.1},{y:.1} -> screen ({:.0},{:.0}) sum {:.0} founding sum {}",
                p.0,
                p.1,
                x + y,
                s.founding.0 + s.founding.1
            );
        }
    }
    scene.set_fixtures(sim_fixtures(&s));
    scene.set_movers(sim_movers(&s));
    if a.force_emergency {
        // Spawn, then let it drive out of the dense core so the still shows it
        // on a street rather than behind a tower.
        s.spawn_emergency();
        for _ in 0..90 {
            s.traffic_step(0.1);
        }
    }
    scene.set_traffic(sim_traffic(&s));
    // Weather before the first rasterise, so wet roads and snow are baked in.
    let wx_on = a.weather.is_some();
    // B76: `B76_DROUGHT=1` takes the still after a week of clear weather, so the
    // summer dry patches and the bleached lawns are on it.
    if std::env::var("B76_DROUGHT").is_ok() {
        let wx = weather_fx::look::preset("clear").unwrap();
        let mut book = weather_fx::look::WeatherBook::default();
        let mut left = 7.0f32 * 86_400.0;
        while left > 0.0 {
            let dt = left.min(5.0);
            book.update(dt, &wx);
            left -= dt;
        }
        scene.set_weather(book.update(0.0, &wx));
    }
    if let Some(name) = &a.weather {
        if let Some(wx) = weather_fx::look::preset(name) {
            // D68: a still is taken in the world's own weather — snow on a
            // desert world is a sandstorm, and a tropical rain is a downpour.
            let wx = weather_fx::look::for_biome(scene.biome(), &wx);
            let mut book = weather_fx::look::WeatherBook::default();
            // B76: snow settles over hours. A still of a snowy day is taken
            // after six hours of it, so the ground and roofs carry a real layer
            // instead of a switch. Other weathers only need the roads wet.
            let secs: f32 = if name == "snow" {
                // B76: `B76_SNOW_SECS` lets a strip show the build-up hour by
                // hour (the default still is six hours into the snowfall).
                std::env::var("B76_SNOW_SECS")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(6.0 * 3600.0)
            } else {
                600.0
            };
            let mut left = secs;
            while left > 0.0 {
                let dt = left.min(5.0);
                book.update(dt, &wx);
                left -= dt;
            }
            let look = book.update(0.0, &wx);
            if std::env::var("SNOW_TRACE").is_ok() {
                eprintln!("preset {name}: snow_cover {:.2} wet {:.2}", look.snow_cover, look.wet);
            }
            scene.set_weather(look);
            if std::env::var("SNOW_TRACE").is_ok() {
                eprintln!("scene now: snow_cover {:.2}", scene.weather().snow_cover);
            }
        }
    }
    if std::env::var("B76_ITEMS").is_ok() {
        eprintln!(
            "scene: {} B26 buildings, {} landforms, {} city lots, {} kit sprites",
            scene.buildings.len(),
            0usize,
            scene.city_objects().len(),
            scene.kit_sprites().len()
        );
    }
    if std::env::var("B76_FULL_REBUILD").is_ok() {
        // B76 r3: draw the whole static layer once more, so a still cannot show a
        // partially re-rasterised city (a debug lever, not a fix).
        scene.rebuild_static();
    }
    if let Ok(at) = std::env::var("B76_AT") {
        // A screen point -> the tile under it, for a still whose ground colour
        // looks wrong (a snow that did not reach a kind of terrain).
        let parts: Vec<f32> = at.split(',').filter_map(|p| p.parse().ok()).collect();
        if parts.len() == 2 {
            let gx = parts[0] / a.scale as f32;
            let gy = parts[1] / a.scale as f32;
            let (tx, ty) = map::screen_to_tile(&map::world_for_viewport(a.w as i32, a.h as i32), gx, gy);
            let t = scene.map.at(tx.floor() as i32, ty.floor() as i32);
            eprintln!("B76 tile {:?} at {:.1},{:.1}: kind {:?} prop {:?}", (tx, ty), tx, ty, t.kind, t.prop);
            // B76 r3: every lot whose projected box contains the point, in painter
            // order (the last one drawn is the visible one).
            let mut hits: Vec<(i32, &str, u16, u8, u32)> = Vec::new();
            for o in scene.city_objects() {
                let base = o.level as i32 * 14;
                let mut r = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                for (x, y) in [
                    (o.x as f32, o.y as f32),
                    ((o.x + o.w) as f32, o.y as f32),
                    ((o.x + o.w) as f32, (o.y + o.h) as f32),
                    (o.x as f32, (o.y + o.h) as f32),
                ] {
                    for z in [base, base + 48.max(o.wall_height() + 34)] {
                        let (px, py) = scene.cam.project(x, y, z);
                        r.0 = r.0.min(px.floor() as i32);
                        r.1 = r.1.min(py.floor() as i32);
                        r.2 = r.2.max(px.ceil() as i32);
                        r.3 = r.3.max(py.ceil() as i32);
                    }
                }
                let (gxi, gyi) = (gx as i32, gy as i32);
                if gxi >= r.0 && gxi < r.2 && gyi >= r.1 && gyi < r.3 {
                    hits.push((o.diagonal(), "lot", o.kit, o.storeys, o.seed));
                    eprintln!(
                        "B76 box {:?} d{} kit {} storeys {} seed {:08x} at {},{}+{}x{}",
                        o.kind, o.diagonal(), o.kit, o.storeys, o.seed, o.x, o.y, o.w, o.h
                    );
                }
            }
            // B76 r3: which sprite actually owns that screen pixel (the last one
            // drawn there), and what surface tag it carries.
            for o in scene.city_objects() {
                if o.kit == 0 {
                    // An engine-drawn lot can stand here too.
                    let base = o.level as i32 * 14;
                    let (sx, sy) = scene.cam.project(o.x as f32, o.y as f32, base);
                    if (sx - gx).abs() < 120.0 && (sy - gy).abs() < 180.0 {
                        eprintln!("B76 engine lot {:?} stage {:?} at {},{}", o.kind, o.stage, o.x, o.y);
                    }
                    continue;
                }
                let Some(k) = scene.kit_sprites().get(o.kit as usize - 1) else { continue };
                let base = o.level as i32 * 14;
                let (ax, ay) = scene.cam.project(o.x as f32, o.y as f32, base);
                let (bx, by) = (ax as i32 - k.ox, ay as i32 - k.oy);
                let (px, py) = (gx as i32 - bx, gy as i32 - by);
                if px < 0 || py < 0 || px as usize >= k.w || py as usize >= k.h {
                    continue;
                }
                let i = py as usize * k.w + px as usize;
                if k.px[i] == 0 {
                    continue;
                }
                eprintln!(
                    "B76 owns {:?} kit {} storeys {} tag {} at sprite {px},{py} colour {:06x} (surf {} B for {} px)",
                    o.kind, o.kit, o.storeys, k.surf_at(px as usize, py as usize), k.px[i] & 0xFFFFFF,
                    k.surf.len(), k.w * k.h
                );
            }
            // B76 r3: every lot whose anchor is near that screen point — the
            // ground projection, so a tall building's own footprint is a little
            // below its roof.
            for o in scene.city_objects() {
                let (sx, sy) = scene.cam.project(o.x as f32, o.y as f32, o.level as i32 * 14);
                if (sx - gx).abs() < 90.0 && (sy - gy).abs() < 150.0 {
                    eprintln!(
                        "B76 near lot kind {:?} storeys {} style {} kit {} stage {:?} type? at {},{} -> screen {:.0},{:.0}",
                        o.kind, o.storeys, o.style, o.kit, o.stage, o.x, o.y, sx, sy
                    );
                }
            }
            // B76 r3: the lot standing on that tile — its art system, family and
            // how tall it is — so a roof that took no snow can be traced.
            for o in scene.city_objects() {
                let (x, y) = (tx.floor() as i32, ty.floor() as i32);
                if x >= o.x && x < o.x + o.w as i32 && y >= o.y && y < o.y + o.h as i32 {
                    eprintln!(
                        "B76 lot kind {:?} storeys {} style {} kit {} stage {:?} at {},{} {}x{}",
                        o.kind, o.storeys, o.style, o.kit, o.stage, o.x, o.y, o.w, o.h
                    );
                }
            }
        }
    }
    if std::env::var("B76_DEBUG").is_ok() {
        let ts = scene.tree_stats();
        eprintln!("B76 trees {} sprites, {:.1} MB", ts.trees, ts.bytes as f64 / 1e6);
        let w = scene.weather();
        eprintln!(
            "B76: snow_cover {:.2} wet {:.2} fog {:.2} drought {:.2} frost {:.2}",
            w.snow_cover, w.wet, w.fog, w.drought, w.frost
        );
    }
    // B76: snow lying is a road condition: the winter service comes out, and
    // the traffic is stepped again so ploughs are on the street in the still.
    if wx_on {
        s.set_snow(scene.weather().snow_cover);
        if scene.weather().snow_cover > 0.2 {
            let n = s.spawn_winter_service(6);
            if std::env::var("B76_DEBUG").is_ok() {
                eprintln!("B76 winter service sent {n}");
            }
            for _ in 0..150 {
                s.traffic_step(0.1);
            }
            let bodies = sim_traffic(&s);
            if std::env::var("B76_DEBUG").is_ok() {
                for b in &bodies {
                    if matches!(b.style, render::traffic::Style::Plough | render::traffic::Style::Gritter) {
                        let z = s.map.ground_px(b.x, b.y);
                        let (px, py) = scene.cam.project(b.x, b.y, z);
                        eprintln!("B76 service {:?} at {:.1},{:.1} -> {:.0},{:.0}", b.style, b.x, b.y, px, py);
                    }
                }
            }
            scene.set_traffic(bodies);
            // B76 r2: the winter service clears the lanes it drives.
            scene.plough_tiles(&s.service_tiles());
        }
    }
    // B50: the ending's say in the weather, before the ground is baked.
    if s.ending_view().is_some() {
        let mut look = scene.weather();
        endfx::weather(&mut look, &s);
        scene.set_weather(look);
    }
    // Greenwich, London (D16) — the same latitude the sky uses.
    skylight::apply_season(&mut scene, a.date.as_deref(), 51.48);
    // B76: a cold clear dawn rimes the grass (after the season, so the still is
    // a winter one).
    skylight::apply_frost(&mut scene, a.time.as_deref(), a.date.as_deref());
    scene.flush_dirty();
    let st = skylight::apply(
        &mut scene,
        a.light.as_deref().or(a.time.as_deref()),
        a.date.as_deref(),
        a.water_anim,
    );
    if s.ending_view().is_some() {
        let look = scene.weather();
        skylight::apply_weather(&mut scene, &st, look);
    }
    if let Some(name) = &a.weather {
        if let Some(wx) = weather_fx::look::preset(name) {
            let wx = weather_fx::look::for_biome(scene.biome(), &wx);
            let mut book = weather_fx::look::WeatherBook::default();
            let secs: f32 = if name == "snow" {
                // B76: `B76_SNOW_SECS` lets a strip show the build-up hour by
                // hour (the default still is six hours into the snowfall).
                std::env::var("B76_SNOW_SECS")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(6.0 * 3600.0)
            } else {
                600.0
            };
            let mut left = secs;
            while left > 0.0 {
                let dt = left.min(5.0);
                book.update(dt, &wx);
                left -= dt;
            }
            let look = book.update(0.0, &wx);
            skylight::apply_weather(&mut scene, &st, look);
        }
    }
    // D45: the villagers on their paths, frozen on one walk frame.
    let n_walkers = if a.walkers { s.walkers(1.0).len() } else { 0 };
    scene.set_walkers(s.walkers(if a.walkers { 1.0 } else { 0.0 }));
    let mut fb = Framebuffer::new(a.w, a.h);
    if let Ok(path) = std::env::var("SKY_DUMP") {
        pngio::write_png(std::path::Path::new(&path), scene.sky())?;
    }
    if std::env::var("SIM_DEBUG").is_ok() {
        eprintln!("debug: {} traffic bodies on the scene", scene.traffic_count());
    }
    // B62: the residents (and, through the scene, any festival crowd).
    {
        let figs: Vec<render::people::Figure> = s
            .people_view()
            .into_iter()
            .map(|f| render::people::Figure {
                x: f.x,
                y: f.y,
                pose: f.pose.to_u8(),
                tone: f.tone,
                child: f.child,
                prop: f.prop.to_u8(),
                dir: f.dir,
                open: f.open,
                lift: f.lift,
                frame: 0,
            })
            .collect();
        scene.set_people(&figs, s.smooth_time() as f32);
        if std::env::var("B75_COUNTS").is_ok() {
            b75_counts(&figs, &scene, &s);
        }
    }
    if a.ending.is_some() {
        if let Some(dir) = a.ending_loop.clone() {
            return run_ending_loop(a, &mut scene, &mut s, &st, &dir);
        }
        ending_frame(a, &mut scene, &mut s, &st);
    }
    scene.composite(&mut fb);
    // B52: `CITYLIVE_CIVIC_BENCH=N` times N animated frames of the civic layer
    // the way the wallpaper draws them (tick, take the damage, composite just
    // those rectangles) and reports the cost at the frame size.
    if let Some(n) = std::env::var("CITYLIVE_CIVIC_BENCH").ok().and_then(|v| v.parse::<usize>().ok()) {
        let _ = scene.take_pending();
        let (mut tick, mut comp, mut area, mut fulls) = (0.0f64, 0.0f64, 0i64, 0usize);
        for i in 0..n {
            scene.set_civic_clock(i as f32 / 30.0);
            let t = Instant::now();
            scene.civic_tick();
            tick += t.elapsed().as_secs_f64();
            let t = Instant::now();
            let mut n_rects = 0usize;
            let mut a_area = 0i64;
            match scene.take_pending() {
                None => {
                    fulls += 1;
                    scene.composite(&mut fb);
                    area += (a.w * a.h) as i64;
                }
                Some(list) => {
                    n_rects = list.len();
                    for r in &list {
                        area += r.area();
                        a_area += r.area();
                        scene.composite_rect(&mut fb, *r);
                    }
                }
            }
            comp += t.elapsed().as_secs_f64();
            if std::env::var("CITYLIVE_SIGN_FRAME_DEBUG").is_ok() {
                eprintln!("frame: {} rects, {} px, {:.3} ms", n_rects, a_area, t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        let ms = |d: f64| d * 1000.0 / n as f64;
        println!(
            "civic bench {}x{}: tick {:.3} ms + composite {:.3} ms a frame = {:.2} % of a core at 30 fps; {:.3} % of the frame damaged, {fulls} full frames of {n}",
            a.w, a.h, ms(tick), ms(comp), (ms(tick) + ms(comp)) * 3.0, area as f64 / n as f64 * 100.0 / (a.w * a.h) as f64
        );
    }
    // B73: `CITYLIVE_SIGN_BENCH=N` times N animated night frames the way the
    // wallpaper draws them (sign tick, take the damage, composite just those
    // rectangles) and reports the cost at the frame size.
    if let Some(n) = std::env::var("CITYLIVE_SIGN_BENCH").ok().and_then(|v| v.parse::<usize>().ok()) {
        let _ = scene.take_pending();
        // The first frame is a full composite: take it out of the average.
        scene.set_sign_clock(0.0);
        scene.mark_full();
        scene.composite(&mut fb);
        let _ = scene.take_pending();
        let (mut tick, mut comp, mut area, mut fulls) = (0.0f64, 0.0f64, 0i64, 0usize);
        for _ in 0..n {
            let t = Instant::now();
            scene.sign_tick(1.0 / 30.0);
            tick += t.elapsed().as_secs_f64();
            let t = Instant::now();
            let mut n_rects = 0usize;
            let mut a_area = 0i64;
            match scene.take_pending() {
                None => {
                    fulls += 1;
                    scene.composite(&mut fb);
                    area += (a.w * a.h) as i64;
                }
                Some(list) => {
                    n_rects = list.len();
                    for r in &list {
                        area += r.area();
                        a_area += r.area();
                        scene.composite_rect(&mut fb, *r);
                    }
                }
            }
            comp += t.elapsed().as_secs_f64();
            if std::env::var("CITYLIVE_SIGN_FRAME_DEBUG").is_ok() {
                eprintln!("frame: {} rects, {} px, {:.3} ms", n_rects, a_area, t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        let ms = |d: f64| d * 1000.0 / n as f64;
        println!(
            "sign bench {}x{}: tick {:.3} ms + composite {:.3} ms a frame = {:.2} % of a core at 30 fps; {:.3} % of the frame damaged, {} signs, {fulls} full frames of {n}",
            a.w, a.h, ms(tick), ms(comp), (ms(tick) + ms(comp)) * 3.0, area as f64 / n as f64 * 100.0 / (a.w * a.h) as f64,
            scene.nightlife().spot_count()
        );
        // A whole frame with and without the layer: the brief's per-frame cost.
        // The box is shared, so take the fastest of 20 (the honest number).
        let reps = 20;
        let mut best = f64::MAX;
        for _ in 0..reps {
            let t = Instant::now();
            scene.composite(&mut fb);
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "sign bench {}x{}: full composite {:.3} ms a frame (fastest of {reps})",
            a.w, a.h, best
        );
        println!(
            "sign bench {}x{}: static layer rebuild {:.1} ms (the plan adds its 0.4 ms to the city install, nothing to the bake)",
            a.w, a.h, scene.stats.full_rebuild_ms
        );
        let (fast, mean) = scene.sign_layer_ms(&mut fb, 20);
        println!("sign bench {}x{}: nightlife layer alone {:.3} ms (fastest), {:.3} ms mean, over a whole frame", a.w, a.h, fast, mean);
    }
    if wx_on {
        if let Some(name) = &a.weather {
            if let Some(wx) = weather_fx::look::preset(name) {
                let wx = weather_fx::look::for_biome(scene.biome(), &wx);
                let mut fx = weather_fx::look::overlay(a.seed, a.w, a.h);
                // A sandstorm needs a moment to fill the frame: warm it up.
                for _ in 0..90 {
                    fx.update(0.016, &wx);
                }
                fx.draw(&mut fb);
            }
        }
    }
    if a.wildlife {
        // B41b: the wildlife layer, warmed so the flocks are mid-crossing.
        let vp = map::Viewport {
            origin: (scene.cam.ox, scene.cam.oy),
            w: a.w as i32,
            h: a.h as i32,
        };
        let objs = endfx::kinds::objects(&s);
        let mut wl = wildlife::Wildlife::new(a.seed ^ 0x77, &s.map);
        let hour = a
            .light
            .as_deref()
            .or(a.time.as_deref())
            .and_then(|t| t.split(':').next().and_then(|h| h.parse::<f32>().ok()))
            .unwrap_or_else(|| (s.time / 3600.0).rem_euclid(24.0) as f32);
        for _ in 0..90 {
            wl.update(1.0 / 30.0, &s.map, &objs, hour);
        }
        wl.draw(&mut fb, &vp, Some(scene.depth()));
        if std::env::var("WL_DEBUG").is_ok() {
            eprintln!("wildlife: active {} counts {:?}", wl.active_count(), wl.counts());
            for (sp, x, y) in wl.positions(&vp).iter() {
                eprintln!("  {sp:?} at {x},{y}");
            }
        }
    }
    if a.road_class {
        road_class_overlay(&mut fb, &s, &scene.cam);
    }
    let path = Path::new(&a.path);
    pngio::write_png_scaled(path, &fb, a.scale)?;
    // B43c review aids: SPIKE_SITE_FRAMES=N runs N frames the way the wallpaper
    // does (damage-limited composites) and reports the cost; SPIKE_SITE_LOOP=dir
    // also writes each frame there. SPIKE_SITE_DT sets the seconds per frame.
    if let Ok(n) = std::env::var("SPIKE_SITE_FRAMES") {
        let n: usize = n.parse().unwrap_or(120);
        let dt: f64 = std::env::var("SPIKE_SITE_DT").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0 / 30.0);
        let loop_dir = std::env::var("SPIKE_SITE_LOOP").ok();
        if let Some(d) = &loop_dir {
            std::fs::create_dir_all(d)?;
        }
        let _ = scene.take_pending();
        let mut t = s.time;
        scene.set_people(&[], t as f32);
        let _ = scene.take_pending();
        let c0 = procstat::process_cpu_ticks().unwrap_or(0);
        let w0 = std::time::Instant::now();
        let mut px_total = 0i64;
        let mut full = 0;
        let mut inside = std::time::Duration::ZERO;
        // B62: the spectacle keeps running through the loop (a show, a parade).
        let spec_hour = a
            .time
            .as_deref()
            .and_then(|t| {
                let mut p = t.split(':');
                let h: f32 = p.next()?.parse().ok()?;
                let m: f32 = p.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                Some(h + m / 60.0)
            })
            .unwrap_or(12.0);
        let spec_zones = skylight::side_zones();
        for i in 0..n {
            t += dt;
            let t1 = std::time::Instant::now();
            spec::apply(&mut scene, &st, a.date.as_deref(), spec_hour, &spec_zones, skylight::LAT, skylight::LON, (i as f64 * dt) as f32);
            // `t as f32` at day 90 is 7.8e6 s, where an f32 step is 0.5 s: the
            // walk cycle would stand still. SPIKE_ANIM_T=1 feeds the people a
            // small clock instead (the numbers for crowds in motion).
            let anim_t = if std::env::var("SPIKE_ANIM_T").is_ok() { (i as f64 * dt) as f32 } else { t as f32 };
            scene.set_people(&[], anim_t);
            match scene.take_pending() {
                None => {
                    scene.composite(&mut fb);
                    full += 1;
                    px_total += (fb.w * fb.h) as i64;
                }
                Some(rects) => {
                    for r in rects {
                        scene.composite_rect(&mut fb, r);
                        px_total += r.area();
                    }
                }
            }
            inside += t1.elapsed();
            if let Some(d) = &loop_dir {
                pngio::write_png(&Path::new(d).join(format!("f{i:04}.png")), &fb)?;
            }
        }
        let wall = w0.elapsed().as_secs_f64();
        let cpu = (procstat::process_cpu_ticks().unwrap_or(0) - c0) as f64 / procstat::USER_HZ;
        let per = inside.as_secs_f64() * 1000.0 / n as f64;
        eprintln!(
            "site frames: {n} at dt {dt:.4}s, {} rigs, mean {per:.3} ms/frame, {:.0} kpx/frame, {full} full frames; at 30 fps = {:.2} % of one core",
            scene.rig_count(),
            px_total as f64 / n as f64 / 1000.0,
            per * 30.0 / 10.0
        );
        let _ = (wall, cpu);
    }
    {
        let v1 = map::world_for_viewport(960, 540);
        let v2 = map::world_for_viewport(960, 378);
        eprintln!(
            "  map {}x{} founding {:?}; vp(960,540)={:?} vp(960,378)={:?} cam={:?}",
            s.map.w, s.map.h, s.founding, v1, v2, (scene.cam.ox, scene.cam.oy)
        );
    }
    for (name, z) in [("port", s.rail.port), ("marina", s.rail.marina)] {
        if let Some(zz) = z {
            let (cx, cy) = zz.centre();
            let lvl = s.map.at(cx, cy).min_level() as i32 * 8;
            let p = scene.cam.project(cx as f32 + 0.5, cy as f32 + 0.5, lvl);
            eprintln!("  {name} {zz:?} -> screen {p:?}");
        }
    }
    if let Some((_, stp)) = s.rail.station.zip(s.rail.station_platform) {
        eprintln!(
            "  platform tile kind {:?} lot {} objs {:?}",
            s.map.at(stp.0, stp.1).kind,
            s.lots.iter().any(|l| l.covers(stp.0, stp.1)),
            endfx::kinds::objects(&s)
                .iter()
                .filter(|o| stp.0 >= o.x && stp.0 < o.x + o.w && stp.1 >= o.y && stp.1 < o.y + o.h)
                .map(|o| (o.x, o.y, o.w, o.h, o.storeys))
                .collect::<Vec<_>>()
        );
    }
    if let Some((sto, stp)) = s.rail.station.zip(s.rail.station_platform) {
        let vp = map::world_for_viewport(a.w as i32, a.h as i32);
        eprintln!(
            "  station tile {sto:?} platform {stp:?} -> screen ({},{}) / ({},{})",
            vp.origin.0 + (sto.0 - sto.1) * 16,
            vp.origin.1 + (sto.0 + sto.1) * 8,
            vp.origin.0 + (stp.0 - stp.1) * 16,
            vp.origin.1 + (stp.0 + stp.1) * 8
        );
    }
    if let Some(&(tx, ty)) = s.rail_tiles().first() {
        let vp = map::world_for_viewport(a.w as i32, a.h as i32);
        eprintln!(
            "  rail first tile {tx},{ty} -> screen ({},{})",
            vp.origin.0 + (tx - ty) * 16,
            vp.origin.1 + (tx + ty) * 8
        );
    }
    println!(
        "  rail: {} tiles, station {:?}, movers {}, port {:?}, marina {:?}, tram {} tiles / {} stops (clock {:.0}, next plane {:.0})",
        s.rail_tiles().len(),
        s.rail.station,
        s.transport().len(),
        s.rail.port,
        s.rail.marina,
        s.rail.tram_line.len(),
        s.rail.tram_stops.len(),
        s.rail.clock,
        s.rail.next_plane
    );
    println!(
        "sim png seed {} horizon {} day {:.2}: {} villagers, {} vehicles ({} no-route {:?}), {} lots ({} finished), pop {}, {} lane tiles, {} fields, {} bridges, {} decisions (mean {}us max {}us) | {}",
        a.seed,
        scene.horizon().name(),
        a.sim_days,
        n_walkers,
        s.vehicles().len(),
        s.traffic.no_route,
        s.traffic.why,
        s.lots.len(),
        s.lots.iter().filter(|l| l.finished()).count(),
        s.population(),
        s.lane_tiles().len(),
        s.fields.len(),
        s.stats.bridges,
        s.stats.decisions,
        s.stats.last_decision_us,
        s.stats.max_decision_us,
        path.display()
    );
    Ok(())
}

/// B35: `KIT_STAGES=DIR` renders one tall building at each of the kit's
/// construction stages (excavation → foundation → framing with a tower crane →
/// cladding → finished) as separate PNGs plus a contact strip, for the
/// `H35-crane.gif` deliverable.
fn run_kit_stages(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use buildkit::{BuildingFunction, BuildingSpec, BuildingStyle, ConstructionStage, Era, Light};
    std::fs::create_dir_all(dir)?;
    let stages = [
        ConstructionStage::Excavation,
        ConstructionStage::Foundation,
        ConstructionStage::Framing,
        ConstructionStage::Cladding,
        ConstructionStage::Finished,
    ];
    let mut panels: Vec<Framebuffer> = Vec::new();
    for (i, st) in stages.iter().enumerate() {
        let spec = BuildingSpec {
            building_type: None,
            function: BuildingFunction::Tower,
            landmark_kind: None,
            style: BuildingStyle::ModernGlass,
            footprint: (3, 3),
            storeys: 14,
            era: Era::City,
            stage: *st,
            age: 0.0,
            seed: 0x9E37_79B9,
            variant: 1,
            biome: buildkit::BiomeStyle::Temperate,
        };
        let mut sp = buildkit::Sprite::new(1, 1);
        buildkit::draw(&spec, &Light::day(), &mut sp);
        // Composite onto grass and sky so the PNG is readable on its own.
        let (pw, ph) = (sp.w.max(1) + 120, sp.h.max(1) + 200);
        let mut fb = Framebuffer::new(pw, ph);
        for y in 0..fb.h {
            let c = if y < ph / 3 {
                render::palette::pal::SKY_HORIZON
            } else {
                render::palette::pal::GRASS[4]
            };
            fb.fill_rect(0, y as i32, fb.w as i32, 1, c);
        }
        // Put the sprite's ground point on the ground line, centred.
        let (ox, oy) = (pw as i32 / 2 - sp.ox, ph as i32 - 100 - sp.oy);
        for y in 0..sp.h {
            for x in 0..sp.w {
                let c = sp.px[y * sp.w + x];
                if c != 0 && (c >> 24) >= 0x80 {
                    fb.put(ox + x as i32, oy + y as i32, c);
                }
            }
        }
        let path = dir.join(format!("stage-{i}.png"));
        pngio::write_png(&path, &fb)?;
        panels.push(fb);
    }
    // A contact strip, side by side.
    let pw: usize = panels.iter().map(|p| p.w).sum();
    let ph = panels.iter().map(|p| p.h).max().unwrap_or(0);
    let mut strip = Framebuffer::new(pw, ph);
    let mut x0 = 0usize;
    for p in &panels {
        for y in 0..p.h {
            for x in 0..p.w {
                let c = p.px()[y * p.w + x];
                strip.put((x0 + x) as i32, y as i32, c);
            }
        }
        x0 += p.w;
    }
    pngio::write_png(&dir.join("strip.png"), &strip)?;
    println!("kit stages: {} panels -> {}", panels.len(), dir.display());
    Ok(())
}

/// B35: `LANDMARK_SHEET=PATH` renders every landmark the sim can place, one
/// panel each, into a contact sheet (the `H35-landmarks.png` deliverable).
fn run_landmark_sheet(out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use buildkit::{BuildingFunction, BuildingSpec, BuildingStyle, Era, LandmarkKind, Light};
    let kinds = [
        ("cathedral", LandmarkKind::Cathedral, 3, 4, 4),
        ("station", LandmarkKind::TrainStation, 5, 3, 2),
        ("museum", LandmarkKind::Museum, 4, 3, 3),
        ("stadium", LandmarkKind::Stadium, 6, 5, 4),
        ("signature tower", LandmarkKind::SignatureTower, 3, 3, 22),
        ("lighthouse", LandmarkKind::Lighthouse, 2, 2, 3),
        ("container port", LandmarkKind::ContainerPort, 5, 4, 1),
        ("ferris wheel", LandmarkKind::FerrisWheel, 3, 3, 4),
    ];
    let civic = [
        ("school", crate::args::Mode::Png), // placeholder, unused
    ];
    let _ = civic;
    let mut panels: Vec<Framebuffer> = Vec::new();
    let mut draw_one = |spec: &BuildingSpec| -> Framebuffer {
        let mut sp = buildkit::Sprite::new(1, 1);
        buildkit::draw(spec, &Light::day(), &mut sp);
        let (pw, ph) = (sp.w.max(1) + 120, sp.h.max(1) + 200);
        let mut fb = Framebuffer::new(pw, ph);
        for y in 0..fb.h {
            let c = if y < ph / 3 {
                render::palette::pal::SKY_HORIZON
            } else {
                render::palette::pal::GRASS[4]
            };
            fb.fill_rect(0, y as i32, fb.w as i32, 1, c);
        }
        let (ox, oy) = (pw as i32 / 2 - sp.ox, ph as i32 - 100 - sp.oy);
        for y in 0..sp.h {
            for x in 0..sp.w {
                let c = sp.px[y * sp.w + x];
                if c != 0 && (c >> 24) >= 0x80 {
                    fb.put(ox + x as i32, oy + y as i32, c);
                }
            }
        }
        fb
    };
    for (_name, kind, w, h, st) in kinds {
        let spec = BuildingSpec {
            building_type: None,
            function: BuildingFunction::Landmark,
            landmark_kind: Some(kind),
            style: BuildingStyle::HighEnd,
            footprint: (w, h),
            storeys: st,
            era: Era::City,
            stage: buildkit::ConstructionStage::Finished,
            age: 0.0,
            seed: 0x9E37_79B9,
            variant: 2,
            biome: buildkit::BiomeStyle::Temperate,
        };
        panels.push(draw_one(&spec));
    }
    // The civic four, which the kit draws as catalogue buildings.
    for (bt, w, h, st) in [
        (buildkit::BuildingType::CivicSchool, 4u8, 3u8, 3u8),
        (buildkit::BuildingType::CivicLibrary, 3, 3, 3),
        (buildkit::BuildingType::CivicFireStation, 3, 3, 2),
        (buildkit::BuildingType::CivicHospital, 5, 4, 6),
    ] {
        let spec = BuildingSpec {
            building_type: Some(bt),
            function: BuildingFunction::Civic,
            landmark_kind: None,
            style: BuildingStyle::BrickMainStreet,
            footprint: (w, h),
            storeys: st,
            era: Era::City,
            stage: buildkit::ConstructionStage::Finished,
            age: 0.0,
            seed: 0x9E37_79B9,
            variant: 1,
            biome: buildkit::BiomeStyle::Temperate,
        };
        panels.push(draw_one(&spec));
    }
    // Lay them out 4 across.
    let cols = 4usize;
    let cw = panels.iter().map(|p| p.w).max().unwrap_or(1);
    let ch = panels.iter().map(|p| p.h).max().unwrap_or(1);
    let rows = panels.len().div_ceil(cols);
    let mut sheet = Framebuffer::new(cw * cols, ch * rows);
    sheet.clear(render::palette::rgb(0x22, 0x2a, 0x33));
    for (i, p) in panels.iter().enumerate() {
        let (cx, cy) = ((i % cols) * cw, (i / cols) * ch);
        for y in 0..p.h {
            for x in 0..p.w {
                let c = p.px()[y * p.w + x];
                sheet.put((cx + x) as i32, (cy + y) as i32, c);
            }
        }
    }
    pngio::write_png(out, &sheet)?;
    println!("landmark sheet: {} panels -> {}", panels.len(), out.display());
    Ok(())
}

fn main() {
    if let Ok(out) = std::env::var("LANDMARK_SHEET") {
        if let Err(e) = run_landmark_sheet(Path::new(&out)) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    if let Ok(dir) = std::env::var("KIT_STAGES") {
        if let Err(e) = run_kit_stages(Path::new(&dir)) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    let a = match args::parse() {
        Ok(a) => a,
        Err(e) => {
            if e.0 != "help" {
                eprintln!("error: {e}\n");
            }
            eprint!("{USAGE}");
            std::process::exit(if e.0 == "help" { 0 } else { 2 });
        }
    };

    let result = match a.mode {
        Mode::Png => run_png(&a),
        Mode::Bench => bench::run(&a),
        Mode::Window => win::run(&a),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// B46d: the landform sheet. Four lands (rows) x three forms (columns), on the
/// land's own ground colour, with one building of that land beside the first as
/// a scale reference. Writes `sheet-1x.png` and a 4x strip per land.
/// B75: the vehicle sheet — every body style in four of its colours, facing
/// +x, on a neutral ground, at 4×. `CITYLIVE_VEHICLE_SHEET=DIR`.
fn vehicle_sheet(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use render::traffic::{style_sprite, Style};
    std::fs::create_dir_all(dir)?;
    let cols = 4usize;
    let cw = 150usize;
    let ch = 84usize;
    let rows = Style::ALL.len().div_ceil(cols);
    let mut sheet = render::Framebuffer::new(cw * cols, ch * rows);
    sheet.clear(render::palette::rgb(0x9a, 0xa4, 0x96));
    for (i, st) in Style::ALL.iter().enumerate() {
        let (cx, cy) = ((i % cols) * cw, (i / cols) * ch);
        for v in 0..4u8 {
            let s = style_sprite(*st, 0, v);
            let x = (cx + 18 + v as usize * 33) as i32 - s.ax;
            let y = (cy + ch - 14) as i32 - s.ay;
            sheet.blit_sprite(&s, x, y);
        }
        println!("{:2} {}", i, st.name());
    }
    pngio::write_png_scaled(&dir.join("vehicles-4x.png"), &sheet, 3)?;
    pngio::write_png(&dir.join("vehicles-1x.png"), &sheet)?;
    Ok(())
}

fn landform_sheet(a: &args::Args, dir: &str) -> Result<(), Box<dyn std::error::Error>> {    use render::landform::{self, Form, Spec, Sun};
    std::fs::create_dir_all(dir)?;
    let lands = [map::Biome::Temperate, map::Biome::Desert, map::Biome::Tundra, map::Biome::Tropical];
    let sun = Sun::default_day();
    let cw = 760usize;
    let ch = 520usize;
    let mut sheet = Framebuffer::new(cw * 3, ch * 4);
    let t0 = Instant::now();
    for (row, &land) in lands.iter().enumerate() {
        let g = render::biome::look(land);
        let ground = g.sand[3];
        let ground = match land {
            map::Biome::Desert => g.sand[3],
            map::Biome::Tundra => render::palette::rgb(0xe4, 0xec, 0xf4),
            _ => g.grass[3],
        };
        let mut strip = Framebuffer::new(cw * 3, ch);
        strip.clear(ground);
        for (col, form) in Form::for_biome(land).iter().enumerate() {
            let (radius, height) = match form {
                Form::Mesa => (6.5, 84.0),
                Form::Butte => (5.0, 110.0),
                Form::Mountain => (6.0, 175.0),
                Form::Cone => (6.0, 100.0),
            };
            let spec = Spec { form: *form, biome: land, seed: a.seed ^ (row as u64 * 31 + col as u64 * 7 + 1), radius, height };
            let tb = Instant::now();
            let model = landform::model(&spec);
            let t_model = tb.elapsed();
            let tr = Instant::now();
            let b = landform::render_model(&model, &sun);
            println!(
                "  {:?} {:?}: model {:.1} ms, render (the re-bake when the sun moves) {:.1} ms, sprite {}x{}",
                land, form, t_model.as_secs_f64() * 1e3, tr.elapsed().as_secs_f64() * 1e3, b.sprite.w, b.sprite.h
            );
            let s = &b.sprite;
            let x = (col * cw) as i32 + cw as i32 / 2 - s.ax;
            let y = ch as i32 - 130 - s.ay;
            strip.blit_sprite(s, x, y);
        }
        let shifted = &strip;
        for yy in 0..ch {
            for xx in 0..cw * 3 {
                let v = shifted.px()[yy * cw * 3 + xx];
                let idx = (row * ch + yy) * cw * 3 + xx;
                sheet.px_mut()[idx] = v;
            }
        }
        pngio::write_png_scaled(
            &std::path::Path::new(dir).join(format!("strip-{}.png", g.name)),
            &strip,
            4,
        )?;
    }
    pngio::write_png(&std::path::Path::new(dir).join("sheet-1x.png"), &sheet)?;
    println!("landform sheet in {:?}", t0.elapsed());
    Ok(())
}
/// B74: the overlap audit for the city a still would show. Runs the same
/// pipeline a frame does (world, viewport, rail, city objects), then asks both
/// audits: the sim's tile audit and the renderer's pixel audit. Prints a table
/// and exits without writing a frame, so it costs a fraction of a render.
fn overlap_report(a: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let (mut scene, mut s) = make_sim_scene(a, a.sim_days);
    s.set_viewport(scene.cam.ox, scene.cam.oy, a.w as i32, a.h as i32);
    s.warm_traffic(240.0);
    s.plan_rail();
    sync_scene_map(&mut scene, &mut s);
    attach_city(&mut scene, &s, a.time.as_deref(), a.date.as_deref());

    let t = Instant::now();
    let report = sim::audit::audit(&s);
    let sim_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let px = render::audit::audit(&scene, &render::terrain::SeasonLook::default());
    let px_ms = t.elapsed().as_secs_f64() * 1000.0;

    println!(
        "B74 overlap report: {} seed {} day {} frame {}x{} | {} lots, {} lane tiles, {} rail tiles",
        s.biome.name(),
        a.seed,
        a.sim_days,
        a.w,
        a.h,
        s.lots.len(),
        s.lane_tiles().len(),
        s.rail_tiles().len()
    );
    for w in sim::audit::Overlap::ALL {
        println!("  {:>14}: {}", w.name(), report.count(w));
    }
    println!(
        "  {:>14}: {} ({} sideways)",
        "sprite",
        px.count(),
        px.sideways
    );
    println!(
        "  total {} ({:.2} ms tiles + {:.2} ms pixels; {} props measured)",
        report.total() + px.count(),
        sim_ms,
        px_ms,
        px.props_measured
    );
    let (derived, skipped) = scene.planting_stats();
    println!(
        "  drawn frame: {derived} derived trees, {skipped} stored props skipped by cleaning"
    );
    for f in report.samples.iter().take(8) {
        println!("  sample {:?} at {},{} lots {:?}", f.what, f.x, f.y, f.lots);
    }
    // Crop hints: where the handback's real-size crops go, in screen pixels of
    // this frame (the camera is the scene's own).
    let cam = scene.cam;
    let mut tallest: Option<(i32, i32, u8)> = None;
    for l in &s.lots {
        if matches!(l.kind, sim::LotKind::Tower | sim::LotKind::Office | sim::LotKind::Apartment) {
            if tallest.map(|t| l.storeys > t.2).unwrap_or(true) {
                tallest = Some((l.x + l.w / 2, l.y + l.h / 2, l.storeys));
            }
        }
    }
    let mut hints: Vec<(&str, (i32, i32))> = Vec::new();
    if let Some((x, y, _)) = tallest {
        hints.push(("towers", (x, y)));
    }
    if let Some(&(x, y)) = s.rail_tiles().get(s.rail_tiles().len() / 2) {
        hints.push(("rail", (x, y)));
    }
    if let Some(l) = s.lots.iter().find(|l| l.kind == sim::LotKind::Park) {
        hints.push(("park", (l.x + l.w / 2, l.y + l.h / 2)));
    }
    if let Some(l) = s
        .lots
        .iter()
        .find(|l| matches!(l.kind, sim::LotKind::Landmark(sim::landmarks::LandmarkKind::Plaza)))
    {
        hints.push(("plaza", (l.x + l.w / 2, l.y + l.h / 2)));
    }
    if let Some(l) = s.lots.iter().find(|l| l.kind == sim::LotKind::House) {
        hints.push(("suburb", (l.x + l.w / 2, l.y + l.h / 2)));
    }
    for (name, (x, y)) in &hints {
        let z = s.map.at(*x, *y).min_level() as i32 * map::iso_consts::HEIGHT_UNIT;
        let (sx, sy) = cam.project_i(*x, *y, z);
        println!("  crop {name}: tile {x},{y} -> screen {sx},{sy}");
    }
    // B74 note: the block of flats with the most kerb-side parking, so a
    // handback crop lands on an apartment street.
    let mut best: Option<(usize, i32, i32)> = None;
    for l in &s.lots {
        if !sim::parking::needs_parking(l) {
            continue;
        }
        // Only blocks the camera can see: a handback crop cannot use a block
        // behind the frame.
        {
            let cx = l.x + l.w / 2;
            let cy = l.y + l.h / 2;
            let z = s.map.at(cx, cy).min_level() as i32 * map::iso_consts::HEIGHT_UNIT;
            let (sx, sy) = scene.cam.project_i(cx, cy, z);
            if sx < 240 || sx > a.w as i32 - 240 || sy < 200 || sy > a.h as i32 - 160 {
                continue;
            }
        }
        let kerb = s
            .parking_bays(l)
            .iter()
            .filter(|b| {
                let (x, y) = b.tile();
                !(x >= l.x && x < l.x + l.w && y >= l.y && y < l.y + l.h)
            })
            .count();
        if best.map(|(n, _, _)| kerb > n).unwrap_or(true) {
            best = Some((kerb, l.x + l.w / 2, l.y + l.h / 2));
        }
    }
    if let Some((n, x, y)) = best {
        println!("  crop apartments: {n} kerb bays near tile {x},{y}");
        // Aim at the kerb row itself, not the block's centre: the row stands
        // outside the plot, on the street.
        let mut sx = 0.0f32;
        let mut sy = 0.0f32;
        let mut m = 0.0f32;
        for l in s.lots.iter().filter(|l| sim::parking::needs_parking(l)) {
            if l.x + l.w / 2 != x || l.y + l.h / 2 != y {
                continue;
            }
            for b in s.parking_bays(l) {
                let (bx, by) = b.tile();
                let inside = bx >= l.x && bx < l.x + l.w && by >= l.y && by < l.y + l.h;
                if inside {
                    continue;
                }
                let z = s.map.at(bx, by).min_level() as i32 * map::iso_consts::HEIGHT_UNIT;
                let (px_, py_) = scene.cam.project(b.x, b.y, z);
                sx += px_;
                sy += py_;
                m += 1.0;
            }
        }
        if m > 0.0 {
            println!("  crop apartments row: screen {:.0},{:.0}", sx / m, sy / m);
        }
    }
    for f in px.intrusions.iter().take(8) {
        println!(
            "  sample sprite {:?} at {},{} into {:?} {} px ({})",
            f.prop, f.x, f.y, f.into, f.px, f.what
        );
    }
    Ok(())
}
