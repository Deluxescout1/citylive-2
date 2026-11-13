//! The planned street grid (B26b items 1, 3 and 4).
//!
//! From the town era the city grows as **rectangular blocks** instead of
//! organic branches: street lines every [`PITCH`] tiles along both axes, whose
//! lines are aligned to the terrain by picking the phase that runs a street
//! through the founding (that line becomes the main street) and by skipping
//! cells whose ground is water, cliff or already built.
//!
//! Growth is driven by need, not by a timer: a street is only laid when the
//! cell it bounds is about to be filled, a cell is only opened when the cell
//! being filled is nearly full, and the lots inside a cell are staked one at a
//! time along its frontage. That keeps road tiles at ~20 % of the developed
//! area (the brief's budget is 30 %) instead of the brown carpet of B26.

use std::collections::BTreeSet;

use map::{Kind, Tile};

use crate::{District, Era, Lot, LotKind, Sim, Stage, HOUSE_SECONDS};

/// Tiles from one street line to the next (8 tiles of block + 1 of street).
pub const PITCH: i32 = 9;
/// Every Nth street line is a two-tile avenue.
const AVENUE_EVERY: i32 = 3;
/// The cell being filled must be this full before the next one opens.
const FILL_BEFORE_OPEN: f32 = 0.55;

/// What a planned lot is for; the kind follows the district and the era at the
/// moment it is staked, so a block reserved in the town era can go mid-rise
/// when the city arrives (D6, D39).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// Detached houses (suburbs, high-end).
    Low,
    /// Shops, townhouses, apartments: the core's street wall.
    Mid,
    /// Offices and towers: downtown.
    High,
    /// Works and warehouses.
    Work,
    /// A reserved green (a square, a courtyard, a park).
    Green,
}

#[derive(Clone, Copy, Debug)]
pub struct PlanLot {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub role: Role,
}

pub struct Blocks {
    /// Street lines sit at `x = ox + i * PITCH` and `y = oy + j * PITCH`.
    pub ox: i32,
    pub oy: i32,
    ready: bool,
    /// Cells whose streets are laid.
    pub open: BTreeSet<(i32, i32)>,
    /// Cells that cannot be built (water, cliff) or that failed to connect.
    pub skip: BTreeSet<(i32, i32)>,
    /// Cells the maturing city has taken back (B26c): they may overrun the old
    /// core's organic lanes, which `cell_why` otherwise protects (D36).
    pub reclaimed: BTreeSet<(i32, i32)>,
    /// The lots of the cell being filled, and how far the fill has got.
    pub plan: Vec<PlanLot>,
    pub at: usize,
    pub cell: Option<(i32, i32)>,
    /// Cells opened so far, for the report.
    pub opened: u32,
    /// Plan slots tried and staked, and why the rest were refused.
    pub tried: u32,
    pub stood: u32,
    pub refuse: [u32; 4],
    /// Why cells were skipped: out of bounds, steep, old core, water line,
    /// did not reach the network, countryside ring.
    pub skip_why: [u32; 6],
}

impl Default for Blocks {
    fn default() -> Self {
        Self {
            ox: 0,
            oy: 0,
            ready: false,
            open: BTreeSet::new(),
            skip: BTreeSet::new(),
            reclaimed: BTreeSet::new(),
            plan: Vec::new(),
            at: 0,
            cell: None,
            opened: 0,
            tried: 0,
            stood: 0,
            refuse: [0; 4],
            skip_why: [0; 6],
        }
    }
}

/// Where a cell sits in tile space.
#[derive(Clone, Copy, Debug)]
pub struct CellRect {
    /// The street row (the cell's north edge) and column (its west edge).
    pub sx: i32,
    pub sy: i32,
    /// The interior, inclusive.
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Blocks {
    /// Set the grid up so a street line runs through the founding: that line is
    /// the main street, and every third line from it is an avenue.
    pub fn align(&mut self, founding: (i32, i32)) {
        if self.ready {
            return;
        }
        self.ox = founding.0;
        self.oy = founding.1 - (PITCH / 2);
        self.ready = true;
    }

    /// Has the grid been aligned (save/load, v17)?
    pub(crate) fn is_ready(&self) -> bool {
        self.ready
    }

    /// Restore a saved alignment (v17).
    pub(crate) fn restore_alignment(&mut self, ox: i32, oy: i32) {
        self.ox = ox;
        self.oy = oy;
        self.ready = true;
    }

    pub fn rect(&self, i: i32, j: i32) -> CellRect {
        let sx = self.ox + i * PITCH;
        let sy = self.oy + j * PITCH;
        CellRect {
            sx,
            sy,
            x0: sx + 1,
            y0: sy + 1,
            x1: sx + PITCH - 1,
            y1: sy + PITCH - 1,
        }
    }

    /// Is the street line at cell index `i` an avenue?
    pub fn avenue(&self, i: i32) -> bool {
        i.rem_euclid(AVENUE_EVERY) == 0
    }

    /// The cell that contains a tile.
    pub fn cell_of(&self, x: i32, y: i32) -> (i32, i32) {
        (
            (x - self.ox).div_euclid(PITCH),
            (y - self.oy).div_euclid(PITCH),
        )
    }

    /// Is this tile a street line of the grid?
    pub fn is_street_line(&self, x: i32, y: i32) -> bool {
        (x - self.ox).rem_euclid(PITCH) == 0 || (y - self.oy).rem_euclid(PITCH) == 0
    }

    /// Is this tile inside a cell that is being, or has been, built?
    pub fn in_built_cell(&self, x: i32, y: i32) -> bool {
        self.open.contains(&self.cell_of(x, y))
    }
}

impl Sim {
    /// The grid, aligned to the founding on first use.
    fn grid(&mut self) -> &mut Blocks {
        if !self.blocks.ready {
            let f = self.founding;
            self.blocks.align(f);
        }
        &mut self.blocks
    }

    /// The cells the planner skipped, for the reclaim pass (B26c).
    pub fn blocks_skipped(&self) -> Vec<(i32, i32)> {
        self.blocks.skip.iter().copied().collect()
    }

    /// Take a skipped cell back: only if the *only* thing against it is the old
    /// core's lanes (B26c item 1). Returns false when nothing qualifies.
    pub(crate) fn unskip_cell(&mut self, c: (i32, i32)) -> bool {
        let why = self.cell_why(c.0, c.1);
        if why != 0 && why != 3 {
            return false;
        }
        self.blocks.reclaimed.insert(c);
        self.blocks.skip.remove(&c);
        if why == 3 {
            self.blocks.skip_why[3] = self.blocks.skip_why[3].saturating_sub(1);
        }
        self.events.push(crate::Event {
            time: self.time,
            text: format!("the old core is redeveloped at {},{}", c.0, c.1),
        });
        true
    }

    /// How many tiles of countryside are kept at the frame edges: ~10 % of the
    /// frame width (D52, B24 item 2), never less than 5 tiles.
    pub fn countryside_ring(&self) -> i32 {
        ((self.map.w.min(self.map.h) as f32) * 0.10).round().max(5.0) as i32
    }

    /// Is this tile inside a block the planner has opened (B26b: dense buildings
    /// live in blocks, never alone in a field).
    pub fn in_open_cell(&self, x: i32, y: i32) -> bool {
        self.blocks.ready && self.blocks.in_built_cell(x, y)
    }

    /// Is this tile on a street line of the planned grid (the plan view uses
    /// it to draw the grid).
    pub fn blocks_on_grid(&self, x: i32, y: i32) -> bool {
        self.blocks.ready && self.blocks.is_street_line(x, y)
    }

    /// Is this road tile part of the planned grid (as opposed to the old core's
    /// organic lanes)? The report splits the two.
    pub fn on_grid(&self, x: i32, y: i32) -> bool {
        self.blocks.ready && self.blocks.is_street_line(x, y)
    }

    pub fn grid_state(&self, i: i32, j: i32) -> u8 {
        self.blocks.state(i, j)
    }

    /// Refusal counters: out-of-bounds, occupied, too steep, lot in the way.
    pub fn grid_refusals(&self) -> (u32, u32, [u32; 4]) {
        (self.blocks.tried, self.blocks.stood, self.blocks.refuse)
    }

    /// Why cells were skipped: no-connect, out of bounds, steep, old core,
    /// water on a street line.
    pub fn grid_skips(&self) -> [u32; 6] {
        self.blocks.skip_why
    }

    /// How the planned grid is doing (the report and the tests): origin, cells
    /// open, cells skipped, the size of the block being filled.
    pub fn grid_report(&self) -> ((i32, i32), u32, u32, u32, u32) {
        let b = &self.blocks;
        (
            (b.ox, b.oy),
            b.open.len() as u32,
            b.skip.len() as u32,
            b.plan.len() as u32,
            b.at as u32,
        )
    }

    pub fn grid_rect(&self, i: i32, j: i32) -> CellRect {
        self.blocks.rect(i, j)
    }

    // --- opening a cell ---------------------------------------------------

    /// Could this cell hold a block? (flat, dry ground with a buildable street
    /// line — a grid that climbs a cliff reads as a mistake, D34/D52 — and not
    /// already covered by the old core's organic lanes, which keep their own
    /// shape, D36.)
    /// 0 = fine, else the first reason it is not (index into `skip_why`).
    fn cell_why(&self, i: i32, j: i32) -> usize {
        let r = self.blocks.rect(i, j);
        // B42f: the belt under the land line is countryside as well: no grid,
        // no streets, no lots on the far edge of the frame.
        if self.behind_horizon(r.x0, r.y0) {
            return 5;
        }
        let ring = self.countryside_ring();
        if r.x0 < ring
            || r.y0 < ring
            || r.x1 >= self.map.w - ring
            || r.y1 >= self.map.h - ring
        {
            // Off the map, or in the ring of countryside D52 keeps at the frame
            // edges (farms, woods, the mountains) — B24 item 2.
            return if r.x0 < 2
                || r.y0 < 2
                || r.x1 >= self.map.w - 2
                || r.y1 >= self.map.h - 2
            {
                1
            } else {
                5
            };
        }
        let mut free = 0;
        let mut total = 0;
        let mut old = 0;
        for y in r.y0..=r.y1 {
            for x in r.x0..=r.x1 {
                let t = self.map.at(x, y);
                total += 1;
                if t.kind.is_lane() {
                    old += 1;
                }
                // A terrace of one or two steps is fine: the block grades its
                // own pads and the grid marks the retaining walls (D34).
                if t.kind.is_water() || t.relief() > 2 || t.kind.is_rocky() {
                    continue;
                }
                if self.lots.iter().any(|l| l.covers(x, y)) {
                    old += 1;
                    continue;
                }
                free += 1;
            }
        }
        if free * 100 < total * 40 {
            return 2;
        }
        // The old core keeps its organic lanes (D36) and is densified along
        // them instead of being cut up by a grid.
        if old * 100 > total * 22 && !self.blocks.reclaimed.contains(&(i, j)) {
            return 3;
        }
        // The two street lines have to be dry as well (a river gets a gap in the
        // grid, not a bridge everywhere).
        for k in 0..PITCH {
            if self.map.at(r.sx, r.sy + k).is_water() || self.map.at(r.sx + k, r.sy).is_water() {
                return 4;
            }
        }
        0
    }

    /// B99c: is this a land the planned grid cannot hold? Every cell opened or
    /// refused (marsh, lakes, islands) *and* only a handful of blocks standing:
    /// the town then keeps growing along the lanes it already has instead of
    /// stopping dead. A cramped-but-dry land still grows as blocks, so the
    /// ordinary city is untouched.
    pub(crate) fn cramped_land(&self) -> bool {
        if self.blocks.open.len() >= 40 {
            return false;
        }
        // ... and a land with a lot of water in it, which is what leaves no dry
        // rectangle for a block. A small but dry map is a different case: it
        // keeps to the blocks it has.
        let wet = self.map.water_class.iter().filter(|c| **c != 0).count();
        wet * 100 >= self.map.water_class.len().max(1) * 8
    }

    /// The next cell to open: the nearest ring of cells around the founding,
    /// among those that touch a cell already open (so the network always grows
    /// outward from the core, D32).
    fn next_cell(&self) -> Option<(i32, i32)> {
        let (fi, fj) = self.blocks.cell_of(self.founding.0, self.founding.1);
        // Nearest first, so the town grows evenly round the old core instead of
        // running off to one corner of the frame (D35, D52).
        let mut best: Option<((i32, i32), i32)> = None;
        for ring in 0..=10i32 {
            for dj in -ring..=ring {
                for di in -ring..=ring {
                    let c = (fi + di, fj + dj);
                    if self.blocks.open.contains(&c) || self.blocks.skip.contains(&c) {
                        continue;
                    }
                    if !self.blocks.open.is_empty()
                        && ![(1, 0), (-1, 0), (0, 1), (0, -1)]
                            .iter()
                            .any(|(a, b)| self.blocks.open.contains(&(c.0 + a, c.1 + b)))
                    {
                        continue;
                    }
                    let d = di * di + dj * dj;
                    if best.map(|(bc, bd)| (d, c) < (bd, bc)).unwrap_or(true) {
                        best = Some((c, d));
                    }
                }
            }
        }
        if std::env::var("SIM_DEBUG").is_ok() && best.is_none() {
            eprintln!(
                "next_cell: none. open {} skip {} founding {} and its cell {:?}",
                self.blocks.open.len(),
                self.blocks.skip.len(),
                self.founding.0 * 1000 + self.founding.1,
                (fi, fj)
            );
        }
        if let Some((c, _)) = best {
            return Some(c);
        }
        // Nothing touches the network any more: try the nearest free cell
        // anyway. `open_cell` keeps it only if its streets reach the network,
        // so a grid broken by a river or the old core can still grow past it.
        for ring in 0..=10i32 {
            for dj in -ring..=ring {
                for di in -ring..=ring {
                    if di.abs() != ring && dj.abs() != ring {
                        continue;
                    }
                    let c = (fi + di, fj + dj);
                    if self.blocks.open.contains(&c) || self.blocks.skip.contains(&c) {
                        continue;
                    }
                    let r = self.blocks.rect(c.0, c.1);
                    if r.x0 < 2
                        || r.y0 < 2
                        || r.x1 >= self.map.w - 2
                        || r.y1 >= self.map.h - 2
                    {
                        continue; // not on the map: not worth a skip
                    }
                    return Some(c);
                }
            }
        }
        None
    }

    /// Lay the street lines of a cell: its row (north) and column (west).
    /// Returns the tiles it overwrote so a failed connection can be undone.
    fn lay_cell_streets(&mut self, i: i32, j: i32) -> (Vec<(i32, i32)>, Vec<((i32, i32), Tile)>) {
        let r = self.blocks.rect(i, j);
        let ave_row = self.blocks.avenue(j);
        let ave_col = self.blocks.avenue(i);
        let mut undo: Vec<((i32, i32), Tile)> = Vec::new();
        let mut laid: Vec<(i32, i32)> = Vec::new();
        // The row runs east from the cell's corner, the column south.
        let segs: Vec<(Vec<(i32, i32)>, Kind)> = vec![
            (
                (0..PITCH).map(|k| (r.sx + k, r.sy)).collect(),
                if ave_row { Kind::Road } else { Kind::Paved },
            ),
            (
                (1..PITCH).map(|k| (r.sx, r.sy + k)).collect(),
                if ave_col { Kind::Road } else { Kind::Paved },
            ),
        ];
        for (seg, kind) in segs {
            for &(x, y) in &seg {
                let t = self.map.at(x, y);
                if t.kind.is_lane() {
                    continue; // already street: keep the older surface
                }
                if !t.is_water() {
                    undo.push(((x, y), t));
                }
                laid.push((x, y));
            }
            self.lay_lane(&seg, kind);
        }
        (laid, undo)
    }

    fn undo_tiles(&mut self, undo: Vec<((i32, i32), Tile)>) {
        for ((x, y), t) in undo {
            self.write(x, y, t);
        }
    }

    /// Do the tiles just laid touch a street that was already there? That is the
    /// local form of D32's "the network is one piece", and it is what a block
    /// needs before it may keep its streets.
    fn reaches_network(&self, laid: &[(i32, i32)]) -> bool {
        let new: BTreeSet<(i32, i32)> = laid.iter().copied().collect();
        laid.iter().any(|&(x, y)| {
            [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(dx, dy)| {
                    let p = (x + dx, y + dy);
                    !new.contains(&p) && self.map.at(p.0, p.1).kind.is_lane()
                })
        })
    }

    /// Lay a street from a cell's corner to the network along the cell's own
    /// grid lines: that is how a block cut off by the old core or a river still
    /// joins the city (D32), and it draws the avenues that link the districts
    /// to the old core (B26b item 3).
    fn connect_cell(
        &mut self,
        i: i32,
        j: i32,
        laid: &mut Vec<(i32, i32)>,
        undo: &mut Vec<((i32, i32), Tile)>,
    ) -> bool {
        let r = self.blocks.rect(i, j);
        let (fx, fy) = self.founding;
        let own: BTreeSet<(i32, i32)> = laid.iter().copied().collect();
        // Is this tile the network, outside the streets we just laid?
        let hit = |map: &map::TileMap, p: (i32, i32)| {
            !own.contains(&p) && map.at(p.0, p.1).kind.is_lane()
        };
        let mut best: Option<Vec<(i32, i32)>> = None;
        for horiz in [true, false] {
            for dir in [-1, 1] {
                let mut seg: Vec<(i32, i32)> = Vec::new();
                let mut ok = false;
                for k in 1..=60 {
                    let p = if horiz {
                        (r.sx + dir * k, r.sy)
                    } else {
                        (r.sx, r.sy + dir * k)
                    };
                    if p.0 < 2 || p.1 < 2 || p.0 > self.map.w - 3 || p.1 > self.map.h - 3 {
                        break;
                    }
                    if self.map.at(p.0, p.1).is_water() {
                        break;
                    }
                    if hit(&self.map, p) {
                        ok = true;
                        break;
                    }
                    seg.push(p);
                }
                if ok && best.as_ref().map(|b| seg.len() < b.len()).unwrap_or(true) {
                    best = Some(seg);
                }
            }
        }
        // Nothing on the cell's own lines: run a street toward the founding.
        if best.is_none() {
            let mut seg: Vec<(i32, i32)> = Vec::new();
            let mut cur = (r.sx, r.sy);
            for _ in 0..90 {
                let dx = (fx - cur.0).signum();
                let dy = (fy - cur.1).signum();
                let next = if (fx - cur.0).abs() >= (fy - cur.1).abs() {
                    (cur.0 + dx, cur.1)
                } else {
                    (cur.0, cur.1 + dy)
                };
                if next.0 < 2
                    || next.1 < 2
                    || next.0 > self.map.w - 3
                    || next.1 > self.map.h - 3
                {
                    break;
                }
                if self.map.at(next.0, next.1).is_water() {
                    break;
                }
                if hit(&self.map, next) {
                    if !seg.is_empty() {
                        best = Some(seg);
                    }
                    break;
                }
                if !own.contains(&next) {
                    seg.push(next);
                }
                cur = next;
            }
        }
        let Some(mut seg) = best else { return false };
        seg.reverse(); // start at the cell's corner and run out to the network
        let kind = match self.era() {
            Era::Town | Era::City | Era::Metropolis => Kind::Paved,
            _ => Kind::Gravel,
        };
        for &(x, y) in &seg {
            let t = self.map.at(x, y);
            if !t.kind.is_lane() {
                undo.push(((x, y), t));
                laid.push((x, y));
            }
        }
        self.lay_lane(&seg, kind);
        true
    }

    /// Open the next cell: lay its streets, then plan its lots.
    pub(crate) fn open_cell(&mut self) -> bool {
        self.open_cell_with(false)
    }

    /// B88: a city that has houses but no street at all (a save whose streets
    /// were lost, or whose planner skipped every cell for want of a network to
    /// join) lays streets again. Every cell the planner had given up on is
    /// forgotten and the grid starts afresh at the founding: the first block
    /// opens with no network to join (it *is* the network), and the rest grow
    /// off it as usual. Returns true when a block opened.
    pub(crate) fn revive_streets(&mut self) -> bool {
        if !self.settlers.struck || self.lots.iter().all(|l| matches!(l.kind, LotKind::LogCabin)) {
            return false;
        }
        if self.map.tiles.iter().any(|t| t.kind.is_lane()) {
            return false;
        }
        // Everything the planner skipped was skipped for want of a network.
        self.blocks.skip.clear();
        self.blocks.skip_why = [0; 6];
        self.blocks.open.clear();
        self.blocks.cell = None;
        self.blocks.plan.clear();
        self.blocks.at = 0;
        for _ in 0..64 {
            if self.open_cell_with(true) {
                self.push_story("the town lays its first streets: the houses have been waiting for a road");
                return true;
            }
        }
        false
    }

    fn open_cell_with(&mut self, isolated_ok: bool) -> bool {
        let Some(c) = self.next_cell() else {
            return false;
        };
        if std::env::var("SIM_DEBUG").is_ok() {
            eprintln!("open_cell try {c:?} (open {})", self.blocks.open.len());
        }
        self.grid();
        let why = self.cell_why(c.0, c.1);
        if std::env::var("SIM_DEBUG").is_ok() && why != 0 {
            eprintln!("cell {c:?} rejected: why {why}");
        }
        if why != 0 {
            self.blocks.skip_why[why] += 1;
            self.blocks.skip.insert(c);
            return false;
        }
        let (mut laid, mut undo) = self.lay_cell_streets(c.0, c.1);
        if isolated_ok {
            // The very first block of a street-less town: nothing to join.
            // It must at least have laid a street (a refused lay is no block).
            if !laid.iter().any(|&(x, y)| self.map.at(x, y).kind.is_lane()) {
                self.undo_tiles(undo);
                self.blocks.skip_why[0] += 1;
                self.blocks.skip.insert(c);
                return false;
            }
        } else if !self.reaches_network(&laid) && !self.connect_cell(c.0, c.1, &mut laid, &mut undo) {
            self.undo_tiles(undo);
            self.blocks.skip_why[0] += 1;
            self.blocks.skip.insert(c);
            return false;
        }
        if !isolated_ok && !self.reaches_network(&laid) {
            if std::env::var("SIM_DEBUG").is_ok() {
                eprintln!(
                    "cell {c:?} still isolated after connector: laid {laid:?} parts {:?}",
                    crate::grid::components(&self.map)
                );
            }
            self.undo_tiles(undo);
            self.blocks.skip_why[0] += 1;
            self.blocks.skip.insert(c);
            return false;
        }
        self.blocks.open.insert(c);
        self.blocks.opened += 1;
        self.stats.cells = self.blocks.opened;
        self.plan_cell(c.0, c.1);
        self.blocks.cell = Some(c);
        self.blocks.at = 0;
        let r = self.blocks.rect(c.0, c.1);
        self.events.push(crate::Event {
            time: self.time,
            text: format!("block opened at {},{}", r.sx, r.sy),
        });
        true
    }

    /// The lots a cell is planned to hold, frontage first.
    fn plan_cell(&mut self, i: i32, j: i32) {
        let r = self.blocks.rect(i, j);
        let (w, h) = (r.x1 - r.x0 + 1, r.y1 - r.y0 + 1);
        let cx = r.x0 + w / 2;
        let cy = r.y0 + h / 2;
        let d = self.district_at(cx, cy);
        let era = self.era();
        let mut plan: Vec<PlanLot> = Vec::new();
        let mut push = |x: i32, y: i32, lw: i32, lh: i32, role: Role| {
            plan.push(PlanLot {
                x,
                y,
                w: lw,
                h: lh,
                role,
            })
        };
        match d {
            District::Industrial => {
                // A yard with a few big sheds, kept off the middle of town.
                push(r.x0, r.y0, 3, 3, Role::Work);
                push(r.x1 - 2, r.y0, 3, 3, Role::Work);
                push(r.x0, r.y1 - 2, 3, 3, Role::Green);
                push(r.x1 - 2, r.y1 - 2, 3, 3, Role::Work);
            }
            District::Downtown if era >= Era::City => {
                // A cluster of towers around a small plaza, tallest at the core
                // (B26b item 4): three towers and an office block, so downtown is
                // a place with a skyline, not a field of identical masts.
                // Slender 2x2 towers at the corners of the block, a plaza in the
                // middle: at 1x a tower has to be taller than it is wide to read
                // as a tower (B26b item 4).
                for (dx, dy) in [(0, 0), (6, 0), (0, 6)] {
                    push(r.x0 + dx, r.y0 + dy, 2, 2, Role::High);
                }
                push(r.x1 - 1, r.y1 - 1, 2, 2, Role::Mid);
                push(r.x0 + 2, r.y0 + 2, 4, 4, Role::Green);
            }
            District::Downtown => {
                // Offices: a solid block of 2x2 lots.
                for gy in 0..4 {
                    for gx in 0..4 {
                        push(r.x0 + gx * 2, r.y0 + gy * 2, 2, 2, Role::High);
                    }
                }
            }
            District::Core | District::European if era >= Era::Town => {
                // The old core: a continuous street wall round a courtyard
                // (B26b item 4). 2x2 lots on a 4x4 lattice, the middle four
                // left as a shared garden.
                for gy in 0..4 {
                    for gx in 0..4 {
                        let inner = (gx == 1 || gx == 2) && (gy == 1 || gy == 2);
                        if inner {
                            continue;
                        }
                        push(r.x0 + gx * 2, r.y0 + gy * 2, 2, 2, Role::Mid);
                    }
                }
                push(r.x0 + 2, r.y0 + 2, 4, 4, Role::Green);
            }
            District::HighEnd => {
                // Four villas with room for gardens.
                for (dx, dy) in [(0, 0), (5, 0), (0, 5), (5, 5)] {
                    push(r.x0 + dx, r.y0 + dy, 3, 3, Role::Low);
                }
            }
            _ => {
                // Suburbs: no two blocks laid out the same way (D40). The
                // variant comes from the cell's position, so it is stable.
                let v = (self.map.tile_hash(r.sx, r.sy) * 4.0) as u8;
                match v {
                    // A ring of detached houses round a garden.
                    0 => {
                        for gy in 0..3 {
                            for gx in 0..3 {
                                let role = if gx == 1 && gy == 1 {
                                    Role::Green
                                } else {
                                    Role::Low
                                };
                                push(r.x0 + gx * 3, r.y0 + gy * 3, 2, 2, role);
                            }
                        }
                    }
                    // Two rows of houses with a back lane of gardens between.
                    1 => {
                        for gx in 0..4 {
                            push(r.x0 + gx * 2, r.y0, 2, 2, Role::Low);
                            push(r.x0 + gx * 2, r.y1 - 1, 2, 2, Role::Low);
                        }
                        push(r.x0 + 2, r.y0 + 3, 4, 2, Role::Green);
                    }
                    // Bigger plots with room for trees: the edge of town.
                    2 => {
                        for gy in 0..2 {
                            for gx in 0..2 {
                                let g = self.map.tile_hash(r.x0 + gx * 4, r.y0 + gy * 4);
                                let role = if g > 0.72 { Role::Green } else { Role::Low };
                                push(r.x0 + gx * 4 + 1, r.y0 + gy * 4 + 1, 3, 3, role);
                            }
                        }
                    }
                    // Terraces along two opposite streets.
                    _ => {
                        for gx in 0..4 {
                            push(r.x0 + gx * 2, r.y0, 2, 2, Role::Low);
                            push(r.x0 + gx * 2, r.y1 - 1, 2, 2, Role::Low);
                        }
                        for gx in 0..2 {
                            push(r.x0 + 1 + gx * 4, r.y0 + 3, 2, 2, Role::Mid);
                        }
                    }
                }
            }
        }
        self.blocks.plan = plan;
    }

    /// Stake one lot of the open cell. Returns true when something was built.
    pub(crate) fn fill_cell(&mut self) -> bool {
        let Some(_c) = self.blocks.cell else {
            return false;
        };
        let mut tries = 0;
        while self.blocks.at < self.blocks.plan.len() && tries < 4 {
            let p = self.blocks.plan[self.blocks.at];
            self.blocks.at += 1;
            tries += 1;
            let d = self.district_at(p.x, p.y);
            let mut kind = match p.role {
                Role::Green => LotKind::Park,
                Role::Work => LotKind::Industry,
                Role::High => {
                // Towers cluster round the middle of downtown and nowhere else
                // (B26b item 4: no mid-rise boxes out in the fields).
                let c = self.downtown_centre();
                let dist = (p.x - c.0).abs() + (p.y - c.1).abs();
                if dist as f32 <= 0.26 * self.span() {
                    LotKind::Tower
                } else {
                    LotKind::Office
                }
            }
                Role::Mid => self.demanded_kind(d),
                Role::Low => {
                    if d == District::HighEnd {
                        LotKind::House
                    } else {
                        self.demanded_kind(d)
                    }
                }
            };
            // A high street is not one repeated building: mix shops into the
            // terrace (D40/D46), and offices among the towers.
            let h = self.map.tile_hash(p.x, p.y);
            if kind == LotKind::Townhouse && h > 0.62 {
                kind = LotKind::Shop;
            } else if kind == LotKind::Tower && h < 0.22 {
                kind = LotKind::Office;
            } else if kind == LotKind::Apartment && h > 0.80 {
                kind = LotKind::Office;
            }
            if self.place_planned(p, kind) {
                return true;
            }
        }
        false
    }

    /// Is the cell being filled full enough to open the next one?
    pub(crate) fn cell_filled(&self) -> bool {
        if self.blocks.cell.is_none() {
            return true;
        }
        if self.blocks.plan.is_empty() {
            return true;
        }
        let done = self.blocks.at as f32 / self.blocks.plan.len() as f32;
        done >= FILL_BEFORE_OPEN
    }

    /// Stake a planned lot, with the block's rules instead of the village's:
    /// lots may touch each other along the street (that is what makes a street
    /// wall) and an inner lot may stand back from the road (back-to-back
    /// terraces). Everything else still holds: level pads, no lot on a street,
    /// no lot on water or a cliff.
    pub(crate) fn place_planned(&mut self, p: PlanLot, kind: LotKind) -> bool {
        let dbg = std::env::var("SIM_DEBUG").is_ok();
        self.blocks.tried += 1;
        if p.x < 2 || p.y < 2 || p.x + p.w > self.map.w - 2 || p.y + p.h > self.map.h - 2 {
            self.blocks.refuse[0] += 1;
            if dbg {
                eprintln!("plan refused {:?}: out of bounds", p);
            }
            return false;
        }
        if kind != LotKind::Cemetery && self.near_cemetery(p.x, p.y, p.w, p.h) {
            self.blocks.refuse[0] += 1;
            return false;
        }
        // Nothing is built on the railway's own ground either (B58d).
        if (p.y..p.y + p.h).any(|y| (p.x..p.x + p.w).any(|x| self.on_rail_ground(x, y))) {
            self.blocks.refuse[0] += 1;
            return false;
        }
        // B58c/D73: nothing is built on the airfield — a station hall on a
        // runway is what the review caught.
        if (p.y..p.y + p.h).any(|y| (p.x..p.x + p.w).any(|x| self.on_airfield(x, y))) {
            self.blocks.refuse[0] += 1;
            if dbg {
                eprintln!("plan refused {:?}: on the airfield", p);
            }
            return false;
        }
        // B42f: no building in the belt under the land line (the horizon would
        // cut it). `(p.x, p.y)` is the tile nearest the back of the frame.
        if self.behind_horizon(p.x, p.y) {
            self.blocks.refuse[0] += 1;
            if dbg {
                eprintln!("plan refused {:?}: under the horizon", p);
            }
            return false;
        }
        for y in p.y..p.y + p.h {
            for x in p.x..p.x + p.w {
                let t = self.map.at(x, y);
                if t.kind.is_water() || t.kind.is_lane() || t.kind == Kind::Sidewalk {
                    self.blocks.refuse[1] += 1;
                    if dbg {
                        eprintln!("plan refused {:?}: {:?} at {x},{y}", p, t.kind);
                    }
                    return false;
                }
                if t.relief() > 2 {
                    self.blocks.refuse[2] += 1;
                    if dbg {
                        eprintln!("plan refused {:?}: relief {} at {x},{y}", p, t.relief());
                    }
                    return false;
                }
                if self.lots.iter().any(|l| l.covers(x, y)) {
                    self.blocks.refuse[3] += 1;
                    if dbg {
                        eprintln!("plan refused {:?}: lot at {x},{y}", p);
                    }
                    return false;
                }
                // B49/B49b: ground a technology structure stands on is not
                // built over — a cell tower, a farm panel, a turbine, and the
                // clear margin a wind farm keeps around itself.
                if t.aux & map::AUX_TECH != 0 {
                    self.blocks.refuse[3] += 1;
                    if dbg {
                        eprintln!("plan refused {:?}: technology at {x},{y}", p);
                    }
                    return false;
                }
                if self
                    .tech
                    .turbines
                    .iter()
                    .any(|&(gx, gy)| (gx - x).abs() <= 2 && (gy - y).abs() <= 2)
                {
                    self.blocks.refuse[3] += 1;
                    if dbg {
                        eprintln!("plan refused {:?}: too close to a turbine", p);
                    }
                    return false;
                }
                // A detached building keeps its tile of garden even inside a
                // planned block (D41): only the attached kinds share walls.
                if !kind.urban()
                    && self.lots.iter().any(|l| {
                        x >= l.x - 1 && x < l.x + l.w + 1 && y >= l.y - 1 && y < l.y + l.h + 1
                    })
                {
                    self.blocks.refuse[3] += 1;
                    return false;
                }
            }
        }
        // The pad takes its level from the nearest street, so a lot never sits
        // above its road (B7 item 3).
        let mut level = None;
        let mut front = 0u8;
        for rad in 1..=3 {
            let mut found = None;
            for y in p.y - rad..p.y + p.h + rad {
                for x in p.x - rad..p.x + p.w + rad {
                    let on_edge = x < p.x || x >= p.x + p.w || y < p.y || y >= p.y + p.h;
                    if !on_edge || !self.map.at(x, y).kind.is_lane() {
                        continue;
                    }
                    let l = self.map.at(x, y).min_level();
                    if found.map(|f: u8| l < f).unwrap_or(true) {
                        found = Some(l);
                        front = if x >= p.x + p.w {
                            0
                        } else if y >= p.y + p.h {
                            1
                        } else if x < p.x {
                            2
                        } else {
                            3
                        };
                    }
                }
            }
            if let Some(l) = found {
                level = Some(l);
                break;
            }
        }
        // B66: a plot in a planned block stands on land the streets are on. A
        // pad two tiles off a lane across a canal has frontage and no way to
        // drive to it (islands seed 3: a home on a sand spit, never routed), so
        // the block fills elsewhere instead.
        // Civic ground is exempt (see `Sim::road_reach_ok`): a park or a school
        // has no cars, and on a cramped island the only free ground may be
        // across a strait.
        let civic = matches!(
            kind,
            LotKind::Park | LotKind::Cemetery | LotKind::Hall | LotKind::Landmark(_)
        );
        let founding = self.founding;
        self.reach.ensure(&self.map, founding);
        if !civic && !self.reach.on_network_land(p.x + p.w / 2, p.y + p.h / 2) {
            self.blocks.refuse[3] += 1;
            if dbg {
                eprintln!("plan refused {:?}: off the street network", p);
            }
            return false;
        }
        let level = level.unwrap_or_else(|| {
            let mut l = map::MAX_LEVEL;
            for y in p.y..p.y + p.h {
                for x in p.x..p.x + p.w {
                    l = l.min(self.map.at(x, y).min_level());
                }
            }
            l
        });
        self.grade_lot(p.x, p.y, p.w, p.h, level);
        let mut rng = self.rng(0x9E00 + (p.x.rem_euclid(997) * 31 + p.y.rem_euclid(997)) as u64);
        let d = self.district_at(p.x, p.y);
        let mut storeys = self.storeys_for(kind, d, p.x, p.y);
        if kind == LotKind::Park {
            storeys = 0;
        } else {
            // B99: a block near the land line stays low enough not to be cut.
            storeys = storeys.min(self.headroom_storeys(p.x, p.y).max(2));
        }
        self.blocks.stood += 1;
        // B58b item 4: a park's `style` is *what kind of park it is*, chosen so
        // that no two touching parks are the same and only one in four gets a
        // pond. Buildings keep using it as the district style.
        let style = if kind == LotKind::Park {
            self.park_style_for(p.x, p.y, p.w, p.h)
        } else {
            d.style()
        };
        self.lots.push(Lot {
            x: p.x,
            y: p.y,
            w: p.w,
            h: p.h,
            level,
            kind,
            storeys,
            style,
            seed: rng.next_u32(),
            front,
            stage: Stage::Staked,
            stage_end: self.time + HOUSE_SECONDS / 6.0,
            started: self.time,
        });
        true
    }

    /// The park kind for a new lot (B58b item 4): the size and the map's own
    /// hash suggest one, and a kind already used by a touching park is stepped
    /// past, so a row of greens varies instead of repeating one pond design.
    pub(crate) fn park_style_for(&self, x: i32, y: i32, w: i32, h: i32) -> u8 {
        let hash = self.map.tile_hash(x, y);
        let mut kind = map::park_kind_for(w, h, hash);
        for _ in 0..6 {
            let clash = self.lots.iter().any(|l| {
                l.kind == LotKind::Park
                    && map::ParkKind::of_style(l.style) == kind
                    && l.x - (x + w) < 2
                    && x - (l.x + l.w) < 2
                    && l.y - (y + h) < 2
                    && y - (l.y + l.h) < 2
            });
            if !clash {
                break;
            }
            let mut i = kind.index() + 1;
            if i as usize >= map::ParkKind::ALL.len() {
                i = 0;
            }
            kind = map::ParkKind::ALL[i as usize];
            if kind == map::ParkKind::Pond && w.min(h) < 5 {
                kind = map::ParkKind::Lawn;
            }
        }
        kind.index()
    }

    /// Heights read as a city (B26b item 4): the tallest downtown, mid-rise on
    /// the main street, low houses in the suburbs, works low by the water.
    pub(crate) fn storeys_for(&self, kind: LotKind, _d: District, x: i32, y: i32) -> u8 {
        let h = self.map.tile_hash(x, y);
        let s = match kind {
            // A terrace street is 2-4 storeys and the neighbours differ, so the
            // roofline is not a ruler (D40).
            LotKind::Townhouse | LotKind::Shop => 2 + (h * 3.0) as u8,
            LotKind::Apartment => 4 + (h * 6.0) as u8,
            LotKind::Office => 3 + (h * 4.0) as u8,
            LotKind::Tower => {
                // Tallest in the middle of downtown, stepping down outward, and
                // no two neighbours the same height (D40). The radius is a
                // fraction of the world so the wide layout gets a real skyline
                // (B26c item 2).
                let c = self.downtown_centre();
                let dist = ((x - c.0).abs() + (y - c.1).abs()) as f32;
                let span = self.span().max(1.0);
                let lift = ((0.40 - dist / span) * 24.0).max(0.0);
                (10.0 + h * 8.0 + lift).clamp(10.0, 34.0) as u8
            }
            LotKind::Hall => 4,
            _ => kind.base_storeys().max(1),
        };
        s.max(1)
    }

    /// The middle of the downtown district (its seed point).
    pub fn downtown_centre(&self) -> (i32, i32) {
        for &(p, d) in self.districts.as_ref().map(|v| v.as_slice()).unwrap_or(&[]) {
            if d == District::Downtown {
                return p;
            }
        }
        self.founding
    }

    /// The old core keeps its organic lanes (D36); the town era fills the gaps
    /// between the cottages along them with terraces, so the main street gets a
    /// continuous 2-4 storey wall (B26b item 4).
    pub(crate) fn densify_core(&mut self) -> bool {
        if self.era() < Era::Town {
            return false;
        }
        let (fx, fy) = self.founding;
        let lanes: Vec<(i32, i32)> = self
            .lane_tiles()
            .into_iter()
            .filter(|&(x, y)| (x - fx).abs() + (y - fy).abs() <= 15)
            .collect();
        if lanes.is_empty() {
            return false;
        }
        let mut rng = self.rng(0x0DA1);
        for _ in 0..20 {
            let &(x, y) = &lanes[rng.usize(lanes.len())];
            let along_x =
                self.map.at(x - 1, y).kind.is_lane() || self.map.at(x + 1, y).kind.is_lane();
            let along = if along_x { (1, 0) } else { (0, 1) };
            let side = if rng.chance(0.5) {
                (-along.1, along.0)
            } else {
                (along.1, -along.0)
            };
            for i in 0..4 {
                for dir in [1, -1] {
                    let step = i * dir;
                    let (px, py) = (
                        x + along.0 * step + side.0 * 2,
                        y + along.1 * step + side.1 * 2,
                    );
                    let d = self.district_at(px, py);
                    let kind = self.demanded_kind(d);
                    let p = PlanLot {
                        x: px,
                        y: py,
                        w: 2,
                        h: 2,
                        role: Role::Mid,
                    };
                    if self.place_planned(p, kind) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Civic extras of the town era: the town hall and its square, a park in
    /// every district (D46), and works on the industrial ground.
    pub(crate) fn try_town_extras(&mut self, _days: f64) -> bool {
        if self.era() >= Era::Town && !self.has_kind(LotKind::Hall) && self.place_hall() {
            return true;
        }
        // B35: the city's landmarks, one at a time, as its era and population
        // reach them.
        // B52: what the voters approved goes up first.
        if self.try_civic_projects() {
            return true;
        }
        if self.try_landmark() {
            return true;
        }
        // B59: the places cars need — filling stations, car parks, the
        // drive-through, and the highway with its interchange.
        if self.try_car_places() {
            return true;
        }
        if self.plan_highway() {
            return true;
        }
        if self.era() >= Era::Town && !self.has_kind(LotKind::Industry) && self.place_industry() {
            return true;
        }
        // D46: a district without a green gets one.
        for d in [
            District::Core,
            District::European,
            District::Suburb,
            District::Downtown,
            District::HighEnd,
            District::Industrial,
        ] {
            let n = self
                .lots
                .iter()
                .filter(|l| l.kind == LotKind::Park && self.district_at(l.x, l.y) == d)
                .count();
            if n == 0 {
                // A proper town park, not a courtyard: 6x6, so the renderer can
                // put a pond, paths and a ring of trees in it (B26c item 3).
                if let Some(p) = self.free_spot_in(d, 6) {
                    if self.place_planned(
                        PlanLot {
                            x: p.0,
                            y: p.1,
                            w: 6,
                            h: 6,
                            role: Role::Green,
                        },
                        LotKind::Park,
                    ) {
                        self.events.push(crate::Event {
                            time: self.time,
                            text: format!("a park reserved in the {} district", d.name()),
                        });
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Is there a street within `r` tiles of a patch?
    fn road_near(&self, x: i32, y: i32, size: i32, r: i32) -> bool {
        for ty in y - r..y + size + r {
            for tx in x - r..x + size + r {
                if self.map.at(tx, ty).kind.is_lane() {
                    return true;
                }
            }
        }
        false
    }

    /// A free square patch of `size` in a district, searched outward from the
    /// founding (deterministic).
    pub(crate) fn free_spot_in(&self, d: District, size: i32) -> Option<(i32, i32)> {
        let (fx, fy) = self.founding;
        for ring in 2..36i32 {
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let (x, y) = (fx + dx, fy + dy);
                    if x < 3 || y < 3 || x + size > self.map.w - 3 || y + size > self.map.h - 3 {
                        continue;
                    }
                    if self.district_at(x + size / 2, y + size / 2) != d {
                        continue;
                    }
                    let mut ok = true;
                    for ty in y - 1..y + size + 1 {
                        for tx in x - 1..x + size + 1 {
                            let t = self.map.at(tx, ty);
                            if t.is_water() || t.kind.is_lane() || t.relief() > 1 {
                                ok = false;
                            }
                            if self.lots.iter().any(|l| l.covers(tx, ty)) {
                                ok = false;
                            }
                        }
                    }
                    if ok {
                        return Some((x, y));
                    }
                }
            }
        }
        None
    }

    /// The town hall on a square just off the main street (D24, B26b item 4).
    fn place_hall(&mut self) -> bool {
        // On a square off the main street, so the civic heart is on the street
        // (B26b item 4).
        let Some(p) = self
            .free_spot_in(District::Core, 3)
            .filter(|&(x, y)| self.road_near(x, y, 3, 3))
        else {
            return false;
        };
        // The square: a green the hall fronts.
        let sq = (p.0 + 3, p.1);
        self.place_planned(
            PlanLot {
                x: p.0,
                y: p.1,
                w: 3,
                h: 3,
                role: Role::Mid,
            },
            LotKind::Hall,
        ) && self.place_planned(
            PlanLot {
                x: sq.0,
                y: sq.1,
                w: 4,
                h: 4,
                role: Role::Green,
            },
            LotKind::Park,
        )
    }

    /// Works on the industrial ground: the first one appears as soon as the town
    /// reaches it (D38).
    fn place_industry(&mut self) -> bool {
        // Works go where the streets already are (D38): industry on the edge of
        // the built town, not a shed alone in a field.
        let Some(p) = self
            .free_spot_in(District::Industrial, 4)
            .filter(|&(x, y)| self.road_near(x, y, 4, 3))
        else {
            return false;
        };
        self.place_planned(
            PlanLot {
                x: p.0,
                y: p.1,
                w: 4,
                h: 3,
                role: Role::Work,
            },
            LotKind::Industry,
        )
    }
}

impl Blocks {
    /// Which cells are open / skipped (the diagnostic probe).
    pub fn state(&self, i: i32, j: i32) -> u8 {
        if self.open.contains(&(i, j)) {
            1
        } else if self.skip.contains(&(i, j)) {
            2
        } else {
            0
        }
    }
}
