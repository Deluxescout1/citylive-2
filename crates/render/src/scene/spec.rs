//! Spectacle in the scene (B62): the state, the places things go, the crowds,
//! and the per-frame drawing.
//!
//! The schedule (what is on, when) lives in `world::spectacle` and arrives here
//! as plain data. This module resolves it against the city: the square is where
//! the festival goes, the avenue where the parade walks, the shore or the park
//! where the fireworks go up, a field where a balloon lifts off. It costs
//! nothing when nothing is on: no events, no balloons, no rainbow means every
//! hook returns at its first line.

use super::*;
use crate::tex;
use crate::people::{Figure, POSE_CHAT, POSE_PLAY, POSE_STAND, POSE_WALK, POSE_WAVE, POSE_WORK};
use crate::spectacle::{self as art, Dress, Events, Light, Show, Square};

/// What a balloon in the sky is doing, as the schedule reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BalloonNow {
    /// 0..1 through the flight.
    pub progress: f32,
    pub scheme: u8,
    pub size: f32,
    /// 0..1: where in the field list it lifts off.
    pub spot: f32,
    pub seed: u32,
    /// Set for a side-monitor landing: it must come down inside this zone.
    pub land_in: Option<usize>,
}

/// A side-monitor moment now on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SideNow {
    pub kind: SideKind,
    pub progress: f32,
    pub zone: usize,
    pub spot: f32,
    pub seed: u32,
    pub seconds: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideKind {
    Fox,
    Kite,
    Balloon,
    Musician,
}

/// Where things go in this city.
#[derive(Default, Clone)]
pub(crate) struct Anchors {
    square: Option<Square>,
    /// Index in `city` of the festival's host.
    host: Option<usize>,
    /// The parade's road, in tile coordinates (tile centres), in walking order.
    avenue: Vec<(f32, f32)>,
    show: Option<Show>,
    /// Open ground a balloon can lift off from or land on, tile coordinates.
    fields: Vec<(i32, i32)>,
    /// The clock tower's tile and level.
    clock: Option<(f32, f32, u8)>,
    zoo: Option<(f32, f32, f32, f32, u8)>,
    /// The gameday lot: centre in tiles, span, level.
    lot: Option<(f32, f32, f32, u8)>,
}

const K_SHOW: usize = 0;
const K_PARADE: usize = 1;
const K_BALLOON: usize = 2;
const K_SIDE: usize = 3;
const K_CLOCK: usize = 4;
const K_NIGHT: usize = 5;
const K_LOT: usize = 6;
const KINDS: usize = 7;

pub(crate) struct SpecState {
    pub events: Events,
    pub day_secs: f32,
    pub show_dur: f32,
    pub anchors: Option<Anchors>,
    /// Crowd and performers, appended to the people each frame.
    pub extra: Vec<Figure>,
    /// Index in the people list where the spectacle's figures start.
    pub extra_from: usize,
    crowd_key: u64,
    balloon_sig: Vec<(i32, i32, i32, i32)>,
    #[allow(clippy::type_complexity)]
    ground_cache: std::cell::RefCell<Vec<((i32, i32, i32, u32, bool), Option<(f32, f32, f32, f32)>)>>,
    pub balloons: Vec<BalloonNow>,
    paths: Vec<(u32, [f32; 4])>,
    pub wind: (f32, f32),
    pub wind_kmh: f32,
    pub rainbow: f32,
    pub rainbow_cx: f32,
    pub storm_special: bool,
    bolt: Option<(u32, f32)>,
    bolt_count: u32,
    last_flash: f32,
    pub zones: Vec<Rect>,
    pub side: Vec<SideNow>,
    /// The boxes each kind of overlay painted last frame (for damage).
    drawn: [Vec<Rect>; KINDS],
    clock_min: i32,
}

impl Default for SpecState {
    fn default() -> Self {
        Self {
            events: Events::default(),
            day_secs: 12.0 * 3600.0,
            show_dur: art::SHOW_SECS,
            anchors: None,
            extra: Vec::new(),
            extra_from: usize::MAX,
            crowd_key: u64::MAX,
            balloon_sig: Vec::new(),
            ground_cache: Default::default(),
            balloons: Vec::new(),
            paths: Vec::new(),
            wind: (1.0, 0.3),
            wind_kmh: 6.0,
            rainbow: 0.0,
            rainbow_cx: 0.5,
            storm_special: false,
            bolt: None,
            bolt_count: 0,
            last_flash: 0.0,
            zones: Vec::new(),
            side: Vec::new(),
            drawn: Default::default(),
            clock_min: -1,
        }
    }
}

/// Props that stand in the way of a person or a tent (a tree, a bush, a rock, a
/// log); wildflowers and tall grass do not.
fn blocks(p: map::Prop) -> bool {
    matches!(p, map::Prop::Tree { .. } | map::Prop::Bush(_) | map::Prop::Boulder(_) | map::Prop::Log(_))
}

fn hs(a: u32, b: u32) -> f32 {
    tex::hash1(a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA6B).rotate_left(9))
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl SpecState {
    pub(crate) fn rainbow_on(&self) -> bool {
        self.rainbow >= 0.01
    }

    pub(crate) fn bolt_on(&self) -> bool {
        self.bolt.is_some()
    }

    /// Is any spectacle live? (False means every hook is a no-op.)
    fn quiet(&self) -> bool {
        !self.events.any_active()
            && self.balloons.is_empty()
            && self.rainbow < 0.01
            && self.bolt.is_none()
            && self.side.is_empty()
    }
}

impl Events {
    fn any_active(&self) -> bool {
        self.festival.is_some() || self.parade.is_some() || self.fireworks.is_some() || self.night_market.is_some()
    }
}

impl Scene {
    // ---- anchors -----------------------------------------------------------

    pub(crate) fn spec_anchors(&self) -> Anchors {
        let mut a = Anchors::default();
        let map = &self.map;
        // The festival's host: the civic square, else the biggest park.
        let plaza = self
            .city
            .iter()
            .position(|o| matches!(o.kind, crate::city::ObjKind::Landmark(crate::city::LandmarkKind::Plaza)));
        let park = self
            .city
            .iter()
            .enumerate()
            .filter(|(_, o)| matches!(o.kind, crate::city::ObjKind::Park) && o.w >= 3 && o.h >= 3)
            .max_by_key(|(_, o)| o.w * o.h)
            .map(|(i, _)| i);
        a.host = plaza.or(park);
        if let Some(i) = a.host {
            let o = &self.city[i];
            // The footprint the plaza art is drawn into.
            a.square = Some(Square {
                x: o.x as f32 + 0.13,
                y: o.y as f32 + 0.13,
                w: o.w as f32 - 0.26,
                h: o.h as f32 - 0.26,
                level: o.level,
            });
            if plaza.is_some() {
                let (tx, ty) = crate::city::clock_tower_spot(o.x as f32 + 0.13, o.y as f32 + 0.13, o.w as f32 - 0.26, o.h as f32 - 0.26);
                a.clock = Some((tx, ty, o.level));
            }
        }
        // The parade's road: the longest straight run of lanes passing the host.
        if let Some(sq) = a.square {
            a.avenue = self.find_avenue(&sq);
        }
        // Open fields: flat grass with nothing on it or beside it, in view.
        let (w, h) = (map.w, map.h);
        let sky = self.sky_h as i32;
        let (fw, fh) = (self.width() as i32, self.height() as i32);
        let open = |x: i32, y: i32| {
            let t = map.at(x, y);
            matches!(t.kind, map::Kind::Grass | map::Kind::Meadow)
                && t.is_flat()
                && !blocks(t.prop)
                && self.city_key.get((y * w + x) as usize).copied().unwrap_or(1) == 0
        };
        let mut y = 2;
        while y < h - 2 {
            let mut x = 2;
            while x < w - 2 {
                if open(x, y) && open(x + 1, y) && open(x - 1, y) && open(x, y + 1) && open(x, y - 1) && self.is_visible(x, y) {
                    let lvl = self.level_px(map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5) as u8);
                    let (sx, sy) = self.cam.project(x as f32 + 0.5, y as f32 + 0.5, lvl);
                    if sx > 80.0 && sx < (fw - 80) as f32 && sy > (sky + 140) as f32 && sy < (fh - 50) as f32 {
                        a.fields.push((x, y));
                    }
                }
                x += 2;
            }
            y += 2;
        }
        // The fireworks site.
        a.show = self.find_show_site(&a);
        // Zoo and the gameday lot.
        for o in &self.city {
            if let crate::city::ObjKind::Landmark(crate::city::LandmarkKind::Zoo) = o.kind {
                if o.stage == crate::city::Stage::Finished {
                    a.zoo = Some((o.x as f32, o.y as f32, o.w as f32, o.h as f32, o.level));
                }
            }
        }
        if let Some(i) = gameday_host(&self.city) {
            a.lot = self.find_lot(&self.city[i]);
        }
        a
    }

    /// The gameday lot: the clearest 5x2 patch of open ground beside the host,
    /// nearest to it. People and tents standing behind a tree are not seen, so
    /// the lot goes where there is room.
    fn find_lot(&self, o: &CityObj) -> Option<(f32, f32, f32, u8)> {
        let map = &self.map;
        let open = |x: i32, y: i32| {
            map.in_bounds(x, y) && {
                let t = map.at(x, y);
                !t.is_water() && t.relief() <= 1 && !blocks(t.prop) && self.city_key[(y * map.w + x) as usize] == 0
                    && !matches!(t.kind, map::Kind::Rock)
            }
        };
        let mut best: Option<(f32, (f32, f32))> = None;
        let (cx0, cy0) = (o.x as f32 + o.w as f32 * 0.5, o.y as f32 + o.h as f32 * 0.5);
        for y in (o.y - 6)..(o.y + o.h + 7) {
            for x in (o.x - 7)..(o.x + o.w + 7) {
                let mut n = 0;
                for dy in 0..3 {
                    for dx in 0..6 {
                        if open(x + dx, y + dy) {
                            n += 1;
                        }
                    }
                }
                if n < 13 {
                    continue;
                }
                let (px, py) = (x as f32 + 3.0, y as f32 + 1.5);
                let d = ((px - cx0).powi(2) + (py - cy0).powi(2)).sqrt() - n as f32 * 0.4;
                if best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, (px, py - 0.7)));
                }
            }
        }
        best.map(|(_, (x, y))| (x, y, 4.4, o.level))
    }

    /// The parade's road: from the lane nearest the square, follow the street
    /// both ways (keeping straight when it can) for up to ~34 tiles. Streets
    /// are whatever the sim laid, so the route turns where the street turns.
    fn find_avenue(&self, sq: &Square) -> Vec<(f32, f32)> {
        let map = &self.map;
        let lane = |x: i32, y: i32| map.in_bounds(x, y) && map.at(x, y).kind.is_lane() && !map.at(x, y).is_water() && self.is_visible(x, y);
        let (cx, cy) = ((sq.x + sq.w * 0.5) as i32, (sq.y + sq.h * 0.5) as i32);
        // The nearest lane tile within reach.
        let mut start: Option<(i32, i32)> = None;
        'search: for r in 1..16i32 {
            let mut best: Option<(i32, (i32, i32))> = None;
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dy.abs()) != r {
                        continue;
                    }
                    let (x, y) = (cx + dx, cy + dy);
                    if lane(x, y) {
                        let d = dx * dx + dy * dy;
                        if best.map_or(true, |(bd, _)| d < bd) {
                            best = Some((d, (x, y)));
                        }
                    }
                }
            }
            if let Some((_, p)) = best {
                start = Some(p);
                break 'search;
            }
        }
        let Some(start) = start else { return Vec::new() };
        let dirs = [(1, 0), (0, 1), (-1, 0), (0, -1)];
        let walk = |first: (i32, i32)| -> Vec<(i32, i32)> {
            let mut out = Vec::new();
            let mut seen = std::collections::HashSet::new();
            seen.insert(start);
            let (mut p, mut d) = (start, first);
            for _ in 0..34 {
                // Straight on, else turn.
                let cands = [d, (d.1, -d.0), (-d.1, d.0)];
                let mut moved = false;
                for c in cands {
                    let n = (p.0 + c.0, p.1 + c.1);
                    if lane(n.0, n.1) && !seen.contains(&n) {
                        seen.insert(n);
                        out.push(n);
                        p = n;
                        d = c;
                        moved = true;
                        break;
                    }
                }
                if !moved {
                    break;
                }
            }
            out
        };
        let mut walks: Vec<Vec<(i32, i32)>> = dirs.iter().map(|d| walk(*d)).collect();
        walks.sort_by_key(|w| std::cmp::Reverse(w.len()));
        // Two longest walks that leave in different directions.
        let a = walks[0].clone();
        let b = walks.iter().skip(1).find(|w| w.first().zip(a.first()).map_or(false, |(p, q)| p != q)).cloned().unwrap_or_default();
        let mut path: Vec<(i32, i32)> = b.into_iter().rev().collect();
        path.push(start);
        path.extend(a);
        if path.len() < 10 {
            return Vec::new();
        }
        path.into_iter().map(|(x, y)| (x as f32 + 0.5, y as f32 + 0.5)).collect()
    }

    fn find_show_site(&self, a: &Anchors) -> Option<Show> {
        let map = &self.map;
        let (fw, fh) = (self.width() as f32, self.height() as f32);
        let sky = self.sky_h as f32;
        let centre = a.square.map(|s| (s.x + s.w * 0.5, s.y + s.h * 0.5)).unwrap_or((map.w as f32 * 0.5, map.h as f32 * 0.5));
        let on_screen = |x: f32, y: f32| {
            let lvl = self.level_px(map.surface_level_at(x, y) as u8);
            let p = self.cam.project(x, y, lvl);
            (p.0 > 360.0 && p.0 < fw - 360.0 && p.1 > sky + 160.0 && p.1 < fh - 24.0).then_some(p)
        };
        // A shore: water with dry land beside it, near the middle of town.
        let mut best: Option<(f32, (f32, f32))> = None;
        for y in (1..map.h - 1).step_by(2) {
            for x in (1..map.w - 1).step_by(2) {
                if !map.at(x, y).is_water() {
                    continue;
                }
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (lx, ly) = (x + dx, y + dy);
                    if map.at(lx, ly).is_water() || self.city_key[(ly * map.w + lx) as usize] != 0 {
                        continue;
                    }
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let d = ((fx - centre.0).powi(2) + (fy - centre.1).powi(2)).sqrt();
                    if !(5.0..44.0).contains(&d) {
                        continue;
                    }
                    if let Some(p) = on_screen(fx, fy) {
                        if best.map_or(true, |(bd, _)| d < bd) {
                            best = Some((d, p));
                        }
                    }
                }
            }
        }
        if let Some((_, (x, y))) = best {
            return Some(Show { cx: x, ground_y: y, span: 300.0, water: true, burst_top: sky * 0.16, burst_bot: sky * 0.82 });
        }
        // Else over the biggest park.
        let park = self
            .city
            .iter()
            .filter(|o| matches!(o.kind, crate::city::ObjKind::Park))
            .max_by_key(|o| o.w * o.h)?;
        let p = on_screen(park.x as f32 + park.w as f32 * 0.5, park.y as f32 + park.h as f32 * 0.5)?;
        Some(Show { cx: p.0, ground_y: p.1, span: 260.0, water: false, burst_top: sky * 0.16, burst_bot: sky * 0.82 })
    }

    fn anchors(&mut self) {
        if self.spec.anchors.is_none() {
            self.spec.anchors = Some(self.spec_anchors());
        }
    }

    /// Called when the city or the map changes: the places are stale.
    pub(crate) fn spec_invalidate(&mut self) {
        self.spec.anchors = None;
        self.spec.crowd_key = u64::MAX;
        self.spec.paths.clear();
        self.spec.ground_cache.borrow_mut().clear();
    }

    /// Where the fireworks go (for the spike and the tests).
    pub fn fireworks_site(&mut self) -> Option<(f32, f32, bool)> {
        self.anchors();
        self.spec.anchors.as_ref()?.show.map(|s| (s.cx, s.ground_y, s.water))
    }

    /// The festival square in tiles (for the spike and the tests).
    pub fn festival_square(&mut self) -> Option<(f32, f32, f32, f32)> {
        self.anchors();
        self.spec.anchors.as_ref()?.square.map(|s| (s.x, s.y, s.w, s.h))
    }

    /// Length of the parade road in tiles (0 = none found).
    pub fn avenue_len(&mut self) -> usize {
        self.anchors();
        self.spec.anchors.as_ref().map_or(0, |a| a.avenue.len())
    }

    /// How many open fields a balloon could use (tests).
    pub fn field_count(&mut self) -> usize {
        self.anchors();
        self.spec.anchors.as_ref().map_or(0, |a| a.fields.len())
    }

    /// The static dressing the next static rebuild draws.
    pub(crate) fn spec_dress(&self) -> Dress {
        self.spec.events.dress()
    }

    // ---- inputs ------------------------------------------------------------

    /// Tell the scene what events are on. `day_secs` is the local time of day in
    /// seconds. Returns true when the static layer had to be re-dressed.
    pub fn set_events(&mut self, ev: Events, day_secs: f32) -> bool {
        let before = self.spec.events;
        let dress_changed = before.dress() != ev.dress();
        self.spec.events = ev;
        self.spec.day_secs = day_secs;
        self.anchors();
        if dress_changed {
            self.rebuild_static();
            self.retint_static();
            self.mark_full();
        }
        // Damage: the show, the parade's reach and the lanterns move or flicker.
        let mut show: Vec<Rect> = Vec::new();
        if ev.fireworks.is_some() {
            if let Some(s) = self.spec.anchors.as_ref().and_then(|a| a.show) {
                show = art::show_rects(&s, ev.fireworks.unwrap_or(0.0), self.spec.show_dur, self.seed as u32);
            }
        }
        self.spec_mark(K_SHOW, show);
        let mut parade: Vec<Rect> = Vec::new();
        if let Some(p) = ev.parade {
            parade = self.spec_parade_rects(p);
        }
        self.spec_mark(K_PARADE, parade);
        // The clock's hands move once a minute.
        let min = (day_secs / 60.0) as i32;
        if min != self.spec.clock_min {
            self.spec.clock_min = min;
            let mut v = Vec::new();
            if let Some((x, y, l)) = self.spec.anchors.as_ref().and_then(|a| a.clock) {
                for (fx, fy) in art::clock_faces(&self.cam, x, y, l) {
                    v.push(Rect::new(fx as i32 - 9, fy as i32 - 8, fx as i32 + 10, fy as i32 + 10));
                }
            }
            self.spec_mark(K_CLOCK, v.clone());
            // Mark once more so the old hands are repainted away.
            for r in v {
                self.mark_rect(r);
            }
        }
        // The night market's lanterns flicker with the stalls: one box.
        let mut night = Vec::new();
        if ev.night_market.is_some() {
            if let Some(sq) = self.spec.anchors.as_ref().and_then(|a| a.square) {
                let c = self.cam.project(sq.x + sq.w * 0.5, sq.y + sq.h * 0.5, sq.level as i32 * crate::iso::HEIGHT_UNIT);
                let w = (sq.w + sq.h) * 34.0;
                night.push(Rect::new((c.0 - w) as i32, (c.1 - w * 0.6) as i32, (c.0 + w) as i32, (c.1 + w * 0.4) as i32));
            }
        }
        // Lanterns only need repainting when the light changes: no per-frame box.
        self.spec.drawn[K_NIGHT] = night;
        self.rebuild_crowd(false);
        dress_changed
    }

    /// Mark the union of last frame's and this frame's boxes of one overlay kind.
    fn spec_mark(&mut self, kind: usize, now: Vec<Rect>) {
        let old = std::mem::take(&mut self.spec.drawn[kind]);
        for r in old.iter().chain(now.iter()) {
            self.mark_rect(*r);
        }
        self.spec.drawn[kind] = now;
    }

    /// The sky's spectacle: balloons on the wind, a rainbow, a spectacular storm.
    /// `wind_from` is where the wind comes from in degrees; `rainbow_cx` the
    /// bow's centre as a 0..1 fraction of the frame's width.
    #[allow(clippy::too_many_arguments)]
    pub fn set_sky_spectacle(
        &mut self,
        balloons: &[BalloonNow],
        wind_from: f32,
        wind_kmh: f32,
        rainbow: f32,
        rainbow_cx: f32,
        storm_special: bool,
    ) {
        // The wind blows toward `wind_from + 180`; map that ground direction to
        // the screen.
        let (gx, gy) = crate::light::toward(wind_from + 180.0);
        let (sx, sy) = ((gx - gy) * 2.0, (gx + gy));
        let n = (sx * sx + sy * sy).sqrt().max(1e-3);
        self.spec.wind = (sx / n, sy / n);
        self.spec.wind_kmh = wind_kmh;
        self.spec.storm_special = storm_special;
        // A rainbow repaints the sky: quantised, and only when it moves a step.
        let q = (rainbow * 24.0).round() / 24.0;
        if (q - self.spec.rainbow).abs() > 0.001 || (rainbow_cx - self.spec.rainbow_cx).abs() > 0.004 {
            self.spec.rainbow = q;
            self.spec.rainbow_cx = rainbow_cx;
            self.mark_full();
        }
        self.anchors();
        // Balloons: where each is, then the damage.
        let list: Vec<BalloonNow> = balloons.to_vec();
        self.spec.balloons = list;
        self.balloon_cache();
        let mut rects = Vec::new();
        let mut sig: Vec<(i32, i32, i32, i32)> = Vec::new();
        let t = self.spec.day_secs;
        for b in self.spec.balloons.clone() {
            if let Some(p) = self.balloon_pose(&b) {
                rects.push(Rect::new(p.x as i32 - 38, (p.base_y - 102.0 * b.size) as i32, p.x as i32 + 38, p.base_y as i32 + 8));
                sig.push((p.x as i32, p.base_y as i32, p.gx as i32, p.gy as i32));
                // Its shadow.
                rects.push(Rect::new(p.gx as i32 - 34, p.gy as i32 - 16, p.gx as i32 + 34, p.gy as i32 + 16));
            }
        }
        let _ = t;
        // A balloon drifts a pixel every few frames: only damage when one moved.
        if sig != self.spec.balloon_sig {
            self.spec.balloon_sig = sig;
            self.spec_mark(K_BALLOON, rects);
        }
        if balloons.is_empty() {
            self.spec.paths.clear();
        }
    }

    /// Side-monitor moments now on, and the zones (framebuffer rects of the side
    /// heads) they happen in.
    pub fn set_side(&mut self, zones: &[Rect], now: &[SideNow]) {
        self.spec.zones = zones.to_vec();
        self.spec.side = now.to_vec();
        self.anchors();
        let mut rects = Vec::new();
        for s in self.spec.side.clone() {
            match s.kind {
                SideKind::Fox => {
                    if let Some((x, y, _)) = self.fox_pose(&s) {
                        rects.push(Rect::new(x as i32 - 20, y as i32 - 14, x as i32 + 20, y as i32 + 4));
                    }
                }
                SideKind::Kite => {
                    if let Some(k) = self.kite_pose(&s) {
                        rects.push(Rect::new(k.0 as i32 - 14, k.1 as i32 - 14, k.0 as i32 + 14, k.2 as i32 + 8));
                    }
                }
                SideKind::Musician => {
                    if let Some((x, y)) = self.musician_spot(&s) {
                        rects.push(Rect::new(x as i32 - 14, y as i32 - 44, x as i32 + 18, y as i32 + 2));
                    }
                }
                SideKind::Balloon => {}
            }
        }
        if std::env::var("SIM_DEBUG").is_ok() {
            eprintln!("side moments: {} asked, {} placed at {:?}", self.spec.side.len(), rects.len(), rects.iter().map(|r| (r.x0, r.y0, r.x1, r.y1)).collect::<Vec<_>>());
        }
        self.spec_mark(K_SIDE, rects);
        self.rebuild_crowd(false);
    }

    // ---- places ------------------------------------------------------------

    /// A free ground tile inside a zone's x-range, near `spot` (0..1 across it),
    /// preferring the lower half of the frame. Returns tile centre + screen xy.
    fn zone_ground(&self, zone: Rect, spot: f32, seed: u32, want_road: bool) -> Option<(f32, f32, f32, f32)> {
        // The search walks the map; the answer does not change while the city
        // does not, so it is remembered (invalidated with the anchors).
        let key = (zone.x0, zone.x1, (spot * 1000.0) as i32, seed, want_road);
        if let Some((_, v)) = self.spec.ground_cache.borrow().iter().find(|(k, _)| *k == key) {
            return *v;
        }
        let v = self.zone_ground_search(zone, spot, seed, want_road);
        let mut c = self.spec.ground_cache.borrow_mut();
        if c.len() > 32 {
            c.clear();
        }
        c.push((key, v));
        v
    }

    fn zone_ground_search(&self, zone: Rect, spot: f32, seed: u32, want_road: bool) -> Option<(f32, f32, f32, f32)> {
        let map = &self.map;
        let w = map.w;
        let target_x = zone.x0 as f32 + zone.width() as f32 * (0.15 + 0.7 * spot);
        let target_y = self.sky_h as f32 + (self.height() as f32 - self.sky_h as f32) * 0.62;
        let free = |x: i32, y: i32| {
            map.in_bounds(x, y) && {
                let t = map.at(x, y);
                !t.is_water() && !t.kind.is_lane() && t.kind != map::Kind::Sidewalk && self.city_key[(y * w + x) as usize] == 0
            }
        };
        // First a true meadow (nothing built within two tiles), then any gap.
        for strict in [true, false] {
            let mut best: Option<(f32, (f32, f32, f32, f32))> = None;
            for y in 3..map.h - 3 {
                for x in 3..w - 3 {
                    let t = map.at(x, y);
                    let ok = if want_road {
                        matches!(t.kind, map::Kind::Sidewalk | map::Kind::Paved | map::Kind::Road) || (t.kind == map::Kind::Grass && t.is_flat())
                    } else {
                        matches!(t.kind, map::Kind::Grass | map::Kind::Meadow)
                    };
                    if !ok || !t.is_flat() || blocks(t.prop) {
                        continue;
                    }
                    if self.city_key[(y * w + x) as usize] != 0 || !self.is_visible(x, y) {
                        continue;
                    }
                    if strict && !want_road {
                        let mut all = true;
                        for dy in -2..=2 {
                            for dx in -2..=2 {
                                if !free(x + dx, y + dy) {
                                    all = false;
                                }
                            }
                        }
                        if !all {
                            continue;
                        }
                    }
                    let lvl = self.level_px(map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5) as u8);
                    let (sx, sy) = self.cam.project(x as f32 + 0.5, y as f32 + 0.5, lvl);
                    if sx < zone.x0 as f32 + 40.0 || sx > zone.x1 as f32 - 40.0 || sy < self.sky_h as f32 + 90.0 || sy > self.height() as f32 - 30.0 {
                        continue;
                    }
                    let d = (sx - target_x).abs() + (sy - target_y).abs() * 0.7 + hs(seed, (x * 131 + y) as u32) * 40.0;
                    if best.map_or(true, |(bd, _)| d < bd) {
                        best = Some((d, (x as f32 + 0.5, y as f32 + 0.5, sx, sy)));
                    }
                }
            }
            if let Some((_, p)) = best {
                return Some(p);
            }
        }
        None
    }

    /// A free run of tiles along +x or +y from a tile, for the fox: returns its
    /// start tile, direction and length.
    fn fox_path(&self, s: &SideNow) -> Option<((f32, f32), (f32, f32), f32)> {
        let zone = *self.spec.zones.get(s.zone)?;
        let (x, y, _, _) = self.zone_ground(zone, s.spot, s.seed, false)?;
        let free = |tx: f32, ty: f32| {
            let (ix, iy) = (tx.floor() as i32, ty.floor() as i32);
            self.map.in_bounds(ix, iy) && {
                let t = self.map.at(ix, iy);
                !t.is_water() && !blocks(t.prop) && self.city_key[(iy * self.map.w + ix) as usize] == 0
            }
        };
        // Choose the longer of the two axes, stepping both ways from the start.
        let mut best = ((x, y), (1.0f32, 0.0f32), 1.0f32);
        for dir in [(1.0f32, 0.0f32), (0.0, 1.0)] {
            let mut a = 0.0;
            while free(x - dir.0 * (a + 1.0), y - dir.1 * (a + 1.0)) && a < 6.0 {
                a += 1.0;
            }
            let mut b = 0.0;
            while free(x + dir.0 * (b + 1.0), y + dir.1 * (b + 1.0)) && b < 6.0 {
                b += 1.0;
            }
            if a + b > best.2 {
                best = ((x - dir.0 * a, y - dir.1 * a), dir, a + b);
            }
        }
        (best.2 >= 3.0).then_some(best)
    }

    /// Screen position and facing of the fox, feet at `(x, y)`.
    fn fox_pose(&self, s: &SideNow) -> Option<(f32, f32, i32)> {
        let (start, dir, len) = self.fox_path(s)?;
        let f = smooth(((s.progress - 0.04) / 0.92).clamp(0.0, 1.0));
        let (tx, ty) = (start.0 + dir.0 * len * f, start.1 + dir.1 * len * f);
        let lvl = self.level_px(self.map.surface_level_at(tx, ty) as u8);
        let p = self.cam.project(tx, ty, lvl);
        // Moving along +x is down-right on the screen, +y down-left.
        let face = if dir.0 > 0.0 { 1 } else { -1 };
        Some((p.0, p.1, face))
    }

    /// The kite: its screen centre x, y and the child's hand y.
    fn kite_pose(&self, s: &SideNow) -> Option<(f32, f32, f32)> {
        let zone = *self.spec.zones.get(s.zone)?;
        let (_, _, sx, sy) = self.zone_ground(zone, s.spot, s.seed, false)?;
        let t = s.progress * s.seconds;
        let rise = smooth(s.progress / 0.12) * smooth((1.0 - s.progress) / 0.12);
        let kx = sx + 26.0 + (t * 0.5).sin() * 14.0 + (t * 1.3).sin() * 4.0;
        let ky = sy - 40.0 - 80.0 * rise + (t * 0.8).cos() * 5.0;
        Some((kx, ky, sy - 7.0))
    }

    fn musician_spot(&self, s: &SideNow) -> Option<(f32, f32)> {
        let zone = *self.spec.zones.get(s.zone)?;
        let (_, _, sx, sy) = self.zone_ground(zone, s.spot, s.seed, false)?;
        Some((sx, sy))
    }

    // ---- balloons ------------------------------------------------------------

    /// Where a balloon is: its ground point (under it), its altitude, its
    /// basket's screen position, and how low it is.
    fn balloon_pose(&self, b: &BalloonNow) -> Option<Pose> {
        let a = self.spec.anchors.as_ref()?;
        let tile_px = |x: i32, y: i32| {
            let lvl = self.level_px(self.map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5) as u8);
            self.cam.project(x as f32 + 0.5, y as f32 + 0.5, lvl)
        };
        let (from, to) = if let Some(&(_, p)) = self.spec.paths.iter().find(|(s, _)| *s == b.seed) {
            ((p[0], p[1]), (p[2], p[3]))
        } else {
            let (w, dir) = (self.spec.wind_kmh, self.spec.wind);
            // The landing: inside the side zone for a side-monitor landing,
            // otherwise the clear field nearest the point the wind would carry
            // a balloon from the launch in this flight's time.
            let dist = (330.0 + w * 28.0).clamp(300.0, 720.0);
            let pick = |ps: &[(i32, i32)], want: (f32, f32), avoid: (f32, f32)| -> Option<(f32, f32)> {
                ps.iter()
                    .map(|&(x, y)| {
                        let p = tile_px(x, y);
                        (((p.0 - want.0).powi(2) + (p.1 - want.1).powi(2)), p)
                    })
                    // Never land where it launched (a balloon goes somewhere).
                    .filter(|(_, p)| (p.0 - avoid.0).abs() + (p.1 - avoid.1).abs() > 90.0)
                    .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
                    .map(|(_, p)| p)
            };
            if a.fields.is_empty() {
                return None;
            }
            let (launch, land) = if let Some(z) = b.land_in.and_then(|z| self.spec.zones.get(z)) {
                let zone_fields: Vec<(i32, i32)> = a
                    .fields
                    .iter()
                    .copied()
                    .filter(|&(x, y)| {
                        let p = tile_px(x, y);
                        p.0 > z.x0 as f32 + 60.0 && p.0 < z.x1 as f32 - 60.0
                    })
                    .collect();
                if zone_fields.is_empty() {
                    return None;
                }
                let land = zone_fields[((b.spot * zone_fields.len() as f32) as usize).min(zone_fields.len() - 1)];
                let lp = tile_px(land.0, land.1);
                let want = (lp.0 - dir.0 * dist, lp.1 - dir.1 * dist * 0.5);
                (pick(&a.fields, want, lp)?, lp)
            } else {
                let li = a.fields[((b.spot * a.fields.len() as f32) as usize).min(a.fields.len() - 1)];
                let lp = tile_px(li.0, li.1);
                let want = (lp.0 + dir.0 * dist, lp.1 + dir.1 * dist * 0.5);
                (lp, pick(&a.fields, want, lp)?)
            };
            let _ = &launch;
            // Cached by seed (the vec is rebuilt on invalidate; `&self` cannot
            // push, so the cache is filled by `balloon_cache`).
            (launch, land)
        };
        let p = b.progress.clamp(0.0, 1.0);
        // Horizontal progress eases in (the lift-off is slow) and out (touchdown).
        let e = smooth((p - 0.05) / 0.9);
        let gx = from.0 + (to.0 - from.0) * e;
        let gy = from.1 + (to.1 - from.1) * e;
        let lift = world_balloon_lift(p);
        let alt = lift * (225.0 + 90.0 * (b.size - 0.9)) + (p * 9.0).sin().abs() * lift * 6.0;
        Some(Pose { x: gx, gx, gy, base_y: gy - alt, lift, climbing: p < 0.2 })
    }

    /// Cache the flights' paths (needs `&mut self`).
    fn balloon_cache(&mut self) {
        let list = self.spec.balloons.clone();
        for b in list {
            if self.spec.paths.iter().any(|(s, _)| *s == b.seed) {
                continue;
            }
            // Resolve once through `balloon_pose`'s search by temporarily
            // asking it for the endpoints.
            if let Some((from, to)) = self.resolve_path(&b) {
                self.spec.paths.push((b.seed, [from.0, from.1, to.0, to.1]));
            }
        }
        if self.spec.paths.len() > 16 {
            let keep: Vec<u32> = self.spec.balloons.iter().map(|b| b.seed).collect();
            self.spec.paths.retain(|(s, _)| keep.contains(s));
        }
    }

    fn resolve_path(&self, b: &BalloonNow) -> Option<((f32, f32), (f32, f32))> {
        // `balloon_pose` finds the endpoints when no cache exists; read them
        // back at progress 0 and 1.
        let saved = BalloonNow { progress: 0.0, ..*b };
        let a = self.balloon_pose(&saved)?;
        let z = BalloonNow { progress: 1.0, ..*b };
        let c = self.balloon_pose(&z)?;
        Some(((a.gx, a.gy), (c.gx, c.gy)))
    }

    // ---- the crowd -------------------------------------------------------------

    fn spec_parade_rects(&self, p: f32) -> Vec<Rect> {
        let Some(a) = self.spec.anchors.as_ref() else { return Vec::new() };
        let Some((head, _)) = self.parade_head(a, p) else { return Vec::new() };
        // A box round the head and the 14 tiles behind it.
        let mut r = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for k in 0..=14 {
            let s = head - k as f32;
            let (x, y) = path_at(&a.avenue, s);
            let lvl = self.level_px(self.map.surface_level_at(x, y) as u8);
            let pt = self.cam.project(x, y, lvl);
            r = r.union(&Rect::new(pt.0 as i32 - 70, pt.1 as i32 - 60, pt.0 as i32 + 70, pt.1 as i32 + 30));
        }
        vec![r]
    }

    /// The parade's head position along the avenue (in tiles from its start).
    fn parade_head(&self, a: &Anchors, p: f32) -> Option<(f32, usize)> {
        let n = a.avenue.len();
        (n >= 10).then(|| (p * (n as f32 + 16.0), n))
    }

    fn rebuild_crowd(&mut self, force: bool) {
        let ev = self.spec.events;
        let bills = self.holiday.holiday.is_bills();
        let hour = self.spec.day_secs / 3600.0;
        let zoo_open = (9.0..17.5).contains(&hour);
        let always = ev.parade.is_some() || ev.night_market.is_some() || !self.spec.side.is_empty();
        let att = ev.festival.map_or(0, |p| (attendance(p) * 16.0).round() as u64 + 1);
        let key = att
            | (ev.night_market.is_some() as u64) << 8
            | (ev.parade.is_some() as u64) << 9
            | (bills as u64) << 10
            | (matches!(self.holiday.holiday, crate::holiday::Holiday::BillsWin) as u64) << 11
            | (zoo_open as u64) << 12
            | (self.spec.anchors.as_ref().map_or(0, |a| a.zoo.is_some() as u64)) << 13;
        if !force && !always && key == self.spec.crowd_key {
            return;
        }
        self.spec.crowd_key = key;
        let mut v: Vec<Figure> = Vec::new();
        let t = self.spec.day_secs;
        let a = self.spec.anchors.clone().unwrap_or_default();
        if let (Some(sq), true) = (a.square, ev.festival.is_some() || ev.night_market.is_some()) {
            if ev.festival.is_some() {
                crowd_festival(&mut v, &sq, attendance(ev.festival.unwrap_or(0.0)), self.seed as u32);
            }
            if ev.night_market.is_some() {
                crowd_night_market(&mut v, &sq, ev.festival.is_some(), t, self.seed as u32);
            }
        }
        if let Some(p) = ev.parade {
            self.crowd_parade(&mut v, &a, p, t);
        }
        if bills || matches!(self.holiday.holiday, crate::holiday::Holiday::BillsWin) {
            if let Some((lx, ly, span, _)) = a.lot {
                crowd_tailgate(&mut v, lx, ly, span, matches!(self.holiday.holiday, crate::holiday::Holiday::BillsWin), self.seed as u32);
            }
        }
        if let (Some((zx, zy, zw, zh, _)), true) = (a.zoo, zoo_open) {
            crowd_zoo(&mut v, zx, zy, zw, zh, self.seed as u32);
        }
        for s in self.spec.side.clone() {
            self.crowd_side(&mut v, &s);
        }
        self.spec.extra = v;
    }

    fn crowd_parade(&self, v: &mut Vec<Figure>, a: &Anchors, p: f32, _t: f32) {
        let Some((head, n)) = self.parade_head(a, p) else { return };
        let av = &a.avenue;
        let seed = self.seed as u32;
        let on = |s: f32| s >= 0.0 && s < n as f32;
        // A figure `off` tiles to the side of the route at `s`, facing along it.
        let at = |s: f32, off: f32| -> (f32, f32, u8) {
            let (x, y) = path_at(av, s);
            let (dx, dy) = path_dir(av, s);
            (x - dy * off, y + dx * off, dir_code(dx, dy))
        };
        // The marching band: a drum major, then two ranks of three.
        if on(head) {
            let (x, y, d) = at(head, 0.0);
            v.push(Figure { x, y, pose: POSE_WAVE, tone: 14, child: false, prop: 0, dir: d, open: false, lift: 0, frame: 0 });
        }
        for (rank, behind) in [1.2f32, 2.1].iter().enumerate() {
            for (i, off) in [-0.26f32, 0.0, 0.26].iter().enumerate() {
                let s = head - behind;
                if on(s) {
                    let (x, y, d) = at(s, *off);
                    v.push(Figure { x, y, pose: if (i + rank) % 2 == 0 { POSE_PLAY } else { POSE_WORK }, tone: 14, child: false, prop: 0, dir: d, open: false, lift: 0, frame: 0 });
                }
            }
        }
        // Marchers in costume between the floats, and riders on the floats.
        for k in 0..4usize {
            let s = head - 6.0 - k as f32 * 4.5;
            for (i, off) in [-0.28f32, 0.28].iter().enumerate() {
                if on(s - 2.2) {
                    let (x, y, d) = at(s - 2.2, *off);
                    v.push(Figure { x, y, pose: POSE_WAVE, tone: (3 + k * 2 + i) as u8 % 8, child: i == 1, prop: 0, dir: d, open: false, lift: 0, frame: 0 });
                }
            }
            if on(s) {
                for (i, off) in [-0.14f32, 0.14].iter().enumerate() {
                    let (x, y, d) = at(s + *off, 0.0);
                    v.push(Figure { x, y, pose: POSE_WAVE, tone: (k + i) as u8 % 8, child: false, prop: 0, dir: d, open: false, lift: 3, frame: 0 });
                }
            }
        }
        // The crowd on both kerbs near the head (people step out as it nears and
        // drift back once it has passed).
        for i in 0..n {
            let d = head - i as f32;
            if !(-7.0..=14.0).contains(&d) {
                continue;
            }
            for (si, side) in [-1.0f32, 1.0].iter().enumerate() {
                let h = hs(seed, (i as u32) * 7 + si as u32 * 3);
                if h > 0.8 {
                    continue;
                }
                let (x, y, _) = at(i as f32, side * (0.95 + (h - 0.4) * 0.3));
                let (ix, iy) = (x.floor() as i32, y.floor() as i32);
                if !self.map.in_bounds(ix, iy)
                    || self.city_key[(iy * self.map.w + ix) as usize] != 0
                    || self.map.at(ix, iy).is_water()
                    || self.map.at(ix, iy).kind.is_lane()
                {
                    continue;
                }
                // Facing the route.
                let (dx, dy) = path_dir(av, i as f32);
                let face = dir_code(dy * side, -dx * side);
                v.push(Figure {
                    x,
                    y,
                    pose: if h < 0.4 { POSE_WAVE } else if h < 0.6 { POSE_CHAT } else { POSE_STAND },
                    tone: (i * 3 + (h * 8.0) as usize) as u8 % 8,
                    child: h > 0.7,
                    prop: 0,
                    dir: face,
                    open: false,
                    lift: 0,
                    frame: 0,
                });
            }
        }
    }

    fn crowd_side(&self, v: &mut Vec<Figure>, s: &SideNow) {
        match s.kind {
            SideKind::Kite => {
                let Some(zone) = self.spec.zones.get(s.zone) else { return };
                let Some((tx, ty, _, _)) = self.zone_ground(*zone, s.spot, s.seed, false) else { return };
                v.push(Figure { x: tx, y: ty, pose: POSE_WAVE, tone: 5, child: true, prop: 0, dir: 3, open: false, lift: 0, frame: 0 });
                v.push(Figure { x: tx + 0.5, y: ty - 0.2, pose: POSE_STAND, tone: 2, child: false, prop: 0, dir: 3, open: false, lift: 0, frame: 0 });
            }
            SideKind::Musician => {
                let Some(zone) = self.spec.zones.get(s.zone) else { return };
                let Some((tx, ty, _, _)) = self.zone_ground(*zone, s.spot, s.seed, false) else { return };
                v.push(Figure { x: tx, y: ty, pose: POSE_PLAY, tone: 6, child: false, prop: 0, dir: 1, open: false, lift: 0, frame: 0 });
                // Two listeners drift up as the evening goes on, then leave.
                let near = smooth(s.progress / 0.2) * smooth((1.0 - s.progress) / 0.2);
                if near > 0.3 {
                    v.push(Figure { x: tx + 0.7, y: ty + 0.55, pose: POSE_STAND, tone: 1, child: false, prop: 0, dir: 3, open: false, lift: 0, frame: 0 });
                }
                if near > 0.7 {
                    v.push(Figure { x: tx - 0.6, y: ty + 0.7, pose: POSE_CHAT, tone: 4, child: true, prop: 0, dir: 3, open: false, lift: 0, frame: 0 });
                }
            }
            _ => {}
        }
    }

    // ---- drawing -----------------------------------------------------------------

    /// Under the people: parade floats and the gameday lot.
    pub(crate) fn draw_spec_under(&self, dst: &mut Framebuffer, r: Rect) {
        let sp = &self.spec;
        let Some(a) = sp.anchors.as_ref() else { return };
        // The gameday lot.
        if self.holiday.holiday.is_bills() {
            if let Some((lx, ly, span, lvl)) = a.lot {
                art::tailgate_lot(dst, &self.cam, lx, ly, span, lvl, sp.day_secs, self.seed as u32);
            }
        }
        if let Some(p) = sp.events.parade {
            if let Some((head, n)) = self.parade_head(a, p) {
                let _ = n;
                for k in 0..4 {
                    let s = head - 5.0 - k as f32 * 4.0;
                    if s < 0.0 || s >= a.avenue.len() as f32 {
                        continue;
                    }
                    let (x, y) = path_at(&a.avenue, s);
                    let lvl = self.map.surface_level_at(x, y) as u8;
                    let (dx, dy) = path_dir(&a.avenue, s);
                    art::float(dst, &self.cam, x, y, lvl, k as u8, dx.abs() >= dy.abs());
                }
            }
        }
        let _ = r;
    }

    /// Over everything but the weather: the lantern glow, the clock's hands, the
    /// fireworks and their light, balloons, side-monitor creatures.
    pub(crate) fn draw_spec_over(&self, dst: &mut Framebuffer, r: Rect) {
        let sp = &self.spec;
        let bills_win = matches!(self.holiday.holiday, crate::holiday::Holiday::BillsWin);
        if sp.quiet() && !bills_win && sp.anchors.as_ref().map_or(true, |a| a.clock.is_none()) {
            return;
        }
        let Some(a) = sp.anchors.as_ref() else {
            // Anchors not computed yet (nothing has set events): the Bills win
            // show does not need them.
            return;
        };
        let tint = self.tint;
        let glow = self.glow;
        // The clock's hands.
        if let Some((x, y, l)) = a.clock {
            let b = Rect::new(0, 0, 0, 0);
            let _ = b;
            art::clock_hands(dst, &self.cam, x, y, l, sp.day_secs);
        }
        // Lanterns and the festival's stage lamps.
        if let Some(sq) = a.square {
            let dress = sp.events.dress();
            if dress.night_market {
                art::night_lights(dst, &self.cam, &sq, dress, glow > 0.3);
            }
        }
        // Balloons.
        for b in &sp.balloons {
            let Some(p) = self.balloon_pose(b) else { continue };
            let bb = Rect::new(p.x as i32 - 40, (p.base_y - 106.0 * b.size) as i32, p.x as i32 + 40, p.base_y as i32 + 10);
            let sh = Rect::new(p.gx as i32 - 40, p.gy as i32 - 24, p.gx as i32 + 40, p.gy as i32 + 24);
            if !(bb.intersects(&r) || sh.intersects(&r)) {
                continue;
            }
            // Its shadow, thrown the way the sun throws it (toward screen-right
            // at dawn).
            let strength = self.light.caster().map_or(0.0, |c| c.strength);
            let off = self.light.ground_offset(p.gy - p.base_y).map_or((0.0, 0.0), |(ox, oy)| (ox, oy));
            let sx = p.gx + (off.0 - off.1) * 32.0;
            let sy = p.gy + (off.0 + off.1) * 16.0;
            art::draw_balloon_shadow(dst, sx, sy, b.size, p.lift, strength);
            let sag = if b.progress > 0.965 { ((b.progress - 0.965) / 0.035).clamp(0.0, 1.0) } else { 0.0 };
            let burn = if p.climbing { 1.0 } else { ((b.progress * 9.0).sin().max(0.0) * 0.8).min(1.0) * 0.9 };
            art::draw_balloon(dst, p.x, p.base_y, b.size, b.scheme, burn, sag, sp.day_secs, tint);
        }
        // Side-monitor creatures and props.
        for s in &sp.side {
            match s.kind {
                SideKind::Fox => {
                    if let Some((x, y, face)) = self.fox_pose(s) {
                        let frame = ((s.progress * s.seconds * 9.0) as i32 & 1) as u8;
                        art::draw_fox(dst, x as i32, y as i32, face, frame, glow, tint);
                    }
                }
                SideKind::Kite => {
                    if let Some((kx, ky, hy)) = self.kite_pose(s) {
                        let Some(zone) = sp.zones.get(s.zone) else { continue };
                        if let Some((_, _, sx, _)) = self.zone_ground(*zone, s.spot, s.seed, false) {
                            dst.line((sx + 3.0, hy), (kx, ky + 6.0), palette::rgb(0xe8, 0xe4, 0xd8));
                        }
                        art::draw_kite(dst, kx, ky, s.progress * s.seconds, (s.seed % 6) as u8, tint);
                    }
                }
                SideKind::Musician => {
                    if let Some((x, y)) = self.musician_spot(s) {
                        art::draw_busker(dst, x as i32, y as i32, s.progress * s.seconds, tint);
                    }
                }
                SideKind::Balloon => {}
            }
        }
        // The fireworks, then the light they cast.
        if let (Some(t), Some(show)) = (sp.events.fireworks, a.show) {
            let night = glow;
            if night > 0.2 {
                let b = art::show_bounds(&show);
                if b.intersects(&r) {
                    let mut lights: Vec<Light> = Vec::with_capacity(12);
                    art::draw_show(dst, &show, t, sp.show_dur, self.seed as u32, &mut lights);
                    art::apply_lights(dst, &lights, ((night - 0.2) / 0.5).clamp(0.0, 1.0));
                }
            }
        }
        // B53: a Bills win has its own show over the gameday host.
        if matches!(self.holiday.holiday, crate::holiday::Holiday::BillsWin) && glow > 0.3 {
            if let Some(show) = self.gameday_show() {
                let cycle = self.holiday_t.rem_euclid(3600.0);
                if cycle < 900.0 && art::show_bounds(&show).intersects(&r) {
                    let mut lights: Vec<Light> = Vec::with_capacity(12);
                    art::draw_show(dst, &show, cycle, 900.0, self.seed as u32 ^ 0xB11, &mut lights);
                    art::apply_lights(dst, &lights, ((glow - 0.3) / 0.5).clamp(0.0, 1.0));
                }
            }
        }
    }

    /// The Bills win show's site: over the stadium (or the biggest park).
    pub(crate) fn gameday_show(&self) -> Option<Show> {
        let (cx, cy, span, _) = self.gameday_host_screen()?;
        let sky = self.sky_h as f32;
        Some(Show { cx, ground_y: cy + span * 0.25, span: (span * 1.6).clamp(220.0, 420.0), water: false, burst_top: sky * 0.16, burst_bot: sky * 0.82 })
    }

    /// The rainbow, painted into the sky band.
    pub(crate) fn draw_rainbow(&self, dst: &mut Framebuffer) {
        let k = self.spec.rainbow;
        if k < 0.01 {
            return;
        }
        let sky = self.sky_h as f32;
        let rr = sky * 1.0;
        let cx = self.width() as f32 * self.spec.rainbow_cx;
        art::draw_rainbow(dst, cx, sky + rr * 0.25, rr, k, self.sky_h as i32);
    }

    /// The lightning bolt of the current strike.
    pub(crate) fn draw_bolt(&self, dst: &mut Framebuffer) {
        let Some((seed, k)) = self.spec.bolt else { return };
        let (w, sky) = (self.width() as f32, self.sky_h as f32);
        let x = w * (0.12 + 0.76 * hs(seed, 1));
        let ground = sky + (self.height() as f32 - sky) * (0.1 + 0.45 * hs(seed, 2));
        let y1 = if self.spec.storm_special { ground } else { sky + 6.0 };
        art::draw_bolt(dst, x, sky * 0.1, y1, seed, self.spec.storm_special, k);
    }

    /// Watch the weather's flash for a new strike (called from `set_ambient`).
    pub(crate) fn spec_flash(&mut self, flash: f32) {
        let prev = self.spec.last_flash;
        self.spec.last_flash = flash;
        if flash > 0.12 && prev <= 0.12 {
            self.spec.bolt_count += 1;
            self.spec.bolt = Some((self.spec.bolt_count.wrapping_mul(0x9E37_79B1) ^ self.seed as u32, (flash / 0.5).clamp(0.2, 1.0)));
        } else if flash > 0.03 {
            if let Some((s, _)) = self.spec.bolt {
                self.spec.bolt = Some((s, (flash / 0.5).clamp(0.1, 1.0)));
            }
        } else {
            self.spec.bolt = None;
        }
    }

    /// Per-tick work that needs `&mut self`: resolve the balloons' paths.
    pub fn spec_tick(&mut self) {
        if !self.spec.balloons.is_empty() {
            self.balloon_cache();
        }
    }

    /// Show a bolt now (stills; a running wallpaper takes its strikes from the
    /// weather's flash).
    pub fn force_bolt(&mut self) {
        self.spec.bolt = Some((0xB017_5EED ^ self.seed as u32, 1.0));
        self.mark_full();
    }

    /// Time of day the scene was last told, in seconds (tests).
    pub fn spec_day_secs(&self) -> f32 {
        self.spec.day_secs
    }

    /// How many extra figures (crowd, band, performers) are on (tests).
    pub fn spec_figures(&self) -> usize {
        self.spec.extra.len()
    }

    /// Is any spectacle running right now (so the wallpaper must keep stepping
    /// it every frame)?
    pub fn spec_live(&self) -> bool {
        !self.spec.quiet()
    }

    /// Is the sky's lightning bolt showing (tests).
    pub fn bolt_showing(&self) -> bool {
        self.spec.bolt.is_some()
    }
}

struct Pose {
    x: f32,
    gx: f32,
    gy: f32,
    base_y: f32,
    lift: f32,
    climbing: bool,
}

fn world_balloon_lift(p: f32) -> f32 {
    let up = (p / 0.16).clamp(0.0, 1.0);
    let down = ((1.0 - p) / 0.24).clamp(0.0, 1.0);
    smooth(up) * smooth(down)
}

/// A point `s` tiles along a polyline of tile centres (clamped to its ends).
fn path_at(path: &[(f32, f32)], s: f32) -> (f32, f32) {
    if path.is_empty() {
        return (0.0, 0.0);
    }
    let s = s.clamp(0.0, (path.len() - 1) as f32);
    let i = s.floor() as usize;
    let f = s - i as f32;
    let a = path[i];
    let b = path[(i + 1).min(path.len() - 1)];
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
}

/// Unit direction of travel along a polyline at `s`.
fn path_dir(path: &[(f32, f32)], s: f32) -> (f32, f32) {
    let a = path_at(path, s - 0.8);
    let b = path_at(path, s + 0.8);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let n = (dx * dx + dy * dy).sqrt();
    if n < 1e-4 { (1.0, 0.0) } else { (dx / n, dy / n) }
}

/// A facing code (0 = +x, 1 = +y, 2 = -x, 3 = -y) for a direction.
fn dir_code(dx: f32, dy: f32) -> u8 {
    if dx.abs() >= dy.abs() {
        if dx >= 0.0 { 0 } else { 2 }
    } else if dy >= 0.0 {
        1
    } else {
        3
    }
}

/// How full the square is: nobody at the start, the peak mid afternoon.
pub fn attendance(p: f32) -> f32 {
    let x = (p.clamp(0.0, 1.0) * std::f32::consts::PI).sin();
    x.powf(0.8)
}

fn fig(x: f32, y: f32, pose: u8, tone: u8, child: bool, dir: u8) -> Figure {
    Figure { x, y, pose, tone, child, prop: 0, dir, open: true, lift: 0, frame: 0 }
}

fn crowd_festival(v: &mut Vec<Figure>, sq: &Square, att: f32, seed: u32) {
    let (sx0, sy0, sx1, sy1) = sq.stage_rect();
    let cx = (sx0 + sx1) * 0.5;
    // The band, on the deck (8 px up = lift 4).
    let mut up = |x: f32, y: f32, pose: u8, tone: u8| {
        let mut f = fig(x, y, pose, tone, false, 1);
        f.lift = 4;
        v.push(f);
    };
    up(cx, sy1 - 0.28, POSE_WAVE, 3);
    up(cx - 0.75, sy0 + 0.55, POSE_PLAY, 5);
    up(cx + 0.38, sy0 + 0.42, POSE_WORK, 2);
    up(cx + 0.82, sy0 + 0.58, POSE_PLAY, 1);
    // The audience: jittered rows in front of the stage, thicker near it.
    let (fx, fy) = (sq.x + sq.w * 0.5, sq.y + sq.h * 0.5);
    let max = ((sq.w * sq.h) * 3.2) as usize;
    let want = (max as f32 * att) as usize;
    let mut placed = 0;
    let mut i = 0u32;
    while placed < want && i < 600 {
        i += 1;
        let h1 = hs(seed, i * 3);
        let h2 = hs(seed, i * 3 + 1);
        // Most of the audience stands in front of the stage; the rest mills about.
        let (x, y) = if hs(seed, i * 3 + 2) < 0.62 {
            (sx0 - 0.2 + h1 * (sx1 - sx0 + 0.4), sy1 + 0.4 + h2 * 1.9)
        } else {
            (sq.x + 0.45 + h1 * (sq.w - 0.9), sy1 + 0.45 + h2 * (sq.y + sq.h - 0.4 - (sy1 + 0.45)))
        };
        if y > sq.y + sq.h - 0.35 {
            continue;
        }
        // Not on the fountain, not in the stalls' rows.
        if ((x - fx).powi(2) + (y - fy).powi(2)).sqrt() < 1.1 {
            continue;
        }
        if x < sq.x + 1.1 && y > sq.y + 1.4 || x > sq.x + sq.w - 1.1 && y > sq.y + 1.4 {
            continue;
        }
        let near_stage = y < sy1 + 1.6;
        let pose = if near_stage {
            if hs(seed, i) < 0.55 { POSE_WAVE } else { POSE_PLAY }
        } else if hs(seed, i) < 0.4 {
            POSE_CHAT
        } else {
            POSE_STAND
        };
        v.push(fig(x, y, pose, (placed * 5 + 1) as u8 % 8, hs(seed, i + 91) < 0.15, 3));
        placed += 1;
    }
    // A vendor's customer at each stall.
    for (k, &(x, y)) in sq.stall_spots(false).iter().enumerate() {
        if hs(seed, k as u32 + 700) < att + 0.1 {
            let toward = if x < sq.x + sq.w * 0.5 { 0.62 } else { -0.62 };
            v.push(fig(x + toward, y + 0.15, POSE_CHAT, (k + 2) as u8 % 8, false, if toward > 0.0 { 2 } else { 0 }));
        }
    }
}

fn crowd_night_market(v: &mut Vec<Figure>, sq: &Square, with_festival: bool, t: f32, seed: u32) {
    let ((ax, ay), (bx, by)) = sq.aisle();
    let n = 22;
    for i in 0..n {
        let h = hs(seed, 4000 + i);
        // Strollers walk the aisle back and forth, slowly (a tile in 9 s).
        let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
        let speed = 0.09 + h * 0.05;
        let ph = (t * speed / len + h * 7.0).rem_euclid(2.0);
        let (f, fwd) = if ph < 1.0 { (ph, true) } else { (2.0 - ph, false) };
        let off = (hs(seed, 4100 + i) - 0.5) * (sq.w - 3.2).max(0.6);
        let (x, y) = (ax + (bx - ax) * f + off, ay + (by - ay) * f);
        if with_festival {
            continue;
        }
        v.push(Figure { x, y, pose: POSE_WALK, tone: (i as usize * 3) as u8 % 8, child: h > 0.86, prop: 0, dir: if fwd { 1 } else { 3 }, open: true, lift: 0, frame: 0 });
    }
    for (k, &(x, y)) in sq.stall_spots(!with_festival).iter().enumerate() {
        if hs(seed, k as u32 + 5000) < 0.8 && !with_festival {
            let toward = if x < sq.x + sq.w * 0.5 { 0.62 } else { -0.62 };
            v.push(fig(x + toward, y + 0.12, POSE_CHAT, (k + 4) as u8 % 8, false, if toward > 0.0 { 2 } else { 0 }));
        }
    }
}

fn crowd_tailgate(v: &mut Vec<Figure>, lx: f32, ly: f32, span: f32, win: bool, seed: u32) {
    let n = ((span / 1.5) as i32).clamp(2, 5);
    for i in 0..n {
        let f = (i as f32 + 0.5) / n as f32;
        let tx = lx - span * 0.5 + span * f;
        for k in 0..4u32 {
            let h = hs(seed, 6000 + i as u32 * 10 + k);
            let jersey = if (i as u32 + k + seed) % 2 == 0 { 12 } else { 13 };
            let pose = if win { POSE_WAVE } else if k == 0 { POSE_WORK } else if k == 3 { POSE_PLAY } else { POSE_CHAT };
            let (dx, dy) = [(0.7f32, 0.3f32), (-0.1, 0.55), (0.35, 0.7), (0.95, 0.65)][k as usize];
            v.push(Figure { x: tx + dx, y: ly + dy + (h - 0.5) * 0.1, pose, tone: jersey, child: k == 3, prop: 0, dir: 1, open: false, lift: 0, frame: 0 });
        }
    }
    if win {
        // A cheering crowd in the plot's open ground.
        for k in 0..16u32 {
            let h = hs(seed, 6500 + k);
            let x = lx - span * 0.5 + h * span;
            v.push(Figure { x, y: ly + 1.1 + hs(seed, 6600 + k) * 0.8, pose: POSE_WAVE, tone: if k % 2 == 0 { 12 } else { 13 }, child: false, prop: 0, dir: 1, open: false, lift: 0, frame: 0 });
        }
    }
}

fn crowd_zoo(v: &mut Vec<Figure>, zx: f32, zy: f32, zw: f32, zh: f32, seed: u32) {
    let (cx, cy) = (zx + zw * 0.5, zy + zh * 0.5);
    for i in 0..9u32 {
        let h = hs(seed, 7000 + i);
        let along_x = i % 2 == 0;
        let o = (hs(seed, 7100 + i) - 0.5) * (if along_x { zw } else { zh }) * 0.8;
        let (x, y) = if along_x { (cx + o, cy + 0.0) } else { (cx, cy + o) };
        let pose = if h < 0.5 { POSE_STAND } else if h < 0.8 { POSE_CHAT } else { POSE_WAVE };
        v.push(Figure { x, y, pose, tone: (i * 2 + 1) as u8 % 8, child: h > 0.75, prop: 0, dir: (i % 4) as u8, open: true, lift: 0, frame: 0 });
    }
}

#[cfg(test)]
mod tests;
