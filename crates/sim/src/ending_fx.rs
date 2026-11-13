//! The ending's effects on the world (B50): land edits, ruins, people.
//!
//! Every effect is a pure function of the ending's plan and its progress `u`
//! (see [`crate::ending`]); tile edits are ordered lists with a cursor, so they
//! are incremental, resumable from a save and identical live or in catch-up.

use std::collections::HashMap;

use map::{Kind, Prop, Tile};

use crate::ending::*;
use crate::{Lot, LotKind, Sim};

fn ramp(x: f32, a: f32, b: f32) -> f32 {
    if b <= a {
        return if x >= b { 1.0 } else { 0.0 };
    }
    ((x - a) / (b - a)).clamp(0.0, 1.0)
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn lot_centre(l: &Lot) -> (f32, f32) {
    (l.x as f32 + l.w as f32 * 0.5, l.y as f32 + l.h as f32 * 0.5)
}

const METEOR_R: f32 = 11.0;
const MOON_R: f32 = 17.0;
const CONE_R: f32 = 10.0;
const CONE_PEAK: f32 = 8.0;

impl Sim {
    /// The main effect's progress, 0..1, for the renderer.
    pub(crate) fn main_frac(&self, _ph: Phase, _p: f32, u: f32) -> f32 {
        match self.ending.kind {
            Ending::Meteor => ramp(u, 1.36, 1.95),
            Ending::Volcano => ramp(u, 1.08, 1.85),
            Ending::Flood => ramp(u, 1.0, 1.55),
            Ending::Kaiju => ramp(u, 0.95, 1.98),
            Ending::Aliens => ramp(u, 1.0, 2.0),
            Ending::AiTakeover => ramp(u, 1.0, 1.92),
            Ending::RedGiant => ramp(u, 0.0, 1.6),
            Ending::IceAge => ramp(u, 0.0, 1.6),
            Ending::FallingMoon => ramp(u, 0.0, 1.7),
            Ending::Decline => ramp(u, 0.2, 2.6),
            Ending::Smog => ramp(u, 0.0, 1.5),
            Ending::Sunset => ramp(u, 0.0, 2.0),
        }
    }

    // -----------------------------------------------------------------------
    // The ordered tile lists
    // -----------------------------------------------------------------------

    fn has_list(kind: Ending, slot: usize) -> bool {
        matches!(
            (kind, slot),
            (Ending::Meteor, 0)
                | (Ending::Meteor, 1)
                | (Ending::FallingMoon, 0)
                | (Ending::Volcano, 0)
                | (Ending::Volcano, 1)
                | (Ending::Flood, 0)
                | (Ending::Flood, 1)
                | (Ending::Kaiju, 0)
                | (Ending::Aliens, 0)
                | (Ending::AiTakeover, 0)
                | (Ending::RedGiant, 0)
                | (Ending::IceAge, 0)
                | (Ending::Decline, 0)
                | (_, 3)
        )
    }

    pub(crate) fn ensure_list(&mut self, slot: usize) {
        if self.ending.lists[slot].is_some() || !Self::has_list(self.ending.kind, slot) {
            return;
        }
        let mut v = self.build_list(slot);
        v.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        // The ruins want to ask "is this tile in the edit, and when?".
        if matches!((self.ending.kind, slot), (Ending::Volcano, 1) | (Ending::Flood, 1)) {
            let m: HashMap<(i32, i32), f32> = v.iter().map(|e| ((e.0, e.1), e.2)).collect();
            self.ending.lookup = Some(m);
        }
        self.ending.overlay_epoch += 1;
        self.ending.lists[slot] = Some(v);
    }

    fn build_list(&self, slot: usize) -> Vec<(i32, i32, f32)> {
        let st = &self.ending;
        let (sx, sy) = st.site;
        let (w, h) = (self.map.w, self.map.h);
        let seed = self.seed;
        let mut v: Vec<(i32, i32, f32)> = Vec::new();
        if slot == 3 {
            // The morph: a ragged ring growing out from the site.
            let maxd = dist((0.0, 0.0), (w as f32, h as f32)).max(1.0);
            let mut keyed: Vec<(i32, i32, f32)> = Vec::with_capacity((w * h) as usize);
            for y in 0..h {
                for x in 0..w {
                    let d = dist((x as f32 + 0.5, y as f32 + 0.5), (sx, sy)) / maxd;
                    let n = map::rng::fbm(x as f32 / 4.0, y as f32 / 4.0, seed ^ 0x40E9, 2);
                    keyed.push((x, y, d * 0.75 + n * 0.25 + h01(seed ^ 0x77, x, y) * 0.03));
                }
            }
            keyed.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
            let n = keyed.len() as f32;
            for (i, e) in keyed.into_iter().enumerate() {
                v.push((e.0, e.1, i as f32 / n));
            }
            return v;
        }
        match (st.kind, slot) {
            (Ending::Meteor, 0) | (Ending::FallingMoon, 0) => {
                let r = if st.kind == Ending::Meteor { METEOR_R } else { MOON_R };
                for y in 0..h {
                    for x in 0..w {
                        let d = dist((x as f32 + 0.5, y as f32 + 0.5), (sx, sy));
                        if d <= r {
                            v.push((x, y, d / r));
                        }
                    }
                }
            }
            (Ending::Meteor, 1) => {
                for y in 0..h {
                    for x in 0..w {
                        let d = dist((x as f32 + 0.5, y as f32 + 0.5), (sx, sy));
                        if d < 3.2 {
                            v.push((x, y, d / 3.2));
                        }
                    }
                }
            }
            (Ending::Volcano, 0) => {
                for y in 0..h {
                    for x in 0..w {
                        let d = dist((x as f32 + 0.5, y as f32 + 0.5), (sx, sy));
                        if d <= CONE_R {
                            v.push((x, y, 1.0 - d / CONE_R));
                        }
                    }
                }
            }
            (Ending::Volcano, 1) => {
                // The direction and distance to the town were fixed when the
                // site was chosen (aux 3..5): the list never depends on what
                // has been built or destroyed since.
                let dir = (st.aux[3], st.aux[4]);
                let massifs: Vec<map::massif::Massif> = self.map.landforms.clone();
                let dl = st.aux[5].max(1.0);
                let lmax = (dl * 0.86).clamp(10.0, 24.0);
                let mut best: HashMap<(i32, i32), f32> = HashMap::new();
                for (k, ang) in [-0.42f32, 0.0, 0.38].iter().enumerate() {
                    let (ca, sa) = (ang.cos(), ang.sin());
                    let d = (dir.0 * ca - dir.1 * sa, dir.0 * sa + dir.1 * ca);
                    let perp = (-d.1, d.0);
                    let len = lmax * (0.8 + 0.2 * h01(seed, k as i32, 7));
                    let mut s = 0.0f32;
                    while s <= len {
                        let wob = (s * 0.55 + k as f32 * 2.1).sin() * 1.3 + (s * 0.21).sin() * 1.1;
                        let cx = sx + d.0 * s + perp.0 * wob;
                        let cy = sy + d.1 * s + perp.1 * wob;
                        let width = 1.0 + 0.9 * (s * 0.4 + k as f32).sin().abs() + 1.4 * (s / lmax);
                        let r = width.ceil() as i32 + 1;
                        for dy in -r..=r {
                            for dx in -r..=r {
                                let (tx, ty) = (cx as i32 + dx, cy as i32 + dy);
                                if !self.map.in_bounds(tx, ty) {
                                    continue;
                                }
                                // No lava under another massif's sprite.
                                if massifs.iter().any(|m| {
                                    dist((m.cx, m.cy), (sx, sy)) > 0.5
                                        && dist((tx as f32 + 0.5, ty as f32 + 0.5), (m.cx, m.cy)) < m.radius + 1.0
                                }) {
                                    continue;
                                }
                                if dist((tx as f32 + 0.5, ty as f32 + 0.5), (cx, cy)) <= width {
                                    let thr = (s / lmax) * 0.92 + h01(seed ^ 0x1A, tx, ty) * 0.08;
                                    let e = best.entry((tx, ty)).or_insert(thr);
                                    if thr < *e {
                                        *e = thr;
                                    }
                                }
                            }
                        }
                        s += 0.5;
                    }
                }
                // The vent itself.
                for y in 0..h {
                    for x in 0..w {
                        let d = dist((x as f32 + 0.5, y as f32 + 0.5), (sx, sy));
                        if d < 2.3 {
                            best.insert((x, y), (d / 2.3) * 0.05);
                        }
                    }
                }
                v = best.into_iter().map(|((x, y), t)| (x, y, t)).collect();
            }
            (Ending::Flood, slot) => {
                let Some(o) = self.map.ocean.clone() else { return v };
                let dl = (st.aux[2] + 2.5).clamp(10.0, 70.0);
                for y in 1..h - 1 {
                    for x in 1..w - 1 {
                        let s = self.inland(&o, x as f32 + 0.5, y as f32 + 0.5);
                        if slot == 0 {
                            // The sea draws back first: a band of wet sand where
                            // the water stood.
                            if (-3.2..=0.2).contains(&s) {
                                v.push((x, y, (s + 3.2) / 3.4));
                            }
                        } else {
                            // A ragged shore, not a ruled line: the reach varies
                            // with a slow noise, so the lagoon has bays and capes.
                            let n = map::rng::fbm(x as f32 / 6.0, y as f32 / 6.0, seed ^ 0xF7, 3);
                            let reach = dl * (0.84 + 0.32 * n);
                            // B99c round 2: the open sea is the same water as the
                            // lagoon over the lower town, so it is in the same
                            // list and takes the same look. Without it a still
                            // black sea sat beside a lit lagoon in one frame, and
                            // the black half read as a hole.
                            if s <= reach {
                                // The order the water arrives in is its distance
                                // from the sea and **nothing else**. Keying it on
                                // each tile's own noisy reach left dry holes
                                // scattered through the middle of the lagoon, so
                                // the flood came out as a speckle instead of a
                                // body of water (B99c).
                                let t = ((s + 3.2) / (dl + 7.2)).clamp(0.0, 1.0) * 0.97
                                    + h01(seed ^ 0xF1, x, y) * 0.03;
                                v.push((x, y, t));
                            }
                        }
                    }
                }
            }
            (Ending::Kaiju, 0) => {
                let a = (st.aux[0], st.aux[1]);
                let b = (st.aux[2], st.aux[3]);
                for y in 0..h {
                    for x in 0..w {
                        let (d, t) = seg_dist((x as f32 + 0.5, y as f32 + 0.5), a, b);
                        if d <= 1.15 {
                            v.push((x, y, t * 0.9 + h01(seed ^ 0x33, x, y) * 0.1));
                        }
                    }
                }
            }
            (Ending::Aliens, 0) => {
                let n = st.aux[2].max(3.0) as usize;
                let rad = st.aux[1];
                for i in 0..n {
                    let a = ginger(seed, i as u32) * std::f32::consts::TAU;
                    let ring = rad * (0.15 + 0.5 * ginger(seed, 40 + i as u32));
                    let p = (sx + a.cos() * ring, sy + a.sin() * ring);
                    for dy in -2..=2 {
                        for dx in -2..=2 {
                            let (tx, ty) = (p.0 as i32 + dx, p.1 as i32 + dy);
                            if !self.map.in_bounds(tx, ty) {
                                continue;
                            }
                            let d = dist((tx as f32 + 0.5, ty as f32 + 0.5), p);
                            if d <= 1.7 {
                                v.push((tx, ty, (i as f32 + d / 1.7) / n as f32));
                            }
                        }
                    }
                }
                v.sort_by_key(|e| (e.0, e.1));
                v.dedup_by_key(|e| (e.0, e.1));
            }
            (Ending::AiTakeover, 0) => {
                let far = st.aux[2].max(8.0);
                for y in 0..h {
                    for x in 0..w {
                        let t = self.map.at(x, y);
                        if t.kind.is_lane() || t.kind == Kind::Sidewalk || t.kind == Kind::Paved {
                            let d = dist((x as f32 + 0.5, y as f32 + 0.5), (sx, sy));
                            v.push((x, y, (d / far).min(1.0)));
                        }
                    }
                }
            }
            (Ending::RedGiant, 0) => {
                // Position only: the list must not depend on what has been
                // built since, or a save would reorder it.
                for y in 0..h {
                    for x in 0..w {
                        v.push((x, y, h01(seed ^ 0x5E, x, y)));
                    }
                }
            }
            (Ending::IceAge, 0) => {
                for y in 0..h {
                    for x in 0..w {
                        if self.map.at(x, y).is_water() {
                            v.push((x, y, 0.15 + 0.7 * h01(seed ^ 0x1C, x, y)));
                        }
                    }
                }
            }
            (Ending::Decline, 0) => {
                for y in 0..h {
                    for x in 0..w {
                        v.push((x, y, h01(seed ^ 0xDE, x, y)));
                    }
                }
            }
            _ => {}
        }
        v
    }

    /// Apply a slot's edits up to `frac` (0..1).
    fn run_edit(&mut self, slot: usize, frac: f32) {
        self.ensure_list(slot);
        let Some(list) = self.ending.lists[slot].take() else { return };
        let n = list.partition_point(|e| e.2 <= frac);
        let mut cur = self.ending.cur[slot] as usize;
        let kind = self.ending.kind;
        while cur < n.min(list.len()) {
            let (x, y, thr) = list[cur];
            self.apply_edit(kind, slot, x, y, thr);
            cur += 1;
        }
        self.ending.cur[slot] = cur as u32;
        self.ending.lists[slot] = Some(list);
    }

    pub(crate) fn land_edits(&mut self, ph: Phase, p: f32, u: f32) {
        match self.ending.kind {
            Ending::Meteor => {
                self.run_edit(0, ramp(u, 1.36, 1.5));
                self.run_edit(1, ramp(u, 2.05, 2.8));
            }
            Ending::FallingMoon => self.run_edit(0, ramp(u, 1.7, 1.86)),
            Ending::Volcano => {
                self.run_edit(0, ramp(u, 0.15, 0.95));
                self.run_edit(1, ramp(u, 1.08, 1.85));
            }
            Ending::Flood => {
                self.run_edit(0, ramp(u, 0.5, 0.95));
                self.run_edit(1, ramp(u, 1.0, 1.55));
            }
            Ending::Kaiju => self.run_edit(0, ramp(u, 0.95, 1.98)),
            Ending::Aliens => self.run_edit(0, ramp(u, 1.3, 1.8)),
            Ending::RedGiant => self.run_edit(0, ramp(u, 0.5, 2.2)),
            Ending::Decline => self.run_edit(0, ramp(u, 0.4, 2.7)),
            _ => {}
        }
        // The morph: the land turns into the next one.
        if ph >= Phase::Morph {
            if self.ending.next_map.is_none() {
                let m = map::generate_with(
                    self.ending.next_seed,
                    self.map.w,
                    self.map.h,
                    Some(self.ending.next_biome),
                )
                .ok();
                self.ending.next_map = m.map(Box::new);
                self.restore_morph_mask();
            }
            if self.ending.next_map.is_some() {
                let frac = if ph == Phase::Morph { p } else { 1.0 };
                self.run_edit(3, frac);
            }
        }
    }

    /// After a load in the middle of a morph: the next land is the
    /// neighbour again, and the tiles already turned are marked.
    fn restore_morph_mask(&mut self) {
        let next = self.ending.next_biome;
        self.map.begin_morph(next);
        self.ending.pending_flips.clear();
        self.ending.neighbour_told = false;
        self.ensure_list(3);
        let cur = self.ending.cur[3] as usize;
        if let Some(list) = self.ending.lists[3].as_ref() {
            let flips: Vec<(i32, i32)> = list.iter().take(cur).map(|e| (e.0, e.1)).collect();
            for (x, y) in &flips {
                self.map.morph_tile(*x, *y);
            }
            self.ending.pending_flips.extend(flips);
        }
        // Water on turned tiles takes the next land's classes.
        if let Some(nm) = self.ending.next_map.as_ref() {
            let w = self.map.w;
            let cls: Vec<(i32, i32, u8)> = self
                .ending
                .pending_flips
                .iter()
                .map(|&(x, y)| (x, y, nm.water_class[(y * w + x) as usize]))
                .collect();
            for (x, y, c) in &cls {
                self.map.water_class[(*y * w + *x) as usize] = *c;
            }
            self.ending.pending_classes.extend(cls);
        }
    }

    /// Put a tile down, with the prop it should keep (a save keeps props only
    /// through [`State::props`]) and its water class.
    fn put_tile(&mut self, x: i32, y: i32, tile: Tile, class: Option<u8>) {
        if !self.map.in_bounds(x, y) {
            return;
        }
        self.write(x, y, tile);
        if tile.prop.is_something() {
            self.ending.props.insert((x, y), tile.prop);
        } else {
            self.ending.props.remove(&(x, y));
        }
        if let Some(c) = class {
            let i = (y * self.map.w + x) as usize;
            if self.map.water_class[i] != c {
                self.map.water_class[i] = c;
                self.ending.pending_classes.push((x, y, c));
            }
        }
    }

    fn apply_edit(&mut self, kind: Ending, slot: usize, x: i32, y: i32, thr: f32) {
        let seed = self.seed;
        let t = self.map.at(x, y);
        let lane = t.kind.is_lane() || t.kind == Kind::Bridge;
        let flat = |lvl: u8| [lvl; 4];
        if slot == 3 {
            let Some(nm) = self.ending.next_map.as_ref() else { return };
            let nt = nm.at(x, y);
            let class = nm.water_class[(y * self.map.w + x) as usize];
            self.put_tile(x, y, nt, Some(class));
            self.map.morph_tile(x, y);
            self.map.wear[(y * self.map.w + x) as usize] = 0;
            self.ending.pending_flips.push((x, y));
            self.fields.retain(|f| !(x >= f.x && x < f.x + f.w && y >= f.y && y < f.y + f.h));
            return;
        }
        match (kind, slot) {
            (Ending::Meteor, 0) | (Ending::FallingMoon, 0) => {
                let r = if kind == Ending::Meteor { METEOR_R } else { MOON_R };
                let sc = r / METEOR_R;
                let d = thr * r;
                let dd = d / sc;
                let l0 = self.ending.aux[0] as u8;
                let fl = l0.saturating_sub(1);
                if dd < 5.2 {
                    // A bowl, then a raised rim of thrown earth, then the old
                    // ground: stepped, so it reads as a crater from across the
                    // town even where the land is nearly flat.
                    let lvl = if dd < 2.8 {
                        fl
                    } else if dd < 3.6 {
                        fl + 1
                    } else if dd < 4.4 {
                        fl + 3
                    } else {
                        fl + 2
                    };
                    let lvl = lvl.min(map::MAX_LEVEL);
                    let k = if h01(seed ^ 0xC1, x, y) < 0.28 { Kind::Scree } else { Kind::Rock };
                    self.put_tile(
                        x,
                        y,
                        Tile { corner: flat(lvl), kind: k, prop: Prop::None, aux: 0 },
                        Some(0),
                    );
                } else if !t.is_water() {
                    let burn = (1.0 - dd / METEOR_R).max(0.0);
                    let mut nt = t;
                    if !lane && h01(seed ^ 0xC2, x, y) < burn * 1.3 {
                        nt.kind = if t.kind.is_worked() {
                            t.kind
                        } else if h01(seed ^ 0xC5, x, y) < 0.35 {
                            Kind::Scree
                        } else {
                            Kind::DryGrass
                        };
                    }
                    if h01(seed ^ 0xC3, x, y) < burn * 0.8 + 0.1 {
                        nt.prop = Prop::None;
                    }
                    if nt != t {
                        self.put_tile(x, y, nt, None);
                    }
                }
            }
            (Ending::Meteor, 1) => {
                let l0 = self.ending.aux[0] as u8;
                let fl = l0.saturating_sub(1);
                self.put_tile(
                    x,
                    y,
                    Tile { corner: flat(fl), kind: Kind::Water, prop: Prop::None, aux: 0 },
                    Some(2),
                );
            }
            (Ending::Volcano, 0) => {
                // A landform massif is the mountain already: nothing is raised.
                if self.ending.aux[6] > 0.5 || t.is_water() || lane {
                    return;
                }
                if self.map.landforms.iter().any(|m| dist((m.cx, m.cy), (x as f32 + 0.5, y as f32 + 0.5)) < m.radius + 1.0) {
                    return;
                }
                // A smooth cone: every corner rises with its own distance
                // from the vent, so the slope is a run of one-level ramps
                // rather than a stair. Outer tiles go first (the ground swells
                // outward in).
                let d = (1.0 - thr) * CONE_R;
                let (sx, sy) = self.ending.site;
                let bump = |px: f32, py: f32| -> u8 {
                    let dd = dist((px, py), (sx, sy));
                    if dd >= CONE_R {
                        return 0;
                    }
                    let mut f = (1.0 - dd / CONE_R).powf(1.5);
                    if dd < 1.4 {
                        f = (1.0 - 1.4 / CONE_R).powf(1.5) * 0.86; // the crater's dip
                    }
                    (CONE_PEAK * f).round() as u8
                };
                let (fx, fy) = (x as f32, y as f32);
                let adds = [bump(fx, fy), bump(fx + 1.0, fy), bump(fx + 1.0, fy + 1.0), bump(fx, fy + 1.0)];
                let mut corner = t.corner;
                for i in 0..4 {
                    corner[i] = (corner[i] + adds[i]).min(map::MAX_LEVEL);
                }
                let k = if d < CONE_R * 0.5 {
                    Kind::Rock
                } else if d < CONE_R * 0.8 || h01(seed ^ 0xC4, x, y) < 0.35 {
                    Kind::Scree
                } else {
                    t.kind
                };
                let prop = if d < CONE_R * 0.8 { Prop::None } else { t.prop };
                self.put_tile(x, y, Tile { corner, kind: k, prop, aux: 0 }, None);
            }
            (Ending::Volcano, 1) => {
                if t.is_water() {
                    return;
                }
                let mut nt = t;
                nt.kind = Kind::Rock;
                nt.prop = Prop::None;
                self.put_tile(x, y, nt, Some(0));
            }
            (Ending::Flood, 0) => {
                if t.is_water() {
                    let lvl = t.corner[0];
                    self.put_tile(
                        x,
                        y,
                        Tile { corner: flat(lvl), kind: Kind::Sand, prop: Prop::None, aux: 0 },
                        Some(0),
                    );
                }
            }
            (Ending::Flood, 1) => {
                let Some(o) = self.map.ocean.clone() else { return };
                if t.max_level() > o.level.saturating_add(14) && !t.is_water() {
                    return;
                }
                // B99c round 2: one class for the whole body — the lagoon over the
                // lower town and the open sea it opens into — so the surface is
                // lit and animated as one thing, not half bright and half black.
                let _ = self.inland(&o, x as f32 + 0.5, y as f32 + 0.5);
                self.put_tile(
                    x,
                    y,
                    Tile { corner: flat(o.level), kind: Kind::Water, prop: Prop::None, aux: 0 },
                    Some(map::WaterKind::Lagoon.code()),
                );
            }
            (Ending::Kaiju, 0) => {
                if lane || t.is_water() || t.kind.is_worked() {
                    return;
                }
                let mut nt = t;
                nt.kind = if h01(seed ^ 0xA2, x, y) < 0.3 { Kind::Scree } else { Kind::DryGrass };
                nt.prop = Prop::None;
                self.put_tile(x, y, nt, None);
            }
            (Ending::Aliens, 0) => {
                if lane || t.is_water() || t.kind.is_worked() {
                    return;
                }
                let mut nt = t;
                nt.kind = if h01(seed ^ 0xA1, x, y) < 0.5 { Kind::DryGrass } else { Kind::Scree };
                nt.prop = Prop::None;
                self.put_tile(x, y, nt, None);
            }
            (Ending::RedGiant, 0) => {
                let mut nt = t;
                if thr < 0.5 {
                    if matches!(t.kind, Kind::Grass | Kind::Meadow | Kind::Upland) {
                        nt.kind = Kind::DryGrass;
                    }
                } else if matches!(t.kind, Kind::Grass | Kind::Meadow | Kind::Upland | Kind::DryGrass) {
                    nt.kind = Kind::Sand;
                    nt.prop = Prop::None;
                } else if t.prop.is_tree() {
                    nt.prop = Prop::None;
                }
                if nt != t {
                    self.put_tile(x, y, nt, None);
                }
            }
            (Ending::Decline, 0) => {
                if t.prop.is_something() || t.is_water() || lane || t.kind.is_worked() {
                    return;
                }
                let r = h01(seed ^ 0xDE5, x, y);
                let prop = if r < 0.30 {
                    match self.nearby_tree(x, y) {
                        Some(sp) => Prop::Tree { species: sp, size: (r * 40.0) as u8 % 2, variant: (r * 255.0) as u8 },
                        None => Prop::Bush((r * 200.0) as u8),
                    }
                } else if r < 0.65 {
                    Prop::TallGrass((r * 200.0) as u8)
                } else {
                    Prop::Bush((r * 200.0) as u8)
                };
                let mut nt = t;
                nt.prop = prop;
                self.put_tile(x, y, nt, None);
            }
            _ => {}
        }
    }

    fn nearby_tree(&self, x: i32, y: i32) -> Option<map::TreeSpecies> {
        for r in 1..=5 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if let Some(t) = self.map.get(x + dx, y + dy) {
                        if let Prop::Tree { species, .. } = t.prop {
                            return Some(species);
                        }
                    }
                }
            }
        }
        None
    }

    /// After a load: the ending's props, the water it made, and a morph in
    /// progress, put back on the regenerated map.
    pub(crate) fn restore_ending(&mut self) {
        self.restore_ending_props();
        // Water the ending made is a plain override tile: give it its class.
        let wet: Vec<(i32, i32)> = self
            .overrides
            .iter()
            .filter(|(_, t)| t.kind == Kind::Water)
            .map(|(k, _)| *k)
            .collect();
        for (x, y) in wet {
            let i = (y * self.map.w + x) as usize;
            if self.map.water_class[i] == 0 {
                // Water the ending made is the flood's water (B99c round 2).
                self.map.water_class[i] = map::WaterKind::Lagoon.code();
            }
        }
        let (ph, _) = self.locate(self.time);
        if self.ending.site_set && ph >= Phase::Morph && self.ending.next_map.is_none() {
            let m = map::generate_with(
                self.ending.next_seed,
                self.map.w,
                self.map.h,
                Some(self.ending.next_biome),
            )
            .ok();
            self.ending.next_map = m.map(Box::new);
            self.restore_morph_mask();
        }
        // The viewer starts from this very map: nothing is pending.
        self.ending.pending_flips.clear();
        self.ending.pending_classes.clear();
        self.ending.neighbour_told = true;
    }

    /// Lamps, signs, lights and lane paint stand on lanes: where the ending
    /// has taken the lane away (a crater, lava, water) they go too.
    pub(crate) fn clear_dead_furniture(&mut self) {
        let m = &self.map;
        // (Lamps stand on verges as well as lanes, so only what the ending has
        // made impossible goes: water, and the bare rock of crater and lava.)
        let on_street = |x: i32, y: i32| {
            let t = m.at(x, y);
            !(t.is_water() || matches!(t.kind, Kind::Rock | Kind::Scree))
        };
        let sw = &mut self.streetworks;
        sw.lamps.retain(|&(x, y)| on_street(x, y));
        sw.paint.retain(|&(x, y)| on_street(x, y));
        sw.lights.retain(|&(x, y)| on_street(x, y));
        sw.signs.retain(|&(x, y, _)| on_street(x, y));
    }

    /// Props the ending put on tiles survive a save.
    pub(crate) fn restore_ending_props(&mut self) {
        let props: Vec<((i32, i32), Prop)> = self.ending.props.iter().map(|(k, v)| (*k, *v)).collect();
        for ((x, y), p) in props {
            self.map.with_tile_mut(x, y, |t| t.prop = p);
        }
    }

    /// Anything the viewer's copy of the map must learn from the ending.
    pub fn take_ending_sync(&mut self) -> Option<Sync> {
        let st = &mut self.ending;
        let neighbour = if self.map.neighbour.is_some() && !self.map.morph.is_empty() && !st.neighbour_told {
            st.neighbour_told = true;
            self.map.neighbour
        } else {
            None
        };
        if neighbour.is_none() && st.pending_flips.is_empty() && st.pending_classes.is_empty() {
            return None;
        }
        Some(Sync {
            neighbour,
            flipped: std::mem::take(&mut st.pending_flips),
            classes: std::mem::take(&mut st.pending_classes),
        })
    }

    // -----------------------------------------------------------------------
    // Ruins
    // -----------------------------------------------------------------------

    pub(crate) fn refresh_ruins(&mut self, u: f32, ph: Phase) {
        let mut codes: Vec<u8> = Vec::with_capacity(self.lots.len());
        for l in &self.lots {
            codes.push(self.ruin_for(l, u, ph));
        }
        if codes.contains(&R_GONE) {
            let mut i = 0;
            self.lots.retain(|_| {
                let keep = codes[i] != R_GONE;
                i += 1;
                keep
            });
            codes.retain(|c| *c != R_GONE);
            self.ending.overlay_epoch += 0;
        }
        if codes != self.ending.ruin {
            self.ending.ruin = codes;
            self.ending.ruin_epoch += 1;
        }
        self.ending.ruin_time = self.time;
    }

    fn lava_at(&self, x: i32, y: i32) -> Option<f32> {
        self.ending.lookup.as_ref().and_then(|m| m.get(&(x, y)).copied())
    }

    fn ruin_for(&self, l: &Lot, u: f32, ph: Phase) -> u8 {
        let st = &self.ending;
        let c = lot_centre(l);
        let hs = h01(self.seed ^ l.seed as u64, l.x, l.y);
        let hs2 = h01(self.seed ^ 0x9917, l.y, l.x);
        let (sx, sy) = st.site;
        let d = dist(c, (sx, sy));
        let park = matches!(l.kind, LotKind::Park | LotKind::Cemetery);
        let commercial = matches!(
            l.kind,
            LotKind::Shop | LotKind::Store | LotKind::Office | LotKind::Tower | LotKind::Industry
        );
        let rad = st.aux[1].max(6.0);
        let mut base = R_OK;
        let mut t0 = 1.0f32; // when the base state began, for the decay
        match st.kind {
            Ending::Meteor => {
                let rs = (rad * 1.15).clamp(12.0, 26.0);
                let shock = ramp(u, 1.36, 1.95) * rs;
                if u >= 1.36 {
                    if d < 5.2 {
                        return R_GONE;
                    }
                    if d <= shock {
                        t0 = 1.36 + (d / rs) * 0.6;
                        base = if d < 0.45 * rs {
                            R_RUBBLE
                        } else if d < 0.8 * rs || hs < 0.6 {
                            R_FIRE
                        } else {
                            R_OK
                        };
                    }
                }
            }
            Ending::FallingMoon => {
                let rs = (rad * 1.7 + 8.0).clamp(26.0, 44.0);
                let shock = ramp(u, 1.7, 1.98) * rs;
                if u >= 1.7 {
                    if d < 7.0 {
                        return R_GONE;
                    }
                    if d <= shock {
                        t0 = 1.7 + (d / rs) * 0.3;
                        base = if d < 0.4 * rs { R_RUBBLE } else if hs < 0.9 { R_FIRE } else { R_OK };
                    }
                }
            }
            Ending::Volcano => {
                let ash = ramp(u, 1.05, 1.9);
                if hs < ash * 0.9 {
                    base = R_SOOT;
                    t0 = 1.5;
                }
                if u >= 1.08 && st.lookup.is_some() {
                    let frac = ramp(u, 1.08, 1.85);
                    let mut near = false;
                    'o: for dy in -3..=3 {
                        for dx in -3..=3 {
                            let (tx, ty) = (c.0 as i32 + dx, c.1 as i32 + dy);
                            if let Some(th) = self.lava_at(tx, ty) {
                                if th <= frac {
                                    let on = tx >= l.x && tx < l.x + l.w && ty >= l.y && ty < l.y + l.h;
                                    if on {
                                        return R_GONE;
                                    }
                                    near = true;
                                    break 'o;
                                }
                            }
                        }
                    }
                    if near {
                        base = R_FIRE;
                        t0 = 1.5;
                    }
                }
                if base == R_SOOT && u > 2.2 && hs2 < 0.3 {
                    base = R_RUBBLE;
                }
            }
            Ending::Flood => {
                if let Some(o) = &self.map.ocean {
                    let dl = (st.aux[2] + 2.5).clamp(10.0, 70.0);
                    let s = self.inland(o, c.0, c.1);
                    // The waterline the lots follow: the same reach the tiles
                    // flood to, without the four extra tiles this used to add —
                    // that margin is what made the ruin front run ahead of the
                    // water and drown half the town (B99c round 2).
                    let sf = -3.2 + ramp(u, 1.0, 1.55) * (dl + 3.2);
                    if u >= 1.0 && s <= sf {
                        // B99c: a drowned building does not burn. The wave leaves
                        // debris along the new waterline and takes everything
                        // behind it, which is what leaves a lagoon where the town
                        // was instead of a city of burning rooftops standing in
                        // the water.
                        let behind = sf - s;
                        if behind < 2.5 && hs2 < 0.55 {
                            base = R_RUBBLE;
                            t0 = 1.2;
                        } else {
                            return R_GONE;
                        }
                    }
                }
            }
            Ending::Kaiju => {
                let a = (st.aux[0], st.aux[1]);
                let b = (st.aux[2], st.aux[3]);
                let (dd, along) = seg_dist(c, a, b);
                let gs = map::endgeo::giants(&st.aux, u);
                for g in &gs {
                    let (_, sg) = seg_dist((g.x, g.y), a, b);
                    let passed = if g.which == 0 { along <= sg + 0.004 } else { along >= sg - 0.004 };
                    let fighting = g.mode == map::endgeo::GiantMode::Fight
                        || g.mode == map::endgeo::GiantMode::Fall
                        || g.mode == map::endgeo::GiantMode::Gone;
                    let near_fight = fighting && dist(c, (g.x, g.y)) < 5.0;
                    if (passed && dd < 2.7) || near_fight || (g.mode == map::endgeo::GiantMode::Gone && dd < 2.7) {
                        base = R_RUBBLE;
                        t0 = u.min(2.0);
                    } else if passed && dd < 5.6 && hs < 0.55 && base == R_OK {
                        base = R_FIRE;
                        t0 = u.min(2.0);
                    }
                }
                if u >= 2.0 && base == R_OK {
                    // Only the wake is damaged; the rest is simply empty.
                    let _ = (dd, along);
                }
            }
            Ending::Aliens => {
                if hs < 0.04 && u > 1.4 {
                    base = R_FIRE;
                    t0 = 1.5;
                } else if hs2 < ramp(u, 1.4, 2.0) {
                    base = R_DARK;
                }
            }
            Ending::AiTakeover => {
                let far = st.aux[2].max(8.0);
                let rf = if u < 0.55 {
                    0.0
                } else if u < 1.0 {
                    ramp(u, 0.55, 1.0) * 0.12 * far
                } else {
                    0.12 * far + ramp(u, 1.0, 1.92) * 0.88 * far
                };
                if d <= rf && !park {
                    base = if u >= 2.05 { R_MACHINE_DARK } else { R_MACHINE };
                }
            }
            Ending::RedGiant => {
                if hs < ramp(u, 0.35, 1.2) {
                    base = R_BLEACH;
                }
                if u >= 1.0 && hs2 < ramp(u, 1.0, 1.8) {
                    base = R_FIRE;
                    t0 = 1.0 + hs2 * 0.8;
                }
            }
            Ending::IceAge => {
                if hs < ramp(u, 0.3, 1.3) {
                    base = R_FROST;
                }
                if u >= 2.0 && hs2 < 0.3 * ramp(u, 2.0, 2.6) {
                    base = R_RUBBLE;
                    t0 = 2.0;
                }
            }
            Ending::Decline => {
                let close = if commercial { 0.1 + 0.5 * hs } else { 0.7 + 1.4 * hs };
                if u >= close && !park {
                    base = R_ABANDONED;
                    t0 = close;
                    if u > 1.3 + hs2 * 1.0 && hs2 < 0.45 {
                        base = R_RUBBLE;
                    } else if u > 1.0 + hs2 * 1.0 && hs2 < 0.85 {
                        base = R_OVERGROWN;
                    }
                }
            }
            Ending::Smog => {
                if hs < ramp(u, 0.1, 1.2) {
                    base = R_SOOT;
                }
                if u >= 1.0 && hs2 < ramp(u, 1.0, 2.0) {
                    base = R_DARK;
                }
            }
            Ending::Sunset => {
                if hs < ramp(u, 1.1, 1.9) {
                    base = R_DARK;
                }
                if u >= 2.1 && hs2 < 0.5 {
                    base = R_ABANDONED;
                }
            }
        }
        let mut code = base;
        // Fires burn down.
        if code == R_FIRE && u > t0 + 0.9 + hs * 0.9 && u >= 2.0 {
            code = R_RUBBLE;
        }
        // Whatever is left empty goes dark once the people have gone.
        if code == R_OK && u > 2.0 + 0.6 * hs && st.kind != Ending::Sunset {
            code = R_DARK;
        }
        // Everything weathers in the morph, before the front takes it.
        if u > 3.0 + hs * 0.8 && code != R_GONE {
            code = R_OVERGROWN;
        }
        if ph >= Phase::Morph && self.map.morphed(l.x, l.y) {
            return R_GONE;
        }
        if park && code != R_OVERGROWN && code != R_GONE {
            return R_OK;
        }
        code
    }

    // -----------------------------------------------------------------------
    // People and streets
    // -----------------------------------------------------------------------

    /// What share of the town has left by now.
    fn leave_share(&self, ph: Phase, p: f32) -> f32 {
        let (w_end, s_end) = match self.ending.kind {
            Ending::Meteor => (0.35, 0.7),
            Ending::Volcano => (0.55, 0.8),
            Ending::Flood => (0.5, 0.75),
            Ending::Kaiju => (0.6, 0.85),
            Ending::Aliens => (0.15, 1.0),
            Ending::AiTakeover => (0.1, 1.0),
            Ending::RedGiant => (0.45, 0.8),
            Ending::IceAge => (0.5, 0.8),
            Ending::FallingMoon => (0.6, 0.85),
            Ending::Decline => (0.55, 0.85),
            Ending::Smog => (0.5, 0.85),
            Ending::Sunset => (0.1, 0.5),
        };
        match ph {
            Phase::Living => 0.0,
            Phase::Warning => ramp(p, 0.3, 1.0) * w_end,
            Phase::Spectacle => w_end + (s_end - w_end) * p,
            Phase::Aftermath => s_end + (1.0 - s_end) * ramp(p, 0.0, 0.9),
            _ => 1.0,
        }
    }

    pub(crate) fn people_leave(&mut self, ph: Phase, p: f32) {
        let share = self.leave_share(ph, p);
        if share <= 0.0 {
            return;
        }
        let seed = self.seed;
        let mut changed = false;
        for (i, per) in self.people.list.iter_mut().enumerate() {
            if per.alive() && h01(seed ^ 0xEAFE, i as i32, 3) < share {
                per.life = crate::people::Life::Away;
                per.job = None;
                per.job_kind = crate::people::JobKind::None;
                per.spouse = None;
                changed = true;
            }
        }
        if changed {
            self.people.recount();
        }
    }

    pub(crate) fn thin_the_streets(&mut self, ph: Phase) {
        if ph >= Phase::Aftermath {
            self.traffic.vehicles.clear();
            // Buses are transit movers since B58, not a traffic list.
            self.rail.movers.retain(|m| {
                !matches!(
                    m.kind,
                    crate::rail::MoverKind::Bus | crate::rail::MoverKind::Tram
                )
            });
        }
    }

    /// The people's part in the ending: who looks up, who runs, who is lifted,
    /// who has been converted. Posed over the normal day's figures.
    pub(crate) fn ending_adjust_figures(&self, out: &mut Vec<crate::people::Figure>) {
        use crate::people::Pose;
        let t = self.smooth_time();
        let (ph, p) = self.locate(t);
        if ph == Phase::Living || ph >= Phase::Morph || !self.ending.site_set {
            return;
        }
        let u = self.ending_u(t);
        let st = &self.ending;
        let (sx, sy) = st.site;
        let seed = self.seed;
        let kind = st.kind;
        let facing = |x: f32, y: f32, tx: f32, ty: f32| -> u8 {
            let (dx, dy) = (tx - x, ty - y);
            if dx.abs() > dy.abs() {
                if dx >= 0.0 { 0 } else { 2 }
            } else if dy >= 0.0 {
                1
            } else {
                3
            }
        };
        let saucers = if kind == Ending::Aliens {
            let n = st.aux[2].max(3.0) as usize;
            map::endgeo::saucers(st.site, st.aux[1], n, seed, u, t as f32)
        } else {
            Vec::new()
        };
        let giants = if kind == Ending::Kaiju { map::endgeo::giants(&st.aux, u) } else { Vec::new() };
        // Nobody stands in a crater, in lava or in the sea.
        let crater = matches!(kind, Ending::Meteor | Ending::FallingMoon) && u >= if kind == Ending::Meteor { 1.36 } else { 1.7 };
        let crater_r = if kind == Ending::Meteor { 5.4 } else { 8.5 };
        out.retain(|f| {
            let t = self.map.at(f.x as i32, f.y as i32);
            !(t.is_water() && t.kind != Kind::Bridge)
                && !(crater && dist((f.x, f.y), (sx, sy)) < crater_r)
        });
        let far = st.aux[2].max(8.0);
        let rf = if u < 1.0 {
            ramp(u, 0.55, 1.0) * 0.12 * far
        } else {
            0.12 * far + ramp(u, 1.0, 1.92) * 0.88 * far
        };
        for (i, f) in out.iter_mut().enumerate() {
            let hf = h01(seed ^ 0xF16, i as i32, 11);
            let hf2 = h01(seed ^ 0xF17, i as i32, 12);
            // Everyone looks at the omen.
            let look_share = match ph {
                Phase::Warning => ramp(p, 0.35, 1.0) * 0.7,
                Phase::Spectacle => 0.55,
                Phase::Aftermath => 0.4,
                _ => 0.0,
            };
            let spot = if kind == Ending::Volcano || kind == Ending::Kaiju {
                (sx, sy)
            } else {
                (sx, sy)
            };
            let on_foot = matches!(f.pose, Pose::Walk | Pose::Stand | Pose::Chat | Pose::Play | Pose::Wave);
            if on_foot && f.lift == 0 && hf < look_share {
                f.pose = if hf2 < 0.28 { Pose::Wave } else { Pose::Stand };
                f.dir = facing(f.x, f.y, spot.0, spot.1);
                f.prop = crate::people::Prop::None;
            }
            // The spectacle: panic.
            if ph == Phase::Spectacle && on_foot && f.lift == 0 && hf >= look_share && hf < 0.9 {
                f.pose = Pose::Play;
            }
            match kind {
                Ending::Aliens => {
                    for s in &saucers {
                        if s.beam > 0.05 {
                            let d = dist((f.x, f.y), (s.x, s.y));
                            if d < 1.35 {
                                let k = (1.0 - d / 1.35) * s.beam;
                                f.lift = ((k * 180.0) as u8).max(f.lift);
                                f.pose = Pose::Stand;
                            }
                        }
                    }
                }
                Ending::AiTakeover => {
                    if dist((f.x, f.y), (sx, sy)) <= rf {
                        f.lift = 255; // a drone, not a person
                        f.pose = Pose::Stand;
                    }
                }
                Ending::Kaiju => {
                    for g in &giants {
                        if dist((f.x, f.y), (g.x, g.y)) < 6.0 && ph == Phase::Spectacle {
                            f.pose = Pose::Play;
                        }
                    }
                }
                Ending::Sunset => {
                    if ph >= Phase::Spectacle && on_foot && f.lift == 0 && hf < 0.8 {
                        f.pose = Pose::Sit;
                        f.dir = 2;
                    }
                }
                _ => {}
            }
        }
    }
}

fn ginger(seed: u64, k: u32) -> f32 {
    // The same hash `map::endgeo` uses for the saucers' places.
    // B99d: the k term wraps (a debug build panicked on the multiply for k >= 2;
    // release builds already wrapped, so the sequence is unchanged).
    let mut z = seed
        .wrapping_add((k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0x1234_5678);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}
