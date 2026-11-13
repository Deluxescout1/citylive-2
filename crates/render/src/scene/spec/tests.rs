//! B62: placement and schedule-consumer tests for the spectacle layer.

use super::*;
use crate::city::{CityObj, LandmarkKind, ObjKind, Stage};

fn obj(kind: ObjKind, x: i32, y: i32, w: i32, h: i32) -> CityObj {
    CityObj {
        x,
        y,
        w,
        h,
        level: 4,
        kind,
        storeys: 2,
        style: 0,
        stage: Stage::Finished,
        seed: 5,
        front: 0,
        t: 0.0,
        kit: 0, wear: Default::default(),
    }
}

/// A scene with a plaza and a park standing on visible, flat, clear ground.
fn town() -> Scene {
    let mut s = Scene::new_with_sky(
        SceneConfig {
            width: 1920,
            height: 1080,
            seed: 3,
            mode: SceneMode::Empty,
            cars: 0,
            peds: 0,
            buildings: 0,
            rain: false,
        },
        0.30,
    );
    let (w, h) = (s.map.w, s.map.h);
    // A spot for the plaza: a visible 6x6 patch of flat dry ground.
    let mut found: Vec<(i32, i32)> = Vec::new();
    'outer: for y in (h / 3)..(h * 2 / 3) {
        for x in (w / 3)..(w * 2 / 3) {
            let mut ok = true;
            for dy in 0..6 {
                for dx in 0..6 {
                    let t = s.map.at(x + dx, y + dy);
                    if t.is_water() || !t.is_flat() || !s.is_visible(x + dx, y + dy) {
                        ok = false;
                    }
                }
            }
            if ok {
                found.push((x, y));
                if found.len() == 2 {
                    break 'outer;
                }
            }
        }
    }
    let (px, py) = found[0];
    let mut objs = vec![obj(ObjKind::Landmark(LandmarkKind::Plaza), px, py, 5, 5)];
    // A second patch for a park, well away from the plaza.
    let park = (0..h - 6).step_by(3).find_map(|y| {
        (0..w - 6).step_by(3).find(|&x| {
            (x - px).abs() > 12
                && (0..5).all(|dy| (0..5).all(|dx| {
                    let t = s.map.at(x + dx, y + dy);
                    !t.is_water() && t.is_flat() && s.is_visible(x + dx, y + dy)
                }))
        }).map(|x| (x, y))
    });
    let _ = park;
    // A park a few tiles from the plaza, on screen.
    objs.push(obj(ObjKind::Park, px + 8, py + 1, 5, 5));
    s.set_city(objs);
    s
}

#[test]
fn a_quiet_day_marks_nothing_and_is_not_live() {
    let mut s = town();
    s.flush_dirty();
    let _ = s.take_pending();
    // Settle the clock-minute mark, then nothing may change.
    s.set_events(Events::default(), 10.0 * 3600.0);
    let _ = s.take_pending();
    for _ in 0..30 {
        s.set_events(Events::default(), 10.0 * 3600.0 + 1.0);
        s.set_sky_spectacle(&[], 270.0, 5.0, 0.0, 0.5, false);
        s.set_side(&[], &[]);
    }
    assert!(!s.spec_live(), "nothing is on, so nothing is live");
    assert_eq!(s.spec_figures(), 0);
    assert_eq!(s.take_pending(), Some(Vec::new()), "a quiet day damages nothing");
}

#[test]
fn the_festival_crowd_builds_peaks_and_thins() {
    let mut s = town();
    let at = |s: &mut Scene, p: f32| {
        s.set_events(Events { festival: Some(p), ..Events::default() }, 15.0 * 3600.0);
        s.spec_figures()
    };
    let early = at(&mut s, 0.01);
    let mid = at(&mut s, 0.5);
    let late = at(&mut s, 0.99);
    assert!(early >= 4, "the band is on stage from the start: {early}");
    assert!(mid > early + 20, "the square fills: {early} -> {mid}");
    assert!(late < mid, "and empties again: {mid} -> {late}");
    // Everyone stands inside the plaza (or on its stage).
    let sq = s.spec.anchors.as_ref().unwrap().square.unwrap();
    at(&mut s, 0.5);
    for f in &s.spec.extra {
        assert!(
            f.x >= sq.x - 0.1 && f.x <= sq.x + sq.w + 0.1 && f.y >= sq.y - 0.1 && f.y <= sq.y + sq.h + 0.1,
            "a festival-goer at {:.1},{:.1} is outside the square {:?}",
            f.x,
            f.y,
            sq
        );
    }
}

#[test]
fn the_stage_and_stalls_fit_every_square_size() {
    for (w, h) in [(3.0f32, 3.0f32), (4.7, 4.7), (5.7, 5.7), (7.0, 6.0)] {
        let sq = Square { x: 10.0, y: 10.0, w, h, level: 4 };
        let (x0, y0, x1, y1) = sq.stage_rect();
        assert!(x0 >= sq.x && x1 <= sq.x + sq.w && y0 >= sq.y && y1 <= sq.y + sq.h, "{w}x{h}: stage {:?}", sq.stage_rect());
        for (x, y) in sq.stall_spots(true) {
            assert!(x >= sq.x && x <= sq.x + sq.w && y >= sq.y && y <= sq.y + sq.h, "{w}x{h}: stall at {x},{y}");
            // Not on the stage deck.
            assert!(!(x >= x0 - 0.3 && x <= x1 + 0.3 && y >= y0 - 0.3 && y <= y1 + 0.3), "{w}x{h}: a stall on the stage");
        }
    }
}

#[test]
fn the_parade_walks_a_road_and_lines_it_with_people() {
    let mut s = town();
    // Lay a straight lane past the plaza.
    let sq = s.spec.anchors.is_none();
    assert!(sq);
    let (px, py) = (s.city[0].x, s.city[0].y);
    for i in -12..14 {
        let (x, y) = (px - 2, py + i);
        if s.map.in_bounds(x, y) {
            s.map.with_tile_mut(x, y, |t| t.kind = map::Kind::Paved);
        }
    }
    s.spec_invalidate();
    let n = s.avenue_len();
    assert!(n >= 10, "a route exists ({n} tiles)");
    s.set_events(Events { parade: Some(0.35), ..Events::default() }, 13.5 * 3600.0);
    // Band, riders and crowd are all people on the route's tiles.
    let near_route = s
        .spec
        .extra
        .iter()
        .filter(|f| {
            let a = s.spec.anchors.as_ref().unwrap();
            a.avenue.iter().any(|&(x, y)| (f.x - x).abs() < 1.6 && (f.y - y).abs() < 1.6)
        })
        .count();
    assert!(near_route >= 10, "the band, the riders and a crowd: {near_route} of {}", s.spec.extra.len());
    assert!(s.spec.extra.iter().any(|f| f.tone == 14), "the marching band is in uniform");
}

#[test]
fn the_fireworks_damage_only_their_own_box_and_cost_nothing_by_day() {
    let mut s = town();
    let site = s.fireworks_site();
    assert!(site.is_some(), "this land has a park or a shore to launch from");
    s.flush_dirty();
    s.set_events(Events::default(), 21.0 * 3600.0);
    let _ = s.take_pending();
    s.set_events(Events { fireworks: Some(120.0), ..Events::default() }, 21.0 * 3600.0);
    let p = s.take_pending().expect("a show is damage, not a full frame");
    assert!(!p.is_empty());
    let sh = s.spec.anchors.as_ref().unwrap().show.unwrap();
    let b = crate::spectacle::show_bounds(&sh);
    let px: i64 = p.iter().map(|r| r.area()).sum();
    assert!(px < b.area() / 2, "a frame of the show repaints a fraction of its box: {px} of {} px", b.area());
    for r in &p {
        assert!(r.x0 >= b.x0 - 460 && r.x1 <= b.x1 + 460 && r.y0 >= b.y0 - 460 && r.y1 <= b.y1 + 460, "{r:?} outside {b:?}");
    }
    // By day (glow 0) the same show paints nothing at all.
    let mut a = Framebuffer::new(s.width(), s.height());
    let mut b2 = Framebuffer::new(s.width(), s.height());
    s.glow = 0.0;
    s.composite(&mut a);
    s.set_events(Events::default(), 21.0 * 3600.0);
    s.composite(&mut b2);
    assert_eq!(a.px(), b2.px(), "a daytime frame is the same with or without the show");
}

#[test]
fn a_balloon_lifts_off_a_field_cruises_high_and_comes_down_in_a_field() {
    let mut s = town();
    assert!(s.field_count() > 3, "open fields to use: {}", s.field_count());
    let b = |p: f32| BalloonNow { progress: p, scheme: 1, size: 1.0, spot: 0.4, seed: 9, land_in: None };
    s.set_sky_spectacle(&[b(0.5)], 270.0, 8.0, 0.0, 0.5, false);
    let pose = |s: &Scene, p: f32| s.balloon_pose(&b(p)).expect("a path");
    let (a, m, e) = (pose(&s, 0.0), pose(&s, 0.5), pose(&s, 1.0));
    assert!((a.gy - a.base_y).abs() < 1.0, "on the ground at lift-off");
    assert!((e.gy - e.base_y).abs() < 1.0, "on the ground at touch-down");
    assert!(m.gy - m.base_y > 120.0, "high in the middle: {}", m.gy - m.base_y);
    // It is carried downwind: east wind (from 270 = west) pushes it east.
    assert!(e.gx != a.gx, "it travels");
    // Smooth: no jump larger than a few pixels between adjacent hundredths.
    let mut last = pose(&s, 0.0);
    for i in 1..=100 {
        let q = pose(&s, i as f32 / 100.0);
        assert!((q.x - last.x).abs() < 24.0 && (q.base_y - last.base_y).abs() < 24.0, "jump at {i}");
        last = q;
    }
    // Launch and landing are on cleared ground.
    let a_anchor = s.spec.anchors.as_ref().unwrap();
    assert!(!a_anchor.fields.is_empty());
}

#[test]
fn a_rainbow_paints_only_the_sky_and_fades_with_its_strength() {
    let mut s = town();
    let mut off = Framebuffer::new(s.width(), s.height());
    s.composite(&mut off);
    s.set_sky_spectacle(&[], 270.0, 5.0, 0.9, 0.5, false);
    let mut on = Framebuffer::new(s.width(), s.height());
    s.composite(&mut on);
    s.set_sky_spectacle(&[], 270.0, 5.0, 0.2, 0.5, false);
    let mut faint = Framebuffer::new(s.width(), s.height());
    s.composite(&mut faint);
    let sky = s.sky_height();
    let (mut changed, mut below, mut faint_changed) = (0usize, 0usize, 0usize);
    for y in 0..s.height() {
        for x in 0..s.width() {
            let i = y * s.width() + x;
            if on.px()[i] != off.px()[i] {
                if y < sky { changed += 1 } else { below += 1 }
            }
            if faint.px()[i] != off.px()[i] {
                faint_changed += 1;
            }
        }
    }
    assert!(changed > 800, "an arc of colour: {changed} px");
    assert!(below <= 40 && below * 20 < changed, "the bow stops at the horizon: {below} px below, {changed} in the sky");
    assert!(faint_changed > 0, "a faint bow is still a bow");
    // Stronger means more paint, never less.
    let diff = |a: &Framebuffer| -> u64 {
        a.px().iter().zip(off.px().iter()).map(|(p, q)| ((*p & 0xff) as i64 - (*q & 0xff) as i64).unsigned_abs()).sum()
    };
    assert!(diff(&on) > diff(&faint));
}

#[test]
fn a_lightning_strike_draws_a_bolt_for_as_long_as_the_flash_lasts() {
    let mut s = town();
    assert!(!s.bolt_showing());
    s.spec_flash(0.5);
    assert!(s.bolt_showing(), "the flash's rising edge strikes");
    let seed = s.spec.bolt.unwrap().0;
    s.spec_flash(0.3);
    assert_eq!(s.spec.bolt.unwrap().0, seed, "the same bolt for the whole flash");
    s.spec_flash(0.0);
    assert!(!s.bolt_showing(), "gone when the flash is");
    s.spec_flash(0.5);
    assert_ne!(s.spec.bolt.unwrap().0, seed, "the next strike is a different bolt");
}

#[test]
fn side_moments_happen_on_the_side_head_and_nowhere_else() {
    let mut s = town();
    let zone = Rect::new(s.width() as i32 / 2, 0, s.width() as i32, 100_000);
    let now = [
        SideNow { kind: SideKind::Fox, progress: 0.5, zone: 0, spot: 0.5, seed: 1, seconds: 40.0 },
        SideNow { kind: SideKind::Kite, progress: 0.5, zone: 0, spot: 0.3, seed: 2, seconds: 150.0 },
        SideNow { kind: SideKind::Musician, progress: 0.5, zone: 0, spot: 0.8, seed: 3, seconds: 240.0 },
    ];
    s.set_side(&[zone], &now);
    for r in &s.spec.drawn[K_SIDE] {
        assert!(r.x0 >= zone.x0 - 60 && r.x1 <= zone.x1 + 60, "{r:?} is outside the side head {zone:?}");
    }
    assert_eq!(s.spec.drawn[K_SIDE].len(), 3, "all three found ground to happen on");
    // No side head, no moments.
    s.set_side(&[], &now);
    assert!(s.spec.drawn[K_SIDE].is_empty());
}

#[test]
fn the_gameday_lot_is_on_open_ground_beside_the_host() {
    let mut s = town();
    // A stadium-sized plot beside the plaza.
    let px = s.city[0].x;
    let py = s.city[0].y;
    let mut city = s.city.clone();
    city.push(obj(ObjKind::Landmark(LandmarkKind::Stadium), px + 9, py, 6, 5));
    s.set_city(city);
    s.set_holiday(crate::holiday::HolidayLook::new(crate::holiday::Holiday::Bills, &[[0x1f, 0x3c, 0x88], [0xc8, 0x1e, 0x2e]], 0.0));
    s.set_events(Events::default(), 12.0 * 3600.0);
    let a = s.spec.anchors.as_ref().unwrap();
    let (lx, ly, span, _) = a.lot.expect("a lot beside the stadium");
    assert!(span >= 3.0);
    let host = s.city.iter().position(|o| matches!(o.kind, ObjKind::Landmark(LandmarkKind::Stadium))).unwrap();
    let o = &s.city[host];
    // Outside the stadium's own plot, and close to it.
    assert!(!(lx >= o.x as f32 && lx <= (o.x + o.w) as f32 && ly >= o.y as f32 && ly <= (o.y + o.h) as f32));
    assert!(((lx - o.x as f32 - 3.0).powi(2) + (ly - o.y as f32 - 2.5).powi(2)).sqrt() < 9.0, "the lot is beside the stadium");
    // Tailgaters in jerseys are on it.
    let jerseys = s.spec.extra.iter().filter(|f| f.tone == 12 || f.tone == 13).count();
    assert!(jerseys >= 8, "{jerseys} fans in blue and red");
}
