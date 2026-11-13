//! Tile routing and network queries.
//!
//! Lanes are routed with A* over the tile grid. Cost per tile: base 1, plus a
//! slope penalty (so lanes prefer gentle ground and climb at most in steps),
//! plus a per-tile noise term (which turns a straight line into a gently
//! wandering one, D36), plus a heavy penalty for water (only crossable once a
//! bridge is allowed).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use map::rng::hash2;
use map::{Kind, TileMap};

#[derive(Clone, Copy, PartialEq)]
struct Node {
    f: f32,
    x: i32,
    y: i32,
}

impl Eq for Node {}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed so BinaryHeap is a min-heap on f, with a stable tie-break.
        other
            .f
            .partial_cmp(&self.f)
            .unwrap_or(Ordering::Equal)
            .then_with(|| other.x.cmp(&self.x))
            .then_with(|| other.y.cmp(&self.y))
    }
}

#[allow(clippy::too_many_arguments)]
/// Rail alignments (B31): a line is its own civil work, so it crosses streets
/// (at a level crossing) but never runs *along* one — the whole point of the
/// extra cost for lane tiles. Grass and meadow are cheap, a river is bridged.
#[allow(clippy::too_many_arguments)]
fn tile_cost_rail(
    map: &TileMap,
    x: i32,
    y: i32,
    from_level: u8,
    allow_water: bool,
    seed: u64,
    blocked: &std::collections::BTreeSet<(i32, i32)>,
) -> Option<f32> {
    if !map.in_bounds(x, y) {
        return None;
    }
    let t = map.at(x, y);
    if t.is_water() {
        let river = matches!(
            map.water_kind(x, y),
            map::WaterKind::River | map::WaterKind::None
        );
        return if allow_water && river { Some(3.0) } else { None };
    }
    let mut cost = 1.0f32;
    if t.kind.is_lane() || t.kind == Kind::Sidewalk {
        // Crossing is fine, following is not.
        cost += 5.0;
    }
    // A rail is a civil work: it may cut through a block (the sim clears the
    // plots on the alignment), but the planner prefers open ground (B31).
    if blocked.contains(&(x, y)) {
        cost += 7.0;
    }
    if t.relief() >= 2 {
        cost += 8.0;
    }
    // B46d: bare rock is a landform's footprint (or a peak): no street or rail
    // crosses it.
    if t.kind == Kind::Rock {
        return None;
    }
    if t.kind == Kind::Scree {
        cost += 4.0;
    }
    let slope = (t.min_level() as i32 - from_level as i32).abs() as f32;
    let noise = hash2(x, y, seed) * 0.7;
    Some(cost + 0.8 * slope + noise)
}

/// A rail alignment from `from` to `to` (B31).
pub fn route_rail(
    map: &TileMap,
    from: (i32, i32),
    to: (i32, i32),
    seed: u64,
    blocked: &std::collections::BTreeSet<(i32, i32)>,
) -> Option<Vec<(i32, i32)>> {
    route_impl_with(map, from, to, true, seed, blocked, false, true, false)
}

fn tile_cost_mode(
    map: &TileMap,
    x: i32,
    y: i32,
    from_level: u8,
    allow_water: bool,
    seed: u64,
    blocked: &std::collections::BTreeSet<(i32, i32)>,
    walk: bool,
    streets: bool,
) -> Option<f32> {
    if !map.in_bounds(x, y) {
        return None;
    }
    if walk {
        // Pedestrian (D45): grass and meadow are fine, a beaten path is much
        // cheaper than the grass beside it (so paths reinforce themselves),
        // a lane is cheapest, and water, cliffs and buildings are walls.
        let t = map.at(x, y);
        if t.is_water() || t.relief() >= 2 || blocked.contains(&(x, y)) {
            return None;
        }
        if t.kind.is_lane() {
            return Some(0.25);
        }
        // The pavement beside a road is the right place for a commuter.
        if streets && t.kind == Kind::Sidewalk {
            return Some(0.20);
        }
        let mut c: f32 = match t.kind {
            Kind::Sand => 0.9,
            Kind::Meadow => 1.05,
            Kind::Upland => 1.2,
            Kind::DryGrass => 1.1,
            Kind::Rock | Kind::Scree => 2.5,
            _ => 1.0,
        };
        c -= match map::WearStage::of(map.wear(x, y)) {
            map::WearStage::CartTrack => 0.45,
            map::WearStage::Footpath => 0.30,
            map::WearStage::Trampled => 0.12,
            map::WearStage::None => 0.0,
        };
        if streets {
            // Grass is a last resort: three times dearer, so a commuter detours
            // along the street rather than crossing a block. A worn footpath is
            // a real path and much cheaper (D45's desire paths).
            let wear = match map::WearStage::of(map.wear(x, y)) {
                map::WearStage::CartTrack => 0.35,
                map::WearStage::Footpath => 0.45,
                map::WearStage::Trampled => 0.85,
                map::WearStage::None => 1.0,
            };
            return Some(c.max(0.15) * 5.0 * wear + 0.05);
        }
        return Some(c.max(0.15) + hash2(x, y, seed) * 0.15);
    }
    // A street may cut through a yard (it is how a village actually grows), so
    // plots are expensive but passable; the caller clears the plot it crosses.
    let through_lot = blocked.contains(&(x, y));
    // Keep lanes a tile clear of yards so every street keeps its verges.
    let near_lot = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .any(|(dx, dy)| blocked.contains(&(x + dx, y + dy)));
    let t = map.at(x, y);
    if t.kind.is_lane() {
        return Some(0.6);
    }
    // Keep new lanes a tile clear of existing ones: parallel duplicate streets
    // read as one enormous road (B11 item 2).
    let hugging = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .filter(|(dx, dy)| map.at(x + dx, y + dy).kind.is_lane())
        .count();
    if hugging > 0 {
        return Some(1.0 + 2.5 * hugging as f32);
    }
    if t.kind == Kind::Sidewalk {
        return None; // pavement belongs to a road that already exists
    }
    if t.is_water() {
        // A lane only ever crosses a river (B21/D44): the sea and a lake are
        // barriers, so no road is ever painted over open water.
        // A map built without water classes (tests, an old save) is treated as
        // a river, so the historical crossings still work.
        let river = matches!(
            map.water_kind(x, y),
            map::WaterKind::River | map::WaterKind::None
        );
        // A deliberate crossing must beat the detour, otherwise the road goes
        // round the river and there is never a deck (B13 item 3).
        return if allow_water && river { Some(2.0) } else { None };
    }
    // Cliffs and bare rock are expensive, not impossible: a lane boxed in by a
    // terrace must still be able to get out.
    let mut cost = 0.0f32;
    if t.relief() >= 2 {
        cost += 8.0;
    }
    // B46d: bare rock is a landform's footprint (or a peak): no street or rail
    // crosses it.
    if t.kind == Kind::Rock {
        return None;
    }
    if t.kind == Kind::Scree {
        cost += 4.0;
    }
    let slope = (t.min_level() as i32 - from_level as i32).abs() as f32;
    let noise = hash2(x, y, seed) * 0.9;
    // A lane may cut a yard (it is how a village grows). B26 made a tile beside
    // a plot impassable, which boxed the village in (B26b: the village stalled
    // at 31 houses and the town era was never reached); it is now merely
    // expensive, and the caller clears the plots a new street cuts or hugs.
    let extra = if through_lot {
        6.0
    } else if near_lot {
        4.0
    } else {
        0.0
    };
    // D45: a new lane prefers ground people already walk on, so the streets
    // follow the desire paths instead of cutting across them.
    let wear_bonus = match map::WearStage::of(map.wear(x, y)) {
        map::WearStage::CartTrack => 1.4,
        map::WearStage::Footpath => 0.9,
        map::WearStage::Trampled => 0.35,
        map::WearStage::None => 0.0,
    };
    Some((1.0 + 0.7 * slope + noise + cost + extra - wear_bonus).max(0.15))
}

/// Pedestrian A* (D45): the same search as [`route`] with the walker's costs —
/// grass passable, paths preferred, water/cliffs/buildings blocked.
pub fn route_walk(
    map: &TileMap,
    from: (i32, i32),
    to: (i32, i32),
    seed: u64,
    blocked: &std::collections::BTreeSet<(i32, i32)>,
) -> Option<Vec<(i32, i32)>> {
    route_impl(map, from, to, false, seed, blocked, true)
}

/// A pedestrian route that **stays on the street** (B43b): pavements and lanes
/// are cheap, open grass is three times dearer, footpaths in between. Used by
/// the commuters, so the morning crowd walks the streets when the network
/// reaches far enough — only a settler village with no lanes walks the grass.
pub fn route_walk_streets(
    map: &TileMap,
    from: (i32, i32),
    to: (i32, i32),
    seed: u64,
    blocked: &std::collections::BTreeSet<(i32, i32)>,
) -> Option<Vec<(i32, i32)>> {
    route_impl_with(map, from, to, false, seed, blocked, true, false, true)
}

/// A* from `from` to `to` with the travel direction in the state, so a turn
/// costs extra and the path comes out as long straight runs joined by a few
/// bends (B11 item 2) instead of a tile-by-tile staircase.
pub fn route(
    map: &TileMap,
    from: (i32, i32),
    to: (i32, i32),
    allow_water: bool,
    seed: u64,
    blocked: &std::collections::BTreeSet<(i32, i32)>,
) -> Option<Vec<(i32, i32)>> {
    route_impl(map, from, to, allow_water, seed, blocked, false)
}

#[allow(clippy::too_many_arguments)]
fn route_impl(
    map: &TileMap,
    from: (i32, i32),
    to: (i32, i32),
    allow_water: bool,
    seed: u64,
    blocked: &std::collections::BTreeSet<(i32, i32)>,
    walk: bool,
) -> Option<Vec<(i32, i32)>> {
    route_impl_with(map, from, to, allow_water, seed, blocked, walk, false, false)
}

/// Scratch for one A* run (B61). Reused across calls, and tagged with a
/// generation number instead of being cleared: a route used to allocate two
/// `w * h * 4` arrays and memset them (840 KB at 162x162) on every call, and a
/// growing city plans a thousand routes a day. Together the allocation and the
/// clear were a quarter of a 30-day run in `perf`.
#[derive(Default)]
struct RouteScratch {
    /// Generation each node's `g`/`prev` entry belongs to. A node is "unvisited"
    /// this run when its stamp is not the current generation.
    epoch: u32,
    seen: Vec<u32>,
    g: Vec<f32>,
    prev: Vec<u32>,
    heap: BinaryHeap<Node>,
}

impl RouteScratch {
    /// Make sure the arrays cover `n` nodes and hand out a fresh generation.
    fn reset(&mut self, n: usize) {
        if self.seen.len() < n {
            // Grow once; after that a bigger map is the only reason to allocate.
            self.seen.resize(n, 0);
            self.g.resize(n, 0.0);
            self.prev.resize(n, 0);
        }
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            // Wrapped: the stamps are stale, so clear them once in 4 billion.
            for s in &mut self.seen[..n] {
                *s = 0;
            }
            self.epoch = 1;
        }
        self.heap.clear();
    }

    #[inline]
    fn has(&self, i: usize) -> bool {
        self.seen[i] == self.epoch
    }
}

thread_local! {
    static ROUTE_SCRATCH: std::cell::RefCell<RouteScratch> =
        std::cell::RefCell::new(RouteScratch::default());
}

#[allow(clippy::too_many_arguments)]
fn route_impl_with(
    map: &TileMap,
    from: (i32, i32),
    to: (i32, i32),
    allow_water: bool,
    seed: u64,
    blocked: &std::collections::BTreeSet<(i32, i32)>,
    walk: bool,
    rail: bool,
    streets: bool,
) -> Option<Vec<(i32, i32)>> {
    // A sub-route runs on the same thread as its caller (a trip is planned from
    // three legs), so the borrow is released before the heap loop starts.
    ROUTE_SCRATCH.with(|cell| {
        let mut s = cell.borrow_mut();
        route_with_scratch(&mut s, map, from, to, allow_water, seed, blocked, walk, rail, streets)
    })
}

#[allow(clippy::too_many_arguments)]
fn route_with_scratch(
    scratch: &mut RouteScratch,
    map: &TileMap,
    from: (i32, i32),
    to: (i32, i32),
    allow_water: bool,
    seed: u64,
    blocked: &std::collections::BTreeSet<(i32, i32)>,
    walk: bool,
    rail: bool,
    streets: bool,
) -> Option<Vec<(i32, i32)>> {
    const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    const TURN_COST: f32 = 3.0;

    let (w, h) = (map.w, map.h);
    let n = (w * h) as usize * 4;
    // A pedestrian trip is short: cap the number of popped nodes so a hundred
    // hourly re-plans during catch-up stay cheap (D45).
    let max_pops = if walk { 1200 } else { usize::MAX };
    let mut pops = 0usize;
    scratch.reset(n);
    let epoch = scratch.epoch;
    let RouteScratch { seen, g, prev, heap, .. } = scratch;
    let idx = |x: i32, y: i32, d: usize| ((y * w + x) as usize) * 4 + d;

    for d in 0..4 {
        let i = idx(from.0, from.1, d);
        seen[i] = epoch;
        g[i] = 0.0;
        prev[i] = u32::MAX;
        heap.push(Node {
            f: 0.0,
            x: from.0,
            y: from.1,
        });
    }
    // Remember which direction each queued node came from: re-derive from `g`.
    while let Some(Node { x, y, .. }) = heap.pop() {
        pops += 1;
        if pops > max_pops {
            return None;
        }
        if (x, y) == to {
            // Walk back to the best start direction.
            let mut best_d = 0;
            let mut best = f32::MAX;
            for d in 0..4 {
                let i = idx(x, y, d);
                if seen[i] == epoch && g[i] < best {
                    best = g[i];
                    best_d = d;
                }
            }
            let mut path = vec![(x, y)];
            let mut cur = (x, y, best_d);
            loop {
                let p = prev[idx(cur.0, cur.1, cur.2)];
                if p == u32::MAX {
                    break;
                }
                let pi = p as usize;
                let cell = pi / 4;
                cur = ((cell % w as usize) as i32, (cell / w as usize) as i32, pi % 4);
                path.push((cur.0, cur.1));
            }
            path.reverse();
            return Some(path);
        }
        for (d, (dx, dy)) in DIRS.iter().enumerate() {
            let (nx, ny) = (x + dx, y + dy);
            let level = map.at(x, y).min_level();
            let cost = if rail {
                tile_cost_rail(map, nx, ny, level, allow_water, seed, blocked)
            } else {
                tile_cost_mode(map, nx, ny, level, allow_water, seed, blocked, walk, streets)
            };
            let Some(cost) = cost else {
                continue;
            };
            // Cheapest way we could be standing here already.
            let mut best_prev = f32::MAX;
            let mut best_pd = 0usize;
            for pd in 0..4 {
                let i = idx(x, y, pd);
                if seen[i] != epoch {
                    continue;
                }
                let v = g[i];
                let turn = if pd == d { 0.0 } else { TURN_COST };
                if v + turn < best_prev {
                    best_prev = v + turn;
                    best_pd = pd;
                }
            }
            let tentative = best_prev + cost;
            let ni = idx(nx, ny, d);
            let known = seen[ni] == epoch;
            if !known || tentative + 1e-4 < g[ni] {
                seen[ni] = epoch;
                g[ni] = tentative;
                prev[ni] = idx(x, y, best_pd) as u32;
                let hx = (to.0 - nx).abs() as f32;
                let hy = (to.1 - ny).abs() as f32;
                heap.push(Node {
                    f: tentative + hx + hy,
                    x: nx,
                    y: ny,
                });
            }
        }
    }
    None
}

/// Every lane tile, in scan order (deterministic).
pub fn lane_tiles(map: &TileMap) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for y in 0..map.h {
        for x in 0..map.w {
            if map.at(x, y).kind.is_lane() {
                v.push((x, y));
            }
        }
    }
    v
}

/// Sizes of the connected components of the lane network, largest first
/// (diagnostics: a network that is not one piece shows up here).
pub fn components(map: &TileMap) -> Vec<usize> {
    let tiles = lane_tiles(map);
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for t in tiles {
        if seen.contains(&t) {
            continue;
        }
        let mut n = 0;
        let mut q = std::collections::VecDeque::new();
        q.push_back(t);
        seen.insert(t);
        while let Some((x, y)) = q.pop_front() {
            n += 1;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let p = (x + dx, y + dy);
                if seen.contains(&p) || !map.at(p.0, p.1).kind.is_lane() {
                    continue;
                }
                seen.insert(p);
                q.push_back(p);
            }
        }
        out.push(n);
    }
    out.sort_unstable_by(|a, b| b.cmp(a));
    out
}

/// True when every lane tile is reachable from every other one (D32).
pub fn network_connected(map: &TileMap) -> bool {
    let tiles = lane_tiles(map);
    if tiles.is_empty() {
        return true;
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(tiles[0]);
    seen.insert(tiles[0]);
    while let Some((x, y)) = queue.pop_front() {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let p = (x + dx, y + dy);
            if seen.contains(&p) || !map.at(p.0, p.1).kind.is_lane() {
                continue;
            }
            seen.insert(p);
            queue.push_back(p);
        }
    }
    seen.len() == tiles.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_across_open_ground() {
        // A plain field: since D44 a generated world has a river, a lake or a
        // sea in the way, which is a different test (the bridge/route rules).
        let m = map::TileMap::fill(71, 71, map::Tile::flat(2), 1);
        let blocked = std::collections::BTreeSet::new();
        // Find a lane-free start on flat ground.
        let mut start = None;
        for y in 20..50 {
            for x in 20..50 {
                if m.is_flat(x, y) && !m.at(x, y).is_water() {
                    start = Some((x, y));
                    break;
                }
            }
            if start.is_some() {
                break;
            }
        }
        let (sx, sy) = start.unwrap();
        for (dx, dy) in [(5, 3), (-4, 6), (7, -2)] {
            let to = (sx + dx, sy + dy);
            let ok = route(&m, (sx, sy), to, false, 7, &blocked);
            assert!(ok.is_some(), "no route {sx},{sy} -> {to:?}");
        }
    }
}
