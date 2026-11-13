//! Street life (B52, D67): the small, true-to-scale things that make a town feel
//! lived in. A street band in the square on a weekend afternoon, a saxophone
//! player under a lamp late at night, cats on the roofs at dusk, children with
//! a lemonade stand by the park, anglers at the water's edge.
//!
//! Nothing here is stored. Every item is a pure function of the clock, the day
//! of the week, the seed and what the city has built, so a saved city shows the
//! same street life on reload, and a frame costs nothing: the wallpaper asks
//! once per ten sim minutes and keeps the answer.

use map::rng::hash2;

use crate::landmarks::LandmarkKind;
use crate::{Era, LotKind, Sim};

/// One thing on the street.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LifeKind {
    /// Four players of the street band, in a row at the square's edge.
    Band,
    /// The late-night saxophonist.
    Sax,
    /// A cat on a roof ridge (`x`,`y`,`w`,`h` are the lot's).
    Cat,
    /// Two children and their stand.
    Lemonade,
    /// An angler on the bank, facing the water in direction `dir`.
    Angler,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LifeItem {
    pub kind: LifeKind,
    /// Tile position (a lot's anchor for a cat).
    pub x: f32,
    pub y: f32,
    /// A cat's lot size; otherwise 0.
    pub w: i32,
    pub h: i32,
    /// 0 = +x, 1 = +y, 2 = -x, 3 = -y: the way the item faces.
    pub dir: u8,
    pub seed: u32,
}

impl Sim {
    /// What is happening on the street at the sim clock.
    pub fn street_life(&self) -> Vec<LifeItem> {
        let mut out = Vec::new();
        if self.era() < Era::Town || self.lots.len() < 30 || self.ending_underway() {
            return out;
        }
        let hour = (self.time / 3600.0).rem_euclid(24.0);
        let day = (self.time / 86400.0).floor() as i64;
        let weekday = self.weekday() as i64;
        let weekend = weekday == 0 || weekday == 6;
        let plaza = self
            .lots
            .iter()
            .find(|l| l.finished() && l.kind == LotKind::Landmark(LandmarkKind::Plaza));

        // The band plays the square on a weekend afternoon, and on the evening
        // of a rally.
        let rally = matches!(self.election_phase(), crate::civics::Phase::Rally(_));
        if let Some(p) = plaza {
            if (weekend && (13.0..16.5).contains(&hour)) || rally {
                // Along the square's near (+y) edge, facing in.
                out.push(LifeItem {
                    kind: LifeKind::Band,
                    x: p.x as f32 + p.w as f32 * 0.5,
                    y: p.y as f32 + p.h as f32 - 0.45,
                    w: p.w,
                    h: p.h,
                    dir: 3,
                    seed: p.seed,
                });
            }
        }

        // The saxophonist: a lamp by the square, late at night.
        if !(hour >= 21.5 || hour < 1.0) {
            // fallthrough: not the hour
        } else if let Some(p) = plaza {
            let lamps = &self.streetworks.lamps;
            let (ax, ay) = (p.x + p.w / 2, p.y + p.h / 2);
            let best = lamps
                .iter()
                .filter(|&&(x, y)| (x - ax).abs() + (y - ay).abs() <= 14)
                .min_by_key(|&&(x, y)| ((x - ax).abs() + (y - ay).abs(), x, y));
            if let Some(&(lx, ly)) = best {
                // Stand on the walkable tile beside the lamp (not the road).
                for (dx, dy) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
                    let (x, y) = (lx + dx, ly + dy);
                    if self.map.in_bounds(x, y) && self.map.at(x, y).kind.is_walkable() && !self.map.at(x, y).kind.is_lane() {
                        out.push(LifeItem {
                            kind: LifeKind::Sax,
                            x: x as f32 + 0.5,
                            y: y as f32 + 0.5,
                            w: 0,
                            h: 0,
                            dir: (3 - (dx + dy * 2).rem_euclid(4)).rem_euclid(4) as u8,
                            seed: (lx * 31 + ly) as u32,
                        });
                        break;
                    }
                }
            }
        }

        // Cats on the roofs, from dusk to the small hours and again at dawn.
        if hour >= 17.5 || hour < 7.5 {
            let mut houses: Vec<&crate::Lot> = self
                .lots
                .iter()
                .filter(|l| l.finished() && matches!(l.kind, LotKind::House | LotKind::Townhouse | LotKind::Shop))
                .collect();
            houses.sort_by(|a, b| {
                hash2(a.x, a.y, self.seed ^ 0xCA7)
                    .partial_cmp(&hash2(b.x, b.y, self.seed ^ 0xCA7))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let (fx, fy) = self.founding;
            for l in houses.iter().filter(|l| (l.x - fx).abs() + (l.y - fy).abs() <= 24).take(3) {
                // Each cat keeps to its own hour or two, so they come and go.
                let h0 = 18.0 + hash2(l.x, l.y, self.seed ^ 0xCA8) as f64 * 8.0;
                let h = if hour < 7.5 { hour + 24.0 } else { hour };
                if h >= h0 && h < h0 + 4.0 {
                    out.push(LifeItem {
                        kind: LifeKind::Cat,
                        x: l.x as f32,
                        y: l.y as f32,
                        w: l.w,
                        h: l.h,
                        dir: (hash2(l.x, l.y, self.seed ^ 0xCA9) * 4.0) as u8 % 4,
                        seed: l.seed,
                    });
                }
            }
        }

        // Children with a lemonade stand by the park, on a warm weekend noon.
        let _ = day;
        if weekend && (11.0..16.0).contains(&hour) {
            if let Some(park) = self.lots.iter().find(|l| l.finished() && l.kind == LotKind::Park) {
                let (dx, dy) = crate::people::door_tile(park);
                out.push(LifeItem {
                    kind: LifeKind::Lemonade,
                    x: dx as f32 + 0.5,
                    y: dy as f32 + 0.5,
                    w: 0,
                    h: 0,
                    dir: park.front % 4,
                    seed: park.seed,
                });
            }
        }

        // Anglers on the bank, early and in the evening.
        if (5.5..10.0).contains(&hour) || (16.0..20.0).contains(&hour) {
            out.extend(self.angler_spots().into_iter().take(2));
        }
        out
    }

    /// The best dry banks near the town: a free walkable tile with water on one
    /// side, ordered by how close it is to the founding.
    pub fn angler_spots(&self) -> Vec<LifeItem> {
        let (fx, fy) = self.founding;
        let mut best: Vec<(i32, LifeItem)> = Vec::new();
        for dy in -34..=34i32 {
            for dx in -34..=34i32 {
                let (x, y) = (fx + dx, fy + dy);
                if !self.map.in_bounds(x, y) {
                    continue;
                }
                let t = self.map.at(x, y);
                if t.is_water() || t.kind.is_lane() || t.relief() > 0 {
                    continue;
                }
                if self.lots.iter().any(|l| l.covers(x, y)) {
                    continue;
                }
                for (i, (nx, ny)) in [(1, 0), (0, 1), (-1, 0), (0, -1)].into_iter().enumerate() {
                    let (wx, wy) = (x + nx, y + ny);
                    if self.map.in_bounds(wx, wy) && self.map.at(wx, wy).is_water() {
                        // The water's depth: a pond is no place for an angler.
                        let mut run = 0;
                        while run < 4 && self.map.in_bounds(wx + nx * run, wy + ny * run) && self.map.at(wx + nx * run, wy + ny * run).is_water() {
                            run += 1;
                        }
                        if run >= 3 && hash2(x, y, self.seed ^ 0xF15) < 0.5 {
                            best.push((
                                dx.abs() + dy.abs(),
                                LifeItem { kind: LifeKind::Angler, x: x as f32 + 0.5, y: y as f32 + 0.5, w: 0, h: 0, dir: i as u8, seed: (x * 17 + y) as u32 },
                            ));
                        }
                        break;
                    }
                }
            }
        }
        best.sort_by_key(|(d, it)| (*d, it.x as i32, it.y as i32));
        // Two anglers a few tiles apart, not shoulder to shoulder.
        let mut out: Vec<LifeItem> = Vec::new();
        for (_, it) in best {
            if out.iter().all(|o| (o.x - it.x).abs() + (o.y - it.y).abs() >= 4.0) {
                out.push(it);
            }
            if out.len() == 2 {
                break;
            }
        }
        out
    }
}
