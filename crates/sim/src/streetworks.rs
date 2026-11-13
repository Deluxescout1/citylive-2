//! Street furniture, installed by visible crews (B41/D63).
//!
//! When a lane becomes a paved street the town furnishes it, in order and one
//! tile at a time so the work is watchable (D33): a **line-painting truck** lays
//! the centre line along the street, then **signs** go up at the junctions, then
//! **street lights** are set at intervals, and in the town era **traffic lights**
//! appear at the big junctions. Everything is static once installed; only the
//! crews and the signal lamps animate.

use std::collections::BTreeSet;

use map::Kind;

use crate::roads::RoadClass;
use crate::Sim;

/// How long one tile of a job takes, in sim seconds (a street is minutes, not
/// one frame — D33).
pub const TILE_SECONDS: f64 = 18.0;
/// A street light every this many paved tiles.
pub const LAMP_EVERY: usize = 5;

/// What a crew is installing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    /// Centre lines: a paint truck moving along the street.
    Paint,
    /// Street-name blades and stop signs at the junctions.
    Signs,
    /// Street-light poles along the street.
    Lamps,
    /// Traffic lights at the big junctions (town and later).
    Lights,
}

impl JobKind {
    pub fn name(self) -> &'static str {
        match self {
            JobKind::Paint => "painting the lines",
            JobKind::Signs => "putting up the signs",
            JobKind::Lamps => "setting the street lights",
            JobKind::Lights => "wiring the traffic lights",
        }
    }
}

/// A crew working along a street, one tile at a time.
#[derive(Clone, Debug)]
pub struct Job {
    pub kind: JobKind,
    pub path: Vec<(i32, i32)>,
    pub i: usize,
    pub next_at: f64,
}

/// Everything the town has installed on its streets (B41).
#[derive(Clone, Debug, Default)]
pub struct Streetworks {
    /// Tiles with a painted centre line.
    pub paint: BTreeSet<(i32, i32)>,
    /// Street-light poles.
    pub lamps: BTreeSet<(i32, i32)>,
    /// Signs: (x, y, 0 = street-name blade, 1 = stop sign).
    pub signs: Vec<(i32, i32, u8)>,
    /// Traffic lights at the big junctions.
    pub lights: BTreeSet<(i32, i32)>,
    /// The crews at work.
    pub jobs: Vec<Job>,
    /// Tiles already visited by a job of each kind, so nothing is redone.
    pub painted_done: BTreeSet<(i32, i32)>,
    pub lamps_done: BTreeSet<(i32, i32)>,
    pub signs_done: BTreeSet<(i32, i32)>,
    pub lights_done: BTreeSet<(i32, i32)>,
    /// Which sign each queued tile will carry (B55): decided when the job is planned.
    pub sign_plan: std::collections::BTreeMap<(i32, i32), u8>,
    /// B61: the town has furnished every street it has, so the crews go quiet.
    /// Holds the era the scan came up empty in (an era change can bring traffic
    /// lights), and is cleared whenever the street network or the town changes
    /// under it (`resume_streetworks`), so the whole network is no longer
    /// re-scanned and re-classed on every planning step for nothing.
    pub halted_in: Option<crate::Era>,
}

impl Streetworks {
    /// Where the crews are, for the renderer: (x, y, kind).
    pub fn crews(&self) -> Vec<(i32, i32, u8)> {
        self.jobs
            .iter()
            .filter_map(|j| {
                let p = j.path.get(j.i.min(j.path.len().saturating_sub(1)))?;
                let k = match j.kind {
                    JobKind::Paint => 0,
                    JobKind::Signs => 1,
                    JobKind::Lamps => 2,
                    JobKind::Lights => 3,
                };
                Some((p.0, p.1, k))
            })
            .collect()
    }

    /// Is anything still being installed?
    pub fn busy(&self) -> bool {
        !self.jobs.is_empty()
    }
}

impl Sim {
    /// B61: the street network changed (a lane laid, a street paved, a block
    /// opened, a plot cleared): let the crews look for work again.
    pub(crate) fn resume_streetworks(&mut self) {
        self.streetworks.halted_in = None;
        // B49: the tech crews look again too — a new street needs poles and a
        // new house has a bare roof.
        self.tech.halted_in = None;
    }

    /// Advance the street works. Called every planning step: cheap when idle.
    pub fn streetworks_step(&mut self) {
        let now = self.time;
        // Finish whatever is due.
        let mut due: Vec<(usize, JobKind, (i32, i32))> = Vec::new();
        for (idx, job) in self.streetworks.jobs.iter_mut().enumerate() {
            while job.i < job.path.len() && now + 1e-9 >= job.next_at {
                due.push((idx, job.kind, job.path[job.i]));
                job.i += 1;
                job.next_at += TILE_SECONDS;
            }
        }
        for (_, kind, (x, y)) in due {
            self.install(kind, x, y);
        }
        self.streetworks
            .jobs
            .retain(|j| j.i < j.path.len());
        // Start the next job when the crews are free. B61: once every street is
        // furnished the scan found nothing every planning step, for the rest of
        // the run — so it is not repeated until something changes.
        if self.streetworks.jobs.is_empty() && self.era() >= crate::Era::Village {
            let era = self.era();
            if self.streetworks.halted_in != Some(era) {
                match self.next_streetwork(now) {
                    Some(job) => {
                        self.streetworks.halted_in = None;
                        self.streetworks.jobs.push(job);
                    }
                    None => self.streetworks.halted_in = Some(era),
                }
            }
        }
    }

    /// Install one feature tile.
    fn install(&mut self, kind: JobKind, x: i32, y: i32) {
        // B48a/D65: the first street works of each kind are an event, told once.
        // The bits follow the `refresh_firsts` order: paint, signs, lamps, lights.
        let bit = 1u8 << match kind {
            JobKind::Paint => 0,
            JobKind::Signs => 1,
            JobKind::Lamps => 2,
            JobKind::Lights => 3,
        };
        if self.firsts.sw_done & bit == 0 {
            self.firsts.sw_done |= bit;
            self.push_story_t(crate::chronicle::Topic::Works, 0, match kind {
                JobKind::Paint => {
                    "the first centre lines are painted down the main street"
                }
                JobKind::Signs => "the street-name signs go up: no more lost deliveries",
                JobKind::Lamps => "the first street lights are set: the town keeps longer hours",
                JobKind::Lights => "the first traffic lights come on at the main junction",
            });
        }
        match kind {
            JobKind::Paint => {
                self.streetworks.paint.insert((x, y));
                self.streetworks.painted_done.insert((x, y));
            }
            JobKind::Lamps => {
                self.streetworks.lamps.insert((x, y));
                self.streetworks.lamps_done.insert((x, y));
            }
            JobKind::Signs => {
                let k = self.streetworks.sign_plan.remove(&(x, y)).unwrap_or(0);
                if !self.streetworks.signs.iter().any(|&(sx, sy, _)| (sx, sy) == (x, y)) {
                    self.streetworks.signs.push((x, y, k));
                }
                self.streetworks.signs_done.insert((x, y));
            }
            JobKind::Lights => {
                self.streetworks.lights.insert((x, y));
                self.streetworks.lights_done.insert((x, y));
            }
        }
        self.remark_around(x, y);
    }

    /// Pick the next job, in the order the brief asks for: paint, signs, lights,
    /// traffic signals.
    fn next_streetwork(&mut self, now: f64) -> Option<Job> {
        let paved: Vec<(i32, i32)> = self
            .lane_tiles()
            .into_iter()
            .filter(|&(x, y)| self.road_class(x, y).is_paved())
            .collect();
        if paved.is_empty() {
            return None;
        }
        // 1. Paint the centre line of every paved street.
        if let Some(path) = self.street_run(&paved, &self.streetworks.painted_done) {
            return Some(Job {
                kind: JobKind::Paint,
                path,
                i: 0,
                next_at: now + TILE_SECONDS,
            });
        }
        // 2. Signs at the junctions (a blade) and on the minor approaches (stop).
        let plan = self.sign_placements(&paved);
        // The town changes: drop any sign its junction no longer warrants.
        self.streetworks
            .signs
            .retain(|&(x, y, k)| plan.get(&(x, y)) == Some(&k));
        self.streetworks.signs_done = self.streetworks.signs.iter().map(|&(x, y, _)| (x, y)).collect();
        let signs_done = self.streetworks.signs_done.clone();
        let mut sign_spots: Vec<(i32, i32)> = Vec::new();
        for (&(x, y), &k) in &plan {
            if !signs_done.contains(&(x, y)) {
                sign_spots.push((x, y));
                self.streetworks.sign_plan.insert((x, y), k);
            }
        }
        if !sign_spots.is_empty() {
            return Some(Job {
                kind: JobKind::Signs,
                path: sign_spots,
                i: 0,
                next_at: now + TILE_SECONDS,
            });
        }
        // 3. Street lights along the streets.
        if let Some(path) = self.lamp_run(&paved, &self.streetworks.lamps_done) {
            return Some(Job {
                kind: JobKind::Lamps,
                path,
                i: 0,
                next_at: now + TILE_SECONDS,
            });
        }
        // 4. Traffic lights at the big junctions, once the town is up.
        if self.era() >= crate::Era::Town {
            let done = self.streetworks.lights_done.clone();
            let mut spots: Vec<(i32, i32)> = Vec::new();
            let centres = self.junction_centres(&paved);
            for &(x, y) in &paved {
                if done.contains(&(x, y)) || self.lane_degree(x, y) < 3 {
                    continue;
                }
                if !self.light_junction(x, y) {
                    continue;
                }
                if !centres.contains(&(x, y)) {
                    continue;
                }
                spots.push((x, y));
            }
            if !spots.is_empty() {
                return Some(Job {
                    kind: JobKind::Lights,
                    path: spots,
                    i: 0,
                    next_at: now + TILE_SECONDS,
                });
            }
        }
        None
    }

    /// One representative tile per junction: every true crossroads, and for a
    /// T the single tile that owns it (B41b, B55).
    fn junction_centres(&self, paved: &[(i32, i32)]) -> Vec<(i32, i32)> {
        let mut spots: Vec<(i32, i32)> = Vec::new();
        for &(x, y) in paved {
            let deg = self.lane_degree(x, y);
            if deg < 3 {
                continue;
            }
            let crowded = (x - 1..=x + 1).any(|nx| {
                (y - 1..=y + 1).any(|ny| {
                    (nx, ny) != (x, y) && self.lane_degree(nx, ny) >= deg.max(3)
                })
            });
            if deg < 4 && crowded {
                continue;
            }
            if spots
                .iter()
                .any(|&(sx, sy)| (sx - x).abs() <= 1 && (sy - y).abs() <= 1)
            {
                continue;
            }
            spots.push((x, y));
        }
        spots
    }

    /// Would a traffic light stand at this junction (a big one: an avenue meets it)?
    fn light_junction(&self, x: i32, y: i32) -> bool {
        self.road_class(x, y) == RoadClass::Avenue || self.adjacent_class(x, y, RoadClass::Avenue)
    }

    /// B55: where signs belong. One street-name blade per junction of paved
    /// streets, and a stop sign on each minor approach into a busier street,
    /// except where a traffic light will stand. Tracks and lanes get none.
    pub fn sign_placements(
        &self,
        paved: &[(i32, i32)],
    ) -> std::collections::BTreeMap<(i32, i32), u8> {
        let rank = |c: RoadClass| match c {
            RoadClass::Local => 1,
            RoadClass::Collector => 2,
            RoadClass::Avenue => 3,
            _ => 0,
        };
        let mut out = std::collections::BTreeMap::new();
        for (x, y) in self.junction_centres(paved) {
            let arms: Vec<(i32, i32)> = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .map(|&(dx, dy)| (x + dx, y + dy))
                .filter(|&(ax, ay)| self.map.at(ax, ay).kind.is_lane())
                .collect();
            // Only where every street meeting here is a named, paved one.
            if arms.iter().any(|&(ax, ay)| !self.road_class(ax, ay).is_paved()) {
                continue;
            }
            out.insert((x, y), 0);
            if self.light_junction(x, y) {
                continue;
            }
            let top = arms
                .iter()
                .map(|&(ax, ay)| rank(self.road_class(ax, ay)))
                .max()
                .unwrap_or(0)
                .max(rank(self.road_class(x, y)));
            for &(ax, ay) in &arms {
                if rank(self.road_class(ax, ay)) < top {
                    out.insert((ax, ay), 1);
                }
            }
        }
        out
    }

    /// Thin a legacy sign list (older saves put a sign on every junction tile)
    /// down to what `sign_placements` would have built.
    pub fn thin_signs(&mut self) {
        let paved: Vec<(i32, i32)> = self
            .lane_tiles()
            .into_iter()
            .filter(|&(x, y)| self.road_class(x, y).is_paved())
            .collect();
        let plan = self.sign_placements(&paved);
        self.streetworks
            .signs
            .retain(|&(x, y, k)| plan.get(&(x, y)) == Some(&k));
        self.streetworks.signs.sort();
        self.streetworks.signs.dedup();
        self.streetworks.signs_done = self.streetworks.signs.iter().map(|&(x, y, _)| (x, y)).collect();
    }

    fn adjacent_class(&self, x: i32, y: i32, want: RoadClass) -> bool {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1), (2, 0), (-2, 0), (0, 2), (0, -2)] {
            if self.road_class(x + dx, y + dy) == want {
                return true;
            }
        }
        false
    }

    /// A contiguous run of paved tiles that still needs a centre line, longest
    /// first (so the main street is painted before the cul-de-sacs).
    fn street_run(
        &self,
        paved: &[(i32, i32)],
        done: &BTreeSet<(i32, i32)>,
    ) -> Option<Vec<(i32, i32)>> {
        let need: BTreeSet<(i32, i32)> = paved
            .iter()
            .copied()
            .filter(|p| !done.contains(p))
            .collect();
        let mut best: Vec<(i32, i32)> = Vec::new();
        for &start in need.iter() {
            // Walk one way then the other along the tile's axis.
            let along_x = need.contains(&(start.0 - 1, start.1))
                || need.contains(&(start.0 + 1, start.1));
            let (dx, dy) = if along_x { (1, 0) } else { (0, 1) };
            let mut run = vec![start];
            for sgn in [1, -1] {
                let mut k = 1;
                while need.contains(&(start.0 + dx * k * sgn, start.1 + dy * k * sgn)) {
                    run.push((start.0 + dx * k * sgn, start.1 + dy * k * sgn));
                    k += 1;
                }
            }
            if run.len() > best.len() {
                best = run;
            }
        }
        if best.len() >= 3 {
            Some(best)
        } else {
            None
        }
    }

    /// Every fifth paved tile along a street, for a lamp post.
    fn lamp_run(
        &self,
        paved: &[(i32, i32)],
        done: &BTreeSet<(i32, i32)>,
    ) -> Option<Vec<(i32, i32)>> {
        let need: BTreeSet<(i32, i32)> = paved
            .iter()
            .copied()
            .filter(|p| !done.contains(p))
            .collect();
        let mut spots: Vec<(i32, i32)> = Vec::new();
        for &start in need.iter() {
            let along_x = need.contains(&(start.0 - 1, start.1))
                || need.contains(&(start.0 + 1, start.1));
            let (dx, dy) = if along_x { (1, 0) } else { (0, 1) };
            // Only count from the start of the run, so the spacing is regular.
            if need.contains(&(start.0 - dx, start.1 - dy)) {
                continue;
            }
            let mut k = 0;
            while need.contains(&(start.0 + dx * k, start.1 + dy * k)) {
                if k as usize % LAMP_EVERY == 2 {
                    spots.push((start.0 + dx * k, start.1 + dy * k));
                }
                k += 1;
            }
        }
        if spots.is_empty() {
            None
        } else {
            Some(spots)
        }
    }

    /// A snapshot for the renderer.
    pub fn streetworks(&self) -> &Streetworks {
        &self.streetworks
    }

    /// How much of the paved network is painted (the handback's progress line).
    pub fn paint_progress(&self) -> f32 {
        let paved: Vec<(i32, i32)> = self
            .lane_tiles()
            .into_iter()
            .filter(|&(x, y)| self.road_class(x, y).is_paved())
            .collect();
        if paved.is_empty() {
            return 1.0;
        }
        let painted = paved
            .iter()
            .filter(|p| self.streetworks.paint.contains(p))
            .count();
        painted as f32 / paved.len() as f32
    }
}

/// A tile's paint kind, for the renderer: 0 = none, 1 = dashes, 2 = double line.
pub fn paint_style(s: &Sim, x: i32, y: i32) -> u8 {
    if !s.streetworks.paint.contains(&(x, y)) {
        return 0;
    }
    match s.road_class(x, y) {
        RoadClass::Avenue => 2,
        RoadClass::Collector => 1,
        _ => 0,
    }
}

/// Is this a paved tile with a kerb (a sidewalk or open ground) on one side?
pub fn has_kerb(map: &map::TileMap, x: i32, y: i32) -> bool {
    [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .any(|(dx, dy)| {
            let t = map.at(x + dx, y + dy);
            !t.kind.is_lane() && !t.is_water() && matches!(t.kind, Kind::Grass | Kind::Meadow | Kind::Sidewalk | Kind::Paved | Kind::Road)
        })
}
