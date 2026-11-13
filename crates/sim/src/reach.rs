//! B66: the land the road network can reach.
//!
//! On a coast the map is not one island: a beach spit, a sandbar or an islet
//! is its own patch of dry land, and no street can ever get to it without a
//! bridge (B58's ferries come later). The sim used to stake plots anywhere it
//! found a worn footpath, so `islands` seed 3 ended up with ten houses on a
//! sandbar twenty tiles from the nearest lane: ten villagers the traffic could
//! never route, and a fact the handback called out.
//!
//! This table answers one question per plot — "is this on land the streets can
//! reach?" — as a land-region flood fill (4-connected dry ground) plus which
//! regions hold a lane. It is derived state: rebuilt lazily whenever a tile
//! changes from water to land or back, or a tile becomes (or stops being) a
//! lane. A rebuild is one pass over the map (~100 us for 107x107).

use map::TileMap;

/// No region: a water tile (or off the map).
pub const WATER: u16 = u16::MAX;

/// The land regions of one map, and where its lanes stand.
pub struct Reach {
    w: i32,
    h: i32,
    /// Per tile: the land region it belongs to, or [`WATER`].
    region: Vec<u16>,
    /// Per region: does a lane stand on it?
    lane: Vec<bool>,
    /// The region the settlement was founded on (the streets start there), and
    /// the tile it was founded at, so a moved founding rebuilds.
    founding: u16,
    founding_tile: (i32, i32),
    /// Does the map hold any lane at all? Before the first cart track there
    /// are no streets and a worn path is frontage enough (D62).
    any_lane: bool,
    /// Set when a tile changed under the table; `ensure` rebuilds.
    dirty: bool,
}

impl Default for Reach {
    fn default() -> Self {
        Self {
            w: 0,
            h: 0,
            region: Vec::new(),
            lane: Vec::new(),
            founding: WATER,
            founding_tile: (i32::MIN, i32::MIN),
            any_lane: false,
            dirty: true,
        }
    }
}

impl Reach {
    /// Something changed: the next [`Self::ensure`] rebuilds.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Rebuild if the table is stale (a tile changed) or the settlement moved.
    pub fn ensure(&mut self, map: &TileMap, founding: (i32, i32)) {
        if !self.dirty && self.w == map.w && self.h == map.h && self.founding_tile == founding {
            return;
        }
        self.rebuild(map, founding);
    }

    /// Does the map hold any lane at all?
    pub fn any_lane(&self) -> bool {
        self.any_lane
    }

    /// Is this tile on the founding landmass, or on one a lane already stands
    /// on? (Water is never on the network.)
    pub fn on_network_land(&self, x: i32, y: i32) -> bool {
        let Some(r) = self.region_of(x, y) else { return false };
        r == self.founding || self.lane.get(r as usize).copied().unwrap_or(false)
    }

    fn region_of(&self, x: i32, y: i32) -> Option<u16> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return None;
        }
        let r = self.region[(y * self.w + x) as usize];
        (r != WATER).then_some(r)
    }

    fn rebuild(&mut self, map: &TileMap, founding: (i32, i32)) {
        self.w = map.w;
        self.h = map.h;
        self.region = vec![WATER; (map.w * map.h).max(0) as usize];
        self.lane = Vec::new();
        self.any_lane = false;
        let mut stack: Vec<(i32, i32)> = Vec::new();
        let mut next = 0u32;
        for y in 0..map.h {
            for x in 0..map.w {
                let i = (y * map.w + x) as usize;
                if self.region[i] != WATER || map.at(x, y).is_water() {
                    continue;
                }
                let id = next as u16;
                next += 1;
                self.region[i] = id;
                stack.push((x, y));
                while let Some((cx, cy)) = stack.pop() {
                    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let (nx, ny) = (cx + dx, cy + dy);
                        if nx < 0 || ny < 0 || nx >= map.w || ny >= map.h {
                            continue;
                        }
                        let j = (ny * map.w + nx) as usize;
                        if self.region[j] == WATER && !map.at(nx, ny).is_water() {
                            self.region[j] = id;
                            stack.push((nx, ny));
                        }
                    }
                }
            }
        }
        self.lane = vec![false; next as usize];
        for y in 0..map.h {
            for x in 0..map.w {
                if !map.at(x, y).kind.is_lane() {
                    continue;
                }
                self.any_lane = true;
                if let Some(r) = self.region_of(x, y) {
                    self.lane[r as usize] = true;
                }
            }
        }
        self.founding = self.region_of(founding.0, founding.1).unwrap_or(WATER);
        self.founding_tile = founding;
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use map::{Kind, Tile, TileMap};

    fn map(kinds: &[&str]) -> TileMap {
        let w = kinds[0].len() as i32;
        let h = kinds.len() as i32;
        let mut m = TileMap::fill(w, h, Tile::new(2, Kind::Grass), 1);
        for (y, row) in kinds.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                let kind = match c {
                    '~' => Kind::Water,
                    '=' => Kind::Paved,
                    _ => Kind::Grass,
                };
                m.set(x as i32, y as i32, Tile::new(2, kind));
            }
        }
        m
    }

    #[test]
    fn an_islet_across_the_water_is_not_on_the_network() {
        let m = map(&[".....~", ".....~", ".....~", ".....~", ".....~"]);
        let mut r = Reach::default();
        r.ensure(&m, (1, 1));
        assert!(r.on_network_land(1, 1));
        assert!(r.on_network_land(4, 3));
        assert!(!r.on_network_land(5, 3), "across the water");
        assert!(!r.on_network_land(9, 9), "off the map");
        assert!(!r.any_lane());
    }

    #[test]
    fn a_lane_puts_its_landmass_on_the_network() {
        let m = map(&[
            ".......~",
            ".......~",
            ".......~",
            "===....~",
            ".......~",
        ]);
        let mut r = Reach::default();
        r.ensure(&m, (6, 1));
        assert!(r.any_lane());
        // The founding landmass is still allowed, and so is the lane's.
        assert!(r.on_network_land(6, 1));
        assert!(r.on_network_land(0, 3));
        assert!(!r.on_network_land(7, 3), "the water is not land");
    }

    #[test]
    fn a_tile_change_rebuilds() {
        let mut m = map(&["...~", "...~", "...~"]);
        let mut r = Reach::default();
        r.ensure(&m, (0, 0));
        assert!(!r.on_network_land(3, 1));
        // Bridge the strait: the far side becomes the same landmass.
        m.set(3, 1, Tile::new(2, Kind::Grass));
        r.invalidate();
        r.ensure(&m, (0, 0));
        assert!(r.on_network_land(3, 1));
    }

    #[test]
    fn every_land_is_on_the_network() {
        // Sanity: a plain map has one region and no lanes.
        let m = map(&["....", "....", "...."]);
        let mut r = Reach::default();
        r.ensure(&m, (1, 1));
        for y in 0..3 {
            for x in 0..4 {
                assert!(r.on_network_land(x, y));
            }
        }
    }

    #[test]
    fn no_home_on_land_the_streets_cannot_reach() {
        // B66, the reported defect: on `islands` the sim staked houses on the
        // beach spits, twenty tiles from any lane, and counted ten trips it
        // could not route (`traffic.why[0]`). Homes now stand on land the
        // streets are on, and a house with no lane simply has no car traffic.
        use crate::{LotKind, Pace, Sim};
        let vp = map::world_for_viewport(1920, 1080);
        for seed in [3u64, 11] {
            let mut s = Sim::with_focus_in(
                seed,
                Pace::Month,
                vp.w,
                vp.h,
                (vp.w as f32 * 0.5, vp.h as f32 * 0.5),
                Some(map::Biome::Islands),
            );
            s.ending.world_days = 1e9;
            s.run_to(20.0 * 86400.0);
            let homes = s
                .lots
                .iter()
                .filter(|l| matches!(l.kind, LotKind::House | LotKind::Townhouse | LotKind::Apartment))
                .count();
            assert!(homes > 10, "seed {seed}: only {homes} homes to test");
            let founding = s.founding;
            s.reach.ensure(&s.map, founding);
            for l in &s.lots {
                if !matches!(l.kind, LotKind::House | LotKind::Townhouse | LotKind::Apartment) {
                    continue;
                }
                assert!(
                    s.reach.on_network_land(l.x + l.w / 2, l.y + l.h / 2),
                    "seed {seed}: a home at {},{} on land with no street on it",
                    l.x,
                    l.y
                );
            }
            s.warm_traffic(300.0);
            // The one that can still happen is a fire engine leaving a fire
            // station whose door has no lane (a landmark needs no frontage).
            assert!(
                s.traffic.why[0] <= 1,
                "seed {seed}: {} trips with no lane at their origin",
                s.traffic.why[0]
            );
        }
    }
}
